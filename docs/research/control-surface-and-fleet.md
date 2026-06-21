# 연구노트: claude 제어 표면 + fleet 선행설계

- 성격: **연구노트(사실 수집)** — 결정 아님. 결정은 PRD/ADR로 따로. 백엔드 추상화(S10) 종료 후 "제어표면/fleet" 설계 진입 시 선행 참조용.
- 날짜: 2026-06-21 · 범위: claude CLI 프로그래밍 제어 + 다수(20+) 에이전트 호스팅.
- 신뢰도 표기: **[F]** 공식문서/코드 확인 · **[2]** 2차출처(블로그·포럼) · **[?]** 미검증/실측필요 · **[op]** 의견.
- 출처 URL은 맨 끝.

---

## 1. claude 제어 표면 — 4채널

제어 깊이: launch args(겉) → PTY 텍스트/슬래시(중간) → **stream-json/SDK(기계제어)** → config·hooks(행동규정).
현재 engram은 ①+②만 사용(PTY spawn + 텍스트 stdin). fleet·LLM제어의 핵심은 ③.

### ① Launch-time (CLI 인자 + env) — 세션 고정값
fleet/프로그래밍 제어에 직결되는 것만(전체 30+ 플래그는 cli-reference.md):
- 세션: `--session-id <uuid>`(우리가 sid 통제, 현행) · `--resume <id>` · `--continue` · `--fork-session`
- 모델/성능: `--model` · `--effort <low|medium|high|xhigh|max>` · `--fallback-model <list>`
- 권한모드(6): `default` · `acceptEdits` · `plan` · `auto`(분류기 자동심사, Opus4.6+) · `dontAsk`(allow rule만, CI 완전자동) · `bypassPermissions`(전부 스킵, 격리환경 전용)
- 도구: `--allowedTools "Bash(git *),Read"` · `--disallowedTools "mcp__*"` · `--tools`(context 노출 제한)
- 출력: `-p/--print`(headless) · `--output-format text|json|stream-json` · `--json-schema`(구조화 검증, json 필수) · `--include-partial-messages` · `--verbose`
- 입력: `--input-format text|stream-json` · `--replay-user-messages`
- **fleet 핵심**: `--bare`(hooks/skills/plugins/MCP/CLAUDE.md 자동발견 스킵 → 기동 빠름·메모리↓, CI용) · `--mcp-config`+`--strict-mcp-config`(MCP 게이팅) · `--max-turns` · `--max-budget-usd` · `--no-session-persistence`(전부 `-p` 한정) · `--exclude-dynamic-system-prompt-sections`(cwd/env 제외 → prompt cache 재사용↑)
- env: `ANTHROPIC_API_KEY` · `ANTHROPIC_MODEL` · `CLAUDE_CONFIG_DIR` · `CLAUDE_CODE_SAFE_MODE`(커스터마이즈 전체 OFF) · `CLAUDE_ENV_FILE`(hook이 env 주입)
- 한계: 런타임 only, 영속 기본값은 settings.json. session-id는 UUID 형식만.

### ② Runtime 텍스트 stdin — 슬래시/멘션 (= 현행 빨대)
`/resume` `/clear` `/compact [instr]` `/cd <path>`(CwdChanged hook) `/branch` `/config k=v` `/permissions` · `@file`·`@/dir` 멘션 · `!bash` · `Shift+Tab`(권한모드 순환).
- 한계: **전부 인터랙티브 전용**. headless(`-p`)에선 `/resume`·`/clear`·`/cd` 불가. 출력이 ANSI 텍스트 → 파싱 취약(현행 xterm 긁기의 근본 약점).

### ③ Runtime 구조화 — `-p --output-format stream-json` ★핵심★
NDJSON(줄단위 JSON) 양방향. 텍스트 긁기 대비: 이벤트 타입·메타(tool_use_id/cost/error)·token경계·ordering이 **구조적으로** 분리됨.

나가는 이벤트(주요): `system/init`(session_id·model·tools·mcp·plugins) · `assistant`(content_block_delta) · `tool_use`(tool_use_id·tool_name·tool_input) · `tool_result`(tool_use_id 매칭·output·error) · `stream_event`(token델타, `--include-partial-messages`) · `permission_request`(headless에선 stdout 직접응답 불가) · `system/api_retry`(rate-limit 백오프 직접구현 가능) · `structured_output`(`--json-schema`) · `result`(최종 result·session_id·usage·cost).

들어가는 메시지(`--input-format stream-json`): `{type:"user_message",content}` · `{type:"permission_response",tool_use_id,decision:"allow|deny|ask"}`.
- 한계: tool 실행은 sequential(parallel 불가) · 권한 prompt는 headless에서 hook으로 사전결정 필요 · 비-`-p` 대화형은 구조화 입력 미지원.
- **engram 함의**: CLAUDE.md `OutputEvent` variant-agnostic(터미널 vs 구조화)의 갈림길이 정확히 여기. `claude_console`을 PTY 텍스트 대신 stream-json으로 받으면 구조화 출력 확보.

### ④ Persistent config + hooks — 행동 규정
- 계층(낮→높): user `~/.claude/settings.json` → project `.claude/settings.json` → local `.claude/settings.local.json` → `--settings` → managed. 대부분 실시간 reload(`model`/`outputStyle`만 재시작).
- 키: `model`/`effortLevel`/`fallbackModel` · `permissions{defaultMode,allow[],deny[],additionalDirectories[]}`(rule 문법 `Tool(pattern)`, 가장 제한적 규칙 우승) · `env{}` · `enabledPlugins{}`.
- hooks: 이벤트 다수(lifecycle `SessionStart`/`SessionEnd`/`UserPromptSubmit`, tool `PreToolUse`/`PermissionRequest`/`PostToolUse`, `PreCompact`/`PostCompact`, subagent `SubagentStart`/`Stop`, `Stop`/`Notification` 등). 타입 `command`/`http`/`prompt`/`agent`/`mcp_tool`. I/O: stdin JSON, exit `0`=통과·`2`=차단(stderr→Claude)·기타=에러, 또는 `hookSpecificOutput{permissionDecision,updatedInput}`로 입력수정·권한결정.
- 차단/수정 가능범위: PreToolUse=차단+입력수정+권한 · PostToolUse=전파차단 · UserPromptSubmit/SessionStart=context 주입.
- `.claude/`: `commands/` · `agents/`(서브에이전트 정의) · `skills/` · `hooks/` · `.mcp.json`.

### Agent SDK(TS/Py) 매핑
- CLI 플래그 대부분 옵션으로 대응(`resume`·`model`·`allowedTools`·`permissionMode`·`mcpServers`·`hooks`). hooks는 **shell이 아닌 async 콜백**(spawn 오버헤드 0).
- SDK 추가 제어: `on_permission_request`(프로그래밍 승인) · `on_ask_user_question` · message 객체 직접접근(파싱 0) · 비동기 control loop.
- **결정적 사실 [F]**: SDK `query()`는 내부적으로 **claude CLI를 subprocess로 spawn**한다. SDK가 in-process인 건 오케스트레이션 코드지 LLM 루프가 아님 → 메모리는 headless(`-p`)와 동일. 단 **custom tool=SDK in-process MCP**는 MCP 자식 프로세스를 없앰(③ 참조).

---

## 2. Fleet 호스팅 (20+ 동시)

### 핵심 결론
1. **메모리 하한 = 프로세스 개수 × Node/V8 베이스라인.** PTY든 headless든 SDK든 인스턴스당 별도 Node 프로세스면 베이스라인은 안 줄어든다. 진짜 절감은 서브에이전트(프로세스 1개)뿐인데 **격리를 포기**한다. → **격리 vs 메모리는 트레이드오프** = PRD에서 사용자 결정감. [op]
2. **현행 PTY+풀 TUI는 fleet 최악 조합.** TUI 렌더(React+Ink/Yoga) 비용을 전 인스턴스가 부담 + 텍스트 파싱 취약. **`-p --bare --output-format stream-json`이 1순위 검토안** — TUI 렌더 제거 + 구조화 출력 + 자동로드 스킵, 프로세스 격리는 유지. engram의 `AgentTransport` seam이 이미 있어 PtyTransport 옆에 "stream-json transport" 추가 형태로 흡수 가능. [op, ADR-0002/0004 정합]
3. **MCP 프로세스 폭증(N×M)이 메모리보다 먼저 터질 수 있다.** 일반 MCP는 인스턴스마다 spawn되는 외부 자식 프로세스. 완화: `--bare`+`--mcp-config` 게이팅 / custom tool은 SDK in-process MCP / MCP를 HTTP-SSE(`type:url`) 공유 엔드포인트로 1개만 띄우고 N개가 붙기[?]. 설계 초기에 정책으로 박아야 함.
4. **prompt cache read는 ITPM에 미산입** [F] → 20개가 시스템프롬프트·CLAUDE.md 등 공유 프리픽스를 캐싱하면 rate-limit 여유 확보 = fleet 동시성 한도를 실질적으로 키움.

### 메모리 구성 (인터랙티브 200–400MB, peak 누수 746MB 관측 [2])
Node/V8 베이스라인(큼·미해제 패턴) + React/Ink/Yoga TUI 렌더 레이어 + 인메모리 컨텍스트(세션 누적) + MCP(별도 프로세스). `-p`는 TUI 렌더 루프를 안 돌려 인터랙티브보다 가벼움. headless "<256MB" 주장 [2] — **정확한 절감폭 [?], engram에서 RSS 직접 측정 필요.**

### 방식 비교
| 방식 | 인스턴스당 메모리 | 격리 | 제어성 | 화면분리 | 난이도 |
|---|---|---|---|---|---|
| (a) PTY 풀 CLI N개 (현행) | 최대(200–400MB) | 프로세스 강 | 텍스트 파싱(취약) | 쉬움 | 낮음(됨) |
| (b) headless `-p stream-json` N개 | 중간(TUI분↓) | 프로세스 강 | NDJSON(견고) | 별도처리 | 중간 |
| (c) Agent SDK in-process 다중 | (b)와 동일(=CLI subprocess) | 프로세스 강 | 타입세이프 라이브러리 | 별도 | 중간 |
| (d) 단일 프로세스 + 서브에이전트 | 최소(1개) | 컨텍스트만(격리X) | Agent tool 위임 | 단일스트림 | 낮음 |

engram이 Rust라 SDK 직접사용 부적합 → **현행 PTY 구조를 Rust에서 stream-json spawn으로 바꾸면 사실상 (b)+(c) 이점 취합**. [op]

### 앵커 (engram 제약 적합분)
- **tmux**: 서버-클라 분리 — 무거운 화면버퍼는 서버 1개, 클라 4MB 경량(100창=서버118MB+클라4.2MB). "무거운 상태는 데몬 1곳, 표시는 얇게"가 핵심. [F]
- **Zellij(Rust)**: 서버-클라 분리지만 유휴 22MB·렌더 8배 느림 → **렌더 자체가 fleet에선 비용, 반면교사.** [F]
- **ttyd/gotty**: PTY 1개를 다수 뷰어에 브로드캐스트 → 데몬이 출력 1벌 보유, 프론트 N개 구독. [F, ADR-0013 정합]
- **Managed Agents(서버호스팅)**: 로컬 프로세스 0개지만 파일이 매니지드 샌드박스 → engram "로컬 파일 직접조작" 모델과 불일치. 로컬 자원 한계 초과 시 재검토 후보. [F]

---

## 3. 코드베이스 선행 지점 (백엔드 끝나기 전 손댈 수 있는 것)

seam이 이미 확장 친화적으로 설계됨. 선행 가능/백엔드 필요 구분(파일:라인은 Explore 보고값, 근사 앵커):

| 지점 | 선행 가능 | 백엔드 필요 | 파일 |
|---|---|---|---|
| OutputEvent/InputEvent enum 확장(variant 추가) | ✅ variant+테스트 | ✅ 실 emit | `core/.../agent/types.rs` |
| Capabilities 필드 추가(bool, serde 호환) | ✅ | ✅ 값 채우기 | `types.rs` |
| AgentTransport trait(예: `reconfigure`) | ⚠️ 선택적 메서드 추가 | ✅ 구현체 | `agent/transport/mod.rs` |
| AgentCommand variant + backend dispatch | ✅ variant+impl 골격 | ✅ 실 동작 | `agent/backend/mod.rs`, `profile.rs` |
| CommandSpec 필드(secrets/model_override) | ✅ | ✅ transport 읽기 | `types.rs` |
| PtyTransport.start pump 패턴 → mock 하네스 | ✅ 틀+테스트 | ✅ API/SDK transport | `transport/pty.rs` |
| AgentClient 메서드(`sendMessage`/`reconfigure` stub) | ✅ 시그니처 | ✅ ProtocolClient impl | `src/api/agentClient.ts` |
| ProtocolClient 이벤트타입 설계+stub | ✅ | ✅ wire | `src/api/protocolClient.ts` |
| clientFactory `fleet` 모드 구조 | ✅ 설계(활성화 나중) | ✅ endpoint | `src/api/clientFactory.ts` |

**지금 바로 가능**: ADR 초안 / OutputEvent·InputEvent·Capabilities·CommandSpec 확장 정의 / AgentCommand variant 골격 / AgentClient stub 메서드 / mock transport 통합테스트 틀 / clientFactory fleet 모드 골격.

**불변식 경고 (깨면 회귀)**:
- ⚠️ 새 transport 출력은 반드시 `OutputEvent` variant로 매핑 — raw 바이트 직접 emit 금지(OutputCore가 seq/replay/fanout 단독 소유).
- ⚠️ kill 2동사 순서 `shutdown() → join_pump(5s)` 역전 금지(역전 시 hang).
- ⚠️ output frame epoch 가드 + seq dedup 유지.
- ⚠️ protocol request_id 매칭 / v2 wire format 보전.

---

## 4. 후보 ADR (결정 시점에 박을 것)
1. 구조화/fleet 백엔드 아키텍처 — CLI확장 vs headless stream-json vs SDK/daemon. OutputEvent variant 확장 계획. **격리 vs 메모리 트레이드오프를 사용자 결정으로.**
2. fleet 모드 선택(clientFactory embedded/daemon/**fleet**) + transport/endpoint.
3. 구조화 입출력 wire format(JSON event vs protobuf, 현 WS binary frame 호환).
4. MCP 자원 정책(인스턴스 외부 공유 HTTP-SSE / in-process 강제) — N×M 폭증 방지.

## 5. 실측 필요 (현재 [?])
- headless `-p` 인스턴스당 실제 RSS(인터랙티브 대비 절감폭) · `--bare` 추가 절감 · stream-json transport 출력 정확도/회귀.
- 측정 수단: `scripts/cdp.mjs` + 통합 하네스 / RSS 폴링.
- 주의: 본 노트 메모리 수치(746MB peak, <256MB, Tier별 토큰)는 **[2] 2차출처** — 확정 전 자체 실측.

## 6. 미결 질문 (Open Questions / Deferred)

> 형식 근거: RFC "Open Questions / Future possibilities"(미결 논쟁의 확립 표준) + 측정가능 트리거 + ADR식 "거부한 대안+이유". ADR 표준엔 '보류' 상태가 없어 여기(research note)에 둔다. surfacing은 관련 코드 앵커(`// PARK: …`)로 보강 — 아래 항목 참조.

### [PARK] claude.exe 워밍 풀로 에이전트 부팅 속도를 높일 수 있나?
- **등록 항목:** 상태·재검토 트리거는 `docs/tracking.md` **T-9** 단일관리(rot 방지). 이 절은 *막다른 길·근거 상세*만 보유.
- **surfacing 앵커(TODO, 백엔드 정착 후):** `AgentManager` spawn 경로에 `// see tracking.md T-9: 프로세스 풀링 — 스폰지연 실측 시 재검토` 한 줄 부착 → 메모리/속도 만질 때 자동 노출.
- 현재 결론: **풀링 무의미(현 전제 하).** headless(`-p`)가 메인이라 워밍 풀을 다른 에이전트로 retarget하려면 cwd 변경이 필요한데 headless는 런타임 cwd 변경(`/cd`) 불가. 인터랙티브 풀이면 `/cd`로 가능하나 그건 메모리 비용 + 비메인 경로라 채택 안 함.
- 재론 방지(이미 따진 것 — 다시 꺼내지 말 것):
  - **session-id는 블로커 아님.** 풀 슬롯을 engram-통제 sid로 미리 부팅 → 배정 시 그 sid를 에이전트 정체성으로 *채택*하면 sid 통제(ADR-0008) 유지. "실행 중 프로세스의 sid 재라벨링"만 불가.
  - **cwd 런타임 변경은 인터랙티브 `/cd`로만.** headless 불가. (미검증: `/cd`가 파일도구 cwd + 새 폴더 CLAUDE.md 재로드까지 *완전* retarget하는지 — 인터랙티브 풀을 진지하게 갈 때만 실측.)
  - **Windows엔 `fork()`/COW 없음** → 워밍된 V8 힙 재활용형 prefork(Zygote) 불가. Node도 인터프리터 fork 미노출.
  - **터미널(cmd `/c`) 재활용은 무의미** — 비용은 claude.exe init(Node/V8+번들+autoload)이지 셸 spawn(수 ms)이 아님.
  - **폴더별 풀**(retarget 대신 활성 폴더마다 미리 부팅)은 headless로도 가능 + `/cd` 불필요. 단 "작업 폴더가 소수로 예측 가능" 전제 필요.
  - **메모리 vs 속도 = 같은 다이얼 양끝.** warm 풀=메모리 비용 지불해 속도 획득 / 유휴 kill=속도 희생해 메모리 획득. claude.exe로는 둘 다 max 불가.
- 빠른 스폰의 실효 레버(풀링 대신): `--bare`(autoload 제거로 부팅↓) + warm 티어(hot 에이전트 안 죽임). 근본 출구 = **API transport**(claude.exe 미사용 → 에이전트=in-process 태스크 → 메모리·속도 다이얼 자체 소멸, §2·후보 ADR).
- 관련: §1 ③(headless 한계) · §2(fleet 호스팅) · §4 후보 ADR.

## Sources
- code.claude.com/docs: headless · cli-reference · sessions · permission-modes · hooks-guide · settings · agent-sdk/overview
- Agent SDK subprocess 동작: buildwithaws.substack.com/p/inside-the-claude-agent-sdk-from
- Ink/메모리: medium.com/@sujaypawar/how-claude-code-actually-works · sathwick.xyz/blog/claude-code.html
- tmux vs Zellij 100-pane: tildalice.io/tmux-vs-zellij-100-pane-benchmark · github.com/zellij-org/zellij/issues/3594
- rate limit: platform.claude.com/docs/en/api/service-tiers
