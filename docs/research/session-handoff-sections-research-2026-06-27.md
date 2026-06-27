# 리서치 보고서 — AI 코딩 에이전트 세션 핸드오프 문서 필수 섹션

**상태:** 완료
**방법:** Claude 팬아웃 3갈래(SW 핸드오프 / AI 에이전트 컨텍스트 트랜스퍼 / 타 도메인 공통 구조) + Codex BLIND 교차 + 핵심 클레임 2개 적대 검증
**날짜:** 2026-06-27
**확신도 범례:** ●확실 / ◑가능성 높음 / ○불확실

---

## 교차검증표 (Claude ↔ Codex 수렴/발산)

| 클레임 | Claude 수집 | Codex | 판정 |
|---|---|---|---|
| Goal/Status — 필수 | ✓ | ✓ | 수렴 |
| Background/Context — 필수 | ✓ | ✓ | 수렴 |
| Decisions + Rationale — 필수 | ✓ | ✓ | 수렴 |
| Next Steps — 필수 | ✓ | ✓ | 수렴 |
| Files Changed — 필수 | ✓ | ✓ | 수렴 |
| Risks/Blockers — 필수 | ✓ | ✓ | 수렴 |
| Environment/Commands — 필수 | ✓ | ✓ | 수렴 |
| Verification Gaps — AI 특화 필수 | ✓ | ✓ | 수렴 (적대검증 통과) |
| Context-window notes — AI 특화 | ✓ | ✓ | 수렴 (적대검증 통과) |
| Session ID/Resume — AI 특화 | ✓ | ✓ | 수렴 |
| Persistent memory locations — AI 특화 | ✓ | ✓ | 수렴 |
| Owner/Responsibility | 묵시적 | 명시 | 발산 → 프로젝트 필수/글로벌 선택 |
| Escalation path | NOC/군사만 | 명시 | 발산 → 선택 |
| Post-incident retrospective | 없음 | ◑ | 발산 → 선택 |

---

## 섹션 카탈로그

### Goal / Mission
목적: 이 세션이 달성하려는 최종 목표. 수신자(에이전트)가 방향을 잃지 않기 위한 닻.
필수/선택: 필수
글로벌/프로젝트: 프로젝트(세션별)
출처: https://www.jdhodges.com/blog/ai-session-handoffs-keep-context-across-conversations/ · https://en.wikipedia.org/wiki/Operations_order
확신도: ●확실

### Current Status
목적: 완료된 것/진행 중인 것/남은 것. 상태는 완료/진행중/차단/롤백/조사중 중 하나로 명시.
필수/선택: 필수
글로벌/프로젝트: 프로젝트(세션별)
출처: https://hermes-agent.ai/blog/ai-agent-session-handoff-checklist · https://www.jdhodges.com/blog/ai-session-handoffs-keep-context-across-conversations/
확신도: ●확실

### Background / Context
목적: 왜 이 작업이 존재하는지, 관련 아키텍처·컨벤션·선행 제약. 수신자가 현재 상태를 해석할 수 있게 함.
필수/선택: 필수
글로벌/프로젝트: 글로벌(불변 아키텍처) + 프로젝트(세션 맥락)로 분리 가능
출처: https://en.wikipedia.org/wiki/SBAR · https://arxiv.org/abs/2606.23752 · https://agents.md/
확신도: ●확실

### Decisions Made + Rationale
목적: 내린 결정과 그 이유. 없으면 다음 에이전트가 같은 대안을 재검토하거나 결정을 번복. ADR 형태로 연결 가능.
필수/선택: 필수
글로벌/프로젝트: 프로젝트(세션별, 글로벌 ADR 포인터)
출처: https://arxiv.org/abs/2606.23752 · https://www.jdhodges.com/blog/ai-session-handoffs-keep-context-across-conversations/
확신도: ●확실

### What to Avoid (Traps / Failed Approaches)
목적: 시도했으나 실패한 방향. 다음 세션이 같은 함정에 빠지는 것 방지. AI는 동일 환각을 반복하는 경향.
필수/선택: 필수 (AI 에이전트 특화)
글로벌/프로젝트: 프로젝트(세션별)
출처: https://gist.github.com/BexTuychiev/95a92f1234772dfb60f9b7470673d82f · https://www.jdhodges.com/blog/ai-session-handoffs-keep-context-across-conversations/
확신도: ●확실

### Next Steps / Remaining Work
목적: 수신자가 즉시 취할 첫 번째 행동. 번호 매긴 순서·우선순위·의존성 포함.
필수/선택: 필수
글로벌/프로젝트: 프로젝트(세션별)
출처: https://hermes-agent.ai/blog/ai-agent-session-handoff-checklist · https://en.wikipedia.org/wiki/Clinical_handover
확신도: ●확실

### Files / Artifacts Changed
목적: 편집된 파일 경로, 생성된 산출물, 브랜치/PR. 소스와 생성물 분리 필요.
필수/선택: 필수
글로벌/프로젝트: 프로젝트(세션별)
출처: https://hermes-agent.ai/blog/ai-agent-session-handoff-checklist · https://arxiv.org/abs/2606.23752
확신도: ●확실

### Commands Run + Results
목적: 실행한 명령어와 실제 결과(성공·실패 포함). 에이전트는 명령 실행으로 재개하므로 산문보다 정확한 명령이 필요.
필수/선택: 필수 (AI 에이전트 특화)
글로벌/프로젝트: 프로젝트(세션별)
출처: https://hermes-agent.ai/blog/ai-agent-session-handoff-checklist · https://agents.md/
확신도: ●확실

### Verification Gaps
목적: 아직 검증되지 않은 항목 명시. "모든 것이 작동할 것" 같은 환각 표현 금지.
필수/선택: 필수 (AI 에이전트 특화 — 인간 핸드오프에도 있으나 AI는 명시 없으면 환각으로 채움)
글로벌/프로젝트: 프로젝트(세션별)
출처: https://hermes-agent.ai/blog/ai-agent-session-handoff-checklist
확신도: ●확실 (적대검증 통과)

### Assumptions + Risks / Blockers
목적: 전제한 가정, 잘못될 수 있는 것, 현재 차단 요인. 비용·속도 제한·권한·3rd-party API 포함.
필수/선택: 필수
글로벌/프로젝트: 프로젝트(세션별)
출처: https://hermes-agent.ai/blog/ai-agent-session-handoff-checklist · https://en.wikipedia.org/wiki/Clinical_handover · https://arxiv.org/abs/2601.07788
확신도: ●확실

### Environment / Build / Test Commands
목적: 재현 가능한 환경 설정, 빌드·테스트·실행 명령 정확 기재. 에이전트는 도구 실행으로 재개하므로 핵심.
필수/선택: 필수
글로벌/프로젝트: 글로벌(템플릿) + 프로젝트(실제 명령)
출처: https://agents.md/ · https://gist.github.com/TimothyJones/1508a7081405d57073b99180312f5caa · https://www.simplethread.com/handing-off-a-software-project/
확신도: ●확실

### Open Questions / Unknowns
목적: 미결 질문, 불확실한 가정. "사실"이 아닌 추정임을 명시하여 수신자가 오신뢰하지 않게 함.
필수/선택: 필수 (AI는 불확실을 확신으로 채우는 경향 때문에 명시 필요)
글로벌/프로젝트: 프로젝트(세션별)
출처: https://arxiv.org/abs/2606.23752 · https://en.wikipedia.org/wiki/SBAR
확신도: ◑가능성 높음

### Source of Truth Pointers
목적: 신뢰할 파일·URL·대시보드·API 목록. 에이전트가 오래된 채팅 텍스트를 사실로 오인하지 않도록.
필수/선택: 필수 (AI 에이전트 특화)
글로벌/프로젝트: 글로벌(CLAUDE.md 위치 등) + 프로젝트(세션 관련 파일)
출처: https://hermes-agent.ai/blog/ai-agent-session-handoff-checklist · https://www.candede.com/articles/mastering-ai-context-windows-handoff-skill/
확신도: ●확실

### Context-Window Survival Notes
목적: 컨텍스트 압축/세션 재시작 후 무엇이 사라지는지, 무엇이 자동 리로드되는지. 인간 핸드오프에 없는 AI 전용 섹션.
필수/선택: 필수 (AI 에이전트 전용)
글로벌/프로젝트: 글로벌(메모리 구조) + 프로젝트(세션별 압축 상태)
출처: https://code.claude.com/docs/en/context-window · https://zylos.ai/research/2026-03-31-context-window-management-session-lifecycle-long-running-agents/
확신도: ●확실 (적대검증 통과)

### Persistent Memory Locations
목적: 영속 지시/메모리가 어디 있는지(CLAUDE.md, .claude/rules/, MEMORY.md 등). 에이전트 시작 시 자동 로드 경로 포함.
필수/선택: 필수 (AI 에이전트 전용)
글로벌/프로젝트: 글로벌
출처: https://code.claude.com/docs/en/memory · https://arxiv.org/abs/2606.23752
확신도: ●확실

### Session Identity / Resume Path
목적: 세션 이름/ID, 브랜치/워크트리, 프로젝트 경로, 트랜스크립트 위치. 에이전트가 정확히 어디서 재개하는지.
필수/선택: 필수 (AI 에이전트 특화)
글로벌/프로젝트: 프로젝트(세션별)
출처: https://code.claude.com/docs/en/sessions
확신도: ●확실

### Constraints / Permissions / Boundaries
목적: 허용/금지 행동, 샌드박스 제한, 보안 규칙, 도구 제약. 에이전트가 범위 밖 행동을 하지 않도록.
필수/선택: 필수 (AI 에이전트 특화)
글로벌/프로젝트: 글로벌(조직 공통) + 프로젝트(세션 한정)
출처: https://code.claude.com/docs/en/memory · https://agents.md/
확신도: ●확실

### Owner / Responsibility
목적: 남은 행동·결정의 소유자(에이전트/사용자/리뷰어/CI). 다중 에이전트 오케스트레이션에서 중요.
필수/선택: 프로젝트 핸드오프 필수 / 단일 에이전트 세션 선택
글로벌/프로젝트: 프로젝트
출처: https://en.wikipedia.org/wiki/Clinical_handover · https://en.wikipedia.org/wiki/Incident_Command_System
확신도: ◑가능성 높음

### Working Agreements / Response Style
목적: 사용자의 상호작용 선호도, 응답 형식 가이드라인. AI 에이전트가 사용자 스타일을 세션 간 유지.
필수/선택: 선택
글로벌/프로젝트: 글로벌
출처: https://gist.github.com/BexTuychiev/95a92f1234772dfb60f9b7470673d82f
확신도: ◑가능성 높음

### Escalation / Communication Path
목적: 승인이나 판단이 필요할 때 어디로 에스컬레이션할지.
필수/선택: 선택 (NOC·군사·의료 도메인 필수, AI 단일 세션 낮음)
글로벌/프로젝트: 글로벌
출처: https://en.wikipedia.org/wiki/Five_paragraph_order · https://arxiv.org/abs/2601.07788
확신도: ◑가능성 높음

### Post-incident / Lessons Learned
목적: 세션 회고, 반복 실수 방지.
필수/선택: 선택
글로벌/프로젝트: 프로젝트
출처: https://arxiv.org/abs/2601.07788
확신도: ◑가능성 높음

---

## 일반 SW 핸드오프 vs AI 에이전트 핸드오프 — 차이점

| 항목 | 일반 SW 핸드오프 | AI 에이전트 핸드오프 |
|---|---|---|
| 독자 | 인간 개발자 | 모델 (산문 파싱, 명령 실행) |
| 환경 명령 | 있으면 좋음 | 필수 — 에이전트가 명령으로 재개 |
| Verification Gaps | 묵시적 | 명시 필수 — 모델 환각 방지 |
| Context-window notes | 없음 | 필수 — 인간에게는 없는 개념 |
| Session ID/Resume | 없음 | 필수 |
| Memory locations | 없음 | 필수 |
| Constraints/Permissions | 암묵적 이해 | 명시 필수 — 모델은 범위를 추정 못함 |
| Traps/Failed approaches | 권장 | 강력 필수 — 모델은 동일 환각 반복 |
| Source of truth pointers | 권장 | 필수 — 채팅 텍스트를 사실로 오인 방지 |

---

## 공백 및 한계

- Claude Code 공식 `session-handoff` 스킬 원문 접근 실패(agentskills.me 502). 스킬 내부 구조는 간접 추론.
- arXiv:2606.23752(ESAA)는 2026년 논문이며 정식 peer review 완료 여부 불확실 — 구조 패턴 참고 수준.
- "글로벌 vs 프로젝트 분리" 경계는 구현마다 다름 — 이 보고서는 개념 구분이며 강제 기준 아님.
