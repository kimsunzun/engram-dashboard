# P3c1 slice contract (fixed by orchestrator — both slices code against exactly this)

Slice A (boot side) creates these; Slice B (answer path) consumes them unchanged.
If Slice A must deviate, it reports the deviation explicitly in its return (do not silently reshape).

## `src-tauri/src/state/restore.rs` (new, created by A)

```rust
/// Wire-visible crash-copy status (TRD §6-7 `crash_copy`). serde snake_case: "none" | "awaiting" | "answered".
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CrashCopyStatus { None, Awaiting, Answered }

/// What boot step ⑥ hands the service when there is an unanswered crash copy.
#[derive(Debug, Clone)]
pub struct CrashCopy {
    /// Raw text the accept path restores from — the crash copy bytes read at boot, or (guard ⅱ — copy could
    /// not be written) the in-memory `state.json` original text (TRD §6-5 ⑥ · L2).
    pub text: String,
    /// `codec::crash_copy_hash(&text)`.
    pub hash: String,
    /// Decoded `text`.
    pub file: StateFile,
    /// `false` when the run is guarded (no saver) — the answer cannot reach disk (`durable:false`).
    pub durable: bool,
}

/// Snapshot for `restore.status`. The last three are `Some` only while `Awaiting`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreStatusView {
    pub crash_copy: CrashCopyStatus,
    pub saved_at_ms: Option<u64>,
    pub windows: Option<u32>,   // number of WindowEntry in the copy
    pub tabs: Option<u32>,      // total tabs across all windows in the copy
}

pub enum AnswerConflict {
    /// Not `Awaiting` (no copy, or already answered). Carries the current status.
    NotAwaiting(CrashCopyStatus),
    /// Another answer is in flight (TRD §6-7 "다른 답이 처리 중이면 CONFLICT").
    InFlight,
}

/// Proof that this caller holds the single in-flight answer slot. Not Clone.
pub struct AnswerTicket { /* private */ }
impl AnswerTicket { pub fn copy(&self) -> &CrashCopy; }

pub enum AnswerEnd {
    /// Accept committed or reject — status becomes `Answered` (notifier fires).
    Answered,
    /// Accept failed before commit (prepare / window creation) — clears in-flight, stays `Awaiting`.
    RolledBack,
}

/// Port: fired after a status transition (boot ⑥ set, `Answered`). Called with NO lock held.
pub trait RestoreNotifier: Send + Sync { fn changed(&self, status: CrashCopyStatus); }

pub struct RestoreService { /* leaf Mutex — never call out while holding it */ }
impl RestoreService {
    pub fn new() -> Self;                                   // status None, no notifier
    pub fn set_notifier(&self, notifier: Arc<dyn RestoreNotifier>);
    /// Boot step ⑥ only (I5). `None` = no unanswered copy.
    pub fn set_boot(&self, copy: Option<CrashCopy>);
    pub fn status(&self) -> RestoreStatusView;
    pub fn begin_answer(&self) -> Result<AnswerTicket, AnswerConflict>;
    pub fn finish_answer(&self, ticket: AnswerTicket, end: AnswerEnd);
}
```

- Dropping an `AnswerTicket` without `finish_answer` must behave as `RolledBack` (panic safety — never leave the slot stuck).
- Ownership: `Arc<RestoreService>` is created in `lib.rs` builder, `.manage()`d (ADR-0102 — exists before first invoke), and passed into the boot plugin (`Boot { restore: Arc<RestoreService>, .. }`).
- Tauri notifier impl (A wires it in the user `setup` or boot plugin, whichever has the `AppHandle`): emits event `restore:changed` with payload = `CrashCopyStatus` to the `main` window. At boot ⑥ there are no windows yet; that is fine (first `restore_status` pull sees the settled value — I5).

## `StateSession` (boot_plugin.rs) — added by A

```rust
pub enum ResolveResult { Durable, NotDurable }
impl StateSession {
    /// Sends `Resolve { hash }` to the saver and waits up to `saver::REPLY_DEADLINE`.
    /// Takes a clone of the saver handle under the leaf cell lock, waits with NO lock held.
    /// No saver (guard / failed to start / already shut down) → NotDurable immediately.
    /// Durable only for `RequestOutcome::Done(SaveOutcome::Written)`.
    pub fn resolve_crash_copy(&self, hash: String) -> ResolveResult;
}
```
`SaverHandle` must be `Clone` (it already is). Remove the `#[allow(dead_code)]` + TODO on `SaverHandle::resolve` once this calls it.

---

# Slice B contract (B1 creates, B2 consumes unchanged — deviations must be reported)

## `src-tauri/src/state/restore.rs` — coordinator (added by B1, next to `RestoreService`)

```rust
/// `restore.answer` result (TRD §6-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnswerReply { pub restored_windows: u32, pub durable: bool }

#[derive(Debug)]
pub enum AnswerError {
    /// Not awaiting / already answered / another answer in flight → bus `CONFLICT`.
    Conflict(AnswerConflict),
    /// Accept failed before commit (prepare or window creation) — nothing changed, still `Awaiting` → bus `INTERNAL`.
    Internal(String),
}

/// The single runtime restore-accept/reject path (TRD §6-7 ①–⑤). Created in `lib.rs`, `.manage()`d as
/// `Arc<RestoreCoordinator>`. Holds `Arc<RestoreService>`, layout state, tree attrs, `Arc<StateSession>`,
/// the popout label source, and a window port (Tauri impl in production, fake in tests).
pub struct RestoreCoordinator { /* private */ }
impl RestoreCoordinator {
    pub fn status(&self) -> RestoreStatusView;                 // delegates to RestoreService
    /// BLOCKING (creates OS windows, waits up to the saver deadline ~2s for Resolve).
    /// ★Must NOT be called on the main/event-loop thread★ — callers use an async command +
    /// `tauri::async_runtime::spawn_blocking` (or equivalent). B1 documents this on the fn.
    pub fn answer(&self, accept: bool) -> Result<AnswerReply, AnswerError>;
}
```
- `restore:changed` notification is fired by `RestoreService` (already wired in slice A) — B2 adds nothing for it.
- Layout/tab change notices after commit are emitted by the coordinator itself (B1), through the same path existing layout mutations use.

## Wire DTOs (B2 owns, ts-rs exported to `src-tauri/bindings/`)
- `restore.status` (Read) → `{ crash_copy: "none"|"awaiting"|"answered", saved_at_ms: number|null, windows: number|null, tabs: number|null }`
- `restore.answer { accept: bool }` (Write) → `{ restored_windows: number, durable: bool }`; `AnswerError::Conflict` → `CONFLICT`, `Internal` → `INTERNAL`.
- Tauri invoke wrappers: `restore_status`, `restore_answer` (in new `src-tauri/src/commands/state.rs`) call the same coordinator.
