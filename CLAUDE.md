# Engram Dashboard

Tauri v2 + React 19 + Rust(portable-pty) 기반 **Claude 에이전트 관리 네이티브 대시보드**.
여러 claude(추후 codex·API) 에이전트를 PTY로 띄우고, xterm 터미널·트리·diff로 한 화면에서 관리한다.

이 파일은 대시보드 폴더에서 claude를 실행할 때의 프로젝트 컨텍스트다. 작업 전 아래 **아키텍처 원칙(불변)**을 반드시 깐다.

## 현재 상태 (2026-06-12) — 상세: `docs/README.md`, 타임라인: `docs/process/step-log.md`

- **백엔드 코어 완성** — PTY spawn/drain/kill, subscribe/replay, Job Object, 로깅(키 마스킹), headless 테스트.
- **S9 세션 저장/복원 완성** — 프로필 영속화 + claude 세션 무손실 복원(`--session-id`/`--resume`) + sid drift 추적. unit test 19 / headless PASS / fable 리뷰 반영.
- **프론트 통합 3a~3c** — 실제 PTY ↔ xterm E2E. 3d(popup+monaco)·복원 UX는 보류.
- **다음** — (게이트) 자동 재시작, 실제 claude 복원 E2E spike, 메시지 시스템, codex/API 백엔드.

검증 흐름: 코딩 → fable LLD 리뷰 → QA(build/test) 3-게이트.

---

## ★ 아키텍처 원칙 (불변 — 아키텍트 구상 시 반드시 고려) ★

> **모든 기능은 추상 인터페이스 위에 구현하고, 내부 구현체는 교체(swappable)되는 형태로 짠다.**
> 특정 모델·전송 방식에 코드를 묶지 않는다. 이게 이 프로젝트를 10년 끌고 가는 법칙이다.

### 0. 판단 기준 — 위험도 낮으면 over-engineering 쪽으로
이 프로젝트는 **장기(10년) 유지보수**가 전제다. 그래서 추상화 결정은 단순 YAGNI가 아니라 **위험도 × 기간**으로 판단한다:
- **저위험 + 장기** (인터페이스 경계, seam, 타입 enum 등 나중에 바꾸면 비싼 것) → **지금 충분히 깐다(over-engineering 허용).** 리팩터 비용이 크고 미래가 확실하면 미리 짓는 게 옳다.
- **고비용·불확실** (실제 동작을 모르는 백엔드 내부, 검증 안 된 가정) → **껍데기/정의만 두고 실측 때 채운다.**
- 예: `OutputEvent`/`InputEvent` seam·capability 영역 구조·콘솔 백엔드 3종은 지금 만든다. API transport 내부·semantic event log는 껍데기만(API 모델 등장 때). 상세: `docs/process/S10-backend-abstraction/`.

### 1. 출력/상태 계약 — `OutputSink` / `StatusSink` (이미 구현, load-bearing seam)
PTY 프로세스든 HTTP API든 모바일 WebSocket이든, **출력·상태는 이 trait으로만 흐른다.** 코어(`pty/`)는 Tauri·전송 방식을 모른다. 그래서 headless 테스트가 가능하고, 새 전송 경로는 sink 구현만 추가하면 흡수된다.

### 2. 세션 런타임 비전 — `AgentSession` 단일 인터페이스 + capability 매트릭스
모든 백엔드가 **같은 인터페이스**(start/write_input/resize/kill/output)를 구현한다. 차이는 구조가 아니라 **"이 capability를 지원하냐 마냐"** 뿐이다. API 백엔드도 출력을 **가상 터미널에 물려** 같은 바이트 스트림으로 흐르게 하므로, 렌더링·런타임 경로가 하나로 통일된다.

| capability | claude_console | codex_console | codex_api |
|---|---|---|---|
| resume(세션 재개) | ✅ `--resume` | ?(실측 필요) | 방식 다름 |
| resize | ✅ PTY resize | ✅ | ❌(무의미·no-op) |
| 모델/옵션 다수 | ❌ | ❌ | ✅(API 전용) |

프론트는 이 매트릭스로 **지원 안 되는 옵션을 회색 처리**한다. capability 목록은 지금 다 채우지 말 것 — resume/resize만 확실하고, "API 옵션"은 codex_api 실측 때 늘린다.

### 3. 백엔드별 지식 격리 — `claude.rs`(→ `backend/claude_console.rs`로 이전 예정)
claude 전용 세션 인자 조립(`--session-id`/`--resume`)은 `pty/claude.rs` **한 곳**에만 있다. manager는 claude를 직접 몰라야 한다. 목표 구조:
```
pty/backend/
  mod.rs            — trait AgentBackend (build_command / injects_session_id / tracking)
  claude_console.rs — 현재 claude.rs
  codex_console.rs  — (추후) PTY + codex CLI
backend_api/codex_api.rs — (추후) 가상 터미널에 물리는 API 백엔드, 같은 AgentSession 구현
```
**현재 상태:** `manager.rs`가 `claude::needs_session()`/`build_command()`를 3곳에서 직접 호출(L110/L116/L261). codex가 실제로 붙을 때 이 3곳을 trait dispatch로 승격한다. claude 지식이 이미 격리돼 있어 **국소 변경**이고 manager 본체는 안 바뀐다. (trait 전면화는 두 번째 백엔드 등장 시 — 그래야 올바른 경계가 잡힘. YAGNI)

### 4. 코어 격리 규칙
- `pty/` 하위 **tauri import 0** (`grep -r "use tauri" src/pty/` → 0줄 유지).
- `AppState { manager: Arc<PtyManager> }` — 외부 Mutex 없음.

---

## 백엔드 모듈 맵 (`src-tauri/src/`)

```
pty/
├── types.rs          # AgentId/AgentStatus/PtyEvent/AgentInfo(+epoch) + OutputSink/StatusSink trait
├── profile.rs        # AgentProfile/AgentCommand/SpawnMode/RestoreOutcome + ProfileRegistry(sid 단일소유자)
├── claude.rs         # ★claude 전용 인자 조립 격리★ build_command/needs_session
├── session.rs        # PtySession(필드별 독립 Mutex) + ReplayBuffer + subscribe(C4)
├── drain.rs          # drain thread(OS thread, send 시 lock 미보유) + terminal 전이
├── manager.rs        # PtyManager — spawn_agent(profile,mode)/restore_all/fallback/kill(6단계)/shutdown_all
├── session_tracker.rs# sid drift 폴링(best-effort, PID shim 우회, 단일스레드+정지핸들)
└── platform/windows.rs # JobObjectHandle (KILL_ON_JOB_CLOSE)
persistence/mod.rs    # FileProfileStore — agents.json atomic(tmp+sync_all+rename+fsync)
logging/mod.rs        # tracing + 런타임 레벨 토글 + 키/토큰 마스킹(기본 OFF=warn)
commands/             # Tauri thin wrapper (agent/pty/profile) — 비즈니스 로직 없음
lib.rs                # AppState 배선, TauriStatusSink/ChannelOutputSink, 부팅 시 백그라운드 restore_all
examples/headless.rs  # 프론트 없이 백엔드 전체 흐름 로그 검증
```

### 핵심 불변식 (변경 금지)
- **kill 6단계:** `shutdown(Release) → child.kill → wait → TerminateJobObject → master.take → drain_done recv_timeout(5s)`.
- **락 순서(§10):** sessions RwLock은 Arc clone 후 즉시 해제 → 그 뒤 session 내부 lock. status lock 보유 중 외부 호출 금지. drain의 send 시 어떤 lock도 미보유.
- **상태 알림 분담:** 과도기 `Exiting` = manager, terminal(`Killed`/`Exited`/`Failed`) = drain 단독. 프론트는 status_changed로 terminal 판정 금지, `agent-list-updated`(목록)로 판정.
- **replay→live:** subscribers lock 보유 중 replay 전송(C4, 순서 역전 방지) + 프론트 seq dedup.
- **epoch:** 같은 AgentId 맵 교체(restart/fresh fallback)마다 +1 → 프론트 `[agentId, epoch]` 재구독 트리거.

### S9 세션 복원 메커니즘 (핵심)
spawn 시 `--session-id <uuid>`로 **우리가 sid를 통제** → 재시작 `--resume <uuid>`로 무손실 복원. `/clear`로 sid가 바뀌면 `~/.claude/sessions/<pid>.json`을 폴링해 따라잡아 **즉시 persist**. resume 조기 종료(윈도 3s 내 terminal) = 실패 → 새 sid로 fresh fallback(종점 Failed, 재귀 금지).
**복원 정확성은 우리가 통제하는 sid에만 의존한다.** `sessions/<pid>.json` 추적은 best-effort 등급 — 못 읽어도 무손상 강등(추적만 끔). 이 파일로 기능을 *확장*하지 말 것.

---

## 의존성 (확정 — 변경 시 보고)
- `tauri = "2"` (최신 2.x — Channel 무손실 Windows 실측 확인, spike)
- `portable-pty = "0.8.1"` · `uuid` · `thiserror` · `base64` · `regex`(로그 마스킹) · `tracing` · `dunce`(cwd canonicalize UNC 회피)
- `windows` (Job Object) — `#[cfg(windows)]`

## 빌드·검증 명령 (`src-tauri/`)
- `cargo test --lib` — unit test (현재 19건)
- `cargo run --example headless` — **프론트 없이** 백엔드 spawn→write→resize→kill 로그 검증
- `cargo fmt` / `grep -r "use tauri" src/pty/` (→ 0줄) — 포맷·격리 게이트
- 프로젝트 루트: `npm run tauri dev` — 전체 E2E
- 로그 ON: `RUST_LOG=debug` (기본 OFF=warn)

## 컨벤션
- 중요 로직(동시성·kill·unsafe·비자명한 결정)에 **왜** 그런지 한국어 주석. 자명한 코드엔 주석 금지.
- 자격증명을 `profile.env`에 넣지 말 것(agents.json 평문 저장 — persistence가 경고).
- 모듈마다 build/test/커밋. 커밋 메시지 끝에 Co-Authored-By 트레일러.

---

## 기술 스택 (프론트)

| 레이어 | 선택 |
|---|---|
| 앱 껍데기 | Tauri v2 (창 + invoke) |
| UI | React 19 + TypeScript + Vite |
| 스타일 | CSS 변수 (Tailwind 미사용) |
| 상태 | Zustand |
| 터미널 | @xterm/xterm + addon-fit |
| 패널 분할 | allotment |
| 에이전트 트리 | react-arborist |
| Diff | @monaco-editor/react |
| 라우팅 | react-router-dom (hash) |

## 프론트 파일 구조 (`src/`)

```
api/        types.ts(백엔드 타입 미러+epoch/AgentProfile/RestoreOutcome) · ptyApi.ts(invoke 래퍼+프로필 CRUD) · decodeBase64.ts
store/      agentStore.ts · slotStore.ts · themeStore.ts · eventBus.ts(Tauri 이벤트 1회 등록, agent-list-updated/status-changed/restore-result)
components/ layout/(AppLayout·Sidebar·StatusBar) · agent/AgentTree · slot/(SlotPane·TerminalSlot·SlotContextMenu) · diff/DiffPanel
pages/      PopupPage(/popup?slotId=N) · TreePage(/tree)
```

### 프론트 통합 규칙 (확정)
- TerminalSlot 구독 effect deps `[agentId, epoch]` — 재spawn 시 reset→재구독→replay(§18-e/f).
- C2 `terminal.reset()` 구독 전 / T-2 seq dedup / G-1 `delete channel.onmessage`(null 아님, #13133) / 입력 가드(terminal 상태) / resize debounce 50ms.

## 창 구성 (tauri.conf.json)
| label | 용도 | 기본 |
|---|---|---|
| main | 메인 대시보드 | visible |
| slot-popup | 슬롯 팝업 분리 | hidden, /popup?slotId=N |
| agent-tree | 트리 분리 | hidden, /tree |

## 테마 CSS 변수 (`data-theme` on `:root`: dark/light/e-ink)
`--bg` `--bg-secondary` `--text` `--text-muted` `--border` `--accent` + 폰트 `--font-ui/terminal/code/claude-*`. 값은 `src/styles/theme.css`·`font.css` 참조.
