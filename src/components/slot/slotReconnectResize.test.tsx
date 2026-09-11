// 재연결 뒤 PTY 치수 재전파 회귀 — 끊긴 동안 떨어진 resize 를 소켓이 서면 한 번 다시 낸다.
//
// 배경(ADR-0195): 소켓이 없는 동안 들어온 명령은 담아 두지 않고 그 자리에서 실패한다 — 명령이 연결
//   경계를 넘지 않게 하려는 결정이다. resize 는 fire-and-forget 이라 그 실패가 조용히 삼켜지고 다시
//   낼 사람이 없어 PTY 가 끊기기 전 치수에 고착된다(재연결 창 = 500ms~10초. 그 사이 분할선을 끌면
//   그 치수는 에이전트에 영영 닿지 않는다).
//
// ★이 회귀가 막는 반대편 둘★
//   ① 같은 증상을 구독 트리거(viewId·agentId deps)에 연결 상태를 섞어 고치면 재연결마다 구독이 갈려
//      replay 도착 전에 화면이 지워지고 화신 표식까지 잃는다(ADR-0164) — 재전파가 **재구독 없이**
//      일어났음을 함께 못 박는다.
//   ② 전이가 아니라 **값**을 보고 쏘는 구현(연결 상태가 참인 동안 effect 가 도는 족족 발사)은 마운트와
//      agentId 교체에서 구독 경로와 겹쳐 쏜다 — 아래 '마운트' · 'agentId 교체' · StrictMode 케이스가
//      그 구현에서 실패하도록 세워져 있다.

import { StrictMode } from 'react'
import { act, cleanup, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { OutputChunk, ViewPhase } from '../../api/agentClient'

// jsdom 미제공 관측자 2종 — 콜백을 한 번도 발화하지 않는 no-op 이라 연결 경로만 남는다(크기 변화·
//   가시성 전이가 내는 전파와 섞이지 않게 한다).
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

// ── 레이아웃 대역 ───────────────────────────────────────────────────────────────────
// jsdom 은 레이아웃이 없어 offsetParent 를 **항상** null 로, getBoundingClientRect 를 **항상** 0 으로
//   낸다. 그 둘이 정확히 슬롯의 숨김 판정 두 갈래(ADR-0056)라, 손대지 않으면 보이는 슬롯의 갈래를 이
//   하네스에서 아예 밟을 수 없다. ★프로토타입에 심는 이유★: render() 이후에 붙이면 **마운트 커밋 시점**
//   에는 여전히 숨김이라, 마운트에서 겹쳐 쏘는 구현조차 가드에 걸려 조용히 통과한다(그 함정이 이 파일의
//   앞 판이었다).
const layout = vi.hoisted(() => ({ visible: true, width: 800, height: 400 }))
Object.defineProperty(HTMLElement.prototype, 'offsetParent', {
  configurable: true,
  get(): Element | null {
    return layout.visible ? document.body : null
  },
})
HTMLElement.prototype.getBoundingClientRect = function (): DOMRect {
  return {
    width: layout.width,
    height: layout.height,
    top: 0,
    left: 0,
    right: layout.width,
    bottom: layout.height,
    x: 0,
    y: 0,
    toJSON: () => ({}),
  } as DOMRect
}

const captured = vi.hoisted(() => ({
  onChunk: null as ((c: OutputChunk) => void) | null,
  onState: null as ((s: ViewPhase) => void) | null,
}))

// 연결 상태 표면 — 실물과 동형으로 구독자를 들고 있다가 전이 때 통지한다(등록 즉시 1회 통지 포함).
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
        onChunk: (c: OutputChunk) => void,
        onState?: (s: ViewPhase) => void,
      ) => {
        captured.onChunk = onChunk
        captured.onState = onState ?? null
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
      cb(conn.state)
      return () => conn.cbs.delete(cb)
    },
  },
  getAgentClient: vi.fn(),
}))

const agentStoreState = vi.hoisted(() => ({ agents: [] as unknown[], agentsLoaded: false }))
vi.mock('../../store/agentStore', () => ({
  useAgentStore: (selector: (s: typeof agentStoreState) => unknown) => selector(agentStoreState),
}))

// 치수는 holder 로 읽는다 — fit() 이 실제로 측정을 반영한 뒤에 읽혔는지를 값으로 가르기 위해서다.
//   초기값 80×24 = xterm 생성자 기본값(실물과 같은 값이어야 한다 — 아래 fit 목 주석).
const xtermDims = vi.hoisted(() => ({ cols: 80, rows: 24 }))
vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    loadAddon = vi.fn()
    open = vi.fn()
    reset = vi.fn()
    write = vi.fn()
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
vi.mock('@xterm/addon-webgl', () => ({
  WebglAddon: class {
    onContextLoss = vi.fn()
    dispose = vi.fn()
  },
}))
vi.mock('@xterm/xterm/css/xterm.css', () => ({}))

import TerminalSlot from './TerminalSlot'

const AGENT = 'aaaa-bbbb-cccc-dddd'
const OTHER_AGENT = 'eeee-ffff-0000-1111'
/** 이 하네스에서 "실측된 살아있는 슬롯" 의 치수. */
const measured = { cols: 137, rows: 25 }

async function flushSubscribe(): Promise<void> {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
}

/** 연결 상태 전이(실물 ProtocolClient 가 구독자 전원에게 통지하는 경로와 동형). */
function setConnection(state: 'connected' | 'reconnecting' | 'down'): void {
  act(() => {
    conn.state = state
    for (const cb of conn.cbs) cb(state)
  })
}

function resizeCalls(client: { resizePty: unknown }): ReturnType<typeof vi.fn> {
  return client.resizePty as ReturnType<typeof vi.fn>
}

function subscribeCalls(client: { subscribeOutput: unknown }): number {
  return (client.subscribeOutput as ReturnType<typeof vi.fn>).mock.calls.length
}

beforeEach(() => {
  captured.onChunk = null
  captured.onState = null
  conn.state = 'connected'
  conn.cbs.clear()
  layout.visible = true
  layout.width = 800
  layout.height = 400
  xtermDims.cols = 80
  xtermDims.rows = 24
  // ★실 addon 계약을 그대로 흉내낸다(`@xterm/addon-fit` 소스 확인)★. `proposeDimensions()` 가 undefined 로
  //   빠지는 조건은 셋뿐이다 — terminal 없음 · `element.parentElement` 없음 · **cell metrics 가 0**.
  //   ★컨테이너 박스 크기는 그 셋에 없다★: metrics 는 terminal 안의 span 에서 재므로 박스와 무관하게 살아
  //   있고, 붕괴한 박스는 음수 폭을 낳아 `Math.max(2,·)`/`Math.max(1,·)` 바닥인 2×1 로 **실제 resize 가
  //   걸린다**. no-op 인 쪽은 display:none 이다(글리프를 못 재 metrics 가 0).
  //   목이 이 계약을 어기면 테스트가 라이브러리가 못 내는 값을 지어내고, 그 위에 세운 가드는 한 번도
  //   발화할 수 없는 코드가 된다 — 이 파일의 앞 판이 정확히 그랬다.
  fitAddonFit.mockReset()
  fitAddonFit.mockImplementation(() => {
    if (!layout.visible) return
    if (layout.width === 0 || layout.height === 0) {
      xtermDims.cols = 2
      xtermDims.rows = 1
      return
    }
    xtermDims.cols = Math.max(2, measured.cols)
    xtermDims.rows = Math.max(1, measured.rows)
  })
  agentStoreState.agents = [
    { id: AGENT, cwd: 'C:/x', status: { type: 'Running' }, epoch: 7 },
    { id: OTHER_AGENT, cwd: 'C:/y', status: { type: 'Running' }, epoch: 8 },
  ]
  agentStoreState.agentsLoaded = true
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('TerminalSlot — 재연결 뒤 PTY 치수 재전파', () => {
  it('끊겼다 다시 붙으면 지금 터미널 치수를 PTY 에 다시 밀어 넣는다', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    const subCalls = subscribeCalls(agentClient)
    fitAddonFit.mockClear()
    resizeCalls(agentClient).mockClear()

    setConnection('reconnecting')
    setConnection('connected')

    // fit 앞에서 치수를 읽으면 실측값이 아니라 생성 기본값(80×24)이 나간다.
    expect(fitAddonFit).toHaveBeenCalled()
    expect(agentClient.resizePty).toHaveBeenCalledWith(AGENT, 137, 25)
    // ★재구독 없이★ — 구독이 갈리면 replay 도착 전에 화면이 지워진다(ADR-0164).
    expect(subscribeCalls(agentClient)).toBe(subCalls)
  })

  it('끊긴 동안에는 아무것도 보내지 않고, 다시 붙는 순간 정확히 한 번 보낸다', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    resizeCalls(agentClient).mockClear()

    setConnection('reconnecting')
    setConnection('down')
    expect(agentClient.resizePty).not.toHaveBeenCalled()

    setConnection('connected')
    expect(agentClient.resizePty).toHaveBeenCalledTimes(1)
  })

  // ★값이 아니라 전이를 보는지 가르는 것은 아래 agentId 교체 블록 하나다★. 같은 상태 재통지는 계기가
  //   아니다 — setState 가 같은 값을 걸러 렌더가 안 나고 deps 도 그대로라, 값만 보는 구현도 거기선 안 쏜다.
  //   앞의 재통지·리렌더 단언은 그 계약을 적어 둔 것이고, 구현을 가르는 힘은 교체 블록에만 있다
  //   — ★그 블록을 「중복」으로 지우면 이 테스트는 아무것도 못 가른다★.
  it('붙어 있는 동안에는 발화하지 않는다 — 재통지·리렌더·agentId 교체 어느 것도 계기가 아니다', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    const { rerender } = render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    resizeCalls(agentClient).mockClear()

    setConnection('connected')
    rerender(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    expect(agentClient.resizePty).not.toHaveBeenCalled()

    // 새 에이전트의 초기 1회는 구독 .then 몫이다 — 여기서도 내면 그 위에 겹친다.
    rerender(<TerminalSlot viewId="v1" agentId={OTHER_AGENT} />)
    await flushSubscribe()
    expect(agentClient.resizePty).toHaveBeenCalledTimes(1)
    expect(agentClient.resizePty).toHaveBeenCalledWith(OTHER_AGENT, 137, 25)
  })

  it('붙은 채 마운트하면 구독 경로 1회뿐 — 이 경로가 겹쳐 쏘지 않는다', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    expect(agentClient.resizePty).toHaveBeenCalledTimes(1)
    expect(agentClient.resizePty).toHaveBeenCalledWith(AGENT, 137, 25)
  })

  // 앱은 StrictMode 로 뜬다(main.tsx) — dev 이중 마운트가 effect 를 두 번 돌린다. 전이 기준선이 ref 라
  //   두 번째 실행은 "이미 붙어 있었다" 로 읽혀야 한다. ★마운트 몫을 걷어내고 재연결만 재면 그 주장이
  //   빠진다★ — 마운트 단언을 먼저 세운다.
  it('StrictMode 이중 마운트에서도 마운트 0 · 재연결 1 이다', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    render(
      <StrictMode>
        <TerminalSlot viewId="v1" agentId={AGENT} />
      </StrictMode>,
    )
    await flushSubscribe()

    // 이중 마운트라 구독은 두 번 걸리지만, 살아남은 구독 하나만 .then 에서 치수를 낸다(앞 구독은
    //   cancelled 가드에 걸려 빠진다). 이 경로가 마운트에서 아무것도 안 내야 총계가 1 이다.
    expect(agentClient.resizePty).toHaveBeenCalledTimes(1)

    resizeCalls(agentClient).mockClear()
    setConnection('reconnecting')
    setConnection('connected')
    expect(agentClient.resizePty).toHaveBeenCalledTimes(1)
  })

  // 숨은 슬롯(탭 keep-alive = display:none). 다시 보일 때 가시성 effect 가 같은 전파를 낸다.
  it('숨은 슬롯은 재연결에서도 치수를 전파하지 않는다', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    layout.visible = false
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    fitAddonFit.mockClear()
    resizeCalls(agentClient).mockClear()

    setConnection('down')
    setConnection('connected')

    expect(fitAddonFit).not.toHaveBeenCalled()
    expect(agentClient.resizePty).not.toHaveBeenCalled()
  })

  // ★숨김 판정의 두 번째 갈래★: 표시는 됐는데(offsetParent 살아 있음) 박스만 0 으로 붕괴한 슬롯.
  //   여기서 fit() 은 빠지지 않는다 — cell metrics 가 멀쩡해 음수 폭이 Math.max 바닥 2×1 로 눌리고 그
  //   치수로 실제 resize 가 걸린다. 가드가 offsetParent 한 갈래뿐이면 그 2×1 이 PTY 로 나간다.
  //   ★치수를 검사해서는 이 케이스를 못 막는다★(2×1 은 정상 치수와 구별되지 않는다) — 막는 것은
  //   "언제 보내나" 쪽이라 fit() 에 닿기 전에 빠져야 한다. 그래서 단언도 fit() 미호출에 건다.
  it('표시됐지만 박스가 붕괴한 슬롯은 fit() 에 닿기 전에 빠진다', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    layout.width = 0
    layout.height = 0
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    fitAddonFit.mockClear()
    resizeCalls(agentClient).mockClear()

    setConnection('down')
    setConnection('connected')

    expect(fitAddonFit).not.toHaveBeenCalled()
    expect(agentClient.resizePty).not.toHaveBeenCalled()
  })
})
