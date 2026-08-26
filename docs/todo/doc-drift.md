# 문서 드리프트 묶음 — 작고 독립적

> **상태:** 착수 가능 · **정본 색인:** [`../todo.md`](../todo.md)

한 브랜치(`v0.3.0/chore/misc`)에 모아 처리한다. 순서 없음, 서로 무관.

- ★**`0` 자리채움 범위 누락 — 이것부터**★. `CLAUDE.md` 「핵심 불변식」과 `.claude/skill-bindings/review.md` 가 「읽기는 건너뛰고 쓰기는 `0` 자리채움」을 **범위 없이** 적는다. 그 비대칭은 디스크 serde 전용이고, wire 에서 `0` 은 정당하게 뽑히는 표식이라 특별취급이 오히려 회귀다. **한 줄 수정인데, 직전 세션 구현 시도의 3분의 1이 이걸 다시 규명하는 데 들었다.**
- ADR-0175 본문 사실 오류 5종.
- `README.md` 의 `--exclude engram-dashboard` 드리프트(그 제외는 2026-08-25 에 걷혔다 — 정본은 `CLAUDE.md` 「빌드·검증 명령」).
- `../reference/architecture-overview.md` 에 `base` crate 절이 없다.
- 게이트 정규식이 못 닫은 구멍 — `use   tokio :: time;` 같은 공백형.
- `daemon` 의 미사용 `tracing-subscriber` 선언(`crates/engram-dashboard-daemon/Cargo.toml`). 걷을지는 판단 필요 — 사용 0줄 실측됨.
