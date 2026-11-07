You are a subagent — this brief is the source of truth. You act as the coder for stage 3-2 unit U4a of the Engram Dashboard repo `I:\Engram\apps\engram-dashboard`, branch `v0.4.1/refactor/transport-attach`. Base = the HEAD you find at start (expected `0d4f79d1` = U3c, or a later docs-only commit). Check `git status` shows no modified tracked source files before you start (untracked handoff attachments are fine); if not, stop and report.

## Scope — U4a only
Main split TRD §6-2 U4 into three sequential units: **U4a = production switch (this brief)** · U4b = delete the old driver + test fate + `tokio-tungstenite` → dev + docs · U4c = D23 ① (token out of `DaemonInfoDto` + `wsTransport.ts`). Do not start U4b/U4c work.

U4a = handover memo §2 items 1, 2, 3:
1. Production constructor `new_real_with_owned_runtime` (`src-tauri/src/daemon_client/mod.rs`, called from `src-tauri/src/lib.rs`) builds the V2 driver (`connection_v2::LinkClient::start`) with the real ws dialer, the real endpoint source, the real clock and the real ports (`TauriEmitter`, `TauriRefusalBox`, the router/registry from `lib.rs`). The `connection_v2` module, the `link` field and the V2 branches stop being `#[cfg(test)]`; production calls go through V2. Keep `new_on_link` (tests) working.
2. Policy injection per TRD §7 + revision 9: request timeout 30 s (D15 ①, ADR-0181) into the policy V2 uses; unlimited reconnects already in `link_policy` (ADR-0346); everything else = transport defaults — check each §7 value against `Policy::default()` (`crates/engram-dashboard-transport/src/policy.rs`) and report any mismatch instead of silently changing transport defaults.
3. Remove `REPLY_TIMEOUT` in `src-tauri/src/commands/agent.rs` (`forward_daemon_command`'s `tokio::time::timeout` wrapper) — the transport `RequestError::TimedOut` gives the same text via `request_error_text`; confirm the text is byte-identical, with a test.

The old driver (`connection.rs`, `lifecycle.rs`, old discovery path) stays in the tree for U4b. It must not be reachable from production after U4a. Keep the build warning-free the way CI requires: check `.github/workflows/ci.yml` (RUSTFLAGS / `-D warnings` / clippy steps). If the now-unused old driver produces warnings that CI would fail on, gate it out of the non-test build the narrowest way (e.g. `#[cfg(test)]` on the old module declarations, since the old test suite still runs until U4b) and say so; do not add blanket `allow(dead_code)`.

## Read first (must)
- `.claude/handoff/attachments/20261106-s21-3-2-u3c-handover-for-u4.md` — whole memo (§1 V2 and its `#[cfg(test)]` seams with file:line, §2 tasks, §3 user-visible differences, §4 traps/test infra, §5 carried items). It is a claim — confirm with code.
- `C:\Users\kimsunzun\.claude\skills\engram-code-conventions\references\conventions.md`.
- `CLAUDE.md` (design principles, modules, language rules).
- TRD `docs/process/S21-crate-boundaries/trd-3-transport.md` — extract only: line 24 (revision 9), the §6-2 U4 row (`grep -n '^| \*\*U4\*\*'`), §7 (policy values). Lines are very long — cut with `grep -o`/`cut`.
- ADRs `docs/decisions/0181-*`, `0195-*`, `0342-*`, `0344-*`…`0347-*`.

## Decisions already closed (do not reopen)
- Unlimited network retries; token/version refusals stop at the first refusal (ADR-0346). Cold start shows Down at once and keeps retrying in the background (TRD rev 9 ①).
- Daemon first-auth window 15 s (ADR-0347, landed).
- Notices (`Routed::Notice`) stay a debug log in U4a — the band / 「다시 연결」 UI is U5 (ADR-0344/0345).
- Hard-coded strings / new logs in English (user: existing Korean logs change later in one batch).

## Tests (completion criteria)
- Production constructor builds V2: a test (or a compile-level check) that the production path no longer touches `lifecycle`/old `connection` (e.g. `rg` evidence + build of the non-test graph without the old driver reachable).
- Request timeout 30 s is what V2's policy carries (test on the policy builder).
- `forward_daemon_command` timeout text unchanged without `REPLY_TIMEOUT` (test with ManualClock / memory link driving a reply that never comes).
- Full shell `lib_unit` green; workspace build green; `cargo fmt -p engram-dashboard --check` 0 diffs; the CI gate scripts that touch the shell (gate 4: no `tauri`/`tauri_*`/`engram_dashboard_agent` path in `daemon_client/link/`; gate 5: transport `test-support` out of the shell release graph) — run them from `ci.yml`.
- GUI live run is QA's job (`/qa full`) — not yours; but list in the report what QA should watch (memo §3 rows, esp. #11 close flicker, #1 cold start, #7 text).

## Rules
- Before coding, count the `#[cfg(test)]` seams and production call sites the memo lists (`rg`); if they differ, stop and report first.
- Long runs via `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-detached.ps1 -Command "<cargo ...>" -WorkDir "I:/Engram/apps/engram-dashboard" -LogFile "<scratch>/<name>.log"`; scratch = `C:\Users\kimsunzun\AppData\Local\Temp\claude\I--Engram-apps-engram-dashboard\31fa411b-c656-4850-8c1d-2128bb9a19dd\scratchpad\u4a\`. Shell tests only as `cargo test -p engram-dashboard --test lib_unit …`. Restore byte-identical `src-tauri/bindings/*.ts` a test run rewrites (memo §4). Write edit scripts with the Write tool.
- Order the edits so the build stands if you stop midway.
- Forbidden: spawning subagents · committing · editing ADRs / TRD / step-log / handoff files.

## Report (also write it to `<scratch>/coder-report.md`)
1. Change summary + file list. 2. Tests: commands, counts, full shell `lib_unit` name diff vs your base (lost 0 · added N, or explain). 3. Self-check (surfaces re-read; doubts raised as they are). 4. Differences from memo / TRD / this brief, with reasons (incl. §7 vs `Policy::default()` mismatches, how the old driver is kept out of production, CI warning policy found). 5. What GUI QA should watch. 6. Most expensive step.
