// WebGL 좌석 가시성 연동 라이프사이클(ADR-0056) 회귀 — 보이는 슬롯만 WebGL 좌석을 쥔다.
//
// 배경: 모든 터미널 탭은 keep-alive(WindowLayout 이 숨은 탭을 display:none) 로 마운트를 유지한다.
//   브라우저(WebView2/Chromium)는 동시 활성 WebGL 컨텍스트를 하드 상한(실측 16)으로 제한하므로,
//   마운트된 모든 슬롯이 WebGL 을 쥐면 상한을 넘겨 오래된 컨텍스트가 소실된다. 그래서 WebGL addon 을
//   가시성에 묶는다 — 보이면 부착, 숨기면 반납(loseContext→dispose). Terminal 인스턴스(버퍼)는 항상 산다.
//
// ★검증 핵심★: jsdom 은 실 WebGL/IntersectionObserver 가 없으므로 로직 계층만 검사한다 —
//   (1) 보임 전이 → WebglAddon 생성 + loadAddon + fit() + refresh(). (2) 숨김 전이 → loseContext() 를
//   먼저, addon.dispose() 를 다음 순서로 호출하고 새 addon 은 안 만든다 + fit() 미호출. (3) 가시성
//   토글이 Terminal 을 dispose 하거나 출력 구독을 끊지 않는다(언마운트에서만). (4) 초기-숨김 마운트 →
//   첫 "보임" 전까지 WebGL 미생성. (5) 언마운트 → 잡아둔 GL 컨텍스트로 loseContext 호출(containerRef 가
//   commit 단계에서 null 이 돼도 방어). (6) loadAddon throw → 부분생성 addon 을 catch 에서 dispose,
//   ref 는 null 유지, 이후 보임에서 새 addon 1개만 부착(중복부착 없음). (7) RO 가 숨김(offsetParent null
//   / 0 크기)에서 발화 → fit()/resizePty 미호출.
//
// 전략: IntersectionObserver 를 제어 가능한 mock 으로 깔아 테스트가 visible/hidden 콜백을 직접 발화한다.
//   xterm Terminal / FitAddon / WebglAddon 을 stub 해 호출을 정적 holder 로 관측한다. loseContext 는
//   TerminalSlot 이 attach 시점에 container 안 canvas 를 훑어 getContext('webgl2'|'webgl') 로 컨텍스트를
//   잡아두므로(언마운트 ref-null 방어), 보임 *이전에* container 에 canvas 를 심고 그 canvas 의 getContext 를
//   spy 해 잡힌 컨텍스트의 loseContext 호출과 순서를 검사한다.

import { act, cleanup, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { OutputChunk } from '../../api/agentClient'

// jsdom 은 ResizeObserver 를 제공하지 않는다 — TerminalSlot 이 마운트 시 new ResizeObserver 한다.
//   Fix 3 검증(숨김 중 fit/resize 스킵)을 위해 콜백을 잡아 테스트가 직접 발화할 수 있게 한다.
type ROEntry = { contentRect: { width: number; height: number } }
const roState = vi.hoisted(() => ({
  instances: [] as Array<{ cb: (entries: ROEntry[]) => void }>,
}))
globalThis.ResizeObserver = class {
  cb: (entries: ROEntry[]) => void
  constructor(cb: (entries: ROEntry[]) => void) {
    this.cb = cb
    roState.instances.push(this)
  }
  observe() {}
  unobserve() {}
  disconnect() {}
} as unknown as typeof ResizeObserver

/** 마지막 RO 인스턴스에 크기 변화 콜백을 발화한다(entry contentRect 크기 지정). */
function fireResize(width: number, height: number): void {
  const ro = roState.instances[roState.instances.length - 1]
  if (!ro) throw new Error('no ResizeObserver instance')
  act(() => ro.cb([{ contentRect: { width, height } }]))
}

// ── 제어 가능한 IntersectionObserver mock ────────────────────────────────────────────
// 실 IO 는 관측 즉시(현재 가시성 반영) + 전이마다 콜백을 발화한다. 여기선 발화를 테스트가 직접 몰아
//   초기-숨김/보임/숨김 전이를 재현한다. instances 로 마지막 인스턴스에 접근해 콜백을 발사한다.
type IOEntry = { isIntersecting: boolean; intersectionRatio: number }
const ioState = vi.hoisted(() => ({
  instances: [] as Array<{ cb: (entries: IOEntry[]) => void; disconnected: boolean }>,
}))
class MockIntersectionObserver {
  cb: (entries: IOEntry[]) => void
  disconnected = false
  constructor(cb: (entries: IOEntry[]) => void) {
    this.cb = cb
    ioState.instances.push(this)
  }
  observe() {}
  unobserve() {}
  disconnect() {
    this.disconnected = true
  }
  takeRecords() {
    return []
  }
}
globalThis.IntersectionObserver = MockIntersectionObserver as unknown as typeof IntersectionObserver

/** 마지막으로 생성된 IO 인스턴스에 가시성 전이 콜백을 발화한다. */
function fireVisibility(visible: boolean): void {
  const io = ioState.instances[ioState.instances.length - 1]
  if (!io) throw new Error('no IntersectionObserver instance')
  act(() => io.cb([{ isIntersecting: visible, intersectionRatio: visible ? 1 : 0 }]))
}

// ── xterm/addon stub — 생성·호출을 정적 holder 로 관측. ──────────────────────────────
// WebglAddon 인스턴스는 부착마다 새로 생기므로, 생성 목록·dispose·onContextLoss 를 hoisted holder 에
//   모아 테스트가 개수/순서를 검사한다. loseContext 순서 검사를 위해 호출 로그(order)를 공유한다.
const order = vi.hoisted(() => ({ log: [] as string[] }))
const webglState = vi.hoisted(() => ({
  instances: [] as Array<{ dispose: ReturnType<typeof vi.fn>; onContextLoss: ReturnType<typeof vi.fn> }>,
}))
vi.mock('@xterm/addon-webgl', () => ({
  WebglAddon: class {
    onContextLoss = vi.fn()
    dispose = vi.fn(() => {
      order.log.push('dispose')
    })
    constructor() {
      webglState.instances.push(this)
    }
  },
}))

const fitState = vi.hoisted(() => ({ fit: vi.fn() }))
vi.mock('@xterm/addon-fit', () => ({
  FitAddon: class {
    fit = fitState.fit
  },
}))

// Terminal stub — loadAddon/refresh/dispose 를 관측한다. dispose 관측으로 "가시성 토글이 Terminal 을
//   죽이지 않는다"를 검사한다. cols/rows 는 refresh(0, rows-1) 인자 검증용 고정값.
const termState = vi.hoisted(() => ({
  loadAddon: vi.fn(),
  refresh: vi.fn(),
  dispose: vi.fn(),
  reset: vi.fn(),
  write: vi.fn(),
}))
vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    loadAddon = termState.loadAddon
    open = vi.fn()
    reset = termState.reset
    write = termState.write
    refresh = termState.refresh
    onData = vi.fn(() => ({ dispose: vi.fn() }))
    dispose = termState.dispose
    cols = 80
    rows = 24
  },
}))
vi.mock('@xterm/xterm/css/xterm.css', () => ({}))

// ── agentClient stub — 출력 구독 unsubscribe 관측(가시성 토글이 구독을 안 끊는지). ──────────
const captured = vi.hoisted(() => ({
  onChunk: null as ((c: OutputChunk) => void) | null,
  unsubscribe: null as ReturnType<typeof vi.fn> | null,
}))
vi.mock('../../api/clientFactory', () => ({
  agentClient: {
    subscribeOutput: vi.fn(
      async (_viewId: string, _agentId: string, onChunk: (c: OutputChunk) => void) => {
        captured.onChunk = onChunk
        const unsubscribe = vi.fn()
        captured.unsubscribe = unsubscribe
        return { unsubscribe }
      },
    ),
    writeStdin: vi.fn(async () => undefined),
    resizePty: vi.fn(async () => undefined),
    connectionState: 'connected',
  },
  getAgentClient: vi.fn(),
}))

// ── agentStore stub — 슬롯이 종료 판정용으로 useAgentStore(s => s.agents) 를 조회. 빈 목록 = 살아있음. ──
const agentStoreState = vi.hoisted(() => ({ agents: [] as unknown[] }))
vi.mock('../../store/agentStore', () => ({
  useAgentStore: (selector: (s: { agents: unknown[] }) => unknown) => selector(agentStoreState),
}))

// ── 테스트 대상 ────────────────────────────────────────────────────────────────────
import TerminalSlot from './TerminalSlot'

const AGENT = 'aaaa-bbbb-cccc-dddd'

/** 마운트 직후 subscribeOutput 이 콜백을 등록(async .then)할 때까지 마이크로태스크를 비운다. */
async function flushSubscribe(): Promise<void> {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
}

/**
 * TerminalSlot 이 attach 시점에 GL 컨텍스트를 잡도록 container 안에 WebGL canvas 를 심고, 그
 * getContext/loseContext 를 spy 로 노출한다. 실 WebglAddon 이 append 하는 canvas 를 대역 — TerminalSlot 은
 * addon 이 아니라 DOM canvas 를 훑어 컨텍스트를 얻으므로(버전-견고 경로), 이 대역이 그 경로를 재현한다.
 *
 * ★호출 시점★: 컨텍스트 캡처는 attachWebgl(=보임 전이)에서 일어나므로, 이 canvas 는 반드시 fireVisibility(true)
 *   *이전에* 심어야 캡처된다. (구조상 언마운트 후 containerRef 가 null 이어도 캡처된 컨텍스트로 반납하는 게 Fix 1.)
 */
function installWebglCanvas(): { loseContext: ReturnType<typeof vi.fn> } {
  const loseContext = vi.fn(() => {
    order.log.push('loseContext')
  })
  const gl = { getExtension: vi.fn((name: string) => (name === 'WEBGL_lose_context' ? { loseContext } : null)) }
  const canvas = document.createElement('canvas')
  // getContext('webgl2') 가 우리 대역 gl 을 돌려주도록 오버라이드(jsdom 은 null 반환).
  canvas.getContext = vi.fn((type: string) => (type === 'webgl2' ? gl : null)) as unknown as typeof canvas.getContext
  // container 는 TerminalSlot 의 containerRef div = 바깥(padding) div 안의 자식 div(자식 없는 leaf).
  //   실 WebglAddon 이 이 div 에 canvas 를 append 하므로 대역도 정확히 같은 위치에 심는다 —
  //   attachWebgl 은 containerRef.current 안에서만 canvas 를 훑으므로 위치가 틀리면 컨텍스트를 못 잡는다.
  const outer = document.querySelector('div[style*="padding"]')
  const container = outer?.querySelector('div')
  if (!container) throw new Error('containerRef div not found')
  container.appendChild(canvas)
  return { loseContext }
}

beforeEach(() => {
  ioState.instances = []
  roState.instances = []
  webglState.instances = []
  order.log = []
  captured.onChunk = null
  captured.unsubscribe = null
  agentStoreState.agents = []
  termState.loadAddon.mockReset()
  termState.refresh.mockClear()
  termState.dispose.mockClear()
  termState.reset.mockClear()
  termState.write.mockClear()
  fitState.fit.mockClear()
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('TerminalSlot — WebGL 좌석 가시성 연동(ADR-0056)', () => {
  it('마운트만으론 WebGL 을 만들지 않는다 — 첫 "보임" 콜백 전까지 좌석 미점유(초기-숨김 안전)', async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    // 아직 가시성 콜백 없음 → WebglAddon 0개.
    expect(webglState.instances).toHaveLength(0)
    // IO 는 마운트 시 생성됐어야 한다(관측 시작).
    expect(ioState.instances.length).toBeGreaterThan(0)
  })

  it('보임 전이 → WebglAddon 생성 + loadAddon + fit() + refresh()', async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    fireVisibility(true)

    expect(webglState.instances).toHaveLength(1)
    // 새 addon 이 Terminal 에 로드됐다.
    expect(termState.loadAddon).toHaveBeenCalledWith(webglState.instances[0])
    // 보이는 상태이므로 fit() 로 치수 갱신 후 refresh(0, rows-1) 로 현재 버퍼를 즉시 그린다.
    expect(fitState.fit).toHaveBeenCalled()
    expect(termState.refresh).toHaveBeenCalledWith(0, 23) // rows(24) - 1
  })

  it('숨김 전이 → loseContext() 를 먼저, addon.dispose() 를 다음 순서로 호출 + 새 addon 미생성 + fit() 미호출', async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    // 캡처는 attach 시점 → 보임 *전에* canvas 를 심어야 컨텍스트가 잡힌다.
    const { loseContext } = installWebglCanvas()
    fireVisibility(true)
    expect(webglState.instances).toHaveLength(1)

    // 숨김 전이 직전까지의 fit() 호출 수 기록 → 이후 증가하지 않아야(숨김 중 fit 금지).
    fitState.fit.mockClear()
    order.log = []

    fireVisibility(false)

    // ★load-bearing 순서★: dispose 만으론 GPU 좌석이 GC 전까지 안 풀린다 → 반드시 loseContext 를 *먼저*.
    expect(loseContext).toHaveBeenCalled()
    expect(order.log).toEqual(['loseContext', 'dispose'])
    // 숨김은 좌석 반납만 — 새 addon 을 만들지 않는다.
    expect(webglState.instances).toHaveLength(1)
    // 숨김 중엔 측정 불가라 fit() 을 부르지 않는다(쓰레기 치수 방지).
    expect(fitState.fit).not.toHaveBeenCalled()
  })

  it('가시성 토글이 Terminal 을 dispose 하거나 출력 구독을 끊지 않는다(언마운트에서만)', async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    expect(captured.unsubscribe).toBeTruthy()

    installWebglCanvas()
    fireVisibility(true)
    fireVisibility(false)
    fireVisibility(true)

    // 토글을 여러 번 돌려도 Terminal 은 살아있고(구독·버퍼 보존), 구독도 안 끊긴다.
    expect(termState.dispose).not.toHaveBeenCalled()
    expect(captured.unsubscribe).not.toHaveBeenCalled()
  })

  it('보임 → 숨김 → 보임 재부착 → 두 번째 WebglAddon 이 새로 생성된다(좌석 재획득)', async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    installWebglCanvas()
    fireVisibility(true)
    fireVisibility(false)
    // 숨김 후엔 addon ref 가 비므로 다음 보임에서 새로 만든다.
    fireVisibility(true)

    expect(webglState.instances).toHaveLength(2)
  })

  // ── Fix 1 (HIGH): 언마운트 시에도 좌석 반납 ──────────────────────────────────────────
  it('언마운트 → 잡아둔 GL 컨텍스트로 loseContext 호출(containerRef 가 null 이 돼도 반납된다)', async () => {
    const { unmount } = render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    // 보임 전에 canvas 를 심어 attach 시점 캡처 → 언마운트에서 containerRef 가 null 이어도 이 컨텍스트로 반납.
    const { loseContext } = installWebglCanvas()
    fireVisibility(true)
    expect(webglState.instances).toHaveLength(1)

    order.log = []
    act(() => unmount())

    // 캡처된 컨텍스트로 loseContext → dispose 순서. containerRef 미의존이 핵심(React 가 commit 에서 null 로 비움).
    expect(loseContext).toHaveBeenCalled()
    expect(order.log).toEqual(['loseContext', 'dispose'])
    // 언마운트에선 Terminal 도 dispose 된다(가시성 토글과 달리).
    expect(termState.dispose).toHaveBeenCalled()
  })

  // ── Fix 2 (HIGH): loadAddon throw → 부분생성 addon dispose + ref null + 중복부착 없음 ──
  it('보임 시 loadAddon 이 throw → 생성된 addon 을 catch 에서 dispose, ref 는 비고, 다음 보임에서 새 addon 1개만 부착', async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    installWebglCanvas()
    // 첫 보임에서 loadAddon 이 던지게 한다(부분생성 경로 재현).
    termState.loadAddon.mockImplementationOnce(() => {
      throw new Error('loadAddon boom')
    })

    fireVisibility(true)

    // addon 은 생성됐지만(1개) loadAddon 실패 → catch 에서 dispose 됐어야 한다.
    expect(webglState.instances).toHaveLength(1)
    expect(webglState.instances[0].dispose).toHaveBeenCalled()

    // 다음 보임(정상 loadAddon)에서 새 addon 이 정확히 하나만 붙는다 — 이전 실패분이 ref 에 남았으면
    //   attach 초입 가드(webglAddonRef.current) 때문에 새로 안 붙는다. 즉 두 번째가 붙었다 = ref 가 비어있었다.
    fireVisibility(true)
    expect(webglState.instances).toHaveLength(2)
    expect(termState.loadAddon).toHaveBeenLastCalledWith(webglState.instances[1])
  })

  // ── Fix 3 (MEDIUM): 숨김 상태에서 RO 발화 → fit()/resizePty 스킵 ─────────────────────
  it('ResizeObserver 가 0 크기(숨김)에서 발화 → fit() 과 resizePty 를 호출하지 않는다', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    fitState.fit.mockClear()
    ;(agentClient.resizePty as ReturnType<typeof vi.fn>).mockClear()

    // 0 폭·높이 = display:none 붕괴 재현. offsetParent 도 jsdom 에서 null 이라 hidden 신호가 성립.
    fireResize(0, 0)

    expect(fitState.fit).not.toHaveBeenCalled()
    expect(agentClient.resizePty).not.toHaveBeenCalled()
  })
})
