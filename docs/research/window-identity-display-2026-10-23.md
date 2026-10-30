# 창 식별 표시 · 주 창 구분 — 관행 조사 (2026-10-23)

> **상태:** 조사 끝 · 적대 리뷰 FIX 반영 · **결정됨(사용자 결정 2026-10-23)** — 번호만 보이는 고정 표식을 탭줄 오른쪽 끝(창 단추 바로 왼쪽)에 늘 둔다 · 모든 창 번호 · 메인 = 1 · OS 제목 · 작업 표시줄 · Alt+Tab = 「<탭 이름> - Engram:<n>」 · 메인 구분은 번호 1 + 메인 ✕(모든 창 숨기기)로 채운다(메인만 따로 꾸미지 않는 것은 메인 제안) → PRD `docs/process/S21-window-chrome/prd.md` 0.4판 R24 · R3 · Q3. T-66(S26) PRD Q3(번호를 어디에 보이나) · 「main 과 팝아웃 구분」의 근거. **방법:** `/research medium` — 수집 갈래 둘(Claude) · 메인 grounding(원문 확인 5 건 — 아래 「메인 확인」) · cross-family 적대 리뷰 1 회(codex · FIX 9 건 → 반영). **확신도:** 확실 = 1차 문서를 메인이 확인 · 가능성 높음 = 출처가 받치나 단일 · 검색 요약 · 불확실 = 받침 약함 · 모름 = 못 찾음. 「못 찾았다」는 표본에서 못 찾았다는 뜻이지 없다는 증명이 아니다.

## 결론

1. **창 번호는 제목 줄에 붙이는 선례가 있다.** Excel 은 같은 문서의 둘째 창을 `Book1.xls:2` 로 제목에 늘 붙인다(문서별 창 번호 — 앱 전체 창 번호는 아니다). iTerm2 는 설정으로 제목에 창 번호를 넣고 ⌘⌥N 으로 그 창에 간다. Windows Terminal 은 늘 보이지 않고 요청하면(`identifyWindow` · `identifyWindows`) 이름 · index 를 덮개로 띄운다. — 확실
2. **탭줄 안 왼쪽에 창을 가르는 표식을 두는 선례가 하나 있다 — Vivaldi 워크스페이스.** 탭줄 왼쪽에 워크스페이스 단추(아이콘 + 이름 · 이름은 숨길 수 있다)를 두고, 한 워크스페이스는 한 번에 한 창에서만 열린다 — 사실상 그 창의 표식이다. 단 영구 창 id 가 아니라 사용자가 만든 이름이다. — 확실
3. **탭줄에 숫자 번호 배지를 늘 두는 피어는 못 찾았다.** WezTerm 은 탭줄 상태 영역 · 창 id API 로 만들 수는 있다(구현 가능성 추론 — 기본 기능 · 실사용 확인 없음). — 가능성 높음
4. **창 이름은 주로 OS 쪽(제목 · 작업 표시줄 · Alt+Tab)에 둔다** — Chrome · Edge 「창 이름 지정」은 작업 표시줄 · Alt+Tab · VS Code · JetBrains 는 제목에 작업 공간 · 프로젝트 이름 · Notepad++ 는 `-titleAdd` 로 제목에 식별 문자열. Edge 워크스페이스는 이름 · 테마를 가진 창을 만든다. — 가능성 높음
5. **주 창을 색 · 배지로 꾸며 가르는 피어는 못 찾았다. 주 창이 있는 앱은 기능 · 수명으로 가른다** — 주 창만 전체 메뉴 · 전역 명령을 갖고(JetBrains 떼어 낸 편집 창엔 메인 메뉴가 없다 · Unreal) 주 창을 닫으면 다른 창도 닫힌다(Unreal · Blender 자식 창). 브라우저 · 터미널은 모든 창이 대등하다. — 가능성 높음
6. **Visual Studio 의 「소유된 창」은 기본으로 도구 창만이다** — 기본값 `Tool Windows` 에서 도구 창은 주 창에 묶이고(작업 표시줄 단추 없음 · 같이 최소화 · 늘 위) 문서 창은 독립 창처럼 뜬다. — 확실
7. **창 수에 따라 보였다 숨는 창 번호는 못 찾았다.** 조건부 표시는 Chrome 프로필 아바타(프로필 2 개 이상)뿐이다. — 가능성 높음

## 피어별

| 피어 | 무엇을 · 어디에 · 언제 | 출처 | 확신도 |
|---|---|---|---|
| Excel | 같은 문서의 새 창 = `Book1.xls:2` — 「window number」, 대부분의 창은 1. index(창 모음 안 자리)와 다르다 | https://learn.microsoft.com/en-us/office/vba/api/excel.window.windownumber (메인 확인) | 확실 |
| IronCAD INOVATE | 같은 장면의 창을 제목에서 `My Project:1` · `:2` 로 가른다 | https://www.ironcad.com/support/INOVATE_Help/Content/Understanding_the_Basics/Menus.htm (리뷰어 제시 · 메인 미확인) | 가능성 높음 |
| iTerm2 | 「window titles include the window number. You can navigate to a window by pressing cmd-opt-N」 · 기본값 · 번호 재사용은 문서에 없다 | https://iterm2.com/documentation-preferences-appearance.html (메인 확인) | 확실(기본값 모름) |
| Windows Terminal | 창 id(정수) · 이름(선택) — `wt -w <id\|name>`. `identifyWindow` = 포커스 창에 이름 · index 덮개 · `identifyWindows` = 모든 창(기본 단축키 없음). 늘 보이는 자리는 못 찾음 | https://learn.microsoft.com/en-us/windows/terminal/customize-settings/actions (메인 확인) | 확실 |
| macOS Terminal | 제목 선택지에 창 번호 대신 「Command key」(그 창으로 가는 키) | https://support.apple.com/guide/terminal/trmlwindw/mac (검색 요약) | 가능성 높음 |
| Vivaldi | 워크스페이스 단추 = 탭줄 왼쪽(세로 탭줄은 목록 맨 위) · 이름 숨김 가능(아이콘만) · 「A workspace can be open in one browser window at a time」 · 제목 = 활성 탭 제목 · 창 이름 기능 없음 | https://help.vivaldi.com/desktop/tabs/workspaces/ (메인 확인) · forum.vivaldi.net | 확실 |
| Chrome · Edge | 「창 이름 지정」 → 작업 표시줄 · Alt+Tab(Mac = Window 메뉴) · 사용자가 켤 때만 · 탭줄 표시 여부 모름 · 프로필 아바타는 프로필 2 개 이상일 때 · Edge 워크스페이스 = 이름 · 테마를 가진 창(왼쪽 위 메뉴로 접근) | androidcentral · microsoft.com/edge · https://support.microsoft.com/en-us/edge/getting-started-with-microsoft-edge-workspaces (리뷰어 제시 · 메인 미확인) | 가능성 높음 |
| Firefox | 기본 창 이름 없음 · 확장 API `titlePreface` 가 OS 제목 앞에 붙인다 | connect.mozilla.org(검색 요약) | 가능성 높음 |
| Notepad++ | 명령줄 `-titleAdd` 로 제목에 식별 문자열 추가 | npp-usermanual command-prompt(리뷰어 제시 · 메인 미확인) | 가능성 높음 |
| WezTerm | 탭줄 오른쪽 상태 영역(`set_right_status`) · 창 id 조회(`window_id`) — 번호 배지는 만들 수 있다는 추론뿐 | https://wezterm.org/config/lua/window/set_right_status.html (리뷰어 제시) | 불확실(기본 기능 아님) |
| VS Code | `window.title` 템플릿(기본 `${activeEditorShort}${separator}${rootName}${separator}${appName}`) — 정체 = 작업 공간 이름 · 번호 없음 · 보조 창은 부모 인스턴스에 딸린 가벼운 창 | https://code.visualstudio.com/docs/configure/custom-layout | 템플릿 확실 · 보조 창 불확실 |
| JetBrains | 제목 = 프로젝트 · 파일 · 떼어 낸 편집 창엔 메인 메뉴가 없고 「원래 창을 닫으면 메뉴가 사라진다」 | intellij-support 포럼 답(직접 열기 403 — 검색 요약) | 가능성 높음 |
| Unreal Editor | 주 창 메뉴가 전역 항목을 갖는다 · 주 창을 닫으면 다른 창이 다 닫힌다 · 떠 있는 탭 창 크롬 세부는 미확인 | 2차 문서(검색 요약) | 가능성 높음 |
| Visual Studio | 「Floating Windows」 설정 — None · Tool Windows(기본 — 도구 창만 소유, 문서 창은 독립) · Documents and Tool Windows. 소유된 창 = 작업 표시줄 단추 없음 · 같이 최소화 · 늘 위 | https://devblogs.microsoft.com/visualstudio/take-full-control-of-your-floating-windows-in-visual-studio (메인 확인) | 확실 |
| Blender | 떼어 낸 자식 창은 원래 창을 닫으면 닫힌다 · 「New Main Window」는 독립 부모 | developer.blender.org T94346(검색 요약) | 가능성 높음 |
| Dear ImGui | `ConfigViewportsNoDecoration` 은 **보조** viewport 에 걸리는 설정이다(주 · 보조 꾸밈이 같다는 근거 아님) | imgui.h docking(리뷰어 제시) | 가능성 높음 |

- **tmux · zellij 는 뺐다** — 그 「window」 · 「tab」은 GUI 앱의 탭에 해당해 최상위 창 번호의 선례로 셀 수 없다(리뷰 지적).

## 우리에게 뜻하는 것 (메인 해석 — 피어 사실 아님)

- **번호를 제목에 붙이는 것** = Excel(`:2`) · iTerm2 선례가 있다.
- **탭줄 왼쪽에 창 표식을 두는 모양** = Vivaldi 워크스페이스 단추가 가장 가까운 선례다(아이콘 + 숨길 수 있는 이름). 숫자 배지 자체의 선례는 못 찾았다.
- **main 구분**은 피어처럼 기능 · 수명으로 이미 갈린다(main 닫기 = 전부 숨김 · 전역 진입점) — 장식을 따로 둘지는 사용자 결정.

## 공백

- Windows Terminal 덮개가 얼마나 떠 있는지 · Chrome · Edge 이름이 탭줄에 나오는지 · 파일 탐색기 탭 · iTerm2 번호 재사용 · VS Code 보조 창의 메뉴 · 제목 줄 · Photoshop · Arc · Zed · Kitty · Ghostty.
- 리뷰어가 제시한 반례 URL 중 INOVATE · Edge 워크스페이스 · Notepad++ · WezTerm · ImGui 헤더는 메인이 다시 열어 보지 않았다.

## 리뷰 (codex · 2026-10-23 · FIX)

- 반영 9 건 — 결론 1 반례(Excel · INOVATE) · Vivaldi 누락(결론 2 신설) · WezTerm 은 추론으로 한정 · Visual Studio 소유 범위 과장 정정(기본 = 도구 창만) · ImGui 인용 오귀속 정정 · tmux · zellij 비교 단위 혼동 → 뺌 · 덮개 패턴을 Windows Terminal 한 곳으로 한정 · Edge 워크스페이스 · Notepad++ 추가 · 「선례 없음」을 「못 찾음」으로 약화.
- 메인 grounding: Windows Terminal · iTerm2 · Visual Studio · Vivaldi · Excel 원문 확인.
