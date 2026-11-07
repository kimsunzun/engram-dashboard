//! ApiTransport — a shell. It only satisfies the interface; HTTP stream · event conversion is filled in the day the API model is attached.
//!
//! Design intent: even if called, it returns `PtyError::Unsupported` safely without an `unimplemented!()` panic.
//! There is no manager routing.
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

    /// An HTTP stream has no concept of terminal size.
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
