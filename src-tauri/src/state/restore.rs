//! 「Restore?」 after an abnormal exit (TRD S21-storage §6-7): three states (`none` · `awaiting` · `answered`) and a marker for the one answer being processed; the restore service ([`RestoreService`]) holding this run's `state.json` read result ([`StateFileStatus`], §6-5); and the coordinator ([`RestoreCoordinator`]) executing the answer on top of it. The state is decided once by boot step 6 ([`RestoreService::set_boot`], I5); the answer goes through the single [`RestoreCoordinator::answer`] (human and LLM alike).
//! - The service lock is a leaf: while holding it, no notification · log · recorder wait · other lock (`ViewManager`); call [`RestoreNotifier`] after releasing.
//! - Dropping the marker unfinished equals `RolledBack` (also on panic release) — a lingering in-progress marker would make every later answer `InFlight` for the whole run.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use engram_dashboard_base::sync;
use ts_rs::TS;

use super::boot_plugin::{report_restore_warning, ResolveResult, StateSession};
use super::convert::{RestoreWarning, Restored};
use super::placement::{land, Fallback, Landing, MonitorArea};
use super::schema::{StateFile, WindowKind};
use crate::layout::manager::WindowLabel;
use crate::layout::{
    LabelSource, LayoutEvents, LayoutState, SubscriptionSync, ViewManager, ViewSnapshot,
    WindowAttrs, WindowBounds, WindowTabsPayload, MAIN_WINDOW_LABEL,
};
use crate::theme::ThemeControl;

pub const EVT_RESTORE_CHANGED: &str = "restore:changed";

/// The `crash_copy` field of `restore.status` and the payload of [`EVT_RESTORE_CHANGED`] (TRD §6-7) — the wire spelling is `"none"` ·
/// `"awaiting"` · `"answered"`.
///
/// - `None` = there is no copy to ask about in this run — it is this value even if a copy this version cannot apply (version too new — N6) is on
///   disk.
/// - `Awaiting` = there is an unanswered copy — the human UI blocks main until it is answered.
/// - `Answered` = answered in this run — the copy file may remain on disk until the recorder deletes it (I2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CrashCopyStatus {
    None,
    Awaiting,
    Answered,
}

impl CrashCopyStatus {
    /// The serde spelling as is — the bus `restore.status` carries `crash_copy` as a string because the declaration macro cannot carry the enum in lowercase.
    pub fn as_wire(self) -> &'static str {
        match self {
            CrashCopyStatus::None => "none",
            CrashCopyStatus::Awaiting => "awaiting",
            CrashCopyStatus::Answered => "answered",
        }
    }
}

/// The `state_file` field of `restore.status` (TRD §6-5) — how this run's boot read `state.json`. Decided by boot step 6,
/// unchanged throughout that run, and always a value regardless of the crash copy. The wire spelling is `"ok"` · `"unreadable"` ·
/// `"newer"` · `"corrupt_copied_aside"` · `"corrupt_not_copied"`.
///
/// - `Ok` = read, or the file did not exist. It is also this value when the copy could not be made and this run does not save (guard ii) — this field
///   carries only the `state.json` read. Whether this run saves is stated by [`RestoreStatusView::saves`], not by this field.
/// - `Unreadable` = the read itself failed (after lock retries · permissions — guard i). The file is left as is, this run saves no
///   screen state at all, and the next boot decides again.
/// - `Newer` = the file was written by a newer version than this shell, so it started with the default screen (the same field as guard i). It neither copies it aside nor overwrites it, this
///   run saves no screen state at all, and the next boot decides again.
/// - `CorruptCopiedAside` = an unusable file (corrupt · over the limit · not UTF-8), so it was copied aside as `state.json.corrupt` and started with the default
///   screen.
/// - `CorruptNotCopied` = an unusable file (corrupt · over the limit · not UTF-8) that could not be copied aside, and it started with the default screen —
///   the original is overwritten with no backup (D8). An existing `state.json.corrupt` was copied aside by an earlier start and is not
///   a backup of this original.
// ADR-0291 R2: a newer version is `Newer`, not the two corrupt values (it neither copies aside nor overwrites).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum StateFileStatus {
    #[default]
    Ok,
    Unreadable,
    Newer,
    CorruptCopiedAside,
    CorruptNotCopied,
}

impl StateFileStatus {
    pub fn as_wire(self) -> &'static str {
        match self {
            StateFileStatus::Ok => "ok",
            StateFileStatus::Unreadable => "unreadable",
            StateFileStatus::Newer => "newer",
            StateFileStatus::CorruptCopiedAside => "corrupt_copied_aside",
            StateFileStatus::CorruptNotCopied => "corrupt_not_copied",
        }
    }
}

#[derive(Clone)]
pub struct CrashCopy {
    /// The original text acceptance will restore — the copy bytes boot read, or for guard ii (a copy that should have been made could not be) the original `state.json` text held in memory (TRD §6-5 ⑥ · L2).
    pub text: String,
    pub hash: String,
    pub file: StateFile,
}

// Do not print the copy's original text (≤4 MiB) — only the length and hash.
impl fmt::Debug for CrashCopy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CrashCopy")
            .field("text_len", &self.text.len())
            .field("hash", &self.hash)
            .field("windows", &self.file.windows.len())
            .finish()
    }
}

/// The answer for `restore_status` · `restore.status`. `saved_at_ms` · `windows` · `tabs` · `durable` are values only when `Awaiting`
/// and `null` otherwise. `saves` · `state_file` are always values regardless of the crash copy, and after boot step 6 decides them they do not change
/// throughout that run (even when answered · even when rolled back).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[ts(export)]
pub struct RestoreStatusView {
    pub crash_copy: CrashCopyStatus,
    /// The time the copy was written — Unix milliseconds.
    // ts-rs's default mapping for u64 is bigint, but serde_json carries it as a number.
    #[ts(type = "number | null")]
    pub saved_at_ms: Option<u64>,
    /// The number of windows in the copy — main + popouts (counted the same as [`AnswerReply::restored_windows`]).
    pub windows: Option<u32>,
    /// The sum of tab counts of all windows in the copy.
    pub tabs: Option<u32>,
    /// The same value as [`Self::saves`] while asking — `true` = not a guard: it tries to attach the answer (accept · decline) to disk and delete the copy
    /// (success is not guaranteed — whether it actually attached is told by [`AnswerReply::durable`]). `false` = a guard:
    /// even if answered, the crash-time screen stays on disk (a copy or a `state.json` without a clean-exit mark) and the next boot asks again.
    pub durable: Option<bool>,
    /// Whether this run saves screen state — decided by the boot guard (TRD §6-5 ③). `true` = not a guard: it tries to start the recorder
    /// (starting · writing is not guaranteed to succeed — a run where the recorder could not be started at ⑦ is also `true`). `false` = a guard: this
    /// run saves no screen state at all and the next boot decides again — guard i (`state_file` is `Unreadable` ·
    /// `Newer`) · guard ii (the crash copy that should have been made could not be — `state_file` is `Ok`).
    pub saves: bool,
    pub state_file: StateFileStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerConflict {
    NotAwaiting(CrashCopyStatus),
    /// Another answer is being processed (TRD §6-7 「CONFLICT if another answer is being processed」).
    InFlight,
}

/// Marker of the one answer being processed — not cloneable. Finish with [`RestoreService::finish_answer`] (dropping rolls it back — module header).
pub struct AnswerTicket {
    cell: Arc<Mutex<Cell>>,
    copy: Arc<CrashCopy>,
    generation: u64,
    settled: bool,
}

impl AnswerTicket {
    pub fn copy(&self) -> &CrashCopy {
        &self.copy
    }
}

// Do not print the copy's original text (≤4 MiB).
impl fmt::Debug for AnswerTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnswerTicket")
            .field("hash", &self.copy.hash)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl Drop for AnswerTicket {
    fn drop(&mut self) {
        if !self.settled {
            tracing::warn!(
                module = "state",
                hash = %self.copy.hash,
                "the restore answer vanished without finishing — reverting the in-progress marker"
            );
            settle(
                &self.cell,
                self.generation,
                &self.copy.hash,
                AnswerEnd::RolledBack,
            );
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerEnd {
    Answered,
    /// The acceptance failed before commit (preparation · window creation) — only the in-progress marker is released and it stays `Awaiting`. It still notifies `Awaiting`: a caller rejected with `InFlight` meanwhile needs it to know it can answer again.
    RolledBack,
}

/// Called after the boot ⑥ decision · `Answered` · a rollback (still `Awaiting`), holding no lock — the implementation may call the service again. For the rollback of a dropped marker it runs inside `Drop` (possibly a panic release).
pub trait RestoreNotifier: Send + Sync {
    fn changed(&self, status: CrashCopyStatus);
}

pub struct RestoreService {
    cell: Arc<Mutex<Cell>>,
}

struct Cell {
    phase: Phase,
    generation: u64,
    notifier: Option<Arc<dyn RestoreNotifier>>,
    state_file: StateFileStatus,
    saves: bool,
}

enum Phase {
    None,
    Awaiting {
        copy: Arc<CrashCopy>,
        in_flight: bool,
    },
    Answered,
}

impl Phase {
    fn status(&self) -> CrashCopyStatus {
        match self {
            Phase::None => CrashCopyStatus::None,
            Phase::Awaiting { .. } => CrashCopyStatus::Awaiting,
            Phase::Answered => CrashCopyStatus::Answered,
        }
    }
}

fn lock(cell: &Mutex<Cell>) -> MutexGuard<'_, Cell> {
    // Inside the lock only field assignments — no state half-changed by a panic; the marker rollback must work in `Drop` too.
    sync::lock(cell)
}

fn notify(notifier: Option<Arc<dyn RestoreNotifier>>, status: CrashCopyStatus) {
    if let Some(notifier) = notifier {
        notifier.changed(status);
    }
}

enum Settled {
    Answered(Option<Arc<dyn RestoreNotifier>>),
    RolledBack(Option<Arc<dyn RestoreNotifier>>),
    Stale,
}

fn settle(cell: &Mutex<Cell>, generation: u64, hash: &str, end: AnswerEnd) {
    let settled = {
        let mut cell = lock(cell);
        let in_flight = matches!(
            cell.phase,
            Phase::Awaiting {
                in_flight: true,
                ..
            }
        );
        if cell.generation != generation || !in_flight {
            Settled::Stale
        } else {
            match end {
                AnswerEnd::Answered => {
                    cell.phase = Phase::Answered;
                    Settled::Answered(cell.notifier.clone())
                }
                AnswerEnd::RolledBack => {
                    if let Phase::Awaiting { in_flight, .. } = &mut cell.phase {
                        *in_flight = false;
                    }
                    Settled::RolledBack(cell.notifier.clone())
                }
            }
        }
    };
    match settled {
        // The one result line of an answer is left by whoever executed that answer (`RestoreCoordinator::answer` — whether accepted · whether it attached to disk).
        Settled::Answered(notifier) => notify(notifier, CrashCopyStatus::Answered),
        Settled::RolledBack(notifier) => {
            tracing::info!(
                module = "state",
                hash = %hash,
                "reverted the restore answer — it can be answered again"
            );
            notify(notifier, CrashCopyStatus::Awaiting);
        }
        Settled::Stale => tracing::warn!(
            module = "state",
            hash = %hash,
            ?end,
            "a restore answer marker that does not belong to the current state — ignoring it"
        ),
    }
}

impl Default for RestoreService {
    fn default() -> Self {
        Self::new()
    }
}

impl RestoreService {
    /// State `None` · `state_file` `Ok` · `saves` `true` · no notification — windows come into being after ⑥ and never see this pre-⑥ value (I5).
    pub fn new() -> Self {
        Self {
            cell: Arc::new(Mutex::new(Cell {
                phase: Phase::None,
                generation: 0,
                notifier: None,
                state_file: StateFileStatus::Ok,
                saves: true,
            })),
        }
    }

    pub fn set_notifier(&self, notifier: Arc<dyn RestoreNotifier>) {
        lock(&self.cell).notifier = Some(notifier);
    }

    /// Called only at boot step 6 (I5). `copy` `None` = no copy to ask about; `saves` = whether this run saves screen state ([`RestoreStatusView::saves`]). An in-progress marker becomes invalid.
    pub fn set_boot(&self, copy: Option<CrashCopy>, state_file: StateFileStatus, saves: bool) {
        let hash = copy.as_ref().map(|copy| copy.hash.clone());
        let (again, status, notifier) = {
            let mut cell = lock(&self.cell);
            let again = cell.generation > 0;
            cell.generation += 1;
            cell.state_file = state_file;
            cell.saves = saves;
            cell.phase = match copy {
                Some(copy) => Phase::Awaiting {
                    copy: Arc::new(copy),
                    in_flight: false,
                },
                None => Phase::None,
            };
            (again, cell.phase.status(), cell.notifier.clone())
        };
        if again {
            tracing::warn!(
                module = "state",
                "the restore state was set again outside the boot stage — an answer in progress becomes void"
            );
        }
        match hash {
            Some(hash) => tracing::info!(
                module = "state",
                hash = %hash,
                saves,
                "there is an unanswered crash copy — asking whether to restore"
            ),
            None => tracing::info!(module = "state", "no crash copy to ask about"),
        }
        notify(notifier, status);
    }

    pub fn status(&self) -> RestoreStatusView {
        let (status, copy, state_file, saves) = {
            let cell = lock(&self.cell);
            let copy = match &cell.phase {
                Phase::Awaiting { copy, .. } => Some(Arc::clone(copy)),
                _ => None,
            };
            (cell.phase.status(), copy, cell.state_file, cell.saves)
        };
        let Some(copy) = copy else {
            return RestoreStatusView {
                crash_copy: status,
                saved_at_ms: None,
                windows: None,
                tabs: None,
                durable: None,
                saves,
                state_file,
            };
        };
        let strips: Vec<usize> = copy
            .file
            .windows
            .iter()
            .map(|window| match &window.kind {
                WindowKind::Main(strip) | WindowKind::Popout(strip) => strip.tabs.len(),
            })
            .collect();
        RestoreStatusView {
            crash_copy: status,
            saved_at_ms: Some(copy.file.saved_at_ms),
            windows: Some(count(strips.len())),
            tabs: Some(count(strips.iter().sum())),
            durable: Some(saves),
            saves,
            state_file,
        }
    }

    pub fn begin_answer(&self) -> Result<AnswerTicket, AnswerConflict> {
        let mut cell = lock(&self.cell);
        let generation = cell.generation;
        match &mut cell.phase {
            Phase::Awaiting { copy, in_flight } if !*in_flight => {
                *in_flight = true;
                Ok(AnswerTicket {
                    cell: Arc::clone(&self.cell),
                    copy: Arc::clone(copy),
                    generation,
                    settled: false,
                })
            }
            Phase::Awaiting { .. } => Err(AnswerConflict::InFlight),
            phase => Err(AnswerConflict::NotAwaiting(phase.status())),
        }
    }

    pub fn finish_answer(&self, mut ticket: AnswerTicket, end: AnswerEnd) {
        debug_assert!(
            Arc::ptr_eq(&self.cell, &ticket.cell),
            "a marker issued by another service"
        );
        ticket.settled = true;
        settle(&ticket.cell, ticket.generation, &ticket.copy.hash, end);
    }
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

// ── Restore coordinator — runtime accept · decline (TRD §6-7 ①–⑤) ─────────────────────────────

/// The answer of `restore_answer` · `restore.answer` (TRD §6-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, TS)]
#[ts(export)]
pub struct AnswerReply {
    /// The number of windows redrawn with the copy's screen — main + the new popouts left in the model (windows closed before the screen changed drop out) · `0` for a decline.
    pub restored_windows: u32,
    /// A `state.json` carrying that answer was published within the deadline. `false` = it could not be confirmed that it attached to disk within the deadline —
    /// the next boot may ask again. If there is no recorder (guard) it always asks again, and on a write failure · deadline overrun the recorder may attach it in a later write
    /// while holding that hash (`saver` header 「resolution field」).
    pub durable: bool,
}

#[derive(Debug)]
pub enum AnswerError {
    Conflict(AnswerConflict),
    /// The shell is still starting up with no window port, so acceptance was refused — nothing changed, still `awaiting`; the same answer stands once setup ends → bus `INTERNAL` (same code as [`Self::Internal`], different wording: 「answer again soon」).
    NotReady,
    /// The acceptance failed before commit (preparation · window creation · commit rejection) — nothing changed, still `awaiting` → bus `INTERNAL`.
    Internal(String),
}

// The bus error `message` and the Tauri wrapper's error string — tell the kind apart by the code (bus) or `restore_status`, not by this text.
impl fmt::Display for AnswerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AnswerError::Conflict(AnswerConflict::NotAwaiting(CrashCopyStatus::Answered)) => {
                f.write_str("the crash copy was already answered in this run (crash_copy is answered)")
            }
            AnswerError::Conflict(AnswerConflict::NotAwaiting(_)) => {
                f.write_str("there is no crash copy to answer in this run (crash_copy is none)")
            }
            AnswerError::Conflict(AnswerConflict::InFlight) => f.write_str(
                "another answer is still being processed — read restore.status after it ends",
            ),
            AnswerError::NotReady => f.write_str(
                "the dashboard is still starting, so the screen was not restored and the crash copy still awaits an answer — answer again shortly",
            ),
            AnswerError::Internal(reason) => write!(
                f,
                "the screen was not restored and the crash copy still awaits an answer — answer again: {reason}"
            ),
        }
    }
}

/// OS window port for runtime restore acceptance (TRD §6-7 ② ④) — production = `placement::TauriRestoreWindows`, test = fake. All of it is called holding no lock: window creation · setters wait for the event loop, and `destroy`'s `Destroyed` handling (usage-interest notification) · `record_placement` · `set_shown` take the layout lock (same discipline as `layout::WindowHost`).
pub trait RestoreWindows: Send + Sync {
    fn app_has_focus(&self) -> bool;
    fn monitors(&self) -> Vec<MonitorArea>;
    /// Create popout `label` hidden and place it at `at` (saved normal placement · its monitor; `None` = the label's default). `Err` = no window (any deferral set was withdrawn too).
    /// `maximized` defers the copy's maximize until the window is first shown — maximizing a hidden window shows then hides it. While deferred the window's placement is not recorded ([`Self::record_placement`] · `Moved` · `Resized`); the show (`set_shown` `true` · the tray show) applies the maximize and clears it, and destruction clears it. Set the deferral before creating the window: if set after, a close of a window the tray 「Show」 revealed in between passes destruction cleanup before the deferral and it stays.
    fn open_hidden(
        &self,
        label: &str,
        at: Option<(WindowBounds, Landing)>,
        maximized: bool,
    ) -> Result<(), String>;
    fn visibility(&self, label: &str) -> Option<bool>;
    /// Apply the copy's normal placement (`at`; `None` = leave current) and maximize to main; maximize is followed both ways (`false` unmaximizes and clears the deferred maximize). A hidden main's maximize is deferred until first shown — set the deferral before applying the placement, else the `Moved` · `Resized` it emits are recorded as 「not maximized」. A visible main uses the same deferral, cleared only if applied: if it could not be applied it stays, main's placement is not recorded meanwhile (including [`Self::record_placement`]) and the next show applies it again.
    fn place_main(&self, at: Option<(WindowBounds, Landing)>, maximized: bool);
    /// Show (`true`) or hide, and tell usage interest — same order as the tray show · hide (ADR-0229 · `tray::actions::for_each_ui_window`). The show comes after that window's first paint (at once if already notified, else at the time limit — `first_paint`) and applies the deferred maximize; it activates and fronts the window (`placement::show_placed`). Hiding withdraws the first-paint wait. No window: nothing.
    fn set_shown(&self, label: &str, shown: bool);
    /// Record that window's current placement in the model — after the window is in the model (an earlier `Moved` is dropped). A window with a deferred maximize is not recorded.
    fn record_placement(&self, label: &str);
    /// Give that window focus, pulling it even if another app is in front. A hidden or minimized window stays unchanged (tao 0.35.3 Windows `set_focus`); no window: nothing.
    fn focus(&self, label: &str);
    fn destroy(&self, label: &str) -> Result<(), String>;
}

/// Looked up every time the subscription-recompute port is used (before taking the lock); `None` = no daemon client, skip the recompute. Looked up once at plug-in, a client registered later would be skipped silently.
pub trait SubscriptionSource: Send + Sync {
    fn current(&self) -> Option<Arc<dyn SubscriptionSync>>;
}

pub struct RestorePorts {
    pub windows: Arc<dyn RestoreWindows>,
    pub events: Arc<dyn LayoutEvents>,
    pub subs: Arc<dyn SubscriptionSource>,
    pub themes: ThemeControl,
}

/// The single path of runtime restore accept · decline (TRD §6-7 ①–⑤); the human (Tauri wrapper) and the LLM (bus `restore.answer`) call the same instance.
/// - One commit point (③, inside one `ViewManager` lock): a failure before it (① preparation · ② hidden window creation) withdraws created windows and rolls back to `awaiting`; a failure after it (④ theme push · notification · placement · show · removing vanished popouts · withdrawing old popouts) only logs.
/// - Restored popouts follow main's visibility (ADR-0229): kept hidden so none appears alone beside a hidden main, revealed together by the tray 「Show」; the copy's maximize applies at first show ([`RestoreWindows::open_hidden`]).
/// - Lock order: the service and session fields are leaves; window ports · notifications · recorder waits hold no lock.
/// - Ports ([`RestorePorts`]) are made in the builder and received with [`Self::attach`] at the end of shell setup; an acceptance before that is [`AnswerError::NotReady`] (still `awaiting`), a decline stands without ports.
pub struct RestoreCoordinator {
    service: Arc<RestoreService>,
    layout: LayoutState,
    session: Arc<StateSession>,
    /// The same issuer as runtime popouts, so a new label does not collide with a window that is up (§6-3).
    labels: Arc<dyn LabelSource>,
    ports: OnceLock<RestorePorts>,
}

struct Prepared {
    layout: ViewManager,
    main: WindowAttrs,
    // The new popouts in label issue order — that window's copy attributes.
    popouts: Vec<(WindowLabel, WindowAttrs)>,
}

struct Committed {
    removed: Vec<WindowLabel>,
    layouts: Vec<ViewSnapshot>,
    tabs: Vec<WindowTabsPayload>,
}

struct Desktop {
    monitors: Vec<MonitorArea>,
    app_had_focus: bool,
}

impl RestoreCoordinator {
    pub fn new(
        service: Arc<RestoreService>,
        layout: LayoutState,
        session: Arc<StateSession>,
        labels: Arc<dyn LabelSource>,
    ) -> Self {
        Self {
            service,
            layout,
            session,
            labels,
            ports: OnceLock::new(),
        }
    }

    pub fn attach(&self, ports: RestorePorts) {
        if self.ports.set(ports).is_err() {
            tracing::warn!(
                module = "state",
                "tried to plug in the restore coordinator's port twice — using the first one"
            );
        }
    }

    pub fn status(&self) -> RestoreStatusView {
        self.service.status()
    }

    /// Answer the crash copy: `accept` replaces the screen with the copy (①–④); both settle the answer and attach it to the recorder (⑤).
    /// Blocks — not for the main (event loop) thread: window create · place · withdraw wait for the event loop (called from main it hangs on itself) and the recorder's answer is awaited until the deadline ([`saver::REPLY_DEADLINE`](super::saver::REPLY_DEADLINE), 2 s). The caller hands it off with an async command + `spawn_blocking`. Call it holding no lock.
    pub fn answer(&self, accept: bool) -> Result<AnswerReply, AnswerError> {
        let ticket = self.service.begin_answer().map_err(AnswerError::Conflict)?;
        let hash = ticket.copy().hash.clone();
        let restored_windows = if accept {
            let Some(ports) = self.ports.get() else {
                tracing::warn!(
                    module = "state",
                    hash = %hash,
                    "the shell is still starting, so the restore accept was not taken — the screen is unchanged and it can be answered again"
                );
                self.service.finish_answer(ticket, AnswerEnd::RolledBack);
                return Err(AnswerError::NotReady);
            };
            match self.accept(ports, ticket.copy()) {
                Ok(windows) => windows,
                Err(reason) => {
                    tracing::warn!(
                        module = "state",
                        hash = %hash,
                        reason = %reason,
                        "the restore accept failed before the commit — the screen is unchanged and it can be answered again"
                    );
                    self.service.finish_answer(ticket, AnswerEnd::RolledBack);
                    return Err(AnswerError::Internal(reason));
                }
            }
        } else {
            0
        };
        self.service.finish_answer(ticket, AnswerEnd::Answered);
        let durable = self.session.resolve_crash_copy(hash.clone()) == ResolveResult::Durable;
        tracing::info!(
            module = "state",
            hash = %hash,
            accept,
            restored_windows,
            durable,
            "answered the crash copy"
        );
        Ok(AnswerReply {
            restored_windows,
            durable,
        })
    }

    fn accept(&self, ports: &RestorePorts, copy: &CrashCopy) -> Result<u32, String> {
        let prepared = self.prepare(copy)?;
        // Focus is read before touching any window (before ②): read after a hidden-built popout (`focused(true)`) took focus, it could miss that another app was in front before the answer (defensive — [unverified — inference]).
        let desktop = Desktop {
            app_had_focus: ports.windows.app_has_focus(),
            monitors: ports.windows.monitors(),
        };
        // A new popout is created before entering the model so its background · first theme · first pull are decided in advance from the copy's window theme — else it paints once with the global theme. After commit the model answers, so it is withdrawn on success or failure alike.
        // ADR-0265
        for (label, attrs) in &prepared.popouts {
            ports.themes.stage(label, attrs.theme);
        }
        let unstage = |popouts: &[(WindowLabel, WindowAttrs)]| {
            for (label, _) in popouts {
                ports.themes.unstage(label);
            }
        };
        let created =
            match open_hidden_popouts(ports.windows.as_ref(), &prepared.popouts, &desktop.monitors)
            {
                Ok(created) => created,
                Err(reason) => {
                    unstage(&prepared.popouts);
                    return Err(reason);
                }
            };

        let Prepared {
            layout,
            main,
            popouts,
        } = prepared;
        let committed = self.commit(ports, layout);
        unstage(&popouts);
        let committed = match committed {
            Ok(committed) => committed,
            Err(reason) => {
                destroy_all(ports.windows.as_ref(), &created, Leftover::HiddenRestored);
                return Err(reason);
            }
        };
        // The copy's window theme is in the model — push it so the live main receives it; new popouts already painted with the value decided before ② and pulled it (TRD S21-storage §6-7).
        // ADR-0265
        ports.themes.push();

        let kept = self.after_commit(ports, committed, main, &popouts, &desktop);
        Ok(count(1 + kept))
    }

    fn prepare(&self, copy: &CrashCopy) -> Result<Prepared, String> {
        let Restored { layout, warnings } =
            ViewManager::from_persisted(copy.file.windows.clone(), self.labels.as_ref());
        warnings.iter().for_each(report_restore_warning);
        if let Some(RestoreWarning::Internal(reason)) = warnings
            .iter()
            .find(|w| matches!(w, RestoreWarning::Internal(_)))
        {
            return Err(format!("could not build the restore model: {reason}"));
        }
        let main = layout
            .window_attrs(MAIN_WINDOW_LABEL)
            .map_err(|e| format!("the restore model has no main: {e}"))?;
        let mut popouts: Vec<(WindowLabel, WindowAttrs)> = layout
            .windows
            .iter()
            .filter(|(label, _)| label.as_str() != MAIN_WINDOW_LABEL)
            .map(|(label, window)| (label.clone(), window.attrs))
            .collect();
        // One issuer: same prefix and run marker + a decimal sequence number, so (length, characters) order is issue order.
        popouts.sort_by_key(|(label, _)| (label.len(), label.clone()));
        {
            let live = self
                .layout
                .0
                .lock()
                .map_err(|_| "the layout lock is poisoned".to_string())?;
            if let Some((label, _)) = popouts.iter().find(|(l, _)| live.windows.contains_key(l)) {
                return Err(format!(
                    "new popout label {label} overlaps a window that is up — the label issuer differs from the runtime popouts"
                ));
            }
        }
        Ok(Prepared {
            layout,
            main,
            popouts,
        })
    }

    fn commit(&self, ports: &RestorePorts, restored: ViewManager) -> Result<Committed, String> {
        let subs = ports.subs.current();
        let mut mgr = self.layout.0.lock().map_err(|_| {
            tracing::error!(
                module = "state",
                "the layout lock is poisoned, so the restored screen could not be taken in"
            );
            "the layout lock is poisoned".to_string()
        })?;
        let removed = mgr
            .adopt_restored(restored)
            .map_err(|e| format!("could not take in the restore model: {e}"))?;
        let mut labels = mgr.list_windows();
        labels.sort_by_key(|label| {
            (
                label.as_str() != MAIN_WINDOW_LABEL,
                label.len(),
                label.clone(),
            )
        });
        let mut layouts = Vec::new();
        let mut tabs = Vec::new();
        for label in &labels {
            if let Ok(snapshot) = mgr.list_tabs(label) {
                layouts.extend(
                    snapshot
                        .tabs
                        .iter()
                        .filter_map(|meta| mgr.snapshot(meta.id).ok()),
                );
                tabs.push(WindowTabsPayload::from(snapshot));
            }
        }
        if let Some(subs) = &subs {
            subs.resync(&mgr);
        }
        Ok(Committed {
            removed,
            layouts,
            tabs,
        })
    }

    fn after_commit(
        &self,
        ports: &RestorePorts,
        committed: Committed,
        main: WindowAttrs,
        popouts: &[(WindowLabel, WindowAttrs)],
        desktop: &Desktop,
    ) -> usize {
        let Committed {
            removed,
            layouts,
            tabs,
        } = committed;
        let monitors = desktop.monitors.as_slice();
        crate::layout::apply::announce(ports.events.as_ref(), &layouts, &tabs);

        let windows = ports.windows.as_ref();
        let main_at = main.bounds.and_then(|bounds| {
            land(MAIN_WINDOW_LABEL, bounds, monitors, Fallback::StayPut).map(|at| (bounds, at))
        });
        windows.place_main(main_at, main.maximized);
        // If the copy's placement was discarded or could not be applied, the model holds a placement the window never went to — align it to the current one. If main's maximize could not be applied, main is not recorded (the deferral stays — `place_main`).
        windows.record_placement(MAIN_WINDOW_LABEL);

        let main_shown = windows.visibility(MAIN_WINDOW_LABEL) != Some(false);
        // A window the user closed via the tray 「Show」 between ② and ③ passed its close request · `Destroyed` cleanup with only a tombstone, since the label was not yet in the model (the ③ adoption ignores tombstones) — not removed here, a window without an OS window would stay in the model.
        let (vanished, live): (Vec<_>, Vec<_>) = popouts
            .iter()
            .partition(|(label, _)| windows.visibility(label).is_none());
        let kept = live.len();
        // Give main focus first so restored popouts, shown at first paint, come above it (user decision 2026-10-20 — main is big; TRD S21-storage §6-7 ④). The popout show activates, so even if its first paint already came it follows this, and popouts end on top in either arrival order (`set_shown` → `first_paint::reveal_held`). A hidden-built popout may have taken focus (`focused(true)` — `commands::popout`, defensive — [unverified — inference]); fronting main returns it before the person who pressed 「Restore」. With no popouts remaining it does not pull (none built: focus did not move; vanished: the person already touched it).
        // Only when our window had focus before the answer and still has it now: if another app is in front, the focus-giving fires a fake Alt key at it and steals its focus (tao 0.35.3 Windows `set_focus` → `force_window_active`) — the person may be working elsewhere while the LLM answers. Read again right before the call (the earlier sample still matters: a hidden-built popout taking focus would otherwise hide that another app was in front before the answer).
        if desktop.app_had_focus && main_shown && kept > 0 && windows.app_has_focus() {
            windows.focus(MAIN_WINDOW_LABEL);
        }
        for (label, attrs) in live {
            windows.set_shown(label, main_shown);
            // A maximized popout kept hidden is not recorded: ② deferred its maximize until shown, and recording the current (normal) placement would erase the copy's maximize from the model. A window that will be shown gets its maximize and placement record at the first-paint show; earlier recording is blocked by the deferral (`placement::record`).
            if main_shown || !attrs.maximized {
                windows.record_placement(label);
            }
        }
        let vanished: Vec<WindowLabel> = vanished
            .into_iter()
            .map(|(label, _)| label.clone())
            .collect();
        self.forget_vanished(ports, &vanished);
        destroy_all(windows, &removed, Leftover::Replaced);
        kept
    }

    // Remove from the model a new popout whose OS window vanished — same job as window destruction cleanup (`popout::drop_window_in_model`: window removal + subscription recompute in one lock); like that path, the frontend is not told.
    fn forget_vanished(&self, ports: &RestorePorts, labels: &[WindowLabel]) {
        if labels.is_empty() {
            return;
        }
        tracing::warn!(
            module = "state",
            ?labels,
            "a restored popout window closed before the screen was replaced — removing it from the model"
        );
        let subs = ports.subs.current();
        let Ok(mut mgr) = self.layout.0.lock() else {
            tracing::error!(
                module = "state",
                ?labels,
                "the layout lock is poisoned, so the closed restored popout could not be removed from the model — a window without a window remains in the model"
            );
            return;
        };
        for label in labels {
            if let Err(e) = mgr.drop_window_gone(label) {
                tracing::debug!(module = "state", label = %label, error = %e, "the restored popout is already gone from the model");
            }
        }
        if let Some(subs) = &subs {
            subs.resync(&mgr);
        }
    }
}

fn open_hidden_popouts(
    windows: &dyn RestoreWindows,
    popouts: &[(WindowLabel, WindowAttrs)],
    monitors: &[MonitorArea],
) -> Result<Vec<WindowLabel>, String> {
    let mut created = Vec::new();
    for (label, attrs) in popouts {
        let at = attrs.bounds.and_then(|bounds| {
            land(label, bounds, monitors, Fallback::DefaultPlace).map(|at| (bounds, at))
        });
        if let Err(e) = windows.open_hidden(label, at, attrs.maximized) {
            destroy_all(windows, &created, Leftover::HiddenRestored);
            return Err(format!(
                "could not create popout window {label} to restore: {e}"
            ));
        }
        created.push(label.clone());
    }
    Ok(created)
}

#[derive(Clone, Copy)]
enum Leftover {
    HiddenRestored,
    Replaced,
}

fn destroy_all(windows: &dyn RestoreWindows, labels: &[WindowLabel], leftover: Leftover) {
    for label in labels {
        let Err(first) = windows.destroy(label) else {
            continue;
        };
        tracing::warn!(module = "state", label = %label, error = %first, "could not reap the window — reaping once more");
        let Err(e) = windows.destroy(label) else {
            continue;
        };
        match leftover {
            Leftover::HiddenRestored => tracing::error!(
                module = "state",
                label = %label,
                error = %e,
                "could not reap a window created hidden for the restore — a hidden window outside the model remains, and the tray 「Show」 reveals it"
            ),
            Leftover::Replaced => tracing::error!(
                module = "state",
                label = %label,
                error = %e,
                "could not reap an old popout window after switching to the copy's screen — a window outside the model remains on the screen"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::schema::{
        PersistedContent, PersistedNode, StateFile, TabEntry, TabStrip, WindowEntry, STATE_VERSION,
    };
    use uuid::Uuid;

    #[derive(Default)]
    struct Recorder {
        seen: Mutex<Vec<CrashCopyStatus>>,
        probe: Mutex<Option<Arc<Mutex<Cell>>>>,
        lock_was_free: Mutex<Vec<bool>>,
    }

    impl RestoreNotifier for Recorder {
        fn changed(&self, status: CrashCopyStatus) {
            if let Some(cell) = self.probe.lock().unwrap().as_ref() {
                self.lock_was_free
                    .lock()
                    .unwrap()
                    .push(cell.try_lock().is_ok());
            }
            self.seen.lock().unwrap().push(status);
        }
    }

    impl Recorder {
        fn seen(&self) -> Vec<CrashCopyStatus> {
            self.seen.lock().unwrap().clone()
        }
    }

    fn tab() -> TabEntry {
        TabEntry {
            id: Uuid::new_v4(),
            name: "Tab".into(),
            named: Some(true),
            focused_slot_id: None,
            layout: PersistedNode::Slot {
                id: Uuid::new_v4(),
                content: PersistedContent::Known(crate::layout::SlotContent::Empty),
            },
        }
    }

    fn window(id: &str, kind: WindowKind) -> WindowEntry {
        WindowEntry {
            id: id.into(),
            number: None,
            kind,
            theme: None,
            bounds: None,
            maximized: false,
        }
    }

    fn strip(tabs: usize) -> TabStrip {
        let tabs: Vec<TabEntry> = (0..tabs).map(|_| tab()).collect();
        TabStrip {
            active_tab: tabs[0].id,
            tabs,
        }
    }

    fn crash_copy(hash: &str) -> CrashCopy {
        CrashCopy {
            text: "raw".into(),
            hash: hash.into(),
            file: StateFile {
                version: STATE_VERSION,
                saved_at_ms: 1234,
                clean_exit: false,
                resolved_crash_copy: None,
                windows: vec![
                    window("main", WindowKind::Main(strip(2))),
                    window("p-1", WindowKind::Popout(strip(1))),
                ],
            },
        }
    }

    fn service() -> (RestoreService, Arc<Recorder>) {
        let service = RestoreService::new();
        let recorder = Arc::new(Recorder::default());
        *recorder.probe.lock().unwrap() = Some(Arc::clone(&service.cell));
        service.set_notifier(recorder.clone());
        (service, recorder)
    }

    fn idle(status: CrashCopyStatus) -> RestoreStatusView {
        RestoreStatusView {
            crash_copy: status,
            saved_at_ms: None,
            windows: None,
            tabs: None,
            durable: None,
            saves: true,
            state_file: StateFileStatus::Ok,
        }
    }

    // ── Three states ──

    #[test]
    fn a_new_service_has_no_copy_and_refuses_an_answer() {
        let service = RestoreService::new();
        assert_eq!(service.status(), idle(CrashCopyStatus::None));
        assert_eq!(
            service.begin_answer().unwrap_err(),
            AnswerConflict::NotAwaiting(CrashCopyStatus::None)
        );
    }

    #[test]
    fn the_boot_without_a_copy_settles_none_and_notifies() {
        let (service, recorder) = service();
        service.set_boot(None, StateFileStatus::Ok, true);
        assert_eq!(service.status(), idle(CrashCopyStatus::None));
        assert_eq!(recorder.seen(), [CrashCopyStatus::None]);
    }

    #[test]
    fn the_boot_with_a_copy_awaits_and_the_view_counts_the_copy() {
        let (service, recorder) = service();
        service.set_boot(Some(crash_copy("h")), StateFileStatus::Ok, true);
        assert_eq!(
            service.status(),
            RestoreStatusView {
                crash_copy: CrashCopyStatus::Awaiting,
                saved_at_ms: Some(1234),
                windows: Some(2),
                tabs: Some(3),
                durable: Some(true),
                saves: true,
                state_file: StateFileStatus::Ok,
            }
        );
        assert_eq!(recorder.seen(), [CrashCopyStatus::Awaiting]);
    }

    #[test]
    fn durable_is_saves_while_awaiting_and_null_outside_awaiting() {
        for saves in [true, false] {
            let (service, _recorder) = service();
            assert_eq!(service.status().durable, None, "none — {saves}");

            service.set_boot(Some(crash_copy("h")), StateFileStatus::Ok, saves);
            assert_eq!(service.status().durable, Some(saves), "awaiting — {saves}");

            let ticket = service.begin_answer().unwrap();
            assert_eq!(
                service.status().durable,
                Some(saves),
                "awaiting even while processing — {saves}"
            );
            service.finish_answer(ticket, AnswerEnd::Answered);
            assert_eq!(service.status().durable, None, "answered — {saves}");
        }
    }

    #[test]
    fn saves_is_on_every_view_of_the_run_and_survives_the_answer() {
        for saves in [true, false] {
            let (none, _recorder) = service();
            none.set_boot(None, StateFileStatus::Ok, saves);
            assert_eq!(
                none.status(),
                RestoreStatusView {
                    saves,
                    ..idle(CrashCopyStatus::None)
                },
                "stands even with no copy to ask about — {saves}"
            );

            let (service, _recorder) = service();
            service.set_boot(Some(crash_copy("h")), StateFileStatus::Ok, saves);
            let awaiting = service.status();
            assert_eq!(awaiting.saves, saves, "awaiting — {saves}");
            assert_eq!(
                awaiting.durable,
                Some(awaiting.saves),
                "the two are the same while asking — {saves}"
            );

            let ticket = service.begin_answer().unwrap();
            assert_eq!(service.status().saves, saves, "processing — {saves}");
            service.finish_answer(ticket, AnswerEnd::RolledBack);
            assert_eq!(service.status().saves, saves, "after reverting — {saves}");

            let ticket = service.begin_answer().unwrap();
            service.finish_answer(ticket, AnswerEnd::Answered);
            assert_eq!(
                service.status(),
                RestoreStatusView {
                    saves,
                    ..idle(CrashCopyStatus::Answered)
                },
                "the run's value as is even after answering — {saves}"
            );
        }
    }

    #[test]
    fn the_boot_sets_the_state_file_status_for_every_view_of_the_run() {
        let (unread, recorder) = service();
        unread.set_boot(None, StateFileStatus::Unreadable, false);
        assert_eq!(
            unread.status(),
            RestoreStatusView {
                saves: false,
                state_file: StateFileStatus::Unreadable,
                ..idle(CrashCopyStatus::None)
            },
            "stands even without a copy"
        );
        assert_eq!(
            recorder.seen(),
            [CrashCopyStatus::None],
            "what the notice carries stays the same"
        );

        // An unusable state.json × an unanswered copy — copy it aside and ask (§6-5 table).
        let (service, _corrupt_recorder) = service();
        service.set_boot(
            Some(crash_copy("h")),
            StateFileStatus::CorruptCopiedAside,
            true,
        );
        assert_eq!(
            service.status().state_file,
            StateFileStatus::CorruptCopiedAside
        );
        let ticket = service.begin_answer().unwrap();
        service.finish_answer(ticket, AnswerEnd::Answered);
        assert_eq!(
            service.status(),
            RestoreStatusView {
                state_file: StateFileStatus::CorruptCopiedAside,
                ..idle(CrashCopyStatus::Answered)
            },
            "the run's value as is even when answered"
        );
    }

    #[test]
    fn an_answer_moves_to_answered_notifies_and_refuses_a_second_answer() {
        let (service, recorder) = service();
        service.set_boot(Some(crash_copy("h")), StateFileStatus::Ok, true);

        let ticket = service.begin_answer().expect("can answer");
        assert_eq!(ticket.copy().hash, "h");
        assert_eq!(ticket.copy().text, "raw");
        service.finish_answer(ticket, AnswerEnd::Answered);

        assert_eq!(service.status(), idle(CrashCopyStatus::Answered));
        assert_eq!(
            recorder.seen(),
            [CrashCopyStatus::Awaiting, CrashCopyStatus::Answered]
        );
        assert_eq!(
            service.begin_answer().unwrap_err(),
            AnswerConflict::NotAwaiting(CrashCopyStatus::Answered)
        );
    }

    #[test]
    fn a_second_answer_while_one_is_in_flight_conflicts() {
        let (service, _recorder) = service();
        service.set_boot(Some(crash_copy("h")), StateFileStatus::Ok, true);

        let _ticket = service.begin_answer().unwrap();
        assert_eq!(
            service.begin_answer().unwrap_err(),
            AnswerConflict::InFlight
        );
        assert_eq!(
            service.status().crash_copy,
            CrashCopyStatus::Awaiting,
            "the status is awaiting even while processing"
        );
    }

    #[test]
    fn a_rolled_back_answer_stays_awaiting_notifies_it_and_can_be_answered_again() {
        let (service, recorder) = service();
        service.set_boot(Some(crash_copy("h")), StateFileStatus::Ok, true);

        let ticket = service.begin_answer().unwrap();
        service.finish_answer(ticket, AnswerEnd::RolledBack);

        assert_eq!(service.status().crash_copy, CrashCopyStatus::Awaiting);
        assert_eq!(
            recorder.seen(),
            [CrashCopyStatus::Awaiting, CrashCopyStatus::Awaiting],
            "a revert is notified too — so the side refused with InFlight in between knows it can answer again"
        );
        assert!(service.begin_answer().is_ok());
    }

    #[test]
    fn dropping_a_ticket_rolls_back() {
        let (service, recorder) = service();
        service.set_boot(Some(crash_copy("h")), StateFileStatus::Ok, true);

        drop(service.begin_answer().unwrap());

        assert_eq!(service.status().crash_copy, CrashCopyStatus::Awaiting);
        assert_eq!(
            recorder.seen(),
            [CrashCopyStatus::Awaiting, CrashCopyStatus::Awaiting]
        );
        assert!(service.begin_answer().is_ok(), "the marker was released");
    }

    #[test]
    fn a_ticket_unwound_by_a_panic_rolls_back() {
        let (service, _recorder) = service();
        service.set_boot(Some(crash_copy("h")), StateFileStatus::Ok, true);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ticket = service.begin_answer().unwrap();
            panic!("panic during the accept");
        }));

        assert!(result.is_err());
        assert!(service.begin_answer().is_ok());
    }

    #[test]
    fn a_ticket_from_before_a_second_boot_setting_changes_nothing() {
        let (service, recorder) = service();
        service.set_boot(Some(crash_copy("old")), StateFileStatus::Ok, true);
        let stale = service.begin_answer().unwrap();

        service.set_boot(Some(crash_copy("new")), StateFileStatus::Ok, true);
        service.finish_answer(stale, AnswerEnd::Answered);

        assert_eq!(service.status().crash_copy, CrashCopyStatus::Awaiting);
        assert_eq!(
            recorder.seen(),
            [CrashCopyStatus::Awaiting, CrashCopyStatus::Awaiting],
            "an answer of the old marker is not notified"
        );
        let fresh = service
            .begin_answer()
            .expect("the new generation is not in progress");
        assert_eq!(fresh.copy().hash, "new");
    }

    #[test]
    fn the_notifier_is_called_with_no_lock_held() {
        let (service, recorder) = service();
        service.set_boot(Some(crash_copy("h")), StateFileStatus::Ok, true);
        let ticket = service.begin_answer().unwrap();
        service.finish_answer(ticket, AnswerEnd::RolledBack);
        drop(service.begin_answer().unwrap());
        let ticket = service.begin_answer().unwrap();
        service.finish_answer(ticket, AnswerEnd::Answered);

        assert_eq!(
            *recorder.lock_was_free.lock().unwrap(),
            [true, true, true, true],
            "boot · revert · discarded marker · answer"
        );
    }

    #[test]
    fn the_notifier_may_call_back_into_the_service() {
        struct Reentrant(
            Mutex<Option<Arc<RestoreService>>>,
            Mutex<Vec<RestoreStatusView>>,
        );
        impl RestoreNotifier for Reentrant {
            fn changed(&self, _status: CrashCopyStatus) {
                if let Some(service) = self.0.lock().unwrap().as_ref() {
                    self.1.lock().unwrap().push(service.status());
                }
            }
        }
        let service = Arc::new(RestoreService::new());
        let notifier = Arc::new(Reentrant(
            Mutex::new(Some(service.clone())),
            Mutex::default(),
        ));
        service.set_notifier(notifier.clone());

        service.set_boot(Some(crash_copy("h")), StateFileStatus::Ok, true);

        assert_eq!(
            notifier.1.lock().unwrap()[0].crash_copy,
            CrashCopyStatus::Awaiting,
            "the status seen inside the notice is already the settled value"
        );
        // Break the cycle (service ↔ notification).
        notifier.0.lock().unwrap().take();
    }

    #[test]
    fn the_status_wire_spelling_is_snake_case() {
        let spell = |status| serde_json::to_string(&status).unwrap();
        assert_eq!(spell(CrashCopyStatus::None), "\"none\"");
        assert_eq!(spell(CrashCopyStatus::Awaiting), "\"awaiting\"");
        assert_eq!(spell(CrashCopyStatus::Answered), "\"answered\"");
        for status in [
            CrashCopyStatus::None,
            CrashCopyStatus::Awaiting,
            CrashCopyStatus::Answered,
        ] {
            assert_eq!(
                format!("\"{}\"", status.as_wire()),
                spell(status),
                "the bus spelling (as_wire) and the Tauri spelling (serde) are the same"
            );
        }
    }

    #[test]
    fn the_state_file_wire_spelling_is_snake_case() {
        let spell = |status| serde_json::to_string(&status).unwrap();
        assert_eq!(spell(StateFileStatus::Ok), "\"ok\"");
        assert_eq!(spell(StateFileStatus::Unreadable), "\"unreadable\"");
        assert_eq!(spell(StateFileStatus::Newer), "\"newer\"");
        assert_eq!(
            spell(StateFileStatus::CorruptCopiedAside),
            "\"corrupt_copied_aside\""
        );
        assert_eq!(
            spell(StateFileStatus::CorruptNotCopied),
            "\"corrupt_not_copied\""
        );
        for status in [
            StateFileStatus::Ok,
            StateFileStatus::Unreadable,
            StateFileStatus::Newer,
            StateFileStatus::CorruptCopiedAside,
            StateFileStatus::CorruptNotCopied,
        ] {
            assert_eq!(
                format!("\"{}\"", status.as_wire()),
                spell(status),
                "the bus spelling (as_wire) and the Tauri spelling (serde) are the same"
            );
        }
    }

    #[test]
    fn the_status_and_the_reply_serialize_as_the_wire_shapes() {
        let (service, _recorder) = service();
        assert_eq!(
            serde_json::to_value(service.status()).unwrap(),
            serde_json::json!({ "crash_copy": "none", "saved_at_ms": null, "windows": null, "tabs": null, "durable": null, "saves": true, "state_file": "ok" })
        );
        service.set_boot(
            Some(crash_copy("h")),
            StateFileStatus::CorruptNotCopied,
            true,
        );
        assert_eq!(
            serde_json::to_value(service.status()).unwrap(),
            serde_json::json!({ "crash_copy": "awaiting", "saved_at_ms": 1234, "windows": 2, "tabs": 3, "durable": true, "saves": true, "state_file": "corrupt_not_copied" })
        );

        // After guard ii has been answered — the fact that this run does not save remains in `saves` alone.
        let guarded = RestoreService::new();
        guarded.set_boot(Some(crash_copy("h")), StateFileStatus::Ok, false);
        let ticket = guarded.begin_answer().unwrap();
        guarded.finish_answer(ticket, AnswerEnd::Answered);
        assert_eq!(
            serde_json::to_value(guarded.status()).unwrap(),
            serde_json::json!({ "crash_copy": "answered", "saved_at_ms": null, "windows": null, "tabs": null, "durable": null, "saves": false, "state_file": "ok" })
        );
        assert_eq!(
            serde_json::to_value(AnswerReply {
                restored_windows: 2,
                durable: false
            })
            .unwrap(),
            serde_json::json!({ "restored_windows": 2, "durable": false })
        );
    }

    // ── Restore coordinator ──

    use crate::layout::{tree, TabName, WindowPlacement};
    use crate::settings::SettingsService;
    use crate::state::convert::to_persisted;
    use crate::theme::{EffectiveThemes, ThemeWindows, UiSettingsPayload, UiTheme};

    const PRIMARY: MonitorArea = MonitorArea {
        x: 0,
        y: 0,
        w: 1920,
        h: 1080,
        scale: 1.0,
    };

    fn rect(x: f64, y: f64, w: f64, h: f64) -> WindowBounds {
        WindowBounds::new(x, y, w, h).unwrap()
    }

    fn seen(maximized: bool, bounds: WindowBounds) -> WindowPlacement {
        WindowPlacement {
            minimized: false,
            maximized,
            bounds: Some(bounds),
        }
    }

    fn foreign() -> serde_json::Map<String, serde_json::Value> {
        serde_json::json!({ "type": "future_slot", "from": "a newer build" })
            .as_object()
            .unwrap()
            .clone()
    }

    fn first_slot(mgr: &ViewManager, view: Uuid) -> Uuid {
        tree::first_slot_id(&mgr.views[&view].layout)
    }

    /// The previous run's screen — main tab 「Last tab」 (placement on the monitor · `main_maximized`) and `popouts` popouts (the first = unknown content slot · placement overlapping no monitor · maximized).
    fn previous_screen(popouts: usize, main_maximized: bool) -> Vec<WindowEntry> {
        let mut prev = ViewManager::new();
        let main_view = prev.windows[MAIN_WINDOW_LABEL].active;
        prev.rename_tab(main_view, "Last tab".into()).unwrap();
        prev.observe_window_placement(
            MAIN_WINDOW_LABEL,
            seen(false, rect(100.0, 80.0, 1200.0, 800.0)),
        )
        .unwrap();
        if main_maximized {
            prev.observe_window_placement(
                MAIN_WINDOW_LABEL,
                seen(true, rect(0.0, 0.0, 1920.0, 1040.0)),
            )
            .unwrap();
        }
        for n in 0..popouts {
            let label = format!("slot-popup-{}", 70 + n);
            let view = prev.create_window(&label).unwrap();
            if n == 0 {
                let slot = first_slot(&prev, view);
                prev.set_unknown_content(view, slot, foreign()).unwrap();
                prev.observe_window_placement(
                    &label,
                    seen(false, rect(-9000.0, 100.0, 700.0, 500.0)),
                )
                .unwrap();
                prev.observe_window_placement(&label, seen(true, rect(0.0, 0.0, 1920.0, 1040.0)))
                    .unwrap();
            }
        }
        to_persisted(&prev)
    }

    fn copy_of(windows: Vec<WindowEntry>) -> CrashCopy {
        CrashCopy {
            text: "raw".into(),
            hash: "h".into(),
            file: StateFile {
                version: STATE_VERSION,
                saved_at_ms: 1,
                clean_exit: false,
                resolved_crash_copy: None,
                windows,
            },
        }
    }

    #[derive(Default)]
    struct FakeWindows {
        layout: LayoutState,
        calls: Mutex<Vec<String>>,
        lock_was_free: Mutex<Vec<bool>>,
        fail_open: Mutex<Option<String>>,
        squat_on_open: Mutex<Option<String>>,
        hidden: Mutex<Vec<String>>,
        gone: Mutex<Vec<String>>,
        fail_destroy: Mutex<Option<String>>,
        deferred: Mutex<Vec<String>>,
        /// Another app is in front (none of our windows has focus). Creation lowers it, assuming a hidden-built popout takes focus (production builds it with `focused(true)`) — reading focus after ② would be fooled.
        background: Mutex<bool>,
        focus_samples: Mutex<Vec<bool>>,
        /// The effective theme the production port looks at when choosing the window background · first theme — if present, the answer at creation is written to `opened_with`.
        themes: OnceLock<Arc<EffectiveThemes>>,
        opened_with: Mutex<Vec<(String, String)>>,
    }

    impl FakeWindows {
        fn new(layout: &LayoutState) -> Self {
            Self {
                layout: layout.clone(),
                ..Self::default()
            }
        }

        fn note(&self, call: String) {
            self.lock_was_free
                .lock()
                .unwrap()
                .push(self.layout.0.try_lock().is_ok());
            self.calls.lock().unwrap().push(call);
        }

        fn in_model(&self, label: &str) -> bool {
            self.layout.0.lock().unwrap().windows.contains_key(label)
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        /// A call without the deferred maximize — with two or more popouts, which new label receives the copy's maximized popout varies (the copy's popout order = persistent id order).
        fn calls_but_deferrals(&self) -> Vec<String> {
            self.calls()
                .into_iter()
                .filter(|call| !call.starts_with("defer_maximize "))
                .collect()
        }

        fn deferred(&self) -> Vec<String> {
            self.deferred.lock().unwrap().clone()
        }

        fn listed(list: &Mutex<Vec<String>>, label: &str) -> bool {
            list.lock().unwrap().iter().any(|l| l == label)
        }
    }

    impl RestoreWindows for FakeWindows {
        fn app_has_focus(&self) -> bool {
            self.note("focused?".into());
            let mut samples = self.focus_samples.lock().unwrap();
            if samples.is_empty() {
                !*self.background.lock().unwrap()
            } else {
                samples.remove(0)
            }
        }

        fn monitors(&self) -> Vec<MonitorArea> {
            self.note("monitors".into());
            vec![PRIMARY]
        }

        // The deferral is set before creating the window and withdrawn if creation fails (the production port's contract).
        fn open_hidden(
            &self,
            label: &str,
            at: Option<(WindowBounds, Landing)>,
            maximized: bool,
        ) -> Result<(), String> {
            if maximized {
                self.note(format!(
                    "defer_maximize {label} in_model={}",
                    self.in_model(label)
                ));
                self.deferred.lock().unwrap().push(label.into());
            }
            self.note(format!(
                "open_hidden {label} at={} in_model={}",
                at.is_some(),
                self.in_model(label)
            ));
            if let Some(themes) = self.themes.get() {
                self.opened_with.lock().unwrap().push((
                    label.to_string(),
                    themes.effective_for(label).as_wire().to_string(),
                ));
            }
            if self.fail_open.lock().unwrap().as_deref() == Some(label) {
                self.deferred.lock().unwrap().retain(|l| l != label);
                return Err("window creation failed (test)".into());
            }
            if self.squat_on_open.lock().unwrap().as_deref() == Some(label) {
                self.layout.0.lock().unwrap().create_window(label).unwrap();
            }
            *self.background.lock().unwrap() = false;
            Ok(())
        }

        fn visibility(&self, label: &str) -> Option<bool> {
            self.note(format!("visible? {label}"));
            if Self::listed(&self.gone, label) {
                return None;
            }
            Some(!Self::listed(&self.hidden, label))
        }

        fn place_main(&self, at: Option<(WindowBounds, Landing)>, maximized: bool) {
            self.note(format!(
                "place_main at={} maximized={maximized}",
                at.is_some()
            ));
        }

        fn set_shown(&self, label: &str, shown: bool) {
            let verb = if shown { "show" } else { "hide" };
            self.note(format!("{verb} {label} in_model={}", self.in_model(label)));
            let applied = {
                let mut deferred = self.deferred.lock().unwrap();
                let pending = shown && deferred.iter().any(|l| l == label);
                deferred.retain(|l| !(pending && l == label));
                pending
            };
            if applied {
                self.note(format!("maximize {label} (deferred)"));
            }
        }

        fn record_placement(&self, label: &str) {
            self.note(format!("record {label}"));
        }

        fn focus(&self, label: &str) {
            self.note(format!("focus {label}"));
        }

        fn destroy(&self, label: &str) -> Result<(), String> {
            self.note(format!("destroy {label} in_model={}", self.in_model(label)));
            if self.fail_destroy.lock().unwrap().as_deref() == Some(label) {
                return Err("window reap failed (test)".into());
            }
            Ok(())
        }
    }

    struct FakeEvents {
        layout: LayoutState,
        layouts: Mutex<Vec<Uuid>>,
        tabs: Mutex<Vec<(String, u64)>>,
        first_seen: Mutex<Option<bool>>,
        order: Mutex<Vec<&'static str>>,
    }

    impl FakeEvents {
        fn note(&self, what: &'static str) {
            let mut first = self.first_seen.lock().unwrap();
            if first.is_none() {
                *first = Some(self.layout.0.try_lock().is_ok());
            }
            self.order.lock().unwrap().push(what);
        }
    }

    impl LayoutEvents for FakeEvents {
        fn layout_updated(&self, snapshot: &ViewSnapshot) {
            self.note("layout");
            self.layouts.lock().unwrap().push(snapshot.view_id);
        }

        fn window_tabs_updated(&self, tabs: &WindowTabsPayload) {
            self.note("tabs");
            self.tabs
                .lock()
                .unwrap()
                .push((tabs.label.clone(), tabs.version));
        }
    }

    #[derive(Default)]
    struct FakeSubs {
        seen: Mutex<Vec<u64>>,
    }

    impl SubscriptionSync for FakeSubs {
        fn resync(&self, mgr: &ViewManager) {
            self.seen.lock().unwrap().push(mgr.version);
        }
    }

    struct ThemeScreen {
        layout: LayoutState,
        sent: Mutex<Vec<(String, String)>>,
        lock_was_free: Mutex<Vec<bool>>,
    }

    impl ThemeWindows for ThemeScreen {
        fn labels(&self) -> Vec<String> {
            let mut labels = self.layout.0.lock().unwrap().list_windows();
            labels.sort();
            labels
        }

        fn send(&self, label: &str, payload: UiSettingsPayload) -> Result<(), String> {
            self.lock_was_free
                .lock()
                .unwrap()
                .push(self.layout.0.try_lock().is_ok());
            self.sent
                .lock()
                .unwrap()
                .push((label.to_string(), payload.theme));
            Ok(())
        }
    }

    struct SubsSlot {
        layout: LayoutState,
        subs: Mutex<Option<Arc<dyn SubscriptionSync>>>,
        lookups_with_lock_free: Mutex<Vec<bool>>,
    }

    impl SubscriptionSource for SubsSlot {
        fn current(&self) -> Option<Arc<dyn SubscriptionSync>> {
            self.lookups_with_lock_free
                .lock()
                .unwrap()
                .push(self.layout.0.try_lock().is_ok());
            self.subs.lock().unwrap().clone()
        }
    }

    // Issues monotonically in the form without run marker (`slot-popup-<n>`) so tests can write the expected label literally; the real `PopupCounter` form and that (length, characters) order is issue order are measured by the `commands::popout` tests.
    #[derive(Default)]
    struct SeqLabels(std::sync::atomic::AtomicU64);

    impl LabelSource for SeqLabels {
        fn next_label(&self) -> String {
            let n = self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            format!("slot-popup-{n}")
        }
    }

    struct Rig {
        coordinator: RestoreCoordinator,
        service: Arc<RestoreService>,
        notices: Arc<Recorder>,
        layout: LayoutState,
        labels: Arc<SeqLabels>,
        windows: Arc<FakeWindows>,
        events: Arc<FakeEvents>,
        subs: Arc<FakeSubs>,
        slot: Arc<SubsSlot>,
        theme_screen: Arc<ThemeScreen>,
        themes: ThemeControl,
        effective: Arc<EffectiveThemes>,
    }

    impl Rig {
        fn new(labels: Option<Arc<SeqLabels>>) -> Self {
            let service = Arc::new(RestoreService::new());
            let notices = Arc::new(Recorder::default());
            service.set_notifier(notices.clone());
            let layout = LayoutState::new();
            let shared = Arc::new(SeqLabels::default());
            let coordinator = RestoreCoordinator::new(
                service.clone(),
                layout.clone(),
                Arc::new(StateSession::default()),
                labels.unwrap_or_else(|| shared.clone()),
            );
            let windows = Arc::new(FakeWindows::new(&layout));
            let events = Arc::new(FakeEvents {
                layout: layout.clone(),
                layouts: Mutex::default(),
                tabs: Mutex::default(),
                first_seen: Mutex::default(),
                order: Mutex::default(),
            });
            let subs = Arc::new(FakeSubs::default());
            let slot = Arc::new(SubsSlot {
                layout: layout.clone(),
                subs: Mutex::default(),
                lookups_with_lock_free: Mutex::default(),
            });
            let theme_screen = Arc::new(ThemeScreen {
                layout: layout.clone(),
                sent: Mutex::default(),
                lock_was_free: Mutex::default(),
            });
            // Settings with write not opened — the global theme is the default (dark), and loading does not create a folder that does not exist.
            let settings = Arc::new(SettingsService::load_from_dir(
                &std::env::temp_dir().join("engram-restore-no-settings"),
            ));
            let effective = Arc::new(EffectiveThemes::new(settings, layout.clone()));
            let _ = windows.themes.set(effective.clone());
            let themes = ThemeControl::new(effective.clone(), theme_screen.clone());
            Rig {
                coordinator,
                service,
                notices,
                layout,
                labels: shared,
                windows,
                events,
                subs,
                slot,
                theme_screen,
                themes,
                effective,
            }
        }

        fn attach(&self) {
            self.register_subs();
            self.attach_without_subs();
        }

        fn attach_without_subs(&self) {
            self.coordinator.attach(RestorePorts {
                windows: self.windows.clone(),
                events: self.events.clone(),
                subs: self.slot.clone(),
                themes: self.themes.clone(),
            });
        }

        fn register_subs(&self) {
            *self.slot.subs.lock().unwrap() = Some(self.subs.clone());
        }

        fn live_popout(&self) -> (String, Uuid) {
            let label = LabelSource::next_label(self.labels.as_ref());
            let mut mgr = self.layout.0.lock().unwrap();
            let view = mgr.create_window(&label).unwrap();
            let slot = first_slot(&mgr, view);
            mgr.set_unknown_content(view, slot, foreign()).unwrap();
            (label, view)
        }

        fn fingerprint(&self) -> (u64, u64, Vec<String>, usize) {
            let mgr = self.layout.0.lock().unwrap();
            let mut labels = mgr.list_windows();
            labels.sort();
            (mgr.version, mgr.attrs_rev(), labels, mgr.views.len())
        }

        fn windows_in_model(&self) -> Vec<String> {
            let mut labels = self.layout.0.lock().unwrap().list_windows();
            labels.sort();
            labels
        }

        fn ports_called_with_the_lock_free(&self) -> bool {
            self.windows
                .lock_was_free
                .lock()
                .unwrap()
                .iter()
                .chain(self.slot.lookups_with_lock_free.lock().unwrap().iter())
                .all(|free| *free)
        }
    }

    #[test]
    fn an_accept_swaps_the_screen_and_answers() {
        let rig = Rig::new(None);
        rig.attach();
        let (live_label, live_view) = rig.live_popout();
        assert_eq!(live_label, "slot-popup-1");
        let (version, attrs_rev) = {
            let mgr = rig.layout.0.lock().unwrap();
            (mgr.version, mgr.attrs_rev())
        };
        let windows = previous_screen(1, false);
        rig.service
            .set_boot(Some(copy_of(windows)), StateFileStatus::Ok, true);

        let reply = rig.coordinator.answer(true).expect("accept");

        assert_eq!(
            reply,
            AnswerReply {
                restored_windows: 2,
                durable: false,
            },
            "main + one popout · no writer"
        );
        {
            let mgr = rig.layout.0.lock().unwrap();
            let mut labels = mgr.list_windows();
            labels.sort();
            assert_eq!(
                labels,
                [MAIN_WINDOW_LABEL, "slot-popup-2"],
                "the old popout is removed and the new popout gets a label that does not overlap windows that are up"
            );
            assert_eq!(
                (mgr.version, mgr.attrs_rev()),
                (version + 1, attrs_rev + 1),
                "the numbers go up by one from the current value"
            );
            let main = &mgr.windows[MAIN_WINDOW_LABEL];
            assert_eq!(
                mgr.views[&main.active].name,
                TabName::Named("Last tab".into())
            );
            assert_eq!(main.attrs.bounds, Some(rect(100.0, 80.0, 1200.0, 800.0)));
            assert!(!mgr.views.contains_key(&live_view));
            let popout = &mgr.windows["slot-popup-2"];
            assert!(popout.attrs.maximized);
            let view = popout.active;
            assert_eq!(
                mgr.unknown_content(view, first_slot(&mgr, view)),
                Some(&foreign()),
                "the unknown content raw text comes with the View"
            );
        }

        assert_eq!(
            rig.windows.calls(),
            [
                "focused?",
                "monitors",
                "defer_maximize slot-popup-2 in_model=false",
                "open_hidden slot-popup-2 at=false in_model=false",
                "place_main at=true maximized=false",
                "record main",
                "visible? main",
                "visible? slot-popup-2",
                "focused?",
                "focus main",
                "show slot-popup-2 in_model=true",
                "maximize slot-popup-2 (deferred)",
                "record slot-popup-2",
                "destroy slot-popup-1 in_model=false",
            ],
            "read focus → create hidden (maximize deferred before creating · outside the model · positions off every monitor discarded) → commit → position → record current position → read focus again → focus main → show popouts (deferred maximize — after the main focus, so the popouts are on top) → reap old windows"
        );
        assert!(
            rig.windows.deferred().is_empty(),
            "the deferral of shown windows was withdrawn"
        );
        assert!(
            rig.ports_called_with_the_lock_free(),
            "the window port · subscription source are called outside the lock"
        );
        assert_eq!(
            *rig.subs.seen.lock().unwrap(),
            [version + 1],
            "the recompute is in the same critical section as the commit"
        );
        assert_eq!(
            *rig.events.first_seen.lock().unwrap(),
            Some(true),
            "notices are outside the lock"
        );
        let tabs = rig.events.tabs.lock().unwrap().clone();
        assert_eq!(
            tabs,
            [
                (MAIN_WINDOW_LABEL.to_string(), version + 1),
                ("slot-popup-2".to_string(), version + 1)
            ]
        );
        assert_eq!(
            rig.events.layouts.lock().unwrap().len(),
            2,
            "a layout per tab"
        );
        assert_eq!(
            *rig.events.order.lock().unwrap(),
            ["layout", "layout", "tabs", "tabs"],
            "layout notice → tab notice"
        );

        assert_eq!(rig.service.status().crash_copy, CrashCopyStatus::Answered);
        assert_eq!(
            rig.notices.seen(),
            [CrashCopyStatus::Awaiting, CrashCopyStatus::Answered]
        );
        assert!(matches!(
            rig.coordinator.answer(true),
            Err(AnswerError::Conflict(AnswerConflict::NotAwaiting(
                CrashCopyStatus::Answered
            )))
        ));
    }

    /// Acceptance pushes the copy's window theme: main is up and a new popout may have pulled its first (global) value before entering the model. Pushed before the show, outside the layout lock.
    #[test]
    fn an_accept_pushes_the_copys_window_themes() {
        let rig = Rig::new(None);
        rig.attach();
        let mut prev = ViewManager::new();
        prev.set_window_theme(MAIN_WINDOW_LABEL, Some(UiTheme::Light))
            .unwrap();
        prev.create_window("slot-popup-70").unwrap();
        prev.set_window_theme("slot-popup-70", Some(UiTheme::EInk))
            .unwrap();
        prev.create_window("slot-popup-71").unwrap();
        rig.service.set_boot(
            Some(copy_of(to_persisted(&prev))),
            StateFileStatus::Ok,
            true,
        );

        rig.coordinator.answer(true).expect("accept");

        let mut sent = rig.theme_screen.sent.lock().unwrap().clone();
        sent.sort();
        let mut themes: Vec<String> = sent.iter().map(|(_, theme)| theme.clone()).collect();
        themes.sort();
        assert_eq!(sent.len(), 3, "main · two new popouts: {sent:?}");
        assert!(sent.contains(&(MAIN_WINDOW_LABEL.to_string(), "light".to_string())));
        assert_eq!(
            themes,
            ["dark", "e-ink", "light"],
            "one new popout has the copy's theme, one the global value: {sent:?}"
        );
        assert!(
            rig.theme_screen
                .lock_was_free
                .lock()
                .unwrap()
                .iter()
                .all(|free| *free),
            "pushed outside the layout lock"
        );

        // Even if a new popout is created before it enters the model, it is painted with the copy's theme from the start — the answer at creation equals the pushed value.
        let opened = rig.windows.opened_with.lock().unwrap().clone();
        assert_eq!(opened.len(), 2, "{opened:?}");
        for window in &opened {
            assert!(
                sent.contains(window),
                "at creation {window:?} · pushed values {sent:?}"
            );
        }
    }

    /// Even if it fails before commit, the popout theme decided in advance does not remain — if the same label is used later, it would receive someone else's theme.
    #[test]
    fn a_failed_accept_leaves_no_staged_theme() {
        let rig = Rig::new(None);
        rig.attach();
        let mut prev = ViewManager::new();
        prev.create_window("slot-popup-70").unwrap();
        prev.set_window_theme("slot-popup-70", Some(UiTheme::EInk))
            .unwrap();
        rig.service.set_boot(
            Some(copy_of(to_persisted(&prev))),
            StateFileStatus::Ok,
            true,
        );
        *rig.windows.fail_open.lock().unwrap() = Some("slot-popup-1".to_string());

        assert!(rig.coordinator.answer(true).is_err());

        assert_eq!(
            *rig.windows.opened_with.lock().unwrap(),
            [("slot-popup-1".to_string(), "e-ink".to_string())],
            "at creation it was the copy's theme"
        );
        assert_eq!(rig.effective.effective_for("slot-popup-1"), UiTheme::Dark);
    }

    /// If it fails before acceptance, the screen is unchanged so there is nothing to push.
    #[test]
    fn a_failed_accept_pushes_no_theme() {
        let rig = Rig::new(None);
        rig.attach();
        let windows = previous_screen(1, false);
        rig.service
            .set_boot(Some(copy_of(windows)), StateFileStatus::Ok, true);
        *rig.windows.fail_open.lock().unwrap() = Some("slot-popup-1".to_string());

        assert!(rig.coordinator.answer(true).is_err());

        assert!(rig.theme_screen.sent.lock().unwrap().is_empty());
    }

    #[test]
    fn restored_popouts_stay_hidden_while_main_is_hidden_and_main_follows_the_copy_maximize() {
        let rig = Rig::new(None);
        rig.attach();
        rig.windows
            .hidden
            .lock()
            .unwrap()
            .push(MAIN_WINDOW_LABEL.into());
        let windows = previous_screen(2, true);
        rig.service
            .set_boot(Some(copy_of(windows)), StateFileStatus::Ok, true);

        let reply = rig.coordinator.answer(true).expect("accept");

        assert_eq!(reply.restored_windows, 3);
        // The copy's popout order is persistent id order, so which new label receives the maximized popout differs every time — read it from the model.
        let maximized = {
            let mgr = rig.layout.0.lock().unwrap();
            assert!(
                mgr.windows[MAIN_WINDOW_LABEL].attrs.maximized,
                "the model holds the copy's main maximize"
            );
            ["slot-popup-1", "slot-popup-2"].map(|label| mgr.windows[label].attrs.maximized)
        };
        assert_eq!(
            maximized.iter().filter(|m| **m).count(),
            1,
            "the hidden popout's maximize follows the copy too"
        );
        let labels = ["slot-popup-1", "slot-popup-2"];
        let mut expected = vec!["focused?".to_string(), "monitors".to_string()];
        for (label, maximized) in labels.iter().zip(maximized) {
            if maximized {
                expected.push(format!("defer_maximize {label} in_model=false"));
            }
            expected.push(format!("open_hidden {label} at=false in_model=false"));
        }
        expected.extend(
            [
                "place_main at=true maximized=true",
                "record main",
                "visible? main",
            ]
            .map(String::from),
        );
        for label in labels {
            expected.push(format!("visible? {label}"));
        }
        for (label, maximized) in labels.iter().zip(maximized) {
            expected.push(format!("hide {label} in_model=true"));
            if !maximized {
                expected.push(format!("record {label}"));
            }
        }
        assert_eq!(
            rig.windows.calls(),
            expected,
            "does not raise popouts beside a hidden main — a hidden maximized popout defers its maximize and does not record it either (so it does not show and hide · so the copy's maximize is not erased) · no window was shown, so focus does not move either"
        );
        let pending: Vec<String> = labels
            .iter()
            .zip(maximized)
            .filter(|(_, maximized)| *maximized)
            .map(|(label, _)| label.to_string())
            .collect();
        assert_eq!(
            rig.windows.deferred(),
            pending,
            "the deferred maximize remains until the tray 「Show」 applies it"
        );
    }

    #[test]
    fn a_popout_window_gone_before_the_commit_is_dropped_from_the_model() {
        let rig = Rig::new(None);
        rig.attach();
        rig.live_popout();
        let version = rig.layout.0.lock().unwrap().version;
        let windows = previous_screen(1, false);
        rig.service
            .set_boot(Some(copy_of(windows)), StateFileStatus::Ok, true);
        rig.windows.gone.lock().unwrap().push("slot-popup-2".into());

        let reply = rig
            .coordinator
            .answer(true)
            .expect("the accept was committed");

        assert_eq!(
            reply.restored_windows, 1,
            "a vanished popout is not counted"
        );
        assert_eq!(rig.windows_in_model(), [MAIN_WINDOW_LABEL]);
        let calls = rig.windows.calls();
        assert!(
            calls.ends_with(&[
                "visible? main".to_string(),
                "visible? slot-popup-2".to_string(),
                "destroy slot-popup-1 in_model=false".to_string(),
            ]),
            "a missing window is neither shown nor recorded: {calls:?}"
        );
        assert_eq!(
            *rig.subs.seen.lock().unwrap(),
            [version + 1, version + 2],
            "recomputes again in the same lock after removing"
        );
        assert!(rig.ports_called_with_the_lock_free());
        assert_eq!(rig.service.status().crash_copy, CrashCopyStatus::Answered);
        // The vanished copy window's `Destroyed` already passed — release the number right away so the next new window gets it (TRD S26-window-chrome §4-1 · §5-6).
        let mut mgr = rig.layout.0.lock().unwrap();
        assert!(mgr.closing_numbers().is_empty());
        mgr.create_window("slot-popup-9").unwrap();
        assert_eq!(mgr.windows["slot-popup-9"].number, 2);
        mgr.assert_number_invariants();
    }

    #[test]
    fn subscriptions_registered_after_the_ports_were_attached_are_used_at_commit() {
        let rig = Rig::new(None);
        rig.attach_without_subs();
        rig.register_subs();
        let windows = previous_screen(0, false);
        rig.service
            .set_boot(Some(copy_of(windows)), StateFileStatus::Ok, true);

        rig.coordinator.answer(true).expect("accept");

        assert_eq!(
            rig.subs.seen.lock().unwrap().len(),
            1,
            "looked up at commit, not when the port is plugged in"
        );
        assert!(
            !rig.windows
                .calls()
                .iter()
                .any(|call| call.starts_with("focus ")),
            "does not pull main forward when no popout remains"
        );
    }

    #[test]
    fn main_takes_the_focus_after_an_accept_only_if_the_app_had_it_before_and_still_has_it() {
        // (another app in front before the answer · the person moved to another app between ②–④ → does it pull main · the number of focus reads)
        for (background, switched_away, takes_focus, samples) in [
            (false, false, true, 2),
            (true, false, false, 1),
            (false, true, false, 2),
        ] {
            let rig = Rig::new(None);
            rig.attach();
            *rig.windows.background.lock().unwrap() = background;
            if switched_away {
                // Even the popout built hidden could not take the front spot from the other app (overrides the `background` that the fake's creation lowers).
                *rig.windows.focus_samples.lock().unwrap() = vec![true, false];
            }
            let windows = previous_screen(1, false);
            rig.service
                .set_boot(Some(copy_of(windows)), StateFileStatus::Ok, true);

            rig.coordinator.answer(true).expect("accept");

            let calls = rig.windows.calls();
            let case = format!("background={background} switched_away={switched_away}");
            assert_eq!(
                calls.first().map(String::as_str),
                Some("focused?"),
                "{case} — read before touching windows: {calls:?}"
            );
            assert!(
                calls
                    .iter()
                    .any(|call| call == "show slot-popup-1 in_model=true"),
                "{case} — there is a shown popout: {calls:?}"
            );
            assert_eq!(
                calls.iter().filter(|call| *call == "focused?").count(),
                samples,
                "{case} — if the sample before the answer is false, it does not read again: {calls:?}"
            );
            assert_eq!(
                calls.iter().any(|call| call.starts_with("focus ")),
                takes_focus,
                "{case} — if another app was or is in front, main is not pulled forward (a fake Alt would steal that app's focus): {calls:?}"
            );
            if takes_focus {
                let focus = calls.iter().position(|call| call == "focus main").unwrap();
                let show = calls
                    .iter()
                    .position(|call| call == "show slot-popup-1 in_model=true")
                    .unwrap();
                assert_eq!(
                    calls[focus - 1],
                    "focused?",
                    "{case} — the re-read is right before giving focus: {calls:?}"
                );
                assert!(
                    focus < show,
                    "{case} — focus goes to main first and then the popouts are shown (popouts above main): {calls:?}"
                );
            }
            assert_eq!(rig.service.status().crash_copy, CrashCopyStatus::Answered);
        }
    }

    #[test]
    fn a_window_that_fails_to_open_undoes_the_ones_made_and_leaves_the_answer_open() {
        let rig = Rig::new(None);
        rig.attach();
        let windows = previous_screen(2, false);
        rig.service
            .set_boot(Some(copy_of(windows)), StateFileStatus::Ok, true);
        let before = rig.fingerprint();
        *rig.windows.fail_open.lock().unwrap() = Some("slot-popup-2".into());

        let err = rig.coordinator.answer(true).unwrap_err();

        assert!(matches!(err, AnswerError::Internal(_)), "{err:?}");
        assert_eq!(
            rig.windows.calls_but_deferrals(),
            [
                "focused?",
                "monitors",
                "open_hidden slot-popup-1 at=false in_model=false",
                "open_hidden slot-popup-2 at=false in_model=false",
                "destroy slot-popup-1 in_model=false",
            ]
        );
        assert_eq!(rig.fingerprint(), before, "the model unchanged");
        assert!(rig.events.tabs.lock().unwrap().is_empty());
        assert!(
            rig.events.order.lock().unwrap().is_empty(),
            "a failure not notified has no notice either"
        );
        assert_eq!(rig.service.status().crash_copy, CrashCopyStatus::Awaiting);
        assert_eq!(
            rig.notices.seen(),
            [CrashCopyStatus::Awaiting, CrashCopyStatus::Awaiting],
            "a revert also notifies awaiting"
        );

        *rig.windows.fail_open.lock().unwrap() = None;
        let reply = rig
            .coordinator
            .answer(true)
            .expect("the marker was released, so it can be answered again");
        assert_eq!(reply.restored_windows, 3);
    }

    #[test]
    fn a_hidden_window_that_will_not_be_destroyed_is_tried_once_more_and_the_answer_stays_open() {
        let rig = Rig::new(None);
        rig.attach();
        let windows = previous_screen(2, false);
        rig.service
            .set_boot(Some(copy_of(windows)), StateFileStatus::Ok, true);
        *rig.windows.fail_open.lock().unwrap() = Some("slot-popup-2".into());
        *rig.windows.fail_destroy.lock().unwrap() = Some("slot-popup-1".into());

        let err = rig.coordinator.answer(true).unwrap_err();

        assert!(matches!(err, AnswerError::Internal(_)), "{err:?}");
        assert_eq!(
            rig.windows.calls_but_deferrals()[4..],
            [
                "destroy slot-popup-1 in_model=false",
                "destroy slot-popup-1 in_model=false",
            ],
            "tries once more and gives up"
        );
        assert_eq!(rig.service.status().crash_copy, CrashCopyStatus::Awaiting);
        assert!(
            rig.service.begin_answer().is_ok(),
            "the marker was released"
        );
    }

    #[test]
    fn a_commit_the_model_refuses_destroys_the_hidden_windows_and_leaves_the_answer_open() {
        let rig = Rig::new(None);
        rig.attach();
        let windows = previous_screen(1, false);
        rig.service
            .set_boot(Some(copy_of(windows)), StateFileStatus::Ok, true);
        let (version, views) = {
            let mgr = rig.layout.0.lock().unwrap();
            (mgr.version, mgr.views.len())
        };
        *rig.windows.squat_on_open.lock().unwrap() = Some("slot-popup-1".into());

        let err = rig.coordinator.answer(true).unwrap_err();

        assert!(matches!(err, AnswerError::Internal(_)), "{err:?}");
        assert_eq!(
            rig.windows.calls(),
            [
                "focused?",
                "monitors",
                "defer_maximize slot-popup-1 in_model=false",
                "open_hidden slot-popup-1 at=false in_model=false",
                "destroy slot-popup-1 in_model=true",
            ],
            "when the commit is refused, the created windows are reaped"
        );
        {
            let mgr = rig.layout.0.lock().unwrap();
            assert_eq!(
                (mgr.version, mgr.views.len()),
                (version + 1, views + 1),
                "unchanged except for the one window that cut in"
            );
            let main = &mgr.windows[MAIN_WINDOW_LABEL];
            assert_ne!(
                mgr.views[&main.active].name,
                TabName::Named("Last tab".into()),
                "main did not change to the copy"
            );
        }
        assert!(
            rig.subs.seen.lock().unwrap().is_empty(),
            "stopped before the commit"
        );
        assert!(rig.events.tabs.lock().unwrap().is_empty());
        assert_eq!(rig.service.status().crash_copy, CrashCopyStatus::Awaiting);
        assert_eq!(
            rig.notices.seen(),
            [CrashCopyStatus::Awaiting, CrashCopyStatus::Awaiting]
        );
        assert!(
            rig.service.begin_answer().is_ok(),
            "the marker was released"
        );
    }

    #[test]
    fn a_label_source_that_collides_with_a_live_popout_fails_before_any_window() {
        // Count only the coordinator separately — it yields the same label as the `slot-popup-1` that is up.
        let rig = Rig::new(Some(Arc::new(SeqLabels::default())));
        rig.attach();
        rig.live_popout();
        let windows = previous_screen(1, false);
        rig.service
            .set_boot(Some(copy_of(windows)), StateFileStatus::Ok, true);
        let before = rig.fingerprint();

        let err = rig.coordinator.answer(true).unwrap_err();

        assert!(matches!(err, AnswerError::Internal(_)), "{err:?}");
        assert!(rig.windows.calls().is_empty(), "stops at preparation");
        assert_eq!(rig.fingerprint(), before);
        assert_eq!(rig.service.status().crash_copy, CrashCopyStatus::Awaiting);
        assert!(
            rig.service.begin_answer().is_ok(),
            "the marker was released"
        );
    }

    #[test]
    fn an_accept_before_the_ports_are_attached_is_refused_but_a_reject_is_not() {
        let rig = Rig::new(None);
        let windows = previous_screen(0, false);
        rig.service
            .set_boot(Some(copy_of(windows)), StateFileStatus::Ok, true);
        let before = rig.fingerprint();

        assert!(matches!(
            rig.coordinator.answer(true),
            Err(AnswerError::NotReady)
        ));
        assert_eq!(rig.service.status().crash_copy, CrashCopyStatus::Awaiting);
        assert_eq!(rig.fingerprint(), before);

        assert_eq!(
            rig.coordinator.answer(false).expect("decline"),
            AnswerReply {
                restored_windows: 0,
                durable: false,
            }
        );
        assert_eq!(rig.service.status().crash_copy, CrashCopyStatus::Answered);
    }

    #[test]
    fn a_reject_answers_without_touching_the_screen() {
        let rig = Rig::new(None);
        rig.attach();
        rig.live_popout();
        let windows = previous_screen(1, false);
        rig.service
            .set_boot(Some(copy_of(windows)), StateFileStatus::Ok, true);
        let before = rig.fingerprint();

        let reply = rig.coordinator.answer(false).expect("decline");

        assert_eq!(
            reply,
            AnswerReply {
                restored_windows: 0,
                durable: false,
            }
        );
        assert_eq!(rig.fingerprint(), before);
        assert!(rig.windows.calls().is_empty());
        assert!(rig.events.tabs.lock().unwrap().is_empty());
        assert_eq!(
            rig.notices.seen(),
            [CrashCopyStatus::Awaiting, CrashCopyStatus::Answered]
        );
    }

    #[test]
    fn an_answer_without_a_copy_or_while_another_is_in_flight_conflicts() {
        let rig = Rig::new(None);
        rig.attach();
        assert!(matches!(
            rig.coordinator.answer(false),
            Err(AnswerError::Conflict(AnswerConflict::NotAwaiting(
                CrashCopyStatus::None
            )))
        ));

        let windows = previous_screen(0, false);
        rig.service
            .set_boot(Some(copy_of(windows)), StateFileStatus::Ok, true);
        let held = rig.service.begin_answer().unwrap();
        for accept in [true, false] {
            assert!(matches!(
                rig.coordinator.answer(accept),
                Err(AnswerError::Conflict(AnswerConflict::InFlight))
            ));
        }
        assert!(rig.windows.calls().is_empty());
        drop(held);
        assert!(rig.coordinator.answer(false).is_ok());
    }

    #[test]
    fn the_coordinator_status_is_the_service_status() {
        let rig = Rig::new(None);
        let windows = previous_screen(1, false);
        rig.service
            .set_boot(Some(copy_of(windows)), StateFileStatus::Ok, true);
        assert_eq!(rig.coordinator.status(), rig.service.status());
    }
}
