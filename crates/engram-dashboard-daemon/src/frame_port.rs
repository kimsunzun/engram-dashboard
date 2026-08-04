//! 네트워크 행이 위층으로 뚫는 **유일한 구멍** — 불투명 프레임 포트(ADR-0129).
//!
//! 여기 정의된 계약은 text/binary/close 세 가지 프레임만 안다. `AgentCommand`·`AgentEvent`·
//! 프로토콜 인코딩은 한 낱말도 등장하지 않으며, 그 어휘를 아는 쪽은 전부 위층(`agent_conn`)이다.
//! 계약(trait)은 아래층인 네트워크 행이 소유하고 실물은 양쪽이 나눠 꽂는다 — `FrameSink`(연결당
//! 출구)와 `FrameFanout`(전-연결 출구)은 네트워크 행(`ws::ConnFrameSink`·`ws::ConnRegistry`)이,
//! `ConnectionHandler`/`ConnectionHandlerFactory` 는 위층이 구현한다(ADR-0110 의 포트 소유 idiom과
//! 같은 결).

use std::sync::Arc;

use futures_util::future::BoxFuture;

/// 연결 식별자(단조 증가). 두 포트 trait 의 모든 메서드가 이걸 받는다 — 그래서 carrier 구현이 아니라
/// 포트 쪽에 산다(레지스트리 키로도 쓰인다).
pub type ConnId = u64;

/// 연결당 단일 writer 큐로 흐르는 출력 단위 — 프레임 어휘는 이 셋이 전부다.
#[derive(Debug)]
pub enum Frame {
    /// 텍스트 페이로드(내용이 무엇인지는 이 층의 관심사가 아니다).
    Text(String),
    /// binary 페이로드(codec frame 등).
    Binary(Vec<u8>),
    /// 연결 종료 — writer 가 이걸 받으면 close 후 루프를 나간다. 큐 **안**으로 들어가므로 앞서 넣은
    /// 프레임이 먼저 다 나간다(FIFO). `reason` 은 로그/디버깅용이라 클라에 전달되지 않는다.
    Close(String),
}

/// 프레임 큐잉 실패 — 연결 송신 큐가 포화이거나 이미 닫혔다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameError;

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "frame sink enqueue failed")
    }
}

impl std::error::Error for FrameError {}

/// 한 연결로 나가는 프레임의 유일한 출구. 위층은 이걸로만 내보내므로 네트워크 행은 실린 내용의
/// 어휘를 모른다.
///
/// ★두 메서드는 포화 시 **다르게 행동한다**(load-bearing 비대칭)★:
/// - `try_send` = **논블록 전용**. 출력 pump 스레드·status 콜백 같은 block 금지 경로에서 불린다.
///   포화면 즉시 `FrameError` 이고, 구현은 그 순간 큐 **밖**의 종료 신호를 울려야 한다 — 포화
///   상태에선 `Frame::Close` 조차 큐에 못 들어가 좀비 연결이 되기 때문이다(`ws` 모듈 헤더).
/// - `send` = async 문맥 전용, backpressure 허용(자리가 날 때까지 대기). 여기서는 종료 신호를
///   **울리지 않는다** — 기다릴 수 있는 호출자라 슬로우 소비자 판정 대상이 아니다.
// ADR-0129
pub trait FrameSink: Send + Sync {
    fn try_send(&self, frame: Frame) -> Result<(), FrameError>;

    fn send(&self, frame: Frame) -> BoxFuture<'_, Result<(), FrameError>>;
}

/// 등록된 연결 **전부**를 대상으로 같은 text 를 미는 전-연결 팬아웃 표면(실제 배달은 아래 의무 2·3
/// 대로 best-effort). 불투명 프레임 구멍의 두 번째 모양이다 — 연결당은 위 `FrameSink`, 전-연결은
/// 이것(ADR-0129 결정 1의 2026-08-04 note).
///
/// ★`Frame` 이 아니라 `String` 인 이유★: 이 포트로 넘어오는 것은 위층이 **이미 인코딩해 둔 text** 하나다
///   (무엇을 인코딩한 것인지는 위층 사정이고 이 계약의 관심사가 아니다). `Frame` 으로 넓히면
///   `Frame::Close` 를 전 연결에 흘리는 것 — 즉 **전-연결 수명 조작** — 이 위층에서 표현 가능해지는데,
///   연결 수명은 네트워크 행이 소유하는 관심사다. 좁은 타입이 그 조작을 애초에 표현 불가로 만든다.
///   덧붙여 `Frame` 은 `Clone` 이 아니므로, 팬아웃 구현이 연결마다 복제하려면 페이로드를 다시 꺼내
///   감싸야 한다 — 좁은 타입은 그 복제 루프도 정직하게 유지한다.
///
/// ★구현 의무 셋 — 위층 동작이 여기 걸려 있다★:
/// 1. **논블록**. 코어의 동기 스레드(pump/manager 의 상태 콜백)에서도 불린다. 여기서 block 하면
///    에이전트 출력 pump 가 통째로 선다.
/// 2. **연결 하나의 실패가 나머지를 막지 않는다**. 큐가 포화한 연결은 건너뛰고 남은 연결에 계속
///    배달해야 한다 — 어기면 슬로우 클라 하나가 전 클라의 갱신을 세운다.
/// 3. **실패를 호출자에게 알리지 않는다**(반환값 없음). 누가 못 받았는지는 이 포트로 관측되지
///    않으므로 호출자는 배달을 전제해선 안 된다.
///
/// ★`FrameSink::try_send` 와의 비대칭(load-bearing)★: 연결당 `try_send` 는 포화를 만나면 그 연결의
///   종료 신호를 울리지만, **팬아웃의 포화는 연결 종료로 잇지 않는다**. 겨냥한 연결이 없는 호출이라
///   슬로우 소비자 판정을 여기 얹지 않는다는 뜻이고, 바꾸려면 별도 결정이 필요하다.
// ADR-0129
pub trait FrameFanout: Send + Sync {
    fn broadcast_text(&self, text: String);
}

/// 프레임 하나를 처리한 뒤 수신 루프가 할 일.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnFlow {
    Continue,
    /// 수신 루프를 빠져나와 이 연결을 닫는다(예: StopDaemon·프로토콜 위반).
    Close,
}

/// 한 연결의 수명 이벤트를 받는 위층 핸들러. 네트워크 행은 소켓과 프레임만 다루고 "그 프레임이
/// 무슨 뜻인가" 는 전부 이 trait 뒤로 넘긴다.
///
/// ★호출 순서★: `on_connect` → (`on_text` | `on_binary`)\* → `on_disconnect`. 연결당 `on_connect`·
///   `on_disconnect` 는 각 1회, 가운데는 0회 이상이다.
///
/// ★`on_disconnect` 는 조용한 시점이 **아니다** — 구현이 반드시 알아야 하는 잔여 경쟁★:
///   네트워크 행은 이 호출 직전에 살아남은 task 에 `abort()` 를 걸지만 그 완료를 기다리지 않는다.
///   `abort()` 는 취소 *요청*이라, 멀티 워커 런타임에서는 취소된 수신 루프가 아직
///   `on_text`/`on_binary` 의 `.await` 안에 있을 수 있다 — 즉 그 호출과 `on_disconnect` 가 **겹칠 수
///   있다**. 예: `on_disconnect` 가 구독 목록을 스냅샷한 **뒤** 등록된 구독은 정리에서 빠진다.
///   구현은 자기 상태를 스스로 동기화해야 하고 "이 시점엔 아무도 안 건드린다" 를 전제하면 안 된다.
///   ★이 경쟁은 이 포트가 만든 것이 아니라 HEAD 도 동일하다★(정리 블록이 `handle_connection` 안에
///   인라인이던 때도 abort 직후 그대로 실행됐다) — 닫을지 여부는 이 슬라이스 밖의 별도 결정이다.
///
/// ★`&Arc<dyn FrameSink>` 인 이유★: 구현이 이 연결보다 오래 사는 sink(코어 subscribers 에 등록되는
///   출력 sink)를 만들려면 `'static` 공유 핸들이 필요하다 — 빌린 참조로는 못 만든다.
/// ★async 표현★: `async fn` in trait 은 dyn 호환이 아니므로 수동 `BoxFuture` 로 쓴다(새 의존 없이).
// ADR-0129
pub trait ConnectionHandler: Send + Sync {
    /// 연결 직후 1회. 여기서 넣은 프레임이 **이 핸들러가 내보내는** 첫 출력이다.
    ///
    /// ★연결 전체의 첫 프레임이라는 뜻은 아니다★: 등록이 이 호출보다 앞서므로, 그 사이 전-연결
    ///   팬아웃(`FrameFanout`)이 같은 큐에 먼저 넣었을 수 있다. FIFO 선두를 점유한다고 가정하지 말 것.
    ///
    /// ★큐 소비자가 아직 없다 — 이게 호출자 쪽 조건이 아니라 **구현 쪽 의무**다★: 이 호출은
    ///   writer task 가 **뜨기 전에** await 되므로 여기서 넣는 프레임은 아무도 빼가지 않는다.
    ///   그래서 ① 프레임을 조금만 넣고 ② 그 밖의 이유로도 오래 block 하면 안 된다. 어기면
    ///   `FrameSink::send` 가 영영 반환하지 않고, 그 연결은 **인증·등록된 채로 reader 도 writer 도
    ///   타임아웃도 `on_disconnect` 도 없이** 남는다.
    ///
    /// ★안전 상한을 숫자로 못 박을 수 없다★: 큐 용량(`ws::CONN_TX_CAP`)을 그 상한으로 읽지 말 것.
    ///   연결은 이 호출 **전에** 이미 fanout 레지스트리에 등록되므로, 그 순간부터 전-연결 팬아웃
    ///   (`FrameFanout`)이 같은 큐에 동시에 슬롯을 넣는다. 즉 실제 여유는 용량보다 **작고 가변**이다.
    fn on_connect<'a>(
        &'a self,
        conn_id: ConnId,
        frames: &'a Arc<dyn FrameSink>,
    ) -> BoxFuture<'a, ()>;

    /// text 프레임 1개. `text` 는 수신 버퍼를 빌려주는 것이라 소유권을 넘기지 않는다.
    fn on_text<'a>(
        &'a self,
        conn_id: ConnId,
        text: &'a str,
        frames: &'a Arc<dyn FrameSink>,
    ) -> BoxFuture<'a, ConnFlow>;

    /// 클라 → 데몬 binary 프레임. **어떤 프레임 종류가 유효한가는 위층 프로토콜의 판단**이라
    /// 네트워크 행이 거부하지 않고 그대로 올린다. `payload` 도 빌림이다 — 필요하면 구현이 복사한다
    /// (현 구현은 내용을 보지 않고 거부하므로 복사하지 않는다).
    fn on_binary<'a>(
        &'a self,
        conn_id: ConnId,
        payload: &'a [u8],
        frames: &'a Arc<dyn FrameSink>,
    ) -> BoxFuture<'a, ConnFlow>;

    /// 연결 정리(구독 해제·자원 반납). 동기 — 정리 경로에 await 지점이 없다.
    ///
    /// ★구현 의무 — 반환 시점까지 자기가 만든 `FrameSink` 사본을 **전부 놓아야 한다**★: 이 포트를
    ///   통해 만든 sink(특히 오래 사는 곳에 등록한 것)를 여기서 회수하지 않으면, 네트워크 행의 송신
    ///   큐 사본이 살아남아 writer task 가 스스로 끝나지 못한다(연결이 끝나도 회수되지 않는 task).
    ///   현 구현이 이 의무를 어떻게 지키는지, 그리고 그 회수를 놓치는 알려진 경쟁은
    ///   `agent_conn::AgentConnection::on_disconnect` 주석에 있다 — 위 abort-겹침 경쟁과 같은 뿌리다.
    ///
    /// ★이 시점 이 연결은 아직 fanout 레지스트리에 남아 있다★(등록 해제는 네트워크 행이 이 호출
    /// **뒤에** 한다). 다만 여기서 나가는 브로드캐스트가 **자기 자신에게 배달된다고 전제할 수 없다**
    /// — 사실상 대개 버려지지만 보장은 아니다:
    /// - writer 가 **먼저 끝난 갈래**에서는 큐의 수신단이 이미 사라져 확실히 버려진다.
    /// - reader 가 먼저 끝난 갈래에서는 writer 가 `abort()` **표시만** 된 상태라, 취소가 실제로
    ///   먹기 전에 이미 큐에 든 프레임을 집어 소켓으로 내보낼 수 있다.
    /// 그래서 안전한 방향 하나만 지키면 된다 — **자기 배달을 전제로 한 정리 로직을 쓰지 말 것.**
    ///
    /// ★이 순서에 배달 정합성이 걸려 있지는 **않다**★: 브로드캐스트는 레지스트리 맵 스냅샷을 떠서
    /// 돌므로, 등록 해제를 앞당기든 미루든 **다른 연결들이 받는 것은 완전히 같다**. 이 순서를 지키는
    /// 이유는 ADR-0129 슬라이스를 순수 리팩터로 유지하려는 것뿐이다(HEAD 와 같은 순서).
    fn on_disconnect(&self, conn_id: ConnId);
}

/// 연결마다 핸들러를 만드는 위층 공장. `handle_connection` 은 이것 하나만 들기 때문에 에이전트
/// 어휘를 타입으로도 모른다.
// ADR-0129
pub trait ConnectionHandlerFactory: Send + Sync {
    fn handler_for(&self, conn_id: ConnId) -> Arc<dyn ConnectionHandler>;

    /// 핸드셰이크 실패를 클라에 알릴 text 프레임. 이 시점엔 연결 큐도 핸들러도 아직 없고 네트워크
    /// 행은 오류 표현 어휘를 모르므로, 인코딩만 위층에서 받아 소켓으로 그대로 흘린다.
    /// `None` = 인코딩 실패 → 본문 없이 close 만 한다.
    fn handshake_error_frame(&self, message: &str) -> Option<String>;
}
