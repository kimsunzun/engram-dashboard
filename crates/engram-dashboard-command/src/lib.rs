//! # engram-dashboard-command — the command bus **tool** (0 commands)
//!
//! Holds only the envelope · error vocabulary · declaration macro · table · routing. Commands themselves are declared **next to the
//!   producer module**
//! (`agent` = `agent.*`, `daemon` = `mail.*`, `src-tauri` = `window/tab/slot`).
//!
//! ## Three invariants (break them and this crate loses its reason to exist)
//!
//! 1. **0 workspace crate dependencies.** Gate = `rg "path\s*=" crates/engram-dashboard-command/Cargo.toml` → 0 lines (others depending on
//!   this crate is unrelated — the arrow goes one way).
//!    This wall lets `agent` declare commands without seeing the wire contract (`protocol`).
//! 2. **0 commands.** A [`declare_commands!`] call in this file tree is a violation — if the tool owns vocabulary, "declarations live next
//!   to the producer" collapses and the central catalog returns.
//! 3. **★T-1★ What linker collection holds stops at [`CommandSpec`].** With `Arc<dyn CommandHandler>` in `static`·linker collection, tests
//!   could not plug in fake dependencies and unit tests would launch the real
//!    processes. Real handlers are injected by each module's `make_table(deps)` at assembly.
//!
//! ## Entry points
//!
//! [`declare_commands!`] declaration · [`CommandTable`] own table · [`Roster`] registry · [`route`] 3-step delivery ·
//! [`CommandTable::check_args`] entrance-only argument check · [`command_specs`] all declarations linked into this binary.
//!
//! ## Known planned items (read before starting Step 2)
//!
//! - **The entrance check is not called by the wiring (ADR-0157).** [`CommandTable::check_args`] is called only on **surfaces typed by
//!   people·LLMs** — inside [`route`] it would hard-fail old owners on new fields
//!   from a newer-version caller, killing additive evolution (TRD §4-③). Per-surface guidance text is appended by the adapter.
//!
//! - **The envelope types do not implement ts-rs.** `protocol`'s wire messages are all `#[derive(TS)]`, so adding
//!   [`CommandEnvelope`]·[`CommandReply`]·[`RequestId`]·[`CommandDecl`] to that enum as variants
//!   **stops binding generation with a compile error.** Not taking ts-rs is intended (minimal dependencies); Step 2 must choose "add ts-rs
//!   to the tool crate" or "cover it in `protocol` with `#[ts(type = …)]`" — a wire contract decision.
//! - **The panic net is not reached in production processes** — see [`route`]'s panic net (ADR-0292 · ADR-0293); it lives in test builds
//!   without the hook.
// ADR-0155
// ADR-0156

mod coerce;
mod envelope;
mod error;
mod link;
mod macros;
mod roster;
mod route;
mod spec;
mod table;

#[cfg(any(test, feature = "test-support"))]
pub mod testing;

pub use envelope::{CommandEnvelope, CommandReply, OwnerToken, RequestId};
pub use error::{CommandError, ErrorCode, RetryMode};
pub use link::{CommandLink, InboundCommands, ReplySink};
pub use macros::transmitted;
pub use roster::{OwnerLookup, OwnerLookupSource, Roster, RosterEntry};
pub use route::route;
pub use spec::{
    catalog_json, command_specs, duplicate_command_names, lint_spec, spec_item_json, spec_of,
    CommandDecl, CommandSpec, DeadlineClass, Effect, LinkedSpec, COMMON_ERRORS,
};
pub use table::{blocking_handler, CommandFuture, CommandHandler, CommandTable, TableError};

/// Used by the declaration macro when it builds linker-collected items — keeps consuming crates from depending on `inventory` directly.
#[doc(hidden)]
pub use inventory;

/// The declaration macro's `Json` field type expands to `serde_json::Value` through this path, so a declaring
/// crate need not depend on `serde_json` itself.
#[doc(hidden)]
pub use serde_json;
