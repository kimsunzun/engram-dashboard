# S31 TRD — spaced paths in source-regex checks

- Status: edition 2 · 2026-11-02 · no source-of-truth ADR (the source of truth for a gate's expected values is each target crate / file header — see 「같은 주장」 below). Edition 2 = PRD+TRD review (Claude · codex) applied.
- Branch base = `v0.5.0/chore/skill-apply` (on top of the bindings · CLAUDE.md cleanup). Integration order = after skill-apply.

## Scope and relation to the PRD

Apply PRD R1–R3 · AC1–AC3 to six checks in `.github/workflows/ci.yml` (line numbers = as of this branch).

| Check (step name) | Line | Current pattern | Spaced form |
|---|---|---|---|
| `Gate: base leaf crate — tenants must not reference each other` | 698 | `(crate\|super)::(logging\|…\|panic)` | `super:: sync` · `crate :: time` pass |
| `Gate: replay_flight purity — no transport/wire ingress` | 1262 | `…\b(tauri\|tokio\|engram_dashboard_[a-z_]+\|std::net)::` | `tokio :: spawn` · `std :: net :: X` pass |
| `transport gate 3: pure layer isolation (pending/stream/machine)` | 1339 | the same shape (`tokio\|futures_util\|futures\|…`) | passes |
| `platform gate 6` (`std_os_re`) | 1197 | `…\bstd::os::(?:windows\|unix)\b` | `std :: os :: windows` passes — a file outside the list is not caught |
| shell gate 2 — no lint turn-off | 1028 | `…(?:wildcard_enum_match_arm\|clippy::restriction)` | `clippy :: restriction` passes |
| shell gate 2 — no rustfmt skip | 1031 | `rustfmt::skip\|rustfmt_skip` | `#[rustfmt :: skip]` passes — the guard that blocks regions rustfmt does not apply to itself leaks |

- Out of scope — `net gate 2a`. A space in the first segment (`engram_dashboard_platform :: fs`) FAILs in the hidden-shape check (`:1389`), and a space in a deeper segment (`engram_dashboard_platform::fs :: f`) FAILs on the extraction (`:1368`) set mismatch. Put one sample into the tests to record this claim as a measurement.
- Out of scope — the existence checks at `:964` · `:987` · `:1026` (the safe side: they fail if a spaced form gets in) · paths that span lines.
- Out of scope · new to-do — other name forms that are not whitespace (`use tokio as rt;` · `extern crate tokio as rt;` · `use std::net;` · a multi-item group `use std::{net::TcpStream, time::Instant};`) also pass replay_flight · transport gate 3 (both the old and the new pattern). A one-item group is unwrapped by rustfmt and so is caught by the gate. This is not widened now; it goes up as a new item in `docs/todo/observed-only.md`, and a one-line 「알려진 한계」 is written only at each gate's source-of-truth place (replay_flight = the `ci.yml` step comment · transport gate 3 = the transport `lib.rs` header).

## What changes

- Allow `\s*` before and after `::` — `\s*::` after a name, and `\s*::\s*` between every pair of segments for a multi-segment name.
  - base: `(crate|super)\s*::\s*(logging|text|time|path|sync|testing|writable|file|panic)`.
  - replay_flight: `^(?:[^/]|/[^/])*?\b(tauri|tokio|engram_dashboard_[a-z_]+|std\s*::\s*net)\s*::`.
  - transport gate 3: the same shape (the name list stays).
  - platform gate 6: `^(?:[^/]|/[^/])*?\bstd\s*::\s*os\s*::\s*(?:windows|unix)\b`.
  - shell gate 2: `clippy\s*::\s*restriction` · `rustfmt\s*::\s*skip|rustfmt_skip`.
- Leave the prefix guard · `\b` · file-existence checks · the decision `case` as they are (R3).
- The patterns sit inside bash double quotes · single quotes — `\s` goes to rg as is.

## Same claim — copies outside the documents

- Places where the old pattern strings are still alive on this branch (confirmed with `rg -F` — `.github` is a hidden folder, so `--hidden`):
  - The step explanation comment of each step in `ci.yml`.
  - The `crates/engram-dashboard-base/src/lib.rs:55` header.
  - The `crates/engram-dashboard-transport/src/lib.rs:48` header 「격리 게이트」.
  - The `crates/engram-dashboard-platform/src/lib.rs:169-171` matcher explanation (+ the `:174` 「알려진 한계」 list).
  - The `src-tauri/src/daemon_client/replay_flight.rs:51-54` header.
  - `docs/testing-strategy.md:50` · `:179`.
- Historical records (other S under `docs/process/**` · `docs/decisions/*` · `.claude/handoff/*` · `docs/refactoring/*` · `docs/research/*`) are not touched.
- ★The base gate has no comment guard★ — when editing the base `lib.rs` header, using an example path like `crate :: time` makes the gate catch its own header (the header `:66` warning). Keep the current way of quoting the pattern together with its parentheses.
- The coder recounts the old pattern strings starting from the list above and, if it differs from the list, reports before implementing.

## Test plan

qa runs each check exactly as the original `ci.yml` step (the qa-verify 「gates 잡」 extraction). Samples are not put into the target files directly — put one line into a real file of a scratch worktree (`$QA_TMP/wt`) and run the step as is in that worktree.
- The scratch worktree must carry the working-tree changes from before the commit (especially the new `ci.yml`) — apply `git diff` to that worktree and then run. If only HEAD is brought over, the old patterns run and AC2 · AC3 PASS while proving nothing.
- For steps that contain several checks (shell gate 2 · platform gate 6 · net gate 2a), put the sample in a place that does not touch the other checks (e.g. before another column-0 item) and confirm the FAIL message belongs to the target check — shell gate 2 「lint 를 끄는 attribute」 · 「rustfmt 건너뛰기」, platform gate 6 「명단 밖인데 걸린 파일」, net gate 2a 「심볼 꼴이 아닌 모양」.

- AC2 — on this branch's tree, all six checks give the same decision as before the fix.
- AC1 (FAIL when inserted) — choose only what passed with the pre-fix pattern:
  - replay_flight · transport: `use   tokio :: time;` · `let _ = tokio ::spawn(f);` · `tokio	::	x` (tabs) · `let _ = std :: net :: TcpStream::connect(a);` · `let _ = std::net :: TcpStream::connect(a);` · `let _ = engram_dashboard_protocol :: X;`
  - base: `crate :: time::now()` · `super:: sync::X`
  - platform gate 6: `use std :: os :: windows::io::AsRawHandle;` in a `.rs` outside the list
  - shell gate 2: `#[rustfmt :: skip]` · `#[allow(clippy :: restriction)]` in a target file
  - The unspaced form (`tokio::spawn`) must still FAIL.
- AC3 (does not FAIL when inserted): `std::time::Instant` · `std :: time :: Instant` · a trailing comment `// tokio :: spawn` · `my_tokio :: x` · for base, `crate::other_mod`.
- net gate 2a sample (measuring the out-of-scope claim): `use engram_dashboard_platform :: fs::open_deny_write;` → FAIL.
- Also show once that the AC1 samples PASSed with the pre-fix pattern, to record that the gap was real.

## Implementation units

- Coder 1 · unit 1 (the files overlap). Changes = 6 pattern places in `ci.yml` + the explanation copies + a one-line 「알려진 한계」 (at the gate's source-of-truth place) + `docs/todo/doc-drift.md` (close this item) · `docs/todo/observed-only.md` (new item).
- End of unit = `ci.yml` YAML parses OK + the coder runs a few AC1 · AC3 samples once with rg (the final decision is qa's).

## Decisions

- Target = six checks (the review added two: platform gate 6 and shell gate 2). net gate 2a is excluded because it already catches this.
- Whitespace allowed = `\s*` (including tabs). Line breaks are excluded.
- Other name forms = a new to-do + a recorded known limit (not merged into S31 — the gap is wider and also shows up in normal code that passed rustfmt, so it needs a separate decision).
- Branch base = skill-apply (the copies are aligned to the bindings · documents after the cleanup).

## Risks

- False positives — AC2 · AC3 stop them (reviewer's prior measurement: the result of running the widened pattern on the current tree is the same as the old pattern).
- `jobs:` changed, so this commit's CI conclusion differs from the master baseline — decide as the qa binding's 「전체 회귀 = CI」 rule says.

## Review log

- 2026-11-02 · PRD+TRD review full (Claude worker-reviewer + codex xhigh) · PASS in round 2 · after the edition 2 commit, 3 recommendations (line numbers · carrying the changes into the scratch worktree · attributing the FAIL message) applied.
- 2026-11-02 · code review full (Claude worker-reviewer + codex) · PASS (must 0) · recommendation-round re-review PASS. As a result, the place for the new to-do (`observed-only.md`) · the place for the limit record (one gate source-of-truth place) · the group example (multiple items) were aligned in this TRD too — the design is unchanged.
