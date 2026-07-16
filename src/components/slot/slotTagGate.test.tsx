// tag 게이트 회귀 테스트(FIX-1, S15/ADR-0045) — tag0 소비 슬롯이 tag1 을 걸러내는지 단언.
//
// 배경: subscribeOutput 스트림은 tag0(터미널 raw 바이트)/tag1(StructuredEvent JSON)을 한 seq 공간으로
//   모든 구독자에게 배달한다. RichSlot 은 tag0 을 무시하지만, tag0 을 소비하는 슬롯(TerminalSlot=xterm
//   write / DomSlot=<pre> append)에 대칭 게이트가 없으면 tag1 JSON 바이트가 그대로 새어 화면을 오염시킨다.
//   여기서 각 tag0 소비자가 tag1 chunk 를 write 하지 않고(오염 방지), tag0 은 그대로 write 하는지(회귀) 본다.
//
// 전략: subscribeOutput 을 mock 해 onChunk 콜백을 캡처한 뒤, 마운트 후 tag0/tag1 chunk 를 직접 주입해
//   xterm.write(TerminalSlot) / <pre> 텍스트(DomSlot)에 반영됐는지 관측한다. xterm·transport 는
//   ViewLayoutRenderer.test.tsx 와 동일 패턴으로 stub.

import { act, cleanup, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { FRAME_TAG_STRUCTURED_EVENT, FRAME_TAG_TERMINAL_BYTES } from '../../api/wsFrame'
import type { OutputChunk } from '../../api/agentClient'

// jsdom 은 ResizeObserver 를 제공하지 않는다 — TerminalSlot 이 마운트 시 new ResizeObserver 하므로
// no-op stub 을 깐다(fit/resize 는 이 테스트의 관심사가 아님 — 관심사는 tag 게이트뿐).
globalThis.ResizeObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
} as unknown as typeof ResizeObserver

// jsdom 은 IntersectionObserver 도 없다 — TerminalSlot 이 마운트 시 WebGL 가시성 연동(ADR-0056)으로
// new IntersectionObserver 한다. 콜백을 한 번도 발화하지 않는 no-op stub → WebGL 은 부착되지 않고
// 데이터 경로(tag 게이트)만 검사된다. WebGL 라이프사이클 자체는 TerminalSlot.test.tsx 에서 별도 검증.
globalThis.IntersectionObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
  takeRecords() {
    return []
  }
} as unknown as typeof IntersectionObserver

// ── subscribeOutput 콜백 캡처 holder ─────────────────────────────────────────────
// vi.hoisted 로 만들어 mock factory 와 테스트 본문이 같은 참조를 공유한다.
const captured = vi.hoisted(() => ({ onChunk: null as ((c: OutputChunk) => void) | null }))

vi.mock('../../api/clientFactory', () => ({
  agentClient: {
    // ADR-0046: 시그니처 (viewId, agentId, onChunk, onState?). onChunk 는 3번째 인자.
    subscribeOutput: vi.fn(
      async (_viewId: string, _agentId: string, onChunk: (c: OutputChunk) => void) => {
        captured.onChunk = onChunk
        return { unsubscribe: vi.fn() }
      },
    ),
    writeStdin: vi.fn(async () => undefined),
    resizePty: vi.fn(async () => undefined),
    connectionState: 'connected',
  },
  getAgentClient: vi.fn(),
}))

// ── agentStore stub — 슬롯이 종료 판정용으로 useAgentStore(s => s.agents) 를 조회한다. ──
const agentStoreState = vi.hoisted(() => ({ agents: [] as unknown[] }))
vi.mock('../../store/agentStore', () => ({
  useAgentStore: (selector: (s: { agents: unknown[] }) => unknown) => selector(agentStoreState),
}))

// ── xterm stub — write 호출을 관측(TerminalSlot). Terminal 인스턴스가 매번 새로 생성되므로
//    write mock 을 정적 holder 에 노출해 테스트가 호출 인자를 검사한다. ──
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

// ── 테스트 대상 ────────────────────────────────────────────────────────────────
import TerminalSlot from './TerminalSlot'
import DomSlot from './DomSlot'

const AGENT = 'aaaa-bbbb-cccc-dddd'
const enc = new TextEncoder()

/** tag0 = 터미널 raw 바이트 chunk. */
function tag0(seq: number, text: string): OutputChunk {
  return { seq, tag: FRAME_TAG_TERMINAL_BYTES, bytes: enc.encode(text) }
}
/** tag1 = StructuredEvent JSON chunk(구조화 슬롯 전용 — tag0 소비자엔 오면 안 됨). */
function tag1(seq: number, json: string): OutputChunk {
  return { seq, tag: FRAME_TAG_STRUCTURED_EVENT, bytes: enc.encode(json) }
}

/** 마운트 직후 subscribeOutput 이 콜백을 등록(async .then)할 때까지 마이크로태스크를 비운다. */
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
    // 마지막 write 호출이 tag0 바이트여야 한다.
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

    act(() => captured.onChunk!(tag1(0, '{"kind":"TextDelta"}'))) // seq 0 — 무시되지만 seq 전진
    act(() => captured.onChunk!(tag0(1, 'after'))) // seq 1 — 정상 write
    const calls = xtermWrite.mock.calls.map((c) => new TextDecoder().decode(c[0] as Uint8Array))
    expect(calls).toEqual(['after'])
  })
})

describe('DomSlot — tag 게이트(FIX-1)', () => {
  it('tag0(터미널 바이트) chunk 는 <pre> 관측 텍스트에 반영한다(회귀)', async () => {
    render(<DomSlot viewId="v1" agentId={AGENT} epoch={0} />)
    await flushSubscribe()
    expect(captured.onChunk).toBeTruthy()

    act(() => captured.onChunk!(tag0(0, 'plain-output')))
    const pre = screen.getByText('plain-output')
    expect(pre).toBeTruthy()
  })

  it('tag1(StructuredEvent JSON) chunk 는 무시한다 — 관측 텍스트를 오염시키지 않는다', async () => {
    render(<DomSlot viewId="v1" agentId={AGENT} epoch={0} />)
    await flushSubscribe()

    const json = '{"kind":"TextDelta","text":"leak"}'
    act(() => captured.onChunk!(tag1(0, json)))
    // JSON 문자열이 <pre> 에 새어 나오면 안 된다.
    expect(screen.queryByText(new RegExp('leak'))).toBeNull()
    const pre = document.querySelector('[data-dom-mode="1"]') as HTMLElement
    expect(pre.textContent).toBe('')
  })
})
