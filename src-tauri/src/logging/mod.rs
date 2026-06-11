use std::sync::OnceLock;

use regex::Regex;
use tracing_subscriber::{layer::SubscriberExt, reload, util::SubscriberInitExt, EnvFilter};

// reload::Handle 타입 별칭 — OnceLock에 보관해 런타임 레벨 재설정에 사용
type FilterHandle = reload::Handle<EnvFilter, tracing_subscriber::Registry>;

static RELOAD_HANDLE: OnceLock<FilterHandle> = OnceLock::new();

// T-1: debug 로그 시 민감값 유출 방지용 정규식 (OnceLock — 1회 컴파일)
static BEARER_RE: OnceLock<Regex> = OnceLock::new();
static KEY_RE: OnceLock<Regex> = OnceLock::new();

/// debug 로그 출력 전 민감 값(API키·Bearer 토큰)을 `***`로 치환 (T-1).
/// 기본 로그 레벨(warn)에서는 PTY 텍스트가 찍히지 않으나,
/// debug/trace 활성화 시 실수로 키가 노출되는 것을 방지한다.
///
/// 커버 범위:
/// - Bearer 토큰 (Authorization 헤더)
/// - sk-... Anthropic/OpenAI 계열 (sk-ant-, sk-proj-, 구형 sk-)
/// - AKIA... AWS access key ID
/// - ghp_/gho_/ghu_/ghs_/ghr_ + github_pat_ GitHub 토큰
/// - AIza... Google API key
///
/// ※ AWS Secret Access Key(40자 base64)는 패턴 식별불가로 미포함.
/// ※ generic api_key=/token= 형태는 오탐 리스크로 미포함.
pub fn mask_secrets(s: &str) -> String {
    // Bearer Authorization 헤더 값 전체를 "Bearer ***"로 치환
    let bearer =
        BEARER_RE.get_or_init(|| Regex::new(r"Bearer\s+\S{10,}").expect("bearer regex compile"));
    // sk- 계열(Anthropic/OpenAI), AWS, GitHub, Google API key
    let keys = KEY_RE.get_or_init(|| {
        Regex::new(
            r"(?:sk-(?:proj-)?[A-Za-z0-9_\-]{20,}|AKIA[A-Z0-9]{16}|(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36}|github_pat_[A-Za-z0-9_]{20,}|AIza[0-9A-Za-z_\-]{35})",
        )
        .expect("key regex compile")
    });
    let step1 = bearer.replace_all(s, "Bearer ***");
    keys.replace_all(step1.as_ref(), "***").into_owned()
}

/// tracing-subscriber 전역 초기화. 앱 부팅 시 1회만 호출 (멱등 — 중복 호출 no-op).
/// 기본 레벨: RUST_LOG 환경변수 우선, 없으면 "warn" (릴리스 기본 OFF — 평상시 거의 무출력).
pub fn init_logging() {
    // 이미 초기화됐으면 no-op
    if RELOAD_HANDLE.get().is_some() {
        return;
    }

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));

    let (filter_layer, handle) = reload::Layer::new(filter);

    // try_init: 다른 subscriber가 이미 설정된 경우(테스트 등) 무시
    let result = tracing_subscriber::registry()
        .with(filter_layer)
        .with(tracing_subscriber::fmt::layer())
        .try_init();

    if result.is_ok() {
        // subscriber 등록 성공 시에만 handle 보관 (실패 시 reload 불필요)
        let _ = RELOAD_HANDLE.set(handle);
    }
}

/// 런타임 로그 레벨 변경. 유효값: "trace"|"debug"|"info"|"warn"|"error"|"off".
pub fn set_log_level(level: &str) -> Result<(), String> {
    let handle = RELOAD_HANDLE
        .get()
        .ok_or_else(|| "logging not initialized".to_string())?;

    let filter =
        EnvFilter::try_new(level).map_err(|e| format!("invalid log level \"{level}\": {e}"))?;

    handle
        .reload(filter)
        .map_err(|e| format!("reload failed: {e}"))
}
