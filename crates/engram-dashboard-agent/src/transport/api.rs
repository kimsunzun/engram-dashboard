//! ApiTransport — 껍데기. 인터페이스만 만족, HTTP 스트림·이벤트 변환은 API 모델 붙는 날 채움.
//!
//! 설계 의도: `unimplemented!()` 패닉 없이 호출돼도 안전하게 `PtyError::Unsupported`를 반환한다.
//! manager 라우팅은 없음.
//!
//! tauri import 0.

use std::sync::Arc;

use crate::output_core::OutputCore;
use crate::transport::AgentTransport;
use crate::types::{ControlCaps, InputCaps, InputEvent, OutputCaps, PtyError, TransportCaps};

pub struct ApiTransport;

impl ApiTransport {
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
    fn start(&self, _core: Arc<OutputCore>) {}

    fn send_input(&self, _input: InputEvent) -> Result<(), PtyError> {
        Err(PtyError::Unsupported(
            "ApiTransport::send_input (껍데기)".into(),
        ))
    }

    /// HTTP 스트림에는 터미널 크기 개념이 없다.
    fn resize(&self, _cols: u16, _rows: u16) -> Result<(), PtyError> {
        Err(PtyError::Unsupported(
            "ApiTransport::resize (껍데기)".into(),
        ))
    }

    fn interrupt(&self) -> Result<(), PtyError> {
        Err(PtyError::Unsupported(
            "ApiTransport::interrupt (껍데기)".into(),
        ))
    }

    fn shutdown(&self) {}

    fn capabilities(&self) -> TransportCaps {
        TransportCaps {
            input: InputCaps {
                raw: false,
                message: false,
                attachment: false,
            },
            output: OutputCaps {
                terminal_bytes: false,
                structured: false,
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
