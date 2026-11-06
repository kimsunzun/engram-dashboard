# S31 PRD — the gap where source-regex gates miss spaced paths (`tokio :: time`)

## 1. What · why

- Several of CI's source-regex gates find a crate path with `\b(<이름>)::`. Rust allows whitespace between path segments, so `use   tokio :: time;` and `tokio :: spawn(…)` compile and do not match this regex.
- Normally `cargo fmt --check` (the ci.yml `gates` job) stops it first — rustfmt rewrites the spaced form to `tokio::time`, so the spaced form as written FAILs at that step.
- The remaining gap = places rustfmt does not touch — the body of a brace macro call (`m! { tokio :: spawn(f) }` — a plain `macro_rules!` body is rewritten by rustfmt) and `#[rustfmt::skip]` regions. A spaced form there passes both the fmt gate and the regex gate. Some of the target gates have no compiler backstop, so the regex is the only wall (ci.yml comment).
- Source = `docs/todo/doc-drift.md` 「게이트 정규식이 못 닫은 구멍」.

## 2. Scope / non-goals

- Scope = those source-regex checks in `.github/workflows/ci.yml` that find a path (crate paths · `std::os` · the tool attribute `rustfmt::skip` · `clippy::restriction`) with `::`. The TRD fixes the list.
- Non-goals — false positives inside block comments (`/* */`) · paths that span lines · other name forms that are not whitespace (`use tokio as rt;` · `use std::{net::TcpStream, time::Instant};` · `use std::net;` — turned into a new to-do) · changing the rustfmt config · turning a gate into a compiler check.

## 3. Requirements

- R1 — the target checks catch spaced forms before, after and between multiple segments of `::` (`tokio :: time` · `tokio ::time` · `std :: net :: X` · `super:: sync` · including tabs) exactly like the unspaced form.
- R2 — no target gate changes its decision on the current tree (zero false positives).
- R3 — each gate keeps its existing properties (outside comments only · `\b` boundary · a legitimate neighbor `std::time` passes · file-existence check).

## 4. Acceptance criteria

- AC1 — for each target gate, putting one spaced-form violation line into a copy of a target file makes it FAIL, and an unspaced-form violation still FAILs (confirmed by running).
- AC2 — running the target gates on the current tree gives the same decision as before the fix (PASS) (confirmed by running).
- AC3 — legitimate neighbor samples (`std::time::Instant` · a self-quote inside a comment · `my_tokio::x`) do not FAIL (confirmed by running).

## 5. Open decisions

- None (the todo set the direction — widen the regex). The list of target gates is fixed in the TRD and shown to the user.

## 6. Risks

- False positives where the widened regex catches legitimate code — AC2 and AC3 stop this.
- `"tokio :: x"` inside a string literal is treated the same as the unspaced form even now (not a new risk).
