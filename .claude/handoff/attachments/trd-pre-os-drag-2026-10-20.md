# TRD — 슬롯 도킹 (탭 끌기 = 데스크톱 Chrome 그대로 · 도킹 = 우리 확장)

> **6.3판 — 리뷰 3 라운드 반영(상한 도달 — 재리뷰 대신 해당 단위 코드 리뷰에서 확인)**(2026-10-18 · `/review trd` 3 라운드 판정 BLOCK 1 · FIX 6 — 라운드 상한이라 전체 재리뷰를 돌리지 않는다 · 고친 곳은 §15-5 의 단위 코드 리뷰가 다시 본다). 6.2판 위에서 고친 것만 줄마다 리뷰 항목 하나로 적는다:
> - **F1** — 정리 할 일(`CloseWindow` · `SpareGiveBack` · 공통 마무리 3 의 다시 보이기)은 소유를 넘기는 할 일(ⓞ)이다: 실패하면 실행기가 `Failed` 를 줄에 넣기 전에 같은 메인 스레드 걸음에서 그 창을 대기 창 명부의 고아로 넘기고, 명부가 세션과 따로 다시 한다(닫기 → `destroy` · 되돌리기 → 다시 · 안 되면 닫고 새 예비 · 모델에 있으면 제자리 다시 보이기) · `CloseWindow` 는 늘 넘긴다(닫기의 성공 = `Destroyed`) · I × `Failed(Soft)` = x7(§2-3 · §2-3-3 · §2-4 · §2-5 · §6-1)
> - **F2** — platform 이 모든 끝 알림에 `hid`(이 고리의 Esc 훅이 그 창을 숨겼나)를 싣고 T37 ② · T38 · T39 가 끝 까닭과 무관하게 `hid` 로 `hidden` 에 적는다 — 끝냄 청과 겹친 들어가기 전 Esc 의 `Ended` · Esc 뗌을 본 뒤의 `Released{false}` 도(§2-3 · §6-2 · §8)
> - **F3** — 두 걸음 놓기의 구멍 넷: 다시 놓기 계기 = `Posted` 동안 막 뗀 창의 첫 `ScaleFactorChanged`(목표 배율과 다를 때 · `Moved` 는 보지 않음) · Detached `original` 은 목표 배율에 자리 잡은 `geom`(`settled`)에서만 · `geom` · `probe` 는 줄에서 꺼낼 때 읽음 · `offscreen_near` 는 배율로 커질 크기 + 여백으로 겹침을 봄(§2-3 · §5-4 · §5-5)
> - **F4** — W 열 × `Failed(Soft)` 여덟 칸 = ⊥(결과 블록 불변식 — 무른 실패는 `cont` 가 빈 뒤에만 처리된다 · T44 가 W 열에서 [끝]을 내면 `cont` 가 선 채 끝난다) · 칸 수를 다시 셈 · T44 칸 = S · M 열 · 잔손(§2-2 「T50」 → T43 · §6-1 들어오는 칸에 T44(`ArmRelease`) · `hidden` = 집합)(§2-3-3 · §2-3-4)
> - **F5** — `Abandon{by: Token | Session}` — leave 시한은 begin 표를 모르는 붙은 창 웹뷰에서도 오므로 leave 가 실은 세션 번호로(§2-3 · §2-3-1 · §2-4 · §4)
> - **F6** — 키 감시가 켜진 채 남지 않게: 세션 번호 1 부터 단조 · 웹뷰는 `lastEnded` 보다 큰 `drag` 만 켬(듣기를 먼저 건 뒤 `window_role`) · `Ended` 를 `armed ∪ 모델 창` 에 · 감시는 키 끝 하나를 보내면 스스로 끔 · `tab_drag_*` 에 마운트 표 `mount` — 바뀌면 그 창 `seen` 을 0 으로(§2-3-1 · §4 · §6-3)
> - **F7** — `Focus` · 붙기 활성은 Tauri `set_focus` 를 쓰지 않음(tao 가 Alt 를 합성해 키 감시가 놓기로 읽는다) · 공통 마무리 3 = 제자리 보이기(`rect: None`) · 공통 마무리 뒤 tao `is_visible` 과 OS 보임이 같은지 debug 단언(§2-3 · §6-1 · §6-3 · §8)
> - **F8** — U5a-1 의 `unimplemented!` 은 「U5a-2 Txx」 꼬리표 · U5b 시작 조건 `rg -n "unimplemented!|todo!" src-tauri/src/dock` 0(§13)
>
> **6.2판 — 리뷰 2 라운드 반영**(2026-10-18 · `/review trd` 2 라운드 판정 FIX · 마지막 재검 필수). 6.1판 위에서 고친 것만 줄마다 리뷰 항목 하나로 적는다:
> - **수렴 조건** — 세션 계약을 행렬 하나로 닫음: 상태 꼴 18 × 사건 꼴 29 = 522 칸이 전부 이름 붙은 전이(T01–T44) 또는 사유 붙은 무시(⊥ · ⊘ · x1–x6)다 · `cell()` 이 그 칸 이름을 돌려주고 `step` · `accept` · `cell` 의 `match` 에 `_ =>` 갈래가 없다 · U5a 시험은 행렬 전부를 표로 돈다(§2-3-3 · §11)
> - **D1** — 할 일 실패를 사건으로 되돌림: 할 일마다 실패 등급(관문 `Take` · `Start` / 무름 / 실패 없음) · 관문이 실패하면 그 묶음의 나머지(쓰기 포함)를 돌리지 않고 `Failed` 가 줄 맨 앞에 온다 · 고리 시작 실패 = 되돌리기(T43 — §8 `Unsupported` 와 §10 이 같은 뜻)(§2-3 · §2-4)
> - **D2** — 보냈으나 아직 안 들어간 고리에서 Esc 를 보면 platform 이 `SC_MOVE` 를 삼키고 `Canceled{esc: true}` 를 한 번 낸다(§8 · T39)
> - **D3** — `Abandon` 은 끝이다: 정한 붙기 · 키 놓기도 되돌리기로 덮는다(T34)
> - **D4** — 루프 들어가기 전 놓기에도 `original` 이 있다: platform · 창 사건이 끄는 창 OS 기하를 싣고 첫 사건에서 적는다(§5-4 · T36 · T37)
> - **D5** — 받기 거르개가 출처 · 단계를 `seq` 보다 먼저 본다(버린 사건이 `seen` 을 올리지 않는다) · holder 아닌 창의 키 놓기 = 놓기 확인(T23)(§2-3-1)
> - **E1** — 되돌리기 쓰기의 결과를 붙기 · 띠처럼 처리(비운 예비 = `SpareGiveBack` · `spare = None` · 다른 빈 창 = 남긴 창 · `main_refilled` 적기) · 쓰기 아닌 되돌리기 단계(`Unpark` · `Restore`)를 `RevertStep` 으로 · 되돌리기 쓰기도 `OnEmptied::Keep`(T16 · §6-2)
> - **E2** — Alt+Tab 빼기 = `WS_EX_TOOLWINDOW` 켜고 `WS_EX_APPWINDOW` 끄기(넣기는 둘 다 되돌림) · 「대기 창에 tao 깃발 setter 를 부르지 않는다」 불변식(tao 가 깃발이 바뀔 때마다 `GWL_EXSTYLE` 을 다시 쓴다 — 코드 확인) · 안내 창 짓는 순서 · 첫 떼기 여백 = `Unpark` 뒤 OS 안쪽 크기(§2-5 · §8 · M18 가상 데스크톱)
> - **E3** — 배율이 다른 모니터로 떼기 = 두 걸음 놓기(커서 모니터 가까운 화면 밖으로 옮겨 배율을 먼저 맞춘 뒤 틀을 읽고 놓음) + 배율 바뀜 다시 놓기(§5-5 · U5b · M10)
> - **E4** — 창째 끌기의 `original` 을 늦은 캔버스 보고가 아니라 OS 안쪽 크기 − `canvas_origin` 으로(§5-4)
> - **E5** — §14 를 사용자 위임으로 고쳐 적음 · U0 은 메인이 검증한 백업 패치를 만든 뒤 버림(§13 · §14)
> - **E6** — `dock:drag{Active}` 를 예비 들임 · 남긴 창 되살림 · 새 모델 창에도 보냄 · `dock:feed{Attach}` 가 카운터를 세움 · 다시 읽힌 창은 `seen` 을 지우고 `window_role` 로 지금 세션을 다시 받음(T07 · T10 · T26 · T29 · §4)
> - **E7** — Strip{Shell} 놓기의 최종 자리: 셸이 뗌을 보면 `dock:feed{Release}` → holder 가 `tab_drag_end{Drop, Release, at}` · G18 시한이 넘으면 모델 자리(T23 · T31 · §4)
> - **E8** — 타입 자리: `PhysPoint` · `MonoMs` → U2 의 `dock/units.rs` · `OverlayView` → U2 · 세계의 명부 재기 → U4 · `DockTo` · `Docked` · `DockOutcome` 의 `Eq` 를 U1 에서 뺌(§13)
> - **E9** — Esc 훅은 DWM 전환 끄기를 늘 하고 숨기기만 `hide_on_escape` 일 때(Chrome 그대로 — §8)
> - **E10** — 들어갈 때 버튼이 떨어졌으면 `WM_CANCELMODE` 를 보내지 않고 올린다(post) · 고리 안 창 소멸에 재귀 깊이 가드 · U3 「고리 안 소멸」 시험(§8)
> - **E11** — C8 되살림 자리는 끄는 탭을 뺀 목록 기준(§6-2 · §7 ④)
> - **E12** — U9 가 CLAUDE.md 「창 구성」도 고침 · `restore_placement` 는 작업 공간 좌표를 맞추고 `Unpark` 뒤에(§8 · §6-2 · §13)
> - **덧** — 닫기 요청 갈래의 명부 빼기는 걷음(어차피 사라지는 창 — `Destroyed` 갈래가 다시 짓는다)(§2-5) · 핸드오프의 「미리 숨겨」 = 화면 밖에 보이는 채(G2)라고 §1 에 적음 · 남긴 창 · 되돌린 예비가 0탭 자가닫힘에 걸리지 않게(§2-5 · §7 ⑤)
>
> **6.1판 — 리뷰 1 라운드 반영**(2026-10-18 · `/review trd` 1 라운드 판정 FIX · 재검 필수). 6판 위에서 고친 것만 줄마다 리뷰 항목 하나로 적는다:
> - **A1 · B1** — 끝 · 실패 · 되돌리기를 공통 마무리 하나로 모으고 보상 순서를 못 박음(살아 있는 루프 끝냄 · 꺼낸 예비 창 되돌림 · 숨긴 창 다시 보임 · 답 못 한 invoke · `dock:drag{Ended}`) · 붙기 쓰기가 실패하면 숨긴 끄는 창을 다시 보이고 탭은 그 창에 남김 · pending 은 처음 정한 것이 이김(§2-3 · §6-1)
> - **A2** — 레이아웃 쓰기(`Write`)는 묶음의 마지막 할 일 · 실행기가 메인 스레드에서 동기로 돌리고 결과 `Wrote` 를 줄 맨 앞에 넣어 다음 사건으로 처리 · 세션은 `cont` 로 이어 감(§2-3 · §2-4)
> - **A3 · B5** — 순서 장치 하나: 보낸 창마다 단조 `seq`(낡은 것 버림) · `end` 가 최종 자리 `at` 을 실음 · begin · leave 답을 기다리는 동안의 끝 입력은 웹뷰가 붙들었다 답 뒤에 보냄 · 끝 사건 출처 규칙(포인터 끝 = holder 만 · 키 = 아무 우리 창) · Strip{Shell} 키 끝 줄(§2-3 · §4 「순서」)
> - **B2** — 모델 밖 창(예비 창 · 남긴 창)의 재기(캔버스 · 지표 · 크롬)를 셸 「대기 창 명부」가 받아 둠 · `AdoptWindow` 가 같은 락에서 새 창 항목에 옮겨 담음(§2-5 · §7)
> - **B3** — 루프의 「보냈으나 아직 안 들어감」 상태 · 그때의 끝 = 들어올 때 삼킴 · 들어올 때 주 버튼 다시 봄(§8)
> - **B4** — 끄는 창이 루프 중 사라져도 끝 알림은 정확히 한 번(`Destroyed`) · §6-5 표의 「예비로」 정정(§8 · §6-5)
> - **B6** — begin 의 웹뷰 표 `token` · 시한 넘긴 처리기가 `Abandon` · Strip{Pointer} 주 버튼 감시 · holder 웹뷰 다시 읽힘 = 끝(§2-3 · §2-4)
> - **B7** — 예비 · 남긴 창을 셸 명부로 트레이 · 작업 표시줄 · Alt+Tab 에서 뺌(접두 대신 명부 — 까닭 §2-5) · 닫기 요청에 묘비 안 닮 · 사라지면 다시 지음 · 대기 모드는 URL 이 아니라 `window_role` · `dock:spare`(§2-5 · G2 · G10 · G12)
> - **B8** — §13 단위를 호출부 `git grep` 으로 다시 셈(파일 · 겹침) · `DockChrome` 모양 바꿈 = 새 모양을 처음 보내는 U8 · 핸들 의존 = platform(U3) · 새 세션 사건 이름 `dock:drag`(옛 `dock:session` 과 겹치지 않게)(§13 · §4)
> - **B9** — `dock_tab` 계약을 갈래별로 고쳐 씀 · 세션 포트는 `NewWindow` 를 타입에서 뺌(§7 · §3)
> - **B10** — 창 핸들 변이 맞추기는 platform 안(`HasWindowHandle` 를 받음 · 셸 의존 없음)(§8 · G16)
> - **B11** — 스택 맞춤 셋(예비 창 · 펌프 시작 · 붙은 뒤 셸 폴링)을 R28 에서 나와 2026-10-18 사용자 보고를 마친 것으로 §1 에 적음 · ADR-b 가 ADR-0295 결정 1 을 고친다고 명시(§1 · §16)
> - **B12** — `DockOverlayView({guide})` 는 이미 props 보기(바꿀 것 = 타입) · `dock_tab` 운영 호출자 = `commands/dock.rs` `TauriDock::apply`(§3 · §7 · §15-3)
> - **근거 없음 a–c** — 버튼을 뗀 뒤 도착한 `SC_MOVE` · 창째 끌 때 WebView2 자식 포커스와 Esc 훅 · 배율 다른 모니터로 옮긴 안내 창 → 설계 메모(§8 · §6-3 · §2-5) + M17 · M5 · M10
> - **C1–C10** — §15-1 을 Chromium 소스(2026-10-18 사본)로 닫음. 6판 기본값이 틀렸던 곳 = C5(크기 규칙 순서) · C8(붙기 전 선택 되살림) · G9(맨 뒤 → 첫 자리에 넣고 같은 함수로 고침) · C3(탭바 구역 가로 = 끌기 영역 — R23 문장 세부 정정) · C4(창째 끌기 자리) · C9(단계별 키 규칙 — 루프 중 Esc 밖 키는 놓기가 아니다 · R12 세분)
> - **§14** — 메인 결정(2026-10-18) 적음 · G9 · G13 은 메인이 6판 추천과 달리 정함 · G2 · G7 · G10 · G12 · G16 은 리뷰 · Chromium 확인에 맞춰 고친 채로 채택 · 나머지는 추천 그대로
>
> **6판 — 다시 씀**(2026-10-18 · ADR-0295 · PRD 4.8). 5.8판(HTML5 끌어 놓기)은 사용자 마우스 실측에서 불합격했고 그 판은 백업 `.claude/handoff/attachments/trd-5.8-before-chrome-2026-10-18.md` 에 있다. 이 판은 5.8 의 살아남는 부분(레이아웃 연산 · 저장 · 탭바 폭 규칙)만 옮겨 오고 나머지는 새로 쓴다.

| 항목 | 값 |
|---|---|
| **상태** | 6.3판 — 리뷰 3 라운드 반영 · 세션 계약 = 전부 덮는 행렬(§2-3-3) · 구현 갈림길 결정됨(§14 · 사용자 위임 · 2026-10-18) · Chromium 확인 닫힘(§15-1) · `/review trd` 라운드 상한 도달 — 재리뷰 없음 · 리뷰 3 의 F1–F8 은 해당 단위의 코드 리뷰가 다시 본다(§15-5) |
| **날짜** | 2026-10-18 |
| **스텝** | S21 |
| **브랜치** | `v0.4.1/feat/slot-docking` |
| **PRD** | [prd.md](prd.md) 4.8판 |
| **결정** | [ADR-0295](../../decisions/0295-슬롯-도킹-탭-끌기-데스크톱-chrome-소스-그대로-도킹-띠-10-붙는-크기-원래-크기칸-절반-상한.md) |
| **근거** | 조사 [slot-docking-chrome-desktop-drag-2026-10-18.md](../../research/slot-docking-chrome-desktop-drag-2026-10-18.md)(표 A–E — 이 문서의 「조사 A3」 같은 표기는 그 표의 행) · 코드 점검 [slot-docking-drag-code-audit-2026-10-18.md](../../refactoring/slot-docking-drag-code-audit-2026-10-18.md)(「점검 결함 n」) · 스파이크 `.claude/handoff/attachments/spike-chrome-drag-2026-10-18.md`(「스파이크 Sn」 — 110 회 실측) · Chromium main 사본 `.claude/handoff/attachments/chromium-src-2026-10-18/`(2026-10-18 받음 · 「TDC#n」 = 그 사본의 `tab_drag_controller.cc` 줄 · 다른 파일은 이름#줄) · 5.8판(「5.8 §n」) |
| **줄 포인터 기준** | HEAD `9b74ff40`(9b 까지 커밋). 9c 는 커밋하지 않는다(ADR-0295 「영향」 · U0 에서 버림) — 이 문서가 가리키는 코드는 심볼 이름이 정본이고, 줄 번호를 쓸 때는 「HEAD:」를 붙인다 |

## 0. 결론 요약

- **끌기는 세 단계이고 단계마다 사건 출처가 정해져 있다.** ① 탭바 안(누름 → 10 px → 순서 이동) = 포인터 캡처를 쥔 웹뷰가 몬다 ② 떼어진 뒤(진짜 창이 Windows 창 이동 루프로 커서를 따라옴) = 셸이 몬다 — 사건은 그 창의 `Moved` · 루프 들어감 · 끝 알림 · 루프 동안의 Esc 훅 ③ 다른 창 탭바에 붙은 뒤 = 그 창 웹뷰가 같은 탭바 안 규칙을 돌리되 커서는 셸이 8 ms 마다 읽어 민다(그 웹뷰의 포인터 캡처는 ~90 ms 뒤 끊긴다 — 스파이크 S4). 키는 세 단계 내내 앱 전체 키 감시(Chrome `EventTracker`)가 받는다. 이것은 Chrome 의 꼴 그대로다 — 캡처를 쥔 쪽이 몰고, 떼면 브라우저 프로세스(우리 = 셸)가 몬다(조사 A1 · A6 · A7). ③ 의 셸 폴링은 이 스택에 맞춘 수단이다(§1 「스택 맞춤」).
- **끌기 세션의 상태 기계는 셸 메인 스레드 하나에 산다.** 모든 사건(웹뷰 invoke · 창 사건 · 루프 알림 · 폴러 · 시한)은 메인 스레드의 사건 줄로 모이고 한 번에 하나씩 끝까지 처리한다. 레이아웃 쓰기는 묶음의 마지막 할 일이고 그 결과가 곧바로 다음 사건으로 돌아온다(§2-4). 실패할 수 있는 할 일은 전부 실패를 사건(`Failed`)으로 되돌리고, 뒤 할 일이 기대는 관문이 실패하면 그 묶음의 나머지를 돌리지 않는다(§2-4). 웹뷰 사건은 보낸 창마다 단조 `seq` 를 싣고 낡은 것은 버린다(§4 「순서」). 상태 기계 본체는 순수 함수 `step(상태, 사건, 세계) → 할 일` 이고 OS · Tauri 는 포트 뒤에 있다 — 가짜 포트로 단독 시험이 선다(ADR-0012). **계약은 상태 꼴 × 사건 꼴 행렬 하나가 정본이고 빈칸이 없다**(§2-3-3 — 칸마다 전이 이름 또는 무시 사유).
- **배치(모델)는 Chrome 처럼 그때그때 바뀐다** — 순서가 바뀌는 순간 · 떼는 순간 · 붙는 순간(첫 자리에 넣고 곧바로 같은 자리 함수로 고침 — C1) · 칸 띠에 놓는 순간 · 되돌리는 순간(PRD R2 「배치를 바꾸는 때」 · Q30). 바꾸는 길은 전부 `layout::apply::dock_tab` 하나다(PRD R16 — T-65 명령이 같은 길에 얹는다).
- **떼어 낼 창은 미리 만들어 둔 예비 창 하나다**(스파이크 S8 — 그때 만들면 검정 · 흰 화면 213–314 ms). 창 이동 루프는 `ReleaseCapture` + `PostMessage(WM_SYSCOMMAND, SC_MOVE|0x0002)` 로 **메시지 펌프에서** 시작한다(스파이크 S3 — IPC 처리기 안에서 동기로 돌리면 루프 안에서 끝난 웹뷰 → 셸 IPC 왕복이 0/21). 끝은 `WM_CANCELMODE`(조사 A6). 둘 다 ADR-0295 결정 1 의 문장을 바꾸는 스택 맞춤이다(§1 · ADR-b).
- **모델 밖에서 기다리는 우리 창(예비 창 · 끄는 중 비워 남긴 창)은 셸의 「대기 창 명부」가 안다** — 재기(캔버스 · 지표 · 크롬)를 받아 두었다 들일 때 옮겨 담고, 트레이 · 작업 표시줄 · Alt+Tab · 자리 기록에서 뺀다(§2-5). 세션이 놓은 창의 정리(닫기 · 예비로 되돌리기 · 다시 보이기)가 실패하면 그 창도 명부가 고아로 받아 세션과 따로 끝까지 정리한다(§2-5 「고아」 · F1).
- **도킹 안내는 따로 된 안내 창 하나에 그린다** — 투명 · 클릭 통과 · 활성화 안 됨 · 항상 위 · 날 `SetWindowPos(HWND_TOPMOST)`(스파이크 S7 — Tauri `set_position` 은 z 순서를 올리지 않는다 · 0/1). 판정과 그리기가 둘 다 셸이라 「칠한 곳 = 붙는 곳」(PRD R6a)이 함수 하나로 선다.
- **TS ↔ Rust 거울이 없다.** 탭바 안 기하(끄는 탭 x · 순서 문턱 · 자리 = Chromium `CalculateInsertionIndex`)는 TS 만, 창 사이 기하(탭바 구역 · 도킹 띠 · 붙는 크기 · 떼어진 창 크기)는 Rust 만 갖는다. 웹뷰는 탭바 구역 사각형을 셸에게서 받아 「점이 그 안인가」만 본다. 그래서 골든 `pick_golden.json` · `dockPick.ts` 거울이 없어진다(점검 결함 5 · 7).
- **OS 코드는 `platform` 세 모듈(`move_loop` · `window` · `input`)의 목적 함수로만 들어간다**(ADR-0266): 창 이동 루프 시작 · 끝(+ 루프 동안의 스레드 마우스 · Esc 훅 · 들어감 · 끝 알림 — 정확히 한 번) · 커서 밑 맨 위 창 · 창 자리와 z 순서 한 번에 놓기 · 숨기기 · 최대화 되돌리기 · Alt+Tab 에서 빼기 · 주 버튼 상태 · 커서 읽기(§8). 창은 `HasWindowHandle` 로 넘기고 변이 맞추기도 platform 안이다.
- **살리는 것** = 레이아웃 연산(`dock_tab` · `DockGuard` · 표식 · 묘비 · `docked_tree` · `incoming_rect`) · 탭바 폭 규칙(`tabStripLayout`) · 크롬 보고 · 도킹 띠의 바탕 기하. **걷는 것** = HTML5 · OLE 끌기에만 있던 것 전부(`book.rs` · `dockDrag.ts` · `dockPick.ts` · `dragImage.ts` · 보고 wire · 늘 0 인 스크롤 배관)(부록 B).
- **구현 갈림길 §14 는 결정됐고(사용자 위임 · 메인 · 2026-10-18) Chromium 확인 §15-1 은 소스로 닫혔다.** **멈출 조건** = §15-2(실측에서 Chrome 방식이 이 스택에서 안 되면 대안을 짓지 않고 사용자에게).

## 1. 전제 · 출처 규칙

- **Chrome 규칙은 조사 문서의 표 행(조사 A1–A12 · E1–E9)과 그 행이 든 Chromium `파일#줄`, 그리고 §15-1 이 Chromium main 사본(2026-10-18)에서 읽은 `파일#줄` 로만 적는다.** 사본에도 없는 Chrome 규칙은 「Chromium 소스 확인 필요」로 적고 값을 짓지 않는다.
- **Chrome 에서 오지 않은 수는 출처를 함께 적는다** — 사용자 결정(ADR-0295) · 스파이크 실측 · 5.8 의 값 · 이 TRD 의 구현 갈림길(§14) 중 하나다.
- **이 스택에서 Chrome 방식이 안 되는 곳은 대안을 짓지 않는다** — 실측 근거를 들고 사용자에게 올린다(PRD R28 · ADR-0295 결정 2 · §15-2).
- **조사 스파이크 S1–S8 은 끝났고 막는 것이 없었다**(110 회 · 전부 PRD R28 합격 기준 통과 — S3 의 IPC 는 「Chrome 호출 모양」에서만 불합격 · 메시지 펌프 시작에서 흐름 · S8 은 미리 만든 창에서 합격).
- **★R28 에서 나온 스택 맞춤 셋 — 2026-10-18 세션에서 사용자에게 보고를 마쳤다★**(핸드오프 「메인이 정한 내부 구현 (스파이크 결과 · 사용자에게 보고함)」). 셋 다 Chrome 의 **동작**(사용자가 보는 것 — 떼는 순간 진짜 창이 커서 밑에서 따라오고, 다른 창 탭바에 닿으면 붙어 순서 이동이 이어진다)은 그대로 두고 그 동작을 이 스택에서 내는 **수단**만 바꾼다. ①은 ADR-0295 결정 1 의 문장(「그 즉시 진짜 새 창을 커서 밑에 만들어」)과, ②는 같은 결정의 루프 시작 방식과 어긋나므로 **ADR-b(§16)가 ADR-0295 결정 1 을 고친다**(`/adr link` — 고침 링크를 양쪽에 단다).

  | # | Chrome | 이 스택 | 근거(스파이크) |
  |---|---|---|---|
  | ① | 떼는 순간 새 브라우저 창을 만든다(`CreateBrowserForDrag`) | 미리 만든 예비 창 하나를 떼는 순간 모델에 들인다(§2-5) | S8 fresh 0/20 · 0/20(창 만들기 163–222 ms · 검정 → 흰 화면 7–10 프레임 = 213–314 ms) / pool 63/63 · 62/63(보임 p50 35.8 ms · 깜빡임 0) |
  | ② | 끌기 처리 안에서 동기로 `RunMoveLoop`(= `SendMessage(WM_SYSCOMMAND, SC_MOVE\|2)`) | `PostMessage` 로 보내 메시지 펌프가 루프를 시작한다(§8) | S3 via=ipc — 루프 안에서 끝난 웹뷰 → 셸 IPC 왕복 0/21(동기 IPC 처리기 안에서 시작한 58/58 run 전부 0 · 대기 요청은 루프 끝에 풀림) / post 84–192 왕복(p50 4.1–4.6 ms · 표본 2 run → M2) |
  | ③ | 붙은 탭바에 캡처를 준다(그 탭바가 포인터를 받는다 — 조사 A7) | 셸이 8 ms 마다 커서 · 주 버튼을 읽어 그 창 웹뷰에 민다(`dock:feed`) | S4 — 누름을 못 본 웹뷰의 `setPointerCapture` 는 drag 영역 진입 89–97 ms 안에 끊기고 창 밖에서 유지 0/22 / 셸 폴링 22/22(지연 p50 0.3 · 최대 11.5 ms · 빠짐 0) |

- **스파이크가 정한 나머지 설계 사실** — 웹뷰가 포인터 캡처를 쥔 창은 `hide()` · `SW_HIDE` 하지 않는다(S2 — 캡처가 조용히 끊긴다) · 안내 창은 날 `SetWindowPos` 로 맨 위에 둔다(S7).
- **스파이크 표본의 한계**(스파이크 「미검」) — RDP · 합성 ~32 Hz · 메시지 펌프 시작 변형은 각 1–2 회 · 붙은 뒤 대상 웹뷰 밖에서 놓기 · 혼합 DPI · 세 번째 창 붙기 · Esc 밖의 키는 재지 않았다 → §12 의 실측 M 항목.
- **예비 창이 기다리는 모습 = 스파이크 C 그대로**(`visible(false)` 로 만든 뒤 화면 밖 — 주 모니터 가까이 — 에 놓고 `show()` · `spike-chrome-drag-src-2026-10-18/app/src/main.rs` setup · `win.rs` `park_origin`). 여기에 트레이 · 작업 표시줄 · Alt+Tab 에서 빼는 것만 더한다(G2 결정 · §2-5). 숨긴(`visible(false)` 그대로) 창은 쓰지 않는다 — 뗄 때 처음 보이는 모습을 잰 적이 없다. ★핸드오프가 사용자에게 보고한 문장은 「미리 숨겨 둔 창」이었다★ — 이 TRD 의 뜻은 「보이는 채 화면 밖(사용자 눈에서 숨김)」이고 OS 의 `SW_HIDE` · `visible(false)` 가 아니다(G2 · 스파이크 S8 pool 이 잰 꼴). 사용자가 보는 동작은 같다.
- **줄 포인터** — 코드는 심볼 이름으로 가리킨다. 9c 미커밋 트리의 줄 번호는 쓰지 않는다.

## 2. 아키텍처

### 2-1. 누가 무엇을 갖나

| 층 | 갖는 것 | 갖지 않는 것 |
|---|---|---|
| **웹뷰** (`src/components/layout/tabStrip/` · `dock/`) | 탭 누름(그 순간 활성 — 조사 A4) · 10 px 문턱 · 포인터 캡처 · **탭바 안 순서 이동 전부**(끄는 탭 x · 순서 문턱 · 삽입 자리 = Chromium `CalculateInsertionIndex` · 이웃 200 ms 비켜남 — §5-1) · 탭바 구역을 나갔는지(셸이 준 사각형으로) · 끄는 동안의 키(앱 전체 키 감시 — §6-3) · 보낸 사건의 `seq` · 답 기다리는 동안 끝 입력 붙들기(§4) · 탭 폭 · 숨김(R24) · 닫기 모드(R26) · 활성 탭 모양(R27) · 파일 드롭 막기(AC22) · 안내 창 그리기 · 대기 모드(빈 틀) | 모델 변경(invoke 로 셸에 청한다) · 창 사이 판정 · 창 만들기 · 옮기기 |
| **셸 세션** (`src-tauri/src/dock/` — 메인 스레드) | 끌기 세션 하나(표식 · 되돌릴 정보 · 단계 · 받은 `seq`) · 떼기 · 붙기 · 놓기 · 되돌리기 판단 · 보상(§6-1) · 탭바 구역 · 도킹 띠 · 붙는 크기 · 떼어진 창 크기 · 커서 밑 우리 창 판정 · 예비 창 · 안내 창 · 대기 창 명부(§2-5) · 붙은 뒤 커서 폴링 · 버튼 감시 · 750 ms 앞으로 올리기 | 탭바 안 기하 · DOM |
| **셸 레이아웃** (`src-tauri/src/layout/`) | 모델 변경의 유일한 길 `apply::dock_tab`(+ `switch_tab` · `close_tab`) · 표식(lease) · 묘비 · 저장 · 들인 창의 재기 옮겨 담기(§7) | 끌기 단계 |
| **platform** (`crates/engram-dashboard-platform`) | Win32 목적 함수(§8) — 이동 루프 · 훅 · z 순서 · 창 놓기 · Alt+Tab 빼기 · 버튼 · 커서 · 창 핸들 변이 맞추기 | Tauri · 레이아웃 · 끌기 단계 |

- 레이아웃 권위는 그대로 셸이다(ADR-0035) — 웹뷰는 순서 이동 자리를 **정해서 청할** 뿐이고 모델에 쓰는 것은 셸이다.
- `dock` 모듈은 `layout` 밖에 둔다(5.8 §1-1 그대로) — 끌기는 레이아웃 권위를 부르는 손님이다.

### 2-2. 단계별 흐름

```
[누름]  웹뷰 TabBar  pointerdown(왼쪽 · 탭 머리)
          → 누른 점(down) · 누르기 전 활성 탭(initial_active — 조사 E2)을 적어 둠
          → 그 탭이 비활성이면 곧바로 switch_tab invoke(누르는 순간 활성 — 조사 A4)
          (닫기 버튼 · `+` 에서 시작한 누름은 끌기가 아니다)
[문턱]  pointermove 유클리드 거리 > 10 CSS px(조사 A2)
          → setPointerCapture — 대상 = 탭바 줄 요소(탭 머리가 아니다: 순서가 바뀌면 React 가 머리 DOM 을 옮기고,
            옮겨지며 문서에서 잠깐 빠진 요소는 캡처를 잃을 수 있다 — Pointer Events 의 암묵 해제 · 미검 · 문턱 전에 잡지 않는 까닭 = 캡처를 쥐면 click · dblclick 대상이 바뀌어
            이름 바꾸기(머리 두 번 누름)가 깨진다)
          → document.activeElement.blur()(Chrome SaveFocus — 조사 E1)
          → invoke tab_drag_begin{token, view, down, area(탭 끌기 영역 — C3), grab, initial_active} ──→ 셸 세션: 표식(lease) 세움
          ←── 답: Strip{session, zone}  (탭 둘 이상)  |  WholeWindow{session}  (탭 하나 — 조사 A9)  |  Busy · Rejected
          · 답을 기다리는 동안의 놓기 · 키 · 캡처 잃음은 웹뷰가 붙들었다 답 뒤에 보낸다(§4 「순서」)
[탭바 안 · 출처]  웹뷰가 몬다(포인터 캡처)
          · 끄는 탭 = 커서를 따라 가로로 — 왼쪽 L = clamp(p − round(r·w), 0, max(0, A − w))(탭 안 폭 비율 · 끌기 영역 폭 A 로 자름 — §5-1 · C3)
          · 움직일 때마다 자리를 셈(Chromium CalculateInsertionIndex — §5-1) → 마지막으로 순서가 바뀐 점에서 round(16 × 그 자리 탭 폭 ÷ S) 넘게
            움직였고 자리가 지금과 다르면 invoke tab_drag_reorder{seq, index}
            → 셸: dock_tab(TabBar{같은 창, index}) — 모델이 곧바로 바뀜 → 이웃은 200 ms(EASE_OUT) 로 비켜남
          · 점이 zone 밖 → invoke tab_drag_leave{seq} → 답 Detached 면 웹뷰는 그 순간부터 이 끌기의 포인터 사건 · lostpointercapture 를 버린다
            · NotReady 면 계속 몰고 다시 청한다(§2-5) · Ended 면 끌기를 버린다
          · pointerup → tab_drag_end{Drop, Pointer, at} · Esc → tab_drag_end{Cancel, Key} · 그 밖의 눌린 키 → tab_drag_end{Drop, Key, at}(Chrome EventTracker — §6-3 · C9)
          · lostpointercapture(떠나기 전) → tab_drag_end{Cancel, CaptureLost}(조사 E8)
          · 셸 버튼 감시(B6)가 뗌을 먼저 보면 dock:feed{Release} → tab_drag_end{Drop, Release, at} — pointerup 과 둘 중 먼저 받은 것으로 끝낸다(E7)
[떼기]  셸 세션(메인 스레드):
          ① dock_tab(AdoptWindow{예비 창 · 명부의 재기} · 떼는 창의 다음 활성 = 붙었다가 떼는 창이면 그 창의 붙기 전 활성 자리(C8) · 아니면 출처의 initial_active)
             ← 모델에서 진짜로 옮김(조사 A6) — 이 쓰기의 결과가 다음 사건으로 돌아온 뒤 ②
          ② 예비 창을 명부에서 빼 작업 표시줄 · Alt+Tab 에 되돌리고(화면 밖에서) 커서 − 끌기 오프셋 자리에 떼어진 창 크기로 놓고 보임 · 활성(조사 A6 · E3)
             — 커서 모니터의 배율이 다르면 먼저 그 모니터 가까운 화면 밖으로 옮겨 배율을 맞춘 뒤 틀을 읽고 놓는다(두 걸음 — §5-5 · E3)
          ③ platform 이동 루프 시작(ReleaseCapture + PostMessage SC_MOVE|2 — §1 스택 맞춤 ②) — 들어갈 때 주 버튼이 이미 떨어져 있으면 그 자리에 놓은 것 ·
             들어가기 전에 Esc 를 보면 루프를 세우지 않고 되돌리기(§8 · D2) · 놓기 · 시작이 실패하면 되돌리기(T43 · D1)
          ④ 다음 예비 창은 끌기가 끝난 뒤 만든다(§2-5)
[따라오는 창]  OS 이동 루프(셸 메인 스레드에서 중첩해 돈다)
          · 끄는 창 Moved 마다: 커서 밑 맨 위 창(끄는 창 · 안내 창 제외 — 조사 D)
              → 우리 창의 탭바 구역(끌기 영역 · 위아래 ±15)이면 [붙기]   (조사 A5 · A7 — 기다림 없음)
              → 우리 창의 도킹 띠면 안내 창에 띠 + 붙을 사각형 · 아니면 안내 창 치움
              → 750 ms 시한 다시 시작(조사 E6)
          · 키: 루프 훅은 Esc 만 본다(새로 뗀 창이면 숨김) · 앱 전체 키 감시가 받은 키는 EventTracker 규칙(Esc = 되돌리기 · 그 밖 = 놓기) — §6-3 · C9
          · 루프 끝 알림(정확히 한 번 — §8): 우리가 끝냄 → 정해 둔 붙기 · 키 놓기 · 되돌리기를 이어감
                         · 뗌을 봄(WM_LBUTTONUP) → [놓기]
                         · 뗌을 못 봄(Esc · Enter 처럼 루프를 끝낸 키 · 잠금 화면 · UAC — 조사 E7 · C9) → [되돌리기]
                         · 끄는 창이 사라짐 → [끝](§6-5)
[붙기]  ① 끄는 창 숨김(SW_HIDE — 스파이크 S4 가 잰 순서 그대로 · 메인이면 숨기지 않음 — §6-4)
        ② platform 이동 루프 끝(WM_CANCELMODE — 조사 A7 「Windows 는 루프가 끝난 다음」 · 아직 안 들어간 루프면 들어올 때 삼킨다 — §8)
        ③ 루프 끝 알림에서: dock_tab(TabBar{대상 창, 0}) — Chrome 은 첫 non-pinned 자리에 넣고 곧바로 같은 자리 함수로 고친다(C1 · 우리에게 고정 탭이 없어 0)
           → 결과가 오면: 비운 끄는 창은 화면 밖 대기로(새로 뗀 창 = 예비로 되돌림 · 창째 움직이던 창 = 되돌리기용으로 남김 — R10)
           → 쓰기가 실패하면 숨긴 끄는 창을 마지막 자리에 다시 보이고 끝 — 탭은 그 창에 남는다(§6-1)
        ④ 대상 창 활성(C10)
        ⑤ emit dock:feed{Attach{zone, grab, 커서}} → 대상 웹뷰가 같은 자리 함수로 문턱 없이 곧바로 자리를 셈해 tab_drag_reorder(§5-1 · C1 just_attached)
        ⑥ 셸 폴러 시작(8 ms · 커서 · 주 버튼) → 바뀔 때만 dock:feed{Move}
[탭바 안 · 붙은 창]  웹뷰가 몬다(입력 = 셸이 민 커서) — 순서 이동 · zone 밖 → tab_drag_leave → [떼기](다시 예비 창으로)
          · 놓기 = 폴러가 주 버튼 뗌을 봄 → emit dock:feed{Release} → 그 웹뷰가 지금 자리로 tab_drag_end{Drop, Release, at}
            (1 초 — G18 — 안에 안 오면 지금 모델 자리 · E7) → [놓기] · 키 = 앱 전체 키 감시(§6-3 — holder 가 아닌 창의 키 놓기도 같은 확인을 탄다)
[놓기]  탭바 안 = 지금 배치로 끝(end 의 at 이 모델과 다르면 먼저 그 자리로) · 칸 띠 = dock_tab(Seam{…, 붙는 몫}) 후 끄는 창 예비로
        · 그 밖 = 끄는 창이 그대로 새 창
        → 공통 마무리(§6-1): 대상 창 앞으로 · 붙은 첫 칸 포커스(R11b) · 표식 내림 · 끄는 중 비워 남긴 창 닫음 · 예비 창 채움
[되돌리기]  조사 A8 RevertDrag — 원래 창 · 원래 자리 · 처음 선택 · 붙었던 창의 붙기 전 선택(C8) · 최대화(§6-2)
```

### 2-3. 끌기 세션 상태 기계 (셸 · 순수)

`src-tauri/src/dock/session.rs`. 상태는 하나의 열거와 세션 기록이다. **세션 계약의 정본은 §2-3-3 행렬이고 빈칸이 없다** — 상태 꼴 18 × 사건 꼴 29 = 522 칸마다 이름 붙은 전이(§2-3-4 T01–T44) 하나 또는 사유 붙은 무시(⊥ · ⊘ · x1–x7 — x7 은 6.3 · F1) 하나다. 리뷰 1 · 2 라운드가 전이 수준 구멍을 거듭 찾아 이 판은 「표에 없는 조합은 ⓪」 같은 나머지 규칙을 두지 않는다.

```rust
pub enum Phase {
    /// begin 의 표식 쓰기 결과를 기다림 — 늘 `cont = Begin` 과 함께.
    Starting,
    /// 탭이 어느 창 탭바 안에 있다 — 그 창 웹뷰가 순서 이동을 몬다.
    Strip { holder: WindowLabel, feed: Feed, releasing: Option<u32> /* 놓기 확인 시한 seq — T23 · E7 */ },
    /// OS 이동 루프를 보냈거나 돈다 — 셸이 몬다.
    Moving { window: WindowLabel, kind: MoveKind, lp: LoopState, hover: Hover, pending: Option<Pending> },
}
pub enum Feed { Pointer /* 출처 웹뷰가 캡처로 몬다 · 셸은 버튼만 본다 */, Shell /* 셸 폴러가 민 커서 */ }
pub enum MoveKind { Detached /* 예비 창에 떼어 담음 */, WholeWindow /* 탭 하나 창이 창째 — 메인 마지막 탭이면 window = 메인 */ }
pub enum LoopState { Posted /* SC_MOVE 를 보냈고 아직 안 들어감 */, Running, Ending /* EndMove 를 냈다 */, Over /* 끝 알림을 받았거나 서지 않았다 */ }
pub enum Hover { Nothing, Band(BandHit), Foreign }      // 안내 창에 그릴 것
pub enum Pending {                                       // 우리가 루프를 끝낸 까닭 — 처음 정한 것이 이긴다 · 단 Abandon 은 Revert 로 덮는다(T34)
    Attach { to: WindowLabel, active_before: Option<usize> /* 대상 창의 붙기 전 활성 자리 — C8 */ },
    DropByKey, Revert { by: EndBy },
}
pub enum Cont {                                          // 낸 `Write` 의 결과를 받아 이어 갈 곳(§2-4)
    Begin, Reorder, Detach, Attach { to: WindowLabel, active_before: Option<usize> }, RefillClose, StripDrop, Seam, Revert(VecDeque<RevertStep>),
}
pub enum RevertStep {                                    // 되돌리기의 차례(§6-2 · E1) — 쓰기와 쓰기 아닌 단계가 섞인다
    Write { write: LayoutWrite, then: Vec<RevertStep> /* 이 쓰기가 성공해야 도는 뒤따름 — 남긴 창 되살림의 Unpark · Restore */ },
    Unpark(WindowLabel), Restore { label: WindowLabel, normal: PhysRect, maximized: bool, behind: Option<WindowLabel>, activate: bool },
}
pub enum SpareUse { None, Taken(WindowLabel) /* 꺼냈으나 아직 안 들임 */, Adopted(WindowLabel), Lost /* 들인 뒤 사라짐 */ }
pub enum Original { None, Provisional(SizeDip) /* 창째 끌기 — 루프 전 자리에서 잰 것 */, Fixed(SizeDip) }   // §5-4 · D4 · E4
pub struct Session {
    pub id: u64 /* 앱 실행마다 1 부터 단조 — 0 = 없음(거울 · F6) */, pub token: u64 /* 웹뷰가 begin 에 실은 표 — Abandon{by: Token} 의 짝(F5) */, pub view: ViewId, pub lease: Option<LeaseId>,
    pub origin: Origin,                 // 되돌릴 곳 — 창 O · 자리 i0 · initial_active a0 · O 의 자리 · 최대화 · 탭 하나였나 · 누른 점(화면)
    pub restored_size: SizeDip,         // 출처 창 복원 바깥 크기(§5-5 — 시작 때 한 번 · 떼어진 창 크기의 바탕)
    pub original: Original,             // 붙는 크기의 「원래 크기」(R7 · §5-4)
    pub grab: Grab,                     // 탭 안 잡은 자리(폭 비율 · y)
    pub seen: HashMap<WindowLabel, u32>,// 보낸 창마다 마지막으로 받은 seq(§4 「순서」) — 그 창이 다시 읽히면 지운다(T26 · E6)
    pub mounts: HashMap<WindowLabel, u64>, // 보낸 창마다 마지막으로 본 마운트 표(F6 — 바뀌면 그 창 seen 을 0 으로 친다 · §2-3-1)
    pub armed: HashSet<WindowLabel>,    // `dock:drag{Active}` 를 보낸 창(E6) — 끝에는 `armed ∪ 모델 창` 에 `Ended`(F6)
    pub before_attach: Option<(WindowLabel, Option<usize>)>, // 지금 붙어 있는 창 · 그 창의 붙기 전 활성 자리(C8)
    pub spare: SpareUse,                // 보상 재료(§6-1)
    pub hidden: BTreeSet<WindowLabel>,  // 이 세션이 숨긴 창(붙기 결정 · 루프 Esc 숨김 `hid`) — 보상 재료 · 집합(F4 — 같은 창을 두 번 적지 않는다 · 차례가 정해져 시험이 할 일 차례를 단언한다)
    pub emptied: Vec<EmptiedWindow>,    // 끄는 중 비워 남긴 창(R10 — 되돌리기 · 끝에 닫기)
    pub main_refill: Option<ViewId>,    // 메인 마지막 탭이 떠나며 만든 빈 탭(R14)
    pub reply: Option<ReplyId>,         // 아직 답하지 않은 begin · leave
    pub cont: Option<Cont>,             // 있는 동안 다음 사건은 반드시 그 `Write` 의 `Wrote` 또는 같은 묶음의 관문 실패(§2-4 — 단언)
    pub loop_no: u32, pub poll_gen: u32, pub raise_seq: Option<u32>,   // 늦은 platform · 폴러 · 시한 사건 거르기
    pub counts: EndCounts,              // reorders · detaches · attaches · errors …(§10 끝 줄)
    pub phase: Phase,
}
pub fn accept(s: &Option<Session>, ev: &Event) -> Accept;   // 받기 거르개(§2-3-1) — Pass | Drop { why, reply } · 상태를 바꾸지 않는다
pub fn cell(s: &Option<Session>, ev: &Event, world: &World) -> Cell;   // 행렬 칸 이름(§2-3-3) — T01…T44 | Ignore(사유) | Impossible · 칸 안 조건(예비 준비됨 · holder 가 모델에 있나)은 세계로 가른다
pub fn step(s: &mut Option<Session>, ev: Event, world: &World) -> Vec<Effect>;   // accept → (웹뷰 사건이면) seen 올림 → cell → 그 전이
```

- **세 함수의 `match` 에 `_ =>` 갈래가 없다**(수렴 조건) — `Phase` · `Event` · `MoveLoopEnd` · `Cont` · `Fail` 의 변이를 전부 이름으로 부른다(or-패턴은 된다). 변이를 하나 더하면 컴파일러가 빠진 칸을 잡는다. `step` 은 `cell` 이 돌려준 이름 하나로만 갈라 전이를 고른다 — 같은 조합을 두 곳에서 따로 판정하지 않는다.
- **`Impossible`(⊥) 칸** — debug 빌드는 단언으로 멈추고, release 빌드는 무시하며 `errors` 를 센다(끝 줄에 남는다).

**사건 `Event`**(어디서 오나 — §2-4):

| 사건 | 출처 · 실린 것 |
|---|---|
| `Begin{token, view, window, down, area, grab, initial_active, reply}` | 웹뷰 `tab_drag_begin` |
| `Reorder{session, seq, mount, index}` | 웹뷰 `tab_drag_reorder` — `mount` = 웹뷰 마운트 표(F6 · §4) |
| `Leave{session, seq, mount, reply}` | 웹뷰 `tab_drag_leave` |
| `End{session, seq, mount, how: Drop \| Cancel, src: Pointer \| Release \| CaptureLost \| Key, at: Option<u32>}` | 웹뷰 `tab_drag_end` — `Pointer` = 출처 캡처의 뗌 · `Release` = 셸의 `dock:feed{Release}` 에 holder 가 지금 자리로 답한 것(E7) · `CaptureLost` · `Key`(그 웹뷰 keydown 또는 앱 전체 키 감시). 받는 짝은 다섯 — `{Drop, Pointer}` · `{Drop, Release}` · `{Drop, Key}` · `{Cancel, Key}` · `{Cancel, CaptureLost}` |
| `Abandon{by: Token(u64) \| Session(u64)}` | begin · leave 처리기가 메인 답 시한(G18 · 1 초)을 넘김 — begin = `Token(token)`(아직 세션 번호가 없다) · leave = `Session(session)`(leave 가 실은 세션 번호 — 붙은 창 웹뷰는 begin 표를 모른다 · F5) |
| `PageLoaded(label)` | Tauri `on_page_load(PageLoadEvent::Started)` — 우리 창 아무것이나(E6 · 6.1 의 `HolderReloaded` 를 넓힘) |
| `Moved{label, cursor, probe, geom}` | 끄는 창 `WindowEvent::Moved` + 실행기가 읽은 커서(사건이 난 때 — 줄에 넣을 때) · 커서 밑 창(`probe`) · 끄는 창 OS 기하(`geom`) — `probe` · `geom` 은 줄에서 꺼내 `step` 에 넘기기 바로 전에 읽는다(F3 c — 줄에서 기다리는 동안 두 걸음 놓기 · 다시 놓기 · 숨기기가 창을 옮기거나 z 를 바꿀 수 있다) |
| `LoopEntered{loop_no, geom}` | platform 루프 알림(`WM_ENTERSIZEMOVE`) |
| `LoopEnded{loop_no, end: MoveLoopEnd, hid: bool, cursor, geom}` | platform 끝 알림 — `Released{before_entry}` · `Ended` · `Canceled{esc}` · `Destroyed` · 고리마다 정확히 한 번(§8) · `hid` = 이 고리의 Esc 훅이 그 창을 숨겼나(끝 까닭과 무관하게 모든 끝에 실린다 — F2) |
| `WindowGone(label)` | 우리 창의 `WindowEvent::Destroyed`(명부가 먼저 지운 뒤 — §2-5) |
| `WorldChanged` | 쓰기 묶음 끝(`LayoutEvents::settled`) · 크롬 · 캔버스 보고 |
| `Pointer{gen, cursor, down}` | 셸 폴러(바뀔 때만) — Strip{Pointer} 에서는 주 버튼만 본다 |
| `RaiseDue{seq}` · `ReleaseDue{seq}` | 시한 스레드 — 750 ms(조사 E6) · 놓기 확인(G18 · 1 초) |
| `Wrote{session, result: Result<WriteOutcome, WriteError>}` | 실행기 — 바로 앞 `Write` 의 결과(줄 맨 앞 — §2-4) |
| `Failed{session, fail: Take \| Start \| Soft{op, label}, err}` | 실행기 — 할 일 실패(§2-4 「할 일 실패」 · D1) |

`geom` = 끄는 창의 `{outer: PhysRect, client: PhysSize, scale, settled: bool}` — 실행기가 그 사건을 줄에서 꺼낼 때 읽는다(F3 c) · 창이 없으면 `None`. `settled` = 실행기가 붙이는 표 — 막 뗀 창(Detached)이면 그 창의 지금 배율이 `PlaceDetached` 가 셈에 쓴 목표 배율과 같을 때만 참 · 그 밖의 창 · 끌기는 늘 참(F3 b · §5-5).

**할 일 `Effect`**(실패 등급 = §2-4 「할 일 실패」 · ⓣ = 관문 Take · ⓢ = 관문 Start · ⓞ = 무름 + 소유 넘김(실패하면 실행기가 그 창을 명부의 고아로 넘긴 뒤 `Failed` 를 낸다 — F1) · 표시 없음 = 무름 · ∅ = 실패 없음). 등급은 할 일 값에 실린다(`Effect` 의 실패 등급 — U5a-1) — 실행기가 묶음 맥락으로 추측하지 않는다:

| 할 일 | 하는 일 |
|---|---|
| `Write(LayoutWrite)` — 결과 = `Wrote` | `Lease{view}` · `Dock(SessionDock)`(늘 `OnEmptied::Keep` · `keep_lease`) · `Switch{window, view}` · `CloseTab{window, view}` — ★묶음의 마지막★ |
| `SpareTake` ⓣ | 명부의 예비 창을 꺼냄 — 준비 안 됨 · 사라짐이면 실패 |
| `PlaceDetached{label, plan, activate}` ⓢ | 떼어진 창 놓기 — 실행기가 그때 읽은 틀 두께 · 배율로 `sizing::detached_origin` · 두 걸음(§5-5 · E3) |
| `PlaceWindow{label, rect: Option<PhysRect>, z, show, activate}` · `RestorePlacement{label, normal, maximized, behind, activate}` | 창째 끌기 시작 묶음(T03)에서는 ⓢ · 공통 마무리 3 의 다시 보이기(`PlaceWindow{rect: None, z: Keep, show: true}` — 제자리 · F7 b)는 ⓞ · 그 밖은 무름 |
| `BeginMove{label, hide_on_escape, loop_no}` ⓢ | platform 이동 루프 시작(§8) |
| `EndMove(label)` ∅ | platform 이동 루프 끝 — 결과는 `LoopEnded` |
| `SpareGiveBack{label, measures}` ⓞ · `CloseWindow(label)` ⓞ | 무름 + 소유 넘김(F1 · §2-4 「소유 넘김」 · §2-5 「고아」) — 세션은 이 할 일을 내는 때 그 창을 놓는다(`spare` · `hidden` · `emptied` 에서 뺌). 실패하면 실행기가 같은 걸음에서 그 창을 명부의 고아로 넘긴 뒤 `Failed(Soft)` 를 낸다 · `CloseWindow` 는 실패와 무관하게 늘 넘긴다(닫기의 성공 = `Destroyed` — 명부의 소멸 갈래가 항목을 지운다) |
| `Hide(label)` · `Park(label, measures)` · `Unpark(label)` · `Focus(label)` · `RaiseNoActivate(label)` · `Overlay(Option<OverlayView>)` · `Emit(…)` · `Reply(ReplyId, Reply)` · `Release(lease)` · `SpareFill` | 무름 — `Focus` = platform `place_window(rect: None, z: Top, show: false, activate: true)` 로만(F7 a — Tauri `set_focus` 를 부르지 않는다 · §6-3 · §8) · `SpareFill` = 명부에 예비(짓는 중 · `Spare` · `respawn` 고아)가 없을 때만 짓는다(F1 · G11) |
| `PollButton` · `PollStart(holder)` · `PollStop` · `ArmRaise(seq)` · `ArmRelease(seq)` | 무름 — 앱 수명 폴러 · 시한 스레드(setup 에서 띄움)에 보내는 명령. 그 스레드가 없을 때만 실패한다(ADR-0292 공황 훅 아래에서는 과정이 끝나는 때 — 실제로는 거의 없다) |
| `Log(…)` ∅ | 끝 줄 · 단계 로그(§10) |

`Park` = 화면 밖 · 보이는 채 · 명부 `Kept` · Alt+Tab · 작업 표시줄에서 뺌 · `dock:spare{Parked}`(대기 모드 — 0탭 자가닫힘을 끈다 · §2-5) / `Unpark` = 명부에서 빼고 작업 표시줄 · Alt+Tab 에 되돌림(화면 밖 그대로 — 자리는 뒤 할 일이 놓는다) · `dock:spare{Adopted}` / `SpareGiveBack` = 명부에 `Spare` 로 · 화면 밖 · 보이는 채(숨어 있었으면 다시 보임) · Alt+Tab · 작업 표시줄에서 뺌 · `dock:spare{Parked}`(§2-5) — 단계 하나라도 실패하면 역할이 `Spare` 가 아니라 고아 `Orphan{GiveBack}` 이다(F1). 셋 다 platform 함수로만 창을 만진다(§2-5 「tao 깃발 불변식」 · E2). `Park` 가 실패해도 그 창은 세션 `emptied` 에 남아 공통 마무리 5 가 닫는다(소유가 세션에 있다 — ⓞ 가 아니다).

#### 2-3-1. 받기 거르개 `accept` (A3 · B5 · D5)

`step` 앞에서 차례로 본다. 하나라도 걸리면 그 사건은 `step` 에 닿지 않는다(행렬의 ⊘).

1. **늦음** — 웹뷰 사건의 `session` · `Abandon` 의 `by`(`Token(t)` 면 `token` 과 · `Session(s)` 면 `id` 와 — 실린 쪽만 비교한다 · F5) · platform 사건의 `loop_no`(Moving 이고 `lp ≠ Over` 일 때만 지금 고리가 있다) · 폴러의 `gen`(폴러 명령 `PollButton` · `PollStart` · `PollStop` 마다 올린다) · 시한의 `seq`(`raise_seq` 는 pending 을 세우거나 Moving 을 떠날 때 지운다 · 놓기 확인은 `releasing`) · `Wrote` · `Failed` 의 `session` 이 지금 것이 아니면 버린다. `Moved` 는 Moving 의 끄는 창 label 만 받는다(메인이 돌아가며 낸 `Moved` 도 여기서 빠진다 — R14). `Begin` · `PageLoaded` · `WindowGone` · `WorldChanged` 는 이 거르개를 지나 `step` 이 칸으로 가른다. 세션이 없을 때(I)의 `Wrote` · `Failed` 도 이 거르개를 지나 칸이 가른다 — I × `Wrote` · 관문 `Failed` = ⊥ · I × `Failed(Soft)` = x7(방금 끝난 세션의 끝 묶음 — F1 · 6.3 에서 적음: 6.2 는 「지금 것이 아니면 버린다」만 적어 그 칸이 ⊘ 와 겹쳐 읽혔다).
2. **꼴** — `End` 의 `(how, src)` 가 위 다섯 짝 밖이면 버린다.
3. **출처 · 단계**(아래 표) — ★`seq` 보다 먼저 본다(D5)★ — 버려질 사건이 `seen` 을 올려 그보다 앞서 보낸 정상 사건을 낡게 만들지 않게.
4. **낡음** — 웹뷰 사건의 `mount` 가 `mounts[보낸 창]` 과 같고 `seq ≤ seen[보낸 창]` 이면 버린다. ★`mount` 가 다르면 그 창의 `seen` 을 0 으로 친다(버리지 않는다 — F6 c)★ — 다시 읽힌 창(T26 이 `seen` 을 지움)에 옛 페이지가 보낸 늦은 사건이 닿아 `seen` 을 다시 올려도, 새 페이지의 사건은 `mount` 가 달라 낡음으로 버려지지 않는다. 옛 페이지의 늦은 사건도 같은 까닭으로 받는다 — 사용자가 다시 읽기 전에 실제로 낸 입력이고 늦게 온 사건과 같다(드문 갈림 — 사건마다 절대값을 실어 잃는 것이 없다 · §4 「순서」).
5. **답** — 버린 `Leave` 에는 곧바로 `Ended` 로 답한다(처리기가 시한까지 기다려 `Abandon` 을 내지 않게). `Begin` 은 이 거르개가 버리지 않는다(세션이 있으면 `step` 이 `Busy` — T17).

통과한 웹뷰 사건은 `step` 이 맨 먼저 `mounts[보낸 창] = mount` · `seen[보낸 창] = seq` 로 둔다(`accept` 는 상태를 바꾸지 않는다).

| 사건 | Strip{Pointer} | Strip{Shell} | Moving |
|---|---|---|---|
| `Reorder` · `Leave` | holder 웹뷰만 | holder 웹뷰만 | 버림(단계) |
| `End{Drop, Pointer}`(pointerup) | holder 만 | 버림 — Shell 입력 웹뷰는 제 포인터 사건을 쓰지 않는다(대상 웹뷰의 pointerup 도 · 뗌은 폴러가 본다) | 버림 — 뗌은 루프 끝이 본다 |
| `End{Drop, Release}` | holder 만 · `releasing` 일 때만 | holder 만 · `releasing` 일 때만 | 버림(단계) |
| `End{_, Key}` | 아무 우리 창 | 아무 우리 창 | 아무 우리 창 — Chrome `EventTracker` 는 끌기 내내 산다(C9 · §6-3) |
| `End{Cancel, CaptureLost}` | holder 만 → 되돌리기(조사 E8) | 버림 | 버림(떠난 출처의 늦은 캡처 잃음 — 스파이크 S3) |
| `Pointer`(폴러) | 받음(`gen` 맞으면) | 받음 | 버림(`PollStop` 뒤의 늦은 표본 — `gen`) |

#### 2-3-2. 상태 꼴 · 사건 꼴

**상태 꼴 18**(행렬의 열):

| 꼴 | 뜻 |
|---|---|
| `I` | 세션 없음(Idle) |
| `Wb` · `Wr` · `Wd` · `Wa` · `Wc` · `Ws` · `Wm` · `Wv` | `cont` 가 선 동안 — 차례로 `Begin`(= `Phase::Starting`) · `Reorder` · `Detach` · `Attach` · `RefillClose` · `StripDrop` · `Seam` · `Revert`. 밑 단계(Strip · Moving)는 그대로지만 이 동안 처리되는 사건은 그 `Write` 의 결과뿐이라 따로 센다 |
| `SP` · `SPr` | Strip{Pointer} — `releasing` 없음 · 있음 |
| `SS` · `SSr` | Strip{Shell} — `releasing` 없음 · 있음 |
| `MP` · `MR` | Moving · pending 없음 — `lp = Posted` · `Running` |
| `MA` · `MK` · `MV` | Moving · `lp = Ending` — pending `Attach` · `DropByKey` · `Revert` |

`Moving` 의 `kind`(Detached · WholeWindow · 메인)는 한 전이 안에서 할 일이 갈린다(§2-3-4 의 「Detached: … / WholeWindow: … / 메인: …」). 「holder 인가」 · 「예비 준비됨인가」 · 「모델에 있나」 같은 갈래는 칸 안에서 전이 이름이 갈린다(`T19·T20` 처럼 적은 칸 — §2-3-4 의 조건). 어느 쪽이든 조건이 빠짐없이 둘 이상으로 나뉘어 있다.

**사건 꼴 29**(행렬의 행) — 위 `Event` 를 끝 까닭 · 결과 · 실패 등급까지 나눈 것: `Begin` · `Reorder` · `Leave` · `End` 다섯 짝 · `Abandon` · `PageLoaded` · `LoopEntered` · `Moved` · `LoopEnded` 여섯(`Released{false}` · `Released{true}` · `Ended` · `Canceled{esc: false}` · `Canceled{esc: true}` · `Destroyed`) · `WindowGone` · `WorldChanged` · `Pointer{down}` 둘 · `RaiseDue` · `ReleaseDue` · `Wrote(Ok)` · `Wrote(Err)` · `Failed(Take)` · `Failed(Start)` · `Failed(Soft)`. 실린 값(`LoopEnded` 의 `hid` · `geom.settled` · `Abandon` 의 `by` · `mount`)은 꼴을 늘리지 않는다 — 받기 거르개가 보거나 전이 안에서 할 일이 갈린다(F2 · F3 · F5 · F6). 명부 고아의 다시 하기 시한(`OrphanDue` — §2-5)은 세션 사건이 아니다 — 실행기가 명부로 보내고 `step` 에 닿지 않아 행렬에 행이 없다(F1).

#### 2-3-3. 행렬 — 상태 꼴 × 사건 꼴 (정본)

칸 = 전이 이름(§2-3-4) 또는 무시 사유(아래). `A·B` = 칸 안의 조건으로 갈리는 둘 이상(§2-3-4 의 그 전이 조건).

| 사건 ↓ · 상태 → | I | Wb | Wr | Wd | Wa | Wc | Ws | Wm | Wv | SP | SPr | SS | SSr | MP | MR | MA | MK | MV |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `Begin` | T01 | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | T17 | T17 | T17 | T17 | T17 | T17 | T17 | T17 | T17 |
| `Reorder` | ⊘ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | T18 | T18 | T18 | T18 | ⊘ | ⊘ | ⊘ | ⊘ | ⊘ |
| `Leave` | ⊘ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | T19·T20 | T21 | T19·T20 | T21 | ⊘ | ⊘ | ⊘ | ⊘ | ⊘ |
| `End{Drop, Pointer}` | ⊘ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | T22 | T22 | ⊘ | ⊘ | ⊘ | ⊘ | ⊘ | ⊘ | ⊘ |
| `End{Drop, Release}` | ⊘ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊘ | T22 | ⊘ | T22 | ⊘ | ⊘ | ⊘ | ⊘ | ⊘ |
| `End{Drop, Key}` | ⊘ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | T22·T23 | T22·x2 | T22·T23 | T22·x2 | T32 | T32 | x1 | x1 | x1 |
| `End{Cancel, Key}` | ⊘ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | T24 | x2 | T24 | x2 | T33 | T33 | x1 | x1 | x1 |
| `End{Cancel, CaptureLost}` | ⊘ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | T24 | x2 | ⊘ | ⊘ | ⊘ | ⊘ | ⊘ | ⊘ | ⊘ |
| `Abandon` | ⊘ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | T24 | T24 | T24 | T24 | T33 | T33 | T34 | T34 | x1 |
| `PageLoaded` | x5 | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | T25·T26 | T25·T26 | T25·T26 | T25·T26 | T26 | T26 | T26 | T26 | T26 |
| `LoopEntered` | ⊘ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊘ | ⊘ | ⊘ | ⊘ | T35 | ⊥ | x6 | x6 | x6 |
| `Moved` | ⊘ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊘ | ⊘ | ⊘ | ⊘ | T36 | T36 | x1 | x1 | x1 |
| `LoopEnded(Released{false})` | ⊘ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊘ | ⊘ | ⊘ | ⊘ | T37 | T37 | T38 | T37 | T39 |
| `LoopEnded(Released{true})` | ⊘ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊘ | ⊘ | ⊘ | ⊘ | T37 | T37 | T38 | T37 | T39 |
| `LoopEnded(Ended)` | ⊘ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊘ | ⊘ | ⊘ | ⊘ | T39 | T39 | T38 | T37 | T39 |
| `LoopEnded(Canceled{esc: false})` | ⊘ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊘ | ⊘ | ⊘ | ⊘ | T39 | T39 | T38 | T37 | T39 |
| `LoopEnded(Canceled{esc: true})` | ⊘ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊘ | ⊘ | ⊘ | ⊘ | T39 | T39 | T38 | T37 | T39 |
| `LoopEnded(Destroyed)` | ⊘ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊘ | ⊘ | ⊘ | ⊘ | T40 | T40 | T40 | T40 | T40 |
| `WindowGone` | x5 | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | T27·T28·x4 | T27·T28·x4 | T27·T28·x4 | T27·T28·x4 | T40·T28·x4 | T40·T28·x4 | T40·T28·x4 | T40·T28·x4 | T40·T28·x4 |
| `WorldChanged` | x5 | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | T27·T29 | T27·T29 | T27·T29 | T27·T29 | T41 | T41 | T41 | T41 | T41 |
| `Pointer{down: true}` | ⊘ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | x3 | x2 | T30 | x2 | ⊘ | ⊘ | ⊘ | ⊘ | ⊘ |
| `Pointer{down: false}` | ⊘ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | T23 | x2 | T23 | x2 | ⊘ | ⊘ | ⊘ | ⊘ | ⊘ |
| `RaiseDue` | ⊘ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊘ | ⊘ | ⊘ | ⊘ | T42 | T42 | ⊘ | ⊘ | ⊘ |
| `ReleaseDue` | ⊘ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊘ | T31 | ⊘ | T31 | ⊘ | ⊘ | ⊘ | ⊘ | ⊘ |
| `Wrote(Ok)` | ⊥ | T02·T03 | T05 | T07 | T10 | T12 | T13 | T14 | T16 | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ |
| `Wrote(Err)` | ⊥ | T04 | T06 | T08 | T11 | T12 | T13 | T15 | T16 | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ |
| `Failed(Take)` | ⊥ | ⊥ | ⊥ | T09 | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ |
| `Failed(Start)` | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | T43 | ⊥ | ⊥ | ⊥ | ⊥ |
| `Failed(Soft)` | x7 | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | ⊥ | T44 | T44 | T44 | T44 | T44 | T44 | T44 | T44 | T44 |

칸 522 = 전이만 124 · 무시만 387(⊥ 255 · ⊘ 107 · x 25) · 전이와 무시가 조건으로 갈리는 칸 11. 6.3 에서 바뀐 칸 = 아홉 — W 열 × `Failed(Soft)` 여덟 칸 T44 → ⊥(F4) · I × `Failed(Soft)` x5 → x7(F1 · x 수는 그대로).

**무시 사유**

| 표시 | 뜻 |
|---|---|
| ⊥ | 일어날 수 없다 — 실행기 불변식(§2-4: `cont` 가 선 동안 줄 맨 앞은 그 `Write` 의 결과 또는 같은 묶음의 관문 실패 · 관문 할 일은 T03 · T07 · T19 묶음에만 있다 · `Wrote` · `Failed` 는 산 세션의 묶음 바로 뒤에만 온다 · ★무른 실패는 `cont` 가 빈 뒤에만 처리된다★ — 결과 블록 · W 열 × `Failed(Soft)` · F4) 또는 platform 계약(고리마다 들어감 한 번 · 끝 한 번 — §8). debug 단언 · release = 무시 + `errors` |
| ⊘ | 받기 거르개가 버렸다(§2-3-1 — 늦음 · 꼴 · 출처 · 단계 · 낡음). 버린 `Leave` 는 곧바로 `Ended` |
| x1 | 이미 정했다 — 처음 정한 pending 이 이긴다(B1 — 붙기를 정해 `Hide` 를 낸 뒤의 키 · `Moved` 는 붙기를 뒤집지 않는다) · MV 의 `Abandon` 은 이미 되돌리기다 |
| x2 | 놓기를 정했다(releasing) — Chrome 은 버튼을 뗀 순간 끌기가 끝나므로 그 뒤의 키 · 캡처 잃음 · 폴러 사건은 뜻이 없다. holder 의 `at` 만 기다린다 |
| x3 | Strip{Pointer} 의 `Pointer{down: true}` — 출처 웹뷰가 캡처로 몰고 폴러는 버튼 뗌만 본다 |
| x4 | 이 세션과 무관한 창의 `WindowGone` — 대기 중 예비 · 안내 창 · 다른 모델 창(예비 · 안내 창 다시 짓기는 명부 몫 · §2-5) · 명부의 고아(세션이 이미 놓은 창 — 지우기 · 새 예비는 명부 몫 · §2-5 「고아」 · F1) · 떼어진 뒤의 출처 O(되돌리기가 세계에서 「O 없음」으로 본다 — §6-2 ④) · 붙기를 정한 대상 W(T38 의 쓰기가 실패해 T11 로 간다) |
| x5 | 세션 없음 — Idle 의 `PageLoaded` · `WindowGone` · `WorldChanged` |
| x6 | 늦은 들어감 — pending 을 세워 `EndMove` 를 낸 뒤 온 `LoopEntered`. 끝은 platform 이 자기 상태로 이미 정했다(보냄 상태면 삼킴 · 들어간 뒤면 `WM_CANCELMODE` — §8) |
| x7 | 끝 묶음의 무른 실패 — 공통 마무리 묶음의 결과 블록이라 세션이 이미 없다(실행기가 warn 으로 적었다 · 끝 줄 `errors` 에는 들지 않는다). ★그 실패가 정리 할 일(ⓞ — `CloseWindow` · `SpareGiveBack` · 공통 마무리 3 의 다시 보이기)이면 실행기가 그 `Failed` 를 줄에 넣기 전에 그 창을 명부의 고아로 넘겼다★ — 세션이 없어도 정리는 명부가 끝까지 한다(§2-4 「소유 넘김」 · §2-5 「고아」 · F1). 6.2 는 이 칸을 x5(로그만)로 두어 공통 마무리의 닫기가 실패한 남긴 창이 화면 밖에 영영 남았다 |

#### 2-3-4. 전이 (T01–T44)

「다음」 = 바뀌는 상태 · 「할 일」 = 순서대로(ⓣ ⓢ = 관문 — 실패하면 그 뒤를 돌리지 않는다 · §2-4). `Write` 는 늘 그 줄 할 일의 끝이다. [끝] = 공통 마무리(§6-1) · [되돌리기] = §6-2 의 계획을 세워 `RevertStep` 을 내기 시작함(T16 이 이어 감). `Reply` 를 내는 전이는 `reply = None` 으로 둔다 · 웹뷰 사건이 들어온 전이는 그 앞에 `seen` 을 올렸다(§2-3-1).

| 전이 | 칸(상태 · 사건 · 조건) | 다음 | 할 일 |
|---|---|---|---|
| T01 | I · `Begin` | `Starting` · `cont = Begin` · `reply = begin` | 되돌릴 정보 기록(`Origin` · `restored_size` — §5-5) · `Write(Lease{view})` |
| T02 | Wb · `Wrote(Ok)` · 그 창 탭 ≥ 2 | Strip{출처 O, Pointer} | `Reply(Strip{session, zone})` · `Emit(drag Active → 모델 창 전부)` · `armed` = 그 창들 · `PollButton` |
| T03 | Wb · `Wrote(Ok)` · 그 창 탭 1 | Moving{O, WholeWindow, Posted} · `loop_no += 1` | `Reply(WholeWindow{session})` · `Emit(drag Active → 모델 창 전부)` · 최대화면 `RestorePlacement{O, normal = (복원 원점 + (커서 − 누른 점), 그 창의 dragged_size), maximized: false}` ⓢ · 아니면 `PlaceWindow(O, 지금 원점 + (커서 − 누른 점))` ⓢ(§5-5 · C4) · `BeginMove{O, hide_on_escape: false}` ⓢ — `original` 은 다음 platform · 창 사건의 `geom` 에서(§5-4 · D4) |
| T04 | Wb · `Wrote(Err)` | 세션 없음 | `Reply(Rejected(reason))` · 거절 로그(`Busy` · `TabNotFound` · `Dragging` …) — 쥔 것이 없어 공통 마무리를 타지 않는다 |
| T05 | Wr · `Wrote(Ok)` | `cont = None` | `reorders` 셈 |
| T06 | Wr · `Wrote(Err)` | `cont = None` | `errors` 셈 · debug(웹뷰는 메아리를 따른다 — §5-1) |
| T07 | Wd · `Wrote(Ok(out))` | Moving{예비, Detached, Posted} · `spare = Adopted(예비)` · `loop_no += 1` · `before_attach = None` · `detaches += 1` · `armed += 예비` | `out.main_refilled` 이면 `main_refill` 에 적음 · `Unpark(예비)` · `Emit(drag Active → 예비)`(E6 — 들인 예비의 키 감시 · 카운터) · `Reply(Detached)` · `Emit(feed holder Detached)` · `PollStop` · `PlaceDetached(예비 · 이 떼기의 dragged_size · §5-5 자리 · activate)` ⓢ · `BeginMove{예비, hide_on_escape: true}` ⓢ · `out.emptied` 가 있으면(떼는 창이 비었다 — 다른 경로가 나머지 탭을 닫았다) `emptied` 에 적고 `Park(그 창)` — 그 창 웹뷰의 캡처는 앞의 `BeginMove` 가 이미 풀었다(S2) · `original` 은 다음 platform · 창 사건의 `geom` 에서(§5-4) |
| T08 | Wd · `Wrote(Err)` | 끝 | [끝](보상이 꺼낸 예비를 되돌리고 답 `Ended`) · `outcome = strip` · `end_by = failed` · `reason` |
| T09 | Wd · `Failed(Take)` | Strip 그대로 · `cont = None` · `spare = None` | `Reply(NotReady)`(§2-5 — 웹뷰가 다음 움직임에 다시 청한다) — 그 묶음의 `Write` 는 돌지 않았다 |
| T10 | Wa · `Wrote(Ok(out))` | Strip{W, Shell} · `before_attach = (W, a)` · `attaches += 1` · `armed += W` | 비운 끄는 창 — Detached: `SpareGiveBack{예비, out.emptied.measures}` ⓞ · `spare = None` · `hidden` 에서 뺌(실패하면 그 예비는 명부의 고아 `Orphan{GiveBack}` — 준비된 예비가 아니라 다시 떼면 T20 · F1) / WholeWindow(메인 아님): `emptied` 에 적고 `Park(O)` · `hidden` 에서 뺌 / 메인: `main_refill = out.main_refilled` · `RestorePlacement(메인, 시작 자리 · 최대화, behind = W, activate = false)`(§6-4) — 다음 `Focus(W)`(C10) · `Emit(feed W Attach{session, zone, grab, x, y})`(W 웹뷰의 키 감시 · 카운터를 세운다 — E6) · `PollStart(W)` · W = 메인이고 `main_refill` 이 아직 빈 탭이면(G14) 끝에 `Write(CloseTab(메인, main_refill))` · `cont = RefillClose` |
| T11 | Wa · `Wrote(Err)` | 끝 | [끝](보상 3 이 숨긴 끄는 창을 마지막 자리에 다시 보이고 활성 · 탭은 그 창에 남아 쓸 수 있다 — A1) · `outcome = new_window`(Detached) · `moved`(WholeWindow) · `end_by = failed` · `reason` |
| T12 | Wc · `Wrote(_)` | `cont = None` · `main_refill = None` | Err 는 `errors` |
| T13 | Ws · `Wrote(_)` | 끝 | [끝 — 탭바 안] · Err 는 `errors` |
| T14 | Wm · `Wrote(Ok(out))` | 끝 | 비운 끄는 창 — Detached: `SpareGiveBack` ⓞ · `spare = None` / WholeWindow: `emptied` 에 적음(공통 마무리가 닫는다 — R10 · AC26) / 메인: `main_refill` · `RestorePlacement`(§6-4) → [끝 — 띠] · `outcome = dock` |
| T15 | Wm · `Wrote(Err)` | 끝 | [끝] — 탭은 끄는 창에 남는다(보이는 채) · `outcome = new_window \| moved` · `end_by = failed` · `reason` |
| T16 | Wv · `Wrote(r)` | 다음 단계 또는 끝 | `r = Ok(out)`: `out.emptied` 가 예비면 `SpareGiveBack{예비, measures}` ⓞ · `spare = None` · `hidden` 에서 뺌 / 다른 창이면 `emptied` 에 적음(끝에 닫힌다) · `out.main_refilled` 이면 `main_refill` 에 적음 · 남긴 창을 되살린 쓰기(`AdoptWindow{O}`)면 `emptied` 에서 O 를 뺌 · 그 쓰기의 `then` 을 다음 단계 앞에 붙임 · `r = Err`: `errors` 셈 · 그 쓰기의 `then` 은 버린다(되살리지 못한 창을 보이지 않게 — 그 창은 `emptied` 에 남아 공통 마무리가 닫는다) — 그다음 남은 `RevertStep` 을 앞에서부터 할 일로 낸다(쓰기 아닌 단계는 그대로 · 처음 만나는 `Write` 를 묶음의 끝으로 내고 `cont = Revert(나머지)`) · 남은 쓰기가 없으면 남은 단계 뒤 [끝](`outcome = reverted`) — 되돌리기는 실패해도 끝까지 간다(E1) |
| T17 | S* · M* · `Begin` | 그대로 | `Reply(Busy)` |
| T18 | S* · `Reorder{i}` | `cont = Reorder` | `Write(Dock(TabBar{holder, i}))` — releasing 중에도(holder 가 확인 답보다 먼저 보낸 마지막 순서) |
| T19 | SP · SS · `Leave` · 명부의 예비 준비됨(§2-5) | `cont = Detach` · `spare = Taken(예비)` · `reply = leave` | `SpareTake` ⓣ · `Write(Dock(AdoptWindow{예비, seed: None, measures: 명부의 재기}, successor — §7 ④))` |
| T20 | SP · SS · `Leave` · 예비 없음 | 그대로 | `Reply(NotReady)`(§2-5) · `spare_wait_ms` 셈 |
| T21 | SPr · SSr · `Leave` | 그대로 | `Reply(NotReady)` — 놓기를 이미 정했다 · 웹뷰는 떼지 않고 붙든 끝(`Release` 답 포함)을 보낸다(§4) |
| T22 | S* · holder 의 놓기(`End{Drop, Pointer \| Release \| Key}` — `at` 을 싣는다) | `at` 이 있고 모델과 다르면 `cont = StripDrop` · 아니면 끝 | `Write(Dock(TabBar{holder, at}))` / [끝 — 탭바 안] · `end_by = pointer \| release \| key` |
| T23 | SP · SS · `Pointer{down: false}` · holder 아닌 창의 `End{Drop, Key}` | `releasing = Some(seq)` | `Emit(feed holder Release)` · `ArmRelease(seq)` — holder 는 지금 자리를 `at` 으로 실어 `End{Drop, Release}` 로 답한다(E7 · D5 — 날아가던 마지막 `reorder` 를 앞지른 놓기가 그 자리를 잃지 않게). Strip{Pointer} 의 그 뗌은 B6 의 버튼 감시다(holder 의 pointerup 이 먼저 오면 T22 가 끝낸다) |
| T24 | SP · SS · `End{Cancel, Key}` / SP · `End{Cancel, CaptureLost}` / S* · `Abandon` | 되돌리기 | [되돌리기] · `end_by = esc \| capture_lost \| abandoned` — `Abandon` 은 releasing 중에도 끝이다(D3) |
| T25 | S* · `PageLoaded(holder)` | 끝 | [끝 — 탭바 안] · `end_by = holder_reloaded`(그 웹뷰의 컨트롤러가 사라졌다 — 탭은 지금 자리) |
| T26 | S* · M* · `PageLoaded(그 밖 창)` | 그대로 · `seen` · `mounts` · `armed` 에서 그 창을 뺌 | — 다시 읽힌 웹뷰는 카운터가 0 부터라 옛 `seen` 이 새 사건을 낡게 만든다(E6) · 그 뒤 옛 페이지의 늦은 사건이 `seen` 을 다시 올려도 새 페이지는 `mount` 가 달라 버려지지 않는다(§2-3-1 ④ · F6 c). 그 웹뷰는 마운트 때 `dock:drag` 듣기를 건 뒤 `window_role` 답의 `drag` 로 지금 세션을 다시 받아 키 감시를 다시 켠다 — `lastEnded` 보다 클 때만(§4 · F6 a) |
| T27 | S* · `WindowGone(holder)` · `WorldChanged` 에서 holder 나 그 탭이 모델에 없음 | 끝 | [끝](§6-5) · `outcome = aborted` · `end_by = holder_gone` — 둘 중 먼저 온 쪽이 끝내고 다른 쪽은 세션이 없어 버린다 |
| T28 | S* · M* · `WindowGone(남긴 창 — emptied 의 창)` | 그대로 · `emptied` 에서 뺌 | — 되돌리기는 그 창을 「O 없음」으로 본다(§6-5) |
| T29 | S* · `WorldChanged`(T27 밖) | 그대로 | 세계에 새로 든 모델 창(`armed` 밖)에 `Emit(drag Active)` · `armed` 에 더함(E6 — Chrome `EventTracker` 는 앱 전체다) |
| T30 | SS · `Pointer{down: true}` | 그대로 | `Emit(feed holder Move{x, y})`(커서 → holder 뷰포트 CSS px — 세계의 기하로) |
| T31 | SPr · SSr · `ReleaseDue` | 끝 | [끝 — 탭바 안] · 지금 모델 자리 · `end_by = release_timeout` |
| T32 | MP · MR · `End{Drop, Key}` | `pending = DropByKey` · `lp = Ending` · `raise_seq = None` | `EndMove` |
| T33 | MP · MR · `End{Cancel, Key}` · `Abandon` | `pending = Revert{esc \| abandoned}` · `lp = Ending` · `raise_seq = None` | `Overlay(None)` · `EndMove` |
| T34 | MA · MK · `Abandon` | `pending = Revert{abandoned}` | — `EndMove` 는 이미 냈다. ★`Abandon` 은 끝이다(D3)★ — 정한 붙기 · 키 놓기를 버리고 루프 끝에서 되돌린다(붙기로 숨긴 끄는 창은 되돌리기 결과 · 공통 마무리가 다시 보인다) |
| T35 | MP · `LoopEntered` | `lp = Running` | — |
| T36 | MP · MR · `Moved{cursor, probe, geom}` | 아래 판정대로 | ① `original` — `None` 이면 `geom` 으로 적음(Detached = `Fixed` — 단 `geom.settled` 일 때만 · 아니면 `None` 그대로 두고 다음 사건을 기다린다(F3 b — 두 걸음 놓기의 배율 바뀜이 아직 자리 잡지 않은 크기) · WholeWindow = `Provisional`) · WholeWindow 이고 `lp = Running` 이고 `Provisional` 이면 이 `geom` 으로 `Fixed`(OS 가 Snap · 최대화를 푼 뒤 — R7 · E4) · `geom` 이 없으면 그대로 ② 판정(§2-2 [따라오는 창]) — 탭바 구역(W) = `Overlay(None)` · 끄는 창이 메인이 아니면 `Hide(끄는 창)` + `hidden` 에 적음 · `EndMove` · `pending = Attach{W, W 의 지금 활성 자리}` · `lp = Ending` · `raise_seq = None` / 띠 = `Overlay(띠 · 미리보기)` · `ArmRaise(새 seq)` / 아니면 `Overlay(None)` · `ArmRaise(새 seq)` |
| T37 | MP · MR · `LoopEnded(Released)` / MK · `LoopEnded(Destroyed 밖)` | `lp = Over` · 띠면 `cont = Seam` · 아니면 끝 | ① `original` 이 `None` 이면 이 사건의 `geom` 으로(D4 — 루프에 들기 전 놓기 · Detached 는 T36 ① 처럼 `settled` 일 때만) ② `hid` 면 끄는 창을 `hidden` 에 적음(훅이 숨겼다 — ★끝 까닭과 무관★: 들어가기 전 끝냄 청과 겹친 Esc 의 `Ended`(MK) · Esc 뗌 · 반복을 본 뒤의 `Released{false}` 도 · F2 — 6.2 는 `Canceled{esc: true}` 에서만 적어 숨은 창이 남았다) ③ 놓은 순간 커서로 지금 세계에서 다시 판정(§6-1): 띠 = `Overlay(None)` · `Write(Dock(Seam{anchor, side, share}))` / 그 밖 = [끝 — 따라오는 창] · `outcome = new_window \| moved` · `end_by = loop_release \| key`. `original` 이 끝내 없으면(그 창 기하를 못 읽음) 띠를 내지 않는다(새 창 · `errors`) |
| T38 | MA · `LoopEnded(Destroyed 밖)` | `lp = Over` · `cont = Attach{W, a}` | (`hid` 면 T37 ② 와 같은 적기 — F2) · `Write(Dock(TabBar{W, Some(0)}))`(C1) — pending 이 루프 끝 까닭보다 앞선다(Chrome — 루프가 돌아오면 `kWaitingToExitRunLoop` 를 먼저 본다 · TDC#2025-2035) |
| T39 | MP · MR · `LoopEnded(Ended \| Canceled)` / MV · `LoopEnded(Destroyed 밖)` | `lp = Over` · 되돌리기 | (T37 ② 와 같은 적기 — `hid` · F2: MV 의 `Ended` 도 · 그 뒤 §6-2 ④(O 없음)면 탭이 숨은 끄는 창에 남고 공통 마무리 3 이 제자리에서 다시 보인다) · [되돌리기] · `end_by = loop_canceled \| esc \| abandoned \| key`. ★MP · MR 의 `Ended` 는 일어나지 않는다★ — `EndMove` 를 내는 전이는 늘 pending 을 세운다(debug 단언 · release 는 되돌리기) |
| T40 | M* · `LoopEnded(Destroyed)` · `WindowGone(끄는 창)` | `lp = Over` · 끝 | [끝](§6-5) · `outcome = aborted` · `end_by = holder_gone` · Detached 면 `spare = Lost`(공통 마무리가 `SpareFill`) — 둘 중 먼저 온 쪽이 끝내고 다른 쪽은 세션이 없어 버린다 |
| T41 | M* · `WorldChanged` | 그대로 | T29 의 `Active` 더하기 · pending 이 없으면(MP · MR) 마지막 커서로 다시 판정해 안내만 다시 그림(R4 · R13c — 붙기는 `Moved` 에서만) |
| T42 | MP · MR · `RaiseDue` | 그대로 | 커서 밑 우리 창이 탭을 받을 수 있으면 `RaiseNoActivate(그 창)` · `RaiseNoActivate(끄는 창)`(조사 E6) |
| T43 | MP · `Failed(Start)` | `lp = Over` · 되돌리기 | 고리가 서지 않았다(D1 — 그 묶음의 남은 할 일은 돌지 않았다 · 답은 관문 앞에서 이미 냈다): Detached = 탭이 든 예비를 되돌린다(§6-2 ① — 결과의 빈 예비는 T16 이 `SpareGiveBack`) / WholeWindow = `RestorePlacement(O, 시작 자리 · 최대화)` 단계 하나로 되돌리고 끝 · `end_by = failed` · `reason`(`Unsupported` — Windows 밖 — 도 이 칸 · §8) |
| T44 | S* · M* · `Failed(Soft{op, label})`(I · W 밖 — W 열은 ⊥: 무른 실패는 `cont` 가 빈 뒤에만 처리된다 · I 는 x7 · F4) | 그대로 | `errors` 셈(끝 줄) · 할 일마다: `Hide(l)` → `hidden` 에서 l 을 뺌(숨지 않았다) · `PollStart` → SS · SSr 이면 [끝 — 탭바 안](지금 모델 자리 · `end_by = failed` — 뗌을 볼 길이 없다) · `ArmRelease` → SPr · SSr 이면 T31 과 같은 끝 · 정리 할 일(ⓞ — `SpareGiveBack` · `CloseWindow` · 다시 보이기) → 셈만(실행기가 이미 그 창을 명부의 고아로 넘겼다 — F1) · 그 밖은 셈만 |

- **세션 번호 · token · 고리 번호 · 시한 seq** — 세션 번호는 앱 실행마다 1 부터 단조로 늘고 0 은 「없음」이다(F6 a — 웹뷰의 `lastEnded` 비교가 이것에 기댄다). 웹뷰의 `tab_drag_*` 은 `begin` 이 돌려준(또는 `dock:drag` · `dock:feed{Attach}` 로 받은) `session` 과 `seq` · `mount` 를, `Abandon` 은 begin 처리기면 `token` 을 · leave 처리기면 leave 의 `session` 을(F5), platform 알림은 `BeginMove` 가 실은 `loop_no` 를(알림 닫개가 쥔다), 폴러는 `gen` 을, 시한 사건은 낸 때의 seq 를 싣는다(받기 거르개 1).
- **세계 `World`** — 레이아웃 락 안에서 뜬 스냅숏(창별 탭 · 활성 · 보이는 탭 배치 · 캔버스 · 크롬 · 창 속성)과 그 창들의 OS 기하(바깥 · 안쪽 원점 · 배율 — 실행기가 메인 스레드에서 읽음) · 모니터 목록(작업 영역 · 배율) · 대기 창 명부의 재기(U4). `WorldChanged` 와 `Wrote` 앞에 다시 뜬다. 끄는 창의 기하는 그와 따로 실행기가 platform · 창 사건마다 그 사건을 줄에서 꺼낼 때 읽어 `geom` 으로 싣는다(§5-4 · F3 c). 5.8 의 `snap.rs` 를 고쳐 쓴다(§3).
- **시험(수렴 조건)** — U5a 는 행렬 522 칸을 표 시험으로 돈다: 상태 꼴마다 대표 세션 · 사건 꼴마다 대표 사건(칸 안 조건 갈래마다 하나 더)을 지어 `cell` 이 행렬의 이름을 돌려주는지 · 전이마다 다음 상태와 할 일(순서 포함)을 · ⊥ 칸은 debug 단언을 단언한다. 행렬의 정본은 이 문서이고 시험 표는 그것을 옮긴 것이다 — 칸을 고치면 둘을 같은 변경에서 고친다(§11).

#### 2-3-5. 대조 — 리뷰 1 · 2 · 3 라운드의 시나리오가 지나는 칸

| 시나리오 | 지나는 칸 |
|---|---|
| A1 · B1 붙기 쓰기 실패 | MR `Moved(W)` → T36(`Hide` · `EndMove`) → MA `LoopEnded` → T38 → Wa `Wrote(Err)` → T11 → 공통 마무리 3 이 끄는 창을 다시 보임 |
| A2 쓰기 결과에 기대는 할 일 | 붙기 · 떼기 · 띠 · 되돌리기의 할 일은 전부 `Wrote` 칸(T07 · T10 · T14 · T16)에 있다 |
| A3 · B5 · D5 날아가는 순서 이동 · 다른 출처 끝 | SS 의 대상 웹뷰 pointerup(높은 seq) = ⊘(출처 — `seen` 을 올리지 않는다) → 앞서 보낸 `Reorder` 가 T18 |
| B3 보냈으나 안 들어간 고리에서 붙기 | Wd → T07(MP) → 배치가 낸 `Moved` → T36(`EndMove` · MA) → platform 이 도착한 `SC_MOVE` 를 삼키고 `Ended` → T38 |
| B4 고리 안 끄는 창 소멸 | M* `LoopEnded(Destroyed)` 또는 `WindowGone` → T40 |
| B6 begin 시한 뒤 늦은 처리 | Wb → T02(답은 받는 쪽이 없어 무른 실패 → T44) → SP `Abandon` → T24 되돌리기 |
| D1 떼기 뒤 고리 시작 실패 | Wd → T07 → `PlaceDetached` · `BeginMove` 실패 → MP `Failed(Start)` → T43 → Wv … T16(빈 예비 `SpareGiveBack`) → 끝(`reverted`) |
| D2 들기 전 Esc | MP · platform 훅이 Esc 를 봄(숨김) → 도착한 `SC_MOVE` 삼킴 · `Canceled{esc: true}` · `hid` → T39(`hidden` 적음) → … T16(빈 예비 `SpareGiveBack` · `hidden` 에서 뺌) |
| D3 붙기를 정한 뒤 `Abandon` | MA `Abandon` → T34(MV) → `LoopEnded` → T39 |
| D4 들기 전 놓기(창째) | Wb → T03(MP) → `LoopEnded(Released{true})` → T37 ①(`geom` → `Provisional`) ③ 판정 |
| E1 Enter · 잠금 · UAC 로 끝난 떼어진 고리 | MR `Canceled{esc: false}` → T39 → Wv `Wrote(Ok(out.emptied = 예비))` → T16(`SpareGiveBack` · `spare = None`) → 끝(`SpareFill` 없음 — G11) |
| E6 다시 읽힌 창 | S* `PageLoaded(holder)` → T25 / 다른 창 → T26(`seen` 지움) → 그 웹뷰가 `window_role` 로 세션을 다시 받음 |
| E7 Strip{Shell} 놓기 | SS `Pointer{down: false}` → T23(SSr) → 날아가던 `Reorder` → T18 → `End{Drop, Release, at}` → T22 → Ws → T13 |
| C8 · E11 붙었던 창 W 에서 Esc | SS `End{Cancel, Key}` → T24 → 되돌리기(§6-2 — W 의 되살림 자리는 끄는 탭을 뺀 목록 기준) |
| 메인 마지막 탭 · 메인 탭바로 돌아옴 | Wa(W = 메인) → T10(… `Write(CloseTab)`) → Wc → T12 |
| F1 창째 붙기 뒤 남긴 창 닫기 실패 | MR `Moved(W)` → T36(`Hide(O)`) → MA `LoopEnded` → T38 → Wa `Wrote(Ok)` → T10(`emptied` 에 O · `Park(O)`) → … 놓기 → 공통 마무리 5 `CloseWindow(O)` ⓞ — 실행기가 O 를 명부 `Orphan{Close}` 로 바꾼 뒤 `close` → 실패 → 결과 블록의 `Failed(Soft)` 가 I 에서 x7 → 명부가 1 초 뒤(`OrphanDue`) `destroy` → `Destroyed` 갈래가 항목을 지움 |
| F1 붙기의 예비 되돌리기 실패 | Wa `Wrote(Ok)` → T10(`SpareGiveBack(예비)` ⓞ 실패 → 실행기가 예비를 명부 `Orphan{GiveBack}` 으로 · 세션은 `spare = None`) → SS `Failed(Soft)` → T44(셈만) → 다시 zone 밖 → T20(`NotReady` — 고아는 준비된 예비가 아니다) → `OrphanDue` 에 되돌리기 다시 → 되면 `Spare` → 다음 청에서 T19 / 또 안 되면 `Close` 사다리 + `respawn`(사라지면 새 예비) |
| F2 키 놓기를 정한 뒤 들어가기 전 Esc | MP `End{Drop, Key}` → T32(MK · `EndMove` — 보냄 상태) → 훅이 Esc 를 봐 숨김 → 도착한 `SC_MOVE` 삼킴 · `Ended`(끝냄 청이 앞선다) · `hid` → T37 ②(`hidden` 적음) ③ 판정 → 공통 마무리 3 이 제자리에서 다시 보임(F7 b) |
| F2 되돌리기 중 숨김 · O 없음 | MV `LoopEnded(Ended)` · `hid` → T39(`hidden` 적음) → §6-2 ④(쓰기 없음 — 탭은 숨은 끄는 창에) → 공통 마무리 3 이 다시 보임 |
| F2 Esc 뗌 뒤 놓기 | MR · 훅이 Esc 뗌(또는 반복)을 봐 숨김 · 루프는 이어 돎 → 버튼 뗌 → `Released{false}` · `hid` → T37 ②(`hidden` 적음) ③ 판정 → 그 밖이면 공통 마무리 3 이 다시 보임 |
| F5 붙은 창의 leave 시한 | SS `Leave` → 메인이 1 초 안에 답하지 못함 → 처리기가 `Abandon{by: Session(s)}` → 늦게 처리된 leave 가 T19 → Wd → T07(MP) 이면 그 `Abandon` → T33 → MV → `LoopEnded` → T39 / leave 가 T20 이었으면 SS `Abandon` → T24 |
| F6 다시 읽힌 창의 옛 페이지 사건 | S* `PageLoaded(그 창)` → T26(`seen` · `mounts` 지움) → 옛 페이지의 늦은 `End{_, Key}`(옛 `mount`) → 받음(그 칸의 전이 — 사용자가 낸 키) / 새 페이지의 `End`(새 `mount` · seq 1) → `mount` 가 달라 `seen` 0 으로 쳐 받음 |

### 2-4. 사건 출처 · 스레드 · 락

- **세션은 메인 스레드에서만 돈다.** invoke 처리기(async · tokio)는 사건을 `run_on_main_thread` 로 넘기고 돌아온다 — `tab_drag_begin` · `tab_drag_leave` 만 메인 스레드의 답을 기다린다(oneshot · 시한 1 초 — G18 · 둘 다 루프가 돌지 않는 때다: begin = Idle · leave = Strip). **시한을 넘기면 처리기가 `Abandon` 을 메인 줄에 넣고 웹뷰에 오류로 답한다** — begin 처리기 = `Abandon{by: Token(token)}`(아직 세션 번호가 없다) · leave 처리기 = `Abandon{by: Session(session)}`(leave 가 실은 번호 — 붙은 창 웹뷰는 begin 의 `token` 을 모르므로 6.2 의 `Abandon{token}` 은 leave 시한에서 지을 수 없었다 · F5) — 웹뷰는 그 끌기를 버리고(포인터 캡처를 놓는다) 아무것도 더 보내지 않는다. 메인 스레드가 늦게라도 그 begin · leave 를 처리했으면 `Abandon` 이 그 세션을 되돌린다(B6). 폴러 · 시한 스레드도 같은 길로 넘긴다. 창 사건(`Moved` · `Destroyed`) · `on_page_load` · platform 루프 알림은 이미 메인 스레드다.
- **줄 순서의 근거(코드 확인)** — 다른 스레드의 `run_on_main_thread` 는 tao 사용자 사건 채널(mpsc)로 보낸 순서대로 메인에 닿는다(tauri-runtime-wry 2.11.3 `send_user_message` → tao 0.35.3 `EventLoopProxy::send_event`). 그래서 같은 처리기가 begin · leave 를 보낸 뒤 넣은 `Abandon` 은 늘 그 사건 뒤에 처리된다 — Idle 에 먼저 닿는 `Abandon` 은 없다. ★메인 스레드에서 부른 `run_on_main_thread` 는 채널을 거치지 않고 그 자리에서 돈다★(같은 함수 — 메인 스레드면 곧바로 처리) — 그래서 메인 스레드의 사건 원천(창 사건 · platform 알림 · 실행기 자신)은 실행기 줄에 넣기만 하고 `step` 을 직접 부르지 않는다.
- **폴러 · 시한 = 앱 수명 스레드 하나씩**(setup 에서 띄움) — 세션의 `PollButton` · `PollStart` · `PollStop` · `ArmRaise` · `ArmRelease` 는 그 스레드에 보내는 명령이다. 폴러 사건은 `gen` 을, 시한 사건은 seq 를 실어 늦은 것을 받기 거르개가 버린다. 실행기도 명부 고아의 다시 하기 시한 `ArmOrphan` 을 같은 시한 스레드에 보낸다 — 세션 할 일이 아니고, 돌아온 `OrphanDue` 는 실행기가 `step` 이 아니라 명부로 보낸다(§2-5 「고아」 · F1).
- **한 번에 하나씩 끝까지(run-to-completion)** — 실행기는 메인 스레드 지역 줄을 갖는다. 할 일을 돌리는 도중에 같은 스레드로 다시 들어온 사건(예: `PlaceWindow` 가 보내는 `WM_MOVE` → `Moved`)은 줄 뒤에 넣고, 지금 묶음이 끝난 뒤 처리한다. 그래서 세션에는 락이 없다(메인 스레드 하나).
- **레이아웃 쓰기의 결과(A2)** — `Write` 는 묶음의 마지막 할 일이다(`step` 이 단언). 실행기는 앞 할 일들을 돌린 뒤 `Write` 를 메인 스레드에서 동기로 돌리고(`LayoutPort` — 레이아웃 락 한 번 · §7), 세계를 다시 떠, 결과 `Wrote` 를 줄 **맨 앞**에 넣는다. 그래서 `Write` 다음에 처리되는 사건은 늘 그 결과이고 그 사이에 다른 사건이 끼지 않는다 — 세션은 `cont` 하나로 이어 가고, 결과에 기대는 할 일(떼기의 창 놓기 · 루프 시작, 붙기의 비운 창 대기 · `main_refilled` 적기 · 폴러 시작)은 전부 그 결과를 본 다음 전이에 있다. 쓰기 안에서 나온 알림(`settled` → `WorldChanged`)은 줄 뒤에 선다.
- **할 일 실패(D1)** — 실패할 수 있는 할 일은 전부 실패를 사건으로 `step` 에 되돌린다. 등급은 셋이다(§2-3 할 일 표):
  - **관문**(`Take` = `SpareTake` · `Start` = `PlaceDetached` · `BeginMove` · 창째 끌기 시작의 `PlaceWindow` · `RestorePlacement`) — 뒤 할 일이 그것에 기댄다. 실패하면 실행기는 **그 묶음의 남은 할 일(`Write` 포함)을 돌리지 않는다**. 관문은 T03 · T07 · T19 의 묶음에만 있다. T03 · T07 은 답(`Reply`)을 관문 앞에 두고, T19 의 leave 답은 그 결과 칸(T07 · T08 · T09)이 낸다 — 관문이 실패해도 답하지 않은 invoke 가 남지 않는다.
  - **무름**(그 밖 — 창 숨기기 · 대기 · emit · 답 · 표식 내리기 · 서비스 스레드 명령 …) — 실패해도 묶음은 이어 간다. 세션은 그 실패를 끝 줄 `errors` 에 세고, 상태가 그 할 일에 기대는 셋(`Hide` · `PollStart` · `ArmRelease`)만 T44 가 따로 다룬다.
  - **소유 넘김 ⓞ(무름의 한 갈래 — F1)** — 정리 할 일(`CloseWindow` · `SpareGiveBack` · 공통 마무리 3 의 다시 보이기 `PlaceWindow{rect: None, show: true}`)은 세션이 그 할 일을 내는 때 창을 놓는다(`spare` · `hidden` · `emptied` 에서 뺌 — 결과를 기다려 이어 쥐면 끝 묶음에서는 쥘 세션이 없다). 그래서 실행기는 그 할 일이 실패하면 **그 `Failed` 를 줄에 넣기 전에, 같은 메인 스레드 걸음에서** 그 창을 명부의 고아로 넘긴다(명부 락 한 번 — §2-5 「고아」). 세션이 그 실패를 보는 때(`cont` 가 빈 뒤 T44 · 또는 세션이 이미 끝나 x7)에는 창이 이미 명부에 있다 — 아무도 쥐지 않은 창이 생기는 틈이 없다. `CloseWindow` 는 실패와 무관하게 늘 넘긴다 — `close()` 의 답은 닫기 요청이 나갔다는 것뿐이고 닫기의 성공은 `Destroyed` 다(명부의 소멸 갈래가 항목을 지운다). 6.2 는 공통 마무리가 `CloseWindow` 와 함께 명부에서 빼고 세션이 끝나, 닫기가 실패한 남긴 창이 화면 밖에 영영 남았다(리뷰 3 F1).
  - **실패 없음**(`EndMove` — 결과는 `LoopEnded` · `Log`).
- **결과 블록** — 묶음이 끝나면(다 돌았거나 관문에서 멈춤) 실행기는 **[그 묶음의 `Wrote` 또는 관문 `Failed` 하나] + [그 묶음의 무른 `Failed` 들]** 을 한 덩어리로 줄 **맨 앞**에 넣는다(한 묶음에 둘이 함께 있을 수 없다 — `Write` 는 마지막이고 관문에서 멈추면 돌지 않는다). 그 결과를 처리한 전이가 새 `Write` 를 내면 새 결과 블록이 다시 맨 앞으로 와 앞 묶음의 무른 실패보다 먼저 처리된다. 그래서 `cont` 가 선 동안 다음 사건은 늘 그 `Wrote` 이거나 같은 묶음의 관문 실패이고(행렬의 W 열), 무른 실패는 `cont` 가 빈 뒤에만 처리된다 — W 열 × `Failed(Soft)` 는 ⊥ 다(F4 — 6.2 는 그 여덟 칸을 T44 로 두어, T44 가 거기서 [끝]을 내면 `cont` 가 선 채 끝날 수 있었다). 줄 뒤에 선 사건(다른 스레드 · 재진입 · `OrphanDue`)도 결과 블록 사이에 끼지 않는다 — `cont` 가 빈 때만 처리된다.
- **웹뷰 사건 순서(A3 · B5)** — invoke 처리기는 tokio 에서 따로 돌아 메인 줄 도착 순서가 웹뷰가 보낸 순서와 다를 수 있다. WebView2 가 겹친 요청을 보낸 순서대로 올리는지도 문서에 없다(`src/components/layout/windowCanvasReport.ts` 머리 주석 — 그래서 그 보고기는 한 번에 하나만 보낸다). 끌기 사건은 보고기처럼 한 번에 하나로 묶지 않고 §4 「순서」의 `seq` 를 실어 받기 거르개가 낡은 것을 버린다(메인 결정 — 리뷰 1 라운드). 사건마다 절대값(결과 자리 · 끝 · 최종 자리 `at`)을 실어, 낡은 것을 버려도 잃는 것이 없다.
- **이동 루프 동안 메인 스레드는 루프 안에서 메시지를 나른다** — 메시지 펌프에서 시작한 루프에서 `run_on_main_thread` · emit · `WindowEvent::Moved` · IPC 가 흐른다(스파이크 S3 — p50 1.1–4.6 ms · 표본 1–2 회 → §12 M2 에서 다시 잰다).
- **레이아웃 락** — `Write` 는 메인 스레드에서 동기로 돈다. 세션 포트(`LayoutPort`)의 쓰기 갈래는 `Seam` · `TabBar` · `AdoptWindow` 뿐이다 — 셋 다 OS 창을 만들지 않으므로 메인 스레드에서 돌려도 된다. 창을 여는 `NewWindow` 는 포트 타입에 없다(B9 · §7 — 5.8 이 「`Apply` 는 async 에서」라 둔 까닭 = 새 창 열기). 레이아웃 락은 쓰기 한 번 안에서만 쥐고 놓는다 — 세션 사건을 처리하는 동안 다른 락을 쥔 채 OS 호출을 하지 않는다(`apply.rs` 「락 규율」 그대로).
- **창 만들기는 메인 스레드에서 하지 않는다** — 예비 창 · 안내 창은 작업 스레드에서 짓는다(스파이크 — 동기 IPC 처리기에서 지으면 Windows 에서 교착).
- **포인터 캡처를 쥔 창은 끄는 동안 숨기지 않는다**(스파이크 S2) — 사라져야 하는 창(비운 출처)은 루프가 그 창 캡처를 이미 풀어 놓은 뒤에만 숨기거나 화면 밖으로 옮긴다(§2-5 · T07 의 `Park` 는 `BeginMove` 뒤).

### 2-5. 대기 창 명부 · 예비 창 · 안내 창

**대기 창 명부(B2 · B7)** — `src-tauri/src/dock/parked.rs`

- **무엇** — 모델 밖에서 기다리는 우리 창의 목록이다: label → `{ 역할: Spare | Kept | Orphan{…}, 재기: FrameMeasures{canvas, metrics, chrome} }`. 예비 창(떼어 낼 창)과 끄는 중 비워 남긴 창(R10), 그리고 정리하다 실패한 창(고아 — 아래 · F1)이 든다. 따로 된 작은 목록 `reveal` 은 모델 안에서 숨은 채 남은 창이다(아래 「고아」). 안내 창은 들지 않는다(팝아웃 label 이 아니고 재기도 없다).
- **재기 받기** — `report_window_canvas` · `report_ui_metrics` · `report_dock_chrome` 처리기는 label 이 명부에 있으면 모델 대신 명부의 재기에 적는다. 지금은 모델에 없는 label 을 `WindowNotFound` 로 거절해(`layout/manager.rs` `set_window_canvas` · `set_ui_metrics` · `set_dock_chrome`) 예비 창의 재기가 사라지고, 첫 떼기의 `original`(§5-4) · `detached_origin`(§5-5) 입력이 없었다. **「예비 창 준비됨」 = 지어졌고 명부 역할이 `Spare` 이고 재기 셋이 다 있음**(고아 `Orphan{GiveBack}` 은 준비된 예비가 아니다 — F1).
- **들이기** — `AdoptWindow` 는 명부의 재기를 싣고, 모델은 같은 락에서 새 창 항목의 `canvas` · `metrics` · `chrome` 에 옮겨 담는다(§7 ②). 웹뷰의 보고기는 같은 값을 다시 보내지 않으므로(「같은 값 안 보냄」) 옮겨 담지 않으면 들인 창의 캔버스 · 크롬이 다음 바뀜까지 없다.
- **내보내기** — 모델이 그 창을 비워 빼면(`OnEmptied::Keep`) 빼기 직전의 재기를 `EmptiedWindow.measures` 로 돌려주고 세션이 그것으로 명부에 다시 적는다(`SpareGiveBack` · `Park`).
- **빼는 곳** — 명부의 창은 ① 트레이 보이기 · 숨기기 · `--hidden` 자동 시작에서 빠진다(`tray/actions.rs` 가 `tray::core::ui_windows` 에 넘기는 술어 = `is_popup_label(l) && !parked(l)` — 지금은 `is_popup_label` 만이라 보이기가 예비 창에 `show` · `unminimize` · `set_focus` 를 한다) ② 작업 표시줄 · Alt+Tab 에서 빠진다(tao `skip_taskbar` + platform `set_switcher_hidden` — §8) ③ 자리 기록(`placement::record`)을 건너뛴다. 닫기 요청 갈래(`lib.rs` 팝아웃 닫기 요청의 모델 지우기 · 묘비)는 ★건드리지 않는다★(6.1 의 ④ 를 걷음 — 리뷰 2 라운드) — 그 갈래는 닫기를 막지 않으므로 그 창은 어차피 사라지고, label 은 다시 쓰지 않으므로(`PopupCounter`) 죽는 label 에 묘비가 남아도 무해하다. 남는 것은 사라진 뒤의 길이다: **사라지면**(`Destroyed`) 명부(항목 · `reveal`)에서 빼고, 예비 창이었으면(역할 `Spare` · `respawn` 이 달린 고아) 명부에 예비(짓는 중 포함)가 없을 때만 작업 스레드에서 새로 짓고, 그 뒤 세션에 `WindowGone(label)` 을 넣는다(남긴 창이면 T28 — 되돌리기가 그 창을 「O 없음」으로 본다 · §6-5 · 고아면 x4). 닫는 창의 명부 항목을 지우는 곳은 **이 갈래뿐이다** — `CloseWindow` 는 항목을 지우지 않고 고아로 바꾼다(닫기의 성공은 소멸이다 · F1). 명부에서 빼는 다른 길은 들이기(`Unpark`)와 고아의 제자리 다시 보이기 성공뿐이다. 고아(`Orphan{Close | GiveBack}`)도 대기 창처럼 ① ② ③ 에서 빠진다(0탭 창이 트레이 · 작업 표시줄 · 자리 기록에 나오지 않게). `reveal` 의 창은 모델 창이라 빠지지 않는다.
- **대기 모드 = 0탭 자가닫힘 끔** — 명부의 창(예비 · 남긴 창) 웹뷰는 `dock:spare{Parked}`(또는 마운트 때 `window_role` = `Parked`)를 받으면 `WindowLayout` 의 「0탭 자가닫힘」(HEAD `WindowLayout.tsx` — 탭 0 개 `window:tabs-updated` 를 받으면 제 창을 닫는 방어 갈래)을 끈다. 모델도 `OnEmptied::Keep` 으로 비운 창에 탭 0 개 사건을 보내지 않는다(§7 ⑤ — 지금 `WindowClosed` 갈래가 보내지 않는 것과 같게). 둘 중 하나만 있어도 남긴 창 · 되돌린 예비가 스스로 닫히지 않지만 둘 다 둔다(보내는 쪽 · 받는 쪽 순서가 정해지지 않았다).
- **★tao 깃발 불변식(E2 — 코드 확인)★** — tao 는 창 깃발이 하나라도 바뀌면 `GWL_EXSTYLE` 을 자기 깃발로 통째 다시 쓴다(tao 0.35.3 `window_state.rs` `apply_diff` · 주인 없는 창은 `WS_EX_APPWINDOW` 가 늘 켜짐 — `window.rs` 의 `Parent::None` → `ON_TASKBAR`). 그 setter 는 `set_visible`(Tauri `show` · `hide`) · `set_ignore_cursor_events` · `set_always_on_top` · `set_maximized` · `set_minimized` · `set_resizable` · `set_decorations` · `set_focusable` 등이다. 그래서 **Alt+Tab 에서 뺀 창(명부의 창 · 안내 창)에는 뺀 뒤 tao · Tauri 창 깃발 setter 를 부르지 않는다** — 보이기 · 숨기기 · 자리 · z 는 전부 platform 함수(날 Win32)로 한다. 짓는 때는 tao 쪽을 먼저 끝내고(`show` · `set_ignore_cursor_events(true)` · 항상 위) 그 뒤에 `set_switcher_hidden(true)` 를 부른다. tao `set_skip_taskbar` 는 깃발을 바꾸지 않고(`ITaskbarList` 만) 탐색기가 다시 뜨면 tao 가 다시 건다(`S_U_TASKBAR_RESTART` — 코드 확인). U4 는 명부 창을 만지는 tao · Tauri 호출 자리를 `rg` 로 확인한다(트레이 술어 · 자리 기록 · 테마 · 항상 위 설정 등).
- **★label 접두가 아니라 명부로 가르는 까닭(B7 — 고른 쪽)★** — 예비 창은 들이는 순간 보통 팝아웃이 되는데 Tauri 창 label 은 바꿀 수 없다. 팝아웃 접두(`slot-popup-`)는 권한 glob(`capabilities/popup.json`) · 닫기 요청 · 소멸 정리(`is_popup_label`)가 함께 쓰므로 예비 창에 다른 접두를 달면 들인 뒤 그 셋이 어긋나고, 남긴 창(R10)은 원래 팝아웃 label 그대로라 접두로는 아예 가를 수 없다. 「대기 중인가」는 label 의 성질이 아니라 지금 상태라 셸이 명부로 적는다.
- **★고아 — 정리하다 실패한 창(F1)★** — 세션이 놓은 창(정리 할 일 ⓞ 의 대상 — §2-3 할 일 표)의 정리가 끝나지 않으면 명부가 이어 쥐고 세션과 따로 끝까지 정리한다. 새 장치가 아니라 명부의 역할 하나 · 목록 하나 · 실행기 줄의 시한 하나다.
  - **항목** — 역할 `Orphan{want: Close | GiveBack, step: First | Escalated | GaveUp, respawn: bool}`(모델 밖 창) · 목록 `reveal`(모델 안에서 숨은 채 남은 창 — 공통 마무리 3 의 다시 보이기가 실패한 창. 탭이 든 모델 창이라 위 술어 ① ② ③ · 재기 받기 갈래에 들지 않는다).
  - **넘기는 때** — 실행기가 정리 할 일을 돌린 같은 메인 스레드 걸음, 그 `Failed` 를 줄에 넣기 전(§2-4 「소유 넘김」 · 명부 락 한 번): `SpareGiveBack` 의 단계 하나라도 실패 → `Orphan{GiveBack}`(이미 `Spare` 로 적었으면 역할을 바꾼다 — 준비된 예비로 보이지 않게) · `CloseWindow` → 성공 · 실패와 무관하게 `Orphan{Close}` 로 둔 뒤 `close()`(명부에 없던 창 — 띠 놓기로 비운 창째 끄는 창처럼 대기에 든 적 없는 창 — 이면 새 항목 · 재기는 비움) · 다시 보이기 실패 → `reveal` 에 더함. 넘긴 뒤 시한 스레드에 `ArmOrphan` 을 보낸다 — 간격은 G18 의 1 초를 그대로 쓴다(새 수를 짓지 않는다).
  - **다시 하기** — `OrphanDue` 가 메인 줄에 오면(세션 사건이 아니다 — 실행기가 명부로 보내고 `step` 에 닿지 않는다 · 줄 뒤에 서므로 `cont` 가 빈 때만 처리된다 · §2-4) 실행기가 항목마다 순수 함수 `parked::orphan_next(항목, 모델에 있나)` 가 고른 할 일을 포트로 돌린다: **모델에 있으면**(탭이 든 창 — 생겨서는 안 되지만 막는다) 닫거나 되돌리지 않고 항목을 `reveal` 로 옮겨 곧바로 다시 보인다 · **`Close`** — 아직 살아 있으면(소멸을 못 봄) `destroy()`(닫기 요청을 거치지 않는 Tauri 강제 소멸 — `Escalated`) · **`GiveBack`** — `SpareGiveBack` 단계를 다시 한다 → 되면 역할 `Spare`(재기가 다 있으면 준비됨) · 안 되면 `Orphan{Close, respawn: true}` 로 내리고 곧바로 `close()` · **`reveal`** — 제자리 다시 보이기(`place_window(rect: None, show)` — §8) → 되면 목록에서 뺀다. 한 단 올린 할 일(`destroy` · 두 번째 다시 보이기)까지 실패하면 error 한 줄 · `GaveUp` 으로 두고 시한을 다시 걸지 않는다 — 실행기가 세션을 끝낸 묶음 뒤마다 한 번 더 본다(되풀이 시한 없음 · 바쁜 돌기 없음). 남은 고아가 있는 동안만 시한을 건다.
  - **끝** — 항목은 위 「사라지면」 갈래가 지운다(닫기 · 강제 소멸의 성공 = 소멸). 창이 이미 없으면(Tauri 창 조회 실패) 소멸을 기다리지 않고 지운다. `respawn` 이 달린 고아(예비였던 창)가 사라지면 명부에 예비가 없을 때만 새 예비를 짓는다 — 예비는 늘 하나(G11 · 다시 하기가 성공해 `Spare` 로 돌아온 창과 새로 지은 창이 겹치지 않게 새 예비는 소멸 뒤에만 짓는다).
  - **세션과 겹치지 않는다(구성)** — 고아는 세션이 이미 놓은 창이고, 세션이 명부의 창을 다시 쥐는 길은 `SpareTake`(역할 `Spare` 만 — 고아면 T20 `NotReady`)뿐이다. `reveal` 의 창은 숨어 있어 사용자가 그 탭바를 누를 수 없고 커서 밑 창 찾기도 숨은 창을 건너뛴다(§8 `top_level_at`). 그래서 다시 하기는 세션이 쥔 창을 건드리지 않는다.
  - **왜 트레이로 되살릴 수 없나** — `Hide` · Esc 훅의 숨김은 날 `SW_HIDE` 라 tao 의 VISIBLE 깃발은 참으로 남는다 → 트레이 보이기(`show`)는 깃발이 바뀌지 않아 아무것도 하지 않는다(tao 0.35.3 `window_state.rs` `apply_diff` — 바뀐 깃발이 없으면 돌아간다 · 리뷰 3 F2). 숨긴 창을 다시 보이는 길은 platform 함수뿐이라 명부가 쥔다. 공통 마무리 뒤 tao 보임과 OS 보임을 대조하는 단언이 이 어긋남을 잡는다(§6-1 · F7 c).

**예비 창(떼어 낼 창)**

- **하나를 미리 만든다** — 보통 팝아웃과 같은 빌더(`commands/popout.rs` `build_window` → `WebviewEnv::finish`)로 label `slot-popup-N`(`PopupCounter`)을 받아 짓되 **모델에 넣지 않고, 짓기 전에 명부에 `Spare` 로 적는다**(짓는 도중의 트레이 · 자동 시작도 그 창을 보지 않게). URL 은 보통 팝아웃과 같다(`#/popup?window=<label>` — 예비 표시를 URL 에 싣지 않는다). 웹뷰는 마운트 때 `window_role` 을 물어 `Parked` 면 **대기 모드**(빈 틀 = 탭바 + 빈 캔버스 · 자가닫힘 · 끌어오기 재시도 없음 · 재기 보고는 그대로)로 그리고, `dock:spare{Parked | Adopted}` 로 모드를 바꾼다. 6판의 URL 표시(`&spare=1`)는 들인 뒤에도 남아 그 창을 대기 모드에 가뒀다(B7).
- **기다리는 자리** = 화면 밖(주 모니터 가까이 — 배율이 다른 모니터 옆에 두면 그 배율로 그려졌다가 보일 때 `WM_DPICHANGED` 가 난다: 스파이크 `park_origin` 주석) · 보이는 상태(스파이크 C 그대로) · 트레이 · 작업 표시줄 · Alt+Tab 밖(G2 결정 — 위 명부).
- **떼면** 모델에 들이고(`AdoptWindow` + 재기) → `Unpark`(명부에서 빼고 작업 표시줄 · Alt+Tab 에 되돌림 — 아직 화면 밖) → 자리 · 크기 · 활성을 한 번의 `SetWindowPos` 로 입힌다(`PlaceDetached` · §8 `place_window`) → 이동 루프. Alt+Tab 빼기 비트(`WS_EX_TOOLWINDOW`)는 틀 두께(제목 표시줄 높이)를 바꾸므로 떼기 자리는 `Unpark` 뒤에 읽은 틀 두께로 셈하고(§5-5 · M1), 대기 중에 잰 캔버스 재기는 그 틀 기준이라 낡는다 — 첫 떼기의 `original` 은 캔버스 재기가 아니라 놓은 뒤 OS 안쪽 크기에서 셈한다(§5-4 · E2). 커서 모니터의 배율이 대기 자리와 다르면 두 걸음으로 놓는다(§5-5 · E3).
- **다시 예비로** — 그 탭이 다른 탭바에 붙거나 칸 띠에 붙어 비면 모델에서 빠지고(`pull_tab` 의 기존 규칙 · `OnEmptied::Keep`) `SpareGiveBack` 으로 화면 밖 · 명부로 돌아가 `dock:spare{Parked}` 로 웹뷰를 빈 틀로 되돌린다. 같은 끌기에서 다시 떼면 그 창을 또 쓴다 — 한 끌기 안에서 그 탭은 한 곳에만 있으므로 예비 창은 늘 하나면 된다(G11).
- **끝에** 그 탭이 예비 창에 남았으면(새 창으로 놓음) 그 창은 보통 팝아웃이 되고(`dock:spare{Adopted}` 는 들일 때 이미 보냈다), 끌기가 끝난 뒤 작업 스레드에서 새 예비 창을 짓는다(G3 · 짓는 데 160–220 ms — 스파이크 S8 fresh).
- **예비 창이 아직 준비되지 않았을 때 떼기**(앞 끌기가 예비 창을 쓰고 새 것을 짓는 ~220 ms + 첫 재기 보고 안에 다음 끌기가 탭바 구역을 나감) — 세션은 `tab_drag_leave` 에 `NotReady` 로 답하고 Strip 에 머문다. 웹뷰는 탭바 안 몰기를 이어 가다(끄는 탭은 끌기 영역 끝에 잘려 선다 — §5-1) 구역 밖 움직임마다 다시 청한다. 준비되면 그다음 청에서 뗀다. 메인 스레드에서 창을 지을 수 없고(§2-4) 그 자리에서 지은 창은 깜빡이므로(S8 fresh) 짓지 않고 기다린다 — 끝 줄 `spare_wait_ms`. 앞 끌기 끝에 짓기를 바로 시작하므로 드물다.
- **예비 창을 잃으면**(누가 닫음 · 끄는 중 그 탭이 닫혀 창이 닫힘 — §6-5) — 명부가 `Destroyed` 를 보고 새로 짓는다.

**비운 출처 창(R10)**

- 탭 하나 창을 창째 끌다가 다른 탭바 · 칸 띠에 붙으면 그 창이 빈다(조사 A7 「비워진 창은 숨겼다가 끌기가 끝날 때 닫는다」). 모델에서는 빠지고(지금 규칙) OS 창은 남긴다 — 그 창 웹뷰가 쥐던 캡처는 이동 루프를 시작할 때 이미 풀렸다(S2 의 위험 밖). 붙는 순간은 `SW_HIDE`(스파이크 S4 의 순서) · 결과가 오면 `Park`(화면 밖에 보이는 채 · 명부 `Kept` · 작업 표시줄 · Alt+Tab · 트레이 밖) — 되돌릴 때 다시 나타나는 모습이 스파이크 S8 pool(화면 밖에서 보이던 창을 커서 밑으로)과 같아지게. 숨긴 창을 다시 보이는 모습은 재지 않았다.
- 그 창의 모델 항목(창 id · 속성)과 재기는 세션에 적어 두었다가(`EmptiedWindow{label, seed, measures}`) 되돌리기에서 `AdoptWindow{label, seed, measures}` 로 되살린다(§7). 끝에 쓰이지 않았으면 닫는다(`host.close` — 모델 뒤 · M2 와 같은 순서).
- 메인은 비지 않는다 — 빈 탭이 생긴다(R14 · §6-4).

**안내 창(R3a)**

- **하나를 미리 만든다** — label `dock-overlay` · 경로 `#/dock-overlay` · 투명 · 테두리 없음 · 그림자 없음 · 크기 고정 · 활성화 안 함(`focused(false)`) · 항상 위 · 작업 표시줄 · Alt+Tab 에서 뺌(`skip_taskbar` + `set_switcher_hidden` — 테두리가 없어 그 비트가 틀을 바꾸지 않는다) · `set_ignore_cursor_events(true)`(스파이크 G 그대로). 화면 밖에서 보이는 채 기다린다. 팝아웃 label 이 아니므로 트레이 대상이 아니다.
- **그릴 때** 대상 창 안쪽 사각형을 덮게 날 `SetWindowPos(HWND_TOPMOST, …, SWP_NOACTIVATE | SWP_SHOWWINDOW)`(스파이크 S7 `g=move` 11/11 — Tauri `set_position` 은 z 순서를 올리지 않아 0/1). 내용(띠 · 미리보기 사각형)은 `dock:overlay` 로 민다 — 좌표 = 안내 창 CSS px(대상 창 CSS px × 대상 배율 ÷ 안내 창 배율). 치울 때 화면 밖으로.
- **배율이 다른 모니터로 옮길 때**(근거 없음 c · 미검) — 안내 창이 배율이 다른 모니터로 가면 OS 가 `WM_DPICHANGED` 와 제안 사각형을 보내고 tao 가 그 사각형으로 창 크기를 바꿀 수 있다. 실행기는 안내 창의 `WindowEvent::ScaleFactorChanged` 를 보면 마지막 `Overlay` 자리 · 내용을 한 번 더 놓는다(배율이 자리 잡은 뒤의 자리) — M10 에서 잰다.
- 판정에서 늘 뺀다(커서 밑 창 찾기의 `skip` — 조사 D).
- **짓는 순서**(E2) — tao 쪽(`set_ignore_cursor_events(true)` · 항상 위 · 화면 밖 자리 · `show`)을 다 한 뒤 `skip_taskbar` · `set_switcher_hidden(true)`. 그 뒤로는 platform `place_window` 만 부른다(위 「tao 깃발 불변식」).
- **사라지면** 작업 스레드에서 다시 짓는다(예비 창과 같은 길 — 그동안의 `Overlay` 할 일은 무른 실패로 세어진다).
- 새 창 권한: `src-tauri/capabilities/` 에 `dock-overlay` 창의 이벤트 듣기 권한을 더한다(§3).

## 3. 모듈 구조

점검 문서의 제안 구조(「제안 모듈 구조」)를 따르되 아래처럼 다듬었다 — 바뀐 곳은 「다듬음」 칸.

**Rust — `src-tauri/src/dock/`**(Tauri · OS 를 직접 부르지 않는다 — 포트로만)

| 파일 | 맡는 것 | 순수? · 시험 | 다듬음 |
|---|---|---|---|
| `session.rs` | 상태 기계 `accept` · `cell` · `step`(§2-3 — 행렬 칸 이름을 `cell` 이 돌려주고 `step` 은 그 이름으로만 가른다 · `_ =>` 갈래 없음) · `Session` · `Event` · `Effect` · `Cont` · `RevertStep` · 공통 마무리와 보상(§6-1) · 되돌리기 계획(§6-2) · 끝 로그 한 줄 만들기(`EndLog::of(&Session, reason)` — 칸을 따로 베끼지 않는다) | 순수 · 행렬 표 시험 | 점검의 `session.rs` 그대로 |
| `exec.rs` | 실행기 — 메인 스레드 줄(한 번에 하나씩 끝까지) · 할 일을 포트로 돌림 · 실패 등급(관문이면 묶음 멈춤) · 결과 블록을 줄 맨 앞에(§2-4) · 사건 보강(`Moved` · platform 알림에 커서 · 커서 밑 창 · 끄는 창 `geom` — `probe` · `geom` 은 줄에서 꺼낼 때 · `settled` 표 — F3) · 끄는 창 label · `loop_no` 로 platform · 창 사건 거르기 · `PlaceDetached` 두 걸음(그때 읽은 틀 두께 · 배율로 `sizing` — §5-5) · 배율 바뀜 뒤 다시 놓기(안내 창 · 막 뗀 창 — 계기 = `Posted` 동안의 첫 `ScaleFactorChanged` · F3) · 정리 할 일 실패의 고아 넘김 · `OrphanDue` 를 명부로(F1) · 공통 마무리 뒤 tao · OS 보임 대조(F7 c) | 포트만 의존 · 가짜 포트 시험 | 점검의 `exec.rs` + `follow.rs` 를 합쳤다 — 따라오는 창은 OS 이동 루프가 옮기므로 따로 몰 코드가 없다 |
| `units.rs` | `PhysPoint` · `PhysRect` · `PhysSize` · `MonoMs` 같은 단위 타입(E8 — 지금은 `book.rs` 에 있어 U8 에서 함께 지워진다 · 새 모듈은 이것을 쓴다) | 순수 | 새로(U2) |
| `ports.rs` | 포트 trait 다섯: `LayoutPort`(세계 뜨기 · 표식 · `dock(SessionDock)` · `switch_tab` · `close_tab` — `SessionDock = Seam \| TabBar \| AdoptWindow` 라 `NewWindow` 를 낼 길이 타입에 없다 · B9) · `WindowPort`(창 놓기 · 숨기기 · 대기 · 이동 루프 · 커서 밑 창 · 커서 · 주 버튼 · 모니터 · 창 기하 · 강제 소멸 — F1 · tao 보임과 OS 보임 — F7 c · 활성은 platform 으로만 — `set_focus` 갈래가 없다 · F7 a) · `EmitPort` · `SparePort`(꺼내기 · 되돌리기 · 채우기 · 닫기 · 준비됨 · 고아로 넘기기 — F1) · `ClockPort`(시한 · 폴러 · 고아 시한 `ArmOrphan` — F1) | trait 뿐 | 새로 |
| `parked.rs` | 대기 창 명부(§2-5) — label → 역할(`Spare` · `Kept` · `Orphan`) · 재기 · 「예비 창 준비됨」 판정 · 고아 다시 하기 고르기(`orphan_next`) · `reveal` 목록(F1) | 순수(명부 자료) · 표 시험 | 새로(B2 · B7) |
| `geometry.rs` | 도킹 띠(10% · 최소 두께) · 바깥 띠(32) · 유효 띠(앞선 영역 빼기) · 띠 고르기(가까운 변) · 붙을 사각형(붙는 몫으로) | 순수 · 표 시험 | `hit.rs` 의 도킹 몫을 옮겨 고침 |
| `strip.rs` | 탭바 구역(끌기 영역 · 위아래 ±15 — C3) · 화면 물리 점 → 창 CSS 점 | 순수 | `hit.rs` `reorder_zone` 을 옮김 |
| `sizing.rs` | 붙는 몫(원래 크기 · 칸 절반 상한) · 떼어진 창 크기(C5 순서) · 커서에 가장 가까운 모니터(C6) · 떼어진 창 자리 · 창째 끌기 자리(C4) · 원래 크기 셈 | 순수 | 새로 |
| `targets.rs` | 세계 + 커서 + 커서 밑 창 → `Target{Attach(W) \| Band(BandHit) \| Nothing}` · 안내 창 내용(`OverlayView` — 그 타입 정의 · ts-rs 도 여기 · E8: U2 가 쓰므로 `wire.rs`(U5a)에 두지 않는다) | 순수 | `regions.rs` 를 대신 — 보낸 영역 · `rev` · `bar_rev` · `bumps` · `Expect` 가 없다(그린 것과 판정이 같은 셸 함수라 맞춰 볼 것이 없다) |
| `world.rs` | 레이아웃 락 안에서 세계 뜨기(창별 탭 · 활성 · 보이는 탭 배치 · 캔버스 · 크롬 · 창 속성 · 표식) · 대기 창 명부의 재기는 U4 가 더한다(E8) | 순수(입력 = `&ViewManager` · 명부) | `snap.rs` 를 고침 — `Snap` 타입이 `book.rs` 에 있던 결합을 끊는다(코드 확인 2026-10-18 — `snap.rs` 가 돌려주는 `Snap` 이 `book.rs` 에 정의돼 있다) |
| `wire.rs` | invoke · emit 타입(ts-rs) | — | `types.rs` 를 대신 |
| `mod.rs` | `DockState`(세션 · 실행기 줄 · 세계 캐시 · 명부) | — | `Book` 을 쥐던 것을 바꿈 |

**Rust — 그 밖**

| 파일 | 바뀌는 것 |
|---|---|
| `src-tauri/src/commands/dock.rs` | invoke 넷(§4) + `window_role` · Tauri 어댑터(포트 구현 — platform 함수 · Tauri 창 API · 예비 창 짓기) · `on_window_event` 의 `Moved` · `Destroyed` 와 `on_page_load` 를 세션으로 · `report_dock_chrome` 의 명부 갈래(§2-5). 옛 명령 여섯 · 옛 실행기(`DockExec` · `reconcile` · `own_hidden`)는 걷는다(부록 B) |
| `src-tauri/src/commands/layout.rs` | `report_window_canvas` · `report_ui_metrics` 의 명부 갈래(§2-5) · `TauriEvents::settled` → `WorldChanged` |
| `src-tauri/src/layout/{dock,manager,apply}.rs` | §7 — 붙는 몫 인자 · `AdoptWindow`(+ 재기 옮겨 담기) · 비운 창 남기기(+ 재기 돌려주기) · 표식 이어 쥐기 · 떼는 창 다음 활성 · 메인 빈 탭 id · `dock_tab` 계약 문서(B9) |
| `src-tauri/src/layout/chrome.rs` | `DockChrome.tab_bar` 의 뜻 = 탭 끌기 영역 사각형(C3 · U7b-1 부터 웹뷰가 이 뜻으로 보낸다) · 모양 `{canvas_origin, tab_bar}` 둘로 줄이기(머리 목록 · `scroll` 걷음)는 U8 — 새 모양을 처음 보내는 단위가 Rust 모양 · 생성물을 함께 바꾼다(B8) |
| `src-tauri/src/commands/popout.rs` | 예비 창 빌더(보통 팝아웃 URL · 짓기 전 명부에 적음) · 예비 창은 모델 · `record_placement` 밖 |
| `src-tauri/src/lib.rs` | 명령 등록 · 창 사건(`Moved` → 세션 · `Destroyed` → 명부 뒤 `WindowGone`) · `on_page_load`(→ `PageLoaded`) · 자리 기록의 명부 건너뜀 · setup 에서 예비 창 · 안내 창을 작업 스레드로 짓고 폴러 · 시한 스레드를 띄움 · 팝아웃 닫기 요청 갈래는 그대로(§2-5 — 6.1 의 명부 확인을 걷음) |
| `src-tauri/src/tray/actions.rs` | `ui_windows` 에 넘기는 술어에 명부 빼기(B7) — `tray/core.rs` 의 `ui_windows` 는 술어를 받으므로 그대로다 |
| `src-tauri/capabilities/` | `dock-overlay` 창 권한(이벤트 듣기) — 새 파일 |
| `crates/engram-dashboard-platform/` | §8 — `move_loop.rs`(새) · `window.rs`(더함) · `input.rs`(새) · `Cargo.toml`(windows 기능 · `raw-window-handle`) |

**TS — `src/components/layout/tabStrip/`(새 폴더 — 탭바 하나에 묶인 것)**

| 파일 | 맡는 것 | 순수? · 시험 |
|---|---|---|
| `layout.ts` | 탭 폭 · 숨김(지금 `tabStripLayout.ts` 를 옮김 — R24 · 5.8 §4-5) + 닫기 모드의 고정 폭 입력 | 순수 · 표 시험(옮겨 옴) |
| `reorderGeometry.ts` | 이상적 탭 경계(폭 누적) · 끌기 영역 폭(C3) · 끄는 탭 x · 삽입 자리(Chromium `CalculateInsertionIndex` — C1) · 순서 문턱(`round(16 × 그 자리 폭 ÷ S)`) · 이웃이 갈 자리 · 비켜남 곡선(C2) | 순수 · 표 시험 |
| `closingMode.ts` | 닫기 모드 경계(아래 40 · 끝 쪽 60 · 반열림) · 상태 줄(들어감 · 떠남 300 ms · 밖 누름 즉시) + `useClosingMode` | 순수 + 훅 · 가짜 시계 시험 |
| `useTabDrag.ts` | 누름 · 문턱 · 캡처 · 탭바 안 몰기 · zone 나감 · 끝 · 키 · `seq` · begin · leave 답 기다리는 동안 끝 입력 붙들기 · `token` · `dock:feed{Release}` 에 지금 자리로 `tab_drag_end{Drop, Release, at}` 답하기(E7 — 이미 끝냈으면 버림) — 입력 포트 하나(`Pointer` = 자기 캡처 \| `Shell` = `dock:feed`) | 가짜 입력 · 가짜 invoke 시험 |
| `measure.ts` | DOM 재기 하나 — 탭바 줄 사각형 · 끌기 영역 사각형(첫 탭 왼쪽 + 끌기 영역 폭) · 캔버스 사각형(머리는 재지 않는다 — 이상적 경계는 `layout.ts` 가 낸다) | 얇음 |

**TS — 그 밖**

| 파일 | 맡는 것 |
|---|---|
| `src/components/layout/TabBar.tsx` | 그리기 · 이름 바꾸기 · 닫기 · `+` 만(끌기는 `useTabDrag` · 닫기 모드는 `useClosingMode`) · 활성 탭 모양 · 구분선 투명도(R27 · C7) |
| `src/components/layout/dock/dockSession.ts` | `dock:drag`(끄는 중이면 앱 전체 키 감시 · 세션 번호가 바뀔 때만 카운터를 0 으로 — 같은 번호의 `Active` 를 또 받아도 그대로 · E6) · `dock:feed`(그 창의 `useTabDrag` 입력 · `Attach` 도 세션 번호를 실어 카운터를 세운다) · `dock:spare` + `window_role`(대기 모드 · 마운트 때 답의 `drag` 로 진행 중 세션을 받아 키 감시를 켠다 — 다시 읽힌 창 · E6) |
| `src/components/layout/dock/frameReport.ts` | 크롬 보고(지금 `dockChromeReport.ts`) — `tab_bar` = 끌기 영역 사각형 · U8 까지는 모양을 그대로 두고 `tabs: []` · `scroll: null` 을 싣는다(B8) — `src/util/latestValueReporter.ts`(새 · 「한 번에 하나 · 같은 값 안 보냄 · 디바운스」) 위에. `windowCanvasReport.ts` 도 같은 도우미로 옮긴다(점검 결함 8) |
| `src/components/layout/dock/fileDropGuard.ts` | 파일 · URL 드롭 막기(지금 `dockDrag.ts` 의 비-우리 갈래 — AC22) · 늘 켜짐 |
| `src/components/layout/dock/DockOverlay.tsx` | 안내 그리기 — **이미 `DockOverlayView({ guide })` 라는 props 보기가 있다**(HEAD 그 파일 — 스토어에 묶인 것은 그것을 감싼 `useDockGuide` 쪽). 바꿀 것 = `guide` 의 타입을 `DockGuide` 에서 `OverlayView \| null` 로 바꾸고 `dockDrag` 스토어 · `DockRegions` 의존을 걷는 것(B12) |
| `src/pages/DockOverlayPage.tsx` · `App.tsx` | 안내 창 경로 `#/dock-overlay` — `dock:overlay` 를 받아 `DockOverlayView` 로 그림 |
| `src/components/layout/WindowLayout.tsx` | 대기 모드(`window_role` · `dock:spare` — 자가닫힘 · 끌어오기 재시도 안 함) · 크롬 보고 · 세션 훅 연결 · 창 안의 `DockOverlay` 걷음 |

**점검 결함 1–9 를 어디서 닫나**

| 결함 | 닫는 곳 |
|---|---|
| 1 `createDockDrag` 거대 클로저 | 파일을 걷고 관심사별로 나눔 — 탭바 안 = `useTabDrag` · 세션 사건 = `dockSession` · 파일 막기 = `fileDropGuard` · 안내 = `DockOverlayPage`. 호버 전환 · Esc 삼키기 · 끌기 신호 유실 감지 · 떠남 시한은 새 모델에 없다 |
| 2 보고 모양 7 번 | 「그린 것」 보고가 없다 — 그리기 · 판정이 둘 다 셸이다. invoke 인자는 각각 한 번만 정의(`wire.rs`) |
| 3 `EndLog` 가 `EndCause` 를 베낌 | 끝 로그는 `EndLog::of(&Session, reason)` 하나가 만든다 |
| 4 늘 0 인 스크롤 배관 | 걷음 — wire 에 `scroll` 칸이 없다(U8 까지 크롬 보고에 `null` 로만 남는다) |
| 5 거울 지킴이가 셋 다 다름 | 거울이 없다(§0) |
| 6 탭바 DOM 재기 둘 | `measure.ts` 하나 · 머리는 재지 않음 → transform 피하기 ref 둘(`reorderStandingRef` · `seenChrome`)이 필요 없다 |
| 7 순서 이동 기하 네 곳 | `reorderGeometry.ts` 하나 |
| 8 보고기 복제 · 디바운스 상수 둘 | `latestValueReporter` 하나 · 상수 하나 |
| 9 작은 것 | `WindowFrame` 원점 · 배율은 이제 실제로 쓰인다(화면 점 → 창 CSS 점 — `strip.rs`) · `pick` 재조립 없음 · `dragImage` 걷음 · 「출처 창」 술어 = 세션 `holder` 하나 · 탭바 높이 28 은 재 쓰지 않는다(잰 사각형만) · OLE 시험 걷음 |

## 4. wire · IPC

모든 invoke 는 창 label 을 인자로 받지 않고 부른 창(`Window`)에서 꺼낸다(지금 규칙). 좌표 = 부른 창의 뷰포트 CSS px. 타입은 `src-tauri/src/dock/wire.rs`(ts-rs → `src-tauri/bindings/`).

**웹뷰 → 셸 invoke(새 다섯)**

```rust
// 문턱(10 px)을 넘은 순간 한 번. 답은 메인 스레드 세션의 판정.
tab_drag_begin { token: u64 /* 웹뷰가 짓는 끌기 표 — 시한 뒤 Abandon 의 짝 */, view: Uuid, down: PointCss /* 누른 점 */,
                 area: RectCss /* 지금 탭 끌기 영역 사각형 — C3 */, grab: Grab, initial_active: Option<Uuid> }
    -> Result<BeginReply, String>
pub struct Grab { x_ratio: f64 /* 탭 안 잡은 x ÷ 탭 폭 — 조사 A3 */, y: f64 /* 탭 위에서 잡은 y */, single_x: f64 /* 떼어진 창(탭 하나)에서의 잡은 x = x_ratio × 그 창 탭 폭 — TS 가 layout.ts 로 셈 */ }
pub enum BeginReply { Strip { session: u64, zone: RectCss }, WholeWindow { session: u64 }, Busy, Rejected { reason: String } }

// mount: u64 = 웹뷰가 마운트 때 한 번 짓는 난수 표(F6 c — 아래 「순서」) · 끌기 사건 셋이 다 싣는다
tab_drag_reorder { session: u64, seq: u32, mount: u64, index: u32 } -> Result<(), String>
tab_drag_leave   { session: u64, seq: u32, mount: u64 } -> Result<LeaveReply, String>   // 커서는 셸이 읽는다 · 시한이면 처리기가 이 session 으로 Abandon{by: Session}(F5)
pub enum LeaveReply { Detached, NotReady /* 예비 창 준비 전 — Strip 에 머물고 다시 청한다(§2-5) */, Ended /* 세션이 그사이 끝남 */ }
tab_drag_end     { session: u64, seq: u32, mount: u64, how: EndHow, src: EndSrc, at: Option<u32> } -> Result<(), String>
pub enum EndHow { Drop, Cancel }
pub enum EndSrc { Pointer /* pointerup */, Release /* dock:feed{Release} 에 holder 가 지금 자리로 답함 — E7 */, CaptureLost, Key }

window_role {} -> WindowRoleReply                                                   // { role: Model | Parked | Unknown, drag: Option<u64> }
    // role = 부른 창의 지금 역할(§2-5) · drag = 진행 중 끌기 세션 번호(E6 — 다시 읽힌 웹뷰가 키 감시를 다시 켠다 · 1 부터 단조 — F6 a)
```

- `single_x` 를 TS 가 보내는 까닭 — 떼어진 창의 탭 폭(표준 폭 `S`)은 TS 상수다. 셸이 그 값을 사본으로 갖지 않게 TS 가 셈해 싣는다(거울 없음 — §0).
- `index` 는 결과 탭 목록의 자리다(지금 `DockTo::TabBar.index` 뜻 그대로). `at` 도 같은 뜻이다.
- `down` 은 창째 끌기의 자리(C4 — 「창 원점 + (커서 − 누른 점)」)에 쓴다. 셸이 begin 을 받는 때의 그 창 안쪽 원점 · 배율로 화면 점으로 옮긴다.
- `window_role` 은 웹뷰 안 배선이다(대기 모드 고르기 · 진행 중 세션 받기) — LLM 이 쓸 명령이 아니어서 버스 명령표에 올리지 않는다. `drag` 는 세션이 메인 스레드에만 있으므로 실행기가 `dock:drag{Active | Ended}` 를 낼 때 함께 적는 원자 거울(`DockState` 의 지금 세션 번호 · 0 = 없음)에서 읽는다 — 동기 명령이 메인 답을 기다리지 않는다. U4 는 `drag: None` 으로 세우고 U6 이 거울을 잇는다.
- **`window_role` 의 `drag` 로 켤 때(F6 a)** — 웹뷰는 마운트 때 `dock:drag` 듣기를 **먼저** 건 뒤 `window_role` 을 묻는다. 그사이 받은 `dock:drag{Ended(s)}` 로 `lastEnded = max(lastEnded, s)` 를 적고, 답의 `drag` 가 `lastEnded` 보다 클 때만 키 감시를 켠다 — 세션 번호가 1 부터 단조라 이 비교가 선다. 6.2 는 답보다 먼저 온 `Ended` 를 지나 답의 옛 번호로 감시를 켜, 끝난 끌기의 감시가 남아 그 창 키보드가 먹혔다.
- **`mount`(F6 c)** — 웹뷰가 마운트 때 한 번 짓는 난수 표(`token` 처럼 웹뷰가 짓는다)다. 셸은 보낸 창마다 마지막으로 본 표(`mounts`)를 적고, 표가 바뀌면 그 창의 `seen` 을 0 으로 쳐 낡음 거르기를 다시 시작한다(§2-3-1 ④). 다시 읽힌 창의 옛 페이지가 늦게 보낸 사건이 T26 이 지운 `seen` 을 다시 올려도 새 페이지의 사건이 낡음으로 버려지지 않게 하려는 것이다.

**순서(A3 · B5) — `seq` · 최종 자리 · 답 기다리는 동안**

- **`seq`** = 그 웹뷰가 그 세션에서 보낸 끌기 사건(`reorder` · `leave` · `end`)의 번호 — 1 부터 단조 증가. 웹뷰마다 세션 번호 하나에 카운터 하나다(`useTabDrag` 와 키 감시가 함께 쓴다 · 새 세션 번호 = begin 답 · `dock:drag{Active}` · `dock:feed{Attach}` · `window_role` 의 `drag` 중 하나로 처음 들을 때 0 에서 시작 · ★같은 번호를 또 들으면 카운터를 그대로 둔다★ — 예비 창을 같은 끌기에서 두 번 들이거나 붙었다 떼도 겹치지 않게 · E6) — 붙기로 되돌리지 않는다(되돌리면 붙기 전에 보낸 사건과 번호가 겹친다). 웹뷰가 다시 읽히면 카운터가 0 부터라 셸은 그 창의 `seen` 을 지운다(T26) — 그 뒤 옛 페이지의 늦은 사건이 `seen` 을 다시 올려도 새 페이지의 사건은 `mount` 가 달라 0 부터 다시 센다(F6 c). 셸은 보낸 창마다 마지막 seq 를 적고 그 이하를 버린다(§2-3 받기 거르개). 창마다 세는 까닭 = 키 끝은 앱 전체 키 감시라 아무 우리 창에서나 온다.
- **사건은 절대값을 싣는다** — `reorder.index` = 결과 자리 · `end.at` = 놓는 순간 웹뷰가 아는 자리(holder 가 보낸 것만 — 다른 창의 키 감시는 `None`). 그래서 늦게 온 낡은 `reorder` 를 버려도 잃는 것이 없고, 놓기가 마지막 `reorder` 를 앞질러도 셸은 `at` 으로 그 자리를 쓴다(§2-3 `StripDrop`). `leave` 는 자리를 싣지 않는다 — 떼면 그 탭이 그 탭바에서 빠지므로 남은 탭 순서는 끄는 탭이 어디 있었든 같다. holder 가 아닌 창의 키 놓기(`at = None`)와 셸이 본 뗌(폴러 · 버튼 감시)은 바로 끝내지 않고 **놓기 확인**을 탄다(T23 · D5 · E7) — 셸이 holder 에 `dock:feed{Release}` 를 보내고 holder 가 지금 자리를 `at` 으로 실어 `tab_drag_end{Drop, Release}` 로 답한다. 그 답은 holder 가 이미 보낸 `reorder` 보다 seq 가 크므로 날아가던 `reorder` 가 늦게 와도 `at` 이 이긴다. holder 가 1 초(G18) 안에 답하지 않으면 지금 모델 자리로 끝낸다(T31).
- **답을 기다리는 동안**(begin · leave) — 웹뷰는 그사이의 놓기 · 키 · 캡처 잃음 중 **처음 것 하나**와 마지막 포인터 점을 붙든다. 답이 오면:
  - `Strip` → 붙든 끝이 있으면 `tab_drag_end` 로(seq 1) · 없고 마지막 점이 zone 밖이면 `leave`.
  - `WholeWindow` · `Detached` → 붙든 **키** 끝만 보낸다 — 포인터 끝(뗌 · 캡처 잃음)은 버린다: 그 단계의 뗌은 루프가 보고, 루프에 들어갈 때 주 버튼이 이미 떨어져 있으면 놓기다(§8).
  - `NotReady` → Strip 에 머문다(붙든 끝이 있으면 보냄 — 놓기 확인 중에 온 leave 의 답도 이것이다 · T21).
  - 답을 기다리는 동안 받은 `dock:feed{Release}` 도 붙드는 끝 입력이다(답 뒤 `End{Drop, Release}` 로).
  - `Busy` · `Rejected` · `Ended` · 오류 · 시한 → 그 끌기를 버린다(포인터 캡처를 놓는다).
- **시한**(G18 · 1 초) — begin · leave 처리기가 메인 답을 시한 안에 못 받으면 `Abandon` 을 메인 줄에 넣고 웹뷰에 오류로 답한다 — begin = `Abandon{by: Token(token)}` · leave = `Abandon{by: Session(session)}`(§2-4 · B6 · F5 — 붙은 창의 holder 웹뷰는 begin 의 `token` 을 모른다).

**웹뷰 → 셸 보고(고침)**

- `report_dock_chrome { chrome: DockChrome }` — **`tab_bar` 의 뜻 = 탭 끌기 영역 사각형**(C3 — x = 첫 탭 왼쪽 ~ 그 + 끌기 영역 폭 A · y = 탭바 줄 위아래 · §5-1). U7b-1 부터 이 뜻으로 보내되 모양은 그대로 두고(`tabs: []` · `scroll: null`), U8 에서 `DockChrome = { canvas_origin: PointCss, tab_bar: RectCss }` 로 줄인다 — 새 모양을 처음 보내는 단위가 Rust 모양 · 생성물 · TS 타입을 함께 바꾼다(B8 · §13). 이름은 그대로 둔다(G10). 모델 밖 창이면 명부에 적는다(§2-5).
  - **재지 못한 동안은 보내지 않는다** — 웹뷰는 탭바 줄의 쓸 수 있는 폭이 유한하지 않거나 0 이하인 동안(줄이 아직 · 잠깐 0 폭) · 캔버스를 아직 재지 않은 동안 이 보고를 보내지 않고, 셸은 그동안 마지막으로 받은 값을 그대로 쓴다(빈 탭바 구역을 보내면 다음 보고까지 그 창에 붙지 못한다 — 웹뷰 `tabStrip/measure.ts` `frameMeasureOf`). 그래서 웹뷰는 폭이 돌아오는 때도 다시 재 보낸다 — 캔버스 · 탭바 줄 크기 관측 · 탭 목록 · 활성 · 모드 바뀜에 더해 **페이지가 다시 보일 때**(`visibilitychange`). **따라 나오는 것** — 예비 창은 첫 유효 보고를 받은 뒤에야 준비된다(명부 `ParkedEntry::ready` 의 `chrome.is_some()` — `dock/parked.rs`). 숨은 채 지은 예비 창에서 그 줄이 0 폭으로 재지는지는 재지 않았다 — 그러면 그 창은 유효 보고가 올 때까지 예비로 쓰이지 않는다.
- `report_window_canvas { w, h }` · `report_ui_metrics` — 그대로(도킹 밖에서도 쓴다) · 모델 밖 창이면 명부에 적는다.

**셸 → 웹뷰 사건(새 넷)**

| 이름 | 받는 창 | 몸 | 쓰임 |
|---|---|---|---|
| `dock:drag` | 시작 때 모델 창 전부 · 그 뒤 들인 예비 · 되살린 남긴 창 · 새로 든 모델 창(E6 — T07 · T29) · 끝에 `armed ∪ 모델 창 전부`(안내 창 제외 — `armed` 인 명부 창도 받는다: 들였다가 예비로 되돌린 창의 감시가 켜진 채 남지 않게 · F6 b) | `{ session, phase: Active \| Ended }` — `session` 은 1 부터 단조(F6 a) | 끄는 중 앱 전체 키 감시 켜기 · 끄기(§6-3 — 감시는 `Ended` 밖에도 키 끝 하나를 보내면 스스로 꺼지고, 같은 세션의 `dock:feed{Attach}` 로 다시 켜진다 · F6 b). ★옛 `dock:session` 과 이름을 달리한다★ — U6 부터 U8 까지 옛 사건이 남아 있어, 같은 이름이면 두 모양이 한 이름에 실린다(B8 · G10) |
| `dock:feed` | 탭을 쥔 창 하나 | `{ session, view, step: Attach{zone, grab, x, y} \| Move{x, y} \| Release \| Detached }` | 붙은 창의 `useTabDrag` 입력(좌표 = 그 창 뷰포트 CSS px) · `Release` = 셸이 놓기를 봤다 — 지금 자리로 `End{Drop, Release, at}` 를 보내라(E7 · 출처 창에도 간다 — 버튼 감시 · 남의 키 놓기) |
| `dock:overlay` | 안내 창 | `OverlayView \| null` — `{ bands: Vec<RectCss>, outer: Vec<RectCss>, preview: Option<RectCss> }`(안내 창 CSS px) | 띠 · 미리보기 그리기 |
| `dock:spare` | 명부 창 | `{ step: Parked \| Adopted }` | 대기 모드 들어감(빈 틀로) · 나옴(보통 팝아웃으로) |

**걷는 것** — 명령 `dock_drag_start` · `dock_drag_over` · `dock_drop` · `dock_drag_end` · `dock_pointer_seen` · `dock_pull` / 사건 옛 `dock:session` · `dock:regions` / 타입 `DockRegions` · `Band` · `OuterBand` · `SlotRegion` · `TabBarRegion` · `ReorderRegion` · `StripStart` · `Picked` · `Started` · `SessionPhase` · `SessionEvent` · `DockPullReply` · `DragOverReport` · `DropReport` · `DrawnReport` · `DragEffect` · `TabHeader` · `DockChrome.tabs` · `DockChrome.scroll` / 골든 `pick_golden.json` / 생성물(`src-tauri/bindings/` 의 해당 `.ts` — CI sync 게이트가 남은 것을 잡는다).

## 5. 순수 함수 명세

단위 = CSS px(= 논리 · DIP px — 웹뷰 확대 1 기준). 사각형은 반열림 `[x0, x1) × [y0, y1)`(왼쪽 · 위 포함 — 5.8 R6a ③ 규칙 그대로). 부동소수 비교만 · 반올림은 적힌 곳에서만.

### 5-1. 탭바 안 순서 이동 — `tabStrip/reorderGeometry.ts` (TS 만 · C1 · C2 · C3)

Chromium 의 탭바 안 끌기(`DraggingTabsSession::MoveAttachedImpl` · `TabStrip` 의 `GetInsertionIndexForDraggedBounds` → `CalculateInsertionIndex`)를 탭 하나 · 고정 탭 없음 · 그룹 없음 · 분할 없음 · 겹침 0 인 우리 경우로 그대로 옮긴다. 줄 번호 = Chromium 사본(§1).

입력: 그 탭바가 그리는 탭의 이상적 경계 `B[i] = [x0_i, x1_i)` — `layout.ts` 폭의 누적 · **끌기 영역 왼쪽(= 첫 탭 왼쪽) 기준** · 지금 모델 순서(끄는 탭 `d` 포함) · 숨긴 탭은 보이는 끝 `c` 에 선 폭 0(5.8 §7 #71 그대로) · 끄는 탭 `d` · 그 폭 `w = x1_d − x0_d` · 잡은 폭 비율 `r`(`Grab.x_ratio`) · 점 `p`(커서 x — 끌기 영역 기준) · `available` = `layout.ts` 가 받는 「머리들이 쓸 수 있는 폭」(탭바 줄 폭 − `+` 폭 — `tabStripLayout.ts` 머리 주석).

| 함수 | 식 | 출처 |
|---|---|---|
| `passedThreshold(down, p)` | `hypot(p − down) > 10`(가로세로 같음 · 같으면 아님) · `DRAG_THRESHOLD_PX = 10` | 조사 A2 `kMinimumDragDistance = 10`(TDC#1068 grounding) |
| `dragAreaWidth(available, B)` | `A = max(available, Σ 보이는 폭)` — ★닫기 모드여도 고정 폭이 아니라 잰 `available`★ | C3 — `GetTabDragAreaWidth` = `max(GetAvailableWidthForTabContainer(), tab_strip_->width())`(tab_strip.cc#530-544 · 주석 「available width for tabs in this case needs to ignore tab closing mode」) |
| `draggedLeft(p, r, w, A)` | `L = clamp(p − round(r · w), 0, max(0, A − w))` — `round` = .5 는 0 에서 멀리 | `GetAttachedDragPoint`(dragging_tabs_session.cc#329-343) · 탭 하나의 `mouse_offset` = `round(r · w)`(#25-36 `CalculateMouseOffset`) |
| `insertionIndex(B, d, L)` | ① 후보 `k = 0…n`(n = 탭 수 · 끄는 탭 포함) — `k = d + 1` 은 건너뛴다(끄는 탭 바로 뒤 = 제자리) ② 이상적 x: `k = 0` → 0 · 아니면 `x1_{k−1} − (k − 1 > d ? w : 0)` ③ `\|L − 이상적 x\|` 가 가장 작은 `k`(같으면 앞의 것 — 엄격한 `<`) ④ `k > d` 면 `k − 1`(끄는 탭을 뺀 결과 목록의 자리) ⑤ `min(k, 보이는 마지막 탭의 자리)` | `CalculateInsertionIndex`(tab_strip.cc#980-1044) · `IsValidInsertionIndex`(#1046-1060 — 그룹 · 분할 갈래는 우리에게 없다) · `CalculateIdealX`(#1096-1119 — 겹침 0) · 보이는 범위 자르기(#589-600 — `last_insertion_point = max(0, 마지막 보이는 탭 자리 + 1 − 1)`) |
| `reorderThreshold(B, to)` | `round(폭(to) ÷ S × 16)` — `폭(to)` = 지금 자리 `to` 에 있는 탭의 폭(비켜남은 x 만 옮겨 폭이 같다) · `S` = 표준 폭(`TAB_STANDARD_W` 160) | `kHorizontalMoveThreshold = 16` · `GetTabAt(to_index)->bounds().width() / GetStandardWidth × 16` · `ClampRound`(dragging_tabs_session.cc#89-94) |
| `moveDue(pLast, p, thr, justAttached)` | `justAttached \|\| abs(p − pLast) > thr` | #99-104(그룹 · 연속 아님 갈래는 탭 하나에 없다) |
| `slideTargets(B′, d)` | 순서가 바뀐 뒤 끄는 탭 밖의 탭은 새 이상적 경계 `B′` 로 **200 ms** 에 간다(옛 자리 → 새 자리 translateX) · 곡선 **EASE_OUT** `1 − (1 − t)²` = CSS `cubic-bezier(0.3333, 0.6667, 0.6667, 1)`(2차 곡선을 3차로 올린 정확한 꼴) · `prefers-reduced-motion: reduce` 면 0 · 끄는 탭은 늘 `L` 에 그린다 · 겹치는 동안 끄는 탭이 위 | C2 — `RichAnimationDuration` 200 ms(풍부한 애니메이션 꺼짐 = 0) · `BoundsAnimator` 기본 tween `EASE_OUT`(tab_container_impl.cc#175-191 · #1153-1160 · bounds_animator.h#205 · #207 · bounds_animator.cc#205-206 · tween.cc#72-73 · animation.cc#114-116) |

- **움직일 때마다**(포인터 · `dock:feed{Move}`) `L` 을 셈해 끄는 탭을 거기 그리고 `to = insertionIndex(B, d, L)` 를 셈한다. `moveDue` 이고 `to` 가 지금 자리와 다를 때만 `tab_drag_reorder{to}` 를 보낸다(Chrome 은 같은 자리에도 `MoveSelectedTabsTo` 를 부르지만 모델이 그대로다).
- **`pLast` 를 새로 적는 때 = 자리가 실제로 바뀐 때만**(C1 — 「Move may do nothing ... Make sure the tabstrip actually changed before updating `last_move_attached_context_loc_`」 #143-149). 문턱을 넘었어도 자리가 같으면 그대로다. 처음 값 = 그 탭바에서 끌기 세션이 시작된 점 — 출처 = 문턱을 넘은 점 · 붙은 창 = 붙은 점(`StartDraggingTabsSession(…, point)` TDC#850 · #1230 · #1259 · #2034 → 생성자 #48-56).
- **붙은 직후**(C1 · G9 결정) — 셸은 `TabBar{W, 0}` 으로 넣는다(Chrome 은 첫 non-pinned 자리 `IndexOfFirstNonPinnedTab` — 우리에게 고정 탭이 없어 0 · TDC#1513-1531 「Insert at any valid index ... We'll fix up the insertion index in MoveAttached() later」). 대상 웹뷰는 `dock:feed{Attach}` 를 받으면 `justAttached = true` 로 한 번 셈해 `to ≠ 0` 이면 곧바로 보낸다(dragging_tabs_session.cc#56 `MoveAttachedImpl(start, true)`). 그때 끄는 탭 밖의 탭은 200 ms 로 가지 않고 곧바로 새 자리에 선다(#157-162 「Snap the non-dragged tabs to their ideal bounds now, otherwise those tabs will animate to those bounds after attach, which looks flickery/bad」). ★6판의 「맨 뒤에 넣고 고침」(G9 a)은 Chrome 이 아니었다★.
- **출처의 처음**도 같다 — Chrome 은 문턱을 넘는 순간 세션을 만들며 `justAttached = true` 로 한 번 셈한다(#56). 그 자리는 대개 지금 자리라 보낼 것이 없다.
- **넘친 탭바**(R24) — ⑤ 의 자르기로 끄는 탭은 보이는 마지막 탭 너머의 자리로 가지 않는다(Chrome 의 「Clamp the insertion point to keep it within the visible region」 #594-599). 그리는 자리 `L` 은 끌기 영역 끝까지 간다.
- **모델이 따라오는 동안** — `tab_drag_reorder` 를 보낸 뒤 모델 메아리(`window:tabs-updated`)가 오기 전까지 TS 는 보낸 자리를 지금 자리로 쓴다(같은 자리를 두 번 청하지 않게). 메아리가 다른 자리를 말하면 메아리를 따른다 — 다른 경로가 탭을 바꾼 것이고 따로 다루지 않는다(Q19).
- **★PRD R23 문장과 다른 곳(C3)★** — R23 은 「그 창 탭들이 차지한 범위 밖으로는 나가지 않고」라 적었지만 Chrome 은 끄는 탭을 **끌기 영역**(탭 컨테이너가 쓸 수 있는 폭 — 마지막 탭 뒤 빈 곳 포함)까지 그린다(tab_strip.cc#531-537 주석 「allows tabs to be dragged past the current bounds of the tabstrip, anywhere along the tab strip region」). R23 의 「데스크톱 Chrome 그대로」(Q23)가 앞서므로 Chrome 을 따르고 부록 A 에 R23 세부 정정으로 적는다(PRD 문장 고침은 메인 몫). 끌기 영역이 `+` 앞에서 끝나는지는 가능성 높음이다(Chrome 의 `GetTabStripDraggableBounds` 가 새 탭 단추를 남겨 둔다 — 「preventing tabs from being dragged past the new tab button」 · horizontal_tab_strip_region_view.cc#872-911 · 그 경계가 이 폭으로 이어지는 연결은 미확인). 우리 `available` 은 `+` 폭을 뺀 값이라 같은 쪽이다.

### 5-2. 탭바 구역 — `dock/strip.rs` (Rust 만 · C3)

```rust
pub const DETACH_MAGNETISM_PX: f64 = 15.0;           // kVerticalDetachMagnetism = 15(TDC#401 · 조사 A5 · grounding)
pub fn strip_zone(area: RectCss) -> RectCss;         // { x0: area.x0, x1: area.x1, y0: area.y0 − 15, y1: area.y1 + 15 }
pub fn client_point(p: PhysPoint, inner_origin: PhysPoint, scale: f64) -> PointCss;   // (p − origin) / scale
```

- `area` = 그 창 크롬 보고의 `tab_bar` = **탭 끌기 영역 사각형**(§4) — 가로 `[첫 탭 왼쪽, + A)` · 세로 = 탭바 줄. Chrome 의 떼기 판정 `DoesTabStripContain` 그대로다: `x − 탭바.x ∈ [TabDragAreaBeginX, TabDragAreaEndX)`(반열림 · 폭 = §5-1 의 `A`) · y 는 탭바 경계 ± 15(`DoesRectContainVerticalPointExpanded` TDC#234 — TDC#1461-1477). ★G7 은 C3 로 닫혔다 — 탭바 줄 전체도, 탭들 범위도 아니다★.
- **나갈 때 · 들어올 때 같은 사각형**(PRD D19) — 출처 · 붙은 창 웹뷰는 `begin` 답 · `dock:feed{Attach}` 로 받은 `zone` 에 대해 「점이 안인가」만 본다. 셸은 이동 루프 동안 각 창의 크롬 보고 `tab_bar` 로 같은 함수를 부른다. 이력(hysteresis) 없음(5.8 §2-5 · D19).
- 터치 값(50)은 쓰지 않는다 — 마우스만 다룬다.
- `REORDER_SLACK_PX` · `reorder_zone` 을 이것으로 옮긴다(이름 · 가로 범위가 바뀜).

### 5-3. 도킹 띠 — `dock/geometry.rs` (Rust 만)

```rust
pub const BAND_SHARE: f64 = 0.10;           // 칸 가장자리 10% — ADR-0295 결정 3 · PRD R3(VS Code editorDropTarget.ts #408 꼴)
pub const BAND_MIN_PX: f64 = /* G5 */;      // 포인터로 짚을 최소 두께 — PRD R3 「값은 TRD」
pub const OUTER_BAND_PX: f64 = 32.0;        // 바깥 띠 — PRD R5 「4.8 에서도 그대로」 · 5.8 §2-3
pub struct BandHit { pub anchor: DockAnchor, pub side: DockSide, pub landing: PxRect, pub areas: Vec<PxRect> }
pub fn slot_band(frame: PxRect, side: DockSide) -> PxRect;
pub struct WindowTargets<'a> {                                       // 한 창 몫의 입력(`targets.rs` 가 세계에서 만든다)
    pub canvas: CanvasPx, pub layout: &'a LayoutNode /* 보이는 탭 배치 */, pub frames: Vec<(Uuid, PxRect)> /* geometry::frame_px */,
    pub incoming: &'a LayoutNode /* 끄는 탭 배치 */, pub original: SizeDip, pub zone_in_canvas: Option<RectCss>,
}
pub fn effective_bands(w: &WindowTargets) -> Vec<BandHit>;           // 유효(R8) · 앞선 영역을 뺀 영역(R6a) · 붙을 사각형
pub fn choose(bands: &[BandHit], p: PointCss, frames: &[(Uuid, PxRect)]) -> Option<usize>;
```

- **칸 띠 두께**(R3) — 좌우 띠 `t = max(round(0.1 × 칸 폭), BAND_MIN_PX)` · 위아래 띠 `t = max(round(0.1 × 칸 높이), BAND_MIN_PX)` · 칸 틀 안으로 자른다(`t` 가 칸보다 크면 칸 전체). `round` = 가장 가까운 정수(.5 는 0 에서 멀리 — G19) — 그린 띠와 판정이 같은 정수 사각형이다.
- **바깥 띠**(R5) — 캔버스 네 변의 32 px 띠(지금 `outer_strips` 그대로).
- **막힌 띠**(R8) — 그 띠에 놓은 결과(§5-4 의 몫으로 `docked_tree`)가 면적 0 칸을 만들면(`ZeroArea`) 그 띠는 없다 — 칠하지도 판정하지도 않고, 막힌 바깥 띠는 밑 칸 띠를 덮지 않는다(R6a · Q15).
- **유효 띠 영역**(R6a) — 바깥 띠 영역 = 그 띠 − 탭바 구역(캔버스 좌표로 옮긴 `strip_zone`) · 칸 띠 영역 = 그 띠 − 탭바 구역 − 유효 바깥 띠 전부. 영역은 사각형 목록이다(축 정렬 사각형끼리 뺀 것 — 많아야 넷). 전부 덮인 띠는 빈 목록이라 칠하지 않는다.
- **고르기**(R6a ①②③) — ⓪ 탭바 구역은 이 함수 앞에서 이미 붙기로 갔다(§2-3 · `targets.rs`) ① 점을 품은 유효 바깥 띠 영역이 있으면 그중 「캔버스 변까지 거리 ÷ 띠 두께」가 가장 작은 것 ② 아니면 점을 품은 칸(반열림 — 이웃 칸이 같은 정수 경계를 나눠 점 하나에 칸 하나)의 유효 띠 영역 중 「칸 변까지 거리 ÷ 띠 두께」가 가장 작은 것 ③ 같으면 `SIDE_ORDER`(Left → Right → Top → Bottom) ④ 없으면 `None`(칸 가운데 — R6 · 새 창으로 남는 자리).
- **안내 창에 그리는 것** — 커서 밑 우리 창의 유효 바깥 띠 영역 넷 · 커서 밑 칸의 유효 띠 영역 · 고른 띠가 있으면 그 `landing`(미리보기). 그리기와 판정이 같은 `BandHit` 를 쓴다 — 칠한 곳 = 붙는 곳.

### 5-4. 붙는 크기 — `dock/sizing.rs`

```rust
pub fn attach_share(original: SizeDip, side: DockSide, target: PxRect) -> f64;
//   L = (Left|Right) ? original.w : original.h        — 나누는 방향의 원래 길이(ADR-0295 결정 4 · PRD R7)
//   T = (Left|Right) ? target.w   : target.h          — 붙는 칸(Outer 면 캔버스)
//   share = clamp(min(L, T / 2) / T, tree::RATIO_MIN, 0.5)
```

- `docked_tree(.., share)` 가 새것 몫을 그대로 쓴다 — Left · Top = `ratio = share`(새것이 a) · Right · Bottom = `ratio = 1 − share`(§7). 다른 방향은 붙는 칸을 따른다(분할이 그 칸을 감싸므로 저절로).
- `RATIO_MIN`(0.01 — `layout/tree.rs`)은 트리가 나타낼 수 있는 가장 작은 몫이다. 원래 길이가 칸의 1% 보다 작을 때만 걸리고, 미리보기 · 결과가 같은 함수를 지나 둘이 어긋나지 않는다. 상한 밖의 px 최소 · 옆 칸 양보는 없다(R7).
- 예(PRD): 원래 폭 400 → 칸 폭 1200 = 400 · 칸 폭 600 = 300(AC2) · 원래 폭 200 · 300 · 400 → 칸 폭 600 = 200 · 300 · 300(AC39) · 칸 폭 50 = min(원래, 25)(AC39) · 원래 높이 300 · 바깥 위 띠 = min(300, 레이아웃 높이 ÷ 2)(AC44).
- **원래 크기 `original`**(R7 · Q27) — 끄는 창의 캔버스(탭바를 뺀 내용 영역) 크기 · 논리 px. ★캔버스 보고가 아니라 OS 기하에서 셈한다(D4 · E4)★ — 캔버스 보고는 100 ms 디바운스라 셸이 크기를 막 바꾼 창(최대화 복원 · Snap 풀림 · `Unpark` 의 틀 바뀜)에서는 낡았다:
  - **식** — `original = 끄는 창 OS 안쪽 크기 ÷ 배율 − canvas_origin` · 실행기가 platform · 창 사건(`Moved` · `LoopEntered` · `LoopEnded`)마다 끄는 창 `geom`(바깥 · 안쪽 크기 · 배율 · `settled`)을 그 사건을 줄에서 꺼낼 때 읽어 싣고(F3 c — 넣을 때 읽으면 줄에서 기다리는 동안 두 걸음 놓기 · 다시 놓기가 바꾼 크기를 놓친다), `canvas_origin` 은 그 창 크롬 재기다(크기와 무관한 탭바 높이 · 왼쪽 여백 — 대기 중에 잰 값도 그대로 맞다). 캔버스가 뷰포트의 `canvas_origin` 부터 오른쪽 · 아래 끝까지라는 것은 HEAD `WindowLayout` 의 `flex: 1` 칸 그대로다(코드 확인).
  - **떼어진 창** = 떼기 뒤 사건 중 `geom.settled` 인 첫 것으로 `Fixed`(그때 예비 창은 `Unpark` · 두 걸음 놓기를 마쳐 `dragged_size` 바깥 크기 · 커서 모니터 배율이다 — §5-5 · E2 · E3). ★`settled` 가 아닌 `geom`(배율 바뀜이 아직 목표 배율에 자리 잡지 않음 — 그 크기는 tao 의 `WM_DPICHANGED` 크기 고침 · 다시 놓기 전이다)은 건너뛴다(F3 b)★ — 6.2 는 「첫 사건」으로 적어, `PlaceDetached` 자신의 `SetWindowPos` 가 낸 `Moved` 가 옛 배율의 크기로 `original` 을 굳힐 수 있었다. 6.1 의 「`dragged_size` − 명부 캔버스 여백」은 대기 중(Alt+Tab 빼기 비트가 켜진 틀)의 캔버스 재기를 써 틀 바뀜만큼 어긋났다(E2).
  - **창째 끄는 창** = 첫 사건의 `geom` 으로 `Provisional`(셸이 루프 앞에서 놓은 크기 — 최대화면 `dragged_size`) → 루프에 든 뒤 첫 `Moved` 의 `geom` 으로 `Fixed`(OS 가 Snap 을 되돌린 크기 — R7 「움직이기 시작한 뒤」 · T36). 루프에 들기 전에 놓아도(`Released{before_entry}`) 그 사건의 `geom` 으로 적고 판정한다(T37 ① · D4 — 6.1 은 첫 `Moved` 전 놓기에 `original` 이 없었다).
  - `geom` 을 끝내 못 읽으면(창이 사라짐 · OS 호출 실패) 띠를 내지 않는다 — 새 창으로 남는다(T37 · `errors`).
  - `Fixed` 뒤에는 다시 떼어도 · 끄는 중 창 크기가 바뀌어도(Snap) 바꾸지 않는다(R7 · R21).

### 5-5. 떼어진 창 크기 · 자리 — `dock/sizing.rs` (조사 E3 · C4 · C5 · C6)

크기는 전부 **바깥 크기(DIP)** 다 — Chrome 이 쓰는 `GetRestoredBounds` · `GetWindowBoundsInScreen` 이 창 바깥 경계이기 때문이다. 우리 모델의 `attrs.bounds` 는 「물리 바깥 위치 + 논리 안쪽 크기」라(`layout/manager.rs` `WindowBounds` 머리) 그 창의 틀 두께(바깥 − 안쪽)를 더해 바깥 크기로 옮긴다. 제목 표시줄을 없애면(T-66) 틀 두께는 0 에 가까워져 Chrome 과 같아진다.

```rust
pub fn dragged_size(restored: SizeDip, maximized: Option<SizeDip>, work: SizeDip) -> SizeDip;
//   s = restored                                                       — 출처 창 복원(보통) 바깥 크기 · 시작 때 한 번 적은 것(TDC#609-610 initial_window_size_ = GetRestoredBounds().size())
//   ① s.w > work.w 면 s.w = max(0, work.w − 20) · 높이도 같다              — 작업 영역 − 2 × kMaximizedWindowInset(10 DIP)(TDC#2765-2777 · #129)
//   ② maximized = Some(m) 면 s.w = max(m.w / 2, s.w) · s.h = max(m.h / 2, s.h)   — 떼는 창이 최대화면 각 변 ≥ 그 창 절반 · ★마지막이라 이긴다★(TDC#2779-2788)
pub fn work_area_near(monitors: &[MonitorDip], cursor: PhysPoint) -> SizeDip;   // 커서에 가장 가까운 모니터의 작업 영역(C6)
pub fn detached_origin(cursor: PhysPoint, scale: f64, frame_offset: PhysPoint, strip_origin: PointCss, grab: (f64, f64)) -> PhysPoint;
//   = cursor − (frame_offset + (strip_origin + (grab.single_x, grab.y)) × scale)   — 커서 − 끌기 오프셋 · 탭 안 상대 위치 유지
// 규칙 출처 = 조사 E3 한 행(TDC#609-610 · #2760-2788 · #130 · #2913-2934 · #1892-1898 · #1789-1814 · #2749-2757)
pub fn whole_window_origin(start_origin: PhysPoint, down: PhysPoint, cursor: PhysPoint) -> PhysPoint;   // = start_origin + (cursor − down)(C4)
pub const PARK_GAP_PX: i32 = 4000;          // 화면 밖 자리의 모니터 변 거리(물리) — 스파이크 `park_origin` 의 값 그대로
pub fn offscreen_near(monitors: &[MonitorPhys], target: usize, size: PhysSize, s_now: f64, s_target: f64) -> Option<PhysPoint>;
//   겹침을 보는 크기 = size × max(1, s_target ÷ s_now)(F3 d — 배율이 바뀐 뒤 커질 크기) · 후보 = target 네 변 바깥 PARK_GAP_PX 자리(스파이크 `park_origin` 꼴)
//   그 크기로 어느 모니터와도 겹치지 않고 그 사각형 가운데에 가장 가까운 모니터가 target 인 첫 후보 · 없으면 None(E3 두 걸음 ①)
```

- **★6판 기본값 정정(C5)★** — 6판은 「절반 하한 → 작업 영역 상한(상한이 이기게)」으로 두었다. Chrome `CalculateDraggedWindowSize` 는 반대다 — 작업 영역 상한을 먼저, 최대화 절반 하한을 마지막에 적용해 절반이 이긴다(TDC#2760-2791).
- **언제 셈하나** — 떼기마다 그때 셈한다(Chrome 은 떼는 순간 `CalculateDraggedWindowSize(attached_context_)` 를 부른다 — TDC#1817 · 창째 끌기의 복원은 #2750). 바탕 `restored` 만 시작 때 한 번 적은 것이고, 최대화 하한은 **그때 떼는 창**(출처 또는 붙었던 창)으로, 작업 영역은 **그때 커서**로 본다.
- `restored` = 출처 창 모델 `attrs.bounds` 의 크기(복원 자리 — 최대화 중이어도 보통 크기 · 5.8 §11-1 #3) + 그 창 틀 두께 · 없으면 지금 바깥 크기. `maximized` = 떼는 창이 최대화면 그 바깥 크기. `work` = **커서에 가장 가까운 모니터**의 작업 영역(C6 — `GetDisplayNearestPoint(last_point_in_screen_)` TDC#2765-2768). 모니터 목록(Tauri `available_monitors` — 작업 영역 · 배율)에서 순수 함수로 고른다 — Tauri `monitor_from_point` 는 점을 품은 모니터만 돌려주므로(tao `MONITOR_DEFAULTTONULL`) 「가장 가까운」을 따로 셈한다.
- 새 창은 최대화로 만들지 않는다(E3 · #1892-1898). 다시 떼어도 바탕은 처음 크기다(E3 · R21).
- `frame_offset` = 예비 창의 `inner_position − outer_position`(물리 — 제목 표시줄 · 테두리) · `strip_origin` = 예비 창 크롬 재기(명부)의 `tab_bar.x0, y0` · `grab.single_x` = TS 가 셈한 떼어진 창 탭에서의 잡은 x(§4 `Grab`) · `scale` = 커서 모니터 배율. 제목 표시줄 높이를 상수로 두지 않는다(§9). ★`frame_offset` 은 `Unpark` 뒤에 읽는다★ — Alt+Tab 빼기 비트(`WS_EX_TOOLWINDOW`)가 틀을 바꾸므로 실행기가 `PlaceDetached` 를 돌릴 때 그때의 OS 기하로 이 순수 함수를 부른다(§2-3 · §8).
- **★배율이 다른 모니터로 떼기 — 두 걸음 놓기(E3)★** — 예비 창은 주 모니터 배율로 기다린다(§2-5). 한 번의 `SetWindowPos` 로 배율이 다른 모니터에 크기까지 놓으면 OS 가 `WM_DPICHANGED` 를 보내고, tao 는 그 처리기에서 안쪽 크기를 「옛 크기 ÷ 옛 배율 × 새 배율」로 다시 놓는다(tao 0.35.3 `event_loop.rs` `WM_DPICHANGED` 갈래 — 코드 확인) — 새 배율로 이미 셈한 크기가 한 번 더 늘거나 준다. 그래서 `PlaceDetached` 는 실행기 안에서 두 걸음이다: ① 예비 창의 지금 배율이 커서 모니터와 다르면 **크기를 두고 자리만**, 가장 가까운 모니터가 커서 모니터인 화면 밖 자리로 옮긴다(platform `move_window` — `SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER` · 그 자리는 `sizing::offscreen_near(monitors, cursor_monitor, 지금 크기, 지금 배율, 커서 모니터 배율)` 가 스파이크 `park_origin` 꼴로 고른다) — 배율 바뀜과 tao 의 크기 고침이 화면 밖에서 끝난다 ② 그 뒤 틀 두께 · 배율을 다시 읽고 `dragged_size` · `detached_origin` 으로 한 번 놓는다(크기 · 자리 · 보임 · 활성 — 읽기는 묶음을 지을 때가 아니라 ② 를 돌리기 바로 전이다). ★미검★ — 화면 밖 창이 가장 가까운 모니터의 배율을 받는다는 것은 스파이크의 관찰이다(`park_origin` 주석 — 「2.0 배율 모니터 옆에 두면 웹뷰가 DPR 2.0 으로 돌았다」). 그 배율 바뀜이 ① 의 `SetWindowPos` 안에서 동기로 끝나는지는 재지 않았다.
  - **다시 놓기 계기(F3 a)** — 그래서 실행기는 막 뗀 창이 `lp = Posted` 인 동안(그 고리의 `LoopEntered` 를 아직 줄에서 내보내지 않았을 때) 그 창의 **첫 `ScaleFactorChanged`** 가 오고 그 배율이 `PlaceDetached` 가 셈에 쓴 목표 배율과 다르면 ② 의 자리 · 크기를 새 배율로 한 번 더 놓는다(안내 창과 같은 길 — §2-5). ★`Moved` 는 계기로 보지 않는다★ — `PlaceDetached` 자신의 `SetWindowPos` 가 비동기로 오는 `ScaleFactorChanged` 보다 먼저 `Moved` 를 줄에 넣으므로, 6.2 의 「첫 `LoopEntered` · `Moved` 전에 오면」은 늘 깨져 다시 놓기가 돌지 않았다. 커서 모니터가 사방으로 다른 모니터에 둘러싸여 ① 의 자리가 없으면 ① 을 건너뛰고 이 다시 놓기에 맡긴다.
  - **`settled` 표(F3 b)** — 실행기는 막 뗀 창의 `geom` 에 `settled = (그 창의 지금 배율 = 목표 배율)` 을 단다. 세션은 `settled` 인 `geom` 에서만 Detached 의 `original` 을 적는다(§5-4 · T36 ① · T37 ①) — 다시 놓기 전의 크기로 붙는 크기가 굳지 않게.
  - **화면 밖 자리의 크기(F3 d)** — `WM_DPICHANGED` 를 받으면 tao 는 Windows 11 에서 OS 제안 사각형을 그대로 쓴다(tao 0.35.3 `event_loop.rs` `WM_DPICHANGED` 갈래 — 「The suggested position is fine w/o adjustment on Windows 11」 · 코드 확인) — 제안 크기는 배율 비만큼 커질 수 있다. 그래서 `offscreen_near` 는 `size × max(1, s_target ÷ s_now)` 로 겹침을 보고, 후보는 모니터 변에서 `PARK_GAP_PX`(스파이크 `park_origin` 의 4000 px) 떨어진 자리다 — 커진 창이 다른 모니터에 걸쳐 그 배율을 받거나 화면에 비치지 않게.
  - 셋 다 M10 에서 잰다(깜빡임 · 크기 · 자리 · 다시 놓기가 도는지) — 불합격이면 S-f.
- **탭 하나 창을 창째 끌 때**(조사 A9 · C4) — Chrome `Drag()` 그대로(TDC#829-846): 최대화면 복원하고 크기를 `dragged_size` 로(`RestoreAttachedWindowForDrag` — TDC#2749-2758) · 그 뒤 `drag_offset = 누른 점 − (복원한) 창 원점`, 루프 시작 때 창을 `커서 − drag_offset` 에 놓는다(`RunMoveLoop` 의 `SetBounds` — TDC#1797-1814 · #1967-1969). 곧 **창은 복원 원점(또는 지금 원점)에서 누른 뒤 커서가 움직인 만큼 옮겨진 자리**에 서고 커서 밑으로 다시 맞추지 않는다. 최대화가 아니어도 같은 식이다(크기만 그대로) — 문턱 10 px 만큼 창이 따라와 누른 자리의 상대 위치가 지켜진다. 이 판의 셸은 begin 결과에서 한 번의 `RestorePlacement`(최대화) 또는 `PlaceWindow`(보통)로 그 자리를 입힌다(§2-3). ★6판의 「복원 뒤 `detached_origin` 꼴」 · 「보통이면 자리를 바꾸지 않음」은 Chrome 이 아니었다★.

### 5-6. 닫기 모드 — `tabStrip/closingMode.ts` (R26 · 조사 A10 · E5)

```ts
export const CLOSE_SLOP_BELOW_PX = 40   // tci #1576 kTabStripAnimationVSlop = 40 (grounding)
export const CLOSE_SLOP_END_PX = 60     // tci #1580 kTabStripAnimationHSlop = 60 (grounding)
export const CLOSE_LEAVE_DELAY_MS = 300 // 조사 E5(mouse_watcher.cc 를 든 행 — 확실)
export function closingBounds(bar: Rect): Rect        // { x0: bar.x0, x1: bar.x1 + 60, y0: bar.y0, y1: bar.y1 + 40 } — 위 · 시작 쪽 0
export function frozenAvailable(widths: number[], closed: number): number   // Σ(보이는 폭) − closed 폭 + 겹침(0 — 5.8 §4-5)
```

- **들어감** — 마우스로 탭의 닫기 버튼을 눌러 닫았고 그 탭이 보이는 마지막 탭이 아니면 `available` 을 `frozenAvailable` 로 고정한다(`layout.ts` 입력). 맨 끝 보이는 탭을 닫으면 들어가지 않는다 · 모드 중이면 나간다(A10).
- **나감** — 경계 밖에서 누름 = 곧바로 · 경계 밖으로 움직임 또는 창 밖으로 나감 = 300 ms 뒤(그 안에 돌아오면 취소) · 탭바 안에 있는 동안은 시간이 지나도 그대로(마우스에 시간만으로 끝나는 길이 없다 — E5 · 터치 2 초는 쓰지 않는다). 나가면 `available` = 잰 폭.
- **반열림** — 아래 39 · 끝 쪽 59 는 안 · 40 · 60 은 밖(AC40). `bar` = 탭바 줄 사각형(`measure.ts`).
- 키보드로 닫는 길(있다면)은 들어가지 않는다(「마우스로 닫으면」 — A10).

### 5-7. 활성 탭 모양 — `TabBar.tsx` 스타일 (R27 · 조사 A11 · C7)

- 활성 탭 위 모서리 반지름 **10** · 아래 바깥 휘어짐 **12**(탭 상자 밖으로 나가 탭바 밑 화면과 이어진다 — 끝의 귀는 이웃 탭 위에 그리고 포인터 사건을 받지 않는다) · 구분선 **2 × 16**(여백 2)(`tab_style.cc` #22-45 · #97-126 · `horizontal_tab_style_views.cc` #65-280).
- **구분선 투명도**(C7 — 따로 된 애니메이션이 없다): 탭 사이 구분선 하나의 불투명도 = 그 탭이 배경이 보이는 탭(활성 — 우리에게 다중 선택 · 고정 탭 스타일은 없다)이면 **곧바로 0**(전환 없음) · 아니면 `1 − max(호버값(이 탭), 호버값(이웃 탭))` — 이웃이 활성이면 그 이웃의 호버값은 0 으로 친다(「otherwise the separator on this tab will disappear while that tab is being dragged」)(horizontal_tab_style_views.cc#536-552 · #631-648). 호버값은 `GlowHoverController` 의 200 ms 미끄럼이다 — 들어갈 때 **EASE_OUT** `1 − (1 − t)²` · 나갈 때 **EASE_IN** `t²`(glow_hover_controller.cc#39-66 · .h#32 · tab.cc#354 · tween.cc#37-38 · #72-73). CSS = 탭마다 호버값 변수 하나를 200 ms 로 옮기고(들어감 `cubic-bezier(0.3333, 0.6667, 0.6667, 1)` · 나감 `cubic-bezier(0.3333, 0, 0.6667, 0.3333)` — 둘 다 2차 곡선의 정확한 3차 꼴) 구분선이 그 둘의 큰 쪽으로 불투명도를 셈한다. 활성이 바뀔 때는 전환 없이 곧바로 바뀐다.
- 겹침(Chrome 18)은 쓰지 않는다 — 우리 폭 규칙은 겹침 0 이다(5.8 §4-5 · PRD R27).
- 폭 상수 `S` 160 · `A` 42 · `I` 32 는 그대로(5.8 §7 #69 · #73 — `A` 는 T-66 이 새 스타일에서 다시 끌어낸다).

## 6. 끝 · 되돌리기 · 특수 경우

### 6-1. 놓기 — 결과 셋 · 공통 마무리 (PRD R7 · R21 · R22 · A1 · B1)

| 놓은 단계 | 판정 | 할 일 |
|---|---|---|
| Strip(출처 · 붙은 창 어디든) | 판정 없음 — 배치는 이미 모델에 있다(순서는 바뀔 때마다 · 붙기는 붙을 때 썼다). `end.at` 이 모델과 다르면 그 자리로 한 번 쓴다(§4 「순서」) — 셸이 본 뗌 · holder 아닌 창의 키 놓기는 놓기 확인으로 holder 의 `at` 을 받고 1 초 안에 안 오면 지금 모델 자리(T23 · T31 · E7) | 붙은 창이면 그 창 앞으로(`Focus`) → 공통 마무리 |
| Moving · 놓음(루프가 뗌을 봄 · 들어가기 전 뗌 포함) 또는 키 놓기 | **놓은 순간 커서로 지금 세계에서 다시 판정**(R13c · AC35 — 마지막으로 그린 안내가 아니다) → 띠 · 그 밖 둘 중 하나. 탭바 구역은 놓을 때 다시 보지 않는다 — 붙기는 창이 움직일 때(`Moved`)만 일어난다(조사 A7 `OnWidgetBoundsChanged → Drag`) | 띠 = `Write(Seam{anchor, side, share}, Reveal, OnEmptied::Keep)` → 결과(§2-3 `cont = Seam`) → 비운 끄는 창: 새로 뗀 창이면 예비로 · 창째 움직인 창이면 공통 마무리가 닫음 · 메인 마지막 탭이면 §6-4 → 대상 창 `Focus` · 붙은 첫 칸 포커스(R11b — `Reveal`) / 그 밖 = 끄는 창이 그대로 그 자리에 남는다(이미 모델에 있는 보통 창 — R22) · 자리 기록(`record_placement`) → 공통 마무리 |

- **Windows Snap 은 따로 다루지 않는다**(PRD R2 · 사용자 결정 2026-10-18 「극 예외 케이스여서 코드 최소한」) — 끄는 창은 OS 이동 루프라 Snap 이 그대로 따른다(스파이크 Snap — 기본 = 붙음). 놓은 자리가 우리 띠면 위 판정대로 붙고 끄는 창은 예비로 가므로 Snap 결과가 남지 않는다(Snap 덮개가 잠깐 보일 수 있다) · 그 밖이면 Snap 이 정한 크기 · 자리 그대로다. Snap 을 끄는 스타일 비트(스파이크 `--nosnap`)는 쓰지 않는다. 모습은 M16 에서 적기만 한다.

**공통 마무리 `Session::finish(outcome, end_by, reason)`** — 놓기 · 되돌리기 · 쓰기 실패 · 시한 · 창 소멸 · `Abandon` 이 전부 여기로 온다. 6판의 「`Failed` → 남은 할 일 버림 · `Release`」 줄은 꺼낸 예비 창 · 숨긴 끄는 창 · 살아 있는 루프를 놓쳐, 예비 창이 새고 숨긴 창에 탭이 갇혔다(A1 · B1). 이 판은 실패 갈래를 따로 두지 않고, 마무리가 「지금 무엇을 쥐고 있나」만 보고 차례로 되돌린다 — 어느 전이에서 끝나도 같은 함수다:

1. **루프** — 고리가 살아 있으면(`lp` 가 `Posted | Running`) `EndMove`(아직 안 들어간 루프는 들어올 때 삼킨다 — §8) · `lp = Over`. `Ending` 이면 이미 냈다. 그 뒤 오는 `LoopEnded` 는 세션이 없어 버린다.
2. **꺼낸 예비 창** — `spare = Taken`(꺼냈으나 `AdoptWindow` 가 성공하지 않음)이면 `SpareGiveBack` ⓞ · `spare = None`.
3. **숨긴 창** — `hidden` 의 창마다: 모델에 있으면 제자리에서 다시 보인다(`PlaceWindow{rect: None, z: Keep, show: true, activate}` ⓞ — 숨기기 전 자리 · 크기 그대로 · 그 탭이 든 창이면 활성 · F7 b) — 붙기 쓰기가 실패해 탭이 숨은 끄는 창에 남은 경우 · 끄는 창에 남긴 채 놓았는데 Esc 훅이 숨긴 경우(끝 까닭과 무관 — `hid` · F2)가 이것이다 · 모델에 없는 예비 창이면 `SpareGiveBack` ⓞ(보이는 채 화면 밖으로) · 비워 남긴 창(R10)이면 5 로. 다시 보이기가 실패하면 그 창은 명부의 `reveal` 로 간다(F1).
4. **답** — 답하지 않은 begin · leave(`reply`)에 답한다 — begin = `Rejected{"ended"}` · leave = `Ended`.
5. **비워 남긴 창** — `emptied` 중 되돌리기에 쓰이지 않은 것을 `CloseWindow` ⓞ(모델은 이미 뺐다 · 닫기는 모델 뒤 — M2 순서). ★명부에서 빼지 않는다★ — 실행기가 그 창을 `Orphan{Close}` 로 넘기고, 항목은 소멸 갈래가 지운다(§2-5 「고아」 · F1 — 6.2 는 여기서 명부에서 빼 닫기가 실패한 창이 화면 밖에 영영 남았다).
6. `Overlay(None)` · `PollStop` · `Release(lease)` · `Emit(dock:drag{Ended} → armed ∪ 모델 창 전부)`(F6 b — 들였다 되돌린 예비처럼 `armed` 인 명부 창도 · 거울도 0 으로 — §4 `window_role`) · 예비 창이 모델에 남았거나(`spare = Adopted` — 새 창으로 놓음) 잃었으면(`Lost`) `SpareFill`(작업 스레드 · 명부에 예비가 없을 때만 — F1) — 되돌려 놓은 예비(`spare = None`)는 다시 짓지 않는다(G11 · E1) · 끝 로그 한 줄(§10 — 이 묶음 자신의 무른 실패는 끝 줄 뒤라 `errors` 에 들지 않는다 · 실행기 warn · 그 실패가 ⓞ 면 그 창은 이미 명부에 있다 — x7).

- **여기 들어오는 칸** — T08 · T11 · T13 · T14 · T15 · T16(마지막) · T22 · T25 · T27 · T31 · T37 · T40 · T43(창째) · T44(`PollStart` · `ArmRelease` — F4). 어느 칸에서 들어와도 「지금 무엇을 쥐고 있나」만 본다 — 그래서 보상이 칸마다 따로 적히지 않는다.
- **보임 대조(F7 c)** — 실행기는 공통 마무리 묶음을 돌린 뒤 이 끌기가 만진 창(끄는 창 · `hidden` · `emptied` · 예비 · holder · 붙은 창 — 아직 있는 것)마다 tao `is_visible()` 과 OS 보임(platform `is_window_visible` = `IsWindowVisible`)이 같은지 debug 단언한다(release = error 로그 · 끝 줄 `errors` 밖). 숨기기 · 다시 보이기는 날 Win32 라 tao 가 모른다 — 다시 보이지 못한 숨김이 남으면 tao 는 보인다고 믿어 트레이로도 되살릴 수 없으므로(§2-5 「고아」) 여기서 잡는다. 화면 밖에 보이는 채 기다리는 명부 창은 둘 다 참이라 걸리지 않는다. 명부 `reveal` 로 넘긴 창은 대조에서 뺀다(명부가 다시 보인다).
- **[끝 — 탭바 안]** = holder 가 출처가 아니면 `Focus(holder)` 를 앞에 · **[끝 — 띠]** = `Focus(대상 창)` · 붙은 첫 칸 포커스(R11b — `Reveal`) · **[끝 — 따라오는 창]** = 끄는 창 자리 기록(`record_placement`).

### 6-2. Esc · 취소 = 되돌리기 (PRD R12 · R13b · 조사 A8 `RevertDrag` · C8)

`Origin` = 시작 때 적은 `{ 창 O, 자리 i0, 처음 활성 a0(누르기 전), O 의 자리 · 최대화, 탭 하나였나 }`. 되돌리기는 **되돌리기 단계 `RevertStep` 의 차례**다 — 되돌리기를 정한 칸(T24 · T39 · T43)이 그때의 세계로 계획을 한 번 세우고, 쓰기 아닌 단계(`Unpark` · `Restore`)는 할 일로 곧바로 내며 처음 만나는 쓰기에서 멈춘다(`cont = Revert(나머지)`). 쓰기마다 결과(`Wrote`)를 받는 T16 이 결과를 처리하고 다음 단계로 간다 — 비운 예비는 `SpareGiveBack` · `spare = None`(그래야 공통 마무리가 둘째 예비를 짓지 않는다 — G11) · 다른 빈 창은 남긴 창으로 · `main_refilled` 는 적는다(E1 — 6.1 은 되돌리기 결과를 보지 않아 빈 예비가 보이는 채 남거나 `Adopted` 로 남았다). 실패는 끝 줄 `errors` 에 세고 이어 간다(되돌리기는 끝까지 간다) · 마지막에 공통 마무리(`reverted`). 되돌리기의 쓰기는 전부 `OnEmptied::Keep` 이다.

계획(위에서 아래로 처음 맞는 갈래 — 단계는 적힌 차례):

1. **O 가 모델에 있다**
   - 그 탭이 O 에 있으면(탭바 안 · 창째 끌기) → `Write TabBar{O, i0}`(이미 그 자리면 단계를 만들지 않는다) · `Write Switch(O, a0)`(a0 가 있고 지금 활성과 다르면) · 창째 끌기였으면 `Restore(O, 시작 때 자리 · 최대화)`(③).
   - 붙은 창 W 에 있으면 → `Write TabBar{O, i0}`(W 에서 빠진다) · **`Write Switch(W, w0)`** — w0 = W 의 붙기 전 활성 자리를 **끄는 탭을 뺀 W 의 탭 목록**에서 고른 탭(그 자리가 그 목록의 길이보다 작을 때만 — E11 · C8: Chrome 은 끄는 탭을 떼어 낸 뒤의 목록에 붙기 전 선택을 되살린다 · TDC#1748-1760 의 `DetachTabsAndCollectionsForInsertion` 뒤 비교). 계획을 세우는 때는 끄는 탭이 아직 W 에 있으므로 그 탭을 빼고 센다 — 빼지 않으면 끄는 탭이 그 자리 앞에 있을 때 하나 어긋난다 · `Write Switch(O, a0)`.
   - 떼어진 창(예비)에 있으면 → `Write TabBar{O, i0}`(예비가 비어 T16 이 `SpareGiveBack` — 루프 중 Esc 였으면 훅이 이미 숨겼고 `SpareGiveBack` 이 화면 밖에서 다시 보인다 · 조사 E1 · 스파이크 Esc 14/14) · `Write Switch(O, a0)`.
   - ★6판 정정(C8)★ — 6판은 붙은 창 W 에 남는 활성을 승계(`succession`)로 두었다. Chrome 은 W 의 붙기 전 선택을 되살린다.
2. **O 가 비워 남겨 둔 창이다**(탭 하나 창을 창째 끌다 다른 곳에 붙음 · 또는 떼는 사이 다른 경로가 O 의 나머지 탭을 닫음) → `Write AdoptWindow{O, seed, measures}` · 그 `then` = `Unpark(O)` · `Restore(O, 원래 자리 · 최대화, activate)` — ★`Unpark` 뒤에★(E12 — `rcNormalPosition` 의 좌표계가 Alt+Tab 빼기 비트에 따라 바뀐다 · §8) · 쓰기가 성공하면 `emptied` 에서 뺀다(쓰임 — 실패하면 `then` 을 버리고 O 는 남긴 창으로 남아 끝에 닫힌다) · 그 탭이 W 에서 빠졌으면 W 는 위 C8 규칙(`Write Switch(W, w0)`).
3. **O 가 창째 움직이던 그 창이고 루프가 뗌 없이 끝남**(Esc 등 — 1 의 첫 갈래가 내는 `Restore` 의 까닭) → OS 루프가 루프 시작 자리로 되돌린다(스파이크 — 「창 위치는 시작 자리로 복원」). 그 자리는 C4 로 옮긴 자리이므로 `Restore(O, 시작 때 자리 · 최대화)` 로 원래 자리를 입힌다(쓰기 없음 — 단계 하나 뒤 곧바로 공통 마무리). 고리 시작이 실패한 창째 끌기(T43)도 이 갈래다.
4. **O 가 없다**(끄는 중 다른 경로로 닫힘 — R13c ②) → 쓰기 없음. 그 탭은 지금 든 창의 지금 자리에 남는다(떼어진 창이면 그 창 · 붙은 창이면 그 창 — R12 · Q19). 그 창이 숨어 있으면(루프 Esc 의 숨김 — 끝 까닭이 `Ended` 여도 `hid` 로 적혀 있다 · F2) 공통 마무리 3 이 제자리에서 다시 보인다.
5. **메인 마지막 탭**이었으면 §6-4 — 그 탭이 W 에 있으면 `Write TabBar{메인, 0}` · 그 `then` = (`main_refill` 이 아직 빈 탭이면 — G14) `Write CloseTab(메인, main_refill)`(탭이 돌아오지 못했으면 빈 탭을 닫지 않는다) · 그 뒤 `Restore(메인, 시작 자리 · 최대화)`.
6. 남은 단계가 없으면 공통 마무리(§6-1).

- **Esc 가 오는 곳** — 탭바 안 · 출처 = 그 웹뷰 keydown · 붙은 창 = 앱 전체 키 감시 · 이동 루프 = 스레드 Esc 훅(숨기기만 — 끝은 루프가 뗌 없이 끝나서) 그리고 키가 우리 웹뷰에 닿으면 앱 전체 키 감시(§6-3).
- **루프에 들기 전의 Esc**(D2) — `SC_MOVE` 를 보냈으나 아직 안 들어간 사이에 스레드 훅이 Esc 를 보면 platform 은 도착한 `SC_MOVE` 를 삼키고 `Canceled{esc: true}` 를 한 번 낸다(§8 — 6.1 은 그 Esc 로 창을 숨긴 채 루프를 세웠다) → T39 되돌리기. 포커스가 WebView2 자식이라 훅이 못 보면 앱 전체 키 감시의 `End{Cancel, Key}` → T33(`EndMove` — 보냄 상태라 삼킴 · `Ended`) → T39. 어느 길이든 같은 되돌리기다. ★끝냄 청이 먼저 섰는데(T32 키 놓기 · T33 · T36 붙기) 그 뒤 훅이 Esc 를 봐 숨겼으면 끝은 `Ended`(끝냄 청이 앞선다)지만 `hid` 가 참이라 T37 · T38 · T39 가 숨긴 창을 `hidden` 에 적는다(F2 — 6.2 는 `Canceled{esc: true}` 에서만 적어, 숨은 예비가 보통 팝아웃이 된 뒤에도 숨은 채 남았다).★
- **캡처 잃음** — 탭바 안(출처 웹뷰가 `lostpointercapture` 를 받았고 아직 떠나지 않음) = 되돌리기(조사 E8 · PRD R13b · Q29). 이동 루프 중의 캡처 잃음은 보지 않는다 — 루프 끝 알림이 정한다(E8). 떠난 뒤 출처 웹뷰가 늦게 받는 `lostpointercapture`(스파이크 S3 「다음 포인터 입력이 올 때 늦게 온다」)는 버린다 — 그 웹뷰는 떠나는 순간 이 끌기를 놓았고, 셸도 받기 거르개가 holder 가 아닌 창의 캡처 잃음을 버린다(B5).
- **루프가 뗌 없이 끝남**(Esc · Enter 처럼 루프를 끝낸 키 · 잠금 화면 · UAC · 보안 데스크톱 — 조사 E7 · C9) = 되돌리기(`LoopEnded(Canceled)`). 훅 설치에 실패한 루프는 놓기로 친다(E7 「훅 설치 실패면 놓기로 침」).
- **세 번째 창에 붙은 뒤의 Esc** = 끌기를 시작한 창으로(조사 A8 — 「출처를 겨눈다」 추론 · 불확실 — 우리는 PRD R12 그대로 O 로 간다).
- **되돌린 뒤 마우스 버튼을 떼도** 아무 일도 없다 — 세션이 끝났으므로 늦은 사건은 세션 번호로 버린다(AC19 · AC25).

### 6-3. 키 (PRD R12 · AC19 · 조사 E1 · C9)

**Chrome 코드를 단계마다 그대로 옮긴다**(C9 — 부록 A 의 R12 세분):

| 단계 | Chrome | 우리 |
|---|---|---|
| 탭바 안(출처 · 붙은 창) | `EventTracker` — 앱 전체 `EventMonitor` 가 `kKeyPressed` 를 본다: `VKEY_ESCAPE` → `EndDrag(kCancel)` · 그 밖의 키 → `EndDrag(kComplete)` · 키 반복 · 끌기 전부터 쥐던 키를 거르는 코드가 없다(쥔 키의 자동 반복도 끝낸다 — 처음 눌림은 감시가 서기 전이라 못 본다) · 수정키도 끝낸다(TDC#302-336 · #570-576) | 출처 웹뷰 keydown(capture 단계) + 앱 전체 키 감시(`dock:drag{Active}` 동안 모든 우리 창 · G15)가 `tab_drag_end{src: Key}` — `Escape` = `Cancel` · 그 밖 = `Drop`(`at` 은 holder 가 보낼 때만 — holder 아닌 창의 `Drop` 이면 셸이 놓기 확인으로 holder 의 지금 자리를 받는다 · T23) · `e.repeat` 를 거르지 않는다 |
| 따라오는 창(OS 이동 루프) | OS 모달 루프가 키보드를 쥔다. Chrome 의 스레드 `WH_KEYBOARD` 훅은 `VK_ESCAPE` 만 본다(눌림 · 뗌 · 반복 구분 없음) — 늘 DWM 전환을 끄고, 새로 만든 창이면 숨긴다. 루프 결과는 `WM_LBUTTONUP` 을 봤느냐 **하나**로 갈린다 — 봤으면 성공 · 아니면 `kCanceled` → `EndDrag(kCancel)`(hwnd_message_handler.cc#139-160 · #189-213 · #942-961 · desktop_window_tree_host_win.cc#657-679 · TDC#1993-1996 · #2041-2045) | platform 루프 훅은 Esc 만 본다(늘 DWM 전환 끄기 · `hide_on_escape` 면 숨김 — 세션에 알리지 않는다 · E9) · 루프 끝 = 뗌을 봤으면 `Released` · 아니면 `Canceled{esc}` → 되돌리기 · Esc 밖의 키는 루프가 쓴다(화살표 = 창 이동 · Enter = 루프 끝 → 뗌이 없으니 되돌리기 — ★추론★ · M5) · `EventTracker` 는 끌기 내내 살아 있으므로 루프 중에 키가 우리 웹뷰에 닿으면 위 줄 규칙(Esc = 되돌리기 · 그 밖 = 놓기)을 그대로 쓴다 — 닿는지는 Chrome 쪽도 미확인(C9 「unsure」) · M5 |

- **★6판 정정(C9)★** — 6판은 루프 중 훅이 본 「그 밖의 키」를 놓기로 쳤다(`LoopKey(Other)` → 루프를 끝내고 놓기). Chrome 훅은 Esc 만 보고, 루프를 끝낸 키는 뗌이 없어 취소다. 그래서 `LoopKey` 사건을 걷었다. PRD R12 「Esc 밖의 눌린 키 = 놓기」는 탭바 안 단계에서 글자 그대로이고, 따라오는 창 단계에서는 OS 루프가 키를 먼저 쓴다 — 부록 A 의 R12 세분(PRD 문장 고침은 메인 몫).
- **문턱 전**의 키는 끌기와 무관하게 평소대로 간다(E1).
- **끝낸 키가 슬롯에 안 들어가게** — 출처 웹뷰는 문턱을 넘는 순간 `document.activeElement.blur()`(Chrome `SaveFocus` — E1) · keydown 처리기(출처의 capture 단계 · 앱 전체 키 감시)는 끄는 동안 받은 키에 `preventDefault` + `stopPropagation` — 어느 슬롯에도 닿지 않는다.
- **붙을 때 대상 창을 활성으로 만든다**(C10 — TDC#1229-1236 · #1260-1265 · #2033-2039 `attached_context_->GetWidget()->Activate()` — 모든 붙기 갈래). 그래서 붙은 단계의 키는 대개 그 창 감시로 간다. 활성은 platform 으로만 한다(아래 F7 a).
- **★활성은 Alt 합성 없이(F7 a)★** — 붙기 활성(T10 `Focus(W)`) · 공통 마무리의 `Focus` 는 platform `place_window(activate)`(= `SetForegroundWindow`)로만 한다. Tauri `set_focus` 는 tao `force_window_active` 를 부르고, 그것은 `SetForegroundWindow` 가 거절되면 Alt 를 `SendInput` 으로 눌렀다 뗀다(tao 0.35.3 `window.rs` `set_focus` → `force_window_active` — 코드 확인) — 끄는 동안 앱 전체 키 감시는 그 Alt keydown 을 「그 밖의 키 = 놓기」로 읽어 끌기를 끝낸다. 전경이 거절되면 그대로 둔다(끄는 동안 우리 앱이 전경이라 드물다 · M3 · M5 에서 본다). U4 의 `rg` 가 끌기 어댑터 · `dock/` 에 `set_focus` 가 없는지 본다.
- **★키 감시 수명(F6)★** — 웹뷰의 앱 전체 키 감시는 ① `dock:drag{Active(s)}` 로, 또는 마운트 때 `window_role` 의 `drag = s` 로 켜되 `s > lastEnded` 일 때만이다(`dock:drag` 듣기를 먼저 건 뒤 묻는다 · 세션 번호는 1 부터 단조 — §4) ② `dock:drag{Ended(s)}` 로 끄고 `lastEnded = max(lastEnded, s)` 를 적는다 ③ 키 끝(`tab_drag_end{src: Key}`)을 하나 보내면 스스로 끈다 — 그 끌기는 대개 그 키로 곧 끝난다(셸이 x2 · MK · MV 의 x1 로 버려도 끝은 이미 정해져 있다 · ⊘ 는 세션이 없을 때뿐이다). ★예외 하나 — 붙기를 정한 뒤(MA)의 키는 x1 로 버려지고 끌기는 붙은 창에서 이어진다★: 붙는 창 W 는 `dock:feed{Attach}` 로 감시를 다시 켜고(활성도 W 로 간다 — C10) 다른 창의 감시는 꺼진 채 남는다 — 붙기를 정한 짧은 틈에 누른 키 하나의 대가로 받아들인다(M5). 셸은 끝에 `Ended` 를 `armed ∪ 모델 창` 에 보낸다(들였다 예비로 되돌린 창도 받는다). 6.2 는 ⓐ 다시 읽힌 웹뷰가 답보다 먼저 온 `Ended` 를 지나 옛 번호로 켜거나 ⓑ 명부로 돌아간 예비가 `Ended` 를 못 받아, 끝난 끌기의 감시가 켜진 채 남아 그 창 키보드가 먹힐 수 있었다.
- **창째 끌 때 포커스가 WebView2 자식에 있으면**(근거 없음 b · 미검) — 웹뷰 자식 HWND 는 다른 프로세스의 스레드라 메인 스레드의 스레드 지역 `WH_KEYBOARD` 가 Esc 를 못 볼 수 있다(스파이크의 Esc 14/14 는 떼어 활성으로 놓은 예비 창이었다). 그때 Esc 가 그 웹뷰에 닿으면 앱 전체 키 감시가 `Cancel` 을 보내 같은 되돌리기가 된다(위 표 · 행렬 MP · MR 의 `End{Cancel, Key}` 칸 = T33 — §2-3). 훅도 감시도 Esc 를 못 받으면 Chrome 의 「Esc = 되돌리기」가 이 단계에서 서지 않는 것이다 → §15-2 S-e(대안을 짓지 않는다) · M5.
- **실측으로 볼 것**(§12 M5) — 탭바 안의 글자 · 수정키 · 키 반복 · 끌기 전부터 쥔 키 · 루프 중 화살표(창 이동) · Enter(뗌 없는 루프 끝 → 되돌리기인지 — 추론 확인) · 루프 중 글자 키가 우리 웹뷰 감시에 닿는지 · 창째 끌기의 Esc(근거 없음 b) · 끝낸 키가 어느 슬롯에도 안 들어가는지 · IME 조합 중 문턱을 넘을 때(`blur` 가 조합 글자를 확정해 넣는지) · Alt-Tab · Win 키(앱에 안 닿는다 — 끌기가 끝나지 않거나 루프가 OS 에 끊겨 `Canceled` → 되돌리기) · 붙기 활성 · `Focus` 뒤에 키 감시가 놓기를 받지 않는지(F7 a) · 다시 읽힌 창 · 예비로 되돌린 창의 키보드가 끌기 뒤에 살아 있는지(F6).

### 6-4. 탭 하나 창 · 메인 마지막 탭 (PRD R2 · R14 · 조사 A9 · C4)

- **탭 하나 창**(`Begin` 때 그 창 모델 탭 수 = 1) — 떼지 않고 그 창이 이동 루프로 따라온다(`WholeWindow` · A9). 루프 앞에서 Chrome `Drag()` 그대로 놓는다 — 최대화면 복원하고 크기를 `dragged_size` 로, 자리는 「복원(또는 지금) 원점 + (커서 − 누른 점)」(§5-5 · C4). `hide_on_escape = false` — Esc 면 OS 가 루프 시작 자리로 되돌리는 모습이 그대로 보이고, 그 뒤 셸이 원래 자리 · 최대화를 입힌다(§6-2 ③ · Chrome 의 숨기기는 새 창일 때만 — E1).
  - 다른 탭바에 붙으면 그 창은 빈다 → 숨겼다가(§2-5) 끝에 닫는다(R10) · 되돌리면 되살린다(§6-2 ②).
  - 띠에 놓으면 그 칸에 붙고 그 창은 닫힌다(AC26).
  - 그 밖에 놓으면 그 창이 놓은 자리에 있다 — 모델은 바뀌지 않는다(자리만 기록).
- **메인 마지막 탭**(R14 · Q28 — 메인 제안 · 4.8 리뷰 1 · 2) — 같은 `WholeWindow` 로 메인 창이 따라온다. 다른 점만:
  - **붙는 순간**(탭바 · 칸 띠) — 메인은 숨기지 않고 루프만 끝낸다 → 쓰기 결과의 `main_refilled = Some(빈 탭)` 을 세션에 적는다(`refill_main` — 지금 규칙 · ADR-0057 ④) → `RestorePlacement(메인, 시작 자리 · 최대화, 뒤 = 붙은 창, 활성화 안 함)`(§8) — 메인은 앞으로 오지 않고 붙은 창 뒤로 간다.
  - **판정 · 750 ms** — 늘 커서 밑에 실제로 있는 창(z 순서)으로 한다. 메인이 돌아가며 낸 `Moved` 는 끄는 창의 것이 아니라 세지 않는다(R14 · R15).
  - **다시 떼어지면** 보통 떼기(예비 창) — 메인은 그대로 있다.
  - **메인 탭바에 다시 붙으면** — 커서 밑 맨 위 우리 창이 메인이고 그 탭바 구역일 때만(가려졌으면 그 자리의 맨 위 창이 정한다 — R15 · AC38) → `TabBar{메인, 0}` 결과 뒤 `CloseTab(메인, main_refill)`(G14 의 조건 — 그 탭이 아직 빈 탭일 때만) · `main_refill = None`.
  - **Esc** — 그 탭을 메인으로 · `main_refill` 탭 닫음(G14 조건) · 메인 원래 자리 · 최대화(§6-2).
  - **최대화를 활성화 없이 되돌리기**가 Win32 에서 되는지 재지 않았다 → §12 M6 · 안 되면 §15-2 S-c(Chrome 동작이 아니라 우리 확장의 메인 제안이라 사용자에게 다시 묻는다).

### 6-5. 끄는 중 배치가 바뀌면 (PRD R13b · R13c · AC16 · AC35 · B4)

`WorldChanged` 마다 세계를 다시 뜨고, 창이 OS 에서 사라지면 `WindowGone` 이 온다(행렬의 두 행 — 아래는 그 칸들의 뜻):

| 단계 | 바뀐 것 | 결과 |
|---|---|---|
| Strip | 그 탭을 쥔 창(holder — 출처 또는 붙은 창)이 닫힘 · 그 탭이 닫힘 | 끝(T27 · `holder_gone` — 되돌릴 곳이 없다 · 그 탭은 그 창과 함께 닫혔다 · R13c ① ③) · 표식은 `remove_view` 가 이미 내렸다 |
| Moving · Detached | 원래 창 O 가 닫힘 · 바뀜 | 그대로 이어감(T41 · `WindowGone` 은 x4 · R13c ②) — 되돌리기의 「O 없음」 갈래가 이것을 받는다 |
| Moving · Detached | 그 탭이 닫힘(LLM `close_tab`) | 그 탭은 끄는 창(쓰던 예비 창)의 마지막 탭이라 `close_tab` 이 `CloseTabOutcome::WindowClosed` 로 `host.close` 를 부른다(`layout/apply.rs` `close_tab`) — **끄는 창이 루프 안에서 사라진다**. platform 끝 알림이 `Destroyed` 로 정확히 한 번 오고(§8) 셸 `Destroyed` 도 `WindowGone` 으로 온다 → 먼저 온 쪽에서 끝(T40 · `aborted` · `holder_gone`) · 예비 창을 잃었으므로 공통 마무리가 `SpareFill`. ★6판의 「루프 끝 · 그 창은 예비로」는 틀렸다 — 그 창은 닫힌다★ |
| Moving · WholeWindow | 그 창이 닫힘(LLM `close_window`) · 그 탭이 닫힘 | 같은 길 — 루프 안 창 소멸 → `Destroyed` → 끝(T40 · `aborted`) |
| Moving | 대상 · 끄는 탭 자신의 배치 | 마지막 커서로 다시 판정해 안내를 곧바로 다시 그림(T41 · R4 · R13c · AC16 · AC35) — 1 프레임 한계는 받아들임(R4) · 붙기는 `Moved` 에서만 |
| 모두 | 비워 남긴 창(명부 `Kept`)이 사라짐 | `emptied` 에서 뺌(T28) — 되돌리기는 그 창을 「O 없음」으로 본다 |
| 모두 | 그 밖 | 아무것도 하지 않음(T29 · T41 — 새로 든 모델 창에 `dock:drag{Active}` 를 보내는 것 밖에 · Q19) |

- 표식(`Dragging`)은 **옮기기**(`dock`)만 막는다 — 다른 경로(LLM)가 끄는 탭을 옮기려 하면 거절되지만(D9 · 지금 규칙), 닫기(`close_tab` · `close_window`)는 막지 않는다. 그래서 위 「닫힘」 줄들이 실제로 일어난다.

### 6-6. 저장 · 원자성 · 출력 연속 (PRD R11 · R11a · AC7 · AC11 · AC37 · AC42)

5.8 §5 가 그대로 선다 — 바뀐 것은 「놓을 때 한 번」이 「바뀔 때마다 한 번씩」이 된 것뿐이다(Q30).

- **바뀜 하나 = `ViewManager::dock` 한 번 = 레이아웃 락 한 번 · `version` 한 번** — 순서 이동 · 떼기 · 붙기 · 띠 · 되돌리기의 각 단계가 그렇다. 관찰자(T-65 `layout.get` 포함)는 끄는 도중의 상태를 볼 수 있다(Q30 — 붙는 순간 첫 자리 0 에 잠깐 선 것도 보인다 · C1). 메인 스레드 세션이 사건을 하나씩 끝까지 처리하므로 경계를 빠르게 오가도 바뀜이 겹치지 않는다(AC37).
- **저장** — 기록기가 락 안에서 통째로 복사하므로 `state.json` 은 어느 바뀜의 앞이거나 뒤다 — 그 탭은 정확히 한 곳(AC11). 끄는 중 꺼지면 꺼질 때의 자리(떼어진 창이면 그 창 — 모델의 보통 팝아웃이다)에 다시 뜬다(R11a). 예비 창 · 안내 창은 모델 밖이라 저장되지 않고, 비워 남겨 둔 창은 모델에서 이미 빠져 다시 뜨지 않는다.
- **출력 연속** — 같은 락 안의 `subs.resync` 가 라우팅을 옮긴다(5.8 §5 그대로). 그 에이전트는 늘 어느 창에든 보여 데몬 구독이 끊기지 않는다. 예비 창 웹뷰는 지어질 때 이미 자기 label 로 출력 구독을 걸어 두므로(`PopoutPage` 의 `TauriTransport` — 연결 때 등록 · 대기 모드에서도 그대로) 그 탭이 들어오면 곧바로 라우팅된다 — U4 시험이 이것을 잰다(AC7 · AC42).
- **자리 기록** — `on_window_event` `Moved` · `Resized` → `placement::record` 는 모델 창만 적는다 — 명부 창 · 안내 창 label 은 건너뛴다(§2-5 · U4).

## 7. 레이아웃 연산 변경 (`layout::apply`)

**살리는 것**(5.8 §1-2 — 그대로이고 여기 되풀어 적지 않는다): `docked_tree` 의 분할 모양 · 슬롯 id · 분할 id 를 그대로 옮김 · 새 면적 0 판정(R8) · `incoming_rect` · `ViewManager::dock` 의 판정 순서(계약) · 표식(lease) 한 번에 하나 · 닫히는 창 = 모델 먼저 + 묘비(M2) · 메인이 비면 빈 탭(`refill_main`) · 알림 묶음 끝 `settled` · `DockError` 의 종류(★U1 이 `InvalidShare` 하나를 더했다 — 판정 7 의 맨 앞: `share` 가 유한하지 않거나 `RATIO_MIN..=0.5` 밖이면 값으로 거절★) · 창 포트에 기본 본문 없음. 지금 `dock_tab` 의 운영 호출자는 `commands/dock.rs` 의 `TauriDock::apply` 하나다 — 옛 장부(`dock/book.rs`)가 낸 `Action::Apply(DockRequest)` 를 옛 실행기가 이것으로 돌린다(코드 확인 — HEAD 의 `apply::dock_tab` 호출은 그것 밖에 `commands/popout.rs` · `tests/layout_apply.rs` 의 시험뿐 · B12).

**바꾸는 것**(여섯 — 전부 끌기 하나가 모델을 여러 번 쓰게 되어서다 · Q30):

```rust
// layout/dock.rs — 새것 몫을 인자로(R7 · Q25). DOCK_NEW_SHARE(1/3) 는 U8 에서 걷는다.
pub fn docked_tree(target: &LayoutNode, anchor: DockAnchor, side: DockSide, incoming: &LayoutNode, share: f64)
    -> Result<LayoutNode, DockTreeError>;   // Left·Top: ratio = share · Right·Bottom: ratio = 1 − share · RATIO_MIN ≤ share ≤ 0.5

// layout/manager.rs
pub enum DockTo {
    Seam { target: ViewId, anchor: DockAnchor, side: DockSide, share: f64 },
    TabBar { window: WindowLabel, index: Option<usize> },
    NewWindow { label: WindowLabel, seed: Option<WindowSeed>, measures: FrameMeasures },  // seed = 되살리는 창 항목(되돌리기)
}
pub struct WindowSeed { pub window_id: String, pub attrs: WindowAttrs, pub(crate) placement_memo: PlacementMemo }   // 자리 기억 = 비운 동안 끊기지 않게(크레이트 밖에서는 못 만든다)
pub struct FrameMeasures { pub canvas: Option<CanvasPx>, pub metrics: Option<UiMetrics>, pub chrome: Option<DockChrome> }   // B2
pub struct EmptiedWindow { pub label: WindowLabel, pub seed: WindowSeed, pub measures: FrameMeasures }
pub struct DockGuard { pub lease: Option<LeaseId>, pub keep_lease: bool, pub expect_source: …, pub expect_target: … }
pub struct DockMove { pub successor: Option<ViewId> }               // 옮긴 탭이 떼는 창의 활성이었을 때 다음 활성(아래 ④)
pub struct Docked { window, slot_ids, source_window,
                    emptied: Option<EmptiedWindow>,                // 빈 팝아웃 항목을 지웠으면 그 label · seed · 재기(지금 source_window_closed 를 대신)
                    main_refilled: Option<ViewId> }                // 메인이 비어 새로 만든 빈 탭

// layout/apply.rs
pub enum DockDest {
    Seam { target, anchor, side, share: f64 },
    TabBar { window, index },
    NewWindow { at: WindowPlace },                                 // 창을 연다 — T-65 · 팝아웃 몫(그대로) · 메인 스레드 · 동기 명령 금지
    AdoptWindow { label: WindowLabel, seed: Option<WindowSeed>, measures: FrameMeasures },  // 이미 열린 모델 밖 창(예비 창 · 비워 남긴 출처)을 그 탭의 창으로 — 창을 열지 않는다
}
pub enum OnEmptied { Close /* 지금 — host.close */, Keep /* 끄는 중 — 닫지 않고 결과로 돌려준다(R10) */ }
pub struct DockRequest { view, to: DockDest, guard: DockGuard, focus: FocusPolicy, mv: DockMove, on_emptied: OnEmptied }
pub struct DockOutcome { window, slot_ids, emptied: Option<EmptiedWindow>, closed_window: Option<WindowLabel> /* Close 일 때만 */,
                         main_refilled: Option<ViewId>, reused: bool }

// dock/ports.rs — 세션이 쓰는 쪽(B9)
pub enum SessionDock { Seam { .. }, TabBar { .. }, AdoptWindow { .. } }   // NewWindow 가 없다 — 어댑터가 DockDest 로 옮긴다
```

1. **붙는 몫** — `Seam.share` 를 `docked_tree` 로 넘긴다. `valid_bands` · 미리보기도 같은 몫으로 부른다(§5-3 · §5-4).
2. **`AdoptWindow`** — 락 한 번 안에서 `ViewManager::dock(NewWindow{label, seed, measures})` 만 한다(지금 `open_new_window` 의 phase C 와 같은 판정 — 묘비면 `WindowClosed` · 이미 모델에 있으면 `WindowExists`). `seed` 가 있으면 새 창 항목의 `window_id` · `attrs` 를 그것으로 둔다(되돌리기로 되살린 창이 영속 id · 테마 · 자리를 잃지 않게 — T-66 R6 의 영속 id 와도 맞는다). 그 `window_id` 를 산 창이 이미 쓰면 새 id 를 뽑고 경고를 남긴다(같은 id 로 저장하면 다음 부팅이 한쪽을 탭째 버린다 — 신원을 잃고 탭을 지킨다). **`measures` 를 새 창 항목의 `canvas` · `metrics` · `chrome` 에 같은 락에서 담는다**(B2 — 모델 밖에서 받아 둔 재기 · §2-5). 창을 열지 않으므로 메인 스레드에서 불러도 된다(§2-4).
3. **표식 이어 쥐기** — 지금 `dock` 은 성공하면 표식을 내린다. `keep_lease = true` 면 내리지 않는다 — 한 끌기의 여러 쓰기가 같은 표식을 쓴다. 끝에 세션이 `release_lease` 한다. `keep_lease = false` 는 지금 그대로(T-65 · 팝아웃).
4. **떼는 창의 다음 활성**(조사 E2 · C8) — 옮긴 탭이 떼는 창의 활성이었고 그 창이 남으면, `successor` 가 남은 탭에 있으면 그것을 · 아니면 지금 `succession`(오른쪽 이웃 → 왼쪽)을 활성으로. 세션이 싣는 값:
   - **떼는 창에 붙었다가 떼는 것이면**(`before_attach` 가 그 창 것 — 다른 창 W 든, 돌아와 붙은 출처 O 든) = 그 창의 **붙기 전 활성 자리**의 탭 — 그 자리가 남은 탭 수보다 작을 때만(Chrome 은 붙을 때 `selection_model_before_attach_` 를 적고 뗄 때 그 자리(index)로 되살린다 · TDC#1514-1515 · #1745-1757 — 이 갈래가 먼저다). 세션이 떼는 때의 세계에서 그 자리를 탭 id 로 옮겨 싣는다 — ★「남은 탭」 = 끄는 탭을 뺀 그 창의 목록이고 자리도 그 목록에서 센다★(E11 — Chrome 은 끄는 탭을 떼어 낸 뒤 그 목록 길이와 비교한다 · TDC#1748-1760 · 끄는 탭이 든 목록으로 세면 끄는 탭이 그 자리 앞에 있을 때 하나 어긋난다 · §6-2 의 `Switch(W, w0)` 도 같은 규칙). ★6판 정정(C8)★ — 6판은 여기를 승계로 두었다.
   - **아니고 떼는 창이 출처 O** = `initial_active`(누르기 전 활성 — 끄는 탭이 아니면 · TDC#1729-1743 「make sure the tab that was initially active is selected」 · #1745-1771 의 `RestoreInitialSelection` 갈래).
   - 이미 활성이던 탭을 끌어 `None` 이면 → `succession` = Chrome `DetermineNewSelectedIndex` 에서 우리에게 없는 opener · 그룹 규칙을 뺀 나머지(오른쪽 이웃 · 마지막이면 왼쪽 — E2 · 가능성 높음).
5. **비운 창 남기기**(R10) — `OnEmptied::Keep` 이면 빈 팝아웃의 모델 항목은 지금처럼 지우되(`pull_tab`) `host.close` 를 부르지 않고 `emptied = {label, seed(지운 항목의 window_id · attrs), measures(지우기 직전 그 항목의 재기)}` 로 돌려준다. ★그 창에 탭 0 개 `window:tabs-updated` 를 보내지 않는다★ — 지금 `WindowClosed` 갈래가 emit 을 비워 두는 것과 같게(웹뷰의 0탭 자가닫힘이 남긴 창 · 되돌린 예비를 닫지 않게 · §2-5 · U1 시험). 세션이 그 창을 예비로 되돌리거나 · 되돌리기용으로 남기거나 · 끝에 닫는다(§2-5). `Close` 는 지금 그대로.
6. **메인 빈 탭 id** — `refill_main` 이 만든 탭 id 를 `main_refilled` 로 돌려준다(R14 — 다시 붙거나 되돌릴 때 그 탭을 닫으려고).

- **`dock_tab` 계약 문서(B9)** — 지금 문서 주석(「★메인 스레드 · 동기 명령 안에서 부르지 말 것★ — 새 창 빌드가 교착한다」)을 갈래별로 고쳐 쓴다: **`NewWindow` 갈래만** 메인 스레드 · 동기 명령에서 부르지 말 것(새 창 빌드 교착 — 그대로) · `Seam` · `TabBar` · `AdoptWindow` 는 창을 열지 않아 어느 스레드에서나 부를 수 있다(`OnEmptied::Close` 의 `host.close` 는 창을 닫기만 한다). 세션 포트는 `SessionDock` 만 받아 `NewWindow` 를 타입에서 뺀다.
- **바뀌는 호출자**(B8 — `git grep` 으로 셈 · §13 U1): 운영 = `commands/dock.rs` `TauriDock::apply`(인자 통과) · `dock/book.rs` 의 `Action::Apply(DockRequest{…})` 를 짓는 곳(지금 동작 값: `share` 1/3 · `keep_lease` false · `successor` 없음 · `Close`) · `dock/hit.rs` 의 `docked_tree(…, incoming)` 부르는 곳(`DOCK_NEW_SHARE` 를 넘김) · 시험 = `commands/dock.rs`(`DockOutcome` 짓는 곳 넷) · `commands/popout.rs`(`DockTo` · `DockRequest` 짓는 시험) · `dock/book.rs` · `dock/hit.rs` · `dock/regions.rs`(`docked_tree` 시험) · `layout/{manager,dock,apply}.rs` · `tests/layout_apply.rs`. 창 포트 가짜들은 바뀌지 않는다 — 새 포트 메서드가 없다. `DockDest` 를 고르는 `match` 는 HEAD 에서 전부 `other =>` 갈래가 있어 새 변이로 깨지지 않는다. ★`Eq` 를 뺀다(E8)★ — `DockTo`(HEAD `manager.rs` 의 `#[derive(…, PartialEq, Eq)]`) · `Docked` · `DockOutcome`(HEAD `apply.rs`)은 `Eq` 를 derive 하는데 새 칸 `share: f64` · `FrameMeasures`(`DockChrome` 은 `PartialEq` 만) · `WindowSeed`(`WindowAttrs` 는 `PartialEq` 만)가 들어가면 derive 가 깨진다 — U1 이 셋에서 `Eq` 를 빼고 `PartialEq` 만 둔다(`assert_eq!` 는 그대로 된다 · `Eq` 경계로 쓰는 곳이 있으면 빌드가 잡는다).
- **같은 창 같은 자리 `TabBar`** — 지금처럼 무변경 · `version` 그대로 · 단 `keep_lease` 면 표식을 그대로 둔다.
- **세션이 내는 쓰기의 포커스** — 순서 · 떼기 · 붙기 · 띠는 `FocusPolicy::Reveal`(옮긴 탭을 보이고 띠면 끼운 첫 칸에 포커스 — R11b · 지금 규칙) · 되돌리기는 `Keep` 으로 옮긴 뒤 `switch_tab` 으로 처음 선택을 되살린다(§6-2).
- **`plan_new_window` 의 `Reuse`**(탭 하나 팝아웃을 그 창째 옮기기 — 5.8 Q19) — 마우스는 이제 창째 이동 루프(§6-4)라 이 갈래를 쓰지 않는다. T-65 의 「새 창으로」 명령 몫으로 그대로 남긴다.
- **LLM 길**(PRD R16) — 마우스의 모든 결과(순서 · 떼기 · 붙기 · 띠 · 되돌리기)가 위 칸들을 단 `dock_tab` 하나를 지난다. T-65 는 같은 함수에 `keep_lease = false` · `OnEmptied::Close` · `FocusPolicy::Keep` 으로 명령을 얹는다 — 두 번째 변경 경로가 없다.

## 8. platform 목적 함수 (ADR-0266)

OS 에 따라 달라지는 일은 전부 `crates/engram-dashboard-platform` 의 목적 함수로만 들어간다. 부르는 쪽(셸 `commands/dock.rs` 의 어댑터)은 OS 를 모른다. **창은 `raw-window-handle` 0.6 의 `&impl HasWindowHandle` 로 넘기고**(Tauri `WebviewWindow` 가 구현한다 — `tauri` 2.11.3 `webview_window.rs` 의 `impl HasWindowHandle`), **그 핸들의 변이 맞추기(Windows = `RawWindowHandle::Win32` · 그 밖 = `Unsupported`)는 platform 안에서 한다**(B10 — 셸이 변이를 맞추면 그것도 platform 밖의 런타임 OS 가름이다 · 게이트 ④ 의 「`cfg` 없는 런타임 OS 가름은 리뷰가 본다」). Tauri 는 platform 에 들이지 않는다(게이트 ②). U3 전에는 platform 의 창 함수가 `window::foreground_is_current_process` 하나뿐이었고 커서 · `SetWindowPos` · 훅 · 캡처를 다루는 코드가 저장소 어디에도 없었다(코드 확인 2026-10-18 — `rg` 0 건).

```rust
// 창 정체 — 비교 · 건너뛰기 · z 순서 기준에 쓴다. 셸은 안을 모른다(Windows = HWND).
pub struct NativeWindow(/* platform 안 */);
impl NativeWindow { pub fn of(w: &impl HasWindowHandle) -> Option<NativeWindow>; }

// move_loop.rs (새) — Windows 창 이동 루프(조사 A6 · 스파이크 S3 · C9)
pub struct MoveLoopOpts { pub hide_on_escape: bool }              // 새로 뗀 창이면 true(조사 E1 — Esc 때 숨김 · DWM 전환 끄기는 깃발과 무관하게 늘 — E9)
pub enum LoopEvent { Entered, Ended { end: MoveLoopEnd, hid: bool } }   // Ended 는 어떤 길로든 정확히 한 번
//   hid = 이 고리의 Esc 훅이 그 창을 숨겼다(`hide_on_escape` 이고 Esc 를 눌림 · 뗌 · 반복 어느 꼴로든 봤다 — Posted · Running · Ending 어느 때든) · 끝 까닭과 무관하게 모든 끝에 싣는다(F2)
pub enum MoveLoopEnd { Released { before_entry: bool }, Ended, Canceled { esc: bool }, Destroyed }
pub fn begin_move_loop(w: &impl HasWindowHandle, opts: MoveLoopOpts,
                       on_event: Box<dyn FnMut(LoopEvent)>) -> Result<(), MoveLoopError>;   // Err = Unsupported · NoWindow · WrongThread · Busy(한 스레드에 루프 하나) · Os — 어느 것이든 Failed(Start) → T43 · Err 면 알림이 하나도 없다
//   Ok 여도 돌아오기 전에 · 그 창을 부수는 호출 안에서 `Ended` 가 올 수 있다 — 받는 쪽은 재진입 가능해야 한다(포트가 자기 상태를 빌린 채 platform 을 부르지 않는다)
pub fn end_move_loop(w: &impl HasWindowHandle);                   // 상태에 따라(아래 표) — 끝 알림은 Ended

// window.rs (더함)
pub struct PhysPoint { pub x: i32, pub y: i32 }                   // platform 의 물리 px 타입(셸 의존 없음) — 셸 어댑터가 `dock/units.rs` 타입과 옮긴다(E8 · U3 착지)
pub struct PhysRect { pub x: i32, pub y: i32, pub width: i32, pub height: i32 }
pub enum ZOrder { Topmost, Top, After(NativeWindow), Keep }
pub fn place_window(w: &impl HasWindowHandle, rect: Option<PhysRect> /* None = 자리 · 크기 그대로 */, z: ZOrder, show: bool, activate: bool) -> Result<(), WindowError>;
pub fn hide_window(w: &impl HasWindowHandle) -> Result<(), WindowError>;       // ShowWindow(SW_HIDE) — 캡처를 쥐지 않은 창에만(스파이크 S2)
pub fn is_window_visible(w: &impl HasWindowHandle) -> bool;                     // IsWindowVisible — tao 보임과 대조(F7 c · §6-1)
pub fn top_level_at(p: PhysPoint, skip: &[NativeWindow]) -> Option<NativeWindow>;
pub fn describe_window(w: NativeWindow) -> Option<WindowFacts>;  // pid · 클래스 · 확장 스타일 · layered 꼴(LayeredKind) · RTL · 창 모양 유무 — 셸 probe 의 debug 로그 몫(메시지를 보내지 않는 조회뿐)
pub fn move_window(w: &impl HasWindowHandle, origin: PhysPoint) -> Result<(), WindowError>;      // 크기 · z · 활성은 두고 자리만(E3 두 걸음 ①)
pub fn restore_placement(w: &impl HasWindowHandle, normal: PhysRect /* 화면 좌표 */, maximized: bool, after: Option<NativeWindow>, activate: bool) -> Result<(), WindowError>;
pub fn set_switcher_hidden(w: &impl HasWindowHandle, hidden: bool) -> Result<(), WindowError>;   // Alt+Tab 에서 빼고 넣기(B7 · E2)

// input.rs (새)
pub fn cursor_pos() -> Option<PhysPoint>;                          // GetCursorPos — 어느 스레드에서나
pub fn primary_button_down() -> bool;                              // GetAsyncKeyState — 단추 바꿈(SM_SWAPBUTTON)이면 오른쪽
```

| 함수 | 하는 일 | 근거 |
|---|---|---|
| `begin_move_loop` | 부르는 스레드 = 그 창의 스레드(메인). ① `SetWindowSubclass` 로 그 창의 시작 신호 도착(④) · `WM_ENTERSIZEMOVE` · `WM_SYSCOMMAND` 반환(= 루프 끝) · `WM_NCDESTROY` 를 지켜본다 ② 스레드 지역 `WH_MOUSE`(`WM_LBUTTONUP` 봄) · `WH_KEYBOARD`(`VK_ESCAPE` **만** — C9) ③ `ReleaseCapture`(웹뷰가 쥔 캡처를 푼다 — 호스트 주 스레드와 웹뷰 UI 스레드가 입력 큐를 나눈다 · 스파이크 S3) ④ 루프 번호를 실은 사설 시작 신호(`RegisterWindowMessage` 의 등록 메시지)를 `PostMessage` 하고 곧바로 돌아온다 — 상태 = **Posted** · 펌프가 그 신호를 나르면 아래 Posted 확인 뒤 `SendMessage(WM_SYSCOMMAND, SC_MOVE \| 0x0002, 커서)` 로 루프를 세운다(스파이크 S3 — IPC 처리기 안 동기 시작은 루프 안 IPC 왕복 0/21 · §1 스택 맞춤 ②). ★`SC_MOVE` 를 직접 올리지 않는 까닭(U3 코드 리뷰 C2)★ — 웹뷰의 `app-region: drag` 도 같은 창에 같은 값 `0xF012` 를 보내므로(스파이크 S6) 값 · 자리로는 「이 루프의 것」을 못 가르고, 잘못 집으면 우리가 올린 것이 나중에 추적 없는 루프를 세운다. 그래서 「이 루프의 것」 = 그 사설 신호이고, 서브클래스는 `SC_MOVE` 를 보내는 동안 선 in-flight 표로 처음 닿는 `WM_SYSCOMMAND` 하나만 집는다 · 남의 `SC_MOVE` 는 건드리지 않는다 | 조사 A6(`SC_MOVE\|0x0002` — hmh #951 grounding · 훅 hmh #91-200) · 스파이크 S3 `post` |
| **상태와 끝**(B3 · B4 · 근거 없음 a) | **Posted** — 올린 시작 신호가 서브클래스에 도착하면: `end_move_loop` 를 이미 불렀으면 `SC_MOVE` 를 보내지 않고(루프가 아예 안 선다) `Ended` · Esc 훅이 이미 Esc 를 봤으면 보내지 않고 `Canceled{esc: true}`(D2 — 훅은 `hide_on_escape` 면 이미 숨겼다 · 6.1 은 이 갈래가 없어 숨은 창으로 루프를 세웠다) · 주 버튼이 이미 떨어졌으면 보내지 않고 `Released{before_entry: true}`(뗌이 웹뷰 자식 창으로 가 스레드 훅이 못 봤을 수 있다 — 근거 없음 a) · 아니면 `SC_MOVE` 를 보내 루프로 넘긴다(이 차례대로 본다 — 끝내라는 청 · Esc · 뗌). **Started**(넘겼고 아직 `WM_ENTERSIZEMOVE` 전 — U3 착지) — `end_move_loop` 는 적어 두기만 하고 들어갈 때 `WM_CANCELMODE` 를 올린다 · 보낸 `SC_MOVE` 가 루프 없이 돌아오면(그 창의 더 위 서브클래스가 먹었다) 아래 「끝」 규칙대로 끝. **들어감**(`WM_ENTERSIZEMOVE`) — `Entered` 알림 · 주 버튼을 다시 본다: 떨어졌으면 `WM_CANCELMODE` 를 ★보내지 않고 올린다(`PostMessage`)★ — 그 처리기 안에서는 루프가 아직 제 펌프 · 캡처를 세우기 전이라 동기로 보낸 취소가 루프에 닿지 않을 수 있고, 올린 것은 루프 자신의 펌프가 나른다(E10 · 가능성 높음 → M17) — 끝을 `Released{before_entry: true}` 로 적는다. **Running** — `end_move_loop` = `SendMessage(WM_CANCELMODE)`(같은 스레드라 바로 불린다 · 4–10 ms 뒤 `WM_EXITSIZEMOVE`). **끝**(`WM_SYSCOMMAND` 반환) — 까닭: `end_move_loop` 를 불렀으면 `Ended` · 아니면 뗌을 봤으면 `Released{false}` · 아니면 `Canceled{esc: Esc 를 봤나}`(조사 E7 · C9 — 뗌을 못 본 끝은 Chrome 도 취소다) · 훅 설치에 실패했으면 `Released`(E7). **창 소멸**(`WM_NCDESTROY` — 어느 상태든) — 훅 · 서브클래스를 걷고 `Destroyed`(B4 — LLM `close_tab` · `close_window` 가 루프 안에서 그 창을 닫을 수 있다 · §6-5). ★재귀 깊이 가드(E10)★ — 루프는 우리 서브클래스의 `WM_SYSCOMMAND` 처리 틀 안에서 돌므로, 루프 안 소멸의 `WM_NCDESTROY` 는 그 틀이 아직 살아 있는 동안 온다. 서브클래스 상태는 깊이 셈(들어올 때 +1 · 나갈 때 −1)을 갖고 `WM_NCDESTROY` 에서는 「걷힘」만 적고 알림을 낸 뒤, 상태를 놓는 것은 깊이가 0 으로 돌아온 틀이 한다 — 펼쳐지며 돌아오는 `WM_SYSCOMMAND` 틀은 끝 알림을 또 내지 않는다(한 번 깃발). tao 가 같은 꼴을 쓴다(tao 0.35.3 `event_loop.rs` `public_window_callback` 의 `recurse_depth` — 코드 확인). 어느 길이든 끝에 훅 · 서브클래스를 걷고 `Ended{end, hid}` 를 **정확히 한 번** 낸다 · 그 뒤 `end_move_loop` 는 아무것도 하지 않는다. ★`hid` 는 끝 까닭과 따로 싣는다(F2)★ — Posted 에서 끝냄 청과 Esc 가 겹치면 끝은 `Ended`(끝냄 청이 앞선다)지만 훅은 이미 숨겼고, Running 에서 Esc 뗌 · 반복만 본 고리는 루프가 이어져 `Released{false}` 로 끝나도 숨어 있다 — 6.2 는 숨김을 `Canceled{esc: true}` 로만 알려 그 둘을 놓쳤다 | 조사 E7 · 스파이크 S4 22/22 · C9(hwnd_message_handler.cc#942-961 — 성공 = `WM_LBUTTONUP` 을 봤을 때만) |
| Esc 훅 | `VK_ESCAPE`(눌림 · 뗌 · 반복 구분 없음 — Chrome 그대로) → **늘** `DwmSetWindowAttribute(DWMWA_TRANSITIONS_FORCEDISABLED)` · `hide_on_escape` 면 그 뒤 `SW_HIDE`(E9 — Chrome `KeyHook` 은 전환 끄기를 조건 없이 하고 숨기기만 깃발을 본다 · hwnd_message_handler.cc#189-201) · Esc 를 봤다고 적음(끝 까닭 `esc`) · 숨겼으면 `hid` 를 적음(끝 알림에 실린다 — F2) · 세션에 따로 알리지 않는다 · 늘 `CallNextHookEx`(먹지 않는다). 날 `SW_HIDE` 라 tao VISIBLE 깃발은 참으로 남는다(트레이 보이기가 무동작 — §2-5 「고아」 · 숨김은 `hid` 로만 세션에 닿는다) · 끝에서 DWM 전환을 되켠다(소멸 끝 제외 · Chrome 은 그 창을 닫는다 — 예비 창 재사용) | C9(hwnd_message_handler.cc#139-160 · #189-213 · desktop_window_tree_host_win.cc#657-679) · 조사 E1 · 스파이크 `key_hook` 14/14 |
| `place_window` | `SetWindowPos` 한 번(자리 · 크기 · z · `SWP_SHOWWINDOW` · 활성이 아니면 `SWP_NOACTIVATE` · `rect: None` 이면 `SWP_NOMOVE \| SWP_NOSIZE` — 제자리 보이기 · F7 b · `z: Keep` 이면 `SWP_NOZORDER`) + `activate` 면 `SetForegroundWindow` — ★거절되면 그대로 둔다★(Alt 를 합성해 전경을 빼앗는 tao `force_window_active` 의 우회를 쓰지 않는다 — 끄는 동안 키 감시가 그 Alt 를 놓기로 읽는다 · F7 a · §6-3). 셸 어댑터는 끌기가 만지는 창의 활성에 Tauri `set_focus` 를 부르지 않는다(U4 `rg`) | 스파이크 `begin_loop` · S7 `g=move` · tao 0.35.3 `window.rs` `force_window_active` |
| `is_window_visible` | `IsWindowVisible` — 실행기가 공통 마무리 뒤 tao `is_visible()` 과 대조한다(F7 c · §6-1) | F7 c |
| `top_level_at` | 맨 위 최상위 창부터 z 순서로 걸으며 `skip` 을 건너뛰고, 보이고 · 최소화 아니고 · 가려지지(DWM cloaked) 않고 · 비쳐 보이는 창이 아니고 · 점을 품은 첫 창. **비쳐 보이는 창** — OS 의 적중 판정을 읽을 수 있는 만큼 따른다(MS 「Layered Windows」: layered 창은 색 키 · 알파 0 자리에서 클릭이 지나가고, `WS_EX_TRANSPARENT` 가 함께 있으면 창 전체가 지나간다): ① **모든 창** — `WS_EX_LAYERED` + `WS_EX_TRANSPARENT`(클릭 통과 — OS `WindowFromPoint` 도 건너뛴다 · U3 착지) ② **다른 앱 layered 창** — `GetLayeredWindowAttributes` 가 답한 창 전체 알파가 0 이거나 색 키(`LWA_COLORKEY`)가 있으면. `WS_EX_TRANSPARENT` 만 켜진 창은 비쳐 보이지 않는다(그 깃발 혼자는 그리기 순서일 뿐 클릭은 그 창이 받는다). **가리는 창으로 남는 다른 앱 layered 창** = 알파 1 이상 · 속성을 못 읽은 창(픽셀마다 알파를 주는 `UpdateLayeredWindow` 창은 OS 가 답하지 않는다 — MS 문서 · 속성을 아직 안 준 layered 창도 같다 — 실측 · 다른 프로세스 창에도 답하는지는 미측정이고 못 받으면 이 갈래라 가리는 쪽이다). 불투명하게 그리는 layered 앱(WPF `AllowsTransparency` · Electron `transparent` · Qt 반투명 창)이 흔해서, 다 건너뛰면 그 앱에 가린 우리 창에 붙고 750 ms 머묾이 그 창을 올린다(R15 · AC43 위반 — 2026-10-19 리뷰). **받은 대가 둘** — ⓐ 색 키 창은 픽셀을 못 읽어 창 전체를 비친 것으로 친다 — 그 창의 칠해진 자리 밑도 가려지지 않은 자리다 ⓑ 픽셀마다 알파 창은 빈 자리 밑도 가려진 자리다 — 2026-10-19 결함(화면 녹화 도구가 녹화 영역 틀을 띄운 동안 우리 창에 붙지 않았다 · 원인 가능성 높음, 실 도구로는 미확인: 가운데가 비어 보이는 늘 위 layered 틀이 바깥 사각형 전체를 품어 그 밑 우리 창을 가렸다)의 틀이 이런 창이면 그 결함은 남고, 셸 `probe` 의 debug 로그가 `layered = PerPixel` 로 보인다 — 그때 고칠지는 사용자 결정이다. **우리 창**은 ① 만 건너뛴다(tao 는 커서 무시 창에만 layered 를 켠다). **창 모양**(`GetWindowRgn` 이 영역을 돌려주는 창 — 모든 창)은 점이 그 모양 안(`PtInRegion`)이어야 하고 밖이면 건너뛴다. 모양 좌표는 바깥 사각형 왼쪽 위가 0 이고, `WS_EX_LAYOUTRTL` 창은 오른쪽 위가 0 이며 x 가 왼쪽으로 는다(모양 x = 바깥 오른쪽 − 1 − 점 x · 실측 — OS `WindowFromPoint` 와 견준 실 창 시험). DPI 인식이 다른 다른 앱 창의 모양이 그 창의 논리 좌표로 오는지는 미측정이다 — 그러면 그 창의 모양 판정이 배율만큼 어긋난다. 판정은 순수 함수(`hit_accepts` — 입력 = 우리 창인가 · layered 꼴 `LayeredKind`{`NotLayered` · `Alpha(u8)` · `ColorKey` · `PerPixel`} · `WS_EX_TRANSPARENT` · 모양 안팎)로 떼어 표 시험 · layered 속성 읽기 · 창 모양 · RTL 모양은 실 창 시험. 다른 앱 창이 맨 위면 셸 `probe` 가 그 창의 진단 사실(`describe_window` — hwnd · pid · 클래스 · 확장 스타일 · layered 꼴 · RTL · 창 모양 유무)을 debug 로 남긴다 — 마우스가 움직일 때마다 불리므로 같은 창이 이어지는 동안은 한 번만이다(커서 밑이 우리 창 · 빈 자리가 되면 다시 남긴다) | 조사 D(KDDW 「끌던 HWND 아래로 z 순서 걷기」 · PRD R15 「다른 앱 창에 가려진 자리 · 최소화된 창은 대상이 아니다」) · Chromium `TopMostFinder::ShouldStopIterating`(Chrome 70 `chrome/browser/ui/views/tabs/window_finder_win.cc` — `WS_EX_TRANSPARENT` 또는 `WS_EX_LAYERED` 면 건너뛰고 `GetWindowRgn` + `PtInRegion` 으로 모양을 본다)이 선례지만 따르지 않는 데가 있다 — 그쪽은 우리 창 · 다른 앱 창을 가르지 않고 layered 창을 모두 건너뛰어 불투명하게 그리는 layered 앱 밑도 대상으로 친다. 우리는 R15 를 지키려 그런 창을 가리는 창으로 남기고, 클릭 통과 · 알파 0 · 색 키 창만 건너뛴다 · MS 「Window Features — Layered Windows」(적중 판정) · MS `GetLayeredWindowAttributes` 문서(`UpdateLayeredWindow` 창이면 실패) |
| `restore_placement` | `SetWindowPlacement`(보통 자리 · 최대화) + `after` 뒤로 · `activate` 가 아니면 활성화 없이(R14) — 되는지 §12 M6. ★좌표계(E12)★ — `WINDOWPLACEMENT.rcNormalPosition` 은 `WS_EX_TOOLWINDOW` 가 없는 최상위 창이면 작업 공간 좌표(작업 표시줄이 위 · 왼쪽에 있으면 그만큼 밀린다)이고 있으면 화면 좌표다(MS `WINDOWPLACEMENT` 문서). 함수는 화면 좌표를 받아 그 창의 지금 확장 스타일 · 모니터 작업 영역으로 옮겨 넣는다. 남긴 창을 되살릴 때는 `Unpark` 뒤에 부른다(§6-2 ②) | PRD R14(우리 확장) · C4(창째 끌기의 복원 — `activate` 는 Chrome 처럼 그대로) |
| `set_switcher_hidden` | 뺄 때 `WS_EX_TOOLWINDOW` 를 켜고 `WS_EX_APPWINDOW` 를 끔 · 넣을 때 둘 다 되돌림(+ `SWP_FRAMECHANGED`) — Alt+Tab 에서 빠진다. ★`WS_EX_APPWINDOW` 도 끄는 까닭(E2 — 코드 확인)★: tao 는 주인 없는 창에 그 비트를 늘 켠다(tao 0.35.3 `window_state.rs` — `ON_TASKBAR` → `WS_EX_APPWINDOW`) · 켜져 있으면 작업 표시줄 · Alt+Tab 에 나갈 수 있다(가능성 높음 → M18). tao 가 깃발이 바뀔 때마다 `GWL_EXSTYLE` 을 다시 써 이 둘을 되돌리므로 뺀 창에는 tao 깃발 setter 를 부르지 않는다(§2-5 「tao 깃발 불변식」). 작업 표시줄은 tao `skip_taskbar`(`ITaskbarList::DeleteTab`)가 함께 뺀다. 그 비트를 같이 쓰는 까닭 = tao 가 자기 숨은 창에 `WS_EX_TOOLWINDOW` 를 다는 주석(「`ITaskbarList` 로 뺀 창은 탐색기 재시작 뒤 다시 나타날 수 있다」 — tao 0.35.3 `event_loop.rs` `create_event_target_window`). `ITaskbarList` 만으로 Alt+Tab 이 빠지지 않는다는 것은 가능성 높음 — M18. ★테두리 있는 창에서는 틀 두께가 바뀐다★ — 그래서 예비 창은 화면 밖에서 이것을 끈 뒤에 자리를 셈한다(§5-5) | B7 · G2 |
| `move_window` | `SetWindowPos(SWP_NOSIZE \| SWP_NOZORDER \| SWP_NOACTIVATE)` — 자리만 | §5-5 두 걸음 ① |
| `cursor_pos` · `primary_button_down` | 붙은 뒤 폴러 · Strip{Pointer} 버튼 감시(앱 수명 폴러 스레드) · 루프의 들어가기 전 · 들어갈 때 확인 | 스파이크 S4 폴러 |

- **windows crate 기능 더하기**(CLAUDE.md 「의존성 (변경 시 보고)」) — `Win32_UI_Input_KeyboardAndMouse`(`ReleaseCapture` · `GetAsyncKeyState`) · `Win32_UI_Shell`(`SetWindowSubclass`) · `Win32_Graphics_Dwm`(전환 끄기 · cloaked) · `Win32_Graphics_Gdi`(`restore_placement` 의 모니터 작업 영역 — U3 빌드로 확인). `Win32_UI_WindowsAndMessaging` 은 이미 켜져 있다(`WS_EX_TOOLWINDOW` · `SetWindowPlacement` 도 그 뒤). 이름은 빌드로 확인한다(그 함수들이 어느 기능 뒤에 있는지는 가능성 높음 수준).
- **`raw-window-handle = "0.6"` 을 platform 의 운영 의존으로 더한다**(B10 · 의존성 보고 대상) — Tauri 가 이미 끄는 같은 판이라 `Cargo.lock` 새 패키지 0(platform 항목의 의존 목록만 늘어난다). 워크스페이스 crate 가 아니므로 의존 상한 게이트(정확히 자기 자신 1 줄)는 그대로다. OS crate 가 아니므로 게이트 ⑤(OS crate 운영 의존은 platform 하나)의 명단에 들지 않는다 — U3 가 그 스텝의 손 명단을 확인한다. **셸은 이 crate 를 의존하지 않는다** — `&WebviewWindow` 를 그대로 넘기므로 트레이트를 이름으로 부를 일이 없다(6판의 「셸이 `RawWindowHandle::Win32` 를 맞춤」 · G16 (a) 는 B10 으로 바뀜).
- **Windows 밖** — 같은 이름의 함수가 `Err(Unsupported)` · `None` · `false` 를 돌려준다. `begin_move_loop` 의 `Err`(`Unsupported` · 창 없음 · 서브클래스 · 훅 설치 실패 밖의 Win32 실패)는 관문 `Start` 실패라 그 묶음을 멈추고 `Failed(Start)` → T43 되돌리기다(D1 — 6.1 의 §10 「실패해도 이어감」과 어긋나던 것을 이 뜻 하나로 맞춤 · macOS 이식 때 따로 조사 — PRD R19 · W11 · `performWindowDragWithEvent`). 훅 설치 실패만은 Chrome 처럼 실패로 치지 않고 루프를 세운 뒤 끝을 `Released` 로 친다(E7). U3 착지의 `Err` 변이(`Unsupported` · `NoWindow` · `WrongThread` · `Busy` · `Os`)는 전부 이 뜻이다.
- **받는 쪽 패닉은 가두지 않는다**(U3 코드 리뷰 C1 · 사용자 결정 2026-10-18 「복잡할거면 안넣는게 맞지」 — ADR-0292 그대로) — 알림 받는 쪽(U6 어댑터)의 패닉은 운영 패닉 훅이 로그한 뒤 abort 하고, 훅이 없어도 되감기가 `extern "system"` 경계를 넘으면 abort 다. U6 은 base `panic::contain` 으로 감싸지 않는다.
- **게이트** — 새 `cfg` 는 전부 platform 안이라 게이트 ④ · ⑥ 명단이 바뀌지 않는다 · OS crate 운영 의존은 여전히 platform 하나(⑤). platform `src/lib.rs` 헤더의 입주자 목록과 CLAUDE.md 「백엔드 모듈 맵」 platform 항목(「지금 입주자는 여덟」)을 함께 고친다(U3 · `/review doc`).
- **ADR-0266 절차** — 새 목적 함수 묶음은 사용자에게 보고하고 ADR 로 박는다(§16 ADR-e).

## 9. 제목 표시줄(T-66)과의 맞물림

이번엔 제목 표시줄을 없애지 않는다 — 탭 끌기가 그것을 **막지 않게만** 짓는다(PRD R25 · 조사 A12 · B6 · 스파이크 S6 합격).

- **누름 처리기는 탭 머리에만** 단다 — 탭바의 빈 곳(머리들 · `+` 뒤 남는 칸)은 따로 된 요소(`tab-strip-empty`)로 그리고 아무 처리기도 달지 않는다. T-66 이 그 요소에 `app-region: drag` 를 달면 OS 창 끌기 · 두 번 눌러 최대화 · Snap 이 그대로 선다(S6 — WebView2 가 호스트에 `WM_SYSCOMMAND 0xF012` 를 스스로 보낸다).
- **비활성 탭 위 6 px**(최대화가 아닐 때 — 조사 A12 `kDragHandleExtension = 6` · tci #47 grounding) — 머리 안에 위 6 px 자식 `tab-drag-handle` 을 둔다. 이번엔 아무 속성도 없어 그 자리 누름도 탭 끌기다. T-66 은 「비활성 · 최대화 아님」일 때만 거기 `app-region: drag` 를 단다.
- **drag 영역에서 새는 누름을 버린다** — 스파이크 S6: drag 영역의 누름이 움직임 없이 끝나면 페이지에 `pointerdown` · `pointerup` 이 새어 들어온다. `useTabDrag` 는 `closest('[data-app-region="drag"]')` 에서 시작한 누름을 무시한다(지금은 그 속성이 없어 무해 — T-66 이 속성과 CSS 를 함께 단다).
- **탭은 `app-region: no-drag`**(T-66 때 CSS 한 줄 — B6 · S6 「`no-drag` 탭 누름 → 캡처 성공」).
- **떼어진 창 자리 셈에 제목 표시줄 높이를 상수로 두지 않는다**(§5-5 — 안쪽 원점 − 바깥 원점 · 크롬 보고로 잰다) — 제목 표시줄을 없애도 같은 식이 선다.
- **창 이동 루프 함수(§8)는 T-66 이 쓰지 않는다** — 빈 탭바 끌기는 OS · WebView2 몫이다. 두 길이 같은 OS 루프(`SC_MOVE|2`)라 Snap 등 모습이 같다.
- **탭바 구역**(§5-2)은 탭 끌기 영역(첫 탭 왼쪽부터 끌기 영역 폭 A — `+` 앞에서 끝난다 · C3) 위아래 ± 15 다. 창 단추 · 번호 표식처럼 `+` 뒤에 서는 것은 구역 밖이다. T-66 PRD R14 「다른 창의 탭을 빈 곳 · 표식 · 창 단추 위에 놓으면 그 탭바 끝에」는 그 자리를 구역으로 보므로 어긋난다 — T-66 TRD 가 Chrome(끌기 영역)과 그 문장 중 무엇을 따를지 정한다(6판의 「탭바 줄 전체 ± 15」 전제는 C3 로 바뀌었다).

## 10. 계측

그룹 = `engram_dashboard::dock` · `engram_dashboard::commands::dock` · `engram_dashboard_platform::move_loop`(`docs/reference/logging-conventions.md` 「계측 의무」 — 세션 수명 · 외부 경계 · 동시성). 식별자 · 수치는 필드, 메시지는 한국어 한 줄. 끄는 동안 되풀이되는 것은 세어 끝 줄에 모은다 · `let _ =` 로 버리는 실패는 없다.

| 어디 | 무엇 | 레벨 | 필드 |
|---|---|---|---|
| 세션 | 시작 | info | `session` · `view` · `window` · `kind = strip \| whole_window` · `tabs` |
| 세션 | 거절(`Busy` · `TabNotFound` · `Dragging` …) | debug | `view` · `reason` |
| 세션 | 단계 바뀜(떼기 · 붙기 · 루프 시작 · 들어감 · 끝) | debug | `session` · `from` · `to` · `window` |
| 세션 | 늦은 사건 · 낡은 seq · 다른 출처 사건 버림(받기 거르개) · 마운트 표 바뀜(F6 c) | debug | `session` · `current` · `event` · `why = session \| stale \| source` · `mount_changed` |
| 세션 끝 | **끝 한 줄**(`EndLog::of`) | info | `session` · `outcome = strip \| dock \| new_window \| moved \| reverted \| aborted` · `end_by = pointer \| release \| release_timeout \| loop_release \| key \| esc \| capture_lost \| loop_canceled \| holder_gone \| holder_reloaded \| abandoned \| failed` · `reason`(`DockError`) · `reorders` · `detaches` · `attaches` · `windows` · `stale` · `spare_wait_ms` · `main_refill` · `compensated`(공통 마무리가 되돌린 것 — 예비 · 숨긴 창 · 루프) · `orphaned`(이 끌기에서 명부 고아 · `reveal` 로 넘긴 창 수 — F1) · `elapsed_ms` · `errors` |
| 실행기 | 할 일 실패 — 무름(끌기는 이어감 · `Failed(Soft)` → T44 · 세션이 끝난 뒤면 x7 · ⓞ 면 먼저 명부 고아로 — F1) · 관문(그 묶음 멈춤 · `Failed(Take \| Start)` → T09 · T43 — D1) | 무름 = warn(세션당 처음 한 번 · 나머지는 끝 줄 `errors`) · 관문 = warn 늘 | `op` · `label` · `gate` · `skipped`(돌리지 않은 할 일 수) · `: {e}` |
| 실행기 | begin · leave 시한 → `Abandon` | warn | `by = token \| session`(F5) · 그 값 · `waited_ms` |
| 명부 고아 | 넘김(정리 할 일 실패 · `CloseWindow`) · 다시 하기 · 한 단 올림 · 포기 · 끝(소멸 · 제자리 다시 보임) · `respawn` 짓기 | 넘김 = debug(`CloseWindow` 의 늘 넘김) · warn(실패로 넘김) · 다시 하기 · 끝 = debug · 포기 = error | `label` · `want` · `step` · `in_model` · `respawn` · `: {e}` |
| 실행기 | 공통 마무리 뒤 tao · OS 보임 어긋남(F7 c) | error(release · debug 는 단언) | `session` · `label` · `tao` · `os` |
| 세션 | ⊥ 칸에 닿음(행렬 — 일어날 수 없는 조합) | error(release 빌드 · debug 는 단언) | `session` · `phase` · `event` |
| 세션 | 놓기 확인 시작 · 답 · 시한(T23 · T22 · T31) | debug | `session` · `holder` · `why = poll \| button_watch \| foreign_key` · `waited_ms` |
| 세션 | 떼기를 예비 창 기다림으로 미룸(`NotReady`) | debug | `session` · `waited_ms` |
| 예비 창 · 명부 | 지음 · 되돌림 · 닫음 · 잃음 · 명부 재기 받음 | debug(지음 실패 · 잃음 = warn) | `label` · `role` · `build_ms` |
| platform 루프 | 시작 · 들어감 · 끝 까닭 · 들어가기 전 삼킴 · 훅 설치 실패 | debug(실패 = warn) | `hwnd` · `end` · `esc` · `hid`(F2) · `mouse_up` · `before_entry` · `ms` |
| `apply::dock_tab` | 적용 · 거절(지금 그대로) | info · debug | 지금 칸 + `mode = adopt` · `kept_lease` · `emptied` · `main_refilled` |

## 11. 테스트 계획

TDD — 모듈마다 시험부터(ADR-0012). 시험 중 실 마우스를 쓰는 것은 없다(§12 로).

| 층 | 무엇을 | 어떻게 | 단위 |
|---|---|---|---|
| Rust `layout` | 붙는 몫(Left · Right · Top · Bottom × 칸 · 바깥) · `AdoptWindow`(묘비 · 이미 있음 · seed 되살림 · **재기 옮겨 담기**) · `keep_lease` · 떼는 창 다음 활성(E2 a · b · **붙었던 창의 붙기 전 자리 — C8**) · `OnEmptied::Keep`(+ 재기 돌려주기) · `main_refilled` · 같은 자리 무변경 + 표식 유지 | 지금 `manager.rs` · `dock.rs` · `apply.rs` · `tests/layout_apply.rs` 시험 고침 + 더함 | U1 |
| Rust `dock::geometry` · `strip` · `sizing` · `targets` · `parked` | 띠 두께(10% · 최소 · 반올림) · 막힌 띠 · 유효 영역(탭바 구역 · 바깥 띠 빼기 · 막힌 바깥 띠는 안 빼기 — AC36) · 고르기(모서리 · 동률 · 경계 반열림) · 붙는 몫 표(AC2 · AC39 · AC44 의 수) · 탭바 구역 = 끌기 영역 ±15(C3 — 가로 반열림 · `+` 자리는 밖) · `dragged_size`(**작업 영역 상한 먼저 · 절반 하한 마지막 — C5** · 큰 최대화 창에서 절반이 이김) · 가장 가까운 모니터(C6 — 점을 품은 모니터 없음 포함) · `detached_origin` · `whole_window_origin`(C4) · 화면 점 → CSS 점(배율 1 · 1.25 · 2) · 커서 밑 창 → 붙기 · 띠 · 없음 · `offscreen_near`(F3 d — 커질 크기 `size × max(1, s_target ÷ s_now)` 로 겹침을 봄 · 배율이 작아지는 쪽은 그대로 · 후보는 변에서 `PARK_GAP_PX` · 사방이 막히면 `None`) · 명부(역할 · 재기 받기 · 준비됨 — 고아는 준비됨이 아님) · **명부 고아**(F1 — `orphan_next` 표: 모델에 있음 → `reveal` · `Close` 살아 있음 → `destroy` · `GiveBack` → 다시 · 실패면 `Close` + `respawn` · 한 단 올린 것도 실패 → `GaveUp` · 고아는 트레이 · 자리 기록 · 재기 갈래에서 대기 창처럼 빠지고 `reveal` 은 빠지지 않음 · 소멸이 고아를 지우고 `respawn` 이면 예비가 없을 때만 새로 지음 · `SpareFill` 이 `Spare` · `respawn` 고아가 있으면 짓지 않음) | 표 시험 · 옛 골든 사례 중 살아남는 기하(바깥 띠 · 칸 · 경계)는 Rust 표로 옮긴다 | U2 · U4(`parked`) |
| platform | `place_window`(z · 활성 없음 · `rect: None` = 자리 · 크기 그대로 보이기 — F7 b) · `is_window_visible`(F7 c) · `move_window`(크기 그대로) · `top_level_at`(skip · 최소화 · 숨김) · `set_switcher_hidden`(뺄 때 `TOOLWINDOW` 켜고 `APPWINDOW` 끔 · 넣을 때 둘 다 되돌림 — E2) · `restore_placement`(도구 창 = 화면 좌표 · 아니면 작업 공간 좌표로 옮김 — E12 · 작업 표시줄 위치는 시험이 못 바꾸므로 옮김 셈은 순수 함수로 떼어 표 시험) · `NativeWindow::of` · `primary_button_down` · `cursor_pos` | 실 Win32 창을 띄우는 시험(platform 의 실프로세스 시험 꼴 · `cfg(windows)` 는 platform 안) · `-- --test-threads=4` | U3 |
| platform 이동 루프 | 끝 까닭 분류(훅 깃발 → `MoveLoopEnd`) · **상태 줄**(Posted 에서 끝냄 → 삼킴 + `Ended` · **Posted 에서 Esc 봄 → 삼킴 + `Canceled{esc: true}`(D2)** · Posted 도착 때 버튼 뗌 → 삼킴 + `Released{before_entry}` · 들어갈 때 뗌 → `WM_CANCELMODE` 를 올림(보내지 않음 — E10) + `Released{before_entry}` · 소멸 → `Destroyed` · 끝 알림 정확히 한 번 · 끝난 뒤 `end_move_loop` 무동작) · Esc 훅(늘 DWM 전환 끔 · 숨김은 깃발일 때만 — E9) · **모든 끝 알림의 `hid`**(F2 — **Posted + `end_move_loop` + Esc → 삼킴 + `Ended{hid: true}`**(끝냄 청이 앞선다) · Running 에서 Esc 뗌(또는 반복)만 본 뒤 버튼 뗌 → `Released{false}` + `hid: true` · `hide_on_escape: false` 면 Esc 를 봐도 늘 `hid: false` · `Destroyed` 에도 실림) · **고리 안 소멸**(E10 — 루프를 도는 자리를 platform 안 시험 seam 으로 바꿔 끼워 그 안에서 `DestroyWindow` → 끝 알림 한 번 · 상태는 깊이 0 에서 놓임 · 펼쳐지는 `WM_SYSCOMMAND` 틀이 두 번째 알림을 내지 않음) | 순수 상태 함수(메시지 · 깃발 → 할 일 · 알림)를 단위 시험 · 서브클래스에 실 창을 두고 `SC_MOVE` 를 보내지 않은 채 소멸 · 삼킴 · 고리 안 소멸은 실 창 시험 · 루프 자체는 실 마우스(§12) | U3 |
| Rust `dock::session` | **행렬 522 칸 전부**(§2-3-3 — 표 시험: 상태 꼴 18 마다 대표 세션 · 사건 꼴 29 마다 대표 사건 · 칸 안 조건 갈래(holder 인가 · 예비 준비됨 · kind · 메인 · `geom` 있음 없음)마다 하나 더 → `cell` 이 행렬의 이름을 돌려주는지 · ⊥ 칸은 debug 단언) · **전이 T01–T44 마다** 다음 상태 · 할 일(순서 · 관문 표시 · `Write` 마지막) · **받기 거르개**(출처 · 단계를 `seq` 보다 먼저 — D5: 버린 높은 seq 가 `seen` 을 올리지 않음 · 늦은 `loop_no` · `gen` · 시한 seq · 버린 `Leave` 에 `Ended`) · **공통 마무리 보상**(쥔 것마다 — 꺼낸 예비 · 숨긴 창 · 살아 있는 루프 · 답 못 한 invoke · 남긴 창 · `SpareFill` 은 `Adopted` · `Lost` 일 때만) · **§2-3-5 대조 표의 시나리오를 사건 열로 끝까지**(A1 · B1 · B3 · B4 · B6 · D1 · D2 · D3 · D4 · D5 · E1(`Canceled{esc: false}` 와 Moving{Detached} 의 `Abandon` 으로 되돌린 빈 예비가 `SpareGiveBack` · `spare = None` · 둘째 예비 없음) · E6 · E7 · C8/E11(끄는 탭이 붙기 전 활성 자리 앞에 선 W)) · 탭 하나(C4 자리) · 메인 마지막 탭(붙기 · 다시 떼기 · 메인 탭바에 다시 붙기 · Esc) · 경계 빠르게 오가기(AC37) · 예비 창 없음 = `NotReady` 뒤 다음 청에서 뗌 · **6.3 칸 · 갈래**(F1 — I × `Failed(Soft)` = x7 · 정리 할 일에 ⓞ 등급이 실림 · 그 칸들이 `spare` · `hidden` · `emptied` 를 낸 때 놓음 · 공통 마무리 5 가 명부에서 빼지 않음 / F2 — **MK × `LoopEnded(Ended)` · `hid` → T37 ② 가 `hidden` 에 적음 → 공통 마무리 3 이 제자리 다시 보이기** · **MV × `LoopEnded(Ended)` · `hid` · O 없음(§6-2 ④) → 탭이 숨은 끄는 창에 남고 공통 마무리 3 이 다시 보임** · MA × `Ended` · `hid` → T38 이 적음 · MR × `Released{false}` · `hid` → T37 ② / F3 b — `settled` 아닌 `geom` 은 Detached `original` 을 적지 않고 다음 `settled` 사건이 적음 · WholeWindow 는 `settled` 를 보지 않음 / F4 — W 열 × `Failed(Soft)` 여덟 칸 = ⊥(debug 단언) · `hidden` 에 같은 창을 두 번 적어도 하나 / F5 — `Abandon{by: Session(id)}` 은 지금 세션 번호와 맞을 때 · `Token(t)` 은 `token` 과 맞을 때만 받음 · 엇갈린 쪽(`Session` 인데 `token` 과 같은 값)은 ⊘ / F6 — 세션 번호 1 부터 단조 · `mount` 가 바뀌면 그 창 `seen` 을 0 으로 쳐 받음(T26 뒤 옛 `mount` 의 늦은 사건 → 새 `mount` 의 seq 1 받음) · 끝 묶음의 `Emit(Ended)` 대상 = `armed ∪ 모델 창`) · F8 — U5a-1 을 마친 트리에서 M 행 전이의 몸이 `unimplemented!("U5a-2 Txx")` 꼬리표를 단다. ★`step` · `accept` · `cell` 에 `_ =>` 갈래가 없는지★는 리뷰가 보고 컴파일러가 지킨다 | 순수 `accept` · `cell` · `step` 표 시험 | U5a |
| Rust `dock::exec` | `Write` 결과가 줄 맨 앞(다른 사건이 끼지 않음) · **할 일 실패**(관문 실패면 그 뒤 할 일 · `Write` 를 돌리지 않음 · 결과 블록 = [`Wrote` 또는 관문 `Failed`] + [무른 `Failed`] 가 맨 앞 · 다음 묶음의 블록이 앞 묶음의 무른 실패를 앞지름 — D1) · 같은 스레드 재진입 사건은 줄 뒤로(순서) · 캡처를 쥔 창에 `Hide` 를 내지 않음(불변식 단언) · `PlaceDetached` 가 `Unpark` 뒤의 틀 두께로 셈 · **두 걸음 놓기**(배율 같음 = 한 걸음 · 다름 = 화면 밖 `move_window` 뒤 다시 읽고 놓기 · 그 자리 없음 = 한 걸음 + 다시 놓기 — E3) · **다시 놓기 계기(F3 a)** — 가짜 `WindowPort` 가 `PlaceDetached` 의 `SetWindowPos` 에서 `Moved` 를 먼저 · `ScaleFactorChanged` 를 나중에 낸다 → `Posted` 동안의 첫 `ScaleFactorChanged` 가 목표 배율과 다르면 한 번 다시 놓음(앞선 `Moved` 가 막지 않음) · 같으면 · `LoopEntered` 뒤면 · 두 번째면 다시 놓지 않음 · **`settled` 표(F3 b)** — 목표 배율 전 `geom` 은 거짓 · 뒤는 참 · 막 뗀 창 밖은 늘 참 · **꺼낼 때 읽기(F3 c)** — 사건을 줄에 넣은 뒤 창을 옮기면 `step` 이 받는 `geom` · `probe` 가 옮긴 뒤의 값(커서는 넣을 때 값) · platform · 창 사건에 끄는 창 `geom` 싣기(D4 · E4) · 안내 창 배율 바뀜 뒤 다시 놓기 · 끄는 창 label · `loop_no` 로 루프 · 창 사건 거름 · `dock:drag` 를 낼 때 `window_role` 거울을 함께 적음(E6) · **고아 넘김(F1)** — `SpareGiveBack` 단계 실패 → 그 `Failed(Soft)` 가 줄에 든 때 명부가 이미 `Orphan{GiveBack}`(세션이 그 실패를 처리하는 걸음에서 명부를 읽어 단언) · `CloseWindow` 는 성공해도 `Orphan{Close}` → 소멸이 지움 · `close` 가 실패하거나 소멸이 안 오면 `OrphanDue` 에 `destroy` → 또 실패면 `GaveUp` → 끌기 끝에 한 번 더 · 되돌리기 고아 다시 하기 성공 → `Spare` → 다음 `Leave` 가 T19 · 다시 보이기 실패 → `reveal` → `OrphanDue` 에 제자리 다시 보이기 · 모델에 있는 고아는 닫지 않음 · `OrphanDue` 가 `cont` 가 선 동안(결과 블록 사이) 처리되지 않음 · 세션이 끝난 뒤의 실패(x7)에도 넘김이 섬 · **F7** — `Focus` · 붙기 활성이 `WindowPort` 의 platform 활성만 부름(포트에 `set_focus` 갈래가 없다) · 공통 마무리 3 이 `rect: None` 으로 보임 · 공통 마무리 뒤 보임 대조(가짜 포트가 tao 보임 ≠ OS 보임을 내면 debug 단언 · `reveal` 로 넘긴 창은 빠짐) | 가짜 포트(가짜 `WindowPort` 가 배율 바뀜 · tao 의 크기 고침 · 사건 차례 · 실패 · 보임 어긋남을 흉내) | U5b |
| 셸 연결 | invoke → 메인 스레드 사건 · 시한 → `Abandon` · `Moved` · `Destroyed` · `on_page_load` · `settled` 배선 · 예비 창 모델 밖 · 명부: 재기 받기(세 보고 처리기) · 트레이 술어(보이기 · 숨기기 · `--hidden` 이 명부 창을 안 건드림) · 사라지면 명부에서 빼고 다시 지은 뒤 `WindowGone` · 자리 기록 건너뜀 · `PageLoaded` 는 우리 창 전부 · `window_role`(역할 · U6 부터 `drag` 거울) · 안내 창 권한 · **남긴 창 · 되돌린 예비에 탭 0 개 사건이 가지 않음**(§7 ⑤) · **명부 창을 만지는 tao · Tauri 깃발 setter 자리 `rg` 0**(E2 — 짓는 때 밖) · **`set_focus` 자리 `rg` 0**(F7 a — 끌기 어댑터 `commands/dock.rs` · `dock/` · 명부 창) · **고아 배선**(F1 — 트레이 술어 · 자리 기록 · 재기 갈래가 고아를 대기 창처럼 뺌 · 소멸 갈래가 고아 항목 · `reveal` 을 지우고 `respawn` 이면 예비가 없을 때만 새로 지음 · 어댑터의 `destroy` 가 닫기 요청 갈래를 거치지 않음 — U4) · **`Abandon{by}`**(F5 — leave 시한 → `Abandon{by: Session}` 이 Strip 에서 받아져 되돌리기 · begin 시한 → `Token` — U6) · **세션 번호 · 끝 알림**(F6 — 거울은 1 부터 단조 번호 · `dock:drag{Ended}` 가 `armed ∪ 모델` 에 감 — U6) · 어댑터의 tao 보임 읽기(`is_visible()`)가 포트 계약대로(F7 c — U6) | `lib_unit`(셸 단위 — `src/**` `#[cfg(test)]` · `tray/core.rs` 의 `ui_windows` 시험은 술어 조합으로) · 통합 타깃 `layout_apply` | U4 · U6 |
| TS `tabStrip` | `layout.ts`(옮긴 표 시험) · `reorderGeometry`(**`insertionIndex` = Chromium 표 — 제자리 후보 건너뜀 · 앞 쪽 동률 · 끄는 탭 뒤 −1 · 보이는 끝 자르기** · 끌기 영역 폭(닫기 모드에도 잰 폭) · 끄는 탭 x 자르기 · 문턱 `round(16 × 그 자리 폭 ÷ S)` · `pLast` 는 자리가 바뀔 때만 · 붙은 직후 문턱 없음 · 넘친 탭바) · `closingMode`(39/40 · 59/60 · 300 ms · 밖 누름 · 맨 끝 탭 — AC40) · `useTabDrag`(10 px 초과(같음은 아님) · 누름 활성 · 캡처는 문턱 뒤 · 탭바 줄 요소(순서가 바뀌어도 안 끊김) · 문턱 전 두 번 누름 = 이름 바꾸기 그대로 · 닫기 · `+` 누름은 끌기 아님 · **`seq` 단조 · 세션 하나에 카운터 하나** · **begin · leave 답 기다리는 동안 끝 입력 붙들기 · 답마다 처리** · zone 나감 → leave 청(답 `NotReady` 면 Strip 유지 · 다시 청) · 떠난 뒤 포인터 · `lostpointercapture` 버림 · Esc · 그 밖 키(반복 포함) · 캡처 잃음 · 붙은 창 입력 · `dock:feed{Release}` → 지금 자리로 `End{Drop, Release}`(이미 끝냈으면 버림 · 답 기다리는 동안이면 붙듦 — E7) · drag 영역 누름 무시 · 끌기 사건 셋에 마운트 표 `mount` 를 실음(마운트마다 하나 · 같은 마운트 안에서 그대로 — F6 c)) | vitest · 가짜 invoke · 가짜 시계 | U7a · U7b-1 |
| TS 그 밖 | `dockSession`(앱 전체 키 감시 켜고 끔 · 같은 세션 번호의 `Active` · `feed{Attach}` 는 카운터를 그대로 둠 · `dock:spare` · `window_role` 대기 모드 · 마운트 때 `drag` 로 진행 중 세션 받기 — E6 · 대기 모드에서 0탭 자가닫힘 끔 · **F6** — `dock:drag` 듣기를 건 뒤에 `window_role` 을 부름 · 답보다 먼저 온 `Ended(s)` 뒤의 답 `drag = s` 는 켜지 않음(`lastEnded`) · 키 끝 하나를 보내면 스스로 꺼짐 · 같은 세션의 `dock:feed{Attach}` 를 받으면 다시 켜짐(MA 의 x1 예외 — §6-3) · `Ended(s)` 뒤에는 `s` 이하 번호로 켜지지 않음) · `fileDropGuard`(AC22 — 옛 시험 옮김) · `DockOverlay` 보기(`OverlayView`) · 대기 모드(자가닫힘 없음) · `latestValueReporter` · 구분선 투명도(C7) · 비켜남 곡선(C2) | vitest | U4 · U7b-1 · U7b-2 · U7c |
| 화면(QA) | AC 전부 — 실 창(`/qa full` · cdp) + 마우스는 §12 | `/qa full` | U7c 뒤 |

- **회귀 수치** — 걷는 시험(`book.rs` 안 ~3,900 줄 · `dockDrag.test.ts` · `dockPick.test.ts` · `dragImage.test.ts` · 골든 132 사례)이 빠져 워크스페이스 · vitest 통과 수가 크게 준다 — 회귀가 아니다. 단위마다 「뺀 수 · 더한 수」를 커밋 본문에 적는다.
- **게이트** — CLAUDE.md 「빌드·검증 명령」 그대로 + platform 줄(`cargo test -p engram-dashboard-platform -- --test-threads=4`)이 새 창 시험을 돈다. CI sync 게이트가 지운 생성물을 잡는다(U8).

## 12. 실측 — 마우스 (메인 세션 · 사용자 자리)

- **마우스를 움직이는 실측은 메인 세션이 사용자가 있을 때 돌린다** — 서브에이전트가 돌리면 자동 권한 검사가 막는다(핸드오프 do-not · 2026-10-18). 입력 = 스파이크의 `SendInput` 드라이버(`.claude/handoff/attachments/spike-chrome-drag-src-2026-10-18/driver/`)를 실 앱 창에 맞게 고쳐 쓰고, 사람 손 확인이 필요한 것(혼합 DPI · 잠금 화면)은 사용자가 한다.
- **앱은 셸에서 직접 띄우지 않는다**(CLAUDE.md 「GUI 실측」 — 런처 · `scripts/launch-detached.ps1`).
- **환경을 적는다** — RDP / 콘솔 · 합성 Hz · 배율 · 모니터. 스파이크는 RDP ~32 Hz 뿐이었다(W12 · M13).
- **합격 기준이 Chrome 에서 오면 Chrome 값**, PRD 에서 오면 PRD 값이다. 불합격 = §15-2 정지 조건(대안을 짓지 않는다).

| # | 무엇 | 합격 | AC · 근거 |
|---|---|---|---|
| M1 | 떼기 — 실제 내용(터미널 · 챗)이 든 탭을 뗄 때 창이 커서 밑에 나타나는 시간 · 깜빡임(빈 화면 포함) · 같은 끌기에서 두 번째 떼기 · `Unpark`(Alt+Tab 빼기 비트를 끄며 틀이 바뀜)가 깜빡임 · 지연 · 자리 어긋남을 내는지 · 예비 창의 메모리(작업 관리자) | 깜빡임 없음 19/20 이상 **그리고** 떼기 → 보임 100 ms 이하 19/20 이상(PRD R28 S8 기준) · 탭이 커서 밑 잡은 자리에 옴 | AC2 · AC20 · AC24 · 스파이크 S8 은 미리 그려 둔 내용으로만 쟀다 · G2 · G3 |
| M2 | 메시지 펌프에서 시작한 루프 동안 웹뷰 → 셸 IPC · emit · `run_on_main_thread` · `Moved` | 20/20 루프 안에서 흐름 · 지연 기록 | 스파이크 S3(펌프 시작 표본 1–2 회) |
| M3 | 다른 창 탭바에 붙기 → 그 창에서 순서 이동(첫 자리 0 에서 곧바로 고쳐짐 — C1) → 그 창 웹뷰 밖(바탕화면 · 다른 창)에서 놓기 · 다시 떼기(그 창의 붙기 전 활성이 돌아옴 — C8) | 붙기 · 뗌 알아챔 20/20 · 웹뷰 밖 놓기 20/20 · 붙는 순간 탭이 0 자리에 보이는 프레임이 있는지 기록 · 붙기 활성(platform `SetForegroundWindow` — F7 a)이 거절되지 않는지 · 그 활성이 끌기를 끝내지 않음 20/20 | AC18 · AC24 · 스파이크 「붙은 뒤 B 밖에서 놓기 미검」 |
| M4 | Esc — 탭바 안 · 떼어진 창 · 붙은 창(그 창의 붙기 전 선택 — C8) · 창째 움직임(원래 자리 — C4 로 옮긴 자리가 아니라) · 최대화 출처 | 원래 창 · 자리 · 처음 선택 · 최대화 20/20 · 뗀 뒤 버튼을 떼도 무변경 · Esc 가 슬롯에 안 들어감 | AC19 · AC25 · AC9 · AC26 |
| M5 | 키 — 탭바 안: 글자 · Ctrl · Shift · Alt · 키 반복 · 끌기 전부터 쥔 키(쥔 채 반복이 끝내는지) / 이동 루프: 화살표(창 이동) · **Enter(뗌 없는 루프 끝 → 되돌리기라는 추론 — C9)** · 글자 키가 우리 웹뷰 감시에 닿는지(닿으면 놓기) · **창째 끌기의 Esc(포커스가 WebView2 자식일 때 훅 · 감시 중 누가 받는지 — 근거 없음 b)** / 붙은 창 / IME 조합 중 문턱 넘기 / Alt-Tab · Win 키 / **끌기 뒤 키보드(F6)** — 끌기 중 다시 읽힌 창 · 들였다 예비로 되돌린 뒤 보통 팝아웃으로 다시 들인 창 · 붙기를 정한 순간 누른 키(MA 의 x1) 뒤 끌기가 끝난 다음 그 창들에 글자가 들어가는지 | 탭바 안 = 놓기로 끝 · 그 키가 어느 슬롯에도 안 들어감 — 10/10 · 이동 루프 = Chrome 규칙(Esc = 되돌리기 · 뗌 없는 끝 = 되돌리기) 10/10 · 나머지는 모습 기록 | AC19 · PRD R12 가장자리 · C9 · 스파이크 「Esc 외 키 미검」 |
| M6 | 탭 하나 팝아웃 · 메인 마지막 탭(최대화 포함) — 창째 따라옴(C4 자리) · 붙은 뒤 메인 자리 · 최대화 복원이 **활성화 없이** 붙은 창 뒤로 · 메인 탭바에 다시 붙기 · Esc | AC9 · AC26 · AC38 문장 그대로 10/10 | §6-4 · §15-2 S-c |
| M7 | 도킹 띠 · 미리보기 · 붙는 크기 · 바깥 위 띠 · 막힌 띠 · 작은 칸 | AC2–AC4 · AC15 · AC33 · AC36 · AC39 · AC44 의 수 그대로 | §5-3 · §5-4 |
| M8 | 750 ms 앞으로 올리기 · 움직이면 다시 셈 · 다른 앱 창은 안 올림 | AC43 문장 그대로 | 조사 E6 |
| M9 | 탭바 구역 경계 빠르게 오가기 · 끄는 중 앱 끄고 다시 켜기 | 창 겹으로 안 생김 · 탭 한 곳 20/20 | AC37 · AC11 |
| M10 | 배율이 다른 모니터 사이(떼어진 창 · 안내 창 · 붙는 크기) · **안내 창을 배율이 다른 모니터의 대상 창 위로 옮긴 뒤 크기 · 자리(`WM_DPICHANGED` 뒤 다시 놓기 — 근거 없음 c)** · **배율이 다른 모니터로 떼기 — 두 걸음 놓기(E3): 떼어진 창 바깥 크기 = `dragged_size × 커서 모니터 배율`(두 번 늘거나 줄지 않음) · 화면 밖 첫 걸음에서 배율 바뀜이 끝나는지(동기인지) · 다시 놓기가 도는지 · 보이는 순간 깜빡임 · 그 뒤 붙는 크기의 `original`** · **두 걸음의 6.3 고침(F3)**: 다시 놓기가 `Moved` 가 먼저 와도 도는지(실행기 로그) · 첫 `original` 이 목표 배율의 `geom`(`settled`)에서 왔는지 · 화면 밖 첫 걸음의 자리가 커진 크기(Windows 11 제안 사각형)로도 다른 모니터에 걸치거나 화면에 비치지 않는지 · 떼어진 창 크기의 작업 영역 = 커서에 가장 가까운 모니터(C6) | AC12 문장 그대로 · 안내 창이 대상 창 안쪽을 덮음 | W4 · 스파이크 「혼합 DPI 미검」 — 사용자 손 |
| M11 | 누름 활성 · 10 px 문턱 · 탭바 안 순서 이동(Chromium 자리 규칙 · 끌기 영역 끝까지 그림 · `+` 앞에서 멈춤 — C3) · 200 ms 비켜남(EASE_OUT) · 멈춘 채 떼기 · 짧게 톡 · 넘친 탭바 · 닫기 모드 경계 · 활성 탭 모양 · 구분선 투명도(C7) | AC23 · AC27 · AC29 · AC30 · AC31 · AC32 · AC40 | §5-1 · §5-6 · §5-7 |
| M12 | 파일 드롭 막기 · 제목줄 끌기 | AC22 · AC1 | 5.8 §8-1 합격 상태가 그대로인지 |
| M13 | 콘솔 세션 60 Hz 에서 M1 · M2 · M3 다시 | 같은 기준 | 스파이크 「콘솔 미검」 · W12 |
| M14 | 이동 루프 중 잠금 화면 · UAC | 되돌리기 · 안내 안 남음 | AC21 · 조사 E7 |
| M15 | 실행 중 에이전트가 든 탭 — 떼기 · 붙기 · 다시 떼기 · 띠 | 출력 끊김 없음 | AC7 · AC42 |
| M16 | Snap 과 겹친 자리 · Snap 해 둔 탭 하나 창 끌기 | 모습만 기록(합불 없음 — PRD R2 · R28 S3) | PRD R2 · R7 · AC33 |
| M17 | 버튼을 곧바로 떼는 떼기 · 창째 끌기 — 문턱을 넘자마자(루프가 들어가기 전) 뗌 · 루프가 버튼 없이 시작되지 않는지(근거 없음 a — 뗌이 웹뷰 자식으로 가 스레드 훅이 못 볼 때) · **들어갈 때 뗌을 봐 올린 `WM_CANCELMODE` 가 루프를 끝내는지(E10)** · 붙기를 정한 순간 아직 안 들어간 루프(B3) · **떼자마자 Esc(들어가기 전 — D2): 루프가 서지 않고 원래 자리로 · 숨은 창이 남지 않음** · **키 놓기 · 붙기를 정한 직후(아직 안 들어간 루프) Esc(F2 — 끝은 `Ended` · `hid`): 숨은 창이 남지 않음** · **Esc 를 쥔 채 떼어 루프 중에 Esc 를 떼고 버튼을 놓기(F2 — `Released{false}` · `hid`): 놓은 창이 보임** | 20/20 놓기로 끝(그 자리 새 창 · 창째면 그 자리) · 루프가 남지 않음 · 끝 줄 `before_entry` 기록 · Esc 는 10/10 되돌리기 | §8 상태 표 |
| M18 | 대기 창 빼기 — 트레이 보이기 · 숨기기 · `--hidden` 자동 시작에서 예비 창 · 안내 창 · 남긴 창이 안 나옴 · 작업 표시줄 · Alt+Tab 에 안 보임(탐색기 재시작 뒤 포함 · `WS_EX_APPWINDOW` 를 끈 채인지 — E2) · 떼어 들인 뒤에는 보통 팝아웃처럼 보임 · **가상 데스크톱(E2)**: 다른 데스크톱으로 옮긴 뒤 떼기 · 안내가 지금 데스크톱에 나타나는지 · 대기 창이 어느 데스크톱 작업 보기에도 안 보이는지 · **남는 창 없음(F1)**: 창째 붙기 · 띠 놓기 · 떼어 붙기 · Esc 를 섞어 20 회 끈 뒤 우리 프로세스의 최상위 창이 「모델 창 + 예비 하나 + 안내 창」뿐인지(`EnumWindows` · 작업 관리자) · 끝 줄 `orphaned` 가 0 이 아닌 회차가 있으면 그 고아가 1 초 안팎에 사라졌는지(명부 로그) | 전부 문장 그대로 · 남는 창 0 | §2-5 · B7 · G2 · F1 |

## 13. 이행 순서

**규칙** — 단위 하나 = 워커 하나 · 단위가 끝날 때마다 `cargo build` · 워크스페이스 시험 · `npm test` · `npx tsc --noEmit` 이 선다. **자료구조와 그 호출자를 같은 단위에서 함께 옮긴다** — 구조를 먼저 갈아엎고 호출자를 나중에 맞추지 않는다. 각 단위의 파일 목록은 HEAD `9b74ff40` 에서 `git grep` 으로 호출부 · 구조체 리터럴 · 패턴을 세어 낸 것이다(B8 — 6판 목록은 U1 의 `commands/dock.rs` · `commands/popout.rs` · `dock/regions.rs` 를 빠뜨렸다). 워커 지시서에는 「목록 밖 파일이 깨지면 멈추지 말고 같은 단위에서 고치되 보고하라 · 중간 어디서 멈춰도 빌드가 서게 순서를 짜라」를 넣는다. 새 길은 옛 길 옆에 먼저 세우고(U1–U6), 웹뷰를 새 길로 옮긴 뒤(U7), 옛 길을 걷는다(U8). 그래서 U7b-2 앞까지는 옛 HTML5 끌기(HEAD 의 7 단계)가 계속 돈다. 각 단위 앞에 로컬 커밋으로 되돌릴 지점을 남긴다.

| 단위 | 내용 | 파일(겹침 ★ — 겹치는 단위) | 크기 · 앞선 단위 | 끝 기준 |
|---|---|---|---|---|
| **U0** | 9c 미커밋 코드 버림 — `tabStripLayout.ts` · `.test.ts`(9c 새 파일)만 남기고 코드 · 생성물을 HEAD 로 · 문서는 그대로 | `git diff --stat HEAD -- src src-tauri/src` 의 수정 파일 전부(`dock/hit.rs` · `dock/types.rs` · `layout/chrome.rs` · `TabBar.tsx` · `WindowLayout.tsx` · `dock/DockOverlay.tsx` · `dock/dockChromeReport.ts` · `dock/dockDrag.ts` 와 각 시험) · `dockDrag.shellConstants.test.ts`(9c 새 파일 — 지움) · git status 에 `M` 인 생성물(`src-tauri/bindings/*` · `crates/engram-dashboard-agent/bindings/*` — 생성물은 HEAD 로 되돌린 뒤 시험이 다시 구운 결과와 같은지 본다) | 작음 · **메인 세션이 G13 결정대로 한다** — ★버리기 전에 메인이 백업 패치를 만들고 검증한다(E5)★: 패치(`.claude/handoff/attachments/slot-docking-9c-code-U0-discard-2026-10-18.patch` — 2026-10-18 만들어 둠)가 HEAD 위에서 `git apply --check` 로 서고 · 그 패치가 덮는 파일 목록이 이 칸의 파일(9c 코드 · 9c 새 시험 · `M` 생성물) 전부와 같은지 대조한 뒤에 버린다 · 어긋나면 다시 만든다(옛 백업 = `slot-docking-9c-before-chrome-redesign-2026-10-18.patch` · `slot-docking-9c-all-2026-10-18.patch` — 문서 변경이 섞여 있다) | HEAD + 두 파일 · 게이트 초록 · 백업 검증 기록(step-log) |
| **U1** | §7 레이아웃 연산 여섯 칸(+ `FrameMeasures` · `EmptiedWindow.measures` · `dock_tab` 계약 문서 · `Keep` 은 비운 창에 탭 0 개 사건을 보내지 않음 · `DockTo` · `Docked` · `DockOutcome` 의 `Eq` 를 뺌 — E8) · 옛 호출자를 지금 동작 값으로(`share` = `DOCK_NEW_SHARE` · `keep_lease` false · `successor` 없음 · `Close` · 재기 비움) | `layout/dock.rs` · `layout/manager.rs`★(U8) · `layout/apply.rs` · `layout/mod.rs`(재수출이 필요하면) · `commands/dock.rs`★(U4 · U6 · U8 — 시험의 `DockOutcome` 리터럴 HEAD:1404 · 2052 · 2348 · 2454) · `commands/popout.rs`★(U4 — 시험의 `DockTo` HEAD:674 · 682 · 689 · 824 · `DockRequest` HEAD:841) · `dock/book.rs`★(U8 — `Action::Apply(DockRequest{…})` HEAD:2072 운영 + 시험) · `dock/hit.rs`★(U8 — `docked_tree` HEAD:211 운영 · 1442 · 1535 · 1540 시험) · `dock/regions.rs`★(U8 — HEAD:578 시험) · `tests/layout_apply.rs`★(U8) | 중간 · U0 | 기존 시험 + §11 U1 줄 초록 · 동작 무변경 |
| **U2** | 순수 기하 넷(`geometry` · `strip` · `sizing`(+ `offscreen_near` — E3) · `targets`(+ `OverlayView` 정의 · ts-rs — E8)) + `units.rs`(`PhysPoint` · `PhysRect` · `PhysSize` · `MonoMs` — 지금 `book.rs` 의 것은 U8 에서 함께 지워지므로 새로 둔다 · E8) + `world.rs`(명부 재기 없이 — 그 칸은 U4) — 새 파일 · 아직 안 부름 | `dock/{geometry,strip,sizing,targets,world,units}.rs`(새) · `dock/mod.rs`★(U4 · U5a · U5b · U6 · U8) · `src-tauri/bindings/OverlayView.ts`(새 생성물) | 중간 · U1(`docked_tree` 몫 인자 · `FrameMeasures`) | 표 시험 초록 · 생성물 sync |
| **U3** | platform 목적 함수(§8 — 6.2 몫: Posted 의 Esc 삼킴(D2) · 들어갈 때 `WM_CANCELMODE` 올림 · 재귀 깊이 가드 · 고리 안 소멸 시험 seam(E10) · DWM 전환 늘 끄기(E9) · `set_switcher_hidden` 의 `APPWINDOW`(E2) · `restore_placement` 좌표계(E12) · `move_window`(E3) · 6.3 몫: 모든 끝 알림의 `hid`(F2) · `place_window` 의 `rect: None` 제자리 보이기 · 활성 거절 시 Alt 우회 없음 · `is_window_visible`(F7)) · windows 기능 · `raw-window-handle` 의존 · 헤더 · CLAUDE.md platform 항목 | `crates/engram-dashboard-platform/{Cargo.toml, src/lib.rs, src/window.rs, src/move_loop.rs(새), src/input.rs(새)}` · `Cargo.lock`(platform 의존 목록 한 줄) · `CLAUDE.md`★(U9) | 중간 · U0 — **U1 · U2 와 나란히 가능**(겹침 없음) | platform 시험 초록 · 게이트 ④⑤⑥ · 의존 상한 그대로 |
| **U4** | 대기 창 명부 · 예비 창 · 안내 창 — 명부(`parked.rs`) · 짓기(작업 스레드 · tao 쪽 먼저 그 뒤 Alt+Tab 빼기 — E2) · 재기 받기 갈래(세 보고 처리기) · 세계의 명부 재기 칸(E8) · `window_role`(`{role, drag: None}` — 거울은 U6) · 트레이 술어 · 소멸(명부에서 뺌 · 예비 · 안내 창 다시 짓기 — 닫기 요청 갈래는 그대로) · 자리 기록 건너뜀 · Alt+Tab · 작업 표시줄 빼기 · 명부 창을 만지는 tao · Tauri 깃발 setter 자리 `rg` 확인(E2) · 권한 · `#/dock-overlay` 빈 페이지 · `WindowLayout` 대기 모드(0탭 자가닫힘 끔) · 6.3 몫: 명부 고아(역할 `Orphan` · `reveal` 목록 · 순수 `orphan_next` · 술어가 고아를 대기 창처럼 뺌 · 준비된 예비 판정에서 뺌 · 소멸 갈래가 고아를 지우고 `respawn` 이면 예비가 없을 때만 새로 지음 — F1) · `set_focus` 자리 `rg` 0(끌기 어댑터 · `dock/` · 명부 창 — F7 a) | `dock/parked.rs`(새) · `dock/world.rs`★(U2) · `dock/mod.rs`★ · `commands/popout.rs`★(U1) · `commands/layout.rs`★(U6) · `commands/dock.rs`★(U1 · U6 · U8 — `report_dock_chrome` 명부 갈래 · `window_role`) · `lib.rs`★(U6 · U8) · `tray/actions.rs` · `capabilities/dock-overlay.json`(새) · `src-tauri/bindings/{WindowRole,WindowRoleReply}.ts`(새 생성물) · `src/pages/DockOverlayPage.tsx`(새 — 아직 그리지 않음) · `src/App.tsx` · `src/components/layout/WindowLayout.tsx`★(U7b-2) · `WindowLayout.test.tsx`★(U7b-2) | 중간 · U1(`parked` 가 `FrameMeasures` 를 씀) · U2(`world.rs`) · U3(`place_window` · `set_switcher_hidden`) | 앱이 뜨면 화면 밖에 예비 · 안내 창 · 트레이 보이기 · 숨기기가 둘을 안 건드림 · 옛 끌기 그대로 돎(예비 · 안내 창은 모델 밖이라 옛 장부가 대상으로 보지 않는다 — 시험) |
| **U5a-1** | 세션 상태 기계 ① — 타입 전부(`Phase` · `Event` · `Effect`(실패 등급) · `Cont` · `RevertStep` · `Cell` · `Session`) · `accept` · `cell`(행렬 522 칸 전부 — 이름만) · 행렬 이름 표 시험 · `step` 의 I · W · S 행 전이(T01–T31 · T44) · 공통 마무리 · 되돌리기 계획 + `wire.rs` | `dock/session.rs`(새) · `dock/wire.rs`(새) · `dock/mod.rs`★ · `src-tauri/bindings/*`(새 wire 타입 생성물) | 큼 · U2 · U4(`parked`) — **파일럿**(실측 토큰 · 도구 호출로 U5a-2 · U5b 를 다시 잰다) | 행렬 이름 시험 · 그 전이들 초록 · 생성물 sync · M 행 전이(T32–T43)는 `step` 갈래가 이름으로 있되 몸이 `unimplemented!("U5a-2 Txx")`(그 칸의 전이 이름을 꼬리표로 — 예 `unimplemented!("U5a-2 T37")` · F8)라 컴파일은 선다(`_ =>` 없음) · `rg -n "unimplemented!" src-tauri/src/dock` 의 줄마다 「U5a-2 T」 꼬리표가 있음 · 6.3 몫(F1 ⓞ 등급 · x7 · F4 ⊥ 여덟 칸 · F5 `Abandon{by}` · F6 `mount` · 세션 번호 1 부터)이 이 단위의 타입 · `accept` · `cell` 에 든다 |
| **U5a-2** | 세션 상태 기계 ② — `step` 의 M 행 전이(T32–T43) · §2-3-5 대조 시나리오 전부를 사건 열로 끝까지(6.3 몫 F2 `hid` 칸 · F3 b `settled` 포함) | `dock/session.rs`★(U5a-1) | 중간 · U5a-1(같은 워커 이어서 권장) | §11 U5a 줄 초록 · `rg -n "unimplemented!\|todo!" src-tauri/src/dock` 0(F8) |
| **U5b** | 실행기 · 포트 `exec.rs` · `ports.rs` — 실패 등급 · 결과 블록(D1) · 사건 보강(`geom` — D4 · E4 · 줄에서 꺼낼 때 읽기 · `settled` 표 — F3) · 두 걸음 놓기 · 배율 바뀜 다시 놓기(E3 · 계기 = `Posted` 동안의 첫 `ScaleFactorChanged` — F3) · 정리 할 일 실패의 고아 넘김 · `OrphanDue` 를 명부로 · 끌기 끝의 `GaveUp` 다시 보기(F1) · 공통 마무리 뒤 보임 대조 · `Focus` = platform 활성(F7) · 가짜 포트 시험. ★시작 조건(F8)★ = `rg -n "unimplemented!\|todo!" src-tauri/src/dock` 가 0 줄 — U5a-2 가 끝났는지 기계로 본다(0 이 아니면 시작하지 않고 메인에 보고) | `dock/exec.rs`(새) · `dock/ports.rs`(새) · `dock/mod.rs`★ | 중간–큼 · U5a-2 | §11 U5b 줄 초록 |
| **U6** | 셸 연결 — 새 invoke 넷 · 시한 · `Abandon{by: Token \| Session}`(F5) · Tauri 어댑터(포트 구현 — 강제 소멸 `destroy` · tao `is_visible` 읽기 · 활성은 platform 만 — F1 · F7) · `Moved` · `Destroyed`(→ `WindowGone`) · `on_page_load`(→ `PageLoaded`) · `settled` → 세션 · 앱 수명 폴러 · 시한 스레드(setup — 고아 시한 `ArmOrphan` 포함 · F1) · `window_role` 의 `drag` 거울(E6 · 1 부터 단조 번호 — F6) · `dock:drag{Ended}` 를 `armed ∪ 모델` 에(F6) · 생성물(`mount` 가 든 invoke 타입 — F6) | `commands/dock.rs`★ · `commands/layout.rs`★(`TauriEvents::settled`) · `lib.rs`★ · `dock/mod.rs`★ · `src-tauri/bindings/*`(새 invoke 타입) | 중간 · U3 · U4 · U5b | 셸 시험 초록 · 새 명령이 등록됨(아직 웹뷰가 안 부름 — `dock:drag` 는 옛 `dock:session` 과 이름이 달라 둘이 섞이지 않는다) |
| **U7a** | TS 순수 — `tabStrip/layout.ts`(옮김) · `reorderGeometry.ts`(C1 · C2 · C3) · `closingMode.ts` | `src/components/layout/tabStrip/{layout,reorderGeometry,closingMode}.ts` + 시험(새) · `tabStripLayout.ts` · `.test.ts`(옮겨 지움 — U0 뒤에는 아무도 들이지 않는다) | 작음–중간 · U0 — **Rust 단위와 나란히 가능** | 표 시험 초록 |
| **U7b-1** | TS 새 모듈 — 아직 아무도 들이지 않는다: `useTabDrag`(seq · 답 기다림 붙들기 · token · 마운트 표 `mount`(F6) · `dock:feed{Release}` 답 — E7) · `measure` · `dockSession`(세션 번호로 카운터 · `window_role` 의 `drag` — E6 · 듣기 먼저 · `lastEnded` · 키 끝 하나 뒤 스스로 끔 · `feed{Attach}` 로 다시 켬 — F6) · `frameReport`(`tab_bar` = 끌기 영역 · 모양은 그대로 `tabs: []` · `scroll: null`) · `fileDropGuard` · `latestValueReporter` · `windowCanvasReport` 를 그 위로(공개 함수 그대로) | `tabStrip/{useTabDrag,measure}.ts`(새) · `dock/{dockSession,frameReport,fileDropGuard}.ts`(새) · `src/util/latestValueReporter.ts`(새) · 각 시험 · `windowCanvasReport.ts` · `.test.ts` | 큼 · U6(생성물) · U7a | vitest · tsc 초록 |
| **U7b-2** | TS 연결 · 옛 길 걷기(웹뷰) — `TabBar` 다시 씀 · `WindowLayout` 배선 · `DockOverlayView` 를 `OverlayView` 로 고쳐 `DockOverlayPage` 에서 그림 · 창 안 `DockOverlay` 걷음 · 옛 `dockDrag` · `dockPick` · `dragImage` · `dockChromeReport` 와 시험 지움 | `TabBar.tsx` · `.test.tsx`★(U7c) · `WindowLayout.tsx` · `.test.tsx`★(U4) · `dock/DockOverlay.tsx` · `.test.tsx` · `src/pages/DockOverlayPage.tsx`★(U4) · `dock/{dockDrag,dockPick,dragImage,dockChromeReport}.ts` + 시험(지움) | 큼 · U7b-1 | vitest · tsc 초록 · `/qa full` 화면(끌기 = M 은 아직) |
| **U7c** | 모양 — 활성 탭 · 구분선 투명도(R27 · C7) · 닫기 모드 연결(R26) · 빈 탭바 요소 · 6 px 자식(§9) | `TabBar.tsx` · `.test.tsx`★(U7b-2) | 작음 · U7b-2 | vitest · 화면 QA AC31 · AC32 |
| **U8** | 옛 길 걷기(셸) + **`DockChrome` 모양 바꾸기** — `book.rs` · `regions.rs` · `hit.rs`(남는 몫은 U2 로 옮겨 있음) · `types.rs` · `snap.rs` · 골든 · 옛 명령 여섯 · 옛 `dock:session` · `DockChrome.tabs` · `scroll` · `TabHeader` · `DOCK_NEW_SHARE` · `WebviewEnv` 주석 · 생성물 · 웹뷰가 새 모양을 보냄 | `dock/{book,regions,hit,types,snap}.rs` · `dock/pick_golden.json` · `dock/mod.rs`★ · `commands/dock.rs`★ · `lib.rs`★ · `layout/chrome.rs` · `layout/dock.rs` · `layout/manager.rs`★(U1 — `DockChrome` 시험 리터럴) · `tests/layout_apply.rs`★(U1 — `DockChrome` 리터럴) · `webview_env.rs` · `src-tauri/bindings/*`(지움 · `DockChrome.ts` 다시 구움) · `src/components/layout/dock/frameReport.ts` · `.test.ts`★(U7b-1) | 중간 · U7b-2 · 화면 QA 뒤 | 게이트 · CI sync 초록 · `rg "dock_drag_\|DragOverReport\|pick_golden\|dock:session"` 0 |
| **U9** | 문서 — CLAUDE.md 「LLM-우선 제어」 남은 갭(마우스 끌기 결과 = `dock_tab` · 명령은 T-65) · CLAUDE.md 「창 구성」(미리 지은 예비 창 — 모델 밖 보통 팝아웃 · 안내 창 `dock-overlay` · 경로 `#/dock-overlay` — E12) · step-log · ADR(§16) | 문서 · `CLAUDE.md`★(U3) | 작음 · 메인 세션 · `/review doc` | — |
| **M** | §12 실측 M1–M18 | — | 메인 세션 + 사용자 · U7c 뒤(M2 · M17 · M1 일부는 U6 뒤 하네스로 먼저 가능) | §12 합격 · 불합격이면 §15-2 |

- **`DockChrome` 모양을 U8 에 두는 까닭**(B8) — 6판은 U7b 의 `frameReport` 가 `{canvas_origin, tab_bar}` 만 보내는데 Rust `DockChrome` 은 U8 까지 `tabs` 를 요구해(`layout/chrome.rs` — `tabs: Vec<TabHeader>` 는 serde 기본값이 없다) 그 사이 크롬 보고가 전부 역직렬화에 실패했다. 이 판은 「새 모양을 처음 보내는 단위가 Rust 모양 · 생성물 · TS 타입을 함께 바꾼다」로 정하고, U7b-1 은 지금 모양에 `tabs: []` · `scroll: null` 을 실어 보낸다(뜻만 끌기 영역으로 — §4). 옛 판정(`hit.rs` · `regions.rs`)이 빈 머리 목록을 받는 것은 U7b-2 뒤라 옛 끌기가 이미 꺼져 있다.
- **새 세션 사건 이름 `dock:drag`**(B8) — 옛 `dock:session` 은 U8 까지 옛 장부가 낸다. 같은 이름을 쓰면 U6 부터 U8 까지 두 모양이 한 이름에 실려 U7b-2 를 원자적으로 맞춰야 했다. 이름을 달리해 그 조건을 없앴다(G10).
- **`raw-window-handle` 은 U3 의 platform 의존이다**(B8 · B10) — 셸이 핸들을 처음 쓰는 단위는 U4 지만 셸은 이 crate 를 의존하지 않는다(§8).
- **`DockOverlay` 고쳐 쓰기를 U7b-2 에 두는 까닭** — `DockOverlayView` 의 새 입력 `OverlayView` 는 U2 의 `targets.rs` 생성물이다(E8 — 6.1 은 U5a 의 `wire.rs` 에 두었는데 U2 의 `targets.rs` 가 먼저 쓴다). U4 는 경로와 빈 페이지만 세우고, 창 안의 옛 `DockOverlay` 는 옛 끌기가 꺼지는 U7b-2 까지 그대로 둔다.
- **겹치는 파일**(순서대로 돌린다) — `dock/mod.rs`(U2 · U4 · U5a-1 · U5b · U6 · U8) · `dock/world.rs`(U2 · U4) · `dock/session.rs`(U5a-1 · U5a-2) · `commands/dock.rs`(U1 · U4 · U6 · U8) · `commands/popout.rs`(U1 · U4) · `commands/layout.rs`(U4 · U6) · `lib.rs`(U4 · U6 · U8) · `layout/manager.rs` · `tests/layout_apply.rs`(U1 · U8) · `dock/{book,hit,regions}.rs`(U1 · U8) · `WindowLayout.tsx` · `.test.tsx`(U4 · U7b-2) · `DockOverlayPage.tsx`(U4 · U7b-2) · `TabBar.tsx` · `.test.tsx`(U7b-2 · U7c) · `frameReport.ts`(U7b-1 · U8) · `CLAUDE.md`(U3 · U9). 나란히 돌릴 수 있는 묶음 = {U1 → U2} ∥ {U3} ∥ {U7a}.
- **크기 경고** — U5a · U5b · U7b-1 · U7b-2 가 가장 크다. 6.2 는 U5a 를 행렬의 행으로 둘로 나눴다(U5a-1 = I · W · S 행 + 타입 · `accept` · `cell` 전부 · U5a-2 = M 행) — 접점 = §2-3 의 타입과 `cell` 의 이름(메인이 지시서에 먼저 못 박는다 · 이 문서의 행렬이 그대로 정본). U5a-1 을 파일럿으로 돌려 실측(토큰 · 도구 호출)으로 U5a-2 · U5b 를 다시 잰다. U7b-1 과 U7b-2 의 접점 = `useTabDrag` 의 입력 포트 타입과 `dockSession` · `frameReport` 의 공개 함수 — U7b-1 지시서에 메인이 먼저 적는다.
- **지우기 · 되돌리기 어려운 작업**(조직 규칙) — U0 은 메인 세션이 G13 결정대로 하되 버리기 전에 백업 패치를 검증한다(위 U0 칸 · E5). U7b-2 · U8 의 지우기는 각 단위의 화면 QA 뒤에 한다.

## 14. 구현 갈림길 — 결정됨 (사용자 위임 · 메인 · 2026-10-18)

**★사용자 위임 2026-10-18 — 「니가 알아서해 경합은 다 나왔잖아」 · 「지금 내가 딱히 결정필요한건 없잖아. 구현까지 쭉 진행해」★** — 그래서 아래 ◆ 갈림길(사용자가 체감하는 것)도 메인 세션이 정했다. 6.1 은 「메인 결정」으로만 적어 위임 근거가 빠져 있었다(E5 · CLAUDE.md 「결정 규율」 — 체감하는 것은 사용자 결정이므로 위임 문장이 근거다). 실측(§12)이 이 결정을 뒤집을 근거를 내면 §15-2 대로 사용자에게 다시 올린다.

Chrome 에서 오지 않은 것 · 이 스택에만 있는 선택이다. 메인 세션이 6판의 추천을 그대로 채택하되 G9 · G13 은 달리 정했고, G2 · G7 · G10 · G12 · G16 은 리뷰 1 라운드(B7 · B8 · B10) · Chromium 확인(C1 · C3)에 맞춰 고친 채로 채택했다. ◆ = 사용자가 체감하는 것(동작 · 자원 · 모습) · ◇ = 안 보이는 내부(CLAUDE.md 「결정 규율」).

| # | 갈림길 | 고른 것 | 까닭 · 6판에서 바뀐 곳 |
|---|---|---|---|
| G1 ◇ | 탭바 안 순서 이동을 누가 모나 | **(a)** 캡처를 쥔 웹뷰(출처) · 붙은 창은 그 웹뷰가 셸이 민 커서로 | Chrome 꼴 그대로(캡처를 쥔 쪽이 몬다 — 조사 A1 · A7)이고 탭바 안 기하가 TS 한 곳에 모인다 |
| G2 ◆ | 예비 창이 기다리는 모습 | **(a)** 화면 밖 · 보임 · 트레이 · 작업 표시줄 · Alt+Tab 밖 | 스파이크가 잰 「화면 밖에서 보이는 창」 그대로에 사용자에게 보이는 흔적만 뺀다. ★B7 로 고침★ — 6판의 `skip_taskbar(true)` 하나로는 Alt+Tab 이 빠지지 않고(가능성 높음 — `ITaskbarList::DeleteTab` 은 작업 표시줄 몫) 트레이 보이기 · 숨기기 · `--hidden` 이 그 창을 건드렸다. 이 판 = 셸 대기 창 명부(트레이 · 자리 기록 · 닫기 요청) + tao `skip_taskbar`(작업 표시줄) + platform `set_switcher_hidden`(Alt+Tab — `WS_EX_TOOLWINDOW` 켬 · `WS_EX_APPWINDOW` 끔 · 뺀 뒤 tao 깃발 setter 금지 — E2) · 안내 창 · 남긴 창(R10)도 같다(§2-5). 들일 때 끄는 비트가 틀을 바꾸는 깜빡임 · 어긋남은 M1 · M18 · S-g |
| G3 ◆ | 예비 창을 짓는 때 | **(a)** 앱이 뜬 뒤 곧바로 하나 · 쓰면 끌기 끝에 다시 | 떼는 순간 늘 준비돼 있다(S8 pool). 대가 = WebView2 창 하나의 메모리가 늘 든다(M1 때 적는다) |
| G4 ◇ | 붙은 뒤 커서 폴링 주기 | **(a)** 8 ms | 스파이크 S4 가 잰 값(지연 p50 0.3 · 최대 11.5 ms · 빠짐 0). 바뀔 때만 보내므로 멈춰 있으면 사건이 없다 |
| G5 ◆ | 띠 최소 두께 `BAND_MIN_PX` | **(b)** 12 px | 조사 C 의 피어(Lumino 뿌리 위 가장자리)가 실제로 쓰는 가장 작은 가장자리 px · 폭 50 칸(AC39)에서도 좌우 띠가 칸 절반을 넘지 않는다 |
| G6 ◇ | 안내 창 수 | **(a)** 하나를 대상 창 위로 옮김 | 스파이크 S7 이 잰 꼴 · 미리 짓기 · 메모리가 적다 |
| G7 ◆ | 탭바 구역의 가로 범위 | **Chrome 의 탭 끌기 영역**(첫 탭 왼쪽부터 `A = max(available, Σ 보이는 폭)` · 반열림) | 6판 추천 (a) 는 「C3 결과가 다르면 Chrome 그대로」를 달고 있었다. C3 결과는 (a) 탭바 줄 전체도 (b) 탭들 범위도 아닌 끌기 영역이다(tab_strip.cc#530-552 · TDC#1461-1477 · §5-2) |
| G8 ◇ | 웹뷰 드롭 처리기(`WebviewEnv::finish` 의 `disable_drag_drop_handler`) | **(a)** 끈 채 두고 TS 파일 막기(AC22) 유지 | AC22 가 이 상태로 실측 합격했다(5.8 §8-1) |
| G9 ◇ | 붙을 때 처음 자리 | **Chrome 규칙** — 셸은 첫 자리(0 — 첫 non-pinned)에 넣고 대상 웹뷰가 같은 자리 함수(`insertionIndex`)를 문턱 없이 돌려 곧바로 고친다 | ★메인 결정(위임) · 6판 (a) 「맨 뒤에 넣고 고침」을 버림★ — Chrome `AttachToNewContext` 가 그렇게 한다(TDC#1513-1531 · dragging_tabs_session.cc#56 · C1). 자리 규칙은 여전히 TS 한 곳이고 거울이 없다 |
| G10 ◇ | 이름 | invoke `tab_drag_begin` · `_reorder` · `_leave` · `_end` · `window_role` / 사건 **`dock:drag`** · `dock:feed` · `dock:overlay` · `dock:spare{Parked \| Adopted}` / 안내 창 label `dock-overlay` · 경로 `#/dock-overlay` / 예비 창 = 보통 팝아웃 label `slot-popup-N` · **URL 표시 없음** / 크롬 보고 `report_dock_chrome` 이름 유지 | 탭바 안 일은 `tab_drag_*`, 창 사이 일은 `dock:*`. ★고침★ — 세션 사건은 옛 `dock:session` 과 겹치지 않게 `dock:drag`(B8) · 예비 표시 `&spare=1` 은 들인 뒤에도 남아 걷고 `window_role` · `dock:spare` 로(B7). 예비 창 label 을 팝아웃과 같게 두는 까닭 = 들이면 보통 팝아웃이 되는데 label 은 바꿀 수 없다(§2-5) |
| G11 ◇ | 예비 창 수 | **(a)** 하나 | 한 끌기 안에서 그 탭은 한 곳뿐이라 예비 하나로 늘 된다(§2-5) |
| G12 ◇ | 끌기 끝에 남은 빈 예비 창 · 비운 창 | **(a)** 예비로 되돌림(새로 짓지 않음) · 비운 출처 창은 닫음 | 보이는 결과는 같고 짓기 비용이 없다. ★B7 로 고침★ — 예비로 되돌린 창 · 끄는 중 남긴 창(R10)은 둘 다 대기 창 명부에 들어 트레이 · 작업 표시줄 · Alt+Tab 밖이다(6판은 화면 밖에 보이는 채 그 셋에 나왔다) |
| G13 ◆ | 9c 미커밋 코드 | **(a) 버림 — 메인 세션이 U0 에서 한다**(버리기 전에 백업 패치를 만들고 검증한다 — §13 U0 · E5) | ★메인 결정(위임)★ — `TabBar` 는 포인터 끌기로 다시 쓰고, 9c 의 탭 폭 규칙(`tabStripLayout.ts` · 시험)만 남겨 `tabStrip/layout.ts` 로 옮긴다 |
| G14 ◆ | 메인 마지막 탭이 메인 탭바로 돌아올 때 자동 빈 탭을 지우는 조건(R14) | **(b)** 그 탭이 아직 빈 탭일 때만 | 끌기가 만들지 않은 내용을 끌기가 지우지 않는다. 검사 = 칸 하나 · 내용 없음 |
| G15 ◇ | 끄는 동안의 키 출처 | **(a)** 앱 전체 키 감시 — 끄는 동안 모든 우리 웹뷰의 keydown | Chrome 의 앱 안 `EventMonitor` 꼴(조사 E1 · C9)이고 OS 훅을 끌기 내내 걸지 않는다. 우리 창이 앞에 없을 때의 키는 받지 않는다(Chrome 과 같다) |
| G16 ◇ | 창 핸들을 넘기는 길 | **(a) 를 고쳐 채택** — `raw-window-handle` 은 **platform** 이 의존하고 `&impl HasWindowHandle` 을 받아 변이를 platform 안에서 맞춘다 · 셸 의존 없음 | ★B10 으로 고침★ — 6판 (a) 는 셸이 `RawWindowHandle::Win32` 를 맞췄다 — `cfg` 는 없어도 platform 밖의 런타임 OS 가름이다. (b) 셸 `#[cfg(windows)]` + `hwnd()` 는 게이트 ④ 위반 그대로 |
| G17 ◇ | 순서 이동 쓰기마다 저장 | **(a)** 지금 기록기 그대로 | 새 장치를 짓지 않는다. 쓰기 수 = 순서가 실제로 바뀐 횟수(문턱 뒤)라 많지 않다 · 끄는 중 꺼져도 꺼질 때 자리(R11a) |
| G18 ◇ | begin · leave 가 메인 스레드 답을 기다리는 시한 | **(a)** 1 초 | 둘 다 루프가 돌지 않는 때라 답은 ms 안에 온다. 넘으면 `Abandon` 으로 그 끌기를 되돌리고 끝낸다(B6). 같은 값을 놓기 확인(T23 — Strip{Pointer} 버튼 감시 · 셸 폴러가 본 뗌 · holder 아닌 창의 키 놓기 · E7)의 시한에도 쓴다(새 수를 짓지 않는다) |
| G19 ◇ | 띠 두께 · 영역의 px 반올림 | **(a)** 가장 가까운 정수(.5 는 0 에서 멀리) | 10% 를 한쪽으로 치우치지 않게 · 그리기와 판정이 같은 Rust 정수 사각형이라 「칠한 곳 = 붙는 곳」은 선다 |

## 15. 정지 조건 · 위험 · 미결

### 15-1. Chromium 소스 확인 — 닫힘 (2026-10-18)

Chromium main(2026-10-18 받음 · `?format=TEXT` 원본을 풀어 둔 사본 `.claude/handoff/attachments/chromium-src-2026-10-18/` — 줄 번호는 그 사본 기준 · TDC = `chrome/browser/ui/views/tabs/dragging/tab_drag_controller.cc`)로 열 개를 다 닫았다. 「6판 기본값」 칸이 「틀림」인 것은 이 판에서 고쳤다.

| # | 무엇 | Chrome 규칙(출처) | 확신 | 6판 기본값 → 이 판 |
|---|---|---|---|---|
| C1 | 탭바 안 삽입 자리 · 「마지막 이동점」 · 붙을 때 자리 | 후보 자리마다 끄는 탭의 이상적 x 를 셈해 지금 x 와의 거리가 가장 작은 자리(끄는 탭 뒤면 −1 · 보이는 범위로 자름)(tab_strip.cc#554-609 · #980-1044 · #1046-1119) · 마지막 이동점은 **모델 자리가 실제로 바뀐 때만** 새로 적음 · 문턱 = `round(to 자리 탭 폭 ÷ 표준 폭 × 16)` · 붙은 직후 · 처음 이동은 문턱 없음(dragging_tabs_session.cc#83-104 · #143-151) · 붙을 때는 첫 non-pinned 자리에 넣고 같은 함수로 고침(TDC#1513-1531) | 확실 | 빈칸 → §5-1 표 · 붙기 = 「맨 뒤」(G9 a) **틀림** → 첫 자리 0 + 같은 함수(G9 결정) |
| C2 | 이웃 200 ms 비켜남의 곡선 | 200 ms(`RichAnimationDuration` — 풍부한 애니메이션 꺼짐이면 0) · 기본 tween `EASE_OUT` = `1 − (1 − t)²`(tab_container_impl.cc#175-191 · #1153-1160 · bounds_animator.h#205 · #207 · bounds_animator.cc#205-206 · tween.cc#72-73 · animation.cc#114-116) | 확실 | 빈칸 → §5-1 `slideTargets` |
| C3 | 「끌기 영역 폭」 | `max(GetAvailableWidthForTabContainer(), tab_strip 폭)` — 탭들이 차지한 범위보다 넓다(닫기 모드를 무시한 쓸 수 있는 폭) · 끄는 탭 x 를 `[0, max(0, 영역 − 끄는 폭)]` 로 자름(dragging_tabs_session.cc#339-343) · 떼기 판정 x 도 `[BeginX, EndX)` 같은 폭 · y 는 탭바 ± 15(tab_strip.cc#530-552 · tab_container_impl.cc#708-714 · TDC#1461-1477 · #401 · #234) · 새 탭 단추 앞에서 끝날 가능성(horizontal_tab_strip_region_view.cc#872-911 — 그 경계가 이 폭으로 이어지는 연결은 미확인) | 식 = 확실 · 뜻(`+` 앞에서 끝) = 가능성 높음 | 끄는 탭 x = 탭들 범위 · 구역 = 줄 전체(G7 a) **둘 다 틀림** → 끌기 영역(§5-1 · §5-2 · G7) · PRD R23 문장 세부 정정(부록 A) |
| C4 | 최대화된 탭 하나 창을 창째 끌 때 자리 | 복원(`Restore`) 뒤 크기 = `CalculateDraggedWindowSize` · `drag_offset = 누른 점 − 복원 원점` · 루프 시작 때 `SetBounds(커서 − drag_offset)` — 복원 원점에서 누른 뒤 커서가 움직인 만큼 옮긴 자리 · 커서 밑으로 다시 맞추지 않음(TDC#829-846 · #2749-2758 · #1797-1814 · #1967-1969) | 확실 | `detached_origin` 꼴 **틀림** → `whole_window_origin`(§5-5) · 보통 창도 같은 식 |
| C5 | 떼어진 창 크기 두 규칙의 순서 | 시작 = 출처 복원 크기(TDC#609-610) → **먼저** 작업 영역을 넘는 변 = 작업 영역 − 2 × 10 DIP → **마지막** 출처가 최대화면 각 변 = `max(최대화 크기 ÷ 2, 값)`(절반이 이긴다)(TDC#2760-2791 · #129) | 확실 | 절반 → 상한(상한이 이김) **틀림** → 상한 → 절반(§5-5) |
| C6 | 작업 영역을 재는 모니터 | 커서(`last_point_in_screen_`)에 가장 가까운 디스플레이(TDC#2765-2768) | 확실 | 커서 모니터 → 커서에 **가장 가까운** 모니터(점을 품은 모니터가 없어도 — §5-5) |
| C7 | 구분선을 서서히 숨기는 시간 | 따로 된 애니메이션이 없다 — 불투명도 = `1 − max(이 탭 호버값, 이웃 호버값)`(이웃이 활성이면 그 값은 0) · 활성 · 선택 바뀜은 곧바로 0 · 호버값 = 200 ms 미끄럼(들어감 EASE_OUT · 나감 EASE_IN `t²`)(horizontal_tab_style_views.cc#536-552 · #631-648 · glow_hover_controller.cc#39-66 · .h#32 · tab.cc#354 · tween.cc#37-38 · #72-73) | 확실 | 빈칸 → §5-7 |
| C8 | 붙은 창에서 다시 뗄 때 그 창의 활성 | 붙을 때 그 창 선택을 `selection_model_before_attach_` 로 적고(TDC#1514-1515), 뗄 때 그 활성 자리가 남은 탭 수보다 작으면 그 선택을 되살림 · 아니면 출처에서 떼는 것이면 처음 선택(TDC#1745-1771 · #1712-1723) — 붙기 전 활성이 승계보다 앞선다 | 확실 | 승계 **틀림** → 붙기 전 활성 자리(§7 ④ · §6-2 ①) |
| C9 | 끝내는 키 — 키 반복 · 쥔 키 · 루프 중 | 탭바 안: `EventTracker` 가 `kKeyPressed` 를 봐 Esc = 취소 · 그 밖 = 완료 · 반복 · 미리 쥔 키를 거르지 않음(TDC#302-336 · #570-576) / Windows 이동 루프: OS 가 키보드를 쥐고 Chrome 훅은 `VK_ESCAPE` 만 봄 · 결과는 `WM_LBUTTONUP` 을 봤느냐로만 갈림 → 루프를 끝낸 Esc · Enter 는 취소(hwnd_message_handler.cc#139-160 · #189-213 · #942-961 · desktop_window_tree_host_win.cc#657-679 · TDC#1993-1996 · #2041-2045) | 코드 = 확실 · 루프 중 `EventTracker` 가 키를 보는지 · Enter = 취소 = 불확실(추론 → M5) | 「모든 눌림 = 놓기 · 루프 중 훅의 그 밖 키 = 놓기」 **루프 단계 틀림** → 단계별(§6-3) · PRD R12 세분(부록 A) |
| C10 | 붙을 때 대상 창 활성 | 붙는 모든 길에서 `attached_context_->GetWidget()->Activate()`(TDC#1229-1236 · #1260-1265 · #2033-2039) | 확실 | 활성으로 — 맞음 |

### 15-2. 정지 조건 — 실측이 Chrome 방식을 막으면 (PRD R28 · ADR-0295 결정 2)

아래 실측이 불합격이면 **대안을 짓지 않고 멈춰 근거를 들고 사용자에게 올린다**. 지금 막힌 것은 없다(스파이크 110 회 통과 · Chromium 확인에서도 이 스택에서 못 하는 규칙은 나오지 않았다) — 스파이크가 재지 못한 갈래가 남아 있을 뿐이다.

| # | 조건 | 왜 이 스택만의 위험인가 |
|---|---|---|
| S-a | M1 불합격 — 실제 내용이 든 탭을 뗄 때 빈 화면 · 깜빡임 또는 100 ms 넘음 | Chrome 은 이미 그려진 WebContents 를 새 창으로 옮긴다. 우리는 그 탭을 다른 웹뷰가 다시 그린다(ADR-0057 라우팅 · 슬롯 내용 = 모델 데이터) — 스파이크 S8 은 미리 그려 둔 화면만 쟀다 |
| S-b | M2 불합격 — 실 앱에서 펌프로 시작한 루프 동안 IPC 가 멈춤 | 스파이크 펌프 시작 표본이 1–2 회뿐이다 |
| S-c | M6 — 메인 최대화를 활성화 없이 되돌릴 수 없음 | Chrome 동작이 아니다(Chrome 은 빈 출처 창을 닫는다) — 우리 확장의 메인 제안(PRD R14)이라 사용자에게 다시 묻는다 |
| S-d | M3 — 붙은 뒤 대상 웹뷰 밖에서 놓기를 못 알아챔 · 폴러로 민 순서 이동이 끊김 | 스파이크 「붙은 뒤 B 밖에서 놓기」 미검 |
| S-e | M5 — 탭바 안에서 「Esc 밖의 키 = 놓기」가 서지 않거나 끝낸 키가 슬롯에 들어감 · 창째 끌기의 Esc 를 훅도 키 감시도 받지 못함(근거 없음 b) | Win32 루프 · WebView2 자식 포커스 몫 미검(조사 E1 · C9) |
| S-f | M10 — 배율이 다른 모니터 사이에서 떼어진 창 · 안내 창 자리 · 크기가 틀림 · 두 걸음 놓기(E3)로도 떼어진 창 크기가 한 번 더 늘거나 줄거나 보이는 순간 깜빡임 | 스파이크 혼합 DPI 미검 · tao 의 `WM_DPICHANGED` 크기 고침과 화면 밖 창의 배율 받기 시점은 코드 · 스파이크 관찰뿐이다(§5-5) |
| S-g | M1 · M18 에서 대기 창 빼기(`set_switcher_hidden` · `skip_taskbar`)를 들일 때 끄는 일이 깜빡임 · 지연 · 자리 어긋남을 냄 · 뺀 창이 Alt+Tab · 작업 표시줄에 저절로 돌아옴(tao 가 우리가 부르지 않은 깃발 바뀜으로 `GWL_EXSTYLE` 을 다시 씀 — §2-5 불변식이 이 스택에서 지켜지지 않음 · E2) · 가상 데스크톱에서 대기 창 · 안내 창이 다른 데스크톱에 묶임 | 잰 적 없음 — G2 를 사용자에게 다시(Chrome 에는 대기 창이 없다 — §1 스택 맞춤 ① 의 대가) |
| S-h | M17 — 루프에 들어가기 전에 뗀 버튼을 놓쳐 루프가 버튼 없이 돈다 · 들어갈 때 올린 `WM_CANCELMODE` 가 루프를 끝내지 못한다(E10) · 들어가기 전 Esc 뒤에 루프가 서거나 숨은 창이 남는다(D2) | 뗌이 웹뷰 자식 창으로 가면 메인 스레드 훅이 못 본다(근거 없음 a) · 펌프 시작이라 보내고 들어가는 사이에 틈이 있다(§1 스택 맞춤 ②) — Chrome 은 동기 시작이라 이 틈이 없다 |

### 15-3. 위험

| # | 위험 | 대응 |
|---|---|---|
| W-1 | 예비 창이 늘 메모리를 쓴다(G3 a) | M1 때 잰다 · 크면 G3 를 다시 |
| W-2 | tauri#10767(창 끌기가 포커스를 뒤집고 뗌을 먹음 — PRD W5) | 우리 루프는 tao `start_dragging` 이 아니라 platform 의 `SC_MOVE\|2` 다(스파이크 S3 의 ipc · post 꼴) — M3 · M4 에서 뗌 · 포커스를 본다 |
| W-3 | 같은 스레드 재진입(`SetWindowPos` · `ShowWindow` 가 보낸 메시지가 `Moved` 로 돌아옴) | 실행기의 한 번에 하나씩 줄(§2-4) · U5b 시험 |
| W-4 | 끄는 중 닫힌 창의 낡은 핸들 | platform 함수가 `IsWindow` 로 걸러 `Err` · 루프 중 소멸은 `Destroyed` 끝 알림(§8) · 세션은 그 창을 세계에서 다시 본다(`WorldChanged`) |
| W-5 | 쓴 예비 창의 옛 화면이 다음 떼기 때 한 프레임 보임 | `dock:spare{Parked}` · M1 에 「같은 끌기에서 두 번째 떼기」를 넣었다 |
| W-6 | 크롬 보고 100 ms 디바운스 — 이름을 막 바꾼 탭바의 구역이 늦음 | 받아들임(Q19 「자연스럽게」) |
| W-7 | 보안 소프트웨어가 훅을 막음 | 스레드 지역 훅을 루프 동안만 건다(저수준 훅 아님 — G15) · 이 PC 에서 스파이크 110 회 통과 · 실패면 끝 까닭 = `Released`(E7) |
| W-8 | 끄는 중 순서가 바뀔 때마다 저장(G17 a) | 쓰기 수가 적다 · 문제면 G17 b |
| W-9 | macOS — 이동 루프 · 훅이 없다 | `Unsupported` 로 끌기를 되돌림 · 이식 때 조사(PRD R19 · W11) |
| W-10 | `book.rs` 를 걷으면 `Snap` 타입 · 숨기기 맞추기 · `regions_for` 의 `StripStart` 인자가 함께 빠진다(코드 확인 2026-10-18 — `Snap` 은 `book.rs` 정의 · `DockState` 가 `Book` 을 쥠 · `regions_for` 가 `StripStart` 를 받음) | U2 의 `world.rs` 가 `Snap` 을 대신하고, 숨기기 맞추기는 새 모델에 없다(포인터 캡처 창을 숨기지 않으므로 맞출 숨김이 없다) |
| W-11 | `DockOverlay.tsx` 의 바깥 래퍼가 `dockDrag` 스토어(`useDockGuide`)에 묶여 있다 — 안의 `DockOverlayView({ guide })` 는 이미 props 보기다(HEAD 그 파일 · B12 — 6판의 「props 없이 `useDockGuide` 만 읽는다」는 틀렸다) | U7b-2 에서 `guide` 타입을 `OverlayView` 로 바꾸고 래퍼를 걷는다 |
| W-12 | 안내 창을 배율이 다른 모니터로 옮기면 tao 가 `WM_DPICHANGED` 제안 크기로 바꿔 대상 창을 덮지 못함(근거 없음 c) | 배율 바뀜 사건 뒤 마지막 자리를 다시 놓음(§2-5) · M10 · 안 되면 S-f |
| W-13 | Alt+Tab 빼기 비트가 테두리 있는 창의 틀 두께를 바꿈 — 떼기 자리 · `original` 이 어긋날 수 있다 | 화면 밖에서 끄고 그 뒤에 읽은 틀 두께로 셈(§5-4 · §5-5) · M1 |
| W-14 | 시한을 넘긴 begin · leave 뒤 메인 스레드가 늦게 처리해 세션이 남음(웹뷰는 이미 포기) | `Abandon{by: Token \| Session}`(F5)이 그 세션을 되돌린다 — 정한 붙기 · 키 놓기도 덮는다(T24 · T33 · T34 · D3) · Strip{Pointer} 버튼 감시(T23) · holder 다시 읽힘 = 끝(T25) |
| W-15 | tao 가 창 깃발이 바뀔 때마다 `GWL_EXSTYLE` 을 통째 다시 써 Alt+Tab 빼기가 풀림(E2 — 코드 확인) | 뺀 창에 tao · Tauri 깃발 setter 를 부르지 않는다(§2-5 불변식 · U4 `rg`) · M18 · 안 되면 S-g |
| W-16 | 행렬이 커서(522 칸) 문서와 시험 표가 어긋남 | 정본 = 이 문서 · 시험 표는 옮긴 것 · 칸을 고치면 같은 변경에서 둘 다(§2-3) · `cell` 의 `match` 에 `_ =>` 없음 — 변이를 더하면 컴파일러가 잡는다 |
| W-17 | 정리 할 일(닫기 · 예비로 되돌리기 · 다시 보이기)이 실패해 아무도 쥐지 않은 창이 화면 밖 · 숨은 채 남음(리뷰 3 F1) | 실패 전에 명부 고아로 넘김 · 명부가 세션과 따로 다시 하고 한 단 올림(§2-5 「고아」) · U5b 시험 · M18 「남는 창 없음」 |
| W-18 | 날 Win32 숨기기 · 보이기를 tao 가 몰라 VISIBLE 깃발이 어긋남 — 트레이 보이기가 무동작(리뷰 3 F2 · F7) | 숨김은 `hid` · `hidden` 으로 세션이 늘 안다 · 공통 마무리 뒤 tao · OS 보임 대조 단언(§6-1) |

### 15-4. 미결 · 이월

- **T-65** — `tab.dock` · `layout.get` 이 `dock_tab`(§7)에 얹는다. 끄는 도중의 상태를 `layout.get` 이 본다(Q30).
- **T-66** — `app-region` 을 다는 일 · `A` 다시 끌어내기(5.8 §7 #73) · 창 단추 · 표식과 탭바 구역(끌기 영역 — §9 · C3)의 관계.
- **CLAUDE.md 「LLM-우선 제어」 남은 갭** — 마우스 끌기의 결과는 `dock_tab` 을 지나지만 명령은 T-65 몫이라는 한 줄(U9 · PRD D22 · Q10).
- **PRD 문장 고침(메인 몫)** — R23 「탭들이 차지한 범위 밖으로는 나가지 않고」 → 끌기 영역(C3) · R12 「Esc 밖의 눌린 키 = 놓기」의 따라오는 창 단계(C9) · ADR-0295 결정 1 의 「그 즉시 새 창」 · 루프 시작 방식(§1 스택 맞춤 · ADR-b). 이 TRD 는 부록 A 에 세분 · 정정으로 적었다.
- **가운데 클릭 닫기**(조사 A4 — 뗄 때 · 커서가 그 탭 위일 때만) — PRD 범위 밖이라 짓지 않는다.
- **PRD AC34**(빈 탭바 = 창 끌기 영역)는 T-66 때 잰다.

### 15-5. 리뷰 3 라운드 이월 — 단위 코드 리뷰가 다시 볼 것

리뷰 3 라운드가 `/review trd` 라운드 상한이라 6.3 의 고침(F1–F8)은 전체 재리뷰를 받지 않았다. 아래 단위의 `/review code` 지시서에 그 F 를 렌즈로 넣어, 설계 문장이 아니라 착지한 코드로 다시 본다(구멍이 나오면 그 단위에서 고치고 이 문서를 같은 변경에서 고친다).

| F | 다시 보는 단위 | 볼 것 |
|---|---|---|
| F1 | U4 · U5b | U4 = 명부 고아 역할 · `reveal` · `orphan_next` · 술어 · 소멸 갈래 · `respawn` 이 예비 하나(G11)를 지키는지 / U5b = ⓞ 할 일 실패가 `Failed` 보다 먼저 명부에 넘는지(아무도 쥐지 않은 틈이 없는지) · `CloseWindow` 늘 넘김 · `OrphanDue` 가 결과 블록 사이에 끼지 않는지 · 다시 하기가 세션이 쥔 창을 건드리지 않는지 · 바쁜 되풀이가 없는지 |
| F2 | U3 · U5a | U3 = 모든 끝 알림에 `hid` · Posted 의 끝냄 청 + Esc · Esc 뗌 뒤 놓기 / U5a = T37 ② · T38 · T39 가 끝 까닭과 무관하게 `hid` 로 적는지 · 공통 마무리 3 이 그 창을 다시 보이는지 |
| F3 | U5b | 다시 놓기 계기(`Posted` 동안 첫 `ScaleFactorChanged` · `Moved` 무시) · `settled` 표 · `geom` · `probe` 를 꺼낼 때 읽기 · `offscreen_near` 의 커질 크기 + 여백(순수 함수는 U2 의 `sizing.rs` — U5b 리뷰가 부르는 쪽과 함께 본다) |
| F4 | U5a | W 열 × `Failed(Soft)` = ⊥ · 칸 수(전이만 124 · 무시만 387 · 섞임 11)가 시험 표와 같은지 · T44 가 S · M 열에만 · `hidden` 이 집합 |
| F5 | U6 · U7b | U6 = leave 시한이 `Abandon{by: Session}` 을 짓고 Strip · Moving 에서 받아지는지 · begin 은 `Token` / U7b = 붙은 창 웹뷰가 begin 표 없이도 leave 시한을 버리는지(웹뷰 쪽 처리 그대로) |
| F6 | U6 · U7b | U6 = 세션 번호 1 부터 단조 · 거울 · `Ended` 가 `armed ∪ 모델` 에 / U7b = 듣기 먼저 · `lastEnded` · 키 끝 하나 뒤 스스로 끔 · `feed{Attach}` 로 다시 켬 · `mount` 를 끌기 사건 셋에 실음 |
| F7 | U4 · U5b | U4 = `set_focus` 자리 `rg` 0 · 명부 창 깃발 setter 0 / U5b = `Focus` · 붙기 활성이 platform 활성만 · 공통 마무리 3 의 `rect: None` · 보임 대조 단언(어댑터의 tao 쪽 읽기는 U6 이 짓는다 — U5b 리뷰는 포트 계약을 본다) |
| F8 | U5a · U5b | U5a-1 = M 행 전이 몸의 `unimplemented!("U5a-2 Txx")` 꼬리표 / U5a-2 끝 · U5b 시작 = `rg -n "unimplemented!\|todo!" src-tauri/src/dock` 0 |

## 16. ADR 제안

번호는 `/adr` 가 준다. 「거부한 대안」은 ADR-0295 · 스파이크 · 조사 · 점검 문서 · 리뷰 1 라운드에 있는 것만 적었다 — 새로 지은 거부 사유는 없다.

- **ADR-a 끌기 사건 출처 = 단계마다 정해짐 · 세션 = 셸 메인 스레드 하나 · 쓰기 결과는 다음 사건 · 웹뷰 사건은 창마다 단조 seq**(§2-3 · §2-4 · §4 · G1)
  - 결정: 탭바 안은 포인터 캡처를 쥔 웹뷰가 몰고, 떼어진 뒤는 셸 세션이 OS 이동 루프의 사건으로, 붙은 뒤는 셸이 민 커서로 그 창 웹뷰가 몬다. 키는 세 단계 내내 앱 전체 키 감시(Chrome `EventTracker`)다. 세션 상태 기계는 메인 스레드에서 사건을 하나씩 끝까지 처리하는 순수 `step` 이고, 레이아웃 쓰기는 묶음의 마지막 할 일로 그 결과가 줄 맨 앞으로 돌아온다. 웹뷰 사건은 보낸 창마다 단조 seq 와 절대값을 싣고 낡은 것은 버린다(출처 · 단계를 seq 보다 먼저 본다). 실패할 수 있는 할 일은 실패를 사건으로 되돌리고 관문이 실패하면 그 묶음을 멈춘다. 세션 계약은 상태 꼴 × 사건 꼴 행렬 하나가 정본이고 칸마다 전이 이름 또는 무시 사유가 있다(빈칸 없음 · `match` 에 `_ =>` 없음 · 시험이 행렬 전부를 돈다). 끝 · 실패 · 되돌리기는 쥔 것만 보고 되돌리는 공통 마무리 하나로 간다.
  - 거부한 대안: ① 붙은 웹뷰가 `setPointerCapture` 로 이어받기 — 누름을 못 본 웹뷰의 캡처는 drag 영역 · 창 밖에서 ~90 ms 뒤 끊긴다(스파이크 S4 0/22) ② 웹뷰끼리 캡처 넘기기 — 웹뷰 HWND 는 다른 프로세스라 셸이 `SetCapture` 할 수 없다(조사 B4) ③ 떼어진 창을 OS 루프 없이 셸이 `SetWindowPos` 로 옮기기(조사 P2) — 사용자 기각(ADR-0295) (순서 장치 · 쓰기 결과 되먹임의 꼴은 리뷰 1 라운드 A2 · A3 의 선택지 중 메인이 고른 것이다 — 버린 쪽의 거부 사유는 기록이 없어 적지 않는다. 행렬로 계약을 닫은 것은 리뷰 2 라운드 뒤 메인의 수렴 조건이다 — 「표에 없는 조합은 무시」 나머지 규칙을 둔 6.1 의 전이표는 두 라운드 내리 전이 수준 구멍이 나왔다.)
- **ADR-b 떼어 낼 창 = 미리 지은 예비 창 · 이동 루프 시작 = 메시지 펌프 · 붙은 뒤 셸 커서 폴링 · 대기 창 명부 · 안내 = 따로 된 항상 위 창 하나 — ★ADR-0295 결정 1 을 고친다★**(§1 스택 맞춤 · §2-5 · §8)
  - 결정: PRD R28 스파이크가 낸 스택 맞춤 셋(§1 표 — 2026-10-18 사용자 보고를 마침)을 박는다. 예비 창 하나를 모델 밖(대기 창 명부)에 두고 떼는 순간 모델에 들인다. 루프는 `ReleaseCapture` + `PostMessage(WM_SYSCOMMAND, SC_MOVE|2)` 로 시작하고 `WM_CANCELMODE` 로 끝내며, 보냈으나 아직 안 들어간 루프의 끝은 들어올 때 삼킨다. 붙은 뒤 그 창 웹뷰 밖의 커서는 셸이 8 ms 마다 읽어 민다. 모델 밖 창은 명부가 재기를 받아 두고 트레이 · 작업 표시줄 · Alt+Tab 에서 뺀다. 세션이 놓은 창의 정리(닫기 · 예비로 되돌리기 · 다시 보이기)가 실패하면 실행기가 실패를 내기 전에 그 창을 명부의 고아로 넘기고 명부가 세션과 따로 끝까지 정리한다(닫기의 성공 = 소멸 · 리뷰 3 F1). 안내는 투명 · 클릭 통과 · 활성화 안 됨 · 항상 위 창 하나를 날 `SetWindowPos(HWND_TOPMOST)` 로 옮겨 그린다. ADR-0295 결정 1 의 「그 즉시 진짜 새 창을 커서 밑에 만들어 … 창 이동 루프로 따라오게 함」 중 **수단**(그 즉시 만들기 · 끌기 처리 안 동기 시작 · 붙은 탭바에 캡처 주기)을 이 결정이 바꾸고 **동작**은 그대로 둔다 — `/adr link` 로 두 ADR 에 고침 링크를 단다(ADR-0295 본문 고침은 메인 몫).
  - 거부한 대안: ① 떼는 순간 창 짓기(Chrome `CreateBrowserForDrag` 꼴) — 검정 · 흰 화면 213–314 ms(스파이크 S8 fresh 0/20) ② IPC 처리기 안 동기 `SendMessage`(Chrome 호출 모양) — 루프 안 웹뷰 → 셸 IPC 왕복 0/21(스파이크 S3 · 동기 시작 58/58 run 전부 0) ③ `run_on_main_thread` 안 `SendMessage` — 다른 스레드 emit · `Moved` 가 루프 끝까지 묶임(스파이크 S3 fresh) ④ 안내를 대상 창 웹뷰 안에 그리기 — 끄는 창이 덮는다(스파이크 S7 · ADR-0295 「영향」) ⑤ Tauri `set_position` 으로 안내 창 옮기기 — z 순서가 안 올라간다(S7 0/1) ⑥ 끄는 중 출처 숨기기에 `hide()` · `SW_HIDE` — 캡처가 조용히 끊긴다(S2) ⑦ 예비 창을 label 접두로 가르기 — 들인 뒤 label 을 바꿀 수 없어 권한 · 닫기 · 소멸 정리가 어긋나고 남긴 창(R10)을 가를 수 없다(리뷰 1 라운드 B7 의 두 갈래 중 TRD 6.1 §2-5 가 고른 근거) ⑧ 예비 표시를 URL 에 싣기 — 들인 뒤에도 남아 그 창을 대기 모드에 가둔다(B7).
- **ADR-c TS ↔ Rust 기하 거울을 두지 않는다 — 탭바 안 = TS · 창 사이 = Rust**(§0 · §3 · G9)
  - 결정: 탭바 안 순서 이동 기하(Chromium `CalculateInsertionIndex` 그대로)는 TS 한 곳, 탭바 구역 · 도킹 띠 · 크기는 Rust 한 곳. 웹뷰는 구역 사각형을 받아 점 포함만 본다. 붙을 때 셸은 첫 자리(0)에 넣고 대상 웹뷰가 같은 자리 함수로 곧바로 고친다(Chrome `AttachToNewContext`).
  - 거부한 대안: 골든으로 묶은 거울(`pick_golden.json` · `dockPick.ts` — 5.8) — 거울마다 지킴이가 달랐고 순서 이동 기하가 네 곳에 흩어졌다(점검 결함 5 · 7).
- **ADR-d 끌기 하나가 모델을 여러 번 쓴다 — 표식 이어 쥐기 · `AdoptWindow`(+ 재기) · 비운 창 남기기**(§7)
  - 결정: 끌기 중의 쓰기는 같은 표식을 이어 쥐고, 예비 창 · 비운 출처 창을 모델에 들이는 `AdoptWindow`(받아 둔 재기를 같은 락에서 옮겨 담음)와 비운 창을 닫지 않고 재기와 함께 돌려주는 `OnEmptied::Keep` 을 둔다. 세션 포트는 창을 여는 갈래(`NewWindow`)를 타입에서 뺀다. 모든 결과가 `dock_tab` 하나를 지난다(T-65 와 같은 길).
  - 거부한 대안: 놓을 때 한 번만 쓰기(5.8 R11a) — 사용자 결정 Q30(Chrome 꼴)이 대신했다.
- **ADR-e platform 목적 함수 — 창 이동 루프 · 커서 밑 창 · 창 놓기 · Alt+Tab 빼기 · 입력 읽기**(§8 · ADR-0266 절차)
  - 결정: `move_loop`(시작 · 끝 · 들어감 · Esc 훅 · 끝 알림 정확히 한 번 — 들어가기 전 끝 · 들어가기 전 Esc · 소멸(재귀 깊이 가드) 포함 · 끝 알림마다 훅이 숨겼나 `hid`) · `window`(`place_window`(활성 거절 시 Alt 합성 우회 없음) · `move_window` · `hide_window` · `is_window_visible` · `top_level_at` · `restore_placement`(작업 공간 좌표 맞춤) · `set_switcher_hidden`(`TOOLWINDOW` · `APPWINDOW`) · `NativeWindow`) · `input`(`cursor_pos` · `primary_button_down`) 을 platform 에 둔다. 창은 `&impl HasWindowHandle` 로 받고 변이 맞추기도 platform 안이다 — `raw-window-handle` 은 platform 의 의존이고 셸은 의존하지 않는다.
  - 거부한 대안: 셸의 `cfg(windows)` + `hwnd()` — 게이트 ④ 위반 · 셸이 `RawWindowHandle::Win32` 를 맞추기 — `cfg` 없는 platform 밖 런타임 OS 가름(리뷰 1 라운드 B10) · tao `start_dragging` — 루프 끝 · 훅 · 끝 까닭이 없다(조사 B5).

## 부록 A. PRD 추적

**요구사항**

| R | 이 TRD | 실측 |
|---|---|---|
| R1 제목줄 = 창 이동만 | 탭 머리에만 처리기(§9) — 제목줄은 OS 몫 그대로 | M12 |
| R2 누름 활성 · 10 px · 탭바 구역 · 떼면 진짜 창 · 그림 없음 · 탭 하나 = 창째 | §2-2 · §2-3 · §5-1 · §5-2 · §5-5(C4) · §6-4 · §8 | M1 · M6 · M11 · M17 |
| R23 자기 탭바 안 순서 이동 | §5-1(C1 · C2 · C3) · §2-3 Strip · §6-2(Esc) | M11 |
| ★R23 세부 정정(C3)★ | PRD 문장 「그 창 탭들이 차지한 범위 밖으로는 나가지 않고」 → Chrome 은 끄는 탭을 **탭 끌기 영역**(탭 컨테이너가 쓸 수 있는 폭 — 마지막 탭 뒤 빈 곳 포함 · `+` 앞까지일 가능성 높음)까지 그리고, 자리는 보이는 마지막 탭 너머로 가지 않는다(§5-1 · tab_strip.cc#530-552 · #589-600). R23 의 「데스크톱 Chrome 그대로」(Q23)를 따른 것 — PRD 문장 고침은 메인 몫 | M11 |
| R24 탭바 넘침 = Chrome | `tabStrip/layout.ts`(5.8 §4-5 그대로 옮김) · §5-1 넘친 탭바(보이는 범위 자르기 — C1) | M11 |
| R25 빈 탭바 = 창 끌기 영역(막지 않음) | §9 | — (T-66 · AC34) |
| R26 닫기 모드 | §5-6 | M11 |
| R27 활성 탭 모양 | §5-7(C7 구분선 투명도) | M11 |
| R3 칸 띠 10% · 최소 두께 | §5-3 · G5 | M7 |
| R3a 안내 창 | §2-5 · §8 `place_window` · G6 | M7 · M10 |
| R4 미리보기 = 결과 사각형 · 곧바로 다시 판정 | §5-3 · §5-4 · §6-5 | M7 |
| R5 바깥 띠(고정 px) · 바깥 위 띠 | §5-3(`OUTER_BAND_PX` 32 · 유효 영역) | M7 |
| R6 칸 가운데 = 표적 아님 | §5-3 고르기 ④ · §6-1 | M7 |
| R6a 앞서는 차례 · 유효 띠 · 칠한 곳 = 붙는 곳 | §5-3 · `targets.rs` | M7 |
| R6b | 걷힘 — 짓는 것 없음 | — |
| R7 붙는 크기 = 원래 크기 · 칸 절반 상한 | §5-4 · §7 ① | M7 |
| R21 탭바에 붙기 · 다시 떼기 | §2-2 [붙기] · §2-3(cont = Attach) · §5-1 붙은 직후(C1 · G9) · §7 ④(C8) · §6-3(C10 활성) | M3 |
| R22 그 밖에 놓기 · 새 창 크기 · 자리 | §5-5(C5 · C6) · §6-1 | M1 · M7 · M10 |
| R8 면적 0 만 막음 | §5-3 막힌 띠 | M7 |
| R9 탭 배치 통째 | §7(`docked_tree` — 5.8 그대로) | M7 |
| R10 비운 창 = 숨겼다 끝에 닫음 | §2-5(대기 창 명부 · 남긴 창 · 닫기가 실패하면 고아 — F1) · §7 ⑤ · §6-1 공통 마무리 5 · G12 | M6 · M18 |
| R11 출력 연속 | §6-6 | M15 |
| R11a 바뀜마다 한 번에 · 저장 | §6-6 · §2-4 · G17 | M9 |
| R11b 놓은 뒤 포커스 | §6-1 | M4 · M7 |
| R12 Esc = 되돌리기 · 그 밖 키 = 놓기 | §6-2(C8) · §6-3(C9) | M4 · M5 |
| ★R12 세분(C9)★ | Chrome 코드를 단계마다 그대로 옮긴다 — **탭바 안** = `EventTracker`(Esc = 되돌리기 · 그 밖의 눌린 키 = 놓기 · 키 반복 · 미리 쥔 키를 거르지 않음 — PRD 문장 그대로) · **따라오는 창(OS 이동 루프)** = OS 가 키보드를 쥐고 Chrome 훅은 Esc 만 보며 결과는 뗌을 봤느냐로만 갈린다 — 그래서 루프가 쓴 Esc 밖 키는 놓기가 아니다(화살표 = 창 이동 · Enter = 뗌 없는 끝 → 되돌리기 — 추론 · M5). 루프 중에 키가 우리 웹뷰 감시에 닿으면 탭바 안 규칙을 쓴다(Chrome 의 `EventTracker` 가 끌기 내내 살아 있으므로 · 닿는지는 미확인). PRD R12 문장 고침은 메인 몫 | M5 |
| R13b 비정상 끝 | §6-2(캡처 잃음 · 루프 끝 까닭 · 들어가기 전 Esc) · §6-1 공통 마무리 · §6-5(루프 안 창 소멸) · §8(끝 알림 정확히 한 번) · §2-4(`Abandon` · 할 일 실패 · 소유 넘김) · §2-5 「고아」(F1) · §2-3-3 행렬(실패 · `Abandon` · 소멸 칸) | M14 · M17 · M18 |
| R13c 끄는 중 배치 바뀜 | §6-5 | M9 |
| R14 메인 마지막 탭 | §6-4 · G14 · S-c | M6 |
| R15 맨 위 창 · 750 ms · 같은 창 다른 탭 | §2-3(`top_level_at` · `RaiseDue`) · §7 ④(E2 · C8) · §8 | M3 · M8 |
| R16 · R17 LLM 명령 = T-65 · 같은 적용 경로 | §7 「LLM 길」 · §15-4 | — |
| R18 추적 = Chrome 방식 · 레이아웃 권위 = 셸 | §2-1 · §2-3 · §1 스택 맞춤 · ADR-a · ADR-b | M2 · M3 |
| R19 OS 코드 = platform 목적 함수 | §8 · ADR-e | — |
| R28 스파이크 먼저 · 안 되면 멈춤 | §1(스택 맞춤 셋 — 보고 마침) · §12 · §15-2 | 전부 |

**수용 기준**(AC10 · AC17 없음 · AC28 걷힘 · AC34 = T-66)

| AC | 이 TRD | 실측 |
|---|---|---|
| AC1 | §9 | M12 |
| AC2 | §2-2 떼기 · §5-3 · §5-4 · §2-5 안내 창 | M1 · M7 |
| AC3 | §6-1 띠 · §5-4 | M7 |
| AC4 | §5-3 바깥 띠 · §5-4 | M7 |
| AC5 | §7(배치 통째) | M7 |
| AC6 | §2-2 떼기 | M1 |
| AC7 | §6-6 | M15 |
| AC8 | §6-1 그 밖 | M7 |
| AC9 | §6-4 메인 | M6 |
| AC11 | §6-6 저장 | M9 |
| AC12 | §5-2 `client_point` · §5-5 배율 · §2-5 안내 창 배율 | M10 |
| AC13 | §2-3(팝아웃 · 메인 구별 없음) | M7 |
| AC14 | §7 ④(E2) | M4 |
| AC15 | §5-3 막힌 띠 | M7 |
| AC16 | §6-5 | M9 |
| AC18 | §2-2 [붙기] · §5-1 | M3 |
| AC19 | §6-2 · §6-3 | M4 · M5 |
| AC20 | 끌기 그림 없음 — OS 끌어 놓기를 쓰지 않는다(§2-2) | M1 |
| AC21 | §6-2 · §8 끝 까닭 | M14 |
| AC22 | `fileDropGuard.ts` · G8 | M12 |
| AC23 | §5-1 | M11 |
| AC24 | §2-2 떼기 · §5-2 | M1 · M3 |
| AC25 | §6-2 ① | M4 |
| AC26 | §6-4 탭 하나 창 | M6 |
| AC27 | §5-2(구역 안 = 떼지 않음) · §6-1 Strip | M11 |
| AC29 | §5-1 넘친 탭바 · `layout.ts` | M11 |
| AC30 | §2-2 누름 | M11 |
| AC31 | §5-6 | M11 |
| AC32 | §5-7 | M11 |
| AC33 | §5-4 원래 크기 | M7 · M16 |
| AC35 | §6-5 · §6-1(놓을 때 다시 판정) | M9 |
| AC36 | §5-3 유효 영역 | M7 |
| AC37 | §2-4 한 번에 하나씩 · §6-6 | M9 |
| AC38 | §6-4 메인 | M6 |
| AC39 | §5-3 최소 두께 · §5-4 | M7 |
| AC40 | §5-6 | M11 |
| AC41 | §7 ④(E2 · `succession`) | M4 |
| AC42 | §6-6 | M15 |
| AC43 | §2-3 `RaiseDue` · §8 | M8 |
| AC44 | §5-3 바깥 위 띠 · §5-4 | M7 |

## 부록 B. 걷는 것

| 어디 | 걷는 것 | 단위 |
|---|---|---|
| `src-tauri/src/dock/` | `book.rs`(장부 · 시한 여섯 · 끝 로그 ~30 칸 · 시험 ~3,900 줄) · `regions.rs`(`rev` · `bar_rev` · `bumps` · `Expect` · `reorder_miss`) · `hit.rs`(남는 기하는 U2 의 `geometry` · `strip` 로 옮긴 뒤) · `types.rs`(§4 「걷는 것」 타입) · `snap.rs`(→ `world.rs`) · `pick_golden.json` · `mod.rs` 의 `book` · `own_hidden` · 세대 · 오류 셈(새 `DockState` 로) | U8 |
| `src-tauri/src/commands/dock.rs` | 명령 여섯(`dock_drag_start` · `_over` · `dock_drop` · `dock_drag_end` · `dock_pointer_seen` · `dock_pull`) · `DockExec`(`reconcile` · `focus` · `hand_off` · `arm` …) · 옛 `DockPorts` · `DockSpawn` · `WindowProbe` · `EVT_REGIONS` · 옛 `dock:session` 내기 | U8(새 것은 U6 에서 옆에 선다) |
| `src-tauri/src/layout/` | `chrome.rs` 의 `TabHeader` · `DockChrome.tabs` · `.scroll`(모양 바꿈 — 웹뷰가 새 모양을 함께 보냄) · `dock.rs` 의 `DOCK_NEW_SHARE` · `manager.rs` 의 `Docked.source_window_closed`(→ `emptied`) | U8 · U1 |
| `src-tauri/src/lib.rs` | 옛 명령 등록 여섯 | U8 |
| `src-tauri/bindings/` | 걷는 wire 타입의 `.ts` · `DockChrome.ts` 다시 구움 | U8 |
| `src/components/layout/dock/` | `dockDrag.ts`(+ `dockDrag.test.ts`) · `dockPick.ts`(+ 시험) · `dragImage.ts`(+ 시험) · `dockChromeReport.ts`(+ 시험 → `frameReport`) · `DockOverlay.tsx` 의 `useDockGuide` 래퍼 | U7b-2 |
| `src/components/layout/dock/frameReport.ts` | 옛 모양 채움(`tabs: []` · `scroll: null`) | U8 |
| `src/components/layout/TabBar.tsx` | `draggable` · `onDragStart` · `onDragEnd` · `ENGINE_CANCEL_MS` · `setTimeout(0)` 세션 열기 · `measureStrip` · `translateXOf` · `reorderStyle` · `detached` · `dragCanvas` · `onScroll` 소품 · 클릭 전환(누름 전환으로) | U7b-2 |
| `src/components/layout/WindowLayout.tsx` | `mountDockDrag` · `ensureDragImageBox` · `useDockSession` · `useDockMode` · `useDockReorder` · `displayedTabOf` · `measureDockChrome` · `reorderStandingRef` · `seenChrome` · 창 안 `DockOverlay` | U7b-2 |
| `src/components/layout/tabStripLayout.ts` | `tabStrip/layout.ts` 로 옮김(내용 그대로) | U7a |
| 9c 미커밋 코드 | `tabStripLayout.ts` · 시험 밖 전부 · `dockDrag.shellConstants.test.ts`(백업 패치) | U0 |
