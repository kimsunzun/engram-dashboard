# Study Note: AI 코딩 에이전트 세션 컨텍스트 관리 (deep tier)

**날짜:** 2026-06-27  
**강도:** deep  
**주제:** Cursor/Aider/Continue.dev/OpenHands/Devin/Copilot/Claude Code/Cline/Roo-Code 세션 컨텍스트 관리

## 쟁점과 해소 과정

### 쟁점 1: Copilot SQLite vs 파일 전용

- Claude 조사: `~/.copilot/session-state/` 파일 기반이라고 먼저 발견
- Codex 조사: SQLite `session-store.db` 추가 언급
- 해소: GitHub 이슈 #3046 검색으로 실제 `~/.copilot/session-store.db` 존재 확인. Codex 추가 발견이 맞음 — 두 family 교차가 보완 역할을 한 실제 사례.

### 쟁점 2: Cursor Memories 저장 방식

- Claude: 프로젝트별 로컬, Settings에서 관리
- Codex: "제품 관리형, 파일 직접 노출 없음"
- 해소 시도: forum.cursor.com 검색 → 파일 경로 미공개 확인. 두 family 모두 공통 공백 — "만장일치 ≠ 정답" 경계 확인.

## deep tier vs medium 차이 체감

- WebFetch로 공식 문서 전문 직접 확인(Claude Code memory 페이지 전체 추출) — medium은 검색 결과 요약에 그침
- Codex BLIND 교차가 SQLite 발견으로 실제 추가 정보 기여 → 교차 효과 확인
- 적대 검증(2 쟁점에 재검색) 비용: 검색 2회 + WebFetch 2회 추가

## 검색 전략 관찰

- `site:` 한정자 활용이 SEO 콘텐츠팜 배제에 효과적 (`site:docs.anthropic.com`, `site:aider.chat` 등)
- WebFetch 리다이렉트 처리(301/308) 주의 필요 — 2회 fetch 요구됨
- 아카이브된 리포(Roo-Code)는 검색 결과에서 archived 언급으로 판별 가능
