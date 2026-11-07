//! 「복원할까요?」 (Restore?) after an abnormal exit (TRD S21-storage §6-7) — three states (`none` · `awaiting` · `answered`) and a marker for the one answer
//! being processed, the restore service ([`RestoreService`]) that holds this run's `state.json` read result ([`StateFileStatus`] · §6-5),
//! and the restore coordinator ([`RestoreCoordinator`]) that executes the answer on top of it.
//! The state is decided in one place by boot step 6 ([`RestoreService::set_boot`] · I5), and the answer goes through the coordinator's single
//! [`RestoreCoordinator::answer`] (the same handle for a human · the LLM).
//!
//! - ★The service lock is a leaf★ — while holding it, make no notification · log · recorder wait · other lock (`ViewManager`).
//!   Call the notification ([`RestoreNotifier`]) after releasing the lock.
//! - ★Dropping the marker without finishing it is the same as `RolledBack`★ (including when released by a panic) — if the in-progress marker stays set,
//!   every later answer gets `InFlight` and can never answer in that run.

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

/// The event emitted to the main window when the restore state changes — payload = [`CrashCopyStatus`].
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
    /// The serde spelling as is — `crash_copy` of the bus `restore.status` is a string because the declaration macro cannot carry the enum in
    /// lowercase spelling.
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
    /// The serde spelling as is — the same reason as [`CrashCopyStatus::as_wire`].
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

/// The unanswered crash copy that boot step 6 hands over.
#[derive(Clone)]
pub struct CrashCopy {
    /// The original text that acceptance will restore — the copy bytes boot read, or, for guard ii (a copy that should have been made could not be), the original text of `state.json`
    /// held in memory (TRD §6-5 ⑥ · L2).
    pub text: String,
    /// `codec::crash_copy_hash(&text)`.
    pub hash: String,
    /// What was read from `text`.
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
    /// Not `Awaiting` (no copy · already answered) — carries the current state.
    NotAwaiting(CrashCopyStatus),
    /// Another answer is being processed (TRD §6-7 「다른 답이 처리 중이면 CONFLICT」).
    InFlight,
}

/// The marker of the one answer being processed — it cannot be cloned. Finish it with [`RestoreService::finish_answer`] (dropping it rolls it back —
/// module header).
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
                "복원 답이 끝맺지 않고 사라졌다 — 처리 중 표지를 되돌린다"
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
    /// The acceptance was committed or declined — the state becomes `Answered` (a notification goes out).
    Answered,
    /// The acceptance failed before commit (preparation · window creation) — only the in-progress marker is released and it stays `Awaiting`. It still
    /// notifies `Awaiting` — a caller rejected with `InFlight` in the meantime must get this notification to know it can answer again.
    RolledBack,
}

/// Called after the boot ⑥ decision · `Answered` · a rollback (still `Awaiting`). ★Called holding no lock★ — the implementation
/// may call the service again. For the rollback of a dropped marker it is called inside `Drop` (which may be running while released by a panic).
pub trait RestoreNotifier: Send + Sync {
    fn changed(&self, status: CrashCopyStatus);
}

pub struct RestoreService {
    cell: Arc<Mutex<Cell>>,
}

struct Cell {
    phase: Phase,
    /// Goes up on every [`RestoreService::set_boot`] — so that a marker from an earlier generation cannot change the current state.
    generation: u64,
    notifier: Option<Arc<dyn RestoreNotifier>>,
    state_file: StateFileStatus,
    /// `durable` while asking also carries this field — leaving no room for the two to diverge.
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
    // Inside the lock only field assignments are made — there is no state half-changed by a panic. The marker rollback must work in `Drop` too.
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
    /// The marker does not belong to the current state — nothing was changed.
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
                "복원 답을 되돌렸다 — 다시 답할 수 있다"
            );
            notify(notifier, CrashCopyStatus::Awaiting);
        }
        Settled::Stale => tracing::warn!(
            module = "state",
            hash = %hash,
            ?end,
            "지금 상태의 것이 아닌 복원 답 표지 — 무시한다"
        ),
    }
}

impl Default for RestoreService {
    fn default() -> Self {
        Self::new()
    }
}

impl RestoreService {
    /// State `None` · `state_file` `Ok` · `saves` `true` · no notification — the values before boot step 6 are the side with nothing to notify
    /// (windows come into being after ⑥ and never see this value — I5).
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

    /// Called only at boot step 6 (I5) — `copy` is `None` = no copy to ask about · `saves` = whether this run saves screen state
    /// (not a guard — [`RestoreStatusView::saves`]). A marker that was in progress becomes invalid.
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
                "복원 상태를 부팅 단계 밖에서 다시 정했다 — 처리 중이던 답은 무효가 된다"
            );
        }
        match hash {
            Some(hash) => tracing::info!(
                module = "state",
                hash = %hash,
                saves,
                "답하지 않은 크래시 사본이 있다 — 복원할지 묻는다"
            ),
            None => tracing::info!(module = "state", "물을 크래시 사본이 없다"),
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

    /// Set the in-progress marker and hand over the copy.
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
            "다른 서비스가 낸 표지"
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
    /// while holding that hash (`saver` header 「해결 칸」).
    pub durable: bool,
}

#[derive(Debug)]
pub enum AnswerError {
    /// Not `awaiting` · another answer is being processed → bus `CONFLICT`.
    Conflict(AnswerConflict),
    /// The shell is still starting up and has no window port, so it could not take the acceptance — nothing changed and it is still `awaiting`. When shell setup
    /// ends the same answer stands → bus `INTERNAL`. Same code as [`Self::Internal`] and only the wording differs (「곧 다시 답하라」, answer again soon).
    NotReady,
    /// The acceptance failed before commit (preparation · window creation · commit rejection) — nothing changed and it is still `awaiting` → bus
    /// `INTERNAL`.
    Internal(String),
}

// The `message` of the bus error and the error string of the Tauri wrapper — tell the kind apart by the code (bus) or `restore_status`, not by
//   this text.
impl fmt::Display for AnswerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AnswerError::Conflict(AnswerConflict::NotAwaiting(CrashCopyStatus::Answered)) => {
                f.write_str("the crash copy was already answered in this run (crash_copy is answered)")
            }
            // `begin_answer` does not carry `Awaiting` — what remains is `None`.
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

/// The OS window port for runtime restore acceptance (TRD §6-7 ② ④) — production = `placement::TauriRestoreWindows`, test = a fake.
///
/// ★All of it is called holding no lock★ — window creation · setters wait for the event loop, and the usage-interest notification in `destroy`'s `Destroyed`
/// handling · `record_placement` · `set_shown` takes the layout lock (the same
/// discipline as `layout::WindowHost`).
pub trait RestoreWindows: Send + Sync {
    /// Does our app have focus now (the foreground window is ours) — `false` if it cannot be read.
    fn app_has_focus(&self) -> bool;
    /// The current monitors — an empty list if it cannot be read (all saved placements are discarded).
    fn monitors(&self) -> Vec<MonitorArea>;
    /// Create popout window `label` hidden and place it at `at` (the saved normal placement · the monitor it goes to) — `None` = the label's
    /// default placement. `Err` = there is no window (the deferral that was set was also withdrawn).
    ///
    /// `maximized` = defer the copy's maximize until that window is first shown — maximizing a hidden window shows and then hides it. While deferred,
    /// that window's placement is not recorded ([`Self::record_placement`] · `Moved` · `Resized`), and the show (`set_shown` `true` ·
    /// the tray show) applies the maximize and clears it. If the window is destroyed, it is cleared. ★Set the deferral before creating the window★ — if set after,
    /// and in between the user closes a window the tray 「보이기」 revealed, destruction cleanup passes before the deferral and the deferral
    /// stays.
    fn open_hidden(
        &self,
        label: &str,
        at: Option<(WindowBounds, Landing)>,
        maximized: bool,
    ) -> Result<(), String>;
    /// Is that window visible now — `None` = there is no window. If visibility cannot be read, `Some(true)` (the window exists).
    fn visibility(&self, label: &str) -> Option<bool>;
    /// Apply the copy's normal placement (`at` — `None` = leave it at the current placement) and maximize to main. Maximize is followed both ways —
    /// if `false`, unmaximize it and also clear the deferred maximize. The maximize of a hidden main is deferred until it is first shown, but ★set that deferral before applying the
    /// placement★ — if set after, the `Moved` · `Resized` emitted by applying the placement are recorded in between as 「최대화 아님」 (not maximized).
    /// A visible main is also applied with the same deferral set and cleared only if applied — ★if it could not be applied, it stays★ and meanwhile main's placement is not recorded
    /// (including [`Self::record_placement`]), and the next show applies it again.
    fn place_main(&self, at: Option<(WindowBounds, Landing)>, maximized: bool);
    /// Show (`true`) or hide, and tell usage interest that visibility — the same order as the tray show · hide (ADR-0229 ·
    /// `tray::actions::for_each_ui_window`). The show comes after that window's first paint (right away if the notification already came · otherwise at the time limit
    /// — `first_paint`) — that show applies the deferred maximize. The show activates and puts that window in front
    /// (`placement::show_placed`). Hiding withdraws the wait for that first paint. If there is no window, it does nothing.
    fn set_shown(&self, label: &str, shown: bool);
    /// Record that window's current placement in the model — called after the window is in the model (a `Moved` before that is dropped). A window whose maximize
    /// is deferred is not recorded.
    fn record_placement(&self, label: &str);
    /// Give that window focus — it pulls it even if another app is in front. If the window is hidden or minimized, nothing happens (tao 0.35.3
    /// Windows `set_focus`). If there is no window, it does nothing.
    fn focus(&self, label: &str);
    /// `Ok` if there is no window.
    fn destroy(&self, label: &str) -> Result<(), String>;
}

/// Look it up every time the subscription-recompute port is used (before taking the lock) — `None` = there is no daemon client: skip the recompute (there is no way for output
/// to come). If looked up only once when the port is plugged in, a client registered after that would be silently skipped.
pub trait SubscriptionSource: Send + Sync {
    fn current(&self) -> Option<Arc<dyn SubscriptionSync>>;
}

/// The ports the coordinator receives after the shell is up — the window port · notification · subscription source · theme push all need an `AppHandle` to stand.
pub struct RestorePorts {
    pub windows: Arc<dyn RestoreWindows>,
    pub events: Arc<dyn LayoutEvents>,
    pub subs: Arc<dyn SubscriptionSource>,
    /// The one effective theme per shell — it sees the same model as the coordinator.
    pub themes: ThemeControl,
}

/// The single path of runtime restore accept · decline (TRD §6-7 ①–⑤) — the human (Tauri wrapper) and the LLM (bus `restore.answer`) call the same
/// instance.
///
/// - ★There is one commit point (③ · inside one `ViewManager` lock)★ — a failure before it (① preparation · ② creating hidden windows) withdraws the windows
///   it created and rolls back to `awaiting`, and a failure after it (④ theme push · notification · placement · show · removing vanished popouts · withdrawing
///   old popouts) only leaves a log.
/// - ★Restored popouts follow main's visibility (ADR-0229)★ — so that no popout appears alone next to a hidden main, they are kept hidden and the tray
///   「보이기」 reveals them together. The copy's maximize is applied when that window is first shown ([`RestoreWindows::open_hidden`]).
/// - ★Lock order★: the service field · session field are leaves. Window ports · notifications · waiting on the recorder are done holding no lock.
/// - The ports ([`RestorePorts`]) are made in the builder and received with [`Self::attach`] at the end of shell setup — an acceptance before that is
///   [`AnswerError::NotReady`] (still `awaiting` · it can answer again) and a decline stands without ports.
pub struct RestoreCoordinator {
    service: Arc<RestoreService>,
    layout: LayoutState,
    session: Arc<StateSession>,
    /// The same issuer as runtime popouts — a new label does not collide with a window that is up (§6-3).
    labels: Arc<dyn LabelSource>,
    ports: OnceLock<RestorePorts>,
}

// The result of ① — nothing has been changed yet.
struct Prepared {
    layout: ViewManager,
    main: WindowAttrs,
    // The new popouts (in label issue order) — that window's copy attributes.
    popouts: Vec<(WindowLabel, WindowAttrs)>,
}

// What ③ took inside the lock — ④ uses it outside the lock.
struct Committed {
    removed: Vec<WindowLabel>,
    layouts: Vec<ViewSnapshot>,
    tabs: Vec<WindowTabsPayload>,
}

// The OS-side circumstances read before ② — used by ② and ④.
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

    /// Once at the end of shell setup — a second call is dropped.
    pub fn attach(&self, ports: RestorePorts) {
        if self.ports.set(ports).is_err() {
            tracing::warn!(
                module = "state",
                "복원 조율자의 포트를 두 번 꽂으려 했다 — 처음 것을 쓴다"
            );
        }
    }

    pub fn status(&self) -> RestoreStatusView {
        self.service.status()
    }

    /// Answer the crash copy — `accept` replaces the screen with the copy (①–④), and both settle the answer and attach it to the recorder (⑤).
    ///
    /// ★It blocks — do not call it from the main (event loop) thread★: while creating · placing · withdrawing windows it waits
    /// for the event loop, and it waits for the recorder's answer until the deadline ([`saver::REPLY_DEADLINE`](super::saver::REPLY_DEADLINE) — 2 seconds).
    /// Called from main, window creation waits for itself and hangs. The caller hands it off with an async command + `spawn_blocking`.
    /// Call it holding no lock.
    pub fn answer(&self, accept: bool) -> Result<AnswerReply, AnswerError> {
        let ticket = self.service.begin_answer().map_err(AnswerError::Conflict)?;
        let hash = ticket.copy().hash.clone();
        let restored_windows = if accept {
            let Some(ports) = self.ports.get() else {
                tracing::warn!(
                    module = "state",
                    hash = %hash,
                    "셸이 아직 뜨는 중이라 복원 수락을 받지 못했다 — 화면은 그대로이고 다시 답할 수 있다"
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
                        "복원 수락이 커밋 전에 실패했다 — 화면은 그대로이고 다시 답할 수 있다"
                    );
                    self.service.finish_answer(ticket, AnswerEnd::RolledBack);
                    return Err(AnswerError::Internal(reason));
                }
            }
        } else {
            0
        };
        // ⑤ — the service emits the notification (`restore:changed`) outside the lock.
        self.service.finish_answer(ticket, AnswerEnd::Answered);
        let durable = self.session.resolve_crash_copy(hash.clone()) == ResolveResult::Durable;
        tracing::info!(
            module = "state",
            hash = %hash,
            accept,
            restored_windows,
            durable,
            "크래시 사본에 답했다"
        );
        Ok(AnswerReply {
            restored_windows,
            durable,
        })
    }

    // ①–④. `Err` = a failure before commit — the windows it created were withdrawn.
    fn accept(&self, ports: &RestorePorts, copy: &CrashCopy) -> Result<u32, String> {
        let prepared = self.prepare(copy)?;
        // Focus is read before touching even one window (before ②) — if read after a popout built hidden (`focused(true)`) took focus,
        //   it could miss that another app was in front before the answer (defensive — [unverified — inference]).
        let desktop = Desktop {
            app_had_focus: ports.windows.app_has_focus(),
            monitors: ports.windows.monitors(),
        };
        // A new popout is created before it enters the model, so that window's background · first theme · first pull are decided in advance to see the copy's window theme —
        //   if not decided, it is painted once with the global theme. After commit the model answers, so it is withdrawn regardless of success or failure.
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
        // The copy's window theme is in the model — push it so the main that is up receives that value. New popouts are already painted with the value decided before ②
        //   and have pulled it (TRD S21-storage §6-7).
        // ADR-0265
        ports.themes.push();

        let kept = self.after_commit(ports, committed, main, &popouts, &desktop);
        Ok(count(1 + kept))
    }

    // ① — no side effects (it only uses issuer numbers — they are monotonic, so discarding is fine).
    fn prepare(&self, copy: &CrashCopy) -> Result<Prepared, String> {
        let Restored { layout, warnings } =
            ViewManager::from_persisted(copy.file.windows.clone(), self.labels.as_ref());
        warnings.iter().for_each(report_restore_warning);
        if let Some(RestoreWarning::Internal(reason)) = warnings
            .iter()
            .find(|w| matches!(w, RestoreWarning::Internal(_)))
        {
            return Err(format!("복원 모델을 세우지 못했다: {reason}"));
        }
        let main = layout
            .window_attrs(MAIN_WINDOW_LABEL)
            .map_err(|e| format!("복원 모델에 main 이 없다: {e}"))?;
        let mut popouts: Vec<(WindowLabel, WindowAttrs)> = layout
            .windows
            .iter()
            .filter(|(label, _)| label.as_str() != MAIN_WINDOW_LABEL)
            .map(|(label, window)| (label.clone(), window.attrs))
            .collect();
        // Same prefix · same run marker (one issuer) + a decimal sequence number, so (length, characters) order is issue order.
        popouts.sort_by_key(|(label, _)| (label.len(), label.clone()));
        {
            let live = self
                .layout
                .0
                .lock()
                .map_err(|_| "레이아웃 락에 독이 들었다".to_string())?;
            if let Some((label, _)) = popouts.iter().find(|(l, _)| live.windows.contains_key(l)) {
                return Err(format!(
                    "새 팝아웃 label {label} 이 떠 있는 창과 겹친다 — label 발급기가 런타임 팝아웃과 다르다"
                ));
            }
        }
        Ok(Prepared {
            layout,
            main,
            popouts,
        })
    }

    // ③ — the only commit point. Model replacement · notification material · subscription recompute in one critical section (`layout::apply` 「락 규율」).
    fn commit(&self, ports: &RestorePorts, restored: ViewManager) -> Result<Committed, String> {
        let subs = ports.subs.current();
        let mut mgr = self.layout.0.lock().map_err(|_| {
            tracing::error!(
                module = "state",
                "레이아웃 락에 독이 들어 복원한 화면을 들이지 못했다"
            );
            "레이아웃 락에 독이 들었다".to_string()
        })?;
        let removed = mgr
            .adopt_restored(restored)
            .map_err(|e| format!("복원 모델을 들이지 못했다: {e}"))?;
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

    // ④ — not rolled back. The port implementation leaves failures in the log. What it returns = the number of new popouts left in the model.
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
        // If the copy's placement was discarded or could not be applied, the model holds a placement the window never went to — align it to the current placement. If main's maximize
        //   could not be applied, main is not recorded (the deferral stays — `place_main`).
        windows.record_placement(MAIN_WINDOW_LABEL);

        let main_shown = windows.visibility(MAIN_WINDOW_LABEL) != Some(false);
        // If, between ② and ③, the user closed a window the tray 「보이기」 revealed, that close request · `Destroyed` cleanup passed with only a tombstone left, because
        //   the label was not yet in the model (the adoption at ③ does not look at tombstones) — if not removed here, a window without an OS window
        //   stays in the model.
        let (vanished, live): (Vec<_>, Vec<_>) = popouts
            .iter()
            .partition(|(label, _)| windows.visibility(label).is_none());
        let kept = live.len();
        // Give focus to main first, and the restored popouts are shown at first paint and come above it (user decision 2026-10-20 — main is
        //   big · TRD S21-storage §6-7 ④). The popout show is activating, so even if its first paint already came, it comes after this, so in whichever
        //   order they arrive the popouts are on top (`set_shown` → `first_paint::reveal_held`). Since a popout built hidden may have
        //   taken focus (`focused(true)` — `commands::popout` · defensive — [unverified — inference]), the focus-giving raises main in front of the other app
        //   and returns it in front of the person who pressed 「복원」 (restore) — the popouts are activated at their first paint after that and appear above main.
        //   If no popouts remain, it does not pull — if none were built, focus did not move, and if they vanished, the person already touched it.
        // ★Only when our window had focus before the answer and still has it now★ — if another app is in front, the focus-giving fires a fake
        //   Alt key at that app and steals its focus (tao 0.35.3 Windows `set_focus` → `force_window_active`). While the LLM answers in the background,
        //   the person may be working in another app. The sample from before the answer alone misses that the person moved to another app between ②–④ —
        //   read again right before the call (the earlier sample is still used — if a popout built hidden took focus, this
        //   value alone would miss that another app was in front before the answer).
        if desktop.app_had_focus && main_shown && kept > 0 && windows.app_has_focus() {
            windows.focus(MAIN_WINDOW_LABEL);
        }
        for (label, attrs) in live {
            windows.set_shown(label, main_shown);
            // A maximized popout kept hidden is not recorded — its maximize was deferred by ② until it is shown, and recording the current (normal) placement would
            //   erase the copy's maximize from the model. A window that will be shown has the maximize applied and its placement recorded by the first-paint show — the recording before that
            //   is blocked by the deferral (`placement::record`).
            if main_shown || !attrs.maximized {
                windows.record_placement(label);
            }
        }
        let vanished: Vec<WindowLabel> = vanished
            .into_iter()
            .map(|(label, _)| label.clone())
            .collect();
        self.forget_vanished(ports, &vanished);
        // That window's `Destroyed` cleanup only does the recompute · tombstone, since the label is already not in the model (`popout::drop_window_in_model`).
        destroy_all(windows, &removed, Leftover::Replaced);
        kept
    }

    // Remove from the model a new popout whose OS window vanished — the same job as window destruction cleanup (removing the window + subscription recompute in one lock —
    //   `popout::drop_window_in_model`). Like that path, the frontend is not told.
    fn forget_vanished(&self, ports: &RestorePorts, labels: &[WindowLabel]) {
        if labels.is_empty() {
            return;
        }
        tracing::warn!(
            module = "state",
            ?labels,
            "복원한 팝아웃 창이 화면을 바꾸기 전에 닫혀 모델에서 지운다"
        );
        let subs = ports.subs.current();
        let Ok(mut mgr) = self.layout.0.lock() else {
            tracing::error!(
                module = "state",
                ?labels,
                "레이아웃 락에 독이 들어 닫힌 복원 팝아웃을 모델에서 지우지 못했다 — 창 없는 창이 모델에 남는다"
            );
            return;
        };
        for label in labels {
            if let Err(e) = mgr.drop_window_gone(label) {
                tracing::debug!(module = "state", label = %label, error = %e, "이미 모델에 없는 복원 팝아웃");
            }
        }
        if let Some(subs) = &subs {
            subs.resync(&mgr);
        }
    }
}

// ② — if even one cannot be created, withdraw everything created and return `Err`.
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
            return Err(format!("복원할 팝아웃 창 {label} 을 못 만들었다: {e}"));
        }
        created.push(label.clone());
    }
    Ok(created)
}

// Which window could not be withdrawn — it only distinguishes the wording of the last failure log.
#[derive(Clone, Copy)]
enum Leftover {
    /// A restored popout built hidden, withdrawn in the rollback.
    HiddenRestored,
    /// An old popout removed from the model while swapping in the copy's screen.
    Replaced,
}

// Withdraw them one by one, and on failure once more — if it still fails, leave that label at error.
fn destroy_all(windows: &dyn RestoreWindows, labels: &[WindowLabel], leftover: Leftover) {
    for label in labels {
        let Err(first) = windows.destroy(label) else {
            continue;
        };
        tracing::warn!(module = "state", label = %label, error = %first, "창을 거두지 못해 한 번 더 거둔다");
        let Err(e) = windows.destroy(label) else {
            continue;
        };
        match leftover {
            Leftover::HiddenRestored => tracing::error!(
                module = "state",
                label = %label,
                error = %e,
                "복원하려고 숨긴 채 만든 창을 거두지 못했다 — 모델에 없는 숨은 창이 남고, 트레이 「보이기」가 그 창을 드러낸다"
            ),
            Leftover::Replaced => tracing::error!(
                module = "state",
                label = %label,
                error = %e,
                "사본 화면으로 바꾼 뒤 옛 팝아웃 창을 거두지 못했다 — 모델에 없는 창이 화면에 남는다"
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
        /// Was the service lock free at the moment of notification — it also holds the service field.
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
            name: "탭".into(),
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
            text: "원문".into(),
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
                "처리 중에도 awaiting — {saves}"
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
                "물을 사본이 없어도 선다 — {saves}"
            );

            let (service, _recorder) = service();
            service.set_boot(Some(crash_copy("h")), StateFileStatus::Ok, saves);
            let awaiting = service.status();
            assert_eq!(awaiting.saves, saves, "awaiting — {saves}");
            assert_eq!(
                awaiting.durable,
                Some(awaiting.saves),
                "묻는 동안 둘은 같다 — {saves}"
            );

            let ticket = service.begin_answer().unwrap();
            assert_eq!(service.status().saves, saves, "처리 중 — {saves}");
            service.finish_answer(ticket, AnswerEnd::RolledBack);
            assert_eq!(service.status().saves, saves, "되돌린 뒤 — {saves}");

            let ticket = service.begin_answer().unwrap();
            service.finish_answer(ticket, AnswerEnd::Answered);
            assert_eq!(
                service.status(),
                RestoreStatusView {
                    saves,
                    ..idle(CrashCopyStatus::Answered)
                },
                "답한 뒤에도 그 실행의 값 그대로 — {saves}"
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
            "사본이 없어도 선다"
        );
        assert_eq!(
            recorder.seen(),
            [CrashCopyStatus::None],
            "알림의 실을 것은 그대로"
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
            "답해도 그 실행의 값 그대로"
        );
    }

    #[test]
    fn an_answer_moves_to_answered_notifies_and_refuses_a_second_answer() {
        let (service, recorder) = service();
        service.set_boot(Some(crash_copy("h")), StateFileStatus::Ok, true);

        let ticket = service.begin_answer().expect("답할 수 있다");
        assert_eq!(ticket.copy().hash, "h");
        assert_eq!(ticket.copy().text, "원문");
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
            "처리 중에도 상태는 awaiting"
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
            "되돌림도 알린다 — 그 사이 InFlight 로 거절당한 쪽이 다시 답할 수 있음을 안다"
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
        assert!(service.begin_answer().is_ok(), "표지가 풀렸다");
    }

    #[test]
    fn a_ticket_unwound_by_a_panic_rolls_back() {
        let (service, _recorder) = service();
        service.set_boot(Some(crash_copy("h")), StateFileStatus::Ok, true);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ticket = service.begin_answer().unwrap();
            panic!("수락 도중 패닉");
        }));

        assert!(result.is_err());
        assert!(service.begin_answer().is_ok());
    }

    #[test]
    fn a_ticket_from_before_a_second_boot_setting_changes_nothing() {
        let (service, recorder) = service();
        service.set_boot(Some(crash_copy("옛")), StateFileStatus::Ok, true);
        let stale = service.begin_answer().unwrap();

        service.set_boot(Some(crash_copy("새")), StateFileStatus::Ok, true);
        service.finish_answer(stale, AnswerEnd::Answered);

        assert_eq!(service.status().crash_copy, CrashCopyStatus::Awaiting);
        assert_eq!(
            recorder.seen(),
            [CrashCopyStatus::Awaiting, CrashCopyStatus::Awaiting],
            "옛 표지의 답은 알리지 않는다"
        );
        let fresh = service.begin_answer().expect("새 세대는 처리 중이 아니다");
        assert_eq!(fresh.copy().hash, "새");
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
            "부팅 · 되돌림 · 버린 표지 · 답"
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
            "알림 안에서 본 상태가 이미 정해진 값이다"
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
                "버스 철자(as_wire)와 Tauri 철자(serde)가 같다"
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
                "버스 철자(as_wire)와 Tauri 철자(serde)가 같다"
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

    /// The previous run's screen — main tab 「지난 탭」 (placement on the monitor · `main_maximized`) · `popouts` popouts (the first = an unknown
    /// content slot · a placement overlapping no monitor · maximized).
    fn previous_screen(popouts: usize, main_maximized: bool) -> Vec<WindowEntry> {
        let mut prev = ViewManager::new();
        let main_view = prev.windows[MAIN_WINDOW_LABEL].active;
        prev.rename_tab(main_view, "지난 탭".into()).unwrap();
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
            text: "원문".into(),
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

    /// Fake window port — records the order called, and on each call checks whether the layout lock was free · whether that label was in the model.
    #[derive(Default)]
    struct FakeWindows {
        layout: LayoutState,
        calls: Mutex<Vec<String>>,
        lock_was_free: Mutex<Vec<bool>>,
        /// Window creation for this label fails.
        fail_open: Mutex<Option<String>>,
        /// When creating the window for this label, a window with the same label intrudes into the model (triggers the commit rejection of ③).
        squat_on_open: Mutex<Option<String>>,
        /// A window whose visibility is 「숨음」 (hidden).
        hidden: Mutex<Vec<String>>,
        /// A window that answers there is no OS window.
        gone: Mutex<Vec<String>>,
        /// Withdrawing this label always fails.
        fail_destroy: Mutex<Option<String>>,
        /// A window whose maximize is deferred until it is shown — the show applies and clears it (the production port's contract).
        deferred: Mutex<Vec<String>>,
        /// Another app is in front (none of our windows has focus). Assuming a popout built hidden takes focus (production builds it with
        /// `focused(true)`), creation lowers this value — reading focus after ② would be fooled by this value.
        background: Mutex<bool>,
        /// Decide the answers of the focus read one by one from the front — when used up, answer with `background`.
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

        /// A call without the deferred maximize — if there are two or more popouts, which new label receives the copy's maximized popout differs every time
        /// (the copy's popout order = persistent id order).
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
                return Err("창 생성 실패(시험)".into());
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
                return Err("창 거두기 실패(시험)".into());
            }
            Ok(())
        }
    }

    /// Notification — records whether the layout lock was free at the first notification moment. `order` = the order the notification came in.
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

    /// Subscription recompute — called inside the layout lock. Records the model number at that moment.
    #[derive(Default)]
    struct FakeSubs {
        seen: Mutex<Vec<u64>>,
    }

    impl SubscriptionSync for FakeSubs {
        fn resync(&self, mgr: &ViewManager) {
            self.seen.lock().unwrap().push(mgr.version);
        }
    }

    /// The window side of the theme push — live webviews = the model's windows. Records what was sent and whether the layout lock was free when sending.
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

    /// Subscription source — if empty, it is a shell that has no daemon client yet. Records whether the lock was free when looking up.
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

    // Issue monotonically in the form without a run marker (`slot-popup-<n>`) — these tests use it to write the expected label literally. The form of the real
    //   issuer (`PopupCounter`) and that (length, characters) order is issue order even in that form are measured by the `commands::popout` tests.
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
        /// `labels` = the coordinator's issuer — if `None`, the same one as runtime popouts (`self.labels`).
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

        /// A shell with a daemon client up — the subscription source yields `self.subs`.
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

        /// Bring up one runtime popout (the same issuer) — a View holding that label · an unknown content slot.
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

        let reply = rig.coordinator.answer(true).expect("수락");

        assert_eq!(
            reply,
            AnswerReply {
                restored_windows: 2,
                durable: false,
            },
            "main + 팝아웃 하나 · 기록기가 없다"
        );
        {
            let mgr = rig.layout.0.lock().unwrap();
            let mut labels = mgr.list_windows();
            labels.sort();
            assert_eq!(
                labels,
                [MAIN_WINDOW_LABEL, "slot-popup-2"],
                "옛 팝아웃은 지우고 새 팝아웃은 떠 있는 창과 겹치지 않는 label 을 받는다"
            );
            assert_eq!(
                (mgr.version, mgr.attrs_rev()),
                (version + 1, attrs_rev + 1),
                "번호는 지금 값에서 하나씩 오른다"
            );
            let main = &mgr.windows[MAIN_WINDOW_LABEL];
            assert_eq!(
                mgr.views[&main.active].name,
                TabName::Named("지난 탭".into())
            );
            assert_eq!(main.attrs.bounds, Some(rect(100.0, 80.0, 1200.0, 800.0)));
            assert!(!mgr.views.contains_key(&live_view));
            let popout = &mgr.windows["slot-popup-2"];
            assert!(popout.attrs.maximized);
            let view = popout.active;
            assert_eq!(
                mgr.unknown_content(view, first_slot(&mgr, view)),
                Some(&foreign()),
                "모르는 내용 원문이 View 와 함께 온다"
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
            "포커스 읽기 → 숨긴 채 만들고(최대화는 만들기 전에 미룬다 · 모델 밖 · 안 걸치는 자리는 버림) → 커밋 → 자리 → 지금 자리 적기 → 포커스 다시 읽기 → main 에 포커스 → 팝아웃 보이기(미룬 최대화 — main 포커스 뒤라 팝아웃이 위) → 옛 창 거두기"
        );
        assert!(rig.windows.deferred().is_empty(), "보인 창의 미룸은 거뒀다");
        assert!(
            rig.ports_called_with_the_lock_free(),
            "창 포트 · 구독 원천은 락 밖에서 부른다"
        );
        assert_eq!(
            *rig.subs.seen.lock().unwrap(),
            [version + 1],
            "재계산은 커밋과 같은 임계구역"
        );
        assert_eq!(
            *rig.events.first_seen.lock().unwrap(),
            Some(true),
            "알림은 락 밖"
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
            "탭마다 레이아웃"
        );
        assert_eq!(
            *rig.events.order.lock().unwrap(),
            ["layout", "layout", "tabs", "tabs"],
            "레이아웃 알림 → 탭 알림"
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

    /// ★Acceptance pushes the copy's window theme★ — main is already up, and a new popout may have pulled its first value before entering the model
    /// (then the global value). Push it before the show · outside the layout lock.
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

        rig.coordinator.answer(true).expect("수락");

        let mut sent = rig.theme_screen.sent.lock().unwrap().clone();
        sent.sort();
        let mut themes: Vec<String> = sent.iter().map(|(_, theme)| theme.clone()).collect();
        themes.sort();
        assert_eq!(sent.len(), 3, "main · 새 팝아웃 둘: {sent:?}");
        assert!(sent.contains(&(MAIN_WINDOW_LABEL.to_string(), "light".to_string())));
        assert_eq!(
            themes,
            ["dark", "e-ink", "light"],
            "새 팝아웃 하나는 사본의 테마, 하나는 전역 값: {sent:?}"
        );
        assert!(
            rig.theme_screen
                .lock_was_free
                .lock()
                .unwrap()
                .iter()
                .all(|free| *free),
            "레이아웃 락 밖에서 민다"
        );

        // Even if a new popout is created before it enters the model, it is painted with the copy's theme from the start — the answer at creation equals the pushed value.
        let opened = rig.windows.opened_with.lock().unwrap().clone();
        assert_eq!(opened.len(), 2, "{opened:?}");
        for window in &opened {
            assert!(sent.contains(window), "만들 때 {window:?} · 민 값 {sent:?}");
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
            "만들 때는 사본의 테마였다"
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

        let reply = rig.coordinator.answer(true).expect("수락");

        assert_eq!(reply.restored_windows, 3);
        // The copy's popout order is persistent id order, so which new label receives the maximized popout differs every time — read it from the model.
        let maximized = {
            let mgr = rig.layout.0.lock().unwrap();
            assert!(
                mgr.windows[MAIN_WINDOW_LABEL].attrs.maximized,
                "모델은 사본의 main 최대화를 쥔다"
            );
            ["slot-popup-1", "slot-popup-2"].map(|label| mgr.windows[label].attrs.maximized)
        };
        assert_eq!(
            maximized.iter().filter(|m| **m).count(),
            1,
            "숨긴 팝아웃의 최대화도 사본대로"
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
            "숨은 main 곁에 팝아웃을 띄우지 않는다 — 숨긴 최대화 팝아웃은 최대화를 미루고 기록도 안 한다(보였다 숨지 않게 · 사본의 최대화를 지우지 않게) · 보인 창이 없으니 포커스도 안 옮긴다"
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
            "미룬 최대화는 트레이 「보이기」가 입힐 때까지 남는다"
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

        let reply = rig.coordinator.answer(true).expect("수락은 커밋됐다");

        assert_eq!(reply.restored_windows, 1, "사라진 팝아웃은 세지 않는다");
        assert_eq!(rig.windows_in_model(), [MAIN_WINDOW_LABEL]);
        let calls = rig.windows.calls();
        assert!(
            calls.ends_with(&[
                "visible? main".to_string(),
                "visible? slot-popup-2".to_string(),
                "destroy slot-popup-1 in_model=false".to_string(),
            ]),
            "없는 창은 보이지도 적지도 않는다: {calls:?}"
        );
        assert_eq!(
            *rig.subs.seen.lock().unwrap(),
            [version + 1, version + 2],
            "지운 뒤 같은 락 안에서 다시 재계산한다"
        );
        assert!(rig.ports_called_with_the_lock_free());
        assert_eq!(rig.service.status().crash_copy, CrashCopyStatus::Answered);
        // The `Destroyed` of the vanished copy window has already passed — release the number right away and the next new window gets it (TRD S26-window-chrome
        //   §4-1 · §5-6).
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

        rig.coordinator.answer(true).expect("수락");

        assert_eq!(
            rig.subs.seen.lock().unwrap().len(),
            1,
            "포트를 꽂을 때가 아니라 커밋 때 찾는다"
        );
        assert!(
            !rig.windows
                .calls()
                .iter()
                .any(|call| call.starts_with("focus ")),
            "남은 팝아웃이 없으면 main 을 끌어오지 않는다"
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

            rig.coordinator.answer(true).expect("수락");

            let calls = rig.windows.calls();
            let case = format!("background={background} switched_away={switched_away}");
            assert_eq!(
                calls.first().map(String::as_str),
                Some("focused?"),
                "{case} — 창을 만지기 전에 읽는다: {calls:?}"
            );
            assert!(
                calls
                    .iter()
                    .any(|call| call == "show slot-popup-1 in_model=true"),
                "{case} — 보인 팝아웃이 있다: {calls:?}"
            );
            assert_eq!(
                calls.iter().filter(|call| *call == "focused?").count(),
                samples,
                "{case} — 답 전의 표본이 거짓이면 다시 읽지 않는다: {calls:?}"
            );
            assert_eq!(
                calls.iter().any(|call| call.starts_with("focus ")),
                takes_focus,
                "{case} — 다른 앱이 앞에 있었거나 지금 있으면 main 을 끌어오지 않는다(가짜 Alt 로 그 앱의 포커스를 뺏는다): {calls:?}"
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
                    "{case} — 다시 읽기는 포커스 주기 바로 앞이다: {calls:?}"
                );
                assert!(
                    focus < show,
                    "{case} — main 에 먼저 포커스를 주고 팝아웃을 보인다(팝아웃이 main 위): {calls:?}"
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
        assert_eq!(rig.fingerprint(), before, "모델은 그대로");
        assert!(rig.events.tabs.lock().unwrap().is_empty());
        assert!(
            rig.events.order.lock().unwrap().is_empty(),
            "알리지 않은 실패는 알림도 없다"
        );
        assert_eq!(rig.service.status().crash_copy, CrashCopyStatus::Awaiting);
        assert_eq!(
            rig.notices.seen(),
            [CrashCopyStatus::Awaiting, CrashCopyStatus::Awaiting],
            "되돌림도 awaiting 을 알린다"
        );

        *rig.windows.fail_open.lock().unwrap() = None;
        let reply = rig
            .coordinator
            .answer(true)
            .expect("표지가 풀려 다시 답할 수 있다");
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
            "한 번 더 해 보고 그만둔다"
        );
        assert_eq!(rig.service.status().crash_copy, CrashCopyStatus::Awaiting);
        assert!(rig.service.begin_answer().is_ok(), "표지가 풀렸다");
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
            "커밋이 거절되면 만든 창을 거둔다"
        );
        {
            let mgr = rig.layout.0.lock().unwrap();
            assert_eq!(
                (mgr.version, mgr.views.len()),
                (version + 1, views + 1),
                "끼어든 창 하나 말고는 그대로"
            );
            let main = &mgr.windows[MAIN_WINDOW_LABEL];
            assert_ne!(
                mgr.views[&main.active].name,
                TabName::Named("지난 탭".into()),
                "main 은 사본으로 안 바뀌었다"
            );
        }
        assert!(rig.subs.seen.lock().unwrap().is_empty(), "커밋 전에 멈췄다");
        assert!(rig.events.tabs.lock().unwrap().is_empty());
        assert_eq!(rig.service.status().crash_copy, CrashCopyStatus::Awaiting);
        assert_eq!(
            rig.notices.seen(),
            [CrashCopyStatus::Awaiting, CrashCopyStatus::Awaiting]
        );
        assert!(rig.service.begin_answer().is_ok(), "표지가 풀렸다");
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
        assert!(rig.windows.calls().is_empty(), "준비에서 멈춘다");
        assert_eq!(rig.fingerprint(), before);
        assert_eq!(rig.service.status().crash_copy, CrashCopyStatus::Awaiting);
        assert!(rig.service.begin_answer().is_ok(), "표지가 풀렸다");
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
            rig.coordinator.answer(false).expect("거절"),
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

        let reply = rig.coordinator.answer(false).expect("거절");

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
