//! Test doubles of the frame port — an isolation harness where the agent system layer runs without a socket (ADR-0129).
//!
//! Their reason to exist is to keep agent layer tests from even calling the network layer's real objects (`ConnFrameSink` and
//! `ConnRegistry` of `engram_dashboard_net::ws`) by name — with slice 1 the crate was actually
//! split so those names are in another lib (`ConnFrameSink` is `pub(crate)` there so it cannot even be called).
//! The doubles depend only on the port contract (`frame_port`).
//!
//! ★Network layer policy is not imitated★: things like the close signal (close_signal) on queue saturation or fan-out target management
//!   are network lifecycle management, and their real contract is guarded by the network crate's own tests. If reproduced
//!   here the test would verify the double instead of the real thing.
// ADR-0129

use std::sync::Mutex;

use futures_util::future::BoxFuture;
use tokio::sync::mpsc;

use engram_dashboard_net::frame_port::{Frame, FrameError, FrameFanout, FrameSink};

/// The channel capacity is itself the saturation condition, so a small cap can drive the upper layer's drop handling path.
pub(crate) struct FakeFrameSink {
    tx: mpsc::Sender<Frame>,
}

impl FakeFrameSink {
    pub(crate) fn new(tx: mpsc::Sender<Frame>) -> Self {
        Self { tx }
    }
}

impl FrameSink for FakeFrameSink {
    fn try_send(&self, frame: Frame) -> Result<(), FrameError> {
        self.tx.try_send(frame).map_err(|_| FrameError)
    }

    fn send(&self, frame: Frame) -> BoxFuture<'_, Result<(), FrameError>> {
        Box::pin(async move { self.tx.send(frame).await.map_err(|_| FrameError) })
    }
}

/// The all-connections fan-out double. It **records calls instead of imitating delivery** — what the upper layer passed to the fan-out and how many times is all
/// that is observable at this layer, and how that text is copied per connection is the contract on the other side of the port
/// (`engram_dashboard_net::ws::ConnRegistry`) and its tests guard it.
#[derive(Default)]
pub(crate) struct RecordingFanout {
    texts: Mutex<Vec<String>>,
}

impl RecordingFanout {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn texts(&self) -> Vec<String> {
        self.texts
            .lock()
            .expect("recording fanout poisoned")
            .clone()
    }

    pub(crate) fn sole_text(&self) -> String {
        let texts = self.texts();
        assert_eq!(
            texts.len(),
            1,
            "팬아웃은 사실 1건당 정확히 1회여야(중복·누락 송신 회귀): {texts:?}"
        );
        texts.into_iter().next().expect("길이 1 을 위에서 확정")
    }
}

impl FrameFanout for RecordingFanout {
    fn broadcast_text(&self, text: String) {
        self.texts
            .lock()
            .expect("recording fanout poisoned")
            .push(text);
    }
}
