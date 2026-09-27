//! 사용량 한도 관측의 벤더 중립 어휘 — 백엔드가 만들고 데몬이 받는다([`crate::types::StatusSink::usage_observed`]).
//!
//! ★벤더 이름·정책(쿨타임·시한)을 여기 두지 않는다★ — 그것은 각 벤더의 backend 모듈이 진다(ADR-0004
//!   「백엔드 확장」). 이 모듈은 벤더를 불투명 낱말 하나로만 안다.
//! 생산자 쪽 공용 도구도 여기 산다 — 칸 값 정규화([`used_pct_from_fraction`] 등) · 상류 문자열 도구
//! ([`display_text`]·[`has_word`]) · 비정상 결과의 근거와 상류 원문([`UsageDetail`]·[`UpstreamText`]) · 기본 계정이
//! 아닌 에이전트의 줍기를 막는 디코더 감싸개([`UsageGate`]).
//! 능동 조회의 seam([`UsageProbe`]·[`ProbeSpawner`])과 그 실물([`OsProbeSpawner`]·[`ScratchDir`])도 여기다 —
//! 벤더 조회기는 각 backend 가 이 seam 위에 구현한다.
// ADR-0004

mod detail;
mod gate;
mod normalize;
mod probe;
mod process;
mod scratch;
#[cfg(test)]
pub(crate) mod testing;
mod text;

pub use detail::{UpstreamText, UsageDetail};
pub use gate::{env_overrides_daemon, UsageGate};
pub use normalize::{
    resets_at_from_epoch_secs, resets_at_from_json, used_pct_from_fraction, used_pct_from_percent,
};
pub use probe::{
    finish_after_answer, ExitInfo, ProbeChild, ProbeCommand, ProbeEnv, ProbeError, ProbeFailure,
    ProbeSpawner, UsagePolicy, UsageProbe,
};
pub use process::{OsProbeSpawner, KILL_WAIT};
pub use scratch::{sweep_stale_scratch, ScratchDir};
pub use text::{display_text, has_word};

/// 벤더를 가리키는 불투명 낱말 — 받는 쪽은 해석하지 않고 칸 키로만 쓴다.
///
/// ★정본 철자는 벤더가 선언한 `&'static str` 하나다★ — 들어오는 낱말(wire 소문자 · 버스 대문자 시작)은
///   그 정본으로 접은 뒤 키로 삼는다. 접지 않은 낱말로 키를 만들면 두 입구가 다른 칸을 친다.
/// ★`&'static str` 만 받는 것은 의도다★ — 디스크·wire 에서 온 문자열로는 (leak 하지 않는 한) 키를 만들
///   수 없어 정본을 거치게 된다. `String` 으로 넓히지 말 것. 단 리터럴은 막지 못한다 — `new` 를 부르는
///   것은 벤더 선언과 시험 대역뿐이어야 한다(데몬이 벤더 리터럴로 부르면 ADR-0004 위반).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UsageVendorKey(&'static str);

impl UsageVendorKey {
    pub const fn new(word: &'static str) -> Self {
        Self(word)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

/// 같은 벤더 안의 계정을 가르는 낱말. ★지금은 늘 [`Default`] 값(`"default"` — 데몬 env 의 기본 로그인)
/// 이다★ — 칸을 미리 둔 이유는 여러 데몬에 붙는 클라이언트가 값의 출처를 가를 수 있게 하려는 것이다.
/// 그 낱말은 wire·디스크로 나간다.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UsageAccountKey(String);

impl UsageAccountKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for UsageAccountKey {
    fn default() -> Self {
        Self("default".to_owned())
    }
}

/// 사용량 상태 한 칸의 키 — `(벤더, 계정)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UsageKey {
    pub vendor: UsageVendorKey,
    pub account: UsageAccountKey,
}

/// 한도 창 하나의 관측.
///
/// - `used_pct` = **쓴** 양의 백분율(남은 양이 아니다). 생산자가 0–100 의 유한수로 정규화해 싣는다.
/// - `resets_at` = 그 창이 리셋되는 시각, epoch 초.
/// - 두 칸 모두 `None` = 모른다이지 0 이 아니다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowObs {
    pub used_pct: Option<f64>,
    pub resets_at: Option<i64>,
}

/// 모델별 주간 창 하나. `label` = 벤더가 준 표시용 이름(키로 쓰지 않는다).
#[derive(Debug, Clone, PartialEq)]
pub struct ScopedWindowObs {
    pub label: String,
    pub window: WindowObs,
}

/// 관측이 어디서 왔나 — [`UsageObservation`] 의 빠진 칸의 뜻을 가른다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageSource {
    /// 대화 스트림에서 주웠다 — 부분 관측이다.
    Passive,
    /// 조회로 받았다 — 그 시점의 전체 그림이다.
    Active,
}

/// 한 벤더의 사용량 관측 한 건.
///
/// ★빠진 칸(`None`)의 뜻은 `source` 가 정한다★:
///   - `Passive` = 안 실렸다 — 받는 쪽은 들고 있던 값을 그대로 둔다(`model_scoped`·`plan` 도 같다).
///   - `Active` = 그 칸이 없다 — 받는 쪽은 들고 있던 값을 비운다(`model_scoped: None` = 모델별 창 없음).
/// `model_scoped: Some` 은 어느 쪽이든 목록 전량 교체다 — 항목 단위로 합치지 않는다.
/// ★관측 시각을 싣지 않는다★ — 받는 데몬이 찍는다. 생산자(디코더)는 시계를 갖지 않는다.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageObservation {
    pub vendor: UsageVendorKey,
    pub five_hour: Option<WindowObs>,
    pub weekly: Option<WindowObs>,
    pub model_scoped: Option<Vec<ScopedWindowObs>>,
    pub plan: Option<String>,
    pub source: UsageSource,
    /// `Some` = 조회는 성공했는데 상류가 「이 계정엔 한도 정보가 없다」고 답했다 — 값은 그 근거다(Claude
    /// `rate_limits_available: false` — API 키·Bedrock·Vertex·profile 권한 없는 토큰·로그아웃).
    /// `Some` 이면 창 칸(`five_hour`·`weekly`·`model_scoped`)은 전부 `None` 이고 `source` 는
    /// `Active` 다 — 만드는 쪽이 지키는 약속이다. 받는 쪽은 창 값을 전부 비우고 「정보 없음」
    /// 상태로 접는다 — 실패가 아니다(실패는 들고 있던 값을 그대로 둔다).
    /// `Passive` 관측은 언제나 `None` 이다.
    pub limits_unavailable: Option<UsageDetail>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AgentId, AgentInfo, AgentStatus, StatusSink};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn observation() -> UsageObservation {
        UsageObservation {
            vendor: UsageVendorKey::new("test-vendor"),
            five_hour: Some(WindowObs {
                used_pct: Some(40.0),
                resets_at: Some(1_900_000_000),
            }),
            weekly: None,
            model_scoped: None,
            plan: None,
            source: UsageSource::Passive,
            limits_unavailable: None,
        }
    }

    /// 필수 훅만 구현한 sink — 새 훅을 안 쓰는 기존 sink 전부가 이 모양이다.
    struct RequiredOnly {
        calls: AtomicUsize,
    }

    impl StatusSink for RequiredOnly {
        fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {
            self.calls.fetch_add(1, Ordering::SeqCst);
        }
        fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {
            self.calls.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn usage_observed_defaults_to_a_no_op() {
        let sink = RequiredOnly {
            calls: AtomicUsize::new(0),
        };
        sink.usage_observed(observation());
        assert_eq!(
            sink.calls.load(Ordering::SeqCst),
            0,
            "기본 구현이 다른 훅으로 새면 안 된다"
        );
    }

    /// 계정 낱말은 wire(`account_key`)와 디스크로 나간다 — 바뀌면 저장된 칸과 어긋난다.
    #[test]
    fn the_default_account_word_is_default() {
        assert_eq!(UsageAccountKey::default().as_str(), "default");
    }
}
