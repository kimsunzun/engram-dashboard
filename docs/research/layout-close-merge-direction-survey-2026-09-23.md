# 분할 레이아웃에서 슬롯을 닫을 때 누가 공간을 흡수하나 — 피어 서베이

- 상태: 확정 조사 · **결정 = 현행 유지(A — 형제 흡수, 방향은 분할 순서가 정한다 · 사용자 결정 2026-09-24)** · 날짜: 2026-09-23 · 강도: medium · 설계-결정 모드
- 방법: 주계열(Claude) 수집자 3갈래(터미널·멀티플렉서 / 타일링 WM·에디터 / 레이아웃 UI 라이브러리) + 메인 grounding(로컬 클론 tmux·zellij 코드, GitHub API로 이슈 3건 본문, VS Code `grid.ts`·Emacs 매뉴얼 원문 대조) + cross-family(codex, effort high, 웹 검색) 적대 리뷰 1회 — 판정 REFUTE-PARTIAL, 지적 6건 전부 반영(피어 누락 2 · 오귀속 1 · 과장 2 · 버전 세부 1)
- 확신도 범례: 확실 = 독립 출처 2개 이상 또는 로컬 코드 직접 확인 / 가능성 높음 = 단일 출처로 지지 / 불확실 = 미지지·추론

## 질문

2×2 그리드에서 슬롯 하나를 닫으면 우리 앱은 "무조건 가로 병합"한다(사용자 보고). 다른 도구는 어떻게 하나, 우리는 무엇으로 갈까.

## 우리 현황 (코드 확인)

- 레이아웃 = **이진 분할 트리**(`LayoutNode::Split{dir, ratio, a, b}`). 닫기 = 닫힌 슬롯의 부모 Split을 **형제 노드로 교체**(`src-tauri/src/layout/tree.rs:101` `close_in_tree`). 기하는 보지 않는다.
- 그래서 흡수 방향은 **그리드를 만든 분할 순서**가 정한다. 위아래(top_bottom)를 먼저 가르고 각 줄을 좌우로 가른 그리드 `TB{LR{1,2}, LR{3,4}}`는 어느 칸을 닫아도 가로 이웃이 흡수한다. 좌우를 먼저 가르면 세로 이웃이 흡수한다(감사 프로브로 재현 — 확실).
- 메뉴의 "가로 분할"은 ADR-0140 어휘상 `top_bottom`이다. "가로 분할 → 각 칸 세로 분할" 순서로 만들면 항상 가로 병합이 된다.

## 발견

### F1. 트리 기반 도구의 지배 패턴 = 형제 흡수. 방향은 분할 이력이 정한다 — 가능성 높음

레이아웃을 트리로 두는 도구는 닫힌 칸의 공간을 **같은 부모 안의 형제**에게 준다. 화면상 위치(기하)는 보지 않는다. 조사한 도구의 다수가 이 계열이다. 단 "거의 전부"는 아니다 — 기하 기준 계열(F3)과 목록 재배치 계열(F3b)이 따로 있다(적대 리뷰 지적).

- 이진 트리 + 형제가 부모 사각형 전체를 가져감: WezTerm(`mux/src/tab.rs` `unsplit_leaf`), Windows Terminal(`Pane.cpp` `_CloseChild`), Ghostty(`split_tree.zig`), kitty Splits(`splits.py`), JetBrains(`EditorWindow.kt` `removeFromSplitter`), Terminator, Tilix, bspwm(`tree.c` `unlink_node`), Hyprland dwindle, herdr(`src/layout.rs` `remove_pane`), react-mosaic ≤v6.
- n-ary 트리 + 같은 컨테이너 안 형제에게: tmux, Vim/Neovim, Emacs, i3/sway, VS Code 에디터 그리드, dockview, FlexLayout, Golden Layout, Lumino, KDDockWidgets.
- 로컬 확인: tmux(클론 5eaf557, 2026-06-14) `layout.c:541` "Merge the space into the previous or next cell" — 앞 형제, 맨 앞이면 뒤 형제. 3.5a도 같다. **현재 master는 뒤 형제를 선호하도록 바뀌었다**(적대 리뷰가 소스 링크로 제시 — 메인 미확인). 어느 쪽이든 트리 형제다.

**우리 동작은 이 지배 패턴과 같다.** 사용자가 본 "무조건 가로"는 결함이 아니라 이 패턴 + 우리 그리드의 분할 순서의 결과다.

### F2. n-ary 트리에서의 분배 규칙은 다섯 갈래 — 가능성 높음

1. 인접 형제 하나가 전부(tmux, Emacs 기본, VS Code `auto`에서 크기가 고르지 않을 때, KDDockWidgets는 양옆에 반반)
2. 현재 크기 비례(i3, FlexLayout, Golden Layout, Emacs `window-combination-resize`)
3. 균등 가산(react-mosaic v7, Lumino)
4. 전부 균등화(Vim `equalalways` 기본 켜짐, dockview, VS Code `distribute`)
5. 이진 트리는 형제가 하나뿐이라 선택지가 없음

우리는 이진 트리라 5에 해당한다. 크기 분배 정책은 병합 방향과 별개 축이다.

### F3. 기하 기준 계열 = Zellij·Sublime Origami. Zellij는 가로 병합 우선이 고정이고, 사용자 불만이 있다 — 확실

- Zellij는 트리가 없고 평면 기하를 쓴다. 일반 칸을 닫을 때 경계가 맞는 이웃을 **왼쪽 → 오른쪽 → 위 → 아래** 순서로 찾는다(로컬 클론 bf8d23a `tiled_pane_grid.rs:1277-1287` 확인). 2×2의 일반 칸이면 분할 순서와 무관하게 가로 병합이다. **단 쌓인(stacked) 칸은 별도 경로이고, 고정 크기 칸은 제거 뒤 전체 재배치로 빠진다** — "항상"은 일반 칸 한정이다(적대 리뷰 지적).
- Sublime Text Origami 플러그인 `destroy_current_pane`도 트리 없이 **경계가 맞는 인접 칸**을 골라 흡수시킨다(격자 셀 모델 — 적대 리뷰가 소스 링크로 제시, 수집자 보고와 일치).
- 그 고정 우선순위에 대한 불만이 열려 있다: zellij #1772 "Select pane orientation priority"(open) — 좌우 먼저 가르고 각 칸을 위아래로 가른 뒤 닫으면 "zellij will prioritize horizontal over vertical"이라며 선택 옵션 또는 여닫은 순서 기억을 요청(GitHub API로 본문 확인). discussion #2419도 "이전 상태로 돌아가길 기대"(수집자 보고, 본문 미확인).

즉 **우리 사용자와 같은 불만이 기하 기준 도구에서도 나온다.** "가로 우선"을 기하로 고정하면 해결이 아니라 불만의 방향만 바뀐다.

### F3b. 목록 재배치 계열 — 이웃이 흡수하지 않고 전체를 다시 짠다 — 가능성 높음

Qtile Matrix, awesome tile처럼 창을 **목록 순서**로 두는 레이아웃은 닫힌 칸을 누가 흡수하는지 정하지 않는다. 남은 창 목록으로 레이아웃 전체를 다시 계산한다(적대 리뷰가 소스 링크로 제시, 메인 미확인). xmonad·dwm 계열도 같은 부류로 보이나 미조사다. 사용자가 자유롭게 분할한 트리를 다루는 우리 모델과는 전제가 다르다.

### F4. "분할 전 상태로 복원"이 명시된 설계 원칙이다 — 확실

- Emacs Lisp 매뉴얼 "Recombining Windows": 분할 뒤 새 창을 지우면 "reestablishes the layout ... as it existed before the splitting"(gnu.org 원문 확인). 메커니즘 = `window-combination-limit`.
- Vim `win_altframe` 주석: `splitbelow`/`splitright`에 따라 받을 창을 바꾸는 이유 = "opening a window and then immediately closing it will preserve the initial window layout"(수집자 보고, Vim·Neovim 소스).
- VS Code #187431: "split editor right" 뒤 가운데 그룹을 닫으면 **나눠 온 쪽이 아니라 반대쪽과 병합**되는 것을 사용자가 버그로 보고했다(이슈 본문 GitHub API 확인). 사용자 기대가 "분할 전으로"라는 증거다. **단 그 수정(PR #188898)은 분할 이력을 기억하는 방식이 아니다** — 크기 분배의 기준 칸을 인덱스상 인접 칸으로 바꾼 것이다(적대 리뷰가 diff로 지적). 이 사례를 "이력 복원 메커니즘"의 선례로 읽지 않는다.

이진 트리 + 형제 흡수는 **분할을 하나씩 해 온 경우 이 원칙을 자동으로 만족한다**(닫으면 그 분할 직전으로 돌아감). 깨지는 것은 ① 프리셋처럼 한 번에 만든 그리드 ② 사용자가 "나눈 순서"와 다른 방향의 병합을 원할 때다.

### F5. 닫을 때 흡수할 이웃을 사용자가 고르는 기능은 거의 없다 — 가능성 높음

- tmux #1499: `kill-pane`에 흡수할 창을 지정하는 플래그를 요청했고, 메인테이너가 "Adding a flag to kill-pane is OK by me"라고 한 뒤 닫혔다(GitHub API로 제목·상태 확인). 로컬 클론의 `kill-pane`에는 그런 플래그가 없다(수집자 보고 — 사용법 `[-a] [-f filter] [-t]`).
- Sublime Text Origami 플러그인 `destroy_pane(direction)`: 방향은 **지울 칸**을 가리키고, 현재 칸이 그 자리로 넓어진다. 즉 "이 칸을 닫고 저쪽이 흡수"가 아니라 **"저쪽 칸을 지우고 내가 흡수"**다(적대 리뷰가 소스로 정정 — 수집자 보고는 의미를 반대로 적었다). 방향 선택이 사용자 손에 있다는 점은 같다.
- VS Code `removeView(view, sizing)`의 인자는 **크기 분배**를 고르는 것이지 흡수 이웃을 고르는 것이 아니다(`grid.ts:450` 확인).

## 우리 쪽 선택지 (제약 적합도)

제약: 레이아웃 권위 = 백엔드(ADR-0035) · 이진 트리 · 모든 기능 LLM 제어 가능(CLAUDE.md 「LLM-우선 제어」) · 드래그 크기는 백엔드에 안 돌아감(ADR-0063).

| 선택지 | 무엇 | 피어 | 적합도 | 비용 |
|---|---|---|---|---|
| A. 현행 유지 | 형제 흡수. 방향 = 분할 순서 | 지배 패턴 전부 | 높음. 트리 모델 그대로 | 0 |
| B. 기하 고정 우선순위 | 경계가 맞는 이웃을 고정 순서로 | Zellij, Origami(`destroy_current_pane`) | 낮음. 이진 트리에 재구성 로직이 필요하고, Zellij에서 같은 불만 존재(#1772) | 중 |
| C. 방향 지정 병합 | 사용자가 방향을 고른다. 기본 닫기는 A 유지. 형태 둘: C1 "이 칸을 닫고 ←/↑ 이웃이 흡수", C2 "← 이웃을 지우고 내가 흡수"(Origami `destroy_pane`) | tmux 요청(수락 뒤 미출시), Origami | 중. LLM 명령 인자로 자연스럽다. 단 흡수 이웃이 트리 형제가 아니면 경계가 맞는 경우에 한해 트리를 재구성해야 함 | 중~상 |
| D. 닫은 뒤 균등화 | 크기 정책(방향과 별개) | Vim 기본, dockview | 방향 문제를 풀지 않음 | 하 |
| E. 전체 재배치 | 남은 칸 목록으로 격자를 다시 짠다 | Qtile Matrix, awesome tile | 낮음. 사용자가 자유 분할한 모양을 버린다 | 중 |

- 거부 후보(→ ADR 거부 대안 후보): **B** — 기하 계열 피어(Zellij)에서 같은 방향 불만이 열려 있다. 고정 우선순위는 불만의 방향만 바꾼다. **E** — 자유 분할 트리라는 우리 모델의 전제를 버린다.

## 쟁점·한계

- tmux 방향 세부는 버전에 따라 다르다: 3.5a·로컬 클론(2026-06) = 앞 형제, 한 수집자가 본 최신 master = "뒤를 선호"(미확인). 결론(트리 형제)에는 영향 없음.
- iTerm2·Konsole·QSplitter의 크기 분배 세부는 미확인. Sublime 본체의 그룹 닫기 동작 미확인.
- UX 원칙을 다룬 학술·가이드 문헌은 찾지 못했다.
- C의 트리 재구성(경계가 맞는 2×2를 좌우 먼저 ↔ 위아래 먼저로 바꾸기)은 선례 코드를 찾지 못했다. 설계 필요.
