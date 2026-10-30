# 라이브러리 슬롯 — 여러 소스 폴더를 어떻게 보여 주나 (피어 서베이)

- **상태:** 적대 리뷰(BLOCK) 반영본 · 결정 대기(프리셋 PRD 이전 구상 — 화면 1 라이브러리 슬롯 재검토)
- **날짜:** 2026-10-14
- **방법:** research medium · 설계-결정 모드. 수집 갈래 3(AI 에이전트 도구 · 게임 엔진/DCC · 일반 파일 라이브러리) · 메인 grounding(아래 「grounding」) · cross-family 적대 리뷰 1회(아래 「적대 리뷰」).
- **확신도 범례:** 확실 = 원문 확인 + 독립 교차확증 · 가능성 높음 = 원문 확인(단일 출처) 또는 부분 지지 · 불확실 = 기억 기반 · 원문 미확인.
- **앞선 조사:** `docs/research/agent-preset-components-survey-2026-10-11.md`(부품 · UI 패턴 서베이) — 이 문서는 그 문서의 「라이브러리 목록 모양」 공백을 메운다.

## 묻는 것

사용자는 지시서 · 스킬이 많아 자기 폴더 구조로 정리해 둘 것이다. 그렇다면

1. 라이브러리 가운데 목록을 **평평하게** 늘어놓는 게 맞나, **폴더 트리**를 보여야 하나, **폴더와 무관한 분류**(카탈로그 · 컬렉션 · 태그)를 둬야 하나?
2. 소스 폴더가 여럿일 때 어떻게 합쳐 보여 주나?
3. 등록한 루트 밖 파일 하나를 툭 링크하는 길이 있나? 있으면 PC 사이 이식성은 어떻게 다루나?
4. 루트는 어떻게 등록하나?

## 결론 (위층)

1. **표본 안에서는, 큰 라이브러리를 다루는 DCC · 파일 도구가 「파일이 놓인 곳(물리 폴더)」과 「찾아보는 길(가상 분류)」을 함께 둔다.** 가장 가까운 실물은 **Unreal Content Browser** — 왼쪽 Sources 패널에 프로젝트 폴더 계층 트리와 Collections 목록이 함께 있고(플러그인 · 엔진 콘텐츠 폴더도 켤 수 있다), 폴더를 Ctrl/Shift 로 여러 개 골라 가운데 Asset View 를 거른다. 그 밖에 Lightroom(Folders 패널 ↔ Collections) · foobar2000(폴더 구조 보기 ↔ 아티스트/앨범 보기 전환) · Obsidian(폴더 ↔ 중첩 태그) · Substance 3D Designer(필터는 조건 여럿을 가질 수 있고 리소스 하나가 여러 필터에 나온다 — 배타적이지 않다). **Blender 는 반대쪽 끝** — 왼쪽 트리가 디스크 폴더가 아니라 **파일 위치와 독립된 카탈로그 트리**다(에셋은 카탈로그 하나에만). 「주류」라고 일반화할 만큼 표본이 넓지는 않다. **가능성 높음**(Unreal · Blender · Substance 필터는 원문 확인 · Lightroom · foobar2000 · Obsidian 일부는 스니펫/포럼/기억).
2. **AI 도구 쪽은 수집에서 「디스크 위치 = 분류」만 확인했고, 가상 분류가 없다고 단정할 수 없다.** 확인한 것: Claude Code 는 범위(Enterprise · Personal · Project · Nested · `--add-dir` · Plugin)로 싣고 이름 충돌을 접두어(`/plugin:skill` · `/apps/web:deploy`)와 우선순위로 가른다 — 이건 이름 충돌 규칙이지 목록 화면 관례의 증거가 아니다. **적대 리뷰가 반례 후보를 들었다**(메인 미확인 — 아래 「적대 리뷰」): TypingMind 프롬프트 라이브러리(태그 · 그룹 공개 범위) · JetBrains AI Assistant Prompt Library · Cursor Rules 관리 UI · Zed Skills 관리자. 파일 기반 스킬과 같지는 않지만 「AI 도구엔 라이브러리 선례가 없다」는 서술은 철회한다. **불확실.**
3. **여러 루트 표시는 두 갈래가 다 보인다.** ① 루트마다 트리(VS Code 멀티 루트 — 루트별 탐색기 트리 · 검색 결과도 루트별 묶음) ② 한 풀로 병합(Houdini 검색 경로 · Espanso · 음악 라이브러리 감시 폴더). Unreal 은 루트(프로젝트 · 플러그인 · 엔진)를 한 폴더 트리 안의 최상위 노드로 둔다. 병합할 때 출처를 어떻게 표시하는지는 일반 관례로 말할 근거가 없다. **가능성 높음**(①) · **불확실**(출처 표시 관례).
4. **「루트 밖 파일 하나 링크」의 가장 확실한 선례는 Zotero linked file**이다 — 원본 위치만 기억하고(동기화 · 그룹 · 모바일 불가), **기준 폴더(Linked Attachment Base Directory) 아래에 있는 파일만** 상대경로로 저장해 PC 마다 기준 폴더를 다시 지정하면 맞는다. 기준 폴더 **밖** 링크는 절대경로 그대로라 이식성이 없다. 기준 폴더를 지정하면 끊긴 절대경로 링크를 찾아 한꺼번에 다시 잇자고 제안한다. Claude Code 는 바깥 스킬을 **심볼릭 링크**로 붙이라고 안내한다(같은 대상을 두 경로가 가리키면 한 번만 싣는다). 적대 리뷰는 **Adobe Bridge Collections**(아무 드라이브의 파일을 끌어 넣어 참조로 유지 · 없는 파일 고치기)도 들었다 — 원문이 403 이라 메인 미확인. **확실**(Zotero · Claude Code 원문) · **불확실**(Bridge). → 루트 밖 단발 링크는 **허용하되 이식성 없는 항목**으로 다루는 모양이 선례에 가깝다.
5. **루트 등록은 「설정의 이름 + 경로 목록」 또는 「폴더 안 매니페스트 + 추가 명령」.** VS Code `.code-workspace` 의 `{name, path}`(상대경로 가능) · foobar2000 Music Folders 목록 · Blender Preferences(기억) · Houdini `HOUDINI_PATH` · Unity `file:` 상대/절대 · Claude Code `marketplace.json` + `claude plugin marketplace add`. 이식성은 **루트 기준 상대경로 + PC 별 루트 위치**(Zotero · Unity · Claude 마켓 — 마켓 루트 기준 상대 · `..` 금지)로 확보한다. Blender UUID 는 **카탈로그**를 가리키는 id 라 루트 위치 이식성 사례가 아니다(초안의 오용 — 리뷰 지적). **가능성 높음.**
6. **찾을 수 없는 항목은 숨기지 않고 표시 + 폴더 단위로 다시 잇는다.** Lightroom: 없는 폴더에 `?` 아이콘 · 우클릭 「Find Missing Folder」 · 개별 파일보다 폴더 단위 재연결 권장. **가능성 높음**(검색 스니펫 · 원문 페이지 일부 403).

## Engram 에 옮기면 (해석 — 결정은 사용자)

지금 브리핑 v13 의 라이브러리 슬롯은 「왼쪽 = 소스 폴더 체크박스 · 가운데 = 평평한 목록(검색 + 칩 + 종류)」이다. 위 결론 1 · 3 에 비추면 **가운데 목록이 폴더 구조를 전혀 못 보여 주는 것**이 빈 곳이다. 선택지:

| 안 | 모양 | 선례 | 장점 | 단점 |
|---|---|---|---|---|
| **A. 평평한 목록 유지** | 지금 그대로 · 폴더는 경로 표시로만 | AI 도구 다수(Claude Code 등) | 가장 단순 · 검색 중심 | 파일이 수십 개 넘으면 사용자가 만든 폴더 정리가 버려진다 |
| **B. 소스마다 폴더 트리** | 가운데가 VS Code 탐색기처럼 루트별 트리 | VS Code 멀티 루트 | 사용자 정리가 그대로 보인다 | 검색 · 「쓰는 중」 같은 가로지르는 거름이 트리와 겹쳐 어색 · 루트가 많으면 길어진다 |
| **C. 왼쪽 = 폴더 트리 내비, 가운데 = 고른 폴더의 결과** | 왼쪽 소스 폴더를 펼치면 하위 폴더 트리 · 고른 폴더(여러 개 가능)의 항목이 가운데 목록에 · 검색 · 칩은 그대로 | **Unreal Sources 패널**(폴더 트리 · 다중 선택 → Asset View) · Lightroom Folders 패널 · foobar2000 폴더 구조 보기 | 폴더 정리와 검색이 같이 산다 · 지금 화면 구조를 거의 유지 | 왼쪽 칸이 깊어지면 좁다 · 고른 폴더의 하위 폴더까지 보이는지는 Unreal 문서에도 없어 우리가 정해야 한다 |
| **D. C + 가상 분류(태그 · 컬렉션)** | C 에 더해 왼쪽에 태그 · 컬렉션 구역 | **Unreal**(같은 패널에 Collections) · Lightroom Collections · Substance 필터(비배타) · Blender 카탈로그(폴더 대신 가상 트리) · Obsidian 중첩 태그 | 폴더와 다른 축으로 묶기(브리핑 T-1 과 합류) | 만들 것이 가장 많다 · 태그 저장 위치(T-2) 결정이 따라온다 |

- **D 의 분류 축이 둘이다** — 하나에만 넣는 트리(Blender 카탈로그 — 파일 시스템처럼 배타)와 여러 곳에 넣는 태그 · 컬렉션(Substance 필터 · Lightroom · Unreal). 브리핑 T-1(중첩 태그)은 뒤쪽이다.
- 루트 밖 단발 링크: 결론 4 를 따르면 **「바깥 링크」를 일급 항목으로 허용하되 이식성 없음으로 표시**하는 쪽이 Zotero 와 닮았다. 왼쪽 칸에 「바깥 링크」 묶음을 하나 두면 C · D 구조에 그대로 들어간다. ★소스 폴더가 Zotero 기준 폴더 역할을 하는 것은 **소스 폴더 안 파일**에 한해서다 — 바깥 링크는 그 덕을 못 본다(초안의 논리 공백 — 리뷰 지적).
- 찾을 수 없음: 결론 6 을 따라 소스 폴더 단위 「폴더 다시 찾기」를 왼쪽 칸 우클릭에 둔다.

## grounding (메인 · 원문 대조)

| 클레임 | 출처 | 판정 |
|---|---|---|
| Blender 카탈로그는 파일 디스크 구성과 독립 · 에셋은 카탈로그 하나에만 · 정의 파일 `blender_assets.cats.txt`(`UUID:경로:단순이름`) · 분리 이유 = 파일 사이 자유로운 이동 | https://developer.blender.org/docs/features/asset_system/backend/asset_catalogs/ | 지지 |
| Zotero 기준 폴더 아래 linked file 은 상대경로 저장 · PC 마다 기준 폴더 지정 · 끊긴 절대경로 자동 재연결 제안 | https://www.zotero.org/support/preferences/advanced | 지지 |
| Claude Code 스킬 위치 표 · Enterprise > Personal > Project 우선 · 심볼릭 링크 허용(같은 대상 한 번만) · 플러그인 `/plugin:skill` · 중첩 `/apps/web:deploy` · `--add-dir` | https://code.claude.com/docs/en/skills | 지지 |
| VS Code 루트별 탐색기 트리 · `.code-workspace` folders `{name, path}` 상대경로 | https://code.visualstudio.com/docs/editing/workspaces/multi-root-workspaces | 수집자 원문 확인 · 메인 미재확인 |
| Lightroom `?` 아이콘 · Find Missing Folder | https://helpx.adobe.com/lightroom-classic/desktop/manage-catalogs-and-files/locate-missing-photos.html | 검색 스니펫만 — 부분 지지 |
| foobar2000 폴더 구조 보기 ↔ 아티스트/앨범 보기 | https://hydrogenaudio.org/index.php?msg=717049 (포럼) | 부분 지지(공식 문서 아님) |
| Unity `file:` 상대(Packages 기준)/절대 · 슬래시 권장 | https://docs.unity3d.com/Manual/upm-localpath.html | 수집자 원문 확인 · 메인 미재확인 |
| Houdini `HOUDINI_PATH/otls` 자동 로드 · 씬 임베드 비권장 | https://www.sidefx.com/docs/houdini/assets/install.html | 수집자 원문 확인 · 메인 미재확인 |
| Unreal Sources 패널 = 프로젝트 폴더 계층 + Collections 목록 · 플러그인/엔진 폴더 선택 표시 · Ctrl/Shift 다중 폴더 선택 | https://dev.epicgames.com/documentation/unreal-engine/sources-panel-reference-in-unreal-engine?lang=en-US | 지지(리뷰가 찾음 · 메인 확인) |
| Substance Designer 필터 = 조건 여럿 · 리소스 하나가 여러 필터 · 배타 아님 · 프로젝트 파일 저장 | https://experienceleague.adobe.com/en/docs/substance-3d-designer/using/workspace/library/managing-custom-content-and-filters | 부분 지지(경로 여럿 · 별칭은 그 페이지에 없음) |
| Claude 마켓 `source` 상대경로(마켓 루트 기준 · `..` 금지) | https://code.claude.com/docs/en/plugin-marketplaces | 수집자 원문 확인 · 메인 미재확인 |

## 적대 리뷰

codex(cross-family · effort high · 레벨 2 · 웹 검색) 1회. 판정 **BLOCK** — 초안 기준. 아래처럼 반영했다.

| 지적 | 심각도 | 반영 |
|---|---|---|
| 「AI 도구엔 가상 분류 · 라이브러리 선례가 없다」는 과장 — TypingMind 프롬프트 라이브러리(태그) · JetBrains Prompt Library · Cursor Rules · Zed Skills | 높음 | 결론 2 를 「단정 못 함 · 철회」로 고침. **반례 넷은 메인 미확인**(리뷰어 주장) |
| Blender 를 C(물리 폴더 트리)의 선례로 씀 — Blender 왼쪽 트리는 카탈로그(디스크와 독립) | 높음 | C 에서 빼고 D(가상 분류)로 옮김 |
| Unreal 을 「못 읽음」으로 둠 — Sources 패널 문서가 폴더 트리 + Collections + 다중 선택 → Asset View 를 보여 준다 | 높음 | **메인이 원문 확인**(https://dev.epicgames.com/documentation/unreal-engine/sources-panel-reference-in-unreal-engine?lang=en-US) — 결론 1 · C · D 의 대표 선례로 올림. 하위 폴더 포함 여부 · 컬렉션 정적/동적은 그 문서에 없음 |
| Substance Designer 를 미조사로 둠 — 감시 경로 여럿 · 가상 필터 · 경로 별칭 | 높음 | **메인 원문 확인은 부분** — 「필터는 조건 여럿 · 리소스 하나가 여러 필터에 · 배타 아님 · 프로젝트 파일에 저장」만 확인(https://experienceleague.adobe.com/en/docs/substance-3d-designer/using/workspace/library/managing-custom-content-and-filters). 다중 경로 추가 절차 · 경로 별칭은 그 페이지에 없음 → 미확인 유지 |
| Zotero 를 유일 선례로 둠 — Adobe Bridge Collections 도 바깥 파일 참조 + 없는 파일 고치기 | 중간 | 결론 4 에 리뷰어 주장으로 추가 · 원문 403 이라 **미확인** |
| 「소스 폴더 = Zotero 기준 폴더」라서 바깥 링크도 이식된다는 논리 공백 — Zotero 상대경로는 기준 폴더 **아래** 파일만 | 높음 | 결론 4 · 옮기면 절에 명시 |
| Blender UUID 를 루트 위치 이식성 사례로 씀 — 카탈로그 id 다 | 중간 | 결론 5 에서 철회 |
| 「혼합이 주류」 · 「병합하면 출처 접두어 · 열」은 표본을 넘는 일반화 | 중간 | 결론 1 · 3 의 표현을 「표본 안에서」 · 「불확실」로 낮춤 |

- 리뷰어 grounding 스팟체크: Blender 카탈로그 · Zotero 기준 폴더 · Claude Code 스킬 위치 셋 다 **지지** — 단 앞의 둘은 위 두 줄처럼 쓰임새가 틀렸다는 단서.
- 반영 후 재리뷰는 돌리지 않았다(medium 단일 패스).

## 한계 · 공백

- **화면 캡처는 하나도 못 봤다** — 「평평한가 · 트리인가」 판정은 대부분 문서 서술에서 끌어낸 추론이다. 특히 AI 도구의 「폴더 트리를 안 보여 준다」는 「보여 준다는 증거를 못 찾았다」이지 확인된 부재가 아니다.
- **못 읽음:** Cursor(리다이렉트) · Zed Rules Library(404 — 리뷰어에 따르면 Skills 로 바뀜) · Substance 3D Painter 리소스 경로 · 별칭(403) · Adobe Folders 패널(403) · Adobe Bridge Collections(403) · MusicBee(402).
- **미조사:** Windsurf · JetBrains AI Assistant · Roo · 전용 프롬프트 매니저 앱 · Godot · Krita · Maya · Figma/Adobe CC Libraries · Logseq · Calibre · Eagle · Raycast/Alfred.
- 가상 분류를 따로 둔 **이유를 사용자 불만으로 적은 자료**는 못 찾았다 — 확인된 근거는 Blender 설계 문서 하나다.
- 후속 후보: Substance 경로 별칭(`your_assets://` 류 — 이식성 후보 · 기억 기반) · Unreal Collections 저장 방식 · Blender 라이브러리 등록과 상대경로.
