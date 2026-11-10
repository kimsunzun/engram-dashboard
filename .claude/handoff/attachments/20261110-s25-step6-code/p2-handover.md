# P2 → P3 handover (S25 step 6, codex launch translation) — 2026-11-10

P1 + P2 sit uncommitted in `I:/Engram/apps/engram-dashboard-wt1` (base HEAD `7d1e0caa`). Agent tests after P2:
1780 passed / 20 ignored / 1800 listed. Paths below are under `crates/engram-dashboard-agent/src/`. Locate by symbol.

## What P2 hands P3 — `LaunchArtifacts` (launch.rs), filled by `backend/codex/launch.rs::materialize`
- Noop kit ⇒ `LaunchArtifacts::default()` (all four fields empty/false) — P3's Noop path must not read them.
- `config_args: Vec<String>` — `-c`, `line`, … in plan order, both modes. Already placed by `build_spec`; P3 ignores.
- `instructions_override: Option<String>` — terminal only; JSON always `None`.
- `instructions_text: Option<String>` — JSON only: merged file text as read (unchanged, not trimmed); `None` = no
  file or blank. Terminal always `None`. A read/UTF-8 failure already failed the spawn (`Err`), never reaches P3.
- `instructions_changed: bool` — JSON only: `text.is_some() && kit.instructions_hash.is_some() && kit.instructions_hash
  != kit.last_applied_instructions_hash`. ★Never true without text★ (review round 1) — so "context = preface + text"
  always has a body. It is NOT mode-aware: a Fresh spawn can carry `true` — TRD B10 says Fresh ⇒ no context, so P3
  must gate on `resume_session_id`, not on this flag alone.
- `skill_roots: Vec<PathBuf>` — JSON only: `[<vol>/engram/skills]` (only with ≥ 1 skill) then `plan.extra_skill_roots`
  in plan order. All absolute + valid UTF-8 (checked) ⇒ `to_str().unwrap()` is safe, but prefer a fallible map.
  Empty ⇒ P3 sends no `skills/extraRoots/set` (Noop and empty real kits stay byte-identical on the handshake).
- `real: bool` — `kit.plan.real`. `args`, `instructions_file`, `settings_file`, `add_dirs`, `env` stay empty for codex.
- Layout constants are shared: `launch.rs` `SKILL_LINKS_PARENT` ("engram") / `SKILL_LINKS_DIR` ("skills").
- `LaunchKit.last_applied_instructions_hash` is filled by `manager.rs` right after the post-`prepare` re-read
  (`// ADR-0337` line). Step 8 (not P3) records the hash at the first-turn event.

## Where the JSON path sits now (`backend/codex/mod.rs`)
- `thread_open(spec, resume_session_id, developer_instructions)` (~:138): resume branch puts NO
  `developerInstructions`; its Korean doc (~:130-135) and `ThreadResumeParams` doc in `protocol.rs` claim that is the
  decision — both become false in P3 (M8 fix) and must be rewritten (English, whole paragraph — CLAUDE.md 언어).
- `app_server_thread_open(spec, resume_session_id, control)` (~:163) = `thread_open(…, priming_text(control))`. There
  is no fn named `app_server_open` (TRD name); this is the one. Tests pin it (Noop regression).
- `open_spawn(…, _artifacts: &LaunchArtifacts)` (~:1130) still ignores artifacts entirely: JSON real spawns today send
  the control-channel priming, no roots, no context. P3: real ⇒ `artifacts.instructions_text` (do not read
  `priming_file`); Noop ⇒ `priming_text(control)` as today.
- `priming_text(control)` (~:343) — the Noop reader (fail-open, warns). Keep it for Noop only.
- `build_spec` real branch is done (P2): preset `-c` after pass-through, MCP, terminal priming from artifacts.

## Comments P3 must make true (or main edits — ask)
- `build_spec` priming paragraph (English, "Where the value comes from depends on the materials") says real ⇒ the
  control channel's priming file is not read. True for the argv; for JSON it becomes true only when `open_spawn` stops
  calling `priming_text` on real spawns.
- `launch.rs` field docs are written in P3 tense: `instructions_text` "Sent as `developerInstructions` on
  `thread/start` and on every `thread/resume`"; `instructions_changed` "a resume then tells the conversation…";
  `skill_roots` (codex JSON roots). Make the code match, or report.
- `open_spawn` doc (Korean, "`control` 도 … app-server 갈래에서만 읽는다 — 프라이밍 내용 하나") — rewrite when you
  change what it reads. Several stale Noop-only Korean comments in `build_spec` (`%VAR%` block, env-block
  `priming_file` note) are main's to edit — leave them.

## TRD-vs-code differences met in P2
- TRD literal-site lists over-count: most "`LaunchKit` literals" are helper fns via `NoopLaunchKits.prepare`. Count
  with `rg "LaunchKit \{"` / `rg "LaunchArtifacts \{"` yourself; `..Default` sites need no edit.
- `app_server_open` (TRD §8 P3) = `app_server_thread_open` in code.
- Real terminal path also refuses a cmd-unsafe cwd (main's addition, not in TRD); no check-side counterpart (step 7).
- Notice wording fallbacks (absent/empty `paths`, non-object args ⇒ generic) are main's decisions, not TRD text.

## Test helpers worth reusing
- `backend/codex/launch.rs` tests: `root()` (fixed absolute cmd-safe fake root — never use `temp_dir()` as a cwd
  that reaches the real terminal guard), `vol()`, `work()`, `library()`, `noop_kit()`, `empty_real_kit()`,
  `real_kit()` (2 skills, 2 extra roots, 1 folder, 2 raw settings, instructions file, hash `k`), `Reader`
  (fake read seam, counts calls), `materialized(mode, kit)` / `materialized_with(mode, kit, cwd, reader)`,
  `endpoint(priming_file)`, `real_spec(...)`. They are private to that test module — copy or lift `pub(super)`.
- `launch.rs` `MemLaunchFs` (cfg(test), `paths()`, `link_target()`), `place_in_volatile` (confinement).
- `manager.rs` tests: `HashedKits(PathBuf)` (real codex kit, hash `k`, real instructions file) +
  `the_stored_instructions_hash_reaches_the_codex_translation` — spec_probe pattern: set `manager.spec_probe`,
  `activate_profile`, `try_recv()` gives `(CommandSpec, LaunchArtifacts)`; it stops before `open_spawn`, so it cannot
  observe the handshake — use the fake app-server for that.
- `scripted_manager()`, `agent_profile(cwd, name)` in `manager.rs` tests.

## Tooling pitfalls and times
- Python/heredoc string edits fold `\\n` into a real newline (hit in P2) — put `\` literals in via Edit/Write.
- Files are LF in the working copy; git warns "LF will be replaced by CRLF" — harmless. Run
  `rustfmt --edition 2021 <file>` only on files you touched; rustfmt follows `mod` into child files (check
  `git status` after).
- Full `cargo test -p engram-dashboard-agent -- --test-threads=4` ≈ 2 min incl. build; lib only ≈ 15–20 s;
  workspace build ≈ 30 s incremental. `os error 5` ⇒ use `--target-dir` in the scratchpad, never kill the app.
- Baseline names first: `cargo test -p engram-dashboard-agent -- --list | grep ': test' | sort`.
- Pre-existing dead-code warning `backend/claude/mod.rs:2667 env_value` (not P1/P2).
