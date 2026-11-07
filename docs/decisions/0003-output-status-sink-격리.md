# ADR-0003: OutputSink / StatusSink — isolating the core from Tauri

- 상태: 확정 (S1 design, S3/S5 implementation)
- 관련: CLAUDE.md 「코어 격리」 · `types.rs::{OutputSink,StatusSink}` · `lib.rs::{ChannelOutputSink,TauriStatusSink}`

## 맥락
If the core (`pty/`) is tied directly to the Tauri AppHandle, it cannot be tested without a screen, and the transport path (PTY/HTTP/WebSocket) cannot be swapped.

## 결정
Output and status flow **only through the traits** `OutputSink`/`StatusSink`. The core knows nothing about Tauri or the transport method. The Tauri implementations (ChannelOutputSink/TauriStatusSink) are injected from `lib.rs`.

## 거부한 대안
- **The core holds the AppHandle directly** — headless testing is impossible, and the transport path is fixed to Tauri. Moving to a daemon or mobile means rewriting the core.

## 근거
Swapping only the sink absorbs a new transport path. `examples/headless·smoke` verify this by running the core standalone with a Noop/test sink.

## 영향 / 불변식
- **Zero tauri imports under `pty/` (currently `engram-dashboard-core`)** — a gate: `rg "use tauri"` → 0 lines.
- The manager is injected with `Arc<dyn StatusSink>`, not the AppHandle.
