# Study Note — Living Doc vs 이력 누적 (2026-06-27)

## 쟁점과 결론 도달 과정

### 쟁점 1: "하이브리드가 최선" 반증 시도
- **반증**: 하이브리드는 동기화 규율 없으면 단독 living doc보다 나쁨 (state.md 구식·session만 최신)
- **결론**: 반증 부분 성립. 하이브리드 우위는 조건부 — 명시적 업데이트 규율 있을 때만 성립
- **시사점**: 보고서에 "규율 없이는 오히려 나빠질 수 있음" 명시 필요

### 쟁점 2: "재귀 요약 왜곡"
- **근거**: Amp 수석 엔지니어 + OpenAI 내부 보고 (tessl.io 출처)
- **반증**: Anthropic은 CLAUDE.md로 압축 방향 제어 가능 → 무감독 compaction에 한정되는 문제
- **결론**: 반증 성립 일부. "무감독 자동 compaction"에 한정 조건 붙임

### 쟁점 3: "단독 living doc = 롤백 불가"
- **반증**: Git 하에서는 파일 히스토리로 롤백 가능
- **결론**: 롤백 자체는 가능하나 세션 단위 행동 추적은 여전히 약함 → 표현 수정

## deep tier에서 뭐가 달라졌나

- 단순 medium이었다면 Amp compaction 폐기 사례 못 발견했을 것 (tessl.io WebFetch가 핵심)
- Codex BLIND가 Claude와 독립적으로 동일 패턴(하이브리드)에 수렴 → "만장일치 ≠ 정답" 경계를 확인하면서도 신뢰도 상승
- 구체적 수치(150줄/70% 트리거)는 Codex가 미언급 → 단일 출처 경고 붙임

## 검색 전략 관찰

- "living document vs changelog" 단독 검색은 개발자 changelog 포캐스트만 나옴 — 너무 일반적
- "Amp compaction abandoned" 구체적 도구명 + 사건으로 검색하자 핵심 사례 발견
- WebFetch가 WebSearch보다 구체적 트레이드오프 정보 품질 높음 (검색 snippet은 표면적)
- Codex BLIND가 공식 docs(anthropic, openai, langgraph, crewai)를 체계적으로 커버 → Claude는 커뮤니티 구현 사례 커버 — 상호 보완

## 명세 개선 메모

→ feedback.md로 이동 예정
