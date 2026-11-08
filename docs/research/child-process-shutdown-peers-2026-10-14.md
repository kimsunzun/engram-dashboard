# How child processes are shut down — peer survey (2026-10-14)

- **Status:** investigation complete · decision = C (`DISABLE_AUTOUPDATER=1` in the env of the claude child that Engram launches — user 2026-10-20) · implemented 2026-10-30 (`NO_AUTOUPDATE_ENV` in `crates/engram-dashboard-agent/src/backend/claude/mod.rs` — both for agent spawn and usage query · if the profile has the same key, that value wins) · ★whether that env actually blocks updates was not measured with a real claude★ (documentation basis — §3 below)
- **Method:** research skill medium — 4 collection tracks (multiplexer source · agent manager source · Windows console/terminal docs · claude updater docs/issues) → main grounding (5 re-reads of local clone `파일:줄` · 1 `gh issue view`) → cross-family adversarial review (result in the section 「적대 리뷰」 below)
- **Confidence legend:** certain = two or more independent sources or direct code check · likely = supported by a single source · uncertain = weak grounds · unknown = not found
- **Trigger:** a user observation that when Engram force-terminates claude, claude's background auto-update is cut off midway and the installation breaks. Questions = 「Engram 이 대응해야 하나, claude 자체 문제인가」 · 「다른 곳은 어떻게 끄나」.

## Conclusion

1. **We found no multiplexer or agent manager on Windows that sends a console event (`CTRL_CLOSE_EVENT` · `ClosePseudoConsole`) first and then waits.** The process stage of the 7 that have a Windows path (herdr · zellij · ttyd · orca · paseo · t3code · vibe-kanban) is a forced kill (`TerminateProcess` · `taskkill /T /F` · Job Object · node-pty `kill()`) (tmux has no Windows path · cline does not manage child PTYs directly). However, **some do put a protocol-level disconnect before the forced process kill** — vibe-kanban sends a stdin JSON `Interrupt` to claude, waits up to 5 seconds, and then kills the group; paseo · t3code do the SDK `close`/stdin close first. — likely (the code re-read at 5 places is certain)
2. **The precedent for closing the console first is on the terminal-app side.** When closing a tab, Windows Terminal releases the pseudoconsole and leaves it to `CTRL_CLOSE_EVENT`, and does not call `TerminateProcess` first. — likely (source summary + the reviewer's independent confirmation)
3. **No peer injects an auto-update off switch into the claude child env** (grep of 5 agent manager clones, 0 hits). — likely
4. **There are several reports that the claude updater broke installations on Windows, unrelated to Engram.** On an npm global install, the executable disappears and new launches fail (#65478 · #90233 — the latter is still open); on a native install, the copy from the versions store → `.local\bin\claude.exe` got corrupted with the same size but a different hash (#86295). The cause is an **assumption** in all three reports, and none states that a forced kill is the cause. — likely (reports exist) · cause uncertain
5. **Engram's shutdown order is the same kind as the peers'.** `child.kill` (=`TerminateProcess`) → `TerminateJobObject` → master drop (`ClosePseudoConsole`) — the same 「강제 먼저, 콘솔 닫기 나중」 (force first, close console later) as zellij · ttyd. The difference is that a Job Object reliably reaps grandchildren too (vibe-kanban also uses a Job Object · orca/paseo get the same effect with `taskkill /T`). — certain (Engram code) · likely on the peer side
6. **So 「터미널은 안전하고 Engram 만 깨뜨린다」 (terminals are safe and only Engram breaks it) has no basis.** An ordinary terminal gives about 5 seconds with `CTRL_CLOSE_EVENT`, but whether the claude updater receives that signal and finishes the replacement is **unknown**. How much Engram's forced kill raises the corruption probability is also **unknown** (not measured).

## 1. Shutdown behavior of Windows OS · terminal apps

| Item | Content | Source | Confidence |
|---|---|---|---|
| `CTRL_CLOSE_EVENT` | Sent to every attached process when the console closes. Timeout = the system terminates after `SPI_GETHUNGAPPTIMEOUT` (default 5000ms). `CTRL_C`·`CTRL_BREAK` have no timeout | learn.microsoft.com/en-us/windows/console/handlerroutine | likely |
| `ClosePseudoConsole` | Sends `CTRL_CLOSE_EVENT` to clients still attached. The docs say it returns immediately from Windows 11 24H2 (26100) and waits until done before that (deadlock if the output pipe is read on the same thread). **Measured on 26200 (S34, portable-pty 0.8.1): it still blocks while conhost holds output nobody reads** — > 20 s with the reader stopped mid-burst, 12–17 ms while the reader kept draining. Keep draining the output pipe until the close returns (`docs/process/S34-pty-kill-hang/`). | learn.microsoft.com/en-us/windows/console/closepseudoconsole · S34 probe | immediate return: contradicted on 26200 (measured) · blocking without a drain: confirmed |
| Windows Terminal | When closing a tab, releases the pseudoconsole handle and leaves it to `CTRL_CLOSE_EVENT`. Does not call `TerminateProcess` | microsoft/terminal `src/cascadia/TerminalConnection/ConptyConnection.cpp` `Close()` (read with a summarizing tool — line level unchecked) | likely |
| VS Code (node-pty) | node-pty Windows `kill()` = on the ConPTY path, destroys the input socket + native kill; on the winpty path, `process.kill` on the console process list. Whether VS Code puts a deferral before that | microsoft/node-pty `src/windowsPtyAgent.ts` | node-pty likely · VS Code unknown |

## 2. Shutdown order per peer

### Multiplexers

| Peer (checkout) | Unix | Windows | Basis |
|---|---|---|---|
| herdr `065ef9d` (v0.9.1) | SIGHUP → SIGTERM → SIGKILL, 250ms per stage (20ms polling), all descendant PIDs | Hangup ignored, Terminate·Kill both `TerminateProcess` per PID (descendants enumerated with Toolhelp) → in effect 「250ms 뒤 트리 강제 종료」 (forced tree kill after 250ms). No per-pane Job Object (only for the status command) | `src/pane.rs:1508-1559` · `src/platform/windows.rs:2197-2210` (confirmed by main re-read) |
| zellij `bf8d23a` | Pane close = SIGHUP once · when the server gets SIGTERM, SIGTERM to the child 3 times → SIGKILL | Pane close = `TerminateProcess` on the direct child once → `ClosePseudoConsole`. No descendants · no Job | `zellij-server/src/os_input_output_windows.rs:370-382` (confirmed by main re-read) · `pty.rs:1801-1828` |
| tmux `5eaf557` (shallow clone) | PTY master close only — SIGHUP is left to the kernel | not applicable | `window.c:1120-1134` |
| ttyd `2922cb8` | SIGHUP to the process group (changeable with `-s`) | `TerminateProcess` regardless of signal → `ClosePseudoConsole` | `src/pty.c:160-164` (confirmed by main re-read) · `src/pty.c:102-110` |

### Agent managers

| Peer (checkout) | Order | Windows | Basis |
|---|---|---|---|
| orca `df7460af` (stale shallow clone) | For an agent, clean up descendants → node-pty `kill()` → after 5 seconds `forceKill` up to 2 times (250ms) | After verifying PID identity, `taskkill /pid X /T /F` (5-second timeout) → node-pty `kill()`. If `kill()` was already issued, skips the force after 5 seconds (comment 「ConPTY 이중 닫기 위험」) | `src/main/daemon/session-termination-controller.ts:55-90,154-173` · `src/main/windows-process-tree-kill.ts:13-28` |
| paseo `bbe3f173` | claude = SDK `input.end()`·`query.close()`·`interrupt()` then tree-kill (2 s + 2 s) · codex = stdin close → SIGTERM → 2 s → SIGKILL → 1 s · shell PTY = on Windows `kill()` → 2 s → `kill()` → 1 s | tree-kill on Windows is `taskkill /T /F` (library not read) | `packages/server/src/utils/tree-kill.ts:33-54` · `.../terminal/terminal.ts:1467-1525` |
| t3code `5378f87f` | claude = SDK `query.close()` (comment: stdin close → SIGTERM → SIGKILL) · codex app-server = SIGTERM → 2 s → SIGKILL · terminal = SIGTERM → 1 s → SIGKILL | No Windows branch (Node semantics — SIGTERM = `TerminateProcess`) | `apps/server/src/terminal/Manager.ts:1616-1660` |
| vibe-kanban `78580443` | stdin JSON `Interrupt` to claude → wait up to 5 seconds → process group kill (Unix SIGINT → 2 s → SIGTERM → 2 s → SIGKILL) | Skips the signal ladder and goes straight to `child.kill()` — command-group Windows group = Job Object (library not read) | `crates/utils/src/process.rs:5-31` (confirmed by main re-read — the ladder is inside `#[cfg(unix)]`) |
| cline `25ef0939` | Does not manage claude·codex directly as child PTYs | — | — |

**Common pattern:** if a structured path exists (SDK · app-server · stdin JSON), they do a protocol-level interrupt/close or a stdin close first, and then go to the forced tree kill. Deferral of 1~5 seconds · wait of 1~2 seconds after the force. No one on Windows tries to end politely with `CTRL_C_EVENT` / `CTRL_CLOSE_EVENT` (zellij's `GenerateConsoleCtrlEvent`, the counterpart of SIGINT, is for relaying the user's Ctrl+C, not for termination).

## 3. claude auto-updater

| Item | Content | Source | Confidence |
|---|---|---|---|
| Behavior | Checks at startup + periodically while running, downloads and installs in the background, applies at the next start. Only native installs are automatic (installs through package managers such as WinGet are off by default) | code.claude.com/docs/en/setup | likely |
| How to turn off | `DISABLE_AUTOUPDATER=1` = turns off only the background check (`claude update`·`claude install` still work) · `DISABLE_UPDATES` = turns off everything including manual | same document | likely |
| Install structure | Windows native = `%USERPROFILE%\.local\bin\claude.exe` + `%USERPROFILE%\.local\share\claude`. macOS/Linux use a versions directory + symlink switch | same document | likely |
| Windows replacement defect (npm) | Rename `claude.exe` → `claude.exe.old.<ts>` and then write the new file (because a running image cannot be overwritten) — if the write fails, a state with no executable is left and new launches fail. #65478 says concurrent sessions die too, but admits that replacement order itself is a 「추정」 (an assumption), and the stale bot closed it as not planned (not a maintainer decision or fix). #90233 (open · `update_apply_exe_locked`) reports a ~240MB `.old` leak per failure + intermittently no `claude.exe` | anthropics/claude-code#65478 · #90233 (checked by main with `gh`) | reports exist = likely · whether even running sessions die = uncertain (reports differ) |
| Windows corruption (native) | After a background update, `%USERPROFILE%\.local\bin\claude.exe` has the same size as the original in the versions store · different SHA256 → immediate `0xC0000005`, no self-repair with `claude update` — so Windows native looks like a **copy**, not a symlink. Cause is an assumption | anthropics/claude-code#86295 (2.1.229 · not planned · checked by main with `gh`) | likely (report) · cause uncertain |
| Other reports | A running binary locks the directory so the staging replacement fails with EBUSY (#67909 — only a mirror summary read) · silent EBUSY failure when the VS Code extension is active (#61676 — search snippet) | each issue | uncertain |
| Concurrent instance lock | `.update.lock` (5 minutes) — only in an unofficial source copy | tangled.org copy | uncertain |
| Shutdown signal handling | Whether the updater finishes the replacement when it receives `CTRL_CLOSE_EVENT` / whether it runs as a separate child | — | unknown |

## 4. Engram's current state (comparison baseline)

- Shutdown = `transport.shutdown()` → `child.kill` (=`TerminateProcess`) + wait → `TerminateJobObject` → master drop (ADR-0001). No graceful stage.
- Even if the daemon dies, `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` ends the whole tree immediately at the OS level (`crates/engram-dashboard-agent/src/platform/windows.rs:156`).
- claude JSON mode already has an interrupt line (`InterruptLine`). The shutdown causal chain (`begin_retire` → `shutdown` → `join_pump` — CLAUDE.md 「핵심 불변식」) has no interrupt stage (not rechecked in code).
- No update-related value is put in the claude child env (repository grep, 0 hits).

## 5. Options (the decision is the user's)

| Option | Effect | Cost · risk | Peer precedent |
|---|---|---|---|
| A. Leave as is | — | A shutdown that overlaps the update window can keep breaking | all 9 force immediately on Windows |
| B. Close the console (`ClosePseudoConsole`) first, then force, with a bounded wait | Same conditions as an ordinary terminal (Windows Terminal) | Shutdown gets a few seconds slower · `ClosePseudoConsole` can block **indefinitely** unless the output pipe is drained at the same time or closed first — the docs say it returns at once from 24H2, but on this machine (26200) it stayed blocked for over 20 s with the output undrained (S34 measurement — the `ClosePseudoConsole` row above) · its return does not mean the client ended → the wait needs a process-exit observation plus a cap timer of its own · **unknown whether the updater finishes its swap on that signal** — effect unverified | None on the manager side · terminal apps (Windows Terminal) take this shape |
| B'. Protocol disconnect (claude JSON-mode interrupt) before the force + bounded wait | Gives room to clean up the in-progress turn | Not applicable to terminal-mode claude · effect on the updater is even more uncertain than B (the updater is unrelated to turns) | vibe-kanban (5 s) · paseo · t3code |
| C. Inject `DISABLE_AUTOUPDATER=1` by default into the claude child env | The background check of the claude that Engram launched disappears | **Other claudes using the same install (ordinary terminals etc.) still update** — the replacement window of the shared install does not go away · a user-behavior change (inside Engram the user updates by hand) · under the 「백엔드 확장」 principle, only inside `backend/claude` | none |
| D. Documentation guidance only (recommend a setting) | Leaves it to the user's choice | Users who do not know keep hitting it | — |

## 6. Follow-up — can we tell that 「업데이트 중」 (an update is in progress) (2026-10-14)

User direction = 「업데이트 중인 것만 알면 종료를 막는다 · 방법이 없으면 Engram 에서 자동 업데이트를 끈다」. 2 collection tracks (official surfaces · the built-in JS strings of installed 2.1.284) → main rechecked the lock code in the string extract.

**Conclusion: there is no documented machine-readable signal. There are unofficial traces, but they differ per install method (npm global · native · Homebrew/WinGet), the lock location splits conditionally even within the same version, and the native lock gives false positives.**

| Surface | 「진행 중」 (in progress) signal | Confidence |
|---|---|---|
| CLI (`claude doctor` · `update` · `install`) | None — doctor gives only the 「마지막 시도 결과」 (result of the last attempt) as human-readable text | likely |
| Settings · env | Control only (`DISABLE_AUTOUPDATER` · `DISABLE_UPDATES` · `autoUpdatesChannel` · `minimumVersion` …) | likely |
| Hooks · OTel · statusline input JSON | No update events or fields | likely |
| SDK / stream-json | Not found (a full comparison of system/init fields was not done) | uncertain |

**Unofficial traces (2.1.284 built-in JS · string extraction · main recheck noted)**

| Install method | Trace | Which process | Confidence |
|---|---|---|---|
| npm global (this PC) | Update lock = a file whose content is `${process.pid}`, treated as stale and deleted and re-acquired once mtime passes 5 minutes (`K=300000`). **The location is conditional**: if some flag (`F()`) is set, `Ae.state("update-lock")` (location unknown), otherwise `<configDir>/.update.lock` (`~/.claude`) | distinguishable by pid | code = certain (main recheck) · which location is actually taken = unknown |
| npm global | After taking the lock (`Xe()` → install function → released in `finally`), it launches an `npm install -g @anthropic-ai/claude-code@<ver>` (or bun) child process. Only on Windows · bundled run · executable under `node_modules/@anthropic-ai/` does it rename the executable to `.old.<ms>` before that (no pid · the install continues even if it fails). Failure code `update_apply_exe_locked` | distinguishable by the child's parent pid | likely (lock → child order confirmed by the reviewer) |
| Native | Per-version lock — default `~/.local/state/claude/locks/<ver>.lock` (follows `XDG_STATE_HOME`), the format splits between pid JSON `{pid, version, execPath, acquiredAt}` and mtime-based. **A running claude also takes its own version's lock if its executable is under the versions directory** → a pid holding another version's lock is not guaranteed to be an updating process (it may be an ordinary session running another version — false positive). There is also a separate temporary lock during installation · staging `~/.cache/claude/staging/<ver>` · launcher replacement `claude.exe.new.<pid>.…` → `claude.exe.old.<ms>.<pid>` | launcher replacement file names are distinguished by pid · the lock alone gives false positives | likely |
| Common | After completion `~/.claude/.last-update-result.json` (not an in-progress signal) | — | likely |

Adversarial review (codex · high · level 2) = **FIX**, 4 applied: the native lock's 「평생 락 → 업데이트 판정」 false positive (high) · lock path·format conditional (med) · `.old` rename condition (med) · narrowing 「공식 신호 없음」 to 「문서화된 기계 판독 신호 없음」, and Homebrew/WinGet are also install methods (low–med). The reviewer too could not settle the condition of `F()`.

**Weaknesses of the detection approach**
- It relies on undocumented minified internals — when the version changes it silently stops matching (failure mode = back to the current state).
- We have to follow both install methods × the lock location branch.
- It cannot block a daemon crash · reboot · Task Manager kill (the Job Object kills the tree).

## Limits · gaps

- We could not confirm the updater's shutdown signal handling (the effect of option B) — a measured reproduction is needed.
- We could not confirm whether VS Code puts a deferral before node-pty `kill()`.
- We could not confirm on herdr Windows whether `ClosePseudoConsole` happens before the 250ms wait · whether all panes are dropped when the herdr server exits.
- The orca · tmux · cline clones are shallow or stale (they may differ from the remote).
- #65478 is a report about an npm global install. The Windows replacement method of a native install is not in official material.
- We found no report dedicated to update contention between concurrent instances.

## 적대 리뷰 (adversarial review)

Cross-family reviewer (codex · effort high · level 2 · web_search) decision = **FIX**. All 6 findings applied.

| # | Finding | Decision | Applied |
|---|---|---|---|
| 1 | 「9곳 전부 첫 단계부터 강제」 overstates the scope — vibe-kanban does stdin `Interrupt` + a 5-second wait, tmux·cline are not Windows targets | accepted (code basis) | Narrowed conclusion 1 to 「콘솔 이벤트 먼저는 못 찾음 · 프로토콜 끊기 먼저는 있음」 · added option B' |
| 2 | Windows Terminal closes the console first, but was missing from the 「선례 없음」 conclusion | accepted | Added conclusion 2 |
| 3 | The replacement order in #65478 is the reporter's assumption, and whether a running session survives differs from #90233 | accepted (#90233 checked with `gh`) | Fixed the §3 table · downgraded confidence |
| 4 | `ClosePseudoConsole` can wait **indefinitely** before 24H2 · returns immediately from 24H2 → 「수 초」 is wrong | accepted (MS docs) | Fixed the cost of option B |
| 5 | Injecting `DISABLE_AUTOUPDATER` turns off only the Engram child and leaves the replacement window of the shared install | accepted (logic) | Fixed the effect·cost of option C |
| 6 | Omission — #90233 (open) · #86295 (native corruption) · #65478 was closed by a bot | accepted (all three checked with `gh` · closer `github-actions[bot]`) | Added to conclusion 4 · the §3 table |

The reviewer reported not finding 「supervisor 아래에서 claude 를 돌릴 때의 공식 종료 보장」 (abstention — not a rebuttal).
