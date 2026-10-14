# 에이전트 프리셋 부품 서베이 — 무엇을 묶고 어떻게 짜나 (2026-10-11)

- **상태:** 조사 완료 · 결정 전(스킬 프리셋 PRD 이전 — 선행 조사 = `external-skill-dir-loading-claude-codex-2026-10-10.md`)
- **방법:** `/research` 연장 · 수집자 1(문서 · 로컬 클론) · 메인 대조(paseo 소스 줄) · **cross-family 적대 리뷰 생략**(열화 — 부품 목록 서베이라 결론이 사용자 결정을 대신하지 않는다)
- **확신도:** 확실 = 공식 문서 · 소스 직독 · 가능성 높음 = 2차 출처 · 불확실 = 미검

## 부품 종류 × 도구

| 부품 | Claude Code | Codex CLI | 실행마다 주입 |
|---|---|---|---|
| 지시서 | `--append-system-prompt(-file)` · CLAUDE.md · 출력 스타일 | `developer_instructions` · AGENTS.md | 둘 다 됨(확실 · codex 한계는 선행 보고서) |
| 스킬 | SKILL.md(Agent Skills 표준) | 같은 형식 | claude `--add-dir`/`--plugin-dir` · codex app-server `extraRoots`(확실) |
| MCP 서버 | `--mcp-config` · `.mcp.json` · 플러그인 | `[mcp_servers.<id>]` · 플러그인 `mcp.json` | 됨(확실) |
| 훅 | settings `hooks` · 플러그인 · 에이전트 frontmatter | `hooks` · 플러그인 | claude `--settings` 로 됨(확실) |
| 서브에이전트 정의 | `.claude/agents/*.md` · `--agents` JSON | 문서에서 못 찾음(불확실) | claude 됨 |
| 슬래시 명령 · 프롬프트 | `commands/`(스킬 권장) | custom prompts(불확실) | 파일 · 플러그인만 |
| 모델 · 추론 강도 | `--model` · `--effort` | `model` · `model_reasoning_effort` | 됨(확실) |
| 권한 · 샌드박스 · 도구 | `--permission-mode` · `--allowedTools`/`--disallowedTools` | `approval_policy` · `sandbox_mode` | 됨(확실) |
| 환경변수 | 프로세스 env | 프로세스 env · `shell_environment_policy` | 됨 |
| 추가 폴더 | `--add-dir` | `--add-dir`(가능성 높음) | 됨 |
| 시작 프롬프트 | 위치 인자 · 에이전트 `initialPrompt` | 위치 인자 | 됨 |
| 묶음(플러그인) | `plugin.json`: 스킬 · 명령 · 에이전트 · 훅 · MCP · LSP · 출력 스타일 등 · `--plugin-dir` 세션 한정 | `plugin.json`: 스킬 · `mcp.json` · 앱 · 훅 | claude 세션 한정 · codex 실행 인자 없음(불확실) |

- **백엔드 공통 핵심(수집자 정리):** 지시서 · 스킬 · MCP · 모델 · 추론 강도 · 권한 등급 · 환경변수 · 작업 폴더/추가 폴더 · 시작 프롬프트. **백엔드 전용:** 훅 · 출력 스타일 · 서브에이전트 · 플러그인.

## 묶음을 짜는 방식

- **파일 하나에 평평한 필드:** claude 서브에이전트 frontmatter · Copilot `.agent.md`(YAML 필드 + 본문 프롬프트). https://code.claude.com/docs/en/sub-agents
- **겹쳐 쌓기:** codex = 시스템 < 관리 < 사용자 < 프로젝트 < 프로필 < CLI `-c`. claude = 관리 > CLI > 로컬 > 프로젝트 > 사용자. https://learn.chatgpt.com/docs/config-file/config-reference
- **조합 묶음:** 플러그인(claude · codex · Gemini) = 부품 폴더 + 매니페스트. https://code.claude.com/docs/en/plugins-reference
- **상속 · 백엔드 중립 실행 기록(가장 가까운 피어):** paseo `ProviderOverride`(`extends` · `label` · `command` · `env` · `params` · `models` · `disallowedTools` · `enabled` · `order` — `packages/protocol/src/provider-config.ts:47-59`)와 백엔드 중립 `AgentSessionConfig`(`systemPrompt` · `daemonAppendSystemPrompt` · `modeId` · `model` · `thinkingOptionId` · `featureValues` · `toolPolicy` · `mcpServers` · `env` … — `packages/server/src/server/agent/agent-sdk-types.ts:575-612` · 각 백엔드가 자기 필드로 옮긴다). 확실(메인 직독).
- orca 는 신뢰 프리셋뿐 · t3code · vibe-kanban 은 프로필 개념 못 찾음(grep · 불확실). Zed · Continue · Cursor · Kiro · Cline/Roo 내부는 미검.

## 도구 간 표준

- **Agent Skills**(agentskills.io) — SKILL.md(`name` · `description` · `license` · `compatibility` · `metadata` · 실험 `allowed-tools`) · claude · codex 둘 다 읽는다(확실).
- **Agent Plugins 1.0**(2026-08-06 발표라는 2차 출처 하나 · 공식 규격 못 찾음 · 가능성 높음) — `plugin.json` + `skills/` + `mcp.json`. codex 문서가 이식형 루트 `plugin.json` 을 확인한다.
- AGENTS.md · MCP · ACP — 재확인 안 함. ACP 는 세션 규약이지 부품 묶음이 아니다.

## 추가 (2026-10-12) — UI 화면 서베이 · 소스 매칭 관행 · claude 모델 목록

UI 브리핑(아티팩트 「Engram 프리셋 화면」)을 만들며 모은 것. 적대 리뷰 없음(열화).

- **UI 패턴(문서 · 소스 · 화면 캡처는 못 봄):** Claude Code `/plugin` = 탭(Discover · Installed · Marketplaces · Errors) + 상세 창의 「Will install」 목록과 맥락 비용(매 턴 / 호출 시 토큰) · 범위 user/project/local · 관리 항목은 잠긴 채 보임. `/agents` 마법사는 v2.1.198 에서 걷히고 파일 + 「에이전트에게 시키기」로 갔다(문서). Cursor Rules = 항목마다 적용 방식 4가지 + 층 순서 Team > Project > User(펼친 결과 보기 없음). Zed = 패널의 프로필 피커 + 「Configure」 모달(생성 · 포크 · 삭제 · 모델 · 도구) — 재사용은 포크뿐. VS Code Profiles = 빈 것 / 복제 / 템플릿으로 만들고 종류별 켬 · 끔 · 안 켠 종류는 Default 를 상속 · 창마다 적용. Copilot custom agents = `.agent.md` · 채팅 드롭다운. Roo modes = 폼(이름 · 설명 · 역할 · 도구 묶음 · 지시) · 프로젝트가 같은 slug 의 전역 모드를 통째로 덮음. paseo = 설정 페이지의 에이전트 프로필(공급자 + 모델 + 모드 + 생각 + 기능값) · 참조 대상이 사라져도 저장된 id 로 목록 줄을 그린다(`packages/app/src/agent-profiles/`). orca = 검색 콤보 + 「Manage agents」 + 우클릭 기본값. t3code = 공급자별 묶음 모델 피커(`components/chat/ModelPickerContent.tsx`). **중첩 묶음 + 끌어서 순서 + 펼친 결과 보기를 함께 가진 도구는 못 찾았다(가능성 높음).**
- **소스 매칭 관행(조사자 기억 기반 · 문서 미검증):** 반복되는 모양 = 폴더 안에 이름 · id, PC 설정에 경로(Claude 마켓 `marketplace.json` name + `plugin@마켓` · Go `go.mod` + `go.work` · Bazel `MODULE.bazel` + `local_path_override` · Syncthing 폴더 ID). 피할 예 = Obsidian(id 가 PC 앱 설정에만 있어 PC 끼리 안 맞음). URL 식별(submodule · flake · Terraform)은 fork · 미러 · 하위 폴더 구분에 약하다. 조사자 권장 = 폴더 안 사람이 읽는 id(`engram-source.toml`) + PC 설정은 id → 경로.
- **claude 모델 목록(실측 · 2.1.284):** JSON 통로(`-p --input-format stream-json --output-format stream-json --verbose`)에 첫 메시지 전 `{"type":"control_request","request_id":"…","request":{"subtype":"initialize"}}` 한 줄 → `control_response` 의 `response.response.models`(5개 — `default` · `opus` · `claude-fable-5-1[1m]` · `sonnet` · `haiku`). 모델마다 `supportedEffortLevels`(low~max) · `supportsEffort` · `supportsFastMode` 등. 토큰 0(사용자 메시지 없음). 같은 응답에 계정 정보(`account`)가 들어 있다 — 기록 · 로그에 남기지 말 것. Agent SDK 의 `supportedModels()` 가 이 응답을 읽는다(`@anthropic-ai/claude-agent-sdk` 0.3.296). 목록은 계정 · 요금제에 따라 다를 수 있다. codex 는 app-server `model/list`(모델마다 지원 강도 · 기본 강도 — 0.156.1 스키마).

## 추가 (2026-10-12 오후) — 묶음에 담기 · 태그 · 공통+대상별 설정

조사자 1(웹) · 문서로 연 것은 VS Code Profiles · codex 설정 순서뿐, 나머지는 **기억 기반 미검증**(Linear 라벨 문서 404 · 언리얼 Collections 본문 비어 있음). 적대 리뷰 없음.

- **담기:** 검색형 피커와 항목 쪽 「…에 추가」 메뉴는 수백 개까지 버티고, 좌우 이동 목록 · 끌어다 놓기는 수십 개가 적정(해석). 이미 든 항목은 체크 · 흐림 · 「N개에 포함」 배지가 공통. 양방향 진입 사례 = Spotify · 언리얼 Collections(미검증). 조사자 권고 = 검색 피커 + 항목 우클릭 둘 다, 순환은 피커에서 비활성.
- **태그:** Obsidian 중첩 태그(`a/b` · 파일 안 · git 이식) · Gmail 중첩 라벨(앱 저장) · Lightroom 계층 키워드 + 키워드 세트 · Linear 라벨 그룹(그룹 안 하나만 — 불확실) · 언리얼 Static/Dynamic 컬렉션(미검증). 저장 위치 = 파일 안 / 사이드카 파일 / 앱 DB — 사용자 결정 사안.
- **공통 + 대상별 설정:** VS Code 언어별 블록(`"[python]": {…}`) · EditorConfig 섹션 · docker-compose override · codex = 기본 `config.toml` < 프로필 오버레이 < 프로젝트 `.codex/config.toml` < CLI `-c`(문서 확인 — https://learn.chatgpt.com/docs/config-file/config-advanced). UI 공통 = 공통 탭 + 대상별 탭 + 「덮임」 표시 + 원문 칸.

## 추가 (2026-10-12) — 전체 스냅샷 push 가 앱 관행인가

조사자 1(웹) · 적대 리뷰 없음. **결론: 작은 목록 + 낮은 빈도 + 로컬 IPC 에서 전체 스냅샷 push 는 관행이다(확실).** 부분 갱신(델타)은 크기 · 빈도(초당 수십 회 이상) · 원격 대역폭 · 동시 편집 중 하나가 문제일 때의 최적화다(가능성 높음 · 수치 기준은 못 찾음).
- React 문서(직독): 같은 자리 · 같은 key 면 상태 유지, key 가 바뀌면 다시 만든다 — 깜박임은 전송 방식이 아니라 그리기 문제(안정된 id key · 갱신 중 이전 데이터 유지 · 다시 만들지 않기). https://react.dev/learn/preserving-and-resetting-state
- TanStack Query(직독): 응답에 새 데이터가 있으면 캐시를 새 객체로 갈아끼운다(직접 수정 금지). https://tanstack.com/query/latest/docs/framework/react/guides/updates-from-mutation-responses
- Kubernetes(직독): list(전체) + watch(그 뒤 델타, resourceVersion) — 델타가 끊기면(410) 전체로 되돌아간다. 델타 방식도 전체 스냅샷을 복구 경로로 둔다. https://kubernetes.io/docs/reference/using-api/api-concepts/
- Engram 함의: 단조 번호로 옛 스냅샷을 버리는 지금 방식 = resourceVersion 과 같은 발상. 델타가 맞는 곳은 이미 seq/replay 를 쓰는 터미널 출력 같은 고빈도 스트림뿐. LSP · Redux · VS Code 웹뷰 · CRDT 항목은 미검증.

## 추가 (2026-10-12) — 「모음에 추가」 팝오버

조사자 1(웹) · 공식 문서 5곳을 열었으나 UI 세부(체크박스 · 검색 · 적용 시점)는 거의 없었다 — **세부 대부분 기억 기반 미검증.** 문서로 확인한 것 = YouTube 「Save」(기존 재생목록 선택 또는 Create new · 추가 뒤 토스트) · Gmail(한 항목에 라벨 여러 개) · 언리얼(우클릭 Add to Collection 하위 메뉴 · 새 컬렉션 · 끌어 넣기 언급). 조사자 권고 = 체크 즉시 적용 + 토스트 되돌리기 · 이미 담긴 곳은 체크 + 위로 · 검색 → 최근 → 전체 → 「+ 새로」(검색어가 없으면 `"검색어"로 만들기`) · 담을 수 없는 곳은 숨기지 않고 비활성 + 사유 · 키보드(열면 검색 포커스 · ↑↓ · Space 토글 · Esc).
