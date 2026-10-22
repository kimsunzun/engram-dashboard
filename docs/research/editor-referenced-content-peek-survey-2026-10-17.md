# ★ 편집 칸을 떠나지 않고 참조한 항목의 내용 보기 — light 조사

| 항목 | 값 |
|---|---|
| 상태 | 조사 완료 · 결정 대기(사용자) |
| 날짜 | 2026-10-17 |
| 방법 | `/research` light — 수집자 1명(검색 · 가져오기 약 10회) · 메인 grounding 스팟체크 2건(VS Code 문서 · Obsidian 문서 WebFetch) · 적대 리뷰 없음 |
| 질문 | 프리셋을 고르면 오른쪽이 편집 칸이 된다. 그 안의 지시서 줄이 가리키는 `.md` 내용을 편집 칸을 떠나지 않고 계속 보려면 — 다른 도구는 어떻게 하나(사용자 착상: 누르면 오른쪽 말고 왼쪽으로 툴팁) |

## 결론

- **가장 흔한 꼴 = 붙어서 잠깐 뜨는 미리보기(hover · peek)** — 참조한 내용을 그 자리 옆에 보여 주고(높이 상한 · 스크롤), 「계속 보기」 는 따로 하는 한 단계다(고정 · 진짜 편집기로 열기 · 도구 창으로). (가능성 높음)
- **띄우는 방법:** 가벼운 미리보기 = 마우스 올리기(편집 중엔 시끄러워서 Ctrl/Cmd + 올리기 — Obsidian 편집 화면 · VS Code) · 무거운 미리보기 = 키(VS Code Alt+F12 · JetBrains Ctrl+Q · macOS Quick Look Space). (확실 — Obsidian · VS Code 원문 확인)
- **「다른 줄을 눌러도 열린 채로 내용이 바뀐다」 는 선택을 따라가는 창에서만 봤다** — macOS Quick Look(떠 있는 창이 선택을 따라 바뀜) · JetBrains 문서 도구 창(탭 고정). (가능성 높음)
- **자리:** 띄운 줄 바로 옆에 붙거나(VS Code 는 편집기 안에 끼워 넣는다 — 아래로 밀어냄), 따로 떠 있는 창(Quick Look). 「늘 정해진 한쪽(목록 위)」 에 띄우는 선례는 못 봤다(불확실 — 못 찾음 ≠ 없음). 다만 우리 편집 칸은 화면 오른쪽 끝이라 **옆에 붙이면 공간 때문에 왼쪽으로 뒤집히는 것** 이 평범한 붙은 팝업의 동작이다(추론).
- ★수집자는 「줄 아래 펼치기는 어디에도 없다」 고 했지만 **VS Code Peek 이 바로 그 꼴이다** — 「we embed the result inline」(원문 확인). 그 요약 문장은 강등.

## 발견

| 도구 | 띄우기 | 무엇 | 자리 | 길이 · 스크롤 | 고정 · 따라가기 | 닫기 | 출처 | 확신 |
|---|---|---|---|---|---|---|---|---|
| Obsidian Page preview | 파일 탐색기 · 검색 · 백링크 = 올리기 · 편집 화면 = Ctrl/Cmd + 올리기 · 설정으로 늘 수식 키 요구 | 떠 있는 미리보기 | 링크 옆(방향 문서 없음) | 문서 없음 | 코어 = 문서 없음 · 커뮤니티 Hover Editor = 고정 · 끌기 · 크기 · 여럿 | 문서 없음 | https://obsidian.md/help/plugins/page-preview (메인 확인) · https://community.obsidian.md/plugins/obsidian-hover-editor | 띄우기 = 확실 · 나머지 = 불확실 |
| VS Code Peek | Alt+F12 · Shift+F12 · 우클릭 | **편집기 안에 끼워 넣는 peek 창**(아래 글을 밀어냄) | 그 줄 아래 | 고칠 수 있는 작은 편집기 + 결과 목록 | 열린 채로 참조 사이를 오간다 · `editor.stablePeek` = 저절로 안 닫힘 · 파일 이름 누름 · 더블클릭 = 바깥 편집기로 | Esc · 더블클릭 | https://code.visualstudio.com/docs/editing/editingevolved (메인 확인) | 확실에 가까움(단일 1차 출처라 태그는 가능성 높음) |
| VS Code 올리기 | 올리기 · Ctrl + 올리기 = 선언 미리보기 | 카드 | 기호 옆 | 미확인 | 미확인 | 미확인 | 같음 | 띄우기 = 가능성 높음 |
| JetBrains Quick Documentation | 올리기(끌 수 있음) · Ctrl+Q | 팝업 또는 문서 도구 창 | 요소 옆 | 미확인 | 도구 창으로 · 문서 탭 고정 | 문서 없음 | https://www.jetbrains.com/help/idea/viewing-reference-information.html | 가능성 높음 |
| macOS Quick Look | 고른 항목에서 Space · Cmd+Y | 따로 떠 있는 창(늘 위) | 자유 | 전체 · 크기 조절 | **선택을 따라 바뀜** | Space · Esc | https://macmost.com/how-to-use-mac-quick-look-in-5-minutes.html | 가능성 높음(2차 출처) |
| 언리얼 콘텐츠 브라우저 | 올리기 | 정보 + 썸네일 툴팁 | 커서 옆 · 화면 오른쪽 아래 끝에선 넘쳐서 안 뜨는 결함 | 내용 아님 | 없음 | 마우스 치움 | https://issues.unrealengine.com/issue/UE-328263 | 가능성 높음 |
| Unity Inspector 참조 칸 | 누름 = Project 에서 그 에셋 깜빡임 · 더블클릭 = 열기 | 미리보기 없음 | — | — | — | — | https://docs.unity.cn/6000.0/Documentation/Manual/UsingTheInspector.html | 가능성 높음 |
| Notion · Figma | — | — | — | — | — | — | — | 기권 |

## grounding

- VS Code Peek — 원문: 「we embed the result inline」 · 열린 채 참조를 오가며 고친다 · Esc · 더블클릭으로 닫히고 `editor.stablePeek` 이 그걸 끈다 · 파일 이름 누름 · 더블클릭 = 바깥 편집기. **지지.**
- Obsidian — 원문: 올리기(탐색기 · 검색 · 백링크) · 편집 화면은 Ctrl/Cmd + 올리기 · 수식 키 요구 설정. 고정 · 스크롤 · 닫기는 문서에 없다. **지지(띄우기만).**

## 우리 설계에 주는 것

- 편집 칸 지시서 줄의 내용 보기는 **줄 옆에 붙는 미리보기 + 「고정」 하면 다른 줄을 골라도 내용이 바뀌는 꼴**(Quick Look 의 선택 따라가기)이 선례의 조합이다. 편집 칸이 화면 오른쪽 끝이라 옆에 붙이면 **왼쪽(목록 위)으로 뜬다** — 사용자 착상과 같은 자리.
- 띄우는 방법은 줄을 고르는 누름과 겹치지 않게(누름 = 끼울 자리 고르기 · R46) 따로 둔다 — 줄 위 미리보기 단추 · Space.

## 공백

- Obsidian 코어 미리보기의 스크롤 · 닫기 · 방향 · VS Code 올리기 카드 고정 · JetBrains Ctrl+Shift+I · Notion · Figma 미확인.
- 화면 끝에서 뒤집히는 동작은 언리얼 결함 하나로만 봤다.
