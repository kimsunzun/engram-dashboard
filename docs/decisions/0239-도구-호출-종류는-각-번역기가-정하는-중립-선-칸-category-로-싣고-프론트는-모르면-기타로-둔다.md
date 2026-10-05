# ADR-0239: 도구 호출 종류는 각 번역기가 정하는 중립 선 칸 category 로 싣고 프론트는 모르면 기타로 둔다

- 상태: 확정 (2026-09-27, 근거: 사용자 결정 U5 (TRD `docs/process/S21-chat-ux/trd.md` §1) + TRD 5판 §4-1 · §4-2 · §4-3 + TRD 5판 리뷰 FIX 반영(TRD §12) · 구현 전 · 코드 무변경) · 부분 폐기 by ADR-0263 (결정 4의 도는 동안 마지막 묶음 자동 펼침)
- 관련: TRD `docs/process/S21-chat-ux/trd.md`(§1 U5 · §4-1 · §4-2 · §4-3 · §4-6 · §6 · §7 B4 · FE-2 · I1 · §9 · §10 0239) · 조사 `docs/research/chat-ux-four-features-2026-09-27.md`(§3) · ADR-0241(도구 끝 결과 — 같은 묶음의 「오류 N」 · 「거부 N」) · ADR-0004(backend 지식 격리) · ADR-0051(행 종류 ↔ 레일) · ADR-0050(StructuredTextView 순수 렌더) · ADR-0167(`help` 없는 창 명령) · step-log S21 · Amended by ADR-0263 (결정 4의 도는 동안 마지막 묶음 자동 펼침)

## 맥락
챗 화면 4건 중 도구 호출 묶기는 연속된 도구 행 ≥2 를 묶음 하나로 접고, 그 머리에 종류별 개수를 고정 순서로 ` · ` 로 이은 요약 줄을 단다(TRD §4-3). 요약이 종류를 세려면 호출마다 종류가 있어야 한다. 도구 이름과 item 타입은 벤더마다 다르다 — claude 는 `Read` · `Grep` · `Bash` 같은 도구 이름을, codex 는 `commandExecution` · `fileChange` 같은 item 타입과 `commandActions` 를 싣는다. 오늘 단일 도구 행의 아이콘은 프론트의 이름 휴리스틱(`StructuredTextView.tsx:150-167`)이 고른다.

이 ADR 은 종류를 **누가** 판별하나와 묶음을 **어디서** 만드나를 적는다. 끝 결과(오류 · 거부)의 선 모양은 축이 달라 ADR-0241 로 뗐다 — 한 ADR 에 섞으면 한쪽을 번복할 때 다른 쪽까지 폐기 도장이 번지고, 다음 세션이 어느 대안이 어느 결정의 것인지 못 가른다(TRD §10).

## 결정
표기 — **[사용자]** = 사용자가 답한 결정 · **[고름]** = TRD 가 고른 내부 구현.

1. **중립 종류 `ToolCategory { Read, Search, List, Edit, Command, Web, Agent, Mcp, Other }` 를 선에 싣는다.** 각 backend 번역기가 정하고 벤더 도구 이름·item 타입은 선에 안 온다(ADR-0004).
   - agent 도메인(`crates/engram-dashboard-agent/src/types.rs`): `OutputEvent::ToolCall` 에 `category: ToolCategory` — `Option` 아님(번역기는 늘 하나를 고른다).
   - wire(`crates/engram-dashboard-protocol/src/messages.rs`): 같은 아홉 변형의 enum · `StructuredEvent::ToolCall` 에 `category: Option<ToolCategory>`(`#[serde(default, skip_serializing_if = "Option::is_none")]` · `#[ts(optional)]`). 데몬 변환(`connection_core.rs:777-789`)은 일대일 match.
   - TS 접점 = 생성물 `ToolCategory.ts`(`"Read" | "Search" | "List" | "Edit" | "Command" | "Web" | "Agent" | "Mcp" | "Other"`) · `StructuredEvent.ts` 의 ToolCall 에 `category?: ToolCategory`.
2. **판별 규칙은 번역기 안에 있다.**
   - claude(`consume_block` `tool_use` 갈래) — 도구 이름 표 `fn tool_category(name) -> ToolCategory`: `Read` ← `Read` · `NotebookRead` / `Search` ← `Grep` · `Glob` / `List` ← `LS` / `Edit` ← `Edit` · `MultiEdit` · `Write` · `NotebookEdit` / `Command` ← `Bash` · `PowerShell` · `BashOutput` · `KillShell` · `KillBash` / `Web` ← `WebFetch` · `WebSearch` / `Agent` ← `Task` · `Agent` / `Mcp` ← `mcp__` 로 시작 / 나머지(`TodoWrite` 등) `Other`. 표는 정확 일치(대소문자 구분)라 모르는 새 도구는 `Other` 로 떨어진다. [고름] `Glob` 을 `Search` 로 — Claude Code 자신이 「파일을 찾는다」로 묶는다.
   - codex(`codex/decoder.rs` `tool_call`) — `commandExecution` 은 `commandActions[].type` 으로: 전부 `read` = `Read` · 전부 `listFiles` = `List` · `search` 가 있고 `unknown` 이 없다 = `Search` · `read`·`listFiles` 만 섞임 = `Read` · `unknown` 이 하나라도 있거나 배열이 없거나 비었다 = `Command`. `fileChange` = `Edit` · `webSearch` = `Web` · `mcpToolCall` = `Mcp` · `collabAgentToolCall` = `Agent` · `dynamicToolCall` = `Other`.
   - 종류는 `tool_call` 이 받는 **온전한 item** 에서 먼저 정하고 `args_json` 만 크기 상한(`bounded_args_json`)을 지난다 — 상한 토막 때문에 종류를 잃는 일은 없다. 이력 복원(`ItemOrigin::History`)도 같은 함수를 지난다.
3. **프론트는 모르면 「기타」다.** 누산기(`structuredAccumulator.ts`)의 `tool` 항목에 `category: ToolCategory` · 방어 읽기 `normalizeToolCategory` — 아는 아홉 낱말이면 그대로, 없거나 모르면 `'Other'`. 생성물이 오기 전 · 옛 데몬 · 더 새 데몬 모두에서 선다. [고름] 단일 도구 행 아이콘은 `category !== 'Other'` 면 종류 아이콘, 아니면 오늘 이름 휴리스틱 — 옛 데몬에서 오늘 모습 그대로.
4. **묶음은 프론트가 렌더 시점 순수 함수로 만든다 — 누산기도 백엔드도 묶음을 만들지 않는다.** `groupToolRuns(items, turnOpen)`(새 `src/components/slot/chat/toolRuns.ts`):
   - 후보 = 첫 `tool` 행부터 마지막 `tool` 행까지. 사이에 올 수 있는 것 = 그리지 않는 행(`rowKindOf` = `skip`)과 **비지 않은 생각** — 흡수한다 [사용자 U5]. 그 밖의 **그리는** 행(글 · 사용자 말풍선 · 구분선 · 결말 · 오류 · 모르는 사건 · 그 밖의 `structured`)은 끊는다. `tool` 이 ≥2 일 때만 묶음이고, 마지막 `tool` 뒤의 생각·skip 행은 멤버가 아니다.
   - `live` = `turnOpen` 이고 묶음 뒤가 전부 skip·비지 않은 생각뿐이다 — 뒤에 생각이 왔다고 접지 않는다(도구가 이어지면 다시 펼쳐지는 깜빡임). 유효 펼침 = 사용자 토글 `?? live` — 사용자 토글이 자동 접힘을 이긴다(펼침 상태는 인메모리 스토어 `toolGroupStore`).
   - 키 = 첫 호출의 백엔드 id(`tool:<id>`) · 없으면 `item:<itemId>` — 누산기가 같은 사건열을 같은 `itemId` 로 재구성하므로 replay 뒤에도 같다.
   - `StructuredTextView` 는 순수 렌더로 남는다 — state · effect · 스토어 구독을 더하지 않고, 펼침 읽기와 토글은 새 자식 `ToolGroupRow` 가 진다. 레일 위치는 **행 목록**으로 계산한다(묶음 = `'assistant'` 한 행 · 묶음 안에 `ChatRow` 레일을 두지 않는다 — ADR-0051).
   - LLM 경로 = 명령 `chat.toolGroup.setExpanded { slotId, groupKey, expanded }` — `help` 없음(창마다 따로 있는 프론트 상태 · ADR-0167).
5. **`PROTOCOL_VERSION` 은 올리지 않는다** — 옛 데몬 → 칸이 없다 → 프론트 `Other`. 옛 셸 → 모르는 칸을 무시한다(셸은 tag1 JSON 을 해석하지 않고 나른다 — `rg StructuredEvent src-tauri/src` 0 줄). `TurnEnd`(`messages.rs:743-763` 주석)와 같은 판단이다(데몬→셸 한 방향 · 옛 쪽엔 오독할 것이 없다).

## 거부한 대안
- **프론트의 도구 이름 표** — 벤더 지식이 프론트로 샌다(ADR-0004 「백엔드 확장」 — 벤더 지식은 `backend` 한 곳).
- **백엔드가 묶음을 만든다** — 표시 관심사를 선에 올린다(TRD §10). TRD 는 그 밖의 사유를 적지 않았다.
- **도구 사이의 비지 않은 생각이 묶음을 끊는다** [사용자 U5 — 흡수를 골랐다] — 끊으면 생각이 도구마다 끼는 claude 턴에서 묶음이 거의 안 선다. 흡수의 대가 = 펼쳐야 생각이 보인다(codex TUI 선례 = 흡수). 이 대안이면 `groupToolRuns` 의 흡수 판정 한 줄이 「끊는다」로 바뀐다.

## 근거
- **사용자 결정 U5(2026-09-27)** — 추천안 그대로다(TRD §1).
- **분류 판독** — codex `commandActions[].type` 어휘(`read` · `listFiles` · `search` · `unknown`)는 판독이다(TRD §4-2). fixture `tool_end_m7.jsonl` 의 `unknown` → `Command` 가 시험에 든다(TRD §4-6).
- **리뷰** — TRD 1–5판 `/review trd` 라운드(TRD §12).
- ★**검증 상태**★ — 구현 전이다(TRD 5판 · 코드 무변경). 시험 계획 = TRD §4-6(묶기 규칙 표 · 요약 · 레일 · 정규화 · claude 이름 표 · codex 조합 표 · 상한을 넘는 item · 직렬화 왕복).

## 영향 / 불변식
- **모든 생성 자리 · 망라 패턴을 한 커밋에**(컴파일러가 가리킨다 — TRD §4-1 목록 · §7 B4). 링 무게(`estimate_cost_bytes`)는 고정 크기 enum 이라 0 이다.
- **생성물** — `crates/engram-dashboard-protocol/bindings/ToolCategory.ts` · `StructuredEvent.ts` 는 `cargo test -p engram-dashboard-protocol` 이 굽고 CI sync 게이트가 주인이다 — 손으로 쓰지 않는다. 프론트는 한동안 같은 아홉 낱말의 지역 리터럴 합을 쓰고, B4 · B5 · FE-2 가 모두 커밋된 뒤 한 커밋(I1)이 생성물 import 로 바꾼다(TRD §7).
- **벤더 도구 이름 표류** — claude 가 도구 이름을 바꾸면 `Other` 로 떨어진다. 묶음은 그대로 서고 요약 문구만 「기타」가 된다(TRD §9).
- **펼침 상태는 인메모리** — 웹뷰 새로고침에 초기화된다(레이아웃과 같은 수준 — CLAUDE.md 「LLM-우선 제어」). 재구독·replay 는 지우지 않는다.
  > 주석 2026-10-04(결정 무변경): 「레이아웃과 같은 수준」은 더는 맞지 않는다 — 레이아웃은 저장 관리 P3b3(`f9db35a`)부터 `shell\state\state.json` 에 영속된다. 펼침은 여전히 인메모리이고 남기지 않는다(TRD `docs/process/S21-storage/trd.md` §6-0 「남기지 않는 것」 — 사용자 결정 2026-10-02).
- **행 종류 ↔ 레일(ADR-0051)** — 레일을 행 목록으로 계산하고 묶음 안에 레일 행을 두지 않는다. 어기면 레일 계산과 DOM 이 한 몸에서 갈린다.
- `OutputChunk::ToolCall`(`messages.rs:907` — S14 스냅숏 잔재)은 건드리지 않는다.
