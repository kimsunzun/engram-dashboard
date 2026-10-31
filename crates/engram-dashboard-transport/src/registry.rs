//! 상대 N 개 명부 — ★구동 층★. 상대마다 감독 하나를, 이름표마다 위로 올라가는 줄 하나를 세우고,
//! [`Inbound`] 가 그 줄들을 돌아가며 읽는다.
//!
//! ★상대는 처음부터 N 개다★(ADR-0177 결정 3 · ADR-0179) — 하나뿐인 세계를 전제한 상태가 어디에도 없다.
//! **상대끼리 상태를 공유하지 않는다**: 데몬 A 가 포기해도 B 는 운영 중이다.
//!
//! ★위로 올라가는 줄도 이름표끼리 공유하지 않는다★ — 줄이 하나면 시끄러운 상대가 그것을 채우는 순간
//! 조용한 상대의 사건·패킷이 그 뒤에 붙들리고, 「A 하나 죽어도 B·C 는 잘 동작해야 한다」가 무너진다
//! (ADR-0180 결정 8). 그래서 줄은 이름표마다 따로 서고(길이 = [`Policy::inbound_queue`]), 줄이 차면 그
//! 이름표의 감독이 자기 순서 칸에 붙들고 그 상대의 읽기를 세운다 — 버리지 않고 연결을 끊지도 않는다(`peer.rs`
//! 헤더). 그래서 받는 쪽이 밀리면 그 상대만 밀린다. 같은 이름표를 다시 올리면 그 줄을 이어 쓴다
//! ([`Registry::add_with_policy`]).
//! 줄의 넣는 끝과 받는 끝은 명부와 [`Inbound`] 가 함께 쥐는 표(`Roster`) 한 곳에 산다 — 줄의 수명 주인이
//! 하나라야 받는 쪽이 안 읽는 동안 어떤 올리기 · 내리기 순서도 줄을 버려 두지 못한다.
//! TODO(TRD 3 §6-0 U1d): 받는 끝을 손잡이마다 `add` 가 소비자에게 직접 돌려준다(ADR-0310 결정 17 —
//! D24 ②). 그러면 [`Inbound`] 의 돌아가며 읽기 · 함께 쥐는 표의 받는 끝 · 이름표별 줄 이어 쓰기가 이
//! crate 에서 빠진다 — 합친 보기는 소비자 어댑터 몫이다.
//!
//! ★「앱 전체가 끊겼나」는 여기서 판정하지 않는다★ — 그건 화면의 개념이고 이 crate 는 상대를 셀 뿐
//! 앱을 모른다. 재료 둘([`Registry::peers`] 와 [`crate::Peer::reconnect_now`])을 주고 셸이 접는다
//! (ADR-0180 의 열린 질문이 그렇게 닫힌다).

use std::collections::HashMap;
use std::future::poll_fn;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll, Waker};

use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TryRecvError;

use crate::budget::Charged;
use crate::clock::Clock;
use crate::event::{Direction, Incoming, PeerId, TransportEvent};
use crate::link::{Address, Dialer, Handshake};
use crate::machine::PeerState;
use crate::peer::{spawn_peer, Peer, PeerConfig, BYTE_BUDGET};
use crate::policy::Policy;
use crate::wire::Wire;

enum Probe<T> {
    Item(T),
    Empty,
    Ended,
}

/// 이름표 하나의 위로 올라가는 줄의 받는 끝.
///
/// 항목마다 그것을 넣은 감독의 바이트 몫이 실려 있고 꺼낼 때 떨어진다(`budget.rs`). ★셈을 줄 단위로 두지
/// 않는다★ — 한 줄을 같은 이름표의 연이은 감독이 함께 쓰므로 줄 단위로 재면 옛 감독의 꼬리가 새 감독의
/// 예산을 먹는다(TRD 3 §3-0 ①). ★몫은 자물쇠 밖에서 떨어뜨린다★(`Inbound::try_recv`).
struct Lane<W: Wire> {
    rx: mpsc::Receiver<Charged<Incoming<W>>>,
}

impl<W: Wire> Lane<W> {
    fn poll_next(&mut self, cx: &mut Context<'_>) -> Probe<Charged<Incoming<W>>> {
        match self.rx.poll_recv(cx) {
            Poll::Ready(Some(item)) => Probe::Item(item),
            Poll::Ready(None) => Probe::Ended,
            Poll::Pending => Probe::Empty,
        }
    }

    fn try_next(&mut self) -> Probe<Charged<Incoming<W>>> {
        match self.rx.try_recv() {
            Ok(item) => Probe::Item(item),
            Err(TryRecvError::Empty) => Probe::Empty,
            Err(TryRecvError::Disconnected) => Probe::Ended,
        }
    }

    fn close(&mut self) {
        self.rx.close();
    }
}

/// 이름표 하나의 줄 — 넣는 끝과 받는 끝을 한 자리에 둔다.
///
/// ★두 끝의 수명 주인은 [`Roster`] 하나다★ — 넣는 끝은 명부가, 받는 끝은 받는 쪽이 따로 쥐면 「걷었다」를
/// 서로 알릴 길이 없어, 받는 쪽이 안 읽는 동안 같은 이름표를 올렸다 내리기를 되풀이하면 버려진 받는 끝이
/// 하나씩 쌓인다. 줄은 `Roster::release` 한 곳에서만 사라진다(받는 쪽이 사라질 때의 일괄 놓기는 예외).
struct Slot<W: Wire> {
    /// 명부의 넣는 끝 — 같은 이름표를 다시 올릴 때 새 감독에게 이어 주고, [`Registry::broadcast`] 가 자기
    /// 유실 신고를 넣는다. `None` = 명부가 통째로 사라졌다(그 뒤로는 감독들이 다 놓으면 줄이 끝난다).
    /// TODO(TRD 3 §6-0 U1d): 받는 끝이 소비자에게 가면 이 강한 손잡이는 「`Closed` 가 마지막 → 송신단 놓기
    /// → `None`」(TRD 3 §3-0 ② · 불변식 5)을 깬다 — 받는 끝을 손잡이마다 따로 주고, 신고는 감독 쪽 셈으로
    /// 돌린다.
    tx: Option<mpsc::Sender<Charged<Incoming<W>>>>,
    lane: Lane<W>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Consumer {
    Open,
    Closed,
    Gone,
}

/// 명부와 받는 쪽이 함께 쥐는 표.
struct Roster<W: Wire> {
    /// 올라 있는 이름표의 손잡이.
    peers: HashMap<PeerId, Peer<W>>,
    lanes: HashMap<PeerId, Slot<W>>,
    /// 돌아가며 읽는 차례. `lanes` 의 열쇠와 늘 같은 집합이다.
    order: Vec<PeerId>,
    next: usize,
    consumer: Consumer,
    registry_gone: bool,
    /// 받는 쪽이 기다리며 남긴 깨울 손잡이. ★새 줄은 이것을 모른다★ — 아직 한 번도 안 찔려 그 줄에 깨울
    /// 손잡이가 안 걸려 있으므로, 줄을 세우거나 명부가 사라질 때 이것으로 깨운다(`wake_pending`).
    waker: Option<Waker>,
    wake_pending: bool,
    /// 자물쇠 안에서 놓은 줄. 놓기는 자물쇠를 놓은 뒤에 한다(`Shared::with`).
    released: Vec<Slot<W>>,
}

impl<W: Wire> Roster<W> {
    /// 이 차례의 줄을 걷어도 되나 — 더 들어올 길이 없고 다 꺼냈다.
    fn finished(&self, at: usize) -> bool {
        let id = &self.order[at];
        let slot = &self.lanes[id];
        // 받는 쪽이 닫은 줄은 새 자리를 못 잡지만, ★닫기 전에 잡아 둔 자리는 닫힌 뒤에도 실린다★(외부 사실 —
        //   tokio 의 넣기는 자리를 잡고 나서 싣는다). 그래서 「비었다」가 아니라 든 것도 잡힌 자리도 없을 때
        //   끝이다 — 넣는 끝의 남은 자리가 꽉 찬 것이 그 둘을 함께 잰다(tokio 가 받는 쪽 끝을 알리는 조건과
        //   같다). 명부가 사라져 그 끝이 없으면 받는 끝이 `Ended` 로 알린다.
        if slot.lane.rx.is_closed() {
            return match &slot.tx {
                Some(tx) => tx.capacity() == tx.max_capacity(),
                None => false,
            };
        }
        if self.peers.contains_key(id) {
            return false;
        }
        match &slot.tx {
            // ★넣는 쪽 수를 먼저, 비었나를 나중에 본다★ — 거꾸로 보면 그 사이 감독이 마지막 것을 넣고 놓았을
            //   때 비었다고 읽은 줄을 걷어 그것을 잃는다. 넣는 쪽이 명부의 끝 하나뿐이면 새 넣기가 생길 수
            //   없고(명부가 끝을 빌려주는 것도 이 자물쇠 안이다), 잡힌 자리도 남을 수 없다 — 자리는 넣는 끝을
            //   쥔 채로만 잡고 싣는다.
            Some(tx) => tx.strong_count() == 1 && slot.lane.rx.is_empty(),
            // 명부가 사라진 뒤에는 감독들이 다 놓을 때 받는 끝이 `Ended` 로 알린다.
            None => false,
        }
    }

    fn release(&mut self, at: usize) {
        let id = self.order.remove(at);
        if at < self.next {
            self.next -= 1;
        }
        if let Some(slot) = self.lanes.remove(&id) {
            self.released.push(slot);
        }
    }

    fn release_finished(&mut self) {
        let mut at = 0;
        while at < self.order.len() {
            if self.finished(at) {
                self.release(at);
            } else {
                at += 1;
            }
        }
    }

    /// 지난번에 꺼낸 줄 다음부터 한 바퀴 돈다. 끝난 줄은 그 자리에서 걷는다.
    ///
    /// ★`Empty` 인 줄도 전부 한 번씩 찔러야 한다★ — `poll_next` 판에서는 그 찌름이 깨울 손잡이를 그 줄에
    /// 거는 일이라, 하나라도 건너뛰면 그 줄에 든 것이 다음 깨움까지 안 보인다.
    fn rotate(
        &mut self,
        mut probe: impl FnMut(&mut Lane<W>) -> Probe<Charged<Incoming<W>>>,
    ) -> Option<Charged<Incoming<W>>> {
        let mut at = self.next;
        let mut empty = 0;
        while empty < self.order.len() {
            if at >= self.order.len() {
                at = 0;
            }
            let slot = self
                .lanes
                .get_mut(&self.order[at])
                .expect("order 와 lanes 는 함께 바뀐다");
            match probe(&mut slot.lane) {
                Probe::Item(item) => {
                    self.next = at + 1;
                    return Some(item);
                }
                Probe::Ended => self.release(at),
                Probe::Empty if self.finished(at) => self.release(at),
                Probe::Empty => {
                    at += 1;
                    empty += 1;
                }
            }
        }
        None
    }

    /// 이 이름표의 줄에 넣는 끝 — 줄이 서 있으면 이어 쓰고, 없으면 `capacity` 길이로 새로 세운다.
    fn lane_for(&mut self, id: &PeerId, capacity: usize) -> mpsc::Sender<Charged<Incoming<W>>> {
        if let Some(tx) = self.lanes.get(id).and_then(|slot| slot.tx.clone()) {
            return tx;
        }
        debug_assert!(
            !self.lanes.contains_key(id),
            "명부가 살아 있는 동안 tx 는 늘 있다"
        );
        let (tx, rx) = mpsc::channel(capacity);
        self.lanes.insert(
            id.clone(),
            Slot {
                tx: Some(tx.clone()),
                lane: Lane { rx },
            },
        );
        self.order.push(id.clone());
        self.wake_pending = true;
        tx
    }
}

/// 명부와 받는 쪽이 함께 쥐는 몫.
struct Shared<W: Wire> {
    roster: Mutex<Roster<W>>,
}

impl<W: Wire> Shared<W> {
    /// 표를 다룬다. ★자물쇠 안에서는 기다리지 않고, 놓기 · 깨우기는 자물쇠를 놓은 뒤에 한다★ — 감독 띄우기도
    /// 자물쇠 밖이다(`Registry::add_with_policy`).
    ///
    /// 독 걸린 자물쇠를 그대로 다시 쓴다 — 자물쇠 안의 갱신은 패닉할 자리 없이 한 칸씩이라 반쯤 바뀐 표가
    /// 남지 않고, 독을 그대로 두면 한 번의 패닉이 명부를 영영 못 쓰게 만든다.
    fn with<R>(&self, f: impl FnOnce(&mut Roster<W>) -> R) -> R {
        let mut roster = self.roster.lock().unwrap_or_else(PoisonError::into_inner);
        let out = f(&mut roster);
        let released = std::mem::take(&mut roster.released);
        let wake = if std::mem::take(&mut roster.wake_pending) {
            roster.waker.take()
        } else {
            None
        };
        drop(roster);
        drop(released);
        if let Some(waker) = wake {
            waker.wake();
        }
        out
    }
}

/// 이름표별 줄들의 받는 쪽. ★명부 하나당 하나뿐이다★ — 갈라 쓰려면 소비자가 갈라 쓴다.
///
/// ★한 번 꺼낼 때마다 다음 줄로 넘어간다★ — 그래서 한 상대에게 아무리 많이 밀려 있어도 다른 상대의
/// 다음 항목은 표에 선 줄의 수만큼의 꺼냄 안에 나온다. 그 수에는 명부에서 내렸거나 닫힌 상대의, 아직
/// 다 꺼내지 않은 줄도 든다(같은 이름표를 다시 올리는 것은 줄을 늘리지 않는다).
///
/// ★순서는 이름표 하나 안에서만 계약이다★ — 한 이름표의 줄에서는 넣은 순서대로 오고, 이름표 사이의
/// 순서는 낸 시각이 아니라 돌아가는 차례가 정한다. 같은 이름표를 다시 올리면 옛 감독과 새 감독이 **같은
/// 줄**에 넣는다 — 옛 감독은 닫히는 동안(통로를 닫고 순서 칸에 붙든 것을 다 실을 때까지) 더 돌 수 있어
/// 둘의 것이 넣은 순서대로 섞인다. 옛 감독의 끝은 그 감독의 [`TransportEvent::Closed`] 다. 어느 감독의
/// 것인지는 줄이 가르지 못하고, 연결 세대도 `Connected` · `Disconnected` · `Closed` · `DecodeFailed` ·
/// 메시지([`crate::Arrival::generation`])에만 실린다 — `ConnectFailed` · `Rejected` · `GaveUp` · `Dropped`
/// 등에는 없다.
/// TODO(TRD 3 §6-0 U1d): 받는 끝을 손잡이마다 따로 주고, 소비자가 TRD 3 §3-3 의 세 규칙으로 거른다 —
/// 모든 사건에 손잡이 거름 · 세대를 실은 사건만 세대 비교 · 지금 손잡이의 세대 없는 생애 사건은 통과.
pub struct Inbound<W: Wire> {
    shared: Arc<Shared<W>>,
}

impl<W: Wire> Inbound<W> {
    /// `None` = 더 올라올 것이 없다 — 명부가 통째로 사라졌거나 [`Inbound::close`] 를 불렀고, 남은 줄을 다
    /// 꺼냈다. ★취소 안전★ — `select!` 팔에 걸어 다른 팔이 이겨도 항목을 잃지 않는다.
    pub async fn recv(&mut self) -> Option<Incoming<W>> {
        poll_fn(|cx| self.poll_recv(cx)).await
    }

    /// 기다리지 않고 한 개. 하네스가 지금까지 올라온 것을 걷을 때 쓴다.
    pub fn try_recv(&mut self) -> Option<Incoming<W>> {
        // 바이트 몫은 자물쇠를 놓은 뒤에 떨어뜨린다 — 그 해제가 감독 · 읽기 태스크를 깨울 수 있다(`Shared::with`).
        self.shared
            .with(|roster| roster.rotate(|lane| lane.try_next()))
            .map(Charged::into_item)
    }

    /// 더 받지 않는다. 이미 큐에 있는 것은 [`Inbound::recv`] 로 계속 꺼낼 수 있다. ★감독들이 순서 칸에
    /// 붙들어 아직 줄에 못 든 것은 말없이 버려진다★ — 닫힌 줄에는 실을 수 없고, 세지도 신고하지도 않는다.
    /// [`Inbound`] 를 떨어뜨려도 같다.
    ///
    /// 감독은 다음에 넣으려다 닫힌 것을 보고 다시 붙지 않고 접는다. 이 뒤에 올린 상대는 걸지 않고 곧바로
    /// 접힌다([`Registry::add_with_policy`]).
    pub fn close(&mut self) {
        self.shared.with(|roster| {
            roster.consumer = Consumer::Closed;
            for slot in roster.lanes.values_mut() {
                slot.lane.close();
            }
        });
    }

    fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Option<Incoming<W>>> {
        self.shared
            .with(|roster| {
                if let Some(item) = roster.rotate(|lane| lane.poll_next(cx)) {
                    return Poll::Ready(Some(item));
                }
                let nothing_more = roster.registry_gone || roster.consumer != Consumer::Open;
                if nothing_more && roster.lanes.is_empty() {
                    return Poll::Ready(None);
                }
                if !matches!(&roster.waker, Some(w) if w.will_wake(cx.waker())) {
                    roster.waker = Some(cx.waker().clone());
                }
                Poll::Pending
            })
            // `try_recv` 의 같은 줄 — 몫은 자물쇠 밖에서 떨어진다.
            .map(|item| item.map(Charged::into_item))
    }

    /// 표에 선 줄 수.
    #[cfg(test)]
    fn held_lanes(&self) -> usize {
        self.shared.with(|roster| roster.lanes.len())
    }
}

impl<W: Wire> Drop for Inbound<W> {
    /// 받는 끝을 전부 놓는다 — 감독은 다음에 넣으려다 닫힌 것을 보고 다시 붙지 않고 접는다.
    fn drop(&mut self) {
        self.shared.with(|roster| {
            roster.consumer = Consumer::Gone;
            roster.order.clear();
            roster.next = 0;
            let all: Vec<Slot<W>> = roster.lanes.drain().map(|(_, slot)| slot).collect();
            roster.released.extend(all);
        });
    }
}

struct RegistryInner<W: Wire> {
    wire: Arc<W>,
    dialer: Arc<dyn Dialer>,
    clock: Arc<dyn Clock>,
    policy: Policy,
    shared: Arc<Shared<W>>,
    /// 이름표별 세대 계수기. ★`remove` 로 지우지 않는다★ — 지우면 같은 이름표를 다시 올릴 때 세대가
    /// 0 으로 되감기고, 소비자의 평범한 방어(「최대 세대 미만은 무시」)가 새 연결을 통째로 버린다.
    ///
    /// 이 map 은 **소비자가 쓴 이름표 수**만큼 자란다(상대가 정하는 값이 아니다).
    generations: Mutex<HashMap<PeerId, Arc<AtomicU64>>>,
}

impl<W: Wire> Drop for RegistryInner<W> {
    /// 명부의 손잡이와 넣는 끝을 놓는다 — 그 뒤로는 감독들이 다 끝나 줄을 비우면 [`Inbound::recv`] 가
    /// `None` 을 낸다.
    fn drop(&mut self) {
        let (peers, senders) = self.shared.with(|roster| {
            roster.registry_gone = true;
            roster.wake_pending = true;
            let senders: Vec<_> = roster
                .lanes
                .values_mut()
                .filter_map(|slot| slot.tx.take())
                .collect();
            (std::mem::take(&mut roster.peers), senders)
        });
        drop(peers);
        drop(senders);
    }
}

/// 상대 명부.
pub struct Registry<W: Wire> {
    inner: Arc<RegistryInner<W>>,
}

impl<W: Wire> Clone for Registry<W> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<W: Wire> Registry<W> {
    /// 명부 하나를 세운다. ★인바운드는 돌려준 [`Inbound`] 로만 나온다★.
    pub fn new(
        wire: Arc<W>,
        dialer: Arc<dyn Dialer>,
        clock: Arc<dyn Clock>,
        policy: Policy,
    ) -> (Self, Inbound<W>) {
        debug_assert_eq!(policy.check(), Ok(()), "정책 값이 자기 불변식을 어긴다");
        let shared = Arc::new(Shared {
            roster: Mutex::new(Roster {
                peers: HashMap::new(),
                lanes: HashMap::new(),
                order: Vec::new(),
                next: 0,
                consumer: Consumer::Open,
                registry_gone: false,
                waker: None,
                wake_pending: false,
                released: Vec::new(),
            }),
        });
        let registry = Self {
            inner: Arc::new(RegistryInner {
                wire,
                dialer,
                clock,
                policy,
                shared: shared.clone(),
                generations: Mutex::new(HashMap::new()),
            }),
        };
        (registry, Inbound { shared })
    }

    /// 상대를 올리고 즉시 붙기 시작한다. 같은 이름표가 이미 있으면 **옛 것을 닫고 갈아치운다**.
    ///
    /// ★tokio 런타임 안에서 불러야 한다★ — 감독 태스크를 띄우므로 밖에서 부르면 패닉한다(명부는 그 뒤에도
    /// 쓸 수 있다). 그렇게 패닉해도 같은 이름표로 올라 있던 상대는 이미 명부에서 내려가 닫힌 뒤다.
    pub fn add(&self, id: PeerId, addr: Address, handshake: Arc<dyn Handshake>) -> Peer<W> {
        let policy = self.inner.policy.clone();
        self.add_with_policy(id, addr, handshake, policy)
    }

    /// 이 상대에게만 다른 정책을 건다. 런타임 조건은 [`Registry::add`] 와 같다.
    ///
    /// ★옛 감독을 먼저 닫고, 그 마지막 세대를 새 감독의 바닥값으로 넘긴다★ — 순서를 뒤집으면 두 소켓이
    /// 같은 상대에게 동시에 붙고, 바닥값을 안 넘기면 세대가 0 으로 되감겨 소비자의 평범한 방어
    /// (「최대 세대 미만은 무시」)가 **새 연결을 통째로 버린다**.
    ///
    /// ★그래도 겹침이 0 이 되지는 않는다★ — [`Peer::close`] 에 완료 신호가 없어 옛 감독은 잠시 더
    /// 통로를 닫는 중일 수 있고, 그 사이 운영 단계에 들어서 세대를 하나 더 발행할 수도 있다.
    /// **공용 계수기**가 그 겹침을 해롭지 않게 만드는 장치다 — 붙을 때 뽑는 연결 세대(`Connected` ·
    /// `Disconnected` · 메시지에 실리는 번호)는 두 번 나오지 않는다. ★한 번도 안 붙은 감독의 세대는 예외다★
    /// — 바닥값(옛 감독의 마지막 세대)을 그대로 실어, 그 감독의 [`TransportEvent::Closed`] 와
    /// [`Peer::generation`] 은 옛 감독의 것과 같은 번호를 보인다.
    ///
    /// ★같은 이름표면 위로 올라가는 줄을 새로 세우지 않고 이어 쓴다★ — 올라 있던 것을 갈아치울 때도, 내린
    /// 뒤 그 줄이 아직 안 걷혔을 때도 같다. 그래서 받는 쪽이 안 읽는 동안 다시 올려도 줄이 늘지 않고, 옛
    /// 감독의 꼬리와 새 감독의 것이 한 줄에 넣은 순서대로 남는다. ★줄 길이는 그 줄이 설 때 정해진다★ —
    /// 이어 쓰는 동안은 이 `policy.inbound_queue` 가 달라도 길이가 안 바뀐다.
    ///
    /// [`Inbound`] 이 닫혔거나 사라진 뒤에 올린 상대는 걸지 않고 곧바로 접힌다. 올리는 도중에 닫히면
    /// 접히되, 그 사이 감독이 이미 시작한 걸기는 닫기가 취소한다.
    pub fn add_with_policy(
        &self,
        id: PeerId,
        addr: Address,
        handshake: Arc<dyn Handshake>,
        policy: Policy,
    ) -> Peer<W> {
        debug_assert_eq!(policy.check(), Ok(()), "정책 값이 자기 불변식을 어긴다");
        // ★옛 감독의 마지막 세대를 **읽지** 않는다★ — 읽는 순간과 그 감독이 마지막 세대를 발행하는
        //   순간 사이에 경합이 있다(그 감독은 `shake_hands` 의 `select` 안에서 핸드셰이크와 `Close` 를
        //   동시에 준비된 채로 만날 수 있고, 핸드셰이크가 이기면 번호를 하나 더 발행한다). 계수기를
        //   물려주면 누가 이기든 같은 번호가 두 번 나오지 않는다.
        let generations = self
            .inner
            .generations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(id.clone())
            .or_insert_with(|| Arc::new(AtomicU64::new(0)))
            .clone();
        let capacity = policy.inbound_queue.max(1);
        let (displaced, lane) = self.inner.shared.with(|roster| {
            roster.release_finished();
            let displaced = roster.peers.remove(&id);
            let lane = (roster.consumer == Consumer::Open).then(|| roster.lane_for(&id, capacity));
            (displaced, lane)
        });
        if let Some(old) = displaced {
            old.close();
        }
        // 받는 쪽이 없으면 받는 끝을 곧바로 버린 줄을 주고 닫힌 채 띄운다 — 감독은 걸기 전에 닫기를 본다.
        let start_closed = lane.is_none();
        let inbound = lane.unwrap_or_else(|| mpsc::channel(1).0);
        let peer = spawn_peer(PeerConfig {
            id: id.clone(),
            addr,
            wire: self.inner.wire.clone(),
            dialer: self.inner.dialer.clone(),
            clock: self.inner.clock.clone(),
            handshake,
            policy,
            inbound,
            byte_budget: BYTE_BUDGET,
            generations,
            start_closed,
        });
        let (displaced, consumer_left) = self.inner.shared.with(|roster| {
            let displaced = roster.peers.insert(id, peer.clone());
            (displaced, roster.consumer != Consumer::Open)
        });
        // 같은 이름표를 동시에 올린 다른 호출이 그 사이에 끼었으면 먼저 들어간 쪽을 닫는다.
        if let Some(other) = displaced {
            other.close();
        }
        // 위에서 받는 쪽을 본 뒤 그것이 닫혔다 — 받는 쪽은 닫을 때 그 순간의 줄만 닫으므로 여기서 접는다.
        if consumer_left {
            peer.close();
        }
        peer
    }

    /// 명부에서 내리고 그 상대를 닫는다. 그 이름표의 줄은 감독이 끝나고 다 꺼낸 뒤, 받는 쪽이 꺼내거나
    /// 명부가 바뀔 때 걷힌다 — 그 전에 같은 이름표를 다시 올리면 그 줄을 이어 쓴다.
    pub fn remove(&self, id: &PeerId) {
        let removed = self.inner.shared.with(|roster| {
            let removed = roster.peers.remove(id);
            roster.release_finished();
            removed
        });
        if let Some(peer) = removed {
            peer.close();
        }
    }

    pub fn get(&self, id: &PeerId) -> Option<Peer<W>> {
        self.inner
            .shared
            .with(|roster| roster.peers.get(id).cloned())
    }

    /// 이름표 순으로 정렬한 지금 한 장. ★셸이 「앱 전체」를 접는 재료가 이것이다★.
    pub fn peers(&self) -> Vec<(PeerId, PeerState)> {
        let mut out: Vec<(PeerId, PeerState)> = self.inner.shared.with(|roster| {
            roster
                .peers
                .iter()
                .map(|(id, peer)| (id.clone(), peer.peer_state()))
                .collect()
        });
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    pub fn len(&self) -> usize {
        self.inner.shared.with(|roster| roster.peers.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 전 상대에게 같은 것을 민다.
    ///
    /// ★한 번만 인코딩하고 프레임을 나눠 준다★ — 그래서 `W::Out` 에 `Clone` 을 요구하지 않는다. 대가로
    /// 팬아웃은 **알림 의미 고정**이다: 답장을 기다리지 않는다.
    ///
    /// 부분 실패는 그 상대의 [`TransportEvent::Dropped`] 로 **그 상대의 줄에** 올라온다 — ★조용히
    /// 사라지는 것을 안 만드는 것이 이 함수의 유일한 까다로운 부분이다★(오늘 `net` 의 팬아웃이 정확히 그
    /// 자리에서 조용하다). 그 신고도 그 상대의 줄이 차 있으면 사라진다(세지 않는다). ★감독의 순서 칸을
    /// 거치지 않는다★ — 그 상대의 감독이 붙들고 있는 것보다 앞서 줄에 들 수 있다.
    ///
    /// ★앱이 [`Peer::close`] 로 닫은 상대에게는 신고하지 않는다★ — 명부에 남아 있어도 같다. 앱이 닫겠다고
    /// 한 뒤의 유실 개수는 읽을 쪽이 없고 깔끔한 종료가 우선이다(사용자 결정 2026-09-26 — `lib.rs`
    /// 「알려진 한계」).
    pub fn broadcast(&self, out: W::Out) {
        let frame = self.inner.wire.encode(&out);
        let targets: Vec<(Peer<W>, Option<mpsc::Sender<Charged<Incoming<W>>>>)> =
            self.inner.shared.with(|roster| {
                roster
                    .peers
                    .iter()
                    .map(|(id, peer)| {
                        let lane = roster.lanes.get(id).and_then(|slot| slot.tx.clone());
                        (peer.clone(), lane)
                    })
                    .collect()
            });
        for (peer, lane) in targets {
            if peer.send_frame(frame.clone()).is_err() && !peer.close_requested() {
                if let Some(lane) = lane {
                    let _ =
                        lane.try_send(Charged::bare(Incoming::Event(TransportEvent::Dropped {
                            peer: peer.id().clone(),
                            direction: Direction::Outbound,
                            count: 1,
                        })));
                }
            }
        }
    }

    /// 표에 선 줄 수.
    #[cfg(test)]
    fn held_lanes(&self) -> usize {
        self.inner.shared.with(|roster| roster.lanes.len())
    }

    /// 그 이름표 줄에 넣는 끝 — 감독이 받는 것과 같은 표의 끝을 복제한다.
    #[cfg(test)]
    fn lane_sender(&self, id: &PeerId) -> Option<mpsc::Sender<Charged<Incoming<W>>>> {
        self.inner
            .shared
            .with(|roster| roster.lanes.get(id).and_then(|slot| slot.tx.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Arrival, Direction};
    use crate::frame::Frame;
    use crate::machine::{GaveUpReason, Generation};
    use crate::testing::{
        settle, settle_until, ImmediateHandshake, ManualClock, MemoryNetwork, RejectingHandshake,
        TestIn, TestOut, TestWire,
    };
    use std::time::Duration;

    struct Fixture {
        registry: Registry<TestWire>,
        inbound: Inbound<TestWire>,
        net: MemoryNetwork,
        clock: ManualClock,
    }

    impl Fixture {
        fn new(policy: Policy) -> Self {
            let net = MemoryNetwork::new();
            let clock = ManualClock::new();
            let (registry, inbound) =
                Registry::new(Arc::new(TestWire), net.dialer(), clock.handle(), policy);
            Self {
                registry,
                inbound,
                net,
                clock,
            }
        }

        fn drain(&mut self) -> Vec<Incoming<TestWire>> {
            let mut out = Vec::new();
            while let Some(item) = self.inbound.try_recv() {
                out.push(item);
            }
            out
        }

        /// 감독들을 돌리며 비운다 — 감독이 순서 칸에 붙든 것을 줄로 옮길 틈을 주면서, 한 바퀴에 아무것도 안
        /// 올라올 때까지.
        async fn drain_until_quiet(&mut self) -> Vec<Incoming<TestWire>> {
            let mut out = Vec::new();
            loop {
                settle().await;
                let batch = self.drain();
                if batch.is_empty() {
                    return out;
                }
                out.extend(batch);
            }
        }
    }

    fn immediate() -> Arc<dyn Handshake> {
        Arc::new(ImmediateHandshake)
    }

    #[tokio::test]
    async fn every_arrival_carries_the_peer_it_came_from() {
        let mut f = Fixture::new(Policy::default());
        f.registry
            .add(PeerId::new("a"), Address::new("mem://a"), immediate());
        settle().await;
        let ea = f.net.accept().unwrap();
        f.registry
            .add(PeerId::new("b"), Address::new("mem://b"), immediate());
        settle().await;
        let eb = f.net.accept().unwrap();
        f.drain();

        ea.push(TestWire::encode_in(&TestIn::Notice("from-a".into())));
        eb.push(TestWire::encode_in(&TestIn::Notice("from-b".into())));
        settle().await;

        let mut seen: Vec<(String, TestIn)> = f
            .drain()
            .into_iter()
            .filter_map(|i| match i {
                Incoming::Message(a) => Some((a.peer.as_str().to_string(), a.msg)),
                Incoming::Event(_) => None,
            })
            .collect();
        seen.sort_by(|x, y| x.0.cmp(&y.0));
        assert_eq!(
            seen,
            vec![
                ("a".to_string(), TestIn::Notice("from-a".into())),
                ("b".to_string(), TestIn::Notice("from-b".into())),
            ]
        );
    }

    #[tokio::test]
    async fn messages_and_events_share_one_line() {
        let mut f = Fixture::new(Policy::default());
        f.registry
            .add(PeerId::new("a"), Address::new("mem://a"), immediate());
        settle().await;
        let endpoint = f.net.accept().unwrap();
        endpoint.push(TestWire::encode_in(&TestIn::Notice("x".into())));
        settle().await;

        let items = f.drain();
        assert!(items.iter().any(|i| i.as_event().is_some()));
        assert!(items.iter().any(|i| i.as_message().is_some()));
    }

    #[tokio::test]
    async fn one_peer_giving_up_leaves_the_others_alone() {
        let mut f = Fixture::new(Policy::default());
        let good = f
            .registry
            .add(PeerId::new("good"), Address::new("mem://g"), immediate());
        settle().await;
        let _endpoint = f.net.accept().unwrap();

        let bad = f.registry.add(
            PeerId::new("bad"),
            Address::new("mem://b"),
            RejectingHandshake::new("nope"),
        );
        settle().await;

        assert_eq!(bad.peer_state(), PeerState::GaveUp(GaveUpReason::Rejected));
        assert_eq!(good.peer_state(), PeerState::Live);
        assert_eq!(
            f.registry.peers(),
            vec![
                (
                    PeerId::new("bad"),
                    PeerState::GaveUp(GaveUpReason::Rejected)
                ),
                (PeerId::new("good"), PeerState::Live),
            ]
        );
        assert!(f.drain().iter().any(|i| i.peer() == &PeerId::new("bad")));
    }

    #[tokio::test]
    async fn broadcast_reaches_everyone_with_a_single_encode() {
        let mut f = Fixture::new(Policy::default());
        f.registry
            .add(PeerId::new("a"), Address::new("mem://a"), immediate());
        settle().await;
        let ea = f.net.accept().unwrap();
        f.registry
            .add(PeerId::new("b"), Address::new("mem://b"), immediate());
        settle().await;
        let eb = f.net.accept().unwrap();
        f.drain();

        f.registry.broadcast(TestOut::Notice("all".into()));
        settle().await;
        assert_eq!(ea.frames(), vec![Frame::Text("not|all".into())]);
        assert_eq!(eb.frames(), vec![Frame::Text("not|all".into())]);
    }

    #[tokio::test]
    async fn a_partial_broadcast_failure_shows_up_as_that_peers_event() {
        let mut f = Fixture::new(Policy::default());
        f.registry
            .add(PeerId::new("up"), Address::new("mem://a"), immediate());
        settle().await;
        let ea = f.net.accept().unwrap();
        f.registry.add(
            PeerId::new("down"),
            Address::new("mem://b"),
            RejectingHandshake::new("nope"),
        );
        settle().await;
        f.drain();

        f.registry.broadcast(TestOut::Notice("all".into()));
        settle().await;
        assert_eq!(ea.frames(), vec![Frame::Text("not|all".into())]);

        let dropped: Vec<PeerId> = f
            .drain()
            .into_iter()
            .filter_map(|i| match i.as_event() {
                Some(TransportEvent::Dropped { peer, .. }) => Some(peer.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(dropped, vec![PeerId::new("down")]);
    }

    #[tokio::test]
    async fn remove_closes_the_peer_and_drops_it_from_the_roster() {
        let mut f = Fixture::new(Policy::default());
        let peer = f
            .registry
            .add(PeerId::new("a"), Address::new("mem://a"), immediate());
        settle().await;
        assert_eq!(f.registry.len(), 1);
        assert!(f.registry.get(&PeerId::new("a")).is_some());

        f.registry.remove(&PeerId::new("a"));
        settle().await;
        assert!(f.registry.is_empty());
        assert_eq!(peer.peer_state(), PeerState::Closed);
        let _ = f.drain();
    }

    #[tokio::test]
    async fn re_adding_the_same_name_replaces_the_old_supervisor() {
        let mut f = Fixture::new(Policy::default());
        let first = f
            .registry
            .add(PeerId::new("a"), Address::new("mem://a"), immediate());
        settle().await;
        let second = f
            .registry
            .add(PeerId::new("a"), Address::new("mem://a2"), immediate());
        settle().await;
        assert_eq!(first.peer_state(), PeerState::Closed);
        assert_eq!(second.peer_state(), PeerState::Live);
        assert_eq!(f.registry.len(), 1);
        let _ = f.drain();
    }

    // ── N7: 같은 이름표를 다시 올려도 세대가 되감기지 않는다 ──
    #[tokio::test]
    async fn re_adding_the_same_name_never_rewinds_the_generation() {
        let mut f = Fixture::new(Policy::default());
        let first = f
            .registry
            .add(PeerId::new("a"), Address::new("mem://a"), immediate());
        settle().await;
        assert_eq!(first.generation(), Generation(1));

        let second = f
            .registry
            .add(PeerId::new("a"), Address::new("mem://a2"), immediate());
        settle().await;
        assert_eq!(
            second.generation(),
            Generation(2),
            "★0 으로 되감기면 「최대 세대 미만은 무시」 방어가 새 연결을 통째로 버린다★"
        );
        let _ = f.drain();
    }

    #[tokio::test]
    async fn the_old_supervisor_is_told_to_close_before_the_new_one_dials() {
        let mut f = Fixture::new(Policy::default());
        let first = f
            .registry
            .add(PeerId::new("a"), Address::new("mem://a"), immediate());
        settle().await;
        f.registry
            .add(PeerId::new("a"), Address::new("mem://a2"), immediate());
        // 아직 아무 태스크도 안 돌린 시점 — 옛 핸들은 이미 종료 신호를 받았어야 한다.
        settle().await;
        assert_eq!(first.peer_state(), PeerState::Closed);
        let _ = f.drain();
    }

    #[tokio::test]
    async fn per_peer_policy_overrides_the_registry_default() {
        let f = Fixture::new(Policy::default());
        let strict = Policy {
            reconnect: crate::policy::ReconnectPolicy {
                max_attempts: 1,
                jitter: 0.0,
                ..Default::default()
            },
            ..Policy::default()
        };
        f.net.fail_forever();
        f.registry.add_with_policy(
            PeerId::new("a"),
            Address::new("mem://a"),
            immediate(),
            strict,
        );
        settle().await;
        f.clock.advance(std::time::Duration::from_millis(500));
        settle().await;
        assert_eq!(
            f.registry.peers()[0].1,
            PeerState::GaveUp(GaveUpReason::BudgetExhausted),
            "상대별 예산 1이 명부 기본값 3을 이긴다"
        );
        assert_eq!(f.net.dials(), 2);
    }

    // ── N1: 위로 올라가는 줄이 찼다고 연결을 끊지도 버리지도 않는다 ──
    #[tokio::test]
    async fn a_saturated_line_neither_cuts_the_link_nor_loses_anything() {
        let mut f = Fixture::new(Policy {
            inbound_queue: 2,
            ..Policy::default()
        });
        let peer = f
            .registry
            .add(PeerId::new("a"), Address::new("mem://a"), immediate());
        settle().await;
        let endpoint = f.net.accept().unwrap();
        // 아무도 안 읽는 채로 밀어 넣고, 상대가 곧바로 닫는다.
        for seq in 1..=200 {
            endpoint.push(TestWire::chunk(1, 1, seq));
        }
        settle().await;
        assert_eq!(
            peer.peer_state(),
            PeerState::Live,
            "위로 올라갈 자리가 없는 것은 이 연결의 잘못이 아니다"
        );
        endpoint.close();
        settle().await;

        let items = f.drain_until_quiet().await;
        let seqs: Vec<u64> = items
            .iter()
            .filter_map(|i| match i.as_message().map(|a| &a.msg) {
                Some(TestIn::Chunk { seq, .. }) => Some(*seq),
                _ => None,
            })
            .collect();
        assert_eq!(
            seqs,
            (1..=200).collect::<Vec<_>>(),
            "★줄이 찬 동안 온 것도 하나도 안 버려지고 온 순서대로 나온다★"
        );
        let kinds: Vec<&'static str> = items
            .iter()
            .filter_map(|i| i.as_event().map(|e| e.kind()))
            .collect();
        assert_eq!(
            kinds,
            vec!["Connected", "Disconnected"],
            "★끊김은 그 앞에 온 것들 뒤에 오고, 유실 신고는 없다★"
        );
        assert!(
            matches!(
                items.last().and_then(|i| i.as_event()),
                Some(TransportEvent::Disconnected { .. })
            ),
            "끊김이 마지막이다"
        );
    }

    // 데몬 되감기 링 한 벌(4096)이 한꺼번에 오는 모양 — 줄 길이(기본 512)의 여덟 배다.
    #[tokio::test]
    async fn a_replay_sized_burst_arrives_whole_while_the_consumer_lags() {
        let mut f = Fixture::new(Policy::default());
        f.registry
            .add(PeerId::new("a"), Address::new("mem://a"), immediate());
        settle().await;
        let endpoint = f.net.accept().unwrap();
        for seq in 1..=4096 {
            endpoint.push(TestWire::chunk(1, 1, seq));
        }
        settle().await;
        assert!(
            endpoint.unread() > 0,
            "★받는 쪽이 밀린 동안 감독은 줄 · 앞잡이 줄 분량까지만 읽고 나머지는 통로에 둔다★(배압)"
        );

        let seqs: Vec<u64> = f
            .drain_until_quiet()
            .await
            .iter()
            .filter_map(|i| match i.as_message().map(|a| &a.msg) {
                Some(TestIn::Chunk { seq, .. }) => Some(*seq),
                _ => None,
            })
            .collect();
        assert_eq!(seqs, (1..=4096).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn a_flooding_peer_does_not_sever_a_healthy_peer() {
        let mut f = Fixture::new(Policy {
            inbound_queue: 2,
            ..Policy::default()
        });
        let noisy = f
            .registry
            .add(PeerId::new("noisy"), Address::new("mem://a"), immediate());
        settle().await;
        let noisy_link = f.net.accept().unwrap();
        // 아무도 안 읽는 줄을 가득 채운다.
        for seq in 1..=500 {
            noisy_link.push(TestWire::chunk(1, 1, seq));
        }
        settle().await;

        let quiet = f
            .registry
            .add(PeerId::new("quiet"), Address::new("mem://b"), immediate());
        settle().await;
        assert_eq!(
            quiet.peer_state(),
            PeerState::Live,
            "★남의 홍수로 내 연결이 끊기면 ADR-0180 결정 8 이 깨진다★"
        );
        assert_eq!(noisy.peer_state(), PeerState::Live);
        assert_eq!(f.net.dials(), 2, "재연결이 돌지 않았다");
        assert!(
            f.drain().iter().any(|i| matches!(
                i.as_event(),
                Some(TransportEvent::Connected { peer, .. }) if peer == &PeerId::new("quiet")
            )),
            "남의 홍수가 내 Connected 를 버리게 하지 못한다"
        );
    }

    // ── 3-0 U1a: 상대별 줄 ──
    fn notice(body: &str) -> Frame {
        TestWire::encode_in(&TestIn::Notice(body.into()))
    }

    #[tokio::test]
    async fn a_noisy_peer_costs_a_quiet_peer_nothing_and_cannot_hold_it_back() {
        let mut f = Fixture::new(Policy {
            inbound_queue: 64,
            frame_buffer: 8,
            ..Policy::default()
        });
        let noisy = f
            .registry
            .add(PeerId::new("noisy"), Address::new("mem://n"), immediate());
        settle().await;
        let noisy_link = f.net.accept().unwrap();
        // 아무도 안 읽는 채로 시끄러운 쪽이 자기 줄을 넘치게 채운다.
        for seq in 1..=500 {
            noisy_link.push(TestWire::chunk(1, 1, seq));
        }
        settle().await;

        let quiet = f
            .registry
            .add(PeerId::new("quiet"), Address::new("mem://q"), immediate());
        settle().await;
        let quiet_link = f.net.accept().unwrap();
        for n in 0..3 {
            quiet_link.push(notice(&format!("q{n}")));
        }
        settle().await;
        // TODO(TRD 3 §6-0 U1d): 손잡이마다 수신단이 서면 「안 비운 수신단은 그 상대만 세운다」를 수신단 단위로
        //   다시 잰다 — 지금은 받는 쪽이 하나라 「시끄러운 쪽 줄만 차 있다」로 그 모양을 만든다.
        assert!(
            noisy_link.unread() > 0,
            "★시끄러운 쪽은 자기 통로에서 배압을 받는다★ — 줄이 찬 뒤로는 그 통로를 안 읽는다"
        );
        assert_eq!(
            quiet_link.unread(),
            0,
            "★조용한 쪽의 통로는 남의 배압과 무관하게 다 읽힌다★"
        );

        let items = f.drain();
        let quiet_at: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.peer() == &PeerId::new("quiet"))
            .map(|(at, _)| at)
            .collect();
        assert_eq!(
            quiet_at.len(),
            4,
            "★조용한 상대의 Connected 와 알림 셋이 하나도 안 버려진다★: {quiet_at:?}"
        );
        for (k, at) in quiet_at.iter().enumerate() {
            assert!(
                *at < 2 * (k + 1),
                "★밀린 64개가 앞에 있어도 상대 수(2)만큼의 꺼냄 안에 나온다★: {k}번째가 {at}"
            );
        }
        assert_eq!(noisy.peer_state(), PeerState::Live);
        assert_eq!(quiet.peer_state(), PeerState::Live);

        // 넘친 시끄러운 쪽의 것도 그 감독이 붙들고 있다가 순서대로 낸다.
        let noisy_seqs: Vec<u64> = items
            .into_iter()
            .chain(f.drain_until_quiet().await)
            .filter(|item| item.peer() == &PeerId::new("noisy"))
            .filter_map(|item| match item.as_message().map(|a| &a.msg) {
                Some(TestIn::Chunk { seq, .. }) => Some(*seq),
                _ => None,
            })
            .collect();
        assert_eq!(noisy_seqs, (1..=500).collect::<Vec<_>>());
    }

    // ── 3-0 U1c: 명부가 바이트 예산을 감독에게 넘긴다 ──
    //
    // ★실제 예산 값으로 잰다★ — 감독 시험은 작은 예산을 직접 넣으므로, 명부가 엉뚱한 값(0 · 상한 없음)을 넘겨도
    //   거기서는 안 보인다. 스트림을 안 쓰는 상대는 예산 전부(16 MiB)가 읽기 몫이라 그 절반보다 큰 두 장이면 둘째에서
    //   읽기 문이 닫힌다(net `CONN_RX_MAX_BYTES` 시험과 같은 모양 — 같은 값이다).
    #[tokio::test]
    async fn the_registry_stops_reading_a_peer_once_it_holds_the_byte_budget() {
        let mut f = Fixture::new(Policy::default());
        let peer = f
            .registry
            .add(PeerId::new("a"), Address::new("mem://a"), immediate());
        settle().await;
        let endpoint = f.net.accept().unwrap();
        f.drain();
        let big = BYTE_BUDGET / 2 + 1;
        for tag in 0..3u8 {
            endpoint.push(Frame::Binary(vec![tag; big]));
        }
        settle().await;
        assert_eq!(
            endpoint.unread(),
            1,
            "★둘째 장에서 읽기 몫을 넘기고 셋째는 통로에 남는다★"
        );
        assert_eq!(peer.held_bytes(), 2 * big);

        let tags: Vec<u8> = f
            .drain_until_quiet()
            .await
            .iter()
            .filter_map(|i| match i.as_message().map(|a| &a.msg) {
                Some(TestIn::Blob(bytes)) => Some(bytes[0]),
                _ => None,
            })
            .collect();
        assert_eq!(tags, vec![0, 1, 2], "★손실 0 · 순서 그대로★");
        assert_eq!(peer.held_bytes(), 0);
    }

    #[tokio::test]
    async fn each_peer_line_takes_its_length_from_that_peers_policy() {
        let mut f = Fixture::new(Policy {
            inbound_queue: 2,
            ..Policy::default()
        });
        f.registry.add_with_policy(
            PeerId::new("roomy"),
            Address::new("mem://r"),
            immediate(),
            Policy {
                inbound_queue: 8,
                ..Policy::default()
            },
        );
        settle().await;
        let link = f.net.accept().unwrap();
        for n in 0..6 {
            link.push(notice(&format!("n{n}")));
        }
        settle().await;
        let messages = f
            .drain()
            .iter()
            .filter(|i| i.as_message().is_some())
            .count();
        assert_eq!(
            messages, 6,
            "명부 기본값 2 가 아니라 상대별 8 이 그 줄의 길이다"
        );
    }

    #[tokio::test]
    async fn a_broadcast_loss_report_is_not_crowded_out_by_another_peers_flood() {
        let mut f = Fixture::new(Policy {
            inbound_queue: 4,
            ..Policy::default()
        });
        f.registry
            .add(PeerId::new("noisy"), Address::new("mem://n"), immediate());
        settle().await;
        let noisy_link = f.net.accept().unwrap();
        for seq in 1..=500 {
            noisy_link.push(TestWire::chunk(1, 1, seq));
        }
        settle().await;
        f.registry.add(
            PeerId::new("down"),
            Address::new("mem://d"),
            RejectingHandshake::new("nope"),
        );
        settle().await;

        f.registry.broadcast(TestOut::Notice("all".into()));
        settle().await;
        let reported: Vec<PeerId> = f
            .drain()
            .into_iter()
            .filter_map(|i| match i.as_event() {
                Some(TransportEvent::Dropped {
                    peer,
                    direction: Direction::Outbound,
                    ..
                }) => Some(peer.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(reported, vec![PeerId::new("down")]);
    }

    #[tokio::test]
    async fn a_waiting_recv_wakes_for_a_peer_added_after_it_began_waiting() {
        let f = Fixture::new(Policy::default());
        let mut inbound = f.inbound;
        let waiting = tokio::spawn(async move {
            let first = inbound.recv().await;
            (first, inbound)
        });
        settle().await;
        assert!(!waiting.is_finished(), "줄이 하나도 없으면 기다린다");

        f.registry
            .add(PeerId::new("late"), Address::new("mem://l"), immediate());
        settle().await;
        let (first, mut inbound) = tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("기다리던 recv 가 새 상대의 줄을 못 봤다 — 영구 대기 회귀다")
            .expect("받는 태스크가 패닉했다");
        assert!(matches!(
            first.as_ref().and_then(|i| i.as_event()),
            Some(TransportEvent::Connected { peer, .. }) if peer == &PeerId::new("late")
        ));

        let link = f.net.accept().unwrap();
        let waiting = tokio::spawn(async move { inbound.recv().await });
        settle().await;
        assert!(!waiting.is_finished());
        link.push(notice("wake"));
        let next = tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("기다리던 recv 가 이미 있는 줄의 새 항목을 못 봤다")
            .expect("받는 태스크가 패닉했다");
        assert!(matches!(
            next.as_ref().and_then(|i| i.as_message()).map(|a| &a.msg),
            Some(TestIn::Notice(body)) if body == "wake"
        ));
    }

    #[tokio::test]
    async fn closing_the_inbound_folds_every_peer_and_keeps_what_was_queued() {
        let mut f = Fixture::new(Policy::default());
        let early = f
            .registry
            .add(PeerId::new("a"), Address::new("mem://a"), immediate());
        settle().await;
        let link = f.net.accept().unwrap();
        link.push(notice("before"));
        settle().await;

        f.inbound.close();
        link.push(notice("after"));
        settle_until("a 접힘", || early.peer_state() == PeerState::Closed).await;
        let late = f
            .registry
            .add(PeerId::new("b"), Address::new("mem://b"), immediate());
        settle_until("b 접힘", || late.peer_state() == PeerState::Closed).await;
        assert_eq!(
            f.net.dials(),
            1,
            "소비자 부재는 재연결할 일이 아니고, 닫힌 뒤에 올린 상대는 걸지도 않는다"
        );

        let bodies: Vec<String> = f
            .drain()
            .into_iter()
            .filter_map(|i| match i {
                Incoming::Message(a) => match a.msg {
                    TestIn::Notice(body) => Some(body),
                    _ => None,
                },
                Incoming::Event(_) => None,
            })
            .collect();
        assert_eq!(
            bodies,
            vec!["before".to_string()],
            "닫기 전에 든 것은 꺼낼 수 있다"
        );
    }

    #[tokio::test]
    async fn dropping_the_inbound_folds_every_peer_and_later_ones_never_dial() {
        let f = Fixture::new(Policy::default());
        let early = f
            .registry
            .add(PeerId::new("a"), Address::new("mem://a"), immediate());
        settle().await;
        let link = f.net.accept().unwrap();
        drop(f.inbound);
        link.push(notice("nobody listens"));
        settle_until("a 접힘", || early.peer_state() == PeerState::Closed).await;

        // 새 이름표(새 줄)와 같은 이름표(이어 쓰는 줄) 둘 다.
        let fresh = f
            .registry
            .add(PeerId::new("b"), Address::new("mem://b"), immediate());
        let again = f
            .registry
            .add(PeerId::new("a"), Address::new("mem://a"), immediate());
        settle_until("둘 다 접힘", || {
            fresh.peer_state() == PeerState::Closed && again.peer_state() == PeerState::Closed
        })
        .await;
        assert_eq!(
            f.net.dials(),
            1,
            "받는 쪽이 사라진 뒤에 올린 상대는 걸지도 않는다"
        );
    }

    // ── 3-0 U1a: 같은 이름표는 줄 하나 ──
    #[tokio::test]
    async fn re_adding_a_name_reuses_its_line_so_no_line_grows_while_nobody_reads() {
        let mut f = Fixture::new(Policy {
            inbound_queue: 4,
            ..Policy::default()
        });
        let name = PeerId::new("a");
        for round in 0..50 {
            if round % 2 == 1 {
                f.registry.remove(&name);
            }
            f.registry
                .add(name.clone(), Address::new("mem://a"), immediate());
            settle().await;
        }
        assert_eq!(f.registry.len(), 1);
        assert_eq!(
            f.inbound.held_lanes(),
            1,
            "★갈아치움 · 내렸다 다시 올림이 줄을 늘리면 받는 쪽이 안 읽는 동안 줄 길이 × 횟수만큼 쥔다★"
        );
        assert_eq!(f.registry.held_lanes(), 1);
        let buffered = f.drain().len();
        assert!(
            buffered <= 4,
            "줄에는 그 길이를 넘게 쌓이지 않는다 — 정리 중인 옛 감독이 붙든 꼬리는 줄 밖 순서 칸에 있다: {buffered}"
        );
    }

    #[tokio::test]
    async fn churning_a_name_before_its_supervisor_runs_holds_one_line_while_nobody_reads() {
        let mut f = Fixture::new(Policy::default());
        let name = PeerId::new("a");
        for _ in 0..30 {
            // 감독이 돌기 전에 내린다 — 감독은 걸기 전에 닫기를 보고 `Closed` 하나만 낸 채 끝난다.
            f.registry
                .add(name.clone(), Address::new("mem://a"), immediate());
            f.registry.remove(&name);
            settle().await;
        }
        assert_eq!(
            f.inbound.held_lanes(),
            1,
            "★받는 쪽이 안 읽는 동안 올렸다 내리기를 되풀이해도 이름표 하나에 줄 하나다★"
        );
        f.drain();
        assert_eq!(
            f.inbound.held_lanes(),
            0,
            "받는 쪽이 찌르면 내린 이름표의 끝난 줄은 걷힌다"
        );
    }

    #[tokio::test]
    async fn re_adding_a_finished_name_without_reading_keeps_one_line_and_its_backlog() {
        let mut f = Fixture::new(Policy {
            inbound_queue: 4,
            ..Policy::default()
        });
        let name = PeerId::new("a");
        for _ in 0..20 {
            f.registry
                .add(name.clone(), Address::new("mem://a"), immediate());
            settle().await;
            f.registry.remove(&name);
            settle().await;
        }
        assert_eq!(f.inbound.held_lanes(), 1);
        let backlog = f.drain();
        let firsts: Vec<(&'static str, Option<Generation>)> = backlog
            .iter()
            .filter_map(|i| i.as_event())
            .map(|e| match e {
                TransportEvent::Connected { generation, .. }
                | TransportEvent::Disconnected { generation, .. }
                | TransportEvent::Closed { generation, .. } => (e.kind(), Some(*generation)),
                other => (other.kind(), None),
            })
            .collect();
        assert_eq!(
            firsts,
            vec![
                ("Connected", Some(Generation(1))),
                ("Disconnected", Some(Generation(1))),
                ("Closed", Some(Generation(1))),
                ("Connected", Some(Generation(2))),
            ],
            "★감독이 끝났어도 다 안 꺼낸 줄은 걷지 않는다★ — 걷으면 그 안의 것을 잃는다"
        );
    }

    #[tokio::test]
    async fn removed_distinct_names_free_their_lines_once_drained() {
        let mut f = Fixture::new(Policy::default());
        let names: Vec<PeerId> = (0..10).map(|n| PeerId::new(format!("conn-{n}"))).collect();
        for name in &names {
            f.registry
                .add(name.clone(), Address::new("mem://c"), immediate());
        }
        settle().await;
        for name in &names {
            f.registry.remove(name);
        }
        settle().await;
        f.drain();
        assert_eq!(
            f.inbound.held_lanes(),
            0,
            "★연결마다 새 이름표를 쓰는 소비자에게 줄이 쌓이지 않는다★"
        );

        f.registry
            .add(PeerId::new("live"), Address::new("mem://l"), immediate());
        settle().await;
        f.drain();
        assert_eq!(f.inbound.held_lanes(), f.registry.len());
    }

    #[tokio::test]
    async fn closing_the_inbound_frees_drained_lines_on_the_next_roster_change() {
        let mut f = Fixture::new(Policy::default());
        f.registry
            .add(PeerId::new("a"), Address::new("mem://a"), immediate());
        settle().await;
        f.drain();
        f.inbound.close();
        // 받는 쪽이 더 찌르지 않아도 다음 명부 변경이 걷는다.
        f.registry.remove(&PeerId::new("nobody"));
        assert_eq!(f.registry.held_lanes(), 0);
    }

    // ── 3-0 U1a: 닫힌 줄은 「비었다」가 아니라 「끝났다」에서 걷는다 ──
    //
    // ★감독의 넣기는 자리를 먼저 잡고 나중에 싣는다★(tokio `try_send` 의 안쪽 두 걸음) — 그 사이에 받는 쪽이
    //   닫히면 줄은 비어 보여도 잡힌 자리가 남아 있다. 여기서는 그 자리를 `reserve` 로 일부러 붙든다.
    #[tokio::test]
    async fn a_put_holding_capacity_across_close_still_arrives_before_the_end() {
        let mut f = Fixture::new(Policy::default());
        f.registry
            .add(PeerId::new("a"), Address::new("mem://a"), immediate());
        settle().await;
        f.drain();

        let lane = f.registry.lane_sender(&PeerId::new("a")).unwrap();
        let permit = lane.reserve().await.unwrap();
        f.inbound.close();
        f.registry.remove(&PeerId::new("nobody"));
        assert!(f.inbound.try_recv().is_none(), "아직 실린 것이 없다");
        permit.send(Charged::bare(Incoming::Message(Arrival {
            peer: PeerId::new("a"),
            generation: Generation(1),
            msg: TestIn::Notice("held".into()),
        })));
        drop(lane);

        let first = tokio::time::timeout(Duration::from_secs(5), f.inbound.recv())
            .await
            .expect("이미 실렸다");
        assert!(
            matches!(
                first.as_ref().and_then(|i| i.as_message()).map(|a| &a.msg),
                Some(TestIn::Notice(body)) if body == "held"
            ),
            "★자리를 잡은 넣기는 닫힌 뒤에도 실리고, 끝보다 먼저 온다★"
        );
        let end = tokio::time::timeout(Duration::from_secs(5), f.inbound.recv())
            .await
            .expect("닫혔고 다 꺼냈다");
        assert!(end.is_none());
    }

    #[tokio::test]
    async fn a_waiting_recv_ends_when_the_registry_goes_away_with_no_lines() {
        let f = Fixture::new(Policy::default());
        let mut inbound = f.inbound;
        let registry = f.registry;
        let waiting = tokio::spawn(async move { inbound.recv().await });
        settle().await;
        assert!(!waiting.is_finished(), "명부가 살아 있으면 기다린다");
        drop(registry);
        settle_until("기다리던 recv 가 끝을 봄", || waiting.is_finished()).await;
        let end = tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("이미 끝났어야 한다")
            .expect("받는 태스크가 패닉했다");
        assert!(
            end.is_none(),
            "★명부가 사라지면 줄이 없던 받는 쪽도 깨어나 끝을 본다★"
        );
    }

    #[test]
    fn an_add_that_panics_outside_a_runtime_leaves_the_registry_usable() {
        let net = MemoryNetwork::new();
        let clock = ManualClock::new();
        let (registry, _inbound) = Registry::new(
            Arc::new(TestWire),
            net.dialer(),
            clock.handle(),
            Policy::default(),
        );
        let spawned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            registry.add(PeerId::new("a"), Address::new("mem://a"), immediate())
        }));
        assert!(spawned.is_err(), "런타임 밖에서는 감독을 못 띄운다");
        assert_eq!(
            registry.len(),
            0,
            "★패닉 한 번이 명부를 못 쓰게 만들지 않는다★"
        );
        registry.remove(&PeerId::new("a"));
        assert_eq!(registry.held_lanes(), 0, "띄우다 만 줄도 걷힌다");
    }

    #[tokio::test]
    async fn a_removed_name_whose_line_was_drained_is_retired() {
        let mut f = Fixture::new(Policy::default());
        f.registry
            .add(PeerId::new("a"), Address::new("mem://a"), immediate());
        settle().await;
        f.registry.remove(&PeerId::new("a"));
        settle().await;
        f.drain();
        // 걷기는 명부를 바꿀 때 돈다.
        f.registry
            .add(PeerId::new("b"), Address::new("mem://b"), immediate());
        settle().await;
        assert_eq!(
            f.registry.held_lanes(),
            1,
            "감독이 끝나고 다 꺼낸 줄은 걷는다 — 안 걷으면 연결마다 새 이름표를 쓰는 소비자가 줄을 쌓는다"
        );
        f.drain();
        assert_eq!(f.inbound.held_lanes(), 1, "받는 쪽도 끝난 줄을 놓는다");
    }

    #[tokio::test]
    async fn a_name_re_added_while_its_old_supervisor_is_still_closing_keeps_one_line() {
        let mut f = Fixture::new(Policy::default());
        f.registry
            .add(PeerId::new("a"), Address::new("mem://a"), immediate());
        settle().await;
        f.drain();
        // 줄은 비었지만 옛 감독은 아직 닫기를 처리하지 않았다 — 그 꼬리가 아직 들어올 수 있다.
        f.registry.remove(&PeerId::new("a"));
        f.registry
            .add(PeerId::new("a"), Address::new("mem://a2"), immediate());
        settle().await;
        assert_eq!(
            f.inbound.held_lanes(),
            1,
            "★빈 줄이라도 옛 감독이 돌고 있으면 걷지 않는다★ — 걷으면 꼬리와 새 감독의 것이 두 줄로 갈라진다"
        );
        // ★옛 감독의 `Closed` 는 새 감독의 `Connected` 뒤에 올 수 있다★ — 옛 감독은 통로를 다 닫은 뒤에야
        //   그것을 내고, 그 사이 새 감독이 붙는다. 그래서 한 감독 안의 순서와 빠짐없음만 단언한다.
        let seen: Vec<(&'static str, Generation)> = f
            .drain()
            .iter()
            .filter_map(|i| match i.as_event() {
                Some(e @ TransportEvent::Disconnected { generation, .. })
                | Some(e @ TransportEvent::Closed { generation, .. })
                | Some(e @ TransportEvent::Connected { generation, .. }) => {
                    Some((e.kind(), *generation))
                }
                _ => None,
            })
            .collect();
        let at = |what: (&'static str, Generation)| seen.iter().position(|s| *s == what);
        assert_eq!(seen.len(), 3, "{seen:?}");
        assert!(at(("Connected", Generation(2))).is_some(), "{seen:?}");
        assert!(
            matches!(
                (at(("Disconnected", Generation(1))), at(("Closed", Generation(1)))),
                (Some(gone), Some(end)) if gone < end
            ),
            "옛 감독 안에서는 끊김 → 끝 순서다: {seen:?}"
        );
    }

    #[tokio::test]
    async fn a_re_added_name_keeps_the_old_tail_and_the_new_events_on_one_line_in_put_order() {
        let mut f = Fixture::new(Policy::default());
        f.registry.add(
            PeerId::new("a"),
            Address::new("mem://a"),
            RejectingHandshake::new("nope"),
        );
        settle().await;
        f.registry
            .add(PeerId::new("a"), Address::new("mem://a2"), immediate());
        settle().await;
        let kinds: Vec<&'static str> = f
            .drain()
            .iter()
            .filter_map(|i| i.as_event().map(|e| e.kind()))
            .collect();
        assert_eq!(kinds, vec!["Rejected", "GaveUp", "Closed", "Connected"]);
    }

    // ── 3-0 U1a: 기다리는 recv 는 모든 줄에서 깬다 ──
    //
    // ★같은 줄에 두 번 넣는다★ — 한 번 꺼낸 줄은 다음 바퀴에 맨 뒤로 가므로, 둘째 `recv` 에서는 넣은 줄이
    //   반드시 비어 있는(`Pending`) 줄 뒤에서 찔린다. 그 찌름이 깨울 손잡이를 거는 일이라 빠뜨린 변이는
    //   둘째 바퀴에서 깨지 못한다.
    #[tokio::test]
    async fn a_waiting_recv_wakes_for_a_lane_probed_after_an_empty_one() {
        let mut f = Fixture::new(Policy::default());
        f.registry
            .add(PeerId::new("a"), Address::new("mem://a"), immediate());
        settle().await;
        let link_a = f.net.accept().unwrap();
        f.registry
            .add(PeerId::new("b"), Address::new("mem://b"), immediate());
        settle().await;
        f.drain();

        let mut inbound = f.inbound;
        for round in 0..2 {
            let waiting = tokio::spawn(async move {
                let got = inbound.recv().await;
                (got, inbound)
            });
            settle().await;
            assert!(!waiting.is_finished(), "둘 다 비었으면 기다린다");
            let body = format!("r{round}");
            link_a.push(notice(&body));
            settle_until("기다리던 recv 가 깸", || waiting.is_finished()).await;
            let (got, back) = tokio::time::timeout(Duration::from_secs(5), waiting)
                .await
                .expect("이미 끝났어야 한다")
                .expect("받는 태스크가 패닉했다");
            inbound = back;
            assert!(matches!(
                got.as_ref().and_then(|i| i.as_message()).map(|a| &a.msg),
                Some(TestIn::Notice(text)) if *text == body
            ));
        }
    }

    // ── 3-0 U1a: 앱이 닫은 상대에게는 팬아웃 유실을 신고하지 않는다(사용자 결정 2026-09-26) ──
    #[tokio::test]
    async fn a_broadcast_loss_is_reported_for_an_open_peer_but_not_for_one_the_app_closed() {
        let mut f = Fixture::new(Policy::default());
        f.registry.add(
            PeerId::new("down"),
            Address::new("mem://d"),
            RejectingHandshake::new("nope"),
        );
        let shut = f
            .registry
            .add(PeerId::new("shut"), Address::new("mem://s"), immediate());
        settle().await;
        shut.close();
        settle_until("shut 접힘", || shut.peer_state() == PeerState::Closed).await;
        assert_eq!(f.registry.len(), 2, "앱이 닫았어도 명부에는 남아 있다");
        f.drain();

        f.registry.broadcast(TestOut::Notice("all".into()));
        settle().await;
        let reported: Vec<PeerId> = f
            .drain()
            .into_iter()
            .filter_map(|i| match i.as_event() {
                Some(TransportEvent::Dropped {
                    peer,
                    direction: Direction::Outbound,
                    ..
                }) => Some(peer.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(reported, vec![PeerId::new("down")]);
    }

    #[tokio::test]
    async fn dropping_the_registry_and_its_peers_closes_the_line() {
        let f = Fixture::new(Policy::default());
        let mut inbound = f.inbound;
        let registry = f.registry;
        registry.add(PeerId::new("a"), Address::new("mem://a"), immediate());
        settle().await;
        registry.remove(&PeerId::new("a"));
        drop(registry);
        settle().await;
        while inbound.try_recv().is_some() {}
        assert!(inbound.recv().await.is_none(), "명부가 통째로 닫혔다");
    }
}
