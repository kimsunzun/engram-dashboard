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
//! ★상대를 올리는 길은 둘이다★ — [`Registry::add`] 는 주소로 걸고 끊기면 다시 건다. [`Registry::attach`] 는 받은
//! 통로([`Accepted`]) 하나를 얹고 다시 걸지 않으며, 끝나면 명부에서 스스로 내린다(TRD 3 §3-1 ①). 수락 루프는 소비자가
//! 돈다 — 명부는 청취기를 쥐지 않는다.
//!
//! ★「앱 전체가 끊겼나」는 여기서 판정하지 않는다★ — 그건 화면의 개념이고 이 crate 는 상대를 셀 뿐
//! 앱을 모른다. 재료 둘([`Registry::peers`] 와 [`crate::Peer::reconnect_now`])을 주고 셸이 접는다
//! (ADR-0180 의 열린 질문이 그렇게 닫힌다).

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::sync::mpsc;

use crate::budget::Charged;
use crate::clock::{with_deadline, Clock, Elapsed};
use crate::event::{Incoming, PeerId};
use crate::frame::CloseCode;
use crate::link::{Accepted, Address, Dialer, Handshake};
use crate::machine::PeerState;
use crate::peer::{prepare_peer, AcceptedLink, LeaveRoster, Origin, Peer, PeerConfig, BYTE_BUDGET};
use crate::policy::Policy;
use crate::wire::Wire;

/// 손잡이 하나의 수신단 — 그것을 돌려준 [`Registry::add`] · [`Registry::attach`] 가 띄운 감독 하나가 낸 것만 온다.
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
    /// ★감독은 곧바로 접는다★ — 넣을 것이 없어도(상대가 조용해도) 그 자리에서 닫기로 가고 다시 붙지 않는다
    /// (crate 헤더 「받는 쪽 불변식」 5). 떨어뜨려도 같다. ★받아 둔 나가는 일도 함께 버려진다★ — 큐에 든 명령은
    /// 거절되고 쓰던 프레임은 버려진다(세지만 신고할 데가 없다). 보내고 떠나려면 `Closed` · `None` 까지 비운 뒤에
    /// 놓는다.
    pub fn close(&mut self) {
        self.rx.close();
    }
}

struct RegistryInner<W: Wire> {
    wire: Arc<W>,
    dialer: Arc<dyn Dialer>,
    clock: Arc<dyn Clock>,
    policy: Policy,
    /// ★자물쇠 안에서는 기다리지 않는다★ — 감독 띄우기 · 닫기 · 손잡이 떨어뜨리기는 자물쇠 밖이다.
    roster: Mutex<Roster<W>>,
}

/// 명부 자물쇠 하나가 지키는 것 전부. ★세대 표도 이 자물쇠 안이다★ — 갈아치울 손잡이 빼기 · 계수기 고르기 · 새
/// 손잡이 올리기가 한 임계 구역이어야 해서다(`Registry::launch`).
struct Roster<W: Wire> {
    /// 올라 있는 이름표의 손잡이.
    peers: HashMap<PeerId, Peer<W>>,
    /// 이름표별 세대 계수기. ★`remove` 로 지우지 않는다★ — 지우면 같은 이름표를 다시 올릴 때 세대가
    /// 0 으로 되감기고, 소비자의 평범한 방어(「최대 세대 미만은 무시」)가 새 연결을 통째로 버린다.
    ///
    /// 이 map 은 **소비자가 [`Registry::add`] 로 쓴 이름표 수**만큼 자란다(상대가 정하는 값이 아니다).
    /// ★받은 상대는 넣지 않는다★ — 연결마다 이름표가 새것이라 넣으면 연결 수만큼 영원히 자란다
    /// ([`Registry::attach_with_policy`]).
    generations: HashMap<PeerId, Arc<AtomicU64>>,
    /// [`Registry::close_all`] 을 불렀다 — 그 뒤 올리는 상대는 명부에 오르지 않고 곧바로 닫힌다. 세우는 쪽(`close_all`
    /// 의 비우기)과 읽는 쪽(`launch`)이 같은 자물쇠 안이라 접기와 엇갈린 올리기가 비운 명부에 남지 않는다.
    folded: bool,
}

impl<W: Wire> RegistryInner<W> {
    /// 독 걸린 자물쇠를 그대로 다시 쓴다 — 독을 그대로 두면 한 번의 패닉이 명부를 영영 못 쓰게 만든다.
    ///
    /// 그래도 반쯤 바뀐 표는 남지 않는다: 자물쇠 안에서 패닉할 수 있는 것은 감독 짓기(`Registry::launch` 의
    /// `prepare_peer` — 주입한 시계 같은 것) 하나이고 표를 바꾸기 전에 끝나며, 표 바꾸기는 패닉할 자리 없이 한
    /// 칸씩이다. ★짓다 패닉해도 풀리며 떨어지는 것이 이 자물쇠를 다시 잡지 않는다★(`peer.rs` 의 `new_peer`) — 잡으면
    /// 같은 스레드에서 교착이다.
    fn roster(&self) -> MutexGuard<'_, Roster<W>> {
        self.roster.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl<W: Wire> Roster<W> {
    /// 이 이름표가 쓸 세대 계수기 — 표에 있으면 그것, 없으면 올라 있는(갈아치울) 손잡이의 것, 둘 다 없으면 새것이다.
    /// 표를 바꾸지 않는다 — 표에 올리기는 `Registry::launch` 가 짓기를 마친 뒤에 한다.
    fn counter(&self, id: &PeerId) -> Arc<AtomicU64> {
        if let Some(counter) = self.generations.get(id) {
            return counter.clone();
        }
        // ★옛 감독의 마지막 세대를 읽지 않고 계수기째 잇는다★(그 경합 = `PeerConfig::generations`). 표에 없는 받은
        //   상대를 `add` 로 갈아치울 때 새 계수기면 받은 상대가 이미 뽑은 번호를 되풀이한다.
        self.peers
            .get(id)
            .map(Peer::generations)
            .unwrap_or_default()
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
    /// 명부 하나를 세운다. ★받는 것은 [`Registry::add`] · [`Registry::attach`] 가 손잡이마다 돌려주는 [`Inbox`] 로만
    /// 나온다★. `dialer` 는 `add` 만 쓴다.
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
                roster: Mutex::new(Roster {
                    peers: HashMap::new(),
                    generations: HashMap::new(),
                    folded: false,
                }),
            }),
        }
    }

    /// 상대를 올리고 즉시 붙기 시작한다 — 그 손잡이와 수신단을 돌려준다. 같은 이름표가 이미 있으면 **옛 것을
    /// 닫고 갈아치운다**.
    ///
    /// ★tokio 런타임 안에서 불러야 한다★ — 감독 태스크를 띄우므로 밖에서 부르면 패닉한다. 그 패닉은 명부를 건드리기
    /// 전이다 — 명부는 그 뒤에도 쓸 수 있고, 같은 이름표로 올라 있던 상대도 그대로 올라 있다.
    ///
    /// [`Registry::close_all`] 을 부른 뒤면 명부에 올리지 않고 걸기 전에 닫는다 — 수신단은 `Closed` → `None` 으로
    /// 끝나고 [`Dialer`] 는 불리지 않는다.
    pub fn add(
        &self,
        id: PeerId,
        addr: Address,
        handshake: Arc<dyn Handshake>,
    ) -> (Peer<W>, Inbox<W>) {
        let policy = self.inner.policy.clone();
        self.add_with_policy(id, addr, handshake, policy)
    }

    /// 이 상대에게만 다른 정책을 건다. 런타임 조건 · 접은 명부는 [`Registry::add`] 와 같다.
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
    /// ★세대가 1 부터 다시 가는 것은 `add` 로 올린 적이 없고 지금 명부에도 없는 이름표뿐이다★ — 받아 얹었다가 스스로
    /// 내렸거나 [`Registry::remove`] 로 내린 이름표를 `add` 하는 것도 여기 든다. 그 밖에는 이 이름표의 계수기(세대
    /// 표에 있는 것 · 없으면 갈아치우는 손잡이의 것)를 잇고 표에 올린다. 같은 이름표를 동시에 올리는 호출끼리도
    /// 그렇다 — 어느 호출도 다른 호출이 뺀 손잡이를 못 본 채 새 계수기를 만들지 않는다.
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
        let (inbound, rx) = mpsc::channel(policy.inbound_queue.max(1));
        let origin = Origin::Dial {
            addr,
            dialer: self.inner.dialer.clone(),
        };
        let peer = self.launch(id.clone(), true, |generations| PeerConfig {
            id,
            origin,
            wire: self.inner.wire.clone(),
            clock: self.inner.clock.clone(),
            handshake,
            policy,
            inbound,
            byte_budget: BYTE_BUDGET,
            generations,
        });
        (peer, Inbox { rx })
    }

    /// 받은 통로를 상대로 얹는다 — 그 손잡이와 수신단을 돌려준다. 정책은 명부의 것이다
    /// ([`Registry::attach_with_policy`] 가 덮는다). 계약은 그쪽에 있다.
    pub fn attach(
        &self,
        id: PeerId,
        accepted: Accepted,
        handshake: Arc<dyn Handshake>,
    ) -> (Peer<W>, Inbox<W>) {
        let policy = self.inner.policy.clone();
        self.attach_with_policy(id, accepted, handshake, policy)
    }

    /// 받은 통로([`Accepted`])를 이 정책으로 상대로 얹는다(TRD 3 §3-1 ①). 런타임 조건 · 같은 이름표 갈아치우기(옛 것을
    /// 닫는다 · 옛 수신단은 `Closed` → `None` 으로 끝난다 · 수신단은 새것이다)는 [`Registry::add_with_policy`] 와 같다.
    ///
    /// ★핸드셰이크부터 시작하고 다시 걸지 않는다★ — 운영 단계에 못 들고 끝나거나(거절 · 시한 · 통로 손실) 운영 중
    /// 끊기면 곧바로 접는다: 수신단은 [`TransportEvent::Closed`](crate::TransportEvent::Closed) 다음 `None` 으로 끝나고
    /// 그 `cause` 가 진짜 까닭을 싣는다(운영 중 끊김이면 앞의 `Disconnected` 와 같은 까닭 — 읽기 끝 · 쓰기 끝 가름
    /// 포함). 거는 쪽 사건(`ConnectFailed` · `Rejected` · `GaveUp`)은 오지 않고 [`Peer::state`] 도 `Dialing` ·
    /// `Backoff` · `GaveUp` 을 거치지 않는다. [`Peer::reconnect_now`] 는 듣지 않는다. 그 밖의 손잡이 계약(보내기 ·
    /// 닫기 · 비우고 닫기)은 건 상대와 같다.
    ///
    /// ★인증 시한 = [`Accepted::upgraded_at`] + [`Policy::connect_timeout`]★(TRD 3 §5 3) — 청취기의 준비 큐에 앉은
    /// 시간도 든다. 얹는 순간 이미 지났으면 핸드셰이크를 돌리지 않고 닫는다(`cause` = `Idle`). 그래서 청취기는 이 명부와
    /// 같은 [`Clock`] 으로 `upgraded_at` 을 찍어야 한다(debug 빌드는 미래 시각을 단언으로 잡는다).
    ///
    /// ★끝나면 명부에서 스스로 내린다★ — 통로를 버린 뒤 정리(받는 쪽이 수신단을 비우기를 기다린다) 앞에서다. 그 사이
    /// 같은 이름표로 갈아치웠으면 새것은 그대로 둔다. 감독이 비정상으로 끝나도 내린다. 첫 폴링에 곧바로 끝나는
    /// 상대(시한이 이미 지났다 · 상대가 이미 끊었다)도 명부에 남지 않는다 — 올린 뒤에 감독을 띄운다.
    ///
    /// ★세대 표에 안 든다★ — 받는 쪽 이름표는 연결마다 새것이라(TRD 3 §3-1 ① `conn-<ConnId>`) 넣으면 연결 수만큼 표가
    /// 자란다. 세대 계수기는 표에 그 이름표가 있으면(`add` 로 올린 적이 있다) 그것을, 없으면 갈아치우는 손잡이의 것을,
    /// 둘 다 없으면 새 것을 쓴다 — 명부에 있는 이름표를 다시 얹으면 동시에 얹어도 세대가 되감기지 않는다. ★세대가 1
    /// 부터 다시 가는 것은 `add` 로 올린 적이 없고 지금 명부에도 없는 이름표뿐이다★ — 처음 얹는 이름표, 그리고 스스로
    /// 내렸거나 [`Registry::remove`] 로 내린 뒤 다시 얹는 이름표다(받는 쪽 이름표를 되쓰지 않는다는 전제).
    ///
    /// ★업그레이드 칸([`Accepted`] 의 [`crate::ConnPermit`])은 운영 단계에 들 때 놓는다★ — 못 들고 끝나면 통로를
    /// 버린 그 자리에서 놓는다(받는 쪽이 수신단을 비우기 전에 — TRD 3 §5 14 「인증에 실패한 상대는 닫히는 즉시 칸을
    /// 놓는다」). 인증 대기 칸이 아직 없어(3-0 U7 · 3-3 U4) 그 몫까지 이 칸이 진다 — 그 칸이 들어오면 여기서 그것을
    /// 쥐고 업그레이드 칸을 놓는다.
    ///
    /// [`Registry::close_all`] 을 부른 뒤면 명부에 올리지 않고 핸드셰이크 전에 닫는다 — 통로에서 아무것도 읽지도
    /// 쓰지도 않은 채 닫기([`CloseCode::GOING_AWAY`])를 싣고 버린다.
    pub fn attach_with_policy(
        &self,
        id: PeerId,
        accepted: Accepted,
        handshake: Arc<dyn Handshake>,
        policy: Policy,
    ) -> (Peer<W>, Inbox<W>) {
        debug_assert_eq!(policy.check(), Ok(()), "정책 값이 자기 불변식을 어긴다");
        let Accepted {
            tx,
            rx,
            info: _,
            upgraded_at,
            permit,
        } = accepted;
        debug_assert!(
            upgraded_at <= self.inner.clock.now(),
            "listener/registry clock mismatch — 청취기가 명부와 다른 시계로 upgraded_at 을 찍었다"
        );
        let (inbound, inbox) = mpsc::channel(policy.inbound_queue.max(1));
        let origin = Origin::Accepted(AcceptedLink {
            tx,
            rx,
            upgraded_at,
            permit,
            leave: self.leave_roster(id.clone()),
        });
        let peer = self.launch(id.clone(), false, |generations| PeerConfig {
            id,
            origin,
            wire: self.inner.wire.clone(),
            clock: self.inner.clock.clone(),
            handshake,
            policy,
            inbound,
            byte_budget: BYTE_BUDGET,
            generations,
        });
        (peer, Inbox { rx: inbox })
    }

    /// `add` · `attach` 의 공통 꼬리 — 감독과 손잡이를 짓고, 명부에 올린 **뒤에** 감독을 띄운다. `record` = 표에 없던
    /// 이름표면 고른 세대 계수기를 표에 올린다(`add`).
    ///
    /// ★올리기가 띄우기보다 앞이다★ — 뒤집으면 첫 폴링에 곧바로 끝나는 받은 상대(시한이 이미 지났다 · 상대가 이미
    /// 끊었다)가 다른 일꾼 스레드에서 명부 내리기까지 마친 뒤에 올라, 끝난 손잡이가 영영 남는다(받는 쪽 이름표는
    /// 연결마다 새것이라 아무도 다시 내리지 않는다 — 빨리 끊기는 연결 수만큼 명부가 자란다). 접은 명부도 띄우기 전에
    /// 가르고 닫기를 먼저 넣는다 — 감독은 첫 폴링에서 그것을 보고 걸지도 핸드셰이크하지도 않는다(`peer.rs` 의 `dial` ·
    /// `shake_hands` 첫 줄). 뒤집으면 다른 일꾼에서 먼저 돈 감독이 한 번 걸거나 핸드셰이크를 시작한다.
    ///
    /// ★계수기 고르기 · 갈아치울 손잡이 빼기 · 올리기가 한 임계 구역이다★ — 쪼개면 같은 이름표를 동시에 올리는 두
    /// 호출 중 하나가 다른 쪽이 뺀 손잡이도 표의 칸도 못 본 채 새 계수기를 만들어, 이미 나간 세대를 다시 뽑는다.
    ///
    /// ★표는 짓기(`prepare_peer`)를 마친 뒤에야 바꾼다★ — 짓다가 패닉하면(주입한 시계 같은 것) 명부는 손대지 않은
    /// 그대로라, 갈아치우려던 손잡이가 올라 있고 계속 돈다. 그 뒤의 표 바꾸기에는 패닉할 자리가 없다(`roster`).
    fn launch(
        &self,
        id: PeerId,
        record: bool,
        config: impl FnOnce(Arc<AtomicU64>) -> PeerConfig<W>,
    ) -> Peer<W> {
        // 런타임 밖이면 여기서 패닉한다 — 명부를 건드리기 전이다(`Registry::add`).
        let runtime = tokio::runtime::Handle::current();
        let (peer, supervisor, displaced, folded) = {
            let mut roster = self.inner.roster();
            let generations = roster.counter(&id);
            let (peer, supervisor) = prepare_peer(config(generations.clone()));
            // `add` 만 고른 계수기를 표에 올린다 — 받은 상대는 표를 읽기만 한다(`Roster::generations`).
            if record {
                roster.generations.entry(id.clone()).or_insert(generations);
            }
            let displaced = roster.peers.remove(&id);
            let folded = roster.folded;
            if !folded {
                roster.peers.insert(id, peer.clone());
            }
            (peer, supervisor, displaced, folded)
        };
        if let Some(old) = displaced {
            old.close();
        }
        if folded {
            peer.close();
        }
        supervisor.start(&runtime);
        peer
    }

    /// 받은 상대가 끝날 때 명부에서 스스로 내리는 손 — 그 이름표가 아직 그 손잡이를 가리킬 때만 내린다. ★약한
    /// 손잡이다★ — 명부를 붙들지 않는다(명부가 먼저 사라지면 내릴 것도 없다).
    fn leave_roster(&self, id: PeerId) -> LeaveRoster<W> {
        let registry = Arc::downgrade(&self.inner);
        Box::new(move |me: &Peer<W>| {
            let Some(registry) = registry.upgrade() else {
                return;
            };
            // 내린 손잡이는 자물쇠 밖에서 떨어뜨린다.
            let _left = {
                let mut roster = registry.roster();
                match roster.peers.get(&id) {
                    Some(listed) if listed.same(me) => roster.peers.remove(&id),
                    _ => None,
                }
            };
        })
    }

    /// 명부에서 내리고 그 상대를 닫는다. 그 손잡이의 수신단은 감독이 정리를 마치면 `Closed` → `None` 으로
    /// 끝난다.
    pub fn remove(&self, id: &PeerId) {
        let removed = self.inner.roster().peers.remove(id);
        if let Some(peer) = removed {
            peer.close();
        }
    }

    /// 명부의 상대를 전부 내리고 각자 받아 둔 나가는 일을 비운 뒤 닫는다([`Peer::close_after_flush`] ·
    /// [`CloseCode::GOING_AWAY`]) — 그 감독들이 다 끝나기를 `deadline`(명부의 시계)까지 기다린다(TRD 3 §3-0 ④ — 데몬
    /// 종료가 이것을 기다린다). `Err` = 시한까지 다 안 끝났다 — 남은 감독은 계속 접고 각자의 시한
    /// ([`Policy::flush_timeout`] · 정리 상한)에 끝난다.
    ///
    /// ★곧바로 닫지 않고 비운다★ — 닫기 직전에 넣은 마지막 답(정지 Ack 같은 것)이 나가게 하려는 것이 이 함수의
    /// 까닭이다. 곧바로 닫으려면 [`Registry::remove`].
    ///
    /// ★부른 순간 명부에 있던 상대만 기다린다★ — 그 전에 `add` 가 갈아치웠거나 `remove` 로 내려 아직 접는 중인
    /// 감독은 모른다.
    ///
    /// ★명부를 접는다 — 다시 열리지 않는다★ — 부른 뒤의 [`Registry::add`] · [`Registry::attach`] 는 명부에 오르지
    /// 않고 걸거나 핸드셰이크하기 전에 닫힌다(그 수신단은 `Closed` → `None` 으로 끝나고, 받은 통로는 아무것도 읽고
    /// 쓰지 않은 채 닫기를 싣고 버린다). 이 함수는 그것을
    /// 기다리지 않는다. 수락 루프가 접기와 엇갈려 얹은 상대가 비운 명부에 닫히지 않은 채 남지 않게 하려는 것이다 —
    /// 그래도 수락 루프는 먼저 세운다(접은 뒤 받은 통로는 얹어 봐야 곧바로 닫힌다).
    ///
    /// ★닫기는 부른 그 자리에서 시작된다★ — 돌려주는 future 는 기다리기만 한다. 떨어뜨려도 닫기는 이어진다
    /// ([`Peer::close_after_flush`] 와 같다). 시한은 그 future 를 처음 폴링한 때부터 잰다.
    ///
    /// ★그 상대들의 수신단을 비우는 task 위에서 기다리지 않는다★ — 감독의 끝은 그 받는 쪽이 비우는 데 달려 있어
    /// (crate 헤더 「알려진 한계」), 거기서 기다리면 그 감독이 정리 상한까지 남아 이 대기가 `deadline` 까지 간다.
    /// 수신단은 끝(`None`)까지 계속 비운다 — 놓으면 그 상대의 남은 나가는 일이 버려진다(crate 헤더 「받는 쪽
    /// 불변식」 5).
    pub fn close_all(
        &self,
        deadline: Duration,
    ) -> impl Future<Output = Result<(), Elapsed>> + Send + 'static {
        let peers: Vec<Peer<W>> = {
            let mut roster = self.inner.roster();
            roster.folded = true;
            roster.peers.drain().map(|(_, peer)| peer).collect()
        };
        // 손잡이는 여기서 놓아도 된다 — 표지가 제어 채널을 붙들어 비우기가 이어진다(`Peer::close_after_flush`).
        let ends: Vec<_> = peers
            .iter()
            .map(|peer| peer.close_after_flush(CloseCode::GOING_AWAY, "going away"))
            .collect();
        let clock = self.inner.clock.clone();
        async move {
            with_deadline(&*clock, deadline, futures_util::future::join_all(ends))
                .await
                .map(|_| ())
        }
    }

    pub fn get(&self, id: &PeerId) -> Option<Peer<W>> {
        self.inner.roster().peers.get(id).cloned()
    }

    /// 이름표 순으로 정렬한 지금 한 장. ★셸이 「앱 전체」를 접는 재료가 이것이다★.
    pub fn peers(&self) -> Vec<(PeerId, PeerState)> {
        let mut out: Vec<(PeerId, PeerState)> = self
            .inner
            .roster()
            .peers
            .iter()
            .map(|(id, peer)| (id.clone(), peer.peer_state()))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    pub fn len(&self) -> usize {
        self.inner.roster().peers.len()
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
        let targets: Vec<Peer<W>> = self.inner.roster().peers.values().cloned().collect();
        for peer in targets {
            // 실패는 그 상대의 감독이 센다(`Peer::send_frame`).
            let _ = peer.send_frame(frame.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Arrival, Direction, TransportEvent};
    use crate::frame::{Close, Frame};
    use crate::link::{
        HandshakeStep, LinkInfo, LinkRead, LinkRx, LinkTx, Listener, PermitPool, RejectKind,
    };
    use crate::machine::{GaveUpReason, Generation};
    use crate::peer::CLOSE_DRAIN_LIMIT;
    use crate::stream::StreamMark;
    use crate::testing::{
        memory_link, settle, settle_until, ClientMsg, HelloHandshake, ImmediateHandshake,
        ManualClock, MemoryConnector, MemoryLinkControl, MemoryListener, MemoryNetwork,
        RejectingHandshake, TestDecodeError, TestIn, TestOut, TestWire,
    };
    use futures_util::future::poll_immediate;
    use std::time::{Duration, Instant};

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

        /// ★돌려받은 수신단을 쥐고 있어야 한다★ — 떨어뜨리면 감독이 곧바로 접는다.
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

    // ── 3-0 U1d: 조용한 상대여도 수신단을 닫거나 놓으면 다음 프레임 없이 접는다 ──
    #[tokio::test]
    async fn closing_an_inbox_on_a_silent_peer_folds_it_and_still_drains_what_was_queued() {
        let f = Fixture::new(Policy::default());
        let (peer, mut inbox) = f.add("a", immediate());
        settle_until("연결", || peer.peer_state() == PeerState::Live).await;
        let link = f.net.accept().unwrap();
        link.push(notice("before"));
        settle().await;

        inbox.close();
        settle().await;
        assert!(
            peer.state().has_changed().is_err(),
            "★넣을 것이 없어도 감독이 접고 정리를 마쳤다★ — 다음 프레임도 상한도 기다리지 않았다"
        );
        assert_eq!(peer.peer_state(), PeerState::Closed);
        assert_eq!(f.net.dials(), 1, "다시 걸지 않았다");
        assert_eq!(
            labels(&drain(&mut inbox)),
            vec!["Connected:1", "msg:before"],
            "닫기 전에 든 것은 다 꺼낼 수 있다"
        );
        assert!(
            ended(&mut inbox).await,
            "그 뒤는 끝이다 — `Closed` 는 실을 데가 없었다"
        );
    }

    // 30 번인 까닭 = 걸기 구간을 폴링하기 전의 검사가 없으면 걸기 future 와 닫힘 팔이 함께 준비돼 어느 쪽이 먼저
    //   폴링될지가 난수다(`select!`) — 한 번으로는 그 검사가 빠진 것을 놓칠 수 있다.
    #[tokio::test]
    async fn an_inbox_dropped_before_its_supervisor_runs_never_dials() {
        let f = Fixture::new(Policy::default());
        for n in 0..30 {
            let (peer, inbox) = f.add(&format!("a{n}"), immediate());
            drop(inbox);
            settle().await;
            assert_eq!(peer.peer_state(), PeerState::Closed, "{n}");
            assert!(peer.state().has_changed().is_err(), "{n}: 감독이 끝났다");
        }
        assert_eq!(
            f.net.dials(),
            0,
            "★첫 사건을 넣기도 전에 놓인 수신단이면 통로를 아예 안 건다★"
        );
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

    // ── 3-0 U3: close_all ──

    #[tokio::test]
    async fn close_all_flushes_every_peer_then_closes_it_and_empties_the_roster() {
        let fx = Fixture::new(Policy::default());
        let (a, mut inbox_a) = fx.add("a", immediate());
        let (b, mut inbox_b) = fx.add("b", immediate());
        settle_until("둘 다 운영", || {
            a.peer_state().is_live() && b.peer_state().is_live()
        })
        .await;
        let endpoints = [fx.net.accept().unwrap(), fx.net.accept().unwrap()];
        // ★쓰기를 막아 둔다★ — 「last」가 쓰이는 도중 닫기가 들어와야 곧바로 닫는 회귀와 갈린다.
        for endpoint in &endpoints {
            endpoint.stall_writes(true);
        }
        a.notify(TestOut::Notice("last".into())).unwrap();
        b.notify(TestOut::Notice("last".into())).unwrap();

        let closing = tokio::spawn(fx.registry.close_all(Duration::from_secs(3)));
        settle().await;
        assert!(!closing.is_finished(), "막힌 쓰기를 비우는 중이다");
        for endpoint in &endpoints {
            endpoint.stall_writes(false);
        }
        settle_until("시계를 밀지 않고 끝났다", || {
            closing.is_finished()
        })
        .await;
        assert_eq!(closing.await.expect("close_all 태스크"), Ok(()));
        assert!(fx.registry.is_empty(), "명부에서 내렸다");
        for endpoint in &endpoints {
            assert_eq!(
                endpoint.drain(),
                vec![
                    ClientMsg::Frame(notice("last")),
                    ClientMsg::Close(Close::new(CloseCode::GOING_AWAY, "going away")),
                ],
                "★곧바로 닫지 않고 앞서 받아 둔 것을 먼저 쓴다★"
            );
        }
        for inbox in [&mut inbox_a, &mut inbox_b] {
            assert_eq!(
                labels(&drain(inbox)),
                vec![
                    "Connected:1",
                    "Disconnected:1:Explicit",
                    "Closed:1:Explicit"
                ]
            );
            assert!(ended(inbox).await);
        }
    }

    // ★future 를 기다리지 않아도 닫는다★ — 닫기는 부른 자리에서 시작되고 future 는 기다리기만 한다.
    #[tokio::test]
    async fn close_all_closes_even_when_nobody_waits_for_it() {
        let fx = Fixture::new(Policy::default());
        let (peer, mut inbox) = fx.add("a", immediate());
        settle_until("운영", || peer.peer_state().is_live()).await;
        drop(fx.registry.close_all(Duration::ZERO));
        let items = drain_until_quiet(&mut inbox).await;
        assert_eq!(
            labels(&items),
            vec![
                "Connected:1",
                "Disconnected:1:Explicit",
                "Closed:1:Explicit"
            ]
        );
        assert!(ended(&mut inbox).await);
    }

    // ★close_all 의 시한(1 s · 3 s)을 비우기의 시한(2 s) 양쪽에 둔다★ — 그 안에 못 끝나면 `Err` 이고, 감독은 기다림과
    //   무관하게 제 시한에 끝난다.
    #[tokio::test]
    async fn close_all_waits_only_up_to_its_deadline() {
        assert_eq!(Policy::default().flush_timeout, Duration::from_secs(2));
        for (deadline, in_time) in [
            (Duration::from_secs(1), false),
            (Duration::from_secs(3), true),
        ] {
            let fx = Fixture::new(Policy::default());
            let (peer, mut inbox) = fx.add("a", immediate());
            settle_until("운영", || peer.peer_state().is_live()).await;
            let endpoint = fx.net.accept().unwrap();
            endpoint.stall_writes(true);
            peer.notify(TestOut::Notice("stuck".into())).unwrap();
            settle().await;

            let closing = tokio::spawn(fx.registry.close_all(deadline));
            settle().await;
            fx.clock.advance(Duration::from_secs(1));
            settle().await;
            assert_eq!(closing.is_finished(), !in_time, "{deadline:?}: 1 s 뒤");
            fx.clock.advance(Duration::from_secs(1));
            settle_until("비우기의 시한에 감독이 끝났다", || {
                closing.is_finished()
            })
            .await;
            let expected = if in_time { Ok(()) } else { Err(Elapsed) };
            assert_eq!(
                closing.await.expect("close_all 태스크"),
                expected,
                "{deadline:?}"
            );
            let items = drain_until_quiet(&mut inbox).await;
            assert_eq!(kinds(&items).last(), Some(&"Closed"), "{deadline:?}");
            assert!(ended(&mut inbox).await, "{deadline:?}");
        }
    }

    // ── 3-0 U6b: 받은 상대(`attach`) ──

    /// TRD 3 §5 3 의 인증 시한.
    const AUTH: Duration = Duration::from_secs(1);
    const GOOD: &str = "auth|good";

    fn auth_policy() -> Policy {
        Policy {
            connect_timeout: AUTH,
            ..Policy::default()
        }
    }

    /// 받는 쪽 인증 — 첫 프레임이 [`GOOD`] 이어야 운영 단계다. 아니면 다시 거는 거절이다(낡은 자격 — TRD 3 §3-0 ⑤).
    struct TokenGate;

    impl Handshake for TokenGate {
        fn start(&self) -> HandshakeStep {
            HandshakeStep::Await
        }

        fn on_frame(&self, frame: &Frame) -> HandshakeStep {
            match frame {
                Frame::Text(text) if text == GOOD => HandshakeStep::Done,
                _ => HandshakeStep::Reject {
                    kind: RejectKind::Retryable,
                    last: None,
                    reason: "bad token".into(),
                },
            }
        }
    }

    /// 받는 쪽 명부와 그 청취기. ★명부의 Dialer 는 세기만 하는 그물이다★ — 받은 상대가 다시 거는지 본다.
    struct Accepting {
        registry: Registry<TestWire>,
        redials: MemoryNetwork,
        clock: ManualClock,
        listener: MemoryListener,
        connector: MemoryConnector,
    }

    /// 거는 쪽 끝 — 감독 없이 손으로 몬다.
    struct Near {
        tx: Box<dyn LinkTx>,
        rx: Box<dyn LinkRx>,
    }

    #[derive(Debug, PartialEq)]
    enum Seen {
        Frame(Frame),
        Closed(Option<Close>),
        Failed,
    }

    impl Accepting {
        fn new(policy: Policy) -> Self {
            let clock = ManualClock::new();
            let redials = MemoryNetwork::new();
            let registry =
                Registry::new(Arc::new(TestWire), redials.dialer(), clock.handle(), policy);
            let (listener, connector) = MemoryListener::new(clock.handle(), 8);
            Self {
                registry,
                redials,
                clock,
                listener,
                connector,
            }
        }

        /// 거는 쪽 끝을 하나 열고 청취기에서 그 통로를 받아 온다 — `upgraded_at` = 지금.
        async fn connect(&mut self) -> (Near, Accepted) {
            let (tx, rx) = self
                .connector
                .dial(&Address::new("mem://acceptor"))
                .await
                .expect("걸기");
            let accepted = self.listener.accept().await.expect("받기");
            (Near { tx, rx }, accepted)
        }

        /// 마지막 통로의 받는 쪽 끝.
        fn far(&self) -> MemoryLinkControl {
            self.connector.latest().expect("통로").acceptor
        }

        fn attach(
            &self,
            name: &str,
            accepted: Accepted,
            handshake: Arc<dyn Handshake>,
        ) -> (Peer<TestWire>, Inbox<TestWire>) {
            self.registry.attach(PeerId::new(name), accepted, handshake)
        }

        fn generation_names(&self) -> usize {
            self.registry.inner.roster().generations.len()
        }
    }

    impl Near {
        async fn say(&mut self, text: &str) {
            self.tx
                .send(Frame::Text(text.into()))
                .await
                .expect("거는 쪽 쓰기");
        }

        /// 지금 와 있는 것을 다 읽는다 — 기다리지 않고, 끝(닫힘 · 오류) 뒤로는 안 읽는다.
        async fn arrived(&mut self) -> Vec<Seen> {
            let mut out = Vec::new();
            while let Some(read) = poll_immediate(self.rx.recv()).await {
                let seen = match read {
                    Ok(LinkRead::Frame(frame)) => Seen::Frame(frame),
                    Ok(LinkRead::Closed(close)) => Seen::Closed(close),
                    Err(_) => Seen::Failed,
                };
                let end = !matches!(seen, Seen::Frame(_));
                out.push(seen);
                if end {
                    break;
                }
            }
            out
        }
    }

    fn closing() -> Seen {
        Seen::Closed(Some(Close::new(CloseCode::GOING_AWAY, "closing")))
    }

    // ★시계를 밀지 않고 끝을 본다★ — 다시 걸기로 가면 백오프 대기에 서서 `Closed` 가 안 온다.
    #[tokio::test]
    async fn an_accepted_peer_that_loses_its_link_ends_with_that_cause_and_never_redials() {
        let mut a = Accepting::new(auth_policy());
        let (near, accepted) = a.connect().await;
        let (peer, mut inbox) = a.attach("conn-1", accepted, immediate());
        settle_until("운영", || peer.peer_state().is_live()).await;
        drop(near);
        let items = drain_until_quiet(&mut inbox).await;
        assert_eq!(
            labels(&items),
            vec![
                "Connected:1",
                "Disconnected:1:ReadFailed",
                "Closed:1:ReadFailed"
            ],
            "거는 쪽 사건(ConnectFailed · GaveUp) 없이 끝난다"
        );
        assert!(ended(&mut inbox).await);
        assert_eq!(peer.peer_state(), PeerState::Closed);
        assert!(a.registry.is_empty(), "끝나면 명부에서 스스로 내린다");
        a.clock.advance(Duration::from_secs(60));
        settle().await;
        assert_eq!(a.redials.dials(), 0, "받은 상대는 다시 걸지 않는다");
    }

    // 데몬 어댑터가 읽기 끝(남은 명령을 마저 돌린다)과 쓰기 끝(곧바로 끊는다)을 이 까닭으로 가른다(TRD 3 §3-0 ①).
    #[tokio::test]
    async fn read_end_and_write_end_causes_reach_closed_on_the_accepted_side() {
        let write_deadline = Policy::default().write_deadline;
        for case in [
            "상대가 닫음",
            "읽기 오류",
            "말 없이 사라짐",
            "쓰기 오류",
            "쓰기 시한",
            "침묵",
        ] {
            let mut a = Accepting::new(auth_policy());
            let (mut near, accepted) = a.connect().await;
            let far = a.far();
            let (peer, mut inbox) = a.attach("conn-1", accepted, immediate());
            settle_until("운영", || peer.peer_state().is_live()).await;
            let expected = match case {
                "상대가 닫음" => {
                    near.tx.close(Close::going_away()).await;
                    "PeerClosed"
                }
                "읽기 오류" => {
                    far.fail_reads("boom");
                    "ReadFailed"
                }
                "말 없이 사라짐" => {
                    drop(near);
                    "ReadFailed"
                }
                "쓰기 오류" => {
                    far.fail_writes();
                    peer.notify(TestOut::Notice("x".into())).unwrap();
                    "WriteFailed"
                }
                "쓰기 시한" => {
                    far.stall_writes(true);
                    peer.notify(TestOut::Notice("x".into())).unwrap();
                    settle().await;
                    a.clock.advance(write_deadline);
                    settle().await;
                    // 통로를 버리는 닫기도 막혀 있다 — 그 대기도 쓰기 시한이 자른다.
                    a.clock.advance(write_deadline);
                    "WriteDeadline"
                }
                // 거는 쪽이 안 읽으니 우리 ping 의 대답도 없다.
                _ => {
                    for _ in 0..5 {
                        settle().await;
                        a.clock.advance(Duration::from_secs(10));
                    }
                    "Idle"
                }
            };
            // 쓰기 쪽 끝은 그 쓰다 만 알림을 `Dropped` 로 함께 센다 — 여기서 재는 것은 끝 까닭이다.
            let ends: Vec<String> = labels(&drain_until_quiet(&mut inbox).await)
                .into_iter()
                .filter(|label| label != "Dropped")
                .collect();
            assert_eq!(
                ends,
                vec![
                    "Connected:1".to_string(),
                    format!("Disconnected:1:{expected}"),
                    format!("Closed:1:{expected}"),
                ],
                "{case}"
            );
            assert!(ended(&mut inbox).await, "{case}");
            assert!(a.registry.is_empty(), "{case}");
        }
    }

    // 운영 단계에 못 든 끝도 읽기 끝 · 쓰기 끝을 가른다(`handshake_once` 의 실패 자리마다).
    #[tokio::test]
    async fn a_handshake_that_ends_on_the_link_reports_which_end_failed() {
        for case in ["상대가 닫음", "말 없이 사라짐", "쓰기 오류", "쓰기 시한"] {
            let mut a = Accepting::new(auth_policy());
            let (mut near, accepted) = a.connect().await;
            let far = a.far();
            // 쓰기 쪽 갈래는 먼저 말하는 핸드셰이크로 잰다.
            let handshake: Arc<dyn Handshake> = match case {
                "쓰기 오류" | "쓰기 시한" => Arc::new(HelloHandshake),
                _ => Arc::new(TokenGate),
            };
            let expected = match case {
                "상대가 닫음" => "PeerClosed",
                "말 없이 사라짐" => "ReadFailed",
                "쓰기 오류" => {
                    far.fail_writes();
                    "WriteFailed"
                }
                _ => {
                    far.stall_writes(true);
                    "WriteDeadline"
                }
            };
            let (_peer, mut inbox) = a.attach("conn-1", accepted, handshake);
            settle().await;
            match case {
                "상대가 닫음" => near.tx.close(Close::going_away()).await,
                "말 없이 사라짐" => drop(near),
                "쓰기 시한" => a.clock.advance(AUTH),
                _ => {}
            }
            assert_eq!(
                labels(&drain_until_quiet(&mut inbox).await),
                vec![format!("Closed:0:{expected}")],
                "{case}"
            );
            assert!(ended(&mut inbox).await, "{case}");
            assert!(a.registry.is_empty(), "{case}");
        }
    }

    #[tokio::test]
    async fn auth_failures_and_silent_links_do_not_stop_a_later_real_connection() {
        let mut a = Accepting::new(auth_policy());
        let permits = a.listener.permits();
        for n in 1..=5 {
            let (mut near, accepted) = a.connect().await;
            let (_peer, mut inbox) = a.attach(&format!("conn-{n}"), accepted, Arc::new(TokenGate));
            near.say("auth|stale").await;
            let items = drain_until_quiet(&mut inbox).await;
            assert_eq!(
                labels(&items),
                vec!["Closed:0:Explicit"],
                "{n}: 거는 쪽 사건(Rejected · ConnectFailed · GaveUp) 없이 끝만 — 거절은 우리가 했다"
            );
            assert!(ended(&mut inbox).await, "{n}");
            assert_eq!(
                near.arrived().await,
                vec![Seen::Closed(Some(Close::new(
                    RejectKind::Retryable.close_code(),
                    "bad token"
                )))],
                "{n}: 거절 코드는 상대에게 간다"
            );
        }
        // 종결 거절도 끝만 온다 — `Rejected` 는 바로 뒤의 `GaveUp` 을 약속하는 거는 쪽 사건이다.
        let (mut near, accepted) = a.connect().await;
        let (_peer, mut inbox) = a.attach(
            "conn-terminal",
            accepted,
            RejectingHandshake::new("version mismatch"),
        );
        assert_eq!(
            labels(&drain_until_quiet(&mut inbox).await),
            vec!["Closed:0:Explicit"]
        );
        assert_eq!(
            near.arrived().await,
            vec![Seen::Closed(Some(Close::new(
                RejectKind::Terminal.close_code(),
                "version mismatch"
            )))]
        );
        // 아무 말도 안 하는 통로 — 인증 시한에 닫기 없이 버린다.
        let (mut silent, accepted) = a.connect().await;
        let (_peer, mut inbox) = a.attach("conn-6", accepted, Arc::new(TokenGate));
        settle().await;
        a.clock.advance(AUTH);
        assert_eq!(
            labels(&drain_until_quiet(&mut inbox).await),
            vec!["Closed:0:Idle"]
        );
        assert_eq!(silent.arrived().await, vec![Seen::Failed]);

        let (mut near, accepted) = a.connect().await;
        let (peer, mut inbox) = a.attach("conn-7", accepted, Arc::new(TokenGate));
        near.say(GOOD).await;
        settle_until("운영", || peer.peer_state().is_live()).await;
        assert_eq!(
            labels(&drain_until_quiet(&mut inbox).await),
            vec!["Connected:1"]
        );
        assert_eq!(a.registry.len(), 1, "실패한 상대는 명부에 남지 않는다");
        assert_eq!(a.generation_names(), 0);
        assert_eq!(permits.in_use(), 0);
        assert_eq!(a.redials.dials(), 0);
    }

    // ★인증 프레임은 이미 와 있다★ — 지난 시한에도 한 번 읽어 보는 길(`with_deadline` 의 0)이면 통과한다.
    #[tokio::test]
    async fn a_link_that_sat_in_the_ready_queue_past_the_auth_deadline_closes_as_soon_as_it_is_attached(
    ) {
        for (sat, live) in [
            (AUTH - Duration::from_millis(1), true),
            (AUTH, false),
            (AUTH * 3, false),
        ] {
            let mut a = Accepting::new(auth_policy());
            let (mut near, accepted) = a.connect().await;
            near.say(GOOD).await;
            a.clock.advance(sat);
            let (_peer, mut inbox) = a.attach("conn-1", accepted, Arc::new(TokenGate));
            let items = drain_until_quiet(&mut inbox).await;
            if live {
                assert_eq!(labels(&items), vec!["Connected:1"], "{sat:?}");
            } else {
                assert_eq!(labels(&items), vec!["Closed:0:Idle"], "{sat:?}");
                assert!(ended(&mut inbox).await, "{sat:?}");
                assert!(a.registry.is_empty(), "{sat:?}");
                assert_eq!(near.arrived().await, vec![Seen::Failed], "{sat:?}");
            }
        }
    }

    #[tokio::test]
    async fn the_auth_deadline_counts_from_the_upgrade_not_from_attach() {
        let mut a = Accepting::new(auth_policy());
        let (_near, accepted) = a.connect().await;
        a.clock.advance(Duration::from_millis(600));
        let (_peer, mut inbox) = a.attach("conn-1", accepted, Arc::new(TokenGate));
        settle().await;
        a.clock.advance(Duration::from_millis(399));
        assert!(
            drain_until_quiet(&mut inbox).await.is_empty(),
            "업그레이드 뒤 999 ms — 아직 시한 전"
        );
        a.clock.advance(Duration::from_millis(1));
        assert_eq!(
            labels(&drain_until_quiet(&mut inbox).await),
            vec!["Closed:0:Idle"],
            "붙인 뒤로는 400 ms 뿐이다"
        );
    }

    #[tokio::test]
    async fn roster_generation_table_and_permits_stay_flat_across_many_accept_close_cycles() {
        let mut a = Accepting::new(auth_policy());
        let permits = a.listener.permits();
        for n in 0..30 {
            let (mut near, accepted) = a.connect().await;
            let (peer, mut inbox) = a.attach(&format!("conn-{n}"), accepted, Arc::new(TokenGate));
            match n % 3 {
                0 => {
                    near.say(GOOD).await;
                    settle_until("운영", || peer.peer_state().is_live()).await;
                    drop(near);
                }
                1 => near.say("auth|stale").await,
                _ => {
                    near.say(GOOD).await;
                    settle_until("운영", || peer.peer_state().is_live()).await;
                    peer.close();
                }
            }
            let items = drain_until_quiet(&mut inbox).await;
            assert_eq!(kinds(&items).last(), Some(&"Closed"), "{n}");
            assert!(ended(&mut inbox).await, "{n}");
            assert!(a.registry.is_empty(), "{n}");
            assert_eq!(
                a.generation_names(),
                0,
                "{n}: 받은 상대는 세대 표에 안 든다"
            );
            assert_eq!(permits.in_use(), 0, "{n}");
        }
        assert_eq!(a.listener.queued(), 0);
        assert_eq!(a.redials.dials(), 0);
    }

    /// 양쪽이 같은 어휘를 쓰는 최소 패킷 — 묻기와 답하기뿐이다. `TestWire` 는 한쪽(클라) 어휘라 두 명부를 마주 잇지
    /// 못한다.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Talk {
        Ask { tag: u64, body: String },
        Answer { tag: u64, body: String },
    }

    struct TalkWire;

    impl Wire for TalkWire {
        type Out = Talk;
        type In = Talk;
        type Tag = u64;
        type StreamKey = u32;
        type DecodeError = TestDecodeError;

        fn encode(&self, out: &Talk) -> Frame {
            Frame::Text(match out {
                Talk::Ask { tag, body } => format!("ask|{tag}|{body}"),
                Talk::Answer { tag, body } => format!("ans|{tag}|{body}"),
            })
        }

        fn decode(&self, frame: Frame) -> Result<Talk, TestDecodeError> {
            let Frame::Text(text) = frame else {
                return Err(TestDecodeError::Unknown("not text".into()));
            };
            let tag = |s: &str| {
                s.parse::<u64>()
                    .map_err(|e| TestDecodeError::BadNumber(e.to_string()))
            };
            match text.splitn(3, '|').collect::<Vec<_>>().as_slice() {
                ["ask", t, body] => Ok(Talk::Ask {
                    tag: tag(t)?,
                    body: (*body).to_string(),
                }),
                ["ans", t, body] => Ok(Talk::Answer {
                    tag: tag(t)?,
                    body: (*body).to_string(),
                }),
                _ => Err(TestDecodeError::Unknown(text)),
            }
        }

        fn request_tag(&self, out: &Talk) -> Option<u64> {
            match out {
                Talk::Ask { tag, .. } => Some(*tag),
                Talk::Answer { .. } => None,
            }
        }

        fn reply_tag(&self, inn: &Talk) -> Option<u64> {
            match inn {
                Talk::Answer { tag, .. } => Some(*tag),
                Talk::Ask { .. } => None,
            }
        }

        fn stream_mark(&self, _inn: &Talk) -> Option<StreamMark<u32>> {
            None
        }

        fn resume_request(&self, _key: &u32, _after: Option<u64>) -> Talk {
            unreachable!("스트림을 안 쓴다")
        }
    }

    /// 수신단에 온 물음마다 그 물음이 온 연결로 답한다. 답한 수를 돌려준다.
    async fn answer_asks(peer: &Peer<TalkWire>, inbox: &mut Inbox<TalkWire>) -> usize {
        settle().await;
        let mut answered = 0;
        while let Some(item) = inbox.try_recv() {
            if let Some(Arrival {
                generation,
                msg: Talk::Ask { tag, body },
                ..
            }) = item.as_message()
            {
                let answer = Talk::Answer {
                    tag: *tag,
                    body: format!("re:{body}"),
                };
                peer.notify_in(*generation, answer).unwrap();
                answered += 1;
            }
        }
        answered
    }

    // transport 수준의 왕복이다 — 데몬끼리 어휘가 아니라 받는 쪽 자리가 양방향 요청을 나르는지를 잰다(TRD 3 §6-0 U6).
    #[tokio::test]
    async fn two_registries_ask_each_other_over_a_dialed_peer_and_an_attached_one() {
        let clock = ManualClock::new();
        let (mut listener, connector) = MemoryListener::new(clock.handle(), 4);
        let client = Registry::new(
            Arc::new(TalkWire),
            connector.dialer(),
            clock.handle(),
            Policy::default(),
        );
        let server = Registry::new(
            Arc::new(TalkWire),
            MemoryNetwork::new().dialer(),
            clock.handle(),
            Policy::default(),
        );
        let (to_server, mut client_inbox) = client.add(
            PeerId::new("server"),
            Address::new("mem://server"),
            immediate(),
        );
        let accepted = listener.accept().await.expect("받기");
        let (to_client, mut server_inbox) =
            server.attach(PeerId::new("conn-1"), accepted, immediate());
        settle_until("양쪽 운영", || {
            to_server.peer_state().is_live() && to_client.peer_state().is_live()
        })
        .await;

        // 요청은 수신단을 비우는 task 밖에서 기다린다(crate 헤더 「받는 쪽 불변식」 1).
        let ask = |peer: &Peer<TalkWire>, tag: u64, body: &str| {
            let peer = peer.clone();
            let out = Talk::Ask {
                tag,
                body: body.into(),
            };
            tokio::spawn(async move { peer.request(out).await })
        };
        let client_asked = ask(&to_server, 1, "ping");
        let server_asked = ask(&to_client, 2, "who");
        assert_eq!(answer_asks(&to_client, &mut server_inbox).await, 1);
        assert_eq!(answer_asks(&to_server, &mut client_inbox).await, 1);
        for (asked, expected) in [
            (
                client_asked,
                Talk::Answer {
                    tag: 1,
                    body: "re:ping".into(),
                },
            ),
            (
                server_asked,
                Talk::Answer {
                    tag: 2,
                    body: "re:who".into(),
                },
            ),
        ] {
            let got = tokio::time::timeout(Duration::from_secs(5), asked)
                .await
                .expect("답이 안 왔다")
                .expect("요청 태스크가 패닉했다")
                .expect("요청이 실패했다");
            assert_eq!(got, expected);
        }
    }

    #[tokio::test]
    async fn an_accepted_peer_flushes_what_it_holds_before_closing() {
        let mut a = Accepting::new(auth_policy());
        let (mut near, accepted) = a.connect().await;
        let far = a.far();
        let (peer, mut inbox) = a.attach("conn-1", accepted, immediate());
        settle_until("운영", || peer.peer_state().is_live()).await;
        far.stall_writes(true);
        peer.notify(TestOut::Notice("ack".into())).unwrap();
        peer.notify(TestOut::Notice("last".into())).unwrap();
        let closing = tokio::spawn(peer.close_after_flush(CloseCode::GOING_AWAY, "bye"));
        settle().await;
        assert!(!closing.is_finished(), "막힌 쓰기를 비우는 중이다");
        far.stall_writes(false);
        settle_until("비우고 닫았다", || closing.is_finished()).await;
        assert_eq!(
            near.arrived().await,
            vec![
                Seen::Frame(Frame::Text("not|ack".into())),
                Seen::Frame(Frame::Text("not|last".into())),
                Seen::Closed(Some(Close::new(CloseCode::GOING_AWAY, "bye"))),
            ]
        );
        assert_eq!(
            labels(&drain_until_quiet(&mut inbox).await),
            vec![
                "Connected:1",
                "Disconnected:1:Explicit",
                "Closed:1:Explicit"
            ]
        );
        assert!(ended(&mut inbox).await);
        assert!(a.registry.is_empty());
    }

    // ★옛 감독은 새것이 오른 뒤에 끝난다★ — 그 끝의 명부 내리기가 새것을 내리면 안 된다.
    #[tokio::test]
    async fn re_attaching_a_name_closes_the_old_peer_and_its_end_leaves_the_new_one_listed() {
        let mut a = Accepting::new(auth_policy());
        let (mut first, accepted) = a.connect().await;
        let (old, mut old_inbox) = a.attach("conn", accepted, immediate());
        settle_until("옛것 운영", || old.peer_state().is_live()).await;
        let (second, accepted) = a.connect().await;
        let (new, mut new_inbox) = a.attach("conn", accepted, immediate());
        settle_until("새것 운영", || new.peer_state().is_live()).await;
        assert_eq!(
            labels(&drain_until_quiet(&mut old_inbox).await),
            vec![
                "Connected:1",
                "Disconnected:1:Explicit",
                "Closed:1:Explicit"
            ]
        );
        assert!(ended(&mut old_inbox).await);
        assert_eq!(first.arrived().await, vec![closing()]);
        assert_eq!(
            labels(&drain_until_quiet(&mut new_inbox).await),
            vec!["Connected:2"],
            "갈아치운 손잡이의 계수기를 물려받는다 — 세대가 되감기지 않는다"
        );
        assert!(
            a.registry
                .get(&PeerId::new("conn"))
                .is_some_and(|listed| listed.same(&new)),
            "옛 감독의 끝이 새것을 내리지 않는다"
        );
        assert_eq!(a.generation_names(), 0);
        drop(second);
        drain_until_quiet(&mut new_inbox).await;
        assert!(a.registry.is_empty());
    }

    // 표에 있는 이름표(`add`)를 받은 통로로 갈아치우면 표의 계수기를 잇고(표는 안 자란다), 받은 상대를 `add` 로
    //   갈아치우면 그 계수기를 표에 올린다 — 어느 쪽이든 같은 번호를 두 번 뽑지 않는다.
    #[tokio::test]
    async fn a_name_passed_between_add_and_attach_never_mints_the_same_generation_twice() {
        let mut a = Accepting::new(auth_policy());
        let x = PeerId::new("x");
        let (dialed, _dialed_inbox) =
            a.registry
                .add(x.clone(), Address::new("mem://x"), immediate());
        settle_until("add 운영", || dialed.peer_state().is_live()).await;
        assert_eq!(dialed.generation(), Generation(1));
        let (_near, accepted) = a.connect().await;
        let (attached, _attached_inbox) = a.attach("x", accepted, immediate());
        settle_until("attach 운영", || attached.peer_state().is_live()).await;
        assert_eq!(attached.generation(), Generation(2));
        assert_eq!(a.generation_names(), 1, "읽기만 했다");

        let (_near, accepted) = a.connect().await;
        let (attached, _attached_inbox) = a.attach("y", accepted, immediate());
        settle_until("attach 운영", || attached.peer_state().is_live()).await;
        assert_eq!(attached.generation(), Generation(1));
        let (dialed, _dialed_inbox) =
            a.registry
                .add(PeerId::new("y"), Address::new("mem://y"), immediate());
        settle_until("add 운영", || dialed.peer_state().is_live()).await;
        assert_eq!(dialed.generation(), Generation(2));
        assert_eq!(a.generation_names(), 2);
    }

    #[tokio::test]
    async fn after_close_all_an_attach_or_an_add_is_closed_at_once_and_never_listed() {
        let mut a = Accepting::new(auth_policy());
        let (_listed_near, accepted) = a.connect().await;
        let (listed, mut listed_inbox) = a.attach("conn-0", accepted, immediate());
        settle_until("운영", || listed.peer_state().is_live()).await;
        let folding = tokio::spawn(a.registry.close_all(Duration::from_secs(3)));

        let (mut near, accepted) = a.connect().await;
        let (_late, mut inbox) = a.attach("conn-1", accepted, immediate());
        assert!(a.registry.get(&PeerId::new("conn-1")).is_none());
        assert_eq!(
            labels(&drain_until_quiet(&mut inbox).await),
            vec!["Closed:0:Explicit"]
        );
        assert!(ended(&mut inbox).await);
        assert_eq!(near.arrived().await, vec![closing()]);

        let (_dialed, mut dialed_inbox) =
            a.registry
                .add(PeerId::new("d"), Address::new("mem://d"), immediate());
        assert_eq!(
            labels(&drain_until_quiet(&mut dialed_inbox).await),
            vec!["Closed:0:Explicit"]
        );
        assert!(ended(&mut dialed_inbox).await);
        assert_eq!(
            a.redials.dials(),
            0,
            "접은 명부에 올린 상대는 걸지도 않는다"
        );

        assert_eq!(
            labels(&drain_until_quiet(&mut listed_inbox).await),
            vec![
                "Connected:1",
                "Disconnected:1:Explicit",
                "Closed:1:Explicit"
            ]
        );
        settle_until("접기 끝", || folding.is_finished()).await;
        assert_eq!(folding.await.expect("close_all 태스크"), Ok(()));
        assert!(a.registry.is_empty());
    }

    #[tokio::test]
    async fn the_upgrade_permit_is_held_through_the_handshake_and_given_back_at_live_or_at_failure()
    {
        let mut a = Accepting::new(auth_policy());
        let permits = a.listener.permits();
        let (mut near, accepted) = a.connect().await;
        assert_eq!(permits.in_use(), 1, "준비 큐");
        let (peer, _inbox) = a.attach("conn-1", accepted, Arc::new(TokenGate));
        settle().await;
        assert_eq!(
            permits.in_use(),
            1,
            "인증 대기 칸이 생기기 전까지 핸드셰이크 동안 쥔다"
        );
        near.say(GOOD).await;
        settle_until("운영", || peer.peer_state().is_live()).await;
        assert_eq!(permits.in_use(), 0, "운영 단계에 들면 놓는다");

        let (mut stale, accepted) = a.connect().await;
        assert_eq!(permits.in_use(), 1);
        let (_failed, _failed_inbox) = a.attach("conn-2", accepted, Arc::new(TokenGate));
        stale.say("auth|stale").await;
        settle().await;
        assert_eq!(
            permits.in_use(),
            0,
            "인증에 실패한 상대는 수신단을 안 비워도 칸을 놓는다"
        );
    }

    #[tokio::test]
    async fn an_accepted_peer_ignores_reconnect_now_and_folds_once_its_inbox_is_dropped() {
        let mut a = Accepting::new(auth_policy());
        let (mut near, accepted) = a.connect().await;
        let (peer, mut inbox) = a.attach("conn-1", accepted, immediate());
        settle_until("운영", || peer.peer_state().is_live()).await;
        peer.reconnect_now();
        settle().await;
        assert!(
            peer.peer_state().is_live(),
            "걸 주소가 없다 — 운영 중인 통로를 안 버린다"
        );
        assert_eq!(
            labels(&drain_until_quiet(&mut inbox).await),
            vec!["Connected:1"]
        );
        assert!(near.arrived().await.is_empty());

        drop(inbox);
        settle().await;
        assert_eq!(near.arrived().await, vec![closing()]);
        assert_eq!(peer.peer_state(), PeerState::Closed);
        assert!(a.registry.is_empty());
    }

    // 정리는 받는 쪽이 수신단을 비우기를 기다린다 — 그 사이 명부에 끝난 손잡이가 보이면 안 된다.
    #[tokio::test]
    async fn an_accepted_peer_leaves_the_roster_before_its_teardown_waits_on_the_inbox() {
        let mut a = Accepting::new(Policy {
            inbound_queue: 1,
            ..auth_policy()
        });
        let (_near, accepted) = a.connect().await;
        let (peer, mut inbox) = a.attach("conn-1", accepted, immediate());
        settle_until("운영", || peer.peer_state().is_live()).await;
        peer.close();
        settle().await;
        assert!(
            a.registry.is_empty(),
            "수신단이 차 정리가 서 있어도 명부에는 없다"
        );
        assert_eq!(
            labels(&drain_until_quiet(&mut inbox).await),
            vec![
                "Connected:1",
                "Disconnected:1:Explicit",
                "Closed:1:Explicit"
            ]
        );
        assert!(ended(&mut inbox).await);
    }

    #[tokio::test]
    async fn an_accepted_peer_whose_supervisor_panics_still_leaves_the_roster() {
        let mut a = Accepting::new(auth_policy());
        let (_near, accepted) = a.connect().await;
        let (_peer, mut inbox) = a.attach("conn-1", accepted, Arc::new(PanickingHandshake));
        settle_until("감독이 끝났다", || a.registry.is_empty()).await;
        assert!(drain_until_quiet(&mut inbox).await.is_empty());
        assert!(ended(&mut inbox).await, "`Closed` 없는 `None`");
    }

    // ── 3-0 U6b: 일꾼 스레드가 여럿인 런타임에서 올리기 ──
    //
    // ★`settle` 을 못 쓴다★(current_thread 전용 — `testing.rs`) — 끝은 실시간 시한을 건 대기로 본다. OS 스레드
    //   `RACE_THREADS` 개가 장벽에서 함께 출발해 같은 명부를 두드린다 — 감독은 그 사이 일꾼 스레드에서 돈다.

    const RACE_THREADS: usize = 4;
    const REAL_WAIT: Duration = Duration::from_secs(10);

    /// 수신단을 끝(`None`)까지 비운다 — 실시간 [`REAL_WAIT`] 안에 안 끝나면 패닉한다.
    async fn drain_to_end(inbox: &mut Inbox<TestWire>) -> Vec<Incoming<TestWire>> {
        tokio::time::timeout(REAL_WAIT, async {
            let mut out = Vec::new();
            while let Some(item) = inbox.recv().await {
                out.push(item);
            }
            out
        })
        .await
        .expect("수신단이 실시간 시한 안에 안 끝났다")
    }

    /// 조건이 서기를 실시간 [`REAL_WAIT`] 까지 기다린다.
    async fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        tokio::time::timeout(REAL_WAIT, async {
            while !done() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("실시간 시한 안에 안 섰다: {what}"));
    }

    /// 받은 통로 하나와 그 거는 쪽 끝 — 업그레이드 끝 = `upgraded_at`.
    fn accepted_link(pool: &PermitPool, upgraded_at: Instant) -> (Near, Accepted) {
        let (near, far) = memory_link();
        let permit = pool.try_take().expect("업그레이드 칸");
        let accepted = Accepted::new(far.tx, far.rx, LinkInfo::default(), upgraded_at, permit);
        (
            Near {
                tx: near.tx,
                rx: near.rx,
            },
            accepted,
        )
    }

    /// OS 스레드 [`RACE_THREADS`] 개가 런타임 문맥 안에서 장벽을 함께 넘어 `call(스레드 번호)` 를 돌린다. ★이 대기는
    /// 시험 본문 스레드를 막는다★ — multi_thread 시험의 본문은 일꾼이 아니라 `block_on` 스레드라 감독은 계속 돈다.
    fn race<T: Send>(call: impl Fn(usize) -> T + Sync) -> Vec<T> {
        let runtime = tokio::runtime::Handle::current();
        let start = std::sync::Barrier::new(RACE_THREADS);
        std::thread::scope(|scope| {
            let threads: Vec<_> = (0..RACE_THREADS)
                .map(|t| {
                    let (runtime, start, call) = (&runtime, &start, &call);
                    scope.spawn(move || {
                        let _entered = runtime.enter();
                        start.wait();
                        call(t)
                    })
                })
                .collect();
            threads
                .into_iter()
                .map(|thread| thread.join().expect("경합 스레드가 패닉했다"))
                .collect()
        })
    }

    /// 거는 쪽 끝을 끝(닫힘 · 오류)까지 읽는다 — 실시간 [`REAL_WAIT`] 까지. ★[`Near::arrived`] 를 쓰지 않는다★ — 기다리지
    /// 않고 한 번만 폴링해, tokio 의 협조 예산이 바닥난 task 에서는 와 있는 것도 못 읽는다.
    async fn read_to_end(near: &mut Near) -> Vec<Seen> {
        tokio::time::timeout(REAL_WAIT, async {
            let mut out = Vec::new();
            loop {
                let seen = match near.rx.recv().await {
                    Ok(LinkRead::Frame(frame)) => Seen::Frame(frame),
                    Ok(LinkRead::Closed(close)) => Seen::Closed(close),
                    Err(_) => Seen::Failed,
                };
                let end = !matches!(seen, Seen::Frame(_));
                out.push(seen);
                if end {
                    return out;
                }
            }
        })
        .await
        .expect("거는 쪽 끝이 실시간 시한 안에 안 끝났다")
    }

    fn connected_generations(items: &[Incoming<TestWire>]) -> Vec<u64> {
        items
            .iter()
            .filter_map(|item| match item {
                Incoming::Event(TransportEvent::Connected { generation, .. }) => Some(generation.0),
                _ => None,
            })
            .collect()
    }

    // 첫 폴링에 곧바로 끝나는 받은 상대 — 시한이 이미 지났거나(`Idle`) 거는 쪽이 이미 끊었다. 올리기가 띄우기보다
    //   뒤면 다른 일꾼에서 명부 내리기까지 마친 감독의 손잡이가 그 뒤에 올라 영영 남는다(`Registry::launch`).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn accepted_peers_that_end_on_their_first_poll_never_stay_listed() {
        const PER_THREAD: usize = 50;
        let clock = ManualClock::new();
        let registry = Registry::new(
            Arc::new(TestWire),
            MemoryNetwork::new().dialer(),
            clock.handle(),
            auth_policy(),
        );
        let upgraded_at = clock.handle().now();
        clock.advance(AUTH * 2);
        let pool = PermitPool::new(RACE_THREADS * PER_THREAD);
        let attached = race(|t| {
            (0..PER_THREAD)
                .map(|n| {
                    let past_deadline = n % 2 == 0;
                    let at = if past_deadline {
                        upgraded_at
                    } else {
                        clock.handle().now()
                    };
                    let (near, accepted) = accepted_link(&pool, at);
                    if !past_deadline {
                        drop(near);
                    }
                    let (_peer, inbox) = registry.attach(
                        PeerId::new(format!("conn-{t}-{n}")),
                        accepted,
                        Arc::new(TokenGate),
                    );
                    inbox
                })
                .collect::<Vec<_>>()
        });
        for mut inbox in attached.into_iter().flatten() {
            assert_eq!(kinds(&drain_to_end(&mut inbox).await), vec!["Closed"]);
        }
        assert_eq!(registry.len(), 0, "끝난 받은 상대는 명부에 남지 않는다");
        assert_eq!(pool.in_use(), 0);
    }

    // 접은 명부에 올린 상대는 띄우기 전에 닫기가 들어 있어, 어느 일꾼에서 먼저 돌든 걸지도 핸드셰이크하지도 않는다.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn after_close_all_no_attach_shakes_hands_and_no_add_dials_on_a_multi_thread_runtime() {
        const PER_THREAD: usize = 25;
        let clock = ManualClock::new();
        let net = MemoryNetwork::new();
        let registry = Registry::new(
            Arc::new(TestWire),
            net.dialer(),
            clock.handle(),
            auth_policy(),
        );
        assert_eq!(registry.close_all(Duration::from_secs(1)).await, Ok(()));
        let pool = PermitPool::new(RACE_THREADS * PER_THREAD);
        let late = race(|t| {
            (0..PER_THREAD)
                .map(|n| {
                    let (near, accepted) = accepted_link(&pool, clock.handle().now());
                    let (_attached, attached_inbox) = registry.attach(
                        PeerId::new(format!("conn-{t}-{n}")),
                        accepted,
                        Arc::new(HelloHandshake),
                    );
                    let (_dialed, dialed_inbox) = registry.add(
                        PeerId::new(format!("d-{t}-{n}")),
                        Address::new("mem://d"),
                        immediate(),
                    );
                    (near, attached_inbox, dialed_inbox)
                })
                .collect::<Vec<_>>()
        });
        for (mut near, mut attached_inbox, mut dialed_inbox) in late.into_iter().flatten() {
            for inbox in [&mut attached_inbox, &mut dialed_inbox] {
                assert_eq!(
                    labels(&drain_to_end(inbox).await),
                    vec!["Closed:0:Explicit"]
                );
            }
            assert_eq!(
                read_to_end(&mut near).await,
                vec![closing()],
                "핸드셰이크 프레임(hello) 없이 닫기만 온다"
            );
        }
        assert_eq!(net.dials(), 0, "접은 명부에 올린 상대는 걸지 않는다");
        assert!(registry.is_empty());
        assert_eq!(pool.in_use(), 0);
    }

    // 같은 이름표를 여러 호출이 동시에 `add` · `attach` 로 갈아치운다. ★표에 칸이 없는 받은 상대가 올라 있을 때가
    //   경합 자리다★ — 그 손잡이를 빼는 자리와 계수기를 고르는 자리가 갈리면 다른 호출이 손잡이도 표의 칸도 못 보고 새
    //   계수기를 만들어 이미 나간 세대를 다시 뽑는다(`Registry::launch`). 그래서 이름표마다 그 상태로 시작한다.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn racing_replacements_of_one_name_never_mint_the_same_generation_twice() {
        const NAMES: usize = 60;
        const PER_THREAD: usize = 4;
        let clock = ManualClock::new();
        let net = MemoryNetwork::new();
        let registry = Registry::new(
            Arc::new(TestWire),
            net.dialer(),
            clock.handle(),
            Policy::default(),
        );
        let pool = PermitPool::new(NAMES * (1 + RACE_THREADS * PER_THREAD));
        for round in 0..NAMES {
            let name = PeerId::new(format!("x{round}"));
            let (first_near, accepted) = accepted_link(&pool, clock.handle().now());
            let (first, first_inbox) = registry.attach(name.clone(), accepted, immediate());
            wait_until("받은 상대 운영", || first.peer_state().is_live()).await;
            let raced = race(|t| {
                (0..PER_THREAD)
                    .map(|n| {
                        if (t + n) % 2 == 0 {
                            let (_peer, inbox) =
                                registry.add(name.clone(), Address::new("mem://x"), immediate());
                            (None, inbox)
                        } else {
                            let (near, accepted) = accepted_link(&pool, clock.handle().now());
                            let (_peer, inbox) =
                                registry.attach(name.clone(), accepted, immediate());
                            (Some(near), inbox)
                        }
                    })
                    .collect::<Vec<_>>()
            });
            // 경합 뒤 표에 남은 계수기로 한 번 더 붙는다 — 경합이 새 계수기를 표에 올렸으면 여기서 1 을 다시 뽑는다.
            let (last, last_inbox) =
                registry.add(name.clone(), Address::new("mem://x"), immediate());
            wait_until("마지막 상대 운영", || last.peer_state().is_live()).await;
            registry.remove(&name);

            let mut nears = vec![first_near];
            let mut inboxes = vec![first_inbox, last_inbox];
            for (near, inbox) in raced.into_iter().flatten() {
                nears.extend(near);
                inboxes.push(inbox);
            }
            let mut minted = Vec::new();
            for inbox in &mut inboxes {
                minted.extend(connected_generations(&drain_to_end(inbox).await));
            }
            let mut unique = minted.clone();
            unique.sort_unstable();
            unique.dedup();
            assert_eq!(
                unique.len(),
                minted.len(),
                "{name:?}: 같은 세대를 두 번 뽑았다 — {minted:?}"
            );
            drop(nears);
        }
    }

    /// [`TrippingClock`] 이 터뜨리는 패닉의 값.
    struct Tripped;

    /// 정해 둔 스레드의 n 번째 `now()` 에서 한 번 패닉하는 시계 — 그 밖은 [`ManualClock`] 그대로다. ★스레드로
    /// 거른다★ — 다른 감독이 일꾼 스레드에서 읽는 것이 세는 수에 섞이지 않게.
    #[derive(Default)]
    struct TrippingClock {
        clock: ManualClock,
        trip: Mutex<Option<(std::thread::ThreadId, usize)>>,
    }

    impl TrippingClock {
        /// 부른 스레드의 `nth` 번째 읽기에서 패닉하게 한다.
        fn arm(&self, nth: usize) {
            *self.trip.lock().unwrap() = Some((std::thread::current().id(), nth));
        }
    }

    impl Clock for TrippingClock {
        fn now(&self) -> Instant {
            let fire = {
                let mut trip = self.trip.lock().unwrap();
                match trip.as_mut() {
                    Some((thread, left)) if *thread == std::thread::current().id() => {
                        *left -= 1;
                        let fire = *left == 0;
                        if fire {
                            *trip = None;
                        }
                        fire
                    }
                    _ => false,
                }
            };
            // 자물쇠를 놓은 뒤에 터뜨린다 — 쥔 채 터지면 그 독이 다른 감독의 읽기까지 터뜨린다.
            if fire {
                std::panic::panic_any(Tripped);
            }
            self.clock.now()
        }

        fn sleep(&self, d: Duration) -> futures_util::future::BoxFuture<'static, ()> {
            self.clock.sleep(d)
        }
    }

    /// `call` 을 런타임 문맥에 든 OS 스레드에서 패닉을 잡으며 돌린다 — 실시간 [`REAL_WAIT`] 안에 안 끝나면 교착으로
    /// 보고 패닉한다. ★그 스레드를 떼어 둔다★ — 교착한 스레드를 기다리면 시험이 실패하지 않고 매달린다.
    fn off_thread<T: Send + 'static>(
        call: impl FnOnce() -> T + Send + 'static,
    ) -> std::thread::Result<T> {
        let runtime = tokio::runtime::Handle::current();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _entered = runtime.enter();
            let _ = done_tx.send(std::panic::catch_unwind(std::panic::AssertUnwindSafe(call)));
        });
        done_rx
            .recv_timeout(REAL_WAIT)
            .expect("실시간 시한 안에 안 끝났다 — 명부 자물쇠 교착")
    }

    // 갈아치우는 `attach` · `add` 가 명부 자물쇠 안에서 감독을 짓다 패닉한다(주입한 시계). ★받은 상대는 짓기 뒤에
    //   패닉하면 풀리며 떨어지는 명부 내리기가 같은 스레드에서 자물쇠를 다시 잡는다★(`peer.rs` 의 `new_peer`) — 그
    //   교착이 시험을 매달지 않게 실시간 시한을 건 스레드에서 부른다. 짓기 전에 표를 바꾸면 갈아치우려던 손잡이가
    //   명부에서 사라진다(`Registry::launch`).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_replacement_that_panics_while_preparing_leaves_the_roster_untouched_and_usable() {
        let clock = Arc::new(TrippingClock::default());
        let registry = Registry::new(
            Arc::new(TestWire),
            MemoryNetwork::new().dialer(),
            clock.clone(),
            Policy::default(),
        );
        let pool = PermitPool::new(2);
        let name = PeerId::new("a");
        let (first, mut first_inbox) =
            registry.add(name.clone(), Address::new("mem://a"), immediate());
        let still_first = |registry: &Registry<TestWire>| {
            registry.len() == 1 && registry.get(&name).is_some_and(|peer| peer.same(&first))
        };

        // debug 빌드는 `attach_with_policy` 의 시계 단언이 짓기의 읽기 하나 앞에 선다.
        let nth = if cfg!(debug_assertions) { 2 } else { 1 };
        let (_near, accepted) = accepted_link(&pool, clock.now());
        let attached = off_thread({
            let (registry, clock, name) = (registry.clone(), clock.clone(), name.clone());
            move || {
                clock.arm(nth);
                registry.attach(name, accepted, immediate())
            }
        });
        assert!(attached.is_err_and(|panic| panic.is::<Tripped>()));
        assert!(
            still_first(&registry),
            "받은 상대 짓기가 패닉해도 표는 그대로다"
        );
        assert_eq!(pool.in_use(), 0, "업그레이드 칸은 패닉과 함께 돌아간다");

        let added = off_thread({
            let (registry, clock, name) = (registry.clone(), clock.clone(), name.clone());
            move || {
                clock.arm(1);
                registry.add(name, Address::new("mem://a"), immediate())
            }
        });
        assert!(added.is_err_and(|panic| panic.is::<Tripped>()));
        assert!(
            still_first(&registry),
            "건 상대 짓기가 패닉해도 표는 그대로다"
        );
        wait_until("갈아치우려던 상대 운영", || {
            first.peer_state().is_live()
        })
        .await;

        let (_near, accepted) = accepted_link(&pool, clock.now());
        let (replaced, _replaced_inbox) = off_thread({
            let (registry, name) = (registry.clone(), name.clone());
            move || registry.attach(name, accepted, immediate())
        })
        .expect("패닉 뒤에도 명부를 쓴다");
        assert!(registry.get(&name).is_some_and(|peer| peer.same(&replaced)));
        assert_eq!(
            kinds(&drain_to_end(&mut first_inbox).await).last(),
            Some(&"Closed"),
            "갈아치운 손잡이는 닫힌다"
        );
    }
}
