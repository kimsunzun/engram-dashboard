//! AgentManager — Phase 1 결합부. backend/transport/output_core/session을 묶어 에이전트
//! 생명주기를 관리한다. S10: PtyManager→AgentManager 개명 + 신경로 전환.
//! S9: 프로필 기반 spawn + 세션 복원(restore_all) + claude 세션 추적 부착(불변).
//!
//! 신경로(S10): manager는 backend(CommandSpec 산출) → PtyTransport(자원) +
//! OutputCore(출력) → AgentSession(합성)을 조립한다. 옛 PtySession/drain.rs/claude.rs는 제거됨.
//!
//! tauri import 0 — 상위 상태 알림은 StatusSink trait으로 주입받는다(AppHandle 아님).
//!
//! 락 순서(LLD §10 규칙1): `sessions` RwLock은 조회 전용이다. Arc<AgentSession>을 clone하고
//! lock을 즉시 해제한 뒤에야 session 내부 lock(core/transport)을 취득한다. sessions lock
//! 보유 중 session 내부 lock 취득은 금지(데드락 방지).

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use crate::pty::backend;
use crate::pty::output_core::OutputCore;
use crate::pty::profile::{
    AgentProfile, ProfileRegistry, RestoreOutcome, RestoreReport, SpawnMode,
};
use crate::pty::session::AgentSession;
use crate::pty::session_tracker::SessionTracker;
use crate::pty::transport::pty::PtyTransport;
use crate::pty::transport::AgentTransport;
use crate::pty::types::{
    AgentId, AgentInfo, AgentStatus, CommandSpec, OutputChunk, OutputSink, PtyError, SinkId,
    StatusSink,
};

const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

/// resume spawn 후 이 시간 안에 비정상 종료(code≠0/Failed/Killed)하면 resume 실패로 보고
/// fresh로 fallback한다(H-1.7 "조기 종료 윈도"). 성공한 resume은 TUI라 계속 떠 있다.
const EARLY_EXIT_WINDOW: Duration = Duration::from_secs(3);
/// 복원 시 에이전트 간 spawn 간격(동시 폭주 방지 stagger).
const RESTORE_STAGGER: Duration = Duration::from_millis(200);

/// 검증·기본용 셸. 프로필 없이 빠르게 띄울 때 commands가 사용한다.
#[cfg(windows)]
pub fn default_shell() -> &'static str {
    "cmd.exe"
}
#[cfg(not(windows))]
pub fn default_shell() -> &'static str {
    "bash"
}

pub struct AgentManager {
    sessions: Arc<RwLock<HashMap<AgentId, Arc<AgentSession>>>>,
    // C1: Tauri AppHandle이 아니라 StatusSink trait 주입(테스트 시 Noop 가능).
    status_sink: Arc<dyn StatusSink>,
    // S9: 프로필 단일 소유자(sid 생성·갱신·persist) + claude 세션 추적기.
    profiles: Arc<ProfileRegistry>,
    tracker: Arc<SessionTracker>,
}

impl AgentManager {
    pub fn new(
        status_sink: Arc<dyn StatusSink>,
        profiles: Arc<ProfileRegistry>,
        tracker: Arc<SessionTracker>,
    ) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            status_sink,
            profiles,
            tracker,
        }
    }

    /// 프로필 레지스트리 접근(commands에서 CRUD에 사용).
    pub fn profiles(&self) -> &Arc<ProfileRegistry> {
        &self.profiles
    }

    // ── spawn ──────────────────────────────────────────────────────────────

    /// 프로필 기반 spawn. backend가 CommandSpec을 산출(claude면 mode에 따라
    /// `--session-id`/`--resume`). 성공 시 AgentInfo 반환.
    pub fn spawn_agent(
        &self,
        profile: &AgentProfile,
        mode: SpawnMode,
    ) -> Result<AgentInfo, PtyError> {
        // 이중 spawn 가드 — 같은 id가 이미 살아있으면 거부(맵 교체는 복원/재시작 경로 전용).
        if self
            .sessions
            .read()
            .expect("sessions poisoned")
            .contains_key(&profile.id)
        {
            return Err(PtyError::SpawnFailed(format!(
                "agent {} already running",
                profile.id
            )));
        }

        // 프로필을 레지스트리에 등록(idempotent + 즉시 persist). 복원 경로는 기존 프로필을 그대로 넘긴다.
        self.profiles.upsert(profile.clone());

        // cwd 정규화 — claude 세션 디렉토리 표기 고정(UNC 회피). 실패 시 원본 사용(best-effort).
        let cwd = dunce::canonicalize(&profile.cwd).unwrap_or_else(|_| profile.cwd.clone());

        // backend가 세션 추적 대상인지 판단(claude=true, shell=false). true면 세션 id 확보
        // (없으면 생성·persist). 생성 책임은 ProfileRegistry(H-1.4).
        let needs = backend::needs_session(&profile.command);
        let sid = if needs {
            self.profiles.ensure_session_id(profile.id)
        } else {
            None
        };

        // epoch는 레지스트리의 현재값(fallback respawn 등에서 미리 bump됨).
        let epoch = self.profiles.get(profile.id).map(|p| p.epoch).unwrap_or(0);

        // backend가 program/args/env/cwd를 중립 CommandSpec으로 산출. transport는 claude/shell을 모른다.
        let spec = backend::build_command_spec(
            &profile.command,
            mode,
            sid,
            cwd.clone(),
            profile.env.clone(),
        );

        let (session, child_pid) = self.spawn_session(profile.id, spec, epoch)?;

        // claude 세션 추적 부착(best-effort). shell은 세션 파일이 없으니 생략(needs_session=false).
        if let (Some(s), Some(pid)) = (sid, child_pid) {
            if needs {
                self.tracker.watch(profile.id, pid, s);
            }
        }

        tracing::info!(agent = %profile.id, epoch, ?mode, "에이전트 spawn");

        let info = self.agent_info(&session);
        self.status_sink.agent_list_updated(self.list_agents());
        Ok(info)
    }

    /// PtyTransport open + OutputCore 생성 + pump 기동(transport.start) + AgentSession 합성 +
    /// sessions 등록의 공통 기계부. 반환: 등록된 세션 Arc + child PID(Option).
    fn spawn_session(
        &self,
        id: AgentId,
        spec: CommandSpec,
        epoch: u32,
    ) -> Result<(Arc<AgentSession>, Option<u32>), PtyError> {
        // 1. PTY 생성 + child spawn + job 편입 + reader/writer 확보. pump는 아직 안 띄움.
        let (transport, child_pid) = PtyTransport::open(&spec, DEFAULT_COLS, DEFAULT_ROWS)?;

        // 2. 출력 측 core 생성(status Running, seq 0). transport와 분리된 출력 fanout 담당.
        let core = Arc::new(OutputCore::new(id, epoch, self.status_sink.clone()));

        // 3. transport를 trait object로 박싱.
        let transport: Box<dyn AgentTransport> = Box::new(transport);

        // 4. ★순서 중요(fable M-1)★ pump 기동을 insert보다 먼저 한다(S9 원본도 drain을 sessions
        //    insert 전에 spawn). start가 reader를 take해 pump 스레드를 띄우고 core.attach_pump로
        //    핸들/done_rx를 적재한다 — 이후 kill의 join_pump가 이걸 쓴다.
        transport.start(core.clone());

        // 5. core + transport를 AgentSession으로 합성(cols/rows atomic은 session 보유).
        let session = Arc::new(AgentSession::new(
            id,
            spec.cwd.clone(),
            epoch,
            DEFAULT_COLS,
            DEFAULT_ROWS,
            core,
            transport,
        ));

        // 6. sessions 등록 — start 후 insert(S9 원본 순서 보존). write lock은 한 statement에서 즉시 해제.
        self.sessions
            .write()
            .expect("sessions poisoned")
            .insert(id, session.clone());

        Ok((session, child_pid))
    }

    // ── 복원 (S9 코어) ───────────────────────────────────────────────────────

    /// auto_restore 프로필 전부 복원 시도. **백그라운드 스레드에서 호출할 것**(stagger·조기종료
    /// 윈도 대기로 블로킹 — setup 동기 호출 금지, H-1.8). 에이전트별 결과를 통지하고 반환한다.
    pub fn restore_all(&self) -> Vec<RestoreReport> {
        let targets = self.profiles.restorable();
        tracing::info!(count = targets.len(), "restore_all 시작");

        let mut reports = Vec::with_capacity(targets.len());
        for profile in targets {
            let outcome = self.restore_one(&profile);
            // fallback에서 epoch가 bump됐을 수 있으니 최신값을 읽는다.
            let epoch = self
                .profiles
                .get(profile.id)
                .map(|p| p.epoch)
                .unwrap_or(profile.epoch);
            let report = RestoreReport {
                agent_id: profile.id,
                epoch,
                outcome,
            };
            tracing::info!(agent = %report.agent_id, ?report.outcome, "복원 결과");
            self.status_sink.restore_result(report.clone());
            reports.push(report);
            std::thread::sleep(RESTORE_STAGGER);
        }
        reports
    }

    /// 프로필 1개 복원. claude+sid 있으면 resume 시도 후 조기종료면 fresh fallback,
    /// 그 외(shell 등)는 fresh로 시작.
    fn restore_one(&self, profile: &AgentProfile) -> RestoreOutcome {
        let resumable =
            backend::needs_session(&profile.command) && profile.claude_session_id.is_some();

        if !resumable {
            // shell이거나 sid 없는 claude → 이어받기가 아니라 새 세션 시작(Started).
            return match self.spawn_agent(profile, SpawnMode::Fresh) {
                Ok(_) => RestoreOutcome::Started,
                Err(e) => RestoreOutcome::Failed {
                    reason: e.to_string(),
                },
            };
        }

        // claude resume 시도
        match self.spawn_agent(profile, SpawnMode::Resume) {
            Err(e) => self.fallback_fresh(profile, format!("resume spawn 실패: {e}")),
            // ★fable M-1★: 성공한 claude resume은 TUI라 윈도 안에 종료하지 않는다.
            // 따라서 윈도 내 terminal 진입은 code와 무관하게 resume 실패 신호다
            // (code==0 조기 종료를 Resumed로 오판하면 빈 화면을 "복원 성공"으로 오보).
            // None(여전히 Running)만 Resumed.
            Ok(_) => match self.early_terminal_status(profile.id, EARLY_EXIT_WINDOW) {
                Some(status) => {
                    self.fallback_fresh(profile, format!("resume 조기 종료({status:?})"))
                }
                None => RestoreOutcome::Resumed,
            },
        }
    }

    /// resume 실패 시: 기존 세션 정리 → 새 sid 발급(old 이력) → epoch++ → fresh spawn.
    /// fresh마저 실패하면 `Failed`로 종결(재귀 금지 — H-1.7 종점).
    fn fallback_fresh(&self, profile: &AgentProfile, reason: String) -> RestoreOutcome {
        tracing::warn!(agent = %profile.id, %reason, "resume 실패 → fresh fallback");
        self.remove_session(profile.id);

        let old_sid = profile.claude_session_id;
        let new_sid = uuid::Uuid::new_v4();
        // sid 교체 + epoch++(맵 교체, H-1.5)를 한 번의 mutate로 — 단일 atomic persist
        // (crash window를 둘로 쪼개지 않음, fable Mn-5).
        self.profiles.update_with(profile.id, |p| {
            if let Some(old) = p.claude_session_id.take() {
                p.old_session_ids.push(old);
            }
            p.claude_session_id = Some(new_sid);
            p.epoch = p.epoch.wrapping_add(1);
        });

        let updated = self
            .profiles
            .get(profile.id)
            .unwrap_or_else(|| profile.clone());

        match self.spawn_agent(&updated, SpawnMode::Fresh) {
            Ok(_) => RestoreOutcome::FreshFallback {
                old_sid,
                new_sid,
                reason,
            },
            Err(e) => RestoreOutcome::Failed {
                reason: format!("fresh fallback도 실패: {e}"),
            },
        }
    }

    /// spawn 후 window 안에 terminal 상태가 되면 그 상태를, 안 되면 None(여전히 살아있음).
    fn early_terminal_status(&self, id: AgentId, window: Duration) -> Option<AgentStatus> {
        let deadline = Instant::now() + window;
        loop {
            let session = match self.get_session(id) {
                Ok(s) => s,
                // 맵에서 사라짐 = 비정상 → 종료로 간주.
                Err(_) => {
                    return Some(AgentStatus::Failed {
                        message: "session gone".into(),
                    })
                }
            };
            let status = session.status();
            if matches!(
                status,
                AgentStatus::Exited { .. } | AgentStatus::Killed | AgentStatus::Failed { .. }
            ) {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// 세션을 조용히 정리(상태 알림 없이) — fallback 전 옛 세션 제거 전용.
    ///
    /// ★fable C-1★: 단순 kill/take만 하고 반환하면 옛 pump 스레드가 아직 살아 있다가
    /// 뒤늦게 `status_changed(id, Killed)`를 emit한다. 직후 같은 id로 fresh respawn하면
    /// 그 stale Killed가 갓 살아난 새 세션을 덮을 수 있다. 따라서 여기서도 kill_agent처럼
    /// session.kill로 **pump 완료를 동기 대기**(join_pump)해 옛 pump의 terminal 알림이
    /// respawn보다 먼저 끝나게 한다. enter_exiting/agent_list_updated는 호출하지 않는다(silent).
    fn remove_session(&self, id: AgentId) {
        self.tracker.unwatch(id);
        let removed = self
            .sessions
            .write()
            .expect("sessions poisoned")
            .remove(&id);
        if let Some(session) = removed {
            // shutdown(자원 폐쇄, master drop) + join_pump(완료 대기). pump의 finish(Killed)는
            // 정상 발행되고 join으로 소진된다 — stale Killed가 respawn 전에 끝남(원본 C-1 동작 동일).
            session.kill(Duration::from_secs(5));
        }
    }

    // ── 구독/입출력 ────────────────────────────────────────────────────────

    /// 구독자 등록 + replay 전송 → SinkId. C4 로직은 core.subscribe에 있다.
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

    /// PTY stdin write → transport(Raw 바이트).
    pub fn write_stdin(&self, agent_id: AgentId, data: &[u8]) -> Result<(), PtyError> {
        self.get_session(agent_id)?.write_input(data)
    }

    /// PTY cols/rows 변경. resize 성공 시에만 cols/rows atomic 갱신(AgentSession 책임).
    pub fn resize(&self, agent_id: AgentId, cols: u16, rows: u16) -> Result<(), PtyError> {
        self.get_session(agent_id)?.resize(cols, rows)
    }

    /// 진행 중 작업만 중단(≠kill). PTY=0x03 주입. 프로세스는 살아 있다.
    pub fn interrupt(&self, agent_id: AgentId) -> Result<(), PtyError> {
        self.get_session(agent_id)?.interrupt()
    }

    // ── kill (LLD §6 절대순서 + S9 tracker unwatch) ──────────────────────────

    /// 에이전트 종료 — ★인과 순서 보존★. enter_exiting(Exiting 알림) → session.kill
    /// (transport.shutdown → master drop → pump EOF → core.finish(Killed) → join_pump)
    /// → sessions 제거 → tracker unwatch → 목록 알림. (spike로 검증된 순서, 변경 금지).
    pub fn kill_agent(&self, agent_id: AgentId) -> Result<(), PtyError> {
        let session = self.get_session(agent_id)?;

        // 0.5. 과도기 Exiting 전이 — kill 누르면 즉시 '종료중' 알림. 전이+발행은 core 안에서
        //      이뤄진다(manager가 트리거, core가 status_changed(Exiting) 발행). 이미 terminal이면
        //      false 반환하나 별도 처리 없음(개별 status_changed(Killed)는 pump의 finish 단독).
        let _ = session.enter_exiting();

        // 1~6. 자원 강제 종료 + pump 완료 대기. shutdown이 master를 drop해 pump read를 EOF로
        //       깨우고(→core.finish(Killed)), join_pump가 그 pump 종료를 5s 대기한다. timeout이면
        //       그냥 진행(세션 제거로 Arc 끊겨 자연 종료).
        session.kill(Duration::from_secs(5));

        // 7. sessions에서 제거 + 세션 추적 해제(S9 — 좀비 watcher 엔트리 방지).
        self.sessions
            .write()
            .expect("sessions poisoned")
            .remove(&agent_id);
        self.tracker.unwatch(agent_id);

        // 8. 목록 변경 알림 (manager 책임). 개별 status_changed(Killed)는 pump 단독.
        self.status_sink.agent_list_updated(self.list_agents());

        Ok(())
    }

    // ── 조회/종료 ─────────────────────────────────────────────────────────────

    /// 전체 목록 스냅샷.
    pub fn list_agents(&self) -> Vec<AgentInfo> {
        let sessions: Vec<Arc<AgentSession>> = {
            let guard = self.sessions.read().expect("sessions poisoned");
            guard.values().cloned().collect()
        };
        sessions.iter().map(|s| self.agent_info(s)).collect()
    }

    /// replay 스냅샷 조회.
    pub fn get_snapshot(&self, agent_id: AgentId) -> Result<Vec<OutputChunk>, PtyError> {
        let session = self.get_session(agent_id)?;
        Ok(session.snapshot())
    }

    /// 앱 종료 시 전체 정리. id를 먼저 모아 sessions lock을 풀고, 각 kill을 병렬 실행한다.
    pub fn shutdown_all(&self) {
        // S9: 세션 추적 스레드부터 정지(폴링이 정리 중인 세션을 건드리지 않게).
        self.tracker.stop();

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

    /// sessions에서 Arc<AgentSession>을 clone해 반환(§10 규칙1: read lock 즉시 해제).
    fn get_session(&self, agent_id: AgentId) -> Result<Arc<AgentSession>, PtyError> {
        self.sessions
            .read()
            .expect("sessions poisoned")
            .get(&agent_id)
            .cloned()
            .ok_or(PtyError::NotFound(agent_id))
    }

    /// session 스냅샷 → AgentInfo. (sessions lock을 보유하지 않은 상태에서만 호출)
    fn agent_info(&self, session: &Arc<AgentSession>) -> AgentInfo {
        AgentInfo {
            id: session.id,
            cwd: session.cwd.to_string_lossy().to_string(),
            status: session.status(),
            cols: session.cols.load(Ordering::Relaxed),
            rows: session.rows.load(Ordering::Relaxed),
            epoch: session.epoch,
        }
    }
}
