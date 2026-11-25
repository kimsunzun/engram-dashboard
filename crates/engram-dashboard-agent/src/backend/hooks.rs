//! Hook components, backend-neutral — the hook component a launch plan carries ([`HookRef`]). No backend loads one
//! yet: each answers [`crate::backend::settings_check::ModeLoads::hooks`] `false`, and its `materialize` refuses a plan
//! that carries one.
// ADR-0420

use std::path::PathBuf;

use serde_json::{Map, Value};

/// One hook component to carry — a library folder holding `hooks.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookRef {
    /// The hook folder's library path (`team/hooks/notify`) — what its notices and refusals name.
    pub path: String,
    /// The hook folder, absolute. Its scripts run in place; nothing is copied into the volatile folder.
    pub dir: PathBuf,
    /// The `hooks` object of its `hooks.json`, as read — placeholders not rewritten.
    pub definition: Map<String, Value>,
}
