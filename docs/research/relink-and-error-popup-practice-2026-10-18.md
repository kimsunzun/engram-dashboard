# 없어진 참조 다시 연결 안내 · 떠 있는 오류 팝업과 겹침 (관행 조사)

- **상태:** light 조사 · 2026-10-18 · 수집자 1(문서형) + 메인 grounding(Microsoft InfoBar 원문 대조) · 적대 리뷰 없음
- **질문:** ① 바깥에서 옮겨져 없어진 파일 참조를 사용자가 다시 잇게 어떻게 이끄나(목록 · 찾는 법 · 하나 고치면 나머지도 고치나 · 이름 맞춤은 자동인가) ② 떠 있는 오류 팝업에서 다른 창을 열면 팝업은 어떻게 되나 · 남은 항목으로 돌아오는 길.

## 결론

- **① 한 군데에서 없어진 것을 보이고, 사용자가 파일 하나(또는 찾을 폴더)를 짚으면 도구가 나머지를 이름으로 고친다** — Premiere Pro(「미디어 연결」 · 하나를 찾으면 같은 · 비슷한 폴더의 나머지를 저절로 다시 잇는다) · After Effects(하나를 바꾸면 같은 폴더를 찾아 「X additional missing items have been found」) · DaVinci Resolve(폴더를 주면 하위까지 파일 이름으로 찾음) · Blender(「Find Missing Files」 — 폴더를 하위까지 찾아 찾은 것을 전부 되살림). 나머지 자동 연결에 항목마다 확인을 받는 피어는 못 찾았다. — 가능성 높음(2차 자료 · Adobe 공식 페이지 403)
- **사용자가 먼저 짚지 않았는데 이름만으로 저절로 잇는 피어는 못 찾았다** — 위 피어는 모두 사용자가 「찾기」를 시작한 뒤 이름으로 맞춘다. Unreal 은 에디터 안 이동 때 리다이렉터를 남겨 참조를 지키고(바깥 이동의 다시 연결 화면은 확인 못 함), Unity 는 「Missing (Mono Script)」 만 보이고 다시 연결 화면이 없다. — 가능성 높음
- **② 앱 상태가 바뀐 오류는 인라인 알림(닫을 때까지 남고 다른 내용을 덮지 않음)이 Microsoft 권고다** — 「By default the notification will remain in the content area until closed by the user but will not necessarily break user flow.」 · 「It will not cover up other content or float on top of it.」 특정 UI 요소 맥락의 정보는 Flyout(지속 불필요) · 반드시 읽어야 하거나 앱 전체를 막아야 하면 ContentDialog(원문 대조). — 확실
- **잠깐 뜨는 알림 + 남는 기록이 흔한 꼴이다** — JetBrains 는 풍선(곧 사라짐 · 끈끈한 풍선 = 닫을 때까지)과 알림 도구 창의 기록을 같이 둔다. — 가능성 높음(검색 요약)
- **팝업에서 링크로 다른 창을 연 뒤 팝업이 어떻게 되는지를 적은 출처는 못 찾았다.**

## 출처

| 피어 | 출처 | 확신도 |
|---|---|---|
| Premiere Pro 미디어 연결 | https://www.adobe.com/learn/premiere-pro/web/relink-offline-media · https://larryjordan.com/articles/premiere-pro-cc-relink-missing-media/ | 가능성 높음 |
| After Effects 푸티지 바꾸기 | https://aejuice.com/blog/how-to-relink-footage-in-after-effects | 가능성 높음 |
| DaVinci Resolve 다시 연결 | Resolve 18 매뉴얼 미러 · https://jayaretv.com/workflow/relink-offline-media-in-davinci-resolve/ | 가능성 높음 |
| Blender Find Missing Files | docs.blender.org 2.79 매뉴얼(요약) | 가능성 높음 |
| Unreal 리다이렉터 | https://dev.epicgames.com/documentation/en-us/unreal-engine/asset-redirectors-in-unreal-engine | 가능성 높음 |
| Microsoft InfoBar | https://learn.microsoft.com/en-us/windows/apps/design/controls/infobar | 확실(원문 대조) |
| JetBrains 알림 | https://www.jetbrains.com/help/idea/notifications.html | 가능성 높음 |

## 공백

- Adobe 공식 도움말 직접 읽기 실패(403). 찾기 창에 옛 이름이 미리 들어가는지 · Resolve 의 항목별 확인 여부 · VS Code 알림 지속 여부 미확인.
