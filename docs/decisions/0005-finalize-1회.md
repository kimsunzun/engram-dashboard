# ADR-0005: finalize exactly once (pump alone)

- 상태: 확정 (S3, S10 moved into OutputCore)
- 관련: CLAUDE.md 핵심 불변식 · `output_core.rs::{finalized,finish}`

## 맥락
The terminal transition and notification can arise from several paths (kill, natural exit, fallback), so there is a risk of duplicates and races.

## 결정
The atomic gate `OutputCore.finalized.swap(AcqRel)` emits the terminal transition/notification **exactly once**. The emitter is **the pump alone** (`core.finish(reason)` on EOF detection).

## 거부한 대안
- **Each path transitions status directly** — if kill and a natural exit fire at the same time, notifications are duplicated and status races. The frontend receives terminal twice.

## 근거
Master drop → reader EOF wakes the pump once, whichever exit path is taken. If finish happens only at that single point, duplicates are structurally impossible.

## 영향 / 불변식
- The transitional `Exiting` is triggered by the manager through `enter_exiting()`; **the terminal states (`Killed`/`Exited`/`Failed`) come from the pump only**.
- The frontend must not decide terminal from status_changed → it decides from `agent-list-updated` (→ together with the ADR-0007 epoch).
