//! 상대 N 개 명부 — ★구동 층★. 상대마다 감독 하나를 세우고, 그 손잡이의 수신단([`Inbox`])을 올린 쪽에
//! 돌려준다.
//!
//! ★상대는 처음부터 N 개다★(ADR-0177 결정 3 · ADR-0179) — 하나뿐인 세계를 전제한 상태가 어디에도 없다.
//! **상대끼리 상태를 공유하지 않는다**: 데몬 A 가 포기해도 B 는 운영 중이다.
//!
//! ★위로 올라가는 줄도 공유하지 않는다 — 손잡이마다 따로다★(ADR-0310 결정 17 · TRD 3 §3-0 ① D24 ②) — 줄이
//! 하나면 시끄러운 상대가 그것을 채우는 순간 조용한 상대의 사건·패킷이 그 뒤에 붙들리고, 「A 하나 죽어도
//! B·C 는 잘 동작해야 한다」가 무너진다(ADR-0180 결정 8). 그래서 [`Registry::add`] 가 올릴 때마다 줄(길이 =
//! [`Policy::inbound_queue`])을 하나 세워 그 받는 끝을 돌려주고, 넣는 끝은 그 감독 하나만 쥔다. 줄이 차면 그
//! 감독이 자기 순서 칸에 붙들고 그 상대의 읽기를 세운다 — 버리지 않고 연결을 끊지도 않는다(`peer.rs` 헤더).
//! 그래서 받는 쪽이 한 수신단을 안 비우면 그 상대만 선다.
//!
//! ★같은 이름표를 다시 올려도 줄을 잇지 않는다★ — 옛 감독은 정리를 마칠 때까지 옛 수신단에, 새 감독은 새
//! 수신단에 넣는다. 그래서 수신단 하나에는 감독 하나의 것만 오고 그 수신단의 끝이 그 감독의 끝이다. 수신단
//! 사이의 순서는 없다 — 합친 보기가 필요하면 소비자 어댑터가 수신단들을 합친다(거르는 규칙은 TRD 3 §3-3).
//!
//! ★「앱 전체가 끊겼나」는 여기서 판정하지 않는다★ — 그건 화면의 개념이고 이 crate 는 상대를 셀 뿐
//! 앱을 모른다. 재료 둘([`Registry::peers`] 와 [`crate::Peer::reconnect_now`])을 주고 셸이 접는다
//! (ADR-0180 의 열린 질문이 그렇게 닫힌다).

use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::mpsc;

use crate::budget::Charged;
use crate::clock::Clock;
use crate::event::{Incoming, PeerId};
use crate::link::{Address, Dialer, Handshake};
use crate::machine::PeerState;
use crate::peer::{spawn_peer, Peer, PeerConfig, BYTE_BUDGET};
use crate::policy::Policy;
use crate::wire::Wire;

/// 손잡이 하나의 수신단 — 그것을 돌려준 [`Registry::add`] 가 띄운 감독 하나가 낸 것만 온다.
///
/// ★끝 = [`TransportEvent::Closed`](crate::TransportEvent::Closed) 다음의 `None`★ — 감독은 정리를 마치며 `Closed`
/// 를 마지막으로 싣고 넣는 끝을 놓는다. ★`Closed` 없이 `None` 이 오면 — 받는 쪽이 스스로 [`Inbox::close`] 를
/// 부른 게 아니라면 — 감독이 정상 정리 없이 끝났다★: 정리가 상한(`peer.rs` 의 `CLOSE_DRAIN_LIMIT`)을 넘겨 남은
/// 것을 `Closed` 까지 버렸거나(세지도 신고하지도 않는다), 태스크가 비정상으로 끝났다(패닉 · 런타임 종료 — 넣는
/// 끝이 태스크와 함께 떨어진다). 상한 초과로만 읽지 말 것. [`Inbox::close`] 를 부른 뒤의 `None` 은 감독의 끝이
/// 아니다 — 감독은 아직 돌고 있을 수 있다. ★끝까지 비운다★ — crate 헤더 「받는 쪽 불변식」 5.
///
/// 항목마다 감독의 바이트 몫이 실려 있고 꺼낼 때 떨어진다(`budget.rs`). 수신단을 떨어뜨리면 든 것의 몫도 함께
/// 떨어진다.
pub struct Inbox<W: Wire> {
    rx: mpsc::Receiver<Charged<Incoming<W>>>,
}

impl<W: Wire> Inbox<W> {
    /// `None` = 끝 — 감독이 넣는 끝을 놓았거나 [`Inbox::close`] 를 불렀고, 든 것을 다 꺼냈다.
    /// ★취소 안전★ — `select!` 팔에 걸어 다른 팔이 이겨도 항목을 잃지 않는다.
    pub async fn recv(&mut self) -> Option<Incoming<W>> {
        self.rx.recv().await.map(Charged::into_item)
    }

    /// 기다리지 않고 한 개. `None` = 지금 든 것이 없거나 끝났다 — 둘을 가르려면 [`Inbox::recv`] 를 쓴다.
    pub fn try_recv(&mut self) -> Option<Incoming<W>> {
        self.rx.try_recv().ok().map(Charged::into_item)
    }

    /// 새로 넣는 것만 막는다. 이미 든 것(이미 실린 `Closed` 포함)은 [`Inbox::recv`] · [`Inbox::try_recv`] 로 계속
    /// 꺼낼 수 있고, 다 꺼내면 감독이 아직 돌고 있어도 `None` 이다. ★감독이 순서 칸에 붙들어 아직 못 실은 것은
    /// 말없이 버려진다★ — 닫힌 줄에는 실을 수 없고, 세지도 신고하지도 않는다. [`Inbox`] 를 떨어뜨려도 같다(든
    /// 것까지 함께 버린다).
    ///
    /// 감독은 다음에 넣으려다 닫힌 것을 보고 다시 붙지 않고 접는다.
    /// TODO(TRD 3 §6-0 U1d): 넣을 것이 없어도 곧바로 접는다(대기 루프마다 `Sender::closed()` 갈래 — crate 헤더
    /// 「받는 쪽 불변식」 5).
    pub fn close(&mut self) {
        self.rx.close();
    }
}

struct RegistryInner<W: Wire> {
    wire: Arc<W>,
    dialer: Arc<dyn Dialer>,
    clock: Arc<dyn Clock>,
    policy: Policy,
    /// 올라 있는 이름표의 손잡이. ★자물쇠 안에서는 기다리지 않는다★ — 감독 띄우기 · 닫기는 자물쇠 밖이다.
    peers: Mutex<HashMap<PeerId, Peer<W>>>,
    /// 이름표별 세대 계수기. ★`remove` 로 지우지 않는다★ — 지우면 같은 이름표를 다시 올릴 때 세대가
    /// 0 으로 되감기고, 소비자의 평범한 방어(「최대 세대 미만은 무시」)가 새 연결을 통째로 버린다.
    ///
    /// 이 map 은 **소비자가 쓴 이름표 수**만큼 자란다(상대가 정하는 값이 아니다).
    generations: Mutex<HashMap<PeerId, Arc<AtomicU64>>>,
}

impl<W: Wire> RegistryInner<W> {
    /// 독 걸린 자물쇠를 그대로 다시 쓴다 — 자물쇠 안의 갱신은 패닉할 자리 없이 한 칸씩이라 반쯤 바뀐 표가
    /// 남지 않고, 독을 그대로 두면 한 번의 패닉이 명부를 영영 못 쓰게 만든다.
    fn peers(&self) -> MutexGuard<'_, HashMap<PeerId, Peer<W>>> {
        self.peers.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// 상대 명부. clone 은 같은 명부를 가리킨다.
///
/// ★마지막 clone 이 사라지면 명부가 쥔 손잡이를 놓을 뿐이다★ — 감독은 그 상대의 손잡이([`Peer`])가 다
/// 사라질 때 닫기로 알아듣고 정리하며, 그 수신단은 `Closed` → `None` 으로 끝난다. 소비자가 손잡이를 쥐고
/// 있으면 그 감독은 계속 돈다.
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
    /// 명부 하나를 세운다. ★받는 것은 [`Registry::add`] 가 손잡이마다 돌려주는 [`Inbox`] 로만 나온다★.
    pub fn new(
        wire: Arc<W>,
        dialer: Arc<dyn Dialer>,
        clock: Arc<dyn Clock>,
        policy: Policy,
    ) -> Self {
        debug_assert_eq!(policy.check(), Ok(()), "정책 값이 자기 불변식을 어긴다");
        Self {
            inner: Arc::new(RegistryInner {
                wire,
                dialer,
                clock,
                policy,
                peers: Mutex::new(HashMap::new()),
                generations: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// 상대를 올리고 즉시 붙기 시작한다 — 그 손잡이와 수신단을 돌려준다. 같은 이름표가 이미 있으면 **옛 것을
    /// 닫고 갈아치운다**.
    ///
    /// ★tokio 런타임 안에서 불러야 한다★ — 감독 태스크를 띄우므로 밖에서 부르면 패닉한다(명부는 그 뒤에도
    /// 쓸 수 있다). 그렇게 패닉해도 같은 이름표로 올라 있던 상대는 이미 명부에서 내려가 닫힌 뒤다.
    pub fn add(
        &self,
        id: PeerId,
        addr: Address,
        handshake: Arc<dyn Handshake>,
    ) -> (Peer<W>, Inbox<W>) {
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
    /// — 바닥값(옛 감독의 마지막 세대)을 그대로 실어, 그 감독의
    /// [`TransportEvent::Closed`](crate::TransportEvent::Closed) 와 [`Peer::generation`] 은 옛 감독의 것과 같은
    /// 번호를 보인다. 어느 감독의 것인지는 수신단이 가른다.
    ///
    /// ★같은 이름표라도 수신단은 새것이다★ — 옛 감독은 정리를 마칠 때까지 옛 수신단에 넣고, 그 끝은 `Closed`
    /// 다음의 `None` 이다. 옛 수신단을 놓으면 옛 감독의 정리가 기다리지 않고 끝나고, 쥔 채 안 비우면
    /// `peer.rs` 의 `CLOSE_DRAIN_LIMIT` 까지 남는다.
    pub fn add_with_policy(
        &self,
        id: PeerId,
        addr: Address,
        handshake: Arc<dyn Handshake>,
        policy: Policy,
    ) -> (Peer<W>, Inbox<W>) {
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
        let displaced = self.inner.peers().remove(&id);
        if let Some(old) = displaced {
            old.close();
        }
        let (inbound, rx) = mpsc::channel(policy.inbound_queue.max(1));
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
        });
        let displaced = self.inner.peers().insert(id, peer.clone());
        // 같은 이름표를 동시에 올린 다른 호출이 그 사이에 끼었으면 먼저 들어간 쪽을 닫는다.
        if let Some(other) = displaced {
            other.close();
        }
        (peer, Inbox { rx })
    }

    /// 명부에서 내리고 그 상대를 닫는다. 그 손잡이의 수신단은 감독이 정리를 마치면 `Closed` → `None` 으로
    /// 끝난다.
    pub fn remove(&self, id: &PeerId) {
        let removed = self.inner.peers().remove(id);
        if let Some(peer) = removed {
            peer.close();
        }
    }

    pub fn get(&self, id: &PeerId) -> Option<Peer<W>> {
        self.inner.peers().get(id).cloned()
    }

    /// 이름표 순으로 정렬한 지금 한 장. ★셸이 「앱 전체」를 접는 재료가 이것이다★.
    pub fn peers(&self) -> Vec<(PeerId, PeerState)> {
        let mut out: Vec<(PeerId, PeerState)> = self
            .inner
            .peers()
            .iter()
            .map(|(id, peer)| (id.clone(), peer.peer_state()))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    pub fn len(&self) -> usize {
        self.inner.peers().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 전 상대에게 같은 것을 민다.
    ///
    /// ★한 번만 인코딩하고 프레임을 나눠 준다★ — 그래서 `W::Out` 에 `Clone` 을 요구하지 않는다. 대가로
    /// 팬아웃은 **알림 의미 고정**이다: 답장을 기다리지 않는다.
    ///
    /// 부분 실패는 그 상대의 [`TransportEvent::Dropped`](crate::TransportEvent::Dropped) 로 **그 상대의
    /// 수신단에** 올라온다 — ★조용히 사라지는 것을 안 만드는 것이 이 함수의 유일한 까다로운 부분이다★(오늘
    /// `net` 의 팬아웃이 정확히 그 자리에서 조용하다). 세고 신고하는 것은 그 상대의 감독이라, 신고는 그 감독이
    /// 낸 다른 것과 같은 순서 칸을 지나고 줄이 차 있어도 버려지지 않는다(`Peer::send_frame`).
    ///
    /// ★앱이 [`Peer::close`] 를 부른 상대가 운영 단계가 아니라서 못 넣은 것은 신고하지 않는다★ — 명부에 남아
    /// 있어도 같다. 앱이 닫겠다고 한 뒤의 유실 개수는 읽을 쪽이 없고 깔끔한 종료가 우선이다(사용자 결정
    /// 2026-09-26 — `lib.rs` 「알려진 한계」). 그 전에 큐에 든 것은 감독이 센다.
    pub fn broadcast(&self, out: W::Out) {
        let frame = self.inner.wire.encode(&out);
        let targets: Vec<Peer<W>> = self.inner.peers().values().cloned().collect();
        for peer in targets {
            // 실패는 그 상대의 감독이 센다(`Peer::send_frame`).
            let _ = peer.send_frame(frame.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Direction, TransportEvent};
    use crate::frame::Frame;
    use crate::link::HandshakeStep;
    use crate::machine::{GaveUpReason, Generation};
    use crate::peer::CLOSE_DRAIN_LIMIT;
    use crate::testing::{
        settle, settle_until, ImmediateHandshake, ManualClock, MemoryNetwork, RejectingHandshake,
        TestIn, TestOut, TestWire,
    };
    use futures_util::future::poll_immediate;
    use std::time::Duration;

    struct Fixture {
        registry: Registry<TestWire>,
        net: MemoryNetwork,
        clock: ManualClock,
    }

    impl Fixture {
        fn new(policy: Policy) -> Self {
            let net = MemoryNetwork::new();
            let clock = ManualClock::new();
            let registry = Registry::new(Arc::new(TestWire), net.dialer(), clock.handle(), policy);
            Self {
                registry,
                net,
                clock,
            }
        }

        /// ★돌려받은 수신단을 쥐고 있어야 한다★ — 떨어뜨리면 감독이 다음에 넣으려다 접는다.
        fn add(
            &self,
            name: &str,
            handshake: Arc<dyn Handshake>,
        ) -> (Peer<TestWire>, Inbox<TestWire>) {
            self.registry.add(
                PeerId::new(name),
                Address::new(format!("mem://{name}")),
                handshake,
            )
        }
    }

    fn immediate() -> Arc<dyn Handshake> {
        Arc::new(ImmediateHandshake)
    }

    /// 감독 태스크를 비정상으로 끝낸다 — 통로가 열리면 `start` 에서 패닉한다.
    struct PanickingHandshake;

    impl Handshake for PanickingHandshake {
        fn start(&self) -> HandshakeStep {
            panic!("시험: 핸드셰이크 시작에서 일부러 패닉한다");
        }

        fn on_frame(&self, _frame: &Frame) -> HandshakeStep {
            HandshakeStep::Done
        }
    }

    fn notice(body: &str) -> Frame {
        TestWire::encode_in(&TestIn::Notice(body.into()))
    }

    fn drain(inbox: &mut Inbox<TestWire>) -> Vec<Incoming<TestWire>> {
        let mut out = Vec::new();
        while let Some(item) = inbox.try_recv() {
            out.push(item);
        }
        out
    }

    /// 감독들을 돌리며 비운다 — 감독이 순서 칸에 붙든 것을 줄로 옮길 틈을 주면서, 한 바퀴에 아무것도 안
    /// 올라올 때까지.
    async fn drain_until_quiet(inbox: &mut Inbox<TestWire>) -> Vec<Incoming<TestWire>> {
        let mut out = Vec::new();
        loop {
            settle().await;
            let batch = drain(inbox);
            if batch.is_empty() {
                return out;
            }
            out.extend(batch);
        }
    }

    /// 지금 끝나 있나 — 든 것이 없고 넣는 끝이 놓였다(또는 닫혔다). ★든 것이 있으면 하나를 꺼내 버린다★ —
    /// 다 꺼낸 뒤에 부른다.
    async fn ended(inbox: &mut Inbox<TestWire>) -> bool {
        matches!(poll_immediate(inbox.recv()).await, Some(None))
    }

    fn label(item: &Incoming<TestWire>) -> String {
        match item {
            Incoming::Message(arrival) => match &arrival.msg {
                TestIn::Notice(body) => format!("msg:{body}"),
                other => format!("msg:{other:?}"),
            },
            Incoming::Event(TransportEvent::Connected { generation, .. }) => {
                format!("Connected:{}", generation.0)
            }
            Incoming::Event(TransportEvent::Disconnected {
                generation, cause, ..
            }) => format!("Disconnected:{}:{cause:?}", generation.0),
            Incoming::Event(TransportEvent::Closed {
                generation, cause, ..
            }) => format!("Closed:{}:{cause:?}", generation.0),
            Incoming::Event(other) => other.kind().to_string(),
        }
    }

    fn labels(items: &[Incoming<TestWire>]) -> Vec<String> {
        items.iter().map(label).collect()
    }

    fn kinds(items: &[Incoming<TestWire>]) -> Vec<&'static str> {
        items
            .iter()
            .filter_map(|i| i.as_event().map(|e| e.kind()))
            .collect()
    }

    fn chunk_seqs(items: &[Incoming<TestWire>]) -> Vec<u64> {
        items
            .iter()
            .filter_map(|i| match i.as_message().map(|a| &a.msg) {
                Some(TestIn::Chunk { seq, .. }) => Some(*seq),
                _ => None,
            })
            .collect()
    }

    fn outbound_losses(items: &[Incoming<TestWire>]) -> Vec<u64> {
        items
            .iter()
            .filter_map(|i| match i.as_event() {
                Some(TransportEvent::Dropped {
                    direction: Direction::Outbound,
                    count,
                    ..
                }) => Some(*count),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn every_arrival_carries_the_peer_it_came_from() {
        let f = Fixture::new(Policy::default());
        let (_a, mut inbox_a) = f.add("a", immediate());
        settle().await;
        let ea = f.net.accept().unwrap();
        let (_b, mut inbox_b) = f.add("b", immediate());
        settle().await;
        let eb = f.net.accept().unwrap();

        ea.push(notice("from-a"));
        eb.push(notice("from-b"));
        settle().await;

        for (inbox, name) in [(&mut inbox_a, "a"), (&mut inbox_b, "b")] {
            let items = drain(inbox);
            assert!(
                items.iter().all(|i| i.peer() == &PeerId::new(name)),
                "★수신단에는 그 손잡이의 감독이 낸 것만 온다★"
            );
            assert_eq!(
                labels(&items),
                vec!["Connected:1".to_string(), format!("msg:from-{name}")]
            );
        }
    }

    #[tokio::test]
    async fn messages_and_events_share_one_line() {
        let f = Fixture::new(Policy::default());
        let (_peer, mut inbox) = f.add("a", immediate());
        settle().await;
        let endpoint = f.net.accept().unwrap();
        endpoint.push(notice("x"));
        settle().await;

        let items = drain(&mut inbox);
        assert!(items.iter().any(|i| i.as_event().is_some()));
        assert!(items.iter().any(|i| i.as_message().is_some()));
    }

    #[tokio::test]
    async fn one_peer_giving_up_leaves_the_others_alone() {
        let f = Fixture::new(Policy::default());
        let (good, _good_inbox) = f.add("good", immediate());
        settle().await;
        let _endpoint = f.net.accept().unwrap();

        let (bad, mut bad_inbox) = f.add("bad", RejectingHandshake::new("nope"));
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
        assert_eq!(kinds(&drain(&mut bad_inbox)), vec!["Rejected", "GaveUp"]);
    }

    #[tokio::test]
    async fn broadcast_reaches_everyone_with_a_single_encode() {
        let f = Fixture::new(Policy::default());
        let (_a, _inbox_a) = f.add("a", immediate());
        settle().await;
        let ea = f.net.accept().unwrap();
        let (_b, _inbox_b) = f.add("b", immediate());
        settle().await;
        let eb = f.net.accept().unwrap();

        f.registry.broadcast(TestOut::Notice("all".into()));
        settle().await;
        assert_eq!(ea.frames(), vec![Frame::Text("not|all".into())]);
        assert_eq!(eb.frames(), vec![Frame::Text("not|all".into())]);
    }

    // 포기한 상대는 제어만 기다리며 잠들어 있다 — 그 감독이 스스로 깨어 신고해야 한다.
    #[tokio::test]
    async fn a_partial_broadcast_failure_shows_up_as_that_peers_event() {
        let f = Fixture::new(Policy::default());
        let (_up, mut up_inbox) = f.add("up", immediate());
        settle().await;
        let ea = f.net.accept().unwrap();
        let (down, mut down_inbox) = f.add("down", RejectingHandshake::new("nope"));
        settle().await;
        assert!(matches!(down.peer_state(), PeerState::GaveUp(_)));
        drain(&mut up_inbox);
        drain(&mut down_inbox);

        f.registry.broadcast(TestOut::Notice("all".into()));
        settle().await;
        assert_eq!(ea.frames(), vec![Frame::Text("not|all".into())]);
        assert_eq!(outbound_losses(&drain(&mut down_inbox)), vec![1]);
        assert!(
            outbound_losses(&drain(&mut up_inbox)).is_empty(),
            "받은 상대의 수신단에는 신고가 없다"
        );
    }

    #[tokio::test]
    async fn remove_closes_the_peer_and_drops_it_from_the_roster() {
        let f = Fixture::new(Policy::default());
        let (peer, mut inbox) = f.add("a", immediate());
        settle().await;
        assert_eq!(f.registry.len(), 1);
        assert!(f.registry.get(&PeerId::new("a")).is_some());

        f.registry.remove(&PeerId::new("a"));
        settle().await;
        assert!(f.registry.is_empty());
        assert_eq!(peer.peer_state(), PeerState::Closed);
        assert_eq!(
            labels(&drain(&mut inbox)),
            vec![
                "Connected:1",
                "Disconnected:1:Explicit",
                "Closed:1:Explicit"
            ]
        );
        assert!(ended(&mut inbox).await, "내린 상대의 수신단은 끝난다");
    }

    #[tokio::test]
    async fn re_adding_the_same_name_replaces_the_old_supervisor() {
        let f = Fixture::new(Policy::default());
        let (first, _first_inbox) = f.add("a", immediate());
        settle().await;
        let (second, _second_inbox) =
            f.registry
                .add(PeerId::new("a"), Address::new("mem://a2"), immediate());
        settle().await;
        assert_eq!(first.peer_state(), PeerState::Closed);
        assert_eq!(second.peer_state(), PeerState::Live);
        assert_eq!(f.registry.len(), 1);
    }

    // ── N7: 같은 이름표를 다시 올려도 세대가 되감기지 않는다 ──
    #[tokio::test]
    async fn re_adding_the_same_name_never_rewinds_the_generation() {
        let f = Fixture::new(Policy::default());
        let (first, _first_inbox) = f.add("a", immediate());
        settle().await;
        assert_eq!(first.generation(), Generation(1));

        let (second, _second_inbox) =
            f.registry
                .add(PeerId::new("a"), Address::new("mem://a2"), immediate());
        settle().await;
        assert_eq!(
            second.generation(),
            Generation(2),
            "★0 으로 되감기면 「최대 세대 미만은 무시」 방어가 새 연결을 통째로 버린다★"
        );
    }

    #[tokio::test]
    async fn the_old_supervisor_is_told_to_close_before_the_new_one_dials() {
        let f = Fixture::new(Policy::default());
        let (first, _first_inbox) = f.add("a", immediate());
        settle().await;
        let (_second, _second_inbox) =
            f.registry
                .add(PeerId::new("a"), Address::new("mem://a2"), immediate());
        // 아직 아무 태스크도 안 돌린 시점 — 옛 핸들은 이미 종료 신호를 받았어야 한다.
        settle().await;
        assert_eq!(first.peer_state(), PeerState::Closed);
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
        let (_peer, _inbox) = f.registry.add_with_policy(
            PeerId::new("a"),
            Address::new("mem://a"),
            immediate(),
            strict,
        );
        settle().await;
        f.clock.advance(Duration::from_millis(500));
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
        let f = Fixture::new(Policy {
            inbound_queue: 2,
            ..Policy::default()
        });
        let (peer, mut inbox) = f.add("a", immediate());
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

        let items = drain_until_quiet(&mut inbox).await;
        assert_eq!(
            chunk_seqs(&items),
            (1..=200).collect::<Vec<_>>(),
            "★줄이 찬 동안 온 것도 하나도 안 버려지고 온 순서대로 나온다★"
        );
        assert_eq!(
            kinds(&items),
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
        let f = Fixture::new(Policy::default());
        let (_peer, mut inbox) = f.add("a", immediate());
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

        let items = drain_until_quiet(&mut inbox).await;
        assert_eq!(chunk_seqs(&items), (1..=4096).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn a_flooding_peer_does_not_sever_a_healthy_peer() {
        let f = Fixture::new(Policy {
            inbound_queue: 2,
            ..Policy::default()
        });
        let (noisy, _noisy_inbox) = f.add("noisy", immediate());
        settle().await;
        let noisy_link = f.net.accept().unwrap();
        // 아무도 안 읽는 수신단을 가득 채운다.
        for seq in 1..=500 {
            noisy_link.push(TestWire::chunk(1, 1, seq));
        }
        settle().await;

        let (quiet, mut quiet_inbox) = f.add("quiet", immediate());
        settle().await;
        assert_eq!(
            quiet.peer_state(),
            PeerState::Live,
            "★남의 홍수로 내 연결이 끊기면 ADR-0180 결정 8 이 깨진다★"
        );
        assert_eq!(noisy.peer_state(), PeerState::Live);
        assert_eq!(f.net.dials(), 2, "재연결이 돌지 않았다");
        assert_eq!(
            kinds(&drain(&mut quiet_inbox)),
            vec!["Connected"],
            "남의 홍수가 내 Connected 를 버리게 하지 못한다"
        );
    }

    // ── 3-0 U1d: 안 비운 수신단은 그 상대만 세운다 ──
    #[tokio::test]
    async fn an_undrained_inbox_stalls_only_its_own_peer() {
        let f = Fixture::new(Policy {
            inbound_queue: 64,
            frame_buffer: 8,
            ..Policy::default()
        });
        let (noisy, mut noisy_inbox) = f.add("noisy", immediate());
        settle().await;
        let noisy_link = f.net.accept().unwrap();
        // 아무도 안 읽는 채로 시끄러운 쪽이 자기 수신단을 넘치게 채운다.
        for seq in 1..=500 {
            noisy_link.push(TestWire::chunk(1, 1, seq));
        }
        settle().await;

        let (quiet, mut quiet_inbox) = f.add("quiet", immediate());
        settle().await;
        let quiet_link = f.net.accept().unwrap();
        for n in 0..3 {
            quiet_link.push(notice(&format!("q{n}")));
        }
        settle().await;
        assert!(
            noisy_link.unread() > 0,
            "★안 비운 수신단의 상대는 자기 통로에서 배압을 받는다★ — 줄이 찬 뒤로는 그 통로를 안 읽는다"
        );
        assert_eq!(
            quiet_link.unread(),
            0,
            "★조용한 쪽의 통로는 남의 배압과 무관하게 다 읽힌다★"
        );
        assert_eq!(
            labels(&drain(&mut quiet_inbox)),
            vec!["Connected:1", "msg:q0", "msg:q1", "msg:q2"],
            "★조용한 상대의 것은 하나도 안 버려지고, 시끄러운 쪽이 안 비워진 채여도 한 번의 정착 안에 다 와 있다★"
        );
        assert_eq!(noisy.peer_state(), PeerState::Live);
        assert_eq!(quiet.peer_state(), PeerState::Live);

        // 넘친 시끄러운 쪽의 것도 그 감독이 붙들고 있다가 순서대로 낸다.
        let noisy_items = drain_until_quiet(&mut noisy_inbox).await;
        assert_eq!(chunk_seqs(&noisy_items), (1..=500).collect::<Vec<_>>());
    }

    // ── 3-0 U1c: 명부가 바이트 예산을 감독에게 넘긴다 ──
    //
    // ★실제 예산 값으로 잰다★ — 감독 시험은 작은 예산을 직접 넣으므로, 명부가 엉뚱한 값(0 · 상한 없음)을 넘겨도
    //   거기서는 안 보인다. 스트림을 안 쓰는 상대는 예산 전부(16 MiB)가 읽기 몫이라 그 절반보다 큰 두 장이면 둘째에서
    //   읽기 문이 닫힌다(net `CONN_RX_MAX_BYTES` 시험과 같은 모양 — 같은 값이다).
    #[tokio::test]
    async fn the_registry_stops_reading_a_peer_once_it_holds_the_byte_budget() {
        let f = Fixture::new(Policy::default());
        let (peer, mut inbox) = f.add("a", immediate());
        settle().await;
        let endpoint = f.net.accept().unwrap();
        drain(&mut inbox);
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

        let tags: Vec<u8> = drain_until_quiet(&mut inbox)
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
        let f = Fixture::new(Policy {
            inbound_queue: 2,
            ..Policy::default()
        });
        let (_peer, mut inbox) = f.registry.add_with_policy(
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
        let messages = drain(&mut inbox)
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
        let f = Fixture::new(Policy {
            inbound_queue: 4,
            ..Policy::default()
        });
        let (_noisy, _noisy_inbox) = f.add("noisy", immediate());
        settle().await;
        let noisy_link = f.net.accept().unwrap();
        for seq in 1..=500 {
            noisy_link.push(TestWire::chunk(1, 1, seq));
        }
        settle().await;
        let (_down, mut down_inbox) = f.add("down", RejectingHandshake::new("nope"));
        settle().await;
        drain(&mut down_inbox);

        f.registry.broadcast(TestOut::Notice("all".into()));
        settle().await;
        assert_eq!(outbound_losses(&drain(&mut down_inbox)), vec![1]);
    }

    // ── 3-0 U1d: 팬아웃 유실은 그 상대의 감독이 세어 자기 수신단에 낸다 ──
    //
    // ★시계를 밀지 않는다★ — 백오프 대기가 시계로 깨면 신고가 그 바퀴 첫머리에 묻어 나와, 감독이 스스로 깼는지를
    //   못 가린다. 줄 길이 1 = `ConnectFailed` 한 장으로 찬 줄이다.
    #[tokio::test]
    async fn a_broadcast_loss_wakes_a_backing_off_supervisor_and_survives_a_full_inbox() {
        let f = Fixture::new(Policy {
            inbound_queue: 1,
            ..Policy::default()
        });
        f.net.fail_forever();
        let (down, mut inbox) = f.add("down", immediate());
        settle_until("백오프", || down.peer_state() == PeerState::Backoff).await;

        f.registry.broadcast(TestOut::Notice("all".into()));
        settle().await;
        let items = drain_until_quiet(&mut inbox).await;
        assert_eq!(
            kinds(&items),
            vec!["ConnectFailed", "Dropped"],
            "★찬 수신단이어도 신고가 버려지지 않고 감독이 낸 순서대로 온다★"
        );
        assert_eq!(outbound_losses(&items), vec![1]);
        assert_eq!(f.net.dials(), 1, "백오프가 끝나 다시 건 것이 아니다");
    }

    #[tokio::test]
    async fn a_waiting_recv_wakes_for_its_first_event_and_for_later_messages() {
        let f = Fixture::new(Policy::default());
        let (_peer, mut inbox) = f.add("late", immediate());
        let waiting = tokio::spawn(async move {
            let first = inbox.recv().await;
            (first, inbox)
        });
        settle().await;
        let (first, mut inbox) = tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("기다리던 recv 가 첫 사건을 못 봤다 — 영구 대기 회귀다")
            .expect("받는 태스크가 패닉했다");
        assert!(matches!(
            first.as_ref().and_then(|i| i.as_event()),
            Some(TransportEvent::Connected { peer, .. }) if peer == &PeerId::new("late")
        ));

        let link = f.net.accept().unwrap();
        let waiting = tokio::spawn(async move { inbox.recv().await });
        settle().await;
        assert!(!waiting.is_finished());
        link.push(notice("wake"));
        let next = tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("기다리던 recv 가 새 항목을 못 봤다")
            .expect("받는 태스크가 패닉했다");
        assert!(matches!(
            next.as_ref().and_then(|i| i.as_message()).map(|a| &a.msg),
            Some(TestIn::Notice(body)) if body == "wake"
        ));
    }

    #[tokio::test]
    async fn closing_an_inbox_folds_its_peer_and_keeps_what_was_queued() {
        let f = Fixture::new(Policy::default());
        let (early, mut inbox) = f.add("a", immediate());
        settle().await;
        let link = f.net.accept().unwrap();
        let (other, _other_inbox) = f.add("b", immediate());
        settle().await;
        link.push(notice("before"));
        settle().await;

        inbox.close();
        link.push(notice("after"));
        settle_until("a 접힘", || early.peer_state() == PeerState::Closed).await;
        assert_eq!(
            other.peer_state(),
            PeerState::Live,
            "★한 수신단을 닫아도 다른 상대는 그대로다★"
        );
        assert_eq!(f.net.dials(), 2, "소비자 부재는 재연결할 일이 아니다");

        let items = drain(&mut inbox);
        assert_eq!(
            labels(&items),
            vec!["Connected:1", "msg:before"],
            "닫기 전에 든 것은 꺼낼 수 있다"
        );
        assert!(ended(&mut inbox).await, "닫힌 수신단은 다 꺼내면 끝난다");
    }

    #[tokio::test]
    async fn dropping_an_inbox_folds_its_peer_and_a_re_added_name_starts_afresh() {
        let f = Fixture::new(Policy::default());
        let (early, inbox) = f.add("a", immediate());
        settle().await;
        let link = f.net.accept().unwrap();
        drop(inbox);
        link.push(notice("nobody listens"));
        settle_until("a 접힘", || early.peer_state() == PeerState::Closed).await;
        assert_eq!(f.net.dials(), 1, "받는 쪽이 사라진 상대는 다시 걸지 않는다");

        let (again, mut fresh) = f.add("a", immediate());
        settle_until("다시 올린 a 운영", || {
            again.peer_state() == PeerState::Live
        })
        .await;
        assert_eq!(
            f.net.dials(),
            2,
            "★옛 수신단을 놓은 것이 같은 이름표의 새 손잡이에 번지지 않는다★"
        );
        assert_eq!(labels(&drain(&mut fresh)), vec!["Connected:2"]);
    }

    // ── 3-0 U1d: 같은 이름표를 다시 올리면 옛 수신단은 옛 감독의 끝에서 끝나고 새 수신단은 새 감독의 것만 받는다 ──
    #[tokio::test]
    async fn a_re_added_name_ends_the_old_inbox_and_starts_a_new_one() {
        let f = Fixture::new(Policy::default());
        let (_old, mut old_inbox) = f.add("a", RejectingHandshake::new("nope"));
        settle().await;
        let (_new, mut new_inbox) =
            f.registry
                .add(PeerId::new("a"), Address::new("mem://a2"), immediate());
        settle().await;

        assert_eq!(
            kinds(&drain(&mut old_inbox)),
            vec!["Rejected", "GaveUp", "Closed"]
        );
        assert!(
            ended(&mut old_inbox).await,
            "★옛 수신단은 옛 감독의 Closed 뒤에 끝난다★"
        );
        assert_eq!(
            labels(&drain(&mut new_inbox)),
            vec!["Connected:1"],
            "★새 수신단에는 새 감독의 것만 온다★"
        );
        assert!(!ended(&mut new_inbox).await);
    }

    // ★옛 감독은 통로를 다 닫은 뒤에야 `Closed` 를 내고, 그 사이 새 감독이 붙는다★ — 한 줄이었으면 둘이 섞여
    //   어느 감독의 것인지 세대로 가려야 했다.
    #[tokio::test]
    async fn a_name_re_added_while_its_old_supervisor_is_still_closing_splits_by_inbox() {
        let f = Fixture::new(Policy::default());
        let (_old, mut old_inbox) = f.add("a", immediate());
        settle().await;
        drain(&mut old_inbox);
        f.registry.remove(&PeerId::new("a"));
        let (_new, mut new_inbox) =
            f.registry
                .add(PeerId::new("a"), Address::new("mem://a2"), immediate());
        settle().await;

        assert_eq!(
            labels(&drain(&mut old_inbox)),
            vec!["Disconnected:1:Explicit", "Closed:1:Explicit"],
            "옛 감독 안에서는 끊김 → 끝 순서다"
        );
        assert!(ended(&mut old_inbox).await);
        assert_eq!(
            labels(&drain(&mut new_inbox)),
            vec!["Connected:2"],
            "★옛 감독의 꼬리는 새 수신단에 섞이지 않는다★"
        );
    }

    #[tokio::test]
    async fn every_inbox_ends_with_closed_then_none_even_when_read_after_later_re_adds() {
        let f = Fixture::new(Policy {
            inbound_queue: 4,
            ..Policy::default()
        });
        let name = PeerId::new("a");
        let mut inboxes = Vec::new();
        for _ in 0..3 {
            let (_peer, inbox) = f
                .registry
                .add(name.clone(), Address::new("mem://a"), immediate());
            inboxes.push(inbox);
            settle().await;
            f.registry.remove(&name);
            settle().await;
        }
        for (at, inbox) in inboxes.iter_mut().enumerate() {
            let g = at + 1;
            assert_eq!(
                labels(&drain(inbox)),
                vec![
                    format!("Connected:{g}"),
                    format!("Disconnected:{g}:Explicit"),
                    format!("Closed:{g}:Explicit"),
                ],
                "★Closed 가 마지막 항목이다★ — 뒤에 올린 감독의 것이 끼지 않는다"
            );
            assert!(
                ended(inbox).await,
                "★Closed 뒤에는 감독이 넣는 끝을 놓아 None 이다★"
            );
        }
    }

    #[tokio::test]
    async fn churning_a_name_before_its_supervisor_runs_never_dials_and_ends_every_inbox() {
        let f = Fixture::new(Policy::default());
        let name = PeerId::new("a");
        let mut inboxes = Vec::new();
        for _ in 0..30 {
            // 감독이 돌기 전에 내린다 — 감독은 걸기 전에 닫기를 보고 `Closed` 하나만 낸 채 끝난다.
            let (_peer, inbox) = f
                .registry
                .add(name.clone(), Address::new("mem://a"), immediate());
            f.registry.remove(&name);
            inboxes.push(inbox);
            settle().await;
        }
        assert_eq!(f.net.dials(), 0);
        for inbox in &mut inboxes {
            assert_eq!(kinds(&drain(inbox)), vec!["Closed"]);
            assert!(ended(inbox).await);
        }
    }

    // ── 3-0 U1d: 정리가 상한을 넘기면 `Closed` 없이 수신단이 끝난다 ──
    //
    // 줄 길이 1 = `Connected` 한 장으로 찬 줄 — 그 뒤의 알림 · 끊김 · `Closed` 는 감독의 순서 칸에서 기다린다.
    #[tokio::test]
    async fn an_inbox_nobody_drains_ends_without_closed_once_the_teardown_gives_up() {
        for (waited, expected) in [
            (
                CLOSE_DRAIN_LIMIT - Duration::from_secs(1),
                vec![
                    "Connected:1",
                    "msg:held",
                    "Disconnected:1:Explicit",
                    "Closed:1:Explicit",
                ],
            ),
            (CLOSE_DRAIN_LIMIT, vec!["Connected:1"]),
        ] {
            let f = Fixture::new(Policy {
                inbound_queue: 1,
                ..Policy::default()
            });
            let (peer, mut inbox) = f.add("a", immediate());
            settle_until("연결", || peer.peer_state() == PeerState::Live).await;
            let link = f.net.accept().unwrap();
            link.push(notice("held"));
            settle().await;
            peer.close();
            settle_until("접힘", || peer.peer_state() == PeerState::Closed).await;
            f.clock.advance(waited);
            settle().await;

            let items = drain_until_quiet(&mut inbox).await;
            assert_eq!(labels(&items), expected, "{waited:?} 기다린 뒤");
            assert!(
                ended(&mut inbox).await,
                "{waited:?}: ★감독이 넣는 끝을 놓았다 — 상한을 넘겼으면 `Closed` 없는 `None` 이 그 버림이다★"
            );
        }
    }

    #[tokio::test]
    async fn dropping_an_old_inbox_ends_its_supervisors_teardown_without_waiting_for_the_limit() {
        let f = Fixture::new(Policy {
            inbound_queue: 1,
            ..Policy::default()
        });
        let (old, old_inbox) = f.add("a", immediate());
        settle_until("연결", || old.peer_state() == PeerState::Live).await;
        let link = f.net.accept().unwrap();
        link.push(notice("held"));
        settle().await;
        let (_new, _new_inbox) =
            f.registry
                .add(PeerId::new("a"), Address::new("mem://a2"), immediate());
        settle().await;
        assert_eq!(old.peer_state(), PeerState::Closed);
        assert!(
            old.state().has_changed().is_ok(),
            "옛 감독이 아직 안 비운 옛 수신단을 기다리며 정리 중이다"
        );

        drop(old_inbox);
        settle().await;
        assert!(
            old.state().has_changed().is_err(),
            "★옛 수신단을 놓으면 옛 감독의 정리가 상한을 기다리지 않고 끝난다★"
        );
    }

    // ── 3-0 U1a: 팬아웃 유실은 열린 상대에게 신고된다 · 끝난 상대는 명부에 남아도 아무것도 안 받는다 ──
    //
    // 앱이 닫기를 부른 뒤의 거절을 안 세는 것(사용자 결정 2026-09-26)은 이 시험이 못 잰다 — 감독이 이미 끝나
    //   수신단이 끝난 뒤라 셈이 있어도 안 보인다. 그 틈은 바로 아래 시험이 잰다.
    #[tokio::test]
    async fn a_broadcast_loss_is_reported_for_an_open_peer_and_never_reaches_an_ended_one() {
        let f = Fixture::new(Policy::default());
        let (_down, mut down_inbox) = f.add("down", RejectingHandshake::new("nope"));
        let (shut, mut shut_inbox) = f.add("shut", immediate());
        settle().await;
        shut.close();
        settle_until("shut 접힘", || shut.peer_state() == PeerState::Closed).await;
        assert_eq!(f.registry.len(), 2, "앱이 닫았어도 명부에는 남아 있다");
        drain(&mut down_inbox);
        drain(&mut shut_inbox);

        f.registry.broadcast(TestOut::Notice("all".into()));
        settle().await;
        assert_eq!(outbound_losses(&drain(&mut down_inbox)), vec![1]);
        assert!(drain(&mut shut_inbox).is_empty());
        assert!(ended(&mut shut_inbox).await);
    }

    // ★닫기를 부른 뒤 감독이 그것을 처리하기 전의 틈을 잰다★ — 포기 대기에 잠든 감독은 제어와 깨움을 함께 받으면
    //   어느 쪽을 먼저 볼지 정해져 있지 않아, 핸들이 그 틈에 세면 `Dropped` 가 `Closed` 앞에 실릴 수 있다. 그래서
    //   양보 없이 닫고 곧바로 민 뒤 핸들 쪽 셈부터 본다.
    #[tokio::test]
    async fn a_broadcast_right_after_close_is_not_counted_before_the_supervisor_sees_the_close() {
        let f = Fixture::new(Policy::default());
        let (closing, mut inbox) = f.add("closing", RejectingHandshake::new("nope"));
        settle().await;
        assert!(matches!(closing.peer_state(), PeerState::GaveUp(_)));
        drain(&mut inbox);

        closing.close();
        f.registry.broadcast(TestOut::Notice("all".into()));
        assert_eq!(
            closing.refused_count(),
            0,
            "★앱이 닫기를 부른 뒤의 거절은 세지 않는다★(사용자 결정 2026-09-26)"
        );
        settle().await;
        assert_eq!(kinds(&drain(&mut inbox)), vec!["Closed"]);
        assert!(ended(&mut inbox).await);
    }

    // ── 3-0 U1d: 감독이 정상 정리 없이 끝나면 `Closed` 없이 수신단이 끝난다 ──
    #[tokio::test]
    async fn a_supervisor_that_ends_abnormally_ends_its_inbox_without_closed() {
        let f = Fixture::new(Policy::default());
        let (peer, mut inbox) = f.add("a", Arc::new(PanickingHandshake));
        settle_until("감독 태스크가 끝남", || {
            peer.state().has_changed().is_err()
        })
        .await;
        assert!(
            drain(&mut inbox).is_empty(),
            "★정리를 거치지 않았으니 `Closed` 가 없다★"
        );
        assert!(
            ended(&mut inbox).await,
            "★넣는 끝이 태스크와 함께 떨어져 `None` 이다 — 상한 초과와 겉모양이 같다★"
        );
    }

    #[test]
    fn an_add_that_panics_outside_a_runtime_leaves_the_registry_usable() {
        let net = MemoryNetwork::new();
        let clock = ManualClock::new();
        let registry = Registry::new(
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
    }

    #[tokio::test]
    async fn a_dropped_registry_leaves_held_peers_running_until_their_handles_go() {
        let Fixture {
            registry,
            net: _net,
            clock: _clock,
        } = Fixture::new(Policy::default());
        let (peer, mut inbox) =
            registry.add(PeerId::new("a"), Address::new("mem://a"), immediate());
        settle().await;
        drop(registry);
        settle().await;
        assert_eq!(
            peer.peer_state(),
            PeerState::Live,
            "★명부가 사라져도 손잡이를 쥔 상대는 계속 돈다★"
        );

        drop(peer);
        settle().await;
        assert_eq!(
            labels(&drain(&mut inbox)),
            vec![
                "Connected:1",
                "Disconnected:1:Explicit",
                "Closed:1:Explicit"
            ]
        );
        assert!(
            ended(&mut inbox).await,
            "손잡이가 다 사라지면 수신단이 끝난다"
        );
    }
}
