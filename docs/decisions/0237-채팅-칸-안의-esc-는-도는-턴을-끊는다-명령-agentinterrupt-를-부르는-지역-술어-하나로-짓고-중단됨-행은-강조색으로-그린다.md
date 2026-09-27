# ADR-0237: 채팅 칸 안의 Esc 는 도는 턴을 끊는다 — 명령 agent.interrupt 를 부르는 지역 술어 하나로 짓고 중단됨 행은 강조색으로 그린다

- 상태: 확정 (2026-09-27, 근거: 사용자 결정 U6 · U8 (TRD `docs/process/S21-chat-ux/trd.md` §1) + TRD 5판 리뷰 FIX 반영(TRD §12) · 구현 전 · 코드 무변경)
- 관련: Amends ADR-0235 (결정 10 UI 정지 단축키 보류) · TRD `docs/process/S21-chat-ux/trd.md`(§1 U6 · U8 · §3-1 · §3-2 · §3-3 · §3-6 · §7 FE-1.2 · FE-2 · §8-2 F2 · §10 0237) · 조사 `docs/research/chat-ux-four-features-2026-09-27.md`(§2) · ADR-0238(claude JSON 끊기 — 이 명령이 닿는 claude 통로) · ADR-0241(주황 「거부됨」 — 중단 행 색과 갈린다) · ADR-0055 · ADR-0167(명령 레지스트리 · `help` 없는 창 명령 = 창 안 전용) · ADR-0173(주황 경고 토큰 `--status-blocked`) · ADR-0155(명령 선언은 생산자 옆) · `docs/tracking.md` T-36(단축키 시스템 — 보류) · step-log S21

## 맥락
ADR-0235 결정 10 은 「UI 정지 버튼·단축키는 지금 넣지 않는다」였다 [사용자 2026-09-26] — 「나중에 단축키 시스템 만들때 넣을거임」.

챗 화면 4건(스크롤 따라가기 · Esc 끊기 · 도구 호출 묶기 · claude 글자 스트리밍)의 동작은 직전 세션 인계 메모에서 사용자와 합의했고 PRD 는 건너뛰었다(사용자 2026-09-27 · TRD 머리). 설계 결정은 오케스트레이터가 내렸고 TRD 가 구현 가능한 명세로 옮겼다. 이 ADR 은 그중 Esc 끊기의 키 · 명령 · 표시를 적는다 — claude 통로 쪽 끊기는 ADR-0238, 끊긴 도구 행의 표시는 ADR-0241 이다.

Esc 를 지금 넣으면서 단축키 시스템은 여전히 보류다(`docs/tracking.md` T-36). 그래서 Esc 는 단축키 표가 아니라 명령을 부르는 **지역 술어** 하나로 짓고, T-36 이 키바인딩 표를 세우면 그 표의 한 줄로 옮긴다(TRD §1 끝 문단).

## 결정
표기 — **[사용자]** = 사용자가 답한 결정(TRD §1 U#) · **[메인]** = 오케스트레이터 판정(TRD §11) · **[고름]** = 사용자 체감이 없는 내부 구현이라 TRD 가 고른 것.

1. **JSON 채팅 슬롯(RichSlot) 안의 맨 Esc = 도는 턴 끊기.** 발화하면 `e.preventDefault()` 하고 `fireAndForget('agent.interrupt', { agentId })`(`src/commands/dispatch.ts:15`) — 사람 키와 LLM 이 같은 핸들을 흔든다. ★낙관 상태를 바꾸지 않는다★ — 끊겼다는 것은 턴 끝 사건이 알린다(claude 의 응답 수신은 「멈췄다」가 아니다 — ADR-0238). ★입력창 글은 건드리지 않는다.★ PTY·xterm 슬롯(`TerminalSlot`)은 바이트 하나 안 바뀐다 — Esc 는 지금처럼 PTY 로 간다.
2. **명령 `agent.interrupt` = 창 + 데몬 버스**(`agent.cancelQueuedInput` 선례 — 선언은 agent crate, 호스팅은 데몬 · 데몬 crate 는 바뀌지 않는다).
   - 창 명령(`src/commands/agentCommands.ts`)은 `agentClient.interruptAgent`(`src/api/protocolClient.ts:988-992`)를 부르고 `{ outcome: 'requested' }` 를 돌려준다. ★`help` 없음★ — 같은 이름을 데몬이 버스에서 답한다. 입력 임대 거절은 `CONFLICT:` 접두로 편다 — [고름] 그 매핑을 작은 함수로 뽑아 `cancelQueuedInput` 과 함께 쓴다.
   - 버스 선언(`crates/engram-dashboard-agent/src/commands.rs`): `catalog_version` 5 → 6 · `#[effect(Write)] #[since(6)]` · `"agent.interrupt" => args AgentInterruptArgs { target: String } -> ok AgentInterruptOk { outcome: String } errors [NOT_FOUND, CONFLICT]`. `outcome` = `requested`(끊기를 보냈다 — 턴이 실제로 멈췄는지는 턴 끝 사건이 알린다). `INPUT_AFFECTING` 에 든다 — WS `Interrupt` 가 임대를 보는 것(`connection_core.rs:1222-1230`)과 같게.
   - 결말 매핑: `Ok` = `requested` · 산 세션 없음 = NOT_FOUND · `PtyError::Unsupported`(codex 「중단할 턴이 없다」 `backend/codex/transport.rs:4045-4061` · claude 턴 열림 문 닫힘 — ADR-0238 · 능력 없는 통로) = CONFLICT(「끊을 턴이 없다 · 이 에이전트는 끊기를 지원하지 않는다」) · 그 밖 = INTERNAL.
3. **범위 = 그 채팅 칸 안 어디든** [사용자 U6] — 입력창 + 대화 본문. 칸 루트가 클릭으로 포커스를 받는다(`tabIndex={-1}` + `outline-none`). 상수 `ESC_SCOPE: 'slot' | 'input' = 'slot'`. 처리기는 루트 `onKeyDownCapture` — 입력창 `onKeyDown` 이 `e.stopPropagation()` 을 먼저 해(`RichSlot.tsx:471-472`) bubble 로는 입력창의 Esc 가 루트에 안 온다. capture 는 입력창과 본문을 한 처리기로 덮는다.
4. **발화 조건은 순수 술어 하나** `isInterruptEscape(e, ctx)`(새 `src/components/slot/interruptKey.ts`) [고름]. 아래 **전부**일 때 참이다:
   - `e.key === 'Escape'` · 수식키 없음 · `!e.repeat`
   - IME 조합 중 아님(`!e.nativeEvent.isComposing && e.keyCode !== 229`)
   - `!e.defaultPrevented`(Radix 레이어는 문서 capture 에서 먼저 먹는다) · 오버레이 표지 `[data-engram-overlay]` 없음
   - `streaming` · `!agentUnavailable` · `agent?.capabilities?.control?.interrupt === true`
   - (`ESC_SCOPE = 'input'` 일 때만) 대상이 입력창

   술어는 DOM 을 읽지 않는다 — 오버레이 여부는 루트 처리기가 재서 `ctx` 로 넘긴다. [고름] 오버레이 뿌리 넷(`SlotContextMenu` · `AgentMonitoringPicker` 백드롭 · `AgentList`·`PresetPalette` 행 메뉴)에 `data-engram-overlay` 를 단다 — 문서 전역 Esc 로 닫히는 셋(조사 §2-1)과 우클릭 메뉴를 한 표지로 잰다. **이 술어가 T-36 키바인딩 표 한 줄의 `when` 으로 그대로 옮겨 간다** — 끊는 동작은 명령에 있어 옮길 때 손댈 것이 없다.
5. **전역 단축키 가드(`src/commands/keybindings.ts:11-35`)는 손대지 않는다** — load-bearing 이다. Esc 는 `BINDINGS`(`:54-57`)에 넣지 않는다.
6. **턴 결말 「중단됨」 행을 살짝 강조한다** [사용자 U8] — `OutcomeRow`(`src/components/slot/StructuredTextView.tsx:289-316`)의 `interrupted` 갈래만 굵게 + 테마 강조색 `var(--accent)`(dark `#4a9eff` · light `#0066cc` · e-ink 검정). 아이콘(`CircleStop`) · 문구(「응답이 중단됐습니다」)는 그대로 · `failed` · `unknown` 은 오늘과 같다. 빨강(실패)도 주황(`--status-blocked` = 거부 — ADR-0241)도 아니라 셋이 갈린다 · 「중단·모름은 붉게 칠하지 않는다」(`StructuredTextView.tsx:286-287`) 그대로. 사용자: 「너무 눈에 띄지 않음?」 → 「색깔은 살짝 강조해야될것같음」. 문구 · 색은 보고 나서 고친다(「나중에 보고 개선하면 되지」).
7. **ADR-0235 결정 10 을 이 범위에서 번복한다** [사용자 — 직전 세션 인계 메모 `.claude/handoff/history/20260927-140032-midturn-master-머지-공개저장소-정리-다음은-브랜치47-결정-기능4건-설계.md:16`: 「에이전트 돌리다가 말치다 끊는거」 · 「Reverses ADR-0235 decision 10」 · 범위는 U6, 색은 U8].
   - **바뀌는 것 = 끊기 키(Esc) 하나뿐이다** — 단축키 시스템을 기다리지 않고 지금, 명령 `agent.interrupt` 를 부르는 지역 술어로 넣는다(결정 1 · 4).
   - **그대로인 것 = 결정 10 의 나머지 둘이다.** ① UI 정지 버튼 보류 — 이 라운드는 버튼을 넣지 않는다(바뀌는 자리 TRD §3-1 에 버튼이 없다). ② 단축키 시스템 보류 — U6 · `docs/tracking.md` T-36(아래 「거부한 대안」).

## 거부한 대안
- **전역 단축키 표에 Esc 를 넣는다** [메인] — 전역 단축키 가드(`src/commands/keybindings.ts:11-35`)를 열어야 하는데 그 가드는 load-bearing 이다(TRD §3-1).
- **단축키 시스템을 지금 만든다** [사용자 — **거부가 아니라 보류**(U6 과 함께 · `docs/tracking.md` T-36)] — Esc 는 지금 명령 `agent.interrupt` 를 부르는 지역 술어이고, T-36 이 키바인딩 표를 세우면 그 표의 한 줄로 옮긴다(결정 4).
- **입력창 포커스일 때만**(`ESC_SCOPE = 'input'`) [사용자 U6 — 칸 안 어디든을 골랐다] — 추천안(칸 안 어디든)은 본문을 클릭해 읽다가도 끊을 수 있다. 이 대안은 피어(Claude Code · codex · paseo)와 같고 포커스 변화가 없다는 이점이 있었다. 상수 하나 + 루트 `tabIndex` 한 줄이라 되돌리기는 싸다.
- **두 번 Esc(글 지우기 · 되감기)** — 이번 범위 밖이다(TRD §1 끝 문단). TRD 는 그 밖의 사유를 적지 않았다.
- **중단 행을 오늘 모양(`text-muted`) 그대로 둔다** [사용자 U8] — 사용자: 「너무 눈에 띄지 않음?」.
- **버스 없이 창 명령만 둔다** [고름] — `help` 없는 창 명령만 남고 버스의 LLM 은 끊기를 못 부른다(TRD §3-3 덩치 판정 — 버스 몫은 선언 1 · trait 메서드 1 · 동사 1 · 시험으로 `cancelQueuedInput` 과 같은 몫이다).

## 근거
- **사용자 결정 2026-09-27** — U6(칸 안 어디든 · 단축키 시스템은 보류) · U8(인용은 결정 6). U6 은 추천안 그대로이고 U8 은 사용자가 먼저 제기했다. 원문 기록 = TRD §1.
- **선례** — `agent.cancelQueuedInput`(`crates/engram-dashboard-agent/src/commands.rs:1-7` — 선언은 agent crate, 호스팅은 데몬) · 명령 레지스트리의 `help` 부재 = 창 안 전용 규칙(`src/commands/renderModeCommands.ts:8-20` · ADR-0167).
- **리뷰** — TRD 1–5판 `/review trd` 라운드 기록(TRD §12). §8-2 F2 ④ 문구 정정(턴이 열리기 전 Esc 는 WS `Interrupt` 경로라 「unsupported」 오류 문자열이 온다 — `CONFLICT:` 접두는 임대 거절만)이 1 라운드 low 로 남아 있다.
- ★**검증 상태**★ — 구현 전이다(TRD 5판 · 코드 무변경). 시험 계획 = TRD §3-6(술어 표 · 조건 표 · 입력창 글 유지 · 본문 클릭 뒤 Esc · `help` 없음 · `INPUT_AFFECTING`) · GUI 실측 계획 = TRD §8-2 F2.

## 영향 / 불변식
- **턴이 열리기 전의 Esc = 무동작.** 프론트 `streaming` 은 보낸 직후 낙관적으로 켜지고 턴 끝 사건까지 늦게 꺼진다. 앞 끝의 Esc 는 통로가 `Unsupported` 로 거절하고(codex · claude 턴 열림 문 — ADR-0238) `fireAndForget` 가 warn 으로 삼킨다 — 사용자는 다시 누르면 된다. 뒤 끝의 늦은 Esc 는 claude 에서 다음 턴을 끊을 수 있다(잔여 경합 — ADR-0238).
- **제어 표면 하나** — 레지스트리 명령과 버스 명령이 같은 이름이고 새 전역 핸들은 없다(CLAUDE.md 「LLM-우선 제어」). 창 명령에 `help` 가 없는 것은 같은 이름을 데몬이 버스에서 답하기 때문이다(`agentCommands.ts:130-133` 의 `cancelQueuedInput` 주석과 같은 사유).
- **전역 단축키 가드 불변** — Esc 를 `BINDINGS` 에 넣으면 이 결정을 넘는다(T-36 이 할 일).
- **우클릭 메뉴가 열린 채 Esc = 아무 일도 없다**(끊기 없음 · 메뉴는 그대로) — `SlotContextMenu` 는 Esc 처리기가 없고 바깥 `mousedown` 으로만 닫힌다(`SlotContextMenu.tsx:110-114`). Esc 닫기는 이번에 더하지 않는다.
- **생성물** — `crates/engram-dashboard-agent/bindings/AgentInterruptArgs.ts` · `AgentInterruptOk.ts` · `commands.schema.json` 은 시험이 굽고 CI sync 게이트가 주인이다 — 손으로 쓰지 않는다.
- **U8 의 알려진 위험** — 채팅 링크가 같은 강조색을 쓸 수 있다. 보고 나서 고친다(사용자).
