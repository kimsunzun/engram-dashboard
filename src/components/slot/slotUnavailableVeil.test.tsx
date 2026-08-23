// 부재 막이 세 슬롯에서 **같은 조건에 같은 모습**으로 뜬다(ADR-0148 결정 2·3 · 사용자 결정 2026-08-20).
//
// 배경: 터미널·DOM 슬롯은 예전에 어두운 막 위에 글을 적었다 — '에이전트 대기 중' · '종료됨' ·
//   `Failed: <메시지>`. 리치 슬롯만 심볼 하나였다. 같은 사실을 슬롯마다 다른 문장으로 말했고, 데몬
//   재기동처럼 판정이 흔들리는 구간에서 '종료됨' 같은 단정이 그대로 남았다. 셋을 심볼 하나로 합쳤고,
//   이 파일이 그 합의의 회귀망이다 — 조건 셋(종료·연결 끊김·구독 부착 대기)과 "글자 없음" 둘 다 잰다.
//
// 전략: slotReplayRestart.test.tsx 와 같은 stub 패턴. 연결 상태는 실물과 동형으로 구독자를 들고 있다가
//   전이 때 통지한다(등록 즉시 1회 통지 포함).

import type { ReactElement } from 'react'
import { act, cleanup, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { OutputChunk, ViewPhase } from '../../api/agentClient'

// jsdom 미제공 관측자 2종 — TerminalSlot 이 마운트 시 둘 다 만들고, Radix ScrollArea 도 RO 를 참조한다.
globalThis.ResizeObserver ||= class {
  observe() {}
  unobserve() {}
  disconnect() {}
} as unknown as typeof ResizeObserver
globalThis.IntersectionObserver ||= class {
  observe() {}
  unobserve() {}
  disconnect() {}
  takeRecords() {
    return []
  }
} as unknown as typeof IntersectionObserver

const captured = vi.hoisted(() => ({
  onState: null as ((s: ViewPhase) => void) | null,
  onReset: null as (() => void) | null,
}))
const conn = vi.hoisted(() => ({
  state: 'connected' as 'connected' | 'reconnecting' | 'down',
  cbs: new Set<(s: string) => void>(),
}))

vi.mock('../../api/clientFactory', () => ({
  agentClient: {
    subscribeOutput: vi.fn(
      async (
        _viewId: string,
        _agentId: string,
        _onChunk: (c: OutputChunk) => void,
        onState?: (s: ViewPhase) => void,
        onReset?: () => void,
      ) => {
        captured.onState = onState ?? null
        captured.onReset = onReset ?? null
        return { unsubscribe: vi.fn() }
      },
    ),
    writeStdin: vi.fn(async () => undefined),
    resizePty: vi.fn(async () => undefined),
    get connectionState() {
      return conn.state
    },
    onConnectionStateChange: (cb: (s: string) => void) => {
      conn.cbs.add(cb)
      cb(conn.state) // 실물과 동일: 등록 즉시 현재 상태 1회 통지.
      return () => conn.cbs.delete(cb)
    },
  },
  getAgentClient: vi.fn(),
}))

const agentStoreState = vi.hoisted(() => ({
  agents: [] as unknown[],
  agentsLoaded: false,
  profiles: [] as unknown[],
}))
vi.mock('../../store/agentStore', () => ({
  useAgentStore: (selector: (s: typeof agentStoreState) => unknown) => selector(agentStoreState),
}))

vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    loadAddon = vi.fn()
    open = vi.fn()
    reset = vi.fn()
    write = vi.fn()
    onData = vi.fn(() => ({ dispose: vi.fn() }))
    dispose = vi.fn()
    refresh = vi.fn()
    cols = 80
    rows = 24
  },
}))
vi.mock('@xterm/addon-fit', () => ({ FitAddon: class { fit = vi.fn() } }))
vi.mock('@xterm/addon-webgl', () => ({ WebglAddon: class { onContextLoss = vi.fn(); dispose = vi.fn() } }))
vi.mock('@xterm/xterm/css/xterm.css', () => ({}))

import TerminalSlot from './TerminalSlot'
import DomSlot from './DomSlot'
import RichSlot from './RichSlot'

const AGENT = 'aaaa-bbbb-cccc-dddd'

/** 세 슬롯이 공유하는 막의 마커 — 이 한 선택자가 "같은 모습" 의 기계적 정의다. */
function veil(): HTMLElement | null {
  return document.querySelector('[data-slot-dead="1"]')
}

async function flush(): Promise<void> {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
}

function setConnection(state: 'connected' | 'reconnecting' | 'down'): void {
  act(() => {
    conn.state = state
    for (const cb of conn.cbs) cb(state)
  })
}

// 세 슬롯의 props 는 서로 다르다(리치는 viewId 가 옵션, 터미널은 agentId 가 nullable) — 렌더만 감싼다.
const SLOTS: Array<[string, () => ReactElement]> = [
  ['TerminalSlot', () => <TerminalSlot viewId="v1" agentId={AGENT} />],
  ['DomSlot', () => <DomSlot viewId="v1" agentId={AGENT} />],
  ['RichSlot', () => <RichSlot viewId="v1" agentId={AGENT} />],
]

beforeEach(() => {
  captured.onState = null
  captured.onReset = null
  conn.state = 'connected'
  conn.cbs.clear()
  agentStoreState.agents = []
  agentStoreState.agentsLoaded = false
  agentStoreState.profiles = []
})

afterEach(() => {
  cleanup()
})

describe.each(SLOTS)('%s — 부재 막(세 슬롯 공통)', (_name, mount) => {
  // ★막과 같은 자리에서 재는 두 번째 공통 의무★: 비우기 콜백을 안 넘긴 슬롯은 커서만 0 으로 돌아가고
  //   화면은 앞 화신 내용을 든 채라, 전량 replay 가 그 위에 겹쳐 그려지고 오류는 어디에도 남지 않는다.
  //   슬롯이 늘어날 때 여기 SLOTS 목록에 한 줄 더하는 것만으로 그 의무가 함께 걸린다.
  it('구독에 비우기 콜백(onReset)을 넘긴다', async () => {
    agentStoreState.agents = [{ id: AGENT, cwd: 'C:/x', status: { type: 'Running' }, epoch: 0 }]
    agentStoreState.agentsLoaded = true
    render(mount())
    await flush()

    expect(captured.onReset).toBeTypeOf('function')
  })

  it('아무 조건도 아니면 막이 없다', async () => {
    agentStoreState.agents = [{ id: AGENT, cwd: 'C:/x', status: { type: 'Running' }, epoch: 0 }]
    agentStoreState.agentsLoaded = true
    render(mount())
    await flush()

    expect(veil()).toBeNull()
  })

  it('명부를 아직 모르는 구간은 부재가 아니다(대기 표시를 그리지 않는다)', async () => {
    agentStoreState.agentsLoaded = false
    render(mount())
    await flush()

    expect(veil()).toBeNull()
  })

  it('연결이 끊기면 막이 뜬다', async () => {
    agentStoreState.agents = [{ id: AGENT, cwd: 'C:/x', status: { type: 'Running' }, epoch: 0 }]
    agentStoreState.agentsLoaded = true
    render(mount())
    await flush()
    expect(veil()).toBeNull()

    setConnection('reconnecting')
    expect(veil()).not.toBeNull()
  })

  it('명부 수신 후에도 해석 안 되면 막이 뜬다(수거된 에이전트)', async () => {
    agentStoreState.agents = []
    agentStoreState.agentsLoaded = true
    render(mount())
    await flush()

    expect(veil()).not.toBeNull()
  })

  it("구독이 'detached' 라고 하면 막이 뜨고, 부착되면 내려간다", async () => {
    agentStoreState.agents = [{ id: AGENT, cwd: 'C:/x', status: { type: 'Running' }, epoch: 0 }]
    agentStoreState.agentsLoaded = true
    render(mount())
    await flush()
    expect(captured.onState).toBeTruthy()
    expect(veil()).toBeNull()

    act(() => captured.onState!('detached'))
    expect(veil()).not.toBeNull()

    act(() => captured.onState!('buffering'))
    expect(veil()).toBeNull()
  })

  // ★글을 적지 않는다(사용자 결정)★: 실패 사유든 종료든 대기든, 막이 말하는 건 심볼 하나뿐이다.
  it('막은 심볼 하나뿐 — 글자를 담지 않는다(실패 사유 포함)', async () => {
    agentStoreState.agents = [
      { id: AGENT, cwd: 'C:/x', status: { type: 'Failed', message: 'boom' }, epoch: 0 },
    ]
    agentStoreState.agentsLoaded = true
    render(mount())
    await flush()

    const v = veil()
    expect(v).not.toBeNull()
    expect(v!.textContent).toBe('')
    expect(v!.querySelector('svg')).not.toBeNull()
  })
})
