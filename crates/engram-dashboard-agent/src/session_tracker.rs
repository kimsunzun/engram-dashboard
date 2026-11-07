//! session_tracker — the **runner** for session id drift tracking (best-effort).
//!
//! ## What it owns
//! The observation period · the per-agent observer registry · the change callback · the lifetime of the single polling thread. That is all.
//!
//! ★**Which program writes the session id where and in what shape is not known to this file**★ (ADR-0004): all that knowledge
//! is behind [`SessionIdSource`] implementations, and implementations are built only inside `backend/<이름>/`
//! ([`crate::backend::session_id_source`]). This module only takes observers and knocks on them every period.
//!
//! ## Why track
//! The sid we designated at spawn can change inside that program, and if we fail to catch up with the changed value, the next restore
//! revives the old session. Even if observation is absent or fails, it degrades without damage — the path of resuming with the first designated sid
//! stays alive as is. **Do not take it as the primary ground of correctness.**
//!
//! ## Entry points
//! `watch`/`unwatch` = registry operations (manager calls them in spawn · kill) · `start`/`stop` = thread lifetime
//! (the daemon assembly, once at boot). With `enabled=false`, both `watch` and the thread are no-ops — a toggle to turn it off without a code deployment even if a claude update
//! breaks the session file format.
//!
//! tauri import 0.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use uuid::Uuid;

use crate::types::AgentId;

/// Polling period. A sid drift is right after the user switches sessions, so a 1-second delay is harmless.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

// ── Observation port (ADR-0004) ────────────────────────────────────────────────────────

/// A port that repeatedly observes one incarnation's session id. It is the **only** backend touchpoint this runner knows.
///
/// Implementations live inside `backend/<이름>/` — where to read (file · socket · process) and its format are all the implementation's
/// job, and this module holds only the period · registry · callback. The only place that builds implementations is
/// [`crate::backend::session_id_source`].
///
/// **Call convention:** the runner thread calls [`SessionIdSource::poll`] exactly once per entry per period.
/// This call happens **while holding** the registry lock, so implementations must keep blocking short (about one file read).
/// The callback for the returned value is called after releasing the lock (ADR-0006 lock order).
// ADR-0004
pub trait SessionIdSource: Send {
    /// One observation. Do not emit [`SessionIdPoll::Degraded`] right away just because one observation failed — the judgment of
    /// when to give up is set by the implementation from the nature of its observation target.
    fn poll(&mut self) -> SessionIdPoll;
}

/// The outcome of one [`SessionIdSource::poll`].
#[derive(Debug, PartialEq, Eq)]
pub enum SessionIdPoll {
    /// Not read yet, or read and unchanged — ask again next period.
    Unchanged,
    /// A new sid was observed. The runner raises it via `on_change`.
    Changed(Uuid),
    /// Cannot observe any more — the runner skips this entry **permanently** (degrade without damage: restore works
    /// normally with the first designated sid). To revive it, `unwatch` and then `watch` again.
    Degraded,
}

// ── watch entry ───────────────────────────────────────────────────────────────

/// The tracking state of one agent.
struct WatchEntry {
    source: Box<dyn SessionIdSource>,
    /// After the observer reports giving up, it is not knocked on again.
    degraded: bool,
}

// ── SessionTracker ─────────────────────────────────────────────────────────────

/// Runner settings. With enabled=false every watch is a no-op (feature toggle).
pub struct TrackerConfig {
    pub enabled: bool,
    pub poll_interval: Duration,
}

impl Default for TrackerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            poll_interval: POLL_INTERVAL,
        }
    }
}

/// A tracker that polls all agents on a **single thread** (H-1.6: no thread per agent).
/// The `on_change(agent_id, new_sid)` callback is wired by the manager to ProfileRegistry::observe_session_id.
pub struct SessionTracker {
    enabled: bool,
    poll_interval: Duration,
    watched: Arc<Mutex<HashMap<AgentId, WatchEntry>>>,
    on_change: Arc<dyn Fn(AgentId, Uuid) + Send + Sync>,
    stop: Arc<AtomicBool>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl SessionTracker {
    /// Create the tracker (the thread is not started yet — it starts when `start()` is called).
    pub fn new(config: TrackerConfig, on_change: Arc<dyn Fn(AgentId, Uuid) + Send + Sync>) -> Self {
        Self {
            enabled: config.enabled,
            poll_interval: config.poll_interval,
            watched: Arc::new(Mutex::new(HashMap::new())),
            on_change,
            stop: Arc::new(AtomicBool::new(false)),
            handle: Mutex::new(None),
        }
    }

    /// Registering the same id again = observer replacement (when the incarnation changes the old observer would look at the old PID).
    pub fn watch(&self, agent_id: AgentId, source: Box<dyn SessionIdSource>) {
        if !self.enabled {
            return;
        }
        let mut guard = self.watched.lock().expect("watched poisoned");
        guard.insert(
            agent_id,
            WatchEntry {
                source,
                degraded: false,
            },
        );
    }

    /// Untrack (on kill/respawn — prevents zombie entries).
    pub fn unwatch(&self, agent_id: AgentId) {
        self.watched
            .lock()
            .expect("watched poisoned")
            .remove(&agent_id);
    }

    pub fn start(&self) {
        if !self.enabled {
            tracing::info!("session_tracker 비활성(토글 off) — 추적 안 함");
            return;
        }
        let mut handle_guard = self.handle.lock().expect("handle poisoned");
        if handle_guard.is_some() {
            return;
        }

        let interval = self.poll_interval;
        let watched = self.watched.clone();
        let on_change = self.on_change.clone();
        let stop = self.stop.clone();

        let handle = std::thread::Builder::new()
            .name("session-tracker".into())
            .spawn(move || {
                tracing::info!("session_tracker 스레드 시작");
                while !stop.load(Ordering::Relaxed) {
                    // Collect changes inside the lock, and call the callback after releasing the lock
                    // (so there is no deadlock even if the callback touches the tracker again).
                    let changes: Vec<(AgentId, Uuid)> = {
                        let mut guard = watched.lock().expect("watched poisoned");
                        let mut out = Vec::new();
                        for (id, entry) in guard.iter_mut() {
                            if entry.degraded {
                                continue;
                            }
                            match entry.source.poll() {
                                SessionIdPoll::Changed(new_sid) => out.push((*id, new_sid)),
                                SessionIdPoll::Unchanged => {}
                                SessionIdPoll::Degraded => entry.degraded = true,
                            }
                        }
                        out
                    };
                    for (id, new_sid) in changes {
                        (on_change)(id, new_sid);
                    }
                    std::thread::sleep(interval);
                }
                tracing::info!("session_tracker 스레드 종료");
            })
            .expect("session-tracker 스레드 생성 실패");

        *handle_guard = Some(handle);
    }

    /// Stop the polling thread + join (stop handle — H-1.6).
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.lock().expect("handle poisoned").take() {
            let _ = handle.join();
        }
    }
}

impl Drop for SessionTracker {
    fn drop(&mut self) {
        // Safety net — clean up the thread even if an explicit stop() is missed.
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.lock().expect("handle poisoned").take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An observer that answers by script — it measures only the runner's registry · degrade · callback and does no real observation.
    struct ScriptedSource {
        replies: Vec<SessionIdPoll>,
        polls: Arc<Mutex<u32>>,
    }

    impl SessionIdSource for ScriptedSource {
        fn poll(&mut self) -> SessionIdPoll {
            *self.polls.lock().unwrap() += 1;
            if self.replies.is_empty() {
                SessionIdPoll::Unchanged
            } else {
                self.replies.remove(0)
            }
        }
    }

    fn tracker(
        enabled: bool,
        on_change: Arc<dyn Fn(AgentId, Uuid) + Send + Sync>,
    ) -> SessionTracker {
        SessionTracker::new(
            TrackerConfig {
                enabled,
                poll_interval: Duration::from_millis(10),
            },
            on_change,
        )
    }

    #[test]
    fn disabled_tracker_watch_is_noop() {
        let tracker = tracker(false, Arc::new(|_, _| {}));
        tracker.watch(
            Uuid::new_v4(),
            Box::new(ScriptedSource {
                replies: vec![],
                polls: Arc::new(Mutex::new(0)),
            }),
        );
        assert!(tracker.watched.lock().unwrap().is_empty());
    }

    #[test]
    fn changed_poll_reaches_callback() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let tracker = tracker(
            true,
            Arc::new(move |id, sid| sink.lock().unwrap().push((id, sid))),
        );

        let agent = Uuid::new_v4();
        let new_sid = Uuid::new_v4();
        tracker.watch(
            agent,
            Box::new(ScriptedSource {
                replies: vec![SessionIdPoll::Unchanged, SessionIdPoll::Changed(new_sid)],
                polls: Arc::new(Mutex::new(0)),
            }),
        );
        tracker.start();

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while seen.lock().unwrap().is_empty() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        tracker.stop();

        assert_eq!(
            *seen.lock().unwrap(),
            vec![(agent, new_sid)],
            "Changed 는 콜백으로 정확히 1회 올라와야 한다"
        );
    }

    #[test]
    fn degraded_source_is_never_polled_again() {
        let polls = Arc::new(Mutex::new(0u32));
        let tracker = tracker(true, Arc::new(|_, _| {}));
        tracker.watch(
            Uuid::new_v4(),
            Box::new(ScriptedSource {
                replies: vec![SessionIdPoll::Degraded],
                polls: polls.clone(),
            }),
        );
        tracker.start();
        std::thread::sleep(Duration::from_millis(200));
        tracker.stop();

        assert_eq!(
            *polls.lock().unwrap(),
            1,
            "Degraded 신고 뒤에는 다시 두드리지 않아야 한다(주기가 10ms 라 계속 돌았다면 여러 번이다)"
        );
    }
}
