# ADR-0005: finalize 정확히 1회 (pump 단독)

- 상태: 확정 (S3, S10 OutputCore 이관)
- 관련: CLAUDE.md 핵심 불변식 · `output_core.rs::{finalized,finish}`

## 맥락
terminal 전이·알림이 여러 경로(kill, 자연 종료, fallback)에서 발생할 수 있어 중복·경합 위험이 있다.

## 결정
`OutputCore.finalized.swap(AcqRel)` 원자 게이트로 terminal 전이/알림을 **정확히 1회**만 발행한다. 발행 주체는 **pump 단독**(EOF 감지 시 `core.finish(reason)`).

## 거부한 대안
- **각 경로가 직접 status 전이** — kill과 자연 종료가 동시에 발화하면 중복 알림, 상태 경합. 프론트가 terminal을 두 번 받음.

## 근거
master drop → reader EOF는 어떤 종료 경로든 pump를 한 번 깨운다. 그 단일 지점에서만 finish하면 중복이 구조적으로 불가능.

## 영향 / 불변식
- 과도기 `Exiting`은 manager가 `enter_exiting()`으로 트리거, **terminal(`Killed`/`Exited`/`Failed`)은 pump만**.
- 프론트는 status_changed로 terminal 판정 금지 → `agent-list-updated`로 판정(→ ADR-0007 epoch와 함께).
