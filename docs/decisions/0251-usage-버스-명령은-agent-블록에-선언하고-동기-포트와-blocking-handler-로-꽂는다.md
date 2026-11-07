# ADR-0251: usage bus commands are declared in the agent block and plugged in with a sync port and blocking_handler

- 상태: 확정 (2026-09-27, basis: TRD §7 #7)
- 관련: TRD §7 #7 · ADR-0155(decision 1 — the vocabulary is declared next to its producer) · `command` crate · step-log S21

## 맥락
We had to decide where to declare bus commands such as `usage.get` · `usage.refresh` and in what handler form to plug them in.

## 결정
`usage.*` commands are **declared in the agent block** (next to the producer — a reconfirmation of ADR-0155 decision 1). The handler is plugged in with a **sync port + `blocking_handler`**.

## 거부한 대안
- **The daemon's own block + table composition** — composition would have to be newly laid on three surfaces: declaration · routing · handler.
- **An async handler** — the daemon entrance turns the result into `OUTCOME_UNKNOWN` (the response model assumes synchronous completion).

## 근거
TRD §7 #7.

## 영향 / 불변식
The command crate's "declare next to the producer" invariant (ADR-0155) applies to the `usage.*` commands as is. Implementation commits: `56776d6`, `c68b885`.
