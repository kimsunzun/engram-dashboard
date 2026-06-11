# ChatGPT 응답

결론

조건부 GO입니다.
① 코드기획은 ② LLD의 큰 방향, 즉 AgentId / claude_session_id 분리, atomic persist, 기본 복원, sid watcher, fresh fallback, epoch 기반 프론트 재구독, 자동재시작 게이트를 대체로 반영하고 있습니다. 특히 코드기획이 코어 GO 범위를 “프로필/분리/spawn/persist/복원/watcher/fallback”으로 제한하고 자동재시작을 보류한 점은 LLD의 확정 범위와 맞습니다. 

session-restore-code-plan

 

session-restore-lld

다만 그대로 구현 시작은 위험합니다. 시작 전 최소한 아래 5개는 고쳐야 합니다.

spawn_agent(&self, profile: &AgentProfile, ...)가 claude_session_id == None일 때 새 sid를 만들고 persist해야 하는데, 현재 시그니처로는 저장 주체가 불명확합니다.

restore_all() -> Vec<RestoreOutcome>는 agent_id, epoch, old_sid/new_sid, Blocked/Failed 같은 컨텍스트가 빠져 UI 이벤트와 운영 진단에 약합니다.

self_test가 “spawn 직후 1회”이면 너무 취약합니다. bounded retry, shim scan, startedAt/updatedAt 검증, degraded 상태 저장이 필요합니다.

atomic persist는 flush → rename만으로 부족합니다. 같은 디렉터리 tmp, sync_all, rename 후 parent dir fsync, stale snapshot 방지가 필요합니다.

구현 순서가 뒤집혀 있습니다. 코드기획은 LLD 개정을 6단계에 두지만, LLD는 상태머신/프론트 개정이 코어 구현 전 선행되어야 한다고 못박고 있습니다. 

session-restore-code-plan

 

session-restore-lld

1. 모듈 분해와 함수 시그니처 검토

모듈 분해 자체는 타당합니다. types, persistence, session_tracker, manager, commands/lib로 나눈 구조는 코어 GO 범위에 맞습니다. 코드기획의 데이터 모델도 AgentId 불변, claude_session_id 가변, old_session_ids, epoch, auto_restore를 넣어 LLD의 핵심 분리를 반영합니다. 

session-restore-code-plan

 

session-restore-lld

하지만 시그니처는 아직 구현 안전성이 부족합니다.

문제 A — spawn_agent(&self, profile: &AgentProfile, mode)는 저장 책임이 애매함

LLD는 sid가 없으면 uuid를 만들고 persist한 뒤 spawn해야 한다고 되어 있습니다. 

session-restore-lld


그런데 코드기획은 &AgentProfile을 넘기면서 “sid 없으면 새 uuid”라고만 합니다. 

session-restore-code-plan

이대로면 세 가지 위험이 생깁니다.

새 sid를 만들었지만 profile store에 저장하지 않고 spawn할 수 있음.

spawn 성공 후 persist 전 크래시가 나면 다음 실행에서 sid를 잃음.

watcher callback, profile update, restore_all이 서로 다른 snapshot을 저장해 최신 sid를 덮어쓸 수 있음.

권장 수정:

Rust
pub fn spawn_agent(&self, id: AgentId, mode: SpawnMode) -> Result<AgentInfo, PtyError>;

또는

Rust
pub fn spawn_agent(&self, req: SpawnRequest) -> Result<SpawnResult, PtyError>;

그리고 내부에서 반드시 ProfileStore를 통해 다음 순서로 처리하는 게 안전합니다.

profile lock 획득

sid 없으면 생성

profile 저장

spawn

self-test

watcher 등록

AgentInfo { id, epoch, ... } 반환

즉, 외부에서 profile clone을 넘기는 방식보다 manager가 store를 통해 원본 profile을 갱신하는 구조가 맞습니다.

문제 B — RestoreOutcome가 너무 작음

코드기획의 RestoreOutcome는 Resumed | FreshFallback { reason }뿐입니다. 

session-restore-code-plan


하지만 LLD의 UI 신호는 agent-restore-result { id, outcome }이고, 실패 시 트리 배지와 터미널 배너까지 연결됩니다. 

session-restore-lld

restore_all() -> Vec<RestoreOutcome>만으로는 어느 agent의 결과인지, epoch가 몇인지, old sid와 new sid가 무엇인지 알기 어렵습니다.

권장 타입:

Rust
pub struct RestoreResult {
    pub agent_id: AgentId,
    pub epoch: u32,
    pub outcome: RestoreOutcome,
    pub old_sid: Option<Uuid>,
    pub new_sid: Option<Uuid>,
    pub reason: Option<String>,
    pub when: i64,
}

그리고 outcome은 최소한 아래처럼 확장하는 게 좋습니다.

Rust
pub enum RestoreOutcome {
    Resumed,
    FreshFallback,
    SkippedShellFresh,
    BlockedNeedsUserAction,
    Failed,
}

BlockedNeedsUserAction가 중요한 이유는 trust/login prompt hang은 fresh fallback으로 해결되지 않을 수 있기 때문입니다. LLD도 trust prompt와 로그인 만료 좀비를 별도 위험으로 보고 있습니다. 

session-restore-lld

문제 C — LLD 데이터 모델 일부가 빠짐

LLD의 AgentProfile에는 restart_policy와 last_restore가 있습니다. 

session-restore-lld


코드기획에는 둘 다 없습니다. 자동재시작은 게이트이므로 restart_policy 생략은 허용 가능합니다. 하지만 last_restore는 코어 복원 결과 표시와 디버깅에 유용하므로 지금 넣는 편이 좋습니다.

권장 판단은 이렇습니다.

restart_policy: YAGNI로 보류 가능. 단 schema migration 계획은 남겨야 함.

last_restore: 지금 추가 권장. 복원 결과 UI/로그/사용자 알림과 바로 연결됨.

2. LLD 개정 7건 커버 여부

문서상으로는 7건을 모두 나열했습니다. 백엔드 a~d, 프론트 e~g가 코드기획에 명시되어 있습니다. 

session-restore-code-plan


LLD도 이 7건이 코드기획 입력이라고 정리합니다. 

session-restore-lld

하지만 나열과 구현 순서가 다릅니다.
코드기획은 manager: spawn_agent/restore_all + fresh fallback을 5단계에 두고, LLD 개정을 6단계에 둡니다. 

session-restore-code-plan


반대로 LLD는 상태머신 충돌 때문에 백엔드/프론트 LLD 개정이 코어 구현 전에 필요하다고 합니다. 

session-restore-lld

이건 BLOCKER입니다.
수정된 구현 순서는 아래가 더 안전합니다.

dunce + types 확장

상태머신/프론트 LLD 개정 a~g 먼저 반영

persistence

session_tracker + self-test

manager spawn/restore/fallback

commands/lib + UI 이벤트

restart_agent는 계속 게이트

3. session_tracker의 PID shim 우회 + self-test 검토

방향은 맞습니다. LLD의 필수 수정 중 하나가 PID shim 우회이고, sessions/<child_pid>.json이 없으면 sessions/*.json에서 sessionId == expected를 찾는 방식입니다. 

session-restore-lld


코드기획도 이를 반영했습니다. 

session-restore-code-plan

하지만 현재 설계는 견고하다고 보기 어렵습니다.

취약점 A — “spawn 직후 1회 self-test”는 너무 약함

파일 생성이 약간 늦거나, atomic replace 중이거나, Windows/보안 프로그램/파일 공유 이슈가 있으면 1회 체크는 쉽게 실패합니다. 코드기획은 읽기 공유위반 시 짧은 재시도를 언급하지만, self-test 자체는 “spawn 직후 1회”로 되어 있습니다. 

session-restore-code-plan

권장: self-test는 1회가 아니라 bounded retry가 되어야 합니다.

예: 50ms → 100ms → 200ms → 500ms → 1s, 총 3~5초 제한

Missing, TransientIo, Malformed, VersionUnsupported, Mismatch, Ambiguous를 구분

실패 시 프로세스를 무조건 죽일지, degraded로 유지할지 정책화

취약점 B — PID 재사용/오래된 파일 검증이 코드기획에 빠짐

LLD는 startedAt/updatedAt으로 우리 프로세스 검증을 하라고 합니다. 

session-restore-lld


코드기획에는 이 검증이 없습니다.

sessionId == expected만으로는 다음 상황이 위험합니다.

이전 실행의 stale file이 남아 있음.

같은 sid를 가진 죽은 세션 파일을 잘못 잡음.

shim process의 child가 여러 단계라 actual node PID가 달라짐.

/clear 직후 파일 replace 타이밍에 예전 내용을 읽음.

권장 self-test 결과 타입:

Rust
pub enum SelfTestResult {
    VerifiedExactPid { path: PathBuf },
    VerifiedShimResolved { path: PathBuf, actual_pid: u32 },
    MissingAfterRetry,
    Mismatch { path: PathBuf, expected: Uuid, found: Option<Uuid> },
    Ambiguous { candidates: Vec<PathBuf> },
    UnsupportedVersion { path: PathBuf, version: Option<u32> },
    TransientIo { error: String },
}

그리고 shim scan 후보는 최소 조건을 걸어야 합니다.

sessionId == expected

updatedAt이 spawn 시작 이후 또는 최근

가능하면 pid가 살아 있음

가능하면 child process tree 안에 있음

여러 후보면 최신 하나를 조용히 고르지 말고 Ambiguous

취약점 C — watcher callback에 epoch guard가 필요함

가장 위험한 레이스는 이겁니다.

agent A epoch 1 실행

watcher 1이 살아 있음

agent가 재시작되어 epoch 2 실행

watcher 1이 늦게 파일 변경을 감지

watcher 1이 profile의 claude_session_id를 덮어씀

코드기획은 watcher callback이 즉시 atomic persist한다고 되어 있지만, 어느 epoch의 watcher인지 검증하는 조건이 없습니다. 

session-restore-code-plan

권장: watcher 등록 시 agent_id + epoch + child_pid + initial_sid를 캡처하고, 저장 전 현재 running session의 epoch와 pid가 일치할 때만 갱신해야 합니다.

또한 spawn_session_watcher는 handle을 반환해야 합니다.

Rust
pub struct SessionWatcherHandle { ... }

pub fn spawn_session_watcher(
    agent_id: AgentId,
    epoch: u32,
    child_pid: u32,
    expected: Uuid,
    on_change: impl Fn(SessionChange) + Send + Sync + 'static,
) -> io::Result<SessionWatcherHandle>;

process exit, kill, restart 시 handle을 반드시 stop해야 watcher leak과 stale write를 막을 수 있습니다.

취약점 D — tracking 실패 시 “silent stale restore 금지”를 구현할 상태가 없음

LLD는 sessions 추적 실패 시 옛 sid로 조용히 복원하지 말고, 불확실하면 명시적 fresh fallback + 알림으로 가라고 합니다. 

session-restore-lld


코드기획도 “불확실하면 fresh”라고 적었지만, profile에 sid 신뢰도를 저장하는 필드가 없습니다. 

session-restore-code-plan

이러면 앱 재시작 시 “이 sid가 확실한 최신 sid인지, watcher가 죽은 뒤 오래된 sid인지” 구분할 방법이 없습니다.

권장: 아래 중 하나가 필요합니다.

sid_confidence: Verified | Degraded | Unknown

또는 session_tracking_state: Healthy | DegradedSince(i64) | Disabled

또는 watcher 실패 시 claude_session_id = None으로 내리고 다음 실행은 Fresh

개인적으로는 sid_confidence가 가장 낫습니다. Verified면 resume, Degraded/Unknown이면 사용자에게 알리고 fresh fallback 또는 수동 선택으로 가면 됩니다.

4. atomic persist / fresh fallback 허점
persist는 “atomic rename”만으로 부족함

코드기획은 tmp 쓰기 → flush → rename을 말합니다. 

session-restore-code-plan


LLD는 atomic write, 손상 보존, 단일 Mutex, last_active 디바운스를 요구합니다. 

session-restore-lld

여기서 adversarial하게 보면 다음이 빠져 있습니다.

tmp 파일은 반드시 같은 디렉터리에 만들어야 함.

flush가 아니라 File::sync_all()까지 해야 함.

rename 후 parent directory fsync가 필요함.

같은 프로세스 Mutex만으로는 다중 앱 인스턴스 충돌을 막지 못함.

save_profiles(profiles: &[AgentProfile])는 stale snapshot 저장으로 watcher update를 덮어쓸 수 있음.

corrupt load 후 빈 목록으로 시작했을 때, 자동 save가 바로 돌면 “손상 파일 보존”은 했지만 새 빈 파일로 상태가 확정되어 복구 UX가 나빠질 수 있음.

권장: save_profiles(Vec)가 아니라 store update closure가 안전합니다.

Rust
pub fn update_profile<R>(
    agent_id: AgentId,
    f: impl FnOnce(&mut AgentProfile) -> R,
) -> io::Result<R>;

또는 전체 store에 대해:

Rust
pub fn update_profiles<R>(
    f: impl FnOnce(&mut ProfilesDocument) -> R,
) -> io::Result<R>;

이렇게 해야 watcher, command, restore가 같은 lock 안에서 최신 문서를 갱신합니다.

profile.env는 “경고”로는 부족함

LLD는 profile.env 평문 persist 금지를 필수 수정으로 둡니다. 

session-restore-lld


코드기획은 env를 그대로 모델에 두고, *_KEY/*_TOKEN 패턴 경고만 둡니다. 

session-restore-code-plan

 

session-restore-code-plan

이건 설계 근거보다 약합니다. “경고”는 결국 유출을 허용합니다.

권장: env를 둘로 나누는 게 안전합니다.

Rust
pub struct AgentEnv {
    pub plain: Vec<(String, String)>,
    pub inherited_keys: Vec<String>,
    pub secret_refs: Vec<SecretRef>,
}

코어 GO에서는 secret_refs 구현까지는 YAGNI로 미뤄도 됩니다. 하지만 평문 persist denylist/block는 지금 넣는 게 좋습니다.

fresh fallback 순서도 명시해야 함

LLD는 resume 실패 시 새 uuid fresh fallback, old sid 이력, UI 알림을 요구합니다. 

session-restore-lld


코드기획도 반영했습니다. 

session-restore-code-plan

하지만 구현 순서가 중요합니다.

권장 순서:

resume 실패를 분류한다.

old sid를 old_session_ids에 넣는다.

new sid를 생성한다.

profile의 current sid를 new sid로 바꾼다.

먼저 persist한다.

fresh spawn한다.

spawn 실패 시 RestoreOutcome::FailedAfterFallback로 남긴다.

persist를 spawn 뒤로 미루면 fresh spawn 성공 직후 크래시에서 새 sid를 잃습니다. 반대로 persist를 먼저 하면 spawn 실패 시 current sid가 “아직 실제 대화가 없는 sid”가 될 수 있지만, old sid가 이력에 남아 있으므로 복구 가능하고 silent stale보다 낫습니다.

5. 미래 확장성 평가

현재 구조가 OutputSink/StatusSink trait + 코어 분리를 제대로 지키고 있다면 확장성 방향은 좋습니다. 하지만 AgentCommand와 session tracking이 Claude 전용으로 굳으면 곧 막힙니다.

(a) Codex CLI 지원

OpenAI의 Codex CLI는 로컬 터미널에서 실행되는 coding agent이고, 대화형 TUI로 codex를 실행하는 형태가 문서화되어 있습니다. 또한 공식 CLI reference에는 interactive TUI를 remote app-server endpoint에 WebSocket 또는 Unix socket으로 연결하는 --remote 모드가 있고, 이 remote mode는 codex, codex resume, codex fork에 지원된다고 되어 있습니다. 
OpenAI 개발자
 
OpenAI 개발자

즉, Codex는 단순히 Shell { program: "codex" }로만 처리하면 나중에 session/resume/remote 의미를 제대로 담기 어렵습니다.

지금 준비할 것:

Rust
pub enum AgentCommand {
    Claude(ClaudeCommand),
    Codex(CodexCommand),
    Shell(ShellCommand),
}

또는 더 일반적으로:

Rust
pub enum AgentKind {
    Claude,
    Codex,
    Shell,
}

그리고 command build를 별도 계층으로 빼야 합니다.

Rust
pub trait AgentCommandBuilder {
    fn build_spawn(&self, profile: &AgentProfile, mode: SpawnMode) -> SpawnSpec;
}

더 중요한 건 session 전략 분리입니다.

Rust
pub enum SessionStrategy {
    ClaudeSessionJson,
    CliNativeResume,
    None,
}

YAGNI로 미룰 것:

Codex의 실제 resume/fork/session 저장소 watcher 구현

Codex remote mode 완성

Codex 전용 config migration

Codex 로그/과금/권한 정책 세부 구현

지금은 enum 확장성과 SpawnSpec 분리만 해두면 충분합니다.

(b) 모바일 원격제어 — PtyManager 위 WebSocket 레이어

OutputSink/StatusSink trait가 Tauri에 묶이지 않은 순수 trait라면 방향은 좋습니다. LLD도 프론트 재구독을 epoch 기반으로 확정했고, 구독자 승계가 아니라 agentId + epoch로 다시 subscribe하는 방향입니다. 

session-restore-lld

지금 준비할 것:

sink payload를 Tauri 이벤트가 아니라 serializable domain event로 정의

subscribe(agent_id, epoch, sink) 구조 유지

OutputFrame에 sequence number 추가

replay buffer reset 정책 명시

input path도 transport-neutral하게 분리: send_input(agent_id, bytes), resize(agent_id, cols, rows)

sink별 backpressure/drop 정책 정의

YAGNI로 미룰 것:

실제 WebSocket 서버

모바일 인증/권한

reconnect token

multi-device cursor/selection sync

원격 파일 브라우저

다만 OutputSink가 단순 write(bytes)뿐이면 WebSocket 레이어에서 상태 이벤트, replay, terminal reset, 배너 순서를 다루기 어렵습니다. 최소한 StatusSink와 OutputSink는 동일한 event envelope를 공유하는 편이 좋습니다.

(c) 종량제 비-PTY API

여기가 가장 큰 구조 검증 포인트입니다. 비-PTY API는 terminal byte stream이 아니라 request/response/event/cost/usage 중심입니다. 지금의 PtyManager가 곧 AgentManager 역할까지 먹고 있으면 나중에 크게 찢어야 합니다.

지금 준비할 것:

PtyManager를 코어 agent lifecycle과 분리

ExecutionBackend 개념만 얇게 예약

Rust
pub enum ExecutionBackendKind {
    Pty,
    Api,
}

또는:

Rust
pub trait AgentBackend {
    fn spawn(&self, req: SpawnRequest) -> Result<RunningAgent, AgentError>;
}

profile에는 PID, PTY size, Claude session JSON path 같은 실행 중 상태를 저장하지 않기

status event에 usage/cost 필드를 나중에 붙일 수 있게 non-breaking 구조 유지

YAGNI로 미룰 것:

실제 API backend

과금 계산/ledger

quota UI

API retry policy

streaming protocol 세부 구현

즉, 지금은 PtyManager를 절대 도메인 최상위 manager로 굳히지 않는 것만 준비하면 됩니다.

6. 구현 순서상의 리스크

가장 위험한 순서는 현재 코드기획의 5→6입니다. manager 구현 후 LLD 개정을 하면 epoch/replay/status/event 계약이 뒤늦게 바뀌어 재작업이 납니다. 

session-restore-code-plan

권장 구현 순서:

LLD a~g 반영 PR

types/schema 추가

ProfileStore 구현 + persist crash/corrupt 테스트

session_tracker self-test를 독립 테스트 가능하게 구현

manager spawn fresh만 연결

manager resume + fallback 연결

watcher clear 감지 + epoch guard 연결

commands/lib restore trigger

UI agent-restore-result + epoch resubscribe

자동재시작은 계속 게이트

특히 3번과 4번은 통합 전에 단위 테스트를 강하게 넣는 게 좋습니다.

최종 판단

이 코드기획대로 “방향성 착수”는 가능하지만, “그대로 구현 시작”은 보류하는 게 맞습니다.

착수 전 반드시 반영할 미결정/위험 사항은 아래입니다.

구분	판정	이유
모듈 분해	GO	큰 경계는 적절함
LLD 7건 커버	조건부 GO	문서상 커버했지만 구현 순서가 틀림
spawn_agent 시그니처	수정 필요	sid 생성/persist 책임 불명확
RestoreOutcome	수정 필요	agent id, epoch, blocked/failed 결과 부족
session_tracker	수정 필요	1회 self-test, PID stale, epoch guard 부족
atomic persist	수정 필요	fsync, parent dir fsync, stale snapshot 방지 필요
env persist	수정 필요	LLD는 평문 금지인데 코드기획은 경고 수준
fresh fallback	조건부 GO	방향은 맞지만 persist 순서/실패 결과 명시 필요
Codex/WebSocket/API 확장	조건부 GO	Sink 분리는 좋지만 AgentCommand/Backend 분리 예약 필요
자동재시작	보류 유지	LLD와 코드기획 모두 게이트라 적절

한 줄로 정리하면: 코어 GO는 맞지만, 구현 전 “ProfileStore + RestoreResult + SelfTestResult + WatcherHandle/epoch guard + persist 내구성”을 먼저 코드기획에 추가해야 합니다.
