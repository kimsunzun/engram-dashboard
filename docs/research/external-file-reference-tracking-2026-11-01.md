# Keeping external file references — how mature tools handle references on overwrite, move and restore

| Item | Value |
|---|---|
| Status | collection · grounding · adversarial review (FIX — applied) done · decision (user 2026-11-01) — overwrite = delete + rename (the reference follows the location · within one list the same path keeps only its first position — the same result as A in the table below) · delete chooses how references are handled · mechanism stays path = ADR-0339 |
| Date | 2026-11-01 |
| Method | `/research` medium · design-decision mode — 2 collection tracks (① direct peers ② identity mechanisms · main line) → main grounding (checked Microsoft Learn `ReplaceFileW` · 「Distributed Link Tracking and Object Identifiers」 · the Godot 4.4 UID post against the source text) → cross-family adversarial review (codex) |
| Question | In the current design (ADR-0315) where an agent attaches a library `.md` preset **by path**, how do we solve the problem that an [overwrite] move inside the library ① creates a duplicate of the same path in the agent's list (`[b, b]`) and ② makes an attachment that pointed at the overwritten old file silently receive the new content — is there a mature system that links references without putting our data inside the file (user 2026-11-01) |
| Trigger | S25 stage 4 P3 code review — `[b, b]` duplicate inside one agent after a path rewrite |

Confidence legend = certain (primary material + independent confirmation) · likely (one primary material or several secondary materials) · uncertain (one secondary material · search summary only) · unknown.

## Conclusion

- **Two things must be separated — 「무엇으로 가리키나(메커니즘)」 and 「덮어쓰기 때 붙임을 어떻게 하나(정책)」.** The mechanism (path · OS identity · sidecar file ID) does not decide the overwrite policy — even in Unity, if only the content is swapped in and the destination `.meta` is kept, the reference receives the new content (adversarial review finding · applied). So this decision is about policy (A · B · C below), and the mechanism is fine as the current path (ADR-0315).
- **Tools that point by path are common — they usually fix only moves inside the app, and treat moves outside as 「찾을 수 없음」 + relink** (Obsidian · VS Code · Foam · Lightroom — likely · mostly secondary material). There are exceptions: Zotero automatically relinks linked files under a configured base folder, and Dendron uses frontmatter ids in its index (counterexamples from the adversarial review · applied). The current engram design is in this camp.
- **There are some documented precedents that keep references through overwrite and trash.** The DEVONthink reading list keeps the references of items sent to the trash until the trash is emptied (it is a database structure, so it differs from a path rewrite) · calibre swaps in a format file on the same book (logical id) · Lightroom swaps in the original on import and can keep the old settings (found by the adversarial review — not checked by main · likely). No precedent exactly like engram's 「이동하며 덮어쓰기」 was found.
- **An OS file ID can change on a save that writes to a temp file and swaps it in** — after `ReplaceFile`, the file ID of the result file = that of the replacement file (certain — checked against the source text). But the same function keeps the NTFS **object ID** of the original file (certain — checked against the source text). So 「모든 쓰기가 신원을 깬다」 is an overstatement — what breaks it is an editor that saves with temp + rename (JetBrains safe write · vim `backupcopy=no` etc. — likely), and for the default save of Notepad · Notepad++ · Obsidian · VS Code it is unknown.
- **Even mature implementations that use OS identity (Windows shortcuts · OLE links) use identity and heuristics only as a 「못 찾았을 때 찾아 주는」 aid** — after object ID tracking (NTFS 3.0+ · not on removable media), they scan near the last path by creation date, size and name. Microsoft itself writes 「늘 찾지는 못하고 오래 걸릴 수 있다」 (certain — checked against the source text). digiKam re-finds files moved outside the app with a partial hash + size (found by the adversarial review · likely).
- **Godot 4.4** — if moved outside, dependent scenes show a warning and must be saved again; do not rely on the path fallback · two resources can be swapped while the path stays unchanged (certain — checked against the source text). That post leads with support for moving outside (the summary 「앱 안 이동 권장」 in the old edition was wrong — adversarial review · applied).

## Options for engram — overwrite policy

Example: the library has `리뷰.md` · `코딩.md` · agent X = `[리뷰, 코딩]` · agent Y = `[코딩]`. In engram, move `리뷰.md` onto the place of `코딩.md` with [overwrite] → the old `코딩.md` goes to `.trash`, and the review content sits at the place of `코딩.md`.

| | X result | Y result | Restore | Cost |
|---|---|---|---|---|
| **A. Follow the name + fold duplicates** | `[코딩]` (review content) | `[코딩]` → silently receives the review content | no automatic restore — restoring the old coding makes the names collide, so another name · it must be attached again | current implementation + a few lines to fold |
| **B. Treat the overwritten one as deleted** | `[코딩]` (review content) | `[]` drops off | no automatic restore | a 「덮어써질 경로의 붙임 떼기」 step in the overwrite plan |
| **C. Sending to the trash is also a 「이동」** | `[코딩(리뷰 내용), 휴지통의 옛 코딩]` — both remain | `[휴지통의 옛 코딩]` — 「휴지통에 있음」 | if restored inside engram, the attachment follows it back (with the decided new name if the names collide) · if taken out from outside, it does not | the rules below |

- **Rules C needs** (adversarial review · applied): ① within one overwrite, the two mappings (`옛 코딩 → .trash/…` · `리뷰 → 코딩`) must be applied simultaneously on the original values so they do not chain into each other (the same as the current P3 rule) ② name-collision handling on restore, and rewriting again to the resulting path ③ rollback · redo when the file operation succeeded but saving the references failed ④ what to do with that attachment when the trash is emptied (detach / 「찾을 수 없음」) ⑤ what to do at launch with an attachment that is in the trash (skip / block).
- **A and C differ not only for Y but also for X** — in C, X keeps holding the old coding attachment, now pointing into the trash (「Y 에서만 다르다」 in the old edition was wrong — adversarial review · applied).
- **Changing the mechanism (OS identity · sidecar file ID · hash · app catalog id) is outside this decision** — all three have value in 「바깥 이동 뒤 다시 찾기」, and the overwrite policy has to be decided separately. They stay as candidates to revisit when outside-move handling grows: object ID (NTFS only) · partial hash + size (digiKam) · an app catalog's path↔id table.

## Grounding (main cross-check)

| Claim | Source | Verdict |
|---|---|---|
| after `ReplaceFile` the file ID of the result file = that of the replacement · object ID · creation time etc. keep those of the original file | https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew (Remarks) | supported |
| path references cannot be trusted · rename · backup · restore keep the object ID and copy does not · NTFS 3.0+ only · not on removable media · the heuristic scan does not always find it and takes long | https://learn.microsoft.com/en-us/windows/win32/fileio/distributed-link-tracking-and-object-identifiers | supported |
| Godot — a move outside requires moving `.uid` along with it · a move outside gives dependent scenes a warning + save again · do not rely on the path fallback · two resources can be swapped while the path stays unchanged | https://godotengine.org/article/uid-changes-coming-to-godot-4-4/ | supported |
| Unity — a move outside requires moving `.meta` too · otherwise it is treated as a new asset | https://docs.unity.com/en-us/engine/current/manual/assets-and-media/import-assets/asset-metadata | collector's citation (not checked by main) — likely |
| Obsidian — only moves inside the app update links · moves outside are not fixed | https://forum.obsidian.md/t/auto-update-links-for-files-moved-while-obsidian-is-closed/91670 (user forum) | partly supported (not official docs) |
| Obsidian — moving onto the same name is blocked by default | plugin docs · search summary | uncertain |
| the find/relink UX of Lightroom · Zotero | secondary material · Zotero KB | likely |

## Gaps · limits

- The overwrite (c) behavior is undocumented in every peer — the basis for calling any of A · B · C 「관행」 is weak.
- Whether the default save of Notepad · Notepad++ · Obsidian · VS Code keeps the file ID was not measured (it can be confirmed with a 10-line measurement).
- The link behavior when Obsidian restores from the trash · Logseq `id::` · the Dendron frontmatter id were not read.
- The macOS row is secondary material only.

## Adversarial review

codex (cross-family) · effort high · web search — **FIX**. Applied:

- The options table that mixed mechanism and policy (old A–E) → rewritten with the policy split into A · B · C; D · E moved down to mechanisms (high).
- 「A 와 C 는 Y 에서만 다르다」 — it differs for X too (C leaves the old attachment pointing into the trash) (high).
- The 「되살리기 = 다시 이동이라 자동」 of C — more rules are needed for name collisions · simultaneous mapping · rollback of partial failure · emptying · launch handling (high).
- The overstatement 「편집기 · LLM 쓰기마다 file ID 가 바뀐다」 · it conflicts with the object ID being kept under `ReplaceFile` (high).
- Missing peers: digiKam (partial hash) · DEVONthink (app UUID · keeps references to trashed items) · calibre · Lightroom original swap-in · Zotero automatic relink · Dendron frontmatter id (medium) — put into the conclusion. **Main did not cross-check these sources** (marked likely).
- Godot summary misattribution (medium) — fixed.
- Obsidian official docs on URI · CLI overwrite · trash settings — how links of an overwritten file are handled is still unconfirmed (medium) — left in the gaps.
