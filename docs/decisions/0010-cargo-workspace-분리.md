# ADR-0010: Split into a 3-crate Cargo workspace

- 상태: 확정 (S12 phase 1)
- 관련: CLAUDE.md 백엔드 모듈 맵·빌드 명령 · root `Cargo.toml`

## 맥락
To daemonize (S12), the core logic must not be tied to `src-tauri` (= the tauri bin). The daemon server bin must depend only on the core, without tauri.

## 결정
Switch to a Cargo **workspace**. Three members:
- `crates/engram-dashboard-protocol` — the wire contract (AgentCommand/AgentEvent/OutputChunk/codec, ts-rs bindings).
- `crates/engram-dashboard-core` — the core (pty/persistence/logging) + examples. Moved from `src-tauri/src/` with git mv (history preserved).
- `src-tauri` — the tauri app. Re-imports the core as `engram_dashboard_core::{...}`.

## 거부한 대안
- **Keep a single src-tauri crate** — the daemon bin would drag in the tauri dependency. Violates core isolation (ADR-0003).

## 근거
The daemon depends only on core+protocol (builds without tauri). Internal `crate::` paths are unchanged (top-level modules are kept), so the move cost is small.

## 영향 / 불변식
- Build and test from the workspace root with `cargo test -p <crate>` / `cargo build`.
- Isolation gate path: `rg "use tauri" crates/engram-dashboard-core/src/` → 0 lines.
