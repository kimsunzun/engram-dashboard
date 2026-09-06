//! 상대 N 개 명부 — ★구동 층★. 상대마다 감독 하나를 세우고, 올라오는 것을 **한 줄로** 합친다.
//!
//! ★상대는 처음부터 N 개다★(ADR-0177 결정 3 · ADR-0179) — 하나뿐인 세계를 전제한 상태가 어디에도 없다.
//! **상대끼리 상태를 공유하지 않는다**: 데몬 A 가 포기해도 B 는 운영 중이다.
//!
//! ★위로 올라가는 한 줄은 상대들이 **함께 쓰는 자원**이다★ — 그것이 차서 어떤 상대의 사건이 못 올라가도
//! **그 상대의 연결을 끊지 않는다**(`peer.rs` 의 `note_inbound_drop`). 끊으면 시끄러운 상대 하나가 조용한
//! 상대의 예산을 태워 함께 포기시키고, 그러면 「A 하나 죽어도 B·C 는 잘 동작해야 한다」가 무너진다
//! (ADR-0180 결정 8). 연결마다 따로 재는 자원은 [`Policy::frame_buffer`] 쪽이다.
//!
//! ★「앱 전체가 끊겼나」는 여기서 판정하지 않는다★ — 그건 화면의 개념이고 이 crate 는 상대를 셀 뿐
//! 앱을 모른다. 재료 둘([`Registry::peers`] 와 [`crate::Peer::reconnect_now`])을 주고 셸이 접는다
//! (ADR-0180 의 열린 질문이 그렇게 닫힌다).

use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use crate::clock::Clock;
use crate::event::{Direction, Incoming, PeerId, TransportEvent};
use crate::link::{Address, Dialer, Handshake};
use crate::machine::PeerState;
use crate::peer::{spawn_peer, Peer, PeerConfig};
use crate::policy::Policy;
use crate::wire::Wire;

/// 위로 올라오는 한 줄의 받는 쪽. ★명부 하나당 하나뿐이다★ — 갈라 쓰려면 소비자가 갈라 쓴다.
pub struct Inbound<W: Wire> {
    rx: mpsc::Receiver<Incoming<W>>,
}

impl<W: Wire> Inbound<W> {
    /// `None` = 명부가 통째로 닫혔다([`Registry`] 와 모든 [`Peer`] 가 사라졌다).
    pub async fn recv(&mut self) -> Option<Incoming<W>> {
        self.rx.recv().await
    }

    /// 기다리지 않고 한 개. 하네스가 지금까지 올라온 것을 걷을 때 쓴다.
    pub fn try_recv(&mut self) -> Option<Incoming<W>> {
        self.rx.try_recv().ok()
    }

    /// 더 받지 않는다. 이미 큐에 있는 것은 [`Inbound::recv`] 로 계속 꺼낼 수 있다.
    pub fn close(&mut self) {
        self.rx.close();
    }
}

struct RegistryInner<W: Wire> {
    wire: Arc<W>,
    dialer: Arc<dyn Dialer>,
    clock: Arc<dyn Clock>,
    policy: Policy,
    inbound: mpsc::Sender<Incoming<W>>,
    peers: Mutex<HashMap<PeerId, Peer<W>>>,
    /// 이름표별 세대 계수기. ★`remove` 로 지우지 않는다★ — 지우면 같은 이름표를 다시 올릴 때 세대가
    /// 0 으로 되감기고, 소비자의 평범한 방어(「최대 세대 미만은 무시」)가 새 연결을 통째로 버린다.
    ///
    /// 이 map 은 **소비자가 쓴 이름표 수**만큼 자란다(상대가 정하는 값이 아니다).
    generations: Mutex<HashMap<PeerId, Arc<AtomicU64>>>,
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
    /// 명부 하나를 세운다. ★인바운드는 이 한 줄로만 나온다★.
    pub fn new(
        wire: Arc<W>,
        dialer: Arc<dyn Dialer>,
        clock: Arc<dyn Clock>,
        policy: Policy,
    ) -> (Self, Inbound<W>) {
        debug_assert_eq!(policy.check(), Ok(()), "정책 값이 자기 불변식을 어긴다");
        let (tx, rx) = mpsc::channel(policy.inbound_queue.max(1));
        let registry = Self {
            inner: Arc::new(RegistryInner {
                wire,
                dialer,
                clock,
                policy,
                inbound: tx,
                peers: Mutex::new(HashMap::new()),
                generations: Mutex::new(HashMap::new()),
            }),
        };
        (registry, Inbound { rx })
    }

    /// 상대를 올리고 즉시 붙기 시작한다. 같은 이름표가 이미 있으면 **옛 것을 닫고 갈아치운다**.
    pub fn add(&self, id: PeerId, addr: Address, handshake: Arc<dyn Handshake>) -> Peer<W> {
        let policy = self.inner.policy.clone();
        self.add_with_policy(id, addr, handshake, policy)
    }

    /// 이 상대에게만 다른 정책을 건다.
    ///
    /// ★옛 감독을 먼저 닫고, 그 마지막 세대를 새 감독의 바닥값으로 넘긴다★ — 순서를 뒤집으면 두 소켓이
    /// 같은 상대에게 동시에 붙고, 바닥값을 안 넘기면 세대가 0 으로 되감겨 소비자의 평범한 방어
    /// (「최대 세대 미만은 무시」)가 **새 연결을 통째로 버린다**.
    ///
    /// ★그래도 겹침이 0 이 되지는 않는다★ — [`Peer::close`] 에 완료 신호가 없어 옛 감독은 잠시 더
    /// 통로를 닫는 중일 수 있고, 그 사이 운영 단계에 들어서 세대를 하나 더 발행할 수도 있다.
    /// **공용 계수기**가 그 겹침을 해롭지 않게 만드는 장치다(같은 번호가 두 번 나오지 않는다).
    pub fn add_with_policy(
        &self,
        id: PeerId,
        addr: Address,
        handshake: Arc<dyn Handshake>,
        policy: Policy,
    ) -> Peer<W> {
        debug_assert_eq!(policy.check(), Ok(()), "정책 값이 자기 불변식을 어긴다");
        let displaced = self.inner.peers.lock().unwrap().remove(&id);
        if let Some(old) = displaced {
            old.close();
        }
        // ★옛 감독의 마지막 세대를 **읽지** 않는다★ — 읽는 순간과 그 감독이 마지막 세대를 발행하는
        //   순간 사이에 경합이 있다(그 감독은 `shake_hands` 의 `select` 안에서 핸드셰이크와 `Close` 를
        //   동시에 준비된 채로 만날 수 있고, 핸드셰이크가 이기면 번호를 하나 더 발행한다). 계수기를
        //   물려주면 누가 이기든 같은 번호가 두 번 나오지 않는다.
        let generations = self
            .inner
            .generations
            .lock()
            .unwrap()
            .entry(id.clone())
            .or_insert_with(|| Arc::new(AtomicU64::new(0)))
            .clone();
        let peer = spawn_peer(PeerConfig {
            id: id.clone(),
            addr,
            wire: self.inner.wire.clone(),
            dialer: self.inner.dialer.clone(),
            clock: self.inner.clock.clone(),
            handshake,
            policy,
            inbound: self.inner.inbound.clone(),
            generations,
        });
        self.inner.peers.lock().unwrap().insert(id, peer.clone());
        peer
    }

    /// 명부에서 내리고 그 상대를 닫는다.
    pub fn remove(&self, id: &PeerId) {
        let removed = self.inner.peers.lock().unwrap().remove(id);
        if let Some(peer) = removed {
            peer.close();
        }
    }

    pub fn get(&self, id: &PeerId) -> Option<Peer<W>> {
        self.inner.peers.lock().unwrap().get(id).cloned()
    }

    /// 이름표 순으로 정렬한 지금 한 장. ★셸이 「앱 전체」를 접는 재료가 이것이다★.
    pub fn peers(&self) -> Vec<(PeerId, PeerState)> {
        let mut out: Vec<(PeerId, PeerState)> = self
            .inner
            .peers
            .lock()
            .unwrap()
            .iter()
            .map(|(id, peer)| (id.clone(), peer.peer_state()))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    pub fn len(&self) -> usize {
        self.inner.peers.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 전 상대에게 같은 것을 민다.
    ///
    /// ★한 번만 인코딩하고 프레임을 나눠 준다★ — 그래서 `W::Out` 에 `Clone` 을 요구하지 않는다. 대가로
    /// 팬아웃은 **알림 의미 고정**이다: 답장을 기다리지 않는다.
    ///
    /// 부분 실패는 그 상대의 [`TransportEvent::Dropped`] 로 올라온다 — ★조용히 사라지는 것을 안 만드는
    /// 것이 이 함수의 유일한 까다로운 부분이다★(오늘 `net` 의 팬아웃이 정확히 그 자리에서 조용하다).
    pub fn broadcast(&self, out: W::Out) {
        let frame = self.inner.wire.encode(&out);
        let peers: Vec<Peer<W>> = self.inner.peers.lock().unwrap().values().cloned().collect();
        for peer in peers {
            if peer.send_frame(frame.clone()).is_err() {
                let _ = self
                    .inner
                    .inbound
                    .try_send(Incoming::Event(TransportEvent::Dropped {
                        peer: peer.id().clone(),
                        direction: Direction::Outbound,
                        count: 1,
                    }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Direction;
    use crate::frame::Frame;
    use crate::machine::{GaveUpReason, Generation};
    use crate::testing::{
        settle, ImmediateHandshake, ManualClock, MemoryNetwork, RejectingHandshake, TestIn,
        TestOut, TestWire,
    };

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

    // ── N1: 공유 줄이 찼다고 연결을 끊지 않는다 ──
    #[tokio::test]
    async fn a_saturated_shared_line_does_not_cut_the_link() {
        let mut f = Fixture::new(Policy {
            inbound_queue: 2,
            ..Policy::default()
        });
        let peer = f
            .registry
            .add(PeerId::new("a"), Address::new("mem://a"), immediate());
        settle().await;
        let endpoint = f.net.accept().unwrap();
        // 아무도 안 읽는 채로 밀어 넣는다.
        for seq in 1..=200 {
            endpoint.push(TestWire::chunk(1, 1, seq));
        }
        settle().await;
        assert_eq!(
            peer.peer_state(),
            PeerState::Live,
            "위로 올라갈 자리가 없는 것은 이 연결의 잘못이 아니다"
        );

        // 자리가 나면 유실을 신고한다.
        f.drain();
        endpoint.push(TestWire::chunk(1, 1, 201));
        settle().await;
        let dropped = f.drain().into_iter().find_map(|i| match i.as_event() {
            Some(TransportEvent::Dropped {
                direction, count, ..
            }) => Some((*direction, *count)),
            _ => None,
        });
        assert!(
            matches!(dropped, Some((Direction::Inbound, n)) if n > 0),
            "세는 것을 그만두면 끊는 것보다 나쁘다"
        );
    }

    #[tokio::test]
    async fn a_flooding_peer_does_not_sever_a_healthy_peer() {
        let f = Fixture::new(Policy {
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

        // 그 뒤에 붙는 조용한 상대. 자기 `Connected` 사건조차 못 올린다.
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
