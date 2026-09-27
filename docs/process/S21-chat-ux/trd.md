# TRD — 챗 화면 4건: 스크롤 따라가기 · Esc 끊기 · 도구 호출 묶기 · claude 글자 스트리밍 (S21)

> 상태: **3판(2026-09-27) — 사용자 결정 U1–U6 반영 · U2 설계 추가(§4-7 · §7 B5) · `/review trd` 대기 · 코드 무변경.** 2판 = `/review trd full` 1 라운드 FIX 반영(두 리뷰어 FIX · 불일치 없음 · light 재검 PASS). PRD 는 건너뛰었다(동작은 직전 세션 인계 메모에서 합의 — 사용자 2026-09-27). 설계 결정은 오케스트레이터가 내렸고 이 문서는 그것을 구현 가능한 명세로 옮긴다. 결정과 다르게 가야 할 곳은 「★메인 확인 필요★」로 올렸고, 2판에서 메인 판정을 받아 본문에 반영했다(모음·판정 = §11 — 3판 새 ⑥⑦ 은 판정 대기). 사용자 체감이 갈리는 U2 세부 한 건은 「★사용자 확인★」으로 올렸다(§4-7 ⑧).
>
> **입력:** [조사 보고서](../../research/chat-ux-four-features-2026-09-27.md)(사실의 정본 — 여기서 되풀지 않고 `조사 §n` 으로 가리킨다). 판독 기준 = 브랜치 `v0.3.2/feat/chat-ux` 머리 `3f06e08`. 이 문서의 `파일:줄` 은 전부 그 커밋에서 직접 열어 확인했다 — 3판이 더한 것은 `095e027` 에서 열었다(`3f06e08` 뒤 커밋은 문서뿐이라 코드 줄은 같다). 벤더 = claude 2.1.280(이 PC 설치본) · codex app-server 스키마(t3code 생성본 `packages/effect-codex-app-server/src/_generated/schema.gen.ts` 판독).
>
> **앵커:** ADR-0004(백엔드 지식 격리) · ADR-0012(시험대) · ADR-0044/0045(stream-json · 정제는 백엔드) · ADR-0051(행 종류 ↔ 레일) · ADR-0053(ScrollArea seam) · ADR-0055/0167(명령 레지스트리 · `help` 부재 = 창 안 전용) · ADR-0056(탭 keep-alive) · ADR-0113/0127(턴 관측) · ADR-0155(명령 선언은 생산자 옆) · ADR-0203(codex 이력 item — 어휘표 한 벌) · ADR-0231/0234/0235(턴 도중 입력).
>
> 표기: **[고름]** = 사용자 체감이 없는 내부 구현이라 이 문서가 골랐다. **U#** = 사용자 결정(§1).

---

## 0. 결론 (먼저)

| 기능 | 무엇을 | 덩치 · 층 | 멈춤 조건 |
|---|---|---|---|
| **F1 스크롤 따라가기** | 두 슬롯(RichSlot · DomSlot)의 무조건 `scrollTop = scrollHeight` 를 공용 훅 하나 + DOM 없는 순수 코어로 바꾼다. 붙음 플래그는 스크롤 이벤트와 위로 가는 입력으로만 바뀐다 | 프론트만 | 없음 |
| **F2 Esc 끊기** | JSON 채팅 슬롯 안에서 맨 Esc = 도는 턴 끊기. 명령 `agent.interrupt`(창 + 데몬 버스). claude JSON 끊기를 새로 짓는다(U1) | 프론트 + agent crate | ★claude 부분은 스파이크(§3-5)가 출구 조건을 못 채우면 멈추고 사용자에게 돌아간다★ |
| **F3 도구 호출 묶기** | 연속 도구 행 ≥2 를 렌더 시점 순수 함수로 묶는다. 종류는 각 번역기가 `ToolCall` 에 싣는 중립 `category`. 묶음 머리 「오류 N」은 두 백엔드 다 — codex 는 도구 끝 상태를 새 선 변형 `ToolResult` 로 옮긴다(U2 · §4-7) | 선 타입 1 필드 + 새 변형 1 + 두 번역기 + 프론트 | ★B5 채취에서 0 아닌 종료가 `completed` 로 오면 멈추고 메인에 올린다(§4-7 ⑨)★ |
| **F4 claude 글자 스트리밍** | `--include-partial-messages` + 번역기 상태(메시지 id · 열린 블록 · 흘린 블록). 흘린 블록의 완결 본문은 버린다 | 백엔드만 · 선 타입·프론트 무변경 | 없음 |

- **구현 순서**(§7): 백엔드 한 워커 = F4 → F2 스파이크 → F2 구현 → F3 백엔드(B4) → U2 codex 끝(B5)(각 단계 빌드 초록 · 커밋). 프론트 두 워커 = FE-1(F1 → F2 프론트 → F3 접착) ∥ FE-2(F3 프론트 — 지역 타입 · 가드로 B4 · B5 를 안 기다린다). 공유 파일은 FE-1.0 이 먼저 한 번에 친다. ★B5 는 FE-2 의 가드 커밋 뒤에만 커밋한다★(새 변형이 프론트 `never` 망라를 건다 — §11 ⑥). B4 · B5 · FE-2 뒤 I1 이 생성물 타입으로 바꾼다.
- **사용자 결정 U1–U6 = 완료(2026-09-27 · §1).** 다섯은 추천안 그대로다. ★U2 만 추천과 달리 「이번에 한다」★ → codex 도구 끝을 새 선 변형 `ToolResult{id, outcome}` 로 번역하고 실패·거부만 낸다 · 두 턴 분류기에서 신호 없음 · `PROTOCOL_VERSION` 은 안 올린다(§4-7). U6 과 함께 단축키 시스템은 보류(`docs/tracking.md` T-36).
- **★사용자 확인★ 1 건**: codex `declined`(실행되지 않은 호출)를 어떻게 보이나 — 추천 = 「거부됨」 따로 · 대안 = 오류로 함께 센다(§4-7 ⑧).
- **★메인 확인 필요★ — 2판 5 건 = 판정 완료**(§11): ①②⑤ 수락 · ③ 턴 열림 문(§3-4)으로 해소 · ④ 스파이크가 끊김을 못 가를 때만 쓰는 예비. 가장 큰 것은 claude 끊긴 턴을 `MessageDone` 대신 `TurnEnd{Interrupted}` 로 닫는 것(②) — `types.rs:70` 주석을 함께 고친다. **3판 새 2 건 = 대기**: ⑥ B5 ↔ FE-2 커밋 순서 · ⑦ claude 결과 중립화를 후속 추적 항목으로 적립.

---

## 1. 사용자 결정 (U1–U6) — 2026-09-27 결정 완료

각 항목 = 사용자 결정 · 추천안 · 한 줄 트레이드오프 · 답에 따라 바뀌는 자리. U2 를 뺀 다섯은 추천안 그대로다.

| # | 질문 | 사용자 결정(2026-09-27) | 추천 | 대안 | 트레이드오프 | 바뀌는 자리 |
|---|---|---|---|---|---|---|
| **U1** | claude JSON 끊기를 이번에 짓나 | **짓는다** — 「claude 쪽 기능 확인해서 적용」(스파이크 먼저 — §3-5 · 설계 그대로) | **짓는다** | codex 만 먼저 | 짓지 않으면 Esc 가 claude 슬롯에서 아무 일도 안 한다(능력 `false`) — 기본 백엔드가 claude 라 체감 대부분이 빈다. 짓는 비용 = 스파이크 + 번역기 한 갈래 + 통로 주입 | 대안이면 §3-4 · §3-5 · §7 B2–B3 의 claude 부분을 뺀다. 버스 명령 · 프론트 키는 그대로 |
| **U2** | codex 묶음에 오류·끝 표시를 이번에 하나 | ★**이번에 한다 — codex 묶음에도 오류 수**(추천과 다름)★. 사용자 근거: 관례(codex TUI 의 「· N failed」 · t3code)는 요약의 개수 + 펼치면 어느 호출이 실패했나이고, codex 번역기는 끝 상태를 싣는 `item/completed` 만 버리고 있다 | 안 한다 — codex 묶음은 개수만 | codex `item/completed` 를 결과 사건으로 번역(**채택**) | 하면 새 선 변형이 필요하고 **옛 셸이 새 데몬을 만나면 결과마다 「표시할 수 없는 신호」 줄을 그린다**(조사 §3-3 — 실패·거부만 내서 줄인다 · §4-7 ③ · ⑤). 안 하면 codex 묶음 머리에 오류 수가 없다 | 새 선 변형 `ToolResult` + codex 번역기 `ItemOrigin::Completed`·`History` 갈래(`backend/codex/decoder.rs:623`) + 누산기 갈래 → **§4-7 · §7 B5** |
| **U3** | 「맨 아래로」 버튼 | **바닥이 아니면 늘** | **바닥이 아니면 늘 · 셰브론 · 개수 없음 · 보이기 150 ms 늦춤** | 위로 올린 동안 새 내용이 왔을 때만 | 피어 전부가 추천안이다(조사 §1-2). 대안은 「읽다 멈춘 자리」에서 버튼이 안 떠 돌아갈 길을 못 찾는다 | 상수 `JUMP_BUTTON_MODE` 하나(§2-4). 대안이면 코어의 `unseenGrowth` 를 버튼 조건에 건다 |
| **U4** | 보내면 다시 바닥에 붙나 | **붙는다** | **붙는다** | 그대로 둔다 | 붙지 않으면 위를 읽다 보낸 글의 답이 화면 밖에서 흐른다. 붙으면 읽던 자리를 잃는다(paseo 선례 = 붙는다) | 상수 `REPIN_ON_SEND` 하나(§2-4) |
| **U5** | 도구 호출 사이의 (비지 않은) 생각 블록 | **흡수** | **묶음에 흡수** | 묶음을 끊는다 | 끊으면 생각이 도구마다 끼는 claude 턴에서 묶음이 거의 안 선다. 흡수하면 펼쳐야 생각이 보인다(codex TUI 선례 = 흡수) | `groupToolRuns` 의 흡수 판정 한 줄(§4-3) |
| **U6** | Esc 가 먹히는 범위 | **칸 안 어디든** · 단축키 시스템은 보류(`docs/tracking.md` T-36) | **그 채팅 칸 안 어디든**(입력창 + 대화 본문 · 칸 루트가 클릭으로 포커스를 받는다) | 입력창 포커스일 때만 | 추천안은 본문을 클릭해 읽다가도 끊을 수 있다. 대안은 피어(Claude Code · codex · paseo)와 같고 포커스 변화가 없다 | 상수 `ESC_SCOPE` 하나 + 루트 `tabIndex` 한 줄(§3-2) |

- 두 번 Esc(글 지우기 · 되감기)는 **이번 범위 밖**이다.
- **단축키 시스템은 보류**(U6 과 함께 · `docs/tracking.md` T-36). Esc 는 지금 명령 `agent.interrupt` 를 부르는 **지역 술어** 하나이고(§3-2), T-36 이 키바인딩 표를 세우면 그 표의 한 줄로 옮긴다 — 끊는 동작은 명령에 있어 손댈 것이 없다. ADR-0237(가안)이 이것을 거부한 대안 「단축키 시스템을 지금 만든다」로 적는다(§10).

---

## 2. F1 스크롤 따라가기

### 2-1. 바뀌는 자리

| 파일 | 지금 | 바뀜 |
|---|---|---|
| `src/components/slot/RichSlot.tsx:101` · `:288-291` | `scrollRef` + `[items]` 마다 무조건 바닥 | `const follow = useScrollFollow(viewId)` · `<ScrollArea ref={follow.viewportRef}>`(`:398`) · 효과 삭제 |
| `RichSlot.tsx:153` · `:251` | 구독 초기화 · 비우기(onReset) | 두 자리에서 `follow.pin()` — 비운 뒤 오는 이력이 바닥에 착지. ★구독 효과 deps 는 `[viewId, agentId]` 그대로(`:283` — CLAUDE.md micro-rule)★ — `follow`·`pin` 을 넣지 않는다(§2-3: 손잡이는 안정적이다) |
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
  savedTop: number | null    // 얼 때 적은 scrollTop — 풀릴 때 되살린다. 처음·풀린 뒤 = null
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

1. **얼음**: 얼지 않은 상태에서 `m.client === 0` 이 오면 `frozen = true` · **`savedTop = expectTop ?? lastTop`**(그 순간의 `m.top` 은 이미 무효일 수 있어 마지막으로 유효하게 본 값을 적는다) · `scrollTo = null`. 얼어 있는 동안 `scroll`·`resize`·`attach`·`intentUp` 은 아무 칸도 안 바꾼다(규칙 2 의 풀림만 예외). `pin`·`unpin` 은 얼어 있어도 `pinned` 만 바꾼다(적용은 풀릴 때).
2. **풀림**(`frozen` 이고 `m.client > 0` 인 `resize`·`attach`): `frozen = false` · `lastTop = m.top`. 붙어 있으면 바닥으로 쓴다. 떨어져 있으면 `savedTop !== null` 이고 `m.top !== savedTop` 일 때만 `savedTop` 으로 쓴다(`null` 이면 쓰지 않는다) — `display:none` 을 지나 `scrollTop` 이 보존되는지 모르므로(조사 §1-1) 되살리기를 늘 건다(보존되면 무동작). 끝에 `savedTop = null`.
3. **우리 쓰기는 절대 풀지 않는다**: `scroll` 의 `m.top` 이 `expectTop` 과 1px 안이면 `lastTop` 만 옮기고 `expectTop = null`.
4. **사용자 스크롤**: 그 밖의 `scroll` — 바닥 거리 `d = height − top − client`. 끝에 늘 `lastTop = top`.
   - 붙어 있을 때: `d > 문턱` 이고 `top < lastTop`(위로 움직였다) → `pinned = false`. 그 밖 → 그대로(문턱 안의 위 움직임은 내용 줄어듦의 클램프일 수 있다 — 풀지 않는다).
   - **떨어져 있을 때 = 위 의도 걸쇠**: 되붙기는 **아래로 움직여(`top > lastTop`) 문턱 안에 든** `scroll` 에서만 — `pinned = true` · `unseenGrowth = false`. 위로·제자리·문턱 밖 아래로는 그대로 떨어져 있다. ★문턱 안의 작은 위 휠이 규칙 5 로 푼 뒤, 그 휠이 만든 `scroll`(위로 · 문턱 안)이 곧바로 되붙이지 않게 하는 것이 이 걸쇠다★. 그 밖에 걸쇠를 푸는 길은 명시 `pin`(버튼 · 보냄 U4 · 비우기 · 명령)뿐이다.
5. **위로 가는 입력**(`intentUp`): `height > client`(스크롤할 것이 있다)면 `pinned = false`. 문턱 안의 작은 휠도 곧바로 푼다 — 그래야 성장이 끌어내리지 않는다(되붙기는 규칙 4 의 걸쇠가 막는다).
6. **성장**(`resize`): ★붙음을 다시 재지 않는다★(조사 §1-3 — 재면 스트리밍 성장이 「위로 올렸다」로 읽힌다). 붙어 있으면 `scrollTo = max(0, height − client)`. 떨어져 있으면 `unseenGrowth = true`.
7. **쓰기 예고**: `scrollTo` 를 낼 때 `|m.top − scrollTo| > 0.5` 일 때만 `expectTop = scrollTo` — 이미 그 자리면 이벤트가 안 오므로 예고를 남기지 않는다(남기면 뒤의 사용자 스크롤을 우리 것으로 오인할 수 있다).
8. 처음 상태 = `pinned: true` · `frozen: false` · `expectTop: null` · `savedTop: null`.

### 2-3. 훅 — `useScrollFollow(slotId)`

- 돌려주는 것: `{ viewportRef: (el: HTMLDivElement | null) => void, pinned: boolean, pin(), unpin() }`. `pinned` 은 값이 바뀔 때만 React 상태를 올린다(스크롤 이벤트마다 리렌더하지 않는다).
- ★`viewportRef`·`pin`·`unpin` 은 렌더마다 같은 함수다★ — 코어 상태·노드를 ref 에 두고 그 ref 만 읽는다(마운트 수명 동안 정체성 불변). 그래서 구독 효과(deps `[viewId, agentId]`)가 안에서 불러도 옛 함수를 쥘 걱정이 없고, ★그 deps 에 넣지 않는다★(넣으면 손잡이 정체성이 흔들릴 때 재구독 → replay 가 돈다).
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

- `followCore.test.ts`(표 시험 — 위 규칙 1–8 각각 + 조합): 우리 쓰기 뒤 이벤트는 안 푼다 · 떨어진 뒤 아래로 문턱 안에 들면 되붙기 · ★걸쇠 — 문턱 안 작은 위 휠(`intentUp`) → 그 휠의 위 `scroll`(문턱 안) → 여전히 떨어짐 · 이어 아래로 문턱 안 `scroll` → 붙음★ · 떨어진 채 제자리·위 `scroll` 은 되붙이지 않음 · 명시 `pin` 은 걸쇠를 푼다 · 붙은 채 문턱 안 위 `scroll`(클램프)은 안 푼다 · 성장 중 풀리지 않음 · 얼 때 `savedTop = expectTop ?? lastTop` · 얼음 동안 스크롤·RO·`intentUp` 무시 · 풀릴 때 붙음=바닥 / 떨어짐=`savedTop` / 떨어짐 + `savedTop` 없음 = 쓰기 없음 · 풀린 뒤 `savedTop = null` · 스크롤할 것 없는 휠은 안 푼다 · 이미 바닥인 쓰기는 예고 없음.
- `useScrollFollow.test.tsx`(가짜 RO · `Object.defineProperty` 로 `scrollHeight`·`clientHeight`·`scrollTop`): 노드 교체 뒤 재부착 · 안쪽 Radix 뷰포트 휠 무시 · `data-scroll-follow` 값 · 리렌더 뒤 `viewportRef`·`pin`·`unpin` 정체성 불변.
- `scrollCommands.test.ts`: 손잡이 있음/없음 · `help` 가 없다(버스 미승선 — `renderModeCommands.test.ts` 의 같은 단언 모양).
- 기존 `RichSlot.test.tsx` · `DomSlot.test.tsx` 는 무조건 스크롤을 단언하지 않는다(`rg scrollTop src --glob '*.test.*'` 의 슬롯 쪽 적중은 RO 가짜뿐) — 회귀로 그대로 돈다.

---

## 3. F2 Esc 끊기

### 3-1. 바뀌는 자리

| 파일 | 바뀜 |
|---|---|
| `src/commands/agentCommands.ts`(`:126-158` 옆) | `agent.interrupt { agentId }` — `agentClient.interruptAgent`(`src/api/protocolClient.ts:988-992`)를 부른다. ★`help` 없음★ — 같은 이름을 데몬이 버스에서 답한다(`:130-133` 의 `cancelQueuedInput` 주석과 같은 사유). 입력 임대 거절은 `:146-155` 와 같은 `CONFLICT:` 접두로 편다 — [고름] 그 매핑을 작은 함수로 뽑아 두 명령이 함께 쓴다. 돌려주는 값 = `{ outcome: 'requested' }` |
| `src/components/slot/RichSlot.tsx:373-392`(루트) | `onKeyDownCapture` 로 Esc 처리(§3-2) · U6(칸 안 어디든)이라 `tabIndex={-1}` + `outline-none` |
| 새 `src/components/slot/interruptKey.ts` | §3-2 의 조건 전부를 담은 순수 술어 하나 |
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
- (`ESC_SCOPE = 'input'` 일 때만) `e.target === textarea`

[고름] 위 조건 전부를 **순수 술어 하나** `isInterruptEscape(e, ctx)`(`interruptKey.ts` — `ctx` = `streaming` · `agentUnavailable` · `canInterrupt` · `overlayOpen`(루트 처리기가 위 `querySelector` 로 재서 넘긴다 — 술어는 DOM 을 안 읽는다) · `scope` · 입력창 노드)에 모은다. 루트 처리기는 그 술어가 참이면 명령을 부르기만 한다. 이 술어가 T-36(단축키 시스템 — 보류)이 세울 키바인딩 표 한 줄의 `when` 으로 그대로 옮겨 간다(§1 U6).

발화하면 `e.preventDefault()` 하고 `fireAndForget('agent.interrupt', { agentId })`(`src/commands/dispatch.ts:15`) — 사람 키와 LLM 이 같은 핸들을 흔든다. ★낙관 상태를 바꾸지 않는다★ — 끊겼다는 것은 턴 끝 사건이 알린다(claude 의 응답 수신은 「멈췄다」가 아니다 — §3-5 S4). ★입력창 글은 건드리지 않는다.★

- 프론트 `streaming` 은 백엔드의 턴 상태와 양 끝에서 어긋난다 — 보낸 직후 낙관적으로 켜지고(`awaiting`) 턴 끝 사건이 올 때까지 늦게 꺼진다. 앞 끝(턴이 아직 안 열렸다)의 Esc 는 통로가 `Unsupported` 로 거절하고(codex · claude 턴 열림 문 — §3-4) `fireAndForget` 가 warn 으로 삼킨다 — 무동작이고 사용자는 다시 누르면 된다. 뒤 끝은 §3-4 의 잔여 경합이다.

- capture 인 이유: 입력창 `onKeyDown` 이 `e.stopPropagation()` 을 먼저 한다(`:471-472`) — bubble 로는 입력창의 Esc 가 루트에 안 온다. capture 는 입력창과 본문을 한 처리기로 덮는다.
- 상수 `ESC_SCOPE: 'slot' | 'input' = 'slot'`(U6 확정 = `'slot'`).

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
- `make_table`(`:531`)에 한 줄 · `verb_interrupt`: 공백 검문 → `resolve` → `host.interrupt_agent` → `Ok` = `requested` · 산 세션 없음 = NOT_FOUND · `PtyError::Unsupported`(codex 「중단할 턴이 없다」(`backend/codex/transport.rs:4045-4061`) · claude 턴 열림 문 닫힘(§3-4) · 능력 없는 통로) = CONFLICT(「끊을 턴이 없다 · 이 에이전트는 끊기를 지원하지 않는다」) · 그 밖 = INTERNAL.
- 생성물: `crates/engram-dashboard-agent/bindings/AgentInterruptArgs.ts` · `AgentInterruptOk.ts` · `commands.schema.json`(CI sync 게이트 대상).
- 시험: `tests/command_declarations.rs:61-70` 에 `INPUT_AFFECTING.contains(&"agent.interrupt")` · 표 시험(결말 셋 매핑).
- **덩치 판정**: 선언 1 · trait 메서드 1 · 동사 1 · 시험 — `cancelQueuedInput` 과 같은 몫이라 과하지 않다. 프론트만 가는 대안(버스 없이 창 명령만)은 필요 없다 — 적어 두면: `help` 없는 창 명령만 남기고 버스 LLM 은 끊기를 못 부른다.

### 3-4. claude JSON 끊기 (U1) — 설계

**제어 줄**: `{"type":"control_request","request_id":"interrupt:<uuid v4>","request":{"subtype":"interrupt"}}\n` — 모양은 공식 Agent SDK(조사 §2-1). `cancel_line`(`claude/mod.rs:698-724`)처럼 typed struct 로 직렬화한다. ★`cancel_queued` 같은 선택 능력은 싣지 않는다★ — 우리가 원하는 뜻은 codex 와 같다: **도는 턴만 멈추고, 이미 받아 둔 대기 입력은 다음 턴으로 간다**(ADR-0235 결정 4 — `Interrupted` 는 멈춤을 세우지 않는다 · 결정 5 — 남은 글은 다음 한 턴에). 그것이 CLI 응답의 `still_queued`(끊은 뒤에도 돌 대기 메시지 목록)의 뜻인지는 스파이크가 확인한다(§3-5 S3).

**줄이 통로에 닿는 길** — ①(메인 수락): 지시는 「`cancel_async_message` 가 닿는 길을 따라 하라」였다. 그 길은 `MidTurnPolicy::SessionClassified { cancel_line }`(`types.rs:215`)을 세션이 입력 자물쇠 안에서 `send_input` 하는 것이다(`session.rs:563-580`). 끊기는 그 길에 맞지 않는다: ① 능력 `control.interrupt` 는 **통로의 caps** 에서 온다(`session.rs:656-657` → `stdio.rs:425-455`) ② 끊기는 입력 id 에 묶이지 않아 입력 자물쇠가 지킬 순서가 없다. 그래서 **`structured` 주입과 같은 모양**을 쓴다(`stdio.rs:52-56` — 「구조화냐」를 backend 가 주입):

```rust
// transport/stdio.rs — 통로는 이 바이트의 뜻을 모른다(바보 파이프 유지)
/// `None` = 「지금은 끊을 턴이 없다」(backend 가 판정) — 통로는 그대로 `Unsupported` 로 옮긴다.
pub type InterruptLine = Arc<dyn Fn() -> Option<Vec<u8>> + Send + Sync>;
impl StdioTransport {
    /// 「지금 턴을 멈춰 달라」는 줄을 만드는 backend 함수를 꽂는다. 없으면 오늘처럼 `Unsupported`.
    pub fn with_interrupt(mut self, line: InterruptLine) -> Self { self.interrupt = Some(line); self }
}
// interrupt(): Some(f) => match f() { Some(b) => self.input.push(b) · None => Unsupported("끊을 턴이 없다") }
//              · None => 오늘의 Unsupported(`:371-376`)
// capabilities(): control.interrupt = self.interrupt.is_some()(`:441`) — 능력은 「끊을 수 있는 통로」이지 「지금 턴이 있다」가 아니다
```

- [고름] `open` 인자를 늘리지 않고 빌더로 둔다 — `StdioTransport::open(` 호출이 시험 포함 9 곳이다(`rg "StdioTransport::open\(" crates`). 기존 단언(`stdio.rs:507` · `session.rs:1249` — 주입 없는 통로는 `Unsupported`·`false`)은 그대로 참이다.
- 줄은 입력 큐(`stdio.rs:46-47` — 라이터 스레드 하나)에 들어가 사용자 줄과 **통째로** 직렬화된다(줄 섞임 없음). 입력 자물쇠(`input_order`)는 안 탄다 — 락 순서 불변식(ADR-0006)에 새 간선이 없다.
- `backend/claude/mod.rs` `open_spawn`(`:424-433`): `open` 은 튜플 `(StdioTransport, Option<u32>)` 을 돌려준다(`stdio.rs:72-76`) — 풀어 낸 뒤 꽂는다: `let (t, pid) = StdioTransport::open(spec, true, Some(decoder))?; let t = t.with_interrupt(interrupt_line(Arc::clone(&turn_gate)));`. claude 지식(줄 모양 · 턴 열림 판정)은 `backend/claude` 에만 산다(「백엔드 확장」).
- 응답 `control_response`(`request_id` 머리 `interrupt:`)는 **번역하지 않는다** — `cancel_response_event`(`:1215-1235`)는 `cancel:` 머리만 보므로 이미 `None` 이다. 응답이 왔다고 턴이 멈춘 것은 아니다(§3-5 S4).

**턴 열림 문** — codex 는 턴이 없으면 끊기를 거절한다(`backend/codex/transport.rs:4045-4061` — `Unsupported("… 중단할 턴이 없다")`). claude 도 같은 버스 계약을 따른다(§3-3 매핑 = CONFLICT):

- `backend/claude` 안의 화신 공유 값 `turn_gate: Arc<TurnGate>` — `DeliveryAck` 공유와 같은 모양이다(`open_spawn` `:424` 에서 하나 만들어 decoder(`stream_decoder` `:758`)와 끊기 줄 함수에 같은 `Arc` 를 준다). 선 타입·세션·통로는 이 값을 모른다 — `types.rs` 에 두지 않는다(`DeliveryAck` 는 세션이 읽어 거기 있다 · 이 값은 backend 만 읽는다).
  ```rust
  struct TurnGate { open: AtomicBool /* , interrupt_sent: AtomicBool — ④ 예비일 때만 */ }
  ```
- **세우는 쪽 = 라이브 decoder**: 한 라이브 줄이 턴 진행 신호 사건(`classify_turn` `:538-545` → `Progress` — 사용자 되울림 `Structured{user}` · `assistant` 블록 · 흘린 `TextDelta` · `command_lifecycle` `started` 의 `Delivered`)을 하나라도 내면 `open = true`. **`result` 에서 `open = false`**(오늘 번역 뒤). 이어받기(`LineSource::Transcript`)는 건드리지 않는다. [고름] 「턴 중」의 정의를 턴 관측과 같은 분류기 하나에서 뽑는다 — 두 정의가 갈리지 않는다.
- **읽는 쪽 = 끊기 줄 함수**: `open` 이면 `Some(줄)`(④ 예비면 `interrupt_sent = true` 도), 아니면 `None` → 통로 `Unsupported` → 버스 CONFLICT · WS `Interrupt` 는 오류 응답(`connection_core.rs:1222-1230`). ③ 은 이것으로 해소된다(메인 판정) — 턴이 열리기 전의 Esc 는 정직하게 거절되는 무동작이고, 미루는 큐를 두지 않는다. ★단 S4 가 「되울림이 `system/init` 보다 먼저 오고 그 틈의 끊기는 무시된다」를 보이면 문이 여는 지점을 **그 턴의 `system/init` 뒤 첫 진행 줄**로 늦춘다★(같은 값 · 같은 거절 — 여는 줄만 바뀐다).
- ★**잔여 경합 — 좁힐 뿐 닫지 못한다**★: 문을 읽고 줄을 큐에 넣는 사이, 또는 줄이 CLI 에 닿기 전에 그 턴의 `result` 가 나오고 **CLI 가 스스로 다음 턴을 열면**(대기 중이던 B · 우편이 나른 턴) 그 줄은 **다음 턴**을 끊는다. 프론트 `streaming` 이 턴 끝 사건까지 늦게 꺼지므로(§3-2) 늦은 Esc · 버스 호출이 이 틈에 든다. claude `control_request` 에는 턴 id 가 없어(codex `turn/interrupt` 는 `turn_id` 를 싣는다) **벤더 프로토콜로는 닫을 수 없다**. 증상 = B 가 한 번 끊긴 턴으로 닫힌다(중단 행 · 오류 아님 · 멈춤 불변 — 우편이 멈추지는 않는다). 스파이크 S7 이 크기를 잰다.

**끊긴 턴의 `result` 분류** — 지금 번역기(`:944-970`)는 `is_error || subtype.starts_with("error")` 면 `Error(RESULT_FAILURE_DETAIL…)` 를 낸다. 끊긴 턴의 `result` 가 `is_error: true` 에 `terminal_reason` = `aborted_streaming` | `aborted_tools` 로 올 수 있다(SDK `types.py` · SDK issue #429 — 메인 대조). 그대로 두면 Esc 한 번이 **오류 행을 그리고 오류 뒤 멈춤(`last_end_failed`)을 세워** 우편을 멈춘다 — 의도와 반대다. 그래서:

- `interrupted(result)` 판정(스파이크가 확정 — S2): `subtype == "interrupted"`(오늘도 오류 아님 — `:951-956`, 유지) **또는** `terminal_reason ∈ {"aborted_streaming","aborted_tools"}`. ④(메인 판정 = 예비로만 둔다): 두 칸 모두 끊김을 가르지 못할 때**만** 「이 결과 앞에 우리가 끊기를 보냈다」 표식 `TurnGate.interrupt_sent` 를 짓는다 — 끊기 줄 함수가 **문이 열려 있을 때만**(줄을 돌려줄 때만) 세우고, 번역기가 **그 턴의 `result`** 에서 읽고 지운다(문을 닫는 같은 자리). 그래서 한가할 때의 끊기 호출은 표식을 세우지 못해 다음 턴의 진짜 오류를 가리지 않는다. 남는 대가 = 끊기와 겹친 **같은 턴의** 진짜 오류는 끊김으로 접힌다 · 위 잔여 경합에 걸리면 표식은 앞 턴의 `result` 에서 쓰이고, 줄이 끊은 다음 턴은 오류로 읽힐 수 있다(그 턴에 오류 뒤 멈춤이 선다). 스파이크가 두 칸 중 하나로 가르면 이 칸은 짓지 않는다.
- 참이면 `Usage`(오늘처럼) 뒤 **`OutputEvent::TurnEnd { turn_id: None, outcome: TurnOutcome::Interrupted }`** 하나 — `Error` 도 `MessageDone` 도 내지 않는다.
  - 턴 분류기(`classify_turn` `:553-557`)가 이미 `Interrupted → Ended(Other)` 로 적는다 → 턴은 끝나고 · 오류 뒤 멈춤을 세우지도 풀지도 않는다(ADR-0234 「뜻 모를 턴 끝은 멈춤을 건드리지 않는다」).
  - 누산기(`structuredAccumulator.ts:213-220`)가 `outcome: 'interrupted'` 행(「응답이 중단됐습니다」 — `ko.ts:137`)과 구분선을 그린다 — codex 끊김과 같은 모양. 프론트·선 타입 무변경.
  - ②(메인 수락 — 턴 관측 `Ended(Other)` · `last_end_failed` 불변 · 초인종 울림 · 누산기 결말 행 · 대기 입력 골든 무영향 확인): `types.rs:70`(「`MessageDone` 을 이것으로 이주시키지 않는다 — claude 는 그대로 `MessageDone` 을 쓴다」)과 `claude/mod.rs:551`(「이 decoder 는 `TurnEnd` 를 내지 않는다」)이 이 변경과 부딪힌다. 이주가 아니라 **끊김 한 갈래만** `TurnEnd` 로 가는 것이고 같은 doc 이 「어느 쪽을 내는지는 각 decoder 가 정한다」고도 적는다. 또 오늘 `subtype:"interrupted"` 는 `MessageDone`(구분선만)으로 닫히는데 이 변경 뒤엔 중단 행이 붙는다(오류 아님은 그대로). 두 주석은 이 변경과 함께 고친다.
- **끊김 직전의 잘린 `assistant` 줄**(SDK issue #338): F4 뒤에는 흘린 블록의 잘린 완결 본문은 버려지고(사용자는 흘린 만큼을 이미 봤다) 안 흘린 블록은 잘린 본문 그대로 나간다. 끝은 위 중단 행이 표시한다.

### 3-5. 스파이크 (B2) — 출구 조건

**방법**: Phase 0 하네스(`.claude/handoff/attachments/20260925-midturn-phase0/claude_harness.js`)로 claude 2.1.280 을 **최종 스폰 인자**(F4 가 먼저 착지 — `--include-partial-messages` 포함)로 띄우고 `{t,dir,line}` 을 기록한다. 결과 = 새 보고서 `docs/research/claude-interrupt-spike-2026-09-2x.md` + fixture `backend/claude/fixtures/interrupt_s1.jsonl`(`fixtures/README.md` 의 가공 규칙).

| # | 시행 | 기록 | 통과 조건 |
|---|---|---|---|
| S1 | 글 흐르는 중 · 도구 도는 중 각각 끊기 | 끊은 뒤 오는 줄 전부 · `result` 의 `subtype`·`is_error`·`terminal_reason` | 턴 끝 줄(`result`)이 **반드시** 온다 |
| S2 | S1 의 `result` | 위 칸 | `subtype` 또는 `terminal_reason` 이 끊김을 가른다(아니면 ④ 예비 표식을 짓는다 — §3-4) |
| S3 | 도구 중 B 를 써서 `queued` 를 본 뒤 끊기 | 응답의 `still_queued` · B 의 `command_lifecycle` | B 가 `still_queued` 에 있고 · 뒤이어 B 의 `started` 가 새 턴에서 온다(B 가 돈다) · `cancelled`/`discarded` 가 오지 않는다 |
| S4 | 새 턴의 사용자 되울림 직후 · `system/init` 전에 끊기(하네스가 직접 쓴다 — 문을 거치지 않는다) | 되울림과 `init` 의 순서 · 응답 · 턴이 멈췄나 | 멈추면 통과(문은 되울림에서 연다). 되울림이 `init` 보다 먼저 오고 그 틈의 끊기가 무시되면(SDK issue #429 — 2.1.241 보고, 2.1.280 미검) 문이 여는 지점을 `init` 뒤 첫 진행 줄로 늦춘다(§3-4 — ③ 판정의 구현 조정 · 멈춤 사유 아님) |
| S5 | S1 중 잘린 `assistant` 줄 | 흘렸나 · 완결 줄이 왔나 | 기록만(§3-4 끝 문단의 전제 확인) |
| S6 | 턴이 없을 때 끊기(하네스가 직접 — 문이 막는 경로지만 S7 의 경합이 이 줄을 한가한 CLI 에 닿게 할 수 있다) | 응답 · 그 뒤 줄 | 턴 신호를 켜는 줄(`assistant`·`user`·`stream_event` 글)이 오지 않는다 — 오면 경합으로 닿은 한가 끊기가 「턴 중」을 켜 30 분 막힘 경로가 된다 |
| S7 | 도구 중 B 를 대기시키고 A 의 `result` 직후(B 의 턴이 열리기 전후) 끊기 | B 의 턴이 끊겼나 · B 의 `result` 모양 · 우리 문 값의 궤적 | 기록만 — §3-4 잔여 경합의 실물 크기. B 가 끊겨도 오류 행 · 오류 뒤 멈춤이 없으면 받아들인다(있으면 S2 의 판정이 그 모양을 못 가른 것 — 멈춘다) |
| S8 | 문 궤적 — S1·S3 채취본을 decoder 에 먹여 문을 여닫는 줄을 적는다 | 열림 = 첫 진행 줄 · 닫힘 = `result` | 턴 밖(한가 구간 · `result` 뒤)에 문을 여는 줄이 없다 |

**스파이크 결과에 기대는 불변식**(결과가 지저분하면 **작업을 멈추고 사용자에게 돌아간다**):

- 「턴 관측 정리 = 두 지점뿐」·30 분 fail-open(ADR-0127) — S1 · S6 · S8. 끊은 턴이 끝 줄 없이 남으면 `in_turn` 이 30 분 붙는다.
- 「오류 뒤 멈춤」 `last_end_failed`(ADR-0231 N11 · ADR-0234) — S2. 끊김이 실패로 읽히면 Esc 한 번이 우편을 멈춘다.
- 「대기 입력 상태 = 링 사건 한 줄기」 환원 규칙과 claude 수명주기 번역표(`claude/mod.rs:1164-1181` — `cancelled` → `Dropped{Unknown}` 묘비) · ADR-0235 결정 2 「글은 늘 보인다」 — S3. CLI 가 끊으며 대기분을 버리면 B 가 말풍선 없이 목록에서 사라진다.
- ADR-0226 첫 제출 래치 — 기대지 않는다(claude 는 보내기 전에 센다 · 끊기는 래치를 안 건드린다).

### 3-6. 시험

- 프론트: `agentCommands.test.ts` — `agent.interrupt` 인자 검문 · `interruptAgent` 호출 · 임대 거절 → `CONFLICT:` · `help` 없음. `interruptKey.test.ts` — 술어 표(아래 조건 하나씩 거짓 · 전부 참). `RichSlot.test.tsx` — 조건 표(수식키 · repeat · 조합 · 오버레이 표지 · `defaultPrevented` · 안 돎 · 능력 거짓 · 부재) 각각 미발화 / 전부 참이면 한 번 발화 · 입력창 글 유지 · 본문 클릭 뒤 Esc 발화(U6).
- 백엔드: `stdio.rs` — 주입 함수가 `Some` 이면 `interrupt()` 가 줄 한 벌을 큐에 넣고 · `None` 이면 `Unsupported` 에 큐 무변경 · 주입 있으면 caps 참(문 값과 무관) · 주입 없으면 오늘 단언. `claude/mod.rs` — 끊기 줄 골든(`cancel_line_bytes_golden_and_its_answer_round_trips` `:4040` 모양) · 스파이크 fixture 로 끊긴 `result` → `[Usage?, TurnEnd{Interrupted}]` · `classify_turn` → `Ended(Other)` · `interrupt:` 응답 → 사건 없음 · 기존 `result_error_handbuilt.jsonl` 은 여전히 `Error` + `MessageDone`(진짜 오류 회귀).
- 백엔드 — 턴 열림 문: 스폰 직후 함수 = `None` · 사용자 되울림 · `assistant` · 흘린 델타 · `Delivered` 줄 뒤 = `Some` · `result` 뒤 = `None` · `Dropped` · `control_response` · `system` 같은 신호 없는 줄로는 안 열린다 · 이어받기 원문으로는 안 열린다 · (④ 예비를 지을 때만) 한가할 때의 호출은 표식을 안 세워 다음 턴의 `is_error` 결과가 여전히 `Error` · 표식은 그 턴 `result` 에서 지워진다.
- ★B3 이 **의도적으로** 고쳐 쓰는 기존 시험 둘★ — 회귀로 읽지 말 것: `result_interrupted_subtype_emits_only_done_no_error`(`claude/mod.rs:3390`) · `result_interrupted_subtype_with_is_error_false_emits_only_done`(`:3402`). 둘 다 `subtype:"interrupted"` 에 `["done"]` 을 단언하는데 ② 뒤 기대값은 `TurnEnd{Interrupted}` 한 벌(Error 없음 그대로)이다. 이름도 기대에 맞게 바꾼다. §5-5 의 11 번(기존 fixture 무수정)과는 다른 축이다 — 그쪽은 F4 가 fixture 사건열을 안 바꾼다는 증거이고 이 둘은 F2 가 뜻을 바꾸는 손 시험이다.

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

- **TS 접점(고정)** — 생성물 `crates/engram-dashboard-protocol/bindings/ToolCategory.ts` = `export type ToolCategory = "Read" | "Search" | "List" | "Edit" | "Command" | "Web" | "Agent" | "Mcp" | "Other";` · `StructuredEvent.ts` 의 ToolCall 에 `category?: ToolCategory`. ★FE-2 는 B4 를 기다리지 않는다★ — 자기 파일 `src/components/slot/structuredAccumulator.ts` 에 **같은 아홉 낱말의 지역 리터럴 합** `export type ToolCategory = "Read" | … | "Other"` 을 선언해 쓰므로(생성물 import 없음) FE-2 의 모든 커밋이 B4 전에 `tsc` 초록이다. 교체는 §7 의 **I1**(주인 · 시점이 거기 있다)에서 한다.
- **호환**: 옛 데몬 → 칸이 없다 → 프론트 `Other`. 옛 셸 → 모르는 칸을 무시한다(셸은 tag1 JSON 을 해석하지 않고 나른다 — `rg StructuredEvent src-tauri/src` 0 줄). `PROTOCOL_VERSION` 은 올리지 않는다 — `TurnEnd`(`:743-763` 주석)와 같은 판단(데몬→셸 한 방향 · 옛 쪽엔 오독할 것이 없다).
- **데몬 변환** `crates/engram-dashboard-daemon/src/connection_core.rs:777-789` — `category: Some(map(category))`(일대일 match).
- **링 무게** `output_core.rs:1225-1230` `estimate_cost_bytes`: 고정 크기 enum 이라 무게 0 — 구조 분해만 `category: _` 로 넓힌다.
- **모든 생성 자리 · 망라 패턴을 한 커밋에**(컴파일러가 가리킨다): `claude/mod.rs:1108` · `codex/decoder.rs:1136` · `codex/decoder.rs:1606` 시험(`..` 없는 구조 분해 패턴 — 칸이 늘면 깨진다) · `output_core.rs` 시험(`:2152-2172` · `:2248`) · `daemon/src/agent_conn.rs:592`·`:618` · `connection_core.rs:5150-5160` · `protocol/src/messages.rs:1047` 시험. ts-rs 생성물은 `cargo test -p engram-dashboard-protocol` 이 굽는다(CI sync 게이트) — ★어느 워커도 `*/bindings/` 아래 파일을 손으로 쓰지 않는다★(§7).
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
/** vendorErrorIds = claude 파싱(`buildToolResultMap`)의 `isError` 호출 id. codex 는 항목의 `resultMark` 로 온다(§4-7 ⑧). */
export function summarizeGroup(calls: readonly ToolItem[], vendorErrorIds: ReadonlySet<string>):
  { counts: ReadonlyArray<readonly [ToolCategory, number]>; errors: number; declined: number }
```

- **묶음 규칙**:
  - 후보 = 첫 `tool` 행부터 마지막 `tool` 행까지. 그 사이에 올 수 있는 것 = 그리지 않는 행(`rowKindOf` = `skip` — usage · claude `tool_result` 운반 행 · 빈 생각 · `StructuredTextView.tsx:377-395`)과 **비지 않은 생각**(U5 흡수 — 대안이면 이 한 줄이 「끊는다」로 바뀐다).
  - 그 밖의 **그리는** 행은 전부 끊는다 — 글 · 사용자 말풍선 · 구분선(턴 끝) · 결말 · 오류 · 모르는 사건 · 그 밖의 `structured`.
  - `tool` 이 ≥2 일 때만 묶음. 하나면 오늘처럼 그 행 그대로.
  - 마지막 `tool` 뒤의 생각·skip 행은 멤버가 아니다(흐름으로 돌아간다).
  - `live` = `turnOpen` 이고 묶음 뒤에 오는 행이 전부 skip·비지 않은 생각뿐이다. ★뒤에 생각이 왔다고 접지 않는다★ — 그 뒤에 도구가 이어지면 다시 펼쳐지는 깜빡임이 된다.
  - `key` = 첫 호출의 백엔드 id(`tool:<id>`) · 없으면 `item:<itemId>` — 누산기는 같은 사건열을 같은 `itemId` 로 재구성하므로(`structuredAccumulator.ts:11-15` 멱등 불변식) replay 뒤에도 같은 키다.
- **렌더**(`StructuredTextView.tsx:514-561`): `items.map(renderItem)` 을 `groupToolRuns(items, streaming)` 의 행 목록으로 바꾼다. ★ADR-0051 불변식★ — 레일 위치(`chat/railPositions.ts`)는 **행 목록**으로 계산한다: 묶음 = `'assistant'` 한 행 · 항목 = 오늘 `rowKindOf`. 묶음은 `ChatRow rail` 하나로 그리고, 펼쳤을 때 멤버는 그 안에서 행 컴포넌트(`ToolItemRow` · `ThoughtRow`)만 그린다 — 안쪽에 `ChatRow` 레일을 두지 않으므로 레일 계산과 DOM 이 한 몸으로 남는다. `isRenderedItem`(`:402-404` — RichSlot 이 쓴다)은 항목 단위 그대로.
- ★`StructuredTextView` 는 순수 렌더로 남는다(`:5` 책임 주석 · `:527` ADR-0050/0051 순수성)★ — 이 컴포넌트에 state · effect · 스토어 구독을 더하지 않는다. `groupToolRuns` 는 렌더 중 파생이고, 펼침 상태(`useToolGroupStore`) 읽기와 토글은 새 자식 컴포넌트 **`ToolGroupRow`**(`chat/ToolGroupRow.tsx` — props = `slotId` · `row` · `results` · `runPos` · `isLast`(마지막 묶음인가 — 렌더 중 파생) · `onGroupToggle`)가 진다. `StructuredTextView` 는 `slotId`·`onGroupToggle` 을 그대로 내려보낼 뿐이다.
- **요약 줄**: 아이콘(lucide `Layers`) · 종류별 `t('chat.toolGroup<Kind>', {count})` 를 고정 순서(검색 · 읽기 · 목록 · 편집 · 명령 · 웹 · 에이전트 · MCP · 기타)로 ` · ` 로 잇고 · 오류가 있으면 끝에 `t('chat.toolGroupErrors', {count})`(붉은 톤) · 거부가 있으면 그 뒤 `t('chat.toolGroupDeclined', {count})`(흐린 톤 — ★사용자 확인★ §4-7 ⑧). 오류 수 = 항목 `resultMark === 'failed'`(codex — §4-7) 이거나 `buildToolResultMap`(`:117-125`)의 `isError`(claude) 인 호출 — 두 백엔드 다 선다(U2). 셰브론 · `aria-expanded`.
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
- ⑤(메인 수락 · t3code 선례): 사람이 **마지막이 아닌** 묶음을 펼치면 `follow.unpin()` 을 부른다. 붙은 채 위쪽 묶음을 펼치면 바닥으로 다시 내리면서 누른 머리가 화면 위로 밀려난다(Claude Code 가 2.1.83 에서 고친 「스크롤이 튄다」와 같은 부류 — 조사 §3-2). 마지막 묶음 토글은 붙음을 그대로 둔다. 빼도 다른 곳은 안 바뀐다(`StructuredTextView` 의 `onGroupToggle` prop 하나). 푼 뒤는 §2-2 규칙 4 의 걸쇠가 지킨다 — 아래로 스크롤해 문턱에 들거나 명시 `pin` 이 올 때까지 떨어져 있다.

### 4-6. 시험

- `toolRuns.test.ts`: 도구 1 개 = 묶음 없음 · 2 개 = 묶음 · `tool_result` 운반·usage·빈 생각 끼어도 이어짐 · 비지 않은 생각 흡수(U5) · 글/말풍선/구분선/결말/오류/모르는 사건이 끊음 · 마지막 도구 뒤 생각은 멤버 아님 · `live` 참(턴 중 · 뒤가 생각뿐) / 거짓(끊는 행이 옴 · 턴 끝) · 키 = 첫 id · id 없으면 `itemId` · 같은 사건열 두 번 = 같은 결과(replay).
- `summarizeGroup`: 고정 순서 · 0 인 종류 생략 · 오류 수(codex `resultMark` · claude 파싱 둘 다) · 거부 수.
- U2(codex 끝 · 실패) 시험 = §4-7 ⑨.
- `StructuredTextView.test.tsx`: 레일 위치가 행 목록과 DOM 에서 일치(ADR-0051 — 기존 레일 시험 모양) · 사용자 토글이 자동 접힘을 이김 · `data-tool-group*` 값.
- `structuredAccumulator.test.ts`: `category` 있음·없음·모르는 낱말 → 정규화.
- `toolGroupStore` · `chatCommands.test.ts`(`help` 없음).
- Rust: claude 이름 표(각 낱말 + `mcp__x__y` + 모르는 이름) · codex `commandActions` 조합 표(fixture `tool_end_m7.jsonl` 의 `unknown` → `Command` 포함) · 상한을 넘는 item 도 종류는 온전한 item 에서 · 데몬 변환 일대일 · 직렬화 왕복(`category` 없는 옛 JSON 도 읽힌다).

### 4-7. codex 도구 끝 · 실패 (U2 — 사용자 결정 「이번에 한다」)

**무엇을**: codex 가 도구 item 의 `item/completed` 에 싣는 끝 상태를 중립 사건 하나로 옮겨, 묶음 머리 「오류 N」과 펼친 행의 배지가 codex 에서도 선다. 결과 **본문**은 옮기지 않는다 — 표시는 개수와 배지뿐이다(관례 = codex TUI 의 「· N failed」 · t3code).

**사실**(확실 — 직접 확인):

- codex 는 도구 item 마다 `item/started`(`status: inProgress`) 뒤 **같은 item id** 로 `item/completed` 를 끝 상태와 함께 낸다. 끝 어휘 = `commandExecution` `completed | failed | declined`(+ `exitCode`) · `fileChange` 같은 넷(`PatchApplyStatus`) · `mcpToolCall`·`dynamicToolCall` `completed | failed` · `collabAgentToolCall` `completed | failed | interrupted` · `webSearch` 는 status 칸이 없다(t3code 생성본 `schema.gen.ts:1928-1938` · `:2004-2009` · `:2278-2283` · `:2356-2366` · item 모양 `:21559-21632`).
- 번역기는 도구 item 의 `ItemOrigin::Completed` 에서 **일부러 아무것도 안 낸다**(`backend/codex/decoder.rs:262-263` · `:623` — 한 호출을 두 번 그리지 않으려고). 그래서 끝 상태가 프론트에 닿는 길이 없다. 이력 문(`ItemOrigin::History`)은 끝난 item 을 받지만 `ToolCall` 만 낸다(같은 `:623`).
- ★**끝이 턴 끝 뒤에 올 수 있다(실측)**★: `codex/fixtures/steer_m6.jsonl` 27 줄에 시작한 명령이, 32 줄에서 그 턴이 끊긴 뒤 **다음 턴의 `turn/completed`(49 줄) 뒤** 50 줄에서 옛 turn id · `status: completed` · `exitCode: 0` 으로 닫힌다(`fixtures/README.md:19` — 끊기 17 초 뒤).

**① 선 모양 — 새 변형 `ToolResult`** [고름]

```rust
// agent types.rs — OutputEvent 에 한 변형
/// 도구 호출 하나의 끝 결과 — 앞선 `ToolCall` 을 `id` 로 가리킨다(새 행이 아니다). 결말은 중립 enum(ADR-0004).
ToolResult { id: String, outcome: ToolOutcome },
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolOutcome { Completed, Failed, Declined }

// protocol messages.rs — StructuredEvent 에 같은 모양(단위 변형이라 serde·ts-rs 는 문자열)
ToolResult { id: String, outcome: ToolOutcome },
```

- TS 접점(고정): 생성물 `crates/engram-dashboard-protocol/bindings/ToolOutcome.ts` = `"Completed" | "Failed" | "Declined"` · `StructuredEvent.ts` 에 `{ "type": "ToolResult", id: string, outcome: ToolOutcome }`.
- `id` 는 `Option` 이 아니다 — 가리킬 호출이 없으면 사건을 내지 않는다(②).
- 기각한 모양 둘:
  - **`ToolCall` 에 선택 `status` 칸 + 누산기 id 병합** — 끝에서 `ToolCall` 을 한 번 더 내야 한다. 두 턴 분류기가 `ToolCall` 을 진행으로 세므로(`claude/mod.rs:541` · `codex/mod.rs:1206`) 턴 끝 뒤에 온 끝(위 실측)이 「턴 중」을 다시 켜 30 분 막힘 경로가 된다. 옛 셸은 끝마다 **같은 도구 행을 하나 더** 그린다 — 「표시할 수 없는 신호」 줄보다 나쁘다(틀린 내용이다).
  - **`Structured{kind:…}` 탈출구** — 같은 두 분류기가 `Structured` 를 통째로 진행으로 센다(대기 입력 사건을 탈출구에 안 싣는 사유와 같다 — `types.rs:94-97`). json 을 claude `tool_result` 모양으로 지으면 옛 셸도 배지를 그리지만, 그것은 벤더 모양을 codex 번역기까지 넓혀 조사 §3-1 의 누수를 키운다.

**② 상태 매핑 — codex 번역기**: 함수 하나 `tool_outcome(item, kind) -> Option<ToolOutcome>` 를 `Completed`·`History` 두 문이 같이 쓴다(ADR-0203 「두 번째 어휘표를 만들지 않는다」).

| 벤더 `status` | → | 비고 |
|---|---|---|
| `failed` | `Failed` | 다섯 도구 변형 모두 |
| `declined` | `Declined` | `commandExecution` · `fileChange` 만 스키마에 있다 — 다른 변형에서 오면 드리프트 계수 |
| `completed` | `Completed` | ★내지 않는다★(③) |
| `inProgress` | — | 이력 페이지의 아직 도는 item |
| `interrupted`(`collabAgentToolCall`) | — | 턴 결말 행이 이미 끊김을 보인다 |
| 칸 없음(`webSearch`) | — | 일상 |
| 그 밖 문자열 · 문자열 아님 | — | `observe(tool-status:…, Drift)` |

- ★**판정은 벤더 `status` 하나다 — `exitCode` 를 읽지 않는다**★ [고름]: 0 아닌 종료를 우리가 실패로 다시 가르면 벤더와 다른 판정이 선다(t3code 도 `status` 만 본다 — `apps/server/src/provider/Layers/CodexAdapter.ts:1013-1019`). `mcpToolCall.error` · `dynamicToolCall.success` 도 같은 이유로 안 읽는다. codex 가 0 아닌 종료를 늘 `failed` 로 적는지는 **미검**이다(가능성 높음 — t3code 시험이 `exitCode: 1` 을 `failed` 와 짝짓는다 `CodexAdapter.test.ts:1545-1552`) → B5 채취가 확인한다(⑨).
- **발행**: `Started` 문 = 오늘 그대로(`ToolCall`) · `Completed` 문 = `tool_outcome` 이 `Failed`/`Declined` 면 `ToolResult` 하나, 그 밖엔 오늘처럼 사건 0 + 일상 계수 · `History` 문 = `[ToolCall, ToolResult?]` 이 순서. `id` 는 `ToolCall` 과 같은 `bounded_id`(`decoder.rs:1141`) — 걸러져 없으면 가리킬 행이 없으므로 내지 않고 `Malformed` 계수.

**③ 실패 · 거부만 낸다** [고름]: `Completed` 는 선 어휘로만 둔다(④ 의 후속이 쓸 자리). 끝마다 내면 codex 도구 호출마다 사건이 하나씩 늘고 옛 셸의 「표시할 수 없는 신호」 줄이 **모든** 호출에 붙는데, 화면이 얻는 것은 없다 — 표시는 오류·거부 수와 배지뿐이다. ★`ToolResult` 가 없다 = 성공으로 읽지 말 것★ — 없음은 「모름」이다(옛 데몬 · 링에서 밀려남 · 끊겨 끝이 안 옴).

**④ claude 는 이번에 바꾸지 않는다** [고름] — claude 번역기가 `tool_result` 블록에서 같은 `ToolResult` 를 함께 내는 안은 짓지 않는다:

- 결과 **본문**은 여전히 벤더 블록(`Structured{kind:"user"}`)에서 읽는다 — 펼친 행의 Out(`StructuredTextView.tsx:262`) · `rowKindOf` 의 skip(`:390`). 상태만 싣는 중립 사건은 그 파싱을 옛 데몬 폴백으로 내리지 못하고, **같은 오류 비트가 두 곳에 산다**(한쪽이 낡는다).
- claude 는 **모든** 호출에 결과를 낸다 — 기본 백엔드라 링 사건과 옛 셸의 표시 불가 줄이 codex 실패 수가 아니라 claude 호출 수만큼 는다.
- 누수를 **키우지는 않는다** — codex 는 벤더 모양을 안 탄다.
- 후속(추적 항목 후보 — §11 ⑦): 본문을 싣는 중립 결과(`ToolResult` 에 본문 칸 · `Completed` 도 냄) + 프론트 파싱을 옛 데몬 폴백으로 내리기. claude 이어받기 원문 경로와 한 사용자 줄의 여러 블록 처리(`structuredAccumulator.ts:175-190`)를 함께 옮겨야 해 이번 범위가 아니다.

**⑤ 판차 — 두 방향**

- **거를 값이 없다(확인)**: 셸이 데몬에 알리는 것은 인증 첫 프레임의 `protocol_version` 하나이고(`crates/engram-dashboard-net/src/auth.rs:33-38`), 데몬은 **같지 않으면 끊는다**(`net/src/ws.rs:348-363`). `Hello` 의 `capabilities` 는 데몬→셸 방향이고 지금 `None` 이다(`daemon/src/connection_core.rs:2214-2220`). 그래서 한 판 안에서 옛 셸과 새 셸을 가를 값이 없고, 연결별 변환(`connection_core.rs:765` `output_event_to_wire`)에서 걸러 낼 근거가 없다. 판을 올리면(6 → 7 · `protocol/src/lib.rs:121`) 옛 셸은 아예 못 붙어 거를 필요가 없어지지만, 개발 일상 조합(옛 데몬 + 새 셸 — discovery 가 재사용을 거부한다 `discovery/src/lib.rs:446`)을 표시 한 줄 때문에 끊는다.
- **결정: 판을 올리지 않고 거르지 않는다** — `TurnEnd`(`protocol/src/messages.rs:743-763`) · `QueuedInput`(`:782-785`)와 같은 판단(데몬→셸 한 방향 · 옛 쪽엔 오독할 것이 없다). 새 변형 doc 에 그 한 줄을 단다.
- 새 데몬 + 옛 셸: codex 실패·거부 호출마다 누산기 기본 갈래가 「표시할 수 없는 신호」 줄을 남긴다(연속이면 한 줄의 수만 는다 — `structuredAccumulator.ts:249-254`). 그 프레임은 `false` 를 돌려 대기 표시를 안 건드린다.
- 옛 데몬 + 새 셸: 사건이 안 온다 → codex 오류 표시가 없을 뿐이다.

**⑥ 턴 관측 · 오류 뒤 멈춤**: 두 분류기(`claude/mod.rs:538` · `codex/mod.rs:1203`)에 `OutputEvent::ToolResult { .. } => None`.

- ★`Progress` 금지★ — 위 실측대로 끝은 턴 끝 뒤에도 오고, 진행으로 세면 「턴 중」이 다시 켜져 30 분 fail-open 까지 우편이 막힌다(CLAUDE.md 「턴 관측 정리」 · 「대기 입력 상태」 끝 문단). `None` 이라 도착 순서에 기대지 않는다.
- ★`Failed` 금지★ — 도구 실패는 에이전트의 평범한 한 걸음이다. 세면 실패한 검색 한 번이 오류 뒤 멈춤(`last_end_failed`)을 세워 다음 깨끗한 턴까지 우편이 멈춘다(ADR-0231 N11).
- 통로의 도구 끝 계기(`is_tool_item` — `decoder.rs:282-288`)는 원 줄의 item 타입을 읽으므로 이 사건과 무관하다 — `tool_end_m7` 시험(`codex/transport.rs:9653`)이 무수정 초록이어야 한다.
- 프론트 누산기의 `ToolResult` 갈래는 `turnDone` 을 안 건드린다(⑧).

**⑦ 링 · 생성물**

- `estimate_cost_bytes`(`output_core.rs:1217`)에 `ToolResult { id, .. } => id.len()`(결말은 고정 크기). 실패·거부 호출당 사건 1 · 수십 바이트 — `REPLAY_MAX_EVENTS = 4096`(`:1291`)에 견주면 무시할 크기다.
- 망라 match 전부(컴파일러가 가리킨다 · B5 한 커밋): 두 분류기 · `output_core.rs:993-1003`(변형 이름) · `:1217` · `daemon/src/connection_core.rs:765`(새 `tool_outcome_to_wire` 일대일 — `_` 갈래 없이) · 시험 `claude/mod.rs:3012-3030` · `agent/tests/stdio_smoke.rs:77-86` · `daemon/src/bin/saturation_pilot.rs:986-995`.
- 생성물 `ToolOutcome.ts`(새) · `StructuredEvent.ts` — `cargo test -p engram-dashboard-protocol` 이 굽는다(CI sync). ★새 변형이라 프론트 `never` 망라(`structuredAccumulator.ts:243`)가 걸린다★ — 그래서 FE-2 가 가드를 먼저 두고 B5 는 그 뒤에 커밋한다(§7).

**⑧ 프론트**

- 누산기 `tool` 항목에 칸 `resultMark: ToolResultMark | null` — `ToolResultMark = 'failed' | 'declined'`(`TurnOutcomeMark` 와 같은 결: 정상 완료는 표식이 없다).
- `ToolResult` 갈래: 뒤에서부터 같은 `id` 의 `tool` 항목을 찾아 copy-on-write 로 `resultMark` 를 바꾼다(`Completed` → `null` · 모르는 낱말 → `null` + warn). 못 찾으면(링에서 밀려남) 버린다. **새 항목을 만들지 않는다** → `rowKindOf` · 레일 · `isRenderedItem` 무변경. `turnDone` 을 안 건드리고 `true` 를 돌려준다. 같은 사건열 = 같은 결과(멱등 — `ToolCall` 이 늘 먼저 온다: 라이브는 `started` → `completed`, 이력은 한 item 안에서 그 순서).
- 요약: `summarizeGroup`(§4-3)의 `errors` = `resultMark === 'failed'` 이거나 `vendorErrorIds` 에 든 호출 · `declined` = `resultMark === 'declined'`.
- 펼친 행 `ToolItemRow`(`StructuredTextView.tsx:209`)에 prop `mark` — `isErr = result?.isError === true || mark === 'failed'`(오늘 배지 · 붉은 테 그대로 · codex 는 Out 칸이 없다) · `mark === 'declined'` = 흐린 테 배지 `t('chat.toolDeclined')`.
- 상수 `DECLINED_MARK: 'own' | 'error' = 'own'`(`chat/toolRuns.ts` 머리) — `'error'` 면 거부를 오류로 함께 센다.
- ★**사용자 확인** — `declined` 를 어떻게 보이나★: **추천 = 따로**(「거부됨」 배지 · 요약 끝 「거부 N」 · 붉게 칠하지 않는다). 도구가 돌다 실패한 것이 아니라 실행되지 않은 것이고, 우리 통로는 승인 요청을 늘 거절하므로(`backend/codex/transport.rs:3052-3054` — 승인 UI 없음) 고칠 자리가 다르다. t3code 도 「Declined」 를 「Failed」 와 따로 쓴다(`apps/web/src/components/chat/MessagesTimeline.logic.ts:86-96`). 대안 = 오류로 함께 센다(「오류 N」 하나 · 붉은 배지). 바뀌는 자리 = 위 상수 하나. ★우리 거절이 벤더에서 `declined` 로 닫히는지 `failed` 로 닫히는지는 미검★ — §8-2 F3 ④ 가 기록한다.

**⑨ 시험**

- 번역기(인라인 줄 — `decoder.rs:1594-1622` 모양): 실패 명령 `item/completed`(`failed` · `exitCode: 1`) → `[ToolResult{Failed}]` · 거부 패치(`fileChange` `declined`) → `Declined` · 실패 MCP(`mcpToolCall` `failed` + `error`) → `Failed` · `completed`(`exitCode` 가 0 이 아니어도) → 사건 0 · 기존 `item_completed_emits_nothing_but_still_inspects_the_item_type`(`:2283`) 무수정 초록 · `inProgress`·`interrupted`·`webSearch` → 0 · 모르는 status → 0 + 드리프트 계수 · id 없음/상한 초과 → 0 + `Malformed` · 이력 문 = `[ToolCall, ToolResult]` 순서.
- **실측 fixture** 새 `codex/fixtures/tool_fail_u2.jsonl` — `codex_harness.js`(`.claude/handoff/attachments/20260925-midturn-phase0/`)로 0 아닌 종료 명령 한 턴을 떠서 `fixtures/README.md` 가공 규칙대로(표에 한 행). 시험 = 그 줄들 → `ToolCall` 뒤 `ToolResult{Failed}` 하나. ★그 채취에서 0 아닌 종료가 `completed` 로 오면 멈추고 메인에 올린다★ — 그러면 codex 명령 실패는 `status` 로 안 보이고, 판정을 `exitCode` 로 넓힐지는 사용자 체감이다. 거부 · MCP 실패는 실측 조건(승인 요청을 부르는 명령 · MCP 서버)이 무거워 위 인라인 줄로만 덮는다 — README 의 「손으로 지은 줄은 없다」는 그대로 참이다(인라인 줄은 fixture 가 아니다).
- 두 분류기: `ToolResult` → `None`(세 결말 각각).
- 턴 끝 뒤 끝: `steer_m6.jsonl` 49–50 줄 모양으로 `turn/completed` 뒤 `failed` 끝 → 턴 표 `in_turn` 거짓 유지 · `last_end_failed` 불변.
- wire: 직렬화 골든 `{"type":"ToolResult","id":"i-1","outcome":"Failed"}` · 왕복 · 데몬 변환 일대일.
- 프론트: 누산기 — 같은 id 에 붙음 · 항목 수 불변 · 없는 id 는 버림 · 턴 끝 뒤 도착해도 `turnDone` 불변 · replay 두 번 = 같은 스냅숏 · 모르는 outcome → 표식 없음. `summarizeGroup` — codex 실패 · claude 파싱 오류 · 거부 셈 · `DECLINED_MARK` 두 값. `StructuredTextView` — 배지 두 종 · codex 행에 Out 없음.
- 골든 영향: 대기 입력 골든(`queued_input_golden.json`) 무영향(이 사건은 명부 환원에 안 든다) · 기존 codex fixture 셋(`empty_turn_m9` · `steer_m6` · `tool_end_m7`)은 도구 끝이 전부 `completed` 라 사건열이 그대로다.

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
- **기본은 합치지 않는다**(codex 와 같다 — 조사 §4-4). 단 §9 링 압박의 수치 문턱을 넘으면 B1 안에서 합친다(§9).
- 문서 갱신: `:762-767`(「decoder 자신의 상태 = 줄 재조립뿐」)을 이 칸으로 고친다.

### 5-3. 불변식

- **턴 끝은 `result` 한 줄 그대로**(조사 §4-4 함정) — `message_stop` 을 끝으로 옮기면 도구 호출마다 턴이 끝난다.
- **스트림 부속 줄을 `Structured` 로 내지 않는다** — claude 턴 분류기는 `Structured` 를 통째로 진행으로 센다(`:538-545`). 턴 끝 뒤의 진행은 30 분 막힘 경로다(CLAUDE.md 「대기 입력 상태」 끝 문단).
- 흘린 델타는 `TextDelta` 라 진행이다 — 턴 **안**에서만 나온다: `result` 가 상태를 지우고, `message_start` 없이 온 늦은 델타는 버리므로 턴 끝 뒤에 「턴 중」을 다시 켤 길이 없다.
  - ★이 「턴 끝 뒤 진행 없음」은 벤더 전제 하나에 기댄다 — **벤더가 턴 밖에서 최상위(`parent_tool_use_id` null) `stream_event` 를 내지 않는다**★. 한가할 때 `message_start` + 글 델타가 오면 번역기는 그것을 새 메시지로 받아 `TextDelta` 를 내고, 「턴 중」(과 §3-4 턴 열림 문)이 켜져 30 분 막힘 경로가 된다. 번역기로는 막을 수 없다(턴 도중 접힌 입력의 새 `message_start` 와 모양이 같다). 그래서 B1 채취가 확인한다(§5-4 ③).
- **프론트 무변경**: 누산기 `TextDelta` 갈래는 마지막 글 항목에 이어 붙이고 중복 제거가 없다(`structuredAccumulator.ts:130-141`) — 제거는 번역기 몫이고 위 표가 진다. 흘린 첫 델타와 완결 본문이 같은 글 항목으로 모이므로 완결 버림이 빠지면 글이 두 벌이 된다(회귀 시험).

### 5-4. 이어받기 기록 (transcript)

- [고름] `None` 경로는 `stream_event` 줄을 **통째로 건너뛴다**(오늘의 `_ => {}` `:1003`). 기록에 그 줄이 있든 없든 완결 `assistant` 줄이 전문을 내므로 한 벌이다 — 지시의 「기록마다 새 상태」보다 단순하고 기록 모양에 기대지 않는다.
- **확인 단계**(B1 끝): 플래그를 켠 세션 하나를 이어받아 ① 그 `~/.claude/projects/<slug>/<sid>.jsonl` 에 `"stream_event"` 가 있는지 ② 완결 `assistant` 글 줄이 남아 있는지 기록한다. ② 가 거짓이면(기록이 부분 줄만 남긴다) 이어받은 화면의 답이 빈다 → **멈추고 메인에 올린다.** ① 이 참이면 그 줄을 fixture 로 떠서 「건너뛴다」를 못 박는다.
- **한가 구간 확인**(B1 채취 · §5-3 전제): 라이브 채취에서 각 `result` 뒤 다음 입력을 쓰기 전까지(≥ 30 초 한 구간 포함) ③ 최상위 `stream_event` 줄이 **0** 인지 기록하고, 그 구간을 fixture 에 담아 「`result` 뒤 · 다음 사용자 줄 전 = `TextDelta` 0」을 시험으로 단언한다(§5-5 의 13 번). 0 이 아니면 **멈추고 메인에 올린다.**
- **링 압박 기록**(B1 채취 · §9): 턴마다 흘린 글 델타 수 · 글 글자 수를 적는다. 긴 답(≥ 2,000 자) 한 턴을 반드시 포함한다.

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
  13. 채취한 한가 구간(`result` 뒤 · 다음 사용자 줄 전)에서 사건 중 진행 신호 0(§5-4 한가 구간 확인).
  14. (§9 문턱을 넘어 합치기를 지을 때만) 한 `decode()` 안의 이웃 `TextDelta` 는 같은 `message_id` 끼리만 하나로 합쳐지고 · 사이에 다른 사건이 끼면 끊기며 · 합친 글 = 원래 글들의 이음(청크 경계가 달라도 최종 글 항목이 같다).
- ADR 후보: 이 중복 제거 규칙은 load-bearing 이다(프론트에 제거가 없고, 빠지면 모든 claude 답이 두 벌이다) — §10.

---

## 6. 건드리는 불변식 (한눈에)

| 불변식(CLAUDE.md 「핵심 불변식」·ADR) | 기능 | 어떻게 지키나 |
|---|---|---|
| 턴 관측 · 30 분 fail-open(ADR-0127) | F2 · F3 · F4 | F2 = 끊긴 턴은 `TurnEnd` 로 닫힌다(S1 · S6) · 턴 밖 끊기는 턴 열림 문이 거절(S8) · F3 = `ToolResult` 는 두 분류기에서 `None` — 턴 끝 뒤에 와도(실측 `steer_m6` 50 줄) 「턴 중」을 안 켠다(§4-7 ⑥) · F4 = 부속 줄은 신호 없음 · 늦은 델타 버림 · 한가 구간 최상위 `stream_event` 0(§5-4 — 벤더 전제) |
| 오류 뒤 멈춤 `last_end_failed`(ADR-0231 · 0234) | F2 · F3 | F2 = 끊김 = `Ended(Other)` — 세우지도 풀지도 않는다(S2) · F3 = 도구 실패는 `Failed` 신호가 아니다(§4-7 ⑥) |
| 대기 입력 한 줄기 · 글은 늘 보인다(ADR-0231 · 0235 결정 2) | F2 | 대기분이 다음 턴에 돈다(S3) · `interrupt:` 응답은 번역 안 함 |
| 락 순서(ADR-0006) | F2 | 끊기 줄은 입력 큐만 탄다 — 새 락 간선 없음 |
| 백엔드 지식은 `backend` 한 곳(ADR-0004) | F2 · F3 · F4 | 줄 모양 · 도구 이름 표 · `commandActions` 해석 · codex 끝 상태 매핑 · 부분 메시지 규칙 전부 `backend/{claude,codex}` · 통로·프론트는 중립 값만(claude `tool_result` 파싱은 오늘 그대로 — 누수를 늘리지 않는다 · §4-7 ④) |
| replay→live · 누산기 멱등 | F3 · F4 | 묶음은 렌더 파생(누산기 상태 아님) · `ToolResult` 는 id 로 기존 항목에 붙고 새 항목을 안 만든다 · 델타도 같은 `TextDelta` 어휘 |
| wire 판 기준(`protocol/src/lib.rs:121` 의 doc) | F3 | `ToolResult` 로 `PROTOCOL_VERSION` 을 안 올린다 — 데몬→셸 한 방향 · 한 판 안에서 셸을 가를 값이 없다(§4-7 ⑤) |
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
| **B1** F4 | 인자 · `PartialMessage` · `consume_line` 인자 · fixture · 시험 1–13 · §5-4 확인(이어받기 · 한가 구간 · 링 압박 기록) · §9 문턱을 넘으면 합치기 + 시험 14 | `claude/mod.rs` · `claude/fixtures/*` | `cargo test -p engram-dashboard-agent -- --test-threads=4` 초록 · 기존 fixture 시험 무수정 · 델타 수 기록이 반환에 실림 |
| **B2** F2 스파이크 | §3-5 S1–S8 · 보고서 · fixture 채취 | 스크래치 하네스 · `docs/research/…` · `claude/fixtures/interrupt_s1.jsonl` | ★출구 조건 미달 → 멈추고 메인에 반환(B3 의 claude 부분 착수 금지)★ |
| **B3** F2 구현 | ⓐ 버스 `agent.interrupt`(스파이크와 무관 — B2 가 멈춰도 이것은 간다) ⓑ `with_interrupt`(`Option` 반환) · 턴 열림 문 · 끊기 줄 · `result` 분류 · `TurnEnd{Interrupted}` · 두 주석 갱신 · 기존 시험 둘 의도적 수정(§3-6) · (S2 가 못 가를 때만) ④ 표식 | `commands.rs` · `agent/bindings/*`(생성물) · `tests/command_declarations.rs` · `transport/stdio.rs` · `claude/mod.rs` · `types.rs`(주석) | 위 게이트 + `cargo test -p engram-dashboard-daemon -- --test-threads=4` |
| **B4** F3 백엔드 | `ToolCategory` 두 벌 · wire 칸 · 데몬 변환 · 생성 자리 전부 · claude 표 · codex 판정 · 생성물 | `types.rs` · `protocol/src/messages.rs` · `protocol/bindings/*`(생성물) · `daemon/src/connection_core.rs` · `daemon/src/agent_conn.rs` · `output_core.rs` · `claude/mod.rs` · `codex/decoder.rs` | `cargo test --workspace -- --test-threads=4` 초록 · 생성물 sync |
| **B5** U2 codex 끝(§4-7) | `ToolOutcome` 두 벌 · `ToolResult` 변형(agent · wire) · 데몬 변환 · 망라 match 전부 한 커밋(§4-7 ⑦) · codex `tool_outcome` · 두 분류기 `None` · 번역기 주석 셋 갱신(`decoder.rs:262-263` · `:523-525` · `:623` 앞) · 새 변형 doc 의 판 한 줄 · 시험 · 실측 fixture `tool_fail_u2.jsonl` · 생성물 | `types.rs` · `protocol/src/messages.rs` · `protocol/bindings/*`(생성물) · `daemon/src/connection_core.rs` · `daemon/src/bin/saturation_pilot.rs` · `output_core.rs` · `claude/mod.rs`(분류기 · 시험 이름표) · `codex/decoder.rs` · `codex/mod.rs` · `codex/fixtures/*` · `agent/tests/stdio_smoke.rs` | B4 게이트 + `npx tsc --noEmit` · `npm test`(생성물이 프론트 망라를 건다) · ★커밋 조건 = FE-2 의 가드 커밋이 트리에 있다 — 메인이 B4 반환 뒤 확인하고, 없으면 B5 지시를 FE-2 뒤로 미룬다(§11 ⑥)★ · ★채취에서 0 아닌 종료가 `completed` 면 멈추고 반환(§4-7 ⑨)★ |

### 프론트 — 워커 둘

- **FE-1.0**(FE-1 의 첫 커밋 · FE-2 는 이 커밋 뒤에 시작): 공유 파일 둘을 한 번에 친다.
  - `src/i18n/ko.ts` 키(전부): `agent.interrupt: '응답 중단'` · `slot.scrollToBottom: '맨 아래로'` · `chat.toolGroupSetExpanded: '도구 묶음 펼치기·접기'` · `chat.toolGroupSearch: '검색 {count}'` · `chat.toolGroupRead: '읽기 {count}'` · `chat.toolGroupList: '목록 {count}'` · `chat.toolGroupEdit: '편집 {count}'` · `chat.toolGroupCommand: '명령 {count}'` · `chat.toolGroupWeb: '웹 {count}'` · `chat.toolGroupAgent: '에이전트 {count}'` · `chat.toolGroupMcp: 'MCP {count}'` · `chat.toolGroupOther: '기타 {count}'` · `chat.toolGroupErrors: '오류 {count}'` · `chat.toolGroupDeclined: '거부 {count}'` · `chat.toolDeclined: '거부됨'`(뒤 둘 = §4-7 ⑧ · 사용자가 「오류로 함께」를 고르면 안 쓰인 채 남아도 무해하다)(키는 두 단 — `src/i18n/index.ts` 의 `StringKey`).
  - `src/commands/contributions.ts:8-13` 에 `import './scrollCommands'` · `import './chatCommands'` + 두 파일을 머리 주석만 있는 빈 모듈로.
- **FE-1**(RichSlot 소유 · 순차): FE-1.1 F1(§2 전부 · DomSlot) → FE-1.2 F2 프론트(`agentCommands.ts` · RichSlot Esc · 오버레이 표지 넷) → FE-1.3 F3 접착(`slotId` · `bind`/`clear` · `onGroupToggle` → `unpin`, FE-2 의 스토어 착지 뒤).
- **FE-2**(FE-1 과 병렬): `chat/toolRuns.ts` · `chat/ToolGroupRow.tsx` · `store/toolGroupStore.ts` · `StructuredTextView.tsx` · `structuredAccumulator.ts`(지역 `ToolCategory` 합 — §4-1 · U2 의 지역 `ToolOutcome` 합 + 가드 — 아래) · `commands/chatCommands.ts`. B4 · B5 를 기다리지 않고, 생성물 `ToolCategory.ts` · `ToolOutcome.ts` 를 import 하지 않는다.
  - ★**U2 가드 — 생성물에 변형이 오기 전후 둘 다 `tsc` 초록이어야 한다**★: `consume` 의 `switch` 앞에 `if (isToolResultEvent(ev)) return this.consumeToolResult(ev)` · `isToolResultEvent(ev: unknown): ev is LocalToolResultEvent` · `LocalToolResultEvent = { type: 'ToolResult'; id: string; outcome: ToolOutcome }`(지역 합 `'Completed' | 'Failed' | 'Declined'`). ★가드 가지 안에서 `ev` 의 칸을 읽지 말고 도우미에 통째로 넘긴다★ — 변형이 오기 전에는 그 가지의 `ev` 가 `never` 로 좁혀져 칸 읽기가 `tsc` 오류다. 변형이 온 뒤에는 거짓 가지에서 그 변형이 빠져 `never` 망라(`:243`)가 그대로 선다 · 생성물의 결말 낱말이 지역 합보다 많으면 빠지지 않아 거기서 빨갛다(낱말 표류 경보). 두 상태 모두 TS 5.9.3 `--strict --noUnusedLocals --noUnusedParameters` 스크래치 파일로 확인했다(3판). 도우미는 `outcome` 을 런타임에 다시 거른다(더 새 데몬의 모르는 낱말 → 표식 없음).
  - 이 가드 커밋이 B5 의 커밋 조건이다(§11 ⑥) — FE-2 는 가드를 FE-2 의 이른 커밋에 싣는다.
- **I1 바인딩 교체**(통합 한 커밋): 주인 = 메인이 『코더(단순)』에 맡긴다 · 시점 = **B4 · B5 · FE-2 가 모두 커밋된 뒤**. `structuredAccumulator.ts` 의 지역 합 둘을 지우고 `import type { ToolCategory } from '../../../crates/engram-dashboard-protocol/bindings/ToolCategory'`·`ToolOutcome` 도 같은 모양(같은 파일의 기존 생성물 import 모양 `:18-21`)으로 바꾼 뒤 그 이름을 다시 내보낸다 — 다른 파일의 import 는 안 바뀐다. U2 가드는 `case 'ToolResult':` 갈래로 바꾼다(도우미는 그대로). 게이트 = `npx tsc --noEmit` · `npm test`(프론트가 쓰는 낱말이 생성물에 없으면 여기서 빨갛다 · 생성물이 낱말을 더 가졌는지는 눈으로 대조한다).
- 겹침 점검: FE-1 ∩ FE-2 = ∅(FE-1.0 뒤) · RichSlot = FE-1 만 · StructuredTextView = FE-2 만 · B5 = 백엔드 파일 + 생성물뿐(프론트 소스 0) · I1 = FE-2 · B5 끝난 뒤라 겹침 없음.
- **커밋 규율(워커 공통)**: ★어느 워커도 `*/bindings/` 아래 파일(`crates/engram-dashboard-protocol/bindings/` · `crates/engram-dashboard-agent/bindings/` · `src-tauri/bindings/`)을 손으로 쓰거나 고치지 않는다★ — 시험이 굽고 CI sync 게이트가 주인이다. 한 트리를 나눠 쓰는 워커는 커밋할 때 **자기 경로만** 스테이징한다(`git add <자기 파일>` — `git add -A`·`git add .` 금지). 남의 미커밋 변경(생성물 포함)을 자기 커밋에 싣지 않는다.

### 메인이 먼저 못 박는 접점

| 접점 | 모양 | 쓰는 쪽 → 읽는 쪽 |
|---|---|---|
| TS `ToolCategory` | `"Read"\|"Search"\|"List"\|"Edit"\|"Command"\|"Web"\|"Agent"\|"Mcp"\|"Other"` · ToolCall `category?` | B4(생성물) · FE-2(같은 모양의 지역 합) → I1 이 하나로 |
| TS `ToolOutcome` · `StructuredEvent` `ToolResult` | `"Completed"\|"Failed"\|"Declined"` · `{ type:"ToolResult"; id: string; outcome: ToolOutcome }` | B5(생성물) · FE-2(같은 모양의 지역 합 + 가드 — 위) → I1 이 하나로 |
| `StructuredItem` `tool` | `{ kind:'tool'; name; argsJson; id; category: ToolCategory; resultMark: 'failed'\|'declined'\|null; itemId }` | FE-2 내부 |
| `StructuredTextView` props | `{ items; streaming?; slotId?: string; onGroupToggle?: (isLast: boolean) => void }` | FE-2 → FE-1.3 |
| `useToolGroupStore` | §4-4 | FE-2 → FE-1.3 |
| `FollowHandle` | `{ pinned: boolean; pin(): void; unpin(): void }` | FE-1.1 → FE-1.3 |
| 버스 `agent.interrupt` | `{target}` → `{outcome:"requested"}` · NOT_FOUND · CONFLICT | B3 → (LLM · CLI) |

---

## 8. 검증 계획

### 8-1. 기계 게이트 (`/qa`)

- 백엔드 = 각 단계 표의 명령 · 마지막 `cargo test --workspace -- --test-threads=4` · `cargo fmt --check` · 생성물 sync(`protocol/bindings` · `agent/bindings`). B5 는 생성물이 프론트 망라를 걸어 `npx tsc --noEmit` · `npm test` 도 돈다.
- 프론트 = `npm test` · `npx tsc --noEmit`.
- 격리 게이트 = CLAUDE.md 「빌드·검증 명령」 그대로(`use tauri` 0 줄 등) — 이 라운드는 crate 경계를 옮기지 않는다.

### 8-2. GUI 실측 (`scripts/cdp.mjs` · `/qa full` 격리 인스턴스 · ★셸에서 직접 띄우지 않는다★)

| 기능 | 무엇을 재나 |
|---|---|
| F1 | ① 떨어진 채 탭을 돌렸다 돌아오기 — `scrollTop` 이 `display:none` 을 지나 남나(조사 §7 미검 — 코어의 되살리기가 필요했는지 기록) · 붙은 채 돌아오면 바닥 ② 창 새로고침·재구독 replay 폭주 뒤 바닥 착지 ③ 스트리밍 중 위로 휠 → 안 끌려 내려옴 · `data-scroll-follow="free"` ④ 펼친 생각 블록 안 휠 → 바깥 안 풀림 ⑤ 대기 목록이 서며 뷰포트가 줄 때 바닥 유지 ⑥ 버튼 150 ms 뒤 등장 · 누르면 바닥 ⑦ `__engramCmd('slot.scrollToBottom', {slotId})` |
| F2 | ① 실 codex 도구 중 Esc → 중단 행 · 대기 글이 다음 턴 ② (B3 뒤) 실 claude 글 흐르는 중 · 도구 중 Esc → 중단 행 · 오류 행 없음 · 대기 글이 돈다 ③ 한글 조합 중 Esc = 조합만 취소 ④ 안 돌 때 Esc = 무동작 · 턴이 열리기 전(보낸 직후) Esc = 무동작(CONFLICT warn 만) · (B3 뒤) `engram agent interrupt` 를 한가할 때 = CONFLICT ⑤ 우클릭 메뉴 열린 채 Esc = **아무 일도 없다**(끊기 없음 · 메뉴는 그대로 — `SlotContextMenu` 는 Esc 처리기가 없고 바깥 `mousedown` 으로만 닫힌다 `SlotContextMenu.tsx:110-114` · Esc 닫기는 이번에 더하지 않는다) ⑥ 입력창 글 유지 ⑦ 본문 클릭 뒤 Esc(U6) ⑧ `engram agent interrupt <이름>` |
| F3 | ① 실 claude 읽기·검색 ≥2 턴 — 도는 동안 펼침 · 글이 오면 접힘 · 요약 문구 ② 실 claude 실패 도구 → 「오류 1」 ③ (B5 뒤) 실 codex 0 아닌 종료 명령(예: `exit 3`)이 든 묶음 → 머리 「오류 1」 · 펼치면 그 행에 배지 · Out 칸 없음 ④ (B5 뒤 · 재현되면) 샌드박스 밖 쓰기처럼 codex 가 승인을 묻는 명령 → 우리 거절 뒤 그 호출이 `declined`/`failed` 중 무엇으로 닫혔나 기록 · 배지 모양(§4-7 ⑧) ⑤ codex 묶음 개수 ⑥ 토글 · 실패 배지가 재구독 replay 뒤에도 남음 ⑦ 같은 이력 replay → 같은 묶음 |
| F4 | ① 실 claude JSON 에서 글이 점점 늘어난다 ② 블록 끝에 글이 두 벌 안 됨 ③ 도구·생각 한 번씩 ④ 그 세션 이어받기 → 글 한 벌 |

---

## 9. 위험 · 알려진 한계

- **F1 내용 노드 = Radix 첫 자식**: Radix 가 래퍼 구조를 바꾸면 성장을 못 본다(뷰포트 RO 는 계속 돈다). 증상 = 스트리밍이 바닥에 안 붙음 → 훅 시험이 첫 자식 부재를 경고로 남긴다. 대안(내용 ref 를 내려보내기)은 두 슬롯 수정이 필요하다.
- **F4 링 압박**: 토큰 단위 `TextDelta` 로 링(`REPLAY_MAX_EVENTS = 4096` — `output_core.rs:1291`)이 더 빨리 찬다 → 긴 대화의 앞 이력이 replay 에서 더 일찍 밀린다. ★수치 출구 문턱★: B1 채취(§5-4 링 압박 기록)의 긴 답(≥ 2,000 자) 한 턴의 흘린 델타 수가 **`REPLAY_MAX_EVENTS` 의 10 %(≈ 410)를 넘으면** B1 안에서 합치기를 짓는다 — 한 `decode()` 호출이 돌려주는 사건 목록 안에서 이웃한 같은 `message_id` 의 `TextDelta` 를 하나로 잇는다(backend 만 · seq 는 emit 때 매겨지므로 구멍이 안 생긴다 · 시험 14). 넘지 않으면 합치기는 후속으로 남긴다(조사 §4-4). 판정 수치는 B1 반환에 싣는다.
- **F4 렌더 비용**: 델타마다 `setItems`(`RichSlot.tsx:215`) + F1 RO. codex 가 오늘 같은 경로를 이미 탄다.
- **F2 턴 열림 전 끊기**(③ · S4): 턴이 열리기 전(보낸 직후 · S4 결과에 따라 `init` 전까지)의 Esc 는 CONFLICT 로 거절되는 무동작이다 — 다시 누르면 된다.
- **F2 잔여 경합**(§3-4 · S7): 턴 끝 직후의 늦은 Esc · 버스 호출이 CLI 가 스스로 연 다음 턴을 끊을 수 있다. 벤더 줄에 턴 id 가 없어 닫을 수 없다.
- **F2 능력 표시**: claude JSON 의 `control.interrupt` 가 참이 되면 트리 `canInterrupt`(`mergeTreeNodes.ts:94`)도 참 — 오늘 소비자가 없다.
- **F3 펼침 상태는 인메모리**(새로고침에 초기화).
- **F3 판차**(U2 · §4-7 ⑤): 새 데몬 + 옛 셸 = codex 실패·거부 호출마다 「표시할 수 없는 신호」 줄 · 옛 데몬 + 새 셸 = codex 오류 표시 없음. 한 판 안에서 셸을 가를 값이 없어 거르지 못한다 — `TurnEnd` 와 같은 수용이다.
- **F3 codex 판정 = 벤더 `status`**: 0 아닌 종료를 벤더가 무엇으로 적는지 미검(B5 채취가 확인 — `completed` 로 오면 멈춘다) · 「못 찾음」을 종료 1 로 알리는 검색 명령이 벤더가 `failed` 로 적으면 「오류」로 보인다.
- **F3 늦은 끝**: 끊긴 호출의 끝이 다음 턴 뒤에 오면(실측 `steer_m6`) 지난 턴의 행에 배지가 붙는다 — 정직한 표시다. 그 프레임이 「보낸 직후 · 첫 응답 전」 창에 들면 알아들은 프레임이 대기를 풀어(`RichSlot.tsx:222`) 대기 표시가 잠깐 꺼질 수 있다 — 같은 부류가 `Usage` 에도 있다.
- **F3 claude 결과 파싱은 그대로**(§4-7 ④): 프론트가 벤더 `tool_result` 모양을 읽는 자리(조사 §3-1)는 이번에 걷지 않는다 — 후속(§11 ⑦).
- **F3 벤더 도구 이름 표류**: claude 가 도구 이름을 바꾸면 `Other` 로 떨어진다(묶음은 그대로 선다 — 요약 문구만 「기타」).

---

## 10. ADR 후보 (번호 = `/adr` 가 채번 · 가안은 0237 부터 — `docs/decisions` 마지막 = 0236 확인)

| 가안 | 결정 | 거부한 대안(사용자·메인이 확정) |
|---|---|---|
| **0237** | 채팅 칸 Esc = 도는 턴 끊기 · 명령 `agent.interrupt`(창 + 버스) · 범위 = 칸 안 어디든(U6) · 조건은 지역 술어 하나(§3-2) — ★ADR-0235 결정 10(「UI 정지 버튼·단축키는 지금 넣지 않는다」)을 번복★(그 ADR 에 개정 도장) | 전역 단축키 표(가드를 연다) · **단축키 시스템을 지금 만든다**(보류 — `docs/tracking.md` T-36 · Esc 는 지금 명령 `agent.interrupt` 를 부르는 지역 술어이고 나중에 키바인딩 표 한 줄로 옮긴다) · 입력창만(U6 — 사용자가 칸 안 어디든을 골랐다) · 두 번 Esc |
| **0238** | claude JSON 끊기 = 통로 주입 제어 줄(턴 열림 문 — 턴 밖이면 `Unsupported`) · 끊긴 `result` → `TurnEnd{Interrupted}`(오류 아님 · 멈춤 불변) · 대기분은 다음 턴(`cancel_queued` 안 씀) · 잔여 경합은 벤더 한계로 문서화 | 세션 입력 자물쇠 경로(①) · `MessageDone` 유지(②) · 턴 열림 전 끊기를 미루는 큐(③) · 대기분 취소 |
| **0239** | `ToolCall.category` 중립 선 칸 — 번역기가 정하고 프론트는 모르면 「기타」 | 프론트 이름 표(벤더 지식 누수) · 백엔드가 묶음을 만든다(표시 관심사를 선에) |
| **0240** | claude 부분 메시지 중복 제거 = 열린 블록이 흘렸으면 완결 글을 버린다 · 턴 끝은 `result` 그대로 | 교체(새 선 변형 + replay 교체) · 꼬리 비교(paseo — 중복 모서리) · 무조건 합치기(§9 문턱을 넘을 때만 짓는다) |
| **0241** | 도구 끝 = 새 선 변형 `ToolResult{id, outcome}` — codex 가 실패·거부만 낸다 · 판정은 벤더 `status` · 두 턴 분류기 `None` · `PROTOCOL_VERSION` 안 올림(판차 수용) · claude 는 이번에 안 바꾼다(U2 · §4-7) | `ToolCall` 선택 status + id 병합(끝이 진행 신호가 됨 · 옛 셸 중복 행) · `Structured` 탈출구(claude 모양 흉내 포함) · 판을 올려 거르기 · 모든 끝을 냄 · claude 도 상태만 냄 · `exitCode` 로 판정 |
| (선택) **0242** | 스크롤 따라가기 = 직접 쓴 순수 코어 · 성장 뒤 다시 재지 않는다 | `use-stick-to-bottom`(Radix 휠 불일치 의심 · 전역 청취자) · CSS 만 · 가상화 |

- **0241 을 0239 에 넣지 않고 새 번호로 두는 이유**: 거부한 대안이 서로 다른 축이다(0239 = 종류를 누가 판별하나 · 0241 = 끝 결과의 선 모양과 판차). 한 ADR 에 섞으면 한쪽을 번복할 때 다른 쪽까지 폐기 도장이 번지고, 다음 세션이 어느 대안이 어느 결정의 것인지 못 가른다.
- 함께 고칠 문서: `types.rs:70` · `claude/mod.rs:551` · `:762-767` 주석 · `codex/decoder.rs:262-263` · `:523-525` · `:623` 앞 주석(끝에서 결과를 낸다 — B5) · CLAUDE.md 「핵심 불변식」(claude 끝 어휘에 끊김 `TurnEnd` 한 갈래 · 「대기 입력 상태」 끝 문단의 「턴 끝 뒤에 오는 사건」 예에 `ToolResult` — 선택) — 착지 라운드에서 `/review doc`.

---

## 11. ★메인 확인 필요★ 모음 · 판정(2판 ①–⑤ · 3판 ⑥⑦)

| # | 무엇 | 왜 올리나 | 메인 판정 |
|---|---|---|---|
| ① | claude 끊기 줄을 `MidTurnPolicy`·세션 입력 자물쇠가 아니라 **통로 주입**(`StdioTransport::with_interrupt`)으로 보낸다(§3-4) | 지시는 `cancel_async_message` 경로를 따르라였다. 그 경로로 가면 능력(`control.interrupt`)이 통로 caps 에서 오는 구조와 어긋나고, 입력 id 가 없어 자물쇠가 지킬 순서도 없다 | **수락** — 리뷰어 확인: ADR-0004 격리 유지 · 새 락 간선 없음 · 입력 임대 검문은 이미 덮인다 |
| ② | 끊긴 claude 턴을 `MessageDone` 대신 `TurnEnd{Interrupted}` 로 닫는다 · 오늘의 `subtype:"interrupted"` 도 같이(§3-4) | `types.rs:70` · `claude/mod.rs:551` 주석과 부딪히고, 오늘 구분선만 그리던 경우에 중단 행이 붙는다. 이득 = codex 와 같은 중단 표시 · 오류 뒤 멈춤 불변 | **수락** — 리뷰어 확인: 턴 관측 `Ended(Other)` · `last_end_failed` 불변 · 초인종 울림 · 누산기가 이미 결말 행을 그림 · 대기 입력 골든 무영향 |
| ③ | S4 에서 init 전 끊기가 무동작이면: 받아들인다(문서화) vs init 을 볼 때까지 끊기를 미룬다(§3-5) | 미루기는 번역기↔통로 사이에 새 공유 상태가 필요하다 | **해소** — 턴 열림 문(§3-4): 턴이 열리기 전의 Esc 는 `Unsupported`(무동작 · 정직한 답) · 사용자가 다시 누른다 · 미루는 큐 없음. S4 가 되울림 뒤 · `init` 전 틈을 보이면 문이 여는 줄만 늦춘다 |
| ④ | S2 에서 `subtype`·`terminal_reason` 이 끊김을 못 가르면 「우리가 보냈다」 표식으로 가른다(§3-4) | 끊기와 겹친 진짜 오류를 끊김으로 접어 가릴 수 있다 | **예비로만 유지** — 스파이크가 두 칸 모두 끊김을 못 가를 때만 짓는다 · 문이 열려 있을 때만 세우고 그 턴의 `result` 에서 지운다 → 한가할 때의 호출이 다음 턴의 진짜 오류를 가리지 못한다 |
| ⑤ | 사람이 마지막이 아닌 도구 묶음을 펼치면 따라가기를 푼다(§4-5) | 지시(「RO 가 처리한다」)에 더한 것 — 빼면 붙은 채 펼친 묶음 머리가 화면 위로 밀려난다 | **수락** |
| ⑥ | B5 는 FE-2 의 U2 가드 커밋 뒤에만 커밋한다(§7) | 새 변형의 생성물이 프론트 `never` 망라(`structuredAccumulator.ts:243`)를 걸어, 가드 없이 들어가면 그 커밋부터 트리의 `tsc` 가 빨갛다 — 두 워커 사이에 순서가 하나 생긴다. 대안 = B5 가 누산기 한 갈래를 함께 친다(FE-2 파일과 겹친다 — 기각) | 대기 |
| ⑦ | claude 결과 중립화(본문 포함 `ToolResult` · 프론트 파싱을 옛 데몬 폴백으로)를 후속 추적 항목으로 적는다(§4-7 ④) | 이 문서는 `docs/tracking.md` 를 안 건드린다 — 적립은 메인 몫 | 대기 |


## 12. 리뷰 기록 · 구현 때 반영할 것

- **`/review trd full` 1 라운드(2026-09-27)** — 두 리뷰어(cross-family Designer · doc-aware Architect-breaker) 모두 FIX · 불일치 없음 · 합 11 건(겹친 것 1) → 2판에 전부 반영. **light 재검(doc-aware) = PASS**, 남은 low 4 건은 아래 — 구현 워커 지시서에 넣는다.
  1. §3-4 S4 보정(「init 뒤 첫 진행 줄에서만 연다」)은 비트가 하나 더 든다(「마지막 `result` 뒤 init 을 봤나」) · `system/init` 이 턴마다 오는지 S4 가 기록한다.
  2. §2-2 규칙 2 의 「떨어진 채 녹는데 `savedTop` 이 비었다」 갈래는 규칙 1 이 얼 때마다 채우므로 손으로 만든 상태로만 시험된다 — 시험에 그렇게 적는다 · 규칙 8 에 `lastTop`·`unseenGrowth` 초기값을 적는다.
  3. §9 · 시험 14 의 이웃 델타 합치기는 펌프 한 번 읽기에 여러 줄이 올 때만 줄어든다 — 문턱을 넘으면 B1 은 **실 펌프 청크 기준** 합친 뒤 사건 수를 보고한다(fixture 청크 기준 아님).
  4. §8-2 F2 ④ 문구: 턴이 열리기 전 Esc 는 WS `Interrupt` 경로라 날 「unsupported」 오류 문자열이 온다(`CONFLICT:` 접두는 임대 거절만) — §3-2 문구가 맞다. 유휴 버스 명령의 CONFLICT 줄은 그대로 맞다.
