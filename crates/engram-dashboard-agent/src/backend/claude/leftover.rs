//! claude 끊기 뒤 잔여물 정리의 **고르기 규칙** — 판 한 차례에 끝낼 후보를 추리고([`candidates`]) 후보 하나를
//! 규칙 넷의 AND 로 가른다([`select`]). 훅 명령줄 · taskkill 표지는 [`is_hook_command`] · [`taskkill_names`].
//!
//! ★규칙은 순수하다★ — OS 호출 · 자물쇠 · 스레드가 없고 모든 OS 에서 컴파일된다(ADR-0012 · ADR-0230). 사실은 붙든
//!   핸들로 이미 읽어 온 값([`ProcessFacts`])만 보고, 「끝났나」는 [`Chain::exited`] 로 주입받는다.
//! ★좁히기만 한다★ — 하나라도 못 믿으면(못 찾은 고리 · 빈 칸 · 조회 오류 · 모양 밖) 끝내지 않는 판정을 낸다. 표지
//!   어느 것도 후보를 늘리지 않는다 — claude 가 훅 감싸기나 끊기 방식을 바꾸면 모두 놓친다(안전한 쪽).
//!
//! 규칙이 보는 탄생은 **탄생 기록**([`Recorder`])이 모은다 — 무리의 가입 알림을 듣는 스레드가 기록이 켜진 동안 받자마자
//! 붙들어 넣는다. 포트와 그 스레드는 화신마다 한 번 [`BirthWatch`] 가 띄운다. 무리는 중립 손잡이로만 보고 여기에 OS
//! 갈래는 없다.
//!
//! 끊기 에피소드는 [`GateCell`](화신마다 하나 — 턴 열림 문 + 에피소드 · 끊기 줄 함수와 decoder 가 함께 쥔다)이 세우고,
//! 줄이 쓰인 뒤 N 초를 일꾼이 잰다 — 그때도 턴 끝이 없으면 정리 한 판([`run_pass`])이 차례마다 고르고 끝낸다.
//! 정리기([`Cleaner`])가 없으면 문만 여닫는다.
//! ★문 자물쇠는 잎이다★ — 쥔 채 필드 · 단조 시각 읽기 · 원자 읽기(기록 번호 · 물러남 표시)만 하고, 로그 · 스레드 기동 ·
//!   할당과 해제 · OS 호출 · 다른 자물쇠는 하지 않는다(유일한 예외는 끝내기 확정의 끝내기 호출 하나). 빼낸 것은 놓은 뒤
//!   버린다. 기록 자물쇠와 겹쳐 잡지 않는다.
//! 명세 = `docs/process/S21-chat-ux/trd-t40.md` §3-2 · §3-3 · §3-4 · §3-5 · §3-7.
// ADR-0262

use std::any::Any;
use std::fmt;
use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, TryLockError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use engram_dashboard_base::sync;
use engram_dashboard_base::time::{Clock, SystemClock};

use crate::platform::process_group::{
    Births, MemberKill, Pinned, PortEvent, ProcessFacts, ProcessGroup, RetiringSignal, GROUP_GONE,
};
use crate::transport::input_queue::OnWritten;
use crate::transport::stdio::InterruptOut;
use crate::types::AgentId;

/// 조상 걸음의 상한 — D(A₁)부터 꼭대기 T 까지의 고리 수. 잰 사슬은 넷 이하(p3t3)이고, 넘으면 끊긴 사슬로 본다.
pub(super) const ANCESTOR_MAX: usize = 16;

/// 고르기가 보는 고리 하나 — 붙든 프로세스의 번호와 그 핸들로 읽은 사실.
///
/// - `seq` = 탄생 기록의 순번(가입 알림을 꺼낸 순서) · `None` = 스냅숏 멤버.
/// - `killable` = 끝내기 권한까지 붙들었나 — `false` 면 사슬은 잇되 후보는 못 된다.
#[derive(Debug, Clone, Copy)]
pub(super) struct Link<'a> {
    pub pid: u32,
    pub facts: &'a ProcessFacts,
    pub seq: Option<u64>,
    pub killable: bool,
}

/// 한 기록에 대한 보기 — 스냅숏과 같은 기록의 탄생이 「알려진 고리」의 전부다.
///
/// - `root_pid` = 통로의 뿌리(`cmd /c`) — 적힌 부모가 이 번호인 고리를 claude 로 본다. `0` = 모름.
/// - `births` 의 순서는 뜻이 없다 — 차례는 `seq` 가 정한다.
/// - `exited` = 알려진 고리 하나가 지금 끝났나. `Err` 은 조회 실패이고 고르기는 그것을 「모름」으로 읽는다.
pub(super) struct Chain<'a> {
    pub root_pid: u32,
    pub snapshot: &'a [Link<'a>],
    pub births: &'a [Link<'a>],
    pub exited: &'a dyn Fn(&Link<'_>) -> io::Result<bool>,
}

/// [`select`] 의 판정 — `Cleanup` 말고는 전부 「끝내지 않는다」이고, 이름은 로그 사유(TRD §3-4 · §3-8)와 짝이다.
///
/// - `Cleanup.chain` = 후보 C 부터 꼭대기 T 까지의 번호(C 가 처음 · T 가 끝 · claude 는 빠진다).
/// - `AncestorAlive.pid` = 살아 있는 첫 조상.
/// - `Unknown` = 빈 칸 · 끝남 조회 오류 · 뿌리 번호 모름.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Verdict {
    Cleanup { chain: Vec<u32> },
    ParentAlive,
    ParentUnknown,
    NotHookCopy,
    AncestorAlive { pid: u32 },
    ChainCut,
    NotHook,
    NoTaskkill,
    Unknown,
}

/// 판 한 차례의 후보 — 이 기록의 탄생 가운데 번호가 쓰기 명단 `w` 에 없고 · 끝내기 권한까지 붙들었고 ·
/// (번호, 생성)이 스냅숏에 없고 · 끝나지 않은 것(끝남 조회 `Err` 이면 뺀다)을 `seq` 오름차순으로.
///
/// `w` 는 번호만 대조하고 순서에 기대지 않는다 — 정렬이 어긋나도 후보가 늘지 않는다. `seq` 가 없는 고리는
/// 탄생이 아니라 빠진다.
pub(super) fn candidates<'a>(view: &Chain<'a>, w: &[u32]) -> Vec<&'a Link<'a>> {
    let mut out: Vec<&'a Link<'a>> = view
        .births
        .iter()
        .filter(|b| {
            b.seq.is_some()
                && b.killable
                && !w.contains(&b.pid)
                && !view
                    .snapshot
                    .iter()
                    .any(|s| s.pid == b.pid && s.facts.create == b.facts.create)
                && matches!((view.exited)(b), Ok(false))
        })
        .collect();
    out.sort_by_key(|b| b.seq);
    out
}

/// 후보 C 하나를 규칙 넷의 AND 로 가른다 — 앞에서 걸린 규칙의 판정 하나를 돌려준다.
///
/// 1. 부모 D 가 알려진 고리이고 끝났다. 2. C 가 D 의 훅 사본이다(실행 파일이 같고 명령줄이 같거나 D 가 훅 명령).
/// 3. D 부터 claude 직전까지 모든 조상이 알려진 고리이고 끝났다(고리마다 부모 생성 ≤ 자식 생성 · claude 는 판단하지
///    않는다). 4. 꼭대기 T 가 훅 실행기이고, 이 기록에 claude 가 띄운 `taskkill /PID <T> /T /F` 가 T 이후에 태어났다.
///
/// `exited` 는 D 와 claude 직전까지의 조상에만 부른다.
// ADR-0262
pub(super) fn select(c: &Link<'_>, view: &Chain<'_>) -> Verdict {
    if view.root_pid == 0 {
        return Verdict::Unknown;
    }

    let Some(d) = parent_of(view, c.pid, c.facts) else {
        return Verdict::ParentUnknown;
    };
    match (view.exited)(d) {
        Ok(true) => {}
        Ok(false) => return Verdict::ParentAlive,
        Err(_) => return Verdict::Unknown,
    }

    if [c.facts, d.facts]
        .iter()
        .any(|f| f.image.is_empty() || f.cmdline.is_empty())
    {
        return Verdict::Unknown;
    }
    let hook_copy = c.facts.image == d.facts.image
        && (c.facts.cmdline == d.facts.cmdline || is_hook_command(d.facts));
    if !hook_copy {
        return Verdict::NotHookCopy;
    }

    let (walked, claude) = match ancestors(c, d, view) {
        Ok(found) => found,
        Err(verdict) => return verdict,
    };
    let Some(&top) = walked.last() else {
        return Verdict::ChainCut;
    };

    if !(image_is(&top.facts.image, "bash.exe") && is_hook_command(top.facts)) {
        return Verdict::NotHook;
    }
    let named_by_claude = view.births.iter().any(|b| {
        taskkill_names(b.facts, claude.pid) == Some(top.pid)
            && b.facts.create >= top.facts.create
            && parent_of(view, b.pid, b.facts)
                .is_some_and(|p| p.pid == claude.pid && p.facts.create == claude.facts.create)
    });
    if !named_by_claude {
        return Verdict::NoTaskkill;
    }

    Verdict::Cleanup {
        chain: std::iter::once(c.pid)
            .chain(walked.iter().map(|l| l.pid))
            .collect(),
    }
}

/// 훅 명령 모양인가 — 명령줄이 「첫 토큰 · `-c` · 큰따옴표로 감싼 인자 S」이고 S 가 `bash ` 로 시작하며
/// `shell-snapshots` 를 담지 않는다. claude 2.1.284 가 `.sh` 훅을 감싸는 모양이다(훅 291 대 Bash 도구 6 · 모호 0).
///
/// S 에 줄바꿈이 있으면 거짓이다 — 잰 대조(`r4sim.py` 의 정규식)가 그런 S 를 받지 않았고, 받으면 넓히는 쪽이다.
pub(super) fn is_hook_command(f: &ProcessFacts) -> bool {
    let Some(rest) = after_program(&f.cmdline) else {
        return false;
    };
    let Some(rest) = rest.strip_prefix("-c") else {
        return false;
    };
    let arg = rest.trim_start();
    if arg.len() == rest.len() {
        return false;
    }
    let Some(s) = arg
        .trim_end()
        .strip_prefix('"')
        .and_then(|a| a.strip_suffix('"'))
    else {
        return false;
    };
    s.starts_with("bash ")
        && !s.contains("shell-snapshots")
        && !s.contains(|ch: char| ch == '\r' || ch == '\n')
}

/// claude 가 띄운 `taskkill /PID n /T /F` 의 `n` — 부모가 `claude` 이고 실행 파일 이름이 `taskkill.exe`(대소문자
/// 무시)이며 프로그램 뒤 인자가 정확히 `/PID` · 십진수 · `/T` · `/F` 일 때만. 대소문자 · `/PID:n` · 순서 바꿈 · 더 붙은
/// 인자 · 앞자리 0 · `0` 은 모두 `None` — 출처 스파이크 표의 taskkill 218 개가 모두 이 모양이고, 그 밖은 받지 않는다.
pub(super) fn taskkill_names(f: &ProcessFacts, claude: u32) -> Option<u32> {
    if f.ppid != claude || !image_is(&f.image, "taskkill.exe") {
        return None;
    }
    let mut args = after_program(&f.cmdline)?.split_whitespace();
    let (Some("/PID"), Some(n), Some("/T"), Some("/F"), None) = (
        args.next(),
        args.next(),
        args.next(),
        args.next(),
        args.next(),
    ) else {
        return None;
    };
    if n.starts_with('0') || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    n.parse().ok()
}

/// 기록된 부모를 알려진 고리에서 찾는다 — 번호가 `child.ppid` 이고 생성이 0 이 아니며 자식의 생성 이하인 것
/// 가운데 생성이 가장 늦은 것. 붙든 번호는 재사용되지 않아 그런 고리는 실제로 많아야 하나지만, 둘 이상이면 자식이
/// 태어날 때 그 번호를 쥐었을 수 있는 마지막 주인을 고른다. 생성까지 같은 둘(스냅숏 멤버의 되알림)이면 스냅숏 쪽.
/// 부모 번호 0 · 자기 번호 · 자식 생성 0 은 모름.
fn parent_of<'a>(view: &Chain<'a>, child_pid: u32, child: &ProcessFacts) -> Option<&'a Link<'a>> {
    if child.ppid == 0 || child.ppid == child_pid || child.create == 0 {
        return None;
    }
    let mut best: Option<&'a Link<'a>> = None;
    for link in view.snapshot.iter().chain(view.births.iter()) {
        if link.pid != child.ppid || link.facts.create == 0 || link.facts.create > child.create {
            continue;
        }
        if best.map_or(true, |b| link.facts.create > b.facts.create) {
            best = Some(link);
        }
    }
    best
}

/// 규칙 3 — D(A₁)에서 적힌 부모를 따라 걸어 `(A₁ … Aₖ₋₁, Aₖ)` 를 돌려준다. `Aₖ` = 적힌 부모가 뿌리인 고리(claude).
/// D 의 끝남은 규칙 1 이 이미 봤다.
fn ancestors<'a>(
    c: &Link<'_>,
    d: &'a Link<'a>,
    view: &Chain<'a>,
) -> Result<(Vec<&'a Link<'a>>, &'a Link<'a>), Verdict> {
    let mut seen = vec![(c.pid, c.facts.create)];
    let mut walked: Vec<&'a Link<'a>> = Vec::new();
    let mut a = d;
    loop {
        if a.pid == view.root_pid || seen.contains(&(a.pid, a.facts.create)) {
            return Err(Verdict::ChainCut);
        }
        seen.push((a.pid, a.facts.create));
        if a.facts.ppid == view.root_pid {
            // k = 1(D 가 곧 claude 자리)이면 꼭대기가 없다.
            return if walked.is_empty() {
                Err(Verdict::ChainCut)
            } else {
                Ok((walked, a))
            };
        }
        if !walked.is_empty() {
            match (view.exited)(a) {
                Ok(true) => {}
                Ok(false) => return Err(Verdict::AncestorAlive { pid: a.pid }),
                Err(_) => return Err(Verdict::Unknown),
            }
        }
        walked.push(a);
        if walked.len() > ANCESTOR_MAX {
            return Err(Verdict::ChainCut);
        }
        a = parent_of(view, a.pid, a.facts).ok_or(Verdict::ChainCut)?;
    }
}

/// 명령줄의 첫 토큰(프로그램)을 건너뛴 나머지 — 첫 토큰은 큰따옴표로 감싼 것이거나 공백 없는 덩이이고, 뒤에 공백이
/// 하나 이상 와야 한다.
fn after_program(cmdline: &str) -> Option<&str> {
    let rest = if let Some(quoted) = cmdline.strip_prefix('"') {
        let close = quoted.find('"')?;
        quoted.get(close + 1..)?
    } else {
        let end = cmdline.find(char::is_whitespace)?;
        if end == 0 {
            return None;
        }
        cmdline.get(end..)?
    };
    let trimmed = rest.trim_start();
    (trimmed.len() < rest.len()).then_some(trimmed)
}

/// 실행 파일 경로의 파일 이름이 `name` 인가(ASCII 대소문자 무시). `\` · `/` 둘 다 경로 구분자로 본다 — 어느 OS 에서
/// 돌아도 Windows 경로를 같게 가른다.
fn image_is(image: &str, name: &str) -> bool {
    image
        .rsplit(|ch: char| ch == '\\' || ch == '/')
        .next()
        .is_some_and(|file| file.eq_ignore_ascii_case(name))
}

// ── 탄생 기록 · 가입 알림 듣기 ──────────────────────────────────────────────────────────────

/// 한 기록에 넣는 탄생의 상한 — 넘는 것은 넣지 않고 [`Recorder::full`] 이 선다(그 고리는 알려지지 않아 놓침 쪽).
pub(super) const BIRTH_MAX: usize = 512;

/// 듣는 스레드가 알림 하나를 기다리는 한도 — 이만큼마다 무리가 사라졌는지 · 물러나는지 본다.
const LISTEN_WAIT: Duration = Duration::from_millis(500);

/// 1601-01-01 부터 1970-01-01 까지의 FILETIME 눈금(100 ns).
const UNIX_EPOCH_FILETIME: u64 = 116_444_736_000_000_000;

/// 기록이 켜진 동안 가입 알림을 받자마자 붙든 프로세스 하나 — 쥐는 동안 그 번호는 재사용되지 않고, 끝난 뒤에도
/// 사실이 남는다. 마지막 `Arc` 를 버리면 핸들이 닫힌다 — 기록 자물쇠 밖에서 버린다.
///
/// - `seq` = 이 화신에서 넣은 순번 — 가입 알림을 꺼낸 순서다(생성 순서가 아니다).
/// - `facts` = `pin` 이 붙들 때 읽은 사실의 사본.
/// - `killable` = 끝내기 권한까지 붙들었나 — `false` 면 사슬은 잇되 후보는 못 된다.
pub(super) struct Birth {
    pub pid: u32,
    pub seq: u64,
    pub facts: ProcessFacts,
    pub pin: Box<dyn Pinned>,
    pub killable: bool,
}

impl Birth {
    pub(super) fn link(&self) -> Link<'_> {
        Link {
            pid: self.pid,
            facts: &self.facts,
            seq: Some(self.seq),
            killable: self.killable,
        }
    }
}

/// 끊기 에피소드가 켜고 끄는 탄생 기록 — 화신마다 하나, 여는 이 · 듣는 스레드 · 판이 같은 `Arc` 로 쥔다.
///
/// - 기록은 번호로 갈린다: [`Self::start`] 가 새 번호를 켜고, 그 번호를 쥔 이만 끄고([`Self::stop`]) 복사한다
///   ([`Self::copy`]). 켜진 번호(`0` = 꺼짐)는 [`Self::active`] 로 자물쇠 없이 읽는다.
/// - [`Self::port_failed`] = 가입 알림 포트가 진짜 OS 실패로 못 쓰게 됐다 — 한 번 서면 화신 동안 내려가지 않는다.
///
/// ★기록 자물쇠는 잎이다★ — 쥔 채 필드 · `Vec` · `Arc` 복제 · 꺼내기만 하고, OS 호출 · 핸들 닫기(`Arc<Birth>` 의
/// 마지막 drop) · 로그 · 다른 자물쇠는 하지 않는다. 문 자물쇠와 겹쳐 잡지 않는다. 그래서 꺼낸 기록은 돌려주고, 받은
/// 쪽이 자물쇠가 놓인 뒤 버린다. 독에 걸려도 그대로 쓴다.
// ADR-0262
pub(super) struct Recorder {
    /// 쓰기는 자물쇠 안에서만 — 「같은 기록인가」 판정과 넣기 · 꺼내기가 한 구간이 된다.
    active: AtomicU64,
    port_failed: AtomicBool,
    inner: Mutex<RecInner>,
}

struct RecInner {
    next_rec: u64,
    next_seq: u64,
    births: Vec<Arc<Birth>>,
    full: bool,
}

/// 듣는 스레드가 붙든 탄생 — 기록에 넣기 전.
struct Caught {
    pid: u32,
    facts: ProcessFacts,
    pin: Box<dyn Pinned>,
    killable: bool,
}

/// [`Recorder::admit`] 의 답 — 넣지 못한 것은 돌려준다(부른 쪽이 자물쇠 밖에서 버린다).
enum Admit {
    Recorded {
        seq: u64,
    },
    /// 받을 기록이 꺼졌거나 다른 기록으로 바뀌었다.
    Stale(Caught),
    /// 같은 (번호, 생성)이 이미 있다 — 중첩 Job 의 재보고 · 붙일 때의 되알림.
    Known(Caught),
    /// 상한에 닿았다 — `first` = 이 기록에서 처음 넘쳤다.
    Full {
        caught: Caught,
        first: bool,
    },
}

impl Recorder {
    pub(super) fn new() -> Self {
        Self {
            active: AtomicU64::new(0),
            port_failed: AtomicBool::new(false),
            inner: Mutex::new(RecInner {
                next_rec: 0,
                next_seq: 0,
                births: Vec::new(),
                full: false,
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, RecInner> {
        sync::lock(&self.inner)
    }

    /// 새 기록을 켜고 그 번호(`0` 아님)를 준다 — 넘침 표시를 내리고, 앞 기록에 남아 있던 것을 꺼내 돌려준다(받은
    /// 쪽이 버린다).
    pub(super) fn start(&self) -> (u64, Vec<Arc<Birth>>) {
        let mut inner = self.lock();
        let r = match inner.next_rec.wrapping_add(1) {
            0 => 1,
            r => r,
        };
        inner.next_rec = r;
        inner.full = false;
        self.active.store(r, Ordering::Release);
        (r, std::mem::take(&mut inner.births))
    }

    /// `r` 이 켜져 있을 때만 끄고 그 기록을 꺼내 돌려준다(받은 쪽이 버린다). 아니면 아무것도 안 바꾸고 빈 것을 준다.
    pub(super) fn stop(&self, r: u64) -> Vec<Arc<Birth>> {
        if r == 0 {
            return Vec::new();
        }
        let mut inner = self.lock();
        if self.active.load(Ordering::Acquire) != r {
            return Vec::new();
        }
        self.active.store(0, Ordering::Release);
        std::mem::take(&mut inner.births)
    }

    pub(super) fn active(&self) -> u64 {
        self.active.load(Ordering::Acquire)
    }

    /// `r` 이 켜져 있으면 지금까지 넣은 것을 복제해 준다 — 아니면 빈 것.
    pub(super) fn copy(&self, r: u64) -> Vec<Arc<Birth>> {
        if r == 0 {
            return Vec::new();
        }
        let inner = self.lock();
        if self.active.load(Ordering::Acquire) != r {
            return Vec::new();
        }
        inner.births.clone()
    }

    /// 마지막으로 켠 기록이 상한에 닿아 넣지 못한 탄생이 있었다.
    pub(super) fn full(&self) -> bool {
        self.lock().full
    }

    pub(super) fn len(&self) -> usize {
        self.lock().births.len()
    }

    pub(super) fn port_failed(&self) -> bool {
        self.port_failed.load(Ordering::Acquire)
    }

    fn fail_port(&self) {
        self.port_failed.store(true, Ordering::Release);
    }

    /// 기록 `r` 에 넣는다 — 아직 `r` 이 켜져 있고 · (번호, 생성)이 새롭고 · 상한 안일 때만.
    fn admit(&self, r: u64, caught: Caught) -> Admit {
        let mut inner = self.lock();
        if r == 0 || self.active.load(Ordering::Acquire) != r {
            return Admit::Stale(caught);
        }
        if inner
            .births
            .iter()
            .any(|b| b.pid == caught.pid && b.facts.create == caught.facts.create)
        {
            return Admit::Known(caught);
        }
        if inner.births.len() >= BIRTH_MAX {
            let first = !inner.full;
            inner.full = true;
            return Admit::Full { caught, first };
        }
        let seq = inner.next_seq;
        inner.next_seq = seq.wrapping_add(1);
        let Caught {
            pid,
            facts,
            pin,
            killable,
        } = caught;
        inner.births.push(Arc::new(Birth {
            pid,
            seq,
            facts,
            pin,
            killable,
        }));
        Admit::Recorded { seq }
    }
}

/// 로그 귀속 — 제어 채널이 있는 스폰이면 에이전트 id 를, 늘 뿌리 PID 를 싣는다(ADR-0217 — 제어 채널이 없는 스폰은
/// id 가 없어 뿌리로 잇는다).
#[derive(Debug, Clone, Copy)]
pub(super) struct LogTag {
    agent: Option<AgentId>,
    root_pid: u32,
}

impl LogTag {
    pub(super) fn new(agent: Option<AgentId>, root_pid: u32) -> Self {
        Self { agent, root_pid }
    }

    fn agent(&self) -> Option<tracing::field::DisplayValue<AgentId>> {
        self.agent.map(tracing::field::display)
    }
}

/// 듣는 스레드 · 포트 확보 · 여는 이 · 쓰기 확인이 무리에게 묻는 것 — 실물 = [`ProcessGroup`]. 시험이 가짜로 갈아
/// 끼운다.
pub(super) trait Group: Send + Sync + 'static {
    fn member_pids(&self) -> io::Result<Vec<u32>>;
    fn pin(&self, pid: u32, kill: bool) -> io::Result<Option<Box<dyn Pinned>>>;
    fn watch_births(&self, start: impl FnOnce(Arc<dyn Births>) -> io::Result<()>)
        -> io::Result<()>;
    fn unwatch_births(&self) -> io::Result<()>;
    fn is_gone(&self) -> bool;
    fn retiring(&self) -> RetiringSignal;
}

// 경로 호출은 고유 메서드를 먼저 고른다 — 이 trait 메서드로 되돌아오지 않는다.
impl Group for ProcessGroup {
    fn member_pids(&self) -> io::Result<Vec<u32>> {
        ProcessGroup::member_pids(self)
    }

    fn pin(&self, pid: u32, kill: bool) -> io::Result<Option<Box<dyn Pinned>>> {
        ProcessGroup::pin(self, pid, kill)
    }

    fn watch_births(
        &self,
        start: impl FnOnce(Arc<dyn Births>) -> io::Result<()>,
    ) -> io::Result<()> {
        ProcessGroup::watch_births(self, start)
    }

    fn unwatch_births(&self) -> io::Result<()> {
        ProcessGroup::unwatch_births(self)
    }

    fn is_gone(&self) -> bool {
        ProcessGroup::is_gone(self)
    }

    fn retiring(&self) -> RetiringSignal {
        ProcessGroup::retiring(self)
    }
}

/// 화신 하나의 가입 알림 포트 확보 — 처음 부른 이만 포트를 만들고 듣는 스레드를 띄우고 무리를 붙이며, 그 뒤로는 그
/// 결과를 읽는다. 무리에 두 번째 `watch_births` 가 가지 않게 막는 것이 이것이다(OS 의 거절에 기대지 않는다).
// ADR-0262
pub(super) struct BirthWatch {
    acquired: OnceLock<Result<(), ()>>,
}

impl BirthWatch {
    pub(super) fn new() -> Self {
        Self {
            acquired: OnceLock::new(),
        }
    }

    /// 탄생을 기록할 수 있나 — `Ok` = 듣는 스레드가 돌고 무리가 포트에 붙었다. `Err` = 포트를 못 쓴다(그 에피소드는
    /// `Failed(port)`): 확보 실패 · 무리가 이미 없음 · 듣는 스레드가 실패로 굳힘([`Recorder::port_failed`]).
    ///
    /// 듣는 스레드가 무리가 사라져서 · 물러나서 끝난 뒤에도 `Ok` 다 — 끝나며 켜진 기록을 껐고, 그 뒤 켠 기록에는 탄생이
    /// 안 든다(놓침 쪽). 처음 부름은 붙이는 동안 Job 을 강하게 쥐지만 막히지 않는다 — `spawn` 은 듣는 스레드를 띄우기만
    /// 한다(처음 부름만 쓴다).
    fn ensure_with<G: Group>(
        &self,
        group: &Arc<G>,
        recorder: &Arc<Recorder>,
        tag: &LogTag,
        spawn: impl FnOnce(Box<dyn FnOnce() + Send>) -> io::Result<()>,
    ) -> Result<(), ()> {
        let acquired = *self
            .acquired
            .get_or_init(|| acquire(group, recorder, *tag, spawn));
        if recorder.port_failed() {
            return Err(());
        }
        acquired
    }
}

/// 포트 확보가 어디까지 갔나 — 실패를 가르는 데만 쓴다.
#[derive(Clone, Copy)]
enum Stage {
    BeforeStart,
    StartFailed,
    Started,
}

/// 포트 → 듣는 스레드 기동 → (기동이 `Ok` 일 때만) 붙이기 — 순서는 `watch_births` 가 강제한다. 그래서 알림이 쌓일 수
/// 있게 되는 때에는 꺼낼 스레드가 이미 떠 있다. 붙었는지는 뜬 스레드에 알린다.
///
/// 굳히는 것은 진짜 OS 실패뿐이다 — 무리가 이미 없거나 이 OS 에 무리가 없으면 그 에피소드만 실패한다.
fn acquire<G: Group>(
    group: &Arc<G>,
    recorder: &Arc<Recorder>,
    tag: LogTag,
    spawn: impl FnOnce(Box<dyn FnOnce() + Send>) -> io::Result<()>,
) -> Result<(), ()> {
    let (attached_tx, attached_rx) = mpsc::sync_channel(1);
    let mut stage = Stage::BeforeStart;
    let watched = group.watch_births(|port| {
        let listener = Listener {
            port,
            attached: attached_rx,
            group: Arc::clone(group),
            recorder: Arc::clone(recorder),
            tag,
        };
        let spawned = spawn(Box::new(move || listen(listener)));
        stage = match spawned {
            Ok(()) => Stage::Started,
            Err(_) => Stage::StartFailed,
        };
        spawned
    });
    // 뜬 스레드가 없으면(기동 전 실패 · 기동 실패) 받을 이가 없어 보내기가 실패한다 — 알릴 이가 없을 뿐이다.
    let _ = attached_tx.send(watched.is_ok());
    let Err(e) = watched else {
        return Ok(());
    };
    match stage {
        Stage::BeforeStart if matches!(e.kind(), GROUP_GONE | io::ErrorKind::Unsupported) => {
            tracing::debug!(
                agent = tag.agent(),
                root_pid = tag.root_pid,
                "무리가 없어 가입 알림 포트를 만들지 않았다: {e}"
            );
            return Err(());
        }
        Stage::BeforeStart => tracing::warn!(
            agent = tag.agent(),
            root_pid = tag.root_pid,
            "가입 알림 포트를 만들지 못했다 — 이 화신은 끊기 잔여물을 정리하지 않는다: {e}"
        ),
        Stage::StartFailed => tracing::warn!(
            agent = tag.agent(),
            root_pid = tag.root_pid,
            "가입 알림 듣는 스레드를 띄우지 못해 포트를 붙이지 않았다 — 이 화신은 끊기 잔여물을 정리하지 않는다: {e}"
        ),
        Stage::Started => tracing::warn!(
            agent = tag.agent(),
            root_pid = tag.root_pid,
            "무리를 가입 알림 포트에 붙이지 못했다 — 듣는 스레드는 끝나고 이 화신은 끊기 잔여물을 정리하지 않는다: {e}"
        ),
    }
    recorder.fail_port();
    Err(())
}

/// 듣는 스레드가 쥐는 것. 무리는 중립 손잡이로만 쥔다 — Job 은 손잡이를 부르는 동안만 올라간다.
struct Listener<G> {
    port: Arc<dyn Births>,
    attached: Receiver<bool>,
    group: Arc<G>,
    recorder: Arc<Recorder>,
    tag: LogTag,
}

/// 듣기가 끝나는 정상 까닭.
#[derive(Clone, Copy)]
enum End {
    Gone,
    Retiring,
}

/// 한 차례의 실패 — 포트를 굳힌다.
enum Failure {
    Take(io::Error),
    Panic(String),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Take(e) => write!(f, "꺼내기 {e}"),
            Failure::Panic(message) => write!(f, "패닉 {message}"),
        }
    }
}

/// 듣는 스레드 몸통 — 화신 수명 동안 돈다. 기록이 켜진 동안 온 가입 알림만 곧바로 붙들어 넣고, 나머지는 버린다.
///
/// - 시작: 여는 이의 「붙었나」를 먼저 받는다. `false` = 붙은 적이 없다 → 뗄 것 없이 끝난다. 보내는 쪽이
///   사라졌으면(붙인 뒤 보내기 전에 여는 이가 패닉) 붙었을 수 있어 평소대로 돈다.
/// - 끝: 알림 · 시간 초과마다 무리가 사라졌는지 · 물러나는지 본다 → 켜진 기록을 끄고 끝난다(물러날 때는 먼저 뗀다).
/// - 실패 모드: 꺼내기 오류 · 차례 패닉 → 포트를 굳히고 켜진 기록을 끄고 뗀다 → 떼면 끝 · 못 떼면 끝나지 않고 위의 끝이
///   올 때까지 꺼내 버리기만 한다(붙은 포트를 아무도 안 꺼내면 커널 큐가 커진다). 그 사이 꺼내기마저 실패하면 비울 수
///   없으니 끝난다.
///
/// ★차례마다 `catch_unwind` 를 두르지만 릴리스는 `panic = "abort"` 다★ — 그래서 몸통은 패닉하지 않게 짠다(풀기 ·
/// 첨자 없음 · 독 견딤). 두른 것은 unwind 빌드(개발 · 시험)의 안전망이다.
// ADR-0262
fn listen<G: Group>(l: Listener<G>) {
    if let Ok(false) = l.attached.recv() {
        return;
    }
    let attached_at = Instant::now();
    let retiring = l.group.retiring();
    let mut draining = false;
    loop {
        let round = panic::catch_unwind(AssertUnwindSafe(|| {
            l.round(&retiring, draining, attached_at)
        }));
        let failure = match round {
            Ok(Ok(None)) => continue,
            Ok(Ok(Some(end))) => {
                l.finish(end);
                return;
            }
            Ok(Err(e)) => Failure::Take(e),
            Err(payload) => Failure::Panic(panic_text(&*payload).to_owned()),
        };
        if draining {
            tracing::debug!(
                agent = l.tag.agent(),
                root_pid = l.tag.root_pid,
                "떼지 못한 가입 알림 포트를 비우다 실패해 듣기를 끝낸다: {failure}"
            );
            return;
        }
        if l.fail(failure) {
            return;
        }
        draining = true;
    }
}

impl<G: Group> Listener<G> {
    /// 알림 하나를 꺼내고, 끝낼 때면 그 까닭을 준다. 비우기만 하는 중이 아니면 가입을 기록한다. ★올린 Job 을 쥔 채
    /// 꺼내지 않는다★ — 손잡이를 거쳐 부를 때만 올라간다.
    fn round(
        &self,
        retiring: &RetiringSignal,
        draining: bool,
        attached_at: Instant,
    ) -> io::Result<Option<End>> {
        let event = self.port.next(LISTEN_WAIT)?;
        if self.group.is_gone() {
            return Ok(Some(End::Gone));
        }
        if retiring.is_set() {
            return Ok(Some(End::Retiring));
        }
        if let (PortEvent::Joined(pid), false) = (event, draining) {
            self.record(pid, attached_at);
        }
        Ok(None)
    }

    /// 기록이 켜져 있으면 곧바로 붙든다(자물쇠 밖) — 알림의 번호는 열어 쥐어야 산 것도 재사용 안 된 것도 선다. 그
    /// 뒤 같은 기록일 때만 넣고, 못 넣은 것은 기록 자물쇠를 놓은 뒤 버린다.
    fn record(&self, pid: u32, attached_at: Instant) {
        let r = self.recorder.active();
        if r == 0 {
            return;
        }
        let Some((pin, killable)) = self.pin(pid) else {
            return;
        };
        let facts = pin.facts().clone();
        let born_ago_us = born_ago_us(facts.create);
        let caught = Caught {
            pid,
            facts,
            pin,
            killable,
        };
        match self.recorder.admit(r, caught) {
            Admit::Recorded { seq } => tracing::debug!(
                agent = self.tag.agent(),
                root_pid = self.tag.root_pid,
                pid,
                seq,
                killable,
                born_ago_us,
                since_attach_ms = millis(attached_at.elapsed()),
                "가입한 프로세스를 붙들어 탄생 기록에 넣었다"
            ),
            Admit::Full { caught, first } => {
                drop(caught);
                if first {
                    tracing::debug!(
                        agent = self.tag.agent(),
                        root_pid = self.tag.root_pid,
                        limit = BIRTH_MAX,
                        "탄생 기록이 상한에 닿아 더 넣지 않는다 — 넘친 고리는 알려지지 않는다"
                    );
                }
            }
            Admit::Stale(caught) | Admit::Known(caught) => drop(caught),
        }
    }

    /// 끝내기 권한까지 붙든다 — 거절되면 조회 권한만으로 한 번 더 붙들어 고리로만 쓴다(`false`).
    fn pin(&self, pid: u32) -> Option<(Box<dyn Pinned>, bool)> {
        let refused = match self.group.pin(pid, true) {
            Ok(found) => return found.map(|pin| (pin, true)),
            Err(e) => e,
        };
        match self.group.pin(pid, false) {
            Ok(Some(pin)) => {
                tracing::debug!(
                    agent = self.tag.agent(),
                    root_pid = self.tag.root_pid,
                    pid,
                    "끝내기 권한이 거절돼 고리로만 붙들었다: {refused}"
                );
                Some((pin, false))
            }
            Ok(None) => None,
            Err(e) => {
                tracing::debug!(
                    agent = self.tag.agent(),
                    root_pid = self.tag.root_pid,
                    pid,
                    "가입한 프로세스를 붙들지 못했다: {e}"
                );
                None
            }
        }
    }

    /// 정상 끝 — 켜진 기록을 끈다. 물러나는 무리는 아직 붙어 있으니 포트를 놓기 전에 뗀다(`Births` 의 규칙).
    fn finish(&self, end: End) {
        drop(self.recorder.stop(self.recorder.active()));
        let reason = match end {
            End::Gone => "gone",
            End::Retiring => {
                // 곧 무리가 닫히므로 떼기 실패는 적기만 한다.
                if let Err(e) = self.group.unwatch_births() {
                    tracing::debug!(
                        agent = self.tag.agent(),
                        root_pid = self.tag.root_pid,
                        "물러나는 무리에서 가입 알림 포트를 못 뗐다: {e}"
                    );
                }
                "retiring"
            }
        };
        tracing::debug!(
            agent = self.tag.agent(),
            root_pid = self.tag.root_pid,
            reason,
            "가입 알림 듣기를 끝낸다"
        );
    }

    /// 실패 모드 — 포트를 굳히고 켜진 기록을 끄고 뗀다. 참 = 뗐다(무리가 이미 없는 것도 뗀 것이다).
    fn fail(&self, failure: Failure) -> bool {
        self.recorder.fail_port();
        drop(self.recorder.stop(self.recorder.active()));
        let (detached, port) = match self.group.unwatch_births() {
            Ok(()) => (true, "뗐다".to_owned()),
            Err(e) if e.kind() == GROUP_GONE => (true, "무리가 이미 없다".to_owned()),
            Err(e) => (false, format!("못 떼어 비우기만 한다({e})")),
        };
        match failure {
            Failure::Take(e) => tracing::warn!(
                agent = self.tag.agent(),
                root_pid = self.tag.root_pid,
                detached,
                "가입 알림을 꺼내지 못해 탄생 기록을 멈췄다 — 이 화신은 끊기 잔여물을 정리하지 않는다 · 포트: {port}: {e}"
            ),
            Failure::Panic(message) => tracing::error!(
                agent = self.tag.agent(),
                root_pid = self.tag.root_pid,
                detached,
                "가입 알림 듣기가 패닉해 탄생 기록을 멈췄다 — 이 화신은 끊기 잔여물을 정리하지 않는다 · 포트: {port}: {message}"
            ),
        }
        detached
    }
}

fn panic_text(payload: &(dyn Any + Send)) -> &str {
    if let Some(text) = payload.downcast_ref::<&str>() {
        text
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text
    } else {
        "알 수 없는 패닉"
    }
}

/// 생성부터 지금까지(µs) — 로그의 「가입 → 엶」 지연 칸에만 쓴다(고르기는 벽시계를 보지 않는다 — TRD §3-6). 생성을 못
/// 읽었거나 시계가 그 앞이면 `None`.
fn born_ago_us(create: u64) -> Option<u64> {
    if create == 0 {
        return None;
    }
    let since = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
    let now = u64::try_from(since.as_nanos() / 100)
        .ok()?
        .checked_add(UNIX_EPOCH_FILETIME)?;
    now.checked_sub(create).map(|ticks| ticks / 10)
}

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

fn micros(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

// ── 끊기 문 · 에피소드 · 일꾼 ───────────────────────────────────────────────────────────────

/// 가장 최근에 쓰인 끊기 줄에서 이만큼 턴 끝이 없으면 판을 열고, 무엇이든 끝낸 판 뒤의 확인도 이 간격이다. 근거
/// (스파이크 2026-09-29): 멈춤 아닌 끊기의 `result` ≤ 1394 ms · 끊기 뒤 생긴 정상 프로세스의 최장 수명 1171 ms ·
/// 끊기 +3 s 에 살아 있던 것은 11/11 시행에서 멈춤 잔여물뿐.
// ADR-0257
// ADR-0262
pub(super) const INTERRUPT_LEFTOVER_GRACE: Duration = Duration::from_secs(3);

/// 스냅숏이 붙드는 멤버 수의 상한 — 명단이 이보다 길면 그 에피소드는 `Failed(too_many)` 다.
pub(super) const PIN_MAX: usize = 256;

/// 스냅숏에서 받아들이는 뿌리 자식의 수 — 콘솔 호스트 · claude(Q2 — 추론). 넘으면 `Failed(shape)`.
const ROOT_CHILDREN_MAX: usize = 2;

/// 한 판이 끝내기까지 가는 차례의 상한 — 잰 가장 긴 판이 2 차례(p3bg: 주인 → 그 fork)이고 한 차례를 여유로 둔다.
/// 그 뒤에도 끝낼 후보가 남으면 끝내지 않고 warn `rounds_exhausted`.
// ADR-0262
pub(super) const PASS_ROUNDS: u32 = 3;

/// 한 차례에 끝낸 것들의 끝남 확인 마감 — 차례마다 하나를 나눠 쓴다. 그래서 판의 기다림은 많아야
/// [`PASS_ROUNDS`] × 이것이다.
pub(super) const KILL_CONFIRM: Duration = Duration::from_millis(200);

const WORKER_THREAD: &str = "engram-claude-leftover";
const LISTENER_THREAD: &str = "engram-claude-births";

/// 정리기가 읽는 단조 시계 · 기다림 · 스레드 기동 — 실물 = [`SystemClock`]. 일꾼과 듣는 스레드가 이것으로 뜬다.
///
/// ★[`now`](Clock::now) 는 문 자물쇠 안에서도 부른다★ — 다른 자물쇠를 잡거나 기다리지 않는다.
// ADR-0262
// ADR-0275
pub(super) trait LeftoverClock: Clock {
    fn sleep(&self, d: Duration);
    /// `body` 를 `name` 스레드로 띄운다. join 하는 이는 없다. `Err` = `body` 는 버려졌고 돌지 않았다.
    fn spawn(&self, name: &str, body: Box<dyn FnOnce() + Send>) -> io::Result<()>;
}

impl LeftoverClock for SystemClock {
    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }

    fn spawn(&self, name: &str, body: Box<dyn FnOnce() + Send>) -> io::Result<()> {
        std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(body)
            .map(drop)
    }
}

/// 에피소드의 단계.
///
/// - `Waiting` = 판을 기다린다(쓰였으면 가장 최근 끊기 + N 에) · `Cleaning` = 일꾼이 판을 도는 중.
/// - `Spent.at` = 무엇이든 끝낸 판이 끝난 때 — `at + N` 에도 같은 에피소드면 warn 하고 `Done`(결정 2 — 더 하지 않는다).
/// - `Done` = 더 판이 없다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Waiting,
    Cleaning,
    Spent { at: Instant },
    Done,
}

/// 스냅숏이 서지 않은 사유 — 그 에피소드의 판은 아무것도 끝내지 않는다(warn `snapshot_failed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SnapCause {
    /// 명단이 불완전했다.
    List,
    /// 명단에 뿌리가 없거나 뿌리를 못 붙들었다.
    Root,
    TooMany,
    /// 뿌리 자식이 [`ROOT_CHILDREN_MAX`] 를 넘는다.
    Shape,
    /// 가입 알림 포트를 못 쓴다 · 판 때 그 기록이 이미 꺼졌다.
    Port,
}

impl SnapCause {
    /// 로그 필드 `cause` 의 값.
    fn label(self) -> &'static str {
        match self {
            SnapCause::List => "list",
            SnapCause::Root => "root",
            SnapCause::TooMany => "too_many",
            SnapCause::Shape => "shape",
            SnapCause::Port => "port",
        }
    }
}

struct Member {
    pid: u32,
    pin: Box<dyn Pinned>,
}

/// 새 에피소드를 세울 때 우리 무리에 있던 멤버 — 조회 권한만으로 붙들어 쥔다(쥐는 동안 번호가 재사용되지 않는다).
/// 뿌리는 늘 붙들었다. 마지막 `Arc` 를 버리면 핸들이 닫힌다 — 두 자물쇠 밖에서 버린다.
struct Snapshot {
    root: Member,
    others: Vec<Member>,
}

impl Snapshot {
    /// 고르기가 보는 고리 — 뿌리가 처음이다. 끝내기 권한 없이 붙들어 `killable` 은 거짓이다.
    fn links(&self) -> Vec<Link<'_>> {
        std::iter::once(&self.root)
            .chain(&self.others)
            .map(|m| Link {
                pid: m.pid,
                facts: m.pin.facts(),
                seq: None,
                killable: false,
            })
            .collect()
    }
}

/// 에피소드가 쥔 스냅숏. `Released` = 판이 끝나 놓았다.
#[derive(Clone)]
enum SnapState {
    Taken(Arc<Snapshot>),
    Failed(SnapCause),
    Released,
}

impl SnapState {
    fn label(&self) -> &'static str {
        match self {
            SnapState::Taken(_) => "taken",
            SnapState::Failed(cause) => cause.label(),
            SnapState::Released => "released",
        }
    }

    fn members(&self) -> usize {
        match self {
            SnapState::Taken(s) => s.others.len().saturating_add(1),
            SnapState::Failed(_) | SnapState::Released => 0,
        }
    }
}

impl fmt::Debug for SnapState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// 끊기 한 판 — 새 에피소드를 세운 끊기로 시작해 턴 끝 · 새 입력 · 갈아 끼우기 · 물러남 · 판 끝으로 끝난다.
///
/// - `first_mono` = 세운 때 · `last_mono` = 가장 최근 끊기(받아들인 때 · 쓰인 때 중 늦은 것) — N 의 기준.
/// - `written` = 이 에피소드의 줄이 쓰였고 그때의 쓰기 명단 `w`(정렬)를 얻었다 — 그 전에는 판이 없다.
/// - `rec` = 이 에피소드가 켠 기록 번호 · `0` = 놓았다.
/// - `killed_any` = 어느 판이든 하나라도 끝냈다.
#[derive(Debug)]
struct Episode {
    gen: u64,
    first_mono: Instant,
    last_mono: Instant,
    written: bool,
    w: Option<Arc<[u32]>>,
    snap: SnapState,
    rec: u64,
    killed_any: bool,
    phase: Phase,
}

impl Episode {
    fn is_live(&self) -> bool {
        matches!(self.phase, Phase::Waiting | Phase::Cleaning)
    }

    /// 스냅숏과 기록을 내놓는다 — 에피소드는 자리에 남는다.
    fn release(&mut self) -> Released {
        let snap = match std::mem::replace(&mut self.snap, SnapState::Released) {
            SnapState::Taken(snap) => Some(snap),
            SnapState::Failed(_) | SnapState::Released => None,
        };
        Released {
            snap,
            w: None,
            rec: std::mem::take(&mut self.rec),
        }
    }

    /// 상태에서 빼낸 에피소드가 쥔 것 전부.
    fn into_released(mut self) -> Released {
        let mut released = self.release();
        released.w = self.w.take();
        released
    }
}

/// 문 자물쇠에서 빼낸 것 — 받은 쪽이 자물쇠를 놓은 뒤 [`GateCell::release`] 로 버리고 기록을 끈다.
#[must_use]
#[derive(Default)]
struct Released {
    snap: Option<Arc<Snapshot>>,
    w: Option<Arc<[u32]>>,
    rec: u64,
}

/// 여는 이가 구간 ① 에서 찍은 값 — 구간 ② 가 그 사이에 무엇이 왔는지 가른다.
#[derive(Debug, Clone, Copy)]
struct Mark {
    closes: u64,
    delivers: u64,
    next_gen: u64,
}

/// [`GateCell`] 의 자물쇠 안 상태. 전이는 순수하다 — 시각 · 원자 값은 인자로 받고, OS 를 부르지 않고, 할당 · 해제를
/// 하지 않고, 패닉하지 않는다. 빼낸 것은 돌려주고 부른 쪽이 자물쇠를 놓은 뒤 버린다.
///
/// - `open` = 끊을 턴이 열려 있다 — 쓰는 이는 decoder 하나.
/// - `closes` · `delivers` = 턴 끝 · 새 입력을 센 값.
/// - `opening` = 여는 이 하나가 두 구간 사이에 있다.
/// - `worker` = 이 화신에 일꾼이 떠 있다(기동 중 포함).
#[derive(Debug, Default)]
struct GateState {
    open: bool,
    closes: u64,
    delivers: u64,
    opening: bool,
    episode: Option<Episode>,
    next_gen: u64,
    worker: bool,
}

/// 구간 ① 의 답.
#[derive(Debug)]
enum Begin {
    /// 문이 닫혔다 — 줄 없음.
    Closed,
    /// 산 에피소드에 이어 적었다 — 그 세대로 쓰기 확인을 받는다.
    Continue { gen: u64 },
    /// 다른 끊기가 여는 중이다 — 줄만 준다(그 에피소드가 이 턴을 맡는다).
    Busy,
    /// 이 끊기가 여는 이다.
    Open(Mark),
}

/// 구간 ② 의 답. 세우지 못한 스냅숏은 돌려준다.
enum Settle {
    /// 두 구간 사이에 턴이 닫혔다 — 줄 없음.
    NoLine(SnapState),
    /// 에피소드 없이 줄만 준다.
    LineOnly { snap: SnapState, why: Stale },
    /// `old` = 갈아 끼운 옛 에피소드.
    Opened { gen: u64, old: Option<Episode> },
}

/// 구간 ② 가 에피소드를 세우지 않은 까닭(debug `stale`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stale {
    /// 켠 기록이 이미 꺼졌다(듣는 스레드의 끝 · 실패).
    Record,
    Delivered,
    Generation,
    /// 산 에피소드가 있다 — 여는 이가 하나라 닿지 않는 갈래.
    Live,
}

impl Stale {
    fn label(self) -> &'static str {
        match self {
            Stale::Record => "record",
            Stale::Delivered => "delivered",
            Stale::Generation => "generation",
            Stale::Live => "live",
        }
    }
}

/// 쓰기 확인 한 번의 결과. `back` = 쓰지 않은 쓰기 명단 — 부른 쪽이 놓은 뒤 버린다.
struct Noted {
    back: Option<Arc<[u32]>>,
    note: WriteNote,
    spawn: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriteNote {
    /// 첫 쓰임 — 쓰기 명단을 받았다.
    First { w_len: usize },
    /// 쓰였지만 명단이 없어 쓰임으로 세지 않았다.
    Unlisted,
    /// 이미 쓰인 에피소드의 뒤 끊기.
    Again,
    /// 그 세대의 산 에피소드가 없다.
    Stale,
}

/// 일꾼의 한 걸음.
enum Step {
    /// 일꾼이 끝난다(`worker` 는 이미 내렸다). `retired` = 물러나는 무리라 치운 에피소드.
    Exit {
        retired: Option<Episode>,
        released: Released,
    },
    Sleep(Duration),
    Pass(Ticket),
    /// 무엇이든 끝낸 판 뒤 N 에도 같은 에피소드다 — 에피소드는 이미 `Done`.
    StillHung {
        waited: Duration,
    },
}

/// 판 한 번의 입력 — `Cleaning` 으로 바꾸는 자물쇠 구간에서 뜬다. 쥔 `Arc` 는 판이 끝난 뒤 자물쇠 밖에서 버린다.
///
/// - `rec` = 이 에피소드의 기록 번호 — 판이 그 기록을 복사한다(`active` 가 다르면 빈 것).
/// - `w` = 첫 쓰임의 쓰기 명단(정렬).
struct Ticket {
    gen: u64,
    first_mono: Instant,
    rec: u64,
    w: Arc<[u32]>,
    snap: SnapState,
}

/// 끝내기 확정 한 구간의 판정.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Commit {
    Go,
    Retiring,
    /// 판 도중 턴 끝 · 새 끊기 · 새 입력 — 판 전체를 멈춘다(debug `superseded`).
    Superseded,
}

impl GateState {
    fn open_turn(&mut self) {
        self.open = true;
    }

    /// 새 입력이 CLI 에 닿았다 — 에피소드를 버리고 문을 연다.
    fn deliver(&mut self) -> Option<Episode> {
        self.delivers = self.delivers.wrapping_add(1);
        self.open = true;
        self.episode.take()
    }

    fn close_turn(&mut self) -> Option<Episode> {
        self.open = false;
        self.closes = self.closes.wrapping_add(1);
        self.episode.take()
    }

    /// 구간 ①. 산 에피소드에 이어 적을 때 도는 판은 다음 끝내기 확정에서 멈추고, 가장 최근 끊기 + N 을 다시 기다린다.
    fn begin_interrupt(&mut self, now: Instant) -> Begin {
        if !self.open {
            return Begin::Closed;
        }
        if let Some(ep) = self.episode.as_mut().filter(|ep| ep.is_live()) {
            ep.phase = Phase::Waiting;
            ep.last_mono = ep.last_mono.max(now);
            return Begin::Continue { gen: ep.gen };
        }
        if self.opening {
            return Begin::Busy;
        }
        self.opening = true;
        Begin::Open(Mark {
            closes: self.closes,
            delivers: self.delivers,
            next_gen: self.next_gen,
        })
    }

    /// 구간 ② — 여는 이가 켠 기록 `rec`(지금 켜진 번호 = `active`)과 찍은 스냅숏으로 새 에피소드를 세운다.
    fn settle_interrupt(
        &mut self,
        mark: Mark,
        rec: u64,
        active: u64,
        snap: SnapState,
        now: Instant,
    ) -> Settle {
        self.opening = false;
        if !self.open || self.closes != mark.closes {
            return Settle::NoLine(snap);
        }
        let stale = if active != rec {
            Some(Stale::Record)
        } else if self.delivers != mark.delivers {
            Some(Stale::Delivered)
        } else if self.next_gen != mark.next_gen {
            Some(Stale::Generation)
        } else if self.episode.as_ref().is_some_and(Episode::is_live) {
            Some(Stale::Live)
        } else {
            None
        };
        if let Some(why) = stale {
            return Settle::LineOnly { snap, why };
        }
        let gen = self.next_gen;
        self.next_gen = gen.wrapping_add(1);
        let old = self.episode.replace(Episode {
            gen,
            first_mono: now,
            last_mono: now,
            written: false,
            w: None,
            snap,
            rec,
            killed_any: false,
            phase: Phase::Waiting,
        });
        Settle::Opened { gen, old }
    }

    /// 쓰기 확인 — 같은 세대의 산 에피소드면 가장 최근 끊기를 당기고, 첫 쓰임은 명단 `w` 가 있을 때만 쓰임으로 센다.
    /// 그 밖의 `w` 는 돌려준다. 쓰인 에피소드에 일꾼이 없으면 띄울 몫을 준다.
    fn note_written(&mut self, gen: u64, w: Option<Arc<[u32]>>, now: Instant) -> Noted {
        let worker = self.worker;
        let Some(ep) = self
            .episode
            .as_mut()
            .filter(|ep| ep.gen == gen && ep.is_live())
        else {
            return Noted {
                back: w,
                note: WriteNote::Stale,
                spawn: false,
            };
        };
        ep.last_mono = ep.last_mono.max(now);
        let (back, note) = match (ep.written, w) {
            (true, w) => (w, WriteNote::Again),
            (false, Some(w)) => {
                let w_len = w.len();
                ep.written = true;
                ep.w = Some(w);
                (None, WriteNote::First { w_len })
            }
            (false, None) => (None, WriteNote::Unlisted),
        };
        let spawn = ep.written && !worker;
        if spawn {
            self.worker = true;
        }
        Noted { back, note, spawn }
    }

    /// 일꾼의 한 걸음(TRD §3-3) — 표식이면 에피소드를 치우고 끝 · 없음 · `Done` · 쓰이지 않은 `Waiting` → 끝 · 쓰인
    /// `Waiting` 은 N 전이면 잔다 · 지났으면 `Cleaning` + 판 · `Spent` 는 N 전이면 잔다 · 지났으면 `Done` + warn.
    fn next_step(&mut self, now: Instant, retiring: bool) -> Step {
        if retiring {
            self.worker = false;
            return Step::Exit {
                retired: self.episode.take(),
                released: Released::default(),
            };
        }
        let Some(ep) = self.episode.as_mut() else {
            self.worker = false;
            return Step::Exit {
                retired: None,
                released: Released::default(),
            };
        };
        let released = match ep.phase {
            Phase::Done => Released::default(),
            Phase::Waiting if !ep.written => Released::default(),
            Phase::Waiting => match ep.last_mono.checked_add(INTERRUPT_LEFTOVER_GRACE) {
                Some(due) if now < due => {
                    return Step::Sleep(due.saturating_duration_since(now));
                }
                Some(_) => {
                    if let Some(w) = ep.w.clone() {
                        ep.phase = Phase::Cleaning;
                        return Step::Pass(Ticket {
                            gen: ep.gen,
                            first_mono: ep.first_mono,
                            rec: ep.rec,
                            w,
                            snap: ep.snap.clone(),
                        });
                    }
                    // 「쓰였으면 명단이 있다」가 깨진 자리 — 판을 세울 명단이 없다.
                    ep.phase = Phase::Done;
                    ep.release()
                }
                None => {
                    ep.phase = Phase::Done;
                    ep.release()
                }
            },
            // `Cleaning` 은 일꾼만 세우고 그 판 끝이 늘 걷으므로 앞으로의 결함 없이는 오지 않는다. 와도 한 에피소드의
            // 완료된 판은 한 번이라 다시 돌지 않는다.
            Phase::Cleaning => {
                ep.phase = Phase::Done;
                ep.release()
            }
            Phase::Spent { at } => match at.checked_add(INTERRUPT_LEFTOVER_GRACE) {
                Some(due) if now < due => {
                    return Step::Sleep(due.saturating_duration_since(now));
                }
                _ => {
                    ep.phase = Phase::Done;
                    return Step::StillHung {
                        waited: now.saturating_duration_since(ep.first_mono),
                    };
                }
            },
        };
        self.worker = false;
        Step::Exit {
            retired: None,
            released,
        }
    }

    /// 끝내기 확정의 재확인 — 표식 없음 · 같은 세대 · 쓰임 · 아직 `Cleaning`(판 도중 끊기 · 턴 끝 · 새 입력이 없었다) ·
    /// 가장 최근 끊기 + N ≤ 지금.
    fn still_due(&self, gen: u64, now: Instant, retiring: bool) -> bool {
        !retiring
            && self.episode.as_ref().is_some_and(|ep| {
                ep.gen == gen
                    && ep.written
                    && ep.phase == Phase::Cleaning
                    && ep
                        .last_mono
                        .checked_add(INTERRUPT_LEFTOVER_GRACE)
                        .is_some_and(|due| due <= now)
            })
    }

    fn commit_check(&self, gen: u64, now: Instant, retiring: bool) -> Commit {
        if retiring {
            Commit::Retiring
        } else if self.still_due(gen, now, retiring) {
            Commit::Go
        } else {
            Commit::Superseded
        }
    }

    /// 끝내기 확정에서 끝내기 호출이 받아들여졌다.
    fn note_kill(&mut self, gen: u64) {
        if let Some(ep) = self.episode.as_mut().filter(|ep| ep.gen == gen) {
            ep.killed_any = true;
        }
    }

    /// 판 끝 — 같은 세대가 아직 `Cleaning` 일 때만 `Spent`(무엇이든 끝냈다)/`Done` 으로 매듭짓고 스냅숏 · 기록을
    /// 내놓는다. 판 도중 끊기로 `Waiting` 에 돌아갔으면 쥔 채 다음 판을 기다린다.
    fn finish_pass(&mut self, gen: u64, killed: bool, now: Instant) -> Released {
        let Some(ep) = self.episode.as_mut().filter(|ep| ep.gen == gen) else {
            return Released::default();
        };
        ep.killed_any |= killed;
        let released = if ep.phase == Phase::Cleaning {
            ep.phase = if ep.killed_any {
                Phase::Spent { at: now }
            } else {
                Phase::Done
            };
            ep.release()
        } else {
            Released::default()
        };
        released
    }

    /// 일꾼이 없어졌다(기동 실패 · 패닉) — 세대가 무엇이든 지금 에피소드를 `Done` 으로 내린다. 그 사이 다른 끊기가
    /// 세운 에피소드도 「일꾼이 떠 있다」고 믿고 아무도 안 띄웠기 때문이다. 곧바로 다시 띄우지 않는다.
    fn abandon(&mut self) -> Released {
        self.worker = false;
        match self.episode.as_mut() {
            Some(ep) => {
                ep.phase = Phase::Done;
                ep.release()
            }
            None => Released::default(),
        }
    }
}

/// 한 화신의 정리기 — 무리 손잡이 · 탄생 기록 · 포트 확보 · 시계 · 뿌리 · 물러남 표시 · 로그 귀속. 조립하지 못하면
/// 만들지 않는다(그때 [`GateCell`] 은 문만 여닫는다).
// ADR-0262
pub(super) struct Cleaner<G: Group = ProcessGroup> {
    group: Arc<G>,
    recorder: Arc<Recorder>,
    watch: BirthWatch,
    clock: Arc<dyn LeftoverClock>,
    root_pid: u32,
    retiring: RetiringSignal,
    tag: LogTag,
    /// 쓰기 명단 실패를 warn 으로 남겼다 — 화신마다 한 번.
    unlisted_warned: AtomicBool,
}

impl Cleaner<ProcessGroup> {
    pub(super) fn new(
        group: Arc<ProcessGroup>,
        clock: Arc<dyn LeftoverClock>,
        root_pid: u32,
        tag: LogTag,
    ) -> Self {
        Self::with_parts(group, Arc::new(Recorder::new()), clock, root_pid, tag)
    }
}

impl<G: Group> Cleaner<G> {
    fn with_parts(
        group: Arc<G>,
        recorder: Arc<Recorder>,
        clock: Arc<dyn LeftoverClock>,
        root_pid: u32,
        tag: LogTag,
    ) -> Self {
        let retiring = group.retiring();
        Self {
            group,
            recorder,
            watch: BirthWatch::new(),
            clock,
            root_pid,
            retiring,
            tag,
            unlisted_warned: AtomicBool::new(false),
        }
    }

    /// 포트 · 듣는 스레드 확보(처음 한 번) — 듣는 스레드도 시계로 띄운다.
    fn ensure_port(&self) -> Result<(), ()> {
        self.watch
            .ensure_with(&self.group, &self.recorder, &self.tag, |body| {
                self.clock.spawn(LISTENER_THREAD, body)
            })
    }
}

/// 새 에피소드의 스냅숏 [고름] — 완전한 명단의 멤버마다 조회 권한만으로 붙들어 사실을 읽는다. 못 붙든 뿌리 밖 멤버
/// (끝남 · 무리 밖 · 거절)는 빠진다 — 알려지지 않은 고리라 그 아래는 놓친다. 두 자물쇠 밖에서 부른다.
// ADR-0262
fn take_snapshot<G: Group>(group: &G, root_pid: u32) -> SnapState {
    let Ok(pids) = group.member_pids() else {
        return SnapState::Failed(SnapCause::List);
    };
    if pids.len() > PIN_MAX {
        return SnapState::Failed(SnapCause::TooMany);
    }
    if root_pid == 0 || !pids.contains(&root_pid) {
        return SnapState::Failed(SnapCause::Root);
    }
    let mut root = None;
    let mut others = Vec::with_capacity(pids.len());
    for pid in pids {
        let Ok(Some(pin)) = group.pin(pid, false) else {
            continue;
        };
        let member = Member { pid, pin };
        if pid == root_pid {
            root = Some(member);
        } else {
            others.push(member);
        }
    }
    let Some(root) = root else {
        return SnapState::Failed(SnapCause::Root);
    };
    let root_children = others
        .iter()
        .filter(|m| m.pin.facts().ppid == root_pid)
        .count();
    if root_children > ROOT_CHILDREN_MAX {
        return SnapState::Failed(SnapCause::Shape);
    }
    SnapState::Taken(Arc::new(Snapshot { root, others }))
}

/// 쓰기 명단을 찍지 못한 까닭.
enum Unlisted {
    List(io::Error),
    /// 명단이 비었다 — 통로가 사라졌다.
    Gone,
}

impl fmt::Display for Unlisted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unlisted::List(e) => write!(f, "명단 {e}"),
            Unlisted::Gone => f.write_str("무리가 없다"),
        }
    }
}

/// 쓰기 명단 W — 줄이 쓰인 직후의 완전한 멤버 번호(정렬). 아무 자물쇠 없이 부른다(손잡이가 그 호출 동안만 Job 을
/// 올린다). 후보는 W 밖이어야 하므로 W 를 못 찍으면 그 쓰임으로는 판이 서지 않는다.
fn write_list<G: Group>(group: &G) -> Result<Arc<[u32]>, Unlisted> {
    let mut pids = group.member_pids().map_err(Unlisted::List)?;
    if pids.is_empty() {
        return Err(Unlisted::Gone);
    }
    pids.sort_unstable();
    Ok(pids.into())
}

/// 턴 열림 문 + 끊기 에피소드 — 화신마다 하나, 끊기 줄 함수 · decoder · 쓰기 확인 · 일꾼이 같은 `Arc` 로 쥔다.
// ADR-0262
pub(super) struct GateCell<G: Group = ProcessGroup> {
    state: Mutex<GateState>,
    cleaner: Option<Arc<Cleaner<G>>>,
}

/// [`GateCell`] 의 Debug 가 자물쇠에서 베껴 오는 것.
#[derive(Debug, Clone, Copy)]
struct Seen {
    open: bool,
    opening: bool,
    episode: Option<(u64, Phase, bool)>,
}

impl Seen {
    fn of(state: &GateState) -> Self {
        Self {
            open: state.open,
            opening: state.opening,
            episode: state
                .episode
                .as_ref()
                .map(|ep| (ep.gen, ep.phase, ep.written)),
        }
    }
}

impl<G: Group> fmt::Debug for GateCell<G> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 쓰기는 자물쇠를 놓은 뒤다 — 포매터가 곧바로 파일 · 콘솔에 쓸 수 있다. 기다리지도 않는다: 자물쇠를 쥔 채
        // 이것을 찍는 결함이 생겨도 제 자신을 기다려 멈추지 않게.
        // ADR-0275: base `sync` 를 쓰지 않는다 — 그쪽은 기다리는 `lock` 뿐이다. 오염 갈래만 여기서 되찾는다.
        let seen = match self.state.try_lock() {
            Ok(state) => Some(Seen::of(&state)),
            Err(TryLockError::Poisoned(state)) => Some(Seen::of(&state.into_inner())),
            Err(TryLockError::WouldBlock) => None,
        };
        let mut out = f.debug_struct("GateCell");
        match seen {
            Some(seen) => out
                .field("open", &seen.open)
                .field("opening", &seen.opening)
                .field("episode", &seen.episode),
            None => out.field("state", &format_args!("<locked>")),
        };
        out.field("cleaner", &self.cleaner.is_some()).finish()
    }
}

/// 여는 이 가드 — 구간 ① 에서 여는 이가 된 뒤 세운다. 새 에피소드를 세우지 못한 모든 출구(패닉 · 줄 없음 · 줄만)에서
/// 문 자물쇠를 잡아 `opening` 을 내리고(구간 ② 에 닿기 전일 때만) 놓은 **뒤에** 켠 기록을 끈다.
///
/// ★구간 ② 뒤에는 `opening` 을 건드리지 않는다★ — ② 가 이미 내렸고, 그 뒤 다른 끊기가 여는 이가 됐을 수 있다(그
/// 표시를 지우면 여는 이가 둘이 된다). 기록은 번호로 끄므로 그 사이 새 기록이 켜졌으면 아무것도 안 한다.
struct Opener<'a, G: Group> {
    cell: &'a GateCell<G>,
    recorder: &'a Recorder,
    rec: u64,
    holds_opening: bool,
    holds_record: bool,
}

impl<G: Group> Drop for Opener<'_, G> {
    fn drop(&mut self) {
        if self.holds_opening {
            self.cell.lock().opening = false;
        }
        if self.holds_record {
            drop(self.recorder.stop(self.rec));
        }
    }
}

impl<G: Group> GateCell<G> {
    /// `cleaner` 가 `None` 이면 문만 여닫는다 — 끊기 줄은 문이 열렸을 때만 주고 에피소드는 없다.
    pub(super) fn new(cleaner: Option<Arc<Cleaner<G>>>) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(GateState::default()),
            cleaner,
        })
    }

    fn lock(&self) -> MutexGuard<'_, GateState> {
        sync::lock(&self.state)
    }

    #[cfg(test)]
    pub(super) fn is_open(&self) -> bool {
        self.lock().open
    }

    /// 끊기 줄 함수의 몸통 — 문이 닫혔으면 `None`(줄 없음). 줄을 주면 `on_written` 은 이 끊기가 에피소드를 세웠거나 산
    /// 에피소드에 이어 적었을 때만 있다(다른 끊기가 여는 중 · 두 구간 사이에 기록 · 입력 · 세대가 바뀜 → 줄만).
    ///
    /// 여는 이는 하나다: 자물쇠 밖에서 포트를 확보하고 → 기록을 켜고 → 스냅숏을 찍는다. ★기록을 스냅숏 앞에 켠다★ —
    /// 그 사이 태어난 것은 기록되거나(명단 뒤) 스냅숏에 든다(명단 앞).
    // ADR-0262
    pub(super) fn interrupt(self: &Arc<Self>, bytes: Vec<u8>) -> Option<InterruptOut> {
        let Some(cleaner) = self.cleaner.as_ref() else {
            let open = self.lock().open;
            return open.then(|| InterruptOut {
                bytes,
                on_written: None,
            });
        };
        let begun = {
            let mut state = self.lock();
            let now = cleaner.clock.now();
            state.begin_interrupt(now)
        };
        let mark = match begun {
            Begin::Closed => return None,
            Begin::Continue { gen } => {
                return Some(InterruptOut {
                    bytes,
                    on_written: Some(self.on_written(gen)),
                });
            }
            Begin::Busy => {
                tracing::debug!(
                    agent = cleaner.tag.agent(),
                    root_pid = cleaner.tag.root_pid,
                    "다른 끊기가 에피소드를 여는 중이라 끊기 줄만 준다"
                );
                return Some(InterruptOut {
                    bytes,
                    on_written: None,
                });
            }
            Begin::Open(mark) => mark,
        };

        let mut opener = Opener {
            cell: self,
            recorder: &cleaner.recorder,
            rec: 0,
            holds_opening: true,
            holds_record: false,
        };
        let port = cleaner.ensure_port();
        let (rec, left) = cleaner.recorder.start();
        opener.rec = rec;
        opener.holds_record = true;
        drop(left);
        let snap = match port {
            Ok(()) => take_snapshot(&*cleaner.group, cleaner.root_pid),
            Err(()) => SnapState::Failed(SnapCause::Port),
        };
        let (snapshot, members) = (snap.label(), snap.members());
        let settled = {
            let mut state = self.lock();
            let active = cleaner.recorder.active();
            let now = cleaner.clock.now();
            state.settle_interrupt(mark, rec, active, snap, now)
        };
        opener.holds_opening = false;

        match settled {
            Settle::Opened { gen, old } => {
                opener.holds_record = false;
                drop(opener);
                if let Some(old) = old {
                    self.release(old.into_released());
                }
                tracing::debug!(
                    agent = cleaner.tag.agent(),
                    root_pid = cleaner.tag.root_pid,
                    gen,
                    rec,
                    snapshot,
                    members,
                    births = cleaner.recorder.len(),
                    "끊기 에피소드를 열었다"
                );
                Some(InterruptOut {
                    bytes,
                    on_written: Some(self.on_written(gen)),
                })
            }
            Settle::NoLine(snap) => {
                drop(opener);
                drop(snap);
                tracing::debug!(
                    agent = cleaner.tag.agent(),
                    root_pid = cleaner.tag.root_pid,
                    "에피소드를 여는 사이 턴이 닫혀 끊기 줄을 주지 않는다"
                );
                None
            }
            Settle::LineOnly { snap, why } => {
                drop(opener);
                drop(snap);
                if why == Stale::Live {
                    tracing::warn!(
                        agent = cleaner.tag.agent(),
                        root_pid = cleaner.tag.root_pid,
                        "여는 이가 하나인데 에피소드를 세우는 자리에 산 에피소드가 있다 — 줄만 준다"
                    );
                } else {
                    tracing::debug!(
                        agent = cleaner.tag.agent(),
                        root_pid = cleaner.tag.root_pid,
                        reason = why.label(),
                        "stale — 에피소드를 여는 사이 기록 · 입력이 바뀌어 끊기 줄만 준다"
                    );
                }
                debug_assert!(why != Stale::Live, "여는 이가 둘이었다");
                Some(InterruptOut {
                    bytes,
                    on_written: None,
                })
            }
        }
    }

    /// 줄이 쓰인 뒤 라이터가 부를 것 — 쓰기 명단을 찍어 그 세대에 적는다.
    fn on_written(self: &Arc<Self>, gen: u64) -> OnWritten {
        let cell = Arc::clone(self);
        Box::new(move || cell.written(gen))
    }

    /// 쓰기 확인 — 라이터 스레드가 아무 락 없이 부른다. 명단은 문 자물쇠 밖에서 찍고, 쓰지 않은 명단도 밖에서 버린다.
    fn written(self: &Arc<Self>, gen: u64) {
        let Some(cleaner) = self.cleaner.as_ref() else {
            return;
        };
        let began = cleaner.clock.now();
        let listed = write_list(&*cleaner.group);
        let w_us = micros(cleaner.clock.now().saturating_duration_since(began));
        let (w, unlisted) = match listed {
            Ok(w) => (Some(w), None),
            Err(e) => (None, Some(e)),
        };
        let Noted { back, note, spawn } = {
            let mut state = self.lock();
            let now = cleaner.clock.now();
            state.note_written(gen, w, now)
        };
        drop(back);
        match (note, unlisted) {
            (WriteNote::First { w_len }, _) => tracing::debug!(
                agent = cleaner.tag.agent(),
                root_pid = cleaner.tag.root_pid,
                gen,
                w_len,
                w_us,
                "끊기 줄이 쓰여 쓰기 명단을 찍었다"
            ),
            (WriteNote::Unlisted, Some(e)) => {
                if cleaner.unlisted_warned.swap(true, Ordering::AcqRel) {
                    tracing::debug!(
                        agent = cleaner.tag.agent(),
                        root_pid = cleaner.tag.root_pid,
                        gen,
                        "끊기 줄이 쓰였지만 쓰기 명단을 못 찍었다: {e}"
                    );
                } else {
                    tracing::warn!(
                        agent = cleaner.tag.agent(),
                        root_pid = cleaner.tag.root_pid,
                        gen,
                        "끊기 줄이 쓰였지만 쓰기 명단을 못 찍어 이 끊기는 잔여물을 정리하지 않는다: {e}"
                    );
                }
            }
            _ => {}
        }
        if spawn {
            self.spawn_worker(cleaner);
        }
    }

    /// 일꾼을 띄운다 — 실패하면 지금 에피소드를 `Done` 으로 내리고 쥔 것을 놓는다.
    fn spawn_worker(self: &Arc<Self>, cleaner: &Arc<Cleaner<G>>) {
        let cell = Arc::clone(self);
        let worker_cleaner = Arc::clone(cleaner);
        let spawned = cleaner.clock.spawn(
            WORKER_THREAD,
            Box::new(move || run_worker(&cell, &worker_cleaner)),
        );
        if let Err(e) = spawned {
            let released = self.lock().abandon();
            self.release(released);
            tracing::warn!(
                agent = cleaner.tag.agent(),
                root_pid = cleaner.tag.root_pid,
                "잔여물 정리 일꾼을 띄우지 못해 이 끊기는 정리하지 않는다: {e}"
            );
        }
    }

    /// decoder 가 턴 열림을 볼 때 — 이미 열려 있으면 아무것도 안 바꾼다.
    pub(super) fn open_turn(&self) {
        self.lock().open_turn();
    }

    /// decoder 가 새 입력(라이브 `started`)을 볼 때 — 에피소드를 버리고 문을 연다(한 구간).
    pub(super) fn deliver(&self) {
        let dropped = self.lock().deliver();
        let Some(ep) = dropped else {
            return;
        };
        if ep.is_live() {
            if let Some(cleaner) = &self.cleaner {
                tracing::debug!(
                    agent = cleaner.tag.agent(),
                    root_pid = cleaner.tag.root_pid,
                    gen = ep.gen,
                    "delivered — 새 입력이 닿아 끊기 에피소드를 버렸다"
                );
            }
        }
        self.release(ep.into_released());
    }

    /// decoder 가 턴 끝을 볼 때 — 문 닫기와 에피소드 버리기를 한 구간에서.
    pub(super) fn close_turn(&self) {
        let dropped = self.lock().close_turn();
        if let Some(ep) = dropped {
            self.release(ep.into_released());
        }
    }

    /// 판 끝(TRD §3-3).
    fn end_pass(&self, cleaner: &Cleaner<G>, gen: u64, killed: bool) {
        let released = {
            let mut state = self.lock();
            let now = cleaner.clock.now();
            state.finish_pass(gen, killed, now)
        };
        self.release(released);
    }

    /// 빼낸 것을 버리고 그 기록을 끈다 — 두 자물쇠 밖에서 부른다.
    fn release(&self, released: Released) {
        let Released { snap, w, rec } = released;
        drop(snap);
        drop(w);
        if let Some(cleaner) = &self.cleaner {
            drop(cleaner.recorder.stop(rec));
        }
    }
}

/// 일꾼 몸통 — 에피소드가 없거나 · 쓰이지 않았거나 · `Done` 이 되거나 물러날 때까지 걷는다. 쥐는 것은 문과 정리기뿐이다.
///
/// ★패닉 가드는 unwind 빌드(개발 · 시험)에서만 선다 — 릴리스는 `panic = "abort"` 다★. 그래서 몸통은 패닉하지 않게
/// 짠다(풀기 · 첨자 없음 · 독 견딤). 되감기면 지금 에피소드를 `Done` 으로 내리고 쥔 것을 놓는다 — 일꾼 없이 쓰인
/// 에피소드가 남지 않게.
// ADR-0262
fn run_worker<G: Group>(cell: &Arc<GateCell<G>>, cleaner: &Arc<Cleaner<G>>) {
    let mut guard = WorkerGuard { cell, armed: true };
    while worker_turn(cell, cleaner) {}
    guard.armed = false;
}

struct WorkerGuard<'a, G: Group> {
    cell: &'a GateCell<G>,
    armed: bool,
}

impl<G: Group> Drop for WorkerGuard<'_, G> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let released = self.cell.lock().abandon();
        self.cell.release(released);
        if let Some(cleaner) = &self.cell.cleaner {
            tracing::error!(
                agent = cleaner.tag.agent(),
                root_pid = cleaner.tag.root_pid,
                "잔여물 정리 일꾼이 패닉해 이 끊기는 정리하지 않는다"
            );
        }
    }
}

/// 일꾼의 한 걸음 — 계속 걸으면 참.
fn worker_turn<G: Group>(cell: &Arc<GateCell<G>>, cleaner: &Arc<Cleaner<G>>) -> bool {
    let step = {
        let mut state = cell.lock();
        let now = cleaner.clock.now();
        state.next_step(now, cleaner.retiring.is_set())
    };
    match step {
        Step::Exit { retired, released } => {
            cell.release(released);
            if let Some(ep) = retired {
                cell.release(ep.into_released());
                tracing::debug!(
                    agent = cleaner.tag.agent(),
                    root_pid = cleaner.tag.root_pid,
                    "retiring — 물러나는 무리라 끊기 에피소드를 치우고 정리 일꾼이 끝난다"
                );
            }
            false
        }
        Step::Sleep(d) => {
            cleaner.clock.sleep(d);
            true
        }
        Step::StillHung { waited } => {
            tracing::warn!(
                agent = cleaner.tag.agent(),
                root_pid = cleaner.tag.root_pid,
                waited_ms = millis(waited),
                "잔여물을 끝낸 판 뒤에도 턴 끝이 없다 — 더 하지 않는다"
            );
            true
        }
        Step::Pass(ticket) => {
            run_pass(cell, cleaner, ticket);
            true
        }
    }
}

/// 판이 새로 끝낼 것이 없는 차례에 닿기 전에 멈춘 까닭.
#[derive(Debug)]
enum PassStop {
    /// 판의 순서 1–3 이 서지 않았거나, 끝내기 전 재확인 · 확정에서 물러남 · 뿌리 끝남을 봤다.
    NotReady(NotReady),
    /// 확정의 재확인이 거절됐다 — 판 도중 턴 끝 · 새 끊기 · 새 입력(debug `superseded`).
    Superseded,
    /// 차례의 기록 복사가 비었고 그 기록이 이미 꺼졌다 — 에피소드가 버려졌거나 듣는 스레드가 끝났다(그쪽이 따로
    /// 적는다).
    RecordGone,
    /// [`PASS_ROUNDS`] 차례를 끝내고도 끝낼 후보가 남았다(warn `rounds_exhausted`).
    RoundsExhausted,
}

/// 끝낸 것 하나 — 정리함 로그의 `terminated` 칸. `chain` = 후보부터 꼭대기 T 까지([`Verdict::Cleanup`]).
struct Killed {
    pid: u32,
    ppid: u32,
    chain: Vec<u32>,
    round: u32,
}

impl fmt::Debug for Killed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "pid={} ppid={} chain={:?} round={}",
            self.pid, self.ppid, self.chain, self.round
        )
    }
}

/// 규칙별로 뺀 후보 수 — 마지막으로 고른 차례의 것이다. 산 채 빠진 후보는 다음 차례에 다시 세어지므로 차례끼리
/// 더하지 않는다.
#[derive(Debug, Default, Clone, Copy)]
struct Excluded {
    parent_alive: usize,
    parent_unknown: usize,
    not_hook_copy: usize,
    ancestor_alive: usize,
    chain_cut: usize,
    not_hook: usize,
    no_taskkill: usize,
    unknown: usize,
}

impl Excluded {
    fn note(&mut self, verdict: &Verdict) {
        let slot = match verdict {
            Verdict::Cleanup { .. } => return,
            Verdict::ParentAlive => &mut self.parent_alive,
            Verdict::ParentUnknown => &mut self.parent_unknown,
            Verdict::NotHookCopy => &mut self.not_hook_copy,
            Verdict::AncestorAlive { .. } => &mut self.ancestor_alive,
            Verdict::ChainCut => &mut self.chain_cut,
            Verdict::NotHook => &mut self.not_hook,
            Verdict::NoTaskkill => &mut self.no_taskkill,
            Verdict::Unknown => &mut self.unknown,
        };
        *slot = slot.saturating_add(1);
    }
}

/// 판 한 번의 결과 — 로그가 읽는다.
///
/// - `stop` = `None` 이면 새로 끝낼 `Cleanup` 이 없는 차례까지 돌았다.
/// - `rounds` = 확정까지 간 차례 수(마지막의 빈 차례는 세지 않는다).
/// - `terminated` = 끝내기 호출이 받아들여진 것 · `confirmed` = 그 가운데 그 차례의 마감 안에 끝남을 확인한 수(판이
///   도중에 멈춘 차례는 확인하지 않는다).
/// - `candidates` = 첫 차례의 후보 수 · `births` = 마지막으로 복사한 기록의 크기 · `full` = 첫 차례 때 그 기록이
///   넘쳤나.
/// - `cleanups` = 확정에서 끝내기 호출까지 간 수 · `failed` = 그 가운데 끝내기 오류 · `gone` = 그 사이 스스로 끝난 수.
#[derive(Debug, Default)]
struct PassReport {
    stop: Option<PassStop>,
    rounds: u32,
    terminated: Vec<Killed>,
    confirmed: usize,
    terminate_max: Duration,
    candidates: usize,
    births: usize,
    full: bool,
    excluded: Excluded,
    cleanups: usize,
    failed: usize,
    gone: usize,
}

/// 정리 한 판(TRD §3-3 「판의 차례」 · §3-4 「판의 순서」) — 순서 1–3 이 서면 차례를 돈다. 하나라도 서지 않으면
/// 아무것도 끝내지 않는다.
///
/// 한 차례 = 기록 복사 → 후보 · 규칙 → `Cleanup` 을 `seq` 오름차순으로 끝내기 확정 → 그 차례에 끝낸 것들을 마감
/// 하나([`KILL_CONFIRM`]) 안에서 확인. 끝낸 것은 다음 차례에서 끝난 알려진 고리로 보여 그것이 거느리던 사본이 후보가
/// 된다. 새로 끝낼 `Cleanup` 이 없는 차례에서 멈춘다 — 한 판에서 확정까지 간 후보는 다시 고르지 않는다(마감 안에 안
/// 끝났거나 끝내기가 실패한 것이 다음 차례에 산 것으로 보여 거듭 끝내지 않게).
///
/// 늘 판 끝으로 매듭짓고, 로그 · 복사한 기록 · `ticket` 은 두 자물쇠 밖에서 버린다.
// ADR-0262
fn run_pass<G: Group>(
    cell: &Arc<GateCell<G>>,
    cleaner: &Arc<Cleaner<G>>,
    ticket: Ticket,
) -> PassReport {
    let mut report = PassReport::default();
    let killed = match precheck(
        &ticket,
        cleaner.recorder.active(),
        cleaner.retiring.is_set(),
        cleaner.recorder.port_failed(),
    ) {
        Ok(snapshot) => kill_rounds(cell, cleaner, &ticket, snapshot, &mut report),
        Err(why) => {
            report.stop = Some(PassStop::NotReady(why));
            false
        }
    };
    cell.end_pass(cleaner, ticket.gen, killed);
    let waited = cleaner
        .clock
        .now()
        .saturating_duration_since(ticket.first_mono);
    log_pass(cleaner, &report, waited);
    drop(ticket);
    report
}

/// 판의 차례들 — 끝내기 호출이 하나라도 받아들여졌으면 참.
fn kill_rounds<G: Group>(
    cell: &GateCell<G>,
    cleaner: &Cleaner<G>,
    ticket: &Ticket,
    snapshot: &Snapshot,
    report: &mut PassReport,
) -> bool {
    let snapshot_links = snapshot.links();
    let mut attempted: Vec<u64> = Vec::new();
    let mut killed_any = false;
    for round in 1..=PASS_ROUNDS.saturating_add(1) {
        let births = cleaner.recorder.copy(ticket.rec);
        report.births = births.len();
        if round == 1 {
            report.full = cleaner.recorder.full();
        }
        if births.is_empty() && cleaner.recorder.active() != ticket.rec {
            report.stop = Some(PassStop::RecordGone);
            break;
        }
        let picks = pick(
            snapshot,
            &snapshot_links,
            &births,
            &ticket.w,
            &attempted,
            round,
            report,
        );
        if picks.is_empty() {
            break;
        }
        if round > PASS_ROUNDS {
            report.stop = Some(PassStop::RoundsExhausted);
            break;
        }
        report.rounds = round;

        let mut confirming: Vec<&Birth> = Vec::new();
        for (seq, chain) in picks {
            let Some(birth) = births.iter().find(|b| b.seq == seq) else {
                continue;
            };
            attempted.push(seq);
            if let Err(why) = recheck(snapshot, &cleaner.retiring) {
                report.stop = Some(PassStop::NotReady(why));
                break;
            }
            // ★끝내기 확정 — 재확인과 끝내기 호출 하나만 자물쇠 안★(TRD §3-3 「자물쇠 규칙」 ⓑ). 가르기 · 로그 ·
            //   핸들 버리기는 놓은 뒤다. 표식은 여기서 한 번 더 읽는다 — 남는 겹침은 이 읽기 직후의 끝내기 하나다.
            // ADR-0262
            let committed = {
                let mut state = cell.lock();
                let now = cleaner.clock.now();
                match state.commit_check(ticket.gen, now, cleaner.retiring.is_set()) {
                    Commit::Go => {
                        let raw = birth.pin.terminate_raw();
                        let took = cleaner.clock.now().saturating_duration_since(now);
                        if raw.is_ok() {
                            state.note_kill(ticket.gen);
                        }
                        Ok((raw, took))
                    }
                    Commit::Retiring => Err(PassStop::NotReady(NotReady::Retiring)),
                    Commit::Superseded => Err(PassStop::Superseded),
                }
            };
            let (raw, took) = match committed {
                Ok(called) => called,
                Err(stop) => {
                    report.stop = Some(stop);
                    break;
                }
            };
            report.cleanups = report.cleanups.saturating_add(1);
            report.terminate_max = report.terminate_max.max(took);
            killed_any |= raw.is_ok();
            match birth.pin.classify(raw) {
                Ok(MemberKill::Terminated) => {
                    report.terminated.push(Killed {
                        pid: birth.pid,
                        ppid: birth.facts.ppid,
                        chain,
                        round,
                    });
                    confirming.push(birth);
                }
                Ok(MemberKill::Gone) => report.gone = report.gone.saturating_add(1),
                Err(e) => {
                    report.failed = report.failed.saturating_add(1);
                    tracing::warn!(
                        agent = cleaner.tag.agent(),
                        root_pid = cleaner.tag.root_pid,
                        pid = birth.pid,
                        round,
                        "끊기 잔여물 하나를 끝내지 못했다: {e}"
                    );
                }
            }
        }
        if report.stop.is_some() {
            break;
        }
        report.confirmed = report
            .confirmed
            .saturating_add(confirm(&*cleaner.clock, &confirming));
    }
    killed_any
}

/// 한 차례의 고르기 — 이 판에서 아직 확정에 가지 않은 `Cleanup` 을 `seq` 오름차순으로 준다. 첫 차례의 후보 수와
/// 규칙별로 뺀 수를 보고서에 적는다.
fn pick(
    snapshot: &Snapshot,
    snapshot_links: &[Link<'_>],
    births: &[Arc<Birth>],
    w: &[u32],
    attempted: &[u64],
    round: u32,
    report: &mut PassReport,
) -> Vec<(u64, Vec<u32>)> {
    let birth_links: Vec<Link<'_>> = births.iter().map(|b| b.link()).collect();
    let exited = |link: &Link<'_>| link_exited(snapshot, births, link);
    let view = Chain {
        root_pid: snapshot.root.pid,
        snapshot: snapshot_links,
        births: &birth_links,
        exited: &exited,
    };
    let found = candidates(&view, w);
    if round == 1 {
        report.candidates = found.len();
    }
    let mut excluded = Excluded::default();
    let mut picks = Vec::new();
    for c in found {
        let Some(seq) = c.seq else {
            continue;
        };
        match select(c, &view) {
            Verdict::Cleanup { chain } => {
                if !attempted.contains(&seq) {
                    picks.push((seq, chain));
                }
            }
            other => excluded.note(&other),
        }
    }
    report.excluded = excluded;
    picks
}

/// 알려진 고리 하나가 지금 끝났나 — 그 고리를 붙든 핸들로 묻는다(탄생은 `seq` · 스냅숏 멤버는 (번호, 생성)으로
/// 찾는다). 못 찾으면 모름(`Err`) — 고르기가 끝내지 않는 쪽으로 읽는다.
fn link_exited(snapshot: &Snapshot, births: &[Arc<Birth>], link: &Link<'_>) -> io::Result<bool> {
    let pin = match link.seq {
        Some(seq) => births.iter().find(|b| b.seq == seq).map(|b| &b.pin),
        None => std::iter::once(&snapshot.root)
            .chain(&snapshot.others)
            .find(|m| m.pid == link.pid && m.pin.facts().create == link.facts.create)
            .map(|m| &m.pin),
    };
    match pin {
        Some(pin) => pin.exited(),
        None => Err(io::Error::from(io::ErrorKind::NotFound)),
    }
}

/// 판의 순서 5 — 끝내기 앞마다 표식 · 붙든 뿌리를 자물쇠 밖에서 다시 본다.
fn recheck(snapshot: &Snapshot, retiring: &RetiringSignal) -> Result<(), NotReady> {
    if retiring.is_set() {
        return Err(NotReady::Retiring);
    }
    root_alive(snapshot)
}

/// 한 차례에 끝낸 것들의 끝남 확인 — 마감 하나를 나눠 쓴다(앞의 것이 기다린 만큼 뒤의 것은 덜 기다린다). 마감 안에
/// 끝남을 확인한 수를 준다.
fn confirm(clock: &dyn LeftoverClock, killed: &[&Birth]) -> usize {
    if killed.is_empty() {
        return 0;
    }
    let deadline = clock.now().checked_add(KILL_CONFIRM);
    killed
        .iter()
        .filter(|birth| {
            let left =
                deadline.map_or(Duration::ZERO, |d| d.saturating_duration_since(clock.now()));
            matches!(birth.pin.wait_exit(left), Ok(true))
        })
        .count()
}

/// 판의 로그(TRD §3-8) — 자물쇠를 놓은 뒤. 무엇이든 끝냈으면 정리함 warn 하나를 남기고, 멈춘 까닭은 그 사유의 레벨로,
/// 아무것도 못 끝내고 다 돈 판은 못 끝냄 warn 하나로.
fn log_pass<G: Group>(cleaner: &Cleaner<G>, report: &PassReport, waited: Duration) {
    let (agent, root_pid) = (cleaner.tag.agent(), cleaner.tag.root_pid);
    let waited_ms = millis(waited);
    let x = report.excluded;
    let killed = report.terminated.len();
    if killed > 0 {
        tracing::warn!(
            agent,
            root_pid,
            waited_ms,
            terminated = ?report.terminated,
            rounds = report.rounds,
            terminate_max_us = micros(report.terminate_max),
            unconfirmed = killed.saturating_sub(report.confirmed),
            births = report.births,
            parent_alive = x.parent_alive,
            parent_unknown = x.parent_unknown,
            not_hook_copy = x.not_hook_copy,
            ancestor_alive = x.ancestor_alive,
            chain_cut = x.chain_cut,
            not_hook = x.not_hook,
            no_taskkill = x.no_taskkill,
            unknown = x.unknown,
            "끊기 뒤 턴 끝이 없어 우리 Job 안의 훅 잔여물을 끝냈다"
        );
    }
    const NOTHING: &str = "끊기 뒤 턴 끝이 없는데 잔여물을 끝내지 않았다";
    match &report.stop {
        Some(PassStop::NotReady(NotReady::Retiring)) => tracing::debug!(
            agent,
            root_pid,
            reason = "retiring",
            killed,
            "잔여물 정리 — 물러나는 무리라 판을 멈췄다"
        ),
        Some(PassStop::NotReady(NotReady::Snapshot(cause))) => tracing::warn!(
            agent,
            root_pid,
            waited_ms,
            reason = "snapshot_failed",
            cause = cause.label(),
            "{NOTHING} — 스냅숏이 서지 않았다"
        ),
        Some(PassStop::NotReady(NotReady::Released)) => tracing::warn!(
            agent,
            root_pid,
            waited_ms,
            reason = "released",
            "{NOTHING} — 스냅숏을 이미 놓았다"
        ),
        Some(PassStop::NotReady(NotReady::RootGone)) => tracing::debug!(
            agent,
            root_pid,
            reason = "root_gone",
            killed,
            "잔여물 정리 — 뿌리가 끝나 판을 멈췄다"
        ),
        Some(PassStop::NotReady(NotReady::RootUnknown(e))) => tracing::warn!(
            agent,
            root_pid,
            waited_ms,
            reason = "root_unknown",
            killed,
            "뿌리가 끝났는지 몰라 잔여물 정리 판을 멈췄다: {e}"
        ),
        Some(PassStop::Superseded) => tracing::debug!(
            agent,
            root_pid,
            reason = "superseded",
            killed,
            "잔여물 정리 — 판 도중 턴 끝 · 새 끊기 · 새 입력이 와 남은 후보와 다음 차례를 버렸다"
        ),
        Some(PassStop::RecordGone) => tracing::debug!(
            agent,
            root_pid,
            reason = "record_gone",
            killed,
            "잔여물 정리 — 판 도중 탄생 기록이 꺼져 판을 멈췄다"
        ),
        Some(PassStop::RoundsExhausted) => tracing::warn!(
            agent,
            root_pid,
            waited_ms,
            reason = "rounds_exhausted",
            rounds = report.rounds,
            killed,
            "잔여물 정리가 차례 상한에 닿아 남은 후보를 끝내지 않았다"
        ),
        None if killed > 0 => {}
        None if report.candidates == 0 => tracing::warn!(
            agent,
            root_pid,
            waited_ms,
            reason = "no_new_member",
            births = report.births,
            full = report.full,
            "{NOTHING} — 쓰기 명단 뒤에 태어난 산 프로세스가 없다"
        ),
        None if report.cleanups == 0 => tracing::warn!(
            agent,
            root_pid,
            waited_ms,
            reason = "parent_rule",
            births = report.births,
            parent_alive = x.parent_alive,
            parent_unknown = x.parent_unknown,
            not_hook_copy = x.not_hook_copy,
            ancestor_alive = x.ancestor_alive,
            chain_cut = x.chain_cut,
            not_hook = x.not_hook,
            no_taskkill = x.no_taskkill,
            unknown = x.unknown,
            "{NOTHING} — 후보가 모두 규칙에서 빠졌다"
        ),
        None => tracing::warn!(
            agent,
            root_pid,
            waited_ms,
            reason = "all_failed",
            failed = report.failed,
            gone = report.gone,
            "{NOTHING} — 고른 잔여물을 하나도 끝내지 못했다"
        ),
    }
}

/// 판의 순서 1–3(TRD §3-4)이 서지 않은 까닭.
#[derive(Debug)]
enum NotReady {
    /// 물러나는 무리다(debug `retiring`).
    Retiring,
    /// warn `snapshot_failed` — 판 때 그 기록이 꺼져 있었으면(듣는 스레드의 끝 · 실패) `Port`.
    Snapshot(SnapCause),
    /// 판 끝이 이미 스냅숏을 놓았다 — 닿지 않는 갈래.
    Released,
    /// 붙든 뿌리가 끝났다(debug `root_gone`).
    RootGone,
    /// 뿌리 끝남 조회 실패(warn `root_unknown`).
    RootUnknown(io::Error),
}

/// 판의 순서 1–3 — 표식 없음 → 포트가 실패로 굳지 않음(`port_failed`) · 스냅숏이 섰고 그 기록이 아직 켜져 있음
/// (`active` = 지금 켜진 번호) → 붙든 뿌리가 산다. 포트가 굳었으면 새 멤버를 못 들었을 수 있어 「새 멤버 없음」 대신
/// `snapshot_failed(port)` 다. 뿌리 끝남 조회는 OS 호출이라 두 자물쇠 밖에서 부른다.
fn precheck(
    ticket: &Ticket,
    active: u64,
    retiring: bool,
    port_failed: bool,
) -> Result<&Snapshot, NotReady> {
    if retiring {
        return Err(NotReady::Retiring);
    }
    if port_failed {
        return Err(NotReady::Snapshot(SnapCause::Port));
    }
    let snap = match &ticket.snap {
        SnapState::Taken(snap) if ticket.rec != 0 && active == ticket.rec => snap,
        SnapState::Taken(_) => return Err(NotReady::Snapshot(SnapCause::Port)),
        SnapState::Failed(cause) => return Err(NotReady::Snapshot(*cause)),
        SnapState::Released => return Err(NotReady::Released),
    };
    root_alive(snap)?;
    Ok(snap)
}

fn root_alive(snapshot: &Snapshot) -> Result<(), NotReady> {
    match snapshot.root.pin.exited() {
        Ok(false) => Ok(()),
        Ok(true) => Err(NotReady::RootGone),
        Err(e) => Err(NotReady::RootUnknown(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 실물 값의 출처 = `.claude/handoff/attachments/20260930-t40-provenance-spike/procs/*.procs.tsv`(출처 스파이크 ·
    //   claude 2.1.284). 번호 · 실행 파일 · 명령줄은 표 그대로이고, 시각은 그 판의 끊기 줄이 쓰인 때(첫 taskkill − 5 ms ·
    //   p3bg 는 TRD §3-0 표의 기준)에서 잰 ms 로 반올림했다. 명령줄 가운데 규칙과 무관한 꼬리(claude 인자 등)는 줄였다.

    const CMD: &str = r"C:\Windows\System32\cmd.exe";
    const CONHOST: &str = r"C:\Windows\System32\conhost.exe";
    const CLAUDE: &str = r"C:\Users\kimsunzun\AppData\Roaming\npm\node_modules\@anthropic-ai\claude-code\bin\claude.exe";
    const BIN_BASH: &str = r"C:\Program Files\Git\bin\bash.exe";
    const USR_BASH: &str = r"C:\Program Files\Git\usr\bin\bash.exe";
    const SLEEP: &str = r"C:\Program Files\Git\usr\bin\sleep.exe";
    const CAT: &str = r"C:\Program Files\Git\usr\bin\cat.exe";
    const TASKKILL: &str = r"C:\Windows\System32\taskkill.exe";
    const NODE: &str = r"C:\Program Files\nodejs\node.exe";

    const ROOT_CMD: &str =
        r#""cmd.exe" /c claude --permission-mode bypassPermissions -p --input-format stream-json"#;
    const CONHOST_CMD: &str = r"\??\C:\WINDOWS\system32\conhost.exe 0xffffffff -ForceV1";
    const CLAUDE_CMD: &str = r#""C:\Users\kimsunzun\AppData\Roaming\npm\\node_modules\@anthropic-ai\claude-code\bin\claude.exe"    --permission-mode bypassPermissions -p --input-format stream-json"#;

    const WIKI: &str = "wiki-preconsult";
    const HANDOFF: &str = "handoff-trigger";

    /// 끊기 줄이 쓰인 때의 FILETIME — 표의 시각은 이 점에서 ms 로 잰다.
    const T0: i64 = 134_352_256_000_000_000;
    const TICKS_PER_MS: i64 = 10_000;
    /// 판 = 줄이 쓰인 때 + 3 s.
    const PASS_MS: i64 = 3_000;

    /// 훅 실행기 — claude 가 훅마다 띄우는 꼭대기 T 의 모양.
    fn launcher(hook: &str) -> String {
        format!(
            r#""C:\Program Files\Git\bin\bash.exe" -c "bash ${{CLAUDE_PLUGIN_ROOT}}/hooks/{hook}.sh""#
        )
    }

    /// 껍데기 — 실행기가 띄운 `usr\bin` bash. 명령줄은 실행기와 같은 `-c "bash …"` 이다.
    fn shell(hook: &str) -> String {
        format!(
            r#""C:\Program Files\Git\bin\..\usr\bin\bash.exe" -c "bash ${{CLAUDE_PLUGIN_ROOT}}/hooks/{hook}.sh""#
        )
    }

    /// 껍데기가 exec 한 훅 스크립트 — 그 fork 사본도 같은 명령줄을 가진다.
    fn script(hook: &str) -> String {
        format!(
            r#""C:\Program Files\Git\usr\bin\bash.exe" C:/Users/kimsunzun/.claude/plugins/cache/nmfc-origin-plugins/nmfc-origin-dev-harness/2.11.0/hooks/{hook}.sh"#
        )
    }

    fn taskkill(target: u32) -> String {
        format!(r"C:\WINDOWS\System32\taskkill.exe /PID {target} /T /F")
    }

    /// Bash 도구 실행기 · 그 껍데기 · fork 의 명령줄(p1 의 백그라운드 반복 `while true; do sleep 2; done`).
    fn bash_tool(program: &str) -> String {
        format!(
            r#""{program}" -c "source /c/Users/kimsunzun/.claude/shell-snapshots/snapshot-bash-1790752041336-sy2let.sh 2>/dev/null || true && eval 'T40BG=1; while true; do sleep 2; done' < /dev/null && pwd -P >| '/c/Users/KIMSUN~1/AppData/Local/Temp/claude-5f7e-cwd'""#
        )
    }

    fn facts(ppid: u32, create: u64, image: &str, cmdline: &str) -> ProcessFacts {
        ProcessFacts {
            ppid,
            create,
            image: image.to_owned(),
            cmdline: cmdline.to_owned(),
        }
    }

    fn snap(pid: u32, facts: &ProcessFacts) -> Link<'_> {
        Link {
            pid,
            facts,
            seq: None,
            killable: true,
        }
    }

    fn birth(pid: u32, seq: u64, facts: &ProcessFacts) -> Link<'_> {
        Link {
            pid,
            facts,
            seq: Some(seq),
            killable: true,
        }
    }

    struct Proc {
        pid: u32,
        born_ms: i64,
        /// `None` = 판 때 산다.
        exit_ms: Option<i64>,
        /// `false` = 가입 알림을 놓쳤다(기록에 없다).
        recorded: bool,
        facts: ProcessFacts,
    }

    /// 표를 본뜬 가짜 세계 — 줄이 쓰인 때(0 ms)에 살아 있던 것이 스냅숏, 0 < 생성 ≤ 판 인 것이 탄생 기록(생성 순
    /// `seq`)이고, 쓰기 명단 W 는 0 ms 에 산 것의 번호다.
    pub(super) struct World {
        root: u32,
        procs: Vec<Proc>,
        /// 끝남 조회가 `Err` 인 번호.
        failing: Vec<u32>,
    }

    impl World {
        fn new(root: u32) -> Self {
            World {
                root,
                procs: Vec::new(),
                failing: Vec::new(),
            }
        }

        /// 뿌리(`cmd /c`) · 그 콘솔 호스트 · claude — 셋 다 판 때 산다.
        fn hosted(root: u32, conhost: u32, claude: u32) -> Self {
            let mut w = World::new(root);
            w.add(root, 26792, -60_000, None, CMD, ROOT_CMD)
                .add(conhost, root, -60_000, None, CONHOST, CONHOST_CMD)
                .add(claude, root, -59_900, None, CLAUDE, CLAUDE_CMD);
            w
        }

        fn add(
            &mut self,
            pid: u32,
            ppid: u32,
            born_ms: i64,
            exit_ms: Option<i64>,
            image: &str,
            cmdline: impl AsRef<str>,
        ) -> &mut Self {
            let create = (T0 + born_ms * TICKS_PER_MS) as u64;
            self.procs.push(Proc {
                pid,
                born_ms,
                exit_ms,
                recorded: true,
                facts: facts(ppid, create, image, cmdline.as_ref()),
            });
            self
        }

        fn proc_mut(&mut self, pid: u32, born_ms: i64) -> &mut Proc {
            self.procs
                .iter_mut()
                .find(|p| p.pid == pid && p.born_ms == born_ms)
                .expect("세계에 있는 프로세스")
        }

        fn lose(&mut self, pid: u32, born_ms: i64) -> &mut Self {
            self.proc_mut(pid, born_ms).recorded = false;
            self
        }

        fn set_exit(&mut self, pid: u32, born_ms: i64, exit_ms: Option<i64>) -> &mut Self {
            self.proc_mut(pid, born_ms).exit_ms = exit_ms;
            self
        }

        fn alive_at(p: &Proc, ms: i64) -> bool {
            p.born_ms <= ms && p.exit_ms.map_or(true, |e| e > ms)
        }

        fn links(&self) -> (Vec<Link<'_>>, Vec<Link<'_>>) {
            let snapshot = self
                .procs
                .iter()
                .filter(|p| p.recorded && Self::alive_at(p, 0))
                .map(|p| snap(p.pid, &p.facts))
                .collect();
            let mut born: Vec<&Proc> = self
                .procs
                .iter()
                .filter(|p| p.recorded && p.born_ms > 0 && p.born_ms <= PASS_MS)
                .collect();
            born.sort_by_key(|p| p.born_ms);
            let births = born
                .iter()
                .zip(0u64..)
                .map(|(p, seq)| birth(p.pid, seq, &p.facts))
                .collect();
            (snapshot, births)
        }

        fn write_list(&self) -> Vec<u32> {
            let mut w: Vec<u32> = self
                .procs
                .iter()
                .filter(|p| Self::alive_at(p, 0))
                .map(|p| p.pid)
                .collect();
            w.sort_unstable();
            w
        }

        fn exited(&self, link: &Link<'_>, killed: &[u32]) -> io::Result<bool> {
            if self.failing.contains(&link.pid) {
                return Err(io::Error::new(io::ErrorKind::Other, "가짜 조회 실패"));
            }
            let p = self
                .procs
                .iter()
                .find(|p| p.pid == link.pid && p.facts.create == link.facts.create)
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "세계에 없는 고리"))?;
            Ok(killed.contains(&p.pid) || p.exit_ms.is_some_and(|e| e <= PASS_MS))
        }

        /// 탄생 기록의 `pid`(같은 번호가 여럿이면 마지막)를 판 때 가른다 — `killed` 는 앞 차례에 끝낸 번호.
        fn judge(&self, pid: u32, killed: &[u32]) -> Verdict {
            let (snapshot, births) = self.links();
            let view = Chain {
                root_pid: self.root,
                snapshot: &snapshot,
                births: &births,
                exited: &|l: &Link<'_>| self.exited(l, killed),
            };
            let c = births
                .iter()
                .rev()
                .find(|l| l.pid == pid)
                .expect("후보는 탄생 기록에 있다");
            select(c, &view)
        }

        /// 판 한 차례 — 후보를 추려 `Cleanup` 이 난 번호를 차례대로.
        fn round(&self, killed: &[u32]) -> Vec<u32> {
            let (snapshot, births) = self.links();
            let view = Chain {
                root_pid: self.root,
                snapshot: &snapshot,
                births: &births,
                exited: &|l: &Link<'_>| self.exited(l, killed),
            };
            candidates(&view, &self.write_list())
                .into_iter()
                .filter(|c| matches!(select(c, &view), Verdict::Cleanup { .. }))
                .map(|c| c.pid)
                .collect()
        }

        /// 판 시험(`gate_tests`)이 무리 · 기록에 옮겨 싣는 모양 — 스냅숏 · 탄생은 [`Self::links`] 와 같은 것이다.
        pub(super) fn staged(&self) -> Staged {
            let state = |p: &Proc| {
                (
                    p.pid,
                    p.facts.clone(),
                    p.exit_ms.is_some_and(|e| e <= PASS_MS),
                )
            };
            let mut born: Vec<&Proc> = self
                .procs
                .iter()
                .filter(|p| p.recorded && p.born_ms > 0 && p.born_ms <= PASS_MS)
                .collect();
            born.sort_by_key(|p| p.born_ms);
            Staged {
                root: self.root,
                listing: self.write_list(),
                members: self
                    .procs
                    .iter()
                    .filter(|p| p.recorded && Self::alive_at(p, 0))
                    .map(state)
                    .collect(),
                births: born.into_iter().map(state).collect(),
            }
        }
    }

    /// [`World::staged`] — 번호 · 사실 · 판 때 끝났나.
    ///
    /// - `listing` = 줄이 쓰인 때의 명단(= 쓰기 명단 W) — 알림을 놓친 것도 든다.
    /// - `births` = 생성 순(넣는 순서가 곧 `seq`).
    pub(super) struct Staged {
        pub(super) root: u32,
        pub(super) listing: Vec<u32>,
        pub(super) members: Vec<(u32, ProcessFacts, bool)>,
        pub(super) births: Vec<(u32, ProcessFacts, bool)>,
    }

    fn cleanup(chain: &[u32]) -> Verdict {
        Verdict::Cleanup {
            chain: chain.to_vec(),
        }
    }

    // ── 스파이크 모양 ─────────────────────────────────────────────────────────────

    /// 09-30 p1 — 주인 ← 훅 bash D ← 껍데기 ← 실행기 T, D 이상은 스냅숏.
    pub(super) fn p1_chain() -> World {
        let mut w = World::hosted(29284, 36360, 35724);
        w.add(26208, 35724, -182, Some(330), BIN_BASH, launcher(WIKI))
            .add(22700, 26208, -115, Some(330), USR_BASH, shell(WIKI))
            .add(36088, 22700, -56, Some(330), USR_BASH, script(WIKI))
            .add(38108, 36088, 298, None, USR_BASH, script(WIKI));
        w
    }

    pub(super) fn p1() -> World {
        let mut w = p1_chain();
        w.add(24140, 35724, 5, Some(342), TASKKILL, taskkill(33432))
            .add(7164, 35724, 9, Some(334), TASKKILL, taskkill(26208))
            .add(25660, 35724, 1511, Some(1903), TASKKILL, taskkill(33432))
            .add(3548, 35724, 1516, Some(1909), TASKKILL, taskkill(26208));
        w
    }

    /// p1 에 같은 훅 bash 의 사본 주인 38200 을 하나 더 — 한 차례에 `Cleanup` 이 둘이다(잰 모양은 아니다).
    pub(super) fn p1_two_owners() -> World {
        let mut w = p1();
        w.add(38200, 36088, 310, None, USR_BASH, script(WIKI));
        w
    }

    /// 09-30 p3t3 — 주인 ← D fork(탄생) ← 훅 bash ← 껍데기 ← 실행기.
    pub(super) fn p3t3() -> World {
        let mut w = World::hosted(37152, 34960, 40848);
        w.add(33144, 40848, -173, Some(540), BIN_BASH, launcher(WIKI))
            .add(10400, 33144, -85, Some(539), USR_BASH, shell(WIKI))
            .add(27108, 10400, -3, Some(534), USR_BASH, script(WIKI))
            .add(26732, 27108, 282, Some(533), USR_BASH, script(WIKI))
            .add(23604, 26732, 457, None, USR_BASH, script(WIKI))
            .add(26780, 40848, 5, Some(548), TASKKILL, taskkill(38240))
            .add(38612, 40848, 10, Some(549), TASKKILL, taskkill(33144))
            .add(27144, 40848, 1511, Some(1911), TASKKILL, taskkill(38240))
            .add(32532, 40848, 1518, Some(1940), TASKKILL, taskkill(33144));
        w
    }

    /// pilot p1bg — 주인 ← D 훅 bash(탄생) ← 껍데기 ← 실행기.
    pub(super) fn pilot() -> World {
        let mut w = World::hosted(19888, 28960, 5212);
        w.add(25096, 5212, -96, Some(392), BIN_BASH, launcher(WIKI))
            .add(37948, 25096, -36, Some(392), USR_BASH, shell(WIKI))
            .add(37552, 37948, 20, Some(392), USR_BASH, script(WIKI))
            .add(3256, 37552, 356, None, USR_BASH, script(WIKI))
            .add(28116, 5212, 5, Some(378), TASKKILL, taskkill(33556))
            .add(38848, 5212, 9, Some(397), TASKKILL, taskkill(25096))
            .add(30872, 5212, 1510, Some(1801), TASKKILL, taskkill(33556))
            .add(32396, 5212, 1515, Some(1795), TASKKILL, taskkill(25096));
        w
    }

    /// 09-30 p3bg — 실행기 · 껍데기 · 주인이 모두 탄생이고, 주인은 껍데기가 exec 한 스크립트(명령줄이 껍데기와
    /// 다르다)이며 판 때 산 fork 24776 을 거느린다. 같은 판의 끊기 뒤 탄생(taskkill · 그 conhost · Bash 도구 반복의
    /// `sleep` · 다른 훅의 `cat`) 가운데 판 때 산 것을 함께 싣는다.
    pub(super) fn p3bg() -> World {
        let mut w = World::hosted(37152, 34960, 40848);
        w.add(35688, 40848, 74, Some(1722), BIN_BASH, launcher(HANDOFF))
            .add(12180, 40848, 125, Some(1761), TASKKILL, taskkill(35688))
            .add(
                31756,
                20868,
                860,
                Some(1375),
                USR_BASH,
                bash_tool(r"C:\Program Files\Git\bin\..\usr\bin\bash.exe"),
            )
            .add(25028, 35688, 871, Some(1695), USR_BASH, shell(HANDOFF))
            .add(
                9032,
                31756,
                1078,
                None,
                SLEEP,
                r#""C:\Program Files\Git\usr\bin\sleep.exe" 2"#,
            )
            .add(32228, 25028, 1432, None, USR_BASH, script(HANDOFF))
            .add(25768, 32228, 1659, Some(1731), USR_BASH, script(HANDOFF))
            .add(29864, 40848, 1665, None, TASKKILL, taskkill(27196))
            .add(32648, 40848, 1671, None, TASKKILL, taskkill(35688))
            .add(39424, 32648, 1708, None, CONHOST, CONHOST_CMD)
            .add(1672, 28436, 1738, Some(2150), USR_BASH, script(WIKI))
            .add(
                30144,
                1672,
                1808,
                None,
                CAT,
                r#""C:\Program Files\Git\usr\bin\cat.exe""#,
            )
            .add(38576, 29864, 2073, None, CONHOST, CONHOST_CMD)
            .add(24776, 32228, 2755, None, USR_BASH, script(HANDOFF));
        w
    }

    /// p3bg 의 fork 24776 아래로 fork 가 둘 더 이어진다(24776 → 24800 → 24810) — 차례마다 하나씩 후보가 되어 네 차례가
    /// 필요하다(잰 모양은 아니다).
    pub(super) fn p3bg_deeper() -> World {
        let mut w = p3bg();
        w.add(24800, 24776, 2800, None, USR_BASH, script(HANDOFF))
            .add(24810, 24800, 2850, None, USR_BASH, script(HANDOFF));
        w
    }

    #[test]
    fn p1_owner_is_cleaned_with_its_chain_and_nothing_else() {
        let w = p1();
        assert_eq!(w.judge(38108, &[]), cleanup(&[38108, 36088, 22700, 26208]));
        assert_eq!(w.round(&[]), vec![38108]);
        assert!(w.round(&[38108]).is_empty());
    }

    #[test]
    fn p3t3_owner_under_a_recorded_fork_is_cleaned() {
        let w = p3t3();
        assert_eq!(
            w.judge(23604, &[]),
            cleanup(&[23604, 26732, 27108, 10400, 33144])
        );
        assert_eq!(w.round(&[]), vec![23604]);
        assert!(w.round(&[23604]).is_empty());
    }

    #[test]
    fn pilot_owner_is_cleaned() {
        let w = pilot();
        assert_eq!(w.judge(3256, &[]), cleanup(&[3256, 37552, 37948, 25096]));
        assert_eq!(w.round(&[]), vec![3256]);
        assert!(w.round(&[3256]).is_empty());
    }

    #[test]
    fn p3bg_owner_first_then_its_fork_once_the_owner_is_gone() {
        let w = p3bg();
        assert_eq!(w.judge(32228, &[]), cleanup(&[32228, 25028, 35688]));
        assert_eq!(w.judge(24776, &[]), Verdict::ParentAlive);
        assert_eq!(w.round(&[]), vec![32228]);

        assert_eq!(
            w.judge(24776, &[32228]),
            cleanup(&[24776, 32228, 25028, 35688])
        );
        assert_eq!(w.round(&[32228]), vec![24776]);
        assert!(w.round(&[32228, 24776]).is_empty());
    }

    #[test]
    fn p3bg_other_births_alive_at_the_pass_are_left_alone() {
        let w = p3bg();
        assert_eq!(w.judge(32648, &[]), Verdict::ParentAlive, "taskkill");
        assert_eq!(
            w.judge(29864, &[]),
            Verdict::ParentAlive,
            "다른 훅의 taskkill"
        );
        assert_eq!(
            w.judge(39424, &[]),
            Verdict::ParentAlive,
            "taskkill 의 conhost"
        );
        assert_eq!(w.judge(9032, &[]), Verdict::NotHookCopy, "반복의 sleep");
        assert_eq!(w.judge(30144, &[]), Verdict::NotHookCopy, "훅이 띄운 cat");
    }

    // ── taskkill 표지 ─────────────────────────────────────────────────────────────

    #[test]
    fn no_taskkill_birth_is_no_taskkill() {
        assert_eq!(p1_chain().judge(38108, &[]), Verdict::NoTaskkill);
    }

    #[test]
    fn taskkill_naming_another_pid_is_no_taskkill() {
        let mut w = p1_chain();
        w.add(24140, 35724, 5, Some(342), TASKKILL, taskkill(33432))
            .add(25660, 35724, 1511, Some(1903), TASKKILL, taskkill(33432));
        assert_eq!(w.judge(38108, &[]), Verdict::NoTaskkill);
    }

    #[test]
    fn taskkill_not_spawned_by_claude_is_no_taskkill() {
        let mut w = p1_chain();
        w.add(7164, 36360, 9, Some(334), TASKKILL, taskkill(26208));
        assert_eq!(w.judge(38108, &[]), Verdict::NoTaskkill);
    }

    #[test]
    fn taskkill_older_than_the_top_is_no_taskkill() {
        // 번호 재사용 흉내 — 옛 꼭대기를 가리키던 taskkill 이 같은 번호의 새 T 보다 먼저 태어났다.
        let mut w = p3bg();
        w.proc_mut(12180, 125).facts.create = (T0 + 60 * TICKS_PER_MS) as u64;
        w.proc_mut(32648, 1671).facts.create = (T0 + 70 * TICKS_PER_MS) as u64;
        assert_eq!(w.judge(32228, &[]), Verdict::NoTaskkill);
    }

    // ── 사슬 ──────────────────────────────────────────────────────────────────────

    #[test]
    fn reused_number_younger_than_the_child_is_not_its_parent() {
        // D fork 26732 의 알림을 놓쳤고 그 번호를 주인보다 늦게 태어난 프로세스가 재사용했다.
        let mut w = p3t3();
        w.lose(26732, 282)
            .add(26732, 27108, 600, Some(900), USR_BASH, script(WIKI));
        assert_eq!(w.judge(23604, &[]), Verdict::ParentUnknown);

        // 가운데 고리(껍데기 25028)의 번호가 재사용된 경우 — 2 차례의 fork 에서 사슬이 끊긴다.
        let mut w = p3bg();
        w.lose(25028, 871)
            .add(25028, 40848, 2000, Some(2500), USR_BASH, shell(HANDOFF));
        assert_eq!(w.judge(32228, &[]), Verdict::ParentUnknown);
        assert_eq!(w.judge(24776, &[32228]), Verdict::ChainCut);
    }

    #[test]
    fn lost_middle_link_cuts_the_chain() {
        let mut w = p3bg();
        w.lose(25028, 871);
        assert_eq!(w.judge(24776, &[32228]), Verdict::ChainCut);
        assert!(w.round(&[32228]).is_empty());
    }

    /// p1 의 Bash 도구 백그라운드 반복 — 실행기 37280 · 반복하는 껍데기 36052(둘 다 스냅숏 · 산다) · 반복마다의 fork
    /// 35732 · 그 `sleep` 9408. 35740 은 같은 fork 의 서브셸(사본)을 지어 넣은 것이다.
    fn bash_tool_loop() -> World {
        let usr = r"C:\Program Files\Git\bin\..\usr\bin\bash.exe";
        let mut w = World::hosted(29284, 36360, 35724);
        w.add(37280, 35724, -10_216, None, BIN_BASH, bash_tool(BIN_BASH))
            .add(36052, 37280, -10_188, None, USR_BASH, bash_tool(usr))
            .add(35732, 36052, 2318, Some(2372), USR_BASH, bash_tool(usr))
            .add(35740, 35732, 2330, None, USR_BASH, bash_tool(usr))
            .add(
                9408,
                35732,
                2350,
                None,
                SLEEP,
                r#""C:\Program Files\Git\usr\bin\sleep.exe" 2"#,
            );
        w
    }

    #[test]
    fn loop_sleep_is_not_a_hook_copy() {
        assert_eq!(bash_tool_loop().judge(9408, &[]), Verdict::NotHookCopy);
    }

    #[test]
    fn subshell_under_a_live_loop_is_ancestor_alive() {
        assert_eq!(
            bash_tool_loop().judge(35740, &[]),
            Verdict::AncestorAlive { pid: 36052 }
        );
    }

    #[test]
    fn subshell_after_the_whole_bash_tool_chain_died_is_not_hook() {
        let mut w = bash_tool_loop();
        w.set_exit(36052, -10_188, Some(2400))
            .set_exit(37280, -10_216, Some(2450));
        assert_eq!(w.judge(35740, &[]), Verdict::NotHook);
        assert!(w.round(&[]).is_empty());
    }

    #[test]
    fn cycle_of_equal_creates_cuts_the_chain() {
        let mut w = World::hosted(1, 2, 3);
        w.add(901, 902, 100, Some(1000), USR_BASH, script(WIKI))
            .add(902, 901, 100, Some(1000), USR_BASH, script(WIKI))
            .add(900, 901, 200, None, USR_BASH, script(WIKI));
        assert_eq!(w.judge(900, &[]), Verdict::ChainCut);
    }

    /// claude ← 실행기 100 ← 101 ← … ← `100 + n − 1`(D) ← 후보 200 — 조상 `n` 개 · 모두 끝남 · taskkill 있음.
    fn tower(n: u32) -> World {
        let mut w = World::hosted(1, 2, 3);
        w.add(100, 3, 10, Some(1000), BIN_BASH, launcher(WIKI));
        for i in 1..n {
            w.add(
                100 + i,
                100 + i - 1,
                10 + i64::from(i),
                Some(1000),
                USR_BASH,
                script(WIKI),
            );
        }
        w.add(200, 100 + n - 1, 500, None, USR_BASH, script(WIKI))
            .add(300, 3, 40, Some(400), TASKKILL, taskkill(100));
        w
    }

    #[test]
    fn ancestor_walk_stops_past_the_cap() {
        let at_cap = tower(16).judge(200, &[]);
        let expected: Vec<u32> = std::iter::once(200).chain((100..116).rev()).collect();
        assert_eq!(at_cap, Verdict::Cleanup { chain: expected });
        assert_eq!(tower(17).judge(200, &[]), Verdict::ChainCut);
    }

    #[test]
    fn parent_in_claudes_seat_is_k_equals_one() {
        // claude 가 끝났다고 치고 그 사본을 후보로 — D 가 곧 뿌리의 자식이라 꼭대기가 없다.
        let mut w = World::hosted(1, 2, 3);
        w.set_exit(3, -59_900, Some(1000))
            .add(200, 3, 500, None, CLAUDE, CLAUDE_CMD);
        assert_eq!(w.judge(200, &[]), Verdict::ChainCut);
    }

    #[test]
    fn reaching_the_root_first_cuts_the_chain() {
        let mut w = World::hosted(1, 2, 3);
        w.set_exit(1, -60_000, Some(1000))
            .add(200, 1, 500, None, CMD, ROOT_CMD);
        assert_eq!(w.judge(200, &[]), Verdict::ChainCut);
    }

    /// 뿌리와 claude 사이에 런처가 낀 모양 — 뿌리 1 ← 런처 4 ← claude 3 ← 훅 사슬.
    fn behind_a_launcher() -> World {
        let mut w = World::new(1);
        w.add(1, 26792, -60_000, None, CMD, ROOT_CMD)
            .add(
                4,
                1,
                -60_000,
                None,
                NODE,
                r#""C:\Program Files\nodejs\node.exe" claude.js"#,
            )
            .add(3, 4, -59_900, None, CLAUDE, CLAUDE_CMD)
            .add(100, 3, 10, Some(1000), BIN_BASH, launcher(WIKI))
            .add(101, 100, 20, Some(1000), USR_BASH, shell(WIKI))
            .add(102, 101, 30, Some(1000), USR_BASH, script(WIKI))
            .add(200, 102, 500, None, USR_BASH, script(WIKI))
            .add(300, 3, 15, Some(400), TASKKILL, taskkill(100));
        w
    }

    #[test]
    fn launcher_between_root_and_claude_is_missed() {
        // 걷기가 claude 를 조상으로 만난다 — 살아 있으면 거기서, 끝났다면 꼭대기가 claude.exe 라 놓친다.
        assert_eq!(
            behind_a_launcher().judge(200, &[]),
            Verdict::AncestorAlive { pid: 3 }
        );
        let mut w = behind_a_launcher();
        w.set_exit(3, -59_900, Some(2000));
        assert_eq!(w.judge(200, &[]), Verdict::NotHook);
    }

    // ── 모름 ──────────────────────────────────────────────────────────────────────

    #[test]
    fn liveness_errors_and_empty_fields_are_unknown() {
        let mut w = p1();
        w.failing.push(36088);
        assert_eq!(w.judge(38108, &[]), Verdict::Unknown, "D 의 끝남 조회 실패");

        let mut w = p1();
        w.failing.push(22700);
        assert_eq!(
            w.judge(38108, &[]),
            Verdict::Unknown,
            "조상의 끝남 조회 실패"
        );

        let mut w = p1();
        w.proc_mut(38108, 298).facts.cmdline.clear();
        assert_eq!(w.judge(38108, &[]), Verdict::Unknown, "후보 명령줄 빈 칸");

        let mut w = p1();
        w.proc_mut(36088, -56).facts.image.clear();
        assert_eq!(w.judge(38108, &[]), Verdict::Unknown, "D 실행 파일 빈 칸");

        let mut w = p1();
        w.root = 0;
        assert_eq!(w.judge(38108, &[]), Verdict::Unknown, "뿌리 번호 모름");
    }

    #[test]
    fn unreadable_parent_number_or_create_is_parent_unknown() {
        let mut w = p1();
        w.proc_mut(38108, 298).facts.ppid = 0;
        assert_eq!(w.judge(38108, &[]), Verdict::ParentUnknown);

        let mut w = p1();
        w.proc_mut(36088, -56).facts.create = 0;
        assert_eq!(w.judge(38108, &[]), Verdict::ParentUnknown);

        let mut w = p1();
        w.proc_mut(38108, 298).facts.create = 0;
        assert_eq!(w.judge(38108, &[]), Verdict::ParentUnknown);
    }

    // ── 부모 찾기 ─────────────────────────────────────────────────────────────────

    fn never(_: &Link<'_>) -> io::Result<bool> {
        Err(io::Error::new(io::ErrorKind::Other, "부르지 않는다"))
    }

    #[test]
    fn parent_is_the_latest_holder_born_no_later_than_the_child() {
        let old = facts(1, 100, USR_BASH, "a");
        let new = facts(1, 300, USR_BASH, "b");
        let zero = facts(1, 0, USR_BASH, "c");
        let snapshot = [snap(10, &old)];
        let births = [birth(10, 0, &new), birth(10, 1, &zero)];
        let view = Chain {
            root_pid: 1,
            snapshot: &snapshot,
            births: &births,
            exited: &never,
        };
        let parent = |create: u64| {
            let child = facts(10, create, USR_BASH, "x");
            parent_of(&view, 20, &child).map(|l| l.facts.create)
        };
        assert_eq!(parent(350), Some(300));
        assert_eq!(parent(300), Some(300), "같은 생성은 부모가 될 수 있다");
        assert_eq!(parent(200), Some(100));
        assert_eq!(parent(50), None);

        let child = facts(10, 350, USR_BASH, "x");
        assert!(parent_of(&view, 10, &child).is_none(), "자기 번호");
    }

    #[test]
    fn equal_holder_in_snapshot_and_births_resolves_to_the_snapshot() {
        let f = facts(1, 100, USR_BASH, "a");
        let snapshot = [snap(10, &f)];
        let births = [birth(10, 0, &f)];
        let view = Chain {
            root_pid: 1,
            snapshot: &snapshot,
            births: &births,
            exited: &never,
        };
        let child = facts(10, 200, USR_BASH, "x");
        assert_eq!(parent_of(&view, 20, &child).map(|l| l.seq), Some(None));
    }

    // ── 후보 ──────────────────────────────────────────────────────────────────────

    #[test]
    fn candidates_are_new_killable_live_births_in_seq_order() {
        let member = facts(1, 100, USR_BASH, "m");
        let late = facts(1, 500, USR_BASH, "late");
        let early = facts(1, 400, USR_BASH, "early");
        let chain_only = facts(1, 450, USR_BASH, "chain-only");
        let in_w = facts(1, 300, USR_BASH, "in-w");
        let reused_member_number = facts(1, 600, USR_BASH, "reused");
        let dead = facts(1, 700, USR_BASH, "dead");
        let unreadable = facts(1, 800, USR_BASH, "unreadable");

        let snapshot = [snap(50, &member)];
        let births = [
            birth(60, 3, &late),
            birth(61, 1, &early),
            Link {
                killable: false,
                ..birth(62, 2, &chain_only)
            },
            birth(63, 0, &in_w),
            birth(50, 4, &member),
            birth(64, 5, &dead),
            birth(65, 6, &unreadable),
            Link {
                seq: None,
                ..birth(66, 7, &early)
            },
            birth(50, 8, &reused_member_number),
        ];
        let exited = |l: &Link<'_>| match l.pid {
            64 => Ok(true),
            65 => Err(io::Error::new(io::ErrorKind::Other, "조회 실패")),
            _ => Ok(false),
        };
        let view = Chain {
            root_pid: 1,
            snapshot: &snapshot,
            births: &births,
            exited: &exited,
        };
        let picked: Vec<(u32, Option<u64>)> = candidates(&view, &[63, 70])
            .iter()
            .map(|l| (l.pid, l.seq))
            .collect();
        assert_eq!(picked, vec![(61, Some(1)), (60, Some(3)), (50, Some(8))]);
    }

    // ── 표지 ──────────────────────────────────────────────────────────────────────

    fn cmd(cmdline: &str) -> ProcessFacts {
        facts(3, 1, BIN_BASH, cmdline)
    }

    #[test]
    fn hook_command_accepts_the_launcher_and_shell_shapes() {
        assert!(is_hook_command(&cmd(&launcher(WIKI))));
        assert!(is_hook_command(&cmd(&shell(HANDOFF))));
        assert!(is_hook_command(&cmd(r#"bash -c "bash x.sh""#)));
        assert!(is_hook_command(&cmd(&format!("{}  \t", launcher(WIKI)))));
    }

    #[test]
    fn hook_command_rejects_other_shapes() {
        let rejected = [
            bash_tool(BIN_BASH),
            r#""C:\Program Files\Git\bin\bash.exe" -c "bash /c/Users/x/.claude/shell-snapshots/a.sh""#.to_owned(),
            r#""C:\Program Files\Git\bin\bash.exe" -c "node \"${CLAUDE_PLUGIN_ROOT}/scripts/stop-review-gate-hook.mjs\"""#.to_owned(),
            r#""C:\Program Files\Git\bin\bash.exe" -c "C:/Users/kimsunzun/.orca/agent-hooks/claude-hook.cmd || echo {}""#.to_owned(),
            r#""C:\Program Files\Git\bin\..\usr\bin\bash.exe" -lc "echo \"$PATH\"""#.to_owned(),
            r#""C:\Program Files\Git\bin\bash.exe" -lc "bash x.sh""#.to_owned(),
            r#""C:\Program Files\Git\bin\..\usr\bin\bash.exe" -c -l "SNAPSHOT_FILE='C:\Users\kimsunzun\.claude\shell-snapshots\snapshot-bash-1-a.sh'""#.to_owned(),
            script(WIKI),
            r#""C:\Program Files\Git\bin\bash.exe" -c bash x.sh"#.to_owned(),
            r#""C:\Program Files\Git\bin\bash.exe" -c "bash""#.to_owned(),
            r#""C:\Program Files\Git\bin\bash.exe" -cx "bash x.sh""#.to_owned(),
            "\"C:\\Program Files\\Git\\bin\\bash.exe\" -c \"bash x.sh\nrm -rf y\"".to_owned(),
            r#" bash -c "bash x.sh""#.to_owned(),
            r#""C:\Program Files\Git\bin\bash.exe""#.to_owned(),
            String::new(),
        ];
        for cmdline in rejected {
            assert!(!is_hook_command(&cmd(&cmdline)), "{cmdline}");
        }
    }

    #[test]
    fn taskkill_names_reads_only_the_exact_claude_shape() {
        let tk = |image: &str, ppid: u32, cmdline: &str| {
            taskkill_names(&facts(ppid, 1, image, cmdline), 40848)
        };
        assert_eq!(
            tk(TASKKILL, 40848, "taskkill.exe /PID 35688 /T /F"),
            Some(35688)
        );
        assert_eq!(tk(TASKKILL, 40848, &taskkill(35688)), Some(35688));
        assert_eq!(
            tk(
                TASKKILL,
                40848,
                r#""C:\WINDOWS\System32\taskkill.exe" /PID 35688 /T /F "#
            ),
            Some(35688)
        );
        assert_eq!(
            tk(r"C:\WINDOWS\SYSTEM32\TASKKILL.EXE", 40848, &taskkill(35688)),
            Some(35688)
        );

        let rejected = [
            "taskkill.exe /PID 35688 /F",
            "taskkill.exe /PID 35688 /T",
            "taskkill.exe /T /F /PID 35688",
            "taskkill.exe /pid 35688 /t /f",
            "taskkill.exe /PID:35688 /T /F",
            "taskkill.exe /PID 35688 /T /F /IM bash.exe",
            "taskkill.exe /PID abc /T /F",
            "taskkill.exe /PID +35688 /T /F",
            "taskkill.exe /PID 035688 /T /F",
            "taskkill.exe /PID 0 /T /F",
            "taskkill.exe /PID 99999999999 /T /F",
            "/PID 35688 /T /F",
            "",
        ];
        for cmdline in rejected {
            assert_eq!(tk(TASKKILL, 40848, cmdline), None, "{cmdline}");
        }
        assert_eq!(tk(CMD, 40848, &taskkill(35688)), None, "다른 실행 파일");
        assert_eq!(tk("", 40848, &taskkill(35688)), None, "실행 파일 빈 칸");
        assert_eq!(
            tk(TASKKILL, 40849, &taskkill(35688)),
            None,
            "부모가 claude 가 아님"
        );
    }
}

#[cfg(test)]
mod birth_tests {
    use super::*;

    use super::test_support::{Capture, FakePin, Line, Probe};

    use std::collections::HashMap;
    use std::sync::mpsc::Sender;
    use std::sync::{PoisonError, Weak};
    use std::thread::{self, JoinHandle};

    use tracing::Level;

    /// 시험이 듣는 스레드를 기다리는 한도 — 넘으면 매달린 것이다.
    const PATIENCE: Duration = Duration::from_secs(10);
    const ROOT: u32 = 7;

    // ── 가짜 ──────────────────────────────────────────────────────────────────────────

    fn facts(ppid: u32, create: u64) -> ProcessFacts {
        ProcessFacts {
            ppid,
            create,
            image: r"C:\Program Files\Git\usr\bin\bash.exe".to_owned(),
            cmdline: "bash x.sh".to_owned(),
        }
    }

    fn caught(pid: u32, create: u64, probe: &Arc<Probe>) -> Caught {
        Caught {
            pid,
            facts: facts(1, create),
            pin: Box::new(FakePin::new(facts(1, create), probe)),
            killable: true,
        }
    }

    fn recorded(recorder: &Recorder, r: u64, c: Caught) -> u64 {
        match recorder.admit(r, c) {
            Admit::Recorded { seq } => seq,
            _ => panic!("넣어야 할 탄생이 안 들어갔다"),
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Call {
        Port,
        Spawn,
        Attach,
        Unwatch,
        Pin(u32, bool),
        PortDrop,
    }

    type Calls = Arc<Mutex<Vec<Call>>>;

    fn push(calls: &Calls, call: Call) {
        calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }

    enum Feed {
        Event(PortEvent),
        Fail,
    }

    /// 시험이 알림을 하나씩 먹이는 포트 — 꺼내기에 들어올 때마다 그때 Job 의 강한 수를 시험에 알린다.
    struct FakePort {
        feed: Mutex<mpsc::Receiver<Feed>>,
        waiting: Sender<usize>,
        job: Weak<()>,
        calls: Calls,
    }

    impl Births for FakePort {
        fn next(&self, _: Duration) -> io::Result<PortEvent> {
            let _ = self.waiting.send(self.job.strong_count());
            let fed = self
                .feed
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .recv_timeout(PATIENCE);
            match fed {
                Ok(Feed::Event(event)) => Ok(event),
                Ok(Feed::Fail) => Err(io::Error::other("꺼내기 실패 흉내")),
                // 먹일 이가 없다 — 실물처럼 시간 초과로 돈다(헛돌지 않게 조금 쉰다).
                Err(_) => {
                    thread::sleep(Duration::from_millis(10));
                    Ok(PortEvent::Timeout)
                }
            }
        }
    }

    impl Drop for FakePort {
        fn drop(&mut self) {
            push(&self.calls, Call::PortDrop);
        }
    }

    #[derive(Clone)]
    enum Answer {
        Ours(ProcessFacts),
        Denied,
        Panic,
    }

    /// 실물 손잡이처럼 Job 을 약하게 쥐고 부르는 동안만 올린다. 부름 순서를 적는다.
    struct FakeGroup {
        job: Weak<()>,
        retiring: Arc<AtomicBool>,
        port: Mutex<Option<Arc<FakePort>>>,
        answers: Mutex<HashMap<(u32, bool), Answer>>,
        refuse_watch: Mutex<Option<io::ErrorKind>>,
        attach_fails: AtomicBool,
        unwatch_fails: AtomicBool,
        probe: Arc<Probe>,
        calls: Calls,
    }

    impl Group for FakeGroup {
        fn member_pids(&self) -> io::Result<Vec<u32>> {
            Ok(Vec::new())
        }

        fn pin(&self, pid: u32, kill: bool) -> io::Result<Option<Box<dyn Pinned>>> {
            let Some(_job) = self.job.upgrade() else {
                return Ok(None);
            };
            if self.probe.held() {
                self.probe.held_calls.fetch_add(1, Ordering::SeqCst);
            }
            push(&self.calls, Call::Pin(pid, kill));
            let answer = self.answers.lock().unwrap().get(&(pid, kill)).cloned();
            match answer {
                None => Ok(None),
                Some(Answer::Ours(facts)) => Ok(Some(Box::new(FakePin::new(facts, &self.probe)))),
                Some(Answer::Denied) => Err(io::Error::from(io::ErrorKind::PermissionDenied)),
                Some(Answer::Panic) => panic!("붙들기 패닉 흉내"),
            }
        }

        fn watch_births(
            &self,
            start: impl FnOnce(Arc<dyn Births>) -> io::Result<()>,
        ) -> io::Result<()> {
            let refused = *self.refuse_watch.lock().unwrap();
            if let Some(kind) = refused {
                return Err(io::Error::new(kind, "무리 없음 흉내"));
            }
            let port = self
                .port
                .lock()
                .unwrap()
                .take()
                .expect("포트는 한 번만 만든다");
            push(&self.calls, Call::Port);
            let shared: Arc<dyn Births> = Arc::clone(&port) as Arc<dyn Births>;
            start(shared)?;
            push(&self.calls, Call::Attach);
            let attached = if self.attach_fails.load(Ordering::SeqCst) {
                Err(io::Error::other("붙이기 실패 흉내"))
            } else {
                Ok(())
            };
            drop(port);
            attached
        }

        fn unwatch_births(&self) -> io::Result<()> {
            push(&self.calls, Call::Unwatch);
            if self.job.strong_count() == 0 {
                Err(io::Error::new(GROUP_GONE, "무리 없음 흉내"))
            } else if self.unwatch_fails.load(Ordering::SeqCst) {
                Err(io::Error::other("떼기 실패 흉내"))
            } else {
                Ok(())
            }
        }

        fn is_gone(&self) -> bool {
            self.job.strong_count() == 0
        }

        fn retiring(&self) -> RetiringSignal {
            RetiringSignal::of(&self.retiring)
        }
    }

    // ── 판 ────────────────────────────────────────────────────────────────────────────

    /// 가짜 무리 · 포트 · 기록 · 포트 확보 한 벌. 듣는 스레드는 시험이 먹이는 알림 하나마다 한 차례를 돌고, 시험은 그
    /// 스레드가 다음 꺼내기에 들어온 것을 보고 상태를 읽는다 — 둘이 겹쳐 돌지 않는다.
    struct Rig {
        job: Option<Arc<()>>,
        group: Arc<FakeGroup>,
        recorder: Arc<Recorder>,
        probe: Arc<Probe>,
        port: Weak<FakePort>,
        watch: BirthWatch,
        feed: Sender<Feed>,
        waits: mpsc::Receiver<usize>,
        thread: Arc<Mutex<Option<JoinHandle<()>>>>,
        done_tx: Sender<()>,
        done: mpsc::Receiver<()>,
        lines: Arc<Mutex<Vec<Line>>>,
    }

    impl Rig {
        fn new() -> Self {
            let job = Arc::new(());
            let recorder = Arc::new(Recorder::new());
            let probe = Probe::new(&recorder);
            let calls = Calls::default();
            let (feed, fed) = mpsc::channel();
            let (waiting, waits) = mpsc::channel();
            let port = Arc::new(FakePort {
                feed: Mutex::new(fed),
                waiting,
                job: Arc::downgrade(&job),
                calls: Arc::clone(&calls),
            });
            let group = Arc::new(FakeGroup {
                job: Arc::downgrade(&job),
                retiring: Arc::default(),
                port: Mutex::new(Some(Arc::clone(&port))),
                answers: Mutex::default(),
                refuse_watch: Mutex::default(),
                attach_fails: AtomicBool::new(false),
                unwatch_fails: AtomicBool::new(false),
                probe: Arc::clone(&probe),
                calls,
            });
            let (done_tx, done) = mpsc::channel();
            Self {
                job: Some(job),
                group,
                recorder,
                probe,
                port: Arc::downgrade(&port),
                watch: BirthWatch::new(),
                feed,
                waits,
                thread: Arc::default(),
                done_tx,
                done,
                lines: Arc::default(),
            }
        }

        /// 듣는 스레드를 시험의 스레드로 띄운다 — 그 스레드의 로그를 모으고, 몸통이 끝나면 알린다.
        fn spawner(&self) -> impl FnOnce(Box<dyn FnOnce() + Send>) -> io::Result<()> {
            let calls = Arc::clone(&self.group.calls);
            let lines = Arc::clone(&self.lines);
            let slot = Arc::clone(&self.thread);
            let done = self.done_tx.clone();
            move |body| {
                push(&calls, Call::Spawn);
                let handle = thread::Builder::new().spawn(move || {
                    tracing::subscriber::with_default(Capture(lines), body);
                    let _ = done.send(());
                })?;
                *slot.lock().unwrap() = Some(handle);
                Ok(())
            }
        }

        fn failing_spawner(&self) -> impl FnOnce(Box<dyn FnOnce() + Send>) -> io::Result<()> {
            let calls = Arc::clone(&self.group.calls);
            move |body| {
                push(&calls, Call::Spawn);
                drop(body);
                Err(io::Error::other("기동 실패 흉내"))
            }
        }

        /// 포트 확보 — 이 스레드의 로그도 모은다.
        fn ensure(
            &self,
            spawn: impl FnOnce(Box<dyn FnOnce() + Send>) -> io::Result<()>,
        ) -> Result<(), ()> {
            tracing::subscriber::with_default(Capture(Arc::clone(&self.lines)), || {
                self.watch
                    .ensure_with(&self.group, &self.recorder, &LogTag::new(None, ROOT), spawn)
            })
        }

        /// 확보하고 듣는 스레드가 첫 꺼내기에 들어올 때까지.
        fn listening(&self) {
            assert_eq!(self.ensure(self.spawner()), Ok(()));
            self.wait_next();
        }

        /// 듣는 스레드가 꺼내기에 들어올 때까지 — 앞 알림의 처리는 다 끝났다. 그때 Job 의 강한 수를 준다.
        fn wait_next(&self) -> usize {
            self.waits
                .recv_timeout(PATIENCE)
                .expect("듣는 스레드가 꺼내기로 돌아오지 않았다")
        }

        /// 알림 하나를 먹이고 그 처리가 끝나기를 기다린다.
        fn feed(&self, event: PortEvent) -> usize {
            self.feed.send(Feed::Event(event)).expect("먹이기");
            self.wait_next()
        }

        /// 꺼내기 오류 하나를 먹이고, 스레드가 끝나지 않고 다음 꺼내기에 들어오기를 기다린다.
        fn fail_take(&self) {
            self.feed.send(Feed::Fail).expect("먹이기");
            self.wait_next();
        }

        /// 먹이고 듣는 스레드가 끝나기를 기다린다.
        fn feed_last(&self, feed: Feed) {
            self.feed.send(feed).expect("먹이기");
            self.ended();
        }

        fn ended(&self) {
            self.done
                .recv_timeout(PATIENCE)
                .expect("듣는 스레드가 끝나지 않았다");
            let handle = self.thread.lock().unwrap().take();
            if let Some(handle) = handle {
                handle.join().expect("듣는 스레드가 패닉으로 죽었다");
            }
        }

        fn answer(&self, pid: u32, kill: bool, answer: Answer) {
            self.group
                .answers
                .lock()
                .unwrap()
                .insert((pid, kill), answer);
        }

        fn calls(&self) -> Vec<Call> {
            self.group.calls.lock().unwrap().clone()
        }

        fn pins(&self) -> Vec<Call> {
            self.calls()
                .into_iter()
                .filter(|call| matches!(call, Call::Pin(..)))
                .collect()
        }

        /// 통로가 Job 을 놓는다 — 무리가 사라진다.
        fn vanish(&mut self) {
            self.job = None;
        }

        fn retire(&self) {
            self.group.retiring.store(true, Ordering::Release);
        }

        fn count(&self, level: Level) -> usize {
            self.lines
                .lock()
                .unwrap()
                .iter()
                .filter(|(l, _)| *l == level)
                .count()
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            // 아직 도는 듣는 스레드가 있으면 끝나게 한다 — 무리를 없애고 한 차례를 돌린다.
            self.job = None;
            let _ = self.feed.send(Feed::Event(PortEvent::Timeout));
        }
    }

    // ── 탄생 기록 ─────────────────────────────────────────────────────────────────────

    /// (번호, 생성)이 새로운 것만 한 번씩 `seq` 오름차순으로 들고, 같은 번호라도 생성이 다르면 따로 든다. 넣지 못한 것은
    /// 돌려받아 자물쇠 밖에서 버린다.
    #[test]
    fn a_record_takes_each_pid_create_pair_once_in_seq_order() {
        let recorder = Arc::new(Recorder::new());
        let probe = Probe::new(&recorder);
        assert_eq!(recorder.active(), 0);
        let (r, left) = recorder.start();
        assert_ne!(r, 0);
        assert!(left.is_empty());
        assert_eq!(recorder.active(), r);

        let first = recorded(&recorder, r, caught(10, 100, &probe));
        let second = recorded(&recorder, r, caught(10, 200, &probe));
        assert!(first < second);
        match recorder.admit(r, caught(10, 100, &probe)) {
            Admit::Known(again) => drop(again),
            _ => panic!("같은 (번호, 생성)이 두 번 들었다"),
        }
        assert_eq!(probe.drops(), 1);

        let births = recorder.copy(r);
        let got: Vec<_> = births
            .iter()
            .map(|b| (b.pid, b.facts.create, b.seq))
            .collect();
        assert_eq!(got, [(10, 100, first), (10, 200, second)]);
        let link = births[0].link();
        assert_eq!((link.pid, link.seq, link.killable), (10, Some(first), true));
        assert_eq!(recorder.len(), 2);
        drop(births);
        assert_eq!(probe.held_drops(), 0);
    }

    /// 상한을 넘는 것은 넣지 않고 `full` 이 선다(처음 넘친 것만 `first`). 새 기록이 `full` 을 내리고 남은 기록을
    /// 돌려주며, 받은 쪽이 버릴 때 — 자물쇠 밖에서 — 핸들이 닫힌다.
    #[test]
    fn a_full_record_refuses_past_the_cap_and_a_new_record_clears_it() {
        let recorder = Arc::new(Recorder::new());
        let probe = Probe::new(&recorder);
        let (r, _) = recorder.start();
        for pid in 0..BIRTH_MAX as u32 {
            recorded(&recorder, r, caught(pid, 1, &probe));
        }
        assert!(!recorder.full());
        for (pid, expect_first) in [(9_999, true), (9_998, false)] {
            match recorder.admit(r, caught(pid, 1, &probe)) {
                Admit::Full { caught, first } => {
                    assert_eq!(first, expect_first);
                    drop(caught);
                }
                _ => panic!("상한을 넘었는데 들었다"),
            }
        }
        assert!(recorder.full());
        assert_eq!(recorder.len(), BIRTH_MAX);

        let (next, left) = recorder.start();
        assert_ne!(next, r);
        assert!(!recorder.full());
        assert_eq!(recorder.len(), 0);
        assert_eq!(left.len(), BIRTH_MAX);
        let before = probe.drops();
        drop(left);
        assert_eq!(probe.drops() - before, BIRTH_MAX);
        assert_eq!(probe.held_drops(), 0);
    }

    /// 켜진 기록만 복사 · 끄기 · 넣기가 된다 — 꺼졌거나 다른 기록이 켜졌으면 빈 것을 주고 아무것도 안 바꾼다.
    #[test]
    fn only_the_running_record_is_copied_stopped_or_admitted() {
        let recorder = Arc::new(Recorder::new());
        let probe = Probe::new(&recorder);
        let (old, _) = recorder.start();
        recorded(&recorder, old, caught(1, 1, &probe));
        let (r, left) = recorder.start();
        assert_eq!(left.len(), 1);
        drop(left);

        assert!(recorder.copy(old).is_empty());
        assert!(recorder.stop(old).is_empty());
        assert_eq!(recorder.active(), r);
        match recorder.admit(old, caught(2, 1, &probe)) {
            Admit::Stale(late) => drop(late),
            _ => panic!("꺼진 기록에 들었다"),
        }
        recorded(&recorder, r, caught(3, 1, &probe));
        assert_eq!(recorder.copy(r).len(), 1);
        assert!(recorder.copy(0).is_empty());
        assert!(recorder.stop(0).is_empty());

        let stopped = recorder.stop(r);
        assert_eq!(stopped.len(), 1);
        assert_eq!(recorder.active(), 0);
        assert!(recorder.copy(r).is_empty());
        match recorder.admit(r, caught(4, 1, &probe)) {
            Admit::Stale(late) => drop(late),
            _ => panic!("끈 기록에 들었다"),
        }
        drop(stopped);
        assert_eq!(probe.held_drops(), 0);
    }

    /// 기록 자물쇠가 독에 걸려도 켜기 · 넣기 · 복사 · 끄기가 돈다.
    #[test]
    fn a_poisoned_record_lock_still_records() {
        let recorder = Arc::new(Recorder::new());
        let probe = Probe::new(&recorder);
        let poisoned = panic::catch_unwind(AssertUnwindSafe(|| {
            let _held = recorder.inner.lock();
            panic!("독 흉내");
        }));
        assert!(poisoned.is_err());
        assert!(recorder.inner.is_poisoned());

        let (r, _) = recorder.start();
        recorded(&recorder, r, caught(1, 1, &probe));
        assert_eq!(recorder.copy(r).len(), 1);
        assert_eq!(recorder.len(), 1);
        assert!(!recorder.full());
        assert_eq!(recorder.stop(r).len(), 1);
        assert_eq!(recorder.active(), 0);
    }

    // ── 듣는 스레드 ───────────────────────────────────────────────────────────────────

    /// 기록이 켜진 동안의 가입만 끝내기 권한으로 곧바로 붙들어 넣는다 — 꺼진 동안은 붙들지도 않고, 무리 밖 · 끝남 ·
    /// 다른 알림은 버린다. 붙들기는 기록 자물쇠 밖이고, 꺼내기에 들어설 때 Job 을 강하게 쥐지 않는다.
    #[test]
    fn the_listener_pins_joined_members_with_the_kill_right_only_while_recording() {
        let rig = Rig::new();
        rig.answer(10, true, Answer::Ours(facts(5, 100)));
        rig.listening();

        assert_eq!(rig.feed(PortEvent::Joined(10)), 1);
        assert!(rig.pins().is_empty(), "꺼진 동안 붙들었다");

        let (r, _) = rig.recorder.start();
        for event in [
            PortEvent::Joined(10),
            PortEvent::Joined(11),
            PortEvent::Other,
            PortEvent::Timeout,
        ] {
            assert_eq!(
                rig.feed(event),
                1,
                "꺼내기에 들어설 때 Job 을 강하게 쥐었다"
            );
        }
        assert_eq!(rig.pins(), [Call::Pin(10, true), Call::Pin(11, true)]);
        let births = rig.recorder.copy(r);
        assert_eq!(births.len(), 1);
        let birth = &births[0];
        assert_eq!(
            (birth.pid, birth.killable, &birth.facts),
            (10, true, &facts(5, 100))
        );
        assert_eq!(birth.link().seq, Some(birth.seq));
        assert_eq!(rig.probe.held_calls(), 0);
        assert_eq!(rig.probe.held_drops(), 0);
    }

    /// 끝내기 권한이 거절되면 조회 권한만으로 다시 붙들어 고리로만 넣는다 — 후보는 못 되지만 사슬은 잇는다. 그마저
    /// 못 붙들면 버린다.
    #[test]
    fn a_member_refused_the_kill_right_is_kept_as_a_chain_link_only() {
        let rig = Rig::new();
        rig.answer(20, true, Answer::Denied);
        rig.answer(20, false, Answer::Ours(facts(5, 100)));
        rig.answer(21, true, Answer::Denied);
        rig.answer(21, false, Answer::Denied);
        rig.answer(22, true, Answer::Denied);
        rig.listening();
        let (r, _) = rig.recorder.start();
        for pid in [20, 21, 22] {
            rig.feed(PortEvent::Joined(pid));
        }

        assert_eq!(
            rig.pins(),
            [
                Call::Pin(20, true),
                Call::Pin(20, false),
                Call::Pin(21, true),
                Call::Pin(21, false),
                Call::Pin(22, true),
                Call::Pin(22, false),
            ]
        );
        let births = rig.recorder.copy(r);
        assert_eq!(births.len(), 1);
        let link = births[0].link();
        assert_eq!((link.pid, link.killable), (20, false));

        let child = facts(20, 150);
        let links = [
            link,
            Link {
                pid: 30,
                facts: &child,
                seq: link.seq.map(|s| s + 1),
                killable: true,
            },
        ];
        let alive = |_: &Link<'_>| -> io::Result<bool> { Ok(false) };
        let view = Chain {
            root_pid: ROOT,
            snapshot: &[],
            births: &links,
            exited: &alive,
        };
        let picked: Vec<u32> = candidates(&view, &[]).iter().map(|l| l.pid).collect();
        assert_eq!(picked, [30], "고리 전용이 후보가 됐다");
        assert_eq!(parent_of(&view, 30, &child).map(|l| l.pid), Some(20));
    }

    /// 무리가 사라지면 켜진 기록을 끄고(놓은 뒤 버린다) 끝난다 — 무리가 없으니 떼지 않는다. 굳히지 않는다.
    #[test]
    fn the_listener_releases_the_record_and_ends_once_the_group_is_gone() {
        let mut rig = Rig::new();
        rig.answer(10, true, Answer::Ours(facts(5, 100)));
        rig.listening();
        rig.recorder.start();
        rig.feed(PortEvent::Joined(10));
        assert_eq!(rig.recorder.len(), 1);

        rig.vanish();
        rig.feed_last(Feed::Event(PortEvent::Timeout));
        assert_eq!(rig.recorder.active(), 0);
        assert_eq!(rig.recorder.len(), 0);
        assert_eq!(rig.probe.drops(), 1);
        assert_eq!(rig.probe.held_drops(), 0);
        assert!(!rig.calls().contains(&Call::Unwatch));
        assert!(
            rig.port.upgrade().is_none(),
            "끝난 스레드가 포트를 쥐고 있다"
        );
        assert!(!rig.recorder.port_failed());
        assert_eq!(rig.ensure(rig.spawner()), Ok(()));
        assert_eq!(rig.count(Level::WARN) + rig.count(Level::ERROR), 0);
    }

    /// 물러나면 켜진 기록을 끄고, 아직 붙은 무리에서 포트를 뗀 **뒤에** 놓는다.
    #[test]
    fn a_retiring_group_is_detached_before_the_port_is_let_go() {
        let rig = Rig::new();
        rig.answer(10, true, Answer::Ours(facts(5, 100)));
        rig.listening();
        rig.recorder.start();
        rig.feed(PortEvent::Joined(10));

        rig.retire();
        rig.feed_last(Feed::Event(PortEvent::Timeout));
        assert_eq!(rig.recorder.active(), 0);
        assert_eq!(rig.probe.drops(), 1);
        assert!(rig.calls().ends_with(&[Call::Unwatch, Call::PortDrop]));
        assert!(!rig.recorder.port_failed());
    }

    /// 꺼내기 오류 → 굳힘 · 켜진 기록 끔 · 뗌 → 끝. warn 한 번이고, 다음 에피소드의 확보는 굳은 칸을 보고 실패한다.
    #[test]
    fn a_failed_take_latches_the_port_stops_the_record_and_detaches() {
        let rig = Rig::new();
        rig.answer(10, true, Answer::Ours(facts(5, 100)));
        rig.listening();
        rig.recorder.start();
        rig.feed(PortEvent::Joined(10));

        rig.feed_last(Feed::Fail);
        assert!(rig.recorder.port_failed());
        assert_eq!(rig.recorder.active(), 0);
        assert_eq!(rig.probe.drops(), 1);
        assert_eq!(rig.probe.held_drops(), 0);
        assert!(rig.calls().ends_with(&[Call::Unwatch, Call::PortDrop]));
        assert_eq!(rig.count(Level::WARN), 1);
        assert_eq!(rig.count(Level::ERROR), 0);

        assert_eq!(rig.ensure(rig.spawner()), Err(()));
        let ports = rig.calls().iter().filter(|c| **c == Call::Port).count();
        assert_eq!(ports, 1, "포트를 두 번 만들었다");
    }

    /// 실패 모드에서 떼기마저 실패하면 끝나지 않고 꺼내 버리기만 한다(그 뒤 켠 기록에도 넣지 않는다) — 무리가 사라지면
    /// 끝난다.
    #[test]
    fn a_failed_detach_drains_without_recording_until_the_group_is_gone() {
        let mut rig = Rig::new();
        rig.group.unwatch_fails.store(true, Ordering::SeqCst);
        rig.answer(10, true, Answer::Ours(facts(5, 100)));
        rig.listening();
        rig.recorder.start();

        rig.fail_take();
        assert!(rig.recorder.port_failed());
        assert_eq!(rig.recorder.active(), 0);

        let (r, _) = rig.recorder.start();
        rig.feed(PortEvent::Joined(10));
        assert!(rig.pins().is_empty(), "비우기만 하는 중에 붙들었다");
        assert_eq!(rig.recorder.len(), 0);
        assert_eq!(rig.recorder.active(), r);

        rig.vanish();
        rig.feed_last(Feed::Event(PortEvent::Timeout));
        assert_eq!(rig.recorder.active(), 0);
        let unwatches = rig.calls().iter().filter(|c| **c == Call::Unwatch).count();
        assert_eq!(unwatches, 1);
        assert!(rig.port.upgrade().is_none());
        assert_eq!(rig.count(Level::WARN), 1);
    }

    /// 차례 몸통의 패닉도 꺼내기 오류처럼 굳히고 뗀 뒤 끝난다 — 패닉이라 error 로 남긴다.
    #[test]
    fn a_panicking_round_latches_the_port_like_a_failed_take() {
        let rig = Rig::new();
        rig.answer(10, true, Answer::Panic);
        rig.listening();
        rig.recorder.start();

        rig.feed_last(Feed::Event(PortEvent::Joined(10)));
        assert!(rig.recorder.port_failed());
        assert_eq!(rig.recorder.active(), 0);
        assert!(rig.calls().ends_with(&[Call::Unwatch, Call::PortDrop]));
        assert_eq!(rig.count(Level::ERROR), 1);
        assert_eq!(rig.count(Level::WARN), 0);
    }

    /// 여는 이가 「붙었나」를 보내기 전에 사라지면(붙었을 수 있다) 스레드는 평소대로 돈다.
    #[test]
    fn a_lost_attached_sender_keeps_the_listener_running() {
        let mut rig = Rig::new();
        rig.answer(10, true, Answer::Ours(facts(5, 100)));
        let port = rig.group.port.lock().unwrap().take().expect("포트");
        let (attached_tx, attached) = mpsc::sync_channel::<bool>(1);
        drop(attached_tx);
        let listener = Listener {
            port,
            attached,
            group: Arc::clone(&rig.group),
            recorder: Arc::clone(&rig.recorder),
            tag: LogTag::new(None, ROOT),
        };
        (rig.spawner())(Box::new(move || listen(listener))).expect("기동");
        rig.wait_next();

        let (r, _) = rig.recorder.start();
        rig.feed(PortEvent::Joined(10));
        assert_eq!(rig.recorder.copy(r).len(), 1);

        rig.vanish();
        rig.feed_last(Feed::Event(PortEvent::Timeout));
        assert_eq!(rig.recorder.active(), 0);
    }

    // ── 포트 확보 ─────────────────────────────────────────────────────────────────────

    /// 포트 → 듣는 스레드 기동 → 붙이기 순서이고, 스레드는 「붙었다」를 받고서 꺼내기에 들어간다. 두 번째 확보는 결과만
    /// 읽는다.
    #[test]
    fn the_port_comes_first_then_the_listener_then_the_attach() {
        let rig = Rig::new();
        assert_eq!(rig.ensure(rig.spawner()), Ok(()));
        rig.wait_next();
        assert_eq!(rig.calls(), [Call::Port, Call::Spawn, Call::Attach]);

        assert_eq!(rig.ensure(rig.spawner()), Ok(()));
        assert_eq!(rig.calls(), [Call::Port, Call::Spawn, Call::Attach]);
        assert!(!rig.recorder.port_failed());
        assert_eq!(rig.count(Level::WARN), 0);
    }

    /// 듣는 스레드를 못 띄우면 붙이지 않고 굳는다 — 포트는 아무도 쥐지 않는다. 두 번째 확보도 실패하고 다시 만들지
    /// 않는다.
    #[test]
    fn a_listener_that_fails_to_start_is_never_attached_and_latches() {
        let rig = Rig::new();
        assert_eq!(rig.ensure(rig.failing_spawner()), Err(()));
        assert_eq!(rig.calls(), [Call::Port, Call::Spawn, Call::PortDrop]);
        assert!(rig.recorder.port_failed());
        assert_eq!(rig.count(Level::WARN), 1);

        assert_eq!(rig.ensure(rig.spawner()), Err(()));
        assert_eq!(rig.calls(), [Call::Port, Call::Spawn, Call::PortDrop]);
    }

    /// 붙이기가 실패하면 굳고, 뜬 스레드는 「안 붙었다」를 받아 꺼내지도 떼지도 않고 끝난다.
    #[test]
    fn a_failed_attach_latches_and_the_listener_ends_without_detaching() {
        let rig = Rig::new();
        rig.group.attach_fails.store(true, Ordering::SeqCst);
        assert_eq!(rig.ensure(rig.spawner()), Err(()));
        rig.ended();

        assert!(rig.waits.try_recv().is_err(), "붙지 않은 포트에서 꺼냈다");
        assert_eq!(
            rig.calls(),
            [Call::Port, Call::Spawn, Call::Attach, Call::PortDrop]
        );
        assert!(rig.recorder.port_failed());
        assert_eq!(rig.count(Level::WARN), 1);
    }

    /// 무리가 이미 없거나 이 OS 에 무리가 없으면 그 에피소드만 실패한다 — 굳히지 않는다(OS 실패가 아니다).
    #[test]
    fn a_missing_group_fails_the_episode_without_latching() {
        for kind in [GROUP_GONE, io::ErrorKind::Unsupported] {
            let rig = Rig::new();
            *rig.group.refuse_watch.lock().unwrap() = Some(kind);
            assert_eq!(rig.ensure(rig.spawner()), Err(()), "{kind:?}");
            assert!(rig.calls().is_empty(), "{kind:?}: {:?}", rig.calls());
            assert!(!rig.recorder.port_failed(), "{kind:?}");
            assert_eq!(rig.count(Level::WARN), 0, "{kind:?}");
        }
    }

    // ── 실프로세스(Windows) ───────────────────────────────────────────────────────────

    /// 실물 손잡이 · 포트 · 스레드로 — 기록이 켜진 뒤 Job 안에서 태어난 ping 이 끝내기 권한으로 붙들려 사실과 함께
    /// 들고, Job 이 사라지면 듣는 스레드가 켜진 기록을 끄고 끝난다. cmd · 그 콘솔 호스트 · ping 셋.
    #[cfg(windows)]
    #[test]
    fn a_birth_in_the_job_is_recorded_with_its_facts_through_the_real_port() {
        use crate::platform::process_group::tests::{
            new_group, open_gate, spawn_gated_cmd, wait_until,
        };
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        let (job, group) = new_group();
        let group = Arc::new(group);
        let mut x = spawn_gated_cmd("ping -n 30 127.0.0.1", CREATE_NO_WINDOW);
        job.assign(x.id()).expect("Job 편입");
        let recorder = Arc::new(Recorder::new());
        let watch = BirthWatch::new();
        assert_eq!(
            watch.ensure_with(&group, &recorder, &LogTag::new(None, x.id()), |body| {
                SystemClock.spawn(LISTENER_THREAD, body)
            }),
            Ok(())
        );
        let (r, _) = recorder.start();
        open_gate(&mut x);

        let ping = wait_until("기록된 ping", || {
            recorder
                .copy(r)
                .into_iter()
                .find(|b| b.facts.image.to_ascii_lowercase().ends_with("\\ping.exe"))
        });
        assert_eq!(ping.facts.ppid, x.id(), "부모: {:?}", ping.facts);
        assert_ne!(ping.facts.create, 0);
        assert!(
            ping.facts.cmdline.contains("ping"),
            "명령줄: {:?}",
            ping.facts
        );
        assert!(ping.killable);
        assert!(!ping.pin.exited().expect("끝났나"));
        drop(ping);
        drop(recorder.stop(r));

        job.terminate(1).expect("Job 끝내기");
        let _ = x.wait();
        drop(job);
        let (after, _) = recorder.start();
        wait_until("무리가 사라져 듣는 스레드가 끈 기록", || {
            (recorder.active() != after).then_some(())
        });
        assert_eq!(recorder.active(), 0);
        assert!(!recorder.port_failed());
    }
}

#[cfg(test)]
mod test_support {
    //! 탄생 기록 · 문 시험이 함께 쓰는 가짜 — 붙든 것 · 자물쇠 탐지 · 로그 잡기.

    use super::*;

    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicU8, AtomicUsize};
    use std::sync::{PoisonError, Weak};

    use tracing::Level;

    /// [`FakePin`] 의 끝남 칸 값.
    pub(super) const ALIVE: u8 = 0;
    pub(super) const EXITED: u8 = 1;
    /// 끝남 조회가 `Err` 다.
    pub(super) const FAILING: u8 = 2;

    /// [`FakePin`] 의 끝내기 흉내.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum Kill {
        /// 끝내기 호출이 거절된다.
        Refuse,
        /// 받아들여지고 곧바로 끝난다.
        Exit,
        /// 받아들여지지만 끝나지 않는다 — 확인 마감을 넘긴다.
        Linger,
    }

    /// 붙든 것이 받은 부름 — [`Probe::calls`] 가 그때 두 자물쇠 가운데 하나라도 잡혀 있었나와 함께 적는다.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum PinCall {
        Exited,
        Wait,
        Terminate,
        Classify,
    }

    type Hook = Arc<dyn Fn() + Send + Sync>;

    /// 붙든 것을 버릴 때 · 무리에게 물을 때 기록 자물쇠나 문 자물쇠가 잡혀 있었는지 센다. 시험은 다른 스레드와 겹쳐
    /// 돌지 않게 순서를 맞추므로, 잡혀 있었다면 부른 스레드 자신이 쥔 것이다. 붙든 것의 부름을 적고, 끝남 조회에 시험의
    /// 일을 끼우고, 기다림 · 끝내기가 쓰는 시간만큼 시험 시계를 민다.
    pub(super) struct Probe {
        recorder: Weak<Recorder>,
        gate: OnceLock<Box<dyn Fn() -> bool + Send + Sync>>,
        pub(super) drops: AtomicUsize,
        pub(super) held_drops: AtomicUsize,
        pub(super) held_calls: AtomicUsize,
        calls: Mutex<Vec<(u32, PinCall, bool)>>,
        hooks: Mutex<BTreeMap<u32, Hook>>,
        clock: OnceLock<Box<dyn Fn(Duration) + Send + Sync>>,
        waits: Mutex<Vec<Duration>>,
        terminate_cost: Mutex<Duration>,
    }

    impl Probe {
        pub(super) fn new(recorder: &Arc<Recorder>) -> Arc<Self> {
            Arc::new(Self {
                recorder: Arc::downgrade(recorder),
                gate: OnceLock::new(),
                drops: AtomicUsize::new(0),
                held_drops: AtomicUsize::new(0),
                held_calls: AtomicUsize::new(0),
                calls: Mutex::default(),
                hooks: Mutex::default(),
                clock: OnceLock::new(),
                waits: Mutex::default(),
                terminate_cost: Mutex::default(),
            })
        }

        /// 기다림 · 끝내기가 쓴 시간을 시험 시계에 민다.
        pub(super) fn drive(&self, advance: impl Fn(Duration) + Send + Sync + 'static) {
            let _ = self.clock.set(Box::new(advance));
        }

        pub(super) fn set_terminate_cost(&self, d: Duration) {
            *self.terminate_cost.lock().unwrap() = d;
        }

        /// `pid` 를 붙든 것의 끝남 조회마다 돌 일 — 자물쇠 안에서 불리면 돌리지 않고 센다(같은 자물쇠에 매달리지 않게).
        pub(super) fn on_exited(&self, pid: u32, hook: impl Fn() + Send + Sync + 'static) {
            self.hooks.lock().unwrap().insert(pid, Arc::new(hook));
        }

        /// 끼운 일이 쥔 문 · 정리기의 고리를 끊는다.
        pub(super) fn clear_hooks(&self) {
            if let Ok(mut hooks) = self.hooks.lock() {
                hooks.clear();
            }
        }

        /// 붙든 것이 받은 부름 — (번호, 부름, 그때 자물쇠가 잡혀 있었나), 부른 순서대로.
        pub(super) fn calls(&self) -> Vec<(u32, PinCall, bool)> {
            self.calls.lock().unwrap().clone()
        }

        /// 산 것을 실제로 기다린 시간 — 부른 순서대로.
        pub(super) fn waits(&self) -> Vec<Duration> {
            self.waits.lock().unwrap().clone()
        }

        fn note(&self, pid: u32, call: PinCall) -> bool {
            let held = self.held();
            if held && call != PinCall::Terminate {
                self.held_calls.fetch_add(1, Ordering::SeqCst);
            }
            self.calls.lock().unwrap().push((pid, call, held));
            held
        }

        fn spend(&self, d: Duration) {
            if let Some(advance) = self.clock.get() {
                advance(d);
            }
        }

        fn hook(&self, pid: u32) -> Option<Hook> {
            self.hooks.lock().unwrap().get(&pid).cloned()
        }

        /// 문 자물쇠도 본다.
        pub(super) fn watch_gate<G: Group>(&self, cell: &Arc<GateCell<G>>) {
            let cell = Arc::downgrade(cell);
            let _ = self.gate.set(Box::new(move || {
                let Some(cell) = cell.upgrade() else {
                    return false;
                };
                let held = matches!(cell.state.try_lock(), Err(TryLockError::WouldBlock));
                held
            }));
        }

        pub(super) fn held(&self) -> bool {
            let record = match self.recorder.upgrade() {
                Some(recorder) => {
                    let held = matches!(recorder.inner.try_lock(), Err(TryLockError::WouldBlock));
                    held
                }
                None => false,
            };
            record || self.gate.get().is_some_and(|gate| gate())
        }

        pub(super) fn drops(&self) -> usize {
            self.drops.load(Ordering::SeqCst)
        }

        pub(super) fn held_drops(&self) -> usize {
            self.held_drops.load(Ordering::SeqCst)
        }

        pub(super) fn held_calls(&self) -> usize {
            self.held_calls.load(Ordering::SeqCst)
        }
    }

    /// 붙든 프로세스 흉내 — 끝남은 칸이 정하고, 버릴 때 두 자물쇠가 잡혀 있었는지 적는다. 끝내기는 `kill` 이 정한다
    /// (기본 = 거절). `pid` 는 부름 기록의 이름표일 뿐이다(기본 0).
    pub(super) struct FakePin {
        facts: ProcessFacts,
        probe: Arc<Probe>,
        exit: Arc<AtomicU8>,
        pid: u32,
        kill: Kill,
    }

    impl FakePin {
        pub(super) fn new(facts: ProcessFacts, probe: &Arc<Probe>) -> Self {
            Self::with_exit(facts, probe, Arc::new(AtomicU8::new(ALIVE)))
        }

        pub(super) fn with_exit(
            facts: ProcessFacts,
            probe: &Arc<Probe>,
            exit: Arc<AtomicU8>,
        ) -> Self {
            Self {
                facts,
                probe: Arc::clone(probe),
                exit,
                pid: 0,
                kill: Kill::Refuse,
            }
        }

        pub(super) fn named(mut self, pid: u32) -> Self {
            self.pid = pid;
            self
        }

        pub(super) fn killed_by(mut self, kill: Kill) -> Self {
            self.kill = kill;
            self
        }

        fn state(&self) -> io::Result<bool> {
            match self.exit.load(Ordering::SeqCst) {
                ALIVE => Ok(false),
                EXITED => Ok(true),
                _ => Err(io::Error::other("끝남 조회 실패 흉내")),
            }
        }
    }

    impl Pinned for FakePin {
        fn exited(&self) -> io::Result<bool> {
            let held = self.probe.note(self.pid, PinCall::Exited);
            if let (false, Some(hook)) = (held, self.probe.hook(self.pid)) {
                hook();
            }
            self.state()
        }

        fn wait_exit(&self, d: Duration) -> io::Result<bool> {
            self.probe.note(self.pid, PinCall::Wait);
            if let Ok(false) = self.state() {
                self.probe.waits.lock().unwrap().push(d);
                self.probe.spend(d);
            }
            self.state()
        }

        fn facts(&self) -> &ProcessFacts {
            &self.facts
        }

        fn terminate_raw(&self) -> io::Result<()> {
            self.probe.note(self.pid, PinCall::Terminate);
            let cost = *self.probe.terminate_cost.lock().unwrap();
            self.probe.spend(cost);
            match self.kill {
                Kill::Refuse => Err(io::Error::from(io::ErrorKind::PermissionDenied)),
                Kill::Exit => {
                    self.exit.store(EXITED, Ordering::SeqCst);
                    Ok(())
                }
                Kill::Linger => Ok(()),
            }
        }

        /// 실물처럼 — 끝내기가 실패했어도 그 사이 끝났으면 `Gone`.
        fn classify(&self, raw: io::Result<()>) -> io::Result<MemberKill> {
            self.probe.note(self.pid, PinCall::Classify);
            match raw {
                Ok(()) => Ok(MemberKill::Terminated),
                Err(_) if matches!(self.state(), Ok(true)) => Ok(MemberKill::Gone),
                Err(e) => Err(e),
            }
        }
    }

    impl Drop for FakePin {
        fn drop(&mut self) {
            self.probe.drops.fetch_add(1, Ordering::SeqCst);
            if self.probe.held() {
                self.probe.held_drops.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    // ── 로그 잡기 ─────────────────────────────────────────────────────────────────────

    pub(super) type Line = (Level, BTreeMap<String, String>);

    pub(super) struct Capture(pub(super) Arc<Mutex<Vec<Line>>>);

    struct Fields<'a>(&'a mut BTreeMap<String, String>);

    impl tracing::field::Visit for Fields<'_> {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
            self.0
                .insert(field.name().to_string(), format!("{value:?}"));
        }

        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.0.insert(field.name().to_string(), value.to_string());
        }
    }

    impl tracing::Subscriber for Capture {
        fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            let mut fields = BTreeMap::new();
            event.record(&mut Fields(&mut fields));
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((*event.metadata().level(), fields));
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }
}

#[cfg(test)]
mod gate_tests {
    use super::test_support::{
        Capture, FakePin, Kill, Line, PinCall, Probe, ALIVE, EXITED, FAILING,
    };
    use super::tests::{p1, p1_chain, p1_two_owners, p3bg, p3bg_deeper, p3t3, pilot, Staged};
    use super::*;

    use std::collections::{BTreeMap, HashMap};
    use std::sync::atomic::{AtomicU8, AtomicUsize};

    use tracing::Level;

    const ROOT: u32 = 100;
    const CONHOST: u32 = 101;
    const CLAUDE: u32 = 102;
    const N: Duration = INTERRUPT_LEFTOVER_GRACE;
    const LINE: &[u8] = b"{\"type\":\"control_request\"}\n";

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn facts(ppid: u32, create: u64) -> ProcessFacts {
        ProcessFacts {
            ppid,
            create,
            image: r"C:\Program Files\Git\usr\bin\bash.exe".to_owned(),
            cmdline: "bash x.sh".to_owned(),
        }
    }

    // ── 가짜 ──────────────────────────────────────────────────────────────────────────

    /// 시험이 모는 시계 — 잠은 그만큼 시각을 밀고(적어 둔다), 기동은 몸통을 쌓아 둔다(돌릴지는 시험이 정한다).
    struct FakeClock {
        base: Instant,
        nanos: AtomicU64,
        sleeps: Mutex<Vec<Duration>>,
        bodies: Mutex<Vec<(String, Box<dyn FnOnce() + Send>)>>,
        keep_workers: AtomicBool,
        refuse_worker: AtomicBool,
        panic_in_sleep: AtomicBool,
    }

    impl FakeClock {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                base: Instant::now(),
                nanos: AtomicU64::new(0),
                sleeps: Mutex::default(),
                bodies: Mutex::default(),
                keep_workers: AtomicBool::new(true),
                refuse_worker: AtomicBool::new(false),
                panic_in_sleep: AtomicBool::new(false),
            })
        }

        fn advance(&self, d: Duration) {
            let nanos = u64::try_from(d.as_nanos()).expect("시험 시각");
            self.nanos.fetch_add(nanos, Ordering::SeqCst);
        }

        /// 시험 시작에서 `d` 뒤의 단조 시각.
        fn at(&self, d: Duration) -> Instant {
            self.base + d
        }

        fn sleeps(&self) -> Vec<Duration> {
            self.sleeps.lock().unwrap().clone()
        }

        fn take(&self, name: &str) -> Vec<Box<dyn FnOnce() + Send>> {
            let mut bodies = self.bodies.lock().unwrap();
            let (taken, kept): (Vec<_>, Vec<_>) =
                bodies.drain(..).partition(|(n, _)| n.as_str() == name);
            *bodies = kept;
            taken.into_iter().map(|(_, body)| body).collect()
        }

        fn spawned(&self, name: &str) -> usize {
            self.bodies
                .lock()
                .unwrap()
                .iter()
                .filter(|(n, _)| n.as_str() == name)
                .count()
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            self.base + Duration::from_nanos(self.nanos.load(Ordering::SeqCst))
        }
    }

    impl LeftoverClock for FakeClock {
        fn sleep(&self, d: Duration) {
            self.sleeps.lock().unwrap().push(d);
            if self.panic_in_sleep.load(Ordering::SeqCst) {
                panic!("잠 패닉 흉내");
            }
            self.advance(d);
        }

        fn spawn(&self, name: &str, body: Box<dyn FnOnce() + Send>) -> io::Result<()> {
            if name == WORKER_THREAD {
                if self.refuse_worker.load(Ordering::SeqCst) {
                    return Err(io::Error::other("기동 실패 흉내"));
                }
                if !self.keep_workers.load(Ordering::SeqCst) {
                    drop(body);
                    return Ok(());
                }
            }
            self.bodies.lock().unwrap().push((name.to_owned(), body));
            Ok(())
        }
    }

    #[derive(Clone)]
    enum Answer {
        Ours(ProcessFacts),
        Denied,
        Panic,
    }

    /// 문 시험의 무리 — 명단 · 붙들기 답을 시험이 정하고, 명단을 물을 때 시험의 일 하나를 끼워 돌린다(두 구간 사이 ·
    /// 쓰기와 명단 사이의 사건을 흉내 낸다).
    struct GateGroup {
        /// `None` = 명단 실패.
        listing: Mutex<Option<Vec<u32>>>,
        answers: Mutex<HashMap<u32, Answer>>,
        exits: Mutex<HashMap<u32, Arc<AtomicU8>>>,
        refuse_watch: Mutex<Option<io::ErrorKind>>,
        retiring: Arc<AtomicBool>,
        hook: Mutex<Option<Box<dyn FnOnce() + Send>>>,
        probe: Arc<Probe>,
        lists: AtomicUsize,
        pins: AtomicUsize,
        watches: AtomicUsize,
    }

    impl GateGroup {
        fn exit_of(&self, pid: u32) -> Arc<AtomicU8> {
            Arc::clone(
                self.exits
                    .lock()
                    .unwrap()
                    .entry(pid)
                    .or_insert_with(|| Arc::new(AtomicU8::new(ALIVE))),
            )
        }
    }

    impl Group for GateGroup {
        fn member_pids(&self) -> io::Result<Vec<u32>> {
            self.lists.fetch_add(1, Ordering::SeqCst);
            if self.probe.held() {
                // 자물쇠를 쥔 채 불렸다 — 끼워 둔 일이 같은 자물쇠를 잡아 매달리지 않게 돌리지 않는다.
                self.probe.held_calls.fetch_add(1, Ordering::SeqCst);
            } else {
                let hook = self.hook.lock().unwrap().take();
                if let Some(hook) = hook {
                    hook();
                }
            }
            let listing = self.listing.lock().unwrap().clone();
            listing.ok_or_else(|| io::Error::other("명단 실패 흉내"))
        }

        fn pin(&self, pid: u32, _kill: bool) -> io::Result<Option<Box<dyn Pinned>>> {
            self.pins.fetch_add(1, Ordering::SeqCst);
            if self.probe.held() {
                self.probe.held_calls.fetch_add(1, Ordering::SeqCst);
            }
            let answer = self.answers.lock().unwrap().get(&pid).cloned();
            match answer {
                None => Ok(None),
                Some(Answer::Ours(facts)) => Ok(Some(Box::new(
                    FakePin::with_exit(facts, &self.probe, self.exit_of(pid)).named(pid),
                ))),
                Some(Answer::Denied) => Err(io::Error::from(io::ErrorKind::PermissionDenied)),
                Some(Answer::Panic) => panic!("붙들기 패닉 흉내"),
            }
        }

        fn watch_births(
            &self,
            start: impl FnOnce(Arc<dyn Births>) -> io::Result<()>,
        ) -> io::Result<()> {
            self.watches.fetch_add(1, Ordering::SeqCst);
            let refused = *self.refuse_watch.lock().unwrap();
            if let Some(kind) = refused {
                return Err(io::Error::new(kind, "포트 거절 흉내"));
            }
            start(Arc::new(IdlePort))
        }

        fn unwatch_births(&self) -> io::Result<()> {
            Ok(())
        }

        fn is_gone(&self) -> bool {
            false
        }

        fn retiring(&self) -> RetiringSignal {
            RetiringSignal::of(&self.retiring)
        }
    }

    struct IdlePort;

    impl Births for IdlePort {
        fn next(&self, _: Duration) -> io::Result<PortEvent> {
            Ok(PortEvent::Timeout)
        }
    }

    // ── 판 ────────────────────────────────────────────────────────────────────────────

    /// 뿌리(`cmd /c`) · 콘솔 호스트 · claude 셋이 든 무리와 그 문 — 문은 닫혀 있다. 듣는 스레드 · 일꾼 몸통은 시계에
    /// 쌓일 뿐 시험이 돌리지 않으면 돌지 않는다(탄생은 시험이 기록에 직접 넣는다).
    struct GateRig {
        root: u32,
        clock: Arc<FakeClock>,
        group: Arc<GateGroup>,
        recorder: Arc<Recorder>,
        probe: Arc<Probe>,
        cleaner: Arc<Cleaner<GateGroup>>,
        cell: Arc<GateCell<GateGroup>>,
        lines: Arc<Mutex<Vec<Line>>>,
    }

    impl GateRig {
        fn new() -> Self {
            Self::build(
                ROOT,
                // 정렬되지 않은 명단 — 쓰기 명단은 정렬해 둔다.
                vec![CLAUDE, ROOT, CONHOST],
                HashMap::from([
                    (ROOT, Answer::Ours(facts(1, 10))),
                    (CONHOST, Answer::Ours(facts(ROOT, 20))),
                    (CLAUDE, Answer::Ours(facts(ROOT, 30))),
                ]),
                None,
            )
        }

        fn build(
            root: u32,
            listing: Vec<u32>,
            answers: HashMap<u32, Answer>,
            agent: Option<AgentId>,
        ) -> Self {
            let recorder = Arc::new(Recorder::new());
            let probe = Probe::new(&recorder);
            let group = Arc::new(GateGroup {
                listing: Mutex::new(Some(listing)),
                answers: Mutex::new(answers),
                exits: Mutex::default(),
                refuse_watch: Mutex::default(),
                retiring: Arc::default(),
                hook: Mutex::default(),
                probe: Arc::clone(&probe),
                lists: AtomicUsize::new(0),
                pins: AtomicUsize::new(0),
                watches: AtomicUsize::new(0),
            });
            let clock = FakeClock::new();
            let driven = Arc::clone(&clock);
            probe.drive(move |d| driven.advance(d));
            let cleaner = Arc::new(Cleaner::with_parts(
                Arc::clone(&group),
                Arc::clone(&recorder),
                Arc::clone(&clock) as Arc<dyn LeftoverClock>,
                root,
                LogTag::new(agent, root),
            ));
            let cell = GateCell::new(Some(Arc::clone(&cleaner)));
            probe.watch_gate(&cell);
            Self {
                root,
                clock,
                group,
                recorder,
                probe,
                cleaner,
                cell,
                lines: Arc::default(),
            }
        }

        fn logged<R>(&self, f: impl FnOnce() -> R) -> R {
            tracing::subscriber::with_default(Capture(Arc::clone(&self.lines)), f)
        }

        fn open(&self) {
            self.cell.open_turn();
        }

        fn esc(&self) -> Option<InterruptOut> {
            self.logged(|| self.cell.interrupt(LINE.to_vec()))
        }

        /// 줄이 쓰인 것으로 친다 — 부를 것이 있으면 라이터처럼 아무 락 없이 부른다.
        fn write(&self, out: InterruptOut) {
            if let Some(on_written) = out.on_written {
                self.logged(on_written);
            }
        }

        /// 쓰기 확인을 받는 끊기 — 줄과 부를 것이 둘 다 있어야 한다.
        fn esc_with_check(&self) -> InterruptOut {
            let out = self.esc().expect("끊기 줄");
            assert_eq!(out.bytes, LINE);
            assert!(out.on_written.is_some(), "쓰기 확인이 없다");
            out
        }

        fn turn(&self) -> bool {
            self.logged(|| worker_turn(&self.cell, &self.cleaner))
        }

        /// 쌓인 일꾼 몸통을 이 스레드에서 끝까지 돌린다.
        fn run_workers(&self) {
            for body in self.clock.take(WORKER_THREAD) {
                self.logged(body);
            }
        }

        /// 다음 명단 조회(스냅숏 · 쓰기 명단) 때 한 번 돌 일.
        fn on_list(&self, f: impl FnOnce() + Send + 'static) {
            *self.group.hook.lock().unwrap() = Some(Box::new(f));
        }

        fn answer(&self, pid: u32, answer: Answer) {
            self.group.answers.lock().unwrap().insert(pid, answer);
        }

        fn listing(&self, listing: Option<Vec<u32>>) {
            *self.group.listing.lock().unwrap() = listing;
        }

        fn state<R>(&self, f: impl FnOnce(&GateState) -> R) -> R {
            let state = self.cell.lock();
            f(&state)
        }

        fn episode<R>(&self, f: impl FnOnce(&Episode) -> R) -> R {
            self.state(|st| f(st.episode.as_ref().expect("에피소드")))
        }

        fn has_episode(&self) -> bool {
            self.state(|st| st.episode.is_some())
        }

        fn worker(&self) -> bool {
            self.state(|st| st.worker)
        }

        fn next_rec(&self) -> u64 {
            self.recorder.lock().next_rec
        }

        fn count(&self, level: Level) -> usize {
            self.lines
                .lock()
                .unwrap()
                .iter()
                .filter(|(l, _)| *l == level)
                .count()
        }

        fn logged_reason(&self, reason: &str) -> bool {
            self.lines
                .lock()
                .unwrap()
                .iter()
                .any(|(_, fields)| fields.get("reason").is_some_and(|r| r == reason))
        }

        fn logged_text(&self, text: &str) -> bool {
            self.lines
                .lock()
                .unwrap()
                .iter()
                .any(|(_, fields)| fields.get("message").is_some_and(|m| m.contains(text)))
        }

        /// 이 기록에 탄생 하나를 넣는다 — 듣는 스레드가 붙들어 넣은 것처럼.
        fn admit(&self, rec: u64, pid: u32, facts: ProcessFacts, exit: u8) {
            admit_into(&self.recorder, &self.probe, rec, pid, facts, exit);
        }

        /// 세계 `w` 를 판 직전까지 싣는다 — 끊기 · 쓰임 · 탄생(`births` 순서가 `seq` · 끝내기 흉내는 `kills`, 그 밖은
        /// [`Kill::Exit`]) · 판 때의 끝남을 싣고 N 을 민다. 끝남 칸은 번호마다 하나다([`GateGroup::exit_of`]).
        fn staged(w: &Staged, kills: &[(u32, Kill)]) -> Self {
            let answers = w
                .members
                .iter()
                .map(|(pid, facts, _)| (*pid, Answer::Ours(facts.clone())))
                .collect();
            let rig = Self::build(
                w.root,
                w.listing.clone(),
                answers,
                Some(AgentId::from_u128(0x7440)),
            );
            rig.open();
            let out = rig.esc_with_check();
            rig.write(out);
            let rec = rig.recorder.active();
            for (pid, facts, exited) in &w.births {
                let exit = rig.group.exit_of(*pid);
                exit.store(if *exited { EXITED } else { ALIVE }, Ordering::SeqCst);
                let kill = kills
                    .iter()
                    .find(|(p, _)| p == pid)
                    .map_or(Kill::Exit, |(_, k)| *k);
                let pin = FakePin::with_exit(facts.clone(), &rig.probe, exit)
                    .named(*pid)
                    .killed_by(kill);
                admit_pin(&rig.recorder, rec, *pid, facts.clone(), pin);
            }
            for (pid, _, exited) in &w.members {
                if *exited {
                    rig.group.exit_of(*pid).store(EXITED, Ordering::SeqCst);
                }
            }
            rig.clock.advance(N);
            rig
        }

        /// 일꾼이 돌 판 한 번 — 판이 서야 한다.
        fn pass(&self) -> PassReport {
            let step = {
                let mut state = self.cell.lock();
                state.next_step(self.clock.now(), self.cleaner.retiring.is_set())
            };
            let Step::Pass(ticket) = step else {
                panic!("판이 서야 한다");
            };
            self.logged(|| run_pass(&self.cell, &self.cleaner, ticket))
        }

        /// 뿌리 끝남 조회의 `nth` 번째에 한 번 돌 일 — 1 = 판의 순서 3 · 2 = 첫 끝내기 앞 재확인 · 3 = 둘째 끝내기 앞.
        /// 고르기와 확정 사이의 사건을 흉내 낸다.
        fn on_root_check(&self, nth: usize, f: impl Fn() + Send + Sync + 'static) {
            let seen = AtomicUsize::new(0);
            self.probe.on_exited(self.root, move || {
                if seen.fetch_add(1, Ordering::SeqCst) + 1 == nth {
                    f();
                }
            });
        }

        /// 끝내기 호출을 받은 번호 — 부른 순서대로.
        fn terminates(&self) -> Vec<u32> {
            self.probe
                .calls()
                .into_iter()
                .filter(|(_, call, _)| *call == PinCall::Terminate)
                .map(|(pid, _, _)| pid)
                .collect()
        }

        /// 끝내기 호출은 모두 문 자물쇠 안이고 한 구간에 하나다 — 두 끝내기 사이에는 늘 자물쇠 밖의 부름(가르기 ·
        /// 재확인)이 있다. 그 밖의 부름 · 핸들 닫기 · 무리 조회는 자물쇠 안에 없다.
        fn assert_lock_sections(&self) {
            let mut outside_since_last = true;
            for (pid, call, held) in self.probe.calls() {
                if call == PinCall::Terminate {
                    assert!(held, "자물쇠 밖에서 끝냈다: {pid}");
                    assert!(outside_since_last, "한 구간에서 둘을 끝냈다: {pid}");
                    outside_since_last = false;
                } else {
                    assert!(!held, "자물쇠 안에서 {call:?}: {pid}");
                    outside_since_last = true;
                }
            }
            self.check();
        }

        /// `reason` 칸이 그 값인 줄 — 레벨과 칸.
        fn line_with_reason(&self, reason: &str) -> Option<Line> {
            self.lines
                .lock()
                .unwrap()
                .iter()
                .find(|(_, fields)| fields.get("reason").is_some_and(|r| r == reason))
                .cloned()
        }

        /// 정리함 줄(`terminated` 칸이 있는 warn).
        fn cleaned_line(&self) -> Option<BTreeMap<String, String>> {
            self.lines
                .lock()
                .unwrap()
                .iter()
                .find(|(level, fields)| *level == Level::WARN && fields.contains_key("terminated"))
                .map(|(_, fields)| fields.clone())
        }

        /// 자물쇠 밖에서 보이는 모든 순간의 불변식(TRD §3-2) + 자물쇠 안에서 버리지도 · 묻지도 않았다.
        fn check(&self) {
            let active = self.recorder.active();
            self.state(|st| {
                if let Some(ep) = &st.episode {
                    if ep.is_live() {
                        assert!(st.open, "산 에피소드인데 문이 닫혔다: {ep:?}");
                    }
                    if ep.written {
                        assert!(ep.w.is_some(), "쓰였는데 쓰기 명단이 없다: {ep:?}");
                    }
                    if (ep.written && ep.is_live()) || matches!(ep.phase, Phase::Spent { .. }) {
                        assert!(st.worker, "일꾼 없이 {ep:?}");
                    }
                    if matches!(ep.snap, SnapState::Taken(_)) || ep.rec != 0 {
                        assert!(
                            ep.is_live(),
                            "끝난 에피소드가 스냅숏 · 기록을 쥐었다: {ep:?}"
                        );
                    }
                    assert!(ep.last_mono >= ep.first_mono);
                }
                assert!(!st.opening, "여는 이가 없는데 opening 이 섰다");
                if active != 0 {
                    assert!(
                        st.episode
                            .as_ref()
                            .is_some_and(|ep| ep.is_live() && ep.rec == active),
                        "켜진 기록 {active} 을 쥔 산 에피소드가 없다: {:?}",
                        st.episode
                    );
                }
            });
            assert_eq!(self.probe.held_drops(), 0, "자물쇠 안에서 핸들을 닫았다");
            assert_eq!(self.probe.held_calls(), 0, "자물쇠 안에서 무리에게 물었다");
        }
    }

    impl Drop for GateRig {
        fn drop(&mut self) {
            // 끼워 둔 일 · 쌓인 몸통이 문을 쥐어 생긴 고리를 끊는다.
            if let Ok(mut hook) = self.group.hook.lock() {
                hook.take();
            }
            self.probe.clear_hooks();
            if let Ok(mut bodies) = self.clock.bodies.lock() {
                bodies.clear();
            }
        }
    }

    /// 기록 `rec` 에 탄생 하나를 넣는다 — 듣는 스레드가 붙들어 넣은 것처럼.
    fn admit_into(
        recorder: &Recorder,
        probe: &Arc<Probe>,
        rec: u64,
        pid: u32,
        facts: ProcessFacts,
        exit: u8,
    ) {
        let pin =
            FakePin::with_exit(facts.clone(), probe, Arc::new(AtomicU8::new(exit))).named(pid);
        admit_pin(recorder, rec, pid, facts, pin);
    }

    fn admit_pin(recorder: &Recorder, rec: u64, pid: u32, facts: ProcessFacts, pin: FakePin) {
        let caught = Caught {
            pid,
            facts,
            pin: Box::new(pin),
            killable: true,
        };
        match recorder.admit(rec, caught) {
            Admit::Recorded { .. } => {}
            _ => panic!("탄생이 기록에 안 들었다"),
        }
    }

    /// 쓰인 `Waiting` 에피소드 — 순수 전이 시험용.
    fn written_episode(gen: u64, at: Instant, rec: u64) -> Episode {
        Episode {
            gen,
            first_mono: at,
            last_mono: at,
            written: true,
            w: Some(Arc::from(&[ROOT][..])),
            snap: SnapState::Failed(SnapCause::List),
            rec,
            killed_any: false,
            phase: Phase::Waiting,
        }
    }

    // ── 두 구간 · 여는 이 ─────────────────────────────────────────────────────────────

    /// 정리기가 없으면 문만 본다 — 닫히면 줄 없음 · 열리면 쓰기 확인 없는 줄.
    #[test]
    fn without_a_cleaner_the_gate_only_answers_open_or_closed() {
        let cell = GateCell::<GateGroup>::new(None);
        assert!(cell.interrupt(LINE.to_vec()).is_none());
        cell.open_turn();
        let out = cell.interrupt(LINE.to_vec()).expect("열린 문");
        assert_eq!(out.bytes, LINE);
        assert!(out.on_written.is_none());
        cell.deliver();
        assert!(cell.is_open());
        cell.close_turn();
        assert!(!cell.is_open());
        assert!(cell.interrupt(LINE.to_vec()).is_none());
        assert!(cell.lock().episode.is_none());
    }

    /// 닫힌 문은 줄을 주지 않고 아무것도 켜지 · 묻지 않는다.
    #[test]
    fn a_closed_gate_gives_no_line_and_touches_nothing() {
        let rig = GateRig::new();
        assert!(rig.esc().is_none());
        assert!(!rig.has_episode());
        assert_eq!(rig.next_rec(), 0);
        assert_eq!(rig.group.lists.load(Ordering::SeqCst), 0);
        assert_eq!(rig.group.watches.load(Ordering::SeqCst), 0);
        rig.check();
    }

    /// 첫 끊기가 여는 이 — 포트 확보 → 기록 켬 → 스냅숏(그때 기록이 이미 켜져 있다) → 쓰이지 않은 `Waiting`
    /// 에피소드. 스냅숏은 뿌리가 처음인 붙든 멤버 셋이다.
    #[test]
    fn the_first_interrupt_records_before_the_snapshot_and_opens_a_waiting_episode() {
        let rig = GateRig::new();
        rig.open();
        let at_snapshot = Arc::new(AtomicU64::new(0));
        let (seen, recorder) = (Arc::clone(&at_snapshot), Arc::clone(&rig.recorder));
        rig.on_list(move || seen.store(recorder.active(), Ordering::SeqCst));

        rig.esc_with_check();
        let rec = rig.recorder.active();
        assert_ne!(rec, 0);
        assert_eq!(
            at_snapshot.load(Ordering::SeqCst),
            rec,
            "스냅숏 때 기록이 꺼져 있었다"
        );
        rig.episode(|ep| {
            assert_eq!(ep.gen, 0);
            assert_eq!(ep.phase, Phase::Waiting);
            assert!(!ep.written);
            assert_eq!(ep.rec, rec);
            assert_eq!(
                (ep.first_mono, ep.last_mono),
                (rig.clock.at(ms(0)), rig.clock.at(ms(0)))
            );
            let SnapState::Taken(snap) = &ep.snap else {
                panic!("스냅숏: {:?}", ep.snap);
            };
            let links = snap.links();
            let pids: Vec<u32> = links.iter().map(|l| l.pid).collect();
            assert_eq!(pids.first(), Some(&ROOT));
            assert_eq!(links.len(), 3);
            assert!(links.iter().all(|l| l.seq.is_none() && !l.killable));
        });
        assert_eq!(rig.group.watches.load(Ordering::SeqCst), 1);
        assert_eq!(rig.group.lists.load(Ordering::SeqCst), 1);
        assert_eq!(rig.clock.spawned(LISTENER_THREAD), 1);
        assert_eq!(
            rig.clock.spawned(WORKER_THREAD),
            0,
            "쓰이기 전에 일꾼을 띄웠다"
        );
        assert!(!rig.worker());
        assert!(rig.logged_text("끊기 에피소드를 열었다"));
        rig.check();
    }

    /// 산 에피소드의 뒤 끊기는 이어 적는다 — 같은 세대로 쓰기 확인을 받고, 명단 · 기록 켬 · 포트가 다시 없다. 도는
    /// 판(`Cleaning`)은 `Waiting` 으로 돌아가고 가장 최근 끊기가 당겨진다.
    #[test]
    fn a_later_interrupt_continues_the_live_episode_without_listing_or_recording() {
        let rig = GateRig::new();
        rig.open();
        rig.esc_with_check();
        rig.clock.advance(ms(500));
        rig.esc_with_check();
        rig.episode(|ep| {
            assert_eq!(ep.gen, 0);
            assert_eq!(ep.last_mono, rig.clock.at(ms(500)));
        });
        rig.cell.lock().episode.as_mut().expect("에피소드").phase = Phase::Cleaning;
        rig.clock.advance(ms(100));
        rig.esc_with_check();
        rig.episode(|ep| {
            assert_eq!(ep.phase, Phase::Waiting);
            assert_eq!(ep.last_mono, rig.clock.at(ms(600)));
        });
        assert_eq!(rig.group.lists.load(Ordering::SeqCst), 1);
        assert_eq!(rig.group.watches.load(Ordering::SeqCst), 1);
        assert_eq!(rig.next_rec(), 1);
        rig.check();
    }

    /// 두 구간 사이에 턴이 닫히면 줄이 없다 — 기록을 끄고 찍은 스냅숏을 자물쇠 밖에서 버리고 `opening` 을 내린다.
    #[test]
    fn a_turn_end_between_the_sections_gives_no_line() {
        let rig = GateRig::new();
        rig.open();
        let cell = Arc::clone(&rig.cell);
        rig.on_list(move || cell.close_turn());
        assert!(rig.esc().is_none());
        assert!(!rig.has_episode());
        assert_eq!(rig.recorder.active(), 0);
        assert_eq!(rig.probe.drops(), 3);
        // 턴이 닫혔다 다시 열려도(그 사이 세대가 바뀌었다) 줄은 없다.
        let cell = Arc::clone(&rig.cell);
        rig.open();
        rig.on_list(move || {
            cell.close_turn();
            cell.open_turn();
        });
        assert!(rig.esc().is_none());
        assert!(!rig.has_episode());
        assert_eq!(rig.recorder.active(), 0);
        rig.check();
    }

    /// 두 구간 사이에 새 입력이 닿거나 켠 기록이 꺼지면(듣는 스레드의 끝) 에피소드 없이 줄만 준다 — 기록 끔 · 스냅숏
    /// 버림은 자물쇠 밖.
    #[test]
    fn new_input_or_a_lost_record_between_the_sections_gives_the_line_only() {
        for (lost, reason) in [(0, "delivered"), (1, "record")] {
            let rig = GateRig::new();
            rig.open();
            if lost == 0 {
                let cell = Arc::clone(&rig.cell);
                rig.on_list(move || cell.deliver());
            } else {
                let recorder = Arc::clone(&rig.recorder);
                rig.on_list(move || drop(recorder.stop(recorder.active())));
            }
            let out = rig.esc().expect("줄만");
            assert_eq!(out.bytes, LINE);
            assert!(
                out.on_written.is_none(),
                "{reason}: 에피소드 없는 줄에 쓰기 확인"
            );
            assert!(!rig.has_episode(), "{reason}");
            assert_eq!(rig.recorder.active(), 0, "{reason}");
            assert_eq!(rig.probe.drops(), 3, "{reason}");
            assert!(rig.logged_reason(reason), "{reason}");
            rig.check();
        }
    }

    /// 두 Esc 가 엇갈리면 둘째는 여는 이를 보고 줄만 받는다(기록 · 명단 · 포트 0). 여는 이의 ② 뒤 셋째 Esc 는 이어
    /// 적는다.
    #[test]
    fn a_second_escape_while_opening_gets_the_line_only() {
        let rig = GateRig::new();
        rig.open();
        let nested: Arc<Mutex<Option<Option<bool>>>> = Arc::default();
        let (cell, slot) = (Arc::clone(&rig.cell), Arc::clone(&nested));
        rig.on_list(move || {
            let out = cell.interrupt(LINE.to_vec());
            *slot.lock().unwrap() = Some(out.map(|o| o.on_written.is_some()));
        });
        rig.esc_with_check();
        assert_eq!(*nested.lock().unwrap(), Some(Some(false)));
        assert_eq!(rig.group.lists.load(Ordering::SeqCst), 1);
        assert_eq!(rig.group.watches.load(Ordering::SeqCst), 1);
        assert_eq!(rig.next_rec(), 1);
        assert!(rig.logged_text("여는 중"));

        rig.esc_with_check();
        rig.episode(|ep| assert_eq!(ep.gen, 0));
        assert_eq!(rig.group.lists.load(Ordering::SeqCst), 1);
        assert_eq!(rig.next_rec(), 1);
        rig.check();
    }

    /// 여는 이 가드 — 스냅숏 도중 패닉해도 `opening` 을 내리고 놓은 뒤 기록을 끄며, 찍다 만 스냅숏도 자물쇠 밖에서
    /// 버린다. 그 뒤의 끊기가 다시 연다.
    #[test]
    fn the_opener_guard_cleans_up_after_a_panic() {
        let rig = GateRig::new();
        rig.open();
        rig.answer(CONHOST, Answer::Panic);
        let caught = panic::catch_unwind(AssertUnwindSafe(|| rig.esc()));
        assert!(caught.is_err());
        assert!(!rig.has_episode());
        assert_eq!(rig.recorder.active(), 0);
        assert_eq!(rig.probe.drops(), 2, "찍다 만 스냅숏(claude · 뿌리)");
        rig.check();

        rig.answer(CONHOST, Answer::Ours(facts(ROOT, 20)));
        rig.esc_with_check();
        assert_eq!(rig.next_rec(), 2);
        rig.check();
    }

    /// 구간 ② 에 산 에피소드가 있으면(여는 이가 하나라 닿지 않는 갈래) 줄만 주고 그 에피소드를 그대로 둔다. 세대가
    /// 바뀌었어도 줄만이다.
    #[test]
    fn the_second_section_never_replaces_a_live_episode_or_a_newer_generation() {
        let t = Instant::now();
        let mut st = GateState {
            open: true,
            ..GateState::default()
        };
        let Begin::Open(mark) = st.begin_interrupt(t) else {
            panic!("여는 이가 돼야 한다");
        };
        st.episode = Some(written_episode(7, t, 3));
        match st.settle_interrupt(mark, 5, 5, SnapState::Failed(SnapCause::List), t) {
            Settle::LineOnly {
                why: Stale::Live, ..
            } => {}
            _ => panic!("산 에피소드를 갈아 끼웠다"),
        }
        assert!(!st.opening);
        assert_eq!(st.episode.as_ref().map(|ep| ep.gen), Some(7));

        let mut st = GateState {
            open: true,
            ..GateState::default()
        };
        let Begin::Open(mark) = st.begin_interrupt(t) else {
            panic!("여는 이가 돼야 한다");
        };
        st.next_gen = 9;
        assert!(matches!(
            st.settle_interrupt(mark, 5, 5, SnapState::Released, t),
            Settle::LineOnly {
                why: Stale::Generation,
                ..
            }
        ));
        assert!(st.episode.is_none());
    }

    // ── 스냅숏 ────────────────────────────────────────────────────────────────────────

    /// 스냅숏이 서지 않으면 그 사유로 에피소드가 열린다(기록은 켜진다) — 명단 실패 · 뿌리 없음 · 뿌리 못 붙듦 · 상한
    /// 넘음(붙들지 않는다) · 뿌리 자식 셋 · 포트 없음(명단을 묻지도 않는다) · 포트 굳음. 붙든 것은 자물쇠 밖에서 버린다.
    #[test]
    fn a_snapshot_that_cannot_stand_fails_with_its_cause() {
        type Setup = fn(&GateRig);
        let cases: [(&str, Setup, SnapCause, usize); 7] = [
            ("list", |rig| rig.listing(None), SnapCause::List, 0),
            (
                "root_missing",
                |rig| rig.listing(Some(vec![CONHOST, CLAUDE])),
                SnapCause::Root,
                0,
            ),
            (
                "root_unpinned",
                |rig| drop(rig.group.answers.lock().unwrap().remove(&ROOT)),
                SnapCause::Root,
                2,
            ),
            (
                "too_many",
                |rig| rig.listing(Some((ROOT..=ROOT + PIN_MAX as u32).collect())),
                SnapCause::TooMany,
                0,
            ),
            (
                "shape",
                |rig| {
                    rig.listing(Some(vec![ROOT, CONHOST, CLAUDE, 103]));
                    rig.answer(103, Answer::Ours(facts(ROOT, 40)));
                },
                SnapCause::Shape,
                4,
            ),
            (
                "port_gone",
                |rig| *rig.group.refuse_watch.lock().unwrap() = Some(GROUP_GONE),
                SnapCause::Port,
                0,
            ),
            (
                "port_latched",
                |rig| rig.recorder.fail_port(),
                SnapCause::Port,
                0,
            ),
        ];
        for (name, setup, cause, drops) in cases {
            let rig = GateRig::new();
            setup(&rig);
            rig.open();
            rig.esc_with_check();
            rig.episode(|ep| {
                assert!(
                    matches!(ep.snap, SnapState::Failed(c) if c == cause),
                    "{name}: {:?}",
                    ep.snap
                );
                assert_eq!(ep.rec, rig.recorder.active(), "{name}");
            });
            match name {
                "too_many" => assert_eq!(rig.group.pins.load(Ordering::SeqCst), 0, "{name}"),
                "port_gone" | "port_latched" => {
                    assert_eq!(rig.group.lists.load(Ordering::SeqCst), 0, "{name}")
                }
                _ => {}
            }
            assert_eq!(
                rig.probe.drops(),
                drops,
                "{name}: 붙든 것은 스냅숏을 세우지 못한 채 버린다"
            );
            rig.check();
        }
    }

    /// 뿌리 밖 멤버를 못 붙들면(거절) 그 멤버만 빠지고 스냅숏은 선다.
    #[test]
    fn an_unpinnable_member_is_left_out_of_the_snapshot() {
        let rig = GateRig::new();
        rig.answer(CONHOST, Answer::Denied);
        rig.open();
        rig.esc_with_check();
        rig.episode(|ep| assert_eq!(ep.snap.members(), 2));
        rig.check();
    }

    // ── 쓰기 확인 · 쓰기 명단 W ───────────────────────────────────────────────────────

    /// 첫 쓰임 — 명단을 자물쇠 밖에서 한 번 찍어 정렬해 두고, 가장 최근 끊기를 쓰인 때로 당기고, 일꾼을 띄운다. 뒤
    /// 끊기의 쓰임은 시각만 당기고 그 명단은 돌려받아 버린다(일꾼은 더 띄우지 않는다).
    #[test]
    fn the_first_write_takes_the_write_list_and_asks_for_one_worker() {
        let rig = GateRig::new();
        rig.open();
        let out = rig.esc_with_check();
        rig.clock.advance(ms(120));
        rig.write(out);
        rig.episode(|ep| {
            assert!(ep.written);
            assert_eq!(ep.w.as_deref(), Some(&[ROOT, CONHOST, CLAUDE][..]));
            assert_eq!(ep.last_mono, rig.clock.at(ms(120)));
            assert_eq!(ep.first_mono, rig.clock.at(ms(0)));
        });
        assert!(rig.worker());
        assert_eq!(rig.clock.spawned(WORKER_THREAD), 1);
        assert_eq!(rig.group.lists.load(Ordering::SeqCst), 2);
        rig.check();

        rig.listing(Some(vec![ROOT, CONHOST, CLAUDE, 300]));
        rig.clock.advance(ms(200));
        let again = rig.esc_with_check();
        rig.write(again);
        rig.episode(|ep| {
            assert_eq!(ep.w.as_deref(), Some(&[ROOT, CONHOST, CLAUDE][..]));
            assert_eq!(ep.last_mono, rig.clock.at(ms(320)));
        });
        assert_eq!(rig.clock.spawned(WORKER_THREAD), 1);
        assert_eq!(rig.group.lists.load(Ordering::SeqCst), 3);
        rig.check();
    }

    /// 뒤 쓰임의 명단은 상태에 들지 않고 돌려받는다 — 받은 쪽이 자물쇠 밖에서 버린다.
    #[test]
    fn a_later_write_hands_its_list_back() {
        let t = Instant::now();
        let mut st = GateState {
            open: true,
            worker: true,
            episode: Some(written_episode(4, t, 1)),
            ..GateState::default()
        };
        let later: Arc<[u32]> = Arc::from(&[ROOT, 7][..]);
        let noted = st.note_written(4, Some(Arc::clone(&later)), t + ms(10));
        assert_eq!(noted.note, WriteNote::Again);
        assert!(noted.back.is_some_and(|back| Arc::ptr_eq(&back, &later)));
        assert!(!noted.spawn);
        let ep = st.episode.as_ref().expect("에피소드");
        assert_eq!(ep.w.as_deref(), Some(&[ROOT][..]));
        assert_eq!(ep.last_mono, t + ms(10));

        let stale = st.note_written(5, Some(later), t + ms(20));
        assert_eq!(stale.note, WriteNote::Stale);
        assert!(stale.back.is_some());
        assert_eq!(st.episode.as_ref().map(|ep| ep.last_mono), Some(t + ms(10)));
    }

    /// 쓰기 명단을 못 찍거나(명단 실패) 무리가 없으면(빈 명단) 쓰임으로 세지 않는다 — 일꾼도 판도 없다. warn 은
    /// 화신마다 한 번. 같은 에피소드의 뒤 쓰임이 명단을 얻으면 그때부터 쓰인 것이다(후보는 그 명단 뒤의 탄생뿐).
    #[test]
    fn an_unlisted_write_never_counts_as_written() {
        let rig = GateRig::new();
        rig.open();
        let out = rig.esc_with_check();
        rig.listing(None);
        rig.write(out);
        rig.episode(|ep| assert!(!ep.written && ep.w.is_none()));
        assert!(!rig.worker());
        assert_eq!(rig.clock.spawned(WORKER_THREAD), 0);
        assert_eq!(rig.count(Level::WARN), 1);
        rig.check();

        rig.listing(Some(Vec::new()));
        let again = rig.esc_with_check();
        rig.write(again);
        rig.episode(|ep| assert!(!ep.written));
        assert_eq!(rig.count(Level::WARN), 1, "warn 은 한 번");
        rig.check();

        rig.listing(Some(vec![ROOT, CONHOST, CLAUDE]));
        let listed = rig.esc_with_check();
        rig.write(listed);
        rig.episode(|ep| assert!(ep.written));
        assert!(rig.worker());
        rig.check();
    }

    /// 끝난 에피소드 · 갈아 끼운 세대의 쓰임은 버린다 — 새 에피소드를 쓰인 것으로 만들지 않는다.
    #[test]
    fn a_write_for_another_episode_is_dropped() {
        let rig = GateRig::new();
        rig.open();
        let first = rig.esc_with_check();
        rig.cell.deliver();
        let second = rig.esc_with_check();
        rig.write(first);
        rig.episode(|ep| {
            assert_eq!(ep.gen, 1);
            assert!(!ep.written);
        });
        assert!(!rig.worker());
        rig.write(second);
        rig.episode(|ep| assert!(ep.written));

        let third = rig.esc_with_check();
        rig.cell.close_turn();
        rig.write(third);
        assert!(!rig.has_episode());
        rig.check();
    }

    /// 후보는 쓰기 명단 W 뒤에 가입한 탄생뿐이다 — 알림이 언제 쌓였는지에 기대지 않는다.
    ///
    /// - X: 쓰기 전에 가입해 산다 → W 에 든다(포트가 그 가입을 W 뒤에 늦게 내줘도) → 후보 아님.
    /// - Z: 쓰기와 W 사이에 가입 → W 에 든다 → 빠짐(놓침 창).
    /// - P′: W 에 있던 번호를 쓰인 뒤 탄생이 재사용 → 번호로만 비교해 빠짐(놓침).
    /// - Q: 쓰기 전에 가입해 W 전에 끝났다 → W 밖이지만 끝나 고리 전용.
    /// - Y: W 뒤 가입 → 후보.
    #[test]
    fn only_births_after_the_write_list_are_candidates() {
        const X: u32 = 200;
        const Z: u32 = 201;
        const P: u32 = 202;
        const Q: u32 = 203;
        const Y: u32 = 204;
        let rig = GateRig::new();
        rig.open();
        let out = rig.esc_with_check();
        let rec = rig.recorder.active();
        rig.admit(rec, Q, facts(CLAUDE, 40), EXITED);
        rig.listing(Some(vec![ROOT, CONHOST, CLAUDE, X, P]));
        let (group, recorder, probe) = (
            Arc::clone(&rig.group),
            Arc::clone(&rig.recorder),
            Arc::clone(&rig.probe),
        );
        // Z 는 줄이 쓰인 뒤 · 명단이 찍히기 전에 태어나 곧바로 기록된다.
        rig.on_list(move || {
            if let Some(listing) = group.listing.lock().unwrap().as_mut() {
                listing.push(Z);
            }
            admit_into(&recorder, &probe, rec, Z, facts(CLAUDE, 60), ALIVE);
        });
        rig.write(out);
        for (pid, create) in [(X, 50), (P, 990), (Y, 70)] {
            rig.admit(rec, pid, facts(CLAUDE, create), ALIVE);
        }
        rig.clock.advance(N);

        let step = rig.cell.lock().next_step(rig.clock.now(), false);
        let Step::Pass(ticket) = step else {
            panic!("판이 서야 한다");
        };
        assert_eq!(&*ticket.w, &[ROOT, CONHOST, CLAUDE, X, Z, P][..]);
        let snapshot =
            precheck(&ticket, rig.recorder.active(), false, false).expect("판의 순서 1–3");
        let births = rig.recorder.copy(ticket.rec);
        let snapshot_links = snapshot.links();
        let birth_links: Vec<Link<'_>> = births.iter().map(|b| b.link()).collect();
        let exited = |l: &Link<'_>| -> io::Result<bool> {
            births
                .iter()
                .find(|b| b.pid == l.pid && b.facts.create == l.facts.create)
                .map_or(Ok(false), |b| b.pin.exited())
        };
        let view = Chain {
            root_pid: ROOT,
            snapshot: &snapshot_links,
            births: &birth_links,
            exited: &exited,
        };
        let picked: Vec<u32> = candidates(&view, &ticket.w).iter().map(|l| l.pid).collect();
        assert_eq!(picked, [Y]);

        drop(view);
        drop(birth_links);
        drop(snapshot_links);
        drop(births);
        rig.cell.end_pass(&rig.cleaner, ticket.gen, false);
        drop(ticket);
        rig.check();
    }

    // ── 일꾼 · 판 끝 ──────────────────────────────────────────────────────────────────

    /// 일꾼은 가장 최근 쓰임 + N 까지 자고 판을 한 번 돈다 — 판 끝이 `Done` 으로 매듭짓고 스냅숏 · 기록을 자물쇠
    /// 밖에서 놓은 뒤 일꾼이 끝난다.
    #[test]
    fn the_worker_sleeps_until_n_after_the_last_write_then_ends_the_pass_once() {
        let rig = GateRig::new();
        rig.open();
        let out = rig.esc_with_check();
        rig.clock.advance(ms(100));
        rig.write(out);
        rig.clock.advance(ms(400));
        rig.run_workers();

        assert_eq!(rig.clock.sleeps(), [ms(2_600)]);
        rig.episode(|ep| {
            assert_eq!(ep.phase, Phase::Done);
            assert!(matches!(ep.snap, SnapState::Released));
            assert_eq!(ep.rec, 0);
        });
        assert!(!rig.worker());
        assert_eq!(rig.recorder.active(), 0);
        assert_eq!(rig.probe.drops(), 3);
        rig.check();

        // 끝난 에피소드 뒤의 끊기는 새 에피소드다.
        rig.esc_with_check();
        rig.episode(|ep| assert_eq!((ep.gen, ep.phase), (1, Phase::Waiting)));
        rig.check();
    }

    /// 뒤 끊기의 쓰임이 판을 미룬다 — N 은 가장 최근에 쓰인 끊기부터다.
    #[test]
    fn a_later_write_moves_the_pass_back() {
        let rig = GateRig::new();
        rig.open();
        let out = rig.esc_with_check();
        rig.write(out);
        rig.clock.advance(ms(2_000));
        let later = rig.esc_with_check();
        rig.write(later);
        assert!(rig.turn());
        assert_eq!(rig.clock.sleeps(), [N]);
        assert!(rig.turn());
        rig.episode(|ep| assert_eq!(ep.phase, Phase::Done));
        assert!(!rig.turn());
        rig.check();
    }

    /// 판 도중 끊기는 에피소드를 `Waiting` 으로 돌리고, 그 판의 끝은 에피소드를 쥔 채 둔다(완료된 판이 아니다).
    #[test]
    fn a_pass_superseded_by_a_new_interrupt_keeps_the_episode() {
        let t = Instant::now();
        let mut st = GateState {
            open: true,
            worker: true,
            episode: Some(written_episode(2, t, 6)),
            ..GateState::default()
        };
        let Step::Pass(ticket) = st.next_step(t + N, false) else {
            panic!("판");
        };
        assert!(matches!(
            st.begin_interrupt(t + N + ms(5)),
            Begin::Continue { gen: 2 }
        ));
        let released = st.finish_pass(ticket.gen, true, t + N + ms(9));
        assert_eq!(released.rec, 0);
        let ep = st.episode.as_ref().expect("에피소드");
        assert_eq!((ep.phase, ep.rec), (Phase::Waiting, 6));
        assert!(ep.killed_any);
        // 다시 N 을 기다린다.
        assert!(matches!(st.next_step(t + N + ms(10), false), Step::Sleep(d) if d == N - ms(5)));
    }

    /// 무엇이든 끝낸 판은 `Spent` — N 뒤에도 같은 에피소드면 한 번 warn(`StillHung`)하고 `Done`, 일꾼이 끝난다.
    /// 아무것도 못 끝낸 판은 곧바로 `Done`.
    #[test]
    fn a_pass_that_killed_waits_n_then_reports_once_and_ends() {
        let t = Instant::now();
        let mut st = GateState {
            open: true,
            worker: true,
            episode: Some(written_episode(1, t, 4)),
            ..GateState::default()
        };
        let Step::Pass(ticket) = st.next_step(t + N, false) else {
            panic!("판");
        };
        let at = t + N + ms(300);
        let released = st.finish_pass(ticket.gen, true, at);
        assert_eq!(released.rec, 4);
        assert!(st.episode.as_ref().is_some_and(|ep| ep.killed_any));
        assert_eq!(
            st.episode.as_ref().map(|ep| ep.phase),
            Some(Phase::Spent { at })
        );
        assert!(
            matches!(st.next_step(at + ms(1_000), false), Step::Sleep(d) if d == N - ms(1_000))
        );
        assert!(matches!(
            st.next_step(at + N, false),
            Step::StillHung { waited } if waited == at + N - t
        ));
        assert_eq!(st.episode.as_ref().map(|ep| ep.phase), Some(Phase::Done));
        assert!(matches!(
            st.next_step(at + N, false),
            Step::Exit { retired: None, .. }
        ));
        assert!(!st.worker);

        let mut st = GateState {
            open: true,
            worker: true,
            episode: Some(written_episode(1, t, 4)),
            ..GateState::default()
        };
        let Step::Pass(ticket) = st.next_step(t + N, false) else {
            panic!("판");
        };
        let _ = st.finish_pass(ticket.gen, false, t + N);
        assert!(st.episode.as_ref().is_some_and(|ep| !ep.killed_any));
        assert_eq!(st.episode.as_ref().map(|ep| ep.phase), Some(Phase::Done));
    }

    /// 끝내기 확정의 재확인은 같은 세대 · 쓰인 `Cleaning` · N 이 지난 때에만 선다. 표식이면 `Retiring`. 끝냄 표시는
    /// 같은 세대에만.
    #[test]
    fn the_commit_check_goes_only_for_the_same_due_cleaning_episode() {
        let t = Instant::now();
        let mut st = GateState {
            open: true,
            worker: true,
            episode: Some(written_episode(3, t, 1)),
            ..GateState::default()
        };
        assert_eq!(
            st.commit_check(3, t + N, false),
            Commit::Superseded,
            "Waiting"
        );
        assert!(matches!(st.next_step(t + N, false), Step::Pass(_)));
        assert_eq!(st.commit_check(3, t + N, false), Commit::Go);
        assert_eq!(st.commit_check(3, t + N - ms(1), false), Commit::Superseded);
        assert_eq!(st.commit_check(4, t + N, false), Commit::Superseded);
        assert_eq!(st.commit_check(3, t + N, true), Commit::Retiring);
        st.note_kill(4);
        assert!(!st.episode.as_ref().expect("에피소드").killed_any);
        st.note_kill(3);
        assert!(st.episode.as_ref().expect("에피소드").killed_any);
        let _ = st.close_turn();
        assert_eq!(st.commit_check(3, t + N, false), Commit::Superseded);
    }

    /// 물러남 표시가 서면 일꾼이 에피소드를 치우고(스냅숏 · 기록을 자물쇠 밖에서 놓는다) 끝난다.
    #[test]
    fn retiring_clears_the_episode_and_ends_the_worker() {
        let rig = GateRig::new();
        rig.open();
        let out = rig.esc_with_check();
        rig.write(out);
        rig.group.retiring.store(true, Ordering::Release);
        assert!(!rig.turn());
        assert!(!rig.has_episode());
        assert!(!rig.worker());
        assert_eq!(rig.recorder.active(), 0);
        assert_eq!(rig.probe.drops(), 3);
        assert!(rig.logged_text("retiring"));
        rig.check();
    }

    /// 일꾼을 못 띄우면 에피소드를 `Done` 으로 내리고 기록 · 스냅숏을 놓는다(warn). 그 뒤 끊기는 새 에피소드를 연다.
    #[test]
    fn a_worker_that_fails_to_spawn_drops_the_episode() {
        let rig = GateRig::new();
        rig.clock.refuse_worker.store(true, Ordering::SeqCst);
        rig.open();
        let out = rig.esc_with_check();
        rig.write(out);
        rig.episode(|ep| {
            assert_eq!(ep.phase, Phase::Done);
            assert!(matches!(ep.snap, SnapState::Released));
            assert_eq!(ep.rec, 0);
        });
        assert!(!rig.worker());
        assert_eq!(rig.recorder.active(), 0);
        assert_eq!(rig.probe.drops(), 3);
        assert_eq!(rig.count(Level::WARN), 1);
        rig.check();

        rig.esc_with_check();
        rig.episode(|ep| assert_eq!((ep.gen, ep.phase), (1, Phase::Waiting)));
        rig.check();
    }

    /// 일꾼 몸통이 패닉하면 가드가 에피소드를 `Done` 으로 내리고 쥔 것을 놓는다(error).
    #[test]
    fn a_panicking_worker_drops_the_episode_through_its_guard() {
        let rig = GateRig::new();
        rig.open();
        let out = rig.esc_with_check();
        rig.write(out);
        rig.clock.panic_in_sleep.store(true, Ordering::SeqCst);
        let bodies = rig.clock.take(WORKER_THREAD);
        assert_eq!(bodies.len(), 1);
        for body in bodies {
            let ran = panic::catch_unwind(AssertUnwindSafe(|| rig.logged(body)));
            assert!(ran.is_err());
        }
        rig.episode(|ep| {
            assert_eq!(ep.phase, Phase::Done);
            assert_eq!(ep.rec, 0);
        });
        assert!(!rig.worker());
        assert_eq!(rig.recorder.active(), 0);
        assert_eq!(rig.probe.drops(), 3);
        assert_eq!(rig.count(Level::ERROR), 1);
        rig.check();
    }

    /// 쓰이지 않은 에피소드에서는 일꾼이 그냥 끝나고(에피소드는 그대로) 그 에피소드가 쓰이면 다시 뜬다. 턴 끝 · 새
    /// 입력은 에피소드를 버리고 기록 · 스냅숏을 자물쇠 밖에서 놓는다 — 자는 일꾼은 깨서 조용히 끝난다.
    #[test]
    fn decoder_hooks_drop_the_episode_and_the_worker_follows() {
        let rig = GateRig::new();
        rig.open();
        let out = rig.esc_with_check();
        rig.write(out);
        rig.logged(|| rig.cell.deliver());
        assert!(!rig.has_episode());
        assert_eq!(rig.state(|st| (st.open, st.delivers)), (true, 1));
        assert!(rig.logged_text("delivered"));
        assert_eq!(rig.recorder.active(), 0);
        assert_eq!(rig.probe.drops(), 3);
        rig.check();

        rig.esc_with_check();
        assert!(!rig.turn(), "쓰이지 않은 에피소드에서 일꾼이 남았다");
        rig.episode(|ep| assert!(ep.is_live() && !ep.written));
        assert_ne!(rig.recorder.active(), 0);
        rig.check();
        let next = rig.esc_with_check();
        rig.write(next);
        assert!(rig.worker());
        assert_eq!(rig.clock.spawned(WORKER_THREAD), 2);

        rig.cell.close_turn();
        assert_eq!(rig.state(|st| (st.open, st.closes)), (false, 1));
        assert!(!rig.has_episode());
        assert_eq!(rig.recorder.active(), 0);
        assert!(!rig.turn());
        rig.check();

        rig.open();
        rig.open();
        assert_eq!(
            rig.state(|st| (st.open, st.closes, st.delivers)),
            (true, 1, 1)
        );
    }

    // ── 판의 순서 1–3 ─────────────────────────────────────────────────────────────────

    /// 판은 표식 · 스냅숏 · 기록 · 뿌리를 먼저 본다 — 듣는 스레드가 끝나 기록이 꺼졌으면(`active ≠ rec`)
    /// `snapshot_failed(port)` 다.
    #[test]
    fn the_pass_checks_retiring_snapshot_record_and_root_first() {
        let rig = GateRig::new();
        rig.open();
        let out = rig.esc_with_check();
        rig.write(out);
        rig.clock.advance(N);
        let step = rig.cell.lock().next_step(rig.clock.now(), false);
        let Step::Pass(ticket) = step else {
            panic!("판");
        };
        let active = rig.recorder.active();
        assert!(precheck(&ticket, active, false, false).is_ok());
        assert!(matches!(
            precheck(&ticket, active, true, false),
            Err(NotReady::Retiring)
        ));

        rig.group.exit_of(ROOT).store(EXITED, Ordering::SeqCst);
        assert!(matches!(
            precheck(&ticket, active, false, false),
            Err(NotReady::RootGone)
        ));
        rig.group.exit_of(ROOT).store(FAILING, Ordering::SeqCst);
        assert!(matches!(
            precheck(&ticket, active, false, false),
            Err(NotReady::RootUnknown(_))
        ));

        drop(rig.recorder.stop(active));
        assert!(matches!(
            precheck(&ticket, rig.recorder.active(), false, false),
            Err(NotReady::Snapshot(SnapCause::Port))
        ));
        rig.cell.end_pass(&rig.cleaner, ticket.gen, false);
        drop(ticket);
        rig.check();

        let failed = GateRig::new();
        failed.listing(None);
        failed.open();
        let out = failed.esc_with_check();
        failed.listing(Some(vec![ROOT]));
        failed.write(out);
        failed.clock.advance(N);
        let step = failed.cell.lock().next_step(failed.clock.now(), false);
        let Step::Pass(ticket) = step else {
            panic!("판");
        };
        assert!(matches!(
            precheck(&ticket, failed.recorder.active(), false, false),
            Err(NotReady::Snapshot(SnapCause::List))
        ));
    }

    /// 포트가 실패로 굳었으면 스냅숏 · 기록이 멀쩡해도 판은 서지 않는다 — `no_new_member` 가 아니라
    /// `snapshot_failed(port)` 다. 표식이 먼저다.
    #[test]
    fn a_failed_port_stops_the_pass_as_a_port_snapshot_failure() {
        let rig = GateRig::new();
        rig.open();
        let out = rig.esc_with_check();
        rig.write(out);
        rig.clock.advance(N);
        let step = rig.cell.lock().next_step(rig.clock.now(), false);
        let Step::Pass(ticket) = step else {
            panic!("판");
        };
        let active = rig.recorder.active();
        assert!(precheck(&ticket, active, false, rig.recorder.port_failed()).is_ok());
        rig.recorder.fail_port();
        assert!(matches!(
            precheck(&ticket, active, false, rig.recorder.port_failed()),
            Err(NotReady::Snapshot(SnapCause::Port))
        ));
        assert!(matches!(
            precheck(&ticket, active, true, rig.recorder.port_failed()),
            Err(NotReady::Retiring)
        ));
        rig.cell.end_pass(&rig.cleaner, ticket.gen, false);
        drop(ticket);
        rig.check();
    }

    // ── 판의 차례 ─────────────────────────────────────────────────────────────────────

    fn pids_rounds(report: &PassReport) -> Vec<(u32, u32)> {
        report.terminated.iter().map(|k| (k.pid, k.round)).collect()
    }

    /// 잰 모양 셋은 한 차례에 주인 하나를 끝내고 확인한 뒤, 새로 끝낼 것이 없는 둘째 차례에서 멈춘다 — 판은 `Spent`
    /// 로 매듭짓고 정리함 warn 하나만 남는다.
    #[test]
    fn measured_shapes_end_their_owner_in_one_round() {
        for (name, world, owner, chain) in [
            ("p1", p1(), 38108, vec![38108, 36088, 22700, 26208]),
            (
                "p3t3",
                p3t3(),
                23604,
                vec![23604, 26732, 27108, 10400, 33144],
            ),
            ("pilot", pilot(), 3256, vec![3256, 37552, 37948, 25096]),
        ] {
            let rig = GateRig::staged(&world.staged(), &[]);
            let report = rig.pass();
            assert_eq!(pids_rounds(&report), [(owner, 1)], "{name}");
            assert_eq!(report.terminated[0].chain, chain, "{name}");
            assert!(report.stop.is_none(), "{name}: {:?}", report.stop);
            assert_eq!((report.rounds, report.confirmed), (1, 1), "{name}");
            assert_eq!(rig.terminates(), [owner], "{name}");
            assert!(
                rig.probe.waits().is_empty(),
                "{name}: 끝난 것은 기다리지 않는다"
            );
            assert!(
                matches!(rig.episode(|ep| ep.phase), Phase::Spent { .. }),
                "{name}"
            );
            assert!(rig.cleaned_line().is_some(), "{name}");
            assert_eq!(rig.count(Level::WARN), 1, "{name}");
            rig.assert_lock_sections();
        }
    }

    /// p3bg — 1 차례에 주인 32228 을 끝내고 확인하면 2 차례에 그것이 끝난 고리로 보여 fork 24776 이 후보가 된다. 3
    /// 차례에는 새로 끝낼 것이 없다. 판 때 산 다른 탄생(taskkill · conhost · sleep · cat)은 건드리지 않는다.
    #[test]
    fn p3bg_ends_the_owner_then_its_fork_in_the_next_round() {
        let rig = GateRig::staged(&p3bg().staged(), &[]);
        let report = rig.pass();
        assert_eq!(pids_rounds(&report), [(32228, 1), (24776, 2)]);
        assert_eq!(report.terminated[1].chain, [24776, 32228, 25028, 35688]);
        assert!(report.stop.is_none(), "{:?}", report.stop);
        assert_eq!((report.rounds, report.confirmed), (2, 2));
        assert_eq!(rig.terminates(), [32228, 24776]);
        assert_eq!(
            (report.excluded.parent_alive, report.excluded.not_hook_copy),
            (4, 2)
        );
        rig.assert_lock_sections();
    }

    /// 한 차례의 `Cleanup` 은 `seq` 오름차순으로 끝낸다 — 생성 순서가 아니라 기록에 든 순서다.
    #[test]
    fn a_round_ends_its_cleanups_in_seq_order() {
        let rig = GateRig::staged(&p1_two_owners().staged(), &[]);
        assert_eq!(pids_rounds(&rig.pass()), [(38108, 1), (38200, 1)]);
        rig.assert_lock_sections();

        let mut w = p1_two_owners().staged();
        let at = |pid: u32| {
            w.births
                .iter()
                .position(|(p, _, _)| *p == pid)
                .expect("탄생")
        };
        let (a, b) = (at(38108), at(38200));
        w.births.swap(a, b);
        let rig = GateRig::staged(&w, &[]);
        assert_eq!(pids_rounds(&rig.pass()), [(38200, 1), (38108, 1)]);
        assert_eq!(rig.terminates(), [38200, 38108]);
        rig.assert_lock_sections();
    }

    /// 한 차례에 끝낸 것들의 확인은 마감 하나(200 ms)를 나눠 쓴다 — 앞의 것이 마감을 다 쓰면 뒤의 것은 기다리지
    /// 않는다. 안 끝난 것은 확인되지 않은 채 남고 다음 차례에 거듭 끝내지 않는다.
    #[test]
    fn a_round_shares_one_confirm_deadline() {
        let rig = GateRig::staged(
            &p1_two_owners().staged(),
            &[(38108, Kill::Linger), (38200, Kill::Linger)],
        );
        let report = rig.pass();
        let waits = rig.probe.waits();
        assert_eq!(waits, [KILL_CONFIRM, Duration::ZERO]);
        assert!(waits.iter().sum::<Duration>() <= KILL_CONFIRM);
        assert_eq!(pids_rounds(&report), [(38108, 1), (38200, 1)]);
        assert_eq!((report.rounds, report.confirmed), (1, 0));
        assert_eq!(rig.terminates(), [38108, 38200]);
        assert_eq!(rig.cleaned_line().expect("정리함")["unconfirmed"], "2");
        rig.assert_lock_sections();
    }

    /// 마감 안에 안 끝난 주인은 다음 차례에 산 것으로 보여 그 fork 는 빠진다(놓침).
    #[test]
    fn an_owner_not_gone_by_the_deadline_keeps_its_fork_out() {
        let rig = GateRig::staged(&p3bg().staged(), &[(32228, Kill::Linger)]);
        let report = rig.pass();
        assert_eq!(pids_rounds(&report), [(32228, 1)]);
        assert_eq!(rig.terminates(), [32228]);
        assert_eq!(rig.probe.waits(), [KILL_CONFIRM]);
        assert_eq!(report.excluded.parent_alive, 5, "24776 의 부모가 산다");
        assert!(report.stop.is_none(), "{:?}", report.stop);
        assert_eq!(rig.cleaned_line().expect("정리함")["unconfirmed"], "1");
        rig.assert_lock_sections();
    }

    /// 네 차례가 필요한 모양은 세 차례까지만 끝내고 warn `rounds_exhausted` — 넷째 후보는 남긴다.
    #[test]
    fn a_fourth_round_is_never_run() {
        let rig = GateRig::staged(&p3bg_deeper().staged(), &[]);
        let report = rig.pass();
        assert_eq!(pids_rounds(&report), [(32228, 1), (24776, 2), (24800, 3)]);
        assert!(matches!(report.stop, Some(PassStop::RoundsExhausted)));
        assert_eq!(report.rounds, PASS_ROUNDS);
        assert!(!rig.terminates().contains(&24810));
        let (level, fields) = rig
            .line_with_reason("rounds_exhausted")
            .expect("rounds_exhausted");
        assert_eq!((level, fields["rounds"].as_str()), (Level::WARN, "3"));
        assert_eq!(rig.count(Level::WARN), 2, "정리함 + rounds_exhausted");
        rig.assert_lock_sections();
    }

    #[derive(Debug, Clone, Copy)]
    enum Cut {
        TurnEnd,
        Escape,
        Input,
        Retire,
    }

    /// 고르기와 확정 사이(둘째 후보 앞 재확인)에 턴 끝 · 새 끊기 · 새 입력 · 물러남이 오면 확정이 거절되고, 남은 후보와
    /// 다음 차례를 버린다(debug). 앞서 끝낸 것은 정리함에 남는다.
    #[test]
    fn a_refused_commit_drops_the_rest_of_the_pass() {
        for cut in [Cut::TurnEnd, Cut::Escape, Cut::Input, Cut::Retire] {
            let rig = GateRig::staged(&p1_two_owners().staged(), &[]);
            let (cell, retiring) = (Arc::clone(&rig.cell), Arc::clone(&rig.group.retiring));
            rig.on_root_check(3, move || match cut {
                Cut::TurnEnd => cell.close_turn(),
                Cut::Escape => drop(cell.interrupt(LINE.to_vec())),
                Cut::Input => cell.deliver(),
                Cut::Retire => retiring.store(true, Ordering::Release),
            });
            let report = rig.pass();
            assert_eq!(pids_rounds(&report), [(38108, 1)], "{cut:?}");
            assert_eq!(rig.terminates(), [38108], "{cut:?}");
            let (stopped, reason) = match cut {
                Cut::Retire => (
                    matches!(report.stop, Some(PassStop::NotReady(NotReady::Retiring))),
                    "retiring",
                ),
                _ => (
                    matches!(report.stop, Some(PassStop::Superseded)),
                    "superseded",
                ),
            };
            assert!(stopped, "{cut:?}: {:?}", report.stop);
            let (level, _) = rig.line_with_reason(reason).expect(reason);
            assert_eq!(level, Level::DEBUG, "{cut:?}");
            assert!(rig.cleaned_line().is_some(), "{cut:?}");
            assert_eq!(rig.count(Level::WARN), 1, "{cut:?}");
            rig.assert_lock_sections();
        }

        // 다음 차례도 버린다 — p3bg 의 2 차례 fork 앞에서 턴이 끝난다.
        let rig = GateRig::staged(&p3bg().staged(), &[]);
        let cell = Arc::clone(&rig.cell);
        rig.on_root_check(3, move || cell.close_turn());
        let report = rig.pass();
        assert_eq!(pids_rounds(&report), [(32228, 1)]);
        assert!(matches!(report.stop, Some(PassStop::Superseded)));
        assert!(!rig.has_episode());
        rig.assert_lock_sections();
    }

    /// 끝내기 앞 재확인 — 뿌리가 끝났으면(debug `root_gone`) · 모르면(warn `root_unknown`) · 물러나면(debug
    /// `retiring`) 아무것도 끝내지 않고 판을 멈춘다.
    #[test]
    fn the_recheck_before_a_kill_stops_the_pass() {
        for (state, reason, level) in [
            (EXITED, "root_gone", Level::DEBUG),
            (FAILING, "root_unknown", Level::WARN),
        ] {
            let rig = GateRig::staged(&p1().staged(), &[]);
            let root = rig.group.exit_of(rig.root);
            rig.on_root_check(2, move || root.store(state, Ordering::SeqCst));
            let report = rig.pass();
            assert!(report.terminated.is_empty(), "{reason}");
            assert!(rig.terminates().is_empty(), "{reason}");
            assert_eq!(
                rig.line_with_reason(reason).map(|(l, _)| l),
                Some(level),
                "{reason}"
            );
            assert_eq!(
                rig.count(Level::WARN),
                usize::from(level == Level::WARN),
                "{reason}"
            );
            rig.episode(|ep| assert_eq!(ep.phase, Phase::Done));
            rig.assert_lock_sections();
        }

        // 판의 순서 3(첫 조회) 뒤에 선 표식은 끝내기 앞에서 본다.
        let rig = GateRig::staged(&p1().staged(), &[]);
        let retiring = Arc::clone(&rig.group.retiring);
        rig.on_root_check(1, move || retiring.store(true, Ordering::Release));
        let report = rig.pass();
        assert!(matches!(
            report.stop,
            Some(PassStop::NotReady(NotReady::Retiring))
        ));
        assert!(rig.terminates().is_empty());
        assert_eq!(
            rig.line_with_reason("retiring").map(|(l, _)| l),
            Some(Level::DEBUG)
        );
        assert_eq!(rig.count(Level::WARN), 0);
        rig.assert_lock_sections();
    }

    /// 끝내기 오류는 하나씩 warn 하고, 고른 것을 하나도 못 끝낸 판은 warn `all_failed` — 그 사이 스스로 끝난 것
    /// (`Gone`)도 끝낸 것이 아니다. 실패한 것을 한 판에서 거듭 끝내지 않는다.
    #[test]
    fn a_pass_that_ends_nothing_it_picked_is_all_failed() {
        let rig = GateRig::staged(&p1().staged(), &[(38108, Kill::Refuse)]);
        let report = rig.pass();
        assert!(report.terminated.is_empty());
        assert_eq!((report.cleanups, report.failed, report.gone), (1, 1, 0));
        assert_eq!(rig.terminates(), [38108]);
        assert!(rig.logged_text("하나를 끝내지 못했다"));
        let (level, fields) = rig.line_with_reason("all_failed").expect("all_failed");
        assert_eq!((level, fields["failed"].as_str()), (Level::WARN, "1"));
        assert_eq!(rig.count(Level::WARN), 2);
        rig.episode(|ep| assert_eq!(ep.phase, Phase::Done));
        rig.assert_lock_sections();

        let rig = GateRig::staged(&p1().staged(), &[(38108, Kill::Refuse)]);
        let owner = rig.group.exit_of(38108);
        rig.on_root_check(2, move || owner.store(EXITED, Ordering::SeqCst));
        let report = rig.pass();
        assert_eq!((report.cleanups, report.failed, report.gone), (1, 0, 1));
        let (_, fields) = rig.line_with_reason("all_failed").expect("all_failed");
        assert_eq!(fields["gone"], "1");
        assert_eq!(rig.count(Level::WARN), 1);
        rig.assert_lock_sections();
    }

    /// 못 끝냄 — 첫 차례에 후보가 없으면 warn `no_new_member`(`births` · `full`), 후보가 모두 규칙에서 빠지면 warn
    /// `parent_rule`(규칙별 수), 스냅숏이 서지 않았으면 warn `snapshot_failed`(`cause`).
    #[test]
    fn a_pass_without_a_cleanup_says_why() {
        let rig = GateRig::new();
        rig.open();
        let out = rig.esc_with_check();
        rig.write(out);
        rig.admit(rig.recorder.active(), 300, facts(CLAUDE, 40), EXITED);
        rig.clock.advance(N);
        let report = rig.pass();
        assert_eq!((report.candidates, report.births), (0, 1));
        let (level, fields) = rig
            .line_with_reason("no_new_member")
            .expect("no_new_member");
        assert_eq!(level, Level::WARN);
        assert_eq!(
            (fields["births"].as_str(), fields["full"].as_str()),
            ("1", "false")
        );
        assert_eq!(rig.count(Level::WARN), 1);
        rig.assert_lock_sections();

        let rig = GateRig::staged(&p1_chain().staged(), &[]);
        let report = rig.pass();
        assert_eq!((report.candidates, report.cleanups), (1, 0));
        let (level, fields) = rig.line_with_reason("parent_rule").expect("parent_rule");
        assert_eq!(level, Level::WARN);
        assert_eq!(
            (
                fields["no_taskkill"].as_str(),
                fields["parent_alive"].as_str()
            ),
            ("1", "0")
        );
        assert!(rig.terminates().is_empty());
        assert_eq!(rig.count(Level::WARN), 1);
        rig.assert_lock_sections();

        let rig = GateRig::new();
        rig.listing(None);
        rig.open();
        let out = rig.esc_with_check();
        rig.listing(Some(vec![ROOT]));
        rig.write(out);
        rig.clock.advance(N);
        rig.pass();
        let (level, fields) = rig
            .line_with_reason("snapshot_failed")
            .expect("snapshot_failed");
        assert_eq!((level, fields["cause"].as_str()), (Level::WARN, "list"));
        rig.assert_lock_sections();
    }

    /// 정리함 warn 은 TRD §3-8 의 칸을 모두 싣는다.
    #[test]
    fn the_cleaned_line_carries_every_field() {
        let rig = GateRig::staged(&p3bg().staged(), &[]);
        rig.probe.set_terminate_cost(Duration::from_micros(150));
        rig.pass();
        let line = rig.cleaned_line().expect("정리함");
        for key in [
            "agent",
            "root_pid",
            "waited_ms",
            "terminated",
            "rounds",
            "terminate_max_us",
            "unconfirmed",
            "births",
            "parent_alive",
            "parent_unknown",
            "not_hook_copy",
            "ancestor_alive",
            "chain_cut",
            "not_hook",
            "no_taskkill",
            "unknown",
        ] {
            assert!(line.contains_key(key), "{key} 칸이 없다: {line:?}");
        }
        assert_eq!(line["agent"], AgentId::from_u128(0x7440).to_string());
        assert_eq!(line["root_pid"], "37152");
        assert_eq!(line["waited_ms"], "3000");
        assert_eq!(line["rounds"], "2");
        assert_eq!(line["terminate_max_us"], "150");
        assert_eq!(line["unconfirmed"], "0");
        assert_eq!(line["births"], "14");
        assert_eq!(
            line["terminated"],
            "[pid=32228 ppid=25028 chain=[32228, 25028, 35688] round=1, \
             pid=24776 ppid=32228 chain=[24776, 32228, 25028, 35688] round=2]"
        );
        assert_eq!(rig.count(Level::WARN), 1);
        rig.assert_lock_sections();
    }

    /// 일꾼으로 돌면 — 끝낸 판 뒤 N 을 자고, 그때도 같은 에피소드면 한 번 warn 하고 끝난다.
    #[test]
    fn the_worker_reports_once_more_n_after_a_pass_that_killed() {
        let rig = GateRig::staged(&pilot().staged(), &[]);
        assert!(rig.turn());
        assert_eq!(rig.terminates(), [3256]);
        assert!(matches!(rig.episode(|ep| ep.phase), Phase::Spent { .. }));
        assert!(rig.turn());
        assert_eq!(rig.clock.sleeps(), [N]);
        assert!(rig.turn());
        assert!(rig.logged_text("더 하지 않는다"));
        assert!(!rig.turn());
        assert_eq!(rig.count(Level::WARN), 2);
        assert_eq!(rig.recorder.active(), 0);
        rig.assert_lock_sections();
    }

    /// Debug 는 자물쇠를 기다리지 않는다.
    #[test]
    fn the_debug_view_never_waits_for_the_lock() {
        let rig = GateRig::new();
        rig.open();
        assert!(format!("{:?}", rig.cell).contains("open: true"));
        let held = rig.cell.lock();
        let text = format!("{:?}", rig.cell);
        drop(held);
        assert!(text.contains("<locked>"), "{text}");
    }

    // ── 상태기계 · 불변식 ─────────────────────────────────────────────────────────────

    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: u64) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x % n
        }
    }

    /// 임의 전이열(끊기 · 두 구간 사이 사건 · 쓰기 확인 · 버린 확인 · decoder 셋 · 시각 · 일꾼 걸음 · 명단 실패 · 듣는
    /// 스레드의 끝 · 일꾼 기동 실패 · 물러남) 뒤마다 TRD §3-2 불변식과 자물쇠 규칙이 선다. 결정적이다(고정 씨앗).
    #[test]
    fn random_transition_sequences_keep_every_invariant() {
        let (mut passes, mut opened, mut line_only, mut no_line) = (0, 0, 0, 0);
        for seed in 1..=200u64 {
            let rig = GateRig::new();
            rig.clock.keep_workers.store(false, Ordering::SeqCst);
            let pending: Arc<Mutex<Vec<OnWritten>>> = Arc::default();
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let take = |out: Option<InterruptOut>, line_only: &mut u32, no_line: &mut u32| match out
            {
                Some(InterruptOut {
                    on_written: Some(cb),
                    ..
                }) => pending.lock().unwrap().push(cb),
                Some(_) => *line_only += 1,
                None => *no_line += 1,
            };
            for _ in 0..200 {
                match rng.below(32) {
                    0..=5 => take(rig.esc(), &mut line_only, &mut no_line),
                    6 | 7 => {
                        let (cell, recorder, pend) = (
                            Arc::clone(&rig.cell),
                            Arc::clone(&rig.recorder),
                            Arc::clone(&pending),
                        );
                        let inner = rng.below(6);
                        rig.on_list(move || match inner {
                            0 => cell.close_turn(),
                            1 => cell.deliver(),
                            2 => {
                                cell.close_turn();
                                cell.open_turn();
                            }
                            3 => drop(recorder.stop(recorder.active())),
                            4 => {
                                if let Some(InterruptOut {
                                    on_written: Some(cb),
                                    ..
                                }) = cell.interrupt(LINE.to_vec())
                                {
                                    pend.lock().unwrap().push(cb);
                                }
                            }
                            _ => cell.open_turn(),
                        });
                        take(rig.esc(), &mut line_only, &mut no_line);
                    }
                    8..=12 => {
                        let cb = {
                            let mut p = pending.lock().unwrap();
                            if p.is_empty() {
                                None
                            } else {
                                let i = rng.below(p.len() as u64) as usize;
                                Some(p.swap_remove(i))
                            }
                        };
                        if let Some(cb) = cb {
                            rig.logged(cb);
                        }
                    }
                    13 => {
                        let mut p = pending.lock().unwrap();
                        if !p.is_empty() {
                            let i = rng.below(p.len() as u64) as usize;
                            drop(p.swap_remove(i));
                        }
                    }
                    14 | 15 => rig.open(),
                    16 => rig.logged(|| rig.cell.deliver()),
                    17 => rig.logged(|| rig.cell.close_turn()),
                    18..=20 => rig.clock.advance(ms(rng.below(4_000))),
                    21..=26 => {
                        if rig.worker() {
                            let now = rig.clock.now();
                            let due = rig.state(|st| {
                                st.episode.as_ref().is_some_and(|ep| {
                                    ep.phase == Phase::Waiting
                                        && ep.written
                                        && ep.last_mono + N <= now
                                })
                            });
                            rig.turn();
                            if due && !rig.group.retiring.load(Ordering::SeqCst) {
                                passes += 1;
                            }
                        }
                    }
                    27 => {
                        let failing = rig.group.listing.lock().unwrap().is_none();
                        rig.listing((failing).then(|| vec![CLAUDE, ROOT, CONHOST]));
                    }
                    28 => drop(rig.recorder.stop(rig.recorder.active())),
                    29 => {
                        let refuse = rng.below(3) == 0;
                        rig.clock.refuse_worker.store(refuse, Ordering::SeqCst);
                    }
                    _ => {
                        if rng.below(20) == 0 {
                            rig.group.retiring.store(true, Ordering::Release);
                        }
                    }
                }
                rig.check();
            }
            opened += rig.state(|st| st.next_gen);
            rig.logged(|| rig.cell.close_turn());
            pending.lock().unwrap().clear();
            rig.check();
            assert_eq!(rig.recorder.active(), 0);
            assert!(!rig.has_episode());
        }
        let seen =
            format!("판 {passes} · 에피소드 {opened} · 줄만 {line_only} · 줄 없음 {no_line}");
        assert!(
            passes > 40 && opened > 500 && line_only > 50 && no_line > 50,
            "{seen}"
        );
    }
}

/// 실물 무리 · 포트 · 듣는 스레드 · 쓰기 확인 · 일꾼 · 판을 한 줄로 잇는다(TRD §5 실프로세스 12). 시험마다 넷 이하
/// (콘솔 호스트 포함)를 띄운다. ★끝내는 경로는 재지 않는다★ — claude 훅 실행기 모양 아래 사본은 넷을 넘어 순수
/// 시험(스파이크 모양)이 잰다. 여기서는 판의 사유가 로그에 남고 아무것도 끝나지 않는 것을 본다.
#[cfg(all(test, windows))]
mod real_tests {
    use super::test_support::{Capture, Line};
    use super::*;

    use std::collections::BTreeMap;
    use std::io::Write;

    use crate::platform::process_group::tests::{
        is_ping, new_group, open_gate, spawn_gated_cmd, wait_until,
    };

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const LINE: &[u8] = b"{\"type\":\"control_request\"}\n";

    /// 잠은 기다리지 않고 단조 시각을 그만큼 민다 — 일꾼이 N 을 곧바로 지난다. 일꾼 몸통은 쌓아 두고 시험 스레드가
    /// 돌린다(판의 로그를 잡으려고). 듣는 스레드는 진짜로 띄운다.
    #[derive(Default)]
    struct SkipClock {
        skipped_ns: AtomicU64,
        workers: Mutex<Vec<Box<dyn FnOnce() + Send>>>,
    }

    impl Clock for SkipClock {
        fn now(&self) -> Instant {
            Instant::now() + Duration::from_nanos(self.skipped_ns.load(Ordering::SeqCst))
        }
    }

    impl LeftoverClock for SkipClock {
        fn sleep(&self, d: Duration) {
            let ns = u64::try_from(d.as_nanos()).expect("시험 잠");
            self.skipped_ns.fetch_add(ns, Ordering::SeqCst);
        }

        fn spawn(&self, name: &str, body: Box<dyn FnOnce() + Send>) -> io::Result<()> {
            if name == WORKER_THREAD {
                self.workers.lock().unwrap().push(body);
                Ok(())
            } else {
                SystemClock.spawn(name, body)
            }
        }
    }

    /// 뿌리 `root` 의 정리기와 그 문 — `open_spawn` 의 조립에서 시계만 바꾼 모양.
    fn cleaner_for(
        group: &Arc<ProcessGroup>,
        root: u32,
    ) -> (Arc<SkipClock>, Arc<Cleaner>, Arc<GateCell>) {
        let clock = Arc::new(SkipClock::default());
        let cleaner = Arc::new(Cleaner::new(
            Arc::clone(group),
            Arc::clone(&clock) as Arc<dyn LeftoverClock>,
            root,
            LogTag::new(None, root),
        ));
        let cell = GateCell::new(Some(Arc::clone(&cleaner)));
        (clock, cleaner, cell)
    }

    /// 열린 턴에 끊기 하나를 받고 그 줄이 쓰인 것으로 친다 — 쓰기 확인이 쓰기 명단을 찍고 일꾼 하나를 부른다.
    /// 돌려주는 것 = 그 에피소드의 기록 번호.
    fn interrupt_and_write(cell: &Arc<GateCell>, cleaner: &Cleaner, clock: &SkipClock) -> u64 {
        cell.open_turn();
        let out = cell.interrupt(LINE.to_vec()).expect("끊기 줄");
        let rec = cleaner.recorder.active();
        assert_ne!(rec, 0, "기록이 켜지지 않았다");
        let on_written = out.on_written.expect("쓰기 확인");
        on_written();
        assert_eq!(
            clock.workers.lock().unwrap().len(),
            1,
            "쓰인 끊기가 일꾼을 부르지 않았다"
        );
        rec
    }

    /// 쌓인 일꾼을 이 스레드에서 끝까지 돌리고 그동안의 로그를 준다.
    fn run_workers(clock: &SkipClock) -> Vec<Line> {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let bodies: Vec<_> = clock.workers.lock().unwrap().drain(..).collect();
        for body in bodies {
            tracing::subscriber::with_default(Capture(Arc::clone(&lines)), body);
        }
        let taken = lines.lock().unwrap().clone();
        taken
    }

    /// 판이 남긴 사유 줄 하나 — 아무것도 끝내지 않았다(정리함 줄이 없다).
    fn the_pass_line(lines: &[Line]) -> BTreeMap<String, String> {
        assert!(
            !lines
                .iter()
                .any(|(_, fields)| fields.contains_key("terminated")),
            "무언가를 끝냈다: {lines:?}"
        );
        let mut reasons = lines
            .iter()
            .filter(|(_, fields)| fields.contains_key("reason"));
        let (_, fields) = reasons.next().expect("판의 사유 줄");
        assert!(reasons.next().is_none(), "사유 줄이 둘이다: {lines:?}");
        fields.clone()
    }

    /// 창 없는 콘솔의 R 이 콘솔 호스트를 띄울 때까지 기다린다 — 문을 열기 전 R 의 자식은 그것 하나다. 그 호스트는
    /// R 이 Job 에 든 뒤에 태어나기도 해서(실측), 기다리지 않으면 쓰기 명단 뒤의 산 탄생(부모 R 산다)으로 끼어든다.
    fn console_host_born(r: &std::process::Child) {
        wait_until("R 의 콘솔 호스트", || {
            (!engram_dashboard_platform::process::child_pids(r.id()).is_empty()).then_some(())
        });
    }

    /// 끊기 뒤 R 이 띄운 B(`cmd`)가 P(ping)를 띄우고 끝났다 — P 의 부모는 끝났지만 P 는 B 의 사본이 아니다. P 는 산다.
    /// R · 그 콘솔 호스트 · B · P 넷.
    #[test]
    fn a_ping_left_by_an_exited_cmd_is_not_a_hook_copy_and_survives() {
        let (job, group) = new_group();
        let group = Arc::new(group);
        // R = 문 → B → 둘째 줄을 기다림(뿌리가 판 때 살아 있게). B = P 를 뒤로 띄우고 한 줄을 기다린 뒤 끝난다.
        let mut r = spawn_gated_cmd(
            r#"cmd /d /c "start "" /b ping -n 30 127.0.0.1 >nul & set /p _=" & set /p _="#,
            CREATE_NO_WINDOW,
        );
        job.assign(r.id()).expect("Job 편입");
        console_host_born(&r);
        let (clock, cleaner, cell) = cleaner_for(&group, r.id());
        let rec = interrupt_and_write(&cell, &cleaner, &clock);

        let mut stdin = r.stdin.take().expect("stdin 파이프");
        stdin.write_all(b"go\r\n").expect("R 의 문");
        let (b, p) = wait_until("기록된 B · P", || {
            let births = cleaner.recorder.copy(rec);
            let b = births
                .iter()
                .find(|x| x.facts.ppid == r.id() && image_is(&x.facts.image, "cmd.exe"))
                .cloned()?;
            let p = births
                .iter()
                .find(|x| x.facts.ppid == b.pid && image_is(&x.facts.image, "ping.exe"))
                .cloned()?;
            Some((b, p))
        });
        assert!(b.killable && p.killable);
        stdin.write_all(b"b\r\n").expect("B 의 줄");
        wait_until("끝난 B", || {
            matches!(b.pin.exited(), Ok(true)).then_some(())
        });

        let pass = the_pass_line(&run_workers(&clock));
        assert_eq!(pass["reason"], "parent_rule", "{pass:?}");
        assert_eq!(pass["not_hook_copy"], "1", "{pass:?}");
        assert_eq!(pass["parent_alive"], "0", "{pass:?}");
        assert!(!p.pin.exited().expect("끝났나"), "P 가 끝났다");

        drop((b, p));
        job.terminate(1).expect("Job 끝내기");
        drop(stdin);
        let _ = r.wait();
    }

    /// 끊기 전에 태어난 것만 있다 — 쓰기 명단에 들어 후보가 없다(붙일 때 되알려져 기록에 들어도). R · 그 콘솔
    /// 호스트 · ping 셋.
    #[test]
    fn births_before_the_interrupt_leave_no_new_member() {
        let (job, group) = new_group();
        let group = Arc::new(group);
        let mut r = spawn_gated_cmd("ping -n 30 127.0.0.1 >nul", CREATE_NO_WINDOW);
        job.assign(r.id()).expect("Job 편입");
        open_gate(&mut r);
        wait_until("명단의 ping", || {
            group
                .member_pids()
                .ok()?
                .into_iter()
                .find(|&pid| is_ping(pid))
        });
        let (clock, cleaner, cell) = cleaner_for(&group, r.id());
        interrupt_and_write(&cell, &cleaner, &clock);

        let pass = the_pass_line(&run_workers(&clock));
        assert_eq!(pass["reason"], "no_new_member", "{pass:?}");

        job.terminate(1).expect("Job 끝내기");
        let _ = r.wait();
    }

    /// 끊기 뒤 태어났지만 부모(뿌리 R)가 산다. R · 그 콘솔 호스트 · ping 셋.
    #[test]
    fn a_birth_under_a_live_parent_is_parent_alive() {
        let (job, group) = new_group();
        let group = Arc::new(group);
        let mut r = spawn_gated_cmd("ping -n 30 127.0.0.1 >nul", CREATE_NO_WINDOW);
        job.assign(r.id()).expect("Job 편입");
        console_host_born(&r);
        let (clock, cleaner, cell) = cleaner_for(&group, r.id());
        let rec = interrupt_and_write(&cell, &cleaner, &clock);

        open_gate(&mut r);
        let p = wait_until("기록된 ping", || {
            cleaner
                .recorder
                .copy(rec)
                .into_iter()
                .find(|x| x.facts.ppid == r.id() && image_is(&x.facts.image, "ping.exe"))
        });

        let pass = the_pass_line(&run_workers(&clock));
        assert_eq!(pass["reason"], "parent_rule", "{pass:?}");
        assert_eq!(pass["parent_alive"], "1", "{pass:?}");
        assert_eq!(pass["not_hook_copy"], "0", "{pass:?}");
        assert!(!p.pin.exited().expect("끝났나"), "P 가 끝났다");

        drop(p);
        job.terminate(1).expect("Job 끝내기");
        let _ = r.wait();
    }
}
