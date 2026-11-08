# ADR-0320: Delete = move to a bundle folder in our hidden `.trash` inside the library root

- 상태: 제안 (2026-10-31, basis: user decisions B8 / F9-confirmed (2026-10-19 「ㅇㅇ 우리 trash 폴더로해」) · G31 (2026-10-30 「휴지통 경로 … 시스템에서 알아서 보내는게 맞는것같은데 고정경로로하고 나중에 변경경로로」) · G44 (2026-10-31) + main M22 · M46 · M52 · M53 (reported) · research `docs/research/trash-unavailable-delete-practice-2026-10-19.md`)
- 관련: ADR-0312 · ADR-0335 · ADR-0322 · ADR-0324 (trash lock) · PRD R18 · TRD §1-1 · §1-4 `store.rs` · §6-5 1 · §7 20

## 맥락
Where do we send a library entry when it is deleted? Humans, the LLM and the CLI all delete.

## 결정
1. Delete = move into the hidden `.trash` inside the library root. The place = a fixed `<루트>\.trash\` decided by the system — a changeable place comes later if a need arises (G31).
2. It is a rename inside the same root. It is hidden from the tree, the list and search. The root itself cannot be deleted. Deletes by the LLM and the CLI go to the same place.
3. If the root is a git working tree, add `.trash/` to that repository's `.git/info/exclude` (local · not committed — M22).
4. **One operation = one bundle** — placed inside the `.trash\<UTC>-<id>\` folder under the original path. Files pushed out by a delete or an overwrite (P5) all go this way, and a move that pushed out two files is also one bundle (M46 · M52 · M53). Emptying sends this bundle folder as a whole to the OS trash (G44 · ADR-0322).
5. Moving to `.trash` and restoring queue with emptying under the trash lock (ADR-0324 Decision 9).
6. Emptying and permanent deletion = ADR-0322.

## 거부한 대안
- **Delete to the OS trash** (`IFileOperation` called by a dedicated STA worker thread in platform — the candidate until 2026-10-18) — it may silently delete permanently instead of using the trash (Microsoft docs `FOF_WANTNUKEWARNING` — if called without UI, that warning is buried · network drives · volumes without a trash · over capacity) · and emptying cannot be limited to humans (Explorer and other tools empty it) (§7 20).
- **The `trash` crate** — a third `windows` copy on the same OS trash path · a `CoInitialize` failure panic (§7 20). It was also withdrawn from the emptying path — there is no per-item report (ADR-0322 · M57).
- **Reject if the OS trash cannot be used** (old F8(b)) — `.trash` is a same-volume rename, so the question itself disappeared (decision record F9-confirmed).
- **Permanent deletion + a strong confirmation** (VS Code shape) — an agent call has no human to answer (§7 20).
- **One central trash place** — it needs a copy across drives (G31 — a same-volume rename is fast and has no half-moved state). ★This reason came from the multiple-roots period (G25)★ — we did not re-check whether it still holds with one root (ADR-0312). (Basis for rejection: weak) Basis (main 2026-11-01 — reported · M98): it holds with one root too — the user picks the root place (R9 · G39), and it can be on a different volume from a central place (a data folder etc.). Then deletion becomes copy + delete rather than a rename, a half-moved state appears, and the question removed by the same-volume rename comes back to life.
- **A changeable trash place** — later if a need arises (G31 「고정경로로하고 나중에 변경경로로」).

## 근거
User decisions B8 · G31 · research (2026-10-19 — the app-specific `.trash` precedent = Obsidian).

## 영향 / 불변식
- No other `library.*` can point under `.trash` (closing the back door — ADR-0322).
- The agent's own `rm` and file tools bypass `.trash` — accepted (P13).
