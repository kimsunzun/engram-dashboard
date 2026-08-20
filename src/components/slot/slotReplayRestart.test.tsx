// 다른 화신 부착(비우기 신호 onReset) 회귀 — 슬롯이 화면과 자기 seq 가드를 함께 비운다. 그리고 그
// 반대편 — 어떤 국면 통지(onState)로도 화면을 건드리지 않는다.
//
// 배경(실측 2026-08-20): 다른 화신은 프레임 번호를 0 부터 다시 매긴다. 클라(ProtocolClient)는 그 replay 가
//   **도착한** 자리에서 뷰 진도 커서를 버리고 onReset 을 부른 뒤 같은 틱에 전량을 배달한다. 슬롯이 그
//   신호에 반응하지 않으면 둘 중 하나로 깨진다 —
//   (a) 로컬 seq 가드가 옛 high-water 에 머물러 0 부터 다시 오는 프레임을 전부 탈락시켜 화면이 영영
//   빈 채로 남거나, (b) 화면을 안 비워 전량 replay 가 기존 내용 위에 겹쳐 쌓인다.
//   반대로 국면 통지에까지 비우면 소켓이 깜빡일 때마다 화면이 사라졌다 다시 그려진다 — 'buffering' 에는
//   같은 화신 이어보기가 섞여 있다.
//
// 전략: subscribeOutput 을 mock 해 onChunk/onState/onReset 을 캡처하고, 테스트가 직접 발화한 뒤
//   seq 0 부터 다시 프레임을 먹인다(slotTagGate.test.tsx 와 동일 stub 패턴). RichSlot 쪽 같은 회귀는
//   RichSlot.test.tsx 가 자기 하네스로 본다.

import { act, cleanup, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { FRAME_TAG_TERMINAL_BYTES } from '../../api/wsFrame'
import type { OutputChunk, ViewPhase } from '../../api/agentClient'

// jsdom 미제공 관측자 2종 — TerminalSlot 이 마운트 시 둘 다 생성한다(크기 추적 · ADR-0056 WebGL 가시성).
//   콜백을 한 번도 발화하지 않는 no-op 이라 데이터 경로만 남는다.
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

const captured = vi.hoisted(() => ({
  onChunk: null as ((c: OutputChunk) => void) | null,
  onState: null as ((s: ViewPhase) => void) | null,
  onReset: null as (() => void) | null,
}))

vi.mock('../../api/clientFactory', () => ({
  agentClient: {
    // ADR-0046 시그니처 (viewId, agentId, onChunk, onState?, onReset?).
    subscribeOutput: vi.fn(
      async (
        _viewId: string,
        _agentId: string,
        onChunk: (c: OutputChunk) => void,
        onState?: (s: ViewPhase) => void,
        onReset?: () => void,
      ) => {
        captured.onChunk = onChunk
        captured.onState = onState ?? null
        captured.onReset = onReset ?? null
        return { unsubscribe: vi.fn() }
      },
    ),
    writeStdin: vi.fn(async () => undefined),
    resizePty: vi.fn(async () => undefined),
    connectionState: 'connected',
    // 연결 상태 표면 — 슬롯이 부재 막 판정에 읽는다(등록 즉시 1회 통지 + disposer 반환).
    onConnectionStateChange: (cb: (s: string) => void) => {
      cb('connected')
      return () => {}
    },
  },
  getAgentClient: vi.fn(),
}))

const agentStoreState = vi.hoisted(() => ({ agents: [] as unknown[], agentsLoaded: false }))
vi.mock('../../store/agentStore', () => ({
  useAgentStore: (selector: (s: typeof agentStoreState) => unknown) => selector(agentStoreState),
}))

// xterm stub — reset/write 를 정적 holder 로 공유해 인스턴스 교체와 무관하게 호출을 센다.
const xtermReset = vi.hoisted(() => vi.fn())
const xtermWrite = vi.hoisted(() => vi.fn())
// 치수는 holder 를 통해 읽는다 — fit() 이 실제로 측정을 반영한 뒤에 읽혔는지를 값으로 가르기 위해서다
//   (fit 전 값 80×24, 후 값은 테스트가 심는다).
const xtermDims = vi.hoisted(() => ({ cols: 80, rows: 24 }))
vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    loadAddon = vi.fn()
    open = vi.fn()
    reset = xtermReset
    write = xtermWrite
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
vi.mock('@xterm/addon-webgl', () => ({ WebglAddon: class { onContextLoss = vi.fn(); dispose = vi.fn() } }))
vi.mock('@xterm/xterm/css/xterm.css', () => ({}))

import TerminalSlot from './TerminalSlot'
import DomSlot from './DomSlot'

const AGENT = 'aaaa-bbbb-cccc-dddd'
const enc = new TextEncoder()

function tag0(seq: number, text: string): OutputChunk {
  return { seq, tag: FRAME_TAG_TERMINAL_BYTES, bytes: enc.encode(text) }
}

async function flushSubscribe(): Promise<void> {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
}

function fireState(state: ViewPhase): void {
  act(() => captured.onState!(state))
}

function fireReset(): void {
  act(() => captured.onReset!())
}

/** 국면 어휘 전량 — "어느 값으로도 비우지 않는다" 를 하나씩이 아니라 집합으로 잰다. */
const ALL_PHASES: ViewPhase[] = ['detached', 'buffering', 'live', 'error']

function written(): string[] {
  return xtermWrite.mock.calls.map((c) => new TextDecoder().decode(c[0] as Uint8Array))
}

/**
 * 슬롯을 "보이는" 상태로 만든다 — jsdom 은 레이아웃이 없어 `offsetParent` 를 **항상** null 로 내고, 그건
 * 슬롯이 숨김(탭 keep-alive = display:none)을 판정하는 바로 그 신호다(ADR-0056). 손대지 않으면 보이는
 * 슬롯의 갈래를 이 하네스에서 아예 밟을 수 없다.
 */
function showContainer(): void {
  const container = document.querySelector('div[style*="padding"]')?.querySelector('div')
  if (!container) throw new Error('containerRef div not found')
  Object.defineProperty(container, 'offsetParent', { value: document.body, configurable: true })
}

function domText(): string {
  return (document.querySelector('[data-dom-mode="1"]') as HTMLElement).textContent ?? ''
}

beforeEach(() => {
  captured.onChunk = null
  captured.onState = null
  captured.onReset = null
  xtermReset.mockClear()
  xtermWrite.mockClear()
  xtermDims.cols = 80
  xtermDims.rows = 24
  fitAddonFit.mockReset()
  agentStoreState.agents = []
  agentStoreState.agentsLoaded = false
})

afterEach(() => {
  cleanup()
})

describe('TerminalSlot — 비우기 신호에 화면·seq 가드 리셋', () => {
  it('onReset 이 오면 xterm 을 지우고, seq 0 부터 다시 오는 전량을 그린다', async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    // ★비우기 콜백을 넘기는 것 자체가 회귀 대상★ — 안 넘기면 커서만 0 으로 돌아가고 화면은 앞 화신
    //   내용을 든 채라 전량 replay 가 그 위에 겹쳐 쌓인다(어디에도 오류가 안 남는다).
    expect(captured.onReset).toBeTruthy()

    act(() => captured.onChunk!(tag0(0, 'first')))
    act(() => captured.onChunk!(tag0(1, 'second')))
    expect(written()).toEqual(['first', 'second'])

    const resetsBefore = xtermReset.mock.calls.length
    fireReset()
    expect(xtermReset.mock.calls.length).toBe(resetsBefore + 1)

    // 다른 화신 — 번호가 0 부터 다시 시작한다. 가드를 안 되돌렸으면 전부 탈락한다.
    act(() => captured.onChunk!(tag0(0, 'again')))
    expect(written()).toEqual(['first', 'second', 'again'])
  })

  // ★재부착에서 PTY 치수를 다시 밀어 넣나★ — 데몬을 재기동하면 복원된 에이전트의 PTY 는 spawn 기본값
  //   (80×24)에서 시작하는데, 이 슬롯의 xterm 은 사용자가 벌려 둔 치수 그대로다. 상위가 넘기는
  //   prop(viewId·agentId)은 재기동 전후로 그대로여서 구독 effect 가 다시 돌지 않으므로, 치수를 전파할
  //   자리가 이 비우기 신호 말고 없다(RO 는 크기 *변화* 시에만, 가시성 effect 는 전이 시에만 발화한다).
  //   어긋난 채 live 가 되면 TUI 가 자기가 믿는 크기로 계산해 매 갱신이 한 행·수십 열 빗나간다.
  it('재부착(비우기 신호)에서 지금 터미널 치수를 PTY 에 다시 밀어 넣는다', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    showContainer()

    const subCalls = (agentClient.subscribeOutput as ReturnType<typeof vi.fn>).mock.calls.length
    // fit() = "지금 컨테이너를 실측했다" — 그 결과가 137×25 다(실측한 라이브 값). 전파가 fit 앞에서
    //   치수를 읽으면 여기 심은 값이 아니라 이전 값(80×24)이 나간다.
    fitAddonFit.mockImplementation(() => {
      xtermDims.cols = 137
      xtermDims.rows = 25
    })
    // mount·구독 경로가 이미 불러 둔 호출을 걷는다(구현은 남는다) — 아래 단언은 *재부착이* 실측했나다.
    fitAddonFit.mockClear()
    ;(agentClient.resizePty as ReturnType<typeof vi.fn>).mockClear()

    fireReset()

    expect(fitAddonFit).toHaveBeenCalled()
    expect(agentClient.resizePty).toHaveBeenCalledWith(AGENT, 137, 25)
    // 재구독을 거치지 않은 채(= prop 불변) 전파됐다는 것까지 못 박는다 — 구독이 갈리면 replay 도착 전에
    //   화면이 지워지는 결함으로 되돌아간다.
    expect((agentClient.subscribeOutput as ReturnType<typeof vi.fn>).mock.calls.length).toBe(subCalls)
  })

  // 반대편 — 숨은 슬롯(탭 keep-alive)에서는 보내지 않는다. 붕괴한 컨테이너의 fit() 은 최소 치수를 내고,
  //   그 치수가 PTY 로 나가면 에이전트 레이아웃이 깨진다(ADR-0056).
  it('숨은 슬롯의 재부착은 치수를 전파하지 않는다', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    fitAddonFit.mockClear()
    ;(agentClient.resizePty as ReturnType<typeof vi.fn>).mockClear()

    fireReset()

    expect(fitAddonFit).not.toHaveBeenCalled()
    expect(agentClient.resizePty).not.toHaveBeenCalled()
  })

  // ★반대 방향 회귀★: 국면 통지에 화면을 비우면, 소켓이 깜빡일 때마다 터미널이 통째로 지워졌다 전량
  //   재replay 로 다시 그려진다(사용자가 거부한 형태). 'buffering' 에 같은 화신 이어보기가 섞여 있어서다.
  it.each(ALL_PHASES)("'%s' 국면은 화면도 가드도 건드리지 않는다", async (phase) => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    act(() => captured.onChunk!(tag0(0, 'keep')))

    const resetsBefore = xtermReset.mock.calls.length
    fireState(phase)
    expect(xtermReset.mock.calls.length).toBe(resetsBefore)
    act(() => captured.onChunk!(tag0(0, 'dup'))) // 이미 그린 seq → 여전히 탈락
    expect(written()).toEqual(['keep'])
  })

  // 부재를 표면에 남긴다 — 명부에 그 에이전트가 없는 동안 슬롯이 "살아 있는데 조용한" 것처럼 보이면 안 된다.
  //   (지우지 않는다는 쪽은 위 국면 전량 케이스가 잰다 — 여기선 표시만.)
  it("'detached' 는 대기 표시를 띄우고, 부착되면 내린다", async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    fireState('detached')
    expect(document.querySelector('[data-slot-detached="1"]')).not.toBeNull()

    fireState('buffering') // 부착되면 대기 표시가 내려간다
    expect(document.querySelector('[data-slot-detached="1"]')).toBeNull()
  })

  // ★사다리를 소진한 뷰도 표면에 남긴다★: 'error' 는 아무것도 못 받는 상태인데, 이걸 부재로 안 보면
  //   명부에도 있고 연결도 붙은 슬롯이 **아무 표시 없는 판**으로 남는다 — 무엇이 잘못됐는지도, 어떻게
  //   되살리는지도 화면에 없다. 대기('detached')와는 관측 표면에서 갈린다(그림은 같다).
  it("'error' 도 부재 막을 띄운다(대기와는 표면에서 갈린다)", async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    fireState('error')
    const veil = document.querySelector('[data-slot-dead="1"]')
    expect(veil).not.toBeNull()
    expect(veil!.getAttribute('data-slot-phase')).toBe('error')
    expect(veil!.getAttribute('data-slot-detached')).toBeNull()
  })
})

describe('DomSlot — 비우기 신호에 관측 텍스트·seq 가드 리셋', () => {
  it('onReset 이 오면 <pre> 를 비우고 전량을 한 벌로 다시 채운다', async () => {
    render(<DomSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    expect(captured.onReset).toBeTruthy()

    act(() => captured.onChunk!(tag0(0, 'old-output')))
    expect(domText()).toBe('old-output')

    fireReset()
    expect(domText()).toBe('')

    act(() => captured.onChunk!(tag0(0, 'old-output')))
    // 겹쳐 쌓였으면 'old-outputold-output', 가드가 남았으면 '' 이 된다.
    expect(domText()).toBe('old-output')
  })

  it.each(ALL_PHASES)("'%s' 국면은 관측 텍스트를 그대로 둔다", async (phase) => {
    render(<DomSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    act(() => captured.onChunk!(tag0(0, 'old-output')))

    fireState(phase)
    expect(domText()).toBe('old-output')
    act(() => captured.onChunk!(tag0(0, 'dup'))) // 가드도 그대로 → 이미 그린 seq 는 탈락
    expect(domText()).toBe('old-output')
  })

  it("'detached' 는 대기 배지를 띄운다", async () => {
    render(<DomSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    fireState('detached')
    expect(document.querySelector('[data-slot-detached="1"]')).not.toBeNull()
  })

  it("'error' 도 부재 막을 띄운다(TerminalSlot 동형)", async () => {
    render(<DomSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    fireState('error')
    const veil = document.querySelector('[data-slot-dead="1"]')
    expect(veil).not.toBeNull()
    expect(veil!.getAttribute('data-slot-phase')).toBe('error')
  })

  // ★청크 경계 상태도 앞 화신 것이다 — 함께 버려야 한다★. 아래 둘은 DomSlot 만의 회귀다(TerminalSlot 은
  //   디코드·ANSI strip 을 하지 않고 바이트를 그대로 xterm 에 넘긴다).

  // 화신 교체가 멀티바이트 문자 한복판에서 걸리면, 살아남은 TextDecoder 가 앞 화신의 미완 바이트를 물고
  //   있다가 새 replay 첫 글자 앞에 U+FFFD 를 붙인다.
  it('onReset 은 미완 멀티바이트를 물고 있는 디코더를 갈아 끼운다', async () => {
    render(<DomSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    // '한'(U+D55C) = EC 95 9C 중 앞 두 바이트만 도착한 채 세션이 끊긴다.
    act(() =>
      captured.onChunk!({
        seq: 0,
        tag: FRAME_TAG_TERMINAL_BYTES,
        bytes: new Uint8Array([0xec, 0x95]),
      }),
    )
    expect(domText()).toBe('')

    fireReset()
    act(() => captured.onChunk!(tag0(0, 'A')))
    // 디코더를 안 갈면 '�A'(혹은 대체문자 2개 + A)가 된다.
    expect(domText()).toBe('A')
  })

  // 같은 자리의 ANSI 판: 화신 교체가 미완 ESC 시퀀스 한복판에서 걸리면 그 꼬리가 다음 replay 첫 바이트에
  //   붙어 시퀀스로 오인되고, strip 이 새 replay 의 첫 글자들을 통째로 삼킨다.
  it('onReset 은 미완 ESC 꼬리(pending)도 버린다', async () => {
    render(<DomSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    act(() => captured.onChunk!(tag0(0, 'ok['))) // CSI 가 종료 바이트 없이 끊김 → pending 에 hold
    expect(domText()).toBe('ok')

    fireReset()
    act(() => captured.onChunk!(tag0(0, 'Hello')))
    // pending 이 남았으면 '[' + 'Hello' 가 미완 CSI 로 다시 hold 돼 화면이 빈 채로 남는다.
    expect(domText()).toBe('Hello')
  })
})
