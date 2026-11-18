//! Launch preprocessing — after the spawn reservation · the 「already running」 check, and before the incarnation marker · session
//! derivation, checks launch materials with the **roster's** profile (called only from `spawn_agent_watching_link`). An id not in the
//! roster is also checked with the caller's copy before it is inserted (called only from `ensure_registered`). Mapping a material
//! preparation failure to a spawn failure (`prepare_failed`) lives here too — preparation is called after the spawn path's incarnation marker.
//!
//! The check ([`crate::launch::LaunchKitSource::check_in`]) is called holding none of the roster · reservation · session · name-assignment
//! locks — asset-library lock → manager lock is one direction, so calling while holding one deadlocks with library writes. It never writes
//! the roster, only compares (filling unattended defaults is a later topic — M16): if the launch inputs (attachments · command · env)
//! changed during the check, the answer is dropped, blocked or not, and redone once; if they changed again, the launch is cut off.
//!
//! Every check here runs inside the launch's scope, which the spawn body opens and passes in — this file never opens
//! one (contract = `LaunchKitSource::begin_launch`).
// ADR-0319
// ADR-0324
// ADR-0334
// TRD S25 §1-5 (ga)

use super::{profile_vanished_mid_spawn, AgentManager};
use crate::launch::{
    notice_fingerprint, LaunchBlocked, LaunchPlan, LaunchScope, Notice, PrepareError,
};
use crate::profile::{AgentProfile, LaunchInputs};
use crate::types::{AgentId, PtyError};

/// How many times check → compare runs — once, plus once more when the launch inputs changed.
// TRD S25 §1-5 (ga) 2
const PRELAUNCH_ATTEMPTS: u32 = 2;

impl AgentManager {
    /// Checks a profile not in the roster with the caller's copy **before** inserting it — when blocked nothing is inserted. A passing plan is
    /// dropped (after insertion [`Self::prelaunch`] checks again with the roster values).
    ///
    /// `scope` = this launch's scope — the one the prelaunch and the prepare of the same launch get.
    // ADR-0360
    // TRD S25 §1-5 (ga) 1 ②
    pub(super) fn check_before_create(
        &self,
        scope: &LaunchScope,
        profile: &AgentProfile,
    ) -> Result<(), PtyError> {
        match self.launch.kits.check_in(scope, profile) {
            Ok(_) => Ok(()),
            Err(blocked) => Err(launch_blocked(profile.id, blocked)),
        }
    }

    /// The plan to launch this agent — checks the profile read from the roster, and returns that answer only if the launch inputs are unchanged afterwards.
    ///
    /// `caller` = the copy the spawn caller passed — not checked. For an existing id the spawn merges the copy's command · env into the
    /// roster after this, so if those differ from the checked values the launch is cut off rather than run with an unchecked command.
    /// `Err` = blocked ([`PtyError::LaunchBlocked`]) · launch inputs changed both times, or the copy's command · env differ from the checked
    /// values ([`PtyError::LaunchRaced`]) · deleted meanwhile (`SpawnFailed`). In every case the roster · sessions · control channel are
    /// unchanged.
    /// With the plan comes its notice comparison against the fingerprint on the same roster read ([`PlanNotices`]) —
    ///   compared here, recorded by nobody here.
    /// `scope` = this launch's scope — both attempts check inside it.
    // ADR-0341
    // ADR-0360
    pub(super) fn prelaunch(
        &self,
        scope: &LaunchScope,
        caller: &AgentProfile,
    ) -> Result<(LaunchPlan, PlanNotices), PtyError> {
        let id = caller.id;
        for attempt in 1..=PRELAUNCH_ATTEMPTS {
            // The roster value, not the caller's copy — sees presets attached after the copy was taken. `get` takes a copy and releases the lock.
            let profile = self
                .profiles
                .get(id)
                .ok_or_else(|| profile_vanished_mid_spawn(id, "prelaunch read"))?;
            let seen = LaunchInputs::of(&profile);
            // ★Compare first while holding the answer — even a block is an answer only when the comparison matches★: a block from
            //   attachments that changed during the check is already stale (fixed attachments would be cut off by a stale block).
            let checked = self.launch.kits.check_in(scope, &profile);
            match self.profiles.launch_inputs_unchanged(id, &seen) {
                Some(true) => {
                    return settle(caller, &seen, checked).map(|plan| {
                        let notices = PlanNotices::against(&plan, &profile);
                        (plan, notices)
                    })
                }
                Some(false) => tracing::debug!(
                    agent = %id,
                    attempt,
                    "launch inputs (attachments, command, env) changed during the check; rereading the roster"
                ),
                None => return Err(profile_vanished_mid_spawn(id, "prelaunch comparison")),
            }
        }
        tracing::info!(
            agent = %id,
            "launch inputs kept changing during launch preparation; not launching (a retry succeeds)"
        );
        Err(PtyError::LaunchRaced)
    }
}

/// Returns the answer of a check whose comparison matched — if the copy's command · env differ from the checked values, it is a race whatever the answer.
fn settle(
    caller: &AgentProfile,
    seen: &LaunchInputs,
    checked: Result<LaunchPlan, LaunchBlocked>,
) -> Result<LaunchPlan, PtyError> {
    let id = caller.id;
    // ★Cut off rather than merge over★: no production path changes command · env today, so the copy and the roster do not diverge — if
    //   they did, both changed after the copy was taken, and a retry succeeds with a new copy.
    if !seen.same_command_and_env(caller) {
        tracing::info!(
            agent = %id,
            "caller copy's command or env differs from the checked roster values; not launching (a retry succeeds)"
        );
        return Err(PtyError::LaunchRaced);
    }
    let plan = checked.map_err(|blocked| launch_blocked(id, blocked))?;
    // Every launch logs its notices, whether or not a reply shows them (ADR-0341).
    for notice in &plan.notices {
        tracing::info!(
            agent = %id,
            code = %notice.code,
            args = %notice.args,
            line = ?notice.line,
            "launch notice"
        );
    }
    Ok(plan)
}

/// A plan's notices against what the agent was last shown — notices once per agent.
// ADR-0341
// TRD S25 §1-5 「Notices once per agent」
pub(super) struct PlanNotices {
    /// The plan's omission identity ([`notice_fingerprint`]) — what a delivering caller records.
    pub(super) fingerprint: String,
    /// For the success reply: the plan's full list, or empty when the stored fingerprint equals `fingerprint`.
    pub(super) shown: Vec<Notice>,
    /// The stored fingerprint `shown` was decided against — the record is a compare-and-set on it.
    pub(super) baseline: Option<String>,
}

impl PlanNotices {
    /// `roster` = the roster copy this plan was checked from — its fingerprint is the one compared.
    fn against(plan: &LaunchPlan, roster: &AgentProfile) -> Self {
        let fingerprint = notice_fingerprint(&plan.notices);
        let baseline = roster.notice_fingerprint.clone();
        let shown = if baseline.as_deref() == Some(fingerprint.as_str()) {
            Vec::new()
        } else {
            plan.notices.clone()
        };
        Self {
            fingerprint,
            shown,
            baseline,
        }
    }
}

fn launch_blocked(id: AgentId, blocked: LaunchBlocked) -> PtyError {
    tracing::info!(agent = %id, %blocked, "launch-material check blocked the launch; not launching");
    PtyError::LaunchBlocked(blocked)
}

/// Maps a material preparation ([`crate::launch::LaunchKitSource::prepare_in`]) failure to a spawn failure — a source changed after the
/// check is a race ([`PtyError::LaunchRaced`] — it never tried to open the process), anything else `SpawnFailed`.
// TRD S25 §5-2 4 (c)
pub(super) fn prepare_failed(id: AgentId, e: PrepareError) -> PtyError {
    match &e {
        PrepareError::Stale(_) => {
            tracing::info!(
                agent = %id,
                %e,
                "launch-material source changed after the check; not launching (a retry succeeds)"
            );
            PtyError::LaunchRaced
        }
        PrepareError::Failed(_) => PtyError::SpawnFailed(format!("{e} (agent {id})")),
    }
}

#[cfg(test)]
impl AgentManager {
    /// Hands the command the spawn assembled and the materials on it to the test — with a receiver present it stops before launching the process.
    pub(super) fn probe_spec(
        &self,
        spec: &crate::types::CommandSpec,
        artifacts: &crate::launch::LaunchArtifacts,
    ) -> Result<(), PtyError> {
        match engram_dashboard_base::sync::lock(&self.spec_probe).as_ref() {
            Some(tx) => {
                let _ = tx.send((spec.clone(), artifacts.clone()));
                Err(PtyError::SpawnFailed(
                    "test probe — assembled the command only, did not launch".to_string(),
                ))
            }
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet, VecDeque};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::{channel, Receiver};
    use std::sync::{Arc, Mutex, OnceLock, RwLock};
    use std::time::Duration;

    use engram_dashboard_base::sync;
    use engram_dashboard_platform::shell::console_command;
    use serde::Deserialize;
    use serde_json::json;
    use uuid::Uuid;

    use super::super::tests::{put_live_session, put_live_session_leased};
    use super::super::{AgentManager, NoticeDelivery, SpawnOutcome, SpawnReservation};
    use super::PlanNotices;
    use crate::backend;
    use crate::failure::AgentFailureKind;
    use crate::launch::{
        BlockReason, LaunchArtifacts, LaunchBlocked, LaunchFs, LaunchKit, LaunchKitSource,
        LaunchLease, LaunchPlan, LaunchPorts, LaunchScope, MemLaunchFs, NoopLaunchKits,
        PrepareError, Reason,
    };
    use crate::profile::scripted_store::ScriptedStore;
    use crate::profile::{
        AgentCommand, AgentOutputFormat, AgentProfile, AttachedPreset, ProfileRegistry,
        RestoreOutcome, SpawnMode,
    };
    use crate::profile_edit::Attachments;
    use crate::reaper::ReaperDeps;
    use crate::session::AgentSession;
    use crate::session_tracker::{SessionTracker, TrackerConfig};
    use crate::types::{
        AgentId, AgentInfo, AgentStatus, CommandSpec, ControlChannel, ControlChannelNeeds,
        ControlEndpoint, ProvisionError, PtyError, StatusSink,
    };

    // The same fixed values as the golden (`tests/launch_argv_golden.rs`) — if those change, the comparison below cannot find the case in the golden and turns red.
    const WORK: &str = "/golden/work";
    const FRESH_SID: Uuid = Uuid::from_u128(0x1111_1111_1111_4111_8111_1111_1111_1111);
    const STORED_SID: Uuid = Uuid::from_u128(0x2222_2222_2222_4222_8222_2222_2222_2222);

    // ── test rig ──

    struct QuietStatus;
    impl StatusSink for QuietStatus {
        fn status_changed(&self, _id: AgentId, _s: AgentStatus, _e: u32) {}
        fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
    }

    /// Counts issue requests — the answer is 「no control channel」 (same as `NoopControlChannel`).
    #[derive(Default)]
    struct CountingControl {
        provisions: AtomicUsize,
    }

    impl ControlChannel for CountingControl {
        fn provision(
            &self,
            _id: AgentId,
            _epoch: u32,
            _needs: ControlChannelNeeds,
        ) -> Result<Option<ControlEndpoint>, ProvisionError> {
            self.provisions.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        }
        fn revoke(&self, _id: AgentId, _epoch: u32) {}
    }

    /// Counts the placement port calls (writes · checks both) — writes only to memory.
    #[derive(Default)]
    struct CountingFs {
        calls: AtomicUsize,
        mem: MemLaunchFs,
    }

    impl LaunchFs for CountingFs {
        fn write_file(&self, path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.mem.write_file(path, bytes)
        }
        fn link_dir(
            &self,
            link: &std::path::Path,
            target: &std::path::Path,
        ) -> std::io::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.mem.link_dir(link, target)
        }
        fn exists(&self, path: &std::path::Path) -> bool {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.mem.exists(path)
        }
        fn is_link(&self, path: &std::path::Path) -> std::io::Result<bool> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.mem.is_link(path)
        }
    }

    /// Memory store · countable control channel · countable placement port · a probe taking the assembled command — launches no process
    /// at all.
    struct Rig {
        manager: AgentManager,
        store: Arc<ScriptedStore>,
        control: Arc<CountingControl>,
        fs: Arc<CountingFs>,
        specs: Receiver<(CommandSpec, LaunchArtifacts)>,
    }

    impl Rig {
        fn new(kits: Arc<dyn LaunchKitSource>) -> Self {
            let store = ScriptedStore::new();
            let profiles = Arc::new(ProfileRegistry::new(store.clone()));
            let tracker = Arc::new(SessionTracker::new(
                TrackerConfig {
                    enabled: false,
                    poll_interval: Duration::from_secs(1),
                },
                Arc::new(|_, _| {}),
            ));
            let control = Arc::new(CountingControl::default());
            let fs = Arc::new(CountingFs::default());
            let manager = AgentManager::new_with_control(
                Arc::new(QuietStatus),
                profiles,
                tracker,
                control.clone(),
            )
            .with_launch_ports(LaunchPorts {
                kits,
                fs: fs.clone(),
            });
            let (tx, specs) = channel();
            *sync::lock(&manager.spec_probe) = Some(tx);
            Self {
                manager,
                store,
                control,
                fs,
                specs,
            }
        }

        /// The number of placement port calls — writes · checks both.
        fn fs_calls(&self) -> usize {
            self.fs.calls.load(Ordering::SeqCst)
        }

        fn noop() -> Self {
            Self::new(Arc::new(NoopLaunchKits))
        }

        /// The command this spawn assembled — none means the spawn did not get as far as assembly.
        fn spec(&self) -> CommandSpec {
            self.assembled().0
        }

        /// The command this spawn assembled and the materials on it.
        fn assembled(&self) -> (CommandSpec, LaunchArtifacts) {
            self.specs
                .try_recv()
                .expect("the spawn did not assemble a command")
        }

        fn assembled_nothing(&self) -> bool {
            self.specs.try_recv().is_err()
        }

        fn provisions(&self) -> usize {
            self.control.provisions.load(Ordering::SeqCst)
        }
    }

    /// What one check saw.
    #[derive(Debug, Clone)]
    struct Seen {
        profile: AgentProfile,
        roster_free: bool,
        reservations_free: bool,
        sessions_free: bool,
        names_free: bool,
        reserved: bool,
        roster_epoch: Option<u32>,
        live_session: bool,
    }

    type Meanwhile = Box<dyn FnOnce(&ProfileRegistry, AgentId) + Send>;

    /// What to do on each check — an empty queue = a Noop plan.
    enum Turn {
        Pass,
        /// Passes with this plan.
        Plan(LaunchPlan),
        Block(LaunchBlocked),
        /// Changes the roster during the check (imitating another connection's attach · delete) — then a Noop plan.
        Meanwhile(Meanwhile),
        /// Changes the roster and blocks — a block issued with the value before the change.
        MeanwhileBlock(Meanwhile, LaunchBlocked),
    }

    /// What one preparation saw.
    #[derive(Debug, Clone)]
    struct Prepared {
        plan: LaunchPlan,
        id: AgentId,
        epoch: u32,
        /// Whether the roster · reservation · session · name-assignment locks were all free.
        locks_free: bool,
        reserved: bool,
        roster_epoch: Option<u32>,
        roster_sid: Option<Uuid>,
        /// The number of control-channel issue requests so far.
        provisions: usize,
        live_session: bool,
    }

    /// The answer for each preparation — an empty queue = Noop materials matching the received plan.
    enum Prep {
        /// Turns on the real-materials mark in the materials' plan — the checked plan is Noop, so a carried material that is on came from the materials.
        MarkReal,
        Fail(PrepareError),
        /// Noop materials with this volatile folder.
        Volatile(PathBuf),
    }

    /// What one reclaim saw — what was reclaimed is held by `released` in the same order.
    #[derive(Debug, Clone)]
    struct Release {
        /// Whether the roster · reservation · session · name-assignment locks were all free.
        locks_free: bool,
        /// Whether that agent's session was still in the session table.
        live_session: bool,
        /// Whether the reaper thread called it.
        on_reaper: bool,
        /// The roster's `auto_restore` then — `None` = the roster lock was held or there was no entry.
        auto_restore: Option<bool>,
    }

    struct Tables {
        profiles: Arc<ProfileRegistry>,
        spawning: Arc<Mutex<HashSet<AgentId>>>,
        sessions: Arc<RwLock<HashMap<AgentId, Arc<AgentSession>>>>,
        names: Arc<Mutex<()>>,
        control: Arc<CountingControl>,
    }

    /// A fake material source that checks · prepares by looking into the manager's roster · reservation · session tables, and counts reclaims.
    #[derive(Default)]
    struct ScriptedKits {
        turns: Mutex<VecDeque<Turn>>,
        seen: Mutex<Vec<Seen>>,
        preps: Mutex<VecDeque<Prep>>,
        prepared: Mutex<Vec<Prepared>>,
        released: Mutex<Vec<(AgentId, u32)>>,
        /// Filled only by a rig that hung the tables.
        releases: Mutex<Vec<Release>>,
        tables: OnceLock<Tables>,
    }

    impl ScriptedKits {
        fn new(turns: Vec<Turn>) -> Arc<Self> {
            Self::preparing(turns, Vec::new())
        }

        fn preparing(turns: Vec<Turn>, preps: Vec<Prep>) -> Arc<Self> {
            Arc::new(Self {
                turns: Mutex::new(turns.into()),
                preps: Mutex::new(preps.into()),
                ..Self::default()
            })
        }

        fn rig(self: &Arc<Self>) -> Rig {
            let rig = Rig::new(self.clone());
            let manager = &rig.manager;
            assert!(self
                .tables
                .set(Tables {
                    profiles: manager.profiles.clone(),
                    spawning: manager.spawning.clone(),
                    sessions: manager.sessions.clone(),
                    names: manager.name_allocation.clone(),
                    control: rig.control.clone(),
                })
                .is_ok());
            rig
        }

        fn seen(&self) -> Vec<Seen> {
            sync::lock(&self.seen).clone()
        }

        fn prepared(&self) -> Vec<Prepared> {
            sync::lock(&self.prepared).clone()
        }

        fn released(&self) -> Vec<(AgentId, u32)> {
            sync::lock(&self.released).clone()
        }

        fn releases(&self) -> Vec<Release> {
            sync::lock(&self.releases).clone()
        }
    }

    impl LaunchKitSource for ScriptedKits {
        fn check(&self, profile: &AgentProfile) -> Result<LaunchPlan, LaunchBlocked> {
            let t = self.tables.get().expect("the rig hung the tables");
            let id = profile.id;
            // First see whether they can be taken — if the manager called while holding one, the reads below would take it again on the same thread and hang.
            let roster_free = t.profiles.lock_is_free();
            let reservations_free = t.spawning.try_lock().is_ok();
            let sessions_free = t.sessions.try_write().is_ok();
            let names_free = t.names.try_lock().is_ok();
            sync::lock(&self.seen).push(Seen {
                profile: profile.clone(),
                roster_free,
                reservations_free,
                sessions_free,
                names_free,
                reserved: reservations_free && sync::lock(&t.spawning).contains(&id),
                roster_epoch: if roster_free {
                    t.profiles.get(id).map(|p| p.epoch)
                } else {
                    None
                },
                live_session: sessions_free && t.sessions.read().unwrap().contains_key(&id),
            });
            let turn = sync::lock(&self.turns).pop_front();
            match turn {
                None | Some(Turn::Pass) => NoopLaunchKits.check(profile),
                Some(Turn::Plan(plan)) => Ok(plan),
                Some(Turn::Block(blocked)) => Err(blocked),
                Some(Turn::Meanwhile(change)) => {
                    if roster_free {
                        change(&t.profiles, id);
                    }
                    NoopLaunchKits.check(profile)
                }
                Some(Turn::MeanwhileBlock(change, blocked)) => {
                    if roster_free {
                        change(&t.profiles, id);
                    }
                    Err(blocked)
                }
            }
        }

        fn prepare(
            &self,
            plan: LaunchPlan,
            id: AgentId,
            epoch: u32,
        ) -> Result<LaunchKit, PrepareError> {
            let t = self.tables.get().expect("the rig hung the tables");
            let roster_free = t.profiles.lock_is_free();
            let reservations_free = t.spawning.try_lock().is_ok();
            let sessions_free = t.sessions.try_write().is_ok();
            let names_free = t.names.try_lock().is_ok();
            let roster = if roster_free {
                t.profiles.get(id)
            } else {
                None
            };
            sync::lock(&self.prepared).push(Prepared {
                plan: plan.clone(),
                id,
                epoch,
                locks_free: roster_free && reservations_free && sessions_free && names_free,
                reserved: reservations_free && sync::lock(&t.spawning).contains(&id),
                roster_epoch: roster.as_ref().map(|p| p.epoch),
                roster_sid: roster.and_then(|p| p.backend_session_id),
                provisions: t.control.provisions.load(Ordering::SeqCst),
                live_session: sessions_free && t.sessions.read().unwrap().contains_key(&id),
            });
            let prep = sync::lock(&self.preps).pop_front();
            match prep {
                None => NoopLaunchKits.prepare(plan, id, epoch),
                Some(Prep::MarkReal) => {
                    let mut kit = NoopLaunchKits.prepare(plan, id, epoch)?;
                    kit.plan.real = true;
                    Ok(kit)
                }
                Some(Prep::Fail(e)) => Err(e),
                Some(Prep::Volatile(dir)) => {
                    let mut kit = NoopLaunchKits.prepare(plan, id, epoch)?;
                    kit.volatile_dir = Some(dir);
                    Ok(kit)
                }
            }
        }

        fn release(&self, id: AgentId, epoch: u32) {
            sync::lock(&self.released).push((id, epoch));
            let Some(t) = self.tables.get() else {
                return;
            };
            let sessions_free = t.sessions.try_write().is_ok();
            let roster_free = t.profiles.lock_is_free();
            sync::lock(&self.releases).push(Release {
                locks_free: roster_free
                    && t.spawning.try_lock().is_ok()
                    && sessions_free
                    && t.names.try_lock().is_ok(),
                live_session: !sessions_free || t.sessions.read().unwrap().contains_key(&id),
                on_reaper: std::thread::current().name() == Some("engram-reaper"),
                auto_restore: if roster_free {
                    t.profiles.get(id).map(|p| p.auto_restore)
                } else {
                    None
                },
            });
        }
    }

    // ── fixtures ──

    fn strings(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    /// The golden's five commands — the name is the head of the golden case name.
    fn golden_commands() -> Vec<(&'static str, AgentCommand)> {
        vec![
            (
                "claude-terminal",
                AgentCommand::Claude {
                    extra_args: strings(&["--model", "golden-model"]),
                    output_format: AgentOutputFormat::Terminal,
                },
            ),
            (
                "claude-json",
                AgentCommand::Claude {
                    extra_args: strings(&["--model", "golden-model"]),
                    output_format: AgentOutputFormat::StreamJson,
                },
            ),
            (
                "codex-terminal",
                AgentCommand::Codex {
                    extra_args: strings(&["-c", "model=golden-model"]),
                    output_format: AgentOutputFormat::Terminal,
                },
            ),
            (
                "codex-json",
                AgentCommand::Codex {
                    extra_args: strings(&["-c", "model=golden-model"]),
                    output_format: AgentOutputFormat::StreamJson,
                },
            ),
            (
                "shell",
                AgentCommand::Shell {
                    program: "golden-shell".to_string(),
                    args: strings(&["--login", "-i"]),
                },
            ),
        ]
    }

    /// The three commands of the on-the-spot creation paths — daemon `SpawnByCwd` (received backend · common default mode · no args) · smoke bin.
    fn spawn_path_commands() -> Vec<(&'static str, AgentCommand)> {
        vec![
            (
                "spawn-by-cwd/claude",
                AgentCommand::Claude {
                    extra_args: Vec::new(),
                    output_format: crate::commands::NEW_AGENT_OUTPUT_FORMAT,
                },
            ),
            (
                "spawn-by-cwd/codex",
                AgentCommand::Codex {
                    extra_args: Vec::new(),
                    output_format: crate::commands::NEW_AGENT_OUTPUT_FORMAT,
                },
            ),
            (
                "smoke-bin/claude-json",
                AgentCommand::Claude {
                    extra_args: strings(&["--model", "golden-model"]),
                    output_format: AgentOutputFormat::StreamJson,
                },
            ),
        ]
    }

    fn golden_env() -> Vec<(String, String)> {
        let path = std::env::join_paths(["/golden/sys", "/golden/tools"])
            .expect("fixed PATH")
            .into_string()
            .expect("UTF-8");
        vec![
            ("PATH".to_string(), path),
            ("GOLDEN_FLAG".to_string(), "1".to_string()),
        ]
    }

    fn profile_of(command: AgentCommand, env: Vec<(String, String)>) -> AgentProfile {
        AgentProfile::new(
            "golden".to_string(),
            command,
            PathBuf::from(WORK),
            env,
            false,
        )
    }

    fn claude_terminal() -> AgentCommand {
        golden_commands().remove(0).1
    }

    fn attachments(default_preset: bool, paths: &[&str]) -> Attachments {
        Attachments {
            default_preset,
            presets: paths
                .iter()
                .map(|p| AttachedPreset {
                    path: (*p).to_string(),
                })
                .collect(),
        }
    }

    fn blocked() -> LaunchBlocked {
        LaunchBlocked {
            reasons: vec![BlockReason::Missing {
                item: "team/a.md".to_string(),
                via: vec!["team/base.yaml".to_string()],
                reason: Reason {
                    code: "missing-file".to_string(),
                    args: json!({ "path": "team/a.md" }),
                    line: Some(1),
                },
            }],
        }
    }

    /// The three activation arms — resume resumes only with a stored handle, otherwise it falls back to a new conversation (`resume_no_fallback` head).
    #[derive(Debug, Clone, Copy)]
    enum Way {
        Fresh,
        Resume,
        ResumeWithoutHandle,
    }

    impl Way {
        const ALL: [Way; 3] = [Way::Fresh, Way::Resume, Way::ResumeWithoutHandle];

        fn mode(self) -> SpawnMode {
            match self {
                Way::Fresh => SpawnMode::Fresh,
                Way::Resume | Way::ResumeWithoutHandle => SpawnMode::Resume,
            }
        }

        /// The roster entry to launch with this arm — only resume gets a handle.
        fn profile(self) -> AgentProfile {
            let mut p = profile_of(claude_terminal(), golden_env());
            if matches!(self, Way::Resume) {
                p.backend_session_id = Some(STORED_SID);
            }
            p
        }
    }

    // ── golden comparison ──

    #[derive(Deserialize)]
    struct GoldenCase {
        case: String,
        console: bool,
        program: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        cwd: String,
    }

    /// (program, args, env, cwd) — laid out in the same shape as the golden (`matches` in `tests/launch_argv_golden.rs`).
    type Shape = (String, Vec<String>, Vec<(String, String)>, String);

    fn golden(case: &str) -> Shape {
        let all: Vec<GoldenCase> =
            serde_json::from_str(include_str!("../../tests/fixtures/launch_argv_golden.json"))
                .expect("the golden is JSON");
        let g = all.into_iter().find(|g| g.case == case).unwrap_or_else(|| {
            panic!("`{case}` is not in the golden — this item's premise is stale")
        });
        let (program, args) = if g.console {
            console_command(&g.program, g.args)
        } else {
            (g.program, g.args)
        };
        (program, args, g.env, g.cwd)
    }

    /// Replaces only the session id the spawn drew with the golden's fixed value — the rest stays byte for byte.
    fn shape(spec: &CommandSpec) -> Shape {
        let mut args = spec.args.clone();
        if let Some(i) = args.iter().position(|a| a == "--session-id") {
            if let Some(sid) = args.get_mut(i + 1) {
                if Uuid::parse_str(sid).is_ok_and(|s| s != STORED_SID) {
                    *sid = FRESH_SID.to_string();
                }
            }
        }
        let env = spec
            .env
            .iter()
            .map(|(k, v)| {
                if k.eq_ignore_ascii_case("PATH") {
                    let parts: Vec<String> = std::env::split_paths(v)
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect();
                    (k.clone(), parts.join(";"))
                } else {
                    (k.clone(), v.clone())
                }
            })
            .collect();
        (
            spec.program.clone(),
            args,
            env,
            spec.cwd.to_string_lossy().into_owned(),
        )
    }

    fn assert_golden(case: &str, path: &str, spec: &CommandSpec) {
        assert_eq!(
            shape(spec),
            golden(case),
            "the argv of the {path} path differs from the golden `{case}` — Noop launch materials must not change a spawn by a single byte"
        );
    }

    // ── Noop regression ──

    /// ★With Noop materials every manager spawn path produces argv byte-identical to today's★ — activation · wake-up (resume) · on-the-spot
    /// creation (daemon `SpawnByCwd` · smoke bin). Noop reads neither the attached presets nor the 「Default」 field, and `extra_args` ride as is.
    /// Placement yields the default and never calls the placement port.
    // TRD S25 §4 「agent manager」 · §5-2 4
    #[test]
    fn noop_spawn_paths_produce_todays_argv() {
        fn assert_noop(rig: &Rig, case: &str, path: &str) {
            let (spec, artifacts) = rig.assembled();
            assert_golden(case, path, &spec);
            assert_eq!(artifacts, LaunchArtifacts::default(), "{path} · {case}");
            assert_eq!(
                rig.fs_calls(),
                0,
                "{path} · {case}: called the placement port"
            );
        }

        for (name, command) in golden_commands() {
            let fresh = format!("{name}/fresh/no-endpoint");

            let rig = Rig::noop();
            let mut p = profile_of(command.clone(), golden_env());
            p.presets = attachments(true, &["/team/a.md"]).presets;
            rig.manager.profiles.upsert(p.clone());
            let _ = rig.manager.activate_profile(&p, SpawnMode::Fresh);
            assert_noop(&rig, &fresh, "activation");

            let rig = Rig::noop();
            let mut p = profile_of(command.clone(), golden_env());
            p.presets = attachments(true, &["/team/a.md"]).presets;
            if backend::can_resume_stored_session(&command) {
                p.backend_session_id = Some(STORED_SID);
            }
            rig.manager.profiles.upsert(p.clone());
            let _ = rig.manager.activate_profile(&p, SpawnMode::Resume);
            assert_noop(&rig, &format!("{name}/resume/no-endpoint"), "wake-up");

            let rig = Rig::noop();
            let p = profile_of(command, golden_env());
            let _ = rig.manager.spawn_agent(&p, SpawnMode::Fresh);
            assert_noop(&rig, &fresh, "on-the-spot creation");
        }

        for (name, command) in spawn_path_commands() {
            let rig = Rig::noop();
            let p = profile_of(command, Vec::new());
            assert!(
                p.default_preset,
                "the constructor attaches 「Default」 — Noop does not read it"
            );
            let _ = rig.manager.spawn_agent(&p, SpawnMode::Fresh);
            assert_noop(&rig, &format!("{name}/fresh/no-endpoint"), name);
        }
    }

    /// ★The working folder is read from the roster, not the caller's copy★ — if the working folder changed after the copy was taken
    /// (`agent.setWorkspace`), the copy's cwd is stale.
    // TRD S25 §1-5 (ga) 3
    #[test]
    fn the_spawned_cwd_is_the_rosters_not_the_callers_copy() {
        let rig = Rig::noop();
        let mut stored = profile_of(claude_terminal(), golden_env());
        stored.cwd = PathBuf::from("/golden/moved");
        rig.manager.profiles.upsert(stored.clone());
        let mut stale = stored.clone();
        stale.cwd = PathBuf::from(WORK);

        let _ = rig.manager.activate_profile(&stale, SpawnMode::Fresh);

        assert_eq!(rig.spec().cwd, PathBuf::from("/golden/moved"));
        assert_eq!(
            rig.manager.profiles.get(stored.id).unwrap().cwd,
            PathBuf::from("/golden/moved"),
            "the copy's cwd does not revert the roster"
        );
    }

    // ── order ──

    /// ★The check runs after the reservation · before the incarnation marker and session derivation, with roster values, without manager locks★.
    // ADR-0319
    #[test]
    fn the_check_runs_on_the_roster_after_the_reservation_and_before_the_incarnation() {
        let kits = ScriptedKits::new(Vec::new());
        let rig = kits.rig();
        let mut caller = profile_of(claude_terminal(), golden_env());
        caller.backend_session_id = Some(STORED_SID);
        rig.manager.profiles.upsert(caller.clone());
        let attached = attachments(false, &["/team/a.md"]);
        assert!(rig
            .manager
            .profiles
            .try_set_attachments(caller.id, attached.clone())
            .unwrap());
        let epoch_before = rig.manager.profiles.get(caller.id).unwrap().epoch;

        let _ = rig.manager.activate_profile(&caller, SpawnMode::Fresh);

        let seen = kits.seen();
        assert_eq!(seen.len(), 1, "one check");
        let s = &seen[0];
        assert!(
            s.roster_free && s.reservations_free && s.sessions_free && s.names_free,
            "the check was called while holding a manager lock: {s:?}"
        );
        assert!(s.reserved, "not after the reservation");
        assert!(!s.live_session);
        assert_eq!(
            (s.profile.default_preset, s.profile.presets.clone()),
            (attached.default_preset, attached.presets),
            "checks the roster value, not the caller's copy"
        );
        assert_eq!(
            s.roster_epoch,
            Some(epoch_before),
            "not before the incarnation marker"
        );
        assert_eq!(
            s.profile.backend_session_id,
            Some(STORED_SID),
            "not before session derivation (the Fresh clear)"
        );

        // The spawn continued after the check — drew the marker, cleared the session id and assembled the command.
        let _ = rig.spec();
        let after = rig.manager.profiles.get(caller.id).unwrap();
        assert_ne!(after.epoch, epoch_before);
        assert_eq!(after.backend_session_id, None);
    }

    /// ★An id not in the roster is checked once with the caller's copy before insertion and once with the roster value after★ — the
    /// first inserts nothing when blocked, and the plan to launch is the second (it sees the assigned name · constructor defaults).
    // TRD S25 §1-5 (ga) 0 · 1 ②
    #[test]
    fn an_unregistered_profile_is_checked_before_registration_and_again_on_the_roster() {
        let kits = ScriptedKits::new(Vec::new());
        let rig = kits.rig();
        let adhoc = profile_of(spawn_path_commands().remove(0).1, Vec::new());

        let _ = rig.manager.spawn_agent(&adhoc, SpawnMode::Fresh);

        let seen = kits.seen();
        assert_eq!(seen.len(), 2, "once before and once after insertion");
        let (before, after) = (&seen[0], &seen[1]);
        assert!(
            before.roster_free
                && before.reservations_free
                && before.sessions_free
                && before.names_free,
            "the pre-insert check was called while holding a manager lock: {before:?}"
        );
        assert!(
            before.reserved,
            "the pre-insert check is not after the reservation"
        );
        assert_eq!(
            before.roster_epoch, None,
            "the pre-insert check ran after insertion"
        );
        assert_eq!(
            before.profile.display_name, None,
            "the pre-insert check sees the caller's copy"
        );
        assert_eq!(
            after.roster_epoch,
            Some(adhoc.epoch),
            "in the roster at check time, and the marker not yet drawn"
        );
        assert!(
            after.profile.display_name.is_some(),
            "the name is assigned before the second check"
        );
        assert!(after.profile.default_preset && after.profile.presets.is_empty());
        let _ = rig.spec();
    }

    /// ★Running or starting = no check★ — preprocessing comes after the reservation · 「already running」 check.
    #[test]
    fn a_running_or_starting_agent_is_not_checked() {
        let kits = ScriptedKits::new(Vec::new());
        let rig = kits.rig();
        let p = profile_of(claude_terminal(), golden_env());
        rig.manager.profiles.upsert(p.clone());

        {
            let _held =
                SpawnReservation::reserve(rig.manager.spawning.clone(), p.id).expect("reserve");
            assert!(matches!(
                rig.manager.spawn_agent(&p, SpawnMode::Fresh),
                Ok(SpawnOutcome::Moot(None))
            ));
        }
        put_live_session(&rig.manager, p.id, 1);
        assert!(matches!(
            rig.manager.spawn_agent(&p, SpawnMode::Fresh),
            Ok(SpawnOutcome::Moot(Some(_)))
        ));

        assert!(kits.seen().is_empty(), "the check ran: {:?}", kits.seen());
        assert!(rig.assembled_nothing());
    }

    // ── blocks · races ──

    /// ★Blocked = not launched★ — returns the structured refusal as is without touching the control channel · incarnation marker · session
    /// id, and the 「last failure」 is 「other」 (the resume path · a handle-less resume falling back to a new conversation give the same answer).
    #[test]
    fn a_blocked_activation_starts_nothing_and_records_other() {
        for way in Way::ALL {
            let kits = ScriptedKits::new(vec![Turn::Block(blocked())]);
            let rig = kits.rig();
            let p = way.profile();
            rig.manager.profiles.upsert(p.clone());
            let epoch_before = rig.manager.profiles.get(p.id).unwrap().epoch;

            let err = rig
                .manager
                .activate_profile(&p, way.mode())
                .expect_err("a block is an error");

            assert!(
                matches!(&err, PtyError::LaunchBlocked(b) if *b == blocked()),
                "{way:?}: {err}"
            );
            assert!(rig.assembled_nothing(), "{way:?}: assembled a command");
            assert_eq!(rig.provisions(), 0, "{way:?}: issued a control channel");
            assert!(rig.manager.list_agents().is_empty(), "{way:?}");
            let after = rig.manager.profiles.get(p.id).unwrap();
            assert_eq!(
                after.epoch, epoch_before,
                "{way:?}: drew the incarnation marker"
            );
            assert_eq!(after.backend_session_id, p.backend_session_id, "{way:?}");
            assert_eq!(after.last_failure, Some(AgentFailureKind::Other), "{way:?}");
        }
    }

    /// ★A blocked boot restore is not launched and is reported as 「failed」★ — both arms, resume · new conversation (`restore_one`).
    // ADR-0334
    #[test]
    fn a_blocked_boot_restore_reports_failed_and_starts_nothing() {
        for way in [Way::Resume, Way::Fresh] {
            let kits = ScriptedKits::new(vec![Turn::Block(blocked())]);
            let rig = kits.rig();
            let p = way.profile();
            rig.manager.profiles.upsert(p.clone());
            let epoch_before = rig.manager.profiles.get(p.id).unwrap().epoch;

            let outcome = rig.manager.restore_one(&p);

            assert!(
                matches!(&outcome, RestoreOutcome::Failed { reason }
                    if reason.contains(&blocked().to_string())),
                "{way:?}: {outcome:?}"
            );
            assert!(rig.assembled_nothing(), "{way:?}: assembled a command");
            assert_eq!(rig.provisions(), 0, "{way:?}: issued a control channel");
            let after = rig.manager.profiles.get(p.id).unwrap();
            assert_eq!(
                after.epoch, epoch_before,
                "{way:?}: drew the incarnation marker"
            );
            assert_eq!(after.backend_session_id, p.backend_session_id, "{way:?}");
            assert_eq!(after.last_failure, Some(AgentFailureKind::Other), "{way:?}");
        }
    }

    /// ★An ad-hoc create blocked by the pre-add check adds nothing★ — a block found before the add leaves no roster row
    /// that cannot start. A create that fails after the add leaves the agent asleep, and the wire `SpawnByCwd` answer
    /// names it by `created_agent_id` so the caller wakes it instead of creating another (ADR-0348).
    // TRD S25 §1-5 (ga) 1 ②
    #[test]
    fn a_blocked_ad_hoc_spawn_registers_nothing_and_starts_nothing() {
        let kits = ScriptedKits::new(vec![Turn::Block(blocked())]);
        let rig = kits.rig();
        let adhoc = profile_of(spawn_path_commands().remove(0).1, Vec::new());

        let err = rig
            .manager
            .spawn_agent(&adhoc, SpawnMode::Fresh)
            .expect_err("a block is an error");

        assert!(
            matches!(&err, PtyError::LaunchBlocked(b) if *b == blocked()),
            "{err}"
        );
        assert_eq!(kits.seen().len(), 1, "cut off at the pre-insert check");
        assert!(rig.manager.profiles.get(adhoc.id).is_none(), "inserted");
        assert_eq!(rig.store.attempts(), 0, "saved");
        assert!(rig.assembled_nothing());
        assert_eq!(rig.provisions(), 0);
    }

    /// Blocked at the post-insert roster-value check = the inserted entry stays (partial success — registration is not rolled back).
    #[test]
    fn an_ad_hoc_spawn_blocked_after_registration_keeps_it_and_starts_nothing() {
        let kits = ScriptedKits::new(vec![Turn::Pass, Turn::Block(blocked())]);
        let rig = kits.rig();
        let adhoc = profile_of(spawn_path_commands().remove(0).1, Vec::new());

        let err = rig
            .manager
            .spawn_agent(&adhoc, SpawnMode::Fresh)
            .expect_err("a block is an error");

        assert!(matches!(err, PtyError::LaunchBlocked(_)), "{err}");
        assert_eq!(kits.seen().len(), 2);
        assert!(rig.manager.profiles.get(adhoc.id).is_some());
        assert_eq!(rig.store.attempts(), 1, "the insert's one save");
        assert!(rig.assembled_nothing());
        assert_eq!(rig.provisions(), 0);
    }

    /// ★If the attachments change during the check, the roster is reread and checked once more★ — the second sees the new value.
    // TRD S25 §1-5 (ga) 2
    #[test]
    fn a_preset_change_during_the_check_is_checked_again_once() {
        let next = attachments(true, &["/team/b.md"]);
        let change = next.clone();
        let kits = ScriptedKits::new(vec![Turn::Meanwhile(Box::new(move |profiles, id| {
            assert!(profiles.try_set_attachments(id, change).unwrap());
        }))]);
        let rig = kits.rig();
        let p = profile_of(claude_terminal(), golden_env());
        rig.manager.profiles.upsert(p.clone());

        let _ = rig.manager.activate_profile(&p, SpawnMode::Fresh);

        let seen = kits.seen();
        assert_eq!(seen.len(), 2, "checks again only once");
        assert!(seen[0].profile.presets.is_empty());
        assert_eq!(
            seen[1].profile.presets, next.presets,
            "the reread roster value"
        );
        let _ = rig.spec();
    }

    /// Changes the attachments to `paths` during the check.
    fn set_presets(paths: &'static [&'static str]) -> Meanwhile {
        Box::new(move |profiles: &ProfileRegistry, id| {
            assert!(profiles
                .try_set_attachments(id, attachments(true, paths))
                .unwrap());
        })
    }

    /// ★Changed both times = not launched, cut off as a race★ — the 「last failure」 is 「other」 (the resume path · a handle-less resume
    /// falling back to a new conversation give the same answer).
    #[test]
    fn presets_that_keep_changing_fail_the_launch_as_raced() {
        for way in Way::ALL {
            let kits = ScriptedKits::new(vec![
                Turn::Meanwhile(set_presets(&["/team/b.md"])),
                Turn::Meanwhile(set_presets(&["/team/c.md"])),
            ]);
            let rig = kits.rig();
            let p = way.profile();
            rig.manager.profiles.upsert(p.clone());

            let err = rig
                .manager
                .activate_profile(&p, way.mode())
                .expect_err("a race is an error");

            assert!(matches!(err, PtyError::LaunchRaced), "{way:?}: {err}");
            assert_eq!(kits.seen().len(), 2, "{way:?}");
            assert!(rig.assembled_nothing(), "{way:?}");
            assert_eq!(rig.provisions(), 0, "{way:?}");
            let after = rig.manager.profiles.get(p.id).unwrap();
            assert_eq!(after.backend_session_id, p.backend_session_id, "{way:?}");
            assert_eq!(after.last_failure, Some(AgentFailureKind::Other), "{way:?}");
        }
    }

    /// ★If the launch inputs changed during the check, even a block is dropped and the check redone★ — that block was issued with the
    /// value before the change and is stale (fixed attachments would be cut off by a stale block).
    // TRD S25 §1-5 (ga) 1 · 2
    #[test]
    fn a_block_from_presets_that_changed_during_the_check_is_checked_again() {
        let kits = ScriptedKits::new(vec![Turn::MeanwhileBlock(
            set_presets(&["/team/b.md"]),
            blocked(),
        )]);
        let rig = kits.rig();
        let p = profile_of(claude_terminal(), golden_env());
        rig.manager.profiles.upsert(p.clone());

        let _ = rig.manager.activate_profile(&p, SpawnMode::Fresh);

        let seen = kits.seen();
        assert_eq!(seen.len(), 2, "drops the stale block and checks once more");
        assert_eq!(
            seen[1].profile.presets,
            attachments(true, &["/team/b.md"]).presets,
            "the reread roster value"
        );
        let _ = rig.spec();
    }

    /// ★Blocked both times with changed inputs = a race, not a block★ — neither block answers for the current roster value.
    #[test]
    fn blocks_from_presets_that_keep_changing_fail_the_launch_as_raced() {
        let kits = ScriptedKits::new(vec![
            Turn::MeanwhileBlock(set_presets(&["/team/b.md"]), blocked()),
            Turn::MeanwhileBlock(set_presets(&["/team/c.md"]), blocked()),
        ]);
        let rig = kits.rig();
        let p = profile_of(claude_terminal(), golden_env());
        rig.manager.profiles.upsert(p.clone());

        let err = rig
            .manager
            .activate_profile(&p, SpawnMode::Fresh)
            .expect_err("a race is an error");

        assert!(matches!(err, PtyError::LaunchRaced), "{err}");
        assert_eq!(kits.seen().len(), 2);
        assert!(rig.assembled_nothing());
        assert_eq!(rig.provisions(), 0);
        assert_eq!(
            rig.manager.profiles.get(p.id).unwrap().last_failure,
            Some(AgentFailureKind::Other)
        );
    }

    /// ★If the passed copy's command · env differ from the checked roster values, it is not launched★ — the spawn moves those two from
    /// the copy to the roster, so going ahead would launch an unchecked command. The roster is not overwritten with that copy either.
    // TRD S25 §1-5 (ga) 2
    #[test]
    fn a_callers_copy_whose_command_or_env_differs_from_the_roster_is_raced() {
        let edits: [(&str, fn(&mut AgentProfile)); 2] = [
            ("env", |c| {
                c.env.push(("STALE".to_string(), "1".to_string()))
            }),
            ("command", |c| c.command = golden_commands().remove(4).1),
        ];
        for (what, edit) in edits {
            let kits = ScriptedKits::new(Vec::new());
            let rig = kits.rig();
            let stored = profile_of(claude_terminal(), golden_env());
            rig.manager.profiles.upsert(stored.clone());
            let mut caller = stored.clone();
            edit(&mut caller);

            let err = rig
                .manager
                .activate_profile(&caller, SpawnMode::Fresh)
                .expect_err("a race is an error");

            assert!(matches!(err, PtyError::LaunchRaced), "{what}: {err}");
            assert_eq!(
                kits.seen().len(),
                1,
                "{what}: the roster is unchanged, so no recheck"
            );
            assert!(rig.assembled_nothing(), "{what}");
            assert_eq!(rig.provisions(), 0, "{what}");
            let after = rig.manager.profiles.get(stored.id).unwrap();
            assert_eq!(
                (after.command, after.env),
                (stored.command.clone(), stored.env.clone()),
                "{what}: the copy overwrote the roster"
            );
            assert_eq!(after.last_failure, Some(AgentFailureKind::Other), "{what}");
        }
    }

    /// Deleted during the check = cut off — no launch with a deleted profile.
    #[test]
    fn a_profile_deleted_during_the_check_is_not_started() {
        let kits = ScriptedKits::new(vec![Turn::Meanwhile(Box::new(|profiles, id| {
            profiles.try_remove(id).unwrap();
        }))]);
        let rig = kits.rig();
        let p = profile_of(claude_terminal(), golden_env());
        rig.manager.profiles.upsert(p.clone());

        let err = rig
            .manager
            .activate_profile(&p, SpawnMode::Fresh)
            .expect_err("deleted");

        assert!(
            matches!(&err, PtyError::SpawnFailed(m) if m.contains("prelaunch comparison")),
            "{err}"
        );
        assert!(rig.manager.profiles.get(p.id).is_none(), "not revived");
        assert!(rig.assembled_nothing());
    }

    // ── Notices once per agent (ADR-0341) ──

    fn notice(code: &str) -> Reason {
        Reason {
            code: code.to_string(),
            args: json!({ "names": [code] }),
            line: Some(1),
        }
    }

    /// The plan `profile` checks to, carrying `notices`.
    fn plan_with(profile: &AgentProfile, notices: Vec<Reason>) -> LaunchPlan {
        LaunchPlan {
            notices,
            ..NoopLaunchKits.check(profile).unwrap()
        }
    }

    #[test]
    fn plan_notices_are_empty_only_when_the_stored_fingerprint_matches() {
        let mut roster = Way::Fresh.profile();
        let x = plan_with(&roster, vec![notice("a"), notice("b")]);
        let fingerprint = crate::launch::notice_fingerprint(&x.notices);

        let never = PlanNotices::against(&x, &roster);
        assert_eq!(never.fingerprint, fingerprint);
        assert_eq!(never.shown, x.notices, "nothing recorded → the full list");
        assert_eq!(
            never.baseline, None,
            "the record's baseline = the value compared"
        );

        roster.notice_fingerprint = Some(fingerprint.clone());
        let same = PlanNotices::against(&x, &roster);
        assert!(same.shown.is_empty(), "same set");
        assert_eq!(same.baseline, Some(fingerprint.clone()));

        roster.notice_fingerprint = Some(crate::launch::notice_fingerprint(&[notice("a")]));
        assert_eq!(
            PlanNotices::against(&x, &roster).shown,
            x.notices,
            "a changed set → the full list, not the difference"
        );

        roster.notice_fingerprint = Some(String::new());
        let none = PlanNotices::against(&plan_with(&roster, Vec::new()), &roster);
        assert_eq!(none.fingerprint, "");
        assert!(none.shown.is_empty());
    }

    /// The compare reads the roster, not the caller's copy — a copy taken before the last record would re-show the set
    /// (and a copy carrying a record the roster lacks would hide it).
    #[test]
    fn prelaunch_compares_with_the_roster_fingerprint_not_the_callers_copy() {
        let p = Way::Fresh.profile();
        let x = vec![notice("a")];
        let shown_before = crate::launch::notice_fingerprint(&x);

        let kits = ScriptedKits::new(vec![
            Turn::Plan(plan_with(&p, x.clone())),
            Turn::Plan(plan_with(&p, x.clone())),
        ]);
        let rig = kits.rig();
        let mut roster = p.clone();
        roster.notice_fingerprint = Some(shown_before.clone());
        rig.manager.profiles.upsert(roster);

        let (_, notices) = rig
            .manager
            .prelaunch(&LaunchScope::default(), &p)
            .expect("passes");
        assert!(
            notices.shown.is_empty(),
            "the roster was already shown this set"
        );
        assert_eq!(notices.fingerprint, shown_before);

        rig.manager
            .profiles
            .update_with(p.id, |q| q.notice_fingerprint = None);
        let mut stale_copy = p.clone();
        stale_copy.notice_fingerprint = Some(shown_before);
        let (_, notices) = rig
            .manager
            .prelaunch(&LaunchScope::default(), &stale_copy)
            .expect("passes");
        assert_eq!(notices.shown, x, "nothing recorded on the roster");
    }

    /// ★No failure marks a set as seen★ — a block, a race, a prepare failure and a spawn error after the assembly all
    /// answer with an error, so a delivering caller records nothing and the next success shows the set.
    // TRD S25 §1-5 「Notices once per agent」
    #[test]
    fn a_failed_delivered_activation_records_no_fingerprint() {
        type Expect = fn(&PtyError) -> bool;
        let cases: [(&str, Option<PrepareError>, Expect); 4] = [
            ("block", None, |e| matches!(e, PtyError::LaunchBlocked(_))),
            (
                "stale prepare",
                Some(PrepareError::Stale("gone".to_string())),
                |e| matches!(e, PtyError::LaunchRaced),
            ),
            (
                "failed prepare",
                Some(PrepareError::Failed("disk".to_string())),
                |e| matches!(e, PtyError::SpawnFailed(m) if !m.contains("test probe")),
            ),
            (
                "spawn error after the assembly",
                None,
                |e| matches!(e, PtyError::SpawnFailed(m) if m.contains("test probe")),
            ),
        ];
        for (what, prep, expected) in cases {
            for way in Way::ALL {
                let p = way.profile();
                let turn = if what == "block" {
                    Turn::Block(blocked())
                } else {
                    Turn::Plan(plan_with(&p, vec![notice("a")]))
                };
                let preps = prep.clone().map(Prep::Fail).into_iter().collect();
                let kits = ScriptedKits::preparing(vec![turn], preps);
                let rig = kits.rig();
                rig.manager.profiles.upsert(p.clone());

                let err = rig
                    .manager
                    .activate_profile_reporting(&p, way.mode(), NoticeDelivery::Delivered)
                    .expect_err(what);
                assert!(expected(&err), "{what} ({way:?}): {err}");
                assert_eq!(
                    rig.manager.profiles.get(p.id).unwrap().notice_fingerprint,
                    None,
                    "{what} ({way:?}) recorded the set as seen"
                );
            }
        }
    }

    // ── material preparation ──

    /// A Noop plan with a mark identifying it as the check's plan.
    fn marked_plan() -> LaunchPlan {
        LaunchPlan {
            notices: vec![Reason {
                code: "checked-plan".to_string(),
                args: json!({}),
                line: None,
            }],
            ..NoopLaunchKits
                .check(&profile_of(claude_terminal(), golden_env()))
                .unwrap()
        }
    }

    /// ★Materials are prepared from the checked plan, after the incarnation marker · before the session-id clear and control-channel issue, without manager locks★.
    // TRD S25 §1-5 (na) 4 · §5-2 4 (a)
    #[test]
    fn the_kit_is_prepared_from_the_checked_plan_after_the_incarnation_without_manager_locks() {
        let kits = ScriptedKits::preparing(vec![Turn::Plan(marked_plan())], Vec::new());
        let rig = kits.rig();
        let p = Way::Resume.profile();
        rig.manager.profiles.upsert(p.clone());
        let epoch_before = rig.manager.profiles.get(p.id).unwrap().epoch;

        let _ = rig.manager.activate_profile(&p, SpawnMode::Fresh);

        let prepared = kits.prepared();
        assert_eq!(prepared.len(), 1, "one preparation");
        let s = &prepared[0];
        assert_eq!(s.plan, marked_plan(), "not the checked plan as is");
        assert!(s.locks_free, "prepared while holding a manager lock: {s:?}");
        assert!(s.reserved, "prepared outside the reservation");
        assert!(!s.live_session);
        assert_eq!(s.id, p.id);
        assert_ne!(s.epoch, epoch_before, "before the incarnation marker");
        assert_eq!(
            s.roster_epoch,
            Some(s.epoch),
            "not prepared with this incarnation's marker"
        );
        assert_eq!(s.roster_sid, Some(STORED_SID), "after the session-id clear");
        assert_eq!(s.provisions, 0, "after the control-channel issue");
        let _ = rig.spec();
    }

    /// ★The assembled materials are the prepared kit, placed once★ — when the checked plan is Noop but the kit is marked
    /// real, the assembled materials are real too. An empty real kit carries only that mark and does not call the layout
    /// port (the claude translation has nothing to place from it). Noop materials stay at the default (argv golden =
    /// `noop_spawn_paths_produce_todays_argv`).
    // TRD S25 §1-5 (na) 6 · 7
    #[test]
    fn the_assembled_artifacts_come_from_the_prepared_kit() {
        let p = profile_of(claude_terminal(), golden_env());

        let kits = ScriptedKits::preparing(Vec::new(), vec![Prep::MarkReal]);
        let rig = kits.rig();
        rig.manager.profiles.upsert(p.clone());
        let _ = rig.manager.activate_profile(&p, SpawnMode::Fresh);
        assert_eq!(
            rig.assembled().1,
            LaunchArtifacts {
                real: true,
                ..LaunchArtifacts::default()
            }
        );
        assert!(rig.assembled_nothing(), "assembles only once");
        assert_eq!(rig.fs_calls(), 0);

        let rig = Rig::noop();
        rig.manager.profiles.upsert(p.clone());
        let _ = rig.manager.activate_profile(&p, SpawnMode::Fresh);
        assert_eq!(rig.assembled().1, LaunchArtifacts::default());
        assert_eq!(rig.fs_calls(), 0);
    }

    /// ★A failed placement is not launched, and the guard reclaims that incarnation's materials exactly once★ — here the preparation yields
    /// a relative volatile folder and placement cuts it off (the confining root would resolve against the daemon's current folder). Before
    /// assembly · launch, and there is no session.
    // TRD S25 §1-5 (na) 6 · §5-2 4 (c2) ①
    #[test]
    fn a_failed_materialize_releases_the_kit_once_and_starts_nothing() {
        let kits = ScriptedKits::preparing(
            Vec::new(),
            vec![Prep::Volatile(PathBuf::from("relative").join("launch"))],
        );
        let rig = kits.rig();
        let p = profile_of(claude_terminal(), golden_env());
        rig.manager.profiles.upsert(p.clone());

        let err = rig
            .manager
            .activate_profile(&p, SpawnMode::Fresh)
            .expect_err("a placement failure is an error");

        assert!(
            matches!(&err, PtyError::SpawnFailed(m) if m.contains("launch-material placement failed")),
            "{err}"
        );
        let prepared = kits.prepared();
        assert_eq!(prepared.len(), 1);
        assert_eq!(
            kits.released(),
            vec![(p.id, prepared[0].epoch)],
            "not reclaimed exactly once"
        );
        let r = &kits.releases()[0];
        assert!(r.locks_free && !r.on_reaper && !r.live_session, "{r:?}");
        assert!(rig.assembled_nothing(), "not cut off before placement");
        assert_eq!(rig.fs_calls(), 0);
        assert!(rig.manager.list_agents().is_empty());
        assert_eq!(
            rig.manager.profiles.get(p.id).unwrap().last_failure,
            Some(AgentFailureKind::SpawnFailed)
        );
    }

    /// ★A failed preparation reclaims that incarnation's materials exactly once and is not launched★ — a source changed after the check is a
    /// race (「last failure」 = other), anything else `SpawnFailed`. Either way the stored session id is not pushed and no control channel is issued.
    // TRD S25 §5-2 4 (c)
    #[test]
    fn a_failed_prepare_releases_the_kit_once_and_starts_nothing() {
        for way in Way::ALL {
            for stale in [true, false] {
                let err = if stale {
                    PrepareError::Stale("team/a.md gone".to_string())
                } else {
                    PrepareError::Failed("could not make the volatile folder".to_string())
                };
                let kits = ScriptedKits::preparing(Vec::new(), vec![Prep::Fail(err)]);
                let rig = kits.rig();
                let p = way.profile();
                rig.manager.profiles.upsert(p.clone());

                let got = rig
                    .manager
                    .activate_profile(&p, way.mode())
                    .expect_err("a preparation failure is an error");

                let case = format!("{way:?} stale={stale}");
                if stale {
                    assert!(matches!(got, PtyError::LaunchRaced), "{case}: {got}");
                } else {
                    assert!(
                        matches!(&got, PtyError::SpawnFailed(m) if m.contains("could not make the volatile folder")),
                        "{case}: {got}"
                    );
                }
                let prepared = kits.prepared();
                assert_eq!(prepared.len(), 1, "{case}");
                assert_eq!(
                    kits.released(),
                    vec![(p.id, prepared[0].epoch)],
                    "{case}: not reclaimed exactly once"
                );
                assert!(rig.assembled_nothing(), "{case}");
                assert_eq!(rig.provisions(), 0, "{case}");
                assert!(rig.manager.list_agents().is_empty(), "{case}");
                let after = rig.manager.profiles.get(p.id).unwrap();
                assert_eq!(after.backend_session_id, p.backend_session_id, "{case}");
                let kind = if stale {
                    AgentFailureKind::Other
                } else {
                    AgentFailureKind::SpawnFailed
                };
                assert_eq!(after.last_failure, Some(kind), "{case}");
            }
        }
    }

    /// ★Cut off after preparation but before the session takes over = materials still reclaimed exactly once★ — here the test probe cuts
    /// it off after assembly, before launch (the same span as a launch · session-registration failure).
    #[test]
    fn a_failure_between_prepare_and_the_session_releases_the_kit_once() {
        let kits = ScriptedKits::preparing(Vec::new(), Vec::new());
        let rig = kits.rig();
        let p = profile_of(claude_terminal(), golden_env());
        rig.manager.profiles.upsert(p.clone());

        let err = rig
            .manager
            .activate_profile(&p, SpawnMode::Fresh)
            .expect_err("the probe cuts off before launch");

        assert!(
            matches!(&err, PtyError::SpawnFailed(m) if m.contains("test probe")),
            "{err}"
        );
        let _ = rig.spec();
        let prepared = kits.prepared();
        assert_eq!(prepared.len(), 1);
        assert_eq!(kits.released(), vec![(p.id, prepared[0].epoch)]);
    }

    /// A rig launching one real shell — the probe is removed so it goes as far as launching.
    fn started_shell(kits: &Arc<ScriptedKits>) -> (Rig, AgentInfo) {
        let rig = kits.rig();
        *sync::lock(&rig.manager.spec_probe) = None;
        let mut p = profile_of(
            AgentCommand::Shell {
                program: engram_dashboard_platform::shell::default_shell().to_string(),
                args: Vec::new(),
            },
            Vec::new(),
        );
        p.cwd = std::env::temp_dir();
        rig.manager.profiles.upsert(p.clone());
        let info = rig
            .manager
            .activate_profile(&p, SpawnMode::Fresh)
            .expect("the shell launches");
        (rig, info)
    }

    /// Waits for the first reclaim, then a little longer to see that no further reclaim comes.
    fn settled_releases(kits: &ScriptedKits) -> Vec<Release> {
        assert!(
            engram_dashboard_base::testing::wait_until(Duration::from_secs(15), || {
                !kits.released().is_empty()
            }),
            "the session ended but nobody reclaimed the materials"
        );
        std::thread::sleep(Duration::from_millis(300));
        kits.releases()
    }

    /// ★The guard does not reclaim a launched incarnation's materials — on kill, the reaper that removed the session from the roster reclaims
    /// exactly once★: not next to kill's early permission revoke (the child is alive then), holding no lock, after auto_restore is lowered. The
    /// rig swaps the material port after construction — a reaper reclaiming through the port caught at construction would bring nothing here.
    // ADR-0019
    // ADR-0334
    // TRD S25 §5-2 4 (c2) ② · ③ · ④
    #[test]
    fn the_reaper_releases_a_killed_agents_kit_once_after_the_session_is_gone() {
        let kits = ScriptedKits::preparing(Vec::new(), Vec::new());
        let (rig, info) = started_shell(&kits);

        assert_eq!(kits.prepared().len(), 1);
        assert!(
            kits.released().is_empty(),
            "the guard reclaimed a launched incarnation's materials: {:?}",
            kits.released()
        );
        rig.manager.kill_agent(info.id).expect("kill");

        let releases = settled_releases(&kits);
        assert_eq!(
            kits.released(),
            vec![(info.id, info.epoch)],
            "not reclaimed exactly once for this incarnation"
        );
        let r = &releases[0];
        assert!(r.on_reaper, "reclaimed outside the reaper: {r:?}");
        assert!(
            !r.live_session,
            "reclaimed with the session still in the roster: {r:?}"
        );
        assert!(
            r.locks_free,
            "reclaimed while holding a manager lock: {r:?}"
        );
        assert_eq!(
            r.auto_restore,
            Some(false),
            "reclaimed before auto_restore was lowered: {r:?}"
        );
    }

    /// ★A self-ended incarnation's materials are reclaimed exactly once by the reaper too★ — ended by typing `exit` into the shell.
    // TRD S25 §5-2 4 (c2) ②
    #[test]
    fn the_reaper_releases_a_naturally_exited_agents_kit_once() {
        let kits = ScriptedKits::preparing(Vec::new(), Vec::new());
        let (rig, info) = started_shell(&kits);

        // If the shell has not read the input yet, type again — write failures after it ended are ignored.
        let exited = (0..10).any(|_| {
            let _ = rig
                .manager
                .write_stdin(info.id, b"exit\r", crate::types::InputOrigin::User);
            engram_dashboard_base::testing::wait_until(Duration::from_secs(3), || {
                !kits.released().is_empty()
            })
        });
        assert!(exited, "the shell did not end with `exit`");

        let releases = settled_releases(&kits);
        assert_eq!(kits.released(), vec![(info.id, info.epoch)]);
        let r = &releases[0];
        assert!(r.on_reaper && !r.live_session, "{r:?}");
        assert_eq!(r.auto_restore, Some(false), "{r:?}");
    }

    /// ★Reclaiming happens only once, for the reap winner whose marker matches★ — a late reap of an earlier incarnation · a repeat of the same
    /// reap reclaims nothing and does not remove the current incarnation's session. A lease reclaims only its own incarnation.
    // ADR-0084
    // TRD S25 §5-2 4 (c2) ②
    #[test]
    fn a_stale_or_repeated_reap_releases_no_other_incarnation() {
        let kits = ScriptedKits::new(Vec::new());
        let rig = kits.rig();
        let manager = &rig.manager;
        let deps = ReaperDeps {
            sessions: manager.sessions.clone(),
            profiles: manager.profiles.clone(),
            status_sink: manager.status_sink.clone(),
            control: manager.control.clone(),
        };
        let id = Uuid::from_u128(0x5eed);
        let reap = |epoch: u32| crate::types::ReapMsg {
            id,
            epoch,
            reason: crate::types::TerminalReason::Killed,
            intent_at_finish: crate::types::TerminationIntent::UserKill,
            shutting_down_at_finish: true,
        };
        let lease = |epoch: u32| Some(LaunchLease::new(kits.clone(), id, epoch));

        put_live_session_leased(manager, id, 7, lease(7));
        deps.reap_one(reap(3));
        assert!(
            kits.released().is_empty(),
            "the earlier incarnation's reap reclaimed"
        );
        assert!(manager.sessions.read().unwrap().contains_key(&id));

        deps.reap_one(reap(7));
        deps.reap_one(reap(7));
        assert_eq!(
            kits.released(),
            vec![(id, 7)],
            "not once for the reap winner"
        );
        assert!(!kits.releases()[0].live_session);

        put_live_session_leased(manager, id, 9, lease(9));
        deps.reap_one(reap(7));
        assert_eq!(
            kits.released(),
            vec![(id, 7)],
            "a late reap of the earlier incarnation reclaimed the current incarnation"
        );
        assert!(manager.sessions.read().unwrap().contains_key(&id));
    }

    // ── Launch scope ──

    /// One kit call made inside a launch scope.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum ScopedCall {
        Check,
        Prepare,
        /// `working_folder` with this `cwd`.
        WorkingFolder(PathBuf),
    }

    /// What one launch scope saw — the kit calls made inside it, in order.
    #[derive(Default)]
    struct ScopeTally {
        calls: Mutex<Vec<ScopedCall>>,
    }

    /// A kit source whose scope counts the IO made in it: `begin_launch` hands out a fresh tally each time, the scoped
    /// methods record into the scope they are given, and a call outside any scope (the unscoped `check` / `prepare`, or
    /// a scoped method given a scope this source did not open) is recorded apart. `working_folder` answers `folder`, a
    /// path neither the profile nor the default canonicalize would produce.
    struct ScopedKits {
        scopes: Mutex<Vec<Arc<ScopeTally>>>,
        unscoped: Mutex<Vec<&'static str>>,
        folder: PathBuf,
        /// Runs during the check with this 1-based number (counted over every check) — a roster change between that
        /// check and its comparison.
        change_at: Mutex<Option<(usize, Meanwhile)>>,
        checks: AtomicUsize,
        profiles: OnceLock<Arc<ProfileRegistry>>,
    }

    impl ScopedKits {
        fn new(change_at: Option<(usize, Meanwhile)>) -> Arc<Self> {
            Arc::new(Self {
                scopes: Mutex::new(Vec::new()),
                unscoped: Mutex::new(Vec::new()),
                folder: PathBuf::from("/scoped/working-folder"),
                change_at: Mutex::new(change_at),
                checks: AtomicUsize::new(0),
                profiles: OnceLock::new(),
            })
        }

        fn rig(self: &Arc<Self>) -> Rig {
            let rig = Rig::new(self.clone());
            assert!(self.profiles.set(rig.manager.profiles.clone()).is_ok());
            rig
        }

        /// Each scope this source opened, with the calls made inside it.
        fn scopes(&self) -> Vec<Vec<ScopedCall>> {
            sync::lock(&self.scopes)
                .iter()
                .map(|tally| sync::lock(&tally.calls).clone())
                .collect()
        }

        fn unscoped(&self) -> Vec<&'static str> {
            sync::lock(&self.unscoped).clone()
        }

        fn record(&self, scope: &LaunchScope, call: ScopedCall, outside: &'static str) {
            match scope.get::<ScopeTally>() {
                Some(tally) => sync::lock(&tally.calls).push(call),
                None => sync::lock(&self.unscoped).push(outside),
            }
        }
    }

    impl LaunchKitSource for ScopedKits {
        fn check(&self, profile: &AgentProfile) -> Result<LaunchPlan, LaunchBlocked> {
            sync::lock(&self.unscoped).push("check");
            NoopLaunchKits.check(profile)
        }

        fn prepare(
            &self,
            plan: LaunchPlan,
            id: AgentId,
            epoch: u32,
        ) -> Result<LaunchKit, PrepareError> {
            sync::lock(&self.unscoped).push("prepare");
            NoopLaunchKits.prepare(plan, id, epoch)
        }

        fn release(&self, _id: AgentId, _epoch: u32) {}

        fn begin_launch(&self) -> LaunchScope {
            let tally = Arc::new(ScopeTally::default());
            sync::lock(&self.scopes).push(tally.clone());
            LaunchScope::new(tally)
        }

        fn check_in(
            &self,
            scope: &LaunchScope,
            profile: &AgentProfile,
        ) -> Result<LaunchPlan, LaunchBlocked> {
            self.record(scope, ScopedCall::Check, "check_in outside a scope");
            let number = self.checks.fetch_add(1, Ordering::SeqCst) + 1;
            let change = {
                let mut slot = sync::lock(&self.change_at);
                match slot.as_ref() {
                    Some((at, _)) if *at == number => slot.take(),
                    _ => None,
                }
            };
            if let Some((_, change)) = change {
                change(
                    self.profiles.get().expect("the rig set the roster"),
                    profile.id,
                );
            }
            NoopLaunchKits.check(profile)
        }

        fn prepare_in(
            &self,
            scope: &LaunchScope,
            plan: LaunchPlan,
            id: AgentId,
            epoch: u32,
        ) -> Result<LaunchKit, PrepareError> {
            self.record(scope, ScopedCall::Prepare, "prepare_in outside a scope");
            NoopLaunchKits.prepare(plan, id, epoch)
        }

        fn working_folder(&self, scope: &LaunchScope, cwd: &std::path::Path) -> PathBuf {
            self.record(
                scope,
                ScopedCall::WorkingFolder(cwd.to_path_buf()),
                "working_folder outside a scope",
            );
            self.folder.clone()
        }
    }

    /// ★One launch opens one scope, and every kit call of that launch runs in it★ — an ad-hoc create checks before the
    /// add, checks twice more when the presets change during the first prelaunch check, then prepares and asks the
    /// working folder, all in the same scope; the spawn runs in the folder the scope answered.
    // ADR-0360
    #[test]
    fn one_scope_spans_the_pre_create_check_both_prelaunch_attempts_the_prepare_and_the_working_folder(
    ) {
        let kits = ScopedKits::new(Some((2, set_presets(&["/team/b.md"]))));
        let rig = kits.rig();
        let adhoc = profile_of(spawn_path_commands().remove(0).1, Vec::new());

        let _ = rig.manager.spawn_agent(&adhoc, SpawnMode::Fresh);

        let spec = rig.spec();
        assert_eq!(
            kits.scopes(),
            vec![vec![
                ScopedCall::Check,
                ScopedCall::Check,
                ScopedCall::Check,
                ScopedCall::Prepare,
                ScopedCall::WorkingFolder(PathBuf::from(WORK)),
            ]],
            "not one scope holding pre-create check → two prelaunch checks → prepare → working folder"
        );
        assert!(kits.unscoped().is_empty(), "{:?}", kits.unscoped());
        assert_eq!(
            spec.cwd, kits.folder,
            "the spawn does not run in the scope's working folder"
        );
    }

    /// ★Every launch kind asks the working folder for the re-read roster cwd, in its one scope, and spawns there★ —
    /// activation (fresh · resume · resume without a handle) and boot restore (fresh · resume); the caller's copy holds
    /// a stale cwd.
    // ADR-0353
    // ADR-0360
    #[test]
    fn every_launch_kind_spawns_in_the_working_folder_of_the_reread_cwd_inside_one_scope() {
        let moved = PathBuf::from("/golden/moved");
        let mut cases: Vec<(String, Way, bool)> = Way::ALL
            .iter()
            .map(|way| (format!("activate {way:?}"), *way, false))
            .collect();
        cases.push(("restore Fresh".to_string(), Way::Fresh, true));
        cases.push(("restore Resume".to_string(), Way::Resume, true));
        for (case, way, restore) in cases {
            let kits = ScopedKits::new(None);
            let rig = kits.rig();
            let mut stored = way.profile();
            stored.cwd = moved.clone();
            rig.manager.profiles.upsert(stored.clone());
            let mut stale = stored.clone();
            stale.cwd = PathBuf::from(WORK);

            if restore {
                let _ = rig.manager.restore_one(&stale);
            } else {
                let _ = rig.manager.activate_profile(&stale, way.mode());
            }

            let spec = rig.spec();
            assert_eq!(
                kits.scopes(),
                vec![vec![
                    ScopedCall::Check,
                    ScopedCall::Prepare,
                    ScopedCall::WorkingFolder(moved.clone()),
                ]],
                "{case}"
            );
            assert!(kits.unscoped().is_empty(), "{case}: {:?}", kits.unscoped());
            assert_eq!(spec.cwd, kits.folder, "{case}");
        }
    }

    /// ★A moot request opens no scope★ — the scope comes after the reservation and the running check, so a request
    /// that finds the agent starting or running does no launch IO.
    #[test]
    fn a_moot_request_opens_no_scope() {
        let kits = ScopedKits::new(None);
        let rig = kits.rig();
        let p = profile_of(claude_terminal(), golden_env());
        rig.manager.profiles.upsert(p.clone());

        {
            let _held =
                SpawnReservation::reserve(rig.manager.spawning.clone(), p.id).expect("reserve");
            assert!(matches!(
                rig.manager.spawn_agent(&p, SpawnMode::Fresh),
                Ok(SpawnOutcome::Moot(None))
            ));
        }
        put_live_session(&rig.manager, p.id, 1);
        assert!(matches!(
            rig.manager.spawn_agent(&p, SpawnMode::Fresh),
            Ok(SpawnOutcome::Moot(Some(_)))
        ));
        assert!(kits.scopes().is_empty(), "{:?}", kits.scopes());
        assert!(kits.unscoped().is_empty());
    }

    // ── source order ──

    /// Cuts the production section between `start` and `end`, drops comment lines and removes all whitespace (the same shape as the helper of
    /// that name in the `manager.rs` tests).
    fn squashed(src: &str, start: &str, end: &str) -> String {
        let production = src.split("mod tests {").next().expect("production section");
        let body = production
            .split(start)
            .nth(1)
            .unwrap_or_else(|| {
                panic!("`{start}` is not in the production section — this item's premise is stale")
            })
            .split(end)
            .next()
            .expect("up to the next section");
        body.lines()
            .map(str::trim)
            .filter(|l| !l.starts_with("//"))
            .collect::<String>()
            .split_whitespace()
            .collect::<String>()
    }

    fn assert_in_order(body: &str, needles: &[&str]) {
        let mut last: Option<(&str, usize)> = None;
        for needle in needles {
            assert_eq!(
                body.matches(needle).count(),
                1,
                "`{needle}` does not occur exactly once — this item cannot pin the spot"
            );
            let at = body.find(needle).unwrap();
            if let Some((prev, prev_at)) = last {
                assert!(prev_at < at, "`{prev}` is not before `{needle}`: {body}");
            }
            last = Some((needle, at));
        }
    }

    /// ★Pins the preprocessing spot in the source★ — the tests above measure in turn on one thread, so moving it before the reservation ·
    /// running check (two launches of one agent checking together) or pushing it after the marker · session derivation (the latch · marker
    /// see a copy before alignment) leaves some green. Also checks that a block cuts off before the control-channel issue, that the roster
    /// reread is after the marker, that the comparison comes before unpacking the check's answer, and that the pre-insert check is before the name-assignment gate.
    // ADR-0319
    #[test]
    fn the_prelaunch_sits_after_the_reservation_and_before_the_incarnation_in_source() {
        let manager = include_str!("../manager.rs");
        let spawn = squashed(
            manager,
            "fn spawn_agent_watching_link(",
            "pub fn activate_profile(",
        );
        assert_in_order(
            &spawn,
            &[
                "SpawnReservation::reserve(",
                "self.get_session(profile.id)",
                "letscope=self.launch.kits.begin_launch();",
                "letinserted=self.ensure_registered(&scope,profile,entry)?;",
                "self.prelaunch(&scope,profile)?;",
                "if!inserted{self.register_for_spawn(profile,entry)?;}",
                ".epoch_for_spawn(profile.id)",
                "letreread=self.profiles.get(profile.id)",
                "letprofile=&reread;",
                "letcwd=self.launch.kits.working_folder(&scope,&profile.cwd);",
                "fresh_spawn_release_session_id(",
                "self.control.provision(",
                "backend::build_command_spec(",
            ],
        );

        let prelaunch = squashed(
            include_str!("prelaunch.rs"),
            "pub(super) fn prelaunch(",
            "#[cfg(test)]",
        );
        assert_in_order(
            &prelaunch,
            &[
                "self.profiles.get(id)",
                        // Compare right away while holding the answer (blocks included) — unpacking in between would let a block of changed inputs go out as is.
                "letchecked=self.launch.kits.check_in(scope,&profile);matchself.profiles.launch_inputs_unchanged(id,&seen)",
                "Some(true)=>{returnsettle(caller,&seen,checked)",
            ],
        );

        let ensure = squashed(manager, "fn ensure_registered(", "fn register_for_spawn(");
        assert_in_order(
            &ensure,
            &[
                "self.profiles.get(profile.id).is_some()",
                "self.check_before_create(scope,profile)?;",
                "self.lock_name_allocation()",
                ".upsert_preserving_hierarchy(&self.profiles,fresh)",
            ],
        );

        // The check is called from this file's two places only (pre-create check, prelaunch), both inside the launch's
        //   scope — another caller would split the roster-copy and lock rules, and an unscoped `check` would leave the
        //   scope. The scope is opened once, in the spawn body, and not here; the spawn path keeps no canonicalize of
        //   its own (the working folder is the kit source's answer).
        // ADR-0360
        let production = manager
            .split("mod tests {")
            .next()
            .expect("production section");
        assert!(!production.contains(".kits.check("));
        assert!(!production.contains(".kits.check_in("));
        assert_eq!(production.matches(".begin_launch()").count(), 1);
        assert!(
            !spawn.contains("dunce::canonicalize("),
            "the spawn path canonicalizes the cwd itself instead of asking the working folder"
        );
        let own = include_str!("prelaunch.rs")
            .split("mod tests {")
            .next()
            .expect("production section");
        assert!(!own.contains(".kits.check("));
        assert_eq!(own.matches(".kits.check_in(scope,").count(), 2);
        assert!(!own.contains(".begin_launch()"));
    }

    /// ★Pins the material guard spots in the source★ — arming is **before** preparation (a preparation failure is reclaimed too — the Err path
    /// exactly once · a panic ends the process in production (ADR-0292), so the boot sweep backs it — it unwinds only in tests · contain) ·
    /// preparation is after the marker · before the session-id clear and control-channel issue · materials are derived once and passed with the
    /// same value to assembly · launch · disarming is after the session is in the roster with no `?` in between (a failure between insert and
    /// disarm = two owners). A session-registration failure is out of reach of the in-turn tests. Also checks that the reaper is the only
    /// reclaimer after registration.
    // ADR-0334
    // TRD S25 §1-5 (na) 4 · 6 · 7 · §5-2 4 (c2)
    #[test]
    fn the_kit_guard_is_armed_before_prepare_and_handed_off_after_the_session_in_source() {
        let manager = include_str!("../manager.rs");
        let spawn = squashed(
            manager,
            "fn spawn_agent_watching_link(",
            "pub fn activate_profile(",
        );
        assert_in_order(
            &spawn,
            &[
                "let(plan,plan_notices)=self.prelaunch(&scope,profile)?;",
                ".epoch_for_spawn(profile.id)",
                "letmutkit_guard=LaunchKitGuard::arm(self.launch.kits.clone(),profile.id,epoch);",
                ".kits.prepare_in(&scope,plan,profile.id,epoch)",
                "letreread=self.profiles.get(profile.id)",
                "fresh_spawn_release_session_id(",
                "self.control.provision(",
                "letartifacts=backend::materialize(&profile.command,&kit,&cwd,&*self.launch.fs)",
                "backend::build_command_spec(",
                "control_endpoint.clone(),&artifacts,);",
                "self.probe_spec(&spec,&artifacts)?;",
                "control_endpoint.as_ref(),&artifacts,)?;",
                "launch_lease:kit_guard.lease(),",
                "self.spawn_session(profile.id,spec,parts,epoch,seed_events,incarnation)?;",
                "kit_guard.disarm();",
            ],
        );
        // TRD S25 §5-2 4 (c2) ①
        let handoff = spawn
            .split("self.spawn_session(profile.id,spec,parts,epoch,seed_events,incarnation)?;")
            .nth(1)
            .and_then(|rest| rest.split("kit_guard.disarm();").next())
            .expect("the session registration ~ disarm span");
        assert!(
            !handoff.contains('?'),
            "a step that can fail sits between session registration and the material guard disarm: {handoff}"
        );
        assert!(
            !spawn.contains("&LaunchArtifacts::"),
            "passes materials other than the prepared ones"
        );
        assert_eq!(
            spawn.matches("&artifacts").count(),
            3,
            "something other than assembly · probe · launch passes the materials"
        );

        // The prepare is called from the spawn path only, inside the launch's scope.
        // ADR-0360
        let production: String = manager
            .split("mod tests {")
            .next()
            .expect("production section")
            .lines()
            .map(str::trim)
            .filter(|l| !l.starts_with("//"))
            .collect::<String>()
            .split_whitespace()
            .collect();
        assert_eq!(production.matches(".kits.prepare(").count(), 0);
        assert_eq!(production.matches(".kits.prepare_in(").count(), 1);

        // TRD S25 §5-2 4 (c2) ② · ④: after session registration the reaper is the only reclaimer — no other production source of this crate
        //   (kill · failure cleanup included) reclaims. Only the guard · lease (`launch.rs`) call the source directly, and the reaper is the
        //   only one taking the lease out (the lease is not cloned and reclaiming consumes it).
        let sources = production_sources();
        let callers = |needle: &str| -> Vec<(String, usize)> {
            sources
                .iter()
                .filter_map(|(file, body)| {
                    let n = body.matches(needle).count();
                    (n > 0).then(|| (file.clone(), n))
                })
                .collect()
        };
        assert_eq!(callers("kits.release("), vec![("launch.rs".to_string(), 2)]);
        assert_eq!(
            callers(".take_launch_lease()"),
            vec![("reaper.rs".to_string(), 1)]
        );
        // The spawn path is the only place that makes a lease (making two = two owners reclaiming one set of materials).
        assert_eq!(
            callers("kit_guard.lease()"),
            vec![("manager.rs".to_string(), 1)]
        );
        // The reap winner removes the session, lets it go and revokes permissions, then reclaims **after** lowering auto_restore · the list
        //   notification (so reclaiming does not hold up ADR-0019 「lowering is last」).
        let reap = squashed(
            include_str!("../reaper.rs"),
            "pub fn reap_one(",
            "pub fn decide(",
        );
        assert_in_order(
            &reap,
            &[
                "Some(s)ifs.epoch==msg.epoch=>sessions.remove(&msg.id),",
                "letlease=removed.take_launch_lease();",
                "drop(removed);",
                "self.control.revoke(msg.id,msg.epoch);",
                "apply_disposition(",
                "self.status_sink.agent_list_updated(agents);",
                "lease.release();",
            ],
        );
    }

    /// This crate's production sources — (path under `src/`, body up to the test module with comment lines dropped and whitespace removed).
    /// Files that are entirely tests (`tests.rs` · under `tests/`) are left out.
    fn production_sources() -> Vec<(String, String)> {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).expect("read the source folder") {
                let path = entry.expect("source folder entry").path();
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if path.is_dir() {
                    if name != "tests" {
                        walk(&path, out);
                    }
                } else if name.ends_with(".rs") && name != "tests.rs" {
                    out.push(path);
                }
            }
        }
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        walk(&src, &mut files);
        assert!(
            files.iter().any(|f| f.ends_with("reaper.rs")),
            "could not scan the source folder: {}",
            src.display()
        );
        files
            .into_iter()
            .map(|file| {
                let text = std::fs::read_to_string(&file).expect("read the source");
                let production = text.split("mod tests {").next().unwrap_or_default();
                let body = production
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.starts_with("//"))
                    .collect::<String>()
                    .split_whitespace()
                    .collect::<String>();
                let rel = file
                    .strip_prefix(&src)
                    .expect("under src")
                    .to_string_lossy()
                    .replace('\\', "/");
                (rel, body)
            })
            .collect()
    }
}
