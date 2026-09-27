# 챗 화면 기능 4건 조사 — Esc 끊기 · 스크롤 따라가기 · 도구 호출 묶기 · claude 글자 스트리밍

| 항목 | 값 |
|---|---|
| **상태** | medium 1 패스 — 수집 4 갈래(기능당 1) · 메인 grounding · cross-family 적대 리뷰(§6) |
| **날짜** | 2026-09-27 |
| **방법** | 이 저장소 코드 정독 · 참조 클론(`I:\Engram_Workspace\opensource\` — t3code·paseo·orca·cline·vibe-kanban) 정독 · 웹·`gh api` 조회 · claude 2.1.280 스트림 표본 1회 실측(스크래치, 저장소 밖) |
| **확신도 범례** | **확실** = 1차 자료 둘 이상 또는 실측 + 메인 대조 · **가능성 높음** = 한 출처 + 메인 대조, 또는 수집자 정독만 · **불확실** = 정황뿐 · **grounding ✓** = 메인이 인용 `파일:줄` 을 직접 열어 주장과 맞는지 확인했다 |
| **왜** | 다음 기능 묶음(TRD = `docs/process/S21-chat-ux/trd.md`)의 설계 전제. 사용자가 PRD 는 인계 메모의 제안으로 충분하다고 했고(2026-09-27) TRD 부터 보고받는다 |

> ★**먼저 읽을 것 — 인계 메모의 전제 둘이 틀렸다**★
>
> 1. ★**「끊기는 백엔드에 이미 있다」는 codex 에만 맞다**★ — claude JSON 은 `StdioTransport` 위에서 돌고(`crates/engram-dashboard-agent/src/backend/claude/mod.rs:428`) 그 통로의 `interrupt()` 는 `Unsupported` 를 돌려주며 능력 표시도 `interrupt: false` 다(`transport/stdio.rs:371-375` · `:441`, grounding ✓). 게다가 **화면에서 끊기를 부르는 곳이 한 군데도 없고 명령 표에도 없다** — 클라이언트 메서드(`src/api/protocolClient.ts:988`)만 있다(가능성 높음). 그러니 Esc 는 키 하나가 아니라 **claude 끊기 구현 + 명령 등록 + 키** 다.
> 2. ★**codex 는 도구 호출의 끝·오류를 라이브로 알리지 않는다**★ — 번역기가 `item/started` 에서만 도구 호출을 내고 `item/completed` 는 일부러 버린다(`backend/codex/decoder.rs:256-263`, grounding ✓). 그래서 「묶음 요약에 오류 표시」는 codex 에서 **선 변경 없이는 안 된다.**

---

## 0. 결론

| 기능 | 추천 | 덩치 | 사용자 결정이 남은 곳 |
|---|---|---|---|
| **스크롤 따라가기** | 두 슬롯이 함께 쓰는 훅 하나 + 순수 판정 코어(직접 작성) | 작다 — 프론트만 | 「맨 아래로」 버튼 모양 · 보낼 때 다시 붙기 |
| **Esc 끊기** | 채팅 칸 안에서만 · 도는 중일 때만 · 입력 글 보존 · 명령 `agent.interrupt` 로 등록 · **claude 는 스파이크 선행 조건부**(§6) | 중간 — **claude 끊기가 새 백엔드 일** | claude 까지 할지(codex 만 먼저?) |
| **도구 호출 묶기** | 렌더 시점 순수 함수로 묶고, 종류(검색·읽기…)는 **각 백엔드 번역기가 중립 값으로** 싣는다 | 중간 — 선 타입 1 필드 + 양 번역기 + 프론트 | codex 오류 표시를 이번에 할지 |
| **claude 스트리밍** | 플래그 추가 + 번역기에 블록 단위 상태 + 「흘린 블록의 완결 본문은 버린다」 | 중간 — 백엔드만, 프론트·선 타입 무변경 | 없음(내부 구현) |

---

## 1. 스크롤 따라가기

### 1-1. 지금 코드

- 두 곳 모두 **조건 없이 맨 아래로 내린다** — `src/components/slot/RichSlot.tsx:288-291`(`[items]` 가 바뀔 때마다) · `src/components/slot/DomSlot.tsx:182-185`(`[text]`). grounding ✓. 스크롤 노드는 Radix ScrollArea 의 Viewport 다(ADR-0053).
- RichSlot 은 프레임마다 `setItems([...acc.snapshot()])` 라 스트리밍 델타 하나하나가 효과를 다시 부른다(`RichSlot.tsx:215`, 가능성 높음). 가상화 없음.
- **새 항목 없이 높이가 바뀌는 곳**(가능성 높음): 생각 블록 펼침(`chat/ThoughtRow.tsx` — 안에 **자기 스크롤 영역**이 있다) · 입력창 위 대기 목록(ScrollArea **밖 형제**라 뷰포트를 줄인다, `RichSlot.tsx:458`) · 이번 라운드의 도구 묶음 접기/펼치기.
- 탭은 `display:none` 으로 살려 둔다(`src/components/layout/WindowLayout.tsx:191-201`, ADR-0056) — 숨은 동안 높이가 0 이라 「바닥과의 거리 ≤ 문턱」 판정이 거짓 참이 된다. `display:none` 을 지나 `scrollTop` 이 보존되는지는 **모른다**(GUI 실측 필요).
- 빈 상태가 보이는 동안 ScrollArea 자체가 내려간다(`RichSlot.tsx:397`) — 훅은 뷰포트가 사라졌다 새 노드로 붙는 것을 견뎌야 한다.
- 스크롤 관련 시험 0 · 명령 0(`rg -i scroll src/commands` → 0).

### 1-2. 피어

| 피어 | 「바닥」 문턱 | 사용자 의도 판별 | 제자리 성장 | 버튼 |
|---|---|---|---|---|
| VS Code 챗(MIT) | 2px | 스크롤 위치 + 잠금 플래그, 텍스트 선택 중 보류 | **성장 전** 바닥이었는지를 잡아 두고 뒤에 드러낸다 | 바닥이 아니면 셰브론, 개수 없음 |
| t3code(MIT) | 40px | wheel·touch·pointer·탐색키 | 목록 라이브러리의 끝 유지 | 「끝으로」 알약 — **보이기만 150ms 늦춘다**(탭 전환 깜빡임 방지) |
| paseo(Apache-2.0) | 64px | **위로 가는 입력 증거**가 있어야 풀린다 · 안쪽 스크롤러가 먹을 입력은 무시 | ResizeObserver | 바닥 아니면 셰브론 · **보내면 다시 붙는다** |
| cline | 10px | wheel 위로 → 자동 끔 · 새 턴이 오면 다시 켬 | `overflowAnchor:none` | ArrowDown 원형 버튼(cline-hub) |

(출처 `파일:줄` = 수집 기록. 수집자 정독 — 가능성 높음.)

- **`use-stick-to-bottom`**(MIT, 778★, 유지 중)은 바로 이 문제용이지만 wheel 탈출 판정이 계산된 `overflow` 가 `scroll`/`auto` 인 조상을 찾는데 Radix 뷰포트는 `hidden scroll` 로 계산될 가능성이 높아 **wheel 탈출이 안 걸릴 수 있다**(불확실 — 돌려 보지 않았다). 문서 전역 mousedown 청취자도 붙인다.
- CSS `overflow-anchor` 핀 · `column-reverse` 는 「지금 붙어 있나」 상태를 주지 않아 버튼·LLM 조회에 못 쓴다.
- ★**「새 내용이 왔을 때만」 버튼을 띄우는 피어는 하나도 없었다**★ — 전부 「바닥이 아니면」 띄우고 개수를 안 단다.

### 1-3. 선택지

1. **(추천) 직접 쓴 공용 훅 + 순수 코어** — 붙음 플래그는 스크롤 이벤트(우리가 쓴 값과 비교해 자기 스크롤을 거른다)와 위로 가는 입력(wheel·PageUp·Home…)으로만 바뀐다. 내용·뷰포트 ResizeObserver 가 붙어 있을 때만 다시 내린다. **성장 뒤에 다시 재지 않는다** — 재면 스트리밍 성장이 「사용자가 위로 올렸다」로 읽힌다(가장 위험한 함정).
2. `use-stick-to-bottom` — 위 Radix 불일치 의심 · 자체 스프링 스크롤러 · 전역 청취자.
3. CSS 만 — 상태가 없다.
4. 가상화 도입 — 지금 필요 없다.

---

## 2. Esc 끊기

### 2-1. 지금 코드

- 끊기 호출 경로는 백엔드 셋이 같다(`connection_core.rs:1222-1227` → manager → session → transport, 가능성 높음). **입력 임대(lease)** 를 쥔 클라이언트만 통과한다.
  - codex: 실제 `turn/interrupt` 를 보낸다 · 열린 턴이 없으면 `Unsupported("중단할 턴이 없다")` (`backend/codex/transport.rs:4041-4076`, 가능성 높음).
  - claude JSON: 늘 `Unsupported` (grounding ✓).
  - PTY: `0x03`(Ctrl-C). 이번 범위 밖.
- ★**claude 는 끊기 제어 줄이 있다**★ — 공식 Agent SDK 가 CLI 에 `control_request {subtype:"interrupt"}` 를 보낸다(`anthropics/claude-agent-sdk-python` `src/claude_agent_sdk/_internal/query.py:792-794`, grounding ✓ — SDK 는 우리와 같은 `--input-format stream-json` 으로 CLI 를 띄운다). 이 저장소도 이미 같은 모양의 `control_request cancel_async_message` 를 쓴다(`claude/mod.rs:694-722`, 가능성 높음). ★**끊은 뒤 CLI 가 이미 받아 둔 대기 입력을 어떻게 하는지는 모른다**★ — 대화형 모드 문서는 「대기 메시지가 있으면 다음에 보낸다」라고 하지만(https://code.claude.com/docs/en/interactive-mode) stream-json 에서는 미실측이다.
- 화면 쪽: 채팅 입력창의 `onKeyDown` 이 모든 키의 전파를 막는다(`RichSlot.tsx:471-481`, grounding ✓) · 전역 단축키는 입력 요소와 `.xterm` 안을 거른다(`src/commands/keybindings.ts:7-35`, grounding ✓ — ★load-bearing 가드★). 그러니 Esc 는 **전역 단축키 표가 아니라 채팅 슬롯 안**에 걸어야 한다.
- 「턴이 도는 중」 신호는 뷰 지역값뿐이다 — `streaming = awaiting || (!turnDone && items>0 && !historyPending)`(`RichSlot.tsx:370`, 가능성 높음).
- 문서 전역 Esc 로 닫히는 팝오버가 셋 있다(`AgentMonitoringPicker` · `AgentList` 행 메뉴 · `PresetPalette`) — 입력창이 전파를 막으니 **입력창 Esc 가 먼저 먹을 수 있다**(지금 챗 위에 뜨는 것은 없다 — 가능성 높음).

### 2-2. 피어

| 피어 | 키 | 범위 | 입력 글 | 안 돌 때 | 두 번 |
|---|---|---|---|---|---|
| Claude Code CLI | Esc | 프롬프트(대화상자가 먼저) | 보존 | — | Esc Esc = 글 지우기/되감기 |
| codex TUI | Esc | 입력칸 | 보존 | — | 빈 칸에서 두 번 = 앞 메시지 고치기 |
| **paseo**(최근접 GUI) | Esc | **활성 입력칸의 창만** · 터미널 제외 | **보존**(e2e 시험) | 키를 흘려보냄 | 없음 |
| t3code | 기본 없음(재바인딩 가능) | — | — | — | — |
| orca | 안 건다(PTY 로 흘림) | — | — | — | — |
| Cursor | Ctrl+Shift+Backspace | — | — | — | — |

(수집자 정독 · 문서 — 가능성 높음. Claude Code 문서 문구는 확실.)

- 경향: 터미널 계열(Claude Code·codex·paseo)은 **맨 Esc · 입력칸 범위 · 글 보존 · 안 돌면 아무것도 안 함**. 편집기 안에 사는 것들(Cursor·Zed·t3code)은 Esc 가 이미 「포커스 해제」라 피한다.

### 2-3. 선택지

- 범위: **S1** 입력창 포커스일 때만 · **S2(추천)** 그 채팅 칸 안 어디든(입력창 + 대화 본문 — 칸 루트가 클릭으로 포커스를 받게) · S3 전역(가드에 예외 — 기각, 위 load-bearing 가드를 연다).
- 발화 조건(추천, 피어 합의): 조합 중 아님 · 팝오버 없음 · 도는 중 · 능력 `control.interrupt` 참 · 맨 Esc(수식키 없음) · 키 반복 아님. 입력 글은 늘 보존. 두 번 누르기는 **미룬다**(별 기능).
- 등록(§5): **명령 `agent.interrupt {agentId}`** 를 프론트 명령 표에 둔다. 데몬 버스 짝을 둘지는 `agent.cancelQueuedInput` 선례(데몬이 이름을 선언하고 프론트는 `help` 없이 같은 이름을 건다 — `src/commands/agentCommands.ts:127-158`)를 따른다. 전역 단축키 표(`BINDINGS`)에 넣는 안은 기각 — 가드와 입력창 전파 차단을 둘 다 풀어야 한다.

---

## 3. 도구 호출 묶기

### 3-1. 지금 코드

- 선 타입 `StructuredEvent`(`crates/engram-dashboard-protocol/src/messages.rs:711-788`)에 `ToolCall{name,args_json,id,…}` 는 있고 **도구 결과 변형은 없다**(가능성 높음).
  - claude: `tool_use` → `ToolCall`(이름 = 벤더 도구 이름) · `tool_result`(`is_error` 포함)는 사용자 블록째 `Structured{kind:"user"}` 로 가고 **프론트가 그 벤더 모양을 파싱한다**(`StructuredTextView.tsx:96-111` — 벤더 지식이 이미 프론트로 샌 자리).
  - codex: 도구 item 여섯 종(`decoder.rs:270-281`) · 이름 = MCP 는 `tool` 필드, 나머지는 item 타입(`commandExecution`·`fileChange`·`webSearch`…) · 「검색이냐 읽기냐」를 가를 `commandActions` 는 `args_json` 안에 있다(크기 상한을 넘으면 `{type,id}` 토막뿐).
- **정규화된 종류 값이 없다** — 아이콘은 이름 부분 문자열로 짐작한다(`StructuredTextView.tsx:150-167`).
- 누산기: `ToolCall` → `{kind:'tool',…}` 한 항목(`structuredAccumulator.ts:143-151`, grounding ✓) · 턴 끝 = `MessageDone`/`TurnEnd` → 구분선 항목. 도구 행은 **기본 접힘**이고 펼침 상태는 행 컴포넌트 `useState` 라 재구독(replay)에서 사라진다.
- 레일(`chat/railPositions.ts`)과 DOM 의 행 종류가 맞아야 한다는 불변식(ADR-0051 — `rowKindOf` ↔ `renderItem`).

### 3-2. 피어

- **codex TUI**(`openai/codex` `codex-rs/tui/src/exec_cell/model.rs`) — **연속된 「탐색」 호출만**(읽기·목록·검색) 묶고 다른 호출이 오면 끊는다 · 도는 동안 「Exploring」 끝나면 「Explored」 · 실패 수를 머리에 붉게 · 숨은 생각은 묶음에 흡수(가능성 높음).
- **Claude Code CLI**(`CHANGELOG.md`) — 읽기·검색 묶음을 도는 동안 현재형, 끝나면 과거형 · Ctrl+O 로 펼침 · 「묶음이 화면 밖에서 끝날 때 스크롤이 튄다」를 2.1.83 에서 고쳤다(확실 — 변경 기록 문구).
- **t3code** — **렌더 시점 파생**(`apps/web/src/components/chat/MessagesTimeline.logic.ts:1198-1330`) · 오류 톤 항목·에이전트 생성은 끊고 따로 · 「Read 3 files, searched code 2 times and ran 1 command」 · 펼침 상태 = 첫 호출 id 키 · **펼칠 때 끝 따라가기를 멈춘다**(가능성 높음).
- vibe-kanban — **백엔드가 행동 종류를 정규화**하고 프론트가 같은 종류 2 개 이상 연속을 모은다 · cline — 「가벼운」 도구(읽기·목록·검색)만, 도는 중인 것은 밖에 둔다 · paseo — 펼친 묶음 id 집합.

### 3-3. 선택지

- 묶는 자리: **(추천) 렌더 시점 순수 함수**(replay 에서 같은 항목이 같은 묶음을 낸다 — t3code·vibe-kanban·cline) · 누산기가 묶음 항목을 내기(기각 — 재공급 멱등과 대기 입력 기계를 복잡하게 한다) · 백엔드가 묶기(기각 — 표시 관심사를 선에 싣는다).
- 종류 판별: 프론트 표(빠르지만 벤더 스키마가 프론트로 더 샌다 — 「백엔드 확장」 위반) · **(추천) 번역기가 `ToolCall` 에 중립 종류를 싣는다**(선택 필드 — 옛 데몬은 비워 보내고 프론트는 「기타」로 본다. vibe-kanban·codex TUI 선례).
- 오류·끝 표시: claude 는 지금 데이터로 된다(`tool_result.is_error`). codex 는 `item/completed` 를 번역해 새로 내야 한다 — 새 선 변형을 만들면 **옛 셸이 새 데몬을 만날 때 결과마다 「지원 안 되는 사건」 줄을 그린다**(`structuredAccumulator.ts` 기본 갈래, 가능성 높음).

---

## 4. claude 글자 스트리밍

### 4-1. 지금 코드

- JSON 스폰 인자에 `--include-partial-messages` 가 없다(`claude/mod.rs:184-194`, grounding ✓).
- 번역기는 모르는 줄 타입을 버린다 — `stream_event` 도 거기 든다(`claude/mod.rs:1004-1006` `_ => {}`, grounding ✓). **플래그만 더하면 화면은 그대로다.**
- 번역기의 줄 처리(`consume_line`)는 줄마다 상태가 없다(연관 함수) — 이어받기 기록(transcript) 경로도 같은 함수를 부른다. 스트리밍은 상태(현재 메시지 id · 블록 번호별 종류 · 흘렸나)가 필요하다(가능성 높음).
- 완결 `text` 블록은 지금도 `TextDelta{text: 전문}` 으로 나간다(`claude/mod.rs:1077-1092`, 가능성 높음).
- **프론트는 무변경으로 된다** — 누산기의 `TextDelta` 갈래는 백엔드도 `message_id` 도 안 보고 마지막 글 항목에 이어 붙인다(`structuredAccumulator.ts:130-141`, grounding ✓). 거꾸로 **프론트에 중복 제거가 없으니 제거는 번역기 몫이다.**
- codex 선례: 델타를 내고 **완결 본문은 라이브에서 번역하지 않는다**(이력 복원 문에서만 — `codex/decoder.rs:246-251`, grounding ✓).

### 4-2. 벤더 형식(실측 — claude 2.1.280, 표본 40 줄)

- 감싸개 `{"type":"stream_event","event":{…},"session_id","parent_tool_use_id","uuid"}`.
- 순서: `message_start`(메시지 id) → `content_block_start idx` → 델타들 → **그 블록만 담은 완결 `assistant` 줄**(같은 id) → `content_block_stop` → … → `message_delta` → `message_stop` → (도구면 `user` 결과 → 새 `message_start`) → `result`.
- **델타에는 메시지 id 가 없고 블록 번호만 있다** — `message_start` 의 id 를 기억해야 한다.
- 공식 문서가 같은 계약을 적는다 — 비지 않은 블록마다 완결 메시지 하나, 그 블록의 `content_block_stop` 보다 먼저(https://code.claude.com/docs/en/agent-sdk/streaming-output). → 실측 + 문서 = **확실**.
- 하위 에이전트 델타: 문서는 「전달되지 않는다」, t3code 주석은 「온다 — 버린다」(`t3code/apps/server/src/provider/Layers/ClaudeAdapter.ts:2866-2889`). **불확실 — 어느 쪽이든 `parent_tool_use_id` 가 비지 않은 글 델타는 버린다.**
- 이 표본의 생각 델타는 빈 문자열이었다(우리 모드의 `MAX_THINKING_TOKENS` 없이 찍었다) — 우리 모드에서 생각이 흐르는지는 모른다.
- 이어받기 기록 파일에 `stream_event` 줄이 들어가는지는 **확인 못 했다**(수집자 접근 거부). 기존 기록 표본은 플래그 없이 찍은 것이다.

### 4-3. 피어

- **t3code** — 블록마다 「델타를 냈나」 플래그 · 완결 시 **냈으면 버리고 안 냈으면 완결 본문을 낸다**(`ClaudeAdapter.ts:2260-2290`) · 하위 에이전트 글·생각 델타는 버린다.
- paseo — 완결 본문을 절대 스냅숏으로 보고 이미 낸 길이 뒤의 꼬리만 낸다(접두 불일치면 0 부터 — 중복 모서리).
- vibe-kanban — 같은 자리 항목을 완결본으로 **교체**(JSON 패치 — 주소 붙은 화면 항목이 필요).

### 4-4. 선택지

- 중복 제거: **(추천) 블록 단위 — 흘린 `(메시지 id, 블록 번호)` 의 완결 본문은 버리고, 안 흘렸으면 완결 본문을 낸다**(t3code · codex 라이브 규칙과 같은 모양 · 선 타입·프론트 무변경) · 교체(새 선 변형 + replay 가 교체까지 되감아야 함) · 꼬리 비교(상태가 더 많고 중복 모서리).
- 범위: **(추천) 글만** — 생각은 지금처럼 완결 블록으로 · `input_json_delta` 무시(도구는 완결 줄의 `ToolCall` 로 충분).
- 합치기: 같은 번역 호출 안의 이웃 델타를 합치는 안은 공짜에 가깝지만 codex 도 안 한다 — 링 압박(`REPLAY_MAX_EVENTS=4096`)을 재기 전에는 넣지 않는 쪽이 단순하다.
- 함정: `message_stop` 을 턴 끝으로 옮기면 **도구 호출마다 턴이 끝난다** — 턴 끝은 계속 `result` 줄 하나다. `stream_event` 부속 줄을 `Structured` 로 내면 턴 관측이 진행으로 읽는다(「턴 관측」 불변식 — 30분 막힘 경로).

---

## 5. grounding 결과

메인이 인용 `파일:줄` 을 직접 열어 대조한 것 = 표 안 「grounding ✓」 표시 10 건. 전부 **지지**. 나머지 `파일:줄` 은 수집자 정독만이다(가능성 높음). 대조 중 발견: codex 완결 본문 주석의 실제 줄은 수집 기록(`:243-249`)보다 3 줄 아래(`:246-251`)다 — 본문은 고친 줄로 적었다.

## 6. cross-family 적대 리뷰

**판정 = BLOCK**(codex, effort high · 검산 레벨 2~3 · 2026-09-27). 우선 확인 대상 7 건 중 확인 가능한 것은 **전부 지지**(claude JSON 끊기 불가 · codex 라이브 도구 끝 없음 · 블록 단위 스트림 순서 · 프론트 `TextDelta` 이어 붙임 · 전역 단축키 가드와 입력창 전파 차단). Radix wheel 의심은 「미검」 표시가 맞다고 봤다. **BLOCK 사유는 전부 claude Esc 끊기의 누락**이다 — 주장 반박이 아니라 권고를 바꾸는 빠진 사실:

1. **끊기 응답이 「그래도 돌 대기 메시지」를 싣는다**(`still_queued`) · 대기까지 지우는 `cancel_queued` 는 선택 능력이다(`anthropics/claude-agent-sdk-typescript` CHANGELOG). → 「지금 턴만 끊나 / 대기까지 끊나」를 명시해야 한다. 우리 의도(codex ADR-0235 결정 4 와 같은 「지금 턴만」)가 이것과 맞는지는 스파이크로 본다. 심각도 높음.
2. ★**끊긴 턴의 `result` 가 `is_error: true` 일 수 있다**★(`terminal_reason` = `aborted_streaming`/`aborted_tools` — `claude-agent-sdk-python` `types.py` · SDK 이슈 #429). 우리 번역기는 `is_error` 면 `Error` 를 낸다(`claude/mod.rs:944-966`, **메인 대조 ✓**) → 화면에 오류가 뜨고 턴 관측의 **오류 뒤 멈춤이 켜져 대기 입력이 안 나간다.** 심각도 높음.
3. `system/init` 전에 보낸 끊기는 **같은 접수 응답을 받고도 아무것도 안 끊는다**(SDK 이슈 #429 — CLI 2.1.241, 메인이 이슈 본문 대조 ✓). 2.1.280 에서도 그런지는 모른다. → 접수 응답을 「멈췄다」로 읽지 않는다. 심각도 중간.
4. 끊길 때 **잘린 `assistant` 메시지가 `result` 보다 먼저 올 수 있다**(SDK 이슈 #338) — 잘린 끝을 화면에 어떻게 표시하나가 빠졌다. 심각도 중간.

**집계:** 1~4 는 §2 의 claude 끊기 권고를 **「스파이크 선행 · 결과 분류 규칙 필수」 조건부로 내린다** — TRD(`docs/process/S21-chat-ux/trd.md`) 의 F2 스파이크 종료 조건으로 넘겼다. 나머지 세 기능의 권고는 리뷰가 건드리지 않았다(통과 ≠ 증명 — 가능성 높음 상한).

## 7. 한계 · 미검

- 돌려 보지 않은 것: Radix + `use-stick-to-bottom` wheel 불일치 · `display:none` 을 지난 `scrollTop` 보존 · replay 폭주가 한 커밋으로 묶이는지 · claude stream-json 에서 끊은 뒤 대기 입력의 운명 · 이어받기 기록에 `stream_event` 가 있는지 · 우리 모드에서 생각 델타가 흐르는지.
- 못 찾은 것: ChatGPT·Claude.ai 웹의 스크롤 동작(기권) · Copilot·Windsurf 도구 묶기(기권) · Zed 의 끊기 키(기권).

---

## 8. 추가 조사 — codex 「거부」 도구를 어떻게 보이나 (light · 2026-09-27)

| 항목 | 값 |
|---|---|
| **상태** | light 1 패스 — 수집 1 · 메인 grounding 스팟(업스트림 소스 1 곳) · 적대 리뷰 없음(light) |
| **왜** | TRD 3판 §4-7 의 ★사용자 확인★ 「`declined` 표시」 — 사용자가 「지금도 빨간 박스가 뜨지 않나 · 다른 곳은 어떻게 하나 보고 제안하라」고 했다 |
| **결정** | ★사용자 2026-09-27 = 따로 표기(B)★ — 주황 「거부됨」 · 줄 안 한 줄 사유 · 요약 「거부 N」(오류와 따로). TRD 4판 §4-7 에 반영 |

### 8-1. 지금 우리 앱

- 우리 통로는 codex 의 서버 요청(승인 요청 포함)에 **전부 JSON-RPC `METHOD_NOT_FOUND` 로 거절**하고 성공을 돌려주지 않는다(`crates/engram-dashboard-agent/src/backend/codex/transport.rs:3054-3087` · 들어오는 요청마다 `:3629` — 수집자 정독, 가능성 높음). 우리는 codex 를 `approvalPolicy: on-request` · `sandbox: workspace-write` 로 띄운다(`backend/codex/mod.rs:132-133`, 메인 대조 ✓) — 작업 폴더 밖 쓰기·네트워크는 실행 전에 승인을 묻는다.
- ★**우리가 거절하면 codex 는 그 도구를 `declined` 가 아니라 `failed` 로 끝낸다**★ — 클라이언트 오류를 `ReviewDecision::denied("approval request failed")` 로 바꾸고 명령 item 을 `status:"failed"` · `exitCode:null` · `aggregatedOutput:null` 로 닫는다(openai/codex `codex-rs/app-server/src/bespoke_event_handling.rs:2026-2032` · `:1468-1483`, main `41f9084` — **메인 대조 ✓**). item 에는 사유 칸이 없다. `declined` 는 명시적 Decline/Cancel(우리는 안 보낸다) · guardian 심사 · 네트워크 정책 거부에서만 온다. 파일 변경 승인 거절도 `denied("approval request failed")` 인데 그 item 의 최종 상태는 **모른다**.
- 설치본은 codex-cli 0.156.1 — 업스트림 main 과 같은 동작인지 **미확인**. 승인 교환을 담은 실측 fixture 는 없다(기존 측정은 `approvalPolicy:"never"` 로 찍었다).
- ★**지금 화면에는 아무것도 안 뜬다**★(가능성 높음 — 실측 아님): 거절 경로는 화면으로 사건을 내지 않고, 도구 끝 알림은 번역기가 버린다(`backend/codex/decoder.rs:262`). 빨간 경고 박스(`StructuredTextView.tsx:484-490`)는 codex `error` 알림(`decoder.rs:747-764`) 또는 거절 답을 못 보낸 제어 큐 넘침에서만 그린다. 남는 흔적 = codex 가 뒤이어 쓰는 글뿐.

### 8-2. 피어

| 피어 | 표시 | 색 | 사유 | 실패로 세나 |
|---|---|---|---|---|
| codex TUI(`codex-rs/tui/src/history_cell/approvals.rs:160-240`) | 따로 줄 「✗ You did not approve codex to run …」 · 심사 거부 「Request denied …」 | 빨강 ✗ | 문구가 곧 사유 | 묶음 요약 없음(가능성 높음) |
| Claude Code(이슈 anthropics/claude-code#10954) | 도구 줄 아래 `⎿ User rejected update to <path>` | 모름 | 문구 | 모름 |
| t3code(`packages/client-runtime/src/work-log/presentation.ts:154-155,422-427`) | 「Declined …」 | 실패 표지와 같음 | 없음 | 줄은 실패 · 「최근 도구 실패」 표지에서는 뺀다 |
| Zed(`crates/agent_ui/src/conversation_view/thread_view.rs:8145-8147`) | Rejected = Failed = Canceled | 빨강 X | 없음 | 실패 |
| vibe-kanban(`packages/ui/src/components/ToolStatusDot.tsx:16-19`) | `denied`(사유 칸 있음) | 빨강 점 | 칸은 있음 | 오류에 합침 |
| paseo(`tool-call-mapper-utils.ts:3-10`) | `rejected`/`denied` → `failed` | 실패 | 없음 | 합침 |
| cline CLI(`apps/cli/src/tui/utils/tool-errors.ts:98-106`) | 「… call was skipped before execution.」 | **경고**(오류 아님) | 세부 유지 | 오류 아님 |

(수집자 정독 · gh — 가능성 높음. Cursor 기권.) 갈래 = **실패와 같이 빨갛게**(Zed · vibe-kanban · paseo) vs **따로 표기**(t3code 문구 · cline 경고 톤).

### 8-3. 선택지와 결정

- A 오류에 합침 — codex 가 보내는 그대로라 추가 작업 없음 · 권한에 막힌 것과 진짜 실패가 같아 보인다.
- **B 따로 표기(결정)** — 주황 「거부됨」 · 줄 안 한 줄 사유 · 요약 「거부 N」. codex 가 `failed` 로 보내므로 **우리 통로가 거절한 item 을 기억했다가** 그 끝을 `Declined` 로 낸다(평범한 `declined` 상태도 그대로 `Declined`). 우리 앱의 거절은 전부 우리 정책(승인 창 없음) 때문이라 빨강은 없는 버그를 찾게 한다.
- 남은 확인: B5 실측 — 0.156.1 이 우리 거절에 이미 `declined` 를 보내면 기억 장치는 뺀다 · 파일 변경 경로의 최종 상태.
