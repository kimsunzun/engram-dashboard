//! PtyManager — Phase 1 결합부. session/drain/platform/types를 묶어 에이전트 생명주기를 관리한다.
//!
//! tauri import 0 — 상위 상태 알림은 StatusSink trait으로 주입받는다(AppHandle 아님).
//!
//! 락 순서(LLD §10 규칙1): `sessions` RwLock은 조회 전용이다. Arc를 clone하고 lock을
//! 즉시 해제한 뒤에야 session 내부 lock을 취득한다. sessions lock 보유 중 session 내부
//! lock 취득은 금지(데드락 방지).

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use portable_pty::{native_pty_system, CommandBuilder, PtySize};

use crate::pty::drain::spawn_drain_thread;
use crate::pty::session::{PtySession, PtySessionInit};
use crate::pty::types::{
    AgentId, AgentInfo, AgentStatus, OutputSink, PtyChunk, PtyError, SinkId, StatusSink,
};

#[cfg(windows)]
use crate::pty::platform::JobObjectHandle;

const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

/// 검증용 기본 셸. 셸/명령 인자화는 추후(LLD: 본래 "claude"). 지금은 기본 동작 우선.
#[cfg(windows)]
fn default_shell() -> &'static str {
    "cmd.exe"
}
#[cfg(not(windows))]
fn default_shell() -> &'static str {
    "bash"
}

pub struct PtyManager {
    sessions: Arc<RwLock<HashMap<AgentId, Arc<PtySession>>>>,
    // C1: Tauri AppHandle이 아니라 StatusSink trait 주입(테스트 시 Noop 가능).
    status_sink: Arc<dyn StatusSink>,
}

impl PtyManager {
    pub fn new(status_sink: Arc<dyn StatusSink>) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            status_sink,
        }
    }

    /// PTY spawn + drain thread 시작. 성공 시 AgentInfo 반환.
    pub fn spawn_agent(&self, cwd: &Path) -> Result<AgentInfo, PtyError> {
        // 1. PTY 생성 (기본 24x80)
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: DEFAULT_ROWS,
                cols: DEFAULT_COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| PtyError::SpawnFailed(format!("openpty: {e}")))?;

        // 2. child spawn
        let mut cmd = CommandBuilder::new(default_shell());
        cmd.cwd(cwd);
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| PtyError::SpawnFailed(format!("spawn: {e}")))?;

        // slave는 spawn 후 불필요 — drop으로 FD 누수 방지(닫혀야 ConPTY EOF도 정상).
        drop(pair.slave);

        // 3. Windows: Job 생성 + child 편입 (spike/windows.rs 검증 순서 그대로).
        #[cfg(windows)]
        let job_handle = {
            let job = JobObjectHandle::new()?;
            if let Some(pid) = child.process_id() {
                job.assign(pid)?;
            }
            job
        };

        // 4. ★ master를 session에 넣기 전에 reader/writer를 먼저 확보 ★
        //    master가 PtySession 안으로 이동하면 try_clone_reader/take_writer 호출이 불가능해진다.
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| PtyError::SpawnFailed(format!("clone_reader: {e}")))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| PtyError::SpawnFailed(format!("take_writer: {e}")))?;

        // 5. PtySession 생성 → Arc
        let id = uuid::Uuid::new_v4();
        let session = Arc::new(PtySession::new(PtySessionInit {
            id,
            cwd: cwd.to_path_buf(),
            master: pair.master,
            writer,
            child,
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
            #[cfg(windows)]
            job_handle,
        }));

        // 6~7. drain 완료 채널 + drain thread 기동, rx/handle를 세션에 사후 주입.
        let (done_tx, done_rx) = mpsc::channel();
        *session
            .drain_done_rx
            .lock()
            .expect("drain_done_rx poisoned") = Some(done_rx);
        let handle = spawn_drain_thread(session.clone(), reader, self.status_sink.clone(), done_tx);
        *session.drain_handle.lock().expect("drain_handle poisoned") = Some(handle);

        // 8. sessions 등록 (write lock — 한 statement에서 즉시 해제).
        self.sessions
            .write()
            .expect("sessions poisoned")
            .insert(id, session.clone());

        // 9. 목록 변경 알림 (manager 책임). 개별 status_changed는 drain 단독이므로 여기선 안 함.
        let info = self.agent_info(&session);
        self.status_sink.agent_list_updated(self.list_agents());

        Ok(info)
    }

    /// 구독자 등록 + replay 전송 → SinkId. C4 로직은 session.subscribe에 있다.
    pub fn subscribe(
        &self,
        agent_id: AgentId,
        sink: Arc<dyn OutputSink>,
    ) -> Result<SinkId, PtyError> {
        let session = self.get_session(agent_id)?;
        Ok(session.subscribe(sink))
    }

    /// 구독 해제 (창 닫힘 cleanup에서 호출).
    pub fn unsubscribe(&self, agent_id: AgentId, sink_id: SinkId) -> Result<(), PtyError> {
        let session = self.get_session(agent_id)?;
        session.unsubscribe(sink_id);
        Ok(())
    }

    /// PTY stdin write.
    pub fn write_stdin(&self, agent_id: AgentId, data: &[u8]) -> Result<(), PtyError> {
        let session = self.get_session(agent_id)?;
        // writer lock만 잡는다(독립 lock). drain은 replay/subscribers만 잡으므로 교착 없음.
        let mut writer = session.writer.lock().expect("writer poisoned");
        writer
            .write_all(data)
            .map_err(|e| PtyError::WriteFailed(e.to_string()))?;
        writer
            .flush()
            .map_err(|e| PtyError::WriteFailed(e.to_string()))?;
        Ok(())
    }

    /// PTY cols/rows 변경.
    pub fn resize(&self, agent_id: AgentId, cols: u16, rows: u16) -> Result<(), PtyError> {
        let session = self.get_session(agent_id)?;
        // master가 이미 take된(=killed) 경우는 조용히 무시하고 atomic만 갱신.
        if let Some(master) = session.master.lock().expect("master poisoned").as_ref() {
            master
                .resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| PtyError::SpawnFailed(format!("resize: {e}")))?;
        }
        session.cols.store(cols, Ordering::Relaxed);
        session.rows.store(rows, Ordering::Relaxed);
        Ok(())
    }

    /// 에이전트 종료 — ★LLD §6 6단계 절대순서★ (spike로 검증된 순서, 변경 금지).
    ///
    /// ★상태 알림 분담★: 과도기 `Exiting`은 kill_agent가 설정·알림하고(아래 step 0.5),
    /// terminal(`Killed`/`Exited`/`Failed`)은 drain thread가 단독 전이·알림한다.
    /// drain의 terminal 가드 덕에 여기서 쓴 Exiting을 나중에 안전하게 덮어쓴다.
    pub fn kill_agent(&self, agent_id: AgentId) -> Result<(), PtyError> {
        // 0. Arc clone 후 sessions read lock 즉시 해제 (§10 규칙1).
        let session = self.get_session(agent_id)?;

        // 0.5. 과도기 Exiting 전이 — kill 누르면 즉시 '종료중' 알림.
        //      status lock 안에서 terminal이 아닐 때만 설정하고, 외부 호출(status_changed)은
        //      §10(status lock 보유 중 외부 호출 금지)에 따라 lock 해제 후 수행한다.
        let entered_exiting = {
            let mut status = session.status.lock().expect("status poisoned");
            if matches!(
                *status,
                AgentStatus::Exited { .. } | AgentStatus::Killed | AgentStatus::Failed { .. }
            ) {
                false
            } else {
                *status = AgentStatus::Exiting;
                true
            }
        };
        if entered_exiting {
            self.status_sink
                .status_changed(agent_id, AgentStatus::Exiting);
        }

        // 1. shutdown 신호 — drain이 종료 시 Killed로 전이하도록.
        session.shutdown.store(true, Ordering::Release);

        // 2~3. child kill + wait(reap, 좀비 방지). 순서 보존 위해 한 lock 구간에서.
        {
            let mut child = session.child.lock().expect("child poisoned");
            let _ = child.kill();
            let _ = child.wait();
        }

        // 4. Windows: Job 전체 종료 → 손자 프로세스까지 → ConPTY slave 핸들 해제.
        #[cfg(windows)]
        {
            let _ = session.job_handle.terminate(1);
        }

        // 5. master.take() → drop → ClosePseudoConsole → reader EOF (C3). 반드시 4 이후 5.
        let _ = session.master.lock().expect("master poisoned").take();

        // 6. drain 완료 대기 (G-1). timeout이면 handle을 그냥 두고 진행 — 세션 제거로 Arc
        //    참조가 끊기면 drain은 자연 종료된다(leak 아님).
        if let Some(rx) = session
            .drain_done_rx
            .lock()
            .expect("drain_done_rx poisoned")
            .take()
        {
            let _ = rx.recv_timeout(Duration::from_secs(5));
        }

        // 7. sessions에서 제거 (write lock — 즉시 해제).
        self.sessions
            .write()
            .expect("sessions poisoned")
            .remove(&agent_id);

        // 8. 목록 변경 알림 (manager 책임). 개별 status_changed(Killed)는 drain 단독.
        self.status_sink.agent_list_updated(self.list_agents());

        Ok(())
    }

    /// 전체 목록 스냅샷.
    pub fn list_agents(&self) -> Vec<AgentInfo> {
        // 규칙1: sessions lock 보유 중 session 내부 lock 금지 → Arc만 모으고 즉시 해제.
        let sessions: Vec<Arc<PtySession>> = {
            let guard = self.sessions.read().expect("sessions poisoned");
            guard.values().cloned().collect()
        };
        sessions.iter().map(|s| self.agent_info(s)).collect()
    }

    /// replay 스냅샷 조회.
    pub fn get_snapshot(&self, agent_id: AgentId) -> Result<Vec<PtyChunk>, PtyError> {
        let session = self.get_session(agent_id)?;
        // snapshot을 먼저 바인딩 — MutexGuard 임시값이 session보다 오래 사는 것을 방지.
        let snapshot = session.replay.lock().expect("replay poisoned").snapshot();
        Ok(snapshot)
    }

    /// 앱 종료 시 전체 정리. id를 먼저 모아 sessions lock을 풀고, 각 kill을 병렬 실행한다.
    ///
    /// 순차로 돌리면 kill_agent마다 drain 완료 recv_timeout(5s)가 직렬로 쌓여
    /// worst case N*5s가 걸린다. scoped thread로 동시 실행하면 worst case가 단일 5s로 줄고,
    /// 정상(즉시 종료) 경로는 영향이 없다. scope는 'static 없이 &self를 빌려 쓰고
    /// 스코프 종료 시 모든 스레드를 join하므로 전부 정리된 뒤 반환된다.
    /// 각 스레드 내부의 kill_agent 6단계 순서는 그대로 유지된다.
    pub fn shutdown_all(&self) {
        let ids: Vec<AgentId> = {
            let guard = self.sessions.read().expect("sessions poisoned");
            guard.keys().copied().collect()
        };
        std::thread::scope(|s| {
            for id in ids {
                s.spawn(move || {
                    let _ = self.kill_agent(id);
                });
            }
        });
    }

    // ── 내부 헬퍼 ─────────────────────────────────────────────

    /// sessions에서 Arc<PtySession>을 clone해 반환(§10 규칙1: read lock 즉시 해제).
    fn get_session(&self, agent_id: AgentId) -> Result<Arc<PtySession>, PtyError> {
        self.sessions
            .read()
            .expect("sessions poisoned")
            .get(&agent_id)
            .cloned()
            .ok_or(PtyError::NotFound(agent_id))
    }

    /// session 스냅샷 → AgentInfo. (sessions lock을 보유하지 않은 상태에서만 호출)
    fn agent_info(&self, session: &Arc<PtySession>) -> AgentInfo {
        AgentInfo {
            id: session.id,
            cwd: session.cwd.to_string_lossy().to_string(),
            status: session.status.lock().expect("status poisoned").clone(),
            cols: session.cols.load(Ordering::Relaxed),
            rows: session.rows.load(Ordering::Relaxed),
        }
    }
}
