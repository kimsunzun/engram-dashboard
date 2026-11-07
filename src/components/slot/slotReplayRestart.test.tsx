// Regression for attaching a different incarnation (clear signal onReset) — the slot clears the screen and its own seq guard together. And the
// opposite side — no phase notification (onState) touches the screen.
//
// Background (measured 2026-08-20): a different incarnation renumbers frames from 0. The client (ProtocolClient) discards the view progress cursor at the spot where that replay
//   **arrives**, calls onReset, then delivers everything in the same tick. If the slot does not react to that
//   signal it breaks one of two ways —
//   (a) the local seq guard stays at the old high-water and drops every frame coming again from 0, so the screen stays
//   blank forever, or (b) the screen is not cleared and the full replay piles on top of the existing content.
//   Conversely, clearing on phase notifications too makes the screen vanish and redraw every time the socket flickers — 'buffering' contains
//   same-incarnation resume.
//
// Strategy: mock subscribeOutput to capture onChunk/onState/onReset, have the test fire them directly, then
//   feed frames again from seq 0 (same stub pattern as slotTagGate.test.tsx). The same regression on the RichSlot side is
//   seen by RichSlot.test.tsx with its own harness.

import { act, cleanup, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { FRAME_TAG_TERMINAL_BYTES } from '../../api/wsFrame'
import type { OutputChunk, ViewPhase } from '../../api/agentClient'

// Two observers jsdom does not provide — TerminalSlot creates both at mount (size tracking · ADR-0056 WebGL visibility).
//   No-ops that never fire the callback, so only the data path remains.
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
    // Connection-state surface — the slot reads it for the absence-curtain decision (one notification at registration + returns a disposer).
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
// Size is read through a holder — to tell by value whether it was read after fit() actually reflected a measurement
//   (value before fit 80×24, the value after is planted by the test).
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
 * Make the slot "visible" — jsdom has no layout, so it returns `offsetParent` **always** null, and that is exactly the
 * signal the slot uses to decide hidden (tab keep-alive = display:none) (ADR-0056). Untouched, the visible-slot
 * branch can never be stepped on in this harness.
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

describe('TerminalSlot — 비우기 신호에 화면·seq 가드 리셋', () => {
  it('onReset 이 오면 xterm 을 지우고, seq 0 부터 다시 오는 전량을 그린다', async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    // ★Passing the clear callback is itself the regression target★ — if not passed, only the cursor returns to 0 while the screen still holds the previous
    //   incarnation's content, so the full replay piles on top of it (no error appears anywhere).
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

  // ★Does reattaching push the PTY size in again★ — when the daemon restarts, a restored agent's PTY starts at the spawn default
  //   (80×24), while this slot's xterm is still at the size the user stretched it to. The props the parent passes
  //   (viewId·agentId) are unchanged across the restart, so the subscription effect does not run again, and there is no place to propagate the size
  //   other than this clear signal (RO fires only on size *change*, the visibility effect only on transition).
  //   If it goes live misaligned, the TUI computes with the size it believes and every update misses by one row · dozens of columns.
  it('재부착(비우기 신호)에서 지금 터미널 치수를 PTY 에 다시 밀어 넣는다', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    showContainer()

    const subCalls = (agentClient.subscribeOutput as ReturnType<typeof vi.fn>).mock.calls.length
    // fit() = "the container was measured just now" — its result is 137×25 (a measured live value). If the propagation reads the size before fit,
    //   the previous value (80×24) goes out, not the value planted here.
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
    // Also pin that it propagated without going through resubscription (= props unchanged) — if the subscription is split, we are back to the defect where the
    //   screen is wiped before the replay arrives.
    expect((agentClient.subscribeOutput as ReturnType<typeof vi.fn>).mock.calls.length).toBe(subCalls)
  })

  // Opposite side — a hidden slot (tab keep-alive) does not send. fit() on a collapsed container yields the minimum size, and
  //   if that size goes out to the PTY the agent layout breaks (ADR-0056).
  it('숨은 슬롯의 재부착은 치수를 전파하지 않는다', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    fitAddonFit.mockClear()
    ;(agentClient.resizePty as ReturnType<typeof vi.fn>).mockClear()

    fireReset()

    expect(fitAddonFit).not.toHaveBeenCalled()
    expect(agentClient.resizePty).not.toHaveBeenCalled()
  })

  // ★Opposite-direction regression★: if phase notifications clear the screen, every socket flicker wipes the whole terminal and redraws it with a full
  //   re-replay (the form the user rejected). Because 'buffering' contains same-incarnation resume.
  it.each(ALL_PHASES)("'%s' 국면은 화면도 가드도 건드리지 않는다", async (phase) => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    act(() => captured.onChunk!(tag0(0, 'keep')))

    const resetsBefore = xtermReset.mock.calls.length
    fireState(phase)
    expect(xtermReset.mock.calls.length).toBe(resetsBefore)
    act(() => captured.onChunk!(tag0(0, 'dup'))) // An already drawn seq → still dropped
    expect(written()).toEqual(['keep'])
  })

  // Leave the unavailability on the surface — while the agent is absent from the registry, the slot must not look "alive but silent".
  //   (The not-clearing side is measured by the full phase case above — here only the indicator.)
  it("'detached' 는 대기 표시를 띄우고, 부착되면 내린다", async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    fireState('detached')
    expect(document.querySelector('[data-slot-detached="1"]')).not.toBeNull()

    fireState('buffering') // Once attached, the waiting indicator goes down
    expect(document.querySelector('[data-slot-detached="1"]')).toBeNull()
  })

  // ★A view that exhausted the ladder also stays on the surface★: 'error' is a state that receives nothing, and if this is not seen as unavailable,
  //   a slot that is in the registry and connected stays as a **panel with no indication** — neither what went wrong nor how to revive
  //   it is on screen. It is told apart from waiting ('detached') on the observation surface (the picture is the same).
  it("'error' 도 부재 막을 띄운다(대기와는 표면에서 갈린다)", async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    fireState('error')
    const veil = document.querySelector('[data-slot-dead="1"]')
    expect(veil).not.toBeNull()
    expect(veil!.getAttribute('data-slot-phase')).toBe('error')
    expect(veil!.getAttribute('data-slot-detached')).toBeNull()
  })
})

describe('DomSlot — 비우기 신호에 관측 텍스트·seq 가드 리셋', () => {
  it('onReset 이 오면 <pre> 를 비우고 전량을 한 벌로 다시 채운다', async () => {
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

  it.each(ALL_PHASES)("'%s' 국면은 관측 텍스트를 그대로 둔다", async (phase) => {
    render(<DomSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    act(() => captured.onChunk!(tag0(0, 'old-output')))

    fireState(phase)
    expect(domText()).toBe('old-output')
    act(() => captured.onChunk!(tag0(0, 'dup'))) // Guard unchanged too → an already drawn seq is dropped
    expect(domText()).toBe('old-output')
  })

  it("'detached' 는 대기 배지를 띄운다", async () => {
    render(<DomSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    fireState('detached')
    expect(document.querySelector('[data-slot-detached="1"]')).not.toBeNull()
  })

  it("'error' 도 부재 막을 띄운다(TerminalSlot 동형)", async () => {
    render(<DomSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    fireState('error')
    const veil = document.querySelector('[data-slot-dead="1"]')
    expect(veil).not.toBeNull()
    expect(veil!.getAttribute('data-slot-phase')).toBe('error')
  })

  // ★Chunk-boundary state also belongs to the previous incarnation — it must be discarded together★. The two below are regressions of DomSlot only (TerminalSlot does not
  //   decode or strip ANSI, and passes bytes straight to xterm).

  // If the incarnation swap lands in the middle of a multibyte character, the surviving TextDecoder holds the previous incarnation's incomplete bytes
  //   and prepends U+FFFD before the first character of the new replay.
  it('onReset 은 미완 멀티바이트를 물고 있는 디코더를 갈아 끼운다', async () => {
    render(<DomSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    // '한'(U+D55C) = EC 95 9C — the session is cut with only the first two bytes arrived.
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

  // The ANSI edition of the same spot: if the incarnation swap lands in the middle of an incomplete ESC sequence, its tail attaches to the next replay's first byte,
  //   is mistaken for a sequence, and strip swallows the first characters of the new replay whole.
  it('onReset 은 미완 ESC 꼬리(pending)도 버린다', async () => {
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
