# ADR-0302: 창 상태는 버스 명령이다 — main 닫기는 모든 창 숨기기 · LLM 보이기는 포커스를 빼앗지 않음 · 창 인자는 영속 id 와 같은 실행 안 label 이고 번호는 받지 않는다

- 상태: 확정 (2026-10-26, 근거: 사용자 결정 2026-10-23(Q7 — main ✕ = 모든 창 숨기기 · Q9 · D11 — 번호를 창 인자로 받지 않는다 · 리뷰 1차 지적 ② — 0탭 자가닫힘은 닫는 순간 셸이 다시 확인) + 메인 결정 M7 · M8 · M12 · M13 · M14 · M22 · M23 · M25 · M29 · M30 · M31 · M34(TRD §12-3) + PRD 기본값(메인 제안 — R17 · R18 · D12 · W10) + 구현 U5 · U6 `bbd34258` · U7a `07f35609` · `e329eb21` · U7b `24782701` · 모델 밖 창 = U8b 리뷰 2 · 3 `056e6a63` · `49b64836`)
- 관련: PRD `docs/process/S21-window-chrome/prd.md`(0.5판 — R6 · R10 · R17 · R18 · R23 · D6 · D11 · D12 · W7 · W9 · W10 · AC8 · AC9 · AC10 · AC24 · AC26 · AC28 · AC32) · TRD `docs/process/S26-window-chrome/trd.md`(0.3판 — §2-3 · §2-4 불변식 3 · 5 · §4-5 · §5-2 · §5-4 · §5-5 · §6 · §9 · §10-4 · §15) · ADR-0026 · ADR-0229(main 닫기 = 모든 창 숨기기 · 숨기기 경로는 하나) · ADR-0299 결정 5(포커스를 빼앗지 않고 z 만 올린다 · Alt 합성 없음) · ADR-0081 · ADR-0155(사람과 LLM 이 같은 적용 서비스 · 명령 표 선언) · ADR-0167(옛 팝아웃 label 은 실행마다 1 부터 다시 센다) · ADR-0300(창 번호) · ADR-0303(창 단추) · `src-tauri/src/layout/window_ctl.rs` · `src-tauri/src/layout/window_arg.rs` · `src-tauri/src/commands/window_ctl.rs` · `src-tauri/src/tray/actions.rs` · CLAUDE.md 「LLM-우선 제어」 · step-log S26 · Amends ADR-0167 (근거의 팝아웃 label 꼴 「앱을 띄울 때마다 1 부터 다시 세는」 카운터)

## 맥락

LLM 이 창을 보이거나 앞으로 낼 길이 없었다 — 보이기 · 숨기기는 트레이와 그 invoke(`show_main_ui` · `hide_main_ui`)뿐이었고 창 하나를 앞으로 내는 명령은 없었으며 `window.close` 는 메인을 거부했다(PRD §1 · R17). 창 단추를 우리가 그리면(ADR-0303) 이 갭이 사람과 LLM 사이에 그대로 드러난다.

창을 가리키는 인자도 문제였다. 보이는 번호는 다시 쓰이고(ADR-0300), 팝아웃 label 은 실행마다 1 부터 다시 세므로(ADR-0167 근거 · `PopupCounter`) 재시작 뒤 늦게 온 명령이 다른 창에 닿을 수 있었다(PRD D12 · 2차 리뷰 BLOCK).

OS 쪽 사실도 묶였다 — tao 의 최대화 · 최소화 · 펴기는 숨은 창을 드러내고(`state::placement::DeferredMaximize` 주석), 새 창 · 복원 창은 첫 화면 문(`first_paint`)이 보이기를 소유한다(ADR-0299 결정 3).

## 결정

1. **창 상태 바꾸기는 전부 버스 명령이고 사람의 단추는 그 명령을 부른다**(R17 — 원칙 · 메인 결정 M7). 새 명령 `window.minimize` · `window.maximize` · `window.unmaximize` · `window.raise` · `window.hideAll` · `window.showAll` · 넓힌 `window.close`(`if_empty`) · `window.list` 에 `details`(영속 id · label · 번호 · main · 보임 · 최소화 · 최대화 · 앞 창 — 옛 `windows` 칸은 그대로 · R18). 서비스 = `layout/window_ctl.rs`(Tauri 0) · 포트 `WindowControl` · 사람 길 invoke 하나 `window_control`(같은 서비스에 떨어진다 — ADR-0081 두 껍데기). 「원래 크기」 명령을 `window.restore` 로 짓지 않는다(메인 결정 — 버스 `restore.*` 와 헷갈린다). 옮기기 · 크기 · 스냅 명령은 없다 — 알려진 갭이다.
2. **웹은 Tauri 창 API 의 최소화 · 최대화 · 닫기 · 보이기 · 앞으로 내기를 부르지 않는다**(TRD §2-4 불변식 3 · R17) — 게이트 = TypeScript 컴파일러 API 로 받는 쪽 타입을 보는 vitest 시험 `src/__tests__/noWindowControl.test.ts`(메인 결정 M25) · 팝아웃 capability 의 `core:window:allow-close` 를 걷어 권한으로도 막는다(TRD §9).
3. **`window.close(main)` = 모든 창 숨기기**(사용자 결정 2026-10-23 — Q7 · ADR-0229 의 한 경로 `tray::actions::hide_main_ui`) — 답 `HiddenAll`(옛 `CONFLICT` 를 바꾼다). 팝아웃 = `apply::close_window`(답 `Closed`) → OS 창 파괴 → `Destroyed` 에서 번호를 푼다(ADR-0300). 단추 · Alt+F4 · 버스 · 마지막 탭 닫기 · 0탭 자가닫힘이 한 결과로 모인다(R23). 팝아웃 닫기 요청을 되돌리지 않는다는 전제(W7)를 지킨다 — 제목 줄 단추는 OS 닫기 요청을 내지 않고 버스로 간다.
4. **0탭 자가닫힘 = `window.close { window: <영속 id>, if_empty: true }`**(사용자 결정 2026-10-23 — 닫는 순간 셸이 다시 확인 · 메인 결정 M13) — 공통 해석기보다 먼저 · 한 락에서: 그 id 의 항목이 모델에 있으면 `Kept` · 없고 `closing` 에 있으면 그 label 을 닫는다 · 둘 다 없으면 `Kept`. 「0 탭」 = 모델에 항목이 없다(팝아웃 항목은 탭이 0 이 되는 연산에서 빠진다). 단 번호 거부가 그보다 먼저다.
5. **LLM 의 보이기 · 앞으로 내기는 포커스를 빼앗지 않는다**(메인 결정 M22 · PRD W10 기본값 · ADR-0299 결정 5) — 이미 있는 `first_paint::raise_and_show` 차례를 쓴다: ⓐ platform `place_window(…, ZOrder::Top, show: false, activate: false)` 로 숨긴 채 z 만 맨 위 ⓑ Tauri `show()` · `unminimize()`(tao 보임 · 최소화 깃발이 맞게 남는다) 뒤 적어 둔 바람 입히기 ⓒ 지금 전경이 우리 프로세스일 때만 `set_focus()` · Alt 합성 없음 · 답의 `focused` 는 사실 그대로. 트레이 보이기와 한 경로 `tray::actions::show_main_ui(app, Activation)` — `TrayClick` = 트레이 아이콘 누름 · 단일 인스턴스 두 번째 실행 콜백(사람의 손) · `NoSteal` = 버스 `window.showAll` · 숨은 창의 `window.raise` · invoke `show_main_ui`(LLM · cdp 표면 — 메인 결정 M31).
6. **보이지 않는 창 규칙**(메인 결정 M23 · M30 · TRD §5-2 표) — 첫 화면 대기 창 · 숨은 창의 `minimize` = `CONFLICT` · `maximize` / `unmaximize` = 바랄 상태를 적고(`DeferredMaximize` — 보일 때 입힌다 · 답 `deferred: true`) 이미 그 크기면 바람만 거둔다 · 첫 화면 대기 창의 `raise` = 무동작(`pending: true`). **LLM 보이기(`NoSteal`)는 첫 화면 대기 창을 건너뛰고 트레이 누름은 지금처럼 기다림을 거두고 보인다**(불변식 5). 모든 보이기 갈래가 미룬 최대화를 입힌다.
7. **창 상태 한 동작 = 메인 스레드 일 하나**(메인 결정 M29) — 상태 읽기(`is_held` · 보임 · 최대화 · 최소화) → 순수 판정 `window_ctl::plan` → 바람 적기 · 거두기 · OS 호출을 `app.run_on_main_thread` 일 하나에서 하고 답을 기다린다(`mpsc::channel` + `recv_timeout`). 버스 동사와 `window.list` 는 `offloaded_handler` 로 꽂고 invoke `window_control` 은 async · `show_all` · `hide_all` 도 같은 꼴이다. 레이아웃 락을 쥔 채 부르지 않는다.
8. **앞 창 판정 = platform `window::is_foreground`**(메인 결정 M12) — Windows 에서 Tauri `is_focused` 를 쓰지 않는다.
9. **창 인자 = 영속 id · 같은 실행 안 label 별칭 · 번호 거부**(사용자 결정 2026-10-23 — Q9 · D11 / label 별칭 · 실행 표식 = PRD D12 기본값 · 꼴 = 메인 결정 M8). 해석기 하나 `layout/window_arg.rs` 를 버스 · invoke 껍데기가 같이 쓴다: `main` · 영속 id(같은 철자 먼저) · 모델 안 label → 그 창 · 숫자만 = `INVALID_ARGUMENT`(도움말이 `window.list` 의 `details` 에서 그 번호의 `window_id` 를 찾으라고 알린다) · 그 밖(앞 실행 label · 닫힌 창) = `CONFLICT`. 팝아웃 label = `slot-popup-<실행 표식 8 hex>-<n>`(실행 안 단조 · glob `slot-popup-*` 그대로 · 앞 실행과 표식이 같을 확률 2⁻³² 는 받아들인다). 이 꼴이 ADR-0167 근거의 「앱을 띄울 때마다 1 부터 다시 세는」 label(`slot-popup-1`…)을 바꾼다 — 그 ADR 의 결정은 건드리지 않는다(U10 2026-10-26 에 개정 링크를 더했다).
10. **창을 만드는 세 자리의 답 = `{ window, window_id, number }`**(메인 결정 M14 · R6) — invoke · 버스(`window.create` · `slot.popout`) · 웹 레지스트리가 같은 셋을 돌려준다.
11. **모델 밖 창의 창 단추**(메인 결정 M34 · 2026-10-26 · U8b 리뷰 2) — 창 인자가 모델에 없고 그 철자가 열린 팝아웃 OS 창의 label 이며 예열 창 풀의 창이 아니면: 최소화 · 최대화 · 원래 크기 = 그 창에 바로(같은 `plan` · 같은 미룬 최대화 표 · 모델 표식만 안 적는다) · 닫기 = **거두기 전에 레이아웃 락 안에서 묘비(`ViewManager::mark_closed`)부터 달고** OS 창을 거둔다(답 `Closed` · 그 사이 모델에 들었으면 모델 창 닫기) · 앞으로 내기 = 그대로 「창 없음」(`CONFLICT`) · wire 모양 그대로. main 은 늘 모델에 있어 이 갈래에 들지 않는다.

## 거부한 대안

- **번호를 창 인자로 그냥 받기**(PRD Q9 ③ · 0.1판 D11 「둘 다 받는다」) — 2 를 닫고 새 2 가 생긴 뒤 늦게 도착한 `window.close("2")` 가 새 창을 닫는다. 기각 = 사용자 결정 2026-10-23(사용자가 까닭을 확인하고 골랐다).
- **번호를 받되 `window.list` 가 준 목록 판을 함께 받고 바뀌었으면 `CONFLICT`**(PRD Q9 ②) — 칸이 늘고 LLM 이 판을 들고 다닌다. 기각 = 사용자 결정 2026-10-23.
- **창 인자 = 영속 id 만(label 거부)**(PRD D12 ①) — 가장 단순하지만 창 인자를 쓰는 명령(`tab.*` · `window.close` · `window.getTheme` / `setTheme` · `slot.popout` 등)의 모든 호출이 바뀐다. 기각 강도 = 서술(변경 범위).
- **label 카운터를 상태 파일에 저장해 실행을 넘어 단조**(PRD D12 ②) — 저장이 늦거나 실패한 채 꺼지면 앞 실행이 나눠 준 label 보다 작은 값부터 다시 센다. 기각 강도 = 서술.
- **지금처럼 label 만**(PRD D12 ③) — 재시작 뒤 늦은 명령이 다른 창에 닿는다(PRD 2차 리뷰 BLOCK). 기각 강도 = 리뷰.
- **main ✕ = 메인만 숨기기**(PRD Q7 ②) — ADR-0229 「숨기기 경로는 하나다」의 번복(새 ADR)이고 메인 없이 팝아웃만 보이는 상태가 생긴다. 기각 = 사용자 결정 2026-10-23.
- **포커스를 빼앗지 않는 보이기를 날 Win32 `place_window(show: true, activate: true)` 로**(TRD 0.2판) — tao 의 보임 깃발이 거짓으로 남아 다음 트레이 숨기기가 아무것도 하지 않는다(`first_paint::raise_and_show` ⓑ 주석 · `commands::spare` 머리 · TRD 0.3판 재리뷰 ①). 기각 강도 = 코드.
- **펴기를 날 Win32 `restore_no_activate`(`SW_SHOWNOACTIVATE`)로**(TRD 0.2판) — 같은 깃발 문제가 생긴다. Tauri `unminimize()`(tao = `SW_RESTORE`)를 쓴다. 기각 강도 = 코드.
- **다른 앱이 앞일 때 전경을 빼앗기(Alt 합성 등의 우회)** — ADR-0299 결정 5(사용자 결정 2026-10-21 「포커스 뺏지말고, 순서만 위로올려」)에 어긋난다. 기각 = 사용자 결정(ADR-0299).
- **Windows 에서 Tauri `is_focused` 로 앞 창을 판정**(TRD 리뷰 Claude ④) — wry 가 창의 `WM_SETFOCUS` 마다 포커스를 웹뷰로 넘겨 웹뷰가 키보드 포커스를 쥔 동안 그 값이 거짓이다(코드 주석 `state::placement::foreground_is_ours` · 소스 독해 · GUI 미검). 기각 강도 = 코드(소스 독해).
- **버스 스레드에서 `is_held` 를 읽고 메인 스레드에서 미룸을 적기**(TRD 0.2판) — 그 틈에 첫 화면 보이기(메인 스레드)가 지나가면 미룸이 홀로 남고, `state::placement::record` 가 미룸이 서 있는 동안 자리 적기와 `window:frame` 밀기를 멈춘다(TRD 0.3판 재리뷰 ②). 기각 강도 = 코드.
- **`if_empty` 를 공통 창 인자 해석기로 보내기**(TRD 리뷰 codex ②) — 그 길의 창은 대개 이미 모델에 없어 해석기가 `NotFound`(= `CONFLICT`)로 답해 버린다. 기각 강도 = 코드.
- **0탭 자가닫힘을 웹이 Tauri 창 닫기로 직접 부르기**(지금까지 — `WindowLayout.tsx` 의 0탭 효과) — 셸 닫기 길(모델 · 번호 · 트레이 정리)을 타지 않고(PRD W9), 웹이 본 때와 셸이 닫는 때 사이 틈에 탭이 들어와도 닫는다(사용자 결정 2026-10-23 — 닫는 순간 다시 확인). 기각 강도 = 사용자 결정 + 서술.
- **AC10 게이트를 rg 한 줄로**(TRD 0.1판) — 줄바꿈 · 별칭(`const w = getCurrentWindow(); w.close()`)에 뚫린다(TRD 리뷰 Claude ⑦). 기각 강도 = 리뷰.
- **모델 밖 창의 명령을 지금처럼 「창 없음」으로**(U8b 앞 동작) — 부팅 실패 화면(모델에 없는 낡은 팝아웃 · 역할 질의가 실패한 들인 예열 창)의 창 단추가 보낸 명령이 말없이 사라졌다(웹은 `fireAndForget`). OS 틀이 걷힌 창이라 그 화면에도 창을 옮기고 닫을 자리가 있어야 한다(TRD §7-12 · ADR-0306 결정 2). 기각 강도 = 코드.
- **모델 밖 창 닫기의 묘비를 `Destroyed` 에 맡기기**(U8b 리뷰 codex · 2026-10-26) — 파괴는 이벤트 루프에 줄 서므로, 그 틈의 새 창 옮기기 · 슬롯 분리 확정이 이 label 로 붙고 `Destroyed` 정리가 옮겨 온 탭째 지운다. 묘비를 먼저 달면 그 확정이 `WindowClosed` 로 거절돼 탭은 출처에 남는다. 기각 강도 = 리뷰(코드).
- **「원래 크기」 명령 이름 `window.restore`** — 버스의 `restore.*`(크래시 사본)와 헷갈린다(메인 결정 · TRD §6-1). 기각 강도 = 서술(약함 — 이름 고르기).

## 근거

- 사용자 결정 PRD §5-1(Q7 · Q9 · 리뷰 1차 지적 ② — 2026-10-23) · ADR-0229 · ADR-0299 결정 5.
- 리뷰 · 실측 기록 = TRD §15(0.1판 리뷰 codex ② · ⑤ · ⑥ · Claude ④ · ⑤ · ⑦ · 0.3판 재리뷰 ① – ③ · 리뷰 4).
- 소스 독해(TRD §5-2 · §6-2): tauri-runtime-wry 2.11.3 `send_user_message` 는 메인 스레드에서 넘긴 일을 그 자리에서 돈다 — 그래서 메인 스레드에서 불러도 교착은 아니지만, 레이아웃 락을 쥔 채 부르면 재진입 안 되는 뮤텍스를 다시 잡고, 답 채널 용량이 0 이면 그 자리에서 도는 일의 보내기가 선다. tao 0.35.3 은 최소화의 `WM_SIZE` 에서 최대화 깃발을 내린다(그래서 보이지 않는 최소화 창의 `maximize` · `unmaximize` 는 늘 바람을 적는다).
- 조사 이름 §2 ①(tmux — 번호와 id 의 가름) · PRD R6.

## 영향 / 불변식

- **불변식 3** — 웹이 Tauri 창 상태 API 를 부르면 `noWindowControl` 시험이 빨개진다. 새 창 단추 · 메뉴도 레지스트리 명령 → invoke `window_control` 로 간다.
- **불변식 5** — 첫 화면 대기 창을 드러내는 것은 첫 화면 문과 사람의 트레이 보이기뿐이다. 창 상태 명령과 LLM 보이기는 그 창을 건너뛴다.
- `WindowControl` 은 레이아웃 락 밖에서 부르고 실물은 한 동작을 메인 스레드 일 하나로 돈다 — 둘로 쪼개면 첫 화면 보이기와 끼어들어 미룸이 홀로 남는다.
- `focused: false` 는 정상 답이다 — 다른 앱이 앞이면 맨 위로만 올린다(의도). `SW_RESTORE` 의 활성화가 Alt 를 합성하지 않는지는 tao 경로를 읽지 않았다(가능성 높음 — QA 가 AC9 로 잰다).
- 모델 밖 창의 남는 갭(받아들임 · M34): 그 화면의 창 사실 당기기는 유계 재시도 뒤 `console.error` 한 줄로 끝나고, 최대화 단추 모양이 OS 상태를 따르지 않는다.
- 알려진 갭: LLM 의 창 옮기기 · 크기 · 스냅 명령이 없다(CLAUDE.md 「LLM-우선 제어」 남은 갭에 더한다).
- 앵커 — `layout/window_ctl.rs`(`plan` · `close_if_empty` · `close_unmodeled`) · `layout/window_arg.rs` · `commands/window_ctl.rs`(메인 스레드 일) · `tray::actions::show_main_ui` 의 `Activation` 갈래 · `PopupCounter` 의 label 꼴에 `// ADR-0302` 를 단다.
