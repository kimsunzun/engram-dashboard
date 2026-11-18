//! Session reaper — the single consumer of the exit classification (ADR-0019).
//!
//! A **single supervisor thread** consumes the `ReapMsg` that the pump emits when it is the finish winner, and does the following:
//! remove from the sessions map (after checking the epoch matches) → profile disposition (auto_restore=false downgrade /
//! left untouched — auto-delete abolished by ADR-0083) → list notification.
//!
//! Invariants:
//! - The kill 2 verbs (ADR-0001) and finalize once (ADR-0005) are not touched by the reaper — it only consumes the done
//!   signal. ReapMsg is emitted once, on the finalize winner path.
//! - Lock order (ADR-0006): the sessions write-lock section = epoch check + remove only. ProfileRegistry
//!   mutate (disk IO) and the status_sink notification are outside the lock.
//! - epoch (ADR-0007/0084): checks the incarnation marker **matches** before reaping → an old done cannot wrongly delete a respawned new session.
//!   A match, not an ordering — the marker is a random number drawn per incarnation, so order has no meaning (`AgentProfile::epoch`).
//!   The same epoch-guard is extended up to apply_disposition (ADR-0084) → a stale reap cannot degrade the auto_restore of a live session
//!   attached by reactivation (a different marker).
//! - idempotency: only the single sessions.remove() Some winner does disposition and notification (once even if the same done comes twice).
//!
//! tauri import 0.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, RwLock};
use std::thread::JoinHandle;

use engram_dashboard_base::panic::payload_text;
use engram_dashboard_base::sync;

use crate::profile::ProfileRegistry;
use crate::session::AgentSession;
use crate::types::{AgentId, AgentInfo, ControlChannel, Disposition, ReapMsg, StatusSink};

/// Even without Stop, when every Sender is dropped recv ends with Err and the loop exits (double safety).
pub enum ReaperCmd {
    Reap(ReapMsg),
    Stop,
}

/// Shares the AgentManager's field Arcs **as is** — the manager and the reaper see the same
/// sessions/profiles/status_sink/control (no copies).
pub struct ReaperDeps {
    pub sessions: Arc<RwLock<HashMap<AgentId, Arc<AgentSession>>>>,
    pub profiles: Arc<ProfileRegistry>,
    pub status_sink: Arc<dyn StatusSink>,
    pub control: Arc<dyn ControlChannel>,
}

impl ReaperDeps {
    /// Handles one reap (ADR-0019 §reap_one). This function is called only from the reaper thread (or tests).
    pub fn reap_one(&self, msg: ReapMsg) {
        // 1. Write-lock section = epoch check + remove only (ADR-0006). Released right after the Arc clone.
        //    ★poison-tolerant★: even if another thread panics while holding the sessions lock and the lock
        //    is poisoned, the reaper must keep reaping (zombie prevention). The data is just a HashMap and no invariant
        //    is broken, so it recovers the guard and proceeds (double safety with catch_unwind). The place that holds the write lock is
        //    outside the containment marker, so in the production process the hook ends that panic first — this is the net for test builds without the hook.
        // ADR-0292
        let removed = {
            let mut sessions = sync::write(&self.sessions);
            match sessions.get(&msg.id) {
                Some(s) if s.epoch == msg.epoch => sessions.remove(&msg.id),
                _ => return,
            }
        };

        // 2. Only the single remove Some winner proceeds below (idempotency).
        if removed.is_none() {
            return;
        }
        // Drops Arc<AgentSession>. ★Does not assume it is the "last" reference★: the resume-failure observation
        //   (`AgentManager::early_activation_verdict`) holds the same Arc during the early-exit window, so on that
        //   path the actual release is pushed back **until that observer's next poll** (currently every 100ms). Harmless —
        //   the terminal transition is the pump's alone (ADR-0005), and the drop of the group owner (`GroupOwner`) held by the link runs as is when the last
        //   reference goes away. Usually the child process is already dead, so only handle reclamation is deferred.
        // ★The end of a contained pump panic is different (ADR-0292 decision 6)★: the CLI reaches here alive, and this drop (if it is the last reference)
        //   lets go of the group owner so `KILL_ON_JOB_CLOSE` ends the CLI tree — if the observer above holds it, that much later.
        //   On non-Windows the group owner is a no-op, so the CLI may remain (unmeasured).
        drop(removed);

        // 2.5. ★This is the single convergence point of every terminal★ — crash, EOF, normal exit, user kill: whatever the path,
        //   it passes this point, so no discard is missed. Only the epoch-check winner comes here, so a stale terminal
        //   does not erase a live token newly attached by reactivation. Overlapping with kill_agent's pre-emptive revoke is harmless.
        // ADR-0086
        self.control.revoke(msg.id, msg.epoch);

        // 3. Applying the disposition happens outside the lock — ProfileRegistry mutate is disk IO (lock order kept).
        if !msg.shutting_down_at_finish {
            let disposition = decide(&msg);
            apply_disposition(&self.profiles, msg.id, msg.epoch, disposition);
        }

        // 4. List change notification (outside the lock, external callback). Builds a snapshot equivalent to list_agents.
        let agents = list_agents(&self.sessions, &self.profiles);
        self.status_sink.agent_list_updated(agents);

        tracing::info!(
            agent = %msg.id,
            epoch = msg.epoch,
            shutting_down = msg.shutting_down_at_finish,
            "reaped session"
        );
    }
}

/// Exit classification (ADR-0019 §decide). Decides only from the frozen snapshot (intent/shutting_down).
///
/// ```text
/// shutting_down_at_finish        => KeepAsIs               // daemon shutdown: boot restore (auto_restore as is)
/// every other exit (user kill, normal exit, crash, EOF, signal)
///                                => KeepDisableAutoRestore // keep the corpse + auto_restore=false
/// ```
/// Per the user policy (inherited from ADR-0082: "don't delete it, keep it even as a corpse"), every runtime exit only collects the session from the map
/// and keeps the profile as a corpse (backend_session_id kept → resumed with --resume on reactivation).
// ADR-0083
pub fn decide(msg: &ReapMsg) -> Disposition {
    if msg.shutting_down_at_finish {
        return Disposition::KeepAsIs;
    }
    Disposition::KeepDisableAutoRestore
}

/// Applies the disposition to the ProfileRegistry (ADR-0019, ADR-0084 epoch-guard).
/// **downgrade-only**: never raises auto_restore to true — KeepDisableAutoRestore only lowers it
/// to false (the condition for the hard-kill safety net to hold). KeepAsIs is a no-op.
///
/// ★ADR-0084 epoch-guard★: lowers auto_restore **only when** `reaped_epoch` (= ReapMsg.epoch = the profile
///   epoch the dead session read when spawned; the same value as session.epoch) **matches the current profile epoch**.
///   If, between sessions.remove and this lock-free disposition, reactivation planted a **new incarnation marker** in the profile via `epoch_for_spawn` (the manager.rs spawn
///   branch), then `p.epoch != reaped_epoch` → the downgrade is **skipped** (so that a stale reap cannot degrade
///   a live session newly attached in between). It extends the same principle as the epoch-guard of sessions.remove (ADR-0007)
///   up to the disposition layer.
/// ★Lock order (ADR-0006)★: the comparison is made **inside the update_with closure** (while holding the profile lock) —
///   the sessions lock is never taken here (disposition stays sessions-lock-free). The epoch decision is made
///   only from the profile's in-memory field, so there is no need to look at the sessions map.
/// ★This is the registry's internal entry (do not change it to `try_`)★ — even if saving fails or is refused, it lowers it in memory and sets dirty.
///   If the lowering is dropped, the in-memory `true` is carried by the next save and the dead agent comes back at the next boot restore (crash loop).
// ADR-0291 (E1)
fn apply_disposition(
    profiles: &ProfileRegistry,
    id: AgentId,
    reaped_epoch: u32,
    disposition: Disposition,
) {
    match disposition {
        Disposition::KeepDisableAutoRestore => {
            profiles.update_with(id, |p| {
                if p.epoch == reaped_epoch {
                    p.auto_restore = false;
                }
            });
        }
        Disposition::KeepAsIs => {}
    }
}

/// sessions map snapshot → AgentInfo. Split out so the reaper can build, outside the lock, the same logic as manager.list_agents
/// (for notification). Releases the sessions read lock first and then assembles — no nesting with the profiles lock (ADR-0006).
fn list_agents(
    sessions: &Arc<RwLock<HashMap<AgentId, Arc<AgentSession>>>>,
    profiles: &Arc<ProfileRegistry>,
) -> Vec<AgentInfo> {
    let snapshot: Vec<Arc<AgentSession>> = {
        // poison-tolerant (same reason as reap_one step 1): it is a snapshot for notification, so it proceeds by recovering the guard.
        let guard = sync::read(sessions);
        guard.values().cloned().collect()
    };
    snapshot.iter().map(|s| session_info(s, profiles)).collect()
}

/// session → AgentInfo (a **byte-identical** mapping to manager.agent_info). Called only without holding the sessions lock.
///
/// ★ADR-0101 (WYSIWYA): must use the same name chain as manager.agent_info★ — reap_one is the hot path that broadcasts this snapshot
///   as agent_list_updated on every session exit, so if the old rule (profile.name) were used here,
///   the tree names would revert to the old full-path labels on any reap. So the canonical name is
///   derived the same way as agent_info, display_name(override) ?? basename(session.cwd), and the profile-absent
///   fallback also goes through the same shared core (name::canonical_name_or_id_fallback) (no duplicated logic).
///
/// ★cwd source = session.cwd★: the same value as what goes into AgentInfo.cwd (canonical). Deriving from profile.cwd (raw)
///   diverges from the tree basename — the same reason as agent_info (see manager.rs resolve_canonical_name).
// ADR-0101
fn session_info(session: &Arc<AgentSession>, profiles: &Arc<ProfileRegistry>) -> AgentInfo {
    use std::sync::atomic::Ordering;
    let cwd = session.cwd.to_string_lossy();
    let display_name = profiles.get(session.id).and_then(|p| p.display_name);
    let name =
        crate::name::canonical_name_or_id_fallback(display_name.as_deref(), &cwd, session.id);
    AgentInfo {
        id: session.id,
        name,
        cwd: session.cwd.to_string_lossy().to_string(),
        status: session.status(),
        cols: session.cols.load(Ordering::Relaxed),
        rows: session.rows.load(Ordering::Relaxed),
        epoch: session.epoch,
        capabilities: session.capabilities(),
        reads_messages: session.reads_messages(),
    }
}

/// Called once by the AgentManager on creation. Launches the supervisor thread that consumes ReapMsg **serially**.
pub fn spawn_reaper(deps: ReaperDeps) -> (Sender<ReaperCmd>, JoinHandle<()>) {
    let (tx, rx): (Sender<ReaperCmd>, Receiver<ReaperCmd>) = std::sync::mpsc::channel();
    let handle = std::thread::Builder::new()
        .name("engram-reaper".into())
        .spawn(move || {
            while let Ok(cmd) = rx.recv() {
                match cmd {
                    // ★The reaper is not contained (ADR-0292)★ — in the production process a reap_one panic
                    //   is ended by the hook, so the catch_unwind below is reached only in test builds without the hook.
                    //   What it prevents in that build: the reaper is a global single thread, so if one reap_one panic (e.g. lock
                    //   poison→expect, a panic inside decide/apply) killed the whole thread, **every later session would
                    //   never be removed from the map and become a zombie**. &deps is reused after unwinding, so
                    //   it is wrapped in AssertUnwindSafe (deps is not moved here).
                    ReaperCmd::Reap(msg) => {
                        let deps = &deps;
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                            move || deps.reap_one(msg),
                        ));
                        if let Err(e) = result {
                            let detail = payload_text(&*e);
                            tracing::error!(panic = %detail, "reap_one panicked — reaper loop survives, continuing with the next message");
                        }
                    }
                    ReaperCmd::Stop => break,
                }
            }
            tracing::debug!("reaper thread stopped");
        })
        .expect("spawn reaper thread");
    (tx, handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{TerminalReason, TerminationIntent};

    fn msg(intent: TerminationIntent, shutting_down: bool, reason: TerminalReason) -> ReapMsg {
        ReapMsg {
            id: uuid::Uuid::new_v4(),
            epoch: 0,
            reason,
            intent_at_finish: intent,
            shutting_down_at_finish: shutting_down,
        }
    }

    #[test]
    fn decide_user_kill_keeps_corpse() {
        let m = msg(TerminationIntent::UserKill, false, TerminalReason::Killed);
        assert_eq!(decide(&m), Disposition::KeepDisableAutoRestore);
    }

    #[test]
    fn decide_clean_exit_keeps_corpse() {
        // The case that closes the code-0 gap (ADR-0082 §Open items ②).
        let m = msg(
            TerminationIntent::None,
            false,
            TerminalReason::Exited { code: Some(0) },
        );
        assert_eq!(decide(&m), Disposition::KeepDisableAutoRestore);
    }

    #[test]
    fn decide_crash_keeps_and_disables() {
        // exit 1 = crash.
        let m = msg(
            TerminationIntent::None,
            false,
            TerminalReason::Exited { code: Some(1) },
        );
        assert_eq!(decide(&m), Disposition::KeepDisableAutoRestore);
    }

    #[test]
    fn decide_unknown_code_is_crash() {
        for reason in [
            TerminalReason::Exited { code: None },
            TerminalReason::StreamClosed,
            TerminalReason::Error("boom".into()),
        ] {
            let m = msg(TerminationIntent::None, false, reason);
            assert_eq!(decide(&m), Disposition::KeepDisableAutoRestore);
        }
    }

    #[test]
    fn decide_shutting_down_keeps_as_is() {
        // The fixture's UserKill and exit1 are there to show that "those two axes are unrelated".
        let m = msg(
            TerminationIntent::UserKill,
            true,
            TerminalReason::Exited { code: Some(1) },
        );
        assert_eq!(decide(&m), Disposition::KeepAsIs);
    }
}
