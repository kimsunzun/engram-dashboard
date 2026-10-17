# TRD — 슬롯 도킹 (탭을 끌어 다른 창 레이아웃에 붙이기)

> **5.7판 — 사용자 결정 Q20(자기 탭바 안 = 순서 이동 · 2026-10-17) · 실 마우스 실측 결함 셋**(PRD 4.6 · R23 · D15–D21): 출처 창의 **순서 이동 구역**(탭바 상자를 위아래로 `REORDER_SLACK_PX` = 15 CSS px 넓힌 사각형)에서는 떼어진 모습 · 가이드 · 머묾 전환 없이 끄는 머리가 커서를 따라 가로로 움직이고 이웃이 가운데 규칙으로 비켜나며, 놓으면 보이던 자리로 순서만 바뀐다(`DockTo::TabBar` 같은 창 — 모델은 놓을 때 한 번). 구역을 벗어나면 그때부터 5.6 의 도킹 그대로(떼어진 모습 · 탭 하나 팝아웃 숨기기 · 가이드 · R21 · R22). ★보이는 자리 = 셸이 적용할 자리★ — 구역 · 자리 규칙의 정본은 셸 `hit` 하나(`Picked::Reorder` · 골든)이고, 모드는 출처 웹뷰가 자기 `pick` 결과로 정하며, 셸은 놓기에서 같은 영역 · 같은 점으로 다시 판정해 같은 `index` 일 때만 적용한다(§1-3 「놓기 판정」 4c · §2-5). 기하는 `dragstart` 안에서 잰 탭바(`strip`)를 세션 내내 얼려 쓴다. 결함 고침 — ① 보고 없이 끝난 끌기(멈춘 채 떼기 16/16 새 창 · 짧은 톡 40–90 % 새 창)와 ③ 영역 전에 낸 보고(`rev` 0 — 이웃 창 6/20 무변경)는 「빈 보고」로 묶어 셸이 끝 순간 커서를 직접 판정한다(§1-3 「커서 판정」 — 원래 창 · 그 끌기를 본 창만) ② 엔진 취소 거르기를 0 ms 시한 경주(4/19 샘)에서 사건 시각(`timeStamp`) 기준으로 바꾸고 이미 연 세션은 `canceled` 끝으로 무변경. wire 더하기 넷(`dock_drag_start.strip` · `dock_drag_end.canceled` · `DockRegions.reorder` · `Picked::Reorder`) — 앞 셋은 선택 칸이라 셸이 먼저 들어가도 7 단계 웹뷰와 선다(§10 9a–9c). ★사용자 확인 둘(§11-1b · PRD §5-3) = 닫힘(2026-10-17)★ — ① 보고 없는 끝 · 빈 보고는 뗀 순간 커서의 판정을 그대로 적용한다(사용자 결정 Q21 — 이음새 · 끼울 자리도 적용 · 판정할 창 = W, 없으면 출처 그대로 · `Verdict` 팔은 잠정이 아니라 확정) — 결함 ① ③ 이 이것으로 닫힌다 · 가이드를 보인 뒤 판정이 다르면 R13c 그대로 무변경 ② 원래 창 탭바의 머묾 전환이 없어짐 = 받아들임(Q20 에서 따라 나온 것 · AC14 · 다른 창 탭바 머묾은 그대로). 이웃 창 멈춘 채 떼기 4/4 새 창은 하네스(커서를 `SendInput` 이동 한 번으로 옮김) 탓으로 의심 — S7 ⑪ 에서 끊김 없는 이동으로 다시 잰다. **리뷰 1 라운드 반영**(2026-10-17 · 적대 리뷰 둘 = FIX · 사용자 지시 하나): ① 모드 신호 = 출처 웹뷰 자기 `dragover`(한 번 들어오면 멈춰도 ~15/s) · 첫 `dragover` 전은 `dragstart` 점(구성상 구역 안) = 순서 이동 — 구역 밖으로 빠르게 빼고 멈춘 끌기의 모습 늦음은 받아들이고 S7 ⑫ 로 잰다(§1-4 「모드」) ② 커서 판정도 캔버스를 거른다 — 보고가 있으면 그 `displayed` · 출처 보고가 없으면 출처 캔버스 = 모델 활성(끄는 탭이면 칸 가이드 없음 → 새 창) · 웹뷰 판정 순서 = `pick` → 모드 → `displayed` → 캔버스 거름(§1-3 「커서 판정」 · §1-4) ③ 미룬 숨기기의 증거 ① 은 그린 보고(지금 `rev` · 막히지 않음)만 · ② 출처 `Left` 는 `LEAVE_GRACE_MS` 시한 뒤(끝 신호가 거둔다 — 구역 안 Esc 의 떠남) · 웹뷰의 도킹 모습 전환도 같은 미룸 · 막힌 영역 = 모드 유지(§1-3 「도킹 모드 증거」) ④ 「빈 보고」 = `rev` 0 · W 의 지금 영역 없음으로 좁힘 ⑤ ★넘친 탭바 = 끝에 대고 있으면 그쪽으로 계속 스크롤(사용자 지시 2026-10-17 · PRD D20 고침)★ — 엔진 끌기 자동 스크롤에 맡기고(안 걸리면 우리 끝 띠 시한) 자리 · 끄는 머리는 내용 좌표(뷰포트 x + 탭바 `scrollLeft`) · 얼린 머리도 내용 좌표 · `Over` · `dock_drop` 이 `scroll` 을 싣는다(선택 칸 · §2-5) ⑥ 낡은 줄 정리(§0 · §6-2 D4 · §9 6b 행 · 9b 행의 유실) · 순서 이동의 LLM 예외를 PRD D22 로 올림 ⑦ 엔진 취소 사건 시각의 기준(누른 시각인가)을 S7 ⑦ 에서 잼 — 못 걸러도 무해. **리뷰 2 라운드 반영**(2026-10-17 · 적대 리뷰 둘 = FIX · 고침은 전부 기본값 · 메인 판단 — §7 #55–#58): ① ★끝 신호가 그린 판정을 싣는다★ — `dock_drag_end.last`(선택 칸 `DrawnReport` — 출처 창이 끝에 아직 위일 때 · 그 창 사건 시각으로 가름) · `dock_drop` 의 `point` · `scroll` 도 그린 판정 것 · 셸은 `last` 를 받은 마지막 `Over` 대신 쓴다 — 보고 줄(`overChain`)과 끝 invoke 가 따로라 가장자리 스크롤 중 Esc 가 옛 자리를 적용하던 틈(§1-3 「끝 신호」 · §1-4 「출처」 · 「모드」 넘친 탭바 ③) ② 9b 가 생성물을 다시 굽고 `dockDrag.ts` 보고 생성 두 곳에 `scroll: null` — ts-rs 의 `Option` 은 키 필수라 「9b 동안 프론트 무변경」이 아니다 · 9b ↔ 9c 병렬 불가(§1-5 「선택 칸」 · §10) ③ ★모든 창 탭바를 내용 좌표로★ — 크롬 머리 · 끼울 x = 스크롤 0 자리 + 잰 때의 `scroll`(`DockChrome.scroll` · `TabBarRegion.scroll`) · R21 도 보고의 `scroll` 로 판정(`pick` 의 `scroll` = `Option`) · 막대 = 끼울 x − 지금 스크롤 · 탭바 `scroll` 은 크롬 계기가 아니라 다시 판정 계기 — 5.7 첫 판의 「다른 창은 크롬 보고가 따라가 안전(무변경)」은 거짓이었다(뒤 디바운스가 연속 스크롤 동안 안 터져 옛 기하로 잘못 붙음 · §1-4 ④ · §11-2 #31 · S7 ⑬ ⓒ 합격 = 막대가 보이던 틈) ④ 숨기기 · 보이기 · 그 뒤 포커스 = 메인 스레드에서 장부의 지금 `source_hidden` 으로 맞추기 · 세션 번호 `gen`(지난 세션 시한은 버림 · 지난 세션 할 일은 지금 값으로만) — 시한의 `Hide` 와 끝의 `Show` 가 다른 스레드라 보이기 → 숨기기로 닿아 팝아웃이 숨은 채 남던 틈(§1-3 「명령의 실행 스레드」) ⑤ 끝 신호 · 시한 콜백의 `now` 를 세계 읽기 전에 · 시한이 앞지른 끝의 출처 = 아직 위(셸 유예 또는 `last`) · 숨긴 출처 `sent` 는 끝까지 · 4c 로 끝나면 되보이기(§1-3 「도킹 모드 증거」 · §11-2 #34). **리뷰 3 라운드 반영**(2026-10-17 · 적대 리뷰 둘 = FIX · 고침은 전부 기본값 · 메인 판단 — §7 #59 · #60): ① ★`last` 는 W 를 고르지 않는다★ — W 는 5.6 그대로 도착 순번(끝 순간 커서 · 사각형 검사 그대로)으로 고르고, 고른 W 가 출처일 때만 `last` 가 셸이 받은 출처 마지막 `Over` 를 갈아 끼운다. 리뷰 2 판은 `last` 가 있으면 W = 출처라, 출처를 떠나 다른 창 W2 의 가이드를 본 뒤 출처 시계로 떠남 100 ms 안에 Esc 를 누르면 커서가 출처 밖 → R22 새 창이 됐다(PRD R12 · AC19 어김 · 5.6 은 도착 순번으로 맞았다). `last` 에 떠남 → `dragend` 사건 시각 간격 `left_before_end_ms` 를 더해, 그 간격이 `CANCEL_LEAVE_MS`(15 · 첫 값) 안이면 엔진이 끌기를 접으며 낸 떠남(출처 위 Esc)으로 보고 사각형 검사 없이 `last` 자리로 간다 · 다른 창에서 출처로 돌아와 그 `Over` 가 오는 중인 끝 = 5.6 그대로(§11-2 #36 · 미결) ② ★시한이 앞지른 끝의 출처도 사각형 검사★ — 시작에 쥔 출처 사각형(`source_rect` · 출처가 든 `Rects` 마다 갈아 끼움 · 숨기기는 창을 옮기지 않는다)으로. 리뷰 2 판의 「사각형 검사 없이」는 바탕화면에 빠르게 던진 끌기를 시한 흔들림에 따라 4c 무변경 또는 R22 로 갈랐고, 그것을 5.6 남은 틈 #1 과 같다고 적은 문장도 틀렸다(#1 은 다른 앱 창이 우리 창을 덮은 자리 · 5.6 은 바탕화면에서 사각형 검사로 R22)(§1-3 「도킹 모드 증거」 · 「끝 신호」 · §11-2 #1 · #34). **리뷰 4 · 5 · 6 라운드 반영(5 = 붙들기는 끝 커서를 품은 창의 줄만 기다린다 · 6 = 겹친 창이면 시한까지)**(2026-10-17 · 적대 리뷰 둘 = PASS 하나 · FIX 하나 → 메인이 FIX 채택(보수) · 고침은 전부 기본값 · 메인 판단 — §7 #61 · #62): ① ★오는 중 `Over` 붙들기(`AwaitOver` · `OVER_WAIT_MS` = 50)★ — 다른 창이 가이드를 그렸는데 그 `Over` 가 오는 중일 때 출처 끝이 먼저 닿으면 도착 순번이 출처를 고르고, 출처 `last` 는 실제 떠남(간격 > `CANCEL_LEAVE_MS`)이라 사각형 검사 · 커서는 그 창 안 = 출처 밖 → 보인 가이드 대신 새 창이었다(PRD R12 · AC19 어김). 그 조건(`none` 끝 · 간격 > `CANCEL_LEAVE_MS` · 도착 순번 W 가 출처 또는 없음)이면 끝을 붙들고, 다른 창 `Over` 가 닿으면 그 창 `Left`(같은 줄 — 앞 `Over` 가 다 닿았다는 표시) 또는 시한에서 그 창을 W 로, 아무것도 안 닿으면 붙들기 없는 판정 그대로 · 판정은 끝 때 읽은 커서 · 사각형으로 · 붙든 동안 세션은 살아 있고 늦은 끝 신호 · `pointer_seen` 은 버리며 새 시작은 거절하고 붙든 끝으로 곧바로 푼다 · 값 = invoke 왕복 실측 최대 ~50 ms(§1-3 「오는 중 `Over` 붙들기」) ② ★W 없음 + `last` = 출처★ — 출처 첫 그린 `Over` 가 오는 중이라 W 가 빈 끝에서 `last` 를 버리고 시작 스크롤로 커서 판정해, 가장자리 스크롤 뒤면 그린 것과 다른 순서 이동 자리가 적용됐다(리뷰 3 판 회귀). W 없음이고 `last` 가 떠남 없음 · 간격 ≤ `CANCEL_LEAVE_MS` 면 W = 출처 · 보고 = `last`(§1-3 「마지막 점에 놓기」) ③ §11-2 #36 은 닫지 않고 좁혔다 — 반대 방향은 ① 이, 아무 `Over` 도 안 닿은 끝은 ② 가 닫고, W2 `Over` 는 닿고 돌아온 출처 `Over` 만 오는 중인 끝은 그대로 열려 있다(닫을 후보 = ① 을 반대로 — 넣지 않는다 · 기본값 · 메인 판단: 끝 직전 16ms 안에 출처로 되돌아온 경우뿐이고 결과는 5.6 과 같다 · S1 ① 실측 뒤 다시 본다).
>
> **5.6판 — 사용자 결정 Q19 반영**(2026-10-16): R22 재사용(탭 하나뿐인 팝아웃을 그 창째 옮김)은 다시 확인 · 되돌리기 없이 「A 판정 → 창 옮기기 → 표식 내리기」 — 놓은 뒤 그 사이 변화는 결과 그대로(PRD 4.5 · R13b · R13c 예외). 5.2–5.5 의 재확인(C′) · 되돌리기(`restore_window` · `Moved` · `undo_reuse_move` · `reset_window_place` · 되돌리기 기록 · 시험)를 걷고 거부한 대안으로 남김(§7 #39 · ADR-A ⑫). Create 길의 phase C 재판정 · 연 창 닫기는 그대로. §11-2 에 Create 길 거두기 실패(#25 · 종료 중뿐 · 받아들임)를 더함. 5.4 의 「재사용은 보통 자리가 있을 때만」 조건도 근거(되돌리기)와 함께 거둠 — 탭 하나 팝아웃은 늘 재사용(메인 결정 · #26 해결 · 끝 줄 `reuse_skipped` 칸 걷음). 코드 리뷰 PASS 뒤 문구 둘 — `open_at` 의 자리 입히기 = 검사 세터 `move_steps`(`place` 아님) · 창 포트 설명에 숨은 최대화 창의 깜빡임 뒤 숨음을 「보이는가」와 맞춤 · 4 단계 상태 = 리뷰 PASS · QA 대기. 6a 구현 · 리뷰 PASS 뒤 동기화 — `DockRegions` 의 `view_id` 는 측정 전에도 모델 값(측정 없음 = 캔버스 0×0 · 원점 (0, 0) · 칸 · 바깥은 크롬 · 캔버스 · 보이는 탭이 다 있을 때만) · `blocked` 몸은 비고 기대값은 모델 값 · `regions` 의 `visible_tab` · `bumps` · `Expect{layout, active, tabs}` · `WindowSnap` 을 §2-1 에 · 6b 는 `visible_tab` 을 옮기지 않는다 · invoke 인자 철자(Tauri v2 camelCase — 6c 는 구조체 인자나 `rename_all`) · `DragEffect` 는 6c · 골든 맨 위 `source_view` · 6a 상태 = 리뷰 PASS · QA 대기. 그 뒤 `regions_for` 의 debug 전용 `debug_assert_eq!`(`visible` = `visible_tab`) · 골든 8 영역 · 83 사례(`offset` 영역 · 위 띠 경계).
>
> **5.5판 — 4 단계 구현 반영 문구 정정**(2026-10-16 · 코드 리뷰가 낸 TRD 어긋남): 재사용 되돌리기의 표식(출처 탭 닫힘이면 View 와 함께 내려감) · `settled` 는 알림을 낸 쓰기만(창 통째 닫기 · 만들기 · 재사용 성공 · 무변경은 없음 — 그 틈) · 자리 검증이 새 창(기본 자리로)과 재사용(제자리)에서 다름 · `open_at` 에 최대화 풀기 없음 · 최대화 출처의 B′ = 「깜빡였다 다시 숨음」(5.3 의 「보이고 포커스」 정정) · `move_window` 가 `Moved{Applied, StayedPut}` 를 돌려주고 옮긴 것이 없으면(`StayedPut`) 재확인 실패에도 되돌리지 않음 · 되돌린 뒤 모델 자리 다시 적기(`reset_window_place`) · 사라진 창 · 어느 모니터에도 안 걸치는 보통 자리의 되돌리기는 위치 실패로 답함 · 재사용의 걸침 판정 크기 = `at.size` · 모델 보통 크기 · 안쪽 크기 · 기본 · 새 창 자리 입히기 실패 = 창 거둠 · 끝 줄 `moved = false` · `changed = false` · 워커 스레드의 세터는 기다리지 않음 · 6 단계 의무 둘 · 창 포트 구현자 일곱 · 「4 단계 고침」의 줄 포인터를 심볼로.
>
> **5.4판 — 리뷰 4라운드 반영**(2026-10-16): R22 재사용은 출처 창의 모델에 보통 자리가 적혀 있을 때만 — 없으면 새 창(Create) 길로(되돌릴 자리가 없어 반쯤 옮긴 창이 남는 것을 막는다 · `plan_new_window` 고침 = 4 단계).
>
> **5.3판 — 리뷰 3라운드 반영**(2026-10-16 · doc-aware PASS · blind FIX — 둘 다 R22 재사용 되돌리기): 되돌릴 자리를 게터가 아니라 phase A 에서 락 안에 복사한 모델 자리(보통 자리 · 최대화)로 · 창 포트 세터가 결과를 돌려주고 옮기기는 첫 실패에서 멈춰 되돌리며 되돌리기는 실패해도 이어 감 · 최대화였던 출처는 최대화 풀기(tao `SW_RESTORE`)가 창을 보이고 포커스를 가져가는 것을 받아들인 경우로 적고 S1 ⑤ 에서 잼 · 숨긴 채 푸는 platform 함수는 미결.
>
> **5.2판 — 리뷰 2라운드 반영**(2026-10-16 · 1라운드 지적은 둘 다 해소 판정 · 새 FIX): 탭바 놓기는 이음새와 따로 대조(탭바 몫 `bar_rev` · 보이던 탭은 이음새만 — 탭 전환 중에도 탭바 놓기가 선다) · R22 재사용(탭 하나 팝아웃 옮기기)에 옮긴 뒤 같은 락 안의 재확인과 되돌리기 · `Apply` 는 늘 async 런타임에서(동기 명령 안에서 창을 만들지 않는다) · 붙들다 버린 id 의 늦은 시작은 거절 · 세션 깃발은 세대로 비교해 내림 · 다른 인스턴스 끌기 문구 · 표식이 내려가기 전 틈의 시작 거절을 받아들인 남은 틈으로 적음.
>
> **5.1판 — 리뷰 1라운드 반영**(2026-10-16 · 적대 리뷰 둘 = FIX · 구조는 그대로): 놓기 대조에 대상 창의 보이는 탭을 더함(웹뷰가 보이던 탭 · 모델 기대값에 창 활성 — 4 단계 고침 둘) · 「가이드 없음 = 새 창」을 「보인 것 · 지금 판정 둘 다 없음」으로 좁히고 그 밖의 어긋남 · 대상 사라짐은 무변경 · 영역 · 세션 사건의 순서(세션을 몰라도 우리 MIME 받기 · `drag_id` 별 최대 `rev` · `seq` · 시작보다 먼저 온 끝 신호 붙들기) · 적용 중(`Applying`)의 스냅숏 · 소멸은 적기만 하고 끝은 `Applied` 가 정함 · 스냅숏 계기 = 쓰기의 알림 묶음 끝 한 번 · react-arborist 의 window 끌기 리스너가 탭 끌기를 막는 것을 피함 · 창별 마지막 Over 와 떠남을 따로 적음 · 그 밖 아홉(끝 증거 = 버튼 0 이동 · 적용 중 새 시작 · 커서 못 읽음 · `rev` 올리는 때 · 게터는 락 밖 · 늦은 replay 문구 · 계측 · 세션 깃발 자리 · Esc 삼키기 상한). 3 단계 커밋(`281bd235`) 반영.
>
> **5판 — 끌기 수단을 브라우저 내장 HTML5 끌어 놓기로 바꿨다**(사용자 결정 2026-10-16 · 조사 [slot-docking-drag-mechanism-2026-10-16.md](../../research/slot-docking-drag-mechanism-2026-10-16.md)): 셸 폴링 틱 · OS 함수 · 딱지 창을 걷고, 셸은 영역 계산 · 놓기 판정 · 적용만 한다. 같은 날 결정 넷(끌기 그림 = Windows 기본 · 비정상 끝 = 마지막 점에 놓기 · 메인 마지막 탭 끌기 · LLM 명령 미룸)을 함께 반영했다. 뺀 것 = §11-3.

| 항목 | 값 |
|---|---|
| **상태** | 5.7판 — 사용자 결정 Q20(자기 탭바 안 = 순서 이동) · Q21(보고 없는 끝 = 커서 판정 적용) · 실측 결함 셋 고침 · 사용자 확인 둘 닫힘(§11-1b) · 리뷰 1 라운드 반영(FIX → 고침 · 넘친 탭바 스크롤 = 사용자 지시) · 리뷰 2 라운드 반영(FIX → 고침 · 끝 스냅숏 · 모든 창 탭바 내용 좌표 · 숨기기 맞추기 · 메인 판단) · 리뷰 3 라운드 반영(FIX → 고침 · `last` 는 W 를 안 고름 · 떠남 간격 · 시한이 앞지른 끝도 사각형 검사 · 메인 판단) · 재리뷰 전 · 5.6판 — 사용자 결정 Q19(재사용 되돌리기 없음) · 5.5판 = 4 단계 구현(미커밋)에 맞춘 문구 정정 · 5.4판 = 리뷰 4라운드 반영 · 5.3판 = 리뷰 3라운드 반영 · 5.2판 = 리뷰 2라운드 반영 · 5.1판 = 리뷰 1라운드 반영 · 5판 = 수단 교체로 다시 씀 · 사용자 확인 8건 = 전부 기본값 확정(2026-10-16 · §11-1) · 남은 실측 6건(§8-2) |
| **날짜** | 2026-10-17 |
| **스텝** | S21 |
| **브랜치** | `v0.4.1/feat/slot-docking` |
| **PRD** | [prd.md](prd.md) 4.6판 — R1–R23(R16 · R17 = 다음 작업 · R20 없음) · AC1–AC9 · AC11–AC16 · AC18–AC28 · Q1–Q21 · D4 · D6–D10 · D12–D22 · W1 · W3–W5 · W7–W15 · §5-3(닫힘) |
| **근거** | 수단 조사 · 실측(위 조사 §3 · §4 · 스파이크 기록 `.claude/handoff/attachments/spike-html5dnd-2026-10-16.md`) · [slot-docking-survey-2026-10-15.md](../../research/slot-docking-survey-2026-10-15.md) §5(코드 실태) · §7–§8(관례 · 크기) · (5.7) [slot-docking-chrome-tab-drag-2026-10-17.md](../../research/slot-docking-chrome-tab-drag-2026-10-17.md)(Chrome 탭바 · Lumino `TabBar` · Synnax Console — 탭바 안 순서 이동 · 떼기 문턱) · 실 마우스 실측 `.claude/handoff/attachments/slot-docking-S1-8-S3-S6-real-2026-10-17.md`(§8-1) |
| **줄 포인터 기준** | (5.7) HEAD `d582331a`(6c 까지 커밋) + 7 단계 미커밋 트리 — 5.7 이 새로 가리키는 곳은 심볼로만 적는다(`book.rs` `last_point` · `verdict` · `TabBar.tsx` `onHeadDragEnd` 등). 그 전 판의 줄 = HEAD `281bd235`(3 단계까지 커밋). 심볼 이름이 정본이고 줄은 보조다. 의존 crate 줄은 `~/.cargo/registry` 의 잠긴 판(tauri 2.11.3 · tauri-runtime-wry 2.11.3 · wry 0.55.1) · npm 줄은 `node_modules` 의 설치 판(react-arborist 3.9.0 · react-dnd 14.0.5 · react-dnd-html5-backend 14.1.0) |

## 0. 결론 요약

- **세 층.** ① 레이아웃 연산(셸 `layout` — 1 · 2 단계 끝 · 4 단계 적용 서비스) ② 끌기 장부(셸 `dock` — 순수 장부 + 잎 락 하나 · 스레드 · 틱 없음) ③ 웹뷰(HTML5 끌어 놓기 — 엔진이 커서 추적 · 맨 위 창 · 끌기 그림 · 버튼 · Esc 를 맡는다). OS 층(`platform`)에 새 함수는 없다(§1-6).
- **탭을 끌면 처음엔 그 창 탭바 안의 순서 이동이고, 탭바 구역을 벗어나면 떼어진다 — 둘 다 배치는 놓을 때 한 번만 바뀐다**(PRD R2 · R23 · R11a · 5.7). 순서 이동의 움직임 · 떼어진 모습은 웹뷰의 그리기이고, 끄는 동안 모델을 바꾸는 유일한 것은 R6b 탭 전환(사람 탭 클릭과 같은 쓰기 · 다른 창 탭바에서만)이다.
- **순서 이동(5.7 · 사용자 결정 Q20)** — 출처 창의 순서 이동 구역(탭바 상자 ± 15 CSS px — §2-5)에서는 끄는 머리가 커서를 따라 가로로(머리들 범위 안) 움직이고 이웃은 끄는 머리 가장자리가 그 가운데를 넘으면 비켜난다(Lumino 규칙). 떼어진 모습 · 가이드 · 머묾 전환이 없고 끌기 그림(Windows 기본)은 그대로다. 놓으면 그 자리로 `DockTo::TabBar{출처 창, index}`. 같은 끌기에서 구역을 나갔다 돌아오면 다시 순서 이동이다(PRD D15). 넘친 탭바는 끄는 탭을 끝에 대고 있으면 그쪽으로 계속 스크롤되고, 자리 · 끄는 머리는 내용 좌표(뷰포트 x + 탭바 `scrollLeft`)로 잰다(사용자 지시 2026-10-17 · PRD D20 · §2-5). (리뷰 2) 다른 창 탭바(R21)도 크롬을 내용 좌표로 보고하고 보고의 `scroll` 로 판정해, 끄는 중 밀려도 막대가 보이던 틈에 들어간다(§1-4 「모드」 넘친 탭바 ④).
- **보이는 자리 = 적용하는 자리(5.7)** — 구역 · 자리 규칙은 셸 `hit` 하나이고 웹뷰는 그 거울(`pickTarget` · 골든)로 그린다. 모드(순서 이동 · 도킹)는 출처 웹뷰가 자기 `pick` 결과로 정한다 — 셸에 따로 묻지 않는다(신호 = 그 창 자기 `dragover` · 첫 `dragover` 전은 `dragstart` 점 = 구역 안 · §1-4 「모드」). 셸은 놓기에서 같은 영역(세션 내내 얼린 시작 측정 — §2-4)과 같은 점으로 다시 판정해 같은 `index` 일 때만 적용한다(§1-3 「놓기 판정」 4c). 갈라지면 무변경이다.
- **보고 없는 끝은 셸이 커서로 판정한다(5.7 · 실측 결함)** — OLE 끌기 루프는 다음 마우스 메시지 전에는 대상 창에 `DragEnter` 를 내지 않아, 멈춘 채 떼면 어느 창도 보고하지 못한다. 그 끝과 영역 전에 낸 보고(「빈 보고」)는 셸이 끝 순간 커서를 **그 끌기를 본 창 · 원래 창**의 뷰포트 점으로 옮겨 그 창의 지금 영역으로 판정한다(캔버스 칸은 보고의 `displayed` 가 — 출처 보고가 없으면 모델 활성이 — 영역의 보이는 탭일 때만 · 끄는 탭이 보이던 캔버스면 가이드 없음 · 리뷰 1) — 원래 창 탭바 구역이면 제자리 순서 이동, 칸 가운데면 새 창, 이음새 · 끼울 자리면 그 자리에 적용(사용자 결정 Q21 · §11-1b ①)(§1-3 「커서 판정」). 가이드를 그린 보고와 판정이 다른 놓기는 그대로 무변경이다(R13c).
- **셸이 영역을 미리 계산해 창마다 보낸다.** 웹뷰는 dragover(움직일 때 60/s)마다 점이 어느 사각형인가만 보고 가이드를 그린다 — 셸 왕복 없이 가이드가 커서를 따른다. 보인 것이 바뀔 때만 셸에 알린다.
- **놓기 판정은 셸이 한다.** 놓은 창 · 점 · 보인 것 · 그 창이 보이던 탭을 받아 같은 규칙(`dock::hit` — 웹뷰의 TS 판은 골든 파일로 묶은 거울)으로 다시 판정한다: 보인 것 · 지금 판정이 둘 다 「가이드 없음」 = R22 새 창 · 같은 이음새면 영역이 최신이고 보이던 탭이 같을 때만 · 같은 탭바 자리면 탭바 몫(`bar_rev`)이 최신일 때만 적용(5.2 — 탭 전환 중에도 탭바 놓기가 선다) · 그 밖의 어긋남(대상이 사라짐 포함)은 무변경(M1 · R13c · Q18). 모델도 같은 락 안에서 대상 배치 · 대상 창 활성(이음새) · 탭 목록(탭바)을 다시 본다(5.1).
- **끝 신호 셋(+ 엔진 취소 — 5.7).** 대상 창 `drop`(우리 창 위에서 뗌) · 출처 `dragend` 의 `none`(창 밖에서 뗌 · Esc · 시스템이 끊음) · 끄는 중 어느 창이 본 `pointermove`(끝 신호 유실). `dragstart` 뒤 `ENGINE_CANCEL_MS`(15 ms — 사건 시각) 안의 `none` 끝은 엔진이 접은 끌기로 보고 `canceled` 로 알려 무변경으로 끝낸다(§1-4 「출처」). 뒤 둘은 「마지막으로 알려진 점에 놓기」다(PRD R12 · R13b): 셸 커서가 마지막으로 끌기를 본 우리 창(W) 안이면 W 의 마지막 보고로 판정하고, W 가 그사이 사라졌으면(가이드를 보이던 중이면) 무변경, W 가 없고 커서가 출처 창 안이면 출처의 커서 판정(5.7 — 멈춘 채 떼기 · 짧은 톡), 그 밖은 그 자리에 새 창. 그래서 Esc 와 창 밖 떼기를 키 상태로 가를 필요가 없다(§1-3 「끝 신호」). (리뷰 2) 출처 끝은 그 창이 끝에 아직 위였으면 마지막으로 **그린** 판정(`last`)을 싣고, 셸은 그것을 받은 마지막 `Over` 대신 쓴다 — 보고 줄과 끝 invoke 의 도착 순서가 보장되지 않아 가장자리 스크롤 중 Esc 가 옛 자리를 적용할 수 있었다. 탭 하나 팝아웃의 숨기기 · 보이기는 메인 스레드에서 장부의 지금 값으로 맞춘다(§1-3 「명령의 실행 스레드」).
- **사건 순서는 보장되지 않는다(5.1).** emit · invoke 가 여러 스레드에서 나가므로 웹뷰는 세션을 몰라도 우리 MIME 을 받고 `drag_id` 별 가장 큰 `rev` · `seq` 만 쓰며, 장부는 시작보다 먼저 온 끝 신호를 잠깐 붙들고 적용 중(`Applying`)에는 끝을 `Applied` 하나에 맡긴다(§1-3).
- **★에이전트 트리(react-arborist)가 window 에 단 끌기 리스너가 탭 끌기를 막는다★** — 우리 MIME 사건은 `document` 에서 끊어 window 에 닿지 않게 하고, 셸 세션은 `dragstart` 가 취소되지 않았을 때만 연다(§1-4 「다른 끌기 리스너」 · S1 ⑦).
- **메인 마지막 탭도 끈다.** 떠나면 같은 연산 안에서 메인에 빈 탭 하나 — `close_tab` 의 기존 규칙과 같은 도우미(ADR-0057 ④ · 4 단계).
- **닫히는 창 = 모델 먼저 지우고 묘비**(M2 · 3 단계 `281bd235` — 그대로).
- **LLM 명령은 없다 — 의식적 예외**(PRD Q10). 적용 서비스 `apply::dock_tab` 은 마우스 놓기가 쓰므로 남고, 다음 작업이 그 위에 명령을 얹는다(§3).
- **남은 실측(§8-2 S1–S7)** — S1 끝 신호 순서(+ 트리 슬롯 · 사건 순서 — 5.1 · ⑧ 은 5.7 에서 합격 기준을 고쳐 다시) · S2 끌기 그림 내용 · S3 바탕화면 · 파일 드롭(2026-10-17 합격 — §8-1) · S4 혼합 DPI · S5 콘솔 세션 · S6 제목줄 끌기(2026-10-17 합격) · **S7 순서 이동 · 보고 없는 끝(5.7) · 빠른 빼기 · 넘친 탭바(리뷰 1)**. ★불합격이면 조용히 물러나지 않고 멈춘다★.

## 1. 아키텍처

### 1-1. 층과 흐름

```
웹뷰(출처 창)  탭 머리 draggable · dragstart(동기): setData("application/x-engram-tab", {drag_id, view_id}) · effectAllowed=move
               · setDragImage(끌기 그림, hotspot) · stopPropagation(window 의 react-dnd 에 안 닿게)
               · 탭바 재기(5.7 — 읽기만): strip{bar, heads 전부(끄는 머리 포함 · 화면 순서), grab_x}
               → 디스패치 뒤(setTimeout 0) 취소되지 않았으면 invoke dock_drag_start{drag_id, view_id, hotspot, strip}
               dragend → invoke dock_drag_end{drag_id, effect, canceled, last}
                 (canceled = none 이고 dragstart 사건 시각 + ENGINE_CANCEL_MS 안 — 5.7 · 세션 전이면 IPC 0)
                 (last = 이 창이 끝에 아직 위면 마지막으로 그린 판정 + 떠남 간격 — 리뷰 2 · 3 · 셸이 도착 순번으로 고른 W 가 이 창일 때만 마지막 Over 대신 쓴다)
               순서 이동 모드(자기 pick = Reorder — 5.7): 끄는 머리 translateX · 이웃 비켜남 · 떼어진 모습 없음 · 크롬 보고 건너뜀
웹뷰(모든 창)  report_dock_chrome{canvas_origin, tab_bar, tabs[], scroll} — 평소 보고(측정 · version 불변 · 머리 = 내용 좌표 — 리뷰 2)
               listen "dock:session"(Started · Display · Ended — 떼어진 표시 · Esc 삼키기) · "dock:regions"(자기 창 영역)
                 — drag_id 별 가장 큰 seq · rev 만 · Started 전에 온 것도 받는다
               dragenter · dragover(우리 MIME 이면 세션을 몰라도 받음 · stopPropagation) → pick(점, 영역) → 가이드 그리기
                 · 출처 창은 pick 이 Reorder 면 순서 이동 모드 · 그 밖(문서 밖 떠남은 LEAVE_GRACE_MS 뒤)이면 도킹 모드(떼어진 모습 — 5.7 · 리뷰 1)
                 · 판정 순서 pick → 모드 → 보이던 탭 → 캔버스 거름 · 탭바 자리는 내용 좌표(clientX + scrollLeft — 리뷰 1 · 모든 창 — 리뷰 2)
                 · 탭바 scroll 사건 → 마지막 점으로 다시 판정(크롬 보고 계기가 아니다 — 리뷰 2) · 막대 = 끼울 x − 지금 scrollLeft
                 · shown · rev · 보이던 탭이 바뀌면 invoke dock_drag_over{Over{rev, bar_rev, displayed, shown, point, scroll}}
               dragleave(창 밖으로) → invoke dock_drag_over{Left} · drop → invoke dock_drop{rev, bar_rev, displayed, shown, point, scroll}
               탭 머리 위 머묾 500ms → 기존 탭 전환(사람 클릭과 같은 길 · R6b · 출처 창 구역은 Reorder 라 없음 — Q20)
               세션 중 첫 pointermove(마우스 · 버튼 0) · pointerdown → invoke dock_pointer_seen (끝 신호 유실 대비)
               우리 MIME 이 아닌 끌기(OS 파일) → dropEffect none + drop preventDefault (탐색 막기 · 늘 · 끊지 않는다)
셸 commands/dock.rs ─ invoke 껍데기 · 실행기(emit · 숨기기 · 보이기(메인 스레드 맞추기 — 리뷰 2) · 커서 · 창 사각형 읽기(락 밖 · now 는 읽기 전) · 시한 · set_focus · 적용 호출)
     └ dock::book::Book::step(사건, 스냅숏) → 할 일   ← 순수 · 잎 락 하나
          영역      = dock::regions::regions_for(창 스냅숏)   (hit::valid_bands · outer_strips · tab_insert_x)
          판정      = dock::regions::pick(영역, 점, 스크롤) = (출처 창) hit::reorder_* → hit::locate + hit::choose_seam   (TS 거울과 골든 공유)
          빈 보고 · 보고 없는 끝 = 커서 판정(끝 순간 커서 → 그 창 뷰포트 점 → pick → 캔버스 거름 → 그대로 적용 — 5.7 · Q21 · 리뷰 1)
          놓기      = layout::apply::dock_tab(DockGuard{표식, 기대값 — 배치 · 창 활성 · 탭 목록}, Reveal)
셸 layout (권위 — ADR-0035) ─ dock.rs · ViewManager::{dock, plan_new_window, lease_tab, release_lease, mark_closed} · apply::dock_tab
셸 계기 ─ 쓰기 하나의 알림 묶음 끝(LayoutEvents::settled) · 크롬 · 캔버스 보고 뒤(세션 깃발이 섰을 때만 스냅숏 → 장부) · on_window_event Destroyed
셸 WebviewEnv::finish ─ disable_drag_drop_handler() — 모든 런타임 창(wry 의 드롭 대상을 떼어 WebView2 내장 끌어 놓기를 살린다)
platform ─ 새 함수 없음
```

| 무엇 | 어디 | 상태 · 비고 |
|---|---|---|
| 트리 연산 `docked_tree` · `DockSide` · `DockAnchor` · 새 면적 0 · `incoming_rect` | `src-tauri/src/layout/dock.rs` | 끝(1 단계 `080893c5`) |
| 도킹 모델 `dock` · `plan_new_window` · 표식 · 묘비 · `DockGuard` · `TargetExpect` · `FocusPolicy` · `DockError` · `succession` | `src/layout/manager.rs` | 끝(2 단계 `ca723ed5`) · 4 단계가 메인 마지막 탭 규칙과 `TargetExpect` 의 Seam 칸(대상 창 활성 — 5.1)을 고친다(§1-2) |
| 닫히는 창(M2) — 닫기 요청에서 모델 먼저 + 묘비 · 닫히는 창의 늦은 Subscribe 막기 | `src/commands/popout.rs` `close_requested_popup_window`(:255) · `drop_window_in_model`(:282) · `lib.rs` `CloseRequested`(:302) · `daemon_client/connection.rs` `RequestReplay` 갈래(:1166) | 끝(3 단계 `281bd235`) |
| 적용 서비스 `dock_tab` · 창 포트 `open_at` · `move_window`(★기본 본문 없음★ · 5.3–5.5 의 `restore_window` 는 Q19 로 걷음) | `src/layout/apply.rs` · 구현자 일곱 = `commands/popout.rs` `TauriWindowHost` · `ClosingHost`(3 단계가 더한 시험 가짜) · `commands/layout.rs` `OwnedWindowHost` · 시험 가짜 넷(`tests/layout_apply.rs` · `tests/layout_commands.rs` · `daemon_client/tests.rs` 두 곳) | 4 단계 — 구현(미커밋 · 리뷰 중) |
| `placement::place` · `land` 공개(`pub(crate)`) | `src/state/placement.rs`(:446 · :470) | 4 단계 — 새 창 자리 |
| 판정 `locate` · `choose_seam` · `choose_band` · `valid_bands` · `landing_rect` · `outer_strips` · `tab_insert_x` | `src/dock/hit.rs` | 끝(5 단계 `e73e0289`) |
| 크롬 측정 타입 `DockChrome` · `PointCss` · `RectCss` · `TabHeader` | `src/layout/chrome.rs` | 끝(5 단계) · 6 단계가 `WindowTabs` 칸과 보고를 짓는다 |
| 영역 `regions_for` · 판정 `pick` · wire 타입(`DockRegions` 등 — `GuideView` · `TabBarHint` 를 대신한다) · 골든 | `src/dock/regions.rs`(신설) · `src/dock/types.rs` · `src/dock/pick_golden.json`(신설) | 6 단계 |
| 끌기 장부 | `src/dock/book.rs`(신설 · 순수) | 6 단계 — 지운 4판 `dock/session.rs`(8ms 틱 상태 기계 · 커밋된 적 없음)를 대신한다(§11-3) |
| invoke 여섯 + `report_dock_chrome` · 실행기 | `src/commands/dock.rs`(신설) · `lib.rs` `generate_handler!`(:360) · `on_window_event` `Destroyed`(:334) | 6 단계 |
| 알림 묶음 끝 `LayoutEvents::settled`(신설 · 기본 본문 없음 — 구현자 아홉) | `src/layout/apply.rs` `LayoutEvents`(:88) · 알림 자리 `notify`(:162) · `rename_tab`(:432) · 슬롯 옮기기(:766) · `state/restore.rs` 알림 루프(:818) · `dock_tab` | 4 단계(빈 몸 — 6 단계가 `TauriEvents` 에 계기를 단다) |
| 모델 · 측정 변경 계기 | `commands/layout.rs` `TauriEvents::settled`(:57 — 소유형 `OwnedEvents` 도 이것으로 넘긴다) · `report_window_canvas`(:562) · `report_dock_chrome` | 6 단계 |
| 드롭 처리기 끄기 | `src/webview_env.rs` `WebviewEnv::finish`(:73) | 6 단계 |
| 프론트 | `TabBar.tsx` · `WindowLayout.tsx` · `DockOverlay.tsx` · `dockDrag.ts` · `dockPick.ts` · `dragImage.ts` · `dockChromeReport.ts`(뒤 다섯 신설 · 전부 `src/components/layout/`) | 7 단계(미커밋 — 실 마우스 실측에서 결함 셋 · 5.7 이 9a–9c 로 고친다) |
| (5.7) 순서 이동 규칙 `REORDER_SLACK_PX` · `reorder_zone` · `reorder_left` · `reorder_index` · wire `StripStart` · `ReorderRegion` · `Picked::Reorder` · `regions_for` 의 `reorder` 칸 · `bumps` · 골든 사례 · TS 거울 | `src/dock/hit.rs` · `src/dock/types.rs` · `src/dock/regions.rs` · `src/dock/pick_golden.json` · `src/components/layout/dock/dockPick.ts`(+ `dockDrag.ts` 의 `pickedKey` 팔 하나) | 9a |
| (5.7) 장부 — 순서 이동 놓기(4c) · 빈 보고 · 커서 판정 · 숨기기 미룸(도킹 모드 증거) · 엔진 취소 끝 · 끝 줄 칸 · 실행기 — `strip` · `canceled` 선택 인자 · 뷰포트 점 변환 | `src/dock/book.rs` · `src/commands/dock.rs` | 9b |
| (5.7) 프론트 — 탭바 재기 · 엔진 취소(사건 시각) · 모드 · 순서 이동 모습 · 떼어진 모습은 도킹 모드에서만 · 오버레이 · 크롬 보고 건너뜀 | `TabBar.tsx` · `WindowLayout.tsx` · `dock/dockDrag.ts` · `dock/DockOverlay.tsx` | 9c |

`dock` 모듈을 `layout` 밑에 두지 않는 이유: 장부는 레이아웃 권위가 아니라 그 권위를 부르는 손님이다 — 놓기도 적용 서비스를 부를 뿐이다. 섞으면 「레이아웃 변경은 적용 서비스 하나」가 흐려진다.

### 1-2. ① 레이아웃 연산 (OS 무관 · 셸 `layout`)

#### 트리 연산 — `layout/dock.rs` (1 단계 · 끝)

```rust
pub enum DockSide { Left, Right, Top, Bottom }      // serde rename 없음 — wire 철자 = 이름(types.rs 맞대기 시험)
pub enum DockAnchor { Slot(Uuid), Outer }           // Outer = 대상 탭 레이아웃 전체(루트)
pub const DOCK_NEW_SHARE: f64 = 1.0 / 3.0;          // R7 — 새것의 몫
/// 대상 트리에 끼운 **결과 트리**를 새로 만든다(제자리 변경 아님 — 영역 · 판정 · 적용이 같은 함수를 쓴다).
pub fn docked_tree(target: &LayoutNode, anchor: DockAnchor, side: DockSide, incoming: &LayoutNode)
    -> Result<LayoutNode, DockTreeError>;
pub enum DockTreeError { SlotNotFound(Uuid), SlotIdOverlap, ZeroArea }   // 판정 순서 = 기준 칸 → 겹침 → 면적
pub fn incoming_rect(result: &LayoutGeometry, incoming_slots: &[Uuid]) -> Option<RectF64>;  // 가이드의 「정확한 사각형」(R4)
```

- **분할 모양** — 기준 노드(슬롯 하나 또는 루트)를 새 분할 `Split{id: 새 UUID(ADR-0223), dir, ratio, a, b}` 로 감싼다.

  | side | dir(ADR-0140) | a | b | ratio(= a 몫) |
  |---|---|---|---|---|
  | Left | `LeftRight` | 새것 | 기준 | 1/3 |
  | Right | `LeftRight` | 기준 | 새것 | 2/3 |
  | Top | `TopBottom` | 새것 | 기준 | 1/3 |
  | Bottom | `TopBottom` | 기준 | 새것 | 2/3 |

  둘 다 `tree::RATIO_MIN`–`RATIO_MAX` 안이라 클램프에 걸리지 않는다. 정규화 좌표라 감싼 기준 노드 안쪽 비율은 그대로다 — 슬롯 옆은 그 슬롯만 2/3 로, 바깥 변은 기존 레이아웃 전체가 안쪽 비율을 지킨 채 2/3 로 준다(R7 · AC4 · AC5).
- **슬롯 id · 분할 id 는 바꾸지 않는다** — 출처 탭의 노드를 그대로 옮긴다(ADR-0223 「분할 id 는 노드를 따라간다」).
- **새 면적 0(R8)** — 결과 트리를 `geometry::compute` 로 재고, 결과에서 면적 0 인 칸 중 **원래 자리에서는 면적이 있던 칸**이 하나라도 있으면 `ZeroArea`. 원래부터 0 이던 칸은 빼고 본다(`set_split_ratio` ⑤ 와 같은 태도). 결과 트리 전체를 재므로 깊은 자식까지 선다. 좌표는 정규화 레이아웃 좌표다.

#### ViewManager — `manager.rs` (2 단계 · 끝 — 4 단계 고침 하나)

```rust
pub enum DockTo {
    Seam { target: ViewId, anchor: DockAnchor, side: DockSide },   // R7
    TabBar { window: WindowLabel, index: Option<usize> },          // R21 — index = 결과 탭 목록의 자리 · None = 맨 뒤
    NewWindow { label: WindowLabel },                              // R22 — 부르는 쪽이 이미 연 OS 창 label(phase C)
}
pub struct DockGuard { pub lease: Option<LeaseId>, pub expect_source: Option<LayoutNode>, pub expect_target: Option<TargetExpect> }
pub enum TargetExpect { Seam { layout: LayoutNode, active: ViewId } /* Seam — 대상 탭 배치 + 대상 창 활성(4 단계 · 5.1) */,
                        Tabs(Vec<ViewId>) /* TabBar — 대상 창 탭 목록 */ }
pub enum FocusPolicy { Keep /* 다음 작업의 명령 */, Reveal /* 마우스 — 옮긴 것을 보이고 첫 슬롯에 포커스 */ }
pub struct Docked { window: WindowLabel, slot_ids: Vec<Uuid>, source_window: WindowLabel, source_window_closed: bool }
pub enum DockError {
    TabNotFound(Uuid), WindowNotFound(String), SlotNotFound(Uuid), SlotNotInTab { slot: Uuid, tab: Uuid },
    SelfTarget, ZeroArea, Dragging, WindowClosed(String),
    StaleLease, SourceChanged, TargetChanged,             // ← 사전조건을 단 요청만
    WindowExists(String), SlotIdOverlap,                  // ← 닿지 않는 방어
    WindowOpen(String),                                   // ← 적용 서비스(새 창 열기 실패)
}                                                         // 4 단계 뒤 모양 — 2 단계의 MainLastTab 을 지운다(아래)
pub fn dock(&mut self, view: ViewId, to: DockTo, guard: &DockGuard, focus: FocusPolicy) -> Result<Docked, DockError>;
pub fn plan_new_window(&self, view: ViewId, guard: &DockGuard) -> Result<NewWindowPlan, DockError>; // R22 phase A
pub enum NewWindowPlan { Reuse { label: WindowLabel }, Create { size: Option<(f64, f64)> } }
pub fn lease_tab(&mut self, view: ViewId) -> Result<LeaseId, DockError>;  // 끌기 시작
pub fn release_lease(&mut self, id: LeaseId);                             // 끌기 끝 — 다른 표식이면 무동작
pub(crate) fn mark_closed(&mut self, label: &str);                        // 묘비(3 단계가 부른다)
pub fn succession(tabs: &[ViewId], removed: ViewId) -> Option<ViewId>;   // 활성 승계 — 떼어진 표시도 같은 규칙
```

**판정 순서가 계약이다**(앞에서 걸리면 뒤는 안 본다 · 걸리면 무변경 — `manager.rs:9`):

1. 출처 View 가 있나 → `TabNotFound`.
2. 목적지 — Seam 의 대상 View 없음 = `TabNotFound` · TabBar 의 창 없음 = `WindowNotFound`(대상 창에 묘비가 있어도 같은 사유) · NewWindow 의 label 에 묘비 = `WindowClosed` · 그 label 이 이미 모델에 있음 = `WindowExists`(닿지 않는 방어).
3. Seam 의 대상 = 출처 → `SelfTarget`(TabBar 의 대상 창 = 출처 창은 자리 옮기기로 허용 — R21).
4. Seam 의 기준 슬롯이 대상 탭에 있나 — 다른 탭이면 `SlotNotInTab`, 어디에도 없으면 `SlotNotFound`.
5. 표식 — 출처에 표식이 섰는데 `guard.lease` 가 없으면 `Dragging`. `guard.lease` 가 지금 표식과 다르거나 표식이 없으면 `StaleLease`(지난 끌기의 늦은 놓기).
6. `expect_source` 와 다르면 `SourceChanged` · `expect_target` 과 다르면 `TargetChanged`(R13c — Seam = 대상 탭 배치와 대상 창 모델 활성 · TabBar = 대상 창 탭 목록).
7. Seam 이면 `docked_tree` → `SlotIdOverlap`(닿지 않는 방어) · `ZeroArea`.

적용(전부 `&mut self` 안 — 락은 부르는 쪽 한 번 · `bump_version` 한 번):

- **Seam** — 대상 View 의 `layout` 을 결과 트리로 바꾼다. **모르는 내용 곁표**(`unknown_content`)의 출처 항목을 슬롯 id 째 대상 View 키로 옮긴다(안 옮기면 `remove_view` 가 원문을 지워 새 판이 쓴 슬롯이 다음 저장에서 사라진다). 출처 탭을 출처 창 탭 목록에서 빼고(도우미) `remove_view(출처)`.
- **TabBar** — 출처 창 목록에서 빼고 대상 창 목록의 `min(index, 길이)` 자리에 넣고 `view_owner` 를 바꾼다(View 째라 id · 곁표가 따라간다). 같은 창이면 자리만 바뀐다 — 같은 자리면 무변경 · `version` 불변 · `Ok`. 단 `Reveal` 이고 그 탭이 그 창의 보이는 탭이 아니면 보이게만 한다(`version` +1 · §11-1 #6 — 확정).
- **NewWindow** — 출처 창에서 빼고 `WindowTabs::new_popout(view)` 로 새 창 항목.
- **탭 목록에서 빼기 도우미** — 활성 승계는 `close_tab` 과 같은 규칙(오른쪽 우선 · 없으면 왼쪽). 팝아웃이 비면 창 항목을 지우고 `source_window_closed = true`(R10). ★4 단계 — 메인이 비면 같은 연산 안에서 빈 탭 하나를 만들어 활성으로 둔다(아래)★.
- **보이는 탭** — 대상 창의 활성 탭은 손대지 않는다. 예외: 출처가 대상 창의 활성 탭이었으면(같은 창 도킹) 대상이 보이는 탭이 된다(Seam = 대상 탭 · TabBar = 옮긴 탭 — §11-1 #1). `Reveal` 이면 늘 대상(Seam = 대상 탭 · TabBar · NewWindow = 옮긴 탭)을 활성으로.
- **포커스** — `Reveal` 이면 Seam 은 대상 View 의 `focused_slot_id` = 끼운 첫 슬롯, TabBar · NewWindow 는 그 탭의 포커스 그대로(R11b). `Keep` 이면 손대지 않는다.
- **표식** — `guard.lease` 가 지금 표식이면 내린다.

`plan_new_window`(R22 phase A) — 판정 1 · 5 · 6 을 본 뒤(대상 쪽은 `expect_target` 이 있으면 `TargetChanged`), 출처가 **탭 하나뿐인 팝아웃**이면 `Reuse{그 창}`(새 창 없이 그 창을 옮긴다 — §7 #24 · 옮긴 뒤 다시 확인 · 되돌리기 없음 — 사용자 결정 Q19 · 5.6 · 아래 「적용 서비스」), 아니면 `Create{size}`(5.4 의 「보통 자리가 있을 때만」 조건은 5.6 에서 거뒀다 — 아래 「4 단계 고침 셋」) — size = 출처가 팝아웃이면 그 창 `attrs.bounds` 의 논리 크기(최대화 중이어도 보통 자리 크기 · §11-1 #3), 메인이면 `None`(팝아웃 기본 크기 720×500 — `popout.rs:143`).

**★4 단계 고침 — 메인 마지막 탭(PRD R14 · Q9)★** — `check_dock` 의 메인 마지막 탭 판정 · `lease_tab` 의 거절 · `plan_new_window` 의 거절을 걷고 `DockError::MainLastTab` 을 지운다. 빼기 도우미가 `Pulled::MainEmptied` 를 내면 메인에 「View 1」 빈 탭을 만들어 활성으로 둔다 — `close_tab` 의 `Pulled::MainEmptied` 갈래와 같은 일이라 도우미 `refill_main` 하나로 뽑아 둘이 나눠 쓴다(`pull_docked_tab` 의 `debug_assert_ne!(MainEmptied)` 는 지운다). 불변식 ④ 는 락 한 번 안에서 선다 — 관찰자는 빈 메인을 못 본다. 같은 창 안(Seam 대상 = 자기 · 탭바 같은 자리)은 그 전에 `SelfTarget` · 무변경으로 걸리므로 빈 탭은 다른 창 · 새 창으로 떠날 때만 생긴다. 기존 시험 중 `MainLastTab` 을 단언하던 것은 「빈 탭이 생긴다 + `assert_invariants`」로 바꾼다.

**~~4 단계 고침 셋 — 재사용은 보통 자리가 있을 때만~~(5.4 · 리뷰 4라운드 → ★5.6 에서 거둠 · Q19 — 근거가 사라졌다★)** — 5.4 는 `plan_new_window` 의 `Reuse` 갈래에 「`source.attrs.bounds` 가 `Some`」을 더해 없으면 `Create{size: None}` 으로 보냈다. 유일한 근거는 재사용 되돌리기가 모델 보통 자리로만 창을 돌려놓는다는 것이었는데, 사용자 결정 Q19 로 되돌리기가 없어졌다. 그래서 탭 하나뿐인 팝아웃은 보통 자리 유무와 무관하게 늘 `Reuse` 다(메인 결정 2026-10-16 — 웹뷰를 새로 띄우지 않는다 · 걸침 판정 크기는 `at.size` → 모델 보통 크기 → 지금 안쪽 크기 → 기본으로 물러난다 — 아래 「자리 검증」). 4 단계 코드도 그 조건 · 주석 · 시험을 걷는다.

**★4 단계 고침 둘 — 대상 창 활성(5.1 · 리뷰 C1)★** — `TargetExpect::Layout(LayoutNode)` 를 `TargetExpect::Seam { layout, active }` 로 바꾸고, `check_dock` 의 대상 대조(`target_holds`)가 Seam 이면 대상 탭 배치와 함께 `windows[대상 창].active == active` 를 같은 락 안에서 본다 — 다르면 `TargetChanged` · 무변경. 이유 = `switch_tab`(`ViewManager::switch_tab` · 적용 서비스 `apply::switch_tab`)은 활성을 락 안에서 바꾸고 알림은 락을 놓은 뒤 낸다. 그 틈에 장부 · 웹뷰는 옛 활성의 영역을 쥐고 있어, 배치만 대조하면 이음새 놓기가 이제는 안 보이는 탭에 들어가고 `Reveal` 이 그 탭을 도로 앞에 낸다(R6b · 사람 클릭으로 바꾼 탭이 말없이 되돌아간다). `active` = 영역을 만든 순간 그 창의 **모델** 활성이다 — 같은 창 도킹에서 끄는 탭이 활성이면 그 끄는 탭 자신이다(화면의 승계 탭이 아니다 — 웹뷰 쪽 대조는 §1-3 「놓기 판정」). TabBar · NewWindow 는 그대로. 바뀌는 곳 = `TargetExpect` 정의 · `check_dock` 의 Seam 팔 · 판정 순서 주석(7) · 시험의 `TargetExpect::Layout` 자리 여덟 + 새 시험(배치는 같고 활성만 다름 = `TargetChanged` · 무변경 · 같은 창 도킹의 기대값 = 끄는 탭이 그대로 통과).

#### 표식(lease) — 「이 탭은 지금 끄는 중」

`ViewManager.lease: Option<(LeaseId, ViewId)>` — 한 번에 하나. `lease_tab` 은 판정 1 · 「이미 표식이 있음」(= `Dragging`)을 보고 세운다. 모델 안에 두는 이유: 「끄는 중인 탭을 옮기는 명령은 실패」(D9 — 다음 작업의 명령)가 그 명령의 변경과 같은 락 안에서 서야 한다. 표식 번호 = 끌기 세션의 표식이다 — 모델은 같은지만 본다. 메모리에만 있다. 표식이 선 탭이 지워지면(`remove_view` — 탭 닫기 · 창 닫기 · 복원 수락) 표식도 내려가고, 그 표식을 쥔 늦은 놓기는 `StaleLease` 다. ★지금 표식 없이 `dock` 을 부르는 길은 없다★ — 마우스 놓기만 부르고 늘 표식을 싣는다. `Dragging` · `FocusPolicy::Keep` 은 다음 작업의 명령이 쓴다.

#### 닫히는 창 — 모델 먼저 · 묘비 (3 단계 · M2)

- **(a) 닫기 요청 = 모델에서 먼저 지움.** `on_window_event` `CloseRequested` 에서 label 이 팝아웃이면 레이아웃 락 한 번 안에서 `drop_window_in_model`(창의 탭 View 전부 · 창 항목 · 라우팅 재계산 · 구독 해제 · 사용량 관심)을 하고 `mark_closed(label)` 로 묘비를 단다. 뒤이은 `Destroyed` 는 같은 정리를 부르지만 모델에 그 창이 없어 재계산 · 묘비만 남는다(멱등). `Destroyed` 도 `mark_closed` 를 부른다 — 닫기 요청 없이 사라진 창까지 묘비가 선다.
- **근거 — 팝아웃의 닫기 요청은 되돌려지지 않는다(확실 · 소스)** — 우리 처리기는 팝아웃에 `prevent_close` 를 부르지 않고(main 만), 프론트에 닫기 요청 리스너가 없으며, 런타임은 콜백이 돌아오자마자 창을 닫는다(`tauri-runtime-wry-2.11.3/src/lib.rs:4438-4466`). ★누가 팝아웃 닫기를 막을 수 있게 만들면 이 절은 「닫는 중 표시 + 막히면 해제」로 바꿔야 한다★ — 처리기 주석과 §9 「게이트」의 점검 명령 두 줄이 그 전제를 지킨다.
- **(b) 새 창 경로는 묘비 앞에서 멈춘다.** phase C 의 판정 2 가 `WindowClosed` 로 무변경 — 탭은 출처에 남고 적용 서비스는 연 창을 `host.close` 한다. `attach_view_as_new_window` 에도 같은 검사가 있다.
- **(c) 묘비 뒤에 붙은 탭이 지워지는 일은 없다(락 순서).** 묘비와 모델 삭제는 같은 락 한 번이다. 그 뒤의 확정은 대상 없음 · 묘비로 무변경이고, 그 앞의 확정은 닫기 시점에 그 창의 탭이라 그 닫기와 함께 닫힌다(창 닫기 규칙 그대로 · 잔해 없음).
- **팝아웃 `slot.popout` 의 같은 틈도 닫혔다** — 기존 창 타깃은 phase C 의 `insert_tab_into` 가 창 없음으로, 새 창 타깃은 묘비로 롤백을 탄다(`apply.rs` `WindowHost` 머리 주석이 3 단계에서 고쳐졌다).
- **닫히는 창의 늦은 replay 요청** — 닫기 요청이 구독 해제를 낸 뒤에도 닫히는 웹뷰가 잠깐 살아 replay 를 청할 수 있다. 연결 task 가 그 에이전트를 보이는 창이 없으면 Subscribe 를 보내지 않는다(`daemon_client/connection.rs` `RequestReplay` 갈래 · 3 단계). ★보이는 창이 하나라도 있으면 지나간다★ — 도킹으로 옮긴 에이전트가 그렇다(무해 — §5 「출력 연속」).
- ADR-0057 불변식(① 양방향 · ② 유니크 소유 · ③ 활성 소속 · ④ main ≥ 1)은 그대로이고 정리의 계기만 소멸 → 닫기 요청으로 옮긴다(ADR-E · §11-1 #2 — 사용자 결정 2026-10-16). 묘비 `closed_labels` 는 메모리만이고 팝아웃 label 은 한 부팅 안에서 다시 쓰이지 않는다(`PopupCounter`).

#### 적용 서비스 — `apply.rs` (4 단계)

```rust
pub struct DockRequest { pub view: ViewId, pub to: DockDest, pub guard: DockGuard, pub focus: FocusPolicy }
pub enum DockDest {
    Seam { target: ViewId, anchor: DockAnchor, side: DockSide },
    TabBar { window: WindowLabel, index: Option<usize> },
    NewWindow { at: WindowPlace },
}
pub struct WindowPlace { pub pos: Option<(i32, i32)> /* 물리 바깥 왼쪽 위 */, pub size: Option<(f64, f64)> /* 논리 안쪽 */ }  // R22 자리
pub enum MoveStep { Position, Size, Maximize }                     // 창 포트의 세터 하나(5.3)
pub struct MoveFailed { pub step: MoveStep, pub error: String }
pub struct DockOutcome { pub window: WindowLabel, pub slot_ids: Vec<Uuid>, pub closed_window: Option<WindowLabel>, pub reused: bool }
pub fn dock_tab(state, subs, events, host, labels, req: DockRequest) -> Result<DockOutcome, DockError>;
```

- **Seam · TabBar = 락 한 번.** `mgr.dock(..)?` → 대상 View · 대상 창 · (남았으면) 출처 창 스냅숏 → `subs.resync(&mgr)` → 락 드롭 → 알림(대상 배치 · 대상 창 · 출처 창 탭 목록) → `events.settled()` 한 번 → 출처 팝아웃이 비었으면 `host.close(label)`(락 밖 · `apply.rs` 머리 「락 규율」). 닫히는 대상 창은 M2 로 막히므로 `is_open` 사전 확인을 두지 않는다.
- **알림 묶음 끝 `LayoutEvents::settled()`(5.1 · 리뷰 F1)** — **알림을 하나라도 낸 쓰기**가 그 묶음 끝에 한 번 부른다(알리는 쓰기는 다 `apply::announce` 한 곳을 지난다 — `notify` · `rename_tab` · 슬롯 옮기기 · `restore.rs` 의 복원 수락 · `dock_tab`). 낸 것이 없는 쓰기(실패 · 무변경)와 재사용 성공(모델 변경 0)은 부르지 않는다. ★창을 통째로 닫거나 만드는 쓰기는 알림이 없어 `settled` 도 없다★ — 팝아웃 마지막 탭의 `close_tab` · `close_window`(둘 다 `host.close`) · `create_window`. 6 단계는 그 경우를 `Destroyed`(닫힘)와 크롬 · 캔버스 보고에 기댄다. 남는 틈 = `create_window` 로 막 생긴 빈 창은 그 웹뷰의 크롬 · 캔버스 보고나 다른 계기가 올 때까지 스냅숏 계기가 없다(끄는 중 빈 창이 새로 생기는 드문 경우 — 그 창 영역이 늦을 뿐이고 웹뷰는 끌기를 만나면 `dock_pull` 로 당긴다). 기본 본문 없음(구현자 아홉이 컴파일에서 걸린다) · `TauriEvents` 만 6 단계에서 「세션 깃발이 섰으면 스냅숏을 떠 장부에 먹인다」를 단다(§1-3 「영역」). 알림 하나마다 먹이면 첫 알림 뒤의 스냅숏이 나머지 알림보다 먼저 영역 · 끝을 내보낸다 — 웹뷰가 새 영역을 탭 목록보다 먼저 받고, 도킹 적용 중이면 장부가 그 스냅숏으로 세션을 끝낸다(§1-3 「적용 중」). 알림 자리가 이것을 빠뜨리면 영역이 늦을 뿐이다 — 그 사이 놓기는 무변경으로 떨어진다(안전 쪽).
- **NewWindow = 세 phase**(팝아웃 `move_slot_to_window` · `apply.rs:657` 을 따른다):
  - A(락) `plan_new_window`. `Reuse{label}` 이면 아래 「재사용」이고 B · C 는 없다.
  - **재사용 = A 판정 → 창 옮기기 → 표식 내리기 · 다시 확인 · 되돌리기 없음(사용자 결정 Q19 · 5.6)**:
    - B′(락 밖) `host.move_window(label, &at)` — 위치 → 크기 순으로 세터를 부르고 첫 실패에서 멈춘다(`MoveFailed{step}`). 실패면 warn 하고 `WindowOpen` — **되돌리지 않는다**. 운영에서 이 실패는 「이벤트 루프가 닫힘 · 창이 없음」뿐이다 — 메인 스레드가 아닌 곳의 Tauri 세터는 이벤트 루프에 메시지를 보내고 기다리지 않는다(`tauri-runtime-wry-2.11.3/src/lib.rs:235-255` — OS 수준 세터 실패는 보이지 않는다). 즉 종료 중에만 탄다.
    - 성공이면 레이아웃 락 안에서 `release_lease` · 모델 변경 0 · `reused = true` → 락 드롭 → 그 창이 아직 모델에 있으면 `host.record_placement(label)`(그 사이 닫혀 모델에 없으면 적을 항목이 없다 · 숨긴 출처를 보이는 것은 장부의 끝 몫 — §1-3).
    - **그 사이 변화는 결과 그대로(Q19)** — A 판정과 창 옮기기 사이(ms)에 다른 경로가 출처를 바꿔도 다시 보지 않는다. 끄는 동안 그 팝아웃은 숨어 있어 그 틈에 바꿀 수 있는 것은 다른 경로(LLM)뿐이고, 재사용은 모델을 바꾸지 않아(창 자리만) 옮긴 채 두어도 모델과 화면이 어긋나지 않는다. 결과: 출처 탭 배치가 바뀜 = 바뀐 채 옮겨짐 · 탭이 닫힘 = 팝아웃은 기존 규칙대로 닫힘(옮기기 전이면 옮기기가 「창 없음」으로 실패) · 끌기가 무변경으로 끝남 = 창은 놓은 자리에 남음 · 탭이 하나 더 들어옴 = 둘이 함께 옮겨짐(받아들임). 놓기 **전**의 출처 변화는 그대로 장부의 `blocked`(R13c)가 막는다 — 예외는 놓은 뒤의 이 짧은 사이뿐이다. 5.2–5.5 의 「옮기고 → 같은 락 안 재확인 → 어긋나면 모델 자리로 되돌리기」는 거부한 대안이다(§7 #39).
    - **보이는가** — 최대화가 아니던 출처는 끄는 동안 숨어 있어 옮기기가 화면에 안 보이고 끝의 되보이기가 놓은 자리에 보인다. 최대화였던 출처는 B′ 의 위치 세터가 최대화를 풀며(`placement.rs` `maximize` 머리 F8) tao `apply_diff` 가 `ShowWindow(SW_RESTORE)` 뒤 보임 표식이 없어 `ShowWindow(SW_HIDE)` 로 되숨긴다(`tao-0.35.3/src/platform_impl/windows/window_state.rs:376-386` · `:420-423`) — 한 번 깜빡인 뒤 숨은 채 남고, 끝의 되보이기가 놓은 자리에 보인다(소스 · GUI 미검 · 받아들인 경우 · S1 ⑤ · 창 포트 설명과 같은 사실). 숨기기가 실패해 보인 채였던 출처는 옮겨 가는 것이 보인다. 깜빡임 없이 최대화를 푸는 길은 platform 목적 함수(Win32 `SetWindowPlacement` 류)가 있어야 하고 이번엔 넣지 않는다(§11-2 #24 미결).
  - B(락 밖) `labels.next_label()` → `host.open_at(label, &at_with_size, activate = focus == Reveal)`. 실패면 `WindowOpen` · 모델 무변경.
  - C(락) `mgr.dock(view, NewWindow{label}, ..)` 로 **다시 판정하고** 적용 — B 와 C 사이에 그 창이 닫혔으면 묘비로 `WindowClosed`. 실패면 락 드롭 → `host.close(label)` → 그 오류. 성공이면 `resync` → 락 드롭 → 알림 → `settled()` → `host.record_placement(label)`.
- **창 포트 확장 — 기본 본문 없음** — `fn open_at(&self, label, at: &WindowPlace, activate: bool) -> Result<(), String>` · `fn move_window(&self, label, at: &WindowPlace) -> Result<(), MoveFailed>`(위치 → 크기 · 첫 실패에서 멈춤 · 창이 없으면 `Err` — 재사용 몫). 5.3–5.5 의 되돌리기 포트 `restore_window` 와 성공 모양 `Moved{Applied, StayedPut}` 은 Q19 로 걷었다(5.6). 기본 본문을 두면 `OwnedWindowHost` 가 물려받아 자리 · 크기 · 활성을 말없이 버린다 — 본문 없이 두어 구현자 일곱(§1-1 표 — 3 단계가 `ClosingHost` 를 더했다)이 컴파일에서 걸리게 한다. Tauri 구현: `open_at` = 창을 첫 자리와 `focused(activate)` 로 만든 뒤 검사 세터 `placement::move_steps`(위치 먼저 → 크기 · 첫 실패에서 멈춤)로 정확히 놓는다 — 새 창은 최대화로 만들지 않으므로 풀 최대화가 없다 · 크기 없으면 팝아웃 기본 크기 · 웹뷰 환경은 `build_window` 가 `WebviewEnv::finish` 로 붙인다 · 자리 입히기가 실패하면 그 창을 거두고 `WindowOpen` — 그 오류 문장에 「잘못 놓여 창을 거뒀다」(거두기도 실패했으면 그 사유)를 싣는다(5.5). `move_window` 도 같은 `move_steps` 를 쓴다 — 부팅 · 복원의 `placement::place`(`placement.rs:470` — 세터 실패를 로그로만 남기고 `()`)는 쓰지 않는다. ★`move_window` 가 숨은 최대화 창을 옮기면 위치 세터의 최대화 풀기에서 한 번 깜빡였다(`SW_RESTORE`) 다시 숨는다(`SW_HIDE`)★ — 창은 숨은 채 남고 끝의 되보이기가 놓은 자리에 보인다(아래 「보이는가」와 같은 사실 · S1 ⑤).
- **자리 검증** — 마우스 자리(커서 − 그림 커서 자리)는 보통 모니터 위지만 화면 가장자리에서 밖으로 나갈 수 있다. 둘이 다르다(5.5):
  - `open_at`(새 창) — `pos` 에 크기(`at.size` · 없으면 팝아웃 기본 크기)를 붙인 사각형을 `placement::land(.., Fallback::DefaultPlace)` 로 본다 · 어느 모니터에도 안 걸치면 기본(계단식) 자리(저장 자리 복원과 같은 규칙 · 같은 info 로그).
  - `move_window`(재사용) — 같은 판정을 그 창이 옮긴 뒤 가질 크기로 한다(`at.size` · 없으면 모델의 보통 자리 크기 · 그것도 없으면 지금 안쪽 크기 · 그것도 없으면 팝아웃 기본 — `placement.rs` `kept_size` · 최대화된 창의 게터는 최대화 크기라 모델 크기가 먼저다). 안 걸치면 위치 걸음을 건너뛰어 창은 제자리에 남는다(`Fallback::StayPut` — 떠 있는 창을 기본 자리로 옮기지 않는다).
- **오류** — `DockError` 는 `Display` 를 갖는다(한국어 사유 + 괄호 속 종류 낱말). 마우스 경로는 끝 로그의 `reason` 으로 남긴다(§1-7). 레이아웃 적용 서비스의 첫 타입드 오류다 — 다음 작업의 명령이 종류로 오류 코드를 고른다.

### 1-3. ② 끌기 장부 (셸 `dock`)

#### 장부 — 순수 상태 하나 + 잎 락

```rust
// dock/book.rs — Tauri · 락 · 시계 읽기 · OS 0. 시각 · 커서 · 창 사각형 · 스냅숏은 전부 입력이다.
pub struct Book {
    drag: Option<Drag>, last_snap: u64,
    held_snap: Option<Snap>,            // 세션 전(시작 락 뒤 · `Start` 전)에 온 스냅숏 — `Start` 가 이어 먹는다(5.1 · N8)
    early: Option<Early>,               // 모르는 drag_id 의 끝 신호 — 한 번에 한 id(5.1 · 「이른 끝 신호」)
    ended: VecDeque<DragId>,            // 최근 끝난 id 넷 — 늦은 신호와 이른 신호를 가른다
}
struct Drag {
    id: DragId /* 출처 웹뷰가 만든 UUID 문자열 */, lease: LeaseId, seq: u64, hotspot: PointCss,
    gen: u64 /* (리뷰 2) 세션 번호 = 시작 락 안에서 세운 세션 깃발 세대 그대로 — 시한 · 숨기기 · 보이기에 단다(「명령의 실행 스레드」) */,
    source_label: WindowLabel, source_view: ViewId, expect_source: LayoutNode, source_hidden: bool,
    source_destroyed: bool /* 적용 중에 출처가 사라짐 — 되보이기를 건너뛴다 */,
    display_active: Option<ViewId>, blocked: bool,
    strip: Option<StripStart>,          // (5.7) 출처 웹뷰가 dragstart 안에서 잰 탭바 — 세션 내내 그대로(얼린 기하 · §2-4)
    source_rect: Option<..>,            // (리뷰 3) 출처 창 클라이언트 사각형 · 배율 — 시작에 읽고 출처가 든 Rects 마다 갈아 끼운다 · 「시한이 앞지른 끝」의 사각형 검사 몫(숨긴 뒤 Rects 에는 출처가 없다)
    hide_pending: bool,                 // (5.7) 탭 하나 팝아웃의 숨기기를 도킹 모드 증거까지 미뤘다(「순서 이동 · 도킹 모드 증거」) — 출처 Left 뒤 숨기기 시한(Leave)의 번호를 함께 쥔다(리뷰 1)
    sent: HashMap<WindowLabel, Sent>,   // 창별 마지막 보낸 영역 — rev · bar_rev · 영역 · 기대값(TargetExpect 재료: Seam{배치, 창 활성} · Tabs) · (리뷰 2) 숨긴 출처 몫은 끝까지 남긴다(「도킹 모드 증거」)
    seen: HashMap<WindowLabel, Seen>,   // 창별 — 마지막 Over{rev, bar_rev, displayed, shown, point, scroll(5.7), 도착 순번, 시각} · 마지막 Left 시각(따로 — 5.1 · F4)
    phase: Live | AwaitDrop { at, cursor } | Grace { at }
         | AwaitOver { at, end /* (리뷰 4) 붙든 출처 끝 — cursor · rects · now · last 그대로 */, from: Option<WindowLabel> /* 붙든 동안 Over 가 닿은 다른 창(마지막) */ }
         | Applying | Ended,
}
pub enum Input { Start{.., strip /* 5.7 */, source_rect /* 리뷰 3 */}, Refused{drag_id}, Snapshot(Snap), Over{..}, Left{..}, Drop{..},
                 SourceEnd{effect, canceled /* 5.7 */, last: Option<DrawnReport> /* 리뷰 2 */, cursor: Option<..>, rects, now}, PointerSeen{now}, Timeout{kind, gen /* 리뷰 2 */, cursor, rects, now},
                 Destroyed{label}, Applied(Result<DockOutcome, DockError>), Pull{label} }
pub enum Action { Broadcast(SessionEvent), Regions{label, DockRegions}, Hide{label, gen}, Show{label, gen} /* 리뷰 2 — 메인 스레드에서 장부 값으로 맞춘다 */, Focus(label),
                  Apply(DockRequest), ArmTimer{kind /* Drop · Grace · Early · Leave(5.7 · 리뷰 1 — 미룬 숨기기의 떠남 시한) · OverWait(리뷰 4 — 오는 중 Over 붙들기) */, gen /* 리뷰 2 */, after}, ReleaseLease(LeaseId), EndLog(..) }
pub fn visibility(&self, label: &WindowLabel) -> Visibility /* (리뷰 2) Hidden = 살아 있는 세션의 출처이고 source_hidden · 그 밖 Shown — 실행기가 메인 스레드에서 읽는다 */;
```

- **잎 락** — 셸 상태 `Mutex<Book>`. 쥔 채 레이아웃 락 · emit · 창 호출 · 적용 서비스 · 커서 읽기를 하지 않는다. 실행기(`commands/dock.rs`)가 필요한 값을 먼저 읽어 입력에 싣고, 장부가 낸 할 일을 락 밖에서 실행한다. 영역 계산(순수 · ms 이하)은 장부 안에서 한다.
- **창 게터는 두 락 밖에서(5.1 · N5)** — `is_visible` · `is_minimized` · `inner_position` · `inner_size` · `scale_factor` · `cursor_position` 은 레이아웃 락도 장부 락도 쥐지 않고 읽는다. 메인 스레드가 아닌 곳의 게터는 이벤트 루프에 메시지를 보내고 답을 기다리는데(`tauri-runtime-wry-2.11.3/src/lib.rs:197-211` · `:235-255`), 메인 스레드의 창 사건 처리기(`Destroyed` · `CloseRequested`)는 레이아웃 락을 잡는다 — 락을 쥔 채 게터를 부르면 교착이다(`apply.rs` `WindowHost` 머리 「락 밖에서만」과 같은 이유). 그래서 스냅숏(락 안 · 모델 값)과 사각형 · 배율 · 커서(락 밖 · 게터)는 따로 떠서 입력에 함께 싣는다. `WindowSnap.hidden` 은 게터가 아니라 장부 값(`source_hidden`)이다 — `hide()` 직후의 `is_visible` 은 늦을 수 있다.
- **스레드 · 틱 없음** — 장부는 사건(invoke · 창 사건 · 모델 계기 · 시한)이 올 때만 돈다. 시한(`AwaitDrop` · `Grace` · (리뷰 4) `AwaitOver` · 이른 신호)은 `tauri::async_runtime::spawn` + `sleep` 한 건이 `Timeout` 을 먹인다. 세션 시한(`AwaitDrop` · `Grace` · (리뷰 4) `AwaitOver`)은 한 번에 하나 · 이른 신호 시한 · (5.7) 숨기기 시한(`Leave` — 「도킹 모드 증거」 ②)은 각각 따로 하나이고 번호를 달아 지난 시한은 버린다 — ★(리뷰 2) 번호에 세션 번호 `gen` 을 함께 단다 — `Timeout` 의 `gen` 이 지금 세션의 것이 아니면 무엇이든 버린다★(끝난 세션의 `Leave` 시한이 새 세션 `Live` 중에 닿아 새 출처를 숨기지 않게 — 세션마다 0 부터 세는 번호만으로는 겹친다) — `Grace` 중 끝 신호는 평소대로 처리하고(`dragend(move)` 면 `AwaitDrop` 으로 갈아탄다), `AwaitDrop` · (리뷰 4) `AwaitOver` 중 `pointer_seen` 은 버린다.
- **놓기 · 끝 신호 대조** — `drag_id` 가 최근 끝난 id(`ended`)면 늦은 신호로 버린다. 처음 보는 id 의 끝 신호(`dock_drop` · `dock_drag_end`)는 붙든다(아래 「이른 끝 신호」). `Over` · `Left` · `pointer_seen` 이 모르는 id 면 버린다. `dock_drop` 의 `view_id` 가 그 세션의 출처가 아니면 버린다(다른 인스턴스의 끌기 포함).
- **스냅숏 번호** — 실행기가 레이아웃 락 안에서 원자 셈을 올려 스냅숏에 붙인다. 장부는 `last_snap` 이하 번호를 버린다 — 두 계기가 엇갈려 옛 스냅숏이 새 영역을 덮지 않게.
- **한 세션에 적용은 많아야 한 번** — 판정과 `Applying` 표시가 장부 락 한 번 안이다. 그 뒤 오는 놓기 · 끝 신호는 늦은 신호다. `Applying` 동안의 스냅숏 · 소멸은 아래 「적용 중」.
- **명령의 실행 스레드** — `dock_drag_start` · `dock_drag_over` · `dock_pointer_seen` · `dock_pull` · `report_dock_chrome` 은 동기 명령(메인 스레드 · 도착 순서대로)이다 — 같은 웹뷰의 시작 → 끝 순서가 그렇게 선다. 창을 만들 수 있는 `dock_drop` · `dock_drag_end` 와 시한 콜백은 async 다(동기 명령에서 창을 빌드하면 교착 — `popout.rs` `move_slot_to_window`(:185) 선례).
- ★**불변식 — `Apply` 는 메인 스레드 · 동기 명령 안에서 돌지 않는다(5.2 · 리뷰 R2-1)**★ — 실행기는 `Apply` 할 일을 먹인 스레드와 무관하게 늘 `tauri::async_runtime::spawn` 으로 넘기고, 그 결과 `Applied` 와 그 끝 할 일도 그 async 작업에서 먹인다. 할 일은 보통 먹인 스레드에서 실행되는데(아래 「순서」), 동기 명령도 `Apply` 를 낼 수 있기 때문이다 — `dock_drag_start`(메인 스레드)가 `Start` 뒤에 붙든 이른 놓기를 이어 먹이면(「이른 끝 신호」) 그 자리에서 판정 → `Apply` → R22 새 창 열기가 메인 스레드의 동기 명령 안에서 돌아 교착한다. `Apply` 말고 동기 경로에서 나올 수 있는 할 일(emit · 숨기기 · 보이기 · 포커스 · 표식 내리기 · 시한 걸기)은 창을 만들지 않아 먹인 스레드에서 그대로 돈다 — ★숨기기 · 보이기 · 그 뒤 포커스는 아래 불변식(리뷰 2)★.
- ★**불변식 — 숨기기 · 보이기는 메인 스레드에서 · 장부의 지금 값으로 맞춘다(리뷰 2 · 기본값 · 메인 판단)**★ — 실행기는 `Hide{label, gen}` · `Show{label, gen}` 을 그대로 부르지 않고 메인 스레드 줄(`run_on_main_thread` — 먼저 올린 것이 먼저 돈다)에 「맞추기」 한 건으로 올린다. 맞추기는 돌 때 장부 락을 잠깐 잡아 `visibility(label)`(살아 있는 세션의 출처이고 `source_hidden` = 숨김 · 그 밖 = 보임)을 읽고 락을 놓은 뒤 그 값으로 `hide()` · 보이기 경로(`apply_deferred_maximize`)를 부른다(이미 그 값이면 무동작). ★지난 세션 `gen` 의 할 일은 제 뜻을 버린다★ — 지난 세션의 `Hide` 는 아무것도 숨기지 않고, 지난 세션의 `Show` 는 그 창의 지금 값(새 세션이 그 창을 숨겨 두지 않았으면 보임)으로만 맞춘다(그대로 버리면 새 세션 시작이 앞질렀을 때 옛 출처 팝아웃이 숨은 채 남는다). 같은 끝의 `Focus` 도 같은 줄에 그 뒤로 올린다(보이기 전에 포커스를 주지 않게). ★왜★ — 5.7 첫 판은 할 일을 먹인 스레드에서 장부 락 밖에 돌렸다(`commands/dock.rs` `run`). `Leave` 시한의 `Hide` 는 런타임 스레드, 끝의 `Show` 는 `dock_drag_end` 의 async 스레드에서 나오고 두 창 호출은 각자 이벤트 루프에 올라가므로, 이벤트 루프가 받는 순서는 장부가 낸 순서가 아니다 — 보이기 → 숨기기로 닿으면 세션이 끝난 뒤 팝아웃이 숨은 채 남는다. 맞추기는 늘 실행 때의 장부 값을 입히므로 마지막으로 돈 맞추기가 장부의 마지막 값이다(장부가 값을 바꿀 때마다 맞추기 한 건이 뒤에 올라간다). 장부 락은 잎이라 메인 스레드에서 잠깐 잡아도 교착이 없다(쥔 채 창 호출 · 게터를 하지 않는다 — 「잎 락」).

| 상수 | 값 | 뜻 |
|---|---|---|
| `LEAVE_GRACE_MS` | 100 | 출처 `dragend`(none) 앞의 대상 「떠남」이 이보다 가까우면 Esc 취소가 낸 떠남으로 보고 그 창을 「아직 위」로 친다 — S1 이 간격을 잰다 · (5.7 · 리뷰 1) 탭 하나 팝아웃의 미룬 숨기기가 출처 `Left` 를 증거로 삼기 전 기다리는 시간 · 출처 웹뷰가 도킹 모습으로 바꾸기 전 기다리는 시간도 같은 값(같은 경계라 판단이 어긋나지 않는다 — 「도킹 모드 증거」 ② · §1-4 「모드」) · (리뷰 2) 출처 웹뷰가 끝 스냅숏 `last` 를 실을지 가르는 떠남 → `dragend` 사건 시각 간격 · 「시한이 앞지른 끝」의 경계도 같은 값 |
| `CANCEL_LEAVE_MS` | 15 | (리뷰 3 · 기본값 · 메인 판단) 출처 끝 `last` 의 떠남 → `dragend` 사건 시각 간격(`left_before_end_ms` — 출처 웹뷰 한 엔진 시계)이 이 안이면(또는 떠남 없음) 그 떠남은 엔진이 끌기를 접으며 현재 대상에 낸 것(Esc · 받지 않는 자리의 떼기 — HTML 끌어 놓기 규칙)이라 끝이 출처 위에서 났다고 본다 → 사각형 검사 없이 `last` 자리 · 그보다 크고 `LEAVE_GRACE_MS` 안이면 실제로 창을 떠난 것 → 사각형 검사 그대로(§1-3 「끝 스냅숏」) · 첫 값(엔진 접기 → `dragend` 2–4 ms — §8-1 ② · Esc 의 떠남 → 끝 간격 자체는 미검) · S1 ① · S7 ⑬ ⓓ 가 잰다 · (리뷰 4) 그보다 큰 간격 + 도착 순번 W 가 출처이거나 없음 = 오는 중 `Over` 붙들기(`OVER_WAIT_MS`) · 안 + W 없음 = W 를 출처로(§1-3 「오는 중 `Over` 붙들기」 · 「마지막 점에 놓기」) |
| `OVER_WAIT_MS` | 50 | (리뷰 4 · 기본값 · 메인 판단) 출처 `none` 끝이 「실제로 떠났다」(`last` 간격 > `CANCEL_LEAVE_MS`)고 말하는데 도착 순번 W 가 출처이거나 없을 때, 다른 창의 오는 중 `Over` 를 기다리며 그 끝을 붙드는 시간(`AwaitOver` — §1-3 「오는 중 `Over` 붙들기」) · 근거 = invoke 왕복 실측 p50 2.2 · p99 ~16 · 최대 ~50 ms(§8-1 — emit → JS 는 p99 11 · 최대 33) — 끝보다 먼저 낸 `Over` 의 남은 도착 시간은 invoke 한 번보다 짧으므로 실측 최대가 덮는다 · 같은 창 줄에서 앞 invoke 뒤에 선 `Over` 는 넘을 수 있다(드묾 · 넘으면 붙들기 없는 판정과 같다 — 5.6 · 리뷰 3 보다 나빠지지 않는다) · 대가 = 그 끝의 판정 · `Ended` 가 많아야 이만큼 늦다 · S1 ① 이 끝 줄 `over_wait` 로 잰다 |
| `DROP_WAIT_MS` | 500 | `dragend`(move) 뒤 대상의 `dock_drop` 을 기다리는 시간 — 두 웹뷰의 invoke 순서는 보장되지 않는다 |
| `END_GRACE_MS` | 300 | `pointermove` 를 본 뒤 끝 신호를 기다리는 시간 — 넘으면 신호 유실로 친다 |
| `EARLY_SIGNAL_MS` | 500 | 장부가 모르는 `drag_id` 의 끝 신호를 그 시작을 기다리며 붙드는 시간 — 시작이 거절되면 곧바로 버린다 · S1 ⑧ 이 잰다 |
| `REORDER_SLACK_PX` | 15 | (5.7 · `hit.rs`) 순서 이동 구역 = 탭바 상자를 위아래로 이만큼 넓힌 것(CSS px) — §2-5 |
| `ENGINE_CANCEL_MS` | 15 | (5.7 · 웹뷰 `TabBar.tsx`) `dragstart` 사건 시각에서 이만큼 안에 온 `dragend(none)` = 엔진이 접은 끌기 — 실측 엔진 취소 2–4 ms · 가장 짧은 실제 OS 끌기 끝 +28 ms 사이 · S7 ⑦ |
| `REORDER_SLIDE_MS` | 150 | (5.7 · 웹뷰) 이웃이 비켜나는 · 놓은 탭이 제자리로 미끄러지는 시간(Lumino `transition: left 150ms` 와 같다) — §4-4 |
| `EDGE_SCROLL_PX` · `EDGE_SCROLL_STEP` | 24 · 10 | (5.7 · 리뷰 1 · 웹뷰 · 물러날 자리) 엔진 끌기 자동 스크롤이 출처 탭바에 걸리지 않을 때만 쓴다 — 순서 이동 모드에서 점이 탭바 상자 왼쪽 · 오른쪽 끝 이 폭(CSS px) 안이면 rAF 마다 이만큼(CSS px) 민다 · 첫 값 · S7 ⑬ (§1-4 「모드」 넘친 탭바) |

#### 시작 — `dock_drag_start{drag_id, view_id, hotspot, strip}`(동기 · `strip` = 5.7)

1. **거절(부수 효과 0)** — 부른 창(Tauri 가 넣어 준 창)이 그 탭의 주인이 아님 = `TabNotFound` · 장부에 세션이 있음 = `AlreadyDragging` · 그 `drag_id` 가 최근 끝난 id(`ended`)임 = `LateStart`(5.2 · 리뷰 R2-2 — 붙들었던 끝 신호를 이미 버린 끌기다: 열면 끝 신호 없는 세션이 `pointer_seen` 까지 남았다가 지금 커서에 새 창으로 끝난다. 거절이면 무변경 — 안전 쪽) · 레이아웃 락 오염 = `LayoutPoisoned` · 옛 세션의 표식이 아직 안 내려감 = `Dragging`(`lease_tab` — 장부가 끝을 낸 뒤 실행기가 `release_lease` 를 부르기 전의 짧은 틈 · 받아들인 남은 틈 — 사람 손으로는 그 틈에 새 끌기를 시작하지 못한다). `AlreadyDragging` 의 옛 세션이 `Live` · `AwaitDrop` · `Grace` 면 끝 신호를 잃은 것이라 곧바로 「마지막 점에 놓기」로 끝낸다(async 한 건 · §7 #12). (리뷰 4) 옛 세션이 `AwaitOver` 면 끝 신호는 이미 왔다 — 붙들기를 끊고 붙든 끝(그때 읽은 커서 · 사각형 · `now` · `last`)으로 곧바로 「마지막 점에 놓기」(시한이 닿은 것과 같다 · `over_wait = cut` · 「오는 중 `Over` 붙들기」). ★옛 세션이 `Applying` 이면 끝내지 않는다(5.1 · N2)★ — 적용 서비스가 도는 중이고(새 창 열기는 수백 ms) 그 끝은 `Applied` 가 정한다. 새 시작만 거절하고(debug · `reason = applying`) 옛 세션의 표식은 그 끝에서 내려간다(성공이면 모델이 이미 내렸다) — 그 뒤의 끌기는 받는다. 거절하면 실행기가 장부에 `Refused{drag_id}` 를 먹여 그 id 로 붙든 끝 신호를 버린다. 거절해도 OS 끌기는 이미 돌고 있다 — 다른 창은 세션을 몰라도 우리 MIME 을 받으므로(§1-4) 커서는 「이동」으로 보이지만, 그 놓기는 모르는 id 라 아무것도 바꾸지 않는다(§11-2 #21).
2. **확정(레이아웃 락 한 번)** — `lease_tab(view)`(실패면 위 사유) · 출처 배치 복제(`expect_source`) · 출처 창 label · 탭 목록 · 활성 · 「탭 하나뿐인 팝아웃인가」 · 영역 재료 스냅숏(모든 창 — §2-4 · 번호를 단다). ★세션 깃발을 이 락 안에서 세운다(5.1 · N8)★ — 장부 `Start` 에서 세우면 이 락을 놓은 뒤 `Start` 전에 끼어든 레이아웃 쓰기가 계기를 건너뛰어 장부가 옛 배치로 남는다. 그 쓰기의 스냅숏이 `Start` 보다 먼저 장부에 닿으면 장부는 그것을 `held_snap` 으로 들고 있다가 `Start` 뒤에 이어 먹는다(번호가 시작 스냅숏보다 클 때만). ★깃발은 불이 아니라 세션 세대(원자 `u64` · 0 = 없음)다(5.2 · 리뷰 R2-3)★ — 여기서 새 세대를 넣고, 장부가 끝을 낸 뒤 실행기가 **자기 세대일 때만** 0 으로 바꾼다(비교-교체). 무조건 내리면 그 사이 선 다음 세션의 깃발까지 내려 그 세션의 계기가 끊긴다. 락 드롭. ★(리뷰 3) 그 뒤 락 밖에서 출처 창 클라이언트 사각형(`inner_position` · `inner_size`) · 배율을 읽어 `Start` 에 싣는다★ — 장부가 `Drag.source_rect` 로 쥐고, 그 뒤 실행기가 먹이는 `Rects` 에 출처가 들어 있을 때마다(보이는 동안의 끝 신호 · 시한 콜백) 갈아 끼운다. 쓰는 곳은 「시한이 앞지른 끝」의 사각형 검사 하나다 — 숨긴 뒤에는 `Rects` 에 출처가 없다(실행기는 보이고 최소화 안 된 창만 읽는다 · `book.rs` `Rects`). 못 읽으면 없음. ★가정(기본값 · 메인 판단)★ — 숨기기는 자리를 바꾸지 않고(아래 4) 끄는 동안 사용자는 창을 옮길 수 없으므로(마우스가 OLE 끌기 고리 안) 숨긴 동안 쥔 사각형이 낡는 길은 다른 경로(버스 · LLM)가 숨긴 그 창을 옮기는 것뿐이다 — 드묾 · 낡은 사각형으로 판정하는 것을 받아들인다. 장부에는 창 이동 사건 경로가 없고 이것 때문에 두지 않는다.
3. **장부** — `Start` → 할 일: 출처가 탭 하나뿐인 팝아웃이면 `Hide(출처)`(★5.7 — 출처 영역에 `reorder` 가 서면 미룬다 · 아래 「순서 이동 · 도킹 모드 증거」★) · `Broadcast(Started{source_label, source_view, display_active})` · 창마다 `Regions`(숨긴 출처 제외). `display_active` = 출처가 그 창의 활성이면 `succession`, 아니면 없음(메인 마지막 탭이면 없음 — 메인은 빈 화면). 그다음 `held_snap` · 그 id 로 붙든 끝 신호를 도착 순서대로 먹는다 — 그 판정이 낸 `Apply` 는 이 동기 명령 안에서 돌지 않고 async 로 넘어간다(위 「명령의 실행 스레드」 불변식).
4. **실행** — 숨기기 = Tauri `hide()`(실측: 숨겨도 끌기 · `dragend` 가 이어진다 — 표본 2 · S1 ⑤). 실패하면 warn 하고 보인 채 간다(끌기는 이어진다). 숨기기는 자리 · 크기를 바꾸지 않아 자리 기록을 멈출 일이 없다. (5.7) 미룬 숨기기도 같은 할 일(`Hide`)이다. ★(리뷰 2) 어느 입력(`Over` — 동기 명령 · `Leave` 시한 — async)이 냈든 메인 스레드의 맞추기로 돈다★(위 「숨기기 · 보이기」 불변식 — 5.7 첫 판의 「낸 입력의 스레드에서」를 버렸다).

#### 순서 이동 · 도킹 모드 증거 (5.7 · PRD R23 · D13 · D15 · D18)

- **시작 측정 `strip`** — `dock_drag_start` 는 출처 웹뷰가 `dragstart` 안에서 잰 탭바(`StripStart{bar, heads, grab_x}` — §1-5)를 선택 칸으로 싣고, 장부는 그것을 `Drag.strip` 에 세션 내내 그대로 둔다. 순서 이동의 구역 · 자리는 이 얼린 기하로만 판정한다(§2-4 · §2-5) — 끄는 동안 머리가 접히거나(도킹 모드) 이웃이 비켜나는(순서 이동 모드) 화면 변화가 판정을 흔들지 않게. 출처 창 영역에 `reorder` 가 서는 조건 = §2-4(없으면 그 끌기는 5.6 그대로다 — 원래 창 탭바 = R21 끼울 자리).
- **모드는 장부가 정하지 않는다** — 순서 이동 · 도킹 모드는 출처 웹뷰의 그리기 상태이고, 그 웹뷰가 자기 `pick` 결과로 정한다(§1-4 「모드」). 장부가 아는 것은 「도킹 모드 증거」 하나이고, 판정(놓기)은 모드와 무관하게 보고 · 지금 영역으로 한다.
- **도킹 모드 증거 → 미룬 숨기기** — 출처가 탭 하나뿐인 팝아웃이고 시작 때 출처 영역에 `reorder` 가 섰으면 `Start` 는 숨기지 않고 `hide_pending` 을 세운다. 그 뒤 처음으로 아래 셋 중 하나가 서면(`Live` 일 때만 — `Applying` · `AwaitDrop` · `Grace` · (리뷰 4) `AwaitOver` · 끝 뒤가 아닐 때) `Hide(출처)` 를 내고 `source_hidden` 을 세운다 — 숨긴 출처 = 영역을 보내지 않음 · `last_over` 후보 아님(5.6 규칙 그대로). ★(리뷰 2) 출처 창의 지금 영역(`sent`)은 지우지 않고 끝까지 남긴다★ — 숨긴 동안의 스냅숏은 그 창 몫을 건너뛴다(지우지도 고치지도 않는다). 5.7 첫 판은 숨길 때 지웠는데, 그러면 아래 「시한이 앞지른 끝」에서 4c 가 판정할 영역이 없어 「W 없음 → 새 창」(재사용 — 팝아웃이 커서 자리로 뜀)이 됐다. 한 번 숨기면 그 세션에서 되보이지 않는다(끝의 되보이기는 그대로 — 무변경 끝 · 재사용 끝).
  - ① **출처 창의 그린 `Over` 가 `shown ≠ Reorder`** — 그린 보고 = 빈 보고(`rev` 0 · 지금 영역 없음 — 「커서 판정」)가 아니고 `rev` 가 그 창의 지금 `rev` 이며 그 영역이 막히지 않음(`blocked` 아님). ★영역 전 `Over{rev 0, 없음}` · 막힌 영역의 `Over{없음}` · 옛 `rev` 의 보고는 증거가 아니다(리뷰 1)★ — 앞 둘은 웹뷰가 판정하지 못한 것이고(그 웹뷰는 모드를 바꾸지 않는다 — §1-4 「모드」), 세션 사건이 영역보다 먼저 닿은 웹뷰는 영역 없이도 세션 id 만으로 `rev` 0 · `shown = 없음` 보고를 낸다(`dockDrag.ts` `evaluate` · 실측 3/10 — §8-1 ③). 그것을 치면 커서가 아직 탭바 안인데 팝아웃이 사라진다(AC26 · D18 어김). 옛 `rev` 보고는 장부가 지금 영역을 다시 보내므로(「영역」 순서) 지금 `rev` 보고가 곧 뒤따른다.
  - ② **출처 창의 `Left` 뒤 `LEAVE_GRACE_MS`(100) 동안 끝 신호도 출처 창의 새 `Over` 도 없음** — ★`Left` 는 곧바로 증거가 아니다(리뷰 1)★. 구역 안 Esc 도 출처 창에 관련 요소 없는 `dragleave` → `Left` 를 낸다(취소면 현재 대상에 떠남 — HTML 끌어 놓기 규칙 · `dockDrag.ts` `onDragLeave`). 그 자리에서 숨기면 끝에서 숨긴 출처가 W 후보에서 빠지고(`last_over`) 사각형도 없어(`last_point` 의 `Rects` — 보이는 창만) 「W 없음 + 커서가 출처 밖」 → R22 → 재사용으로 팝아웃이 커서 − hotspot 자리로 뛴다(AC25 · AC26 · D18 어김). 그래서 `Left` 에서 숨기기 시한(`ArmTimer{Leave}`)만 걸고, 시한이 닿았을 때 `Live` 가 아니면(끝 신호 — `drop` · 출처 끝 · `pointer_seen` — 가 먼저 왔다) 버리며, 그 전에 출처 창 `Over` 가 다시 오면 그 번호를 버리고 그 보고를 ① 로 가른다. 시한이 `Live` 에서 닿으면 그때 `Hide`. ★값 = `LEAVE_GRACE_MS` 그대로★ — 「마지막 점에 놓기」의 「아직 위」와 같은 경계라 두 판단이 같은 자리에서 갈린다: 떠남 → 끝 신호 간격이 100 ms 미만이면 숨기지 않았고 W = 출처가 아직 위라 4c(보이던 자리)로 간다 · 100 ms 이상이면 숨긴 뒤의 끝이 어차피 「W 를 떠남 → 새 창」 갈래다(5.6 과 같은 남은 틈 · §11-2 #34 · S1 ① 이 간격을 잰다 · 리뷰 2 · 3 — 출처 끝이 `last` 를 실으면 아래 「시한이 앞지른 끝」으로 숨기지 않은 것으로 쳐 쥔 출처 사각형으로 가른다). 근거 — Esc 의 떠남과 `dragend` 는 같은 엔진 사건 줄에서 잇달아 나온다고 본다(엔진이 끌기를 접을 때 `dragend` 까지 2–4 ms — §8-1 ② · Esc 의 떠남 → 끝 간격 자체는 미검 · S1 ①) — 셸 도착 간격은 invoke 흔들림(p99 ~16 · 최대 ~50 ms — §8-1)이 대부분이다. 창 밖(바탕화면 · 다른 앱)으로 나간 팝아웃은 100 ms 늦게 숨는다(받아들임). 우리 다른 창으로 나간 것은 ③ 이 먼저 숨긴다.
  - ③ **다른 창의 `Over`**(빈 보고여도) — 커서가 출처 창 밖이라는 OS 의 증거다(출처 창은 누름으로 맨 앞 — §11-2 #28).
- `hide_pending` 인 채 끝나면(구역 안에서 놓음 · 구역 안 Esc — 그 떠남의 시한은 끝 신호가 앞질러 버려진다 · 엔진 취소 · 보고 없는 끝) 숨긴 적이 없으니 되보이기도 없다. 5.7 첫 판의 같은 문장은 `Left` 를 곧바로 증거로 쳐 Esc 에서 거짓이었다(리뷰 1).
- ★**시한이 앞지른 끝(리뷰 2 · 리뷰 3 · 기본값 · 메인 판단)**★ — 구역 안 Esc 의 끝 신호가 `Left` + `LEAVE_GRACE_MS` 전에 셸에 닿았어도, 실행기가 세계(커서 · 사각형 — 게터라 이벤트 루프 왕복)를 읽는 사이 `Leave` 시한이 장부에 먼저 먹혀 출처를 숨길 수 있다. 5.7 첫 판은 `now` 를 세계 읽기 **뒤에** 찍어(`commands/dock.rs` `source_end` · `drop_at`) 그 끝이 시한 뒤로 보였고, 숨길 때 `sent` 를 지워 출처가 W 후보에서도 영역에서도 빠졌다 → 「W 없음 → 새 창」 → 재사용으로 팝아웃이 커서 자리로 뛰었다(AC25 · D18 어김). 셋으로 막는다 — ① 실행기는 끝 신호 · 시한 콜백의 `now` 를 세계 읽기 **전에** 찍는다(도착 시각) ② 「마지막 점에 놓기」에서 `Leave` 시한이 숨긴 출처는 그 숨김을 부른 `Left` 의 도착이 끝의 `now` 전 `LEAVE_GRACE_MS` 안이면(= 끝이 시한 전에 닿았다) · 또는 그 출처 끝이 `last` 를 실었으면(웹뷰가 자기 사건 시각으로 같은 유예를 이미 쟀다 — 「끝 신호」 · 셸 도착 흔들림과 무관) **숨기지 않은 것으로 친다** — W 후보(도착 순번으로 — 「마지막 Over 창」 · 리뷰 3) · 아직 위 · 영역 = 끝까지 남긴 `sent`(위). ★(리뷰 3) 사각형 검사는 그대로 한다 — 사각형 = 장부가 쥔 `source_rect`(「시작」 2 · 숨기기 직전까지 갈아 끼운 것)★. 숨긴 뒤라 실행기의 `Rects` 에는 그 창이 없지만 숨기기는 창을 옮기지 않으므로 그 사각형이 지금 자리다. 그래서 갈래는 보이는 출처와 같다(「마지막 점에 놓기」) — `last` 의 떠남 간격이 `CANCEL_LEAVE_MS` 안이면 사각형 검사 없이 「놓기 판정」(출처, `last`) · 아니면 커서가 그 사각형 안 = 「놓기 판정」(출처, 그 보고 — `last` 가 있으면 그것 · 빈 보고면 커서 판정) · 밖 = R22(재사용 — 팝아웃이 커서 자리로 · 5.6 과 같다) · `source_rect` 가 없으면(시작에 못 읽음) 「W 의 사각형이 없음」 갈래. 리뷰 2 판의 「사각형 검사 없이 · 「커서를 못 읽으면」 갈래와 같다」는 버렸다 — 실행기는 보이는 창의 사각형만 읽으므로, 같은 빠른 던지기(바탕화면)가 시한이 먼저 돌았으면 검사 없이 4c(제자리 무변경) · 늦게 돌았으면 검사로 R22 가 되어 시한 흔들림에 따라 결과가 갈렸다. 위 증거 ① · ③ 으로 숨긴 출처는 이 예외가 아니다(커서가 출처 캔버스 · 출처 밖이었다) ③ 그 끝이 출처를 제자리에 남기면(4c — 대상 = 출처 · 무변경 · 재사용) 끝의 되보이기가 다시 보인다(「끝」) — 늦게 돈 그 시한의 `Hide` 맞추기는 장부의 지금 값(보임)을 입혀 숨기지 않는다(「명령의 실행 스레드」). 창 밖(바탕화면 · 틈)으로 빠르게 나가 100 ms 안에 뗀 끌기는 사각형 검사로 R22 다 — 시한이 먼저 돌았든 아니든 같다(리뷰 3). 남는 틈 = 출처를 덮은 **다른 앱 창** 위로 나가 100 ms 안에 뗀 것만 그 팝아웃의 제자리 판정으로 끝난다 — 5.6 의 「아직 위」 남은 틈(§11-2 #1 — 다른 앱 창이 우리 창을 덮은 자리)과 같다. 리뷰 2 판은 바탕화면에 던진 것까지 이 틈과 같다고 적었는데 틀렸다 — 5.6 은 바탕화면에서 사각형 검사를 해 R22 였다.
- **왜 미루나** — 사용자 결정 Q20 은 「탭바 안 = 순수한 순서 이동 · 벗어나면 지금 그대로」이고, 탭 하나 팝아웃의 탭바 안 순서 이동은 무동작이지만(D18) 창은 보여야 한다(그 탭바가 보여야 그 안에 머무는 것이 보인다). 탭바를 벗어나는 순간이 5.6 의 「끌기 시작」에 해당한다.
- **메인 · 여러 탭 창은 장부가 할 일이 없다** — 떼어진 모습(머리 접기 · 승계 탭 보이기)은 웹뷰가 도킹 모드일 때만 그린다(§4-1). `Started` · `Display` 의 `display_active` 는 5.6 그대로 시작부터 싣는다(도킹 모드에서 쓴다).
- ★순서 이동 놓기는 다른 놓기와 같은 함수를 지난다★ — `drop` · 「마지막 점에 놓기」 · 커서 판정 · 유실이 다 「놓기 판정」 4c 로 간다. 그래서 Esc(구역 안) = 그 순간 보이던 자리 · 비정상 끝 = 마지막 점(PRD D16 · R13b).

#### 영역 — 언제 무엇을 보내나

- **보내는 때** — 시작 · 모델 계기(`TauriEvents::settled` — 쓰기 하나가 알림을 다 낸 뒤 한 번 · 버스 · invoke · 팝아웃 · 복원이 이 어댑터를 지난다 · §1-2 「알림 묶음 끝」) · 측정 계기(`report_dock_chrome` · `report_window_canvas` 처리 뒤) · `dock_pull`. 계기마다 실행기가 세션 깃발이 섰을 때만(원자 깃발 — 시작의 레이아웃 락 안에서 선다) 스냅숏을 떠 장부에 먹인다.
- **창마다 (영역, 기대값)이 마지막 보낸 것과 다를 때만** `rev` 를 올려 `emit_to(label, "dock:regions")`. `rev` 는 창별 · 세션 안 단조다. 장부는 그 `rev` 의 영역과 기대값(`TargetExpect::Seam{그 창 보이는 탭 배치, 그 창 모델 활성}` · `Tabs(그 창 탭 목록)`)을 함께 둔다 — 놓기 대조의 기준이다. ★기하가 같아도 기대값이 바뀌면 `rev` 를 올린다(5.1 · N4)★ — 대상 칸의 내용만 바뀐 경우(터미널 → 챗 · 같은 기하)다. 올리지 않으면 웹뷰는 옛 `rev` 로 놓고 장부는 옛 기대값을 실어 모델 판정 6 이 `TargetChanged` 로 막는다 — 사용자가 본 가이드 그대로인데 무변경이 된다. 올리면 웹뷰가 같은 영역을 새 `rev` 로 받아 그 뒤 놓기는 새 기대값으로 들어간다(남는 것은 영역이 닿기 전의 짧은 틈뿐이고 그것은 무변경이다). ★탭바 몫은 따로 센다(5.2 · 리뷰 B1)★ — `bar_rev` 는 창별 · 세션 안 단조이고 탭바 영역(`tab_bar` — 사각형 · 머리 · 끼울 x)이나 탭 목록 기대값(`Tabs`)이 바뀔 때만 오른다. 활성 전환은 보이는 탭 · 칸 · 창 활성만 바꾸므로 `rev` 는 오르고 `bar_rev` 는 그대로다 — 그래서 전환 중의 탭바 놓기가 선다(「놓기 판정」 4b).
- **스냅숏이 같이 판정하는 것** — 출처 View 가 없음 → `SourceGone` 끝 · 출처 배치 ≠ `expect_source` → `blocked = true`(모든 창 영역을 비우고 다시 보냄 — 웹뷰는 `dropEffect = 'none'` · 커서 「안 됨」 · R13c · AC16 · info 한 줄) · 출처 창 탭 목록 · 활성이 바뀌어 `display_active` 가 바뀜 → `Broadcast(Display)` · 스냅숏에 없는 창(닫힘)은 `sent` 를 지운다 — 그 창의 지금 영역은 없음이고 `seen` 은 남긴다(놓기 판정이 「대상 사라짐」을 알아야 한다). `Applying` 중이면 이 판정을 하지 않는다(아래 「적용 중」).
- **늦게 뜬 창 · 놓친 emit** — 웹뷰는 영역 없이 끌기를 만나면(`dragenter`) `dock_pull` 로 한 번 당긴다. 장부가 그 창 영역을 새로 만들어 돌려준다(바뀌었으면 `rev` +1).
- **순서 — 영역 · 세션 사건은 뒤바뀌어 닿을 수 있다(5.1 · C3 · F5)** — 할 일은 그것을 먹인 스레드에서 장부 락을 놓은 뒤 실행되므로(동기 명령 = 메인 · async 명령 · 시한 = 런타임 스레드 · 모델 계기 = 그 쓰기의 스레드 · 단 `Apply` 는 늘 async · 숨기기 · 보이기 · 그 뒤 포커스는 메인 스레드 맞추기 — 위 불변식 둘) `rev` 4 의 emit 이 `rev` 3 보다 먼저 닿을 수 있다. 웹뷰는 `drag_id` 별로 가장 큰 `rev` 만 쓰고 작은 것은 버린다 · `Started` 전에 온 영역도 받아 둔다(§1-4). 장부는 `Over` 의 `rev` 가 그 창의 지금 `rev` 보다 작으면 지금 영역을 같은 `rev` 로 다시 보낸다(emit 유실 회복 · debug) — 웹뷰가 옛 `rev` 에 머물러 모든 놓기가 `TargetChanged` 가 되는 것을 막는다.

#### 보인 것 보고 — `dock_drag_over`(동기)

- 대상 웹뷰는 `pick` 결과(`shown` — 출처 창의 순서 이동 구역이면 `Reorder{index}` · 5.7) · 쓰는 영역 `rev` · 그 창이 보이던 탭(`displayed` — 5.1 · C1)이 바뀔 때만 `Over{rev, bar_rev, displayed, shown, point, scroll}`(`scroll` = 그 판정 때 그 창 탭바 `scrollLeft` — 출처 창 순서 이동 판정과 (리뷰 2) 모든 창의 탭바 끼울 자리(R21) 판정에 쓴다 · 5.7 · 리뷰 1 · §2-5) 를, 창 밖으로 나갈 때 `Left` 를 보낸다(움직일 때마다가 아니다 · `drag_id` 를 모르면 — 영역 · 세션 전 — 보내지 않는다). 장부는 창별로 **마지막 Over**(보고 · 도착 순번 · 시각)와 **마지막 Left 의 도착 시각**을 따로 적는다(5.1 · F4 — Left 가 Over 를 지우지 않는다). 할 일은 없다 — 단 `rev` 가 그 창의 지금 `rev` 보다 작으면 지금 영역을 다시 보낸다(「영역」 순서) · (5.7) 그 보고가 도킹 모드 증거이고 `hide_pending` 이면 `Hide(출처)` · 출처 창 `Left` 면 숨기기 시한(「순서 이동 · 도킹 모드 증거」 ②).
- 「마지막 Over 창」 W = 마지막 Over 의 도착 순번이 가장 늦은 창 — 그 뒤에 Left 가 왔어도 W 다. 숨긴 출처 창은 후보가 아니다(리뷰 2 — 「시한이 앞지른 끝」은 예외). ★(리뷰 3) W 는 늘 이 도착 순번으로 고른다 — 출처 끝의 `last` 는 W 를 정하지 않고, 고른 W 가 출처일 때 그 마지막 `Over` 자리에 들어갈 뿐이다(아래 「끝 스냅숏」)★. 출처를 떠나 다른 창에 들어간 뒤 그 창 `Over` 가 출처의 마지막 `Over` 보다 늦게 닿았으면 그 창이 W 다. W 가 **아직 위** = W 의 마지막 Over 뒤에 Left 가 없거나, 있어도 그 Left 의 도착이 끝 신호 도착 전 `LEAVE_GRACE_MS` 안(리뷰 4 — 끝 도착 **뒤** 붙든 동안 닿은 Left 도 그 안으로 친다). ★(리뷰 4) 도착 순번에 기대는 대가 둘을 좁힌다★ — ① 출처 끝이 「실제로 떠났다」고 말하는데(`last` 간격 > `CANCEL_LEAVE_MS`) 도착 순번 W 가 출처이거나 없으면 다른 창의 `Over` 가 아직 오는 중일 수 있어 그 끝을 잠깐 붙든다(아래 「오는 중 `Over` 붙들기」) ② W 가 없는데(어느 창 `Over` 도 안 닿음) 출처 `last` 가 끝이 출처 위에서 났다고 말하면(떠남 없음 · 간격이 `CANCEL_LEAVE_MS` 안) W = 출처로 세운다(「마지막 점에 놓기」). 둘 다 `last` 가 다른 창이 고른 W 를 갈아 치우지는 않는다(리뷰 3 원칙 그대로). (5판처럼 창별 「마지막 보고 하나」만 두면 Left 가 Over 를 덮어 유예 안 Left 의 그 Over 를 되찾을 수 없다 — 창 안 Esc 의 떠남이 `dragend` 보다 먼저 닿으면 새 창이 되어 AC19 를 어긴다.)

#### 끝 신호 셋과 「마지막 점에 놓기」

```
우리 창 W 위에서 뗌   W: drop → dock_drop{rev, bar_rev, displayed, shown, point, scroll} 출처: dragend(move) → (이미 판정했으면 늦은 신호)
창 밖 · 틈 · 바탕화면   (drop 없음)                                     출처: dragend(none) → 마지막 점에 놓기
Esc(어디서든)           W: dragleave(취소 — 끝보다 먼저 올 수 있다)      출처: dragend(none){last — 리뷰 2} → 마지막 점에 놓기
끝 신호 유실            아무 창: 끝난 뒤 pointermove → dock_pointer_seen → END_GRACE 뒤 마지막 점에 놓기
엔진이 접은 끌기(5.7)   (drop 없음)                                     출처: dragend(none) · canceled → 무변경
멈춘 채 뗌(5.7)         (어느 창도 dragover 없음 — OLE 가 DragEnter 를 안 냄) 출처: dragend(none) → 마지막 점에 놓기 → 커서 판정
```

- **`dock_drop`(W)** → 아래 「놓기 판정」을 (W, 보고의 rev · bar_rev · displayed · shown · point · scroll)로. ★(리뷰 2) 이 칸들은 전부 그 창이 마지막으로 그린 판정 하나(`dockDrag.ts` `drawn`)에서 온다 — `scroll` 도 놓는 순간 다시 읽지 않는다★(`point` 가 이미 그렇다 — 다시 읽으면 짝이 갈라져 그린 자리와 다른 판정이 난다).
- **`dock_drag_end{effect}`(출처)** — 실행기가 그 순간 셸 커서(Tauri `cursor_position` — 물리 px · 못 읽으면 없음)와 보이고 최소화 안 된 우리 창들의 클라이언트 사각형(`inner_position` · `inner_size`) · 배율을 락 밖에서 읽어 싣는다 — ★`now` 는 그 읽기 **전에** 찍는다(리뷰 2 · 「시한이 앞지른 끝」 · `drop_at` · 시한 콜백도 같다)★.
  - `effect ≠ none`(받은 창이 있다) — 이미 판정했으면 늦은 신호. 아니면 `AwaitDrop{그 커서}` 로 두고 `DROP_WAIT_MS` 시한을 건다. 그 안에 `dock_drop` 이 오면 그것으로 판정, 시한이 지나면 「마지막 점에 놓기」(그 커서로).
  - `effect = none` → 「마지막 점에 놓기」 — ★`canceled` 면 판정 없이 무변경(5.7 — 아래 「엔진 취소 끝」)★ · ★(리뷰 4) 아래 「오는 중 `Over` 붙들기」 조건이면 `AwaitOver` 로 붙들고 그 끝에서★.
  - ★**끝 스냅숏 `last`(리뷰 2 · 리뷰 3 · 기본값 · 메인 판단)**★ — 출처 웹뷰는 자기 창이 끝에 아직 위였으면(떠남이 없었거나 그 떠남이 `dragend` 사건 시각 전 `LEAVE_GRACE_MS` 안 — §1-4 「출처」) 그 창이 마지막으로 **그린** 판정(`DrawnReport{rev, bar_rev, displayed, shown, point, scroll}` — 화면을 그린 바로 그 `evaluate` 의 값)과 (리뷰 3) 그 판정 뒤 떠남 → `dragend` 사건 시각 간격 `left_before_end_ms`(떠남 없음 = 없음)를 `last` 에 싣는다. ★(리뷰 3) `last` 는 W 를 고르지 않는다★ — 장부는 「마지막 점에 놓기」(`none` 끝 · 놓기 기다리기 시한)에서 W 를 5.6 그대로 도착 순번으로 고르고(「마지막 Over 창」 — 끝 순간 커서 · 사각형 검사도 그대로), **고른 W 가 출처이고** `last` 가 있고 출처를 숨기지 않았으면(「시한이 앞지른 끝」은 숨기지 않은 것으로 친다) 그 보고 = `last`(셸이 받은 출처 마지막 `Over` 대신) · 아직 위 = 참(출처 시계가 이미 쟀다 — 셸 도착 유예를 보지 않는다)로 둔다. W 가 다른 창이면 `last` 는 쓰지 않는다(끝 줄 `last = not_w`). (리뷰 4) W 가 **없으면** — 다른 창이 고른 W 가 없으니 — 간격이 엔진 접기일 때만 `last` 로 W = 출처를 세우고(「마지막 점에 놓기」 첫 갈래 · `lifted`), 간격이 실제 떠남이면 먼저 다른 창의 오는 중 `Over` 를 붙들어 기다린다(아래 「오는 중 `Over` 붙들기」 — W = 출처일 때도 같다). 그 뒤 갈래(사각형 · 커서 안 · 빈 보고면 커서 판정)는 그대로다. 리뷰 2 판은 `last` 가 있으면 W = 출처로 정했는데 틀렸다 — 출처를 떠나 다른 창 W2 에 들어가 가이드를 본 뒤 출처 시계로 떠남 100 ms 안에 Esc 를 누르면 출처는 여전히 `last` 를 싣고(떠남이 유예 안), W = 출처 · 커서가 출처 밖 → R22 새 창이 되어 W2 가 보인 가이드에 놓아야 할 Esc(PRD R12 · AC19)를 어겼다. 도착 순번은 그 끝을 맞게 W2 로 보낸다(W2 의 `Over` 가 출처 마지막 `Over` 뒤에 닿았다 · W2 는 Esc 떠남이 유예 안이라 아직 위 · 커서 W2 안). ★떠남 간격으로 엔진 접기와 실제 떠남을 가른다(리뷰 3)★ — W = 출처이고 `left_before_end_ms` 가 없거나 `CANCEL_LEAVE_MS` 안이면 그 떠남은 엔진이 끌기를 접으며 현재 대상(출처)에 낸 것이라(Esc · 받지 않는 자리의 떼기 — 취소면 현재 대상에 떠남) 끝은 출처 위에서 났다 — 사각형 검사 없이 「놓기 판정」(출처, `last`)(빈 보고면 `last` 의 점으로 커서 판정)으로 간다. 셸이 커서를 읽는 것은 끝 신호가 닿은 뒤라 그 사이 커서가 출처 밖으로 나가도(§11-2 #1 셋째) 사용자가 본 그 자리에 놓인다. 간격이 그보다 크면(실제로 출처를 떠난 지 100 ms 안의 끝 — 바탕화면 · 틈 · 다른 앱 위) 사각형 검사 그대로다(커서가 출처 밖 = R22). 다른 창 위의 Esc 떠남은 그 창에만 가므로 출처의 간격은 그 창으로 나간 실제 떠남이다 — 그래서 위 W2 끝에서 출처 간격은 엔진 접기로 읽히지 않는다(그 끝은 어차피 W = W2 라 `last` 를 쓰지 않는다). ★왜★ — `Over` 는 출처 웹뷰의 보고 줄(`dockDrag.ts` `sendOver` 의 `overChain`)을, 끝 신호는 `TabBar.tsx` `onHeadDragEnd` 의 따로 된 invoke 를 타 셸에 닿는 순서가 보장되지 않는다. 가장자리 스크롤 중이면 `scroll` 사건의 다시 판정이 새 자리를 그리고 `Over` 를 줄에 올린 직후 Esc 가 끝을 내는데, 끝이 먼저 닿으면 셸은 그 전 `Over` 로 4c 를 판정하고 그 보고의 (점, 스크롤) 짝은 늘 그 옛 자리를 내므로(§1-4 「모드」 넘친 탭바 ③) 그려진 것과 **다른 자리가 적용된다**(무변경도 아니다 — D16 · AC25 어김). 끝 신호가 그린 판정을 스스로 실으면 줄 순서와 무관하다 — 늦게 닿은 그 `Over` 는 끝난 id 라 버려진다. ★(리뷰 3) 출처의 「아직 위」는 W 가 출처일 때만 셸의 유예를 대신한다★ — 그 창 사건 시각(한 엔진 시계)으로 가른 것이라 셸 도착 흔들림과 무관하지만, 출처는 자기를 떠난 뒤 커서가 어느 창에 들어갔는지 모른다. 리뷰 2 판은 이것을 「셸의 도착 순번보다 낫다」로 적어 다른 창에서 출처로 돌아와 그 `Over` 가 아직 오는 중인 끝까지 출처로 보냈는데, 같은 규칙이 위 결함(출처를 떠나 다른 창 가이드 위 Esc → 새 창)을 냈다 — 그 돌아온 끝은 5.6 그대로 둔다(아래 남는 틈). ★도킹 모드 증거 ① · ③ 으로 숨긴 출처의 `last` 는 버린다★ — ① 로 숨긴 웹뷰는 떠남을 받지 못해 「안」으로 남는다(커서는 이미 그 밑의 다른 것 위다). `Leave` 시한으로 숨긴 출처는 반대로 `last` 가 있으면 숨기지 않은 것으로 친다(「도킹 모드 증거」 「시한이 앞지른 끝」 ②). `last` 가 없으면(7 단계 웹뷰 · 출처 창을 떠난 끝) 5.6 · 5.7 첫 판 그대로다 — 선택 칸이라 9b 가 9c 전 웹뷰와 선다. 남는 틈 = 유실(`pointer_seen` 시한)은 끝 신호가 없어 셸이 받은 마지막 `Over` 로 판정한다 — 그 짝이 낸 자리는 셸이 받은 마지막으로 그린 자리라 무변경도 오판도 아니지만, 그 뒤 늦은 그림과는 다를 수 있다(드묾 · 받아들임). (리뷰 3) 다른 창에서 출처로 돌아와 그 `Over` 가 셸에 닿기 전에 끝난 끌기는 도착 순번상 W = 그 다른 창이라 5.6 그대로다(커서가 그 창 밖이면 새 창 · 드묾 · §11-2 #36). 떠남 간격이 `CANCEL_LEAVE_MS` 안인 `last` 로 W 를 출처로 올리면 닫히지만 그 경계가 미검이라 S1 ① 뒤로 미룬다(미결). (리뷰 4) 그 반대 짝 — 출처를 떠나 다른 창에 가이드가 그려졌는데 그 창 `Over` 가 오는 중인 끝 — 은 아래 「오는 중 `Over` 붙들기」가, 출처 첫 그린 `Over` 가 오는 중이라 W 가 비는 끝은 「마지막 점에 놓기」의 W 없음 + `last` 갈래가 닫는다.
- **「마지막 점에 놓기」(PRD R12 · R13b)** — `blocked` 면 판정 없이 무변경(`SourceChanged` — 놓기 판정 1 과 같고 새 창도 만들지 않는다). 아니면 W = 마지막 Over 창(위 — 도착 순번 · 뒤에 Left 가 와도 W) · (리뷰 3) 그 W 가 출처이고 `last` 가 쓰이면 그 보고 = `last` · 아직 위(위 「끝 스냅숏」 — `last` 는 W 를 바꾸지 않는다).
  - ★(리뷰 4 · 기본값 · 메인 판단) **W 없음**이고 출처 `last` 가 쓰일 수 있고(숨긴 출처 ① · ③ 이 아님) 그 떠남 간격이 없거나 `CANCEL_LEAVE_MS` 안 → W = 출처 · 그 보고 = `last` · 아직 위★ — 바로 아래 갈래로 간다. 출처 첫 그린 `Over` 가 아직 오는 중일 때 끝이 닿으면 W 가 비어 `last` 가 버려지고 「W 없음 → 커서 판정(출처, 끝 순간 커서)」이 시작 때 스크롤(`strip.scroll`)로 다시 골라, 가장자리 스크롤 뒤면 그린 것과 **다른 순서 이동 자리**가 적용됐다(리뷰 3 판의 회귀). 다른 창 `Over` 가 하나도 없으니 「`last` 가 다른 창이 고른 W 를 갈아 치운다」(리뷰 2 판 결함)가 아니다. 간격이 `CANCEL_LEAVE_MS` 보다 크면 출처를 실제로 떠난 끝이라 이 갈래가 아니다(먼저 「오는 중 `Over` 붙들기」 · 그 뒤 W 없음 갈래 그대로 — `last = not_w`). 남는 위험은 위 갈래와 같은 경계 하나 — 출처를 떠나 `CANCEL_LEAVE_MS` 안에 창 밖에서 뗀 빠른 던지기가 출처 `last` 자리로 간다(S1 ① · S7 ⑬ ⓓ 가 경계를 잰다).
  - (리뷰 3) W = 출처 · `last` 를 쓰고 그 떠남 간격이 없거나 `CANCEL_LEAVE_MS` 안 → 사각형 검사 없이 「놓기 판정」(출처, `last`) — 끝이 출처 위에서 났다(위 「끝 스냅숏」) · 그 보고가 빈 보고면 「커서 판정」(출처, `last` 의 점).
  - W 가 아직 위이고 W 의 사각형이 있고(「시한이 앞지른 끝」으로 숨기지 않은 것으로 친 출처 = 쥔 `source_rect` — 리뷰 3) **커서가 그 안** → 「놓기 판정」을 (W, W 의 마지막 Over 보고)로 — ★그 보고가 빈 보고면 「커서 판정」(W, 끝 순간 커서)(5.7 · 결함 ③)★.
  - W 가 아직 위인데 **W 의 사각형이 없음**(소멸 · 숨김 · 최소화 · 못 읽음 — (리뷰 3) 「시한이 앞지른 끝」의 출처는 쥔 `source_rect` 도 없을 때만) → W 의 마지막 보고가 가이드를 보였으면 **무변경**(`TargetChanged` — 대상이 사라짐 · Q18), 「가이드 없음」이었으면 커서 자리에 새 창(R22).
  - ★**W 없음**(이 세션에 숨기지 않은 창의 `Over` 가 하나도 없음 · (리뷰 4) 위 W 없음 + `last` 갈래가 아님)이고 커서가 **출처 창 사각형 안** → 「커서 판정」(출처, 끝 순간 커서)(5.7 · 결함 ①)★ — 멈춘 채 떼기 · 짧은 톡이다(OLE 가 다음 마우스 메시지 전에는 `DragEnter` 를 내지 않는다 — 실측 2026-10-17). 출처 창은 누르는 순간 앞으로 와 그 사각형 안의 맨 위 창이다(§11-2 #28).
  - 그 밖(W 없음 + 커서가 출처 창 밖 · W 를 떠남 · 커서가 W 사각형 밖) → 커서 자리에 새 창(R22 — 5.6 그대로 · 커서 판정을 그 밖의 우리 창으로 넓히지 않는 까닭 = 「커서 판정」). 그 자리에는 사용자에게 보인 가이드가 없다 — 가이드는 창 안에만 그려지고 창을 떠나면 지운다. W 가 아직 위인데 커서가 그 밖인 것은 창 밖으로 빠르게 나가 떼어 떠남 보고가 아직 오는 중이거나(그대로 새 창이 맞다 — 무변경이면 던진 탭이 원래 창으로 돌아가 「되돌리기 없음」(Q8)과 어긋난다) 끝 신호 뒤 커서가 움직인 경우다(5판과 같다 · §11-2 #1).
  - **커서를 못 읽으면**(잠금 화면 · 보안 데스크톱 — AC21 · 5.1 · N3) — 「마지막으로 알려진 점」 = W 의 마지막 Over 점. W 가 아직 위면 「놓기 판정」(W, 그 보고)(사각형 검사 없음 — 유실과 같다). 아니면 「가이드 없음」으로 보고 그 점을 W 의 화면 좌표(안쪽 원점 + 점 × 배율)로 옮긴 자리에 새 창 · W 가 없거나 W 의 사각형도 못 읽으면 기본 자리(`pos = None`).
  - ★Esc 와 떼기를 가를 필요가 없는 이유★ — 우리 창 위에서 버튼으로 떼면 그 창의 `drop` 이 처리하므로(`dropEffect = move`), `none` 끝이 우리 창 위를 가리키면 그것은 Esc(또는 시스템 취소)다. 그리고 Esc 와 떼기는 같은 「그 자리에 놓기」라(PRD R12) 가를 것이 없다 — 남는 일은 「지금 커서가 어느 우리 창의 보인 가이드 위인가」이고, 그것을 마지막 보고 + 커서 사각형 검사가 답한다. Esc 취소는 대상 창에 `dragleave` 를 먼저 줄 수 있고(HTML 끌어 놓기 규칙 — 취소면 현재 대상에 떠남) 그 보고가 `dragend` 보다 셸에 먼저 닿을 수 있어 `LEAVE_GRACE_MS` 를 둔다. (리뷰 3) 출처 위의 Esc 는 웹뷰가 그 떠남 간격을 싣고(`last.left_before_end_ms` — `CANCEL_LEAVE_MS` 안) 셸은 사각형 검사 없이 그 자리로 간다 — 다른 창 위의 Esc 는 그 떠남이 그 창(현재 대상)에만 가므로 도착 순번 · 유예 · 사각형 그대로다.
  - 남는 오판 하나 — W 를 덮은 **다른 앱 창** 위로 나가 100ms 안에 떼면 그 밑 W 의 마지막 가이드로 붙는다(사용자가 그 가이드를 직전에 봤다 · §11-2 #1).
- ★**오는 중 `Over` 붙들기(리뷰 4 · 기본값 · 메인 판단 · `AwaitOver`)**★
  - **왜** — 다른 창 X 가 가이드를 그리고 그 `Over` 를 줄에 올린 직후 Esc(또는 받지 않는 자리의 떼기)가 나면, 출처 끝(async)이 X 의 `Over`(동기)보다 먼저 셸에 닿을 수 있다(두 웹뷰의 invoke 는 따로라 도착 순서가 보장되지 않는다). 그러면 도착 순번상 W = 출처(또는 없음)이고, 출처 `last` 는 실제 떠남(간격 > `CANCEL_LEAVE_MS`)이라 사각형 검사를 하는데 커서는 X 안 = 출처 밖 → R22 새 창이 된다 — 사용자가 본 X 의 가이드가 아니다(PRD R12 · AC19 어김). 리뷰 3 판은 이 경주를 놓쳤다.
  - **조건** — 출처 `none` 끝(`canceled` 아님 · `blocked` 아님)이 `Live` · `Grace` 에서 닿았고, 출처 `last` 가 쓰일 수 있고(숨긴 출처 ① · ③ 이 아님) 그 `left_before_end_ms` > `CANCEL_LEAVE_MS` 이며, 도착 순번 W(「마지막 Over 창」)가 출처이거나 없다 — 곧 「출처는 끝 전에 떠났다고 말하는데 출처 마지막 `Over` 뒤 다른 창의 `Over` 는 하나도 안 닿았다」. 붙들지 않는 끝 = `last` 없음(떠남이 `LEAVE_GRACE_MS` 보다 앞섰다 — 그 사이 다른 창 `Over` 는 실측 최대 흔들림 ~50 ms 안에 닿았다 · 7 단계 웹뷰) · 간격이 `CANCEL_LEAVE_MS` 안(끝이 출처 위 — 기다릴 다른 창이 없다) · 놓기 기다리기 시한 · 유실 · 새 시작으로 들어온 「마지막 점에 놓기」(앞 둘은 이미 500 · 300 ms 를 기다렸고 뒤 둘은 출처 끝이 없다). 붙든 이른 끝 신호를 시작에 이어 먹일 때도 같은 조건을 본다.
  - **붙들기** — phase = `AwaitOver{at, end, from: None}` · 할 일 = `ArmTimer{OverWait, gen, OVER_WAIT_MS}` 하나뿐(판정 · `Ended` · 되보이기 없음). `end` = 그 끝 신호가 실은 것 그대로(커서 · 사각형 · `now` — 실행기가 세계 읽기 전에 찍은 도착 시각 · `last`). ★판정은 늘 이 값으로 한다 — 시한 콜백 · 뒤에 온 입력이 읽은 커서 · 사각형은 쓰지 않는다★(뗀 순간의 커서다 — 붙드는 동안 움직인 커서로 판정하면 §11-2 #1 셋째 틈이 넓어진다).
  - **붙든 동안** — 다른 창 X 의 `Over`(빈 보고여도) = 평소대로 적고(`seen` · 도착 순번) · ★X 의 끝 때 사각형이 붙든 끝 커서를 품을 때만 `from = X`★(리뷰 5 · 기본값 · 메인 판단 — 출처 → X → Y 끌기에서 X 의 늦은 `Over` · `Left` 가 Y 의 오는 중 `Over` 보다 먼저 닿아 붙들기를 풀면 끝 커서가 Y 안이라 새 창이 된다 · 커서를 품은 창의 줄만 기다린다) · 아직 판정하지 않는다(같은 창 줄의 뒤 `Over` — 바뀐 가이드 — 가 따라올 수 있다). `Left{from}` = 적고 **곧바로 푼다** — 같은 창의 `Over` · `Left` 는 한 줄로 낸 순서대로 닿으므로(`dockDrag.ts` `overChain`) 그 `Left` 가 닿았으면 그 창이 앞서 낸 `Over` 는 다 닿았다(Esc 면 그 떠남이 현재 대상 X 에 온다). 끝 커서를 품지 않는 창의 `Over` · `Left` = 적기만(풀지 않는다 — 시한까지 기다린다). ★끝 커서를 품은 우리 창이 둘 이상(겹친 창)이면 `Left` 로 풀지 않고 시한까지 기다린 뒤 도착 순번으로 W 를 고른다★(리뷰 6 · 기본값 · 메인 판단 — 어느 창이 앞인지 셸은 모른다(맨 위 창 판정 함수 없음 · §1-6) · 뒤 창 X 의 늦은 `Left` 로 풀면 앞 창 Y 의 오는 중 `Over` 를 놓친다 · 대가 = 겹친 창 위 끝은 최대 `OVER_WAIT_MS` 늦어진다). 끝 커서가 우리 창 어느 사각형에도 없으면 붙들지 않는다(창 밖 = R22 · 기다릴 대상이 없다). ★출처 `Over` = 적지 않는다 — 붙들기 전의 출처 마지막 `Over`(보고 · 도착 순번)를 그대로 둔다(9b-1 코드 리뷰)★ — 적으면 그 늦은 `Over` 가 도착 순번상 가장 늦어 `from` 없는 풀기(겹친 창 · 시한)가 W 를 출처로 되돌리고(사용자가 본 다른 창 가이드 대신 R22 — AC19 어김), 끝 스냅숏 대조 `last_differs` 도 그 늦은 그림과 견주게 된다. `from` 이 아닌 창의 `Left` = 적기만. 붙든 동안 받은 `Over` 는 `Live` 가 아니라 숨기기 증거(① · ③)가 되지 않고, `Leave` 시한도 `Live` 가 아니라 버려진다(「도킹 모드 증거」 그대로). 스냅숏 = 평소대로(영역 · `blocked` · `SourceGone` — 판정은 풀 때의 지금 영역으로). `dock_drop` · 같은 id 의 둘째 `SourceEnd` = 늦은 신호로 버린다(debug — 출처 끝이 `none` 이라 받은 창이 없다). `pointer_seen` = 버림. `Destroyed{출처}` = 그냥 끝(`SourceDestroyed` — 다른 단계와 같다). 지금 번호 · `gen` 이 아닌 `Timeout{OverWait}`(풀린 뒤 닿은 것 포함) = 버림.
  - **풀기** — `Left{from}` · `Timeout{OverWait}` · 새 시작(「시작」 1 — 새 끌기는 `AlreadyDragging` 거절) 중 먼저 온 것 → 붙든 `end` 로 「마지막 점에 놓기」를 그대로 돈다. W = `from` 이 있으면 그 창(마지막 보고 · `last` 는 안 씀 — `not_w` · 아직 위 — 그 창 떠남은 끝 도착 뒤라 유예 안 · 사각형 = 끝 때 읽은 `Rects`) → 커서가 그 안이면 「놓기 판정」(X, 그 보고) = 사용자가 본 그 가이드. ★붙든 동안 닿은 출처 `Over` 는 W 를 출처로 되돌리지 않는다★ — 출처 줄의 늦은 `Over` 는 `last` 보다 옛 그림이고 출처는 떠났다고 말했다. `from` 이 없으면 그때의 도착 순번으로 — 붙들기 없이 판정한 것과 같다. 풀기가 `Left` · 새 시작(동기 명령 — 메인 스레드)에서 나도 `Apply` 는 늘 spawn 이라 교착이 없다(「명령의 실행 스레드」 불변식).
  - **세션 수명** — 붙든 동안 세션은 살아 있다(표식 · 세션 깃발 · 영역 · `seen` 그대로 · `ended` 에 아직 없음). 그래서 다른 창 `Over` 를 모르는 id 로 버리지 않고, 새 시작은 `AlreadyDragging` 이다(끝 뒤 50 ms 안에 새 끌기 문턱을 넘기는 사람 손으로는 드물다). 대가 = 그 끝의 판정 · `Ended`(떼어진 모습 거두기) · 숨긴 출처 되보이기가 많아야 `OVER_WAIT_MS` 늦다 — 그 끝은 대개 R22 새 창(창 열기 수백 ms)이라 눈에 띄지 않는다.
  - **남는 틈** — X 의 `Over` 가 `OVER_WAIT_MS` 를 넘겨 닿으면(같은 창 줄에서 앞 invoke 뒤에 섰을 때 등) 붙들기 없는 판정 그대로다(새 창 — 리뷰 3 판과 같다 · 나빠지지 않는다). X 가 떠남을 보내지 않는 끝은 시한까지 기다린다. X 의 `Over` 가 줄로 둘 이상인데 그 `Left` 가 시한 뒤면 시한 때까지 닿은 보고로 판정한다(그 뒤 그림과 다를 수 있다 — 드묾).
- **`dock_pointer_seen`** — 끄는 동안 웹뷰에는 포인터 사건이 오지 않는다(끌기가 대신한다 — 출처는 시작 때 `pointercancel`). 그래서 세션 중 어느 창이든 버튼을 뗀 마우스 이동 · 새 누름을 보면 끌기는 이미 끝났다(웹뷰 쪽 거름 = §1-4 · S1 ③). `Live` 면 `Grace` 로 두고 `END_GRACE_MS` 시한 — 그 안에 `drop` · `dragend` 가 오면 평소대로, 지나면 신호 유실: W 가 아직 위면 (W, 그 마지막 Over 보고)로 「놓기 판정」(커서는 그 뒤 움직였으므로 사각형 검사를 하지 않는다 · W 가 사라졌으면 판정이 「지금 영역 없음」으로 무변경), 아니면 W 에서 본 가이드가 없으므로 지금 커서 자리에 새 창(커서를 못 읽으면 위 「커서를 못 읽으면」).
- **그 밖의 끝** — 출처 창 `Destroyed` = `SourceDestroyed` · 스냅숏에 출처 View 없음 = `SourceGone` — 둘 다 **그냥 끝**(무변경 · 되보이기 없음 · PRD R13b 예외 · AC16). 단 `Applying` 중이면 적기만 한다(아래 「적용 중」). 최근 끝난 세션의 늦은 끝 신호 · OS 끌기는 버려진다.
- **엔진 취소 끝(5.7 · 결함 ②)** — `dock_drag_end{canceled: true}` 는 출처 웹뷰가 「`dragstart` 사건 시각 + `ENGINE_CANCEL_MS` 안의 `none` 끝」으로 가른 것이다(§1-4 「출처」). `Live` · `Grace` 면 판정 없이 무변경(`EngineCanceled` · `end_by = engine_cancel`) — OS 끌기가 열리지 않았거나 문턱 직후 곧바로 끝나 커서가 누른 자리 그대로이고, 사용자는 아무것도 보지 못했다. 붙든 이른 신호로 와도 같다(시작에 이어 먹어 무변경). `canceled` 가 없거나 거짓이면 5.6 그대로다(7 단계 웹뷰와 선다).

#### 커서 판정 (5.7 — 보고 없는 끝 · 빈 보고 · 결함 ① ③)

- **빈 보고** = `shown = 가이드 없음` 이고 판정할 영역이 없던 보고 — `rev` 0(웹뷰가 영역을 받기 전) 이거나 장부에 그 창(W)의 지금 영역이 없음(스냅숏에서 빠짐 · 소멸). 웹뷰가 아무것도 그리지 못했다는 뜻이라 「사용자가 가이드 없음을 봤다」의 증거가 못 된다. 5.6 은 이것을 보인 것 「가이드 없음」으로 대조해, 그 자리가 지금 가이드면 무변경으로 끝냈다(실측 — 이웃 창 6/20 · 같은 창 25 ms 3/10 · 전부 `rev` 0 · `shown` 없음). ★0 이 아닌 옛 `rev` 의 「가이드 없음」은 빈 보고가 아니다(리뷰 1 — 좁힘)★ — 그 보고는 실제 영역으로 판정해 「가이드 없음」을 그렸으므로 보고다(놓기 판정 3 — 지금 판정도 없음이면 새 창 · 지금 가이드면 무변경). 가이드를 그린 보고(`shown ≠ 없음`)도 옛 `rev` 여도 빈 보고가 아니다 — 본 가이드가 바뀐 것이라 5.6 그대로 무변경(Q18).
- **판정** — (창 T, 점 p, 스크롤 s, 보이던 탭 v) 하나로: `judged = pick(T 의 지금 영역, p, s, source_view)`(T 의 지금 영역이 없으면 「가이드 없음」) 뒤 **캔버스 거름** — `judged` 가 이음새인데 `v` ≠ 그 영역의 `view_id` 면 「가이드 없음」(웹뷰 그리기와 같은 거름 — §1-4 「판정 · 그리기」 · 탭바 · 순서 이동은 거르지 않는다). `blocked` 는 먼저 걸린다(무변경 · 놓기 판정 1).
  - ★**`v` — 보고가 있으면 그 `displayed` · 출처 보고가 없으면 출처 캔버스가 모델 활성(그 영역 기대값의 `active`)을 보인 것으로 친다(리뷰 1)**★ — 보고 없는 끌기는 출처 웹뷰가 한 번도 판정하지 못해 순서 이동 모습 그대로였다(떼어진 표시 없음 — 캔버스 = 모델 활성 · §1-4 「모드」). 셸 영역의 `view_id` 는 끄는 탭이 활성이면 승계 탭(`regions::visible_tab`)이라, 거르지 않으면 활성 탭을 캔버스 위 1/3 로 톡 끌어 보고 없이 뗀 끌기가 화면에 한 번도 안 보인 승계 탭의 칸에 붙는다(PRD R15 「지금 보이는 탭」 · 「보이는 자리 = 적용하는 자리」 어김). 거르면 그 자리는 가이드 없음 → R22 새 창이다(Q21 의 「슬롯 가운데 · 그 밖 = 새 창」과 같은 결과 — 끄는 탭이 제 칸 위에 놓였다). 끄는 탭이 활성이 아니었으면 모델 활성 = `view_id` 라 거름이 걸리지 않는다.
  - **`s`** — 그 보고의 `scroll` · 보고가 없거나 그 칸이 없으면 영역마다 잰 때의 스크롤 — 순서 이동 = `strip.scroll`(시작 때 — 그 창에 끌기 사건이 없었으니 엔진 자동 스크롤도 우리 끝 띠 스크롤도 돌지 않았다 · §2-5) · (리뷰 2) 탭바 끼울 자리 = 그 창 크롬의 `scroll`(`TabBarRegion.scroll` · 없으면 0 — §1-5).
  - `judged` = 가이드 없음 → **R22 새 창**(자리 = 끝 순간 커서 · 없으면 그 점의 화면 자리).
  - `judged` = `Reorder{i}`(T = 출처 · 구역 안) → **순서 이동 적용** `TabBar{T, i}` · 기대값 `Tabs`(T 의 지금 영역의 기대값) · `Reveal` — 구역에는 가이드가 없으므로 「못 본 가이드」 문제가 없다.
  - `judged` = 이음새 · 끼울 자리(`Seam` · `TabBar`) → ★**그 자리에 적용(사용자 결정 Q21 · 2026-10-17 · §11-1b ①)**★(이음새 기대값 = T 의 지금 영역의 `Seam{배치, 창 활성}` · 탭바 = `Tabs` · 보고가 없으므로 웹뷰 `displayed` 대조는 없고 모델 판정 6 이 같은 락 안에서 활성 · 배치를 본다). 근거(사용자) = 창 안으로 옮겨 온 것 자체가 마우스 움직임이라 뗀 순간 커서가 있는 곳이 목적지다. 고르지 않은 안 = 무변경(`TargetChanged`). 이 갈래는 장부의 한 팔(`Verdict` 하나)에 둔다 — 잠정이 아니라 확정이다.
  - ★Q21 의 범위 = 빈 보고 · 보고 없는 끝뿐★ — 지금 영역으로 그린 보고가 있으면 「놓기 판정」 3 · 4 그대로다(가이드를 보인 뒤 판정이 다르거나 · 「가이드 없음」을 보인 자리가 지금 가이드면 무변경 — PRD R13c · Q18). 사용자가 본 것이 있을 때만 「보지 못한 결과를 만들지 않는다」가 걸린다.
- **T 와 p — 어디서 부르나**
  - 「마지막 점에 놓기」에서 커서가 곧 놓은 자리일 때(`rect_check` — 출처 끝 `none` · 놓기 기다리기 시한) — T = 위 「마지막 점에 놓기」 갈래가 고른 W 또는 출처 · p = 끝 순간 커서를 T 의 뷰포트 CSS 점으로(`(커서 − 안쪽 원점) ÷ 배율` — `screen_point` 의 역 · 실행기가 읽은 `Rects` 그대로 · 「시한이 앞지른 끝」의 출처는 쥔 `source_rect` — 리뷰 3) · s · v = T 가 W 면 W 의 마지막 보고 것 · 출처(W 없음)면 `strip.scroll` · 모델 활성.
  - 「놓기 판정」에 빈 보고가 온 때(`drop` · 붙든 이른 놓기 · 커서를 못 읽은 「마지막 점」 · 유실 — W 가 아직 위인 갈래 · (리뷰 3) 떠남 간격이 엔진 접기인 출처 `last` — (리뷰 4) W 없음으로 세운 것 포함) — T = 그 보고의 창 · p · s · v = 그 보고의 점 · `scroll` · `displayed`(놓기면 놓은 점 그대로).
  - ★유실(`pointer_seen` 시한) · 새 시작은 「W 없음 → 출처 커서 판정」을 하지 않는다★ — 커서가 끝 뒤에 움직였다(`rect_check = false` — 5.6 그대로 새 창). ★단 W 가 아직 위이고 그 마지막 보고가 빈 보고면 바로 위 갈래로 커서 판정(W, 그 보고의 점)에 닿는다★ — 끝 순간 커서가 아니라 그 보고의 점이라 끝 뒤 움직임과 무관하다(리뷰 1 — §9 9b 행과 맞춤). ★예외 — 놓기 기다리기(`AwaitDrop`) 중의 새 시작은 「W 없음 → 출처 커서 판정」에 닿는다(`rect_check = true` · 기본값 · 메인 판단 · 9b-1 코드 리뷰)★ — 그 세션은 끝 신호(`dragend(move)`)가 이미 왔고 「시작」 1 이 그 끝 신호 때 읽은 커서로 놓는다. 그 커서는 끝 뒤에 움직인 커서가 아니라 뗀 순간의 커서라 놓기 기다리기 시한과 같은 판정이 맞다(새 시작은 그 시한을 앞당길 뿐이다).
- ★**판정할 창을 원래 창 · W 둘로 좁힌 까닭(D21)**★ — 셸은 커서 밑 맨 위 창을 모른다(OS 함수 0 — §1-6). `Rects` 는 우리 창 사각형뿐이라, 다른 앱 창이 우리 창 X 를 덮은 자리에서 뗀 것(그 앱은 받지 않아 `none` 끝)과 X 위에서 멈춘 채 뗀 것이 똑같이 「보고 없음 + 커서가 X 사각형 안」으로 보인다. 아무 우리 창으로나 판정하면 앞의 것을 X 에 붙이거나 무변경으로 끝내 PRD AC8(다른 앱 창 위 = 새 창)을 어긴다. W(그 끌기를 본 창 · 아직 위)와 출처(누름으로 맨 앞)만은 그 자리의 맨 위가 그 창이라는 증거가 있다. 대가 = 이웃 창이 그 끌기를 한 번도 보지 못한 채(`Over` 0) 그 위에서 뗀 끌기는 5.6 그대로 새 창이다(§11-2 #29). ★실측 4/4(이웃 창 멈춘 채 떼기 = 새 창)는 하네스 탓으로 의심된다★ — 하네스가 커서를 `SendInput` 이동 한 번으로 이웃 창 탭바까지 옮겨, OLE 가 그 창에 `DragEnter` 를 낼 다음 마우스 메시지가 없었다. 사람 손은 이웃 창까지 끊김 없이 움직이므로 그 창이 `dragover` 를 받아 W 가 되고, 그 보고가 빈 보고여도 W 의 커서 판정으로 붙는다(Q21). S7 ⑪ 이 끊김 없는 이동으로 다시 잰다(합격 = 이웃 창에 붙음). 메인 제안(2026-10-17)의 「겹친 우리 창이 둘이면 무변경」 갈래는 이 좁힘으로 필요 없어졌다 — W 가 있으면 W, 없으면 출처, 그 밖은 새 창이라 겹침을 가를 일이 없다.
- **끝 줄** — 커서 판정으로 끝났으면 `unseen = true`(S7 ⑥ · 현장 셈).

#### 이른 끝 신호 — 시작보다 먼저 온 `drop` · 출처 끝 (5.1 · C3)

- **왜** — 시작(`dock_drag_start`)은 동기 명령(메인 스레드)이고 끝 신호(`dock_drop` · `dock_drag_end`)는 async 라 셸에 닿는 순서가 보장되지 않는다. 문턱 직후 곧바로 떼는 짧은 끌기(같은 창 · 이웃 창)는 끝 신호가 시작보다 먼저 장부에 올 수 있다 — 시작을 사건 디스패치 뒤로 미룬 것(§1-4 「다른 끌기 리스너」)이 이 틈을 조금 넓힌다. 5판은 그 신호를 모르는 id 로 버려 세션이 끝 신호 없이 남았다(떼어진 탭 · 표식이 `pointer_seen` 까지 남음).
- **규칙** — 장부가 모르는 `drag_id` 이고 최근 끝난 id 도 아니면 그 id 의 끝 신호를 도착 순서대로 `early` 에 붙들고 `EARLY_SIGNAL_MS`(500) 시한을 건다. 한 번에 한 id 다 — 다른 모르는 id 의 끝 신호가 오면 옛 것은 버린다. 그 안에 같은 id 의 `Start` 가 오면 시작 할 일을 낸 뒤 붙든 신호를 도착 순서대로 먹인다. 시작이 거절되면(`Refused`) · 시한이 지나면 버린다(debug). ★버린 id(시한 · 밀려남)는 `ended` 에 적는다(5.2 · 리뷰 R2-2)★ — 그 뒤에 그 id 의 시작이 오면 `LateStart` 로 거절된다(「시작」 1). 적지 않으면 그 시작이 끝 신호 없는 세션을 열어 `pointer_seen` 뒤 지금 커서에 새 창을 띄운다. 세션이 살아 있는 동안 온 모르는 id 의 끝 신호도 같다 — 그 시작은 `AlreadyDragging` 으로 거절되므로 곧 버려진다.
- **붙든 놓기의 판정** — 시작 전의 웹뷰는 영역이 없어 「가이드 없음」 · `rev` 0 을 싣는다 = ★빈 보고(5.7)★ → 「커서 판정」(그 창, 놓은 점): 출처 탭바 구역이면 그 자리 순서 이동 · 지금 판정도 「가이드 없음」이면 새 창 · 가이드 자리면 그 자리에 적용(Q21). 5.6 은 가이드 자리면 무변경이었다(사용자는 가이드를 못 봤다 — 그 판단이 §11-1b ① 의 고르지 않은 안이다).

#### 적용 중 (`Applying` — 5.1 · F1)

- **스냅숏 · 소멸은 적기만 한다.** `Applying` 동안 스냅숏은 번호만 받아 두고(영역 · `Display` · 끝 판정 없음), `Destroyed{출처}` 는 `source_destroyed` 만 세운다. 끝은 `Applied` 하나가 정한다 — 성공이면 `applied`(대상 `set_focus` — R11b · AC3), 실패면 무변경(사유 = 그 `DockError`).
- **왜** — 성공한 Seam 은 출처 View 를 지우고 알림을 낸다. 그 알림의 계기가 먹인 스냅숏(출처 View 없음)이나, 빈 출처 팝아웃을 `host.close` 한 뒤 메인 스레드가 내는 `Destroyed` 가 `Applied` 보다 먼저 장부에 닿으면 5판 규칙으로는 세션이 `SourceGone` · `SourceDestroyed` 로 끝난다 — 대상 창에 포커스를 주지 않고(R11b · AC3 어김), `Ended` 가 그 계기의 스레드에서 출처 창의 탭 목록 알림보다 먼저 나가 떼어진 탭이 한 번 되비친다(아래 「끝」이 지키려던 순서). 계기를 알림 묶음 끝 한 번으로 모은 것(§1-2 「알림 묶음 끝」)은 두 번째 반만 막는다 — `Destroyed` 는 그 밖이다.
- `Applied` 는 `dock_tab` 을 부른 실행기가 그것이 돌아온 뒤(알림 · `settled` · `host.close` 다음) 같은 스레드에서 먹인다 — 그래서 `Ended` 는 적용 알림 뒤에 나간다.

#### 놓기 판정 (M1 · PRD R13c)

(W, rev, bar_rev, displayed, shown, point, scroll(5.7)) 하나로 — `drop` · 「마지막 점에 놓기」 · 유실 · 붙든 이른 놓기가 같은 함수를 지난다. ★원칙 = 보인 것과 지금 판정이 같을 때만 무엇이든 바뀐다★:

1. `blocked` → 무변경(`SourceChanged`). (웹뷰가 `dropEffect = 'none'` 이라 보통 `drop` 은 오지 않는다 — 끝 신호로 온다.)
2. **지금 판정** — W 의 지금 영역 = 장부가 W 에 마지막으로 보낸 영역(W 가 스냅숏에서 빠졌거나 `Destroyed` 면 없음). `judged` = 지금 영역이 있으면 `pick(지금 영역, point, scroll)`(5.7 — 그 보고의 `scroll` · 없으면 영역마다 잰 때의 스크롤(`strip.scroll` · 크롬 `scroll`) · 순서 이동과 (리뷰 2) 탭바 끼울 자리 판정에 든다 — 캔버스 판정은 스크롤 무관), 없으면 「가이드 없음」.
3. `shown` = 가이드 없음 — ★빈 보고(`rev` 0 · W 의 지금 영역 없음 — 5.7 · 리뷰 1 로 좁힘)면 「커서 판정」(W, 그 보고의 점)으로 넘긴다★. 아니면 `judged` 도 「가이드 없음」이면 **R22 새 창**(`rev` · `displayed` 는 보지 않는다 — 새 창은 대상 기대값이 없다), 아니면 **무변경**(`TargetChanged` — 사용자가 가이드를 못 본 자리에 지금은 가이드가 있다).
4. `shown` = 가이드 — 모드마다 따로 대조한다(5.2 · 리뷰 B1). 하나라도 다르면 **무변경**(`TargetChanged`) — W 의 영역이 사라진 경우(대상 창이 닫힘 · Q18)도 여기다. 영역을 보낸 뒤 모델이 바뀌었으면 모델 판정 6 이 같은 락 안에서 `TargetChanged` 로 막는다.
   - 4a. **이음새** — `rev` = W 의 지금 `rev` · `displayed` = 그 영역의 `view_id` · `judged` = `shown` 이 **모두** 맞을 때만 Seam{target = 그 영역의 보이는 탭, anchor, side} 를 `DockGuard{lease, expect_source, expect_target = Seam{배치, 창 활성}}` · `Reveal` 로 적용.
   - 4b. **탭바** — `bar_rev` = W 의 지금 `bar_rev` · `judged` = `shown`(같은 `index` — `over` 는 보지 않는다)이 맞을 때만 TabBar{window W, index} 를 `DockGuard{lease, expect_source, expect_target = Tabs(탭 순서)}` · `Reveal` 로 적용. ★`rev` 와 `displayed` 는 보지 않는다★ — 탭바 영역 · 탭 순서는 어느 탭이 보이는지와 무관하고, 활성 전환은 `rev` 만 올리므로 그것까지 보면 전환 알림 틈의 탭바 놓기가 무변경이 된다(웹뷰는 그 틈에도 탭바 가이드를 그린다 — §1-4). 모델 쪽 기대값 `TargetExpect::Tabs` 도 활성을 보지 않는다(이름은 2 단계 그대로 `Tabs`). (리뷰 2) 다시 고르기는 그 보고의 (점, `scroll`) 짝으로 한다 — 탭바 머리 · 끼울 x 가 내용 좌표라(§1-5 크롬) 스크롤은 그 기하를 바꾸지 않고 `bar_rev` 도 올리지 않는다. 5.7 첫 판은 뷰포트 기하 + 스크롤 없는 판정이라, 넘친 탭바가 끄는 중 밀리면 `bar_rev` 가 그대로인 채 옛 기하로 다른 자리를 적용했다(§1-4 「모드」 넘친 탭바 ④).
   - 4c. **순서 이동(5.7)** — W = 출처 창이고 `judged` = `shown`(둘 다 `Reorder` · 같은 `index` — `judged` 는 그 보고의 (점, `scroll`) 짝으로 다시 고른다 · 리뷰 1)일 때만 TabBar{window W, index} 를 `DockGuard{lease, expect_source, expect_target = Tabs(탭 순서)}` · `Reveal` 로 적용(`mode = reorder`). ★`rev` · `bar_rev` · `displayed` 는 보지 않는다★ — 순서 이동 영역은 세션 내내 얼린 시작 측정이라(§2-4) 영역이 실려 있는 한 같은 점의 판정은 늘 같고, 영역 번호가 오르는 까닭(도킹 모드의 머리 접힘 크롬 보고 · 활성 전환 · 내용 변경)은 그 판정과 무관하다 — 그것까지 보면 구역에 돌아온 직후의 놓기가 크롬 보고 왕복 틈에 무변경이 된다. 영역에서 `reorder` 가 빠졌으면(출처 창 탭 목록이 바뀜 — §2-4) `judged` 가 `Reorder` 가 아니므로 무변경이고, 모델 판정 6 이 같은 락 안에서 탭 순서를 다시 본다. 같은 자리면 모델이 무변경(`changed = false`) — 안 보이던 탭이면 보이게만 한다(§11-1 #6 · PRD Q16). W ≠ 출처의 `Reorder` 보고는 닿지 않는 방어로 무변경이다.

- ★5판의 「지금 판정이 가이드 없음이면 보고가 무엇이든 새 창」(4판에서 이어짐)을 버렸다(5.1 · 리뷰 C2 · F3)★ — 사용자가 가이드를 봤는데 그 영역이 바뀌거나 사라져 지금 판정이 없음이면, 새 창은 못 본 결과다(M1 · R13c 「판정이 마지막으로 보인 가이드와 다르면」 · Q18 「대상 사라짐 = 무변경」). 새 창은 사용자도 가이드를 못 봤고 셸도 가이드가 없다고 판정할 때뿐이다.
- **보이는 탭 경쟁(5.1 · C1)** — `switch_tab`(R6b 머묾 · 사람 클릭 · 다른 경로)은 활성을 레이아웃 락 안에서 바꾸고 알림은 락을 놓은 뒤 낸다. 그 사이 웹뷰 화면 · 장부 영역 · 모델이 서로 다른 탭을 가리킬 수 있다. 세 겹으로 맞춘다 — ① 웹뷰는 그 창 캔버스가 보이던 탭(`displayed` — 떼어진 표시의 승계 탭 포함)을 보고 · 놓기에 싣고, 그것이 영역의 `view_id` 와 다르면 그 영역으로 그리지 않는다(가이드 없음 — §1-4) ② 장부는 `displayed` ≠ 그 `rev` 의 `view_id` 면 이음새 놓기를 무변경으로(위 4a — 탭바 놓기는 이 대조를 하지 않는다 · 4b) ③ 기대값 `TargetExpect::Seam` 이 그 `rev` 때 대상 창의 **모델** 활성을 쥐고 모델이 같은 락 안에서 대조한다(§1-2 「4 단계 고침 둘」). ①②는 웹뷰 · 장부 사이, ③은 장부 · 모델 사이의 틈을 닫는다 — ③이 없으면 활성이 바뀐 직후의 이음새 놓기가 이제는 안 보이는 탭에 들어가고 `Reveal` 이 그 탭을 도로 앞에 낸다.
- **새 창** — `NewWindow{at: pos = 커서 − hotspot × 출처 창 배율}`(`hotspot` = 시작 때 웹뷰가 알린 그림 안 커서 자리 · 배율은 실행기가 읽는다 — 그림은 출처 웹뷰가 그 배율로 만든다 · 그림이 있던 자리에 창이 뜬다 · 크기는 적용 서비스가 R22 규칙으로) · `DockGuard{lease, expect_source}`. 커서를 못 읽었으면 「마지막으로 알려진 점」(위)으로, 그것도 없으면 `pos = None`(기본 자리).
- **적용 결말** — `Applied` 가 정한다(위 「적용 중」). 성공: 대상(새 · 재사용) 창 `set_focus()`(R11b — 첫 슬롯 포커스는 적용 서비스가 끝냈다) · 재사용이면 그 창을 보인다. 실패(사유 무관): 무변경.

#### 끝 (모든 갈래 공통)

(성공이면 적용 서비스의 알림이 이미 나갔다 — `Applied` 는 그 뒤에 먹는다) → `Broadcast(Ended)` → 숨긴 출처 되보이기(무변경 끝 · 재사용 끝 · (리뷰 2) 대상 = 출처인 적용(4c — 「시한이 앞지른 끝」) · 출처가 사라졌으면 건너뜀 · 5.7 — 실제로 숨긴 적이 있을 때만(미룬 채 끝났으면 없음) · (리뷰 2) 메인 스레드의 맞추기로 — 끝난 세션의 출처는 장부 값이 보임이다(「명령의 실행 스레드」) · 미룬 최대화를 입히는 보이기 경로 `apply_deferred_maximize` 를 지난다 — 미룸이 서 있는 창이면 그것을 거두고 입힌다 · 5.6 에서 재사용 되돌리기가 미룸을 세우는 일은 없어졌다) → `set_focus`(성공) → `release_lease`(성공이면 이미 내려갔다 — 무동작) → 세션 깃발 내림(자기 세대일 때만 — 비교-교체 · 5.2) · `ended` 에 id 적기 → 끝 로그. `Ended` 를 적용 알림 **뒤에** 내므로 출처 창은 탭이 빠진 목록을 먼저 받고 떼어진 표시를 거둔다 — 탭이 한 번 비쳤다 사라지지 않는다(같은 스레드의 emit 순서 — 가능성 높음 · QA). `dock:session` 은 세션 안 단조 `seq` 를 싣고, 웹뷰는 `drag_id` 별로 가장 큰 `seq` 의 상태만 쓴다 — `Started` · `Display` 는 같은 칸(`source_label` · `source_view` · `display_active`)을 다 실어 어느 것이 먼저 닿아도 같은 상태가 서고, `Ended` 는 그 id 의 마지막이다(뒤에 닿은 작은 `seq` 는 버린다).

#### R6b 탭 전환

대상 웹뷰가 「보이지 않는 탭의 머리」 위 머묾을 재고(`HOVER_SWITCH_MS` 500ms) 기존 탭 전환(`viewStore` 의 탭 전환 → `switch_tab` — 사람 클릭과 같은 길)을 부른다. 모델 활성이 바뀌면(`version` +1 · 저장) 모델 계기가 그 창 영역을 다시 보낸다(`view_id` · 기대값의 활성이 바뀌어 `rev` +1). 그 알림 · 영역이 닿기 전의 이음새 놓기는 「보이는 탭 경쟁」(위)으로 무변경이다. 탭바 놓기는 탭바 몫(`bar_rev`)과 탭 순서만 보므로 그대로 들어간다(「놓기 판정」 4b — 5.2). ★끄는 동안 모델을 바꾸는 유일한 동작이고, 끝이 무변경이어도 되돌리지 않는다(§11-1 #4)★. 장부는 머묾을 모른다 — UI 시간 재기는 그리는 쪽 몫이고 쓰기는 기존 표면이다. (5.7) 출처 창의 순서 이동 구역에서는 `pick` 이 `Reorder` 라(`over` 가 없다) 머묾이 걸리지 않는다 — 커서 밑은 늘 끄는 머리다(PRD R23 · Q20 에서 따라 나온 것 · §11-1b ② 닫힘). 다른 창 탭바의 머묾 전환은 그대로다.

### 1-4. ③ 웹뷰 — HTML5 끌어 놓기 (렌더링 + 사건 전달만)

#### 출처 — 탭 머리

- **`draggable`** — 탭 머리마다(이름 입력 중 · 닫기 버튼에서 시작한 끌기는 `dragstart` 에서 `preventDefault`). 문턱 · 캡처 · 끌기 그림 이동 · 창 밖 추적은 엔진 몫이다 — 4판의 4px 문턱 · `setPointerCapture` · `pointerup` · `blur` 리스너는 없다. 끌기가 시작되면 엔진이 그 누름의 `click`(탭 전환)을 내지 않는다.
- **`dragstart`(동기 — 이 안에서 끝낼 것)** — `drag_id = crypto.randomUUID()` · `setData('application/x-engram-tab', JSON{drag_id, view_id})` ★이 형식 하나만★(`text/plain` · URL 을 싣지 않는다 — 다른 앱이 받으면 `dropEffect` 가 none 이 아니게 되고 그 앱에 글자가 들어간다) · `effectAllowed = 'move'` · `setDragImage(끌기 그림 요소, 커서 자리)`(§4-2) · `stopPropagation()`(아래 「다른 끌기 리스너」).
  - **셸 세션은 끌기가 정말 시작됐을 때만 연다(5.1 · F2)** — invoke `dock_drag_start{drag_id, view_id, hotspot = 그 커서 자리}`(기다리지 않는다)는 `dragstart` 안이 아니라 그 사건 디스패치가 끝난 뒤(`setTimeout 0`) 원 사건(`nativeEvent`)의 `defaultPrevented` 가 거짓일 때만 부른다. 뒤 리스너가 시작을 취소하면 OS 끌기가 없어 끝 신호도 오지 않는다 — 그때 세션을 열면 떼어진 탭 · 표식 · 숨긴 팝아웃이 `pointer_seen` 까지 남는다. 디스패치 안의 마이크로태스크로는 안 된다(리스너 하나가 끝날 때마다 돌아 뒤 리스너보다 먼저다). 이 미룸으로 끝 신호가 시작보다 먼저 셸에 닿을 수 있고, 그것은 장부가 붙든다(§1-3 「이른 끝 신호」).
  - ★`dragstart` 안에서 DOM(탭 머리 · 탭 목록)을 바꾸지 않는다★ — 끌리는 요소를 그 자리에서 숨기거나 빼면 Chromium 이 끌기를 곧바로 끝낸다(가능성 높음 · S1 ④). 떼어진 표시는 `Started` 를 받은 뒤다 — 5.7 부터는 그 뒤 도킹 모드가 될 때다(아래 「모드」).
  - **탭바 재기 `strip`(5.7)** — 같은 `dragstart` 안에서 탭바 상자(스크롤 상자 자신 — 뷰포트 · 스크롤로 움직이지 않는다)와 그 안 머리 **전부**(끄는 머리 포함 · DOM 순서)의 `getBoundingClientRect` · `grab_x = dragstart.clientX − 끄는 머리 x0` · 탭바 `scrollLeft`(`scroll`)를 재 `dock_drag_start.strip`(`StripStart`)에 싣는다. ★머리의 x 는 내용 좌표로 싣는다 — 잰 x + 그 `scrollLeft`(스크롤 0 일 때의 자리 · 리뷰 1 · 사용자 지시 2026-10-17)★ — 끄는 중 탭바가 스크롤되어도 얼린 기하가 그대로 맞고 바뀌는 것은 스크롤 하나다(§2-5). 읽기만이라 끌기를 끝내지 않는다(위 「DOM 을 바꾸지 않는다」는 쓰기 몫이다). 접힌 머리는 아직 없다. 수 하나라도 유한하지 않거나 끄는 머리가 목록에 없으면 `null`(그 끌기는 순서 이동 없이 5.6 그대로). ★`grab_x` 는 누른 점이 아니라 `dragstart` 점으로 잰다★ — 첫 `dragover` 의 점이 그 근처라 순서 이동 모습이 시작할 때 머리가 문턱 폭만큼 튀지 않는다.
  - **엔진 취소(5.7 · 결함 ②)** — `dragstart` 의 사건 시각(`event.timeStamp`)을 끌기 기록에 둔다. `dragend` 의 `dropEffect` 가 `none` 이고 `dragend.timeStamp − dragstart.timeStamp < ENGINE_CANCEL_MS`(15 ms)면 엔진이 접은 끌기다 — 시작 시한(`setTimeout 0`)이 아직이면 지우고 IPC 0(5.6 과 같다) · 이미 세션을 열었으면 `dock_drag_end{effect: none, canceled: true}`(장부가 무변경으로 끝낸다 — §1-3 「엔진 취소 끝」). ★「시한 전에 온 끝 = 엔진 취소」(5.6 · 7 단계)를 버렸다★ — 실측에서 엔진이 접은 끌기의 `dragend` 는 `dragstart` 와 같은 task 가 아니라 2–4 ms 뒤 따로 온 task 였고(4/19 가 0 ms 시한 뒤에 와 세션이 열리고 새 창이 됐다), task 순서는 시한 · 끌기 사건이 서로 다른 task 원천이라 보장되지 않는다. 사건 시각은 처리기가 언제 돌든 사건이 만들어진 때라 경주가 없다. 15 ms = 실측 엔진 취소(2–4 ms)와 가장 짧은 실제 OS 끌기 끝(+28 ms) 사이. 실제 끌기를 엔진 취소로 잘못 가르면(15 ms 안의 진짜 끝) 무변경 — 그 끌기는 커서가 누른 자리 근처라 무해하다. `move` · `other` 끝은 가르지 않는다(받은 창이 있다). 엔진 취소가 아닌 끝이 시작 시한보다 먼저 오면(웹뷰 스레드가 그만큼 막힘) 그 자리에서 시한을 당겨 세션을 연 뒤(`defaultPrevented` 는 이미 정해졌다) 평소 끝을 보낸다 — 5.6 처럼 조용히 버리면 대상의 `drop` 이 모르는 id 로 붙들렸다 버려져 사용자가 본 놓기가 사라진다. ★사건 시각의 기준은 미검(리뷰 1)★ — Chromium 이 `dragstart` 에 누른 순간(`pointerdown`)의 시각을 찍으면 차가 문턱 이동 시간만큼 커져 15 ms 거름이 드물게만 걸린다. S7 ⑦ 이 잰다. 못 걸러도 무해하다 — 그 끌기는 `dragover` 가 없어 보고 없는 끝으로 출처 커서 판정에 가고(§1-3), 커서는 누른 머리 위(구역 안)라 `Reorder{제자리}` → 모델 무변경(안 보이던 탭이면 보이게만 — 그 누름이 클릭이었으면 했을 일과 같다) · 모습은 순서 이동 그대로라 떼어진 모습 · 팝아웃 숨김도 없다(PRD AC28 은 그대로 선다).
- **`dragend`** → invoke `dock_drag_end{drag_id, effect: dropEffect, canceled, last}`(5.7 — 위 「엔진 취소」 · `last` = 리뷰 2). ★**`last`(리뷰 2 · 기본값 · 메인 판단)**★ = 이 창이 그 끌기에서 마지막으로 그린 판정(`dockDrag.ts` `evaluate` 가 화면을 그린 그 값 그대로 — `rev` · `bar_rev` · `displayed` · `shown` · `point` · `scroll`)이고, 이 창이 끝에 아직 위일 때만 싣는다 — 그 판정 뒤 떠남이 없었거나, 있었어도 그 떠남의 사건 시각이 `dragend.timeStamp` 전 `LEAVE_GRACE_MS` 안(구역 안 Esc 는 떠남을 먼저 낸다 — §1-3). (리뷰 3) 그 떠남 → `dragend` 사건 시각 간격을 `left_before_end_ms` 로 함께 싣는다(떠남 없음 = `null` · 한 창 한 엔진 시계라 셸 도착 흔들림과 무관) — 셸이 엔진이 끌기를 접으며 낸 떠남(`CANCEL_LEAVE_MS` 안 — Esc · 받지 않는 자리의 떼기)과 실제로 창을 떠난 것을 가른다(§1-3 「끝 스냅숏」). 그래서 떠남(`clearTarget`)이 `drawn` 을 지워도 직전 그림과 그 떠남의 사건 시각을 그 끌기 끝까지 따로 쥔다. 그 밖(그린 것 없음 · 오래전에 떠남 · 다른 `drag_id`)은 `null`. 셸은 도착 순번으로 고른 W 가 이 창일 때만 마지막 `Over` 대신 쓴다(§1-3 「끝 신호」 — 보고 줄과 끝 invoke 의 도착 순서 경주를 닫는다 · 리뷰 3 — `last` 는 W 를 정하지 않는다 · 리뷰 4 — 도착 순번 W 가 없고 간격이 엔진 접기면 W = 이 창 · 간격이 실제 떠남이면 셸이 다른 창의 오는 중 `Over` 를 잠깐 기다린다 — 웹뷰 쪽 바꿀 것은 없다). ★끌린 머리 요소는 끄는 동안 DOM 에 남긴다★(React key 그대로 · CSS 로 접기) — 빼면 `dragend` 가 그 요소 밖으로 올라오지 않는다(가능성 높음 · S1 ④). 그 요소에 리스너를 단다.

#### 대상 — 모든 창(출처 창 포함 · 숨긴 출처 제외)

- **받기** — `document` 의 `dragenter` · `dragover` 에서 `types` 에 우리 MIME 이 있으면 ★세션을 몰라도(`Started` 전이어도 — 5.1 · C3)★ `preventDefault()` + `dropEffect = blocked ? 'none' : 'move'` + `stopPropagation()`. `blocked` 는 그 끌기의 영역(`DockRegions.blocked`)으로 알고, 영역 전엔 `move` 다. 5판은 `Started` 를 안 뒤에만 받았는데, 이웃 창으로 빠르게 끈 끌기는 그 창에 `Started` 보다 먼저 닿을 수 있어 그 창이 끌기를 거절하고(커서 「안 됨」 · `none` 끝) 「마지막 점에 놓기」가 보고 없는 끌기로 새 창을 띄웠다. `drag_id` 확인은 셸이 한다(모르는 id 의 놓기는 장부가 붙들었다 버린다 — §1-3). `dragover` 에서는 데이터를 못 읽는다(보호 모드 — `types` 만) — 「지금 끌기」 = `dock:session` · `dock:regions` 로 가장 늦게 알게 된, 끝나지 않은 `drag_id`.
- **영역 · 세션 받기(5.1 · C3 · F5)** — `drag_id` 별로 가장 큰 `rev`(영역) · `seq`(세션)만 쓰고 작은 것은 버린다 · `Started` 전에 온 영역 · `Display` 도 받아 둔다 · `Ended` 된 id 의 것은 버린다(§1-3 「영역」 순서).
- **판정 · 그리기** — 점 = `(clientX, clientY)`(그 창 뷰포트 CSS px) · (리뷰 2) 스크롤 = 그 창 탭바의 지금 `scrollLeft`(모든 창 — 탭바 자리 · 순서 이동 자리는 내용 좌표) → `pickTarget(영역, 점, 스크롤)`(`dockPick.ts` — §2-4 의 거울) → `DockOverlay` 상태. 그 창 탭바의 `scroll` 사건도 끄는 중이면 마지막 점으로 다시 판정한다(`refresh` — 커서가 멈춘 채 탭바만 밀려도 자리 · 막대가 따라간다 · 리뷰 2 — 출처 창만이 아니다). ★그 창 캔버스가 보이는 탭(`displayed` — 모델 활성이거나 떼어진 표시의 승계 탭)이 영역의 `view_id` 와 다르면 그 영역으로 그리지 않는다(가이드 없음 · 탭바는 그대로 그리고 탭바 놓기도 선다 — 5.1 · C1 · 5.2)★ — 탭 목록 알림과 영역이 엇갈려 닿는 틈에 다른 탭의 칸 위에 옛 가이드를 그리지 않게. `shown` · 영역 `rev` · `displayed` 중 하나가 바뀌면 `dock_drag_over{Over}`(그 영역의 `bar_rev` 를 함께 싣는다)(`drag_id` 를 모르면 보내지 않는다). 영역이 없으면 `shown = 가이드 없음`이고 `dragenter` 에서 `dock_pull` 한 번.
- **떠남** — 문서 밖으로 나간 `dragleave`(관련 요소 없음)에서 가이드 · 머묾을 지우고 `dock_drag_over{Left}` · `stopPropagation()`. (5.7 · 리뷰 1) 출처 창이 도킹 모습으로 바꾸는 것은 곧바로가 아니라 `LEAVE_GRACE_MS` 뒤다(아래 「모드」).
- **`drop`** — `preventDefault()` + `stopPropagation()` · `getData` 로 `{drag_id, view_id}` · `dock_drop{drag_id, view_id, rev, bar_rev, displayed, shown = 마지막으로 그린 것, point, scroll}`(리뷰 2 — `point` · `scroll` 까지 전부 그 그린 판정 하나의 값 · 놓는 순간 다시 읽지 않는다) · 가이드를 지운다.
- **머묾(R6b)** — `pick` 의 탭바 `over` 가 보이지 않는 탭이면 500ms 시한 → 탭 전환. `over` 가 바뀌거나 떠남 · 놓기 · `Ended` 면 취소. 탭바 오버레이가 대기 중인 머리에 진행 표시를 한다. (5.7) `Reorder` 에는 `over` 가 없어 출처 창 구역에서는 걸리지 않는다.

#### 모드 — 순서 이동 · 도킹 (출처 창만 · 5.7 · PRD R2 · R23 · D15)

- **누가 정하나** — 출처 창 웹뷰가 자기 `pick` 결과로 정한다. 셸에 묻지 않고(왕복 없음 · 60Hz) 셸도 따로 정하지 않는다 — 판정 규칙이 셸 `hit` 의 거울이므로 웹뷰가 그린 모드 · 자리와 셸이 놓기에서 다시 고른 것이 같다(§1-3 「놓기 판정」 4c). 다른 창은 모드가 없다(끄는 탭을 그리지 않는다).
- **규칙 — 신호 = 출처 창 자기 `dragover`(리뷰 1)** — 끌기가 그 창에 한 번 들어오면(첫 `dragenter`) 엔진은 커서가 멈춰 있어도 `dragover` 를 계속 낸다(실측 멈춤 ~15/s · 움직임 60/s — §8-1). 그 사건마다(그리고 영역 · 세션 · 활성 · 탭바 스크롤 변화의 다시 판정마다 — `dockDrag.ts` `refresh`) `pick` 이 `Reorder` 면 순서 이동, 그 밖이면 도킹이다. ★첫 `dragover` 전 = `dragstart` 점으로 정한다 → 순서 이동★ — OLE 는 다음 마우스 메시지 전에 `DragEnter` 를 내지 않아(§1-3 「커서 판정」) 그 틈엔 판정할 점이 없다(`evaluate` 는 `dragover` 와, `inside` · `lastPoint` 가 선 뒤의 다시 판정에서만 돈다). `dragstart` 점은 끄는 머리에서 엔진 문턱(몇 px)만큼 움직인 자리라 구성상 구역(탭바 ± 15 px) 안이다. **판정 불가 = 지금 모드 유지** — 영역이 아직 없음(`rev` 0) · 막힌 영역(`blocked` — 몸이 비어 `pick` 을 하지 않는다 · 리뷰 1)이 그렇다. 막힌 끌기는 놓아도 무변경이라 떼어진 모습을 새로 세울 까닭이 없고, 장부의 도킹 모드 증거도 그 보고를 치지 않는다(§1-3 ①) — 둘이 같은 규칙이어야 탭 하나 팝아웃이 보인 채 떼어진 빈 모습을 그리지 않는다. 영역에 `reorder` 가 없으면(측정 없음 · 어긋남 — §2-4) 첫 판정에서 도킹이 되고 5.6 그대로다.
- **떠남은 미룬다(리뷰 1)** — 관련 요소 없는 `dragleave`(문서 밖)는 곧바로 도킹이 아니라 `LEAVE_GRACE_MS`(100 — 장부와 같은 값 · 판정과 무관한 모양 값이라 두 언어에 두어도 판정이 갈리지 않는다) 뒤 도킹이다. 그 안에 자기 `dragend`(끝) · `dragenter` · `dragover`(돌아옴 — 그 판정이 모드를 정한다) · `Ended` 가 오면 거둔다. 구역 안 Esc 는 출처 창에 떠남을 먼저 주고 곧바로 `dragend` 가 따라오므로(같은 웹뷰 · IPC 없음), 미루지 않으면 머리 접힘 · 승계 탭 캔버스가 한 번 번쩍인 뒤 순서 이동 놓기(4c)로 끝난다. 창 밖으로 나간 끌기의 떼어진 모습은 100 ms 늦게 선다(받아들임 — 커서는 이미 창 밖이다). 가이드 · 머묾 지우기와 `Left` 보고는 미루지 않는다(장부가 스스로 미룬다 — §1-3 「도킹 모드 증거」 ②).
- **판정 순서(9c 가 박는다 · 리뷰 1)** — 한 판정 안에서 ① `pick` ② 모드(① 의 결과로) ③ `displayed`(② 의 모드로 — 도킹이면 승계 탭) ④ 캔버스 거름(③ 이 영역의 `view_id` 와 다르면 이음새를 가이드 없음으로) ⑤ 그리기 · `Over`. 지금 `evaluate` 는 `displayed` 를 `pick` 앞에서 정한다(`dockDrag.ts` — `displayedNow()` 가 첫머리). 모드를 그 뒤에 더하면 구역 밖 첫 프레임은 앞 모드(순서 이동)의 `displayed` = 끄는 탭으로 거름을 해 지금 `rev` 의 `Over{없음, displayed = 끄는 탭}` 을 보낸다. 그 프레임 뒤 곧바로 떼면 그 보고는 빈 보고가 아니라(지금 `rev`) 놓기 판정 3 이 「가이드 없음을 본 자리가 지금 가이드」로 무변경을 낸다 — 사용자는 바로 그 프레임에 도킹 모습과 가이드를 함께 볼 참이었다. 순서를 박으면 첫 프레임부터 도킹 모드의 `displayed`(승계 탭)로 거르고 보고한다.
- **남는 틈(받아들임 · 리뷰 1)** — 문턱 직후 구역 밖으로 빠르게 빼고 더 움직이지 않으면 그 창에 `dragenter` 가 오지 않아(그 움직임이 OLE 루프 전에 지나갔다) 모습은 순서 이동 그대로다(탭이 탭바에 · 탭 하나 팝아웃은 보인 채). 다음 마우스 움직임이 `dragenter` · `dragover` 를 열어 그 판정에서 도킹 모습이 된다. 그대로 떼면 보고 없는 끝이라 셸이 끝 순간 커서로 판정한다(§1-3 「커서 판정」 — 출처 캔버스 = 모델 활성으로 거름) — 결과는 커서 자리 그대로이고 모습만 늦다. S7 ⑫ 가 잰다(§11-2 #35).
- **도킹 모드에서만** — 떼어진 표시(§4-1 — 머리 접기 · 캔버스 = 승계 탭) · `displayed` = `displayedTabOf(..)`(승계 탭). **순서 이동 모드** — 머리를 접지 않고 순서 이동 모습(§4-4)을 그리며 `displayed` = 모델 활성(끄는 탭이 활성이면 그 탭 그대로). 보고 · 그리기 · 화면이 같은 함수(`displayedTabOf` 에 모드를 더한 것)를 쓴다 — 5.1 C1 의 「보이던 탭」이 갈라지지 않게.
- **오버레이** — `Reorder` 면 아무것도 그리지 않는다(바깥 띠 빗금 포함 — 끄는 탭이 활성이 아닌 창에서는 `canvas` 가 참이라 지금 코드는 바깥 띠를 그린다 · §4-3).
- **모드가 바뀌는 렌더** — 머리 접기 · 되살리기와 순서 이동 모습 세우기 · 거두기를 같은 렌더에서 하고 그 렌더는 전환(transition)을 끈다(§4-4) — 접힘으로 바뀐 배치에 옛 transform 이 미끄러지면 머리 하나 폭만큼 튄다.
- **크롬 보고** — 순서 이동 모습이 서 있는 동안(transform 이 남은 동안) 출처 창은 크롬 보고를 건너뛰고, 모습을 거둔 렌더 뒤 한 번 다시 잰다. `getBoundingClientRect` 는 transform 을 담아 비켜난 자리를 잰다 — 셸의 그 창 `tab_bar`(끼울 자리) 기하가 흔들려 `bar_rev` 만 헛돈다. 출처 창의 구역 · 자리는 얼린 `strip` 으로 판정하므로 그동안 셸에 필요한 보고가 없다.
- **넘친 탭바 — 끝에 대고 있으면 계속 스크롤(사용자 지시 2026-10-17 · PRD D20 · 리뷰 1)** — 탭바는 스크롤 상자다(`TabBar.tsx` 의 `overflowX: 'auto'`). 순서 이동 모드에서 커서를 탭바 상자 왼쪽 · 오른쪽 끝 근처에 대고 있으면 그쪽으로 계속 민다(브라우저 탭바 · 끌기 자동 스크롤의 관례).
  - ① **엔진에 맡긴다** — Chromium 의 끌기 자동 스크롤(끌기 중 스크롤 상자 가장자리 근처에 머물면 그 상자를 민다 · 띠 폭 · 지연 · 속도는 엔진 값)이 이 탭바에 걸리는지는 미검이다. S7 ⑬ ⓐ 가 재고, 안 걸리거나 쓸 수 없으면 9c 안에서(커밋 전) 우리 끝 띠 시한으로 바꾼다 — 순서 이동 모드 · 점이 탭바 상자 끝 `EDGE_SCROLL_PX` 안 · 그쪽으로 더 밀 수 있을 때 rAF 마다 `EDGE_SCROLL_STEP` 씩 · 띠를 벗어남 · 모드 바뀜 · 자기 `dragend` · `Ended` 에서 멈춘다(멈춘 커서에도 `dragover` 가 와 점을 안다). 둘을 함께 켜지 않는다.
  - ② **내용 좌표** — 자리 · 끄는 머리 왼쪽은 점의 내용 x(`clientX + scrollLeft`)로 판정한다(§2-5 — 얼린 머리도 내용 좌표). `dragover` 마다 · 탭바 `scroll` 사건마다(마지막 점으로 다시 판정 — 커서가 멈춘 채 탭바만 밀려도) 다시 판정해, 끄는 머리는 커서 밑에 머물고(화면 x = 점 − `grab_x` — 스크롤 무관) 지나가는 이웃이 비켜나며 자리가 그려진 그대로 바뀐다.
  - ③ **셸에 닿는 스크롤** — `Over` · `dock_drop` 이 그 판정의 `scroll` 을 싣는다(선택 칸 — §1-5). 장부는 같은 (점, 스크롤) 짝으로 `pick` 을 다시 해 같은 `index` 를 낸다(4c). `Over` 는 자리가 바뀔 때만 가므로 그 사이의 스크롤은 실리지 않지만, 한 보고의 (점, 스크롤) 짝은 늘 그 보고의 자리를 낸다. ★(리뷰 2) 단 셸이 **받은** 마지막 보고가 마지막으로 **그린** 자리라는 보장은 없다★ — 보고 줄(`overChain`)과 끝 invoke 가 따로라, 가장자리 스크롤이 새 자리를 그리고 `Over` 를 줄에 올린 직후의 Esc 는 끝이 먼저 닿아 옛 자리가 적용될 수 있었다(5.7 첫 판의 「마지막 보고 = 마지막으로 그린 자리」는 거짓). 그래서 끝 신호가 그린 판정을 스스로 싣는다 — `dock_drop` 의 칸 · `dock_drag_end.last`(§1-3 「끝 신호」 · §1-4 「출처」). 끝 신호가 없는 유실만 셸이 받은 마지막 보고로 판정한다. 보고가 없으면 `strip.scroll`(§1-3 「커서 판정」).
  - ④ **다른 창 탭바(R21 · D17) — 모든 창 탭바를 내용 좌표로(리뷰 2 · 기본값 · 메인 판단)** — 크롬 보고는 머리 · 끼울 자리를 스크롤 상자 기준 내용 좌표(스크롤 0 일 때의 자리 — 잰 x + 그때 `scrollLeft` · 출처 `strip` 과 같은 꼴)로 싣고 잰 때의 `scroll` 을 함께 싣는다(§1-5 `DockChrome`). 모든 창이 탭바 자리를 점의 내용 x(`clientX + 지금 scrollLeft`)로 판정해 보고 · 놓기에 그 `scroll` 을 싣고, 셸은 그 (점, `scroll`) 짝으로 다시 고른다(4b). 끼울 자리 막대는 `insert_x − 지금 scrollLeft` 에 그린다(§4-3). 스크롤은 내용 좌표 기하를 바꾸지 않으므로 탭바 `scroll` 사건은 크롬 보고 계기에서 빼고(`WindowLayout.tsx` 의 `TabBar` `onScroll={scheduleChrome}` 을 거둔다) 다시 판정 계기로 쓴다(§1-4 「판정 · 그리기」). ★5.7 첫 판의 「엔진 자동 스크롤이 돌면 크롬 보고가 따라가고, 닿기 전 놓기는 `bar_rev` 가 옛것이라 무변경(안전 쪽)」은 거짓이었다★ — 크롬 보고는 부를 때마다 다시 거는 뒤 디바운스라(`dockChromeReport.ts` `scheduleDockChromeReport`) 연속 자동 스크롤 동안 한 번도 터지지 않고, 그러면 `bar_rev` 가 그대로라 4b 가 통과해 셸이 **옛 뷰포트 기하로 다른 자리를 적용**하며 막대도 옛 x 에 서 있었다(무변경이 아니라 잘못 붙음). 우리 끝 띠 시한은 출처 창 순서 이동에만 둔다.

#### 세션 동안 그 밖 (`dock:session` 구독 한 곳)

- **떼어진 표시**(§4-1) — `Started` · `Display` 로 재료를 받고(5.7 — 그리는 것은 출처 창이 도킹 모드일 때만) `Ended` 로 거둔다.
- **끝 신호 유실 대비** — `Started` 뒤 그 창의 첫 「끝 증거」 하나만 `dock_pointer_seen`(세션마다 창마다 한 번). 끝 증거 = `pointerType === 'mouse'` 이고 `buttons === 0` 인 `pointermove` · 또는 `pointerdown`(끄는 동안 새 누름은 없다 — 5.1 · N1). 버튼 조건을 두는 이유 — 떼어진 표시가 머리를 접는 재배치 뒤 엔진이 멈춘 커서 밑에 합성 이동을 낼 수 있다(미검 · S1 ③). 그것이 끄는 중(버튼을 쥔 채)에 오면 걸러진다. 대가 = Esc 로 끝낸 뒤 버튼을 쥔 채 움직이면 증거가 버튼을 뗄 때까지 늦는다 — 이것은 끝 신호를 잃었을 때만의 대비라 무해하다.
- **Esc 삼키기** — `Started` 부터 `Ended` 뒤 `ESC_SWALLOW_TAIL_MS`(200ms)까지 `window` capture 단계 `keydown` 에서 `Escape` 를 `preventDefault` + `stopPropagation`(보내지는 않는다 — Esc 는 OS 끌기 루프가 받아 끌기를 끝낸다). OS 가 그 keydown 을 웹뷰에 넘기는지는 모른다 — 넘기면 터미널(claude 턴 끊기) · 입력창에 닿지 않게 하는 보험이다(S1 ⑥). ★상한(5.1 · N9)★ — 그 창이 위 끝 증거를 보면 그때부터 200ms 뒤 그 창에서는 그친다(세션이 아직 살아 있어도). 끝 신호를 잃어 세션이 남아도 마우스를 다시 쓴 창의 Esc(claude 턴 끊기)를 계속 먹지 않는다 — 그 증거가 `pointer_seen` 으로 세션도 `END_GRACE_MS` 뒤 끝낸다. 남는 것 = 마우스를 안 쓰고 키보드만 쓰는 창은 세션이 끝날 때까지 Esc 가 먹힌다(세션이 남는 것 자체가 끝 신호 유실 — §11-2 #2).
- **`Ended`** — 가이드 · 머묾 · 떼어진 표시를 무조건 지운다(R13b 「남은 가이드 없음」).

#### 늘 — 파일 끌어 놓기 막기(PRD AC22 · W9)

모든 창 `document` 에 우리 MIME 이 아닌 끌기면 `dragover` 에서 `preventDefault()` + `dropEffect = 'none'`, `drop` 에서 `preventDefault()`. 드롭 처리기를 끈 창(main 은 지금도 `dragDropEnabled: false` — `tauri.conf.json:21`)에 OS 파일을 놓으면 웹뷰가 그 파일로 탐색할 수 있다(미검 — S3 ②). 세션과 무관하게 늘 둔다. ★우리 MIME 이 아닌 사건은 끊지 않는다(`stopPropagation` 없음)★ — 트리 행 재부모화 끌기는 이 리스너 뒤의 window 리스너(react-dnd)가 자기 `dropEffect` 로 덮어 그대로 돈다(아래 · S1 ⑦).

#### 다른 끌기 리스너 — react-arborist (5.1 · F2)

- **무엇이 막나(확실 · 소스)** — 에이전트 트리(`src/components/agent/AgentList.tsx` — 슬롯 내용 `agent_list` · `LayoutLeaf.tsx:319` · keep-alive 라 안 보이는 탭의 트리도 마운트돼 있다)는 react-arborist 3.9.0 이고, 그 `DndProvider` 는 react-dnd HTML5 백엔드를 `dndRootElement` 를 안 넘기면 뿌리 = `window` 로 단다(`react-arborist/dist/module/components/provider.js:54-60` · `react-dnd-html5-backend/dist/esm/OptionsReader.js:48-52`). 그 백엔드의 window `dragstart`(버블)는 자기 끌기 원천도 브라우저 기본형(파일 · URL · 글자 · HTML)도 아닌 끌기를 `preventDefault` 한다(`HTML5BackendImpl.js:112-213` — :212) — 우리 MIME 하나만 실은 탭 끌기는 OS 끌기가 시작되지 않는다. window `dragover`(버블)는 자기 끌기 중이 아니면 `preventDefault` + `dropEffect = 'none'` 이다(:280-292) — 우리 `document` 리스너가 먼저 `move` 를 두어도 덮여 놓기가 거절된다. 트리 슬롯이 하나라도 마운트된 창이면 다 그렇다. 2026-10-16 스파이크는 앱 의존이 없는 맨 페이지라 이것을 못 봤다(§8-1).
- **대응 = 우리 MIME 사건이 window 에 닿지 않게 끊는다** — 탭 머리 `dragstart`(React 처리기 — React 는 뿌리 컨테이너에서 처리기를 부르므로 원 사건이 거기서 멈춘다)와 `document` 의 `dragenter` · `dragover` · `dragleave` · `drop` 이 우리 MIME 이면 `stopPropagation()`(위 「출처」 · 「대상」). react-dnd 의 window **캡처** 리스너는 우리보다 먼저 돌지만 브라우저 기본형이 아닌 끌기에는 판정용 칸을 비우거나 세기만 한다(`:106-110` · `:225-241` · `:276-278` · `:335-369` — 읽음). `dragend` 는 끊지 않는다(window 쪽은 캡처 리스너 하나뿐이고 자기 끌기가 아니면 아무것도 안 한다 · `:216-223`).
- **뿌리 좁히기(`dndRootElement` = 트리 컨테이너)는 택하지 않는다** — prop 은 있지만(3.9.0 · `types/tree-props.d.ts:55`), react-dnd 의 `DndProvider` 는 `manager` 를 안 받으면 창(전역 문맥)마다 관리자 하나를 처음 마운트한 트리의 옵션으로 만들어 나눠 쓴다(`react-dnd/dist/esm/core/DndProvider.js:63-88`) — 한 창에 트리 슬롯이 둘이면 둘째 트리의 뿌리가 첫째 트리 컨테이너가 되어 둘째의 재부모화가 끊긴다. 트리마다 관리자(`dndManager`)를 만들려면 `dnd-core` 를 직접 의존해야 한다(지금은 간접). 끊기는 트리 코드 · 의존을 건드리지 않는다.
- **불변식** — react-dnd 의 뿌리는 `window` 이고 우리 리스너는 `document` 다(뿌리가 `document` 면 같은 노드의 등록 순서가 이겨 끊기가 안 선다 — 컨테이너로 좁히는 것은 괜찮다: 그쪽이 먼저 돌고 우리가 뒤에서 `move` 로 덮는다). 트리에 `dndRootElement` · `dndManager` 를 주거나 HTML5 끌기 라이브러리를 새로 들이면 S1 ⑦ 을 다시 돈다(§9 「게이트」의 점검 줄). `src/` 의 그 밖 HTML5 끌기 리스너는 없다(2026-10-16 · `rg` — `addEventListener('drag…' | 'drop')` · JSX `onDrag*` · `onDrop` · `draggable` · `dataTransfer` 전부 0 · 분할선은 포인터 사건).

### 1-5. 웹뷰 ↔ 셸

| 방향 | 이름 | 꼴 | 페이로드 |
|---|---|---|---|
| 웹뷰 → 셸 | `dock_drag_start` | invoke(동기) | `{ drag_id: String, view_id: Uuid, hotspot: PointCss, strip: Option<StripStart> }` → `()` · `strip`(5.7) = `dragstart` 안에서 잰 탭바 — 선택 칸(없음 = 순서 이동 없이 5.6 그대로 · 9b 가 9c 전 웹뷰와 서게) · `hotspot` = 끌기 그림 안의 커서 자리(R22 새 창 자리에 쓴다 — 상수를 두 언어에 두지 않는다) · `Err` = `TabNotFound` · `AlreadyDragging` · `LateStart` · `Dragging` · `LayoutPoisoned`(웹뷰는 무시 — 그 끌기의 놓기는 아무것도 바꾸지 않고 끝난다) · 부르는 때 = `dragstart` 디스패치 뒤 · 취소되지 않았을 때만(§1-4) |
| 웹뷰 → 셸 | `dock_drag_over` | invoke(동기) | `{ drag_id, report: Over{ rev: u64, bar_rev: u64, displayed: Option<Uuid>, shown: Picked, point: PointCss, scroll: Option<f64> } \| Left }` → `()` · 창 label 은 Tauri 가 넣어 준 창 · `displayed` = 그 창 캔버스가 보이던 탭(5.1) · `bar_rev` = 쓰던 영역의 탭바 몫(5.2) · `scroll`(5.7 · 리뷰 1) = 그 판정 때 그 창 탭바 `scrollLeft`(CSS px) — 선택 칸(없음 = 영역마다 잰 때의 스크롤 — 순서 이동 `strip.scroll` · 탭바 `TabBarRegion.scroll` · 순서 이동과 (리뷰 2) 모든 창의 탭바 끼울 자리 판정에 쓴다 · §2-5) |
| 웹뷰 → 셸 | `dock_drop` | invoke(async) | `{ drag_id, view_id, rev, bar_rev, displayed: Option<Uuid>, shown: Picked, point: PointCss, scroll: Option<f64> }` → `()` · `view_id` = 끄는 탭(끌기 데이터) · `displayed` = 놓은 창이 보이던 탭 · `scroll` = 위와 같다(5.7) |
| 웹뷰 → 셸 | `dock_drag_end` | invoke(async) | `{ drag_id, effect: Move \| None \| Other, canceled: Option<bool>, last: Option<DrawnReport> }` → `()` · `Other`(copy · link) = 받은 창이 있다 · `effect` 의 타입 `DragEffect`(move · none · other)는 6c 가 정했다 · `canceled`(5.7) = 엔진이 접은 끌기(§1-4 「출처」) — 선택 칸(없음 = 거짓) · `last`(리뷰 2) = 출처 창이 끝에 아직 위였으면 그 창이 마지막으로 그린 판정 `DrawnReport{ rev, bar_rev, displayed, shown, point, scroll, left_before_end_ms }`(앞 여섯 = `Over` 와 같은 칸 · (리뷰 3) `left_before_end_ms` = 그 판정 뒤 떠남 → `dragend` 사건 시각 간격 ms · 없음 = 떠남 없음) — 선택 칸(없음 = 셸이 받은 마지막 `Over` · 쓰는 것은 도착 순번으로 고른 W 가 출처일 때뿐 · §1-3 「끝 신호」) |
| 웹뷰 → 셸 | `dock_pointer_seen` | invoke(동기) | `{ drag_id }` → `()` |
| 웹뷰 → 셸 | `dock_pull` | invoke(동기) | `()` → `Option<{ started: Started, regions: Option<DockRegions> }>` — 지금 세션과 그 창 영역 |
| 웹뷰 → 셸 | `report_dock_chrome` | invoke(측정) | `DockChrome{ canvas_origin, tab_bar, tabs: Vec<TabHeader>, scroll: Option<f64> }` — 끄는 동안 접힌 머리는 싣지 않는다 · 창 label = Tauri 가 넣어 준 창(`report_window_canvas` 와 같다) · (리뷰 2) `tabs` 의 x 는 내용 좌표(잰 x + 그때 탭바 `scrollLeft` — 스크롤 0 일 때의 자리) · `tab_bar` 는 탭바 상자(뷰포트 — 스크롤로 안 움직인다) · `scroll` = 잰 때의 `scrollLeft`(없음 = 0 — 7 단계 웹뷰는 뷰포트로 재고 `scroll` 없이 보고하므로 둘 다 0 으로 읽혀 판정이 5.6 그대로다 · 9b 가 9c 전 웹뷰와 선다) · 셸은 그것을 `TabBarRegion.scroll` 로 옮겨 보고에 `scroll` 이 없을 때만 쓴다(기하가 아니라 `bumps` 가 견주지 않는다) · 탭바 `scroll` 사건은 이 보고의 계기가 아니다(§1-4 「모드」 넘친 탭바 ④) |
| 셸 → 웹뷰 | `dock:session` | emit(전체) | `{ drag_id, seq, phase: Started{ source_label, source_view, display_active: Option<Uuid> } \| Display{ source_label, source_view, display_active } \| Ended }` — `Display` 도 같은 칸을 다 싣는다(5.1 — 어느 것이 먼저 닿아도 같은 상태) · 웹뷰는 `drag_id` 별 가장 큰 `seq` |
| 셸 → 웹뷰 | `dock:regions` | emit_to(label) | `DockRegions`(아래) · 웹뷰는 `drag_id` 별 가장 큰 `rev` · `Started` 전에 온 것도 받는다 |
| 셸 → 웹뷰 | `layout:updated` · `window:tabs-updated` | 기존 | 도킹 결과는 **기존 알림으로만** 흐른다 |
| 웹뷰 ↔ 웹뷰 | `application/x-engram-tab` | OS 끌기 데이터 | `{"drag_id": "…", "view_id": "…"}` — 받는 쪽은 `drop` 에서만 읽는다 |

```rust
// dock/types.rs (ts-rs · 6 단계 — 5 단계의 GuideView · TabBarHint 를 대신한다. PxRectWire · Band 는 그대로)
pub struct DockRegions {
    pub drag_id: String, pub rev: u64,
    pub bar_rev: u64,                   // 탭바 몫(`tab_bar` · 탭 목록 기대값)이 바뀔 때만 오른다 — 탭바 놓기의 대조 기준(5.2)
    pub blocked: bool,                  // 출처 배치가 바뀜 — 받지 않는다(dropEffect none) · 몸은 빈다(view_id null · 원점 · 캔버스 0 · 목록 빔) — 기대값은 모델 값 그대로(6a)
    pub view_id: Option<Uuid>,          // 이 창에 보이는 탭(모델 — 측정 전에도 선다 · 6a) — None = 보이는 탭 없음(메인 마지막 탭 · 탭 하나 팝아웃을 뗌) · 웹뷰의 `displayed` 와 대조(C1)
    pub canvas_origin: PointCss,        // 크롬 보고 전 = (0, 0)
    pub canvas: CanvasPx,               // 캔버스 보고 전 = 0×0
    pub outer: Vec<OuterBand>,          // 유효한 바깥 변만(R8) — 크롬 · 캔버스 · 보이는 탭이 다 있을 때만
    pub slots: Vec<SlotRegion>,         // 보이는 탭의 칸 전부 — 같은 조건
    pub tab_bar: Option<TabBarRegion>,  // None = 탭바 놓기를 받지 않음(크롬 보고 전 · blocked)
    pub reorder: Option<ReorderRegion>, // (5.7) 출처 창에만 — 순서 이동 구역 · 얼린 머리. None = 순서 이동 없음(다른 창 · 측정 없음 · 어긋남 · blocked — §2-4)
}
// (5.7) 끌기 시작 때 출처 웹뷰가 dragstart 안에서 잰 탭바 — dock_drag_start 의 선택 칸 · 장부가 세션 내내 그대로 쥔다
pub struct StripStart { pub bar: RectCss /* 탭바 상자(스크롤 상자 자신) · 뷰포트 CSS px */,
                        pub heads: Vec<TabHeader> /* 머리 전부 — 끄는 머리 포함 · 화면(DOM) 순서 · x 는 내용 좌표(잰 x + scroll — 리뷰 1) */,
                        pub grab_x: f64 /* dragstart.clientX − 끄는 머리 x0(뷰포트끼리 — 스크롤 무관) */,
                        pub scroll: f64 /* (리뷰 1) dragstart 때 탭바 scrollLeft — 보고 없는 판정의 스크롤 */ }
// (5.7) 출처 창의 순서 이동 영역 — 웹뷰는 zone 포함 검사 · 자리 · 끄는 머리 왼쪽을 이 값만으로 거울 계산한다
pub struct ReorderRegion { pub zone: RectCss /* = hit::reorder_zone(bar) · 반열림 · 뷰포트 */, pub heads: Vec<TabHeader> /* = strip.heads · 내용 좌표 */,
                           pub dragged: u32 /* heads 안 끄는 머리 자리 */, pub grab_x: f64,
                           pub scroll: f64 /* (리뷰 2) = strip.scroll — pick 의 scroll 이 None 일 때 · 내용이 얼어 있어 bumps 와 무관 */ }
pub struct OuterBand { pub side: DockSide, pub strip: PxRectWire /* 그리기 · 판정 */, pub landing: PxRectWire /* 칠할 결과 */ }
pub struct SlotRegion { pub slot_id: Uuid, pub frame: PxRectWire, pub bands: Vec<Band> /* rect = 결과 사각형 = 띠(R3) */ }
pub struct TabBarRegion { pub rect: RectCss, pub heads: Vec<TabHeader> /* 끄는 탭 뺌 · (리뷰 2) x = 내용 좌표 */, pub insert_xs: Vec<f64> /* 자리 0..=len 의 끼울 x — 탭바 상자 기준 · (리뷰 2) 내용 좌표(그릴 땐 − 지금 scrollLeft) */,
                         pub scroll: f64 /* (리뷰 2) 크롬을 잰 때의 scrollLeft(없음 = 0) — 보고에 scroll 이 없을 때의 판정 스크롤 · bumps 가 견주지 않는다 */ }
// (리뷰 2) 끝 스냅숏 — dock_drag_end 의 선택 칸 · 출처 창이 마지막으로 그린 판정(Over 와 같은 칸) + (리뷰 3) 떠남 간격
pub struct DrawnReport { pub rev: u64, pub bar_rev: u64, pub displayed: Option<Uuid>, pub shown: Picked, pub point: PointCss, pub scroll: Option<f64>,
                         pub left_before_end_ms: Option<f64> /* (리뷰 3) 그 판정 뒤 떠남 → dragend 사건 시각 간격 · None = 떠남 없음 — CANCEL_LEAVE_MS 와 견준다 */ }
pub enum Picked { None, Seam { anchor: Option<Uuid> /* None = 바깥 변 */, side: DockSide }, TabBar { index: u32, over: Option<Uuid> },
                  Reorder { index: u32 } /* (5.7) 출처 창 구역 — 끄는 탭이 놓일 자리 = 결과 탭 목록의 자리(DockTo::TabBar 의 index 그대로) */ }
```

- ★**선택 칸들은 9b 가 9c 보다 먼저 들어가게 하려는 것이다(5.7)**★ — `strip` · `canceled` 는 Tauri 명령의 `Option` 인자라 키가 없으면 `None` 이고, `reorder` 는 셸이 `strip` 이 있을 때만 채운다. (리뷰 1) 보고 · 놓기의 `scroll` 도 `Option`(serde 기본값)이라 7 단계 웹뷰의 보고가 그대로 풀린다. 그래서 9a · 9b 를 넣고 멈춰도 7 단계 웹뷰는 5.6 그대로 돈다. ★(리뷰 2) 단 serde 의 선택은 TS 의 선택이 아니다★ — ts-rs 는 `Option<f64>` 칸을 `scroll: number | null`(키 필수)로 굽는다. 그래서 그런 칸을 더하는 단계가 생성물을 다시 굽고 그 타입을 만드는 기존 TS 자리에 `null` 을 같은 커밋에서 싣는다 — 9a = `DockChrome.scroll`(`dockChromeOf`) · 9b = `DragOverReport` · `DropReport` 의 `scroll`(`dockDrag.ts` 의 `Over` 생성 · 놓기 생성 두 갈래) · TS 시험 고정물(§10). 「9b 동안 7 단계 프론트 무변경」이 아니라 「동작 무변경 · `null` 한 칸씩」이다. `strip` · `canceled` · `last` 는 명령 인자라(`invoke` 맨 객체 — TS 타입이 아니다) 7 단계 호출을 고칠 것이 없고, 새 타입 `DrawnReport.ts` 는 생성물로 더해질 뿐 7 단계에 부르는 자리가 없다. `Picked::Reorder` 는 웹뷰에 새 variant 라 9a 가 TS 거울 · `pickedKey` 팔을 같은 커밋에서 닫는다(tsc 가 짚는다 — §10).

- ★**invoke 인자 철자(6c)**★ — Tauri v2 는 명령의 맨 위 인자 이름을 기본으로 camelCase 로 받고(`drag_id` → JS 쪽 `dragId`) 이 저장소의 명령은 `rename_all` 을 쓰지 않는다. 위 표의 snake_case 페이로드를 그대로 쓰려면 6c 는 인자를 구조체 하나로 받거나(예: `DropReport` — 안쪽 칸은 serde 철자를 따른다) 명령에 `rename_all = "snake_case"` 를 단다 — 둘 중 하나로 정하고 웹뷰(7 단계)와 맞춘다.
- 사각형은 전부 그 창 캔버스(칸 · 띠) 또는 뷰포트(탭바 · 순서 이동 구역) 의 CSS px 다 — 웹뷰는 그 사각형 그대로 점-사각형 검사를 하고 그린다(ADR-0227 결정 1 과 같은 태도). (5.7 · 9a) 예외는 탭바 내용 좌표(탭 머리 x · 끼울 x · 끄는 머리 왼쪽)뿐이다 — 웹뷰는 그것과 견줄 때 점 x 에 탭바 스크롤을 더하고(셸 `pick` 과 같은 식 · §2-4), 그릴 때 지금 `scrollLeft` 를 뺀다. 화면 좌표 · 배율은 웹뷰로 가지 않는다.
- **크롬 보고를 늘 하는 이유** — 끌기 시작 때 보고를 기다리면 첫 영역이 늦다. 탭바 · 캔버스 원점은 드물게 바뀌므로 평소에 보고하고(디바운스 · 한 번에 하나 — `windowCanvasReport.ts` 와 같은 꼴), 셸은 `WindowTabs` 의 측정 칸 `chrome`(`canvas` · `metrics` 옆)에 둔다 — `version` 을 올리지 않는다(ADR-0227 의 측정 규칙). 끄는 동안 떼어진 표시가 머리를 접으면 다시 보고한다.

### 1-6. ③' OS 층 — 새 함수 없음

- 커서 추적 · 맨 위 창 · 버튼 · Esc · 끌기 그림은 엔진(WebView2 브라우저 프로세스의 OS 끌기 루프)이 한다. 셸 메인 스레드는 막히지 않는다(실측 — §8-1).
- 셸이 직접 쓰는 OS 값은 셋이고 전부 Tauri API 다: 커서(`cursor_position`) · 창 사각형(`inner_position` · `inner_size` · `is_visible` · `is_minimized`) · 배율(`scale_factor`). 전부 레이아웃 · 장부 락 밖에서 읽는다(§1-3 「창 게터」). 그래서 `platform` · 게이트 4f · 4g · 4h 명단 · 4i 가 하나도 안 바뀐다. (5.7) 커서 판정도 이 셋만 쓴다 — 커서 밑 맨 위 창(Win32 `WindowFromPoint` 류)을 묻는 함수는 들이지 않고, 그 대신 판정할 창을 그 끌기를 본 창 · 원래 창으로 좁혔다(§1-3 「커서 판정」 · §11-2 #28 · #29). 그 함수가 필요해지면 ADR-0266 절차(platform 목적 함수 · 사용자 보고)다.
- **드롭 처리기 끄기** — `WebviewEnv::finish` 에 `.disable_drag_drop_handler()`(tauri `webview_window.rs:1031`)를 더한다 — 창을 만드는 모든 코드가 지나는 한 곳(ADR-0283)이라 팝아웃 · 앞으로의 런타임 창이 다 덮인다. 켜 두면 wry 가 자기 `IDropTarget` 을 달아 HTML5 끌어 놓기가 막힌다(`wry-0.55.1/src/webview2/drag_drop.rs:45-75` · `mod.rs:150-157`). 잃는 것 = Tauri `DragDropEvent`(우리 코드 사용 0 — rg) · wry 의 외부 드롭 거절(→ §1-4 파일 막기).
- **OS 별 일이 생기면** — 실측(§8-2)이 Esc 구분에 키 · 버튼 상태가 필요하다고 보이면 그때 platform 목적 단위 함수 하나(「주 버튼이 지금 눌려 있나」)를 들인다(ADR-0266 · 사용자 보고). 지금은 넣지 않는다.
- **macOS** — WKWebView 의 창 사이 끌어 놓기 동작은 모른다. 이식 때 §8-2 의 같은 실측을 돌린다.

### 1-7. 계측 (`docs/reference/logging-conventions.md` 「계측 의무」)

끌기는 세션 수명 · 동시성 · 외부 경계(창 만들기 · 숨기기)를 가진 load-bearing 경로다. 그룹 = 기본 모듈 경로(`engram_dashboard::dock` · `engram_dashboard::commands::dock` · `engram_dashboard::layout`). 식별자 · 수치는 필드로, 메시지는 한국어 한 줄. 상태 전이 · 가드 발동(옛 번호 · 옛 `rev` · 모르는 id 버림)은 같은 문서 「동시성 전이」 의무다(5.1 · N7 — 「장부」 줄들).

| 어디 | 무엇 | 레벨 | 필드 |
|---|---|---|---|
| `dock_drag_start` | 끌기 시작(표식) | info | `drag` · `view` · `source` · `tabs` · `hidden` · (5.7) `strip` · `hide_deferred` |
| 같은 곳 | 거절 | debug | `drag` · `view` · `reason`(`TabNotFound` · `AlreadyDragging` · `applying` · `LateStart` — 붙들다 버린 id 의 늦은 시작 · `Dragging` — 옛 표식이 내려가기 전 · `LayoutPoisoned`) |
| 같은 곳 | 출처 숨기기 실패(끌기는 이어짐) | warn | `label` · `: {e}` |
| 실행기 | emit · 보이기 · 커서 · 창 사각형 읽기 실패 | warn(세션당 처음 한 번) · 나머지는 끝 줄 `errors` 셈 | `label` · `: {e}` |
| 실행기 | 늦은 신호(최근 끝난 id) · 모르는 id 의 `Over` · `Left` · `pointer_seen` 버림 | debug | `drag` · `current` · `event` |
| 장부 | `blocked` 전이(출처 배치가 바뀜 — 모든 창 영역 비움) | info | `drag` · `view` |
| 장부 | 옛 스냅숏 번호 버림 · 세션 전 스냅숏 들고 있다가 이어 먹음 | debug | `drag` · `snap` · `last` · `action = drop \| hold \| replay` |
| 장부 | 옛 `rev` 의 보고 → 지금 영역 다시 보냄 | debug | `drag` · `label` · `rev` · `current` |
| 장부 | 모르는 `drag_id` 의 끝 신호 붙듦 · 시작에 이어 먹임 · 버림(시한 · 거절 · 다른 id 가 밀어냄 — 시한 · 밀려남으로 버린 id 는 `ended` 로) | debug | `drag` · `event` · `action = park \| replay \| discard` · `cause` |
| 장부 | 적용 중 스냅숏 · 출처 소멸을 적기만 함 | debug | `drag` · `event` |
| 장부 끝 | 끌기 끝 — 한 줄 | info | `drag` · `outcome = applied \| noop \| aborted` · `reason` · `end_by = drop \| source_none \| drop_wait \| signal_lost \| new_start \| source_gone` · `mode = seam \| tab_bar \| new_window \| reuse` · `window` · `over_window` · `leave_gap_ms`(「마지막 점에 놓기」에서 끝 신호와 W 떠남 보고의 간격 — S1 이 시한을 다시 잰다) · `cursor_read`(커서를 읽었나) · `early`(붙든 신호로 끝났나) · `reports` · `regions_sent` · `regions_resent` · `elapsed_ms` |
| 장부 끝 | `reason` 어휘 | — | `SourceChanged` · `TargetChanged` · `StaleLease` · `ZeroArea` · `WindowClosed` · `TabNotFound` · `WindowNotFound` · `SlotNotFound` · `SlotNotInTab` · `WindowOpen` · `SourceDestroyed` · `SourceGone` · (5.7) `EngineCanceled` |
| 장부 끝(5.7) | 더한 칸 · 낱말 | — | `mode` 에 `reorder` · `end_by` 에 `engine_cancel` · `unseen`(커서 판정으로 끝났나 — 빈 보고 · 보고 없는 끝) · `strip`(시작 측정이 있었나) · `hide_deferred`(탭 하나 팝아웃 숨기기를 미뤘나 · 실제로 숨겼나 = `hidden`) · (리뷰 2) `last = none \| used \| dropped \| not_w`(끝 스냅숏 — 없음 · 썼음 · 숨긴 출처라 버림 · (리뷰 3) 도착 순번 W 가 다른 창이라 안 씀 · (리뷰 4) W 없음 + 간격 > `CANCEL_LEAVE_MS` 도 `not_w`) · (리뷰 4) `lifted`(W 없음이라 `last` 로 W = 출처를 세웠나) · `over_wait = none \| hit \| timeout \| cut`(오는 중 `Over` 붙들기 — 안 붙듦 · `from` 의 `Left` 로 풂 · 시한으로 풂 · 새 시작이 끊음) · `over_wait_ms`(붙든 시간) · `over_from`(붙든 동안 `Over` 가 닿은 창 · 없음 — S1 ① 이 이 셋으로 `OVER_WAIT_MS` 를 다시 잰다) · (리뷰 3) `last_gap_ms`(`last.left_before_end_ms` 그대로 — 출처 사건 시각 · S1 ① · S7 ⑬ ⓓ 가 엔진 접기 간격을 잰다) · `tight`(그 간격이 `CANCEL_LEAVE_MS` 안이라 사각형 검사를 건너뛰었나) · `last_differs`(쓴 `last` 의 `shown` 이 셸이 받은 출처 마지막 `Over` 와 달랐나 — 줄 경주가 실제로 났나 · S7 ⑬ ⓓ) · `late_hide`(시한이 앞지른 끝 — 숨긴 출처를 숨기지 않은 것으로 쳤나) · (리뷰 3) `held_rect`(그 출처를 쥔 `source_rect` 로 사각형 검사했나) |
| 장부(5.7) | 미룬 숨기기 발동(도킹 모드 증거) | debug | `drag` · `label`(출처 창) · `event = over \| left_timeout \| other_over`(리뷰 1) · `by`(`other_over` 의 증거를 낸 다른 창) · `shown` · (리뷰 1) 거른 보고 `filtered = rev0 \| noregion \| blocked \| stale`(`rev` 0 · 출처 창의 지금 영역 없음 · 막힘 · 옛 `rev`) · 끝 신호에 앞질린 떠남 시한도 한 줄(`event = left_canceled`) |
| 장부(5.7) | 빈 보고 → 커서 판정 · 그 결과 | debug | `drag` · `label` · `rev` · `current` · `judged` · `from = last_point \| drop \| held` |
| 장부 시작(5.7) | `strip` 이 있는데 첫 출처 영역에 `reorder` 가 안 섬(§2-4 조건) — 순서 이동 없이 5.6 그대로 | debug | `drag` · `cause = ids \| order \| finite \| dragged` |
| `apply::dock_tab` | 적용 성공 | info | `view` · `mode` · `window` · `closed_window` · `reused` · `main_refilled` · `changed = false`(같은 창 같은 자리 탭바 놓기의 무변경 — 5.5) |
| 같은 곳 | 거절(`DockError`) | debug(마우스 경로는 끝 줄이 남긴다) | `view` · `mode` · `reason` |
| 같은 곳 | 새 창 phase C 가 묘비로 멈춤 · 연 창 거두기 | warn | `label` |
| 같은 곳 | 재사용 창 옮기기 실패(운영 = 종료 중) → `WindowOpen` · 되돌리지 않음(Q19 · 5.6) | warn | `label` · `step` · `: {e}` |
| 실행기 | `Apply` 를 async 로 넘김(먹인 스레드가 동기 명령이었는지) | debug | `drag` · `from_sync` |
| `lib.rs` `CloseRequested` | 팝아웃 모델 먼저 지움 + 묘비(3 단계) | info | `label` · `dropped_views` |

`let _ =` 로 버리는 실패는 없다. 끄는 동안 반복되는 것은 셈으로 끝 줄에 모은다.

## 2. 대상 판정 — 순수 함수

`src-tauri/src/dock/hit.rs`(5 단계 · 끝) · `src-tauri/src/dock/regions.rs`(6 단계) — Tauri · 락 · OS 호출 0. 같은 입력이면 같은 답이다(R6a ③).

### 2-1. 입력 · 출력

```rust
// hit.rs (끝)
pub const OUTER_BAND_PX: u32 = 32;                                   // R5 — CSS px 정수(그린 띠와 판정이 같은 정수 사각형)
pub const SIDE_ORDER: [DockSide; 4] = [Left, Right, Top, Bottom];    // 동률 깨기
pub struct WindowFrame<'a> { label, inner_origin: (f64, f64), scale: f64, chrome: &'a DockChrome, canvas: CanvasPx,
                             visible_view: Uuid, slots: &'a [(Uuid, PxRect)] }
pub enum Locate { Nothing, TabBar { label, index, over }, Outer { label, view, sides, slot, p }, Slot { label, view, slot, frame, p } }
pub fn locate(cursor: (f64, f64), top: Option<&WindowFrame>, source_view: Uuid) -> Locate;            // 1 단계 — 어디 위인가
pub fn choose_seam(loc: &Locate, outer: &[(DockSide, PxRect)], bands: &[(DockSide, PxRect)]) -> Option<(DockAnchor, DockSide)>;
pub fn choose_band(p: (f64, f64), frame: PxRect, bands: &[(DockSide, PxRect)]) -> Option<DockSide>;
pub fn landing_rect(target, anchor, side, incoming, canvas) -> Option<PxRect>;   // 띠 = 놓았을 때 끼운 것의 px 사각형(R3)
pub fn valid_bands(target, anchor, incoming, canvas) -> Vec<(DockSide, PxRect)>; // 면적 0 변은 빠진다(R8)
pub fn outer_strips(canvas) -> [(DockSide, PxRect); 4];
pub fn tab_insert_x(chrome, source_view, index) -> f64;
// (5.7 · 9a) 순서 이동 — §2-5
pub const REORDER_SLACK_PX: f64 = 15.0;
pub fn reorder_zone(bar: RectCss) -> RectCss;                                        // x 그대로 · y 위아래로 REORDER_SLACK_PX
pub fn reorder_left(heads: &[TabHeader], dragged: usize, grab_x: f64, x: f64) -> f64; // 끄는 머리 왼쪽 — 커서를 따르되 머리들 범위 안 · x = 점의 내용 x(p.x + scroll — 리뷰 1)
pub fn reorder_index(heads: &[TabHeader], dragged: usize, left: f64) -> usize;        // 그 왼쪽에서 끄는 탭이 놓일 자리

// regions.rs (6a — 끝 · 5.7 고침 = regions_for 의 strip 인자 · reorder 칸 · pick 의 첫 단계 · bumps)
pub struct WindowSnap { label, tabs: Vec<ViewId>, active: ViewId /* 모델 활성 */,
                        visible: Option<(ViewId, LayoutNode)> /* 6c 스냅숏 빌더가 visible_tab 으로 채운다 — regions_for 는 그 값을 믿고 debug 빌드에서만 debug_assert_eq! 로 visible_tab 과 대조한다(숨긴 출처 조기 반환 뒤 · release 에선 빠짐) */,
                        canvas: Option<CanvasPx>, chrome: Option<DockChrome>,
                        hidden: bool /* 장부 값(source_hidden) — 게터 아님 · §1-3 */, is_source: bool }
pub struct Expect { layout: Option<LayoutNode> /* 보이는 탭 배치 */, active: ViewId /* 모델 활성 */, tabs: Vec<ViewId> }
impl Expect { fn seam(&self) -> Option<TargetExpect> /* Seam{layout, active} · 보이는 탭 없으면 None */; fn tabs(&self) -> TargetExpect /* Tabs */ }
pub fn visible_tab(tabs: &[ViewId], active: ViewId, source_view: ViewId) -> Option<ViewId>;  // 활성이 끄는 탭이면 succession · 아니면 활성
pub fn regions_for(w: &WindowSnap, source_view: Uuid, incoming: &LayoutNode, blocked: bool,
                   strip: Option<&StripStart> /* 5.7 — 장부의 Drag.strip · 출처 창(w.is_source)에만 쓴다 */)
    -> Option<(DockRegions /* drag_id · rev · bar_rev 는 비워 두고 장부가 채운다 */, Expect /* blocked 와 무관하게 모델 값 */)>;   // None = 숨긴 출처
pub fn pick(r: &DockRegions, p: PointCss, scroll: Option<f64> /* (5.7 · 리뷰 1 · 리뷰 2) 그 창 탭바 scrollLeft — reorder 자리 · 탭바 자리(R21 · over)에 든다 · None = 영역마다 잰 때의 것(reorder = strip.scroll · tab_bar.scroll) · 구역 · 탭바 상자 · 캔버스 검사는 p 그대로 */, source_view: Uuid) -> Picked;   // = (reorder 구역이면 Reorder) → locate + choose_seam 을 영역 값 위에서
pub struct Bumps { rev: bool, bar_rev: bool }
pub fn bumps(sent: Option<(&DockRegions, &Expect)>, next: (&DockRegions, &Expect)) -> Bumps;
    // 처음 보냄 = 둘 다 · rev = 영역 몸이나 기대값이 바뀜(활성 전환 · 내용만 바뀜 포함) · bar_rev = 탭바 기하나 Tabs 가 바뀜(그때 rev 도) — drag_id · rev · bar_rev 칸은 견주지 않는다
```

`pick` 은 새 규칙이 아니다 — 영역에서 `WindowFrame`(안쪽 원점 (0, 0) · 배율 1 · 크롬 = 영역의 탭바 · 머리 · 캔버스 원점)과 띠 목록을 다시 세워 `locate` → `choose_seam` 을 부른다. 규칙의 정본은 `hit.rs` 하나이고, 웹뷰의 `pickTarget` 은 그 거울이다(§2-4 골든). (리뷰 2) 탭바 단계가 내용 x(`css.x + s`)로 견주므로 `pick` 은 `hit` 의 탭바 규칙에 `s` 를 넘긴다 — 규칙은 그대로 `hit` 에 둔다(`locate` 가 스크롤을 받게 넓히든 탭바 단계를 `hit` 함수로 빼든 9a 가 고른다 · 기존 `locate` 시험은 `s = 0` 으로 그대로).

### 2-2. 규칙 (`hit.rs` — 5 단계에 박힌 그대로)

0. **(5.7) 순서 이동 구역** — 영역에 `reorder` 가 있고(출처 창 · §2-4) 점이 그 `zone` 안(반열림)이면 `Reorder{index}`(§2-5) — 아래 어느 규칙보다 먼저다(탭바 · 바깥 위 띠 · 칸 위 띠를 이긴다). 이 단계는 `pick`(regions.rs)의 첫 줄이고 규칙 함수는 `hit` 의 `reorder_*` 다 — `locate` 는 영역 밖의 일반 판정으로 그대로 둔다.
1. **맨 위 창 없음** → `Nothing`. (5판: 맨 위 창은 엔진이 고른다 — 사건을 받은 창이 그 창이다.)
2. **창 안쪽 좌표** — `css = (cursor − inner_origin) / scale`. 5판의 실행 경로는 뷰포트 CSS px 를 그대로 넣는다(원점 (0, 0) · 배율 1) — 화면 좌표 변환이 판정에서 빠졌다(W4 가 좁아졌다).
3. **탭바** — `css` 가 탭바 안이면 `TabBar`. ★머리 목록에서 `view_id == source_view` 인 것을 먼저 뺀다★ — 거른 목록 = 결과 목록이라 `index` = 가운데 x 가 커서보다 왼쪽인 머리의 수 · `over` = 커서를 품은 머리 · (리뷰 2) 이 두 견줌의 커서 x 는 내용 x(`css.x + s` — `s` = `pick` 의 스크롤 · 머리가 내용 좌표다) · 「탭바 안」 검사는 `css` 그대로. id 로 거르는 것은 목록만 고친다 — 기하는 다음 크롬 보고가 고친다(그 틈의 처리 = §2-4).
4. **캔버스** — `p = css − canvas_origin` · `p ∉ [0, W) × [0, H)` 면 `Nothing`.
5. **끄는 탭 자신** — `visible_view == source_view` 면 `Nothing`(방어 — 보이는 탭이 없는 창도 `Nothing`).
6. **바깥 띠(R5 · R6a ①)** — `outer_strips` 의 반열림 사각형에 든 변 **전부**를 캔버스 변까지 거리가 작은 순(동률 `SIDE_ORDER`)으로 `Outer.sides` 에 싣고 그 밑 칸 · `p` 도 싣는다. 슬롯 띠보다 먼저 본다(AC4).
7. **칸 찾기** — 반열림 `[x0, x1) × [y0, y1)` · 이웃 칸은 같은 정수 경계를 나눠 점 하나에 칸 하나다.
8. **띠 고르기(`choose_seam`)** — `Outer` 면 `sides` 중 유효한 첫 변 · 품은 변이 전부 무효일 때만 그 밑 칸 띠로 넘어간다(§11-1 #5). 칸에서는 점이 띠 사각형(그린 그대로 · 칸 틀 밖으로 삐진 띠는 보지 않음) 안이면 후보 · 없으면 `None`(가운데 — R6) · 둘 이상(모서리)이면 칸 변까지 거리 ÷ 띠 두께가 가장 작은 띠(R6a ②) · 동률 `SIDE_ORDER`.

**경계 포함 규칙(R6a ③)** — 칸 · 탭바 · 바깥 띠 · 칸 띠 전부 그린 사각형 그대로의 반열림(왼쪽 · 위 포함) · 탭바 자리는 가운데 x 와 같으면 오른쪽 · 동률은 늘 `Left → Right → Top → Bottom` · 부동소수 비교만(반올림 없음).

### 2-3. 바깥 띠 두께 = 32 CSS px

Eclipse 4 의 35px(조사 §7)와 같은 크기대 · 탭바 높이(28px — `TabBar.tsx:65`)보다 조금 두꺼워 노리기 쉽다 · CSS px 라 배율과 무관하게 같은 눈 크기 · 값은 `hit.rs` 상수 하나(R5 「값은 TRD」).

### 2-4. 영역 만들기 · 거울 · 골든

- **창별 영역(`regions_for`)** — 숨긴 출처는 만들지 않는다. 보이는 탭 = `regions::visible_tab`(모델 활성 · 활성이 끄는 탭이면 `succession` — 떼어진 표시의 `display_active` 와 같은 규칙 · 6a 가 `regions.rs` 에 새로 짰다 — 4판 `session.rs` 에서 옮기지 않는다)이고(6c 스냅숏 빌더가 `WindowSnap.visible` 을 이것으로 채운다 — `regions_for` 의 `debug_assert_eq!` 가 debug 빌드에서 어긋남을 잡는다), `view_id` 는 측정 전에도 그 모델 값이다. 측정이 없으면 캔버스 = 0×0 · 캔버스 원점 = (0, 0)이다. `blocked` 면 몸을 비운다(`view_id` 없음 · 원점 · 캔버스 0 · 칸 · 바깥 · 탭바 없음) — 기대값은 모델 값 그대로다. 칸 · 바깥은 크롬 · 캔버스 · 보이는 탭이 다 있어야, 탭바는 크롬이 있어야 싣는다.
  - `outer` = `valid_bands(보이는 탭 배치, Outer, expect_source, canvas)` 의 변만 `outer_strips` 의 띠(그림 · 판정)와 결과 사각형(칠할 것)으로.
  - `slots` = 칸마다(`geometry::frame_px`) `valid_bands(배치, Slot(칸), expect_source, canvas)` — 면적 0 띠는 빠진다(R8 · AC15). 비용 = 칸 수 × 4 + 4 번의 `docked_tree` — 끄는 동안 계기마다 · 트리가 작아 ms 이하(§11-2 #9).
  - `tab_bar` = 크롬 탭바 + 출처를 id 로 뺀 머리 + 자리마다 `tab_insert_x` · (리뷰 2) `scroll` = 크롬 `scroll`(없음 = 0). 머리 · 끼울 x 는 크롬이 실은 내용 좌표 그대로다(`tab_insert_x` 는 탭바 상자 x0 를 뺄 뿐이라 식이 그대로 선다) — 판정은 `pick` 이 점의 내용 x(`p.x + s`)로 머리 가운데와 견주고, 탭바 상자 포함 검사는 `p` 그대로다(상자는 스크롤로 안 움직인다). `bumps` 는 `scroll` 을 견주지 않는다(기하가 아니다 — 견주면 다른 계기의 다시 잼마다 `bar_rev` 만 헛돈다). 출처 창의 크롬에 접히기 전 출처 머리가 아직 남아 있으면(떼어진 표시 뒤 첫 보고 전) 기하가 옛것이라 끼울 자리 막대가 머리 하나 폭만큼 어긋나 보일 수 있다 — 그래도 그림과 판정이 같은 영역(같은 `rev`)이라 보인 그대로 들어가고, 접은 뒤의 보고가 오면 `rev` 가 올라 맞춰진다(§11-2 #10). 4판처럼 그 틈에 탭바를 비우면 출처 탭바 위 짧은 떼기가 「가이드 없음 = 새 창」이 되므로 비우지 않는다. (5.7) 출처 창에 `reorder` 가 서면 그 창 탭바 위는 구역이 먼저라 이 기하는 `reorder` 가 빠졌을 때의 물러날 자리로만 쓰인다(아래 「출처 창의 `reorder`」).
  - 기대값 = `Seam{보이는 탭 배치, 그 창 모델 활성}`(활성 = `WindowSnap.active`) · `Tabs(그 창 탭 목록)` — 영역이 같아도 기대값이 바뀌면 장부가 `rev` 를 올린다 · 탭바 영역 · `Tabs` 가 바뀔 때만 `bar_rev` 도(§1-3 「영역」).
- **TS 거울(`dockPick.ts` `pickTarget`)** — 영역 위에서 같은 순서로: 탭바(반열림 · `index` = 가운데 x ≤ 점인 머리 수 · `over`) → 캔버스 밖 · 보이는 탭 없음 = 없음 → 점을 품은 유효 바깥 띠를 변까지 거리 · `SIDE_ORDER` 순으로 첫 것 → 아니면 밑 칸의 띠를 정규화 거리로(`choose_band`) → 없음. 유효한 바깥 띠만 오므로 「품은 변이 전부 무효면 칸 띠로」가 따로 손대지 않아도 같다.
- **골든 `src-tauri/src/dock/pick_golden.json`**(6a — 끝) — 꼴 = 맨 위 `source_view` 하나 · 이름 붙은 `regions`(dock:regions 로 보내는 wire 꼴 그대로 — Rust 시험이 `regions_for` 로 다시 만들어 견준다) · `cases`(영역 이름 · 점 · 기대 `Picked`). 지금 영역 13 · 사례 132(9a 뒤) — 캔버스 원점 (40, 28) · 탭바 x0 40 인 `offset` 영역과 위 띠 경계 한 쌍을 포함한다. 7 단계의 TS 거울(`pickTarget`)이 같은 파일을 읽는다. Rust 시험(`pick`)과 vitest(`pickTarget`)가 같은 파일을 먹는다 — 규칙을 고치면 골든 · TS 판을 같은 변경에서 고친다(갈라지면 한쪽이 빨개진다 — 대기 입력 골든 `queued_input_golden.json` 과 같은 꼴). 사례는 §9 판정 행의 경계를 다 덮는다. 갈라지면 놓기가 「보인 것과 다름 = 무변경」으로 떨어진다 — 잘못 붙지도, 본 가이드 대신 새 창이 뜨지도 않는다(5.1 · §1-3 「놓기 판정」).
- **(5.7) 출처 창의 `reorder`** — `regions_for` 가 `w.is_source` 인 창에 장부의 `strip` 으로 `ReorderRegion{zone = reorder_zone(strip.bar), heads = strip.heads, dragged, grab_x, scroll = strip.scroll(리뷰 2)}` 를 싣는다. ★싣는 조건 — 하나라도 어기면 `None`(그 끌기의 그 창은 5.6 그대로 — 원래 창 탭바 = R21 끼울 자리 · 처음부터 도킹 모드)★: ① `strip` 이 있다 ② `blocked` 가 아니다(막힌 영역은 몸을 비운다 — 5.6) ③ 숨긴 출처가 아니다(숨긴 출처는 영역 자체가 없다) ④ `strip.heads` 의 id 를 순서대로 늘어놓은 것 = 그 창 **모델** 탭 목록(`WindowSnap.tabs`) — 끄는 동안 다른 경로가 탭을 더하거나 빼거나 옮기면 얼린 기하가 모델과 갈라지므로 그 순간부터 빠진다(모습도 거둔다 — §4-4) ⑤ 끄는 머리(`view_id = source_view`)가 정확히 하나 있다(`dragged` = 그 자리) ⑥ 머리의 `x0` 가 오름차순(화면 순서 = 왼쪽부터) · 모든 수가 유한 · 각 머리 폭 ≥ 0. ★`tab_bar` 는 그대로 싣는다★ — `reorder` 가 있으면 구역(⊇ 탭바)이 먼저라 닿지 않고, 빠지는 순간의 물러날 자리가 된다.
  - **`bumps`** — `reorder` 가 바뀌면(생김 · 빠짐 — 세션 안에서 내용은 얼어 있어 그것뿐이다) `rev` · `bar_rev` 를 둘 다 올린다(탭바 몫이다). `same_body` 구조 분해에 칸이 하나 는다.
  - **왜 시작 측정을 얼리나(셸 크롬 · 살아 있는 측정을 쓰지 않는 까닭)** — ① 도킹 모드에서 머리가 접히면 크롬 보고에서 끄는 머리가 빠져(접힌 머리는 싣지 않는다 — §1-5) 그 폭을 잃는다 — 구역으로 돌아온 첫 판정이 그 보고 왕복을 기다리게 된다 ② 순서 이동 모습(transform)이 걸린 머리를 재면 비켜난 자리가 실려 판정이 자기 그림을 따라 흔들린다 ③ 셸의 크롬은 100 ms 디바운스라 끌기 직전 바뀐 탭바(이름 바꾼 직후)에 늦을 수 있다(리뷰 2 — 스크롤은 크롬이 내용 좌표라 더는 늦을 거리가 아니다). `dragstart` 안 측정은 그 순간 DOM 그대로다. ④ (리뷰 1) 머리를 내용 좌표로 얼리므로 끄는 중 탭바 스크롤(엔진 자동 스크롤 · 우리 끝 띠 — §1-4 「모드」)도 기하를 흔들지 않는다 — 바뀌는 스크롤은 보고가 싣는다(§2-5).
  - **골든(9a)** — 영역 `source_strip`(머리 넷 · 폭이 다른 머리 · 탭바 x0 40 · 캔버스 원점 (40, 28) — `offset` 과 같은 꼴) · `source_lone`(머리 하나)을 더하고, 사례에 기대 `Picked` 와 함께 끄는 머리 왼쪽 `left` 를 싣는다(웹뷰가 그리는 자리도 같은 파일로 묶는다 — `left` 는 순서 이동 사례에만). 사례가 덮을 것 = §2-5 의 경계 전부(구역 위 · 아래 경계 · 여유 안 칸 위 띠 · 바깥 위 띠 자리 · 탭바 오른쪽 빈 곳 · 왼쪽 · 오른쪽 이웃 가운데와 같음 · 1 ULP 넘음 · 양 끝 고정 · 머리 하나). (리뷰 1) 사례마다 `scroll`(없으면 스크롤을 주지 않는다 = 그 영역이 잰 스크롤 `reorder.scroll` · `tab_bar.scroll` — (리뷰 2) `pick` 의 `None`)을 싣고 넘친 탭바 영역 `source_overflow`(탭바 폭보다 넓은 머리들)를 더한다 — 같은 점 · 다른 스크롤 = 다른 자리 · 스크롤한 채 양 끝 고정 · 구역 검사는 스크롤과 무관. (리뷰 2) 다른 창 넘친 탭바 영역 `target_overflow`(내용 좌표 머리 · 끼울 x · `tab_bar.scroll`)를 더한다 — 같은 점 · 다른 스크롤 = 다른 `TabBar` 자리 · `over` · 사례 `scroll` 없음 = `tab_bar.scroll` 로 판정 · 탭바 상자 경계 · 캔버스 판정은 스크롤 무관. (구현 9a) 1 ULP 사례는 영역 `source_ulp`(이웃 가운데가 짧은 소수 점에서 정확히 1 ULP 떨어진 머리)에 둔다 — serde_json 기본 기능(`float_roundtrip` 없음)이 17 자리 소수를 1 ULP 어긋나게 읽어(실측 `119.99999999999999` → `120.0`) 1 ULP 옆 점을 골든에 바로 적지 못한다. 「여유 안 칸 위 띠」는 `regions_for` 가 만드는 영역에서 늘 바깥 위 띠가 덮으므로 손으로 세운 영역 시험(Rust `regions.rs` · TS `dockPick.test.ts` 짝)이 잰다.
- **TS 거울 순서(5.7)** — `pickTarget` 은 `reorder` 구역 → 탭바 → 캔버스 순서다. `reorderLeft` · `reorderIndex` 를 `hit` 과 같은 f64 연산 · 같은 순서로 낸다(`min` · `max` 로 고정 · 가운데 = `(x0 + x1) / 2` · 점의 내용 x = `p.x + scroll` 을 먼저 한 번 더한다 — 리뷰 1). (리뷰 2) 탭바 단계도 같은 내용 x 로 `index` · `over` 를 낸다 · `scroll` 이 `null` 이면 영역마다 잰 때의 것(`reorder.scroll` · `tab_bar.scroll`).

### 2-5. 순서 이동 구역 · 자리 (5.7 · PRD R23 · 사용자 결정 Q20 · Lumino 규칙)

얼린 머리 `H[0..n)`(화면 순서 · 끄는 머리 `d` 포함) · 각 `[x0, x1)`(내용 좌표 — 스크롤 0 일 때의 뷰포트 x · 리뷰 1) · 끄는 머리 폭 `w = H[d].x1 − H[d].x0` · 잡은 자리 `g = grab_x` · 점 `p`(뷰포트 CSS px) · 스크롤 `s`(그 판정 때 탭바 `scrollLeft` — 보고 · 놓기의 `scroll` · 없으면 `strip.scroll`) · 점의 내용 x `q = p.x + s`.

1. **구역** — `zone = { x0: bar.x0, x1: bar.x1, y0: bar.y0 − 15, y1: bar.y1 + 15 }` · 반열림(왼쪽 · 위 포함). 가로로 구역을 나가면 — 창 밖(문서 밖 `dragleave`)이든 탭바 옆의 다른 크롬 위(메인처럼 탭바가 창 폭 전체가 아닌 창)든 — 도킹 모드다(§1-4 「모드」 · 문서 밖 떠남은 `LEAVE_GRACE_MS` 뒤 — 리뷰 1). 위 여유는 지금 탭바가 뷰포트 맨 위(`y0 = 0`)라 닿지 않지만 탭바 자리가 바뀌어도 같은 규칙이 서게 둔다.
2. **끄는 머리 왼쪽** — `L = max(H[0].x0, min(q − g, H[n−1].x1 − w))`(내용 좌표 · 커서를 따르되 머리들이 차지한 범위 `[H[0].x0, H[n−1].x1)` 안 · `+` 버튼 · 빈 탭바 위로 나가지 않는다 · 범위가 `w` 보다 좁은 닿지 않는 경우는 `H[0].x0`). 오른쪽 `R = L + w`. 가로만이다 — `p.y` 는 구역 판정에만 쓴다(구역 안에서 위아래로 흔들려도 머리는 탭바 안에 머문다).
3. **자리** — `index = a + b` · `a` = 왼쪽 이웃(`i < d`) 중 가운데 `m_i = (H[i].x0 + H[i].x1) / 2` 가 `m_i ≤ L` 인 수 · `b` = 오른쪽 이웃(`i > d`) 중 `m_i < R` 인 수. 같으면 비키지 않는다(왼쪽 = `L = m_i` 면 그대로 · 오른쪽 = `R = m_i` 면 그대로 — Lumino 의 `targetPos < threshold` · `targetEnd > threshold` 와 같다). `m_i` 가 오름차순이라 비키는 왼쪽 이웃은 `[index, d)` 의 연속 구간 · 오른쪽은 `(d, index]` 이고 둘이 함께 서지 않는다(머리가 붙어 있고 폭이 양수면 `m_{d+1} − m_{d−1} > w` 라 `L < m_{d−1}` 과 `L + w > m_{d+1}` 이 같이 설 수 없다). `index` = 끄는 탭이 놓일 자리 = 결과 탭 목록(끄는 탭을 뺀 목록에 끼운 자리)이라 `DockTo::TabBar{index}` 의 뜻 그대로다(`min(index, 길이)` · 같은 자리면 무변경).
4. **머리 하나** — `n = 1` 이면 `L = H[0].x0` · `index = 0` — 늘 제자리다(PRD D18). 구역은 그대로 서므로 탭 하나 팝아웃은 그 안에 머무는 동안 숨지 않는다(§1-3 「도킹 모드 증거」).
5. **부동소수** — 반올림 없음 · `f64` 비교만(R6a ③). TS 는 같은 식을 같은 순서로 계산한다(골든).

- **여유 15 px 의 근거** — Chrome 탭 스트립의 떼기 문턱 `kVerticalDetachMagnetism = 15`(조사)와 같고, 붙을 때 · 뗄 때 같은 넓힌 사각형 하나를 쓰는 꼴도 Chrome 과 같다. Lumino 는 탭바 밖 20 px 다(조사) — 사용자가 「Chrome 처럼 · 약 15 px」로 정해 Chrome 값을 따랐다. 대가: 출처 창에서는 캔버스 맨 위 15 px 이 구역에 들어가 바깥 위 띠(32 px)의 남은 몫이 17 px 이 되고, 그 자리의 칸 위 띠도 15 px 만큼 덜 닿는다(다른 창은 그대로 · 골든이 경계를 묶는다). 바깥 위 띠는 15 px 아래부터 그려진 띠 그대로 고를 수 있다.
- **이력(hysteresis) 없음** — 들어올 때 · 나갈 때 같은 경계다(PRD D19). 이력을 두면 `pick` 이 앞 모드를 입력으로 받아야 해 셸이 놓기에서 같은 답을 다시 낼 수 없거나(보고 없는 끝 — 앞 모드를 모른다) 보고에 모드를 더 실어야 한다. 경계가 탭바에서 15 px 떨어져 있어 거기서 머무는 일이 드물다 — 흔들림은 S7 ③ 이 잰다(§11-2 #30).
- **이웃이 비키는 폭 = `w`** — 머리들이 빈틈 없이 붙어 있다(`TabBar` 의 flex · 간격 없음). 빈틈이 생기면 그림이 그만큼 어긋나지만 자리(판정)는 위 식 그대로라 보인 자리로 들어간다.
- **스크롤(사용자 지시 2026-10-17 · 리뷰 1)** — 구역은 탭바 상자(스크롤 상자 자신 · 뷰포트)라 스크롤로 움직이지 않고, 구역 검사는 `p` 그대로다. `H` · `L` · 자리는 내용 좌표라 화면 x = 내용 x − `s` 이고, 끄는 머리 화면 왼쪽 = `L − s` = `p.x − g`(양 끝에 고정되지 않을 때) — 탭바가 밀려도 커서 밑에 머문다. 셸 · 웹뷰가 같은 (점, 스크롤) 짝으로 같은 식을 돌리므로 보인 자리 = 적용하는 자리가 그대로 선다(4c). 끝에 대고 있을 때 미는 쪽은 엔진 자동 스크롤 · 그것이 안 되면 우리 끝 띠다(§1-4 「모드」) — 어느 쪽이든 판정은 `s` 만 받는다.

## 3. LLM 표면 — 이번에 없다 (의식적 예외)

- **사용자 결정(PRD Q10 · 2026-10-16)** — 이 기능은 LLM 명령(`tab.dock` 류 · `layout.get`)도 프론트 레지스트리 명령도 만들지 않는다. CLAUDE.md 「LLM-우선 제어」(「새 UI 기능엔 LLM 호출 경로를 함께」)의 **의식적 예외**이고, 도킹 바로 다음 작업이 닫는다(메모 = `.claude/handoff/attachments/llm-layout-surface-notes-2026-10-16.md` — 첫 단계 = LLM 도구 설계 관례 조사 · 흔한 요청을 1–2 호출로).
- **남기는 것** — `apply::dock_tab`(마우스 놓기가 쓰는 단일 적용 경로) · 타입드 `DockError` · `FocusPolicy::Keep` · 표식의 `Dragging` 판정. 다음 작업의 명령은 그 위에 껍데기만 얹는다 — 두 번째 변경 경로를 짓지 않는다(ADR-0081 결정 3).
- **끌기 자체는 명령이 아니다** — `dock_*` invoke 는 사람 손의 입력과 그 측정이라 레지스트리 · 버스에 싣지 않는다.
- **(5.7) 탭 순서 바꾸기도 같은 예외다** — 지금 버스 · `__engramCmd` 에 탭 순서를 바꾸는 명령이 없고(`rg` 0), 순서 이동은 마우스에 새로 생기는 동작이다. 같은 적용 경로(`apply::dock_tab` 의 `TabBar{같은 창, index}`)를 쓰므로 다음 작업의 명령이 그 위에 얹으면 된다 — 두 번째 경로를 짓지 않는다. ★사용자 결정 Q10 의 예외 범위에 순서 이동을 넣은 것은 메인 판단이다★(PRD Q20 은 LLM 표면을 말하지 않는다) — 8 단계의 「남은 갭」 한 줄에 함께 적는다. (리뷰 1) 사용자가 보게 PRD D22 에 올렸다. 정당한 까닭 = 5.6 까지도 원래 창 탭바 놓기(R21 끼울 자리)가 같은 `dock_tab` 으로 순서를 바꿨고 Q10 이 그것을 이미 덮었다 — 순서 이동은 새 변경 경로가 아니라 같은 경로의 새 손잡이다.
- 위험 · 후속 = §11-2 #15 · 8 단계에서 CLAUDE.md 「LLM-우선 제어」 「남은 갭」에 한 줄(`/review doc`).

## 4. 떼어진 표시 · 끌기 그림 · 가이드 · 순서 이동 모습 (R2 · R23 · W7 · W8 · W13 · W14)

### 4-1. 떼어진 표시 — 그리기만

- ★**(5.7) 떼어진 표시는 출처 창이 도킹 모드일 때만이다**★(§1-4 「모드」) — 순서 이동 모드(구역 안)에서는 머리를 접지 않고 캔버스도 모델 활성 그대로이며, 대신 §4-4 의 모습을 그린다. 아래 문단은 도킹 모드의 모습이다.
- **탭이 여럿인 창 · 메인** — 창은 제자리다. ★스토어의 창 탭 목록(`win.tabs`)을 거르지 않는다★ — `WindowLayout.tsx:152-155` 는 그 목록이 0 개가 되면 자기 창을 닫는다(0탭 자가닫힘). 떼어진 표시는 렌더 때만 쓰는 별도 값(`detached: { view, displayActive } | null`)이다: `TabBar` 는 그 머리를 CSS 로 접고(DOM 에는 남긴다 — §1-4), 캔버스는 모델 활성이 그 탭인 동안 `displayActive` 탭을 보인다(keep-alive 표시 토글 — `WindowLayout.tsx:196-200` · ADR-0056). 승계는 셸이 계산해 보낸다(`succession` — 모델 · 판정과 같은 규칙).
- **메인의 마지막 탭** — `displayActive = null` 이라 메인은 빈 탭바 · 빈 캔버스(테마 배경)다. 빈 탭은 놓을 때 모델이 만든다(§1-2 — 끄는 동안 모델은 그대로).
- **탭이 하나뿐인 팝아웃** — 떼어진 표시 대신 셸이 창을 숨긴다(Tauri `hide()` · §1-3 「시작」 · 5.7 — 출처 영역에 `reorder` 가 서면 그 끌기가 구역을 벗어날 때 · 「도킹 모드 증거」). 끝에서 무변경이면 되보이고, 재사용(R22)이면 옮긴 자리에서 보이며, 붙으면 빈 창이라 적용 서비스가 닫는다. 숨김은 작업 표시줄에서도 빠지므로 끄는 중 사용자가 그 창을 닫을 길은 없다(LLM · 다른 경로로 닫히면 `Destroyed` → `SourceDestroyed`).
- `(drag_id, seq)` 가 오래된 것은 버린다. 모델에는 아무것도 보내지 않는다.

### 4-2. 끌기 그림 (PRD R2 · Q5)

- **모양 = Windows 기본** — `setDragImage(요소, 커서 자리)` 로 넘긴 요소를 엔진이 그림으로 떠 OS 가 그린다: 반투명 · 오른쪽 · 아래로 흐려짐 · 끌어 놓기를 받지 않는 다른 앱 창 위에선 사라짐(실측 · 받아들임). 따로 된 창 · 페이지 · 권한이 없다.
- **요소 = 화면 밖에 늘 붙어 있는 그림 상자 하나**(`dragImage.ts` — `position: fixed` · 화면 밖 좌표). `setDragImage` 는 `dragstart` 안에서만 부를 수 있고 그 순간 그려진 요소를 쓰므로, 상자를 `dragstart` 안에서 동기로 채운다: 위 = 머리 띠(탭 제목 · 높이 28 논리 px) · 아래 = `<canvas>` 축소 그림.
- **축소 그림 — 되는 만큼**(결정 2 · S2) — 끄는 탭이 그 창에 보이는 탭이면 칸마다: 터미널 칸은 그 xterm WebGL 캔버스를 `drawImage` 로 줄여 옮긴다 · 그 밖(챗 · 목록 · 사용량 · 빈 칸 — DOM 이라 동기로 그림에 못 옮긴다)은 테마 패널 색 + 칸 이름 자리채움. 칸 사각형은 그 칸 DOM 의 `getBoundingClientRect` 를 캔버스 원점 기준으로 줄인다. ★WebGL 캔버스는 `preserveDrawingBuffer` 없이는 그린 프레임 밖에서 비어 읽힐 수 있다(가능성 높음)★ — S2 가 「켜기(`WebglAddon` 생성자 인자) · 끄기」로 재고, 비면 그 칸도 자리채움이다. 보이지 않던 탭이면(그 칸들은 `display:none` 이라 그릴 것이 없다) 제목만(§11-1 #7).
- **크기(논리 px · 웹뷰가 계산 — 놓기 판정과 무관한 모양 값)** — 그림 있음 = R22 새 창 크기(팝아웃 = 그 창 `innerWidth × innerHeight` · 메인 = 팝아웃 기본 720×500) × `DRAG_IMAGE_SCALE`(0.4)를 비율 그대로 `DRAG_IMAGE_MAX`(320×240) 안에 들게 줄이고, 너비가 `DRAG_IMAGE_MIN_W`(180)보다 작으면 상한 안에서 키운다 · 그림 없음 = 180×28(제목만). 예: 메인에서 떼기 → 288×200 · 1600×1000 팝아웃 → 320×200 · 400×300 팝아웃 → 180×135. 엔진의 요소 그림 상한(변 8192px)보다 한참 작다(조사 §3).
- **커서 자리** — `DRAG_IMAGE_HOTSPOT` (16, 14) CSS px — 커서가 머리 띠 왼쪽 안에 앉아 제목이 커서 옆에 보인다. 그림이 반투명이라 그 밑 가이드가 비친다(4판의 「불투명 딱지가 가이드를 가림」이 사라졌다). 이 값을 `dock_drag_start` 의 `hotspot` 으로 셸에 알리고, R22 새 창은 「커서 − hotspot × 출처 배율」에 뜬다 — 그림이 있던 자리(§1-3). 엔진 기본 그림으로 물러났을 때는 누른 점의 머리 안 자리를 싣는다.
- **실패** — 그림 만들기에서 무엇이 던지면 잡아 제목만 상자로 `setDragImage` · 그것도 실패하면 엔진 기본(끌린 머리 요소의 그림). 끌기는 늘 이어진다(PRD R2).

### 4-3. 가이드 그리기 (`DockOverlay.tsx`)

- 캔버스 div(`position: relative`) 안에 `position: absolute; pointer-events: none` 로: 커서 밑 칸의 띠(빗금 — R3) · 바깥 띠(빗금) · 고른 띠의 결과 사각형(반투명 강조 — R4 · R5 · 바깥이면 `landing`). 탭바 위에는 끼울 자리 세로 막대(R21 · `insert_xs[index]` — (리뷰 2) 내용 좌표라 그 창 탭바의 **지금** `scrollLeft` 를 빼 그리고 탭바 상자 안으로 자른다 · 막대 x 는 그 자리를 고른 판정과 같은 스크롤로 · 스크롤 사건의 다시 판정이 함께 옮긴다) · 전환 대기 진행 표시(R6b — 머리 자리도 같은 변환). 색은 테마 CSS 변수(`--accent` 계열) · 시각 세부는 시안(아티팩트 「슬롯 도킹 시안」 v2).
- 지우는 때 = 떠남 · 놓기 · `Ended` · 영역의 `drag_id` 가 지금 끌기가 아님 · 그 창이 보이는 탭이 영역의 `view_id` 와 다름(5.1 · §1-4) · (5.7) 판정이 `Reorder`(출처 창 구역 — 바깥 띠 빗금까지 아무것도 그리지 않는다 · 끄는 탭이 활성이 아니면 `canvas` 가 참이라 지금 코드는 바깥 띠를 그린다 — 9c 가 고친다).

### 4-4. 순서 이동 모습 (5.7 · PRD R23 · Lumino `TabBar` 꼴 · `TabBar.tsx`)

입력 = 그 창 영역의 `reorder`(얼린 머리 `H` · `d` · `w`)와 지금 판정 `Reorder{t}` · 끄는 머리 왼쪽 `L`(§2-5 — `pickTarget` 과 같은 함수의 값). 셸에 보내는 것은 `t` 가 바뀔 때의 `Over` 뿐이다(`L` 은 보내지 않는다).

- **끄는 머리** — `transform: translateX(L − H[d].x0)` · 전환 없음(커서에 붙어 간다) · 이웃 위에 그린다(`position: relative` · 이웃보다 높은 `z-index`). 모양(활성 · 비활성 색)은 그대로다. DOM 순서 · React key 는 바꾸지 않는다(§1-4 「`dragend`」 — 끌린 요소는 그 자리에 둔다).
- **이웃** — `t ≤ i < d` 는 `translateX(+w)` · `d < i ≤ t` 는 `translateX(−w)` · 그 밖 0. `transition: transform REORDER_SLIDE_MS(150 ms) ease`(Lumino `left 150ms` 와 같다).
- **언제 거두나 · 전환을 끄는 렌더** — ① ★스토어의 그 창 탭 순서(`win.tabs`) ≠ 얼린 순서(`H` 의 id)면 모습을 그리지 않는다★ — 놓기가 적용되면 `window:tabs-updated` 가 새 순서를 실어 오고, 그 렌더에서 transform 을 전환 없이 거둔다(새 배치의 자리 = 비켜난 자리라 그림이 그대로다 · 전환을 켠 채 거두면 새 자리에서 옛 transform 만큼 한 번 더 튀었다 돌아온다). 다른 경로가 순서를 바꾼 경우도 같다(셸도 `reorder` 를 뺀다 — §2-4 ④). ② 모드가 바뀌는 렌더(구역 ↔ 밖)는 전환을 끈다 — 머리 접기 · 되살리기와 transform 이 같은 렌더에서 바뀐다(§1-4 「모드」). ③ 놓음(구역 안 `drop` · Esc 끝을 웹뷰가 알 때) = 끄는 머리를 자리 `t` 의 x(`t < d` = `H[t].x0` · `t > d` = `H[t].x1 − w` · 같으면 `H[d].x0`)로 `REORDER_SLIDE_MS` 동안 미끄러뜨리고 이웃은 그대로 둔다(Lumino 의 「놓은 뒤 애니메이션 · 그다음 확정」 꼴 — 우리는 확정이 셸 왕복이라 기다리지 않는다) → ① 의 순서 알림이 오면 그 렌더에서 거둔다. ④ 무변경 끝(`Ended` 인데 순서가 그대로 — 같은 자리 · 대조 실패 · 엔진 취소) = 전환을 켠 채 0 으로 돌린 뒤 `REORDER_SLIDE_MS` 뒤 전환을 끈다(제자리로 미끄러져 돌아온다).
- **`Ended` 와 순서 알림의 순서** — 셸은 적용 알림(`window:tabs-updated`) 뒤에 `Ended` 를 낸다(§1-3 「끝」 — 같은 스레드의 emit 순서 · 가능성 높음). 순서가 뒤바뀌어 `Ended` 가 먼저 닿으면 ④ 로 제자리로 미끄러졌다가 알림 렌더에서 새 순서로 바뀐다 — 한 번 출렁이고 끝은 맞다(S7 ① 이 잰다).
- **크롬 보고** — 모습이 서 있는 동안 건너뛴다(§1-4 「모드」).
- **스크롤(리뷰 1 · 사용자 지시)** — `H` · `L` · ③ 의 미끄러질 x 는 내용 좌표다(§2-5). transform 은 내용 안의 이동이라 이웃은 스크롤과 함께 움직이고, 끄는 머리의 `L − H[d].x0` 은 탭바 `scroll` 사건의 다시 판정에서 새 `s` 로 다시 잡혀 커서 밑에 머문다.
- **끌기 그림** — 그대로다(PRD Q20 ① — 구역 안에서도 Windows 기본 그림이 커서를 따른다 · W14).
 `Ended` 는 무조건 지운다 — 어떤 창이 떠남을 놓쳐도 끝 방송 한 장이 지운다(R13b · AC16).

## 5. 저장 · 원자성 · 출력 연속 (R11 · R11a)

기존 장치가 그대로 덮는다 — 새로 짓는 저장 · 라우팅 장치는 없다.

- **(5.7) 순서 이동도 같은 길이다** — 놓을 때 `ViewManager::dock(TabBar{같은 창, index})` 한 번(락 한 번 · 같은 자리면 `version` 불변)이고, 끄는 동안의 움직임은 웹뷰 그리기라 저장 · 모델에 아무것도 남지 않는다(PRD R11a · R23).
- **끄는 동안 저장되는 것 = R6b 탭 전환 하나** — 표식 · 묘비 · 장부는 메모리, 떼어진 표시는 웹뷰에만 있고, 출처 팝아웃 숨기기는 자리를 바꾸지 않는다. 그래서 끄는 동안 앱이 꺼지면 다음 부팅은 끌기 전 배치에 그 사이 R6b 로 바뀐 활성 탭만 반영된 판이다(숨겼던 팝아웃도 저장 자리로 다시 뜬다).
- **원자성(R11a 앞 절반)** — 출처 탭 빼기 · 대상에 넣기 · 빈 팝아웃 항목 지우기 · ★메인이 비면 빈 탭 채우기★ · 곁표 옮기기가 `ViewManager::dock` 한 번(= 락 한 번 · `version` 한 번)이다. 새 창은 OS 창을 먼저 열고(phase B) 모델 변경은 phase C 한 번이다. 그 락을 거치지 않고 모델을 보는 관찰자는 없다(`apply.rs` 머리 「락 규율」).
- **저장(R11a 뒤 절반)** — 기록기는 변경 번호가 바뀌면 스냅숏을 같은 락 안에서 통째로 복사해 쓴다(`state/convert.rs` `StateRevision` · `state/saver.rs`). 그래서 `state.json` 은 도킹 전이거나 뒤이고, 탭이 두 곳이거나 어디에도 없는 판은 없다. 원자 쓰기(base `file`)라 쓰는 도중 꺼져도 이전 판이 남는다(AC11). 새 창 phase B 와 C 사이에 꺼지면 OS 창만 있고 모델은 도킹 전이라 다음 부팅에 그 창은 없다.
- **OS 창 닫기는 모델 뒤 — 모든 경로가** — 빈 팝아웃은 모델에서 먼저 지우고(락 안) 락 밖에서 destroy 한다(`close_tab` 과 같다). 사용자 X 도 닫기 요청에서 모델을 먼저 지운다(M2).
- **출력 연속(R11 · AC7)** — 같은 락 안의 `subs.resync(&mgr)`(`output_router.rs` 「같은 critical section」)가 라우팅 표를 바꾼다. 에이전트가 출처 창에서 대상 창으로 옮겨도 「어느 창에든 보이는가」는 그대로라 데몬 구독은 끊기지 않고 경로만 바뀐다. 대상 웹뷰에 새로 마운트된 칸은 기존 구독 효과(`[viewId, agentId]` · replay 요청)로 지난 출력을 받는다. 출처 창이 닫히면 그 창 Channel 이 해제된다. ★닫히는 출처 웹뷰의 늦은 replay 요청을 3 단계 가드가 거르는 것은 그 에이전트를 보이는 창이 하나도 없을 때뿐이다(5.1 · N6)★ — 도킹에서는 대상 창이 그 에이전트를 보이므로 그 요청은 지나가 Subscribe 가 나가고, 그 replay 는 라우팅 표의 지금 대상(대상 창)으로 배달된다. 대상 뷰는 seq 연속 규칙(CLAUDE.md 「replay→live」 — 이미 받은 seq 이하는 버린다)으로 겹친 것을 버리므로 무해하다(가능성 높음 — replay 는 에이전트당 단일 비행이라 대상 뷰 자신의 요청과 병합될 수 있다 · AC7 화면 확인). 재사용(R22 · 탭 하나 팝아웃)은 모델이 안 바뀌어 라우팅도 안 바뀐다.
- **웹뷰 안 화면 상태(D10 비목표)** — 입력 초안 · 스크롤 · 렌더 모드 오버라이드는 출처 웹뷰에만 있어 따라가지 않는다.

## 6. 사용자 결정

### 6-1. 확정 (PRD 4.3 「(결정)」 · §5-1)

- **제목줄 = 창 이동만, 탭 = 도킹**(R1 · R2) — 제목줄은 OS 기본 이동이고 이 기능은 끼어들지 않는다.
- **탭을 끌면 곧바로 떼어지고 끌기 그림이 따라간다 · 창은 따라오지 않는다 · 「원래대로」는 없다**(R2 · Q1) — §4-1. ★(5.7 · Q20) 「곧바로」가 「탭바 구역을 벗어나는 순간」으로 바뀌었다 — 그 안에서는 순서 이동(R23) · 끌기 그림은 처음부터 따라간다★.
- **끌기 그림 = Windows 기본 모양 · 내용 = 되면 탭 축소 그림, 아니면 제목 · 딱지 창 없음**(R2 · Q5 · 2026-10-16) — §4-2.
- **끌린 탭의 배치는 놓을 때만 바뀐다**(R2 · R11a) — §1-3 · §5.
- **가이드 = 슬롯 상하좌우 띠(1/3) + 정확한 결과 사각형 + 창 바깥 띠, 가운데는 없음**(R3–R6) — §2.
- **놓으면 셋 중 하나 — 이음새 · 탭바 · 그 밖은 새 창**(R7 · R21 · R22 · Q3 · Q4) — §1-2 · §1-3.
- **크기 = 새것 1/3 · 기존 2/3 하나로 · 막는 것은 면적 0 뿐**(R7 · R8) · **붙는 단위 = 탭 하나의 배치 통째**(R9).
- **Esc = 그 자리에 놓기**(R12 · Q6) · **비정상 끝 = 마지막 커서 자리에 놓기 · 예외 = 출처 탭이 사라짐**(R13b · Q8) — §1-3 「끝 신호」.
- **끌기 수단 = 브라우저 내장 HTML5 끌어 놓기 · 감싸는 라이브러리 없음**(R18 · Q7) — §1 · ADR-C.
- **메인 마지막 탭도 끌기 · 떠나면 메인에 빈 탭**(R14 · Q9) — §1-2 4 단계 고침.
- **LLM 명령은 다음 작업**(R16 · R17 · Q10) — §3 · ADR-D.
- **직전 도킹 되돌리기 = 비목표**(Q2) · **슬롯 하나 직접 끌기 · 슬롯 가운데 칸 · Ctrl 끌기는 범위 밖**(PRD §2).
- **자기 탭바 안 = 순서 이동(사용자 결정 2026-10-17 · PRD Q20 · R23)** — 브라우저 끌기 그림은 그대로(투명 · 교체 없음) · 원래 창 탭바 안에서는 끄는 탭이 커서를 따라 가로로만(탭바 안) 움직이고 이웃이 건너면(가운데 규칙) 비켜나며 끄는 탭이 위에 그려진다 · 떼어진 모습 · 가이드 없음 · 놓으면 보인 자리로 · 모델은 놓을 때만 · 탭바를 (Chrome 처럼 위아래 여유를 두고) 벗어나면 지금 있는 것 전부 — §1-3 「순서 이동 · 도킹 모드 증거」 · §1-4 「모드」 · §2-5 · §4-4. 따라 나온 것 = 원래 창 탭바의 머묾 전환 없음 · 같은 창 합치기는 그 탭을 먼저 띄우고 다른 탭을 캔버스로(PRD R15 · AC14 · §11-1b ②) · 다른 창 탭바 머묾은 그대로.
- **보고 없는 끝 = 뗀 순간 커서 판정을 적용(사용자 결정 2026-10-17 · PRD Q21 · R13b)** — 어느 대상 창도 `Over` 를 보고하지 못한 채 끝난 끌기 · 빈 보고는 셸이 뗀 순간 커서를 W(없으면 출처)의 영역으로 판정해 적용한다 — 구역 = 순서 이동(대개 제자리) · 이음새 · 끼울 자리 = 거기 적용(이음새는 그 창에 그때 보이던 탭의 칸만 — 리뷰 1 · 캔버스 거름) · 칸 가운데 · 그 밖 = R22 새 창. 범위는 보고 없는 끝뿐이고 가이드를 보인 뒤 판정이 다르면 R13c 그대로 무변경 — §1-3 「커서 판정」 · §11-1b.

### 6-2. PRD 기본값으로 따른 것 (사용자가 바꿀 수 있다)

- **D4** 같은 창 다른 탭 합치기 = 그 탭을 먼저 띄워 둔 채 다른 탭을 끌어 탭바 밖(캔버스)으로 내려 놓기(R15 · PRD 4.6 — Q20 에서 따라 나온 것 · §11-1b ② 닫힘). 원래 창 탭바는 순서 이동 구역이라 머묾 전환이 없다(§1-3 「R6b 탭 전환」). 4.5 까지의 「머물면 탭 전환(R6b) — 대기 500ms 뒤 놓기」로 합치는 길은 원래 창에서 없어졌고, 다른 창 탭바의 머묾 전환(500ms)은 그대로다.
- **D6** 크기 기억 · 마지막 도킹 자리 = 나중 — 설계에 자리를 두지 않았다.
- **D7** 띠 겹침 = 바깥 띠 우선 · 정규화 거리(R6a) — §2-2 규칙 6 · 8.
- **D8** 마우스로 놓은 뒤 대상(새) 창 활성화 + 첫 슬롯 포커스(R11b) — `FocusPolicy::Reveal` + `set_focus`.
- **D9** 끄는 중 같은 탭을 옮기는 명령은 실패 — `DockError::Dragging`(표식 · 같은 락 판정 — 다음 작업의 명령이 쓴다).
- **D10** 웹뷰 안 화면 상태 이전 = 비목표 — §5 끝 줄.
- **D12** 새 창 크기 = 출처 창 크기 · 메인에서 떼었으면 팝아웃 기본 크기 — `plan_new_window`.
- **D13** 탭 하나뿐인 팝아웃은 끄는 동안 숨김 — Tauri `hide()`(§4-1 · S1 ⑤) · (5.7) 구역을 벗어날 때(도킹 모드 증거 — §1-3).
- **D14** 메인 마지막 탭을 끄는 동안 메인 = 빈 화면 · 빈 탭은 놓을 때(§4-1) · (5.7) 도킹 모드에서만(구역 안 = 그 탭이 탭바에 그대로).
- **R5 바깥 띠 두께** — 32 CSS px(§2-3) · **R13c** 출처 바뀜 = 받지 않음(「안 됨」) · 대상 바뀜 · 보인 것과 다름 = 무변경(§1-3).
- **(5.7 · 사용자가 세부를 메인에 맡긴 것 — 기본값 · 메인 판단)** **D15** 같은 끌기에서 구역으로 돌아오면 순서 이동(§1-4 「모드」) · **D16** 구역 안 Esc = 보이던 자리에 놓기(§1-3 「도킹 모드 증거」 끝 줄 — 「마지막 점에 놓기」가 4c 로 간다) · **D17** 다른 창 탭바 = 끼울 자리 막대 그대로 · **D18** 탭 하나 = 무동작 · 벗어나면 도킹(§2-5 ④) · **D19** 여유 15 CSS px · 이력 없음(§2-5) · **D21** 보고 없는 끝의 커서 판정 = 원래 창 · W 만(§1-3 「커서 판정」) · **D22** 순서 이동도 이번 LLM 명령 예외(Q10)에 든다(§3 — 리뷰 1 에서 PRD 에 올림).
- **D20(사용자 지시 2026-10-17 — 메인 판단이 아니다 · 리뷰 1 라운드)** 넘친 탭바 = 끄는 탭을 끝에 대고 있으면 그쪽으로 계속 스크롤(관례) · 끄는 탭은 커서 밑에 머물고 놓으면 그려진 자리 — 엔진 자동 스크롤 · 내용 좌표 · 보고의 `scroll`(§1-4 「모드」 · §2-5). 4.6 첫 판의 「자동 스크롤 없음(비목표)」을 바꿨다 — 리뷰가 낸 「끄는 동안 스크롤 막기」 안도 사용자가 관례로 바꿨다.

## 7. 내가 정한 내부 선택 (보고)

**구조**
1. **끌기 장부를 `layout` 밖 `dock` 모듈에 둔다** — 장부는 레이아웃 권위를 부르는 손님이다.
2. **장부 = 순수 `step(사건, 스냅숏) → 할 일` + 잎 락 하나 · 스레드 · 틱 없음** — 가짜 시각 · 커서 · 창 사각형 · 스냅숏으로 단독 시험이 선다(ADR-0012). 커서 · 사각형은 실행기가 읽어 입력에 싣는다.
3. **영역을 셸이 미리 계산해 창마다 보내고 웹뷰는 점-사각형만 본다 · 보인 것은 바뀔 때만 보고** — 움직일 때마다 셸에 묻는 길(invoke p99 ~16ms · 최대 ~50ms — 실측)은 60Hz 가이드를 눈에 띄게 늦춘다(ADR-C 거부한 대안 ⑥). 셸은 영역을 만들고 놓기를 다시 판정하므로 권위는 그대로다.
4. **판정 규칙은 Rust 하나(`hit`) · `pick` 은 그 위의 어댑터 · 웹뷰 판은 골든으로 묶은 거울** — 둘이 갈라져도 결과는 「무변경」이지 엉뚱한 도킹이 아니다.
5. **놓기 대조 = 보인 것과 지금 판정이 같을 때만 적용(영역 `rev` 최신 · 보이던 탭 같음 포함) · 둘 다 「가이드 없음」일 때만 새 창 · 그 밖은 무변경** — 4판의 「마지막으로 보낸 오버레이」를 「마지막으로 보낸 영역 + 웹뷰가 그린 것」으로 옮겼다. M1(못 본 가이드에 놓이지 않음)은 그대로이고, 「지금 판정이 가이드 없음이면 보고가 무엇이든 새 창」은 5.1 에서 버렸다(본 가이드가 사라진 자리의 새 창은 못 본 결과다 · Q18).
6. **영역 다시 계산 계기 = 쓰기 하나의 알림 묶음 끝(`LayoutEvents::settled`) 한 자리 + 크롬 · 캔버스 보고 · 스냅숏 번호로 순서 보장** — 레이아웃을 바꾸는 쓰기는 다 그 어댑터로 알리므로 한 자리에 건다. 알림 하나마다가 아니라 묶음 끝인 것은 5.1 — 영역 · 끝이 그 쓰기의 나머지 알림을 앞지르지 않게.

**끝 신호**
7. **Esc · 창 밖 떼기를 키 상태로 가르지 않는다** — `none` 끝이 우리 창 위를 가리키면 그것은 Esc 이고, Esc · 떼기는 같은 「그 자리에 놓기」다. 판정 = 마지막 Over 창(창별로 마지막 Over 와 떠남을 따로 적는다 — 5.1) + 떠남 유예 100ms + 셸 커서가 그 창 사각형 안. OS 함수 0 · 같은 규칙이 시스템 취소에도 선다.
8. **`dragend`(move) 뒤 `drop` 을 500ms 기다린다** — 두 웹뷰 invoke 의 도착 순서는 보장되지 않는다. 시한이 지나면 「마지막 점에 놓기」(그 끝 신호 때의 커서).
9. **끝 신호 유실 = 세션 중 본 버튼 0 의 마우스 `pointermove` · `pointerdown`** — 끄는 동안 포인터 사건이 안 오는 엔진 규칙을 끝의 증거로 쓴다(웹 표준 · OS 0). 300ms 기다린 뒤 W 가 아직 위면 그 마지막 Over 보고로 놓고, 아니면 지금 커서에 새 창.
10. **세션 id = 출처 웹뷰가 `dragstart` 에서 만든 UUID** — `setData` 를 셸 왕복 전에 끝내야 하므로. 셸은 그 id 를 세션 키로 쓰고, 최근 끝난 id 의 신호는 버리며 처음 보는 id 의 끝 신호는 잠깐 붙든다(5.1).
11. **끌기 데이터에 우리 MIME 하나만** — 다른 앱(탐색기 · 편집기 · 브라우저)이 받지 않아 `none` 으로 끝나고 그 앱에 아무것도 안 들어간다(AC8).
12. **새 시작이 살아 있는 옛 세션을 만나면 새 끌기를 거절하고 옛 세션을 「마지막 점에 놓기」로 끝낸다 · 단 옛 세션이 적용 중이면 끝내지 않고 새 시작만 거절한다(5.1)** — 표식은 하나뿐이고 옛 세션은 신호를 잃은 것이다(OS 끌기는 한 번에 하나). 적용 중인 세션은 신호를 잃은 것이 아니라 적용 서비스를 기다리는 중이라 `Applied` 가 끝낸다. 사용자는 다시 끈다(§11-2 #3).
13. **시작 · 보고는 동기 명령(메인 스레드 · 도착 순서), 창을 만들 수 있는 놓기 · 끝은 async** — 같은 웹뷰의 시작 → 끝 순서가 서고, 창 빌드 교착을 피한다.
14. **시작 되감기 가드를 두지 않는다** — OS 끌기는 이미 돌고 있어 되감을 것이 표식 · 숨김뿐이고, 그 둘은 모든 끝 경로가 거둔다. 숨기기 실패는 보인 채 이어 간다.

**웹뷰**
15. **떼어진 표시는 `Started` 뒤 · `dragstart` 안에서는 DOM 을 안 바꾼다 · 끌린 머리는 DOM 에 남긴다(CSS 접기)** — 끌기가 곧바로 끝나거나 `dragend` 가 올라오지 않는 엔진 동작을 피한다.
16. **R6b 머묾은 대상 웹뷰가 재고 기존 탭 전환을 부른다** — 장부에 UI 시간을 두지 않고, 쓰기는 사람 클릭과 같은 표면이다.
17. **Esc 삼키기(세션 + 200ms · 그 창이 끝 증거를 보면 200ms 뒤 그침)와 파일 드롭 막기(늘)를 웹뷰에 둔다** — 둘 다 보험 · 회귀 막이이고 셸이 할 일이 아니다.
18. **끌기 그림 크기 · 내용은 웹뷰가 정한다(4.2 딱지의 크기 규칙 그대로)** — `setDragImage` 가 `dragstart` 안 동기만 받아 셸에 물을 틈이 없고, 놓기 판정과 무관한 모양 값이다. 픽셀은 터미널 WebGL 캔버스만 옮기고 DOM 칸은 자리채움이다.
19. **모든 런타임 창의 드롭 처리기 끄기 = `WebviewEnv::finish` 한 곳** — 창 만드는 코드가 다 지나는 곳(ADR-0283)이라 앞으로의 창도 빠지지 않는다.

**레이아웃 연산 (2 · 4 단계 — 4판에서 이어짐)**
20. **`close_tab` 의 빼기 · 승계 · 빈 창 판정을 도우미로 나눠 쓰고, 승계는 순수 함수로도 내보낸다 · 4 단계에서 메인 빈 탭 채우기(`refill_main`)도 같은 도우미로** — 규칙이 모델 · 떼어진 표시 · 판정에서 한 벌이다.
21. **끼우는 서브트리의 슬롯 id · 분할 id 를 그대로 · 탭바 · 새 창은 View 째(탭 id 그대로)**.
22. **결과 트리 함수 하나(`docked_tree`)를 영역 · 판정 · 적용이 같이 쓴다 · 새 면적 0 = 원래 면적이 있던 칸이 0** — 「가이드 = 결과」(R4)가 식 하나로 선다.
23. **사전조건 = 호출자 중립 값(`DockGuard`) · 포커스 정책(`FocusPolicy`) · 출처 대조 = 배치 값 비교** — 모델이 「마우스」를 모르고, 변경 번호를 올리는 자리를 빠뜨려도 틀리지 않는다.
24. **R22 에서 출처가 탭 하나뿐인 팝아웃이면 새 창 대신 그 창을 옮긴다** — 결과가 같고 웹뷰를 새로 띄우지 않으며 출력 경로가 안 바뀐다. 옮긴 뒤 다시 확인하지도 되돌리지도 않는다(사용자 결정 Q19 · 5.6 · #39). 보통 자리가 모델에 없어도 그 창을 옮긴다(5.4 의 조건은 근거였던 되돌리기와 함께 5.6 에서 거뒀다 · §11-2 #26).
25. **탭바 `index` 넘침 = 맨 뒤 · 같은 창 같은 자리 = 무변경 · 단 `Reveal` 이고 안 보이던 탭이면 보인다** — 사용자 결정(§11-1 #6 · 2026-10-16).
26. **창 포트의 새 메서드는 기본 본문 없이 · 마우스 자리도 `placement::land` 로 검증**.
27. **(M2) 팝아웃 닫기 요청에서 모델을 먼저 지우고 묘비** — §1-2 · ADR-E.

**판정 (5 단계 — 그대로)**
28. **칠하지 않은 바깥 띠가 전부면 밑 칸 띠로 넘어간다** — §2-2 규칙 8 · §11-1 #5.
29. **그린 사각형 = 판정 사각형(네 변 반열림 · 정수 바깥 띠)** · **크롬 측정 타입은 `layout::chrome`**(창 항목이 들고 있어 `layout` → `dock` 의존을 만들지 않는다).

**5.1판 (리뷰 1라운드)**
30. **`TargetExpect::Seam` 에 대상 창 모델 활성을 더하고 웹뷰는 보이던 탭(`displayed`)을 싣는다 · 영역 `view_id` 와 다르면 그리지 않는다** — 활성 전환의 「모델 먼저 · 알림 나중」 틈에 안 보이는 탭으로 붙지 않게(§1-3 「보이는 탭 경쟁」). 새 오류 종류는 두지 않는다 — 사용자에겐 「대상이 바뀜」과 같은 무변경이다.
31. **우리 MIME 은 세션을 몰라도 받는다 · 영역 · 세션은 `drag_id` 별 최대 `rev` · `seq` · 옛 `rev` 보고에는 영역을 다시 보낸다 · 시작보다 먼저 온 끝 신호는 500ms 붙든다** — emit · invoke 가 여러 스레드에서 나가 순서가 보장되지 않는다. 붙드는 것은 끝 신호(`drop` · 출처 끝)뿐이고 한 번에 한 id 다 — 시험이 시한 하나 · 칸 하나로 선다.
32. **적용 중에는 스냅숏 · 소멸을 적기만 하고 끝은 `Applied` 가 정한다 · 스냅숏 계기는 알림 묶음 끝 한 번(`LayoutEvents::settled` — 기본 본문 없음)** — 성공한 도킹이 자기 알림 · 자기 창 닫기로 끝나지 않게.
33. **react-arborist 의 window 끌기 리스너는 우리 MIME 사건을 `document` · 탭 머리에서 끊어 피한다 · 트리 뿌리 좁히기(`dndRootElement`)는 택하지 않는다 · 셸 세션은 취소되지 않은 `dragstart` 뒤에만** — 뿌리 좁히기는 react-dnd 의 창당 관리자 하나 때문에 트리 둘에서 깨지고, 고치려면 `dnd-core` 직접 의존이 든다(§1-4).
34. **기하가 같아도 기대값이 바뀌면 `rev` 를 올린다** — 내용만 바뀐 대상 칸에 본 가이드대로 붙게.
35. **창 게터는 레이아웃 · 장부 락 밖에서 · `WindowSnap.hidden` 은 장부 값** — 메인 스레드로 넘어가 기다리는 게터와 메인 스레드 처리기의 레이아웃 락이 교착하지 않게.
36. **세션 깃발은 시작의 레이아웃 락 안에서 세우고, 세션 전에 온 스냅숏은 들고 있다가 `Start` 뒤에 먹는다** — 시작과 장부 사이에 끼어든 쓰기를 놓치지 않게.
37. **끝 증거는 버튼 0 의 마우스 이동 · 누름 · Esc 삼키기는 그 창의 끝 증거 뒤 200ms 에서 그친다 · 커서를 못 읽으면 W 의 마지막 점** — 합성 이동을 끝으로 오인하지 않고, 남은 세션이 모든 창의 Esc 를 먹지 않고, 「마지막 점」 원칙을 커서 없는 끝에도 지킨다.

**5.2판 (리뷰 2라운드)**
38. **탭바 놓기는 이음새와 따로 대조한다 — 탭바 몫 `bar_rev` · 탭 순서만 · 보이던 탭은 이음새만** — 활성 전환은 탭바를 바꾸지 않으므로 전환 알림 틈에도 탭바 놓기가 선다(§1-3 「놓기 판정」 4b). 영역을 탭바 · 칸 둘로 쪼개 보내는 안은 택하지 않았다 — 메시지 · 최대 `rev` 규칙이 둘이 된다. 번호 하나를 더 싣는 것으로 같은 효과다.
39. **R22 재사용은 다시 확인하지도 되돌리지도 않는다 — A 판정 → 창 옮기기 → 표식 내리기(사용자 결정 Q19 · 5.6)** — 끄는 동안 그 팝아웃은 숨어 있어 그 틈(ms)에 출처를 바꿀 수 있는 것은 다른 경로(LLM)뿐이고, 재사용은 모델을 바꾸지 않아(창 자리만) 옮긴 채 두어도 모델과 화면이 어긋나지 않는다 — 그 사이 변화는 자연스러운 결과로 둔다(배치 바뀜 = 바뀐 채 · 탭 닫힘 = 기존 규칙대로 닫힘 · 탭 하나 더 = 함께 옮겨짐). **거부한 대안 = 5.2–5.5 의 「옮기고 → 같은 락 안에서 `plan_new_window` 재확인 → 어긋나면 모델 자리로 되돌리기」**(되돌릴 자리를 A 에서 모델로 복사 · 검사 세터 · 되돌리기 포트 `restore_window` · 성공 모양 `Moved{Applied, StayedPut}` · 모델 자리 다시 적기 `reset_window_place` · 되돌리기 기록 · 시험) — 숨은 창을 ms 사이에 다른 경로가 바꾸는 드문 경우를 위해 포트 · 모델 함수 · 기록 · 시험이 늘었고, 되돌리기 자체가 최대화 깜빡임 · 모델 자리 어긋남 · 미룬 최대화 같은 새 틈을 낳아 라운드마다 고침이 따라왔다. 사용자가 「끌기 시작했으면 뗀 곳에 놓는다 · 예외 로직을 더하지 않는다」로 정했다. Create 길의 「열고 → phase C 재판정 → 어긋나면 연 창 닫기」는 남긴다 — 그쪽은 모델을 바꾸므로 모델 정합이 걸린다. 되돌릴 자리 때문에 들였던 「재사용은 보통 자리가 있을 때만」(5.4)도 함께 거뒀다(메인 결정 2026-10-16).
40. **`Apply` 는 늘 async 런타임에서 · 동기 명령 · 메인 스레드에서는 창을 만들지 않는다** — 붙든 이른 놓기를 시작(동기)에서 이어 먹이는 길이 생겼기 때문이다(§1-3 「명령의 실행 스레드」).
41. **붙들다 버린 id 는 최근 끝난 id 로 적어 늦은 시작을 `LateStart` 로 거절 · 세션 깃발은 세대 비교-교체로 내림** — 끝 신호 없는 세션과 다음 세션의 계기 끊김을 막는다.

**5.7판 (사용자 결정 Q20 · 실 마우스 실측 결함)**
42. **순서 이동의 자리 규칙은 셸 `hit` 에 두고 웹뷰는 거울(골든)로 그린다 — 웹뷰가 자리를 정해 보고하고 셸이 그것을 믿는 안은 택하지 않았다** — 보고 없는 끝(멈춘 채 떼기)에서 셸이 스스로 자리를 낼 수 있어야 하고(결함 ①), 놓기를 같은 함수로 다시 판정하는 5판의 원칙(M1 · 4c)이 그대로 선다. 기존 `Picked::TabBar` 의 「커서 x ≤ 머리 가운데」 규칙을 그대로 쓰는 안도 택하지 않았다 — 끄는 탭이 이웃을 다 덮은 뒤에야 비켜나 Lumino · Chrome 의 「가장자리가 가운데를 넘으면」과 다르다(사용자 결정 「건너면 비켜남(가운데 규칙)」).
43. **기하 = `dragstart` 안 측정을 세션 내내 얼린다(`strip`) · 셸 크롬 보고를 쓰지 않는다** — 도킹 모드의 접힌 머리 · 순서 이동의 transform · 크롬 디바운스가 판정을 흔들지 않게(§2-4). 대가 = 끄는 동안 다른 경로가 그 창 탭을 바꾸면 순서 이동이 빠진다(그 끌기는 5.6 으로 물러난다 · §11-2 #32).
44. **모드는 출처 웹뷰가 자기 `pick` 으로 정하고 장부는 「도킹 모드 증거」(미룬 숨기기)만 안다** — 모드를 장부가 정해 방송하면 왕복이 모드 전환을 늦추고(60Hz 가 아니다) 그 사이 웹뷰 그림과 장부가 갈라진다. 판정은 모드와 무관하게 보고 · 지금 영역으로 한다.
45. **순서 이동 놓기(4c)는 `rev` · `bar_rev` · `displayed` 를 보지 않는다** — 영역이 얼어 있어 같은 점의 판정이 늘 같다 · 번호가 오르는 까닭이 그 판정과 무관하다. 모델 쪽은 `Tabs` 기대값이 같은 락 안에서 본다.
46. **구역 여유 15 CSS px · 이력 없음 · 구역이 탭바 · 바깥 위 띠 · 칸 위 띠보다 먼저** — Chrome 과 같은 넓힌 사각형 하나 · 상태 없는 `pick`(보고 없는 끝에서도 같은 답) · 대가 = 출처 창의 위쪽 띠가 15 px 덜 닿는다(§2-5).
47. **탭 하나 팝아웃의 숨기기를 도킹 모드 증거까지 미룬다 · 한 번 숨기면 그 끌기 동안 되보이지 않는다** — 구역 안 순서 이동이 보이려면 창이 보여야 한다 · 숨긴 창으로는 돌아올 수 없다(PRD D13 · D15).
48. **빈 보고(`shown` 없음 + `rev` 0 · 지금 영역 없음 — 리뷰 1 로 좁힘)를 보고 없음과 같이 보고 셸이 커서로 판정한다 · 판정할 창 = 그 끌기를 본 W · 원래 창(누름으로 맨 앞)** — 결함 ① ③ 이 같은 뿌리(웹뷰가 지금 영역으로 그리지 못함)이고, 아무 우리 창으로나 넓히면 다른 앱 창이 덮은 자리를 우리 창으로 오판한다(AC8 · §1-3 「커서 판정」). 메인 제안의 「겹친 창 둘이면 무변경」은 이 좁힘으로 필요 없어졌다. 이음새 · 끼울 자리 결과 = 그 자리에 적용(사용자 결정 Q21 · §11-1b ①) — 사용자 결정이 이 좁힘을 그대로 받았다.
49. **엔진 취소 = `dragend.timeStamp − dragstart.timeStamp < 15 ms` 의 `none` · 이미 연 세션은 `canceled` 로 무변경** — 0 ms 시한과 끌기 사건의 task 순서는 보장되지 않는다(실측 4/19 샘) · 사건 시각은 처리기 지연과 무관하다. 연 세션을 「마지막 점에 놓기」에 맡기는 안(커서 판정이 제자리 순서 이동을 낼 것)도 택하지 않았다 — 판정에 기대지 않고 무변경이 확실하다.
50. **wire 는 더하기만 · 셸 쪽 새 칸은 선택 칸** — 9a · 9b 를 먼저 넣고 멈춰도 7 단계 웹뷰가 5.6 그대로 돈다(§10 「어디서 멈춰도 선다」).

**5.7판 리뷰 1 라운드**
51. **미룬 숨기기의 증거 = 그린 보고 · 시한 뒤의 떠남 · 다른 창 보고** — 영역 전 · 막힌 영역의 「없음」과 Esc 가 낸 떠남은 커서가 구역 밖이라는 증거가 아니다. 떠남 시한을 「아직 위」와 같은 `LEAVE_GRACE_MS` 로 둔 것은 두 판단이 같은 경계에서 갈리게 하려는 것이다(§1-3 「도킹 모드 증거」 ②). 택하지 않은 안 = 끝 신호 뒤에만 숨기기를 정하기(창 밖에서 끝까지 보인다 — D13) · 떠남을 키 상태(Esc)로 가르기(OS 함수 — §1-6).
52. **커서 판정도 캔버스를 거른다 — 보고의 `displayed` · 출처 보고가 없으면 모델 활성** — 셸 영역의 보이는 탭은 도킹 모드(승계 탭) 기준이고 보고 없는 끌기의 출처 화면은 순서 이동 모습(모델 활성)이다. 택하지 않은 안 = 보고 없는 끝의 출처 캔버스를 승계 탭으로 치기(그 탭은 화면에 없었다 — PRD R15).
53. **웹뷰 판정 순서 = `pick` → 모드 → `displayed` → 캔버스 거름** — `displayed` 를 앞 모드로 정하면 구역 밖 첫 프레임이 지금 `rev` 의 「없음」 보고를 내 그 직후 놓기가 무변경이 된다(§1-4 「모드」).
54. **넘친 탭바 스크롤 = 엔진 자동 스크롤(안 되면 우리 끝 띠) + 내용 좌표 · 셸엔 보고의 `scroll`(사용자 지시 2026-10-17)** — 얼린 머리를 내용 좌표로 두면 스크롤이 기하를 흔들지 않고 바뀌는 수가 하나뿐이라 보고에 칸 하나로 싣는다. 택하지 않은 안 = 끄는 동안 스크롤 막기(리뷰가 낸 첫 안 — 사용자가 관례로 바꿨다) · 스크롤마다 머리를 다시 재 보내기(transform 이 걸린 머리를 잰다 — §2-4 ②).

**5.7판 리뷰 2 라운드**(모두 기본값 · 메인 판단)
55. **끝 신호가 그린 판정을 싣는다** — 출처 끝의 `last` · 놓기의 칸 전부가 화면을 그린 판정 하나에서 오고, 셸은 도착 순번으로 고른 W 가 출처일 때 `last` 를 받은 마지막 `Over` 보다 앞세운다(§1-3 「끝 신호」 · 리뷰 3 고침 — 리뷰 2 판은 W 까지 `last` 로 정했다 · #59). 보고 줄(`overChain`)과 끝 invoke 는 셸 도착 순서가 보장되지 않는다 — 가장자리 스크롤 중 Esc 가 옛 자리를 적용하던 틈이다.
56. **모든 창 탭바를 내용 좌표로** — 크롬 머리 · 끼울 x = 스크롤 0 자리 · 보고 · 놓기의 `scroll` 로 R21 도 판정 · 막대 = x − 지금 스크롤 · 탭바 `scroll` 은 크롬 계기가 아니라 다시 판정 계기(§1-4 「모드」 넘친 탭바 ④). 택하지 않은 안 = 그 창 스크롤 동안 크롬 디바운스를 줄이기(5.7 첫 판 S7 ⑬ ⓒ 의 물러날 자리 — 연속 스크롤 동안의 놓기는 여전히 옛 기하를 탄다).
57. **숨기기 · 보이기 = 메인 스레드의 맞추기 · 세션 번호** — 할 일이 낸 순서가 아니라 실행 때의 장부 값을 입히고, 지난 세션의 시한은 버리며 지난 세션의 할 일은 제 뜻 대신 지금 값으로 맞춘다(§1-3 「명령의 실행 스레드」).
58. **끝의 `now` 는 세계 읽기 전 · 시한이 앞지른 끝의 출처는 아직 위 · 숨긴 출처의 `sent` 는 끝까지** — 구역 안 Esc 가 숨기기 시한과 겹쳐 팝아웃이 뛰던 틈(§1-3 「도킹 모드 증거」 「시한이 앞지른 끝」). (리뷰 3) 그 출처도 사각형 검사는 한다 — 시작에 쥔 출처 사각형으로(#60).

**5.7판 리뷰 3 라운드**(모두 기본값 · 메인 판단)
59. **`last` 는 W 를 고르지 않는다 · 떠남 간격을 싣는다** — W 는 5.6 그대로 도착 순번으로 고르고(끝 순간 커서 · 사각형 검사 그대로), 고른 W 가 출처일 때만 `last` 가 그 마지막 `Over` 를 갈아 끼운다((리뷰 4) 예외 하나 = 도착 순번 W 가 없을 때 — #62 · 다른 창이 고른 W 를 갈아 치우지는 않는다). `last.left_before_end_ms` 가 없거나 `CANCEL_LEAVE_MS` 안이면 엔진 접기의 떠남이라 사각형 검사 없이 `last` 자리(§1-3 「끝 스냅숏」). 택하지 않은 안 = `last` 가 있으면 W = 출처(리뷰 2 판 — 출처를 떠나 다른 창 가이드를 본 뒤 출처 시계로 100 ms 안 Esc 가 새 창이 됐다 · PRD R12 · AC19 어김) · 간격이 `CANCEL_LEAVE_MS` 안인 `last` 로 W 를 출처로 올리기(돌아온 뒤 `Over` 가 오는 중인 끝을 닫지만 그 경계가 미검이다 — S1 ① 뒤 다시 본다 · §11-2 #36).
60. **시한이 앞지른 끝의 출처도 사각형 검사 — 시작에 쥔 출처 사각형으로** — 숨기기는 창을 옮기지 않아 시작(과 그 뒤 출처가 보이던 `Rects`)에서 쥔 사각형이 지금 자리다(§1-3 「시작」 2 · 「도킹 모드 증거」). 택하지 않은 안 = 사각형 검사 없이(리뷰 2 판 — 바탕화면에 빠르게 던진 끌기가 시한 흔들림에 따라 4c 무변경 · R22 로 갈렸다) · 창 이동 사건으로 사각형 갈기(끄는 동안 사용자는 창을 못 옮긴다 — 경로를 늘릴 값이 없다).

**5.7판 리뷰 4 라운드**(모두 기본값 · 메인 판단)
61. **출처가 떠났다고 말하는데 다른 창 `Over` 가 없으면 끝을 `OVER_WAIT_MS`(50) 붙든다** — 조건 = `none` 끝 · `last` 간격 > `CANCEL_LEAVE_MS` · 도착 순번 W 가 출처이거나 없음. 다른 창 `Over` 가 닿으면 그 창의 `Left`(같은 줄 — 그 창 앞 `Over` 가 다 닿았다는 표시) 또는 시한에서 그 창을 W 로, 아무것도 안 닿으면 붙들기 없는 판정 그대로 · 판정은 끝 때 읽은 커서 · 사각형으로(§1-3 「오는 중 `Over` 붙들기」). 값 = invoke 왕복 실측 최대 ~50 ms(§8-1). 택하지 않은 안 = 붙들지 않음(리뷰 3 판 — 다른 창 가이드 위 Esc 가 출처 끝이 먼저 닿으면 새 창 · PRD R12 · AC19 어김) · `last` 의 간격만으로 W 를 다른 창으로 옮기기(출처는 자기를 떠난 뒤 어느 창에 들어갔는지 모른다 — 바탕화면 던지기와 가를 수 없다) · 다른 창 `Over` 가 닿는 즉시 풀기(같은 창 줄의 뒤 `Over` — 바뀐 가이드 — 를 놓쳐 그린 것과 다른 자리를 적용한다) · 모든 `none` 끝을 붙들기(엔진 접기 · 떠남 없는 끝까지 늦어진다 — 기다릴 다른 창이 없다) · 더 긴 시한(`LEAVE_GRACE_MS` 100 — 실측 최대를 넘는 값을 모든 해당 끝에 물린다).
62. **W 없음 + 출처 `last` 가 출처 위 끝이면 W = 출처** — 떠남 없음 · 간격 ≤ `CANCEL_LEAVE_MS` 일 때만(§1-3 「마지막 점에 놓기」). 출처 첫 그린 `Over` 가 오는 중인 끝에서 `last` 를 버리고 시작 스크롤로 커서 판정해 가장자리 스크롤 뒤 다른 순서 이동 자리를 적용하던 리뷰 3 판 회귀를 닫는다. 다른 창 `Over` 가 없으니 #59 의 「`last` 는 다른 창이 고른 W 를 갈아 치우지 않는다」와 어긋나지 않는다. 택하지 않은 안 = 리뷰 3 판 그대로(W 없음 = `last` 버림) · W 없음이면 간격과 무관하게 `last` 로 W = 출처(실제 떠난 끝 — 바탕화면 던지기 — 이 출처 자리로 간다 · 그 끝은 #61 붙들기 뒤 W 없음 갈래로).

## 8. 실측

### 8-1. 끝난 실측 (2026-10-16 · 상세 = 조사 §4 · 스파이크 기록 · 2026-10-17 실 입력 — 아래 끝 묶음)

환경 = tauri 2.11.3 · wry 0.55.1 · WebView2 Edg/155 · Windows 11 · ★RDP 세션 · DPR 1.25 · 22 끌기★ · config 창(`dragDropEnabled: false`)과 런타임 창(`disable_drag_drop_handler()`) · 같은 데이터 폴더.

- **셸 메인 스레드는 막히지 않는다** — OS 끌기 루프는 WebView2 브라우저 프로세스 UI 스레드에서 돈다. 끄는 중 `run_on_main_thread` 지연 최대 4.1ms(평소 1.4) · emit → JS p99 11 · 최대 33ms · invoke 왕복 p50 2.2 · p99 ~16 · 최대 ~50ms.
- 대상 창 `dragover` 60/s(움직일 때) · ~15/s(멈춤) · `clientX/Y` 정확 · 커스텀 MIME 12/12 · `dragend` 는 모든 경우(놓음 move · 창 밖 none · Esc none) · 원본 창을 끄는 중 숨겨도 끌기 · `drop` · `dragend` 가 이어짐(2회).
- 끌기 그림 = Windows 셸 모양(반투명 · 흐려짐) · 다른 앱 위에선 사라짐. 어느 웹뷰도 아닌 자리(틈 · 바탕화면)엔 JS 사건 없음.
- 4판의 스파이크 S1–S6(포인터 직접 읽기 · 맨 위 창 · 딱지 창 · 화면 잡기)은 수단이 바뀌어 뜻을 잃었다 — §11-3.
- ★이 스파이크는 앱 의존이 없는 맨 페이지(따로 지은 Tauri 앱의 `a.html` · `b.html` + 공용 스크립트 하나)에서 돌았다(5.1)★ — React · react-arborist 처럼 window 에 끌기 리스너를 다는 앱 의존이 끌기를 막는지는 재지 않았고, 실제로 막는다(소스 — §1-4 「다른 끌기 리스너」 · S1 ⑦). 표본도 작다(숨김 2 · 창 밖 Esc 3).

**2026-10-17 — 실 입력(Win32 `SendInput`) · S1 ⑧ · S3 · 트리 재확인 · S6**(상세 = `.claude/handoff/attachments/slot-docking-S1-8-S3-S6-real-2026-10-17.md` · 빌드 = HEAD `d582331a` + 7 단계 미커밋 · RDP · DPR 1.25)

- **S3 ① 합격 40/40 · S3 ② 합격(탐색 0/20) · 트리 재확인 합격(재부모화 10/10 · 탐색 0/10) · S6 합격 10/10.** S3 ② 의 커서 「안 됨」은 화면 잡기에 끌기 그림 · 커서가 안 잡혀 판정 못 함.
- **S1 ⑧ 조건부 불합격 → 5.7 의 결함 셋**
  - ① **멈춘 채 떼면 새 창(결정적)** — 문턱만 넘고 300 · 1000 ms 멈춘 뒤 뗌: 같은 창 12/12 · 이웃 창 4/4 새 창(이웃 창 4/4 는 하네스가 커서를 `SendInput` 이동 한 번으로 이웃 창 탭바까지 옮긴 탓으로 의심 — S7 ⑪ 에서 끊김 없는 이동으로 다시). 짧게 톡(10 · 25 · 50 ms): 새 창 9/10 · 6/10 · 4/10. 새 창이 된 끌기 49 건 전부 셸에 닿은 `Over` 0 — 대상 창의 첫 `dragenter` 가 뗌 −3 ms 에야 오고 곧바로 `dragleave(null)`. 해석(추정) = OLE `DoDragDrop` 은 다음 마우스 메시지 전에 `DragEnter` 를 부르지 않는다 → §1-3 「커서 판정」.
  - ② **엔진 취소 거르기가 0 ms 시한 경주** — 같은 창 2 ms 뗌: `dragstart` 뒤 2–4 ms 의 `dragend(none)` 중 15/19 는 시한 전이라 걸렀고 4/19 는 시한 뒤에 와 세션이 열려 새 창. 「같은 task」 전제가 틀렸다 → §1-4 「엔진 취소」.
  - ③ **영역 전 보고로 판정돼 무변경** — 출처 `none` 끝이 `rev` 0 · `shown` 없음의 `Over` 로 판정: 이웃 창 6/20 · 같은 창 25 ms 3/10(결과는 무변경 — 안전 쪽) → 「빈 보고」.
  - ① · ③ 의 고침 = 사용자 결정 Q21(2026-10-17) — 셸이 뗀 순간 커서를 판정해 그 결과를 적용한다(이음새 · 끼울 자리 포함 · §1-3 「커서 판정」 · §11-1b ①).
  - 붙든 이른 신호 0(셸 세션 139 · 시작이 늘 먼저) — 「붙든 신호 20/20」 기준은 표본 0 으로 판정 불가 · `rev` 역전 0.
- **메모(결함 아님)** — R22 새 창이 화면 아래 끝에서 거의 화면 밖에 선다(놓은 자리 기준 · 작업 영역으로 당기지 않음) · 다른 Chromium 페이지가 우리 MIME 이름을 본다 · 우리 창 제목줄이 OS 파일 끌기를 「복사」로 받는다(범위 밖).

### 8-2. 남은 실측

- **어디서** — 버리는 스파이크 브랜치(예: `v0.4.1/spike/dock-*`)나 7 단계 작업 트리에서 런처로 띄워(CLAUDE.md 「GUI 실측」) 잰다. 결과는 이 문서 개정으로 남긴다. S5 를 뺀 다섯은 7 단계 앞(S2 는 7 단계의 그림 만들기와 함께 해도 된다). 단 S1 ⑦ · ⑧ 은 앱 코드(끊기 · 붙들기)가 있어야 해서 7 단계 작업 트리에서 그 단계 커밋 전에 잰다.
- **★불합격 = 멈춤★** — 「불합격이면」에는 요구를 바꾸지 않는 조정만 적었다. 그것으로도 못 넘으면 구현을 멈추고 PRD 개정(사용자 결정)으로 올린다. 수치는 첫 값이다 — 비현실적이면 개정하고 보고한다.

| # | 위험 | 무엇을 어떻게 | 합격 기준 | 불합격이면 |
|---|---|---|---|---|
| **S1** | W10 · W1 — 끝 신호 | ① Esc 를 우리 창 위(띠 · 칸 가운데 · 탭바) · 틈 · 바탕화면 · 우리 창을 덮은 다른 앱 창 위에서 각 20회 — 대상 `dragleave` 와 출처 `dragend` 가 셸에 닿는 순서 · 간격 · (리뷰 3) 출처 위 Esc 의 끝 줄 `last_gap_ms`(출처 사건 시각 떠남 → `dragend`) · 출처를 떠나 다른 창 가이드 위에서 곧바로 Esc 20회(출처 떠남 100 ms 안 포함 · (리뷰 4) 끝 줄 `over_wait` · `over_wait_ms` · `over_from` — 그 창 가이드에 붙나 · 시한 넘김이 있나) ② 창 밖 떼기 20회 — 마지막 `dragleave` → `dragend` 간격 ③ 끝 뒤 첫 `pointermove` 가 오나(그 `pointerType` · `buttons`) · 끄는 동안 0 인가 · 떼어진 표시로 머리가 접히는 재배치 뒤 커서를 멈춘 채 합성 `pointermove` 가 오나 · 그 `buttons`(5.1) ④ 끌린 머리를 CSS 로 접어도 `dragend` 가 오나 · `dragstart` 안 DOM 변경이 끌기를 끝내나(정보) ⑤ 탭 하나 팝아웃 `hide()` 20회 — 끌기 계속 · 되보일 때 자리 · 크기 · 최대화 · 그 뒤 탭 클릭 정상 · 최대화였던 · 아니던 탭 하나 팝아웃을 R22 재사용으로 옮기기 각 10회(5.6 — 되돌리기는 없다) — 최대화였던 창은 B′ 에서 깜빡였다 다시 숨나(소스로는 `SW_RESTORE` 뒤 `SW_HIDE` — 5.5) · 깜빡임 길이 · 포커스가 옮겨 가나 · 끝의 되보이기 뒤 놓은 자리 · 보통 크기(Q13)인가 · 최대화 아니던 팝아웃은 옮기는 동안 숨은 채인가 ⑥ Esc keydown 이 웹뷰에 오나 · claude 터미널에 ESC 가 들어가나 ⑦ **트리 슬롯을 마운트한 채(5.1)** — 에이전트 목록 슬롯을 출처 · 대상 창에 보이는 탭 · 안 보이는 탭으로 두고 탭 끌기 시작 · 가이드 · 놓기 각 20회 · 트리 행 재부모화 끌기 10회 · 그 창에 OS 파일 놓기 10회 · 콘솔의 react-dnd 오류 ⑧ **사건 순서(5.1)** — 같은 창 · 이웃 창에서 문턱 직후 곧바로 떼기 20회 — 셸 도착 순서(시작 · 놓기 · 출처 끝) · 붙든 신호 수 · 영역 `rev` 역전 수 · 다시 보낸 영역 수 | ① 결과가 AC19 대로 20/20 · 창 안 Esc 의 「떠남 → 끝」 최대 < `LEAVE_GRACE_MS` · (리뷰 3) 출처 위 Esc 의 `last_gap_ms` 최대 < `CANCEL_LEAVE_MS` · 다른 창 가이드 위 곧바로 Esc = 그 가이드에 20/20 · ② 간격이 시한 안인 비율 기록(덮은 창 오판의 크기) · ③ 20/20 · 끄는 중 0 · 합성 이동은 버튼 0 이 아님(또는 없음) · ④ 20/20 · ⑤ 20/20 · 최대화 아니던 팝아웃의 비침 0/10 · 놓은 자리 ±2 논리 px 10/10 · 최대화였던 팝아웃의 깜빡임 · 포커스 이동은 기록(합격 조건 아님 — 받아들인 경우 · 깜빡인 뒤 숨지 않고 남으면 **멈춤**) · ⑥ 터미널 유입 0/20 · ⑦ 끌기 · 놓기 20/20 · 재부모화 10/10 · 파일 탐색 0/10 · 오류 0 · ⑧ 붙든 신호가 시작에 이어 처리 20/20(표본이 생길 때) · 옛 `rev` 에 머문 놓기 0 · ★(5.7) 빈 보고로 끝난 끌기가 「커서 판정」으로 처리 20/20(끝 줄 `unseen`) · 같은 창 문턱 직후 떼기의 새 창 0/20(구역 안 = 순서 이동) — 2026-10-17 판(§8-1)은 이 기준 전이다★ | ① 간격이 넘으면 시한을 늘려 다시 잰다 · (리뷰 3) `last_gap_ms` 가 넘으면 `CANCEL_LEAVE_MS` 를 늘린다 — 실제 떠남 간격(②)과 겹치면 늘리지 않고 그 갈래를 끈다(0 — 사각형 검사로 되돌림) · 그래도 안 되면 platform 목적 함수(「주 버튼이 지금 눌려 있나」)로 Esc · 떼기를 가르는 안(§1-6 · 사용자 보고) · ③ 실패 = 유실 대비를 「다음 끌기 시작」으로만 · 합성 이동이 버튼 0 이면 재배치 뒤 짧은 시간의 이동을 증거에서 뺀다 · ④ 실패 = 머리를 접지 않고 흐리게만 · ⑤ 실패 = 숨기지 않고 빈 창으로 둠(PRD R2 개정 — 사용자) · 최대화 아니던 팝아웃이 비치면 → 원인 조사 · 놓은 자리가 틀리면 **멈춤**(R22 위치) · 최대화였던 팝아웃의 깜빡임은 받아들임 — 거슬리면 사용자 보고 · 숨긴 채 푸는 platform 함수(미결 · §11-2 #24) · ⑥ 유입 = 삼키는 꼬리를 늘려 다시 · ⑦ 실패 = 트리마다 관리자(`dndManager` · `dnd-core` 직접 의존 — 의존 변경 보고)로 뿌리를 좁히는 안 · 그래도 → **멈춤** · ⑧ 실패 = `EARLY_SIGNAL_MS` 를 늘려 다시 |
| **S2** | W8 — 끌기 그림 내용 | ① 보이는 터미널 칸(WebGL)을 `drawImage` 로 옮긴 그림이 비지 않나 — `preserveDrawingBuffer` 끔 · 켬 · 켰을 때 출력 많은 터미널의 프레임 비용 차 ② `dragstart` 안 합성 시간(칸 4 · 1920×1080 캔버스) ③ 화면 밖 상자의 `setDragImage` 가 그 그림을 쓰나 · 320×240 · 배율 125 / 150% | ① 비지 않음 20/20(어느 설정이든) · 켜야 한다면 프레임 비용 차 ≤ 10% · ② p95 ≤ 8ms · ③ 20/20 | ① 실패 = 터미널 칸도 자리채움(Q5 의 「안 되면」 범위) · ② 느리면 칸 수 · 상한을 낮추거나 제목만 · ③ 실패 = 제목만 상자 · 그것도 안 되면 엔진 기본 그림 |
| **S3** | W9 · AC8 — 바탕화면 · 다른 앱 · 파일 | ① 실제 탐색기 바탕화면 · 탐색기 창 · 메모장 · Edge 위 떼기 각 10회 ② OS 파일을 main · 팝아웃에 끌어 놓기 — 막기 없음 · 있음 각 10회 | ① `none` · 새 창 · 그 앱에 아무것도 안 들어감 40/40 · ② 막기 있으면 탐색 0/20 · 커서 「안 됨」 | ① 받는 앱이 있으면 `DROP_WAIT` 뒤 「마지막 점에 놓기」가 맡는다 — 그 앱에 무엇이 들어갔는지 보고 · ② 탐색이 남으면 **멈춤**(회귀) |
| **S4** | W4 · AC12 — 혼합 DPI · 다중 모니터 | 100% · 150% 모니터 둘 · 각 모니터 창 · 걸친 창에서 칸 경계 ±1 CSS px 를 짚어 그린 띠와 `pick` 대조 · 다른 모니터 빈 곳에 놓아 새 창 · 걸친 창 위 Esc(셸 커서 · 창 사각형 판정) | 판정 = 그린 띠 ±1 CSS px · 새 창이 놓은 모니터에 R22 크기(±2 논리 px) · 창 안 Esc 20/20 | 창 안 판정이 어긋나면 W 의 마지막 보고 점을 화면 좌표로 옮긴 값과 대조하는 안 · 그래도 → **멈춤**(AC12) |
| **S5** | W12 — 콘솔(비 RDP) 세션 · 다른 PC | 2026-10-16 실측의 핵심 수치(메인 스레드 지연 · `dragover` 주기 · `dragend` · 좌표 · MIME)를 로컬 콘솔 세션에서 다시 | 같은 결론 | **멈춤** — 수단 재검토(사용자) |
| **S6** | W5 — tauri #10767 | 팝아웃 제목줄을 main 위로 끌어 놓고(R1) 곧바로 탭 끌기 10회 | AC1 + 뒤이은 탭 끌기 10/10 | 걸리면 탭 끌기 전 웹뷰 포커스를 다시 잡는다(`window.focus`) · 그래도 → **멈춤**(R1) |
| **S7** | (5.7) W13 · W15 · R23 · AC23–AC28 — 순서 이동 · 보고 없는 끝 · 엔진 취소 · (리뷰 1) 넘친 탭바 · 빠른 빼기 | 실 입력(`SendInput` — 2026-10-17 하네스) · 탭 넷(폭이 다른 머리 포함) 창 · 9c 작업 트리에서 그 커밋 전에. ① 끄는 머리 가장자리를 이웃 가운데 ±1 CSS px 로 넘나들기 좌 · 우 각 10 — 비켜나는 점 · 놓은 결과 순서 ② 끄는 머리에 transform 을 건 채 `dragend` · `drop` 이 오나 20 · 끌기가 끊기나 ③ 구역 아래 경계(탭바 + 15 px)를 ±1 px 로 넘나들기 10 — 모드 전환 점 · 접기 · 되살리기 · 전환 렌더의 튐 ④ 구역 밖(캔버스 · 다른 창)으로 나갔다가 돌아와 놓기 10 ⑤ 구역 안 Esc 10 — 결과 순서 · 그 뒤 떼기 · (리뷰 1) 탭 하나 팝아웃 · 메인 마지막 탭 구역 안 Esc 각 10 — 숨는가 · 접힘이 번쩍이나 · 팝아웃이 뛰나 ⑥ 문턱만 넘고 멈춘 채 떼기(300 · 1000 ms) 각 10 · 짧은 톡(2 · 10 · 25 · 50 ms) 각 10 — 같은 창 · 안 보이던 탭에서 각 절반 ⑦ 엔진 취소(같은 창 2 ms 뗌) 20 — `dragstart` → `dragend` 사건 시각 차 분포 · (리뷰 1) `pointerdown` · `dragstart` · `dragend` 의 `timeStamp` 와 처리기 안 `performance.now()` 를 함께 — `dragstart` 시각이 누른 시각인가 ⑧ 탭 하나 팝아웃 · 메인 마지막 탭 — 구역 안 끌기 · 놓기 10 · 구역 밖으로 나가 다른 창에 붙이기 10 ⑨ 이웃 창 `rev` 0 끝(S1 ⑧ 의 이웃 2 ms) 20 ⑩ 놓은 직후 순서 알림 · `Ended` 순서 · 출렁임 ⑪ **이웃 창 멈춘 채 떼기 다시(Q21 · 2026-10-17 판 「이웃 창 4/4 새 창」 재측정)** — 출처 탭을 문턱 넘게 끈 뒤 이웃 창 탭바 · 이음새까지 **끊김 없이**(`SendInput` 이동을 여러 걸음으로 — 한 번에 옮기지 않는다) 움직이고 300 · 1000 ms 멈춘 채 떼기 각 10 · 떼는 점은 출처 사각형 밖에 둔다(§11-2 #29) — 그 판은 커서를 이동 한 번으로 옮긴 하네스 탓으로 의심된다 ⑫ **구역 밖으로 빠르게 빼고 멈춤(리뷰 1)** — 문턱 직후 한 걸음(`SendInput` 이동 한 번)에 캔버스 위 1/3 · 창 밖으로 옮기고 1000 ms 멈춘 뒤 ⓐ 1 px 움직이기 ⓑ 그대로 떼기 각 10 · 여러 탭 창(끄는 탭 = 활성) · 탭 하나 팝아웃 — 멈춘 동안 출처 `dragover` 가 오나(오면 주기) · 도킹 모습까지 ms ⑬ **넘친 탭바(사용자 지시 · 리뷰 1)** — 머리 열둘(탭바 폭의 두 배쯤) 창 · ⓐ 순서 이동 중 오른쪽 · 왼쪽 끝에 대고 2 초 머물기 각 10 — 탭바가 계속 밀리나(엔진 자동 스크롤이 걸리나 · 속도) · 끄는 머리가 커서 밑에 머무나 ⓑ 밀린 뒤 놓기 10 · 밀린 뒤 구역 안 Esc 10 — 적용된 자리 ⓒ 다른 창 넘친 탭바 끝에 대고 머문 뒤 놓기 10(R21) · (리뷰 2) 밀리는 동안 놓기 10 ⓓ (리뷰 2) 출처 탭바가 밀리는 **동안**(멈추지 않고) 구역 안 Esc 10 — 끝 줄에 `last` 를 썼나 · 셸이 받은 마지막 `Over` 자리와 다른 끝 수 기록 · (리뷰 3) 끝 줄 `last_gap_ms` 분포(엔진 접기의 떠남 → 끝 — `CANCEL_LEAVE_MS` 를 고친다) | ① 비켜나는 점 = §2-5 식 ±1 CSS px 20/20 · 결과 순서 = 놓기 직전 보인 순서 20/20 ② 20/20 이어짐 ③ 전환 점 = 경계 ±1 px 10/10 · 튐 0 ④ 순서 이동 적용 10/10 ⑤ 보이던 자리 10/10 · 떼기 무동작 · 탭 하나 팝아웃 · 메인 마지막 탭 = 숨지 않음 · 접힘 번쩍임 0 · 제자리 · 무변경 20/20(AC25 · AC26) ⑥ 새 창 0/60 · 결과 = 놓은 자리 순서 이동(대개 제자리 · 안 보이던 탭은 보임) ⑦ 새 창 0/20 · 떼어진 모습 0/20 · 차 분포 기록(`ENGINE_CANCEL_MS` 검증 — 실제 끝 최소와 겹치면 불합격) · `dragstart` 시각 기준 기록 — 누른 시각이라 거름이 드물게만 걸려도 물러날 자리(출처 커서 판정 = 제자리 순서 이동 · 무해 — §1-4 「엔진 취소」)로 위 둘이 서면 합격 ⑧ 구역 안 무변경 10/10 · 팝아웃이 구역 안에서 숨지 않음 10/10 · 밖에서 숨음(지연 ms 기록) · 붙이기 10/10 ⑨ 무변경 0 · 그 자리에 적용 20/20(사용자 결정 Q21 — 구역 = 순서 이동 · 이음새 · 끼울 자리 = 거기 · 칸 가운데 = 새 창) ⑩ 출렁임 횟수 기록 ⑪ 이웃 창에 붙음 20/20(탭바 = 그 창의 새 탭 · 이음새 = 그 자리) · 새 창 0/20 · 끝 줄 `end_by` · `unseen` · 이웃 창 `Over` 수 기록 ⑫ 멈춘 동안 출처 `dragover` 를 받은 끌기 = 멈춘 뒤 200 ms 안 도킹 모습 · 받지 못한 끌기 = 순서 이동 모습 그대로(받아들인 틈 — 수 기록 · §11-2 #35) · ⓐ 움직인 뒤 200 ms 안 도킹 모습 20/20 · ⓑ 결과 = 커서 판정(칸 가운데 · 창 밖 = 새 창 · 끄는 탭이 활성인 창의 이음새 자리 = 새 창 — 승계 탭에 붙음 0) 20/20 · 순서 이동 적용 0 ⑬ ⓐ 계속 밀림 20/20 · 끄는 머리 화면 x = 점 − `grab_x` ±1 CSS px(양 끝 고정 전) · 엔진이 밀었나 기록 ⓑ 적용 자리 = 놓기 직전 그려진 자리 20/20 ⓒ (리뷰 2) 막대가 눈에 보이던 틈에 들어감 20/20 · 잘못 붙음 0/20 · 무변경도 불합격(5.7 첫 판의 「아니면 무변경(옛 `bar_rev`)」 합격을 버렸다 — 연속 스크롤 동안엔 크롬 보고가 안 터져 무변경이 아니라 잘못 붙었다 · §1-4 「모드」 넘친 탭바 ④) ⓓ 적용 = Esc 순간 그려진 자리 10/10 · (리뷰 3) `tight` 10/10 | ② 끊기면 **멈춤** — 머리 대신 겹친 복제 요소를 옮기는 안(수단 바꿈 · 사용자 보고) · ① 어긋나면 골든 · 거울부터(갈림 = 무변경이라 잘못 붙지는 않는다) · ③ 튐 = 전환 렌더 규칙 고침 · 흔들림이 거슬리면 이력 안(§2-5 · 사용자 보고) · ⑥ 새 창이 남으면 끝 줄 `end_by` · `unseen` 으로 갈래를 찾고 → 못 닫으면 **멈춤** · ⑦ 겹치면 시한을 다시 잡는다 · 누른 시각 기준에서 새 창 · 떼어진 모습이 나오면 물러날 자리 결함(**멈춤**) · ⑩ 잦으면 알림 · `Ended` 순서를 같은 emit 줄로 묶는 안 · ⑪ 새 창이 남으면 끝 줄로 가른다 — 이웃 창이 `Over` 를 냈는데 새 창이면 장부 결함(**멈춤**) · 끊김 없이 움직였는데도 `Over` 0 이면 사람 손에서도 생기는 일이라 §11-2 #29 를 다시 열어 사용자에게 올린다(커서 밑 맨 위 창 platform 함수 — ADR-0266 · 지금은 들이지 않는다) · ⑫ ⓐ 에서 모습이 안 바뀌면 판정 순서 · 모드 규칙 결함(고침) · ⓑ 에서 승계 탭에 붙으면 장부 캔버스 거름 결함(**멈춤**) · ⑬ ⓐ 엔진이 안 밀면 우리 끝 띠 시한으로(9c 안 · 커밋 전 · §1-4 「모드」) — 그것도 안 되면 **멈춤**(사용자 지시 — 사용자 보고) · ⓑ 어긋나면 내용 좌표 · 보고 `scroll` 부터(갈림 = 무변경이라 잘못 붙지 않는다) · ⓒ 어긋나면 내용 좌표 크롬 · 보고 `scroll` · 막대 변환(x − 지금 스크롤)부터(리뷰 2 — 디바운스를 줄이는 안은 버렸다 · §7 #56) · ⓓ 어긋나면 끝 스냅숏(`last` — 떠남 사건 시각 · 그린 판정 쥐기)부터 · 그래도 → **멈춤** |

- **macOS** — WKWebView 의 창 사이 끌어 놓기 · 끄는 중 포인터 사건 · 끌기 그림은 이식 때 S1 · S2 · S4 를 같은 꼴로 돌린다(W11).

## 9. 테스트 계획

층마다 seam 을 끊어 단독으로 잰다(ADR-0012). 대상은 CLAUDE.md 「빌드 · 검증 명령」의 기존 타깃이고 새 타깃은 만들지 않는다.

| 층 | 무엇 | 어디(타깃) |
|---|---|---|
| 트리 연산(끝) | `docked_tree` 네 변 × (슬롯 · 바깥) · 1/3 · 2/3 · 안쪽 비율 보존 · id 보존 · 새 면적 0 · `incoming_rect` | `layout/dock.rs` → `cargo test -p engram-dashboard --test lib_unit` |
| 모델(끝 + 4 단계) | `dock` 세 모드 판정 순서 · 사유마다 무변경 · 성공마다 `version` +1 한 번 · 곁표 · 탭바 자리 · 승계 · 빈 팝아웃 제거 · `Keep` / `Reveal` · 같은 창 활성 · 표식 · `SourceChanged` · `TargetChanged` · 묘비 · `WindowExists` · `plan_new_window` · `succession` = `close_tab` · **(4 단계) 메인 마지막 탭 → 다른 창 Seam · TabBar · NewWindow 셋 다 메인에 빈 탭 하나 · 그 탭이 활성 · `lease_tab` · `plan_new_window` 가 메인 마지막 탭을 받음 · 같은 창(SelfTarget · 같은 자리)은 빈 탭을 안 만듦 · `close_tab` 의 기존 빈 탭 시험이 도우미 뽑기 뒤에도 초록 · (4 단계 고침 둘) `TargetExpect::Seam` — 대상 탭 배치가 같아도 대상 창 활성이 다르면 `TargetChanged` · 무변경 · 같은 창 도킹(활성 = 끄는 탭)의 기대값은 그대로 통과 · TabBar 는 활성과 무관 · (5.6 — 4 단계 고침 셋을 거둠) `plan_new_window` — 탭 하나 팝아웃이면 보통 자리 `None` 이어도 · 최대화 중이어도 `Reuse{그 창}`** · 매 시험 뒤 `assert_invariants`(ADR-0057 ①–④) | `layout/manager.rs` → `lib_unit` |
| 닫기 요청(3 단계) | 닫기 요청 정리 뒤 그 창 탭이 모델에서 사라짐 · 묘비 · `Destroyed` 정리 멱등 · 뒤이은 Seam · TabBar · 새 창 · 팝아웃 기존 · 새 창 타깃 전부 무변경 · 닫히는 창의 늦은 replay 는 Subscribe 0 | `commands/popout.rs` · `daemon_client/tests.rs` → `lib_unit` |
| 적용 서비스(4 단계) | 가짜 포트로 Seam · TabBar 의 변경 · 스냅숏 · `resync` 가 한 락 안 · 알림은 락 밖 · 출처 창 닫힘 = 알림 뒤 `host.close` 한 번 · 실패면 알림 0 / NewWindow 열기 실패 = 무변경 · phase C 실패(묘비 포함) = 연 창을 닫고 탭은 출처 · **재사용(Q19 · 5.6)** = `move_window` 한 번 · 같은 락 안에서 표식 내려감 · 모델 `version` 불변 · `record_placement` 한 번 / **옮기는 사이 끼어든 쓰기**(가짜 창 포트의 `move_window` 가 그 자리에서 출처 탭에 슬롯 더함 · 그 팝아웃에 탭 더함) = 그대로 성공 · 다시 확인 없음 · 되돌리기 없음 / 그 사이 출처 탭 닫음 = 그대로 `Ok`(그 창이 모델에 없으면 `record_placement` 0) / `move_window` 실패(위치 · 크기 걸음) = `WindowOpen` · 되돌리기 없음 · 모델 그대로 · 표식 그대로(장부 끝이 내린다) · 끝 줄 warn 에 그 걸음 / **끝 줄 로그 붙잡기(가짜 tracing 구독자)** — 같은 창 같은 자리 탭바 놓기 = `changed = false` / 모델 `bounds` 없는 탭 하나 팝아웃도 재사용(`move_window` 한 번 — 걸침 판정은 지금 안쪽 크기로 · 5.6) · `activate` = `FocusPolicy` · 새 창의 안 걸치는 자리 = 기본(계단식) 자리 · 재사용의 안 걸치는 자리 = 위치 걸음 없이 제자리(5.5) · 새 창 만든 뒤 자리 입히기 실패 = 그 창 거둠 · `WindowOpen`(5.5) · 메인 빈 탭이 알림의 메인 탭 목록에 실림 · `settled` 는 알림을 낸 쓰기마다 그 묶음 끝에 정확히 한 번(`dock_tab` · 기존 쓰기 · 복원 수락 — 가짜 이벤트가 순서를 적는다) · 실패 · 무변경 쓰기 · 재사용 성공 · 창 통째 닫기(팝아웃 마지막 탭 `close_tab` · `close_window`) · `create_window` 는 `settled` 0(5.5) | `tests/layout_apply.rs` → `--test layout_apply` |
| 판정(끝) | `locate` · `choose_seam` · `choose_band` 의 규칙 전부(5 단계 시험 그대로) | `dock/hit.rs` → `lib_unit` |
| 영역 · 거울(6 단계) | `regions_for` — 숨긴 출처 없음 · `blocked` = 비움 · 크롬 · 캔버스 없음 = 칸 · 바깥 없음 · 출처 머리는 크롬에 남아 있어도 id 로 걸러짐 · 보이는 탭 = `display_active` · 면적 0 띠 빠짐 · `insert_xs` = `tab_insert_x` · 기대값(`Seam{배치, 모델 활성}` — 출처 창에서 끄는 탭이 활성이면 그 탭 · `Tabs`) / `pick` = 골든 전부 / **골든 사례가 덮을 것** — 각 변 띠 · 가운데 · 모서리 정규화 거리 · 바깥이 이김 · 무효 바깥 → 밑 칸 · 경계 픽셀(네 변 · 1px 띠) · 탭바 자리(가운데 x · 출처 머리 거름) · `over` · 캔버스 밖 · 보이는 탭 없음 / 철자 — `Picked` · `DockRegions` 의 wire 꼴(snake_case · 문자열 id) | `dock/regions.rs` · `dock/types.rs` → `lib_unit` · 같은 골든을 vitest |
| 장부(6 단계) | 가짜 시각 · 커서 · 사각형 · 스냅숏으로 `step` 의 할 일: 시작(숨김은 탭 하나 팝아웃만 · `Started` · 창마다 영역 · 메인 마지막 탭 = `display_active` 없음) · 영역은 (영역, 기대값)이 바뀐 창만 `rev` +1 · 기하가 같고 기대값(대상 칸 내용 · 창 활성)만 바뀌어도 `rev` +1 · 옛 스냅숏 번호 버림 · 세션 전 스냅숏(번호 큼)은 `Start` 뒤 이어 먹힘 · 작으면 버림 · 출처 배치 바뀜 = `blocked` 로 모든 창 다시 · `display_active` 바뀜 = `Display`(값이 바뀔 때만 · `Started` 와 같은 칸) · 출처 View 없음 = `SourceGone` 끝(되보이기 없음) · 출처 `Destroyed` = `SourceDestroyed` 끝 · 스냅숏에서 빠진 창 = `sent` 지움 · `seen` 남음 / **놓기 판정** — `blocked` = 무변경 · 보인 것 없음 + 지금 판정 없음 = 새 창(`rev` 무관) · 보인 것 없음 + 지금 판정 있음 = 무변경 · 보인 것 있음 + W 영역 없음(빠짐 · 소멸) = 무변경 · 이음새 — `rev` 다름 = 무변경 · `displayed` ≠ 그 `rev` 의 `view_id` = 무변경 · 다시 고른 것 ≠ 보인 것 = 무변경 · 모두 같음 = 적용 1회(`Reveal` · 표식 · 기대값 `Seam{배치, 창 활성}`) · **탭바(5.2)** — `bar_rev` 같고 같은 `index` = 적용 1회(기대값 `Tabs`) · `rev` · `displayed` 가 달라도(활성 전환 직후) 적용 · `bar_rev` 다름 = 무변경 · 활성만 바뀐 스냅숏 = `rev` +1 · `bar_rev` 그대로 · 탭 순서 · 탭바 기하가 바뀐 스냅숏 = 둘 다 +1 · 탭바 `over` 차이는 같음으로 / **끝 신호** — `drop` 뒤 `dragend(move)` = 늦은 신호 · `dragend(move)` 먼저 → `drop` 이 시한 안 = 적용 · 시한 지남 = 마지막 점(그 끝 신호 때 커서) · `dragend(none)`: W(마지막 Over 창 — 뒤에 Left 가 와도 W) + 아직 위 + 커서 안 = W 마지막 보고로 판정 · Left 가 유예 안 + 커서 안 = 같음 · Left 가 유예 밖 = 새 창 · 아직 위 + 커서가 W 밖 = 새 창(보인 가이드와 무관) · W 사각형 없음(소멸 · 최소화) + 가이드를 보임 = 무변경 · 가이드 없음 = 새 창 · 보고 없음 = 새 창(★5.7 이 바꿈 — 커서가 출처 안이면 출처 커서 판정 · 9b 행★) · 커서 못 읽음 = W 마지막 점(아직 위 = 판정 · 아니면 그 점의 화면 자리에 새 창 · W 없음 = 기본 자리) · 숨긴 출처는 후보 아님 · `blocked` 면 어느 끝이든 무변경(새 창 없음) / 시한 — 지난 번호의 `Timeout` 버림 · `Grace` 중 `dragend(move)` = `AwaitDrop` 으로 · `AwaitDrop` 중 `pointer_seen` 버림 · `drop` 의 `view_id` 가 출처가 아니면 버림 / 유실 — `pointer_seen` → 시한 안 끝 신호 = 평소 · 시한 지남 + W 아직 위 = 그 보고로 판정(사각형 검사 없음 · W 소멸이면 무변경) · W 떠남 = 지금 커서에 새 창 / **이른 끝 신호** — 모르는 id 의 `drop` · 출처 끝 = 붙듦 → 시한 안 그 id 의 `Start` = 시작 할 일 뒤 도착 순서대로 처리 · 시한 지남 = 버림 + 그 id 가 `ended` 로 · `Refused` = 버림 · 다른 모르는 id 가 오면 옛 것 버림 + `ended` 로 · 최근 끝난 id = 붙들지 않고 버림 · 버린 id 의 늦은 시작 = `LateStart` 거절(표식 · 숨기기 · `Started` 0) · 붙든 놓기(보인 것 없음)는 지금 판정이 가이드면 무변경(★5.7 이 바꿈 — 빈 보고 → 커서 판정 · 그 자리에 적용(Q21) · 9b 행★) / **순서** — 옛 `rev` 의 `Over` = 지금 영역을 같은 `rev` 로 다시 보냄 / **적용 중** — Seam 성공 중 출처 View 없는 스냅숏 = 끝나지 않음(영역 · `Display` 도 없음) → `Applied(Ok)` = applied · 대상 `Focus` · `Ended` 는 `Applied` 뒤 / 탭 하나 팝아웃 출처가 적용 중 `Destroyed` = 끝나지 않음 → `Applied(Ok)` = applied · 출처 되보이기 없음 · 대상 `Focus` / `Applied(Err)` = 무변경(사유 그대로) / 한 세션에 적용 한 번 · `Applying` 중 끝 신호 버림 / **새 시작** — 옛 세션 `Live` = 새 것 거절 · 옛 것 마지막 점 · 옛 세션 `Applying` = 새 것만 거절(`applying`) · 옛 것은 `Applied` 로 끝 / 끝 할 일 순서 `Ended` → 되보이기 → `set_focus` → `release_lease` · 재사용 = 보이기 · `seq` 단조 · 끝난 id 가 `ended` 에 남음 | `dock/book.rs` → `lib_unit` |
| 실행기(6 단계) | 순수 조각 — 새 창 자리 = 커서 − 그림 커서 자리 × 배율 · 커서 못 읽음 = 마지막 점 · 그것도 없으면 자리 없음 · 사각형은 보이고 최소화 안 된 창만 / 계기 깃발 — 세션 없으면 스냅숏을 안 뜬다 · 깃발은 시작의 레이아웃 락 안에서 세대로 서고(`lease_tab` 성공 뒤) 끝에서 자기 세대일 때만 내려간다(다음 세션이 세운 세대는 옛 세션의 끝이 못 내림 — 5.2) · `settled` 한 번에 스냅숏 하나 / 게터 — 가짜 창 포트가 불릴 때 레이아웃 락이 풀려 있음 / **`Apply` 는 늘 spawn(5.2)** — 가짜 spawn 포트로: 동기 시작이 붙든 놓기를 이어 먹여 낸 `Apply` 도 spawn 으로 가고 시작 명령 안에서 적용 서비스 · 창 포트가 불리지 않음 · `Applied` 와 끝 할 일이 그 spawn 작업에서 먹힘 | `commands/dock.rs` `#[cfg(test)]` → `lib_unit` |
| 프론트(7 단계) | `TabBar` — 머리 `draggable` · 이름 입력 · 닫기 버튼에서 시작한 끌기 막음 · `dragstart` 가 우리 MIME 하나만 · `effectAllowed = move` · `setDragImage` 호출 · `dragstart` 안 DOM 무변경 · `dock_drag_start` 는 디스패치 뒤 · 다른 리스너가 `dragstart` 를 취소하면 0 · `dragend` → `dock_drag_end{effect}` / **끊기(5.1)** — window 에 react-dnd 꼴 리스너(`preventDefault` · `dropEffect = 'none'`)를 단 jsdom 에서 우리 `dragstart` · `dragenter` · `dragover` · `drop` 이 거기 닿지 않음 · 우리 MIME 이 아닌 끌기는 닿음 / 대상 — 우리 MIME 이면 세션을 몰라도 받음 · `blocked` = `dropEffect none` · `pickTarget` 결과대로 `DockOverlay` · 보이던 탭 ≠ 영역 `view_id` = 그리지 않음 · `shown` · `rev` · `displayed` 가 바뀔 때만 `Over` · `drag_id` 모르면 보고 0 · 떠남 = `Left` · `drop` = `dock_drop{rev, bar_rev, displayed, shown, point}` · 보이던 탭 ≠ 영역 `view_id` 여도 탭바 가이드는 그리고 탭바 놓기를 보낸다(5.2) · 영역 없음 = `dock_pull` 한 번 · 머묾 500ms = 탭 전환 · 떠남 · `Ended` 가 머묾 취소 / 영역 · 세션 — `drag_id` 별 가장 큰 `rev` · `seq` 만 · 작은 것 버림 · `Started` 전 영역 받음 · `Display` 가 `Started` 보다 먼저 와도 같은 상태 · `Ended` 된 id 버림 / 떼어진 표시 — 스토어 `win.tabs` 그대로 · 0탭 자가닫힘 안 불림 · 머리는 DOM 에 남고 접힘 · `displayActive` 로 캔버스 · 메인 마지막 탭 = 빈 캔버스 · `Ended` 로 거둠 / `pointer_seen` 은 세션당 창당 한 번 · 마우스 · `buttons === 0` 이동 · `pointerdown` 만(버튼 쥔 이동 · 펜 · 터치는 아님) / Esc — 세션 + 꼬리 동안 capture 단계에서 삼킴 · 그 창의 끝 증거 뒤 200ms 에서 그침 · 그 밖은 그대로 / 파일 막기 — 우리 MIME 이 아닌 끌기는 `dropEffect none` · `drop` `preventDefault` · 끊지 않음 / `pickTarget` = 골든 / 끌기 그림 크기(720×500 → 288×200 · 1600×1000 → 320×200 · 400×300 → 180×135 · 제목만 180×28) · 그림 실패 = 제목만 / `dockChromeReport` — 한 번에 하나 · 마지막 값이 덮음 · 접힌 머리 뺌 | vitest(`npm test`) + `npx tsc --noEmit` |
| 순서 이동 규칙 · 영역(9a · 5.7) | `reorder_zone` — x 그대로 · y 위아래 15 · 반열림(위 경계 포함 · 아래 경계 밖) / `reorder_left` — 커서를 따름 · 왼쪽 끝 `H[0].x0` · 오른쪽 끝 `H[n−1].x1 − w` 고정 · 머리 하나 = 늘 `H[0].x0` / `reorder_index` — 왼쪽 이웃 `m = L` 이면 그대로 · `L` 이 1 ULP 작으면 비킴 · 오른쪽 이웃 `m = R` 이면 그대로 · 1 ULP 크면 비킴 · 맨 왼쪽 0 · 맨 오른쪽 `n−1` · 같은 자리 = `d` · 폭이 다른 머리 / `regions_for` — 출처 창에만 `reorder` · `strip` 없음 = 없음 · id 순서 ≠ 모델 탭 = 없음(더함 · 뺌 · 옮김 각각) · 끄는 머리 없음 · 둘 = 없음 · `x0` 오름차순 아님 · 유한하지 않음 = 없음 · `blocked` = 없음 · 숨긴 출처 = 영역 없음(그대로) · 다른 창 = 없음 · `tab_bar` 는 그대로 실림 / `pick` — 구역이 탭바 · 바깥 위 띠 · 칸 위 띠보다 먼저 · 구역 아래 경계 바로 밑 = 지금 판정 · 탭바 오른쪽 빈 곳 = 맨 끝 자리 / `bumps` — `reorder` 생김 · 빠짐 = `rev` · `bar_rev` 둘 다 / 골든 — `source_strip` · `source_lone` · (리뷰 1) `source_overflow` 사례 전부(`Picked` + `left` · 사례의 `scroll`) / `pick` 의 `scroll` — 같은 점 · 다른 스크롤 = 다른 `Reorder` 자리 · 구역 · 탭바 상자 경계 · 캔버스 판정은 스크롤 무관 · (리뷰 2) 탭바 자리(`TabBar.index` · `over`)도 같은 점 · 다른 스크롤 = 다른 자리(`target_overflow` 사례) · `scroll` 없음 = 영역마다 잰 때의 것(`reorder.scroll` · `tab_bar.scroll`) · 스크롤 0 · 크롬 `scroll` 없음 = 6a 판정 그대로(골든 옛 사례 무변경) / (리뷰 2) `bumps` — `tab_bar.scroll` 만 다름 = 둘 다 안 올림 / wire — `StripStart` · `ReorderRegion` · `Picked::Reorder` 철자(snake_case · `type = "reorder"`) · `DockRegions.reorder` 가 `null` 로 나감 · (리뷰 2) `DockChrome.scroll` 없음 = 0 으로 풀림 · `TabBarRegion.scroll` · `ReorderRegion.scroll` 철자 / (리뷰 2) TS — `dockChromeOf` 가 `scroll: null` 을 싣고 `npx tsc --noEmit` 초록 · (9a 코드 리뷰) `evaluate` → `deps.pick` 이 스크롤 `null` 을 넘기고, 골든 `target_overflow` 의 스크롤 없는 사례를 실제 `pickTarget` 으로 그린 · 보고한 `shown` = 골든(= 셸 `pick(None)`) | `dock/hit.rs` · `dock/regions.rs` · `dock/types.rs` → `lib_unit` · 같은 골든을 vitest(`dockPick.test.ts`) |
| 장부(9b · 5.7) | **순서 이동 놓기(4c)** — 출처 `drop{Reorder i}` + 다시 고른 것 같음 = TabBar{출처, i} 적용 1회(기대값 `Tabs` · `Reveal` · `mode = reorder`) · `rev` · `bar_rev` · `displayed` 가 달라도 적용 · 다시 고른 것 다름 = 무변경 · 영역에서 `reorder` 가 빠진 뒤 = 무변경 · 다른 창의 `Reorder` 보고 = 무변경 · 구역 안 Esc(`none` 끝 · W = 출처 · 아직 위) = 그 보고의 자리 / **빈 보고** — `shown` 없음 + `rev` 0 · W 영역 없음 = 커서 판정 · `shown` 없음 + 0 아닌 옛 `rev` · 지금 `rev` = 5.6 그대로(보고다 — 리뷰 1) · `shown` 가이드 + 옛 `rev` = 무변경(그대로) / **커서 판정** — W 없음 + 커서가 출처 안 = 출처 영역으로(구역 = 순서 이동 · 칸 가운데 = 새 창 · 이음새 · 끼울 자리 = 적용 — Q21) · W 없음 + 출처 밖(다른 우리 창 안이어도) = 새 창 · W 아직 위 + 빈 보고 + 커서 안 = W 영역 · 끝 순간 커서로 · `drop` 의 빈 보고 = 놓은 점으로 · 붙든 이른 놓기 = 같음 · 커서 못 읽음 + 빈 보고 = 그 보고 점으로 · 유실 · 새 시작 + W 없음 = 출처 커서 판정 없음(새 창 — 5.6 그대로) · 유실 + W 아직 위 + 빈 보고 = 그 보고 점으로 커서 판정(리뷰 1 — §1-3 「T 와 p」) · **(리뷰 1) 캔버스 거름** — 출처 보고 없음 + 끄는 탭이 활성 + 커서가 출처 캔버스 이음새 자리 = 새 창(승계 탭에 붙지 않음) · 끄는 탭이 활성 아님 = 그 이음새 · 빈 보고 `displayed` ≠ 영역 `view_id` + 이음새 자리 = 새 창 · 탭바 · 구역 = 거름 없음 · **(리뷰 1) 스크롤** — 보고 `scroll` 로 4c 다시 고르기(같은 점 · 다른 스크롤 = 다른 자리) · `scroll` 없는 보고 · 보고 없는 끝 = `strip.scroll` · 숨긴 출처 = 후보 아님 · `blocked` = 무변경 / **미룬 숨기기** — 탭 하나 팝아웃 + `reorder` 섬 = 시작에 `Hide` 없음 · 출처 `Over{Reorder}` = 없음 · 출처 그린 `Over{None}`(지금 `rev` · 막히지 않음) · 다른 창 `Over`(빈 보고여도) = 그 걸음에 `Hide` 한 번 · 그 뒤 출처에 영역 안 보냄 · 출처 `sent` 는 남김(리뷰 2 — 5.7 첫 판의 「지움」을 버렸다) · (리뷰 1) 출처 `Over{rev 0, None}` · 막힌 영역 `Over{None}` · 옛 `rev` 보고 = 없음 · 출처 `Left` = `ArmTimer{Leave}` 만 → `Live` 에서 시한 지남 = `Hide` · 시한 전 끝 신호(구역 안 Esc — `Left` 뒤 `none` 끝) = 없음 · 그 끝은 W = 출처 아직 위 · 4c 그 자리 · 되보이기 없음 · 시한 전 출처 `Over{Reorder}` = 없음(번호 버림) · `Live` 가 아닐 때 닿은 시한 = 버림 · 둘째 증거 = 없음 · `Applying` 중 = 없음 · `reorder` 없음 = 시작에 숨김(5.6) · 구역 안 놓기 끝 = 되보이기 없음 / **엔진 취소** — `SourceEnd{canceled}` = 무변경(`EngineCanceled` · `engine_cancel`) · 붙든 것도 같음 · `canceled` 없음 = 5.6 그대로 / **(리뷰 2) 끝 스냅숏 `last`** — ★가장자리 스크롤 중 늦은 `Over` + Esc★: 출처 `Over{Reorder i, scroll s1}` 받음 → `SourceEnd{none, last = {Reorder j, scroll s2}}`(그 `Over{j}` 는 아직 안 닿음) = TabBar{출처, j} 적용(적용 = 그린 것) → 뒤늦은 `Over{j}` = 늦은 신호 버림 · `last` 없음 = 받은 마지막 `Over`(i — 5.7 첫 판 그대로) · (리뷰 3) 다른 창 W2 가 도착 순번상 마지막이면 `last` 가 있어도 W = W2(`last` 안 씀 · `not_w`) · ★출처 → W2(가이드) → 출처 떠남 100 ms 안 Esc★: 출처 `Left` → W2 `Over{가이드}` → W2 `Left`(Esc) → `SourceEnd{none, last(left_before_end_ms 60)}` · 커서 W2 안 = W2 그 가이드 적용(리뷰 2 판은 새 창) · W2 에서 출처로 돌아온 뒤 출처 `Over` 가 아직 오는 중인 끝 = W = W2 · 커서 W2 밖 = 새 창(5.6 그대로 · §11-2 #36) · W = 출처 + `last`(간격 > `CANCEL_LEAVE_MS`) + 커서가 출처 사각형 밖 = (리뷰 4 — `OverWait` 시한 뒤) 새 창 · W = 출처 + `last`(간격 ≤ `CANCEL_LEAVE_MS` · 또는 떠남 없음) + 커서가 출처 밖 = 사각형 검사 없이 `last` 로 4c(`tight`) · W = 출처 + `last` + 셸이 받은 출처 `Left` 가 끝 도착보다 100 ms 넘게 앞섬 = 아직 위(출처 시계) · `last` 가 빈 보고 = 커서 판정 · 숨긴 출처(증거 ① · ③)의 `last` = 버림(W = 도착 순번) · `effect = move` 의 놓기 기다리기 시한 뒤에도 `last` / **(리뷰 4) 오는 중 `Over` 붙들기** — ⓐ 출처 `Over{가이드 아님}` → 출처 `Left` → `SourceEnd{none, last(간격 60), 커서 = W2 안}`(W2 `Over` 는 아직 안 닿음) = `AwaitOver` · 할 일 = `ArmTimer{OverWait, gen, 50}` 하나(적용 · `Ended` · `Show` 없음) → +20 ms W2 `Over{가이드 g}` = 할 일 없음 · `Hide` 없음(증거 ③ 아님) → +22 ms W2 `Left`(Esc) = 곧바로 W2 의 g 적용(커서 · 사각형 = 끝 때 것 · `last = not_w` · `over_wait = hit` · `over_from = W2`) · 같은 순서에 W2 `Left` 없음 = 시한에서 같은 g 적용(`timeout`) · W2 `Over{g1}` → `Over{g2}` → `Left` = g2 적용(g1 에서 풀지 않음) · 붙든 동안 닿은 늦은 출처 `Over` = W 를 출처로 되돌리지 않음 · (리뷰 5) 출처 → X → Y: 끝 커서는 Y 안 · X 의 늦은 `Over` · `Left` 가 먼저 닿음 = 풀지 않음(`from` 은 X 가 아니다) → +30 ms Y `Over{g}` → Y `Left` = Y 의 g 적용 · Y 가 안 오면 시한에서 「마지막 점에 놓기」 그대로 · (리뷰 6) X · Y 가 겹쳐 둘 다 끝 커서를 품음 · X 의 늦은 `Left` 먼저 = 풀지 않음 → Y `Over{g}` 뒤 시한 = Y 의 g 적용(도착 순번) · 시한 콜백의 커서 · 사각형(W2 밖)을 줘도 끝 때 것으로 판정 · ⓑ 아무 `Over` 도 안 옴 → `OverWait` 시한 = 붙들기 없이 판정한 것과 같은 결과(커서 출처 밖 = 새 창 · 출처 안 = 지금 갈래 그대로) · 붙들지 않음 = 간격 ≤ `CANCEL_LEAVE_MS` · 떠남 없음 · `last` 없음 · 도착 순번 W 가 다른 창 · `canceled` · `blocked` · `effect = move` 의 놓기 기다리기 시한 · 유실 시한 · 붙든 동안 `dock_drop` · 둘째 `SourceEnd` = 늦은 신호 버림 · `pointer_seen` = 버림 · `Leave` 시한 = 버림 · 스냅숏이 `blocked` 를 세움 → 풀 때 무변경 · `Destroyed{출처}` = 그냥 끝 · 풀린 뒤 닿은 `OverWait` · 지난 `gen` 의 `OverWait` = 버림 · ⓒ 붙든 동안 새 `Start` = `AlreadyDragging` 거절 + 붙든 세션을 붙든 끝으로 곧바로 「마지막 점에 놓기」(`over_wait = cut` · 커서 = 끝 때 것 · 붙든 동안 닿은 다른 창 `Over` 가 있으면 그 창) · 그 뒤 `OverWait` 시한 = 버림 · 그 뒤 새 시작 = 받음(표식이 내려간 뒤) · 붙든 동안 세션은 살아 있음(`ended` 에 없음 — 다른 창 `Over` 가 모르는 id 로 버려지지 않음) / **(리뷰 4) W 없음 + `last`** — 출처 첫 그린 `Over{Reorder j, scroll s2}` 가 아직 안 닿은 채(셸이 받은 `Over` 0) `SourceEnd{none, last = {Reorder j, scroll s2}, 간격 없음}` = W = 출처 · TabBar{출처, j} 적용(`strip.scroll` 로 다시 고른 다른 자리 아님 · `lifted`) · 간격 ≤ `CANCEL_LEAVE_MS` + 커서 출처 밖 = 같음(사각형 검사 없음) · `last` 가 빈 보고 = `last` 점 · 스크롤로 출처 커서 판정 · 간격 > `CANCEL_LEAVE_MS` = 붙들기 → 시한 = W 없음 갈래 그대로(`not_w` · `lifted` 아님) · 증거 ① 로 숨긴 출처의 `last` = 버림(세우지 않음 — W 없음 갈래) · `last` 없음 = 지금 W 없음 갈래 그대로 / **(리뷰 2) 세션 번호** — 끝난 세션의 `Leave` 시한이 새 세션 `Live` 중에 닿음 = 버림(`Hide` 0 · 새 출처 그대로) · 지난 `gen` 의 `Timeout` 은 종류 무관 버림 / **(리뷰 2) 시한이 앞지른 끝** — 탭 하나 팝아웃 · 구역 안 Esc: 출처 `Left`(t0) → `Timeout{Leave}`(t0 + 100 · 먼저 먹힘 → `Hide`) → `SourceEnd{none, now = t0 + 95, rects 에 출처 없음}` = 출처 아직 위 · 끝까지 남긴 `sent` · 커서가 쥔 `source_rect` 안(리뷰 3 — 사각형 검사 그대로) = 4c · 되보이기 `Show` · 새 창 · 재사용 0 · (리뷰 3) 같은 순서 + 커서가 `source_rect` 밖(바탕화면에 빠르게 던짐) = R22 재사용 · 시한이 끝보다 늦게 먹혀도(숨김 없음) 같은 R22(시한 흔들림과 무관) · `last` 간격 ≤ `CANCEL_LEAVE_MS` + 커서 밖 = 사각형 검사 없이 4c · `source_rect` 없음 = 「사각형 없음」 갈래 · `source_rect` = 시작 값 → 출처가 든 `Rects` 가 갈아 끼움 · 숨긴 뒤 `Rects`(출처 없음)는 그대로 둠 · `now` = t0 + 105 면 숨긴 뒤의 끝(「W 를 떠남」 — 5.7 첫 판 그대로) · 숨긴 뒤 스냅숏이 그 창 `sent` 를 지우지도 고치지도 않음 · `now` = t0 + 105 여도 끝이 `last` 를 실었으면 = 숨기지 않은 것으로(커서 `source_rect` 안 = 4c · 되보이기) · 증거 ① · ③ 으로 숨긴 출처 = 이 예외 아님(`last` 가 있어도) · `visibility` — 살아 있는 세션의 숨긴 출처 = 숨김 · 끝난 뒤 = 보임 / 끝 줄 `unseen` · `strip` · `hide_deferred` | `dock/book.rs` → `lib_unit` |
| 실행기(9b · 5.7) | `dock_drag_start` — `strip` 없음 · 있음 둘 다 받음(없음 = `None`) · `dock_drag_end` — `canceled` 없음 = 거짓 · (리뷰 2) `last` 없음 = `None` / 뷰포트 점 변환 — 화면 물리 → 그 창 뷰포트 CSS(`screen_point` 의 역 · 배율 1.25 · 2 · 음수 원점) / **(리뷰 2) 맞추기** — 가짜 메인 스레드 줄(올린 순서대로 돌린다)과 가짜 창 포트로: `Show`(끝) 뒤 `Hide`(시한)가 줄에 올라도 마지막 상태 = 보임 · 살아 있는 세션의 `Hide` 는 숨김 · 지난 `gen` 의 `Hide` = 숨기지 않음 · 지난 `gen` 의 `Show` 는 새 세션이 그 창을 숨겨 두지 않았으면 보임(새 세션 시작이 앞질러도 옛 팝아웃이 숨은 채 남지 않음) · 끝의 `Focus` 는 같은 줄에서 그 `Show` 뒤 · 맞추기가 창 호출 때 장부 락을 쥐지 않음 · ★시한 걸음이 끝 신호의 세계 읽기와 장부 걸음 사이에 듦★(가짜 게터 안에서 `Timeout{Leave}` 를 먹인다) = 끝 결과 4c · 마지막 상태 보임 / **(리뷰 4) 붙들기 풀기의 스레드** — `dock_drag_over`(`Left` — 동기 명령)가 붙들기를 풀어 낸 `Apply` 도 가짜 spawn 포트로 가고 그 명령 안에서 적용 서비스 · 창 포트가 불리지 않음 · `OverWait` 시한도 다른 시한과 같은 spawn + sleep 한 건 / **(리뷰 2) `now` 순서** — `source_end` · `drop_at` · 시한 콜백이 시계를 세계 읽기 전에 읽음(가짜 시계 · 게터가 읽은 순서를 적는다) / **(리뷰 2) 생성물 · tsc** — `DragOverReport.ts` · `DropReport.ts` 의 `scroll: number \| null` · 새 `DrawnReport.ts` · `dockDrag.ts` 의 `Over` 생성 · 놓기 생성 두 갈래가 `scroll: null` 을 실어 `npx tsc --noEmit` 초록 · 7 단계 vitest 그대로 초록 | `commands/dock.rs` `#[cfg(test)]` · `dock/book.rs` → `lib_unit` · vitest(`npm test`) + `npx tsc --noEmit` |
| 프론트(9c · 5.7) | `TabBar` — `dragstart` 가 탭바 · 머리 전부 · `grab_x` 를 재 `strip` 으로(DOM 무변경 · 측정 실패 = `null`) / 엔진 취소 — `dragend(none)` 이 `dragstart` 사건 시각 + 15 ms 안: 시작 시한 전 = IPC 0 · 시작 뒤 = `dock_drag_end{canceled: true}` · 15 ms 이상 = 평소 · `move` 는 가르지 않음(가짜 시각 — 사건 `timeStamp` 를 꽂는다) / 모드 — 첫 `dragover` 전 = 순서 이동 · 출처 창 `pick` = `Reorder` → 순서 이동 · 그 밖 → 도킹 · 문서 밖 떠남 → `LEAVE_GRACE_MS` 뒤 도킹 · 그 안 자기 `dragend` · `dragenter` · `dragover` · `Ended` = 그대로(구역 안 Esc 에 접힘 번쩍임 0 — 리뷰 1) · 영역 전 · 막힌 영역 = 그대로(리뷰 1) · 판정 순서 = `pick` → 모드 → `displayed` → 캔버스 거름(구역 밖 첫 프레임의 `Over` = 도킹 모드의 `displayed` · 이음새 — 리뷰 1) · `reorder` 없는 영역 = 도킹 · 다른 창은 모드 없음 / 모습 — 끄는 머리 `translateX(L − x0)` · 전환 없음 · 위에 그림 · 이웃 `[t, d)` = `+w` · `(d, t]` = `−w` · 전환 150 ms · 스토어 순서 ≠ 얼린 순서 = 모습 없음(전환 없이) · 모드가 바뀌는 렌더 = 전환 끔 · 놓음 = 끄는 머리가 자리 `t` 로 · 무변경 `Ended` = 0 으로 미끄러진 뒤 전환 끔 / 떼어진 표시 · `displayed` — 도킹 모드에서만 승계 탭 · 순서 이동 모드 = 모델 활성 · `detached` 는 도킹 모드에서만 / 오버레이 — `Reorder` = 아무것도 안 그림(끄는 탭이 활성이 아닌 창의 바깥 띠 포함) / 머묾 — `Reorder` 위 = 없음 / 크롬 보고 — 모습이 선 동안 건너뜀 · 거둔 뒤 한 번 / `Over` 키에 `Reorder` 자리 — 자리가 바뀔 때만 보냄 / `pickTarget` · `reorderLeft` = 골든 / **넘친 탭바(사용자 지시 · 리뷰 1)** — `strip` 머리 = 내용 좌표(잰 x + `scrollLeft`) · `strip.scroll` · `Reorder` 판정 = `clientX + scrollLeft` · 탭바 `scroll` 사건 = 마지막 점으로 다시 판정 · 끄는 머리 화면 x = 점 − `grab_x`(스크롤 무관) · `Over` · `drop` 이 `scroll` 을 싣는다 · (끝 띠로 바꾼 경우) 띠 안 = rAF 마다 밀기 · 띠 밖 · 모드 바뀜 · 끝 = 멈춤 / **(리뷰 2) 끝 스냅숏** — ★가장자리 스크롤 중 `scroll` 사건이 새 자리 j 를 그리고 `Over{j}` 를 줄에 올린 뒤(가짜 invoke 가 그 응답을 붙든다) Esc(떠남 → `dragend(none)`)★ = `dock_drag_end.last` = 그 그린 판정(점 · `scroll` · `Reorder j` · `rev` · `bar_rev` · `displayed`) · 떠남이 `dragend.timeStamp` 전 `LEAVE_GRACE_MS` 넘게 앞섬 = `last: null` · 떠남 없음 = 마지막 그림 · 그린 것 없음 · 다른 `drag_id` = `null` · `dock_drop` 의 `point` · `scroll` = 그린 판정 것(놓는 순간 `scrollLeft` 가 달라도) / **(리뷰 2) 모든 창 탭바 = 내용 좌표** — 크롬 보고 머리 x = 잰 x + `scrollLeft` · 잰 때의 `scroll` · 탭바 `scroll` 사건 = 크롬 보고 0 · 다시 판정 1(출처 · 다른 창 둘 다) · 다른 창 `TabBar` 판정 = `clientX + scrollLeft` · 그 창 `Over` · `drop` 이 `scroll` 을 싣는다 · 끼울 자리 막대 `left` = 탭바 x0 + `insert_x` − 지금 `scrollLeft`(탭바 상자 안으로 자름) · 연속 스크롤(사건 여럿 · 커서 멈춤) 동안 막대가 판정과 같은 틈을 따라감 · 머묾 진행 표시도 같은 변환 | vitest(`npm test`) + `npx tsc --noEmit` |
| 생성물 | 새 ts-rs 타입 · 지운 `GuideView.ts` · `TabBarHint.ts` · (5.7) `StripStart.ts` · `ReorderRegion.ts` · 바뀐 `Picked.ts` · `DockRegions.ts` · (리뷰 2) 9a = 바뀐 `DockChrome.ts` · `TabBarRegion.ts` · `ReorderRegion.ts` · 9b = 바뀐 `DragOverReport.ts` · `DropReport.ts` · 새 `DrawnReport.ts` — 굽는 단계가 그 타입을 만드는 TS 자리를 같은 커밋에서 닫는다(§1-5 「선택 칸」) | CI 생성물 sync 게이트(`lib_unit` 이 굽는다) — ★지운 타입의 `.ts` 는 게이트가 못 잡으므로 6 단계 커밋에서 손으로 지운다★ |
| 화면(QA) | AC1–AC9 · AC11–AC16 · AC18–AC28 을 실제 창으로(5.7 — AC23–AC28 은 S7 과 같은 꼴을 사람 눈으로도 한 번 · 순서 이동의 미끄러짐 · 끄는 머리 겹침 · 끌기 그림 이중 표시) — `/qa full` 의 cdp 실측 + 팝아웃 X 와 끌기 겹치기(M2) · 문턱 직후 떼기 · Esc 위치별 · 트리 슬롯이 있는 창에서 끌기와 트리 재부모화(S1 ⑦) · 머묾 전환 직후 이음새 놓기(무변경) · 머묾 전환 직후 탭바 놓기(적용 — 5.2) · 최대화 탭 하나 팝아웃의 R22 재사용. 배율 둘 모니터(AC12)는 사람 손 확인이 필요할 수 있다 | `/qa full` |

- **게이트** — OS 분기 0 이라 4f · 4g · 4h 명단이 안 바뀌어야 한다(바뀌면 설계 위반 신호). 셸 → agent 운영 의존 0(4i). `cargo fmt --check` · 워크스페이스 회귀(`-- --test-threads=4`).
- **M2 전제 점검(3 단계부터 매 `/qa` · 저장소 루트)** — `rg -i "close.?requested" src/` → **0줄**(프론트 닫기 요청 리스너 없음) · `rg "prevent[_]close" src-tauri/src/` → **1줄**(`lib.rs` 의 main 갈래). `[_]` 는 처리기 주석의 자기 인용이 걸리지 않게 하려는 것이다. 어긋나면 M2 의 틈이 다시 열린 것이라 그 변경을 막고 §1-2 를 「닫는 중 표시」 안으로 고친다. **`tauri-plugin-*` 의존을 더하거나 판을 올릴 때** — 그 crate 소스에서 `prevent_close` 를 손으로 찾는다.
- **드롭 처리기 점검(6 단계부터)** — `rg "disable_drag_drop_handler" src-tauri/src/` → **1줄**(`webview_env.rs`) · `rg -i "DragDropEvent|onDragDropEvent" src/ src-tauri/src/` → **0줄**(끈 처리기의 사건을 기대하는 코드 없음).
- **다른 끌기 리스너 점검(7 단계부터 · 5.1)** — `rg "dndRootElement|dndManager" src/` → **0줄**(react-dnd 뿌리 = `window` — §1-4 불변식) · `package.json` 에 HTML5 끌기 라이브러리(react-dnd 계열 · dnd-kit · pragmatic-drag-and-drop 등)를 더하거나 react-arborist 판을 올릴 때 S1 ⑦ 을 다시 돈다.
- **시험이 못 덮는 것** — (5.7) OLE 가 멈춘 커서에 `DragEnter` 를 안 내는 동작 · transform 이 걸린 끌린 요소의 끌기 유지 · 「누른 출처 창 = 맨 위」 가정 · 미끄러짐의 눈 확인(S7 · `/qa full`) · 실제 OS 끌기(사건 순서 · 간격 · 유실) · 실제 react-arborist 와 엔진 사이의 끌기 시작 · 거절(jsdom 은 OS 끌기를 시작하지 않는다 — S1 ⑦) · 끌기 그림의 모양 · WebGL 그림 · 숨김 · 다중 모니터 · 파일 드롭 · `lib.rs` 배선은 §8-2 와 `/qa full` 몫이다. 시험은 명세한 동작의 회귀 안전망이지 완전성 보장이 아니다.

## 10. 이행 순서

★커밋마다 빌드 · 시험 초록 — 어디서 멈춰도 선다★. 자료구조를 먼저 갈아엎고 부르는 쪽을 나중에 맞추지 않는다 — 각 단계는 새 것을 **더하고** 그 단계 안에서 부르는 쪽까지 닫는다. 커밋 제목 = `S21: <타입>(<범위>): …`.

| 단계 | 내용 | 상태 · 의존 | 멈추면 남는 것 |
|---|---|---|---|
| **1** | `layout/dock.rs` 트리 연산 + 시험 | 끝(`080893c5`) | 순수 함수 |
| **2** | `ViewManager` 도킹 모델(`dock` 세 모드 · `plan_new_window` · 표식 · 묘비 · `DockGuard` · `FocusPolicy` · `DockError` · `close_tab` 도우미 · `succession`) + 시험 | 끝(`ca723ed5`) | 모델 연산 |
| **5** | `dock/hit.rs` · `dock/types.rs` · `layout/chrome.rs` + 시험 | 끝(`e73e0289`) | 순수 판정 |
| **3** | 닫히는 창(M2) — 닫기 요청 모델 먼저 · 묘비 · `apply.rs` 주석 · 닫히는 창의 늦은 Subscribe 가드 + 시험 | 끝(`281bd235` — 미커밋이던 4판 `dock/session.rs` · `pub mod session;` 은 넣지 않고 지웠다 · §11-3) | 팝아웃 닫기의 기존 틈이 닫힌 셸(도킹 없이도 값어치) |
| **4** | ★3 단계 뒤★ 메인 마지막 탭 고침(`MainLastTab` 제거 · `refill_main` 도우미를 `close_tab` 과 나눔 · 시험 갱신) → ★대상 창 활성 고침(5.1 · C1 — 2 단계 코드 변경)★ `TargetExpect::Layout` → `TargetExpect::Seam{layout, active}`(`manager.rs` 정의 · `check_dock` 의 Seam 대조 팔(`target_holds`)에 `windows[대상 창].active` · 판정 순서 주석 · 시험의 `TargetExpect::Layout` 자리 여덟 + 「활성만 바뀜 = `TargetChanged`」 새 시험) → ~~재사용 조건 고침(5.4)~~ — 5.6 에서 거둠(`plan_new_window` 의 `attrs.bounds` 조건 · 그 주석 · 시험을 걷고 탭 하나 팝아웃이면 늘 `Reuse` · `reuse_skipped` 끝 줄 칸도 걷음) → ★구현이 정한 것(5.5 · 5.6)★ 재사용 = 옮기기 → 표식 내리기 · 다시 확인 · 되돌리기 없음(Q19 — 5.3–5.5 에 넣었던 `restore_window` · `Moved` · `undo_reuse_move` · `ViewManager::reset_window_place` · 되돌리기 기록 · 시험은 걷음) · `move_window` → `Result<(), MoveFailed>` · 재사용의 안 걸치는 자리 = 제자리 · 걸침 판정 크기 = `at.size` → 모델 보통 크기 → 안쪽 크기 → 기본 · 새 창 자리 입히기 실패 = 창 거둠 · 창 포트 구현자 일곱 → ★알림 묶음 끝(5.1 · F1)★ `LayoutEvents::settled`(기본 본문 없음 — 구현자 아홉 · `TauriEvents` 는 빈 몸 · `OwnedEvents` 는 `TauriEvents` 로 넘기기 · 부르는 자리 = `notify` · `rename_tab` · 슬롯 옮기기 · `restore.rs` 알림 루프) → 적용 서비스 `apply::dock_tab`(Seam · TabBar 한 락 · NewWindow 세 phase · 재사용 = 옮기기 → 표식 내리기(Q19) · 알림 뒤 `settled`) + `WindowHost::{open_at, move_window(검사 세터)}` · `placement` 의 결과를 돌려주는 자리 입히기 짝(`move_steps`)(★기본 본문 없이 — 구현자 일곱을 같은 커밋에서 닫는다★) + Tauri 구현 · `OwnedWindowHost` 넘기기 · `placement::place` · `land` 공개 + 시험(`lib_unit` · `layout_apply`). 화살표 순서대로 커밋을 나눠도 각각 선다 | 끝(`52446b05` — 코드 리뷰 PASS(codex · doc-aware · 수명 렌즈 — 3 라운드) · 5.5 가 문구를 맞추고 5.6 이 Q19 로 재사용 되돌리기를 걷었다) | 아무도 안 부르는 적용 서비스(다음 작업의 명령도 그 위에 선다) |
| **6a** | `dock/regions.rs`(`regions_for` · `pick` · `visible_tab` · `bumps → Bumps{rev, bar_rev}` · 기대값 `Expect{layout, active, tabs}` — `seam()` · `tabs()` · `WindowSnap`) · `dock/types.rs`(`DockRegions` · `OuterBand` · `SlotRegion` · `TabBarRegion` · `Picked` · 세션 사건(`Display` 도 `Started` 와 같은 칸) · 보고(`Over` · 놓기의 `displayed` · `bar_rev`) · `DockRegions.bar_rev`(5.2) — `GuideView` · `TabBarHint` 와 그 `.ts` 를 지움) · `pick_golden.json`(맨 위 `source_view` · 7 단계 TS 거울이 읽는다) + 시험 | 끝(`b36d424f`) — 4 의 `TargetExpect` 고침 뒤(`TargetExpect` 를 쓴다) · 그 밖은 4 와 병렬 가능(겹침 = `dock/mod.rs` 한 줄) | 순수 영역 · 판정 |
| **6b** | `dock/book.rs` 장부 + 시험 — 5.1 · 5.2 규칙(§1-3 — 창별 Over · Left · 놓기 판정(탭바는 `bar_rev`) · 이른 끝 신호(버린 id 는 `ended` 로) · 적용 중 · 세션 전 스냅숏 · 옛 `rev` 다시 보냄 · 커서 못 읽음) · 6a 의 `regions::bumps`(rev · bar_rev 를 올릴지) · `Expect::seam()` · `tabs()` 를 쓴다 · `visible_tab` 은 6a 가 `regions.rs` 에 새로 짰으므로 옮기지 않는다 · 4판 `session.rs`(지움)에서는 끝 어휘(`Outcome` · `EndReason`) · 띠 캐시 꼴만 참고한다(`git` 이력에 없다 — 사본 `.claude/handoff/attachments/slot-docking-session-rs-replaced-2026-10-16.rs.txt`) | 끝(`961b7c26`) — 6a 뒤 · 4 와 병렬 가능 | 순수 장부 |
| **6c** | 크롬 측정(`WindowTabs.chrome` · `set_dock_chrome` · `state/convert.rs` 구조 분해에 `chrome: _` — 저장 안 함) · `commands/dock.rs`(invoke 여섯 + `report_dock_chrome` — 인자는 구조체(예: `DropReport`)로 받거나 `rename_all = "snake_case"`(Tauri v2 의 기본 camelCase · 이 저장소에 `rename_all` 없음 — §1-5) · `DragEffect`(move · none · other) 정의 · 스냅숏 빌더가 `WindowSnap.visible` 을 `regions::visible_tab` 으로 채움(어긋나면 debug 셸은 `regions_for` 의 `debug_assert_eq!` 가 패닉 훅으로 끝낸다 · release 는 검사 없음) · 실행기 · 시한 셋 · 게터는 락 밖 · 세션 깃발 = 세대(시작의 레이아웃 락 안에서 세우고 비교-교체로 내림) · `Apply` 는 늘 spawn · `Refused` · `LateStart` · 계측) · ★6 단계 의무(5.5)★ `dock_tab` 을 메인 스레드 · 동기 명령에서 부르지 않는다(Create 길이 창을 만들어 교착 — §1-3 불변식) · 숨긴 출처의 되보이기는 `apply_deferred_maximize` 를 지나는 보이기 경로로(5.6 — 되돌리기는 없어졌지만 미룬 최대화가 선 창을 같은 길로 보인다) · `lib.rs`(`generate_handler!` · `Destroyed` → 장부) · `TauriEvents::settled` 계기 · 측정 보고 뒤 계기 · `WebviewEnv::finish` 의 `disable_drag_drop_handler()` + 시험 | 끝(`d582331a`) — 4 · 6b 뒤(`manager.rs` · `commands/layout.rs` 겹침) | 웹뷰가 안 부르는 셸 끌기 처리(무해 — 끈 처리기의 사건은 쓰는 곳 0) |
| — | **실측 S1–S4 · S6**(§8-2) — 결과를 이 문서 개정으로. 불합격이면 멈추고 사용자에게 | — | — |
| **7** | 프론트 — `TabBar` 끌기 출처(우리 MIME `stopPropagation` · 디스패치 뒤 시작) · `dockDrag.ts`(대상 · 세션 몰라도 받기 · 끊기 · `drag_id` 별 최대 `rev` · `seq` · 보이던 탭 대조 · 세션 구독 · 유실(버튼 0) · Esc 삼키기(창별 상한) · 파일 막기) · `dockPick.ts` + 골든 vitest · `dragImage.ts` · `DockOverlay.tsx` · `dockChromeReport.ts` · `WindowLayout` 떼어진 표시 + 시험 · 커밋 전 S1 ⑦ · ⑧ — **마우스 도킹이 끝까지 된다** | 구현(미커밋) · 실 마우스 실측(2026-10-17 · §8-1) — S3 · S6 · 트리 합격 · S1 ⑧ 결함 셋. ★5.7 — 지금 트리를 그대로 커밋한다(그 단계 게이트 · 결함 셋은 커밋 본문에 알려진 결함으로 · 게이트 전이면 로컬 스냅숏 커밋으로라도 먼저 남긴다 — 9a 가 그 파일을 고치므로 되돌릴 지점이 있어야 한다)★ | 마우스 도킹(결함 셋 남음) |
| **9a** | (5.7) 순수 규칙 · 영역 · wire — `hit.rs` `REORDER_SLACK_PX` · `reorder_zone` · `reorder_left` · `reorder_index` · `types.rs` `StripStart` · `ReorderRegion` · `Picked::Reorder` · `DockRegions.reorder` · `regions.rs` `regions_for(.., strip)`(출처 창 조건 ①–⑥ · §2-4) · `pick` 의 첫 단계 · `bumps` · `same_body` · `pick_golden.json` 사례 · `left` 칸 + 시험 · **같은 커밋에서 닫을 부르는 쪽** — 장부의 `regions_for` 호출에 `None`(아직 `strip` 이 없다) · `dockPick.ts` 거울(`reorder` 구역 → 탭바 → 캔버스 · `reorderLeft` · `reorderIndex`) + 골든 vitest · `dockDrag.ts` `pickedKey` 의 `reorder` 팔(tsc 가 짚는다) · TS 시험 고정물의 `DockRegions` 에 `reorder: null` · 생성물(`StripStart.ts` · `ReorderRegion.ts` · `Picked.ts` · `DockRegions.ts`) · (리뷰 1) `pick` 의 `scroll` 인자(부르는 쪽 = 장부 · 시험 — 지금은 0) · `StripStart.scroll` · 골든 사례의 `scroll` · `source_overflow` · (리뷰 2) `pick` 의 `scroll` = `Option<f64>`(장부는 보고의 칸 그대로 · 없으면 `None`) · 탭바 단계(`index` · `over`)도 내용 x · `DockChrome.scroll` · `TabBarRegion.scroll` · `ReorderRegion.scroll` · `bumps` 가 `tab_bar.scroll` 을 견주지 않음 · 골든 `target_overflow` · TS 거울 탭바 단계 · `dockChromeOf` 에 `scroll: null`(tsc — 7 단계 측정은 그대로 뷰포트라 판정 무변경) · `dockDrag.ts` `evaluate` 는 아직 스크롤을 주지 않는다(`null` — 셸 다시 판정의 `None` 과 같다 · 0 을 주면 영역 스크롤이 0 이 아닌 넘친 탭바에서 그린 자리 ≠ 셸 자리 — 9a 코드 리뷰) | 7 커밋 뒤 | 쓰는 이 없는 순수 규칙(동작은 5.6 그대로 — `reorder` 가 늘 `None`) |
| **9b** | (5.7) 셸 — `book.rs`(`Input::Start.strip` · `SourceEnd.canceled` · `Drag.strip` · `hide_pending` · 도킹 모드 증거 → 미룬 `Hide` · 놓기 판정 4c · 빈 보고 · 커서 판정(W · 출처 · 뷰포트 점 변환 · `rect_check` 일 때만) · 붙든 이른 놓기 · 엔진 취소 끝 · `EndReason::EngineCanceled` · `EndBy` · `Mode::Reorder` · 끝 줄 `unseen` · `strip` · `hide_deferred`) · `commands/dock.rs`(`dock_drag_start` 의 `strip: Option<StripStart>` · `dock_drag_end` 의 `canceled: Option<bool>` — 둘 다 선택 인자 · (리뷰 1) 보고 · 놓기의 `scroll: Option<f64>`) · (리뷰 1) 미룬 숨기기 증거(그린 보고 · `ArmTimer{Leave}` · 다른 창) · 커서 판정의 캔버스 거름(보고 `displayed` · 없으면 모델 활성) · 「빈 보고」 좁힘 · 계측 + 시험. ★§11-1b ① 의 갈래는 `Verdict` 한 팔에 둔다(= 적용 · 사용자 결정 Q21 — 확정)★ · (리뷰 2) `dock_drag_end` 의 `last: Option<DrawnReport>`(선택 인자 · 도착 순번으로 고른 W 가 출처일 때만 장부가 마지막 `Over` 대신 씀 — 리뷰 3) · (리뷰 3) `last.left_before_end_ms` 와 `CANCEL_LEAVE_MS` 갈래(사각형 검사 건너뜀) · 실행기 `dock_drag_start` 가 출처 사각형을 읽어 `Input::Start.source_rect` 로 · `Drag.source_rect`(출처가 든 `Rects` 마다 갈아 끼움) · 시한이 앞지른 끝의 사각형 검사 · 세션 번호 `gen`(시한 · `Hide` · `Show`) · 숨기기 · 보이기 · 그 뒤 `Focus` 를 메인 스레드 맞추기로(`Book::visibility`) · 끝 신호 · 시한 콜백의 `now` 를 세계 읽기 전에 · 시한이 앞지른 끝 · 숨긴 출처 `sent` 남김 · (리뷰 4) 오는 중 `Over` 붙들기(`AwaitOver` · `OverWait` · `OVER_WAIT_MS` · 풀기 셋 · 끝 줄 `over_wait` · `over_wait_ms` · `over_from`) · W 없음 + `last` 로 W = 출처(`lifted`) · ★생성물을 다시 굽고(`DragOverReport.ts` · `DropReport.ts` · `DrawnReport.ts`) `dockDrag.ts` 의 `Over` 생성 · 놓기 생성 두 갈래에 `scroll: null` 을 같은 커밋에서 싣는다 — `npx tsc --noEmit` 초록 · 7 단계 동작 무변경★(§1-5 「선택 칸」 — 9b 는 프론트 무변경이 아니다) | 9a 뒤 | 7 단계 웹뷰 그대로에서 결함 ① ③ 이 고쳐진 셸(빈 보고 · 보고 없는 끝 = 커서 판정 — 원래 창 탭바는 아직 끼울 자리라 그 갈래는 §11-1b ① 의 적용(Q21)을 탄다) · 엔진 취소는 웹뷰가 아직 안 보내 5.6 그대로 · (리뷰 2) `last` 도 아직 안 와 마지막 `Over` 그대로 · 숨기기 · 보이기 맞추기는 켜진다(시작 때 숨김 — 5.6 — 에도 같은 길) |
| **9c** | (5.7) 프론트 — `TabBar.tsx`(탭바 재기 → `strip` · 엔진 취소(사건 시각 · `canceled`) · 순서 이동 모습 §4-4 · `reorder` 받는 인자) · `dock/dockDrag.ts`(모드 · `displayedTabOf` 에 모드 · `useDockMode` 류 훅 · `Over` 키) · `WindowLayout.tsx`(`detached` 는 도킹 모드에서만 · 크롬 보고 건너뜀 · 모습 입력) · `dock/DockOverlay.tsx`(`Reorder` = 그리지 않음) · (리뷰 1) 판정 순서 `pick` → 모드 → `displayed` → 캔버스 거름 · 떠남 미룸 · 막힌 영역 = 모드 유지 · 넘친 탭바(내용 좌표 · 탭바 `scroll` 다시 판정 · 보고 `scroll` · 엔진 자동 스크롤 — S7 ⑬ 이 안 되면 끝 띠) · (리뷰 2) 끝 스냅숏(`dockDrag.ts` 가 그린 판정과 떠남 사건 시각을 끝까지 쥐고 `TabBar.tsx` `onHeadDragEnd` 가 `last` 로 싣는다 · (리뷰 3) 떠남 → `dragend` 사건 시각 간격 `left_before_end_ms` · `dock_drop` 의 `scroll` = 그린 판정 것) · 모든 창 탭바 내용 좌표(`WindowLayout.tsx` `measureDockChrome` 이 잰 x + `scrollLeft` · `scroll` · `onScroll={scheduleChrome}` 을 거두고 다시 판정 계기로 · `evaluate` 가 그 창 `scrollLeft` 로 `pickTarget` · `DockOverlay.tsx` 막대 = x − 지금 스크롤) + 시험 · 커밋 전 **S7**(§8-2) | 9b 뒤(웹뷰가 `strip` 을 보내기 시작하면 셸 4c · 미룬 숨기기가 켜진다 — 9b 없이 9c 만 들어가면 `strip` 이 버려질 뿐 5.6 그대로) | 사용자 결정 Q20 의 순서 이동 · 결함 셋 고침이 다 된 마우스 도킹 |
| — | **실측 S7 · S1 ⑧ 다시**(§8-2 — 5.7 합격 기준) — 결과를 이 문서 개정으로. 불합격이면 멈추고 사용자에게 | 9c 작업 트리 | — |
| **8** | 문서 — ADR(§12) · step-log · CLAUDE.md 「LLM-우선 제어」 남은 갭 한 줄 · `docs/process/S21-slot-docking/structure.html` 갱신(`/review doc`) · `/qa full` 화면 실측(AC 전부 · S5) | — | — |

- 매 단계 게이트 = `/implement`(구현 → `/review code` → `/qa`) — CLAUDE.md 「구현 실행 규약」.
- **나누는 축 = 파일 겹침** — 3 → 4 → 6c 는 `layout/*` · `commands/*` · `lib.rs` 가 겹쳐 순서대로. 6a · 6b 는 `dock/*` 만이라 4 와 병렬(단 6a 는 4 의 `TargetExpect` 고침을 쓰므로 그 커밋 뒤). 7 은 프론트만이라 6c 와 접점(§1-5 wire)을 먼저 못 박으면 병렬도 된다 — 단 실측 결과를 기다린다.
- ★3 → 4 순서는 바꾸지 않는다★ — 적용 서비스는 닫히는 창을 3 단계의 모델 먼저 지우기 · 묘비에만 기대어 막는다(사전 · 사후 `is_open` 확인 없음). §11-1 #2 의 답 = M2 확정(2026-10-16)이라 2판의 두 검사(Seam · TabBar 의 `is_open` 사전 확인 · NewWindow 의 확정 뒤 확인과 정리)는 되살리지 않는다.
- 7 단계 뒤가 PRD 의 기본 흐름(팝아웃의 탭을 끌어 붙이기 — PRD §1) 완성이다. 다음 작업(LLM 표면)은 4 단계만 있으면 시작할 수 있다.
- **(5.7) 9a → 9b → 9c 순서 · 어디서 멈춰도 선다** — 9a 는 새 것을 더하기만 하고(`reorder` 는 늘 `None` 이라 판정 결과가 5.6 과 같다) 새 `Picked` variant 를 부르는 쪽(TS 거울 · `pickedKey` · 시험 고정물)을 같은 커밋에서 닫는다. 9b 의 wire 인자는 선택이라 7 단계 웹뷰와 선다 — (리뷰 2) 단 9b 가 굽는 TS 타입의 `scroll: number | null` 은 키가 필수라 9b 가 `dockDrag.ts` 의 보고 생성 두 곳에 `scroll: null` 을 같은 커밋에서 싣는다(9a 의 `DockChrome.scroll` → `dockChromeOf` 도 같은 꼴). 9c 는 프론트만이다. ★자료구조를 먼저 갈아엎지 않는다★ — 기존 `Picked::TabBar` · `tab_bar` · 5.6 의 원래 창 끼울 자리 갈래는 지우지 않고 `reorder` 가 없을 때의 물러날 자리로 남긴다(§2-4). **나누는 축 = 파일 겹침** — 9a(`dock/hit` · `regions` · `types` · 골든 · `dockPick.ts` · (리뷰 2) `dockChromeReport.ts` 한 칸) · 9b(`dock/book` · `commands/dock` · (리뷰 2) 생성물 · `dockDrag.ts` 보고 생성 두 곳) · 9c(`TabBar` · `WindowLayout` · `dockDrag` · `DockOverlay`) — 9a 와 9b 는 `book.rs` 의 `regions_for` 호출 한 줄이 겹쳐 순서대로. ★(리뷰 2) 9b 와 9c 도 `dockDrag.ts` 가 겹쳐 순서대로다★ — 5.7 첫 판의 「9c 는 9b 와 병렬도 된다」는 9b 가 프론트를 안 건드린다는 전제였고 그 전제가 깨졌다(위).
- **(5.7) §11-1b 의 답(2026-10-17)** — ① = 적용(사용자 결정 Q21)이라 9b 의 `Verdict` 팔은 잠정이 아니라 확정이고 바꿀 것이 없다. ② = 받아들임(Q20 에서 따라 나온 것)이라 PRD R23 · R6b · AC14 개정도 없다 — 9c 는 그대로.

## 11. 위험 · 미결

### 11-1. 사용자 확인 대상 — 사용자 결정 2026-10-16 · 8건 전부 기본값 확정

PRD 4.4 Q11–Q18 이 같은 결정이다. 아래 「다른 안」은 거부한 대안으로 남긴다.

1. **같은 창 안 도킹에서 출처가 보이던 탭이면 무엇이 보이나** — 기본값 = **대상이 보이게 된다**(이음새 = 대상 탭 · 탭바 = 옮긴 그 탭) — 보던 내용이 그 탭에 들어가 화면에 남는다. 다른 안 = 탭 닫기와 같은 승계(오른쪽 · 왼쪽 이웃). → **확정**.
2. **닫히는 창과 모델 변형의 조정 = M2**(§1-2 「닫히는 창」 — 3 단계에 구현) — 팝아웃 닫기 요청에서 모델을 먼저 지우고 묘비. 대가 = 팝아웃 모델 정리의 계기가 소멸 → 닫기 요청으로 옮긴다(ADR-0057 불변식 그대로 · ADR-E). 다른 안 = 열림 사전 확인 + 남는 위험 기록(몇 ms 겹침에 붙인 탭이 그 창과 함께 사라짐). → **확정** — 따로 예외 처리 없이 흐름에 맡긴다: 도킹 확정이 닫기보다 먼저면 붙은 탭은 그 창과 함께 닫히고(기존 팝아웃 닫기 규칙), 닫기가 먼저면 대상 없음으로 무변경(탭은 출처). 「닫기가 먼저면 그 자리에 새 창」은 채택하지 않았다 · 토스트 · 툴팁도 없다.
3. **R22 새 창 크기 — 출처 창이 최대화였을 때** — 기본값 = 그 창의 보통(최대화 전) 크기로 연다. 다른 안 = 최대화 상태 그대로. → **확정**(사용자가 Chrome 도 최대화 창에서 뗀 탭을 보통 크기로 연다는 것을 확인).
4. **R6b 로 바꾼 탭은 끌기가 무변경으로 끝나도 그대로 둔다** — 기본값 = 되돌리지 않음(브라우저 · VS Code). 다른 안 = 끌기 시작 때 탭으로 되돌림. → **확정**(관례대로).
5. **칠하지 않은(면적 0) 바깥 띠 위에 놓으면** — 기본값 = 밑 슬롯 띠로 넘김(그 칸 띠에 들면 그 이음새, 안 들면 R22) 다른 안 = 무효 바깥 띠 = 새 창. → **확정** — PRD R8 을 맞췄다(4.4). 면적 0 은 정규화 좌표 계산이라 극단적 중첩에서만 생긴다 — px 로 작은 칸(예: 50px)은 막히지 않는다.
6. **같은 자리 탭바 놓기 + 마우스 = 그 탭을 보인다** — 안 보이던 탭을 끌어 자기 창 탭바의 원래 자리에 놓으면 기본값 = 그 탭을 보인다(`version` +1). 다른 안 = 진짜 무변경(리뷰어 대립이던 것). → **확정**(사용자 결정).
7. **끌기 그림 내용(PRD Q5 「되는 만큼」의 범위)** — 기본값 = 끄는 탭이 보이는 탭이면 **터미널 칸 = 그 화면의 축소 그림 · DOM 칸(챗 · 목록 · 사용량 · 빈 칸) = 칸 이름 자리채움**(DOM 은 끌기 시작 순간 동기로 그림에 옮길 수 없다) · **안 보이던 탭 = 제목만**(그 칸들은 화면에 없다). 다른 안 = 누르는 순간 그 탭으로 전환해 띄운 뒤 그리기(탭 클릭이 「누를 때 전환」으로 바뀌고, 끌지 않은 누름도 탭을 바꾼다 — 브라우저 · VS Code 는 누를 때 전환). → **확정**.
8. **「보인 것과 다름」 · 출처 · 대상 바뀜의 끝 = 무변경**(R13c · M1) — 결정 Q8 의 「원래 창 되돌리기 없음」은 **비정상 끝**에 대한 것으로 읽었다. 이 무변경은 못 본 결과를 만들지 않는 안전장치라 남겼고, 결과로 탭은 원래 창에 다시 보인다. 다른 안 = 이 경우에도 그 자리에 새 창(R22) — 못 본 이음새에 붙지는 않으면서 「되돌림」이 사라진다. → **확정** — 따로 처리하지 않는다: 붙일 슬롯이 없으면 도킹이 그냥 실패하고 아무것도 안 바뀐다. 「그 자리에 새 창」 · 「툴팁 표시」는 채택하지 않았다.

**PRD 와 다른 점**
- R21 「원래 창 탭바에 놓으면 그 창의 탭으로 돌아간다」는 놓은 자리로 옮겨 넣으므로 탭 순서 바꾸기도 함께 된다. (5.7 — PRD 4.6 이 그것을 R23 순서 이동으로 따로 세웠다. `reorder` 가 없는 끌기는 이 5.6 갈래로 물러난다 — §2-4.)

### 11-1b. 사용자 확인 대상 — 5.7 (PRD §5-3 · 2026-10-17 닫힘)

둘 다 닫혔다 — ① 은 사용자 결정 Q21, ② 는 Q20 에서 따라 나온 것(새 질문 아님). 구현(§10 9b · 9c)에 바꿀 갈래가 남지 않는다.

1. **① 보고 없는 끝 · 빈 보고의 커서 자리가 이음새 · 끼울 자리일 때**(§1-3 「커서 판정」) — 멈춘 채 떼기 · 짧은 톡 · 영역이 닿기 전 떼기라 사용자는 그 자리의 가이드를 보지 못했다(그 창이 `dragover` 를 못 받았거나 영역 없이 받았다). 대개 탭을 잡고 문턱만 넘게 옮긴 뒤 곧바로, 또는 잠깐 멈췄다 뗀 것이다.
   - **답 = 그 자리에 적용(사용자 결정 Q21 · 2026-10-17 · 채팅)** — 판정할 창은 좁힌 규칙 그대로(그 끌기를 마지막으로 본 창 W, 없으면 출처 — D21). 구역 = 놓은 자리 순서 이동(대개 무변경) · 이음새(가이드 자리) · 끼울 자리 = 거기 적용 · 칸 가운데 · 그 밖 = R22 새 창. 근거(사용자) = 창 안으로 옮겨 온 것 자체가 마우스 움직임이므로 뗀 순간 커서가 있는 곳이 목적지다. 이웃 창 `rev` 0 끝(실측 6/20 무변경)도 이 갈래로 붙는다. 이것으로 결함 ①(멈춘 채 떼기 · 짧은 톡 → 새 창) · ③(`rev` 0 보고 → 무변경)이 닫힌다(§8-1 · S7 ⑥ ⑨ ⑪).
   - **고르지 않은 안** = 무변경(`TargetChanged`) — Q18 「보인 것과 다름 = 무변경」 · M1(못 본 가이드에 놓지 않는다)을 이 경우에도 지키는 안.
   - ★**범위 = 보고 없는 끝뿐 — R13c 는 그대로다**★ — 가이드를 보인 보고가 있고 놓은 자리의 판정이 그 가이드와 다르면(또는 「가이드 없음」을 보인 자리가 지금 가이드면) 「놓기 판정」 3 · 4 그대로 무변경이다(PRD R13c 「보지 못한 결과를 만들지 않는다」 · Q18).
2. **② 원래 창 탭바의 머묾 탭 전환이 없어짐**(PRD R6b · R15 · AC14 · D4) — **답 = 받아들임(Q20 「탭 영역에서 움직이면 탭 간 이동만」에서 따라 나온 것)**. 순서 이동 중 커서 밑은 늘 끄는 머리라 「다른 탭 머리 위 머묾」이 성립하지 않는다. 같은 창 다른 탭에 합치려면 그 탭을 먼저 띄워 둔 채 다른 탭을 끌어 캔버스로 내린다(PRD AC14 4.6 문구). ★다른 창 탭바의 머묾 전환은 그대로다★(§1-3 「R6b 탭 전환」 · §1-4 「머묾」).

### 11-2. 기술 위험

| # | 위험 | 확신 · 대응 |
|---|---|---|
| 1 | 「마지막 점에 놓기」의 오판 — 우리 창을 덮은 다른 앱 창 위로 나가 `LEAVE_GRACE_MS` 안에 떼면 그 밑 우리 창의 마지막 가이드로 붙는다 · Esc 취소의 떠남이 시한보다 늦게 앞서 오면 창 안 Esc 가 새 창이 된다 · 가이드 위 Esc 뒤 셸이 커서를 읽기 전에 커서가 그 창 밖으로 나가면 새 창((리뷰 3) 출처 창 위 Esc 는 `last` 의 떠남 간격이 닫는다 — 다른 창은 그대로 · (리뷰 4) 오는 중 `Over` 붙들기는 끝 때 읽은 커서로 판정해 이 틈을 넓히지 않는다) | 가능성 낮음 · 사용자는 그 가이드를 직전에 봤다. S1 ① ② 가 간격 분포를 재고 시한을 고친다 · 끝 줄 `leave_gap_ms` 로 현장에서도 본다 |
| 2 | 끝 신호 유실 뒤 마우스를 우리 창에 안 가져오면 장부 · 표식이 남는다 — 그 탭의 떼어진 표시가 남고 다음 끌기가 거절되며, 키보드만 쓰는 창은 Esc 가 먹힌다 | 가능성 낮음(엔진의 `dragend` 는 실측 모든 경우) · 마우스가 우리 창에 오는 순간 `pointer_seen` 이 끝내고 그 창의 Esc 삼키기는 200ms 뒤 그친다(5.1 · §1-4) |
| 3 | 새 시작이 옛 세션을 만나면 새 끌기는 거절된다 · 옛 세션이 적용 중(새 창 열기 수백 ms)이어도 거절된다 · 옛 세션의 끝과 그 표식 내리기 사이의 새 시작은 `Dragging` 으로 거절된다 | 가능성 낮음(유실 + 300ms 안 새 누름 · 놓은 직후 곧바로 다시 끌기 · 마지막 것은 ms 아래의 틈 — 받아들인 남은 틈 · 5.2) · 사용자가 다시 끈다 |
| 4 | TS 거울과 Rust 판정이 갈림 | 골든이 막는다 · 갈라지면 놓기가 무변경으로 떨어진다(엉뚱한 도킹 아님) |
| 5 | 끌기 그림 — 반투명 · 다른 앱 위 사라짐 · 터미널 그림이 비어 보일 수 있음 · DOM 칸은 자리채움 | 확실(앞 둘 — 받아들임) · S2 가 WebGL 을 잰다 · §11-1 #7 |
| 6 | 파일 드롭으로 웹뷰가 탐색 | 불확실 · 막기 리스너(§1-4) · S3 ② · main 은 지금도 처리기가 꺼져 있다 |
| 7 | 숨긴 출처의 끌기(표본 2) · 숨김이 활성 창을 다른 앱으로 넘김 | 가능성 높음(무해 — 끌기는 전경과 무관하게 엔진이 몬다 · 놓은 뒤 `set_focus`) · S1 ⑤ |
| 8 | 도킹 직후 닫히는 팝아웃 웹뷰가 탭 0 개 순간을 그림 | 가능성 높음 · 무해 — 모델에서 창을 먼저 지운 뒤 OS 창을 닫는 순서는 `close_tab` 과 같다 · 탭 하나 팝아웃은 이미 숨어 있다 |
| 9 | 영역 다시 계산 비용(계기마다 모든 창 · 칸당 `docked_tree` 넷) | 가능성 낮음 · 끄는 동안만 · 바뀐 창만 보낸다 · 트리가 작다 |
| 10 | 떼어진 표시 직후 출처 창 탭바의 끼울 자리 막대가 접히기 전 기하로 머리 하나 폭만큼 어긋나 보임 | 확실(짧은 틈 — 보고 디바운스 + 왕복) · 무해 — 그림과 판정이 같은 영역이라 보인 자리로 들어간다 · 거슬리면 떼어진 표시 직후의 크롬 보고만 디바운스 없이 · (5.7) 출처 창에 `reorder` 가 서면 그 창 탭바는 구역이라 끼울 자리 막대가 없다 — `reorder` 가 빠진 끌기에만 남는다 |
| 11 | 같은 웹뷰 안 도킹에서 칸 컴포넌트가 다시 마운트되며 터미널이 재구독 | 확실(구조) · 팝아웃과 같은 수준(D10). 출력은 replay 로 이어진다 |
| 12 | 끌기 중 앱 종료 · 셸 패닉 | 셸 패닉은 훅이 곧 종료다(ADR-0292) — 모델 · 저장은 도킹 전 판이라 탭은 한 곳이다(§5). 장부 · 실행기 안에서는 패닉하지 않게 짠다. 표식 · 묘비 · 장부는 메모리라 다음 부팅에 없다 |
| 13 | R22 새 창이 웹뷰를 띄우는 동안(수백 ms 추정) 화면에 아무것도 없음 | 가능성 높음 · 탭 하나 팝아웃은 재사용이라 해당 없음. 거슬리면 QA 에서 판단 |
| 14 | 닫기 요청 계기의 모델 정리가 기존 팝아웃 닫기를 바꿈 · X 직후 먼저 확정된 도킹의 탭이 그 창과 함께 닫힘 | 가능성 낮음 · 확실(닫기 의미 그대로 · 잔해 없음) — 팝아웃 닫기를 막을 수 있게 되는 날 근거가 무너진다(§9 점검) |
| 15 | LLM 이 도킹 · 레이아웃 조회를 못 한다(의식적 예외) | 확실 · 사용자 결정 · 다음 작업이 닫는다 — 그 사이 CLAUDE.md 「LLM-우선 제어」 남은 갭에 적는다(8 단계) |
| 16 | 문턱 직후 캔버스에서 떼면 새 창(또는 보이던 가이드에 붙음 · 영역이 아직 안 닿았는데 그 점이 가이드 자리면 5.1–5.6 은 무변경 · 5.7 은 빈 보고라 커서 판정으로 그 자리에 적용 — Q21) | 확실(PRD R22 · 가이드 그대로) · 엔진 문턱이라 값을 고치지 못한다 — 5.1 은 거슬리면 「출처 창 안 · 시작 뒤 짧은 시간의 가이드 없음 놓기 = 무변경」을 사용자에게 올리려 했으나, 사용자 결정 Q21(뗀 순간 커서 자리 = 목적지)이 이 경우도 덮으므로 올리지 않는다 · (5.7) 탭 머리에서 문턱 직후 떼면 구역 안이라 그 자리 순서 이동(대개 제자리)이다 — 이 위험은 구역 밖(캔버스)까지 내려 곧바로 뗀 경우만 남는다 · 보고 없는 끝은 커서 판정(§1-3) |
| 17 | `adopt_restored` 가 묘비를 보지 않아 닫힌 팝아웃 label 을 들임 | 확실(구조) · 무해 — 복원 ④ 의 「사라진 창」 확인 · 뒤이은 `Destroyed` 가 지운다 |
| 18 | macOS(WKWebView)의 창 사이 끌어 놓기 | 모름 · 이식 때 S1 · S2 · S4 |
| 19 | react-arborist(react-dnd HTML5 백엔드)가 window 에 단 끌기 리스너 — 트리 슬롯이 마운트된 창(안 보이는 탭 포함)에서 우리 `dragstart` 를 취소하고(`HTML5BackendImpl.js:212`) `dragover` 의 `dropEffect` 를 none 으로 덮는다(:284-292) — 끊지 않으면 그 창에서 탭 끌기가 시작되지 않고 그 창에 놓을 수 없다 | 확실(소스 · 5.1) · 우리 MIME 사건을 `document` · 탭 머리에서 끊는다 · 세션은 취소 안 된 시작만 · S1 ⑦ · 뿌리를 `document` 로 옮기거나 끌기 라이브러리를 더하면 다시 잰다(§9 점검 줄) |
| 20 | 사건 순서 — emit · invoke 가 여러 스레드라 영역 `rev` 가 뒤바뀌어 닿고, 끝 신호가 시작보다 먼저 닿을 수 있다 | 가능성 낮음(빠른 끌기) · 웹뷰 최대 `rev` · `seq` · 옛 `rev` 보고에 다시 보냄 · 끝 신호 500ms 붙듦 · S1 ⑧ · 끝 줄 `regions_resent` · `early` |
| 21 | 거절된 끌기(옛 세션 · 탭 없음 · 락 오염 · 늦은 시작)도 다른 창이 받아 커서가 「이동」으로 보인다 — 놓아도 받은 쪽은 아무것도 바꾸지 않는다(모르는 id). 다른 앱 인스턴스(다른 데이터 폴더의 Engram)에서 끈 탭을 우리 창에 놓으면 우리 쪽은 그대로이고, 출처 인스턴스는 `move` 끝을 받아 `DROP_WAIT` 뒤 「마지막 점에 놓기」로 커서 자리에 새 창을 띄운다 — 그쪽에서 우리 창은 다른 앱 창이라 PRD R22 그대로다(5.1 전에는 우리가 거절해 `none` 끝 → 같은 새 창) | 확실(세션을 몰라도 받는 대가 · 5.1 · 문구 5.2) · 드묾 · 무해 |
| 22 | 보이는 탭이 바뀌는 순간(R6b 머묾 · 클릭)의 이음새 놓기는 무변경 | 확실(짧은 틈 — 알림 · 영역 왕복) · 사용자가 다시 끈다 · 탭바 놓기는 `bar_rev` 로 따로 대조해 그대로 적용된다(5.2) |
| 23 | 동기 명령 안의 창 만들기 교착 — 시작(메인 스레드 · 동기)이 붙든 이른 놓기를 이어 먹이면 그 판정의 `Apply`(R22 새 창)가 동기 명령 안에서 돈다 | 확실(구조 — 막지 않으면) · 가능성 낮음(이른 놓기 + 새 창 판정) · `Apply` 는 늘 spawn(§1-3 불변식 · 5.2) · 실행기 시험이 spawn 포트로 잰다 |
| 24 | R22 재사용 — 놓은 뒤 창을 옮기기까지의 사이(ms)에 다른 경로(LLM)가 출처를 바꿈(슬롯 · 탭 더함 · 닫힘) · 최대화였던 출처의 깜빡임 | 가능성 낮음 · ★다시 확인 · 되돌리기 없이 결과 그대로 — 사용자 결정 Q19(5.6)★: 배치 바뀜 = 바뀐 채 옮겨짐 · 탭 닫힘 = 팝아웃은 기존 규칙대로 닫힘 · 탭 하나 더 = 둘이 함께 옮겨짐(받아들임) · 재사용은 모델을 바꾸지 않아 모델과 화면은 어긋나지 않는다 · 옮기기 실패(운영 = 종료 중뿐 — 워커 스레드의 세터는 기다리지 않아 OS 실패가 안 보인다)는 `WindowOpen` 이고 되돌리지 않는다 · 최대화 아니던 출처는 숨어 있어 옮기기가 안 보인다 · 최대화였던 출처는 B′ 의 최대화 풀기에서 잠깐 깜빡였다 다시 숨는다(tao `apply_diff` — `SW_RESTORE` 뒤 `SW_HIDE` · 소스 · GUI 미검 · 받아들인 경우 · S1 ⑤) · ★미결 — 깜빡임 없이 최대화를 푸는 길은 platform 목적 함수(Win32 `SetWindowPlacement` 류)가 있어야 하고 이번엔 넣지 않는다★(거슬리면 사용자 보고 뒤 ADR-0266 절차로) |
| 25 | Create 길 — 새 창을 만든 뒤 자리 입히기가 실패하고 그 창 거두기도 실패하면 모델에 없는 창이 남는다 | 이벤트 루프가 없을 때(앱 종료 중)뿐 · 받아들임(5.6) — 오류 문장에 거두기 실패 사유를 싣는다 · 종료가 그 창을 함께 없앤다 |
| 26 | ~~「재사용은 보통 자리가 있을 때만」(4 단계 고침 셋 · 5.4)의 남길지~~ | ★해결(메인 결정 2026-10-16)★ — 거뒀다. 근거였던 되돌리기가 Q19 로 없어졌고, 탭 하나 팝아웃은 늘 그 창을 옮긴다(웹뷰를 새로 띄우지 않음 · 모델 변경 없음 · 걸침 판정 크기는 지금 안쪽 크기로 물러난다) |
| 27 | (5.7) 끄는 중 끌린 머리를 transform 으로 옮기면 끌기가 끊김 | 가능성 낮음 — 접어도 306/307 이어졌고(S1 ④) Chrome 의 웹 탭바도 끄는 중 탭 요소를 옮긴다(조사) · 우리 꼴은 미검 · S7 ② · 끊기면 멈춤(복제 요소를 옮기는 안 — 사용자 보고) |
| 28 | (5.7) 커서 판정의 「누른 출처 창 = 그 사각형 안 맨 위」 가정 | 가능성 높음(누르면 그 창이 앞으로 온다 — 실측 2026-10-17 S6 덧붙임 · 하네스가 그것 때문에 이웃 창을 맨 위 고정으로 둬야 했다) · 항상 위에 뜨는 다른 앱 창이 출처를 덮은 자리에서 멈춘 채 떼면 그 밑 출처 판정(순서 이동 · 이음새)이 된다 — 드묾 · 받아들임 · 닫으려면 platform 목적 함수(커서 밑 맨 위 창 — ADR-0266 · 사용자 보고) |
| 29 | (5.7) 이웃 창 위에서 멈춘 채 뗀 끌기(그 창 보고 0)는 5.6 그대로 새 창 | 확실(설계 — 커서 판정을 W · 출처로 좁힌 대가 · D21) · 드묾(창 둘이 문턱 거리 안에 겹친 자리) · 실측 4/4 는 하네스가 이웃 창을 맨 위 고정(`HWND_TOPMOST`)으로 둔 경우였다 — 그 하네스에서 떼는 점이 출처 사각형과도 겹치면 5.7 은 출처로 판정한다(맨 위가 이웃이니 오판 · 제품은 우리 창을 맨 위 고정으로 두지 않는다 — S7 하네스는 그 자리를 출처 밖에 둔다) · ★그 4/4 자체가 하네스 탓으로 의심된다★ — 하네스가 커서를 `SendInput` 이동 한 번으로 이웃 창 탭바까지 옮겨 그 창이 `DragEnter` 를 받을 다음 마우스 메시지가 없었다. 사람 손은 이웃 창까지 끊김 없이 움직이므로 그 창이 끌기를 보고(W) 커서 판정으로 붙는다(Q21) — 그래서 이 위험은 「이웃 창이 끌기를 정말 한 번도 못 본 끝」에만 남는다 · S7 ⑪ 이 끊김 없는 이동으로 다시 잰다(합격 = 이웃 창에 붙음) · 닫으려면 #28 과 같은 platform 함수 — S7 ⑪ 이 불합격일 때만 사용자에게 올린다 |
| 30 | (5.7) 구역 경계에서 커서가 흔들리면 떼어진 모습이 깜빡임 | 가능성 낮음(경계가 탭바에서 15 px 아래) · 이력 없음은 설계(§2-5 — 상태 없는 판정) · S7 ③ · 거슬리면 이력 안(사용자 보고) |
| 31 | (5.7 · 리뷰 1에서 고침 · 리뷰 2에서 고침) 넘친 탭바(가로 스크롤)의 끄는 중 스크롤 | ★사용자 지시 2026-10-17 로 비목표가 아니다(PRD D20)★ — 끝에 대면 계속 스크롤 · 기하는 내용 좌표라 스크롤이 흔들지 않고 보고가 `scroll` 을 싣는다(§2-5) · (리뷰 2) 다른 창 탭바도 크롬이 내용 좌표라 R21 도 보고의 `scroll` 로 판정하고 막대는 x − 지금 스크롤에 그린다(§1-4 「모드」 넘친 탭바 ④). ★5.7 첫 판의 「다른 창 탭바가 밀린 직후의 R21 놓기는 크롬 보고(100 ms) 전이면 무변경(안전 쪽)」은 거짓이었다★ — 크롬 보고는 부를 때마다 다시 거는 뒤 디바운스라 연속 자동 스크롤 동안 영영 터지지 않아 `bar_rev` 가 그대로였고, 4b 를 통과해 옛 뷰포트 기하의 다른 자리에 붙었다(막대도 옛 x). 남은 위험 = 엔진 끌기 자동 스크롤이 이 탭바에 안 걸릴 수 있음(미검 — S7 ⑬ · 안 걸리면 우리 끝 띠 시한 · 출처 창만) · 「끄는 동안 스크롤 막기」(리뷰 첫 안)는 사용자가 관례로 바꿨다 |
| 32 | (5.7) 순서 이동 중 다른 경로(LLM)가 출처 창 탭을 바꿈 | 가능성 낮음 · 그 순간 영역에서 `reorder` 가 빠져(§2-4 ④) 그 창 탭바는 5.6 끼울 자리 · 처음 판정에서 도킹 모드로 · 모습은 스토어 순서 ≠ 얼린 순서로 거둔다 · 그 전에 놓은 것은 `Tabs` 기대값이 같은 락 안에서 막는다(무변경) |
| 33 | (5.7) 순서 이동 놓기 뒤 `Ended` 가 순서 알림보다 먼저 닿으면 한 번 출렁임 | 가능성 낮음(같은 스레드 emit 순서 — §1-3 「끝」) · 끝 순서는 맞다 · S7 ⑩ |
| 34 | (5.7 · 리뷰 1) 구역 안 Esc 의 떠남 → 끝 신호 간격이 `LEAVE_GRACE_MS` 를 넘으면 탭 하나 팝아웃이 숨은 뒤 「W 를 떠남 → 새 창」(재사용 — 팝아웃이 커서 자리로 뜀) | 가능성 낮음(같은 엔진 사건 줄 · 셸 도착 흔들림 최대 ~50 ms — §8-1) · 숨기지 않아도 「아직 위」가 아니라 같은 새 창이다(5.6 의 남은 틈 그대로 — 여러 탭 창은 새 창) · S1 ① 이 간격을 잰다 · 넘으면 시한을 늘린다(두 판단이 같은 값을 쓴다) · (리뷰 2) 9c 뒤에는 출처 끝의 `last` 가 이 틈을 닫는다 — 웹뷰가 자기 사건 시각으로 유예를 재므로 셸 도착 간격이 100 ms 를 넘어도 시한으로 숨긴 출처를 숨기지 않은 것으로 쳐 4c · 되보이기(§1-3 「시한이 앞지른 끝」 · (리뷰 3) 사각형 검사는 시작에 쥔 출처 사각형으로 남는다 — 커서가 그 안이면 4c · 밖이면 R22 · 엔진 접기 간격이면 검사 없이 4c) · 남는 것은 `last` 를 안 보내는 7 단계 웹뷰(9b 만 든 트리)뿐 · 끝이 시한 직전에 닿고 그 사이 시한 걸음이 먼저 먹힌 경주는 `now` 를 읽기 전에 찍어 닫았다 |
| 35 | (5.7 · 리뷰 1) 문턱 직후 구역 밖으로 빠르게 빼고 멈춘 끌기는 다음 움직임까지 순서 이동 모습(탭이 탭바에 · 팝아웃 보임) | 확실(OLE 가 다음 마우스 메시지 전에 `DragEnter` 를 내지 않는다 — §1-4 「모드」 남는 틈) · 결과는 커서 판정이라 맞다 · 모습만 늦다 · S7 ⑫ |
| 36 | (리뷰 3) 다른 창 W2 에서 출처로 돌아와 그 `Over` 가 셸에 닿기 전에 끝난 끌기(Esc · 받지 않는 자리 떼기)는 도착 순번상 W = W2 — 커서가 W2 밖이면 새 창 · ★(리뷰 4) 리뷰 4 는 이 행을 닫지 않았다 — 좁혔을 뿐이다★: 둘레의 두 끝이 닫혔다 — 반대 방향(출처 → W2 가이드 · 그 `Over` 가 오는 중인 Esc)은 #61 붙들기가, 셸에 아무 `Over` 도 안 닿은 끝(돌아온 출처 `Over` 까지 전부 오는 중)은 #62(W 없음 + `last`)가 닫는다. 이 행의 끝(W2 `Over` 는 닿았고 돌아온 출처 `Over` 만 오는 중)은 W = W2 이고 출처 `last` 간격이 엔진 접기라 #61 조건(간격 > `CANCEL_LEAVE_MS` · W = 출처 · 없음)에도 #62 조건(W 없음)에도 들지 않아 그대로 열려 있다 | 가능성 낮음(돌아온 뒤 수십 ms 안의 끝 — invoke 흔들림 p99 ~16 · 최대 ~50 ms · §8-1) · 5.6 그대로 · 출처 `last` 의 떠남 간격이 엔진 접기(`CANCEL_LEAVE_MS` 안)면 출처가 현재 대상이었다는 증거라 W 를 출처로 올려 닫을 수 있지만 그 경계가 미검이라 미룬다(리뷰 2 판처럼 `last` 만으로 올리면 출처를 떠나 다른 창 가이드 위 Esc 가 새 창이 된다 — §7 #59) · ★(리뷰 4) 경계에 기대지 않는 닫을 후보 = #61 붙들기를 반대로★ — W = 다른 창인데 출처 `last` 가 떠남 없음 · 간격 ≤ `CANCEL_LEAVE_MS` 면 출처의 오는 중 `Over` 를 `OVER_WAIT_MS` 기다려, 닿으면 도착 순번이 W = 출처로 바꾸고(그 뒤는 리뷰 3 갈래 — 사각형 검사 없이 `last`) 안 닿으면 W2 그대로 — 증거가 경계값이 아니라 도착이다. 대가 = 출처를 떠난 지 `CANCEL_LEAVE_MS` 안에 다른 창 위에서 끝난 끝도 시한까지 늦어진다(결과는 W2 그대로). 이번 판은 넣지 않는다(기본값 · 메인 판단 — 끝 직전 되돌아옴만 해당 · 결과 = 5.6) · S1 ① · S7 ⑬ ⓓ 가 `last_gap_ms` 를 잰 뒤 다시 본다 |

### 11-3. 5판에서 뺀 것

- **셸 폴링 끌기**(4판 §1-3 · §1-4) — 끌기 스레드 · 8ms 마감 틱 · `TickClock` · 세션 입력 enum(`ForegroundSeen` · `PointerSeen` · `KeySeen` · `TopSeen`) · `Probe` 질문 · 전경 유예(`INPUT_LOST_MS`) · `esc_armed` · 시작 되감기 가드(`BeginGuard`) — 엔진이 같은 일을 한다. 미커밋이던 `dock/session.rs` 는 지웠다(3 단계 커밋에 넣지 않음 · 사본 = `.claude/handoff/attachments/slot-docking-session-rs-replaced-2026-10-16.rs.txt`) — 끝 어휘 · 띠 캐시 꼴만 6b 가 참고한다(`visible_tab` 은 6a 가 `regions.rs` 에 새로 짰다).
- **platform 함수 · 의존** — `topmost_at` · `NativeWindow` · `foreground`(+ 옛 `foreground_is_current_process` 감싸기) · `pointer` · `escape_key` · `capture_client` · (조건부) `fine_timer` · `raw-window-handle` 운영 의존 · `windows` feature 넷(4판 §11-1 #3 · ADR-C).
- **딱지 창** — `dock-chip.html` · `src/dockChip/main.ts` · Vite 입력 · `capabilities/dock-chip.json` · `dock/chip.rs` · `dock/preview.rs` · 셸 직접 의존 `png` · `base64` · `dock:chip` · `dock:chip-preview` · 자리 기록 거름 · 4판 #43–#46 · #50.
- **출처 숨기기의 화면 밖 옮기기 · 자리 기록 멈춤**(`placement.rs` `record` 거름) — `hide()` 는 자리를 안 바꾼다.
- **LLM 표면**(4판 §3 · ADR-D) — 버스 `tab.dock` · `catalog_version` 18 · 오류 코드 표 · invoke `dock_tab` · 프론트 레지스트리 `tab.dock` · `viewStore.dockTab` · `DockSide` 버스 철자 맞대기 — 다음 작업.
- **`MainLastTab`** — PRD Q9(메인 마지막 탭 끌기)로 거절 사유가 없어졌다.
- **비정상 끝 = 출처 창 탭바로**(4.2 R13b · D11) — PRD Q8.
- **4판 스파이크 S1–S6** · `spike-s2-topmost-2026-10-15.md` 의 결과 — 대상이 없어졌다(§8-1).
- (1판에서 이미 뺀 것) 직전 도킹 되돌리기 · 커서를 따라오는 창 · Esc 취소 · 명령 둘(`tab.dock` · `tab.moveToWindow`).

## 12. ADR 제안 (번호는 `/adr` 가 준다)

결정 날조 금지 — 「거부한 대안」은 PRD · 조사 · 사용자 대화 · 리뷰에서 나온 것만 적었다. 사용자 결정분은 사용자 확인 뒤 박는다(§11-1 은 2026-10-16 확정).

- **ADR-A 도킹 모델 = 탭 끌기 · 탭바 안 순서 이동 · 벗어나면 떼기(5.7) · 배치는 놓을 때 한 번 · 보인 것과 같을 때만 적용 · 붙는 단위는 탭 하나의 배치 통째 · Esc · 비정상 끝 = 그 자리에 놓기 · 메인 마지막 탭도 끌고 떠나면 빈 탭**(사용자 결정 R1 · R2 · R9 · R12 · R13b · R14 · R23 · Q1 · Q6 · Q8 · Q9 · Q19 · Q20 · Q21 + 이 TRD 의 구현 선택)
  - 결정: 제목줄은 OS 이동만, 탭을 끌면 그 창 탭바 구역(탭바 ± 15 CSS px) 안에서는 순서 이동이고(끄는 탭이 커서를 따라 가로로 · 이웃은 가운데 규칙으로 비켜남 · 떼어진 모습 · 가이드 없음 · 브라우저 끌기 그림은 그대로 · 놓으면 보인 자리로 — Q20 · 2026-10-17), 구역을 벗어나면 그 자리에서 떼어진다(그리기). 순서 이동의 자리 규칙은 셸 판정의 일부라(`Picked::Reorder`) 놓기를 같은 함수로 다시 판정하는 원칙이 그대로 선다. 웹뷰가 놓은 자리를 알리지 못한 끝(OLE 가 멈춘 커서에 `DragEnter` 를 안 냄)은 셸이 그 끌기를 본 창 · 원래 창에서만 끝 순간 커서로 판정하고 그 결과를 적용한다(5.7 · 이음새 · 끼울 자리 포함 — Q21 · 가이드를 보인 끌기는 아래 「보인 것과 같을 때만」 그대로). 놓으면 이음새 · 탭바 · 새 창 셋 중 하나이고, 셸이 놓은 점을 다시 판정해 보인 가이드와 같을 때만 적용한다(보인 것도 판정도 가이드 없음 = 새 창 · 그 밖의 어긋남 · 대상 사라짐 = 무변경 — TRD 5.1). Esc · 비정상 끝은 마지막으로 알려진 점에 놓는다(출처 탭이 사라진 경우만 그냥 끝). 붙이기는 모델 이동이고 메인이 비면 같은 연산 안에서 빈 탭을 채운다(ADR-0057 ④). R22 에서 탭 하나뿐인 팝아웃은 그 창째 놓은 자리로 옮기고, 놓은 뒤 옮기기까지의 사이 변화는 다시 확인하거나 되돌리지 않는다(Q19 — 모델을 바꾸지 않는 갈래라 결과 그대로 둔다).
  - 거부한 대안: ① 제목줄(네이티브 이동 루프) 도킹 — 사용자 정정(R1). ② 창이 커서를 따라오는 끌기(1판) — Q1. ③ 끄는 동안 모델을 바꾸고 취소 때 되돌리기 — R2 · R11a. ④ 놓는 순간 판정만(1판) · 마지막 가이드만(2판) · 다르면 늘 무변경(3판) — 못 본 가이드에 놓임 · 손이 움직인 사이 엉뚱한 결과 · PRD R22 어김. ⑤ 웹뷰 reparent — 슬롯 내용은 모델 데이터다(조사 §4). ⑥ 슬롯 하나 직접 끌기 — PRD §2. ⑦ 도킹 라이브러리 채택(dockview · FlexLayout · golden-layout · Lumino) — 자기 레이아웃 모델을 소유해 셸 권위(ADR-0035)와 겹친다(조사 §5). ⑧ 불투명 축소 그림 딱지 창(4.2) — Q5(Windows 기본 그림). ⑨ Esc = 취소 · Esc 없음 — R2 · Q6. ⑩ 비정상 끝 = 출처 탭바로(4.2) — Q8. ⑪ 메인 마지막 탭은 끌기 시작 안 됨(4.2 D3) — Q9. ⑫ R22 재사용을 옮긴 뒤 같은 락 안에서 다시 확인하고 어긋나면 원래 자리로 되돌리기(TRD 5.2–5.5) — Q19(「끌기 시작했으면 뗀 곳에 놓는다 · 예외 로직을 더하지 않는다」 · 숨은 창을 ms 사이 다른 경로만 바꿀 수 있고 그 갈래는 모델을 바꾸지 않는다 · 되돌리기 자체가 깜빡임 · 모델 자리 어긋남 같은 새 틈을 낳았다). ⑬ 끌기 시작부터 떼기 + 원래 창 탭바는 끼울 자리 막대(5.6 · 7 단계 구현) — 사용자 「크롬처럼」 · Q20(2026-10-17). ⑭ 탭바 안은 포인터 사건으로 순서 이동하고 탭바를 벗어나는 순간 셸이 OS 끌기를 시작(조사 `slot-docking-chrome-tab-drag-2026-10-17.md` 선택지 B) — 미검 위험 셋(제스처 도중 OS 끌기 시작 · 커스텀 데이터 · macOS) · OS 함수 신설 · 앞선 조사(ADR-C ①)가 같은 축(셸이 끌기를 몬다)을 기각 · 사용자가 고르지 않음. ⑮ 투명 끌기 그림 + 우리 창 안에서만 우리가 그린 그림(선택지 C) — 사용자 거절(「브라우저 끌기 그림은 그대로」 — 창 밖에서 그림이 사라져 R2 후퇴). ⑯ 웹뷰가 순서 이동 자리를 정해 보고하고 셸이 그것을 믿기 — 보고 없는 끝에서 셸이 자리를 못 낸다 · 놓기 재판정 원칙이 깨진다(§7 #42). ⑰ 탭바 안 판정을 지금의 「커서 x ≤ 머리 가운데」로 — 끄는 탭이 이웃을 다 덮은 뒤에야 비켜나 「가장자리가 가운데를 넘으면」(Lumino · Chrome · 사용자 결정)과 다름(§7 #42). ⑱ 보고 없는 끝을 아무 우리 창 사각형으로나 판정(메인 첫 제안의 넓은 꼴) — 다른 앱 창이 덮은 우리 창으로 오판(AC8 · §7 #48). ⑲ 보고 없는 끝 · 빈 보고의 이음새 · 끼울 자리 = 무변경(Q18 · M1 을 이 경우에도) — 사용자 결정 Q21(2026-10-17)이 고르지 않음(창 안으로 옮겨 온 것 자체가 마우스 움직임이라 뗀 순간 커서 자리가 목적지다).
- **ADR-B 크기 규칙 = 새것 1/3 하나 · 면적 0 만 거절**(사용자 결정 R7 · R8) — 거부한 대안: Unity 식(대상 1/3 · 최소 100px · 바깥 200px · 옆 칸 양보) · 반반 + 바깥 1/3 · 크기 기억(JetBrains · AvalonDock) · 퍼센트 + 기준 px · 200px 고정 띠 · 30%(조사 §8 · 사용자 확정 2026-10-15).
- **ADR-C 창 사이 끌기 수단 = 브라우저 내장 HTML5 끌어 놓기 · 셸은 영역 · 놓기 판정 · 적용**(사용자 결정 Q7 + 이 TRD 의 구현 선택 · 조사 `slot-docking-drag-mechanism-2026-10-16.md`)
  - 결정: 탭 머리는 `draggable` 이고 커서 추적 · 맨 위 창 · 끌기 그림 · 버튼 · Esc 를 엔진에 맡긴다. 모든 런타임 창은 wry 드롭 처리기를 끈다. 셸은 창별 영역을 미리 계산해 보내고, 웹뷰는 점-사각형 판정으로 가이드를 그리며, 놓기는 셸이 같은 규칙으로 다시 판정해 적용한다. 끝 신호 = 대상 `drop` · 출처 `dragend` · 끝 뒤 포인터 사건이고, `none` 끝은 마지막 보고 · 셸 커서로 「그 자리」를 정한다. OS 별 코드는 없다.
  - 거부한 대안: ① **셸 폴링 루프(4판 설계)** — 8ms 틱으로 OS 커서 · 버튼 · Esc · 전경 · 맨 위 창을 읽고 딱지 창을 옮기는 앱 주도 끌기. OS 함수 다섯과 `raw-window-handle` 의존(ADR-0266 규칙 개정) · 딱지 창 · 타이머 해상도 · 전경 유예가 필요했고, 같은 일을 엔진이 OS 끌기로 이미 한다(실측 — 메인 스레드 안 막힘). ② **crabnebula `tauri-plugin-drag-as-window` / `drag-rs`** — `DoDragDrop` 을 Tauri 메인 스레드에서 돌려 wry 의 위치 사건이 끄는 동안 막혔다 몰려온다(실시간 가이드 불가 · 열린 이슈 #77) · Esc = 취소 고정 · 끄는 동안 위치 없음 · 실제 임시 파일을 실음 · 그림 크기 조절 없음 · 끝나지 않아 앱이 멈추는 열린 버그 #94(조사 §1 — 소스 대조). ③ **네이티브 제목줄 끌기(창이 따라오는 이동 루프 — Chromium 탭 떼기 · 조사 §4 전제)** — 제목줄 = 이동만이라는 사용자 결정(R1)과 어긋나고 이동 루프 끝 감지에 창 서브클래스가 든다. ④ **포인터 직접 추적 + 클릭 통과 고스트 창**(block/buzz-app 등 — 조사 §2) — ① 과 같은 꼴이고 그 피어들의 「HTML5 는 창 밖으로 못 나간다」는 wry 처리기를 켠 채의 관찰이었다. ⑤ **DOM 끌기 라이브러리**(pragmatic-drag-and-drop · dnd-kit · react-dnd) — 문서 하나 안에서만 돈다. ⑥ **움직일 때마다 셸에 판정을 묻기** — invoke p99 ~16ms · 최대 ~50ms(실측)라 60Hz 가이드가 늦고 응답 순서를 따로 맞춰야 한다.
- **ADR-D 도킹의 LLM 표면을 다음 작업으로 미룬다 — CLAUDE.md 「LLM-우선 제어」의 의식적 예외**(사용자 결정 Q10)
  - 결정: 이 기능은 버스 · 프론트 레지스트리 명령을 만들지 않는다. 적용 서비스 `dock_tab` 과 타입드 오류 · `Keep` 정책 · 표식 판정을 남겨 다음 작업이 그 위에 명령만 얹게 한다. 다음 작업은 LLM 도구 설계 관례 조사로 시작한다.
  - 거부한 대안: ① **TRD 4판의 `tab.dock` 하나 + `mode` · 선택 칸** — 조합 실수 여지가 크다(사용자 기준: 흔한 요청은 1–2 호출 · 메모의 동사 셋 후보로 밀림). ② **CDP(원격 디버깅 포트)로 DOM 을 LLM 이 보기** — 포트에 인증이 없고(로컬 아무 프로세스나 앱 JS 실행) DOM 은 모델이 아닌 그림 결과라 운영 불가(메모 2026-10-16).
- **ADR-E 팝아웃 닫기 요청에서 모델 먼저 지우기 + 닫힌 label 묘비**(3판 결정 M2 · §11-1 #2 사용자 결정 2026-10-16)
  - 결정: 팝아웃의 `CloseRequested` 에서 레이아웃 락 한 번 안에 그 창의 모델 정리를 하고 label 에 묘비를 단다. 도킹 · 새 창 확정 · 팝아웃 phase C 는 없는 창 · 묘비 앞에서 무변경으로 멈춘다. 근거 = 팝아웃 닫기 요청은 되돌려지지 않는다. ADR-0057 불변식은 그대로이고 정리 계기만 옮긴다.
  - 거부한 대안: ① 열림 사전 확인 + 남는 위험 기록(2판) — 틈을 줄일 뿐 닫지 못한다. ② 닫는 중 표시만 하고 정리는 소멸 때 — 지금은 같은 결과에 칸만 늘린다(닫기를 막을 수 있게 되면 이 안으로). ③ 확정 뒤 `is_open` 으로 다시 보기 — 옮긴 탭을 되살리지 못한다. ④ 닫기가 먼저면 그 자리에 새 창 · 토스트 알림 — 사용자가 「흐름에 맡긴다」로 거절(2026-10-16).

## 부록 — 요구사항 추적

| PRD | 어디서 서나 |
|---|---|
| R1 · AC1 | 제목줄은 OS 이동 그대로 — 이 기능이 끼어들지 않는다 · S6 |
| R2 · AC2 · AC6 · AC20 · AC24 | (5.7) 떼기는 구역을 벗어날 때 — 모드(§1-4) · 미룬 숨기기(§1-3 「도킹 모드 증거」) · 떼어진 표시(§4-1 — 그리기만 · 도킹 모드에서만) · 탭 하나 팝아웃 숨기기(§1-3 · S1 ⑤) · 끌기 그림(§4-2 · S2) · 모델 무변경(§1-3) |
| R3 · R4 · R5 · R6 · AC2 · AC4 | 영역(띠 = 결과 사각형 · 바깥 띠 32px · 가운데 없음 — §2-4) · 그리기(§4-3) |
| R6a | §2-2 규칙 6 · 8 · 경계 포함 규칙 · 골든 |
| R6b · AC14 | (5.7) 원래 창 구역엔 머묾 없음(`Reorder` 에 `over` 없음 · Q20 에서 따라 나온 것 · §11-1b ②) · 다른 창 탭바 머묾은 그대로 · 웹뷰 머묾 500ms → 기존 탭 전환(§1-3 · §1-4) · 전환 순간의 이음새 놓기 = 무변경 · 탭바 놓기는 적용(§1-3 「보이는 탭 경쟁」 · 「놓기 판정」 4b) |
| R7 · AC3 · AC4 · AC5 | `docked_tree` 분할 모양 표(§1-2) |
| R8 · AC15 | 새 면적 0(§1-2) · 빠진 띠(§2-4) |
| R9 | 출처 View 의 트리 통째 이동(§1-2) |
| R10 · AC3 · AC6 | 빈 팝아웃 항목 제거 + `host.close`(§1-2 · §5) |
| R11 · AC7 | 같은 락 `resync`(§5) · 닫히는 창 replay 가드(3 단계 — 보이는 창이 있으면 지나가고 seq 연속이 거른다 · §5) |
| R11a · AC11 | 끄는 동안 모델 무변경 · 락 한 번 · 메인 빈 탭도 같은 락 · 기록기 스냅숏(§1-2 · §5) |
| R11b · AC3 | `FocusPolicy::Reveal` + `set_focus`(§1-2 · §1-3) · 적용 중 끝 = `Applied` 하나(§1-3 「적용 중」) |
| R12 · AC19 · AC25 | (5.7) 구역 안 Esc = 그 보고로 4c(보이던 자리) · `dragend(none)` → 마지막 점에 놓기 · 창별 마지막 Over · 떠남 따로(§1-3) · (리뷰 2) 끝 스냅숏 `last` · 시한이 앞지른 끝(§1-3 「끝 신호」 · 「도킹 모드 증거」) · (리뷰 3) W = 도착 순번 · `last` 는 W = 출처일 때만 · 떠남 간격 `CANCEL_LEAVE_MS` · 시한이 앞지른 끝도 사각형 검사(쥔 출처 사각형) · (리뷰 4) 오는 중 `Over` 붙들기(`OVER_WAIT_MS`) · W 없음 + `last` = 출처(§1-3) · Esc 삼키기(창별 상한 — §1-4) · S1 |
| R13b · AC16 · AC21 · AC27 · AC28 | (5.7) 빈 보고 · 보고 없는 끝 = 커서 판정(W · 출처만 — §1-3) · 판정 결과 적용(이음새 · 끼울 자리 포함 — Q21 · §11-1b ①) · 엔진 취소 = 사건 시각 · `canceled` 무변경(§1-4 · §1-3) · 끝 신호 셋 · 마지막 점에 놓기(커서 못 읽음 = W 의 마지막 점) · 유실 대비 · 이른 끝 신호 붙들기 · 출처 사라짐 = 그냥 끝 · `Ended` 방송(§1-3 · §4-3) |
| R13c · AC16 | `blocked` · 영역 `rev` · 보인 것 ↔ 지금 판정 대조(대상 사라짐 = 무변경) · 보이던 탭(이음새만) · 탭바 몫 `bar_rev`(탭바) · `expect_source` · `expect_target`(이음새 = 배치 · 대상 창 활성 · 탭바 = 탭 순서 — §1-2 · §1-3) · 예외 = 재사용의 놓은 뒤 변화는 결과 그대로(Q19 · §1-2) |
| R14 · AC9 | `refill_main` · 메인 빈 화면(§1-2 · §4-1) |
| R15 · AC13 · AC14 | 엔진의 맨 위 창 · 창별 영역 · 보이는 탭 · 탭바(§1-4 · §2-4) |
| R16 · R17 | 다음 작업 — §3 · ADR-D |
| R18 | HTML5 끌어 놓기 + 셸 영역 · 판정 · 적용(§1 · ADR-C) · 다른 끌기 리스너(react-arborist) 피하기(§1-4 · S1 ⑦) |
| R19 | OS 함수 0 · Tauri API 셋(§1-6) |
| R21 · AC18 | (5.7) 다른 창 탭바만 — 원래 창은 R23 · `reorder` 없으면 5.6 끼울 자리로 물러남(§2-4) · `DockTo::TabBar`(§1-2) · 탭바 영역 · 자리(§2-2 규칙 3 · §2-4) · 탭바 몫 대조(§1-3 「놓기 판정」 4b) · (리뷰 2) 내용 좌표 크롬 · 보고 `scroll` · 막대 변환(§1-4 「모드」 넘친 탭바 ④ · §1-5 · §4-3 · S7 ⑬ ⓒ) |
| R23 · AC23 · AC24 · AC25 · AC26 | (5.7) 구역 · 자리 규칙(§2-5 · `hit::reorder_*` · 골든) · 출처 영역 `reorder`(얼린 `strip` · 조건 ①–⑥ — §2-4) · 모드(§1-4) · 순서 이동 모습(§4-4) · 놓기 판정 4c(§1-3) · 탭 하나 = 무동작 · 미룬 숨기기(§2-5 ④ · §1-3) · S7 |
| R22 · AC8 · AC12 | `DockTo::NewWindow` · 세 phase · 묘비 · 재사용(다시 확인 · 되돌리기 없음 — Q19) · 크기 · 자리 검증(§1-2) · 「보인 것 · 판정 모두 가이드 없음 = 새 창」 · 새 창 자리(§1-3) · S3 · S4 |
| AC22 | 파일 드롭 막기(§1-4) · S3 ② |
| W1 · W10 | S1(⑦ 트리 슬롯 · ⑧ 사건 순서 포함) |
| W4 | S4 |
| W5 | S6 |
| W7 · W8 | §4-2 · S2 · §11-1 #7 |
| W9 | §1-4 · S3 |
| W11 | §8-2 macOS |
| W12 | S5 |
| W13 · W14 | §4-4 · S7 ② · §11-2 #27 |
| W15 · D21 | §1-3 「커서 판정」 · §11-2 #28 · #29 |
| D15–D19 | §6-2 끝 줄 · §1-4 「모드」 · §2-5 |
| D20(사용자 지시 — 리뷰 1) | §1-4 「모드」 넘친 탭바 · §2-5 스크롤 · §4-4 · S7 ⑬ · §11-2 #31 |
| D22 | §3 |
| §5-3(닫힘) · Q21 | §11-1b ① ② · §1-3 「커서 판정」 · S7 ⑪ |
