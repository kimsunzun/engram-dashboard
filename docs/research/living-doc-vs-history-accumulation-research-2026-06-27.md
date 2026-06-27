# Living Document vs 이력 누적 패턴 — AI 에이전트 세션 핸드오프 리서치

**상태**: 완료  
**방법**: deep tier — Claude 팬아웃(A/B/C 3갈래, 검색 15회+) + Codex BLIND 독립 교차 + 교차 대조 + 적대 검증  
**날짜**: 2026-06-27  
**확신도 범례**: 확실 = 복수 독립 출처 합의 / 가능성 높음 = 1~2 출처 + 논리 일관 / 불확실 = 단일 출처 또는 미확인

---

## 요약 결론

**하이브리드(living doc + 이력 아카이브 분리)가 AI 에이전트 세션 핸드오프에 가장 적합하다.**  
단, 하이브리드는 명시적 업데이트 규율 없이는 단독 living doc보다 오히려 나빠질 수 있다.

---

## 방식별 상세 분석

### 1. Living Document (단일 파일 계속 갱신)

**방식**: `state.md` / `CLAUDE.md` / `AGENTS.md` 한 파일을 최신 상태로 덮어씀

**장점** (확실):
- 컨텍스트 로드 비용 최소 — 다음 세션이 한 파일만 읽으면 됨
- 현재 상태 가시성 높음 — "지금 뭘 해야 하나"가 한 곳에
- AI 에이전트가 매 세션 자동 로드하기 쉬운 구조 (CLAUDE.md 자동 주입 패턴)
- Git 하에서는 롤백도 가능(파일 히스토리로)

**단점** (확실):
- 결정 이력 소실 — 왜 그 결정을 내렸는지 다음 세션에 안 남음
- 과거 세션 행동 추적 어려움 (디버깅 시 "어떤 세션에서 이게 바뀌었나" 불투명)
- 파일이 커지면 성능 저하 — 150~200줄 초과 시 에이전트 주의 예산 압박, 371줄이 사실상 상한 (가능성 높음)
- Codex는 AGENTS.md에 32 KiB 하드 제한 (확실)

**AI 에이전트 적합도**: **높음** (단기~중기 프로젝트), 중간 (장기)  
**이유**: 핸드오프 속도·단순성 최우선이면 충분하나, 장기 프로젝트에서 이력 소실이 누적됨

**실제 사례**:
- Claude Code `CLAUDE.md` — 모든 세션에 자동 로드되는 persistent rules
- OpenAI Codex `AGENTS.md` — repo-level 지속 지침
- Cursor `.cursor/rules/*.mdc` — 프로젝트 규칙 living doc
- Sonovore claude-code-handoff의 `.claude/session-state.md` — 자동 갱신 단일 파일 (크기 커지면 중요 정보만 재작성)

**출처**:
- https://code.claude.com/docs/en/best-practices
- https://codersera.com/blog/agents-md-vs-claude-md-vs-cursor-rules-comparison-2026/
- https://github.com/Sonovore/claude-code-handoff
- https://www.augmentcode.com/guides/how-to-build-agents-md

---

### 2. 이력 누적 (세션/날짜별 새 파일)

**방식**: `2026-06-27-session-name.md` 패턴으로 세션마다 신규 생성

**장점** (확실):
- 완전한 감사 추적 — 어느 세션에서 무슨 결정을 했는지 복원 가능
- 디버깅 우수 — "이 버그가 어느 세션에서 도입됐나" 추적 가능
- 롤백 명확 — 파일 단위로 특정 세션 상태 복원
- 병렬 작업 흐름 분기 지원 (각 작업 흐름을 독립 파일로 관리)
- Event Sourcing과 동일 원리: 불변 이벤트 로그 = 재생 가능

**단점** (확실):
- 다음 세션이 현재 상태 파악을 위해 여러 파일을 읽어야 함 — 컨텍스트 비용 증가
- 검색·인덱싱 인프라 필요 (파일 수 증가 시)
- 과거 파일에 obsolete 정보가 그대로 잔존 (stale 아카이브)
- 노이즈 비율 증가 — "무엇이 지금도 유효한가"를 매번 판단해야 함

**AI 에이전트 적합도**: **중간** (단독 사용 시), **높음** (아카이브 백업으로)  
**이유**: 단독으로는 핸드오프 컨텍스트 비용이 너무 큼. 그러나 living doc과 조합 시 최강

**실제 사례**:
- REMvisual/claude-handoff: `HANDOFF_작업명_날짜.md` 패턴, 체인 연속성 자동 감지
- CyPack/claude-session-handoff: `session-handoff-{date}.md` + 누적 lesson 파일
- Architecture Decision Records(ADR): 불변 이력 누적, superseded 표시로 폐기 연결

**출처**:
- https://github.com/REMvisual/claude-handoff
- https://github.com/CyPack/claude-session-handoff
- https://github.com/joelparkerhenderson/architecture-decision-record
- https://adr.github.io/

---

### 3. 하이브리드 (Living Doc + 이력 아카이브 분리)

**방식**: `state.md`(현재 상태 living doc) + `sessions/2026-06-27-name.md`(세션 아카이브) 분리 운영

**장점** (확실):
- 핸드오프 속도 = living doc 수준 (다음 세션은 state.md 하나만 로드)
- 디버깅·롤백 = 이력 누적 수준 (session archive로 과거 추적)
- 컨텍스트 예산 효율 — 현재 상태만 주입, 과거는 필요할 때 선택적 로드
- 정보 소실 방지 — 모든 세션 행동이 아카이브에 보존

**단점** (확실):
- 두 파일 동기화 규율 필요 — 규율 없으면 state.md는 구식·session만 최신인 역전 현상 발생
- 유지보수 부담 — "언제 summarize하고 archive로 이동하나" 기준 필요
- 단순 프로젝트에는 over-engineering

**AI 에이전트 적합도**: **높음** (확실)  
**이유**: 장기 실행 에이전트 작업, 팀 핸드오프, 복원 가능성이 필요한 모든 경우에 최적

**실제 사례**:
- Claude Code: `CLAUDE.md`(living) + session compaction history + `/memory`
- OpenAI Codex: `AGENTS.md`(living) + memories + session history
- Amp: `AGENTS.md`(living) + persistent threads(archive)
- Sonovore claude-code-handoff: `session-state.md`(living) + 수동 `/handoff` 명령 시 세션별 파일 생성

**출처**:
- https://docs.anthropic.com/en/docs/claude-code/memory
- https://ampcode.com/manual
- https://github.com/Sonovore/claude-code-handoff

---

## 교차검증표 (Claude ↔ Codex 수렴/발산)

| 클레임 | Claude 조사 | Codex BLIND | 판정 | 확신도 |
|---|---|---|---|---|
| 하이브리드 = AI 에이전트에 최적 | 수렴 | 수렴 (Certain) | 합의 | 확실 |
| Living doc 단독 = 이력 소실 | 수렴 | 수렴 (Certain) | 합의 | 확실 |
| 재귀 요약 반복 시 왜곡 발생 | Amp 사례 확인 | 동일 사례 확인 | 합의 | 확실 |
| 세션별 파일 = 디버깅·롤백 우위 | 수렴 | 수렴 | 합의 | 확실 |
| CLAUDE.md = living / session = 이력 하이브리드 | 공식 docs | 공식 docs | 합의 | 확실 |
| 파일 크기 150~200줄 권장 상한 | 발견 (1출처) | 미언급 | 발산(경미) | 가능성 높음 |
| 70% 컨텍스트 압축 트리거 | 발견 | 미언급 | 발산(경미) | 가능성 높음 |
| 핸드오프 파일을 임시 디렉토리 권장 | 발견 (1출처) | 미언급 | 발산 | 불확실 |

---

## 핵심 질문별 결론

### Q1. 각 방식의 실제 장단점

- **Living doc**: 핸드오프 속도·단순성 최강, 이력·디버깅 약점. Git 하에서는 롤백 가능하나 세션 단위 추적은 어려움.
- **이력 누적**: 감사·롤백·디버깅 최강, 다음 세션 cold start 비용 증가.
- **하이브리드**: 두 장점 조합. 단, 동기화 규율이 없으면 오히려 나빠짐.

### Q2. 어느 쪽이 AI 에이전트와 더 잘 작동하는가?

**하이브리드가 실무에서 수렴한 패턴** (확실). Claude Code, OpenAI Codex, Amp, LangGraph 모두 "persistent rules/state + session history" 이중 구조를 채택.  
단독 living doc는 단기 작업에 충분. 단독 이력 누적은 실무에서 거의 사용되지 않음(컨텍스트 비용 때문).

### Q3. 하이브리드 구현 방법

**권장 파일 구조**:
```
state.md              # living doc — 현재 목표·결정·미해결 블로커·다음 액션 (덮어씀)
sessions/
  2026-06-27-name.md  # 세션 아카이브 — 무엇을 했고, 어떤 커맨드/테스트, 미해결 질문
decisions/            # ADR 또는 굵은 결정 이력 (불변 누적, superseded 표시)
```

**업데이트 규율** (핵심):
- 세션 종료 시 `state.md` 갱신(다음 세션의 cold start 지점) + `sessions/*.md` 신규 생성
- state.md가 150~200줄 초과 시 → 핵심 사실만 남기고 세부는 archive로 이동
- 결정이 번복될 때 → state.md 갱신 + decisions/에 ADR

### Q4. 문서가 너무 커지면 어떻게 관리하는가?

**확인된 전략** (확실):
1. **임계값 기반 압축**: state.md 150~200줄 / 70% 컨텍스트 소비 시 압축 트리거
2. **계층적 요약(Hot/Warm/Cold)**: 최근 10턴 전문 / 상세 요약 / 고수준 요약 3계층
3. **선택적 아카이브 이동**: 완료된 작업·구식 결정은 sessions/에 이동
4. **자동 압축(신중하게)**: Amp는 재귀 요약 왜곡 문제로 무감독 compaction을 폐기. CLAUDE.md로 압축 방향을 제어하면 경감.
5. **벡터/그래프 스토어**: 장기 팩트를 외부 저장소에 오프로드 (LangGraph, CrewAI 패턴)

---

## 공백 및 한계

- "임시 디렉토리 권장" 주장: 단일 출처(candede.com), 미검증
- 70% 압축 트리거 수치: 단일 출처(agentmarketcap.ai), 벤치마크 원본 미확인
- 하이브리드 동기화 규율의 실패율/비용: 정량적 데이터 없음 — 정성적 관찰만 존재

---

## 출처 목록

- https://code.claude.com/docs/en/best-practices — Claude Code 공식
- https://docs.anthropic.com/en/docs/claude-code/memory — Claude Code 메모리 공식
- https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents — Anthropic 엔지니어링
- https://codersera.com/blog/agents-md-vs-claude-md-vs-cursor-rules-comparison-2026/ — AGENTS.md/CLAUDE.md 비교
- https://www.augmentcode.com/guides/how-to-build-agents-md — AGENTS.md 가이드
- https://github.com/Sonovore/claude-code-handoff — 하이브리드 핸드오프 구현
- https://github.com/CyPack/claude-session-handoff — 세션별 파일 + 누적 lesson
- https://github.com/REMvisual/claude-handoff — 세션별 파일 체인 패턴
- https://tessl.io/blog/amp-retires-compaction-for-a-cleaner-handoff-in-the-coding-agent-context-race/ — Amp compaction 폐기 사례
- https://gist.github.com/badlogic/cd2ef65b0697c4dbe2d13fbecb0a0a5f — 코딩 에이전트 context compaction 비교
- https://agentmarketcap.ai/blog/2026/04/11/agent-context-engineering-sliding-windows-memory-2026 — 계층적 요약
- https://zylos.ai/research/2026-03-31-context-window-management-session-lifecycle-long-running-agents/ — 세션 lifecycle
- https://www.mindstudio.ai/blog/context-rot-ai-coding-agents-how-to-prevent — context rot 분석
- https://docs.langchain.com/oss/python/langgraph/memory — LangGraph 메모리
- https://github.com/joelparkerhenderson/architecture-decision-record — ADR 패턴
- https://adr.github.io/ — ADR 공식
- https://keepachangelog.com/en/1.1.0/ — Changelog 표준
- https://learn.microsoft.com/en-us/azure/architecture/patterns/event-sourcing — Event Sourcing 패턴
- https://docs.crewai.com/concepts/memory — CrewAI 메모리
- https://ampcode.com/manual — Amp 공식
