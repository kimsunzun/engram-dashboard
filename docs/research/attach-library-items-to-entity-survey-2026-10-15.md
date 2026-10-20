# 라이브러리 항목을 라이브러리 밖 대상(에이전트)에 붙이는 화면 — 관행 (피어 서베이)

- **상태:** 적대 리뷰(FIX) 반영본 · 결정 대기(프리셋 PRD 이전 구상 — 에이전트에 프리셋을 붙이는 화면. 세션 초안 둘 — 트리 옆 팝업 · 라이브러리 오른쪽 칸에 에이전트 띄우기 — 을 사용자가 「뭔가 이상하다」며 관행을 보자고 함)
- **날짜:** 2026-10-15
- **방법:** research medium · 설계-결정 모드. 수집 갈래 2(콘텐츠 도구 · AI 도구) · 메인 grounding(Ableton · Open WebUI 원문) · cross-family 적대 리뷰 1회(아래).
- **확신도 범례:** 확실 = 원문 + 독립 교차확증 · 가능성 높음 = 원문(단일) 또는 부분 지지 · 불확실 = 기억 · 요약.

## 결론 (위층)

1. **대상의 편집 칸과 라이브러리(브라우저)는 따로 된 두 칸이다 — 대상을 골라도 라이브러리 화면이 바뀌지 않는다.** 언리얼(아웃라이너에서 액터 고름 → 디테일 칸 · 콘텐츠 브라우저는 따로) · 유니티(인스펙터 · 프로젝트 창 따로) · Figma(오른쪽 속성 · 왼쪽 에셋 따로) · Ableton(브라우저 따로 · 고른 트랙이 대상). 라이브러리 칸 안에 대상을 띄우는 피어는 없었다(「없다」는 단정이 아니라 「못 찾았다」). **가능성 높음.** → 세션 대안(라이브러리 오른쪽 칸에 에이전트)은 관행과 어긋난다.
2. **대상의 붙은 목록은 붙박이 편집 칸(인스펙터 · 디테일)에서 고친다.** 언리얼 디테일 · 유니티 인스펙터 · Figma 오른쪽 사이드바. AI 도구에선 대상(모델 · 에이전트)의 편집 화면 안에 종류별 고르기가 있다 — **Open WebUI 모델 편집기**: Knowledge · Tools · Skills 등이 「검색되는 고르기 창(searchable picker)」을 열고, Clear · Enable all · **Manage(그 항목들을 만드는 워크스페이스 페이지로 이동)** 가 붙는다(메인 원문 확인). **가능성 높음.**
3. **넣는 길은 넷이고 보통 함께 준다.** ① 칸의 검색 고르기 창(유니티 ⊙ · Figma · Open WebUI) ② 브라우저에서 고른 것 넣기 ⇐(언리얼) ③ 브라우저에서 칸이나 대상 위로 끌어 놓기(유니티 · Ableton · 언리얼은 기억) ④ **대상을 골라 둔 채 브라우저에서 더블클릭 · Enter = 그 대상에 적용**(Ableton — 원문: 「double-click it in the browser, which adds it to the selected track or creates a new track if one is not selected」 · Lightroom 프리셋 클릭 · Figma 스타일 클릭). **가능성 높음**(Ableton 원문 · 나머지 요약).
4. **AI 도구는 「대상 = 이름 붙은 에이전트/프로필, 항목은 참조로」** 가 많다(Copilot 사용자 에이전트 · Claude Code 서브에이전트 frontmatter `tools`/`skills`/`mcpServers` · Open WebUI · GPT 편집기). 붙이는 단위를 인스턴스가 아니라 범위(폴더 · 워크스페이스)로 두는 곳도 있다(VS Code 프로필 — 폴더에 묶임 · Cursor 규칙). **가능성 높음**(수집자 원문 요약 · 메인 대조는 Open WebUI 만).

## Engram 에 옮기면 (후보 — 결정 아님)

- **에이전트 편집 칸 = 라이브러리와 따로 된 붙박이 칸**(언리얼 아웃라이너 + 디테일 꼴): 에이전트 트리 슬롯에서 에이전트를 고르면 그 에이전트의 편집 칸(프리셋 편집 칸과 같은 부품 · 프리셋 칸만)이 따로 뜨고, 라이브러리 슬롯은 옆에 그대로 둔다.
- **넣는 길:** `+`(검색 고르기) · `⇐`(라이브러리 슬롯에서 고른 것) · 라이브러리에서 에이전트 줄이나 편집 칸으로 끌어 놓기 · (선택) 에이전트를 골라 둔 채 라이브러리에서 더블클릭 = 붙이기(Ableton 꼴 — 단 라이브러리 더블클릭은 이미 「OS 로 열기」라 충돌).
- 편집 칸을 어디에 붙박을지(트리 슬롯 안 아래 · 오른쪽 칸 / 따로 된 슬롯)는 열린 질문.

## grounding

| 클레임 | 출처 | 판정 |
|---|---|---|
| Ableton 더블클릭 = 고른 트랙에 추가 · 없으면 새 트랙 · Enter · 끌어 놓기 | https://www.ableton.com/en/manual/working-with-instruments-and-effects/ (메인 · 요약기 경유 원문 인용) | 지지 |
| Open WebUI 모델 편집기 — searchable picker · Clear · Manage · Enable all | https://docs.openwebui.com/features/workspace/models (메인 · 요약기 경유 원문 인용) | 지지 |
| 언리얼 ⇐ use selected · 디테일/콘텐츠 브라우저 따로 | https://docs.unrealengine.com/4.27/en-US/BuildingWorlds/LevelEditor/Details/Properties/ObjectReference (수집자 · 403 — 검색 요약) | 부분지지(사용자 스크린숏의 ⇐ 아이콘과 맞음) |
| 유니티 오브젝트 칸 고르기 창 · 끌어 놓기 | https://docs.unity3d.com/430/Documentation/Manual/EditingReferenceProperties.html (수집자 요약) | 미확인 |
| Claude Code 서브에이전트 frontmatter · Copilot 사용자 에이전트 · Cursor 규칙 · VS Code 프로필 폴더 묶음 | 수집자(원문 요약) | 미확인(메인 대조 안 함) |
| Lightroom 프리셋 클릭 적용 · Figma 에셋 클릭 적용 · GPT 편집기 | 요약 · 서드파티 | 불확실 |

## 적대 리뷰

codex(cross-family) · effort high · 레벨 2 · 웹 검색 · 판정 **FIX**. 반영(이 절이 위 결론보다 우선한다):

1. **결론 1 반례(높음)** — Blender Pose Library: 활성 오브젝트가 포즈 모드의 아마추어면 에셋 브라우저에 Pose Library 패널이 나온다 → 대상이 브라우저 화면을 바꾸는 사례가 있다. 결론 1 은 「표본 대부분은 따로 둔다」로 낮춘다(리뷰어 보고 · 메인 미확인).
2. **「라이브러리 칸 안에 대상을 띄우면 관행과 어긋난다」는 과장(높음)** — 그 꼴의 정확한 피어는 리뷰어도 못 찾았다. 「어긋난다」가 아니라 「선례를 못 찾았다」.
3. **결론 2 누락 반례(높음)** — n8n AI Agent 는 도구 노드를 그래프로 잇고, Zed 에이전트 프로필은 모달에서 도구를 고른다 → 붙박이 인스펙터는 여러 꼴 중 하나다(리뷰어 보고 · 메인 미확인).
4. **결론 3 「넷을 보통 함께 준다」는 근거 없는 종합(중간)** — 제품마다 다른 길을 준다는 것까지만 확인된다.
5. **결론 4 유비 공백(중간)** — Open WebUI · Zed 는 저장된 모델/프로필 설정이지 돌고 있는 에이전트 인스턴스 편집이 아니다.

리뷰어 스팟 grounding: Ableton · Open WebUI · 유니티 Object Picker · 언리얼 「Use Selected Asset」 지지.

## 공백

- Blender · 포토샵 라이브러리 · Dify · Flowise · JetBrains · LibreChat · Zed 프로필 편집 방식 · Continue 허브 블록 추가 흐름.
