# S9 spike measurement results (the claude session ID mechanism)

Measured on 2026-06-11, with `claude -p` (non-interactive) + observing `~/.claude/projects/` files. Isolated cwd `C:/temp/claude-spike`.
**※ This data is based on `-p` non-interactive. TUI (interactive)-specific behavior is unconfirmed (below).**

## Folder/file structure principle

```
~/.claude/projects/<cwd-치환>/<sessionId>.jsonl
```
- **Directory = the cwd path with characters substituted** (`/`·`:`·`\` → `-`). Not a hash. Reversible, but **case- and slash-sensitive** (`f--…` and `F--…` were confirmed to coexist) → if the same folder is written differently, the session is lost. **Canonicalize cwd before spawn is mandatory.**
- **File name = session ID (uuid).jsonl**, matching the `sessionId` on the first line.
- **On resume, no new file is created; it appends to the existing jsonl.**

## Behavior measurements (all reproduced)

| Test | Command | Result |
|--------|------|------|
| Specify ID | `claude -p ... --session-id <U>` | exit 0, json `session_id`=U. **We control the ID** |
| **Does resume fork** | `--resume <U>` | exit 0, **session_id=U kept**, no new file (append). **Does not fork** |
| Reuse ID | `--session-id <기존U>` | **exit 1** `Session ID … is already in use.` |
| Resume a nonexistent session | `--resume <랜덤>` | **exit 1** `No conversation found with session ID: …` |

## Grounds for the design decision

1. **`--resume` does not fork** → passes spike #4, which fable called "decides whether the design holds". **A retreat line (fresh-new-id) is not needed.**
2. **Full control of the session ID** (`--session-id` specified + json returned) → zero non-determinism.
3. **Session change detection = file watch** (file name = session ID, a new jsonl is created) → the path to active sync.
4. **fallback = a new uuid** (reusing an existing ID is confirmed impossible by exit 1 → as designed).
5. **The restore failure signal is clear** (exit 1 + stderr, not a hang).

## TUI (interactive) measurement results — wezterm pane + file observation

Spawn `claude --session-id <U2>` as a TUI, operate it with send-text, and observe `~/.claude/projects/`.

| Item | Result |
|------|------|
| `--session-id` in the TUI | ✓ Same as `-p` — the `<U2>.jsonl` we specified is created |
| **`/clear`** | **Creates a new session ID!** The process is unchanged, but a new `<ae718820>.jsonl` is added next to `U2.jsonl`, and later conversation goes to the new file. **→ Decisive evidence that AgentId/session_id separation is required** |
| `/clear` file detection | The new `.jsonl` is **created immediately** → **active detection by file watch is possible** (the user's idea is confirmed feasible) |
| Trust prompt for a new cwd | **Did not appear in this environment** (already trusted, "Welcome back"). But it may appear on a completely new machine/account — mind this in operation |

### ★Key finding — `~/.claude/sessions/<PID>.json` (resolves the lookup problem)★
```json
{"pid":45404,"sessionId":"<현재세션>","cwd":"...","name":"...","status":"idle|busy","kind":"interactive","startedAt":...,"updatedAt":...}
```
- **File name = PID, with the current sessionId + status + name + cwd inside.**
- **Reading this file by the child PID we spawned gives the current sessionId deterministically** → no need to guess the cwd directory mapping (the lookup problem is solved at the root).
- **On `/clear`, the sessionId in the same PID file was confirmed to update in real time** (561e5d0b → 1e91e1e2). updatedAt also updates.
- **Active sync = watch only the single file `sessions/<child_pid>.json`** (watching the whole cwd directory + the mapping ambiguity are all unnecessary).
- Bonus: `status` (idle/busy/shell) — can also be used as a supplementary signal for agent status.

**Caution (keep best-effort):** It looks like an unofficial internal file (a version field exists → the format may vary by version). Risk of being stale when a PID is reused → verify with `startedAt`/`updatedAt`. Whether the file is cleaned up when the process exits is unconfirmed. → Use it as a deterministic source but keep the fallback (separation + new uuid).

### Remaining minor items (low risk)
- **fresh PTY redraw**: our dashboard manages cols/rows (the resize command), so this is our responsibility area. claude renders at the size it is given — not a big risk.
- **`/resume` picker**: since the mechanism "session change inside the TUI → new file" was confirmed with `/clear`, `/resume` is of the same kind. The file watch covers it.

### Release notes caution (observation)
"Fixed sessions not saving transcripts (...) when launched from VS Code integrated terminal" — a bug where sessions were not saved in PTY/integrated-terminal environments has been **fixed**. We also spawn a PTY, so **keeping the claude version up to date** is the safety net (risk of sessions not being saved on an old version).

## Cleanup needed
- Temporary data: `C:/temp/claude-spike` + `~/.claude/projects/C--temp-claude-spike` (for measurement, to be deleted)
