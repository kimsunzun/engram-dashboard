//! The shell-side bridge for commands the webview owns — it takes the boot report and puts it on the registration packet, sends the envelope the daemon delivered
//! down to that webview, and then correlates the reply (TRD §3-7 clause 2 · hop ② of §3-8).
//!
//! ★It is not a second control surface★: what actually runs inside the webview is still the single
//! `run(id, args)` of `src/commands/registry.ts` (ADR-0055). All this module adds is the **outer path** to that entrance, and it creates
//! neither a new global handle nor a second registry (CLAUDE.md 「LLM-우선 제어」).
//!
//! ## ★Reserved names — only shell-table names are filtered · the wall for daemon names is the daemon★
//! The webview registry has ids with **the same spelling** as those the shell's own table answers first, such as `tab.create` · `slot.close`.
//! The shell table's declarations and the webview's declarations go in one registration packet, and the registry folds the same name within one packet into one without an error,
//! so if someone attaches `help` to that id, two owners silently overlap. So [`reserved_names`] takes that set **from the shell-table
//! declarations** and removes it at report time (no hand-written list — as the vocabulary grows, the list silently falls behind).
//! ★Names the daemon answers (`agent.spawn` · `agent.rename`, etc. — the webview registry has the same spelling) are not
//! filtered here (ADR-0270 decision 2)★ — the daemon bounces a registration packet wholesale if even one such name is carried
//! (`refuse_names_i_answer` in `engram-dashboard-daemon`). Then this shell's commands become 0 and the same bounce comes at every reconnect until
//! the declaration at fault is fixed and rebuilt. That bounce is left in the shell warn log at every reconnect, and the OS error
//! window appears only once per process for the same (place, wording) (`RefusalAlerts` · ADR-0270 decision 4 · ADR-0281).
//!
//! ## ★Why the delivery target is one window★
//! If an event is scattered over all windows, the same command runs as many times as there are windows and as many replies come back (one reply per `request_id` —
//! TRD §4-⑤ violation). So only **one** of the windows that reported is made the destination, and **the advertised list is also that window's** —
//! if the two come from different windows, there is no window to run a name the daemon advertised ([`Registered`]).
//! The picking rule is [`Registered::repick`], and the path to recover from a dead window is [`ViewCommandBridge::prune_dead`].
//!
//! ## Entry points
//! - [`ViewCommandBridge::report_and_push`] — the boot report (`report_view_commands` invoke).
//! - [`ViewCommandBridge::settle`] — the outcome the webview sent (`report_command_outcome` invoke).
//! - The [`crate::daemon_client::inbound::ViewCommandPort`] implementation — the three endpoints the receiver uses.
// ADR-0155
// ADR-0156

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

/// Shell → webview. Goes only to one destination window (module header 「배달 대상이 창 하나인 이유」).
pub const EVT_COMMAND_REQUEST: &str = "command:request";

/// The owner marker of a webview command in the registry.
///
/// ★The daemon does not look at this value★ — the registry's owner key is derived from the **connection** the packet came from, and webview names are also listed together under the shell
/// token (TRD §3-7 clause 2). This token is used only **inside the shell**: `route` writes the registry's answer over the
/// outer cover (that is the material of two-stage delivery), so there must be a value to write.
const VIEW_OWNER: &str = "engram-dashboard-webview";

/// The upper bound on waiting for the webview reply — ★it must fit **inside** the outer budget★.
///
/// ★Without it, an envelope after the window disappears never ends★ — `route` by contract sets no deadline
/// (that function's doc: 「마감시각은 조립부의 몫」), and this spot is the assembly.
///
/// ★★The criterion for choosing the value is **not** 「store 조작이 얼마나 걸리나」★★ — this round trip runs **inside** the deadline of the seat table
/// the daemon holds (`CommandDeliveries::DEFAULT_DEADLINE` of `engram-dashboard-daemon`). If set
/// larger than the outside, two things break at once:
/// - **This deadline becomes unreachable.** The daemon withdraws the seat first and answers the caller `TIMEOUT`/`retry: never`,
///   and this side's late outcome is dropped as a seat-less reply (`NoSeat`) — neither the diagnostic text written here nor
///   `retry: same-request-id` ever reaches the caller **even once**.
/// - **The same command runs twice.** If the caller calls again with the same id after the daemon deadline, this seat is still
///   holding the first round trip, so a second envelope goes down to the webview again. The defense line is the refusal of [`PendingSlot::hold`], and
///   the inequality keeps that situation from arising at all.
///
/// ★Inequality: `VIEW_REPLY_DEADLINE + VIEW_HOP_MARGIN ≤ 데몬 자리 마감` (the daemon seat deadline)★. That seat deadline **cannot be seen
/// here** — the daemon crate is a dev dependency of this package, so production code cannot reference it. So the value is fixed here,
/// and the relation is pinned where both can be seen (in `tests/layout_commands.rs`,
/// `the_webview_deadline_fits_inside_the_daemon_seat`). ★Do not copy that arithmetic here★ — the daemon-side doc is
/// canonical, and on the day the two copies diverge neither can be trusted (the same clause as core `CLI_CONTROL_READ_TIMEOUT_SECS`).
pub const VIEW_REPLY_DEADLINE: Duration = Duration::from_secs(4);

/// The share of time that flows **outside** this round trip — the socket round trips of the two daemon↔shell hops · scheduling of the apply task · serializing
/// the outcome. A term of the same kind as the daemon's `CALLER_MARGIN`, and with it the inequality above becomes fact rather than prose.
pub const VIEW_HOP_MARGIN: Duration = Duration::from_secs(3);

// ── Shape the webview sends ───────────────────────────────────────────────────────

/// The shape of one argument field — the only place the webview **writes by hand** (on the Rust side the declaration macro fills it in).
///
/// It is unfolded as is into a JSON Schema fragment ([`ViewCommandHelp::args_schema`]) — so the field names are that side's vocabulary.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ViewArgSchema {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub ty: Option<String>,
    #[serde(rename = "enum", default, skip_serializing_if = "Option::is_none")]
    pub allowed: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// The read/write marker — a Rust declaration decides it beside compilation with `#[effect(..)]`, but TypeScript has no such place.
///
/// ★So the webview **must carry it**★: if hard-coded as a constant, the day the first query command is attached the registry would advertise a false marker,
/// and that value feeds straight into the daemon's write-preservation accounting (`Effect::Read` = dedup exempt — ADR-0156). An entry that does not carry it
/// is dropped from registration ([`ViewCommandBridge::report`]) — because choosing a default is itself that lie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewEffect {
    Read,
    Write,
}

impl ViewEffect {
    /// Catalog vocabulary — must have the **same spelling** as the Rust-side `Effect::as_str` (no two dialects in one registry).
    fn as_catalog_str(self) -> &'static str {
        match self {
            ViewEffect::Read => "Read",
            ViewEffect::Write => "Write",
        }
    }
}

/// The description of one command — a name alone cannot be called, so the **argument shape** comes with it (ADR-0156).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ViewCommandHelp {
    pub summary: String,
    /// ★`None` = registration refused★. It is `#[serde(default)]` but **does not choose** a default — for the reason, see [`ViewEffect`].
    /// Only an old webview bundle fails to carry this field, and in that case having no name is better than quietly advertising it as `Write`.
    #[serde(default)]
    pub effect: Option<ViewEffect>,
    #[serde(default)]
    pub args: BTreeMap<String, ViewArgSchema>,
    #[serde(default)]
    pub required: Vec<String>,
}

/// One entry the webview reports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewCommandDecl {
    pub name: String,
    pub help: ViewCommandHelp,
}

/// The envelope that goes down to the webview — ★it does not carry `owner` · `proto_ver`★. Those two are routing material between hops
/// (`CommandEnvelope` doc), and the webview, as the last hop, sees only its own table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewCommandRequest {
    pub request_id: String,
    pub name: String,
    pub args: serde_json::Value,
}

// ── Delivery seam ────────────────────────────────────────────────────────────────

/// The channel that pushes an envelope to one window — ★why injected★: the real thing is `AppHandle::emit_to`, so it cannot be stood up without a window
/// (ADR-0012). If a fake is plugged in, all of filtering·correlation·deadline are asserted with zero windows.
pub trait ViewDispatch: Send + Sync {
    fn deliver(&self, target: &str, request: &ViewCommandRequest) -> Result<(), String>;

    /// Does the window with that label still exist — ★it cannot be replaced by a failure of [`ViewDispatch::deliver`]★.
    ///
    /// Tauri's `emit_to` **gives `Ok` even for a label that does not exist** (if there are 0 matching listeners it simply wakes no one —
    /// `Listeners::emit_js_filter`). So the cheap decision 「보내 보고 실패하면 죽은 것」 (send it and if it fails it is dead) does not hold
    /// here, and a bridge holding a dead window as its destination waits for the deadline on every envelope and then emits `TIMEOUT`.
    fn is_alive(&self, label: &str) -> bool;
}

/// Production implementation — sends only to the webview of that label.
///
/// ★★`emit_to` alone does not narrow it to one window — the receiving side must subscribe by its own label★★
/// Tauri wakes **every listener registered as `Any` regardless of the filter** (`match_any_or_filter`), and the JS
/// `listen()` default target is exactly `Any`. So the webview side hooks with `{ target: getCurrentWindow().label }`
/// (`src/commands/viewCommandBridge.ts`) — without that argument the same command runs as many times as there are windows and as many replies
/// come back. **These two lines are a pair.**
pub struct TauriViewDispatch(pub AppHandle);

impl ViewDispatch for TauriViewDispatch {
    fn deliver(&self, target: &str, request: &ViewCommandRequest) -> Result<(), String> {
        self.0
            .emit_to(target, EVT_COMMAND_REQUEST, request)
            .map_err(|e| format!("웹뷰 '{target}' 로 명령을 보내지 못했다: {e}"))
    }

    /// ★Use `get_webview_window`, not `get_window`★ — the former is behind tauri's `unstable` feature,
    /// which this shell cannot turn on (growing features is not this piece's decision). Every window of this app is a single-webview
    /// window (the two static ones in `tauri.conf.json` + popouts made with `WebviewWindowBuilder`), so the two labels are
    /// the same value, and so the value kept with `Window::label()` is looked up here as is. If a window with two webviews
    /// is made, this equality breaks — then the keeping side and the lookup side must be moved together.
    fn is_alive(&self, label: &str) -> bool {
        self.0.get_webview_window(label).is_some()
    }
}

// ── Reserved names ────────────────────────────────────────────────────────────────

/// Names the webview cannot take = those the shell's own table answers.
///
/// ★Taken **from the declarations**, not a hand-written list★ — this net grows along with the shell-table vocabulary.
/// ★It counts **declarations** (not what is plugged into the table)★ — the safe direction: if the webview took a name that is only declared and not plugged in,
/// the moment that is plugged in later, delivery would silently move to the shell (`route` looks at the table first).
/// ★It does not include names the daemon answers★ — that wall is the daemon's registration bounce (module header). The shell does not
/// depend on the agent crate, so it cannot even see that declaration (ADR-0270 decision 1).
/// ★Do not remove this net★ — the daemon bounce looks only at names the daemon answers, and the registry folds the same name within one packet without an error
/// (`register` in command `roster.rs`). This alone blocks the same name in shell and webview.
// ADR-0270
pub fn reserved_names() -> BTreeSet<String> {
    crate::layout::commands::COMMAND_SPECS
        .iter()
        .map(|spec| spec.name.to_string())
        .collect()
}

// ── Bridge ─────────────────────────────────────────────────────────────────────

/// How this report changed the list — material for the caller to decide whether to send a diff registration.
#[derive(Debug, Default, PartialEq)]
pub struct ReportOutcome {
    /// The whole webview share after the report.
    pub accepted: Vec<CommandDecl>,
    /// What was not in the previous list.
    pub added: Vec<CommandDecl>,
    /// What was there before but is not carried this time.
    pub removed: Vec<String>,
    /// What was dropped for being a reserved name (for the log — not a failure to the reporting side).
    pub refused: Vec<String>,
}

impl ReportOutcome {
    /// Is there a diff to send to the daemon.
    pub fn changed(&self) -> bool {
        !self.added.is_empty() || !self.removed.is_empty()
    }
}

/// Name → shape (an opaque string — the daemon does not open it, ADR-0156).
type Shapes = BTreeMap<String, String>;

/// Keep the per-window reports as they are — ★do not merge them★.
///
/// ★The advertised list and the window that receives envelopes **must come from the same window**★: if merged into one, a popout's report
/// swaps the list while delivery keeps going to main, so the daemon advertises B and main runs A — an advertised
/// name goes out as `UNKNOWN_COMMAND` or a name that runs is not in the advertisement. Today every window uploads the same static
/// `contributions`, so the two sets are byte-identical, but **no contract is laid on that coincidence.**
#[derive(Default)]
struct Registered {
    /// Window label → everything that window reported.
    reports: BTreeMap<String, Shapes>,
    /// The window to receive envelopes = **the owner of the advertised list**. `None` = there is no live reporter.
    host: Option<String>,
}

impl Registered {
    /// Remove dead reporters from the list — ★the liveness decision is taken **from outside**★ (lock discipline = [`ViewCommandBridge::prune_dead`]).
    fn forget(&mut self, dead: &[String]) {
        for label in dead {
            self.reports.remove(label);
        }
    }

    /// Pick the destination again — ★main first · keep a live host · last resort is the first reporter in lexicographic order★.
    ///
    /// **Why pick again:** if the host is decided once and that is the end, there is no recovery path after that window closes. The real path —
    /// main's single report fails (that invoke has no retry) and a popout becomes host, and then that popout closes,
    /// after which every delivery goes to a dead label, waits for the deadline, and becomes `TIMEOUT`. The name
    /// remains in the registry as is, so the state 「있는데 영영 안 되는」 (present but never working) hardens for the whole connection.
    /// **Why main first:** every window runs the same App and all of them report (`src/App.tsx`), so following the last report would make
    /// the destination jump to popouts and wobble every time that window closes. Closing main only hides it
    /// (`CloseRequested` in `lib.rs`), so its lifetime equals the app's.
    /// - ★**That premise has not been measured yet**★: to check 「숨은 main 이 웹뷰 표에 남는가」 (whether a hidden main stays in the webview table), an automation
    ///   harness would have to hide the window, but this app's permission settings block `hide()` · `close()`, so it could not be reached by GUI
    ///   (2026-08-23).
    ///
    /// **Why the last resort does not ask about visibility:** there is no static window the config hides (ADR-0225). If such a window is brought back, picking the first
    /// survivor in lexicographic order could make it the destination — then `tab.next` would pass the tabs of a window no one is looking at **while answering success**.
    /// Even if filtered then, the main of a `--hidden` boot must not be excluded (that window is opened via the tray, so hidden is
    /// its state, not its nature).
    fn repick(&mut self) {
        let keep = self
            .host
            .as_deref()
            .filter(|label| self.reports.contains_key(*label));
        if keep == Some(MAIN_WINDOW_LABEL) {
            return;
        }
        self.host = match self.reports.contains_key(MAIN_WINDOW_LABEL) {
            true => Some(MAIN_WINDOW_LABEL.to_string()),
            // Do not swap out a live host for no reason — the place moves only when it has died.
            false => keep
                .map(str::to_string)
                .or_else(|| self.reports.keys().next().cloned()),
        };
    }

    /// The list to advertise now = **what the host window reported**.
    fn advertised(&self) -> Shapes {
        self.host
            .as_ref()
            .and_then(|label| self.reports.get(label))
            .cloned()
            .unwrap_or_default()
    }
}

/// Holds the webview share of registration·delivery·correlation in one place.
pub struct ViewCommandBridge {
    dispatch: Arc<dyn ViewDispatch>,
    reserved: BTreeSet<String>,
    deadline: Duration,
    state: Mutex<Registered>,
    /// The gate that binds one report's **state change and its diff send** into one block (FIX: reversal of two reports).
    ///
    /// ★Without it the daemon and the bridge diverge★: after report A adds a name and report B removes it, if B's send enters the queue **before** A's,
    /// the daemon is left holding the union of the two (A's `added` arrives later). The bridge knows only B's
    /// set, yet A's name is alive in the registry, and if that name is delivered the webview answers that it does not know it.
    /// A field to give the order cannot be added to the wire (contract unchanged), so **the sending side is serialized**.
    outbound: tokio::sync::Mutex<()>,
    pending: Arc<Pending>,
}

/// One seat waiting for a reply — it also holds **whom it was sent to**.
///
/// ★Writing the destination here is the point★: the party to check against when an outcome comes in is not **the host at that time** but
/// the window that actually received this envelope. The host can change by report order (main overwrites the popout), so
/// checking against the current host would bounce the real reply, and conversely a forgery from the old host would pass.
struct Waiting {
    /// The identity of this seat — distinguishes it from the **next** seat that uses the same `request_id` (reason = [`PendingSlot::drop`]).
    seat: u64,
    target: String,
    answer: oneshot::Sender<Result<serde_json::Value, CommandError>>,
}

/// The seats waiting for replies — ★why `Arc`★: the future the deadline timer runs in is `'static`, so it cannot borrow the bridge,
/// yet when the time passes it must **clear its own seat directly** (if not cleared, one dead window grows the map permanently).
#[derive(Default)]
struct Pending {
    seats: Mutex<HashMap<RequestId, Waiting>>,
    /// A number drawn anew for each seat — never rewound.
    next_seat: AtomicU64,
}

impl ViewCommandBridge {
    pub fn new(dispatch: Arc<dyn ViewDispatch>) -> Self {
        Self::with_reserved(dispatch, VIEW_REPLY_DEADLINE, reserved_names())
    }

    /// For the harness — gives the reserved set · deadline directly. ★In production use [`reserved_names`]★ (a hand-written list falls behind).
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
    ///
    /// ★★The liveness lookup is run **outside my lock** — do not reverse this order★★ (ADR-0006 「락 보유 중 외부 호출
    /// 금지」): the real lookup takes Tauri's global webview table lock and per-window locks. There is no path now that re-enters this bridge
    /// while holding that lock, but once the order stands, a change outside this file could silently reverse that fact, and
    /// then it is a deadlock. So it is split by **① snapshot only the labels inside the lock → ② look up outside the lock → ③ take the lock again and apply**
    /// into three steps.
    /// ★The window may die in between or a new report may come in★ — a label that was not in the snapshot is not cleared this time
    /// (the next call catches it), and clearing an already vanished label is harmless. Recovery being one beat late is the contract.
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

    /// One window reports its own list at boot — it is **that window's whole set** (not a diff).
    ///
    /// ★Reserved names are dropped quietly but not passed over quietly★ — if bounced, the remaining names could not go up either, and if nothing
    /// were said, the cause of 「등록했는데 안 불린다」 (registered but never called) would be left nowhere. So it drops them and returns them via [`ReportOutcome::refused`]
    /// (the caller logs them).
    /// ★Entries with an empty shape are dropped too★ — `help` is an opaque string to the daemon, so registration succeeds even if empty, but a caller that
    /// discovers that name has no material to fill in arguments and cannot call it (that is why ADR-0156 removed this round trip).
    /// ★The diff is the before/after difference of the **advertised set** — not that window's list★: even if a non-host window reports, if the advertisement
    /// stays the same there is nothing to send (the coupling rule of [`Registered`] above guarantees it).
    /// ★★Only the harness calls this directly★★ — the production path is [`ViewCommandBridge::report_and_push`],
    /// which binds the state change and the diff send into one gate (reason = the `outbound` field). Calling only this gives no such order
    /// guarantee.
    ///
    /// ## ★What remains — the names of a dead host stay in the registry **until reconnect** (a known residual)★
    /// If the lookup path ([`ViewCommandBridge::prune_dead`]) clears the dead host first, the `previous` this function then sees
    /// is an **already shrunk** set — so the vanished names are not sent out as `removed`. They stay as they are in the daemon
    /// registry, and calling one reaches the shell and comes back as 「모르는 이름」 (unknown name).
    /// ★The only resolution is the whole registration of a reconnect, and **nothing schedules that reconnect**★ — do not read it as 「다음 재연결이
    /// 곧 고친다」 (the next reconnect will fix it soon). To fix it, the delivery path would have to be able to emit a diff, and for that this crate would
    /// have to know the daemon link (a dependency we decided not to carry — module header).
    pub fn report(&self, label: &str, reported: Vec<ViewCommandDecl>) -> ReportOutcome {
        let mut refused = Vec::new();
        let mut next: Shapes = BTreeMap::new();
        for decl in reported {
            // A reserved name · an empty description · **no marker** — all three are dropped at the same spot (the reason for each is in the doc above and
            //   [`ViewEffect`]).
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

        // The liveness lookup runs first **outside the lock** (reason = `prune_dead`) — so that what is inside the single lock below becomes pure.
        self.prune_dead();
        let mut state = sync::lock(&self.state);
        let previous = state.advertised();
        state.reports.insert(label.to_string(), next);
        // ★Pick again **after** putting in the report★ — a window that just came up must be able to take over from a dead host.
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

    /// Production path — apply the report and, if anything changed, send the diff out **within the same gate**.
    ///
    /// `push(added, removed)` is the round trip out to the daemon (`commands/view_bus.rs` builds `UpdateCommands`) — this crate does not know that wire, so
    /// the caller injects it.
    /// ★The point is that the gate covers both 「변경 + 송신」 (change + send)★: if only the send were serialized, two reports would flip the state first and only
    /// the diffs would go out in order, so the later report's diff would be computed against the old state (reason = the `outbound` field).
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

    /// Attach the outcome the webview sent to that round trip — `Err` if there is no seat (a late reply · a second reply · someone else's key).
    ///
    /// `from` = the window that sent the outcome (the value Tauri puts into the invoke — `commands/view_bus.rs`).
    /// ★Only the window that received the envelope may answer★: if opened by the correlation key alone, **another window could end another's round trip** —
    /// the caller would receive that forged outcome, and while the real window's own reply is bounced the side effect happens just the same.
    /// The reason the party checked against is not 「지금 host」 (the current host) but **the window that actually received that envelope** is in [`Waiting`].
    /// ★Even if the check fails, the seat is left as is★ — if removed, a single forgery would erase the real reply's seat.
    ///
    /// ★A second reply is not swallowed quietly★: that situation means two windows received the same envelope (a sign that the delivery target is
    /// no longer one), so the caller must be able to log it.
    pub fn settle(
        &self,
        from: &str,
        request_id: &str,
        outcome: Result<serde_json::Value, CommandError>,
    ) -> Result<(), String> {
        let parsed = uuid::Uuid::parse_str(request_id.trim())
            .map(RequestId)
            .map_err(|_| format!("request_id 가 UUID 가 아니다: {request_id:?}"))?;
        let mut waiting = sync::lock(&self.pending.seats);
        let Some(slot) = waiting.get(&parsed) else {
            return Err(format!(
                "기다리는 왕복이 없다(이미 답했거나 마감을 넘겼다): {parsed}"
            ));
        };
        if slot.target != from {
            return Err(format!(
                "이 왕복은 '{}' 로 내려갔는데 '{from}' 이 답했다 — 결말을 붙이지 않는다: {parsed}",
                slot.target
            ));
        }
        let slot = waiting.remove(&parsed).expect("바로 위에서 본 자리다");
        drop(waiting);
        slot.answer
            .send(outcome)
            .map_err(|_| format!("답장을 받을 쪽이 이미 사라졌다: {parsed}"))
    }

    /// The window to receive envelopes now and the list that window advertises — ★this is the answer after filtering out dead reporters★.
    ///
    /// Advertisement (registration packet) · lookup (delivery stage 2) · delivery **all pass through this one** — if the three places each read state, the moment
    /// any one sees the old host, advertisement and execution diverge.
    fn live_host(&self) -> Option<(String, Shapes)> {
        self.prune_dead();
        let state = sync::lock(&self.state);
        let label = state.host.clone()?;
        Some((label, state.advertised()))
    }

    /// The window currently held as the destination (for diagnostics · tests).
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
    /// Unfold into one catalog entry — ★it uses the **same field names** as what the Rust declaration emits★
    /// (`engram_dashboard_command::spec_item_json`). If they diverge, the shapes in one registry mix as two dialects, and the LLM that
    /// reads them has to use a different reading for each command.
    ///
    /// `effect` uses the value the webview carried as is (reason = [`ViewEffect`]). The other three are decided here, and each is
    /// true:
    /// - `since: 1` — a name coming up this path has no generation. To really count generations of the webview vocabulary, a
    ///   catalog number would have to stand on that side, and rather than invent a number we pin it to 「첫 세대」 (first generation).
    /// - `ok: {}` — an empty schema means 「무엇이든」 (anything). The return of webview `run` varies per command (void · id · Promise)
    ///   and has never been fixed by a contract — instead of inventing a shape, we write that it is unknown.
    /// - `errors` — the **whole set** this bridge can actually emit (the branches of [`ViewCommandBridge`]): the webview
    ///   failed (`INTERNAL`) · the deadline passed (`TIMEOUT`) · there is no window to send to (`UNSUPPORTED`) · the same number
    ///   is already running (`REQUEST_ID_CONFLICT`). ★Do not emit a code that is not in the advertisement★ — callers build their branching from this list,
    ///   so if an unlisted code arrives it falls to that branching's default branch (usually 「모르는 실패」, unknown failure).
    ///   ★An error thrown by the webview cannot be told apart by kind (a known limitation)★ — an argument mistake and a real failure both go out as `INTERNAL` + text.
    ///   To tell them apart the webview would have to carry typed codes, and that is not this piece's decision.
    ///
    /// ★`effect` is **taken as an argument** — the `Option` in `self` is not read★: if read here, the question 「값이 없으면
    /// 무엇으로 하나」 (what do we use if there is no value) comes back to life, and whichever answer is picked it is a false advertisement ([`ViewEffect`]).
    /// The caller ([`ViewCommandBridge::report`]) has already peeled that field off, so only the value comes here.
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
        // Serialization cannot fail (everything is a JSON value). Still, it folds to an empty string rather than killing the packet —
        //   an empty `help` was already dropped by the report filter, so reaching here means the contract above is broken, and the log carries that signal.
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
        // ★`required` keeps only declared fields★ — if a nonexistent field is put in required, that schema is satisfied by no arguments at all,
        //   so the caller can never call it (Rust-side `lint_spec` blocks the same thing beside compilation).
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
    /// ★What is advertised is **the list of the live host window**★ — it is picked again on every reconnect, so names held by a dead window
    ///   do not go out in the next registration packet.
    fn declarations(&self) -> Vec<CommandDecl> {
        self.live_host()
            .map(|(_, shapes)| decls_of(&shapes))
            .unwrap_or_default()
    }

    fn lookup(&self, name: &str) -> OwnerLookup {
        // ★If there is no destination window it is 「모르는 이름」 (unknown name)★ — if we answered that there is an owner when there is nowhere to send, delivery would
        //   go as far as `link.send` and produce an error, and the caller could not tell the two apart.
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
        // ★Hang the seat **before** sending★ — the reply coming by invoke may arrive before the emit returns, and then,
        //   if there is no seat, that reply is dropped as 「기다리는 왕복이 없다」 (no round trip is waiting).
        // ★If the same number is already running, **do not send**★ — for the reason, see `PendingSlot::hold`.
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
            // `slot` is dropped here and takes the seat back.
            return ready(CommandReply::err(
                request_id,
                CommandError::of(ErrorCode::Internal, detail),
            ));
        }

        let deadline = self.deadline;
        Box::pin(async move {
            // ★Carried into the future★ — whether by deadline or cancellation (dropping this future discards the round trip as is,
            //   the cancellation meaning of `route`), the path that takes the seat back converges on a single `Drop`.
            let _slot = slot;
            match tokio::time::timeout(deadline, wait).await {
                Ok(Ok(outcome)) => CommandReply {
                    request_id,
                    outcome,
                },
                // The sender vanished without an answer — only the case where `settle` took the seat away but could not put in a value.
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

/// The lifetime of one reply seat — ★the clearing path is a single `Drop`★.
///
/// If seats were cleared by hand, a missed branch would grow the map permanently. There really are four branches: delivery failure · normal
/// reply (`settle` already took it) · deadline · **the case where this future itself is dropped** (cancellation — that meaning of `route`).
/// The last one is a spot that cannot be reached by hand, so without it a seat is left every time the runtime shuts down.
struct PendingSlot {
    pending: Arc<Pending>,
    request_id: RequestId,
    /// My seat's identity — the only material that makes [`PendingSlot::drop`] **not clear another's seat**.
    seat: u64,
}

impl PendingSlot {
    /// Take a seat — ★if the number is already running it is `None`, and **a live waiter is not pushed out**★.
    ///
    /// If covered quietly, the same command runs twice: if the same number comes again while the first round trip is still running in the webview,
    /// the envelope goes down one more time (side effect twice), the first waiter is dropped without receiving an answer, and the **old**
    /// outcome that arrived first attaches as the new attempt's answer. The refusal blocks all three at once.
    /// ★Even so, it is normal that this situation itself does not arise★ — for a retry to reach this seat, the outer deadline would have to
    /// end before the inner one, and the inequality of [`VIEW_REPLY_DEADLINE`] blocks that order. This refusal is the second net for when that
    /// inequality breaks.
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
    /// ★Clear **only when it is my seat**★ — even with the same number, if the seat differs it is another's.
    ///
    /// The interleave it blocks: `settle` takes X out and sends the answer → after a new envelope takes a new seat with the same X → this guard of the old
    /// future drops and evicts the **new seat**, and that waiter fails with a closed channel.
    /// ★Today that interleave does not arise — still we block it by identity★: the guard drops at the end of the async block, and that
    /// moment is **before** the outcome reaches the daemon, so the daemon has no window to reopen the same number. But that argument hangs on
    /// the properties of **two modules** — 「가드가 언제 떨어지나」 (when the guard drops) · 「데몬이 언제 번호를 재사용하나」 (when the daemon reuses numbers) — so if either
    /// changes it silently collapses (and there is no reason for that change to touch this file). The identity comparison makes that argument itself
    /// unnecessary.
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
