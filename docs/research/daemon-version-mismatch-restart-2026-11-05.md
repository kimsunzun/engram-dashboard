# What the user does about an old-version daemon after an update

- Status: done (2026-11-05). Decision → ADR-0343. Tier: medium **(single-family — cross-family review skipped by user agreement 2026-11-05: 「문구 하나 조사하는데 거창한거 아님?」; findings are not cross-checked)**.
- Method: two collectors (industry practice — web, snippets only; codebase — source read with file:line), main grounding spot-check of the load-bearing code claims (`src-tauri/src/discovery/mod.rs:465-490`, `crates/engram-dashboard-protocol/src/handshake.rs:27-30`, `crates/engram-dashboard-net/src/ws.rs:336-362`, `src/api/daemonControl.ts:109` — all confirmed).
- Starts from: `docs/research/daemon-version-mismatch-2026-08-14.md` (→ ADR-0158) and ADR-0178 / 0180 / 0342.
- Raw collector output (not kept in the repo): session scratchpad `version-mismatch-collect-1.md` / `-2.md`.

## Question

The app is updated (portable zip extracted over the old folder); the old daemon keeps running with an older protocol version. The shell sees the mismatch in `daemon.json` and does not connect; the tray 「데몬 끄기」 goes through the normal handshake and returns `NoTarget` (`src-tauri/src/daemon_client/stop.rs:89`), so it does nothing. What should the app tell the user, and what should it let them do? The user rejected both "kill it in Task Manager" text and "tray force kill" (2026-11-05: 「둘다 별로」).

## Findings

**F1 — Industry: whoever replaces the binary restarts the daemon** (likely — snippets only, originals not confirmed). Installers/updaters/package managers stop and restart the background service; held work is not preserved by default (Docker Desktop stops containers on update; Docker Engine keeps them only with opt-in live-restore). Version-keyed side-by-side servers (VS Code Remote, JetBrains Gateway) leave old ones running until the user stops them (JetBrains offers a "Stop IDE Backend" action). Version-agnostic control paths are kept separate from the versioned protocol (Syncthing REST shutdown, OS service control, PID kill). No product found that asks "stop the old daemon?" for a daemon holding live user work — this is an absence claim from a small search budget, not established.

**F2 — Here nothing restarts the daemon at update time** (certain — code). Updates are a portable zip, no installer/updater; the only stop is the tray 「완전 종료」 before extracting (`quit_app` → `send_stop`).

**F3 — A graceful stop of every shipped old daemon is possible today** (certain — code). The daemon checks the token, then requires `protocol_version == expected` (`net/src/ws.rs:336-362`, same order since v0.1.0); `daemon.json` holds both the token and the daemon's own protocol version. A stop client that authenticates with the daemon's advertised version passes, and `StopDaemon {force, kill_agents, request_id}` has the same shape in every shipped tag (v3–v7); every shipped daemon handles it with `shutdown_all`. It breaks the stated rule at `protocol/src/handshake.rs:27-30` ("send your own compiled version — echoing daemon.json kills the version gate"); that rule protects normal sessions, and a stop-only connection that sends nothing but `StopDaemon` does not need the gate. Needs the same token as any client (same OS user) — no new privilege.

**F4 — An identity-checked force kill also exists, unused** (certain — code). `discovery::daemon_stop` (`src-tauri/src/discovery/mod.rs:465-490`) checks pid + start time, then `taskkill /F /T`; only caller `DaemonControl.stop()` (`src/api/daemonControl.ts:109`), no UI calls it. Check and kill are separate steps (pid reuse window); a handle-pinned `terminate_if_same` exists only in test code.

**F5 — What a restart costs** (certain — code). Graceful or forced, every agent of that daemon stops (`StopDaemon` ignores `kill_agents`). Boot auto-resume is off; the roster and session ids persist (`agents.json`), so re-activating an agent resumes it with `claude --resume`. The running turn, in-flight output and in-memory mail/parking state are lost. The 「복원할까요?」 prompt restores window layout only.

**F6 — Decision history.** ADR-0158 decision 3 bans automatic kills only; a kill the user presses is not banned (ADR-0178, ADR-0180). ADR-0180 rejected a 「데몬 강제 종료」 button for four reasons: (a) a graceful stop cannot get through, (b) all agents die, (c) the app cannot list what is running, (d) the case is rare. F3 overturns (a). (b) is unchanged but every alternative also ends the agents, and they are resumable (F5). (c) holds — the warning text must say "agents of the old daemon stop and can be resumed" without a list. ADR-0178 left "add the button or not" open.

## Options (constraint fit)

| Option | Works on shipped daemons | Agents | Conflicts | Effort |
|---|---|---|---|---|
| A. Text: kill it in Task Manager | yes | all stop (forced, no `shutdown_all`) | none | text only — **user rejected** |
| B. Tray force kill after consent | yes (F4) | all stop (forced) | ADR-0180 rejected alt | small — **user rejected** |
| **C. 「데몬 재시작」 action: graceful stop (F3), fallback identity-checked kill, then start the new daemon** | yes | all stop gracefully; resumable | amends ADR-0180 rejected alt (premise (a) overturned), ADR-0178 open item, handshake rule exception | shell stop client + one button + LLM command |
| D. Side-by-side generations (new daemon in a new slot) | n/a | old keep running | none, but roster/lock are per data folder → second daemon starts empty | large redesign (long-term, ADR-0158) |
| E. Explain only | — | — | none | text only — leaves the user stuck |

## Recommendation (main inference — not cross-checked)

C. It matches F1 (the side that replaces the binary restarts the daemon) while keeping ADR-0158's consent rule (the user presses it), and it is the only option that both works against shipped daemons and stops them through their own `shutdown_all`. Fallback to the identity-checked kill only if the graceful stop times out, using the handle-pinned check. For future daemons, add a version-independent stop frame decoded before the version check so the echo exception is not needed forever (follow-up).
