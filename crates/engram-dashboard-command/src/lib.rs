//! # engram-dashboard-command — the command bus **tool** (0 commands)
//!
//! Holds only the envelope · error vocabulary · declaration macro · table · routing. Commands themselves are declared **next to the producer module**
//! (`agent` = `agent.*`, `daemon` = `mail.*`, `src-tauri` = `window/tab/slot`).
//!
//! ## Three invariants (break them and this crate loses its reason to exist)
//!
//! 1. **0 workspace crate dependencies.** Gate = `rg "path\s*=" crates/engram-dashboard-command/Cargo.toml`
//!    → 0 lines. Others depending on this crate is unrelated to this invariant — the arrow goes one way only.
//!    This wall is what lets `agent` declare commands without seeing the wire contract (`protocol`).
//! 2. **0 commands.** A [`declare_commands!`] call in this file tree is a violation. The moment the tool owns vocabulary,
//!    「declarations live next to the producer」 collapses and the central catalog comes back through another door.
//! 3. **★T-1★ What linker collection holds stops at [`CommandSpec`].** If `static`·linker collection
//!    held `Arc<dyn CommandHandler>`, there would be no spot for tests to plug in fake dependencies, and unit tests would launch the real thing
//!    (processes). Real handlers are injected by each module's `make_table(deps)` at assembly.
//!
//! ## Entry points
//!
//! [`declare_commands!`] declaration · [`CommandTable`] own table · [`Roster`] registry · [`route`] 3-step delivery ·
//! [`CommandTable::check_args`] entrance-only argument check · [`command_specs`] all declarations linked into this binary.
//!
//! ## Known planned items (read before starting Step 2)
//!
//! - **The entrance check is not called by the wiring (ADR-0157).** [`CommandTable::check_args`] is called only on **surfaces typed by
//!   people·LLMs** — putting it inside [`route`] would make new fields carried by a newer-version caller hard-fail an old owner,
//!   killing additive evolution (TRD §4-③). Per-surface guidance text is appended by the adapter.
//!
//! - **The envelope types do not implement ts-rs.** The wire messages of `protocol` are all `#[derive(TS)]`, so
//!   the moment [`CommandEnvelope`]·[`CommandReply`]·[`RequestId`]·[`CommandDecl`] are put into that enum as additive variants,
//!   **binding generation stops with a compile error.** Not taking ts-rs in now is intended (minimal external dependencies) —
//!   in Step 2 one of 「add ts-rs to the tool crate」 and 「cover it on the protocol side with `#[ts(type = …)]`」 must
//!   be chosen, and that is a wire contract decision, so it is judged then.
//! - **The panic net is not reached in production processes** — the daemon · shell end any panic outside the 「containing」 marker before the panic
//!   unwinds (ADR-0292), and nowhere drives command execution inside the marker (ADR-0293). The net lives in test
//!   builds without the hook.
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
    CommandDecl, CommandSpec, Effect, LinkedSpec, COMMON_ERRORS,
};
pub use table::{blocking_handler, CommandFuture, CommandHandler, CommandTable, TableError};

/// Used by the declaration macro when it builds linker-collected items — keeps consuming crates from depending on `inventory` directly.
#[doc(hidden)]
pub use inventory;
