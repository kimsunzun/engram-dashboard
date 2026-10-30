# 분할 트리 레이아웃에서 칸을 위치로 지정하는 방식 — 2026-10-26

- **상태:** light 요약 · 결정 없음 — T-65(LLM 레이아웃 명령) 의 입력. 앞선 조사 = `llm-command-design-query-vs-action-2026-10-26.md`.
- **방법:** `/research` light(사용자 지정 「간단하게」) · 서브클래스 engram · 수집자 1 명(검색 7 회) · 메인 grounding(tmux · ace-window · niri 원문 확인). 적대 리뷰 없음(light).
- **질문:** 사용자 제안 「왼쪽부터 x1 y2 식 좌표」 — 격자가 아닌 분할 트리에서 다른 도구는 칸을 어떻게 위치로 지정하나.

## 결론

**본 도구들 중 「열 N · 행 M」 좌표로 분할 트리의 칸을 지정하는 곳은 없었다.** 대신 넷을 쓴다 — ① 모서리·가장자리·방향 토큰 ② 순번(만든 순서 또는 읽는 순서) ③ 화면에 그린 번호·글자를 사람이 보고 고르기 ④ 이름·표식. 확신도 = 가능성 높음(표본 4 도구 · 원문 확인 2).

| # | 클레임 | 출처 | grounding | 확신도 |
|---|---|---|---|---|
| 1 | tmux 칸 번호는 위치가 아니라 **만든 순서**다 — "Panes are numbered beginning from zero in the order they are created." | [tmux(1)](https://man7.org/linux/man-pages/man1/tmux.1.html) | 지지(메인 확인) | 가능성 높음 |
| 2 | tmux 공간 토큰 = 모서리 넷(`{top-left}` …) · 가장자리 넷(`{top}` · `{bottom}` · `{left}` 가장 왼쪽 · `{right}`) · 지금 칸 기준 이웃 넷(`{up-of}` · `{left-of}` …) · `{next}`/`{previous}`(번호순) · `{marked}`. 열·행 좌표는 없다. **우리 토큰(모서리 4 + 방향 4)과 거의 같은 모양**이고 tmux 에는 가장자리 넷이 더 있다. | 같은 곳 | 지지(메인 확인) | 가능성 높음 |
| 3 | tmux `display-panes` 는 칸마다 번호를 그리고 0~9 키나 클릭으로 고르게 한다(사람이 보고 고르는 방식 · 10 칸까지). | 같은 곳 | 지지(메인 확인) | 가능성 높음 |
| 4 | ace-window(Emacs)는 창마다 글자를 그리고 **위→아래, 왼→오 순**으로 매긴다 — 배치를 기억하면 글자를 안 봐도 고를 수 있다. 불규칙 배치에 결정적 위치 순번을 매기는 가장 가까운 선례. | [ace-window](https://github.com/abo-abo/ace-window) | README 문장 지지(메인 확인) · ★문서 리뷰(2026-10-26)가 소스 `aw-window<` 는 왼쪽 경계를 위보다 먼저 비교한다(= 열 우선)고 지적 — 메인 미확인★ | 불확실 |
| 5 | i3 는 방향 이동(「그 방향의 가장 가까운 컨테이너」) · 표식(`mark`) · 조건으로 지정하고 순번이 없다. | [i3 user guide](https://i3wm.org/docs/userguide.html) | 수집자 요약 | 가능성 높음 |
| 6 | WezTerm `PaneSelect` 는 칸마다 1~2 글자를 그리고 사람이 친다(옮기기 · 바꾸기 모드 포함). 스크립트는 어느 글자가 어느 칸인지 모르면 못 쓴다. | [WezTerm PaneSelect](https://wezterm.org/config/lua/keyassignment/PaneSelect.html) | 수집자 요약 | 가능성 높음 |
| 7 | niri · VS Code 의 「N 번째 열 / N 번째 편집기 그룹」 은 **미확인**이다(수집자 기억 · 메인이 본 niri 키 바인딩 위키에는 `focus-column-left/right` 같은 방향 동작만 있었다). niri 는 분할 트리가 아니라 「열의 줄」 모델이라 열 번호가 잘 정의된다(추론). | — | 미확인 | 불확실 |

- **여러 이웃 중 어느 것을 고르나(방향 이동의 동률 규칙)는 tmux · i3 · WezTerm 문서 어디에도 없었다**(소스는 안 읽음). 우리는 「가장 많이 맞닿은 칸」으로 정해 두었다(`src-tauri/src/layout/spatial.rs` 의 `neighbor_in_dir`).

## 우리에게 옮기면 (추론 · 결정 아님)

- **열·행 좌표는 선례가 없고, 트리에서 「행」이 애매한 문제를 푼 곳도 못 찾았다.** 그 자리를 다른 도구는 순번 + 화면 표시로 메운다.
- 선례에서 나오는 선택지:
  1. **읽는 순서 순번**(ace-window) — 위→아래, 왼→오. 우리 코드에 이미 있다(`spatial.rs` 의 중심점 정렬 `ordinal` · 낱말로는 안 받음). 비대칭 분할에선 「한 열」이 끊길 수 있다고 그 모듈 머리 주석이 적어 둔다.
  2. **화면 번호를 사람과 LLM 이 함께 쓰기**(tmux `display-panes`) — 창 번호 표식(ADR-0300)과 같은 모양.
  3. **가장자리 토큰 넷 추가**(tmux `{top}` · `{left}` …) — 싸게 표현력을 늘린다.
  4. 그래도 모르면 조회 — 지금 순번을 돌려주고 LLM 이 고른다.

## 공백

- niri · VS Code · Hyprland · Zellij · PaperWM 미확인.
- 방향 이동의 동률 규칙 — 문서에 없음, 소스 안 읽음.
- tmux `display-panes` 번호가 칸 번호와 같은지 원문 미확인.
