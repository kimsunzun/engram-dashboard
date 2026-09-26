# ADR-0234: 턴 도중 입력 착지 보정 — 후속 턴 빚은 steer 로 든 글만 지고 뜻 모를 턴 끝은 멈춤을 건드리지 않으며 codex 방출 줄도 팬아웃 예외다

- 상태: 확정 (2026-09-26, 근거: 구현 P0–P8 착지 + TRD 9판 §10-6(행마다 결정 주체) + 구현 리뷰(`/review code deep` — P2 · P3 · P8) + M15 · M16 실측) · 부분 폐기 by ADR-0235 (결정 1 빚 판정, 결정 2의 항목 처분, 결정 4 넘기기 기본값)
- 관련: Amends ADR-0231 (결정 2 재론 조건, 결정 7 빚 판정, 결정 8 계기 목록, 결정 9 codex 방출 줄 팬아웃, 결정 10 claude 받음 판정, 영향의 착지 상태와 명부 소유) · Amends ADR-0006 (codex 방출 줄 order 를 쥔 채 하는 팬아웃 예외) · TRD `docs/process/S21-codex-backend/trd-mid-turn-input-queue.md`(§5-5 · §5-9 · §10-6) · PRD `docs/process/S21-codex-backend/prd-mid-turn-input-queue.md`(§3-1 · R8) · `docs/research/mid-turn-m15-measurements-2026-09-26.md` · `docs/research/mid-turn-m16-measurements-2026-09-26.md` · `docs/research/mid-turn-phase0b-measurements-2026-09-26.md`(M9 · M10) · 관련 ADR-0127(커널 분류기의 「그 밖의 끝」) · ADR-0233(같은 착지의 세션 id 영속 개정) · step-log S21 · Amended by ADR-0235 (결정 1 빚 판정, 결정 2의 항목 처분, 결정 4 넘기기 기본값)

## 맥락
ADR-0231 은 코드 전에 박혔다(그 ADR 「영향」 — 「착지 전 — 코드 없음」). 구현(P0–P8)은 TRD 8판 서술과 여러 자리에서 다르게 착지했고, TRD 9판 §10-6 이 그 차이를 행마다 결정 주체와 함께 색인한다 — 그 표의 「8판 서술」 칸이 새 ADR 의 거부한 대안이 되도록 적혀 있다.

그 행의 대부분은 TRD 층의 구현 모양이라 TRD 에 남는다. 여기는 **ADR-0231 · ADR-0006 의 결정 문장을 바꾸거나, 그 문장만 읽은 다음 세션이 다시 열 자리**만 옮긴다. codex JSON 세션 id 영속 시점(같은 착지의 사용자 결정)은 ADR-0233 이 맡는다.

## 결정
표기는 ADR-0231 범례를 따른다 — **[사용자]** = 사용자가 답한 결정 · **[위임]** = 사용자가 맡긴 판정(TRD §10-1 23)과 리뷰 판정 — TRD §10-6 의 「오케스트레이터」·「리뷰」 · **[고름]** = 구현이 스스로 고른 내부 구현 — TRD §10-6 의 「구현」·「계획」.

1. ★**후속 턴 빚은 `turn/steer` 로 든 글만 진다**★(ADR-0231 결정 7 을 좁힌다) [위임 — 오케스트레이터 2026-09-26]. 빚 = 열린 턴에 steer 로 넘긴 글의 받음(되울림) 뒤로 그 턴의 샘플링 시작(출력 항목의 `item/started`)이 없이 그 턴이 `completed` 로 끝났다(`State::unanswered` → `follow_up_owed`). `turn/start` 가 실은 글(Direct · `Held` 머리 · 거절 뒤 다시 보낸 글)은 빚을 지지 않는다 — 그 요청이 연 턴은 자기 입력을 먼저 샘플링하고, 「기록만」(M10)은 steer 의 현상이다. 가름은 항목 단계의 칸 `Stage::InFlight.steered` 이고 steer 를 꺼낼 때만 참이 된다. ADR-0231 결정 7 의 문장(「기록만 되고 답을 못 받으면」) · 예외 셋 · 푸는 때(내는 순간 — 우편 턴은 안 푼다) · 빚 쓴 턴이 이상하게 끝나면 다시 내지 않음은 그대로다.
2. ★**뜻 모를 codex 턴 끝(`TurnClose::Unknown`)은 오류 뒤 멈춤을 세우지도 풀지도 않고 빚도 없다**★(ADR-0231 결정 8 의 「계기가 아니다」 목록에 더한다) [위임 — P2 리뷰]. `turn/completed` 의 status 가 없거나 모르는 값(`inProgress` 포함)이면 항목 처분은 `completed` 와 같고(에코 없는 목록 항목은 수락 모름 · 응답 기다리는 steer 가 있으면 정산) 멈춤은 그대로 · 빚 없음 · 경고 계수(`unpaid`)에도 안 든다. 사유: 커널 분류기가 같은 줄을 「그 밖의 끝」(`TurnEndKind::Other`)으로 접어 `last_end_failed` 를 그대로 둔다(`turn.rs`) — 통로만 풀면 두 층의 멈춤이 갈린다. 그래서 결정 8 의 「세우지도 풀지도 않는」 끝 = 끊긴 끝 · steer 실패 · 뜻 모를 끝.
3. **claude 받음 가능은 알아볼 수 있는 수명주기 줄로만 연다**(ADR-0231 결정 10 「옛 벤더 버전 = 오늘 동작」을 모르는 벤더 표면까지 넓힌다) [위임 — P3 리뷰]. `command_lifecycle` 줄이 비지 않은 `command_uuid` 와 아는 상태어를 함께 가질 때만 `Available`(`is_recognisable_lifecycle` — 상태어 명단 정본 = 그 옆 상수). type 만 맞는 줄로는 열지 않는다 — 그대로 두면 능력 목록에 `msg_lifecycle_v1` 이 없는 init 이 `Unavailable` 로 보내 오늘 경로로 떨어진다.
4. **codex 운영 기본 넘기기 = `AtEarliestBoundary` · 답 구간도 붙든다**(ADR-0231 결정 2 의 착지 결과) [고름 — 결정 2 가 정한 착지 순서대로]. M15 녹(도구 끝 반응 합 2275 µs < 창 최소 6 ms · 답 끝 221 µs < 9.4 ms) · M16 녹 → `HAND_OVER_POLICY = AtEarliestBoundary` · `ANSWER_SEGMENT_HOLDS = true`(둘 다 codex backend 의 상수 — 사용자 설정 표면이 아니다). `Immediate` 는 변형으로 남아 통로를 짓는 seam(`with_hand_over`)으로만 끼운다 — 시험이 두 정책의 갈림을 재고, 측정이 빨간 구간의 값이다(ADR-0231 「거부한 대안」의 「`Immediate` 는 착지 첫 걸음과 측정이 빨간 구간의 값으로만 남는다」 그대로). seam 없이 지은 통로도 운영 정책이다 — 시험대가 운영 값을 잰다.
   - ★**재론 트리거 = 벤더가 창을 좁힌다(재측정 빨강).**★ 그때 할 일은 여기서 새로 정하지 않는다 — ADR-0231 결정 2 · PRD R8 이 그대로 선다: 답 끝 창만 좁아지면 `ANSWER_SEGMENT_HOLDS = false`(답 구간만 곧바로 — 사용자 질문이 아니다, 그 상수 doc) · 도구 끝 창이 좁아지면 코드 전에 사용자에게 묻는다(「붙들기 대 다른 길」). ★`HAND_OVER_POLICY` doc 의 옛 문장 「벤더가 그 창을 좁히면 `Immediate` 로 되돌린다」는 그 물음을 건너뛰는 위임 서술이라 이 두 문장과 어긋났다★ — 구현 계획의 「M15 빨강 → 묻지 않고 `Immediate`」가 같은 어긋남으로 TRD §10-6 에 적혔고, M15 가 녹이라 서지 않았다. 그 주석은 이 결정과 함께 재론 조건으로 고쳤다(아래 「이 결정과 함께 맞춘 표면」).
   - **붙듦이 기본이 되며 결정 8 의 「codex 남은 항목」이 흔해진다** — 오류로 끝난 턴 앞에 붙든 글은 사용자가 다음 글을 칠 때까지 나가지 않는다. P8 리뷰가 이것을 결함으로 올렸다가 설계대로로 닫았다(TRD §10-1 9 — 사용자 결정 Q8: 오류로 끝난 턴 뒤 codex 는 저절로 보내지 않고, 한도가 풀려도 그렇다).
5. ★**codex 방출 줄(`Announcer.order`)을 쥔 채 팬아웃한다 — ADR-0006 「lock 미보유 send」의 둘째 예외다**★(ADR-0231 결정 9 · ADR-0006 개정) [고름 — 구현]. codex 목록 항목·합성 말풍선을 내는 쪽은 둘이다(입력 스레드의 `send_turn` · 라이터의 하한 판정 `settle_floor` — 화신당 한 번). 방출은 상태 락 밖이어야 하므로(ADR-0006) `announce` 가 `order` 로 둘을 줄 세우고, 줄을 쥔 쪽이 방출 전 항목 **전부**를 도착 순서대로 낸다 — 그래서 `OutputSink::send` 가 `order` 안에서 돈다. 리더와 `withdraw` 는 `order` 를 잡지 않는다(방출을 마친 항목만 처분하고, 방출 전 항목에는 죽음 표시만 단다). codex 경로에는 세션 입력 자물쇠(`input_order`)가 없다 — 그것은 claude(`SessionClassified`) 경로의 것이고, codex 에서 같은 줄 세우기를 `order` 가 한다.
   - **착지한 락 순서 전부**: 세션(claude) = `input_order → replay → 명부` · `replay → 대기 목록 표`(둘 다 replay 아래의 잎 — 명부 가드를 놓은 뒤 표에 적는다, `OutputCore::record_list_event`) — ADR-0231 결정 9 의 문장 그대로다. codex = `order → 상태`(짧게 — emit 전에 놓는다) · `order → replay → 명부` · `replay → 대기 목록 표` · 통로 안의 옛 순서 `상태 → 대기표`(ADR-0006 · 통로 파일 헤더) 그대로. **팬아웃 예외는 둘** — 세션 `input_order` 안(ADR-0231 결정 9) · codex `order` 안(이 결정). 둘 다 `OutputSink::send` 의 논블록 계약에 기댄다.
6. **대기 입력 명부는 코어가 쥔다 — 세션은 코어를 거친다**(ADR-0231 「영향」의 「소유권 분할의 session += 대기 입력 명부·입력 자물쇠」를 고친다) [고름 — 구현]. 코어가 명부 `Arc`(`QueuedInputs`)와 대기 목록 표 `Arc`(`InputsPendingTable` — 표 자체는 `AgentManager` 가 하나 소유한다)를 `with_queued(QueuedWiring)` 로 받고, 세션의 명부 접근자(`AgentSession::queued_inputs`)는 코어의 그 `Arc` 를 꺼내 준다. 세션 쪽에 남는 것은 입력 자물쇠(`input_order`)다.
7. **ADR-0231 의 착지 상태** — 착지했다: P0–P8, 코드 push 끝 = P2 `3014247` · P8 `37ecc5e`(CI 녹). 그 ADR 「영향」의 「착지 전 — 코드 없음(2026-09-26)」은 작성 시점의 서술이다.

## 거부한 대안
- **넘긴 글 전부에 빚 판정**(TRD 8판 §5-5 「답 없는 항목」 · 정산 3 · §5-9 의 글자 그대로) [위임] — `turn/start` 가 실은 글에도 걸려, 그 턴이 에코 뒤 출력 항목 없이 끝나면 운영에서 빈 `turn/start` 가 나가고 모델이 앞 답을 되풀이한다(M9 대조군 — 답 못 받은 것이 없는데 빈 턴을 열면 직전 답을 되풀이한다).
- **steer 여부를 턴 id 로 가른다** [고름] — `turn/start` 가 실은 글도 응답이 오면 턴 id 를 적어 가를 수 없다(`Stage::InFlight` doc) → 항목 칸 `steered`.
- **뜻 모를 status 를 `completed` 로 읽는다**(첫 착지) [위임 — P2 리뷰] — 통로의 멈춤을 풀고 빚까지 세울 수 있는데 커널은 같은 끝을 「그 밖」으로 접어 `last_end_failed` 를 쥔다 → 두 층의 멈춤이 갈린다.
- **첫 `command_lifecycle` 줄을 보면 `Available`**(TRD 8판 §5-4 · §7-1) [위임 — P3 리뷰] — 내부 표면이라 벤더가 키·상태어를 바꾸면 열어 둔 명부 행이 영영 안 닫힌다(상한 없음 — ADR-0231 결정 7 · N8).
- **codex 방출을 상태 락 안에서** [고름] — ADR-0006(상태 락 보유 중 외부 호출 금지).
- **방출 줄 없이 두 방출자가 각자 락 밖에서 낸다** [고름] — 둘이 락을 놓은 사이 서로의 방출을 앞질러 목록이 친 순서와 어긋난다(`Announcer` doc).
- **명부 `Arc` 를 세션과 코어에 따로 꽂는다**(TRD 8판 §5-3 「명부는 세션 소유 · 코어는 관찰자」) [고름] — 두 곳에 따로 꽂으면 세션이 읽는 명부와 코어가 먹이는 명부가 갈릴 수 있고, 그 어긋남은 컴파일도 오류도 없이 빈 목록으로만 보인다(`AgentSession::queued_inputs` doc).

## 근거
- **결정 주체** — TRD 9판 §10-6 이 행마다 적는다(이 ADR 의 태그는 그 표를 옮긴 것이다). TRD 확정과 구현은 사용자가 맡겼다(TRD §10-1 23 — 「구현 너가 알아서하고 후에 최종 보고하는걸로」) · [위임] 항목은 최종 보고에 싣는다(TRD §10-6 머리).
- **실측** — M15(release 데몬 · 네 로그 합산 · 도구 끝 합 2275 µs · 답 끝 221 µs — 확실) · M16(답 구간 창 최소 9.4 ms) · M9 · M10(Phase 0b). ★M15 가 재지 않은 것★: 도구 끝 신호가 직접 깨운 steer 가 한 건도 없었다 — 붙든 글이 도구 끝에 넘어가는 경로를 끝에서 끝까지 잰 값은 없다(그 보고서 §6 · 확실).
- **리뷰** — P2 deep: codex FIX(에코가 steer 응답보다 먼저 올 때) · doc-aware F2(뜻 모를 status) → 수정 → 재검 PASS · 동시성 렌즈 PASS(락 고리 없음). P3 deep: 알아볼 수 있는 줄 게이트 채택 → PASS. P8 deep: doc-aware PASS · 동시성 PASS · codex 의 FIX(오류 턴 앞에 붙든 글이 멈춘다) = 설계대로(결정 4 둘째 항) → P8 게이트 통과(`37ecc5e` · CI 녹).
- ★**검증 상태(2026-09-26)**★ — P2 로컬 QA full = PARTIAL(결함 없음). `Immediate` 아래라 붙든 행이 짧아 **못 일으킨 것**: 붙든 행의 UI ✕ · 빈 후속 `turn/start`(다섯 번 시도) · 늦은 에코·수락 모름. 빈 후속 턴은 codex 0.156.1 에서만 재었다(하한 0.140 — TRD §5-9). P8 빌드의 최종 `/qa full` 은 남았다.

## 영향 / 불변식
- **코드 앵커** — `is_recognisable_lifecycle` · `HAND_OVER_POLICY` · `State::unanswered` · `TurnClose` · `Announcer` · `announce` 는 `// ADR-0234` 를 함께 단다. 나머지(`Stage::InFlight` · `OutputSink::send` · `with_queued` · `AgentSession::queued_inputs`)는 `// ADR-0231` 만 가리킨다 — 찾는 법 = `rg "ADR-023[14]" crates/engram-dashboard-agent/src/`.
- **어기면 깨지는 것**: `turn/start` 가 실은 글에 빚을 지우면 빈 `turn/start` 가 앞 답을 되풀이한다 · 뜻 모를 끝에서 통로 멈춤을 세우거나 풀면 커널과 갈린다 · 수명주기 게이트를 첫 줄로 넓히면 벤더 드리프트에 행이 영영 안 닫힌다 · `OutputSink::send` 가 막히면 codex 입력 스레드와 라이터의 하한 판정이 `order` 에서 선다(세션 `input_order` 안 팬아웃과 같은 부류).
- **이 결정과 함께 맞춘 표면**: `OutputSink::send` 의 trait doc(예외 둘) · `HAND_OVER_POLICY` doc 과 `HandOverPolicy::Immediate` doc(되돌림 문장 → 재론 조건, 결정 4) · CLAUDE.md 「핵심 불변식」의 락 순서·팬아웃 예외·소유권 분할 줄. **남긴 것**: `OutputCore::record_list_event` 머리 주석은 락 순서를 `replay → 명부 → 대기 목록 표` 한 줄로 적는다(틀린 순서는 아니다 — 코드는 명부 가드를 놓은 뒤 표에 적어 둘이 겹쳐 잡히지 않는다).
- **받아들인 잔여**(TRD §5-9): 알아볼 수 있는 줄 게이트는 드리프트가 능력 없는 init 과 함께 올 때만 돕는다 — 능력은 그대로 두고 키만 바꾸면 행이 안 닫힌다(상한 없음).
