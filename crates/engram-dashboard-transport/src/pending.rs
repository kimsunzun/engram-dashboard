//! 요청/응답 상관 — ★순수 층★. 대기 표 · 시한 · 만료 번호 재사용 거절.
//!
//! ★이 파일에는 tokio 도 소켓도 워크스페이스 crate 도 없다★ — 게이트가 그것을 잰다(정본 =
//! `lib.rs` 헤더). 깨우는 손잡이(`oneshot::Sender` 같은 것)는 **제네릭 슬롯 `S`** 로 받아 두기만 하고,
//! 실제로 깨우는 것은 [`crate::peer`] 의 감독 태스크다.
//!
//! ★표를 감독 태스크가 단독 소유한다(락 없음)★ — 오늘 셸의 `PendingMap<T>` 과 같은 모양이다.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::hash::Hash;
use std::time::{Duration, Instant};

use crate::machine::{DisconnectCause, Generation};

/// 요청이 실패하는 방식 전부.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestError {
    /// 시한이 지났다. ★자동 재전송하지 않는다★ — 무응답은 "실패" 가 아니라 **"모른다"** 라, 다시 보내면
    /// 상대가 실은 처리했던 조작이 두 번 적용된다(ADR-0181 결정 3).
    TimedOut { after: Duration },
    /// 보내기 전에 연결이 끊겼거나, 보낸 뒤 답장 전에 끊겼다.
    Disconnected {
        generation: Generation,
        cause: DisconnectCause,
    },
    /// 나가는 큐가 찼다.
    QueueFull,
    /// 같은 번호가 겹쳐 새 요청이 슬롯을 승계했다 — 옛 대기자가 이것으로 깨어난다.
    Superseded,
    /// 이 명령은 답장을 안 받는다([`crate::Wire::request_tag`] 가 `None`). `notify` 를 쓰라는 뜻이다.
    NotARequest,
    /// 만료됐던 번호를 다시 썼다.
    TagRetired,
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TimedOut { after } => write!(f, "no reply within {after:?}"),
            Self::Disconnected { generation, cause } => {
                write!(f, "disconnected (generation {}, {cause:?})", generation.0)
            }
            Self::QueueFull => f.write_str("outbound queue full"),
            Self::Superseded => f.write_str("request id reused by a newer request"),
            Self::NotARequest => f.write_str("this message does not take a reply"),
            Self::TagRetired => f.write_str("request id was retired by an earlier timeout"),
        }
    }
}

impl std::error::Error for RequestError {}

/// 만료된 번호를 기억하는 유계 링.
///
/// ★진짜 보증은 여기가 아니라 **번호를 만드는 쪽**에 있다★ — 우리 경우 uuid v4 다. 이 링이 할 수 있는
/// 것은 **최근** 만료 번호가 다시 오면 거절하는 것뿐이고, 기억은 무한할 수 없다. 넘치면 가장 오래된
/// 것부터 조용히 잊고, 그 번호는 다시 쓸 수 있게 된다. ★이 한계를 보증으로 읽지 말 것★.
#[derive(Debug)]
struct RetiredRing<T> {
    order: VecDeque<T>,
    set: HashSet<T>,
    cap: usize,
}

impl<T: Eq + Hash + Clone> RetiredRing<T> {
    fn new(cap: usize) -> Self {
        Self {
            order: VecDeque::new(),
            set: HashSet::new(),
            cap,
        }
    }

    fn insert(&mut self, tag: T) {
        if self.cap == 0 || !self.set.insert(tag.clone()) {
            return;
        }
        self.order.push_back(tag);
        while self.order.len() > self.cap {
            if let Some(old) = self.order.pop_front() {
                self.set.remove(&old);
            }
        }
    }

    fn contains(&self, tag: &T) -> bool {
        self.set.contains(tag)
    }

    fn len(&self) -> usize {
        self.order.len()
    }
}

/// [`Pending::insert`] 의 결과. 어느 갈래든 **슬롯을 잃지 않는다** — 못 넣은 슬롯은 그대로 돌려주므로
/// 부르는 쪽이 반드시 깨운다.
#[derive(Debug)]
pub enum Insert<S> {
    /// 자리를 잡았다.
    Placed,
    /// 같은 번호가 이미 **대기 중**이었다 — 그 번호를 은퇴시키고 양쪽을 다 깨운다.
    ///
    /// 옛 대기자는 [`RequestError::Superseded`](오늘 동작 보존), 새 요청은
    /// [`RequestError::TagRetired`] 다. 새 것을 그대로 승계시키면 안 되는 이유: 상관 키가 이 번호
    /// 하나뿐이라 **먼저 나간 요청의 늦은 답장이 나중 요청의 짝으로 배달된다.** 어느 쪽 답장인지
    /// 구별할 재료가 wire 에 없으므로, 잘못 배달하느니 둘 다 실패시킨다.
    Collided { displaced: S, rejected: S },
    /// 만료됐던 번호를 다시 썼다.
    Retired(S),
}

#[derive(Debug)]
struct Waiter<S> {
    slot: S,
    deadline: Instant,
}

/// 짝짓기 번호 → 대기 슬롯.
///
/// ## ★알려진 한계 — 늦은 답장의 오배달을 **불가능하게 만들지 못한다**★ (고치려 들지 말 것)
///
/// 소비자가 `Tag` 를 **되쓰는** 타입으로 고르면 이 crate 는 늦은 답장이 엉뚱한 요청에 배달되는 것을
/// 막을 수 없고, **막으려 하지도 않는다.** 기제와 상한은 이렇다:
///
/// - 시한 만료로 걷힌 번호는 [`Pending`] 이 **유계 링**에 넣어 두고, 다시 오면
///   [`RequestError::TagRetired`] 로 거절한다. 링 길이 = [`crate::Policy::retired_tags`](기본 256).
/// - 링은 **FIFO** 다. 그 뒤로 256개가 더 은퇴하면 가장 오래된 번호는 **잊힌다**.
/// - ★잊힌 뒤에 그 번호의 늦은 답장이 도착하면, 같은 번호를 쓴 **새 요청**의 짝으로 배달된다.★
///   상관 키가 그 번호 하나뿐이라 두 답장을 가를 재료가 wire 에 없다.
///
/// ★**잊는 것이 결함이 아니라 이 링이 하는 일의 절반이다**★ — 잊지 않으면 그 번호를 **정당하게**
/// 다시 쓰는 것도 영구히 막힌다. 즉 「오배달 완전 차단」과 「정당한 재사용 허용」은 같은 손잡이의
/// 양 끝이고, 상한 없는 기억은 애초에 불가능하다(무한히 자란다).
///
/// ★두 리뷰어가 여기서 갈렸고 그 갈림 자체가 기록이다★ — 한쪽은 「누수 없음, 잊는 것이 정당한
/// 재사용을 가능케 한다」, 다른 쪽은 「그 잊음이 원래 답장을 새 대기자에게 보낸다」로 읽었다. 둘 다
/// 맞고, **유계 링을 지금 모양 그대로 두는 것이 사용자 결정이다**(2026-09-06). 근거: 우리 실제
/// `Tag` 는 UUID 라 **정당한 충돌이 애초에 안 난다**. 남는 구멍은 ★소비자 버그(번호 되쓰기)★와
/// ★링 하나를 가득 채울 만큼(기본 256회) 더 은퇴한 뒤에야 도착하는 답장★이 **동시에** 필요하다.
/// 회귀망 = `the_retired_ring_forgets_its_oldest_when_it_overflows`(그 잊음을 **의도로** 못 박는다).
#[derive(Debug)]
pub struct Pending<T, S> {
    waiting: HashMap<T, Waiter<S>>,
    retired: RetiredRing<T>,
}

impl<T: Eq + Hash + Clone, S> Pending<T, S> {
    pub fn new(retired_cap: usize) -> Self {
        Self {
            waiting: HashMap::new(),
            retired: RetiredRing::new(retired_cap),
        }
    }

    /// 슬롯을 등록한다. ★보내기 **전에** 부른다★ — 뒤집으면 빠른 답장이 슬롯을 못 찾는다.
    pub fn insert(&mut self, tag: T, slot: S, deadline: Instant) -> Insert<S> {
        if self.retired.contains(&tag) {
            return Insert::Retired(slot);
        }
        if let Some(old) = self.waiting.remove(&tag) {
            self.retired.insert(tag);
            return Insert::Collided {
                displaced: old.slot,
                rejected: slot,
            };
        }
        self.waiting.insert(tag, Waiter { slot, deadline });
        Insert::Placed
    }

    /// 답장이 왔다. 슬롯이 없으면 `None` — ★만료 뒤 늦게 온 답장이 그렇게 조용히 버려진다★.
    pub fn take(&mut self, tag: &T) -> Option<S> {
        self.waiting.remove(tag).map(|w| w.slot)
    }

    /// 시한이 지난 것들을 걷는다. 걷힌 번호는 링에 등재되어 이후 재사용이 거절된다.
    pub fn expire(&mut self, now: Instant) -> Vec<(T, S)> {
        let due: Vec<T> = self
            .waiting
            .iter()
            .filter(|(_, w)| w.deadline <= now)
            .map(|(t, _)| t.clone())
            .collect();
        due.into_iter()
            .filter_map(|tag| {
                let waiter = self.waiting.remove(&tag)?;
                self.retired.insert(tag.clone());
                Some((tag, waiter.slot))
            })
            .collect()
    }

    /// 끊겼다 — 대기자 전원을 걷는다. ★이쪽은 링에 등재하지 않는다★: 그 번호가 죽은 것이 아니라 연결이
    /// 죽은 것이라, 소비자가 같은 번호로 다시 청하는 것이 정당하다.
    pub fn drain(&mut self) -> Vec<(T, S)> {
        self.waiting.drain().map(|(t, w)| (t, w.slot)).collect()
    }

    /// 가장 이른 시한. `None` 이면 기다리는 것이 없다.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.waiting.values().map(|w| w.deadline).min()
    }

    pub fn contains(&self, tag: &T) -> bool {
        self.waiting.contains_key(tag)
    }

    pub fn is_retired(&self, tag: &T) -> bool {
        self.retired.contains(tag)
    }

    pub fn len(&self) -> usize {
        self.waiting.len()
    }

    pub fn is_empty(&self) -> bool {
        self.waiting.is_empty()
    }

    pub fn retired_len(&self) -> usize {
        self.retired.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Pending<u32, &'static str> {
        Pending::new(4)
    }

    #[test]
    fn a_reply_wakes_exactly_its_own_slot() {
        let mut p = table();
        let t0 = Instant::now();
        p.insert(1, "a", t0 + Duration::from_secs(8));
        p.insert(2, "b", t0 + Duration::from_secs(8));
        assert_eq!(p.take(&1), Some("a"));
        assert_eq!(p.take(&1), None);
        assert_eq!(p.len(), 1);
        assert!(p.contains(&2));
    }

    #[test]
    fn only_the_expired_slot_is_woken() {
        let mut p = table();
        let t0 = Instant::now();
        p.insert(1, "early", t0 + Duration::from_secs(8));
        p.insert(2, "late", t0 + Duration::from_secs(30));

        let fired = p.expire(t0 + Duration::from_secs(8));
        assert_eq!(fired, vec![(1, "early")]);
        assert_eq!(p.len(), 1);
        assert!(p.contains(&2));
    }

    #[test]
    fn nothing_expires_before_its_deadline() {
        let mut p = table();
        let t0 = Instant::now();
        p.insert(1, "a", t0 + Duration::from_secs(8));
        assert!(p.expire(t0 + Duration::from_millis(7999)).is_empty());
        assert_eq!(p.len(), 1);
    }

    #[test]
    fn a_reply_that_arrives_after_the_timeout_finds_no_slot() {
        let mut p = table();
        let t0 = Instant::now();
        p.insert(7, "a", t0 + Duration::from_secs(8));
        p.expire(t0 + Duration::from_secs(8));
        assert_eq!(p.take(&7), None);
    }

    #[test]
    fn an_expired_tag_may_not_be_used_again() {
        let mut p = table();
        let t0 = Instant::now();
        p.insert(7, "a", t0 + Duration::from_secs(8));
        p.expire(t0 + Duration::from_secs(8));
        assert!(p.is_retired(&7));

        match p.insert(7, "b", t0 + Duration::from_secs(20)) {
            Insert::Retired(slot) => assert_eq!(slot, "b", "슬롯을 돌려줘야 호출자가 깨울 수 있다"),
            other => panic!("만료된 번호가 그대로 다시 들어갔다: {other:?}"),
        }
        assert!(p.is_empty());
    }

    // ── M1: 상관 키가 하나뿐이라 겹친 번호는 배달을 못 가른다 ──
    #[test]
    fn a_colliding_tag_wakes_both_sides_and_retires_the_number() {
        let mut p = table();
        let t0 = Instant::now();
        p.insert(9, "old", t0 + Duration::from_secs(8));
        match p.insert(9, "new", t0 + Duration::from_secs(8)) {
            Insert::Collided {
                displaced,
                rejected,
            } => {
                assert_eq!(displaced, "old");
                assert_eq!(rejected, "new");
            }
            other => panic!("{other:?}"),
        }
        assert!(p.is_empty(), "어느 쪽도 표에 남지 않는다");
        assert!(p.is_retired(&9));
    }

    #[test]
    fn a_reply_for_a_collided_tag_reaches_nobody() {
        let mut p = table();
        let t0 = Instant::now();
        p.insert(9, "first", t0 + Duration::from_secs(8));
        p.insert(9, "second", t0 + Duration::from_secs(8));
        assert_eq!(
            p.take(&9),
            None,
            "먼저 나간 요청의 늦은 답장이 나중 요청의 짝으로 배달되면 안 된다"
        );
    }

    #[test]
    fn a_disconnect_takes_everyone_and_retires_nobody() {
        let mut p = table();
        let t0 = Instant::now();
        p.insert(1, "a", t0 + Duration::from_secs(8));
        p.insert(2, "b", t0 + Duration::from_secs(8));

        let mut drained = p.drain();
        drained.sort();
        assert_eq!(drained, vec![(1, "a"), (2, "b")]);
        assert!(p.is_empty());
        assert!(!p.is_retired(&1));
        assert!(matches!(
            p.insert(1, "again", t0 + Duration::from_secs(8)),
            Insert::Placed
        ));
    }

    #[test]
    fn the_retired_ring_forgets_its_oldest_when_it_overflows() {
        let mut p: Pending<u32, &'static str> = Pending::new(2);
        let t0 = Instant::now();
        for tag in 1..=3u32 {
            p.insert(tag, "x", t0);
            p.expire(t0);
        }
        assert_eq!(p.retired_len(), 2);
        assert!(!p.is_retired(&1), "가장 오래된 것은 조용히 잊는다");
        assert!(p.is_retired(&2));
        assert!(p.is_retired(&3));
        assert!(
            matches!(
                p.insert(1, "reused", t0 + Duration::from_secs(8)),
                Insert::Placed
            ),
            "잊힌 번호는 다시 쓸 수 있게 된다 — 이 링은 보증이 아니다"
        );
    }

    #[test]
    fn a_zero_length_ring_retires_nothing() {
        let mut p: Pending<u32, &'static str> = Pending::new(0);
        let t0 = Instant::now();
        p.insert(1, "x", t0);
        p.expire(t0);
        assert!(!p.is_retired(&1));
        assert_eq!(p.retired_len(), 0);
    }

    #[test]
    fn next_deadline_is_the_earliest_one() {
        let mut p = table();
        let t0 = Instant::now();
        assert_eq!(p.next_deadline(), None);
        p.insert(1, "a", t0 + Duration::from_secs(30));
        p.insert(2, "b", t0 + Duration::from_secs(8));
        assert_eq!(p.next_deadline(), Some(t0 + Duration::from_secs(8)));
        p.take(&2);
        assert_eq!(p.next_deadline(), Some(t0 + Duration::from_secs(30)));
    }

    #[test]
    fn request_errors_say_what_happened() {
        assert!(RequestError::TimedOut {
            after: Duration::from_secs(8)
        }
        .to_string()
        .contains('8'));
        assert!(RequestError::TagRetired.to_string().contains("retired"));
        assert!(RequestError::NotARequest.to_string().contains("reply"));
        assert!(RequestError::Disconnected {
            generation: Generation(3),
            cause: DisconnectCause::Idle
        }
        .to_string()
        .contains('3'));
    }
}
