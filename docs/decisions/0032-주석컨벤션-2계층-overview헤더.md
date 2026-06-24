# ADR-0032: 주석 컨벤션 — 2계층(인라인 좁히기 + load-bearing overview 헤더) + ADR 앵커 점진 확대

- 상태: 확정 (2026-06-24, 근거: `docs/research/code-commenting-conventions-research-2026-06-23.md` + opus/Codex 적대 리뷰)
- 관련: CLAUDE.md `## 컨벤션` · `docs/reference/commenting-conventions.md`(살아있는 캐논) · `docs/research/code-commenting-conventions-research-2026-06-23.md`(근거)

## 맥락
에이전트 방식 개발로 사용자가 코드 전체를 직접 읽지 않게 되니 "주석이 더 자세해야 그 부분만 보고 한눈에 이해된다"는 사용자 가설이 나왔다. 이게 맞는지, 업계 주석 관행이 에이전트 시대에 어떻게 바뀌는지 리서치했다(컨설/PRD 단계 — 조사 → 선택지 → 사용자 결정). 결정 전까지 engram의 주석 원칙은 CLAUDE.md `## 컨벤션` 절에만 있었고, 주석 전용 정설 문서나 ADR은 없었다.

## 결정
**선택지 B 채택.** 2계층 주석 규약을 명문화한다:
- **(1) 인라인 = why/intent/invariant/load-bearing으로 좁힌다**(기존 engram 규약 유지).
- **(2) load-bearing 파일은 overview 헤더(`//!`) 권장(soft)** — 역할·책임·핵심 불변식·진입점·"시그니처로 안 보이는 load-bearing 의미"를 헤더에 요약. 게이트가 아니라 boy-scout 점진(아래 "강제(hard guardrail)가 아니라 점진 권고(soft)"와 정합).
- **(3) `// ADR-NNNN` 앵커 점진 확대** — load-bearing 코드에 결정 앵커, 신규·수정분부터.

진화형 컨벤션 본문(실천 규약·예시)은 `docs/reference/commenting-conventions.md`(살아있는 캐논)에 두고, 이 ADR은 "왜 B인가"만 남긴다(근거 중복 금지 — ADR-0031 전례).

강제(hard guardrail)가 아니라 **점진 권고(soft)** 다. rot 정리·헤더 확대 모두 boy-scout(파일 만질 때 곁다리)로 한다 — 리서치가 짚은 "문서=soft context, 대량정리는 표본편향 위험".

## 거부한 대안
- **선택지 A (최소 — 컨벤션 암묵 유지, 점진 보강만)** — 비용·rot 위험은 최소이나 **표준이 암묵적이라 다음 세션이 overview "한눈 이해" 품질을 빠뜨린다.** engram이 이미 ~85% 따르고 있어도 명문화 없이는 신규 파일에서 누락이 반복된다.
- **선택지 C (강 — NL-outline식 자동 갱신 living-doc 툴링)** — rot를 구조적으로 차단하고 에이전트 친화 최강이나 **고비용·불확실(현 단계 over-engineering).** 저위험-장기가 아니라 고비용-불확실 영역이라 판단 기준상 "껍데기만"에 해당. Rust/Tauri 환경에서 living-doc 툴링이 현실적인지도 미조사(리서치 §5 공백). 필요해지면 별도 prior-art + ADR.

## 근거
방법론·실증 근거는 전부 `docs/research/code-commenting-conventions-research-2026-06-23.md`에 둔다(단일 출처 — 딥리서치 내용을 ADR 본문에 복붙하지 않는다, ADR-0031 전례). 요지만:
- **사용자 가설 = 조건부지지.** naive형("전반적으로 더 자세히")은 반증 — LLM 생성 verbose 주석의 redundant·hallucination 역효과 실증. 다듬은형("인라인은 좁히고, '한눈 이해'는 overview 별도 계층, 자동 갱신, 인간 리뷰 대체 안 함")은 지지.
- 좋은 관행 = "주석을 줄인다"가 아니라 "책임을 좁힌다"(4층위). engram 기존 규약과 우연히 ~85% 일치.

## 영향 / 불변식
- CLAUDE.md `## 컨벤션` 절이 캐논(`docs/reference/commenting-conventions.md`)을 가리킨다 — 본문은 캐논에 두고 베끼지 않는다(rot 방지·단일 출처).
- 점진 권고(soft)지 게이트가 아니다 — 코드 일괄 변경을 강제하지 않는다. overview 헤더를 기존 코드에 일괄 다는 작업은 이 결정에 포함되지 않는다(boy-scout로만).
- **위치 잠정:** 캐논을 `docs/reference/`에 둔 건 잠정이다. 문서 프로세스(reference 슬롯의 정식 역할)가 정립되면 캐논 위치를 재조정하고, 이 ADR과 CLAUDE.md 라우터 링크를 함께 갱신한다.
