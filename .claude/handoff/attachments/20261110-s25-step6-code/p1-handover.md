# P1 → P2 handover (S25 step 6, codex settings-check port) — 2026-11-10

P1 is uncommitted in the working tree of `I:/Engram/apps/engram-dashboard-wt1` (base HEAD `7d1e0caa`). Agent tests: 1758 passed /
20 ignored / 1778 listed; platform 115 passed / 4 ignored / 119 listed.

## What P2 calls (all under `crates/engram-dashboard-agent/src/backend/codex/`)
Children of `codex` (incl. a new `codex/launch.rs`) see private items of `codex/mod.rs`; nothing needed `pub(super)` there.
- `cmdline::config_args(raw: &Value) -> Result<Vec<String>, Reason>` — one preset raw value → `["-c", line, "-c", line, …]`,
  lines exactly as written. Refuses: not a string array (`codex-config-not-a-list`, args `{}` or `{index}`); a line the `-c`
  split rejects — no `=`, empty key, or trimmed text starting with `-` (`codex-config-line-invalid`, `{index, line}`); a line
  `text_is_cli_safe` refuses or with an odd `"` count (`codex-config-unsafe-char`, `{index, line}`). This is also what
  `CODEX_SETTINGS_CHECK.command_line_args` returns, so `LaunchArtifacts.config_args` can take it as is.
- `cmdline::priming_value(text: &str) -> Result<String, PrimingRefusal>` — `developer_instructions='…'` (TOML literal, CR/LF →
  the two chars `\n`). `PrimingRefusal { bad: char }` (field `pub(super)`). Refuses `' " % \`, controls other than CR/LF, and —
  only when the text has no U+0020 — `% & ^ | < >` (unquoted on the line otherwise). U1: the real path must block on `Err`.
- `cmdline::fits(line: &CommandLine<'_>) -> Result<(), Reason>` — upper-bound count vs 8191. Errors:
  `codex-priming-unsafe-char` `{char}` (terminal only) and `command-line-too-long` `{needed, limit}`. Counts: wrapper
  (`PROGRAM_TOKEN_BOUND` 262 + ` /c` + ` codex` + `SHIM_MARGIN` 300), terminal `resume <nil uuid>` always, `--cd <cwd> -s
  workspace-write -a on-request`, extra_args, config_args, the MCP attachment at `MCP_URL_BUDGET`, then the priming (exact
  cost: len + 3 for the value). JSON: `app-server --stdio` + extra + config + MCP bound; `instructions` ignored entirely.
- `cmdline::units(text) -> usize` — UTF-16 length with each `"` and `\` counted twice (also the MCP URL bound measure).
- `settings::{config_lines, split_config_line, reason}` — the `-c` split rule and a `Reason` constructor (`pub(super)`).
- `mod.rs`: `mcp_override_value(url) -> String` (the MCP `-c` value — use it, do not re-format), `MCP_URL_BUDGET = 128`,
  `mcp_attachment` now also refuses a URL over that bound (in `units`) or holding `"` / `& ^ | < >`.
- Noop path unchanged: `developer_instructions_override(text, args_so_far) -> Option<String>` = warn-and-drop wrapper over
  `priming_value` + the old `command_line_cost` budget. Keep it for Noop; the real path uses `priming_value` + `fits`.
- `thread_lock::codex_home_from(Option<OsString>, Option<PathBuf>) -> Option<PathBuf>` is now `pub(super)`.

`CommandLine<'a>` (in `backend/settings_check.rs`): `{ mode: AgentOutputFormat, cwd: &Path, extra_args: &[String],
config_args: &[String] /* flags included, attach order */, instructions: Option<&str> /* terminal priming text */ }`.
New port types there: `SkillFormat {NotChecked | Valid{name, description} | Invalid(Reason)}`, `NativePlaces`,
`NativeSkillRoots`, `SkillFrontMatter`/`SkillMetadata`, `FolderMalformed`, `FolderSkillName`, `FolderLoadKind::SkillRoot`,
`FolderLoadSet.{malformed, skill_names, omitted}`, `SchemaRefetch::PerCliVersion`, `FrontMatterParser::{skill, is_yaml_value}`.
`FolderLoadSet` literals: none outside its file (build with `default()`); the codex terminal set uses `..default()`.

## TRD vs code (check before trusting a TRD anchor)
- Anchors were taken at `1e36176d`; locate by symbol. `codex/mod.rs` headers/comments are mostly Korean; rewrite a paragraph
  in English only if you change it (CLAUDE.md 「언어」).
- `pub(super)` for the codex consts / `command_line_cost` (TRD §8 P1) was not needed and not done (child-module visibility).
- §4-3 says each `\` counts 2, §7-1 says a priming LF counts 2 — P1 counts the priming value exactly (len + 3).
- §4-2b SkillFrontMatter: copied from codex, `metadata` is a plain struct with `#[serde(default)]`, not `Option`.
- Codex parses again after repair only when the repair changed a line (P1 mirrors this).
- Premise 17 settled from source: `root/a/b/c/d/e/f/SKILL.md` (6 folders below the root) loads, one more does not; a
  `SKILL.md` directly in a root loads; hidden folders below the root are pruned; file symlinks are skipped by codex.
- `raw_to_json` mirrors the structured-feature merge (`features.{code_mode,multi_agent_v2,network_proxy,sleep_tool}` and
  `profiles.<p>.features.<x>`); other `merge.rs` path rules are not mirrored.
- Pre-existing, not fixed: duplicated sentence in the Korean `mcp_attachment` doc; pass-through `extra_args` and `cwd` are
  not metachar-guarded (R10 / P2 scope per main).

## Codex source copy — `scratchpad/codexsrc/` (fetched from raw.githubusercontent at tag `rust-v0.163.0`)
`config_override.rs` (`-c` split/TOML/fallback), `overrides.rs` + `merge.rs` + `key_aliases.rs` (path application,
structured features), `parser.rs` + `parser_tests.rs` (SKILL.md split/repair/check), `host_roots.rs` (native roots),
`discovery.rs` + `host.rs` + `mod.rs` + `metadata.rs` (skill walk, `MAX_SCAN_DEPTH = 6`, default name = folder `to_str` else
"skill"), `local_file_system.rs` (exec-server `sync_walk` depth rule), `project_root_markers.rs` (`[".git"]`), `tree.json`
(full repo tree at the tag — find more files there). Project convention: borrow behaviour, do not paste code; if you must
port, write your own structure and add `// Behaviour mirrors <path> @ rust-v0.163.0`.

## Measurements (in `docs/research/codex-launch-materials-measurements-2026-11-09.md`, section M10)
- cmd applies 8191 again to the expanded `codex.cmd` line (E = L + 148 on this PC); `SHIM_MARGIN` stays 300.
- `-c a=1 -c a.b=2` → `{a:{b:2}}`; reverse → `{a:1}`. Method: `config/read {includeLayers:true}` through `cmd.exe /c codex
  app-server --stdio`, temp `CODEX_HOME` (no tokens). Scripts: `scratchpad/shim_probe.py`, `scratchpad/config_read_probe.js`.

## Tooling pitfalls
- The Bash tool folds `\\` to `\` inside heredocs/arguments (single escapes like `\n`, `\x0b` survive). Put code containing
  `\\` in a file via the Write tool, or fix afterwards with Edit; a Python splice from a Write-tool file is safe.
- Repo files are CRLF in the working copy but some got LF after edits; rustfmt (`--edition 2021`) normalizes. Python string
  replaces must match the file's current line ending. All touched files were rustfmt-clean at HEAD — run rustfmt on them.
- `cargo build`/`test` `os error 5` = a running app/daemon holds an exe: do not kill it, use `--target-dir` in the scratchpad.
- Times here: full agent tests ~2–3 min incl. build; lib tests ~15 s; `cmdline::the_count_bounds_the_std_quoting` spawns
  PowerShell (~7 s, `#[cfg(windows)]`). Use `-- --test-threads=4`; never bare `cargo test` at the repo root.
- Baseline test names first: `cargo test -p <pkg> -- --list | grep ': test' | sort`, then `comm` after.

## Failed / rejected approaches
- `powershell -EncodedCommand <b64> <extra args>` rejects trailing args — use `-File <script.ps1> <args>` for a probe child.
- Text-only repair (quote every `[`/`{` value) diverged from codex — replaced by the `is_yaml_value` parser seam (review r1).
- Do not resolve `cmd.exe` exactly (decision #17): the program token is a fixed 262 bound.
