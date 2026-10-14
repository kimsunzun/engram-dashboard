# 라이브러리 슬롯의 뒤로 / 앞으로(이동 기록) — 관행 요약

- **상태:** light · 결정 대기(프리셋 PRD 이전 구상 — 라이브러리 슬롯에서 「쓰는 곳」 링크를 탄 뒤 돌아오기)
- **날짜:** 2026-10-14
- **방법:** research light. 수집자 1 · 메인 grounding(핵심 1건 원문 확인) · 적대 리뷰 없음(light).

## 결론

1. **파일 · 에셋 브라우저의 뒤로/앞으로는 「위치(폴더)」만 기록하는 웹 브라우저식이다.** 언리얼 콘텐츠 브라우저 원문: 「웹 브라우저의 Back · Forward 처럼 동작한다 — 최근 파일 경로 사이를 오간다」. 윈도우 탐색기 · macOS Finder 도 폴더 위치만(기억 — 미확인). **가능성 높음.**
2. **편집 되돌리기(undo/redo)와 이동 기록은 별개 명령이다.** 에디터류(VS Code Go Back/Forward = 「파일과 편집 위치 사이 이동」 · JetBrains = 캐럿 위치)도 이동 기록은 undo 와 따로다(분리 서술은 기억 — 키 바인딩만 원문). **가능성 높음.**
3. **기록 범위는 창 · 패널마다가 흔하다**(브라우저 탭 모델). 언리얼은 콘텐츠 브라우저마다 기록 관리자(`FHistoryManager`)를 두고 기록 목록을 메뉴로 내는 API 가 있다(API 문서 — 인스턴스별 여부는 미확인). **불확실.**
4. **조작:** ← → 단추 · Alt+←/→(VS Code Windows · 탐색기) · 마우스 옆 단추 · 단추를 길게 누르거나 우클릭하면 기록 목록. **가능성 높음**(일부 기억).

## Engram 에 옮기면 (해석 — 결정은 사용자)

- 새 기록 = 자리가 바뀔 때(폴더 이동 · 「쓰는 곳」 링크 · 묶음 보기). 같은 폴더 안 클릭은 기록을 늘리지 않고 그 기록의 「마지막 선택」만 고쳐 쓴다 → 뒤로 = 이전 자리 + 그때 마지막 선택. 「마지막 선택 복원」은 위 브라우저들(위치만)보다 한 걸음 더 간 것이다.
- 편집(담기 · 빼기)은 기록에 넣지 않는다 — 결론 2.
- 슬롯마다 따로 · ← → 단추 + Alt+←/→ + 마우스 옆 단추. 기록 목록 드롭다운은 나중.

## grounding

| 클레임 | 출처 | 판정 |
|---|---|---|
| 언리얼 콘텐츠 브라우저 History Back/Forward = 웹 브라우저처럼 최근 파일 경로 사이 이동 | https://dev.epicgames.com/documentation/unreal-engine/content-browser-interface-in-unreal-engine | 지지(메인 원문 확인) |
| 언리얼 `FHistoryManager` · 기록 메뉴 항목 API | https://dev.epicgames.com/documentation/unreal-engine/API/Editor/ContentBrowser/FHistoryManager | 수집자 확인 · 메인 미재확인 |
| VS Code Go Back/Forward 키(Windows Alt+←/→) | https://code.visualstudio.com/docs/editor/editingevolved | 수집자 확인 · 메인 미재확인 |
| JetBrains 뒤로/앞으로 = 캐럿 위치 · Ctrl+Alt+←/→ | https://www.jetbrains.com/help/idea/navigating-through-the-source-code.html | 수집자 확인 · 메인 미재확인 |

## 한계

- light 라 적대 리뷰 없음. 탐색기 · Finder 는 기억 기반. Blender 미조사.
