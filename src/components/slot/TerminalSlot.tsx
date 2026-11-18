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
  /** Subscription key (ADR-0046) = slot id; two slots with the same agentId get independent subscriptions and progress. */
  viewId: string
  agentId: string | null
}

export default function TerminalSlot({ viewId, agentId }: TerminalSlotProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const terminalRef = useRef<Terminal | null>(null)
  const fitAddonRef = useRef<FitAddon | null>(null)
  // ADR-0056: a WebGL seat only for a "visible slot"; on hide it is returned (loseContext+dispose) and this ref cleared. The Terminal instance (buffer/scrollback) stays alive; only the WebGL addon is attached and detached.
  const webglAddonRef = useRef<WebglAddon | null>(null)
  // ADR-0056: the GL context grabbed at attach time for deterministic seat return — releaseWebgl must not depend on containerRef.current (see its comment below).
  const glRef = useRef<WebGLRenderingContext | null>(null)
  const agentIdRef = useRef<string | null>(agentId)
  // The absent value the input guard reads — onData runs inside an xterm callback and cannot see re-rendered state, so the curtain value (agentUnavailable) is carried via a ref updated by the effect below.
  const unavailableRef = useRef(false)
  const resizeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  // The phase the subscription last announced — the basis of the overlay below; taken as the subscription decided it, not re-derived from registry·connection state (one control surface — ADR-0046 §5).
  const [phase, setPhase] = useState<ViewPhase | null>(null)

  useEffect(() => {
    agentIdRef.current = agentId
  }, [agentId])

  const agents = useAgentStore(s => s.agents)
  const agent = agentId ? (agents.find(a => a.id === agentId) ?? null) : null
  // ADR-0148: did we receive the authoritative registry? — a guard so the empty registry before receipt (right after boot/reconnect) is not mistaken for "gone".
  const agentsLoaded = useAgentStore(s => s.agentsLoaded)
  // ★Absent = found in terminal state ∪ unresolved even after the registry is received (ADR-0148)★: the latter is the real outcome of a kill (the reaper removes the session from the registry, so status does not show it); without it the dead agent stays as a slot with neither curtain nor input guard.
  const agentGone =
    (agentsLoaded && agentId != null && agent == null) ||
    (agent != null &&
      (agent.status.type === 'Exited' ||
        agent.status.type === 'Killed' ||
        agent.status.type === 'Failed'))

  // ADR-0148 decision 1 / ADR-0149 decision 4: the absent decision is the same for all three slots (RichSlot has the canonical form) — ended ∪ connection lost ∪ the subscription cannot produce output. Only **reads** the connection state surface (agentClient); the subscription callback fires once with the current state, so no initial sync.
  const [connected, setConnected] = useState(() => agentClient.connectionState === 'connected')
  useEffect(() => agentClient.onConnectionStateChange(s => setConnected(s === 'connected')), [])
  // ★'error' is absent too★: a view that exhausted the re-request ladder receives nothing; without this a slot alive in the registry with the connection attached stays a **blank panel with no indicator**.
  const subscriptionDown = phase === 'detached' || phase === 'error'
  const agentUnavailable = agentGone || !connected || subscriptionDown

  // ★Input-blocking condition = curtain-raising condition★: the curtain (SlotUnavailableVeil) is pointer-events-none, so an xterm that kept focus keeps eating keys even with it up; blocking on exit alone lets input keep going out from a disconnected or subscription-dead slot behind a "gone" screen, and the failure is swallowed and seen as no response.
  useEffect(() => { unavailableRef.current = agentUnavailable }, [agentUnavailable])

  useEffect(() => {
    if (!containerRef.current) return
    // The WebGL/canvas renderer rasterizes glyphs with canvas 2D (ctx.font), which cannot resolve CSS var() — passing 'var(--font-terminal)' makes the font invalid and the screen black, so it is resolved to the real font string at creation (canvas rejected '13px var(--font-terminal)' → 10px sans-serif).
    const fontFamily =
      getComputedStyle(document.documentElement).getPropertyValue('--font-terminal').trim() || 'monospace'
    const term = new Terminal({
      fontFamily,
      fontSize: 13,
      // Only the default is a white blinking bar; opening it as a user setting was deferred — it would need an LLM control path too (CLAUDE.md 「LLM-우선 제어」). // hangul-ok
      // An unfocused slot is a bar too — the xterm default ('outline') draws the whole cell as an empty box, which conflicts with that choice.
      cursorStyle: 'bar',
      cursorInactiveStyle: 'bar',
      cursorBlink: true,
      theme: { background: '#0a0a0a', foreground: '#e0e0e0', cursor: '#ffffff' },
    })
    const fitAddon = new FitAddon()
    term.loadAddon(fitAddon)
    term.open(containerRef.current)
    // ADR-0056: the WebGL renderer is not attached at mount — it moved to the visibility-linked effect (keep-alive hidden tabs keep their Terminal instance, but WebGL seats (browser hard cap 16) go only to visible slots).
    fitAddon.fit()
    terminalRef.current = term
    fitAddonRef.current = fitAddon

    const ro = new ResizeObserver(entries => {
      // ADR-0056: when hidden (tab keep-alive = display:none) the container collapses to size 0 and RO fires; fit() then computes minimum/garbage cols/rows and resizePty would propagate them to the PTY and break the agent layout.
      //   Return early on the hidden signal and clear the pending timer. The signal is offsetParent===null (null with display:none on itself or any ancestor) — more robust than a collapsed contentRect 0 (no false positives from 1px · overflow residual size).
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
        // Rejection sink — same shape as sibling call sites. A dead/disconnected slot stays mounted with RO alive (ADR-0148), so a divider drag lands here; resize is fire-and-forget, so only absorb.
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
  // ★load-bearing invariant★: xterm's `WebglAddon.dispose()` only detaches the canvas/layer from the DOM and does not call `WEBGL_lose_context.loseContext()` (source checked: WebglRenderer has no loseContext),
  //   so with dispose alone the GPU context seat stays occupied until GC (non-deterministic) → past the browser hard cap 16 old contexts are lost (ref: PixiJS #8215). So *before* dispose we call loseContext() ourselves.
  // ★Why grab the context at attach time (glRef)★: React clears the host ref (containerRef) to null in the commit's mutation phase — *before* the passive effect cleanup where releaseWebgl runs on unmount; scanning containerRef.current
  //   on the unmount path would find null, skip the canvas loop, never call loseContext, and leak the seat until GC (closing a tab/slot is exactly the path that needs the return). So at attach time (right after loadAddon succeeds) the GL context is
  //   grabbed into glRef and loseContext is called with it — it survives ref-null and DOM removal. (A detached canvas's context stays valid until lose/dispose; optional chaining throughout + try-catch — never throws if the context cannot be obtained.)
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
  // Only a visible slot holds a WebGL seat. Under keep-alive (tab hidden = display:none) IO reports a hidden target as not-intersecting and fires on transition.
  //   ★Initial-hidden caution★: mounted in a hidden tab, the first observe() callback is already not-intersecting, so do not assume the first callback means visible — a slot mounted hidden creates no WebGL until its first "visible" callback (saves seats).
  useEffect(() => {
    const container = containerRef.current
    if (!container) return

    const attachWebgl = () => {
      const term = terminalRef.current
      if (webglAddonRef.current || !term) return
      // WebGL renderer — the DOM renderer does not support customGlyphs, so block/box-drawing characters go to the font and at fractional DPI (non-integer rowHeight) the top pixel of the first row gets shaved; WebGL draws them itself
      // (research: xterm.js #2409/#3807/#967). On unsupported/context loss it falls back to DOM automatically.
      // ★Partial-creation leak defense★: if new WebglAddon() already grabbed a GL context and then loadAddon etc. throws, an addon not disposed in catch leaks the seat and a second addon attaches at the next "visible" —
      //   so the reference is kept outside try and disposed in catch. The ref is set only on full success.
      let webgl: WebglAddon | undefined
      try {
        webgl = new WebglAddon()
        // On context loss (seat eviction etc.): reactive DOM fallback; the addon is dropped and the ref cleared so it can be reattached at the next "visible".
        webgl.onContextLoss(() => {
          webgl?.dispose()
          if (webglAddonRef.current === webgl) {
            webglAddonRef.current = null
            glRef.current = null
          }
        })
        term.loadAddon(webgl)
        webglAddonRef.current = webgl
        // ADR-0056: grab the GL context for deterministic return now (container live·visible), so unmount's releaseWebgl can call loseContext without containerRef (see the releaseWebgl comment). Among the canvases the addon appended, find the one whose WebGL(2) context can be grabbed (version-robust, independent of layer class names; the 2d atlas canvas returns null and is skipped).
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
        // So the newly attached GPU renderer draws the current buffer at once: visible, so fit()→PTY size propagation→refresh (no fit() while hidden — garbage dimensions).
        fitAddonRef.current?.fit()
        const aid = agentIdRef.current
        if (aid) void agentClient.resizePty(aid, term.cols, term.rows).catch(() => {})
        // If rows===0 (before measurement etc.), refresh(0, -1) results and is meaningless/misbehaves → refresh only when rows>0.
        if (term.rows > 0) term.refresh(0, term.rows - 1)
      } catch (e) {
        // WebGL unsupported/load failure → DOM renderer fallback. A partially created addon must be disposed (no seat leak or duplicate attach); warn, or a silently disabled clipping fix would go unnoticed.
        webgl?.dispose()
        console.warn('[TerminalSlot] WebGL disabled → falling back to the DOM renderer', e)
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

  // Send the current dimensions to the PTY only when they can be measured; otherwise send nothing and the PTY keeps its last dimensions — a rule common to the xterm-based peers surveyed (docs/research/hidden-tab-terminal-subscription-survey-2026-09-25.md).
  // ★There are two unmeasurable branches★ — display:none (caught via offsetParent) and a shown box with **even one axis** of width·height collapsed to 0. FitAddon divides the parent's computed width·height by the cell size xterm measured from the font,
  //   so a collapsed axis is pressed to the floor (2 columns · 1 row), and computed values under display:none are not real layout (measured, WebView2: a cell mounted hidden has computed width·height "100%", so fit() read 100px and produced 11×6;
  //   a cell visible then hidden keeps its previous grid because nobody calls fit() while hidden).
  // ★So the dimensions themselves are not inspected★: that grid cannot be told from a normal one by value, and after fit() cols/rows are neither 0 nor NaN (`@xterm/addon-fit` contract) — no outlier marker; so it is split by **when to send** — both branches drop out before reaching fit().
  // ★The RO callback and onReset are still looser than this decision★ (RO = only when both axes are 0 · onReset = offsetParent only) — a known gap for a one-axis-collapsed box, to be matched when the four send points are folded together (docs/todo/terminal-sizing.md). Do not loosen this decision back to match theirs.
  // ☐ Not caught: a visible slot with a sound box whose cell size is still 0 — fit() is a no-op so the previous dimensions go out; no retry, the next layout or visibility event fixes it.
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
          // ★tag gate (S15/ADR-0045)★: this slot writes only terminal raw bytes (tag0) to xterm; tag1 (StructuredEvent JSON) is ignored — symmetric with RichSlot ignoring tag0.
          //   A terminal slot attached to a structured agent (renderModeOverride · multiple subscriptions) or a wiring bug leaking tag1 into the shared stream would otherwise print JSON bytes into xterm. seq was already advanced above (one seq space), so skipping tag1 keeps dedup consistent.
          if (chunk.tag !== FRAME_TAG_TERMINAL_BYTES) return
          terminal.write(chunk.bytes) // Decoding ends inside the client (transport encapsulation)
        },
        // The screen is not touched on a phase — 'buffering' includes same-incarnation resume, so clearing there would flicker a briefly disconnected slot every time. The value is used only for the absent curtain decision.
        state => {
          if (cancelled) return
          setPhase(state)
        },
        // ★The clearing callback must be passed in all three slots (canonical statement — DomSlot·RichSlot mirror this place)★: right after calling it the router delivers another incarnation's full replay in the same tick and drops the progress cursor.
        //   Without it only the cursor goes back to 0 while the screen holds the previous incarnation, so the replay is drawn over it with no error anywhere. The screen and this local guard are cleared **together** — guard only: content piles up; screen only: frames renumbered from 0 are all dropped and it stays blank.
        () => {
          if (cancelled) return
          terminal.reset()
          lastSeq.current = -1
          // ★A reattach is a different PTY★: the restored agent's PTY starts at the daemon spawn default (80×24) while this slot's xterm keeps its stretched dimensions (measured 137×25); going live out of step, the TUI (Ink) computes erase rows and column positions from the size it believes, and every update misses by a row and dozens of columns.
          //   ★Do not move this propagation back into the subscription trigger above (the incarnation)★ — the failure mode is in the trigger comment below. Skipped while hidden (fit() on a collapsed container = minimum dimensions — RO comment); when visible again the visibility effect issues the same propagation.
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
        // Task1 (ADR-0036 carry-forward): propagate the initial size once right after subscribing. ResizeObserver rings once at observation start (for a rendered non-0 box) and afterwards only on size *change*, so an agent attaching to a slot whose size is already set
        // never rings and the PTY stays at the spawn default (80×24) → the claude welcome box is drawn for 80 columns and breaks in a narrow slot. This fills that "initial once" gap (the gotty pattern; client-first (ttyd) is impossible because the ADR-0035 structure has the daemon not know the View).
        // The fit() just before sending measures the box synchronously, so the latest cols/rows go out even if RO has not rung yet (RO fires at the next rendering step). resizePty is fire-and-forget (Resize has no request_id); a failure from a just-before kill is absorbed.
        // ★Skip when it cannot be measured (ADR-0056 keep-alive)★: a subscription released in a hidden tab would send an untrustworthy fit() grid and overwrite the PTY dimensions a visible slot viewing the same agent had set.
        //   When a hidden tab becomes visible the dimensions come from the visibility effect (fit()+resizePty in attachWebgl) and RO (a collapsed box growing to its real size). If WebGL attach fails the former drops out but RO remains — leaving the skipped conditions (display:none · a box with even one axis 0) is itself a box size change.
        sendSizeIfMeasurable(agentId, terminal)
      })
      // A subscription failure (e.g. NotFound from a just-before kill) is absorbed to prevent an unhandled rejection.
      .catch(() => {})

    return () => {
      cancelled = true
      // Inside unsubscribe it also does the transport cleanup (#13133 delete onmessage) + backend subscription release.
      sub?.unsubscribe()
    }
    // ★The incarnation (epoch) is not in this trigger★: clearing the screen is onReset alone. Mixed in here, every span where the value wavers (re-spawn, daemon restart) would replace the whole subscription, so **before the replay arrives** the terminal.reset() above erases the screen (the daemon ring is already gone — no recovery),
    //   and the replacement loses the incarnation marker so the rotation decision (protocolClient.observeRoster) cannot stand.
    // viewId included — it is the subscription key (ADR-0046) so a change resubscribes (in practice key=viewId so a slot swap is a remount).
  }, [viewId, agentId])

  // Dimension re-propagation after reconnect ──────────────────────────────────────────────────────────────
  // ADR-0195: a command that comes in while there is no socket is not queued and fails on the spot. resize is fire-and-forget, so that failure is swallowed with nowhere to resend: a divider drag in the reconnect window (measured 500ms~10s) never reaches the agent.
  //   Instead of queueing, once the socket is up the dimensions are sent here once.
  // ★Do not build this recovery by mixing connection state into the subscription trigger above★ — its failure mode is in the trigger comment above. This effect does not touch the subscription and only sends dimensions.
  // ★It fires on a transition, not on a state★: connected can stay true for a whole session; only looking at the value goes wrong at **mount** and **agentId swap** (deps are just those two, and a repeat notification of the same value is filtered by setState).
  //   The baseline is the value at mount, so a slot that came up attached sends nothing here.
  // If the daemon restarted and the agent is gone, the refusal is one console warning line — noise touching neither state nor the re-request ladder (a reattach is issued by registry observation alone — ADR-0164).
  // If two visible slots view the same agentId both fire — which remains on the PTY is undetermined (separate async round trips); the arbitration already exists in RO firing, and this effect adds one more trigger and no new class.
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
