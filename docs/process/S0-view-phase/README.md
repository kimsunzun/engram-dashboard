# Engram Dashboard — Design Documents

Design and decision records for the Engram agent-management desktop app.
The actual app code is in `apps/engram-dashboard/`.

## Document list

- [architecture.md](architecture.md) — tech stack, component structure
- [requirements.md](requirements.md) — functional requirements

## Key decisions

- **Stack**: Tauri + React + xterm.js + Monaco DiffEditor
- **Code location**: `I:\Engram\apps\engram-dashboard\`
- WezTerm cannot be embedded → render the terminal directly with xterm.js
- Built from scratch, not a fork of the existing CCB (Claude Code Bridge)
