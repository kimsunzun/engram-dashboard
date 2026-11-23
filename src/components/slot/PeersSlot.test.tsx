// PeersSlot (ADR-0394) — the PC list screen over the real peer store and the real `peers.*` commands; only the daemon call is a fake.
// Measures: each state renders, a request card shows the daemon's question, nothing that says yes acts under a pointer something else
// moved there (hold on list / language / size changes, answered placeholders, two-step buttons), actions reach the right `peer.*`
// through the human path with their outcome beside them, and text from outside stays text.

import { StrictMode } from 'react'
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { PeerState } from '../../../crates/engram-dashboard-protocol/bindings/PeerState'

const client = vi.hoisted(() => ({
  peerCommand: vi.fn(async (_name: string, _args: Record<string, unknown>) => ({}) as unknown),
}))
vi.mock('../../api/clientFactory', () => ({ agentClient: client }))

import '../../commands/peerCommands' // side-effect: peers.* registration
import { setLanguage, t } from '../../i18n'
import { en } from '../../i18n/en'
import { ko } from '../../i18n/ko'
import { usePeerStore } from '../../store/peerStore'
import { useViewStore } from '../../store/viewStore'
import PeersSlot, { ANSWERED_MS, CONFIRM_MS, groupId, HOLD_MS, useHold } from './PeersSlot'

const MY_ID = 'a'.repeat(64)
const PC_ID = 'b'.repeat(64)
/** A right-to-left override and markup — must come out as plain, isolated text. */
const HOSTILE = '‮evil<img src=x onerror=alert(1)>'

function state(over: Partial<PeerState> = {}): PeerState {
  return {
    state: 'ready',
    reason: null,
    id: MY_ID,
    addresses: ['192.168.0.2:47120'],
    network: { port: null, listen_port: 47120, relay: 'default', address_lookup: 'default', mdns: true },
    endpoint: 'up',
    endpoint_error: null,
    pcs: [],
    knocks: [],
    requests: [],
    actions: [],
    ...over,
  }
}

function pc(online: string, name = 'office') {
  return { id: PC_ID, name, address: null, online, warning: null, directory: 'unknown', agents: [] }
}

function request(kind: string, over: Partial<PeerState['requests'][number]> = {}): PeerState['requests'][number] {
  return {
    request: 'r1',
    kind,
    summary: `add PC "office" (${groupId(PC_ID)})`,
    agent_id: '6f1c2a90-0000-4000-8000-000000000001',
    agent_name: '"worker"',
    origin: null,
    asked_at_ms: 0,
    expires_in_s: 100,
    title: 'Engram — approve a trust change?',
    heading: 'An agent asks to trust another PC',
    lines: ['An agent (id 6f1c2a90…) asks.', '', `PC ID: ${groupId(PC_ID)}`, '', 'Approving adds it.'],
    ...over,
  }
}

async function mount(s: PeerState) {
  // The mount read answers with the same state (the broadcast-vs-read rule is the store's).
  client.peerCommand.mockImplementation(async (name: string) => (name === 'peer.list' ? s : {}))
  render(<PeersSlot />)
  await act(async () => {})
}

/** Waits out the hold of the yes-buttons. */
async function pastHold() {
  await act(async () => {
    vi.advanceTimersByTime(HOLD_MS)
  })
}

async function click(el: HTMLElement) {
  fireEvent.click(el)
  await act(async () => {})
}

const isOff = (el: HTMLElement) => el.getAttribute('aria-disabled') === 'true'

beforeEach(() => {
  vi.useFakeTimers()
  client.peerCommand.mockReset()
  usePeerStore.setState({ state: null, broadcasts: 0, readFailed: false })
})
afterEach(() => {
  cleanup()
  vi.useRealTimers()
  vi.unstubAllGlobals()
  setLanguage('ko')
})

describe('what it shows', () => {
  it('reads peer.list on mount — the human path that makes the ID the first time', async () => {
    await mount(state())
    // Without the agent rows, as the broadcast — the slot does not show them.
    expect(client.peerCommand).toHaveBeenCalledWith('peer.list', { agents: false })
    expect(screen.getByTestId('peers-my-id').textContent).toBe(groupId(MY_ID))
    expect(groupId(MY_ID).split(' ')).toHaveLength(16)
    expect(screen.getByText('192.168.0.2:47120')).toBeTruthy()
  })

  it.each([
    ['online', () => t('peers.onlineOnline')],
    ['offline', () => t('peers.onlineOffline')],
    ['refused', () => t('peers.onlineRefused')],
    ['version_mismatch', () => t('peers.onlineVersionMismatch')],
    ['duplicate_identity', () => t('peers.onlineDuplicateIdentity')],
    ['endpoint_error', () => t('peers.onlineEndpointError')],
  ])('a PC in state %s shows its word', async (word, text) => {
    await mount(state({ pcs: [pc(word)] }))
    const row = document.querySelector(`[data-pc="${PC_ID}"]`) as HTMLElement
    expect(within(row).getByText(text())).toBeTruthy()
  })

  it('an unknown daemon word shows inside an isolated <bdi>', async () => {
    await mount(state({ pcs: [pc(HOSTILE)] }))
    const cell = document.querySelector(`[data-online]`) as HTMLElement
    const bdi = cell.querySelector('bdi[data-outside]') as HTMLElement
    expect(bdi.textContent).toBe(HOSTILE)
    expect(document.querySelector('img')).toBeNull()
  })

  it('a refused PC says what to do', async () => {
    await mount(state({ pcs: [pc('refused')] }))
    expect(screen.getByText(t('peers.refusedHint'))).toBeTruthy()
  })

  it.each([
    ['key_unreadable', () => t('peers.stateKeyUnreadable')],
    ['list_changed_outside', () => t('peers.stateListChangedOutside')],
    ['adopt_or_fresh', () => t('peers.stateAdoptOrFresh')],
  ])('the %s state is said', async (word, text) => {
    await mount(state({ state: word }))
    expect(screen.getByRole('alert').textContent).toBe(text())
  })

  it('an endpoint error is isolated outside text', async () => {
    await mount(state({ endpoint: 'error', endpoint_error: HOSTILE }))
    const alert = screen.getByRole('alert')
    expect(alert.querySelector('bdi[data-outside]')?.textContent).toBe(HOSTILE)
  })

  it('every PC list string has its Korean twin', () => {
    expect(Object.keys(ko.peers).sort()).toEqual(Object.keys(en.peers).sort())
    expect(ko.slot.fillPeers.length).toBeGreaterThan(0)
  })

  it('a duplicate-identity warning shows on its PC', async () => {
    await mount(state({ pcs: [{ ...pc('duplicate_identity'), warning: 'duplicate identity — two running copies' }] }))
    expect(screen.getByText(/two running copies/)).toBeTruthy()
  })

  it('the requests section is first and always there, with a count and an empty state', async () => {
    await mount(state())
    const sections = document.querySelectorAll('[data-peers-slot] [data-testid]')
    expect(sections[0].getAttribute('data-testid')).toBe('peers-requests')
    expect(screen.getByText(t('peers.noRequests'))).toBeTruthy()
    act(() => usePeerStore.getState().receive(state({ requests: [request('add')] })))
    expect(screen.getByTestId('peers-requests-count').textContent).toBe(t('peers.requestsWaiting', { count: '1' }))
  })

  it("a request card shows the daemon's question — heading and every line — and no English summary beside a translated heading", async () => {
    const lines = [
      'An agent (id 6f1c2a90…) asks. Its name: "lead"',
      '',
      'Relay server: n0’s public servers (default) → none',
      `Note: earlier in this run the name "office" belonged to another PC (ID ${groupId(PC_ID)}).`,
      '',
      'Approving gives this PC a new ID: every other PC must remove the old ID and pair with the new one.',
    ]
    await mount(state({ requests: [request('setNetwork', { heading: 'An agent asks to change how this PC reaches other PCs', lines })] }))
    const card = document.querySelector('[data-request="r1"]') as HTMLElement
    expect(within(card).getByText('An agent asks to change how this PC reaches other PCs')).toBeTruthy()
    expect(within(card).queryByText(request('add').summary)).toBeNull()
    const shown = Array.from(card.querySelectorAll('[data-testid="peers-request-lines"] bdi')).map(el => el.textContent)
    expect(shown).toEqual(lines.filter(l => l !== ''))
    for (const el of card.querySelectorAll('[data-testid="peers-request-lines"] bdi')) {
      expect((el as HTMLElement).style.unicodeBidi).toBe('isolate')
    }
  })

  it('an older daemon without question lines: the kind word and the summary show', async () => {
    await mount(state({ requests: [request('add', { heading: '', lines: [] })] }))
    const card = document.querySelector('[data-request="r1"]') as HTMLElement
    expect(within(card).getByText(t('peers.kindAdd'))).toBeTruthy()
    expect(within(card).getByText(request('add').summary)).toBeTruthy()
  })

  it('an agent without a name is said without an empty name part', async () => {
    await mount(state({ requests: [request('add', { agent_name: null })] }))
    const card = document.querySelector('[data-request="r1"]') as HTMLElement
    const expected = t('peers.askedByNoName', { id: '6f1c2a90' })
    expect(card.textContent).toContain(expected)
    expect(card.textContent).not.toContain(t('peers.askedBy', { id: '6f1c2a90', name: '' }))
  })

  it('outside strings are text inside an isolated <bdi>, never markup — the system id before the quoted agent name', async () => {
    await mount(
      state({
        pcs: [pc('online', HOSTILE)],
        requests: [request('add', { heading: '', agent_name: HOSTILE, origin: HOSTILE, summary: HOSTILE })],
      }),
    )
    expect(document.querySelector('img')).toBeNull()
    const hostile = Array.from(document.querySelectorAll('bdi[data-outside]')).filter(el => el.textContent === HOSTILE)
    // name, agent name, origin, summary
    expect(hostile.length).toBe(4)
    for (const el of hostile) expect((el as HTMLElement).style.unicodeBidi).toBe('isolate')
    const card = document.querySelector('[data-request="r1"]') as HTMLElement
    const meta = Array.from(card.querySelectorAll('bdi[data-outside]')).map(el => el.textContent)
    expect(meta.indexOf('6f1c2a90')).toBeGreaterThan(-1)
    expect(meta.indexOf('6f1c2a90')).toBeLessThan(meta.indexOf(HOSTILE, meta.indexOf('6f1c2a90')))
  })

  it('the daemon-quoted agent name shows with its quotes', async () => {
    await mount(state({ requests: [request('add')] }))
    const card = document.querySelector('[data-request="r1"]') as HTMLElement
    expect(Array.from(card.querySelectorAll('bdi[data-outside]')).some(el => el.textContent === '"worker"')).toBe(true)
  })

  it('an unbroken run wraps inside the slot instead of widening it', async () => {
    await mount(state({ requests: [request('add')] }))
    const root = document.querySelector('[data-peers-slot]') as HTMLElement
    expect(root.style.overflowWrap).toBe('anywhere')
  })
})

/** A fake ResizeObserver; `live()` = the observers not disconnected, each with the boxes it watches. */
function fakeResizeObserver() {
  const observers: { cb: ResizeObserverCallback; targets: Element[]; disconnected: boolean }[] = []
  vi.stubGlobal(
    'ResizeObserver',
    class {
      private entry: { cb: ResizeObserverCallback; targets: Element[]; disconnected: boolean }
      constructor(cb: ResizeObserverCallback) {
        this.entry = { cb, targets: [], disconnected: false }
        observers.push(this.entry)
      }
      observe(target: Element) {
        this.entry.targets.push(target)
      }
      disconnect() {
        this.entry.disconnected = true
      }
      unobserve() {}
    },
  )
  /** Reports card `id` at `width` to every live observer watching it — outside `act`, as the browser does. */
  const resize = (id: string, width: number) => {
    for (const o of observers.filter(o => !o.disconnected)) {
      const card = o.targets.find(el => el.getAttribute('data-request') === id)
      if (card) o.cb([{ target: card, contentRect: { width, height: 40 } } as unknown as ResizeObserverEntry], {} as ResizeObserver)
    }
  }
  return { observers, resize }
}

describe('mis-aimed clicks', () => {
  it('useHold is held in the very render whose key changed — not one commit later', async () => {
    const log: [string, boolean][] = []
    function Probe({ k }: { k: string }) {
      log.push([k, useHold(k)])
      return null
    }
    const { rerender } = render(<Probe k="a" />)
    await act(async () => {
      vi.advanceTimersByTime(HOLD_MS)
    })
    expect(log[log.length - 1]).toEqual(['a', false])
    rerender(<Probe k="b" />)
    // Every render of the new key until its own timer settles it is held — including the first.
    const renders = log.filter(([k]) => k === 'b')
    expect(renders[0]).toEqual(['b', true])
    // A timer started for an older key does not settle the new one.
    await act(async () => {
      vi.advanceTimersByTime(HOLD_MS / 2)
    })
    expect(log[log.length - 1]).toEqual(['b', true])
    await act(async () => {
      vi.advanceTimersByTime(HOLD_MS / 2)
    })
    expect(log[log.length - 1]).toEqual(['b', false])
  })

  it('the yes-buttons are held after the cards appear and after they change; no is never held', async () => {
    await mount(state({ requests: [request('add')] }))
    expect(isOff(screen.getByTestId('peers-approve'))).toBe(true)
    expect(screen.getByTestId('peers-deny').getAttribute('aria-disabled')).toBeNull()
    await click(screen.getByTestId('peers-approve'))
    expect(client.peerCommand).not.toHaveBeenCalledWith('peer.approve', expect.anything())

    await pastHold()
    expect(isOff(screen.getByTestId('peers-approve'))).toBe(false)

    act(() => usePeerStore.getState().receive(state({ requests: [request('add'), request('add', { request: 'r2' })] })))
    for (const el of screen.getAllByTestId('peers-approve')) expect(isOff(el)).toBe(true)
    await pastHold()
    // The order changes alone: held again.
    act(() => usePeerStore.getState().receive(state({ requests: [request('add', { request: 'r2' }), request('add')] })))
    for (const el of screen.getAllByTestId('peers-approve')) expect(isOff(el)).toBe(true)
  })

  it('a language switch holds the yes-buttons again; the answer buttons keep one width in both languages', async () => {
    await mount(state({ requests: [request('add')] }))
    await pastHold()
    const width = screen.getByTestId('peers-approve').style.width
    expect(width).not.toBe('')
    expect(screen.getByTestId('peers-deny').style.width).toBe(width)
    act(() => {
      setLanguage('en')
    })
    expect(isOff(screen.getByTestId('peers-approve'))).toBe(true)
    expect(screen.getByTestId('peers-approve').style.width).toBe(width)
    expect(screen.getByTestId('peers-deny').style.width).toBe(width)
  })

  it('a box that changes size (slot, section or card) holds the yes-buttons again', async () => {
    const ro = fakeResizeObserver()
    await mount(state({ requests: [request('add')] }))
    const fire = (width: number) => act(() => ro.resize('r1', width))
    fire(300) // first size seen — not a move
    await pastHold()
    expect(isOff(screen.getByTestId('peers-approve'))).toBe(false)
    fire(200) // the card re-wrapped
    expect(isOff(screen.getByTestId('peers-approve'))).toBe(true)
  })

it('a return to an already settled key (A → B → A) is held again — the hold counts changes, not values', async () => {
    const log: [string, boolean][] = []
    function Probe({ k }: { k: string }) {
      log.push([k, useHold(k)])
      return null
    }
    const { rerender } = render(<Probe k="a" />)
    await act(async () => {
      vi.advanceTimersByTime(HOLD_MS)
    })
    expect(log[log.length - 1]).toEqual(['a', false])
    rerender(<Probe k="b" />)
    await act(async () => {
      vi.advanceTimersByTime(HOLD_MS / 2)
    })
    rerender(<Probe k="a" />)
    expect(log[log.length - 1]).toEqual(['a', true])
    // B's timer would have settled by now; A's own revision is not settled until HOLD_MS after the return.
    await act(async () => {
      vi.advanceTimersByTime(HOLD_MS / 2)
    })
    expect(log[log.length - 1]).toEqual(['a', true])
    await act(async () => {
      vi.advanceTimersByTime(HOLD_MS / 2)
    })
    expect(log[log.length - 1]).toEqual(['a', false])
  })

  it('a press right after a resize is refused before React re-renders the hold', async () => {
    const ro = fakeResizeObserver()
    await mount(state({ requests: [request('add')] }))
    act(() => ro.resize('r1', 300)) // first size seen — not a move
    await pastHold()
    const approve = screen.getByTestId('peers-approve')
    expect(isOff(approve)).toBe(false)
    ro.resize('r1', 200) // the card re-wrapped; React has not rendered yet
    expect(isOff(approve)).toBe(false)
    fireEvent.click(approve)
    expect(client.peerCommand).not.toHaveBeenCalledWith('peer.approve', expect.anything())
    await act(async () => {})
    await pastHold()
    await click(screen.getByTestId('peers-approve'))
    expect(client.peerCommand).toHaveBeenCalledWith('peer.approve', { request: 'r1' })
  })

  it('a press right after the slot scrolls or the window resizes is refused; no is never refused', async () => {
    await mount(state({ requests: [request('add')] }))
    await pastHold()
    const viewport = document.querySelector('[data-radix-scroll-area-viewport]') as HTMLElement
    expect(viewport).not.toBeNull()
    viewport.dispatchEvent(new Event('scroll'))
    expect(isOff(screen.getByTestId('peers-approve'))).toBe(false)
    fireEvent.click(screen.getByTestId('peers-approve'))
    expect(client.peerCommand).not.toHaveBeenCalledWith('peer.approve', expect.anything())
    await act(async () => {})
    await pastHold()
    window.dispatchEvent(new Event('resize'))
    fireEvent.click(screen.getByTestId('peers-approve'))
    expect(client.peerCommand).not.toHaveBeenCalledWith('peer.approve', expect.anything())
    fireEvent.click(screen.getByTestId('peers-deny'))
    expect(client.peerCommand).toHaveBeenCalledWith('peer.deny', { request: 'r1' })
    await act(async () => {})
  })

  it('a press right after the window comes to the front or becomes visible is refused — the click that activates a raised window', async () => {
    await mount(state({ requests: [request('add')] }))
    for (const fire of [() => window.dispatchEvent(new Event('focus')), () => document.dispatchEvent(new Event('visibilitychange'))]) {
      await pastHold()
      expect(isOff(screen.getByTestId('peers-approve'))).toBe(false)
      fire() // outside act — React has not rendered the hold
      fireEvent.click(screen.getByTestId('peers-approve'))
      expect(client.peerCommand).not.toHaveBeenCalledWith('peer.approve', expect.anything())
      await act(async () => {})
      expect(isOff(screen.getByTestId('peers-approve'))).toBe(true)
    }
  })

  it("a slot moved in the layout without resizing holds and refuses a press — its own rectangle is in the hold", async () => {
    const at = (x0: number, version: number) =>
      useViewStore.setState({
        layouts: {
          v1: {
            layout: { type: 'slot', id: 's1', content: { type: 'peers' } } as never,
            focusedSlotId: null,
            slotRects: [{ slot_id: 's1', x0, y0: 0, x1: x0 + 0.5, y1: 1 }],
            splitRects: [],
            ratioMin: 0.1,
            ratioMax: 0.9,
            foreignSlots: [],
            version,
          },
        },
      })
    try {
      at(0, 1)
      client.peerCommand.mockImplementation(async (name: string) => (name === 'peer.list' ? state({ requests: [request('add')] }) : {}))
      render(<PeersSlot viewId="v1" slotId="s1" />)
      await act(async () => {})
      await pastHold()
      expect(isOff(screen.getByTestId('peers-approve'))).toBe(false)
      // Same size, shifted right (a compensating split ratio pair).
      act(() => at(0.25, 2))
      expect(isOff(screen.getByTestId('peers-approve'))).toBe(true)
      fireEvent.click(screen.getByTestId('peers-approve'))
      expect(client.peerCommand).not.toHaveBeenCalledWith('peer.approve', expect.anything())
      await pastHold()
      // A layout update that leaves this slot where it was holds nothing.
      act(() => at(0.25, 3))
      expect(isOff(screen.getByTestId('peers-approve'))).toBe(false)
    } finally {
      useViewStore.setState({ layouts: {} })
    }
  })

  it('a row added inside the slot (the scroll content grows) holds and refuses a press', async () => {
    const ro = fakeResizeObserver()
    await mount(state({ requests: [request('add')] }))
    const content = (document.querySelector('[data-radix-scroll-area-viewport]') as HTMLElement).firstElementChild as Element
    expect(ro.observers.some(o => !o.disconnected && o.targets.includes(content))).toBe(true)
    const grow = (height: number) => {
      for (const o of ro.observers.filter(o => !o.disconnected && o.targets.includes(content))) {
        o.cb([{ target: content, contentRect: { width: 300, height } } as unknown as ResizeObserverEntry], {} as ResizeObserver)
      }
    }
    act(() => grow(400)) // first size seen — not a move
    await pastHold()
    grow(430) // the state row appeared above My ID; React has not rendered yet
    fireEvent.click(screen.getByTestId('peers-approve'))
    expect(client.peerCommand).not.toHaveBeenCalledWith('peer.approve', expect.anything())
    await act(async () => {})
    expect(isOff(screen.getByTestId('peers-approve'))).toBe(true)
  })

  it('a two-step button refuses a press right after a move too', async () => {
    await mount(state())
    await pastHold()
    window.dispatchEvent(new Event('resize'))
    fireEvent.click(screen.getByTestId('peers-regenerate'))
    expect(screen.getByTestId('peers-regenerate').textContent).toBe(t('peers.regenerate'))
    await act(async () => {})
  })

  it('under Strict Mode the observer that survives still watches every box', async () => {
    const ro = fakeResizeObserver()
    client.peerCommand.mockImplementation(async (name: string) =>
      name === 'peer.list' ? state({ requests: [request('add')] }) : {},
    )
    // The card is there from the first render — the boxes a Strict Mode re-run must watch again.
    usePeerStore.setState({ state: state({ requests: [request('add')] }) })
    render(
      <StrictMode>
        <PeersSlot />
      </StrictMode>,
    )
    await act(async () => {})
    expect(ro.observers.some(o => o.disconnected)).toBe(true)
    act(() => ro.resize('r1', 300))
    await pastHold()
    expect(isOff(screen.getByTestId('peers-approve'))).toBe(false)
    act(() => ro.resize('r1', 200))
    expect(isOff(screen.getByTestId('peers-approve'))).toBe(true)
  })

  it('the answer buttons never grow past a narrow slot', async () => {
    await mount(state({ requests: [request('add')] }))
    for (const id of ['peers-approve', 'peers-deny', 'peers-deny-stop']) {
      expect(screen.getByTestId(id).style.maxWidth).toBe('100%')
    }
  })

  it("an answered card says what was answered in the UI language — the request's heading, not the daemon's English summary", async () => {
    await mount(state({ requests: [request('add', { heading: 'PC 추가 요청' })] })) // hangul-ok
    await pastHold()
    await click(screen.getByTestId('peers-approve'))
    const ghost = document.querySelector('[data-ghost="r1"]') as HTMLElement
    expect(ghost.textContent).toContain(t('peers.answeredApproved', { what: 'PC 추가 요청' })) // hangul-ok
    expect(ghost.textContent).not.toContain(request('add').summary)
  })

  it('an older daemon without a heading: the answered card falls back to the kind word and the summary', async () => {
    await mount(state({ requests: [request('add', { heading: '', lines: [] })] }))
    await pastHold()
    await click(screen.getByTestId('peers-approve'))
    const ghost = document.querySelector('[data-ghost="r1"]') as HTMLElement
    expect(ghost.textContent).toContain(t('peers.kindAdd'))
    expect(ghost.textContent).toContain(t('peers.answeredApproved', { what: request('add').summary }))
    act(() => {
      setLanguage('en')
    })
    expect(ghost.textContent).toContain(en.peers.kindAdd)
  })

  it('a held button keeps the keyboard focus', async () => {
    await mount(state({ requests: [request('add')] }))
    const approve = screen.getByTestId('peers-approve')
    approve.focus()
    expect(document.activeElement).toBe(approve)
    expect(approve.hasAttribute('disabled')).toBe(false)
  })

  it('an answered card keeps its place as a placeholder, so the next card does not slide under the pointer', async () => {
    const two = state({
      requests: [request('add'), request('add', { request: 'r2', summary: 'add PC "lаptop"' })],
    })
    await mount(two)
    await pastHold()
    // The daemon drops r1 right after the approve; r2 would move up into its place.
    client.peerCommand.mockImplementation(async (name: string) =>
      name === 'peer.list' ? state({ requests: [request('add', { request: 'r2', summary: 'add PC "lаptop"' })] }) : {},
    )
    await click(screen.getAllByTestId('peers-approve')[0])
    expect(client.peerCommand).toHaveBeenCalledWith('peer.approve', { request: 'r1' })
    const ghost = document.querySelector('[data-ghost="r1"]') as HTMLElement
    expect(ghost).not.toBeNull()
    expect(ghost.textContent).toContain(t('peers.answeredApproved', { what: request('add').heading }))
    expect(within(ghost).getByRole('status').textContent).toBe(t('peers.done'))
    expect(ghost.querySelector('button')).toBeNull()
    // The placeholder sits where r1 was — before r2.
    const order = Array.from(document.querySelectorAll('[data-ghost], [data-request]')).map(
      el => el.getAttribute('data-ghost') ?? el.getAttribute('data-request'),
    )
    expect(order).toEqual(['r1', 'r2'])
    await act(async () => {
      vi.advanceTimersByTime(ANSWERED_MS)
    })
    expect(document.querySelector('[data-ghost="r1"]')).toBeNull()
    // r2 moved up: held again.
    expect(isOff(screen.getByTestId('peers-approve'))).toBe(true)
  })
})

describe('what it does', () => {
  it('approve answers an agent request', async () => {
    await mount(state({ requests: [request('add')] }))
    await pastHold()
    await click(screen.getByTestId('peers-approve'))
    expect(client.peerCommand).toHaveBeenCalledWith('peer.approve', { request: 'r1' })
  })

  it('deny-and-stop is never held and answers with stop_asking', async () => {
    await mount(state({ requests: [request('add')] }))
    await click(screen.getByTestId('peers-deny-stop'))
    expect(client.peerCommand).toHaveBeenCalledWith('peer.deny', { request: 'r1', stop_asking: true })
    expect(document.querySelector('[data-ghost="r1"]')?.textContent).toContain(
      t('peers.answeredDenied', { what: request('add').heading }),
    )
  })

  it("the adopt question's third answer is start fresh — an approve after a second, unhurried click, never a deny", async () => {
    await mount(state({ state: 'adopt_or_fresh', requests: [request('adopt', { agent_id: null, agent_name: null })] }))
    await pastHold()
    await click(screen.getByTestId('peers-start-fresh'))
    expect(screen.getByText(t('peers.startFreshConfirm'))).toBeTruthy()
    // A double-click's second half is not taken.
    await click(screen.getByTestId('peers-start-fresh'))
    expect(client.peerCommand).not.toHaveBeenCalledWith('peer.approve', expect.anything())
    await pastHold()
    await click(screen.getByTestId('peers-start-fresh'))
    expect(client.peerCommand).toHaveBeenCalledWith('peer.approve', { request: 'r1', start_fresh: true })
    expect(client.peerCommand).not.toHaveBeenCalledWith('peer.deny', expect.anything())
  })

  it('an armed two-step button disarms after a while', async () => {
    await mount(state())
    await pastHold()
    await click(screen.getByTestId('peers-regenerate'))
    expect(screen.getByTestId('peers-regenerate').textContent).toBe(t('peers.confirmAgain'))
    await act(async () => {
      vi.advanceTimersByTime(CONFIRM_MS)
    })
    expect(screen.getByTestId('peers-regenerate').textContent).toBe(t('peers.regenerate'))
    await click(screen.getByTestId('peers-regenerate'))
    expect(client.peerCommand).not.toHaveBeenCalledWith('peer.regenerateKey', expect.anything())
  })

  it('regenerate and remove share the hold — a new request holds them too', async () => {
    await mount(state({ pcs: [pc('online')] }))
    await pastHold()
    expect(isOff(screen.getByTestId('peers-regenerate'))).toBe(false)
    act(() => usePeerStore.getState().receive(state({ pcs: [pc('online')], requests: [request('add')] })))
    expect(isOff(screen.getByTestId('peers-regenerate'))).toBe(true)
    expect(isOff(screen.getByTestId('peers-remove'))).toBe(true)
  })

  it('an approve that replaced moved network fields says so in the answered card', async () => {
    await mount(state({ requests: [request('setNetwork')] }))
    await pastHold()
    client.peerCommand.mockImplementation(async (name: string) =>
      name === 'peer.approve' ? { outcome: 'applied', message: 'm', id: null, moved: ['relay'] } : state(),
    )
    await click(screen.getByTestId('peers-approve'))
    const ghost = document.querySelector('[data-ghost="r1"]') as HTMLElement
    expect(within(ghost).getByRole('status').textContent).toBe(t('peers.networkMoved', { fields: 'relay' }))
  })

  it('a refused answer keeps the card and says why beside the requests', async () => {
    await mount(state({ requests: [request('add')] }))
    await pastHold()
    client.peerCommand.mockImplementation(async (name: string) => {
      if (name === 'peer.approve') throw new Error('NOT_FOUND: no such request')
      return state({ requests: [request('add')] })
    })
    await click(screen.getByTestId('peers-approve'))
    expect(document.querySelector('[data-ghost="r1"]')).toBeNull()
    expect(document.querySelector('[data-request="r1"]')).not.toBeNull()
    const status = within(screen.getByTestId('peers-requests')).getByRole('status')
    expect(status.querySelector('bdi[data-outside]')?.textContent).toBe('NOT_FOUND: no such request')
  })

  it('remove is two unhurried clicks, and the outcome says what was removed — never "this PC\'s ID is now …"', async () => {
    await mount(state({ pcs: [pc('online')] }))
    await pastHold()
    client.peerCommand.mockImplementation(async (name: string) =>
      name === 'peer.remove' ? { id: PC_ID, name: 'office', saved: true, message: 'm' } : state(),
    )
    await click(screen.getByTestId('peers-remove'))
    await click(screen.getByTestId('peers-remove'))
    expect(client.peerCommand).not.toHaveBeenCalledWith('peer.remove', expect.anything())
    await pastHold()
    await click(screen.getByTestId('peers-remove'))
    expect(client.peerCommand).toHaveBeenCalledWith('peer.remove', { id: PC_ID })
    expect(screen.getByRole('status').textContent).toBe(t('peers.removed', { name: 'office' }))
    expect(screen.getByRole('status').textContent).not.toContain(groupId(PC_ID))
  })

  it('a removal whose save failed says it holds for this run only', async () => {
    await mount(state({ pcs: [pc('online')] }))
    await pastHold()
    client.peerCommand.mockImplementation(async (name: string) =>
      name === 'peer.remove' ? { id: PC_ID, name: 'office', saved: false, message: 'm' } : state(),
    )
    await click(screen.getByTestId('peers-remove'))
    await pastHold()
    await click(screen.getByTestId('peers-remove'))
    expect(screen.getByRole('status').textContent).toBe(t('peers.removedNotSaved', { name: 'office' }))
  })

  it('add sends the ID and name, and a refused add keeps what was typed', async () => {
    await mount(state())
    await click(screen.getByTestId('peers-add'))
    const form = screen.getByTestId('peers-add-form')
    fireEvent.change(within(form).getByLabelText(t('peers.id')), { target: { value: PC_ID } })
    fireEvent.change(within(form).getByLabelText(t('peers.name')), { target: { value: 'office' } })
    client.peerCommand.mockImplementation(async (name: string) => {
      if (name === 'peer.add') throw new Error('CONFLICT: the name is taken')
      return state()
    })
    await click(screen.getByTestId('peers-add-save'))
    expect(client.peerCommand).toHaveBeenCalledWith('peer.add', { id: PC_ID, name: 'office' })
    expect((within(screen.getByTestId('peers-add-form')).getByLabelText(t('peers.id')) as HTMLInputElement).value).toBe(PC_ID)
    const status = screen.getByRole('status')
    expect(status.querySelector('bdi[data-outside]')?.textContent).toBe('CONFLICT: the name is taken')

    client.peerCommand.mockImplementation(async (name: string) => (name === 'peer.add' ? { outcome: 'applied' } : state()))
    await click(screen.getByTestId('peers-add-save'))
    expect(screen.queryByTestId('peers-add-form')).toBeNull()
  })

  it('a knock opens the add form with the ID and asks to check it out of band', async () => {
    await mount(state({ knocks: [{ id: PC_ID, count: 3, last_at_ms: 0 }] }))
    await click(screen.getByText(t('peers.knockAdd')))
    const form = screen.getByTestId('peers-add-form')
    expect(within(form).getByRole('note').textContent).toBe(t('peers.knockConfirm'))
    expect((within(form).getByLabelText(t('peers.id')) as HTMLInputElement).value).toBe(PC_ID)
  })

  it('regenerate needs a second click, warns about re-pairing and reports the new ID', async () => {
    await mount(state())
    await pastHold()
    await click(screen.getByTestId('peers-regenerate'))
    expect(screen.getByText(t('peers.regenerateConfirm'))).toBeTruthy()
    expect(client.peerCommand).not.toHaveBeenCalledWith('peer.regenerateKey', expect.anything())
    client.peerCommand.mockImplementation(async (name: string) =>
      name === 'peer.regenerateKey' ? { outcome: 'applied', message: 'm', request: null, id: PC_ID } : state(),
    )
    await pastHold()
    await click(screen.getByTestId('peers-regenerate'))
    expect(client.peerCommand).toHaveBeenCalledWith('peer.regenerateKey', {})
    expect(screen.getByRole('status').textContent).toBe(t('peers.regenerated', { id: groupId(PC_ID) }))
  })

  it.each(['key_transient', 'adopt_or_fresh', 'unavailable', 'not_started'])(
    'regenerate is not offered while the state is %s',
    async word => {
      await mount(state({ state: word }))
      expect(screen.queryByTestId('peers-regenerate')).toBeNull()
    },
  )

  it('a notice follows a language switch (kept as data, worded at render)', async () => {
    await mount(state({ requests: [request('add')] }))
    await pastHold()
    await click(screen.getByTestId('peers-approve'))
    const status = () => within(screen.getByTestId('peers-requests')).getByRole('status').textContent
    expect(status()).toBe(ko.peers.done)
    act(() => {
      setLanguage('en')
    })
    expect(status()).toBe(en.peers.done)
  })
})

describe('the store', () => {
  it('a read answer that comes after a broadcast does not overwrite it', async () => {
    let answer: (s: PeerState) => void = () => {}
    client.peerCommand.mockImplementation(() => new Promise(resolve => (answer = resolve as (s: PeerState) => void)))
    const reading = usePeerStore.getState().read()
    usePeerStore.getState().receive(state({ id: 'from-broadcast' }))
    answer(state({ id: 'from-read' }))
    await reading
    expect(usePeerStore.getState().state?.id).toBe('from-broadcast')
  })
})
