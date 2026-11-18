// RichSlot (live mode) send() failure path regression — local UI-state error handling (WIRE unchanged ADR-0044/45/46).
//
// send() sets awaiting=true at once (FIX 5b); if writeStdin rejects, no response comes and derived streaming (= awaiting || (!turnDone && items.length>0)) stays true, so the indicator sticks.
// Fix: setAwaiting(false) in catch; this test observes that recovery.
//
// ★Observation surface★: the only visual signal of streaming is the "Wait" label (`chat/WaitRow.tsx`) above the input box. The subscription callback is captured and fed
//   TextDelta + MessageDone → items=[text,separator] & turnDone=true, so streaming narrows to awaiting and the "Wait" indicator mirrors awaiting.
//
// Strategy: stub agentClient (clientFactory) and agentStore like slotTagGate.test.tsx; capture the subscribeOutput callback (onChunk) to inject tag1 chunks, and swap writeStdin between reject/resolve.

import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { FRAME_TAG_STRUCTURED_EVENT } from '../../api/wsFrame'
import { INPUT_LOCKED_REFUSAL, type OutputChunk, type ReplayLiveInfo, type ViewPhase } from '../../api/agentClient'
import { t } from '../../i18n'
import '../../commands/chatCommands' // side-effect register — measures whether the tool-bundle command reaches the mounted slot.
import '../../commands/agentCommands' // side-effect register — carries the interrupt command all the way to the path that sets 「interrupting」 (ADR-0244).
import { run, type CommandArgs } from '../../commands/registry'
import { useToolGroupStore } from '../../store/toolGroupStore'
import { pendingInterrupt, useInterruptStore } from '../../store/interruptStore'
import { getFollow } from './scrollFollow/followRegistry'
import { JUMP_BUTTON_DELAY_MS } from './scrollFollow/JumpToBottom'

// ── subscribeOutput callback capture + writeStdin holder (swapped per test). ──
const captured = vi.hoisted(() => ({
  onChunk: null as ((c: OutputChunk) => void) | null,
  onState: null as ((s: ViewPhase, info?: ReplayLiveInfo) => void) | null,
  onReset: null as (() => void) | null,
}))
const clientMock = vi.hoisted(() => ({
  writeStdin: vi.fn(async () => undefined) as (id: string, bytes: Uint8Array) => Promise<void>,
  // ADR-0231 re-attach check — called on every 'live' (marker carried). The test swaps it to release the reply late.
  listQueuedInputs: vi.fn(async () => ({ inputs: [], as_of_seq: null, epoch: 0, stopped_after_error: false })) as (
    id: string,
  ) => Promise<unknown>,
  interruptAgent: vi.fn(async () => undefined) as (id: string) => Promise<void>,
  // Connection state surface (half of the ADR-0148 absent decision) — follows the real contract of firing once with the current state on registration.
  connectionState: 'connected' as 'connected' | 'reconnecting' | 'down',
  stateCbs: new Set<(s: 'connected' | 'reconnecting' | 'down') => void>(),
}))

vi.mock('../../api/clientFactory', () => ({
  agentClient: {
    // ADR-0046 signature (viewId, agentId, onChunk, onState?, onReset?).
    subscribeOutput: vi.fn(
      async (
        _viewId: string,
        _agentId: string,
        onChunk: (c: OutputChunk) => void,
        onState?: (s: ViewPhase, info?: ReplayLiveInfo) => void,
        onReset?: () => void,
      ) => {
        captured.onChunk = onChunk
        captured.onState = onState ?? null
        captured.onReset = onReset ?? null
        return { unsubscribe: vi.fn() }
      },
    ),
    writeStdin: (id: string, bytes: Uint8Array) => clientMock.writeStdin(id, bytes),
    listQueuedInputs: (id: string) => clientMock.listQueuedInputs(id),
    interruptAgent: (id: string) => clientMock.interruptAgent(id),
    resizePty: vi.fn(async () => undefined),
    get connectionState() {
      return clientMock.connectionState
    },
    onConnectionStateChange: (cb: (s: 'connected' | 'reconnecting' | 'down') => void) => {
      clientMock.stateCbs.add(cb)
      cb(clientMock.connectionState) // Same as the real one: notifies the current state once on registration.
      return () => clientMock.stateCbs.delete(cb)
    },
  },
  getAgentClient: vi.fn(),
}))

// ── Human-path command calls (ADR-0237 Esc interrupt) — checks only what was called, not execution. ──
const dispatchMock = vi.hoisted(() => ({ fireAndForget: vi.fn() }))
vi.mock('../../commands/dispatch', () => ({
  fireAndForget: (...args: unknown[]) => dispatchMock.fireAndForget(...args),
}))
// The interrupt command module imports the folder dialog — these tests do not call that door.
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }))

// ── agentStore stub — the slot queries agents and agentsLoaded for the absent decision. ──
// agentsLoaded=false by default = "authoritative registry not received" → an empty list is not mistaken for absent (ADR-0148 guard).
const agentStoreState = vi.hoisted(() => ({
  agents: [] as unknown[],
  agentsLoaded: false,
}))
vi.mock('../../store/agentStore', () => ({
  useAgentStore: (selector: (s: typeof agentStoreState) => unknown) => selector(agentStoreState),
}))

// ── Test subject ────────────────────────────────────────────────────────────────
import RichSlot from './RichSlot'

const AGENT = 'aaaa-bbbb-cccc-dddd'
const enc = new TextEncoder()

/** tag1 = StructuredEvent JSON chunk (the only tag the structured slot consumes). */
function tag1(seq: number, json: string): OutputChunk {
  return { seq, tag: FRAME_TAG_STRUCTURED_EVENT, bytes: enc.encode(json) }
}

/** Flushes the subscribeOutput/writeStdin async microtasks (subscription registration, write reject taking effect). */
async function flush(): Promise<void> {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
}

/** Injects 1 completed turn (TextDelta → MessageDone): items=[text,separator], turnDone=true, so streaming narrows to awaiting and "Wait" reflects awaiting as is. */
function feedCompletedTurn(): void {
  act(() => captured.onChunk!(tag1(0, JSON.stringify({ type: 'TextDelta', text: 'assistant reply' }))))
  act(() => captured.onChunk!(tag1(1, JSON.stringify({ type: 'MessageDone' }))))
}

/** Connection state transition (same shape as the path where the real ProtocolClient notifies all subscribers). */
function setConnection(state: 'connected' | 'reconnecting' | 'down'): void {
  clientMock.connectionState = state
  act(() => {
    for (const cb of clientMock.stateCbs) cb(state)
  })
}

/** Absent overlay (ADR-0148 decisions 2·3) — blur curtain + symbol. Visual values (opacity, size) are not asserted. */
function deadOverlay(): HTMLElement | null {
  return document.querySelector('[data-slot-dead="1"]')
}

beforeEach(() => {
  captured.onChunk = null
  captured.onState = null
  captured.onReset = null
  clientMock.writeStdin = vi.fn(async () => undefined)
  clientMock.listQueuedInputs = vi.fn(async () => ({
    inputs: [],
    as_of_seq: null,
    epoch: 0,
    stopped_after_error: false,
  }))
  clientMock.interruptAgent = vi.fn(async () => undefined)
  clientMock.connectionState = 'connected'
  clientMock.stateCbs.clear()
  useInterruptStore.setState({ pending: {}, views: {} })
  agentStoreState.agents = []
  agentStoreState.agentsLoaded = false
  dispatchMock.fireAndForget.mockReset()
  // The expansion store is module-global — every test uses the same slot · agent (v1 · AGENT), so a prior test's chosen expansion carries over.
  useToolGroupStore.setState({ bySlot: {} })
})

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

describe('RichSlot(live) — awaiting released when send() fails', () => {
  it('if writeStdin rejects the "Wait" streaming signal does not stick and returns to idle', async () => {
    // console.warn is the fix's error surface — silenced both to keep test logs clean and to observe calls.
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    clientMock.writeStdin = vi.fn(async () => {
      throw new Error('write failed')
    })

    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    expect(captured.onChunk).toBeTruthy()

    // Inject one completed turn → turnDone=true & content present. In this state streaming = awaiting.
    feedCompletedTurn()
    expect(screen.queryByText(t('chat.wait'))).toBeNull()

    // send() turns the streaming hint on at once with awaiting=true.
    const textarea = screen.getByPlaceholderText(/메시지 입력/)
    fireEvent.change(textarea, { target: { value: 'hello' } })
    fireEvent.keyDown(textarea, { key: 'Enter' })

    expect(clientMock.writeStdin).toHaveBeenCalledTimes(1)

    // Flush the microtask for the reject taking effect (catch → setAwaiting(false)).
    await flush()

    expect(screen.queryByText(t('chat.wait'))).toBeNull()
    expect(warn).toHaveBeenCalled()
  })

  it('if writeStdin succeeds (before a response) awaiting stays so the "Wait" signal turns on (control)', async () => {
    clientMock.writeStdin = vi.fn(async () => undefined) // success = resolve

    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()

    feedCompletedTurn()
    expect(screen.queryByText(t('chat.wait'))).toBeNull()

    const textarea = screen.getByPlaceholderText(/메시지 입력/)
    fireEvent.change(textarea, { target: { value: 'hello' } })
    fireEvent.keyDown(textarea, { key: 'Enter' })
    await flush()

    // Success path — there is no response event yet, so streaming is kept by the awaiting bridge.
    expect(screen.getByText(t('chat.wait'))).toBeTruthy()
  })
})

// The wait indicator stands outside the scroll list, on an always-present row right above the input box.
describe('RichSlot(live) — waiting indicator line', () => {
  it('during a conversation the line is always up, outside the scroll list · right above the input group — the indicator is inside only while spinning', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    feedCompletedTurn()
    const strip = document.querySelector('[data-wait-strip="1"]') as HTMLElement
    const viewport = document.querySelector('[data-radix-scroll-area-viewport]') as HTMLElement
    const textarea = screen.getByPlaceholderText(/메시지 입력/)
    const label = document.querySelector('[data-rich-label="1"]') as HTMLElement
    expect(strip).not.toBeNull()
    expect(viewport.contains(strip)).toBe(false)
    expect(strip.nextElementSibling?.contains(textarea)).toBe(true)
    expect(strip.contains(label)).toBe(true) // The identity label is this row's right-hand area — it does not float and overlap
    expect(label.className).not.toContain('absolute')
    expect(screen.queryByText(t('chat.wait'))).toBeNull()

    fireEvent.change(textarea, { target: { value: 'hello' } })
    fireEvent.keyDown(textarea, { key: 'Enter' })
    await flush()
    const wait = screen.getByText(t('chat.wait'))
    expect(document.querySelector('[data-wait-strip="1"]')).toBe(strip)
    expect(strip.contains(wait)).toBe(true)
    expect(viewport.contains(wait)).toBe(false)
    // The wait text stands on the left and the label on the right in one line, and the label is the same node even when the wait indicator turns on.
    expect(wait.compareDocumentPosition(label) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    expect(document.querySelector('[data-rich-label="1"]')).toBe(label)
    // The input box is the same node even when the row is inserted (textarea comment — the same element in both layouts).
    expect(screen.getByPlaceholderText(/메시지 입력/)).toBe(textarea)
  })

  it('the first-run screen (empty state) has neither the line nor the label', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    act(() => captured.onState!('live', { continuesConversation: false }))
    expect(emptyState()).not.toBeNull()
    expect(document.querySelector('[data-wait-strip="1"]')).toBeNull()
    expect(document.querySelector('[data-rich-label="1"]')).toBeNull() // The label collapse in the empty state is unchanged
  })
})

// ★Follow-up send flicker regression★: on a send after a completed turn (turnDone=true) the synthetic user echo arrives before the first assistant token; "Wait" must hold when that echo clears awaiting
//   (accumulator: user Structured lowers turnDone to false). Before the fix streaming = awaiting(false) || (!turnDone(true) && items>0) = false and "Wait" flickered off.
describe('RichSlot(live) — a synthetic user echo on a follow-up send does not switch off "Wait" (flicker FIX)', () => {
  it('send after a completed turn → "Wait" stays even after the user echo arrives', async () => {
    clientMock.writeStdin = vi.fn(async () => undefined) // success = resolve

    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()

    // Previous turn complete (turnDone=true) + content present.
    feedCompletedTurn()
    expect(screen.queryByText(t('chat.wait'))).toBeNull()

    // Follow-up send — streaming hint on with awaiting=true.
    const textarea = screen.getByPlaceholderText(/메시지 입력/)
    fireEvent.change(textarea, { target: { value: 'follow-up' } })
    fireEvent.keyDown(textarea, { key: 'Enter' })
    await flush()
    expect(screen.getByText(t('chat.wait'))).toBeTruthy()

    // The synthetic user echo arrives before the first assistant token (right after write_input in json mode); the callback does setAwaiting(false) but the accumulator lowers turnDone, so streaming holds.
    act(() =>
      captured.onChunk!(
        tag1(2, JSON.stringify({
          type: 'Structured',
          kind: 'user',
          json: JSON.stringify({ type: 'text', text: 'follow-up', uuid: 'U' }),
        })),
      ),
    )

    expect(screen.getByText(t('chat.wait'))).toBeTruthy()
  })
})

// ★A frame that was not understood does not turn off the wait indicator★ — visible only over **multiple turns**: a fresh slot has turnDone false, so "Wait" stands for any frame.
//   Defect: after a turn closed (turnDone=true) a new send goes out and the first frame is of an unknown kind — the callback clears awaiting, turnDone stays true, and "Wait" goes off mid-response.
describe('RichSlot(live) — a frame of an unknown kind does not switch off "Wait"', () => {
  it('send after a completed turn → "Wait" stays even if that turn\'s first frame is an unknown kind', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    clientMock.writeStdin = vi.fn(async () => undefined)

    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()

    feedCompletedTurn() // turnDone=true + content present
    expect(screen.queryByText(t('chat.wait'))).toBeNull()

    const textarea = screen.getByPlaceholderText(/메시지 입력/)
    fireEvent.change(textarea, { target: { value: 'follow-up' } })
    fireEvent.keyDown(textarea, { key: 'Enter' })
    await flush()
    expect(screen.getByText(t('chat.wait'))).toBeTruthy()

    // A kind sent by a newer daemon that is not in this shell's bindings.
    act(() => captured.onChunk!(tag1(2, JSON.stringify({ type: 'SomethingNewer', whatever: 1 }))))
    expect(screen.getByText(t('chat.wait'))).toBeTruthy()

    // When a known frame arrives, control of the indicator passes to turnDone, and the boundary turns it off normally.
    act(() => captured.onChunk!(tag1(3, JSON.stringify({ type: 'TextDelta', text: 'answer' }))))
    expect(screen.getByText(t('chat.wait'))).toBeTruthy()
    act(() => captured.onChunk!(tag1(4, JSON.stringify({ type: 'MessageDone' }))))
    expect(screen.queryByText(t('chat.wait'))).toBeNull()
    expect(warn).toHaveBeenCalled() // The diagnostic name goes to console only
  })

  it('"Wait" does not stick even if the end of a restored history is an unknown kind (no send)', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()

    feedCompletedTurn()
    act(() => captured.onChunk!(tag1(2, JSON.stringify({ type: 'SomethingNewer', whatever: 1 }))))

    expect(screen.queryByText(t('chat.wait'))).toBeNull()
    expect(warn).toHaveBeenCalled()
  })
})

// ★A retryable stream error does not turn off "Wait"★ — codex reports overload and rate limits as a single `Error` line and the same turn continues; claude closes a failed turn with `MessageDone`,
//   so counting `Error` as a turn end was wrong on both sides (before the fix "Wait" went off on one error line and came back on the next delta).
describe('RichSlot(live) — an Error mid-turn does not switch off "Wait"', () => {
  it('"Wait" stays throughout delta → Error → delta, and only the boundary switches it off', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()

    act(() => captured.onChunk!(tag1(0, JSON.stringify({ type: 'TextDelta', text: 'thinking' }))))
    expect(screen.getByText(t('chat.wait'))).toBeTruthy()

    act(() =>
      captured.onChunk!(
        tag1(1, JSON.stringify({ type: 'Error', message: 'overloaded (serverOverloaded) [willRetry]' })),
      ),
    )
    expect(screen.getByText(t('chat.wait'))).toBeTruthy() // Before the fix: it went off here

    act(() => captured.onChunk!(tag1(2, JSON.stringify({ type: 'TextDelta', text: ' resumed' }))))
    expect(screen.getByText(t('chat.wait'))).toBeTruthy()

    act(() => captured.onChunk!(tag1(3, JSON.stringify({ type: 'MessageDone' }))))
    expect(screen.queryByText(t('chat.wait'))).toBeNull()
  })
})

// ★ADR-0145 empty state★: the JSON-mode first-run screen (mascot + "Claude Code" + centered input box).
//
// Core regression: the display gate is not "0 items" but "restore-complete signal ('live') + 0 items" — the view clears and subscribes on mount, so a session with history has 0 items before restore ends and
//   0 items alone would flash the intro then flip to the conversation. The signal is the state callback of subscribeOutput.
//
// Observation surface: [data-rich-empty] · mascot [data-rich-mascot] · identity label only in the bottom layout [data-rich-label] · input box className (empty rounded-xl ↔ bottom flex-1).

/** Empty-state container (mascot + text). null = the existing (conversation) layout. */
function emptyState(): HTMLElement | null {
  return document.querySelector('[data-rich-empty="1"]')
}

function mascot(): HTMLElement | null {
  return document.querySelector('[data-rich-mascot="1"]')
}

/** The input box is the same element in both layouts, so there is always exactly 1 in the document (this function also upholds that premise along the way). */
function textarea(): HTMLTextAreaElement {
  const els = document.querySelectorAll('textarea')
  if (els.length !== 1) throw new Error(`there must be exactly 1 textarea — actually ${els.length}`)
  return els[0]
}

/** Fires the replay phase callback (valid only after subscription registration). */
function fireState(state: ViewPhase): void {
  act(() => captured.onState!(state))
}

/** Fires the clearing signal — the real one calls it in the same tick **just before** delivering the new incarnation's full replay. */
function fireReset(): void {
  act(() => captured.onReset!())
}

describe('RichSlot(live) — ADR-0145 first-run empty state', () => {
  it('during restore (live not fired) the empty state is not drawn even with 0 items', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()

    // Passing the 4th argument (state callback) is itself the regression target — if it is not passed, the gate never opens.
    expect(captured.onState).toBeTruthy()
    expect(emptyState()).toBeNull()
    expect(screen.queryByText('Claude Code')).toBeNull()

    fireState('buffering')
    expect(emptyState()).toBeNull()
  })

  it("'live' + 0 items draws the mascot · text · rounded input box", async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')

    expect(emptyState()).not.toBeNull()
    expect(mascot()).not.toBeNull()
    expect(screen.getByText('Claude Code')).toBeTruthy()
    expect(textarea().className).toContain('rounded-xl')
    // The mascot is pure decoration, so it is hidden from the accessibility tree (ADR-0146 invariant) — a screen reader does not read the dots.
    expect(mascot()?.getAttribute('aria-hidden')).toBe('true')
  })

  it("ending in 'error' does not draw the empty state even with 0 items (the current empty screen is kept)", async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('error')

    expect(emptyState()).toBeNull()
  })

  // ★Re-attach regression (pairs with protocolClient's buffering notification)★: after a daemon disconnect/reattach the view goes live → buffering and receives a full replay; if that notification is not heard
  //   and 'restore complete' stays latched, the mascot flips to the conversation when history pours in.
  it("if 'buffering' comes again after live the empty state is taken down (blocks re-replay flicker)", async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')
    expect(emptyState()).not.toBeNull()

    fireState('buffering')
    expect(emptyState()).toBeNull()

    // History arrives by re-replay — the empty state is already down, so the transition is not noticeable.
    act(() => captured.onChunk!(tag1(0, JSON.stringify({ type: 'TextDelta', text: 'restored' }))))
    expect(emptyState()).toBeNull()
  })

  // ★Re-replay accumulation regression★: the clearing signal means another incarnation numbers frames from 0 and the subscription dropped its cursor. Without clearing the accumulator, history piles over the
  //   current conversation; without resetting the local seq guard, all those frames are dropped and the conversation never returns.
  it('onReset empties the conversation and redraws the new incarnation as a single set only', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    // Passing the clearing callback is itself the regression target — if it is not passed, even overlapping drawing leaves no error anywhere.
    expect(captured.onReset).toBeTypeOf('function')
    act(() => captured.onChunk!(tag1(0, JSON.stringify({ type: 'TextDelta', text: 'session one' }))))
    fireState('live')
    expect(screen.getAllByText('session one')).toHaveLength(1)

    fireReset()
    expect(screen.queryByText('session one')).toBeNull()

    // A restarted daemon's new incarnation — seq comes again from 0.
    act(() => captured.onChunk!(tag1(0, JSON.stringify({ type: 'TextDelta', text: 'session one' }))))
    fireState('live')
    expect(screen.getAllByText('session one')).toHaveLength(1)
  })

  // ★Not cleared on a phase notification★: when only the socket flickers, the client keeps its cursor and dedup absorbs the overlapping front; that span is also 'buffering', so clearing there would
  //   erase and redraw the conversation on every brief disconnect.
  it.each<ViewPhase>(['detached', 'buffering', 'live', 'error'])(
    "'%s' phase leaves the conversation as is",
    async (phase) => {
      render(<RichSlot viewId="v1" agentId={AGENT} />)
      await flush()
      act(() =>
        captured.onChunk!(tag1(0, JSON.stringify({ type: 'TextDelta', text: 'session one' }))),
      )
      fireState('live')

      fireState(phase)
      expect(screen.getAllByText('session one')).toHaveLength(1)
    },
  )

  // ★Clearing also takes down the send traces★: history of **another incarnation** is coming, so an earlier send was received by a gone incarnation and its reply never comes; if left, the new incarnation
  //   with 0 history items does not show the first-run screen and the wait indicator sticks.
  //   ★This signal does not come on resuming the same incarnation★ — the wait indicator there is measured by the phase case right below.
  it('onReset also takes down the send trace and returns the new incarnation to the first-run screen', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')
    expect(emptyState()).not.toBeNull()

    fireEvent.change(textarea(), { target: { value: 'hello' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()
    expect(emptyState()).toBeNull()
    expect(screen.getByText(t('chat.wait'))).toBeTruthy() // wait indicator (awaiting)

    // Another incarnation attached — if it ends with 0 history items, the first-run screen is correct.
    fireReset()
    fireState('live')
    expect(emptyState()).not.toBeNull()
  })

  // Opposite direction: in the span of re-attaching to the same incarnation (no clearing signal) the wait indicator must stay alive —
  //   a send that went out is not cancelled by re-attach.
  it('moving only between phases without clearing does not keep the send trace (waiting indicator kept)', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')
    fireEvent.change(textarea(), { target: { value: 'hello' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()

    fireState('buffering')
    fireState('live')
    expect(emptyState()).toBeNull()
    expect(screen.getByText(t('chat.wait'))).toBeTruthy()
  })

  // While the registry has no such agent the slot must not look "alive but quiet" (ADR-0148 absent curtain also applies to detached).
  it("'detached' raises the absence veil", async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    expect(deadOverlay()).toBeNull()

    fireState('detached')
    expect(deadOverlay()).not.toBeNull()
  })

  // Same for a view that exhausted the re-request ladder — it can receive nothing, and with no indicator it becomes a quiet blank panel.
  it("'error' also raises the absence veil (same shape across the three slots)", async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('error')
    expect(deadOverlay()?.getAttribute('data-slot-phase')).toBe('error')
  })

  it("even with 'live', if there are conversation items it is not the empty state", async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    feedCompletedTurn()
    fireState('live')

    expect(emptyState()).toBeNull()
    expect(textarea().className).toContain('flex-1')
  })

  it('on the first send the empty state disappears at once and the input box returns to the bottom placement — the same element (no remount)', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')
    expect(emptyState()).not.toBeNull()

    const before = textarea()
    before.focus()
    fireEvent.change(before, { target: { value: 'hello' } })
    fireEvent.keyDown(before, { key: 'Enter' })
    await flush()

    expect(emptyState()).toBeNull()
    // The two layouts must be one element so input focus does not break during the transition (ADR-0145 §5).
    expect(textarea()).toBe(before)
    expect(document.activeElement).toBe(before)
    expect(before.className).toContain('flex-1')
    expect(document.querySelector('[data-rich-label="1"]')).not.toBeNull()
  })

  // Resubscribe runs only when the assignment changes to a different agent (a re-spawn does not replace the subscription — see the 'identity' describe). If that inherited the restore-complete mark, the empty state
  //   would show before the new agent's history arrives and then flip to the conversation.
  it('if the assignment changes to another agent the restore-complete flag goes down, so until live fires again it is not the empty state', async () => {
    const { rerender } = render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')
    expect(emptyState()).not.toBeNull()

    rerender(<RichSlot viewId="v1" agentId="eeee-ffff-0000-1111" />)
    await flush()
    expect(emptyState()).toBeNull()

    fireState('live') // The new subscription's state callback
    expect(emptyState()).not.toBeNull()
  })

  // Text differs per layout: the empty state shows the greeting, the bottom the Enter/Shift+Enter hint; the shared element hides a broken key branch from the screen, so pin it here.
  it('placeholder uses different text for the empty state and the bottom placement', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')

    expect(textarea().placeholder).toBe(t('agent.emptyInputPlaceholder'))
    expect(textarea().placeholder).toBe('메시지를 입력하세요')

    fireEvent.change(textarea(), { target: { value: 'hi' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()

    expect(emptyState()).toBeNull()
    expect(textarea().placeholder).toBe(t('agent.inputPlaceholder'))
  })

  it('an exited agent: the exit text wins even in the empty-state placement', async () => {
    agentStoreState.agents = [{ id: AGENT, cwd: 'C:/x', status: { type: 'Exited' } }]
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')

    // The layout is the empty state (mascot and text shown), but only the hint goes to the ended side.
    expect(emptyState()).not.toBeNull()
    expect(textarea().placeholder).toBe(t('agent.terminatedPlaceholder'))
  })

})

// ★Why the ADR-0145 empty-state gate is "already sent on this subscription", not awaiting★
//
// awaiting is also cleared (a) on tag1 frames that create no item (empty TextDelta · user uuid dedup skip · empty MessageDone) and (b) by the writeStdin failure catch without knowing which send it belongs to.
// Both leave a span "waiting for a response but items is 0"; reviving the mascot there would cover the conversation with the first screen.
describe('RichSlot(live) — ADR-0145 empty-state suppression follows the fact of sending (not awaiting)', () => {
  it('even if a frame that makes no item (an empty delta) comes after a send it does not return to the empty state', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')
    expect(emptyState()).not.toBeNull()

    fireEvent.change(textarea(), { target: { value: 'hello' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()
    expect(emptyState()).toBeNull()

    // Empty delta = a frame the accumulator lets through without an item (TextDelta arm); the callback clears awaiting on it too → with an awaiting gate the mascot would return here.
    act(() => captured.onChunk!(tag1(0, JSON.stringify({ type: 'TextDelta', text: '' }))))
    expect(emptyState()).toBeNull()
  })

  it('a late failure of a send made to the previous incarnation does not revert the new incarnation\'s send mark', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    // Send A — holds a promise with no outcome yet and rejects it later.
    let rejectA!: (e: Error) => void
    clientMock.writeStdin = vi.fn(
      () =>
        new Promise<void>((_, rej) => {
          rejectA = rej
        }),
    )

    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')
    fireEvent.change(textarea(), { target: { value: 'send A' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()

    // Another incarnation attached (re-spawn) → clearing takes down send traces, live arrives again, back to the empty state.
    //   ★The instance stays the same★ (a re-spawn does not replace the component), so a failure carried across lands on the same instance; the sequence guard below blocks it.
    fireReset()
    fireState('live')
    expect(emptyState()).not.toBeNull()

    // Send B (new incarnation) — success path.
    clientMock.writeStdin = vi.fn(async () => undefined)
    fireEvent.change(textarea(), { target: { value: 'send B' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()
    expect(emptyState()).toBeNull()

    // A was a send to the vanished incarnation: it must not touch any indicator on the current screen (neither back to the empty state nor turning off B's wait indicator).
    rejectA(new Error('write failed'))
    await flush()
    expect(emptyState()).toBeNull()
    expect(screen.getByText(t('chat.wait'))).toBeTruthy()
    expect(warn).toHaveBeenCalled()
  })

  it('even in the same instance a late failure of an earlier send does not erase the latest send\'s waiting indicator', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    let rejectA!: (e: Error) => void
    clientMock.writeStdin = vi.fn(
      () =>
        new Promise<void>((_, rej) => {
          rejectA = rej
        }),
    )

    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')

    // Send A — outcome undecided.
    fireEvent.change(textarea(), { target: { value: 'send A' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()
    expect(screen.getByText(t('chat.wait'))).toBeTruthy()

    // Send B (success) — the latest send passes to B.
    clientMock.writeStdin = vi.fn(async () => undefined)
    fireEvent.change(textarea(), { target: { value: 'send B' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()

    // Belated A failure — it is not the latest send, so it touches no indicator (B is still waiting for a response).
    rejectA(new Error('write failed'))
    await flush()
    expect(screen.getByText(t('chat.wait'))).toBeTruthy()
    expect(emptyState()).toBeNull()
    expect(warn).toHaveBeenCalled()
  })

  it('if the first send fails it returns to the empty state — nothing actually went out', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    clientMock.writeStdin = vi.fn(async () => {
      throw new Error('write failed')
    })

    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')
    expect(emptyState()).not.toBeNull()

    fireEvent.change(textarea(), { target: { value: 'hello' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    // At send time it is taken down optimistically (to the conversation screen while waiting for success).
    expect(emptyState()).toBeNull()

    // It comes back once the failure is confirmed — if not, the first-run screen never shows until resubscribe.
    await flush()
    expect(emptyState()).not.toBeNull()
    expect(screen.queryByText(t('chat.wait'))).toBeNull()
    expect(warn).toHaveBeenCalled()
  })

  it('if there was a successful send, it does not return to the empty state even if a later send fails', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    clientMock.writeStdin = vi.fn(async () => undefined)

    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')

    // A: success (no response yet so items is 0 — a span where the revert decision cannot be settled by items alone).
    fireEvent.change(textarea(), { target: { value: 'send A' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()

    // B: failure. A's response may be on its way, so it must not go back to the first-run screen.
    clientMock.writeStdin = vi.fn(async () => {
      throw new Error('write failed')
    })
    fireEvent.change(textarea(), { target: { value: 'send B' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()

    expect(emptyState()).toBeNull()
    expect(warn).toHaveBeenCalled()
  })

  // ★A success carried across a clearing is not a send fact of the new incarnation★: if the incarnation changes while send A is in flight, A's outcome belongs to the vanished incarnation. If that success set
  //   "a send went out from this instance", a real failed send in the new incarnation could not return to the first-run screen. The sequence guard must cover both success and failure, and clearing must bump it too.
  it('a success of the previous incarnation\'s send arriving after clearing does not block the new incarnation\'s failure revert', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    // Send A — the test holds its outcome and ends it as a success **after** the clearing.
    let resolveA!: () => void
    clientMock.writeStdin = vi.fn(
      () =>
        new Promise<void>((res) => {
          resolveA = res
        }),
    )

    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')
    fireEvent.change(textarea(), { target: { value: 'send A' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()
    expect(emptyState()).toBeNull()

    // Another incarnation attached — the send traces are taken down and it returns to the first-run screen (the instance stays the same).
    fireReset()
    fireState('live')
    expect(emptyState()).not.toBeNull()

    // A, sent to the vanished incarnation, now ends as a success.
    resolveA()
    await flush()

    // B, sent to the new incarnation, fails — this incarnation received nothing, so the first-run screen is correct.
    clientMock.writeStdin = vi.fn(async () => {
      throw new Error('write failed')
    })
    fireEvent.change(textarea(), { target: { value: 'send B' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()

    expect(emptyState()).not.toBeNull()
    expect(screen.queryByText(t('chat.wait'))).toBeNull()
    expect(warn).toHaveBeenCalled()
  })
})

// ★ADR-0148: a slot with no targeted agent★
//
// Process exit and connection loss are treated the same. Content is not erased (conversation or first-run screen stays as it was) — only a blur curtain + symbol is laid on; a dead slot looking like "just start now" was the defect.
describe('RichSlot(live) — ADR-0148 agent absence expression', () => {
  it('an exited agent: does not erase the conversation, only lays a dimming veil + symbol', async () => {
    agentStoreState.agents = [{ id: AGENT, cwd: 'C:/x', status: { type: 'Killed' } }]
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    feedCompletedTurn()

    // Content retained + overlay present must both hold on one screen.
    expect(screen.getByText('assistant reply')).toBeTruthy()
    expect(deadOverlay()).not.toBeNull()
    // Symbol only — no text or banner is put in (a decision not to leave the information hard-coded).
    expect(deadOverlay()?.textContent).toBe('')
    expect(deadOverlay()?.querySelector('svg')).not.toBeNull()
    // The input box keeps its existing disabled handling.
    expect(textarea().disabled).toBe(true)
    expect(textarea().placeholder).toBe(t('agent.terminatedPlaceholder'))
  })

  it('even when it exits from the empty state (0 items) the dimming veil covers while the mascot and text stay', async () => {
    agentStoreState.agents = [{ id: AGENT, cwd: 'C:/x', status: { type: 'Exited' } }]
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')

    expect(emptyState()).not.toBeNull()
    expect(mascot()).not.toBeNull()
    expect(screen.getByText('Claude Code')).toBeTruthy()
    expect(deadOverlay()).not.toBeNull()
  })

  // ★The real outcome of a kill★: the reaper collects the session and **removes it from the registry** (the profile stays as a corpse), so status does not catch it; an id still unresolved after the registry is
  //   received is absent. Otherwise the dead agent's input box would stay active.
  it('an agent that cannot be resolved after the roster is received is also seen as absent — content kept + overlay + input blocked', async () => {
    // Builds a state where a conversation accumulated while alive (present in the registry).
    agentStoreState.agents = [{ id: AGENT, cwd: 'C:/x', status: { type: 'Running' } }]
    agentStoreState.agentsLoaded = true
    const { rerender } = render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    feedCompletedTurn()
    expect(deadOverlay()).toBeNull()
    expect(textarea().disabled).toBe(false)

    // Collection done = gone from the registry (only the profile stays — this component does not use profiles for the absent decision).
    agentStoreState.agents = []
    rerender(<RichSlot viewId="v1" agentId={AGENT} />)

    expect(screen.getByText('assistant reply')).toBeTruthy() // Content unchanged
    expect(deadOverlay()).not.toBeNull()
    expect(textarea().disabled).toBe(true)
    expect(textarea().placeholder).toBe(t('agent.terminatedPlaceholder'))
  })

  it('the span where the roster is not yet received (agentsLoaded=false) is not seen as absent', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()

    // agents is empty but the registry is not received, so the decision is held — a slot must not be killed right after boot.
    expect(deadOverlay()).toBeNull()
    expect(textarea().disabled).toBe(false)
  })

  it('when the connection drops a live agent is treated the same, and it lifts when reattached', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    feedCompletedTurn()
    expect(deadOverlay()).toBeNull()

    setConnection('down')
    expect(deadOverlay()).not.toBeNull()
    expect(screen.getByText('assistant reply')).toBeTruthy() // Content unchanged

    setConnection('reconnecting') // Still absent until attached
    expect(deadOverlay()).not.toBeNull()

    setConnection('connected')
    expect(deadOverlay()).toBeNull()
  })

  // ★Every condition for raising the curtain is a condition for blocking input (ADR-0148 decision 1 / ADR-0149 decision 4)★: the curtain is pointer-events-none and `disabled` cannot stop programmatic key events.
  //   Blocking on exit alone would let a send go out from a disconnected slot and fail silently. Here the agent is kept alive in the registry and only the connection is cut, to measure the non-exit branch.
  it('while the veil is up due to a dropped connection, sending is blocked and the input box is disabled even if the agent is alive', async () => {
    agentStoreState.agents = [{ id: AGENT, cwd: 'C:/x', status: { type: 'Running' } }]
    agentStoreState.agentsLoaded = true
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    feedCompletedTurn()
    expect(textarea().disabled).toBe(false)

    // The draft is entered while attached — after the disconnect the input box is disabled and cannot be filled.
    fireEvent.change(textarea(), { target: { value: 'hello' } })

    setConnection('down')
    expect(deadOverlay()).not.toBeNull()
    expect(textarea().disabled).toBe(true)
    ;(clientMock.writeStdin as ReturnType<typeof vi.fn>).mockClear()

    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()
    expect(clientMock.writeStdin).not.toHaveBeenCalled()
    // The draft stays — if a blocked send swallowed the input, what the user wrote would vanish.
    expect(textarea().value).toBe('hello')

    // When it reattaches, that draft goes out as it is (the guard does not stick locked).
    setConnection('connected')
    expect(textarea().disabled).toBe(false)
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()
    expect(clientMock.writeStdin).toHaveBeenCalledTimes(1)
  })
})

// ★Identity change = new instance (key remount)★
//
// If only the subscription effect cleared the list and indicators, the **first commit** after an identity change would draw new props + old state (passive effects run after paint) — a different agent in the same slot
// would flash the previous agent's conversation. Asserting after flushing effects hides this, so look directly at whether the instance was replaced (DOM node replaced = unmount then remount).
describe('RichSlot(live) — when identity changes the instance is mounted anew', () => {
  const OTHER = 'eeee-ffff-0000-1111'

  it('if agentId changes the slot root node is replaced (no commit inherits the old state)', async () => {
    const { rerender } = render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    feedCompletedTurn()
    expect(screen.getByText('assistant reply')).toBeTruthy()

    const rootBefore = document.querySelector('[data-rich-live="1"]')
    rerender(<RichSlot viewId="v1" agentId={OTHER} />)

    expect(document.querySelector('[data-rich-live="1"]')).not.toBe(rootBefore)
    expect(document.querySelector('[data-rich-live="1"]')?.getAttribute('data-agent-id')).toBe(OTHER)
    expect(screen.queryByText('assistant reply')).toBeNull()
  })

  // ★Opposite direction (do not revive the old regression)★: the incarnation number was once in this key, so a re-spawn replaced the instance and erased the conversation before the replay arrived — and the daemon
  //   discards the ring on exit, so it could not be revived. Now the same instance is kept and the clearing signal erases **when the new incarnation's history arrives** (the onReset cases above).
  it('even redrawing with the same agentId does not swap the instance (conversation preserved)', async () => {
    const { rerender } = render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    feedCompletedTurn()

    const rootBefore = document.querySelector('[data-rich-live="1"]')
    rerender(<RichSlot viewId="v1" agentId={AGENT} />)

    expect(document.querySelector('[data-rich-live="1"]')).toBe(rootBefore)
    expect(screen.getByText('assistant reply')).toBeTruthy()
  })
})

// ★ADR-0226 history wait of a resumed incarnation (D5)★
//
// When 'live' announces "this incarnation came up to resume a saved conversation", a loading overlay replaces the first-run screen. It ends on the first history row · a delivered input's bubble (ADR-0231) · absence (the curtain wins) · restart (clearing + a false 'live').
// With the marker missing or false nothing may differ from before the marker existed (the old daemon/shell path carries none).
describe('RichSlot(live) — ADR-0226 history wait of a takeover incarnation', () => {
  const OTHER = 'eeee-ffff-0000-1111'

  function loadingPanel(): HTMLElement | null {
    return document.querySelector('[data-loading-panel="1"]')
  }
  function awaitingAttr(): string | null {
    return document.querySelector('[data-rich-live="1"]')!.getAttribute('data-rich-awaiting-history')
  }
  function fireLive(continuesConversation: boolean): void {
    act(() => captured.onState!('live', { continuesConversation }))
  }
  const USAGE = JSON.stringify({ type: 'Usage', input_tokens: 10, output_tokens: 2 })

  it('marker true + 0 items → loading and a faint veil instead of the first screen, the input box active at the bottom placement', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireLive(true)

    expect(emptyState()).toBeNull()
    expect(mascot()).toBeNull()
    expect(screen.queryByText('Claude Code')).toBeNull()
    const panel = loadingPanel()
    expect(panel).not.toBeNull()
    expect(panel!.textContent).toBe('') // No hint text — just an icon
    expect(awaitingAttr()).toBe('1')
    // It does not intercept operation — scroll, selection and clicks pass through.
    expect(panel!.className).toContain('pointer-events-none')
    expect(panel!.className).toContain('bg-foreground/')
    // The curtain covers only the conversation area (scroll area). The input box is outside it.
    const conversationArea = panel!.closest('[data-radix-scroll-area-viewport]')
    expect(conversationArea).not.toBeNull()
    expect(conversationArea!.contains(textarea())).toBe(false)
    // This is the way out for input if history never comes — active + bottom layout.
    expect(textarea().disabled).toBe(false)
    expect(textarea().className).toContain('flex-1')
    expect(textarea().placeholder).toBe(t('agent.inputPlaceholder'))
    expect(screen.queryByText(t('chat.wait'))).toBeNull()
  })

  // The first live frame of a resume is usage; ending the wait on it would show an empty list before history, and using it for the wait indicator would show loading and the indicator together.
  it('while only usage that draws no row arrives loading stays and there is no waiting indicator', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    act(() => captured.onChunk!(tag1(0, USAGE))) // The replay is delivered before 'live'
    fireLive(true)
    expect(loadingPanel()).not.toBeNull()
    expect(screen.queryByText(t('chat.wait'))).toBeNull()

    act(() => captured.onChunk!(tag1(1, USAGE))) // Live usage
    expect(loadingPanel()).not.toBeNull()
    expect(screen.queryByText(t('chat.wait'))).toBeNull()
    expect(emptyState()).toBeNull()
  })

  // The wait indicator is not raised while the panel briefly goes down ('buffering' of a same-incarnation reattach) either — it would flicker panel → indicator → panel. Basis: "no history yet", not the phase.
  it("even if the panel goes down by 'buffering' while waiting, the waiting indicator does not intrude (only a turn-open usage)", async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    act(() => captured.onChunk!(tag1(0, USAGE))) // The turn stays open (turnDone=false)
    fireLive(true)
    expect(loadingPanel()).not.toBeNull()
    expect(screen.queryByText(t('chat.wait'))).toBeNull()

    fireState('buffering')
    expect(loadingPanel()).toBeNull()
    expect(screen.queryByText(t('chat.wait'))).toBeNull()

    fireLive(true)
    expect(loadingPanel()).not.toBeNull()
    expect(screen.queryByText(t('chat.wait'))).toBeNull()
  })

  it.each<[string, () => void]>([
    ['connection dropped', () => setConnection('down')],
    ["'detached'", () => fireState('detached')],
    ["'error'", () => fireState('error')],
  ])('even under the absence veil (%s) the waiting indicator does not intrude (only a turn-open usage)', async (_name, goUnavailable) => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    act(() => captured.onChunk!(tag1(0, USAGE)))
    fireLive(true)
    goUnavailable()
    expect(deadOverlay()).not.toBeNull()
    expect(loadingPanel()).toBeNull()
    expect(screen.queryByText(t('chat.wait'))).toBeNull()
  })

  // A turn boundary attaches a divider even when the preceding item is only usage; it is an empty spacer, not history — ending the wait on it leaves an empty panel (0-item resume = loading until input).
  it.each([
    ['MessageDone', JSON.stringify({ type: 'MessageDone' })],
    ['TurnEnd', JSON.stringify({ type: 'TurnEnd', outcome: { kind: 'Completed' } })],
  ])('the divider of a turn boundary (%s) after usage does not end the loading', async (_name, boundary) => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireLive(true)
    act(() => captured.onChunk!(tag1(0, USAGE)))
    act(() => captured.onChunk!(tag1(1, boundary)))
    expect(document.querySelector('[data-rich-live="1"] .h-3')).not.toBeNull() // The divider really did stand
    expect(loadingPanel()).not.toBeNull()
    expect(emptyState()).toBeNull()
    expect(screen.queryByText(t('chat.wait'))).toBeNull()
  })

  // Opposite direction: dropping all 'boundary' rows would leave even history in which only a user bubble was restored stuck in the wait.
  it('when a user bubble arrives after a divider the loading lifts', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireLive(true)
    act(() => captured.onChunk!(tag1(0, USAGE)))
    act(() => captured.onChunk!(tag1(1, JSON.stringify({ type: 'MessageDone' }))))
    const user = JSON.stringify({
      type: 'Structured',
      kind: 'user',
      json: JSON.stringify({ type: 'text', text: 'earlier question', uuid: 'U0' }),
    })
    act(() => captured.onChunk!(tag1(2, user)))
    expect(loadingPanel()).toBeNull()
    expect(screen.getByText('earlier question')).toBeTruthy()
  })

  it('when the first history that draws a row arrives the loading lifts and the first screen does not appear either — the input box is the same element', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireLive(true)
    const before = textarea()
    act(() => captured.onChunk!(tag1(0, USAGE)))
    act(() => captured.onChunk!(tag1(1, JSON.stringify({ type: 'TextDelta', text: 'restored history' }))))

    expect(loadingPanel()).toBeNull()
    expect(awaitingAttr()).toBeNull()
    expect(emptyState()).toBeNull()
    expect(screen.getByText('restored history')).toBeTruthy()
    expect(textarea()).toBe(before)
  })

  // ADR-0231: input does not lift the loading — text typed while connecting stands in the queued list; only a row (history · delivered bubble) lifts it, else an empty panel shows before history.
  it('typing does not lift the loading — it lifts the moment the text standing in the list becomes a delivered bubble', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireLive(true)
    fireEvent.change(textarea(), { target: { value: 'hello' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()
    expect(loadingPanel()).not.toBeNull()
    expect(emptyState()).toBeNull()

    act(() => captured.onChunk!(queuedFrame(0, { kind: 'Queued', id: 'X', text: 'hello' })))
    expect(loadingPanel()).not.toBeNull()
    expect(listedIds()).toEqual(['X'])
    expect(screen.queryByText(t('chat.wait'))).toBeNull()

    act(() => captured.onChunk!(queuedFrame(1, { kind: 'Delivered', id: 'X' })))
    expect(loadingPanel()).toBeNull()
    expect(queuedList()).toBeNull()
    expect(screen.getByText('hello')).toBeTruthy()
    expect(screen.getByText(t('chat.wait'))).toBeTruthy()
  })

  it('the synthetic bubble of text typed while idle is also a row so it lifts the loading — the send itself does not lift it', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireLive(true)
    fireEvent.change(textarea(), { target: { value: 'hi' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()
    expect(loadingPanel()).not.toBeNull()

    const echo = JSON.stringify({
      type: 'Structured',
      kind: 'user',
      json: JSON.stringify({ type: 'text', text: 'hi', uuid: 'D1' }),
    })
    act(() => captured.onChunk!(tag1(0, echo)))
    expect(loadingPanel()).toBeNull()
    expect(screen.getByText('hi')).toBeTruthy()
    expect(screen.getByText(t('chat.wait'))).toBeTruthy()
  })

  it('when the connection drops the absence veil wins and there is no loading', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireLive(true)
    expect(loadingPanel()).not.toBeNull()

    setConnection('down')
    expect(deadOverlay()).not.toBeNull()
    expect(loadingPanel()).toBeNull()
    expect(awaitingAttr()).toBeNull()
  })

  it('the takeover incarnation of an exited agent — only the veil appears, no loading and no first screen', async () => {
    agentStoreState.agents = [{ id: AGENT, cwd: 'C:/x', status: { type: 'Exited' } }]
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireLive(true)

    expect(deadOverlay()).not.toBeNull()
    expect(loadingPanel()).toBeNull()
    expect(emptyState()).toBeNull()
  })

  // D4 restart — an incarnation relaunched with a new conversation after the resume failed arrives with a false marker.
  it("clear + 'live'(false) → first screen, clear + 'live'(true) → loading again", async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireLive(true)
    expect(loadingPanel()).not.toBeNull()

    act(() => {
      captured.onReset!()
      captured.onState!('live', { continuesConversation: false })
    })
    expect(loadingPanel()).toBeNull()
    expect(emptyState()).not.toBeNull()

    act(() => {
      captured.onReset!()
      captured.onState!('live', { continuesConversation: true })
    })
    expect(emptyState()).toBeNull()
    expect(loadingPanel()).not.toBeNull()
  })

  // 'buffering' of a reattach (same incarnation) lowers restore-complete — in that span it is neither loading nor the first screen (today's rule).
  it("if 'buffering' comes while waiting the loading comes down, and the next 'live' marker is followed again", async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireLive(true)
    fireState('buffering')
    expect(loadingPanel()).toBeNull()
    expect(emptyState()).toBeNull()

    fireLive(true)
    expect(loadingPanel()).not.toBeNull()
  })

  // A 'live' without info reads as false — it must not inherit the true of the previous 'live'.
  it("a 'live' without info is false — a true received earlier does not remain", async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireLive(true)
    fireState('buffering')
    fireState('live')

    expect(loadingPanel()).toBeNull()
    expect(emptyState()).not.toBeNull()
  })

  it('if the assignment changes to another agent the previous agent\'s marker does not follow', async () => {
    const { rerender } = render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireLive(true)
    expect(loadingPanel()).not.toBeNull()

    rerender(<RichSlot viewId="v1" agentId={OTHER} />)
    await flush()
    expect(loadingPanel()).toBeNull()
    fireState('live')
    expect(loadingPanel()).toBeNull()
    expect(emptyState()).not.toBeNull()
  })

  // With the marker missing or false the screen is the same as before the marker was introduced — measured byte for byte (the old daemon/shell path that carries no marker).
  it("'live'(false) and a 'live' without info draw the same DOM", async () => {
    const a = render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')
    const withoutInfo = a.container.innerHTML
    cleanup()

    const b = render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireLive(false)
    expect(b.container.innerHTML).toBe(withoutInfo)
    expect(loadingPanel()).toBeNull()
    expect(emptyState()).not.toBeNull()
  })

  // Outside history wait the wait-indicator rule is unchanged: a window that received only usage also shows it if the turn is open; it goes down only while the loading panel is up (case above).
  it.each([
    ['no info', undefined],
    ['false', { continuesConversation: false }],
  ])("marker %s + a window with only usage → the waiting indicator shows without loading (the rule from before the marker was introduced)", async (_name, info) => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    act(() => captured.onChunk!(tag1(0, USAGE)))
    act(() => captured.onState!('live', info))
    expect(loadingPanel()).toBeNull()
    expect(emptyState()).toBeNull()
    expect(screen.getByText(t('chat.wait'))).toBeTruthy()
  })
})

// ── ADR-0231: queued-input list layout · empty-state gate · re-attach check wiring ─────────────────────────
function queuedFrame(seq: number, op: Record<string, unknown>): OutputChunk {
  return tag1(seq, JSON.stringify({ type: 'QueuedInput', op }))
}
function queuedList(): HTMLElement | null {
  return document.querySelector('[data-queued-inputs="1"]')
}
function listedIds(): string[] {
  return Array.from(document.querySelectorAll('[data-queued-input]')).map(
    (el) => el.getAttribute('data-queued-input') ?? '',
  )
}
/** List query whose reply the test releases — stacks resolvers in the order called. */
function deferredListing(): Array<(listing: unknown) => void> {
  const resolvers: Array<(listing: unknown) => void> = []
  clientMock.listQueuedInputs = vi.fn(() => new Promise((resolve) => resolvers.push(resolve)))
  return resolvers
}

describe('RichSlot(live) — queued input list (ADR-0231)', () => {
  it('if a queued item stands it is not the empty state — the list is above the input box · the label on the line above, the textarea is not remounted', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')
    expect(emptyState()).not.toBeNull()
    expect(queuedList()).toBeNull()
    const before = textarea()

    act(() => captured.onChunk!(queuedFrame(0, { kind: 'Queued', id: 'X', text: 'hello' })))
    expect(emptyState()).toBeNull()
    expect(listedIds()).toEqual(['X'])
    const list = queuedList()!
    const label = document.querySelector('[data-rich-label="1"]')!
    expect(label.closest('[data-wait-strip="1"]')).not.toBeNull() // The label is on the wait indicator row above the list — the list does not cover it
    expect(label.compareDocumentPosition(list) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    expect(list.compareDocumentPosition(textarea()) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    expect(textarea()).toBe(before)

    // Receipt → leaves the list and a bubble takes its place. The input box is still the same element.
    act(() => captured.onChunk!(queuedFrame(1, { kind: 'Delivered', id: 'X' })))
    expect(queuedList()).toBeNull()
    expect(screen.getByText('hello')).toBeTruthy()
    expect(textarea()).toBe(before)
  })

  it('a list event releases the waiting indicator (P1b verdict kept — an understood frame)', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    feedCompletedTurn()
    fireEvent.change(textarea(), { target: { value: 'again' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()
    expect(screen.queryByText(t('chat.wait'))).toBeTruthy()
    act(() => captured.onChunk!(queuedFrame(2, { kind: 'Queued', id: 'Y', text: 'again' })))
    expect(screen.queryByText(t('chat.wait'))).toBeNull()
  })

  // P1b decision 2: placing a receipt turns the wait indicator on even after the turn closed — codex hands text typed mid-turn to the head of the next turn, so `Delivered` is that turn's first event and the
  //   reasoning span emits nothing for a while. ★The accumulator must not block the resume the placement causes★ (it would turn the indicator off in that span); the producer's turn boundary turns it off.
  it('text typed mid-turn — stands in the list then one bubble set at the receipt place at the head of the next turn, the waiting indicator is switched off by that turn\'s boundary', async () => {
    const turnEnd = JSON.stringify({ type: 'TurnEnd', outcome: { kind: 'Completed' } })
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    act(() => captured.onChunk!(tag1(0, JSON.stringify({ type: 'TextDelta', text: 'working' }))))
    fireEvent.change(textarea(), { target: { value: 'also this' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()
    act(() => captured.onChunk!(queuedFrame(1, { kind: 'Queued', id: 'Q', text: 'also this' })))
    expect(listedIds()).toEqual(['Q'])
    expect(screen.getByText(t('chat.wait'))).toBeTruthy()

    act(() => captured.onChunk!(tag1(2, turnEnd)))
    expect(screen.queryByText(t('chat.wait'))).toBeNull()
    expect(listedIds()).toEqual(['Q'])

    const echo = JSON.stringify({
      type: 'Structured',
      kind: 'user',
      json: JSON.stringify({ type: 'text', text: 'also this', uuid: 'Q' }),
    })
    act(() => captured.onChunk!(queuedFrame(3, { kind: 'Delivered', id: 'Q' })))
    act(() => captured.onChunk!(tag1(4, echo)))
    expect(queuedList()).toBeNull()
    expect(screen.getAllByText('also this')).toHaveLength(1)
    expect(screen.getByText(t('chat.wait'))).toBeTruthy()

    act(() => captured.onChunk!(tag1(5, JSON.stringify({ type: 'TextDelta', text: 'answer' }))))
    act(() => captured.onChunk!(tag1(6, turnEnd)))
    expect(screen.queryByText(t('chat.wait'))).toBeNull()
  })

  it("at every 'live' (marker carried) it asks the list and recovers queued items that were not in the ring", async () => {
    const resolvers = deferredListing()
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    act(() => captured.onChunk!(tag1(40, JSON.stringify({ type: 'TextDelta', text: 'long turn' }))))
    act(() => captured.onState!('live', { continuesConversation: false, epoch: 5 }))
    expect(clientMock.listQueuedInputs).toHaveBeenCalledWith(AGENT)
    await act(async () => {
      resolvers[0]({
        inputs: [{ id: 'Q', text: 'evicted but waiting', state: 'queued', cancel: null }],
        as_of_seq: 30,
        epoch: 5,
        stopped_after_error: false,
      })
    })
    expect(listedIds()).toEqual(['Q'])
  })

  it('a live without a marker does not ask (the answer\'s incarnation cannot be told apart)', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')
    expect(clientMock.listQueuedInputs).not.toHaveBeenCalled()
  })

  it('if it enters the buffering phase before the answer comes, a late old answer is dropped · an answer of another incarnation is dropped too', async () => {
    const resolvers = deferredListing()
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    act(() => captured.onState!('live', { continuesConversation: false, epoch: 5 }))
    fireState('buffering')
    const row = { id: 'Q', text: 'stale', state: 'queued', cancel: null }
    await act(async () => {
      resolvers[0]({ inputs: [row], as_of_seq: null, epoch: 5, stopped_after_error: false })
    })
    expect(queuedList()).toBeNull()

    act(() => captured.onState!('live', { continuesConversation: false, epoch: 5 }))
    await act(async () => {
      resolvers[1]({ inputs: [row], as_of_seq: null, epoch: 6, stopped_after_error: false })
    })
    expect(queuedList()).toBeNull()
  })

  it('even if the list query fails (a sleeping agent) the slot is unchanged', async () => {
    clientMock.listQueuedInputs = vi.fn(async () => {
      throw new Error('NOT_FOUND: agent is not running')
    })
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    act(() => captured.onState!('live', { continuesConversation: false, epoch: 5 }))
    await flush()
    expect(emptyState()).not.toBeNull()
    expect(queuedList()).toBeNull()
  })
})

// ★Scroll-follow wiring (ADR-0242 · TRD §2-1)★: the decision is measured by followCore.test.ts and the hook by useScrollFollow.test.tsx. Here only that the slot plugged the hook into the conversation viewport and reaches
//   the two attach places (send U4 · clearing) and the button · handle map. jsdom has no layout, so only the Radix Viewport gets geometry and ResizeObserver is a recording fake.
//   ★Do not fire scroll events★ — that mounts the Radix scrollbar and pulls in things outside this test's subject.
describe('RichSlot(live) — scroll follow (ADR-0242)', () => {
  const geo = { height: 1000, client: 200 }
  const tops = new WeakMap<Element, number>()
  const isViewport = (el: Element): boolean => el.hasAttribute('data-radix-scroll-area-viewport')

  beforeEach(() => {
    vi.stubGlobal(
      'ResizeObserver',
      class {
        observe(): void {}
        unobserve(): void {}
        disconnect(): void {}
      },
    )
    vi.spyOn(Element.prototype, 'scrollHeight', 'get').mockImplementation(function (this: Element) {
      return isViewport(this) ? geo.height : 0
    })
    vi.spyOn(Element.prototype, 'clientHeight', 'get').mockImplementation(function (this: Element) {
      return isViewport(this) ? geo.client : 0
    })
    vi.spyOn(Element.prototype, 'scrollTop', 'get').mockImplementation(function (this: Element) {
      return tops.get(this) ?? 0
    })
    vi.spyOn(Element.prototype, 'scrollTop', 'set').mockImplementation(function (this: Element, v: number) {
      tops.set(this, v)
    })
  })
  afterEach(() => {
    vi.unstubAllGlobals()
    vi.useRealTimers()
  })

  const viewport = (): HTMLElement => document.querySelector('[data-radix-scroll-area-viewport]') as HTMLElement
  const followAttr = (): string | null => viewport().getAttribute('data-scroll-follow')
  const jumpButton = (): HTMLElement | null => document.querySelector('[data-jump-to-bottom="1"]')

  async function mountWithTurn(): Promise<void> {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    feedCompletedTurn()
  }

  it('starts attached to the conversation viewport and uses it as the bottom', async () => {
    await mountWithTurn()
    expect(followAttr()).toBe('pinned')
    expect(viewport().scrollTop).toBe(800)
  })

  it('if sent while loosened upward it attaches to the bottom again (U4)', async () => {
    await mountWithTurn()
    tops.set(viewport(), 300)
    fireEvent.wheel(viewport(), { deltaY: -40 })
    expect(followAttr()).toBe('free')

    const textarea = screen.getByPlaceholderText(/메시지 입력/)
    fireEvent.change(textarea, { target: { value: 'hello' } })
    fireEvent.keyDown(textarea, { key: 'Enter' })
    expect(followAttr()).toBe('pinned')
    expect(viewport().scrollTop).toBe(800)
  })

  it('clear (onReset) attaches again — the new incarnation\'s history lands at the bottom', async () => {
    await mountWithTurn()
    fireEvent.wheel(viewport(), { deltaY: -40 })
    expect(followAttr()).toBe('free')
    fireReset()
    expect(followAttr()).toBe('pinned')
  })

  it('when it falls off, a 「to bottom」 button appears after a moment, and pressing attaches and it disappears', async () => {
    await mountWithTurn()
    vi.useFakeTimers()
    fireEvent.wheel(viewport(), { deltaY: -40 })
    expect(jumpButton()).toBeNull()
    act(() => vi.advanceTimersByTime(JUMP_BUTTON_DELAY_MS))
    // The button is a child of ScrollArea — the absolute reference is the seam's Root, so it does not scroll.
    expect(viewport().contains(jumpButton())).toBe(true)

    fireEvent.click(jumpButton()!)
    expect(followAttr()).toBe('pinned')
    expect(jumpButton()).toBeNull()
  })

  it('pressing 「to bottom」 leaves focus on the viewport so an Esc typed afterward interrupts the turn (ADR-0237 U6)', async () => {
    agentStoreState.agents = [
      { id: AGENT, cwd: 'C:/x', status: { type: 'Running' }, capabilities: { control: { interrupt: true } } },
    ]
    agentStoreState.agentsLoaded = true
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    act(() => captured.onChunk!(tag1(0, JSON.stringify({ type: 'TextDelta', text: 'streaming reply' }))))
    vi.useFakeTimers()
    fireEvent.wheel(viewport(), { deltaY: -40 })
    act(() => vi.advanceTimersByTime(JUMP_BUTTON_DELAY_MS))

    // jsdom does not move focus on click — in a real window, pressing the button gives the button focus first.
    jumpButton()!.focus()
    fireEvent.click(jumpButton()!)
    expect(jumpButton()).toBeNull()
    expect(document.activeElement).toBe(viewport())

    fireEvent.keyDown(document.activeElement!, { key: 'Escape' })
    expect(dispatchMock.fireAndForget.mock.calls.filter(([id]) => id === 'agent.interrupt')).toEqual([
      ['agent.interrupt', { agentId: AGENT }],
    ])
  })

  it('it goes into the handle map with that slot (viewId) — the LLM path (slot.scrollToBottom) calls this', async () => {
    await mountWithTurn()
    fireEvent.wheel(viewport(), { deltaY: -40 })
    act(() => getFollow('v1')!.pin())
    expect(followAttr()).toBe('pinned')
    cleanup()
    expect(getFollow('v1')).toBeUndefined()
  })
})

// ★This table deliberately imitates the command (`fireAndForget` mock)★ — the real window command ignores a turn's second Esc (ADR-0244 · the 「interrupting」 table below), so focus tests pressing twice in one turn lean on the mock.
describe('RichSlot(live) — Esc interrupts the running turn (ADR-0237)', () => {
  // 'absent' = there is no capability field at all (passing undefined makes the default go in instead).
  function running(interrupt: boolean | 'absent' = true): unknown[] {
    const capabilities = interrupt === 'absent' ? undefined : { control: { interrupt } }
    return [{ id: AGENT, cwd: 'C:/x', status: { type: 'Running' }, capabilities }]
  }
  const interrupts = (): unknown[][] =>
    dispatchMock.fireAndForget.mock.calls.filter(([id]) => id === 'agent.interrupt')
  const root = (): HTMLElement => document.querySelector('[data-rich-live="1"]') as HTMLElement
  const input = (): HTMLTextAreaElement => screen.getByPlaceholderText(/메시지 입력/) as HTMLTextAreaElement

  /** The turn of an agent with the interrupt capability is running (only deltas have come — no turn end). */
  async function mountStreaming(interrupt: boolean | 'absent' = true): Promise<void> {
    agentStoreState.agents = running(interrupt)
    agentStoreState.agentsLoaded = true
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    act(() => captured.onChunk!(tag1(0, JSON.stringify({ type: 'TextDelta', text: 'streaming reply' }))))
    expect(screen.queryByText(t('chat.wait'))).not.toBeNull()
  }

  it('if all conditions are true it calls the command once and consumes the key — optimistic state and the input box text are unchanged', async () => {
    await mountStreaming()
    fireEvent.change(input(), { target: { value: '초안' } })

    const notPrevented = fireEvent.keyDown(input(), { key: 'Escape' })

    expect(notPrevented).toBe(false)
    expect(interrupts()).toEqual([['agent.interrupt', { agentId: AGENT }]])
    expect(input().value).toBe('초안')
    expect(screen.queryByText(t('chat.wait'))).not.toBeNull()
  })

  it.each<[string, Record<string, unknown>]>([
    ['ctrl', { ctrlKey: true }],
    ['alt', { altKey: true }],
    ['shift', { shiftKey: true }],
    ['meta', { metaKey: true }],
    ['held repeat', { repeat: true }],
    ['during IME composition (isComposing)', { isComposing: true }],
    ['during IME composition (keyCode 229)', { keyCode: 229 }],
  ])('does not call when a key condition is off — %s', async (_label, init) => {
    await mountStreaming()
    const notPrevented = fireEvent.keyDown(input(), { key: 'Escape', ...init })
    expect(notPrevented).toBe(true)
    expect(interrupts()).toEqual([])
  })

  it('it does not call for other keys', async () => {
    await mountStreaming()
    fireEvent.keyDown(input(), { key: 'a' })
    expect(interrupts()).toEqual([])
  })

  it('an Esc first consumed at the document capture (a Radix layer) does not call', async () => {
    await mountStreaming()
    const eat = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') e.preventDefault()
    }
    document.addEventListener('keydown', eat, true)
    try {
      fireEvent.keyDown(input(), { key: 'Escape' })
    } finally {
      document.removeEventListener('keydown', eat, true)
    }
    expect(interrupts()).toEqual([])
  })

  it('while an overlay marker is in the document it does not call, and when it lifts it calls again', async () => {
    await mountStreaming()
    const overlay = document.createElement('div')
    overlay.setAttribute('data-engram-overlay', '1')
    document.body.appendChild(overlay)
    try {
      expect(fireEvent.keyDown(input(), { key: 'Escape' })).toBe(true)
      expect(interrupts()).toEqual([])
    } finally {
      overlay.remove()
    }
    fireEvent.keyDown(input(), { key: 'Escape' })
    expect(interrupts()).toHaveLength(1)
  })

  it('if no turn is running it does not call (after the turn end)', async () => {
    await mountStreaming()
    act(() => captured.onChunk!(tag1(1, JSON.stringify({ type: 'MessageDone' }))))
    expect(screen.queryByText(t('chat.wait'))).toBeNull()
    fireEvent.keyDown(input(), { key: 'Escape' })
    expect(interrupts()).toEqual([])
  })

  it.each<[string, boolean | 'absent']>([
    ['capability false', false],
    ['capability field absent', 'absent'],
  ])('does not call if the channel does not support interrupt — %s', async (_label, interrupt) => {
    await mountStreaming(interrupt)
    fireEvent.keyDown(input(), { key: 'Escape' })
    expect(interrupts()).toEqual([])
  })

  it('if the agent is absent now (vanished from the roster · connection dropped) it does not call', async () => {
    await mountStreaming()
    setConnection('down')
    fireEvent.keyDown(root(), { key: 'Escape' })
    expect(interrupts()).toEqual([])
    setConnection('connected')

    agentStoreState.agents = []
    act(() => captured.onChunk!(tag1(1, JSON.stringify({ type: 'TextDelta', text: ' more' }))))
    expect(deadOverlay()).not.toBeNull()
    fireEvent.keyDown(root(), { key: 'Escape' })
    expect(interrupts()).toEqual([])
  })

  it('even after pressing the queued input ✕ · 「N more」 focus stays in the area so an Esc typed afterward interrupts the turn (U6)', async () => {
    await mountStreaming()
    for (const [seq, id] of [[1, 'Q1'], [2, 'Q2'], [3, 'Q3'], [4, 'Q4']] as const) {
      act(() => captured.onChunk!(queuedFrame(seq, { kind: 'Queued', id, text: `later ${id}` })))
    }

    // jsdom does not move focus on click — in a real window, pressing the button gives the button focus first.
    const remove = document.querySelector('[data-queued-input="Q1"] button') as HTMLButtonElement
    remove.focus()
    fireEvent.click(remove)
    expect(dispatchMock.fireAndForget).toHaveBeenCalledWith('agent.cancelQueuedInput', { agentId: AGENT, inputId: 'Q1' })
    expect(document.activeElement).toBe(root())
    fireEvent.keyDown(document.activeElement!, { key: 'Escape' })
    expect(interrupts()).toHaveLength(1)

    const more = document.querySelector('[data-queued-more="1"]') as HTMLButtonElement
    more.focus()
    fireEvent.click(more)
    expect(document.querySelector('[data-queued-more="1"]')).toBeNull()
    expect(document.activeElement).toBe(root())
    fireEvent.keyDown(document.activeElement!, { key: 'Escape' })
    expect(interrupts()).toHaveLength(2)
  })

  it('anywhere in the area (U6) — it calls even when focus is on the conversation body (viewport) or the area root', async () => {
    await mountStreaming()
    const viewport = document.querySelector('[data-radix-scroll-area-viewport]') as HTMLElement
    viewport.focus()
    expect(document.activeElement).toBe(viewport)
    fireEvent.keyDown(viewport, { key: 'Escape' })
    expect(interrupts()).toHaveLength(1)

    // Pressing outside the viewport, like the body margin, gives the root focus (tabIndex -1 — outside tab order).
    expect(root().tabIndex).toBe(-1)
    root().focus()
    expect(document.activeElement).toBe(root())
    fireEvent.keyDown(root(), { key: 'Escape' })
    expect(interrupts()).toHaveLength(2)
  })
})

// ★「interrupting」 after Esc (ADR-0244)★: the window command sets it and the area lifts it; here the human path runs to the real command (the ADR-0237 table above only imitates it, so its second Esc is still called).
describe('RichSlot(live) — 「interrupting…」 after Esc · Esc ignored until the turn ends (ADR-0244)', () => {
  const root = (): HTMLElement => document.querySelector('[data-rich-live="1"]') as HTMLElement
  const input = (): HTMLTextAreaElement => screen.getByPlaceholderText(/메시지 입력/) as HTMLTextAreaElement
  const indicator = (): Element | null => document.querySelector('[data-wait-interrupting="1"]')
  const interrupting = (agentId = AGENT): boolean =>
    pendingInterrupt(useInterruptStore.getState(), agentId) !== undefined
  const turnEnd = (seq: number, kind: 'Interrupted' | 'Completed' = 'Interrupted'): OutputChunk =>
    tag1(seq, JSON.stringify({ type: 'TurnEnd', turn_id: null, outcome: { kind } }))
  const delta = (seq: number, text: string): OutputChunk => tag1(seq, JSON.stringify({ type: 'TextDelta', text }))

  beforeEach(async () => {
    const actual = await vi.importActual<typeof import('../../commands/dispatch')>('../../commands/dispatch')
    dispatchMock.fireAndForget.mockImplementation((...args: unknown[]) =>
      actual.fireAndForget(...(args as [string, CommandArgs | undefined])),
    )
    agentStoreState.agents = [
      { id: AGENT, cwd: 'C:/x', status: { type: 'Running' }, capabilities: { control: { interrupt: true } } },
    ]
    agentStoreState.agentsLoaded = true
  })

  /** The turn of an agent with the interrupt capability is running (only deltas have come — no turn end). */
  async function mountStreaming(): Promise<void> {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    act(() => captured.onState!('live'))
    act(() => captured.onChunk!(delta(0, 'streaming reply')))
    expect(screen.getByText(t('chat.wait'))).toBeTruthy()
  }

  it('one Esc = one interrupt · the waiting indicator becomes 「interrupting…」 at once · meanwhile Esc is neither sent nor consumed', async () => {
    await mountStreaming()
    expect(fireEvent.keyDown(input(), { key: 'Escape' })).toBe(false)
    expect(clientMock.interruptAgent).toHaveBeenCalledTimes(1)
    expect(clientMock.interruptAgent).toHaveBeenCalledWith(AGENT)
    expect(indicator()).not.toBeNull()
    expect(screen.getByText(t('chat.interrupting'))).toBeTruthy()
    expect(screen.queryByText(t('chat.wait'))).toBeNull()

    // Even if the interrupt is accepted, it stays as is until the turn end arrives.
    await flush()
    expect(indicator()).not.toBeNull()
    expect(fireEvent.keyDown(input(), { key: 'Escape' })).toBe(true)
    expect(fireEvent.keyDown(root(), { key: 'Escape' })).toBe(true)
    expect(clientMock.interruptAgent).toHaveBeenCalledTimes(1)
  })

  it.each<[string, (seq: number) => OutputChunk]>([
    ['TurnEnd(interrupted)', (seq) => turnEnd(seq)],
    ['TurnEnd(completed)', (seq) => turnEnd(seq, 'Completed')],
    ['MessageDone', (seq) => tag1(seq, JSON.stringify({ type: 'MessageDone' }))],
  ])('lifts at the turn end (%s), and in the next running turn Esc interrupts again', async (_label, end) => {
    await mountStreaming()
    fireEvent.keyDown(input(), { key: 'Escape' })
    await flush()
    expect(interrupting()).toBe(true)

    act(() => captured.onChunk!(end(1)))
    expect(interrupting()).toBe(false)
    expect(indicator()).toBeNull()

    act(() => captured.onChunk!(delta(2, 'next turn')))
    expect(screen.getByText(t('chat.wait'))).toBeTruthy()
    expect(fireEvent.keyDown(input(), { key: 'Escape' })).toBe(false)
    expect(clientMock.interruptAgent).toHaveBeenCalledTimes(2)
    expect(indicator()).not.toBeNull()
  })

  it('even if the turn end and the next turn\'s first frame are bundled into one render it lifts — no indicator remains on the next turn and Esc is consumed', async () => {
    await mountStreaming()
    fireEvent.keyDown(input(), { key: 'Escape' })
    await flush()

    act(() => {
      captured.onChunk!(turnEnd(1))
      captured.onChunk!(delta(2, 'queued message reply'))
    })
    expect(indicator()).toBeNull()
    expect(screen.getByText(t('chat.wait'))).toBeTruthy()
    fireEvent.keyDown(input(), { key: 'Escape' })
    expect(clientMock.interruptAgent).toHaveBeenCalledTimes(2)
  })

  it('an Esc right after sending · before the first frame also lifts at that turn\'s end (a turn the accumulator was already reading as ended)', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    act(() => captured.onState!('live'))
    feedCompletedTurn()
    fireEvent.change(input(), { target: { value: 'hello' } })
    fireEvent.keyDown(input(), { key: 'Enter' })
    await flush()
    expect(screen.getByText(t('chat.wait'))).toBeTruthy()

    fireEvent.keyDown(input(), { key: 'Escape' })
    await flush()
    expect(indicator()).not.toBeNull()

    // Only the end arrives with no frame that opened the turn (the accumulator goes end → end) — it is lifted by the end event itself.
    act(() => captured.onChunk!(turnEnd(2)))
    expect(screen.queryByText(t('chat.wait'))).toBeNull()
    expect(interrupting()).toBe(false)
  })

  it('even if the end of a turn interrupted right after sending and the next turn\'s first frame are bundled into one render it lifts — the accumulator was still reading the previous turn\'s end', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    act(() => captured.onState!('live'))
    feedCompletedTurn()
    fireEvent.change(input(), { target: { value: 'hello' } })
    fireEvent.keyDown(input(), { key: 'Enter' })
    await flush()
    fireEvent.keyDown(input(), { key: 'Escape' })
    await flush()
    expect(indicator()).not.toBeNull()

    act(() => {
      captured.onChunk!(turnEnd(2))
      captured.onChunk!(delta(3, 'next turn'))
    })
    expect(interrupting()).toBe(false)
    expect(indicator()).toBeNull()
    expect(screen.getByText(t('chat.wait'))).toBeTruthy()
    expect(fireEvent.keyDown(input(), { key: 'Escape' })).toBe(false)
    expect(clientMock.interruptAgent).toHaveBeenCalledTimes(2)
    expect(indicator()).not.toBeNull()
  })

  it('it lifts when the sent text stands in the waiting list without opening a turn and the waiting indicator goes off (no turn boundary)', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    act(() => captured.onState!('live'))
    feedCompletedTurn()
    fireEvent.change(input(), { target: { value: 'hello' } })
    fireEvent.keyDown(input(), { key: 'Enter' })
    await flush()
    fireEvent.keyDown(input(), { key: 'Escape' })
    await flush()
    expect(interrupting()).toBe(true)

    act(() => captured.onChunk!(queuedFrame(2, { kind: 'Queued', id: 'M', text: 'hello' })))
    expect(screen.queryByText(t('chat.wait'))).toBeNull()
    expect(interrupting()).toBe(false)
  })

  it.each<[string, () => unknown]>([
    ['lease refused (CONFLICT)', () => INPUT_LOCKED_REFUSAL],
    ['no turn to interrupt', () => 'no turn to interrupt'],
  ])('when the interrupt is refused (%s) it lifts and Esc interrupts again', async (_label, error) => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    clientMock.interruptAgent = vi.fn(async () => {
      throw error()
    })
    await mountStreaming()
    fireEvent.keyDown(input(), { key: 'Escape' })
    expect(indicator()).not.toBeNull()

    await flush()
    expect(indicator()).toBeNull()
    expect(screen.getByText(t('chat.wait'))).toBeTruthy()
    expect(warn).toHaveBeenCalled()
    fireEvent.keyDown(input(), { key: 'Escape' })
    expect(clientMock.interruptAgent).toHaveBeenCalledTimes(2)
  })

  it('even if a second view of the same agent attaches and replays an old turn end it does not lift — what lifts it is a live turn end', async () => {
    const history = [delta(0, 'old reply'), tag1(1, JSON.stringify({ type: 'MessageDone' })), delta(2, 'streaming reply')]
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    act(() => {
      for (const frame of history) captured.onChunk!(frame)
      captured.onState!('live')
    })
    const view1 = captured.onChunk!
    fireEvent.keyDown(input(), { key: 'Escape' })
    await flush()
    expect(interrupting()).toBe(true)

    // Second view — an empty accumulator rebuilds the history (the old turn end seq 1 passes from end to end).
    render(<RichSlot viewId="v2" agentId={AGENT} />)
    await flush()
    act(() => {
      for (const frame of history) captured.onChunk!(frame)
      captured.onState!('live')
    })
    expect(interrupting()).toBe(true)
    expect(document.querySelectorAll('[data-wait-interrupting="1"]')).toHaveLength(2)
    expect(clientMock.interruptAgent).toHaveBeenCalledTimes(1)

    act(() => view1(turnEnd(3)))
    expect(interrupting()).toBe(false)
  })

  it.each<[string, () => void]>([
    [
      'the agent vanishes from the roster (exit)',
      () => {
        agentStoreState.agents = []
        act(() => captured.onChunk!(delta(1, ' more')))
      },
    ],
    ['the subscription exhausted its re-requests (error)', () => act(() => captured.onState!('error'))],
    ['connection dropped', () => setConnection('down')],
  ])('when it becomes absent (%s) it lifts, and the interrupt command of this window sends again', async (_label, becomeUnavailable) => {
    await mountStreaming()
    fireEvent.keyDown(input(), { key: 'Escape' })
    await flush()
    expect(interrupting()).toBe(true)

    becomeUnavailable()
    expect(deadOverlay()).not.toBeNull()
    expect(interrupting()).toBe(false)
    await act(async () => {
      await run('agent.interrupt', { agentId: AGENT })
    })
    expect(clientMock.interruptAgent).toHaveBeenCalledTimes(2)
  })

  it('if only the second view\'s subscription stops (error) the first view\'s 「interrupting」 stays and does not send again — when the last view also stops it lifts', async () => {
    const history = [delta(0, 'old reply'), tag1(1, JSON.stringify({ type: 'MessageDone' })), delta(2, 'streaming reply')]
    const first = render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    const view1State = captured.onState!
    act(() => {
      for (const frame of history) captured.onChunk!(frame)
      view1State('live')
    })
    const view1Input = first.container.querySelector('textarea') as HTMLTextAreaElement
    fireEvent.keyDown(view1Input, { key: 'Escape' })
    await flush()

    render(<RichSlot viewId="v2" agentId={AGENT} />)
    await flush()
    const view2State = captured.onState!
    act(() => {
      for (const frame of history) captured.onChunk!(frame)
      view2State('live')
    })
    expect(useInterruptStore.getState().views).toEqual({ [AGENT]: 2 })

    act(() => view2State('error'))
    expect(interrupting()).toBe(true)
    expect(useInterruptStore.getState().views).toEqual({ [AGENT]: 1 })
    expect(first.container.querySelector('[data-wait-interrupting="1"]')).not.toBeNull()
    expect(fireEvent.keyDown(view1Input, { key: 'Escape' })).toBe(true)
    await act(async () => {
      await run('agent.interrupt', { agentId: AGENT })
    })
    expect(clientMock.interruptAgent).toHaveBeenCalledTimes(1)

    act(() => view1State('error'))
    expect(interrupting()).toBe(false)
    expect(useInterruptStore.getState().views).toEqual({})
  })

  it.each<[string, OutputChunk[], boolean]>([
    ['lifts if it ends as a closed turn', [delta(0, 'reply'), tag1(1, JSON.stringify({ type: 'MessageDone' }))], false],
    [
      'stays if it ends as an open turn',
      [delta(0, 'reply'), tag1(1, JSON.stringify({ type: 'MessageDone' })), delta(2, 'streaming reply')],
      true,
    ],
  ])('interrupt was called during the first buffering and that replay %s', async (_label, replay, stays) => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    await act(async () => {
      await run('agent.interrupt', { agentId: AGENT })
    })
    expect(interrupting()).toBe(true)

    // The turn end is only in the history before the catch-up — the wait indicator is off before and after, so there is no on → off either.
    act(() => {
      for (const frame of replay) captured.onChunk!(frame)
      captured.onState!('live')
    })
    expect(interrupting()).toBe(stays)
  })

  it('a same-incarnation re-buffering after catching up does not lift even if it ends as a closed turn — right after sending, so the new turn\'s first frame is not there yet', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    act(() => captured.onState!('live'))
    feedCompletedTurn()
    fireEvent.change(input(), { target: { value: 'hello' } })
    fireEvent.keyDown(input(), { key: 'Enter' })
    await flush()
    fireEvent.keyDown(input(), { key: 'Escape' })
    await flush()
    expect(interrupting()).toBe(true)

    act(() => {
      captured.onState!('buffering')
      captured.onState!('live')
    })
    expect(interrupting()).toBe(true)
  })

  it('it lifts when a new incarnation (clear) comes — even if the new incarnation\'s history leaves a turn open in the same tick', async () => {
    await mountStreaming()
    fireEvent.keyDown(input(), { key: 'Escape' })
    await flush()

    act(() => {
      captured.onReset!()
      captured.onChunk!(delta(0, 'new incarnation'))
    })
    expect(interrupting()).toBe(false)
    expect(screen.getByText(t('chat.wait'))).toBeTruthy()
    fireEvent.keyDown(input(), { key: 'Escape' })
    expect(clientMock.interruptAgent).toHaveBeenCalledTimes(2)
  })

  it('it lifts on unmount · agent replacement — a stale indicator does not survive', async () => {
    await mountStreaming()
    fireEvent.keyDown(input(), { key: 'Escape' })
    await flush()
    expect(interrupting()).toBe(true)
    cleanup()
    expect(interrupting()).toBe(false)
    expect(useInterruptStore.getState().views).toEqual({})

    const OTHER = 'eeee-ffff'
    const { rerender } = render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    act(() => captured.onChunk!(delta(0, 'streaming reply')))
    fireEvent.keyDown(input(), { key: 'Escape' })
    expect(interrupting()).toBe(true)
    rerender(<RichSlot viewId="v1" agentId={OTHER} />)
    await flush()
    expect(interrupting()).toBe(false)
    expect(useInterruptStore.getState().views).toEqual({ [OTHER]: 1 })
  })
})

// ★Tool bundle glue (ADR-0239 · TRD S21-chat-ux §4-4 · §4-5)★: bundling · summary · toggle are measured by toolRuns.test.ts · ToolGroupRow.test.tsx. Here only that the slot passes its slot id to the conversation view and binds to the store
//   (head alive, command reaches it), the binding lifetime (mount · agent swap · new incarnation), and that expanding a non-last bundle releases follow.
describe('RichSlot(live) — tool group attachment (ADR-0239)', () => {
  const OTHER = 'eeee-ffff-0000-1111'

  function toolCallFrame(seq: number, id: string): OutputChunk {
    return tag1(
      seq,
      JSON.stringify({ type: 'ToolCall', name: 'Read', args_json: '{}', id, turn_id: null, message_id: null }),
    )
  }
  function turnEndFrame(seq: number): OutputChunk {
    return tag1(seq, JSON.stringify({ type: 'TurnEnd', turn_id: null, outcome: { kind: 'Completed' } }))
  }

  /** One finished turn = two tool calls — with no chosen value it is collapsed. Bundle key = the id of the first call (`tool:<id>`). */
  function feedFinishedGroup(seq: number, ids: readonly [string, string]): void {
    act(() => captured.onChunk!(toolCallFrame(seq, ids[0])))
    act(() => captured.onChunk!(toolCallFrame(seq + 1, ids[1])))
    act(() => captured.onChunk!(turnEndFrame(seq + 2)))
  }

  const groups = (): HTMLElement[] => Array.from(document.querySelectorAll<HTMLElement>('[data-tool-group]'))
  const group = (key: string): HTMLElement => {
    const el = document.querySelector<HTMLElement>(`[data-tool-group="${key}"]`)
    if (el === null) throw new Error(`group '${key}' is not drawn`)
    return el
  }
  // The first button of the bundle root is the head — when expanded, the member rows' buttons stand after it.
  const header = (key: string): HTMLButtonElement => group(key).querySelector('button') as HTMLButtonElement
  const openAttr = (key: string): string | null => group(key).getAttribute('data-tool-group-open')
  const chosen = (key: string): boolean | undefined => useToolGroupStore.getState().bySlot.v1?.open[key]
  const followAttr = (): string | null =>
    document.querySelector('[data-radix-scroll-area-viewport]')!.getAttribute('data-scroll-follow')

  it('the head of a finished group is alive, and pressing it expands and collapses through the store', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    feedFinishedGroup(0, ['r1', 'r2'])

    expect(groups()).toHaveLength(1)
    expect(group('tool:r1').getAttribute('data-tool-group-count')).toBe('2')
    // Regression net for a defect the FE-2b-2 review caught — if the slot id is not passed down, the head is off and a collapsed bundle cannot be reopened.
    expect(header('tool:r1').disabled).toBe(false)
    expect(openAttr('tool:r1')).toBe('0')

    fireEvent.click(header('tool:r1'))
    expect(openAttr('tool:r1')).toBe('1')
    expect(chosen('tool:r1')).toBe(true)

    fireEvent.click(header('tool:r1'))
    expect(openAttr('tool:r1')).toBe('0')
    expect(chosen('tool:r1')).toBe(false)
  })

  it('the LLM command (chat.toolGroup.setExpanded) reaches the mounted slot, and after it goes down it answers with an error', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    feedFinishedGroup(0, ['r1', 'r2'])

    let answer: unknown
    act(() => {
      answer = run('chat.toolGroup.setExpanded', { slotId: 'v1', groupKey: 'tool:r1', expanded: true })
    })
    expect(answer).toEqual({ expanded: true })
    expect(openAttr('tool:r1')).toBe('1')

    cleanup()
    expect(() => run('chat.toolGroup.setExpanded', { slotId: 'v1', groupKey: 'tool:r1', expanded: false })).toThrow(
      /no conversation view of slot 'v1'/,
    )
  })

  it('expanding a non-last group releases follow, and the last group · collapsing leaves attachment as is (§4-5)', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    feedFinishedGroup(0, ['a1', 'a2'])
    feedFinishedGroup(3, ['b1', 'b2'])
    expect(groups().map((g) => g.getAttribute('data-tool-group'))).toEqual(['tool:a1', 'tool:b1'])
    expect(followAttr()).toBe('pinned')

    fireEvent.click(header('tool:b1')) // Last bundle
    expect(openAttr('tool:b1')).toBe('1')
    expect(followAttr()).toBe('pinned')
    expect(getFollow('v1')!.pinned).toBe(true)

    fireEvent.click(header('tool:a1')) // Non-last bundle
    expect(openAttr('tool:a1')).toBe('1')
    expect(followAttr()).toBe('free')
    expect(getFollow('v1')!.pinned).toBe(false)

    // Collapse is not called — even if the bundle above is collapsed after re-attaching, the pin remains.
    act(() => getFollow('v1')!.pin())
    fireEvent.click(header('tool:a1'))
    expect(openAttr('tool:a1')).toBe('0')
    expect(followAttr()).toBe('pinned')
  })

  it('remounting with the same agent re-attaches the chosen expansion, and when another agent comes it does not follow', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    feedFinishedGroup(0, ['r1', 'r2'])
    fireEvent.click(header('tool:r1'))
    expect(openAttr('tool:r1')).toBe('1')

    // Unmount (render mode swap · pop-out move) → again with the same agent — a replay of the same event sequence stands with the same keys.
    cleanup()
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    feedFinishedGroup(0, ['r1', 'r2'])
    expect(openAttr('tool:r1')).toBe('1')

    // A different agent in the same slot — even if a bundle with the same key comes, it is drawn as the default (collapsed).
    cleanup()
    render(<RichSlot viewId="v1" agentId={OTHER} />)
    await flush()
    feedFinishedGroup(0, ['r1', 'r2'])
    expect(openAttr('tool:r1')).toBe('0')
    expect(useToolGroupStore.getState().bySlot.v1).toMatchObject({ agentId: OTHER, open: {} })
  })

  // That an old release cannot release a new binding (mount marker) is measured by `toolGroupStore.test.ts`; React runs the old cleanup first here, so that order cannot be told apart.
  it('when the agent changes in one instance (key remount) it is regrouped with the new agent so the chosen expansion is empty and the new head writes', async () => {
    const { rerender } = render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    feedFinishedGroup(0, ['r1', 'r2'])
    fireEvent.click(header('tool:r1'))

    rerender(<RichSlot viewId="v1" agentId={OTHER} />)
    await flush()
    feedFinishedGroup(0, ['r1', 'r2'])
    expect(openAttr('tool:r1')).toBe('0')
    // The new instance's head writes to the store (a new binding stands).
    fireEvent.click(header('tool:r1'))
    expect(openAttr('tool:r1')).toBe('1')
  })

  it('clear of a new incarnation (onReset) empties the chosen expansion — even if a group with the same key arrives it is drawn at the default (collapsed)', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    feedFinishedGroup(0, ['r1', 'r2'])
    fireEvent.click(header('tool:r1'))
    expect(openAttr('tool:r1')).toBe('1')

    fireReset()
    feedFinishedGroup(0, ['r1', 'r2'])
    expect(openAttr('tool:r1')).toBe('0')
    // The binding stays — the head writes even after clearing.
    fireEvent.click(header('tool:r1'))
    expect(openAttr('tool:r1')).toBe('1')
  })

  it("a re-replay of the same incarnation ('buffering' → 'live') does not erase the chosen expansion", async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    feedFinishedGroup(0, ['r1', 'r2'])
    fireEvent.click(header('tool:r1'))

    fireState('buffering')
    fireState('live')
    expect(openAttr('tool:r1')).toBe('1')
    expect(chosen('tool:r1')).toBe(true)
  })

  // ★Late tool end window (TRD §4-7 ⑧ · ⑨ frontend · ADR-0241)★: codex does not kill a running command on interrupt, so a previous turn's tool end arrives late. Landing right after a send · before the first reply, the previous turn is closed
  //   (turnDone true) and only awaiting holds the wait indicator; if `ToolResult` released awaiting, "Wait" would go off with no reply.
  it('right after sending, if only the end (ToolResult) of a past-turn tool arrives "Wait" stays and a marker attaches to that row — from the first real answer the turn takes over', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    act(() => captured.onChunk!(toolCallFrame(0, 'c1')))
    act(() => captured.onChunk!(turnEndFrame(1)))
    expect(screen.queryByText(t('chat.wait'))).toBeNull()

    const input = screen.getByPlaceholderText(/메시지 입력/)
    fireEvent.change(input, { target: { value: 'next' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    await flush()
    expect(screen.getByText(t('chat.wait'))).toBeTruthy()

    act(() => captured.onChunk!(tag1(2, JSON.stringify({ type: 'ToolResult', id: 'c1', outcome: 'Failed' }))))
    expect(screen.getByText(t('chat.wait'))).toBeTruthy()
    expect(document.querySelector('[data-tool-mark="failed"]')).not.toBeNull()

    // First real reply — awaiting is released and the indicator is held up by the open turn. The turn boundary turns it off (if awaiting were stuck it would not go off).
    act(() => captured.onChunk!(tag1(3, JSON.stringify({ type: 'TextDelta', text: 'answer' }))))
    expect(screen.getByText(t('chat.wait'))).toBeTruthy()
    act(() => captured.onChunk!(turnEndFrame(4)))
    expect(screen.queryByText(t('chat.wait'))).toBeNull()
  })
})
