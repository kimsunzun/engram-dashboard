//! Session reaper — the single consumer of the exit classification (ADR-0019).
//!
//! A **single supervisor thread** consumes the `ReapMsg` the pump emits as finish winner: remove from the sessions map (epoch must match)
//! → drop the control channel → profile disposition (auto_restore=false downgrade / untouched — auto-delete abolished by ADR-0083) →
//! list notification → reclaim the launch materials.
//!
//! Invariants:
//! - The kill 2 verbs (ADR-0001) and finalize once (ADR-0005) are untouched — the reaper only consumes the done signal, emitted once on
//!   the winner path.
//! - Lock order (ADR-0006): the sessions write-lock section = epoch check + remove only; ProfileRegistry mutate (disk IO) and the
//!   status_sink are outside it.
//! - epoch (ADR-0007/0084): the incarnation marker must **match** before reaping, so an old done cannot delete a respawned session. A
//!   match, not an ordering (random per incarnation; `AgentProfile::epoch`). The same guard extends to apply_disposition (ADR-0084).
//! - idempotency: only the single sessions.remove() Some winner does disposition and notification.
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

/// Shares the AgentManager's field Arcs **as is** — the manager and the reaper see the same sessions/profiles/status_sink/control.
/// ★The launch-material source is not kept here★ — this bundle is built in the manager constructor and does not see ports that
///   `with_launch_ports` swaps in later. Materials are reclaimed through the lease the session holds (`AgentSession::take_launch_lease`).
// TRD S25 §5-2 4 (c2) ③
pub struct ReaperDeps {
    pub sessions: Arc<RwLock<HashMap<AgentId, Arc<AgentSession>>>>,
    pub profiles: Arc<ProfileRegistry>,
    pub status_sink: Arc<dyn StatusSink>,
    pub control: Arc<dyn ControlChannel>,
}

impl ReaperDeps {
    // ADR-0019 §reap_one
    pub fn reap_one(&self, msg: ReapMsg) {
        // 1. Write-lock section = epoch check + remove only (ADR-0006). Released right after the Arc clone.
        //    ★poison-tolerant★: the reaper must keep reaping (zombie prevention) on a poisoned sessions lock — the data is a plain HashMap with
        //    no invariant to break, so it takes the guard. The lock holder is outside the containment marker: in production the hook ends that
        //    panic first; this is the net for test builds without the hook.
        // ADR-0292
        let removed = {
            let mut sessions = sync::write(&self.sessions);
            match sessions.get(&msg.id) {
                Some(s) if s.epoch == msg.epoch => sessions.remove(&msg.id),
                _ => return,
            }
        };

        let Some(removed) = removed else {
            return;
        };
        // Take the materials lease before letting the session go — step 5 below uses it.
        let lease = removed.take_launch_lease();
        // Drops Arc<AgentSession>. ★Does not assume it is the "last" reference★: the resume-failure observer
        //   (`AgentManager::early_activation_verdict`) holds the same Arc during the early-exit window, so the release can wait until its
        //   next poll (100ms). Harmless — the terminal transition is the pump's alone (ADR-0005), and the link's `GroupOwner` drops with
        //   the last reference.
        // ★A contained pump panic ends differently (ADR-0292 decision 6)★: the CLI arrives alive, and this drop (if last) releases the
        //   group owner so `KILL_ON_JOB_CLOSE` ends the CLI tree. Non-Windows: the group owner is a no-op, so the CLI may remain (unmeasured).
        drop(removed);

        // 2.5. ★The single convergence point of every terminal★ — crash, EOF, normal exit, user kill: no discard is missed. Only the
        //   epoch-check winner comes here, so a stale terminal cannot erase a live token newly attached by reactivation. Overlap with
        // kill_agent's pre-emptive revoke is harmless. ADR-0086
        self.control.revoke(msg.id, msg.epoch);

        // 3. Applying the disposition happens outside the lock — ProfileRegistry mutate is disk IO (lock order kept).
        if !msg.shutting_down_at_finish {
            let disposition = decide(&msg);
            apply_disposition(&self.profiles, msg.id, msg.epoch, disposition);
        }

        let agents = list_agents(&self.sessions, &self.profiles);
        self.status_sink.agent_list_updated(agents);

        // 5. ★The only place that reclaims launch materials after session registration★ — same marker winner · outside the lock. It
        //   runs **after** disposition · list notification so a slow or stuck reclaim does not hold up lowering auto_restore (ADR-0019
        //   「lowering is last」) or the list notification. The kill · failure cleanup paths do not reclaim (the child is still alive
        //   then and writes the volatile folder). The lease holds the source that prepared that incarnation, so ports swapped in after
        //   manager construction reach here too.
        // ★Best effort★ — the CLI tree may still be alive here (an early-decision observer holds the session `Arc` · a contained pump
        //   panic · `KILL_ON_JOB_CLOSE` is asynchronous). A reclaim that never runs (daemon exit · forced kill · a `ReapMsg` after
        //   `Stop`) is backed by the boot sweep.
        // ADR-0334
        // TRD S25 §5-2 4 (c2) ② · ④ · ⑥
        if let Some(lease) = lease {
            debug_assert_eq!(lease.epoch(), msg.epoch);
            lease.release();
        }

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
/// Policy (ADR-0082: "don't delete it, keep it as a corpse"): a runtime exit only removes the session from the map and keeps the profile
/// (backend_session_id kept → `--resume` on reactivation).
// ADR-0083
pub fn decide(msg: &ReapMsg) -> Disposition {
    if msg.shutting_down_at_finish {
        return Disposition::KeepAsIs;
    }
    Disposition::KeepDisableAutoRestore
}

/// Applies the disposition to the ProfileRegistry (ADR-0019, ADR-0084 epoch-guard).
/// **downgrade-only**: never raises auto_restore — KeepDisableAutoRestore only lowers it (the hard-kill safety net depends on it).
///   KeepAsIs is a no-op.
///
/// ★ADR-0084 epoch-guard★: lowers **only when** `reaped_epoch` (= ReapMsg.epoch = session.epoch) **matches the current profile epoch** — if
///   reactivation planted a new marker via `epoch_for_spawn` after sessions.remove, the downgrade is **skipped** (ADR-0007; a stale reap must not
///   degrade a newly attached live session).
/// ★Lock order (ADR-0006)★: compared inside the update_with closure (profile lock held); the sessions lock is never taken here.
/// ★Registry's internal entry (do not change to `try_`)★ — on save failure or refusal it still lowers in memory and sets dirty; dropping the
///   lowering would let the next save carry the in-memory `true` and the dead agent return at boot restore (crash loop).
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

/// sessions map snapshot → AgentInfo, so the reaper can build the same list as manager.list_agents (for notification) outside the lock.
/// Releases the sessions read lock first, then assembles — no nesting with the profiles lock (ADR-0006).
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

/// session → AgentInfo (**byte-identical** to manager.agent_info). Called without the sessions lock.
///
/// ★ADR-0101 (WYSIWYA): same name chain as manager.agent_info★ — reap_one broadcasts this as agent_list_updated on every exit, so
///   profile.name would revert tree names to full-path labels. Name = display_name(override) ?? basename(session.cwd); the profile-absent
///   fallback uses the shared core (name::canonical_name_or_id_fallback).
/// ★cwd source = session.cwd★ (the launch's working folder, as AgentInfo.cwd): profile.cwd (raw) would diverge from the tree basename (manager.rs
///   resolve_canonical_name).
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

pub fn spawn_reaper(deps: ReaperDeps) -> (Sender<ReaperCmd>, JoinHandle<()>) {
    let (tx, rx): (Sender<ReaperCmd>, Receiver<ReaperCmd>) = std::sync::mpsc::channel();
    let handle = std::thread::Builder::new()
        .name("engram-reaper".into())
        .spawn(move || {
            while let Ok(cmd) = rx.recv() {
                match cmd {
                    // ★The reaper is not contained (ADR-0292)★ — in production a reap_one panic is ended by the hook, so this catch_unwind
                    //   is reached only in test builds without it. There it prevents the one global reaper thread dying on a panic (lock
                    //   poison→expect, decide/apply) and **every later session staying in the map as a zombie**. &deps is reused after
                    //   unwinding → AssertUnwindSafe.
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
