//! session_tracker — 세션 id drift 추적 **러너**(best-effort).
//!
//! ## 무엇을 소유하나
//! 관측 주기 · 에이전트별 관측기 명부 · 변경 콜백 · 단일 폴링 스레드의 수명. 그뿐이다.
//!
//! ★**어느 프로그램이 세션 id 를 어디에 어떤 모양으로 적는지는 이 파일이 모른다**★(ADR-0004): 그 지식은
//! 전부 [`SessionIdSource`] 구현체 뒤에 있고, 구현체는 `backend/<이름>/` 안에서만 만들어진다
//! ([`crate::backend::session_id_source`]). 이 모듈은 관측기를 받아 주기마다 두드릴 뿐이다.
//!
//! ## 왜 추적하나
//! 우리가 spawn 시 지정한 sid 가 그 프로그램 안에서 바뀔 수 있고, 바뀐 값을 못 따라잡으면 다음 복원이
//! 옛 세션을 살린다. 관측이 없거나 실패해도 무손상 강등된다 — 최초 지정 sid 로 이어받는 경로가 그대로
//! 살아 있다. **정확성의 1차 근거로 삼지 말 것.**
//!
//! ## 진입점
//! `watch`/`unwatch` = 명부 조작(manager 가 spawn·kill 에서 부른다) · `start`/`stop` = 스레드 수명
//! (데몬 조립이 부팅에 1회). `enabled=false` 면 `watch` 도 스레드도 no-op 이다 — claude 업데이트가
//! 세션 파일 포맷을 깨도 코드 배포 없이 끄기 위한 토글.
//!
//! tauri import 0.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use uuid::Uuid;

use crate::types::AgentId;

/// 폴링 주기. sid drift 는 사용자가 세션을 갈아탄 직후라 1초 지연은 무해하다.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

// ── 관측 포트(ADR-0004) ────────────────────────────────────────────────────────

/// 한 화신의 세션 id 를 되풀이해 관측하는 포트. 이 러너가 아는 **유일한** 백엔드 접점이다.
///
/// 구현체는 `backend/<이름>/` 안에 산다 — 어디를 읽나(파일·소켓·프로세스)와 그 포맷은 전부 구현체
/// 몫이고, 이 모듈은 주기·명부·콜백만 갖는다. 구현체를 만드는 곳은
/// [`crate::backend::session_id_source`] 하나뿐이다.
///
/// **호출 규약:** 러너 스레드가 주기마다 엔트리당 정확히 1회 [`SessionIdSource::poll`] 을 부른다.
/// 이 호출은 명부 락을 **쥔 채** 일어나므로 구현체는 블로킹을 짧게 유지해야 한다(파일 1회 읽기 수준).
/// 되돌아온 값의 콜백은 락을 놓은 뒤 불린다(ADR-0006 락 순서).
// ADR-0004
pub trait SessionIdSource: Send {
    /// 1회 관측. 관측이 한 번 실패했다고 곧장 [`SessionIdPoll::Degraded`] 를 내지 말 것 — 포기 시점의
    /// 판단은 구현체가 자기 관측 대상의 성질로 정한다.
    fn poll(&mut self) -> SessionIdPoll;
}

/// [`SessionIdSource::poll`] 1회의 결말.
#[derive(Debug, PartialEq, Eq)]
pub enum SessionIdPoll {
    /// 아직 못 읽었거나 읽었는데 그대로 — 다음 주기에 다시 묻는다.
    Unchanged,
    /// 새 sid 를 관측했다. 러너가 `on_change` 로 올린다.
    Changed(Uuid),
    /// 더 관측할 수 없다 — 러너가 이 엔트리를 **영구히** 건너뛴다(무손상 강등: 복원은 최초 지정 sid 로
    /// 정상 동작한다). 되살리려면 `unwatch` 후 다시 `watch` 해야 한다.
    Degraded,
}

// ── watch 엔트리 ───────────────────────────────────────────────────────────────

/// 에이전트 1개의 추적 상태.
struct WatchEntry {
    source: Box<dyn SessionIdSource>,
    /// 관측기가 포기를 신고한 뒤로는 다시 두드리지 않는다.
    degraded: bool,
}

// ── SessionTracker ─────────────────────────────────────────────────────────────

/// 러너 설정. enabled=false 면 모든 watch 가 no-op(feature 토글).
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

/// 모든 에이전트를 **단일 스레드**로 폴링하는 추적기(H-1.6: 에이전트당 스레드 금지).
/// `on_change(agent_id, new_sid)` 콜백은 manager 가 ProfileRegistry::observe_session_id 로 연결한다.
pub struct SessionTracker {
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
            enabled: config.enabled,
            poll_interval: config.poll_interval,
            watched: Arc::new(Mutex::new(HashMap::new())),
            on_change,
            stop: Arc::new(AtomicBool::new(false)),
            handle: Mutex::new(None),
        }
    }

    /// 같은 id 재등록 = 관측기 교체(화신이 바뀌면 옛 관측기는 옛 PID 를 본다).
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

    /// 추적 해제(kill/respawn 시 — 좀비 엔트리 방지).
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
                    // 변경을 lock 안에서 수집하고, 콜백은 lock 해제 후 호출한다
                    // (콜백이 다시 tracker를 건드려도 데드락 없도록).
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

    /// 대본대로 답하는 관측기 — 러너의 명부·강등·콜백만 재고 실제 관측은 하지 않는다.
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
