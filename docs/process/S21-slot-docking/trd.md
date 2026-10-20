# TRD — 슬롯 도킹 (탭 끌기 = OS(HTML5) 끌기 · 놓을 때 한 번 옮김 · 도킹 = 우리 확장)

> **7판 — 새로 씀**(2026-10-20 · ADR-0296 · PRD 4.9 · VS Code 벤치마크 = 조사 F). 6.3판(데스크톱 Chrome 식 — 셸 끌기 세션 · 창 이동 루프 · 예비 창 · 안내 창)은 ADR-0296 이 대신했다. 그 판은 `.claude/handoff/attachments/trd-pre-os-drag-2026-10-20.md` 에 백업했다. 6.3 의 §번호 · T번호 · G번호 · F번호를 가리키는 옛 기록은 그 백업을 본다.
>
> 이 판이 6.3 에서 이어받는 것은 다섯이다 — 레이아웃 적용 경로(`layout::apply::dock_tab`) · 도킹 띠 규칙(6.3 §5-3) · 붙는 크기 · 새 창 크기 규칙(6.3 §5-4 · §5-5 의 `dragged_size`) · 탭바 안 순서 이동 기하(6.3 §5-1) · 탭바 폭 · 닫기 모드(6.3 §5-6). 나머지는 새로 썼다.
>
> **7판 개정 2026-10-20 — ADR-0297**(사용자 결정 · U3 코드 리뷰 뒤) — ① 끄는 동안 아무것도 막지 않는다: 불변식 = 「끌기가 끄는 동안 자기 판정 · 상태를 셸과 맞추지 않는다 — 끌기 명령은 놓을 때 보낸다」 · 터미널 크기 미루기(`dragActive` · `TerminalSlot`)를 걷음(§0 · §1 · §2-1 · §2-2 · §4-1 · §4-2 · §4-7 · §7 U3 · §8-2 · §8-4 C7 · §8-5 M11) ② 놓을 때 셸이 대상(끄는 탭 · 대상 탭 · 대상 칸)을 id 로 다시 찾아 있으면 지금 상태에 맞춰 적용 · 없으면 무동작 — 엄격 거절(탭 목록 대조 · `SeamAt` 판 번호 · 활성 · `expect_window`)을 걷음 · 명령에서 `expect` · `rev` · `active` 를 뺌 · 판 번호 `layout_rev` 자체는 소비자 없이 남아 U6 이 걷음 · 대상이 그사이 다른 창으로 갔으면 그 창에서 처리(`successor_window` · 포커스 = 옮긴 탭이 든 창 · 메인 결정 — §3-3)(§2-4 · §3-1 · §3-2 · §3-3 · §3-4 · §3-6 · §3-7 · §4-3 · §4-6 · §5-2 · §5-4 · §7 · §8-1 · §9-3) ③ 같은 날 정한 것 — 같은 창 같은 자리 놓기 · 엔진이 접은 끌기는 출처 dragend 도 보내지 않음 · 놓기는 마지막 판정(미리보기) 기준 · 끄던 탭을 지우는 `window:tabs-updated` 가 dragend 전에 바꿔 그리기를 푸는 그물은 그대로(§2-2 · §2-4 · §4-2 · §4-3 · §4-6 · §8-2 · §9-3).
>
> **사용자 결정 반영(2026-10-20)**
> - Q34 — 탭 하나뿐인 팝아웃은 다른 탭처럼 끈다. 창 밖에 놓으면 그 창을 옮긴다(크기 그대로). **최대화돼 있었으면 복원한 뒤 옮긴다**(「1. 맞고」 — PRD Q37).
> - Q35 — 메인의 마지막 탭은 끌기를 시작하지 않는다.
> - Q36 — 끄는 중 750 ms 올리기 · VS Code 의 1500 ms 머물러 열기를 둘 다 넣지 않는다 — 「사이에 끼는 기능만」.
> - W13 — 자기 탭바에서는 탭이 커서를 따라 움직이고 이웃이 비켜난다. 다른 창 탭바에서는 VS Code 의 2 px 세로선을 보인다.
> - 미커밋 처리 = §6 (B) 갈라 커밋(세부는 메인).
> - Q38 — 우리 창의 제목 표시줄 · 테두리 위에서 떼면 아무 일 없다(취소와 같음 — 「별도의 처리를 안하는 방향으로.」 · 7판 초안의 메인 결정을 사용자 결정으로 옮김 · §3-5 ③).
> - Q39 — 다른 앱이 놓기를 받아도 새 창(「새창.」) — 우리 웹뷰 밖에서 뗀 끌기는 「우리 창 위면 아무 일 없음 · 그 밖은 전부 새 창」 하나다. 받아들인 한계 = 받은 페이지가 끌기 자료(탭 · 창 식별자와 크기)를 읽음(§9-2).
> - 탭 모양 = Chrome 처럼 둥글게(「크롬처럼 둥글게」 · PRD R27) — ADR-0168 결정 5 를 ADR-0295 가 부분 폐기(위 모서리 6px · 구분선 규칙 · `--tab-inactive-bg` · 나머지는 유효)로 박았다. U0 를 막던 옛 「사용자 결정 대기 ②」가 닫혀 §9-3 「정해짐」으로 옮겼다.
>
> **리뷰 반영(2026-10-20 · 재리뷰 3차 light FIX)** — ① 터미널 `resizePty` 보내기를 전부 칸마다 `sendSize` 하나로 · 가시성(IntersectionObserver) 길도 미룸 · 꺼질 때 다시 재어 한 번 · 그물에 첫 pointermove(§2-2 · §4-7 · §8-2 — **→ ADR-0297 로 걷음**: 미루지 않는다) ② `DragEnd{effect}` — `move` 면 취소 안 봄 · 우리 창 · 못 읽음 = 무시 · `Foreign` = 새 창(§3-1 · §3-5 ⑤ · §9-1 · §9-2) ③ 진행 중 새 창에 버려진 `Drop` + 그 거절 = 받아들인 틈(§9-2) ④ 같은 웹뷰 제자리 놓기는 출처 dragend 에서 곧바로 풂(§2-4 ④) ⑤ `move` + `done{Canceled}` = `Ignored` 와 같음(§2-4 · §4-6) ⑥ 대소문자 겹침 시험 = git 색인 + 내용 서명(§8-1 ⑪) ⑦ U2 는 옛 wire 타입을 HEAD 그대로 두고 구역 선 함수만 실음(§7).
>
> **리뷰 반영(2026-10-20 · 재리뷰 light FIX)** — 생성 상수 파일 하나 `DockConstants.ts`(대소문자만 다른 생성물 금지 — §4-5 · F1) · 끄는 동안 터미널 `resizePty` 미루기(§2-2 · §4-7 — F2 · **→ ADR-0297 로 걷음**) · 출처는 dragend 마다 `DragEnd`(죽은 `copy` · `link` 갈래 걷음 · Q39 — §2-3 · §4-2 · F3) · 남은 탭 바꿔 그림을 푸는 그물 계기(다음 `window:tabs-updated` · pointerdown — §2-4 · B5) · 끌기 그림 창 = WebView2 브라우저 프로세스 → 클래스로 거름(§3-5 · §9-1 · B4) · `layout_rev` 픽스처 넷(§7 U1 · F4 · **→ ADR-0297 로 소비자가 없다** — 판 번호는 U6 이 걷음) · `SeamAt.or_initial`(같은 창 이음새 · §3-3 · §3-7 · F5 · **→ ADR-0297 로 걷음**) · 장부가 거절된 새 창 뒤의 `Drop` 을 받음(`settle` — §3-2 · §3-4 · F6) · 구역 선 Rust 몫을 U2 가 다시 실음 · 바인딩 20 개(§5-4 · §7 · F7) · 「CDP 따위」 예 걷음(§3-4) · `layout_rev` 비공개 setter 권고(§3-7 · **→ ADR-0297 로 소비자가 없다** — U6 이 걷음). 이를 맞추며 더한 것 = 자기 dragend 가 `move` 면 `done{Ignored}` 로 풀지도 되돌리지도 않음(§4-6).
>
> **리뷰 반영(2026-10-20 · `/review trd` 2 건 FIX)** — 창 밖 판정 = dragend 뒤 셸의 z 순서 판정(웹 ↔ 웹 채널 · 변 48 px · 「출처 사각형 안이면 무시」 걷음) · Drop 이 앞선 DragEnd 의 취소 · 무시를 이김 · 취소 · 무시 뒤 원래 선택 되돌리기 · 1 s 시한 대신 결과 알림 · Seam 사전조건 = 뷰별 판 번호(**→ ADR-0297 로 걷음** — 놓을 때 다시 찾기 · §3-3) · 막힌 띠 숨김(R8) · 기하 정본 = Rust(골든을 양쪽이 먹음) · U0 (B) 의존 목록 · 단위마다 초록인 걷기 순서 · `window:frame` 은 바뀔 때만 · 로그 · 스파이크 사실의 범위.

| 항목 | 값 |
|---|---|
| **상태** | 7판 — 재리뷰 4차(light) PASS · 비차단 5 건 반영 · **구현 중** — U1(`52de2625`) · U2(`411fc96d`) 커밋 · U3 진행 중(미커밋) · **7판 개정 2026-10-20 — ADR-0297**(위 머리 글) |
| **날짜** | 2026-10-20 |
| **스텝** | S21 |
| **브랜치** | `v0.5.0/feat/slot-docking`(2026-10-20 개명 — 옛 `v0.4.1/feat/slot-docking` · 사용자 결정) |
| **PRD** | [prd.md](prd.md) 4.9판 — 보이는 규칙의 정본. 이 TRD 가 4.9 와 다르게 정한 곳 · 4.9 가 고쳐야 할 곳 = §9-5 |
| **결정** | [ADR-0296](../../decisions/0296-슬롯-도킹-탭-끌기-1차-oshtml5-끌기-놓을-때-한-번만-이동-끌기-중-셸-동기화-없음.md)(끌기 수단 · 불변식) · [ADR-0297](../../decisions/0297-슬롯-도킹-탭-끌기-끄는-동안-아무것도-막지-않음-놓을-때-대상을-다시-찾아-있으면-붙이고-없으면-무동작.md)(불변식 범위를 좁힘 · 놓을 때 다시 찾기) · [ADR-0295](../../decisions/0295-슬롯-도킹-탭-끌기-데스크톱-chrome-소스-그대로-도킹-띠-10-붙는-크기-원래-크기칸-절반-상한.md) 결정 3 · 4(10% 띠 · 붙는 크기). 결정 1 · 2 의 끌기 수단은 ADR-0296 이 대신했다 — 결정 1 의 활성 탭 모양은 살아 있고 ADR-0168 결정 5 를 부분 폐기한다(§9-3) |
| **근거** | 스파이크 `.claude/handoff/attachments/spike-os-drag-2026-10-20.md` + `.patch`(「스파이크 Sn」) · 조사 [slot-docking-drag-ownership-peers-2026-10-19.md](../../research/slot-docking-drag-ownership-peers-2026-10-19.md)(F = VS Code `6c8316e` 소스) · 코드 점검 [slot-docking-drag-code-audit-2026-10-18.md](../../refactoring/slot-docking-drag-code-audit-2026-10-18.md) |
| **코드 기준** | HEAD `1c5ad58b` + 미커밋(§6). 줄 수 = 작업 트리 `wc -l`(2026-10-20 · 미커밋 포함). 심볼 이름이 정본이다 |

---

## 0. 결론 요약

- **끌기 = WebView2 의 HTML5 끌어 놓기**(VS Code 꼴 · 그림 = 탭 이름 라벨). **끌기는 끄는 동안 자기 판정 · 상태를 셸과 맞추지 않는다 — 끌기 명령은 놓을 때 보낸다**(ADR-0297 결정 1). 끌기와 무관한 셸 호출(터미널 크기 맞춤 · 창 틀 조회 · 캔버스 크기 보고 · LLM 의 탭 · 창 조작)은 끄는 동안에도 평소대로 흐른다 — 막지도 미루지도 않는다(§2-2 · §4-7). 끌기 하나에 셸 명령은 끝난 뒤 `tab_drop` **많아야 둘**(출처 dragend 의 하나 + 받은 창의 하나) + 원래 선택 되돌리기 `switch_tab_if` 0–1 회(§2-2 · §4-6)이고 놓기 쓰기(`dock_tab`)는 **많아야 한 번**이다.
- **우리 창 웹뷰는 우리 탭 끌기를 어디서든 받는다**(뿌리에서 `preventDefault`) — 우리 웹뷰 위에서 떼면 drop 이 늘 그 창에 온다. 그 창이 순수 판정 `judge` 로 고른다: 탭바 구역 → `TabBar` · 유효한 띠 → `Seam` · 그 밖(칸 가운데 등) → `NewWindow{by: Drop}`. 셸은 그것을 LLM 명령과 같은 `apply::dock_tab` 으로 한 번 적용한다 — 놓기가 가리킨 대상을 id 로 다시 찾아 있으면 지금 상태에 맞춰 적용하고 없으면 무동작이다(ADR-0297 결정 2 · §3-3).
- **출처는 dragend 마다(`none` · `move` 모두) `NewWindow{by: DragEnd{effect}}` 를 보낸다**(`effect` = 자기 `dropEffect` · 같은 창 같은 자리 놓기 · 엔진이 접은 끌기는 보내지 않는다 — §2-4 ④). 셸은 **끌기가 끝난 뒤** 맨 먼저 커서 · 주 버튼 · Esc · **커서 밑 맨 위 창(z 순서)** 을 읽고 가른다: `none` 이고 버튼 · Esc 눌림 = 취소 · 맨 위 창이 우리 창 = 무시(`move` 면 맨 위 창을 못 읽어도 무시 — 받은 창의 `Drop` 을 기다림)(우리 웹뷰가 받았으면 그 창의 `by: Drop` 이 이긴다 · 제목 표시줄 · 테두리면 아무 일 없음 — 사용자 결정 Q38) · 그 밖 = 커서 자리에 새 창(다른 앱이 받았어도 — 사용자 결정 Q39 · 탭 하나뿐인 팝아웃이면 그 창을 옮김 — 최대화였으면 복원한 뒤). 곧 우리 웹뷰 밖에서 뗀 끌기는 「우리 창 위면 아무 일 없음 · 그 밖은 전부 새 창」이다(예외 — §9-2 · §9-5: 끄는 중 LLM 이 끄는 탭을 다른 창으로 옮기거나 닫음). 셸은 처리마다 결과를 출처 창에만 알리고(`tab_drop:done` — 중복과, 새 창이 거절되기 전에 같은 끌기의 `Drop` 을 맡아 둔 `DragEnd` 는 알리지 않는다 · §3-2 ④), 취소 · 무시면 출처가 원래 선택을 되돌린다(자기 dragend 가 `move` 면 되돌리지 않음 — §4-6).
- **보이는 모습** — 자기 탭바 = Chrome 식 순서 이동(transform 만) · 다른 창 탭바 = 2 px 세로선 · 칸 = 10% 띠 · 바깥 20 px 띠(결과에 면적 0 칸이 생기는 띠는 칠하지도 고르지도 않음 — R8) · 붙는 크기 미리보기 · 출처 창은 탭바 구역을 벗어나면 남은 탭을 보인다(R15 · 웹 표시만).
- **판정 기하의 정본 = 셸(Rust)** — 옛 `dock/{geometry,sizing,strip}` 의 순수 함수를 `layout/` 로 옮겨 남기고(T-65 LLM 명령이 쓴다), 웹은 dragover 마다 쓸 거울(`bands.ts` · `dockedTree.ts` · `dragSize.ts`)을 둔다. 둘은 골든 JSON 하나를 양쪽 시험이 먹어 묶는다(선례 `queued_input_golden.json`). **상수(띠 10% · 탭바 구역 15 px · 붙는 몫 상한 0.5 등)는 Rust 한 곳에서 정하고 TS 는 바인딩 생성 길로 만든 `src-tauri/bindings/DockConstants.ts` 하나를 읽는다**(§4-5). 원래 크기 · 새 창 크기는 출처 웹뷰가 dragstart 에 셈해 싣는다 — 재료 = 셸이 바뀔 때만 미는 창 사실(`window:frame`) + `screen.avail*`.
- **걷는 것 ≈ 1.5 만 줄**(§5-5). **이행** = U0((B) 갈라 커밋) → U1 셸 명령 → U2 기하 옮김 · 웹 순수 모듈 · 골든 → U3 전환 → 사용자 실측 → U4a–c 옛 엔진 걷기 → U5 platform → U6 모델 군살 → U7 문서 — 어느 단위 뒤에 멈춰도 초록이다.

## 1. 전제 · 관계 · 받아들인 대가

- **ADR-0296** — OS 끌기 · 모델은 놓을 때 한 번(`layout::apply`) · 미리보기는 대상 웹뷰 · 창 밖은 셸이 커서를 읽음 · **끌기 도중 셸 호출 0 이 불변식이다** — **ADR-0297 이 범위를 좁혔다**: 끌기가 끄는 동안 자기 판정 · 상태를 셸과 맞추지 않는다 — 끌기 명령은 놓을 때 보낸다. 끌기와 무관한 셸 호출은 막지 않는다. dragend 뒤의 `tab_drop` · 그 안의 입력 읽기 · 원래 선택 되돌리기는 끌기가 끝난 뒤라 이 불변식을 어기지 않는다.
- **ADR-0297**(ADR-0296 개정) — 끄는 동안 아무것도 막지 않음(§2-2 · §4-7) · 놓을 때 셸이 대상을 id 로 다시 찾아 있으면 지금 상태에 맞춰 적용 · 없으면 무동작(§2-4 · §3-3).
- **그대로인 것** — ADR-0295 결정 3 · 4(칸 10% 띠 · 가운데는 도킹 아님 · 붙는 크기 = 원래 크기 · 칸 절반 상한) · 사용자 결정 2026-10-19(바깥 띠 20 px · 안내 = 커서 칸의 얇은 선 · 바깥 틀은 닿을 때만 · 옅은 미리보기 · 탭바 판정 영역 = `+` 앞까지).
- **스파이크에서 기대는 사실**(2026-10-20 · Win11 · WebView2):

| 사실 | 쓰는 곳 |
|---|---|
| dragover = 움직일 때 108–122/s · 멈췄을 때 13–16/s(≈ 63 ms) · 처리기 ≤ 0.5 ms · 프레임 끊김 0 | dragover 마다 판정해도 된다 · 미리보기 감시 시한 300 ms(§2-4) |
| 끌기 자료 = 우리 사용자 MIME 하나 · **dragover 동안 `getData` 막힘**(`types` 만 보임) | 원래 크기 · drag id · 끄는 탭 모양을 타입 문자열에 싣는다(§4-2) |
| dragend 직후 셸 `GetCursorPos` = 놓은 점(물리 px) 10/10 · dragend `screenX/Y` 는 DIP — **우리 창 안에서 뗀 경우만 쟀다 · 바탕화면 · 다른 앱 위 = 미검** | 새 창 자리 · 맨 위 창 판정은 셸이 커서를 읽어 한다(§3-5) · M2 |
| Esc 와 창 밖 놓기 = 둘 다 `dragend` + `none` · Esc 끝 = 버튼 · Esc 눌림 4/4 · 놓기 = 둘 다 뗌 5/5 · 짧은 Esc 를 OS 가 놓침 1/5 — **키 · 버튼 상태도 우리 창 안에서만 쟀다 · 바탕화면 = 미검** | 취소 가름(§3-5) · 위험(§9-2) · M1 |
| 끌기 문턱 = Chromium(물리 4 px · 125% 에서 3.2 CSS px) · 늦출 수 없다 | PRD 4.9 R2 가 받아들였다 |
| 누르는 순간 활성이면 같은 창 다른 탭 배치가 안 보인다(S12) | 출처 창이 남은 탭을 바꿔 그린다(§4-2 · R15) |
| CDP 가로채기로 창 사이에 놓으면 출처가 `dragCancel` 전까지 「끄는 중」으로 남는다 · 출처 dragend 가 안 난다(`dragCancel` 뒤에는 `none` 으로 날 때도 있다 · 같은 페이지 놓기는 `move` — §8-4) | CDP 시험 절차(§8-4) |

- **받아들인 대가**(ADR-0296 「영향」 · PRD R29) — 점선 상자 커서 · 그림이 다른 앱 위에서 사라짐 · Esc 와 창 밖이 같은 신호 · 되튐 애니메이션 · Snap 없음 · Chromium 문턱(물리 4 px) · 끄는 동안 창을 올리지 않음(다른 우리 창에 가려진 자리에는 못 놓음 — Q36) · 빠른 Esc 를 OS 가 놓칠 수 있음 · 우리 창 제목 표시줄 · 테두리 위에서 떼면 아무 일 없음(사용자 결정 Q38 · §3-5 ③) · 다른 앱이 놓기를 받으면 그 앱이 끌기 자료(탭 · 창 식별자와 크기)를 읽을 수 있음(사용자 결정 Q39 의 받아들인 한계 · §9-2).

## 2. 누가 무엇을 갖나 · 흐름

### 2-1 소유

| 주체 | 갖는 것 | 갖지 않는 것 |
|---|---|---|
| **출처 웹뷰**(끄는 탭의 창) | 끌기 시작(실음 · 원래 크기 · 새 창 크기 · 끄는 탭 모양 · 그림) · 자기 탭바 순서 이동 모습 · 구역을 벗어나면 남은 탭 보이기 · dragend 의 명령(제자리 놓기 · 엔진이 접은 끌기는 없음 — §2-4 ④) · `done`(과 그물 계기)으로 바꿔 그리기 풀기 · 취소 · 무시 뒤 원래 선택 되돌리기(조건부 `switch_tab_if` — §4-6) | 모델 쓰기(되돌리기도 셸 명령) |
| **대상 웹뷰**(커서 밑 창 — 출처 자신 포함) | 뿌리 dragover 에서 받기 · `judge`(탭바 구역 · 유효한 띠 · 그 밖) · 미리보기 · 세로선 · drop 의 명령 | 모델 쓰기 · 다른 창과의 통신 |
| **셸 `tab_drop`** | 명령 하나 검증 · `DragEnd` 면 입력 상태 · 맨 위 창 읽기와 가름 · 장부(중복 · Drop 이 이김) · 새 창 자리 셈 · `dock_tab` 한 번 · 옮긴 탭이 든 창 포커스(§3-2 ⑤) · 결과 알림 · 로그 한 줄 | 끄는 동안의 상태 |
| **셸 기하**(`layout/bands.rs` · `layout/drag_size.rs`) | 띠 · 고르기 · 붙는 몫 · 미리보기 · 유효성 · 새 창 크기의 정본 — LLM 명령(T-65)이 쓰고 골든이 웹 거울과 묶는다 | 끄는 동안 불리지 않는다 |
| **레이아웃**(`apply::dock_tab` · `ViewManager::dock`) | 모델 쓰기 · 판정(대상 없음 — 놓을 때 id 로 다시 찾기(ADR-0297 · §3-3) · 면적 0 · 몫 범위) · 알림 · 출력 구독 재동기 | 끌기를 모른다 |
| **platform `input` · `window::top_level_at`** | 커서 · 주 버튼 · Esc 의 지금 상태 · 점 밑 맨 위 최상위 창(z 순서 · 비쳐 보이는 창 거름) | — |
| **OS(OLE) · WebView2** | 커서 따라가기 · 그림 · 맨 위 창에만 사건 배달(R15) · 문턱 · Esc | — |

### 2-2 경계를 넘는 것 — 불변식

| 때 | 방향 | 무엇 |
|---|---|---|
| 평소(끌기 밖) | 셸 → 웹 | `window:frame`(그 창 복원 크기 · 최대화 · 틀 inset — **값이 바뀔 때만**) · 마운트 때 `window_frame` 당기기(§3-8) |
| 누름(pointerdown) | 웹 → 셸 | 기존 `switch_tab`(R2 누르는 순간 활성 — 끌기 시작 전 · 클릭과 같은 길) |
| dragstart ~ drop · dragend | 웹 ↔ 셸 · 웹 ↔ 웹 | **끌기 관련 0** — 끌기는 자기 판정 · 상태를 셸에 보내지 않는다(ADR-0297 결정 1). 끌기와 무관한 호출(터미널 `resizePty` · `window_frame` · 캔버스 크기 보고 · LLM 명령)은 평소대로 흐른다(§4-7). 창 사이 통신도 없다 — 대상이 아는 것은 끌기 자료의 타입 문자열뿐이다(§4-2) |
| 우리 웹뷰에서 drop | 웹 → 셸 | `tab_drop` 1 회 — 같은 창 같은 자리면 0 회 |
| 출처 dragend(`none` · `move` 모두) | 웹 → 셸 | `tab_drop{NewWindow{by: DragEnd{effect}}}` 1 회 — **같은 창 같은 자리 놓기 · 엔진이 접은 끌기는 0 회**(사용자 결정 2026-10-20 「드래그 스타트 안보내는거면 엔드도 보내지마」 · §2-4 ④) |
| 처리 뒤 | 셸 → 웹 | 적용이면 기존 알림 `window:tabs-updated` · `layout:updated` → 그 뒤 **`tab_drop:done{drag, outcome}`**(출처 창(`from`)에만 · 처리마다 많아야 1 회 · 중복과 맡은 `Drop` 에 자리를 내준 거절은 안 냄 — §3-2 ④) |
| `done` 이 취소 · 무시 | 웹 → 셸 | 출처의 `switch_tab_if(initial, expect = 끄는 탭)` 0–1 회(§4-6) |

- 끄는 동안의 다른 통신(에이전트 출력 · 터미널 크기 맞춤 · 창 틀 조회 · 캔버스 크기 보고 · LLM 명령 따위)은 이 불변식 밖이고 막지도 미루지도 않는다(ADR-0297 결정 1). 끌기는 창을 움직이지 않는다 — 끄는 동안 창 크기가 바뀌면 받는 창이 띠 기하를 다시 센다(ADR-0297 「영향」 · §4-5).
- **끌기가 일으키는 터미널 크기 맞춤도 막지 않는다**(ADR-0297 결정 1 — 옛 판의 미루기는 걷었다 · §4-7) — R15 남은 탭 보이기 · 누름의 `switch_tab` 이 keep-alive 탭의 `display:none` 을 바꿔 `TerminalSlot` 이 끄는 동안 `resizePty` 를 보내도 그대로 둔다. 끌기의 판정 · 상태가 아니기 때문이다. 시험 = C7(끌기 명령만 센다 · §8-4).
- **끌기 하나 = `tab_drop` 많아야 둘 · 놓기 쓰기(`dock_tab`) 많아야 한 번.** 출처는 dragend 마다 `DragEnd` 하나를 보내고(§4-2 · 같은 창 같은 자리 놓기 · 엔진이 접은 끌기는 0 — §2-4 ④), 우리 웹뷰가 받았으면 그 창이 `Drop` 하나를 더 보낸다. 둘이 함께 오는 것이 보통이고 어느 순서로 닿든 셸 장부가 `Drop` 을 이기게 한다(§3-4). 그 밖 = 같은 사건 두 번 배달.
- `window:frame` · `tab_drop:done` 은 기존 이벤트 표면의 평범한 셸 → 웹 사건이다 — 새 전역 핸들(CLAUDE.md 「LLM-우선 제어」)이 아니다. 듣는 쪽이 자기 disposer 를 갖는다(`dock/windowFrame.ts` · `dock/tabDrop.ts` — 구독 수명 가름 규칙). **웹은 둘 다 `listen(…, { target: getCurrentWindow().label })` 로 듣는다** — 셸이 `emit_to` 로 그 창에만 내도 그냥 `listen()`(타깃 `Any`)은 남의 창으로 지목된 사건까지 받는다(Tauri 2.11.3 `event/listener.rs` 의 `match_any_or_filter`). `WindowFrame` 몸에는 label 이 없어 받은 쪽이 가를 수도 없다. 같은 꼴 = `src/api/restoreClient.ts:244-247`.

### 2-3 흐름

1. **누름** — 탭 머리 pointerdown 이 그 탭을 곧바로 활성으로 한다(R2 · `switch_tab`). 웹은 두 가지를 적는다: 누르기 전 그 창의 모델 활성 탭 `initial` · 누른 점(머리 안 비율 · y). 움직이지 않고 떼면 클릭이다(S14 5/5). 두 번 누르면 이름 바꾸기다(2/2).
2. **dragstart**(Chromium 문턱) — 출처 웹뷰가 실음 · 원래 크기 · 새 창 크기 · 끄는 탭 모양 · 그림을 단다(§4-2). `effectAllowed = 'move'` 를 두고, 사건은 `stopPropagation` 으로 끊는다(§4-2 react-dnd).
3. **dragover** — 모든 우리 창이 뿌리에서 받고(`preventDefault` · `dropEffect = 'move'` · 끊기) `judge` 한다(§4-3).
   - **출처 탭바 구역** = Chrome 식 순서 이동 모습이다(R23 · transform).
   - **다른 창 탭바 구역** = 2 px 세로선이다(W13).
   - **칸** = 유효한 띠 · 미리보기(R3 · R4 · R8). 띠가 아니면 칠하지 않는다(R6).
   - **출처 창은 탭바 구역 밖이면 남은 탭을 보인다**(R2 · R15 — 구역에 돌아오면 끄는 탭을 다시 보인다).
   - 우리 창이 아닌 곳 · 가려진 곳 · 최소화된 창에는 OS 가 사건을 주지 않는다(R15).
4. **끌기의 끝** — 출처는 dragend 마다(`dropEffect` 가 `none` 이든 `move` 든) `NewWindow{by: DragEnd{effect}}` 를 하나 보낸다(§4-2 · 같은 창 같은 자리 놓기 · 엔진이 접은 끌기는 보내지 않는다 — §2-4 ④). 셸이 가른다(§3-5): `none` 이고 버튼 · Esc 눌림 = 취소 · 커서 밑 맨 위 창이 우리 창 = 무시 · `move` 인데 맨 위 창을 못 읽음 = 무시 · 그 밖 = 새 창. 그 위에 끝이 어디였나로 둘이 갈린다.
   - **a. 우리 웹뷰에서 drop** — 처리기 첫 문장이 `preventDefault` 라 출처 dragend 는 `move` 다. 받은 창이 마지막 `judge` 결과로 `tab_drop` 을 하나 더 보낸다. 대상은 `TabBar` · `Seam` · `NewWindow{by: Drop}`(칸 가운데 · 그 밖 — R6 · R22)이고, 같은 창 같은 자리면 보내지 않는다(출처 dragend 도 보내지 않는다 — §2-4 ④). 같은 끌기의 `DragEnd{move}` 는 맨 위 창이 우리 창(또는 못 읽음)이라 무시로 갈리고, 둘이 어느 순서로 닿든 `Drop` 이 이긴다(§3-4 — 맨 위 창을 `Foreign` 으로 잘못 읽은 경우는 빼고 · §9-1 M13). 출처는 그 무시로 되돌리지 않는다(§4-6).
   - **b. 우리 웹뷰 밖**(다른 앱 · 바탕화면 · 우리 창 제목 표시줄 · 테두리) · Esc · 비정상 끝 — `DragEnd` 하나뿐이다. 제목 표시줄 · 테두리 = 아무 일 없음(사용자 결정 Q38 — 「별도의 처리를 안하는 방향으로.」) · 다른 앱이 받았어도(dragend `move`) 새 창(사용자 결정 Q39 — 「새창.」 · 맨 위 창을 못 읽으면(`Unknown`) 무시 — §3-5 ⑤ · §9-2). 곧 「우리 창 위면 아무 일 없음 · 그 밖은 전부 새 창」이다(예외 — §9-2 · §9-5: 끄는 중 LLM 이 끄는 탭을 다른 창으로 옮기거나 닫음).
   - 옛 「c. 다른 앱이 받음(`copy` · `link` · `move`) = 보내지 않음 · 탭 제자리」는 걷었다 — `effectAllowed = 'move'` 라 `copy` · `link` 는 오지 않는 죽은 길이었고, `move` 는 Q39 로 새 창이 됐다(재리뷰 2026-10-20).
5. **처리 뒤** — 셸이 `tab_drop:done` 을 낸다. 각 창 스토어는 앞선 모델 알림으로 이미 갱신돼 있고, 출처는 `done` 으로 바꿔 그리기를 풀며(같은 웹뷰 제자리 놓기는 dragend 에서 이미 풀었다 — §2-4 ④), 취소 · 무시면 원래 선택을 되돌린다(자기 dragend 가 `move` 인 무시 · 취소는 빼고 — §2-4 · §4-6).

### 2-4 순서 · 경합

- **출처의 바꿔 그리기(남은 탭 보이기) 풀기** — 순수 리듀서 `dock/dragOverride.ts`. **시한이 없다** — 푸는 계기는 넷이다(재리뷰 2026-10-20 B5 로 넓힘 · ④ = 재리뷰 3차).
  - **① 그 끌기의 `tab_drop:done`**(자기 invoke 오류면 곧바로). `done` 이 `Applied` 면 「그 창 스토어의 탭 목록이 dragstart 때와 다름」도 닿은 뒤에 푼다(옮겨 나감 · 순서 바꿈 · 같은 창 이음새 셋 다 목록이 바뀐다 · 탭 하나뿐인 팝아웃은 바꿔 그림이 없다). 셸이 `done` 을 모델 알림 뒤에 같은 스레드에서 내므로 실제로는 같은 틱이다. 놓은 뒤 모델 활성(`successor` — §3-3)이 바꿔 그리던 탭과 같아 풀 때 화면이 바뀌지 않는다(시험 §8-3). 자기 dragend 가 `move` 인데 `done{Ignored}` · `done{Canceled}` 면 그것으로는 풀지 않는다(뒤따르는 `Drop` 의 결과를 기다림 — 끝 상태는 이기는 `Drop` 이 정하고, 그 `Drop` 을 잃거나 그 `Drop` 이 무시되면(`no-cursor`) ② · ③ 이 푼다 · §4-6).
  - **② dragend 뒤 그 창의 `window:tabs-updated` 가 끄는 탭을 활성으로 두지 않을 때**(목록에서 빠짐 · 다른 탭이 활성 — LLM 의 `switch_tab` 포함). 그래서 늦게 남은 바꿔 그림이 LLM 이 고른 탭을 덮지 않는다. 끄는 탭이 활성인 알림(늦게 닿은 누름의 `switch_tab` 따위)으로는 풀지 않는다 — 풀면 `Drop` 적용 전 끄는 탭이 보인다(§8-3 ②). 끄는 탭이 그 창 탭 목록에서 **빠지는** 알림은 dragend 전이어도 푼다(§9-3 U2 ⑥ — 사용자 결정 2026-10-20 그대로 둠 「극 예외는 실 발생될때 처리」).
  - **③ 그 창의 다음 pointerdown · dragstart.**
  - **④ 같은 웹뷰 제자리 놓기 — 출처 dragend 에서 곧바로.** 받은 웹뷰가 출처 자신이고 drop 이 아무것도 보내지 않았으면(같은 창 같은 자리 탭바 — §4-3) 그 웹뷰가 drop 때 그 drag id 를 적어 두고, 뒤이은 출처 dragend 가 그것을 읽어 **`DragEnd` 도 보내지 않고** 바꿔 그리기를 풀고 transform 을 0 으로 거둔다(사용자 결정 2026-10-20 「드래그 스타트 안보내는거면 엔드도 보내지마」 — 엔진이 접은 끌기도 같다 · §9-3 U2 ⑨). 이 끌기는 셸 호출 0 회라 `done` 이 없다. 모델은 그대로라 끄는 탭이 그 자리에 보인다(클릭과 같음).
  - 출처가 dragend 마다 `DragEnd` 를 보내므로 `done` 은 보통 늘 오고, 보내지 않은 제자리 놓기 · 엔진이 접은 끌기는 ④ 가 푼다. ② · ③ 은 invoke 나 사건이 사라진 경우의 그물일 뿐이다.
- **자기 탭바 순서 이동 모습** — 끄는 동안은 transform 만 쓴다. DOM 순서 · React key 는 바꾸지 않고, 끄는 머리 노드를 떼지 않는다(떼면 끌기 출처가 사라진다). 놓기 뒤에는 모델 알림이 온 그 프레임에 transform 을 0 으로 거둔다 — 그사이 순서가 튀지 않게. 보내지 않은 제자리 놓기는 모델 알림이 없으므로 출처 dragend 에서 곧바로 거둔다(④).
- **대상 미리보기 · 세로선 지우기**(VS Code 꼴) — drop · 창 떠남(뿌리 `dragleave` · 관련 대상 없음) · **300 ms 동안 dragover 없음** · 덮개 위 `mousemove`(끌기가 끝났다는 뜻) 뒤 300 ms. 멈춘 커서에도 dragover 는 약 63 ms 마다 오고(S1), 다른 창에서 끝난 끌기는 그 창에 dragend 를 주지 않으므로 시한이 지운다. 덮개 위 `mousemove` 는 주 버튼을 뗀 것(`buttons === 0`)만 본다 — 끄는 동안의 합성 · 밀린 이동(바꿔 그리기가 낳는 hover 재계산 따위)은 버튼이 눌린 채라 거른다(`dock/windowDrop.ts`). ★「HTML5 끌기 동안 출처 · 대상은 버튼 눌린 마우스 사건만 받는다(`buttons ≠ 0`)」는 실측하지 않은 전제다★ — 틀리면 끄는 동안 미리보기가 300 ms 뒤 지워지고 새 dragenter 까지 되살지 않는다(놓기는 그 점에서 판정 — §4-3 · M11 이 잰다 · 옛 §4-7 의 그물 (b) 가 같은 전제에 기댔다).
- **놓기가 모델 알림보다 먼저 셸에 닿으면**(전송 중 바뀜 — R4 · R13c · LLM 이 같은 탭을 옮김 · D9) — 명령은 판정 때 상태를 싣지 않고, 셸이 놓을 때 대상을 id 로 다시 찾아 있으면 지금 상태에 맞춰 적용 · 없으면 무동작이다(규칙 정본 = §3-3 「놓을 때 다시 찾기」 · ADR-0297 결정 2). 놓기가 본 상태와 다르다는 것만으로는 거절하지 않는다 — 결과가 조금 어긋나도 치명적이지 않고 다시 끌면 된다(사용자 결정 2026-10-20 「크리티컬한게 아니고 어차피 유저가 다시 하면 되는거라서」).
- **누름의 `switch_tab` 왕복이 놓기보다 늦으면** — 놓을 때 활성을 대조하지 않으므로 거절되지 않는다(옛 판의 `active` 사전조건은 걷었다 · ADR-0297).

## 3. 셸 — 놓기 명령 `tab_drop`

### 3-1 모양

```rust
// 명령 = src-tauri/src/commands/tab_drop.rs · wire 타입 = src-tauri/src/tab_drop/wire.rs(ts-rs → src-tauri/bindings/) · 부른 창 label 은 인자가 아니라 Tauri `Window` 에서 꺼낸다(기존 invoke 규칙)
#[tauri::command]
pub async fn tab_drop(window: Window, app: AppHandle, layout: State<'_, LayoutState>, book: State<'_, Arc<DropBook>>,
                      router: State<'_, Arc<OutputRouter>>, counter: State<'_, Arc<PopupCounter>>, client: State<'_, Arc<DaemonClient>>,
                      drop: TabDrop) -> Result<TabDropReply, String>;

pub struct TabDrop {
    pub drag: Uuid,             // dragstart 에 출처가 뽑는다(crypto.randomUUID) — 장부 열쇠(§3-4)
    pub view: Uuid,             // 끄는 탭
    pub from: String,           // dragstart 때 그 탭의 창 label — `done` 을 받는 출처 창(사전조건이 아니다 — ADR-0297)
    pub initial: Option<Uuid>,  // 누르기 전 그 창 활성 — 다음 활성(R2 · R15 「Chrome 규칙이 고른 남은 탭」)
    pub to: DropTo,
}
pub enum DropTo {
    /// 부른 창의 탭바. index = 결과 탭 목록 자리(DockTo::TabBar 뜻 — 지금 목록 끝을 넘으면 끝 자리 · §3-3)
    TabBar { index: u32 },
    /// 부른 창에 보이던 탭 target 의 이음새(anchor 칸이 남아 있으면 그 칸 · §3-3)
    Seam { target: Uuid, anchor: DropAnchor, side: DockSide, share: f64 },
    /// 새 창(R6 · R22). size = 새 창 안쪽 크기(논리 · null = 탭 하나뿐인 팝아웃 → 그 창을 옮김 · Q34) · grab = 새 창
    /// 뷰포트에서 커서가 올 점(CSS px · §3-5)
    NewWindow { by: NewWindowBy, size: Option<SizeCss>, grab: PointCss },
}
pub enum NewWindowBy { Drop /* 우리 웹뷰의 drop — 놓기가 확실 */, DragEnd { effect: EndEffect } /* 출처 dragend(`none` · `move` 모두) — 셸이 가른다 */ }
/// 출처 dragend 의 `dropEffect` — wire 값 = 그 문자열 그대로(`none` · `move`). `move` = 어딘가가 놓기를 받았다
pub enum EndEffect { None, Move }
pub enum DropAnchor { Slot { id: Uuid }, Outer }
pub enum TabDropReply { Applied { window: String, reused: bool }, Canceled, Ignored { why: String }, Duplicate,
                        Rejected { kind: String, reason: String } }
/// 사건 `tab_drop:done` — 처리마다 출처 창(`from`)에만 많아야 1 회 · `Duplicate` 와 맡은 `Drop` 에 자리를 내준 거절은 내지 않는다(§3-2 ④)
pub struct TabDropDone { pub drag: Uuid, pub outcome: DropOutcome /* Applied | Canceled | Ignored | Rejected */ }
```

- `DockSide` 의 wire 값은 variant 이름 그대로다(`layout/dock.rs` 머리 — rename 금지). 지금은 `TS` derive 가 없어 **U1 이 `layout/dock.rs:21` 에 단다**(바인딩 `DockSide.ts`). 나머지 serde 태그 꼴은 이웃 wire 타입과 맞춘다. U1 이 실었던 판정 때 상태 칸(`TabBar.expect` · `Seam.rev` · `Seam.active`)은 ADR-0297 결정 2 로 뺀다 — 셸이 놓을 때 대상을 다시 찾는다(§3-3).
- `PointCss` — U1 ~ U4b 동안은 `layout/chrome.rs` 의 것을 쓴다. U4c 에서 그 파일을 지울 때 `RectCss` 와 함께 같은 모양 그대로 `layout/bands.rs`(§4-5)로 옮긴다. 같은 이름의 타입을 둘 세우면 ts-rs 가 한 바인딩 파일을 서로 덮어쓴다 — 둘을 세우지 않는다. `SizeCss` 는 새로 세운다. 틀 inset 은 이미 있는 `layout::geometry::Insets`(t · r · b · l · CSS px — 바인딩 `Insets.ts`)를 쓴다(뜻 설명만 넓힌다).
- **버스 명령이 아니다**(§3-6). 웹 쪽 감싸개와 `done` 듣기는 `dock/tabDrop.ts` 하나다.

### 3-2 셸 처리 순서

1. **입력 읽기를 맨 먼저 한다** — 락 · 장부보다 앞이다. `DragEnd` = platform `input::cursor_pos` · `primary_button_down` · `escape_down`, 이어서 `window::top_level_at(cursor)` → 그 창이 우리 웹뷰 창이면(`NativeWindow::of` 와 견줌) `TopAt::Ours(label)` · 아니면 `Foreign` · 커서 · 창을 못 읽으면 `Unknown`. `Drop` 은 커서만 읽는다 — drop 사건 자체가 놓기의 증거다. dragend 뒤 0–0.1 ms 에 읽으면 놓은 점과 같다(S14 · S1 — 우리 창 안에서만 잼). **끌기가 끝난 뒤의 호출이다** — 출처 dragend 가 보낸 명령 안에서 읽으므로 「끄는 동안 끌기 명령 0」(ADR-0296 · ADR-0297)은 그대로다.
2. **`tab_drop::plan(&drop, caller, &InputNow, &SourceFrame)`**(순수 · Tauri 0) → `Plan::Dock(DockRequest) | Plan::Cancel | Plan::Ignore(why)`. `SourceFrame` = 출처 창 틀 inset · 배율(Tauri 게터 — 입력 읽기 **뒤**에 읽는다 · §3-5).
3. **장부** — `DropBook::admit`(§3-4)이 받지 않으면 `Duplicate` 다(debug 로그 · 알림 없음). 같은 끌기의 `DragEnd` 새 창이 진행 중일 때(그 거절 알림을 내는 중 포함) 닿은 `Drop` 도 `Duplicate` 로 답하되 장부가 맡아 둔다(④ · §3-4).
4. `Dock` → `apply::dock_tab(&layout, &RouterSubs{..}, &TauriEvents{..}, &TauriWindowHost{..}, &**counter, req)` — 팝아웃 · T-65 와 같은 어댑터다. **async 명령에서 부른다** — `NewWindow` 를 메인 스레드 · 동기 명령에서 부르면 창 짓기가 교착한다(`dock_tab` 문서). **`by: DragEnd` 의 새 창은** `dock_tab` 이 돌아온 뒤 `DropBook::settle(drag, outcome)` 이 결과를 적는다(§3-4 · 잎 락 · 레이아웃 락을 놓은 뒤). 적용이면 그동안 맡아 둔 `Drop` 을 버린다(debug 한 줄 · 알림 없음). **거절인데 맡은 `Drop` 이 있으면 이 처리는 자기 `done` 을 내지 않는다**(⑦ 의 info 줄은 남기고 `superseded = true` 를 단다) — 그 `Drop` 을 보통 `Drop` 과 같은 길(④ ~ ⑦)로 이어 처리하므로 출처가 받는 그 끌기의 `done` 은 그 `Drop` 의 것 하나다. 거절 `done` 이 먼저 가면 출처가 바꿔 그리기를 풀어(§2-4 ①) 끄는 탭이 번쩍인다. **맡은 것이 없으면** 기록을 「거절 알림 중」(`Rejecting`)으로 두고 `done{Rejected}` 을 낸 **뒤** `DropBook::close(drag)` 를 부른다 — 기록을 「거절된 `DragEnd`」로 낮추고, 그사이 맡아 둔 `Drop` 이 있으면 돌려줘 같은 길로 이어 처리한다(그 `done` 은 거절 뒤 — 차례가 보장된다 · 그 틈의 번쩍임은 받아들임 · §9-2). 옛 판은 알림 전에 기록을 낮춰, 그 틈에 닿은 `Drop` 이 다른 스레드에서 적용되면 그 `done{Applied}` 이 거절보다 먼저 갈 수 있었다(재리뷰 2026-10-20 · 메인 결정).
5. 성공이고 `TabBar` · `Seam` 이면 옮긴 탭이 든 창(보통 부른 창 — 이음새 대상 탭이 그사이 다른 창으로 갔으면 그 창 · 응답 `Applied.window` 와 같다 · §3-3)에 Tauri `set_focus()` 를 부른다(R11b). 6.3 의 「`set_focus` 금지」(F7 — tao 가 합성한 Alt 를 끌기 키 감시가 놓기로 읽음)는 그 감시가 없어져 풀린다. 새 창은 `open_at(activate = Reveal)` 이 활성으로 연다.
6. 응답: `DockError` → `Rejected{kind, reason}` · 성공 → `Applied{window, reused}` · `Cancel` → `Canceled` · `Ignore` → `Ignored{why}`. 취소 · 무시는 셸이 모델을 쓰지 않는다 — 누름이 바꾼 선택은 출처가 끝난 뒤 되돌린다(§4-6).
7. **알림 · 로그** — 락을 다 놓은 뒤 `tab_drop:done{drag, outcome}` 을 출처 창(`from`)에만 낸다(그 창이 닫혔으면 debug 한 줄로 건너뜀 · 적용이면 `dock_tab` 의 모델 알림 뒤 — 같은 스레드에서 이어 낸다 · 맡은 `Drop` 에 자리를 내준 거절이면 내지 않음 — ④). 로그(`docs/reference/logging-conventions.md` — 정상 수명 사건 = info · 식별자 · 수치는 필드) = 처리마다 info 한 줄: `drag` · `by`(`DragEnd` 면 `effect` 도) · `view` · `from` · `to`(갈래) · `decision` · `cursor` · `button` · `esc` · `top`(`ours=<label>` · `foreign` + 그 창 클래스 이름 · `unknown` — 클래스는 M13 이 끌기 그림 창을 가르는 데 쓴다) · 결과(거절이면 `kind` · 무시면 `why` · 맡은 `Drop` 에 자리를 내준 거절이면 `superseded = true`). `Duplicate` 는 debug 한 줄.

### 3-3 대응표 — `DropTo` → `DockRequest`

| `DropTo` | `DockDest` | `DockGuard` | `focus` | `mv.successor` | `on_emptied` |
|---|---|---|---|---|---|
| `TabBar{index}` | `TabBar{window: 부른 창, index: Some(index)}`(지금 목록 끝을 넘으면 끝 자리) | `DockGuard::NONE` — 대상 존재만(아래 「놓을 때 다시 찾기」) | `Reveal` | `initial`(≠ `view` 이고 적용 때 끄는 탭이 `from` 에 있을 때 — `successor_window = from`) | `Close` |
| `Seam{target, anchor, side, share}` | `Seam{target, anchor: Slot(id) \| Outer, side, share}` | 〃 | `Reveal` | 〃 | `Close` |
| `NewWindow` → 놓기 | `NewWindow{at: WindowPlace{pos: Some(new_window_origin(..)), size}}` | 〃 | `Reveal` | 〃 | `Close` |

- `lease = None` · `keep_lease = false` — 끌기 표식은 쓰지 않는다. 발급자(옛 세션)가 사라진다(U6).
- **`successor`** — `initial ≠ view` 면 `initial`, 아니면 셸 `succession`(오른쪽 이웃 → 왼쪽)이다. 이것이 R2 · R15 의 「Chrome 규칙이 고른 남은 탭」이고, 출처가 끄는 동안 바꿔 보인 탭과 같다(§4-2). **`initial` 은 끄는 탭이 놓을 때도 `from` 에 있을 때만 쓴다**(`DockMove.successor_window = from` — 적용 때 옮긴 탭이 그 창에 있을 때만 `successor` 를 쓴다) — 아니면 그 탭이 지금 사는 창에 셸 `succession` 을 쓴다(아래 「다른 창으로 옮겨졌으면」).
- **`size: None`**(탭 하나뿐인 팝아웃) — 기존 `plan_new_window` 가 `Reuse` 로 그 창을 `at.pos` 로 옮기고 크기를 유지한다(Q34). 경합으로 그사이 탭이 늘었으면 `Create` 가 출처 보통 크기로 연다.
- 탭 하나뿐인 팝아웃을 다른 창 탭바나 칸에 놓으면 출처가 비고, `OnEmptied::Close` 가 그 창을 닫는다(Q34 · R10).
- **놓을 때 다시 찾기**(ADR-0297 결정 2 · 사용자 결정 2026-10-20 — 옛 「사전조건을 왜 싣나」를 대신한다) — 판정은 웹 스토어로 했고 그 스토어가 모델보다 늦을 수 있다(§2-4). 셸은 놓기가 가리킨 대상을 id 로 다시 찾는다 — 끄는 탭 · 대상 탭(`Seam.target`) · 대상 칸(`anchor = Slot(id)`). **있으면 지금 상태에 맞춰 적용한다** — `TabBar` 자리가 지금 목록 길이를 넘으면 끝 자리로 자른다 · `Seam` 은 anchor 칸이 남아 있으면(그사이 나뉘었어도) 그 칸에 붙인다. **끄는 탭 · 대상 탭 · 대상 칸 중 하나라도 없으면 무동작이다**(거절 = 모델 그대로 · §3-4). 놓기가 본 상태(탭 목록 · 대상 뷰 판 번호 · 그 창 활성 · 출처 창)와 다르다는 것만으로는 거절하지 않는다 — 관행의 주류(VS Code · dockview · FlexLayout · Zed · Lumino 도크 — dockview 만 무동작 대신 던진다)와 같다(조사 `docs/research/drag-external-events-practice-2026-10-20.md`). 셸은 놓기에 `DockGuard::NONE` 을 넘긴다 — 옛 판의 `TargetExpect::Tabs` · `SeamAt{window, rev, active, or_initial}` · `expect_window` 는 쓰지 않는다(`SeamAt` · `expect_window` 는 걷었고 남은 옛 대조는 U6 이 걷는다 — §3-7). 되살리지 말 것(ADR-0297 「영향」).
  - **대상이 그사이 다른 창으로 옮겨졌으면**(메인 결정 2026-10-20 — ADR-0297 「있으면 지금 상태에 맞춰」의 적용) — 끄는 탭은 **지금 사는 창**에서 빼 온다. 그 창에 남을 탭은 셸의 평소 규칙(`succession` — 탭을 닫을 때와 같음)으로 고르고, `initial` 은 그 창이 `from` 일 때만 쓴다(`successor_window` — 위 `successor`). `Seam` 대상 탭이 다른 창으로 갔어도 그 탭과 anchor 칸이 남아 있으면 **그 탭이 지금 사는 창에서** 붙이고 포커스도 그 창에 준다(§3-2 ⑤ · 응답 `Applied.window`).
  - **받아들인 것 — 옛 `SeamAt.window` 대조를 잃음**(ADR-0297 「있으면 지금 상태에 맞춰」) — 웹 결함으로 다른 창의 탭을 이음새 대상으로 보내도 셸은 거절하지 않고 그 탭이 사는 창에 말없이 붙인다. 정상 길에서는 대상 탭이 부른 창의 보이는 탭이라 생기지 않는다.

### 3-4 거절 · 장부 · 무변경

- **거절** — `DockError` 를 `Rejected{kind, reason}` 으로 돌려준다(`kind` = 오류 문구 괄호 속 낱말). 모델 그대로 · 웹은 화면을 바꾸지 않고 debug 로그만. 생기는 경우 = 대상 없음(ADR-0297 결정 2 의 무동작 · §3-3 — 끄는 탭 · 이음새 대상 탭 = `TabNotFound` · 탭바 창 = `WindowNotFound` · 대상 칸 = `SlotNotFound` · 그 칸이 다른 탭으로 갔으면 `SlotNotInTab`) · `ZeroArea`(웹 거울이 놓친 경우 — §4-5) · `WindowOpen` · 웹 결함으로만 `InvalidShare` · `SelfTarget`. 놓기는 `DockGuard::NONE` 이라 대조 거절 `SourceChanged` · `TargetChanged` 가 나지 않는다 — 그 변형과 그것을 내는 옛 엔진 대조(`expect_source` · `TargetExpect::{Seam, Tabs}`)는 U6 까지 남고, 놓기 몫이던 `expect_window` · `SeamAt` 은 걷었다(ADR-0297 · §3-7).
- **장부 `DropBook`** — 최근 drag id 64 개와 끌기마다의 갈래: `Drop`(처리함) · `Quiet`(모델을 쓰지 않은 `DragEnd`) · `Placing`(새 창 진행 중) · `Rejecting`(새 창이 거절돼 그 알림을 내는 중) · `Placed`(새 창을 만듦). `Placing` · `Rejecting` 은 그동안 닿은 첫 `Drop` 을 품는다. `admit(Handling) -> Admit`(`Handling` = 명령 · 부른 창 · 입력 · 맨 위 창 클래스 · 판정 — 맡아 둘 수 있게 처리에 쓰는 것을 한데 쥔다 · `Admit` = `Run(Handling) | Parked | Duplicate`)은 한 락 안에서 판정하고 적는다. `settle` · `close` 도 같은 락이다(잎 락 — 이 셋 안에서만 쥐고, 쥔 채 레이아웃 락 · emit · 창 호출 없음). 넘치면 끝난 기록 중 가장 오래된 것부터 잊는다 — 진행 중(`Placing` · `Rejecting`)은 맡은 `Drop` 을 잃지 않게 잊지 않는다(64 개가 모두 진행 중이면 — 실제로는 없다 — 늘리고 warn 을 장부마다 한 번 낸다).
  - 처음 본 id → 받음.
  - **`Drop` 인데 앞선 기록이 `DragEnd` 의 취소 · 무시 · 거절된 새 창(그 거절 알림 · `close` 뒤) → 받음(Drop 이 이긴다 · 기록을 `Drop` 으로 바꿈).** 앞선 `DragEnd` 는 모델을 쓰지 않았다 — 커서가 우리 창 위면(§3-5 ③) · `move` 인데 맨 위 창을 못 읽었으면(§3-5 ⑤) 새 창을 만들지 않는다 — 그리고 확실한 놓기가 뒤에 왔다.
  - **`DragEnd` 의 새 창은 `admit` 때 `Placing` 으로 적고, 그 결과를 `settle` 이 적는다**(§3-2 ④ · 재리뷰 2026-10-20 F6). 진행 중에 닿은 `Drop` 은 그 자리에서 처리하지 않고(둘이 함께 모델을 쓰지 않게) **장부가 맡아 둔다**(첫 것만 · 그 호출의 답은 `Duplicate`). 새 창이 서면 `Placed` 로 적고 맡은 `Drop` 은 버린다(알림 없음). 거절이면 — 맡은 것이 있으면 `settle` 이 기록을 `Drop` 으로 적고 그것을 돌려준다(그 `DragEnd` 처리는 `done` 없이 그것을 이어 처리 — 출처의 `done` 은 그 `Drop` 의 것 하나). 없으면 `Rejecting` 으로 두어 거절 알림을 내는 동안 닿는 `Drop` 도 맡고, 알림 뒤 `close` 가 기록을 「거절된 `DragEnd`」(`Quiet`)로 낮춘다 — 그사이 맡은 것이 있었으면 `Drop` 으로 적고 돌려줘 이어 처리하고, 그 뒤에 닿는 `Drop` 은 위 둘째 줄로 이긴다(재리뷰 2026-10-20 · 메인 결정). 옛 판은 적용 결과를 보기 전에 「새 창을 만든 `DragEnd`」로 적어 거절 뒤의 `Drop` 을 `Duplicate` 로 버렸다.
  - 그 밖(같은 id 의 `Drop` 둘 · `Drop` 뒤 `DragEnd` · 새 창을 만든 `DragEnd` 뒤 `Drop` · 이미 맡아 둔 것이 있을 때의 둘째 `Drop`) → `Duplicate`.
  - 생길 수 있는 경우 = **우리 웹뷰가 받은 끌기는 늘 둘이 온다** — 받은 창의 `Drop` 과 출처의 `DragEnd`(§2-3 4a · 출처는 dragend 마다 보낸다 · 같은 창 같은 자리 놓기는 둘 다 0 — §2-4 ④). 순서는 정해져 있지 않다. 그 밖 = 같은 사건 두 번 배달. `DragEnd` 가 먼저면 그 dragend 는 `move` 라(§4-3) 맨 위 창이 우리 창이거나 못 읽었으면 무시로 적히고 위 둘째 줄로 `Drop` 이 이긴다. **맨 위 창을 `Foreign` 으로 잘못 읽으면**(끌기 그림 · 다른 앱 덮개 · 알림 창 따위 — M13) 새 창 진행 중으로 적혀 그 새 창이 서면 뒤의 `Drop` 은 `Duplicate` 다 — 우리 웹뷰가 받은 놓기가 새 창이 된다(§9-1 M13 · §9-2). 그 새 창이 거절되면 맡아 둔 `Drop` 이 이어 처리된다. `Drop` 이 먼저면 뒤의 `DragEnd` 가 `Duplicate` 다.
- **무변경** — 같은 창 같은 자리면 웹이 보내지 않는다(출처 dragend 도 보내지 않고 바꿔 그리기를 스스로 푼다 — §2-4 ④). 보내도 `dock_tab` 이 무변경 `Ok` 다.
- **재시도 없음** — IPC 오류면 웹은 로그만 남긴다. 출처 자기 invoke 오류면 바꿔 그리기를 곧바로 풀고, 받은 창의 invoke 오류는 출처가 모른다(자기 dragend 가 `move` 라 `DragEnd` 의 무시로는 풀지 않는다 — §4-6) — 그물 계기(§2-4 ② · ③)가 푼다.

### 3-5 새 창 — 가름 · 자리 (`tab_drop/plan.rs` · 순수)

```rust
pub enum TopAt { Ours(WindowLabel), Foreign, Unknown }
pub struct InputNow { pub cursor: Option<PhysPoint>, pub primary_down: bool, pub escape_down: bool, pub top: TopAt,
                      pub scale: f64 /* 커서 밑 모니터 */ }
pub fn decide(by: &NewWindowBy, i: &InputNow) -> Decision;   // Cancel | Ignore(why) | Place(cursor)
pub fn new_window_origin(cursor: PhysPoint, scale: f64, inset_dip: (f64, f64), grab: PointCss) -> (i32, i32);
//   = cursor − round((inset_dip + grab) × scale)   · round = .5 는 0 에서 멀리
```

**`decide` — 차례가 계약이다**

| # | 조건 | 결과 | 근거 |
|---|---|---|---|
| ① | `Drop` | `Place` | drop 사건이 놓기의 증거다. 커서 없음이면 `Ignore("no-cursor")` |
| ② | `DragEnd{None}` + (Esc 눌림 ∨ 주 버튼 눌림 ∨ 커서 없음) | `Cancel` | R12 · R13b. Esc 끝 = 버튼 · Esc 눌림 4/4 · 놓기 = 둘 다 뗌 5/5(우리 창 안 실입력 — 바탕화면 = 미검 · M1). 버튼이 가장 굳은 판별이다(Esc 를 짧게 쳐도 버튼은 쥐고 있다). `DragEnd{Move}` 는 이 줄을 건너뛴다 — 어딘가가 놓기를 받았으니 취소된 끌기가 아니다 |
| ③ | `DragEnd` + `top = Ours(_)` | `Ignore` — `None` = `"lost-drop"` · `Move` = `"accepted-ours"` | 우리 웹뷰 위에서 뗐다면 뿌리가 늘 받으므로(§4-3) 그 창의 `by: Drop` 이 따로 온다 — 어느 순서로 닿든 `Drop` 이 이긴다(§3-4). `Move` 면 그것이 보통 길이다. `None` 인데 drop 이 없으면 잃은 놓기다(4.6 결함 · AC27 · AC28 · R13b) — 새 창을 만들지 않는다. 웹뷰 밖 틀(제목 표시줄 · 테두리)도 우리 창이라 무시 = **아무 일 없음**(사용자 결정 2026-10-20 Q38 — 「별도의 처리를 안하는 방향으로.」). z 순서를 보므로 겹친 우리 창 · 다른 창 탭바 위의 잃은 놓기 · 최대화된 출처를 덮은 다른 앱 창(→ `Foreign`)이 갈린다(R22 · AC8) |
| ④ | `DragEnd` + `Foreign`(`None` · `Move` 둘 다) · `DragEnd{None}` + `Unknown` | `Place` | R22. `Foreign` 은 `Move`(다른 앱이 놓기를 받음)여도 같다 — **새 창**(사용자 결정 2026-10-20 Q39 — 「새창.」 · 받아들인 한계 = §9-2). 맨 위 창을 `Foreign` 으로 잘못 읽으면 우리 웹뷰가 받은 놓기도 여기로 온다(§3-4 · §9-1 M13). `Unknown` + `None` = Windows 밖 · 읽기 실패 — 새 창 쪽으로 떨어진다(§9-2) |
| ⑤ | `DragEnd{Move}` + `Unknown` | `Ignore("accepted-unknown")` | 받은 곳을 모른다 — 우리 웹뷰면 그 창의 `Drop` 을 기다리고(③ 과 같다), 다른 앱이면 새 창이 없다(§9-2). 그래서 Windows 밖(`top` 이 늘 `Unknown`)에서도 우리 웹뷰 놓기가 `DragEnd` 와 경주하지 않는다 |

- **걷은 것**(7판 초안 · 리뷰 2026-10-20) — 웹 ↔ 웹 「아직 받는 창이 있나」(VS Code `isWindowDraggedOver` · `BroadcastChannel`) · 변 48 px(`EDGE_PX`) 다듬기 · 「커서가 출처 창 바깥 사각형 안이면 무시」. 겹친 창에서 집계가 틀리고(A 를 겹친 B 위로 끌어 B 밖 바탕화면에서 떼도 「받는 중」) · 탭바 구역이 변 띠 안이라 다른 창 탭바 위 잃은 놓기를 「떠남」으로 읽어 새 창이 생기며(AC27 재발) · 사각형 규칙은 z 를 몰라 최대화된 출처 위 다른 앱 창에 놓아도 새 창이 안 생겼다(R22 · AC8 위반).
- **`top_level_at`** — platform 의 것을 얇게 남긴다(ADR-0296 결정 5 가 남길지를 TRD 에 맡겼다 · §5-1 · ADR-0296 관련 줄에 이 TRD 를 그 정본으로 적었다). z 순서 위에서부터 걸으며 보이지 않음 · 최소화 · DWM 가림 · 비쳐 보이는 창(클릭이 지나가는 창 · 다른 앱의 투명 layered 창 — B2 `hit_accepts`)을 건너뛴다 — 화면 캡처 도구의 덮개가 우리 창을 덮어도 우리 창으로 읽힌다(M8). **끌기 그림 창은 우리 프로세스 것이 아니다** — 끌기 출처 · 그림은 WebView2 브라우저 프로세스 안이라(ADR-0296 「거부한 대안」 셋째) `hit_accepts` 에는 `ours = false` 로 보인다. 그 창이 dragend 때 남아 맨 위로 읽히면(M13) 프로세스가 아니라 **창 클래스**로 건너뛴다(§9-1).

**자리 · 크기**

- **`scale`** = 커서 밑 모니터 배율이다(Tauri `monitor_from_point` · 없으면 출처 창 배율).
- **`inset_dip`** = 출처 창의 (안쪽 원점 − 바깥 원점) ÷ 출처 배율이다 — `window:frame` 의 틀 inset 과 **같은 셸 함수 하나**로 셈한다(§3-8). 팝아웃과 main 의 창 틀이 같다는 전제다. T-66 이 제목 표시줄을 없애면 0 에 가까워진다.
- **`grab`**(출처 웹뷰가 dragstart 에 셈) = (탭바 왼쪽 + r × W1, 누른 y) — r = 누른 점의 머리 안 폭 비율 · W1 = 새 창 탭바의 탭 하나 머리 폭(`layout.ts` 로 새 창 폭에서 셈 — 보통 160). PRD R22 「커서 − 끌기 오프셋 — 누른 점의 탭 안 상대 위치를 지킨다」(Chrome `GetTabOffsetForDetachedWindow`)다. VS Code 의 「커서 − (폭/2, 30 + 높이/2)」는 PRD 가 정해 따르지 않는다.
- **탭 하나뿐인 팝아웃** — 그 탭이 이미 첫 탭이고 W1 = 지금 머리 폭이라, `grab` 은 누른 점이다. 그래서 `Reuse` 는 창을 **끈 만큼 옮긴다**(Q34 · 같은 배율일 때 정확 · 배율이 섞이면 M2).
- **최대화된 탭 하나뿐인 팝아웃을 창 밖에 놓기**(사용자 결정 2026-10-20 「1. 맞고」 — PRD Q37) — 복원한 뒤 「커서 − 끌기 오프셋」(같은 식)에 옮긴다. `WindowHost::move_window` 는 최대화된 창의 최대화를 이미 풀고 보통 크기로 보인다(`apply.rs` 문서). `grab` 의 W1 은 복원 창 폭으로 센다.
- **`size`(새 창 크기 · R22)** = 출처 웹뷰가 dragstart 에 셈해 싣는다(§4-2). 셸은 유한 · 양수인지만 보고, 아니면 `None`(기존 `plan_new_window` 크기)으로 바꾼다. 새 창은 최대화로 만들지 않는다.

### 3-6 LLM 경로 (T-65 · CLAUDE.md 「LLM-우선 제어」)

- **`tab_drop` 은 버스 명령표에 올리지 않는다.** 마우스 놓기의 끝이고 입력 상태를 읽는 갈래가 있어, LLM 이 흉내 낼 것이 아니다.
- **같은 동작의 LLM 명령** = T-65 `tab.dock` 이다. 같은 `apply::dock_tab` 위에 `FocusPolicy::Keep` · `DockGuard::NONE` 으로 얹는다. 변경 경로가 둘이 되지 않는다. PRD R16 · R17 의 의식적 예외는 그대로다.
- **T-65 에 넘길 것** — 붙는 몫 · 새 창 크기 · 띠 유효성의 정본은 셸(`layout/bands.rs` · `layout/drag_size.rs`)에 남는다(§4-5). LLM 의 기본값을 그 함수로 셈하면 마우스와 같다 — 웹 거울과의 같음은 골든이 지킨다.
- **`switch_tab_if` 도 버스 명령표에 올리지 않는다** — 끌기의 원래 선택 되돌리기 전용 웹 → 셸 명령이다(§4-6). LLM 은 `tab.switch` 를 쓴다.
- **끄는 중 LLM 이 같은 탭을 옮기면**(D9) — 셸은 끌기를 모르므로 그 명령은 성공한다(끄는 동안 막지 않는다 — ADR-0297 결정 1). 뒤이은 놓기는 셸이 그 탭을 다시 찾아 처리한다 — 다른 창으로 갔으면 그 창에서 빼 오고 · 닫혔으면 무동작(정본 = §3-3 「놓을 때 다시 찾기」 · ADR-0297 결정 2). 다른 창으로 옮기거나 닫은 경우 출처 dragend 가 오지 않아 우리 웹뷰 밖 놓기는 무동작이다(§9-2).

### 3-7 모델 쪽 변경 (작게)

- ★**ADR-0297(2026-10-20) — U1 이 엄격 거절용으로 지은 셋(`DockGuard.expect_window` · `TargetExpect::SeamAt` · 판 번호 `layout_rev`)을 `tab_drop` 은 쓰지 않는다.**★ 놓을 때 다시 찾기(§3-3)로 바뀌었다(놓기 = `DockGuard::NONE`). **`DockGuard.expect_window` · `TargetExpect::SeamAt` = 걷었다**(ADR-0297 반영 · U3). **`layout_rev`(셸 `layout_revs` · `edit_layout` 의 적기 · `ViewManager::layout_rev` · 스냅숏 칸 `ViewSnapshot.layout_rev`) = 남음 — 소비자 없음 · 걷는 단위 = U6**(§7). 웹 쪽 짝 `CachedView.layoutRev` 는 걷어 `viewStore` 가 HEAD 그대로다. 되살리지 말 것.
- **(남음 · U6 이 걷음) 뷰별 판 번호 `layout_rev`** — `ViewManager` 가 뷰마다 「그 뷰 레이아웃(트리 · 비율 · 슬롯 내용)을 마지막으로 바꾼 때의 전역 `version`」을 쥔다(영속 안 함 · 뷰를 만들거나 들일 때 지금 `version`). 레이아웃 쓰기 지점은 `ViewManager` 의 비공개 `edit_layout` 하나로 모았다(재리뷰 2026-10-20 · 비차단 권고). 포커스 · 이름만 바뀌면 오르지 않는다. 스냅숏 `ViewSnapshot.layout_rev` 가 싣지만 읽는 쪽이 없다(웹은 쥐지 않는다). 영속 · `PartialEq` 인 `View` 구조체에는 넣지 않았다. 옛 쓰임 = `SeamAt.rev` 대조(판정과 놓기 사이에 내용만 바뀌어도 `TargetChanged` 로 거절하는 보수적인 쪽이었다). 쓰기 지점 시험(`manager.rs` `each_layout_write_moves_the_layout_rev_and_nothing_else_does` 따위) · 스냅숏 시험(`layout_apply` `the_announced_snapshot_carries_the_layout_rev`)도 U6 까지 남는다.
- 6.3 이 끌기 세션용으로 더한 칸 · 옛 엔진 대조는 U6 에서 걷는다: `DockDest::AdoptWindow` · `OnEmptied::Keep` · `keep_lease` · 표식 발급 · `WindowSeed` · `FrameMeasures` 되살림 · 옛 대조 `DockGuard.expect_source` · `TargetExpect::{Seam, Tabs}` · `DockError::{SourceChanged, TargetChanged}` · 위 `layout_rev` 와 그 스냅숏 칸. **창 항목 `chrome` · `set_dock_chrome` · `apply::report_dock_chrome` 은 U4c 에서 `layout/chrome.rs` 와 함께 걷는다**(`DockChrome` 이 그 파일에 산다).

### 3-8 창 사실 밀기 — `window:frame` (R7 · R22 의 재료)

- **웹이 지금은 갖지 않는다** — `WindowTabsPayload` 에는 탭 · 활성 · version 뿐이다. 모델 `WindowAttrs` 에는 `bounds`(복원 — 물리 바깥 위치 + 논리 안쪽 크기)와 `maximized` 가 있다.
- **더하는 것** — 셸 → 그 창 사건 `window:frame { restored: Option<SizeCss> /* 논리 안쪽 */, maximized: bool, insets: Insets /* 바깥 − 안쪽 · 논리 px · t · r · b · l — 기존 타입 */ }` + 마운트 때 당기는 동기 명령 `window_frame()`(웹뷰 안 배선 · 버스 명령 아님). 옛 엔진의 대기 창(예비 · 끄는 중 비워 남긴 창 — U4 까지)은 셸 창 모델 밖이라 당기지 않는다(셸이 그 창을 몰라 실패한다 · `useTabDragSource` 의 `pullFrame`).
- **값이 바뀔 때만 낸다** — 자리 기록 경로(`Moved` · `Resized` · `record_placement`)에서 그 창의 값을 새로 셈해 마지막으로 낸 값과 다를 때만 낸다(창별 마지막 값 = 셸의 작은 표 · 잎 락). 제목 표시줄로 창을 옮기는 동안의 `Moved` 연타는 값이 같아 내지 않는다. 셈 · 비교는 락 안에서, emit 은 락을 놓은 뒤에 한다.
- **틀 inset 은 셸이 셈한다** — 웹에서 `outer* − inner*` 로 셈하지 않는다(WebView2 에서 그 값이 창 틀과 맞는지 미검). 셸은 Tauri 게터(`outer_position` · `inner_position` · `outer_size` · `inner_size`) ÷ 배율로 셈하고, §3-5 `inset_dip` 도 같은 함수를 쓴다.
- **불변식과 맞나** — 둘 다 끌기의 판정 · 상태가 아니다. 끌기는 창을 움직이지 않고, 끄는 동안 다른 까닭으로 나거나 당겨져도 막지 않는다(ADR-0297 결정 1). 웹은 창마다 마지막 값만 쥔다.

## 4. 웹 모듈

### 4-1 파일

| 파일 | 맡는 것 | 순수? · 시험 |
|---|---|---|
| `tabStrip/useTabDragSource.ts`(새) | 머리 `draggable` 조건 · dragstart(실음 · 크기 · 모양 · 그림 · 끊기) · dragend(`NewWindow{by: DragEnd{effect}}` · 자기 `dropEffect` 적음 · 같은 웹뷰 제자리 놓기 · 엔진이 접은 끌기면 보내지 않고 곧바로 풂) · `done` 받기 · 원래 선택 되돌리기(§4-6) — 정하는 것은 `tabStrip/dragSource.ts` · 이 훅은 머리 사건을 값으로 바꿔 넘기고 구독을 걸고 뗀다 | 가짜 DragEvent · 가짜 invoke · 가짜 사건 |
| `tabStrip/dragSource.ts`(새 · U3) | 출처 웹뷰의 끌기 하나를 끝까지 모는 한 곳 — 바꿔 그리기 풀기 · 원래 선택 되돌리기(`switch_tab_if`) · dragend 의 보내기(제자리 놓기 · 엔진이 접은 끌기면 보내지 않음) · 되돌리기 뒤 활성 탭 누름의 `forceSwitch` · 자기 탭바 순서 이동 모습(Chrome 순서 문턱). 웹뷰마다 하나 · DOM 모름 | 가짜 포트 · fast-check 순서(§8-3) |
| `tabStrip/appIdentifier.ts`(새 · U3) | 끌기 자료 열쇠 — Tauri identifier(소문자)를 웹뷰마다 묻고 쥔다 · 답 전에 시작한 끌기는 열지 않는다 | 가짜 묻기 |
| `dock/dragImage.ts`(새 · U3) | 끌기 그림 = 탭 이름 라벨(화면 밖에 붙여 `setDragImage` · 다음 task 에 뗌 · 실패 = 엔진 기본 그림) | jsdom |
| `dock/useWindowDrop.ts`(새) | 창 뿌리 dragenter · dragover · dragleave · drop → `judge` → 탭바 모습 · 세로선 · 미리보기 · 바꿔 그리기 · 명령 — 정하는 것은 `dock/windowDrop.ts` · 이 훅은 문서 사건과 포트(스토어 · 탭바 기하 · Tauri)를 잇는다 | 가짜 DragEvent |
| `dock/windowDrop.ts`(새 · U3) | 창 하나의 받는 쪽 정하는 곳 — `judge` 판정 → 안내 · 세로선 · 출처 자기 탭바 판정 · drop 의 명령 하나(마지막 판정 기준 · 같은 창 같은 자리인지는 그때 스토어로 가려 맞으면 보내지 않음). 회차 = dragenter 가 열고 drop · 창 떠남 · 시한이 닫음(시한은 건 회차를 쥐고 울려 그 회차일 때만 닫는다) · 끄는 동안 캔버스 크기가 바뀌면 다시 재고 마지막 점으로 다시 판정 · DOM 모름 | 가짜 포트 · fast-check 순서(§8-3) |
| `dock/localDrag.ts`(새 · U3) | 웹뷰마다 하나인 훅 사이 seam — 나간 끌기 · 출처 자기 탭바 판정 · 세로선 · 탭바 기하 · 제자리 놓기 표시(칸마다 쓰는 쪽 하나 · 창 사이로는 아무것도 건너지 않음) | 표 |
| `dock/judge.ts`(새) | 점 + 창 맥락 → `Strip{index} \| Band{..} \| Elsewhere` · 차례 = 탭바 구역 > 바깥 띠 > 칸 띠(R6a) · 유효한 띠만 | 순수 · 표 |
| `dock/bands.ts`(새) | 띠 · 고르기 · 붙는 몫 · 미리보기 · 칸 구역 선 · 탭바 구역 빼기 · **유효성(R8)** — 셸 `layout/bands.rs` 의 거울(§4-5). 상수 = 생성물 `src-tauri/bindings/DockConstants.ts` 의 `DOCK_CONSTANTS`(이 표의 거울 파일 넷 공통) | 순수 · 골든(양쪽) |
| `dock/dockedTree.ts`(새) | 셸 `layout::dock::{docked_tree, incoming_rect}` · `layout::geometry::{compute, px_edge}` 의 거울 — 유효성 · 미리보기가 쓴다 | 순수 · 골든(양쪽) |
| `dock/dragSize.ts`(새) | R7 원래 크기 · R22 새 창 크기(Chrome `dragged_size` — 셸 `layout/drag_size.rs` 의 거울) · `grab`(§4-2) | 순수 · 골든 + 표 |
| `dock/tabDragData.ts`(새) | MIME 타입 문자열(크기 · drag id · 끄는 탭 모양) · 값 짓기 · 읽기 | 순수 · 표 |
| `dock/dragOverride.ts`(새) | 출처 창 바꿔 그리기 상태 · 풀기 리듀서(§2-4 — 시한 없음 · 계기 넷) | 순수 · fast-check |
| `dock/tabDrop.ts` · `dock/windowFrame.ts`(새) | `tab_drop` 감싸개 + `tab_drop:done` 듣기 · `window:frame` 받기와 마운트 당기기 | 얇음 |
| `dock/overlayTypes.ts`(새 · U2 가 세우고 U3 가 더함) | TS 고유 타입 — 바인딩 `RectCss` 를 잇지 않는다(§5-4). U2 = `RectCss`(바인딩과 같은 모양) · `SlotZones`(구역 선) — `bands.ts` · `judge.ts` · `dock/testing/bandsGolden.ts` 가 쓴다. U3 = `OverlayView`(HEAD 바인딩 모양 + 구역 선 칸 — 구역 선은 HEAD 에 없어 묶음 2 O 를 백업 패치에서 다시 싣는다. U4b 까지 옛 안내 창 페이지도 이 타입으로 받으므로 옛 셸 판이 보내지 않는 구역 선 칸은 없어도 되는 칸이다) | 타입만 |
| `tabStrip/reorderGeometry.ts`(남김 + 더함) | Chrome 순서 이동 함수(`draggedLeft` · `insertionIndex` · `reorderThreshold` · `moveDue` · `slideTargets` · `slideMs` · `easeOut`)는 남긴다(W13). 다른 창용 `dropIndex` · `dropMarkerX` 를 더한다. 끌기 문턱 함수 · 상수는 걷는다(Chromium 문턱) | 순수 · 표 |
| `tabStrip/succession.ts`(새) | 셸 `layout::manager::succession` 과 같은 규칙 | 순수 · 셸과 같은 표 |
| `dock/DockOverlay.tsx`(고침) | 안내를 **대상 창 캔버스 위 층**에 그린다 · 타입 = `overlayTypes.ts` | 렌더 |
| `TabBar.tsx` · `WindowLayout.tsx`(고침) | 위 훅 연결 · transform 그리기 · 세로선 · 바꿔 그리기 반영 · 탭 모양(미커밋 묶음 6) · B1 폭 처리는 남긴다 | 컴포넌트 |
| `dock/fileDropGuard.ts` · `tabStrip/{layout,closingMode,measure}.ts` | 그대로(`measure.ts` 는 `frameMeasureOf` 만 걷음 — `dragAreaOf` 는 탭바 구역에 쓴다) | — |

### 4-2 끌기 출처 — `useTabDragSource`

- **`draggable` 조건** — 이름 편집 중이 아니다 · `+` 와 빈 탭바 곳이 아니다 · **main 의 탭이 하나뿐이면 아니다**(Q35).
- **dragstart**
  - **`e.stopPropagation()` — 우리 끌기에 한해 뿌리의 dragenter · dragover · drop 도 끊는다.** 에이전트 트리(react-arborist → react-dnd)의 HTML5 백엔드가 `window` 거품 단계 리스너로, 등록된 소스가 받지 않은 dragstart 를 `preventDefault` 해 끌기를 죽이고(`handleTopDragStart` 「If by this time no drag source reacted, tell browser not to drag」) 자기 것이 아닌 dragover 의 `dropEffect` 를 `none` 으로 덮는다(`handleTopDragOver`) — `react-dnd-html5-backend` 14.1.0 `dist/esm/HTML5BackendImpl.js` 코드 확인 · 스파이크 창에는 트리 칸이 없었다. 우리 것이 아닌 끌기(파일 · 트리 행)는 끊지 않는다(`fileDropGuard` 머리와 같은 이유).
  - **실음** — 타입 문자열 = `application/x-engram-tab;app=<identifier>;drag=<uuid>;w=<정수>;h=<정수>;shape=<모양>` · 값 = JSON `{v:1, drag, view, from, initial, size, grab}`.
    - **왜 타입 문자열인가** — dragover 동안 `getData` 가 막혀 대상이 볼 수 있는 것은 `types` 뿐인데, 미리보기(R4)에 원래 크기가 · 띠 유효성(R8)에 끄는 탭 모양이 그때 필요하다. **거부한 것** = 셸 경유(불변식 위반) · 웹 ↔ 웹 `BroadcastChannel`(배달이 비동기라 dragenter 와 경주하고 창 사이 통신을 하나 더 세운다). 타입 문자열 칸이 창 사이 OLE 를 못 건너면(M3) §9-1.
    - **`shape`** = 끄는 탭 레이아웃의 모양만(슬롯 id · 내용 없음) — 전위 순 `l`(칸) · `h<비율>_` · `v<비율>_`(좌우 · 위아래 분할, 뒤에 두 자식). 비율 = JS `String(n)`(f64 왕복 정확 · 소문자 · `;` `=` 없음). 칸 하나 = `l`. 대상은 이것으로 그 탭을 끼운 결과의 면적 0 을 판정한다(§4-5 · 슬롯 id 는 전위 순서로 지어 붙인다). 길이 ≈ 분할 수 × 20 자 — 상한은 미검(M3).
    - `app` = Tauri `getIdentifier()`(마운트 때 한 번 · 실패는 유계 재시도 `retryAsync` — ADR-0102 · 답 전에 시작한 끌기는 우리 것으로 읽지 않는다 · `tabStrip/appIdentifier.ts`) — 다른 판(개발 · 배포 · 스파이크)의 끌기를 받지 않는다. Chromium 이 타입을 소문자로 바꾸므로 소문자로 싣고 비교한다.
  - **크기**(`dragSize.ts` · 재료 = `window:frame`(복원 크기 · 최대화 · 틀 inset — 셸이 셈) · `screen.availWidth/Height`(작업 영역 · DIP) · 뷰포트 − 캔버스(크롬))
    - **탭이 둘 이상인 창** — 새 창 안쪽 크기 `size` = Chrome `dragged_size` 를 안쪽 크기로 옮긴 것(PRD R22 · 6.3 §5-5 C5 차례): s = 복원 크기(없으면 지금) → 작업 영역(− 틀)을 넘는 변은 작업 영역 − 20 → 최대화면 각 변 ≥ 최대화 안쪽 크기 ÷ 2(마지막이라 이긴다). 원래 크기(R7 — `w` · `h`) = `size` − 크롬.
    - **탭 하나뿐인 팝아웃** — `size = null`(그 창을 옮김) · 원래 크기 = (최대화면 복원 크기 · 아니면 지금 크기) − 크롬.
    - 크기 칸이 없거나 깨졌으면 대상은 원래 길이 = 칸 절반(상한)으로 친다.
  - **그림**(R29 · VS Code `applyDragImage` · 조사 F) — 탭 이름 라벨 · 안쪽 여백 1 px 7 px · 둥글기 10 px · 글자 12 px · 최대 폭 120 px · 말줄임 · 색 = 테마 토큰. 화면 밖에 붙이고 `setDragImage(el, −10, −10)` 뒤 다음 틱에 지운다.
- **끄는 동안 출처 창**(R2 · R15 · R23) — 커서가 **자기 탭바 구역 안**이면 Chrome 식 순서 이동이다(끄는 머리가 커서를 가로로 따르고 이웃은 200 ms EASE_OUT 로 비켜남 · `reorderGeometry` 그대로 · transform 만). 구역 밖 · 창 밖이면 이동 모습을 거두고(200 ms 되돌아감) **남은 탭을 보인다** — `initial ≠ view ? initial : succession(tabs, view)` · 탭 하나뿐이면 끄는 탭 그대로. 남은 탭은 모델을 바꾸지 않는 웹 표시이고 셸이 놓을 때 고르는 `successor` 와 같다 — 같은 창의 다른 탭에 이음새로 붙는 유일한 길이다(S12). 구역에 돌아오면 끄는 탭을 다시 보이고 이동이 이어진다.
- **dragend** — **`dropEffect` 와 무관하게(`none` · `move` 모두)** `tab_drop{NewWindow{by: DragEnd{effect}, size, grab}}`(§3-5)를 보낸다(재리뷰 2026-10-20 · 사용자 결정 Q39 — 다른 앱이 받아도 새 창). **단 같은 웹뷰 제자리 놓기 · 엔진이 접은 끌기(dragstart 와 같은 task 의 dragend — §9-3 U2 ⑨)는 보내지 않는다**(사용자 결정 2026-10-20 「드래그 스타트 안보내는거면 엔드도 보내지마」 · 아래 · §2-4 ④). `effect` = 자기 `dropEffect` 가 `move` 면 `move` · 그 밖은 `none`(재리뷰 3차 — 셸이 `move` 를 보고 우리 창 · 못 읽음을 무시로 가른다 · §3-5 ③ · ⑤). 응답은 로그만 남기고 결과는 `done` 으로 받는다(invoke 오류면 바꿔 그리기를 곧바로 푼다). 자기 `dropEffect` 는 적어 둔다 — `done{Ignored|Canceled}` 를 읽을 때 쓴다(§4-6). 같은 웹뷰의 drop 이 아무것도 보내지 않았으면(같은 창 같은 자리 — §4-3) `DragEnd` 를 보내지 않고 바꿔 그리기를 풀고 transform 을 곧바로 거둔다(§2-4 ④).
  - **옛 `copy` · `link` 갈래는 걷었다** — `effectAllowed = 'move'` 라 남이 받아도 dragend 는 `move` 뿐이어서 죽은 길이었다. 다른 앱이 받은 `move` 는 셸이 커서 밑 맨 위 창(`Foreign`)으로 새 창을 고른다(`move` 라는 사실은 이제 셸에 실어 보낸다 — 우리 웹뷰가 받은 `move` 와 가르는 것은 맨 위 창뿐이다).
  - **대가** — 우리 웹뷰가 받은 끌기도 끝난 뒤 `DragEnd` invoke 를 하나 더 낸다(받은 창의 `Drop` 과 함께 둘 · 같은 창 같은 자리 놓기는 둘 다 0 · 놓기 쓰기(`dock_tab`)는 많아야 한 번 — 장부 §3-4). 끌기가 끝난 뒤의 호출이라 불변식은 그대로다.

### 4-3 판정 — `judge` (모든 우리 창의 뿌리 dragover)

- **받기** — 우리 끌기(`app` 일치)면 늘 받는다(`preventDefault` · `dropEffect = 'move'` · `stopPropagation`). 그래서 우리 웹뷰 위에서 떼면 drop 이 늘 온다. 받는 곳이 없으면 OLE 는 Drop 대신 DragLeave 를 줘 「창을 떠남」과 가를 수 없다(가능성 높음 · MS `DoDragDrop` · HTML 끝 처리) — 그것을 피한다(잃은 놓기 — §9-2).
- **탭바 구역**(R2) = 탭바 판정 영역의 가로 범위(`measure.ts` `dragAreaOf` — 첫 탭 왼쪽 ~ `+` 앞) × 탭바 위아래 15 px.
  - **출처 창** — `insertionIndex(B, d, draggedLeft(..))` · `moveDue` 로 순서 이동 모습(R23).
  - **다른 창** — 2 px 세로선(W13 · VS Code). `dropIndex(B, x, null)` = 보이는 머리 중 가운데 < `x` 인 것의 수(엄격한 `<` — `x` 가 어느 머리의 가운데와 같으면 그 머리 왼쪽 자리) · 보이는 마지막 머리 다음 자리를 넘지 않음(R24 — 6.3 `insertionIndex` ⑤) · 빈 곳 = 끝. 선 = 그 자리 왼쪽 머리 오른쪽 끝 + 오른쪽 머리 왼쪽 끝(VS Code `::after` · `::before` 꼴 · `--accent`).
- **캔버스** — `bands` 로 **유효한** 띠를 고른다(§4-5) · 띠가 있으면 미리보기 · 없으면 칠하지 않음(R6 · R8).
- **그 밖**(칸 가운데 · 캔버스 밖 우리 UI) → `Elsewhere`(drop 이면 `NewWindow{by: Drop}` — R6 · R22).
- **drop** — **처리기 첫 문장 = `e.preventDefault()`**(다음이 `stopPropagation`) — 그래야 출처 dragend 가 `move` 다. 출처는 그 끌기에도 `DragEnd` 를 보내지만(§4-2), `move` 를 보고 그 `DragEnd` 의 무시로 풀지도 되돌리지도 않는다(§4-6 — 결과는 이 창의 `Drop` 이 정한다 · 장부 §3-4). 그 뒤 **마지막 판정(미리보기)이 가리킨 대상**으로 보낸다 — 놓은 순간 좌표로 다시 판정하지 않는다(사용자 결정 2026-10-20 「유저는 미리보기로 판단하는거잖아」). **유일한 폴백** = 열린 미리보기 회차가 없을 때(300 ms 무소식 · mousemove 시한이 닫음 — §2-4) 온 drop 은 그 drop 점에서 판정한다(버리지 않는다). 갈래: 탭바 → `TabBar{index}`(같은 창 같은 자리면 보내지 않고 그 drag id 를 그 웹뷰에 적어 둔다 — 뒤이은 출처 dragend 가 읽고 `DragEnd` 도 보내지 않고 곧바로 푼다 · §2-4 ④) · 띠 → `Seam{target: 보이는 탭, anchor, side, share}` · `Elsewhere` → `NewWindow{by: Drop, size, grab}`(값 = 실음 JSON). 판정 때 상태(탭 목록 · 판 번호 · 활성)는 싣지 않는다 — 셸이 놓을 때 대상을 다시 찾는다(ADR-0297 결정 2 · §3-3). **응답은 로그만 남긴다** — 결과는 출처가 `done` 으로 받는다(§4-6). `Duplicate` 는 놓기를 잃었다는 뜻이 아니다 — 같은 끌기의 `DragEnd` 새 창이 진행 중이면 장부가 맡아 둔 이 `Drop` 이 뒤에 적용될 수 있다(§3-4).
- **끄는 탭 자신이 보이는 캔버스**(탭 하나뿐인 출처 창) — 띠가 없다(`SelfTarget` · VS Code 「출처 칸이 비게 되면 덮개 없음」 꼴). 그 위에서 떼면 `Elsewhere` → `Reuse` 로 그 창이 끈 만큼 옮겨진다(R6 · Q34).
- **그대로인 것** — 탭 폭 · 숨김 · 넘친 탭바(R24) · 닫기 모드(R26 — 미연결 TODO U7c 그대로) · 탭바 스크롤 없음. 머물러 열기 · 창 올리기는 없다(Q36).

### 4-4 그리기 — 탭바 · 안내

- **자기 탭바** — 끄는 동안은 `transform: translateX` 만 쓴다(Firefox `drag-and-drop.js` 꼴). DOM 순서 · key 는 바꾸지 않고 끄는 머리는 떼지 않는다. 순서는 놓을 때 한 번 모델로 바뀌고, 모델 알림이 오는 프레임에 transform 을 0 으로 거둔다. 진짜 출처 요소를 옮겨도 OS 끌기가 끊기지 않는지는 미검이다 — 선례 = Chromium 옛 WebUI 탭바가 dragover 동안 진짜 요소를 DOM 에서 옮겼다(`drag_manager.ts` `placeTabElement`) · M12 가 잰다.
- **다른 창 탭바** — 2 px 세로선만 · 탭은 움직이지 않는다(W13).
- **안내**(`DockOverlay`) — 대상 창 캔버스 위 `position: absolute; inset: 0; pointer-events: none` 층에 `DockOverlayView({ guide })` 를 그대로 쓴다(좌표 = 캔버스 CSS px). 그리는 것(사용자 결정 2026-10-19 · 미커밋 묶음 2 — HEAD 에 없어 U2(Rust) · U3(웹)가 백업 패치에서 다시 싣는다) = 커서 칸의 구역 선(가운데 사각형 + 대각선 넷) · 바깥 띠에 들면 레이아웃 틀 · 고른 띠의 붙을 사각형(옅은 채움 + 얇은 테두리). 움직임은 VS Code(나타남 · 사라짐 150 ms 페이드 · 자리 옮김 70 ms) · 크기는 ADR-0295 결정 4(VS Code 의 반 칸 · 온 칸과 다름). 안내 창(`dock-overlay`) · `dock:overlay` 는 없어진다.

### 4-5 도킹 띠 — 정본 = 셸 `layout/bands.rs` · 거울 = 웹 `bands.ts` · `dockedTree.ts`

- **왜 둘인가** — 끄는 동안 끌기는 판정을 셸과 맞추지 않으므로(ADR-0296 · ADR-0297) dragover 마다 셸에 묻지 않고 웹이 판정한다. 셸 판은 LLM 명령(T-65 — 붙는 몫 · 새 창 크기의 기본값)과 놓을 때 검증이 쓰므로 남긴다. `landing_rect` 가 기대는 `layout::dock::{docked_tree, incoming_rect}` · `layout::geometry::{compute, px_edge}` 는 이미 `layout/` 에 산다.
- **옮김(U2)** — 옛 `dock/geometry.rs`(`effective_bands` · `bands_at` · `choose` · `band_frame` · `slot_band` · `slot_centre` · `outer_strips` · `frames` · `frame_at` · `landing_rect` · `subtract` · `zone_in_canvas` · `outer_sides`) · `dock/sizing.rs` 의 `attach_share` · `dock/strip.rs` 의 `strip_zone` · `contains` · 입력 구조 `WindowTargets`(옛 `targets.rs`) · 그 시험을 새 `src-tauri/src/layout/bands.rs` 로, `sizing.rs` 의 `restored_size` · `dragged_size` 와 그 시험을 새 `layout/drag_size.rs` 로 옮긴다. 옛 자리는 `pub use` 로 다시 내보내 옛 엔진이 그대로 컴파일된다(U4c 가 그 껍데기와 함께 지운다). 끌기 세션 전용(`detached_*` · `strip_width_at` · `whole_window_origin` · `client_point` · 모니터 고르기 · `original_size`)은 옮기지 않는다.
- **상수 — 한 곳(Rust)에서 정하고 TS 는 생성물을 읽는다**(메인 결정 2026-10-20) — 함수는 두 벌(아래 골든이 묶음)이지만 숫자는 한 벌이다. `layout/bands.rs` 머리에 `pub const` 로 둔다: `BAND_SHARE`(0.10 — 칸 띠 = 칸 길이의 10%) · `BAND_MIN_PX`(12) · `OUTER_BAND_PX`(20) · `STRIP_ZONE_PX`(15 — 탭바 구역 위아래 · 옛 `strip::DETACH_MAGNETISM_PX` 를 옮기며 이름을 바꿈) · `ATTACH_SHARE_MAX`(0.5 — 붙는 몫 상한 · 지금 `attach_share` 안의 맨 숫자) · `MAXIMIZED_INSET_DIP`(10 — 새 창 크기의 작업 영역 여백 한쪽) · `SIDE_ORDER`(동률 차례). Rust 기하는 이 이름만 쓰고 몸 안에 같은 숫자를 다시 적지 않는다(`OUTER_BAND_PX > STRIP_ZONE_PX` 컴파일 단언은 그대로).
  - **생성 길 — 파일 하나**(리뷰 2026-10-20 · 메인 결정) — 내보내기 시험 `export_bindings_dock_constants` 하나가 `src-tauri/bindings/DockConstants.ts` 를 쓴다. 담는 것 = `export const DOCK_CONSTANTS = { … } as const;` 와 `export type DockConstants = typeof DOCK_CONSTANTS;` 두 줄(+ 생성물 머리 주석). 값은 `serde_json` 으로 적어 결정적이다(Rust 쪽 값 묶음은 `Serialize` 만 단 시험 전용 구조체). **ts-rs 내보내기는 달지 않는다** — `DockConstants` 라는 이름의 `TS` derive 를 두지 않는다. 시험은 `layout/bands.rs` 의 `#[cfg(test)]` 에 두고, 이름이 `export_bindings_` 로 시작해 셸 `lib_unit` 이 돌 때마다 쓰며, CI 생성물 동기 게이트(`src-tauri/bindings/` 포함)가 커밋본과의 어긋남을 잡는다.
  - **규칙 — 생성 바인딩 파일 이름은 대소문자만 다른 둘이 있으면 안 된다.** NTFS 는 대소문자를 가리지 않고 이 저장소는 `core.ignorecase=true` 이며 CI 러너도 Windows 라, 그런 둘(옛 안의 `DockConstants.ts` 타입 + `dockConstants.ts` 값)은 서로를 덮어쓴다. 이름은 다른 바인딩과 같이 PascalCase 다. 시험 = §8-1 ⑪(git 색인 겹침 · 내용 서명).
  - **TS** — `bands.ts` · `dockedTree.ts` · `dragSize.ts` · `judge.ts` 는 `src-tauri/bindings/DockConstants.ts` 의 `DOCK_CONSTANTS`(타입이 필요하면 같은 파일의 `DockConstants`)를 import 한다. 띠 · 구역 · 상한 · 여백 숫자를 TS 에 손으로 적지 않는다(리뷰 렌즈). 분할 비율 한계(`RATIO_MIN` · `RATIO_MAX`)는 지금처럼 스냅숏 `ratio_min` · `ratio_max` 로 받는다(ADR-0227 — 길을 둘로 만들지 않는다). 웹에만 있는 값(미리보기 시한 300 ms · 페이드 150 ms · 순서 이동 200 ms · 표준 탭 폭)은 공유 상수가 아니라 웹에 남는다.
  - **골든과 묶음** — 골든 머리 `_constants` 에 뜰 때의 상수를 적고, Rust 시험은 그것이 지금 `pub const` 와, TS 시험은 그것이 `DOCK_CONSTANTS` 와 같은지 본다 — 상수를 바꾸고 골든을 다시 뜨지 않으면 양쪽이 빨개진다.
- **골든 — 양쪽이 먹는다**(선례 `crates/engram-dashboard-agent/src/queued_input_golden.json` · ADR-0231 꼴) — `src-tauri/src/layout/bands_golden.json` 하나. U2 가 Rust 로 뜨고(생성 시험은 `#[ignore]`) 그 뒤로 Rust 시험(`include_str!`)과 TS 시험(`dock/testing/bandsGolden.ts` — 경로에서 `?raw` · 사본 금지)이 둘 다 단언한다. 한쪽 규칙만 고치면 다른 쪽이 빨개진다. 담는 것 = 점 격자 × 캔버스 × 배치 × 원래 크기 × 끄는 탭 모양 × 탭바 구역 → 유효한 띠 · 고른 띠 · 몫 · 미리보기 · 구역 선(U2 가 묶음 2 O 의 Rust 몫을 다시 실은 뒤 뜬다 — §7) · 틀(막힌 띠 · 막힌 바깥 위 띠 밑 칸 띠 포함) + 새 창 크기 표.
- **입력** — 보이는 탭의 레이아웃(칸 틀도 이것으로 센다 — 셸 `frames` 처럼 `computeSlots` → `framePx` · 스냅숏 `slotRects` 는 받지 않아 레이아웃과 어긋날 길이 없다) · `ratioMin` · `ratioMax`(ADR-0227) · 캔버스 크기(dragenter 때 `getBoundingClientRect` 를 `Math.round` 해 캐시 · 회차 동안 크기가 바뀌면 다시 잰다 — 아래 「값싸게」) · 원래 크기 · 끄는 탭 모양(타입 문자열) · 탭바 구역(캔버스 좌표로).
- **규칙**(6.3 §5-3 · §5-4 · 미커밋 묶음 4 · 사용자 결정 2026-10-19 · CSS px · 반열림 정수 사각형) — 칸 틀 = `round(캔버스 × 정규화 경계)`(셸 `px_edge`) · 칸 띠 두께 = `max(round(0.1 × 칸 길이), 12)`(띠 틀 안으로 자름) · 띠 틀 = 칸 틀 − 그 칸이 닿은 캔버스 변의 바깥 띠 몫(`band_frame`) · 바깥 띠 = 네 변 20 px(탭바 구역 15 보다 커야 바깥 위 띠가 5 px 남는다) · 띠 영역 = 띠 − 탭바 구역(− 유효 바깥 띠) · 고르기 = 바깥 띠 > 칸 띠, 같은 차례 안에서는 「변까지 거리 ÷ 띠 두께」 최소, 같으면 `Left → Right → Top → Bottom`.
- **VS Code 와 다른 곳** — VS Code 는 띠 안 방향을 칸의 1/3 · 2/3 세로 경계(좌 · 우)와 1/2 가로 경계(위 · 아래)로 가른다(조사 F · EDT #402-503). 우리는 가장 가까운 변(대각선 — 2026-10-19 안내 선)이다. PRD R6a 가 정했으므로 그대로 둔다(§9-5).
- **붙는 몫**(R7) = `clamp(min(L, T/2) / T, ratioMin, 0.5)` — `L` = 원래 크기의 나누는 방향 길이 · `T` = 붙는 칸 틀(바깥 띠면 캔버스)의 같은 방향 길이. 예: 원래 400 → 칸 600 = 300 · 칸 1200 = 400 · 칸 50 = 25(AC2 · AC39 · AC44).
- **미리보기 사각형**(R4) = 기준(칸 또는 캔버스) 정규화 사각형을 `boundary(lo, hi, ratio)` 로 나눈 새것 쪽(`ratio` = Left · Top 이면 몫 · Right · Bottom 이면 `1 − 몫` · 셸 `clamp_ratio` 꼴로 `[ratioMin, ratioMax]`) → `round(캔버스 × e)`. 셸 `landing_rect`(`docked_tree` → `compute` → `incoming_rect`)와 비트 단위로 같다 — 끼운 서브트리 상자 = 분할 자식 상자(`layout/geometry.rs` 머리 불변식). 골든이 잰다.
- **유효성(R8 · AC36) — 웹이 한다** — 띠마다 「그 자리에 끄는 탭을 끼운 결과에 새로 면적 0 인 칸이 생기나」를 `dockedTree` 로 판정한다(셸 `band_hit` 이 `landing_rect` 의 `None` 으로 띠를 빼는 것과 같은 답). 걸린 띠는 칠하지도 고르지도 않는다 — **칠하기와 놓기 고르기가 같은 걸러진 목록 하나를 쓴다.** 막힌 바깥 띠는 칸 띠에서 빼는 몫에도 들지 않아, 그 밑 칸 띠가 그 자리에서 칠해지고 골린다(셸 `collect` 와 같은 차례 — AC36 마지막). 끄는 탭 모양이 없거나 깨졌으면 칸 하나로 친다 — 셸이 놓을 때 `ZeroArea` 로 다시 막는다.
- **값싸게** — 칸 틀 · 띠 · 유효성은 보이는 탭 · 그 스냅숏 캐시 · 캔버스 크기 · 탭바 구역이 바뀔 때만 셈하고, dragover 마다는 탭바 기하 읽기와 점 고르기만 한다(S1 처리기 ≤ 0.5 ms). 캔버스는 dragenter 와 **회차 동안의 크기 바뀜**(회차마다 ResizeObserver 하나 — `useWindowDrop` 의 `watchCanvas`)에만 다시 잰다 — 바뀌면 다시 재고 **마지막 판정 점으로 곧바로 다시 판정**해, dragover 를 기다리지 않고 미리보기가 새 크기를 따른다(끄는 동안 창 크기 바뀜을 막지 않으므로 — ADR-0297 「영향」).

### 4-6 끝 뒤 — 결과 알림 · 원래 선택 되돌리기 (출처)

- 출처는 `tab_drop:done` 중 자기 drag id 의 것만 본다(그 끌기의 dragstart 부터 다음 dragstart 까지). 모든 갈래의 결과가 이 한 길로 온다 — 받은 창이 보낸 놓기의 결과도 출처는 이것으로 안다. 출처가 dragend 마다 `DragEnd` 를 보내므로(§4-2) `done` 은 보통 늘 온다 — 같은 창 같은 자리 놓기 · 엔진이 접은 끌기는 `DragEnd` 를 보내지 않아 `done` 이 없다(§2-4 ④).
- **바꿔 그리기 풀기** — §2-4.
- **자기 dragend 가 `move` 인데 `done{Ignored}` 또는 `done{Canceled}`** — 받은 곳이 있었고 맨 위 창이 우리 창이거나 못 읽었다(§3-5 ③ · ⑤) = 보통 우리 웹뷰가 받았다. 그 창의 `Drop` 결과가 뒤따른다(`Drop` 이 이김 — §3-4 · 같은 창 같은 자리 놓기는 출처 dragend 도 보내지 않아 이 `done` 이 없다 — §2-4 ④). **그 `done` 으로는 풀지도 되돌리지도 않는다** — 풀면 `Drop` 적용 전 한두 프레임 끄는 탭이 보이고(§8-3 ②), 되돌리면 보내지 않은 제자리 놓기가 누른 탭 대신 `initial` 을 보인다(클릭과 다름 — 지금은 그 놓기가 `DragEnd` 도 보내지 않아 이 `done` 이 오지 않는다). 뒤따르는 `done` 또는 그물 계기(§2-4)가 푼다 — 끝 상태는 이기는 `Drop` 이 정하고, 그 `Drop` 을 잃거나 그 `Drop` 이 무시되면(`no-cursor`) §2-4 ② · ③ 그물이 푼다. `Canceled` 는 지금 `decide` 로는 `move` 에 나지 않지만(`move` 는 취소 줄을 건너뜀 — §3-5 ②) 오면 `Ignored` 와 같다(재리뷰 3차). `Unknown` 인데 다른 앱이 받았으면 `Drop` 이 오지 않는다 — 누른 탭이 활성으로 남고(클릭과 같음) 그물이 푼다(§9-2). (재리뷰 2026-10-20 의 F3 를 맞추며 더함)
- **원래 선택 되돌리기**(R12 · R15 · AC19 · AC25) — `outcome` 이 `Canceled` · `Ignored`(위 `move` 갈래 밖 — 곧 자기 dragend 가 `none`)면 출처가 `switch_tab_if(initial, expect = 끄는 탭)` 을 한 번 보낸다. 누름이 `switch_tab` 으로 모델 활성을 이미 바꿨기 때문이다(R2). 건너뜀 = `initial` 없음 · `initial = view` · `initial` 이 그 창 탭이 아님 · **그 창 스토어 활성이 이미 `view` 가 아님**(그사이 사용자 · LLM 이 바꿈 — 덮지 않는다). **스토어는 모델보다 늦을 수 있어 셸이 한 번 더 본다**(리뷰 2026-10-20) — `switch_tab_if` 는 레이아웃 락 안에서 그 창 활성이 아직 `expect` 일 때만 바꾸고(알림은 `switch_tab` 과 같다), 아니면 아무것도 바꾸지도 알리지도 않고 `false` 다. 그래서 되돌리기를 낸 뒤 닿기 전에 사용자 클릭 · LLM `tab.switch` 가 고른 탭을 덮지 않는다. **단 그 사이 누른 것이 끄던 탭 자신이면** 스토어 활성이 이미 그 탭이라 보통 클릭은 `switch_tab` 을 내지 않고, 뒤에 닿은 되돌리기가 그 클릭을 덮는다 — 그래서 되돌리기가 날아가는 동안 그 탭을 누르면 출처가 명시 `switch_tab` 을 보낸다(`DragSource.forceSwitch`). Tauri 동기 명령은 메인 스레드에서 차례대로 돌므로 그것이 되돌리기 뒤에 적용돼 클릭이 이긴다. 버스 명령이 아니다(§3-6). 끌기가 끝난 뒤의 호출이라 불변식을 어기지 않는다. `Rejected` 는 되돌리지 않는다 — 놓기를 했고 탭은 제자리라 누른 탭이 활성으로 남는다(클릭과 같음).
- **되돌리기 오류** — `switch_tab_if` 가 오류를 돌려줄 수 있다(되돌릴 `initial` 이 그 사이 닫힘 등 — 정상 경주). 부르는 쪽은 잡아 로그만 남긴다(덮개는 모델 활성으로 물러난다).
- 같은 drag 에 `done` 이 둘(`DragEnd` 무시 → `Drop` 적용 — §3-4)이면 뒤의 것이 이긴다. 자기 dragend 가 `move` 면 앞의 무시로 되돌리지 않으므로 보통은 되돌리기가 `Drop` 보다 앞서지 않는다. 앞선 경우(dragend 는 `none` 인데 `Drop` 이 옴 — 같은 사건 두 번 배달 따위)에도 **끝 모델은 같다** — 다른 창 대상은 `Drop` 적용의 `successor` 가 `initial` 이고(§3-3), 같은 창 이음새 · 탭바는 놓을 때 활성을 대조하지 않으므로 되돌리기가 먼저 닿아도 거절되지 않는다(ADR-0297 결정 2 · §3-3 — 옛 판은 이음새에서 `SeamAt.or_initial` 로 이것을 따로 받았다 · `Reveal` 로 옮긴 탭이 활성).

### 4-7 끄는 동안 터미널 크기 — 미루지 않는다(ADR-0297)

- **걷었다**(ADR-0297 결정 1 · 사용자 결정 2026-10-20 「상관없어 그냥 예외 최대한 적게 넣으면 됨. 관행에 최대한 맞춰서」) — 끄는 동안 터미널 크기 맞춤(`resizePty` — ResizeObserver · 가시성 두 길)은 평소대로 흐른다. 끌기의 판정 · 상태가 아니므로 「끄는 동안 끌기 명령 0」(§2-2) 밖이다. `dock/dragActive.ts`(끌기 살아 있음 표시)와 `TerminalSlot` 의 `sendSize` 미루기(U3 조각 A)는 짓지 않는다 — §4-1 표 · §7 U3 · §8-2 에서 뺐다. 창 틀 조회(`window_frame`) · 캔버스 크기 보고의 미루기도 짓지 않는다. 끌기 밖의 50 ms 디바운스 규칙(CLAUDE.md 통합 micro-rules)은 그대로다.
- **거부한 대안 = 미룸**(옛 판 · U3 조각 A) — 끌기와 무관한 호출이 생길 때마다 예외를 더해야 한다(U3 리뷰가 그 틀 밖의 창 틀 조회 · 캔버스 크기 보고를 잡은 것이 실증 · 사용자 「모든 예외사항들을 추가해야되서」). 관행에도 없다 — 끄는 동안 IPC · 저장을 막는 피어를 못 찾았다(조사 `docs/research/drag-external-events-practice-2026-10-20.md` 결론 3). 되살리지 말 것. 옛 판 서술(`resizePty` 의 두 길 · 표시를 끄는 그물 · 꺼질 때 다시 재기 · 거부한 대안 (a)–(d))은 `git show 642c69bd:docs/process/S21-slot-docking/trd.md` 의 이 절(옛 제목 「끄는 동안 터미널 크기 미루기 — `dragActive` · `TerminalSlot`」)에 있다.
- **그대로 거부 — 「마지막으로 보낸 cols · rows 와 같으면 건너뜀」**(옛 판 거부한 대안 (a)) — 창마다 따로 된 JS 맥락이라 다른 창이 같은 에이전트의 PTY 크기를 바꿨을 수 있어, 건너뛰면 필요한 크기 맞춤을 잃는다. 끌기와 무관한 창 여럿의 일반 사실이라 미룸을 걷어도 유효하다.

## 5. 걷는 것 / 남기는 것

줄 수 = 작업 트리 `wc -l`(미커밋 포함). 「일부」의 걷힘 몫은 함수 범위로 어림했다 — 착수 단위가 diff 로 다시 잰다.

### 5-1 platform (U5)

| 파일 | 줄 | 처리 | 근거(호출부 `git grep` — platform 밖) |
|---|---|---|---|
| `src/move_loop.rs` | 564 | 걷음 | 부르는 곳 = `commands/drag*.rs` · `dock/*` 뿐 |
| `src/move_loop/windows.rs` | 1,565 | 걷음 | 〃(미커밋 묶음 7 멈춤 수정 · 8 빠른 끌기 맞춤 포함) |
| `src/window.rs` | 1,370 | **일부** — 걷음 ≈ 500 · 남김 ≈ 870 | 걷음: `place_window` · `ZOrder` · `hide_window` · `is_window_visible` · `move_window` · `set_switcher_hidden` · `describe_window` · `WindowFacts` · `top_level_at` 의 `skip` 인자(건너뛸 우리 창 — 예비 · 안내 · 끄는 창 — 이 없다) · 그 시험 — 끌기 · 예비 · 안내 창만 부른다. 남김: `PhysPoint` · `PhysRect` · `NativeWindow` · `WindowError` · `restore_placement`(미커밋 첫 화면 묶음 `state/placement.rs` 가 부른다) · `foreground_is_current_process`(`state/placement.rs`) · **`top_level_at` 과 그 비쳐 보이는 창 판정(`hit_accepts` · `LayeredKind` · 창 모양 · DWM 가림 — 미커밋 B2 · §6) — dragend 판정(§3-5)이 부른다** · 그 시험 |
| `src/input.rs` | 81 | 남김 + `escape_down()` 더함(U1) | `tab_drop` 이 부른다 |
| `src/test_window.rs` | 107 | 남김 | `restore_placement` · `top_level_at` 실창 시험이 쓴다 |
| `src/lib.rs` · `Cargo.toml` | 221 · — | 고침 | 머리 입주자 목록(`move_loop` 빼기 · `window` 설명 줄임). 기능 `Win32_UI_Shell`(이동 루프 서브클래스)은 남은 쓰임을 `rg` 로 본 뒤 뺀다. `Win32_Graphics_Dwm` 은 남는다(`top_level_at` 의 가림 판정 — Esc 전환 쓰임만 빠진다). `Gdi`(창 모양 · 자리 되돌리기) · `KeyboardAndMouse` · `WindowsAndMessaging` · `raw-window-handle` 도 남는다 |

- **OS 게이트** — 4f(OS cfg) · 4h(`std::os`) 명단에 끌기 파일이 없다(`ci.yml` 확인). 명단은 고칠 것이 없다. 짝(platform 안 1 파일 이상)과 4g(OS crate 운영 의존 = platform 하나)도 그대로 선다. 고칠 것은 platform `src/lib.rs` 머리 글(`window` 입주자 설명 — `top_level_at` 은 남음) · CLAUDE.md 의 platform `window` 설명(「커서 밑 맨 위 창 `top_level_at` — 탭 끌기가 그 위에 선다」 → 놓기 끝 판정) · `windows` 의존 「탭 끌기 쪽」 문단이다(U7).

### 5-2 셸 (`src-tauri`)

| 파일 | 줄 | 처리 |
|---|---|---|
| `src/dock/` 전부 — `session.rs` 1,544 · `session/tests.rs` 2,173 · `exec.rs` 682 · `ports.rs` 397 · `parked.rs` 365 · `world.rs` 218 · `targets.rs` 817 · `strip.rs` 114 · `units.rs` 232 · `wire.rs` 353 · `geometry.rs` 1,301 · `sizing.rs` 746 · `mod.rs` 253 | **9,195** | **옮김 ≈ 1,800(U2 → `layout/bands.rs` · `drag_size.rs` — §4-5) · 걷음 ≈ 7,400(U4c)**. 옮김 = `geometry.rs` 거의 전부 · `sizing.rs` 의 `attach_share` · `restored_size` · `dragged_size` 와 그 시험 · `strip.rs` 의 `strip_zone` · `contains` · `targets.rs` 의 입력 구조 |
| `src/commands/drag.rs` · `drag_window.rs` | 681 · 721 | 걷음(U4c) |
| `src/commands/dock.rs` | 324 | 걷음(U4c) — `report_dock_chrome` · `window_role` · 예비 · 안내 창 짓기 · 명부 사건 · `is_overlay_label`. `tab_drop` · `window_frame` 은 새 `commands/tab_drop.rs` 에 산다 |
| `src/commands/settings.rs` | — | 고침(U4c) — 테마 칠하기의 안내 창 건너뛰기(`is_overlay_label`) 갈래 |
| `src/layout/chrome.rs` | 68 | 걷음(U4c — `PointCss` · `RectCss` 는 `layout/bands.rs` 로 옮김 · `RectCss` 의 ts 내보내기는 뗌) |
| `src/layout/manager.rs` · `apply.rs` | 6,972 · 1,389 | 고침 — U1(지은 기록): `expect_window` · `layout_rev` · `SeamAt`. U3(ADR-0297): `expect_window` · `SeamAt` 걷음 · 놓기 다시 찾기 · `DockMove.successor_window` 더함. U4c: 창 항목 `chrome` · `set_dock_chrome` · `apply::report_dock_chrome`. U6: §3-7 세션용 칸 · 옛 엔진 대조(`expect_source` · `TargetExpect::{Seam, Tabs}` · `DockError::{SourceChanged, TargetChanged}`) · `layout_rev`(줄 수 미측정 — 착수 때 잰다) |
| `src/layout/types.rs` · `dock.rs` | — | 고침(U1) — `DockSide` 의 `TS` derive · `ViewSnapshot.layout_rev`(U1 이 더함 · 소비자 없음 · U6 이 걷음) |
| `src/lib.rs` | — | 고침 — U1: `tab_drop` · `window_frame` 등록 · `DropBook` manage · `window:frame` 내기(자리 기록 갈래 안 · 바뀔 때만). U4a: 예비 · 안내 창 짓기 · 끌기 실행기 `install` 끄기. U4c: 옛 명령 · `Destroyed` 갈래의 끌기 호출 셋(`on_parked_window_destroyed` · `on_overlay_destroyed` · `drag::on_window_gone`) · `Moved` 갈래의 `drag::on_window_moved` 와 안내 · 대기 창 술어 — **`placement::record` 와 `window:frame` 은 남는다**(`lib.rs:392-401`) · 명부 건너뜀 · 종료 표시 |
| `src/commands/{popout,layout}.rs` · `src/tray/actions.rs` | 920 · 643 · 436 | 고침(U4c) — `build_spare_window` · `record_parked` 갈래 · 트레이 `parked` 술어 걷기(파일은 남음) |
| `src/commands/first_paint.rs` · `src/first_paint.rs` | 10 · 233 | 남김 — 첫 화면(끌기 무관 · 미커밋 묶음 5) |
| `capabilities/dock-overlay.json` | 10 | 걷음(U4c) |
| 새 `src/layout/{bands,drag_size}.rs` + `bands_golden.json` · 생성물 `bindings/DockConstants.ts`(값 + 타입 한 파일) | 옮김 ≈ 1,800 + 골든 · 상수 내보내기 | U2 |
| 새 `src/tab_drop/{mod,plan,wire}.rs` · `src/commands/tab_drop.rs` | ≈ 450(시험 포함 · 어림) | U1. U3(ADR-0297): `tab_drop/{mod,plan,wire}.rs` 의 사전조건 걷음 · 시험 다시 씀(`commands/tab_drop.rs` 는 주석만) |

### 5-3 웹 (`src`)

| 파일 | 줄 | 처리 |
|---|---|---|
| `tabStrip/useTabDrag.ts` + 시험 | 849 + 1,058 | 걷음(U4b) |
| `dock/dockSession.ts` + 시험 | 176 + 330 | 걷음(U4b) — `dock:drag` · `dock:feed` · `dock:spare` · `window_role` · 대기 모드 |
| `dock/frameReport.ts` + 시험 | 94 + 116 | 걷음(U3 에서 호출 끊음 · U4b 에서 파일 — 그 전에 `DragAreaMeasure` 를 `measure.ts` 로 옮김) |
| `pages/DockOverlayPage.tsx` + 시험 · `App.tsx` 의 `#/dock-overlay` 경로 | 53 + 74 · — | 걷음(U4b) |
| `tabStrip/measure.ts` + 시험 | 87 + 107 | **일부** — `frameMeasureOf` · `FrameMeasure` 와 그 시험 걷음(≈ 30). `dragAreaOf` · B1 폭 처리 남김 |
| `tabStrip/reorderGeometry.ts` + 시험 | 229 + 421 | 남김(Chrome 순서 이동 — W13) · 문턱 몫(≈ 20) 걷음 · `dropIndex` · `dropMarkerX` 더함 |
| `WindowLayout.tsx` + 시험 · `TabBar.tsx` + 시험 | 484 + 1,459 · 599 + 744 | 고침 — 대기 모드 · 틀 보고 · 옛 훅 연결 걷음 · 새 훅 · transform · 세로선 |
| `src/components/slot/TerminalSlot.tsx` + 시험 | 406 + — | 그대로 — 끌기 중 미루기는 짓지 않는다(ADR-0297 · §4-7) |
| `tabStrip/{closingMode,layout}.ts` + 시험 · `dock/fileDropGuard.ts` + 시험 · `dock/DockOverlay.tsx` + 시험 · `dock/dockDiag.ts` · `util/latestValueReporter.ts`(+ 시험 — 캔버스 보고가 쓴다) | 1,180 + 328 | 남김(`DockOverlay` 는 타입 · 자리 · 움직임만 고침) |

### 5-4 생성 바인딩 · 권한

- `src-tauri/bindings/` 의 끌기 타입 **20 개(203 줄 — U0 뒤 = HEAD 판)** 를 걷는다: `BeginReply` · `DockChrome` · `EndHow` · `EndSrc` · `FeedStep` · `Grab` · `LeaveReply` · `OverlayView` · `RectCss` · `SpareEvent` · `SpareStep` · `TabDrag{Begin,End,Event,Feed,Leave,Phase,Reorder}` · `WindowRole` · `WindowRoleReply`. 작업 트리의 미커밋 `SlotZones.ts`(새 · 묶음 2 O)와 고친 `FeedStep` · `Grab` · `OverlayView` · `TabDragLeave` 는 ④ 라 U0 이 되돌린다(옛 판의 「21 개 · 234 줄」은 그 작업 트리를 센 값이다). `PointCss` 는 남는다. `RectCss` 는 Rust 타입이 `layout/bands.rs` 에 남지만 ts 내보내기를 뗀다 — 웹은 `overlayTypes.ts` 의 TS 고유 타입을 쓰고 두 판은 골든이 묶는다. U2 가 다시 싣는 구역 선의 Rust 타입 `SlotZones` 도 같은 처리다 — ts 내보내기를 달지 않는다(웹 = `overlayTypes.ts`).
- ts-rs 는 Rust 타입이 사라져도 옛 `.ts` 를 지우지 않는다. **U4c 가 손으로 지운다.** 웹 import 는 그 전에 U4b 가 끊는다(U4b 끝 조건 — §7).
- 새 바인딩(U1): `TabDrop` · `DropTo` · `NewWindowBy` · `EndEffect` · `DropAnchor` · `TabDropReply` · `TabDropDone` · `DropOutcome` · `SizeCss` · `WindowFrame` · `DockSide`(새 derive). 고침: `ViewSnapshot`(`layout_rev` — 소비자 없이 U6 까지 남는다 · §3-7). 그대로 씀: `Insets`.
- **ADR-0297 로 다시 구움** — `DropTo` 에서 판정 때 상태 칸 `TabBar.expect` · `Seam.rev` · `Seam.active` 를 뺐다(남는 짐 = `TabBar{index}` · `Seam{target, anchor, side, share}` · `NewWindow{…}`). `TabDrop` · `TabDropReply` · `ViewSnapshot` 은 문서 주석만 바뀌었다(`from` = 결과 알림을 받는 창 · `initial` 은 끄는 탭이 놓을 때 `from` 에 있을 때만 · 거절 낱말 예에서 `SourceChanged` · `TargetChanged` 를 뺌).
- 새 생성물(U2 · §4-5 「상수」): `src-tauri/bindings/DockConstants.ts` **하나**(값 `DOCK_CONSTANTS` + 타입 `DockConstants = typeof DOCK_CONSTANTS` — 내보내기 시험이 씀 · ts-rs 아님). 커밋하고 CI 생성물 동기 게이트가 덮는다. 대소문자만 다른 두 파일로 가르지 않는다(§4-5 규칙).

### 5-5 합계

| | 걷음 | 비고 |
|---|---|---|
| platform | ≈ 2,630(통째 2,129 + `window.rs` ≈ 500) | |
| 셸 | ≈ 9,450(`dock/` ≈ 7,400 + 1,726 + 68 + 10 · 배선 ≈ 250) | U6 미포함 · `dock/` 의 ≈ 1,800 은 `layout/` 로 옮겨 남는다 |
| 웹 | ≈ 3,000(통째 2,750 + 일부 ≈ 50 · 바인딩 203) | 바인딩 = U0 뒤 HEAD 판(§5-4) |
| **계** | **≈ 15,100** | 시험 포함 · 미커밋 포함. ADR-0296 의 「약 1 만」은 손으로 만든 엔진 파일만 센 값이다 — 이 표는 시험 · 바인딩까지 센다 |
| 새로 | ≈ 450(Rust) + ≈ 1,600(TS — 거울 `dockedTree` 포함 · 시험 · 골든 제외) | 어림 |

## 6. 지금 미커밋 변경의 처리

작업 트리 = HEAD `1c5ad58b` 위에 쌓인 셋이다 — 앞 세션의 리뷰 전 묶음 8건(`.claude/handoff/history/20261019-193211-…` 「미커밋 묶음」 1–8) · B1/B2 수정(리뷰 2 라운드 · QA standard PASS) · 문서. 「팝업으로 분리」 미완성 10 파일은 이미 되돌려졌다(`layout/{manager,apply,mod}.rs` · `dock/{parked,mod}.rs` 의 `git diff` 가 빔 — 줄끝만 다름).

| 갈래 | 파일 | 처리 |
|---|---|---|
| **① 문서만** | `.claude/handoff/latest.md` · `docs/decisions/0295-*` · `docs/decisions/README.md` · `docs/process/S21-slot-docking/{prd.md, structure.html, trd.md}` · `docs/process/S21-window-chrome/prd.md` · `docs/refactoring/{README.md, slot-docking-drag-code-audit-2026-10-18.md}` · `docs/research/{child-process-shutdown-peers, custom-titlebar-tauri, slot-docking-chrome-desktop-drag, slot-docking-chrome-tab-drag, window-naming-titlebar}-*.md` · `docs/tracking.md` · 추적 안 됨 = `docs/decisions/0296-*` · `docs/research/slot-docking-drag-ownership-peers-2026-10-19.md` | `/review doc` 뒤 커밋. `structure.html` 은 7판에 맞춰 다시 그린다 |
| **② 살림 — 끌기 무관 · B2**(묶음 5 첫 화면 · 테마 · 6 탭 모양 토큰 · B2 맨 위 창 판정) | `src-tauri/src/{first_paint.rs(새), commands/first_paint.rs(새), commands/mod.rs, commands/settings.rs, commands/popout.rs, theme.rs, webview_env.rs, state/placement.rs, state/restore.rs, lib.rs(첫 화면 몫)}` · `src/{App.tsx, App.test.tsx, main.tsx, index.css, i18n/ko.ts, store/agentStore.ts, store/themeStore.ts, styles/theme.css, theme/ThemeManager.ts, theme/uiSettings.ts(+시험), theme/{bootTheme,firstPaint,themeMemory}.ts(+시험 · 새), components/agent/AgentList.tsx(+시험 · Reparent 시험), components/slot/PresetPalette.tsx(+시험)}` · platform `src/window.rs`(B2) · `src/lib.rs`(머리의 `window` 줄) · **아래 의존 hunk** | 리뷰 · QA 뒤 커밋. 묶음 6 이 기대는 ADR-0168 결정 5 부분 폐기는 박았다(사용자 결정 2026-10-20 · §9-3) |
| **③ 섞임 — 파일은 살고 일부 몫이 죽음** | `TabBar.tsx`(+시험 — 탭 모양 · B1 폭 · Z 「탭바 영역 = `+` 앞까지」= 살림 / L 떼어 낸 창 탭 x = 죽음) · `WindowLayout.tsx`(+시험 — 첫 화면 알림 = 살림 / 틀 재보고 = 죽음) · `tabStrip/measure.ts`(+시험 — B1 · Z 끌기 영역 = 살림) · `tabStrip/reorderGeometry.ts`(+시험 — Z 맞물림 = 살림) · `dock/DockOverlay.tsx`(+시험 — 안내 모양 O = 살림) · `dock/dockDiag.ts`(추적 안 됨 — B1 진단 = 살림) | ② 가 기대는 hunk(아래)는 ② 와 함께 커밋 · 나머지는 HEAD 로 되돌리고 U3 가 백업 패치로 이어받는다 |
| **④ 죽음 — 걷힐 코드** | platform `move_loop.rs` · `move_loop/windows.rs`(묶음 7 · 8) · 셸 `commands/{dock,drag,drag_window}.rs` · `dock/{geometry,ports,session,session/tests,sizing,strip,targets,wire}.rs`(묶음 1 L · 2 O · 3 Z · 4 가장자리 띠) · 바인딩 `FeedStep` · `Grab` · `OverlayView` · `SlotZones`(새) · `TabDragLeave` · 웹 `useTabDrag.ts`(+시험) · `dockSession.test.ts` · `frameReport.ts`(+시험) · `pages/DockOverlayPage.tsx`(+시험) | 리뷰 · 커밋하지 않는다. 백업 패치로 남긴다. 사용자가 정한 **값**(바깥 띠 20 · 안내 모양 · 칸 띠를 칸 크기에 비례 · 탭바 영역 `+` 앞까지)은 §4 가 이어받는다 — U2 가 옮기는 Rust 기하(§4-5)는 HEAD 판에서 출발하므로 묶음 4 의 띠 규칙을 백업 패치에서 다시 싣는다 |
| **⑤ 쓰레기로 보임** | `ff.json`(추적 안 됨 · nginx 301 HTML 178 바이트) | 사용자가 지운다(§9-3) |

**처리 = (B) 갈라 커밋**(사용자 결정 2026-10-20 · 세부는 메인) — 트리 전체를 백업 패치로 뜨고(뜬 것 = `.claude/handoff/attachments/worktree-full-2026-10-20.patch` · `GIT_INDEX_FILE` 임시 인덱스 — 핸드오프 do-not 「`git add -N .` 금지」 · 추적 안 된 코드 파일 포함), ② 와 아래 의존 hunk 를 임시 인덱스에 hunk 단위로 담아 리뷰 · QA 뒤 커밋한다. 나머지는 HEAD 로 되돌리고, ③ 의 살릴 모양 · 폭 · Z 규칙은 U3 지시서에 백업 패치를 근거로 넘긴다. 고르지 않은 (A) 스냅숏 커밋 = 규약 「커밋은 게이트 통과 후에만」과 부딪치고 그래프에 걷힐 코드가 한 번 실린다.

**② ↔ 되돌리는 쪽의 의존 — 함께 가야 초록**(2026-10-20 diff 독해 · U0 착수 때 HEAD + ② 트리에서 lib_unit · platform · `npm test` · `npx tsc --noEmit` 으로 다시 잰다)

1. **첫 화면** — 셸 `first_paint.rs` 의 숨김(`hold_window` — `commands/popout.rs` · `state/placement.rs`)은 웹 `reportFirstPaint` 알림에 창을 보인다. 그 알림은 `WindowLayout.tsx` 첫 화면 hunk(import `:47` · 효과 `:178` · `activeLaidOut` · `firstScreen`)에서만 나간다 → 그 hunk(+ `WindowLayout.test.tsx` 의 짝)를 ② 와 함께 커밋한다. 빠지면 창이 셸 시한에야 보인다. 그 hunk 는 HEAD 의 `framed` · `bootFailed` · `selectView` 위에 그대로 얹힌다(U4b 가 대기 모드를 걷을 때 `firstScreen` 에서 `framed` 를 뺀다).
2. **로딩 문구** — ② `i18n/ko.ts` 가 `window.loading` 을 지우는데 HEAD `WindowLayout.tsx:245` 가 그것을 쓴다 → 그 자리의 바꿈(LoadingPanel)이 1 의 hunk 와 함께 간다.
3. **탭 모양 토큰** — ② `styles/theme.css` 가 `--tab-inactive-bg` 를 지우는데 HEAD `TabBar.tsx:238` 이 그것을 쓴다 → `TabBar.tsx` 의 묶음 6 탭 모양 hunk(+ 시험)를 theme.css 와 함께 커밋한다(빠지면 빌드는 서지만 비활성 탭 바탕이 빈다). 묶음 6 은 ADR-0168 결정 5 부분 폐기(by ADR-0295 — 위 모서리 · 구분선 · `--tab-inactive-bg` · 사용자 결정 2026-10-20 「크롬처럼 둥글게」)와 한 몸이고, 그 도장을 박아 U0 를 막지 않는다(§9-3).
4. **안내 창 테마** — ② `App.test.tsx` 의 `#/dock-overlay` 시험 hunk(`installUiSettings` 1 회 단언)와 `App.tsx` 머리 주석 hunk 는 ④ `DockOverlayPage.tsx` 의 테마 따르기를 전제한다 → 둘을 ④ 와 함께 되돌린다(안내 창은 U4a 에서 없어진다). `commands/settings.rs` 의 안내 창 건너뛰기는 HEAD `commands/dock.rs:135` 의 `is_overlay_label` 을 불러 컴파일이 서므로 ② 에 남기고 U4c 가 걷는다.
5. **맨 위 창 판정** — platform `window.rs` 의 B2 몫(`hit_accepts` · `LayeredKind` · 창 모양 · 그 시험)은 이제 dragend 판정(§3-5)이 쓴다 → ② 로 옮겨 커밋한다(옛 표의 ④ 에서 뺐다 · platform 안에서 닫혀 있다). 같이 실리는 `describe_window` · `WindowFacts` 는 부르는 곳(④ 셸 dock 코드)이 되돌려져 부를 곳이 없고, U5 가 걷는다.
6. **의존 없음을 본 것** — `state/placement.rs` 의 `restore_placement` 호출(HEAD 시그니처와 같다) · `commands/mod.rs`(`first_paint` 등록뿐) · 셸 `lib.rs` diff(첫 화면 몫뿐) · `theme/firstPaint.ts`(끌기 import 없음 — `commands::dock` · `dock::ports` 언급은 주석뿐).
7. **웹뷰 바탕 인자** — ② `webview_env.rs` 가 `WebviewEnv::finish` 에 둘째 인자 `Backdrop` 을 더하는데, HEAD ④ `commands/dock.rs` 의 `build_overlay`(안내 창 짓기 — U4a 가 걷는다)가 인자 하나로 부른다 → 그 파일의 두 hunk(`use crate::webview_env::{Backdrop, WebviewEnv}` · `Backdrop::Unpainted` 인자)를 ② 와 함께 커밋한다(U0 실측 2026-10-20 — 옛 판 6 이 `webview_env.rs` 를 「의존 없음」으로 잘못 적었다).

## 7. 이행 순서

**어느 단위 뒤에 멈춰도 빌드 · 시험이 초록이게 짰다** — 새 길을 옛 길 옆에 먼저 세우고, 웹이 옮겨 탄 뒤에야 옛 길을 걷는다. 걷기는 셸이 창을 덜 만드는 것부터 한다(U4a). 웹이 먼저 대기 모드를 걷으면, 셸이 아직 짓는 예비 창이 탭 0 개로 스스로 닫히고 셸이 다시 짓는 고리에 빠지기 때문이다. 타입은 쓰는 쪽을 먼저 옮기고 원래 자리를 나중에 지운다.

단위마다 `/review code` → `/qa`(U3 는 full) → 커밋한다. **옛 HTML5 끌기 구현을 최대한 가져다 쓴다**(사용자 2026-10-20 — 「기존에 os 먼저하고 나중에 개선했잖아. 그런것들 코드 최대한 참고해서 하면 될듯」) — 7판과 뜻이 같은 곳은 옮기고(시험 포함) 다른 곳만 7판대로 쓴다. 참조점(`git show <rev>:<path>` 로 읽는다): 웹 = `78a946c4^` 의 `src/components/layout/dock/{dockDrag,dockPick,dragImage,dockChromeReport}.ts`(+시험 · 처음 들인 커밋 `bb693264` 는 `TabBar.tsx` · `WindowLayout.tsx` · `DockOverlay.tsx` 연결도 담는다) · 셸 = `9b6860bd^` 의 `src-tauri/src/dock/{book,hit,regions,types}.rs` · `commands/dock.rs`(`dock_drag_drop` · `dock_drag_end`) · `4b12978a^` 의 `dock/pick_golden.json`. 코더 지시서에는 「중간 어디서 멈춰도 빌드가 서게 순서를 짜라」를 넣는다.

| 단위 | 파일 | 시험 | 끝 조건 |
|---|---|---|---|
| **U0** 미커밋 정리 — (B) | §6 · 백업 패치 `.claude/handoff/attachments/worktree-full-2026-10-20.patch`(트리 전체 · 이미 떠 둠 — U2 · U3 도 이것을 근거로 다시 싣는다) | lib_unit · platform · `npm test` · tsc | ② + 의존 hunk 커밋 · 나머지 HEAD · 트리에 문서만 남음 · 초록 |
| **U1** 셸 명령 · 창 사실 · 판 번호(옛 엔진 옆에 더함 · `52de2625` — 이 줄은 지은 기록이다. U3(ADR-0297)에서 `expect_window` · `SeamAt` 을 걷었고 `layout_rev` 는 U6 이 걷는다 — §3-7) | 새 `src-tauri/src/tab_drop/{mod,plan,wire}.rs` · 새 `commands/tab_drop.rs`(`tab_drop` · `window_frame`) · `commands/mod.rs` · `lib.rs` · 자리 기록 경로의 `window:frame`(바뀔 때만 · 틀 inset) · `layout/manager.rs`(`expect_window` · `layout_rev` · `SeamAt`) · `layout/types.rs`(`ViewSnapshot.layout_rev`) · `layout/dock.rs`(`DockSide` `TS` derive) · platform `input.rs`(`escape_down`) · 새 바인딩(§5-4) · **`ViewSnapshot` 픽스처**(필수 칸 `layout_rev` 가 늘어 tsc 가 깨지던 곳 — 칸이 U6 까지 남아 픽스처도 그대로다 · `src/components/layout/WindowLayout.test.tsx` · `src/components/layout/useSplitDrag.test.tsx` · `src/store/viewStore.test.ts` · 착수 때 `rg "ViewSnapshot" src -g "*.test.*"` 로 다시 본다) | lib_unit(§8-1 ①–⑥ · ⑨) · `layout_apply`(⑦) · platform input · 바인딩 동기 · tsc | 명령이 있으나 웹은 아직 안 부름 · 초록 |
| **U2** 기하 옮김 + 상수 한 곳 + 웹 순수 모듈 + 골든 | 셸(먼저): 옛 `dock/{geometry,sizing,strip}.rs` 의 남길 함수 · `WindowTargets` · 상수 → 새 `layout/{bands,drag_size}.rs`(옛 자리는 `pub use` · 묶음 4 띠 규칙과 **묶음 2 O 의 구역 선 Rust 몫**(칸의 가운데 사각형 + 대각선 넷 — HEAD 에는 없다 · `SlotZones` 는 ts 내보내기 없이 — §5-4)을 백업 패치에서 싣는다 — 옛 엔진 wire 타입(`OverlayView` · `FeedStep` · `Grab` · `TabDragLeave`)은 HEAD 그대로 — 묶음 2 O 에서는 구역 선 계산 함수만 싣는다 · `attach_share` 의 맨 0.5 → `ATTACH_SHARE_MAX`) · 상수 내보내기 시험 `export_bindings_dock_constants` → 생성물 `bindings/DockConstants.ts` 하나 커밋 · 새 `layout/bands_golden.json`(머리 `_constants`) + Rust 단언 · 생성(`#[ignore]`) 시험. 웹(그 뒤): 새 `dock/{overlayTypes,tabDragData,bands,dockedTree,judge,dragSize,dragOverride,tabDrop,windowFrame}.ts`(상수 = `DOCK_CONSTANTS` import · `overlayTypes` = `RectCss` · `SlotZones` 만 — §4-1) · `dock/testing/bandsGolden.ts` · `tabStrip/succession.ts` · `reorderGeometry.ts` 에 `dropIndex` · `dropMarkerX` · `fast-check` dev 의존 | lib_unit(§8-1 ⑩ · ⑪) · 바인딩 동기 · §8-2 · §8-3 · tsc | 아무 데도 안 붙음 · 생성물 동기 · 초록 |
| **U3** 전환 | 새 `tabStrip/useTabDragSource.ts` · 새 `dock/useWindowDrop.ts` · `dock/overlayTypes.ts` 넓힘(U2 가 세움 — `OverlayView` 를 더하고 `DockOverlay.tsx` 의 바인딩 import `OverlayView` · `RectCss` 를 그리로 · 구역 선 타입은 U2 의 `SlotZones` — HEAD 의 `DockOverlay.tsx` 에는 구역 선이 없어 묶음 2 O 의 웹 몫(`SlotLines`)을 백업 패치에서 다시 싣는다) · (`viewStore` 는 고치지 않는다 — 옛 판의 `CachedView.layoutRev` 는 ADR-0297 로 쓸 곳이 없다 · §3-7) · `TabBar.tsx`(transform · 세로선 · 출처 탭바의 순서 이동 모습은 `judge` 의 `index` 를 곧바로 그리지 않고 `moveDue` 가 참일 때만 확정한다 — `judge` 는 문턱을 보지 않는다 · 부르는 쪽 몫) · `WindowLayout.tsx`(뿌리 받기 · 안내 층 · 바꿔 그리기) · `DockOverlay.tsx` — `useTabDrag` 연결 끊기 · 틀 보고 호출 끊기. 대기 모드 · 안내 창 페이지는 **그대로 둔다**(셸이 아직 예비 · 안내 창을 짓는다) · **ADR-0297 셸 몫** — `src-tauri/src/tab_drop/{plan,mod,wire}.rs` · `src-tauri/src/commands/tab_drop.rs` · `src-tauri/src/layout/{manager,apply,types,bands}.rs` · `src-tauri/src/dock/{ports,session/tests}.rs` · `src-tauri/tests/layout_apply.rs` · 바인딩 `src-tauri/bindings/{DropTo,TabDrop,TabDropReply,ViewSnapshot}.ts` | 컴포넌트 시험(§8-2 — 옛 `dragActive` · `TerminalSlot` 미루기 시험은 ADR-0297 로 뺐다 · §4-7) · `lib_unit`(§8-1 ① · ⑦) · `layout_apply`(⑦) · 바인딩 동기 · `/qa full`(CDP §8-4 · C7) | 새 길로 끌림 · 옛 셸 엔진은 불리지 않는 채 남음 |
| — **사용자 실측** | §8-5 M1–M4 · M8 · M12 · M13 우선 | — | 정지 조건(§9-1)에 안 걸림 → U4 |
| **U4a** 셸이 창을 덜 짓는다 | `lib.rs` setup — `spawn_spare_build` · `spawn_overlay_build` · 끌기 실행기 `install` 끄기(명령 등록 · `window_role` 은 남김 — 웹 대기 모드가 아직 부른다) | lib_unit · 앱 기동 실측(창 수) | 예비 · 안내 창 없음 · 초록 |
| **U4b** 웹 옛 길 걷기 | **타입 먼저** — `measure.ts` 의 `RectCss`(바인딩) → `overlayTypes.ts` · `DragAreaMeasure` → `measure.ts` 로 옮김 · `FrameMeasure` 는 `frameMeasureOf` 와 함께 걷음. 그 뒤 `useTabDrag` · `dockSession` · `frameReport` · `DockOverlayPage` + 경로 · `WindowLayout` 대기 모드(`firstScreen` 에서 `framed` 를 뺌) · `reorderGeometry` 죽은 몫 | `npm test` · tsc | `window_role` · `tab_drag_*` · `report_dock_chrome` 를 부르는 웹 코드 0 · `rg "src-tauri/bindings/(BeginReply\|DockChrome\|EndHow\|EndSrc\|FeedStep\|Grab\|LeaveReply\|OverlayView\|RectCss\|SpareEvent\|SpareStep\|TabDrag[A-Za-z]+\|WindowRole)" src` 0 |
| **U4c** 셸 옛 엔진 걷기 | `src/dock/` 전부(옮긴 함수의 `pub use` 껍데기 포함) · `commands/{dock,drag,drag_window}.rs` · `layout/chrome.rs`(`PointCss` · `RectCss` → `layout/bands.rs`) + 창 항목 `chrome` · `set_dock_chrome` · `apply::report_dock_chrome` · `lib.rs`(옛 명령 · `Destroyed` · `Moved` 갈래의 끌기 호출만 — `placement::record` · `window:frame` 은 남김) · `commands/settings.rs` 의 `is_overlay_label` 갈래 · 트레이 · `popout` · `layout.rs` 명부 갈래 · `capabilities/dock-overlay.json` · 바인딩 20(손으로 — §5-4) | 워크스페이스 회귀 · 바인딩 동기 · 셸 타깃 여섯 · `npm test` · tsc | `rg "dock::(session\|exec\|parked)\|tab_drag_\|window_role\|is_overlay_label" src-tauri src` 0 |
| **U5** platform 걷기 | `move_loop.rs` · `move_loop/windows.rs` · `window.rs` 끌기 전용 함수(§5-1 — `top_level_at` 은 `skip` 인자만 빼고 셸 호출부를 맞춤) · 시험 · `lib.rs` 머리 · `Cargo.toml` 기능 | platform · `cargo build`(워크스페이스 — 셸 포함) · 셸 lib_unit · 4f · 4g · 4h · 의존 상한 게이트 | 게이트 PASS · 남은 `windows` 기능 쓰임을 `rg` 로 확인 |
| **U6** 모델 군살 | `layout/{manager,apply}.rs` · `tests/layout_apply.rs` · `tab_drop/mod.rs`(거절 낱말 시험 표의 `SourceChanged` · `TargetChanged` 줄) — §3-7 세션용 칸 · 옛 엔진 대조(`DockGuard.expect_source` · `TargetExpect::{Seam, Tabs}` · `DockError::{SourceChanged, TargetChanged}`) · **판 번호 `layout_rev`**(셸 `layout_revs` · `edit_layout` 의 적기 · `ViewManager::layout_rev` · `ViewSnapshot.layout_rev` 칸과 그 바인딩 · TS 픽스처 · 쓰기 지점 · 스냅숏 시험 — ADR-0297 로 소비자가 없다 · `layout/types.rs` · `src-tauri/bindings/ViewSnapshot.ts` 포함) | lib_unit · `layout_apply` · 바인딩 동기 · `npm test` · tsc | 남은 `dock_tab` 계약 시험 그대로 초록 · `rg "layout_rev\|layoutRev\|expect_source\|SourceChanged\|TargetChanged\|TargetExpect\|expect_target" src-tauri src` 0 · T-65 착수 전 |
| **U7** 문서 | CLAUDE.md(platform 입주자 「열」 → 아홉 · `window` 설명 · `windows` 의존 「탭 끌기 쪽」) · **남는 코드 · CLAUDE.md 의 `TRD S21-slot-docking §n` 앵커 ≈ 60 곳을 7판 절로 다시 맞춤**(7판이 번호를 다시 매겼다 — `rg "TRD S21-slot-docking §"`) · docs 의 옛 6.3판 포인터(tracking 등 — `rg "S21-slot-docking/trd.md"` docs) · PRD 4.9 대조(§9-5) · `structure.html` · step-log · tracking · 핸드오프 | `/review doc` | 남은 앵커가 7판 절을 가리킴 |

## 8. 테스트 계획

### 8-1 Rust 단위 (`lib_unit` · `layout_apply` · platform)

1. **`plan` 대응**(§3-3 표 전부) — `TabBar` 는 부른 창 label(인자 창 아님) · `successor` 는 `initial ≠ view` 일 때만 · `mv.successor_window = from` · 판정 때 상태를 대조하지 않음(`DockGuard::NONE` — 탭 목록 · 판 번호 · 활성 · 출처 창 대조 없음 — ADR-0297 결정 2) · `Slot{id}` → `DockAnchor::Slot(id)` · `size: None` → `WindowPlace.size: None` · 깨진 크기 → `None`.
2. **`decide`**(§3-5 표의 차례) — `Drop` × 커서 있음 · 없음 · `DragEnd` × `effect` 둘(`None` · `Move`) × (버튼, Esc) 네 조합 × `top` 셋(`Ours` · `Foreign` · `Unknown`). 앞선 줄이 이긴다. `None` + 둘 다 뗌 + `Ours` = `Ignore`(제목 표시줄 — Q38) · `None` + `Foreign` · `Unknown` = `Place` · **`Move` 표 — `Ours` = `Ignore("accepted-ours")` · `Unknown` = `Ignore("accepted-unknown")` · `Foreign` = `Place`(다른 앱이 받음 — Q39) · 커서 없음 + `Move` = `Ignore("accepted-unknown")`(⑤ 로 떨어짐) · 버튼 · Esc 눌림이어도 `Cancel` 아님**.
3. **`new_window_origin`** — 배율 1 · 1.25 · 2 · 틀 0 · (8, 31) DIP · 반올림 .5 · 탭 하나뿐인 팝아웃에서 「누른 점 = grab → 끈 만큼 이동」.
4. **`DropBook`** — 처음 것만 · 같은 id 의 `Drop` 둘 = `Duplicate` · `DragEnd` 취소 · 무시 뒤 `Drop` = 받음(기록이 `Drop` 으로 바뀜) · 새 창을 만든 `DragEnd` 뒤 `Drop` = `Duplicate` · **새 창이 거절된 `DragEnd`(`settle` · `close`) 뒤 `Drop` = 받음**(F6 · `done` = `Rejected` → `Applied`) · **새 창 진행 중에 닿은 `Drop` = 맡아 둠** → 거절이면 `DragEnd` 의 `done` 없이 이어 처리(`done` = 그 `Drop` 의 `Applied` 하나 · 그 `DragEnd` 의 info 줄에 `superseded`) · 서면 버림(`done` 없음) · **거절 알림 뒤 `close` 전에 닿은 `Drop` = 맡아 둠 → `close` 가 넘겨 이어 처리(`done` = `Rejected` → `Applied` 차례)** · `Drop` 뒤 `DragEnd` = `Duplicate` · 65 번째에 끝난 기록 중 가장 오래된 것 밀려남 — 진행 중(`Placing` · `Rejecting`)은 건너뜀 · 모두 진행 중이면 늘렸다가 끝난 뒤 상한으로 돌아옴 · 거절도 적음.
5. **거절 옮기기** — `DockError` 종류마다 `Rejected.kind` = 괄호 낱말 · 취소 · 무시는 모델 쓰기 0(가짜 포트).
6. **`window:frame`** — 값(복원 크기 · 최대화 · 틀 inset)이 바뀔 때만 내고 같은 값의 `Moved` 연타 · 최소화 중에는 안 냄 · emit 이 락 밖 · `window_frame` 당기기 = 모델 값 · 틀 inset 과 `inset_dip` 이 같은 함수.
7. **놓을 때 다시 찾기 · 경합**(`manager.rs` · `tests/layout_apply.rs` — ADR-0297 결정 2) — 끄는 탭이 없으면 무동작(거절 · version 그대로) · **`TabBar`: 자리가 지금 목록 길이를 넘으면 끝 자리 · 탭 목록이 바뀌어도 거절 없음** · **`Seam`: 대상 탭 · anchor 칸이 없으면 무동작 · anchor 칸이 그사이 나뉘었어도 남아 있으면 그 칸에 붙음 · 대상 뷰 레이아웃 · 그 창 활성이 바뀌어도 거절 없음(같은 창 이음새에서 되돌리기 `switch_tab_if(initial)` 가 먼저 닿은 경우 포함)** · **`switch_tab_if`: 활성 = `expect` 면 바꿈(알림 한 묶음) · 아니면 무변경 · 알림 · 재동기 없음 · 없는 창 · 남의 탭은 `switch_tab` 과 같은 오류** · 탭 하나뿐인 팝아웃 `NewWindow` → `Reuse` · 크기 유지(가짜 `WindowHost` 의 `move_window` 인자 `at.size = None`) · **대상이 그사이 다른 창으로 옮겨짐**(§3-3 메인 결정) — 주어진 `successor` 는 옮긴 탭이 `successor_window` 에 있을 때만 쓰임(`manager.rs` `the_given_successor_is_taken_only_while_the_tab_is_in_its_successor_window`) · 끄는 탭이 `from` 을 떠났으면 지금 창에서 빼 오고 그 창은 셸 `succession`(`layout_apply` `a_drop_whose_tab_left_its_source_window_moves_it_from_where_it_is_now` · `a_drop_whose_tab_left_its_source_window_leaves_that_window_to_the_shell_succession`) · 이음새 대상 탭이 다른 창으로 갔으면 그 창에 붙음(`layout_apply` `a_seam_drop_onto_a_tab_that_moved_to_another_window_docks_where_that_tab_is_now`) · 그때 포커스 · 응답 창 = 그 창(lib_unit `tab_drop` `a_seam_drop_whose_target_tab_moved_to_another_window_docks_there_and_focuses_that_window`).
8. **platform `escape_down`** — 누르지 않은 상태에서 거짓(CI 러너). 시험 정의는 OS 와 무관하게 컴파일된다(CLAUDE.md 「플랫폼 중립」).
9. **`tab_drop:done`** — 처리마다 많아야 한 번(맡은 `Drop` 에 자리를 내준 거절은 안 냄) · 출처 창(`from`)에만 · `Duplicate` 는 안 냄 · 적용이면 모델 알림 뒤 · 락 밖(가짜 사건 포트로 순서를 잰다).
10. **골든(Rust 쪽)** — `layout/bands.rs` · `drag_size.rs` 가 `bands_golden.json` 전부와 같다 · 골든 머리 `_constants` = 지금 `pub const` · **묶음 4 를 다시 실어 U3 까지 옛 엔진의 기하가 바뀐다**(바깥 띠 32 → 20 · 칸 띠를 띠 틀 위에) — 옛 엔진 시험 둘(`dock/targets.rs`)을 그 값으로 고쳤다.
11. **상수 내보내기** — `export_bindings_dock_constants` 가 쓴 `DockConstants.ts` 의 값 = `pub const`(같은 시험이 쓴 뒤 다시 읽어 견줌) · 두 번 돌려도 같은 바이트(결정적) · 커밋본과의 어긋남은 CI 생성물 동기 게이트가 잡는다. **대소문자 겹침** — 디렉터리를 읽어 견주는 것은 NTFS 에서 겹친 둘이 이미 한 파일이라 늘 통과한다(재리뷰 3차). 대신 둘: (a) `git -C $CARGO_MANIFEST_DIR ls-files -z --cached --others --exclude-standard -- bindings`(cargo 시험의 cwd 는 `src-tauri/` 라 `src-tauri/bindings` 경로는 빈 목록이 된다 · `--others --exclude-standard` = 아직 커밋 안 된 새 생성물도 본다)를 소문자로 접어 겹침 0 — 짝: 목록이 비지 않고 `DockConstants.ts` 를 담음(빈 목록의 눈먼 통과를 막는다) · (b) `DockConstants.ts` 내용 서명 — `export const DOCK_CONSTANTS` 와 `typeof DOCK_CONSTANTS` 가 있음(같은 경로를 ts-rs 내보내기가 덮어쓰면 빨개짐 · 병렬 내보내기와의 경주가 있어 진짜 그물은 CI 생성물 동기 게이트다).

### 8-2 TS 단위 (vitest)

- **`tabDragData`** — 짓기 → 읽기 왕복 · 대문자 identifier 를 소문자로 비교 · 크기 · drag · 모양 칸 없음 · 깨짐 거름 · 다른 `app` 은 우리 끌기 아님 · 값 JSON 깨짐 → `null` · `Files` · `text/uri-list` 만이면 우리 끌기 아님 · 모양 왕복(분할 · 비율 f64 그대로).
- **`dragSize`** — 골든의 새 창 크기 표 + Chrome C5 차례(작업 영역 상한 → 최대화 절반 하한이 이김) · 복원 크기 없음 → 지금 크기 · 탭 하나 팝아웃 → `size: null` · 원래 크기 = 복원(최대화일 때) − 크롬 · 틀 = `window:frame` 의 inset · `grab` = r × W1 · 탭 하나 → 누른 점.
- **`judge`** — 차례(탭바 구역 > 바깥 띠 > 칸 띠 > 그 밖) · 탭바 구역 경계(위아래 15 · 가로 = `+` 앞) · 출처 창이면 Chrome `insertionIndex` · 다른 창이면 `dropIndex` · 끄는 탭 자신의 캔버스 → 띠 없음 · 막힌 띠는 고르지 않음.
- **`dropIndex` · `dropMarkerX`** — 숨긴 탭이 넘친 탭바는 자르기 · 탭 0 · 1 개 · 빈 곳 = 끝 · 머리 가운데와 같은 x = 그 머리 왼쪽 자리(엄격한 `<`).
- **`succession`** — 셸 `succession_gives_the_same_tab_as_closing` 과 같은 표(n = 1…4 × 빠지는 자리).
- **상수** — 골든 머리 `_constants` = 생성물 `DockConstants.ts` 의 `DOCK_CONSTANTS`. 거울 파일에 공유 숫자를 손으로 적지 않았는지는 리뷰가 본다.
- **`bands` · `dockedTree`** — 골든 전부(같은 파일을 Rust 도 먹는다) + 골든 밖 표(칸 띠 최소 12 · 칸 50 폭 · 동률 차례 · 칸 가운데 `null` · 크기 없음 → 절반 몫 · 모양 없음 → 칸 하나) · 몫 예 AC2 · AC39 · AC44 · **유효성** — 면적 0 이 생길 띠 없음(칠하기 목록 = 고르기 목록) · 막힌 바깥 위 띠 밑의 칸 띠가 그 자리에서 칠해지고 골림(AC36).
- **`dragOverride`** — 리듀서 표(§2-4 의 계기 ① · ② · ③ · ④ 마다 · 시한 없음 · ② 는 끄는 탭이 활성인 알림으로는 안 풂 · 자기 dragend `move` + `done{Ignored}` · `done{Canceled}` 로는 안 풂 · ④ 같은 웹뷰 제자리 놓기 → dragend 에서 풂 · 그 끌기에는 `done` 이 오지 않는다 — 와도 무동작) + §8-3.
- **훅 · 컴포넌트** — `draggable` 조건(편집 중 · `+` · main 탭 하나 Q35 · 팝아웃 탭 하나는 끌림 Q34) · dragstart 의 `setData` · `effectAllowed` · 라벨 `setDragImage` · 우리 끌기의 `stopPropagation` · 탭바 구역 밖 = 남은 탭 · 돌아오면 끄는 탭 · 출처 탭바는 transform 만(DOM 순서 · key 그대로) · 다른 창 탭바는 세로선만 · 캔버스는 유효한 띠에서만 미리보기 · drop 처리기의 첫 호출이 `preventDefault` · drop 의 명령 인자(세 갈래 · 마지막 판정(미리보기)의 대상 · 판정 때 상태(`expect` · `rev` · `active`)를 싣지 않음 — ADR-0297) · 제자리면 안 보냄 · dragend 는 `none` · `move` 모두 보냄(옛 `copy` · `link` 갈래 없음 · 같은 창 같은 자리 놓기 · 엔진이 접은 끌기는 안 보냄) · `effect` = 자기 `dropEffect`(`move` 밖은 `none`) · `done{Canceled|Ignored}` → `switch_tab_if(initial, expect = 끄는 탭)` 한 번 · 활성이 바뀌었으면 · `initial = view` 면 · `Rejected` 면 · **자기 dragend 가 `move` 인데 `Ignored` · `Canceled` 면** 안 보냄(그 `done` 으로 풀지도 않음 — §4-6) · **같은 창 같은 자리 탭바 drop(아무것도 안 보냄) 뒤 출처 dragend 가 `DragEnd` 도 안 보내고 곧바로 바꿔 그리기를 풀고 transform 을 0 으로 거둠**(§2-4 ④ — 다른 창 drop 뒤에는 dragend 에서 풀지 않음) · 미리보기 지우기(dragleave · 300 ms 무소식 · 덮개 위 `mousemove` 300 ms — 버튼을 쥔 이동은 안 봄) · 열린 회차 없이 온 drop 은 그 점에서 판정(버리지 않음 · §4-3) · 회차 동안 캔버스 크기가 바뀌면 dragover 없이 마지막 점으로 다시 판정 · 같은 크기 알림은 무시 · 회차가 닫히면 지켜보기도 멈춤(§4-5 — `dock/windowDrop.test.ts`) · 되돌리기가 날아가는 동안 끄는 탭을 누르면 스토어 활성이 그 탭이어도 `switch_tab` 을 보냄(§4-6 — `TabBar.test.tsx`) · 우리 것이 아닌 끌기는 안 건드림(react-dnd · 파일).

### 8-3 순서 시험 — fast-check `fc.scheduler()`

- **`fast-check` 를 dev 의존으로 새로 들인다** — CLAUDE.md 「의존성 (변경 시 보고)」 대상이다.
- **출처 창** — `tab_drop:done` · 자기 invoke 응답 · `window:tabs-updated` · `layout:updated` · dragend 를 아무 순서로 섞는다(시한 없음). ① `done`(또는 invoke 오류 · 그물 계기 — §2-4 ② · ③ · 같은 웹뷰 제자리 놓기의 dragend — ④)이 닿으면 바꿔 그리기는 언제나 풀린다(자기 dragend `move` + `done{Ignored|Canceled}` 는 계기가 아니다 — 뒤따르는 `Drop` 의 `done` 또는 그물이 푼다) ② **시한 뒤집기가 없다** — `Applied` 로 끄는 탭이 그 창을 떠났으면 그 창이 그리는 탭은 끝까지(한 프레임도) 끄는 탭이 아니다: 풀림은 `done{Applied}` 와 탭 목록 바뀜이 둘 다 닿은 프레임이다 ③ 풀린 뒤 그리는 탭 = 모델 활성 ④ 같은 창 순서 바꿈은 transform 을 거둔 프레임과 모델 순서가 바뀐 프레임이 같다(튐 없음) ⑤ `done{Canceled|Ignored}` 뒤 `switch_tab_if(initial)` 은 많아야 한 번이고 · 그것이 셸에 닿기 전에 다른 탭이 골라지면 그 탭이 남고, 그사이 활성이 바뀌었으면 · 자기 dragend 가 `move` 인 `Ignored` · `Canceled` 면 0 번이다 · 뒤이은 `done{Applied}`(Drop 이 이김)와 섞여도 끝 화면 = 모델 활성 ⑥ dragend 뒤 끄는 탭을 활성으로 두지 않는 `window:tabs-updated`(LLM `switch_tab`)가 오면 그 뒤 그리는 탭 = 모델 활성 — 늦게 남은 바꿔 그림이 덮지 않는다(B5).
- **대상 창** — 지우는 사건(drop · dragleave · 시한)과 늦게 온 dragover 를 섞는다. 지운 뒤 늦은 dragover 가 새 끌기 없이 미리보기를 되살리지 않는다.

### 8-4 CDP — 실제 앱 안 시뮬레이션 (`scripts/cdp.mjs` · `/qa full`)

- **수단** — 출처 페이지에서 `Input.setInterceptDrags{enabled:true}` 를 켜고 페이지 마우스 사건으로 `Input.dragIntercepted`(DragData)를 받는다. 대상 페이지에는 `Input.dispatchDragEvent{dragEnter · dragOver · drop}` 을 넣는다.
- **★창 사이 놓기 뒤에는 출처 페이지에 `dispatchDragEvent{type: 'dragCancel'}`★** — 넣지 않으면 출처가 「끄는 중」으로 남아 다음 시험을 막는다(스파이크).
- **가로채기 아래의 출처 dragend 는 실제 입력과 다르다**(실측 2026-10-20 · U3 `/qa full`) — 같은 페이지 놓기는 늘 `move` 로 나고(순서 바꾸기면 `DragEnd{move}` · 제자리면 제자리 표시가 막는다), `dragCancel` 은 `none` 으로 날 때도 안 날 때도 있다. 난 `DragEnd` 는 늘 `Drop` 뒤에 와 셸이 「같은 끌기를 이미 처리」로 버렸다. 그래서 dragend 경로(`DragEnd`)의 판정은 CDP 로 재지 않는다 — §8-1 ② · §8-2 · §8-5 가 맡는다. 실제 커서를 읽는 길이라, CDP 로 띄우면 사용자 마우스 자리에 창이 생긴다. `NewWindow{by: Drop}` 도 같은 까닭으로 CDP 에서는 셸 로그까지만 본다.
- **항목** — C1 같은 창 순서 바꾸기(transform 만 · 놓기 1 회) · C2 창 사이 탭바(main ↔ 팝아웃 · 세로선) · C3 띠 넷 · 바깥 띠 · 칸 가운데(`NewWindow{by: Drop}` 명령까지) · C4 미리보기 사각형 = 기대값(DOM) · C5 **에이전트 트리 칸이 있는 창에서** dragstart 가 막히지 않고(DragData 가로채짐) dragover 를 받음(react-dnd) · C6 파일 DragData → 탐색 없음 · C7 **끌기 하나의 끌기 명령 수** — 모든 창 페이지의 CDP `Network.requestWillBeSent` 로 `ipc.localhost` POST 의 명령 이름과 `tab_drop` 몸의 `by`(`Drop` · `DragEnd`)를 적고(`window.__TAURI_INTERNALS__.invoke` 는 쓰기 · 재정의 불가라 감쌀 수 없다 — 실측 2026-10-20) 구간 둘로 센다. ① dragstart ~ drop 넣기 직전 = **끌기 명령(`tab_drop` · `switch_tab_if`)** 0 — 끌기와 무관한 호출(`resizePty` 의 `Resize` 를 싣는 `forward_daemon_command` · `window_frame` · 캔버스 크기 보고 따위)은 세지 않는다(ADR-0297 결정 1 · §4-7) ② drop 넣기 ~ `dragCancel` 직후(같은 창 놓기는 `dragCancel` 이 없다 — 출처 dragend 까지) = 받은 창의 `tab_drop{by: Drop}` 1(제자리 0). `tab_drop{by: DragEnd}` 는 따로 적고 합격 조건에 넣지 않는다 — 가로채기 아래에서는 0–1 번 난다(위 항목). 났으면 셸 로그에 그것을 중복(「같은 끌기를 이미 처리」)으로 버린 줄이 있어야 한다(실제 입력에서는 `DragEnd` 가 하나 더 · 같은 창 같은 자리 놓기는 0 — M3) · 셸 로그의 `tab_drop` info 줄과 대조 · C8 탭 하나뿐인 팝아웃 → 다른 창 탭바(그 팝아웃 닫힘) · C9 출처 탭바 구역 밖 → 남은 탭 → 돌아오면 끄는 탭 · C10 `done` 사건이 출처 창에만 한 번 · 다른 창 0(창 지정 듣기로 잰다 — 그냥 `listen()` 은 남의 창 몫까지 받는다 · §2-2).
- **`Drop` 없이 `dragCancel` 로 끝내는 흐름을 만들지 않는다** — 그때 난 `DragEnd{none}` 은 실제 커서 · 버튼 · 맨 위 창으로 갈려 사용자 마우스 자리에 창이 생길 수 있다.
- **자동화한 실제 마우스 입력은 쓰지 않는다** — 사용자가 마우스를 쓰고 있다(스파이크의 오염 기록).

### 8-5 실제 입력 체크리스트 — 사용자가 돌린다 (U3 뒤 · U4 전에 M1–M4 · M8 · M12 · M13)

| # | 항목 | 볼 것 |
|---|---|---|
| M1 | **Esc N ≥ 20** — 30 ms 톡 vs 100 ms 누름 · 바탕화면 위 · 우리 창 위 · 다른 앱 위 | 취소율(놓침 = 그 뒤 떼기가 놓기가 됨) · 취소 뒤 원래 선택 · 화면 그대로(R12) · Esc 가 슬롯에 들어가지 않음 · **바탕화면 · 다른 앱 위의 키 · 버튼 상태**(스파이크는 우리 창 안만 쟀다 — 셸 로그 `button` · `esc`) |
| M2 | **바탕화면 놓기** — 떼기 → dragend 지연(되튐) · 새 창 자리 = 커서 − 끌기 오프셋 · 크기(R22) · 두 모니터(주 125% · 보조 200%) · 모니터 넘기 | 지연 ms · dragend 뒤 셸 커서 = 뗀 점인가(바탕화면 = 미검) · 자리 어긋남 px(main 출처 vs 팝아웃 출처 — §9-2 `dragGrab`) · 배율이 섞일 때의 크기 · 자리 |
| M3 | **창 사이 OS 배달** — main ↔ 팝아웃 · 팝아웃 ↔ 팝아웃 · 탭바 세로선 · 띠 · 바깥 띠 | 미리보기 크기 = 원래 크기(**타입 문자열 칸이 창 사이 OLE 를 건너나** — `shape` 포함 · 깊은 탭의 긴 `shape`) · 출처 dragend `move` · 결과 한 번(셸 로그 = `Drop` 적용 + 출처 `DragEnd`(`effect = move`)의 `Duplicate` 또는 무시 — 어느 순서든 · 출처 화면에 끄는 탭이 다시 비치지 않음) |
| M4 | **탭 하나뿐인 팝아웃** — 다른 창에 놓기 · 창 밖 놓기 · 자기 캔버스 위 떼기 · 자기 제목 표시줄 위 떼기 · 최대화한 채 창 밖 놓기 | 닫힘 · 크기 유지하며 끈 만큼 옮김 · 옮김 · 아무 일 없음 · 복원한 뒤 옮김(사용자 결정) |
| M5 | **다른 앱에 놓기** — 메모장 · 탐색기 · 브라우저(놓기 받는 페이지 포함) | 늘 새 창(그 앱이 받아도 — 사용자 결정 Q39) · 글자 · 파일 새어 나감 없음 · 받는 페이지가 읽는 것은 끌기 자료뿐(받아들인 한계 — §9-2) · 셸 로그 `top` = `foreign`(받은 앱이면 `effect = move`) |
| M6 | **그림 · 커서 모양** — 점선 상자 · 라벨 · 다른 앱 위 · 배율 다른 모니터 | 육안(VS Code 라벨 꼴) |
| M7 | **main 의 탭 하나** — 끌기 시작 안 됨 · 클릭 · 이름 바꾸기는 됨(Q35) | — |
| M8 | **화면 캡처 도구(꿀캠) 켠 채** 창 사이 놓기 · 우리 창 위 잃은 놓기(옛 B2 짝) | 배달 · 결과 · 셸 로그 `top`(덮개를 건너뛰고 우리 창으로 읽나) |
| M9 | **에이전트 트리 칸이 있는 창**에서 끌기 · 놓기(react-dnd) · 트리 행 끌기는 그대로 | 둘 다 됨 |
| M10 | **진짜 탐색기 파일 끌기**를 각 창에 | 탐색 없음(AC22) |
| M11 | 끄는 중 터미널이 출력을 흘릴 때 · 같은 창 다른 탭에 이음새로 붙이기 · 출처 창 터미널 칸이 남은 탭 보이기로 드러났다 숨었다 할 때(WebGL 부착 · 가시성 길) | 끊김 · 깜빡임 없음 · 끌기 뒤 터미널 크기가 맞음(이음새로 칸이 줄어도 그 줄어든 크기 — 크기 맞춤은 끄는 동안에도 평소대로 흐른다 · §4-7) · **끄는 동안 대상 미리보기가 저절로 지워지지 않음**(mousemove 그물이 `buttons ≠ 0` 을 거른다는 실측 안 된 전제 — §2-4 「미리보기 지우기」 · 출처 창 · 다른 창 위에서 움직이고 멈추기를 되풀이하며 미리보기가 끊기지 않는지 본다) |
| M12 | **자기 탭바 순서 이동** — 끄는 탭이 transform 으로 움직이는 동안 OS 끌기가 끊기지 않나(W13) · 이웃 200 ms · 놓은 뒤 튐 | 끊김 0 · 튐 0 |
| M13 | **잃은 놓기 · z 판정**(4.6 결함 — AC27 · AC28 · R13b · R22 · AC8) — 멈춘 채 떼기 · 짧은 툭 · 빠른 휙 · 다른 창 탭바 위에서 떼기 · 겹친 두 우리 창(A 를 B 위로 끌어 B 밖 바탕화면에서 떼기) · 최대화된 출처를 덮은 다른 앱 창 위에서 떼기 · 우리 창 제목 표시줄 위에서 떼기 | 우리 창 위에서 뗐는데 새 창이 생기지 않음 · 바탕화면 · 다른 앱 창 위는 새 창 · 제목 표시줄은 아무 일 없음(Q38) · **dragend 때 셸 로그 `top` 이 끌기 그림 창으로 읽히지 않나** — 그 창은 WebView2 브라우저 프로세스 것이라 우리 프로세스 판정으로는 못 거른다 · `foreign` 줄의 창 클래스 이름을 본다 · 우리 웹뷰가 받은 놓기 뒤 새 창이 따로 생기지 않음(`effect = move` 인 `DragEnd` 가 `foreign` 으로 읽히면 생긴다 — §3-4) |

## 9. 정지 조건 · 위험 · 미결

### 9-1 정지 조건 (걸리면 멈추고 사용자에게 올린다 — 대안을 지어내지 않는다)

| 조건 | 그때 |
|---|---|
| M1 의 Esc 놓침이 잦다(예: 20 회 중 2 회 이상) | 받아들일지, ADR-0296 「거부한 대안」(셸 OLE 출처)을 다시 열지 사용자 결정 |
| M2 의 되튐 지연 동안 커서가 움직여, 새 창이 뗀 점에서 눈에 띄게 어긋난다 | dragend `screenX/Y`(DIP)를 쓸지, 받아들일지 사용자 결정 |
| M12 — 끄는 탭을 transform 으로 움직이면 OS 끌기가 끊긴다 | 자기 탭바도 세로선으로 바꿀지 사용자 결정(W13 을 다시 연다) |
| M13 — 우리 창 위에서 뗐는데 새 창이 생기거나, 바탕화면에서 뗐는데 안 생기는 일이 잦다 | z 판정을 다시 정하고 사용자에게 보고 |
| M13 — dragend 때 맨 위 창이 끌기 그림 창으로 읽힌다(그 창은 **우리 프로세스가 아니다** — 끌기 출처 · 그림은 WebView2 브라우저 프로세스 안이라(ADR-0296 「거부한 대안」 셋째) `hit_accepts` 에 `ours = false` · `top = Foreign` 으로 보이고, 그러면 우리 창 위 잃은 놓기가 새 창이 된다. **우리 웹뷰가 받은 성공한 놓기도** `DragEnd` 가 `Drop` 보다 먼저 닿으면 새 창이 되고 진짜 `Drop` 은 `Duplicate` 로 버려진다 — `move` + `Foreign` 은 다른 앱이 받은 것과 가를 수 없다 · §3-4. 다른 `Foreign` 오판(덮개 · 알림 창)도 같다) | **멈추지 않는다 — 내부 대체.** `top_level_at` 이 그 창을 **프로세스가 아니라 창 클래스로** 건너뛴다(클래스 이름 = M13 의 셸 로그). 보고만 한다 |
| M3 — 타입 문자열의 칸(크기 · 모양)이 창 사이를 못 건넌다 | **멈추지 않는다 — 내부 대체.** 그때 웹 ↔ 웹 `BroadcastChannel` 을 들여 실음을 보낸다(지금은 없다 — 배달 경주는 미리보기 한두 프레임의 기본값으로 받는다). 보고만 한다 |
| 어떤 창 종류(런타임 팝아웃)가 HTML5 사건을 못 받는다 | 멈춤(S11 — 셋 다 `no_drop_handler` 확인 · 실제 OLE 팝아웃 수신은 이번 스파이크 미검) |
| react-dnd 와의 겹침이 `stopPropagation` 으로 안 풀린다(C5 · M9) | 멈춤 |

### 9-2 위험

- **빠른 Esc 를 놓침** — OS 끌기 고리의 성질이다(S14 1/5). 놓치면 그 뒤 떼기가 놓기가 된다. 우리 코드가 고칠 수 없다(M1 · PRD R12 가 받아들임).
- **잃은 놓기**(PRD R13b · AC27 · AC28 의 되살림) — 줄이는 장치 둘 = 뿌리에서 늘 받기(§4-3) · dragend 뒤 z 판정(§3-5 ③ — 맨 위 창이 우리 창이면 무시). 남는 틈 = 맨 위 창 판정이 틀림(끌기 그림 창이 남음 · 다른 앱의 비쳐 보이는 창 · 알림 창 — B2 `hit_accepts` · M8 · M13). 그 오판(`Foreign`)은 `DragEnd` 가 먼저 닿으면 **우리 웹뷰가 받은 성공한 놓기도 새 창으로 바꾼다** — `move` 여도 다른 앱이 받은 것과 가를 수 없고, 뒤의 진짜 `Drop` 은 `Duplicate` 다(§3-4). 읽기 실패(`Unknown`)는 `move` 면 무시라(§3-5 ⑤) 이 틈이 없고, `none` 이면 지금처럼 새 창이다. 잠금 화면 · UAC 가 끝낸 끌기의 키 · 버튼 상태는 미검이라 새 창으로 읽힐 수 있다(PRD R13b).
- **진행 중인 새 창에 닿은 `Drop` · 그 새 창의 거절**(재리뷰 3차 · 리뷰 2026-10-20 U1 — 막았다 · 재리뷰 2026-10-20 · 메인 결정) — `DragEnd` 의 새 창이 진행 중일 때(그 거절 알림 중 포함) 닿은 `Drop` 은 버리지 않고 장부가 맡아 둔다(§3-4). 그 새 창이 거절되면 맡은 `Drop` 을 이어 처리하므로 놓기를 잃지 않는다. 거절 판정 때 이미 맡아 둔 것이면 그 `DragEnd` 는 `done` 을 내지 않아 출처는 그 `Drop` 의 `done` 하나만 받는다 — 바꿔 그리기가 거절에 풀렸다 다시 걸리는 일이 없다(§3-2 ④). **받아들인 틈** = 그 `Drop` 이 거절 알림을 낸 뒤 · `close` 전에 닿은 경우 — 출처는 `done{Rejected}` 을 먼저 받아 바꿔 그리기를 풀고(§2-4 ①) 곧이어 그 `Drop` 의 `done` 을 받는다. 차례는 보장되지만 그 사이 한 프레임쯤 끄는 탭이 보일 수 있다. 이 길에 닿는 것은 우리 웹뷰가 받은 끌기(`move`)의 맨 위 창을 `Foreign` 으로 잘못 읽은 경우뿐이다(그 밖 = dragend 는 `none` 인데 `Drop` 이 오는 비정상 배달 — §4-6) — `Unknown` 은 `move` 면 무시라(§3-5 ⑤) 빠졌다. 장부가 넘쳐도 진행 중인 기록은 잊지 않으므로(§3-4) 맡은 `Drop` 을 잊어 잃는 길은 없다.
- **되돌리기 ABA**(재리뷰 2026-10-20 · 메인 결정 — 받아들인 틈) — 되돌리기 명령이 셸에 닿기 전 몇 ms 사이에 활성이 B→C→B 로 두 번 바뀌면 조건(활성 == 끄는 탭)이 통과해 `initial` 로 덮는다. IPC 한 왕복 안의 두 번 전환이라 받아들인다.
- **끄는 중 LLM 이 끄는 탭을 다른 창으로 옮기거나 닫음 — 그 끌기의 출처 dragend 가 오지 않음**(메인 판단 — ADR-0297 원칙(예외 최소)의 적용 · 사용자 위임으로 받아들임(2026-10-21) · §9-5) — 끄는 동안 막지 않으므로(ADR-0297 결정 1 · §3-6) 그 명령은 성공한다. 출처 탭바는 머리를 모델 목록에서 그리므로(`src/components/layout/TabBar.tsx` 의 `order` 는 `ids` 에서) 끄는 머리가 떼어지고, 그 끌기의 출처 dragend 는 오지 않는다(같은 파일 머리 주석 `:12-13` 이 이 사실을 적는다). 그래서 `DragEnd` 가 없다 — 우리 웹뷰 밖(바탕화면 · 다른 앱)에서 떼면 아무 일 없다(새 창 없음) · 우리 웹뷰에서 떼면 `Drop` 이 평소대로 처리된다(§3-3 「놓을 때 다시 찾기」 — 옮겨졌으면 지금 창에서 빼 옴 · 닫혔으면 무동작) · 출처의 바꿔 그리기는 그 탭을 빼는 `window:tabs-updated` 가 푼다(§2-4 ②). 탭 하나뿐인 팝아웃이면 끄는 중 출처 창 자체가 닫힌다 — WebView2 OLE 끌기에서 어떻게 되는지는 미검.
- **우리 창 제목 표시줄 · 테두리 위 떼기** — 웹뷰 밖이라 drop 이 없고 z 판정이 우리 창으로 보므로 새 창이 아니라 아무 일 없다(사용자 결정 2026-10-20 Q38 · PRD R22 · AC8 에 적었다).
- **다른 앱이 받은 놓기 — 끌기 자료를 남이 읽음**(사용자 결정 2026-10-20 Q39 「새창.」의 받아들인 한계 · 숨기지 않는다) — 놓기를 받는 웹 페이지(다른 브라우저 등)는 drop 에서 우리 MIME 의 값(JSON `{v, drag, view, from, initial, size, grab}` — 탭 · 창 식별자 · 창 label · 크기)과 타입 문자열(`app` identifier · 크기 · drag id · 끄는 탭 모양)을 읽을 수 있다. 에이전트 내용 · 출력은 싣지 않는다(§4-2). 우리 쪽 결과는 새 창이다(§3-5 ④ — 맨 위 창을 못 읽으면(`Unknown`) 무시라 새 창이 없고 누른 탭이 활성으로 남는다 · §3-5 ⑤). 값은 우리 대상 웹뷰가 drop 에서 읽어야 하므로(§4-3) 실음에서 뺄 수 없다.
- **취소 가름이 틀릴 때** — Esc 를 아주 짧게 치고 곧바로 버튼을 떼면 둘 다 떨어진 채 읽혀 놓기가 된다. 되튐 동안 다시 누르면 취소로 읽힌다. 명령이 늦게 닿으면(셸이 바쁨) 이 틈이 넓어진다 — 그래서 읽기를 맨 먼저 한다(§3-2). 키 · 버튼 · 커서 읽기는 우리 창 안에서만 쟀다 — 바탕화면 · 다른 앱 위는 미검(M1 · M2).
- **바꿔 그리기가 남음** — 출처가 dragend 마다 `DragEnd` 를 보내므로 `done` 은 보통 늘 온다. 남는 틈 = 우리 웹뷰가 받았는데(자기 dragend `move` → `done{Ignored|Canceled}` 로는 풀지 않음 — §4-6) 받은 창의 invoke 가 셸에 못 닿음 · 사건 유실 · 맨 위 창을 못 읽었는데 다른 앱이 받음(§3-5 ⑤ — `Drop` 이 오지 않는다) · 받은 창의 `Drop` 이 커서를 못 읽음(`Ignore("no-cursor")` — `move` 라 그 `done{Ignored}` 로 풀지 않는다 · 맡아 둔 `Drop` 이 거절을 대신한 길도 같다). 같은 끌기의 `DragEnd{move}` 무시(`accepted-ours`)의 `done{Ignored}` 가 `Drop` 의 `done{Applied}` 보다 늦게 닿을 수 있다(두 처리가 다른 스레드 — 이 판 전부터 있던 틈) — `move` + `Ignored` 는 풀지도 되돌리지도 않으므로(§2-4 ①) 화면 영향이 없다. 보내지 않은 같은 웹뷰 제자리 놓기는 dragend 에서 풀므로(§2-4 ④) 틈이 아니다. 그때 출처의 바꿔 그리기가 그물 계기(§2-4 ② 그 창의 다음 `window:tabs-updated` 중 끄는 탭이 활성이 아닌 것 · ③ 다음 누름)까지 남는다(남은 탭이 보이고 모델 활성은 누른 탭) — 시한을 두지 않은 대가다.
- **끄는 출처 요소를 옮김**(W13) — 끄는 탭을 transform 으로 움직이는 것이 OS 끌기를 끊지 않는다는 실측이 없다(M12). 선례는 Chromium 옛 WebUI 탭바 하나다.
- **웹 거울이 Rust 와 갈라짐** — 골든이 담은 표본만 지킨다. 갈라지면 미리보기 ≠ 결과(R4)이고 셸은 놓을 때 몫 범위 · 면적 0 만 다시 본다.
- **`shape` 칸의 길이** — 분할이 아주 많은 탭에서 타입 문자열 상한은 미검이다(M3). 못 실으면 칸 하나로 쳐 막힌 띠가 칠해질 수 있고, 놓으면 `ZeroArea` 거절 · 탭 제자리다.
- **배율이 섞인 새 창** — `open_at` 은 물리 자리 + 논리 크기를 받는다. 배율이 다른 모니터에 열 때 tao 의 `WM_DPICHANGED` 처리가 크기를 한 번 더 고칠 수 있다(6.3 E3 이 본 현상 — 이번에는 두 걸음 놓기를 하지 않는다). 웹의 `screen.avail*` 은 출처 창 화면 기준이다(Chrome 은 커서에 가까운 화면). M2 가 잰다.
- **새 창 커서 자리(`dragGrab`)** — 새 창 탭바의 왼쪽 · 세로 자리가 출처 탭바와 같다고 가정한다([미검]). 새 창은 팝아웃인데 main 은 탭바 왼쪽 · 위의 크롬이 팝아웃과 다를 수 있어, main 에서 끌어 낸 새 창은 커서에서 그 차이만큼 밀려 뜬다. `headRoom`(새 창 탭바 폭)은 U3 가 새 창 크기로 센다. M2 가 main → 새 창의 커서 어긋남을 잰다.
- **Snap 해 둔 출처 창의 「복원 크기」** — Snap 전 크기인지 미검이다(PRD R7).
- **macOS(WKWebView) · WebKitGTK** — 창 사이 HTML5 끌기 · 타입 문자열 칸 · dragend `dropEffect` 가 미검이다(ADR-0296). `escape_down` · `top_level_at` 은 Windows 밖에서 거짓 · `None` 이라 `top` 이 늘 `Unknown` 이다. dragend 가 `move` 면 무시라(§3-5 ⑤) 우리 웹뷰가 받은 놓기는 `Drop` 이 정하고 `DragEnd` 와 경주하지 않는다 — 대신 다른 앱이 받은 놓기에 새 창이 없다(Q39 와 다름). `none` 이면 취소 가름이 버튼 하나로 줄고 잃은 놓기가 새 창으로 떨어진다(`Unknown` + `none` → `Place`). 이식 때 platform 에 그 OS 의 키 상태 · 맨 위 창 읽기를 더한다.
- **CDP 시험의 한계** — 출처 dragend 가 실제 입력과 다르고(§8-4) OLE 를 지나지 않는다. 창 사이 배달 · 타입 문자열 건넘 · 그림 · 잃은 놓기 · z 판정은 M 항목만 잰다.

### 9-3 사용자 결정 대기 · 정해진 것 · 메인이 정한 것

- **사용자 결정 대기**
  - **①** `ff.json` 지우기 — 쓰레기로 보인다(nginx 301 HTML).
  - ~~**②** AC8 둘째 예외 — 끄는 중 LLM 이 끄는 탭을 옮기거나 닫으면 우리 웹뷰 밖 놓기가 무동작이다(→ §9-2 · §9-5).~~ → 받아들인다 · PRD 에는 적지 않는다(사용자 위임으로 메인이 정함 · 2026-10-21).
  - **③** AC8 `Unknown` + `move` 예외 — 다른 앱이 받았는데 맨 위 창을 못 읽으면 새 창이 없다(→ §9-5).
- **정해짐(2026-10-20)**
  - 최대화된 탭 하나뿐인 팝아웃을 창 밖에 놓기 = 복원한 뒤 옮김(「1. 맞고」 · §3-5 · PRD Q37) · 미커밋 처리 = (B) 갈라 커밋(§6).
  - **우리 창의 제목 표시줄 · 테두리 위에서 떼기 = 아무 일 없음**(취소와 같다 — 「별도의 처리를 안하는 방향으로.」 · PRD Q38 · R22 · AC8) — 그 수단이 dragend 뒤 z 판정(커서 밑 맨 위 창이 우리 창 = 무시 · §3-5 ③)이다. 7판 초안은 이것을 아래 「메인이 정한 것」에 두었다.
  - **다른 앱이 놓기를 받아도 새 창**(「새창.」 · PRD Q39 · R22 · AC8) — 그래서 우리 웹뷰 밖에서 뗀 끌기는 「우리 창 위면 아무 일 없음 · 그 밖은 전부 새 창」 하나다(§2-3 · §3-5 ④). 받아들인 한계 = 놓기를 받은 페이지가 끌기 자료를 읽을 수 있음(§9-2).
  - **탭 모양 = Chrome 처럼 둥글게**(「크롬처럼 둥글게」 · PRD R27 — 위 모서리 10 · 아래 바깥 휘어짐 12 · 구분선 2×16 이 배경 보이는 탭 옆에서 사라짐) — **ADR-0168 결정 5 에 「부분 폐기 by ADR-0295」를 박았다**(ADR-0295 결정 1 의 「활성 탭 모양」 몫 — ADR-0296 이 대신한 것은 그 결정의 끌기 수단뿐이라 이 몫은 살아 있다). 폐기 범위 = R27 이 바꾼 것 — 위 모서리 6px · 구분선 규칙(「비활성 탭 사이에만」) · 비활성 탭 바탕 토큰 `--tab-inactive-bg`. 남는 것 = 닫기 `×` 항상 표시 · 활성 탭이 바보다 밝음 등 결정 5 의 나머지. 미커밋 묶음 6(§6 의존 3)이 이것으로 막힘이 풀렸다 — U0 를 막지 않는다.
  - **ADR-0297 — 끄는 동안 아무것도 막지 않음**(「상관없어 그냥 예외 최대한 적게 넣으면 됨. 관행에 최대한 맞춰서」 · §2-2 · §4-7) — 끌기와 무관한 셸 호출은 끄는 동안에도 흐른다. 옛 메인 결정 「끄는 동안 터미널 `resizePty` 미루기」를 대신한다.
  - **ADR-0297 — 놓을 때 대상을 id 로 다시 찾아 있으면 지금 상태에 맞춰 적용 · 없으면 무동작**(「크리티컬한게 아니고 어차피 유저가 다시 하면 되는거라서」 · §2-4 · §3-3) — 옛 메인 결정 「사전조건(`TargetExpect` · `SeamAt` 판 번호 · `expect_window`)으로 전송 중 바뀜 거절」을 대신한다.
    - **(메인 결정 · 보고함) ADR-0297 결정 2 의 적용 — 놓기 대상이 그사이 다른 창으로 옮겨졌으면 지금 사는 창에서 처리**(사용자 말 인용이 아님 — 메인이 정해 보고 2026-10-20 · §3-3): 끄는 탭은 지금 창에서 빼 오고 그 창의 남는 탭은 셸 `succession`(`initial` 은 그 탭이 `from` 에 있을 때만 — `successor_window`) · 이음새 대상 탭이 옮겨졌으면 그 창에 붙이고 그 창에 포커스 · 웹 결함으로 남의 창 탭을 겨눠도 거절하지 않음(옛 `SeamAt.window` 대조를 잃음 — 받아들임).
  - **놓기 = 마지막 판정(미리보기) 기준** — 놓은 순간 좌표로 다시 판정하지 않는다(「유저는 미리보기로 판단하는거잖아」 · §4-3).
  - **같은 창 같은 자리 놓기 = 셸 호출 0 회 — 출처 dragend 도 보내지 않음**(「드래그 스타트 안보내는거면 엔드도 보내지마」 · 엔진이 접은 끌기도 같다 · §2-4 ④ · §4-2).
  - **끄던 탭을 지우는 `window:tabs-updated` 가 dragend 전에 출처 바꿔 그리기를 푸는 그물은 그대로**(「극 예외는 실 발생될때 처리」 · §2-4 ② · 아래 U2 ⑥).
- **메인이 정하고 보고만 하는 것**(내부 구현 — 사용자가 뒤집을 수 있다) — 원래 크기 · drag id · 끄는 탭 모양을 타입 문자열에 실음 · 판정 기하 정본 = Rust(`layout/`) + 웹 거울 + 양쪽이 먹는 골든 · 공유 상수는 Rust `pub const` 한 곳 → 생성 `src-tauri/bindings/DockConstants.ts` 하나(메인 결정 2026-10-20 · 대소문자만 다른 생성물 금지 — §4-5) · 창 틀 inset 에 기존 `Insets` 타입을 씀 · 띠 유효성(R8)을 웹이 판정 · drag id 장부(Drop 이 앞선 취소 · 무시 · 거절된 새 창을 이김) · 뿌리에서 늘 받기 · drop 처리기 첫 문장 `preventDefault` · 출처는 dragend 마다 `DragEnd` 를 보냄(메인 결정 2026-10-20 · `copy` · `link` 갈래 걷음 · 같은 창 같은 자리 놓기 · 엔진이 접은 끌기는 제외 — 위 사용자 결정) · 되돌리기가 날아가는 동안 끄던 탭을 누르면 명시 `switch_tab`(클릭이 이김 — §4-6) · `DragEnd` 에 자기 dragend `effect`(`none` · `move`)를 실어 `move` 면 셸이 취소를 보지 않고 우리 창 · 못 읽음을 무시로 가름(재리뷰 3차 — §3-5 ⑤) · 같은 웹뷰 제자리 놓기는 출처가 dragend 에서 곧바로 풂(§2-4 ④) · 결과 알림 `tab_drop:done`(시한 없음 · 그물 계기 — §2-4) · 취소 · 무시 뒤 `switch_tab_if(initial, expect = 끄는 탭)`(셸이 그 창 활성을 한 번 더 봄 · 거절은 안 함 · 자기 dragend 가 `move` 면 안 함) · `window:frame` 바뀔 때만 · 틀 inset 셸 셈 · 모듈 이름 `tab_drop` · U4 를 a → b → c 순서로 · U0 (B) 의 hunk 가름(§6).
  - **U2 에서 정한 것**(메인 · 2026-10-20)
    - U2b(웹 순수 모듈): ① `dropIndex(B, x, dragged)` 의 `dragged` = 이 탭바에서 끄는 탭의 자리(빼고 센다) ② `dropMarkerX` = 경계 x(탭 0 개 = 0) · 2 px 선은 양쪽 머리에 1 px 씩 ③ 모양의 슬롯 · 분할 id = 전위 순서 `shape-slot-<k>` · `shape-split-<k>` · 내용 비움 — `dockedTree` · 골든이 `parseShape` 를 함께 씀 ④ 타입 문자열의 `w` · `h` 가 양의 정수가 아니면 `null`(대상 → 절반 몫) · drag 칸이 없으면 우리 끌기 + drag `null` ⑤ 탭 하나뿐인 팝아웃 `Reuse` — 바꿔 그림이 없으면 `Applied` 에서 곧바로 풂 ⑥ 끄는 탭이 그 창 탭 목록을 떠나면 어느 단계든 풂(dragend 전에 닿은 `Drop` 의 `window:tabs-updated` · 끄는 중 LLM 닫기) ⑦ U3 계약 — `DragEnd` invoke 를 보내기 전에 `end` 를 넣는다 · `live` 중 `done` = 받은 창의 `Drop`(`move`) ⑧ Esc · 취소 · 무시 되돌리기 뒤 모델 활성이 `initial` 이 될 때까지(또는 그물 ② · ③) `initial` 을 그린다 — `initial` → 끄는 탭 → `initial` 번쩍임 없음 ⑨ dragstart 와 같은 task 의 dragend(엔진이 접음) = 아무것도 안 보내고 ④ 처럼 제자리에서 풂(`end{local: true}`) · 되돌리지 않음 ⑩ §8-3 대상 창 「늦은 dragover 가 미리보기를 되살리지 않음」 시험은 U3 로.
    - U2a(셸 기하): `SizeDip` 를 `layout/bands.rs` 로(U4c 에서 U1 `SizeCss` 와 합침) · `drag_sizes` 를 더함(원래 크기는 0 이상으로 자름) · §8-1 ⑪ (a) 의 목록에 `--others --exclude-standard` 도 넣음(아직 커밋 안 한 새 생성물) · 셸 dev 의존 `serde_json` 의 `float_roundtrip` 기능 · 웹 dev 의존 `fast-check`(둘 다 사용자에게 보고 2026-10-20) · 골든은 탭바 · 띠 · `null` 만 덮는다 · `SlotZones.slot` = 캔버스 좌표 · 모델 몫 상한 = `layout::dock::SHARE_MAX`(`ATTACH_SHARE_MAX` 는 그 이하 — 컴파일 단언) · 끌기 되돌리기 = 셸 조건부 명령 `switch_tab_if`(버스 밖 — §4-6).
    - U2c(웹 기하): `dragGrab` 은 새 팝아웃 탭바의 왼쪽 · 높이가 출처와 같다고 가정한다([미검] — §9-2 · `headRoom` 은 U3 가 셈) · TS `RectCss` · `SlotZones` 는 U2 가 세운 `overlayTypes.ts` 에 산다(새 파일은 바인딩 `RectCss` 를 잇지 않는다 — §5-4) · `moveDue` 는 부르는 쪽 몫(§7 U3) · `windowBands` 는 레이아웃만 받아 칸 틀을 스스로 센다(셸 `frames` 와 같은 셈) · 출처 탭바의 `dragged` 가 목록 밖(끄는 탭이 그 탭바를 떠남)이면 `judge` 는 다른 창 길(`dropIndex` · `dropMarkerX`)로 센다 · 고정 id `'docked-wrap'`(기하 전용).

### 9-4 VS Code 대조 (조사 F · `6c8316e`)

| 동작 | VS Code | 우리 | 근거 |
|---|---|---|---|
| 끌기 시작 | 문턱 없음(브라우저 기본) | 같음 | PRD R2 |
| 끌기 그림 | 라벨 `setDragImage(el, −10, −10)` · 여백 1 px 7 px · 둥글기 10 · 12 px · 최대 120 px · 말줄임 | **따름** | §4-2 · R29 |
| 자기 탭바 | 2 px 세로선 · 탭 안 움직임 | Chrome 식 순서 이동(transform) | 사용자 결정 W13 |
| 다른 창 탭바 | 탭 가운데 기준 · 2 px 세로선 · 빈 곳 = 끝 | **따름** | 사용자 결정 W13 |
| 탭 위 1500 ms 머물면 열기 · 창 올리기 | 열기 있음 · 올리기 없음 | 둘 다 없음 | 사용자 결정 Q36 |
| 칸 띠 | 가장자리 10% · 방향 = 1/3 · 2/3 · 1/2 경계 | 10% · 방향 = 가장 가까운 변(대각선) · 면적 0 이 생길 띠는 숨김 | ADR-0295 · PRD R6a · R8(§9-5 확인) |
| 미리보기 | 나누기 50% · 합치기 100% · 150 ms 페이드 · 70 ms 옮김 · `pointer-events: none` · 멈춘 덮개 300 ms 가드 | 크기 = 붙는 크기(ADR-0295) · 움직임 · 가드는 **따름** | §4-4 · §2-4 |
| 자기 칸 | 출처 칸이 비게 되면 덮개 없음 | 끄는 탭 자신의 캔버스에 띠 없음 | §4-3 |
| 칸 가운데 | 그 그룹에 합침 | 새 창 | PRD R6 |
| 놓기 · 빈 칸 | `moveEditor` 한 번 · 보조 창은 마지막 칸이 빠지면 닫힘 · 메인은 안 지움 | 같음(`dock_tab` 한 번 · R10 · R14) | §3 |
| 창 밖 판정 | 출처 dragend · 「어느 창도 dragover 중 아님」 불리언(`BroadcastChannel`) · 네이티브 커서 · 활성 창 사각형 안이면 안 만듦 | 출처 dragend(늘 — `none` · `move` 를 실음) · 끝난 뒤 셸이 커서 · 키 · 버튼 · **커서 밑 맨 위 창(z 순서)** 을 읽음 — 우리 창이면 무시(`move` 면 못 읽어도 무시) · 그 밖은 다른 앱이 받아도 새 창(Q38 · Q39) | **따르지 않음** — 겹친 창 · 다른 창 탭바 위 잃은 놓기 · 최대화된 출처(리뷰 2026-10-20 · §3-5) |
| 새 창 자리 · 크기 | 커서 − (폭/2, 30 + 높이/2) · 1024 × 768 | 커서 − 끌기 오프셋 · Chrome `dragged_size` | PRD R22 |
| Esc | 다루지 않음(창 밖 Esc 도 새 창일 수 있음) | 셸 키 상태로 취소 · 원래 선택 되돌림 | PRD R12 |
| 메인 마지막 탭 | 끌 수 있음(빈 칸이 남음) | 끌기 시작 안 함 | 사용자 결정 Q35 |
| 창 사이 자료 | 같은 JS 맥락 싱글턴 | MIME 타입 문자열 · 셸 없음 · 창 사이 통신 없음 | §4-2 |
| 빈 탭바 · 창 끌기 | 빈 탭바 = 그룹 통째 끌기 손잡이 · 창 끌기 = 제목 표시줄에만 · 빈 탭바 두 번 = 새 문서 | 1차에서 정하지 않음 — **T-66 맞물림 지점**(사용자 바람 = 제목 표시줄을 걷은 뒤 빈 탭바로 창 끌기) | R25 · T-66 |

### 9-5 PRD 4.9 와 맞출 곳

- **R21 · AC18 · §2 범위 · 4.9 머리 · R3a 등 「다른 창 탭바의 틈 · 비켜남」** — W13(2026-10-20)대로 2 px 세로선으로 고쳤다(2026-10-20 · 리뷰 반영). 남은 맞춤 없음.
- **R22 · R6 · AC8 「그 밖에 놓기 → 새 창」** — 이 TRD 가 경계를 더했다: 잃은 놓기 무시(§3-5 ③ — dragend 때 커서 밑 맨 위 창이 우리 창이면) · 그래서 **우리 창의 제목 표시줄 · 테두리 위에서 떼면 아무 일 없다** · 맨 위가 다른 앱 창이면 출처 창 사각형 안이어도 · **그 앱이 놓기를 받아도** 새 창. 사용자 결정 Q38 · Q39(2026-10-20)로 PRD 4.9 R22 · AC8 · §5-1 에 적었다 — 우리 웹뷰 밖에서 뗀 끌기는 「우리 창 위면 아무 일 없음 · 그 밖은 전부 새 창」 하나다. **남은 맞춤 하나(사용자에게 보고 — 재리뷰 4차)**: 다른 앱이 받았는데(`move`) 커서 밑 맨 위 창을 못 읽으면(`Unknown` — Windows 에서는 읽기 실패뿐 · Windows 밖에서는 늘) 새 창이 없다(§3-5 ⑤) — 우리 웹뷰의 놓기가 `DragEnd` 와 경주해 가짜 새 창이 생기는 것을 막는 대가다. PRD R22 · AC8 에 이 예외를 적을지는 사용자 확인 뒤.
- **AC8 「그 밖은 전부 새 창」의 둘째 예외(받아들임 — 사용자 위임으로 메인이 정함 · 2026-10-21)** — 끄는 중 LLM 이 끄는 탭을 다른 창으로 옮기거나 닫으면 그 끌기의 출처 dragend 가 오지 않아 우리 웹뷰 밖에서 떼도 새 창이 없다(§9-2 — 메인 판단 · ADR-0297 원칙(예외 최소)의 적용). PRD 쪽 자리 = D9(4.9 — 끄는 중 같은 탭을 옮기는 명령은 그대로 적용 · 「T-65 가 다시 정한다」) · T-65. PRD 에는 적지 않고 여기에만 둔다(같은 위임).
- **R22 새 창 크기** — 웹이 dragstart 에 셈한다(§4-2). 작업 영역 = 출처 창 화면이다(Chrome 은 커서에 가까운 화면 — M2 로 차이를 본다).
- **R2 · R15 남은 탭** — 출처 탭바 구역을 벗어나면 보이고 돌아오면 거둔다(§4-2). 이 TRD 는 PRD R15c 의 「구역 떠남」 시점을 따랐다.
- **R6a 띠 방향** — VS Code(1/3 경계)와 다른 지금 규칙(가장 가까운 변)을 그대로 두었다. VS Code 를 따를지 다시 보려면 사용자 결정이다(안내 선 모양도 함께 바뀐다).
- **R8 · AC36** — PRD 그대로 따른다(웹이 막힌 띠를 칠하지도 고르지도 않는다 — §4-5). 7판 초안의 「웹은 R8 사전 판정을 하지 않는다」는 걷었다.
