# ADR-0235: codex 턴 도중 입력 단순화 — 도구가 끝나면 쥔 글을 끼워 넣고 되울림으로만 받음을 치며 ✕ 는 뺐다는 답을 보고서야 줄을 지운다

- 상태: 확정 (2026-09-26, 근거: 사용자 결정 2026-09-26(인용 원문 = `.claude/handoff/attachments/20260926-midturn-impl/simplify-plan.md` · 핸드오프 `.claude/handoff/history/20260926-225442-…md` · `c43d382` 판 `simplify-plan.md` — 인용별 출처는 「근거」 첫 항) + 피어 서베이 + 구현 S1–S5 로컬 착지(조각마다 게이트 녹 · 리뷰·GUI QA·CI 전))
- 관련: Amends ADR-0231 (결정 2 넘기기 정책, 결정 3과 4의 ✕ 규칙, 결정 6과 8의 codex 몫, 결정 7 후속 턴 빚, 결정 9 정산과 수락 모름) · Amends ADR-0234 (결정 1 빚 판정, 결정 2의 항목 처분, 결정 4 넘기기 기본값) · 계획 `.claude/handoff/attachments/20260926-midturn-impl/simplify-plan.md`(사용자 결정 · 목표 동작 1–11 · 걷은 것) · `docs/research/codex-mid-turn-input-peer-implementations-2026-09-26.md` · TRD `docs/process/S21-codex-backend/trd-mid-turn-input-queue.md`(§5-5 의 codex 통로 서술이 이 결정에 밀린다 · §0 머리 표지 — §0 ③ · §5-2 · §5-6 의 ✕ 규칙은 결정 7 에, codex 오류 뒤 멈춤 풀림(§0 ⑦ · §5-5 · §5-6 `stopped_after_error`)은 결정 4 에 밀린다) · `docs/research/mid-turn-m15-measurements-2026-09-26.md`(계측 대상이 `engram::codex_handover` → `engram::codex_steer` 로 바뀌었다) · 관련 ADR-0233(첫 턴 영속 `witness_first_turn` 은 그대로 — 개정 아님) · ADR-0006(방출 줄 `order` 팬아웃 예외 = ADR-0234 결정 5 그대로) · ADR-0193(턴을 여는 자리는 여전히 `take_turn_locked` 하나) · step-log S21

## 맥락
ADR-0231 · ADR-0234 가 정한 codex 턴 도중 입력(P2 `3014247` · P8 `37ecc5e`)은 통로 안에 기계장치를 여럿 쌓았다 — 경계 판정과 넘기기 정책(`HandOverPolicy` · 구간 `Zone` · 운영 기본 `AtEarliestBoundary` · 답 구간 붙듦 `ANSWER_SEGMENT_HOLDS`), 턴마다 steer 응답을 기다리는 정산(`settling` · `steers_owed`), 기록만 된 글에 빈 `turn/start` 를 내는 후속 턴 빚, 수락 모름 단계(`Stage::Unconfirmed` · 이상 계수), 사용자 턴이 성공해야 풀리는 오류 뒤 멈춤, M15 계측. `transport.rs` 는 약 12.9k 줄이었고 P2 가 production 약 2.1k 줄을 더했다(핸드오프 2026-09-26 17:47).

사용자는 P8 뒤에 단순화를 함께 보자고 했고(「다끝난뒤에 단순화 한번 나하고 얘기해보자」), 그 자리에서 이 설계를 과설계로 물렸다 — 「도저히 파일 여러개 필요한 구현이 아닌데」 · 「그냥 도구하나 사용 이벤트 끝나면 그냥 슬쩍 끼워넣으라고」. 피어 서베이(`docs/research/codex-mid-turn-input-peer-implementations-2026-09-26.md`)를 돌린 뒤 사용자와 다시 정했고(아래 결정), codex 경로를 순차 조각 S1–S5 로 줄였다(`672df0a` `5bcd7f6` `621c7ae` `c43d382` `2237078` `97eaa52` · 범위 `338740f..97eaa52` 24 파일 +2202/−4166 (.claude 제외)). 못 뺐다는 답을 받은 행의 ✕ 규칙은 그 뒤에 따로 정했다(`af30654`).

★같은 라운드 안에서 사용자 결정 하나가 뒤집혔다★ — S3b(`c43d382`)는 「실패는 지워 … 에러 났는데 뭔가 대기목록에 뜨는것도 이상하잖아」에 따라 실패로 끝난 턴의 에코 없는 글을 지웠고, 사용자가 곧이어 선택지 B(「B가 크게 구현 힘들지 않으면 해」 — 오류에도 전부 두고 멈춘다)로 바꿨다(`d724387` · S4 `2237078` 에 반영). 이 ADR 은 B 를 적는다.

★ADR-0231 이 거부했던 대안 셋이 이 결정으로 되살아난다★ — 모순이 아니라 사용자 번복이다. ① 「구간별 ✕」(넘긴 뒤엔 ✕ 를 숨긴다 — 2026-09-25 사용자 거부) → 결정 7 의 「보냄 행은 ✕ 없음」. ② 「누른 창에서도 결말까지 남긴다」(Q7 후속 거부) → 결정 7 의 「뺐다는 답을 보고서야 지운다」. ③ 「오류 뒤 기다림을 도착 사건으로 푼다」([고름] 거부) → 결정 4 의 「다음 사용자 글이 푼다」 — 그 거부 사유(✕ 로 빠진 글 때문에 남은 항목이 저절로 나간다)는 아래 「받아들인 결과」에 그대로 적었다.

## 결정
표기는 ADR-0231 범례를 따른다 — **[사용자]** = 사용자가 답한 결정 · **[고름]** = 구현·계획이 스스로 고른 내부 구현. 범위 = codex JSON(`TransportOwned`) 경로. claude 경로는 두 백엔드가 함께 쓰는 프론트 ✕ 규칙(결정 7)만 바뀐다.

1. **넘기는 때 = 도는 턴의 도구 끝 — t3code 모양** [사용자]. 턴 도중 친 사용자 글은 통로가 쥐고(목록 행 「대기」 · codex 에는 아무것도 안 보낸다), 우리 thread 의 **지금 턴 id** 의 도구 항목(`is_tool_item`)이 `item/completed` 되면 계기(`steer_due`)가 서고 라이터가 쥔 사용자 글을 **전부** 친 순서대로 넘긴다 — 항목마다 `turn/steer` 하나 · 제 `clientUserMessageId`. 턴 id 가 오기 전의 도구 끝은 세지 않는다(`expectedTurnId` 가 없다). 병렬 도구도 끝마다 계기다. 우편은 steer 하지 않는다(Idle 에 나간다). 경계 판정 · 구간 · 답 구간 붙듦은 없다 — 도구가 없는 턴에 친 글은 턴 끝을 기다린다.
2. **받음 = 되울림 하나 · 글은 늘 보인다** [사용자] — 「내가 입력하고 대기가 되던가 창에 뜨던가 해야됨. 붕뜨면 안됨. 그게 가장 중요」. 글은 언제나 목록 행이거나 말풍선이고 둘 다 아닌 순간이 없다. 목록에서 받음으로 빠지는 길은 그 `clientId` 를 실은 codex `userMessage` 되울림(`Delivered` → 행이 빠지고 대화 끝 말풍선) 하나뿐이다. steer 성공 응답은 아무것도 옮기지 않는다.
3. **에코 없이 끝난 글은 쥔 자리로 — 끝의 모양과 무관하게** [사용자 — 선택지 B]. 넘긴 글이 에코 없이 턴이 닫히면(`Completed` · `Unknown` · `Interrupted` · `Failed` · `Rejected` · `Unanswered` 어느 것이든), 또는 steer 가 거절되거나(−32600 · not steerable · 턴 id 불일치) 쓰기에 실패하면 그 글은 제자리(친 순서)에서 「대기」로 돌아가 ✕ 가 다시 선다. ✕ 가 닿은 글은 되돌리지 않고 `Dropped{Withdrawn}`. 넘긴 우편은 사건 없이 빠진다(그대로). 한가할 때 그린 말풍선(Direct)도 지우지 않고 쥐어 다음에 다시 보낸다. 정산 · 수락 모름 · 빚은 없다. 거절·쓰기 실패가 난 턴은 표시(`steer_refused`)해 같은 턴에 되풀이 보내지 않는다 [고름].
4. **오류 뒤 멈춤 = 실패 끝에서 서고 다음 사용자 글이 푼다** [사용자]. `Failed` · `Rejected` · `Unanswered` 가 통로 칸 `halted: bool` 을 세운다(하한 충족 화신만). 서 있는 동안 Idle 은 턴을 하나도 열지 않는다(쥔 사용자 글 · 우편). 풀리는 때 = 사용자 글이 드는 순간(`push_pending`) · 통로 닫힘 — ✕ 와 시간은 풀지 않는다. 끊기(`Interrupted`) · `Completed` · `Unknown` 은 세우지 않는다. **끊기 = 벤더 Esc** — 쥔 글과 되돌아온 글이 다음 Idle 에 곧바로 나간다(「esc누르는게 도구 기다리기 귀찮고 내꺼 빨리 푸쉬할려고」). 오류 멈춤(예: 사용 한도)은 저절로 보내지 않는다. 우편이 통로에 드는 문 = 커널 `last_end_failed` 는 그대로다.
5. **턴 끝에 남은 글은 한 턴에** [사용자]. Idle 은 가장 오래된 사용자 `Held` 로 다음 턴을 열고(`turn/start`), 그 턴 id 가 알려지면 나머지를 곧바로 그 턴에 넘긴다(계기를 세운다 — 결정 1 경로). Idle 순서 = 사용자 `Held` → 우편.
6. **목록 행 상태 셋 = 대기(✕) · 보냄(✕ 없음) · 말풍선** [사용자]. 보냄 표지는 링 사건 `QueuedInputEvent::HandedOver{id, sent}` 이다 [고름] — steer · `turn/start` 를 쓴 뒤와 쥔 자리로 되돌린 뒤에 내고, 값은 부르는 쪽이 정하지 않고 항목의 지금 단계를 다시 읽어 바뀌었으면 또 낸다(`publish_hand_over` — 링의 마지막 표지 = 마지막 단계). 명부 행 `QueuedRow.sent` · 목록 조회 낱말 `sent` 라서 재부착·팝아웃에도 보냄이 남는다. `PROTOCOL_VERSION` 은 올리지 않는다(6 그대로) [고름].
7. **✕ = 뺐다는 답을 보고서야 줄을 지운다 — 두 백엔드(프론트 공용)** [사용자 — ✕ 선택지 A] — 「a를 하자 … X누르는것도 목록에서 실제로 빼는거 보고 빼야될듯 그냥 무조건 없애는게 아니라」.
   - 보냄 행에는 ✕ 가 없다(넘긴 글은 거둘 수 없고 받음으로만 빠진다).
   - ✕ 를 누르면 행은 남고(✕ 비활성) 백엔드의 제거 사건(`Dropped{Withdrawn}` · `CancelAnswered{removed:true}`)이 와야 빠진다 — 곧바로 감추지 않는다. codex 쥔 행은 통로 `withdraw` 가 곧바로 `Dropped{Withdrawn}` 을 낸다. claude 는 CLI 취소 답이 뺐다고 할 때만 빠지고, 아니면 `Delivered` 까지 남는다.
   - **못 뺐다는 답이 온 행은 보냄 행처럼 그린다(✕ 없음)** [사용자] — claude `removed:false` · `CancelFailed`, codex ✕ 가 넘기기와 겹친 행(`TooLate` → `CancelAnswered{removed:false}`) — 라이브와 재부착 모두. 「이미 넘어가면 codex와 같이 못누르게 하면 되지 않음?」. 프론트만 바뀐다(목록 조회와 링 사건이 이미 `not_removed` 를 싣는다 — `af30654`).
   - LLM 제어는 그대로다 — `CancelQueuedInput` · `ListQueuedInputs`(행 칸이 늘지 않고 행 `state` 의 값에 `"sent"` 가 더해졌다 — `ListedState::Sent`).
8. **하한 미달 codex(0.140 미만 — 되울림에 `clientId` 없음) = 이 기능 전체가 꺼진 채 오늘 동작** [사용자]. 하한 판정은 그대로 둔다.
9. **계측 = 항목마다 debug 한 줄** [고름 — 계획 항목 11]. `tracing::debug!(target: "engram::codex_steer")` · 단계 `tool_start` · `tool_end`(쥔 수) · `steer` · `write_failed`(steer 를 못 썼다) · `refused` · `echo` · `held_again`(끝의 모양) · 턴 id 와 에이전트 id(`agent`)를 싣고, 메시지는 단계마다 한 줄 한국어다(코드 리뷰 반영 2026-09-27 — 여러 codex 에이전트를 한 데몬이 띄워도 줄을 가를 수 있게, 쓰기 실패도 이 계측만 보는 쪽에 보이게). M15 계측(`engram::codex_handover`)을 대신한다. 목적 = 아래 「영향」의 열린 물음.
10. **UI 정지 버튼·단축키는 지금 넣지 않는다** [사용자] — 「나중에 단축키 시스템 만들때 넣을거임」.
11. **이 설계를 넘는 것은 먼저 묻는다** [사용자] — 「단순하게 가고 복잡할것같으면 먼저 나에게 보고해」. 여기 적은 칸을 넘는 새 상태 · 새 사건 변형 · wire/`PROTOCOL_VERSION` 변경 · 새 기제가 필요해지면 짓기 전에 사용자에게 보고한다.

## 거부한 대안
- **P2/P8 codex 설계를 그대로 둔다**(ADR-0231 결정 2 · 7 · 9 · ADR-0234 결정 1 · 4) [사용자] — 「도저히 파일 여러개 필요한 구현이 아닌데」 · 「그냥 도구하나 사용 이벤트 끝나면 그냥 슬쩍 끼워넣으라고」. 크기: `transport.rs` 약 12.9k 줄(P2 가 production 약 2.1k 줄) 대 피어의 같은 기능 150–1.8k 줄(t3code 약 450 — 서베이 비교표, 크기 확신도는 표에 딸림) · 걷어낸 결과 24 파일 +2202/−4166 (.claude 제외). 부품별 사유:
  - **경계 판정 · 넘기기 정책 · 구간 · 답 구간 붙듦**(`HandOverPolicy` · `Zone` · `AtEarliestBoundary` · `ANSWER_SEGMENT_HOLDS`) — 경계를 재거나 타이밍 경합을 거는 피어가 없다(서베이 결론 3 · 확실): 벤더는 core 가 루프 머리에서 접어 넣는 것에 기대고, t3code 는 새 `tool.completed` 가 보이는지만 본다.
  - **턴 끝 정산**(steer 응답을 기다림 · `settling` · `steers_owed`) — 받힌 steer 의 에코는 `turn/completed` 앞에 온다(벤더 소스 0.156.1 · M10 적중 3건 모두 4.5–4.8 ms 앞 — ADR-0231 거부한 대안 「턴 끝 탐침」) → 턴 끝에 에코 없는 글은 안 받혔다. 벤더 TUI 도 steer 응답이 아니라 `clientId` 에코로 정산한다(서베이 2-1 · 확실).
  - **후속 턴 빚**(빈 `turn/start`) — 확인한 피어(벤더 TUI · t3code · vibe-kanban)에는 없고 벤더 TUI 도 그 손실을 받아들인다(서베이 결론 2 · 불확실 — paseo · Zed 미조사).
  - **수락 모름 단계**(`Stage::Unconfirmed` · 이상 계수) — 정산과 같은 근거(턴 끝에 에코 없음 = 안 받힘)로 결정 3 의 되돌림 하나가 대신한다. ADR-0231 의 N10 · N12 (a) · N13 (b) 답은 codex 에서 대상이 없어졌다.
  - **끊긴 턴의 넘긴 글을 버린다**(`Dropped{Interrupted}` — P2 로컬 QA 시도 7 에서 관측된 착지 동작) — 「esc누르는게 도구 기다리기 귀찮고 내꺼 빨리 푸쉬할려고」. 벤더 TUI 의 Esc 도 interrupt 뒤 대기 steer 를 한 턴으로 합쳐 재제출한다(서베이 2-1 · 확실).
  - **실패로 끝난 턴의 글을 지운다**(ADR-0231 결정 6 의 codex 몫 · 한때 사용자 결정 「실패는 지워 … 에러 났는데 뭔가 대기목록에 뜨는것도 이상하잖아」 — S3b `c43d382` 에 구현했다가 되돌림) [사용자] — 선택지 B 「B가 크게 구현 힘들지 않으면 해」: 오류에도 전부 두고 멈춘다(결정 3 · 4).
  - **오류 처리 선택지 A · C** — 제시한 문구가 기록(`simplify-plan.md` · 핸드오프)에 남지 않았다. 남은 것은 채택한 B 와 그것이 대신한 「실패는 지워」뿐이다.
- **벤더 즉시 steer**(Codex CLI TUI 의 Enter · paseo 기본값 — 친 즉시 `turn/steer`) [사용자 — **거부가 아니라 미룸**] — 「일단 t2code 방식으로 해보되 도구3에서 받으면 그냥 공식벤더로 해야되는걸 고민해야될듯」(「t2code」 = t3code · 사용자 오타 — 인용은 원문 그대로). 도구 끝을 먼저 해 보는 근거: codex 는 도구 끝까지 붙들어도 같은 경계에 든다(M7 10/10 — ADR-0231 맥락) · 붙드는 동안은 ✕ 가 글을 거둔다(ADR-0231 거부한 대안 「codex 를 도구 시작에 넘긴다」 — 「✕ 창이 더 길다」) · `Immediate` 아래 P2 QA 에서 넘긴 행이 약 30 초 회색으로 남고 ✕ 가 행을 지웠는데 글은 그대로 갔다(핸드오프 2026-09-26 17:47 QA 관측). **재론 조건** = 아래 「영향」의 열린 물음이 「도구 1 뒤에 친 글이 도구 3 뒤에야 든다」로 답해지면 사용자가 벤더 즉시 steer 를 고민한다.
- **턴 끝까지만 붙든다**(서베이의 모양 ③ — vibe-kanban · paseo queue · 벤더 TUI Tab · ADR-0190 의 옛 큐) [사용자] — 사용자가 도구 끝을 지시했고(위 인용), 원칙 「받을 수 있는 가장 이른 때 전달」(ADR-0231 결정 2 — 사용자 원칙 · 이 ADR 이 바꾸지 않는다)과 M7 10/10(도구 끝에 넘겨도 같은 경계)이 그대로 선다.
- **✕ 를 누르면 곧바로 줄을 없앤다**(ADR-0231 결정 4 — 모든 창에서 곧바로 감춤) [사용자 — ✕ 선택지 A 를 고름] — 「… X누르는것도 목록에서 실제로 빼는거 보고 빼야될듯 그냥 무조건 없애는게 아니라」. P2 QA 에서 ✕ 가 행을 지웠는데 글은 그대로 간 관측이 이 모양의 실례다(위 항목과 같은 출처). ✕ 선택지 B 의 원문은 기록에 없고, 이 항목은 인용이 대비한 「무조건 없앰」으로 적었다.
- **넘긴 행에도 ✕ 를 둔다**(ADR-0231 결정 3 — 목록의 모든 항목에 늘 · 넘긴 codex 항목은 목록에서만 뺀다) [사용자 — 결정 7] — 넘긴 글은 거둘 수 없다. 이미 넘긴 항목을 취소하는 기능은 확인한 피어(벤더 TUI · t3code · vibe-kanban)에 없고 ✕ 는 클라이언트가 항목을 쥔 동안만 있다(서베이 결론 4 · 불확실 — paseo · Zed 미조사).
- **못 뺐다는 답이 온 행을 보통 행으로 되돌린다**(S5 `97eaa52` 착지 모양) [사용자] — 다시 선 ✕ 는 눌러도 아무 일도 하지 않는다(세션에 취소가 이미 걸려 있다 — 죽은 ✕ · `af30654` 커밋 본문). 「이미 넘어가면 codex와 같이 못누르게 하면 되지 않음?」.

## 근거
- **사용자 결정 2026-09-26** — 목표 동작 1–11 과 대부분의 인용 원문의 정본 = `.claude/handoff/attachments/20260926-midturn-impl/simplify-plan.md` · 결정 목록 = 핸드오프 `.claude/handoff/history/20260926-225442-codex-midturn-단순화-S1-S5-완료-다음은-S6문서-리뷰-QA.md` 「Decisions this session」. ★인용 몇은 계획 파일 지금 판에 없다★ — 「도저히 파일…」 · 「B가 크게…」 · 「t2code…」 · 「esc누르는게…」 · 「단축키 시스템…」은 위 핸드오프에만 있고, 「실패는 지워 … 대기목록에 뜨는것도 이상하잖아」의 뒷부분은 `c43d382` 판 계획 파일에만 남았다(`git show c43d382:.claude/handoff/attachments/20260926-midturn-impl/simplify-plan.md` — 지금 판은 앞부분만 싣는다). 「다끝난뒤에…」는 핸드오프 `.claude/handoff/history/20260926-174757-…md` 와 `.claude/handoff/attachments/20260926-midturn-impl/contacts.md` 에 있다.
- **피어 서베이** — `docs/research/codex-mid-turn-input-peer-implementations-2026-09-26.md`(medium · 소스 직독 5 프로젝트 · cross-family 적대 리뷰 FIX 5건 반영). 모양 셋(즉시 steer · 도구 끝까지 · 턴 끝까지)과 결론 2–4 가 위 거부 사유의 출처다.
- **실측** — M7(codex 는 도구 끝까지 붙들어도 같은 경계 10/10 — ADR-0231) · M10(받힌 steer 의 에코가 `turn/completed` 앞 — ADR-0231 거부한 대안). ★M15 는 도구 끝이 직접 깨운 steer 를 한 건도 재지 않았다★(ADR-0234 근거) — 결정 1 의 경로를 끝에서 끝까지 잰 값은 아직 없다.
- **구현** — S1 `672df0a`(계측 교체) · S2 `5bcd7f6`(빚 제거 · 멈춤 bool) · S3a `621c7ae`(되돌림 규칙 하나 · 수락 모름 제거) · S3b `c43d382`(정산 제거) · S4 `2237078`(계기 = 도구 끝 · 선택지 B) · S5 `97eaa52`(보냄 상태 · ✕ 규칙) · `af30654`(못 뺐다는 답의 행).
- ★**검증 상태(2026-09-26)**★ — S1–S5 조각마다 격리 실행으로 게이트 녹: `cargo fmt --check` · `cargo build` · agent 1093 통과(11 무시) · daemon 746 통과(3 무시) · S5 에서 protocol 97 · `lib_unit` 342 · `npx tsc --noEmit` · `npm test` 70 파일 1342. **안 돈 것**: 워크스페이스 회귀 · `/review code` · `/review doc` · GUI QA(`/qa full`) · CI(push 전). 병렬 도구에서 첫 도구 끝에 넘기는 경우는 미검. `af30654` 의 게이트는 이 ADR 이 확인하지 않았다.

## 영향 / 불변식
- **통로 모양(`backend/codex/transport.rs` 의 `State`)** — `pending`(단계 `Held` · `InFlight{gen, turn_id}`) · `steer_due` · `halted: bool` · `steer_refused` · 하한 판정 · 두 단계 방출. 걷은 것: `HandOverPolicy` · `Zone` · `ANSWER_SEGMENT_HOLDS` · `Segment` · `Signal` · `with_hand_over` · `HAND_OVER_POLICY` · 디코더 `ItemClass`(→ `is_tool_item`) · M15 계측 · 정산(`Settling` · `SteerOwed` · `settling` · `steers_owed`) · 빚(`unanswered` · `follow_up_owed` · `Opening::FollowUp` · 픽스처 `record_only_m10.jsonl`) · `Stage::Unconfirmed` · 이상 계수. 남긴 것: 하한 판정 · 두 단계 방출(`Queued` 가 `Delivered` 보다 먼저)과 방출 줄 `order` 의 팬아웃 예외(ADR-0234 결정 5 그대로) · `witness_first_turn`(ADR-0233) · 턴을 여는 자리 `take_turn_locked` 하나(ADR-0193).
- **어기면 깨지는 것**
  - steer 성공 응답으로 항목을 옮기거나 빼면 에코 전에 행이 사라져 목록에도 대화에도 없는 글이 생긴다(결정 2 위반).
  - `close_turn_items` 는 끝의 모양마다 갈래를 따로 적는다(`_` 로 묶지 않는다) — 새 끝이 생기면 컴파일이 멈춰야 그 끝의 처분을 정한다. 이름 `close_turn_items` · `end_turn_if` 와 Idle 로 가는 자리는 소스 스캔 시험 `every_place_that_idles_a_turn_is_accounted_for` 가 센다.
  - 넘김 표지를 고정값으로 내면 늦게 낸 쪽의 낡은 값이 링의 마지막 표지로 남는다(넘긴 글이 「대기」로 · 쥔 글이 ✕ 없는 「보냄」으로) — `publish_hand_over` 는 다시 읽어 마지막 표지 = 마지막 단계를 지킨다. 표지는 방출을 마친 목록 항목만 낸다(링에서 `Queued` 를 앞서지 않는다).
  - codex 통로 시험의 도우미(`trace` · `list_ops`)는 `HandedOver` 를 걸러 정확 일치 단언의 뜻을 지킨다 — 목록 사건 변형을 더하면 같은 식으로 거른다.
- **받아들인 결과**(단순한 규칙에서 나온 것 — 사용자가 하나씩 확인했다, 2026-09-27)
  - 멈춤이 사용자 글이 드는 순간 풀린다(전: 그 사용자 턴이 성공해야 풀렸다). 그 턴이 끊기면 통로에 이미 든 우편이 나갈 수 있다. [사용자] 「ㅇㅇ 심플하잖아 그게 구현도」.
  - 멈춤을 푼 그 글을 ✕ 로 빼도 멈춤은 다시 서지 않고 앞서 쥔 글이 나간다(ADR-0231 이 거부했던 「도착 사건으로 푼다」의 거부 사유 그대로). ✕ 를 누른 그 글 자체는 새지 않는다 — 아직 쥔 글이면 빠지고, 넘긴 글이면 ✕ 가 없다. [사용자] 「ㅇㅇ」.
  - `turn/start` 가 실패한 한가 말풍선(Direct)은 남아 다음 사용자 글과 함께 다시 나간다. 목록 밖이라 ✕ 가 없다. ★지금은 받아들이되 빈도를 지켜본다★ — [사용자] 「이건 나오는거 보고 대응하자. 많이 나오면 나중에 개선」. 보류한 대안 = 실패하면 말풍선을 거둬 목록의 쥔 줄로 되돌린다(말풍선을 지우고 줄을 다시 세우는 장치가 새로 든다).
  - 따로 선 규칙이 아니다 — 규칙은 「다음 기회에 넣는다」 하나이고 그 기회는 들어오는 사건 종류가 정한다(도구 완료 → steer · 턴 끝 → 다음 턴). 그래서 모델이 답만 쓰는 동안 친 글은 턴 끝에 든다. [사용자] 「어쨋든 다음 채팅올때니깐 니가 알아서해」 · 「우리는 채 오는걸로 판단하고 있는거잖아. 물론 타입에 따라 다르겠지만」.
  - 끈질긴 실패(예: 사용 한도)면 새 사용자 글마다 쥔 글을 한 번씩 다시 시도한다(시간 간격 자동 재시도 없음 · 버리지 않음). 상한 = `INPUT_QUEUE_LIMIT`(32) — ★사용자 목록이 아니라 통로 대기 큐 `pending` 전체를 센다★(우편 · 쥔 Direct 말풍선도 든다 — `backend/codex/transport.rs` 의 `INPUT_QUEUE_LIMIT` 검사) 그래서 사용자 입력이 33 번째보다 먼저 거절될 수 있다. 넘치면 조용히 버리지 않고 오류로 거절한다. [사용자] 「이대로 받아들이면되는데」.
  - 빚을 걷었으므로 steer 로 든 글이 기록만 되고 답을 못 받는 경우(M10 — ADR-0234 결정 1 의 「기록만」 현상)는 그대로 남는다 — 벤더 TUI 도 그 손실을 받아들인다(서베이 결론 2). ★받아들이되 나중에 다시 본다★ — [사용자] 「일반 받아들이고 나중에 한번 살펴보자. 어쨋든 대화가 의미없게 남는다는거잖아」. 되울림이 온 글은 codex 대화에 든 것이라 목록 밖(✕ 없음)이고 우리 쪽에 대화에서 빼는 수단은 없다.
- ★**열린 물음(GUI QA 몫)**★ — 도구 1 이 끝난 뒤 넘긴 글이 도구 2 전에 드나, 도구 3 뒤에야 드나. `engram::codex_steer` 계측으로 본다(`RUST_LOG` 가 데몬에 닿아야 한다). 늦게 들면 벤더 즉시 steer 를 사용자가 다시 본다(거부한 대안 둘째 항).
- **남은 잠든 배관** — codex 수락 모름의 배관이 `queued_input.rs` · `session.rs` · `commands.rs` · `manager.rs` · `connection_core.rs` · protocol · bindings 에 남아 있다(codex 의 trait 기본값은 빈 목록). 정리 후보이고 이 결정에서 걷지 않았다.
- **다음 단순화 후보(사용자 암시 · 미결)** — 데몬이 쥔 목록(명부 + 링 사건 + seq 연속). QA 뒤 사용자와 논의한다.
- **코드 앵커** — 남긴 자리는 `// ADR-0231`(과 `TurnClose` · `Announcer` · `announce` 의 `// ADR-0234`)을 단다. ADR-0234 「영향」이 적은 `HAND_OVER_POLICY` · `State::unanswered` 앵커는 그 심볼과 함께 사라졌다. 이 결정의 규칙을 지는 여섯 자리(`publish_hand_over` · `close_turn_items` · `take_steer_locked` · `note_tool_item` · `push_pending` 의 멈춤 풀기 주석 · 필드 `State::halted` — `backend/codex/transport.rs`)에는 `// ADR-0235` 를 더 단다. 찾는 법 = `rg "ADR-023[145]" crates src`.
