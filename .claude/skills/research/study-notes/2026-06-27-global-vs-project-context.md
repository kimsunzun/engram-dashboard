# Study Note: global-vs-project-context (2026-06-27) — deep tier

## 쟁점과 결론 도달 과정

### 쟁점 1: "글로벌에 뭘 넣나"의 경계가 애매함
- 검색 1차에서 "개인 선호" vs "조직 기준" 두 갈래가 나옴
- Claude Code 공식 문서에서 4레이어(managed policy / user / project / local)로 명확히 분리됨 → managed policy = 조직, user = 개인 선호
- 결론: "글로벌"이라는 단어가 사실 두 개의 다른 레이어를 덮고 있었음. 단일 레이어로 취급하면 안 됨.

### 쟁점 2: ETH Zurich "비용 20% 증가" 수치의 신뢰도
- Augment Code 블로그에서 이 수치 인용 → 단일 출처로 불안
- 별도 검색에서 MarkTechPost와 복수 미디어가 동일 수치 재인용 확인
- 원 논문(ETH Zurich, arxiv)을 직접 접근해 확인하지 못함 → "가능성 높음"으로 표기
- 중요: 이 수치는 **LLM 생성 컨텍스트 파일** 대상. 인간 작성은 ~4% 개선. 구분 중요.

### 쟁점 3: 핸드오프 문서 = 글로벌? 프로젝트?
- 초기 검색에서 "재사용 가능한 절차는 프로젝트에" 식으로 애매하게 나옴
- Codex 독립 조사가 "핸드오프는 별도 작업 단위 레이어"라고 명시 → 중요한 추가 명확화
- Claude 갈래에서는 암시만 되어 있었음. Codex가 더 명확히 정리.
- 결론: 세션 핸드오프는 글로벌도 프로젝트도 아님. 섞으면 컨텍스트 rot 가속.

### 쟁점 4: path-scoped rules의 위치
- 처음엔 Claude Code 한정 기능인 줄 알았음
- 조사 결과 Cursor, Copilot(path-specific instructions), Windsurf도 유사 메커니즘 보유
- 도구마다 구현이 다르지만 원칙은 동일: 조건부 로드로 토큰 절약

## deep tier가 medium 대비 무엇을 더 했나
- ETH Zurich 논문 적대 검증 (반증 시도 → 수치 확인)
- WebFetch로 Claude Code 공식 문서 전체 구조 직접 확인 (검색 요약만 아님)
- Codex 독립 교차에서 핸드오프 레이어 구분이 추가로 명확화됨 → medium이었으면 이 포인트 묻혔을 것
- 총 검색 약 13회 + WebFetch 6회

## 스킬 명세 개선 메모 (feedback.md로 이동 예정)
- deep tier에서 Codex가 Claude보다 일부 항목을 더 명확히 정리한 경우(핸드오프 레이어)가 발생 → cross-family 교차의 실제 효과 관찰됨
