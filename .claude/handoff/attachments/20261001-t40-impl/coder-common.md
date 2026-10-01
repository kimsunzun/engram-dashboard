# T-40 step ① — common coder brief (shared by chunks A–D)

You are a coder subagent. Implement ONLY the chunk named in your spawn prompt. Work in `I:\Engram\apps\engram-dashboard-wt1` (a git worktree; do not cd elsewhere).

## Source of truth
- TRD: `docs/process/S21-chat-ux/trd-t40.md` (Korean). Read §0, §2 (file table), §3-5 (OS pieces), §5 (tests), §6 (order + 접점 signatures) fully. Read other sections only as needed by your chunk.
- The interface shapes in TRD §6 「접점」 are FIXED by the orchestrator. Do not change names/signatures. If one is impossible as written, stop and report why instead of inventing another.
- "HEAD" line numbers in the TRD refer to commit `9fa9215`; the current HEAD has only docs on top, but earlier chunks of this step may have moved lines. Locate by symbol, not line.
- Backup of an abandoned design: `refs/backup/t40-clock-design-20260930` (read with `git show <ref>:<path>`). Step ① does NOT import from it except where the TRD says.

## Hard rules
- Each sub-step must compile on its own; order your edits so that stopping anywhere leaves a building tree (remove callers/tests in the same edit as the thing they use).
- Behaviour visible to users must not change in step ① (wiring is step ③).
- Keep project invariants (CLAUDE.md 「핵심 불변식」): lock order, ownership split, kill causality (ADR-0001), core isolation (no `tauri` in agent), platform neutrality (OS branching only inside `platform/*`, no `cfg!(windows)` sprinkled at callers; non-Windows stubs as TRD §3-5 says).
- Comments: Read and follow `C:\Users\kimsunzun\.claude\skills\code-conventions\references\comments.md` and `.claude/skill-bindings/code-conventions.md`. Comment language follows the surrounding file (Korean). Add `// ADR-NNNN` anchors only where the TRD names them; the new ADR number does not exist yet — do not invent one.
- Line endings: preserve each file's existing endings. `transport/stdio.rs` is CRLF; others in `platform/` and `input_queue.rs` are LF. `cargo fmt` may convert CRLF→LF; if it does, restore CRLF (e.g. `unix2dos` or a short script) and verify with `file`.
- Dependencies: only the two `windows` features the TRD names (`Win32_System_IO`, `Wdk_System_Threading`) may be added, and only in chunk B. No new crates; `Cargo.lock` must not change otherwise.

## Tooling constraints (important)
- A Bash pre-hook BLOCKS any Bash command whose text contains Win32 API names (e.g. OpenProcess, TerminateProcess, CreateIoCompletionPort …). Use the Grep/Read/Edit/Write tools for code search and edits; never put API names into Bash command text. Run scripts by path.
- ALL cargo commands go through the detached runner, never directly:
  `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-detached.ps1 -Command "<cargo …>" -WorkDir "I:\Engram\apps\engram-dashboard-wt1" -LogFile "<abs path>.log"`
  It returns immediately. Completion = the log's last line `__EXIT=<code>`; poll for that marker (e.g. `tail -n 3 <log>` in a loop with short waits via `powershell Start-Sleep`), never judge by process disappearance. Read only the needed lines of the log (errors, `test result:` lines), not the whole file.
  Put logs under `C:\Users\kimsunzun\AppData\Local\Temp\t40-impl\` (create it).
- Test commands: `cargo test -p engram-dashboard-agent --no-fail-fast -- --test-threads=4` (keep both flags; `submit_delivery_boundary` PTY timing test is known flaky under load — rerun it alone before calling it red). To confirm the whole workspace still compiles incl. tests: `cargo test --workspace --no-run`. `cargo fmt --check` at the end (run `cargo fmt` if needed, then restore CRLF on stdio.rs).
- Do not run more than one cargo job at a time.
- `src-tauri/bindings/*.ts` show pre-existing EOL noise in `git status` — never touch or stage them.

## Forbidden
- Spawning subagents. Committing, stashing, resetting, or any git write (the orchestrator snapshots). Editing task lists. Editing docs outside code comments (ADR/CLAUDE.md/step-log are step ④).

## TDD
Write/adjust tests with the change (TRD §5 names them per chunk). Real-process tests: `#[cfg(windows)]` per test, ≤4 processes each (console host counts), reuse the existing helper noted in TRD §5. Git-Bash-dependent ones are `#[ignore]` as the TRD says.

## Before returning — adversarial self-check
Re-read your own diff (`git diff`) with a hostile eye: handle leaks, a kill path that could hit the wrong process, lock taken while calling out, panics crossing threads, non-Windows stubs, tests that pass vacuously. Report what you scanned and anything suspicious — do not resolve doubts silently.

## Return format (concise; no tool-output dumps)
1. Change summary per sub-step. 2. Files touched. 3. Commands run + result lines (`test result:` counts, `__EXIT`). 4. Self-check: surfaces scanned + findings. 5. Deviations from the TRD (if any) with reason. 6. Anything the next chunk must know (moved symbols, leftover TODOs). 7. One line: the most expensive part of this task (for process improvement), only if notable.

## TRD reading budget
The TRD is ~42k tokens. Read only the sections your spawn prompt names (use `grep -n "^#" <TRD>` then Read with offset/limit). Do not page through the whole file.

## CRLF files
Never edit CRLF files with `sed -i` under Git Bash (it silently converts the whole file to LF). Use the Edit tool, or a binary-mode script, and verify with `file` + `git diff --stat`.
