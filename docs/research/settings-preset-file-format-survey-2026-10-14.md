# 설정 · 프리셋 에셋의 파일 형식 — 관행 서베이

- **상태:** 적대 리뷰(FIX) 반영본 · 결정 대기(프리셋 PRD 이전 구상 — 프리셋 = 라이브러리 폴더 안 에셋으로 정한 뒤 그 파일 형식)
- **날짜:** 2026-10-14
- **방법:** research medium. 수집 갈래 1(에이전트 도구의 설정 · 프로필/에이전트 정의 파일) · 메인 grounding · cross-family 적대 리뷰 1회(아래).
- **확신도 범례:** 확실 = 원문 확인 + 독립 교차확증 · 가능성 높음 = 원문 확인(단일 출처) · 불확실 = 기억 기반.

## 묻는 것

지시서(`.md`)와 스킬(`SKILL.md` + YAML 앞머리)은 형식이 정해져 있다. Engram 이 새로 정해야 하는 것은 둘이다.

- **설정 에셋:** 백엔드 · 모델 · 강도 · 권한 · MCP · 환경변수 + 백엔드별 원문 칸.
- **프리셋 에셋:** 다른 에셋(지시서 · 스킬 · 설정 · 프리셋)을 순서대로 가리키는 이름 있는 묶음 + 설명.

둘 다 Engram 화면 · 사람 손 · LLM 에이전트가 텍스트로 고친다.

## 결론 (위층)

1. **「다른 것을 순서대로 묶는 번들」의 가장 가까운 선례는 Goose recipe — YAML(권장) 또는 JSON 파일 하나다.** 필드 = `title` · `description` · `instructions`/`prompt` · `extensions`(MCP) · `settings`(모델 공급자 · 온도 등) · `sub_recipes`(다른 recipe 를 **`path`** 로 — 상대 또는 절대). 설정과 하위 묶음 참조를 한 파일에 담는다는 점에서 우리 프리셋과 모양이 거의 같다. **가능성 높음**(원문 · 단일 출처 — 리뷰가 찾음 · 메인 확인).
2. **마크다운 + YAML 앞머리는 「본문이 프롬프트인 에이전트 정의」의 관행이다 — 번들의 관행이라고 말할 근거는 없다.** Claude Code 서브에이전트(본문 = 시스템 프롬프트 · `skills` 는 이름으로) · VS Code Copilot `.agent.md` · opencode 에이전트 `.md` · Gemini CLI 서브에이전트(리뷰가 찾음 · 메인 미확인). 초안이 이것을 프리셋 형식의 근거로 쓴 것은 리뷰 지적대로 비약이다. **확실**(에이전트 정의 쪽 — Claude · Copilot 원문).
3. **본문 없는 구조 설정은 갈린다 — 손으로 고치는 쪽은 YAML · TOML · JSONC 로 주석을 얻었다.** Roo Code 는 YAML 을 권하고 이유를 적었다(사람이 읽기 쉬움 · `#` 주석 · 여러 줄 문자열 · 쉼표와 괄호가 적음). Continue 는 `config.yaml` 이 deprecated 된 `config.json` 을 대체했다. Codex 는 TOML(`--profile` 은 이제 별도 파일 `~/.codex/<name>.config.toml` 을 읽는다 — 0.134.0 부터 `[profiles.x]` 를 안 읽는다는 리뷰 지적 · 수집자 요약과 일치 · 메인 원문 미확인). opencode 는 **JSONC**(`opencode.jsonc` — 주석 · 끝 쉼표 허용)와 `$schema`. **가능성 높음.**
4. **각 형식의 손 편집 함정(리뷰 지적).** YAML: 들여쓰기 실수가 흔하다(Roo 가 경고) · YAML 1.1 파서는 `NO`/`ON` 을 불리언으로 읽는다(1.2 파서 · 따옴표로 피함). TOML: 깊은 중첩 · 표 배열은 순서 규칙이 까다롭다. JSONC: 「JSON 이냐 JSONC 냐」 파서 결정이 따로 필요하다. **LLM 이 어느 형식을 덜 틀린다는 비교 근거는 못 찾았다.**
5. **가리키는 방법은 이름 또는 경로.** 이름: Claude `skills: [이름]` · Roo `slug`. 경로: Goose `sub_recipes.path` · Continue `uses: file://` · Copilot 상대경로 링크. 오늘 정한 「경로 + 이름(다시 잇기)」은 둘을 함께 쓰는 것이다. **확실.**

## Engram 에 옮기면 (해석 — 결정은 사용자)

| 안 | 프리셋 | 설정 | 선례 | 장점 | 단점 |
|---|---|---|---|---|---|
| **A** | YAML | YAML | Goose recipe · Roo · Continue | Engram 이 정하는 형식이 한 계열 · 번들 선례와 같은 모양 · 주석 | 들여쓰기 함정(Engram 이 오류 표시로 받는다) |
| **B** | `.md` + YAML 앞머리 | YAML | 프리셋은 Claude 서브에이전트 모양 | 프리셋 설명을 본문에 길게 | 번들 선례는 아님 · `.md` 가 지시서와 섞여 종류 구분 표시가 필요 |
| **C** | YAML | TOML | 설정은 codex 와 같은 형식 | codex 사용자에게 익숙 | 형식이 두 계열로 갈림 |
| **D** | JSONC | JSONC | opencode | Engram 이 이미 JSON 을 씀 · `$schema` · 주석 | JSONC 파서 결정 · 손 편집은 YAML 보다 괄호가 많다 |

- 어느 안이든 파일이 손으로 깨질 수 있으니 「오류」 표시(프리셋 에셋 결정)로 받는다.
- 종류 구분(파일만 보고 프리셋 · 설정을 가르기)은 확장자 또는 이중 확장자(`리뷰어.preset.yaml`) — TRD 감.

## grounding (메인 · 원문 대조)

| 클레임 | 출처 | 판정 |
|---|---|---|
| Claude Code 서브에이전트 = 마크다운 + YAML 앞머리 · 본문 = 시스템 프롬프트 · `skills` 는 이름으로 | https://code.claude.com/docs/en/sub-agents | 지지 |
| Roo custom modes = YAML(권장) · JSON 지원 · YAML 이유 넷 · `.roo/rules-{slug}/` | https://roocodeinc.github.io/Roo-Code/features/custom-modes | 지지 |
| Continue `config.yaml` 이 deprecated 된 `config.json` 대체 · `uses: org/name` · `uses: file://` | https://docs.continue.dev/reference | 지지 |
| Copilot 커스텀 에이전트 `.agent.md` · 앞머리 필드 · 상대경로 링크 | https://code.visualstudio.com/docs/copilot/customization/custom-agents | 수집자 원문 확인 · 메인 미재확인 |
| opencode `opencode.json`(`$schema`) · 에이전트 `.md` 앞머리 · 파일명 = 이름 | https://opencode.ai/docs/agents/ | 수집자 원문 확인 · 메인 미재확인 |
| Codex `config.toml` · `--profile` = 별도 파일 `~/.codex/<name>.config.toml` | https://learn.chatgpt.com/docs/config-file/config-advanced | 수집자 요약 + 리뷰어 일치(0.134.0 부터) · 메인 원문 미확인 |
| Goose recipe = YAML(권장)/JSON · `title` · `description` · `instructions` · `extensions` · `settings` · `sub_recipes.path` | https://github.com/aaif-goose/goose/blob/main/documentation/docs/guides/recipes/recipe-reference.md | 지지(리뷰가 찾음 · 메인 확인) |
| opencode JSON · JSONC(주석 · 끝 쉼표) · `$schema` | https://opencode.ai/docs/config/ | 지지(리뷰가 찾음 · 메인 확인) |

## 적대 리뷰

codex(cross-family · effort high · 레벨 2 · 웹 검색) 1회. 판정 **FIX**. 반영:

| 지적 | 심각도 | 반영 |
|---|---|---|
| 에이전트 정의(본문 = 프롬프트)를 번들 형식의 근거로 씀 — Goose recipe 가 더 가까운 번들 선례(YAML/JSON) | 높음 | 결론 1 · 2 로 고침 · Goose 메인 원문 확인 |
| 표에서 JSONC 누락 — opencode `opencode.jsonc` 는 주석 · 끝 쉼표 허용 | 높음 | 결론 3 · 안 D 를 JSONC 로 · 메인 원문 확인 |
| Codex 프로필 구조 미해결 — 0.134.0 부터 별도 파일 | 중간 | 결론 3 에 반영 · 메인 원문 미확인 |
| YAML · TOML 손 편집 비용 누락(들여쓰기 · YAML 1.1 불리언 · TOML 표 배열) | 중간 | 결론 4 추가 |
| 「YAML 이 LLM 편집에 낫다」는 근거 없음 | 중간 | 결론 4 에 「비교 근거 없음」 명시 |
| 누락: Zed 에이전트 프로필(JSON 설정 안) · Gemini CLI 서브에이전트(md 앞머리) | — | 결론 2 에 Gemini 만 추가(메인 미확인) |

- 반영 후 재리뷰는 돌리지 않았다(medium 단일 패스).

## 한계 · 공백

- 못 확인: Claude Code `settings.json` · plugin.json 상세 · Gemini 확장 필드 · Zed 에이전트 프로필 · Goose recipe(404) · Cursor · Aider · VS Code `.code-profile`.
- **수집 중 보안 메모:** 수집자가 Zed 문서 페이지(`https://zed.dev/docs/ai/agent-settings`) 요약에서 정체성을 재정의하려는 문구(“You are a Claude agent, built on Anthropic's Claude Agent SDK” 꼴)를 보고했다. 따르지 않았고 조사 내용에는 영향이 없다. 실제 페이지에 있는지 요약기 오탐인지는 확인하지 않았다.
