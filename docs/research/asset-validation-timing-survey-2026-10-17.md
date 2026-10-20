# ★ 에셋 · 설정 파일 검사를 언제 하나 — 훑을 때 전부 · 고를 때 · 바뀔 때 · 따로 누를 때 — light 조사

| 항목 | 값 |
|---|---|
| 상태 | 조사 완료 · 결정 대기(사용자) |
| 날짜 | 2026-10-17 |
| 방법 | `/research` light — 수집자 1명(검색 · 가져오기 약 11회) · 메인 grounding 스팟체크 1건(언리얼 Data Validation 문서 WebFetch) · 적대 리뷰 없음 |
| 질문 | 라이브러리의 설정 · 프리셋 파일을 문법 + 스키마로 검사해 목록에 ⚠/⛔ 아이콘을 붙일 때, 훑을 때마다 전부 검사해야 하나(비용) — 고를 때 · 바뀐 파일만 · 따로 누를 때 중 무엇이 관행인가 |
| 앞 실측 | `docs/research/settings-asset-validation-spike-2026-10-17.md` |

## 결론

- **훑을 때마다 전부 검사해 표시를 붙이는 곳은 없다 — 기본은 조금씩**: 열 때 · 고칠 때 · 저장할 때(바뀐 것만) 검사하고 결과를 기억해 두며, 프로젝트 전체는 **따로 누르는 명령**이나 CI 로 돌린다. (가능성 높음)
- **언리얼 Data Validation** — 「on asset save (enabled by default), and through menu options in the editor and Content Browser」 + CIS · 결과는 「Message Log in the editor, and in the console in CIS」 · 콘텐츠 브라우저 표시 아이콘 언급 없음. (확실에 가까움 — 메인 원문 확인)
- **열어 보지 않은 파일에 목록 표시를 붙이는 것은 드물다** — 붙는 곳은 백그라운드 분석 결과를 쌓아 둔 경우(JetBrains 프로젝트 전체 분석 — 선택 · yaml-language-server `workspace` 모드 — 선택). (가능성 높음 · VS Code 탐색기 표시는 미확인)
- **비용은 걸림돌이 아니다** — 스키마는 한 번 컴파일하고 여러 번 재사용(ajv 문서: 컴파일은 느리고 실행은 빠르다). Rust `jsonschema` 수치는 안 봄. (패턴 = 확실 · 수치 = 미확인)

## 발견

| 도구 | 언제 | 범위 | 안 연 파일 목록 표시 | 출처 | 확신 |
|---|---|---|---|---|---|
| VS Code + yaml-language-server | 열 때 · 고칠 때 · 진단 모드 `openFilesOnly` / `workspace` | 기본은 연 파일 · 작업 공간 전체는 선택 | 미확인 | https://github.com/redhat-developer/vscode-yaml | 모드 = 가능성 높음 |
| JetBrains | 연 파일 실시간 · 「Inspect Code」 = 따로 누름 · 프로젝트 전체 분석 = 선택 | 연 파일 / 따로 / 선택 | Project Errors 는 기본으로 연 적 있는 파일만 | https://blog.jetbrains.com/idea/2020/08/working-with-code-problems-in-intellij-idea/ | 가능성 높음 |
| 언리얼 Data Validation | 저장할 때(기본 켜짐) · 메뉴 「Validate Assets」 · 명령줄(CI) | 저장한 것 / 고른 것 / 전체 | 없음(문서에 언급 없음) | https://dev.epicgames.com/documentation/en-us/unreal-engine/data-validation-in-unreal-engine (메인 확인) | 확실에 가까움 |
| Unity | 파일이 바뀌면 가져오기(import) · 오류는 콘솔 | 바뀐 파일 | 근거 없음 | https://docs.unity3d.com/Packages/com.unity.asset-manager-for-unity@1.7/manual/import-assets.html | 불확실 |
| Godot | 바뀌면 훑기 · 가져오기 · 깨진 의존은 씬을 열 때(「Fix Dependencies」) | 바뀐 파일 / 연 씬 | 근거 없음 | (서드파티 글) | 불확실 |
| ajv | 한 번 컴파일 · 여러 번 실행 | — | — | https://ajv.js.org/guide/getting-started | 패턴 = 확실 |

## 우리 설계에 주는 것

- 문법(yaml 읽기)은 훑기가 어차피 `kind` 를 읽으려고 파일을 연다 — 그때 나는 문법 오류는 공짜로 ⛔.
- 스키마 내용 검사는 **고를 때(카드를 열 때) + 바뀐 파일을 다시 읽을 때** 하고 결과를 (경로 · 수정 시각 · 크기)로 기억 · 목록 아이콘은 기억된 결과만 보인다(검사 전엔 아이콘 없음) · 라이브러리 전체 검사는 따로 누르는 명령(사람 = 메뉴 · LLM = 같은 명령).
- 실행 직전 확인(claude doctor · codex 모델 목록)은 별개로 그대로.

## 공백

- VS Code 탐색기 · 문제 칸이 안 연 파일을 보이는지 · JetBrains 프로젝트 보기 표시 · Unity · Godot 표시 · Obsidian 전체 검사 — 1차 출처 없음.
