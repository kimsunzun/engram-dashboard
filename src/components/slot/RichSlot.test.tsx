// RichSlot (live mode) send() failure path regression — local UI-state error handling (WIRE unchanged ADR-0044/45/46).
//
// Background: send() turns the streaming hint on at once with awaiting=true right after sending (FIX 5b). When the response event arrives,
//   awaiting is cleared, but if writeStdin itself rejects, the response never comes and awaiting stays set →
//   derived streaming (= awaiting || (!turnDone && items.length>0)) stays true, so the indicator sticks.
//   fix: setAwaiting(false) in catch. This test observes that recovery.
//
// ★Observation surface★: the only visual signal of streaming is the "Wait" label (`chat/WaitRow.tsx`) on the wait row above the input box,
//   shown when streaming. So to make the state observable, the subscription callback is captured
//   and fed TextDelta + MessageDone → items=[text,separator] & turnDone=true. Then streaming narrows to awaiting
//   (the !turnDone term dies), and the presence of the "Wait" indicator mirrors awaiting.
//
// Strategy: stub agentClient (clientFactory) and agentStore with the same pattern as slotTagGate.test.tsx. Capture the subscribeOutput
//   callback (onChunk) to inject tag1 (StructuredEvent) chunks, and swap writeStdin between reject/resolve.

import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { FRAME_TAG_STRUCTURED_EVENT } from '../../api/wsFrame'
import { INPUT_LOCKED_REFUSAL, type OutputChunk, type ReplayLiveInfo, type ViewPhase } from '../../api/agentClient'
import { t } from '../../i18n'
import '../../commands/chatCommands' // side-effect register — measures whether the tool-bundle command reaches the mounted slot.
import '../../commands/agentCommands' // side-effect register — carries the interrupt command all the way to the path that sets 「중단하는 중」 (interrupting) (ADR-0244).
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

/**
 * Injects 1 content turn in a completed state (TextDelta → MessageDone). Result: items=[text,separator], turnDone=true.
 * In this state streaming narrows to awaiting (the !turnDone term neutralized), so the "Wait" indicator reflects awaiting as is.
 */
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

describe('RichSlot(live) — send() 실패 시 awaiting 해제', () => {
  it('writeStdin 이 reject 되면 "Wait" 스트리밍 신호가 고착되지 않고 idle 로 복귀한다', async () => {
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
    expect(screen.queryByText('Wait')).toBeNull()

    // send() turns the streaming hint on at once with awaiting=true.
    const textarea = screen.getByPlaceholderText(/메시지 입력/)
    fireEvent.change(textarea, { target: { value: 'hello' } })
    fireEvent.keyDown(textarea, { key: 'Enter' })

    expect(clientMock.writeStdin).toHaveBeenCalledTimes(1)

    // Flush the microtask for the reject taking effect (catch → setAwaiting(false)).
    await flush()

    expect(screen.queryByText('Wait')).toBeNull()
    expect(warn).toHaveBeenCalled()
  })

  it('writeStdin 이 성공하면(응답 전) awaiting 이 유지돼 "Wait" 신호가 켜진다(대조군)', async () => {
    clientMock.writeStdin = vi.fn(async () => undefined) // success = resolve

    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()

    feedCompletedTurn()
    expect(screen.queryByText('Wait')).toBeNull()

    const textarea = screen.getByPlaceholderText(/메시지 입력/)
    fireEvent.change(textarea, { target: { value: 'hello' } })
    fireEvent.keyDown(textarea, { key: 'Enter' })
    await flush()

    // Success path — there is no response event yet, so streaming is kept by the awaiting bridge.
    expect(screen.getByText('Wait')).toBeTruthy()
  })
})

// User decision 2026-10-01: the wait indicator stands outside the scroll list, on an always-present row right above the input box.
describe('RichSlot(live) — 대기 표시 줄', () => {
  it('대화 중이면 줄이 늘 서 있고 스크롤 목록 밖 · 입력 묶음 바로 위다 — 표시는 도는 동안만 그 안에', async () => {
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
    expect(screen.queryByText('Wait')).toBeNull()

    fireEvent.change(textarea, { target: { value: 'hello' } })
    fireEvent.keyDown(textarea, { key: 'Enter' })
    await flush()
    const wait = screen.getByText('Wait')
    expect(document.querySelector('[data-wait-strip="1"]')).toBe(strip)
    expect(strip.contains(wait)).toBe(true)
    expect(viewport.contains(wait)).toBe(false)
    // The wait text stands on the left and the label on the right in one line, and the label is the same node even when the wait indicator turns on.
    expect(wait.compareDocumentPosition(label) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    expect(document.querySelector('[data-rich-label="1"]')).toBe(label)
    // The input box is the same node even when the row is inserted (textarea comment — the same element in both layouts).
    expect(screen.getByPlaceholderText(/메시지 입력/)).toBe(textarea)
  })

  it('첫 실행 화면(빈 상태)에는 줄도 라벨도 없다', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    act(() => captured.onState!('live', { continuesConversation: false }))
    expect(emptyState()).not.toBeNull()
    expect(document.querySelector('[data-wait-strip="1"]')).toBeNull()
    expect(document.querySelector('[data-rich-label="1"]')).toBeNull() // The label collapse in the empty state is unchanged
  })
})

// ★Follow-up send flicker regression★: when a new send follows a completed previous turn (turnDone=true), the synthetic user echo
//   arrives before the first assistant token. "Wait" must hold even at the moment that echo clears awaiting
//   (accumulator fix: user Structured lowers turnDone to false, keeping the derived streaming alive). Before the fix, at this moment
//   streaming = awaiting(false) || (!turnDone(true) && items>0) = false, and "Wait" flickered off.
describe('RichSlot(live) — 후속 전송 시 합성 user 에코가 "Wait" 을 끄지 않는다(flicker FIX)', () => {
  it('완결 턴 뒤 전송 → user 에코 도착 후에도 "Wait" 이 유지된다', async () => {
    clientMock.writeStdin = vi.fn(async () => undefined) // success = resolve

    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()

    // Previous turn complete (turnDone=true) + content present.
    feedCompletedTurn()
    expect(screen.queryByText('Wait')).toBeNull()

    // Follow-up send — streaming hint on with awaiting=true.
    const textarea = screen.getByPlaceholderText(/메시지 입력/)
    fireEvent.change(textarea, { target: { value: 'follow-up' } })
    fireEvent.keyDown(textarea, { key: 'Enter' })
    await flush()
    expect(screen.getByText('Wait')).toBeTruthy()

    // The synthetic user echo arrives before the first assistant token (right after write_input in json mode). The subscription callback does
    //   setAwaiting(false), but the accumulator lowers turnDone to false, so the derived streaming holds.
    act(() =>
      captured.onChunk!(
        tag1(2, JSON.stringify({
          type: 'Structured',
          kind: 'user',
          json: JSON.stringify({ type: 'text', text: 'follow-up', uuid: 'U' }),
        })),
      ),
    )

    expect(screen.getByText('Wait')).toBeTruthy()
  })
})

// ★A frame that was not understood does not turn off the wait indicator★ — the regression shows only over **multiple turns**: a freshly mounted slot has
//   turnDone false, so "Wait" stands whatever frame arrives, and a one-turn test cannot see this defect.
//   Defect shape: after the previous turn closed (turnDone=true) a new send goes out, and if the first frame of that turn
//   is of a kind this shell does not know — the subscription callback clears awaiting, turnDone stays true from the previous turn, and "Wait" goes off while the response is running.
describe('RichSlot(live) — 모르는 종류의 프레임이 "Wait" 을 끄지 않는다', () => {
  it('완결 턴 뒤 전송 → 그 턴 첫 프레임이 모르는 종류여도 "Wait" 이 유지된다', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    clientMock.writeStdin = vi.fn(async () => undefined)

    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()

    feedCompletedTurn() // turnDone=true + content present
    expect(screen.queryByText('Wait')).toBeNull()

    const textarea = screen.getByPlaceholderText(/메시지 입력/)
    fireEvent.change(textarea, { target: { value: 'follow-up' } })
    fireEvent.keyDown(textarea, { key: 'Enter' })
    await flush()
    expect(screen.getByText('Wait')).toBeTruthy()

    // A kind sent by a newer daemon that is not in this shell's bindings.
    act(() => captured.onChunk!(tag1(2, JSON.stringify({ type: 'SomethingNewer', whatever: 1 }))))
    expect(screen.getByText('Wait')).toBeTruthy()

    // When a known frame arrives, control of the indicator passes to turnDone, and the boundary turns it off normally.
    act(() => captured.onChunk!(tag1(3, JSON.stringify({ type: 'TextDelta', text: 'answer' }))))
    expect(screen.getByText('Wait')).toBeTruthy()
    act(() => captured.onChunk!(tag1(4, JSON.stringify({ type: 'MessageDone' }))))
    expect(screen.queryByText('Wait')).toBeNull()
    expect(warn).toHaveBeenCalled() // The diagnostic name goes to console only
  })

  it('복원된 이력의 끝이 모르는 종류여도 "Wait" 이 고착되지 않는다(전송 없음)', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()

    feedCompletedTurn()
    act(() => captured.onChunk!(tag1(2, JSON.stringify({ type: 'SomethingNewer', whatever: 1 }))))

    expect(screen.queryByText('Wait')).toBeNull()
    expect(warn).toHaveBeenCalled()
  })
})

// ★A retryable stream error does not turn off "Wait"★ — codex reports overload and rate limits as a single `Error` line
//   and the same turn continues after it (see the decoder comment on that side). claude also closes a failed turn with `MessageDone`, so
//   the old rule of counting `Error` as a turn end was wrong on both sides. Symptom before the fix = "Wait" went off on one error line and
//   came back on the next delta, a flicker.
describe('RichSlot(live) — 턴 한복판의 Error 가 "Wait" 을 끄지 않는다', () => {
  it('델타 → Error → 델타 구간 내내 "Wait" 이 유지되고, 경계만이 끈다', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()

    act(() => captured.onChunk!(tag1(0, JSON.stringify({ type: 'TextDelta', text: 'thinking' }))))
    expect(screen.getByText('Wait')).toBeTruthy()

    act(() =>
      captured.onChunk!(
        tag1(1, JSON.stringify({ type: 'Error', message: 'overloaded (serverOverloaded) [willRetry]' })),
      ),
    )
    expect(screen.getByText('Wait')).toBeTruthy() // Before the fix: it went off here

    act(() => captured.onChunk!(tag1(2, JSON.stringify({ type: 'TextDelta', text: ' resumed' }))))
    expect(screen.getByText('Wait')).toBeTruthy()

    act(() => captured.onChunk!(tag1(3, JSON.stringify({ type: 'MessageDone' }))))
    expect(screen.queryByText('Wait')).toBeNull()
  })
})

// ★ADR-0145 empty state★: the JSON-mode first-run screen (mascot + "Claude Code" + centered input box).
//
// Core regression = **the display gate is not "0 items" but "restore-complete signal ('live') + 0 items"**. The chat view clears the
//   list and subscribes on mount, so even a session with history has 0 items before restore ends → looking at 0 items alone makes the
//   intro screen show and then flip to the conversation, a flicker. That signal comes as the 4th argument of subscribeOutput (the state callback).
//
// Observation surface: empty-state container [data-rich-empty] · mascot [data-rich-mascot] · identity label attached only in the
//   bottom layout [data-rich-label] · input box className (empty state rounded-xl ↔ bottom flex-1).

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
  if (els.length !== 1) throw new Error(`textarea 는 1개여야 한다 — 실제 ${els.length}개`)
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

describe('RichSlot(live) — ADR-0145 첫 실행 빈 상태', () => {
  it('복원 중(live 미발화)에는 0건이어도 빈 상태를 그리지 않는다', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()

    // Passing the 4th argument (state callback) is itself the regression target — if it is not passed, the gate never opens.
    expect(captured.onState).toBeTruthy()
    expect(emptyState()).toBeNull()
    expect(screen.queryByText('Claude Code')).toBeNull()

    fireState('buffering')
    expect(emptyState()).toBeNull()
  })

  it("'live' + 0건이면 마스코트·문구·둥근 입력창을 그린다", async () => {
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

  it("'error' 로 끝나면 0건이어도 빈 상태를 그리지 않는다(현행 빈 화면 유지)", async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('error')

    expect(emptyState()).toBeNull()
  })

  // ★Re-attach regression (a pair with protocolClient's buffering notification)★: when the daemon disconnects and reattaches, the view goes live →
  //   back to buffering and receives a full replay again. If that notification is not heard and 'restore complete' stays latched, the mascot
  //   flips to the conversation when history accumulated during the disconnect pours in.
  it("live 이후 'buffering' 이 다시 오면 빈 상태를 내린다(재replay 깜빡임 차단)", async () => {
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

  // ★Re-replay accumulation regression (user decision 2026-08-20)★: the clearing signal means another incarnation numbers frames from 0 again and
  //   the subscription dropped its progress cursor. If the accumulator is not cleared, history coming again from the start piles
  //   over the current conversation, and if the local seq guard is not reset, all those frames are dropped and the conversation never returns.
  it('onReset 은 대화를 비우고 새 화신을 한 벌로만 다시 그린다', async () => {
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

  // ★Not cleared on a phase notification (regression in the opposite direction)★: when only the socket flickers and returns, the client keeps its cursor,
  //   issues a full re-replay, and dedup absorbs the overlapping front — that span is also 'buffering', so if the phase
  //   cleared it, the conversation would vanish and be redrawn on every brief disconnect.
  it.each<ViewPhase>(['detached', 'buffering', 'live', 'error'])(
    "'%s' 국면은 대화를 그대로 둔다",
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

  // ★Clearing also takes down the send traces★: this signal means history of **another incarnation** is coming, so the
  //   message sent earlier was received by an incarnation that is already gone and the reply never comes. If left, the new incarnation with 0 history items
  //   does not show the first-run screen ("empty slot but in conversation"), and the wait indicator sticks with no reply.
  //   ★This signal does not come on resuming the same incarnation★ — the wait indicator in that span is measured by the phase case right below.
  it('onReset 은 전송 흔적도 함께 내려 새 화신을 첫 실행 화면으로 되돌린다', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')
    expect(emptyState()).not.toBeNull()

    fireEvent.change(textarea(), { target: { value: 'hello' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()
    expect(emptyState()).toBeNull()
    expect(screen.getByText('Wait')).toBeTruthy() // wait indicator (awaiting)

    // Another incarnation attached — if it ends with 0 history items, the first-run screen is correct.
    fireReset()
    fireState('live')
    expect(emptyState()).not.toBeNull()
  })

  // Opposite direction: in the span of re-attaching to the same incarnation (no clearing signal) the wait indicator must stay alive —
  //   a send that went out is not cancelled by re-attach.
  it('비우기 없이 국면만 오가면 전송 흔적을 지키지 않는다(대기 표시 유지)', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')
    fireEvent.change(textarea(), { target: { value: 'hello' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()

    fireState('buffering')
    fireState('live')
    expect(emptyState()).toBeNull()
    expect(screen.getByText('Wait')).toBeTruthy()
  })

  // Leaves absence on the surface — while the registry has no such agent, the slot must not look "alive but quiet"
  //   (the ADR-0148 absent curtain is applied to detached too).
  it("'detached' 는 부재 막을 띄운다", async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    expect(deadOverlay()).toBeNull()

    fireState('detached')
    expect(deadOverlay()).not.toBeNull()
  })

  // Same for a view that exhausted the re-request ladder — it can receive nothing, and with no indicator it becomes a quiet blank panel.
  it("'error' 도 부재 막을 띄운다(세 슬롯 동형)", async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('error')
    expect(deadOverlay()?.getAttribute('data-slot-phase')).toBe('error')
  })

  it("'live' 여도 대화 item 이 있으면 빈 상태가 아니다", async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    feedCompletedTurn()
    fireState('live')

    expect(emptyState()).toBeNull()
    expect(textarea().className).toContain('flex-1')
  })

  it('첫 전송 즉시 빈 상태가 사라지고 입력창이 하단 배치로 돌아간다 — 같은 엘리먼트(remount 없음)', async () => {
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

  // The only path where resubscribe runs is **the assignment changing to a different agent** (a re-spawn does not replace the subscription —
  //   see the 'identity' describe below). If that transition inherited the restore-complete mark, the empty state would show in the span before
  //   the new agent's history arrives and then flip to the conversation, a flicker.
  it('다른 에이전트로 배정이 바뀌면 복원 완료 표시가 내려가 live 재발화 전까지 빈 상태가 아니다', async () => {
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

  // Text differs per layout: the empty state shows the user-set greeting, the bottom shows the Enter/Shift+Enter usage hint.
  //   The two layouts share one element, so if the key branching breaks it is not visible from the screen alone → pinned down here.
  it('placeholder 는 빈 상태와 하단 배치가 서로 다른 문구를 쓴다', async () => {
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

  it('종료된 에이전트는 빈 상태 배치에서도 종료 문구가 이긴다', async () => {
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
// awaiting is also cleared (a) on tag1 frames that create no item (empty TextDelta · user uuid dedup skip ·
// empty MessageDone), and (b) by the writeStdin failure catch without knowing which send it belongs to. Both create a span of "waiting for a response but
// items is still 0", and if the mascot comes back to life in that span, the first screen covers the conversation.
describe('RichSlot(live) — ADR-0145 빈 상태 억제는 전송 사실을 따른다(awaiting 아님)', () => {
  it('전송 후 item 을 만들지 않는 프레임(빈 델타)이 와도 빈 상태로 돌아가지 않는다', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')
    expect(emptyState()).not.toBeNull()

    fireEvent.change(textarea(), { target: { value: 'hello' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()
    expect(emptyState()).toBeNull()

    // Empty delta = a frame the accumulator lets through without creating an item (the TextDelta arm of structuredAccumulator).
    //   The subscription callback clears awaiting on this frame too → with an awaiting gate, the mascot would come back to life here.
    act(() => captured.onChunk!(tag1(0, JSON.stringify({ type: 'TextDelta', text: '' }))))
    expect(emptyState()).toBeNull()
  })

  it('앞 화신에 보낸 전송의 뒤늦은 실패가 새 화신의 전송 표시를 되돌리지 않는다', async () => {
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

    // Another incarnation attached (re-spawn) → the clearing signal takes down the send traces, live is received again and it returns to the empty state.
    //   ★The instance stays the same★ — a re-spawn does not replace the component, so a failure carried across lands on the **same instance**.
    //   What blocks that is the sequence guard below.
    fireReset()
    fireState('live')
    expect(emptyState()).not.toBeNull()

    // Send B (new incarnation) — success path.
    clientMock.writeStdin = vi.fn(async () => undefined)
    fireEvent.change(textarea(), { target: { value: 'send B' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()
    expect(emptyState()).toBeNull()

    // Now A fails. A was a send to the vanished incarnation, so it must not touch any indicator on the current screen
    //   — it neither goes back to the empty state nor turns off B's wait indicator.
    rejectA(new Error('write failed'))
    await flush()
    expect(emptyState()).toBeNull()
    expect(screen.getByText('Wait')).toBeTruthy()
    expect(warn).toHaveBeenCalled()
  })

  it('같은 인스턴스에서도 앞선 전송의 뒤늦은 실패가 최신 전송의 대기 표시를 지우지 않는다', async () => {
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
    expect(screen.getByText('Wait')).toBeTruthy()

    // Send B (success) — the latest send passes to B.
    clientMock.writeStdin = vi.fn(async () => undefined)
    fireEvent.change(textarea(), { target: { value: 'send B' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()

    // Belated A failure — it is not the latest send, so it touches no indicator (B is still waiting for a response).
    rejectA(new Error('write failed'))
    await flush()
    expect(screen.getByText('Wait')).toBeTruthy()
    expect(emptyState()).toBeNull()
    expect(warn).toHaveBeenCalled()
  })

  it('첫 전송이 실패하면 빈 상태로 되돌아간다 — 실제로 아무것도 안 나갔다', async () => {
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
    expect(screen.queryByText('Wait')).toBeNull()
    expect(warn).toHaveBeenCalled()
  })

  it('성공한 전송이 있으면 이후 전송이 실패해도 빈 상태로 되돌아가지 않는다', async () => {
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

  // ★A success carried across a clearing is not a send fact of the new incarnation★: if the incarnation changes while send A is in flight, A's
  //   outcome belongs to the already vanished incarnation. If that success set "a send went out from this instance", then even when a send that
  //   actually failed to go out in the new incarnation fails, it cannot return to the first-run screen, and a slot that sent nothing
  //   stays as in-conversation. The sequence guard must cover both success and failure, and clearing must bump that sequence too — with only
  //   one side, this case passes through the other side as is.
  it('비우기 뒤 도착한 앞 화신 전송의 성공은 새 화신의 실패 되돌리기를 막지 않는다', async () => {
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
    expect(screen.queryByText('Wait')).toBeNull()
    expect(warn).toHaveBeenCalled()
  })
})

// ★ADR-0148: a slot with no targeted agent★
//
// Process exit and connection loss are treated the same (user decision). Screen content is not erased — whether conversation or first-run
// screen, it stays as it was at that moment — only a blur curtain + symbol is laid on. A dead slot looking like "just start now" was the defect.
describe('RichSlot(live) — ADR-0148 에이전트 부재 표현', () => {
  it('종료된 에이전트: 대화 내용을 지우지 않고 흐림 막 + 심볼만 얹는다', async () => {
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

  it('빈 상태(0건)에서 종료돼도 마스코트·문구를 남긴 채 흐림 막이 덮는다', async () => {
    agentStoreState.agents = [{ id: AGENT, cwd: 'C:/x', status: { type: 'Exited' } }]
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')

    expect(emptyState()).not.toBeNull()
    expect(mascot()).not.toBeNull()
    expect(screen.getByText('Claude Code')).toBeTruthy()
    expect(deadOverlay()).not.toBeNull()
  })

  // ★The real outcome of a kill★: the reaper collects the session and **removes it from the registry** (the profile stays as a corpse).
  //   So status does not catch it, and an id that still does not resolve after the registry is received is absent. If that were not seen as absent,
  //   the dead agent's input box would stay active.
  it('명부 수신 후 해석 안 되는 에이전트도 부재로 본다 — 내용 유지 + 오버레이 + 입력 차단', async () => {
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

  it('명부를 아직 못 받은 구간(agentsLoaded=false)은 부재로 보지 않는다', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()

    // agents is empty but the registry is not received, so the decision is held — a slot must not be killed right after boot.
    expect(deadOverlay()).toBeNull()
    expect(textarea().disabled).toBe(false)
  })

  it('연결이 끊기면 살아있는 에이전트도 같게 취급하고, 다시 붙으면 걷힌다', async () => {
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

  // ★Every condition for raising the curtain is a condition for blocking input (ADR-0148 decision 1 / ADR-0149 decision 4)★: the curtain is
  //   pointer-events-none so it does not intercept operation, and disabled cannot stop programmatic key events either.
  //   Blocking on exit alone lets a send go out from a disconnected slot and fail, which the user sees only as no response.
  //   Here the agent is **kept alive** in the registry and only the connection is cut, to measure the branch that is not an exit.
  it('연결이 끊겨 막이 뜬 동안은 에이전트가 살아 있어도 전송이 막히고 입력창이 비활성이다', async () => {
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
// If the subscription effect alone takes down the list and indicators, the **first commit** after the identity changes is drawn with
// new props + old state (passive effects run after paint). Assigning a different agent to the same slot flashes the previous agent's conversation
// in that frame. Asserting after flushing effects hides this defect, so here we look directly at "was the instance replaced?"
// (DOM node replaced = unmount then remount).
describe('RichSlot(live) — 정체성이 바뀌면 인스턴스를 새로 마운트한다', () => {
  const OTHER = 'eeee-ffff-0000-1111'

  it('agentId 가 바뀌면 슬롯 루트 노드가 교체된다(옛 상태를 물려받는 커밋이 없다)', async () => {
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

  // ★Opposite direction (do not revive the old regression)★: once, the incarnation number was in this key, so a re-spawn replaced the instance.
  //   Then the conversation is erased before the replay even arrives, and since the daemon discards the ring on exit, that
  //   conversation cannot be revived anywhere. Now the same instance is kept, and erasing is done by the clearing signal **when the new incarnation's
  //   history actually arrives** (the onReset cases above).
  it('같은 agentId 로 다시 그려도 인스턴스를 갈지 않는다(대화 보존)', async () => {
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
// When 'live' announces "this incarnation came up to resume a saved conversation", a loading overlay goes on the conversation area instead of the first-run screen. There are four
// ways it ends — the first history row · input · absence (the curtain wins) · restart (clearing + a false 'live').
// With the marker missing or false, nothing may differ by a single character from before the marker was introduced (the old daemon/shell path that carries no marker is this case).
describe('RichSlot(live) — ADR-0226 이어받기 화신의 이력 대기', () => {
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

  it('표식 참 + 0건 → 첫 화면 대신 로딩과 옅은 막, 입력창은 하단 배치로 활성', async () => {
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
    expect(screen.queryByText('Wait')).toBeNull()
  })

  // As measured, the first live frame of a resume is usage. Ending the wait on it shows an empty list before history arrives,
  //   and if the wait indicator decision also uses it, loading and the wait indicator show together.
  it('행을 안 그리는 usage 만 온 동안 로딩이 유지되고 대기 표시는 없다', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    act(() => captured.onChunk!(tag1(0, USAGE))) // The replay is delivered before 'live'
    fireLive(true)
    expect(loadingPanel()).not.toBeNull()
    expect(screen.queryByText('Wait')).toBeNull()

    act(() => captured.onChunk!(tag1(1, USAGE))) // Live usage
    expect(loadingPanel()).not.toBeNull()
    expect(screen.queryByText('Wait')).toBeNull()
    expect(emptyState()).toBeNull()
  })

  // The wait indicator is not raised in the window where the panel briefly goes down ('buffering' of a same-incarnation reattach) either — if raised, it flickers panel →
  //   wait indicator (elapsed seconds) → panel. The basis for taking the wait indicator down is "no history yet", not the phase.
  it("대기 중 'buffering' 으로 패널이 내려가도 대기 표시가 끼지 않는다(턴 열린 usage 만)", async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    act(() => captured.onChunk!(tag1(0, USAGE))) // The turn stays open (turnDone=false)
    fireLive(true)
    expect(loadingPanel()).not.toBeNull()
    expect(screen.queryByText('Wait')).toBeNull()

    fireState('buffering')
    expect(loadingPanel()).toBeNull()
    expect(screen.queryByText('Wait')).toBeNull()

    fireLive(true)
    expect(loadingPanel()).not.toBeNull()
    expect(screen.queryByText('Wait')).toBeNull()
  })

  it.each<[string, () => void]>([
    ['연결 끊김', () => setConnection('down')],
    ["'detached'", () => fireState('detached')],
    ["'error'", () => fireState('error')],
  ])('부재 막(%s) 아래에도 대기 표시가 끼지 않는다(턴 열린 usage 만)', async (_name, goUnavailable) => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    act(() => captured.onChunk!(tag1(0, USAGE)))
    fireLive(true)
    goUnavailable()
    expect(deadOverlay()).not.toBeNull()
    expect(loadingPanel()).toBeNull()
    expect(screen.queryByText('Wait')).toBeNull()
  })

  // A turn boundary attaches a divider even when the preceding item is only usage. The divider is an empty spacer, not history — ending
  //   the wait on it leaves an empty panel with neither loading nor the first screen (0-item resume = loading until input).
  it.each([
    ['MessageDone', JSON.stringify({ type: 'MessageDone' })],
    ['TurnEnd', JSON.stringify({ type: 'TurnEnd', outcome: { kind: 'Completed' } })],
  ])('usage 뒤 턴 경계(%s)의 구분선은 로딩을 끝내지 않는다', async (_name, boundary) => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireLive(true)
    act(() => captured.onChunk!(tag1(0, USAGE)))
    act(() => captured.onChunk!(tag1(1, boundary)))
    expect(document.querySelector('[data-rich-live="1"] .h-3')).not.toBeNull() // The divider really did stand
    expect(loadingPanel()).not.toBeNull()
    expect(emptyState()).toBeNull()
    expect(screen.queryByText('Wait')).toBeNull()
  })

  // Opposite direction: dropping all 'boundary' rows would leave even history in which only a user bubble was restored stuck in the wait.
  it('구분선 뒤 유저 말풍선이 오면 로딩이 걷힌다', async () => {
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

  it('행을 그리는 첫 이력이 오면 로딩이 걷히고 첫 화면도 뜨지 않는다 — 입력창은 같은 엘리먼트', async () => {
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

  // ADR-0231: input does not lift the loading — text typed while connecting stands in the queued list, and what lifts the loading is a row (history ·
  //   a delivered bubble). Lifting it by input would show an empty panel before history arrives.
  it('입력은 로딩을 걷지 않는다 — 목록에 선 글이 전달된 말풍선이 되는 순간 걷힌다', async () => {
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
    expect(screen.queryByText('Wait')).toBeNull()

    act(() => captured.onChunk!(queuedFrame(1, { kind: 'Delivered', id: 'X' })))
    expect(loadingPanel()).toBeNull()
    expect(queuedList()).toBeNull()
    expect(screen.getByText('hello')).toBeTruthy()
    expect(screen.getByText('Wait')).toBeTruthy()
  })

  it('한가할 때 친 글의 합성 말풍선도 행이라 로딩을 걷는다 — 전송 자체는 걷지 않는다', async () => {
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
    expect(screen.getByText('Wait')).toBeTruthy()
  })

  it('연결이 끊기면 부재 막이 이기고 로딩은 없다', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireLive(true)
    expect(loadingPanel()).not.toBeNull()

    setConnection('down')
    expect(deadOverlay()).not.toBeNull()
    expect(loadingPanel()).toBeNull()
    expect(awaitingAttr()).toBeNull()
  })

  it('종료된 에이전트의 이어받기 화신 — 막만 뜨고 로딩도 첫 화면도 없다', async () => {
    agentStoreState.agents = [{ id: AGENT, cwd: 'C:/x', status: { type: 'Exited' } }]
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireLive(true)

    expect(deadOverlay()).not.toBeNull()
    expect(loadingPanel()).toBeNull()
    expect(emptyState()).toBeNull()
  })

  // D4 restart — an incarnation relaunched with a new conversation after the resume failed arrives with a false marker.
  it("비우기 + 'live'(거짓) → 첫 화면, 비우기 + 'live'(참) → 다시 로딩", async () => {
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
  it("대기 중 'buffering' 이 오면 로딩을 내리고, 다음 'live' 의 표식을 다시 따른다", async () => {
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
  it("info 없는 'live' 는 거짓 — 앞서 받은 참이 남지 않는다", async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireLive(true)
    fireState('buffering')
    fireState('live')

    expect(loadingPanel()).toBeNull()
    expect(emptyState()).not.toBeNull()
  })

  it('다른 에이전트로 배정이 바뀌면 앞 에이전트의 표식이 따라오지 않는다', async () => {
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
  it("'live'(거짓)과 info 없는 'live' 는 같은 DOM 을 그린다", async () => {
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

  // The wait indicator rule outside history wait is as before the marker was introduced — a window that received only usage, which draws no row, also shows the wait indicator if the turn is not
  //   closed. The wait indicator is taken down only while the loading panel is up (the usage case right above).
  it.each([
    ['info 없음', undefined],
    ['거짓', { continuesConversation: false }],
  ])("표식 %s + usage 만 온 창 → 로딩 없이 대기 표시가 뜬다(표식 도입 전 규칙)", async (_name, info) => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    act(() => captured.onChunk!(tag1(0, USAGE)))
    act(() => captured.onState!('live', info))
    expect(loadingPanel()).toBeNull()
    expect(emptyState()).toBeNull()
    expect(screen.getByText('Wait')).toBeTruthy()
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

describe('RichSlot(live) — 대기 입력 목록(ADR-0231)', () => {
  it('대기 항목이 서 있으면 빈 상태가 아니다 — 목록은 입력창 위 · 라벨은 그 위 줄, textarea 는 remount 되지 않는다', async () => {
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

  it('목록 사건은 대기 표시를 푼다(P1b 판정 유지 — 알아들은 프레임)', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    feedCompletedTurn()
    fireEvent.change(textarea(), { target: { value: 'again' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()
    expect(screen.queryByText('Wait')).toBeTruthy()
    act(() => captured.onChunk!(queuedFrame(2, { kind: 'Queued', id: 'Y', text: 'again' })))
    expect(screen.queryByText('Wait')).toBeNull()
  })

  // P1b decision 2: placing a receipt turns the wait indicator on even if it arrives after the turn closed — codex hands text typed mid-turn to the head of the next turn,
  //   so that `Delivered` is the first event of the new turn, and the reasoning span is not translated, so no events come for a while after it.
  //   ★So the accumulator side does not block the resume that the placement causes★ — blocking it would turn the wait indicator off in that span. What turns it off is the boundary
  //   the producer emits for that turn (not placing a receipt where there is no turn is the producer's job).
  it('턴 도중 친 글 — 목록에 섰다가 다음 턴 머리의 받음 자리에 말풍선 한 벌, 대기 표시는 그 턴의 경계가 끈다', async () => {
    const turnEnd = JSON.stringify({ type: 'TurnEnd', outcome: { kind: 'Completed' } })
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    act(() => captured.onChunk!(tag1(0, JSON.stringify({ type: 'TextDelta', text: 'working' }))))
    fireEvent.change(textarea(), { target: { value: 'also this' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()
    act(() => captured.onChunk!(queuedFrame(1, { kind: 'Queued', id: 'Q', text: 'also this' })))
    expect(listedIds()).toEqual(['Q'])
    expect(screen.getByText('Wait')).toBeTruthy()

    act(() => captured.onChunk!(tag1(2, turnEnd)))
    expect(screen.queryByText('Wait')).toBeNull()
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
    expect(screen.getByText('Wait')).toBeTruthy()

    act(() => captured.onChunk!(tag1(5, JSON.stringify({ type: 'TextDelta', text: 'answer' }))))
    act(() => captured.onChunk!(tag1(6, turnEnd)))
    expect(screen.queryByText('Wait')).toBeNull()
  })

  it("'live'(표식 실림)마다 목록을 물어 링에 없던 대기 항목을 되찾는다", async () => {
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

  it('표식 없는 live 는 묻지 않는다(답의 화신을 가를 수 없다)', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')
    expect(clientMock.listQueuedInputs).not.toHaveBeenCalled()
  })

  it('답이 오기 전에 버퍼 국면에 들면 늦게 온 옛 답은 버린다 · 다른 화신의 답도 버린다', async () => {
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

  it('목록 조회가 실패해도(잠든 에이전트) 슬롯은 그대로다', async () => {
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

// ★Scroll-follow wiring (ADR-0242 · TRD §2-1)★: the decision is measured by followCore.test.ts and the hook by useScrollFollow.test.tsx.
//   Here we only measure that the slot plugged that hook into the conversation viewport, and that the two places that attach (send U4 · clearing) and the button · handle map are reached.
//   jsdom has no layout, so only the Radix Viewport is given geometry, and ResizeObserver is a fake that only records calls.
//   ★Do not fire scroll events★ — doing so mounts the Radix scrollbar and pulls in things outside this test's subject.
describe('RichSlot(live) — 스크롤 따라가기(ADR-0242)', () => {
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

  it('대화 뷰포트에 붙어 시작하고 바닥으로 쓴다', async () => {
    await mountWithTurn()
    expect(followAttr()).toBe('pinned')
    expect(viewport().scrollTop).toBe(800)
  })

  it('위로 풀린 채 보내면 다시 바닥에 붙는다(U4)', async () => {
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

  it('비우기(onReset)는 다시 붙인다 — 새 화신의 이력이 바닥에 착지한다', async () => {
    await mountWithTurn()
    fireEvent.wheel(viewport(), { deltaY: -40 })
    expect(followAttr()).toBe('free')
    fireReset()
    expect(followAttr()).toBe('pinned')
  })

  it('떨어지면 잠깐 뒤 「맨 아래로」 버튼이 뜨고, 누르면 붙고 사라진다', async () => {
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

  it('「맨 아래로」 를 누르면 포커스가 뷰포트에 남아 이어서 친 Esc 가 턴을 끊는다(ADR-0237 U6)', async () => {
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

  it('손잡이 맵에 그 슬롯(viewId)으로 오른다 — LLM 경로(slot.scrollToBottom)가 이것을 부른다', async () => {
    await mountWithTurn()
    fireEvent.wheel(viewport(), { deltaY: -40 })
    act(() => getFollow('v1')!.pin())
    expect(followAttr()).toBe('pinned')
    cleanup()
    expect(getFollow('v1')).toBeUndefined()
  })
})

// ★This table deliberately only imitates the command (`fireAndForget` mock)★ — the real window command ignores the second Esc of a turn (ADR-0244 ·
//   the 「중단하는 중」 table below), so the focus tests that press twice in one turn and call twice lean on that mock.
describe('RichSlot(live) — Esc 는 도는 턴을 끊는다(ADR-0237)', () => {
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
    expect(screen.queryByText('Wait')).not.toBeNull()
  }

  it('조건이 전부 참이면 명령을 한 번 부르고 키를 먹는다 — 낙관 상태도 입력창 글도 그대로다', async () => {
    await mountStreaming()
    fireEvent.change(input(), { target: { value: '초안' } })

    const notPrevented = fireEvent.keyDown(input(), { key: 'Escape' })

    expect(notPrevented).toBe(false)
    expect(interrupts()).toEqual([['agent.interrupt', { agentId: AGENT }]])
    expect(input().value).toBe('초안')
    expect(screen.queryByText('Wait')).not.toBeNull()
  })

  it.each<[string, Record<string, unknown>]>([
    ['ctrl', { ctrlKey: true }],
    ['alt', { altKey: true }],
    ['shift', { shiftKey: true }],
    ['meta', { metaKey: true }],
    ['누른 채 반복', { repeat: true }],
    ['IME 조합 중(isComposing)', { isComposing: true }],
    ['IME 조합 중(keyCode 229)', { keyCode: 229 }],
  ])('키 조건이 어긋나면 부르지 않는다 — %s', async (_label, init) => {
    await mountStreaming()
    const notPrevented = fireEvent.keyDown(input(), { key: 'Escape', ...init })
    expect(notPrevented).toBe(true)
    expect(interrupts()).toEqual([])
  })

  it('다른 키는 부르지 않는다', async () => {
    await mountStreaming()
    fireEvent.keyDown(input(), { key: 'a' })
    expect(interrupts()).toEqual([])
  })

  it('문서 capture 에서 먼저 먹힌 Esc(Radix 레이어)는 부르지 않는다', async () => {
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

  it('오버레이 표지가 문서에 있는 동안은 부르지 않고, 걷히면 다시 부른다', async () => {
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

  it('턴이 안 돌면 부르지 않는다(턴 끝 뒤)', async () => {
    await mountStreaming()
    act(() => captured.onChunk!(tag1(1, JSON.stringify({ type: 'MessageDone' }))))
    expect(screen.queryByText('Wait')).toBeNull()
    fireEvent.keyDown(input(), { key: 'Escape' })
    expect(interrupts()).toEqual([])
  })

  it.each<[string, boolean | 'absent']>([
    ['능력 거짓', false],
    ['능력 칸 없음', 'absent'],
  ])('통로가 끊기를 지원하지 않으면 부르지 않는다 — %s', async (_label, interrupt) => {
    await mountStreaming(interrupt)
    fireEvent.keyDown(input(), { key: 'Escape' })
    expect(interrupts()).toEqual([])
  })

  it('에이전트가 지금 없으면(명부에서 사라짐 · 연결 끊김) 부르지 않는다', async () => {
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

  it('대기 입력 ✕ · 「외 N개」 를 누른 뒤에도 포커스가 칸 안에 남아 이어서 친 Esc 가 턴을 끊는다(U6)', async () => {
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

  it('칸 안 어디든(U6) — 대화 본문(뷰포트)이나 칸 루트에 포커스가 있어도 부른다', async () => {
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

// ★「중단하는 중」 (interrupting) after Esc (ADR-0244)★: the side that sets it is the window command and the side that lifts it is the area, so here the human path is carried
//   all the way to the real command (the ADR-0237 table above only imitates the command, so the second Esc in those tests is still called).
describe('RichSlot(live) — Esc 뒤 「중단하는 중…」 · 턴 끝까지 Esc 무시(ADR-0244)', () => {
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
    expect(screen.getByText('Wait')).toBeTruthy()
  }

  it('Esc 한 번 = 끊기 한 번 · 대기 표시가 곧바로 「중단하는 중…」 · 그동안 Esc 는 보내지도 먹지도 않는다', async () => {
    await mountStreaming()
    expect(fireEvent.keyDown(input(), { key: 'Escape' })).toBe(false)
    expect(clientMock.interruptAgent).toHaveBeenCalledTimes(1)
    expect(clientMock.interruptAgent).toHaveBeenCalledWith(AGENT)
    expect(indicator()).not.toBeNull()
    expect(screen.getByText(t('chat.interrupting'))).toBeTruthy()
    expect(screen.queryByText('Wait')).toBeNull()

    // Even if the interrupt is accepted, it stays as is until the turn end arrives.
    await flush()
    expect(indicator()).not.toBeNull()
    expect(fireEvent.keyDown(input(), { key: 'Escape' })).toBe(true)
    expect(fireEvent.keyDown(root(), { key: 'Escape' })).toBe(true)
    expect(clientMock.interruptAgent).toHaveBeenCalledTimes(1)
  })

  it.each<[string, (seq: number) => OutputChunk]>([
    ['TurnEnd(중단)', (seq) => turnEnd(seq)],
    ['TurnEnd(완료)', (seq) => turnEnd(seq, 'Completed')],
    ['MessageDone', (seq) => tag1(seq, JSON.stringify({ type: 'MessageDone' }))],
  ])('턴 끝(%s)에 걷히고, 다음에 도는 턴에서는 Esc 가 다시 끊는다', async (_label, end) => {
    await mountStreaming()
    fireEvent.keyDown(input(), { key: 'Escape' })
    await flush()
    expect(interrupting()).toBe(true)

    act(() => captured.onChunk!(end(1)))
    expect(interrupting()).toBe(false)
    expect(indicator()).toBeNull()

    act(() => captured.onChunk!(delta(2, 'next turn')))
    expect(screen.getByText('Wait')).toBeTruthy()
    expect(fireEvent.keyDown(input(), { key: 'Escape' })).toBe(false)
    expect(clientMock.interruptAgent).toHaveBeenCalledTimes(2)
    expect(indicator()).not.toBeNull()
  })

  it('턴 끝과 다음 턴의 첫 프레임이 한 렌더로 묶여도 걷힌다 — 다음 턴에 표시가 남지 않고 Esc 가 먹힌다', async () => {
    await mountStreaming()
    fireEvent.keyDown(input(), { key: 'Escape' })
    await flush()

    act(() => {
      captured.onChunk!(turnEnd(1))
      captured.onChunk!(delta(2, 'queued message reply'))
    })
    expect(indicator()).toBeNull()
    expect(screen.getByText('Wait')).toBeTruthy()
    fireEvent.keyDown(input(), { key: 'Escape' })
    expect(clientMock.interruptAgent).toHaveBeenCalledTimes(2)
  })

  it('보낸 직후 · 첫 프레임 전의 Esc 도 그 턴 끝에 걷힌다(누산기가 이미 끝으로 읽고 있던 턴)', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    act(() => captured.onState!('live'))
    feedCompletedTurn()
    fireEvent.change(input(), { target: { value: 'hello' } })
    fireEvent.keyDown(input(), { key: 'Enter' })
    await flush()
    expect(screen.getByText('Wait')).toBeTruthy()

    fireEvent.keyDown(input(), { key: 'Escape' })
    await flush()
    expect(indicator()).not.toBeNull()

    // Only the end arrives with no frame that opened the turn (the accumulator goes end → end) — it is lifted by the end event itself.
    act(() => captured.onChunk!(turnEnd(2)))
    expect(screen.queryByText('Wait')).toBeNull()
    expect(interrupting()).toBe(false)
  })

  it('보낸 직후 끊은 턴의 끝과 다음 턴의 첫 프레임이 한 렌더로 묶여도 걷힌다 — 누산기는 앞 턴의 끝을 읽던 채였다', async () => {
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
    expect(screen.getByText('Wait')).toBeTruthy()
    expect(fireEvent.keyDown(input(), { key: 'Escape' })).toBe(false)
    expect(clientMock.interruptAgent).toHaveBeenCalledTimes(2)
    expect(indicator()).not.toBeNull()
  })

  it('보낸 글이 턴을 열지 않고 대기 목록에 서서 대기 표시가 꺼지면 걷힌다(턴 경계 없음)', async () => {
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
    expect(screen.queryByText('Wait')).toBeNull()
    expect(interrupting()).toBe(false)
  })

  it.each<[string, () => unknown]>([
    ['임대 거절(CONFLICT)', () => INPUT_LOCKED_REFUSAL],
    ['끊을 턴 없음', () => 'no turn to interrupt'],
  ])('끊기가 거절되면(%s) 걷히고 Esc 가 다시 끊는다', async (_label, error) => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    clientMock.interruptAgent = vi.fn(async () => {
      throw error()
    })
    await mountStreaming()
    fireEvent.keyDown(input(), { key: 'Escape' })
    expect(indicator()).not.toBeNull()

    await flush()
    expect(indicator()).toBeNull()
    expect(screen.getByText('Wait')).toBeTruthy()
    expect(warn).toHaveBeenCalled()
    fireEvent.keyDown(input(), { key: 'Escape' })
    expect(clientMock.interruptAgent).toHaveBeenCalledTimes(2)
  })

  it('같은 에이전트의 둘째 뷰가 붙어 옛 턴 끝을 replay 해도 걷히지 않는다 — 걷는 것은 라이브 턴 끝이다', async () => {
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
      '에이전트가 명부에서 사라짐(종료)',
      () => {
        agentStoreState.agents = []
        act(() => captured.onChunk!(delta(1, ' more')))
      },
    ],
    ['구독이 재요청을 소진함(error)', () => act(() => captured.onState!('error'))],
    ['연결 끊김', () => setConnection('down')],
  ])('부재가 되면(%s) 걷히고, 이 창의 끊기 명령이 다시 보낸다', async (_label, becomeUnavailable) => {
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

  it('둘째 뷰의 구독만 멈추면(error) 첫 뷰의 「중단하는 중」은 남고 다시 보내지 않는다 — 마지막 뷰까지 멈추면 걷힌다', async () => {
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
    ['닫힌 턴으로 끝나면 걷힌다', [delta(0, 'reply'), tag1(1, JSON.stringify({ type: 'MessageDone' }))], false],
    [
      '열린 턴으로 끝나면 남는다',
      [delta(0, 'reply'), tag1(1, JSON.stringify({ type: 'MessageDone' })), delta(2, 'streaming reply')],
      true,
    ],
  ])('첫 buffering 중에 끊기를 불렀고 그 replay 가 %s', async (_label, replay, stays) => {
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

  it('따라잡은 뒤의 같은 화신 재buffering 은 닫힌 턴으로 끝나도 걷지 않는다 — 보낸 직후라 새 턴의 첫 프레임이 아직 없다', async () => {
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

  it('새 화신(비우기)이 오면 걷힌다 — 새 화신의 이력이 같은 틱에 턴을 열어 두어도', async () => {
    await mountStreaming()
    fireEvent.keyDown(input(), { key: 'Escape' })
    await flush()

    act(() => {
      captured.onReset!()
      captured.onChunk!(delta(0, 'new incarnation'))
    })
    expect(interrupting()).toBe(false)
    expect(screen.getByText('Wait')).toBeTruthy()
    fireEvent.keyDown(input(), { key: 'Escape' })
    expect(clientMock.interruptAgent).toHaveBeenCalledTimes(2)
  })

  it('마운트 해제 · 에이전트 교체에 걷힌다 — 낡은 표시가 살아남지 않는다', async () => {
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

// ★Tool bundle glue (ADR-0239 · TRD S21-chat-ux §4-4 · §4-5)★: bundling · summary · toggle themselves are measured by toolRuns.test.ts ·
//   ToolGroupRow.test.tsx. Here we only measure that the slot passes its slot id down to the conversation view and is bound to the store (the head is alive and
//   the command reaches it) · the lifetime of the binding (mount · agent swap · new incarnation) · that expanding a non-last bundle releases follow.
describe('RichSlot(live) — 도구 묶음 접착(ADR-0239)', () => {
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
    if (el === null) throw new Error(`묶음 '${key}' 이 그려져 있지 않다`)
    return el
  }
  // The first button of the bundle root is the head — when expanded, the member rows' buttons stand after it.
  const header = (key: string): HTMLButtonElement => group(key).querySelector('button') as HTMLButtonElement
  const openAttr = (key: string): string | null => group(key).getAttribute('data-tool-group-open')
  const chosen = (key: string): boolean | undefined => useToolGroupStore.getState().bySlot.v1?.open[key]
  const followAttr = (): string | null =>
    document.querySelector('[data-radix-scroll-area-viewport]')!.getAttribute('data-scroll-follow')

  it('끝난 묶음의 머리가 살아 있고, 누르면 저장소를 거쳐 펼쳐졌다 접힌다', async () => {
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

  it('LLM command(chat.toolGroup.setExpanded)가 마운트된 슬롯에 닿고, 내려간 뒤에는 오류로 답한다', async () => {
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
      /마운트된 슬롯 'v1'/,
    )
  })

  it('마지막이 아닌 묶음을 펼치면 따라가기를 풀고, 마지막 묶음 · 접기는 붙음을 그대로 둔다(§4-5)', async () => {
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

  it('같은 에이전트로 다시 마운트하면 고른 펼침이 다시 붙고, 다른 에이전트가 오면 따라오지 않는다', async () => {
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

  // That an old release cannot release a new binding (the mount marker) is measured by `toolGroupStore.test.ts` — here React runs the old cleanup first,
  //   so that order cannot be told apart.
  it('한 인스턴스에서 에이전트가 바뀌면(key 재마운트) 새 에이전트로 다시 묶여 고른 펼침은 비고 새 머리가 적는다', async () => {
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

  it('새 화신의 비우기(onReset)는 고른 펼침을 비운다 — 같은 키의 묶음이 와도 기본(접힘)으로 그린다', async () => {
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

  it("같은 화신의 재replay('buffering' → 'live')는 고른 펼침을 지우지 않는다", async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    feedFinishedGroup(0, ['r1', 'r2'])
    fireEvent.click(header('tool:r1'))

    fireState('buffering')
    fireState('live')
    expect(openAttr('tool:r1')).toBe('1')
    expect(chosen('tool:r1')).toBe(true)
  })

  // ★Late tool end window (TRD §4-7 ⑧ · ⑨ frontend · ADR-0241)★: codex does not kill a running command even when interrupted, so the end of a previous turn's tool
  //   arrives late. If that end lands right after a send · before the first reply — the previous turn is closed so turnDone is true, and what holds up the wait indicator is
  //   awaiting alone. If `ToolResult` released awaiting, "Wait" would go off here with no reply.
  it('보낸 직후 지난 턴 도구의 끝(ToolResult)만 오면 "Wait" 이 남고 그 행에 표식이 붙는다 — 첫 진짜 답부터는 턴이 이어받는다', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    act(() => captured.onChunk!(toolCallFrame(0, 'c1')))
    act(() => captured.onChunk!(turnEndFrame(1)))
    expect(screen.queryByText('Wait')).toBeNull()

    const input = screen.getByPlaceholderText(/메시지 입력/)
    fireEvent.change(input, { target: { value: 'next' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    await flush()
    expect(screen.getByText('Wait')).toBeTruthy()

    act(() => captured.onChunk!(tag1(2, JSON.stringify({ type: 'ToolResult', id: 'c1', outcome: 'Failed' }))))
    expect(screen.getByText('Wait')).toBeTruthy()
    expect(document.querySelector('[data-tool-mark="failed"]')).not.toBeNull()

    // First real reply — awaiting is released and the indicator is held up by the open turn. The turn boundary turns it off (if awaiting were stuck it would not go off).
    act(() => captured.onChunk!(tag1(3, JSON.stringify({ type: 'TextDelta', text: 'answer' }))))
    expect(screen.getByText('Wait')).toBeTruthy()
    act(() => captured.onChunk!(turnEndFrame(4)))
    expect(screen.queryByText('Wait')).toBeNull()
  })
})
