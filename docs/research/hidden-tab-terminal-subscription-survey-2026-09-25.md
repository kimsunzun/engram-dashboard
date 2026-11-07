# Output subscription and size-reporting policy for hidden-tab terminals — industry survey

- 상태: **확정 — user decision 2026-09-25: the PC keeps keep-alive (ADR-0056); the hidden-tab size problem is fixed with a guard 「잴 수 없는 크기는 보내지 않는다」 (never send a size that cannot be measured); mobile power saving (unsubscribe when hidden) is designed when the mobile client is built** · Date: 2026-09-25
- Method: medium (design-decision mode) · 2 collectors (lead, light) in parallel — A hidden-tab subscription policy · B size reporting while hidden · local clones (`I:\Engram_Workspace\opensource\`) first · main grounding (source-text cross-check) · one cross-family adversarial review (FIX — incorporated below).
- Confidence: certain (independently cross-confirmed) · likely (single primary source) · uncertain (unchecked · knowledge-based).
- Trigger: a hidden-tab terminal sends the PTY size right after subscribing without having been able to measure its size (`src/components/slot/TerminalSlot.tsx:304-305` — the other three send paths `:99-101` · `:285` · `:355` are skipped when hidden). The user's proposal 「숨은 탭은 구독하지 않다가 전환 때 구독」 (do not subscribe in a hidden tab, subscribe when switching to it) was checked against industry rules.
- Prior research: ADR-0056 surveyed the xterm.js family (VS Code · Tabby · Hyper · Theia) and chose keep-alive. This time server-style multiplexers · agent managers and size reporting were added.

## Conclusion

1. **No one in the surveyed scope stops parsing the output of a hidden terminal.** What differs is 「늘 먹이는 에뮬레이터가 어디 사나」 (where the always-fed emulator lives). (likely)
   - **View keep-alive** — VS Code · Tabby · Hyper · Theia (ADR-0056) · vibe-kanban · t3code (up to a cap of 10 threads) · paseo (retained tabs). A hidden view keeps being written to and only stops drawing. **Our approach (ADR-0056).**
   - **Server VT + blocking delivery to hidden views** — tmux · zellij · herdr · orca (Phase 4 「hidden-delivery gate」). The server holds the VT, so when it becomes visible it **redraws the current screen from the VT state** (orca snapshots the scrollback too). orca moved from keep-alive to this because of renderer load.
2. **「숨으면 끊고, 전환 때 원시 바이트를 되감아 따라잡기」 (disconnect when hidden, rewind raw bytes on switching to catch up) is rare, and the one confirmed case reverted it.** (likely)
   - **CARB/IDE** took down the hidden-tab renderer and rewound the whole scrollback on every switch, but xterm answered the terminal queries inside the rewound output (`CSI 6n` cursor position · `OSC 11;?` background color) again, so **those answers leaked into the shell input** (#87), and `xterm.reset()` on switching lost the buffer → it changed to 「열린 탭마다 렌더러 하나 + fit 은 0×0 가드」 (ADR-014).
   - **Codeman** receives and rewinds a 1 MB tail on switching to a background tab — it has tmux underneath, and that truncation was raised as the problem 「출력이 사라진 것처럼 보인다」 (#258).
   - The remaining rewinds appear only on cold paths (first open · reopening after being taken down past the cap · restart restore) (t3code · paseo first attach · orca cold restore).
3. **Size: a size that cannot be measured (0 because hidden, or unmeasurable) is not sent, and the last size is kept — common to the xterm-based peers surveyed.** (certain — VS Code · paseo · orca · CARB source texts)
   - It is not 「숨은 동안은 무조건 안 보낸다」 — **VS Code defers a measurable size change while hidden to idle time, and does `flush()` when it becomes visible.** Only a layout of 0 or less is dropped.
   - xterm.js `FitAddon` (0.11.0) does not check for being hidden. If the parent's computed width is `auto` it is NaN so `fit()` does nothing, and if it is 0 it shrinks to 2×1 (`FitAddon.ts:23-24,36,72-74,87-88`). So each peer has its own guard (paseo `offsetWidth/offsetHeight === 0` · orca 48×24 px · 8×4 cell lower bound · VS Code ignores 0 or less · CARB 0×0 guard).
   - First attach that has never been measured: VS Code = the grid last measured by another terminal → if none, 80×30 · server-style = the configured headless default (herdr 120×40 · zellij watching client 80×24).
   - When several views watch one PTY, hidden · inactive views do not decide the size (tmux `window-size` default latest · herdr foreground client · paseo explicit owner).

## Findings table

| Peer | While hidden | When visible | Size | Source | Confidence |
|---|---|---|---|---|---|
| VS Code integrated terminal | Keeps writing | Not applicable | Ignores layout of 0 or less (`:2021`) · defers sending to idle while hidden (`terminalResizeDebouncer.ts:61`) · `flush()` when visible (`:1449`) · first size = last grid (`:2092`) → 80×30 (`:105-106`) | microsoft/vscode@main `terminalInstance.ts` · `terminalResizeDebouncer.ts` · ADR-0056 | Certain |
| Tabby · Hyper · Theia | Keeps writing | Not applicable | (not surveyed) | ADR-0056 research | Likely |
| vibe-kanban | Renders every tab · each has its own xterm + WebSocket | Not applicable | (not surveyed) | vibe-kanban@`packages/ui/src/components/TerminalPanel.tsx:14` | Likely |
| t3code | Keeps mounting · writing up to the cap of 10, skips frames when not visible | Those taken down past the cap: server `open` → snapshot + history | Skips drawing at size 0 (the PTY send side not checked) | t3code@`apps/web/src/terminal/ghostty/surface.ts:1808-1816` · `apps/server/src/terminal/Manager.ts:93-94` | Likely |
| paseo | Retained tabs keep the stream (headless xterm on the server) | First attach · restore = server `TerminalState` snapshot | No fit and no send if `offsetWidth/offsetHeight === 0` · the server updates the size only for the explicit owner | paseo@`packages/app/src/terminal/runtime/terminal-emulator-runtime.ts:371` · `packages/server/src/terminal/terminal-size-ownership.ts:11-38` | Certain (size) · likely (stream — e2e `terminal-retained-tab-stream.spec.ts:46` only asserts that the attach overlay does not appear) |
| orca | A headless xterm (scrollback 5000) parses everything, delivery to the hidden renderer is dropped (default on · a switch turns it off) | Model snapshot (screen + scrollback), seq guard | Defers fit if under 48×24 px · 8×4 cells | orca@`src/main/ipc/pty-hidden-delivery-gate.ts:1-11` · `src/renderer/src/lib/pane-manager/pane-fit-measurability.ts:8-11` (snapshot df7460af — upstream not rechecked) | Certain |
| tmux | The server grid updates every window; output normally goes only to the client viewing that window (there is an exception path for invisible panes, `TTY_CTX_INVISIBLE_PANES`) | Switching windows = redraw from the grid · scrollback stays on the server and is pulled explicitly | `window-size` latest (default)/largest/smallest/manual | tmux@`screen-write.c:138-150` · `server-fn.c:94-104` | Certain |
| zellij | The server grid updates; if no client is viewing that tab, `render` returns early | Full render of the active tab | Per-client size (the final PTY size path not traced) | zellij@`zellij-server/src/tab/mod.rs:4835-4850` | Certain (render) · uncertain (size) |
| herdr | Server is headless; updates for a PTY nobody is watching are dropped | Redraws from the server pane state | Decided by the foreground client · 120×40 if there is no client | herdr@`src/render_signal.rs:14-15` · `src/server/headless.rs:237-243` | Likely |
| CARB/IDE | Now: a renderer is kept per open tab · before: no renderer for hidden tabs | Before: rewind the whole scrollback on every switch → abandoned because of the query-answer leak (#87) | fit 0×0 guard | [ADR-014](https://docs.carbidecore.online/design/decisions/adr-014-terminal-renderer-model/) | Certain (source text checked) |
| Codeman | (tmux backend) | Rewinds a 1 MB tail on switching to a background tab · the truncation marker was weak so it looked like loss | (not surveyed) | [Ark0N/Codeman#258](https://github.com/Ark0N/Codeman/issues/258) | Likely |
| WezTerm · Windows Terminal · kitty · iTerm2 · ttyd/gotty | Keep parsing, draw only what is visible | Not applicable | (not surveyed) | Knowledge-based (no source) | Uncertain |

## grounding (main source-text cross-check)

- Supported: the orca gate comment · tmux `server_redraw_window` · the `screen-write.c` filter · paseo's early return at size 0 · zellij's early return in `render` · VS Code `width <= 0 || height <= 0` · 80×30 · the `_isVisible` deferral · `flush()` · the CARB ADR-014 text · the Codeman #258 text.
- **Correction (collector's misdiagnosis):** 「가려진 부모에서 `fit()` 이 2×1」 (fit() is 2×1 under a hidden parent) → if the computed width is `auto` it is NaN so it is a no-op, and only at 0 does it become 2×1. Which of the two our container (`width:100%`, `TerminalSlot.tsx:381`) falls into under a hidden ancestor was not measured.

## Adversarial review (codex · FIX → incorporated)

- 「되감기는 콜드 경로에만」 → two counterexamples (CARB · Codeman). **Conclusion 2 was fixed.** The reason CARB reverted (query-answer leak) may apply to our rewind path too — see "What remains for us" below.
- 「숨은 동안 크기를 안 보낸다가 예외 없는 규칙」 → VS Code defers sending if it can be measured. **Conclusion 3 was narrowed to 「잴 수 없는 크기」.**
- tmux 「그 창을 보는 클라이언트에만」 → the `TTY_CTX_INVISIBLE_PANES` exception. **A caveat was added to the table.**
- The paseo e2e citation is only partly supported → confidence lowered.

## Options and decision

| Option | What | Gain | Loss | Precedent | Decision |
|---|---|---|---|---|---|
| A keep-alive kept + do not send a size that cannot be measured | The same guard as the other three paths on the initial send right after subscribing | Resolves defect 1 · instant, lossless switching · small change | Hidden tabs still pay the output processing cost (drawing is already stopped) | VS Code · Tabby · Hyper · Theia · vibe-kanban · t3code · paseo · CARB (after reverting) | **Adopted for PC** |
| B server VT + blocking delivery to hidden views | A headless VT in the daemon, snapshot of the current screen when visible | Zero cost for hidden views · restore is sturdier than rewind | A new daemon VT (big change) | tmux · zellij · herdr · orca | On hold — when the hidden-tab cost becomes a problem by measurement |
| C unsubscribe when hidden + rewind when visible (= the path that opens a new pane) | Reuse the current resubscribe path | Zero cost for hidden views · light to implement | Rewind delay · jitter on every switch · risk of rewind query-answer leak (CARB #87) · loss of output beyond the buffer limit | CARB (discarded) · Codeman (over tmux) | **Rejected for PC · mobile candidate** (bandwidth · battery, one pane at a time) — designed when the mobile client is built |

## What remains for us

- **Fix defect 1 (A)** — branch `v0.3.2/fix/terminal-cursor-ime-resize`.
- **Check the rewind query-answer leak** — our input guard (`TerminalSlot.tsx` `onData`) blocks only when there is no agent. When a running agent is placed in a new pane, it needs reproducing whether xterm answers the `CSI 6n` · `OSC 11;?` in the rewound output and leaks into the PTY (`docs/todo.md`).
- Mobile power saving — waiting for its trigger in `docs/todo.md`.

## Limits · gaps

- WezTerm · Windows Terminal · kitty · iTerm2 · ttyd/gotty source texts not checked (knowledge-based) · superset · Coder v2 · sshx not surveyed.
- The orca clone is a month behind upstream.
- Hidden-view handling on the mobile client was not surveyed separately.
