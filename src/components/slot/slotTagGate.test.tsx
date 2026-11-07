// tag gate regression test (FIX-1, S15/ADR-0045).
//
// Background: the subscribeOutput stream delivers tag0 (terminal raw bytes)/tag1 (StructuredEvent JSON) to every subscriber in one seq space.
//   RichSlot ignores tag0, but if a slot that consumes tag0 (TerminalSlot=xterm write / DomSlot=<pre> append) has no symmetric gate,
//   tag1 JSON bytes leak straight through and pollute the screen.
//
// Strategy: mock subscribeOutput to capture the onChunk callback, inject tag0/tag1 chunks directly after mount, and observe whether they are
//   reflected in xterm.write (TerminalSlot) / <pre> text (DomSlot). xterm · transport are stubbed with the same pattern as
//   ViewLayoutRenderer.test.tsx.

import { act, cleanup, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { FRAME_TAG_STRUCTURED_EVENT, FRAME_TAG_TERMINAL_BYTES } from '../../api/wsFrame'
import type { OutputChunk } from '../../api/agentClient'

// jsdom does not provide ResizeObserver — TerminalSlot does new ResizeObserver at mount, so lay down a
// no-op stub (fit/resize are not this test's concern — its concern is only the tag gate).
globalThis.ResizeObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
} as unknown as typeof ResizeObserver

// jsdom has no IntersectionObserver either — TerminalSlot does new IntersectionObserver at mount for WebGL visibility linkage (ADR-0056).
// A no-op stub that never fires the callback → WebGL is not attached and only the data path (tag gate) is checked.
// The WebGL lifecycle itself is verified separately in TerminalSlot.test.tsx.
globalThis.IntersectionObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
  takeRecords() {
    return []
  }
} as unknown as typeof IntersectionObserver

// ── holder capturing the subscribeOutput callback ─────────────────────────────────────────────
// Made with vi.hoisted so the mock factory and the test body share the same reference.
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

// ── xterm stub — observe write calls (TerminalSlot). Because a Terminal instance is created fresh each time,
//    expose the write mock through a static holder so the test checks the call arguments. ──
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

describe('TerminalSlot — tag 게이트(FIX-1)', () => {
  it('tag0(터미널 바이트) chunk 는 xterm 에 write 한다(회귀)', async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    expect(captured.onChunk).toBeTruthy()

    act(() => captured.onChunk!(tag0(0, 'hello')))
    const calls = xtermWrite.mock.calls.map((c) => new TextDecoder().decode(c[0] as Uint8Array))
    expect(calls).toContain('hello')
  })

  it('tag1(StructuredEvent JSON) chunk 는 무시한다 — xterm 에 write 하지 않는다(오염 방지)', async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    act(() => captured.onChunk!(tag1(0, '{"kind":"TextDelta","text":"x"}')))
    expect(xtermWrite).not.toHaveBeenCalled()
  })

  it('tag1 을 건너뛰어도 seq dedup 은 정합 — 이어지는 tag0 은 정상 write(한 seq 공간)', async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    act(() => captured.onChunk!(tag1(0, '{"kind":"TextDelta"}'))) // seq 0 — ignored but seq advances
    act(() => captured.onChunk!(tag0(1, 'after')))
    const calls = xtermWrite.mock.calls.map((c) => new TextDecoder().decode(c[0] as Uint8Array))
    expect(calls).toEqual(['after'])
  })
})

describe('DomSlot — tag 게이트(FIX-1)', () => {
  it('tag0(터미널 바이트) chunk 는 <pre> 관측 텍스트에 반영한다(회귀)', async () => {
    render(<DomSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    expect(captured.onChunk).toBeTruthy()

    act(() => captured.onChunk!(tag0(0, 'plain-output')))
    const pre = screen.getByText('plain-output')
    expect(pre).toBeTruthy()
  })

  it('tag1(StructuredEvent JSON) chunk 는 무시한다 — 관측 텍스트를 오염시키지 않는다', async () => {
    render(<DomSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    const json = '{"kind":"TextDelta","text":"leak"}'
    act(() => captured.onChunk!(tag1(0, json)))
    expect(screen.queryByText(new RegExp('leak'))).toBeNull()
    const pre = document.querySelector('[data-dom-mode="1"]') as HTMLElement
    expect(pre.textContent).toBe('')
  })
})
