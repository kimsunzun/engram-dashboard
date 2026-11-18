//! Shell-side bridge for commands the webview owns: takes the boot report onto the registration packet, sends the envelope the daemon delivered down to that webview, and correlates the reply (TRD §3-7 clause 2 · hop ② of §3-8).
//!
//! Not a second control surface: what runs inside the webview is still the single `run(id, args)` of `src/commands/registry.ts` (ADR-0055); this module adds only the outer path to it, and creates
// hangul-ok-begin
//! neither a new global handle nor a second registry (CLAUDE.md 「LLM-우선 제어」).
// hangul-ok-end
//!
//! ## Reserved names — only shell-table names are filtered; the wall for daemon names is the daemon
//! The webview registry has ids spelled the same as names the shell table answers first (`tab.create` · `slot.close`). Both declaration sets go in one registration packet and the roster folds a same name within one packet without error, so a `help` attached to such an id would silently overlap two owners. [`reserved_names`] therefore takes the set from the shell-table declarations (no hand list) and removes it at report time.
//! Names the daemon answers (`agent.spawn` · `agent.rename`, spelled the same in the webview registry) are not filtered here (ADR-0270 decision 2): the daemon bounces a registration packet wholesale if even one is carried (`refuse_names_i_answer` in `engram-dashboard-daemon`), leaving this shell with 0 commands and the same bounce at every reconnect until the declaration is fixed and rebuilt. The bounce is logged at each reconnect; the OS error window appears once per process per (place, wording) (`RefusalAlerts` · ADR-0270 decision 4 · ADR-0281).
//!
//! ## Why the delivery target is one window
//! Scattering an event over all windows runs the same command once per window and returns as many replies (one reply per `request_id` — TRD §4-⑤ violation). Only one reporting window is the destination, and the advertised list is that window's too — otherwise no window can run a name the daemon advertised ([`Registered`]). Picking rule: [`Registered::repick`]; recovery from a dead window: [`ViewCommandBridge::prune_dead`].
//!
//! ## Entry points
//! - [`ViewCommandBridge::report_and_push`] — the boot report (`report_view_commands` invoke).
//! - [`ViewCommandBridge::settle`] — the outcome the webview sent (`report_command_outcome` invoke).
//! - The [`crate::daemon_client::inbound::ViewCommandPort`] implementation — the three endpoints the receiver uses.
// ADR-0155 · ADR-0156

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use engram_dashboard_base::sync;
use engram_dashboard_command::{
    CommandDecl, CommandEnvelope, CommandError, CommandReply, ErrorCode, OwnerLookup, OwnerToken,
    RequestId,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;

use crate::daemon_client::inbound::ViewCommandPort;
use crate::layout::MAIN_WINDOW_LABEL;

pub const EVT_COMMAND_REQUEST: &str = "command:request";

/// Owner marker of a webview command in the registry. The daemon does not look at it (its owner key derives from the connection; webview names are listed under the shell token — TRD §3-7 clause 2); it is used only inside the shell, where `route` writes the registry's answer over the outer cover for two-stage delivery.
const VIEW_OWNER: &str = "engram-dashboard-webview";

/// Upper bound on waiting for the webview reply; it must fit inside the outer budget. `route` sets no deadline by contract (its doc: the deadline is the assembly's job) — this is the assembly, and without it an envelope after the window vanishes never ends.
/// The value is not chosen by "how long a store operation takes": this round trip runs inside the daemon's seat deadline (`CommandDeliveries::DEFAULT_DEADLINE` of `engram-dashboard-daemon`). Larger than that, two things break:
/// - This deadline becomes unreachable: the daemon withdraws the seat first and answers `TIMEOUT`/`retry: never`, and this side's late outcome is dropped as `NoSeat`.
/// - The same command runs twice: a re-call with the same id after the daemon deadline finds the seat still holding the first round trip, so a second envelope goes down. [`PendingSlot::hold`]'s refusal is the defense line; the inequality prevents the situation.
/// Inequality: `VIEW_REPLY_DEADLINE + VIEW_HOP_MARGIN ≤ the daemon seat deadline`. That deadline cannot be seen here (the daemon crate is a dev dependency), so the value is fixed here and the relation pinned in `tests/layout_commands.rs` (`the_webview_deadline_fits_inside_the_daemon_seat`). Do not copy the arithmetic here — the daemon-side doc is canonical (same clause as core `CLI_CONTROL_READ_TIMEOUT_SECS`).
pub const VIEW_REPLY_DEADLINE: Duration = Duration::from_secs(4);

/// Time that flows outside this round trip — the two daemon↔shell socket hops, apply task scheduling, outcome serialization. Same kind as the daemon's `CALLER_MARGIN`.
pub const VIEW_HOP_MARGIN: Duration = Duration::from_secs(3);

// ── Shape the webview sends ───────────────────────────────────────────────────────

/// Shape of one argument field — the only place the webview writes by hand (Rust fills it via the declaration macro). It is unfolded as is into a JSON Schema fragment ([`ViewCommandHelp::args_schema`]), so field names are that side's vocabulary.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ViewArgSchema {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub ty: Option<String>,
    #[serde(rename = "enum", default, skip_serializing_if = "Option::is_none")]
    pub allowed: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Read/write marker — a Rust declaration decides it with `#[effect(..)]`, TypeScript has no such place, so the webview must carry it: a hard-coded constant would advertise a false marker the day a query command is attached, and that value feeds the daemon's write-preservation accounting (`Effect::Read` = dedup exempt — ADR-0156). An entry without it is dropped from registration ([`ViewCommandBridge::report`]); choosing a default is itself the lie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewEffect {
    Read,
    Write,
}

impl ViewEffect {
    fn as_catalog_str(self) -> &'static str {
        match self {
            ViewEffect::Read => "Read",
            ViewEffect::Write => "Write",
        }
    }
}

/// One command's description — a name alone cannot be called, so the argument shape comes with it (ADR-0156).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ViewCommandHelp {
    pub summary: String,
    /// `None` = registration refused. `#[serde(default)]` but no default is chosen ([`ViewEffect`]); only an old webview bundle lacks the field, and having no name beats quietly advertising `Write`.
    #[serde(default)]
    pub effect: Option<ViewEffect>,
    #[serde(default)]
    pub args: BTreeMap<String, ViewArgSchema>,
    #[serde(default)]
    pub required: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewCommandDecl {
    pub name: String,
    pub help: ViewCommandHelp,
}

/// The envelope that goes down to the webview — carries no `owner` · `proto_ver` (routing material between hops, `CommandEnvelope` doc; the webview as last hop sees only its own table).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewCommandRequest {
    pub request_id: String,
    pub name: String,
    pub args: serde_json::Value,
}

// ── Delivery seam ────────────────────────────────────────────────────────────────

/// Channel that pushes an envelope to one window. Injected because the real `AppHandle::emit_to` needs a window (ADR-0012); a fake lets filtering · correlation · deadline be asserted with zero windows.
pub trait ViewDispatch: Send + Sync {
    fn deliver(&self, target: &str, request: &ViewCommandRequest) -> Result<(), String>;

    /// Does the window with that label still exist — cannot be replaced by a failure of [`ViewDispatch::deliver`]: Tauri's `emit_to` gives `Ok` even for a nonexistent label (0 matching listeners wake no one — `Listeners::emit_js_filter`), so "send and see if it fails" does not work, and a bridge holding a dead destination would wait the deadline on every envelope and emit `TIMEOUT`.
    fn is_alive(&self, label: &str) -> bool;
}

/// Production implementation — sends only to the webview of that label. `emit_to` alone does not narrow to one window: Tauri wakes every listener registered as `Any` regardless of the filter (`match_any_or_filter`), and JS `listen()` defaults to `Any`. So the webview hooks with `{ target: getCurrentWindow().label }` (`src/commands/viewCommandBridge.ts`) — without it the command runs once per window with as many replies. The two lines are a pair.
pub struct TauriViewDispatch(pub AppHandle);

impl ViewDispatch for TauriViewDispatch {
    fn deliver(&self, target: &str, request: &ViewCommandRequest) -> Result<(), String> {
        self.0
            .emit_to(target, EVT_COMMAND_REQUEST, request)
            .map_err(|e| format!("could not send the command to webview '{target}': {e}"))
    }

    /// Use `get_webview_window`, not `get_window` (the latter is behind tauri's `unstable` feature, which this shell cannot enable). Every window of this app is single-webview (the two static ones in `tauri.conf.json` + popouts via `WebviewWindowBuilder`), so both labels are equal and the value kept from `Window::label()` is looked up as is. A two-webview window would break that equality — move the keeping and lookup sides together.
    fn is_alive(&self, label: &str) -> bool {
        self.0.get_webview_window(label).is_some()
    }
}

// ── Reserved names ────────────────────────────────────────────────────────────────

/// Names the webview cannot take = those the shell's own table answers.
/// Taken from the declarations, not a hand list, so it grows with the shell-table vocabulary. It counts declarations (not what is plugged in) — the safe direction: a webview taking a declared-but-unplugged name would silently lose delivery to the shell once it is plugged (`route` looks at the table first). It excludes names the daemon answers (that wall is the daemon's bounce; the shell does not depend on the agent crate — ADR-0270 decision 1). Do not remove it: the daemon bounce sees only daemon-answered names and the roster folds a same name within one packet without error (`register` in command `roster.rs`).
// ADR-0270
pub fn reserved_names() -> BTreeSet<String> {
    crate::layout::commands::COMMAND_SPECS
        .iter()
        .map(|spec| spec.name.to_string())
        .collect()
}

// ── Bridge ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Default, PartialEq)]
pub struct ReportOutcome {
    pub accepted: Vec<CommandDecl>,
    pub added: Vec<CommandDecl>,
    pub removed: Vec<String>,
    pub refused: Vec<String>,
}

impl ReportOutcome {
    pub fn changed(&self) -> bool {
        !self.added.is_empty() || !self.removed.is_empty()
    }
}

/// ADR-0156
type Shapes = BTreeMap<String, String>;

/// Per-window reports, kept as they are — do not merge. The advertised list and the envelope-receiving window must come from the same window: merged, a popout's report swaps the list while delivery keeps going to main, so the daemon advertises B and main runs A (an advertised name goes out as `UNKNOWN_COMMAND`, or a running name is not advertised). Every window uploads the same static `contributions` today, but no contract is laid on that coincidence.
#[derive(Default)]
struct Registered {
    reports: BTreeMap<String, Shapes>,
    /// The envelope-receiving window = the owner of the advertised list. `None` = no live reporter.
    host: Option<String>,
}

impl Registered {
    /// Removes dead reporters — the liveness decision is made outside (lock discipline: [`ViewCommandBridge::prune_dead`]).
    fn forget(&mut self, dead: &[String]) {
        for label in dead {
            self.reports.remove(label);
        }
    }

    /// Pick the destination again — main first, keep a live host, last resort the first reporter in lexicographic order.
    /// Why pick again: a host decided once has no recovery after its window closes — main's single report fails (that invoke has no retry), a popout becomes host and closes, and every delivery then goes to a dead label, waits the deadline and becomes `TIMEOUT` while the name stays registered "present but never working".
    /// Why main first: every window runs the same App and reports (`src/App.tsx`), so following the last report would make the destination wobble whenever a popout closes. Closing main only hides it (`CloseRequested` in `lib.rs`), so its lifetime equals the app's.
    /// [unverified — GUI] that a hidden main stays in the webview table: the automation harness cannot hide the window (the app's permissions block `hide()` · `close()`).
    /// Why the last resort ignores visibility: no static window is config-hidden (ADR-0225); if one returns, the first survivor in lexicographic order could become the destination and `tab.next` would pass tabs of a window nobody sees while answering success. Even then the main of a `--hidden` boot must not be excluded (it opens via the tray, so hidden is its state, not its nature).
    fn repick(&mut self) {
        let keep = self
            .host
            .as_deref()
            .filter(|label| self.reports.contains_key(*label));
        if keep == Some(MAIN_WINDOW_LABEL) {
            return;
        }
        // Do not swap out a live host without reason — apart from main taking it, the place moves only when the host has died.
        self.host = match self.reports.contains_key(MAIN_WINDOW_LABEL) {
            true => Some(MAIN_WINDOW_LABEL.to_string()),
            false => keep
                .map(str::to_string)
                .or_else(|| self.reports.keys().next().cloned()),
        };
    }

    fn advertised(&self) -> Shapes {
        self.host
            .as_ref()
            .and_then(|label| self.reports.get(label))
            .cloned()
            .unwrap_or_default()
    }
}

pub struct ViewCommandBridge {
    dispatch: Arc<dyn ViewDispatch>,
    reserved: BTreeSet<String>,
    deadline: Duration,
    state: Mutex<Registered>,
    /// Gate that binds one report's state change and its diff send into one block (FIX: reversal of two reports). Without it, report A adds a name and B removes it, B's send can enter the queue before A's and the daemon keeps the union (A's `added` arrives later) while the bridge knows only B's set — so a delivery of A's name gets "unknown" from the webview. The wire cannot carry an ordering field (contract unchanged), so the sending side is serialized.
    outbound: tokio::sync::Mutex<()>,
    pending: Arc<Pending>,
}

/// One seat waiting for a reply, including whom it was sent to. The party an outcome is checked against is the window that actually received this envelope, not the host at that time: the host can change by report order (main overwrites the popout), so checking the current host would bounce the real reply and let an old host's forgery through.
struct Waiting {
    seat: u64,
    target: String,
    answer: oneshot::Sender<Result<serde_json::Value, CommandError>>,
}

/// Seats waiting for replies. `Arc` because the deadline timer's future is `'static` and cannot borrow the bridge, yet must clear its own seat directly (else one dead window grows the map permanently).
#[derive(Default)]
struct Pending {
    seats: Mutex<HashMap<RequestId, Waiting>>,
    /// Drawn anew for each seat — never rewound.
    next_seat: AtomicU64,
}

impl ViewCommandBridge {
    pub fn new(dispatch: Arc<dyn ViewDispatch>) -> Self {
        Self::with_reserved(dispatch, VIEW_REPLY_DEADLINE, reserved_names())
    }

    /// Harness-only: gives the reserved set · deadline directly (production uses [`reserved_names`]).
    pub fn with_reserved(
        dispatch: Arc<dyn ViewDispatch>,
        deadline: Duration,
        reserved: impl IntoIterator<Item = String>,
    ) -> Self {
        ViewCommandBridge {
            dispatch,
            reserved: reserved.into_iter().collect(),
            deadline,
            state: Mutex::new(Registered::default()),
            outbound: tokio::sync::Mutex::new(()),
            pending: Arc::new(Pending::default()),
        }
    }

    /// Clear out dead reporters and pick the destination again.
    /// The liveness lookup runs outside my lock — do not reverse this (ADR-0006: no external calls while holding a lock): the real lookup takes Tauri's global webview table lock and per-window locks. No path re-enters this bridge under that lock today, but an outside change could silently reverse that into a deadlock. Hence three steps: (1) snapshot labels inside the lock, (2) look up outside, (3) re-lock and apply.
    /// A window may die in between or a new report may come in: a label not in the snapshot is cleared next call, and clearing an already vanished label is harmless — recovery one beat late is the contract.
    fn prune_dead(&self) {
        let labels: Vec<String> = {
            let state = sync::lock(&self.state);
            state.reports.keys().cloned().collect()
        };
        let dead: Vec<String> = labels
            .into_iter()
            .filter(|label| !self.dispatch.is_alive(label))
            .collect();
        let mut state = sync::lock(&self.state);
        state.forget(&dead);
        state.repick();
    }

    /// One window reports its own list at boot — that window's whole set (not a diff).
    /// Reserved names are dropped quietly but not silently: bouncing would keep the rest from going up, and saying nothing would leave "registered but never called" without a cause — so they come back via [`ReportOutcome::refused`] for the caller to log. Entries with an empty shape are dropped too: `help` is opaque to the daemon so empty registers, but a caller discovering such a name has no material for arguments (ADR-0156 removed that round trip).
    /// The diff is the before/after difference of the advertised set, not that window's list: a non-host report that leaves the advertisement unchanged sends nothing ([`Registered`] coupling).
    /// Only the harness calls this directly; production uses [`ViewCommandBridge::report_and_push`], which binds state change and diff send in one gate (`outbound` field) — this alone gives no ordering guarantee.
    /// Known residual: names of a dead host stay in the daemon registry until reconnect. If [`ViewCommandBridge::prune_dead`] clears the dead host first, the `previous` seen here is already shrunk, so vanished names are not sent as `removed`; calling one reaches the shell and returns "unknown name". Only a reconnect's whole registration resolves it, and nothing schedules that reconnect. Fixing it would need the delivery path to emit a diff, i.e. this crate knowing the daemon link (a dependency declined — module header).
    pub fn report(&self, label: &str, reported: Vec<ViewCommandDecl>) -> ReportOutcome {
        let mut refused = Vec::new();
        let mut next: Shapes = BTreeMap::new();
        for decl in reported {
            // A reserved name · an empty description · no marker — all three drop here (reasons in the doc above and [`ViewEffect`]).
            let Some(effect) = decl.help.effect else {
                refused.push(decl.name);
                continue;
            };
            if self.reserved.contains(&decl.name) || decl.help.summary.trim().is_empty() {
                refused.push(decl.name);
                continue;
            }
            let help = decl.help.to_catalog_item(&decl.name, effect);
            next.insert(decl.name, help);
        }

        self.prune_dead();
        let mut state = sync::lock(&self.state);
        let previous = state.advertised();
        state.reports.insert(label.to_string(), next);
        // Pick again after putting in the report, so a window that just came up can take over from a dead host.
        state.repick();
        let current = state.advertised();
        drop(state);

        let added = current
            .iter()
            .filter(|(name, help)| previous.get(*name) != Some(help))
            .map(|(name, help)| CommandDecl {
                name: name.clone(),
                help: help.clone(),
                agent_args: Vec::new(),
            })
            .collect();
        let removed = previous
            .keys()
            .filter(|name| !current.contains_key(*name))
            .cloned()
            .collect();
        ReportOutcome {
            accepted: decls_of(&current),
            added,
            removed,
            refused,
        }
    }

    /// Production path — apply the report and, if anything changed, send the diff within the same gate. `push(added, removed)` is the round trip to the daemon (`commands/view_bus.rs` builds `UpdateCommands`); the caller injects it because this crate does not know that wire. The gate covers change + send together: serializing only the send would flip state first and compute the later report's diff against stale state (`outbound` field).
    pub async fn report_and_push<F, Fut>(
        &self,
        label: &str,
        reported: Vec<ViewCommandDecl>,
        push: F,
    ) -> ReportOutcome
    where
        F: FnOnce(Vec<CommandDecl>, Vec<String>) -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let _order = self.outbound.lock().await;
        let outcome = self.report(label, reported);
        if outcome.changed() {
            push(outcome.added.clone(), outcome.removed.clone()).await;
        }
        outcome
    }

    /// Attach the outcome the webview sent to its round trip — `Err` if there is no seat (late · second reply · someone else's key). `from` = the sending window (the value Tauri puts into the invoke — `commands/view_bus.rs`).
    /// Only the window that received the envelope may answer: with the correlation key alone another window could end another's round trip — the caller would take the forged outcome while the real window's reply is bounced and the side effect still happens. The party checked is the actual receiver recorded in [`Waiting`], not the current host. A failed check leaves the seat — removing it would let one forgery erase the real reply's seat.
    /// A second reply is not swallowed quietly: it means two windows received the same envelope (the delivery target is no longer one), so the caller must be able to log it.
    pub fn settle(
        &self,
        from: &str,
        request_id: &str,
        outcome: Result<serde_json::Value, CommandError>,
    ) -> Result<(), String> {
        let parsed = uuid::Uuid::parse_str(request_id.trim())
            .map(RequestId)
            .map_err(|_| format!("request_id is not a UUID: {request_id:?}"))?;
        let mut waiting = sync::lock(&self.pending.seats);
        let Some(slot) = waiting.get(&parsed) else {
            return Err(format!(
                "no round trip is waiting (already answered or past the deadline): {parsed}"
            ));
        };
        if slot.target != from {
            return Err(format!(
                "this round trip went down to '{}' but '{from}' answered — not attaching the outcome: {parsed}",
                slot.target
            ));
        }
        let slot = waiting.remove(&parsed).expect("the seat seen just above");
        drop(waiting);
        slot.answer
            .send(outcome)
            .map_err(|_| format!("the party to receive the reply is already gone: {parsed}"))
    }

    /// The envelope-receiving window and its advertised list, after filtering dead reporters. Advertisement (registration packet) · lookup (delivery stage 2) · delivery all pass through this one — three separate state reads would diverge the moment one sees the old host.
    fn live_host(&self) -> Option<(String, Shapes)> {
        self.prune_dead();
        let state = sync::lock(&self.state);
        let label = state.host.clone()?;
        Some((label, state.advertised()))
    }

    pub fn host(&self) -> Option<String> {
        self.live_host().map(|(label, _)| label)
    }
}

fn decls_of(map: &BTreeMap<String, String>) -> Vec<CommandDecl> {
    map.iter()
        .map(|(name, help)| CommandDecl {
            name: name.clone(),
            help: help.clone(),
            agent_args: Vec::new(),
        })
        .collect()
}

impl ViewCommandHelp {
    /// Unfolds into one catalog entry using the same field names the Rust declaration emits (`engram_dashboard_command::spec_item_json`) — diverging would mix two dialects in one registry and force the LLM to read each command differently.
    /// `effect` is the webview's value as is ([`ViewEffect`]); the other three are decided here:
    /// - `since: 1` — a webview-path name has no generation; counting generations would need a catalog number on that side, so it is pinned to the first generation rather than invented.
    /// - `ok: {}` — "anything": webview `run` returns vary per command (void · id · Promise) with no contract, so unknown is written instead of an invented shape.
    /// - `errors` — the whole set this bridge can emit ([`ViewCommandBridge`] branches): webview failure (`INTERNAL`) · deadline (`TIMEOUT`) · no window to send to (`UNSUPPORTED`) · same number already running (`REQUEST_ID_CONFLICT`). Do not emit an unadvertised code — callers build branching from this list and an unlisted code falls to their default (usually "unknown failure"). A webview-thrown error cannot be told apart by kind (known limitation): argument mistake and real failure both go out as `INTERNAL` + text; typed webview codes are not this piece's decision.
    /// `effect` is taken as an argument and `self`'s `Option` is not read — reading it here would revive "what if no value", whose every answer is a false advertisement; [`ViewCommandBridge::report`] already peeled the field.
    fn to_catalog_item(&self, name: &str, effect: ViewEffect) -> String {
        let item = serde_json::json!({
            "name": name,
            "effect": effect.as_catalog_str(),
            "since": 1,
            "summary": self.summary.trim(),
            "args": self.args_schema(),
            "ok": {},
            "errors": [
                ErrorCode::Internal.as_str(),
                ErrorCode::Timeout.as_str(),
                ErrorCode::Unsupported.as_str(),
                ErrorCode::RequestIdConflict.as_str(),
            ],
        });
        // Serialization cannot fail (all JSON values); it still folds to an empty string rather than killing the packet — an empty `help` was dropped by the report filter, so reaching here means the contract is broken, and the log carries that signal.
        serde_json::to_string(&item).unwrap_or_default()
    }

    fn args_schema(&self) -> serde_json::Value {
        let properties: serde_json::Map<String, serde_json::Value> = self
            .args
            .iter()
            .map(|(field, schema)| {
                (
                    field.clone(),
                    serde_json::to_value(schema).unwrap_or_else(|_| serde_json::json!({})),
                )
            })
            .collect();
        // `required` keeps only declared fields — a nonexistent field in `required` makes the schema unsatisfiable (Rust-side `lint_spec` blocks the same).
        let required: Vec<&String> = self
            .required
            .iter()
            .filter(|field| self.args.contains_key(*field))
            .collect();
        serde_json::json!({
            "type": "object",
            "properties": properties,
            "required": required,
        })
    }
}

impl ViewCommandPort for ViewCommandBridge {
    /// What is advertised is the list of the live host window; picked again on every reconnect, so a dead window's names do not go out in the next registration packet.
    fn declarations(&self) -> Vec<CommandDecl> {
        self.live_host()
            .map(|(_, shapes)| decls_of(&shapes))
            .unwrap_or_default()
    }

    fn lookup(&self, name: &str) -> OwnerLookup {
        // No destination window = "unknown name": answering that an owner exists with nowhere to send would go as far as `link.send` and error, indistinguishable to the caller.
        match self
            .live_host()
            .is_some_and(|(_, shapes)| shapes.contains_key(name))
        {
            true => OwnerLookup::Available(OwnerToken::new(VIEW_OWNER)),
            false => OwnerLookup::Unknown,
        }
    }

    fn dispatch(&self, env: CommandEnvelope) -> Pin<Box<dyn Future<Output = CommandReply> + Send>> {
        let request_id = env.request_id;
        let Some((target, _)) = self.live_host() else {
            return ready(CommandReply::err(
                request_id,
                CommandError::of(
                    ErrorCode::Unsupported,
                    "no webview has reported its commands yet, so there is nowhere to run this",
                ),
            ));
        };
        let request = ViewCommandRequest {
            request_id: request_id.to_string(),
            name: env.name,
            args: env.args,
        };

        let (answer, wait) = oneshot::channel();
        // Hang the seat before sending: the reply by invoke may arrive before the emit returns and would be dropped as "no round trip waiting". A number already running is not sent (`PendingSlot::hold`).
        let Some(slot) = PendingSlot::hold(&self.pending, request_id, &target, answer) else {
            return ready(CommandReply::err(
                request_id,
                CommandError::of(
                    ErrorCode::RequestIdConflict,
                    "this request id is already running in the webview — wait for its answer instead of resending",
                ),
            ));
        };
        if let Err(detail) = self.dispatch.deliver(&target, &request) {
            return ready(CommandReply::err(
                request_id,
                CommandError::of(ErrorCode::Internal, detail),
            ));
        }

        let deadline = self.deadline;
        Box::pin(async move {
            // Carried into the future: deadline or cancellation (dropping the future discards the round trip, the `route` cancellation meaning) converge on a single `Drop`.
            let _slot = slot;
            match tokio::time::timeout(deadline, wait).await {
                Ok(Ok(outcome)) => CommandReply {
                    request_id,
                    outcome,
                },
                Ok(Err(_)) => CommandReply::err(
                    request_id,
                    CommandError::of(
                        ErrorCode::Internal,
                        "the webview answer channel closed without an answer",
                    ),
                ),
                Err(_) => CommandReply::err(
                    request_id,
                    CommandError::of(
                        ErrorCode::Timeout,
                        format!(
                            "the webview did not answer within {}s — the window may be gone",
                            deadline.as_secs()
                        ),
                    ),
                ),
            }
        })
    }
}

/// Lifetime of one reply seat — clearing is a single `Drop`. Hand-clearing would grow the map on a missed branch; there are four: delivery failure · normal reply (`settle` took it) · deadline · this future dropped (cancellation, the `route` meaning) — the last cannot be reached by hand, and without it a seat remains whenever the runtime shuts down.
struct PendingSlot {
    pending: Arc<Pending>,
    request_id: RequestId,
    /// My seat's identity — the only thing that keeps [`PendingSlot::drop`] from clearing another's seat.
    seat: u64,
}

impl PendingSlot {
    /// Take a seat — `None` if the number is already running; a live waiter is not pushed out. Covering quietly would run the same command twice when the same number comes again while the first round trip is in the webview: a second envelope (side effect twice), the first waiter dropped unanswered, and the old outcome attached as the new attempt's answer. The situation itself should not arise (the outer deadline would have to end before the inner; [`VIEW_REPLY_DEADLINE`]'s inequality blocks that) — this refusal is the second net.
    fn hold(
        pending: &Arc<Pending>,
        request_id: RequestId,
        target: &str,
        answer: oneshot::Sender<Result<serde_json::Value, CommandError>>,
    ) -> Option<Self> {
        let mut waiting = sync::lock(&pending.seats);
        match waiting.entry(request_id) {
            Entry::Occupied(_) => None,
            Entry::Vacant(vacancy) => {
                let seat = pending.next_seat.fetch_add(1, Ordering::Relaxed);
                vacancy.insert(Waiting {
                    seat,
                    target: target.to_string(),
                    answer,
                });
                drop(waiting);
                Some(PendingSlot {
                    pending: Arc::clone(pending),
                    request_id,
                    seat,
                })
            }
        }
    }
}

impl Drop for PendingSlot {
    /// Clear only when it is my seat — the same number with a different seat is another's. Blocks: `settle` takes X and sends the answer → a new envelope takes a new seat with X → this old future's guard drops and evicts the new seat (that waiter fails with a closed channel). It does not arise today (the guard drops before the outcome reaches the daemon, so the daemon cannot reuse the number), but that argument rests on two modules' properties and would collapse silently if either changed; the identity comparison removes the need for it.
    fn drop(&mut self) {
        let mut waiting = sync::lock(&self.pending.seats);
        if waiting
            .get(&self.request_id)
            .is_some_and(|held| held.seat == self.seat)
        {
            waiting.remove(&self.request_id);
        }
    }
}

fn ready(reply: CommandReply) -> Pin<Box<dyn Future<Output = CommandReply> + Send>> {
    Box::pin(async move { reply })
}
