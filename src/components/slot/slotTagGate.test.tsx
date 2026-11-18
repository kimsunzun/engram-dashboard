// tag gate regression test (FIX-1, S15/ADR-0045).
// Why: the subscribeOutput stream delivers tag0 (terminal raw bytes) / tag1 (StructuredEvent JSON) to every subscriber in one seq space. RichSlot ignores tag0, but a tag0 consumer (TerminalSlot = xterm write / DomSlot = <pre> append) without a symmetric gate would leak tag1 JSON bytes straight through and pollute the screen.
// Strategy: mock subscribeOutput to capture onChunk, inject tag0/tag1 chunks after mount, and observe xterm.write (TerminalSlot) / <pre> text (DomSlot); xterm · transport are stubbed as in ViewLayoutRenderer.test.tsx.

import { act, cleanup, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { FRAME_TAG_STRUCTURED_EVENT, FRAME_TAG_TERMINAL_BYTES } from '../../api/wsFrame'
import type { OutputChunk } from '../../api/agentClient'

// jsdom has no ResizeObserver — a no-op stub (TerminalSlot creates one at mount; fit/resize are not this test's concern, only the tag gate).
globalThis.ResizeObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
} as unknown as typeof ResizeObserver

// jsdom has no IntersectionObserver either (TerminalSlot creates one at mount for WebGL visibility linkage, ADR-0056): a no-op stub that never fires, so WebGL is not attached and only the data path (tag gate) is checked; the WebGL lifecycle is in TerminalSlot.test.tsx.
globalThis.IntersectionObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
  takeRecords() {
    return []
  }
} as unknown as typeof IntersectionObserver

// ── holder capturing the subscribeOutput callback ─────────────────────────────────────────────
const captured = vi.hoisted(() => ({ onChunk: null as ((c: OutputChunk) => void) | null }))

vi.mock('../../api/clientFactory', () => ({
  agentClient: {
    // ADR-0046: signature (viewId, agentId, onChunk, onState?).
    subscribeOutput: vi.fn(
      async (_viewId: string, _agentId: string, onChunk: (c: OutputChunk) => void) => {
        captured.onChunk = onChunk
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

// ── agentStore stub — the slot queries useAgentStore(s => s.agents) for the exit decision. ──
const agentStoreState = vi.hoisted(() => ({ agents: [] as unknown[] }))
vi.mock('../../store/agentStore', () => ({
  useAgentStore: (selector: (s: { agents: unknown[] }) => unknown) => selector(agentStoreState),
}))

// ── xterm stub — observe write calls (TerminalSlot). The Terminal instance is created fresh each time, so the write mock is exposed through a static holder. ──
const xtermWrite = vi.hoisted(() => vi.fn())
vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    loadAddon = vi.fn()
    open = vi.fn()
    reset = vi.fn()
    write = xtermWrite
    onData = vi.fn(() => ({ dispose: vi.fn() }))
    dispose = vi.fn()
    cols = 80
    rows = 24
  },
}))
vi.mock('@xterm/addon-fit', () => ({ FitAddon: class { fit = vi.fn() } }))
vi.mock('@xterm/addon-webgl', () => ({ WebglAddon: class { onContextLoss = vi.fn(); dispose = vi.fn() } }))
vi.mock('@xterm/xterm/css/xterm.css', () => ({}))

// ── Subject under test ────────────────────────────────────────────────────────────────
import TerminalSlot from './TerminalSlot'
import DomSlot from './DomSlot'

const AGENT = 'aaaa-bbbb-cccc-dddd'
const enc = new TextEncoder()

/** tag0 = terminal raw bytes chunk. */
function tag0(seq: number, text: string): OutputChunk {
  return { seq, tag: FRAME_TAG_TERMINAL_BYTES, bytes: enc.encode(text) }
}
/** tag1 = StructuredEvent JSON chunk (structured slots only — must not reach tag0 consumers). */
function tag1(seq: number, json: string): OutputChunk {
  return { seq, tag: FRAME_TAG_STRUCTURED_EVENT, bytes: enc.encode(json) }
}

/** Right after mount, flush microtasks until subscribeOutput registers the callback (async .then). */
async function flushSubscribe(): Promise<void> {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
}

beforeEach(() => {
  captured.onChunk = null
  xtermWrite.mockClear()
  agentStoreState.agents = []
})

afterEach(() => {
  cleanup()
})

describe('TerminalSlot — tag gate (FIX-1)', () => {
  it('a chunk of tag0 (terminal bytes) is written to xterm (regression)', async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    expect(captured.onChunk).toBeTruthy()

    act(() => captured.onChunk!(tag0(0, 'hello')))
    const calls = xtermWrite.mock.calls.map((c) => new TextDecoder().decode(c[0] as Uint8Array))
    expect(calls).toContain('hello')
  })

  it('a chunk of tag1 (StructuredEvent JSON) is ignored — not written to xterm (prevents pollution)', async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    act(() => captured.onChunk!(tag1(0, '{"kind":"TextDelta","text":"x"}')))
    expect(xtermWrite).not.toHaveBeenCalled()
  })

  it('even skipping tag1 the seq dedup is consistent — the following tag0 is written normally (one seq space)', async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    act(() => captured.onChunk!(tag1(0, '{"kind":"TextDelta"}'))) // seq 0 — ignored but seq advances
    act(() => captured.onChunk!(tag0(1, 'after')))
    const calls = xtermWrite.mock.calls.map((c) => new TextDecoder().decode(c[0] as Uint8Array))
    expect(calls).toEqual(['after'])
  })
})

describe('DomSlot — tag gate (FIX-1)', () => {
  it('a chunk of tag0 (terminal bytes) is reflected in the <pre> observation text', async () => {
    render(<DomSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    expect(captured.onChunk).toBeTruthy()

    act(() => captured.onChunk!(tag0(0, 'plain-output')))
    const pre = screen.getByText('plain-output')
    expect(pre).toBeTruthy()
  })

  it('a chunk of tag1 (StructuredEvent JSON) is ignored — does not corrupt the observation text', async () => {
    render(<DomSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    const json = '{"kind":"TextDelta","text":"leak"}'
    act(() => captured.onChunk!(tag1(0, json)))
    expect(screen.queryByText(new RegExp('leak'))).toBeNull()
    const pre = document.querySelector('[data-dom-mode="1"]') as HTMLElement
    expect(pre.textContent).toBe('')
  })
})
