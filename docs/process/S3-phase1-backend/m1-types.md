# Module 1 — pty/types.rs briefing (owner: dcs24, Sonnet)

From: ed12 (manager)
Source document: `docs/backend-lld-stage1.md` §3 (type definitions). Read the original §3 and follow it exactly.

## Goal

Write `src-tauri/src/pty/types.rs`. The types and traits shared by the whole PTY backend.

## Must include (LLD §3)

```rust
pub type AgentId = uuid::Uuid;
pub type SinkId  = uuid::Uuid;

// 프론트엔드가 discriminated union으로 받도록 internally-tagged 필수
#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "type")]
pub enum AgentStatus {
    Running,
    Exiting,
    Exited { code: Option<i32> },
    Failed { message: String },
    Killed,
}

// drain 내부 전달용 (raw 바이트)
pub struct PtyChunk { pub seq: u64, pub data: Vec<u8> }

// 프론트로 나가는 wire 포맷 (base64 인코딩 문자열)
#[derive(Clone, serde::Serialize)]
pub struct PtyEvent { pub agent_id: AgentId, pub seq: u64, pub data_b64: String }

#[derive(Clone, serde::Serialize)]
pub struct AgentInfo { pub id: AgentId, pub cwd: String, pub status: AgentStatus, pub cols: u16, pub rows: u16 }

// 추상화 trait — 이 파일에 tauri import 절대 금지 (headless 테스트 가능해야 함)
pub trait OutputSink: Send + Sync + 'static {
    fn send(&self, event: PtyEvent) -> Result<(), SinkError>;
    fn sink_id(&self) -> SinkId;
}
pub trait StatusSink: Send + Sync + 'static {
    fn status_changed(&self, id: AgentId, status: AgentStatus);
    fn agent_list_updated(&self, agents: Vec<AgentInfo>);
}

#[derive(thiserror::Error, Debug)]
pub enum SinkError { #[error("sink closed")] Closed, /* 필요시 추가 */ }
```

## ReplayBuffer (LLD §3 — ring buffer for late-attaching windows)

- Capacity cap **2MB**. When exceeded, drop from the front (ring).
- Provide `push(chunk: PtyChunk)` and `snapshot() -> Vec<PtyChunk>` (clone).
- Internal structure: `VecDeque<PtyChunk>` + a cumulative byte counter. Implementation is up to you, but the 2MB cap must be exact.
- If LLD §3 spells out the signatures, follow them exactly.

## Invariants (mandatory review items)

- **Zero `use tauri` / tauri types in this file.** (`grep "use tauri" src/pty/types.rs` → 0 lines)
- `AgentStatus` must have `#[serde(tag = "type")]`.
- Derive macros (Clone/Debug/Serialize) follow the LLD. Do not add too many.

## Code quality (10-year maintenance)

- Above each type, **a one-line Korean comment on what the type is for**. No comments on self-evident fields.
- `cargo fmt` passes. Blank lines between logical blocks.

## Build check and report

- Add `pub mod types;` to `pty/mod.rs` and make the minimal connection so that `mod pty;` is picked up in `lib.rs`.
- Confirm `cargo build` passes (dco23 already added the Cargo.toml dependencies — uuid/thiserror/serde are usable).
- Completion report: `orch 12 "⟁dcs24 types.rs 완료 — cargo build OK"` (attach an error excerpt on failure)

If stuck, send an interim report within 30 minutes. If a version/spec change is needed, do not change it on your own; report to ed12.
