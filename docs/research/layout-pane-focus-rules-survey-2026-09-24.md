# 분할 레이아웃의 포커스 규칙 — 닫은 뒤 포커스와 조작용 패널 피어 서베이

- 상태: 확정 조사 · 결정 = 미정 · 날짜: 2026-09-24 · 강도: medium · 설계-결정 모드
- 방법: 주계열(Claude) 수집자 3 갈래(터미널·멀티플렉서·타일링 WM / 에디터·IDE·도킹 라이브러리 / 조작용 패널과 작업 대상 칸) + 셋째 갈래를 Sonnet 으로 한 번 더(파일럿 — 결과 비교는 research 스킬 feedback) + 메인 grounding(로컬 클론 tmux·zellij 코드, VS Code `editorPart.ts`·Zed `workspace.rs`·Emacs `window.el` 원문 대조) + cross-family(codex, effort high, 웹 검색) 적대 리뷰 1 회 — 판정 REFUTE-PARTIAL, 지적 6 건 전부 반영(누락 3 · 논리 공백 2 · 출처 누락 1)
- 확신도 범례: 확실 = 독립 출처 2개 이상 또는 코드 직접 확인 / 가능성 높음 = 단일 출처로 지지 / 불확실 = 미지지·추론

## 질문

1. 포커스된 칸을 닫으면 포커스는 어디로 가나.
2. 트리·사이드바 같은 조작용 패널이 "작업 대상 칸"(열기·생성이 들어가는 칸)이 될 수 있나.

## 우리 현황 (코드 확인)

- 포커스 포인터는 뷰마다 하나(`focused_slot_id`, 백엔드 소유 — ADR-0035).
- 클릭 포커스는 콘텐츠 칸(`empty`/`agent`)만 받는다 — 제어 칸(`agent_list`/`preset_palette`)은 프론트가 건너뛴다(ADR-0073).
- 트리의 「열기」는 포커스가 콘텐츠 칸이면 그 칸, 아니면 첫 빈 칸, 없으면 안내(`src/components/agent/selectOpenTarget.ts`).
- 포커스된 칸을 닫으면 트리의 **첫 칸**으로 간다(`src-tauri/src/layout/manager.rs` `fixup_focus`). 첫 칸이 트리면 포커스 링이 트리에 뜬다(ADR-0073 이 보류한 외형 잔존).
- LLM 의 명시 포커스(`slot.focus`)는 제어 칸도 받는다.

## 발견

### F1. 닫은 뒤 포커스 — 업계 공통 규칙은 없고 네 계열이다 — 확실

1. **직전 포커스 칸(MRU / 포커스 이력)** — 에디터·멀티플렉서의 다수.
   - tmux: 창별 직전 칸 스택의 머리, 비면 앞 칸 → 뒤 칸(로컬 클론 5eaf557 `window.c` `window_lost_pane` — 확인).
   - zellij: 남은 칸 중 마지막 활성 시각이 가장 늦은 칸(로컬 클론 bf8d23a `tiled_panes/mod.rs` `move_clients_out_of_pane` — 확인).
   - VS Code 에디터 그룹: 직전 활성 그룹 `MOST_RECENTLY_ACTIVE[1]`(`editorPart.ts` — 확인).
   - Emacs: `delete-window-choose-selected` 기본값 `'mru`(`window.el` — 확인). 설정으로 `'pos`(위치)·`nil`(첫 창)도 고를 수 있는 유일한 사례.
   - kitty(그룹 이력 deque) · bspwm(데스크톱별 이력) · herdr(직전 포커스 한 칸) · Hyprland `focus_on_close=2`(수집자 보고).
   - i3·sway: 전역 MRU 가 아니라 **가장 가까운 살아남은 조상 안에서의 MRU**(계층형 — 수집자 보고).
2. **공간을 흡수한 형제 / 트리 순서** — Vim·Neovim 분할(흡수한 창이 current), JetBrains 에디터 분할(형제 중 첫 창), Windows Terminal(흡수한 형제 → 그 첫 잎), WezTerm·Ghostty·iTerm2(트리 순서상 앞 칸, 없으면 뒤 칸)(수집자 보고).
3. **포인터(커서) 아래 창** — Hyprland `input:focus_on_close=1`(공식 옵션 문서 — 적대 리뷰가 누락으로 지적). 마우스 중심 WM 의 계열이다.
4. **첫 칸, 또는 후속 없음** — 웹 도킹 라이브러리. dockview(남은 첫 그룹 — `baseComponentGridview.ts` `doRemoveGroup`·`dockviewComponent.ts` `activateFallbackGroupIfRemoved`, 수집자 소스 판독 · 공식 문서는 활성 그룹이 없을 수 있다고만 적는다 — 가능성 높음) · FlexLayout(활성 탭셋이 없어짐) · Golden Layout(포커스 해제)(수집자 보고). **우리 현행이 이 계열이다.**

- **포커스 없는 칸을 닫으면 포커스는 그대로** — 확인한 모든 구현이 「닫힌 것 == 활성」일 때만 후속을 고른다(수집자 보고, tmux 코드로 확인).
- **MRU 구현은 전부 이력이 비면 위치 규칙으로 떨어진다**(앞 → 뒤, 또는 첫 잎).
- **사용자 불만** — Neovim #20455 「창을 닫으면 직전 창으로 가게 해 달라」(흡수 형제 규칙에 대한 MRU 기대 — 수집자 보고). Ghostty discussion #8831 「분할을 닫으면 포커스가 아무 데도 없는 경우」.

### F2. 패널을 따로 두는 도구는 조작용 패널을 작업 대상에서 뺀다 — 포인터를 둘로 나눈다 — 가능성 높음

★**「어느 도구도 패널을 작업 대상으로 삼지 않는다」가 아니다**★ — 반례 계열이 있다(적대 리뷰 지적 + Sonnet 파일럿 수집): **브라우저를 평범한 버퍼로 두고, 열면 그 자리를 대체하는 모델** — Vim netrw 기본값(선택한 파일이 브라우저 창을 대체), oil.nvim, neo-tree `window.position = current`. ★**우리 앱에서 이 모델은 이미 기각됐다**★ — 트리 칸이 「열기」로 에이전트 화면에 덮이는 것을 ADR-0073 이 결함으로 보고 막았다.

패널을 따로 두는 도구는 **키보드 포커스**(트리 포함 어디든)와 **작업 대상 칸**(열기가 들어가는 칸)을 따로 둔다. 작업 대상 칸은 **콘텐츠 칸에 포커스가 갈 때만 움직인다** — 트리를 눌러도 그대로이고, 트리의 「열기」는 그 칸으로 간다.

- VS Code: `activeGroup` = "An active group is the default location for new editors to open". 탐색기는 그룹이 아니다(수집자 보고, 문서).
- Zed: 포커스는 도크 패널에도 가지만, 열기는 `last_active_center_pane`(마지막으로 포커스된 중앙 칸)으로 간다(`workspace.rs` — 확인).
- JetBrains: `FileEditorManagerEx.currentWindow` — 도구 창은 에디터 창이 아니다(수집자 보고).

트리를 **일반 분할 안에** 두는 경우는 자격 필터로 막는다 — 우리 모양과 가장 가깝다.

- NERDTree: 직전 창이 「쓸 수 있으면」 거기, 아니면 쓸 수 있는 첫 창, 없으면 새 분할.
- nvim-tree: 기본은 창 고르기(window picker), `eject` 로 트리 창에는 안 연다. 터미널·도움말 등은 후보에서 뺀다.
- neo-tree: `open_files_in_last_window`, `open_files_do_not_replace_types = { "terminal", … }`.
- Neovim `winfixbuf` · VS Code 「잠긴 그룹」: 콘텐츠 칸도 명시 지정 없이는 대상이 안 되게 잠글 수 있다.
- Emacs: 사이드 창은 기본이 전용(dedicated)이고 `no-other-window` 로 순회에서 빠진다.

- **작업 대상만 따로 표시하는 링은 드물다** — VS Code 는 활성 그룹의 탭 색을 달리한다(테마 토큰 — 의미 세부 미확인). tmux 의 「표시된 칸(marked pane)」이 가장 가까운 선례다(수집자 보고).

## 우리 쪽 선택지 (제약 적합도)

제약: 포커스는 백엔드 권위(ADR-0035) · LLM 이 같은 핸들을 흔든다(「LLM-우선 제어」) · 「열기」는 칸 내용을 **바꾼다**(에이전트 프로세스는 살지만 그 칸의 화면이 바뀌고, 죽은 에이전트의 보존 화면이면 사라진다 — ADR-0148).

| 선택지 | 닫은 뒤 포커스 | 제어 칸 | 피어 | 비용 | 위험 |
|---|---|---|---|---|---|
| A. 현행 유지 | 첫 칸(트리일 수 있음) | 클릭만 배제, LLM 명시는 허용 | 웹 도킹 라이브러리 | 0 | 포커스 링이 트리에 뜬다 + 그때 「열기」는 작업하던 칸이 아니라 첫 빈 칸(없으면 안내)으로 간다 — 순수 외형이 아니다 |
| B. 첫 **빈** 칸 → 없으면 포커스 없음 | 빈 칸만 | 백엔드도 배제(명시 포커스 거절) | nvim-tree 자격 필터와 결이 같음 | 하 | 빈 칸이 없으면 포커스가 사라져 방향 이동(`left` 등)의 기준점이 없어짐 |
| C. MRU(직전 콘텐츠 칸) → 없으면 첫 빈 칸 → 없음 | 직전에 쓰던 칸 | 이력에 안 들어감 + 명시 포커스 거절 | tmux·zellij·VS Code·Emacs 기본 | 중(뷰별 이력 유지·정리) | 직전 칸이 에이전트 칸이면 「열기」가 그 칸 화면을 바꾼다 — 오늘 되돌린 회귀와 같은 모양이다. ★VS Code 와 같다고 볼 수 없다★ — VS Code 의 「열기」는 그룹에 탭을 **더하고**(그룹을 닫을 때도 에디터를 다른 그룹으로 옮긴다) 우리는 칸 내용을 **바꾼다** |
| D. 포인터 둘(키보드 포커스 / 작업 대상) | 작업 대상은 C 규칙 | 키보드 포커스는 트리도 가능, 작업 대상은 콘텐츠만 | VS Code·Zed·JetBrains | 상(와이어·프론트·LLM 표면 전부) | C 의 위험을 그대로 물려받는다 + 표면이 둘로 늘어 LLM 설명·링 표시를 다시 짜야 함 |

- **패널을 따로 두는 피어는 「제어 칸은 작업 대상이 아니다」가 같다**(브라우저-대체 모델은 ADR-0073 이 이미 기각) — LLM 명시 포커스가 제어 칸을 받는 현행은 그 합의에서 벗어난다.
- 거부 후보(→ ADR 거부 대안 후보): **흡수 형제 규칙** — 우리 트리는 형제가 트리 칸일 수 있어 자격 필터가 어차피 필요하고, 그 계열(Neovim)에 MRU 를 원하는 불만이 있다.

## 쟁점·한계

- tmux `window_lost_pane` 줄 번호가 수집자마다 다르다(로컬 클론 845 대 master 1138) — 버전 차이. 규칙은 같다.
- Hyprland 기본값(0 = 「다음 후보」)의 레이아웃별 의미, kitty 의 비활성 그룹 제거 가장자리, Ghostty GTK 에서 비포커스 칸을 닫을 때 포커스 이동 여부는 미확인.
- Sublime Text·react-mosaic 는 조사하지 못했다(기권). 파일 브라우저 계열은 netrw·oil.nvim·neo-tree 까지만 봤다(적대 리뷰가 추가한 둘은 메인 미확인).
- 모든 동작은 코드·문서 읽기 기반이고 실제 실행 관측은 없다.
