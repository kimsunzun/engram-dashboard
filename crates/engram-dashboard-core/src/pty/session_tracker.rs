//! session_tracker — claude 세션 id drift 추적(**best-effort**).
//!
//! ## 왜 필요한가
//! 우리가 spawn 시 `--session-id`로 지정한 sid는 `/clear`·`/resume`로 **프로세스는 그대로인데
//! 바뀐다**(spike 실측 확인). 바뀐 sid를 따라잡아 프로필에 반영해 둬야 다음 복원이 옛 세션을
//! 살리는 사고를 막는다.
//!
//! ## 메커니즘
//! claude는 `~/.claude/sessions/<PID>.json`에 현재 sessionId를 기록하고 `/clear` 시 실시간
//! 갱신한다(spike). 우리 child PID로 이 파일을 읽으면 현재 sid를 결정적으로 안다.
//!
//! ## 등급: best-effort (correctness 의존 금지)
//! 이 파일은 claude 내부 비공식 파일이라 포맷·존재가 버전마다 바뀔 수 있다. 따라서:
//! - 정확성의 1차 근거가 **아니다**. 못 읽어도 "최초 지정 sid → resume → 실패 시 fresh fallback"
//!   경로로 무손상 강등된다.
//! - 디렉토리 watch(파일 핸들 손실) 대신 **폴링**으로 단순·견고하게 간다.
//! - feature 토글(config.enabled)로 claude 업데이트가 포맷을 깨도 코드 배포 없이 끌 수 있다.
//!
//! ## PID shim 우회(H-1.1)
//! Windows에서 `claude`가 shim(`claude.cmd`→cmd→node)을 경유하면 우리 child PID ≠ 파일 PID다.
//! 그래서 먼저 `<child_pid>.json`을 보고, 안 맞으면 `sessions/*.json` 전체에서 우리가 지정한
//! (유일한) sid를 가진 파일을 1회 스캔해 **실제 PID를 학습**한다(추측 아님 — 결정적).
//!
//! tauri import 0.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use serde::Deserialize;
use uuid::Uuid;

use crate::pty::types::AgentId;

/// 폴링 주기. sid drift는 사용자가 `/clear`를 친 직후라 1초 지연은 무해하다.
const POLL_INTERVAL: Duration = Duration::from_secs(1);
/// 세션 파일이 안 보일 때 포기까지의 폴링 횟수(≈ POLL_INTERVAL * N 의 관측 윈도).
/// 초과 시 해당 에이전트 추적을 degraded(무손상)로 끈다.
const MAX_RESOLVE_ATTEMPTS: u32 = 15;
/// 파일 읽기 공유 위반(claude가 쓰는 중) 시 짧은 재시도 횟수.
const READ_RETRIES: u32 = 3;

// ── 세션 파일 표현(관대한 파싱) ────────────────────────────────────────────────

/// `sessions/<pid>.json` 의 부분 표현. 비공식 파일이라 모든 필드를 optional로 두고
/// 알 수 없는 필드는 무시한다(serde 기본). 우리가 쓰는 건 사실상 pid + sessionId 둘뿐.
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

/// 세션 파일 1개 읽기. 부재면 None, 공유 위반 등 일시 오류는 짧게 재시도.
fn read_session_path(path: &Path) -> Option<SessionFile> {
    for attempt in 0..READ_RETRIES {
        match fs::read(path) {
            Ok(bytes) => return parse_session_json(&bytes),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
            Err(_) => {
                // claude가 쓰는 중일 수 있음 — 잠깐 쉬고 재시도.
                if attempt + 1 < READ_RETRIES {
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
    }
    None
}

// ── PID 해석 ───────────────────────────────────────────────────────────────────

/// `<child_pid>.json` 직접 일치 / 스캔으로 학습 / 못 찾음.
#[derive(Debug, PartialEq, Eq)]
pub enum ResolveOutcome {
    /// `<child_pid>.json` 의 sessionId가 우리 지정값과 일치 — shim 없음(이상적).
    DirectMatch { pid: u32 },
    /// child_pid는 안 맞고 스캔으로 다른 PID에서 sid 발견 — shim 경유 추정.
    ScanMatch { pid: u32 },
    /// 못 찾음(아직 미생성이거나 추적 불가).
    NotFound,
}

/// 우리가 지정한 (유일한) `expected` sid로 실제 세션 파일의 PID를 결정적으로 찾는다.
/// 먼저 `<child_pid>.json`을 보고, 안 맞으면 디렉토리 전체를 스캔한다(§8이 금지한
/// "최신 파일 추정"과 다름 — 우리 sid는 유일하므로 매칭은 결정적).
pub fn resolve_in_dir(dir: &Path, child_pid: u32, expected: Uuid) -> ResolveOutcome {
    // 1) 직접: <child_pid>.json
    let direct = dir.join(format!("{child_pid}.json"));
    if let Some(sf) = read_session_path(&direct) {
        if sf.session_id == Some(expected) {
            return ResolveOutcome::DirectMatch { pid: child_pid };
        }
    }

    // 2) 스캔: sessions/*.json 중 sessionId == expected 인 파일의 pid.
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

// ── watch 엔트리 ───────────────────────────────────────────────────────────────

/// 에이전트 1개의 추적 상태. 단일 watcher 스레드가 polling하며 갱신한다.
struct WatchEntry {
    child_pid: u32,
    /// 최초 우리가 지정한 sid — PID 학습(스캔)의 키.
    expected_sid: Uuid,
    /// PID shim 우회로 학습한 실제 파일 PID. None이면 아직 미해석.
    resolved_pid: Option<u32>,
    /// 마지막으로 관측한 sid(초기 = expected). 이와 달라지면 변경으로 본다.
    last_seen_sid: Uuid,
    /// 미해석 상태에서의 시도 횟수(포기 판단용).
    attempts: u32,
    /// 추적 포기(무손상). self_test 실패·버전 깨짐 등에서 set.
    degraded: bool,
}

/// 한 엔트리 1회 폴링. sid 변경을 감지하면 새 sid를 반환(콜백 대상)하고 엔트리를 갱신한다.
/// 순수 로직으로 분리해 watcher 스레드 없이도 단위 테스트한다.
fn poll_entry(dir: &Path, agent_id: AgentId, entry: &mut WatchEntry) -> Option<Uuid> {
    if entry.degraded {
        return None;
    }

    match entry.resolved_pid {
        // ── 아직 PID 미해석: 해석 시도(self_test 역할도 겸함) ──
        None => {
            entry.attempts += 1;
            match resolve_in_dir(dir, entry.child_pid, entry.expected_sid) {
                ResolveOutcome::DirectMatch { pid } => {
                    entry.resolved_pid = Some(pid);
                    entry.last_seen_sid = entry.expected_sid;
                    tracing::info!(agent = %agent_id, pid, "session_tracker: PID 직접 일치(shim 없음)");
                    None
                }
                ResolveOutcome::ScanMatch { pid } => {
                    entry.resolved_pid = Some(pid);
                    entry.last_seen_sid = entry.expected_sid;
                    // 가정 붕괴 조기감지(H-1.1): 우리 child PID와 파일 PID가 다르다 = shim.
                    tracing::warn!(
                        agent = %agent_id,
                        child_pid = entry.child_pid,
                        resolved_pid = pid,
                        "session_tracker: PID shim 감지 — 스캔으로 실제 PID 학습"
                    );
                    None
                }
                ResolveOutcome::NotFound => {
                    if entry.attempts >= MAX_RESOLVE_ATTEMPTS {
                        entry.degraded = true;
                        // 무손상 강등: 추적만 끈다. 복원 정확성은 이 파일에 의존하지 않음.
                        tracing::warn!(
                            agent = %agent_id,
                            child_pid = entry.child_pid,
                            attempts = entry.attempts,
                            "session_tracker: 세션 파일 미발견 — 추적 degraded(무손상, 복원은 정상 동작)"
                        );
                    }
                    None
                }
            }
        }
        // ── PID 해석됨: 현재 sid를 읽어 변경 감지 ──
        Some(pid) => {
            let path = dir.join(format!("{pid}.json"));
            let sf = read_session_path(&path)?;
            // PID 재사용 stale 방어: 파일의 pid 필드가 우리가 학습한 pid와 같을 때만 신뢰.
            if sf.pid != Some(pid) {
                return None;
            }
            let current = sf.session_id?;
            if current != entry.last_seen_sid {
                let old = entry.last_seen_sid;
                entry.last_seen_sid = current;
                tracing::info!(
                    agent = %agent_id,
                    %old,
                    new = %current,
                    "session_tracker: 세션 id 변경 감지(/clear 등)"
                );
                Some(current)
            } else {
                None
            }
        }
    }
}

// ── 세션 디렉토리 해석 ─────────────────────────────────────────────────────────

#[cfg(windows)]
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(PathBuf::from)
}
#[cfg(not(windows))]
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// `~/.claude/sessions` 경로 해석. `CLAUDE_CONFIG_DIR`이 설정돼 있으면 우선한다(보강 §4).
pub fn default_sessions_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir).join("sessions"));
        }
    }
    home_dir().map(|h| h.join(".claude").join("sessions"))
}

// ── SessionTracker ─────────────────────────────────────────────────────────────

/// 추적기 설정. enabled=false면 모든 watch가 no-op(feature 토글).
pub struct TrackerConfig {
    pub sessions_dir: Option<PathBuf>,
    pub enabled: bool,
    pub poll_interval: Duration,
}

impl Default for TrackerConfig {
    fn default() -> Self {
        Self {
            sessions_dir: default_sessions_dir(),
            enabled: true,
            poll_interval: POLL_INTERVAL,
        }
    }
}

/// 모든 에이전트를 **단일 스레드**로 폴링하는 추적기(H-1.6: 에이전트당 스레드 금지).
/// `on_change(agent_id, new_sid)` 콜백은 manager가 ProfileRegistry::observe_session_id로 연결한다.
pub struct SessionTracker {
    dir: Option<PathBuf>,
    enabled: bool,
    poll_interval: Duration,
    watched: Arc<Mutex<HashMap<AgentId, WatchEntry>>>,
    on_change: Arc<dyn Fn(AgentId, Uuid) + Send + Sync>,
    stop: Arc<AtomicBool>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl SessionTracker {
    /// 추적기 생성(스레드는 아직 안 띄움 — `start()` 호출 시 기동).
    pub fn new(config: TrackerConfig, on_change: Arc<dyn Fn(AgentId, Uuid) + Send + Sync>) -> Self {
        Self {
            dir: config.sessions_dir,
            enabled: config.enabled,
            poll_interval: config.poll_interval,
            watched: Arc::new(Mutex::new(HashMap::new())),
            on_change,
            stop: Arc::new(AtomicBool::new(false)),
            handle: Mutex::new(None),
        }
    }

    /// 활성 여부 — 비활성(토글 off 또는 디렉토리 미해석)이면 watch가 무의미.
    fn active(&self) -> bool {
        self.enabled && self.dir.is_some()
    }

    /// 에이전트 추적 시작. 비활성이면 no-op. 이미 추적 중이면 갱신.
    pub fn watch(&self, agent_id: AgentId, child_pid: u32, expected_sid: Uuid) {
        if !self.active() {
            return;
        }
        let mut guard = self.watched.lock().expect("watched poisoned");
        guard.insert(
            agent_id,
            WatchEntry {
                child_pid,
                expected_sid,
                resolved_pid: None,
                last_seen_sid: expected_sid,
                attempts: 0,
                degraded: false,
            },
        );
    }

    /// 추적 해제(kill/respawn 시 — 좀비 엔트리 방지).
    pub fn unwatch(&self, agent_id: AgentId) {
        self.watched
            .lock()
            .expect("watched poisoned")
            .remove(&agent_id);
    }

    /// 단일 폴링 스레드 기동. 비활성이면 띄우지 않는다. 중복 호출은 무시.
    pub fn start(&self) {
        if !self.active() {
            tracing::info!(
                "session_tracker 비활성(토글 off 또는 sessions_dir 미해석) — 추적 안 함"
            );
            return;
        }
        let mut handle_guard = self.handle.lock().expect("handle poisoned");
        if handle_guard.is_some() {
            return;
        }

        let dir = self.dir.clone().expect("active()가 dir 존재 보장");
        let interval = self.poll_interval;
        let watched = self.watched.clone();
        let on_change = self.on_change.clone();
        let stop = self.stop.clone();

        let handle = std::thread::Builder::new()
            .name("session-tracker".into())
            .spawn(move || {
                tracing::info!(?dir, "session_tracker 스레드 시작");
                while !stop.load(Ordering::Relaxed) {
                    // 변경을 lock 안에서 수집하고, 콜백은 lock 해제 후 호출한다
                    // (콜백이 다시 tracker를 건드려도 데드락 없도록).
                    let changes: Vec<(AgentId, Uuid)> = {
                        let mut guard = watched.lock().expect("watched poisoned");
                        let mut out = Vec::new();
                        for (id, entry) in guard.iter_mut() {
                            if let Some(new_sid) = poll_entry(&dir, *id, entry) {
                                out.push((*id, new_sid));
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

    /// 폴링 스레드 정지 + join(정지 핸들 — H-1.6).
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.lock().expect("handle poisoned").take() {
            let _ = handle.join();
        }
    }
}

impl Drop for SessionTracker {
    fn drop(&mut self) {
        // 안전망 — 명시적 stop() 누락 시에도 스레드를 정리.
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.lock().expect("handle poisoned").take() {
            let _ = handle.join();
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
        // 우리 child_pid는 2000인데, 실제 파일은 다른 PID(3000)에 우리 sid로 존재(shim).
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
        let agent = Uuid::new_v4();
        let sid1 = Uuid::new_v4();
        write_session(&dir, 1000, sid1);

        let mut entry = WatchEntry {
            child_pid: 1000,
            expected_sid: sid1,
            resolved_pid: None,
            last_seen_sid: sid1,
            attempts: 0,
            degraded: false,
        };

        // 1차: PID 해석(변경 없음)
        assert_eq!(poll_entry(&dir, agent, &mut entry), None);
        assert_eq!(entry.resolved_pid, Some(1000));

        // /clear 시뮬레이션 — 같은 PID 파일의 sessionId 교체
        let sid2 = Uuid::new_v4();
        write_session(&dir, 1000, sid2);

        // 2차: 변경 감지
        assert_eq!(poll_entry(&dir, agent, &mut entry), Some(sid2));
        // 3차: 동일 → 변경 없음
        assert_eq!(poll_entry(&dir, agent, &mut entry), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn poll_degrades_after_max_attempts() {
        let dir = temp_dir("degrade");
        let agent = Uuid::new_v4();
        let sid = Uuid::new_v4();
        let mut entry = WatchEntry {
            child_pid: 1,
            expected_sid: sid,
            resolved_pid: None,
            last_seen_sid: sid,
            attempts: 0,
            degraded: false,
        };
        for _ in 0..MAX_RESOLVE_ATTEMPTS {
            assert_eq!(poll_entry(&dir, agent, &mut entry), None);
        }
        assert!(entry.degraded, "MAX_RESOLVE_ATTEMPTS 후 degraded여야 함");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn disabled_tracker_watch_is_noop() {
        let tracker = SessionTracker::new(
            TrackerConfig {
                sessions_dir: Some(std::env::temp_dir()),
                enabled: false,
                poll_interval: POLL_INTERVAL,
            },
            Arc::new(|_, _| {}),
        );
        tracker.watch(Uuid::new_v4(), 1, Uuid::new_v4());
        assert!(tracker.watched.lock().unwrap().is_empty());
    }
}
