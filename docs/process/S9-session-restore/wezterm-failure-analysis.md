# Analysis of the wezterm session restore failure — premises for the dashboard design

A prior analysis laid down **before** designing session save/restore. It analyzes from the code why the existing wezterm-based restore system (`~/.config/wezterm/`) was unstable, and derives defensive principles so that the dashboard avoids the same traps.
**Conclusion first: claude `--resume` session restore cannot be trusted 100%. Store our metadata independently, and treat claude restore as best-effort.**

## 1. The existing wezterm restore system (the field)

| File | Role |
|------|------|
| `layout_capture.lua` | Saves the layout with Ctrl+Shift+S. Detects claude panes with `is_claude_pane` |
| `sid-map.json` | Mapping from `cwd:win:tab:pane` → claude session uuid |
| `workspaces.lua` | Restores with `claude --resume <sid>` (powershell). `OUR_SID` hardcoded |
| `split-debug.log` / `save-debug.log` | 5-minute periodic dump logs |

Restore method: `powershell -NoExit -Command "Set-Location <cwd>; claude --resume <sid>"` — the sid is looked up from sid-map by the position key.

## 2. Failure mechanisms (why it cannot be trusted)

1. **Position-based session mapping** — the `sid-map.json` key is `cwd:win:tab:pane` (indexes). When a pane is **added/moved/deleted, the indexes shift** and the wrong session is restored. (This is the actual cause of the symptom "the ID changed when I moved a pane")
2. **Process-name-based claude detection** — `is_claude_pane` decides by the foreground process name `node`/`claude`. Because claude is node-based, it **falsely detects other node processes (a vite dev server, etc.) as claude**.
3. **Non-determinism of the session ID** — claude generates the sid and we capture it, so it depends on capture timing/accuracy. The hardcoded `OUR_SID` is a trace of not being able to stand this non-determinism and pinning a particular session in the code.
4. **Fragility of claude --resume itself** — it resumes the TUI in a new PTY → depends on redraw and cols/rows, and behavior is unclear when the session is damaged.

## 3. Dashboard defensive design principles (derived)

1. **Session identification = a stable unique ID (AgentId/uuid), independent of position.** Do not bind a session to a slot index. (Avoids the position-based mapping trap)
2. **claude process tracking = we spawn it → hold the child PID/AgentId directly.** Never depend on process-name (node) detection. (Avoids false detection)
3. **We control the claude session ID** — if we can specify the session ID at spawn (`--session-id` etc., check with claude-code-guide), do so to remove the non-determinism. If not possible, capture it deterministically once right after spawn.
4. **Store layout and session separately.** Store the slot layout (frontend) and the agent profile/session (backend) separately. wezterm tied the two by position and broke.
5. **Graceful restore failure.** Even if `claude --resume` fails (session missing/damaged), **fall back to fresh**, and our metadata (profile/AgentId) is preserved independently and does not break. claude restore is a "nice if it works" best-effort.

## 4. Key questions to pass to design/review

- Does the claude CLI support **specifying the session ID at spawn** (`--session-id`)? Is deterministic resume possible?
- What are the TUI/rendering problems when `--resume <id>` is **spawned non-interactively in a new PTY**?
- What does `--resume` do when the session is damaged/absent (error? empty session? hang?) — what should trigger the fallback?

→ Pass this analysis + the premise "cannot be trusted 100%" identically to the claude-code-guide survey / web (Gemini · GPT) / fable review.
