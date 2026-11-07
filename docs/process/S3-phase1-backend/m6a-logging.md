# Module 6a — logging/mod.rs (core) briefing (owner: dcs24, Sonnet)

From: ed12 (manager)
Source: `docs/backend-lld-stage1.md` §14 (logging).

## Scope (core only this time)

Only the **initialization + level toggle core** of `src-tauri/src/logging/mod.rs`. 
The headless test (LogSink/examples) is done separately after Phase 1 (the whole pty module) is complete — do not touch it now.
This module is independent of pty/, so it can proceed in parallel.

## Goal

```rust
/// tracing-subscriber 전역 초기화. 앱 부팅 시 1회 호출.
/// 기본 레벨은 환경변수 RUST_LOG 우선, 없으면 "warn" (= 평상시 거의 무출력 = 기본 OFF).
pub fn init_logging();

/// 런타임 로그 레벨 토글. "trace"|"debug"|"info"|"warn"|"error"|"off".
/// EnvFilter reload handle 로 재설정한다.
pub fn set_log_level(level: &str) -> Result<(), String>;
```

Key points:
- `tracing_subscriber::fmt()` + `EnvFilter`. Default `EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"))`.
- For `set_log_level`, use a **reload layer** (`tracing_subscriber::reload`) — keep the handle in a static (OnceLock etc.) to reconfigure at runtime.
- init is idempotent (safe to call repeatedly). If already initialized, ignore it or no-op instead of returning an error.

## Rules

- Zero tauri imports in this file (the Tauri command wrapper goes separately in Phase 2 commands/). The logging core is pure tracing.
- State in a comment that the default level is OFF (warn) — the "release default OFF" requirement.
- Korean comments on functions/major branches. cargo fmt passes.

## Wiring and report

- Add `mod logging;` to `lib.rs` (minimal connection).
- After confirming `cargo build` passes: `orch 12 "⟁dcs24 logging core 완료 — init_logging/set_log_level, build OK"`

If stuck, send an interim report within 30 minutes.
