//! 받은 것의 바이트 셈 — ★구동 층★(TRD 3 §3-0 ① · ADR-0310 결정 17).
//!
//! 감독 하나에 셈 하나([`Ledger`])다. 읽기 태스크가 통로에서 프레임을 읽는 순간 그 페이로드 길이를 더하고
//! ([`Ledger::charge`]), 그 몫([`Charge`])은 항목을 따라 앞잡이 줄 → 순서 칸 → 위로 올라가는 줄을 건너가다(그
//! 사이 이어받기 창에 붙들릴 수도 있다) 떨어지며 뺀다 — 받는 쪽이 꺼낼 때, 또는 감독이 버릴 때(답장 · 겹친 조각
//! · 답이 와 창에서 버린 조각). ★그래서 세 층과 창을 셈 하나가 덮는다★ — 층마다 따로 재면 각 층이 자기 상한까지
//! 채워 합이 곱으로 커진다(앞잡이 512칸 × 한 메시지 상한이 그 모양이다).
//!
//! ★몫은 항목에 실리고 층에 실리지 않는다★ — 항목이 앞잡이 줄 · 순서 칸 · 수신단([`crate::Inbox`])을 건너가는
//! 동안 자기를 넣은 감독의 셈을 쥐고 가야 세 층을 셈 하나가 덮는다. 수신단을 떨어뜨리면 든 항목과 함께 몫도
//! 떨어진다.
//!
//! ★셈 안에서 창 몫을 따로 센다★ — 이어받기 창이 붙든 스트림 조각의 바이트다. 감독이 붙들 때 더하고
//! ([`Charge::hold`]) 창에서 내보내거나 버릴 때 뺀다. 셈 전체(물높이)는 그대로다. 예산 B 를 읽기 몫과 창 상한으로
//! 나눠 각자 상한을 건다.
//! - 읽기 문 — 「물높이 − 창 몫」이 읽기 몫 이상이면 읽기 태스크가 통로를 더 안 읽는다. 판정은 「더하기 전
//!   물높이」(net `CONN_RX_MAX_BYTES` 와 같은 규칙)라 문이 열려 있으면 크기와 무관하게 한 장은 들어간다.
//!   ★창 몫을 문에서 빼는 것이 요점이다★ — 붙든 조각이 읽기를 세우면 그 구멍을 메울 재생분을 못 읽고, 창이
//!   시한에 닫힌 뒤 늦게 온 재생분은 지나간 번호로 버려진다. 메울 수 있던 구멍이 영구 손실이 된다.
//! - 창 상한 — 창 몫은 상한을 한 장도 넘겨 쥐지 않는다. 넘길 조각은 붙들지 않고 그 창을 잘림으로 닫는다
//!   (`Supervisor::route`).
//!
//! ★나누기는 스트림 조각을 처음 볼 때 시작한다([`Ledger::use_streams`])★ — 그 전에는 읽기 몫 = B · 창 상한 = 0
//! 이고, 그 뒤로는 반씩이다([`split`]). 스트림 층을 안 쓰는 상대(ADR-0310 결정 16 — 두 Wire 의 `stream_mark` 가
//! 늘 `None`)는 창이 붙들 일이 없다 — 처음부터 반씩 나누면 창 몫은 늘 0 인 채 읽기 문만 반으로 준다. `Wire` 에는
//! 스트림을 쓰는지 미리 알리는 칸이 없어서(칸을 더하면 소비자 전부가 깨진다 — `wire.rs`) 처음 본 조각으로 안다.
//!
//! 그래서 셈 전체의 천장 = 읽기 몫 + 창 상한 + 한 메시지 = B + M(TRD 3 §3-0 ①). 셈을 늘리는 것은 읽기 하나뿐이고
//! 읽기는 문이 열렸을 때(그때 물높이 < 읽기 몫 + 창 상한) 한 장씩만 한다 — 창에 넣고 빼는 것은 물높이를 안
//! 바꾸고, 문이 열린 것을 본 뒤 그 한 장을 읽기 전에 창에서 꺼낸 몫이 문을 다시 닫아도 물높이는 그대로다.
//! 나누기가 시작되는 순간에도 그대로다 — 읽기 몫이 B 에서 반으로 줄 뿐이라 문은 닫히는 쪽으로만 움직이고, 그때까지
//! 든 것은 B + M 밑이다. 한 메시지의 상한은 어댑터가 정한다(crate 헤더 「알려진 한계」).
//!
//! ★셈은 자물쇠 하나 아래 바꾼다★ — 문을 여닫는 쪽이 세 태스크다: 읽기 태스크(더해서 닫는다) · 감독(창에 넣어
//! 열고, 꺼내 닫고, 버려 연다) · 받는 쪽(꺼내서 연다). 자물쇠가 문이 닫힌 시각과 닫혔다 다시 열린 구간을 문 상태와
//! 함께 적어([`Ledger::take_shut`]), 감독이 붙듦을 늦게 알아채도 그것이 시작된 시각을 잃지 않고, 감독이 쓰기를
//! 기다리던 사이 문이 닫혔다 다시 열렸어도 그 시간을 잃지 않는다.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use tokio::sync::Notify;

use crate::clock::Clock;

/// 스트림을 쓰는 감독의 예산 B 를 (읽기 몫, 창 상한)으로 나눈다. 읽기 몫은 0 이 되지 않는다 — 0 이면 문이 영영
/// 안 열린다.
///
/// 반씩인 까닭 = 둘의 합이 B 라야 셈 전체의 천장이 B + M 이고(ADR-0310 결정 17), 반씩이 어느 쪽도 굶기지 않는
/// 가장 단순한 값이다(메인이 정한 내부 값 — 실측 근거는 없다).
pub(crate) const fn split(budget: usize) -> (usize, usize) {
    let read = if budget / 2 == 0 { 1 } else { budget / 2 };
    (read, budget.saturating_sub(read))
}

/// 감독 하나가 받아 쥔 페이로드 바이트의 셈 — 창 몫을 그 안에 따로 센다(이 파일 헤더).
pub(crate) struct Ledger {
    counts: Mutex<Counts>,
    /// 예산 B — 읽기 몫과 창 상한은 [`Ledger::caps`] 가 스트림을 쓰는지에 따라 낸다.
    budget: usize,
    /// 문이 닫힌 시각을 적는 시계 — 감독과 같은 시계라야 감독의 시한 계산과 맞는다.
    clock: Arc<dyn Clock>,
    /// 읽기 문이 열렸다 — 읽기 태스크 몫.
    reader_room: Notify,
    /// 같은 사실의 감독 몫. ★깨울 손잡이를 둘로 가른다★ — `notify_one` 은 기다리는 쪽 하나만 깨우고 남는
    /// 표는 하나만 쥐므로, 둘이 한 손잡이를 나눠 쓰면 한쪽이 상대 몫을 먹고 다른 쪽은 안 깨어난다.
    holder_room: Notify,
}

struct Counts {
    /// 셈 전체(물높이) — 창 몫 포함.
    level: usize,
    held: usize,
    /// 스트림 조각을 봤나 — 그 뒤로는 예산을 반씩 나눈다([`Ledger::use_streams`]).
    streams: bool,
    /// 읽기 문이 닫혀 있으면 닫힌 시각.
    shut_at: Option<Instant>,
    /// 지난 [`Ledger::take_shut`] 뒤 닫혔다 다시 열린 구간들.
    closed: Closed,
}

/// 읽기 문이 닫혔다 다시 열린 구간들의 요약. 구간은 서로 겹치지 않고 시각 순이다.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Closed {
    /// 길이의 합.
    pub total: Duration,
    /// 첫 구간이 닫힌 시각. 구간이 없으면 `None`.
    pub from: Option<Instant>,
    /// 마지막 구간이 열린 시각. 구간이 없으면 `None`.
    pub until: Option<Instant>,
}

impl Closed {
    fn add(&mut self, from: Instant, until: Instant) {
        self.total = self
            .total
            .saturating_add(until.saturating_duration_since(from));
        self.from.get_or_insert(from);
        self.until = Some(until);
    }
}

/// [`Ledger::take_shut`] 의 답 — 지난번 물은 뒤 읽기 문이 닫혀 있던 시간.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Shut {
    /// 지금 닫혀 있으면 닫힌 시각.
    pub since: Option<Instant>,
    /// 그 사이 닫혔다 다시 열린 구간들 — 다 `since` 보다 앞이다.
    pub closed: Closed,
}

impl Ledger {
    pub(crate) fn new(budget: usize, clock: Arc<dyn Clock>) -> Arc<Self> {
        Arc::new(Self {
            counts: Mutex::new(Counts {
                level: 0,
                held: 0,
                streams: false,
                shut_at: None,
                closed: Closed::default(),
            }),
            budget,
            clock,
            reader_room: Notify::new(),
            holder_room: Notify::new(),
        })
    }

    /// (읽기 몫, 창 상한) — 스트림을 쓰기 전에는 예산 전부가 읽기 몫이다(이 파일 헤더). 어느 쪽이든 합은 예산
    /// 이하라 천장 B + M 이 선다(예산 0 이면 읽기 몫 1 — 문이 영영 안 열리지 않게).
    fn caps(&self, streams: bool) -> (usize, usize) {
        if streams {
            split(self.budget)
        } else {
            (self.budget.max(1), 0)
        }
    }

    // 자물쇠 안에서는 덧셈 · 뺄셈 · 시각 읽기뿐이라 오염돼도 셈은 온전하다.
    fn counts(&self) -> MutexGuard<'_, Counts> {
        self.counts.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// 셈 전체 — 창 몫 포함.
    #[cfg(test)]
    pub(crate) fn level(&self) -> usize {
        self.counts().level
    }

    /// 그중 이어받기 창이 붙든 몫.
    #[cfg(test)]
    pub(crate) fn held(&self) -> usize {
        self.counts().held
    }

    /// 읽기 문이 닫혔나 — 읽기 태스크가 통로를 안 읽는 조건이다.
    pub(crate) fn over(&self) -> bool {
        self.counts().shut_at.is_some()
    }

    /// 읽기 문이 지금 닫혀 있으면 닫힌 시각과, 지난번 부른 뒤 닫혔다 다시 열린 구간들을 돌려주고 그 구간들은
    /// 잊는다(시각은 이 셈의 시계로). 감독이 붙든 시간을 셀 때 부른다(`Supervisor::track_pause`) — 부르는 쪽이
    /// 둘이면 한쪽이 가져간 구간을 다른 쪽은 못 본다.
    pub(crate) fn take_shut(&self) -> Shut {
        let mut c = self.counts();
        Shut {
            since: c.shut_at,
            closed: std::mem::take(&mut c.closed),
        }
    }

    /// 스트림 조각을 봤다 — 이 뒤로는 예산을 읽기 몫과 창 상한으로 반씩 나눈다(이 파일 헤더). 되돌리지 않는다.
    /// 읽기 몫이 주는 쪽이라 문이 닫힐 수는 있어도 열리지는 않는다.
    pub(crate) fn use_streams(&self) {
        self.update(|c| c.streams = true);
    }

    /// 이 바이트를 창에 더 붙들 수 있나 — 창 상한 판정이다. 읽기 문과 달리 「더한 뒤」로 잰다 — 창 몫은 한
    /// 장도 상한을 넘겨 쥐지 않는다. [`Ledger::use_streams`] 전에는 상한이 0 이라 늘 거짓이다.
    pub(crate) fn window_fits(&self, bytes: usize) -> bool {
        let c = self.counts();
        c.held.saturating_add(bytes) <= self.caps(c.streams).1
    }

    pub(crate) fn charge(self: &Arc<Self>, bytes: usize) -> Charge {
        if bytes == 0 {
            return Charge::none();
        }
        self.update(|c| c.level += bytes);
        Charge {
            ledger: Some(self.clone()),
            bytes,
            held: false,
        }
    }

    /// 읽기 문이 열릴 때까지 기다린다 — 읽기 태스크가 다음 읽기 앞에서 부른다.
    pub(crate) async fn reader_room(&self) {
        while self.over() {
            self.reader_room.notified().await;
        }
    }

    /// 같은 기다림의 감독 몫. ★취소 안전 — `select!` 팔에 건다★: 폴링마다 문부터 본다.
    pub(crate) async fn holder_room(&self) {
        while self.over() {
            self.holder_room.notified().await;
        }
    }

    fn update(&self, change: impl FnOnce(&mut Counts)) {
        let opened = {
            let mut c = self.counts();
            change(&mut c);
            let shut = c.level - c.held >= self.caps(c.streams).0;
            match (c.shut_at, shut) {
                (None, true) => {
                    c.shut_at = Some(self.clock.now());
                    false
                }
                (Some(since), false) => {
                    c.shut_at = None;
                    c.closed.add(since, self.clock.now());
                    true
                }
                _ => false,
            }
        };
        // ★문이 열릴 때마다 깨운다★ — 깨어난 쪽은 문을 다시 보고 닫혀 있으면 다시 잔다(위 `while`). 그래서
        //   자는 동안 문이 몇 번 여닫혀도 남는 표 하나로 충분하고, 기다리기 전에 열렸으면 그 표가 다음 기다림을
        //   곧바로 끝낸다. 깨우기는 자물쇠 밖이다 — 깨어난 쪽이 곧바로 그 자물쇠를 잡는다.
        if opened {
            self.reader_room.notify_one();
            self.holder_room.notify_one();
        }
    }
}

/// 항목 하나가 쥔 바이트 몫. 떨어질 때 자기 셈에서 뺀다(창 몫이면 창 몫에서도).
pub(crate) struct Charge {
    ledger: Option<Arc<Ledger>>,
    bytes: usize,
    /// 창 몫에 서 있나.
    held: bool,
}

impl Charge {
    /// 바이트를 안 쥔 항목(생애 사건 · 신고)의 몫.
    pub(crate) fn none() -> Self {
        Self {
            ledger: None,
            bytes: 0,
            held: false,
        }
    }

    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    /// 이 몫을 창 몫으로 센다 — 감독이 조각을 이어받기 창에 붙들 때 부른다. 물높이는 그대로이고 읽기 문이
    /// 열릴 수 있다(그러면 깨운다). 이미 창 몫이거나 빈 몫이면 그대로다.
    pub(crate) fn hold(&mut self) {
        if let (Some(ledger), false) = (&self.ledger, self.held) {
            let bytes = self.bytes;
            ledger.update(|c| c.held += bytes);
            self.held = true;
        }
    }

    /// 창 몫에서 뺀다 — 창에서 내보낼 때 부른다. 물높이는 그대로이고 읽기 문이 닫힐 수 있다(그 시각을 적는다).
    pub(crate) fn unhold(&mut self) {
        if let (Some(ledger), true) = (&self.ledger, self.held) {
            let bytes = self.bytes;
            ledger.update(|c| c.held -= bytes);
            self.held = false;
        }
    }
}

impl Drop for Charge {
    fn drop(&mut self) {
        if let Some(ledger) = self.ledger.take() {
            let (bytes, held) = (self.bytes, self.held);
            ledger.update(|c| {
                c.level -= bytes;
                if held {
                    c.held -= bytes;
                }
            });
        }
    }
}

/// 몫을 쥔 채 건너가는 항목.
pub(crate) struct Charged<T> {
    pub item: T,
    pub charge: Charge,
}

impl<T> Charged<T> {
    pub(crate) fn bare(item: T) -> Self {
        Self {
            item,
            charge: Charge::none(),
        }
    }

    /// 꺼낸다 — 몫은 여기서 떨어진다(받는 쪽이 꺼냈다).
    pub(crate) fn into_item(self) -> T {
        let Self { item, charge } = self;
        drop(charge);
        item
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::ManualClock;
    use futures_util::future::poll_immediate;
    use std::time::Duration;

    /// 스트림을 쓰는 예산 20 = 읽기 몫 10 · 창 상한 10.
    fn ledger() -> (Arc<Ledger>, ManualClock) {
        let clock = ManualClock::new();
        let ledger = Ledger::new(20, clock.handle());
        ledger.use_streams();
        (ledger, clock)
    }

    async fn woken(waiter: tokio::task::JoinHandle<()>, what: &str) {
        tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .unwrap_or_else(|_| panic!("문이 열렸는데 {what} 쪽을 안 깨웠다"))
            .expect("기다리던 태스크가 패닉했다");
    }

    #[test]
    fn the_budget_splits_in_half_and_the_read_share_never_hits_zero() {
        assert_eq!(split(16 << 20), (8 << 20, 8 << 20));
        assert_eq!(split(100), (50, 50));
        assert_eq!(split(3), (1, 2));
        assert_eq!(split(1), (1, 0), "읽기 몫이 0 이면 문이 영영 안 열린다");
    }

    #[test]
    fn a_charge_gives_its_bytes_back_when_dropped() {
        let (ledger, _clock) = ledger();
        let a = ledger.charge(4);
        let b = ledger.charge(7);
        assert_eq!(ledger.level(), 11);
        assert!(ledger.over());
        drop(a);
        assert_eq!(ledger.level(), 7);
        assert!(!ledger.over());
        drop(b);
        assert_eq!(ledger.level(), 0);
    }

    #[test]
    fn a_zero_byte_item_holds_no_share_of_the_ledger() {
        let (ledger, _clock) = ledger();
        let none = ledger.charge(0);
        assert_eq!(Arc::strong_count(&ledger), 1, "빈 몫은 셈을 쥐지 않는다");
        drop(none);
        assert_eq!(ledger.level(), 0);
    }

    #[test]
    fn taking_the_item_out_releases_its_share() {
        let (ledger, _clock) = ledger();
        let carried = Charged {
            item: "x",
            charge: ledger.charge(3),
        };
        assert_eq!(ledger.level(), 3);
        assert_eq!(carried.into_item(), "x");
        assert_eq!(ledger.level(), 0);
    }

    // ★두 쪽이 다 잠든 채로 문이 열린다★ — 잠들기 전이면 문 확인이 먼저 풀어 줘서 깨우기를 재지 못한다.
    #[tokio::test]
    async fn opening_the_gate_wakes_both_the_reader_and_the_holder() {
        let (ledger, _clock) = ledger();
        let first = ledger.charge(6);
        let second = ledger.charge(6);
        let reader = {
            let ledger = ledger.clone();
            tokio::spawn(async move { ledger.reader_room().await })
        };
        let holder = {
            let ledger = ledger.clone();
            tokio::spawn(async move { ledger.holder_room().await })
        };
        crate::testing::settle().await;
        assert!(
            !reader.is_finished() && !holder.is_finished(),
            "읽기 몫 이상이면 기다린다"
        );

        drop(first);
        woken(reader, "읽기").await;
        woken(holder, "감독").await;
        drop(second);
    }

    #[test]
    fn the_window_cap_check_counts_the_bytes_about_to_be_added() {
        let (ledger, _clock) = ledger();
        let mut held = ledger.charge(6);
        held.hold();
        assert!(ledger.window_fits(4), "꽉 차는 것까지는 넘는 것이 아니다");
        assert!(!ledger.window_fits(5));
        let _gated = ledger.charge(9);
        assert!(
            ledger.window_fits(4),
            "★창 상한은 창 몫만 잰다★ — 읽기 몫이 찼다고 붙들 자리가 줄지 않는다"
        );
    }

    // ★창 몫으로 센 조각은 읽기 문을 세우지 않는다★ — 붙든 조각이 그 구멍을 메울 재생분의 읽기를 막지 않는다는
    //   것의 실물이다. 물높이는 그대로다(셈 하나).
    #[tokio::test]
    async fn holding_a_charge_takes_it_out_of_the_gate_but_not_out_of_the_level() {
        let (ledger, _clock) = ledger();
        let mut chunk = ledger.charge(12);
        let reader = {
            let ledger = ledger.clone();
            tokio::spawn(async move { ledger.reader_room().await })
        };
        crate::testing::settle().await;
        assert!(!reader.is_finished(), "읽기 몫 이상이면 기다린다");

        chunk.hold();
        assert_eq!((ledger.level(), ledger.held()), (12, 12));
        assert!(!ledger.over());
        woken(reader, "읽기").await;

        chunk.hold();
        assert_eq!(
            (ledger.level(), ledger.held()),
            (12, 12),
            "이미 창 몫이면 그대로다"
        );
        chunk.unhold();
        assert_eq!((ledger.level(), ledger.held()), (12, 0));
        assert!(ledger.over(), "창에서 나온 몫은 다시 문을 세운다");
        chunk.hold();
        drop(chunk);
        assert_eq!(
            (ledger.level(), ledger.held()),
            (0, 0),
            "창 몫인 채 버려지면 둘 다에서 빠진다"
        );
    }

    #[test]
    fn the_gate_remembers_when_it_shut_until_it_opens() {
        let (ledger, clock) = ledger();
        let t0 = clock.now();
        let small = ledger.charge(5);
        assert_eq!(ledger.take_shut().since, None);

        clock.advance(Duration::from_secs(3));
        let big = ledger.charge(6);
        assert_eq!(
            ledger.take_shut().since,
            Some(t0 + Duration::from_secs(3)),
            "★선을 넘은 그 시각이다★ — 감독이 나중에 알아채도 이 시각부터 붙든 것으로 센다"
        );
        clock.advance(Duration::from_secs(1));
        let _more = ledger.charge(1);
        assert_eq!(
            ledger.take_shut().since,
            Some(t0 + Duration::from_secs(3)),
            "닫힌 채 더해도 처음 닫힌 시각 그대로다"
        );

        drop(big);
        assert_eq!(ledger.take_shut().since, None, "열리면 지운다");

        let mut chunk = ledger.charge(4);
        chunk.hold();
        clock.advance(Duration::from_secs(2));
        chunk.unhold();
        assert_eq!(
            ledger.take_shut().since,
            Some(t0 + Duration::from_secs(6)),
            "창에서 꺼내 닫혀도 그 시각을 적는다"
        );
        drop(small);
    }

    // ★닫혔다 다시 열린 구간을 감독이 물을 때까지 모은다★ — 감독이 쓰기를 기다리는 사이 문이 여닫히면 바퀴
    //   첫머리에서는 열린 문만 본다. 그 시간을 잃으면 창 시한으로 세져 메울 수 있던 구멍이 잘림으로 접힌다.
    #[test]
    fn the_gate_adds_up_every_interval_it_was_shut_until_asked() {
        let (ledger, clock) = ledger();
        let at = |secs| clock.now() + Duration::from_secs(secs);
        let (t1, t10, t11, t13) = (at(1), at(10), at(11), at(13));

        clock.advance(Duration::from_secs(1));
        let first = ledger.charge(10);
        clock.advance(Duration::from_secs(2));
        drop(first);
        clock.advance(Duration::from_secs(4));
        let second = ledger.charge(10);
        clock.advance(Duration::from_secs(3));
        drop(second);
        clock.advance(Duration::from_secs(1));
        let third = ledger.charge(10);
        assert_eq!(
            ledger.take_shut(),
            Shut {
                since: Some(t11),
                closed: Closed {
                    total: Duration::from_secs(5),
                    from: Some(t1),
                    until: Some(t10),
                },
            },
            "★두 구간(2 s · 3 s)을 다 센다★ — 지금 닫힌 구간은 시작 시각으로만 알린다"
        );
        assert_eq!(
            ledger.take_shut().closed,
            Closed::default(),
            "한 번 넘긴 구간은 잊는다"
        );

        clock.advance(Duration::from_secs(2));
        drop(third);
        assert_eq!(
            ledger.take_shut(),
            Shut {
                since: None,
                closed: Closed {
                    total: Duration::from_secs(2),
                    from: Some(t11),
                    until: Some(t13),
                },
            },
            "물을 때 열려 있던 구간은 열린 뒤 통째로 넘긴다 — 그 시작을 앞서 `since` 로 본 쪽이 겹치지 않게 가려 쓴다"
        );
    }

    // ★스트림 조각을 보기 전에는 예산 전부가 읽기 몫이고 창 상한은 0 이다★ — 스트림을 안 쓰는 상대(ADR-0310
    //   결정 16)의 읽기 문을 반으로 줄이지 않는다. 조각을 보면 반씩 나누고, 이미 반 넘게 쥐었으면 그 자리에서 닫힌다.
    #[test]
    fn the_whole_budget_is_the_read_share_until_streams_are_used() {
        let clock = ManualClock::new();
        let ledger = Ledger::new(20, clock.handle());
        let below = ledger.charge(19);
        assert!(!ledger.over(), "예산 밑이면 문이 열려 있다");
        assert!(!ledger.window_fits(1), "창 상한 0 — 붙들 자리가 없다");
        let full = ledger.charge(1);
        assert!(ledger.over(), "예산에 닿으면 닫힌다");
        drop(full);

        clock.advance(Duration::from_secs(1));
        ledger.use_streams();
        assert!(ledger.over(), "읽기 몫이 반으로 줄어 그 자리에서 닫힌다");
        assert_eq!(ledger.take_shut().since, Some(clock.now()));
        assert!(ledger.window_fits(10));
        assert!(!ledger.window_fits(11));
        drop(below);
        assert!(!ledger.over());
    }

    #[tokio::test]
    async fn below_the_read_share_there_is_nothing_to_wait_for() {
        let (ledger, _clock) = ledger();
        let _held = ledger.charge(9);
        assert!(poll_immediate(ledger.reader_room()).await.is_some());
        assert!(poll_immediate(ledger.holder_room()).await.is_some());
    }
}
