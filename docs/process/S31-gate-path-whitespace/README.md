# S31 — 소스 정규식 게이트의 공백형 경로

CI 소스 정규식 게이트 셋이 `tokio :: time` 처럼 경로 마디 사이에 공백이 낀 꼴을 놓치는 구멍을 막는다. rustfmt 가 손대지 않는 매크로 본문 · `rustfmt::skip` 구간에서만 실제로 뚫리는 좁은 구멍이다.

- 출처: `docs/todo/doc-drift.md` 「게이트 정규식이 못 닫은 구멍」
- 문서: [`prd.md`](prd.md) · [`trd.md`](trd.md)
- 관련 ADR: 없음(게이트 기대값의 정본 = 대상 crate · 파일 헤더)
- 타임라인: `docs/process/step-log.md` `## S31`
