//! 프레임 포트의 테스트 더블 — 에이전트 시스템 행이 소켓 없이 도는 격리 하네스(ADR-0129).
//!
//! 에이전트 행 테스트가 네트워크 행 실물(`engram_dashboard_net::ws` 의 `ConnFrameSink`·
//! `ConnRegistry`)을 이름으로도 부르지 않게 하는 것이 존재 이유다 — 슬라이스 1 로 crate 가 실제로
//! 갈려 그 이름들은 다른 lib 에 있다(`ConnFrameSink` 는 그쪽 `pub(crate)` 라 부를 수조차 없다).
//! 더블은 포트 계약(`frame_port`)에만 의존한다.
//!
//! ★네트워크 행 정책은 흉내내지 않는다★: 큐 포화 시 종료 신호(close_signal)나 팬아웃 대상 관리
//!   같은 것은 네트워크 살림이고 그 실물 계약은 네트워크 crate 의 자기 테스트가 지킨다. 여기서
//!   재현하면 테스트가 실물이 아니라 더블을 검증하게 된다.
// ADR-0129

use std::sync::Mutex;

use futures_util::future::BoxFuture;
use tokio::sync::mpsc;

use engram_dashboard_net::frame_port::{Frame, FrameError, FrameFanout, FrameSink};

/// 채널 용량이 곧 포화 조건이라, cap 을 작게 잡으면 위층의 drop 처리 경로를 태울 수 있다.
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

/// 전-연결 팬아웃 더블. **배달을 흉내내지 않고 호출을 기록**한다 — 위층이 팬아웃에 무엇을 몇 번
/// 넘겼는지가 이 층에서 관측 가능한 전부이고, 그 text 가 연결마다 어떻게 복제되는지는 포트 건너편
/// (`engram_dashboard_net::ws::ConnRegistry`)의 계약이라 그쪽 테스트가 지킨다.
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
