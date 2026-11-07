# Terminal rendering (xterm.js on WebView2) — renderer/glyph clipping + root cause of the "black screen"

> Status: survey complete + measurement verified. Date: 2026-07-02. Method: /research medium (Claude Sonnet 2 tracks BLIND + Codex independent cross-check) + cdp measurement on the real app.
> Confidence legend: **certain** (primary source + measurement converge) / **likely** / **uncertain**.

## Background (what started the survey)

When a full-screen TUI such as claude Code was shown in an xterm.js slot, the **top pixels of the first row** of a logo drawn with block/box-drawing glyphs **looked visually clipped** (cosmetic). Our stack: `@xterm/xterm` (6.0.0 at the time) + only `@xterm/addon-fit` loaded = **the default DOM renderer**, WebView2 (Chromium) on Windows, fractional DPI (devicePixelRatio non-integer, rowHeight 15.2px).

## ★ The biggest lesson of this session — the real cause of the "black screen" was not the renderer ★

During debugging the terminal came out entirely black, and we **misdiagnosed it as a renderer problem** and spent a lot of effort on webgl/xterm-bump, but the **real cause was a polluted daemon state** (the user pointed it out exactly as "claude 실행이 안 되는 것" (claude does not launch)).

- The app was restarted many times during the session → **18 orphan claude processes accumulated** + **a daemon tangled by re-attach** (the single engram-dashboard-daemon instance persists across restarts). Output routing (subscription/replay) broke, so the slot stayed empty with only the cursor shown.
- **Fix:** kill only the daemon process (child agents are cleaned up by the Job Object) → clean restart from the baseline → a fresh claude gave **normal output and rendering immediately**. **Certain** (measured).
- **Operating rule:** if the render is an empty slot/cursor only and the agent is "Running" but there is no output → **check the daemon/orphan processes before suspecting the renderer** (the `Get-Process claude` count, whether the daemon is alive). Repeated restarts create cumulative pollution. No indiscriminate kill of `claude.exe` (it would kill my session and the user's other sessions) — kill **only the engram daemon** and let the Job Object clean up.

## Renderer/glyph clipping — survey conclusion (cross-family convergence)

| Claim | Claude | Codex | Verdict |
|---|---|---|---|
| The DOM renderer does **not support** `customGlyphs` → delegates block/box-drawing to the font → top clipping at fractional DPI | certain | certain | **converged · certain** |
| canvas/webgl **draw these glyphs directly as vectors** (customGlyphs default true) → cuts the dependence on font·DPI = removes the clipping | certain | likely | **converged** |
| Mature OSS (ttyd·VS Code·Hyper) does not use DOM — they use canvas/webgl | certain | certain | **converged** |
| Persistent clipping is a renderer problem, not a PTY size problem | certain | likely | **converged** (matches the measurement) |

Related issues: xterm.js #2409 (box/block pixel-perfect→customGlyphs), #967 (rows cut with lineHeight≠1 + DPR≠1), #3807 (DOM glyph top cut off), #4813 (DOM emoji clipping, "webgl does not clip"). Fractional-DPI fix = v5.0.0 PR #3926/#4009/#4105 (canvas/webgl only).

## ★ Where the proper clipping fix is blocked in this environment (measured) ★

As the survey says, canvas/webgl is the answer, but **all three hit a wall in our environment** — so this time we **left the clipping unresolved and kept the baseline (DOM, cosmetic clip)**:

1. **WebGL** — when `@xterm/addon-webgl` is loaded, this WebView2 **cannot draw text glyphs (textures)** (only the cursor rectangle, glyphs are black). Reproduced even with a fresh claude → a pure webgl texture failure with claude's launch ruled out. **Certain** (measured). A GPU/ANGLE texture path limitation is presumed. *However, this measurement may have been mixed with item 3 below (beta promotion), so it needs re-verification — see do-not below.*
2. **Font trap (a separate real bug, confirmed):** `new Terminal({ fontFamily: 'var(--font-terminal)' })` — canvas/webgl rasterize glyphs with canvas 2D `ctx.font`, but **canvas cannot resolve CSS `var()`** (measured: `13px var(--font-terminal)` → fell back to `10px sans-serif`). Only the DOM renderer resolves the var through the CSS cascade, which is why it worked. **To use canvas/webgl, the real font string must be resolved at creation time and passed in** (`getComputedStyle(root).getPropertyValue('--font-terminal')`). **Certain**. (But even with the font fixed, the webgl texture failure of item 1 remained.)
3. **Canvas addon** — `@xterm/addon-canvas` has **peer `@xterm/xterm ^5.0.0` in both** stable 0.7.0 and beta 0.8.0-beta.48 = effectively a **dead end** with xterm 6 (no release for 6.x). CDP capture works (2D), but the version blocks it.
4. **xterm beta promotion = regression** — moving from 6.0.0 to 6.1.0-beta.288 **broke rendering even with the DOM renderer**. Do not jump to a beta. (Rolled back and restored baseline 6.0.0.)

**CDP measurement limit:** `Page.captureScreenshot` in `scripts/cdp.mjs` **cannot capture a WebGL (GPU) canvas** (DOM·canvas are captured). webgl does not even show the cursor in the screenshot → webgl verification needs human eyes. **canvas/DOM can be self-verified by CDP.** ⇒ For autonomous work, prefer a renderer that CDP can capture (canvas/DOM).

## Initial PTY winsize (a separate latent improvement)

The OSS standard = **client-first**: fix the real cols/rows in the frontend → inject the size at spawn time. Rust calls `portable_pty::openpty(PtySize{rows,cols,..})` before `spawn_command` (the two calls are separate). ttyd (protocol.c receives the client {cols,rows} and then spawns)·VS Code·node-pty are all client-first. We currently do a single late resize after spawn. For an alt-screen TUI, the initial size mismatch ends as a **temporary first-frame glitch** and is recovered by SIGWINCH, so it is *not the cause of the persistent clipping*. **Likely**.

## Recommendation (when retrying the clipping next time)

- The **canvas renderer**, which can be self-verified through CDP, is best, but the version blocks it with xterm 6 → (a) wait for a release of the canvas addon that supports xterm6, or (b) use webgl but retry on the premise of **the font-resolution fix + human-eye verification** (but whether this WebView2 texture failure is due to the font or the GPU must be re-verified separately from item 3), or (c) keep DOM + accept the cosmetic clip (the user said "대충 OK" (roughly OK)).
- **Do not promote to a beta xterm.** When changing the renderer, do it one piece at a time from the baseline, verifying with CDP capture.

## do-not / re-verification needed
- The webgl texture-failure measurement was **mixed with the font bug + beta promotion** → "webgl is fundamentally impossible on this WebView2" is **not settled**. Re-verify webgl alone in a clean baseline (xterm 6.0.0 stable) + font-resolution fix state (but screenshot capture is impossible, so human eyes).

## Sources (key)
- xterm.js: #2409, #967, #3807, #4813 · release 5.0.0/5.1.0 notes · ITerminalOptions (customGlyphs/lineHeight/letterSpacing) docs
- ttyd Client-Options wiki (rendererType) · ttyd src/protocol.c (client-first) · VS Code PR #84440·issue #106202 (webgl switch) · Hyper term.tsx · portable-pty docs.rs
