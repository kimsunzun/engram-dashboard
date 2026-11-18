//! Observing the claude session file — the claude implementation of the [`SessionIdSource`] port (ADR-0004).
//!
//! This file owns three things: the **path convention** of `~/.claude/sessions/<PID>.json` · the **JSON schema** of that file · the
//! **scan** that recovers the PID mismatch caused by the shim. The runner ([`crate::session_tracker`]) knows none of these.
//!
//! ## Why observe
//! The sid we assign with `--session-id` at spawn **changes while the process stays the same** through `/clear` · `/resume` (measured in a
//! spike). claude writes the changed value into that file in real time, so reading it by our child PID gives the current sid
//! deterministically; missing it makes the next restore revive the old session.
//!
//! ## Grade: best-effort — do not hang correctness on this
//! An unofficial internal claude file: its format and existence may change per version. If unreadable, resuming from the first assigned
//! sid survives as is. So:
//! - **Polling** instead of a directory watch (file handle loss) — simple and robust.
//! - The runner's feature toggle turns it off without a code deploy if a claude update breaks the format.
//!
//! ## PID shim bypass (H-1.1)
//! On Windows, `claude` behind a shim (`claude.cmd`→cmd→node) makes our child PID ≠ the file's PID. So it reads `<child_pid>.json` first,
//! and on a mismatch scans `sessions/*.json` for the file with the sid we assigned (unique) and **learns the actual PID** (deterministic,
//!   not a guess).
// ADR-0004

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use uuid::Uuid;

use crate::session_tracker::{SessionIdPoll, SessionIdSource};
use crate::types::AgentId;

/// Number of polls before giving up when the session file is not visible (≈ the observation window of runner period × N).
const MAX_RESOLVE_ATTEMPTS: u32 = 15;
/// Short retries on a file read sharing violation (while claude is writing).
const READ_RETRIES: u32 = 3;

// ── Session file representation (lenient parsing) ────────────────────────────────────────────────

/// A partial representation of `sessions/<pid>.json` — unofficial, so every field is optional and unknown fields are ignored (serde default).
#[derive(Debug, Deserialize)]
struct SessionFile {
    #[serde(default)]
    pid: Option<u32>,
    #[serde(rename = "sessionId", default)]
    session_id: Option<Uuid>,
    #[serde(default)]
    #[allow(dead_code)]
    version: Option<u32>,
    #[serde(rename = "updatedAt", default)]
    #[allow(dead_code)]
    updated_at: Option<i64>,
}

fn parse_session_json(bytes: &[u8]) -> Option<SessionFile> {
    serde_json::from_slice::<SessionFile>(bytes).ok()
}

fn read_session_path(path: &Path) -> Option<SessionFile> {
    for attempt in 0..READ_RETRIES {
        match fs::read(path) {
            Ok(bytes) => return parse_session_json(&bytes),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
            Err(_) => {
                if attempt + 1 < READ_RETRIES {
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
    }
    None
}

/// `<claude config dir>/sessions`. None if neither home nor `CLAUDE_CONFIG_DIR` can be found.
fn sessions_dir() -> Option<PathBuf> {
    Some(super::config_dir()?.join("sessions"))
}

// ── PID resolution ───────────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
enum ResolveOutcome {
    /// The sessionId of `<child_pid>.json` matches the value we assigned — no shim (ideal).
    DirectMatch { pid: u32 },
    /// child_pid does not match, and the scan found the sid under another PID — presumed to go through a shim.
    ScanMatch { pid: u32 },
    /// Not yet created, or untraceable.
    NotFound,
}

/// Finds the real session file's PID by the (unique) `expected` sid we assigned. A whole-directory scan is not the "guess the latest file"
/// that §8 forbids — our sid is unique, so the match is deterministic.
fn resolve_in_dir(dir: &Path, child_pid: u32, expected: Uuid) -> ResolveOutcome {
    let direct = dir.join(format!("{child_pid}.json"));
    if let Some(sf) = read_session_path(&direct) {
        if sf.session_id == Some(expected) {
            return ResolveOutcome::DirectMatch { pid: child_pid };
        }
    }

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            if let Some(sf) = read_session_path(&path) {
                if sf.session_id == Some(expected) {
                    if let Some(pid) = sf.pid {
                        return ResolveOutcome::ScanMatch { pid };
                    }
                }
            }
        }
    }

    ResolveOutcome::NotFound
}

// ── Observer ─────────────────────────────────────────────────────────────────────

pub(super) struct ClaudeSessionIdSource {
    dir: PathBuf,
    /// Only a correlation key for diagnostic logs — not used to choose what to read.
    agent_id: AgentId,
    child_pid: u32,
    /// The sid we first assigned — the key of PID learning (scan).
    expected_sid: Uuid,
    /// The actual file PID learned through the PID shim bypass. None means not yet resolved.
    resolved_pid: Option<u32>,
    /// The sid last observed (initially = `expected_sid`) — a difference from it counts as a change.
    last_seen_sid: Uuid,
    attempts: u32,
}

impl ClaudeSessionIdSource {
    /// `None` = the sessions directory was not found → this incarnation is not an observation target (no damage).
    pub(super) fn new(agent_id: AgentId, child_pid: u32, expected_sid: Uuid) -> Option<Self> {
        Some(Self {
            dir: sessions_dir()?,
            agent_id,
            child_pid,
            expected_sid,
            resolved_pid: None,
            last_seen_sid: expected_sid,
            attempts: 0,
        })
    }
}

impl SessionIdSource for ClaudeSessionIdSource {
    fn poll(&mut self) -> SessionIdPoll {
        match self.resolved_pid {
            // ── PID not yet resolved: try to resolve ──
            None => {
                self.attempts += 1;
                match resolve_in_dir(&self.dir, self.child_pid, self.expected_sid) {
                    ResolveOutcome::DirectMatch { pid } => {
                        self.resolved_pid = Some(pid);
                        self.last_seen_sid = self.expected_sid;
                        tracing::info!(agent = %self.agent_id, pid, "session_tracker: PID matched directly (no shim)");
                        SessionIdPoll::Unchanged
                    }
                    ResolveOutcome::ScanMatch { pid } => {
                        self.resolved_pid = Some(pid);
                        self.last_seen_sid = self.expected_sid;
                        // Early detection of a broken assumption (H-1.1): our child PID differs from the file PID = shim.
                        tracing::warn!(
                            agent = %self.agent_id,
                            child_pid = self.child_pid,
                            resolved_pid = pid,
                            "session_tracker: PID shim detected — learning the real PID by scanning"
                        );
                        SessionIdPoll::Unchanged
                    }
                    ResolveOutcome::NotFound => {
                        if self.attempts >= MAX_RESOLVE_ATTEMPTS {
                            tracing::warn!(
                                agent = %self.agent_id,
                                child_pid = self.child_pid,
                                attempts = self.attempts,
                                "session_tracker: session file not found — tracking degraded (without damage; restore works normally)"
                            );
                            return SessionIdPoll::Degraded;
                        }
                        SessionIdPoll::Unchanged
                    }
                }
            }
            // ── PID resolved: read the current sid and detect a change ──
            Some(pid) => {
                let path = self.dir.join(format!("{pid}.json"));
                let Some(sf) = read_session_path(&path) else {
                    return SessionIdPoll::Unchanged;
                };
                // Stale defense against PID reuse: trust only when the file's pid field equals the pid we learned.
                if sf.pid != Some(pid) {
                    return SessionIdPoll::Unchanged;
                }
                let Some(current) = sf.session_id else {
                    return SessionIdPoll::Unchanged;
                };
                if current != self.last_seen_sid {
                    let old = self.last_seen_sid;
                    self.last_seen_sid = current;
                    tracing::info!(
                        agent = %self.agent_id,
                        %old,
                        new = %current,
                        "session_tracker: session id change detected (/clear etc.)"
                    );
                    SessionIdPoll::Changed(current)
                } else {
                    SessionIdPoll::Unchanged
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("engram-tracker-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_session(dir: &Path, pid: u32, sid: Uuid) {
        let json = format!(r#"{{"pid":{pid},"sessionId":"{sid}","status":"idle"}}"#);
        fs::write(dir.join(format!("{pid}.json")), json).unwrap();
    }

    fn source_in(dir: &Path, child_pid: u32, sid: Uuid) -> ClaudeSessionIdSource {
        ClaudeSessionIdSource {
            dir: dir.to_path_buf(),
            agent_id: Uuid::new_v4(),
            child_pid,
            expected_sid: sid,
            resolved_pid: None,
            last_seen_sid: sid,
            attempts: 0,
        }
    }

    #[test]
    fn resolve_direct_match() {
        let dir = temp_dir("direct");
        let sid = Uuid::new_v4();
        write_session(&dir, 1000, sid);
        assert_eq!(
            resolve_in_dir(&dir, 1000, sid),
            ResolveOutcome::DirectMatch { pid: 1000 }
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_scan_match_on_pid_shim() {
        let dir = temp_dir("scan");
        let sid = Uuid::new_v4();
        // Our child_pid is 2000, but the actual file exists under another PID (3000) with our sid (shim).
        write_session(&dir, 3000, sid);
        assert_eq!(
            resolve_in_dir(&dir, 2000, sid),
            ResolveOutcome::ScanMatch { pid: 3000 }
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_not_found() {
        let dir = temp_dir("notfound");
        assert_eq!(
            resolve_in_dir(&dir, 1, Uuid::new_v4()),
            ResolveOutcome::NotFound
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn poll_detects_sid_change() {
        let dir = temp_dir("change");
        let sid1 = Uuid::new_v4();
        write_session(&dir, 1000, sid1);

        let mut source = source_in(&dir, 1000, sid1);

        // ADR-0226: if the first poll emits the baseline value (the minted id) as `Changed`, the tracker persists a 0-turn id right away
        //   and D1 leaks — do not weaken.
        assert_eq!(source.poll(), SessionIdPoll::Unchanged);
        assert_eq!(source.resolved_pid, Some(1000));

        // /clear simulation — replace the sessionId of the same PID's file
        let sid2 = Uuid::new_v4();
        write_session(&dir, 1000, sid2);

        assert_eq!(source.poll(), SessionIdPoll::Changed(sid2));
        assert_eq!(source.poll(), SessionIdPoll::Unchanged);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn poll_degrades_after_max_attempts() {
        let dir = temp_dir("degrade");
        let mut source = source_in(&dir, 1, Uuid::new_v4());
        for _ in 0..MAX_RESOLVE_ATTEMPTS - 1 {
            assert_eq!(source.poll(), SessionIdPoll::Unchanged);
        }
        assert_eq!(
            source.poll(),
            SessionIdPoll::Degraded,
            "must give up on the MAX_RESOLVE_ATTEMPTS-th attempt"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
