# 글로벌 vs 프로젝트 컨텍스트 분리 패턴 리서치

**상태:** 완료  
**날짜:** 2026-06-27  
**방법:** Claude Sonnet 팬아웃(4갈래) × Codex BLIND 독립 교차 → 교차 대조 + ETH Zurich 논문 적대 검증  
**강도:** deep  
**확신도 범례:** 확실 = 1차 출처 직접 확인 / 가능성 높음 = 복수 출처 수렴 / 불확실 = 단일 출처 or 간접 추론

---

## 교차검증표 (Claude ↔ Codex 수렴/발산)

| 클레임 | Claude | Codex | 판정 |
|---|---|---|---|
| 글로벌 = 개인 선호·스타일, 프로젝트 = 아키텍처·명령 | 수렴 | 수렴 | 합의(확실) |
| 계층 우선순위: 더 특정 범위 우선 | 수렴 | 수렴 | 합의(확실) |
| 핸드오프/세션 문서는 글로벌도 프로젝트도 아닌 별도 레이어 | 암시 | 명시 | Codex 추가 명확화 |
| ETH Zurich: LLM 생성 파일 성공률 ~3% 감소, 비용 20% 증가 | 인용 | 미언급 | 적대 검증 완료 → 확인 |
| 경로별(path-specific) rules가 세 번째 레이어 | Claude 확인 | Codex 명시 | 합의(확실) |
| AGENTS.md = repo 단위(글로벌 아님) | 수렴 | 수렴 | 합의(확실) |

---

## 패턴 1: 4-레이어 계층 (Claude Code 공식 모델)

**글로벌에 들어가는 것:**
- 개인 코딩 선호 (타입 엄격도, 에러 처리 방식, 응답 언어)
- 모든 프로젝트에 반복되는 작업 방식 ("커밋은 요청 시만" 등)
- 조직 전체 보안/컴플라이언스 기준 (managed policy 레이어)

**프로젝트에 들어가는 것:**
- 빌드/테스트/린트 명령어 (새 팀원이 반복 물어볼 것)
- 아키텍처·디렉터리 경계·프레임워크 선택
- 프로젝트 코딩 규칙·PR 절차·알려진 함정
- 팀 공유 관례 (git을 통해 팀 전체 적용)

**경계 기준 원칙:**
- "나는 이렇게 일한다" → 글로벌
- "이 저장소는 이렇게 동작한다" → 프로젝트
- "이 파일 타입에만 적용" → 경로별(path-scoped) rules
- "이 작업의 현재 상태·열린 이슈" → 핸드오프/세션 문서(별도 레이어)

**실무 함정:** 글로벌에 프로젝트별 내용 복붙 시 context 낭비 누적. 프로젝트에만 넣으면 repo마다 선호가 drift.

**출처:** https://code.claude.com/docs/en/memory  
**확신도:** 확실

---

## 패턴 2: 크로스-도구 공통 구조 (Cursor / Windsurf / Copilot / Aider)

**글로벌에 들어가는 것:**
- Cursor Settings > Rules for AI: 언어 무관 기본 동작, 모든 코딩 작업 공통 규칙
- Windsurf Settings > Cascade > Custom Instructions: 개인 선호도(언어, 주석 방식, 디버깅 스타일)
- Copilot personal instructions: 응답 형식, 설명 스타일
- Aider ~/.aider.conf.yml: 홈 디렉토리에서 먼저 로드

**프로젝트에 들어가는 것:**
- Cursor `.cursor/rules/*.mdc` (버전 관리됨): 프로젝트 관례·팀 공유
- Windsurf `.windsurfrules`: global을 override 가능한 project-specific parameters
- Copilot `.github/copilot-instructions.md`: repo 전체 지침
- Aider `.aider.conf.yml` (git root): 저장소별 설정
- Devin `.devin/rules/`: 팀 공유 지식

**경계 기준 원칙:**
- 충돌 시 프로젝트 규칙이 글로벌 규칙을 override (모든 도구 공통)
- 글로벌 = settings UI에 저장 (버전 관리 외), 프로젝트 = 파일로 버전 관리

**실무 함정:** 두 레이어 모두 모든 세션에 주입되므로 합산 토큰이 증가. 글로벌과 프로젝트가 충돌하면 모델이 임의 선택.

**출처:**
- https://kirill-markin.com/articles/cursor-ide-rules-for-ai/
- https://docs.github.com/en/copilot/how-tos/custom-instructions/adding-repository-custom-instructions-for-github-copilot
- https://datalakehousehub.com/blog/2026-03-context-management-windsurf/
- https://aider.chat/docs/config/aider_conf.html  
**확신도:** 확실

---

## 패턴 3: 핸드오프 문서의 별도 레이어

**글로벌 vs 프로젝트 경계가 갈리는 지점:**

| 레이어 | 내용 | 지속성 |
|---|---|---|
| 글로벌(user) | 개인 선호·스타일 | 영구(모든 세션) |
| 프로젝트 | 아키텍처·관례·빌드 명령 | 영구(팀 공유) |
| 로컬(CLAUDE.local.md) | 개인적 프로젝트별 설정 | 영구(개인 전용) |
| **핸드오프/세션** | **현재 상태·열린 이슈·임시 결정·다음 액션** | **세션 단위(폐기됨)** |

핸드오프 문서는 글로벌·프로젝트 어디에도 속하지 않는 **작업 단위 레이어**. 재사용 가능한 지식(프로젝트 레이어)과 일회성 상태(핸드오프)를 섞으면 컨텍스트 rot 가속.

**경계 기준 원칙:**
- "새 에이전트/팀원이 이 repo에서 일하려면 반드시 알아야 하는 것" → 프로젝트
- "이번 작업 세션의 현재 상태·임시 판단" → 핸드오프 문서(세션 후 폐기 또는 아카이브)

**출처:**
- https://xtrace.ai/blog/ai-agent-context-handoff
- https://blakelink.us/posts/session-handoff-protocol-solving-ai-agent-continuity-in-complex-projects/
- https://hermes-agent.ai/blog/ai-agent-session-handoff-checklist  
**확신도:** 가능성 높음

---

## 패턴 4: 실무 함정 — 글로벌 과적재 vs 프로젝트 고립

### 글로벌에 너무 많이 넣었을 때

1. **어텐션 희석(Context Rot):** 토큰이 증가하면 핵심 지시가 최근 콘텐츠에 묻혀 adherence 저하. "lost-in-the-middle" 현상.
2. **관련 없는 지시 누수:** 프로젝트 A의 특수 규칙이 프로젝트 B에도 적용되어 충돌.
3. **충돌 발생 시 임의 선택:** 두 규칙이 모순되면 모델이 한쪽을 임의 선택.
4. **비용 증가:** 매 세션 토큰 소비. Anthropic 공식 문서: "bloated CLAUDE.md는 실제 지시를 무시하게 한다", 권장 200줄 이하.

### 프로젝트에만 넣었을 때

1. **선호 drift:** 개인 선호가 repo마다 다르게 복제되어 일관성 상실.
2. **세션 간 재설명:** 새 세션마다 같은 작업 방식을 다시 설명해야 함.
3. **보안 기준 누락:** 조직 공통 정책이 일부 프로젝트에서 빠짐.

### ETH Zurich 연구 적대 검증 결과 (확인됨)

- **LLM 생성 컨텍스트 파일:** 성공률 ~3% 감소, 비용 20% 이상 증가
- **인간 작성 컨텍스트 파일:** ~4% 미미한 개선
- **권장:** 포함 = 기술 스택·의도·비표준 도구 / 제외 = 디렉터리 트리·스타일 가이드·작업별 지시 / 길이 = 300줄 이하(전문 팀 60줄 이하)
- **주의:** "에이전트는 지시를 잘 따르지만 불필요한 요구사항은 문제를 어렵게 만든다"

**출처:**
- https://www.mindstudio.ai/blog/context-rot-ai-coding-agents-explained
- https://www.augmentcode.com/blog/your-agents-context-is-a-junk-drawer
- https://www.marktechpost.com/2026/02/25/new-eth-zurich-study-proves-your-ai-coding-agents-are-failing-because-your-agents-md-files-are-too-detailed/
- https://arxiv.org/abs/2602.14690  
**확신도:** 확실(함정 존재), 가능성 높음(수치 — LLM 생성 파일 대상)

---

## 패턴 5: Cursor Rules 경험적 분류 (arxiv 2512.18925)

401개 오픈소스 저장소 분석 결과, 개발자가 프로젝트 컨텍스트에 넣는 정보의 5가지 상위 분류:

1. **Conventions** — 코딩 규칙·명명·포맷
2. **Guidelines** — 작업 방법·프로세스
3. **Project Information** — 아키텍처·기술 스택·구조
4. **LLM Directives** — 에이전트 동작 제어
5. **Examples** — 코드 패턴·참고 사례

→ 이 분류 전체가 **프로젝트 레이어**에 해당. 글로벌에는 도구 독립적 개인 선호만.

**출처:** https://arxiv.org/abs/2512.18925  
**확신도:** 확실 (논문 직접 확인)

---

## 종합 결론 — 경계 기준 원칙 (3문장)

1. **글로벌 = "나는 이렇게 일한다"**: 사용자 선호·스타일·항상 유효한 원칙. 도구나 프로젝트가 바뀌어도 변하지 않는 것.
2. **프로젝트 = "이 저장소는 이렇게 동작한다"**: 버전 관리되어 팀 전체 공유. 빌드 명령·아키텍처·관례. 새 팀원 온보딩에 필요한 것.
3. **핸드오프 = "지금 이 작업의 상태"**: 어느 레이어에도 넣지 않는다. 세션 단위로 생성·소비·폐기. 여기 있는 정보를 프로젝트 파일에 섞으면 컨텍스트 rot 가속.

**파생 규칙:** 경로별 rules(path-scoped)는 프로젝트 레이어의 세분화 — 파일 타입·서브시스템별 조건부 로드로 토큰 절약. 글로벌+프로젝트 합산이 200~300줄을 넘기 시작하면 path-scoped로 분할하거나 내용을 제거한다.

---

## 공백 및 한계

- Codex 독립 조사에서 ETH Zurich 수치를 언급하지 않음 → Claude 쪽 출처에서만 확인 (단, 별도 검색으로 직접 확인 완료)
- 인간 작성 컨텍스트 파일의 ~4% 개선 수치는 샘플 특성(오픈소스 저장소)에 의존 — 엔터프라이즈 내부 프로젝트에서 다를 수 있음
- 도구별 글로벌 규칙 파일 위치 차이로 실제 로드 순서는 도구마다 다름 (모든 도구에 공통되는 단일 표준 없음)
