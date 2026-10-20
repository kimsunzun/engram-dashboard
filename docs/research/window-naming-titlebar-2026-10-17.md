# 창 이름 · 창 번호 · 제목 표시줄 — 관행 조사 (2026-10-17)

- **상태:** 조사 완료 · 사용자 결정 일부(아래 「사용자 결정」) · 메인 제안 미확정
- **방법:** research 스킬 light — 수집자 1갈래(공개 문서 · 포럼) → 메인 grounding(우리 코드 `popout.rs` 줄 재열람 · 출처 일부 확인). ★적대 리뷰는 거치지 않았다★ — 아래 확신도는 그 전제로 읽는다.
- **확신도 범례:** 확실 = 독립 출처 둘 이상 또는 코드 · 공식 문서 직접 확인 · 가능성 높음 = 단일 출처로 지지 · 불확실 = 근거 약함 · 모름 = 못 찾음
- **계기:** 팝아웃 창 제목이 「Engram — slot-popup-11」, 팝아웃의 새 탭 이름이 「Popup 6」처럼 계속 커지는 번호로 보여 사용자가 거슬려 했다. 질문 = 「다른 앱은 창 · 탭 이름과 번호를 어떻게 다루나」.

## 결론

1. **내부 id 와 사람에게 보이는 이름을 나누는 것이 일반적이다.** 계속 커지는 전역 번호를 그대로 사람에게 보이는 주류 앱은 표본에서 찾지 못했다. — 가능성 높음
2. **보이는 번호는 「비어 있는 가장 작은 번호」를 다시 쓰는 쪽이 선례가 있다**(tmux 창 index 기본 · Apple HIG 의 「Untitled 2」). — 가능성 높음
3. **창 제목은 「내용 먼저, 앱 이름 뒤」이거나 앱 이름을 뺀다**(Chrome · VS Code · Obsidian). Microsoft 가이드는 순서 · 구분자를 규정하지 않는다. — 가능성 높음
4. **Tauri 에서 제목은 label 과 무관하게 바꿀 수 있다**(`setTitle()`). 그러니 label 을 단조로 두고 보이는 이름만 바꾸는 길이 막혀 있지 않다. — 확실
5. **크롬 · Windows 11 탐색기식 「탭 줄이 제목 표시줄 자리」는 Tauri 에서 직접 그려야 한다**(`decorations: false` + 끌기 영역 + 창 버튼). Windows 11 스냅 레이아웃은 따로 맞춰야 한다. — 가능성 높음

## 1. 우리 사실(코드)

| 항목 | 내용 | 근거 |
|---|---|---|
| 창 label | 단조 카운터 `PopupCounter` · 접두 `slot-popup-` — capabilities 의 `"slot-popup-*"` glob 과 짝 | `src-tauri/src/commands/popout.rs:35-55` |
| label 재사용 금지 | 코드 주석상 닫힌 label 로 다시 빌드하면 에러가 나서 재사용하지 않는다 — ★주석만 읽었고 직접 확인하지 않았다★ | `popout.rs` 의 `PopupCounter` 주석 |
| 창 제목 | `Engram — {label}` | `popout.rs:231` |
| 팝아웃 새 탭 이름 | label 꼬리 번호로 「Popup N」 | `popout.rs:58-61` 의 `tab_name` |
| 이름 바꾸기 | 탭 = 바꿀 수 있다(화면 · `tab.rename`) · 창 = 못 바꾼다 | — |
| LLM 의 창 지정 | `window.list` 로 목록을 받고 label 로 창을 지목한다 | — |

## 2. 관행

| # | 내용 | 출처 | 확신도 |
|---|---|---|---|
| ① | tmux: 창마다 고유 id(`@N`) · 위치 번호(index — 기본 = 비어 있는 가장 작은 번호, `renumber-windows` 로 당겨 매김) · 이름(`rename-window`) 셋을 따로 둔다. 대상 지정은 셋 다 받는다 | https://mintlify.wiki/tmux/tmux/concepts/windows | 가능성 높음 |
| ② | 계속 커지는 전역 번호를 사람에게 보이는 주류 앱 | 표본에서 못 찾음 | 모름(없다는 증명 아님) |
| ③-a | Chrome 창 제목 = 「<활성 탭> - Google Chrome」 | 지식 기반 | 가능성 높음 |
| ③-b | VS Code `window.title` 기본 템플릿 = `${dirty}${activeEditorShort}${separator}${rootName}${separator}${profileName}${separator}${appName}` | VS Code 설정 문서 | 가능성 높음 |
| ③-c | Obsidian 제목 = 노트명 → vault → 앱 | https://forum.obsidian.md/t/option-to-set-window-title-format/46303 | 가능성 높음 |
| ③-d | Apple HIG: 제목이 같으면 2부터 숫자 접미(「Untitled 2」) | https://developer.apple.com/design/human-interface-guidelines/windows | 확실 |
| ③-e | Microsoft 제목 표시줄 가이드는 순서 · 구분자를 규정하지 않는다 | https://learn.microsoft.com/en-us/windows/apps/design/basics/titlebar-design | 확실 |
| ④ | Chrome 은 창 이름 지정 기능(「Name Window…」)이 있다 | https://til.codeinthehole.com/posts/about-chromes-named-windows/ | 가능성 높음 |
| ⑤ | Tauri: `setTitle()` 로 label 과 무관하게 제목을 정한다 · label 은 "unique identifier" | https://v2.tauri.app/reference/javascript/api/namespacewindow/ | 확실 · 닫힌 label 재사용 가능 여부는 모름 |
| ⑥ | 크롬 · Windows 11 탐색기 · Windows Terminal 은 OS 제목 표시줄 대신 탭 줄을 그 자리에 둔다 | 사용자 관찰 + 지식 기반 | 가능성 높음 |
| ⑥-Tauri | `decorations: false` + 끌기 영역(`data-tauri-drag-region`) + 창 버튼 직접 그리기 · Windows 11 최대화 버튼의 스냅 레이아웃은 따로 맞춰야 한다 · 커뮤니티 플러그인 `tauri-plugin-decorum` 이 이를 다룬다고 알려져 있다 | 지식 기반 | 가능성 높음 · 플러그인은 미확인 |

## 3. 사용자 결정 (2026-10-17)

- **보이는 창 번호 = 살아 있는 창 중 비어 있는 가장 작은 번호를 다시 쓴다** — 1 · 2 · 3 에서 2 를 닫으면 새 창 = 2.
- **내부 label 은 계속 단조다.**
- **크롬 · 탐색기식 제목 표시줄** — 탭 줄이 맨 위 · 빈 곳 = 창 끌기 · 창 버튼을 직접 그린다.
- **탭바 넘침 = 크롬과 똑같이** — 스크롤 없음, 최소 폭까지 줄고 그래도 넘치면 안 보인다.
- **시점 = 슬롯 도킹 9c 직후 바로 · 최종 머지에 포함.**

## 4. 메인 제안 (미확정)

- 창 제목 = 「<활성 탭 이름> - Engram」, 탭이 없으면 「Engram 2」(보이는 번호).
- 팝아웃 새 탭 이름 = 옮긴 슬롯의 에이전트 이름, 비었으면 「새 탭」.
- `window.list` 가 보이는 번호와 label 을 함께 주고 명령은 둘 다 받는다 — 번호는 사람 말(「2번 창」) 해석용이고, 에이전트가 기억해 두는 대상은 label 이다(번호는 재사용되므로).

## 5. 공백 · 미확인

- VS Code 「Untitled-N」 번호 재사용 정책
- zellij 의 탭 · 세션 번호 규칙
- Tauri 에서 닫힌 label 로 창을 다시 만들 수 있는지(우리 코드 주석은 「에러」라 적지만 확인 안 함)
- 크롬에서 탭이 넘쳤을 때의 정확한 동작(최소 폭 · 안 보이는 탭의 처리)
