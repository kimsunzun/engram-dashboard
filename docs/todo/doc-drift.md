# Doc drift bundle — small and independent

> **Status:** Ready to start · **Canonical index:** [`../todo.md`](../todo.md)

No order, mutually unrelated.

★**`v0.3.0/chore/misc` is an abandoned branch — do not reuse it or try to bring it into master (measured 2026-08-26)**★. It forked **before** the crate boundary restructure (ADR-0175), and pulling in master produces, on top of a huge diff, **three modify/delete conflicts** — because master deleted `.claude/skills/` wholesale (the canonical skill source moved outside this repository).

**What that branch was holding has been recovered.** The contents of the three unmerged commits (skill feedback accumulation + ADR numbering discipline) were **moved verbatim** to the canonical skill source on 2026-08-26 — before moving, absence was confirmed by content comparison (all five representative phrases returned 0 hits), and after moving, verbatim match and 0 conflict markers were verified. ★So no unique content remains on this branch★. For new work, create a new branch from master. **The branch itself is not deleted** (this repository has a convention of not deleting branches).

**What remains:**

- Five factual errors in the ADR-0175 body. ★Which five is not written down anywhere yet — the first step of starting is to build that list.★
- The unused `tracing-subscriber` declaration in `daemon` (`crates/engram-dashboard-daemon/Cargo.toml`). Whether to remove it needs judgment — 0 lines of use were measured. Removing it changes the manifest, so a build check follows.

**Closed (2026-08-26):**

- ~~`0` placeholder range omission~~ → **closed.** We put 「범위는 `agents.json` 디스크 serde 뿐 · wire 의 `0` 은 정당한 표식이라 특별취급이 회귀」 into both `CLAUDE.md` 「핵심 불변식」 and `.claude/skill-bindings/review.md`.
- ~~The `--exclude engram-dashboard` drift in `README.md`~~ → **closed.** The flag was removed from the command line, and the comment above it was also switched from 「알려진 실패가 남아 있다」 to 「사유 둘이 차례로 죽어 더는 제외하지 않는다」.
