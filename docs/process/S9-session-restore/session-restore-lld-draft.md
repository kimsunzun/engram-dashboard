# 세션 저장/복원 + 에이전트 프로필 + 자동 재시작 — 설계 초안 (검토용)

상태: **초안.** fable / 웹 Gemini / 웹 GPT 3자 검증 대기. 전제: `wezterm-failure-analysis.md`.
범위: ① 세션 저장/복원 + spawn 설정화(에이전트 프로필) + ② 프로세스 자동 재시작. (메시지 시스템 제외)

## 0. 핵심 결정 (검토 포인트)

**AgentId(uuid v4) = claude `--session-id`로 통합.** 우리가 UUID를 생성해 spawn 시 `claude --session-id <AgentId>`로 지정 → claude 세션 ID를 우리가 통제. 복원은 `claude --resume <AgentId>`. 이로써 wezterm 실패의 비결정성/위치매핑을 근본 회피.

## 1. 데이터 모델

```rust
// 에이전트 프로필 — spawn 단위이자 persist 단위
struct AgentProfile {
    id: AgentId,            // = claude --session-id (uuid v4, 우리 생성)
    name: String,           // 표시명 (사용자 지정)
    command: AgentCommand,  // 무엇을 띄우나
    cwd: PathBuf,
    env: Vec<(String,String)>,
    created_at, last_active, // 메타
}

enum AgentCommand {
    Claude { resume: bool },          // claude (--session-id) or claude --resume
    Shell  { program: String, args: Vec<String> },  // cmd/pwsh/커스텀
}
```
- spawn 설정화(③) = `AgentCommand`로 "무엇을". cwd로 "어디서". (현재 claude 하드코딩 대체)

## 2. Persist (저장)

- **무엇:** `AgentProfile` 목록만. (claude 대화 자체는 claude가 `~/.claude/projects/`에 저장 — 우리는 안 건드림)
- **형식:** JSON 파일 (`<app_data>/agents.json`). 단순·검사 용이. SQLite는 과함(에이전트 수십 개 규모).
- **시점:** 프로필 변경 시(spawn/kill/rename) 즉시 저장 + 앱 종료 시. (주기 저장은 불필요 — 프로필은 저빈도 변경)
- **레이아웃 분리:** 슬롯↔AgentId 매핑은 **프론트(slotStore) 책임**. 백엔드는 프로필만. (wezterm 위치결합 함정 회피)

## 3. spawn (프로필 기반)

```
spawn_agent(profile):
  id = profile.id (없으면 uuid v4 생성)
  args = match profile.command {
    Claude{resume:false} => ["claude", "--session-id", id]
    Claude{resume:true}  => ["claude", "--resume", id]
    Shell{program,args}  => [program, ...args]
  }
  PTY spawn (cwd, env) → 기존 PtyManager.spawn_agent 흐름
```
- 기존 `spawn_agent(cwd)` → `spawn_agent(profile)`로 확장. PtySession/drain/manager 코어는 불변(인터페이스만).

## 4. 복원 (앱 시작)

```
restore_all():
  profiles = load(agents.json)
  for p in profiles:
    p.command = Claude{resume:true}   // 복원 모드
    spawn_agent(p)
    // claude --resume <id> 실패 시 → fallback (아래)
```
- **fallback (방어 — claude 복원 100% 불신):** `--resume`가 실패(세션 부재/손상)하면 자동으로 `--session-id <id>`(fresh)로 재spawn. 우리 프로필 메타는 보존되므로 에이전트는 살아남고 대화만 새로 시작.
- **실패 감지 신호:** spike로 확정 필요 — `--resume <없는id>`의 exit code/stderr/행 여부 (불확실 ②).

## 5. 자동 재시작 (②)

```
정책: AgentProfile.restart_policy = { Never | OnCrash | Always }, max_retries
drain thread가 terminal 전이 감지 (Exited{code≠0} or 비정상) →
  manager가 정책 확인 → OnCrash & code≠0 & retries<max →
    claude --resume <id> 로 재spawn (대화 이어가기)
```
- 정상 종료(code=0, 사용자 kill)는 재시작 안 함.
- 재시작도 `--resume`라 대화 유지. 실패 시 fresh fallback.
- 무한 재시작 방지: max_retries + backoff.

## 6. spike 필요 (wezterm 교훈 — 실측 우선)

본 구현 전 Windows 실측:
1. `claude --session-id <uuid>` spawn → 정상 동작? 같은 uuid 재spawn 시 append/error? (불확실 ①)
2. `claude --resume <없는/손상 uuid>` → exit code/stderr (fallback 트리거 확정, 불확실 ②)
3. `claude --resume <id>` 다른 크기 PTY → TUI redraw 정상? (불확실 ③)

## 7. 방어 원칙 (wezterm-failure-analysis §3 적용)

1. 세션 = AgentId(위치 무관) ✅ — 슬롯 인덱스에 안 묶음
2. claude 추적 = 우리 spawn child PID/AgentId ✅ — 프로세스명(node) 감지 안 함
3. 세션 ID = `--session-id`로 우리가 통제 ✅ — 비결정성 제거
4. 레이아웃↔세션 분리 ✅ — 프론트 슬롯 / 백엔드 프로필
5. 복원 실패 graceful ✅ — fresh fallback, 프로필 메타 보존

## 8. 검토 질문 (3자에게)

- AgentId=session-id 통합이 타당한가? (1 AgentId ↔ 1 claude 세션 가정의 허점은?)
- JSON persist + 변경시 저장 정책이 충분한가? (동시성/크래시 중 저장 손상?)
- 자동 재시작 `--resume`가 재시작 루프/대화 오염 위험은?
- fallback(resume 실패→fresh) 시 사용자가 "대화 날아감"을 어떻게 인지? (UX 신호)
- spike 3항목 외 놓친 claude CLI 함정은?
