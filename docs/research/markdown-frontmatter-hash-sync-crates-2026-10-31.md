# Markdown excerpt · SKILL.md frontmatter · rev hash · gate/trash locks — external crate choice on the server side (investigation · medium)

- **Status:** medium · 2026-10-31 · 1 collector + main grounding + codex adversarial review (effort high) — verdict FIX. The server's share of TRD `docs/process/S25-preset-library/trd.md` §5-1 step 0a (external library investigation per implementation part).
- **Occasion:** the preset library server builds (1) a markdown body excerpt (2) SKILL.md frontmatter validation (the validator must match codex) (3) the rev hash of the attachment list · files (4) the scan gate · trash lock. For each, decide whether to use a crate or write it by hand. TRD §1-12 already chose serde-saphyr 2.0.0 as the YAML parser, and the rev hash was 「미정」 (undecided).
- **Confidence legend:** certain = primary source + independent cross-confirmation (main grounding or codex recheck agrees) · likely = single primary source, no rebuttal · uncertain = guess/untested · contested = the review rebutted with counter-evidence.

## Conclusion

| Need | Pick | Reason | Runner-up |
|---|---|---|---|
| Markdown excerpt | **pulldown-cmark 0.13.6** (2026-10-30 · MIT · not in the lock file = new) | An event stream, so it is easy to skip code · tables · HTML | comrak 0.59.0 (BSD-2 · heavier). markdown-rs 1.0.0 is slow |
| Frontmatter split | **A hand-written split (~20 lines, following the codex `extract_frontmatter` rule as is) + serde-saphyr parsing** | If it differs from codex's rules, it blocks skills that codex reads or lets through ones it cannot read | None. gray_matter 0.3.2 (pulls in yaml-rust2 · rules ≠ codex) · yaml-front-matter 0.1.0 (2021 · serde_yaml) rejected |
| rev hash | **sha2 `"0.10"` (SHA-256)** — 0.10.9 | Even digest · block-buffer · cpufeatures are already in the lock file, so 0 new packages. The output is fixed by the specification | sha1 0.10.6 (already in the daemon graph · via tungstenite — if the token is not for security, completely 0 new). xxhash-rust 0.8.19 (BSL-1.0 · new) |
| Gate · trash lock | **std `Mutex` + `Condvar` by hand (~30–50 lines each)** | A ticket counter gives FIFO. A mixed condition like 「스캔은 단독, 편집은 동시」 (scan alone, edits concurrent) cannot be expressed by lock fairness. Uses `notify_all` | parking_lot (`unlock_fair`). See the correction below |

★Corrections the review forced★ (the body below is the corrected edition)

1. **The codex repair is ported in full, not only the colon case.** The collector-stage recommendation was 「최소 콜론 경우」 (the minimum colon case). The review pointed out that a broken flow-shaped value of an unknown field (`tags: [next,@supabase/ssr]`) is also a repair target, and it was corrected to 「전부 이식」 (port everything). The rules are in a separate section below.
2. **The pulldown-cmark excerpt explicitly skips the metadata block.** `ENABLE_YAML_STYLE_METADATA_BLOCKS` does not drop `---` but emits a `MetadataBlock` event that contains text. The collector-stage 「`---` 를 버린다」 (drops the delimiter) description is corrected on this point. Table events appear only when `ENABLE_TABLES` is turned on.
3. **parking_lot 0.13.0 removed automatic eventual fairness and its MSRV rose to 1.95** (`unlock_fair` remains). When mentioning 0.13 as a candidate, write both of these together.
4. The collector's statement that the existing investigation document (`skill-md-recognition-claude-codex-2026-10-20.md`) missed these two was **only half right** — colon repair and BOM rejection are already written (line 40), and what is missing is only the whole repair algorithm and the mechanism by which the BOM is rejected.

## 1. Markdown excerpt

| Name | Version · date · license | In the lock file? | Fit | Confidence | Source |
|---|---|---|---|---|---|
| pulldown-cmark | 0.13.6 · 2026-10-30 · MIT | No (new) | Event stream · skipping code/tables/HTML · metadata block option | likely (crates.io + the review confirmed the option behavior in the v0.13.6 source) | crates.io API · the review's `parse.rs` L2203 · `lib.rs` L209 |
| comrak | 0.59.0 · BSD-2 | Unconfirmed | AST-based · heavier | likely | crates.io API |
| markdown-rs | 1.0.0 | Unconfirmed | Slow | uncertain (collector's sole description) | crates.io API |

Excerpt implementation notes (review reflected):

- If `ENABLE_YAML_STYLE_METADATA_BLOCKS` is turned on, the frontmatter span does not disappear but comes out as a `Text` event between the `MetadataBlock` start/end events. **The excerpt side must explicitly skip that subtree** or the frontmatter YAML leaks into the excerpt.
- To rely on table events, turn on `ENABLE_TABLES` first (if it is off, a table is read as an ordinary paragraph).

## 2. SKILL.md frontmatter — rules that must match codex

The validator **must be the same** as the rules by which codex reads skills. If they diverge, our validator blocks skills that codex loads. Basis = codex `codex-rs/skills/src/parser.rs` (L40–206) @ rust-v0.156.1 — main grounding supported it by comparing `repair_frontmatter_scalar_fields` (L52–57 · L98 retry) and `extract_frontmatter`/`MissingFrontmatter`. The review reconfirmed that `parser.rs` of the latest stable version 0.166.0 (2026-10-29) is **identical** to 0.156.1. Both versions lock serde_yaml 0.9.34+deprecated. **Confidence = certain** (source + main + review agree).

### 2-1. Delimiter split (`extract_frontmatter`)

- Split lines with `lines()` (CRLF safe).
- The first line must be `trim() == "---"` to start. The frontmatter runs up to the next line that is `trim() == "---"`.
- If there is no delimiter or zero lines, `MissingFrontmatter`. **A block of only whitespace passes on to the YAML stage instead of being MissingFrontmatter** (the review corrected this — the collector stage lumped it together as 「empty → Missing」).
- **The BOM is rejected not by an intended rule but as a side effect of `trim()`** — `trim()` does not strip U+FEFF, so the first line does not look like `---`. So **do not strip the BOM before validating** (stripping it would let through a file that codex rejects).

### 2-2. YAML parsing and repair

1. Deserialize with `serde_yaml` into `{name, description, metadata.short-description}`. If this stage fails, go through the repair pass **only once** and try again.
2. Repair targets (**all top-level scalar fields, not only known fields** — the review's correction):
   - A value where an **unquoted scalar**, not a quoted or block scalar, contains **a colon + any whitespace** (not only `: ` but any whitespace after the colon) — e.g. `description: Build for AWS: ECS`.
   - A value that begins with `[` `{` `@` `` ` `` and is **wrongly read as flow** — unknown fields included. The review's example: `tags: [next,@supabase/ssr]`.
3. Repair method: **wrap the whole value in single quotes**. A single quote inside the value is **doubled** (`'` → `''`). A **trailing `#` comment is preserved**. Block scalars (`|` `>`) and already-quoted values are **left untouched**.
4. If parsing still fails after the repair, return **the original error** (not the repaired version's error). At that point `InvalidYaml`.
5. Implication for us: if we only validate strictly with serde-saphyr and do not port the repair, we will **wrongly block skills that codex loads.** Port the whole repair, and check the port against the upstream pinned tests (`codex-rs/skills/src/parser_tests.rs` L56–85 @ rust-v0.156.1) and by running the two parsers side by side and comparing.

### 2-3. Checks after deserialization (they do not trigger repair)

- **name:** fold whitespace (consecutive whitespace → one). If empty or missing, substitute the **folder name** (per the investigation document — the actual folder after following the junction). An error if the final name's `chars().count() > 64` (Unicode scalar count — the review reconfirmed it against the core rule).
- **description:** fold whitespace. An error if missing or empty. No length cap (the review reconfirmed).
- If `metadata.short-description` is only whitespace, treat it as absent (the review reconfirmed).
- The two checks above (description missing · name length) happen **after deserialization**, so they do not trigger repair.
- claude accepts all of these (collector's description · the existing investigation document is canonical).

### 2-4. Candidate comparison

| Name | Version · date · license | In the lock file? | Fit | Confidence | Source |
|---|---|---|---|---|---|
| Hand-written split + serde-saphyr | serde-saphyr 2.0.0 (already chosen by TRD §1-12) | Separate (TRD) | Can be written following the codex rules as is | likely — the split side is certain from source, **edge differences between parsers are untested** | codex `parser.rs` |
| gray_matter | 0.3.2 | No (pulls in yaml-rust2) | Rules ≠ codex | likely | crates.io API |
| yaml-front-matter | 0.1.0 · 2021 | No | Depends on serde_yaml · old | likely | crates.io API |

## 3. rev hash

| Name | Version · date · license | In the lock file? | Fit | Confidence | Source |
|---|---|---|---|---|---|
| **sha2** | 0.10.9 (`"0.10"` pinned) | Yes (+ digest · block-buffer · cpufeatures — 0 new). **It newly enters the daemon binary** (for now it comes via tauri/wry) | Output fixed by the specification · stable | certain (main confirmed with Cargo.lock + the review reconfirmed its absence from the daemon graph) | Cargo.lock · crates.io |
| sha1 | 0.10.6 | **Already in the daemon graph** (via tungstenite) | If it is a token not for security, completely 0 new | certain (review reconfirmed) | 〃 |
| xxhash-rust | 0.8.19 · BSL-1.0 | No (new) | Fast · stable output | likely | crates.io API |
| `DefaultHasher` · rustc-hash · fxhash | — | — | Rejected: `DefaultHasher` is unstable between versions · rustc-hash 2.0 changed its algorithm and depends on `usize` · fxhash is 2017 · platform-dependent | likely (collector's description · untested) | Collector |

Pitfalls (collector's description · implementation design guidance):

- **Hash raw bytes only.** Do not go through `std::hash::Hash` (it varies by implementation · version).
- **The rev of the attachment list needs a canonical encoding** — length-prefixed UTF-8 + fixed LE u64 + a default-preset slot marker. Without this, `["ab","c"]` and `["a","bc"]` collide.

## 4. Gate · trash lock

| Name | Version · date · license | In the lock file? | Fit | Confidence | Source |
|---|---|---|---|---|---|
| **Hand-written std Mutex + Condvar** | std | — | A ticket counter gives FIFO regardless of the mutex's barging · conditions can be written freely (scan alone · edits concurrent) | likely (a design judgment — confirm in implementation tests) | Collector |
| parking_lot | lock 0.12.5 (via tauri) · latest 0.13.0 (2026-10-21) | Only 0.12.5. **Not in the daemon graph** (new) | Hand-over via `unlock_fair`. **Correction:** 0.13.0 removed automatic eventual fairness and its MSRV rose to 1.95 (`unlock_fair` remains) | certain (CHANGELOG 0.13.0 + review) | CHANGELOG 0.13.0 |
| spin::TicketMutex | — | — | Rejected: it spins | likely | Collector |
| `ticket_lock` crate | — | — | 404 on crates.io — does not exist | certain | crates.io API |

Basis for fairness:

- On Windows (non-win7), std `Mutex` is not SRWLOCK but a **futex (WaitOnAddress)** — there is no hand-over (handoff) and barging occurs. Confidence = **certain** (rust master source + the review reconfirmed on the rust 1.95.0 tag).
- So we do not trust the lock's own fairness and **build FIFO directly with a ticket counter**. A mixed condition like 「스캔은 배타, 편집은 동시」 (scan exclusive, edits concurrent) cannot be expressed by lock fairness anyway. Waking uses `notify_all` (the waiters have different conditions, so waking only one may wake the wrong one).

## Adversarial review result (codex · effort high · verdict FIX)

| # | Severity | Finding | Handling |
|---|---|---|---|
| 1 | MED | Repair fixes not only the colon case but also a broken flow value of an unknown field (`tags: [next,@supabase/ssr]`). A full port + the upstream test (`parser_tests.rs` L56–85) and a comparison of the two parsers are needed | **Reflected** — wrote the full rules in 2-2 and corrected it to 「전체 이식」 (full port). The comparison test is in the measurement list below |
| 2 | MED | The pulldown-cmark metadata option emits a `MetadataBlock` event that contains text. The subtree must be skipped explicitly, and for tables `ENABLE_TABLES` must be turned on | **Reflected** — implementation notes in section 1 |
| 3 | MED | parking_lot 0.13.0 removed automatic eventual fairness and raised the MSRV to 1.95 (`unlock_fair` is kept) | **Reflected** — section 4 table + conclusion correction 3 |
| 4 | LOW | Repair detects colon + any whitespace · preserves a trailing comment · doubles single quotes · returns the original error when repair fails. MissingFrontmatter = no delimiter or 0 lines (a whitespace-only block passes on to YAML). The description-missing · length checks come after deserialization and so do not invoke repair | **Reflected** — 2-1 ~ 2-3 |
| 5 | LOW | The existing investigation document (`…2026-10-20.md:40`) already wrote colon repair and BOM rejection — what is missing is only the whole repair algorithm and the BOM mechanism | **Reflected** — conclusion correction 4. Corrected the collector's 「문서가 빠뜨렸다」 (the document missed it) statement. Supplementing that document is **left** (this report does not fix it · whether to put one line in TRD §1-3 is the main session's call) |

Reconfirmed (review): codex latest stable version 0.166.0 (2026-10-29) `parser.rs` = identical to 0.156.1 · both versions lock serde_yaml 0.9.34+deprecated · core rules confirmed (name ≤ 64 Unicode scalars · description has no length cap · an empty short-description is treated as absent) · sha2 0.10.9 is in the lock file · sha1 0.10.6 is in the daemon graph (via tungstenite) · sha2 · parking_lot are not in the daemon graph · confirmed on the rust 1.95.0 tag that the Windows non-win7 std Mutex is futex/WaitOnAddress + barging.

## Gaps · measurements needed (implementation pilot)

- **Edge differences between serde-saphyr and serde_yaml are untested** — duplicate keys · type coercion like `description: 123` · tabs · anchors · YAML 1.1 versus 1.2. Build a **golden test that runs the two parsers side by side** with the existing investigation document's 26 pinned tests + repair cases + the upstream `parser_tests.rs` L56–85.
- The exact version of serde_yaml that codex uses was confirmed only up to the lock (0.9.34+deprecated) — behavior differences from other versions are untested.
- The cost of newly shipping sha2 in the daemon (binary size · build) is unmeasured. Whether to replace it with sha1 and go completely 0 new depends on the premise that rev is not for security — a decision is needed.
- The FIFO · 「스캔 단독 / 편집 동시」 (scan alone / edits concurrent) behavior of the hand-written ticket gate — prove it by tests (collector's design judgment).
- The proposed size 「~20줄 · ~30–50줄」 (~20 lines · ~30–50 lines) is the collector's estimate.

## Sources

- https://github.com/openai/codex/blob/rust-v0.156.1/codex-rs/skills/src/parser.rs (L40–206 · repair L52–57 · L98) · the same version's `codex-rs/skills/src/parser_tests.rs` L56–85 · 0.166.0 `parser.rs` confirmed identical (review)
- https://github.com/rust-lang/rust/blob/master/library/std/src/sys/sync/mutex/mod.rs (· rechecked on the rust 1.95.0 tag)
- pulldown-cmark v0.13.6 `parse.rs` L2203 · `lib.rs` L209 (review)
- parking_lot CHANGELOG 0.13.0 (2026-10-21)
- crates.io API — pulldown-cmark · comrak · markdown-rs · gray_matter · yaml-front-matter · sha2 · sha1 · xxhash-rust · parking_lot · ticket_lock(404)
- Repository `Cargo.lock` (sha2 0.10.9 · sha1 0.10.6) · `docs/research/skill-md-recognition-claude-codex-2026-10-20.md` (:40) · TRD `docs/process/S25-preset-library/trd.md` §1-12 · §5-1
