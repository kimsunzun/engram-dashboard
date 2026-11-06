# Module 4 — pty/platform/windows.rs briefing (owner: dco23, Opus)

From: ed12 (manager)
Source: `docs/backend-lld-stage1.md` §13 (Windows JobObject). + the spike code that just passed.

## Why this goes first

`PtySession` in session.rs (module 2) has a `#[cfg(windows)] job_handle: JobObjectHandle` field.
The JobObjectHandle type must exist first for session.rs to build. The spike already verified it works, so formalize that code.

## Prerequisite: edit Cargo.toml (found in the spike)

Add **`Win32_Security`** to the windows feature list (the build fails without it). Final:
```toml
[target.'cfg(windows)'.dependencies]
windows = { version = "0.58", features = [
    "Win32_System_JobObjects",
    "Win32_Foundation",
    "Win32_System_Threading",
    "Win32_Security",
] }
```
(Whether to pin the tauri version to `=2.4` is a separate decision ed12 makes before Phase 2. Do not touch it now.)

## Goal

Write `src-tauri/src/pty/platform/windows.rs` + `pty/platform/mod.rs`.

```rust
/// Windows Job Object 래퍼.
/// 자식 PTY 프로세스를 Job에 묶어, 우리가 명시적으로 죽이거나(TerminateJobObject)
/// Tauri 프로세스가 크래시될 때(KILL_ON_JOB_CLOSE) 손자 프로세스까지 함께 정리한다.
pub struct JobObjectHandle { /* HANDLE */ }

impl JobObjectHandle {
    /// Job 생성 + KILL_ON_JOB_CLOSE 설정.
    pub fn new() -> std::io::Result<Self>;

    /// child process를 이 Job에 편입. (process id 또는 raw handle 받아 OpenProcess→Assign)
    pub fn assign(&self, process_id: u32) -> std::io::Result<()>;

    /// Job 내 전 프로세스 강제 종료.
    pub fn terminate(&self, exit_code: u32) -> std::io::Result<()>;
}

impl Drop for JobObjectHandle {
    // CloseHandle. KILL_ON_JOB_CLOSE 덕에 핸들이 닫히면 잔여 프로세스도 정리됨.
}
```

For non-Windows builds, add a cfg branch in `platform/mod.rs` (only windows is implemented for now; other OSes later). Re-export so session.rs can use `use crate::pty::platform::JobObjectHandle`.

## Rules

- Above every `unsafe` block, a **Korean comment on why it is safe / which Win32 call it is**.
- This file is platform-only, so importing the windows crate is OK. But **still zero tauri imports**.
- Keep the call order/flags verified in the spike. No arbitrary changes.
- `cargo fmt` passes.

## Report

- After confirming `cargo build` (windows) passes: `orch 12 "⟁dco23 windows.rs 완료 — JobObjectHandle, cargo build OK"`
- After moving the code from spike.rs (examples) to windows.rs, do not delete spike.rs — leave it as is, for regression checks. (ed12 cleans up later)

If stuck, send an interim report within 30 minutes.
