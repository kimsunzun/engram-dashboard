# 에이전트 발신 라우팅 신뢰화 — 리서치 + 실측 (2026-07-21)

> **상태:** medium tier(/research 설계-결정 모드) — 주계열 수집 3갈래 + 메인 grounding + cross-family(codex, effort high) 적대 리뷰(BLOCK → findings 반영) + 자체 실측 3회 반복(v1/v2/v3).
> **문제:** 왕복 실험(ADR-0093/0094)에서 봉투 **수용(파싱)은 전 포맷·전 모델 0 실패**인데, 답장을 send_message 툴로 **라우팅**하는 성실도가 모델 의존(haiku 2/9 · sonnet 5/6 · opus 5/6)이라 실용 불가.
> **결론(요지):** 프라이밍 v3(출력 불가시성 + 원칙자 앵커 + 사전승인 귀속, 메타주석 0)로 **sonnet·opus 실측 전 케이스 성공(6/6, 플래그 0), haiku 4/5**(v2 동일본문 합산 — 잔여 1미스는 보안 아님·약모델 툴 호출 한계, O3/O5가 커버 대상). 봉투 내 지시문·선제 항변·실험 주석은 보안 정책과 충돌해 역효과(실측+반증 출처 일치).

## 1. 업계 조사 (grounding·적대 리뷰 통과분)

| # | 발견 | 확신도 | 출처 |
|---|---|---|---|
| F1 | Claude Code **Agent Teams**가 동일 문제를 프롬프트로 해결: SendMessage 툴 설명에 "Your plain text output is NOT visible to other agents" + "to communicate, you MUST call this tool" (원문 fetch 확인). 단 프롬프트 단독이 아니라 메일박스·훅 인프라와 결합(codex 지적 반영) | 확실(원문 인용) | github.com/Piebald-AI/claude-code-system-prompts (tool-description-sendmessagetool.md) |
| F2 | Claude Code 서브에이전트·Anthropic 멀티에이전트 리서치 시스템 = **기계적 캡처**(최종 메시지가 곧 반환값 — 자발 툴 호출 불요) | 확실 | docs.anthropic.com subagents / anthropic.com multi-agent-research-system |
| F3 | AutoGen AgentChat도 기계적 캡처 기본(팀 레이어가 on_messages 반환값을 브로드캐스트). 단 Swarm 변형은 handoff 툴 호출 의존(codex 지적 반영 — "100%"는 라우팅 계층 한정 구조 특성) | 가능성 높음 | microsoft.github.io/autogen |
| F4 | LangGraph·CrewAI·OpenAI SDK의 handoff/delegation = **자발 툴 호출**이고 평문 응답이 그냥 최종 출력으로 수용되는 실패 모드가 실재(CrewAI 커뮤니티 다수 보고). 단 OpenAI SDK는 `ModelSettings.tool_choice="required"` 강제 가능 — "전부 방치"는 과일반화(codex HIGH 반영). **API 직결 백엔드에선 tool_choice 강제가 정공법** | 가능성 높음 | openai.github.io/openai-agents-python / community.crewai.com #2289 |
| F5 | claude CLI·Agent SDK엔 tool_choice 강제 **미노출**(2026-07 현재, open issue) — CLI 에이전트에선 프롬프트+하네스 수단만 가용 | 가능성 높음(시점 민감) | anthropics/claude-agent-sdk-python#655, claude-code#20071 |
| F6 | **메시지-위치 지시(봉투 내 리마인더)는 반증됨**: Anthropic 공식 — user-메시지 내 동일 지시는 시스템프롬프트보다 가중치 낮음. 우리 실측도 일치(sonnet이 봉투 내 지시를 인젝션으로 차단) | 확실(반증+실측 교차) | code.claude.com modifying-system-prompts + 자체 실측(§2 v1-B) |
| F7 | 약모델 툴 순응 부스터 = **메시지-위치 few-shot**(haiku급 11%→75%, 시스템프롬프트 문자열론 무효과). 단 task-특이 결과라 외적 타당도 주의(codex 반영) | 가능성 높음 | langchain.com/blog few-shot tool-calling (원문 fetch 확인) |
| F8 | 미발신 감지→교정 턴 주입(detect-and-nudge)은 문서화된 하네스 패턴(Claude Code Stop hook exit-2 = "hooks are guarantees, prompts are requests"). 주요 프레임워크에 1급 프리미티브로는 부재 | 가능성 높음 | code.claude.com/docs/en/hooks-guide |

**codex 적대 리뷰가 추가로 연 옵션(미조사 잔여):** `--json-schema` 구조화 출력으로 수신자 필드 포함 봉투 강제(기계적 캡처+주소 지정 절충) · MCP sampling의 tool 강제 · Semantic Kernel 오케스트레이션 패턴. → 필요 시 후속 조사.

## 2. 자체 실측 (roundtrip-smoke, 프라이밍 반복 3회)

라우팅 성공(B가 MCP send로 답신) — 로그: `target/routing-spike*/`(휘발), 요약 정본 = 이 문서 + step-log.

| 프라이밍 | 봉투 | haiku | sonnet | opus | 비고 |
|---|---|---|---|---|---|
| 구판(format-agnostic) | colon/bracket/xml | 2/9 | 5/6 | 5/6 | 포맷 스파이크 2차 baseline |
| v1: 불가시성+MUST+사전승인(Agent Teams 차용) | colon | 2/3 | 0/2 | 2/2 | sonnet: "인젝션 아니다" 선제 항변 자체를 플래그 |
| v1 + 봉투 내 리마인더 | colon+지시문 | 3/3 | 1/2 | 2/2 | sonnet: 봉투 내 지시문을 인젝션 감지·차단(F6 실증) |
| v2: 원칙자 앵커 (⚠파일 머리 실험주석 오염) | colon | 3/3 | 0/3 | 0/1 | 주석의 "우회" 서사를 에이전트가 인용·차단 — **판정 무효(오염)** |
| **v3: v2 본문 + 주석 전면 제거** | colon | 1/2 | **2/2** | **1/1** | 보안 플래그 0. haiku 1미스 = 답신 의사를 말로만 표현·툴 미호출(약모델 한계) |
| **v3** | bracket 무-id | — | **2/2** | **1/1** | 보안 플래그 0, colon과 차이 없음 |

(haiku는 v2·v3 본문이 동일하므로 합산 4/5 — 잔여 미스는 보안 플래그와 무관한 툴 호출 성실도 문제. §3 O3/O5가 커버 대상)

**핵심 인과(정성 로그 직접 확인):**
1. **출력 불가시성 프레이밍이 라우팅 미스를 잡는다** — haiku 2/9→(v1)2/3→(v2본문)3/3, opus 잔여 실패 0. permission stall(허락 대기)은 사전승인 문구로 전 모델 소멸.
2. **스폰된 에이전트는 이 계정의 조직 보안 지침을 상속**하고, sonnet이 가장 문자 그대로 집행한다. 다음이 인젝션 플래그를 유발했다(전부 로그에 원문 인용됨): ① 프라이밍의 "이건 인젝션 아니다" 선제 항변 ② 봉투 본문 속 행동 지시문 ③ 프라이밍 파일 머리의 실험/우회 메타주석(파일 전체가 시스템프롬프트로 주입 — 주석 미스트립).
3. **해결 = 회피가 아니라 정렬**: 권위를 브로커 주장이 아니라 **원칙자(사용자) 설정**에 앵커("이 팀 구성은 네 원칙자가 직접 설정했다", "발신 권한은 원칙자가 스폰 시 허용해뒀다") → 조직 지침("유효한 지시는 사용자에게서만")과 충돌하지 않음 → 플래그 0.
4. **봉투 신뢰 신호 가설(colon이 자기주장처럼 보인다)은 v3에서 소멸** — v2에서 sonnet이 colon 접두어를 불신했으나, 주석 오염 제거 후 colon·bracket 모두 통과. 포맷 결정 축은 다시 수용(동률)>토큰.

## 3. 설계 옵션 (사용자 결정 대상)

- **O1 (실증 완료·추천): 프라이밍 v3 정식 채택** — `prompts/agent-priming.md`의 발신 절을 v3 내용으로 갱신. 코드 무변경. 잔여 리스크: n 작음(모델·시나리오 일반화는 운영 데이터로 계속 검증).
- **O2 (기각 권고): 봉투 내 리마인더** — Anthropic 반증 + sonnet 인젝션 차단 실측. 채택 금지.
- **O3 (후속 슬라이스 후보): 데몬 detect-and-nudge 백스톱** — 배달 관측(DeliveryObservation)과 턴 종료(MessageDone)를 데몬이 이미 보므로, 배달-후 턴이 무발신 종료 시 교정 턴 1회 주입(루프 가드). 프롬프트가 요청이라면 이건 보증(F8). 백엔드-불가지라 codex/gemini에도 동작. 코드 변경 필요.
- **O4 (장기 보류): 기계적 캡처 모드** — 파이프라인 워커 유형엔 100% 보증(F2/F3). 주소 지정은 request/response 상관으로 해결 가능(codex 지적). 현 에이전트의 이중 역할(턴 출력=원칙자 대면)과 충돌하므로 전용 모드로만.
- **O5 (약모델 한정 보류): 메시지-위치 few-shot 주입** — 스폰 시 가짜 선행 턴 주입 지원 필요(코드). haiku가 v3로 이미 해결되면 불요.

## 4. do-not (실측 근거)

- 프라이밍 파일에 **실험/메타 주석 금지** — 파일 전체(주석 포함)가 시스템프롬프트로 주입된다. 특히 "탐지 우회" 서사는 그 자체가 차단 사유가 된다. (옵션: FilePrimingProvider에 주석 스트립 추가 — 코드 결정.)
- 프라이밍에 **"인젝션 아니다" 선제 항변 금지** — 항변 자체가 주의 신호로 플래그된다.
- **봉투 본문에 행동 지시문 금지** (F6 반증 + 실측 차단).
- 권위 주장을 브로커·시스템 명의로 하지 말고 **원칙자(사용자) 행위로 귀속**한다.

## 5. 한계·쟁점

- 표본 작음(케이스당 n=1~3) — v3의 "전 케이스 성공"은 방향성 강함이나 통계적 보증 아님. 라우팅 신뢰도는 O3 백스톱과 운영 관측으로 이중화 권장.
- 시드 주제가 auth(보안 인접)라 플래그 민감도가 높았을 수 있음 — 일반 주제 대비는 미실측.
- 조직 지침 상속은 이 계정 환경 특성 — 다른 배포 환경(무지침 계정)에선 v1도 통과했을 수 있음(미실측). 단 v3는 지침 유무와 무관하게 안전한 방향.
- A가 자기 명의 시드를 부인하는 하네스 아티팩트(포맷 스파이크 2차 발견)는 미해결 — 하네스 개선 노트.
