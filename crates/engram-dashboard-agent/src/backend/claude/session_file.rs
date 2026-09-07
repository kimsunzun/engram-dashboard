//! claude 세션 파일 관측 — [`SessionIdSource`] 포트의 claude 구현(ADR-0004).
//!
//! 이 파일이 소유하는 것 셋: `~/.claude/sessions/<PID>.json` 의 **경로 규약** · 그 파일의 **JSON
//! 스키마** · shim 때문에 PID 가 어긋나는 것을 되찾는 **스캔**. 러너([`crate::session_tracker`])는
//! 주기·명부·콜백만 갖고 이 셋을 하나도 모른다.
//!
//! ## 왜 관측하나
//! 우리가 spawn 시 `--session-id` 로 지정한 sid 는 `/clear`·`/resume` 로 **프로세스는 그대로인데
//! 바뀐다**(spike 실측). claude 는 바뀐 값을 위 파일에 실시간으로 적으므로, 그 파일을 우리 child PID
//! 로 읽으면 현재 sid 를 결정적으로 안다. 따라잡지 못하면 다음 복원이 옛 세션을 살린다.
//!
//! ## 등급: best-effort — correctness 를 여기 걸지 말 것
//! claude 내부 비공식 파일이라 포맷·존재가 버전마다 바뀔 수 있다. 못 읽어도 최초 지정 sid 로 이어받는
//! 경로가 그대로 살아 무손상 강등된다. 그래서:
//! - 디렉토리 watch(파일 핸들 손실) 대신 **폴링**이다 — 단순·견고.
//! - 러너의 feature 토글로 claude 업데이트가 포맷을 깨도 코드 배포 없이 끌 수 있다.
//!
//! ## PID shim 우회(H-1.1)
//! Windows 에서 `claude` 가 shim(`claude.cmd`→cmd→node)을 경유하면 우리 child PID ≠ 파일 PID 다.
//! 그래서 먼저 `<child_pid>.json` 을 보고, 안 맞으면 `sessions/*.json` 전체에서 우리가 지정한
//! (유일한) sid 를 가진 파일을 스캔해 **실제 PID 를 학습**한다(추측 아님 — 결정적).
// ADR-0004

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use uuid::Uuid;

use crate::session_tracker::{SessionIdPoll, SessionIdSource};
use crate::types::AgentId;

/// 세션 파일이 안 보일 때 포기까지의 폴링 횟수(≈ 러너 주기 × N 의 관측 윈도).
const MAX_RESOLVE_ATTEMPTS: u32 = 15;
/// 파일 읽기 공유 위반(claude 가 쓰는 중) 시 짧은 재시도 횟수.
const READ_RETRIES: u32 = 3;

// ── 세션 파일 표현(관대한 파싱) ────────────────────────────────────────────────

/// `sessions/<pid>.json` 의 부분 표현. 비공식 파일이라 모든 필드를 optional 로 두고
/// 알 수 없는 필드는 무시한다(serde 기본).
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

/// 부재면 `None`, 공유 위반 등 일시 오류는 짧게 재시도.
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

/// `<claude config dir>/sessions`. home 도 `CLAUDE_CONFIG_DIR` 도 못 찾으면 None.
fn sessions_dir() -> Option<PathBuf> {
    Some(super::config_dir()?.join("sessions"))
}

// ── PID 해석 ───────────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
enum ResolveOutcome {
    /// `<child_pid>.json` 의 sessionId 가 우리 지정값과 일치 — shim 없음(이상적).
    DirectMatch { pid: u32 },
    /// child_pid 는 안 맞고 스캔으로 다른 PID 에서 sid 발견 — shim 경유 추정.
    ScanMatch { pid: u32 },
    /// 아직 미생성이거나 추적 불가.
    NotFound,
}

/// 우리가 지정한 (유일한) `expected` sid 로 실제 세션 파일의 PID 를 결정적으로 찾는다.
/// 디렉토리 전체 스캔은 §8 이 금지한 "최신 파일 추정" 과 다르다 — 우리 sid 는 유일하므로
/// 매칭이 결정적이다.
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

// ── 관측기 ─────────────────────────────────────────────────────────────────────

/// 에이전트 1개의 claude 세션 파일 관측 상태.
pub(super) struct ClaudeSessionIdSource {
    dir: PathBuf,
    /// 진단 로그 상관 키 전용 — 무엇을 읽을지 고르는 데는 쓰지 않는다.
    agent_id: AgentId,
    child_pid: u32,
    /// 최초 우리가 지정한 sid — PID 학습(스캔)의 키.
    expected_sid: Uuid,
    /// PID shim 우회로 학습한 실제 파일 PID. None 이면 아직 미해석.
    resolved_pid: Option<u32>,
    /// 마지막으로 관측한 sid(초기 = expected). 이와 달라지면 변경으로 본다.
    last_seen_sid: Uuid,
    /// 미해석 상태에서의 시도 횟수(포기 판단용).
    attempts: u32,
}

impl ClaudeSessionIdSource {
    /// `None` = sessions 디렉토리를 못 찾았다 → 이 화신은 관측 대상이 아니다(무손상).
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
            // ── 아직 PID 미해석: 해석 시도 ──
            None => {
                self.attempts += 1;
                match resolve_in_dir(&self.dir, self.child_pid, self.expected_sid) {
                    ResolveOutcome::DirectMatch { pid } => {
                        self.resolved_pid = Some(pid);
                        self.last_seen_sid = self.expected_sid;
                        tracing::info!(agent = %self.agent_id, pid, "session_tracker: PID 직접 일치(shim 없음)");
                        SessionIdPoll::Unchanged
                    }
                    ResolveOutcome::ScanMatch { pid } => {
                        self.resolved_pid = Some(pid);
                        self.last_seen_sid = self.expected_sid;
                        // 가정 붕괴 조기감지(H-1.1): 우리 child PID 와 파일 PID 가 다르다 = shim.
                        tracing::warn!(
                            agent = %self.agent_id,
                            child_pid = self.child_pid,
                            resolved_pid = pid,
                            "session_tracker: PID shim 감지 — 스캔으로 실제 PID 학습"
                        );
                        SessionIdPoll::Unchanged
                    }
                    ResolveOutcome::NotFound => {
                        if self.attempts >= MAX_RESOLVE_ATTEMPTS {
                            tracing::warn!(
                                agent = %self.agent_id,
                                child_pid = self.child_pid,
                                attempts = self.attempts,
                                "session_tracker: 세션 파일 미발견 — 추적 degraded(무손상, 복원은 정상 동작)"
                            );
                            return SessionIdPoll::Degraded;
                        }
                        SessionIdPoll::Unchanged
                    }
                }
            }
            // ── PID 해석됨: 현재 sid 를 읽어 변경 감지 ──
            Some(pid) => {
                let path = self.dir.join(format!("{pid}.json"));
                let Some(sf) = read_session_path(&path) else {
                    return SessionIdPoll::Unchanged;
                };
                // PID 재사용 stale 방어: 파일의 pid 필드가 우리가 학습한 pid 와 같을 때만 신뢰.
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
                        "session_tracker: 세션 id 변경 감지(/clear 등)"
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
        // 우리 child_pid 는 2000 인데, 실제 파일은 다른 PID(3000)에 우리 sid 로 존재(shim).
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

        assert_eq!(source.poll(), SessionIdPoll::Unchanged);
        assert_eq!(source.resolved_pid, Some(1000));

        // /clear 시뮬레이션 — 같은 PID 파일의 sessionId 교체
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
            "MAX_RESOLVE_ATTEMPTS 번째 시도에서 포기해야 함"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
