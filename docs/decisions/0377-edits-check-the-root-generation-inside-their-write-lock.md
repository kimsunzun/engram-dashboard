# ADR-0377: Edits take the root generation as an optional argument checked inside the edit's own write-lock take; a stale one is refused with root-generation-stale

- 상태: 확정 (2026-11-17, 근거: S25 step-9a TRD §8 (b) B6, PRD+TRD review PASS 2026-11-17 (full)) · 부분 폐기 by ADR-0381 (the starting value of the root generation)
- 관련: ADR-0324 (library lock, generation tickets) · ADR-0366 (decision 1 per-item writes, decision 3 exclusive root change) · ADR-0379 (refusal shape and bus-code map) · step-9a TRD `docs/process/S25-preset-library/trd-library-edit.md` §1 · §3.3 step 3 · §4 · §5 · §8 (a) · §8 (b) B6 · §9 U1 · Amended by ADR-0381 (the starting value of the root generation)

## 맥락
Step 9a makes `set_root` live, so the root can change while edits are queued. That the root generation enters edits as an optional argument was already decided (M14 · M26, TRD §8 (a)); what was open is where it is checked. The root generation lives in the gate's `Numbers.root` (not in the config) and `status` answers it as `root_generation`. `change_root` is exclusive from its ticket to its publish, and edits queued behind it run against the new root (ADR-0366 decision 3).

## 결정
1. **The root generation is checked inside the edit's own write-lock take** — a root-checked sibling of `LibraryService::change`. The check and the edit's disk read and write happen in the same take.
2. **A stale supplied generation is refused** with `Conflict` · `root-generation-stale`, args `{given, current}`. Nothing is applied.
3. **Absent = the current root** (the argument is optional, M26).

## 거부한 대안
- **Check the root generation before the ticket** — the check could pass and the edit then write after a queued root change, into a root it did not ask for (TRD B6, §4). QA mutation: 「the generation check moved before the ticket」 (TRD §9 U1).

## 근거
- TRD B6 reason: a check before the ticket could pass and then write after a queued root change.
- `change_root` is exclusive from its ticket to its publish (ADR-0366 decision 3), so only a check inside the edit's own take sees the root the edit will write into (TRD §4).
- Step-9a PRD+TRD review (full, Claude lead + codex) PASS on `43f64c4f`, 2026-11-17 (TRD §13).

## 영향 / 불변식
- An edit can never write into a root it did not ask for (TRD §4).
- `root-generation-stale` is a new code (TRD §5); its class `Conflict` maps onto `CONFLICT`, which does not retain the request id (ADR-0379).
- Test that must hold (TRD §9 U1): a stale root generation is refused inside the lock, including an edit queued behind a root change.
- Implemented in step 9a U1 (the root-checked `change`); U2 to U4 edits use it.
