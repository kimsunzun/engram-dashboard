// Regression for attaching a different incarnation (clear signal onReset): the slot clears the screen and its own seq guard together, and no phase notification (onState) touches the screen.
// Why: a different incarnation renumbers frames from 0. ProtocolClient discards the view progress cursor where that replay **arrives**, calls onReset, then delivers everything in the same tick. A slot ignoring onReset breaks one of two ways — (a) the local seq guard stays at the old high-water and drops every frame from 0 (blank forever), or (b) the screen is not cleared and the full replay piles on top. Clearing on phase notifications too would wipe and redraw on every socket flicker — 'buffering' contains same-incarnation resume.
// Strategy: mock subscribeOutput to capture onChunk/onState/onReset, fire them directly, then feed frames from seq 0 (same stub pattern as slotTagGate.test.tsx). The RichSlot side has its own harness in RichSlot.test.tsx.

import { act, cleanup, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { FRAME_TAG_TERMINAL_BYTES } from '../../api/wsFrame'
import type { OutputChunk, ViewPhase } from '../../api/agentClient'

// Two observers jsdom lacks, created by TerminalSlot at mount (size tracking · ADR-0056 WebGL visibility): no-ops that never fire, leaving only the data path.
globalThis.ResizeObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
} as unknown as typeof ResizeObserver
globalThis.IntersectionObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
  takeRecords() {
    return []
  }
} as unknown as typeof IntersectionObserver

const captured = vi.hoisted(() => ({
  onChunk: null as ((c: OutputChunk) => void) | null,
  onState: null as ((s: ViewPhase) => void) | null,
  onReset: null as (() => void) | null,
}))

vi.mock('../../api/clientFactory', () => ({
  agentClient: {
    // ADR-0046 signature (viewId, agentId, onChunk, onState?, onReset?).
    subscribeOutput: vi.fn(
      async (
        _viewId: string,
        _agentId: string,
        onChunk: (c: OutputChunk) => void,
        onState?: (s: ViewPhase) => void,
        onReset?: () => void,
      ) => {
        captured.onChunk = onChunk
        captured.onState = onState ?? null
        captured.onReset = onReset ?? null
        return { unsubscribe: vi.fn() }
      },
    ),
    writeStdin: vi.fn(async () => undefined),
    resizePty: vi.fn(async () => undefined),
    connectionState: 'connected',
    onConnectionStateChange: (cb: (s: string) => void) => {
      cb('connected')
      return () => {}
    },
  },
  getAgentClient: vi.fn(),
}))

const agentStoreState = vi.hoisted(() => ({ agents: [] as unknown[], agentsLoaded: false }))
vi.mock('../../store/agentStore', () => ({
  useAgentStore: (selector: (s: typeof agentStoreState) => unknown) => selector(agentStoreState),
}))

// xterm stub — shares reset/write through a static holder so calls are counted regardless of instance replacement.
const xtermReset = vi.hoisted(() => vi.fn())
const xtermWrite = vi.hoisted(() => vi.fn())
// Size read through a holder to tell by value whether it was read after fit() reflected a measurement (80×24 before fit; the test plants the value after).
const xtermDims = vi.hoisted(() => ({ cols: 80, rows: 24 }))
vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    loadAddon = vi.fn()
    open = vi.fn()
    reset = xtermReset
    write = xtermWrite
    onData = vi.fn(() => ({ dispose: vi.fn() }))
    dispose = vi.fn()
    refresh = vi.fn()
    get cols() {
      return xtermDims.cols
    }
    get rows() {
      return xtermDims.rows
    }
  },
}))
const fitAddonFit = vi.hoisted(() => vi.fn())
vi.mock('@xterm/addon-fit', () => ({ FitAddon: class { fit = fitAddonFit } }))
vi.mock('@xterm/addon-webgl', () => ({ WebglAddon: class { onContextLoss = vi.fn(); dispose = vi.fn() } }))
vi.mock('@xterm/xterm/css/xterm.css', () => ({}))

import TerminalSlot from './TerminalSlot'
import DomSlot from './DomSlot'

const AGENT = 'aaaa-bbbb-cccc-dddd'
const enc = new TextEncoder()

function tag0(seq: number, text: string): OutputChunk {
  return { seq, tag: FRAME_TAG_TERMINAL_BYTES, bytes: enc.encode(text) }
}

async function flushSubscribe(): Promise<void> {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
}

function fireState(state: ViewPhase): void {
  act(() => captured.onState!(state))
}

function fireReset(): void {
  act(() => captured.onReset!())
}

/** The full phase vocabulary — measure "never clears with any value" as a set, not one at a time. */
const ALL_PHASES: ViewPhase[] = ['detached', 'buffering', 'live', 'error']

function written(): string[] {
  return xtermWrite.mock.calls.map((c) => new TextDecoder().decode(c[0] as Uint8Array))
}

/**
 * Make the slot "visible": jsdom returns `offsetParent` **always** null, which is exactly the hidden signal (tab keep-alive = display:none, ADR-0056); untouched, the visible-slot branch can never run in this harness.
 */
function showContainer(): void {
  const container = document.querySelector('div[style*="padding"]')?.querySelector('div')
  if (!container) throw new Error('containerRef div not found')
  Object.defineProperty(container, 'offsetParent', { value: document.body, configurable: true })
}

function domText(): string {
  return (document.querySelector('[data-dom-mode="1"]') as HTMLElement).textContent ?? ''
}

beforeEach(() => {
  captured.onChunk = null
  captured.onState = null
  captured.onReset = null
  xtermReset.mockClear()
  xtermWrite.mockClear()
  xtermDims.cols = 80
  xtermDims.rows = 24
  fitAddonFit.mockReset()
  agentStoreState.agents = []
  agentStoreState.agentsLoaded = false
})

afterEach(() => {
  cleanup()
})

describe('TerminalSlot — clear signal resets screen · seq guard', () => {
  it('when onReset comes it clears xterm and redraws everything that comes again from seq 0', async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    // ★Passing the clear callback is itself the regression target★ — otherwise only the cursor returns to 0 while the screen keeps the previous incarnation's content and the full replay piles on top (no error anywhere).
    expect(captured.onReset).toBeTruthy()

    act(() => captured.onChunk!(tag0(0, 'first')))
    act(() => captured.onChunk!(tag0(1, 'second')))
    expect(written()).toEqual(['first', 'second'])

    const resetsBefore = xtermReset.mock.calls.length
    fireReset()
    expect(xtermReset.mock.calls.length).toBe(resetsBefore + 1)

    // A different incarnation — numbering restarts from 0. If the guard was not reset, all of them get dropped.
    act(() => captured.onChunk!(tag0(0, 'again')))
    expect(written()).toEqual(['first', 'second', 'again'])
  })

  // ★Does reattaching push the PTY size in again★ — after a daemon restart a restored agent's PTY starts at the spawn default (80×24) while this xterm keeps the stretched size. Props (viewId·agentId) are unchanged, so the subscription effect does not rerun, and this clear signal is the only place to propagate the size (RO fires only on size *change*, the visibility effect only on transition). Going live misaligned makes the TUI miss by a row · dozens of columns on every update.
  it('on reattach (clear signal) it pushes the current terminal size to the PTY again', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    showContainer()

    const subCalls = (agentClient.subscribeOutput as ReturnType<typeof vi.fn>).mock.calls.length
    // fit() = "container measured just now" and returns 137×25 (measured live); reading the size before fit would send the previous 80×24.
    fitAddonFit.mockImplementation(() => {
      xtermDims.cols = 137
      xtermDims.rows = 25
    })
    // Wipe the calls the mount · subscription path already made (the implementation stays) — the assertion below is whether *the reattach* measured.
    fitAddonFit.mockClear()
    ;(agentClient.resizePty as ReturnType<typeof vi.fn>).mockClear()

    fireReset()

    expect(fitAddonFit).toHaveBeenCalled()
    expect(agentClient.resizePty).toHaveBeenCalledWith(AGENT, 137, 25)
    // Also pins that it propagated without resubscription (props unchanged) — splitting the subscription would bring back the screen being wiped before the replay arrives.
    expect((agentClient.subscribeOutput as ReturnType<typeof vi.fn>).mock.calls.length).toBe(subCalls)
  })

  // Opposite side — a hidden slot (tab keep-alive) does not send: fit() on a collapsed container yields the minimum size, which would break the agent layout if sent to the PTY (ADR-0056).
  it('reattach of a hidden slot does not propagate the size', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    fitAddonFit.mockClear()
    ;(agentClient.resizePty as ReturnType<typeof vi.fn>).mockClear()

    fireReset()

    expect(fitAddonFit).not.toHaveBeenCalled()
    expect(agentClient.resizePty).not.toHaveBeenCalled()
  })

  // ★Opposite-direction regression★: if phase notifications cleared the screen, every socket flicker would wipe the terminal and redraw with a full re-replay; 'buffering' contains same-incarnation resume.
  it.each(ALL_PHASES)("'%s' phase does not touch the screen or the guard", async (phase) => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    act(() => captured.onChunk!(tag0(0, 'keep')))

    const resetsBefore = xtermReset.mock.calls.length
    fireState(phase)
    expect(xtermReset.mock.calls.length).toBe(resetsBefore)
    act(() => captured.onChunk!(tag0(0, 'dup'))) // An already drawn seq → still dropped
    expect(written()).toEqual(['keep'])
  })

  // Unavailability must stay visible — while the agent is absent from the registry the slot must not look "alive but silent". (The not-clearing side is covered by the full phase case above.)
  it("'detached' shows the waiting indicator, and lowers it when attached", async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    fireState('detached')
    expect(document.querySelector('[data-slot-detached="1"]')).not.toBeNull()

    fireState('buffering') // Once attached, the waiting indicator goes down
    expect(document.querySelector('[data-slot-detached="1"]')).toBeNull()
  })

  // ★A view that exhausted the ladder also stays visible★: 'error' receives nothing; if not shown as unavailable, a slot that is in the registry and connected is a **panel with no indication** (neither what went wrong nor how to revive). Told apart from waiting ('detached') on the observation surface.
  it("'error' also raises the absence veil (it differs from waiting on the surface)", async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    fireState('error')
    const veil = document.querySelector('[data-slot-dead="1"]')
    expect(veil).not.toBeNull()
    expect(veil!.getAttribute('data-slot-phase')).toBe('error')
    expect(veil!.getAttribute('data-slot-detached')).toBeNull()
  })
})

describe('DomSlot — clear signal resets observed text · seq guard', () => {
  it('when onReset comes it empties <pre> and refills everything as a single set', async () => {
    render(<DomSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    expect(captured.onReset).toBeTruthy()

    act(() => captured.onChunk!(tag0(0, 'old-output')))
    expect(domText()).toBe('old-output')

    fireReset()
    expect(domText()).toBe('')

    act(() => captured.onChunk!(tag0(0, 'old-output')))
    // If piled up it is 'old-outputold-output', if the guard remained it is ''.
    expect(domText()).toBe('old-output')
  })

  it.each(ALL_PHASES)("'%s' phase leaves the observed text as is", async (phase) => {
    render(<DomSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    act(() => captured.onChunk!(tag0(0, 'old-output')))

    fireState(phase)
    expect(domText()).toBe('old-output')
    act(() => captured.onChunk!(tag0(0, 'dup'))) // Guard unchanged too → an already drawn seq is dropped
    expect(domText()).toBe('old-output')
  })

  it("'detached' shows the waiting badge", async () => {
    render(<DomSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    fireState('detached')
    expect(document.querySelector('[data-slot-detached="1"]')).not.toBeNull()
  })

  it("'error' also raises the absence veil (same as TerminalSlot)", async () => {
    render(<DomSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    fireState('error')
    const veil = document.querySelector('[data-slot-dead="1"]')
    expect(veil).not.toBeNull()
    expect(veil!.getAttribute('data-slot-phase')).toBe('error')
  })

  // ★Chunk-boundary state also belongs to the previous incarnation — discard it together★. The two below are DomSlot-only regressions (TerminalSlot passes bytes straight to xterm).

  // An incarnation swap in the middle of a multibyte character: the surviving TextDecoder would hold the previous incarnation's incomplete bytes and prepend U+FFFD to the new replay's first character.
  it('onReset replaces a decoder holding an incomplete multibyte', async () => {
    render(<DomSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    // the Hangul syllable U+D55C = EC 95 9C — the session is cut with only the first two bytes arrived.
    act(() =>
      captured.onChunk!({
        seq: 0,
        tag: FRAME_TAG_TERMINAL_BYTES,
        bytes: new Uint8Array([0xec, 0x95]),
      }),
    )
    expect(domText()).toBe('')

    fireReset()
    act(() => captured.onChunk!(tag0(0, 'A')))
    // If the decoder is not replaced it becomes '�A' (or two replacement characters + A).
    expect(domText()).toBe('A')
  })

  // The ANSI edition: a swap in the middle of an incomplete ESC sequence would attach its tail to the new replay's first byte, be mistaken for a sequence, and strip would swallow the first characters.
  it('onReset also drops an incomplete ESC tail (pending)', async () => {
    render(<DomSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    act(() => captured.onChunk!(tag0(0, 'ok['))) // CSI cut off without a terminating byte → hold in pending
    expect(domText()).toBe('ok')

    fireReset()
    act(() => captured.onChunk!(tag0(0, 'Hello')))
    // If pending remained, '[' + 'Hello' would be held again as an incomplete CSI and the screen would stay blank.
    expect(domText()).toBe('Hello')
  })
})
