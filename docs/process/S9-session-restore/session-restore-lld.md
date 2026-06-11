# 세션 저장/복원 + 프로필 + 자동 재시작 — 통합 설계 (3자 검증 반영)

상태: **draft 개정본.** fable/Gemini/GPT 3자 검증 통합. 전제: `wezterm-failure-analysis.md`.
판정 종합: **Spike 즉시 GO / 백엔드 코어 풀구현은 spike 후** (Gemini NO-GO, GPT "신뢰기능 보류", fable "spike #4가 성립 좌우").
초안 대비 변경 이력: `session-restore-lld-draft.md`(원안) → 본 문서. 리뷰 전문: `s9-review-fable.md`, `session-restore-review-gemini.md`, `-gpt.md`.

## 0. 핵심 결정 — 통합이 아니라 "분리된 통제" (3자 만장일치)

원안의 `AgentId = claude --session-id` **동일시는 폐기.** 둘은 수명이 다르다(AgentId 영구 / session_id는 손상·fork·resume실패·교체 가능). **분리:**

- `AgentId` (uuid v4) — 우리의 **불변** 키. 슬롯/프로필/추적의 유일 기준. 절대 안 바뀜.
- `claude_session_id` (uuid v4, **가변, Optional**) — 현재 claude 세션. 초기값은 새로 생성, fallback/교체 시 갱신, 이력 보관.

## 1. 데이터 모델

```rust
struct AgentProfile {
    id: AgentId,                          // 불변 키
    name: String,
    command: AgentCommand,
    cwd: PathBuf,                          // spawn 전 canonicalize (spike #8)
    env: Vec<(String,String)>,
    claude_session_id: Option<Uuid>,      // 가변 — 현재 세션
    old_session_ids: Vec<Uuid>,           // fallback/교체 이력 (복구·표시용)
    restart_policy: RestartPolicy,        // Never | OnCrash | Always
    auto_restore: bool,                   // 앱 시작 시 자동 복원 여부 (spike #9)
    last_restore: Option<RestoreResult>,  // { when, outcome }
    created_at, last_active,              // last_active는 spawn/kill만 갱신(디바운스)
}
enum AgentCommand {
    Claude { extra_args: Vec<String> },   // 모델/권한 등 (resume 여부는 spawn 인자로 결정, 프로필에 안 박음)
    Shell  { program: String, args: Vec<String> },
}
// 파일 최상단: schema_version: u32  (미래 마이그레이션)
```

## 2. Persist (3자 — atomic 필수)

- **atomic write:** `agents.json.tmp` 쓰기 → flush → `rename`(교체). 크래시 중 프로필 통째 증발 방지. **(Gemini Split-Brain / fable 필수)**
- **load 실패 보존:** 손상 시 `agents.json.corrupt-<ts>`로 이동 후 빈 목록 시작 + 사용자 알림. 손상 위에 덮어쓰기 금지.
- **저장 직렬화:** 호출자 다수(commands spawn/kill/rename + drain 재시작) → 단일 Mutex, lock 안에서 전체 스냅샷.
- **last_active 디바운스:** 변경시저장에 매번 포함 금지(고빈도화). spawn/kill만 즉시.
- 형식 JSON(수십 개 규모 충분, SQLite 기각 3자 동의).

## 3. spawn (프로필 기반)

```
spawn_agent(profile, mode):
  guard: profile.id가 sessions 맵에 이미 있으면 거부 (이중 spawn 가드)  ← 1-e
  sid = profile.claude_session_id (없으면 uuid v4 생성 후 persist)
  args = match (profile.command, mode) {
    (Claude{extra}, Fresh)  => ["claude","--session-id", sid, ...extra]
    (Claude{extra}, Resume) => ["claude","--resume", sid, ...extra]   // spike #5: +--session-id 조합 가능하면 그걸로
    (Shell{p,a}, _)         => [p, ...a]      // Shell은 mode 무시 (원안 1-d 버그 수정)
  }
  PTY spawn (canonicalize(cwd), env) → 기존 PtyManager 흐름
```

## 4. 복원 (앱 시작) — fallback은 "분리"지 "소실" 아님

```
restore_all():  // auto_restore=true 프로필만, stagger(순차+간격) — spike #9
  for p in profiles where p.auto_restore:
    if let Claude = p.command:  spawn(p, Resume)   // Shell은 Fresh
    else: spawn(p, Fresh)
```
- **fallback (resume 실패 시):** **새 uuid v4**로 Fresh 재spawn (같은 id 재사용은 already-in-use로 또 실패 — 1-b). 옛 sid → `old_session_ids`. **claude transcript는 `~/.claude/projects/`에 보존됨 → 소실 아니라 분리.**
- **실패 감지:** spike #2/#6/#7로 신호 확정 — exit/stderr 종료 vs trust·로그인 프롬프트 Hang 구별. (Hang을 성공으로 오판 금지)
- **UI 신호 (silent 금지, 3자 만장일치):** `agent-restore-result { id, outcome: Resumed | FreshFallback{reason} }` emit → 트리 배지(⚠) + 터미널 배너("이전 대화 복원 실패 — 새 세션. 이전 대화는 보존됨"). **모달 금지**(N개 스팸).

## 5. 자동 재시작 — spike 후 활성화 (3자: 신뢰기능으로 미리 넣지 말 것)

- **상태 판정:** exit code 휴리스틱 아님 — 백엔드 `transition()`의 status로 판정(shutdown=사용자kill→재시작 안 함). Exited{code≠0}/비정상만.
- **재시도 사다리:** resume K회(2) → fresh 새 id 1회 → 정지 + 알림. (Poison Pill 루프 방지 — Gemini/fable)
- **retry 리셋:** 건강 가동 N분(10) 경과 시 카운터 리셋(누적 소진 방지).
- **인증 만료 좀비:** spike #7 — 재시작이 로그인 프롬프트에서 멈추는 것 감지 필요.

## 6. ★상태머신 충돌 — 양쪽 LLD 개정 필요 (fable 최대 누락)★

재시작 = "같은 AgentId로 새 PtySession". 확정 백엔드 LLD에서 Exited/Killed는 **terminal**이라 충돌:
- **구독자 승계:** 옛 PtySession `subscribers`(Arc sink)를 새 세션으로 **이주** (프론트 Channel 안 끊김, UX 최선). 또는 프론트가 status 보고 재구독. **→ 결정 필요, 백엔드 LLD §9 개정.**
- **replay 리셋:** 새 세션 시작이므로 replay buffer 리셋(이전 출력은 끝났음 표시).
- **프론트 상태:** `Restarting` 상태 추가 또는 Exited→Running 재진입 허용을 **frontend-integration-lld 개정**.
→ 이 절 미정의 시 재시작이 기존 확정 설계 2개와 충돌. **코어 구현 전 양쪽 LLD 개정 선행.**

## 7. Spike (3자 통합 — 코어 전 실측, fork가 성립 좌우)

| # | 항목 | 출처 | 우선 |
|---|------|------|------|
| 1 | `--session-id <uuid>` spawn 정상 / 재사용 시 append vs error | 원안 | 高 |
| 2 | `--resume <없는/손상 uuid>` exit·stderr (fallback 신호) | 원안 | 高 |
| 3 | `--resume` 다른 크기 PTY redraw | 원안/GPT | 中 |
| **4** | **resume 왕복 2회 — session id 유지 vs fork** | **fable** | **최우선(성립 좌우)** |
| 5 | `--resume <old> --session-id <new>` 조합 가능 여부 | fable | 高(되면 근본해결) |
| 6 | 신규 cwd trust 프롬프트에 복원이 Hang | fable | 高 |
| 7 | 로그인 만료 N개 동시 — 대기 좀비 | fable/Gemini | 中 |
| 8 | cwd 정규화(`I:\`vs`i:\`/후행) resume 영향 | fable | 中 |
| 9 | 동시 N spawn stagger / 동일 session-id 동시 PTY 2개 오염 | fable/Gemini | 中 |

서버측 세션 만료 반응(Gemini)도 #2/#7 계열로 관찰.

### 7-R. Spike 실측 결과 (2026-06-11 완료 — `spike-results.md`)
| # | 결과 |
|---|------|
| 1 | `--session-id <uuid>` 정상, json에 우리 uuid 반환 ✓. 재사용은 exit 1 `already in use` |
| 2 | `--resume <없는uuid>` exit 1 `No conversation found` ✓ — fallback 신호 명확(Hang 아님) |
| **4** | **`--resume` fork 안 함** — session id 유지 + append ✓ (**후퇴선 §8 불필요**) |
| /clear | **새 sessionId 생성**, `sessions/<pid>.json`에서 실시간 갱신 ✓ |
| 핵심 | **`~/.claude/sessions/<PID>.json` = {pid, sessionId, status, …}** — PID로 현재 sessionId 결정적 조회 |
| trust | 이 환경 미발생(신뢰됨). 새 머신선 가능 — 운영 주의 |
| 3·redraw | 경미(우리가 cols/rows 관리) |

## 8. 후퇴선 — **불필요 (spike #4로 fork 안 함 확인)**

`--resume`가 fork하지 않음이 실측됨 → "fresh-new-id 후퇴" 시나리오 자체가 발생 안 함. (단 transcript 디렉토리 최신파일 탐색으로 sid 추정하는 길은 여전히 금지 — sessions/<pid>.json이 결정적 대체.)

## 9. 진행 순서

1. ✅ Spike 실측 완료 (§7-R)
2. 🔄 §6 상태머신 개정안 확정 → 백엔드·프론트 LLD 개정
3. ⏳ 코어 구현 (프로필/persist/복원/재시작)

## 10. 방어 원칙 (wezterm 교훈 — 유지)

위치 무관 AgentId / 직접 spawn 추적 / 세션ID 분리통제 / 레이아웃↔세션 분리 / 복원실패 graceful(새id fallback + 보존 명시).

## 11. sid 확보 메커니즘 (spike 확정 — 최종)

**핵심: 최초는 우리가 지정(캡처 불필요), 변경 시에만 sessions/<pid>.json으로 결정적 재확보.**

### 11-1. 최초 sid — spawn 시
- `AgentId`(불변 키) + `claude_session_id`(초기 uuid) **둘 다 우리 생성**
- `claude --session-id <claude_session_id>`로 spawn → **우리 지정값으로 시작.** wezterm식 "claude 생성 sid 사후 캡처" 불필요 (비결정성 원천 제거)
- child PID 확보 (PtyManager 보유 — 휘발성 핸들, 저장 안 함)

### 11-2. sid 추적 — 실행 중 (변경 감지)
- `~/.claude/sessions/<현재 child_pid>.json` 한 파일 watch (create/modify)
- `sessionId`가 우리가 아는 `claude_session_id`와 달라지면(=`/clear`·fork) → `claude_session_id` 갱신, 옛 값 `old_session_ids` 이력
- **PID 불안정 무관** — 저장하지 않고, 현재 살아있는 child PID는 PtyManager가 항상 보유 → 그때그때 조회

### 11-3. 복원 — 재시작/앱 시작
- 저장된 `claude_session_id`(최신)로 `claude --resume <id>`
- 실패(exit 1 `No conversation found`) → **새 uuid로 fresh fallback** + 옛 id 이력 + `agent-restore-result` UI 알림("새 세션, 이전 대화 보존됨")
- 복원 후 새 child PID로 11-2 재개

### 11-4. 방어 (best-effort)
- `sessions/<pid>.json`은 **비공식 내부 파일**(version 필드 존재 → 포맷 변동 가능) → 없거나 바뀌어도 **최소 "최초 지정값" 유지** → 복원 시도 → 실패 시 fallback. **어느 경우든 무손상.**
- PID 재사용 → `startedAt`/`updatedAt`으로 우리 프로세스 검증
- claude 버전 최신 유지 (구버전 PTY 세션 미저장 버그 — release notes 관찰)
