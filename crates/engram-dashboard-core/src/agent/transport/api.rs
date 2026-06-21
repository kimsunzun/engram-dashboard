//! ApiTransport — 껍데기. 인터페이스만 만족, HTTP 스트림·이벤트 변환은 API 모델 붙는 날 채움.
//!
//! 설계 의도: `unimplemented!()` 패닉 없이 호출돼도 안전하게 `PtyError::Unsupported`를 반환한다.
//! manager 라우팅은 없음 — 이 transport는 구조(존재)만 확보하는 단계(stage 7).
//!
//! tauri import 0.

use std::sync::Arc;

use crate::agent::output_core::OutputCore;
use crate::agent::transport::AgentTransport;
use crate::agent::types::{
    ControlCaps, InputCaps, InputEvent, OutputCaps, PtyError, TransportCaps,
};

/// HTTP 스트림 API 백엔드용 transport 껍데기.
///
/// 모든 제어 동사는 `PtyError::Unsupported`를 반환한다.
/// start·shutdown은 자원이 없으므로 no-op.
pub struct ApiTransport;

impl ApiTransport {
    /// 새 인스턴스 생성. 현재 보유 자원 없음.
    pub fn new() -> Self {
        Self
    }
}

impl Default for ApiTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentTransport for ApiTransport {
    /// no-op — HTTP 스트림은 API 모델 붙는 날 채움.
    fn start(&self, _core: Arc<OutputCore>) {
        // 자원 없음. pump 스레드 미생성.
    }

    /// 미지원 — API transport는 raw 바이트 입력을 처리하지 않음.
    fn send_input(&self, _input: InputEvent) -> Result<(), PtyError> {
        Err(PtyError::Unsupported(
            "ApiTransport::send_input (껍데기)".into(),
        ))
    }

    /// 미지원 — HTTP 스트림에는 터미널 크기 개념 없음.
    fn resize(&self, _cols: u16, _rows: u16) -> Result<(), PtyError> {
        Err(PtyError::Unsupported(
            "ApiTransport::resize (껍데기)".into(),
        ))
    }

    /// 미지원 — HTTP 스트림 cancel은 API 모델 붙는 날 채움.
    fn interrupt(&self) -> Result<(), PtyError> {
        Err(PtyError::Unsupported(
            "ApiTransport::interrupt (껍데기)".into(),
        ))
    }

    /// no-op — 보유 자원 없음.
    fn shutdown(&self) {
        // 자원 없음. 정리할 것 없음.
    }

    /// 물리 채널 caps 전부 false — API 모델 미연결 상태에서 지원하는 기능 없음.
    /// session·model 은 backend 소관이라 여기서 만들지 않는다(TransportCaps 엔 없음).
    fn capabilities(&self) -> TransportCaps {
        TransportCaps {
            input: InputCaps {
                raw: false,
                message: false,
                attachment: false,
            },
            output: OutputCaps {
                terminal_bytes: false,
                markdown: false,
                tool_events: false,
                usage: false,
            },
            control: ControlCaps {
                resize: false,
                interrupt: false,
                cancel: false,
                graceful_shutdown: false,
            },
        }
    }
}
