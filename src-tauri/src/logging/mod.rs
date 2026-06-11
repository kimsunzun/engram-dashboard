use std::sync::OnceLock;
use tracing_subscriber::{layer::SubscriberExt, reload, util::SubscriberInitExt, EnvFilter};

// reload::Handle 타입 별칭 — OnceLock에 보관해 런타임 레벨 재설정에 사용
type FilterHandle = reload::Handle<EnvFilter, tracing_subscriber::Registry>;

static RELOAD_HANDLE: OnceLock<FilterHandle> = OnceLock::new();

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
