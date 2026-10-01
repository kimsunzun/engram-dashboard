# ADR-0241: 도구 끝 결과는 새 선 변형 ToolResult 로 codex 실패와 거부만 싣는다 — 판은 올리지 않고 우리 거절은 통로가 귀속해 거부로 따로 보인다

- 상태: 확정 (2026-09-27, 근거: 사용자 결정 U2 · U2-a · U7 · 알려진 한계 수락 · 채취 둘째 ⓐ (TRD `docs/process/S21-chat-ux/trd.md` §1 · §11) + 메인 판정 ⑧ ⑪ (TRD §11) + 업스트림 codex 소스 판독(main `41f9084` · 태그 `rust-v0.156.1` · 실행 미검) + TRD 5판 리뷰 FIX 반영(TRD §12) · 구현 전 · B5 채취 전 · 멈추면 해당 결정은 새 ADR 로 개정) · 부분 폐기 by ADR-0243 (결정 12 claude 끊긴 도구 행 아래 맥락은 끊김 표시 행이 준다)
- 관련: TRD `docs/process/S21-chat-ux/trd.md`(§1 U2 · U2-a · U7 · §4-3 · §4-7 · §6 · §7 B5 · FE-2 · I1 · §8-2 F3 · §9 F3 · §10 0241 · §11 ⑥ ⑧ ⑨ ⑩ ⑪ · §12) · 조사 `docs/research/chat-ux-four-features-2026-09-27.md`(§3 · §8) · ADR-0239(도구 종류 `category` — 같은 묶음의 요약 · 축이 달라 뗐다) · ADR-0237(중단 행 강조 U8 — 늦은 실패 옆의 맥락) · ADR-0238(claude 끊김 `TurnEnd{Interrupted}`) · ADR-0203(codex 이력 item — 어휘표 한 벌) · ADR-0004(backend 지식 격리) · ADR-0051(행 종류 ↔ 레일) · ADR-0127(턴 관측 · 30 분 fail-open) · ADR-0231(N11 오류 뒤 멈춤 · codex 통로 락 순서) · ADR-0234(방출 줄 팬아웃 예외) · ADR-0173(주황 경고 토큰 `--status-blocked`) · `docs/tracking.md` T-43(claude 결과 중립화) · T-44(채팅 승인) · T-12(Wait 재설계) · step-log S21 · Amended by ADR-0243 (결정 12 claude 끊긴 도구 행 아래 맥락은 끊김 표시 행이 준다)

## 맥락
도구 호출 묶기(ADR-0239)의 머리에 「오류 N」을 세우려면 호출마다 끝 결과가 프론트에 닿아야 한다. claude 는 결과 본문을 벤더 블록(`tool_result`)으로 이미 싣고 프론트가 그 모양을 읽는다(`buildToolResultMap` 의 `isError`). codex 는 도구 item 마다 `item/started` 뒤 같은 item id 로 `item/completed` 를 끝 상태와 함께 내지만, 우리 번역기는 도구 item 의 `ItemOrigin::Completed` 에서 **일부러 아무것도 안 낸다**(`backend/codex/decoder.rs:262-263` · `:623` — 한 호출을 두 번 그리지 않으려고). 이력 문(`ItemOrigin::History`)도 `ToolCall` 만 낸다. 그래서 codex 의 끝 상태가 프론트에 닿는 길이 없다.

사용자는 추천(「안 한다 — codex 묶음은 개수만」)과 달리 **이번에 한다**고 정했다 [사용자 U2 2026-09-27]. 사용자 근거: 관례(codex TUI 의 「· N failed」 · t3code)는 요약의 개수 + 펼치면 어느 호출이 실패했나이고, codex 번역기는 끝 상태를 싣는 `item/completed` 만 버리고 있다.

거기에 **우리 거절**이 겹친다. 대시보드에는 승인 UI 가 없어 codex 통로가 서버 요청을 전부 `METHOD_NOT_FOUND` 로 거절한다(`backend/codex/transport.rs:3054-3087` · 들어오는 요청마다 `:3629`). codex 는 그 거절을 명령은 `failed` 로, 파일 변경은 `declined` 로 닫는다(아래 「근거」 — 소스 판독) — 벤더 status 만으로는 명령은 진짜 실패와, 파일 변경은 codex 스스로의 거부와 모양이 같다. 지금 화면에는 아무것도 안 뜬다(가능성 높음 — 실측 아님 · 조사 §8-1).

또 끝은 **턴 끝 뒤에 올 수 있다**(실측 — `codex/fixtures/steer_m6.jsonl` 27 줄에 시작한 명령이, 32 줄에서 그 턴이 끊긴 뒤 다음 턴의 `turn/completed`(49 줄) 뒤 50 줄에서 옛 turn id · `status: completed` · `exitCode: 0` 으로 닫힌다 · 끊기 17 초 뒤).

## 결정
표기 — **[사용자]** = 사용자가 답한 결정 · **[메인]** = 오케스트레이터 판정 · **[고름]** = TRD 가 고른 내부 구현. 범위 = codex 가 싣는 끝 결과와 그 표시. claude 번역기는 바꾸지 않는다(결정 6).

1. **선 모양 = 새 변형 `ToolResult { id: String, outcome: ToolOutcome }`** [고름] — agent `OutputEvent` 와 wire `StructuredEvent` 에 같은 모양으로. 앞선 `ToolCall` 을 `id` 로 가리킨다(새 행이 아니다). `id` 는 `Option` 이 아니다 — 가리킬 호출이 없으면 사건을 내지 않는다.
   ```rust
   pub enum ToolOutcome {
       Completed,
       Failed,
       Declined, // 에이전트(벤더)가 스스로 실행을 거부했다
       Refused,  // 우리(호스트)가 승인 요청을 거절해 실행되지 않았다 — 번역기는 내지 않고 통로가 바꿔 쓴다(결정 8)
   }
   ```
   TS 접점 = 생성물 `ToolOutcome.ts`(`"Completed" | "Failed" | "Declined" | "Refused"`) · `StructuredEvent.ts` 에 `{ "type": "ToolResult", id: string, outcome: ToolOutcome }`.
2. **codex 매핑 = 함수 하나 `tool_outcome(item, kind) -> Option<ToolOutcome>`** — `Completed` · `History` 두 문이 같이 쓴다(ADR-0203 「두 번째 어휘표를 만들지 않는다」).

   | 벤더 `status` | → | 비고 |
   |---|---|---|
   | `failed` | `Failed` | 다섯 도구 변형 모두 · 우리가 거절한 id 면 통로가 `Refused` 로 바꾼다(결정 8) |
   | `declined` | `Declined` | `commandExecution` · `fileChange` 만 스키마에 있다 — 다른 변형에서 오면 드리프트 계수 · 우리가 거절한 id 면 통로가 `Refused` 로 바꾼다 |
   | `completed` | `Completed` | ★내지 않는다★(결정 3) |
   | `inProgress` | — | 이력 페이지의 아직 도는 item |
   | `interrupted`(`collabAgentToolCall`) | — | 턴 결말 행이 이미 끊김을 보인다 |
   | 칸 없음(`webSearch`) | — | 일상 |
   | 그 밖 문자열 · 문자열 아님 | — | `observe(tool-status:…, Drift)` |

   - ★**판정은 벤더 `status` 하나다 — `exitCode` 를 읽지 않는다**★ [고름]. `mcpToolCall.error` · `dynamicToolCall.success` 도 같은 이유로 안 읽는다.
   - **발행**: `Started` 문 = 오늘 그대로(`ToolCall`) · `Completed` 문 = `tool_outcome` 이 `Failed`/`Declined` 면 `ToolResult` 하나(번역기는 `Refused` 를 내지 않는다), 그 밖엔 오늘처럼 사건 0 + 일상 계수 · `History` 문 = `[ToolCall, ToolResult?]` 이 순서. `id` 는 `ToolCall` 과 같은 `bounded_id` — 걸러져 없으면 가리킬 행이 없으므로 내지 않고 `Malformed` 계수.
3. **실패 · 거부만 낸다** [고름] — `Completed` 는 선 어휘로만 둔다(결정 6 의 후속이 쓸 자리). ★`ToolResult` 가 없다 = 성공으로 읽지 말 것★ — 없음은 「모름」이다(옛 데몬 · 링에서 밀려남 · 끊겨 끝이 안 옴).
4. **두 턴 분류기(`claude/mod.rs:538` · `codex/mod.rs:1203`)에서 `OutputEvent::ToolResult { .. } => None`.**
   - ★`Progress` 금지★ — 끝은 턴 끝 뒤에도 오고(맥락의 실측), 진행으로 세면 「턴 중」이 다시 켜져 30 분 fail-open 까지 우편이 막힌다(CLAUDE.md 「턴 관측 정리」 · 「대기 입력 상태」 끝 문단). `None` 이라 도착 순서에 기대지 않는다.
   - ★`Failed` 금지★ — 도구 실패는 에이전트의 평범한 한 걸음이다. 세면 실패한 검색 한 번이 오류 뒤 멈춤(`last_end_failed`)을 세워 다음 깨끗한 턴까지 우편이 멈춘다(ADR-0231 N11).
   - 통로가 `Refused` 로 바꿔 쓴 사건도 같은 `None` 이다 — 바꿔 쓰기는 결말 칸만 바꾸고, 사건이 보통 문 · 관측 없는 문 중 어디로 나가는지(`transport.rs:3659-3670` 의 `ours` 판정)도 그대로다.
5. **`PROTOCOL_VERSION` 을 올리지 않고 거르지도 않는다** — `TurnEnd`(`protocol/src/messages.rs:743-763`) · `QueuedInput`(`:782-785`)와 같은 판단(데몬→셸 한 방향 · 옛 쪽엔 오독할 것이 없다). 새 변형 doc 에 그 한 줄을 단다. 대가 = 새 데몬 + 옛 셸에서 codex 실패·거부 호출마다 누산기 기본 갈래가 「표시할 수 없는 신호」 줄을 남긴다(연속이면 한 줄의 수만 는다 · 그 프레임은 대기 표시를 안 건드린다) · 옛 데몬 + 새 셸은 codex 오류 표시가 없을 뿐이다. 이 대가는 U2 의 트레이드오프로 사용자에게 올라갔다(TRD §1 U2 — 실패·거부만 내서 줄인다).
6. **claude 는 이번에 바꾸지 않는다** [고름] — claude 번역기가 `tool_result` 블록에서 같은 `ToolResult` 를 함께 내는 안은 짓지 않는다(거부한 대안). claude 의 오류 수는 오늘처럼 프론트 파싱(`buildToolResultMap` 의 `isError`)에서 온다. 본문을 싣는 중립 결과는 후속이다(`docs/tracking.md` T-43).
7. **거부 = 따로 표기 · 결말 둘** [사용자 U2-a · 결말 분리 = 4판 리뷰 FIX · 메인 채택] — 도구가 돌다 실패한 것이 아니라 실행되지 않은 것이다.
   - **`Refused`** = 우리 거절 · 사유 줄 「대시보드가 승인 요청을 처리하지 않아 실행되지 않음」(U2-a 의 문구 — 결말을 나눈 5판에 `Refused` 쪽으로 · 메인).
   - **`Declined`** = 기억에 없는 거부(codex 스스로의 거부 — guardian 심사 · 네트워크 정책 등 · 벤더 준비 실패 · 아래 한계의 복원 · 상한 밀림) · 사유 줄 「실행되지 않음」 [사용자 2026-09-27 · 5판 light 재검 — 메인의 5판 문구 「codex 가 실행을 거부함」을 바꿨다].
   - **배지 · 색 · 셈은 둘이 같다**: 붉은 `Error` 배지 자리에 `t('chat.toolDeclined')`(「거부됨」) · 글자 · 테두리 = `var(--status-blocked)`(주황 경고 토큰 — dark `#d29922` · light `#9a6700` · e-ink 는 본문색 · ADR-0173) · 머리(`RowHeader`)는 기본 톤 — 붉게 칠하지 않는다 · 묶음 요약 `chat.toolGroupDeclined`(「거부 {count}」)가 `refused` · `declined` 를 함께 세고 「오류 N」 뒤에 온다 · **오류 수에 안 든다**. [고름] 새 토큰을 짓지 않는다(e-ink 무력화가 이미 들어 있다).
   - **사유 줄**: 같은 줄 상자 안, 머리 버튼 바로 아래 한 줄(`text-xs` · `text-muted`) · **접힌 채로도 보인다** · DOM 표지 `data-tool-declined-reason` = 사유 키(`"chat.toolRefusedReason"` | `"chat.toolDeclinedReason"`). ★출처는 이 키로 가른다 — 글로 가르지 말 것★: 「실행되지 않음」은 우리 사유의 꼬리와 같아 글 포함 검사로는 두 출처가 안 갈린다. 줄 뿌리 `data-tool-mark="failed" | "declined"`(`refused` 도 `"declined"`). [고름] 문구는 출처마다 고정이다 — 벤더 item 에 사유 칸이 없다.
   - 상수 `DECLINED_MARK: 'own' | 'error' = 'own'`(`chat/toolRuns.ts` 머리) — `'error'` 는 대안 A 의 스위치로 남긴다(거부 둘 다 오류로 세고 붉은 배지 · 사유 줄 없음).
8. **우리 거절 귀속 — codex 통로가 거절한 승인 요청의 item id 를 기억해 그 끝을 `Refused` 로 바꿔 낸다** [고름 — 자리 = 리더 전용 칸 · 메인 ⑧ 수락 · 바꿔 쓰기 규칙 = 메인 5판].
   - **기억하는 요청 = 둘뿐** — `item/commandExecution/requestApproval` · `item/fileChange/requestApproval`. 조건: `params.itemId` 가 문자열이고 `MAX_ID_BYTES`(128) 안 · 명령 승인은 `approvalId` · `networkApprovalContext` 가 없거나 `null` 일 때만(하위 명령 승인은 부모 item 의 실행 거절이 아니고 · 네트워크 승인은 그 처리기가 끝을 안 낸다) · 거절 줄이 실제로 제어 큐에 들어갔다(`refuse` 가 `closed` 로 일찍 돌아가거나 넘침이면 적지 않는다). 그 밖의 요청 · `itemId` 없음 → 귀속 없음(오늘처럼 거절만).
   - **자리 = 리더(`Reader` — `transport.rs:3041-3049`) 전용 칸 하나** `refused: RefusedItems` — `first_turn` 과 같은 결이고 상태 락(`State`) 밖이다. 쓰는 쪽(`refuse`)과 읽는 쪽(같은 `handle_line` 의 `item/completed` 알림)이 둘 다 리더 스레드라 기억이 늘 끝보다 먼저 선다. `refuse` 는 `&self` → `&mut self` + 인자 `params: Option<&Value>`. 메서드 이름 상수 둘은 `protocol.rs` 에.
   - **모양 · 상한** — `VecDeque<String>` · 상한 `REFUSED_ITEM_SLOTS = 16`(넘치면 가장 오래된 것을 버린다 · debug 로그) · ★이미 기억에 있는 `itemId` 는 다시 넣지 않는다★(해가 없는 방어 규칙 — 4판 리뷰 low). 보통은 0–1 개다.
   - **바꿔 쓰기 · 비우기** — `handle_line` 알림 갈래에서 번역 뒤 · 경계 뽑기 전 한 줄. `item/completed` 의 `params.item.id` 가 집합에 있으면 **꺼내고**(끝 상태와 무관 — 끝 줄이 곧 비우는 자리다), `events` 안의 `ToolResult { id, outcome: Failed | Declined }` 를 `Refused` 로 바꾼다. 번역기가 아무것도 안 냈으면(`completed` 등) 기억만 지운다. ★턴 끝에서는 비우지 않는다★ [고름]. 새 화신 = 새 리더 = 빈 집합.
   - **번역기는 이 집합을 모른다** [고름] — 번역기는 알림 줄만 받는 순수 번역기로 남는다(번역기는 `Refused` 를 내지 않는다).
   - **스위치는 없다 — 귀속은 늘 짓는다**(5판). 벤더가 우리 거절을 `declined` 로 닫아도 옳은 사유를 고르려면 귀속이 든다 — 파일 변경은 이미 그렇다.
9. **알려진 한계 넷을 받아들인다** [사용자 2026-09-27 · 메인 추천 — U7 과 한 묶음 · 귀속 장치를 더 키우지 않는다] — 집합은 화신마다 비고 벤더 item 에 사유 칸이 없다. 같은 화신 안의 재구독 replay 는 링의 `Refused` 를 그대로 받아 맞다.
   - ① **이력의 명령** — 이어받은 이력에 우리가 거절한 명령 행이 **없다**(「오류」도 「거부됨」도 아니다) (가능성 높음 — 소스 판독 `rust-v0.156.1`). 라이브 때 「거부됨」이던 행이 사라지고 묶음 개수와 「거부 N」이 하나씩 준다.
   - ② **상한에서 밀려난 명령** — 늦은 `failed` 끝이 기억에 없어 `Failed`(「오류」)다.
   - ③ **이력의 파일 변경** — 이력에 `declined` 로 남아 `Declined` 가 된다 (가능성 높음 — 소스 판독 `rust-v0.156.1`) — 배지 · 셈은 같고 사유만 「실행되지 않음」이다. 우리 사유를 잃지만 거짓은 아니다.
   - ④ **상한에서 밀려난 파일 변경** — ③ 과 같은 「거부됨 · 실행되지 않음」.
   - 바른 해법 = `docs/tracking.md` T-44(대시보드가 자동 거절을 그만두면 이 귀속이 필요 없어질 가능성이 높다).
   - ★판정의 틀★ — 사용자는 4판의 틀(재활성화 · 상한 밀림에서 우리 거절이 「오류」로 바뀐다)에서 판정했다. 판정 직후 메인이 정정한 사실(거절된 명령 행은 이력에 없어 다시 열면 사라진다 · 거절된 파일 변경은 `declined` 로 닫힌다)을 사용자에게 알렸고, 사용자는 이의를 달지 않았고 이어서 `Declined` 사유 문구를 정했다(TRD §11 ⑨ · §12).
10. **채팅 안의 승인은 다음 과제다** [사용자 U7] — 「다음 과제로 빼자. 지금 많이 엮여있어」(`docs/tracking.md` T-44). 이 라운드에서도 대시보드는 codex 승인 질문을 자동 거절하고, 거절된 호출을 「거부됨」으로 보이기만 한다.
11. **끊긴 뒤 늦게 `failed` 로 닫히는 codex 명령 = 그대로 붉은 「오류」** [사용자 2026-09-27 — 선택지 ⓐ] — 「일단 밑에 중단됨 줄이 있는데 뭐가 문제라는거야?」. 끊어도 명령 프로세스는 살아 있다가 실제 `exitCode` 로 닫히므로(가능성 높음 — 소스 판독), 0 아닌 종료면 늦은 끝은 `failed` 다. 그 끝은 그 행에 보통 붉은 「오류」로 붙고 묶음 머리가 「오류 1」 을 센다. 바로 아래 그 턴의 「중단됨」 행(ADR-0237 결정 6 — 강조)이 맥락을 주고 명령은 실제로 실패했으므로 거짓 표시가 아니다. **새 장치 없음** — 누산기에 규칙을 더하지 않고 붙이기 경로(결정 12)도 그대로다.
12. **claude 의 Esc 로 끊긴 도구(`tool_result` `is_error:true`) = 결정 11 과 같은 규칙** [메인 결정 · 사용자 위임 2026-09-27 — 「너가 생각하는 방향으로 하고 나중에 보고 개선하면 되지」] — 그 행은 오늘 붉은 `Error` 배지(`isErr = result?.isError === true`)를 그대로 달고, 묶음이 서면 머리가 「오류 1」 을 센다 · 바로 아래 강조된 「중단됨」 행이 맥락을 준다. 근거 = 사용자의 ⓐ 사유를 그대로 적용 · 두 백엔드가 한 규칙 · 새 장치 없음. 끊긴 턴 안의 `is_error` 를 다시 가르지 않는다.
13. **프론트 — 결과를 기존 행에 붙이고, 붙이기는 「새 내용이 왔다」가 아니다.**
   - 누산기 `tool` 항목에 칸 `resultMark: 'failed' | 'declined' | 'refused' | null`(`TurnOutcomeMark` 와 같은 결 — 정상 완료는 표식이 없다) [고름 · 5판 — 표식이 출처를 안다, 사유 줄을 가르려면 든다].
   - `ToolResult` 갈래: 뒤에서부터 같은 `id` 의 `tool` 항목을 찾아 copy-on-write 로 `resultMark` 를 바꾼다(모르는 낱말 → `null` + warn · 못 찾으면 버린다). **새 항목을 만들지 않는다** → `rowKindOf` · 레일 · `isRenderedItem` 무변경. `turnDone` 을 안 건드린다.
   - ★**`feed` 는 `false` 를 돌려준다 — 붙였든 못 찾았든**★ [3판 리뷰 Designer FIX · 사용자 동의 2026-09-27] — `feed` 의 반환값은 `RichSlot.tsx:222` 에서 대기 표시를 푸는 신호로만 쓰인다. 지난 턴 도구의 늦은 끝이 사용자가 새 글을 보낸 직후 · 새 턴이 답하기 전에 들면, `true` 일 경우 새 턴의 Wait 표시가 답 없이 꺼진다. 행의 다시 그리기는 그대로다(`RichSlot` 이 반환값과 무관하게 `setItems` 를 먼저 부른다). `feed` 의 doc 과 `RichSlot.tsx:218-221` 호출부 주석을 새 뜻(`false` = 대기를 풀지 않는 프레임)에 맞춰 고친다. Wait 자체의 규칙은 바꾸지 않는다(`docs/tracking.md` T-12).
   - 요약: `summarizeGroup` 의 `errors` = `resultMark === 'failed'` 이거나 claude 파싱의 `isError` 인 호출 · `declined` = `resultMark` 가 `'declined'` 또는 `'refused'`. 펼친 행 `ToolItemRow` 는 `isErr = result?.isError === true || mark === 'failed'`(codex 는 Out 칸이 없다).

## 거부한 대안
**선 모양**
- **`ToolCall` 에 선택 `status` 칸 + 누산기 id 병합** [고름] — 끝에서 `ToolCall` 을 한 번 더 내야 한다. 두 턴 분류기가 `ToolCall` 을 진행으로 세므로(`claude/mod.rs:541` · `codex/mod.rs:1206`) 턴 끝 뒤에 온 끝이 「턴 중」을 다시 켜 30 분 막힘 경로가 된다. 옛 셸은 끝마다 **같은 도구 행을 하나 더** 그린다 — 「표시할 수 없는 신호」 줄보다 나쁘다(틀린 내용이다).
- **`Structured{kind:…}` 탈출구(claude `tool_result` 모양 흉내 포함)** [고름] — 같은 두 분류기가 `Structured` 를 통째로 진행으로 센다(대기 입력 사건을 탈출구에 안 싣는 사유와 같다 — `types.rs:94-97`). claude 모양으로 지으면 옛 셸도 배지를 그리지만, 벤더 모양을 codex 번역기까지 넓혀 조사 §3-1 의 누수를 키운다.
- **판을 올려 거른다**(6 → 7 · `protocol/src/lib.rs:121`) — 셸이 데몬에 알리는 것은 인증 첫 프레임의 `protocol_version` 하나이고(`net/src/auth.rs:33-38`) 데몬은 같지 않으면 끊는다(`net/src/ws.rs:348-363`) · `Hello` 의 `capabilities` 는 데몬→셸 방향이고 지금 `None` 이다(`connection_core.rs:2214-2220`). 그래서 한 판 안에서 셸을 가를 값이 없다. 판을 올리면 옛 셸은 아예 못 붙어 거를 필요가 없어지지만, 개발 일상 조합(옛 데몬 + 새 셸 — discovery 가 재사용을 거부한다 `discovery/src/lib.rs:446`)을 표시 한 줄 때문에 끊는다.
- **모든 끝을 낸다**(`Completed` 까지) [고름] — codex 도구 호출마다 사건이 하나씩 늘고 옛 셸의 「표시할 수 없는 신호」 줄이 **모든** 호출에 붙는데, 화면이 얻는 것은 없다 — 표시는 오류·거부 수와 배지뿐이다.
- **claude 도 상태만 싣는 `ToolResult` 를 낸다** [고름] — 결과 **본문**은 여전히 벤더 블록에서 읽는다(펼친 행의 Out `StructuredTextView.tsx:262` · `rowKindOf` 의 skip `:390`). 상태만 싣는 중립 사건은 그 파싱을 옛 데몬 폴백으로 내리지 못하고 **같은 오류 비트가 두 곳에 산다**(한쪽이 낡는다). claude 는 **모든** 호출에 결과를 내므로 링 사건과 옛 셸의 표시 불가 줄이 claude 호출 수만큼 는다. 오늘 누수를 **키우지는 않는다** — codex 는 벤더 모양을 안 탄다.
- **`exitCode` 로 판정한다** [고름] — 0 아닌 종료를 우리가 실패로 다시 가르면 벤더와 다른 판정이 선다(t3code 도 `status` 만 본다 — `apps/server/src/provider/Layers/CodexAdapter.ts:1013-1019`).

**거부 표시**
- **거부를 오류로 함께 센다**(대안 A — Zed · vibe-kanban · paseo 선례 · 조사 §8-2) [사용자 U2-a — B 를 골랐다] — 권한에 막힌 것과 진짜 실패가 같아 보이고, 우리 앱의 거절은 전부 우리 정책이라(승인 창이 없다 — 조사 §8-3) 빨강은 없는 버그를 찾게 한다. A 는 추가 작업이 없다는 이점이 있었다. 스위치 `DECLINED_MARK = 'error'` 로 남는다.
- **`Declined` 하나에 사유 하나**(4판 설계) [4판 리뷰 FIX — 두 리뷰어 · 메인 채택] — codex 스스로의 거부(guardian · 네트워크 정책)에도 우리 사유가 붙는 거짓 사유 결함.
- **`Declined` 사유 「codex 가 실행을 거부함」**(메인 5판 문구) [사용자 2026-09-27 · 5판 light 재검] — 프론트 문자열은 백엔드 이름 없는 중립 낱말이어야 한다(`structuredAccumulator.ts:25` · ADR-0004). 문서화된 경로에서 거짓이다 — 복원된 우리 파일 변경 거절 · 상한에서 밀려난 우리 파일 변경 거절 · 벤더 준비 실패의 Declined 보고(`core/src/tools/events.rs:445-452@v0.156.1`). claude `ToolResult`(T-43)가 오면 codex 이름을 물려받는다.

**우리 거절 귀속**
- **채팅 승인을 이번에 짓는다** [사용자 U7 — **거부가 아니라 다음 과제**(T-44)] — 「다음 과제로 빼자. 지금 많이 엮여있어」. 지으면 승인 요청 · 답 경로 · UI 가 새로 들고 이 라운드의 귀속 · 턴 관측 · 대기 표시와 엮인다.
- **item 모양(`failed` + `exitCode:null` + `aggregatedOutput:null`)으로 거부를 판정한다**(4판 리뷰 갈림 2 · 3 을 한 번에 푸는 메인 제안) [메인 5판 · 사유 순서는 light 재검이 고쳤다] — 같은 모양을 내는 다른 길(스레드 unload · 되돌림으로 승인 콜백이 버려질 때 `beh:2032-2038@v0.156.1` · guardian 시한 초과 `beh:376-396@v0.156.1`)을 우리 거절로 읽어 거짓 사유를 붙인다. 거절된 명령은 이어받은 이력에 없어 복원도 돕지 못한다 — 얻는 것은 라이브에서 상한 16 에 밀려난 명령 하나뿐이다.
- **출처를 이력 복원까지 보존한다**(4판 리뷰 Designer — 같은 대화를 다시 열면 배지·요약 수가 바뀐다) [사용자 — 알려진 한계로 수락(결정 9)] — 바른 해법은 T-44 이다.
- **설치본이 우리 거절에 `declined` 를 보내면 귀속을 짓지 않는 스위치**(4판) [메인 5판] — 결말이 둘로 갈린 뒤엔 옳은 사유를 고르려면 귀속이 늘 든다. 파일 변경은 이미 `declined` 로 닫힌다(소스 판독).
- **거절 집합을 통로 `State` 에 둔다** [고름 — 메인 ⑧ 수락] — 쓰는 쪽(`transport.rs:3629` → `refuse`)과 읽는 쪽이 둘 다 리더 스레드라 락이 지킬 것이 없다. `State` 는 라이터 · `send_input` · 리더가 나눠 쥐는 임계 구역이라(`:382-384` doc) 락 구간과 「소유권 분할」 표의 칸만 는다(옳지만 얻는 것이 없다).
- **번역기에 공유 집합을 주입한다**(claude `TurnGate` 처럼 공유 `Arc<Mutex<…>>`) [고름] — 한 스레드인데 락이 하나 늘고 번역기 시험이 통로 상태에 묶인다.
- **턴 끝에서 집합을 비운다** [고름] — 끝이 턴 끝 뒤에도 온다(실측 `steer_m6` 50 줄). 비우면 그 늦은 끝이 붉은 「오류」가 된다.

**끊긴 뒤의 늦은 끝**(선택지 이름표 = TRD §11 ⑩ · 사용자 결정 ⓐ 가 아래 넷을 거부했다)
- **ⓑ-1 끊긴 턴의 늦은 결과를 숨긴다** — 진짜 실패가 안 보이고 새 로직이 든다.
- **ⓑ-2 늦은 실패를 그 행의 「중단됨」으로 보인다** — 규칙이 든다(사용자: 별도 장치 없이). 이력 페이지는 턴 결말을 싣지 않아(`backend/codex/decoder.rs:1179-1183` · ADR-0203) 다시 열면 적용이 안 된다.
- **ⓑ-3 끊는 순간 아직 도는 행을 모두 「중단됨」으로** — 계기가 늦은 끝이 아니라 끊는 순간이다. 번역기가 성공엔 아무것도 안 내(결정 3) 프론트가 성공으로 끝난 codex 행과 아직 도는 행을 못 가른다 — 새 성공 신호나 기억이 든다.
- **ⓒ 끊을 때 도는 명령 프로세스를 끝낸다**(벤더 실험 API `thread/backgroundTerminals/terminate` · `clean` · `list` — `app-server-protocol/src/protocol/common.rs:745-762@v0.156.1` `#[experimental]` · 인자 세부는 판독하지 않았다) [**거부가 아니라 후속 후보** — 아직 추적 항목 없음] — 끊을 때 쓰는 피어가 없다(codex TUI 는 끊을 때 프로세스를 죽이지 않고 도는 exec 칸을 그 자리에서 실패로 닫으며 백그라운드 터미널을 죽이는 수동 `/stop` 을 따로 둔다 `codex-rs/tui/src/chatwidget/turn_runtime.rs:314-352` · t3code · paseo 는 늦은 status 를 그대로 흘리고 끝내지도 않는다 `apps/server/src/provider/Layers/CodexAdapter.ts:1013-1020` · `packages/server/src/server/agent/providers/codex-app-server-agent.ts:4684-4692`). 결과 표시에 결정이 따로 든다.

**claude 끊긴 도구**(메인 결정 · 사용자 위임 — 결정 12)
- **끊김 `tool_result` 를 본문 글로 판별한다**(「The user doesn't want to proceed with this tool use…」) — 벤더 문자열에 기대 깨지기 쉽다.
- **끊긴 claude 턴 안의 `is_error` 를 「중단됨」으로 바꿔 단다** — 사용자가 codex 에서 ⓑ-2 로 거부한 규칙 장치와 같다.

## 근거
- **사용자 결정(2026-09-27 · TRD §1 · §11)** — U2(추천과 다름 · 근거는 맥락) · U2-a(따로 표기 · 사용자 근거 = 조사 §8-3) · `Declined` 문구(5판 light 재검) · U7 · 알려진 한계 수락(4판 리뷰 갈림 2 · 3 — 메인 추천) · 채취 둘째 ⓐ. 메인 결정 · 사용자 위임 = 결정 12(TRD §11 ⑪).
- **업스트림 codex 소스 판독** — 전부 소스 판독이고 실행 확인이 아니다. 확신은 TRD §4-7 「사실」이 항목마다 적은 대로다:
  - 확실(직접 확인): 도구 item 은 `item/started`(`status: inProgress`) 뒤 같은 id 로 `item/completed` 를 끝 상태와 함께 낸다 · 끝 어휘 = `commandExecution` · `fileChange` `completed | failed | declined` · `mcpToolCall` · `dynamicToolCall` `completed | failed` · `collabAgentToolCall` `completed | failed | interrupted` · `webSearch` 는 status 칸이 없다(t3code 생성본 `packages/effect-codex-app-server/src/_generated/schema.gen.ts:1928-1938` · `:2004-2009` · `:2278-2283` · `:2356-2366`). 보통 명령 승인은 `item/started` 를 요청보다 먼저 내고(`beh.rs:754-771` → `:785-806`), 끝은 우리 답을 받은 그 처리기가 곧바로 낸다(`:2058-2074`).
  - 우리 거절 → 명령 `failed` · `exitCode:null` · `aggregatedOutput:null` · 사유 칸 없음(main `beh.rs:2026-2032` · `:1446-1493` · 조사 §8-1 메인 대조 · 태그 `beh:1446-1491@v0.156.1` · `beh:2025-2031@v0.156.1` — 가능성 높음).
  - 우리 거절 → 파일 변경 `declined` · 시작은 승인 전 · 시작 · 승인 요청 · 끝이 같은 id · fileChange item 에 사유 칸 없음(`core/src/tools/handlers/apply_patch.rs:595` · `:608-623` · `core/src/tools/approvals.rs:460` · `core/src/tools/events.rs:333-343` · `beh:638-647` · `core/src/tools/events.rs:256` · `app-server-protocol/src/protocol/v2/item.rs:326-330` — 전부 `@v0.156.1` · 가능성 높음). 벤더의 `declined` 는 거절 전용 낱말도 아니다(`core/src/tools/events.rs:445-452@v0.156.1`).
  - 거절된 명령은 이어받은 이력에 없다(`app-server-protocol/src/protocol/thread_history_projection.rs:77-85` · `rollout/src/policy.rs:142-192` · `core/src/unified_exec/process_manager.rs:530-539` — `@v0.156.1` · 가능성 높음 · 실행 확인 아님).
  - 끊긴 명령은 계속 돌다 실제 `exitCode` 로 늦게 끝난다(`core/src/tools/parallel.rs:237-265` · `core/src/unified_exec/process_manager.rs:598-625` · `core/src/unified_exec/async_watcher.rs:186-247` — `@v0.156.1` · 가능성 높음 · `steer_m6` 실측과 맞다).
  - 같은 failed/null/null 끝을 내는 다른 길 — 스레드 unload · 되돌림으로 승인 콜백이 버려질 때 · guardian 시한 초과(`beh:2032-2038 · 376-396@v0.156.1` — TRD §4-7 ②-2 · 가능성 높음).
  - 태그 `rust-v0.156.1` = 커밋 `b412ff32c417` = 설치본 codex-cli 0.156.1 과 같은 판.
- **실측** — `steer_m6.jsonl`(끝이 턴 끝 뒤에 온다 — 맥락). t3code 시험이 `exitCode: 1` 을 `failed` 와 짝짓는다(`CodexAdapter.test.ts:1545-1552`) — codex 가 0 아닌 종료를 늘 `failed` 로 적는지는 **미검**이다(B5 채취가 확인).
- **피어 선례** — 「거부됨」 따로 표기 = t3code 「Declined」(`apps/web/src/components/chat/MessagesTimeline.logic.ts:86-96`) · cline 경고 톤(조사 §8-2). 오류로 함께 세기(대안 A) = Zed · vibe-kanban · paseo(조사 §8-2). 늦은 끝 = 위 ⓒ 항목의 피어 셋.
- **리뷰** — 3판 Designer FIX(늦은 `ToolResult` 가 Wait 를 지운다 → 결정 13) · 4판 두 리뷰어 FIX(결말 분리 · 주석 · 같은 id 한 칸)와 갈림 2 건(사용자 판정) · 5판 light 재검 FIX · 5판 좁은 재검 FIX(TRD §12).
- ★**검증 상태**★ — 구현 전이다(TRD 5판 · 코드 무변경). B5 채취 셋(실측 fixture `codex/fixtures/tool_fail_u2.jsonl` · 채취 둘째 · 채취 셋째 `codex/fixtures/refuse_u2a.jsonl`)은 아직 안 돌았다. 시험 계획 = TRD §4-7 ⑨(번역기 · 통로 귀속 ①–⑫ · 두 분류기 · 턴 끝 뒤 끝 · wire 골든 · 프론트) · GUI 실측 계획 = TRD §8-2 F3 ③ ④ ⑧ ⑨.

## 영향 / 불변식
- **턴 관측 · 오류 뒤 멈춤** — `ToolResult` 는 두 분류기에서 `None` 이다(결정 4). 통로의 도구 끝 계기(`is_tool_item` — `decoder.rs:282-288`)는 원 줄의 item 타입을 읽으므로 이 사건과 무관하다 — `tool_end_m7` 시험(`codex/transport.rs:9653`)이 무수정 초록이어야 한다.
- **codex 통로 락 순서 · 소유권 분할 무변경** — 거부 귀속 집합은 리더 전용 칸이다. `State` 칸 목록 · 락 순서(`Announcer.order → {상태 락, replay → …}`) 무변경 · 새 락 없음 · 바꿔 쓰기는 emit 전에 락 없이 한다.
- **ADR-0004** — 승인 메서드 이름 · `itemId`/`approvalId`/`networkApprovalContext` 읽기는 전부 `backend/codex` 안이다. 통로 밖(코어 · 데몬 · 프론트)은 중립 `ToolOutcome` 만 본다. 프론트 사유 문구에도 백엔드 이름이 없다.
- **ADR-0051** — 사유 줄은 `ToolItemRow` 의 DOM 안이다(새 항목도 새 행도 아니다) → `rowKindOf` · `railPositions` · `isRenderedItem` 무변경 · 레일 점은 줄 하나에 하나 그대로.
- **링 · 생성물** — `estimate_cost_bytes` 에 `ToolResult { id, .. } => id.len()`(실패·거부 호출당 사건 1 · 수십 바이트 — `REPLAY_MAX_EVENTS = 4096` 에 견주면 무시할 크기). 망라 match 전부를 한 커밋에(TRD §4-7 ⑦ 목록 · 데몬 변환 `tool_outcome_to_wire` 는 `_` 갈래 없이 일대일). 생성물 `ToolOutcome.ts` · `StructuredEvent.ts` 는 `cargo test -p engram-dashboard-protocol` 이 굽고 CI sync 게이트가 주인이다 — 손으로 쓰지 않는다.
- ★**새 변형이 프론트 `never` 망라(`structuredAccumulator.ts:243`)를 건다**★ — `protocol/tests/ts_export.rs:9-20` 이 protocol 시험을 돌릴 때마다 `bindings/StructuredEvent.ts` 를 다시 써, 새 변형은 B5 의 커밋이 아니라 첫 시험 실행에서 공유 트리에 내려앉는다. 그래서 FE-2 가 지역 합 + 타입 가드(`isToolResultEvent` — 가드 가지 안에서 칸을 읽지 않고 도우미에 통째로 넘긴다)를 먼저 커밋하고, B5 는 그 뒤에 **착수**한다(TRD §7 · §11 ⑥ — 메인 판정은 「대기」로 남아 있다).
- ★**B5 가 멈추고 메인에 반환하는 조건**★(TRD §4-7 ⑨ · §7 B5):
  - 채취 첫째 — 0 아닌 종료가 `completed` 로 오면(그러면 codex 명령 실패는 `status` 로 안 보이고, 판정을 `exitCode` 로 넓힐지는 사용자 체감이다).
  - 채취 셋째 — 명령 · 파일 변경 **각각** 끝의 item id 가 승인 요청의 `itemId` 와 다르다 · 끝이 안 온다 · 끝 status 가 `failed`/`declined` 가 아니다 · 그 밖에 그 턴이 `failed` 로 끝나거나 `error` 알림이 온다. 파일 변경도 멈춤인 이유 = 놓치면 우리 거절이 벤더 `Declined`(「거부됨 · 실행되지 않음」)로 **경보 없이** 보인다(명령은 놓치면 붉은 「오류」로 드러난다).
  - 채취 둘째는 확인이다(멈춤 아님) — 늦은 `failed` 끝은 예상대로다. 늦은 끝이 안 오거나 · 다른 id 로 오거나 · `failed` 가 아닌 늦은 status(`interrupted` · `completed`)면 기록해 반환에 싣는다.
- **거부 귀속은 업스트림 매핑에 기댄다** — 벤더가 바꾸면(끝을 안 내거나 다른 id 로 내면) 기억은 상한에서 밀려나고 표시는 없음 · 명령은 「오류」 · 파일 변경은 「거부됨 · 실행되지 않음」 — **경보 없이 조용히 틀린다**(status 가 `failed`↔`declined` 로 바뀌는 것은 둘 다 `Refused` 로 바꿔 쓰므로 무해하다). 설치본(0.156.1)의 이 표류는 채취 셋째의 멈춤이 잡는다 · 그 뒤의 벤더 판 변경에는 여전히 경보가 없다.
- **같은 모양을 내는 다른 길** — failed/null/null 끝은 우리 거절만의 것이 아니다(승인 콜백 버려짐 · guardian 시한 초과 — 자동 심사를 켤 때만 · 우리 스폰은 안 켠다) (가능성 높음 — 소스 판독 `rust-v0.156.1`). 기억에 없으니 `Failed`(「오류」)로 보인다.
- **기억에 없는 거부의 사유는 이유를 말하지 않는다** — 「실행되지 않음」은 기억에 없는 `declined` 의 출처 전부에서 참이지만(가능성 높음) 왜 안 돌았는지는 알려 주지 않는다. 벤더 스스로의 거부가 우리 앱에서 얼마나 나오는지는 모른다 — 사용자 codex 설정에 달렸을 수 있다(불확실).
- **늦은 끝** — 끊긴 호출의 끝이 다음 턴 뒤에 오면 지난 턴의 행에 배지가 붙는다. 그 프레임이 「보낸 직후 · 첫 응답 전」 창에 들어도 대기 표시는 꺼지지 않는다(결정 13). ★같은 부류가 `Usage` 에는 남는다★ — 이번 범위가 아니고 Wait 재설계(T-12)의 몫이다.
- **codex 판정 = 벤더 `status`** — 「못 찾음」을 종료 1 로 알리는 검색 명령을 벤더가 `failed` 로 적으면 「오류」로 보인다.
- **claude 결과 파싱은 그대로** — 프론트가 벤더 `tool_result` 모양을 읽는 자리(조사 §3-1)는 이번에 걷지 않는다(T-43).
- **골든** — 대기 입력 골든(`queued_input_golden.json`)은 무영향이다(이 사건은 명부 환원에 안 든다). 기존 codex fixture 셋(`empty_turn_m9` · `steer_m6` · `tool_end_m7`)은 도구 끝이 전부 `completed` 라 사건열이 그대로다.
- **함께 고칠 문서** — `codex/decoder.rs:262-263` · `:523-525` · `:623` 앞 주석(끝에서 결과를 낸다) · `codex/transport.rs:3052-3053`(`refuse` doc — 거절한 승인 item 을 기억한다) · CLAUDE.md 「핵심 불변식」 「대기 입력 상태」 끝 문단의 「턴 끝 뒤에 오는 사건」 예에 `ToolResult`(선택) — 착지 라운드에서 `/review doc`(TRD §10 끝).
