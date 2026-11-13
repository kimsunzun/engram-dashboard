# ADR-0368: The built-in folder is found on the install-root anchor, and the priming file moves into it now

- 상태: 확정 (2026-11-13, 근거: main decision 2026-11-13 (internal — reported to the user) · TRD S25 step 17)
- 관련: ADR-0092 (the priming path and the install-root anchor) · ADR-0100 (release assembly ships `builtin\`) · ADR-0361 (a missing built-in never blocks) · TRD S25 §2-4 1 · §5 17 · Amends ADR-0327 (decision 5 lookup rule)

## 맥락
ADR-0327 put the built-in assets (the preset 「기본」, its priming instructions and its settings) in one distribution `builtin\` folder and left open how a development build, the tests and `ENGRAM_DATA_DIR` find it (decision 5 · TRD §2-4 1). Step 17 ships the folder in the release zip, so the rule has to exist before it. Until now the priming file lived at `prompts/agent-priming.md`, read by the Noop launch path (`FilePrimingProvider`) relative to `data_dir::find_install_root`, the same anchor the help screen (`prompts/engram-help.md`) uses.

## 결정
1. **The built-in folder = `<install root>\builtin`**, where the install root is `data_dir::find_install_root` — a development build walks up from the exe to the repo root (`.git` or a `[workspace]` `Cargo.toml`), a release with no marker above it uses the exe's folder, where the release zip puts `builtin\` beside the exes. The daemon helper is `data_dir::find_builtin_dir`; the daemon passes its value as `LibraryValues.builtin_dir` when the library is wired in (step 8).
2. **Tests pass a temp folder** as `LibraryValues.builtin_dir`; one test reads the repo's own `builtin\` to pin that the shipped files load with no stand-in.
3. **`ENGRAM_DATA_DIR` does not move it** — the built-in folder is an install resource, not data.
4. **The priming file moves now**: `prompts/agent-priming.md` → `builtin/instructions/priming.md` (text unchanged), so there is one priming source (ADR-0327 decision 2). The Noop path reads it there (`REL_MCP_PRIMARY`); its `ENGRAM_PRIMING_FILE` override stays, and when to withdraw that path stays open (TRD §2-4 2). `prompts/engram-help.md` does not move.
5. **The release manifest is pinned to the code**: `scripts/build-release.ps1` lists the built-in files in `$ExpectedBuiltins` and the folder name in `$BuiltinDirName`; an asset-library test keeps `$ExpectedBuiltins` equal to the built-in file set, a daemon test keeps `$BuiltinDirName` equal to the daemon's folder name.

## 거부한 대안
- **A copy of the priming in both `prompts/` and `builtin/`** until the old path is withdrawn — two sources of the same text drift, and ADR-0327 decision 2 makes `builtin\` the one source.
- **Finding the built-in folder relative to the data folder** (so `ENGRAM_DATA_DIR` moves it) — `ENGRAM_DATA_DIR` is a test-isolation escape hatch, not a deploy knob (`data_dir.rs` header), and the built-ins ship with the exes, not with the user's data.

## 근거
The install-root anchor already serves the priming file and the help screen, measured in both layouts (ADR-0092; `FilePrimingProvider::from_install_root`, `HelpSource`) — one rule for every shipped resource. Step 17 code review full (Claude + codex, 2 rounds) PASS; the manifest guard tests fail when either side gains an entry the other lacks (coder's temporary-break check, five cases).

## 영향 / 불변식
- Every shipped resource the daemon reads resolves on `find_install_root`; a new built-in file needs its `BuiltinId`, its `file_of` arm, the repo file and the `$ExpectedBuiltins` entry — the manifest test fails if the last is missing.
- A release whose `builtin\` is missing or incomplete still launches (ADR-0361 — stand-ins and the `builtin-unavailable` notice); the release script's tripwire is what stops such a zip from shipping.
- A release folder unpacked inside a checkout reads that checkout's `builtin\` (the walk-up finds the repo first) — the same as the priming and help files today.
