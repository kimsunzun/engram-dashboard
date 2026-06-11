# S9 세션 저장/복원 설계 검토 — fable

**검토자:** fable (pane 8), 2026-06-11
**대상:** `session-restore-lld-draft.md` (전제: `wezterm-failure-analysis.md`)
**확신 수준:** [확실] / [가능성 높음] / [불확실]

---

## 종합 판정: 방향 타당, 단 "통합"을 "통제"로 한 단계 풀어라 — 그리고 spike 1건이 설계 성립 자체를 좌우한다

wezterm 교훈의 적용(위치 무관 ID, 직접 spawn 추적, 레이아웃 분리, graceful fallback)은 올바르다. 그러나 **핵심 결정 "AgentId = session-id 통합"은 spawn 시점에는 성립하지만 세션 수명 전체에서는 깨질 수 있다.** 권고: `AgentId`(우리의 불변 키)와 `claude_session_id`(가변, 초기값 = AgentId)를 **프로필에서 분리**하라. 통제는 유지하되 동일시는 버리는 것 — 아래 (1)의 깨지는 경우들이 전부 이 분리 하나로 흡수된다.

---

## (1) AgentId = session-id 1:1 가정이 깨지는 경우

### 1-a. resume가 session id를 fork하는 동작 이력 [가능성 높음 — 버전 의존, spike 필수]

Claude Code는 시기에 따라 `--resume`가 **새 session id로 fork**하는 동작을 보였고, 이후 `--fork-session` 플래그가 분리된 이력이 있다(= 디폴트가 유지/포크 중 무엇인지가 버전에 따라 다를 수 있음). 만약 현재 버전에서 resume가 fork한다면: 복원 후 이어진 대화는 **새 id로 저장**되고, 다음 앱 재시작의 `--resume <AgentId>`는 **fork 이전 시점의 stale 대화**를 복원한다 — 사용자에겐 "대화가 과거로 되돌아가는" 최악의 증상. wezterm 분석 §2-3이 지적한 "캡처 비결정성"이 형태만 바꿔 되돌아오는 것이다.

**→ spike #4 (최우선 추가):** resume → 대화 2턴 진행 → 종료 → 다시 `--resume <AgentId>` → 직전 2턴이 보이는가. 왕복 2회.
**→ spike #5:** `claude --resume <old> --session-id <new>` 조합 가능 여부. 가능하면 "resume 결과를 우리가 지정한 id에 받기"가 되어 fork 문제가 근본 해결된다.

### 1-b. fresh fallback이 같은 id를 재사용 — 손상 세션과 충돌 [가능성 높음]

`--session-id`는 **이미 존재하는 세션 id로는 시작할 수 없다**(already in use 계열 오류)는 제약이 알려져 있다. §4 fallback의 전형적 트리거가 "세션 파일이 존재하지만 손상"인데, 이때 `--session-id <같은id>` fresh 재spawn은 **그 존재하는 파일과 충돌해 또 실패**한다 — fallback이 fallback을 못 한다. **수정:** fallback은 **새 uuid v4**로 spawn하고 `claude_session_id`를 갱신·persist (분리 권고를 채택하면 자연스럽다). 옛 id는 버리지 말고 프로필의 `old_session_ids` 이력에 보관 — (4)의 UX와 직결.

### 1-c. TUI 내부 사용자 행위 [가능성 높음]

실행 중인 claude TUI 안에서 사용자가 `/resume` 픽커로 다른 세션을 열거나 `/clear`를 하면, **프로세스는 같은데 대화 세션이 바뀐다.** 우리는 PTY 밖에서 이를 감지할 수 없다. 1:1은 "우리가 띄운 시점"의 사실이지 불변식이 아니다. 완화: best-effort로 수용하고 문서화 + (분리 채택 시) "마지막으로 우리가 아는 session_id"라는 의미로 필드 명명.

### 1-d. §4 restore_all 의사코드 버그 — Shell 프로필까지 Claude로 덮어쓴다 [확실]

`p.command = Claude{resume:true}`를 **무조건** 대입한다. `Shell{...}` 프로필이 복원 시 claude resume으로 둔갑. `match p.command`로 Claude variant일 때만 resume 모드 전환.

### 1-e. 동일 id 이중 spawn 가드 부재 [확실]

이미 실행 중인 AgentId에 대해 restore/재시작이 겹치면 같은 세션 파일에 claude 2개가 붙는다(손상 위험). manager에 "id가 sessions 맵에 존재하면 spawn 거부" 가드 1줄 명시.

### 1-f. `Claude` variant에 args 필드가 없다 [확실]

`Shell`은 args가 있는데 `Claude{resume}`는 모델/권한 모드 등 플래그를 지정할 수 없다. 프로필이 "spawn 설정화"가 목적이므로 `Claude { resume: bool, extra_args: Vec<String> }`로.

---

## (2) JSON persist 정책 — 골격 충분, 원자성이 빠졌다

- **[확실] atomic write 필수:** 현 설계대로 그냥 쓰면 크래시가 쓰기 도중에 나는 순간 `agents.json`이 잘려 **전 프로필 소실** — 이 문서가 막으려는 바로 그 사고다. `agents.json.tmp`에 쓰고 → flush → `rename`(Windows std::fs::rename은 대상 교체 가능). 3줄짜리 방어.
- **load 실패 시 덮어쓰기 금지:** 손상 JSON이면 `agents.json.corrupt-<ts>`로 보존 후 빈 목록으로 시작 + 사용자 알림. 손상 파일 위에 바로 저장해버리면 복구 기회 소멸.
- **저장 직렬화:** 변경시 저장의 호출자가 여럿이다(commands 스레드의 spawn/kill/rename + drain thread의 재시작 정책). 저장 함수는 단일 Mutex로 직렬화하고, 저장 시점에 전체 상태를 lock 안에서 스냅샷.
- **last_active 갱신 빈도 주의:** "변경 시 저장"에 last_active가 포함되면 사실상 고빈도 저장이 된다. last_active는 spawn/kill 시점만 갱신하거나 디바운스.
- **schema version 필드** 1개 추가 — 미래 마이그레이션 비용이 공짜가 된다.

이상 반영하면 JSON+변경시저장은 이 규모(수십 개)에 충분하다. SQLite 기각 동의.

---

## (3) 자동 재시작 — 루프보다 더 큰 문제는 백엔드 상태머신과의 충돌

### 3-a. [확실 — 통합 최대 누락] 재시작은 backend-lld-stage1 §9의 terminal 상태 전제를 깬다

확정된 백엔드 LLD에서 `Exited/Failed/Killed`는 **terminal**이고 자원 정리·세션 제거로 이어진다. 재시작은 "같은 AgentId로 새 PtySession"인데:

- 옛 PtySession의 `subscribers`는 세션과 함께 죽는다 → **프론트 Channel이 조용히 끊긴다.** 새 세션에 구독자를 승계할 것인가(Arc sink를 새 세션으로 이주 — 가능하고 UX 최선), 프론트가 status 이벤트를 보고 재구독할 것인가? **미정 — 결정 필수.**
- replay buffer 리셋 여부(이전 출력 소실?) 미정.
- 프론트 상태머신: Exited → (재시작) → Running 재진입을 frontend-integration-lld가 가정하지 않는다. `Restarting` 상태 추가 또는 Exited→Running 재진입 허용을 **양쪽 LLD에 명시**해야 한다.

이 절을 정의하지 않으면 재시작 기능이 기존 확정 설계 두 개와 충돌한 채 구현된다.

### 3-b. 결정적 재크래시 — resume가 크래시 원인을 다시 로드한다 [가능성 높음]

크래시 원인이 세션 내용(거대 컨텍스트, 손상 transcript)이면 `--resume`는 같은 폭탄을 다시 밟는다. backoff+max_retries로 멈추긴 하나, 사다리를 명시하라: **resume 재시도 K회(2회 권장) → fresh(새 id, 1-b 방식) 1회 → 정지+사용자 알림.**

### 3-c. retry 카운터 리셋 정책 부재 [확실]

lifetime 누적 max_retries면 "한 달간 가끔 죽는 정상 에이전트"가 어느 날 한도 소진으로 재시작 불능이 된다. "건강 가동 N분(예: 10분) 경과 시 카운터 리셋"을 정책에 포함.

### 3-d. 사용자 kill 구분은 이미 해결돼 있음 — 연결만 명시

백엔드 `transition()`이 shutdown 플래그로 Killed/Exited를 구분하므로 "정상 종료·사용자 kill 재시작 금지"는 exit code가 아니라 **status로 판정**하라(코드≠0 휴리스틱보다 견고). 한 줄 연결 명시.

---

## (4) fallback UX 신호 — 핵심 통찰: "대화 소실"이 아니라 "분리"다

fresh fallback해도 **옛 transcript는 `~/.claude/projects/`에 그대로 남는다.** 우리가 지우지 않는 한 잃는 게 아니라 연결이 끊기는 것. 따라서:

1. **이벤트:** `agent-restore-result { id, outcome: Resumed | FreshFallback { reason } }` 백엔드 emit (기존 StatusSink 확장).
2. **표시:** 트리 노드 배지(⚠) + 해당 터미널에 프론트가 직접 배너 write("이전 대화 복원 실패 — 새 세션으로 시작. 이전 대화는 보존되어 있습니다"). 시작 시 모달은 금지 — N개 에이전트면 모달 N개 스팸.
3. **복구 경로 제공:** 1-b의 `old_session_ids` 이력 덕에 "이전 세션 수동 재연결" 메뉴(추후)나 최소한 id 표시가 가능 — "보존되어 있다"는 말에 실체가 생긴다.
4. **persist:** 프로필에 `last_restore: { when, outcome }` 기록 — 나중에 "언제부터 새 대화였지?"에 답할 수 있다.

---

## (5) spike 3항목 외 놓친 claude CLI 함정

기존 spike 1~3 타당. 추가:

| # | 항목 | 왜 |
|---|---|---|
| 4 | **resume 후 session id 유지/fork 왕복 2회 테스트** | (1-a) — 이 결과가 설계 성립을 좌우. 최우선 |
| 5 | `--resume <old> --session-id <new>` 조합 가능 여부 | 가능하면 1-a 근본 해결 |
| 6 | 신규 cwd 첫 spawn 시 **trust 프롬프트/온보딩 대화상자** | 복원이 인터랙티브 프롬프트에 걸려 "행"처럼 보임 — exit code도 안 나온다. restore가 이를 구분 못 하면 fallback 트리거(②)가 오발/불발 |
| 7 | 로그인 만료 상태에서 복원 — N개가 동시에 로그인 프롬프트 | spike 6과 같은 계열: "복원 성공처럼 보이는 대기 상태" |
| 8 | **cwd 정규화** — claude 세션은 cwd별 디렉토리에 저장되므로 경로 표기가 흔들리면(`I:\` vs `i:\`, 후행 구분자) resume가 세션을 못 찾는다. spawn 전 canonicalize 1회 + 폴더 이동/개명 시 resume 실패를 fallback 트리거에 포함 | [가능성 높음] |
| 9 | restore_all의 **동시 N spawn** — claude N개 동시 기동의 메모리/기동 경합. stagger(순차+간격) 또는 동시 상한. 추가로 프로필에 `auto_restore: bool` — 전부 무조건 복원이 아니라 선택 가능하게 | 규모 커지면 체감 |

또한 claude CLI **버전 업그레이드로 플래그/세션 포맷이 바뀔 수 있음** — spike 결과는 "현재 버전 기준"임을 문서에 박고, resume 실패 fallback이 버전 변화의 안전망 역할을 겸하게 하라.

---

## (6) wezterm 교훈 적용이 충분한가

| 원칙 | 평가 |
|---|---|
| 1 위치 무관 ID | 충실 ✅ |
| 2 직접 spawn 추적 | 충실 ✅ |
| 3 세션 ID 우리가 통제 | **spawn 시점만 충족** — resume fork(1-a)·TUI 내 행위(1-c)에서 수명 전체 통제는 깨질 수 있다. AgentId/claude_session_id 분리로 보완해야 "통제" 주장이 정직해진다 |
| 4 레이아웃↔세션 분리 | 충실 ✅ — 슬롯 매핑을 프론트 책임으로 둔 것 올바름 |
| 5 복원 실패 graceful | 방향 ✅, 단 same-id 충돌(1-b)이 실제로는 graceful을 깨뜨린다 — 새 id fallback으로 고쳐야 완성 |

특히 경계할 것: 1-a가 사실로 확인되고 spike 5(조합)도 불가하면, "새 session id를 알아내는" 문제가 생기는데 이를 transcript 디렉토리 최신 파일 탐색 따위로 풀면 **wezterm 실패 §2-3(캡처 비결정성)으로 정확히 회귀**한다. 그 경우의 답은 캡처가 아니라 분리(AgentId 불변 + 매 복원을 fresh-with-new-id로 정책 변경)까지 후퇴하는 것이다 — 후퇴선을 미리 정해두면 spike 결과가 어떻게 나와도 설계가 무너지지 않는다.

---

## 정리 — 반영 권고 순서

1. **AgentId / claude_session_id 분리** (1-a·b·c 흡수, 원칙 3 완성)
2. spike #4·5 추가, #4 결과에 따른 후퇴선 명시 (§6)
3. fallback = 새 uuid + old_session_ids 이력 (1-b)
4. 재시작 ↔ 기존 LLD 상태머신/구독자 승계 절 신설 (3-a) — 백엔드·프론트 LLD 양쪽 개정 필요
5. atomic write + load 실패 보존 (2)
6. restore_all match 버그(1-d), 이중 spawn 가드(1-e), Claude args(1-f), retry 리셋(3-c), restore-result 이벤트(4), spike 6~9
