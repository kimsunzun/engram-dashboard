# ADR-0319: Run preprocessing goes after the reservation and the alive check, before the session is derived — a registry change is one mutate

- 상태: 제안 (2026-10-31, basis: TRD `/review trd` round 2 applied (TRD header history) · M16 · TRD §1-5 (a) · §3)
- 관련: ADR-0226 (first-submit latch) · ADR-0334 · TRD §8 (working-folder share) · §7 17

## 맥락
Where in the start-up flow do we put the pre-run check and comparison (expanding attached presets, deciding blocks)?

## 결정
1. Run preprocessing goes right after the reservation and the alive check, before the session is derived. It queues per agent.
2. A registry change is one mutate (compare-and-replace), so it does not overwrite a value changed in between.
3. A block is also cut off before the control channel is issued.
4. Filling unattended defaults is the next topic (M16). Matching the working folder and the Engram folder share are in §8.

## 거부한 대안
- **Matching in the middle of start-up** — the latch and the incarnation marker see the old copy (§7 17).
- **Before the reservation** — two runs of the same agent overlap (§7 17).

## 근거
Applied the FIX of both reviewers in TRD `/review trd` round 2 — put the preprocessing right after the reservation and the alive check + a registry change as one mutate (compare-and-replace) (TRD header history).

## 영향 / 불변식
- The latch (ADR-0226) and the incarnation marker see only the matched profile. If the preprocessing is moved to another place, the two rejection reasons come back to life.
