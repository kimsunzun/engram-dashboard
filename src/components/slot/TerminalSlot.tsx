import { useRef, useEffect, useState } from 'react'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { WebglAddon } from '@xterm/addon-webgl'
import '@xterm/xterm/css/xterm.css'
import { agentClient } from '../../api/clientFactory'
import { FRAME_TAG_TERMINAL_BYTES } from '../../api/wsFrame'
import type { OutputSubscription, ViewPhase } from '../../api/agentClient'
import { useAgentStore } from '../../store/agentStore'
import { SlotUnavailableVeil } from './SlotUnavailableVeil'

interface TerminalSlotProps {
  /** Subscription key (ADR-0046) = slot id. Two slots with the same agentId also get independent subscriptions and independent progress through this value (bug B resolved). */
  viewId: string
  agentId: string | null
}

export default function TerminalSlot({ viewId, agentId }: TerminalSlotProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const terminalRef = useRef<Terminal | null>(null)
  const fitAddonRef = useRef<FitAddon | null>(null)
  // ADR-0056: a WebGL seat only for a "visible slot". On hide it is returned (loseContext+dispose) and this ref is
  //   cleared to null. The Terminal instance (buffer/scrollback) stays alive and only the WebGL addon is attached and detached.
  const webglAddonRef = useRef<WebglAddon | null>(null)
  // ADR-0056: the GL context grabbed at attach time for deterministic seat return. The core of the unmount path defense —
  //   because releaseWebgl must not depend on containerRef.current (see the releaseWebgl comment below).
  const glRef = useRef<WebGLRenderingContext | null>(null)
  const agentIdRef = useRef<string | null>(agentId)
  // The absent value the input guard reads — onData runs inside an xterm callback and cannot see state updated by re-render, so
  //   the value that raises the curtain (agentUnavailable) is carried via a ref. The effect below updates it on every decision.
  const unavailableRef = useRef(false)
  const resizeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  // The phase the subscription last announced — the basis of the overlay below. It takes the fact the subscription decided as it is, rather than
  //   deriving registry·connection state separately (one control surface — ADR-0046 §5).
  const [phase, setPhase] = useState<ViewPhase | null>(null)

  useEffect(() => {
    agentIdRef.current = agentId
  }, [agentId])

  const agents = useAgentStore(s => s.agents)
  const agent = agentId ? (agents.find(a => a.id === agentId) ?? null) : null
  // ADR-0148: did we receive the authoritative registry? — a guard so the empty registry before receipt (right after boot/reconnect) is not mistaken for "gone".
  const agentsLoaded = useAgentStore(s => s.agentsLoaded)
  // ★Absent = found in terminal state ∪ unresolved even after the registry is received (ADR-0148)★. The latter is the real outcome of a kill
  //   — the reaper collects the session and removes it from the registry, so status does not catch it. If it were not seen as absent, the dead
  //   agent would remain as a slot with neither curtain nor input guard (one term of agentUnavailable below).
  const agentGone =
    (agentsLoaded && agentId != null && agent == null) ||
    (agent != null &&
      (agent.status.type === 'Exited' ||
        agent.status.type === 'Killed' ||
        agent.status.type === 'Failed'))

  // ADR-0148 decision 1 / ADR-0149 decision 4: the absent decision is the same for all three slots (RichSlot has the canonical form) — ended ∪ connection
  //   lost ∪ the subscription cannot produce output. It only **reads** the existing connection state surface (agentClient). The subscription callback fires once with
  //   the current state on registration, so no separate initial value sync is needed.
  const [connected, setConnected] = useState(() => agentClient.connectionState === 'connected')
  useEffect(() => agentClient.onConnectionStateChange(s => setConnected(s === 'connected')), [])
  // ★'error' is absent too★: a view that exhausted the re-request ladder receives nothing, and without this a slot alive in the registry and
  //   with the connection attached stays as a **blank panel with no indicator** (no signal and no recovery hint).
  const subscriptionDown = phase === 'detached' || phase === 'error'
  const agentUnavailable = agentGone || !connected || subscriptionDown

  // ★Input-blocking condition = curtain-raising condition★: the curtain is pointer-events-none so it does not intercept operation below
  //   (SlotUnavailableVeil) — an xterm that kept focus keeps eating keys even with the curtain up. Blocking on exit alone makes input keep
  //   going out from a disconnected or subscription-dead slot behind a screen that says "없다" (it is gone), and that failure is quietly
  //   swallowed and seen by the user only as no response.
  useEffect(() => { unavailableRef.current = agentUnavailable }, [agentUnavailable])

  useEffect(() => {
    if (!containerRef.current) return
    // The WebGL/canvas renderer rasterizes glyphs with canvas 2D (ctx.font), and a canvas cannot resolve CSS var() →
    // passing 'var(--font-terminal)' as is makes the font invalid and a black screen. It is resolved to the real font string at creation time
    // and passed. (Measured: canvas rejected '13px var(--font-terminal)' → 10px sans-serif)
    const fontFamily =
      getComputedStyle(document.documentElement).getPropertyValue('--font-terminal').trim() || 'monospace'
    const term = new Terminal({
      fontFamily,
      fontSize: 13,
      // User decision 2026-09-25: only the default changes to a white bar and blinking. Opening it as a user setting was put off — for it to be a
      //   setting an LLM control path must come with it (CLAUDE.md 「LLM-우선 제어」).
      // An unfocused slot is a bar too — the xterm default ('outline') draws the whole cell as an empty box, which conflicts with that choice.
      cursorStyle: 'bar',
      cursorInactiveStyle: 'bar',
      cursorBlink: true,
      theme: { background: '#0a0a0a', foreground: '#e0e0e0', cursor: '#ffffff' },
    })
    const fitAddon = new FitAddon()
    term.loadAddon(fitAddon)
    term.open(containerRef.current)
    // ADR-0056: the WebGL renderer is not attached here (at mount) — it moved to the visibility-linked effect. With keep-alive
    //   hidden tabs also keep their Terminal instance alive, but WebGL seats (browser hard cap 16) are given only to visible
    //   slots.
    fitAddon.fit()
    terminalRef.current = term
    fitAddonRef.current = fitAddon

    const ro = new ResizeObserver(entries => {
      // ADR-0056: when hidden (tab keep-alive = display:none) the container collapses to size 0 and RO fires.
      //   Calling fit() then computes cols/rows at minimum/garbage over size 0, and resizePty propagates those dimensions to the
      //   PTY and breaks the agent layout. Return early on the hidden signal and also clear the pending timer.
      //   The signal is offsetParent===null — with display:none (on itself or any ancestor) it is null, which is more robust than the collapsed contentRect
      //   0 width·height (avoids false positives from 1px · overflow residual size).
      const hidden =
        containerRef.current?.offsetParent === null ||
        (entries[0]?.contentRect.width === 0 && entries[0]?.contentRect.height === 0)
      if (hidden) {
        if (resizeTimerRef.current) {
          clearTimeout(resizeTimerRef.current)
          resizeTimerRef.current = null
        }
        return
      }
      fitAddon.fit()
      const aid = agentIdRef.current
      if (!aid) return
      // debounce 50ms — prevents firing IPC every frame during a drag
      if (resizeTimerRef.current) clearTimeout(resizeTimerRef.current)
      resizeTimerRef.current = setTimeout(() => {
        resizeTimerRef.current = null
        // Rejection sink — same shape as sibling call sites. A dead/disconnected slot stays mounted with RO alive (ADR-0148 view
        //   kept), so dragging the divider makes the failure come here. Resize is fire-and-forget so it only absorbs.
        void agentClient.resizePty(aid, term.cols, term.rows).catch(() => {})
      }, 50)
    })
    ro.observe(containerRef.current)

    return () => {
      ro.disconnect()
      if (resizeTimerRef.current) clearTimeout(resizeTimerRef.current)
      // ADR-0056: Terminal dispose alone does not free the GPU seat until GC (see the releaseWebgl comment below).
      releaseWebgl()
      term.dispose()
      terminalRef.current = null
      fitAddonRef.current = null
    }
  }, [])

  // ADR-0056 ── Deterministic return of the WebGL seat ────────────────────────────────
  // ★load-bearing invariant★: xterm's `WebglAddon.dispose()` only detaches the canvas/layer from the DOM and does not call
  //   `WEBGL_lose_context.loseContext()` (source checked: WebglRenderer has no loseContext).
  //   So with dispose alone the GPU context seat stays occupied until GC (non-deterministic) → past the browser hard cap
  //   16 old contexts are lost (ref: PixiJS #8215 — warnings persist after destroy until GC).
  //   So *before* dispose we call loseContext() ourselves to release the seat deterministically.
  // ★Why grab the context at attach time (glRef)★: React clears the host ref (containerRef) to null in the commit's mutation
  //   phase — this is *before* the passive effect cleanup (the point where releaseWebgl runs on unmount).
  //   So if releaseWebgl scans containerRef.current on the unmount path it is already null → the canvas loop
  //   is skipped entirely, loseContext is not called and the seat leaks until GC (closing a tab/slot = exactly the path that needs the return).
  //   So at attach time (attachWebgl, right after loadAddon succeeds) while the container is alive and visible, the GL context is
  //   grabbed into glRef, and here loseContext is called with that grabbed context — it survives ref-null and DOM removal.
  //   (Even the context of a detached canvas is valid until lose/dispose, so a getExtension call is possible. As a precaution,
  //   optional chaining throughout + try-catch — it never throws even if the context cannot be obtained.)
  const releaseWebgl = () => {
    const addon = webglAddonRef.current
    if (!addon) return
    try {
      glRef.current?.getExtension('WEBGL_lose_context')?.loseContext()
    } catch {
      // A loseContext failure is not fatal — dispose carries on (the seat is eventually freed by GC at least).
    }
    addon.dispose()
    webglAddonRef.current = null
    glRef.current = null
  }

  // ADR-0056 ── Visibility-linked WebGL lifecycle ──────────────────────────────────────────
  // Only a visible slot holds a WebGL seat. Under keep-alive (tab hidden = display:none, WindowLayout) IO reports a hidden
  //   target as not-intersecting and fires the callback on transition. ★Initial-hidden caution★: if mounted in a hidden tab
  //   the first observe() callback is already not-intersecting, so you must not assume the first callback means visible —
  //   a slot mounted hidden does not create WebGL until its first "visible" callback (saves seats).
  useEffect(() => {
    const container = containerRef.current
    if (!container) return

    const attachWebgl = () => {
      const term = terminalRef.current
      if (webglAddonRef.current || !term) return
      // WebGL renderer — the DOM renderer does not support customGlyphs so it delegates block/box-drawing characters to the font,
      // and at fractional DPI (non-integer rowHeight) the top pixel of the first row gets shaved. WebGL draws these glyphs itself and
      // removes the clipping (research: xterm.js #2409/#3807/#967). On unsupported/context loss it falls back to DOM automatically.
      // ★Partial-creation leak defense★: if new WebglAddon() already grabbed a GL context and then loadAddon etc. throws,
      //   and the catch does not dispose this addon, the seat leaks and a second addon attaches at the next "visible".
      //   So the addon reference is kept outside try and disposed in catch. The ref is set only on full success.
      let webgl: WebglAddon | undefined
      try {
        webgl = new WebglAddon()
        // On context loss (seat eviction etc.) a reactive DOM fallback. The addon is dropped and the ref cleared
        //   so it can be reattached at the next "visible".
        webgl.onContextLoss(() => {
          webgl?.dispose()
          if (webglAddonRef.current === webgl) {
            webglAddonRef.current = null
            glRef.current = null
          }
        })
        term.loadAddon(webgl)
        webglAddonRef.current = webgl
        // ADR-0056: grab the GL context for deterministic return now (container live·visible). So that on unmount releaseWebgl
        //   can call loseContext even without containerRef — see the releaseWebgl comment for why. Among the canvases the addon
        //   appended to the container, find the one whose WebGL(2) context can be grabbed (a version-robust path independent of layer class names;
        //   the 2d atlas canvas has getContext('webgl2'|'webgl') null so it is naturally skipped).
        const container = containerRef.current
        if (container) {
          for (const canvas of container.querySelectorAll('canvas')) {
            const gl =
              (canvas.getContext('webgl2') as WebGLRenderingContext | null) ??
              (canvas.getContext('webgl') as WebGLRenderingContext | null)
            if (gl) {
              glRef.current = gl
              break
            }
          }
        }
        // So the newly attached GPU renderer draws the current buffer at once: it is visible, so fit()→PTY size propagation→refresh.
        //   (No fit() while hidden — it cannot be measured so garbage dimensions come out.)
        fitAddonRef.current?.fit()
        const aid = agentIdRef.current
        if (aid) void agentClient.resizePty(aid, term.cols, term.rows).catch(() => {})
        // If rows===0 (before measurement etc.), refresh(0, -1) results and is meaningless/misbehaves → refresh only when rows>0.
        if (term.rows > 0) term.refresh(0, term.rows - 1)
      } catch (e) {
        // WebGL unsupported/load failure → DOM renderer fallback. A partially created addon must be disposed (prevents seat leak and duplicate
        //   attach). With no logging we would not know if the clipping fix were silently disabled, so warn.
        webgl?.dispose()
        console.warn('[TerminalSlot] WebGL 비활성 → DOM 렌더러로 폴백', e)
      }
    }

    const io = new IntersectionObserver(entries => {
      for (const entry of entries) {
        const visible = entry.isIntersecting && entry.intersectionRatio > 0
        if (visible) attachWebgl()
        else releaseWebgl() // The DOM renderer takes over automatically (nothing is drawn anyway since it is not visible).
      }
    })
    io.observe(container)

    return () => io.disconnect()
  }, [])

  // Send the current dimensions to the PTY only when they can be measured. When they cannot, send nothing and the PTY keeps its last dimensions
  //   — a rule common to the xterm-based peers surveyed (docs/research/hidden-tab-terminal-subscription-survey-2026-09-25.md).
  // ★There are two branches of the unmeasurable state★ — display:none (caught via offsetParent) and a box that is shown but has
  //   **even one axis** of width·height collapsed to 0. FitAddon divides the parent's computed width·height by the cell size xterm measured from the font
  //   (unrelated to the box), so a collapsed axis is pressed down to the floor value (2 columns · 1 row), and computed values under display:none are not
  //   values measured from real layout. Either way the grid fit() produces cannot be trusted — measured (2026-09-25, WebView2): a cell mounted
  //   hidden has computed width·height of the string "100%", so fit() read it as 100px and produced 11×6 (a cell that was visible then hidden keeps its previous grid because nobody calls fit() while hidden).
  // ★So the dimensions themselves are not inspected★: that grid cannot be told from a normal one by value, and after fit() cols/rows
  //   can be neither 0 nor NaN (the `@xterm/addon-fit` contract) — there is no outlier marker. Since it cannot be split by value, it is split by
  //   **when to send** — both branches drop out before reaching fit().
  // ★The RO callback and onReset are still looser than this decision★ (RO = only when both axes are 0 · onReset = offsetParent only)
  //   — a known gap that cannot filter a box with only one axis collapsed, to be matched when the four send points are folded together
  //   (docs/todo/terminal-sizing.md). Do not loosen this decision back to match theirs.
  // ☐ Not caught: a slot that is visible with a sound box but whose cell size is still 0 — fit() is a no-op so the previous dimensions
  //   go out. There is no retry, and the next layout or visibility event fixes it.
  const sendSizeIfMeasurable = (aid: string, terminal: Terminal) => {
    const container = containerRef.current
    if (!container || container.offsetParent === null) return
    const box = container.getBoundingClientRect()
    if (box.width === 0 || box.height === 0) return
    fitAddonRef.current?.fit()
    void agentClient.resizePty(aid, terminal.cols, terminal.rows).catch(() => {})
  }

  useEffect(() => {
    const terminal = terminalRef.current
    if (!agentId || !terminal) return

    let sub: OutputSubscription | null = null
    let cancelled = false

    terminal.reset() // C2: StrictMode duplicate prevention
    setPhase(null) // The phase of a new subscription is set again by that subscription's notification.
    const lastSeq = { current: -1 } // T-2/G-2: seq dedup (component defense — the client also dedups internally)

    agentClient
      .subscribeOutput(
        viewId,
        agentId,
        chunk => {
          if (cancelled) return
          if (chunk.seq <= lastSeq.current) return
          lastSeq.current = chunk.seq
          // ★tag gate (S15/ADR-0045)★: this slot writes only terminal raw bytes (tag0) to xterm. When tag1
          //   (StructuredEvent JSON) arrives it is ignored — exactly symmetric to RichSlot ignoring tag0.
          //   If a terminal slot attaches to a structured agent (renderModeOverride · multiple subscriptions) or a wiring bug lets tag1 leak into the
          //   shared stream (one seq space), without the gate JSON bytes would be printed to xterm as they are and
          //   pollute the screen. seq was already advanced above (one seq space regardless of tag), so skipping tag1 keeps
          //   dedup consistent.
          if (chunk.tag !== FRAME_TAG_TERMINAL_BYTES) return
          terminal.write(chunk.bytes) // Decoding ends inside the client (transport encapsulation)
        },
        // The screen is not touched on a phase — 'buffering' includes same-incarnation resume, so clearing
        //   there makes a slot that briefly disconnected and came back flicker every time. The value is used only for the absent curtain decision.
        state => {
          if (cancelled) return
          setPhase(state)
        },
        // ★The clearing callback must be passed in all three slots (canonical statement — DomSlot·RichSlot mirror this place)★:
        //   right after calling this callback the router delivers another incarnation's full replay in the same tick and drops the progress cursor.
        //   If it is not passed, only the cursor goes back to 0 while the screen still holds the previous incarnation's content, so the full replay is drawn over it and
        //   the error is left nowhere. The same is why the screen and this local guard are cleared **together** — if only the guard
        //   is reset, content piles up; if only the screen is erased, frames renumbered from 0 are all dropped here and it stays blank.
        () => {
          if (cancelled) return
          terminal.reset()
          lastSeq.current = -1
          // ★A reattach is a different PTY★: the restored agent's PTY starts at the daemon spawn default (80×24) and this
          //   slot's xterm stays at the dimensions it was stretched to (measured 137×25), so if it goes live out of step, the TUI (Ink) computes
          //   the rows to erase and column positions from the size it believes and every update misses by one row and dozens of columns (duplicate lines · status line mixing in).
          //   ★Do not move this propagation back into the subscription trigger above (the incarnation)★ — the failure mode of that form is
          //   canonical in the trigger comment below. It is skipped while hidden (fit() on a collapsed container = minimum dimensions — RO comment) — when visible again
          //   the visibility effect issues the same propagation.
          if (containerRef.current?.offsetParent != null) {
            fitAddonRef.current?.fit()
            void agentClient.resizePty(agentId, terminal.cols, terminal.rows).catch(() => {})
          }
        },
      )
      .then(handle => {
        if (cancelled) {
          handle.unsubscribe()
          return
        }
        sub = handle
        // Task1 (ADR-0036 carry-forward): propagate the initial size once right after subscribing. ResizeObserver rings once when it starts observing
        // (if it is a rendered non-0 box) and after that rings only on size *change*, so when an agent attaches to or changes in a slot whose size is already set
        // it never rings once and the PTY stays stuck at the spawn default (80×24) → the claude
        // welcome box is drawn for 80 columns and breaks in a narrow slot.
        // We fill that empty "initial once" here (the gotty pattern; client-first (ttyd) is impossible because the ADR-0035 structure has the daemon not know the
        // View). The fit() just before sending measures the current box synchronously — even if the cell size changed during the subscription round trip and RO has not rung yet
        // (RO fires only at the next rendering step), the latest cols/rows go out.
        // resizePty is fire-and-forget (Resize has no request_id) — a failure from a just-before kill etc. is absorbed.
        // ★Skip when it cannot be measured (ADR-0056 keep-alive)★: if a subscription is released in a hidden tab, the grid that would go out here is an untrustworthy value from fit() in an
        //   unmeasurable state, and would overwrite the PTY dimensions that a visible slot viewing the same agent had set.
        //   When a hidden tab becomes visible the dimensions come from two paths — the visibility effect (fit()+
        //   resizePty in attachWebgl) and RO (fires as a collapsed box grows to its real size). If WebGL attach fails the former
        //   drops out but RO remains — leaving the two conditions skipped here (display:none · a box with even one axis 0) is
        //   itself a box size change.
        sendSizeIfMeasurable(agentId, terminal)
      })
      // A subscription failure (e.g. NotFound from a just-before kill) is absorbed to prevent an unhandled rejection.
      .catch(() => {})

    return () => {
      cancelled = true
      // Inside unsubscribe it also does the transport cleanup (#13133 delete onmessage) + backend subscription release.
      sub?.unsubscribe()
    }
    // ★The incarnation (epoch) is not put in this trigger★: clearing the screen is borne by onReset alone. If the incarnation is mixed in here, in every span where
    //   the value wavers like a re-spawn or daemon restart the subscription is replaced whole, so **before the replay even arrives** the terminal.reset() above runs and the screen is erased
    //   (the daemon ring is already gone too, so no recovery), and that replacement even loses the incarnation marker so the rotation decision
    //   (protocolClient.observeRoster) cannot stand.
    // viewId included — it is the subscription key (ADR-0046) so a change resubscribes (in practice key=viewId so a slot swap is a remount).
  }, [viewId, agentId])

  // Dimension re-propagation after reconnect ──────────────────────────────────────────────────────────────
  // ADR-0195: a command that comes in while there is no socket is not queued and fails on the spot. resize is
  //   fire-and-forget so that failure is quietly swallowed and there is nowhere to send it again, so if the divider is dragged in the reconnect window (measured 500ms~10s)
  //   those dimensions never reach the agent. Instead of queueing, once the socket is up it is sent here once.
  // ★Do not build this recovery by mixing connection state into the subscription trigger above★ — the failure mode of that form is
  //   canonical in the trigger comment above. This effect does not touch the subscription and only sends dimensions.
  // ★It fires on a transition, not on a state★: connected can stay true for a whole session. An implementation that only looks at the value
  //   goes wrong in two places — **mount** and **agentId swap**. A re-render alone does not run it again (deps are just those two),
  //   and a repeat notification of the same value is filtered by setState so no render even happens. The baseline is the value at mount, so a slot that came up attached
  //   sends nothing here.
  // If the daemon restarted and the agent is gone, the refusal is left as one console warning line — noise that touches neither state nor the re-request ladder
  //   (what issues a reattach is registry observation alone — ADR-0164).
  // If two visible slots view the same agentId both fire — which one remains on the PTY is not
  //   determined (separate async round trips, so firing order does not guarantee arrival order). It is arbitration already present in RO firing,
  //   and this effect only adds one more trigger and creates no new class.
  const wasConnectedRef = useRef(connected)
  useEffect(() => {
    const wasConnected = wasConnectedRef.current
    wasConnectedRef.current = connected
    if (!connected || wasConnected) return
    const terminal = terminalRef.current
    if (!agentId || !terminal) return
    sendSizeIfMeasurable(agentId, terminal)
  }, [connected, agentId])

  useEffect(() => {
    const terminal = terminalRef.current
    if (!agentId || !terminal) return
    const disp = terminal.onData(data => {
      if (unavailableRef.current) return
      void agentClient.writeStdin(agentId, new TextEncoder().encode(data)).catch(() => {})
    })
    return () => disp.dispose()
  }, [agentId])

  return (
    <div style={{
      width: '100%',
      height: '100%',
      position: 'relative',
      boxSizing: 'border-box',
      padding: '4px 8px',        // Terminal left/right margin (wezterm style). Inset by the margin → FitAddon computes cols/rows at that size.
      background: '#0a0a0a',     // Same as the terminal background (Terminal theme) → the margin is seamless
    }}>
      <div ref={containerRef} style={{ width: '100%', height: '100%' }} />
      {agentUnavailable && <SlotUnavailableVeil phase={phase} />}
    </div>
  )
}
