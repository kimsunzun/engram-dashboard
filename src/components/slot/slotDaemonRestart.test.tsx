// 데몬 재기동이 슬롯 화면을 어떻게 다루나 — **실 ProtocolClient 를 통해** 실 슬롯까지 태우는 회귀.
//
// ★손으로 onReset 을 부르지 않는 이유(이 파일의 존재 이유)★: 비우기 신호 자체를 손으로 발화하는 테스트는
//   "슬롯이 신호에 반응하나" 만 재고, **그 신호가 실제로 오나** 는 못 잰다. 실제로 그 결함이 그렇게 숨었다 —
//   슬롯은 신호에 잘 반응했지만, 상위가 회차(epoch)를 prop 으로 내려보내던 탓에 **신호가 오기도 전에**
//   구독이 통째로 갈리며 화면이 지워졌고(그 교체가 화신 표식까지 잃어 회전 판정도 못 섰다), 그래서 신호는
//   영영 오지 않았다. 그 구간을 재려면 상위 렌더러 → 구독 → 전송까지 한 줄로 이어야 한다.
//
// 못 박는 두 문장:
//   ① 데몬을 재기동해도 **대체 replay 가 도착하기 전에는** 슬롯을 지우지 않는다.
//   ② 그 replay 가 실패로 끝나면 앞 화신의 화면이 **그대로 남는다**(거기에 부재 표시만 얹힌다).
//
// 관측 슬롯 = DomSlot — 같은 출력 스트림을 평문 <pre> 로 그려 textContent 로 읽힌다(터미널은 canvas).

import { act, cleanup, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { ConnectionState } from '../../api/agentClient'
import type { InboundMessage, Transport } from '../../api/transport'
import { ProtocolClient } from '../../api/protocolClient'
import type { AgentInfo, Capabilities } from '../../api/types'
import type { LayoutNode } from '../../api/layoutTypes'

// ── Tauri stub — viewStore(레이아웃 권위 미러)가 부팅 시 invoke/listen 을 건다. ──
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => undefined),
  Channel: class {
    onmessage: unknown = null
  },
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => vi.fn()) }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ label: 'main' }),
}))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(async () => null) }))

// ── agentClient = **실 ProtocolClient**(테스트마다 새로 만든다) ──────────────────────
// Proxy 로 감싸 매 접근을 그때의 인스턴스로 넘긴다 — mock factory 는 호이스팅돼 인스턴스보다 먼저 돈다.
const holder = vi.hoisted(() => ({ client: null as unknown as Record<string, unknown> }))
vi.mock('../../api/clientFactory', () => ({
  agentClient: new Proxy(
    {},
    {
      get(_t, prop: string) {
        const c = holder.client as unknown as Record<string, unknown>
        const v = c[prop]
        return typeof v === 'function' ? (v as (...a: unknown[]) => unknown).bind(c) : v
      },
    },
  ),
  getAgentClient: vi.fn(),
}))

// ── agentStore stub — 상위 렌더러의 세 갈래 게이트(ADR-0148/0149)가 여기를 본다. ──
const agentStoreState = vi.hoisted(() => ({
  agents: [] as unknown[],
  agentsLoaded: false,
  profiles: [] as unknown[],
  profilesLoaded: false,
  presets: [] as unknown[],
}))
vi.mock('../../store/agentStore', () => ({
  useAgentStore: Object.assign(
    (selector: (s: typeof agentStoreState) => unknown) => selector(agentStoreState),
    { getState: () => agentStoreState },
  ),
}))

// 슬롯 분기와 무관한 무거운 형제들(트리·팔레트·split)은 세우지 않는다 — 이 테스트의 노드는 agent 슬롯 하나다.
vi.mock('../agent/AgentList', () => ({ default: () => <div /> }))
vi.mock('./PresetPalette', () => ({ default: () => <div /> }))
vi.mock('allotment', () => ({
  Allotment: Object.assign(() => <div />, { Pane: () => <div /> }),
}))

import ViewLayoutRenderer from '../layout/ViewLayoutRenderer'
import { useViewStore } from '../../store/viewStore'

// ── 최소 Transport — 프레임·마커·연결 상태를 테스트가 직접 민다. ─────────────────────
class FakeTransport implements Transport {
  private _state: ConnectionState = 'connected'
  private stateCbs = new Set<(s: ConnectionState) => void>()
  private msgCb: ((m: InboundMessage) => void) | null = null
  replayCalls: bigint[] = []
  private genCounter = 0n

  get connectionState(): ConnectionState {
    return this._state
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
  send(): void {}
  ensureReady(): Promise<void> {
    return Promise.resolve()
  }
  start(): Promise<void> {
    return Promise.resolve()
  }
  close(): void {}
  requestReplay(): Promise<bigint> {
    const gen = ++this.genCounter
    this.replayCalls.push(gen)
    return Promise.resolve(gen)
  }

  // ── 구동 ──
  setState(s: ConnectionState): void {
    this._state = s
    act(() => {
      for (const cb of this.stateCbs) cb(s)
    })
  }
  roster(agents: Array<{ id: string; epoch?: number }>): void {
    act(() => this.msgCb?.({ kind: 'control', event: { AgentListUpdated: { agents } } }))
  }
  output(epoch: number, seq: number, text: string): void {
    act(() =>
      this.msgCb?.({
        kind: 'output',
        tag: 0,
        agentId: AGENT,
        epoch,
        seq,
        bytes: new TextEncoder().encode(text),
      }),
    )
  }
  marker(epoch: number, gen: bigint, failed = false): void {
    act(() =>
      this.msgCb?.({ kind: 'replayBoundary', agentId: AGENT, epoch, gen, truncated: false, failed }),
    )
  }
  get lastGen(): bigint {
    return this.replayCalls[this.replayCalls.length - 1]
  }
}

const AGENT = 'aaaa-bbbb-cccc-dddd'
const SLOT = 's1'

function caps(): Capabilities {
  return {
    input: { raw: true, message: false, attachment: false },
    output: { terminal_bytes: true, structured: false, markdown: false, tool_events: false, usage: false },
    control: { resize: true, interrupt: true, cancel: false, graceful_shutdown: true },
    session: { resume: true, snapshot: false, cwd_env: true },
    model: { select: false, temperature: false, max_tokens: false },
  }
}

function agentInfo(epoch: number): AgentInfo {
  return {
    id: AGENT,
    name: AGENT,
    cwd: '/tmp',
    status: { type: 'Running' },
    cols: 80,
    rows: 24,
    epoch,
    capabilities: caps(),
  }
}

/**
 * ★프로필의 회차는 살아있는 값이 아니다★: 백엔드는 이 칸을 더는 갱신하지 않는다. 한때 상위 렌더러가
 * 여기서 회차를 읽어 슬롯 prop 으로 내려보냈고, 그래서 데몬이 재기동하면 값이 뚝 떨어지며 재구독·화면
 * 소거가 돌았다. 실제 값과 어긋나게 시딩해 그 경로가 되살아나면 곧바로 드러나게 둔다.
 */
function profile(): unknown {
  return { id: AGENT, name: AGENT, cwd: '/tmp', display_name: null, parent_id: null, created_at: 0, epoch: 0 }
}

const node: LayoutNode = { type: 'slot', id: SLOT, content: { type: 'agent', agent_id: AGENT } }

function domText(): string {
  return (document.querySelector('[data-dom-mode="1"]') as HTMLElement | null)?.textContent ?? ''
}

function veil(): HTMLElement | null {
  return document.querySelector('[data-slot-dead="1"]')
}

async function flush(): Promise<void> {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
}

let t: FakeTransport
let client: ProtocolClient

beforeEach(() => {
  t = new FakeTransport()
  client = new ProtocolClient(t)
  holder.client = client as unknown as Record<string, unknown>
  agentStoreState.agents = [agentInfo(7)]
  agentStoreState.agentsLoaded = true
  agentStoreState.profiles = [profile()]
  agentStoreState.profilesLoaded = true
  // 관측 가능한 렌더러(DomSlot)로 고정 — 같은 스트림을 평문으로 그린다(§5 관측 수단).
  useViewStore.setState({ renderModeOverride: { [SLOT]: 'dom' } })
})

afterEach(() => {
  cleanup()
  client.close()
  useViewStore.setState({ renderModeOverride: {} })
  vi.restoreAllMocks()
})

/** 첫 화신(표식 7)의 이력을 그려 둔 상태까지 진행한다. */
async function mountWithHistory() {
  const view = render(<ViewLayoutRenderer node={node} focusedSlotId={null} />)
  await flush()
  t.output(7, 0, 'run-one output')
  t.marker(7, t.lastGen)
  expect(domText()).toBe('run-one output')
  return view
}

/** 데몬이 내려가고 그 에이전트가 명부에서 사라지는 구간(재기동 중). */
function daemonGoesDown(rerender: (ui: React.ReactElement) => void): void {
  t.setState('reconnecting')
  agentStoreState.agents = [] // reaper 가 수거 = 명부에서 사라진다
  act(() => rerender(<ViewLayoutRenderer node={node} focusedSlotId={null} />))
}

describe('데몬 재기동 — 대체 replay 가 오기 전에는 슬롯을 지우지 않는다', () => {
  it('끊김 + 명부에서 사라짐만으로는 화면이 남고, 부재 표시만 얹힌다', async () => {
    const { rerender } = await mountWithHistory()

    daemonGoesDown(rerender)

    // ★headline★ 아직 아무것도 안 왔다 — 지울 근거가 없다.
    expect(domText()).toBe('run-one output')
    expect(veil()).not.toBeNull()
  })

  it('부착한 replay 가 실패로 끝나도 앞 화신 화면이 그대로 남는다', async () => {
    vi.useFakeTimers()
    try {
      const { rerender } = await mountWithHistory()
      daemonGoesDown(rerender)

      // 소켓이 다시 서고, 명부는 **다른 표식**으로 그 에이전트를 말한다(재spawn).
      t.setState('connected')
      t.roster([{ id: AGENT, epoch: 8 }])
      await act(async () => {
        await Promise.resolve()
      })

      // 사다리를 끝까지 소진시킨다 — 어느 단계의 실패도 화면을 비우지 않는다.
      for (let i = 0; i < 4; i++) {
        t.marker(8, t.lastGen, /*failed*/ true)
        await act(async () => {
          await vi.advanceTimersByTimeAsync(5000)
        })
      }

      expect(client.getViewOutputState(SLOT)?.phase).toBe('error')
      expect(domText()).toBe('run-one output')
      // ★소진한 사다리도 표면에 남는다★: 이걸 빼면 신호도 회복 경로도 없는 빈 판이 된다.
      expect(veil()).not.toBeNull()
    } finally {
      vi.useRealTimers()
    }
  })

  it('새 화신의 replay 가 도착하면 그때 한 번에 갈아 끼운다', async () => {
    const { rerender } = await mountWithHistory()
    daemonGoesDown(rerender)

    t.setState('connected')
    t.roster([{ id: AGENT, epoch: 8 }])
    await act(async () => {
      await Promise.resolve()
    })
    // 부착만으로는 아직 그대로다(요청 시점 비우기 금지).
    expect(domText()).toBe('run-one output')

    // 새 화신이 실제로 왔다 — 명부·store 도 그렇게 갱신된다.
    agentStoreState.agents = [agentInfo(8)]
    t.output(8, 0, 'run-two output') // 번호가 0 부터 다시 온다
    t.marker(8, t.lastGen)
    act(() => rerender(<ViewLayoutRenderer node={node} focusedSlotId={null} />))

    // 겹쳐 쌓이지도(앞 화신 잔존), 통째로 탈락하지도(빈 화면) 않는다 — 한 벌로만 남는다.
    expect(domText()).toBe('run-two output')
    expect(veil()).toBeNull()
    expect(client.getViewOutputState(SLOT)?.phase).toBe('live')
  })

  // ★소켓이 한 번도 안 끊긴 재spawn★: 슬롯 입장에선 아무 일도 없는 것처럼 보이는 구간이라, 회전을
  //   알아보는 자리가 명부 관측뿐이다. 이 갈래가 없으면 열려 있던 슬롯은 재spawn 이후 한 바이트도 못 그린다.
  it('끊긴 적 없어도 명부의 새 표식이 열려 있는 슬롯에 닿는다', async () => {
    await mountWithHistory()

    t.roster([{ id: AGENT, epoch: 9 }])
    await act(async () => {
      await Promise.resolve()
    })
    t.output(9, 0, 'respawned output')
    t.marker(9, t.lastGen)

    expect(domText()).toBe('respawned output')
  })
})
