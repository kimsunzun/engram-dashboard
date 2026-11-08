# ADR-0323: `daemon\cache\` = only data that can be rebuilt — it may be deleted at any time

- 상태: 제안 (2026-10-31, basis: user decisions B14 (2026-10-19 「cache로 하자. 그래야 유저가 인지하고 삭제하지」) · P10)
- 관련: Amends ADR-0264 (adds a third category) · ADR-0332 (settings check) · TRD `docs/process/S25-preset-library/trd.md` §1-1 · §1-4 `validate.rs` · §7 29

## 맥락
Where does the daemon put the settings schema it downloaded? Today the data root is split into the protected `state\` and the run-time volatile `run\` (ADR-0264).

## 결정
1. `daemon\cache\` holds only data that can be rebuilt — it may be deleted at any time. It is separated from `state\` (what is protected) and `run\` (run-time volatile · swept at boot).
2. The first occupant = the downloaded settings schema `daemon\cache\schemas\…`. Data that can be rebuilt in the future also goes here.
3. A copy of the schema always ships with the distribution, and here is only the updated copy (P10).

## 거부한 대안
- **The schema in `state\` or somewhere in the data folder** — we cannot tell from the name whether it may be deleted (user reason — B14).

## 근거
User decisions B14 · P10.

## 영향 / 불변식
- Behavior must hold even if everything under `cache\` is deleted — if it is deleted, it is downloaded again at the next version check (TRD §1-4).
- Data that must be protected is not put here. The place is provided by the daemon's `DataLayout`.
