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
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::agent::backend;
use crate::agent::output_core::OutputCore;
use crate::agent::profile::{
    AgentProfile, ProfileRegistry, RestoreOutcome, RestoreReport, SpawnMode,
};
use crate::agent::reaper::{self, ReaperCmd, ReaperDeps};
use crate::agent::session::AgentSession;
use crate::agent::session_tracker::SessionTracker;
use crate::agent::transport::pty::PtyTransport;
use crate::agent::transport::AgentTransport;
use crate::agent::types::{
    AgentId, AgentInfo, AgentStatus, CommandSpec, OutputChunk, OutputSink, PtyError, ReapMsg,
    SinkId, StatusSink, SubscribeOutcome, TerminalReason, TerminationIntent,
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

    // ── ADR-0019 reaper ──────────────────────────────────────
    /// 데몬/앱 셧다운 전역 플래그. shutdown_all 이 각 kill **전에** set 한다 → 그 사이 종료된
    /// 세션의 finish hook 이 true 를 snapshot 해 reaper 가 disposition 을 스킵(부팅 복원 유지).
    shutting_down: Arc<AtomicBool>,
    /// 세션/pump finish hook 이 ReapMsg 를 보내는 채널(단일 supervisor 가 소비).
    reaper_tx: Sender<ReaperCmd>,
    /// reaper 스레드 핸들. Drop 시 join(Stop 송신 후 대기) — 테스트 누수 방지.
    reaper_handle: Option<JoinHandle<()>>,
}

impl AgentManager {
    pub fn new(
        status_sink: Arc<dyn StatusSink>,
        profiles: Arc<ProfileRegistry>,
        tracker: Arc<SessionTracker>,
    ) -> Self {
        let sessions = Arc::new(RwLock::new(HashMap::new()));

        // reaper supervisor 1개 기동 — manager 와 동일한 sessions/profiles/status_sink 를 공유한다
        // (두 주체가 같은 모델을 본다). reap_one 이 lock 밖에서 disposition·통지를 수행한다.
        let deps = ReaperDeps {
            sessions: sessions.clone(),
            profiles: profiles.clone(),
            status_sink: status_sink.clone(),
        };
        let (reaper_tx, reaper_handle) = reaper::spawn_reaper(deps);

        Self {
            sessions,
            status_sink,
            profiles,
            tracker,
            shutting_down: Arc::new(AtomicBool::new(false)),
            reaper_tx,
            reaper_handle: Some(reaper_handle),
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

        // 2.5. ★ADR-0019 finish-snapshot hook 배선★. 세션별 intent atomic 신규 생성 + 전역
        //      shutting_down·reaper_tx 를 클로저로 캡처해 core 에 주입한다. core.finish 의 finalize
        //      승자 경로에서 1회 호출되며, **그 순간** intent·shutting_down 을 snapshot 해 ReapMsg 를
        //      송신한다(reap 시점 live read 금지 — 크래시→유저kill 오분류 race 방지).
        //      transport 는 이 의미를 모른다(그냥 core.finish 호출). send 실패(reaper 종료)는 무시.
        let intent = Arc::new(AtomicU8::new(TerminationIntent::None as u8));
        {
            let intent_hook = intent.clone();
            let shutting_down_hook = self.shutting_down.clone();
            let reaper_tx = self.reaper_tx.clone();
            core.set_on_terminal(Box::new(move |reason: TerminalReason| {
                let msg = ReapMsg {
                    id,
                    epoch,
                    reason,
                    // ★snapshot★: 이 두 load 가 finish 승자 순간의 frozen 값이다.
                    intent_at_finish: TerminationIntent::from_u8(
                        intent_hook.load(Ordering::SeqCst),
                    ),
                    shutting_down_at_finish: shutting_down_hook.load(Ordering::SeqCst),
                };
                let _ = reaper_tx.send(ReaperCmd::Reap(msg));
            }));
        }

        // 3. transport를 trait object로 박싱.
        let transport: Box<dyn AgentTransport> = Box::new(transport);

        // 4. core + transport를 AgentSession으로 합성(cols/rows atomic은 session 보유).
        let session = Arc::new(AgentSession::new(
            id,
            spec.cwd.clone(),
            epoch,
            DEFAULT_COLS,
            DEFAULT_ROWS,
            intent,
            core,
            transport,
        ));

        // 5. ★ADR-0019 순서 변경★ sessions 등록을 pump 기동(start)보다 **먼저** 한다.
        //    (구 S9: start 후 insert.) 이유: finish hook 이 ReapMsg 를 보내는데, pump 가 즉시
        //    EOF→finish 하면 그 시점에 세션이 맵에 있어야 reaper 가 reap 한다. insert 전에 start 하면
        //    빠른 종료 시 hook send 가 맵에 없는 id 를 가리켜 reap 가 no-op→세션 좀비화. attach_pump 는
        //    start 내부 동기 완료라 join_pump 영향 없음(insert 순서 무관). write lock 즉시 해제.
        self.sessions
            .write()
            .expect("sessions poisoned")
            .insert(id, session.clone());

        // 5.5. ★ADR-0019 활성화 — 반드시 start_pump 전★: spawn(=지금 떠 있어야 함)이면 프로필을
        //      auto_restore=true 로 확정·persist 한다(강제종료 후 부팅 복원 대상이 되게). 이 플립을
        //      pump 기동 **전**에 둬야 race 가 닫힌다: 즉시 크래시(`cmd /c exit 1`)는 start_pump 직후
        //      pump 가 EOF→finish→reaper 가 auto_restore=false 로 내리는데, 이 플립이 그보다 늦으면
        //      false 를 true 로 덮어써 크래시 세션이 부팅 복원 대상으로 잘못 남는다(크래시 루프).
        //      순서를 "플립 true → start_pump → (크래시 시) reaper false" 로 고정해 reaper 의
        //      downgrade(false)가 항상 **마지막**이 되게 한다. spawn 은 활성화 행동이므로 여기서만 올린다
        //      (reaper 는 downgrade-only — true 로 올리지 않음).
        self.profiles.update_with(id, |p| p.auto_restore = true);

        // 6. pump 기동 — reader take + pump 스레드 spawn + core.attach_pump(핸들/done_rx 적재).
        //    이제부터 출력·종료가 흐른다. 종료 시 finish hook→ReapMsg(맵에 이미 존재).
        session.start_pump();

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

    /// after_seq/epoch resume 구독 → SubscribeOutcome. epoch_matches 는 데몬이 요청 epoch 과
    /// 세션 현재 epoch 을 비교해 넘긴다(코어는 protocol 무의존이라 epoch 비교를 외부에서 받는다).
    pub fn subscribe_from(
        &self,
        agent_id: AgentId,
        sink: Arc<dyn OutputSink>,
        after_seq: Option<u64>,
        epoch_matches: bool,
        on_ready: impl FnOnce(&SubscribeOutcome),
    ) -> Result<SubscribeOutcome, PtyError> {
        let session = self.get_session(agent_id)?;
        Ok(session.subscribe_from(sink, after_seq, epoch_matches, on_ready))
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

    /// 에이전트 종료 — ★인과 순서 보존 + ADR-0019 reaper 위임★.
    /// intent=UserKill 태깅(shutdown **전**) → enter_exiting(Exiting 알림) → session.kill
    /// (transport.shutdown → master drop → pump EOF → core.finish(Killed)+finish hook→ReapMsg
    /// → join_pump). **맵 제거·disposition·통지는 하지 않는다** — pump 가 보낸 ReapMsg 를 reaper 가
    /// 단일 소비해 처리한다(done 단일 소비자). tracker unwatch 만 직접(reaper 는 tracker 를 모름).
    ///
    /// 의미 변경: 맵 제거가 reaper(비동기)로 옮겨졌다. kill_agent 반환 직후엔 아직 맵에 있을 수
    /// 있으므로, 호출자가 "사라짐"을 단언하려면 폴링해야 한다(headless 테스트가 그렇게 한다).
    pub fn kill_agent(&self, agent_id: AgentId) -> Result<(), PtyError> {
        let session = self.get_session(agent_id)?;

        // 0. ★intent 태깅을 shutdown 전에★ — finish hook 이 finish 순간 snapshot 하므로, shutdown
        //    이 pump 를 깨워 finish 하기 전에 UserKill 이 보여야 reaper 가 DeleteProfile 로 분류한다.
        session.set_intent(TerminationIntent::UserKill);

        // 0.5. 과도기 Exiting 전이 — kill 누르면 즉시 '종료중' 알림. 전이+발행은 core 안에서
        //      이뤄진다(manager가 트리거, core가 status_changed(Exiting) 발행). 이미 terminal이면
        //      false 반환하나 별도 처리 없음(개별 status_changed(Killed)는 pump의 finish 단독).
        let _ = session.enter_exiting();

        // 1~6. 자원 강제 종료 + pump 완료 대기. shutdown이 master를 drop해 pump read를 EOF로
        //       깨우고(→core.finish(Killed)+hook→ReapMsg), join_pump가 그 pump 종료를 5s 대기한다.
        //       timeout이면 그냥 진행(세션 제거로 Arc 끊겨 자연 종료).
        session.kill(Duration::from_secs(5));

        // 7. 세션 추적 해제(S9 — 좀비 watcher 엔트리 방지). 맵 제거·통지는 reaper 가 한다.
        self.tracker.unwatch(agent_id);

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

    /// 단일 에이전트의 현재 epoch 경량 조회(없으면 None). list_agents 전체 순회·AgentInfo
    /// 조립(profiles lock 등)을 피해 epoch 만 본다 — handle_subscribe 의 epoch_matches 계산용.
    pub fn agent_epoch(&self, agent_id: AgentId) -> Option<u32> {
        self.sessions
            .read()
            .expect("sessions poisoned")
            .get(&agent_id)
            .map(|s| s.epoch)
    }

    /// 앱 종료 시 전체 정리. id를 먼저 모아 sessions lock을 풀고, 각 kill을 병렬 실행한다.
    pub fn shutdown_all(&self) {
        // ★ADR-0019★: shutting_down 을 각 kill **전에** set 한다. 이게 kill 보다 늦으면 그 틈에
        //   종료된 세션의 finish hook 이 shutting_down=false 를 snapshot 해 크래시/유저kill 로
        //   오분류(disposition 적용 → 부팅 복원 대상에서 탈락)하는 race 가 생긴다. set 이 먼저면
        //   이 시점 이후 모든 finish 가 shutting_down=true 를 snapshot → reaper 가 KeepAsIs(손 안 댐).
        self.shutting_down.store(true, Ordering::SeqCst);

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
        // name은 ProfileRegistry(단일 진실원)에서 조회한다. get()이 profiles lock을 잡아 clone 후
        // 즉시 해제하므로 sessions lock과 동시에 보유하지 않는다(§10 락 순서, agent_info는 sessions
        // lock 미보유 상태에서만 호출). 프로필이 없으면 id 앞 8자로 fallback.
        let name = self
            .profiles
            .get(session.id)
            .map(|p| p.name)
            .unwrap_or_else(|| {
                let s = session.id.to_string();
                s[..8].to_string()
            });
        AgentInfo {
            id: session.id,
            name,
            cwd: session.cwd.to_string_lossy().to_string(),
            status: session.status(),
            cols: session.cols.load(Ordering::Relaxed),
            rows: session.rows.load(Ordering::Relaxed),
            epoch: session.epoch,
            // transport 종류별 capability — session.capabilities()가 transport.capabilities()를 위임.
            capabilities: session.capabilities(),
        }
    }
}

impl Drop for AgentManager {
    /// reaper 스레드 정리 — Stop 송신 후 join. manager 의 reaper_tx 가 drop 되면 channel 이
    /// 닫혀 recv 가 Err 로도 끝나지만(이중 안전), 세션들이 보유한 hook 클로저가 reaper_tx clone 을
    /// 들고 있어 그것만으로는 즉시 안 닫힐 수 있다. 명시 Stop 으로 확실히 깨운 뒤 join 한다.
    fn drop(&mut self) {
        // Stop 송신(reaper 가 이미 죽었으면 Err — 무시).
        let _ = self.reaper_tx.send(ReaperCmd::Stop);
        if let Some(handle) = self.reaper_handle.take() {
            let _ = handle.join();
        }
    }
}
