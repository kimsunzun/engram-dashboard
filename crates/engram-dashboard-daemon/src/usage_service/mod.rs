//! 사용량 한도 서비스 — 벤더 중립.
//!
//! ★벤더 이름·키 목록·정책(쿨타임·시한)을 여기 두지 않는다★ — 전부 받은 조회기(실물 =
//! `engram_dashboard_agent::backend::usage_probes()`)에서 받는다. 벤더 match·벤더 리터럴이 이 모듈에 생기면
//! 「백엔드 확장」 위반이다.
//!
//! [`UsageService`] 가 책([`book::UsageBook`])·구독 명부([`watch::UsageWatch`])·시계·인코더·조회기·거절
//! 저장소·스케줄러 깨우기 송신단을 쥐고 줍기 적용·구독 교체·요청·조회 구동·발행을 몬다(TRD §1-4).
//! ★락 순서 = 명부 → 책 · `save_lock` → 책 · 책은 잎이다★ — 책을 쥔 채 명부·출구·인코더·깨우기·합류자 깨우기·
//!   조회 스레드 기동·파일 쓰기를 하지 않는다. 명부와 `save_lock` 을 함께 쥐는 경로는 없다.
//! ★발행은 「적용 먼저, 대상 열거 나중」이다★ — 책 락을 놓은 **뒤** 명부에서 대상을 뜨고, 어느 락도 없이
//!   나른다([`watch::deliver`]). 구독 교체의 「등록 먼저, 스냅숏 나중」과 짝을 이뤄 겹친 교체·발행이 그 연결에
//!   책의 값 이상을 닿게 한다(TRD §3 #57).
//! ★한 연결에 닿는 차례는 경로끼리 보장하지 않는다★ — 보내기가 락 밖이라 두 경로(줍기·교체·조회·스케줄러)의
//!   한 장이 뒤바뀌어 작은 revision 이 늦게 닿거나 같은 한 장이 두 번 닿을 수 있다. 받는 쪽이 칸마다 큰 revision
//!   만 남기는 것(TRD §1-7·§1-8)에 기댄 설계다.
//! ★조회는 시작한 쪽과 무관하게 한 벌의 끝을 탄다★ — 요청이 시작했든 스케줄러가 시작했든 [`ProbeGuard`] 가
//!   닫고, 합류한 요청은 그 끝을 기다린다. 조회 스레드는 서비스를 `Weak` 로만 쥔다.
// ADR-0004
// ADR-0006

pub mod book;
pub mod clock;
pub mod observe;
pub mod reject_store;
pub mod schedule;
pub mod watch;

use std::collections::{BTreeSet, HashMap};
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use engram_dashboard_agent::usage::{
    ProbeEnv, ProbeError, ProbeFailure, ProbeSpawner, UsageAccountKey, UsageKey, UsageObservation,
    UsageProbe, UsageVendorKey,
};
use engram_dashboard_net::frame_port::ConnId;
use engram_dashboard_protocol::UsageLimitSnapshot;

use book::{Coalesce, Judgment, Now, RequestKind, TickPlan, UsageBook};
use clock::UsageClock;
use reject_store::{RejectEntry, RejectStore};
use watch::{deliver, Replaced, UsageEncoder, UsageOutlet, UsageWatch};

/// 요청이 조회 끝을 기다리는 상한(§3 #16) — 버스 마감 7초·셸 답장 상한 30초 아래. 넘으면 캐시 + `in_flight`
/// 로 답하고 조회는 계속된다.
pub const REPLY_WAIT_MAX: Duration = Duration::from_secs(5);

/// 요청의 답이 그 요청이 기다린 조회에서 나온 값인가.
///
/// ★데몬 안에서만 쓴다 — wire 스냅숏은 싣지 않는다★: wire 스냅숏은 방송과 셸 캐시로만 흐르고(⟳ 답은
///   `Ack`) 거기선 늘 `Cached` 라 뜻이 없다(TRD §3 #50·#81). 싣는 곳은 버스 행이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageServed {
    /// 이 요청이 기다린 조회(또는 그 뒤의 조회)가 성공해 받아 온 값이다.
    Fresh,
    /// 이번 요청 동안 새로 받은 값이 없다(조회 안 함 · 실패 · 기다림 상한 초과) — 서비스가 들고 있던 값이다.
    Cached,
}

/// 요청 하나의 답 — 답하는 순간의 한 장.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageAnswer {
    pub snapshot: UsageLimitSnapshot,
    pub served: UsageServed,
}

/// 조회 스레드를 띄우는 손잡이 — 실물 = [`OsProbeThreads`].
pub trait ProbeThreads: Send + Sync {
    /// `run` 을 새 스레드에서 한 번 돌리고 곧바로 돌아온다(기다리지 않는다). `Err` = 못 띄웠다 — 부르는 쪽이
    /// 그 자리에서 조회를 실패로 닫는다. `run` 을 놓든 쥐든(뒤에 돌리든) 조회는 한 번만 닫힌다.
    fn spawn(&self, run: Box<dyn FnOnce() + Send>) -> io::Result<()>;
}

/// 이름 `usage-probe` 인 분리 std 스레드 — tokio blocking 풀이 아니라 런타임 drop 이 기다리지 않고, 자식은 Job 이
/// 데몬과 함께 거둔다(§3 #39).
pub struct OsProbeThreads;

impl ProbeThreads for OsProbeThreads {
    fn spawn(&self, run: Box<dyn FnOnce() + Send>) -> io::Result<()> {
        std::thread::Builder::new()
            .name("usage-probe".to_owned())
            .spawn(run)
            .map(drop)
    }
}

/// [`UsageService::new`] 가 받는 것 — 실물은 조립이 준다.
pub struct UsageParts {
    /// 칸 = 조회기마다 기본 계정 하나(키·정책은 조회기가 준다 · 같은 키가 둘이면 앞의 것 · wire 벤더로 못 바꾸는
    /// 키는 칸이 없다 — [`UsageBook::new`]).
    pub probes: Vec<&'static dyn UsageProbe>,
    pub spawner: Arc<dyn ProbeSpawner>,
    /// 조회마다 임시 폴더를 만들 부모 폴더([`ProbeEnv::scratch_root`]) — 데몬 전용 폴더.
    pub scratch_root: PathBuf,
    pub threads: Arc<dyn ProbeThreads>,
    /// 조회 끝이 거절 기한을 바꿀 때마다 전량 저장한다. 읽기(복원)는 조립이 한다([`UsageService::restore_rejects`]).
    pub rejects: Arc<dyn RejectStore>,
    pub clock: Arc<dyn UsageClock>,
    pub encoder: Arc<dyn UsageEncoder>,
}

/// 책 락 하나가 함께 쥐는 것 — 책과 칸마다의 조회 끝 기록. 합류자 등록(요청 판정)과 조회 끝 적용(완료 가드)이
/// 같은 락 안에서 차례로 서므로, 합류한 요청이 기다리는 끝이 어느 조회의 끝인지 어긋나지 않는다.
struct Desk {
    book: UsageBook,
    runs: HashMap<UsageKey, Runs>,
}

/// 칸 하나의 조회 끝 기록 — 칸마다 조회는 많아야 하나가 진행 중이므로(책의 `in_flight`) 진행 중인 조회는 늘
/// `finished + 1` 번째 끝이다.
#[derive(Default)]
struct Runs {
    finished: u64,
    /// 마지막으로 성공한 끝의 번호(0 = 없음) — 기다린 끝 이후에 성공이 있었으면 그 요청의 답은 `Fresh` 다.
    last_ok: u64,
    /// 진행 중인 조회를 기다리는 버스 요청 — 완료 가드가 꺼내 깨운다(받는 쪽이 이미 떠났으면 버려진다).
    blocking: Vec<SyncSender<()>>,
}

/// 데몬마다 하나.
///
/// ★기다리는 것은 요청 둘([`UsageService::request`]·[`UsageService::request_blocking`])뿐이다★ — 나머지는 전부
///   논블록이다: 잡는 락은 책·명부의 짧은 메모리 구간(과 조회 끝의 거절 저장 `save_lock`)뿐이고, 보내기는 출구의
///   `try_send` · 깨우기는 채널 `try_send` · 조회는 분리 스레드다. 그래서 pump 스레드(줍기)와 연결 태스크(구독
///   교체)에서 곧장 부른다.
pub struct UsageService {
    /// 책 락 — 잎이다.
    desk: Mutex<Desk>,
    watch: UsageWatch,
    probes: Vec<&'static dyn UsageProbe>,
    spawner: Arc<dyn ProbeSpawner>,
    scratch_root: PathBuf,
    threads: Arc<dyn ProbeThreads>,
    rejects: Arc<dyn RejectStore>,
    /// 거절 저장을 한 줄로 세운다 — 이 안에서 책을 다시 떠 쓰므로 늦게 뜬 목록이 먼저 뜬 목록에 덮이지 않는다.
    save_lock: Mutex<()>,
    /// 칸마다 끝난 조회 수 — async 합류자가 기다린다. 값은 [`Runs::finished`] 를 따라가고 줄지 않는다.
    finished: HashMap<UsageKey, tokio::sync::watch::Sender<u64>>,
    clock: Arc<dyn UsageClock>,
    encoder: Arc<dyn UsageEncoder>,
    /// 스케줄러 깨우기 — 용량 1. ★사본을 밖(조회 스레드 등)에 주지 않는다★ — 주면 서비스가 사라져도 받는 쪽이
    /// `Disconnected` 를 못 봐 스케줄러가 끝나지 않는다.
    wake: SyncSender<()>,
    reply_wait: Duration,
    /// 완료 가드에 넘기는 자기 참조 — 조회 스레드가 서비스를 붙들지 않게 약하게만.
    me: Weak<Self>,
}

/// 요청 판정 뒤 — 곧바로 답하거나, 그 끝 번호까지 기다린다.
enum Opened {
    Answer(UsageAnswer),
    Wait(u64),
}

impl UsageService {
    /// 함께 돌려주는 수신단은 스케줄러 스레드 몫이다([`schedule::spawn_scheduler`] 에 넘긴다) — 깸(`()`)은 많아야
    /// 하나가 대기하고, 서비스가 drop 되면 `Disconnected` 가 된다. 받는 쪽이 없어도(버렸음) 서비스는 그대로 돈다 —
    /// 깨우기만 허공에 간다.
    pub fn new(parts: UsageParts) -> (Arc<Self>, Receiver<()>) {
        Self::build(parts, REPLY_WAIT_MAX)
    }

    fn build(parts: UsageParts, reply_wait: Duration) -> (Arc<Self>, Receiver<()>) {
        let UsageParts {
            probes,
            spawner,
            scratch_root,
            threads,
            rejects,
            clock,
            encoder,
        } = parts;
        let vendors: Vec<_> = probes.iter().map(|p| (p.key(), p.policy())).collect();
        let book = UsageBook::new(&vendors);
        let keys = book.keys();
        let runs = keys
            .iter()
            .map(|key| (key.clone(), Runs::default()))
            .collect();
        let finished = keys
            .into_iter()
            .map(|key| (key, tokio::sync::watch::channel(0).0))
            .collect();
        let (wake, wakes) = mpsc::sync_channel(1);
        let service = Arc::new_cyclic(|me| Self {
            desk: Mutex::new(Desk { book, runs }),
            watch: UsageWatch::new(),
            probes,
            spawner,
            scratch_root,
            threads,
            rejects,
            save_lock: Mutex::new(()),
            finished,
            clock,
            encoder,
            wake,
            reply_wait,
            me: me.clone(),
        });
        (service, wakes)
    }

    /// 저장된 거절 기한을 되살린다 — 조립이 연결을 받기 전에 한 번 부른다. 불러온 수와 되살린 수를 info 로 남기고
    /// 되살린 수를 돌려준다. 되살린 칸의 발행은 빚으로 남는다 — 구독되면 스케줄러가 낸다.
    pub fn restore_rejects(&self, entries: &[RejectEntry]) -> usize {
        let restored = {
            let mut desk = self.desk();
            let now = self.now();
            desk.book.restore_rejects(entries, now)
        };
        tracing::info!(
            loaded = entries.len(),
            restored,
            "저장된 사용량 거절 기한을 되살렸다"
        );
        restored
    }

    /// 줍기 관측 한 건 — pump 스레드가 그 자리에서 부른다(쌓지 않는다). 바뀐 것이 없으면 발행도 깨우기도 없다.
    /// 책에 칸이 없는 벤더면 아무것도 안 한다.
    pub fn observe(&self, obs: &UsageObservation) {
        let key = account_key(obs.vendor);
        let (applied, coalesce) = {
            let mut desk = self.desk();
            // 시각은 책 락 안에서 읽는다 — 두 pump 스레드의 적용 순서와 시각 순서가 어긋나지 않는다.
            let now = self.now();
            let applied = desk.book.apply_passive(obs, now);
            let coalesce = if applied.changed {
                desk.book.coalesce_passive(&key, now)
            } else {
                None
            };
            (applied, coalesce)
        };
        let deferred = matches!(coalesce, Some(Coalesce::Deferred));
        if let Some(Coalesce::PublishNow(sheet)) = coalesce {
            self.publish(key.vendor, &sheet);
        }
        if applied.next_auto_changed || deferred {
            self.wake_scheduler();
        }
    }

    /// `UsageSubscribe` — 그 연결의 구독 집합을 통째로 바꾼다(빈 집합 = 해제). 한 연결의 교체는 도착순으로 부른다.
    ///
    /// 차례: ① 요청 집합의 벤더마다 시각 평가 — 새긴 것(래치·거절 끝)이 있으면 그 벤더의 **지금** 구독자 전부에게
    /// 한 장 ② 명부 교체 — 새로 든 벤더마다 첫 한 장을 이 연결에만 ③ 스케줄러 깨우기(첫 한 장은 빚을 갚지 않고,
    /// 새로 든 벤더의 자동 조회 기한이 이미 지났을 수 있다). 끊긴 뒤 늦게 든 교체는 버린다 — 칸을 되살리지 않고
    /// 그 연결에는 아무것도 보내지 않는다(① 의 래치 발행은 그 벤더의 다른 구독자에게 나간다). 책에 칸이 없는
    /// 벤더는 명부 집합에만 들고 한 장이 없다.
    pub fn replace_subscription(&self, conn: ConnId, vendors: BTreeSet<UsageVendorKey>) {
        let latched: Vec<(UsageVendorKey, UsageLimitSnapshot)> = {
            let mut desk = self.desk();
            let now = self.now();
            vendors
                .iter()
                .filter_map(|&vendor| {
                    let key = account_key(vendor);
                    if !desk.book.eval_time(&key, now) {
                        return None;
                    }
                    desk.book
                        .broadcast_sheet(&key, now)
                        .map(|sheet| (vendor, sheet))
                })
                .collect()
        };
        for (vendor, sheet) in &latched {
            self.publish(*vendor, sheet);
        }

        // 명부 락 안에서 불린다(명부 → 책) — 책의 메모리 일만 한다.
        let replaced = self.watch.replace(conn, vendors, |added| {
            let mut desk = self.desk();
            let now = self.now();
            added
                .iter()
                .filter_map(|&vendor| {
                    let key = account_key(vendor);
                    desk.book.eval_time(&key, now);
                    desk.book.snapshot(&key, now)
                })
                .collect()
        });
        let Some(Replaced {
            first_sheets,
            target,
        }) = replaced
        else {
            tracing::debug!(conn, "끊긴 연결의 사용량 구독 교체가 늦게 왔다 — 버린다");
            return;
        };
        for sheet in &first_sheets {
            deliver(sheet, vec![target.clone()], self.encoder.as_ref());
        }
        self.wake_scheduler();
    }

    /// 연결이 붙었다(`on_connect`) — 빈 집합의 명부 칸([`UsageWatch::attach`]).
    pub fn attach(&self, conn: ConnId, outlet: impl UsageOutlet + 'static) {
        self.watch.attach(conn, outlet);
    }

    /// 연결이 끊겼다(`on_disconnect`) — 명부 칸을 지우고 그 출구를 **반환 전에** 거둔다([`UsageWatch::detach`]).
    /// 반환 = 칸이 있었다.
    pub fn detach(&self, conn: ConnId) -> bool {
        self.watch.detach(conn)
    }

    /// ⟳(연결 태스크)의 요청 하나(§1-4 요청 표) — `None` = 칸이 없는 벤더.
    ///
    /// 판정 전에 시각 평가를 새기고(래치·거절 끝 — 새겼으면 구독자 전부에게 한 장) 표의 행 순서대로 답한다:
    /// - 캐시·간격·거절 → 곧바로 지금 한 장(`Cached`). ★거절은 보이든 안 보이든 같은 갈래다★ — 한 장의 상태가
    ///   보이는 거절이면 `Rejected{retry_in_secs}` 이고, 값이 와서 `Ready` 로 보이는 동안이면 조용한 캐시다(§3 #88 ⓑ).
    /// - 시작 → 「갱신 중」 한 장을 구독자 전부에게 낸 뒤 조회를 띄운다 · 합류 → 진행 중인 조회에 붙는다. 둘 다 그
    ///   조회의 끝을 `REPLY_WAIT_MAX` 까지 기다린 뒤 지금 한 장을 답한다 — 기다린 끝 이후에 성공한 조회가 있으면
    ///   `Fresh`, 아니면 `Cached`(실패 → 실패 상태 · 상한 초과 → `in_flight = true` 이고 조회는 계속된다 — 끝나면
    ///   구독자에게 방송으로 닿는다).
    pub async fn request(&self, vendor: UsageVendorKey, kind: RequestKind) -> Option<UsageAnswer> {
        let key = account_key(vendor);
        let target = match self.open(&key, kind, None)? {
            Opened::Answer(answer) => return Some(answer),
            Opened::Wait(target) => target,
        };
        // 판정 뒤에 받아도 끝을 놓치지 않는다 — `wait_for` 는 지금 값부터 본다.
        if let Some(done) = self.finished.get(&key) {
            let mut finished = done.subscribe();
            // 상한 초과든 끝이든 지금 책으로 답한다 — 초과의 답은 `in_flight` 를 싣는다.
            let _ =
                tokio::time::timeout(self.reply_wait, finished.wait_for(|&seen| seen >= target))
                    .await;
        }
        self.close(&key, target)
    }

    /// 버스(`usage.*` — blocking 풀 스레드)의 요청 하나 — 판정·답은 [`UsageService::request`] 와 같다.
    ///
    /// 기다림은 std 채널의 `recv_timeout` 이다 — ★tokio 타이머를 쓰지 않는다★: 종료 중 런타임이 시간 드라이버를
    ///   내리면 타이머 poll 이 패닉할 수 있고 릴리즈에서는 곧 abort 다(§3 #17).
    pub fn request_blocking(
        &self,
        vendor: UsageVendorKey,
        kind: RequestKind,
    ) -> Option<UsageAnswer> {
        let key = account_key(vendor);
        let (joined, woken) = mpsc::sync_channel(1);
        let target = match self.open(&key, kind, Some(joined))? {
            Opened::Answer(answer) => return Some(answer),
            Opened::Wait(target) => target,
        };
        // 깸이든 상한 초과든 지금 책으로 답한다 — 초과의 답은 `in_flight` 를 싣는다.
        let _ = woken.recv_timeout(self.reply_wait);
        self.close(&key, target)
    }

    /// 요청의 앞 몫 — 책 락 안에서 시각 평가·판정·(기다리면) 합류자 등록을 하고, 락을 놓은 뒤 발행 → 조회 기동 →
    /// 깨우기. `joined` = 버스 요청의 깨움 송신단(기다릴 때만 등록한다).
    fn open(
        &self,
        key: &UsageKey,
        kind: RequestKind,
        joined: Option<SyncSender<()>>,
    ) -> Option<Opened> {
        let (judgment, sheet, opened) = {
            let mut guard = self.desk();
            let desk = &mut *guard;
            let now = self.now();
            let latched = desk.book.eval_time(key, now);
            let judgment = desk.book.judge(key, kind, now)?;
            let sheet = if latched || judgment == Judgment::Start {
                desk.book.broadcast_sheet(key, now)
            } else {
                None
            };
            let opened = match judgment {
                Judgment::Cached | Judgment::Rejected => {
                    let snapshot = match &sheet {
                        Some(sheet) => sheet.clone(),
                        None => desk.book.snapshot(key, now)?,
                    };
                    Opened::Answer(UsageAnswer {
                        snapshot,
                        served: UsageServed::Cached,
                    })
                }
                Judgment::Start | Judgment::Join => {
                    let runs = desk.runs.entry(key.clone()).or_default();
                    if let Some(joined) = joined {
                        runs.blocking.push(joined);
                    }
                    Opened::Wait(runs.finished.saturating_add(1))
                }
            };
            (judgment, sheet, opened)
        };
        if let Some(sheet) = &sheet {
            self.publish(key.vendor, sheet);
        }
        if judgment == Judgment::Start {
            self.start_probe(key.clone());
        }
        if sheet.is_some() {
            self.wake_scheduler();
        }
        Some(opened)
    }

    /// 요청의 뒤 몫 — 지금 한 장을 뜬다(그 사이 선 래치가 있으면 새겨 구독자 전부에게도). `target` = 기다린 끝 번호.
    fn close(&self, key: &UsageKey, target: u64) -> Option<UsageAnswer> {
        let (answer, latched) = {
            let mut guard = self.desk();
            let desk = &mut *guard;
            let now = self.now();
            let latched = if desk.book.eval_time(key, now) {
                desk.book.broadcast_sheet(key, now)
            } else {
                None
            };
            let snapshot = match &latched {
                Some(sheet) => sheet.clone(),
                None => desk.book.snapshot(key, now)?,
            };
            let fresh = desk
                .runs
                .get(key)
                .is_some_and(|runs| runs.last_ok >= target);
            let served = if fresh {
                UsageServed::Fresh
            } else {
                UsageServed::Cached
            };
            (UsageAnswer { snapshot, served }, latched)
        };
        if let Some(sheet) = &latched {
            self.publish(key.vendor, sheet);
            self.wake_scheduler();
        }
        Some(answer)
    }

    /// 스케줄러 한 번의 계획을 나른다 — 한 장들을 구독자 전부에게 낸 뒤 시작한 조회를 띄운다(§1-4 경로 표 ·
    /// 「갱신 중」 한 장은 `plan.publish` 에 이미 들었다). 돌려주는 값 = `plan.sleep`.
    ///
    /// ★계획을 뜬 책 락을 놓은 뒤 부른다★ — 락을 쥔 채 부르면 교착이다(발행이 명부를, 못 띄운 조회의 가드가 책을
    ///   잡는다). 계획의 조회는 책이 이미 `in_flight` 를 세웠으므로 여기서 반드시 띄운다(안 띄우면 그 칸은 영영
    ///   진행 중이다).
    fn carry_out(&self, plan: TickPlan) -> Duration {
        let TickPlan {
            start,
            publish,
            sleep,
        } = plan;
        for (key, sheet) in &publish {
            self.publish(key.vendor, sheet);
        }
        for key in start {
            self.start_probe(key);
        }
        sleep
    }

    /// 책이 이미 `in_flight` 를 세운 조회 하나를 띄운다 — [`Judgment::Start`]·[`TickPlan::start`] 뒤, 그 책 락을
    /// 놓고 부른다. 논블록이다.
    ///
    /// ★끝은 반드시 한 번 온다★ — 완료 가드를 띄우기 **전에** 만들어 칸에 두고, 일은 돌기 시작할 때 칸에서 꺼내
    ///   간다. 스레드를 못 띄우면 여기서 꺼내 놓고(띄우기 실패로 닫힘) · 조회가 패닉하면 풀리며 놓인다(시험 빌드.
    ///   릴리즈는 abort). 가드는 하나라 어느 쪽이 꺼내든 닫힘은 한 번이다. 조회는 늦어도 마감 + `KILL_WAIT` + 임시
    ///   폴더 지우기 재시도(약 0.2초) 안에 돌아온다([`UsageProbe::query`] 의 계약).
    fn start_probe(&self, key: UsageKey) {
        let vendor = key.vendor;
        let mut guard = ProbeGuard::new(self.me.clone(), key);
        let Some(probe) = self.probe_for(vendor) else {
            // 칸은 조회기로만 서므로 오지 않는 갈래다 — 오면 진행 중으로 남기지 않고 닫는다.
            tracing::warn!(
                vendor = vendor.as_str(),
                "조회기가 없는 칸의 조회가 시작됐다 — 곧바로 닫는다"
            );
            guard.outcome = Err(ProbeError::Unsupported.into());
            return;
        };
        // 못 띄운 일을 스레드 쪽이 놓는지에 기대지 않는다 — `Ok` 가 올 때까지 가드는 이쪽이 꺼낼 수 있다.
        let slot = Arc::new(Mutex::new(Some(guard)));
        let handed = slot.clone();
        let spawner = self.spawner.clone();
        let scratch_root = self.scratch_root.clone();
        let run = Box::new(move || {
            let Some(mut guard) = take_guard(&handed) else {
                return;
            };
            guard.outcome = Err(ProbeError::Io("사용량 조회가 답 없이 끝났다".to_owned()).into());
            // 시한이 넘치면 지금 마감 — 조회기가 곧바로 `Timeout` 으로 돌아온다(패닉하지 않는다).
            let deadline = Instant::now()
                .checked_add(probe.policy().timeout)
                .unwrap_or_else(Instant::now);
            guard.outcome = probe.query(&ProbeEnv {
                spawner: spawner.as_ref(),
                deadline,
                scratch_root: &scratch_root,
            });
        });
        if let Err(e) = self.threads.spawn(run) {
            // 기본 결과(띄우기 실패)로 닫힌다 — 칸 락 밖에서.
            drop(take_guard(&slot));
            tracing::warn!(
                vendor = vendor.as_str(),
                error = %e,
                "사용량 조회 스레드를 못 띄웠다 — 조회를 실패로 닫았다"
            );
        }
    }

    /// 조회 하나를 닫는다 — [`ProbeGuard`] 만 부른다(§1-4 「조회 끝」). 책 락 안에서 결과 적용 · 끝 번호 · 한 장 ·
    /// 합류자 꺼냄 → 락 밖에서 발행 → 스케줄러 깨우기 → 합류자 깨우기 → (거절 기한이 바뀌었으면) 저장. 발행은 바뀐
    /// 것이 없어도 한다(`in_flight` 가 풀렸다). 저장이 맨 뒤라 파일 I/O 가 답을 늦추지 않는다.
    fn finish_probe(&self, key: &UsageKey, outcome: Result<UsageObservation, ProbeFailure>) {
        let ok = outcome.is_ok();
        let (applied, sheet, finished, joiners) = {
            let mut guard = self.desk();
            let desk = &mut *guard;
            let now = self.now();
            let applied = desk.book.finish_probe(key, outcome, now);
            let sheet = desk.book.broadcast_sheet(key, now);
            let runs = desk.runs.entry(key.clone()).or_default();
            runs.finished = runs.finished.saturating_add(1);
            if ok {
                runs.last_ok = runs.finished;
            }
            (
                applied,
                sheet,
                runs.finished,
                std::mem::take(&mut runs.blocking),
            )
        };
        if let Some(sheet) = &sheet {
            self.publish(key.vendor, sheet);
        }
        self.wake_scheduler();
        if let Some(done) = self.finished.get(key) {
            // 두 끝의 알림이 락 밖에서 뒤바뀌어도 값이 되감기지 않게 — 큰 쪽만 남긴다.
            done.send_if_modified(|seen| {
                let newer = finished > *seen;
                if newer {
                    *seen = finished;
                }
                newer
            });
        }
        for joiner in joiners {
            // 용량 1 에 한 번만 보낸다 — 실패 = 기다리던 요청이 이미 떠났다.
            let _ = joiner.try_send(());
        }
        if applied.reject_changed {
            self.save_rejects();
        }
    }

    /// 거절 기한을 전량 저장한다 — `save_lock` 안에서 책을 다시 떠 쓴다(§1-4 「거절 저장」). 파일 I/O 는 책 락 밖이다.
    /// 저장소는 실패하지 않는다(쓰기 실패는 저장소가 warn 하고 계속).
    fn save_rejects(&self) {
        let _serial = self
            .save_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let entries = {
            let desk = self.desk();
            let now = self.now();
            desk.book.reject_entries(now)
        };
        self.rejects.save(&entries);
    }

    /// 빚을 갚은 한 장을 그 벤더의 지금 구독자 전부에게 나른다. ★어느 락도 쥐지 않은 채 부른다★.
    fn publish(&self, vendor: UsageVendorKey, sheet: &UsageLimitSnapshot) {
        deliver(sheet, self.watch.targets(vendor), self.encoder.as_ref());
    }

    fn wake_scheduler(&self) {
        // `Full` = 깸 하나가 이미 대기 중이라 같은 뜻이다 · `Disconnected` = 받는 스케줄러가 없다. 어느 쪽이든
        //   깨우는 쪽(pump 스레드 포함)을 막지 않는다.
        let _ = self.wake.try_send(());
    }

    fn probe_for(&self, vendor: UsageVendorKey) -> Option<&'static dyn UsageProbe> {
        self.probes.iter().copied().find(|p| p.key() == vendor)
    }

    fn now(&self) -> Now {
        Now {
            mono: self.clock.mono(),
            wall: self.clock.wall(),
        }
    }

    fn desk(&self) -> MutexGuard<'_, Desk> {
        // poison 을 견딘다 — 여기서 패닉하면 부른 pump 스레드가 죽는다. 책의 메서드는 패닉하지 않으므로 poison 은
        //   debug·시험에서만 선다(릴리즈는 `panic = "abort"`).
        self.desk.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// 조회 하나의 끝을 지는 가드 — 놓이는 자리가 어디든(조회를 마침 · 조회가 패닉해 풀림 · 스레드를 못 띄워 부른
/// 쪽이 꺼내 놓음) 한 번 [`UsageService::finish_probe`] 로 닫는다. 서비스가 이미 없으면(upgrade 실패) 결과를 버린다.
///
/// ★서비스를 `Weak` 로만 쥔다★ — 막힌 조회가 서비스를 붙들면 서비스 drop → 스케줄러 끝(깨우기 `Disconnected`)이
///   조회 시한까지 밀린다.
struct ProbeGuard {
    service: Weak<UsageService>,
    key: UsageKey,
    /// 놓일 때 적용할 결과 — 조회가 답하기 전에 놓이면 그 자리의 기본 실패다.
    outcome: Result<UsageObservation, ProbeFailure>,
}

impl ProbeGuard {
    fn new(service: Weak<UsageService>, key: UsageKey) -> Self {
        Self {
            service,
            key,
            outcome: Err(ProbeError::Spawn("사용량 조회 스레드를 띄우지 못했다".to_owned()).into()),
        }
    }
}

impl Drop for ProbeGuard {
    fn drop(&mut self) {
        // 가드는 한 번만 놓인다 — 자리채움은 읽히지 않는다.
        let outcome = std::mem::replace(&mut self.outcome, Err(ProbeError::Timeout.into()));
        let Some(service) = self.service.upgrade() else {
            tracing::debug!(
                vendor = self.key.vendor.as_str(),
                "사용량 서비스가 먼저 사라졌다 — 조회 결과를 버린다"
            );
            return;
        };
        service.finish_probe(&self.key, outcome);
    }
}

fn take_guard(slot: &Mutex<Option<ProbeGuard>>) -> Option<ProbeGuard> {
    slot.lock().unwrap_or_else(PoisonError::into_inner).take()
}

/// 벤더의 기본 계정 칸 — 계정 낱말은 지금 늘 기본값이다(`UsageAccountKey`).
fn account_key(vendor: UsageVendorKey) -> UsageKey {
    UsageKey {
        vendor,
        account: UsageAccountKey::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage_service::book::REFRESH_MIN_SPACING;
    use crate::usage_service::clock::ManualUsageClock;
    use crate::usage_service::watch::tests::{recording, words, Counting, Log};
    use crate::usage_service::watch::UsageFrame;
    use engram_dashboard_agent::backend::usage_probes;
    use engram_dashboard_agent::usage::{
        ProbeChild, ProbeCommand, UsagePolicy, UsageSource, WindowObs,
    };
    use engram_dashboard_protocol::{AgentBackendKind, UsageVendorState};
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::sync::mpsc::TryRecvError;
    use std::sync::{Barrier, OnceLock};

    const H: i64 = 3_600;
    pub(super) const T0: i64 = 1_900_000_000;
    /// 깨움을 기다려야 하는 시험의 요청 기다림 — 깨우기가 사라지면 그 시험이 이 상한에서 드러난다.
    const LONG_WAIT: Duration = Duration::from_secs(10);
    /// 시험 쪽의 모든 기다림의 상한 — 교착·깨우기 유실이 매달리는 대신 실패하게.
    const BOUND: Duration = Duration::from_secs(10);

    // 조회기 키로만 칸을 만든다 — 시험에도 벤더 리터럴을 두지 않는다.
    fn vendor(i: usize) -> UsageVendorKey {
        usage_probes()[i].key()
    }

    pub(super) fn key(i: usize) -> UsageKey {
        account_key(vendor(i))
    }

    pub(super) fn set(indices: &[usize]) -> BTreeSet<UsageVendorKey> {
        indices.iter().map(|&i| vendor(i)).collect()
    }

    /// 가짜 조회기 한 번의 대본.
    pub(super) enum Scripted {
        Answer(Result<UsageObservation, ProbeFailure>),
        Panic,
    }

    /// 가짜 조회기 — 조회마다 대본 하나를 받을 때까지 막힌다. 보낼 쪽([`Rig`])이 사라지면 시한 초과로 끝난다.
    /// 키·정책은 실 조회기에서 받는다. 조회기는 `'static` 이라 시험마다 하나씩 샌다.
    struct FakeProbe {
        key: UsageVendorKey,
        policy: UsagePolicy,
        queries: AtomicUsize,
        script: Mutex<Receiver<Scripted>>,
    }

    impl UsageProbe for FakeProbe {
        fn key(&self) -> UsageVendorKey {
            self.key
        }

        fn policy(&self) -> UsagePolicy {
            self.policy
        }

        fn query(&self, _env: &ProbeEnv<'_>) -> Result<UsageObservation, ProbeFailure> {
            self.queries.fetch_add(1, Ordering::SeqCst);
            let next = self
                .script
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .recv();
            match next {
                Ok(Scripted::Answer(result)) => result,
                Ok(Scripted::Panic) => panic!("시험 — 조회기 패닉"),
                Err(_) => Err(ProbeError::Timeout.into()),
            }
        }
    }

    /// 가짜 조회기는 자식을 안 띄운다 — 불리면 실패다.
    struct NoChildren;

    impl ProbeSpawner for NoChildren {
        fn spawn(
            &self,
            _cmd: &ProbeCommand,
            _deadline: Instant,
        ) -> Result<Box<dyn ProbeChild>, ProbeError> {
            Err(ProbeError::Spawn("시험 — 자식을 띄우지 않는다".to_owned()))
        }
    }

    #[derive(Default)]
    struct ThreadCounts {
        spawned: AtomicUsize,
        /// 일이 끝난 수 — 끝났다 = 완료 가드까지 놓였다.
        ended: AtomicUsize,
        panicked: AtomicUsize,
    }

    /// 조회 스레드 대역 — `refuse` 면 띄우지 않고 받은 일을 **쥔 채**(`kept`) `Err` 다. 못 띄운 일을 놓는지에
    /// 서비스가 기대지 않음을 재고, 쥔 일은 시험이 뒤늦게 돌려 볼 수 있다.
    #[derive(Default)]
    struct TestThreads {
        refuse: AtomicBool,
        kept: Mutex<Vec<Box<dyn FnOnce() + Send>>>,
        counts: Arc<ThreadCounts>,
    }

    impl ProbeThreads for TestThreads {
        fn spawn(&self, run: Box<dyn FnOnce() + Send>) -> io::Result<()> {
            if self.refuse.load(Ordering::SeqCst) {
                self.kept.lock().unwrap().push(run);
                return Err(io::Error::other("시험 — 스레드를 못 띄운다"));
            }
            self.counts.spawned.fetch_add(1, Ordering::SeqCst);
            let counts = self.counts.clone();
            std::thread::spawn(move || {
                if catch_unwind(AssertUnwindSafe(run)).is_err() {
                    counts.panicked.fetch_add(1, Ordering::SeqCst);
                }
                counts.ended.fetch_add(1, Ordering::SeqCst);
            });
            Ok(())
        }
    }

    /// 저장마다 받은 목록을 적는다.
    #[derive(Default)]
    struct SavingStore {
        saves: Mutex<Vec<Vec<RejectEntry>>>,
    }

    impl SavingStore {
        fn saves(&self) -> Vec<Vec<RejectEntry>> {
            self.saves.lock().unwrap().clone()
        }
    }

    impl RejectStore for SavingStore {
        fn load(&self) -> Vec<RejectEntry> {
            Vec::new()
        }

        fn save(&self, entries: &[RejectEntry]) {
            self.saves.lock().unwrap().push(entries.to_vec());
        }
    }

    pub(super) struct Rig {
        pub(super) service: Arc<UsageService>,
        pub(super) wakes: Receiver<()>,
        encoder: Arc<Counting>,
        probes: Vec<&'static FakeProbe>,
        scripts: Vec<mpsc::Sender<Scripted>>,
        threads: Arc<TestThreads>,
        store: Arc<SavingStore>,
    }

    /// 칸 = 실 조회기 앞에서부터 `count` 개(키·정책만 빌린 가짜).
    fn rig_full(clock: Arc<dyn UsageClock>, count: usize, reply_wait: Duration) -> Rig {
        let (probes, scripts): (Vec<&'static FakeProbe>, Vec<_>) = usage_probes()[..count]
            .iter()
            .map(|real| {
                let (tx, rx) = mpsc::channel();
                let fake: &'static FakeProbe = Box::leak(Box::new(FakeProbe {
                    key: real.key(),
                    policy: real.policy(),
                    queries: AtomicUsize::new(0),
                    script: Mutex::new(rx),
                }));
                (fake, tx)
            })
            .unzip();
        let encoder = Arc::new(Counting::default());
        let threads = Arc::new(TestThreads::default());
        let store = Arc::new(SavingStore::default());
        let (service, wakes) = UsageService::build(
            UsageParts {
                probes: probes
                    .iter()
                    .map(|&p| p as &'static dyn UsageProbe)
                    .collect(),
                spawner: Arc::new(NoChildren),
                scratch_root: std::env::temp_dir(),
                threads: threads.clone(),
                rejects: store.clone(),
                clock,
                encoder: encoder.clone(),
            },
            reply_wait,
        );
        Rig {
            service,
            wakes,
            encoder,
            probes,
            scripts,
            threads,
            store,
        }
    }

    pub(super) fn rig_with(clock: Arc<dyn UsageClock>, count: usize) -> Rig {
        rig_full(clock, count, LONG_WAIT)
    }

    pub(super) fn rig() -> (Rig, Arc<ManualUsageClock>) {
        let clock = Arc::new(ManualUsageClock::new(Duration::from_secs(100), T0));
        (rig_with(clock.clone(), usage_probes().len()), clock)
    }

    impl Rig {
        /// 붙이고 구독까지 — 첫 한 장과 깸은 비워 둔다.
        pub(super) fn subscriber(&self, conn: ConnId, indices: &[usize]) -> Arc<Log> {
            let (outlet, log) = recording();
            self.service.attach(conn, outlet);
            self.service.replace_subscription(conn, set(indices));
            log.frames.lock().unwrap().clear();
            self.drain_wakes();
            log
        }

        fn drain_wakes(&self) -> usize {
            self.wakes.try_iter().count()
        }

        fn revision(&self, i: usize) -> u64 {
            self.service.desk().book.revision(&key(i)).expect("아는 키")
        }

        fn wire(&self, i: usize) -> AgentBackendKind {
            self.sheet(i).vendor
        }

        fn sheet(&self, i: usize) -> UsageLimitSnapshot {
            let now = self.service.now();
            self.service
                .desk()
                .book
                .snapshot(&key(i), now)
                .expect("아는 키")
        }

        pub(super) fn answer(&self, i: usize, scripted: Scripted) {
            self.scripts[i].send(scripted).expect("가짜 조회기");
        }

        pub(super) fn queries(&self, i: usize) -> usize {
            self.probes[i].queries.load(Ordering::SeqCst)
        }

        /// 그 칸의 진행 중인 조회를 기다리는 요청 수 — 버스 + async.
        pub(super) fn joiners(&self, i: usize) -> usize {
            let blocking = self.service.desk().runs[&key(i)].blocking.len();
            blocking + self.service.finished[&key(i)].receiver_count()
        }

        pub(super) fn in_flight(&self, i: usize) -> bool {
            self.service
                .desk()
                .book
                .in_flight(&key(i))
                .expect("아는 키")
        }
    }

    pub(super) fn five(i: usize, pct: f64, reset: i64) -> UsageObservation {
        UsageObservation {
            vendor: vendor(i),
            five_hour: Some(WindowObs {
                used_pct: Some(pct),
                resets_at: Some(reset),
            }),
            weekly: None,
            model_scoped: None,
            plan: None,
            source: UsageSource::Passive,
            limits_unavailable: None,
        }
    }

    pub(super) fn far_reset() -> i64 {
        T0 + 5 * H
    }

    fn both(i: usize, five_pct: f64, weekly_pct: f64) -> UsageObservation {
        UsageObservation {
            weekly: Some(WindowObs {
                used_pct: Some(weekly_pct),
                resets_at: Some(T0 + 100 * H),
            }),
            ..five(i, five_pct, far_reset())
        }
    }

    pub(super) fn frames(log: &Log) -> Vec<UsageFrame> {
        log.frames.lock().unwrap().clone()
    }

    pub(super) fn last_pct(log: &Log) -> Option<f64> {
        frames(log)
            .last()
            .and_then(|f| f.snapshot.five_hour.as_ref())
            .and_then(|w| w.used_pct)
    }

    // ── 줍기 ──

    #[test]
    fn an_observed_change_goes_to_that_vendors_subscribers_only_and_a_repeat_sends_nothing() {
        let (rig, _clock) = rig();
        let a = rig.subscriber(1, &[0]);
        let b = rig.subscriber(2, &[1]);
        let c = rig.subscriber(3, &[]);

        rig.service.observe(&five(0, 40.0, far_reset()));
        let got = frames(&a);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].snapshot.revision, rig.revision(0));
        assert_eq!(&*got[0].json, words(&set(&[0])));
        assert_eq!((b.count(), c.count()), (0, 0));

        rig.service.observe(&five(0, 40.0, far_reset()));
        assert_eq!(a.count(), 1, "같은 값을 다시 주웠다 — 바뀜이 아니다");
    }

    #[test]
    fn without_subscribers_the_book_still_moves_and_nothing_is_encoded() {
        let (rig, _clock) = rig();
        let before = rig.revision(0);
        rig.service.observe(&five(0, 40.0, far_reset()));
        assert!(rig.revision(0) > before);
        assert_eq!(rig.encoder.calls(), 0);
    }

    #[test]
    fn a_change_inside_the_coalesce_window_waits_for_the_scheduler() {
        let (rig, clock) = rig();
        let a = rig.subscriber(1, &[0]);
        rig.service.observe(&five(0, 40.0, far_reset()));
        rig.drain_wakes();

        clock.advance_both(Duration::from_millis(500));
        rig.service.observe(&five(0, 41.0, far_reset()));
        assert_eq!(a.count(), 1, "1초 안의 두 번째 바뀜은 합친다");
        assert_eq!(rig.drain_wakes(), 1, "끝 발행은 스케줄러가 낸다");
    }

    #[test]
    fn an_observation_that_moves_the_next_auto_query_wakes_the_scheduler_once() {
        let (rig, clock) = rig();
        let a = rig.subscriber(1, &[0]);
        {
            let now = rig.service.now();
            let mut desk = rig.service.desk();
            let book = &mut desk.book;
            assert!(book.begin_probe(&key(0)));
            book.finish_probe(&key(0), Ok(both(0, 10.0, 10.0)), now);
            book.broadcast_sheet(&key(0), now);
        }

        clock.advance_both(Duration::from_secs(2));
        rig.service.observe(&five(0, 20.0, far_reset()));
        assert_eq!(
            (a.count(), rig.drain_wakes()),
            (1, 0),
            "한 창만 새로 섰다 — 다음 자동 조회는 그대로"
        );

        clock.advance_both(Duration::from_secs(2));
        let now = rig.service.now();
        let before = rig.service.desk().book.next_auto(&key(0), now);
        rig.service.observe(&both(0, 30.0, 30.0));
        assert_eq!(
            (a.count(), rig.drain_wakes()),
            (2, 1),
            "두 창이 새로 서 쿨타임 기점이 옮겨졌다(D12)"
        );
        assert_ne!(rig.service.desk().book.next_auto(&key(0), now), before);
    }

    #[test]
    fn an_observation_still_applies_after_the_book_lock_is_poisoned() {
        let (rig, _clock) = rig();
        let a = rig.subscriber(1, &[0]);
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _held = rig.service.desk.lock().unwrap();
            panic!("시험 — 책 락을 쥔 채 패닉");
        }));
        assert!(panicked.is_err());
        assert!(rig.service.desk.is_poisoned(), "시험이 poison 갈래를 탔다");

        rig.service.observe(&five(0, 40.0, far_reset()));
        assert_eq!(last_pct(&a), Some(40.0));
    }

    // 조회 시작(곧바로 갚을 빚)을 아직 안 냈으면 1초 안의 줍기가 그것까지 곧바로 낸다.
    #[test]
    fn a_passive_right_after_an_unpublished_probe_start_publishes_at_once() {
        let (rig, clock) = rig();
        let a = rig.subscriber(1, &[0]);
        rig.service.observe(&five(0, 40.0, far_reset()));
        assert!(rig.service.desk().book.begin_probe(&key(0)));

        clock.advance_both(Duration::from_millis(500));
        rig.service.observe(&five(0, 42.0, far_reset()));
        let got = frames(&a);
        assert_eq!(got.len(), 2);
        assert!(got[1].snapshot.in_flight);
        assert_eq!(last_pct(&a), Some(42.0));
        assert_eq!(got[1].snapshot.revision, rig.revision(0));
    }

    #[test]
    fn a_vendor_without_a_cell_gets_no_sheet_and_its_observation_does_nothing() {
        let clock = Arc::new(ManualUsageClock::new(Duration::from_secs(100), T0));
        let rig = rig_with(clock, 1);
        let (outlet, log) = recording();
        rig.service.attach(1, outlet);
        rig.service.replace_subscription(1, set(&[0, 1]));
        let got = frames(&log);
        assert_eq!(got.len(), 1, "칸이 있는 벤더만 첫 한 장");
        assert_eq!(&*got[0].json, words(&set(&[0, 1])), "집합은 요청 그대로");
        rig.drain_wakes();

        rig.service.observe(&five(1, 40.0, far_reset()));
        assert_eq!(log.count(), 1);
        assert_eq!(rig.drain_wakes(), 0);
    }

    // ── 구독 교체 ──

    #[test]
    fn a_latch_due_at_subscription_is_published_to_existing_subscribers_too() {
        let (rig, clock) = rig();
        let a = rig.subscriber(1, &[0]);
        rig.service.observe(&five(0, 40.0, T0 + 10));
        assert_eq!(a.count(), 1);
        clock.set_wall(T0 + 20);

        let (outlet, b) = recording();
        rig.service.attach(2, outlet);
        rig.service.replace_subscription(2, set(&[0]));

        let latched = frames(&a);
        assert_eq!(latched.len(), 2, "기존 구독자에게 래치 한 장");
        let sheet = &latched[1].snapshot;
        assert!(sheet.five_hour.as_ref().expect("창").expired);
        assert_eq!(sheet.revision, rig.revision(0));
        let first = frames(&b);
        assert_eq!(first.len(), 1, "새 구독자는 첫 한 장만");
        assert_eq!(first[0].snapshot, *sheet);
        assert_eq!(rig.drain_wakes(), 1);

        rig.service.replace_subscription(1, set(&[0]));
        rig.service.replace_subscription(2, set(&[0]));
        assert_eq!(
            (a.count(), b.count()),
            (2, 1),
            "같은 집합 교체 — 한 장도 없다"
        );
    }

    #[test]
    fn a_first_sheet_goes_to_the_new_subscriber_only_and_wakes_the_scheduler() {
        let (rig, _clock) = rig();
        let a = rig.subscriber(1, &[0]);
        rig.service.observe(&five(0, 40.0, far_reset()));
        let (outlet, b) = recording();
        rig.service.attach(2, outlet);
        rig.drain_wakes();

        rig.service.replace_subscription(2, set(&[0, 1]));
        assert_eq!(a.count(), 1);
        let first = frames(&b);
        assert_eq!(first.len(), 2, "새로 든 벤더마다 한 장");
        assert!(first.iter().all(|f| &*f.json == words(&set(&[0, 1]))));
        assert_eq!(rig.drain_wakes(), 1);
    }

    #[test]
    fn a_late_replace_after_detach_is_dropped() {
        let (rig, _clock) = rig();
        let (outlet, log) = recording();
        rig.service.attach(1, outlet);
        assert!(rig.service.detach(1));
        assert_eq!(log.revokes(), 1);
        rig.service.observe(&five(0, 40.0, far_reset()));
        rig.drain_wakes();

        rig.service.replace_subscription(1, set(&[0]));
        assert_eq!(log.count(), 0);
        assert_eq!(Arc::strong_count(&log), 1, "출구가 되살아나지 않았다");
        assert!(rig.service.watch.union().is_empty());
        assert_eq!(rig.drain_wakes(), 0);
        assert_eq!(rig.encoder.calls(), 0);
    }

    #[test]
    fn a_late_replace_after_detach_still_publishes_a_due_latch_to_the_other_subscribers() {
        let (rig, clock) = rig();
        let other = rig.subscriber(2, &[0]);
        let (outlet, gone) = recording();
        rig.service.attach(1, outlet);
        assert!(rig.service.detach(1));
        rig.service.observe(&five(0, 40.0, T0 + 10));
        clock.set_wall(T0 + 20);

        rig.service.replace_subscription(1, set(&[0]));
        let got = frames(&other);
        assert_eq!(got.len(), 2, "줍기 한 장 + 래치 한 장");
        assert!(got[1].snapshot.five_hour.as_ref().expect("창").expired);
        assert_eq!(gone.count(), 0, "끊긴 연결에는 아무것도 없다");
    }

    #[test]
    fn adding_a_vendor_while_another_latches_sends_the_latch_first_then_the_first_sheet() {
        let (rig, clock) = rig();
        let a = rig.subscriber(1, &[0]);
        rig.service.observe(&five(0, 40.0, T0 + 10));
        a.frames.lock().unwrap().clear();
        clock.set_wall(T0 + 20);

        rig.service.replace_subscription(1, set(&[0, 1]));
        let got = frames(&a);
        let order: Vec<_> = got
            .iter()
            .map(|f| (f.snapshot.vendor, f.json.to_string()))
            .collect();
        assert_eq!(
            order,
            [
                (rig.wire(0), words(&set(&[0]))),
                (rig.wire(1), words(&set(&[0, 1]))),
            ],
            "래치(교체 전 집합) 다음 첫 한 장(교체 뒤 집합)"
        );
        assert!(got[0].snapshot.five_hour.as_ref().expect("창").expired);
    }

    // ── 락 ──

    /// 처음 받을 때 서비스를 다시 부르는 출구 — 줍기·교체·자기 해제.
    struct CallsBack {
        service: Arc<OnceLock<Weak<UsageService>>>,
        calls: Arc<AtomicUsize>,
    }

    impl UsageOutlet for CallsBack {
        fn send(&self, _frame: &UsageFrame) {
            if self.calls.fetch_add(1, Ordering::SeqCst) > 0 {
                return;
            }
            let service = self.service.get().and_then(Weak::upgrade).expect("서비스");
            service.observe(&five(0, 77.0, far_reset()));
            service.replace_subscription(1, set(&[0, 1]));
            service.detach(1);
        }

        fn revoke(&self) {}
    }

    #[test]
    fn an_outlet_may_call_back_into_the_service() {
        // 교착이면 매달리는 대신 실패하도록 다른 스레드에서 돌린다.
        let (done_tx, done_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (rig, _clock) = rig();
            let slot = Arc::new(OnceLock::new());
            slot.set(Arc::downgrade(&rig.service)).expect("한 번");
            let calls = Arc::new(AtomicUsize::new(0));
            rig.service.attach(
                1,
                CallsBack {
                    service: slot,
                    calls: calls.clone(),
                },
            );
            rig.service.replace_subscription(1, set(&[0]));
            let union = rig.service.watch.union();
            done_tx.send((calls.load(Ordering::SeqCst), union)).unwrap();
        });
        let (calls, union) = done_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("교착 없이 끝난다");
        assert!(calls >= 1);
        assert!(union.is_empty(), "출구 안에서 부른 해제가 칸을 지웠다");
    }

    // ── 동시성 ──

    /// 읽을 때마다 단조 시간이 2초 흐르는 시계 — 바뀜마다 합침 창이 이미 닫혀 곧바로 나간다(이 시험엔 끝 발행을
    /// 낼 스케줄러가 없다). 벽시계는 멈춰 있어 래치가 서지 않는다.
    struct Ticking(AtomicU64);

    impl UsageClock for Ticking {
        fn mono(&self) -> Duration {
            Duration::from_secs(self.0.fetch_add(2, Ordering::SeqCst))
        }

        fn wall(&self) -> i64 {
            T0
        }
    }

    /// 연결 둘이 구독을 바꾸는 동안 줍기 둘이 두 벤더를 바꾸는 판 하나 — 겹침은 판마다 달라지고 일의 양은 정해져
    /// 있다. 연결 0 은 정해진 만큼 바꾸다 비우고 판이 끝난 뒤 다시 구독한다. 연결 1 은 정해진 만큼 바꾼 뒤 비우고,
    /// 줍기 0 의 마지막 바뀜 둘과 **같은 순간에** 다시 구독한다 — 마지막 값이 발행으로 닿을지 첫 한 장으로 닿을지는
    /// 그 겹침이 정한다(TRD §3 #57 의 두 차례가 맞물리는 자리). 돌려주는 것 = 어긋난 칸마다 한 줄.
    fn race_trial(trial: usize) -> Vec<String> {
        const OBSERVERS: usize = 2;
        const OBSERVATIONS: usize = 16;
        const REPLACES: usize = 8;
        const CHURN: usize = 24;
        let rig = rig_with(Arc::new(Ticking(AtomicU64::new(100))), 2);
        let logs: Vec<Arc<Log>> = (0..2)
            .map(|conn| {
                let (outlet, log) = recording();
                rig.service.attach(conn, outlet);
                log
            })
            .collect();
        let cycle: [&[usize]; 5] = [&[0], &[0, 1], &[1], &[], &[1, 0]];
        let barrier = Arc::new(Barrier::new(2 + OBSERVERS));
        let endgame = Arc::new(Barrier::new(2));

        let mut threads = Vec::new();
        {
            let (service, barrier) = (rig.service.clone(), barrier.clone());
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                for round in 0..CHURN {
                    service.replace_subscription(0, set(cycle[round % cycle.len()]));
                }
                service.replace_subscription(0, set(&[]));
            }));
        }
        {
            let (service, barrier, endgame) =
                (rig.service.clone(), barrier.clone(), endgame.clone());
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                for round in 0..REPLACES {
                    service.replace_subscription(1, set(cycle[(round + 1) % cycle.len()]));
                }
                service.replace_subscription(1, set(&[]));
                endgame.wait();
                service.replace_subscription(1, set(&[0, 1]));
            }));
        }
        for observer in 0..OBSERVERS {
            let (service, barrier, endgame) =
                (rig.service.clone(), barrier.clone(), endgame.clone());
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                for n in 0..OBSERVATIONS {
                    let pct = ((observer * 7_919 + n * 13) % 1_000) as f64 / 10.0;
                    service.observe(&five(n % 2, pct, far_reset()));
                }
                if observer == 0 {
                    // 앞의 값은 모두 100 아래라 늘 바뀜이다.
                    endgame.wait();
                    for i in 0..2 {
                        service.observe(&five(i, 100.0, far_reset()));
                    }
                }
            }));
        }
        for thread in threads {
            thread.join().expect("시험 스레드");
        }
        rig.service.replace_subscription(0, set(&[0, 1]));

        let mut misses = Vec::new();
        for i in 0..2 {
            let (wire, last) = (rig.wire(i), rig.revision(i));
            for (conn, log) in logs.iter().enumerate() {
                let highest = frames(log)
                    .iter()
                    .filter(|f| f.snapshot.vendor == wire)
                    .map(|f| f.snapshot.revision)
                    .max();
                if last == 0 || highest != Some(last) {
                    misses.push(format!(
                        "판 {trial} · 연결 {conn} · 벤더 {i}: 받은 최고 {highest:?} · 책 {last}"
                    ));
                }
            }
        }
        misses
    }

    /// 판마다 그 연결이 받은 최고 revision = 책의 마지막 revision. 올바른 코드에서는 겹침과 무관하게 선다 — 시계가
    /// 읽을 때마다 2초 흘러 바뀜마다 곧바로 나가고, 벽시계가 멈춰 래치가 없다.
    #[test]
    fn racing_replaces_and_observations_leave_each_subscriber_at_the_final_revision() {
        const TRIALS: usize = 2_000;
        // 락 차례가 깨져 교착하면 매달리는 대신 실패하도록 다른 스레드에서 돌린다.
        let (done_tx, done_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let misses: Vec<String> = (0..TRIALS).flat_map(race_trial).collect();
            done_tx.send(misses).unwrap();
        });
        let misses = done_rx
            .recv_timeout(Duration::from_secs(120))
            .expect("시험 스레드가 끝나지 않았다(교착 또는 패닉)");
        let total = misses.len();
        assert!(
            misses.is_empty(),
            "{total}건 어긋남 — 처음: {:?}",
            misses.first()
        );
    }

    // ── 거절 복원 ──

    #[test]
    fn restored_rejects_are_counted_and_the_cell_reads_rejected() {
        let (rig, _clock) = rig();
        let entries = [
            RejectEntry {
                key: key(0),
                until_epoch_s: T0 + 600,
            },
            RejectEntry {
                key: key(1),
                until_epoch_s: T0 - 1,
            },
        ];
        assert_eq!(
            rig.service.restore_rejects(&entries),
            1,
            "지난 항목은 버린다"
        );
        let now = rig.service.now();
        let sheet = rig
            .service
            .desk()
            .book
            .snapshot(&key(0), now)
            .expect("아는 키");
        assert!(matches!(sheet.state, UsageVendorState::Rejected { .. }));
    }

    // ── 요청·조회 ──

    /// 기다림 상한을 재는 시험의 요청 기다림.
    const SHORT_WAIT: Duration = Duration::from_millis(200);

    pub(super) fn active(i: usize, pct: f64) -> UsageObservation {
        UsageObservation {
            source: UsageSource::Active,
            ..five(i, pct, far_reset())
        }
    }

    pub(super) fn ok(obs: UsageObservation) -> Scripted {
        Scripted::Answer(Ok(obs))
    }

    /// 조건이 설 때까지 짧게 쉬며 본다 — [`BOUND`] 를 넘으면 실패한다.
    pub(super) fn wait_until(what: &str, mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + BOUND;
        while !ready() {
            assert!(Instant::now() < deadline, "{what} — 기다림 상한");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// 버스 요청을 다른 스레드에서 — 받는 쪽([`answered`])의 상한이 매달림을 실패로 만든다.
    pub(super) fn bus_request(
        service: &Arc<UsageService>,
        i: usize,
        kind: RequestKind,
    ) -> Receiver<Option<UsageAnswer>> {
        let (tx, rx) = mpsc::channel();
        let service = service.clone();
        std::thread::spawn(move || {
            let _ = tx.send(service.request_blocking(vendor(i), kind));
        });
        rx
    }

    pub(super) fn answered(rx: &Receiver<Option<UsageAnswer>>) -> UsageAnswer {
        rx.recv_timeout(BOUND)
            .expect("요청이 상한 안에 끝난다")
            .expect("아는 벤더")
    }

    pub(super) fn pct(sheet: &UsageLimitSnapshot) -> Option<f64> {
        sheet.five_hour.as_ref().and_then(|w| w.used_pct)
    }

    fn failed_kind(sheet: &UsageLimitSnapshot) -> Option<String> {
        match &sheet.state {
            UsageVendorState::Failed { detail, .. } => detail.as_ref().map(|d| d.kind.clone()),
            _ => None,
        }
    }

    fn ended(rig: &Rig) -> usize {
        rig.threads.counts.ended.load(Ordering::SeqCst)
    }

    #[test]
    fn a_probe_thread_that_cannot_start_closes_the_probe_as_a_spawn_failure_and_publishes() {
        let (rig, _clock) = rig();
        let log = rig.subscriber(1, &[0]);
        rig.threads.refuse.store(true, Ordering::SeqCst);

        let started = Instant::now();
        let answer = rig
            .service
            .request_blocking(vendor(0), RequestKind::Refresh)
            .expect("아는 벤더");
        assert!(
            started.elapsed() < LONG_WAIT,
            "닫힌 조회의 합류자는 곧바로 깬다"
        );
        assert_eq!(answer.served, UsageServed::Cached);
        assert!(!answer.snapshot.in_flight);
        assert_eq!(failed_kind(&answer.snapshot).as_deref(), Some("spawn"));
        assert!(!rig.in_flight(0), "진행 중으로 남지 않는다");
        assert_eq!(rig.queries(0), 0);

        let got = frames(&log);
        assert_eq!(got.len(), 2, "시작 한 장 + 끝 한 장");
        assert!(got[0].snapshot.in_flight);
        assert!(!got[1].snapshot.in_flight);
        assert_eq!(got[1].snapshot.revision, answer.snapshot.revision);

        let kept: Vec<_> = rig.threads.kept.lock().unwrap().drain(..).collect();
        assert_eq!(kept.len(), 1, "못 띄운 일은 대역이 쥐고 있었다");
        for job in kept {
            job();
        }
        assert_eq!(log.count(), 2, "뒤늦게 돈 일은 아무것도 안 한다");
        assert_eq!(rig.queries(0), 0);
        assert_eq!(rig.service.desk().runs[&key(0)].finished, 1, "닫힘은 한 번");
    }

    #[test]
    fn concurrent_requests_join_one_probe_and_all_get_its_answer() {
        let (rig, _clock) = rig();
        let first = bus_request(&rig.service, 0, RequestKind::Refresh);
        wait_until("조회가 떴다", || rig.queries(0) == 1);

        let buses: Vec<_> = [RequestKind::Get, RequestKind::Refresh, RequestKind::Get]
            .into_iter()
            .map(|kind| bus_request(&rig.service, 0, kind))
            .collect();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_time()
            .build()
            .expect("런타임");
        let tasks: Vec<_> = [RequestKind::Refresh, RequestKind::Get]
            .into_iter()
            .map(|kind| {
                let service = rig.service.clone();
                runtime.spawn(async move { service.request(vendor(0), kind).await })
            })
            .collect();
        wait_until("여섯 요청이 모두 합류했다", || {
            rig.joiners(0) == 6
        });

        rig.answer(0, ok(active(0, 33.0)));
        let mut answers = vec![answered(&first)];
        answers.extend(buses.iter().map(answered));
        for task in tasks {
            let answer = runtime
                .block_on(async { tokio::time::timeout(BOUND, task).await })
                .expect("상한 안에 끝난다")
                .expect("요청 태스크")
                .expect("아는 벤더");
            answers.push(answer);
        }
        assert_eq!(
            (
                rig.queries(0),
                rig.threads.counts.spawned.load(Ordering::SeqCst)
            ),
            (1, 1),
            "조회 하나"
        );
        for answer in &answers {
            assert_eq!(answer.served, UsageServed::Fresh);
            assert_eq!(pct(&answer.snapshot), Some(33.0));
            assert!(!answer.snapshot.in_flight);
        }
    }

    #[test]
    fn a_request_gives_up_after_the_reply_wait_with_the_cache_and_in_flight_and_the_probe_goes_on()
    {
        let clock = Arc::new(ManualUsageClock::new(Duration::from_secs(100), T0));
        let rig = rig_full(clock, 2, SHORT_WAIT);
        let log = rig.subscriber(1, &[0]);

        let started = Instant::now();
        let answer = rig
            .service
            .request_blocking(vendor(0), RequestKind::Refresh)
            .expect("아는 벤더");
        assert!(started.elapsed() >= SHORT_WAIT);
        assert!(started.elapsed() < LONG_WAIT);
        assert_eq!(answer.served, UsageServed::Cached);
        assert!(answer.snapshot.in_flight, "조회는 계속된다");

        // async 도 같은 상한 — 진행 중인 조회에 합류해 기다리다 포기한다.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("런타임");
        let answer = runtime
            .block_on(rig.service.request(vendor(0), RequestKind::Get))
            .expect("아는 벤더");
        assert_eq!(answer.served, UsageServed::Cached);
        assert!(answer.snapshot.in_flight);
        assert_eq!(rig.queries(0), 1);

        rig.answer(0, ok(active(0, 55.0)));
        wait_until("조회 끝이 구독자에게 닿는다", || {
            last_pct(&log) == Some(55.0)
        });
        assert!(!rig.in_flight(0));
        assert!(!frames(&log).last().expect("한 장").snapshot.in_flight);
    }

    #[test]
    fn a_probe_outliving_the_service_drops_its_result_without_a_panic() {
        let (rig, _clock) = rig();
        assert!(rig.service.desk().book.begin_probe(&key(0)));
        rig.service.start_probe(key(0));
        wait_until("조회가 떴다", || rig.queries(0) == 1);

        let Rig {
            service,
            wakes,
            scripts,
            threads,
            ..
        } = rig;
        let weak = Arc::downgrade(&service);
        drop(service);
        assert!(
            weak.upgrade().is_none(),
            "조회 스레드는 서비스를 붙들지 않는다"
        );
        wakes.try_iter().count();
        assert!(
            matches!(wakes.try_recv(), Err(TryRecvError::Disconnected)),
            "막힌 조회가 도는 중에도 스케줄러가 서비스의 끝을 본다"
        );

        scripts[0].send(ok(active(0, 10.0))).expect("가짜 조회기");
        wait_until("조회 스레드가 끝났다", || {
            threads.counts.ended.load(Ordering::SeqCst) == 1
        });
        assert_eq!(threads.counts.panicked.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_rejection_is_saved_once_and_a_result_that_leaves_the_list_alone_saves_nothing() {
        let (rig, _clock) = rig();
        let run = |scripted: Scripted| {
            let before = ended(&rig);
            assert!(rig.service.desk().book.begin_probe(&key(0)));
            rig.service.start_probe(key(0));
            rig.answer(0, scripted);
            wait_until("조회 끝", || ended(&rig) == before + 1);
        };
        let limited = || {
            Scripted::Answer(Err(ProbeError::RateLimited {
                retry_after: Some(Duration::from_secs(600)),
            }
            .into()))
        };

        run(limited());
        assert_eq!(
            rig.store.saves(),
            [vec![RejectEntry {
                key: key(0),
                until_epoch_s: T0 + 600,
            }]]
        );

        run(limited());
        run(Scripted::Answer(Err(ProbeError::Timeout.into())));
        assert_eq!(
            rig.store.saves().len(),
            1,
            "목록이 그대로면 다시 쓰지 않는다"
        );

        run(ok(active(0, 20.0)));
        let saved = rig.store.saves();
        assert_eq!(saved.len(), 2, "성공이 거절 기한을 지웠다");
        assert!(saved[1].is_empty());
    }

    #[test]
    fn a_refresh_during_a_rejection_answers_from_the_cache_without_a_probe_seen_or_hidden() {
        let (rig, _clock) = rig();
        rig.service.restore_rejects(&[RejectEntry {
            key: key(0),
            until_epoch_s: T0 + 600,
        }]);

        let seen = rig
            .service
            .request_blocking(vendor(0), RequestKind::Refresh)
            .expect("아는 벤더");
        assert!(matches!(
            seen.snapshot.state,
            UsageVendorState::Rejected { .. }
        ));
        assert_eq!(seen.served, UsageServed::Cached);

        rig.service.observe(&five(0, 40.0, far_reset()));
        let hidden = rig
            .service
            .request_blocking(vendor(0), RequestKind::Refresh)
            .expect("아는 벤더");
        assert!(
            matches!(hidden.snapshot.state, UsageVendorState::Ready),
            "값이 와서 거절이 안 보인다"
        );
        assert_eq!(hidden.served, UsageServed::Cached);
        assert!(!hidden.snapshot.in_flight);
        assert_eq!(pct(&hidden.snapshot), Some(40.0));
        assert_eq!(
            rig.threads.counts.spawned.load(Ordering::SeqCst),
            0,
            "조회가 안 나갔다"
        );
    }

    #[test]
    fn a_probe_start_reaches_subscribers_before_its_result() {
        let (rig, _clock) = rig();
        let log = rig.subscriber(1, &[0]);
        let other = rig.subscriber(2, &[1]);
        let pending = bus_request(&rig.service, 0, RequestKind::Refresh);
        wait_until("「갱신 중」 한 장", || log.count() == 1);
        let start = frames(&log)[0].snapshot.clone();
        assert!(start.in_flight);
        assert_eq!(pct(&start), None);

        rig.answer(0, ok(active(0, 61.0)));
        let answer = answered(&pending);
        assert_eq!(answer.served, UsageServed::Fresh);
        let got = frames(&log);
        assert_eq!(got.len(), 2, "끝 한 장은 합류자가 깨기 전에 나갔다");
        let end = &got[1].snapshot;
        assert!(!end.in_flight);
        assert!(end.revision > start.revision);
        assert_eq!(pct(end), Some(61.0));
        assert!(answer.snapshot.revision >= end.revision);
        assert_eq!(other.count(), 0, "다른 벤더의 구독자에겐 없다");
    }

    #[test]
    fn a_panicking_probe_still_closes_the_probe() {
        let (rig, _clock) = rig();
        rig.answer(0, Scripted::Panic);
        let answer = answered(&bus_request(&rig.service, 0, RequestKind::Refresh));
        assert_eq!(answer.served, UsageServed::Cached);
        assert!(!answer.snapshot.in_flight);
        assert_eq!(failed_kind(&answer.snapshot).as_deref(), Some("io"));
        wait_until("조회 스레드가 풀렸다", || ended(&rig) == 1);
        assert_eq!(rig.threads.counts.panicked.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_dropped_request_future_leaves_the_probe_to_its_guard() {
        let (rig, _clock) = rig();
        let log = rig.subscriber(1, &[0]);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("런타임");
        let dropped = runtime.block_on(async {
            tokio::time::timeout(
                Duration::from_millis(50),
                rig.service.request(vendor(0), RequestKind::Refresh),
            )
            .await
        });
        assert!(dropped.is_err(), "요청은 끝을 기다리던 중에 버려졌다");
        assert!(rig.in_flight(0));
        assert_eq!(rig.joiners(0), 0, "버려진 요청은 기다림을 놓았다");

        rig.answer(0, ok(active(0, 8.0)));
        wait_until("끝 한 장", || last_pct(&log) == Some(8.0));
        assert!(!rig.in_flight(0));
    }

    #[test]
    fn a_stuck_probe_does_not_hold_up_another_vendor() {
        let (rig, _clock) = rig();
        let stuck = bus_request(&rig.service, 0, RequestKind::Refresh);
        wait_until("조회가 떴다", || rig.queries(0) == 1);

        rig.answer(1, ok(active(1, 12.0)));
        let answer = answered(&bus_request(&rig.service, 1, RequestKind::Refresh));
        assert_eq!(answer.served, UsageServed::Fresh);
        assert!(rig.in_flight(0), "앞 벤더는 아직 막혀 있다");

        rig.answer(0, ok(active(0, 1.0)));
        assert_eq!(answered(&stuck).served, UsageServed::Fresh);
    }

    #[test]
    fn a_tick_plan_publishes_its_sheets_then_starts_its_probes() {
        let (rig, _clock) = rig();
        let log = rig.subscriber(1, &[0]);
        // 못 띄운 조회는 `carry_out` 안에서 곧바로 닫혀 끝 한 장을 낸다 — 두 장의 차례가 곧 발행과 기동의 차례다.
        rig.threads.refuse.store(true, Ordering::SeqCst);
        let plan = {
            let now = rig.service.now();
            rig.service.desk().book.plan_tick(&set(&[0]), now)
        };
        assert_eq!(
            plan.start,
            [key(0)],
            "기준점이 없는 구독된 칸 — 곧바로 조회"
        );
        assert_eq!(plan.publish.len(), 1);
        let sleep = plan.sleep;
        assert_eq!(rig.service.carry_out(plan), sleep);

        let got = frames(&log);
        assert_eq!(got.len(), 2);
        assert!(got[0].snapshot.in_flight, "계획의 「갱신 중」 한 장이 먼저");
        assert!(!got[1].snapshot.in_flight, "기동(여기선 곧 닫힘)이 나중");
        assert!(got[0].snapshot.revision < got[1].snapshot.revision);
        assert!(!rig.in_flight(0));
    }

    #[test]
    fn a_failed_probe_after_an_earlier_success_answers_cached() {
        let (rig, clock) = rig();
        rig.answer(0, ok(active(0, 30.0)));
        let first = rig
            .service
            .request_blocking(vendor(0), RequestKind::Refresh)
            .expect("아는 벤더");
        assert_eq!(first.served, UsageServed::Fresh);

        clock.advance_both(REFRESH_MIN_SPACING + Duration::from_secs(1));
        rig.answer(0, Scripted::Answer(Err(ProbeError::Timeout.into())));
        let second = rig
            .service
            .request_blocking(vendor(0), RequestKind::Refresh)
            .expect("아는 벤더");
        assert_eq!(rig.queries(0), 2, "간격이 지나 새 조회가 떴다");
        assert_eq!(
            second.served,
            UsageServed::Cached,
            "기다린 조회가 실패했다 — 앞의 성공은 이 요청의 새 값이 아니다"
        );
        assert_eq!(pct(&second.snapshot), Some(30.0), "값은 유지");
        assert_eq!(failed_kind(&second.snapshot).as_deref(), Some("timeout"));
    }

    #[test]
    fn a_joiner_that_gives_up_after_an_earlier_success_answers_cached() {
        let clock = Arc::new(ManualUsageClock::new(Duration::from_secs(100), T0));
        let rig = rig_full(clock.clone(), 2, SHORT_WAIT);
        // 앞의 성공은 요청 없이 띄워 끝낸다 — 짧은 기다림에 걸리지 않게.
        assert!(rig.service.desk().book.begin_probe(&key(0)));
        rig.service.start_probe(key(0));
        rig.answer(0, ok(active(0, 30.0)));
        wait_until("앞 조회 끝", || ended(&rig) == 1);
        assert_eq!(rig.service.desk().runs[&key(0)].last_ok, 1);

        clock.advance_both(REFRESH_MIN_SPACING + Duration::from_secs(1));
        let answer = rig
            .service
            .request_blocking(vendor(0), RequestKind::Refresh)
            .expect("아는 벤더");
        assert!(answer.snapshot.in_flight, "상한에 포기했다");
        assert_eq!(answer.served, UsageServed::Cached);
        assert_eq!(pct(&answer.snapshot), Some(30.0));
    }

    /// 무장한 뒤 받은 끝 한 장(`in_flight` 아님)에서 멈췄다고 알리고 `release` 가 올 때까지 `send` 안에 머무는 출구.
    /// 시간 상한이 없다 — 시험이 실패해 `release` 송신단이 사라지면 그때 풀린다.
    struct Parking {
        armed: Arc<AtomicBool>,
        parked: mpsc::Sender<()>,
        release: Mutex<Receiver<()>>,
    }

    impl UsageOutlet for Parking {
        fn send(&self, frame: &UsageFrame) {
            if !self.armed.load(Ordering::SeqCst) || frame.snapshot.in_flight {
                return;
            }
            let _ = self.parked.send(());
            let _ = self.release.lock().unwrap().recv();
        }

        fn revoke(&self) {}
    }

    #[test]
    fn joiners_wake_only_after_the_end_sheet_is_published() {
        let clock = Arc::new(ManualUsageClock::new(Duration::from_secs(100), T0));
        // 합류자의 기다림을 시험 쪽 상한보다 넉넉히 — 멈춰 있는 동안 기다림 초과로 답하는 갈래를 없앤다.
        let rig = rig_full(clock, 2, BOUND * 6);
        let armed = Arc::new(AtomicBool::new(false));
        let (parked_tx, parked) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        rig.service.attach(
            1,
            Parking {
                armed: armed.clone(),
                parked: parked_tx,
                release: Mutex::new(release_rx),
            },
        );
        rig.service.replace_subscription(1, set(&[0]));
        armed.store(true, Ordering::SeqCst);

        let pending = bus_request(&rig.service, 0, RequestKind::Refresh);
        wait_until("조회가 떴다", || rig.queries(0) == 1);
        // 시험이 직접 쥔 버스 합류자 하나 — 깸이 왔는지를 시각 없이 본다(조회는 대본을 기다리며 진행 중이다).
        let (joined, woken) = mpsc::sync_channel(1);
        rig.service
            .desk()
            .runs
            .get_mut(&key(0))
            .expect("칸")
            .blocking
            .push(joined);

        rig.answer(0, ok(active(0, 5.0)));
        parked
            .recv_timeout(BOUND)
            .expect("끝 한 장이 출구 안에서 멈췄다");
        // 멈춘 동안 — 깨우기가 발행보다 앞섰다면 이미 와 있다.
        assert!(
            matches!(woken.try_recv(), Err(TryRecvError::Empty)),
            "발행이 끝나기 전에 버스 합류자를 깨웠다"
        );
        assert_eq!(
            *rig.service.finished[&key(0)].borrow(),
            0,
            "발행이 끝나기 전에 async 합류자를 깨웠다"
        );
        assert!(matches!(pending.try_recv(), Err(TryRecvError::Empty)));

        release.send(()).expect("출구");
        woken.recv_timeout(BOUND).expect("발행 뒤에 깬다");
        assert_eq!(answered(&pending).served, UsageServed::Fresh);
    }
}
