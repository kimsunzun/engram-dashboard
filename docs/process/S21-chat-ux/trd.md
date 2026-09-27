# TRD — 챗 화면 4건: 스크롤 따라가기 · Esc 끊기 · 도구 호출 묶기 · claude 글자 스트리밍 (S21)

> 상태: **1판(2026-09-27) — `/review trd` 전 · 코드 무변경.** PRD 는 건너뛰었다(동작은 직전 세션 인계 메모에서 합의 — 사용자 2026-09-27). 설계 결정은 오케스트레이터가 내렸고 이 문서는 그것을 구현 가능한 명세로 옮긴다. 결정과 다르게 가야 할 곳은 고치지 않고 「★메인 확인 필요★」로 적었다(모음 = §11).
>
> **입력:** [조사 보고서](../../research/chat-ux-four-features-2026-09-27.md)(사실의 정본 — 여기서 되풀지 않고 `조사 §n` 으로 가리킨다). 판독 기준 = 브랜치 `v0.3.2/feat/chat-ux` 머리 `3f06e08`. 이 문서의 `파일:줄` 은 전부 그 커밋에서 직접 열어 확인했다. 벤더 = claude 2.1.280(이 PC 설치본) · codex app-server 스키마(t3code 생성본 `packages/effect-codex-app-server/src/_generated/schema.gen.ts` 판독).
>
> **앵커:** ADR-0004(백엔드 지식 격리) · ADR-0012(시험대) · ADR-0044/0045(stream-json · 정제는 백엔드) · ADR-0051(행 종류 ↔ 레일) · ADR-0053(ScrollArea seam) · ADR-0055/0167(명령 레지스트리 · `help` 부재 = 창 안 전용) · ADR-0056(탭 keep-alive) · ADR-0113/0127(턴 관측) · ADR-0155(명령 선언은 생산자 옆) · ADR-0231/0234/0235(턴 도중 입력).
>
> 표기: **[고름]** = 사용자 체감이 없는 내부 구현이라 이 문서가 골랐다. **U#** = 사용자 결정(§1).

---

## 0. 결론 (먼저)

| 기능 | 무엇을 | 덩치 · 층 | 멈춤 조건 |
|---|---|---|---|
| **F1 스크롤 따라가기** | 두 슬롯(RichSlot · DomSlot)의 무조건 `scrollTop = scrollHeight` 를 공용 훅 하나 + DOM 없는 순수 코어로 바꾼다. 붙음 플래그는 스크롤 이벤트와 위로 가는 입력으로만 바뀐다 | 프론트만 | 없음 |
| **F2 Esc 끊기** | JSON 채팅 슬롯 안에서 맨 Esc = 도는 턴 끊기. 명령 `agent.interrupt`(창 + 데몬 버스). claude JSON 끊기를 새로 짓는다(U1) | 프론트 + agent crate | ★claude 부분은 스파이크(§3-5)가 출구 조건을 못 채우면 멈추고 사용자에게 돌아간다★ |
| **F3 도구 호출 묶기** | 연속 도구 행 ≥2 를 렌더 시점 순수 함수로 묶는다. 종류는 각 번역기가 `ToolCall` 에 싣는 중립 `category` | 선 타입 1 필드 + 두 번역기 + 프론트 | 없음 |
| **F4 claude 글자 스트리밍** | `--include-partial-messages` + 번역기 상태(메시지 id · 열린 블록 · 흘린 블록). 흘린 블록의 완결 본문은 버린다 | 백엔드만 · 선 타입·프론트 무변경 | 없음 |

- **구현 순서**(§7): 백엔드 한 워커 = F4 → F2 스파이크 → F2 구현 → F3 백엔드(각 단계 빌드 초록 · 커밋). 프론트 두 워커 = FE-1(F1 → F2 프론트 → F3 접착) ∥ FE-2(F3 프론트). 공유 파일은 FE-1.0 이 먼저 한 번에 친다.
- **열린 사용자 결정 = U1–U6**(§1). 전부 추천안으로 본문을 섰고, 다른 답이면 바뀌는 자리를 각 항목에 적었다.
- **★메인 확인 필요★ = 5 건**(§11). 가장 큰 것은 claude 끊긴 턴을 `MessageDone` 대신 `TurnEnd{Interrupted}` 로 닫는 것(§3-4) — `types.rs:70` 의 「claude 는 그대로 `MessageDone`」 주석과 부딪힌다.

---

## 1. 사용자 결정 (U1–U6)

각 항목 = 추천안 먼저 · 한 줄 트레이드오프 · 답에 따라 바뀌는 자리.

| # | 질문 | 추천 | 대안 | 트레이드오프 | 바뀌는 자리 |
|---|---|---|---|---|---|
| **U1** | claude JSON 끊기를 이번에 짓나 | **짓는다** | codex 만 먼저 | 짓지 않으면 Esc 가 claude 슬롯에서 아무 일도 안 한다(능력 `false`) — 기본 백엔드가 claude 라 체감 대부분이 빈다. 짓는 비용 = 스파이크 + 번역기 한 갈래 + 통로 주입 | 대안이면 §3-4 · §3-5 · §7 B2–B3 의 claude 부분을 뺀다. 버스 명령 · 프론트 키는 그대로 |
| **U2** | codex 묶음에 오류·끝 표시를 이번에 하나 | **안 한다** — codex 묶음은 개수만 | codex `item/completed` 를 결과 사건으로 번역 | 하면 새 선 변형이 필요하고 **옛 셸이 새 데몬을 만나면 결과마다 「표시할 수 없는 신호」 줄을 그린다**(조사 §3-3). 안 하면 codex 묶음 머리에 오류 수가 없다 | 대안이면 새 선 변형 + codex 번역기 `ItemOrigin::Completed` 갈래(`backend/codex/decoder.rs:623`) + 누산기 갈래가 붙는다 — 별 TRD |
| **U3** | 「맨 아래로」 버튼 | **바닥이 아니면 늘 · 셰브론 · 개수 없음 · 보이기 150 ms 늦춤** | 위로 올린 동안 새 내용이 왔을 때만 | 피어 전부가 추천안이다(조사 §1-2). 대안은 「읽다 멈춘 자리」에서 버튼이 안 떠 돌아갈 길을 못 찾는다 | 상수 `JUMP_BUTTON_MODE` 하나(§2-4). 대안이면 코어의 `unseenGrowth` 를 버튼 조건에 건다 |
| **U4** | 보내면 다시 바닥에 붙나 | **붙는다** | 그대로 둔다 | 붙지 않으면 위를 읽다 보낸 글의 답이 화면 밖에서 흐른다. 붙으면 읽던 자리를 잃는다(paseo 선례 = 붙는다) | 상수 `REPIN_ON_SEND` 하나(§2-4) |
| **U5** | 도구 호출 사이의 (비지 않은) 생각 블록 | **묶음에 흡수** | 묶음을 끊는다 | 끊으면 생각이 도구마다 끼는 claude 턴에서 묶음이 거의 안 선다. 흡수하면 펼쳐야 생각이 보인다(codex TUI 선례 = 흡수) | `groupToolRuns` 의 흡수 판정 한 줄(§4-3) |
| **U6** | Esc 가 먹히는 범위 | **그 채팅 칸 안 어디든**(입력창 + 대화 본문 · 칸 루트가 클릭으로 포커스를 받는다) | 입력창 포커스일 때만 | 추천안은 본문을 클릭해 읽다가도 끊을 수 있다. 대안은 피어(Claude Code · codex · paseo)와 같고 포커스 변화가 없다 | 상수 `ESC_SCOPE` 하나 + 루트 `tabIndex` 한 줄(§3-2) |

- 두 번 Esc(글 지우기 · 되감기)는 **이번 범위 밖**이다.

---

## 2. F1 스크롤 따라가기

### 2-1. 바뀌는 자리

| 파일 | 지금 | 바뀜 |
|---|---|---|
| `src/components/slot/RichSlot.tsx:101` · `:288-291` | `scrollRef` + `[items]` 마다 무조건 바닥 | `const follow = useScrollFollow(viewId)` · `<ScrollArea ref={follow.viewportRef}>`(`:398`) · 효과 삭제 |
| `RichSlot.tsx:153` · `:251` | 구독 초기화 · 비우기(onReset) | 두 자리에서 `follow.pin()` — 비운 뒤 오는 이력이 바닥에 착지 |
| `RichSlot.tsx:293-331`(`send`) | — | `if (REPIN_ON_SEND) follow.pin()`(U4) |
| `RichSlot.tsx:398-412` | ScrollArea 자식 = 본문 · 로딩 막 | 버튼 `<JumpToBottom follow={follow} />` 를 로딩 막 옆에 둔다 — absolute 기준이 Root 라 스크롤되지 않는다(`:400-408` 주석과 같은 근거) |
| `src/components/slot/DomSlot.tsx:81` · `:180-185` · `:194-197` | 같은 무조건 효과 | 같은 교체 · `follow.pin()` 은 구독 초기화(`:106-107`)와 onReset(`:155`) |
| 새 `src/components/slot/scrollFollow/followCore.ts` | — | 순수 코어(§2-2) |
| 새 `…/scrollFollow/useScrollFollow.ts` · `followRegistry.ts` · `JumpToBottom.tsx` | — | 훅 · 슬롯 id → 손잡이 모듈 맵 · 버튼 |
| 새 `src/commands/scrollCommands.ts` | — | `slot.scrollToBottom`(§2-5) |

### 2-2. 순수 코어 — `followCore.ts` (DOM 0)

```ts
export const FOLLOW_THRESHOLD_PX = 48 // [고름] 피어 범위 40–64 의 가운데(t3code 40 · paseo 64)
export interface Metrics { top: number; height: number; client: number } // scrollTop · scrollHeight · clientHeight
export interface FollowState {
  pinned: boolean
  frozen: boolean            // client === 0 (display:none 탭) — 재는 값이 무효
  lastTop: number            // 마지막으로 본 scrollTop
  expectTop: number | null   // 우리가 쓰고 아직 이벤트로 못 본 값
  savedTop: number           // 얼기 직전의 scrollTop — 풀릴 때 되살린다
  unseenGrowth: boolean      // 떨어진 동안 내용이 자랐다(U3 대안용)
}
export type FollowInput =
  | { type: 'attach'; m: Metrics }   // 뷰포트 노드가 (다시) 붙었다
  | { type: 'scroll'; m: Metrics }   // scroll 이벤트
  | { type: 'resize'; m: Metrics }   // 내용·뷰포트 ResizeObserver
  | { type: 'intentUp'; m: Metrics } // 위로 가는 사용자 입력(§2-3)
  | { type: 'pin' }                  // 비우기 · 버튼 · 보냄 · LLM
  | { type: 'unpin' }                // F3 묶음 펼침(§4-5)
export function step(s: FollowState, i: FollowInput): { state: FollowState; scrollTo: number | null }
```

규칙(이 표가 시험 표다):

1. **얼음**: `m.client === 0` 이면 `frozen = true`, 다른 칸은 그대로, `scrollTo = null`. `pin`·`unpin` 은 얼어 있어도 `pinned` 만 바꾼다(적용은 풀릴 때).
2. **풀림**(`frozen` 이고 `m.client > 0` 인 `resize`·`attach`): `frozen = false`. 붙어 있으면 바닥으로, 떨어져 있고 `m.top !== savedTop` 이면 `savedTop` 으로 쓴다 — `display:none` 을 지나 `scrollTop` 이 보존되는지 모르므로(조사 §1-1) 되살리기를 늘 건다(보존되면 무동작).
3. **우리 쓰기는 절대 풀지 않는다**: `scroll` 의 `m.top` 이 `expectTop` 과 1px 안이면 `lastTop` 만 옮기고 `expectTop = null`.
4. **사용자 스크롤**: 그 밖의 `scroll` — 바닥 거리 `d = height − top − client`. `d ≤ 문턱` → `pinned = true`(되붙기) · `unseenGrowth = false`. 아니고 `top < lastTop`(위로 움직였다) → `pinned = false`. 아래로 움직였는데 문턱 밖 → 그대로.
5. **위로 가는 입력**(`intentUp`): `height > client`(스크롤할 것이 있다)면 `pinned = false`. 문턱 안의 작은 휠도 곧바로 푼다 — 그래야 성장이 끌어내리지 않는다.
6. **성장**(`resize`): ★붙음을 다시 재지 않는다★(조사 §1-3 — 재면 스트리밍 성장이 「위로 올렸다」로 읽힌다). 붙어 있으면 `scrollTo = max(0, height − client)`. 떨어져 있으면 `unseenGrowth = true`.
7. **쓰기 예고**: `scrollTo` 를 낼 때 `|m.top − scrollTo| > 0.5` 일 때만 `expectTop = scrollTo` — 이미 그 자리면 이벤트가 안 오므로 예고를 남기지 않는다(남기면 뒤의 사용자 스크롤을 우리 것으로 오인할 수 있다).
8. 처음 상태 = `pinned: true`.

### 2-3. 훅 — `useScrollFollow(slotId)`

- 돌려주는 것: `{ viewportRef: (el: HTMLDivElement | null) => void, pinned: boolean, pin(), unpin() }`. `pinned` 은 값이 바뀔 때만 React 상태를 올린다(스크롤 이벤트마다 리렌더하지 않는다).
- **콜백 ref** 라 뷰포트가 내려갔다 새 노드로 붙어도(RichSlot 빈 상태 `:397`) 옛 노드의 청취자·관찰자를 떼고 새 노드에 `attach` 를 먹인다.
- 관찰 대상 둘(ResizeObserver): 뷰포트 자신(입력창 위 대기 목록 `:458` 이 뷰포트를 줄인다 · 창 크기 · 탭 표시)과 **뷰포트의 첫 자식 요소**(Radix 내용 래퍼 — `StructuredTextView.tsx:552-557` 주석의 `display:table` 래퍼). [고름] 내용 ref 를 StructuredTextView·`<pre>` 로 내려보내지 않는다 — 두 슬롯이 손대지 않고 같은 훅을 쓴다. 대가 = Radix 내부 구조에 기댄다(§9).
- **휠**: `wheel` 에서 `deltaY < 0` 이고, 대상에서 가장 가까운 `[data-radix-scroll-area-viewport]` 가 이 뷰포트가 아니면서 `scrollTop > 0` 이면 **안쪽 스크롤러가 먹는 입력**이라 무시한다(ThoughtRow 의 자기 ScrollArea — `chat/ThoughtRow.tsx:53-58`). 그 속성은 Radix dist 가 Viewport 에 싣는다(`node_modules/@radix-ui/react-scroll-area/dist/index.mjs` 판독). 전역 청취자를 걸지 않는다 — 뷰포트에만 건다(`use-stick-to-bottom` 기각 사유 — 조사 §1-2).
- **키**: 뷰포트의 `keydown` 에서 `PageUp`·`Home`·`ArrowUp` → `intentUp`(같은 안쪽 스크롤러 거름). 키로 뷰포트가 스크롤되는 것은 포커스가 그 안에 있을 때뿐이라 뷰포트에 거는 것으로 충분하다.
- **쓰기**: 코어가 낸 `scrollTo` 를 `el.scrollTop` 에 쓴다. RO 콜백은 레이아웃 뒤 · 페인트 전이라 깜빡임이 없다.
- **관측 표면**: 뷰포트에 `data-scroll-follow="pinned" | "free"` 를 훅이 직접 적는다(React 상태 경유 없음).
- **시험 seam**(ADR-0012): `useScrollFollow(slotId, { ResizeObserver? })` — 시험이 가짜 관찰자를 꽂는다(jsdom 엔 RO 가 없다 — `DomSlot.test.tsx:12-17` 이 이미 전역 가짜를 둔다).

### 2-4. 버튼 · 보냄 (U3 · U4)

- `JumpToBottom`: `!pinned` 이 150 ms 이어지면 보인다(t3code — 탭 전환 깜빡임 방지) · lucide `ChevronDown` · 개수 없음 · 누르면 `pin()`(즉시 쓰기 — 부드러운 스크롤은 중간 이벤트를 만든다). `aria-label`·`title` = `t('slot.scrollToBottom')`. DOM `data-jump-to-bottom="1"`.
- 상수 둘(`scrollFollow/useScrollFollow.ts` 머리): `JUMP_BUTTON_MODE: 'whenFree' | 'whenUnseen' = 'whenFree'`(U3) · `REPIN_ON_SEND = true`(U4 — RichSlot 이 읽는다).

### 2-5. LLM 경로 (CLAUDE.md 「LLM-우선 제어」)

- `followRegistry.ts`: `Map<slotId, FollowHandle>` 모듈 맵. 훅이 마운트 때 올리고 내릴 때 **자기 손잡이일 때만** 지운다(StrictMode 이중 마운트). 새 전역 핸들 없음.
- `slot.scrollToBottom { slotId }`(`scrollCommands.ts`) — ★`help` 없음★: 스크롤 위치는 **창마다 따로 있는** 프론트 상태라 `renderModeCommands.ts:8-20` 의 규칙(ADR-0167)이 그대로 든다. 손잡이가 없으면 throw(「이 창에 그 슬롯의 대화·텍스트 뷰가 없다」). 돌려주는 값 = `{ pinned: true }`. 숨은 탭이면 붙음만 세우고 쓰기는 보일 때 한다.
- 읽기 = DOM 속성(`data-scroll-follow`) — `cdp.mjs` eval 로 본다.

### 2-6. 알려진 한계 (고치지 않는다)

- **DomSlot 머리 자르기(200 KB · `DomSlot.tsx:36` · `:142`)**: 떨어져 읽는 동안 앞이 잘리면 본문이 위로 밀린다(표류). 이번 범위 밖.
- 글자를 끌어 선택하는 중 성장 · 스크롤바를 문턱 안에서 끄는 중 성장은 바닥으로 끌린다. 거슬리면 `pointerdown` 을 의도 입력으로 더한다(후보).

### 2-7. 시험

- `followCore.test.ts`(표 시험 — 위 규칙 1–8 각각 + 조합): 우리 쓰기 뒤 이벤트는 안 푼다 · 문턱 안 되붙기 · 성장 중 풀리지 않음 · 얼음 동안 스크롤·RO 무시 · 풀릴 때 붙음=바닥 / 떨어짐=`savedTop` · 스크롤할 것 없는 휠은 안 푼다 · 이미 바닥인 쓰기는 예고 없음.
- `useScrollFollow.test.tsx`(가짜 RO · `Object.defineProperty` 로 `scrollHeight`·`clientHeight`·`scrollTop`): 노드 교체 뒤 재부착 · 안쪽 Radix 뷰포트 휠 무시 · `data-scroll-follow` 값.
- `scrollCommands.test.ts`: 손잡이 있음/없음 · `help` 가 없다(버스 미승선 — `renderModeCommands.test.ts` 의 같은 단언 모양).
- 기존 `RichSlot.test.tsx` · `DomSlot.test.tsx` 는 무조건 스크롤을 단언하지 않는다(`rg scrollTop src --glob '*.test.*'` 의 슬롯 쪽 적중은 RO 가짜뿐) — 회귀로 그대로 돈다.

---

## 3. F2 Esc 끊기

### 3-1. 바뀌는 자리

| 파일 | 바뀜 |
|---|---|
| `src/commands/agentCommands.ts`(`:126-158` 옆) | `agent.interrupt { agentId }` — `agentClient.interruptAgent`(`src/api/protocolClient.ts:988-992`)를 부른다. ★`help` 없음★ — 같은 이름을 데몬이 버스에서 답한다(`:130-133` 의 `cancelQueuedInput` 주석과 같은 사유). 입력 임대 거절은 `:146-155` 와 같은 `CONFLICT:` 접두로 편다 — [고름] 그 매핑을 작은 함수로 뽑아 두 명령이 함께 쓴다. 돌려주는 값 = `{ outcome: 'requested' }` |
| `src/components/slot/RichSlot.tsx:373-392`(루트) | `onKeyDownCapture` 로 Esc 처리(§3-2) · U6 추천안이면 `tabIndex={-1}` + `outline-none` |
| 오버레이 뿌리 넷 | `data-engram-overlay` 속성 하나씩 — `SlotContextMenu.tsx`(루트) · `AgentMonitoringPicker.tsx:100`(백드롭) · `AgentList.tsx`·`PresetPalette.tsx` 의 행 메뉴 뿌리. [고름] 문서 전역 Esc 로 닫히는 셋(조사 §2-1)과 우클릭 메뉴를 한 표지로 잰다 |
| `crates/engram-dashboard-agent/src/commands.rs` | 버스 선언 `agent.interrupt`(§3-3) |
| `crates/engram-dashboard-agent/src/transport/stdio.rs` · `backend/claude/mod.rs` | claude 끊기(§3-4 · U1) |

- **전역 단축키 가드(`src/commands/keybindings.ts:11-35`)는 손대지 않는다** — load-bearing 이다. Esc 는 `BINDINGS`(`:54-57`)에 넣지 않는다.
- PTY·xterm 슬롯(`TerminalSlot`)은 바이트 하나 안 바뀐다 — Esc 는 지금처럼 PTY 로 간다.

### 3-2. 키 조건 (RichSlot 루트 `onKeyDownCapture`)

발화 = 아래 **전부**:

- `e.key === 'Escape'` · 수식키 없음(`ctrl`·`alt`·`shift`·`meta` 모두 거짓) · `!e.repeat`
- IME 조합 중 아님 — `!e.nativeEvent.isComposing && e.keyCode !== 229`(`:476` 과 같은 판정)
- `!e.defaultPrevented`(Radix 레이어는 문서 capture 에서 먼저 먹는다) · `document.querySelector('[data-engram-overlay]') === null`
- `streaming`(`:370`) · `!agentUnavailable`(`:130`) · `agent?.capabilities?.control?.interrupt === true`
- U6 대안이면 추가로 `e.target === textarea`

발화하면 `e.preventDefault()` 하고 `fireAndForget('agent.interrupt', { agentId })`(`src/commands/dispatch.ts:15`) — 사람 키와 LLM 이 같은 핸들을 흔든다. ★낙관 상태를 바꾸지 않는다★ — 끊겼다는 것은 턴 끝 사건이 알린다(claude 의 응답 수신은 「멈췄다」가 아니다 — §3-5 S4). ★입력창 글은 건드리지 않는다.★

- capture 인 이유: 입력창 `onKeyDown` 이 `e.stopPropagation()` 을 먼저 한다(`:471-472`) — bubble 로는 입력창의 Esc 가 루트에 안 온다. capture 는 입력창과 본문을 한 처리기로 덮는다.
- 상수 `ESC_SCOPE: 'slot' | 'input' = 'slot'`(U6).

### 3-3. 명령 — 창 + 데몬 버스 (`agent.cancelQueuedInput` 선례)

선례를 끝까지 따라가면 **선언은 agent crate, 호스팅은 데몬**이다(`commands.rs:1-7`). 데몬 crate 는 바뀌지 않는다 — 입력 임대 검문(`daemon control/commands.rs` 의 `admit_input`)이 `INPUT_AFFECTING` 을 읽는 일반 경로다.

- `commands.rs` `declare_commands!`(`:31-226`): `catalog_version: 5 → 6`(`:41` · v6 주석 한 줄 — 이름이 늘었다) · 새 항목
  ```text
  /// 도는 턴을 끊는다(≠ kill — 프로세스는 산다). `outcome` = `requested`(끊기를 보냈다 — 턴이 실제로 멈췄는지는 턴 끝 사건이 알린다).
  #[effect(Write)] #[since(6)]
  "agent.interrupt" => args AgentInterruptArgs { target: String } -> ok AgentInterruptOk { outcome: String } errors [NOT_FOUND, CONFLICT];
  ```
- `INPUT_AFFECTING`(`:234`)에 `"agent.interrupt"` — WS `Interrupt` 가 임대를 보는 것(`connection_core.rs:1222-1230`)과 같게.
- `AgentCommandHost`(`:263-291`)에 `fn interrupt_agent(&self, id: AgentId) -> Result<(), PtyError>` · `impl for AgentManager`(`:306`) = `AgentManager::interrupt`(`manager.rs:2875-2877`) · 시험 `FakeHost`(`:1343`).
- `make_table`(`:531`)에 한 줄 · `verb_interrupt`: 공백 검문 → `resolve` → `host.interrupt_agent` → `Ok` = `requested` · 산 세션 없음 = NOT_FOUND · `PtyError::Unsupported`(codex 「중단할 턴이 없다」 · 능력 없는 통로) = CONFLICT(「끊을 턴이 없다 · 이 에이전트는 끊기를 지원하지 않는다」) · 그 밖 = INTERNAL.
- 생성물: `crates/engram-dashboard-agent/bindings/AgentInterruptArgs.ts` · `AgentInterruptOk.ts` · `commands.schema.json`(CI sync 게이트 대상).
- 시험: `tests/command_declarations.rs:61-70` 에 `INPUT_AFFECTING.contains(&"agent.interrupt")` · 표 시험(결말 셋 매핑).
- **덩치 판정**: 선언 1 · trait 메서드 1 · 동사 1 · 시험 — `cancelQueuedInput` 과 같은 몫이라 과하지 않다. 프론트만 가는 대안(버스 없이 창 명령만)은 필요 없다 — 적어 두면: `help` 없는 창 명령만 남기고 버스 LLM 은 끊기를 못 부른다.

### 3-4. claude JSON 끊기 (U1) — 설계

**제어 줄**: `{"type":"control_request","request_id":"interrupt:<uuid v4>","request":{"subtype":"interrupt"}}\n` — 모양은 공식 Agent SDK(조사 §2-1). `cancel_line`(`claude/mod.rs:698-724`)처럼 typed struct 로 직렬화한다. ★`cancel_queued` 같은 선택 능력은 싣지 않는다★ — 우리가 원하는 뜻은 codex 와 같다: **도는 턴만 멈추고, 이미 받아 둔 대기 입력은 다음 턴으로 간다**(ADR-0235 결정 4 — `Interrupted` 는 멈춤을 세우지 않는다 · 결정 5 — 남은 글은 다음 한 턴에). 그것이 CLI 응답의 `still_queued`(끊은 뒤에도 돌 대기 메시지 목록)의 뜻인지는 스파이크가 확인한다(§3-5 S3).

**줄이 통로에 닿는 길** — ★메인 확인 필요 ①★: 지시는 「`cancel_async_message` 가 닿는 길을 따라 하라」였다. 그 길은 `MidTurnPolicy::SessionClassified { cancel_line }`(`types.rs:215`)을 세션이 입력 자물쇠 안에서 `send_input` 하는 것이다(`session.rs:563-580`). 끊기는 그 길에 맞지 않는다: ① 능력 `control.interrupt` 는 **통로의 caps** 에서 온다(`session.rs:656-657` → `stdio.rs:425-455`) ② 끊기는 입력 id 에 묶이지 않아 입력 자물쇠가 지킬 순서가 없다. 그래서 **`structured` 주입과 같은 모양**을 쓴다(`stdio.rs:52-56` — 「구조화냐」를 backend 가 주입):

```rust
// transport/stdio.rs — 통로는 이 바이트의 뜻을 모른다(바보 파이프 유지)
pub type InterruptLine = Arc<dyn Fn() -> Vec<u8> + Send + Sync>;
impl StdioTransport {
    /// 「지금 턴을 멈춰 달라」는 줄을 만드는 backend 함수를 꽂는다. 없으면 오늘처럼 `Unsupported`.
    pub fn with_interrupt(mut self, line: InterruptLine) -> Self { self.interrupt = Some(line); self }
}
// interrupt(): Some(f) => self.input.push(f()) · None => 오늘의 Unsupported(`:371-376`)
// capabilities(): control.interrupt = self.interrupt.is_some()(`:441`)
```

- [고름] `open` 인자를 늘리지 않고 빌더로 둔다 — `StdioTransport::open(` 호출이 시험 포함 9 곳이다(`rg "StdioTransport::open\(" crates`). 기존 단언(`stdio.rs:507` · `session.rs:1249` — 주입 없는 통로는 `Unsupported`·`false`)은 그대로 참이다.
- 줄은 입력 큐(`stdio.rs:46-47` — 라이터 스레드 하나)에 들어가 사용자 줄과 **통째로** 직렬화된다(줄 섞임 없음). 입력 자물쇠(`input_order`)는 안 탄다 — 락 순서 불변식(ADR-0006)에 새 간선이 없다.
- `backend/claude/mod.rs` `open_spawn`(`:425-429`): stream-json 갈래에서 `StdioTransport::open(spec, true, Some(decoder))?.with_interrupt(…)`. claude 지식(줄 모양)은 `backend/claude` 에만 산다(「백엔드 확장」).
- 응답 `control_response`(`request_id` 머리 `interrupt:`)는 **번역하지 않는다** — `cancel_response_event`(`:1215-1235`)는 `cancel:` 머리만 보므로 이미 `None` 이다. 응답이 왔다고 턴이 멈춘 것은 아니다(§3-5 S4).

**끊긴 턴의 `result` 분류** — 지금 번역기(`:944-970`)는 `is_error || subtype.starts_with("error")` 면 `Error(RESULT_FAILURE_DETAIL…)` 를 낸다. 끊긴 턴의 `result` 가 `is_error: true` 에 `terminal_reason` = `aborted_streaming` | `aborted_tools` 로 올 수 있다(SDK `types.py` · SDK issue #429 — 메인 대조). 그대로 두면 Esc 한 번이 **오류 행을 그리고 오류 뒤 멈춤(`last_end_failed`)을 세워** 우편을 멈춘다 — 의도와 반대다. 그래서:

- `interrupted(result)` 판정(스파이크가 확정 — S2): `subtype == "interrupted"`(오늘도 오류 아님 — `:951-956`, 유지) **또는** `terminal_reason ∈ {"aborted_streaming","aborted_tools"}`. 두 칸 모두 끊김을 가르지 못하면 대안 = 「이 결과 앞에 우리가 끊기를 보냈다」 표식(`DeliveryAck` 와 같은 화신 공유 `Arc<AtomicBool>` — 끊기 줄 함수가 세우고 번역기가 `result` 에서 읽고 지운다) — ★메인 확인 필요 ④★(오류와 겹친 끊김을 끊김으로 접어 진짜 오류를 가릴 수 있다).
- 참이면 `Usage`(오늘처럼) 뒤 **`OutputEvent::TurnEnd { turn_id: None, outcome: TurnOutcome::Interrupted }`** 하나 — `Error` 도 `MessageDone` 도 내지 않는다.
  - 턴 분류기(`classify_turn` `:553-557`)가 이미 `Interrupted → Ended(Other)` 로 적는다 → 턴은 끝나고 · 오류 뒤 멈춤을 세우지도 풀지도 않는다(ADR-0234 「뜻 모를 턴 끝은 멈춤을 건드리지 않는다」).
  - 누산기(`structuredAccumulator.ts:213-220`)가 `outcome: 'interrupted'` 행(「응답이 중단됐습니다」 — `ko.ts:137`)과 구분선을 그린다 — codex 끊김과 같은 모양. 프론트·선 타입 무변경.
  - ★메인 확인 필요 ②★: `types.rs:70`(「`MessageDone` 을 이것으로 이주시키지 않는다 — claude 는 그대로 `MessageDone` 을 쓴다」)과 `claude/mod.rs:551`(「이 decoder 는 `TurnEnd` 를 내지 않는다」)이 이 변경과 부딪힌다. 이주가 아니라 **끊김 한 갈래만** `TurnEnd` 로 가는 것이고 같은 doc 이 「어느 쪽을 내는지는 각 decoder 가 정한다」고도 적는다. 또 오늘 `subtype:"interrupted"` 는 `MessageDone`(구분선만)으로 닫히는데 이 변경 뒤엔 중단 행이 붙는다(오류 아님은 그대로). 두 주석은 이 변경과 함께 고친다.
- **끊김 직전의 잘린 `assistant` 줄**(SDK issue #338): F4 뒤에는 흘린 블록의 잘린 완결 본문은 버려지고(사용자는 흘린 만큼을 이미 봤다) 안 흘린 블록은 잘린 본문 그대로 나간다. 끝은 위 중단 행이 표시한다.

### 3-5. 스파이크 (B2) — 출구 조건

**방법**: Phase 0 하네스(`.claude/handoff/attachments/20260925-midturn-phase0/claude_harness.js`)로 claude 2.1.280 을 **최종 스폰 인자**(F4 가 먼저 착지 — `--include-partial-messages` 포함)로 띄우고 `{t,dir,line}` 을 기록한다. 결과 = 새 보고서 `docs/research/claude-interrupt-spike-2026-09-2x.md` + fixture `backend/claude/fixtures/interrupt_s1.jsonl`(`fixtures/README.md` 의 가공 규칙).

| # | 시행 | 기록 | 통과 조건 |
|---|---|---|---|
| S1 | 글 흐르는 중 · 도구 도는 중 각각 끊기 | 끊은 뒤 오는 줄 전부 · `result` 의 `subtype`·`is_error`·`terminal_reason` | 턴 끝 줄(`result`)이 **반드시** 온다 |
| S2 | S1 의 `result` | 위 칸 | `subtype` 또는 `terminal_reason` 이 끊김을 가른다(아니면 ★④★) |
| S3 | 도구 중 B 를 써서 `queued` 를 본 뒤 끊기 | 응답의 `still_queued` · B 의 `command_lifecycle` | B 가 `still_queued` 에 있고 · 뒤이어 B 의 `started` 가 새 턴에서 온다(B 가 돈다) · `cancelled`/`discarded` 가 오지 않는다 |
| S4 | 새 턴의 사용자 줄 직후 · `system/init` 전에 끊기 | 응답 · 턴이 멈췄나 | 멈추면 통과. 응답은 오는데 턴이 정상 완료되면(SDK issue #429 — 2.1.241 보고, 2.1.280 미검) ★메인 확인 필요 ③★ |
| S5 | S1 중 잘린 `assistant` 줄 | 흘렸나 · 완결 줄이 왔나 | 기록만(§3-4 끝 문단의 전제 확인) |
| S6 | 턴이 없을 때 끊기 | 응답 · 그 뒤 줄 | 턴 신호를 켜는 줄(`assistant`·`user`·`stream_event` 글)이 오지 않는다 — 오면 LLM 버스의 한가 끊기가 「턴 중」을 켜 30 분 막힘 경로가 된다 |

**스파이크 결과에 기대는 불변식**(결과가 지저분하면 **작업을 멈추고 사용자에게 돌아간다**):

- 「턴 관측 정리 = 두 지점뿐」·30 분 fail-open(ADR-0127) — S1 · S6. 끊은 턴이 끝 줄 없이 남으면 `in_turn` 이 30 분 붙는다.
- 「오류 뒤 멈춤」 `last_end_failed`(ADR-0231 N11 · ADR-0234) — S2. 끊김이 실패로 읽히면 Esc 한 번이 우편을 멈춘다.
- 「대기 입력 상태 = 링 사건 한 줄기」 환원 규칙과 claude 수명주기 번역표(`claude/mod.rs:1164-1181` — `cancelled` → `Dropped{Unknown}` 묘비) · ADR-0235 결정 2 「글은 늘 보인다」 — S3. CLI 가 끊으며 대기분을 버리면 B 가 말풍선 없이 목록에서 사라진다.
- ADR-0226 첫 제출 래치 — 기대지 않는다(claude 는 보내기 전에 센다 · 끊기는 래치를 안 건드린다).

### 3-6. 시험

- 프론트: `agentCommands.test.ts` — `agent.interrupt` 인자 검문 · `interruptAgent` 호출 · 임대 거절 → `CONFLICT:` · `help` 없음. `RichSlot.test.tsx` — 조건 표(수식키 · repeat · 조합 · 오버레이 표지 · `defaultPrevented` · 안 돎 · 능력 거짓 · 부재) 각각 미발화 / 전부 참이면 한 번 발화 · 입력창 글 유지 · 본문 클릭 뒤 Esc 발화(U6).
- 백엔드: `stdio.rs` — 주입 있으면 `interrupt()` 가 줄 한 벌을 큐에 넣고 caps 참 · 없으면 오늘 단언. `claude/mod.rs` — 끊기 줄 골든(`cancel_line_bytes_golden_and_its_answer_round_trips` `:4040` 모양) · 스파이크 fixture 로 끊긴 `result` → `[Usage?, TurnEnd{Interrupted}]` · `classify_turn` → `Ended(Other)` · `interrupt:` 응답 → 사건 없음 · 기존 `result_error_handbuilt.jsonl` 은 여전히 `Error` + `MessageDone`(진짜 오류 회귀).

---

## 4. F3 도구 호출 묶기

### 4-1. 선 타입 — `category` (Rust ↔ TS 접점, ★워커 간 고정★)

**agent 도메인** `crates/engram-dashboard-agent/src/types.rs`:

```rust
/// 도구 호출의 중립 종류 — 각 backend 번역기가 정한다(ADR-0004). 벤더 도구 이름·item 타입은 여기 안 온다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCategory { Read, Search, List, Edit, Command, Web, Agent, Mcp, Other }
// OutputEvent::ToolCall(:46-53) 에 칸 하나: `category: ToolCategory` — Option 아님(번역기는 늘 하나를 고른다)
```

**wire** `crates/engram-dashboard-protocol/src/messages.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub enum ToolCategory { Read, Search, List, Edit, Command, Web, Agent, Mcp, Other }
// StructuredEvent::ToolCall(:719-726) 에:
//   #[serde(default, skip_serializing_if = "Option::is_none")]
//   #[ts(optional)]
//   category: Option<ToolCategory>,
```

- **TS 접점(고정)** — 생성물 `crates/engram-dashboard-protocol/bindings/ToolCategory.ts` = `export type ToolCategory = "Read" | "Search" | "List" | "Edit" | "Command" | "Web" | "Agent" | "Mcp" | "Other";` · `StructuredEvent.ts` 의 ToolCall 에 `category?: ToolCategory`. 프론트 워커는 생성물이 오기 전에도 이 모양으로 쓴다(§4-3 의 방어 읽기).
- **호환**: 옛 데몬 → 칸이 없다 → 프론트 `Other`. 옛 셸 → 모르는 칸을 무시한다(셸은 tag1 JSON 을 해석하지 않고 나른다 — `rg StructuredEvent src-tauri/src` 0 줄). `PROTOCOL_VERSION` 은 올리지 않는다 — `TurnEnd`(`:743-763` 주석)와 같은 판단(데몬→셸 한 방향 · 옛 쪽엔 오독할 것이 없다).
- **데몬 변환** `crates/engram-dashboard-daemon/src/connection_core.rs:777-789` — `category: Some(map(category))`(일대일 match).
- **링 무게** `output_core.rs:1225-1230` `estimate_cost_bytes`: 고정 크기 enum 이라 무게 0 — 구조 분해만 `category: _` 로 넓힌다.
- **모든 생성 자리를 한 커밋에**(컴파일러가 가리킨다): `claude/mod.rs:1108` · `codex/decoder.rs:1136` · `output_core.rs` 시험(`:2152-2172` · `:2248`) · `daemon/src/agent_conn.rs:592`·`:618` · `connection_core.rs:5150-5160` · `protocol/src/messages.rs:1047` 시험. ts-rs 생성물은 `cargo test -p engram-dashboard-protocol` 이 굽는다(CI sync 게이트).
- `OutputChunk::ToolCall`(`messages.rs:907` — S14 스냅숏 잔재)은 건드리지 않는다.

### 4-2. 백엔드 분류

- **claude**(`claude/mod.rs` `consume_block` `tool_use` 갈래 `:1092-1115`) — 도구 이름 표 `fn tool_category(name: &str) -> ToolCategory`:
  - `Read` ← `Read` · `NotebookRead` / `Search` ← `Grep` · `Glob` / `List` ← `LS` / `Edit` ← `Edit` · `MultiEdit` · `Write` · `NotebookEdit` / `Command` ← `Bash` · `PowerShell` · `BashOutput` · `KillShell` · `KillBash` / `Web` ← `WebFetch` · `WebSearch` / `Agent` ← `Task` · `Agent` / `Mcp` ← `mcp__` 로 시작 / 나머지(`TodoWrite` 등) `Other`.
  - [고름] `Glob` 을 `Search` 로 — Claude Code 자신이 「파일을 찾는다」로 묶는다. 표는 정확 일치(대소문자 구분)라 모르는 새 도구는 `Other` 로 떨어진다.
- **codex**(`codex/decoder.rs` `tool_call` `:1126-1146`) — item 타입과 `commandActions`:
  - `commandExecution` → `commandActions[].type`(판독: `read` · `listFiles` · `search` · `unknown`). 전부 `read` = `Read` · 전부 `listFiles` = `List` · `search` 가 있고 `unknown` 이 없다 = `Search` · `read`·`listFiles` 만 섞임 = `Read` · `unknown` 이 하나라도 있거나 배열이 없거나 비었다 = `Command`.
  - `fileChange` = `Edit` · `webSearch` = `Web` · `mcpToolCall` = `Mcp` · `collabAgentToolCall` = `Agent` · `dynamicToolCall` = `Other`.
  - ★크기 상한 토막은 문제가 안 된다★: 종류는 `tool_call` 이 받는 **온전한 item**(`&Value`)에서 먼저 정하고 `args_json` 만 상한을 지난다(`bounded_args_json` `:1159`). 지시의 「토막이면 item 타입으로 폴백」은 그래서 생기지 않는다 — `commandActions` 가 없을 때의 폴백(`Command`)만 남는다. 이력 복원(`ItemOrigin::History`)도 같은 함수를 지난다.

### 4-3. 프론트 — 묶기 함수 (렌더 시점 · 누산기는 묶음을 만들지 않는다)

- **누산기** `structuredAccumulator.ts:34`(`tool` 변형)에 `category: ToolCategory` · `:143-151` 에서 `category: normalizeToolCategory((ev as { category?: unknown }).category)` — 아는 9 낱말이면 그대로, 없거나 모르면 `'Other'`. 방어 읽기라 생성물이 오기 전·옛 데몬·더 새 데몬 모두에서 선다.
- **새** `src/components/slot/chat/toolRuns.ts`(순수):

```ts
export type DisplayRow =
  | { kind: 'item'; item: StructuredItem }
  | { kind: 'toolGroup'; key: string; members: StructuredItem[]; calls: ToolItem[]; live: boolean }
/** turnOpen = StructuredTextView 의 `streaming`(RichSlot `:370`). */
export function groupToolRuns(items: readonly StructuredItem[], turnOpen: boolean): DisplayRow[]
export function summarizeGroup(calls: readonly ToolItem[], errorIds: ReadonlySet<string>):
  { counts: ReadonlyArray<readonly [ToolCategory, number]>; errors: number }
```

- **묶음 규칙**:
  - 후보 = 첫 `tool` 행부터 마지막 `tool` 행까지. 그 사이에 올 수 있는 것 = 그리지 않는 행(`rowKindOf` = `skip` — usage · claude `tool_result` 운반 행 · 빈 생각 · `StructuredTextView.tsx:377-395`)과 **비지 않은 생각**(U5 흡수 — 대안이면 이 한 줄이 「끊는다」로 바뀐다).
  - 그 밖의 **그리는** 행은 전부 끊는다 — 글 · 사용자 말풍선 · 구분선(턴 끝) · 결말 · 오류 · 모르는 사건 · 그 밖의 `structured`.
  - `tool` 이 ≥2 일 때만 묶음. 하나면 오늘처럼 그 행 그대로.
  - 마지막 `tool` 뒤의 생각·skip 행은 멤버가 아니다(흐름으로 돌아간다).
  - `live` = `turnOpen` 이고 묶음 뒤에 오는 행이 전부 skip·비지 않은 생각뿐이다. ★뒤에 생각이 왔다고 접지 않는다★ — 그 뒤에 도구가 이어지면 다시 펼쳐지는 깜빡임이 된다.
  - `key` = 첫 호출의 백엔드 id(`tool:<id>`) · 없으면 `item:<itemId>` — 누산기는 같은 사건열을 같은 `itemId` 로 재구성하므로(`structuredAccumulator.ts:11-15` 멱등 불변식) replay 뒤에도 같은 키다.
- **렌더**(`StructuredTextView.tsx:514-561`): `items.map(renderItem)` 을 `groupToolRuns(items, streaming)` 의 행 목록으로 바꾼다. ★ADR-0051 불변식★ — 레일 위치(`chat/railPositions.ts`)는 **행 목록**으로 계산한다: 묶음 = `'assistant'` 한 행 · 항목 = 오늘 `rowKindOf`. 묶음은 `ChatRow rail` 하나로 그리고, 펼쳤을 때 멤버는 그 안에서 행 컴포넌트(`ToolItemRow` · `ThoughtRow`)만 그린다 — 안쪽에 `ChatRow` 레일을 두지 않으므로 레일 계산과 DOM 이 한 몸으로 남는다. `isRenderedItem`(`:402-404` — RichSlot 이 쓴다)은 항목 단위 그대로.
- **요약 줄**: 아이콘(lucide `Layers`) · 종류별 `t('chat.toolGroup<Kind>', {count})` 를 고정 순서(검색 · 읽기 · 목록 · 편집 · 명령 · 웹 · 에이전트 · MCP · 기타)로 ` · ` 로 잇고 · 오류가 있으면 끝에 `t('chat.toolGroupErrors', {count})`(붉은 톤). 오류 수 = `buildToolResultMap`(`:117-125`)의 `isError` 인 호출 id — claude 만 채워진다(U2). 셰브론 · `aria-expanded`.
- DOM 표지: 묶음 뿌리 `data-tool-group={key}` · `data-tool-group-open="1"|"0"` · `data-tool-group-count={calls.length}`.
- [고름] 단일 도구 행 아이콘은 `category !== 'Other'` 면 종류 아이콘, 아니면 오늘 이름 휴리스틱(`:150-167`) — 옛 데몬에서 오늘 모습 그대로.

### 4-4. 펼침 상태 · 명령

- **새** `src/store/toolGroupStore.ts`(zustand):
  ```ts
  interface ToolGroupState {
    bySlot: Record<string, { agentId: string; open: Record<string, boolean> }>
    bind(slotId: string, agentId: string): void   // 다른 에이전트면 그 슬롯 칸을 비운다
    clear(slotId: string): void                   // 새 화신 비우기(onReset) 전용
    setOpen(slotId: string, key: string, open: boolean): void
  }
  ```
  유효 펼침 = `open[key] ?? live` — **사용자 토글이 자동 접힘을 이긴다**(양방향). 재구독·replay 는 지우지 않는다. 웹뷰 새로고침은 인메모리라 초기화된다(레이아웃과 같은 수준 — CLAUDE.md 「LLM-우선 제어」).
- RichSlot 접착(FE-1.3): `StructuredTextView` 에 `slotId={viewId}` · 마운트 효과에서 `bind(viewId, agentId)` · onReset(`:249-266`)에서 `clear(viewId)`. `StructuredTextView` 의 새 prop 은 선택이다 — 없으면 자동 규칙만 쓴다(FE-2 가 FE-1 을 기다리지 않는다).
- **명령** `chat.toolGroup.setExpanded { slotId, groupKey, expanded: boolean }`(새 `src/commands/chatCommands.ts`) — ★`help` 없음★(창마다 따로 있는 프론트 상태 — `renderModeCommands.ts:8-20` · ADR-0167). 인자 검문은 `requireSlotId` 모양으로 throw. `groupKey` 는 DOM `data-tool-group` 에서 읽는다.

### 4-5. F1 과의 맞물림

- 접기·펼치기·자동 접힘으로 높이가 바뀌면 F1 의 내용 RO 가 받는다 — 붙어 있으면 바닥으로 간다.
- ★메인 확인 필요 ⑤★ [고름 후보 · t3code 선례]: 사람이 **마지막이 아닌** 묶음을 펼치면 `follow.unpin()` 을 부른다. 붙은 채 위쪽 묶음을 펼치면 바닥으로 다시 내리면서 누른 머리가 화면 위로 밀려난다(Claude Code 가 2.1.83 에서 고친 「스크롤이 튄다」와 같은 부류 — 조사 §3-2). 마지막 묶음 토글은 붙음을 그대로 둔다. 지시(「RO 가 처리한다」)에 더하는 것이라 올린다 — 빼도 다른 곳은 안 바뀐다(`StructuredTextView` 의 `onGroupToggle` prop 하나).

### 4-6. 시험

- `toolRuns.test.ts`: 도구 1 개 = 묶음 없음 · 2 개 = 묶음 · `tool_result` 운반·usage·빈 생각 끼어도 이어짐 · 비지 않은 생각 흡수(U5) · 글/말풍선/구분선/결말/오류/모르는 사건이 끊음 · 마지막 도구 뒤 생각은 멤버 아님 · `live` 참(턴 중 · 뒤가 생각뿐) / 거짓(끊는 행이 옴 · 턴 끝) · 키 = 첫 id · id 없으면 `itemId` · 같은 사건열 두 번 = 같은 결과(replay).
- `summarizeGroup`: 고정 순서 · 0 인 종류 생략 · 오류 수.
- `StructuredTextView.test.tsx`: 레일 위치가 행 목록과 DOM 에서 일치(ADR-0051 — 기존 레일 시험 모양) · 사용자 토글이 자동 접힘을 이김 · `data-tool-group*` 값.
- `structuredAccumulator.test.ts`: `category` 있음·없음·모르는 낱말 → 정규화.
- `toolGroupStore` · `chatCommands.test.ts`(`help` 없음).
- Rust: claude 이름 표(각 낱말 + `mcp__x__y` + 모르는 이름) · codex `commandActions` 조합 표(fixture `tool_end_m7.jsonl` 의 `unknown` → `Command` 포함) · 상한을 넘는 item 도 종류는 온전한 item 에서 · 데몬 변환 일대일 · 직렬화 왕복(`category` 없는 옛 JSON 도 읽힌다).

---

## 5. F4 claude 글자 스트리밍

### 5-1. 인자

- `claude/mod.rs:184-194` StreamJson 갈래에 `--verbose` 뒤 `args.push("--include-partial-messages")`. 터미널 갈래(`:171-180`)는 그대로.
- 시험: 골든 `json_mode_build_spec_uses_headless_stream_json_args`(`:2674-2704`)에 그 낱말을 더하고, 터미널 모드에 없다는 단언 하나.

### 5-2. 번역기 상태 · 규칙

`ClaudeStreamDecoder`(`:768-794`)에 칸 하나 — [고름] 줄 버퍼와 같은 수명(화신 하나):

```rust
/// 라이브 부분 메시지 추적 — `stream_event` 가 채우고 완결 `assistant` 줄이 읽는다.
#[derive(Debug, Default)]
struct PartialMessage {
    /// 지금 메시지의 id(`message_start`). `None` = 추적 중인 메시지가 없다.
    id: Option<String>,
    /// 시작했고 아직 멈추지 않은 블록 번호(`content_block_start` ~ `content_block_stop`).
    open: Option<u64>,
    /// 글 델타를 하나라도 흘린 블록 번호들.
    streamed: BTreeSet<u64>,
}
```

`consume_line`(`:882-1005`)은 인자를 하나 더 받는다: `partial: Option<&mut PartialMessage>` — 라이브(`decode` `:836-840` · `flush` `:865`)는 `Some(&mut self.partial)`, 이어받기(`parse_transcript_events` `:1351`)는 `None`.

| 줄 | 규칙 |
|---|---|
| `stream_event` + `parent_tool_use_id` 가 null 아님 | **통째로 버린다**(`message_start` 포함 — 하위 에이전트 흐름이 부모 상태를 덮지 않게 · 조사 §4-2) |
| `message_start` | `*partial = { id: message.id, open: None, streamed: {} }` — 턴 도중 접힌 입력이 여는 새 메시지도 여기서 새로 선다 |
| `content_block_start{index}` | `open = Some(index)` |
| `content_block_delta{index, delta:{type:"text_delta", text}}` | `id` 가 있고 `text` 가 비지 않으면 `TextDelta{text, turn_id: None, message_id: id.clone()}` · `streamed.insert(index)` · `id` 가 없으면(`message_start` 없이 온 늦은 델타) **버린다** |
| 그 밖의 델타(`thinking_delta` · `input_json_delta` · `signature_delta`) | 버린다 — 생각·도구는 완결 줄에서 오늘처럼 |
| `content_block_stop{index}` | `open == Some(index)` 면 `open = None` |
| `message_delta` · `message_stop` · 모르는 `event.type` | 아무것도 안 낸다 — ★`MessageDone`/`TurnEnd`/`Structured` 금지★(§5-3) |
| 완결 `assistant` 줄 | `message.id == partial.id` 이고 `open = Some(k)` 이고 `streamed ∋ k` 면 그 줄의 **`text` 블록만** `TextDelta` 를 내지 않는다. 나머지 블록(`tool_use` · `thinking` · …)과 그 밖의 모든 경우(안 흘린 블록 · id 불일치 · `partial` 없음)는 오늘 그대로(`consume_block` `:1045-1128`) |
| `result` | 오늘 번역 뒤 `*partial = PartialMessage::default()` |

- **「(메시지 id, 블록 번호)」를 여는 블록으로 잡는 이유** [고름]: 완결 줄에는 블록 번호가 없다. 계약은 「비지 않은 블록마다 완결 메시지 하나, 그 블록의 `content_block_stop` 보다 먼저」다(조사 §4-2 — 실측 + 공식 문서 = 확실). 그러니 완결 줄이 오는 순간 열려 있는 블록이 곧 그 줄의 블록이다. 빈 블록(완결 줄 없이 멈춤)은 자리를 차지하지 않는다. 벤더가 순서를 바꾸면(멈춘 뒤 완결) `open` 이 비어 **오늘처럼 전문을 낸다 — 잃지 않고 겹친다**(보이는 쪽으로 틀린다).
- **합치지 않는다**(codex 와 같다 — 조사 §4-4). 링 압박은 §9.
- 문서 갱신: `:762-767`(「decoder 자신의 상태 = 줄 재조립뿐」)을 이 칸으로 고친다.

### 5-3. 불변식

- **턴 끝은 `result` 한 줄 그대로**(조사 §4-4 함정) — `message_stop` 을 끝으로 옮기면 도구 호출마다 턴이 끝난다.
- **스트림 부속 줄을 `Structured` 로 내지 않는다** — claude 턴 분류기는 `Structured` 를 통째로 진행으로 센다(`:538-545`). 턴 끝 뒤의 진행은 30 분 막힘 경로다(CLAUDE.md 「대기 입력 상태」 끝 문단).
- 흘린 델타는 `TextDelta` 라 진행이다 — 턴 **안**에서만 나온다: `result` 가 상태를 지우고, `message_start` 없이 온 늦은 델타는 버리므로 턴 끝 뒤에 「턴 중」을 다시 켤 길이 없다.
- **프론트 무변경**: 누산기 `TextDelta` 갈래는 마지막 글 항목에 이어 붙이고 중복 제거가 없다(`structuredAccumulator.ts:130-141`) — 제거는 번역기 몫이고 위 표가 진다. 흘린 첫 델타와 완결 본문이 같은 글 항목으로 모이므로 완결 버림이 빠지면 글이 두 벌이 된다(회귀 시험).

### 5-4. 이어받기 기록 (transcript)

- [고름] `None` 경로는 `stream_event` 줄을 **통째로 건너뛴다**(오늘의 `_ => {}` `:1003`). 기록에 그 줄이 있든 없든 완결 `assistant` 줄이 전문을 내므로 한 벌이다 — 지시의 「기록마다 새 상태」보다 단순하고 기록 모양에 기대지 않는다.
- **확인 단계**(B1 끝): 플래그를 켠 세션 하나를 이어받아 ① 그 `~/.claude/projects/<slug>/<sid>.jsonl` 에 `"stream_event"` 가 있는지 ② 완결 `assistant` 글 줄이 남아 있는지 기록한다. ② 가 거짓이면(기록이 부분 줄만 남긴다) 이어받은 화면의 답이 빈다 → **멈추고 메인에 올린다.** ① 이 참이면 그 줄을 fixture 로 떠서 「건너뛴다」를 못 박는다.

### 5-5. 시험 · fixture

- **새 fixture** `backend/claude/fixtures/partial_stream_p1.jsonl` — 우리 실제 스폰 인자(`MAX_THINKING_TOKENS=8000` 포함)로 떠서 `fixtures/README.md` 의 가공 규칙(잡음 줄 · `system/init` 손질 · 개인정보 치환)을 적용한다. 담을 것: 글만인 답 · 글 → 도구 → 결과 → 글 · 생각이 있는 답 · 턴 도중 접힌 입력(새 `message_start`). README 표에 한 행.
- 시험(조사 §4 의 경우 + 지시의 추가):
  1. 글 델타마다 `TextDelta`(메시지 id = `message_start` 의 것) · 그 블록의 완결 글은 안 나온다.
  2. 델타 없는 글 블록 → 완결 전문(폴백).
  3. `tool_use` 는 완결 줄에서 `ToolCall` 한 번 · `input_json_delta` 는 사건 0.
  4. 생각 델타 무시 · 완결 생각 블록 → `Structured{thinking}` 오늘처럼.
  5. `parent_tool_use_id` 있는 `stream_event` 전부 버림(그 `message_start` 가 상태를 안 바꾼다).
  6. `message_start`/`message_delta`/`message_stop` → 사건 0(`MessageDone`·`TurnEnd`·`Structured` 없음).
  7. 새 `message_start` 가 상태를 갈아 끼운다(도구 루프 · 접힌 입력).
  8. `result` 뒤 늦은 델타 → 사건 0.
  9. 줄이 청크 둘에 걸친 `stream_event`(한글 포함) → 한 번.
  10. 이어받기 원문에 `stream_event` 가 섞여도 완결 글 한 벌.
  11. ★회귀 — 부분 줄 없는 기존 fixture 전부(`claude_text` · `claude_tool` · `claude_transcript` · `lifecycle_m1` · `cancel_m3` · `drain_m7` · `slash_m13` · `transcript_queued_m5` · `result_error_handbuilt`)의 사건열이 바이트 단위로 같다★ — 기존 시험을 고치지 않고 통과시키는 것이 증거다(고쳐야 하면 설계가 틀렸다).
  12. `classify_turn`: 흘린 `TextDelta` 는 진행 · 부속 줄은 신호 없음.
- ADR 후보: 이 중복 제거 규칙은 load-bearing 이다(프론트에 제거가 없고, 빠지면 모든 claude 답이 두 벌이다) — §10.

---

## 6. 건드리는 불변식 (한눈에)

| 불변식(CLAUDE.md 「핵심 불변식」·ADR) | 기능 | 어떻게 지키나 |
|---|---|---|
| 턴 관측 · 30 분 fail-open(ADR-0127) | F2 · F4 | F2 = 끊긴 턴은 `TurnEnd` 로 닫힌다(S1 · S6) · F4 = 부속 줄은 신호 없음 · 늦은 델타 버림 |
| 오류 뒤 멈춤 `last_end_failed`(ADR-0231 · 0234) | F2 | 끊김 = `Ended(Other)` — 세우지도 풀지도 않는다(S2) |
| 대기 입력 한 줄기 · 글은 늘 보인다(ADR-0231 · 0235 결정 2) | F2 | 대기분이 다음 턴에 돈다(S3) · `interrupt:` 응답은 번역 안 함 |
| 락 순서(ADR-0006) | F2 | 끊기 줄은 입력 큐만 탄다 — 새 락 간선 없음 |
| 백엔드 지식은 `backend` 한 곳(ADR-0004) | F2 · F3 · F4 | 줄 모양 · 도구 이름 표 · `commandActions` 해석 · 부분 메시지 규칙 전부 `backend/{claude,codex}` · 통로·프론트는 중립 값만 |
| replay→live · 누산기 멱등 | F3 · F4 | 묶음은 렌더 파생(누산기 상태 아님) · 델타도 같은 `TextDelta` 어휘 |
| 행 종류 ↔ 레일(ADR-0051) | F3 | 레일을 행 목록으로 계산 · 묶음 안에 레일 행 없음 |
| 제어 표면 하나 · 새 전역 핸들 금지(「LLM-우선 제어」) | F1 · F2 · F3 | 레지스트리 명령 셋 · 모듈 맵 · DOM 표지 |
| 전역 단축키 가드(`keybindings.ts:11-35`) | F2 | 손대지 않는다 |
| PTY 무변경 | F2 · F4 | 터미널 인자·`TerminalSlot` 무변경 |

---

## 7. 구현 순서 · 워커 분할 — 파일 겹침으로 가른다

★**어디서 멈춰도 빌드가 서게 짠다** — 각 단계가 끝나면 그 워커의 게이트가 초록이고 커밋이 있다. 자료구조를 바꾸는 단계는 그 생성 자리를 같은 커밋에서 다 맞춘다.★ 위임 전 되돌릴 지점 = 이 TRD 커밋.

### 백엔드 — 워커 하나, 순차 (`backend/claude/mod.rs` 를 F2 · F3 · F4 가 다 건드린다)

| 단계 | 내용 | 파일 | 끝났을 때 |
|---|---|---|---|
| **B1** F4 | 인자 · `PartialMessage` · `consume_line` 인자 · fixture · 시험 1–12 · §5-4 확인 | `claude/mod.rs` · `claude/fixtures/*` | `cargo test -p engram-dashboard-agent -- --test-threads=4` 초록 · 기존 fixture 시험 무수정 |
| **B2** F2 스파이크 | §3-5 S1–S6 · 보고서 · fixture 채취 | 스크래치 하네스 · `docs/research/…` · `claude/fixtures/interrupt_s1.jsonl` | ★출구 조건 미달 → 멈추고 메인에 반환(B3 의 claude 부분 착수 금지)★ |
| **B3** F2 구현 | ⓐ 버스 `agent.interrupt`(스파이크와 무관 — B2 가 멈춰도 이것은 간다) ⓑ `with_interrupt` · 끊기 줄 · `result` 분류 · `TurnEnd{Interrupted}` · 두 주석 갱신 | `commands.rs` · `agent/bindings/*` · `tests/command_declarations.rs` · `transport/stdio.rs` · `claude/mod.rs` · `types.rs`(주석) | 위 게이트 + `cargo test -p engram-dashboard-daemon -- --test-threads=4` |
| **B4** F3 백엔드 | `ToolCategory` 두 벌 · wire 칸 · 데몬 변환 · 생성 자리 전부 · claude 표 · codex 판정 · 생성물 | `types.rs` · `protocol/src/messages.rs` · `protocol/bindings/*` · `daemon/src/connection_core.rs` · `daemon/src/agent_conn.rs` · `output_core.rs` · `claude/mod.rs` · `codex/decoder.rs` | `cargo test --workspace -- --test-threads=4` 초록 · 생성물 sync |

### 프론트 — 워커 둘

- **FE-1.0**(FE-1 의 첫 커밋 · FE-2 는 이 커밋 뒤에 시작): 공유 파일 둘을 한 번에 친다.
  - `src/i18n/ko.ts` 키(전부): `agent.interrupt: '응답 중단'` · `slot.scrollToBottom: '맨 아래로'` · `chat.toolGroupSetExpanded: '도구 묶음 펼치기·접기'` · `chat.toolGroupSearch: '검색 {count}'` · `chat.toolGroupRead: '읽기 {count}'` · `chat.toolGroupList: '목록 {count}'` · `chat.toolGroupEdit: '편집 {count}'` · `chat.toolGroupCommand: '명령 {count}'` · `chat.toolGroupWeb: '웹 {count}'` · `chat.toolGroupAgent: '에이전트 {count}'` · `chat.toolGroupMcp: 'MCP {count}'` · `chat.toolGroupOther: '기타 {count}'` · `chat.toolGroupErrors: '오류 {count}'`(키는 두 단 — `src/i18n/index.ts` 의 `StringKey`).
  - `src/commands/contributions.ts:8-13` 에 `import './scrollCommands'` · `import './chatCommands'` + 두 파일을 머리 주석만 있는 빈 모듈로.
- **FE-1**(RichSlot 소유 · 순차): FE-1.1 F1(§2 전부 · DomSlot) → FE-1.2 F2 프론트(`agentCommands.ts` · RichSlot Esc · 오버레이 표지 넷) → FE-1.3 F3 접착(`slotId` · `bind`/`clear` · `onGroupToggle` → `unpin`, FE-2 의 스토어 착지 뒤).
- **FE-2**(FE-1 과 병렬): `chat/toolRuns.ts` · `store/toolGroupStore.ts` · `StructuredTextView.tsx` · `structuredAccumulator.ts` · `commands/chatCommands.ts`.
- 겹침 점검: FE-1 ∩ FE-2 = ∅(FE-1.0 뒤) · RichSlot = FE-1 만 · StructuredTextView = FE-2 만.

### 메인이 먼저 못 박는 접점

| 접점 | 모양 | 쓰는 쪽 → 읽는 쪽 |
|---|---|---|
| TS `ToolCategory` | `"Read"\|"Search"\|"List"\|"Edit"\|"Command"\|"Web"\|"Agent"\|"Mcp"\|"Other"` · ToolCall `category?` | B4 → FE-2 |
| `StructuredItem` `tool` | `{ kind:'tool'; name; argsJson; id; category: ToolCategory; itemId }` | FE-2 내부 |
| `StructuredTextView` props | `{ items; streaming?; slotId?: string; onGroupToggle?: (isLast: boolean) => void }` | FE-2 → FE-1.3 |
| `useToolGroupStore` | §4-4 | FE-2 → FE-1.3 |
| `FollowHandle` | `{ pinned: boolean; pin(): void; unpin(): void }` | FE-1.1 → FE-1.3 |
| 버스 `agent.interrupt` | `{target}` → `{outcome:"requested"}` · NOT_FOUND · CONFLICT | B3 → (LLM · CLI) |

---

## 8. 검증 계획

### 8-1. 기계 게이트 (`/qa`)

- 백엔드 = 각 단계 표의 명령 · 마지막 `cargo test --workspace -- --test-threads=4` · `cargo fmt --check` · 생성물 sync(`protocol/bindings` · `agent/bindings`).
- 프론트 = `npm test` · `npx tsc --noEmit`.
- 격리 게이트 = CLAUDE.md 「빌드·검증 명령」 그대로(`use tauri` 0 줄 등) — 이 라운드는 crate 경계를 옮기지 않는다.

### 8-2. GUI 실측 (`scripts/cdp.mjs` · `/qa full` 격리 인스턴스 · ★셸에서 직접 띄우지 않는다★)

| 기능 | 무엇을 재나 |
|---|---|
| F1 | ① 떨어진 채 탭을 돌렸다 돌아오기 — `scrollTop` 이 `display:none` 을 지나 남나(조사 §7 미검 — 코어의 되살리기가 필요했는지 기록) · 붙은 채 돌아오면 바닥 ② 창 새로고침·재구독 replay 폭주 뒤 바닥 착지 ③ 스트리밍 중 위로 휠 → 안 끌려 내려옴 · `data-scroll-follow="free"` ④ 펼친 생각 블록 안 휠 → 바깥 안 풀림 ⑤ 대기 목록이 서며 뷰포트가 줄 때 바닥 유지 ⑥ 버튼 150 ms 뒤 등장 · 누르면 바닥 ⑦ `__engramCmd('slot.scrollToBottom', {slotId})` |
| F2 | ① 실 codex 도구 중 Esc → 중단 행 · 대기 글이 다음 턴 ② (B3 뒤) 실 claude 글 흐르는 중 · 도구 중 Esc → 중단 행 · 오류 행 없음 · 대기 글이 돈다 ③ 한글 조합 중 Esc = 조합만 취소 ④ 안 돌 때 Esc = 무동작 ⑤ 우클릭 메뉴 열린 채 Esc = 메뉴만 닫힘 ⑥ 입력창 글 유지 ⑦ 본문 클릭 뒤 Esc(U6) ⑧ `engram agent interrupt <이름>` |
| F3 | ① 실 claude 읽기·검색 ≥2 턴 — 도는 동안 펼침 · 글이 오면 접힘 · 요약 문구 ② 실패하는 도구 → 오류 수 ③ codex 묶음 개수 ④ 토글이 재구독 뒤에도 남음 ⑤ 같은 이력 replay → 같은 묶음 |
| F4 | ① 실 claude JSON 에서 글이 점점 늘어난다 ② 블록 끝에 글이 두 벌 안 됨 ③ 도구·생각 한 번씩 ④ 그 세션 이어받기 → 글 한 벌 |

---

## 9. 위험 · 알려진 한계

- **F1 내용 노드 = Radix 첫 자식**: Radix 가 래퍼 구조를 바꾸면 성장을 못 본다(뷰포트 RO 는 계속 돈다). 증상 = 스트리밍이 바닥에 안 붙음 → 훅 시험이 첫 자식 부재를 경고로 남긴다. 대안(내용 ref 를 내려보내기)은 두 슬롯 수정이 필요하다.
- **F4 링 압박**: 토큰 단위 `TextDelta` 로 링(`REPLAY_MAX_EVENTS = 4096` — `output_core.rs:1291`)이 더 빨리 찬다 → 긴 대화의 앞 이력이 replay 에서 더 일찍 밀린다. B1 fixture 채취 때 한 턴의 델타 수를 기록하고, 문제면 번역 호출 안의 이웃 델타 합치기를 후속으로(조사 §4-4).
- **F4 렌더 비용**: 델타마다 `setItems`(`RichSlot.tsx:215`) + F1 RO. codex 가 오늘 같은 경로를 이미 탄다.
- **F2 init 전 끊기**(S4): 2.1.280 에서 재현되면 턴 첫 1 초 안의 Esc 가 무동작일 수 있다.
- **F2 능력 표시**: claude JSON 의 `control.interrupt` 가 참이 되면 트리 `canInterrupt`(`mergeTreeNodes.ts:94`)도 참 — 오늘 소비자가 없다.
- **F3 오류 수는 claude 만**(U2) · 펼침 상태는 인메모리(새로고침에 초기화).
- **F3 벤더 도구 이름 표류**: claude 가 도구 이름을 바꾸면 `Other` 로 떨어진다(묶음은 그대로 선다 — 요약 문구만 「기타」).

---

## 10. ADR 후보 (번호 = `/adr` 가 채번 · 가안은 0237 부터 — `docs/decisions` 마지막 = 0236 확인)

| 가안 | 결정 | 거부한 대안(사용자·메인이 확정) |
|---|---|---|
| **0237** | 채팅 칸 Esc = 도는 턴 끊기 · 명령 `agent.interrupt`(창 + 버스) — ★ADR-0235 결정 10(「UI 정지 버튼·단축키는 지금 넣지 않는다」)을 번복★(그 ADR 에 개정 도장) | 전역 단축키 표(가드를 연다) · 입력창만(U6 답에 따라) · 두 번 Esc |
| **0238** | claude JSON 끊기 = 통로 주입 제어 줄 · 끊긴 `result` → `TurnEnd{Interrupted}`(오류 아님 · 멈춤 불변) · 대기분은 다음 턴(`cancel_queued` 안 씀) | 세션 입력 자물쇠 경로(★①★) · `MessageDone` 유지(★②★) · 대기분 취소 |
| **0239** | `ToolCall.category` 중립 선 칸 — 번역기가 정하고 프론트는 모르면 「기타」 | 프론트 이름 표(벤더 지식 누수) · 백엔드가 묶음을 만든다(표시 관심사를 선에) |
| **0240** | claude 부분 메시지 중복 제거 = 열린 블록이 흘렸으면 완결 글을 버린다 · 턴 끝은 `result` 그대로 | 교체(새 선 변형 + replay 교체) · 꼬리 비교(paseo — 중복 모서리) · 합치기 |
| (선택) **0241** | 스크롤 따라가기 = 직접 쓴 순수 코어 · 성장 뒤 다시 재지 않는다 | `use-stick-to-bottom`(Radix 휠 불일치 의심 · 전역 청취자) · CSS 만 · 가상화 |

- 함께 고칠 문서: `types.rs:70` · `claude/mod.rs:551` · `:762-767` 주석 · CLAUDE.md 「핵심 불변식」(claude 끝 어휘에 끊김 `TurnEnd` 한 갈래) — 착지 라운드에서 `/review doc`.

---

## 11. ★메인 확인 필요★ 모음

| # | 무엇 | 왜 올리나 |
|---|---|---|
| ① | claude 끊기 줄을 `MidTurnPolicy`·세션 입력 자물쇠가 아니라 **통로 주입**(`StdioTransport::with_interrupt`)으로 보낸다(§3-4) | 지시는 `cancel_async_message` 경로를 따르라였다. 그 경로로 가면 능력(`control.interrupt`)이 통로 caps 에서 오는 구조와 어긋나고, 입력 id 가 없어 자물쇠가 지킬 순서도 없다 |
| ② | 끊긴 claude 턴을 `MessageDone` 대신 `TurnEnd{Interrupted}` 로 닫는다 · 오늘의 `subtype:"interrupted"` 도 같이(§3-4) | `types.rs:70` · `claude/mod.rs:551` 주석과 부딪히고, 오늘 구분선만 그리던 경우에 중단 행이 붙는다. 이득 = codex 와 같은 중단 표시 · 오류 뒤 멈춤 불변 |
| ③ | S4 에서 init 전 끊기가 무동작이면: 받아들인다(문서화) vs init 을 볼 때까지 끊기를 미룬다(§3-5) | 미루기는 번역기↔통로 사이에 새 공유 상태가 필요하다 |
| ④ | S2 에서 `subtype`·`terminal_reason` 이 끊김을 못 가르면 「우리가 보냈다」 표식으로 가른다(§3-4) | 끊기와 겹친 진짜 오류를 끊김으로 접어 가릴 수 있다 |
| ⑤ | 사람이 마지막이 아닌 도구 묶음을 펼치면 따라가기를 푼다(§4-5) | 지시(「RO 가 처리한다」)에 더한 것 — 빼면 붙은 채 펼친 묶음 머리가 화면 위로 밀려난다 |
