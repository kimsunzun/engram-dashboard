# 실행이 막혔을 때 사유 표시 · 안쪽 오류를 바깥 줄에 끌어올리기 (관행 조사)

- **상태:** light 조사 · 2026-10-18 · 수집자 1(문서형) + 메인 grounding(NN/g · Unreal 원문 대조) · 적대 리뷰 없음
- **질문:** ① 붙은 구성 요소가 깨져 실행이 막혔을 때 사유를 어디에 띄우나(모달 · 그 자리 · 알림) ② 안쪽 항목의 오류를 접힌 바깥 줄에 보이나.
- **맥락:** 에이전트 프리셋 슬롯에서 깨진 설정이 든 프리셋이 한눈에 보이고 눌러서 따라 들어가야 한다(사용자 2026-10-18).

## 결론

- **모달은 사용자가 모든 것을 멈추고 대응해야 할 때만이고, 그 밖의 오류는 출처 가까이 보이고 출처로 따라가게 한다.** NN/g「Display the error message close to the error's source.」 · 「modal dialogs … should be reserved for severe errors.」(원문 대조). Microsoft「Dialogs block interactions with the app window until being explicitly dismissed.」 — 앱 전체 오류(연결 끊김 등)만 대화상자. 구성 요소 하나 때문에 막힌 실행은 모달 경우가 아니다(추론). — 가능성 높음
- **Unreal 블루프린트 컴파일 결과 패널이 가장 가까운 선례다** — 오류가 나면 저절로 열리고(「The panel will also open automatically whenever a Blueprint compile results in errors or warnings.」), 항목을 누르면 문제 노드로 간다(「Click on a message to jump the graph view directly to the problematic node」). — 확실(원문 대조)
- **VS Code 탐색기는 안쪽 파일의 오류를 부모 폴더로 끌어올린다** — 개수 배지(10개 이상 「9+」) · 가장 심한 단계의 색 · 설정 `problems.decorations.enabled`. 릴리스 노트「The number of errors/warnings is shown in the decoration.」 부모로 끌어올림(`bubble: true`)은 소스 요약으로만 확인. — 가능성 높음
- VS Code · JetBrains 실행 구성 오류 화면 · Docker Desktop · Windows Terminal 설정 오류 배너 · JetBrains/Visual Studio 의 부모 표시 — 1차 자료를 못 찾았다(기권).

## 출처

| 출처 | 내용 | 확신도 |
|---|---|---|
| https://www.nngroup.com/articles/error-message-guidelines/ | 출처 가까이 · 모달은 심각한 오류만 | 확실(원문 대조) |
| https://learn.microsoft.com/en-us/windows/apps/design/controls/dialogs-and-flyouts/ | 대화상자는 막는다 · 앱 전체 오류만 | 확실(수집자 인용) |
| https://dev.epicgames.com/documentation/en-us/unreal-engine/compiler-results-in-the-blueprints-visual-scripting-editor-for-unreal-engine | 오류 때 자동으로 열림 · 누르면 문제 노드로 | 확실(원문 대조) |
| https://code.visualstudio.com/updates/v1_20 · `markersFileDecorations.ts` | 오류 개수 배지 · 부모 폴더로 끌어올림 | 가능성 높음 |
