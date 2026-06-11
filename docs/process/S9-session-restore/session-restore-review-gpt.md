# ChatGPT 응답

결론: 구현 시작은 가능. 단, “복원/자동재시작”을 신뢰 기능으로 넣으면 안 됨

② 설계는 ①의 핵심 교훈인 위치 기반 매핑 금지, node 프로세스명 감지 금지, claude resume best-effort 처리에는 잘 반응하고 있습니다. ① 문서도 claude --resume은 100% 신뢰 불가이며, 우리 메타데이터는 독립 저장하고 claude 복원은 best-effort로 보라고 못 박고 있습니다. 

wezterm-failure-analysis

다만 지금 상태로 바로 구현한다면 프로필/PTY/spawn 기반 구조는 시작 가능하지만, AgentId=session-id 통합, fresh fallback, 자동재시작 정책은 spike 전까지 잠정 구현으로 둬야 합니다. ② 설계 자체도 --resume <없는/손상 uuid>의 exit code/stderr, --session-id 재사용 동작, PTY redraw 등을 spike 필요 항목으로 명시하고 있습니다. 

session-restore-lld-draft

1. AgentId = claude --session-id 통합 타당성
방향은 맞지만, 그대로 “동일 ID”로 고정하면 위험합니다

좋은 점은 분명합니다. wezterm 실패 원인이 cwd:win:tab:pane 같은 위치 키에 세션을 묶은 것이었기 때문에, 안정적인 AgentId(uuid)를 기준으로 삼는 건 맞습니다. ①에서도 세션 식별은 위치 무관한 안정 ID여야 한다고 정리되어 있습니다. 

wezterm-failure-analysis

하지만 adversarial하게 보면 AgentId와 claude의 실제 session id를 동일 개념으로 취급하는 순간 새 결합이 생깁니다.

가장 큰 함정은 이것입니다.

AgentId = 우리 앱의 에이전트 정체성
ClaudeSessionId = claude CLI 내부 대화 세션 참조

이 둘은 수명이 다릅니다.

AgentId는 영구적이어야 합니다. 이름, cwd, env, restart policy, UI 슬롯 연결 같은 우리 앱의 프로필 정체성입니다.

반면 ClaudeSessionId는 손상될 수 있고, resume 실패할 수 있고, claude CLI 버전에 따라 의미가 바뀔 수 있으며, fresh fallback 시 교체될 수도 있습니다.

따라서 권장 구조는 다음입니다.

Rust
struct AgentProfile {
    id: AgentId,                 // 우리 앱의 안정 ID. 절대 위치/프로세스에 묶지 않음
    name: String,
    command: AgentCommand,
    cwd: PathBuf,
    env: Vec<(String, String)>,

    claude: Option<ClaudeBinding>,
}

struct ClaudeBinding {
    desired_session_id: Option<String>,   // 우리가 지정하려는 ID
    last_known_session_id: Option<String>,// 실제 관측된/성공한 ID
    generation: u32,                      // fresh fallback 횟수
    last_resume_status: ResumeStatus,
}

AgentId와 desired_session_id의 값은 초기에는 같게 둘 수 있습니다. 그러나 설계 의미상은 분리해야 합니다.

특히 위험한 부분: resume 실패 → --session-id <같은 id> fresh

② 설계는 resume 실패 시 --session-id <id>로 fresh fallback한다고 되어 있습니다. 

session-restore-lld-draft


이건 위험합니다.

왜냐하면 --resume <id>가 실패한 이유가 다음 중 무엇인지 모릅니다.

1. 세션이 없음
2. 세션은 있으나 손상됨
3. 세션은 있으나 cwd/project mismatch
4. 세션은 있으나 CLI 버전 변경으로 로드 실패
5. 세션은 있으나 다른 프로세스가 이미 사용 중
6. TUI/PTY 문제로 실패처럼 보임
7. exit code는 0인데 실제로는 빈 세션으로 들어감

이 상태에서 같은 <id>로 fresh를 열면, 최악의 경우 기존 손상 세션에 append, 덮어쓰기, 중복 세션 생성, 이후 resume 대상 오염이 생길 수 있습니다.

따라서 fallback은 이렇게 바꾸는 편이 안전합니다.

resume 실패
→ 기존 ClaudeSessionId는 failed/degraded로 표시
→ 사용자에게 "새 대화로 시작" 또는 "수동 복구" 선택권 제공
→ 자동 fresh가 필요하면 새 ClaudeSessionId 생성
→ AgentId는 유지

즉, Agent는 살아남고, Claude 세션은 갈아끼울 수 있어야 합니다.

2. JSON persist 정책: 무엇을 언제 저장/복원해야 하나

②의 “AgentProfile 목록만 저장, claude 대화 자체는 건드리지 않음” 방향은 맞습니다. claude 대화 자체를 직접 파싱/수정하기 시작하면 claude 내부 저장 포맷에 강결합됩니다. 

session-restore-lld-draft

다만 “프로필 변경 시 + 앱 종료 시 저장”만으로는 부족합니다.

저장해야 하는 것

최소 저장 대상은 다음입니다.

Rust
AgentProfile {
    id,
    name,
    command_kind,       // Claude / Shell
    cwd,
    env,
    created_at,
    last_active,

    restart_policy,
    auto_restore,       // 앱 시작 시 자동 복원할지
    user_stopped,       // 사용자가 의도적으로 끈 것인지

    claude_binding,     // AgentId와 분리
    last_known_state,   // Running / Exited / Crashed / ResumeFailed / FreshFallback
}

특히 빠지면 위험한 필드는 이것입니다.

필드	없을 때 위험
user_stopped	사용자가 끈 에이전트를 다음 실행 때 자동 복원
auto_restore	모든 과거 프로필이 무조건 되살아남
restart_policy	crash와 정상 종료를 구분 못 함
last_exit_reason	자동재시작 판단 불가
claude generation	fresh fallback이 몇 번째 대화인지 추적 불가
schema_version	JSON 구조 변경 시 깨짐
updated_at / last_active	오래된 죽은 프로필 정리 불가
app_instance_id 또는 lock 정보	앱 중복 실행 시 저장 충돌
저장 시점

“spawn/kill/rename 시 즉시 저장”은 맞지만, 다음 이벤트도 저장해야 합니다.

spawn requested
spawn succeeded
spawn failed
resume attempted
resume succeeded
resume failed
fresh fallback started
process exited
user killed
auto restart scheduled
auto restart exhausted
cwd/env/profile changed

즉, 저장 단위가 “프로필 변경”이 아니라 상태 전이여야 합니다.

JSON 정합성 위험

JSON 파일 하나로 가도 됩니다. SQLite까지는 아직 과합니다. 하지만 반드시 아래는 필요합니다.

1. atomic write
   agents.json.tmp에 쓰고 fsync 후 rename

2. backup
   agents.json.bak 유지

3. schema_version
   마이그레이션 가능하게

4. validate on load
   깨진 profile은 전체 복원 실패가 아니라 quarantine

5. single writer
   manager thread만 저장

6. app lock
   앱 두 개가 동시에 agents.json 쓰지 못하게

권장 파일 구조는 이렇게입니다.

agents.json
agents.json.bak
agents.lock

로드 실패 시 정책:

agents.json 파싱 성공 → 사용
agents.json 실패 + bak 성공 → bak 사용, 사용자에게 경고
둘 다 실패 → 빈 프로필로 시작, 원본 파일 보존
3. 자동재시작 위험: 크래시루프, 중복실행, 상태오염

② 설계의 자동재시작은 아직 위험합니다. OnCrash & code != 0 & retries < max → claude --resume <id>는 단순하지만, 실제 장애 모드가 너무 많습니다. 

session-restore-lld-draft

크래시루프

단순 max_retries + backoff만으로는 부족합니다.

필요한 정책은 이쪽입니다.

Rust
struct RestartPolicy {
    mode: RestartMode, // Never | OnCrash | Always
    max_retries_per_window: u32,
    window_sec: u64,
    backoff: BackoffPolicy,
    min_uptime_sec: u64,
}

핵심은 min_uptime_sec입니다.

예를 들어 실행 후 1초 만에 죽는다면 이건 “한 번 실패”가 아니라 즉시 루프입니다. 이 경우 빠르게 circuit breaker를 열어야 합니다.

3분 안에 3회 실패
또는 실행 후 10초 미만 종료가 2회 반복
→ 자동재시작 중지
→ 상태: RestartSuppressed
→ 사용자에게 복구 버튼 노출
중복실행

가장 위험한 건 같은 AgentId 또는 같은 ClaudeSessionId로 프로세스가 두 개 떠는 것입니다.

방어 조건:

spawn 전:
- manager.sessions에 AgentId가 Running이면 spawn 거부
- OS child pid가 아직 alive면 spawn 거부
- 이전 PTY drain thread 종료 확인 전 재spawn 금지
- resume/fallback 중에는 AgentId lock 획득

상태 머신이 필요합니다.

Stopped
Starting
Running
Stopping
Exited
Crashed
RestartScheduled
ResumeAttempting
ResumeFailed
FreshFallbackPending
DegradedRunning
RestartSuppressed

Starting, ResumeAttempting, RestartScheduled 상태에서 같은 AgentId spawn 요청이 들어오면 무조건 reject 또는 coalesce 해야 합니다.

상태오염

자동재시작에서 --resume <id>를 무조건 쓰면 대화 오염 가능성이 있습니다.

예를 들어 claude가 내부적으로 마지막 write를 flush하기 전에 죽었거나, 세션 파일이 half-write 상태일 수 있습니다. 그 직후 resume하면 손상 상태를 더 확정시킬 수 있습니다.

권장 정책:

crash 발생
→ 즉시 resume하지 말고 짧은 cooldown
→ 마지막 출력/exit reason 저장
→ resume 시도
→ 성공 판정 전까지 Running으로 보지 않음
→ 실패하면 자동 fresh 금지 또는 새 ClaudeSessionId로 격리
4. resume 실패 시 fallback UX

현재 ②의 “실패하면 fresh fallback”은 기술적으로는 graceful이지만, UX 관점에서는 사용자가 대화가 날아간 걸 모른 채 새 대화에 들어가는 문제가 큽니다. ②도 이 UX 신호를 검토 질문으로 남겨두고 있습니다. 

session-restore-lld-draft

권장 UX는 “조용히 복구”가 아니라 상태를 노출하는 복구입니다.

상태 뱃지

각 에이전트 카드에 이런 상태를 보여야 합니다.

Restored
Resume failed
Fresh session started
Restarting
Restart suppressed
Manual recovery needed
fallback 메시지

fresh fallback이 발생하면 터미널 상단 또는 카드 배너에 명확히 표시해야 합니다.

이 에이전트의 이전 Claude 대화 복원에 실패했습니다.
프로필, cwd, env는 유지되었지만 Claude 대화는 새 세션으로 시작되었습니다.

[다시 복원 시도] [새 세션으로 계속] [로그 보기] [프로필만 유지하고 닫기]
자동 fresh fallback 기본값

개발 초기에는 자동 fresh fallback을 기본 OFF로 두는 것이 안전합니다.

권장 기본값:

resume 실패
→ 자동 fresh 실행 X
→ 사용자에게 선택지 표시

단, 사용자가 설정에서 “resume 실패 시 자동 새 세션 시작”을 켠 경우에만 자동 fresh.

이유는 간단합니다. claude --resume이 불신 대상이라면, resume 실패 감지도 불신 대상입니다. 실패를 잘못 판정해서 기존 대화를 덮거나 새 대화로 몰래 바꾸면 더 나쁜 UX가 됩니다.

5. spike로 먼저 검증할 claude CLI 함정 추가 제안

②의 기존 spike 3개는 필수입니다: --session-id 지정 가능 여부, 없는/손상 id resume 동작, 다른 PTY 크기 redraw. 

session-restore-lld-draft

여기에 아래 항목을 추가해야 합니다.

A. --session-id와 --resume의 ID가 정말 같은 namespace인가

검증:

claude --session-id <uuid>로 시작
종료
claude --resume <same uuid>

확인할 것:

정말 같은 대화가 열리는가?
아니면 session-id와 resume id가 다른 개념인가?
uuid 형식을 강제하는가?
임의 문자열 허용인가?
B. 같은 --session-id를 동시에 두 번 실행하면?

가장 중요합니다.

terminal A: claude --session-id X
terminal B: claude --session-id X

확인할 것:

거부되는가?
둘 다 뜨는가?
대화 파일이 꼬이는가?
나중에 resume하면 어느 쪽이 살아남는가?

이 결과가 나오기 전까지는 같은 AgentId 중복 spawn 방어가 필수입니다.

C. --resume X 중 이미 X가 실행 중이면?
terminal A: claude --session-id X
terminal B: claude --resume X

확인할 것:

attach 개념인가?
복제 실행인가?
에러인가?
세션 오염인가?
D. resume 성공/실패를 exit code로 판별할 수 있는가

중요한 건 “없는 id가 exit code != 0인가”만이 아닙니다.

확인할 것:

없는 id
손상 id
권한 없는 세션 파일
다른 cwd/project의 id
이미 실행 중인 id
네트워크 실패
인증 만료

각각에 대해:

exit code
stderr
stdout
TUI 표시 문구
프로세스 생존 여부
입력 가능 상태 도달 여부
E. “성공처럼 보이는 실패”가 있는가

예를 들어 프로세스는 살아 있고 exit code도 없지만, 실제로는 다음 상태일 수 있습니다.

인증 필요 화면
엔터 대기 화면
빈 새 세션
권한 오류 화면
MCP/tool 초기화 실패 화면

따라서 성공 판정은 단순히 process alive가 아니라 ready marker가 필요합니다.

가능하면 TUI 출력에서 안정적으로 감지 가능한 문구를 찾고, 불가능하면 “spawn 성공”과 “resume 검증 성공”을 분리해야 합니다.

F. cwd가 달라지면 resume 의미가 바뀌는가

같은 session id라도 cwd가 바뀌었을 때:

원래 cwd로 resume되는가?
현재 cwd 기준으로 새 context가 잡히는가?
project 경로별 저장소가 달라지는가?
resume 실패하는가?

이건 AgentProfile.cwd와 ClaudeSessionId의 결합 위험을 판단하는 핵심입니다.

G. claude CLI 버전 변경 시 호환성
이전 버전에서 만든 session
업데이트 후 resume
다운그레이드 후 resume

확인해야 합니다.

H. 비정상 종료 직후 resume
kill -9 / 강제 종료 / 터미널 종료 / 앱 크래시
→ 즉시 resume
→ 1초 후 resume
→ 5초 후 resume

세션 파일 flush 타이밍 문제를 확인해야 합니다.

구현 시작 가능 범위
지금 바로 시작해도 되는 것
1. AgentProfile 모델 도입
2. AgentId uuid 기반 식별
3. 위치/slot과 backend profile 분리
4. PtyManager.spawn_agent(profile) 인터페이스 확장
5. 우리가 spawn한 child PID를 AgentId와 직접 매핑
6. JSON persist 기본 구조
7. atomic write / backup / schema_version
8. 상태 머신 뼈대
9. restart_policy 필드 추가하되 기본값 Never
10. resume/fallback 결과를 UI에 노출할 상태 필드 추가

여기까지는 ①의 실패 원인을 피하는 방향이라 구현 시작해도 됩니다.

아직 확정하면 안 되는 것
1. AgentId와 ClaudeSessionId를 영구적으로 같은 개념으로 고정
2. resume 실패 시 같은 id로 자동 fresh
3. process alive만 보고 resume 성공 처리
4. 자동재시작 기본 ON
5. code != 0만 crash로 간주
6. 앱 시작 시 모든 profile 무조건 restore
7. 같은 AgentId에 대해 중복 spawn 허용
8. claude 내부 저장소를 직접 수정/정리
최종 판정

조건부 Go입니다.

구현은 시작해도 됩니다. 단, 1차 구현 목표를 이렇게 제한해야 합니다.

목표:
에이전트 프로필/PTY/spawn/persist/상태 머신 기반을 먼저 만든다.

비목표:
claude resume을 신뢰 가능한 세션 복원 기능으로 완성했다고 선언하지 않는다.
자동재시작을 기본 활성화하지 않는다.
resume 실패 시 조용히 fresh로 넘어가지 않는다.

설계 수정의 핵심 한 줄은 이것입니다.

AgentId는 우리 앱의 영구 정체성이고,
ClaudeSessionId는 교체 가능한 외부 세션 참조다.
초기값은 같을 수 있지만, 개념은 반드시 분리한다.
