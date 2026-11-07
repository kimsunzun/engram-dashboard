# Gate scope-escape check — whether to add it to the conventions

> **Status:** Ready to start · **Canonical index:** [`../todo.md`](../todo.md)

- **Problem:** ★**A gate's scope is a path, but a move changes the set of files that path covers. Nothing compares the two.**★ CI source-scan gates (`rg "^\s*use tauri" <경로>` and the like) set their scope by path, so when a file moves **out of** that path, the gate passes silently with 0 matches — the invariant is not upheld and there is no signal.
- **Actual occurrences:** It fired **twice** in a single session of the ADR-0175 crate boundary cleanup, and ★**both times tests, build and CI were all green**★. In phase 2 the coder happened to stop it by setting up a gate in the new home, and in phase 3 the review caught it — in neither case did an automatic check catch it.
- **★The real cause is the wording★:** The ADR wrote "**a new crate** registers gates in three places", but that 「새 crate」 (new crate) **did not cover 「기존 패키지 안으로의 이사」** (a move into an existing package). So nobody looked at the gates again when moving.
- **Minimal fix (candidate):** Widen the trigger wording to 「새 crate **또는 게이트 스코프를 벗어나는 이사**」. One line.
- **Check procedure (candidate):** Extract the path list of the CI source-scan gates and compare, with `comm -23`, **the set of tracked files each path covers before / after the move**. The names left in the output are the files that slipped out of view.
- **Why not now:** **It is a convention change, so where to put it awaits judgment** — whether fixing the wording alone suffices (one line in the ADR trigger), whether to load it as a `ci.yml` gate, or whether it belongs to the qa binding (`.claude/skill-bindings/qa.md`) is undecided. The three differ in nature and these are load-bearing documents, so it is not a place to choose arbitrarily.
- **Full text:** the 2026-08-26 section of `~/.claude/skills/qa/feedback.md`. ★**It is a file outside the repository and is not tracked**★ — it may not exist on other PCs, and if it does not, the summary above is the whole of this item.
