# 프리셋 편집 칸 — 직접 담은 것과 안에 든 프리셋에서 온 것을 어떻게 보여 주나 (피어 서베이)

- **상태:** 적대 리뷰(FIX) 반영본 · 결정 대기(프리셋 PRD 이전 구상 — 사용자 규칙 「레이아웃은 관례 기준으로」. 세션이 낸 「담은 것 | 최종 구성」 탭은 피어 확인 없는 세션 발상이었다)
- **날짜:** 2026-10-15
- **방법:** research medium · 설계-결정 모드. 수집 갈래 2(AI 에이전트 · 개발 도구 · 상속/중첩이 있는 콘텐츠 도구) · 메인 grounding · cross-family 적대 리뷰 1회(아래).
- **확신도 범례:** 확실 = 원문 + 독립 교차확증 · 가능성 높음 = 원문(단일) 또는 부분 지지 · 불확실 = 기억 · 요약 · 원문 미확인.

## 묻는 것

프리셋은 지시서 · 스킬 · 설정 · 다른 프리셋을 담는다(중첩). 편집 칸에서

1. 직접 담은 것과, 안에 든 프리셋을 통해 들어온 것을 어떻게 구분해 보여 주나 — 따로 된 「최종(적용 결과)」 보기? 한 목록 안에 출처 표시? 펼침 트리? 켜고 끄는 단추?
2. 종류별 칸으로 나누나, 한 목록에 섞나?
3. 순서를 사용자가 바꾸나 — 일부 종류만?
4. 담기는 칸(종류)마다인가, 단추 하나인가?

## 결론 (위층)

1. **종류별로 나눈다 — 이번 표본에서 가장 고르게 보인 관행이다.** Continue `config.yaml` 은 맨 위 칸이 `models` · `rules` · `prompts` · `context` · `mcpServers` 등 종류별이다 · VS Code 프로필은 설정 · 단축키 · MCP 서버 · 스니펫 · 작업 · 확장 단위로 고르고 연다 · Zed 에이전트 프로필은 기본 도구 · MCP 도구 · 기본 모델 · Claude Code 플러그인 상세는 「Will install」 아래 명령 · 에이전트 · 스킬 · 훅 · MCP/LSP 서버 · Chrome DevTools Computed 의 「Group」(접히는 분류, 선택). 섞인 한 목록은 표본에서 못 찾았다. **가능성 높음**(Continue · Claude · Chrome 원문 · VS Code 부분 원문).
2. **직접 vs 물려받은 것 — 콘텐츠 편집기는 「한 목록에 함께 + 줄마다 표시」가 주류다.**
   - **Unity 프리팹 인스턴스** — 물려받은 것과 바꾼 것이 한 인스펙터에 함께 있고, 바꾼 것은 **왼쪽 파란 선 + 굵은 이름**, 더하거나 뺀 컴포넌트는 아이콘에 **+/− 배지**. 「Overrides」 메뉴에서 Apply/Revert(전부 · 고른 것 · 하나씩). **가능성 높음**(원문).
   - **Unreal 블루프린트** — 「Show Inherited Variables」 켜면 부모 변수가 자기 것과 함께 나온다(켜고 끄는 단추). **불확실**(포럼 스니펫).
   - **Chrome DevTools Styles** — 적용되는 규칙 전부(덮인 것 포함)를 보여 주고, 물려받은 것은 「Inherited from <요소>」 구획으로 따로 묶는다. **가능성 높음**(원문).
   - Godot 상속 씬 · Figma 인스턴스(재정의 · Reset) — **불확실**(스니펫 · 기억).
3. **「내가 정한 것」과 「실제 적용 결과」를 두 화면으로 나누는 선례는 하나다 — Chrome DevTools 의 Styles 대 Computed 탭.** Computed = 「실제로 적용되는 CSS 만」. **가능성 높음**(원문). AI 도구 쪽은 적용 결과를 **명령으로만** 본다 — `docker compose config`(합친 결과 출력 · 불확실 — 검색 요약) · `claude plugin list`(의존 상태). GUI 에서 줄마다 「무엇 때문에 들어왔나」를 보여 주는 AI 도구는 확인하지 못했다.
4. **사용자가 순서를 끌어 바꾸는 선례는 이번 표본에 없다.** Claude 플러그인 목록은 정렬 · 묶음(꺼진 것은 아래 접힌 머리 밑)이지 사용자 순서가 아니다 · Continue 는 규칙 순서 · 우선순위를 문서에 적지 않았다. **공백.**
5. **담기는 종류 · 대상 단위가 많다.** Continue 는 칸마다 `uses:` 줄 · Zed 는 종류별 토글 · Claude Code 는 플러그인 찾기(Discover 탭)와 `/plugin install`. 칸마다 `+` 단추를 둔 GUI 는 확인 못 했다. **불확실.**

## Engram 에 옮기면 (후보 — 결정 아님)

- **종류별 칸** — 관행과 맞는다(결론 1). 이미 사용자 결정(2026-10-15).
- **안에 든 프리셋에서 온 것** — 관행은 두 갈래다.
  - (A) **한 목록 + 줄마다 표시 + 켜고 끄기**(Unity · Unreal · DevTools Styles): 각 종류 칸 안에 물려받은 줄을 흐리게 · 「← 공용 규칙」 표시로 함께 보이고, 칸 위 「안에 든 것 보기」로 켜고 끈다. 물려받은 줄은 ×로 못 뺀다(빼려면 그 프리셋을 연다).
  - (B) **두 화면**(DevTools Styles 대 Computed): 세션이 그린 「담은 것 | 최종 구성」 탭.
  - 콘텐츠 편집기(우리 라이브러리 슬롯과 가까운 쪽)는 (A) 가 많다.
- **순서** — 선례가 없다. 지시서 · 설정은 이어 붙는 순서가 결과를 바꾸므로 순서를 다룰 길은 필요하다 — 끌어 바꾸기는 우리 필요에서 나온 것이지 관행이 아님을 적어 둔다.
- **칸마다 `+`** — 선례 미확인. 사용자가 편하다고 정했다.

## grounding (메인 외부 체크)

| 클레임 | 출처 | 판정 |
|---|---|---|
| Unity 파란 선 · +/− 배지 · Overrides Apply/Revert | https://docs.unity3d.com/Manual/PrefabInstanceOverrides.html (요약기 경유 원문 인용) | 지지 |
| DevTools Styles = 덮인 것 포함 전부 · 「Inherited from」 · Computed = 실제 적용 · Group 체크박스 | https://developer.chrome.com/docs/devtools/css/reference (요약기 경유 원문 인용) | 지지 |
| Claude Code 묶음 = `dependencies` 만 가진 플러그인 · 상세 「Will install」 종류별 · 꺼진 플러그인 아래 접힘 | https://code.claude.com/docs/en/plugin-dependencies (메인 원문 직접 — 묶음 부분) · install 문서(수집자) | 부분지지(묶음 = 메인 확인 · UI 문구 = 수집자) |
| Continue `config.yaml` 종류별 맨 위 칸 · `uses:` | https://docs.continue.dev/reference (수집자) | 미확인(메인 대조 안 함) |
| VS Code 프로필 종류 단위 · Zed 프로필 종류 | 수집자(부분 원문) | 미확인 |
| Unreal Show Inherited Variables · Godot · Figma · docker compose config | 수집자(스니펫 · 기억) | 불확실 |

## 적대 리뷰

codex(cross-family) · effort high · 레벨 2 · 웹 검색 · 판정 **FIX**. 반영(이 절이 위 결론보다 우선한다):

1. **결론 2 는 과장이다(높음).** Unity 는 「내가 바꾼 것」 표시이고 Chrome 은 DOM 조상에서 물려받은 CSS 묶음이라, 둘 다 **중첩 묶음이 넣어 준 항목의 줄별 출처 표시** 선례가 아니다. 둘 다 부모 하나의 **상속**이다 — 우리 프리셋은 여러 프리셋을 **포함**한다(사용자도 같은 점을 짚었다: 「프리셋을 펼쳐 보는 게 아니라 각 칸에 묻히는 거냐」). → (A) 를 「관례상 주류」로 쓰지 않는다. 더 가까운 피어는 VS Code 프로필(종류 단위로 기본 프로필에서 통째로 물려받음).
2. **두 화면 선례 추가(중간)** — Argo CD ApplicationSet UI: 편집기 + 만들어질 자식 앱 미리보기(리뷰어 보고 · 메인 미확인).
3. **섞인 한 목록 · 중첩 GUI 누락(중간)** — Ableton 랙: 랙 안에 랙(중첩 — 메인 원문 확인: 「you will create a Rack within a Rack」), 체인 안에 여러 종류 장치가 섞인다 · 랙은 그 자리에서 접고 편다(「Fold」 — 메인 원문 확인). **포함 관계를 그 자리에서 펼쳐 보이는 선례**다.
4. **결론 4 「순서 바꾸기 선례 없음」 반박(높음)** — Ableton 은 랙 체인 목록 · 장치 순서를 바꾸고 그 순서가 결과를 바꾼다(리뷰어 보고). 메인 원문 대조에서는 「Racks, chains and devices can be freely dragged into and out of other Racks」만 확인 — 체인 순서 문장은 확인 못 함 → **부분지지**.
5. **결론 5 「칸마다 `+` 선례 미확인」 반박(중간)** — **Unreal My Blueprint 패널: 칸 제목마다 만들기 단추 + 맨 위 「Add New」** (메인 원문 확인: 「shortcut buttons (… at the end of each heading) for creating new variables, functions, macros…」 · 「the Add New button at the top of the My Blueprint Panel」). 우리 결정(칸마다 `+` + 위 「+ 담기」)과 같은 모양이다.
6. Claude 「Will install」 문구 출처는 install 문서(오귀속 — 낮음).

## 공백 · 한계

- 순서 바꾸기 · 칸마다 `+` 의 선례 없음.
- 못 본 곳: Cursor 규칙 화면 · Goose 레시피 편집기 · Dify · Flowise · LangSmith · Copilot 사용자 에이전트 · JetBrains · Kustomize · Blender 라이브러리 재정의 · Ableton 랙.
- GUI 화면 문구 · 스크린숏은 거의 못 얻었다(문서가 화면을 잘 안 적는다).
