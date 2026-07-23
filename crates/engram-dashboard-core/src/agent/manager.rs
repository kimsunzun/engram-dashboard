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

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::agent::backend;
use crate::agent::backend::InputEncoder;
use crate::agent::output_core::OutputCore;
use crate::agent::preset::PresetRegistry;
use crate::agent::profile::{
    AgentProfile, ProfileRegistry, RestoreOutcome, RestoreReport, SpawnMode,
};
use crate::agent::reaper::{self, ReaperCmd, ReaperDeps};
use crate::agent::session::AgentSession;
use crate::agent::session_tracker::SessionTracker;
use crate::agent::transport::pty::PtyTransport;
use crate::agent::transport::stdio::StdioTransport;
use crate::agent::transport::{AgentTransport, OutputDecoder};
use crate::agent::types::{
    AgentId, AgentInfo, AgentStatus, BackendCaps, CommandSpec, ControlChannel, NoopControlChannel,
    OutputChunk, OutputEvent, OutputSink, PtyError, ReapMsg, SinkId, StatusSink, SubscribeOutcome,
    TerminalReason, TerminationIntent,
};

const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

/// resume spawn 후 이 시간 안에 비정상 종료(code≠0/Failed/Killed)하면 resume 실패로 판정한다
/// (H-1.7 "조기 종료 윈도"). 성공한 resume은 TUI라 계속 떠 있다.
/// ★ADR-0082★: 옛날엔 이 신호를 fresh-fallback(새 대화 자동 생성)으로 번역했으나, 이제는
/// **Failed(시체) 종점 + 원인 로그**로 번역한다 — 자동으로 새 대화를 만들지 않는다.
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

/// 모드에 따라 transport를 고른다(ADR-0044 SEAM 1 조립 분기). json 모드 = StdioTransport(파이프,
/// 구조화 출력 캐리어), 그 외(터미널·shell) = PtyTransport(ConPTY, 기존 동작 불변).
///
/// 조립 규칙(양 끝 3지점 중 ①): backend가 mode 보고 CommandSpec 인자를 구성하고, manager는 여기서
/// transport 종류를 고른다. transport 자체는 claude/json을 모른다 — spec만 받아 프로세스를 띄운다.
/// 반환: (박싱된 transport, child_pid). 별도 함수로 뺀 이유 = 실 claude 없이 선택 로직을 단위
/// 테스트하기 위함(ADR-0012 격리 — json→structured caps / 터미널→아님).
///
/// ★조립점 — "mode → 통로가 나르는 것"의 단일 위치(FIX 2, 사용자 요청: 한 곳에 모음)★:
///   transport 종류 선택뿐 아니라 **출력이 구조화(NDJSON)인지도 여기서 결정해 주입**한다. 파이프
///   자체는 내용을 모르므로(통로 무정제 불변) StdioTransport 는 structured 를 하드코딩하지 않고
///   이 지점의 주입값을 받아 caps 로 신고한다. json 모드 = claude `--output-format stream-json` →
///   NDJSON 캐리어 → structured=true. 터미널(PtyTransport)은 그 자체로 terminal-bytes(구조화 아님).
///   출처 분리(output=transport 소유, ADR-0030)는 유지 — 값만 이 조립점에서 주입한다.
// ADR-0044
// ADR-0030
fn select_transport(
    json_mode: bool,
    spec: &CommandSpec,
    cols: u16,
    rows: u16,
    decoder: Option<Box<dyn OutputDecoder>>,
) -> Result<(Box<dyn AgentTransport>, Option<u32>), PtyError> {
    if json_mode {
        // json 모드: PTY 없는 파이프. cols/rows는 파이프에 개념 없어 무시.
        // structured=true 주입 — json 모드가 곧 NDJSON 캐리어라는 mode→caps 매핑(위 조립점 규칙).
        // ★decoder 주입(ADR-0004)★: backend 가 만든 출력 정제기를 통로에 꽂는다 — StdioTransport 는
        //   이게 어떤 디코더인지 모른 채 pump 에서 적용만 한다(통로는 claude 를 모름).
        let (t, pid) = StdioTransport::open(spec, true, decoder)?;
        Ok((Box::new(t), pid))
    } else {
        // 터미널·shell = PtyTransport. decoder 는 여기 경로에선 항상 None(직통) — 방어적으로 무시.
        // (backend::output_decoder 가 json 모드에만 Some 을 주므로 non-json 은 애초에 None 이 온다.)
        let (t, pid) = PtyTransport::open(spec, cols, rows)?;
        Ok((Box::new(t), pid))
    }
}

pub struct AgentManager {
    sessions: Arc<RwLock<HashMap<AgentId, Arc<AgentSession>>>>,
    // C1: Tauri AppHandle이 아니라 StatusSink trait 주입(테스트 시 Noop 가능).
    status_sink: Arc<dyn StatusSink>,
    // S9: 프로필 단일 소유자(sid 생성·갱신·persist) + claude 세션 추적기.
    profiles: Arc<ProfileRegistry>,
    // ADR-0061: 프리셋(cwd 북마크) 단일 소유자. 프로필과 동일하게 데몬이 보유(유저 데이터 단일 소유,
    // ADR-0029)한다. reaper 는 프리셋을 안 보므로(에이전트 수명과 무관) manager 필드로만 둔다.
    presets: Arc<PresetRegistry>,
    tracker: Arc<SessionTracker>,

    // ── ADR-0019 reaper ──────────────────────────────────────
    /// 데몬/앱 셧다운 전역 플래그. shutdown_all 이 각 kill **전에** set 한다 → 그 사이 종료된
    /// 세션의 finish hook 이 true 를 snapshot 해 reaper 가 disposition 을 스킵(부팅 복원 유지).
    shutting_down: Arc<AtomicBool>,
    /// 세션/pump finish hook 이 ReapMsg 를 보내는 채널(단일 supervisor 가 소비).
    reaper_tx: Sender<ReaperCmd>,
    /// reaper 스레드 핸들. Drop 시 join(Stop 송신 후 대기) — 테스트 누수 방지.
    reaper_handle: Option<JoinHandle<()>>,

    /// ADR-0086 제어 채널 provisioning seam. spawn 시 provision(토큰+mcp-config 발급), terminal 시
    /// reaper 가 revoke(폐기+파일 삭제). 데몬만 실제 구현(`DaemonControlChannel`)을 주입하고, 기본은
    /// NoopControlChannel(제어 채널 없음 — headless 테스트·shell-only 경로). Arc 라 reaper 와 공유.
    control: Arc<dyn ControlChannel>,

    /// ADR-0086 provision 레이스 가드(FIX 6) — 현재 spawn 진행 중인 AgentId 예약 집합. contains_key
    /// 가드(read lock)와 실제 sessions.insert(write lock) 사이의 TOCTOU 창에서 **다른 연결**이 같은
    /// AgentId 를 동시에 spawn 하면, 둘 다 provision 을 불러 같은 (AgentId,epoch) config 경로에 쓰고
    /// 한쪽 reaper 가 상대 산 세션을 오삭제할 수 있다. 진입 시 이 집합에 원자적으로 예약(이미 있으면 즉시
    /// Err)해 두 번째 동시 spawn 을 깨끗이 거부한다. 예약은 성공(등록 완료)·실패(어느 조기 반환)든
    /// SpawnReservation(RAII)이 drop 시 해제한다. ★sessions 맵과 별개 leaf lock★: 이 Mutex 보유 중
    /// sessions/status 락을 잡지 않는다(ADR-0006 — 짧은 임계구역, 순수 HashSet 조작).
    spawning: Arc<Mutex<HashSet<AgentId>>>,
}

/// spawn 진행 중 AgentId 예약을 잡고, drop 시 자동 해제하는 RAII 가드(ADR-0086 FIX 6). spawn_agent
/// 의 어느 조기 반환(provision 실패·PTY 실패·`?`)에서도 예약이 새지 않게 한다. `reserve` 가 이미 예약된
/// id 면 None(두 번째 동시 spawn 거부).
struct SpawnReservation {
    spawning: Arc<Mutex<HashSet<AgentId>>>,
    id: AgentId,
}

impl SpawnReservation {
    /// (AgentId) 예약 시도. 이미 다른 spawn 이 예약 중이면 None. 성공 시 가드 반환(drop 에 해제).
    fn reserve(spawning: Arc<Mutex<HashSet<AgentId>>>, id: AgentId) -> Option<Self> {
        {
            let mut set = spawning.lock().expect("spawning set poisoned");
            if !set.insert(id) {
                return None; // 이미 진행 중 — 두 번째 동시 spawn 거부.
            }
        }
        Some(Self { spawning, id })
    }
}

impl Drop for SpawnReservation {
    fn drop(&mut self) {
        // 예약 해제(성공·실패 무관). 없어도 무해(remove 는 없으면 false).
        let _ = self
            .spawning
            .lock()
            .expect("spawning set poisoned")
            .remove(&self.id);
    }
}

/// provision 성공 후 세션 등록 **전에** 실패(exe/PTY 오류·`?` 조기 반환)하면 발급된 토큰+config
/// 파일이 영원히 샌다(세션이 없어 reaper 가 영영 revoke 안 함) — 이를 막는 RAII 가드(ADR-0086 FIX 3).
/// provision 이 실제 endpoint 를 돌려줬을 때만 arm 되고, 세션 등록이 끝나면 `disarm()` 으로 무장 해제한다.
/// drop 시 아직 armed 면 revoke(폐기+파일 삭제)를 부른다 — 모든 pre-registration 실패 경로를 커버한다.
///
/// ★lock 미보유(ADR-0006)★: drop 은 sessions/status 락을 잡지 않는 지점(spawn_agent 조기 반환)에서만
///   일어나므로 revoke(registry leaf lock + 파일 IO)가 락 순서를 깨지 않는다.
struct ProvisionGuard {
    control: Arc<dyn ControlChannel>,
    id: AgentId,
    epoch: u32,
    /// true 인 동안 drop 하면 revoke. 세션 등록 성공 시 disarm() 이 false 로 내려 revoke 를 막는다
    /// (등록된 세션의 revoke 는 이제 kill_agent/reaper 소관 — 이중 revoke 방지, 정상 수명으로 이관).
    armed: bool,
}

impl ProvisionGuard {
    /// 세션 등록 완료 후 호출 — 무장 해제(정상 수명으로 이관). 이후 drop 은 revoke 하지 않는다.
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ProvisionGuard {
    fn drop(&mut self) {
        if self.armed {
            // 세션 등록 전 실패 — 새는 토큰+config 를 회수한다(revoke 는 idempotent).
            tracing::warn!(
                agent = %self.id,
                epoch = self.epoch,
                "ADR-0086: spawn 실패(세션 등록 전) — 발급된 제어 채널 토큰/config 회수(revoke)"
            );
            self.control.revoke(self.id, self.epoch);
        }
    }
}

impl AgentManager {
    /// 기본 생성자 — 제어 채널 없음(NoopControlChannel). headless 테스트·제어 채널 미사용 경로.
    pub fn new(
        status_sink: Arc<dyn StatusSink>,
        profiles: Arc<ProfileRegistry>,
        presets: Arc<PresetRegistry>,
        tracker: Arc<SessionTracker>,
    ) -> Self {
        Self::new_with_control(
            status_sink,
            profiles,
            presets,
            tracker,
            Arc::new(NoopControlChannel),
        )
    }

    /// 제어 채널 주입형(ADR-0086) — 데몬이 `DaemonControlChannel` 을 끼운다. reaper 도 같은 Arc 를
    /// 공유해 terminal 수렴 지점에서 revoke 한다(spawn=provision / terminal=revoke 인과 대칭).
    pub fn new_with_control(
        status_sink: Arc<dyn StatusSink>,
        profiles: Arc<ProfileRegistry>,
        presets: Arc<PresetRegistry>,
        tracker: Arc<SessionTracker>,
        control: Arc<dyn ControlChannel>,
    ) -> Self {
        let sessions = Arc::new(RwLock::new(HashMap::new()));

        // reaper supervisor 1개 기동 — manager 와 동일한 sessions/profiles/status_sink 를 공유한다
        // (두 주체가 같은 모델을 본다). reap_one 이 lock 밖에서 disposition·통지를 수행한다.
        // ★control 도 공유(ADR-0086)★: reaper 가 terminal(단일 소비자) 시 revoke 를 부른다.
        let deps = ReaperDeps {
            sessions: sessions.clone(),
            profiles: profiles.clone(),
            status_sink: status_sink.clone(),
            control: control.clone(),
        };
        let (reaper_tx, reaper_handle) = reaper::spawn_reaper(deps);

        Self {
            sessions,
            status_sink,
            profiles,
            presets,
            tracker,
            shutting_down: Arc::new(AtomicBool::new(false)),
            reaper_tx,
            reaper_handle: Some(reaper_handle),
            control,
            spawning: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// 프로필 레지스트리 접근(commands에서 CRUD에 사용).
    pub fn profiles(&self) -> &Arc<ProfileRegistry> {
        &self.profiles
    }

    /// 프리셋 레지스트리 접근(connection_core 의 프리셋 CRUD 에 사용, ADR-0061). profiles() 와 동형.
    pub fn presets(&self) -> &Arc<PresetRegistry> {
        &self.presets
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
        // ★ADR-0082★: 이 Err 는 순수 방어선일 뿐 파괴 트리거가 아니다. activate_profile 이 진입 시
        //   contains_key 를 **선제로** 검사해 산 에이전트면 여기 닿기 전에 무해한 재활성화로 처리한다
        //   (옛날엔 이 Err 가 resume_with_fresh_fallback 에 의해 "resume 실패"로 오인돼 산 에이전트를
        //   kill 하는 a4aac1a 회귀를 낳았다 — 이제 그 오인 경로 자체가 없다).
        // ★잔여 레이스(ADR-0082 미해결·후속)★: 이 가드는 여기서 read lock 을 잡아 contains_key 를 본 뒤
        //   놓고, 실제 등록(sessions.insert)은 아래에서 별개 write lock 으로 한다 — 그 사이 창이 있다.
        //   같은 id 를 **서로 다른 연결**이 동시에 SpawnProfile 하면 둘 다 이 검사와 activate_profile 의
        //   pre-check 를 통과해 double-spawn 이 날 수 있다(데몬 명령 처리는 연결당 직렬일 뿐 연결 간엔
        //   아니다 — 각 연결이 제 read_task 에서 dispatch 를 await 한다). 이 window 는 ADR-0082 이전부터
        //   있던 **선재(pre-existing) 레이스**이며 이번 변경이 도입하지도 닫지도 않았다(후속 과제로 flag).
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

        // ★provision 레이스 가드(FIX 6)★: 위 contains_key(read lock)와 아래 sessions.insert(write lock)
        //   사이의 TOCTOU 창에서 **다른 연결**이 같은 AgentId 를 동시에 spawn 하면, 둘 다 provision 을
        //   불러 같은 (AgentId,epoch) config 경로에 쓰고 한쪽 reaper 가 상대 산 세션을 오삭제할 수 있다.
        //   진입 즉시 (AgentId) 를 원자적으로 예약해 두 번째 동시 spawn 을 깨끗이 거부한다. 예약은 아래
        //   어느 조기 반환(provision 실패·PTY 실패)에서도 SpawnReservation drop 이 해제한다(RAII).
        //   ★leaf lock(ADR-0006)★: spawning Mutex 는 짧게 잡고 sessions/status 락과 겹치지 않는다.
        let _reservation = SpawnReservation::reserve(self.spawning.clone(), profile.id)
            .ok_or_else(|| {
                PtyError::SpawnFailed(format!(
                    "agent {} spawn already in progress (concurrent spawn rejected)",
                    profile.id
                ))
            })?;

        // 프로필을 레지스트리에 등록(idempotent + 즉시 persist). 복원 경로는 기존 프로필을 그대로 넘긴다.
        // ★hierarchy-preserving★: profile 은 SpawnProfile 등에서 뜬 **스냅샷**이라, spawn 사이 다른 연결이
        // reparent/rename 한 최신 parent_id/display_name 을 덮어쓰면 안 된다(lost update). 그 두 트리 메타는
        // live 엔트리 값을 보존하고 나머지(cwd/command/env/session)만 반영한다(ADR-0070/0072).
        self.profiles.upsert_preserving_hierarchy(profile.clone());

        // cwd 정규화 — claude 세션 디렉토리 표기 고정(UNC 회피). 실패 시 원본 사용(best-effort).
        let cwd = dunce::canonicalize(&profile.cwd).unwrap_or_else(|_| profile.cwd.clone());

        // backend가 세션 추적 대상인지 판단(claude=true, shell=false). true면 세션 id 확보.
        // 생성 책임은 ProfileRegistry(H-1.4).
        //
        // ★mode 별 sid 발급 규칙(ADR-0076 — "activate=resume, fresh=new sid" 봉인)★:
        //   - Resume: 저장된 sid 를 그대로 써야 기존 대화를 이어받는다 → ensure_session_id(있으면 그대로,
        //     드물게 없으면 최초 발급). backend 가 `--resume <sid>` 로 무손실 복원(ADR-0008).
        //   - Fresh: **반드시 새 sid**. ensure_session_id 를 쓰면 저장된 sid 를 재사용해
        //     `--session-id <저장 sid>` 로 떠 디스크 세션과 충돌한다("Session ID already in use" → claude
        //     즉사, 이 세션의 재현 버그). new_session_id 가 항상 새 uuid 를 발급(옛 sid 는 이력 보존).
        //   spawn_agent 이 이 판정의 단일 권위점이라 어떤 호출자(Spawn/SpawnProfile/restore/fallback)든
        //   mode 만 맞게 넘기면 sid 충돌이 원천 봉인된다(FIX 2 backend-authoritative).
        let needs = backend::needs_session(&profile.command);
        let sid = if needs {
            match mode {
                SpawnMode::Resume => self.profiles.ensure_session_id(profile.id),
                SpawnMode::Fresh => self.profiles.new_session_id(profile.id),
            }
        } else {
            None
        };

        // epoch는 레지스트리의 현재값(fallback respawn 등에서 미리 bump됨).
        let epoch = self.profiles.get(profile.id).map(|p| p.epoch).unwrap_or(0);

        // ADR-0086: 제어 채널 provisioning. 데몬이 (AgentId,epoch)용 토큰+mcp-config 를 발급해
        //   ControlEndpoint 를 돌려준다. ★spec 조립 직전에 부른다★ — build_command_spec 이 endpoint 를
        //   받아 backend 방식(claude=`--mcp-config`, ADR-0004)으로 명령줄에 주입해야 하므로. epoch 는
        //   위에서 확정된 현재값이라 재활성화(bump) 때마다 새 토큰이 발급된다(토큰 수명=(AgentId,epoch)).
        //
        // ★backend-conditional(round-2 F3)★: 제어 채널을 **소비하는** backend(claude)에만 provision 을
        //   부른다 — shell 은 supports_control_channel=false 라 provision 을 아예 건드리지 않는다(registry
        //   미접촉). 이렇게 하면 config-write 실패가 MCP 가 필요 없던 셸 스폰을 중단시키는 회귀가 없다.
        //   판정은 backend dispatch(ADR-0004) — manager 가 command 를 직접 matches! 하지 않는다.
        // ★fail-closed(FIX 2)★: provision 을 **부르는** backend 에서 provision 3-값(Ok(Some)/Ok(None)/
        //   Err) 중 Err(CSPRNG/파일 write 실패)면 **스폰을 중단**한다(제어 채널 없이 몰래 도는 에이전트
        //   금지 — health 위장 방지). Ok(None)=제어 채널을 안 쓰는 정당한 부재(Noop)라 그대로 진행.
        //   Ok(Some)=발급 성공 → 아래 ProvisionGuard 로 arm 해, 세션 등록 전 어느 실패에서든 발급된
        //   토큰/config 를 회수한다(FIX 3 leak 방지). supports_control_channel=false 인 backend 는 provision
        //   을 건너뛰므로 None(부재)과 동일하게 흐른다 — 그 backend 엔 fail-closed 계약이 적용되지 않는다.
        // ADR-0086
        let control_endpoint = if backend::supports_control_channel(&profile.command) {
            // ADR-0099: backend 의 MCP-capability 를 provision 에 넘겨 채널 물리 배선·프라이밍 변형·grant 를
            //   한꺼번에 가르게 한다(정합 불변식 = 깐 채널 == 프라이밍이 가르치는 채널). 판정은 backend
            //   dispatch(ADR-0004) — manager 는 command 를 직접 matches! 하지 않는다.
            // ADR-0099
            let accepts_mcp = backend::accepts_mcp_config(&profile.command);
            self.control
                .provision(profile.id, epoch, accepts_mcp)
                .map_err(|e| {
                    PtyError::SpawnFailed(format!(
                        "control channel provision failed (fail-closed): {e}"
                    ))
                })?
        } else {
            // 제어 채널 미소비 backend(shell): provision 미호출 → registry 미접촉 → endpoint 없음.
            None
        };
        // provision 이 실제 endpoint 를 줬으면 회수 가드를 arm(세션 등록 성공 시 disarm). None(부재)이면
        //   회수할 게 없어 arm 하지 않는다.
        let mut provision_guard = control_endpoint.as_ref().map(|_| ProvisionGuard {
            control: self.control.clone(),
            id: profile.id,
            epoch,
            armed: true,
        });

        // backend가 program/args/env/cwd를 중립 CommandSpec으로 산출. transport는 claude/shell을 모른다.
        // control_endpoint(추상 descriptor)를 함께 넘긴다 — backend 가 자기 프로그램 방식으로 주입한다.
        let spec = backend::build_command_spec(
            &profile.command,
            mode,
            sid,
            cwd.clone(),
            profile.env.clone(),
            control_endpoint,
        );

        // backend(프로그램)가 결정하는 caps(session/model)를 spec과 별도로 산출해 흘린다.
        // spec은 backend-neutral(program/args뿐)이라 caps를 spec에 싣지 않고 따로 전달한다 —
        // session이 transport caps와 compose 한다(claude=resume true, shell=resume false 정확화).
        let bcaps = backend::backend_caps(&profile.command);

        // ADR-0044 조립 분기(양 끝 3지점): json 모드면 StdioTransport 선택 + 입력을 claude 유저 JSON
        // 라인으로 감싸는 encoder. 그 외는 PtyTransport + Raw(터미널 경로 바이트 불변). 판정은
        // 프로필 command 단일 출처(is_json_mode/input_encoder) — spawn_session은 backend를 모른다.
        let json_mode = profile.command.is_json_mode();
        let encoder = backend::input_encoder(&profile.command);
        // 출력 정제기(입력 encoder 의 대칭 짝) — json 모드면 backend 가 claude decoder 를 만들고,
        // 그 외엔 None(바이트 직통). claude 스키마 지식은 backend 단독이라 여기선 command 만 넘긴다.
        let decoder = backend::output_decoder(&profile.command);

        // ADR-0079: resume(=과거 대화 이어받기) 스폰이면 `.jsonl` transcript 에서 과거 이벤트를 읽어
        //   버퍼에 seed 한다(pump 전). Fresh 는 이어받을 대화가 없으므로 빈 Vec(기존 동작 불변). json
        //   모드 claude 만 실제로 읽고(터미널은 TUI PTY repaint 로 복원, shell 은 대화 없음), 그 외엔
        //   backend dispatch 가 빈 Vec 을 돌려준다. transcript 경로·파싱 지식은 backend 단독(ADR-0004).
        let seed_events = match mode {
            SpawnMode::Resume => match sid {
                Some(s) => backend::resume_transcript_events(&profile.command, &cwd, s),
                None => Vec::new(),
            },
            SpawnMode::Fresh => Vec::new(),
        };

        let (session, child_pid) = self.spawn_session(
            profile.id,
            spec,
            bcaps,
            encoder,
            decoder,
            json_mode,
            epoch,
            seed_events,
        )?;

        // ★provision 가드 무장 해제(FIX 3)★: 여기 도달 = spawn_session 이 sessions 맵에 세션을 등록 완료.
        //   이제 이 토큰/config 의 수명은 세션에 붙어(kill_agent 선제 revoke + reaper terminal revoke 가
        //   책임진다) — 가드가 이중 revoke 하지 않게 무장 해제한다. 이 줄 위의 어느 `?` 조기 반환이든
        //   가드가 armed 인 채 drop 돼 revoke 가 발급 자원을 회수한다.
        if let Some(g) = provision_guard.as_mut() {
            g.disarm();
        }

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

    /// ★수동 활성화 진입점 — 이어받기(resume) 전용, fresh-fallback 폐지(ADR-0082)★.
    /// SpawnProfile 핸들러가 `spawn_agent` 대신 이걸 부른다. 세 갈래로 나뉜다:
    ///
    /// 1. **이미 실행 중(재활성화 가드)** — 같은 id 세션이 살아 있으면 **아무것도 죽이거나
    ///    재spawn 하지 않고** 그 세션의 AgentInfo 를 그대로 돌려준다(무해한 "이미 실행 중" 신호,
    ///    epoch 불변). ★이게 a4aac1a 회귀의 핵심 수정★: 예전엔 이 경로가 `spawn_agent` 이중-spawn
    ///    가드의 "already running" Err 를 만나 `resume_with_fresh_fallback` 이 그걸 "resume 실패"로
    ///    오인 → `fallback_fresh` 가 **멀쩡히 돌던 산 에이전트를 kill** → epoch++ → 빈 fresh 로 교체
    ///    (유저 실측 회귀). 이제 가드 Err 에 닿기 전에 선제 contains_key 로 걸러 산 에이전트를 놔둔다.
    ///    (이 pre-check 는 흔한 경로 — **같은 연결**에서 직렬로 들어오는 재활성화 — 를 닫는다. spawn_agent
    ///    의 이중-spawn 가드는 최후 방어선으로 남지만, pre-check 와 실제 spawn 사이의 TOCTOU 를 완전히
    ///    닫지는 못한다: **다른 연결**이 같은 id 를 동시에 활성화하면 둘 다 pre-check 와 contains_key 를
    ///    통과해 double-spawn 이 날 수 있다(데몬 명령 처리는 연결당 직렬일 뿐 연결 간엔 아님). 이 레이스는
    ///    ADR-0082 이전부터 있던 선재(pre-existing) window 로, 이번 변경이 닫지 않는다 — 후속 과제.)
    /// 2. **Fresh(진짜 신규 — 세션 없음)** — `spawn_agent(Fresh)` 위임(이어받을 대화 없음, 기존 동작
    ///    보존). 이건 실패-fallback 이 아니라 정상 신규 생성이다(ADR-0076 "Fresh=새 sid" 유효).
    /// 3. **Resume** — `resume_no_fallback` 로 이어받기만 시도한다. 이어받을 수 없으면(빈/미대화/손상 —
    ///    claude 가 "No conversation found ..." 로 즉사) **새 대화를 만들지 않고** Failed(시체)로
    ///    남기고 사유를 로그로 남긴다(ADR-0082 — 원인은 LLM 이 읽어 에스컬레이션). 여기선 Err 로 노출.
    ///
    /// ★blocking★: Resume 모드는 EARLY_EXIT_WINDOW(현 3s)만큼 조기종료를 폴링하므로 호출이 그만큼
    ///   블록될 수 있다(restore_all 과 동일 성질). 데몬의 명령 처리 스레드에서 호출되므로 그 연결의
    ///   응답만 지연되고 다른 세션에는 영향 없다. Fresh 모드·재활성화 가드는 폴링 없이 즉시 반환한다.
    // ADR-0082
    // ADR-0076
    pub fn activate_profile(
        &self,
        profile: &AgentProfile,
        mode: SpawnMode,
    ) -> Result<AgentInfo, PtyError> {
        // 1. ★재활성화 가드(ADR-0082) — 산 에이전트를 절대 건드리지 않는다★. 같은 id 세션이 이미
        //    살아 있으면 kill/재spawn/epoch-bump 없이 현재 세션의 AgentInfo 를 무해하게 돌려준다.
        //    이중-spawn 가드 Err 가 파괴 트리거(옛 fresh-fallback)로 번역되던 회귀를 원천 차단한다.
        //    (read lock 은 clone 후 즉시 해제 — §10 락 순서 준수, agent_info 는 lock 미보유로 호출.)
        if let Ok(session) = self.get_session(profile.id) {
            tracing::info!(
                agent = %profile.id,
                "activate_profile: 이미 실행 중 — 재활성화 무시(산 에이전트 보존, ADR-0082)"
            );
            return Ok(self.agent_info(&session));
        }

        // 2. Fresh(진짜 신규 — 세션 없음)는 이어받을 대화가 없으므로 spawn_agent 위임(정상 신규 생성).
        if mode == SpawnMode::Fresh {
            return self.spawn_agent(profile, SpawnMode::Fresh);
        }

        // 3. Resume: 이어받기만 시도(fresh-fallback 폐지). resume_no_fallback 이 RestoreOutcome 을
        //    돌려주므로 결말을 AgentInfo/Err 로 번역한다.
        //
        // ★재활성화 = epoch++★: 여기 도달했다는 건 위 가드에서 산 세션이 **없음**을 이미 확인했다는
        //   뜻이다 — 즉 reap 으로 세션이 맵에서 빠진 **시체**를 같은 AgentId 로 다시 띄우는 맵 교체다.
        //   ADR-0007 불변식("같은 AgentId 맵 교체마다 epoch +1")을 그대로 적용해, 새 세션이 죽은
        //   세션과 다른 `[agentId, epoch]` 를 갖게 한다 → 프론트 구독(deps [viewId,agentId,epoch])이
        //   재발화해 resume 출력이 화면에 붙고, 옛 seq/cursor 가 새 스트림에 오적용되지 않는다.
        //   spawn_agent(L223)이 이 bump **뒤** 프로필 epoch 를 읽으므로 순서가 load-bearing 이다.
        //   (산 세션 재활성화는 위 가드에서 이미 걸러졌으므로 절대 여기 오지 않는다 — bump 안전.)
        //   또 이 bump 는 stale reap 의 apply_disposition epoch-guard(reaper.rs)가 재활성화된 산
        //   세션을 강등 못 하게 하는 구분자이기도 하다.
        // ADR-0084
        // ADR-0007
        self.profiles.bump_epoch(profile.id);

        match self.resume_no_fallback(profile) {
            // resume 성공 — 살아있는 세션의 info 반환.
            RestoreOutcome::Resumed => self.agent_info_by_id(profile.id),
            // resume 실패/조기종료 → 종점 Failed(시체). 새 대화 안 만듦. 호출자(핸들러)엔 Err 로 노출.
            RestoreOutcome::Failed { reason } => Err(PtyError::SpawnFailed(reason)),
            // resumable 프로필로만 진입하므로 Started/Blocked/FreshFallback 은 도달 불가(방어적 Err).
            other => Err(PtyError::SpawnFailed(format!(
                "activate_profile: 예상 밖 결말 {other:?}"
            ))),
        }
    }

    /// PtyTransport open + OutputCore 생성 + pump 기동(transport.start) + AgentSession 합성 +
    /// sessions 등록의 공통 기계부. 반환: 등록된 세션 Arc + child PID(Option).
    #[allow(clippy::too_many_arguments)]
    fn spawn_session(
        &self,
        id: AgentId,
        spec: CommandSpec,
        backend_caps: BackendCaps,
        encoder: InputEncoder,
        decoder: Option<Box<dyn OutputDecoder>>,
        json_mode: bool,
        epoch: u32,
        // ADR-0079: resume 시 `.jsonl` 에서 복원한 과거 이벤트. pump 전에 core 버퍼에 seed 한다.
        //   Fresh(및 비-json)는 빈 Vec → seed 안 함(기존 fresh 버퍼 동작 불변).
        seed_events: Vec<OutputEvent>,
    ) -> Result<(Arc<AgentSession>, Option<u32>), PtyError> {
        // 1. 모드에 맞는 transport 조립(json=StdioTransport 파이프 / 그 외=PtyTransport ConPTY).
        //    child spawn + job 편입 + 파이프/reader·writer 확보. pump는 아직 안 띄움(start에서).
        //    json 모드면 출력 정제 decoder 도 함께 통로에 주입한다(ADR-0004 — 통로는 claude 모름).
        let (transport, child_pid) =
            select_transport(json_mode, &spec, DEFAULT_COLS, DEFAULT_ROWS, decoder)?;

        // 2. 출력 측 core 생성(status Running, seq 0). transport와 분리된 출력 fanout 담당.
        let core = Arc::new(OutputCore::new(id, epoch, self.status_sink.clone()));

        // 2.1. ★ADR-0079 seed-before-publish(load-bearing 순서 — cross-family review 2026-07-13)★:
        //      resume 복원 과거 이벤트를 **세션이 관측 가능해지기 전에**(= sessions 맵 insert 전) core
        //      Ring 에 seed 한다. 지금 core 는 이 함수 로컬 Arc 뿐이라 다른 스레드가 닿을 수 없다(구독·emit
        //      경로 모두 sessions 맵 조회를 거친다). 그래서 seed 를 여기서 끝내면 다음 두 윈도가 원천 차단된다:
        //        (a) empty-ring replay: insert 후 seed 전에 재접속 구독이 끼면 빈 Ring 을 replay 하고
        //            seed 는 fanout 안 하므로 과거를 영구 유실 → insert 전 seed 로 제거.
        //        (b) seq interleave: 그 윈도의 동시 emit/write 가 seed 와 seq 를 뒤섞어 Ring 순서를
        //            [0,2,1] 로 깨 replay 의 partition_point 전제를 위반 → seed 선행으로 제거.
        //      seed 는 여전히 start_pump 전이다(라이브 emit 은 pump 가 켜야 시작). seed_events 가 비면
        //      (Fresh·비-json·transcript 부재) no-op → 기존 fresh 버퍼 동작 불변.
        if !seed_events.is_empty() {
            tracing::info!(
                agent = %id,
                epoch,
                count = seed_events.len(),
                "ADR-0079: resume transcript seed (before publish)"
            );
            core.seed(seed_events);
        }

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

        // 3. transport는 select_transport가 이미 Box<dyn AgentTransport>로 박싱해 반환한다.

        // 4. core + transport를 AgentSession으로 합성(cols/rows atomic은 session 보유).
        //    encoder(입력 인코딩 태그)도 함께 주입 — write_input이 transport로 넘기기 전 적용.
        let session = Arc::new(AgentSession::new(
            id,
            spec.cwd.clone(),
            epoch,
            DEFAULT_COLS,
            DEFAULT_ROWS,
            intent,
            backend_caps,
            encoder,
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

    /// 프로필 1개 복원. claude+sid 있으면 resume 시도(실패 시 Failed 시체, fresh-fallback 폐지),
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

        // claude resume 시도 → 실패/조기종료면 Failed(시체) 종점(fresh-fallback 폐지, ADR-0082).
        self.resume_no_fallback(profile)
    }

    /// ★resume 전용 공용 규율(ADR-0082 — 부팅복원·수동활성화 공유, fresh-fallback 폐지)★.
    /// 전제: 호출 시점에 이 프로필은 resumable(claude + sid 존재)이라고 이미 판정됐다.
    ///
    /// resume 을 시도하고, spawn 실패거나 EARLY_EXIT_WINDOW 안에 비정상 종료(빈/미대화/손상
    /// 세션이면 claude 가 "No conversation found ..." 로 즉사)하면 **새 대화(fresh)를 자동으로
    /// 만들지 않고** Failed(시체) 종점으로 직행한다 — 사유를 로그로 남겨 LLM 이 읽고 에스컬레이션한다.
    /// ★아무것도 kill·재spawn 하지 않는다★: resume child 는 자기 pump 가 EOF→finish 하고, reaper 가
    ///   그 세션을 맵에서 수거하며 프로필을 `auto_restore=false`(KeepDisableAutoRestore)로 내려
    ///   트리에 `Failed` 시체로 남긴다(profile 은 지워지지 않음 — exit≠0/불명은 삭제 대상이 아님).
    ///   이 헬퍼는 종료를 관측만 하고 어떤 파괴 동작도 하지 않는다(옛 fallback_fresh 의 remove_session·
    ///   epoch++·respawn 을 전부 걷어냈다 — ADR-0082 사용자 결정: "아무것도 죽지마, 새로 만들지마").
    /// 이 로직을 restore_one(부팅 복원)과 activate_profile(수동 활성화)이 **똑같이** 재사용한다.
    // ADR-0082
    // ADR-0008
    fn resume_no_fallback(&self, profile: &AgentProfile) -> RestoreOutcome {
        match self.spawn_agent(profile, SpawnMode::Resume) {
            Err(e) => {
                // resume spawn 자체 실패 — 원인을 로그로 남긴다(삼키면 §5 위반). 새 대화 안 만듦.
                let reason = format!("resume spawn 실패: {e}");
                tracing::warn!(
                    agent = %profile.id,
                    %reason,
                    "ADR-0082: resume 실패 → Failed(시체), fresh-fallback 없음"
                );
                RestoreOutcome::Failed { reason }
            }
            // ★fable M-1★: 성공한 claude resume은 TUI라 윈도 안에 종료하지 않는다.
            // 따라서 윈도 내 terminal 진입은 code와 무관하게 resume 실패 신호다
            // (code==0 조기 종료를 Resumed로 오판하면 빈 화면을 "복원 성공"으로 오보).
            // None(여전히 Running)만 Resumed.
            Ok(_) => match self.early_terminal_status(profile.id, EARLY_EXIT_WINDOW) {
                Some(status) => {
                    // resume 조기종료 = 이어받을 수 없는 세션(claude "No conversation found ...").
                    // ★원인 로그(제어 표면 입력)★: LLM 에이전트가 이 로그를 읽어 사용자에게
                    //   에스컬레이션한다(ADR-0082 §5). 자동 fresh 대체 없음 — Failed 시체로 남긴다.
                    //   세션은 이미 스스로 종료했으므로 여기서 remove/kill 하지 않는다(reaper 가 수거).
                    let reason = format!("resume 조기 종료({status:?})");
                    tracing::warn!(
                        agent = %profile.id,
                        %reason,
                        "ADR-0082: resume 조기종료 → Failed(시체), fresh-fallback 없음 — LLM 에스컬레이션 대상"
                    );
                    RestoreOutcome::Failed { reason }
                }
                None => RestoreOutcome::Resumed,
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

    // (옛 remove_session 삭제 — ADR-0082 fresh-fallback 폐지로 유일 호출자 fallback_fresh 가
    //  사라져 dead code 가 됐다. "옛 세션 kill 후 fresh 로 교체" 자체가 폐지된 동작이라 이 silent
    //  cleanup 헬퍼도 함께 제거한다. 정식 kill 은 kill_agent(reaper 위임)가 담당한다.)

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

    /// `write_stdin` 의 배달-경계 계측판(ADR-0088 Stage 0) — 성공 시 `WriteOutcome`(논리 메시지 바이트 +
    ///   이 턴의 `msg_uuid`)을 반환한다. 동작은 `write_stdin` 과 동일하고 관측 산출물만 삼키지 않는다.
    ///   제어 채널 relay(ingress::handle_send)가 배달 관측 레코드를 만들 때 쓴다("전송 실패" vs
    ///   "모델 무시" 구별의 전제 — ADR-0088). 완결성 = Ok-vs-Err(바이트 비교 아님)은 `WriteOutcome` 주석 참조.
    pub fn write_stdin_observed(
        &self,
        agent_id: AgentId,
        data: &[u8],
    ) -> Result<crate::agent::types::WriteOutcome, PtyError> {
        self.get_session(agent_id)?.write_input_observed(data)
    }

    /// ★하네스 전용 세션 주입 seam(ADR-0088 / ADR-0012)★ — 미리 조립한 `AgentSession`(테스트 transport
    ///   포함)을 sessions 맵에 직접 등록한다. spawn 파이프(실 PTY·claude 바이너리)를 거치지 않고
    ///   배달-경계 관측 테스트(reachable=structured 캐리어인데 write 성공/실패)를 **바이너리 의존 없이**
    ///   구동하려는 목적이다 — daemon 통합 테스트가 cross-crate 로 봐야 하므로 `test-harness` 기능으로
    ///   게이트한다(`#[doc(hidden)]` 은 접근을 막지 못한다 — 임의 `AgentSession` 주입은 spawn 예약·
    ///   profile+epoch 조율·control-token 발급·pump/reaper 배선·tracker 수명을 통째로 우회하므로,
    ///   운영 빌드에는 아예 컴파일되지 않아야 한다). 기능 OFF = 운영 빌드에 이 메서드 부재. 기능은
    ///   daemon 의 `[dev-dependencies]` 에서만 켜지므로(운영 dep 아님) 운영 daemon 바이너리로 유니피케이션
    ///   되지 않는다. 런타임/운영 경로는 절대 부르지 않는다 — spawn_session 만이 정규 등록점.
    ///
    /// ★안전/불변식★: (a) reaper 미배선 — 주입 세션은 pump 를 start 하지 않으므로 finish hook 이 없고,
    ///   ReapMsg 가 나가지 않아 manager Drop 까지 sessions 맵에 남는다(노출된 remove 없음, `kill_agent` 도
    ///   주입 세션을 맵에서 빼지 않는다 — 각 테스트가 fresh manager 를 쓰고 그 Drop 으로 정리된다).
    ///   (b) 락 규율(ADR-0006) — sessions write lock 을 잡아 insert 후 즉시 해제, 내부 lock 미취득.
    ///   (c) profiles 미터치 — auto_restore 플립·persist 없음(순수 맵 등록). 같은 id 재주입은 교체.
    #[cfg(feature = "test-harness")]
    #[doc(hidden)]
    pub fn insert_test_session(&self, session: Arc<AgentSession>) {
        self.sessions
            .write()
            .expect("sessions poisoned")
            .insert(session.id, session);
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
        // 대상 세션 epoch 을 Arc clone 직후(락 해제 상태) 확정한다 — revoke 대상 (AgentId,epoch).
        let epoch = session.epoch;

        // 0. ★제어 채널 토큰 즉시 폐기 — 블로킹 kill **전에**(FIX 4)★. get_session 이 Arc 를 clone 하고
        //    sessions read lock 을 이미 해제했으므로(§10), 여기서 revoke 를 불러도 락 보유 중이 아니다
        //    (ADR-0006 — registry 는 leaf lock, sessions/status 락 미보유). 예전엔 이 revoke 가
        //    session.kill(최대 5s join) **뒤**라, 죽어가는 에이전트의 토큰이 그 5s 창 동안 유효했다 —
        //    그 사이 에이전트가 제어 채널로 명령을 낼 수 있었다(TOCTOU). 이제 kill 을 시작하기 전에 먼저
        //    폐기해 그 창을 없앤다. revoke 는 idempotent(remove-if-present)라 아래 pump/reaper 의
        //    terminal revoke 와 겹쳐도 무해(그게 backstop). 산 세션이므로 이 epoch 토큰이 지금 폐기 대상.
        // ADR-0086
        self.control.revoke(agent_id, epoch);

        // 0.1. ★intent 태깅을 shutdown 전에★ — finish hook 이 finish 순간 snapshot 하므로, shutdown
        //    이 pump 를 깨워 finish 하기 전에 UserKill 이 보여야 reaper 가 DeleteProfile 로 분류한다.
        session.set_intent(TerminationIntent::UserKill);

        // 0.5. 과도기 Exiting 전이 — kill 누르면 즉시 '종료중' 알림. 전이+발행은 core 안에서
        //      이뤄진다(manager가 트리거, core가 status_changed(Exiting) 발행). 이미 terminal이면
        //      false 반환하나 별도 처리 없음(개별 status_changed(Killed)는 pump의 finish 단독).
        let _ = session.enter_exiting();

        // 1~6. 자원 강제 종료 + pump 완료 대기. shutdown이 master를 drop해 pump read를 EOF로
        //       깨우고(→core.finish(Killed)+hook→ReapMsg), join_pump가 그 pump 종료를 5s 대기한다.
        //       timeout이면 그냥 진행(세션 제거로 Arc 끊겨 자연 종료). ★revoke 배치가 이 인과를 건드리지
        //       않는다(ADR-0001)★: revoke 는 registry/파일만 만지고 shutdown 체인(child.kill→master
        //       drop→pump EOF→finish)에 개입하지 않는다 — kill 을 블록/재정렬하지 않는다.
        session.kill(Duration::from_secs(5));

        // 7. 세션 추적 해제(S9 — 좀비 watcher 엔트리 방지). 맵 제거·통지는 reaper 가 한다.
        //    (제어 채널 revoke 는 위 0단계에서 선제 완료 — reaper terminal revoke 가 idempotent backstop.)
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

    /// id 로 세션을 찾아 AgentInfo 를 조립(없으면 NotFound). activate_profile 이 resume 성공 후
    /// 살아있는 세션의 info 를 얻는 데 쓴다 — resume_no_fallback 은 세션을 맵에 등록만 하고 info 를
    /// 돌려주지 않으므로(RestoreOutcome 반환) id 로 재조회한다. §10 락 순서 준수(get_session 이 read
    /// lock 즉시 해제 → agent_info 는 lock 미보유 상태에서 호출).
    fn agent_info_by_id(&self, id: AgentId) -> Result<AgentInfo, PtyError> {
        let session = self.get_session(id)?;
        Ok(self.agent_info(&session))
    }

    /// id 로 canonical 표시명만 조회(없으면 None). 봉투 sender 등 AgentInfo 전체가 필요 없는
    /// 호출부(daemon ingress::sender_display_name)가 **agent_info 와 byte-identical** 한 이름을
    /// 얻게 하는 단일 출처다 — session.cwd 기반 resolve 를 여기 한 곳에 모아 로직 복제를 막는다.
    /// §10 락 순서: get_session 이 read lock 을 즉시 해제 → resolve 는 lock 미보유에서 수행.
    // ADR-0101
    pub fn canonical_name(&self, id: AgentId) -> Option<String> {
        let session = self.get_session(id).ok()?;
        Some(self.resolve_canonical_name(&session))
    }

    /// session → canonical 표시명(display_name ?? basename(session.cwd)). agent_info·canonical_name
    /// 공유 코어 — 이름 파생을 한 곳으로 모아 reaper/ingress/cli 와 어긋나지 않게 한다.
    ///
    /// ADR-0101 (WYSIWYA — canonical 이름 통일): AgentInfo.name = "사람이 트리에서 보는 이름"으로
    ///   맞춘다. 예전엔 profile.name(= createClaudeProfile 에 넘긴 full cwd 문자열, 종종 경로)을 그대로
    ///   써서 라우팅/로스터가 기대하는 주소와 트리 표시명(display_name ?? basename(cwd))이 어긋났다.
    ///   라우팅(resolve_recipient)·로스터·봉투 sender·프론트 트리가 **같은 문자열**을 써야 "보이는
    ///   이름으로 지목하면 그 에이전트에게 간다"가 성립한다.
    ///
    /// ★cwd 출처 = session.cwd(profile.cwd 아님)★: 프론트 트리는 `display_name ?? basename(AgentInfo.cwd)`
    ///   로 그리고 AgentInfo.cwd = session.cwd(spawn 시 canonicalize). profile.cwd 는 raw("."·".."·심링크)
    ///   라 여기서 파생하면 basename 이 갈려 트리 표시 ≠ 라우팅 주소가 된다. 그래서 AgentInfo.cwd 와
    ///   **같은 값**(session.cwd)에서 파생한다.
    // ADR-0101
    fn resolve_canonical_name(&self, session: &Arc<AgentSession>) -> String {
        // session.cwd = AgentInfo.cwd 와 동일 출처(canonical). 프론트 basename 규칙과 1:1.
        let cwd = session.cwd.to_string_lossy();
        // get()이 profiles lock을 잡아 clone 후 즉시 해제하므로 sessions lock과 동시에 보유하지 않는다
        //   (§10 락 순서, 이 함수는 sessions lock 미보유 상태에서만 호출).
        let display_name = self.profiles.get(session.id).and_then(|p| p.display_name);
        // 프로필 부재(ad-hoc / 산 세션에 DeleteProfile) 시에도 트리는 basename(cwd)를 그리므로 여기도
        //   cwd basename 으로 파생해야 트리 ≠ 라우팅 이 안 생긴다. cwd 가 placeholder/빈값을 낼 때만
        //   id 앞 8자로 degrade(blank·경로없음 라벨을 주소로 쓰지 않게).
        crate::agent::name::canonical_name_or_id_fallback(display_name.as_deref(), &cwd, session.id)
    }

    /// session 스냅샷 → AgentInfo. (sessions lock을 보유하지 않은 상태에서만 호출)
    fn agent_info(&self, session: &Arc<AgentSession>) -> AgentInfo {
        let name = self.resolve_canonical_name(session);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// harmless 자식(cmd.exe /c echo, 즉시 종료)으로 spec을 만든다 — 실 claude 없이 transport
    /// **선택 로직**만 검증하기 위한 격리 하네스(ADR-0012). transport의 caps로 어느 종류가
    /// 골렸는지 판정한다(spawn한 프로세스는 shutdown으로 정리).
    #[cfg(windows)]
    fn probe_spec() -> CommandSpec {
        CommandSpec {
            program: "cmd.exe".into(),
            args: vec!["/c".into(), "echo select-probe".into()],
            env: vec![],
            cwd: std::path::PathBuf::from("."),
        }
    }

    // ── ADR-0044: manager가 json 모드엔 StdioTransport(구조화 caps)를 고른다 ──
    #[cfg(windows)]
    #[test]
    fn select_transport_json_mode_picks_stdio_structured() {
        let (transport, _pid) =
            select_transport(true, &probe_spec(), DEFAULT_COLS, DEFAULT_ROWS, None)
                .expect("select");
        let caps = transport.capabilities();
        assert!(
            caps.output.structured && !caps.output.terminal_bytes,
            "json 모드 → StdioTransport(structured 출력, 터미널 바이트 아님)"
        );
        assert!(!caps.control.resize, "파이프 resize 불가");
        transport.shutdown();
    }

    // ── 회귀: 터미널 모드는 PtyTransport(터미널 바이트, resize 가능, 구조화 아님) ──
    #[cfg(windows)]
    #[test]
    fn select_transport_terminal_mode_picks_pty() {
        let (transport, _pid) =
            select_transport(false, &probe_spec(), DEFAULT_COLS, DEFAULT_ROWS, None)
                .expect("select");
        let caps = transport.capabilities();
        assert!(
            caps.output.terminal_bytes && !caps.output.structured,
            "터미널 모드 → PtyTransport(터미널 바이트, 구조화 아님)"
        );
        assert!(caps.control.resize, "PTY resize 가능");
        transport.shutdown();
    }
}
