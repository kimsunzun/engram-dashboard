//! 사용량 구독 명부 — 연결마다 구독한 벤더 집합과 그 연결의 출구(TRD §1-4 「구독 명부」). 발행 한 장을 대상에
//! 나르는 [`deliver`] 도 여기다.
//!
//! ★칸을 만드는 곳은 [`UsageWatch::attach`] 하나, 지우는 곳은 [`UsageWatch::detach`] 하나다★ —
//!   [`UsageWatch::replace`] 는 있는 칸만 고친다. 그래서 끊김 정리와 겹쳐 든 늦은 교체가 지운 칸을 되살리지
//!   못한다(§3 #56 — 명령 명부의 `attach`·`refuse_if_detached` 와 같은 모양).
//! ★칸을 지우면 그 출구를 거둔다([`UsageOutlet::revoke`])★ — 출구가 쥔 그 연결의 프레임 출구는
//!   [`UsageWatch::detach`] 가 돌아오기 전에 놓이고(연결 정리 포트의 의무 — `net/src/frame_port.rs` `on_disconnect`),
//!   밖에 남은 [`Target`] 사본은 조용해진다. 남는 것은 그때 진행 중이던 보내기마다 하나씩의 일시 사본뿐이다.
//! ★명부 락을 쥔 채 짓지도 보내지도 않는다★ — 락 안에서는 대상(출구 사본 + 그 연결의 지금 집합)만 뜨고,
//!   인코딩·보내기는 [`deliver`] 가 락 없이 한다. 그래서 출구·인코더가 이 명부를 다시 불러도 교착이 없다.
//! ★락 순서 = 명부 → 책★ — 명부 락 안에서 남의 코드를 부르는 자리는 [`UsageWatch::replace`] 의 `first_sheets`
//!   하나다. 거기서 책을 잡는 것은 되고 명부를 다시 부르면 교착이다(std `Mutex` 는 재진입하지 않는다). 책을 쥔
//!   채 명부를 부르는 자리는 없어야 한다.
// ADR-0006

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use engram_dashboard_agent::usage::UsageVendorKey;
use engram_dashboard_net::frame_port::ConnId;
use engram_dashboard_protocol::UsageLimitSnapshot;

/// 연결 하나의 출구 — 실물은 연결 계층이 `on_connect` 에서 연결마다 새로 만든다: 칸 하나
/// (`Mutex<Option<Arc<dyn FrameSink>>>`)를 쥐고, `send` = 칸을 잠가 (있으면) `Arc` 사본만 뜨고 놓은 뒤 **칸 밖에서**
/// `try_send` · `revoke` = 칸을 잠가 꺼내(`take`) 놓는다.
///
/// ★칸을 쥔 채 보내지 않는다(ADR-0006)★ — 칸 안의 일은 사본 뜨기·꺼내기뿐이다: 명부·서비스·다른 락을 부르지
///   않고, 로그도 칸을 놓은 뒤 찍는다. 그래서 `revoke` 가 진행 중인 보내기를 기다리지 않는다.
/// 두 메서드 다 이 명부의 락을 쥐지 않은 채 불린다 — 명부 쪽에서는 안에서 명부를 다시 불러도 교착이 없다.
pub trait UsageOutlet: Send + Sync {
    /// ★논블록이어야 한다★ — pump 스레드·스케줄러·조회 완료 가드, 그리고 연결 태스크(구독의 첫 한 장 · 요청
    /// 경로의 발행)에서 불린다. 실패(포화·닫힘)는 출구가 스스로 처리하고 부르는 쪽에 돌려주지 않는다.
    fn send(&self, frame: &UsageFrame);

    /// 이 출구를 거둔다 — 명부가 칸을 지울 때(끊김 · 같은 연결 id 로 다른 출구가 덮음) 그 칸마다 한 번, 명부 락
    /// 밖에서 부른다. 연결 정리 경로(동기)라 막히지 않아야 한다.
    ///
    /// ★돌아온 뒤에는 출구가 프레임 출구를 쥐지 않고 `send` 가 아무것도 안 한다★ — 밖에 남은 [`Target`] 은
    ///   조용해진다. 남는 것은 그때 진행 중이던 `send` 마다(스레드마다) 뜬 사본 하나뿐이고, 그 `try_send` 한 번
    ///   뒤 놓인다(TRD §1-4 「동시성·락」의 일시 사본 · §3 #87).
    fn revoke(&self);
}

/// 발행 한 장을 `subscribed` 집합 하나로 지은 것 — `json` 은 `snapshot` 과 그 집합을 실은 이벤트 한 줄이다. 같은
/// 집합의 대상은 같은 `json` 을 나눠 쓴다.
#[derive(Debug, Clone)]
pub struct UsageFrame {
    pub snapshot: UsageLimitSnapshot,
    pub json: Arc<str>,
}

/// 이벤트 JSON 을 짓는 포트 — 실물은 조립이 준다.
///
/// 명부 락 밖에서, 발행 한 번에 서로 다른 `subscribed` 마다 한 번 불린다. `None` = 짓기 실패 — 그 집합의 대상은
/// 이번 발행을 건너뛴다. ★실패 로그는 인코더가 스스로 남긴다★ — [`deliver`] 는 찍지 않는다.
pub trait UsageEncoder: Send + Sync {
    fn encode(
        &self,
        snapshot: &UsageLimitSnapshot,
        subscribed: &BTreeSet<UsageVendorKey>,
    ) -> Option<Arc<str>>;
}

/// 발행 대상 하나 — 명부 락 안에서 뜬 그 연결의 출구 사본과 그때의 집합.
///
/// ★사본은 락을 놓는 순간 낡는다★ — `subscribed` 가 한 발 늦을 수 있고(셸의 대조가 같은 집합을 한 장 더 보낼
///   뿐이다 · TRD §1-7), 그 연결이 이미 끊겼을 수도 있다(거둔 출구라 보내기가 아무것도 안 한다).
/// ★뜬 자리에서 [`deliver`] 에 넘긴다 — 쌓아 두거나 기다림을 건너 쥐지 말 것★: 쥔 동안 낡는 집합을 싣게 되고,
///   끊긴 연결의 출구 껍데기도 그만큼 산다.
#[derive(Clone)]
pub struct Target {
    pub outlet: Arc<dyn UsageOutlet>,
    pub subscribed: Arc<BTreeSet<UsageVendorKey>>,
}

/// [`UsageWatch::replace`] 의 결과 — `first_sheets` 는 이번 교체로 새로 든 벤더의 첫 한 장들이고 `target` 은 그
/// 연결 하나다(`subscribed` = 교체한 **뒤**의 집합). 첫 한 장은 `target` 에만 [`deliver`] 한다.
pub struct Replaced {
    pub first_sheets: Vec<UsageLimitSnapshot>,
    pub target: Target,
}

struct Entry {
    // 교체는 새 `Arc` 를 세운다 — 대상마다 집합을 복사하지 않고 나눠 쓴다.
    vendors: Arc<BTreeSet<UsageVendorKey>>,
    outlet: Arc<dyn UsageOutlet>,
}

pub struct UsageWatch {
    entries: Mutex<HashMap<ConnId, Entry>>,
}

impl Default for UsageWatch {
    fn default() -> Self {
        Self::new()
    }
}

impl UsageWatch {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// 연결이 붙었다(`on_connect`) — 빈 집합의 칸을 만든다. ★칸을 만드는 곳은 여기뿐이다★.
    /// 같은 연결 id 가 다시 오면 앞 칸을 덮고 앞 출구를 거둔다(연결 id 는 재사용되지 않는다 — 오면 error 로그).
    ///
    /// ★출구를 값으로 받아 여기서 `Arc` 로 싼다 — 칸마다 출구 객체가 따로다★: 그래서 거두기가 다른 칸의 출구를
    ///   조용하게 만들 수 없다(지운 칸의 `revoke` 는 락을 놓은 뒤 돌아, 그 사이 새로 든 칸과 겹칠 수 있다). 밖에서
    ///   만든 `Arc` 를 받지 말 것 — `Arc`·`Box` 에 대한 `UsageOutlet` 일괄 구현도, 칸(자물쇠)을 다른 출구와 나눠
    ///   쥐는 출구(`Clone` 되는 `Arc` 감싸개)도 같은 길을 연다. 실물은 칸을 직접 쥔다.
    pub fn attach(&self, conn: ConnId, outlet: impl UsageOutlet + 'static) {
        let replaced = self.lock().insert(
            conn,
            Entry {
                vendors: Arc::new(BTreeSet::new()),
                outlet: Arc::new(outlet),
            },
        );
        if let Some(previous) = replaced {
            tracing::error!(
                conn,
                "같은 연결 id 로 사용량 명부에 두 번 붙었다 — 앞 칸을 덮었다(연결 id 는 재사용되지 않는다)"
            );
            // 락 밖에서 거두고 놓는다 — 사유는 `detach` 와 같다.
            previous.outlet.revoke();
        }
    }

    /// 연결이 끊겼다(`on_disconnect`) — 칸을 지우고 그 출구를 **반환 전에** 거둔다([`UsageOutlet::revoke`]).
    /// 그래서 출구가 쥔 그 연결의 프레임 출구는 이 호출 안에서 놓이고(연결 정리 포트의 의무 — `net/src/frame_port.rs`
    /// `on_disconnect`) 밖에 남은 [`Target`] 사본은 조용해진다. ★칸을 지우는 곳은 여기뿐이다★. 반환 = 칸이 있었다.
    pub fn detach(&self, conn: ConnId) -> bool {
        // ★거두기·놓기는 락 **밖에서** 한다★ — 거두기·소멸이 무엇을 하는지(실물 = 연결 큐 송신단 닫기·깨우기) 이
        //   명부는 모른다. 명령 명부 `detach` 와 같은 이유다.
        let Some(removed) = self.lock().remove(&conn) else {
            return false;
        };
        removed.outlet.revoke();
        true
    }

    /// `UsageSubscribe` — 그 연결의 집합을 통째로 바꾼다(빈 집합 = 해제 · TRD §3 #54). 집합이 그대로여도 `Some`
    /// 이다. ★칸이 없으면(끊긴 뒤 늦게 든 교체) 아무것도 안 하고 `None`★ — 칸을 되살리지 않고, 여기서는 찍지도
    /// 않는다: 로그(연결 id · debug)는 부르는 쪽이 락 밖에서 남긴다(명령 명부 `refuse_if_detached` 와 같은 분담).
    ///
    /// ★`first_sheets` 는 명부 락 안에서, 새로 든 벤더가 있을 때만 한 번 불린다★(인자 = 새로 든 벤더, 키 순서) —
    ///   그 안에서 책을 잡아 첫 한 장을 뜬다(락 순서 `명부 → 책`). ★책 안의 짧은 메모리 일(`eval_time`·스냅숏)뿐
    ///   — I/O·기다림·이 명부 호출 금지★(명부 호출은 교착이다). 도는 동안 pump·스케줄러의 [`UsageWatch::targets`]
    ///   가 막힌다. 집합을 고친 **뒤** 부르므로 「등록 먼저, 스냅숏 나중」이 선다(§3 #57): 겹친 발행은 이 스냅숏에
    ///   들었거나 바뀐 집합으로 이 연결을 대상으로 본다.
    #[must_use = "첫 한 장은 부르는 쪽이 그 연결에 보낸다"]
    pub fn replace(
        &self,
        conn: ConnId,
        vendors: BTreeSet<UsageVendorKey>,
        first_sheets: impl FnOnce(&[UsageVendorKey]) -> Vec<UsageLimitSnapshot>,
    ) -> Option<Replaced> {
        let mut entries = self.lock();
        let entry = entries.get_mut(&conn)?;
        let previous = std::mem::replace(&mut entry.vendors, Arc::new(vendors));
        let added: Vec<UsageVendorKey> = entry.vendors.difference(&previous).copied().collect();
        let target = Target {
            outlet: entry.outlet.clone(),
            subscribed: entry.vendors.clone(),
        };
        let first_sheets = if added.is_empty() {
            Vec::new()
        } else {
            first_sheets(&added)
        };
        Some(Replaced {
            first_sheets,
            target,
        })
    }

    /// 모든 연결의 집합의 합 — 스케줄러가 돌볼 벤더다.
    pub fn union(&self) -> BTreeSet<UsageVendorKey> {
        self.lock()
            .values()
            .flat_map(|entry| entry.vendors.iter().copied())
            .collect()
    }

    /// `vendor` 를 구독한 연결마다 대상 하나 — 그 연결의 지금 집합을 싣는다. 순서는 정하지 않는다.
    /// pump 스레드의 발행마다 불린다 — 락 안에서는 `Arc` 사본 뜨기만 한다.
    pub fn targets(&self, vendor: UsageVendorKey) -> Vec<Target> {
        self.lock()
            .values()
            .filter(|entry| entry.vendors.contains(&vendor))
            .map(|entry| Target {
                outlet: entry.outlet.clone(),
                subscribed: entry.vendors.clone(),
            })
            .collect()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<ConnId, Entry>> {
        // poison 을 견딘다 — 이 명부는 pump 스레드의 발행 경로에 있다. 락 안의 변경은 넣기·빼기·집합 한 번
        //   바꾸기뿐이라 칸은 늘 온전하다: `first_sheets` 가 패닉하면 새 집합이 선 채 그 첫 한 장만 안 나간다
        //   (다음 revision 발행이 닿는다). 릴리즈는 `panic = "abort"` 라 debug·시험에서만 서는 갈래다.
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// 한 장을 대상들에 나른다 — ★어떤 락도 쥐지 않은 채 부른다★. 대상이 실은 서로 다른 `subscribed` 마다 한 번
/// 짓고(대상이 없으면 안 짓는다) 그 집합의 대상마다 보낸다. 짓기가 `None` 이면 그 집합의 대상만 건너뛴다.
/// 대상은 받아 가지고 돌아오기 전에 전부 놓는다.
pub fn deliver(snapshot: &UsageLimitSnapshot, targets: Vec<Target>, encoder: &dyn UsageEncoder) {
    // 집합 수는 연결 수를 넘지 않고 운영에서는 셸 하나라 대개 1 이다 — 선형 탐색으로 묶는다.
    type Group = (Arc<BTreeSet<UsageVendorKey>>, Vec<Arc<dyn UsageOutlet>>);
    let mut groups: Vec<Group> = Vec::new();
    for Target { outlet, subscribed } in targets {
        match groups
            .iter_mut()
            .find(|(set, _)| Arc::ptr_eq(set, &subscribed) || **set == *subscribed)
        {
            Some((_, outlets)) => outlets.push(outlet),
            None => groups.push((subscribed, vec![outlet])),
        }
    }
    for (subscribed, outlets) in groups {
        let Some(json) = encoder.encode(snapshot, &subscribed) else {
            continue;
        };
        let frame = UsageFrame {
            snapshot: snapshot.clone(),
            json,
        };
        for outlet in outlets {
            outlet.send(&frame);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage_service::book::{Now, UsageBook};
    use engram_dashboard_agent::backend::usage_probes;
    use engram_dashboard_agent::usage::{UsageAccountKey, UsageKey};
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Weak};
    use std::time::Duration;

    // 조회기 키로만 집합을 만든다 — 시험에도 벤더 리터럴을 두지 않는다.
    fn vendor(i: usize) -> UsageVendorKey {
        usage_probes()[i].key()
    }

    fn set(indices: &[usize]) -> BTreeSet<UsageVendorKey> {
        indices.iter().map(|&i| vendor(i)).collect()
    }

    fn sheet_of(vendor: UsageVendorKey) -> UsageLimitSnapshot {
        let book = UsageBook::new(&usage_probes().map(|p| (p.key(), p.policy())));
        let key = UsageKey {
            vendor,
            account: UsageAccountKey::default(),
        };
        let now = Now {
            mono: Duration::ZERO,
            wall: 1_900_000_000,
        };
        book.snapshot(&key, now).expect("아는 키")
    }

    fn sheet(i: usize) -> UsageLimitSnapshot {
        sheet_of(vendor(i))
    }

    fn words(subscribed: &BTreeSet<UsageVendorKey>) -> String {
        subscribed
            .iter()
            .map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }

    fn vendors_of(watch: &UsageWatch, conn: ConnId) -> Option<BTreeSet<UsageVendorKey>> {
        watch
            .lock()
            .get(&conn)
            .map(|entry| (*entry.vendors).clone())
    }

    fn no_sheets(_: &[UsageVendorKey]) -> Vec<UsageLimitSnapshot> {
        Vec::new()
    }

    /// 출구가 명부에 들어간 뒤에도 시험이 들여다보는 기록. 출구가 살아 있는 동안 강참조가 하나 더 있다 —
    /// `Arc::strong_count(&log) == 1` 이면 출구가 소멸했다.
    #[derive(Default)]
    struct Log {
        frames: Mutex<Vec<UsageFrame>>,
        revoked: AtomicUsize,
        after_revoke: AtomicUsize,
    }

    impl Log {
        fn jsons(&self) -> Vec<String> {
            self.frames
                .lock()
                .unwrap()
                .iter()
                .map(|f| f.json.to_string())
                .collect()
        }

        fn count(&self) -> usize {
            self.frames.lock().unwrap().len()
        }

        fn revokes(&self) -> usize {
            self.revoked.load(Ordering::SeqCst)
        }
    }

    /// 받은 프레임을 [`Log`] 에 적는다. 거둔 뒤의 보내기는 적지 않고 따로 센다.
    struct Recording(Arc<Log>);

    fn recording() -> (Recording, Arc<Log>) {
        let log = Arc::new(Log::default());
        (Recording(log.clone()), log)
    }

    impl UsageOutlet for Recording {
        fn send(&self, frame: &UsageFrame) {
            if self.0.revokes() > 0 {
                self.0.after_revoke.fetch_add(1, Ordering::SeqCst);
                return;
            }
            self.0.frames.lock().unwrap().push(frame.clone());
        }

        fn revoke(&self) {
            self.0.revoked.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// JSON = 집합의 낱말들. `fail_for` 집합이면 `None`.
    #[derive(Default)]
    struct Counting {
        calls: AtomicUsize,
        fail_for: Option<BTreeSet<UsageVendorKey>>,
    }

    impl Counting {
        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl UsageEncoder for Counting {
        fn encode(
            &self,
            _snapshot: &UsageLimitSnapshot,
            subscribed: &BTreeSet<UsageVendorKey>,
        ) -> Option<Arc<str>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_for.as_ref() == Some(subscribed) {
                return None;
            }
            Some(Arc::from(words(subscribed)))
        }
    }

    /// 명부를 거치지 않은 대상 하나와 그 기록.
    fn target(subscribed: BTreeSet<UsageVendorKey>) -> (Target, Arc<Log>) {
        let (outlet, log) = recording();
        let target = Target {
            outlet: Arc::new(outlet),
            subscribed: Arc::new(subscribed),
        };
        (target, log)
    }

    // ── 칸의 수명 ──

    #[test]
    fn attach_starts_with_an_empty_set() {
        let watch = UsageWatch::new();
        watch.attach(1, recording().0);
        assert_eq!(vendors_of(&watch, 1), Some(BTreeSet::new()));
        assert!(watch.union().is_empty());
        assert!(watch.targets(vendor(0)).is_empty());
    }

    #[test]
    fn replace_swaps_the_whole_set() {
        let watch = UsageWatch::new();
        watch.attach(1, recording().0);
        watch.replace(1, set(&[0, 1]), no_sheets).expect("칸");
        watch.replace(1, set(&[1]), no_sheets).expect("칸");
        assert_eq!(
            vendors_of(&watch, 1),
            Some(set(&[1])),
            "두 번째가 첫 번째를 덮는다"
        );
        watch.replace(1, set(&[0]), no_sheets).expect("칸");
        assert_eq!(vendors_of(&watch, 1), Some(set(&[0])));
    }

    #[test]
    fn an_empty_replace_unsubscribes() {
        let watch = UsageWatch::new();
        watch.attach(1, recording().0);
        watch.replace(1, set(&[0, 1]), no_sheets).expect("칸");
        let replaced = watch.replace(1, BTreeSet::new(), no_sheets).expect("칸");
        assert!(replaced.target.subscribed.is_empty());
        assert!(watch.union().is_empty());
        assert!(watch.targets(vendor(0)).is_empty());
        assert_eq!(vendors_of(&watch, 1), Some(BTreeSet::new()), "칸은 남는다");
    }

    #[test]
    fn a_replace_after_detach_is_dropped_without_reviving_the_entry() {
        let watch = UsageWatch::new();
        watch.attach(1, recording().0);
        assert!(watch.detach(1));
        let called = AtomicUsize::new(0);
        let late = watch.replace(1, set(&[0]), |_| {
            called.fetch_add(1, Ordering::SeqCst);
            vec![sheet(0)]
        });
        assert!(late.is_none());
        assert_eq!(called.load(Ordering::SeqCst), 0, "첫 한 장을 뜨지 않는다");
        assert_eq!(vendors_of(&watch, 1), None, "칸이 되살아나지 않는다");
        assert!(watch.union().is_empty());
        assert!(watch.targets(vendor(0)).is_empty());
        assert!(
            watch.replace(9, set(&[0]), no_sheets).is_none(),
            "붙은 적 없는 연결도 같다"
        );
    }

    #[test]
    fn detach_revokes_the_outlet_once_and_a_held_target_goes_quiet() {
        let watch = UsageWatch::new();
        let (outlet, log) = recording();
        watch.attach(1, outlet);
        watch.replace(1, set(&[0]), no_sheets).expect("칸");
        let held = watch.targets(vendor(0));
        assert_eq!(Arc::strong_count(&held[0].outlet), 2, "명부 + 쥔 대상");

        assert!(watch.detach(1));
        assert_eq!(log.revokes(), 1);
        assert_eq!(
            Arc::strong_count(&held[0].outlet),
            1,
            "명부가 쥔 사본이 놓였다"
        );

        let encoder = Counting::default();
        deliver(&sheet(0), held, &encoder);
        assert_eq!(log.count(), 0, "거둔 뒤의 보내기는 버려진다");
        assert_eq!(log.after_revoke.load(Ordering::SeqCst), 1);
        assert_eq!(
            Arc::strong_count(&log),
            1,
            "나르기가 대상을 놓아 출구가 소멸했다"
        );

        assert!(!watch.detach(1), "두 번째 해제는 칸이 없다");
        assert_eq!(log.revokes(), 1, "거두기는 한 번");
    }

    #[test]
    fn an_overwriting_attach_revokes_the_old_outlet_and_keeps_the_new_one_live() {
        let watch = UsageWatch::new();
        let (old, old_log) = recording();
        let (new, new_log) = recording();
        watch.attach(2, old);
        watch.attach(2, new);
        assert_eq!(old_log.revokes(), 1);
        assert_eq!(Arc::strong_count(&old_log), 1, "앞 출구가 소멸했다");
        assert_eq!(new_log.revokes(), 0);

        watch.replace(2, set(&[0]), no_sheets).expect("칸");
        deliver(&sheet(0), watch.targets(vendor(0)), &Counting::default());
        assert_eq!((old_log.count(), new_log.count()), (0, 1), "새 출구는 산다");

        assert!(watch.detach(2));
        assert_eq!(new_log.revokes(), 1);
        assert_eq!(Arc::strong_count(&new_log), 1);
    }

    /// 거둘 때·소멸할 때 명부 락이 풀려 있었는지 적는 출구.
    struct LockProbe {
        watch: Weak<UsageWatch>,
        log: Arc<Mutex<Vec<(&'static str, Option<bool>)>>>,
    }

    impl LockProbe {
        fn note(&self, what: &'static str) {
            let free = self
                .watch
                .upgrade()
                .map(|watch| watch.entries.try_lock().is_ok());
            self.log.lock().unwrap().push((what, free));
        }
    }

    impl UsageOutlet for LockProbe {
        fn send(&self, _frame: &UsageFrame) {}

        fn revoke(&self) {
            self.note("revoke");
        }
    }

    impl Drop for LockProbe {
        fn drop(&mut self) {
            self.note("drop");
        }
    }

    #[test]
    fn removed_outlets_are_revoked_then_dropped_outside_the_lock() {
        let watch = Arc::new(UsageWatch::new());
        let expected = [("revoke", Some(true)), ("drop", Some(true))];

        let detached = Arc::new(Mutex::new(Vec::new()));
        watch.attach(
            1,
            LockProbe {
                watch: Arc::downgrade(&watch),
                log: detached.clone(),
            },
        );
        watch.detach(1);
        assert_eq!(*detached.lock().unwrap(), expected);

        let overwritten = Arc::new(Mutex::new(Vec::new()));
        watch.attach(
            2,
            LockProbe {
                watch: Arc::downgrade(&watch),
                log: overwritten.clone(),
            },
        );
        watch.attach(2, recording().0);
        assert_eq!(*overwritten.lock().unwrap(), expected);
    }

    // ── 합집합·대상 ──

    #[test]
    fn union_is_the_union_of_connections() {
        let watch = UsageWatch::new();
        watch.attach(1, recording().0);
        watch.attach(2, recording().0);
        watch.replace(1, set(&[0]), no_sheets).expect("칸");
        watch.replace(2, set(&[1]), no_sheets).expect("칸");
        assert_eq!(watch.union(), set(&[0, 1]));
        watch.detach(1);
        assert_eq!(watch.union(), set(&[1]), "남은 연결의 벤더는 남는다");
    }

    #[test]
    fn targets_are_exactly_the_subscribers_with_their_current_sets() {
        let watch = UsageWatch::new();
        let (a, a_log) = recording();
        let (b, b_log) = recording();
        let (c, c_log) = recording();
        watch.attach(1, a);
        watch.attach(2, b);
        watch.attach(3, c);
        watch.replace(1, set(&[0]), no_sheets).expect("칸");
        watch.replace(2, set(&[0, 1]), no_sheets).expect("칸");

        let encoder = Counting::default();
        deliver(&sheet(0), watch.targets(vendor(0)), &encoder);
        assert_eq!(a_log.jsons(), [words(&set(&[0]))]);
        assert_eq!(b_log.jsons(), [words(&set(&[0, 1]))]);
        assert_eq!(c_log.count(), 0, "아무것도 구독하지 않은 연결");

        deliver(&sheet(1), watch.targets(vendor(1)), &encoder);
        assert_eq!(a_log.count(), 1, "그 벤더를 구독하지 않은 연결");
        assert_eq!(b_log.count(), 2);

        watch.replace(1, set(&[0, 1]), no_sheets).expect("칸");
        let after: Vec<_> = watch
            .targets(vendor(0))
            .into_iter()
            .map(|t| (*t.subscribed).clone())
            .collect();
        assert_eq!(
            after,
            [set(&[0, 1]), set(&[0, 1])],
            "바뀐 뒤엔 새 집합을 싣는다"
        );
    }

    // ── 첫 한 장 ──

    #[test]
    fn first_sheets_are_only_for_newly_added_vendors_and_only_for_that_connection() {
        let watch = UsageWatch::new();
        let (mine, mine_log) = recording();
        let (other, other_log) = recording();
        watch.attach(1, mine);
        watch.attach(2, other);
        watch.replace(2, set(&[0, 1]), no_sheets).expect("칸");
        let asked: Mutex<Vec<Vec<UsageVendorKey>>> = Mutex::new(Vec::new());
        let encoder = Counting::default();
        let replace = |vendors: BTreeSet<UsageVendorKey>| {
            let replaced = watch
                .replace(1, vendors, |added| {
                    asked.lock().unwrap().push(added.to_vec());
                    added.iter().map(|&v| sheet_of(v)).collect()
                })
                .expect("칸");
            for first in &replaced.first_sheets {
                deliver(first, vec![replaced.target.clone()], &encoder);
            }
            replaced
        };

        let r = replace(set(&[0]));
        assert_eq!(r.first_sheets.len(), 1);
        assert_eq!(*r.target.subscribed, set(&[0]), "교체한 뒤의 집합");
        let r = replace(set(&[0, 1]));
        assert_eq!(r.first_sheets.len(), 1, "그대로인 벤더는 다시 안 뜬다");
        assert_eq!(*r.target.subscribed, set(&[0, 1]));
        assert_eq!(replace(set(&[0, 1])).first_sheets.len(), 0, "그대로");
        assert_eq!(replace(set(&[1])).first_sheets.len(), 0, "빠지기만 함");

        assert_eq!(*asked.lock().unwrap(), [vec![vendor(0)], vec![vendor(1)]]);
        assert_eq!(
            mine_log.jsons(),
            [words(&set(&[0])), words(&set(&[0, 1]))],
            "첫 한 장마다 교체한 뒤의 집합을 싣는다"
        );
        assert_eq!(other_log.count(), 0, "다른 연결에는 안 간다");
    }

    #[test]
    fn two_new_vendors_are_asked_in_one_call_in_key_order() {
        let watch = UsageWatch::new();
        watch.attach(1, recording().0);
        let mut expected = vec![vendor(0), vendor(1)];
        expected.sort();
        let asked: Mutex<Vec<Vec<UsageVendorKey>>> = Mutex::new(Vec::new());
        let replaced = watch
            .replace(1, set(&[1, 0]), |added| {
                asked.lock().unwrap().push(added.to_vec());
                added.iter().map(|&v| sheet_of(v)).collect()
            })
            .expect("칸");
        assert_eq!(*asked.lock().unwrap(), [expected]);
        assert_eq!(replaced.first_sheets.len(), 2);
    }

    // ── 나르기 ──

    #[test]
    fn deliver_encodes_once_for_one_set_and_releases_the_targets() {
        let (targets, logs): (Vec<_>, Vec<_>) = (0..3).map(|_| target(set(&[0]))).unzip();
        let encoder = Counting::default();
        deliver(&sheet(0), targets, &encoder);
        assert_eq!(encoder.calls(), 1);
        for log in &logs {
            assert_eq!(log.count(), 1);
            assert_eq!(Arc::strong_count(log), 1, "대상 사본이 남지 않는다");
        }
        let first = logs[0].frames.lock().unwrap()[0].json.clone();
        let second = logs[1].frames.lock().unwrap()[0].json.clone();
        assert!(Arc::ptr_eq(&first, &second), "같은 JSON 을 나눠 쓴다");
    }

    #[test]
    fn deliver_encodes_once_per_distinct_set() {
        let (a, a_log) = target(set(&[0]));
        let (b, b_log) = target(set(&[0, 1]));
        let (c, c_log) = target(set(&[0]));
        let encoder = Counting::default();
        deliver(&sheet(0), vec![a, b, c], &encoder);
        assert_eq!(encoder.calls(), 2);
        assert_eq!(a_log.jsons(), [words(&set(&[0]))]);
        assert_eq!(b_log.jsons(), [words(&set(&[0, 1]))]);
        assert_eq!(c_log.jsons(), [words(&set(&[0]))]);
    }

    #[test]
    fn deliver_without_targets_does_not_encode() {
        let encoder = Counting::default();
        deliver(&sheet(0), Vec::new(), &encoder);
        assert_eq!(encoder.calls(), 0);
    }

    #[test]
    fn a_failed_encoding_skips_only_its_set() {
        let (a, a_log) = target(set(&[0]));
        let (b, b_log) = target(set(&[0, 1]));
        let encoder = Counting {
            fail_for: Some(set(&[0])),
            ..Counting::default()
        };
        deliver(&sheet(0), vec![a, b], &encoder);
        assert_eq!(encoder.calls(), 2);
        assert_eq!(a_log.count(), 0);
        assert_eq!(Arc::strong_count(&a_log), 1, "건너뛴 몫의 대상도 놓는다");
        assert_eq!(b_log.jsons(), [words(&set(&[0, 1]))]);
    }

    // ── 락 ──

    /// 보내는 중에 명부를 다시 부르는 출구 — 교체·대상·해제(자기 연결 — 그래서 자기 `revoke` 가 겹친다).
    struct Reentrant {
        watch: Weak<UsageWatch>,
        conn: ConnId,
        sent: Arc<AtomicUsize>,
        revoked: Arc<AtomicUsize>,
    }

    impl UsageOutlet for Reentrant {
        fn send(&self, _frame: &UsageFrame) {
            let watch = self.watch.upgrade().expect("명부");
            let _ = watch.replace(2, set(&[1]), no_sheets);
            watch.targets(vendor(0));
            watch.detach(self.conn);
            self.sent.fetch_add(1, Ordering::SeqCst);
        }

        fn revoke(&self) {
            self.revoked.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// 짓는 중에 합집합을 읽는 인코더.
    struct UnionReading {
        watch: Weak<UsageWatch>,
        seen: Mutex<Option<BTreeSet<UsageVendorKey>>>,
    }

    impl UsageEncoder for UnionReading {
        fn encode(
            &self,
            _snapshot: &UsageLimitSnapshot,
            subscribed: &BTreeSet<UsageVendorKey>,
        ) -> Option<Arc<str>> {
            *self.seen.lock().unwrap() = Some(self.watch.upgrade().expect("명부").union());
            Some(Arc::from(words(subscribed)))
        }
    }

    #[test]
    fn outlets_and_encoders_may_call_back_into_the_watch() {
        // 교착이면 매달리는 대신 실패하도록 다른 스레드에서 돌린다.
        let (done_tx, done_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let watch = Arc::new(UsageWatch::new());
            let sent = Arc::new(AtomicUsize::new(0));
            let revoked = Arc::new(AtomicUsize::new(0));
            watch.attach(
                1,
                Reentrant {
                    watch: Arc::downgrade(&watch),
                    conn: 1,
                    sent: sent.clone(),
                    revoked: revoked.clone(),
                },
            );
            watch.attach(2, recording().0);
            watch.replace(1, set(&[0]), no_sheets).expect("칸");
            watch.replace(2, set(&[0]), no_sheets).expect("칸");
            let encoder = UnionReading {
                watch: Arc::downgrade(&watch),
                seen: Mutex::new(None),
            };
            deliver(&sheet(0), watch.targets(vendor(0)), &encoder);
            let seen = encoder.seen.lock().unwrap().clone();
            done_tx
                .send((
                    sent.load(Ordering::SeqCst),
                    revoked.load(Ordering::SeqCst),
                    seen,
                    vendors_of(&watch, 1),
                ))
                .unwrap();
        });
        let (sent, revoked, seen, after) = done_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("교착 없이 끝난다");
        assert_eq!(sent, 1);
        assert_eq!(revoked, 1, "출구 안에서 부른 해제가 그 출구를 거뒀다");
        assert_eq!(seen, Some(set(&[0])));
        assert_eq!(after, None);
    }

    #[test]
    fn a_poisoned_lock_stays_usable() {
        let watch = UsageWatch::new();
        let (outlet, log) = recording();
        watch.attach(1, outlet);
        let panicked = catch_unwind(AssertUnwindSafe(|| {
            watch.replace(1, set(&[0]), |_| panic!("시험 — 첫 한 장을 뜨다 패닉"))
        }));
        assert!(panicked.is_err());
        assert!(watch.entries.is_poisoned(), "시험이 poison 갈래를 탔다");

        assert_eq!(watch.union(), set(&[0]), "새 집합이 선 채 남는다");
        watch.replace(1, set(&[1]), no_sheets).expect("칸");
        assert_eq!(watch.targets(vendor(1)).len(), 1);
        watch.attach(2, recording().0);
        assert!(watch.detach(1));
        assert_eq!(log.revokes(), 1);
        assert_eq!(Arc::strong_count(&log), 1);
    }
}
