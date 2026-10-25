# TRD — 창 크롬 · 이름 (크롬식 제목 표시줄 · 보이는 창 번호 · 창 제목 · 창 상태 명령)

> **U2 구현 중 고침**(2026-10-25 · U2 `/review code full` 2 라운드): §3-3 번호 경고(칸 없음 · null · main 은 경고 없음) · §4-1 `Destroyed` 번호 풀기 자리(보통 갈래는 한 락 안 · 상태가 빠진 갈래만 따로) · §11 U7a 행에서 그 몫을 걷음.
>
> **0.3판 — 재리뷰(Claude Architect-breaker · light) FIX 반영 · 디버그 세션 실측 반영**(2026-10-24 · 표기 = 「0.3판」 · 상세 = §15-5): ① LLM 의 포커스 안 빼앗는 보이기 = 날 Win32 가 아니라 `first_paint::raise_and_show` 차례(tao 보임 깃발이 거짓으로 남지 않게) · `restore_no_activate` 걷음(§6-2) ② 창 상태 한 동작 = 메인 스레드 일 하나 — 첫 화면 보이기와의 경주를 없앤다 · 시험 더함(§5-2) ③ LLM 보이기는 첫 화면 대기 창을 건너뛰고 트레이 누름은 그대로 — 불변식 5 를 맞춤(§2-4 · §5-2) ④ `caption_style()` 을 U7a 로(§11) · 잔손 다섯(사건 값 · 늦은 제목 일 · 번호 거부 차례 · invoke `show_main_ui` = `NoSteal` · 번호 풀기를 조건문 앞으로) ⑤ 디버그 세션 실측 셋 — 접힌 끌기(W13 · S13 · 위 여백 = 메인 제안 · 사용자 확인 필요(QA)) · 끌 자리 ②(CSS `app-region`) 위 놓기 잃음 확인 · 위 가장자리 자식 창 위 놓기 잃음 가능성(W14 · S14) · 출처의 접힌 끌기 판정 기록(§7-9). PRD 는 0.5판 안에서 R1 예열 창 줄 · R7/R8 위 여백 · W13 · W14 를 고쳤다.
>
> **0.2판 — 사용자 결정 · 스파이크 결과 · 리뷰 2 건 반영**(2026-10-24 · S26 · T-66). ① **T1 = 첫 화면 전 실패를 따로 처리하지 않는다**(결정 · 2026-10-24 — 「웹 화면 프로세스가 죽는 드문 경우」) — 0.1판의 첫 화면 기다림 · 되돌림 · `TabsFate` · 1.5 s 시한 보이기 걷기 · U7(옛) 전부를 걷었다. 새 창은 지금처럼 첫 화면 알림이나 시한(1.5 s)에 보인다 ② **Q1 = 스냅 레이아웃 메뉴를 켜지 않는다**(결정 · 2026-10-24) — 오버레이 코드는 저장소에 두되 기본이 꺼진 cargo feature `snap-overlay` 뒤로 컴파일에서 뺀다(§8-4 · U9) ③ **T2 = 낱말** 빈 칸 · 에이전트 · 에이전트 트리 · 프리셋 · 사용량(결정 · 2026-10-24 — 「알 수 없는 내용」은 메인이 남겼다 · 까닭 §4-2) ④ **T3 = main 셋째 단추** lucide `EyeOff` · 옅은 올림 바탕(빨강 아님) · 툴팁 「모든 창 숨기기」(결정 · 2026-10-24) ⑤ **스파이크 결과(사용자 손 실측 2026-10-24)를 접었다** — Q5 = ① Tauri `data-tauri-drag-region`(메인 결정 — 스파이크 실측 · 사용자 위임 · 갈래 ② 는 쓸 일이 없어 걷었다) · Q10(Alt+Space · 오른쪽 클릭 창 메뉴) = 재지 않음 — ① 이 주는 네이티브 동작을 그대로 받아들이고 QA 가 적는다 · tauri#10767 = 창 끌기 뒤 누름이 곧바로 먹히는지는 QA 몫(§1-3) ⑥ 리뷰 2 건(codex Designer FIX · Claude Architect-breaker FIX)을 반영했다 — 무엇을 고치고 무엇이 T1 로 없어졌나 = §15.
>
> **0.1판 — 첫 판**(2026-10-23 · 리뷰 전). 요구의 정본 = PRD [0.5판](../S21-window-chrome/prd.md)(R · AC · D · W 번호는 그 문서 것 — 이 판과 함께 0.5판으로 고쳤다 · §13). 이 문서는 그 요구를 **모듈 경계 · 인터페이스 · 차례 · 구현 단위**로 내린다(CLAUDE.md 「개발 스텝」 2 · 3).
>
> **표기** — 「(결정 · 2026-10-2x)」 = 사용자 결정 · 「(메인 결정)」 = 이 TRD 가 정했다(근거를 같이 적는다 · 사용자가 바꿀 수 있다) · 「미검」 = 코드 · 실측으로 확인하지 않았다 · 「(코드)」 = 2026-10-23/24 에 코드에서 확인했다(줄 번호 대신 심볼 · 파일로 가리킨다 — 줄은 밀린다).
>
> **스파이크 코드** — 작업 트리의 `SPIKE T-66` 표시 파일(`src/components/layout/T66TitleChrome.tsx` · `src-tauri/src/commands/t66_spike.rs` · `crates/engram-dashboard-platform/src/t66_snap_spike.rs` 와 그 배선 · 사본 = `.claude/handoff/attachments/t66-titlebar-spike-2026-10-23.patch`)은 **실현성 참고**로 읽었다. 제목 줄 · 끌 자리 몫은 버리고(U0), 스냅 오버레이 몫만 꺼진 feature 뒤로 옮겨 다시 짓는다(U9).

| 항목 | 값 |
|---|---|
| **상태** | 0.3판 — 0.2판(사용자 결정 T1 · T2 · T3 · Q1 · 스파이크 Q5 = ① · 리뷰 2 건) + 재리뷰 light FIX · 디버그 세션 실측 반영 · **사용자 결정 대기 없음** — 남은 것은 QA 실측 몫(§1-3 — S13 위 여백은 「사용자 확인 필요(QA)」 · §12-1) |
| **날짜** | 2026-10-24(0.1판 = 2026-10-23) |
| **스텝 · 브랜치** | S26 · `v0.5.0/feat/window-chrome` |
| **PRD** | [`docs/process/S21-window-chrome/prd.md`](../S21-window-chrome/prd.md) 0.5판(이 판과 함께 고침 — §13) |
| **근거 문서** | [custom-titlebar-tauri-2026-10-18](../../research/custom-titlebar-tauri-2026-10-18.md)(수단 · 피어 · 권한) · [window-identity-display-2026-10-23](../../research/window-identity-display-2026-10-23.md)(번호 표식 자리) · 슬롯 도킹 [TRD 7판](../S21-slot-docking/trd.md) §3-5 · §3-8 · §3-9 · 저장 [TRD](../S21-storage/trd.md) §6 · Chromium 소스 사본 `.claude/handoff/attachments/chromium-src-2026-10-18/`(끌 자리 최소 폭 — §7-6) |
| **관련 ADR** | 0026 · 0229(메인 닫기 = 모든 창 숨기기) · 0035 · 0057(레이아웃 · 탭 소유 = 셸) · 0066(포커스 슬롯) · 0081 · 0155(사람 · LLM 같은 적용 서비스 · 명령 표) · 0167(창 영속 id) · 0174(셸 단위 스위트 `lib_unit`) · 0227(측정 보고 — 정책 상수 정본 = 화면) · 0265(창 테마) · 0266 · 0275(OS 코드 = platform) · 0283(창은 `WebviewEnv::finish` 를 지난다) · 0296 · 0297(탭 끌기 = OS(HTML5) 끌기) · 0299(새 창 = 숨긴 예열 창 · 포커스를 빼앗지 않는다) |
| **리뷰 기록** | 0.1판 → codex(Designer) FIX · Claude(Architect-breaker) FIX → 0.2판 반영(§15) · 다음 = `/review trd` 재리뷰 |

---

## 0. 결론 요약

- **번호는 레이아웃 모델이 쥔다.** `ViewManager` 의 창 항목(`WindowTabs`)에 `number` 칸 · 모델에서 빠졌지만 OS 창이 아직 있는 창의 번호 표(`closing`) · 순수 함수 모듈 `layout/numbering.rs`(비어 있는 가장 작은 번호 · 저장 번호 고치기). 번호는 창을 모델에 넣는 그 락 안에서 받고 `Destroyed` 에서 푼다(D13 을 남긴다 — M2). **OS 창이 이미 없는 창**(복원 팝아웃을 못 열었거나 · 바꾸기 전에 사라진 창)은 `closing` 을 거치지 않고 곧바로 푼다(`drop_window_gone` — 리뷰 Claude ⑥).
- **탭 이름은 셸이 짓는다.** `View.name: String` 을 `TabName { Auto, Named(String) }` 로 바꿔 「정한 이름인가」를 타입으로 든다. 유효 이름 = 정한 이름, 아니면 포커스 슬롯의 표시 이름 — `layout/tab_name.rs` 의 `SlotContent` 위 와일드카드 없는 `match` 하나(D17). 낱말 = T2(셸 표 한 곳). 에이전트 이름은 모델 안 장부가 데몬 `AgentListUpdated` 로 받고, 유효 이름이 바뀌면 레이아웃 `version` 을 올린다(웹의 판 가드가 버리지 않게 — 리뷰 Claude ③).
- **OS 제목은 셸이 입힌다.** 창 탭 알림 페이로드(`WindowTabsPayload`)가 번호 · 영속 id · 제목 「<활성 탭 이름> - Engram:<n>」을 싣고, 그 알림의 Tauri 어댑터가 **메인 스레드에서 창별 마지막 판 번호보다 새 판일 때만** `set_title` 한다(늦게 온 옛 판은 버림 — 리뷰 codex ⑤). 불변식 = 「창 제목이 바뀌는 쓰기는 그 창의 탭 목록을 새 판 번호로 알린다」.
- **창 상태는 버스 명령이다.** `window.minimize` · `window.maximize` · `window.unmaximize` · `window.raise` · `window.hideAll` · `window.showAll` 을 새로 두고, `window.close` 는 main = 모든 창 숨기기 · `if_empty`(0탭 자가닫힘 재확인 — 공통 해석기보다 먼저 같은 락에서 가른다 · 리뷰 codex ②), `window.list` 는 옛 `windows` 칸을 두고 `details` 칸을 더한다. LLM 의 앞으로 내기 · 보이기는 **포커스를 빼앗지 않는다** — 이미 있는 `first_paint::raise_and_show` 차례(숨긴 채 z 올림 → Tauri `show` · `unminimize` → 전경이 우리일 때만 `set_focus` · Alt 합성 없음 · ADR-0299 · 리뷰 Claude ⑤ · 0.3판 재리뷰 ①)이고 트레이 보이기와 한 경로(`Activation::{TrayClick, NoSteal}`)다. 앞 창 판정은 platform 의 전경 창 비교(Tauri `is_focused` 를 Windows 에서 쓰지 않는다 · 리뷰 Claude ④). 창 상태 한 동작은 **메인 스레드 일 하나**(읽기 · 판정 · 미룸 · OS 호출)로 돌아 첫 화면 보이기와 끼어들지 않고, LLM 보이기는 첫 화면 대기 창을 건너뛴다(미룬 최대화 선례 — 리뷰 codex ⑥ · 0.3판 재리뷰 ② ③).
- **창 인자 = 영속 id · 같은 실행 안 label 별칭 · 번호 거부.** 해석기 하나(`layout/window_arg.rs`)를 버스 · invoke 껍데기가 같이 쓴다. 팝아웃 label 은 `slot-popup-<실행 표식 8 hex>-<n>` 로 바꿔 앞 실행 label 이 맞지 않게 한다(D12).
- **새 창 세 자리의 답 = `{ window, window_id, number }`**(invoke · 버스 · 웹 레지스트리 같은 셋 — R6). 첫 화면은 기다리지 않는다(T1).
- **제목 줄(웹)은 그리기만 한다.** `TitleRow` = 탭바(탭 · `+`) · 끌 빈 자리(최소 42 — Chromium `FrameGrabHandle`) · 번호 표식(끌 자리 · `pointer-events: none`) · 창 단추 셋. 단추는 레지스트리 명령을 부르고 Tauri 창 API 를 부르지 않는다. **탭 놓기 판정의 「탭바 끝」 구역을 제목 줄 전체로 넓힌다**(빈 곳 · 표식 · 단추 위 놓기 = 탭바 끝 · R14 · AC12 — 리뷰 codex ①). 창 사실(최대화 · 앞 창 · 크롬 모양)은 **기존 `window:frame` 길에 칸을 더해** 받는다(새 채널 없음 — 리뷰 codex ⑧). 끌 자리 = Tauri `data-tauri-drag-region`(Q5 — 스파이크에서 그 위로 끌어 온 탭 놓기가 닿았다) · Alt+Space · 오른쪽 클릭 창 메뉴는 그 기법이 주는 대로 받아들인다(Q10 — QA 가 적는다).
- **OS 틀은 `WebviewEnv::finish` 한 곳에서 걷는다**(`decorations(false)` · `shadow(true)`). 「단추를 우리가 그리나 · 왼쪽 비울 폭」은 platform `window::caption_style()` 한 곳이 답한다(R16 · D10). 최소 창 폭은 웹이 창마다 재서 보고하고 셸이 OS 최소 크기로 입힌다(보통 상태만 · 넓혀 당기기는 열 때만 — 순수 판정 `chrome::ChromeBook` · 리뷰 Claude ⑩ · codex ⑩).
- **구현 단위 = U0(스파이크 코드 걷기) · U1–U7(셸 · 모델 · 명령) · U8a · U8b(웹 제목 줄 → OS 틀 걷기) · U9(스냅 오버레이를 꺼진 feature 뒤로) · U10(문서).** 각 단위 뒤 빌드 · 회귀가 초록이고 앱이 쓸 수 있는 모습이다.

---

## 1. 전제 · 범위 · PRD 와의 관계

### 1-1 받는 결정(PRD §5-1 — 확정)

- 2026-10-23: Q2(팝아웃도 같은 제목 줄) · Q3(번호 표식 = 탭줄 오른쪽 끝 · 단추 바로 왼쪽 · 번호만 · 늘 · 메인 = 1 · 끌 자리 · OS 제목 「<탭> - Engram:<n>」) · Q8(번호를 영속 id 와 함께 저장) · Q9(명령은 번호를 창 인자로 받지 않는다) · Q7(메인 ✕ = 모든 창 숨기기 · 아이콘 · 툴팁으로 가름) · Q6(탭 이름 = 포커스 슬롯 표시 이름 · 정하면 굳음 · 옛 파일 칸 없음 = 자동) · Q4(창 이름 바꾸기 범위 밖) · 슬롯 도킹 실측 ② 만 한다(R25) · 리뷰 1 지적 ①(복원 중 잠깐 겹침 = 받아들인 위험) · ②(0탭 자가닫힘 재확인) · ⑥(빈 이름 저장 안 함).
- 2026-10-24: **T1**(첫 화면 전 실패를 따로 처리하지 않음) · **Q1**(스냅 메뉴 켜지 않음 · 코드는 꺼진 feature 뒤에 남김) · **T2**(낱말) · **T3**(main 셋째 단추 모양).

### 1-2 바꾸지 않는 것

- 레이아웃 · 창 · 탭 소유 = 셸(ADR-0035 · ADR-0057) · 사람과 LLM 은 같은 적용 서비스에 떨어진다(ADR-0081) · 명령 표 선언 = `src-tauri/src/layout/commands.rs` 의 `declare_commands!` 한 블록(ADR-0155).
- **탭 끌기 규칙 = OS(HTML5) 끌기 · 놓을 때 한 번 · 끄는 동안 셸과 맞추지 않는다**(ADR-0296 · ADR-0297 — R14 · R26–R28). 이 TRD 가 끌기 쪽에서 바꾸는 것은 **놓기 판정의 탭바 구역 넓이 하나**다(§7-9 — 0.1판의 「끌기 경로를 건드리지 않는다」는 틀렸다 · 리뷰 지적).
- 새 창 = 숨긴 예열 창을 먼저 맡는다(ADR-0299 · `layout::apply` 의 `new_window_label` · `withdraw_new_window` · `settle_new_window`). 첫 화면 문(`crate::first_paint`)의 알림 · 1.5 s 시한 보이기는 그대로다(T1).
- 메인 닫기 · 트레이 숨기기 = 한 경로(ADR-0229 · `tray::actions::hide_main_ui`).
- 화면 상태 파일 = `shell\state\state.json` 하나를 통째로 원자 쓰기(저장 TRD §6-4) · `STATE_VERSION` 은 올리지 않는다(선택 칸 더하기 — `state/schema.rs` 머리 「칸 더하기 규칙」).
- 레이아웃 락 규율 — 락 안 외부 호출 0 · 구독 재동기만 락 안(`layout::apply` 머리 「락 규율」) · 예열 창 차례 = 레이아웃 → 풀 → 첫 화면 문(`crate::spare::SpareState` · `crate::first_paint` 머리).

### 1-3 스파이크 결과 · 남은 실측

PRD 가 Q5 · Q10 을 메인에게 맡겼다(스파이크 뒤 결정 · 사용자에게 보고). 스파이크 코드는 끌기 영역 두 갈래(A = Tauri `data-tauri-drag-region` · B = CSS `app-region: drag`)를 실행 중에 바꾸고 끌기 영역 위 `dragover` · `drop` · `mousedown` · `mouseup` 을 셌다(코드 — `T66TitleChrome.tsx` 머리). **사용자가 2026-10-24 에 손으로 돌렸다**(아래 「결과」 칸 — 실측 = 사용자 관측 · 숫자 = 스파이크 세기 표시).

| # | 잴 것 | 걸린 요구 | 결과 · 남은 것 |
|---|---|---|---|
| S1 | 끌기 영역 위로 다른 창의 탭을 끌어 올 때 `dragover` · `drop` 이 오는가 | R14 · D9 · W2 · AC12 | **A 통과** — 팝아웃에서 끈 탭을 main 제목 줄 빈 곳에 놓으면 main 탭바에 들어갔다 · 끌기 영역에 `dragover` 13 번. B 는 재지 않았다(A 로 정해 쓸 일이 없다) → **Q5 = A**(§7-2) |
| S2 | 끌기 영역으로 창을 옮긴 직후 탭 누름 · HTML5 `dragstart` · 단추가 정상인가(tauri#10767) | W1 · AC13 | `mouseup` 이 누름 8 번에 4 번 왔다 — A 는 창 이동을 OS 이동 루프가 넘겨받으므로 예상한 값이다. **그 직후 누름이 곧바로 먹히는지는 재지 않았다 → QA**(AC13) |
| S3 | 끌기 영역 오른쪽 클릭 → 시스템 창 메뉴 | PRD §5-2 표 · Q10 | **재지 않음 → QA 가 적는다**. A 는 왼쪽 단추만 보므로(조사 · W6) 안 뜰 것으로 본다 — 받아들인다(Q10) |
| S4 | 터치 · 펜으로 끌기 | PRD §5-2 표 | 재지 않음(장치 없음) — 미검으로 남긴다 |
| S5 | 테두리 없는 창에서 Alt+Space 창 메뉴가 뜨는가 | Q10 · AC23 | **재지 않음 → QA 가 적는다** · 무엇이 나오든 A 가 주는 네이티브 동작을 받아들인다(Q10 ①) |
| S6 | 스냅 오버레이 — 최대화 단추에 올리면 메뉴가 뜨나 | Q1 · R15 | **떴다**(사용자 관측) — 그래도 켜지 않는다(Q1 결정) · 코드는 U9 가 꺼진 feature 뒤로 옮긴다 |
| S7 | Win+Z | R15 · AC16 | QA |
| S8 | 테두리 없는 창의 최대화 사각형 = 작업 영역인가(작업 표시줄을 덮지 않나) · OS 최소 크기가 최대화에 끼어드나 | AC14 · AC34 · R20 | QA(U8b) |
| S9 | 위 가장자리 크기 조절 띠 두께와 보통 상태 위 여백(끌 자리)의 겹침 | R12 · R25 · AC30 | QA(U8b) |
| S10 | `shadow(true)` 의 그림자 · 둥근 모서리 · 1 px 테두리 색(테마별) | R12 · AC18 | QA(U8b) |
| S11 | 기준 빌드(OS 제목 표시줄)의 제목 줄 높이 · 캡션 단추 폭 — 배율 100 · 150 · 200 % | D8 · AC18 · AC22 | QA(U8a 전 기준 빌드에서 떠 둔다) |
| S12 | 창 `Focused` 사건이 OS 활성 바뀜(Alt+Tab · 다른 창 누름 · 끌 자리 누름)마다 오는가 — 흐림의 계기 · 사건 시점의 `is_foreground` 도 함께 찍는다(§6-5) | R11 · AC15 | QA(U7a · U8a) |
| S13 | **접힌 끌기**(W13 — 0.3판) — 탭 줄이 창 맨 위에 붙은 테두리 없는 창에서 막 보인 팝아웃의 탭을 위로 빠르게 끌면 OS 끌기가 시작(누름 → dragstart 42–145 ms)되기 전에 커서가 창을 벗어나고, OS 끌기가 dragstart 약 6 ms 뒤 끝난다(dragend none · 단추는 아직 눌림) → 셸이 취소로 읽어 사용자에게는 아무 일도 없다 · 다시 하면 된다. 스파이크 세션에 18 번 · 스파이크 앞 기록에 0 번(스파이크 탓일 가능성 높음 · Win32/Blink 쪽 원인은 모름 — 후보: Blink 의 다시 적중 판정 · OLE `DoDragDrop` 의 단추 상태 · 위 가장자리 크기 조절 자식 창의 캡처) | R14 · AC11 | **측정 문(U8b)**: 빠른 위쪽 끌기 × 10 을 ① 위 여백 있음 ② 없음 ③ 보인 뒤 2 초 기다림 ④ master 기준 빌드로 — ① 이 ④ 와 같으면 통과. 원인을 가르기 전에 시한 · 문턱을 고치지 않는다(`docs/reference/debugging-conventions.md`) |
| S14 | **놓기 잃음**(W14 — 0.3판) — 위 가장자리 크기 조절 자식 창 · (켤 때) 스냅 오버레이 위에 놓은 탭이 받는 쪽 없이 끝나는가 | R14 · AC12 | **측정 문(U8b)** — 결과로 §7-9 의 ① · ② 를 고른다(M33) |

증거(S13 · ② 놓기 잃음) = 디버그 세션 2026-10-24 의 기록(`dump.json` · `rec2.jsonl` · `tl.log` — 세션 스크래치 · 저장소 밖 · 휘발 — 이 TRD 를 쓴 세션은 열어 보지 않았고 수치는 그 세션의 정리를 옮겼다).

**정지 조건으로 걸어 두었던 「S1 이 두 갈래 다 실패」는 풀렸다**(A 통과). 남은 정지 조건 = §14.

### 1-4 PRD 기본값(메인 제안)에서 벗어난 곳 — 보고

- **R4 「보이는 글자 · 빈 슬롯 낱말은 i18n 을 따른다」 → 셸 표 한 곳**(메인 결정 M4 · 낱말 = T2). 탭 유효 이름을 셸이 지어야 OS 제목 · `tab.list` · `window.list` 가 웹 없이 맞는다(§4-2). 셸에는 i18n 층이 없고 이미 사용자 낱말을 상수 표로 쥔 선례가 있다(`tray::core::MenuAction::label`). 로컬라이제이션(트래킹 T-68)이 그 표를 웹 i18n 과 합친다 — T-68 에 그 몫을 적는다(U10). 거부한 갈래 = 웹이 낱말을 셸에 보고(창 웹뷰가 하나도 보고하지 않은 동안 제목 · 명령 답이 낱말 없이 선다).

---

## 2. 모듈 경계 · seam

### 2-1 누가 무엇을 쥐나

| 것 | 주인 | 바꾸는 길 | 읽는 쪽 |
|---|---|---|---|
| 창 번호 · 닫히는 창 번호 | `ViewManager`(`layout/manager.rs`) — 순수 셈은 `layout/numbering.rs` | 창 항목을 넣고 빼는 `ViewManager` 메서드뿐 · 풀기 = `release_closing`(`Destroyed`) · `drop_window_gone`(OS 창이 이미 없음) | 탭 알림 페이로드 · `window.list` · 영속 |
| 탭 「정한 이름인가」 · 정한 이름 | `View.name: TabName`(`layout/types.rs`) | `rename_tab` · `create_tab(Some(name))` | 유효 이름 셈 |
| 탭 유효 이름(자동 이름 포함) | 파생 — `layout/tab_name.rs` | 저장하지 않는다(영속에는 정보용으로 그 순간 값을 적는다 — §3-3) | 탭 알림 · `tab.list` · OS 제목 |
| 에이전트 표시 이름 장부 | `ViewManager.agent_names`(파생 캐시 · 영속 안 함) | `apply::agent_names_updated`(데몬 `AgentListUpdated`) · 부팅 시드(§4-3) | 유효 이름 셈 |
| OS 창 제목 | OS 창 — 값의 정본은 탭 알림 페이로드 `title` · 판 = 페이로드 `version` | `LayoutEvents` 의 Tauri 어댑터가 메인 스레드에서 새 판만 `set_title`(마지막 판 = `chrome::ChromeBook`) | 작업 표시줄 · Alt+Tab |
| 창 OS 상태(최대화 · 최소화 · 보임 · 앞 창) | OS 창 | 버스 · invoke → `window_ctl` → `WindowControl` 포트 | `window.list` · `window:frame` |
| 크롬 모양(단추를 그리나 · 왼쪽 비울 폭) | platform `window::caption_style()` | 없음(OS 상수) | 셸 `WebviewEnv::finish` · 웹(`window:frame` 의 칸) |
| 창 사실(복원 크기 · 최대화 · 틀 · **앞 창 · 크롬 모양**) | 셸 `tab_drop::FrameBook`(기존 — 슬롯 도킹 TRD §3-8) | 자리 기록(`state::placement::record`) · **`Focused` 사건**이 `push_window_frame` 을 부른다 | 웹 `createWindowFrame`(구독 먼저 · 당기기 · 사건 우선 가드 — 기존) |
| 최소 창 폭 | 웹(정책 상수 정본 = 화면 — ADR-0227 선례) | 웹이 창마다 보고(`report_window_min_width`) | 셸 `chrome::ChromeBook` 이 정하고 어댑터가 OS 최소 크기로 입힘 |
| 제목 줄 그림 · 끌 자리 표시 · 흐림 | 웹(렌더만) | — | — |

### 2-2 새 파일 · 바뀌는 파일

| 파일 | 새/바뀜 | 하는 일 | Tauri 의존 |
|---|---|---|---|
| `src-tauri/src/layout/numbering.rs` | 새 | `smallest_free` · `resolve_saved`(D3) · 상한 `MAX_SAVED_NUMBER` | 0(순수) |
| `src-tauri/src/layout/tab_name.rs` | 새 | U1 = `slot_label`(D17 — 와일드카드 없는 `match`) · `window_title` · 낱말 표 `word` / U3a = `effective_tab_name` | 0(순수) |
| `src-tauri/src/layout/window_arg.rs` | 새 | `resolve(&ViewManager, &str) -> Result<WindowLabel, WindowArgError>` | 0 |
| `src-tauri/src/layout/window_ctl.rs` | 새 | 창 상태 서비스(최소화 · 최대화 · 원래 크기 · 앞으로 · 숨기기 · 보이기 · 닫기 합류 · 비었으면 닫기) · 포트 `WindowControl` | 0 |
| `src-tauri/src/chrome/book.rs` | 새 | `ChromeBook` — 순수 판정(창별 마지막 제목 판 · 열기 표시 · 최소 크기 켜기/걷기 · 넓혀 당기기 여부) · 단위 시험 | 0(순수) |
| `src-tauri/src/chrome/mod.rs` | 새 | `ChromeBook` 을 잎 락으로 쥔 Tauri 어댑터 — 제목 입히기(메인 스레드) · 최소 크기 입히기 · `Destroyed` 때 잊기 | Tauri |
| `src-tauri/src/commands/window_ctl.rs` | 새 | `WindowControl` 의 Tauri 어댑터 · invoke `window_control` · `report_window_min_width` | Tauri |
| `src-tauri/src/layout/{types,manager,apply,commands}.rs` | 바뀜 | `TabName` · `WindowTabs.number` · `closing` · `agent_names` · `drop_window_gone` · 페이로드 칸 · 새 명령 · 만들기 답 | — |
| `src-tauri/src/state/{schema,codec,convert,placement,restore}.rs` | 바뀜 | 영속 칸 둘(`number` · `named`) · D3 고치기 · 이름 시드 · OS 창이 없는 창은 `drop_window_gone` · `fit_min_width` | — |
| `src-tauri/src/tab_drop/{wire,mod}.rs` · `src-tauri/src/commands/tab_drop.rs` | 바뀜 | `WindowFrame` 에 `focused` · `caption` 칸 · 앞 창 읽기(platform) | — / Tauri |
| `src-tauri/src/commands/popout.rs` | 바뀜 | label 꼴(D12) · `cascade_position` 풀이 · 빌드 제목 「Engram」 · `LabelSource::tab_name` 걷기 · `drop_window_in_model` 의 원인 인자 | Tauri |
| `src-tauri/src/first_paint.rs` · `src-tauri/src/state/placement.rs` | 바뀜 | 첫 화면 대기 여부 조회(`is_held`) · 모든 보이기 갈래가 미룬 최대화를 입힌다(§5-2) | Tauri |
| `src-tauri/src/tray/actions.rs` | 바뀜 | 보이기에 「포커스 빼앗지 않기」 갈래(LLM 길 · §6-2) | Tauri |
| `src-tauri/src/daemon_client/{connection,mod}.rs` | 바뀜 | 포트 `AgentNamesSink`(emit 포트 `DaemonEvents` 를 넓히지 않는다 — 그 파일 머리 「넓히기 금지」) | — |
| `crates/engram-dashboard-daemon/src/connection_core.rs` | 바뀜 | `Renamed` 뒤 · `RosterFanout::roster_changed` 도 `AgentListUpdated` 를 낸다(§4-3) | — |
| `crates/engram-dashboard-platform/src/window.rs` | 바뀜 | `caption_style()` · `is_foreground(w)` · (필요하면) 활성화 없이 펴기(§6-2) · `snap` 모듈(feature `snap-overlay` 뒤 — §8-4) | — |
| `crates/engram-dashboard-platform/Cargo.toml` · `src-tauri/Cargo.toml` | 바뀜 | feature `snap-overlay`(기본 꺼짐) — 셸 feature 는 platform feature 를 켠다 | — |
| `src-tauri/src/webview_env.rs` | 바뀜 | `finish` 가 `decorations(false)` · `shadow(true)` 를 붙인다(`EnvOptions` 로 — 시험이 본다) | Tauri |
| `src-tauri/src/lib.rs` | 바뀜 | `Focused` → `push_window_frame` · `Resized` → 최소 크기 판정 · `Destroyed` → 번호 풀기 · 크롬 잊기 · invoke 등록 | Tauri |
| `src/components/layout/titleRow/{TitleRow,NumberMarker,CaptionButtons}.tsx` · `titleRow/metrics.ts` · `titleRow/dragRegion.ts` | 새 | 제목 줄 렌더 · 크기 상수 · 최소 폭 셈 · 끌 자리 속성 | — |
| `src/components/layout/{TabBar,WindowLayout}.tsx` · `tabStrip/useTabDragSource.ts` · `dock/windowFrame.ts` · `src/store/viewStore.ts` · `src/commands/tabCommands.ts` · `src/i18n/ko.ts` · `src/styles/theme.css` | 바뀜 | 제목 줄 끼우기 · 놓기 구역 넓히기 · 창 사실 칸 · 0탭 자가닫힘 → 버스 · 창 명령 등록 · 이름 편집 확정 · 낱말 · 토큰 | — |
| `src-tauri/capabilities/{default,popup}.json` | 바뀜 | §9 | — |

### 2-3 포트(seam) — 바깥 세계마다 하나

| 포트 | 위치 | 새/바뀜 | 실물 | 하네스 |
|---|---|---|---|---|
| `WindowHost` | `layout::apply` | 그대로 | `commands::popout::TauriWindowHost` | 기존 |
| `LabelSource` | `layout::apply` | 바뀜 — `tab_name` 걷기(자동 이름이 대신) | `PopupCounter` | 기존 |
| `LayoutEvents` | `layout::apply` | 그대로(페이로드 칸만 는다) — Tauri 어댑터가 `title` 을 판 가드와 함께 OS 창에 입힌다 | `commands::layout::TauriEvents` | 기록형 가짜가 `title` · `version` 을 단언 |
| `WindowControl` | `layout::window_ctl` | 새 — 창 OS 상태 조작 · 읽기 | `commands::window_ctl::TauriWindowControl` | 기록형 가짜(부른 차례 · 꾸민 상태 · 첫 화면 대기 · 앞 창 잠김 꾸밈) |
| `AgentNamesSink` | `daemon_client` | 새 — `AgentListUpdated` 의 이름 몫 | 셸 조립(`lib.rs`)이 `apply::agent_names_updated` 를 부르는 구현 | 기록형 가짜(`daemon_client/tests.rs` 의 실 소켓 왕복 옆) |

**Tauri 0 경계** — `numbering` · `tab_name` · `window_arg` · `window_ctl` · `chrome::book` 은 `tauri` 를 import 하지 않는다(`layout::apply` 와 같은 규율 — 창 없이 `lib_unit` · `layout_apply` 가 잰다).

**시험 몫 도우미**(리뷰 Claude ⑨ — 메인 결정 M20) — 셸 패키지에 시험 feature 를 새로 만들지 않는다. 내부 상태를 들여다보는 도우미(`ViewManager::closing_numbers` 등)는 `#[cfg(test)]` 로만 두고, 그것을 쓰는 시험은 모듈 옆 단위 시험(`--test lib_unit` — 셸 `src/**` 를 `cfg(test)` 로 굽는다 · ADR-0174)에 둔다. 통합 타깃 `layout_apply` · `layout_commands` 는 공개 API + 가짜 포트만 쓴다. 「모델에서만 빠지고 OS 창은 남은 창」(0탭 자가닫힘의 상태)은 도우미 없이 선다 — 가짜 `WindowHost::close` 가 아무것도 하지 않으면 평범한 닫기가 그 상태를 만든다. 그래서 운영 바이너리에 시험 갈래가 들어갈 자리가 없고, 그 상태를 GUI 에서 꾸미는 길도 두지 않는다(AC24 의 0탭 갈래 · AC32 · AC20 ② = 단위 하네스 — §13 · PRD 0.5판 AC24 · AC32).

### 2-4 경계 불변식(이 TRD 가 더하는 것)

1. **번호 셈은 `ViewManager` 메서드 안에서만 한다** — 창 항목을 넣는 길(`create_window` · `attach_view_as_new_window` · `dock` 의 새 창 · `from_restored` · `adopt_restored`)과 빼는 길(`pull_tab` 의 팝아웃 비움 · `close_window` · `drop_window_gone` · `adopt_restored`)은 모두 사설 메서드 둘(`insert_window_entry` · `retire_window_entry(label, Os::{Alive, Gone})`)을 지난다. `windows` 칸을 직접 `insert` · `remove` 하는 새 코드를 쓰지 않는다(그 칸이 `pub` 인 것은 시험 · 영속 변환의 읽기 때문 — 쓰기는 이 규칙으로 막고 시험이 맞댄다 §10-1).
2. **창 제목이 바뀌는 쓰기는 그 창의 탭 목록을 새 판 번호로 알린다** — 활성 탭 · 탭 유효 이름 · 번호 중 하나라도 바뀌면 `version` 을 올리고 그 창의 `WindowTabsPayload` 를 낸다(`announce`). 오늘 탭 알림을 내지 않는 쓰기 중 제목을 바꾸는 것 = `create_window`(새 창) · 에이전트 이름 바뀜(`version` 도 안 올린다) · 부팅 — 셋을 고친다(§4-3 · §4-4). 슬롯 쓰기(`split_slot` · `close_slot` · `focus_slot` · `assign_agent` · `set_slot_content` …)는 이미 `owner_tabs` 를 싣고 `version` 을 올린다(코드 — `layout::apply` 의 그 함수들 · 전수 대조는 U4 몫 — 미검).
3. **웹은 Tauri 창 API 의 최소화 · 최대화 · 닫기 · 보이기 · 앞으로 내기를 부르지 않는다**(R17 · AC10) — 타입 검사 시험 §10-4.
4. **`slot_label` 에 와일드카드 · 묶음 이름 갈래를 두지 않는다**(D17) — 함수에 `#[deny(clippy::wildcard_enum_match_arm)]` + 그 파일 rg 게이트(§10-4 — CI 가 clippy 를 돌리지 않아서).
5. **첫 화면 대기 창(첫 화면 문에 올려진 창)을 드러내는 것은 첫 화면 문과 사람의 트레이 보이기뿐이다** — 창 상태 명령과 LLM 의 보이기(`window.showAll` · 숨은 창의 `window.raise` · invoke `show_main_ui`)는 그 창을 건너뛴다. 트레이 누름 보이기는 지금처럼 그 기다림을 거두고 보인다(코드 — `tray::actions::show_main_ui` 의 `cancel_hold` → `show()` · 사람이 누른 보이기다 — 0.3판 재리뷰 ③ · §5-2).

---

## 3. 데이터 모델 · wire · 영속

### 3-1 메모리 모델(`src-tauri/src/layout/`)

```rust
// types.rs
pub enum TabName { Auto, Named(String) }      // ts-rs export
pub struct View { pub id: Uuid, pub name: TabName, pub layout: LayoutNode, pub focused_slot_id: Option<Uuid> }
pub struct ViewMeta { pub id: Uuid, pub name: String /* 유효 이름 */, pub named: bool }

// manager.rs
pub struct WindowTabs { /* 기존 칸 */ pub number: u32, /* … */ }
pub struct ViewManager {
    /* 기존 칸 */
    closing: HashMap<WindowLabel, Closing>,          // 모델에서 빠졌고 OS 창이 아직 있다 — D13
    agent_names: HashMap<String, String>,            // agent_id → 표시 이름(파생 캐시)
}
struct Closing { window_id: String, number: u32 }
```

- **`View.name: String` → `TabName`**(메인 결정 M4′) — 이름을 정했나를 타입이 든다. 옛 `name` 을 직접 읽던 자리(`list_tabs` 의 `ViewMeta` 짓기 · `convert::tab_entry` 등)는 컴파일이 깨져 전부 유효 이름 셈으로 옮겨진다. 그대로 `String` 에 「정한 이름인가」 칸을 덧대는 갈래는 거부했다 — 자동 탭의 낡은 문자열(「View 1」)을 누가 바로 읽으면 AC3 이 깨진다.
- **기본 이름 짓기를 걷는다** — `make_view(name)` → `make_view()`(`TabName::Auto`) · `create_tab(label, name)` 은 `Some` = `Named` · `None` = `Auto` · `refill_main` · `from_restored` 의 빈 main · `create_window` · `prepare_detached_view` 의 이름 인자 · `apply::move_slot_to_window` 의 `"Tab"` · `LabelSource::tab_name`(「Popup N」 — `commands::popout::PopupCounter`)을 모두 걷는다(코드 — 그 자리들에 지금 「View N」 · 「View 1」 · 「Tab」 · 「Popup N」 이 있다).
- **`rename_tab` 은 빈 이름 · 공백뿐인 이름을 거절한다**(모델 쪽 방어 — `LayoutError::EmptyName`). 지금은 「정규화는 경계 몫」이라 받은 문자열을 그대로 쓴다(`ViewManager::rename_tab` 주석). 버스(`commands::text`)와 화면(`TabBar.tsx` `commitEdit` · 레지스트리 `tab.rename`)은 이미 거르므로 결과는 같고, 앞으로 새 경계가 생겨도 「정한 이름」이 빈 글자로 굳지 않는다(AC36).
- **`rename_tab` 은 지금 유효 이름과 같은 글자여도 `Named` 로 굳힌다** — 자동 이름 「myproj」 탭을 사람이 「myproj」로 「바꾸면」 그 이름을 정한 것이다(그 뒤 에이전트 이름이 바뀌어도 따라가지 않는다). 웹 `TabBar.tsx` `commitEdit` 은 지금 `trimmed !== tab.name` 일 때만 보낸다(코드) — `tab.named` 가 거짓이면 같은 글자도 보내게 고친다(U3a · 리뷰 codex ⑨). 같은 글자는 Enter(일부러 한 확정)에서만 보내고, 포커스 잃음 · 덮인 이웃 누름은 바뀐 글자만 보낸다 — 메인 결정(U3a 리뷰 · 2026-10-25).
- **`WindowTabs::new` 가 `number` 를 인자로 받는다** — 새 창 항목은 전부 그 생성자를 지나므로(코드 — 그 함수 주석) 번호 없는 항목이 생기지 않는다.

### 3-2 wire

| 표면 | 바뀜 | 비고 |
|---|---|---|
| `WindowTabsPayload`(`window:tabs-updated` · invoke `list_tabs` 답 — `layout::apply`) | `window_id: String` · `number: u32` · `title: String` 더함 · `tabs[].named` | 웹은 `label` 으로 거르고 `version` 이 지난 것보다 크지 않으면 버린다(코드 — `viewStore.ts` 의 `payload.version <= prev.version`) · `title` 은 웹이 쓰지 않는다(셸 어댑터 몫 — 하네스가 단언하는 재료) |
| `ViewMeta`(ts-rs) | `name` = 유효 이름 · `named: bool` 더함 | `src-tauri/bindings/` 다시 구움 → CI 생성물 sync 게이트 |
| `View`(ts-rs) | `name: TabName` | 웹 사용처 없음(코드 — `src/api/layoutTypes.ts` 가 다시 내보내기만 하고 가져다 쓰는 곳 0). 영속 모양과 「같은 JSON」 규칙은 이름 칸만 깨진다 — §3-3 |
| `WindowFrame`(`window:frame` · invoke `window_frame` — `tab_drop/wire.rs`) | `focused: bool` · `caption: CaptionChrome { buttons: bool, leading_inset: f64, snap_overlay: bool }` 더함 | 기존 길 그대로(구독 먼저 · 당기기 · 사건이 오면 당기기 답을 버림 — `windowFrame.ts` `createWindowFrame`). `snap_overlay` = 셸이 feature `snap-overlay` 로 지어졌나(늘 거짓 — §8-4) |
| invoke `create_window` 답 | `String` → `NewWindow { window, window_id, number }` | 웹 `viewStore.createWindow` · 레지스트리 `window.create` 가 그 셋을 돌려준다(R6) |
| invoke `move_slot_to_window` 답(`SlotMove`) | `window_id` · `number` 더함 | 기존 창 대상이면 그 창의 것 |
| invoke `tab_drop` 의 답 `TabDropReply`(`applied` — 셸 안 `DockOutcome.identity` 를 옮긴다) | `applied` 에 `window_id` · `number` 더함(둘 다 null 가능) — 새 창이든 기존 창(이음새 · 탭바 · 재사용)이든 옮긴 탭이 든 창의 것 · null = 재사용 길에서 옮기는 사이 그 창이 닫힘 | ts-rs `TabDropReply` 다시 구움 · 웹은 아직 읽지 않는다 · 앞으로의 `tab.dock`(T-65)이 쓴다 |
| invoke `window_control`(새) · `report_window_min_width`(새) | §6-4 · §7-7 | ts-rs export |
| 버스 `tab.list` | `TabRow.named` 더함 | 선언 세대 올림(§6-6) |
| 버스 `window.create` · `slot.popout` · `window.list` · `window.close` · 새 명령 | §6 | |

### 3-3 영속(`shell\state\state.json` — `state/schema.rs` · `codec.rs` · `convert.rs`)

- **`WindowEntry.number: Option<u32>`** — 쓰기는 늘 값(main 도 1 을 적는다 — 읽기는 main 값을 보지 않는다 · D3). 읽기(`codec::WindowHead`)는 `#[serde(default)] number: serde_json::Value` 로 받아 **너그럽게** 푼다: 정수 `1..=MAX_SAVED_NUMBER` 만 `Some`. 칸 없음 · `null` = 경고 없이 `None` · 그 밖의 못 쓰는 값(0 · 음수 · 실수 · 문자열 · 불 · 상한 넘음) = `None` + 경고 `DecodeWarning::NumberDropped`(원문은 앞 64 글자까지) · main 의 값은 보지 않고 알리지도 않는다. 번호를 다시 매긴 사실은 복원 경고 `NumberReassigned` 가 알린다. ★창을 건너뛰지 않는다★ — 번호가 이상하다고 창의 탭을 잃으면 안 된다(AC25 · AC38).
- **`TabEntry.named: Option<bool>`** — 쓰기는 늘 값. 읽기는 `#[serde(default, deserialize_with = "lenient_bool")]` — 칸 없음 · `null` · 불이 아닌 값 = `None`. `None` · `Some(false)` = 자동(D16 — 옮기는 규칙 없음), `Some(true)` = `TabName::Named(name)`. 불이 아닌 값으로 탭째 건너뛰지 않는다.
- **`TabEntry.name`** — 쓰기 = 그 순간의 **유효 이름**. 에이전트 이름 바뀜도 `version` 을 올리므로(§4-3) 기록기가 다시 써 파일의 이름이 늘 지금 값이다. 옛 빌드가 이 파일을 읽으면 그 이름을 그대로 보인다(내림은 지원하지 않지만 보기에 낫다 — 저장 TRD §12 R3). 이 판은 `named != Some(true)` 인 탭의 `name` 을 이름으로 쓰지 않는다 — 에이전트 이름 장부 시드에만 쓴다(§4-3).
- **「같은 JSON」 규칙** — `state/schema.rs` 머리는 `TabEntry` 와 `layout::View` 가 같은 JSON 이라고 적고 시험(`view_shapes`)이 맞댄다. 이름 칸이 갈라지므로 그 머리 · 시험을 「이름 칸만 `convert` 가 풀어 옮긴다」로 고친다(U3a). 나머지 칸은 그대로 맞댄다.
- **`STATE_VERSION` 은 그대로 1** — 둘 다 빠져도 읽히는 선택 칸이다. 옛 빌드가 다시 저장하면 두 칸이 사라진다 = 다음 이 판 부팅에서 번호는 메인부터 저장 순서대로 다시 매겨지고(AC25 의 「옛 파일」 규칙) 정한 이름은 자동으로 돌아간다. 받아들인다(저장 TRD §12 R3).

### 3-4 저장 번호 고치기(D3 — `numbering::resolve_saved`)

입력 = 저장 순서의 팝아웃 번호들(`Option<u32>` — 위 너그러운 읽기 뒤) · 이미 쥔 번호 집합 `held`(늘 main 의 1 포함). 출력 = 같은 길이의 번호.

1. 팝아웃이 적은 1 = 없음(메인 몫).
2. 같은 번호를 둘 이상 적었으면 저장 순서에서 앞 창만 갖고 나머지는 없음.
3. `held` 에 든 번호(그때 닫히는 중이 아닌 다른 창이 쥔 것) = 없음 — **이미 쥔 창이 지킨다**(D2 · D3).
4. 바른 번호를 먼저 다 세운 뒤, 없음인 창이 저장 순서대로 비어 있는 가장 작은 번호를 받는다.

`MAX_SAVED_NUMBER = 999`(메인 결정 M3) — 세 자리까지 보이는 꼴을 AC38 이 잰다. 실행 중 새 창은 상한을 보지 않는다. 경고 `RestoreWarning::NumberReassigned { window_id, saved, given }` 을 로그로 남긴다.

### 3-5 스냅숏 일관성(R29 · AC37)

번호 · 「정한 이름인가」 · 이름 · 영속 id 는 같은 `StateFile` 의 같은 창 항목 · 탭 항목에 든다 — 따로 떼지 않는다. 기록기는 레이아웃 `version` 이 바뀌면 통째로 다시 쓴다(저장 TRD §6-4). 번호는 창 항목을 넣고 빼는 쓰기에서만, 「정한 이름」은 `rename_tab` · `create_tab` 에서만 바뀌고 그 쓰기들은 이미 `version` 을 올린다. 자동 이름의 바뀜은 포커스 쓰기(이미 올린다)와 에이전트 이름 바뀜(§4-3 이 올리게 한다) 두 길뿐이다 — 그래서 파일의 `name` 은 늘 그 판의 유효 이름이고, 부팅 시드(§4-3)가 읽는 값과 모순이 없다(0.1판 §3-5 의 「한 박자 낡을 수 있다」는 걷었다 — 리뷰 지적).

---

## 4. 번호 · 이름 · 제목 규칙

### 4-1 번호 수명(R1 · D1 · D2 · D13)

| 사건 | 모델이 하는 일 | 자리 |
|---|---|---|
| 부팅 기본 · 부팅 복원 | main = 1 · 팝아웃 = `resolve_saved(저장값, {1})` | `ViewManager::new` · `from_restored` · `convert::Restorer` |
| 새 창 항목 넣기(세 자리 · 예열 창이든 새로 지은 창이든) | `number = smallest_free(살아 있는 번호 ∪ closing 번호)` — 넣는 그 락 안 | `insert_window_entry` |
| 팝아웃 항목 빼기 — OS 창이 살아 있다(마지막 탭 닫기 · 도킹으로 비움 · `window.close` · 닫기 요청) | 그 번호를 `closing[label] = { window_id, number }` 로 옮긴다 | `retire_window_entry(label, Os::Alive)` |
| OS 창 `Destroyed` | `closing.remove(label)` — 없으면 무동작(예열 창 · 이미 푼 창). 그때 아직 모델에 있던 창이면 `Os::Gone` 으로 빼 곧바로 푼다 | `release_closing` · `drop_window_in_model(label, DropCause::Destroyed)` 같은 락 안 |
| **팝아웃 항목 빼기 — OS 창이 이미 없다**(부팅 복원 팝아웃을 못 열었다 — `state::placement::open_restored_popouts` · 사본 수락 뒤 바꾸기 전에 사라졌다 — `state::restore` `forget_vanished`) | `closing` 을 거치지 않고 곧바로 뺀다 — 그 창의 `Destroyed` 가 이미 지나갔거나 오지 않는다(리뷰 Claude ⑥ — 0.1판은 그 둘이 `close_window` 를 불러 번호가 실행 끝까지 잡혔다) | `drop_window_gone`(= `retire_window_entry(label, Os::Gone)`) |
| 사본 수락 | 옛 팝아웃(살아 있는 것 · `closing` 전부)의 번호를 다 풀고 사본 번호를 `resolve_saved(사본값, 지금 쥔 번호)` 로 세운다 — `held` = main 의 1(아래 주) | `adopt_restored` |
| 예열 창이 맡겨지기 전 · 되물림 | 모델 밖 — 번호 없음 | — |
| 열기 실패 · 넣기 거절(연 창을 거둠) | 번호를 잡은 적이 없다 — 풀 것 없음(PRD R1 「만들기 실패」) | — |

- **D13 을 남긴다**(메인 결정 M2) — 값 = 모델 안 표 하나(`closing`) + 넣고 빼는 사설 메서드 둘 + `Destroyed` 정리의 한 줄. 걷으면 보통 닫기 뒤 새 창이 닫히는 창과 같은 번호를 잠깐 보이고 AC20 ② 를 걷어야 한다.
- **`Destroyed` 가 끝내 안 오면** 그 번호는 실행 끝까지 잡힌다(W7 — 받아들임). `closing` 은 영속하지 않는다.
- **`drop_window_in_model` 의 원인** — 닫기 요청과 `Destroyed` 가 둘 다 그 함수를 부른다(코드 — `commands::popout`). 인자 `cause: DropCause { CloseRequested, Destroyed }` 를 더해 모델에서 빼는 몫은 `CloseRequested` = `Os::Alive` · `Destroyed` = `Os::Gone` 으로, `Destroyed` 면 끝에 `release_closing` 까지 한다(같은 락 안 — 그 함수가 이미 묘비 · 구독 정리를 락 안에서 한다). `output_router::cleanup_window_core` 가 그 안에서 `close_window` 를 부르는 길은 원인을 넘겨받게 고친다.
- **`Destroyed` 정리를 못 부르는 갈래도 번호를 푼다**(0.3판 재리뷰 잔손 · U2 리뷰가 자리를 고침) — `lib.rs` 의 `Destroyed` 갈래는 `LayoutState` · 라우터 · 채널 명부 · 데몬 클라이언트 넷이 다 있을 때만 `cleanup_popup_window` 를 부른다(코드 — 「하나라도 없으면 조용히 스킵」). 넷이 다 있으면 번호는 그 안의 `drop_window_in_model(Destroyed)` 가 한 락 안에서 푼다 — 빼기 · 묘비 · 구독 정리가 같은 락에 남고 지운 탭 수 로그가 뜻을 지킨다(번호 풀기를 따로 앞에 두면 모델 창이 그 앞 락에서 묘비 없이 빠지고 그 로그는 늘 0 이다). 하나라도 없으면 그 조건문의 `else` 갈래가 `LayoutState` 하나만으로 `layout::release_window_number(&state, label)`(= 락 → `release_closing` → 아직 모델에 있으면 `Os::Gone` 으로 빼기)를 부른다 — 안 부르면 번호가 실행 끝까지 잡힌다.
- **사본 수락의 `held`** — `adopt_restored` 는 main 이 아닌 모든 창 항목을 같은 락 안에서 걷는다(코드). 그래서 커밋 순간 「닫히는 중이 아닌 다른 창」은 main 뿐이고, D3 의 「이미 쥔 창이 지킨다」 갈래는 이 차례에서 일어나지 않는다. 일반 규칙으로 짜 두고(입력에 `held` 를 받는다) 순수 시험으로 잰다 — PRD 0.5판 AC31 에 그 사실을 적었다(§13).
- **부팅 복원** — 모델은 플러그인 초기화(`state::boot_plugin`)가 setup 앞에서 세운다(가능성 높음 — 버스 연결이 그 뒤라는 것은 미검). 그러면 부팅 중 `window.create` 는 늘 복원 창 뒤에 넣어져 빈 번호를 받는다(AC31 ①).

### 4-2 탭 유효 이름(R4 · D17 · AC4 · AC35 · AC36)

```rust
// tab_name.rs — 순수. U1 이 넣는 몫(View · ViewManager 를 모른다 — U1 이 홀로 빌드된다)
#[deny(clippy::wildcard_enum_match_arm)]
pub fn slot_label<'n>(content: &SlotContent, agent: impl for<'a> Fn(&'a str) -> Option<&'n str>) -> String {
    match content {
        SlotContent::Empty => word(Word::EmptySlot).to_owned(),
        SlotContent::Agent { agent_id } => agent(agent_id).unwrap_or(word(Word::AgentUnknown)).to_owned(),
        SlotContent::AgentList => word(Word::AgentTree).to_owned(),
        SlotContent::PresetPalette => word(Word::Presets).to_owned(),
        SlotContent::Usage { show_claude: _, show_codex: _ } => word(Word::Usage).to_owned(),
    }
}
pub fn window_title(active_tab: Option<&str>, number: Option<u32>) -> String

// U3a 가 더하는 몫(TabName · ViewManager 가 선 뒤)
pub fn effective_tab_name(mgr: &ViewManager, view: &View) -> String  // Named → 그 이름 · Auto → 포커스 슬롯(없거나 트리에 없으면 트리 첫 슬롯)의 이름 · 모르는 내용 슬롯 = Word::Foreign
```

- **U1 은 홀로 빌드된다**(리뷰 codex ⑦) — `slot_label` · `window_title` · `word` 는 `SlotContent` 하나만 안다. `effective_tab_name` 과 빈 이름 장부(`agent_names`)는 `TabName` · `ViewManager` 칸이 서는 U3a 로 옮겼다.
- **포커스 슬롯** = `View.focused_slot_id`(ADR-0066 · 탭마다 · 백엔드 소유). 슬롯이 없어지면 `ViewManager::fixup_focus` 가 트리 첫 슬롯으로 옮긴다(코드). 유효 이름 셈은 같은 규칙을 다시 적용한다(방어).
- **포커스가 옮는 길과 낱말이 탭 이름이 되는 때**(리뷰 Claude ⑪ — 0.1판 §4-2 · §13 ④ 의 사실 오류를 고침) — 포커스를 옮기는 쓰기는 `apply::focus_slot`(사람 클릭 · `slot.focus`) · `split_slot`(새 칸으로 — 코드 `ViewManager::split_slot`) · `fixup_focus` 이고, **`set_slot_content` 는 포커스를 옮기지 않는다**(코드). 새 탭 · 새 창의 하나뿐인 빈 칸은 처음부터 포커스 슬롯이다(코드 — `ViewManager::make_view`). 그래서 그 칸에 에이전트 트리 · 프리셋 · 사용량을 놓으면 탭 이름은 곧바로 그 낱말이 된다. 웹은 내용 슬롯(빈 칸 · 에이전트)만 클릭으로 포커스하므로(코드 — `LayoutLeaf.tsx` 의 `isContentSlot` · ADR-0066) 그 세 종류 슬롯은 **누름으로는** 포커스를 받지 않는다 — 포커스가 이미 그 칸에 있었던 경우에만 그 낱말이 탭 이름이다. PRD R4 는 이 점을 잘못 적지 않았다(고칠 문장 없음).
- **모르는 내용 슬롯**(이 판이 모르는 종류 — 메모리 내용은 `Empty`, 곁표 `unknown_content` 에 원문 · 저장 TRD §6-2) — `effective_tab_name` 이 `match` 앞에서 곁표를 보고 「알 수 없는 내용」(`Word::Foreign`)을 준다. **이것은 `SlotContent` 의 갈래가 아니다** — `match` 는 와일드카드 없이 다섯 변형만 다룬다. 낱말을 남긴 까닭(메인 결정 — 사용자 T2 의 「필요하면만」): 새 판이 저장한 슬롯은 「점유」인데 `slot_label` 에 넘기면 「빈 칸」이 되어 거짓을 말한다. 낱말은 웹이 이미 그 슬롯에 쓰는 것과 같다(코드 — `ko.ts` `slot.foreignContent`).
- **낱말 표 `word`**(결정 · 2026-10-24 — T2) — 셸 한 곳(`tab_name.rs` 의 `const fn word(Word) -> &'static str`): 빈 칸 「빈 칸」 · 이름 모르는 에이전트 「에이전트」 · 에이전트 목록 「에이전트 트리」 · 프리셋 팔레트 「프리셋」 · 사용량 「사용량」 · (모르는 내용 「알 수 없는 내용」 — 위).
- **빈 이름**(AC36) — 버스 `tab.rename` 의 `""` · `"   "` = `INVALID_ARGUMENT`(코드 — `commands::text`) · 화면 = 무시(코드 — `TabBar.tsx` `commitEdit`) · 모델 = 거절(§3-1). 어느 길도 `TabName` 을 바꾸지 않는다.

### 4-3 에이전트 이름 장부

- **출처 = 데몬 `AgentListUpdated` 의 `AgentInfo.name`**(protocol — canonical 표시 이름 = `display_name ?? basename(cwd)` · 이름 유일화 포함 — 데몬 `AgentManager` 가 짓는다 · ADR-0101 · ADR-0120). 셸은 그 이름을 다시 짓지 않는다(유일화 · canonicalize 가 데몬 안에 있다 — 셸이 흉내 내면 갈라진다).
- **데몬 개명 뒤 알림** — U3b 전에는 `RenameProfile` 성공이 `ProfileListUpdated` 만 냈다 → 지금은 `Renamed` 면 `AgentListUpdated` 도 낸다(`agent_list_event` 재사용 · 산 에이전트면 `AgentInfo.name` 이 바뀜을 U3b 시험이 확인). 거부한 갈래 = 셸이 `ProfileListUpdated` 에서 이름을 다시 셈(유일화 접미사를 놓친다). **버스 · CLI `agent.rename`** 은 이 WS 팔을 지나지 않고 명부 통지 포트(`RosterFanout::roster_changed`)로 알리므로, 그 포트도 `ProfileListUpdated` 뒤 `AgentListUpdated` 를 **늘** 낸다(메인 결정 · U3b 리뷰 2026-10-25). 포트는 어느 동사가 불렀는지 모른다 — 띄우기 · 만들기 · 옮기기 · 무변경 개명이 덧붙여 내는 목록은 이름이 그대로면 셸의 이름 장부가 판도 알림도 없이 넘긴다(웹의 에이전트 목록 교체 · 명부 관측은 그대로 한 번 더 돈다).
- **장부 = `ViewManager.agent_names`**(메인 결정 M5) — 이름 셈이 레이아웃 락 하나 안에서 끝난다(새 락 · 새 차례 없음). 바꾸는 길 = `apply::agent_names_updated(state, events, &[(id, name)])`: 락 → 바뀌기 전 자동 이름 탭들의 유효 이름을 뜬다 → 장부 덮어쓰기(지우지 않는다 — 죽은 에이전트 슬롯도 마지막 이름을 쓴다) → 유효 이름이 실제로 바뀐 탭이 하나라도 있으면 **`version` 을 올리고** 그 탭들의 창 페이로드를 복사 → 락 놓고 `announce`(제목 포함 — 불변식 2). 안 올리면 웹이 같은 판 번호의 탭 알림을 버리고(코드 — `viewStore.ts` 의 판 가드) 제목 입히기도 옛 판으로 본다(리뷰 Claude ③ · codex ⑤). 바뀐 것이 없으면 아무것도 내지 않는다.
- **잇는 길** — 연결 태스크의 `emit_broadcast`(`daemon_client/connection.rs`)가 `AgentListUpdated` 팔에서 기존 `events.agent_list_updated` 뒤에 새 포트 `AgentNamesSink::agents(&[AgentInfo])` 를 부른다. 실물은 셸 조립이 `LayoutState` · `TauriEvents` 로 `apply::agent_names_updated` 를 부른다. emit 포트 `DaemonEvents` 를 넓히지 않는다(그 파일 머리 「넓히기 금지」 · 형제 포트 `refusal` 과 같은 가름). `emit_broadcast` 를 부르는 자리(`main_loop` 의 Text 팔 한 곳)는 다른 락을 쥐지 않는다(U3b 확인).
- **부팅 시드**(메인 결정 M16) — `from_persisted` 가 `named == Some(false)` 이고 포커스 슬롯이 `Agent` 인 탭의 저장 `name` 으로 `agent_names[agent_id]` 를 채운다. 옛 파일(`named` 칸 없음)은 시드하지 않는다 — 그 `name` 은 「View 3」 같은 옛 기본값일 수 있다. 데몬이 늦게 붙어도 OS 제목이 「에이전트 - Engram:1」로 한 번 깜빡이지 않게 한다. 사본 수락은 시드를 살아 있는 장부에 **빈 칸만** 더한다(살아 있는 값이 더 새롭다).
- **데몬이 없을 때** — 장부에 없는 에이전트 = 「에이전트」(T2). 데몬이 붙어 목록이 오면 바뀐다.

### 4-4 창 제목(R3 · AC3 · AC21)

- **꼴** = `window_title(활성 탭 유효 이름, 번호)`: 둘 다 있음 → 「<이름> - Engram:<n>」 · 탭 없음 → 「Engram:<n>」 · 번호 없음(숨은 예열 창 · 모델 밖 창) → 「Engram」. 자르지 않는다(작업 표시줄 자르기는 OS 몫).
- **입히는 자리 · 차례**(메인 결정 M6 · 리뷰 codex ⑤) — `WindowTabsPayload` 의 `title` · `version` 을 `commands::layout::TauriEvents::window_tabs_updated` 가 emit 바로 뒤 `app.run_on_main_thread` 로 넘긴다. 메인 스레드에서 `chrome::ChromeBook::accept_title(label, version)` 이 그 창의 마지막으로 입힌 판보다 클 때만 참을 주고, 참이면 `set_title`(실패 = warn). 알림은 락 밖에서 여러 스레드가 내 순서가 뒤바뀔 수 있는데(`layout::apply` 「락 규율」), 메인 스레드 한 줄 + 판 비교가 옛 판을 버린다. 같은 제목이면 OS 호출을 건너뛴다. 장부 항목은 그 창 `Destroyed` 에서 잊는다(`ChromeBook::forget`). **U4 구현**: 판정은 셋 — `Stale`(같거나 옛 판 · 버림) · `Same`(새 판 · 같은 글자 · 판만 적음) · `Set`(입힘) — 이고, `set_title` 이 실패하면 `title_failed` 가 글자만 지워 다음 새 판이 같은 글자로도 다시 입힌다(`chrome::book::apply_title`). Tauri 2.11.3 사실(U4 리뷰가 소스로 확인): 메인 스레드에서 부른 `run_on_main_thread` 는 그 자리에서 돈다(`tauri-runtime-wry` `send_user_message`) — 제목 일은 장부 잎 락 · Tauri 내부만 쥐므로 부르는 쪽이 어떤 락을 쥐어도 교착 길이 없다 · `Destroyed` 때 Tauri 가 사용자 `on_window_event` 보다 먼저 창을 지운다(`manager.on_window_close`) — 그 뒤 일은 `get_webview_window` 가 None 이라 항목을 만들지 않는다.
- **잊은 뒤 온 제목 일은 항목을 되살리지 않는다**(0.3판 재리뷰 잔손) — 넘긴 일이 메인 스레드에서 돌 때 먼저 `app.get_webview_window(label)` 로 창이 있는지 보고, 없으면 장부를 만지지 않고 끝낸다. `Destroyed` 처리기도 메인 스레드라 한 일 안에서 「창 있음 확인 → 장부 적기」 사이에 `forget` 이 끼어들 수 없다. 그래서 `forget` 뒤에 온 늦은 일이 그 label 의 항목을 다시 만들지 않는다(label 은 실행 안에서 다시 쓰이지 않으니 묘비는 두지 않는다). 시험 = `ChromeBook` 에 「창 없음」을 주는 차례(§10-1).
- **알림을 더할 곳** — ① `apply::create_window`: 넣은 뒤 그 새 창 페이로드를 `announce`(지금은 창이 마운트 때 당겨 알림이 없다 — 코드 머리 주석) ② `apply::agent_names_updated`(§4-3) ③ 부팅: 창을 지은 뒤(setup ⑧ · ⑨) 모든 창 페이로드로 제목을 한 번 입힌다(`apply::sync_titles(state, events)` — 프론트 emit 은 아직 듣는 창이 없어 무해) ④ 사본 수락은 이미 모든 창 탭을 `announce` 한다(코드 — `state::restore` `commit` · `after_commit`).
- **지을 때 제목** — 팝아웃 · 예열 창 빌더의 `.title(format!("Engram — {label}"))`(코드 — `commands::popout::build_window`)를 「Engram」으로. main 의 설정 제목 「Engram Dashboard」(`tauri.conf.json`)도 「Engram」으로 — 첫 제목은 부팅 ③ 이 곧 덮는다.

### 4-5 창 인자 · label(R6 · D11 · D12 · Q9 · AC26 · AC28)

- **label 꼴**(메인 결정 M8) = `slot-popup-<run>-<n>` — `run` = 프로세스마다 한 번 뽑는 소문자 hex 8 자(`Uuid::new_v4` 앞 8), `n` = 지금처럼 단조. 글롭 `slot-popup-*`(capability) · `is_popup_label`(접두) 그대로. 고칠 자리 = `PopupCounter::next_label` · `cascade_position`(끝 `-<n>` 을 푼다 — 지금은 접두 뒤 전체를 수로 풀어 실행 표식이 붙으면 모든 창이 같은 계단에 선다) · `state::restore` `prepare` 의 「같은 접두 + 십진 순번이라 (길이, 글자) 순이 발급 순」(같은 실행 안이면 그대로 참) · `tray::core::popout_number`(이미 끝 `-` 조각을 읽어 맞음 · 시험 더함). 충돌 확률 = 앞 실행과 같은 `run` 일 때 2⁻³² · 받아들인다.
- **해석기 하나** — `window_arg::resolve(&ViewManager, arg)`:

| 인자 | 답 |
|---|---|
| `main` | `main` |
| 영속 id(같은 철자 · 또는 UUID 값이 같은 철자) | 그 영속 id 를 가진 **모델 안** 창의 label(같은 철자 먼저 — 저장된 id 가 정규 UUID 철자가 아니어도 앱이 내준 그대로 맞는다) · 없으면 label 갈래로 — 운영엔 UUID 꼴 label 이 없어 결국 `NotFound` · 영속 id 와 label 이 같은 철자면 id 가 이긴다 |
| 모델 안 label | 그대로(같은 실행 안 별칭 — 운영 모델의 label 은 `main` · `slot-popup-*` 뿐) |
| 숫자만(`"2"`) | `NumberGiven` |
| 그 밖(앞 실행 label · 닫힌 label 포함) | `NotFound` |

- **오류 코드** — `NumberGiven` = `INVALID_ARGUMENT`, 문구 = 「창 번호는 창 인자로 받지 않는다 — 번호는 다시 쓰인다. window.list 의 details 에서 그 번호의 window_id 를 찾아 넘기라」 · `NotFound` = `CONFLICT`(지금 레이아웃 실패와 같은 코드 — `commands::not_applied`).
- **쓰는 자리** — 창 인자를 받는 모든 버스 동사(`tab.create` · `tab.switch` · `tab.close` · `tab.list` · `window.close`(단 `if_empty` 는 아래 예외) · `window.getTheme` · `window.setTheme` · `slot.popout` 의 `to_window` · `agent.spawnInto` · `slot.resolveSpatial` · §6 의 새 명령)와, 창 인자를 받는 invoke 껍데기(`commands/layout.rs` 의 창 인자 command · `window_control`). 웹은 label 을 넘기므로 지나가기만 한다. 웹 레지스트리 중 창 인자로 웹 스토어를 고르던 것(`tab.next` · `tab.close` 의 뷰 생략)은 invoke `list_tabs` 로 셸이 푼 답(`label` · 활성 · 탭)을 쓴다 — 스토어 키는 label 뿐이라 window_id 를 못 찾는다. 셸 거절은 그 명령의 오류로 던진다 — 웹 경로의 거절은 브리지 한계로 코드가 INTERNAL 이고 이유는 문구에만 실린다(`commands/view_bus.rs` — AC26 의 웹 명령 몫은 기록한 어긋남).
- **예외 — `window.close { if_empty: true }` 는 해석기를 거치지 않는다**(리뷰 codex ②) — 그 길의 창은 대개 이미 모델에 없다(팝아웃은 탭이 0 이 되는 연산에서 빠진다 — 코드 `pull_tab`). 해석기는 그 창을 `NotFound`(= `CONFLICT`)로 답해 버린다. 그래서 동사가 `if_empty` 를 먼저 보고 `window_ctl::close_if_empty` 로 보낸다 — 판정은 그 함수가 한 락 안에서 한다(§5-5). **단 번호 거부는 그보다 먼저다**(0.3판 재리뷰 잔손 · Q9 · AC26) — 동사의 차례 = ① 인자가 숫자만이면 `INVALID_ARGUMENT`(`if_empty` 여도) ② `if_empty` 면 `close_if_empty`(인자 = 영속 id) ③ 아니면 해석기.
- **도움말**(버스 summary) — 「window = window.list 의 window_id(재시작을 넘어 같은 창) 또는 같은 실행 안 label. 번호는 받지 않는다(다시 쓰인다). label 은 그 실행 안에서만 맞는다.」 · `window.list` summary 에 「번호는 비어 있는 가장 작은 수를 다시 쓴다 · 닫히는 창이 아직 없어지지 않았으면 그 번호는 비지 않는다(D13)」.

---

## 5. 동시성 · 차례

### 5-1 락 · 포트

| 락 | 종류 | 쥔 채 하는 일 | 차례 |
|---|---|---|---|
| 레이아웃 `LayoutState(Mutex<ViewManager>)` — 번호 · `closing` · 이름 장부 포함 | 기존 | 모델 변형 · 페이로드 복사 · `SubscriptionSync::resync` 뿐 | 바깥 |
| 예열 창 풀 `SpareState` | 기존 | — | 레이아웃 → 풀 |
| 첫 화면 문 `FirstPaintGate.held` | 기존 | — (`is_held` 읽기 하나 더함) | 레이아웃 → 풀 → 첫 화면 문(잎) |
| 창 사실 `tab_drop::FrameBook` | 기존 | 칸 비교 · 적기 | 잎 |
| 크롬 `chrome::ChromeBook`(어댑터의 `Mutex`) | 새 | 판정만 — 쥔 채 OS 호출 · emit · 다른 락 없음 | 잎 |
| 미룬 최대화 `state::placement::DeferredMaximize` | 기존 | — | 잎(코드 — 그 타입 문서 「락은 잎이다」) |

- **새 OS 호출(제목 · 최소화 · 최대화 · 최소 크기 · 상태 읽기 · 앞 창 읽기)은 전부 레이아웃 락 밖**이다(「락 규율」). 상태 읽기는 `WindowPlacement` 와 같은 차례 — 락 밖에서 읽고 모델 행과 짝짓는다(조회 답은 순간값이다).
- **제목 입히기는 메인 스레드 한 줄**(§4-4) — `ChromeBook` 의 판 비교와 `set_title` 이 같은 메인 스레드 일 안에서 차례로 돈다(락을 쥔 채 OS 를 부르지 않는다 — 비교 뒤 락을 놓고 부른다. 같은 창의 두 일이 끼어들 수 없는 것은 메인 스레드가 한 줄이기 때문이다).
- **이름 장부는 레이아웃 락 안 칸**이라 새 차례가 없다.

### 5-2 아직 보이지 않은 창 · 창 상태 명령 = 메인 스레드 일 하나(리뷰 codex ⑥ · 0.3판 재리뷰 ② ③ — 메인 결정 M23 · M29 · M30)

새 창 · 복원 창은 첫 화면 알림이나 시한까지 숨은 채 첫 화면 문에 「올려져」 있고(`first_paint` — 그 문이 보이기를 소유한다), 모두 숨기기 뒤의 창은 트레이 보이기 · `window.showAll` · `window.raise` 가 드러낸다. tao 의 최대화 · 최소화 · 펴기는 숨은 창을 드러낸다(코드 주석 — `state::placement::DeferredMaximize` 「숨은 창을 최대화하면 tao 가 `SW_MAXIMIZE`(창을 보이고 활성화한다)를 낸 뒤에야 `SW_HIDE` 로 되숨긴다」). 그래서 보이지 않는 창에는 이렇게 한다:

| 창 | `minimize` | `maximize` | `unmaximize` | `raise` | `close` |
|---|---|---|---|---|---|
| 보임 | 한다 | 한다 | 한다 | §6-2 | §5-4 |
| 첫 화면 대기(첫 화면 문에 항목 — `first_paint::is_held`) | `CONFLICT`「아직 첫 화면 전인 창」 | 바랄 최대화 상태 `true` 를 적는다(`DeferredMaximize` — label 마다 bool · 먼저 적은 것을 덮는다 · 모델 `attrs.maximized = true`) — 보일 때 입힌다 · 답 `{ deferred: true }`. 이미 최대화(최소화 아님)면 적어 둔 바람만 거둔다 · `{ deferred: false }` | 바랄 상태 `false` 를 적는다(모델 `attrs.maximized = false`) — 보일 때 원래 크기로 입힌다 · 답 `{ deferred: true }`. 이미 보통 크기(최대화 · 최소화 아님)면 바람만 거둔다 · `{ deferred: false }`. 거절하지 않는다 | 아무것도 하지 않는다 — 첫 화면 문이 곧 보인다 · 답 `{ shown_all: false, focused: false, pending: true }` | 한다(닫기는 보이기와 무관) |
| 숨김(모두 숨기기 · `--hidden`) | `CONFLICT`「숨은 창 — 먼저 window.raise 또는 window.showAll」 | 위와 같다(다음 보이기가 입힌다 — 코드 `tray::actions::show_main_ui` · `window_ctl::show_one_no_steal`) | 위와 같다 | LLM 보이기(`NoSteal` — §6-2) → 그 창 앞으로 | 한다 |

- **최소화된 창의 `is_maximized` 는 거짓이다**(tao 0.35.3 `platform_impl/windows/event_loop.rs` — 최소화의 `WM_SIZE` 가 최대화 깃발을 내린다 · 소스 독해) — 최대화한 뒤 최소화한 창은 `min = true, max = false` 로 읽혀 펴면 어느 크기로 돌아올지 모른다. 그래서 보이지 않는 최소화 창의 `maximize` · `unmaximize` 는 늘 바람을 적고(보이기 → 펴기 → 바람 입히기 차례 — §6-2 ⓑ), 보이는 최소화 창의 `unmaximize` 는 무동작이다(먼저 `window.raise`).
- **이미 그 크기면 바람을 거둔다** — 거두지 않으면 먼저 적은 반대 바람(숨은 최대화 창의 `unmaximize` 뒤 `maximize` 등)이 보일 때 입혀져 뒤 동작이 뒤집힌다. 보이는 창도 같다 — `maximize` · `unmaximize` 는 OS 호출 뒤에, 이미 그 크기(최소화 아님)인 무동작 갈래도 바람을 거두고 모델 `attrs.maximized` 를 그 값으로 둔다(`window_ctl::plan` 의 `Settle`). 보이기의 바람 입히기가 실패하면 바람이 보이는 창에 남고(`apply_deferred` 는 못 입히면 남긴다), 남은 바람은 다음 숨기기 · 보이기에서 입혀져 그 사이 동작을 뒤집는다. 보이는 최소화 창의 `unmaximize`(무동작)는 바람을 건드리지 않는다.
- **숨은 창이 OS 로 최대화돼 있는 것은 최대화된 채 모두 숨기기를 했을 때뿐이다** — 부팅 · 런타임 복원은 main 의 최대화도 미룬다(바람으로 적고 보일 때 입힌다). 그래서 `--hidden` 으로 뜬 main 은 저장 상태가 최대화여도 숨은 동안 OS 최대화가 아니다(`is_maximized` 거짓 · 바람 `true`) — 그 창의 `maximize` 는 `Settle` 이 아니라 바람 덮어쓰기이고, `unmaximize` 는 바람만 거둔다.

- **읽기 · 판정 · 입히기를 한 메인 스레드 일로 돈다**(0.3판 재리뷰 ② — M29) — 첫 화면 보이기는 메인 스레드에서 돈다(코드 — `first_paint` 의 `release_on_main_thread` → `show_as` → `show_placed` 가 그때 미룬 최대화 표를 읽는다). 0.2판처럼 버스 스레드에서 `is_held` 를 읽고 메인 스레드에서 미룸을 적으면, 그 틈에 보이기가 지나가 미룸이 홀로 남는다 — 그러면 `state::placement::record` 가 미룸이 서 있는 동안 자리 적기와 `window:frame` 밀기를 조기 반환으로 멈춘다(코드 — `record` 의 `is_pending`). 그래서 실물 포트(`WindowControl::apply`)는 상태 읽기(`is_held` · 보임 · 최대화 · 최소화) → 표 판정(`window_ctl::plan` — 순수) → 미룸 적기/거두기 · OS 호출을 **`app.run_on_main_thread` 일 하나** 안에서 하고 그 답을 기다린다. 메인 스레드는 한 줄이라 그 일과 첫 화면 보이기가 끼어들 수 없다. 부르는 쪽은 메인 스레드가 아니고, 기다리는 동안 async 워커도 붙들지 않는다. 셸 표의 `blocking_handler` 본문은 블로킹 풀이 아니라 데몬 클라이언트 런타임의 async 워커에서 돈다(코드 — `layout/commands.rs` `offloaded_handler` 문서). 그래서 새 동사 `window.minimize` · `window.maximize` · `window.unmaximize` · `window.raise` · `window.showAll` · `window.hideAll` 과 OS 상태를 읽는 `window.list` 는 `offloaded_handler`(`spawn_blocking` 뒤에서 돈다)로 꽂는다. invoke `window_control` 은 async 명령으로 둔다. ★메인 스레드에서 불러도 교착은 아니다★ — tauri-runtime-wry 2.11.3 `send_user_message` 가 메인 스레드에서 넘긴 일을 그 자리에서 돈다(소스 독해). 위험은 둘이다: ① 레이아웃 락을 쥔 채 부르면 그 일이 같은 락을 다시 잡는다(재진입 안 되는 뮤텍스) ② 답 채널이 용량 0 이면 그 자리에서 도는 일의 보내기가 아직 오지 않은 받기 앞에서 영영 선다 — 실물은 `mpsc::channel()`(또는 `sync_channel(1)`)로 받고 `recv_timeout` 으로 기다린다(메인 스레드가 멈추면 풀 스레드를 붙들지 않고 오류 값이 된다).
- **LLM 보이기는 첫 화면 대기 창을 건너뛴다 · 트레이 누름은 그대로**(0.3판 재리뷰 ③ — M30) — 트레이 보이기는 지금 예열 창이 아닌 모든 창에 `cancel_hold` → `show()` 를 한다(코드 — `tray::actions::show_main_ui`) — 첫 화면 대기 창도 그리기 전에 드러낸다. 사람이 누른 보이기라 그 동작을 둔다. LLM 길(`NoSteal` — `window.showAll` · 숨은 창의 `window.raise` · invoke `show_main_ui`)은 같은 일 안에서 `is_held` 인 창을 건너뛴다 — 그 창은 곧 첫 화면 문이 보인다(불변식 5).
- **모든 보이기 갈래가 미룬 최대화를 입힌다** — 지금은 첫 화면 보이기 중 「지어서 올린 창 · 복원 창」 갈래(`state::placement::show_placed`)와 트레이 보이기만 입히고, 창 포트가 포커스 없이 지은 창 · 맡긴 예열 창 갈래(`first_paint` 의 `raise_and_show` · `show_adopted`)는 입히지 않는다(코드 — `first_paint` 머리 「보이는 법은 셋이다」). U7a 가 그 둘에도 바람 입히기(`apply_deferred_maximize` — 바람이 bool 이 되어 `false` 면 원래 크기로 입힌다)를 붙인다(적은 것이 없으면 무동작).
- **시험**(0.3판 재리뷰 ②) — `lib_unit` 에 「메인 스레드 줄」 흉내(일을 넣은 차례대로 하나씩 도는 큐)를 두고, 같은 창에 대해 「첫 화면 보이기 일」과 「`maximize` 일」을 두 차례로 넣는다: (a) 보이기 먼저 → `maximize` 는 보인 창으로 판정돼 그 자리에서 입혀지고 미룸 표가 비어 있다 (b) `maximize` 먼저 → 미룸에 적히고 뒤의 보이기가 입히고 거둔다. 두 차례 모두 끝에 미룸 표가 비어 있다(= `record` 가 멈추지 않는다). `unmaximize` · 숨김 창 · LLM `showAll` 의 대기 창 건너뛰기도 같은 흉내로 잰다. 판정 순수 함수 `plan` 은 표 시험으로 따로 잰다.

### 5-3 새 창 세 자리 — 답(R6 · AC2)

열기(락 밖 — 그대로) → 넣기(락 — 번호 받음 · 페이로드 복사) → 알리기(락 밖 — 제목 입힘) → 확정(락 밖 — `settle_new_window` 그대로) → 답 `{ window, window_id, number }`. 첫 화면은 기다리지 않는다(결정 · 2026-10-24 — T1). 새 창은 지금처럼 첫 화면 알림이나 1.5 s 시한에 보인다(`first_paint::FALLBACK_REVEAL` 그대로) — 시한에 먼저 보인 창은 첫 그림 전이라 번호 표식이 아직 없을 수 있다(받아들임 — PRD 0.5판 AC2). 첫 화면 뒤에 웹뷰가 죽는 것 · 그리지 못한 채 시한에 보인 창은 따로 처리하지 않는다(T1).

### 5-4 닫는 길 합류(R10 · R23 · AC8 · AC24)

| 길 | main | 팝아웃 |
|---|---|---|
| OS 닫기(Alt+F4 · 작업 표시줄 닫기) | `CloseRequested` → `prevent_close` + `hide_main_ui`(코드 — `lib.rs`) | `CloseRequested` → `close_requested_popup_window`(모델 먼저 지움 · 묘비 — 코드) → `Destroyed` → 번호 풂 |
| 제목 줄 셋째 단추 | 레지스트리 `window.close` → invoke `window_control{Close}` → `window_ctl::close` → `hide_all` | 같은 길 → `apply::close_window` → `host.close` → `Destroyed` |
| 버스 `window.close` | `window_ctl::close` → `hide_all`(답 `HiddenAll` — 지금의 `CONFLICT` 를 바꾼다) | `apply::close_window`(답 `Closed`) |
| 마지막 탭 닫기 | (메인은 빈 탭을 다시 세운다 — 그대로) | `apply::close_tab` → `WindowClosed` → `host.close`(그대로) |
| 0탭 자가닫힘 | — | `window.close { window: <영속 id>, if_empty: true }` — §5-5 |

- **같은 결과의 근거** — main 의 모든 길이 `tray::actions::hide_main_ui` 하나로 모인다(ADR-0229). 팝아웃의 모든 길이 「모델에서 지움 → OS 창 파괴 → `Destroyed` 에서 번호 풂」 한 차례다.
- **팝아웃 닫기 요청을 되돌리지 않는다**는 전제(코드 — `lib.rs` `CloseRequested` 주석 · W7)를 지킨다 — 제목 줄 단추는 OS 닫기 요청을 내지 않고 버스로 간다. 웹에 닫기 요청 리스너를 두지 않는다(그 주석의 점검 명령 그대로).

### 5-5 0탭 자가닫힘 재확인(R23 · W9 · AC24 · AC32)

- **모델 말로 옮기면** — 팝아웃 항목은 탭이 0 이 되는 그 연산에서 모델에서 빠진다(코드 — `ViewManager::pull_tab` 의 `PopoutRemoved`). 그래서 「아직 탭 0 개」 = **그 label 의 항목이 모델에 없다**, 「그사이 탭이 들어왔다」 = **항목이 모델에 있다**(항목이 있으면 탭이 1 개 이상이다 — 불변식). 탭 0 개인 항목은 없다.
- **명령** — 웹(`WindowLayout.tsx` 의 0탭 효과)이 `getCurrentWindow().close()` 대신 레지스트리 경유 invoke `window_control{ Close, if_empty: true }` 를 자기 **영속 id**(페이로드 `window_id` — 탭이 있던 때 받아 둔 값)로 부른다.
- **판정(`window_ctl::close_if_empty(state, host, window_id)` — 해석기보다 먼저 · 한 락)** — 락 안에서 차례로: ① 그 id 의 항목이 모델에 있으면 `Kept`(무동작 · 오류 아님) ② 없고 `closing` 에 그 id 가 있으면 그 label 을 고른다 ③ 둘 다 없으면 `Kept`. 락 밖에서: ② 의 label 의 OS 창이 열려 있으면 `host.close` → `Closed`. ①–③ 이 한 락 안이라 그 사이 탭이 들어오는 틈이 없다(들어오면 ① 이다).
- **AC32 「새 창(다른 영속 id)은 닫히지 않는다」** — 영속 id 로 찾으므로 다른 창에 닿지 않는다(label 도 실행 안에서 다시 쓰이지 않는다).
- **재는 법** — 그 상태(모델에 없고 OS 창은 있다)는 운영에서 정상 흐름으로 생기지 않는다(백엔드가 닫기를 직접 한다 — 코드 `WindowLayout.tsx` 0탭 효과 주석). 단위 하네스(가짜 `WindowHost::close` 가 아무것도 하지 않는 평범한 닫기)로 만들어 잰다(§2-3 「시험 몫 도우미」 · PRD 0.5판 AC24 · AC32).

### 5-6 복원 · 사본 수락(D3 · W12 · AC25 · AC31)

- 부팅 = §4-1. 사본 수락 = 지금 차례 그대로(② 숨긴 새 팝아웃 짓기 → ③ 커밋(`adopt_restored`) → ④ 보이기 · 옛 창 거두기 — 코드 `state::restore`). 번호는 ③ 에서 세운다(§4-1 표). ④ 의 옛 창 거두기가 사본 창이 보인 뒤라 같은 번호가 잠깐 두 창에 설 수 있다 — **받아들인 위험**(W12). 차례를 바꾸지 않는다(PRD D3 「지금 코드 차례여도 된다」).
- **남는 겹침 없음**(AC31) — 옛 팝아웃 번호는 ③ 에서 다 풀리고(`closing` 포함), 옛 창의 `Destroyed` 는 `release_closing(label)` 이 label 로만 찾아 사본 창 번호를 건드리지 않는다. ④ 의 `forget_vanished`(바꾸기 전에 사라진 사본 창)는 `drop_window_gone` 으로 곧바로 푼다(§4-1). ③ 뒤에 넣는 새 창은 비어 있는 가장 작은 번호를 받는다.

### 5-7 받아들인 잠깐 겹침(다시 막지 않는 것)

복원 · 사본 수락 도중(W12) · `Destroyed` 를 기다리는 동안의 `window.list` 번호 빈칸(D13 의 대가 — 도움말에 적는다). 명령은 영속 id 로 가리켜 엇나가지 않는다(Q9).

이름 장부(§4-3)에서 받아들인 것 둘(메인 결정 · U3b 리뷰 2026-10-25):

- **잠든 · 죽은 에이전트의 개명은 그 슬롯 탭에 그 에이전트가 다시 살아날 때까지 안 비친다** — `list_agents` 는 산 세션만 싣고 셸은 이름을 다시 짓지 않는다.
- **`AgentListUpdated` 를 내는 쪽끼리 차례가 없다** — 동시 띄우기 · 끄기가 낸 옛 목록이 다음 목록까지 옛 이름을 되살리고, 끝난 에이전트를 웹 명부에 되살려 트리가 살아 있다고 보이며 그 뷰가 구독 거절을 거쳐 `error` 로 앉을 수 있다(목록은 띄우기 · 끄기 · 명부 동사 · 재연결 때만 오므로 조용하면 오래 갈 수 있다 · U3b 가 내는 쪽을 늘렸다). 원래 있던 경쟁 부류이고, 진짜 해법은 protocol 의 목록 판 번호다(범위 밖).

---

## 6. 창 상태 명령(LLM 길 — R17 · R18 · AC9)

### 6-1 명령 표(버스 `declare_commands!` — `layout/commands.rs`)

| 이름 | 인자 | 답 | 하는 일 | 오류 |
|---|---|---|---|---|
| `window.minimize` | `window` | `{}` | 그 창 최소화(§5-2 표) | 없는 창 `CONFLICT` · 번호 `INVALID_ARGUMENT` · 보이지 않는 창 `CONFLICT` |
| `window.maximize` | `window` | `{ deferred: bool }` | 최대화(이미면 무동작 · 보이지 않으면 바랄 상태를 적는다 — §5-2) | 없는 창 `CONFLICT` · 번호 `INVALID_ARGUMENT` |
| `window.unmaximize` | `window` | `{ deferred: bool }` | 최대화 → 원래 크기(최대화가 아니거나 최소화면 무동작 · 보이지 않으면 바랄 상태를 적는다 — §5-2) | 같음 |
| `window.raise` | `window` | `{ shown_all: bool, focused: bool, pending: bool }` | 숨어 있으면 모든 창 보이기 → 최소화면 펴기 → 맨 위 · 전경을 **청한다**(빼앗지 않는다) | 같음 |
| `window.hideAll` | — | `{}` | 모든 창 숨기기(`hide_main_ui` — main 셋째 단추와 같다) | — |
| `window.showAll` | — | `{ focused: bool }` | 모든 창 보이기(트레이 보이기와 같은 차례 · 포커스를 빼앗지 않는 갈래 — §6-2) | — |
| `window.close`(바뀜) | `window` · `if_empty: Option<bool>` | `{ outcome: Closed \| HiddenAll \| Kept }` | §5-4 · §5-5 | 같음(`if_empty` 는 `CONFLICT` 를 내지 않는다) |
| `window.list`(바뀜) | — | `{ windows: Vec<String>, details: Vec<WindowRow> }` | §6-3 | — |
| `window.create`(바뀜) | — | `{ window, window_id, number }` | §5-3 | `CONFLICT` |
| `slot.popout`(바뀜) | 그대로 | `{ window, new_view_id, window_id, number }` | §5-3 | 같음 |

- **이름** — 「원래 크기」를 `window.restore` 로 짓지 않는다 — 버스의 `restore.*`(크래시 사본)와 헷갈린다(메인 결정). 「되살리기」 셋(PRD R17)은 `unmaximize` · `raise` · `showAll` 이 맡는다.
- **`focused` 는 사실 그대로**(W10 · AC9) — 다른 앱이 앞일 때 OS 가 전경 바꾸기를 막으면 맨 위로만 올리고 `focused: false` 를 답한다. 우회(Alt 합성 등)를 하지 않는다(ADR-0299 결정 5 — §6-2).
- **옮기기 · 크기 · 스냅 명령은 없다** — 알려진 갭(PRD §2 · CLAUDE.md 「LLM-우선 제어」 남은 갭에 U10 이 더한다).

### 6-2 서비스 `layout/window_ctl.rs` · 포트 `WindowControl`

```rust
pub trait WindowControl: Send + Sync {          // ★레이아웃 락 밖에서 부른다 · 실물은 한 동작을 메인 스레드 일 하나로 돈다(§5-2)★
    fn apply(&self, label: &str, action: WindowAction) -> Result<Applied, String>;  // 일 몸통 = run_action
    fn show_all(&self, how: Activation) -> bool; // 실물 = tray::actions::show_main_ui(app, how) — 메인 스레드 일 하나 · 답 = 그 뒤 main 이 OS 전경 창
    fn hide_all(&self);                          // 실물 = tray::actions::hide_main_ui — 메인 스레드 일 하나
    fn state(&self, label: &str) -> Option<OsWindowState>;   // window.list 몫 순간값 · None = OS 창 없음
}
pub trait WindowOps { /* read · minimize · maximize · unmaximize · defer_maximized(label, bool) · drop_deferred · raise_without_activation · show · unminimize · apply_deferred · foreground_is_ours · set_focus · show_all_no_steal */ }  // 일 몸통이 쓰는 창 하나의 OS 조작 seam — Tauri 어댑터가 구현
pub fn run_action(ops: &impl WindowOps, layout: &LayoutState, label: &str, action: WindowAction) -> Result<Applied, String>;  // 읽기 → plan → 바람 · OS 호출
pub fn show_one_no_steal(ops: &impl WindowOps, label: &str, focus: bool) -> bool;  // NoSteal 보이기의 창 하나 몫(ⓐ–ⓒ · 대기 창은 건너뛴다)
pub enum WindowAction { Minimize, Maximize, Unmaximize, Raise }
pub enum Activation { TrayClick, NoSteal }
pub struct Applied { pub deferred: bool, pub pending: bool, pub shown_all: bool, pub focused: bool }
pub struct OsWindowState { pub visible: bool, pub minimized: bool, pub maximized: bool, pub focused: bool, pub awaiting_first_paint: bool }
pub fn plan(action: WindowAction, s: OsWindowState) -> Plan;   // 순수 — §5-2 표 · Tauri 0 · 표 시험
```

- **`show_all` · `hide_all` 도 메인 스레드 일 하나로 돈다**(리뷰 4) — `NoSteal` 의 ⓐ `place_window` 는 날 Win32 호출이라 메인 스레드에서 불러야 하고, 창마다의 대기 창 판정(`is_held`) · 보이기 · 미룬 최대화도 첫 화면 보이기와 끼어들지 않아야 한다(§5-2). 실물은 `apply` 와 같이 `run_on_main_thread` 로 넘기고 답을 기다린다(트레이 누름 처리기 · 단일 인스턴스 콜백은 이미 메인 스레드라 그 자리에서 돈다).

- **포커스를 빼앗지 않는 보이기 · 앞으로 내기**(리뷰 Claude ⑤ · 0.3판 재리뷰 ① — 메인 결정 M22) — 0.2판의 「날 Win32 `place_window(show: true, activate: true)` 로 보인다」는 걷는다: 날 Win32 로 보이면 tao 의 보임 깃발이 거짓으로 남아 다음 트레이 숨기기가 아무것도 하지 않는다(코드 주석 — `first_paint::raise_and_show` 의 ⓑ · `commands::spare` 예열 창 짓기 머리). 대신 이미 있는 「포커스 없이 보이기」 차례(`first_paint::raise_and_show` · ADR-0299)를 그대로 쓴다 — 모두 메인 스레드에서:
  - ⓐ platform `window::place_window(&os, None, ZOrder::Top, show: false, activate: false)` — 숨긴 채(보인 창이면 보인 채) z 만 맨 위.
  - ⓑ 숨었으면 Tauri `show()` · 최소화면 Tauri `unminimize()`(보임 · 최소화 깃발을 tao 가 맞게 쥔다). 그 뒤 적어 둔 바람이 있으면 입힌다(`WindowOps::apply_deferred` — 최대화 · 원래 크기 · 펴기 전에 입히면 `SW_RESTORE` 가 덮는다).
  - ⓒ Tauri `set_focus()` 는 **지금 전경이 우리 프로세스일 때만**(`first_paint::new_window_show(true, foreground_is_ours)` 의 규칙 그대로 — 남의 앱이 전경이면 tao 가 가짜 Alt 로 빼앗으므로 부르지 않는다). 답의 `focused` = 그 뒤 `is_foreground`.
- **`restore_no_activate` 는 필요 없다**(0.3판 재리뷰 ① 의 「다시 끌어내기」) — 펴기를 날 Win32(`SW_SHOWNOACTIVATE`)로 하면 위 깃발 문제가 그대로 생긴다. Tauri `unminimize()`(tao = `SW_RESTORE`)를 쓴다. `SW_RESTORE` 의 활성화는 OS 의 전경 잠금을 따르고 Alt 를 합성하지 않는다(가능성 높음 — tao 의 `unminimize` 경로는 읽지 않았다 · QA 가 다른 앱이 앞일 때 `window.raise` 로 잰다 · AC9). 다른 앱 위로 올라서는지(ⓐ 만으로)는 기존 미측정 그대로다(코드 주석 — `raise_and_show` 「[미측 — M22]」).
- **트레이 보이기와 한 경로**(ADR-0229 「경로는 하나」) — `tray::actions::show_main_ui(app, Activation)`: `TrayClick` = 예열 창 아닌 모든 창에 `cancel_hold` → `show()` → `unminimize()` → 미룬 상태 입히기 → `set_focus()`(main 이 마지막 — 누름이 전경 권한을 준다). 입히기를 펴기 뒤에 두는 것은 최소화된 창의 `false` 바람을 잃지 않으려는 것이다 — tao 가 최소화된 동안 최대화 깃발을 내려 두어 원래 크기 입히기의 차이가 비고(아무것도 안 한다), 그 뒤 `SW_RESTORE` 가 최대화로 되돌린다(§5-2 「최소화된 창의 `is_maximized`」 · ⓑ). 지금 코드는 입히기가 펴기 앞이다 — U7a 2부가 `tray::actions::show_main_ui` 의 차례를 이렇게 바꾼다 · `NoSteal` = 첫 화면 대기 창을 건너뛰고(§5-2) 각 창에 ⓐ–ⓑ, ⓒ 는 전경이 우리일 때 main 에만(창 사이 차례 그대로). 예열 창 건너뛰기 · 사용량 관심은 두 갈래가 같이 탄다.
- **누가 어느 갈래인가**(0.3판 재리뷰 잔손 — M31) — `TrayClick` = 트레이 아이콘 누름 처리기(`tray::mod`)와 단일 인스턴스 플러그인의 두 번째 실행 콜백(`lib.rs` 의 `tauri_plugin_single_instance::init` — 사용자가 앱을 다시 실행했다)뿐 — 둘 다 사람의 손이다. **invoke `show_main_ui`(`commands::tray` — LLM · cdp 가 부르는 §5 표면 · 코드 주석)는 `NoSteal`** 로 바꾼다 — 버스 `window.showAll` 과 같은 결과가 되고, LLM 에게 두 번째 「빼앗는 길」이 남지 않는다.
- **앞 창 판정**(리뷰 Claude ④ — 메인 결정 M12) — Windows 에서는 Tauri `is_focused` 를 쓰지 않는다: wry 가 창의 `WM_SETFOCUS` 마다 포커스를 웹뷰로 넘겨 그 값이 웹뷰가 키보드 포커스를 쥔 동안 거짓이다(코드 주석 — `state::placement::foreground_is_ours` · 소스 독해 · GUI 미검). 대신 platform 에 `window::is_foreground(w: &impl HasWindowHandle) -> Option<bool>`(Windows = `GetForegroundWindow` 가 그 최상위 창인가 · 그 밖 = `None` → 셸이 Tauri `is_focused` 로 물러난다)을 더한다 — `foreground_is_current_process` 와 같은 꼴 · `cfg` 는 그 함수 안에만. 「많아야 한 창만 참」은 전경 창이 하나라 선다.
- 서비스 함수(`window_ctl`)는 창 인자를 해석하고(§4-5) · 레이아웃 락 안에서 그 창이 모델에 있는지 확인하고 · 락 밖에서 `WindowControl::apply` 를 부른다(`raise` 의 숨은 창 갈래는 그 일 안에서 `NoSteal` 보이기를 먼저 한다). 판정 표는 순수 `plan` 이고 실물이 메인 스레드 일 안에서 그것을 부른다 — 그래서 판정은 Tauri 없이 표 시험으로, 순서(보이기 일과의 끼어듦)는 §5-2 의 메인 스레드 줄 흉내로 잰다.
- 실물(`commands::window_ctl::TauriWindowControl`)은 메인 스레드 일 안에서 Tauri `WebviewWindow` 의 `minimize` · `maximize` · `unmaximize` · `show` · `unminimize` · `set_focus` · `is_visible` · `is_minimized` · `is_maximized` 와 platform `place_window` · `is_foreground` · 셸 `DeferredMaximize` · `first_paint::is_held` · `foreground_is_ours` 를 부른다.

### 6-3 `window.list`

- `windows` = 지금처럼 label 목록(R18 「깨지 않고 더한다」).
- `details[]` = `{ window_id, window /* label */, number, main: bool, visible, minimized, maximized, focused }` — 모델 행은 락 안에서 복사, OS 상태는 락 밖에서 `WindowControl::state` 로 읽어 짝짓는다. OS 창이 없는 행(모델 · OS 가 막 갈린 순간)은 상태 칸을 모두 거짓으로 둔다. 순서 = 번호 순.
- **닫히는 창(`closing`)은 싣지 않는다**(모델에 없다 — PRD 「`window.list` 에 보이는 창」). 그 번호가 아직 비지 않았다는 것은 도움말이 말한다(D13).

### 6-4 사람 길 — 웹 레지스트리 · invoke

- **invoke 하나** `window_control { window: String, action: WindowControlAction, ifEmpty?: bool } -> WindowControlOutcome`(ts-rs export) — 동사마다 invoke 를 늘리지 않는다(메인 결정 M7). 같은 서비스(`window_ctl`)에 떨어진다(ADR-0081 두 껍데기). `ifEmpty` 이면 해석기를 거치지 않는다(§4-5 예외).
  - **U7b 가 붙는 면**(코드 — `commands/window_ctl.rs`) — `WindowControlAction` 은 서비스의 `WindowAction` 과 따로 선 wire enum 이다: `minimize` · `maximize` · `unmaximize` · `raise` · `hide_all` · `show_all` · `close`(snake_case). 인자 이름은 `ifEmpty`(Tauri 가 JS 쪽을 camelCase 로 받는다). 답 `WindowControlOutcome { deferred, pending, shown_all, focused, close }` — `close` 는 `close` 동작에만 `WindowCloseResult`(`closed` · `hidden_all` · `kept`) · 그 밖은 `null`.
  - main 의 `close` 는 출력 라우터 · 데몬 클라이언트 없이 모든 창 숨기기로 간다(팝아웃 닫기만 그 둘을 요구한다).
- **레지스트리**(`src/commands/tabCommands.ts` 또는 새 `windowCommands.ts`) — `window.minimize` · `window.maximize` · `window.unmaximize` · `window.raise` · `window.hideAll` · `window.showAll` 을 등록하고 `viewStore` 액션 → invoke 로 보낸다. `window.close` 는 지금 등록을 넓힌다(main 도 받는다 · `if_empty`). 셸 표와 같은 id 다(지금 `window.create` · `window.close` 가 두 표에 함께 있는 것과 같은 꼴).
- **단추 → 레지스트리 명령**(R17 · D6) — 최대화 단추는 창 사실(`window:frame`)의 `maximized` 를 보고 `window.maximize` / `window.unmaximize` 를 고른다.

### 6-5 창 사실 — 기존 `window:frame` 에 칸 더하기(리뷰 codex ⑧ — 메인 결정 M21)

- **새 사건 · 새 당기기 invoke 를 만들지 않는다**(0.1판의 `window:chrome` · `window_chrome_state` · `window_chrome` 을 걷었다). 슬롯 도킹의 창 사실 길(셸 `tab_drop::FrameBook` · `push_window_frame` · invoke `window_frame` · 웹 `dock/windowFrame.ts` 의 `createWindowFrame` — 사건 듣기를 먼저 걸고 한 번 당기며, 당기기 뒤 사건이 하나라도 오면 당기기 답을 버린다 · 코드)에 칸을 더한다: `focused` · `caption`(§3-2).
- **미는 때** — 지금 자리 기록(`state::placement::record` — `Moved` · `Resized`)이 부르는 `push_window_frame` 에 더해, `lib.rs` 의 `WindowEvent::Focused(b)` 에서도 그 창의 `push_window_frame` 을 부른다. 바뀌었을 때만 민다(`FrameBook` 의 기존 규칙). 다른 Engram 창이 앞으로 오면 그 창의 `Focused(true)` 와 옛 창의 `Focused(false)` 가 둘 다 와 두 창이 각자 민다.
- **`focused` 값의 출처**(0.3판 재리뷰 잔손) — `Focused(b)` 로 미는 그 창 자신의 칸은 **사건의 `b`** 를 쓴다 — 사건이 날 때 OS 전경 창 값이 아직 바뀌지 않았을 수 있다(차례 미검). 당기기(`window_frame`) · 그 밖의 밀기 · `window.list` 는 platform `is_foreground` 를 쓴다. 두 값이 어긋나는 순간이 있는지는 S12 가 사건 시점의 `is_foreground` 도 함께 찍어 잰다 — 어긋나면 사건 뒤 다음 메인 스레드 차례에서 `is_foreground` 로 한 번 더 민다.
- **`Focused` 사건이 OS 활성 바뀜마다 오는지는 미검**(S12) — tauri-runtime-wry 는 창의 포커스 사건을 웹뷰의 GotFocus · LostFocus 에서 만든다(코드 주석 — `state::placement::foreground_is_ours`). 빠지는 계기가 있으면 흐림이 늦는다(AC15). 그때의 길 = platform 에 `WM_ACTIVATE` 를 듣는 서브클래스(스냅 오버레이와 같은 꼴) — 지금 짓지 않는다(S12 결과로 정한다).
- **예열 창** — `window_frame` 당기기는 탭 모델에 없는 창이면 `Err` 다(코드). 제목 줄은 보통 모드(맡겨진 뒤)에서만 그려지므로 문제가 없다.
- **비활성 흐림(R11)** — 제목 줄이 `frame.focused` 로 `data-window-active` 를 정한다(§7-5). `window.list` 의 `focused` 와는 위 출처 규칙으로 맞춘다(AC15 — S12).

### 6-6 선언 세대

선언이 바뀌는 단위마다 `catalog_version` 을 올린다(지금 17 — `layout/commands.rs` 머리 규칙). 예상: U3a(`TabRow.named` — 18) · U5 + U6(창 인자 도움말 · 번호 거부 · `window.create` · `slot.popout` 답 — 한 번에 19) · U7a(새 명령 · `window.close` · `window.list` — 20). 같은 PR 에 묶이면 한 번 올려도 된다(번호는 세대 표지이지 분기 재료가 아니다 — 그 머리 주석).

---

## 7. 제목 줄(웹 — 렌더만)

### 7-1 배치(R7 · R13 · R24)

```
<TitleRow ref=titleRow data-window-active=… data-maximized=…>   ← 끌 자리(자기 자신) · 보통 상태 위 여백 TOP_GAP
  <TabBar …/>            flex 1 1 0 · min-width 0          ← 탭 · `+` · 탭 줄 빈 곳(끌 자리 — 자기 자신)
  <GrabGap/>             flex 0 0 GRAB_MIN_W                 ← 끌 자리(빈 곳 최소)
  <NumberMarker n=… />   flex 0 0 auto · pointer-events:none ← 끌 자리(부모가 받는다)
  <CaptionButtons main=… maximized=… />  flex 0 0 auto        ← 단추 셋(caption.buttons 일 때만)
</TitleRow>
```

- **탭이 먼저 준다** — 탭바만 `flex-shrink` 가 있고 나머지는 고정이라, 줄이 좁아지면 탭바의 잰 폭(`useStripAvailable` — `TabBar.tsx`)이 줄고 Chrome 규칙대로 탭이 줄고 숨는다(`tabStrip/layout.ts` — 규칙 그대로 · R13).
- `WindowLayout.tsx` 가 지금 `TabBar` 를 놓는 자리에 `TitleRow` 를 놓는다. 대기 모드(예열 창)는 탭이 없어 `TitleRow` 를 그리지 않는다(지금처럼 바탕만 — ADR-0299).

### 7-2 끌 자리 — 기법 = ① Tauri `data-tauri-drag-region`(Q5 — 메인 결정 · 스파이크 실측 2026-10-24 · 사용자 위임)

- **다는 곳** — `TitleRow` · 탭 줄 행(`tab-strip-row`) · `GrabGap` 에 값 없는 속성(자기 자신만 — 자식 탭 · `+` · 단추는 끌기를 시작하지 않는다 · 확실 — Tauri `drag.js`). 번호 표식은 `pointer-events: none` 이라 누름이 부모(`TitleRow`)로 간다.
- **두 번 누름** = Tauri `drag.js` 의 `internal_toggle_maximize`(`core:default` 에 있다).
- **권한** = `core:window:allow-start-dragging`(main · 팝아웃 둘 다 — §9).
- **고른 근거** — S1: 그 영역 위로 끌어 온 탭 놓기가 닿았다(main 탭바에 들어감 · `dragover` 13 번 — 사용자 실측 2026-10-24). R14 · D9 · AC12 의 전제가 이 기법에서 선다.
- **잃는 것(받아들임 — 사용자에게 보고)** — 끌 자리 오른쪽 클릭 시스템 창 메뉴(`drag.js` 는 왼쪽 단추만 본다 — W6 · 재지 않음 · QA 가 적는다) · 터치 · 펜 끌기(재지 않음 · 미검). Alt+Space 는 §7-10.
- **걷은 갈래** = ② CSS `app-region: drag` — **그 위 놓기는 잃는다**(디버그 세션 2026-10-24 실측 — 확실): `app-region` 영역의 적중 판정이 최상위 창(HWND)으로 가고, 거기엔 tao 가 파일만 받는 `RegisterDragDrop` 을 걸어 두었다(tao `window.rs` — 우리 `webview_env` 의 `no_drop_handler` 는 웹뷰 것만 끈다) — 그래서 탭 놓기가 받는 쪽 없이 끝난다. Q5 = ① 을 굳힌다.

웹 코드는 속성을 한 곳(`titleRow/dragRegion.ts` — `dragProps()` 가 속성을 돌려준다)에 둔다. 스파이크의 실행 중 전환 · 세기 장치는 들이지 않는다.

- **두 번 누름 · 끌기는 버스를 지나지 않는다**(D7) — OS 제스처 층이다.
- **탭 둘레(R25 · AC30)** = 탭 줄 행의 탭 사이 · 마지막 탭 뒤 빈 곳(자기 자신) + `TitleRow` 의 보통 상태 위 여백(`TOP_GAP`). 탭 머리 요소는 지금 줄 높이 전체를 덮고 그 안 위 4 px(`SURFACE_TOP`)는 그리지 않는 머리 몫이다 — 그 4 px 를 끌 자리로 돌리지 않는다(머리 누름이 이긴다 — R19). Chrome 의 「비활성 탭 위 6 px」는 `TOP_GAP` 이 맡는다(§7-6).

### 7-3 번호 표식(R24 · AC29 · AC38)

- 글자 = 번호만(`String(number)`) · `font-variant-numeric: tabular-nums` · 12 px(탭 글자와 같다) · 색 = `--chrome-number-fg`(비활성이면 흐림).
- 폭 = `max(MARKER_MIN_W, 글자 폭 + 2 × MARKER_PAD)` — 자리 수만큼 넓어지고 자르지 않는다. 폭이 바뀌면 최소 창 폭 보고를 다시 한다(§7-7).
- 누르는 요소가 아니다 — `pointer-events: none` · Tab 순서 밖(`tabIndex` 없음).

### 7-4 창 단추(R10 · R21 · AC8 · AC23)

- 셋 다 `<button type="button">` · 탭바 뒤 Tab 순서 · Enter · Space · `:focus-visible` 테두리.
- 접근성 이름 · 툴팁(i18n `ko.ts` `window.*` 새 키): 「최소화」 · 「최대화」/「이전 크기로 복원」(상태 따라 — `frame.maximized`) · 셋째 = main 「모든 창 숨기기」 · 팝아웃 「창 닫기」.
- 아이콘 — 최소화 · 최대화/복원 = Windows 11 꼴(lucide `Minus` · `Square` · `Copy`) · 팝아웃 닫기 = `X` · 올림 = 빨강(`--caption-close-hover-bg` — Windows 11 의 닫기 빨강 · 값 미검 · S11) · **main 셋째 = lucide `EyeOff` · 올림 = 다른 단추와 같은 옅은 바탕(빨강 아님) · 툴팁 「모든 창 숨기기」**(결정 · 2026-10-24 — T3).
- 누르면 레지스트리 명령(§6-4)을 부른다. **Tauri 창 API 를 부르지 않는다**(§10-4).
- `caption.buttons == false`(macOS 몫 — 지금 없음)이면 그리지 않는다(R16).

### 7-5 비활성 흐림(R11 · AC15)

`TitleRow` 의 `data-window-active="false"`(창 사실 `focused` 거짓)일 때 탭 글자 · 번호 · 단추 글리프에 테마 토큰 `--chrome-inactive-fg` 를 쓴다(테마 셋 각각 — `src/styles/theme.css`). 끄는 동안의 흐림은 이 AC 밖이다(PRD AC15 · 슬롯 도킹 Q36).

### 7-6 크기 값(D8 — 메인 결정 · 기준 빌드 실측 S9 · S11 로 확정)

| 값 | 0.2판 제안(DIP = CSS px · 확대 1) | 근거 |
|---|---|---|
| 탭바 높이 | 28(그대로) | 지금 값(`TabBar.tsx` `height: '28px'`) — 탭 머리 모양이 그대로라 탭 폭 상수도 그대로다(§7-8) |
| 보통 상태 위 여백 `TOP_GAP` | 6 → 줄 높이 34 · **값은 S13 이 정한다**(0.3판) | 슬롯 도킹 R25 의 「최대화 아닐 때 비활성 탭 위 6 px」(Chrome) · 위 가장자리 크기 조절 띠와 겹치는 몫은 S9 · **접힌 끌기 대응**(W13 — 탭 줄이 창 맨 위에 붙으면 빠른 위쪽 끌기가 OS 끌기 시작 전에 창을 벗어난다 — 위 여백이 그 길을 출처 창 안에 둔다 · 메인 제안 · 사용자 확인 필요(QA)) |
| 최대화 위 여백 | 0 → 줄 높이 28 | Chrome 은 최대화(condensed) 때 끌 손잡이를 탭 위로 늘리지 않는다(Chromium `TabStrip::CanExtendDragHandle` — 사본 `tab_strip.cc`) |
| 끌 빈 자리 최소 `GRAB_MIN_W` | 42 · 번호 표식과 **따로** | Chromium `FrameGrabHandle::CalculatePreferredSize` = 42 · 늘 둔다(사본 `horizontal_tab_strip_region_view.cc` — 「탭줄이 차도 잡을 자리를 남긴다」). PRD R13 이 「번호 표식으로 갈음할지」를 TRD 에 맡겼다 — 갈음하지 않는다(메인 결정 M10) |
| 번호 표식 `MARKER_MIN_W` · `MARKER_PAD` | 28 · 8 | 메인 결정 — 한 자리 = 28 · 세 자리 ≈ 8 + 3 × 7 + 8 |
| 창 단추 | 폭 46 × 줄 높이 | Windows 11 캡션 단추 폭 46(관행 — 미검 · S11) |

값은 `src/components/layout/titleRow/metrics.ts` 한 곳(웹 — 정본)에 둔다. 셸은 최소 폭 보고(§7-7)로만 받는다.

### 7-7 최소 창 폭 · 저장 크기(R20 · D14 · D15 · AC22 · AC27 · AC34)

- **셈(웹 · 창마다)** = 탭 줄 왼 여백 + `TAB_MIN_ACTIVE_W`(52) + `+` 칸(지금 4 + 22 — `TabBar.tsx` `TAB_ADD_GAP` · `TAB_BUTTON_SIZE`) + `GRAB_MIN_W` + 번호 표식의 지금 폭 + 단추 3 × 46. 한 자리 번호에서 ≈ 286 + 왼 여백(미검 — 실측). 최소 높이 = 줄 높이 + 칸 최소(`min_pane_px` — ADR-0227 의 그 값).
- **보고** — 새 invoke `report_window_min_width { width, height }`(웹 → 셸 · 창 label 은 껍데기가 Tauri 창에서 꺼낸다 · `report_ui_metrics` 와 같은 꼴). 마운트 때 · 표식 폭이 바뀔 때 보낸다. **첫 화면 알림보다 먼저 보낸다** — 첫 화면 보이기 전에 넓히기가 끝나게(웹 `WindowLayout` 의 차례 — 보고 `await` 뒤 `reportFirstPaint`. 1.5 s 시한에 먼저 보인 창은 보인 뒤 넓혀질 수 있다 — T1 로 받아들임).
- **판정(순수 — `chrome::ChromeBook`, 리뷰 Claude ⑩)**:
  - `mark_opening(label)` — 창을 **열 때** 셸이 적는다: 창 포트의 `open` · `open_at`(새로 짓기) · 맡긴 예열 창의 `committed` · 부팅 복원 창 짓기(`state::placement` 의 main · 팝아웃) · 사본 수락의 숨긴 팝아웃 짓기(`RestoreWindows::open_hidden`).
  - `on_min_report(label, min, maximized) -> MinPlan { set_min: Option<(w, h)> /* None = 걷기 */, widen: bool }` — 보통 상태면 최소 크기를 입히고 최대화면 걷는다(R20 — 최대화 사각형 = 작업 영역). `widen` 은 그 창에 열기 표시가 있을 때만 참이고 표시를 거둔다 — **넓혀 당기기는 열 때 한 번뿐**이다(사람이 창을 끌어 좁은 모니터로 옮기는 동안은 OS 최소 크기만 걸린다 — 리뷰 codex ⑩ · PRD 0.5판 R20 · AC34).
  - `on_frame(label, maximized)` — 최대화가 바뀌면 최소 크기를 다시 입히거나 걷는다(창 사실 밀기 길 — §6-5 에서 부른다). 최대화에 최소 크기가 끼어드는지는 미검(S8) — 끼어들지 않으면 이 토글을 걷는다.
  - `forget(label)` — `Destroyed`.
- **넓혀 당기기(D14)** — `widen` 이면 그 창 보통 크기가 최소보다 좁을 때 최소 폭으로 넓히고, 넓힌 창이 작업 영역을 넘으면 자리를 작업 영역 안으로 당긴다 · 작업 영역 자체가 최소 폭보다 좁으면 왼쪽 끝 = 작업 영역 왼쪽 끝(R20). 셈 = `state::placement` 의 순수 함수 `fit_min_width(bounds, min_w, work_area) -> bounds`(그 모듈의 `resolve_place` 와 같은 층). 입히기 = `placement::move_steps`(어댑터 · 락 밖).
- **옛 파일의 안쪽 크기**(W5 · D14) — 저장 크기 = 안쪽 크기(코드 — `state/placement.rs` 머리)라 OS 틀이 없어지면 첫 부팅 창이 틀 높이만큼 짧아진다. 받아들인다(PRD D14 — 한 번뿐).

### 7-8 탭 폭 상수 다시 끌어내기(W3 · AC17)

탭 머리 모양(패딩 12 · 간격 6 · 단추 원 22 · 높이 28)이 이 TRD 에서 바뀌지 않으므로 다시 끌어낸 값은 **지금 값 그대로**다: 표준 `TAB_STANDARD_W` = 160 · 활성 최소 `TAB_MIN_ACTIVE_W` = 52 · 비활성 최소 `TAB_MIN_INACTIVE_W` = 32(`src/components/layout/tabStrip/layout.ts`). 바뀌는 것은 탭이 쓸 폭(`available`)뿐이고 배치(§7-1)가 재서 준다.

### 7-9 탭 놓기 판정 — 「탭바 끝」 구역을 제목 줄 전체로(R14 · D9 · AC12 — 리뷰 codex ①)

- **지금(코드)** — 놓기 판정의 탭바 구역 = 탭 줄 왼끝부터 `dragAreaWidth`(= 잰 `available`)까지 · 위아래 `STRIP_ZONE_PX`(`tabStrip/useTabDragSource.ts` 의 `geometryOf` → `dock/bands.ts` `stripZone`). 그 밖은 띠(캔버스) 아니면 「다른 곳」이고, 놓으면 새 창이다(`dock/windowDrop.ts` `deliver` 의 `to ??= { type: 'new_window' … }`). 제목 줄에 빈 곳 · 표식 · 단추가 서면 그 자리가 구역 밖이 되어 **빈 곳 · 표식 · 단추 위 놓기가 새 창이 된다** — R14 · AC12 와 어긋난다.
- **고침** — `useTabDragSource` 옵션에 `titleRow` 참조를 더하고 `geometryOf` 의 구역 사각형을 `{ x0: 탭 줄 왼끝, y0: 제목 줄 위, x1: 제목 줄 오른끝, y1: 제목 줄 아래 }` 로 넓힌다(위 여백 `TOP_GAP` 포함). 넣을 자리 셈은 그대로다: 남의 창 = `dropIndex` 가 모든 머리 가운데를 넘은 x 에 끝 번호를 준다(코드 — `reorderGeometry.ts` `dropIndex`) · 자기 탭바 = `draggedLeft` 가 `reorderAreaWidth` 로 잘라 끝자리에 머문다(코드 — `judge.ts` `stripAt`). 띠 빼기(`windowBands` 의 `zone`)도 같은 넓힌 구역을 본다.
- **단추는 놓기를 막지 않는다** — HTML5 놓기는 `click` 을 내지 않는다(가능성 높음 · AC12 가 잰다). 놓기는 뿌리 `dragover` · `drop` 이 받는다(ADR-0296).
- **웹뷰 밖 자식 창 위의 놓기는 잃을 수 있다**(W14 · 0.3판) — 위 ② 와 같은 길이 OS 틀을 걷은 창의 **위 가장자리 크기 조절 자식 창**(Tauri 코어 `undecorated_resizing` — 위 끝에 `SM_CYFRAME` 높이)과 스냅 오버레이 `STATIC` 자식 창(feature 를 켤 때만 — §8-4)에도 있을 수 있다(미검 — 추론). 그러면 넓힌 놓기 구역(위 여백 `TOP_GAP` 포함) 중 맨 위 몇 px · 최대화 단추 자리에 놓은 탭은 받는 쪽 없이 끝난다(출처는 「우리 창 위 = 아무 일 없음」 — 슬롯 도킹 Q38 — 이라 탭을 잃지는 않는다). **S14 로 잰 뒤 정한다**(메인 제안 · M33): ① 그 자리를 놓기 구역에서 빼고 받아들임 ② platform 에 최상위 창 수준 놓기 받는 쪽(`IDropTarget` — 우리 끌기 자료 형식만 받아 웹뷰에 넘김)을 둠 — ② 는 tao 의 파일 놓기 등록을 갈아끼우는 일이라 크다.
- **접힌 끌기와 출처 판정**(0.3판 · 기록) — 웹의 `tabStrip/dragSource.ts` 는 dragstart 와 같은 task 의 dragend 를 「엔진이 접은 끌기」로 보고 셸에 보내지 않는다(코드 — `Flight.sameTask`). W13 의 접힌 끌기는 dragend 가 **다음 task**(약 6 ms 뒤)에 와 그 판정을 지나쳐 셸로 간다 — 셸은 단추가 아직 눌린 끝을 취소로 읽어 아무 일도 하지 않으므로 지금은 해가 없다. 그 판정을 넓히는 것은 W13 의 원인을 가른 뒤다.
- **끌기 영역 기법과의 관계** — 이 구역은 그 자리에 `dragover` 가 와야 선다. ① 에서 온다(S1 — 끌기 영역에 `dragover` 13 번 · 놓기가 탭바에 들어감). 단 스파이크가 잰 것은 **빈 곳**이다 — 번호 표식 · 단추 · 위 여백 위의 놓기는 U8a 뒤 QA 가 AC12 로 다시 잰다.
- 끌 자리로 창을 옮긴 뒤 탭 누름 · `dragstart` 정상(tauri#10767) = S2 — `mouseup` 이 빠지는 것은 확인했고(누름 8 · 뗌 4 — OS 이동 루프 몫) 그 직후 누름이 먹히는지는 QA(AC13).

### 7-10 창 메뉴 · Alt+Space(Q10 — 메인 결정 · 사용자 위임 2026-10-23 · 2026-10-24 지시)

- **① 네이티브를 그대로 받아들인다** — 아무것도 짓지 않는다. Alt+Space · 끌 자리 오른쪽 클릭의 시스템 창 메뉴가 테두리 없는 창 + ① 끌기 영역에서 무엇을 내는지는 재지 않았다(S3 · S5) — **QA 가 결과를 적고**(AC23 ①: 뜨면 통과 · 안 뜨면 그 회귀를 PRD §2 비목표에 적고 사용자에게 보고) 받아들인다. 오른쪽 클릭 메뉴는 ① 에서 안 뜰 것으로 본다(W6).
- 걷은 갈래 = ② 우리 메뉴(Alt+Space · 끌 자리 오른쪽 클릭에 웹 메뉴 — 키 가로채기 · 메뉴 그리기 · 접근성 몫이 는다).

### 7-11 macOS 를 막지 않기(R16 · D10)

platform `window::caption_style() -> CaptionStyle { drawn_buttons: bool, leading_inset: f64 }` — Windows · 그 밖 = `{ true, 0 }` · macOS = `{ false, <신호등 폭> }`(값 미검 — 짓지 않는다). ★지금 코드는 모든 OS 에서 `{ true, 0 }` 이다★ — macOS 갈래는 macOS 이식 때 짓는다. 셸이 창 사실(`WindowFrame.caption`)에 실어 웹에 준다. 셸 `WebviewEnv::finish` 는 `drawn_buttons` 일 때만 OS 틀을 걷는다. macOS 의 「Overlay 제목 표시줄」 빌더 호출은 Tauri 의 macOS 전용 API 라 셸에 `cfg` 를 부른다 — 그 자리는 macOS 를 지을 때 정한다(지금 정하지 않는다).

---

## 8. 플랫폼 · 창 만들기

### 8-1 `WebviewEnv::finish`(ADR-0283 — 모든 창이 지난다)

- `EnvOptions` 에 `decorations(bool)` · `shadow(bool)` 를 더하고 `attach` 가 `caption_style().drawn_buttons` 일 때 `decorations(false)` · `shadow(true)` 를 붙인다 — 가짜 빌더 시험이 「붙였다」를 본다(그 모듈의 기존 시험 꼴). main(설정 창 `from_config`) · 팝아웃 · 예열 창이 모두 지난다(코드 — 그 모듈 머리).
- `tauri.conf.json` 의 main 선언에 `decorations` 를 적지 않는다(마무리가 덮는다 — 한 곳).
- 크기 조절 테두리 = Tauri 코어의 테두리 없는 창 크기 조절(조사 — `tauri-runtime-wry` `undecorated_resizing.rs` · 확실) · 그림자 · 둥근 모서리 · 1 px 테두리 색 = S10.

### 8-2 platform crate(`crates/engram-dashboard-platform/src/window.rs`)

더하는 것 = `caption_style()`(§7-11 · U7a) · `is_foreground(w)`(§6-2 · U7a) · feature `snap-overlay` 뒤 `snap` 모듈(§8-4 · U9). `cfg` 는 각 함수 안 · 그 모듈 안에만. 0.2판의 `restore_no_activate` 는 걷었다 — 펴기는 Tauri `unminimize()` 로 한다(§6-2 · 0.3판 재리뷰 ①).

### 8-3 게이트 영향(CLAUDE.md 「플랫폼 중립」 · `/qa` 4f · 4g · 4h)

- 4f(OS `cfg` 파일 명단 — platform **밖**) — 바뀌지 않는다: 새 OS `cfg` 는 platform 안뿐이다. 셸의 `#[cfg(feature = "snap-overlay")]` 는 OS 술어가 아니다(4f 의 matcher 는 OS 술어를 본다 — 정본 = platform `src/lib.rs` 헤더 게이트 ④ · 그 matcher 가 `feature` 를 잡지 않는다는 것은 U9 가 확인 — 미검).
- 4g(OS crate 운영 의존 = platform 하나) — 바뀌지 않는다. 스냅 오버레이가 쓰는 `windows` 바인딩(`Win32_UI_WindowsAndMessaging` · `Win32_UI_Input_KeyboardAndMouse`)은 이미 켜져 있다(코드 — platform `Cargo.toml` · 스파이크가 지금 feature 로 빌드됐다 — 가능성 높음).
- 4h(`std::os::windows` 파일 명단 — platform 밖) — 바뀌지 않는다.
- 셸 쪽 새 코드(`chrome/` · `commands/window_ctl.rs`)에 OS `cfg` 를 두지 않는다.

### 8-4 스냅 레이아웃 오버레이 — 켜지 않는다 · 꺼진 feature 뒤에 둔다(Q1 — 결정 · 2026-10-24)

- **결정** — 스냅 레이아웃 메뉴(최대화 단추에 올리면 뜨는 배치 고르기)를 이번에 켜지 않는다. 회귀로 받아들인다(PRD R15 · §2 — 화면 끝 끌기 스냅 · Win+화살표는 남는다). AC19 는 없다.
- **코드는 남긴다 — 주석이 아니라 cargo feature 로**(메인 결정 M19) — 주석으로 막은 코드는 컴파일되지 않아 썩고, 켜는 날 고칠 것이 쌓인다. feature 로 두면 켠 빌드를 CI 가 컴파일해 썩지 않는다.
  - platform `Cargo.toml` `[features] snap-overlay = []` · 모듈 `window::snap`(스파이크 `t66_snap_spike.rs` 를 옮겨 다시 짓는다 — 참고한 `Hyph-M/tauri-plugin-snap-layout` MIT 귀속을 그 모듈 머리에 남긴다) 전체가 `#[cfg(feature = "snap-overlay")]` 뒤.
  - 셸 `src-tauri/Cargo.toml` 에 `[features] snap-overlay = ["engram-dashboard-platform/snap-overlay"]`(셸의 첫 `[features]` 표 — 기본 꺼짐). 부르는 자리(invoke `report_max_button_rect` · 그 어댑터)와 그 invoke 등록이 같은 feature 뒤.
  - 웹은 창 사실의 `caption.snap_overlay` 가 참일 때만 최대화 단추 사각형을 보고한다(feature 를 끈 빌드는 늘 거짓 — 웹 코드는 늘 컴파일된다).
- **CI · `/qa` 몫(U9)** — ① 켠 빌드 컴파일: `cargo check --locked -p engram-dashboard --features snap-overlay`(그리고 platform 몫 `cargo check --locked -p engram-dashboard-platform --features snap-overlay`) ② 운영 그래프에 꺼져 있음: `cargo tree --locked -p engram-dashboard -e normal,features -i engram-dashboard-platform --target all | rg 'feature "snap-overlay"'` → 0 줄(짝 = 같은 명령에 `--features snap-overlay` 를 주면 1 줄 이상 — 기능 이름이 바뀌어 0 기대 줄이 눈먼 채 통과하지 않게 · base · platform 의 `test-support` 게이트와 같은 꼴). CLAUDE.md 「빌드·검증 명령」에 켜는 법 한 줄을 더한다: 「스냅 오버레이를 켠 빌드 = `--features snap-overlay`(셸) — 기본 꺼짐 · 사용자 결정 2026-10-24」.

---

## 9. 권한(capability)

| 파일 | 지금(커밋된 것) | 바꿈 | 까닭 |
|---|---|---|---|
| `default.json`(main) | `core:default` · `core:event:default` · `dialog:allow-open` | `core:window:allow-start-dragging` 더함(Q5 = ①) | Tauri `drag.js` 의 `start_dragging` · 두 번 누름의 `internal_toggle_maximize` 는 `core:default` 에 있다(조사 결론 1 — 확실). 빠뜨리면 끌기가 소리 없이 안 된다(W4) |
| `popup.json`(`slot-popup-*`) | 위 + `core:window:allow-close` | 같은 더함 · **`core:window:allow-close` 걷음** | 0탭 자가닫힘이 버스로 가면(§5-5) 웹이 창 닫기 API 를 부를 일이 없다 — 권한을 걷어 AC10 을 권한으로도 막는다 |

- 단추 · 상태 명령은 우리 invoke(`window_control`)라 `core:window:allow-minimize` · `allow-toggle-maximize` 등을 더하지 않는다. 스파이크가 더한 넷은 U0 이 되돌리고, `allow-start-dragging` 만 U8a 가 다시 더한다(쓰는 단위와 함께 — 그 전에는 쓰는 곳이 없다).
- `src-tauri/gen/schemas/capabilities.json` 은 빌드가 다시 굽는다(스파이크가 바꾼 것 되돌림).

---

## 10. 테스트 계획

### 10-1 Rust 단위(`cargo test -p engram-dashboard --test lib_unit` · platform · 데몬)

| 모듈 | 잴 것 | AC |
|---|---|---|
| `numbering` | `smallest_free`(빈칸 · 끝 · 쥔 집합) · `resolve_saved` 표 시험 — 옛 파일(전부 없음 → 1·2·3) · 두 팝아웃 3 → 앞 3 뒤 2 · 팝아웃의 1 · 0 · -1 · 「x」 · 상한 넘음 → 빈 가장 작은 · `held` 에 든 번호 → 비켜섬 | AC25 · AC38 · AC31(순수) |
| `manager` 번호 | 1·2·3 에서 2 닫기(`closing`) → 새 창 = 4 → `release_closing` 뒤 새 창 = 2 · 거의 동시 둘 닫기 · 예열 창 label `Destroyed` 무동작 · `drop_window_gone` 곧바로 빔 · 모델에 남은 채 `Destroyed` → 곧바로 빔 · 숨은 main 1 · `adopt_restored` 가 `closing` 을 비우고 사본 번호를 세움 · 넣기/빼기가 사설 메서드만 지남(창 수 · 번호 집합 불변식 검사 도우미 — `#[cfg(test)]` — 를 모든 쓰기 시험 끝에 부른다) | AC1 · AC20 ①–⑤ |
| `state::placement` · `state::restore` | 못 연 복원 팝아웃 · 사라진 사본 창이 번호를 곧바로 내놓음(그 다음 새 창이 그 번호) | AC20 · AC25 |
| `tab_name`(U1) | `SlotContent` 변형마다 낱말 · 에이전트 이름 있음/없음 · `window_title` 세 꼴 · 200 자 · 이모지 · 한글 자르지 않음 | AC3 · AC21 |
| `tab_name`(U3a) · `manager` 이름 | 정한 이름이 이김 · 포커스 칸이 트리 밖 → 첫 슬롯 · 모르는 내용 슬롯 · 새 탭 빈 칸에 트리를 놓으면 그 낱말 · 기본 이름 걷힘 · `rename_tab` 빈 이름 거절 · 같은 글자 이름 바꾸기 = `Named` · 이름 장부 갱신이 바뀐 창만 고르고 `version` 을 올림(바뀐 것 없으면 안 올림) | AC4 · AC35 · AC36 |
| `window_arg` | 표(§4-5) 전부 · 앞 실행 label · 닫힌 창 영속 id | AC26 · AC28 |
| `window_ctl` | main 닫기 = `hide_all` · 숫자 인자 = `INVALID_ARGUMENT`(`if_empty` 여도 — 그 앞) · `if_empty` 가 해석기 앞에서 갈림(모델에 있음 → Kept · 없음+closing → 그 label 닫기 · 다른 id → Kept) · `plan` = §5-2 표 전부(첫 화면 대기 · 숨김 × 다섯 동사) · **메인 스레드 줄 흉내**(§5-2 — 보이기 일 · `maximize` 일 두 차례 · 끝에 미룸 표가 빔 · `unmaximize` · 숨김 · `NoSteal` 이 대기 창을 건너뜀) · `raise`(숨음 → `NoSteal` 먼저 · 전경이 남의 것으로 꾸미면 `set_focus` 를 부르지 않고 `focused: false`) | AC8 · AC9 · AC24 · AC26 · AC32 |
| `tray::actions` | `show_ui_windows` 의 두 갈래 — `TrayClick` = 지금 차례 그대로(기존 시험) · `NoSteal` = 대기 창 건너뜀 · `set_focus` 는 전경이 우리일 때 main 에만 | AC9 |
| `chrome::book` | 제목 판 — 옛 판 버림 · 같은 제목 건너뜀 · 잊은 뒤 온 늦은 일(창 없음)은 항목을 만들지 않음 · 최소 크기 — 보통 = 입힘 · 최대화 = 걷기 · 최대화 바뀜 토글 · 넓히기 = 열기 표시가 있을 때 한 번 · 표시 없는 보고는 넓히지 않음 | AC3 · AC22 · AC27 · AC34 |
| `codec` · `convert` | `number` · `named` 너그러운 읽기(창 · 탭을 건너뛰지 않는다) · 왕복 · 시드 규칙(`named` 없음은 시드 안 함) · 「같은 JSON」 시험 고침 | AC25 · AC37 |
| `popout` | label 꼴 · 실행 표식 고정 · `cascade_position` 이 끝 수를 푼다 | AC1 · AC28 |
| `placement` | `fit_min_width` — 좁은 저장 크기 · 작업 영역 오른 끝 · 작업 영역 < 최소 폭 → 왼 끝 맞춤 | AC27 · AC34 |
| `webview_env` | 가짜 빌더에 `decorations(false)` · `shadow(true)` 가 붙음 | AC5 |
| `tab_drop` | `WindowFrame` 칸 · 바뀔 때만 밂(`focused` 포함) | AC15 |
| platform | `caption_style` · `is_foreground`(Windows 실 창 — 기존 `test_window` 도우미) | AC15 |
| 데몬 `connection_core` | `Renamed` 뒤 · `RosterFanout::roster_changed` 도 `AgentListUpdated` 가 나가고 그 이름이 새 이름이다 | AC4 |

### 10-2 통합(`--test layout_apply` · `--test layout_commands`)

- `layout_apply`(공개 API + 가짜 포트만 — §2-3) — 세 자리 각각 × (예열 창 · 새로 짓기) → 답의 `window_id` · `number` · 알림 페이로드의 `number` · `title` · `version` · 제목 불변식(제목이 바뀌는 쓰기마다 그 창 페이로드가 새 판 번호로 나간다 — 기록형 이벤트로). 이름 장부 갱신 → 바뀐 창만 알림 · 판 오름. 가짜 `WindowHost::close` 가 아무것도 하지 않는 닫기 뒤 `window_ctl::close_if_empty` 가 닫는다(AC24 의 0탭 갈래) · 그사이 탭을 넣으면 안 닫는다(AC32).
- `layout_commands` — 선언: 새 명령 · `window.list` 의 `windows` 유지 + `details` · `window.create` · `slot.popout` 답 칸 · `window.close` 답 · `if_empty` 가 없는 창에 `CONFLICT` 를 내지 않음 · 번호 인자 `INVALID_ARGUMENT` 와 도움말 문구 · 앞 실행 label `CONFLICT` · `tab.list` `named` · 세대 번호.
- 셸 `daemon_client/tests.rs` — 기록형 `AgentNamesSink` 가 실 소켓 왕복에서 목록을 받는다.

### 10-3 TS(vitest)

`TitleRow` 배치 순서(탭바 → 빈 곳 → 표식 → 단추) · 표식이 Tab 순서 밖 · `pointer-events: none` · 자리 수별 폭 · 단추가 레지스트리 명령을 부름 · 최대화 상태에 따라 단추 · 접근성 이름 · main/팝아웃 셋째 단추 이름 · 아이콘(`EyeOff`) · `frame.focused` → 비활성 속성 · 최소 폭 셈과 보고 차례(보고 뒤 첫 화면 알림) · 0탭 효과가 `window_control{Close, if_empty}` 를 자기 `window_id` 로 · `viewStore` 의 `number` · `windowId` 칸 · 탭바 `available` 이 표식 · 단추 · 빈 곳을 뺀 폭 · **놓기 구역**(`geometryOf` · `windowDrop.decide` — 빈 곳 · 표식 · 단추 · 위 여백 위 점 = `tab_bar` 끝 번호 · 제목 줄 밖 = 지금처럼) · **이름 편집 확정**(자동 이름 탭에 같은 글자 Enter = 보냄 · 같은 글자 포커스 잃음 · 이웃 누름 = 안 보냄 · 정한 이름 탭에 같은 글자 = 안 보냄) · `windowFrame.ts` 가 새 칸을 읽고 모양이 틀리면 버림.

### 10-4 구조 게이트(CI · `/qa` 바인딩 — U1 · U7b 가 더한다)

- **D17** — CI `shell gate 2`(판정 함수는 그 스텝 하나 · `/qa` 바인딩 4j 가 그것을 부른다)가 `src-tauri/src/layout/tab_name.rs` 의 `slot_label` 본문(0 열 `pub fn slot_label` 줄부터 첫 0 열 `}` 까지 · `//` 와 한 줄 `/* */` 주석을 걷은 것 — 같은 파일의 다른 함수 · 시험은 보지 않는다)을 **양성으로** 잰다. ① 변형 덮기 — `types.rs` 의 `pub enum SlotContent` 블록에서 뽑은 변형이 전부 본문에 `SlotContent::<변형>` 으로 나온다(블록을 못 찾거나 변형이 0 개면 FAIL). 새 변형을 묶음 갈래가 받으면 문법과 상관없이 걸린다. 범위가 잘리면 잘린 자리가 어느 변형 갈래보다 앞일 때만 그 변형이 빠져 닫힌 쪽으로 실패한다 — 모든 변형 갈래 뒤에서 잘려 그 뒤에 숨은 묶음 갈래는 지금은 안 보이고 다음 변형을 더할 때 ① 이 잡는다. ② 갈래 모양 — `match` 줄 들여쓰기 + 4 가 갈래 자리이고, 그 자리 줄은 `=>` 를 많아야 하나 담고(한 줄에 둘째 갈래를 이어 쓰면 FAIL), `}` · `},` · `} => …`(여러 줄 구조 패턴의 끝) 말고는 `=>` 앞이 비지 않아야 하며 앞의 `|` 를 뗀 뒤 `=>` 앞을 `|` 로 가른 모든 갈래가 `SlotContent::` 로 시작해야 한다. 그 밖 자리의 `=>`(중첩 `match` · rustfmt 가 다음 줄로 내린 가드 등)는 FAIL 이다. 짝 = 0 열 `pub fn slot_label` 하나와 그 앞의 `#[deny(clippy::wildcard_enum_match_arm)]`(사이 `#[…]` · `//` 줄 허용) · 그 파일에 이 lint 를 끄는 `allow` · `expect` · `warn` 없음 · 그 파일 어디에도 rustfmt 건너뛰기(`rustfmt::skip` · `rustfmt_skip`) 없음(② 가 기대는 rustfmt 꼴을 끈다 — U1 리뷰 3) · 주석 밖 `match` · 도우미 자기 시험(갈래 모양 위반 · 위반 아닌 표본과 한 변형이 빠진 가짜 enum/본문 — 수 고정). **의도는 0.3판 그대로 — 새 변형은 이름을 짓기 전에 통과하지 못하고 와일드카드 · 묶음 이름 갈래를 두지 않는다**(메인 결정 · U1 리뷰 2 · 2026-10-24 — 위반 꼴을 정규식으로 열거하던 음성 게이트(0.3판 한 줄 · 리뷰 1 의 네 갈래)는 `Other =>` · `other | other =>` · `other @ &_ =>` · 원시 문자열의 0 열 `}` 로 범위가 잘려 뒤 갈래가 빠지는 것 · 주석 글자 오탐에 계속 샜다. 허용 꼴만 통과시키는 양성 검사는 그 열거가 필요 없다). 그래서 `slot_label` 은 0 열 함수로 두고 본문에 `match` 하나만 담으며, 갈래는 `SlotContent::<변형>` 경로 패턴만 쓴다(가드의 `||` · `crate::…` 경로 · 중첩 `match` 는 닫힌 쪽으로 걸린다 — 파일 머리에 적는다). CI 에 clippy 가 없어 함수 attribute 만으로는 지켜지지 않는다(메인 결정 M15). 기대는 것 = rustfmt(`cargo fmt --check`)가 갈래를 줄마다 4 칸 들여쓰기로 놓는다는 것. 남는 구멍 = 여러 줄 블록 주석 · 매크로가 짓는 갈래 · 본문 문자열 속 `SlotContent::<변형>`(① 을 속인다 — 그때도 ② 와 컴파일러의 망라 검사가 남는다). **일부러 속이는 꼴 셋은 받아들인 채 사용자 결정을 기다린다**(U1 리뷰 3 에서 다툼 — 가짜 `match` 를 담은 원시 문자열 · ① 을 채우는 문자열 속 `SlotContent::<새 변형>` · 망라한 `match` 를 닿지 않게 두고 그 앞에서 묶음 갈래를 가진 도우미로 일찍 돌아가기 — 코드 리뷰가 본다).
- **AC10(웹이 창 제어 API 를 부르지 않는다)** — 0.1판의 rg 한 줄은 줄바꿈 · 별칭(`const w = getCurrentWindow(); w.close()`)에 뚫린다(리뷰 Claude ⑦). 대신 vitest 시험 하나(`src/__tests__/noWindowControl.test.ts`)가 TypeScript 컴파일러 API 로 `src/**/*.{ts,tsx}`(시험 파일 제외)를 프로젝트 `tsconfig` 로 타입 검사해, **받는 쪽 타입이 `@tauri-apps/api` 의 `Window` · `WebviewWindow` 인 속성 접근** 중 이름이 `close` · `destroy` · `minimize` · `maximize` · `unmaximize` · `toggleMaximize` · `show` · `hide` · `setFocus` · `unminimize` 인 것을 찾으면 실패한다(메인 결정 M25 — `typescript` 는 이미 개발 의존이다 · `npx tsc --noEmit` 이 쓴다). 별칭 · 줄바꿈 · 구조 분해(`const { close } = getCurrentWindow()` — 그 꼴은 바인딩 요소의 원천 타입으로 같이 본다)를 타입으로 잡는다. 지금 `getCurrentWindow` 를 쓰는 자리(label · `listen` 몫 — 코드)는 걸리지 않는다.
- CLAUDE.md 「LLM-우선 제어」 찾는 법 결과가 이 작업 앞과 같다(AC10 — 새 전역 핸들 0).
- 스냅 오버레이 feature 게이트 둘(§8-4).

### 10-5 화면 실측 · 실제 입력(`/qa full` · 사용자 체크리스트)

CDP(`scripts/cdp.mjs`)가 재는 것 — DOM 배치 · 번호 · `window.list` · 명령 답 · 접근성 이름 · 테마 칠 · OS 제목(PowerShell 창 목록 — 스파이크 첨부 `cdp-native-windows-2026-10-21.ps1` 꼴). OS 끌기 루프 · 스냅 · Win 키 · Alt+Space · 실제 마우스 끌기는 자동 입력 금지(핸드오프 규약)라 사용자 체크리스트로 돌린다.

| AC | 단위 | 재는 법 |
|---|---|---|
| AC1 · AC2 · AC20 · AC25 · AC26 · AC28 · AC31 · AC38 | U2 · U5 · U6 | 단위 · 통합 + CDP(번호 표식 · `window.list` · OS 제목 셋 대조) · 재시작 셋 · AC20 ② = 단위 하네스 |
| AC3 · AC4 · AC21 · AC35 · AC36 | U3 · U4 | 단위 + CDP(탭 머리 · OS 제목) |
| AC5 · AC14 · AC18 · AC27 · AC34 | U8b | CDP 스크린샷 + 사용자(배율 셋 · 작은 화면에서 열기) |
| AC6 · AC7 · AC16 · AC30 | U8a · U8b | 사용자 체크리스트(끌기 · 두 번 누름 · Win+화살표 · Win+Z) — 기준 빌드와 견줌 |
| AC8 · AC9 · AC15 · AC23 · AC24 · AC29 | U7 · U8a | CDP(명령 · 상태 칸 · 접근성) + 사용자(Alt+F4 · 내레이터 · Alt+Space · 다른 앱 앞에서 `window.raise`) · AC24 의 0탭 갈래 = 단위 하네스 |
| AC10 | U7b · U8a | §10-4 |
| AC11 · AC12 · AC13 | U8a · U8b | 슬롯 도킹 PRD 4.9 AC 다시 잼(사용자 실제 끌기) · S1 · S2 · 놓기 구역 vitest |
| AC17 · AC22 | U8a · U8b | CDP(창 폭 줄이기) |
| AC32 · AC37 | U7a · U2 | 단위 하네스(AC37 = 저장 기록기의 기존 실패 · 원자 쓰기 시험 + 새 칸 왕복) |
| AC19 · AC33 | — | 없음(Q1 · T1) |

- **숨은 창 보이기의 전경**(AC9 · U7a) — 다른 앱을 앞에 둔 채 숨은 main 을 `window.showAll` · `window.raise` 로 보이고, 전경이 옮겨 가는지를 갈래마다 적는다: main 이 숨기 전 최대화였나 · 아니었나 × 바람 없음 · `window.unmaximize` 로 적은 `false` 바람(숨은 팝아웃에 `window.maximize` 로 적은 `true` 바람도 같이 본다). 활성화가 끼어들 자리가 셋이다 — tao `show()` 가 내는 `SW_SHOW`(포커스를 받게 지은 창이면 활성화한다) · 바람을 입히는 `SW_MAXIMIZE` · 펴기와 원래 크기 입히기의 `SW_RESTORE`(둘 다 활성화를 겸한다). 옮겨 가면 AC9 위반이다. LLM 길에서 이것이 지켜지는지는 모른다(M22).
- **S12 — 앞 창 칸 두 출처**(U7a) — 사건 계기(Alt+Tab · 다른 창 누름 · 끌 자리 누름)마다 ① `window:frame` 의 `focused`(그 창의 `Focused` 사건 값) 와 같은 순간 `window.list` 의 `focused`(`is_foreground`)가 어긋나는지 ② 메인 스레드 밖에서도 불리는 자리 기록(`state::placement::record`)이 민 `window:frame` 과 `Focused` 밀기의 차례가 뒤바뀌어 낡은 `focused` 가 마지막에 남는지를 함께 적는다(§6-5).

---

## 11. 구현 단위

> 규칙 — 각 단위 끝에서 빌드 · 워크스페이스 회귀 · 생성물 sync 가 초록이고 앱이 쓸 수 있는 모습이다(OS 틀은 웹 제목 줄이 선 뒤 U8b 에서 걷는다). 단위 안에서도 「자료구조 → 호출부」 순서를 지켜 중간에 멈춰도 컴파일이 서게 한다(타입을 바꾸는 단위는 생성자 · 읽기 도우미를 먼저 두고 호출부를 옮긴다). 코더 한 명이 한 단위. `/review code` · `/qa` 는 단위마다. 0.1판과 번호가 바뀐 곳 = §15-3.
>
> 머지 — **U5 · U6 은 U7a 없이 master 에 머지하지 않는다**: 창 인자 도움말 · 번호 거절 문구가 `window.list` 의 window_id · details 를 가리키는데 그 칸은 U7a 가 싣는다.

| 단위 | 내용 | 기대는 것 | 크기 · 비고 |
|---|---|---|---|
| **U0** | 스파이크 코드 걷기 — `SPIKE T-66` 배선 전부(제목 줄 · 끌 자리 · 세기 장치 · 스파이크 invoke · `t66_snap_spike` 모듈 · 권한 넷 · `WebviewEnv::finish` 의 `decorations(false)`)를 되돌린다. 스냅 오버레이 몫은 U9 가 옮길 원본으로 첨부 패치(`t66-titlebar-spike-2026-10-23.patch`)에 남는다. 스파이크 결과(§1-3)는 이미 접었다 | — | 작음. 되돌린 뒤 트리 = master 와 같은 동작 |
| **U1** | 순수 몫 — `numbering.rs` · `tab_name.rs` 의 `slot_label` · `window_title` · `word` · 단위 시험 · D17 rg 게이트(CI · `/qa`) | — | 작음. 홀로 빌드된다(`View` · `ViewManager` 를 모른다). 호출부 없음 — 미사용 경고는 `#[cfg_attr(not(test), allow(dead_code))]` 를 U2 · U3a 가 걷는다 |
| **U2** | 번호 — `WindowTabs.number` · `closing` · `insert/retire_window_entry` · `release_closing` · `drop_window_gone`(+ `placement` · `restore` 두 자리) · `drop_window_in_model` 원인 · `adopt_restored` · `from_restored` · 페이로드 `number` · `window_id` · 영속(`WindowEntry.number` · 너그러운 읽기 · `resolve_saved`) · 바인딩 | U1 | 중간. `manager.rs` · `state/*` · `commands/popout.rs` · `output_router.rs` |
| **U3a** | 탭 이름 — `TabName` · `effective_tab_name` · 빈 장부 칸 · 기본 이름 걷기(`LabelSource::tab_name` 포함) · `ViewMeta` 유효 이름 · `named` · `rename_tab` 빈 이름 거절 · `TabEntry.named` · 「같은 JSON」 시험 고침 · 버스 `TabRow.named`(세대) · 바인딩 · 웹 `commitEdit` 같은 글자 확정 | U1 · (U2 — 같은 파일이라 뒤에) | 중간 |
| **U3b** | 이름 장부 채우기 — `AgentNamesSink` · `apply::agent_names_updated`(판 오름) · 데몬 개명 뒤 `AgentListUpdated` · 부팅 · 사본 수락 시드 | U3a | 중간. 데몬 crate 를 건드린다 |
| **U4** | 창 제목 — 페이로드 `title` · `chrome::ChromeBook`(제목 판 몫) · `TauriEvents` 가 메인 스레드에서 새 판만 `set_title` · `Destroyed` 때 잊기 · `create_window` 알림 · `sync_titles`(부팅) · 빌드 제목 「Engram」 · `tauri.conf.json` main 제목 · 슬롯 쓰기의 탭 알림 전수 대조 | U2 · U3a | 작음–중간 |
| **U5** | 창 인자 — label 실행 표식 · `cascade_position` · `window_arg::resolve` 를 버스 동사 · invoke 껍데기에 · 번호 거부 · 도움말 · 세대 | U2 | 작음–중간. `commands.rs` · `commands/layout.rs` · `popout.rs` |
| **U6** | 만들기 답 — `window.create` · `slot.popout` · invoke `create_window` · `move_slot_to_window` · `DockOutcome` · 웹 `viewStore.createWindow` · 레지스트리 `window.create` 가 `{window, window_id, number}`(세대) | U2 · U5 | 작음(0.1판 U7 에서 첫 화면 기다림을 걷고 남은 몫) |
| **U7a** | 창 상태 명령(셸) — `window_ctl` · `WindowControl` · Tauri 어댑터 · §5-2 보이지 않는 창 규칙 · 모든 보이기 갈래의 미룬 최대화 · 버스 명령 · `window.list` details · `window.close`(main · `if_empty` 를 해석기 앞에서) · invoke `window_control`(async) · 메인 스레드 일 하나로 읽기 · `plan` · 입히기(§5-2) + 메인 스레드 줄 흉내 시험 · 트레이 보이기의 `Activation` 갈래(`NoSteal` = 대기 창 건너뜀 · `raise_and_show` 차례) · invoke `show_main_ui` → `NoSteal` · platform `is_foreground` · **platform `caption_style()`**(이 단위가 `WindowFrame.caption` 을 채운다 — 0.3판 재리뷰 ④) · `WindowFrame.focused`(사건 값) · `caption` · `Focused` → 밀기 | U2 · U5 | 큼 |
| **U7b** | 창 상태 명령(웹) — 레지스트리 · `viewStore` 액션 · 0탭 자가닫힘 → 버스 · `popup.json` 의 `allow-close` 걷기 · `windowFrame.ts` 새 칸 · AC10 타입 검사 시험 | U7a | 중간. 접점 = invoke `window_control` 모양 · `WindowFrame` 모양 하나씩(U7a 가 먼저 못 박는다) |
| **U8a** | 제목 줄(웹) — `TitleRow` · `NumberMarker` · `CaptionButtons`(T3) · `metrics.ts` · i18n · 테마 토큰 · 흐림 · 끌 자리 속성(`data-tauri-drag-region` — Q5) + `core:window:allow-start-dragging`(main · 팝아웃) · **놓기 구역 넓히기**(`useTabDragSource` · `windowDrop`) · 최소 폭 셈과 보고 · `report_window_min_width` 껍데기(입히기는 U8b) — OS 틀은 아직 있다(OS 제목 줄 아래 우리 제목 줄 — 그대로 쓸 수 있다) | U4 · U7b · U0 | 큼. 시작 전에 기준 빌드(OS 제목 표시줄)의 줄 높이 · 단추 폭을 떠 둔다(S11) |
| **U8b** | OS 틀 걷기 — `WebviewEnv::finish` 의 `decorations(false)` · `shadow(true)`(`caption_style()` 을 읽는다 — U7a 가 지었다) · 줄 높이 보통/최대화 · `ChromeBook` 최소 크기 몫 · 열기 표시 · `fit_min_width`(D14) · QA 가 S8–S10 · AC13(창 끌기 뒤 누름) · AC23(Alt+Space · 오른쪽 클릭 — 결과만 적는다)을 잰다 · **측정 문 S13 · S14**(접힌 끌기 · 놓기 잃음 — §1-3 · W13 · W14) — 통과 전에는 U8b 를 닫지 않는다 | U8a | 중간 |
| **U9** | 스냅 오버레이를 꺼진 feature 뒤로 — platform `window::snap`(스파이크 옮김 · MIT 귀속) · 셸 `[features] snap-overlay` · invoke `report_max_button_rect`(feature 뒤) · 웹 보고(`caption.snap_overlay` 일 때만) · CI 컴파일 · 그래프 게이트 · CLAUDE.md 켜는 법 | U8b · U0(원본 패치) | 중간. 스파이크에서 메뉴가 떴다(S6) — 옮긴 뒤 켠 빌드의 동작 확인은 켜는 날 한다 |
| **U10** | 문서 — ADR(§12-3) · CLAUDE.md(「LLM-우선 제어」 남은 갭 · platform `window` 입주자 · 의존 절 · 스냅 feature) · step-log · tracking(T-66 · T-68 몫) · 코드 앵커 · `/qa` 바인딩 게이트 · `prompts/engram-help.md`(창 인자 · window_id · 번호 거부 — U7a 뒤) | 앞 단위 전부 | `/review doc` |

---

## 12. 결정

### 12-1 사용자에게 남은 것

**사용자 결정 대기는 없다.** Q5 · Q10 은 메인이 스파이크 실측(2026-10-24)으로 정했다(위임 — §12-3 M27 · M28). 사용자에게 **보고**만 남는다: Q5 = ① 로 잃는 것(끌 자리 오른쪽 클릭 창 메뉴 · 터치 끌기 미검) · QA 가 적을 결과(Alt+Space · 오른쪽 클릭 · 창 끌기 직후 누름 · Win+Z — 안 되는 것이 나오면 PRD §2 비목표에 적고 보고). **QA 에서 사용자 확인이 필요한 것 하나**(0.3판) — 보통 상태 위 여백(`TOP_GAP`)을 접힌 끌기 대응으로 두는 것(S13 · W13 — 메인 제안 · 측정으로 값을 정한 뒤 사용자가 모양을 확인한다).

### 12-2 사용자 결정(2026-10-24)

| # | 결정 | 반영 |
|---|---|---|
| T1 | 첫 화면 전 실패를 따로 처리하지 않는다 — 지금처럼 시한에 보인다 | §5-3 · 0.1판 §5-2 · §5-3 · U7(옛) 걷음 · PRD R1 · R6 · AC2 · AC33 · D13 |
| Q1 | 스냅 레이아웃 메뉴를 켜지 않는다 · 코드는 남긴다 | §8-4 · U9 · PRD R15 · AC19 · §2 |
| T2 | 낱말 = 빈 칸 · 에이전트 · 에이전트 트리 · 프리셋 · 사용량 | §4-2 · PRD R4 |
| T3 | main 셋째 단추 = `EyeOff` · 옅은 올림 바탕 · 툴팁 「모든 창 숨기기」 | §7-4 · PRD R10 |

### 12-3 메인 결정(근거는 본문 — 사용자가 바꿀 수 있다)

| # | 결정 | 절 |
|---|---|---|
| M1 | 번호 표 = `ViewManager` 안 · 셈 = 순수 `numbering.rs` | §2-1 · §4-1 |
| M2 | D13(닫히는 창 번호 쥐기)을 남긴다 — 싸다 · OS 창이 이미 없는 창은 곧바로 푼다 | §4-1 |
| M3 | 저장 번호 상한 999 | §3-4 |
| M4 | 탭 유효 이름은 셸이 짓고 낱말은 셸 표 한 곳(PRD 기본값 「i18n」 에서 벗어남) · `View.name` → `TabName`(M4′) · 「알 수 없는 내용」을 남김(모르는 내용 슬롯 — `SlotContent` 갈래 아님) | §1-4 · §3-1 · §4-2 |
| M5 | 이름 장부 = 모델 안 · 출처 = `AgentInfo.name` · 데몬 개명 뒤 `AgentListUpdated` · 유효 이름이 바뀌면 판 오름 | §4-3 |
| M6 | OS 제목 = 탭 알림 페이로드 `title` 을 Tauri 어댑터가 메인 스레드에서 새 판만 입힘 · 새 포트 없음 · 불변식 2 | §4-4 |
| M7 | 창 상태 = 새 포트 `WindowControl` · 서비스 `window_ctl` · invoke 하나 · 버스 이름(`unmaximize` · `raise` · `hideAll` · `showAll`) | §6 |
| M8 | label = `slot-popup-<8hex>-<n>` · 창 인자 해석기 하나 · 번호 = `INVALID_ARGUMENT` · `if_empty` 는 해석기 앞 | §4-5 |
| M10 | 크기 값 표 — 줄 34/28 · 단추 46 · 끌 빈 자리 42 를 표식과 따로 | §7-6 |
| M11 | 최소 폭 정본 = 웹 · 창마다 보고 · 셸이 입힘(보통 상태만) · 넓혀 당기기는 열 때만 · 판정 = 순수 `ChromeBook` | §7-7 |
| M12 | 앞 창 = platform `is_foreground`(Windows 에서 Tauri `is_focused` 금지) | §6-2 · §6-5 |
| M13 | 0탭 자가닫힘 = `window.close{if_empty}` · 「0 탭」 = 모델에 항목 없음 · 한 락 판정 | §5-5 |
| M14 | 만들기 답 = `{window, window_id, number}`(invoke · 버스 · 레지스트리 같은 셋) | §3-2 · §5-3 |
| M15 | D17 = 함수 attribute + rg 게이트(와일드카드 · 묶음 이름) | §10-4 |
| M16 | 이름 장부 부팅 시드(새 파일의 자동 탭만) | §4-3 |
| M17 | 크롬 모양 = platform `caption_style()` · 창 사실에 실음 | §7-11 |
| M18 | 탭 폭 상수 = 그대로(모양이 안 바뀐다) | §7-8 |
| M19 | 스냅 오버레이 = 주석이 아니라 기본 꺼진 cargo feature · CI 가 켠 빌드를 컴파일 | §8-4 |
| M20 | 시험 도우미 = `#[cfg(test)]` · 그 시험은 `lib_unit` · 셸 시험 feature 없음 · 0탭 · AC32 · AC20 ② = 단위 하네스 | §2-3 |
| M21 | 창 사실 = 기존 `window:frame` 에 칸 더하기(새 채널 없음) | §6-5 |
| M22 | LLM 의 보이기 · 앞으로 내기는 포커스를 빼앗지 않는다 — `raise_and_show` 차례(ⓐ 숨긴 채 z 올림 → ⓑ Tauri `show` · `unminimize` → ⓒ 전경이 우리일 때만 `set_focus`) · 날 Win32 보이기 없음 · `restore_no_activate` 없음(0.3판) · 트레이 누름 길은 그대로 | §6-2 |
| M23 | 보이지 않는 창의 상태 명령 = §5-2 표(미룬 최대화 · 거절) · 모든 보이기 갈래가 미룬 최대화를 입힘 | §5-2 |
| M29 | 창 상태 한 동작 = 메인 스레드 일 하나(읽기 · 판정 · 미룸 · OS 호출) — 첫 화면 보이기와 끼어들 수 없게 · invoke `window_control` 은 async | §5-2 |
| M30 | LLM 보이기(`NoSteal`)는 첫 화면 대기 창을 건너뜀 · 트레이 누름(`TrayClick`)은 지금처럼 기다림을 거두고 보임 · 불변식 5 를 그에 맞춤 | §2-4 · §5-2 |
| M31 | invoke `show_main_ui`(LLM · cdp 표면) = `NoSteal` · `TrayClick` 은 트레이 누름 처리기와 단일 인스턴스 두 번째 실행 콜백뿐 | §6-2 |
| M32 | 보통 상태 위 여백 = 접힌 끌기 대응 겸함 · 값은 측정(S13)으로 · 원인을 가르기 전 시한 · 문턱을 고치지 않는다 | §7-6 · W13 |
| M33 | 끌 자리 · 위 가장자리 자식 창 위 놓기 잃음 = 측정(S14) 뒤 정한다(갈래 = 그 자리를 놓기 구역에서 빼고 받아들임 · 창 수준 놓기 받는 쪽을 platform 에) | §7-9 · W14 |
| M24 | OS 창이 이미 없는 창 빼기 = `drop_window_gone`(곧바로 풂) | §4-1 |
| M25 | AC10 = TypeScript 타입 검사 시험(rg 대신) | §10-4 |
| M26 | 탭 놓기 「탭바 끝」 구역 = 제목 줄 전체 | §7-9 |
| M27 | Q5 = ① Tauri `data-tauri-drag-region`(스파이크 S1 통과 · ② 는 재지 않고 걷음 · 사용자 위임) | §1-3 · §7-2 |
| M28 | Q10 = ① 네이티브를 그대로 받아들임 · 짓는 것 없음 · 결과는 QA 가 적는다(사용자 위임 · 2026-10-24 지시) | §7-10 |

### 12-4 ADR 로 박을 것(U0 결론 뒤 · `/adr`)

① 창 번호(살아 있는 창이 안 쓰는 가장 작은 수 · 모델이 쥠 · `Destroyed` 에서 풂 · OS 창 없음 = 곧바로 · 영속 · 고치기 규칙) ② 탭 자동 이름은 셸이 짓는다(낱말 셸 표 · 거부 = 웹 보고 · 이름 바뀜 = 판 오름) ③ 창 상태 버스 명령 · `window.close(main)` = 숨기기 · 포커스를 빼앗지 않는 LLM 보이기 · 보이지 않는 창 규칙 · 창 인자 = 영속 id/label 별칭 · 번호 거부 · label 실행 표식 ④ 크롬식 제목 표시줄(OS 틀 걷기 · 끌기 영역 기법 · Alt+Space · 놓기 구역 · 창 사실 칸) ⑤ 스냅 오버레이 = 꺼진 feature(사용자 결정 Q1) ⑥ 첫 화면 전 실패를 처리하지 않음(사용자 결정 T1 — 거부한 대안 = 0.1판 §5-2 · §5-3 의 시한 실패 · 되돌림, `ProcessFailed` 잡기). 거부한 대안은 본문의 「거부한 갈래」를 옮긴다.

---

## 13. PRD 와 맞춘 곳(0.5판 — 이 판과 함께 고침)

| # | PRD 자리 | 고침 | 근거 |
|---|---|---|---|
| 1 | R1 「모델에 든 뒤 첫 화면 전 실패」 · R6 끝 문장 · AC33 · AC2 의 「보이는 순간 번호가 있다」 · D13 의 「한 번도 보이지 않은 창」 예외 | 걷음 · 「사용자 결정 2026-10-24 — 따로 처리하지 않음(웹 화면 프로세스가 죽는 드문 경우)」 · D13 예외는 「OS 창이 이미 없는 창」으로 바꿈 | T1 · §4-1 · §5-3 |
| 2 | R15 · AC19 · §2 · §5-1 Q1 | 켜지 않음 · 코드는 꺼진 feature 뒤 · AC19 없음 | Q1 · §8-4 |
| 3 | R4 빈 슬롯 낱말 예 | 결정 낱말 표 | T2 · §4-2 |
| 4 | R10 main 닫기 단추 모양 | `EyeOff` · 옅은 올림 · 툴팁 | T3 · §7-4 |
| 5 | R20 · AC34 · D14 「그 모니터로 옮기면」 | 왼쪽 맞춤 · 넓혀 당기기 = 열 때만(새 창 · 재시작 · 사본 수락) · 옮김 중은 OS 최소 크기만 — 기본값 | 리뷰 codex ⑩ · §7-7 |
| 6 | W10 · AC9 · R17 | LLM 의 보이기 · 앞으로 내기는 포커스를 빼앗지 않는다 — 막히면 맨 위로만 · `focused` 그대로 — 기본값 | 리뷰 Claude ⑤ · §6-2 |
| 7 | AC24 의 0탭 갈래 · AC32 · AC20 ② 의 「하네스」 | 셸 단위 하네스(「모델에서 빠지고 OS 창은 남은 창」 = 평범한 닫기 + 아무것도 안 하는 가짜 창 포트) · 「0 탭」 = 모델에 항목 없음 | §2-3 · §5-5 |
| 8 | AC31 ② 「겹쳐 만든 창이 지킨다」 | 사본 수락 커밋이 main 밖 창을 모두 걷는 지금 동작을 적음(그 갈래는 순수 시험으로 잰다) | §4-1 |
| 9 | §5-1 · §5-2 Q5 · Q10 · §7 | 스파이크 결과 · 메인 결정(Q5 = ① · Q10 = 네이티브 그대로 · QA 가 적음) 기록 | §1-3 · §7-2 · §7-10 |

PRD R4 의 포커스 문장은 고칠 것이 없다(§4-2 — 0.1판 §13 ④ 의 사실 오류는 TRD 쪽이었다).

---

## 14. 위험 · 정지 조건

- **정지 조건** — ① 테두리 없는 창의 최대화가 작업 표시줄을 덮고 Tauri · tao 설정으로 못 고친다(S8 — AC14 를 지킬 길이 platform Win32 코드뿐이면 사용자에게 범위를 묻는다) ② 넓힌 놓기 구역의 표식 · 단추 · 위 여백 자리에 `dragover` 가 오지 않는다(스파이크는 빈 곳만 쟀다 — §7-9 · AC12). (0.2판 앞의 「S1 이 두 갈래 다 실패」는 S1 통과로 풀렸다.)
- **W1**(PRD — tauri#10767) — 창 끌기 뒤 `mouseup` 이 빠진다(S2 — 확인). 그 직후 탭 누름 · `dragstart` 가 어긋나면 AC13 이 빨갛다 — QA.
- **W2**(PRD) — 빈 곳 위 놓기는 닿았다(S1). 나머지 자리는 위 정지 조건 ②.
- **W13 접힌 끌기**(0.3판 — PRD W13) — §1-3 S13. 대응 = 보통 상태 위 여백(`TOP_GAP` — 출처 창 안에서 누름 → dragstart 길을 지킨다 · Chrome 의 보통 상태 창도 탭 위에 끌 띠가 있다 · 최대화에는 없다 — 화면 맨 위라 위로 벗어날 곳이 없다, 위에 다른 모니터가 붙은 배치는 예외 · 미검). 메인 제안 · 사용자 확인 필요(QA). 원인(Win32/Blink)은 모른다.
- **W14 웹뷰 밖 자식 창 위 놓기 잃음**(0.3판 — PRD W14) — §7-9 · S14 · M33.
- **그리기 전에 보인 창**(T1 로 받아들임) — 1.5 s 시한에 먼저 보인 새 창은 번호 표식 · 제목 줄 없이 한순간 바탕만 보일 수 있다 · 웹뷰가 죽은 창은 빈 창으로 남는다(사람이 닫는다).
- **흐림 계기 빠짐**(S12) — `Focused` 사건이 OS 활성 바뀜을 다 덮지 않으면 흐림이 늦다 · 그때의 길 = §6-5.
- **포커스를 빼앗지 않는 앞으로 내기**(M22) — 다른 앱이 앞이면 LLM 의 `window.raise` · `showAll` 은 창을 맨 위로만 올리고 전경은 그 앱에 남는다(의도 · 답이 그 사실을 말한다).
- **옛 빌드로 내림** — 번호 · 정한 이름이 사라진다(§3-3 — 저장 TRD §12 R3).
- **데몬 crate 변경** — 개명 뒤 `AgentListUpdated` 가 늘어난다(개명은 드물다). 웹의 `agent-list-updated` 소비는 이미 그 사건을 다룬다.
- **크기 값이 기준 빌드와 어긋남** — S11 로 확정하기 전 값으로 U8a 를 지으면 U8b 에서 고친다(상수 한 파일).
- **스냅 feature 썩음** — CI 컴파일 단계(§8-4)가 막는다. 켠 빌드의 동작(배율 · 창마다 · 포커스)은 켤 때 다시 잰다(W8).

---

## 15. 리뷰 기록

### 15-1 0.1판 리뷰(2026-10-23 · 2 건 — codex Designer FIX · Claude Architect-breaker FIX)

**사용자 결정 T1 로 대상이 없어진 지적** — 첫 화면 기다림 · 실패 되돌림 몫에 묶였던 것: codex ③ · ④ · ⑥ 의 실패 처리 몫 · Claude ① · ② · ⑧. 그 설계(0.1판 §5-2 · §5-3 · `WindowHost::await_painted` · `abort_new_window` · `TabsFate` · 시한 보이기 걷기 · U7)를 통째로 걷었다. 원문 지적은 그 리뷰 기록에 있다(이 표에 옮겨 적지 않는다 — 대상이 없다).

| 지적 | 고친 곳 |
|---|---|
| codex ① 제목 줄 빈 곳 · 표식 · 단추 위 놓기가 새 창이 된다 | §7-9 — 놓기 구역을 제목 줄 전체로(`geometryOf` · `windowDrop`) · §10-3 · U8a |
| codex ② 공통 해석기가 `close_if_empty` 를 `CONFLICT` 로 막는다 | §4-5 예외 · §5-5 — 해석기 앞에서 가르고 한 락에서 판정 |
| codex ⑤ · Claude ③ OS 제목 · 웹 탭 알림의 순서 · 판 — 이름 장부 갱신이 판을 안 올려 웹이 버린다 | §4-3(판 오름) · §4-4 · §5-1(메인 스레드 + 판 비교) · 불변식 2 |
| codex ⑥ 보이지 않는 창의 상태 명령이 첫 화면 보이기 소유를 깬다 | §5-2 표 · 모든 보이기 갈래의 미룬 최대화 · 불변식 5 · U7a |
| codex ⑦ U1 이 홀로 빌드되지 않는다 | §4-2 · U1 = `slot_label` · `window_title` 만 · `effective_tab_name` · 장부 → U3a |
| codex ⑧ 새 사건 채널 대신 기존 `WindowFrame` 길 | §6-5 · §3-2 · `window:chrome` · `window_chrome` · `window_chrome_state` 걷음 |
| codex ⑨ 자동 이름과 같은 글자 이름 바꾸기가 굳지 않는다 | §3-1 · U3a(`commitEdit` · 모델) · §10-3 |
| codex ⑩ 좁은 모니터로 옮길 때의 왼쪽 맞춤 범위 | §7-7(열 때만) · PRD 0.5판 R20 · AC34 기본값으로 적음 |
| Claude ④ Windows 에서 Tauri `is_focused` 를 쓰면 안 된다 | §6-2 · platform `is_foreground` · M12 |
| Claude ⑤ `raise` · `showAll` 이 포커스를 빼앗는다 | §6-2 · 트레이 보이기 갈래 · M22 · PRD 0.5판 W10 · AC9(0.3판에서 수단을 `raise_and_show` 차례로 고침 — §15-5) |
| Claude ⑥ 이미 사라진 OS 창의 `close_window` 로 번호가 샌다 | §4-1 `drop_window_gone` · `placement` · `restore` 두 자리 · M24 |
| Claude ⑦ AC10 rg 게이트가 줄바꿈 · 별칭에 뚫린다 | §10-4 TypeScript 타입 검사 시험 · M25 |
| Claude ⑨ 셸에 `test-support` feature 가 없다 — 하네스 훅의 자리 | §2-3 · M20(`#[cfg(test)]` · `lib_unit` · 셸 시험 feature 안 만듦) |
| Claude ⑩ `chrome.rs` 에 seam 이 없다 · 「열 때만」을 어떻게 아나 | §2-2 · §7-7 · `chrome::ChromeBook`(순수) + 어댑터 · `mark_opening` 자리 |
| Claude ⑪ `set_slot_content` 는 포커스를 옮기지 않는다 — 사실 오류 | §4-2 고침 · §13 ④ 걷음(PRD 는 고칠 문장 없음) |
| 잔손 — D17 게이트가 묶음 이름 갈래를 못 잡는다 | §10-4 정규식 넓힘 · 파일 규칙 |
| 잔손 — invoke 이름 불일치(`window_chrome` · `window_chrome_state`) · 창별 정리 | 둘 다 걷음(§6-5) · `ChromeBook::forget` · `FrameBook::forget`(기존) |
| 잔손 — §3-5 와 M16 시드의 모순 | §3-5 고침(이름 바뀜도 판을 올려 파일 이름이 늘 지금 값) |
| 잔손 — 닫힌 함수 서명의 수명 | §4-2 `for<'a>` · `'n` |
| 잔손 — U6/U7 차례 | §11 — U6 = 만들기 답(작음) · U7a/U7b = 창 상태 |
| 잔손 — §1-2 「끌기 경로를 건드리지 않는다」 | §1-2 고침(놓기 구역 넓이 하나를 바꾼다) |

### 15-2 이 판에서 사용자 결정으로 바뀐 것

T1(첫 화면 전 실패 처리 걷음) · Q1(스냅 = 꺼진 feature) · T2(낱말) · T3(main 셋째 단추) — §12-2.

### 15-3 단위 번호 대응(0.1판 → 0.2판)

0.1 U0–U5 = 0.2 U0–U5(U1 · U3a · U4 의 몫이 옮겨졌다 — §11) · 0.1 U6(창 상태) = 0.2 U7a · U7b · 0.1 U7(첫 화면 기다림 · 되돌림) = 걷음 — 남은 「만들기 답」 몫이 0.2 U6 · 0.1 U8a · U8b = 그대로 · 0.1 U9(스냅 — 넣을 때만) = 0.2 U9(꺼진 feature 뒤로 옮기기) · U10 = 그대로.

### 15-4 스파이크 결과 접기(사용자 손 실측 2026-10-24)

Q5 = ① Tauri 끌기 영역(팝아웃의 탭을 main 제목 줄 빈 곳에 놓으면 main 탭바에 들어감 · 끌기 영역 `dragover` 13 번 · 갈래 ② 는 재지 않고 걷음) · 스냅 오버레이 메뉴는 떴으나 켜지 않음(Q1) · Alt+Space · 오른쪽 클릭 창 메뉴 = 재지 않음 → ① 의 네이티브 동작을 받아들이고 QA 가 적음(Q10) · tauri#10767 = 창 끌기 뒤 `mouseup` 누름 8 · 뗌 4(OS 이동 루프 몫 — 예상) · 그 직후 누름 = QA(AC13). 바뀐 곳 = 머리 ⑤ · §0 · §1-3 · §7-2 · §7-9 · §7-10 · §9 · §11(U0 = 스파이크 코드 걷기 · U8a 가 권한을 더함) · §12-1 · §12-3(M27 · M28) · §14.

### 15-5 0.2판 재리뷰(2026-10-24 · Claude Architect-breaker · light = FIX) · 디버그 세션 실측 → 0.3판

| 지적 · 실측 | 고친 곳 |
|---|---|
| ① 포커스 안 빼앗는 보이기를 날 Win32(`place_window(show: true)`)로 하면 tao 보임 깃발이 거짓으로 남는다(`first_paint::raise_and_show` ⓑ 주석 · `DeferredMaximize` 문서) · `restore_no_activate` 필요를 다시 끌어내라 | §6-2 — `raise_and_show` 차례(ⓐ 숨긴 채 z → ⓑ Tauri `show` · `unminimize` → ⓒ 전경이 우리일 때만 `set_focus`) · 메인 스레드 · `restore_no_activate` 걷음(Tauri `unminimize` 로 — 깃발이 맞는다) · §8-2 · M22 |
| ② `is_held` 확인(버스 스레드)과 미룸 적기 사이에 첫 화면 보이기(메인 스레드)가 끼면 미룸이 홀로 남아 `placement::record` 가 자리 적기 · `window:frame` 밀기를 멈춘다 | §5-2 — 읽기 · 판정 · 미룸 · OS 호출 = 메인 스레드 일 하나 · invoke `window_control` async · 메인 스레드 줄 흉내 시험(§10-1) · M29 |
| ③ 트레이 보이기는 첫 화면 대기 창도 `cancel_hold` → `show()` 한다 — 불변식 5 와 어긋남 | 고른 것 = LLM 보이기(`NoSteal`)는 대기 창을 건너뛰고 트레이 누름은 그대로 · 불변식 5 를 그 문장으로 · M30 |
| ④ `caption_style()` 은 `WindowFrame.caption` 을 채우는 U7a 몫 | §11 U7a · U8b · §8-2 |
| 잔손 — `Focused` 사건과 `is_foreground` 의 차례 | §6-5 — 그 창 자신의 칸 = 사건 값 · 나머지 = `is_foreground` · S12 가 둘을 함께 찍음 |
| 잔손 — `forget` 뒤 늦은 제목 일이 항목을 되살린다 | §4-4 — 일 안에서 창 있음부터 보고 없으면 장부를 안 만짐 · §10-1 |
| 잔손 — `if_empty` 에 숫자 인자 | §4-5 — 번호 거부가 먼저(Q9 · AC26) · §10-1 |
| 잔손 — invoke `show_main_ui` 의 갈래 | §6-2 — `NoSteal`(LLM · cdp 표면) · M31 |
| 잔손 — PRD R1 예열 창 줄 「번호 없이 보이는 창이 없다」가 T1 · AC2 와 어긋남 | PRD 0.5판 R1 고침 |
| 잔손 — `Destroyed` 정리가 넷 다 있을 때만 돌아 번호가 샐 수 있다 | §4-1 — `LayoutState` 하나로 조건문 앞에서 번호 풀기 → U2 리뷰가 고침: 넷이 다 있으면 `drop_window_in_model(Destroyed)` 한 락 안 · 하나라도 없을 때만 `else` 갈래에서 풀기(§4-1) |
| 실측 (a) 접힌 끌기 — 맨 위 탭 줄에서 빠른 위쪽 끌기가 OS 끌기 전에 창을 벗어나 취소된다(스파이크 세션 18 · 앞 기록 0) | §1-3 S13(측정 문) · §7-6 `TOP_GAP` · §14 W13 · U8b · M32 · PRD R7 · W13(사용자 확인 필요(QA)) |
| 실측 (b) 끌 자리 ②(CSS `app-region`) 위 놓기 잃음(최상위 창의 tao 파일 전용 `RegisterDragDrop`) — 위 가장자리 크기 조절 자식 창 · 스냅 오버레이에도 같은 길이 있을 수 있다 | §7-2(② 걷은 근거 확정) · §7-9 · §1-3 S14(측정 문) · §14 W14 · U8b · M33 · PRD W14 |
| 실측 (c) `dragSource.ts` 의 같은 task 판정이 접힌 끌기(다음 task 의 dragend)를 못 거른다 — 셸이 취소로 읽어 지금은 해 없음 | §7-9 기록 |

### 15-6 리뷰 4(Claude · light 범위) PASS · 본문 고침 3(2026-10-24)

§5-2 — 새 창 상태 동사와 `window.list` 는 `offloaded_handler` 로 꽂는다(`blocking_handler` 본문은 async 워커에서 돈다) · §6-2 · M31 — `TrayClick` 의 둘째 호출자 = 단일 인스턴스 두 번째 실행 콜백(「부팅 보이기」 아님) · §6-2 — `show_all` · `hide_all` 도 메인 스레드 일 하나.
