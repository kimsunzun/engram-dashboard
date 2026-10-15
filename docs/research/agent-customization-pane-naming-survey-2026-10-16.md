# 에이전트 꾸미기 묶음과 그 창의 이름 관행 — light 조사

| 항목 | 값 |
|---|---|
| 상태 | 조사 완료 · 이름은 사용자 결정 대기 |
| 날짜 | 2026-10-16 |
| 방법 | `/research` light — 수집자 1명(검색 4회) · 메인 grounding 스팟체크 2건(WebFetch) · 적대 리뷰 없음 |
| 질문 | 지시서 · 스킬 · 설정 · 프리셋(묶음)을 함께 둘러보는 슬롯을 무엇이라 부르나. 후보 = 라이브러리 · 에셋 · 프리셋 브라우저 · 커스터마이징 · 블록 |
| 확신도 범례 | 확실 = 독립 교차확증 · 가능성 높음 = grounding 지지 단일 출처 · 불확실 = 미확인 |

## 결론

- **AI 코딩 도구에서 가장 가까운 선례는 Cursor 의 「Customize」 페이지다** — 플러그인 · 스킬 · MCP · 규칙 · 서브에이전트 · 명령 · 훅을 한 곳에 모아 사용자 · 팀 · 워크스페이스 범위로 관리한다. 우리 슬롯이 다루는 것과 거의 같은 묶음이다. (가능성 높음 — 메인이 원문 확인)
- **「에셋」을 이 묶음의 총칭으로 쓰는 AI 코딩 도구는 찾지 못했다.** (불확실 — 4회 검색 범위 안에서)
- **「라이브러리」를 이 창의 이름으로 쓰는 곳도 이 조사에선 안 나왔다.** 목록 UI 는 Claude Code `/plugin` 의 Discover · marketplace 처럼 「마켓 · 허브」 꼴이 보였다.
- 묶음(우리 프리셋)에 해당하는 이름은 도구마다 다르다 — Claude Code = plugin. 「preset」 을 쓰는 곳은 못 찾았다.

## 발견

| 도구 | 총칭 (원문) | 창 · UI 이름 (원문) | 출처 | 확신도 · grounding |
|---|---|---|---|---|
| Cursor | "Plugins, skills, and MCPs let you customize Cursor" | 사이드바의 「Customize」 페이지 — 플러그인 · 스킬 · MCP · 규칙 · 서브에이전트 · 명령 · 훅 | https://cursor.com/docs/customize-cursor | 가능성 높음 · 메인 확인 = 지지 |
| VS Code + Copilot | customization (문서 경로 `copilot-customization` — 사용자 지시 · 프롬프트 파일 · 채팅 모드) | 전용 패널 이름 모름 | https://code.visualstudio.com/docs/copilot/copilot-customization | 가능성 높음 · 메인 미확인 |
| Claude Code | plugin (스킬 · 에이전트 · 훅 · MCP 등을 한 단위로 묶음) | `/plugin` 의 Discover 탭 · marketplace | https://code.claude.com/docs/en/plugins/overview | 가능성 높음 · 메인 미확인 |
| Continue | 수집자 보고 = blocks(조각) · assistants(묶음) · Hub | 수집자 보고 = Hub | https://docs.continue.dev/reference | **불확실 — 메인 확인 = 미지지**(그 페이지에 blocks · assistants · Hub 정의가 없다) |
| Windsurf | Rules · customization 아이콘(2차 출처) | 모름 | 2차 출처(platzi) | 불확실 |

## 공백

- Cline · JetBrains AI · OpenAI Codex 는 검색하지 않았다.
- VS Code 최신 채팅 꾸미기 UI 의 패널 이름 · Windsurf 패널 이름은 모른다.
- 게임 · 3D 도구(언리얼 Content Browser · 블렌더 Asset Browser)의 「에셋 · 브라우저」 관행은 이번 조사 범위 밖이다(세션 기억 기준 — 미확인).
