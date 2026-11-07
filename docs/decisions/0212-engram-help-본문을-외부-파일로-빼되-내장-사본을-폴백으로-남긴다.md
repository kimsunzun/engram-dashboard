# ADR-0212: Move the engram help body into an external file, but keep a built-in copy as the fallback

- 상태: 확정 (2026-09-19, basis: user decision + measured reflection without a build and byte-identical rendering) · 부분 폐기 by ADR-0220 (file name and the shape of the section markers) · 부분 폐기 by ADR-0285 (decisions 3 and 4, the three rejected alternatives, and the lost guarantee in Impact)
- 관련: ADR-0211(the round in which the contract moved into help) · ADR-0092(content lives in an external MD) · ADR-0100(release manifest) · `prompts/engram-cli-help.md` · `crates/engram-dashboard-daemon/src/bin/engram.rs` · Amended by ADR-0220 (file name and the shape of the section markers) · Amended by ADR-0285 (decisions 3 and 4, the three rejected alternatives, and the lost guarantee in Impact)

## 맥락

ADR-0211 moved the agent-facing mail contract **from the instruction document to `engram help`.** But the help body is **compiled into the binary** as a Rust string constant. After the move, the shape ended up like this:

- The instruction document (`prompts/agent-priming.md`) is an **external MD**, so it takes effect immediately when edited — ADR-0092 nailed this down as 「내용을 코드에 박지 마라」 (do not bake content into code).
- For help, **changing even one character of wording has to go through a build.**

That is, **the contract moved from a file into the binary**, and help brought back the problem that ADR-0092 had removed for the instruction document. The user's judgment = 「빌드할 때마다 문구 수정하는 건 오버다」 ("having to build every time we edit wording is overkill").

## 결정

1. **The body is the single file `prompts/engram-cli-help.md`**, and the screen pieces are separated by **delimiter comments that take up a whole line** (17 sections).
   - ★**One file per page does not hold**★ — since the mail visibility filter removes **pieces inside** the root and agent **pages**, section units are needed anyway. If sections are required, one file is one manifest entry and the whole surface reads as one document.
2. **The place is `prompts/`** — 「에이전트가 읽는 외부 텍스트를 exe 옆에 싣는다」 (put the external text an agent reads next to the exe) is already the meaning of that folder, and the copy and two-way tripwires of ADR-0100 already cover that folder. A new folder would create **one more each** of a copy path, a manifest array and a tripwire.
3. **Path resolution has the same shape as ADR-0092** — the environment-variable override first, and if absent, an exe-based walk-up + a fixed relative path. It does not look at cwd and does not go through the daemon or credentials. **An override failure does not fall down to the fixed path** (inheriting the instruction-document-side rule).
4. ★**If the file is missing or broken, answer with the built-in copy**★ — we also embed the same file in the binary with `include_str!`, and on failure **stdout is a complete screen · exit code 0 · the warning is one line on stderr**. **A file missing even one section is rejected as a whole** (no half screens).
5. **Register it in the manifest** — the release list becomes two files, and the two-way tripwire for missing and surplus covers it as it is.

## 거부한 대안

- **Just leave it as a compiled constant** (keep the status quo) — ★**Rejection = user decision**★. Since the contract moved into help, the frequency of wording edits goes up.
- **Die loudly if the file is missing** — ★**Rejection = both have the same ending**★. `engram help` is the **place where an agent learns of the existence of the surface**. Whether it shows an empty screen or dies with an error, the agent **leaves without the surface** — then a state in which mail exists but cannot be used quietly holds. **A stale but correct screen is strictly better.** We send the warning to stderr so that **stdout, which the LLM reads, is not dirtied** while a human can still notice.
- **Do not keep a built-in copy and use only the file** — rejected. Same reason as above, and as a bonus, **diagnosis becomes impossible on a machine with a broken install**.
- **Re-implement the exe walk-up inside the CLI** — rejected. The mechanisms doing the same job would become two and diverge. ★**The price was paid**★ — the `engram` binary now links one symbol from the discovery crate (that crate is already a daemon dependency, so **no third party was added**). It is true that we crossed the CLI's 「의존 최소」 (minimal dependency) line by one notch, and that is the price this decision accepted.

## 영향 / 불변식

- ★**We confirmed that the render result is byte-for-byte identical to before**★ — we took the 9 screens (root and agent, each shown/hidden · mail overview · the four sub-pages) before and after the change and compared them. The externalization is **not work that changes the wording.**
- **We measured that it takes effect without a build** — we edited the external file and the screen changed with no rebuild. That is the whole purpose of this decision.
- ★**We note one lost guarantee**★ — before, **the compiler enforced** that every sub-item has a page (a match with no catch-all). Now the section id is built from a token, so that enforcement is gone, and **the 「필요한 절 명단」 (list of required sections) check and the built-in-copy test take its place.** If you delete either of the two, **a sub-item pointing to a nonexistent section compiles.**
- **Newline normalization** — this repository is configured to turn into CRLF at checkout, so the parsing folds `\r\n` so that the same bytes come out in any clone. ★If this folding is removed, the screen differs from clone to clone.★
- **Unverified** — we **did not confirm** that actually running the release script puts the new file into the package (it requires going through the full build). We only judged by reading the manifest array and the four places that consume it.
