# 모듈 4 — pty/platform/windows.rs 브리핑 (담당: dco23, Opus)

발신: ed12 (매니저)
근거: `docs/backend-lld-stage1.md` §13 (Windows JobObject). + 방금 통과한 spike 코드.

## 왜 이걸 먼저 하나

session.rs(모듈 2)의 `PtySession` 이 `#[cfg(windows)] job_handle: JobObjectHandle` 필드를 가진다.
JobObjectHandle 타입이 먼저 있어야 session.rs가 빌드된다. spike에서 이미 동작 검증했으니 그 코드를 정식화한다.

## 선행: Cargo.toml 수정 (spike에서 발견)

windows feature 목록에 **`Win32_Security` 추가** (이거 없으면 빌드 실패). 최종:
```toml
[target.'cfg(windows)'.dependencies]
windows = { version = "0.58", features = [
    "Win32_System_JobObjects",
    "Win32_Foundation",
    "Win32_System_Threading",
    "Win32_Security",
] }
```
(tauri 버전 핀 `=2.4` 여부는 ed12가 Phase 2 전 별도 결정한다. 지금 건드리지 말 것.)

## 목표

`src-tauri/src/pty/platform/windows.rs` + `pty/platform/mod.rs` 작성.

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

비-Windows 빌드를 위해 `platform/mod.rs` 에 cfg 분기 (지금은 windows만 구현, 다른 OS는 추후). session.rs가 `use crate::pty::platform::JobObjectHandle` 로 쓸 수 있게 re-export.

## 규칙

- 모든 `unsafe` 블록 위에 **왜 안전한지/무슨 Win32 호출인지 한국어 주석**.
- 이 파일은 platform 전용이라 windows crate import OK. 단 **tauri import는 여전히 0개**.
- spike에서 검증한 호출 순서/플래그 그대로. 임의 변경 금지.
- `cargo fmt` 통과.

## 보고

- `cargo build` (windows) 통과 확인 후: `orch 12 "⟁dco23 windows.rs 완료 — JobObjectHandle, cargo build OK"`
- spike.rs(examples)는 windows.rs로 코드 옮긴 뒤 삭제하지 말고 그대로 둬라 — 회귀 검증용. (ed12가 나중 정리)

막히면 30분 내 중간보고.
