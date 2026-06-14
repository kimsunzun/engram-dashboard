# Engram Dashboard

Tauri v2 + React 19 + Rust(portable-pty) 기반 **Claude 에이전트 관리 네이티브 대시보드**.
여러 claude(추후 codex·API) 에이전트를 PTY로 띄우고, xterm 터미널·트리·diff로 한 화면에서 관리한다.

이 파일은 대시보드 폴더에서 claude를 실행할 때의 프로젝트 컨텍스트다. 작업 전 아래 **아키텍처 원칙(불변)**을 반드시 깐다.

## 현재 상태 (2026-06-12) — 상세: `docs/README.md`, 타임라인: `docs/process/step-log.md`

- **백엔드 코어 완성** — PTY spawn/drain/kill, subscribe/replay, Job Object, 로깅(키 마스킹), headless 테스트.
- **S9 세션 저장/복원 완성** — 프로필 영속화 + claude 세션 무손실 복원(`--session-id`/`--resume`) + sid drift 추적.
- **S10 백엔드 추상화 완성** — `AgentManager → AgentSession(OutputCore) → dyn AgentTransport`. seam을 `OutputEvent`/`InputEvent`/`CommandSpec`/`Capabilities`로 인터페이스화(교체 가능). PtyTransport(콘솔) 구현 + ApiTransport 껍데기 + codex/gemini backend stub. **회귀 0**(unit test 38 / headless·smoke PASS / fable 게이트 GO).
- **프론트 통합 3a~3c** — 실제 PTY ↔ xterm E2E. 3d(popup+monaco)·복원 UX는 보류.
- **다음** — codex/gemini CLI spike(플래그 확정→variant 라우팅), (게이트) 자동 재시작, 실제 claude 복원 E2E spike, 메시지 시스템, ApiTransport 내부.

검증 흐름: 코딩 → fable LLD 리뷰 → QA 3-게이트. **QA는 build/test + GUI 실측(`scripts/cdp.mjs` eval/shot)을 항상 포함**한다 — 코드(test/tsc)가 통과해도 실제 화면에서 동작을 확인하기 전엔 미완으로 본다.

---

## ★ 아키텍처 원칙 (불변 — 아키텍트 구상 시 반드시 고려) ★

> **모든 기능은 추상 인터페이스 위에 구현하고, 내부 구현체는 교체(swappable)되는 형태로 짠다.**
> 특정 모델·전송 방식에 코드를 묶지 않는다. 이게 이 프로젝트를 10년 끌고 가는 법칙이다.
>
> **모든 시스템·메뉴는 LLM이 제어 가능해야 한다. LLM이 메인 조작 주체, 사용자의 직접 UI 조작은 서브다.** (§5)

### 0. 판단 기준 — 위험도 낮으면 over-engineering 쪽으로
이 프로젝트는 **장기(10년) 유지보수**가 전제다. 그래서 추상화 결정은 단순 YAGNI가 아니라 **위험도 × 기간**으로 판단한다:
- **저위험 + 장기** (인터페이스 경계, seam, 타입 enum 등 나중에 바꾸면 비싼 것) → **지금 충분히 깐다(over-engineering 허용).** 리팩터 비용이 크고 미래가 확실하면 미리 짓는 게 옳다.
- **고비용·불확실** (실제 동작을 모르는 백엔드 내부, 검증 안 된 가정) → **껍데기/정의만 두고 실측 때 채운다.**
- 예: `OutputEvent`/`InputEvent` seam·capability 영역 구조·콘솔 백엔드 3종은 지금 만든다. API transport 내부·semantic event log는 껍데기만(API 모델 등장 때). 상세: `docs/process/S10-backend-abstraction/`.

### 1. 출력/상태 계약 — `OutputSink` / `StatusSink` (이미 구현, load-bearing seam)
PTY 프로세스든 HTTP API든 모바일 WebSocket이든, **출력·상태는 이 trait으로만 흐른다.** 코어(`pty/`)는 Tauri·전송 방식을 모른다. 그래서 headless 테스트가 가능하고, 새 전송 경로는 sink 구현만 추가하면 흡수된다.

### 2. 세션 런타임 비전 — `AgentSession` 단일 인터페이스 + capability 매트릭스
모든 백엔드가 **같은 인터페이스**(start/write_input/resize/kill/output)를 구현한다. 차이는 구조가 아니라 **"이 capability를 지원하냐 마냐"** 뿐이다. **출력은 종류를 가정하지 않는다(터미널 강제 금지)** — `OutputEvent`(터미널 바이트는 한 variant일 뿐, API는 TextDelta/Usage/ToolCall 등)와 `capabilities.output`(terminal_bytes/markdown/tool_events/usage)로 **종류를 구분**하고, 슬롯이 그에 맞는 렌더러를 고른다(터미널=xterm / API=구조화·마크다운 뷰). ~~"API도 가상 터미널에 물려 같은 바이트 스트림으로"~~ 옛 가정은 S10 OutputEvent 결정으로 폐기 — API는 터미널이 아닐 수 있다.

| capability | claude_console | codex_console | codex_api |
|---|---|---|---|
| resume(세션 재개) | ✅ `--resume` | ?(실측 필요) | 방식 다름 |
| resize | ✅ PTY resize | ✅ | ❌(무의미·no-op) |
| 모델/옵션 다수 | ❌ | ❌ | ✅(API 전용) |

프론트는 이 매트릭스로 **지원 안 되는 옵션을 회색 처리**한다. capability 목록은 지금 다 채우지 말 것 — resume/resize만 확실하고, "API 옵션"은 codex_api 실측 때 늘린다.

### 3. 백엔드별 지식 격리 — `backend/` (S10 완료)
claude 전용 세션 인자 조립(`--session-id`/`--resume`)은 `pty/backend/claude.rs` **한 곳**에만 있다. manager는 `backend::needs_session()`/`build_command_spec()` dispatch만 부르고 claude/codex를 직접 모른다. `CommandSpec{program,args,env,cwd}`만 transport에 주입 — PtyTransport도 어느 백엔드인지 모른다. 현 구조:
```
pty/backend/
  mod.rs       — trait AgentBackend + dispatch(backend_for: AgentCommand→backend)
  claude.rs    — ClaudeBackend (--session-id/--resume)
  shell.rs     — ShellBackend (범용 패스스루)
  codex.rs/gemini.rs — stub(best-guess 플래그, dispatch 미연결)
```
**다음(YAGNI 해제 조건):** codex/gemini CLI 실측 spike 후 `AgentCommand`에 Codex/Gemini variant 추가 + `backend_for` 라우팅 연결. API 백엔드는 `transport/api.rs`(ApiTransport)가 같은 `AgentTransport` trait로 끼워짐 — 내부 HTTP 스트림만 채우면 됨.

### 4. 코어 격리 규칙
- `pty/` 하위 **tauri import 0** (`rg "use tauri" src/pty/` → 0줄 유지).
- `AppState { manager: Arc<AgentManager> }` — 외부 Mutex 없음.

### 5. LLM-우선 제어 — 모든 메뉴가 프로그래밍 가능해야 한다 (불변)
Engram의 **모든 기능은 LLM이 제어 가능**해야 한다. 백엔드(spawn/kill/write/interrupt 등)뿐 아니라 **UI/레이아웃 동작 전부 — 화면 분할, 슬롯 배치, 레이아웃 저장/복원, 에이전트 트리 추가·이동, diff accept/revert, 테마 전환 등 모든 메뉴**가 프로그래밍 가능한 제어 표면을 가져야 한다. **LLM이 메인 조작 주체이고, 사용자의 직접 UI 조작은 보조(편의)일 뿐이다.**
- **손발/두뇌 분리(핵심 멘탈모델):** 프론트엔드는 **순수 I/O**(출력 표시 + 입력 캡처)만 가지며 **제어를 소유하지 않는다(렌더링만 소유)**. 모든 기능은 **백엔드측 LLM(감독자=두뇌)** 이 쥐고 휘두르는 "핸들(자루)"로 노출된다. 사람의 UI 클릭은 같은 핸들을 대신 흔드는 **보조 입력**일 뿐이다. 그래서 프론트 액션의 핸들도 **프론트 내부 전용이면 안 되고 백엔드측 LLM이 닿을 수 있어야** 한다(프론트는 손발, 백엔드측 LLM이 두뇌). 죽음 감지·재시작 같은 감독도 프론트가 아니라 백엔드측에서 판단한다.
- **함의(현 갭):** 백엔드 동작은 Tauri command(`invoke`)로 이미 LLM 제어 가능. 그러나 **UI/레이아웃 동작은 현재 프론트(Zustand store) 전용 — LLM 제어 표면이 없다.** 새 UI 기능을 추가할 땐 반드시 그 동작을 LLM이 호출할 경로(command / 이벤트 버스 / 문서화된 JS 제어 API)를 **함께** 만든다. "UI 먼저, 제어는 나중"은 이 원칙 위반.
- **설계 지향:** UI 컴포넌트는 상태 store의 액션을 호출만 하고, 그 액션들은 LLM도 동일하게 부를 수 있는 단일 control surface(예: 의도 단위 command 버스)로 모은다. 사람 클릭과 LLM 호출이 같은 진입점을 거치게 = 두 조작 주체가 같은 모델을 본다.
- **임시 경로:** 정식 제어 표면 전까지는 `scripts/cdp.mjs eval`이 WebView에서 임의 JS·invoke를 실행할 수 있어 LLM 제어/검증의 임시 수단이 된다.

---

## 백엔드 모듈 맵 (`crates/engram-dashboard-core/src/` — S12 phase 1 이동)
> 아래 `pty/`·`persistence/`·`logging/`는 S12 phase 1에서 `src-tauri/src/`→`crates/engram-dashboard-core/src/`로 이동(git mv, history 보존). 내부 `crate::` 경로는 무수정(코어 crate 의 top-level 모듈). `src-tauri`는 `engram_dashboard_core::{pty,persistence,logging}`로 re-import. wire 계약은 `crates/engram-dashboard-protocol`(AgentCommand/AgentEvent/OutputChunk/codec, ts-rs).

```
pty/                          # S10 추상화: AgentManager → AgentSession(OutputCore) → dyn AgentTransport
├── types.rs          # AgentStatus/PtyEvent/AgentInfo(+epoch+capabilities)/OutputSink·StatusSink
│                     #  + 중립 seam: OutputEvent/InputEvent(확장 enum)·TerminalReason·CommandSpec·Capabilities·OutputChunk
├── profile.rs        # AgentProfile/AgentCommand/SpawnMode/RestoreOutcome + ProfileRegistry(sid 단일소유자)
├── output_core.rs    # ★OutputCore★ seq/replay/subscribers/status/finalize — emit(variant-agnostic)/finish(finalize 1회)/join_pump/enter_exiting/subscribe(C4). transport 무관 공용 1벌
├── session.rs        # AgentSession(구체) = OutputCore + Box<dyn AgentTransport> 합성. write_input/resize/interrupt/kill(=shutdown+join_pump)/capabilities
├── transport/
│   ├── mod.rs        # trait AgentTransport (start/send_input/resize/interrupt/shutdown/capabilities) — seam
│   ├── pty.rs        # PtyTransport(콘솔 공용) — master/writer/child/shutdown/job + pump 스레드. kill 1~5단계·drain 흡수
│   └── api.rs        # ApiTransport 껍데기 — 전부 Unsupported, capabilities false (HTTP는 API 모델 때)
├── backend/          # CommandSpec 산출(transport는 claude/codex 모름)
│   ├── mod.rs        # trait AgentBackend + dispatch(needs_session/build_command_spec/backend_for)
│   ├── claude.rs     # ★claude 인자 조립 격리★ --session-id/--resume
│   ├── shell.rs      # 범용 셸 패스스루
│   └── codex.rs/gemini.rs # stub(best-guess 플래그, dispatch 미연결 — CLI spike 후 variant 추가)
├── session_tracker.rs# sid drift 폴링(best-effort, PID shim 우회, 단일스레드+정지핸들)
└── platform/windows.rs # JobObjectHandle (KILL_ON_JOB_CLOSE)
persistence/mod.rs    # FileProfileStore — agents.json atomic(tmp+sync_all+rename+fsync)
logging/mod.rs        # tracing + 런타임 레벨 토글 + 키/토큰 마스킹(기본 OFF=warn)
commands/             # Tauri thin wrapper (agent/pty/profile, interrupt_agent) — 비즈니스 로직 없음
lib.rs                # AppState 배선, TauriStatusSink/ChannelOutputSink, 부팅 시 백그라운드 restore_all
examples/             # headless.rs(manager 전체) · transport_smoke.rs/session_smoke.rs(신경로 직접 실측)
```

### 핵심 불변식 (변경 금지)
- **kill 인과(2동사):** `transport.shutdown()`(shutdown flag Release → child.kill+wait → TerminateJobObject → master.take/drop) → `core.join_pump(5s)`. master drop → reader EOF → pump break → `core.finish(reason)` → done_tx. = 옛 6단계 동치.
- **finalize 1회:** `OutputCore.finalized.swap(AcqRel)` — terminal 전이/알림 정확히 1회(pump 단독).
- **락 순서(§10):** sessions RwLock은 Arc clone 후 즉시 해제 → 그 뒤 session 내부 접근. status lock 보유 중 외부 호출 금지(finish/enter_exiting는 lock 해제 후 status_changed). emit의 send 시 subscribers clone 후 lock 미보유.
- **상태 알림 분담:** 과도기 `Exiting` = manager(`session.enter_exiting()` 트리거), terminal(`Killed`/`Exited`/`Failed`) = pump(`core.finish`) 단독. 프론트는 status_changed로 terminal 판정 금지, `agent-list-updated`(목록)로 판정.
- **replay→live:** subscribers lock 보유 중 replay 전송(C4, 순서 역전 방지) + 프론트 seq dedup.
- **epoch:** 같은 AgentId 맵 교체(restart/fresh fallback)마다 +1 → 프론트 `[agentId, epoch]` 재구독 트리거.
- **소유권 분할:** transport=master/writer/child/shutdown/job · core=subscribers/replay/seq/status/finalized/drain_handle · session=id/cwd/epoch/cols/rows.

### S9 세션 복원 메커니즘 (핵심)
spawn 시 `--session-id <uuid>`로 **우리가 sid를 통제** → 재시작 `--resume <uuid>`로 무손실 복원. `/clear`로 sid가 바뀌면 `~/.claude/sessions/<pid>.json`을 폴링해 따라잡아 **즉시 persist**. resume 조기 종료(윈도 3s 내 terminal) = 실패 → 새 sid로 fresh fallback(종점 Failed, 재귀 금지).
**복원 정확성은 우리가 통제하는 sid에만 의존한다.** `sessions/<pid>.json` 추적은 best-effort 등급 — 못 읽어도 무손상 강등(추적만 끔). 이 파일로 기능을 *확장*하지 말 것.

---

## 의존성 (확정 — 변경 시 보고)
- `tauri = "2"` (최신 2.x — Channel 무손실 Windows 실측 확인, spike)
- `portable-pty = "0.8.1"` · `uuid` · `thiserror` · `base64` · `regex`(로그 마스킹) · `tracing` · `dunce`(cwd canonicalize UNC 회피)
- `windows` (Job Object) — `#[cfg(windows)]`

## 빌드·검증 명령 (Cargo workspace — S12 phase 1 이후 루트에서 실행)
S12 phase 1 이후 **Cargo workspace**: 루트 `Cargo.toml`(멤버 `crates/engram-dashboard-protocol`·`crates/engram-dashboard-core`·`src-tauri`). 코어(pty/persistence/logging)는 `crates/engram-dashboard-core`로 이동, examples도 거기로. `target/`는 워크스페이스 루트.
- `cargo test -p engram-dashboard-core --lib` — 코어 unit test (현재 38건)
- `cargo test -p engram-dashboard-protocol` — protocol codec golden + ts-rs 바인딩 (현재 21건)
- `cargo run -p engram-dashboard-core --example headless` — **프론트 없이** 백엔드 spawn→write→resize→kill 로그 검증
- `cargo run -p engram-dashboard-core --example transport_smoke` / `session_smoke` — manager 없이 PtyTransport/AgentSession 직접 실측
- `cargo build` (루트) — 전체 workspace 빌드
- `cargo fmt` / `rg "use tauri" crates/engram-dashboard-core/src/` (→ 0줄) — 포맷·격리 게이트
- 프로젝트 루트: `npm run tauri dev` — 전체 E2E
- 로그 ON: `RUST_LOG=debug` (기본 OFF=warn)

### GUI 시각/동작 검증 (`scripts/cdp.mjs`) — 실제 앱을 코드로 확인
실제 Tauri 창(WebView2)에 **CDP로 직접 붙어** 스크린샷·DOM 조회·실제 `invoke` 호출까지 한다. MCP·새 세션·재시작 불필요(node 내장 WebSocket만). **Windows 전용**(WebView2). 절차:
```bash
# 1) 디버그 포트 열고 앱 실행 (bash: env var 붙여 백그라운드)
WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223" npm run tauri dev
# 2) 포트 뜰 때까지 대기: curl http://127.0.0.1:9223/json/version
# 3) 검증
node scripts/cdp.mjs info                 # 페이지 목록
node scripts/cdp.mjs shot out.png          # 스크린샷 → Read로 확인
node scripts/cdp.mjs eval "<js>"           # 앱 안에서 JS 실행(결과 JSON 출력)
```
`eval`로 DOM 텍스트(`document.body.innerText`)나 **백엔드 직접 호출**(`window.__TAURI__.core.invoke('spawn_agent',{cwd})` 등) 가능 → spawn/write/interrupt/kill 전 경로를 실제 IPC로 검증.
포트는 9223 고정(9222는 Gemini 자동화 Chrome — 충돌 회피). 다른 포트는 `CDP_PORT` env.
검증 목적이면 스샷보다 `eval` 텍스트가 토큰·정확도 유리(픽셀 해석 회피). S10 GUI E2E를 이 방식으로 회귀 0 확인함(2026-06-12).

## 커뮤니케이션

사용자 응답 시 생소할 만한 전문용어는 그대로 쓰되, 바로 옆이나 밑에 쉬운 한 줄 풀이를 단다. 단 모든 용어가 아니라 사용자가 막힐 만한 것만 — 흔한 용어까지 풀면 장황해진다. 코드·파일명·경로는 영문 그대로 둔다.

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
api/        types.ts(백엔드 타입 미러+epoch/AgentProfile/RestoreOutcome) · ptyApi.ts(invoke 저수준 래퍼) · decodeBase64.ts
            agentClient.ts(★제어 표면 인터페이스) · embeddedClient.ts(in-process 구현, invoke/Channel 캡슐화) · clientFactory.ts(싱글톤+window.__ENGRAM_AGENT__ 노출, phase4 mode 토글 자리)
            ※ 컴포넌트·스토어는 agentClient 인터페이스만 의존(ptyApi 직접 호출 X). DaemonClient(WS)는 phase4에 동일 인터페이스로 추가.
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
