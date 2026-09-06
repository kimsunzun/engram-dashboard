//! 전송 seam — 통로를 여는 것(`Dialer`)과 열린 통로의 양 끝(`LinkTx`/`LinkRx`), 그리고 그 위에서
//! 도는 불투명 핸드셰이크.
//!
//! ★왜 제네릭이 아니라 dyn 인가★ — 한 명부 안에 **서로 다른 전송이 섞일 수 있어야** 한다(로컬 WS 상대와
//! 나중의 TLS 상대를 같은 [`crate::Registry`] 에 담는다). 제네릭 파라미터로는 그 컬렉션이 표현되지 않는다.
//! 프레임은 이미 `String`/`Vec<u8>` 값이라 프레임당 dyn 호출은 syscall 옆에서 잡음이다. (ADR-0177 결정 7)
//!
//! ★`async fn` in trait 을 쓰지 않는 이유★ — dyn 호환이 아니다. 그래서 `BoxFuture` 로 수동 박싱한다
//! (같은 저장소의 `net::frame_port` 가 같은 이유로 같은 형태다).

use std::fmt;

use futures_util::future::BoxFuture;

use crate::frame::{Close, Frame};

/// 상대의 주소. ★crate 는 이 문자열을 파싱하지 않는다★ — 뜻은 [`Dialer`] 구현체만 안다.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Address(pub String);

impl Address {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// 통로 자체가 낸 오류. ★사람이 읽는 문구뿐이고 기계 분기용 코드가 없다★ — 전송마다 오류 어휘가
/// 달라서 여기서 갈래를 못 박으면 어댑터가 자기 사유를 억지로 우리 갈래에 욱여넣는다. 분기가 필요한
/// 구분(못 붙음 / 붙었는데 거절)은 [`crate::ConnectFailure`] 가 진다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkError {
    pub message: String,
}

impl LinkError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for LinkError {}

/// 통로에서 읽어 올린 것.
///
/// ★[`LinkRead::Closed`] 가 닫기 코드를 나르는 것이 계약의 핵심이다★ — 이 칸이 없으면 「상대가 나를
/// 거절했다」와 「상대가 그냥 사라졌다」를 받는 쪽이 구별할 수 없고, 그러면 ADR-0180 결정 5(거절은 재연결
/// 예산을 안 쓴다)가 **코드로는 성립하지 않는다**. 거절은 재시도해도 결과가 같으므로 예산을 태우면 안 되고,
/// 그 판정 재료가 이 코드다.
pub enum LinkRead {
    Frame(Frame),
    /// 상대가 닫았다. `Some` 이면 **코드와 문구를 실어** 닫았다는 뜻이고, `None` 이면 말 없이 끊긴 것이다.
    Closed(Option<Close>),
}

impl fmt::Debug for LinkRead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Frame(frame) => f.debug_tuple("Frame").field(frame).finish(),
            Self::Closed(close) => f.debug_tuple("Closed").field(close).finish(),
        }
    }
}

/// 주소 하나로 통로를 연다.
///
/// ★시한은 부르는 쪽(감독 태스크)이 건다★ — 구현체가 자기 시한을 또 걸면 정책 값이 두 곳에 산다.
///
/// ★반환 future 가 취소될 수 있다★ — 감독이 그 사이 `close()` 를 들으면 그대로 버린다. 구현체는 통로를
/// **future 안에서** 만들어야 하고, 밖에 등록해 두면 아무도 안 쓰는 통로가 남는다.
pub trait Dialer: Send + Sync + 'static {
    fn dial(
        &self,
        addr: &Address,
    ) -> BoxFuture<'_, Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), LinkError>>;
}

/// 통로의 나가는 쪽. ★감독 태스크 하나만 이것을 쥔다(단일 writer)★.
pub trait LinkTx: Send + 'static {
    /// ★반환 future 가 취소되면 부르는 쪽이 통로를 통째로 버린다★ — 절반 나간 프레임 뒤에 더 쓰지 않는다.
    fn send(&mut self, frame: Frame) -> BoxFuture<'_, Result<(), LinkError>>;

    /// 살아있나 물어본다. ★"언제" 는 이 crate 가 정하고 "어떻게" 만 구현체가 안다★ — 그래서 WS 처럼
    /// 전송 계층이 자체 Ping 을 가진 경우 프레임을 새로 짜지 않아도 된다.
    fn ping(&mut self) -> BoxFuture<'_, Result<(), LinkError>>;

    /// 코드와 문구를 실어 닫는다. 실패해도 알릴 데가 없으므로 반환값이 없다.
    fn close(&mut self, close: Close) -> BoxFuture<'_, ()>;
}

/// 통로의 들어오는 쪽.
pub trait LinkRx: Send + 'static {
    /// keepalive 응답은 [`Frame::Keepalive`] 로 올라온다.
    fn recv(&mut self) -> BoxFuture<'_, Result<LinkRead, LinkError>>;
}

/// 불투명 핸드셰이크. ★왕복 횟수를 crate 가 세지 않는다★ — [`HandshakeStep::Send`]/[`HandshakeStep::Await`]
/// 를 몇 번 돌려주든 받는다.
///
/// ★방향 대칭★ — 받는 쪽(데몬)은 `start()` 가 [`HandshakeStep::Await`] 를 주고 `on_frame` 에서 판정한다.
/// 그래서 버전·능력 협상 어휘는 **구현체 안에만** 있고 crate 는 그것을 모른다(ADR-0178).
pub trait Handshake: Send + Sync + 'static {
    fn start(&self) -> HandshakeStep;
    fn on_frame(&self, frame: &Frame) -> HandshakeStep;
}

/// 핸드셰이크의 다음 걸음.
///
/// ★[`HandshakeStep::Send`] 는 "보내고 **상대의 다음 프레임을 기다린다**" 는 뜻이다★ — trait 에 프레임
/// 없이 다음 걸음을 묻는 입구가 없으므로(`start`/`on_frame` 둘뿐) 보낸 뒤 갈 곳은 대기밖에 없다. 그래서
/// **연속 두 프레임을 보내는 핸드셰이크는 이 모양으로 표현되지 않는다**(TRD §3-5 가 든 예 두 걸음은
/// 표현된다: `start() = Send(auth)` → 상대 Hello → `on_frame = Done`).
///
/// ★[`Frame::Keepalive`] 는 `on_frame` 에 전달되지 않는다★ — 핸드셰이크 중 전송 계층 keepalive 가
/// 섞여 들어오면 판정이 흔들린다.
pub enum HandshakeStep {
    /// 보내고 상대의 다음 프레임을 기다린다.
    Send(Frame),
    /// 아무것도 안 보내고 상대의 다음 프레임을 기다린다.
    Await,
    /// 운영 단계로.
    Done,
    /// ★붙었는데 거절★ — `last` 를 보내고 닫는다. 재시도해도 결과가 같으므로 **재연결 예산을 쓰지
    /// 않는다**(ADR-0180 결정 5).
    Reject { last: Option<Frame>, reason: String },
}

impl fmt::Debug for HandshakeStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Send(frame) => f.debug_tuple("Send").field(frame).finish(),
            Self::Await => f.write_str("Await"),
            Self::Done => f.write_str("Done"),
            Self::Reject { last, reason } => f
                .debug_struct("Reject")
                .field("last", last)
                .field("reason", reason)
                .finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::CloseCode;

    #[test]
    fn address_is_opaque() {
        let a = Address::new("ws://127.0.0.1:1/path?x=1");
        assert_eq!(a.as_str(), "ws://127.0.0.1:1/path?x=1");
        assert_eq!(a.to_string(), "ws://127.0.0.1:1/path?x=1");
    }

    #[test]
    fn link_error_displays_its_message() {
        let e = LinkError::new("refused");
        assert_eq!(e.to_string(), "refused");
    }

    #[test]
    fn a_close_can_carry_a_code_the_reader_can_branch_on() {
        let spoken = LinkRead::Closed(Some(Close::new(
            CloseCode::HANDSHAKE_REJECTED,
            "version 3 != 4",
        )));
        let silent = LinkRead::Closed(None);
        match (&spoken, &silent) {
            (LinkRead::Closed(Some(c)), LinkRead::Closed(None)) => {
                assert_eq!(c.code, CloseCode::HANDSHAKE_REJECTED);
                assert!(c.reason.contains("version"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn handshake_step_debug_names_each_arm() {
        assert_eq!(format!("{:?}", HandshakeStep::Await), "Await");
        assert_eq!(format!("{:?}", HandshakeStep::Done), "Done");
        assert!(format!("{:?}", HandshakeStep::Send(Frame::Keepalive)).starts_with("Send"));
        assert!(format!(
            "{:?}",
            HandshakeStep::Reject {
                last: None,
                reason: "bad version".into()
            }
        )
        .contains("bad version"));
    }
}
