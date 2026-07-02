# Study Notes — Agentic Messaging Patterns 리서치 (2026-06-28)

주제: LLM 에이전트 메시징·오케스트레이션 패턴 서베이 | 강도: deep

## 어떤 검색을 했나

1차 병렬 WebSearch 4개: A2A 스펙, LangGraph, AutoGen, agentic vs traditional messaging
Codex blind 교차: 전 갈래 독립 조사 (동시 스폰)
2차 WebFetch 3건: A2A 스펙 직접 (a2a-protocol.org), Agents SDK handoff 문서, arxiv 논문
2차 WebSearch 4개: Agents SDK handoff, CrewAI, supervisor/orchestrator 패턴, Microsoft Agent Framework
3차 WebFetch + WebSearch: A2A 스펙 상세, LangGraph Command, Rust 구현 사례

## 쟁점 해소 과정

### 쟁점 1: A2A 버전 혼재 (v0.3 vs v1.0)
- Google Cloud 블로그는 "v0.3, gRPC 추가" 표기
- a2a-protocol.org/latest/specification/은 "1.0.0" 표기
- 결론: 두 버전 모두 실재. Google이 v0.3 기준으로 발표했고 Linux Foundation 이관 후 1.0.0 스펙이 공식화된 것으로 추정. 확신도: 가능성 높음

### 쟁점 2: AutoGen 장기지원 여부
- AutoGen README에 migration to Agent Framework 권고
- Microsoft learn에 공식 마이그레이션 가이드 존재
- 결론: 유지보수 모드 맞음. 확신도: 확실

### 쟁점 3: 무감독 peer 토론 안티패턴 정량 근거
- beam.ai, gurusup 블로그 등 복수 출처에서 언급
- 학술 논문 직접 인용 없음
- 결론: 가능성 높음으로 표기, 정량 수치는 공백으로 표시

## Deep tier가 Medium과 어떻게 달랐나

- WebFetch 3건 추가: A2A 스펙 공식 확인, arxiv 논문 직접 읽기
- 적대 검증 3클레임: "A2A 내부 IPC 부적합", "AutoGen 유지보수 모드", "peer 토론 안티패턴"
- A2A 버전 불일치를 발산 처리하지 않고 WebFetch로 직접 해소
- Rust 생태계 검색 추가 (ai-agents crate, ADK-Rust)
- 총 검색·Fetch: 약 20회

## 확신도 분포 결과

- 확실: A2A 라이선스/스펙, LangGraph, AutoGen 상태, Swarm 비프로덕션, Agents SDK handoff, CrewAI 라이선스
- 가능성 높음: A2A 성숙도/버전, Agent Framework 성숙도, 안티패턴 근거
- 불확실: CrewAI Consensus 상세, Agent Framework production 사례, Rust ADK-Rust 성숙도

## engram 설계 함의 (이 리서치가 ADR 후보가 될 부분)

- ADR-0014(오케스트레이션 후보)에 추가할 내용:
  - A2A: 외부 어댑터 계층용 (내부 IPC X)
  - LangGraph 패턴: 내부 supervisor/Command 이식 권장
  - Agents SDK handoff 패턴: typed input_type + input_filter 이식 권장
  - AutoGen/Agent Framework: 참고용, 의존성 X
  - CrewAI: 참고용, 결 다름
