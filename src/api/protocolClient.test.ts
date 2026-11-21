// ProtocolClient unit tests — carrier-independent protocol semantics + the view-direct replay state machine (ADR-0046). WS-specific (Auth/Hello/reconnect timing) is in wsTransport.test.
//
// TRD §5 scenarios pinned: staggered mount (ignore someone else's marker → flush on own gen) · fan-out+dedup of 2 views of the same agent (bug B) · sort+dedup restore with a live frame before the replay ·
//   marker token mismatch ignored (StrictMode) · marker before myGen (held→flush) · epoch rotation while buffering (discard+re-request · old-epoch markers ignored) · disconnect=detached (no requests) / attach trigger=roster ·
//   failure marker→ladder→error after 3 tries · watchdog re-request (fake timers) · rotation of an attached view (re-spawn without disconnect) · roster entry without a marker = rotation · roster absence → attached view detached ·
//   marker set by the success marker, not frames · per-view dedup independent · unsubscribe cleanup · seq continuity (ADR-0231): hold after a live gap, flush start = max(last+1, replay head), hold overflow → buffer phase.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { ProtocolClient } from './protocolClient'
import connectionCoreSource from '../../crates/engram-dashboard-daemon/src/connection_core.rs?raw'
import { StructuredEventAccumulator } from '../components/slot/structuredAccumulator'
import {
  INPUT_LOCKED_REFUSAL,
  type ConnectionState,
  type OutputChunk,
  type UsageSnapshotPull,
} from './agentClient'
import type { InboundMessage, Transport } from './transport'
import type { Band } from '../../src-tauri/bindings/Band'
import type { AgentInfo, AgentProfile, Preset, RestoreReport } from './types'
import type { UsageLimitSnapshot } from '../../crates/engram-dashboard-protocol/bindings/UsageLimitSnapshot'

class MockTransport implements Transport {
  sent: unknown[] = []
  private _state: ConnectionState
  private stateCbs = new Set<(s: ConnectionState) => void>()
  private msgCb: ((m: InboundMessage) => void) | null = null
  ensureReadyCalls = 0
  startCalls = 0
  closed = false
  replayCalls: Array<{ agentId: string; gen: bigint }> = []
  private replayGenCounter = 0n
  /** Controls what requestReplay returns; a planted function's Promise reproduces a delayed myGen settle (marker arriving first). */
  replayGenImpl: ((agentId: string, gen: bigint) => Promise<bigint>) | null = null

  ensureReadyImpl: (() => Promise<void>) | null = null

  constructor(initial: ConnectionState = 'connected') {
    this._state = initial
  }

  private _band: Band | null = null

  get connectionState(): ConnectionState {
    return this._state
  }
  get connectionBand(): Band | null {
    return this._band
  }
  onConnectionStateChange(cb: (s: ConnectionState) => void): () => void {
    this.stateCbs.add(cb)
    cb(this._state)
    return () => this.stateCbs.delete(cb)
  }
  onMessage(cb: (m: InboundMessage) => void): () => void {
    this.msgCb = cb
    return () => {
      if (this.msgCb === cb) this.msgCb = null
    }
  }
  send(payload: unknown): void {
    this.sent.push(payload)
  }
  ensureReady(): Promise<void> {
    this.ensureReadyCalls += 1
    if (this.ensureReadyImpl) return this.ensureReadyImpl()
    return Promise.resolve()
  }
  start(): Promise<void> {
    this.startCalls += 1
    return Promise.resolve()
  }
  close(): void {
    this.closed = true
  }
  requestReplay(agentId: string): Promise<bigint> {
    const gen = ++this.replayGenCounter
    this.replayCalls.push({ agentId, gen })
    if (this.replayGenImpl) return this.replayGenImpl(agentId, gen)
    return Promise.resolve(gen)
  }
  usagePull: UsageSnapshotPull = { socketEpoch: 0, snapshots: [] }
  getUsageSnapshot(): Promise<UsageSnapshotPull> {
    return Promise.resolve(this.usagePull)
  }

  // ── test driver ──
  deliver(msg: InboundMessage): void {
    this.msgCb?.(msg)
  }
  control(event: Record<string, unknown>): void {
    this.deliver({ kind: 'control', event })
  }
  output(agentId: string, epoch: number, seq: number, bytes = new Uint8Array([seq & 0xff]), tag = 0): void {
    this.deliver({ kind: 'output', tag, agentId, epoch, seq, bytes })
  }
  marker(
    agentId: string,
    epoch: number,
    gen: bigint,
    opts: {
      failed?: boolean
      truncated?: boolean
      continuesConversation?: boolean
      replayFrom?: number
    } = {},
  ): void {
    this.deliver({
      kind: 'replayBoundary',
      agentId,
      epoch,
      gen,
      truncated: opts.truncated ?? false,
      failed: opts.failed ?? false,
      continuesConversation: opts.continuesConversation ?? false,
      // Default 0 = an intact replay from 0 in the ring (no head jump — the starting point is `last+1`).
      replayFrom: opts.replayFrom ?? 0,
    })
  }
  setState(s: ConnectionState): void {
    this._state = s
    for (const cb of this.stateCbs) cb(s)
  }
  /** The shell's `(state, band)` status, as `TauriTransport` notifies it — a band change alone notifies too. */
  setStatus(s: ConnectionState, band: Band | null): void {
    this._band = band
    this.setState(s)
  }
  lastSent<T = Record<string, unknown>>(key: string): T | undefined {
    for (let i = this.sent.length - 1; i >= 0; i--) {
      const m = this.sent[i]
      if (m && typeof m === 'object' && key in (m as object)) return (m as Record<string, T>)[key]
    }
    return undefined
  }
}

const AGENT = '12345678-9abc-def0-1234-56789abcdef0'
const V1 = 'view-1'
const V2 = 'view-2'

let uuidCounter = 0
beforeEach(() => {
  uuidCounter = 0
  vi.spyOn(globalThis.crypto, 'randomUUID').mockImplementation(
    () => `req-${++uuidCounter}` as `${string}-${string}-${string}-${string}-${string}`,
  )
})
afterEach(() => {
  vi.restoreAllMocks()
})

// ── request_id pending matching (carrier-independent — no ADR-0046 impact) ─────────────────────────────
describe('request_id pending matching', () => {
  it('spawnAgent → sends SpawnByCwd{request_id} + resolves on Spawned{request_id,agent}', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const p = c.spawnAgent('C:/work', 'codex')
    await Promise.resolve()
    const sent = t.lastSent<{ request_id: string; cwd: string; backend: string }>('SpawnByCwd')!
    expect(sent.cwd).toBe('C:/work')
    // The picked word must be carried in the envelope — otherwise the daemon refuses (no default).
    expect(sent.backend).toBe('codex')
    t.control({ Spawned: { request_id: sent.request_id, agent: { id: 'a1' } } })
    expect(await p).toEqual({ id: 'a1' })
  })

  it('killAgent → resolves void on Ack{request_id}', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const p = c.killAgent('a1')
    await Promise.resolve()
    const rid = t.lastSent<{ request_id: string }>('Kill')!.request_id
    t.control({ Ack: { request_id: rid } })
    await expect(p).resolves.toBeUndefined()
  })

  it('rejects on Error{request_id}', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const p = c.killAgent('a1')
    await Promise.resolve()
    const rid = t.lastSent<{ request_id: string }>('Kill')!.request_id
    t.control({ Error: { request_id: rid, message: 'boom' } })
    await expect(p).rejects.toThrow('boom')
  })

  it('2 concurrent getSnapshot (same agent_id) are matched exactly by request_id too', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const p1 = c.getSnapshot(AGENT)
    const p2 = c.getSnapshot(AGENT)
    await Promise.resolve()
    const sent = t.sent.filter(
      (m): m is { GetSnapshot: { request_id: string } } =>
        !!m && typeof m === 'object' && 'GetSnapshot' in m,
    )
    expect(sent.length).toBe(2)
    const rid1 = sent[0].GetSnapshot.request_id
    const rid2 = sent[1].GetSnapshot.request_id
    t.control({ Snapshot: { request_id: rid2, agent_id: AGENT, chunks: [{ seq: 2 }] } })
    t.control({ Snapshot: { request_id: rid1, agent_id: AGENT, chunks: [{ seq: 1 }] } })
    await expect(p1).resolves.toEqual([{ seq: 1 }])
    await expect(p2).resolves.toEqual([{ seq: 2 }])
  })
})

// ── the two queued input commands (ADR-0231 — dedicated replies collected by request_id) ───────────────────────────
describe('queued input commands (ADR-0231) — ListQueuedInputs · CancelQueuedInput', () => {
  it('listQueuedInputs → ListQueuedInputs{agent_id,request_id} + resolves with QueuedInputs (as_of_seq is a number)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const p = c.listQueuedInputs(AGENT)
    await Promise.resolve()
    const sent = t.lastSent<{ agent_id: string; request_id: string }>('ListQueuedInputs')!
    expect(sent.agent_id).toBe(AGENT)
    const row = { id: 'q1', text: 'hi', state: 'queued', cancel: null }
    t.control({
      QueuedInputs: {
        request_id: sent.request_id,
        agent_id: AGENT,
        inputs: [row],
        as_of_seq: 41,
        epoch: 9,
        stopped_after_error: true,
      },
    })
    await expect(p).resolves.toEqual({ inputs: [row], as_of_seq: 41, epoch: 9, stopped_after_error: true })
  })

  it('as_of_seq null stays null · rows with unknown words are passed on uninterpreted (filtering is the consumer\'s job)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const p = c.listQueuedInputs(AGENT)
    await Promise.resolve()
    const rid = t.lastSent<{ request_id: string }>('ListQueuedInputs')!.request_id
    const odd = { id: 'q9', text: 'x', state: 'parked', cancel: { answer: 'maybe', vendor_closed: false } }
    t.control({
      QueuedInputs: { request_id: rid, agent_id: AGENT, inputs: [odd], as_of_seq: null, epoch: 1, stopped_after_error: false },
    })
    await expect(p).resolves.toEqual({ inputs: [odd], as_of_seq: null, epoch: 1, stopped_after_error: false })
  })

  it('cancelQueuedInput → CancelQueuedInput{agent_id,input_id,request_id} + resolves with the outcome word of QueuedInputCancelReply', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const p = c.cancelQueuedInput(AGENT, 'q1')
    await Promise.resolve()
    const sent = t.lastSent<{ agent_id: string; input_id: string; request_id: string }>('CancelQueuedInput')!
    expect(sent).toMatchObject({ agent_id: AGENT, input_id: 'q1' })
    t.control({
      QueuedInputCancelReply: { request_id: sent.request_id, agent_id: AGENT, input_id: 'q1', outcome: 'requested' },
    })
    await expect(p).resolves.toBe('requested')
  })

  it('an unknown outcome word is given as is without throwing', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const p = c.cancelQueuedInput(AGENT, 'q1')
    await Promise.resolve()
    const rid = t.lastSent<{ request_id: string }>('CancelQueuedInput')!.request_id
    t.control({ QueuedInputCancelReply: { request_id: rid, agent_id: AGENT, input_id: 'q1', outcome: 'deferred' } })
    await expect(p).resolves.toBe('deferred')
  })

  it('Error{request_id} rejects both commands — the lease refusal comes as the code-less text as is', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const list = c.listQueuedInputs(AGENT)
    const cancel = c.cancelQueuedInput(AGENT, 'q1')
    await Promise.resolve()
    const listRid = t.lastSent<{ request_id: string }>('ListQueuedInputs')!.request_id
    const cancelRid = t.lastSent<{ request_id: string }>('CancelQueuedInput')!.request_id
    t.control({ Error: { request_id: listRid, message: 'NOT_FOUND: agent is not running' } })
    t.control({ Error: { request_id: cancelRid, message: INPUT_LOCKED_REFUSAL } })
    await expect(list).rejects.toThrow('NOT_FOUND: agent is not running')
    await expect(cancel).rejects.toThrow(INPUT_LOCKED_REFUSAL)
  })

  it('the lease refusal text is byte-identical to the daemon constant (if they diverge, the CONFLICT mapping dies silently)', () => {
    expect(connectionCoreSource).toContain(`pub(crate) const INPUT_LOCKED_REFUSAL: &str = "${INPUT_LOCKED_REFUSAL}";`)
  })
})

// ── preset CRUD (ADR-0061 — wire command/reply matching) ─────────────────────────────────────
describe('preset CRUD (ADR-0061)', () => {
  it('listPresets → sends ListPresets{request_id} + resolves on PresetList{request_id,presets}', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const p = c.listPresets()
    await Promise.resolve()
    const rid = t.lastSent<{ request_id: string }>('ListPresets')!.request_id
    const presets = [{ id: 'pr1', cwd: 'C:/proj' }] as Preset[]
    t.control({ PresetList: { request_id: rid, presets } })
    expect(await p).toEqual(presets)
  })

  it('createPreset(cwd) → sends CreatePreset{cwd,request_id} + resolves void on Ack', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const p = c.createPreset('C:/work')
    await Promise.resolve()
    const sent = t.lastSent<{ request_id: string; cwd: string }>('CreatePreset')!
    expect(sent.cwd).toBe('C:/work')
    t.control({ Ack: { request_id: sent.request_id } })
    await expect(p).resolves.toBeUndefined()
  })

  it('deletePreset(id) → sends DeletePreset{preset_id,request_id} + resolves void on Ack', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const p = c.deletePreset('pr1')
    await Promise.resolve()
    const sent = t.lastSent<{ request_id: string; preset_id: string }>('DeletePreset')!
    expect(sent.preset_id).toBe('pr1')
    t.control({ Ack: { request_id: sent.request_id } })
    await expect(p).resolves.toBeUndefined()
  })

  it('renamePreset(id, name) → sends RenamePreset{preset_id,name,request_id} + resolves void on Ack', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const p = c.renamePreset('pr1', 'my preset')
    await Promise.resolve()
    const sent = t.lastSent<{ request_id: string; preset_id: string; name: string | null }>('RenamePreset')!
    expect(sent.preset_id).toBe('pr1')
    expect(sent.name).toBe('my preset')
    t.control({ Ack: { request_id: sent.request_id } })
    await expect(p).resolves.toBeUndefined()
  })

  it('renamePreset(id, null) → sends name=null (clear the override)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const p = c.renamePreset('pr1', null)
    await Promise.resolve()
    const sent = t.lastSent<{ request_id: string; preset_id: string; name: string | null }>('RenamePreset')!
    expect(sent.name).toBeNull()
    t.control({ Ack: { request_id: sent.request_id } })
    await expect(p).resolves.toBeUndefined()
  })

  it('renameProfile(id, name) → sends RenameProfile{profile_id,name,request_id} + resolves void on Ack', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const p = c.renameProfile('a1', 'my agent')
    await Promise.resolve()
    const sent = t.lastSent<{ request_id: string; profile_id: string; name: string | null }>('RenameProfile')!
    expect(sent.profile_id).toBe('a1')
    expect(sent.name).toBe('my agent')
    t.control({ Ack: { request_id: sent.request_id } })
    await expect(p).resolves.toBeUndefined()
  })
})

// ── profile creation — measures **the line that assembles** the wire command ───────────────────────────────────
//
// ★Does not overlap `agentCommands.test.ts`★ — that one mocks the client and never sees the `CreateProfile` envelope assembled here; whether the backend word and mode go out to the socket is measured only here.
describe('profile creation (CreateProfile envelope)', () => {
  it('createCodexProfile(outputFormat) → carries backend=codex + the picked mode as is', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const profile = { id: 'p1' } as AgentProfile
    const p = c.createCodexProfile('n', 'C:/work', ['--foo'], [], false, 'StreamJson')
    await Promise.resolve()
    const sent = t.lastSent<{
      request_id: string
      backend: string
      output_format: string
      extra_args: string[]
    }>('CreateProfile')!
    expect(sent.backend).toBe('codex')
    expect(sent.output_format).toBe('StreamJson')
    expect(sent.extra_args).toEqual(['--foo'])
    t.control({ Created: { request_id: sent.request_id, profile } })
    await expect(p).resolves.toEqual(profile)
  })

  // An old caller giving no mode — if this loses `Terminal`, that caller silently switches to app-server.
  it('createCodexProfile (mode omitted) → output_format=Terminal', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    void c.createCodexProfile('n', 'C:/work', [], [], false)
    await Promise.resolve()
    expect(t.lastSent<{ output_format: string }>('CreateProfile')!.output_format).toBe('Terminal')
  })

  // Sibling — these two together catch an edit that wrongly pins one side's word on the other.
  it('createClaudeProfile(outputFormat) → backend=claude + the picked mode', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    void c.createClaudeProfile('n', 'C:/work', [], [], false, 'StreamJson')
    await Promise.resolve()
    const sent = t.lastSent<{ backend: string; output_format: string }>('CreateProfile')!
    expect(sent.backend).toBe('claude')
    expect(sent.output_format).toBe('StreamJson')
  })
})

// ── subscribeOutput basic wiring (per view, ADR-0046) ─────────────────────────────────────
describe('subscribeOutput basics (per-view replay)', () => {
  it('subscribe → issues requestReplay (once per view) + sends no wire Subscribe (BLOCK-1)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    await c.subscribeOutput(V1, AGENT, () => {})
    expect(t.replayCalls.map((r) => r.agentId)).toEqual([AGENT])
    const subs = t.sent.filter((m) => !!m && typeof m === 'object' && 'Subscribe' in (m as object))
    expect(subs.length).toBe(0)
  })

  it('buffering before the success marker (no direct delivery), after the marker live transition+flush', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
    const gen = t.replayCalls[0].gen
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    expect(got).toEqual([])
    t.marker(AGENT, 1, gen)
    expect(got).toEqual([0, 1])
    t.output(AGENT, 1, 2)
    expect(got).toEqual([0, 1, 2])
  })

  it('passes the tag to onChunk as is (tag0/tag1) + dedup over one seq space', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const chunks: OutputChunk[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => chunks.push(chunk))
    const gen = t.replayCalls[0].gen
    t.output(AGENT, 1, 0, new Uint8Array([1]), 0)
    t.output(AGENT, 1, 1, new Uint8Array([2]), 1)
    t.marker(AGENT, 1, gen)
    t.output(AGENT, 1, 1, new Uint8Array([2]), 0)
    t.output(AGENT, 1, 2, new Uint8Array([3]), 1)
    expect(chunks.map((x) => x.seq)).toEqual([0, 1, 2])
    expect(chunks.map((x) => x.tag)).toEqual([0, 1, 1])
  })
})

// ── staggered mount (a view that received only the tail of an in-progress replay ignores someone else's marker → full flush on its own gen marker) ──────
describe("staggered mount — ignore someone else's marker, full flush on its own gen marker (gen fence)", () => {
  it("the replay tail of the view mounted first + someone else's marker (gen<myGen) are ignored, full flush on its own gen marker", async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const g1got: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => g1got.push(chunk.seq))
    const gen1 = t.replayCalls[0].gen
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    const g2got: number[] = []
    await c.subscribeOutput(V2, AGENT, (chunk) => g2got.push(chunk.seq))
    const gen2 = t.replayCalls[1].gen
    t.output(AGENT, 1, 2) // V1 replay tail = V2 buffer head
    t.marker(AGENT, 1, gen1)
    expect(g1got).toEqual([0, 1, 2])
    expect(g2got).toEqual([])
    // V2's own whole replay (full re-replay after the single-flight merge) → seq 0,1,2 resent + closing gen2 marker.
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    t.output(AGENT, 1, 2)
    t.marker(AGENT, 1, gen2)
    expect(g2got).toEqual([0, 1, 2])
    expect(g1got).toEqual([0, 1, 2])
  })
})

// ── independent fan-out + dedup of 2 views of the same agent (bug B regression) ──────────────────────────────────
describe('independent fan-out of 2 views of the same agent + per-view dedup (bug B regression)', () => {
  it('both views receive everything with independent progress (one view does not pollute the other\'s progress)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const g1: number[] = []
    const g2: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => g1.push(chunk.seq))
    const gen1 = t.replayCalls[0].gen
    await c.subscribeOutput(V2, AGENT, (chunk) => g2.push(chunk.seq))
    const gen2 = t.replayCalls[1].gen
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    t.marker(AGENT, 1, gen1)
    t.marker(AGENT, 1, gen2)
    expect(g1).toEqual([0, 1])
    expect(g2).toEqual([0, 1])
    t.output(AGENT, 1, 2)
    expect(g1).toEqual([0, 1, 2])
    expect(g2).toEqual([0, 1, 2])
  })

  it("per-view dedup independent — even when one view is live, unrelated to the other view's dedup high-water", async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const g1: number[] = []
    const g2: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => g1.push(chunk.seq))
    const gen1 = t.replayCalls[0].gen
    t.output(AGENT, 1, 0)
    t.marker(AGENT, 1, gen1)
    expect(g1).toEqual([0])
    await c.subscribeOutput(V2, AGENT, (chunk) => g2.push(chunk.seq))
    const gen2 = t.replayCalls[1].gen
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    t.marker(AGENT, 1, gen2)
    expect(g2).toEqual([0, 1])
    expect(g1).toEqual([0, 1]) // V1 is live, so only seq 1 is new (0 is dedup)
  })
})

// ── sort+dedup restore even when a live frame comes before the replay (marker) ────────────────────────────
describe('out-of-order frame sort+dedup (order restore)', () => {
  it('out-of-order [2,0,1] arrives in the buffer → delivered in seq order (0,1,2) at the marker flush', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
    const gen = t.replayCalls[0].gen
    // Arrival order differs from seq order — flushing in array order without sorting would deliver 2 first and drop 0,1 by dedup.
    t.output(AGENT, 1, 2)
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    t.marker(AGENT, 1, gen)
    expect(got).toEqual([0, 1, 2])
  })
})

// ── ignore a marker token mismatch (StrictMode dead subscription) ──────────────────────────────────────
describe('ignore a marker token mismatch (StrictMode resubscribe)', () => {
  it('an old subscription replaced by a resubscribe does not consume the marker (only the surviving subscription flushes)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const g1: number[] = []
    const g2: number[] = []
    const p1 = c.subscribeOutput(V1, AGENT, (chunk) => g1.push(chunk.seq))
    const p2 = c.subscribeOutput(V1, AGENT, (chunk) => g2.push(chunk.seq))
    await Promise.all([p1, p2])
    expect(t.replayCalls.length).toBe(1)
    const gen = t.replayCalls[0].gen
    t.output(AGENT, 1, 0)
    t.marker(AGENT, 1, gen)
    expect(g2).toEqual([0])
    expect(g1).toEqual([])
  })
})

// ── marker arrives before myGen settles (held → re-evaluation flush) — NEW-3 ─────────────────────────
describe('marker arrives before myGen settles (held → flush)', () => {
  it('marker arrives while the requestReplay return is delayed → kept, flushed when myGen settles', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    // The requestReplay return is delayed with a gate — reproduces the pipe crossing where the marker comes before myGen.
    let releaseGen!: (gen: bigint) => void
    t.replayGenImpl = () => new Promise<bigint>((r) => (releaseGen = r))
    const got: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
    const gen = t.replayCalls[0].gen
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    t.marker(AGENT, 1, gen)
    expect(got).toEqual([])
    releaseGen(gen)
    await Promise.resolve()
    await Promise.resolve()
    expect(got).toEqual([0, 1])
  })

  it('held keeps only the 1 highest gen — a lower gen marker does not overwrite a higher one', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    let releaseGen!: (gen: bigint) => void
    t.replayGenImpl = () => new Promise<bigint>((r) => (releaseGen = r))
    const got: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
    const myGen = t.replayCalls[0].gen
    t.output(AGENT, 1, 0)
    t.marker(AGENT, 1, myGen - 1n)
    t.marker(AGENT, 1, myGen)
    releaseGen(myGen)
    await Promise.resolve()
    await Promise.resolve()
    expect(got).toEqual([0])
  })

  // ── FIX-3: same gen failed→success replacement (zombie late-Complete recovery) ──────────────────────
  it('a following success marker replaces the held failed marker of the same gen → flush when myGen settles (not the ladder)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    let releaseGen!: (gen: bigint) => void
    t.replayGenImpl = () => new Promise<bigint>((r) => (releaseGen = r))
    const got: number[] = []
    const states: string[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq), (s) => states.push(s))
    const myGen = t.replayCalls[0].gen
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    // While myGen is unsettled: a gen's failure marker (deadline) comes first → held; then the same gen's success marker (late Complete) → FIX-3 replacement rule (same gen && held.failed && !m.failed) lets success push out failure.
    t.marker(AGENT, 1, myGen, { failed: true })
    t.marker(AGENT, 1, myGen, { failed: false })
    // myGen settles → held (success) re-evaluated → flush (live). Had the failure won, it would fall to the ladder, keep buffering, and got would be empty.
    releaseGen(myGen)
    await Promise.resolve()
    await Promise.resolve()
    expect(got).toEqual([0, 1])
    expect(c.getViewOutputState(V1)?.phase).toBe('live')
    expect(states).toContain('live')
  })
})

// ── incarnation marker = opaque (no magnitude comparison) ────────────────────────────────────────────────
//
// The marker is a random number per incarnation, so magnitude does not mean "newer": the frame-side rule is **mismatch = not mine (drop)** and only the authoritative roster declares the incarnation changed.
// Reviving a magnitude comparison would ignore a whole new incarnation whose marker happens to be smaller.
describe('frames with a different incarnation marker are not delivered (magnitude is not looked at)', () => {
  it('a frame with a **larger** marker is also dropped without delivery or rotation (the old "epoch rise = re-spawn" path removed)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got: number[] = []
    const states: string[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq), (s) => states.push(s))
    const gen1 = t.replayCalls[0].gen
    await Promise.resolve() // myGen=gen1 settled
    t.output(AGENT, 1, 0) // this frame pins the view's marker to 1
    t.output(AGENT, 2, 5) // another incarnation — dropped (no re-request, no notification)
    expect(t.replayCalls.length).toBe(1)
    expect(states).toEqual([])

    t.output(AGENT, 1, 1)
    t.marker(AGENT, 1, gen1)
    expect(got).toEqual([0, 1]) // only the frames of marker 1
  })

  it('a frame with a **smaller** marker is dropped by the same rule', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
    const gen1 = t.replayCalls[0].gen
    await Promise.resolve()
    t.output(AGENT, 7, 0)
    t.output(AGENT, 3, 1) // a smaller marker — dropped under the old rule too, and dropped now
    t.marker(AGENT, 7, gen1)
    expect(got).toEqual([0])
  })

  it('a live view does not eat frames with a different marker either', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
    t.output(AGENT, 4, 0)
    t.marker(AGENT, 4, t.replayCalls[0].gen)
    expect(c.getViewOutputState(V1)?.phase).toBe('live')

    t.output(AGENT, 9, 1) // larger marker
    t.output(AGENT, 2, 2) // smaller marker
    expect(got).toEqual([0])
  })
})

// ── disconnect = detached · attach trigger = roster ────────────────────────────────────────────────
//
// ★The subscription trigger is the roster, not the socket★: a disconnect only settles into detached, and requests go out only on observing "the agent is in the roster". A socket re-transition trigger would fire a full
//   Subscribe at a restarted daemon's empty roster (no automatic restore at boot) and all would be refused — do not revive it.
describe('when the connection drops, views go detached — keep screen and progress and send nothing', () => {
  it("no re-request goes out on disconnect, only onState('detached') comes", async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const states: string[] = []
    await c.subscribeOutput(V1, AGENT, () => {}, (s) => states.push(s))
    t.marker(AGENT, 1, t.replayCalls[0].gen)
    expect(states).toEqual(['live'])
    const callsBefore = t.replayCalls.length

    t.setState('reconnecting')
    expect(t.replayCalls.length).toBe(callsBefore)
    expect(states).toEqual(['live', 'detached'])
    expect(c.getViewOutputState(V1)?.phase).toBe('detached')
  })

  it('the socket coming back alone does not re-request (waits for the roster)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    await c.subscribeOutput(V1, AGENT, () => {})
    const callsBefore = t.replayCalls.length

    t.setState('reconnecting')
    t.setState('connected')
    expect(t.replayCalls.length).toBe(callsBefore)
    expect(c.getViewOutputState(V1)?.phase).toBe('detached')
  })

  // ★The root of the refusal storm★: a restarted daemon's roster lacks that agent; Subscribe at an absent agent is refused, so while absence is confirmed nothing is sent and the waiting state stays on the surface.
  it('the view of an agent not in the roster stays detached and sends no Subscribe', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    await c.subscribeOutput(V1, AGENT, () => {})
    const callsBefore = t.replayCalls.length

    t.setState('reconnecting')
    t.setState('connected')
    t.control({ AgentListUpdated: { agents: [{ id: 'other-agent' }] } })

    expect(t.replayCalls.length).toBe(callsBefore)
    expect(c.getViewOutputState(V1)?.phase).toBe('detached')
  })

  it("a detached view does not eat someone else's replay fan-out frames (prevents progress pollution)", async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
    t.output(AGENT, 1, 0)
    t.marker(AGENT, 1, t.replayCalls[0].gen)
    expect(got).toEqual([0])

    t.setState('reconnecting')
    t.output(AGENT, 1, 1) // the situation where another view's replay of the same agent reaches here through fan-out
    expect(got).toEqual([0])
  })

  // ★mount uses the same rule as re-attach★: the old unconditional mount fire hardened a slot pointing at a missing agent into refusal → ladder → error. The three cases below are that rule — folding "unknown" into "absent" blocks output on boot, reload and query failure.
  it('mount: sends as is when the roster is not known yet (unknown ≠ absent)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const states: string[] = []
    await c.subscribeOutput(V1, AGENT, () => {}, (s) => states.push(s))
    expect(t.replayCalls.length).toBe(1)
    expect(states).toEqual([]) // no waiting display either — the slot just stays as it is now
    expect(c.getViewOutputState(V1)?.phase).toBe('buffering')
  })

  it('mount: sends when the roster is known and the agent is in it', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    t.control({ AgentListUpdated: { agents: [{ id: AGENT, epoch: 4 }] } })
    await c.subscribeOutput(V1, AGENT, () => {})
    expect(t.replayCalls.length).toBe(1)
    expect(c.getViewOutputState(V1)?.phase).toBe('buffering')
  })

  it('mount: sends nothing when the roster is known and the agent is not in it', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const states: string[] = []
    t.control({ AgentListUpdated: { agents: [{ id: 'other-agent', epoch: 1 }] } })
    await c.subscribeOutput(V1, AGENT, () => {}, (s) => states.push(s))
    expect(t.replayCalls.length).toBe(0)
    expect(states).toEqual(['detached'])
    expect(c.getViewOutputState(V1)?.phase).toBe('detached')

    // And when the agent appears in the roster, it attaches then.
    t.control({ AgentListUpdated: { agents: [{ id: AGENT, epoch: 1 }] } })
    expect(t.replayCalls.length).toBe(1)
    expect(c.getViewOutputState(V1)?.phase).toBe('buffering')
  })

  // On disconnect the roster knowledge is dropped too — a dead daemon's roster used as a fact about a live daemon would read agents that vanished after a restart as alive.
  it('on disconnect the roster knowledge is dropped, so a later mount takes the "unknown" rule again', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    t.control({ AgentListUpdated: { agents: [{ id: 'other-agent', epoch: 1 }] } })
    t.setState('reconnecting')
    t.setState('connected')
    await c.subscribeOutput(V1, AGENT, () => {})
    expect(t.replayCalls.length).toBe(1)
  })

  it('connected→non-connected transition rejects pending commands (connection lost)', async () => {
    const t = new MockTransport('connected')
    const c = new ProtocolClient(t)
    const p = c.killAgent('a1')
    await Promise.resolve()
    t.setState('reconnecting')
    await expect(p).rejects.toThrow('connection lost')
  })
})

describe('roster attach — continuing the same incarnation vs rebuilding for a different incarnation', () => {
  // The socket only blinked (roster marker unchanged) → original ADR-0046 behaviour: keep the cursor and issue a full re-replay; dedup absorbs the overlapping front and only the frames missed while disconnected are drawn.
  it('when the roster marker is unchanged, it continues with the cursor kept (no clear signal)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got: number[] = []
    const states: string[] = []
    let resets = 0
    await c.subscribeOutput(
      V1,
      AGENT,
      (chunk) => got.push(chunk.seq),
      (s) => states.push(s),
      () => (resets += 1),
    )
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    t.marker(AGENT, 1, t.replayCalls[0].gen)
    expect(got).toEqual([0, 1])

    t.setState('reconnecting')
    t.setState('connected')
    t.control({ AgentListUpdated: { agents: [{ id: AGENT, epoch: 1 }] } })

    expect(t.replayCalls.length).toBe(2)
    const gen2 = t.replayCalls[1].gen
    await Promise.resolve() // myGen=gen2 settled
    t.output(AGENT, 1, 0) // the front part of the full re-replay — already drawn, so dropped by dedup
    t.output(AGENT, 1, 1)
    t.output(AGENT, 1, 2) // only the frames missed while disconnected are newly delivered
    t.marker(AGENT, 1, gen2)
    expect(got).toEqual([0, 1, 2])
    // There must never be a clear signal = the consumer does not clear the screen.
    expect(resets).toBe(0)
    expect(states).toEqual(['live', 'detached', 'buffering', 'live'])
  })

  // ★The common ending of a daemon restart, re-spawn and selective restore★: a reappearing agent is a different incarnation numbering frames from 0 again; without dropping the cursor everything is buried under the old high-water
  //   and the slot cannot draw a byte until relaunch (an empty-stub slot).
  it('when the roster marker changes, the cursor is dropped and set up again (clear signal = right before delivery)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got: number[] = []
    const states: string[] = []
    // Clearing and delivery are recorded **mixed in one line** — counted separately, "clear first" could not be measured, and that order is exactly "no empty screen in between".
    const order: string[] = []
    await c.subscribeOutput(
      V1,
      AGENT,
      (chunk) => {
        got.push(chunk.seq)
        order.push(`chunk:${chunk.seq}`)
      },
      (s) => states.push(s),
      () => order.push('reset'),
    )
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    t.output(AGENT, 1, 2)
    t.marker(AGENT, 1, t.replayCalls[0].gen)
    expect(got).toEqual([0, 1, 2]) // cursor = 2

    t.setState('reconnecting')
    t.setState('connected')
    t.control({ AgentListUpdated: { agents: [] } }) // a restarted daemon's empty roster = absence
    expect(t.replayCalls.length).toBe(1) // nothing is sent yet
    // ★It came back with a changed marker★ — the ground for the decision is **difference**, not order (a deliberately smaller value here).
    t.control({ AgentListUpdated: { agents: [{ id: AGENT, epoch: 0 }] } })

    expect(t.replayCalls.length).toBe(2)
    // Not cleared yet at request time — it is unknown whether this replay will really come.
    expect(states).toEqual(['live', 'detached', 'buffering'])
    expect(order).toEqual(['chunk:0', 'chunk:1', 'chunk:2'])
    const gen2 = t.replayCalls[1].gen
    await Promise.resolve() // myGen=gen2 settled
    t.output(AGENT, 0, 0) // new incarnation — numbers start again from 0
    t.output(AGENT, 0, 1)
    expect(got).toEqual([0, 1, 2]) // still buffering — the screen stays
    t.marker(AGENT, 0, gen2)
    // At the moment it arrives the clear signal goes out, and **right after it** the new history is delivered (no empty screen in between).
    expect(order).toEqual(['chunk:0', 'chunk:1', 'chunk:2', 'reset', 'chunk:0', 'chunk:1'])
    expect(states).toEqual(['live', 'detached', 'buffering', 'live'])
    expect(got).toEqual([0, 1, 2, 0, 1])
    expect(c.getViewOutputState(V1)?.phase).toBe('live')
    // The cursor must be set up again on the new incarnation for later live frames to connect too (dropping without losing the advance).
    t.output(AGENT, 0, 2)
    t.output(AGENT, 0, 2) // a re-arrival of the same seq is still dedup
    expect(got).toEqual([0, 1, 2, 0, 1, 2])
  })

  // ★Why clearing must not happen at request time★: right after a daemon restart the attach goes out but the replay is refused; had the screen been cleared then, the slot would stay empty forever.
  it('when the attached replay fails, it does not ask to clear the screen (the previous incarnation\'s screen stays)', async () => {
    vi.useFakeTimers()
    try {
      const t = new MockTransport()
      const c = new ProtocolClient(t)
      const got: number[] = []
      const states: string[] = []
      let resets = 0
      await c.subscribeOutput(
        V1,
        AGENT,
        (chunk) => got.push(chunk.seq),
        (s) => states.push(s),
        () => (resets += 1),
      )
      t.output(AGENT, 1, 0)
      t.marker(AGENT, 1, t.replayCalls[0].gen)
      expect(got).toEqual([0])

      t.setState('reconnecting')
      t.setState('connected')
      t.control({ AgentListUpdated: { agents: [{ id: AGENT, epoch: 2 }] } }) // the marker changed
      await Promise.resolve()
      // The ladder is exhausted to the end — at no step does a failure call for a clear.
      for (let i = 0; i < 4; i++) {
        t.marker(AGENT, 2, t.replayCalls[t.replayCalls.length - 1].gen, { failed: true })
        await vi.advanceTimersByTimeAsync(5000)
      }
      expect(c.getViewOutputState(V1)?.phase).toBe('error')
      expect(resets).toBe(0)
      expect(got).toEqual([0]) // what was drawn stays
    } finally {
      vi.useRealTimers()
    }
  })

  // The rotation of an attached view is issued by the roster too — the frame side simply drops a mismatch, so there is no other place to recognize it.
  it('even when re-spawned without a disconnect, the new roster marker sets up the live view again', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
    t.output(AGENT, 1, 0)
    t.marker(AGENT, 1, t.replayCalls[0].gen)
    expect(c.getViewOutputState(V1)?.phase).toBe('live')

    t.control({ AgentListUpdated: { agents: [{ id: AGENT, epoch: 77 }] } })
    expect(t.replayCalls.length).toBe(2)
    expect(c.getViewOutputState(V1)?.phase).toBe('buffering')
    const gen2 = t.replayCalls[1].gen
    await Promise.resolve()
    t.output(AGENT, 77, 0)
    t.marker(AGENT, 77, gen2)
    expect(got).toEqual([0, 0])
  })

  // ★The clear signal must reach **an already open view** too★: the incarnation is out of the resubscribe trigger, so this is the only place that recognizes a re-spawn and resets the cursor. The case above is "disconnected and back";
  //   this is the path where the socket never disconnected — from the slot nothing seems to happen, so it is easy to miss.
  it('a live view that never disconnected also gets the clear signal on a re-spawn and draws the new history', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const order: string[] = []
    await c.subscribeOutput(
      V1,
      AGENT,
      (chunk) => order.push(`chunk:${chunk.seq}`),
      undefined,
      () => order.push('reset'),
    )
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    t.marker(AGENT, 1, t.replayCalls[0].gen)
    expect(order).toEqual(['chunk:0', 'chunk:1'])

    t.control({ AgentListUpdated: { agents: [{ id: AGENT, epoch: 77 }] } })
    expect(t.replayCalls.length).toBe(2)
    const gen2 = t.replayCalls[1].gen
    await Promise.resolve()
    // New incarnation — numbers come again from 0. Had the cursor not been dropped, all would be dropped by dedup.
    t.output(AGENT, 77, 0)
    t.output(AGENT, 77, 1)
    expect(order).toEqual(['chunk:0', 'chunk:1']) // still buffering — screen unchanged
    t.marker(AGENT, 77, gen2)
    expect(order).toEqual(['chunk:0', 'chunk:1', 'reset', 'chunk:0', 'chunk:1'])
  })

  // ★A roster entry without a marker is not assumed "the same incarnation"★: that assumption was the original defect (the kept cursor dedups away the whole new incarnation from 0, the slot stays empty forever);
  //   a wrong rotation costs one full replay that is invisible (clear and redraw on the same tick).
  it('a roster entry without a marker is treated as a rotation (the cursor is not silently kept)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const order: string[] = []
    await c.subscribeOutput(
      V1,
      AGENT,
      (chunk) => order.push(`chunk:${chunk.seq}`),
      undefined,
      () => order.push('reset'),
    )
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    t.marker(AGENT, 1, t.replayCalls[0].gen)
    expect(order).toEqual(['chunk:0', 'chunk:1'])

    t.control({ AgentListUpdated: { agents: [{ id: AGENT }] } }) // no marker
    expect(t.replayCalls.length).toBe(2)
    const gen2 = t.replayCalls[1].gen
    await Promise.resolve()
    t.output(AGENT, 3, 0)
    t.marker(AGENT, 3, gen2)
    expect(order).toEqual(['chunk:0', 'chunk:1', 'reset', 'chunk:0'])
  })

  // ★When the roster says "absent", an attached view is lowered too★: otherwise a slot watching a reaped agent stays live in getViewOutputState (§5 automation contract) while the screen has stopped. Screen and cursor stay — a reappearance is a continuation.
  it('when it disappears from the roster, a live view also settles into detached and keeps screen and cursor', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got: number[] = []
    const states: string[] = []
    let resets = 0
    await c.subscribeOutput(
      V1,
      AGENT,
      (chunk) => got.push(chunk.seq),
      (s) => states.push(s),
      () => (resets += 1),
    )
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    t.marker(AGENT, 1, t.replayCalls[0].gen)
    expect(c.getViewOutputState(V1)?.phase).toBe('live')

    t.control({ AgentListUpdated: { agents: [{ id: 'other-agent', epoch: 1 }] } })
    expect(c.getViewOutputState(V1)?.phase).toBe('detached')
    expect(states).toEqual(['live', 'detached'])
    expect(resets).toBe(0) // the screen is not touched
    expect(t.replayCalls.length).toBe(1) // no request to what is absent

    // Coming back with the same marker is a continuation — the cursor remains, so dedup absorbs the overlapping front part.
    t.control({ AgentListUpdated: { agents: [{ id: AGENT, epoch: 1 }] } })
    const gen2 = t.replayCalls[1].gen
    await Promise.resolve()
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 2)
    t.marker(AGENT, 1, gen2)
    expect(got).toEqual([0, 1, 2])
    expect(resets).toBe(0)
  })

  // ★The marker is set by the success marker, not by frames★: settling the incarnation from the first frame to arrive would let one late frame of the previous incarnation win, **every frame of the real new incarnation**
  //   would then be dropped as a mismatch, and the view would exhaust the ladder and settle in error (one frame kills the slot).
  it('even when late frames of the previous incarnation mix in right after attach, it draws the new incarnation history intact', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got: string[] = []
    const dec = new TextDecoder()
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push(dec.decode(chunk.bytes)))
    await Promise.resolve() // myGen settled
    const gen = t.replayCalls[0].gen

    // The tail of the previous incarnation (marker 5) arrives first — it does not belong to the replay this view claimed.
    t.output(AGENT, 5, 0, new TextEncoder().encode('stale'))
    // Then the full replay of the real new incarnation (marker 6).
    t.output(AGENT, 6, 0, new TextEncoder().encode('fresh-0'))
    t.output(AGENT, 6, 1, new TextEncoder().encode('fresh-1'))
    t.marker(AGENT, 6, gen)

    expect(c.getViewOutputState(V1)?.phase).toBe('live')
    expect(got).toEqual(['fresh-0', 'fresh-1'])
  })

  // ★The pulling door attaches too★: the first roster after a reconnect comes through this query, not a broadcast (eventBus.resyncAfterReconnect); the reply bypasses the broadcast routing, so omitting it means the main recovery path never attaches.
  it('attaches from the roster query (getAgents) result too', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    await c.subscribeOutput(V1, AGENT, () => {})
    t.setState('reconnecting')
    t.setState('connected')
    expect(c.getViewOutputState(V1)?.phase).toBe('detached')

    const p = c.getAgents()
    await Promise.resolve()
    const rid = t.lastSent<{ request_id: string }>('ListAgents')!.request_id
    t.control({ AgentList: { request_id: rid, agents: [{ id: AGENT }] } })
    await p

    expect(t.replayCalls.length).toBe(2)
    expect(c.getViewOutputState(V1)?.phase).toBe('buffering')
  })

  // ADR-0145: an attach is also the start of a restore window — without the notification the consumer keeps the screen, mistakes it for 'restore done', and flips at the re-flush.
  it("an attached view gets the 'live' notification again after the re-flush", async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const states: string[] = []
    await c.subscribeOutput(V1, AGENT, () => {}, (s) => states.push(s))
    t.marker(AGENT, 1, t.replayCalls[0].gen)
    t.setState('reconnecting')
    t.setState('connected')
    t.control({ AgentListUpdated: { agents: [{ id: AGENT }] } })

    const gen2 = t.replayCalls[1].gen
    await Promise.resolve() // myGen=gen2 settled
    t.marker(AGENT, 1, gen2)
    expect(states).toEqual(['live', 'detached', 'buffering', 'live'])
  })

  // A view settled in error gets a recovery chance once "the agent really exists" is confirmed (the roster attach takes over what the old full reset on reconnect gave).
  it('a view settled in error also resets the ladder on a roster attach and comes back', async () => {
    vi.useFakeTimers()
    try {
      const t = new MockTransport()
      const c = new ProtocolClient(t)
      await c.subscribeOutput(V1, AGENT, () => {})
      await Promise.resolve()
      for (let i = 0; i < 3; i++) {
        t.marker(AGENT, 1, t.replayCalls[t.replayCalls.length - 1].gen, { failed: true })
        await vi.advanceTimersByTimeAsync(5000)
      }
      t.marker(AGENT, 1, t.replayCalls[t.replayCalls.length - 1].gen, { failed: true })
      expect(c.getViewOutputState(V1)?.phase).toBe('error')

      t.setState('reconnecting')
      t.setState('connected')
      t.control({ AgentListUpdated: { agents: [{ id: AGENT }] } })
      expect(c.getViewOutputState(V1)?.phase).toBe('buffering')
      expect(c.getViewOutputState(V1)?.attempts).toBe(0)
    } finally {
      vi.useRealTimers()
    }
  })
})

// ── connection failure reason (ADR-0134) ────────────────────────────────────────────────────────
describe('the connection failure reason is carried on the connection state surface too', () => {
  it('subscribers are notified when a reason appears even if the state stays the same', () => {
    const t = new MockTransport('down')
    const c = new ProtocolClient(t)
    const seen: Array<ConnectionState> = []
    c.onConnectionStateChange((s) => seen.push(s))
    expect(seen).toEqual(['down']) // once right at registration
    expect(c.connectionError).toBeNull()

    c.reportConnectionError('cannot write to the data folder (C:\\x): Access is denied. (os error 5) — please extract to a writable location')

    // The state string stays 'down', but it must be notified again for the screen to draw the reason.
    expect(seen).toEqual(['down', 'down'])
    expect(c.connectionError).toContain('writable location')
  })

  it('reporting the same reason again does not notify', () => {
    const t = new MockTransport('down')
    const c = new ProtocolClient(t)
    const seen: Array<ConnectionState> = []
    c.onConnectionStateChange((s) => seen.push(s))
    c.reportConnectionError('same reason')
    c.reportConnectionError('same reason')
    expect(seen.length).toBe(2) // first 1 + change 1
  })

  it('the reason is cleared on the transition to connected', () => {
    const t = new MockTransport('down')
    const c = new ProtocolClient(t)
    c.reportConnectionError('folder problem')
    expect(c.connectionError).toBe('folder problem')
    t.setState('connected')
    expect(c.connectionError).toBeNull()
  })

  // ★R4 regression guard★: recording a reason while connected means the clearing transition never comes and the banner sticks; this surface is exposed as window.__ENGRAM_AGENT__ (LLM and cdp reach it).
  it('a reason reported while already connected does not stay', () => {
    const t = new MockTransport('connected')
    const c = new ProtocolClient(t)
    const seen: Array<ConnectionState> = []
    c.onConnectionStateChange((s) => seen.push(s))

    c.reportConnectionError('a reason that came in while connected')

    expect(c.connectionError).toBeNull()
    expect(seen).toEqual(['connected']) // only the registration once — no notification either
  })

  it('a reason recorded after the disconnect stays until the reconnect', () => {
    const t = new MockTransport('connected')
    const c = new ProtocolClient(t)
    t.setState('down')
    c.reportConnectionError('folder problem')
    expect(c.connectionError).toBe('folder problem')
    t.setState('connected')
    expect(c.connectionError).toBeNull()
  })

  it('no notification goes out after unsubscribing', () => {
    const t = new MockTransport('down')
    const c = new ProtocolClient(t)
    const seen: Array<ConnectionState> = []
    const off = c.onConnectionStateChange((s) => seen.push(s))
    off()
    c.reportConnectionError('reason')
    t.setState('connected')
    expect(seen).toEqual(['down'])
  })
})

// ── Connection band and the start cause (TRD S36 §3-1 · TRD S21 trd-3-2-u5 R21) ─────────────────────────────
describe('connection band and the start cause', () => {
  const plain: Band = { kind: 'not_connected', refused: false }
  const refused: Band = { kind: 'not_connected', refused: true }
  const version: Band = { kind: 'version', can_stop: true }
  const account: Band = { kind: 'account', unverified: false }
  const volume: Band = { kind: 'volume', unreadable: false }

  it('notifies listeners on a band change with the same state word and reads the band through', () => {
    const t = new MockTransport('down')
    const c = new ProtocolClient(t)
    const seen: Array<ConnectionState> = []
    c.onConnectionStateChange((s) => seen.push(s))
    expect(c.connectionBand).toBeNull()
    t.setStatus('down', plain)
    expect(seen).toEqual(['down', 'down'])
    expect(c.connectionBand).toEqual(plain)
    t.setStatus('down', version)
    expect(seen).toEqual(['down', 'down', 'down'])
    expect(c.connectionBand).toEqual(version)
  })

  it("keeps a failed start's cause through that attempt's own plain band ①", () => {
    const t = new MockTransport('down')
    const c = new ProtocolClient(t)
    c.reportConnectionError('folder problem')
    t.setStatus('down', plain)
    expect(c.connectionError).toBe('folder problem')
  })

  it.each([
    ['band ②', version],
    ['band ① refused', refused],
    ['the account band', account],
    ['the volume band', volume],
  ])('clears the cause when %s arrives', (_name, band) => {
    const t = new MockTransport('down')
    const c = new ProtocolClient(t)
    c.reportConnectionError('folder problem')
    t.setStatus('down', band)
    expect(c.connectionError).toBeNull()
  })

  it.each([
    ['band ②', version],
    ['band ① refused', refused],
    ['the account band', account],
    ['the volume band', volume],
  ])('ignores a cause set while %s holds', (_name, band) => {
    const t = new MockTransport('down')
    const c = new ProtocolClient(t)
    t.setStatus('down', band)
    const seen: Array<ConnectionState> = []
    c.onConnectionStateChange((s) => seen.push(s))
    c.reportConnectionError('a reason that came late')
    expect(c.connectionError).toBeNull()
    expect(seen).toEqual(['down']) // registration only — nothing changed
  })

  // A version-mismatch boot records the failed start's text; ② hides it; the tray stop's re-add then shows a plain ① — which must not carry that stale text.
  it.each([
    ['band ②', version],
    ['band ① refused', refused],
  ])('shows a plain ① with no stale cause after %s and a stop', (_name, band) => {
    const t = new MockTransport('down')
    const c = new ProtocolClient(t)
    c.reportConnectionError('protocol version mismatch')
    t.setStatus('down', band)
    c.reportConnectionError('protocol version mismatch') // a retried start under the same band
    t.setStatus('down', plain)
    expect(c.connectionBand).toEqual(plain)
    expect(c.connectionError).toBeNull()
  })
})

// ── failure marker → ladder → error after 3 tries ─────────────────────────────────────────────────
describe('failure marker → re-request ladder → error when the cap (3) is reached', () => {
  it('a backoff re-request on each failure marker, error state + onState notification after 3 are exhausted', async () => {
    vi.useFakeTimers()
    try {
      const t = new MockTransport()
      const c = new ProtocolClient(t)
      const states: string[] = []
      await c.subscribeOutput(V1, AGENT, () => {}, (s) => states.push(s))
      expect(t.replayCalls.length).toBe(1)
      let gen = t.replayCalls[t.replayCalls.length - 1].gen
      // Failure marker → ladder step 1 (re-request after a 1s backoff).
      t.marker(AGENT, 1, gen, { failed: true })
      await vi.advanceTimersByTimeAsync(1000)
      expect(t.replayCalls.length).toBe(2)
      gen = t.replayCalls[t.replayCalls.length - 1].gen
      // Failure marker → ladder step 2 (2s).
      t.marker(AGENT, 1, gen, { failed: true })
      await vi.advanceTimersByTimeAsync(2000)
      expect(t.replayCalls.length).toBe(3)
      gen = t.replayCalls[t.replayCalls.length - 1].gen
      // Failure marker → ladder step 3 (4s).
      t.marker(AGENT, 1, gen, { failed: true })
      await vi.advanceTimersByTimeAsync(4000)
      expect(t.replayCalls.length).toBe(4)
      gen = t.replayCalls[t.replayCalls.length - 1].gen
      // 4th failure marker → cap exhausted → error (no re-request).
      t.marker(AGENT, 1, gen, { failed: true })
      await vi.advanceTimersByTimeAsync(10000)
      expect(t.replayCalls.length).toBe(4)
      expect(states).toContain('error')
      expect(c.getViewOutputState(V1)?.phase).toBe('error')
    } finally {
      vi.useRealTimers()
    }
  })

  // ★The failure marker comes before the epoch fence★: a failure has no SubscribeAck, so the sender carries the last known epoch (0 when none) as best effort (src-tauri plan_subscribe_refusal · deadline sweep);
  //   fencing on it would drop the marker silently and the view would wait for the 10-second watchdog.
  // ★Holds only for a view that knows its marker★: that window is buffering after a re-attach as "continuing the same incarnation" (a new-incarnation attach clears the marker; markers are not picked up from frames), so set up the continuation first.
  it('a failure marker with a mismatched epoch also pushes the ladder (a success marker is still caught by the fence)', async () => {
    vi.useFakeTimers()
    try {
      const t = new MockTransport()
      const c = new ProtocolClient(t)
      await c.subscribeOutput(V1, AGENT, () => {})
      await Promise.resolve() // myGen settled
      t.output(AGENT, 9, 0)
      t.marker(AGENT, 9, t.replayCalls[0].gen) // the success marker makes it adopt marker 9
      expect(c.getViewOutputState(V1)?.phase).toBe('live')

      // Re-attach with the same marker = continuation → the view goes back to buffering holding 9.
      t.setState('reconnecting')
      t.setState('connected')
      t.control({ AgentListUpdated: { agents: [{ id: AGENT, epoch: 9 }] } })
      await Promise.resolve() // myGen settled
      const gen = t.replayCalls[1].gen
      expect(c.getViewOutputState(V1)?.phase).toBe('buffering')

      // A success marker with a mismatched epoch is still ignored (no early flush of an incomplete buffer).
      t.marker(AGENT, 0, gen)
      expect(c.getViewOutputState(V1)?.phase).toBe('buffering')

      // A failure marker passes despite the same epoch mismatch and pushes the ladder immediately.
      const before = t.replayCalls.length
      t.marker(AGENT, 0, gen, { failed: true })
      await vi.advanceTimersByTimeAsync(1000)
      expect(t.replayCalls.length).toBe(before + 1)
    } finally {
      vi.useRealTimers()
    }
  })

  // ★The refusal's diagnostic remains★: once the refusal leaves `Error`, the backend-error warn cannot catch it; without this branch `SubscribeFailed` would vanish in handleEvent's silent tail.
  //   ★The state machine is not touched★ — the same event also arrives as a replayBoundary (carrier normalization), so touching it here would push the ladder twice.
  it('a SubscribeFailed control event leaves only a warning and does not touch the view state', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      const t = new MockTransport()
      const c = new ProtocolClient(t)
      await c.subscribeOutput(V1, AGENT, () => {})
      const before = t.replayCalls.length

      t.control({ SubscribeFailed: { agent_id: AGENT, reason: `agent ${AGENT} not found` } })

      expect(warn).toHaveBeenCalled()
      expect(warn.mock.calls.some((a) => String(a[0]).includes('subscribe refused'))).toBe(true)
      expect(t.replayCalls.length).toBe(before)
      expect(c.getViewOutputState(V1)?.phase).toBe('buffering')
    } finally {
      warn.mockRestore()
    }
  })
})

// ── ADR-0226: the resume incarnation marker is carried on the 'live' notification ─────────────────────────────────
describe("resume incarnation marker — carried as info only on 'live' (ADR-0226)", () => {
  type Seen = { state: string; info: unknown; argc: number }
  function recorder(): { seen: Seen[]; cb: (...args: unknown[]) => void } {
    const seen: Seen[] = []
    return {
      seen,
      cb: (...args: unknown[]) => seen.push({ state: String(args[0]), info: args[1], argc: args.length }),
    }
  }

  it('the success marker\'s flag goes as info as is (both true and false explicit)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const r1 = recorder()
    const r2 = recorder()
    await c.subscribeOutput(V1, AGENT, () => {}, r1.cb)
    await c.subscribeOutput(V2, AGENT, () => {}, r2.cb)
    t.marker(AGENT, 1, t.replayCalls[0].gen, { continuesConversation: true })
    expect(r1.seen).toEqual([{ state: 'live', info: { continuesConversation: true, epoch: 1 }, argc: 2 }])

    t.marker(AGENT, 1, t.replayCalls[1].gen)
    expect(r2.seen).toEqual([{ state: 'live', info: { continuesConversation: false, epoch: 1 }, argc: 2 }])
  })

  // A marker that comes before myGen is kept and re-evaluated at settle time — if the kept copy drops the field, loading fails to show only on this path (common in practice: marker and invoke response are different channels).
  it('the flag is carried on the kept marker (myGen unsettled) path too', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    let releaseGen!: (gen: bigint) => void
    t.replayGenImpl = () => new Promise<bigint>((r) => (releaseGen = r))
    const r = recorder()
    await c.subscribeOutput(V1, AGENT, () => {}, r.cb)
    const gen = t.replayCalls[0].gen
    t.marker(AGENT, 1, gen, { continuesConversation: true })
    expect(r.seen).toEqual([])
    releaseGen(gen)
    await Promise.resolve()
    await Promise.resolve()
    expect(r.seen).toEqual([{ state: 'live', info: { continuesConversation: true, epoch: 1 }, argc: 2 }])
  })

  it("phases other than 'live' (buffering·detached·error) have no info", async () => {
    vi.useFakeTimers()
    try {
      const t = new MockTransport()
      const c = new ProtocolClient(t)
      const r = recorder()
      await c.subscribeOutput(V1, AGENT, () => {}, r.cb)
      t.marker(AGENT, 1, t.replayCalls[0].gen, { continuesConversation: true })
      t.control({ AgentListUpdated: { agents: [{ id: AGENT, epoch: 2 }] } }) // rotation → buffering
      t.control({ AgentListUpdated: { agents: [] } }) // absence → detached
      t.control({ AgentListUpdated: { agents: [{ id: AGENT, epoch: 2 }] } }) // re-attach → buffering
      for (let i = 0; i < 4; i++) {
        await Promise.resolve()
        t.marker(AGENT, 2, t.replayCalls[t.replayCalls.length - 1].gen, { failed: true })
        await vi.advanceTimersByTimeAsync(4000)
      }
      const nonLive = r.seen.filter((s) => s.state !== 'live')
      expect(nonLive.map((s) => s.state)).toEqual(['buffering', 'detached', 'buffering', 'error'])
      for (const s of nonLive) expect(s.argc).toBe(1)
    } finally {
      vi.useRealTimers()
    }
  })

  // ★Order pin (TRD §3-5)★: RichSlot does not take down the restore-done display in the clear callback; that is safe because 'buffering' has already taken it down before the clear and 'live' sets it up again right after, in the same call, with the new flag.
  //   Another phase notification between the clear and 'live' would show the first screen in that gap.
  it("onReset is always called after 'buffering', and only right before 'live' within the same flush", async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const order: string[] = []
    await c.subscribeOutput(
      V1,
      AGENT,
      (chunk) => order.push(`chunk:${chunk.seq}`),
      (s, info) => order.push(info ? `${s}:${info.continuesConversation}` : s),
      () => order.push('reset'),
    )
    t.output(AGENT, 1, 0)
    t.marker(AGENT, 1, t.replayCalls[0].gen)
    t.control({ AgentListUpdated: { agents: [{ id: AGENT, epoch: 2 }] } }) // a different incarnation
    await Promise.resolve() // myGen settled
    t.marker(AGENT, 2, t.replayCalls[1].gen, { continuesConversation: true })
    expect(order).toEqual(['chunk:0', 'live:false', 'buffering', 'reset', 'live:true'])
  })
})

// ── FIX-2: buffer cap exceeded → re-request after discarding the buffer (no partial flush) ─────────────────────
describe('buffer cap exceeded → discard + re-request (FIX-2)', () => {
  it('buffer discarded on cap overflow → a pre-overflow gen success marker does not flush stale frames, the re-requested replay flushes completely', async () => {
    vi.useFakeTimers()
    try {
      const t = new MockTransport()
      const c = new ProtocolClient(t)
      const got: number[] = []
      await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
      const gen1 = t.replayCalls[0].gen
      // Over the cap (4MB): 5 frames of 1MB (=5MB). On overflow pushBuffer discards the buffer + ladder re-request (1s backoff).
      const big = new Uint8Array(1024 * 1024)
      for (let seq = 0; seq < 5; seq++) t.output(AGENT, 1, seq, big)
      expect(c.getViewOutputState(V1)?.buffered).toBe(0)
      // After the ladder backoff (1s) the re-request is issued. After the requestReplay return (microtask) myGen=gen2 settles.
      await vi.advanceTimersByTimeAsync(1000)
      expect(t.replayCalls.length).toBe(2)
      const gen2 = t.replayCalls[1].gen
      // ★No stale flush★: even when the pre-overflow gen (gen1) success marker comes, the discarded 5MB frames are not flushed (the gen fence ignores it anyway, and the buffer is empty).
      t.marker(AGENT, 1, gen1)
      expect(got).toEqual([])
      t.output(AGENT, 1, 0, new Uint8Array([0]))
      t.output(AGENT, 1, 1, new Uint8Array([1]))
      t.output(AGENT, 1, 2, new Uint8Array([2]))
      t.marker(AGENT, 1, gen2)
      expect(got).toEqual([0, 1, 2])
    } finally {
      vi.useRealTimers()
    }
  })
})

// ── FIX-A: no flush even when an old-gen success marker comes before the backoff after an overflow (gen fence invalidated) ────────
describe('old-gen success marker after a buffer overflow (arriving before the backoff) → no flush + re-request kept (FIX-A)', () => {
  it('overflow discard → invalidating myGen stops the old-gen success marker from flushing the empty buffer (content loss), the re-request then flushes completely', async () => {
    vi.useFakeTimers()
    try {
      const t = new MockTransport()
      const c = new ProtocolClient(t)
      const got: number[] = []
      const states: string[] = []
      await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq), (s) => states.push(s))
      // myGen is already settled as gen1 by the subscribeOutput await (the initial issue resolves immediately).
      const gen1 = t.replayCalls[0].gen
      // Over the cap (4MB) → pushBuffer discards the buffer + schedules a ladder re-request (1s backoff). ★Before FIX-A★ myGen stayed gen1 here, and a gen1 success marker arriving before the backoff passed the gen fence,
      //   flushed an empty buffer to live (content lost) and clearTimers cancelled the re-request.
      const big = new Uint8Array(1024 * 1024)
      for (let seq = 0; seq < 5; seq++) t.output(AGENT, 1, seq, big)
      expect(c.getViewOutputState(V1)?.buffered).toBe(0)

      // ★Key: an old-gen (gen1) success marker arrives *before* the backoff (1s) fires★ — FIX-A invalidated myGen, so it only goes to held (evalMarker's myGen===undefined branch) and does not flush.
      t.marker(AGENT, 1, gen1)
      expect(got).toEqual([])
      expect(states).not.toContain('live')
      expect(c.getViewOutputState(V1)?.phase).toBe('buffering')

      // Backoff fires → re-request issued (proves the re-request was not cancelled). After the requestReplay return myGen=gen2 settles.
      await vi.advanceTimersByTimeAsync(1000)
      expect(t.replayCalls.length).toBe(2)
      const gen2 = t.replayCalls[1].gen

      t.output(AGENT, 1, 0, new Uint8Array([0]))
      t.output(AGENT, 1, 1, new Uint8Array([1]))
      t.output(AGENT, 1, 2, new Uint8Array([2]))
      t.marker(AGENT, 1, gen2)
      expect(got).toEqual([0, 1, 2])
      expect(c.getViewOutputState(V1)?.phase).toBe('live')
    } finally {
      vi.useRealTimers()
    }
  })
})

// ── FIX-6: after myGen settles, failure(G)→success(G) arriving before the backoff → flush + backoff cleaned up ──────────
describe('a same-gen success marker arriving before the backoff after a failure marker (myGen settled) → flush (backoff cleaned up)', () => {
  it('after failed(G) schedules the ladder, success(G) before the backoff expires → complete buffer flush + backoff timer cleaned up', async () => {
    vi.useFakeTimers()
    try {
      const t = new MockTransport()
      const c = new ProtocolClient(t)
      const got: number[] = []
      // myGen settles immediately (replayGenImpl null = resolves immediately). The subscribeOutput await guarantees the settle.
      await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
      const gen = t.replayCalls[0].gen
      t.output(AGENT, 1, 0)
      t.output(AGENT, 1, 1)
      // Failure marker (gen matches, myGen settled) → ladder scheduled (1s backoff). The buffer is kept (no flush).
      t.marker(AGENT, 1, gen, { failed: true })
      expect(got).toEqual([])
      // ★Before the backoff expires★ the success marker of the same gen (late Complete) → flushToLive (complete buffer flush).
      t.marker(AGENT, 1, gen, { failed: false })
      expect(got).toEqual([0, 1])
      expect(c.getViewOutputState(V1)?.phase).toBe('live')
      // ★Backoff timer cleaned up★: the flush cancelled the scheduled re-request with clearTimers, so no stray re-request as time passes (stops at the 1 initial issue).
      await vi.advanceTimersByTimeAsync(5000)
      expect(t.replayCalls.length).toBe(1)
    } finally {
      vi.useRealTimers()
    }
  })
})

// ── watchdog = a re-request, not a flush (fake timers) ───────────────────────────────────────
describe('watchdog expiry → re-request (not flush)', () => {
  it('10s pass without a success marker → re-request without flushing the buffer', async () => {
    vi.useFakeTimers()
    try {
      const t = new MockTransport()
      const c = new ProtocolClient(t)
      const got: number[] = []
      await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
      expect(t.replayCalls.length).toBe(1)
      t.output(AGENT, 1, 0)
      t.output(AGENT, 1, 1)
      // watchdog expiry (10s) → re-request. ★Not a flush★: got must still be empty.
      await vi.advanceTimersByTimeAsync(10000)
      expect(got).toEqual([])
      // watchdog → ladder re-request (after a 1s backoff).
      await vi.advanceTimersByTimeAsync(1000)
      expect(t.replayCalls.length).toBe(2)
    } finally {
      vi.useRealTimers()
    }
  })
})

// ── FIX-1: clean up the old SubState timers on a resubscribe of the same viewId ─────────────────────────────────
describe('resubscribe (same viewId) → old watchdog/backoff timers cleaned up (FIX-1)', () => {
  it('resubscribe of the same viewId while buffering (watchdog armed) → no stray re-request even when the old watchdog expires', async () => {
    vi.useFakeTimers()
    try {
      const t = new MockTransport()
      const c = new ProtocolClient(t)
      // 1) V1 first subscribe → buffering, watchdog armed (1 initial issue).
      await c.subscribeOutput(V1, AGENT, () => {})
      expect(t.replayCalls.length).toBe(1)
      // 2) Resubscribe of the same viewId (new token) → the old SubState replaced. 1 resubscribe issue (2 total).
      await c.subscribeOutput(V1, AGENT, () => {})
      expect(t.replayCalls.length).toBe(2)
      // 3) If the old watchdog (10s) is alive, on expiry it schedules ladderRerequest with the old st → stray re-request after the backoff (1s). FIX-1: resubscribe must have cleared the old timers.
      await vi.advanceTimersByTimeAsync(11000)
      // The watchdog of the surviving subscription (new st) works normally = 1 re-request (3 total). If the old watchdog also fires, 4 or more.
      expect(t.replayCalls.length).toBe(3)
    } finally {
      vi.useRealTimers()
    }
  })
})

// ── unsubscribe cleanup (timer cleanup + fan-out stop) ────────────────────────────────────────
describe('unsubscribe cleanup', () => {
  it('after unsubscribe, frames/markers do not go to that view (subs removed) + timers cleaned up', async () => {
    vi.useFakeTimers()
    try {
      const t = new MockTransport()
      const c = new ProtocolClient(t)
      const got: number[] = []
      const sub = await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
      const gen = t.replayCalls[0].gen
      t.output(AGENT, 1, 0)
      t.marker(AGENT, 1, gen)
      expect(got).toEqual([0])
      sub.unsubscribe()
      t.output(AGENT, 1, 1)
      expect(got).toEqual([0])
      expect(c.getViewOutputState(V1)).toBeNull()
      // Even if timers were pending (they were cleaned up), no extra re-request.
      const before = t.replayCalls.length
      await vi.advanceTimersByTimeAsync(20000)
      expect(t.replayCalls.length).toBe(before)
    } finally {
      vi.useRealTimers()
    }
  })

  it('a stale unsubscribe (an old unsubscribe arriving late after a resubscribe) does not delete the live subscription', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const g1: number[] = []
    const g2: number[] = []
    const sub1 = await c.subscribeOutput(V1, AGENT, (chunk) => g1.push(chunk.seq))
    await c.subscribeOutput(V1, AGENT, (chunk) => g2.push(chunk.seq))
    const gen = t.replayCalls[t.replayCalls.length - 1].gen
    sub1.unsubscribe()
    t.output(AGENT, 1, 0)
    t.marker(AGENT, 1, gen)
    expect(g2).toEqual([0])
    expect(g1).toEqual([])
  })
})

// ── epoch guard (live) ──────────────────────────────────────────────────────────────────
describe('epoch guard (live)', () => {
  it('a higher-epoch frame on a live view → drop (the [agentId,epoch] remount flow handles it)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got: Array<[number, number]> = []
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push([chunk.seq, chunk.bytes[0]]))
    const gen = t.replayCalls[0].gen
    t.output(AGENT, 1, 0)
    t.marker(AGENT, 1, gen) // epoch 1 adopted, live
    t.output(AGENT, 2, 1)
    t.output(AGENT, 1, 1)
    expect(got.map((x) => x[0])).toEqual([0, 1])
  })
})

// ── FIX: ensureReady reject → zombie subscription rollback ────────────────────────────────────────────
describe('ensureReady reject → zombie subscription rollback', () => {
  it('ensureReady reject → no zombie left in subs + rethrow', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    t.ensureReadyImpl = () => Promise.reject(new Error('daemon down'))
    const got: number[] = []
    await expect(c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))).rejects.toThrow(
      'daemon down',
    )
    t.output(AGENT, 1, 0)
    expect(got).toEqual([])
    expect(c.getViewOutputState(V1)).toBeNull()
  })
})

// ── event routing (eventBus common surface) ──────────────────────────────────────────────────
describe('event routing (eventBus common surface)', () => {
  it('StatusChanged → (id, status, epoch) received exactly', () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const calls: Array<[string, unknown, number]> = []
    const off = c.onStatusChanged((id, status, epoch) => calls.push([id, status, epoch]))
    const status = { type: 'Running' }
    t.control({ StatusChanged: { agent_id: 'agent-7', status, epoch: 3 } })
    expect(calls).toEqual([['agent-7', status, 3]])
    off()
    t.control({ StatusChanged: { agent_id: 'agent-7', status, epoch: 4 } })
    expect(calls).toEqual([['agent-7', status, 3]])
  })

  it('RestoreResult{report} → cb receives the report', () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const seen: RestoreReport[] = []
    c.onRestoreResult((r) => seen.push(r))
    const report = { agent_id: 'a9', epoch: 1, outcome: { type: 'Resumed' } } as RestoreReport
    t.control({ RestoreResult: { report } })
    expect(seen).toEqual([report])
  })

  it('ProfileListUpdated → the onProfileListUpdated cb receives profiles', () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const seen: AgentProfile[][] = []
    const off = c.onProfileListUpdated((p) => seen.push(p))
    const profiles = [{ id: 'p1' }] as unknown as AgentProfile[]
    t.control({ ProfileListUpdated: { profiles } })
    expect(seen).toEqual([profiles])
    off()
    t.control({ ProfileListUpdated: { profiles: [{ id: 'p2' }] } })
    expect(seen).toEqual([profiles])
  })

  it('PresetListUpdated → the onPresetListUpdated cb receives presets (ADR-0061)', () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const seen: Preset[][] = []
    const off = c.onPresetListUpdated((p) => seen.push(p))
    const presets = [{ id: 'pr1', cwd: 'C:/proj' }] as Preset[]
    t.control({ PresetListUpdated: { presets } })
    expect(seen).toEqual([presets])
    off()
    t.control({ PresetListUpdated: { presets: [{ id: 'pr2', cwd: 'C:/x' }] } })
    expect(seen).toEqual([presets])
  })

  it('does not piggyback on a broadcast AgentListUpdated while getAgents is in progress', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const broadcasts: AgentInfo[][] = []
    c.onAgentListUpdated((a) => broadcasts.push(a))
    const p = c.getAgents()
    await Promise.resolve()
    const rid = t.lastSent<{ request_id: string }>('ListAgents')!.request_id
    const other = [{ id: 'other' }] as unknown as AgentInfo[]
    t.control({ AgentListUpdated: { agents: other } })
    let settled = false
    void p.then(() => (settled = true)).catch(() => (settled = true))
    await Promise.resolve()
    expect(settled).toBe(false)
    expect(broadcasts).toEqual([other])
    const mine = [{ id: 'mine' }] as unknown as AgentInfo[]
    t.control({ AgentList: { request_id: rid, agents: mine } })
    await expect(p).resolves.toEqual(mine)
  })
})

// ── usage (TRD S21 usage-limit-slot §1-8) ───────────────────────────────────────────────
describe('usage — broadcast branch · ⟳ · shell cache pull', () => {
  const snap = (revision: number) =>
    ({
      vendor: 'claude',
      account_key: 'default',
      five_hour: null,
      weekly: null,
      model_scoped: [],
      plan: null,
      in_flight: false,
      state: { kind: 'Ready' },
      revision,
    }) as UsageLimitSnapshot

  it('UsageLimitsUpdated → onUsageLimitsUpdated(snapshot, socketEpoch) · nothing after off', () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const seen: Array<[UsageLimitSnapshot, number]> = []
    const off = c.onUsageLimitsUpdated((s, e) => seen.push([s, e]))
    t.control({ UsageLimitsUpdated: { snapshot: snap(3), socketEpoch: 7 } })
    expect(seen).toEqual([[snap(3), 7]])
    off()
    t.control({ UsageLimitsUpdated: { snapshot: snap(4), socketEpoch: 7 } })
    expect(seen).toHaveLength(1)
  })

  it('a daemon frame without a marker (direct carrier — only subscribed) → socketEpoch 0', () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const seen: number[] = []
    c.onUsageLimitsUpdated((_s, e) => seen.push(e))
    t.control({ UsageLimitsUpdated: { snapshot: snap(1), subscribed: ['claude'] } })
    expect(seen).toEqual([0])
  })

  it('refreshUsageLimits → exactly one RefreshUsageLimits{vendor, request_id} · resolves void on Ack', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const p = c.refreshUsageLimits('codex')
    await Promise.resolve()
    expect(t.sent).toEqual([{ RefreshUsageLimits: { vendor: 'codex', request_id: 'req-1' } }])
    t.control({ Ack: { request_id: 'req-1' } })
    await expect(p).resolves.toBeUndefined()
  })

  it('disconnect while refreshUsageLimits is waiting → reject (connection lost)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const p = c.refreshUsageLimits('claude')
    await Promise.resolve()
    t.setState('reconnecting')
    await expect(p).rejects.toThrow('connection lost')
  })

  it('getUsageSnapshot → delegated to the transport (does not take the command path or ensureReady)', async () => {
    const t = new MockTransport()
    t.usagePull = { socketEpoch: 5, snapshots: [snap(2)] }
    const c = new ProtocolClient(t)
    await expect(c.getUsageSnapshot()).resolves.toEqual({ socketEpoch: 5, snapshots: [snap(2)] })
    expect(t.ensureReadyCalls).toBe(0)
    expect(t.sent).toEqual([])
  })
})

// ── close ───────────────────────────────────────────────────────────────────────────
describe('close', () => {
  it('close() → pending reject + transport.close called + timers cleaned up', async () => {
    vi.useFakeTimers()
    try {
      const t = new MockTransport()
      const c = new ProtocolClient(t)
      await c.subscribeOutput(V1, AGENT, () => {})
      const p = c.killAgent('a1')
      await Promise.resolve()
      c.close()
      await expect(p).rejects.toThrow('client closed')
      expect(t.closed).toBe(true)
      // No timers after close (watchdog cleaned up) — re-requests do not grow.
      const before = t.replayCalls.length
      await vi.advanceTimersByTimeAsync(20000)
      expect(t.replayCalls.length).toBe(before)
    } finally {
      vi.useRealTimers()
    }
  })
})

// ── connect/disconnect (ADR-0021 §1·note3) ────────────────────────────────────────────
describe('connect/disconnect (ADR-0021 §1·note3)', () => {
  it('connect() → delegated to transport.start (explicit spawn)', async () => {
    const t = new MockTransport('down')
    const c = new ProtocolClient(t)
    await c.connect()
    expect(t.startCalls).toBe(1)
    expect(t.ensureReadyCalls).toBe(0)
  })

  it('disconnect() → delegated to transport.close', () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    c.disconnect()
    expect(t.closed).toBe(true)
  })
})

// ── ADR-0231: seq continuity — the next seq delivered to a view is always last+1 (live hold · flush starting point) ─────────
//   The daemon fans out outside the replay lock (ADR-0006), so two producers' N·N+1 can invert; passing the early N+1 and raising the cursor would drop the late N forever (a pending list event leaves that window's list wrong).
describe('seq continuity (ADR-0231) — live hold · flush starting point = max(last+1, replay head)', () => {
  /** Attaches V1, receives 0..last by replay, sets it live, then clears the received seq record and returns it. */
  async function liveAt(
    t: MockTransport,
    c: ProtocolClient,
    last: number,
    onChunk?: (chunk: OutputChunk) => void,
  ): Promise<number[]> {
    const got: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => {
      got.push(chunk.seq)
      onChunk?.(chunk)
    })
    for (let seq = 0; seq <= last; seq++) t.output(AGENT, 1, seq)
    t.marker(AGENT, 1, t.replayCalls[0].gen)
    expect(c.getViewOutputState(V1)?.phase).toBe('live')
    got.length = 0
    return got
  }

  it('[5, 7, 6] in live → in the order 5·6·7 (6 is not dropped because of the 7 that came first)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got = await liveAt(t, c, 4)
    t.output(AGENT, 1, 5)
    t.output(AGENT, 1, 7)
    expect(got).toEqual([5])
    expect(c.getViewOutputState(V1)?.held).toBe(1)
    t.output(AGENT, 1, 6)
    expect(got).toEqual([5, 6, 7])
    expect(c.getViewOutputState(V1)?.held).toBe(0)
    // Even when the 7 that was held and passed comes again, it is not drawn twice.
    t.output(AGENT, 1, 7)
    expect(got).toEqual([5, 6, 7])
  })

  it('a flow without gaps is not held but passed right away (bytes and timing of a terminal view are the same as today)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got = await liveAt(t, c, 0)
    for (let seq = 1; seq <= 5; seq++) {
      t.output(AGENT, 1, seq)
      expect(got[got.length - 1]).toBe(seq)
      expect(c.getViewOutputState(V1)?.held).toBe(0)
    }
    expect(got).toEqual([1, 2, 3, 4, 5])
  })

  it('after [5, 7], 7 is not passed even as time passes (no time limit) — no re-request goes out either', async () => {
    vi.useFakeTimers()
    try {
      const t = new MockTransport()
      const c = new ProtocolClient(t)
      const got = await liveAt(t, c, 4)
      t.output(AGENT, 1, 5)
      t.output(AGENT, 1, 7)
      await vi.advanceTimersByTimeAsync(10 * 60_000)
      expect(got).toEqual([5])
      expect(c.getViewOutputState(V1)).toMatchObject({ phase: 'live', held: 1 })
      expect(t.replayCalls.length).toBe(1)
      t.output(AGENT, 1, 6)
      expect(got).toEqual([5, 6, 7])
    } finally {
      vi.useRealTimers()
    }
  })

  it('a tag1 placeholder Error fills the gap of a terminal view and the frames behind it flow', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const chunks: OutputChunk[] = []
    await liveAt(t, c, 4, (chunk) => chunks.push(chunk))
    chunks.length = 0
    t.output(AGENT, 1, 6, new Uint8Array([0x41]), 0)
    expect(chunks).toEqual([])
    const placeholder = new TextEncoder().encode(JSON.stringify({ type: 'Error', message: 'placeholder' }))
    t.output(AGENT, 1, 5, placeholder, 1)
    expect(chunks.map((x) => [x.seq, x.tag])).toEqual([
      [5, 1],
      [6, 0],
    ])
  })

  // The daemon puts `ReplayComplete` into the connection queue outside the subscribers lock — connection queue `[…L, L+2, marker, L+1]`.
  it('buffer […L, L+2] + success marker → delivered only up to L · the following L+1 → L+1·L+2 in order', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    t.output(AGENT, 1, 2) // L
    t.output(AGENT, 1, 4) // L+2 — a live frame that came before L+1
    t.marker(AGENT, 1, t.replayCalls[0].gen)
    expect(got).toEqual([0, 1, 2])
    expect(c.getViewOutputState(V1)).toMatchObject({ phase: 'live', held: 1 })
    t.output(AGENT, 1, 3) // L+1
    expect(got).toEqual([0, 1, 2, 3, 4])
    expect(c.getViewOutputState(V1)?.held).toBe(0)
  })

  it('new incarnation (last −1) + marker replay head 40 → from 40 (head jump of a replay with a truncated ring)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const order: string[] = []
    await c.subscribeOutput(
      V1,
      AGENT,
      (chunk) => order.push(`chunk:${chunk.seq}`),
      undefined,
      () => order.push('reset'),
    )
    for (let seq = 0; seq <= 50; seq++) t.output(AGENT, 1, seq)
    t.marker(AGENT, 1, t.replayCalls[0].gen)
    order.length = 0
    // The roster states a new marker = a different incarnation. The new incarnation's ring has already pushed out what is before 40.
    t.control({ AgentListUpdated: { agents: [{ id: AGENT, epoch: 2 }] } })
    const gen2 = t.replayCalls[1].gen
    await Promise.resolve() // myGen=gen2 settled
    for (let seq = 40; seq <= 42; seq++) t.output(AGENT, 2, seq)
    t.marker(AGENT, 2, gen2, { replayFrom: 40, truncated: true })
    expect(order).toEqual(['reset', 'chunk:40', 'chunk:41', 'chunk:42'])
    t.output(AGENT, 2, 43)
    expect(order[order.length - 1]).toBe('chunk:43')
    expect(c.getViewOutputState(V1)?.held).toBe(0)
  })

  it('same incarnation (last 50) + buffer 40–60 → from 51 (when the head is before the cursor, the cursor wins)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got = await liveAt(t, c, 50)
    t.setState('reconnecting')
    t.setState('connected')
    t.control({ AgentListUpdated: { agents: [{ id: AGENT, epoch: 1 }] } })
    const gen2 = t.replayCalls[1].gen
    await Promise.resolve() // myGen=gen2 settled
    for (let seq = 40; seq <= 60; seq++) t.output(AGENT, 1, seq)
    t.marker(AGENT, 1, gen2, { replayFrom: 40 })
    expect(got).toEqual(Array.from({ length: 10 }, (_, i) => 51 + i))
    expect(c.getViewOutputState(V1)?.held).toBe(0)
  })

  // ★Estimating the head as the buffer's minimum seq goes wrong here★: the buffer has only 1, so it would pass from 1 and drop the late 0.
  it('empty replay: new incarnation + marker head 0 · seq 1 before the marker · seq 0 after the marker → both 0·1 in order', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
    t.output(AGENT, 1, 1)
    t.marker(AGENT, 1, t.replayCalls[0].gen, { replayFrom: 0 })
    expect(got).toEqual([])
    expect(c.getViewOutputState(V1)).toMatchObject({ phase: 'live', held: 1 })
    t.output(AGENT, 1, 0)
    expect(got).toEqual([0, 1])
  })

  it('the kept marker (myGen unsettled) path carries the replay head too', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    let resolveGen: (() => void) | null = null
    t.replayGenImpl = (_a, gen) =>
      new Promise<bigint>((r) => {
        resolveGen = () => r(gen)
      })
    const got: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
    const gen = t.replayCalls[0].gen
    t.output(AGENT, 1, 40)
    t.output(AGENT, 1, 41)
    t.marker(AGENT, 1, gen, { replayFrom: 40 })
    expect(got).toEqual([]) // myGen unsettled — kept
    resolveGen!()
    await Promise.resolve()
    await Promise.resolve()
    expect(got).toEqual([40, 41])
  })

  it('when the held amount exceeds the cap, startBuffering → the re-claim actually goes out right away and the hold list is emptied', async () => {
    vi.useFakeTimers()
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      const t = new MockTransport()
      const c = new ProtocolClient(t)
      const states: string[] = []
      const got: number[] = []
      await c.subscribeOutput(
        V1,
        AGENT,
        (chunk) => got.push(chunk.seq),
        (s) => states.push(s),
      )
      t.output(AGENT, 1, 0)
      t.marker(AGENT, 1, t.replayCalls[0].gen)
      got.length = 0
      // 2048 frames are held while seq 1 has not come — still within the cap (smaller than the ring's 4096 entries).
      for (let seq = 2; seq <= 2049; seq++) t.output(AGENT, 1, seq)
      expect(c.getViewOutputState(V1)).toMatchObject({ phase: 'live', held: 2048 })
      expect(t.replayCalls.length).toBe(1)
      t.output(AGENT, 1, 2050) // the 2049th — overflows
      // ★The re-claim has already gone out without running the timers★ — not the ladder (buffering check after the backoff).
      expect(t.replayCalls.length).toBe(2)
      expect(c.getViewOutputState(V1)).toMatchObject({ phase: 'buffering', held: 0, buffered: 0 })
      expect(states).toEqual(['live', 'buffering'])
      expect(got).toEqual([])
      // Same incarnation, so the cursor (0) is kept — the replay that comes again flows from 1 without gaps (only a short ring tail here).
      const gen2 = t.replayCalls[1].gen
      await Promise.resolve() // myGen=gen2 settled
      for (let seq = 0; seq <= 10; seq++) t.output(AGENT, 1, seq)
      t.marker(AGENT, 1, gen2)
      expect(got).toEqual(Array.from({ length: 10 }, (_, i) => 1 + i))
      expect(c.getViewOutputState(V1)).toMatchObject({ phase: 'live', held: 0 })
      await vi.advanceTimersByTimeAsync(10_000)
      expect(t.replayCalls.length).toBe(2) // no late ladder or watchdog re-claim
    } finally {
      warn.mockRestore()
      vi.useRealTimers()
    }
  })

  it('the same disposal when the held bytes exceed the cap (1 MiB — smaller than the ring\'s 2 MiB)', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      const t = new MockTransport()
      const c = new ProtocolClient(t)
      await liveAt(t, c, 0)
      const quarter = new Uint8Array(256 * 1024)
      for (let seq = 2; seq <= 5; seq++) t.output(AGENT, 1, seq, quarter) // exactly 1 MiB — still within the cap
      expect(c.getViewOutputState(V1)).toMatchObject({ phase: 'live', held: 4 })
      t.output(AGENT, 1, 6, new Uint8Array(1))
      expect(t.replayCalls.length).toBe(2)
      expect(c.getViewOutputState(V1)).toMatchObject({ phase: 'buffering', held: 0 })
    } finally {
      warn.mockRestore()
    }
  })

  it('on disconnect the held frames are dropped too — the replay attached again fills the gap and each seq is drawn only once', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got = await liveAt(t, c, 4)
    t.output(AGENT, 1, 7)
    expect(c.getViewOutputState(V1)?.held).toBe(1)
    t.setState('reconnecting')
    expect(c.getViewOutputState(V1)).toMatchObject({ phase: 'detached', held: 0 })
    t.setState('connected')
    t.control({ AgentListUpdated: { agents: [{ id: AGENT, epoch: 1 }] } })
    const gen2 = t.replayCalls[1].gen
    await Promise.resolve() // myGen=gen2 settled
    for (let seq = 0; seq <= 8; seq++) t.output(AGENT, 1, seq)
    t.marker(AGENT, 1, gen2)
    expect(got).toEqual([5, 6, 7, 8])
  })

  // Up to the accumulator — a wrong list is the symptom of this defect (losing `Delivered{X}` leaves X and no bubble either).
  it('up to the accumulator: even when Queued{X} (seq N) comes after a conversation frame (N+1), X stands in the list and the following Delivered{X} draws the bubble', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const acc = new StructuredEventAccumulator()
    await liveAt(t, c, 4, (chunk) => {
      if (chunk.tag === 1) acc.feed(chunk.bytes)
    })
    const ev = (e: unknown) => new TextEncoder().encode(JSON.stringify(e))
    t.output(AGENT, 1, 6, ev({ type: 'TextDelta', text: 'answer' }), 1) // N+1 comes first
    expect(acc.snapshotQueued()).toEqual([])
    t.output(AGENT, 1, 5, ev({ type: 'QueuedInput', op: { kind: 'Queued', id: 'X', text: 'hello' } }), 1) // N
    expect(acc.snapshotQueued().map((e) => e.id)).toEqual(['X'])
    t.output(AGENT, 1, 7, ev({ type: 'QueuedInput', op: { kind: 'Delivered', id: 'X' } }), 1)
    expect(acc.snapshotQueued()).toEqual([])
    const userBubbles = acc
      .snapshot()
      .flatMap((it) => (it.kind === 'structured' && it.label === 'user' ? [JSON.parse(it.json)] : []))
    expect(userBubbles).toEqual([{ type: 'text', text: 'hello', uuid: 'X' }])
  })
})
