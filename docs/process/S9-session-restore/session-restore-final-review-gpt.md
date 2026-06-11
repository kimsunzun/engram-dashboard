# ChatGPT 응답

결론부터 말하면 “구현 착수는 가능하지만, 신뢰 기능으로 공개하기 전 반드시 게이트를 걸어야 하는 설계”입니다.
특히 프로필/AgentId 분리, spawn, 기본 restore, sid watcher, fresh fallback까지는 GO이고, 자동 재시작·프론트 채널 승계·terminal 상태 재진입은 아직 NO-GO입니다.

0. 총평

문서의 핵심 결정인 AgentId와 claude_session_id 분리는 맞습니다. /clear가 같은 프로세스 안에서 새 session id를 만든다는 실측이 있으므로, AgentId = session_id였으면 바로 깨졌을 구조입니다. 설계 문서도 AgentId는 불변, claude_session_id는 가변 Optional로 분리하고 있습니다. 

session-restore-lld

Spike 결과도 §11을 상당히 강하게 지지합니다. --session-id 지정이 정상 동작하고, --resume은 fork 없이 기존 jsonl에 append되며, 없는 session resume은 exit 1로 실패합니다. 

spike-results

 또한 TUI에서도 --session-id가 동작하고 /clear 시 새 session id가 생성되며 파일 감지가 가능하다는 점이 확인됐습니다. 

spike-results

다만 adversarial하게 보면, 현재 설계의 가장 위험한 부분은 이것입니다.

sessions/<pid>.json 추적이 실패했을 때 “fallback으로 안전하게 새 세션”이 아니라, 옛 session id로 조용히 복원되어 사용자가 기대한 최신 세션이 아닌 곳으로 돌아갈 수 있다.

이건 데이터 손상은 아니지만, 사용자 입장에서는 복원 성공처럼 보이는 잘못된 복원이라 더 위험합니다.

1. §11 sid 확보 메커니즘 검토
판정

큰 방향은 타당합니다.
“최초 sid는 우리가 지정하고, 실행 중 변경만 sessions/<pid>.json으로 따라간다”는 구조는 현재 실측에 가장 잘 맞습니다. 설계도 최초 spawn 시 AgentId와 claude_session_id를 둘 다 생성하고, claude --session-id <id>로 시작하며, 이후 child PID 기반 파일을 watch하도록 되어 있습니다. 

session-restore-lld

또한 spike에서 ~/.claude/sessions/<PID>.json 안에 현재 sessionId, cwd, status, startedAt, updatedAt 등이 있고, /clear 시 같은 PID 파일의 sessionId가 실시간 갱신됨이 확인됐습니다. 

spike-results

하지만 함정이 있습니다

첫 번째 함정은 watch 누락입니다. /clear 직후 앱이 죽거나, 파일 이벤트가 coalescing되거나, write 중 partial JSON을 읽거나, watch 대상 파일이 rename 방식으로 교체되면 최신 sid 저장을 놓칠 수 있습니다. 이 경우 다음 앱 시작 때 저장된 옛 sid로 --resume이 성공해버릴 수 있습니다. 실패가 아니므로 fresh fallback도 타지 않습니다. 즉 “오래된 대화로 정상 복원”되는 silent wrong-restore가 생깁니다.

두 번째 함정은 fallback 조건이 너무 넓으면 안 된다는 점입니다. §4에는 “resume 실패 시 fresh fallback”이라고 되어 있지만, 실제로는 모든 exit 1을 fallback하면 위험합니다. 

session-restore-lld

 No conversation found는 fresh fallback이 맞지만, already in use, 인증 만료, trust prompt, cwd 오류, command not found, permission 문제는 fallback 대상이 아닙니다. 특히 already in use를 fresh fallback하면 기존 살아있는 세션을 버리고 새 세션을 만들어 중복 agent / orphan process를 만들 수 있습니다.

세 번째 함정은 PID가 우리가 생각한 claude PID가 아닐 수 있음입니다. PtyManager가 직접 claude를 spawn하면 괜찮지만, 중간에 shell, wrapper, shim, Windows launcher, npm/bun wrapper 같은 것이 끼면 우리가 가진 child PID와 sessions/<PID>.json의 PID가 달라질 수 있습니다. 문서에는 “PtyManager가 child PID를 보유”한다고 되어 있지만, 그 PID가 실제 Claude interactive process의 PID라는 보장은 구현에서 강제해야 합니다. 

session-restore-lld

네 번째 함정은 멀티 인스턴스입니다. 현재 spawn guard는 sessions map에 같은 profile id가 있으면 거부하는 인메모리 가드입니다. 

session-restore-lld

 그러나 앱이 두 개 떠 있으면 각 프로세스의 sessions map은 서로 모릅니다. OS-level app singleton lock 또는 agents store lock이 없으면 같은 AgentId / same sid를 두 앱이 동시에 resume하려고 할 수 있습니다.

§11에 추가해야 할 방어선

§11은 그대로 구현하지 말고 다음 보강을 넣는 게 좋습니다.

sid 관찰 상태를 저장하세요.
claude_session_id만 저장하지 말고 sid_observation_state = Authoritative | Stale | InitialOnly | Unknown 같은 상태를 둬야 합니다. sessions/<pid>.json을 정상 추적 중이면 Authoritative, 파일이 없거나 schema mismatch면 InitialOnly/Degraded로 표시해야 합니다.

종료 직전 동기화가 필수입니다.
watch에만 의존하지 말고, user kill / app shutdown / process exit drain 직전에 sessions/<pid>.json을 한 번 동기 read해야 합니다. 이게 /clear 직후 앱 종료 시나리오의 핵심 방어선입니다.

fresh fallback은 오류 분류 후에만 하세요.
No conversation found 또는 명확한 session-not-found 계열만 fresh fallback.
already in use는 ConflictExistingSession.
auth/trust prompt는 BlockedNeedsUserAction.
cwd 없음은 InvalidProfileCwd.
command 없음은 SpawnFailed.
이 분류가 없으면 fallback이 오히려 손상을 만듭니다.

새 sid는 spawn 성공 전 commit하지 마세요.
fallback 시 새 uuid를 만들더라도 바로 claude_session_id에 확정 저장하지 말고 pending_session_id로 둔 뒤, process running + transcript/session file 생성 확인 후 commit하는 편이 안전합니다.

2. §6 상태머신 충돌 해결 방향

여기는 현재 문서 그대로는 구현하면 안 됩니다. 문서도 Exited/Killed가 terminal인데 재시작은 같은 AgentId로 새 PtySession을 만들어야 해서 백엔드·프론트 LLD 개정이 필요하다고 명시하고 있습니다. 

session-restore-lld

권장 해결책

핵심은 terminal 상태를 AgentId 레벨에서 없애고, ProcessGeneration 레벨로 내리는 것입니다.

현재 충돌은 “Agent = PtySession”처럼 취급하기 때문에 생깁니다. 해결 구조는 이렇게 나누는 게 좋습니다.

AgentProfile / AgentRuntime
  - AgentId: 불변
  - current_generation: u64
  - state: Starting | Running | Restarting | Stopped | Failed | Blocked

PtyProcessGeneration
  - generation: u64
  - pty_session_id
  - pid
  - state: Running | Exited | Killed

즉, Exited/Killed는 개별 프로세스 generation의 terminal이고, Agent 자체는 Restarting을 거쳐 새 generation으로 Running에 재진입할 수 있어야 합니다.

구독자 승계는 “raw subscriber 이주”보다 Agent-level hub가 낫습니다

문서에는 옛 PtySession의 subscribers를 새 세션으로 이주하거나, 프론트가 재구독하는 선택지가 있습니다. 

session-restore-lld


adversarial하게 보면 raw Arc sink를 세션 간 이주하는 방식은 나중에 버그가 많이 납니다. ABA 문제, 늦게 도착한 old output, backpressure, close ordering, replay buffer 혼선이 생깁니다.

더 안전한 설계는 AgentId 단위 subscription hub입니다.

프론트는 PtySession이 아니라 AgentId에 subscribe합니다. 백엔드는 현재 generation의 output을 hub로 forward합니다. 프로세스가 죽으면 hub가 다음 이벤트를 보냅니다.

agent-output { agent_id, generation, bytes }
agent-process-ended { agent_id, generation, reason }
agent-restart-started { agent_id, from_generation, attempt }
agent-restart-succeeded { agent_id, generation }
agent-restart-failed { agent_id, reason }

이러면 프론트 채널은 안 끊기고, 내부 PtySession만 교체됩니다. 그리고 모든 이벤트에 generation을 붙이면 old process의 늦은 output을 버릴 수 있습니다.

replay buffer 정책

문서에는 “새 세션 시작이므로 replay buffer 리셋”이라고 되어 있습니다. 

session-restore-lld

 방향은 맞지만, 그냥 리셋하면 사용자가 “갑자기 로그가 사라졌다”고 느낄 수 있습니다.

권장 정책은 다음입니다.

generation N output
--- process exited: code=..., restarting attempt 1 ---
generation N+1 output

프론트 replay는 “현재 generation 기본 표시 + 이전 generation은 접힌 로그/경계 배너”가 가장 안전합니다. 최소 구현에서는 current replay만 유지해도 되지만, 반드시 restart boundary event는 남겨야 합니다.

자동 재시작 조건도 더 엄격해야 합니다

문서의 자동 재시작 설계는 “exit code 휴리스틱이 아니라 transition status로 판단”하고, resume 2회 → fresh 1회 → 정지하는 retry ladder를 둡니다. 

session-restore-lld

 이 방향은 맞습니다.

다만 Always의 의미를 명확히 해야 합니다.

사용자 kill: 재시작 금지

정상 exit 0: Always면 재시작할지, OnCrash면 안 할지 명확화

crash during user kill: user intent token이 있으면 kill로 분류

auth/trust prompt hang: 재시작 루프로 보지 말고 Blocked 상태

already in use: fresh fallback 금지, Conflict 상태

3. sessions/<pid>.json 비공식 파일 의존 리스크
판정

~/.claude/sessions/<PID>.json은 현재 실측상 매우 유용하지만, 신뢰 경계 밖의 undocumented contract입니다. Spike 문서도 version 필드로 인한 포맷 변경 가능성, PID 재사용 stale 위험, 프로세스 종료 시 파일 정리 여부 미확인을 명시하고 있습니다. 

spike-results

 설계 문서 역시 해당 파일을 비공식 내부 파일로 보고, 없거나 바뀌어도 최초 지정값 유지 → resume 시도 → fallback을 방어선으로 둡니다. 

session-restore-lld

이 방어는 “데이터 손상 방지”에는 충분하지만, “항상 최신 session 복원”에는 부족합니다.

대비책

첫째, parser는 tolerant하게 작성해야 합니다. sessionId만 UUID string으로 읽고, 나머지 필드는 있으면 검증 보조로만 써야 합니다. version, status, kind 값이 바뀌어도 fatal로 죽으면 안 됩니다.

둘째, 파일 read는 retry가 필요합니다. modify event 직후 partial JSON일 수 있으므로, 바로 실패 처리하지 말고 짧은 retry/backoff를 둬야 합니다.

셋째, PID 검증은 pid와 startedAt만으로는 부족할 수 있습니다. 가능하면 다음 조건을 같이 보세요.

json.pid == child_pid
json.cwd == canonical_profile_cwd
json.kind == "interactive" 이거나 unknown 허용
json.startedAt >= spawn_start_time - tolerance
OS process start time과 json.startedAt이 크게 어긋나지 않음

넷째, 파일 watch는 파일 단독 watch보다 parent directory watch + path filter가 더 안전합니다. 파일이 create/modify/replace/delete될 수 있기 때문입니다.

다섯째, 세션 동기화가 degraded되면 UI/로그에 표시해야 합니다. 예를 들어:

Session tracking degraded: Claude internal session file unavailable.
Restore will use last known session id.

이 경고가 없으면 사용자는 “완전 복원 보장”으로 오해합니다.

여섯째, Claude 버전 compatibility gate가 필요합니다. Spike 문서에도 PTY/통합터미널 환경에서 transcript 저장 버그가 release note로 관찰됐고, 최신 유지가 안전망이라고 되어 있습니다. 

spike-results

 최소한 앱 로그에 claude --version, sid file schema 상태, transcript creation 여부를 남겨야 합니다.

4. 이 최종 설계로 구현 시작해도 되는가?

조건부 GO입니다.

바로 구현해도 되는 범위:

AgentId / claude_session_id 분리

profile persist, atomic write, corrupt 보존

cwd canonicalize

fresh spawn with --session-id

resume spawn with --resume

sessions/<pid>.json watcher

/clear 후 sid 갱신

No conversation found 한정 fresh fallback

restore result UI event

아직 구현 착수 전에 설계 확정해야 하는 범위:

§6 상태머신 개정

Agent-level subscription hub vs raw subscriber migration

generation id 도입

restart boundary/replay 정책

자동 재시작 retry/cancel/blocked 상태

오류 분류표

multi-instance lock

즉, 세션 저장/복원 코어는 시작 가능하지만, 자동 재시작을 “동작한다”고 붙이는 순간 위험합니다. 문서의 진행 순서도 §6 상태머신 개정안 확정 후 코어 구현으로 되어 있어, 현재 문서 자체도 이 부분을 gate로 보고 있습니다. 

session-restore-lld

5. 미결정 위험 사항

가장 큰 미결정 위험은 아래입니다.

watch 누락 시 old sid로 조용히 복원되는 문제
fresh fallback보다 위험합니다. 종료 직전 sync와 sid_observation_state가 필요합니다.

resume exit 1 분류 미정
No conversation found만 fallback해야 합니다. already in use, auth, trust, cwd, spawn failure는 fallback하면 안 됩니다.

PtyManager child PID가 실제 Claude PID인지 보장 미정
shell/wrapper 경유 시 sessions/<pid>.json 조회가 틀어질 수 있습니다.

멀티 앱 인스턴스 가드 없음
인메모리 sessions map guard만으로는 부족합니다. OS-level lock이 필요합니다.

상태머신에서 Agent terminal과 Process terminal이 섞여 있음
Agent는 Restarting으로 재진입 가능해야 하고, terminal은 process generation에만 적용해야 합니다.

비공식 파일 schema 변경 대응 수준 미정
tolerant parser, degraded mode, version logging, compatibility probe가 필요합니다.

trust/login prompt hang 처리 미완성
문서에서도 trust는 해당 환경에서 미발생이며 새 머신에서는 가능하다고 되어 있습니다. 

spike-results

 성공/실패가 아니라 BlockedNeedsUserAction 상태로 모델링해야 합니다.

cwd canonicalize 실패/경로 변경 처리 미정
Spike에서 cwd 표기 차이만으로 다른 transcript 폴더가 생김이 확인됐으므로 canonicalize는 필수입니다. 

spike-results

 cwd가 사라진 경우 fallback이 아니라 profile error여야 합니다.

최종 권고

구현은 시작하세요. 단, “자동 재시작 포함 완성본”이 아니라 “복원 코어 + 계측 + 안전한 degrade”부터 시작하는 게 맞습니다.

가장 먼저 반영할 설계 수정은 이 3개입니다.

1. AgentRuntime / ProcessGeneration 분리
2. sid_observation_state + shutdown 직전 pid.json sync
3. resume error classification table

이 세 가지 없이 들어가면, 기능은 돌아가도 장애 상황에서 “틀린 세션으로 정상 복원”, “중복 agent 생성”, “재시작 루프”가 나올 가능성이 큽니다.
