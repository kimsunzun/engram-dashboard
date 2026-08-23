// uiSettings — 디스크 값이 `data-theme` 까지 닿는가 + ★그 적용이 트리를 다시 마운트하지 않는가★.
//
// 뒤쪽이 이 스위트의 존재 이유다(ADR-0149): 슬롯이 다시 마운트되면 챗은 컴포넌트 상태라 대화가 영구 소실된다.
//
// ★`Child` 가 테마를 **구독**하는 것이 그 단언의 전제다★ — 아무것도 구독하지 않는 자식은 구현이 무엇을 하든
// 리렌더조차 안 돼서 mount 횟수·노드 동일성이 항상 통과한다(즉 리마운트하는 구현도 초록이다). 구독시켜야
// 테마 변경이 실제 리렌더를 일으키고, 그때도 마운트가 한 번뿐이라는 것이 비로소 재는 값이 된다.
// 구독이 살아 있다는 것 자체도 매번 확인한다(그린 텍스트가 바뀌는지) — 안 그러면 이 전제가 조용히 썩는다.
//
// ★안 재는 것 — 운영 트리(알려진 갭)★: 여기 `Host`/`Child` 는 **합성 하네스**이지 실제 슬롯 트리가 아니다.
// 그래서 이 스위트는 "테마 적용 경로가 리마운트를 유발하지 않는다"까지만 재고, 앞으로 누가 테마를 구독하면서
// **동시에 키가 갈리는** 컴포넌트를 운영 트리에 넣으면 여기는 초록인 채로 남는다. 그 갭을 메우려면 실제
// 슬롯 서브트리를 띄우는 테스트가 따로 서야 한다.

import { act, cleanup, render, screen } from '@testing-library/react'
import { useEffect } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invokeMock = vi.fn(async (_cmd: string) => ({ theme: 'dark' }) as unknown)
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string) => invokeMock(cmd),
}))

/** 열어 줄 때까지 매달리는 약속 — `listen()` 등록을 테스트가 붙잡아 두는 데 쓴다. */
function deferred(): { promise: Promise<void>; open: () => void } {
  let open: () => void = () => {}
  const promise = new Promise<void>(resolve => {
    open = resolve
  })
  return { promise, open }
}

// listen mock: 등록된 핸들러를 보관해 테스트가 셸 푸시를 직접 흉내낸다(viewStore.test 와 같은 모양).
// listenGate 가 걸려 있으면 등록 자체를 붙잡아 둔다 — 「등록 전에는 부팅 조회를 안 낸다」를 재는 수단.
// ★`options` 도 함께 넘긴다★ — 구독 타깃(어느 창 몫인가)이 이 인자에 실린다.
const listeners = new Map<string, (e: { payload: unknown }) => void>()
let listenGate: { promise: Promise<void>; open: () => void } | null = null
const unlistenMock = vi.fn()
async function defaultListen(
  event: string,
  handler: (e: { payload: unknown }) => void,
  _options?: unknown,
) {
  if (listenGate) await listenGate.promise
  listeners.set(event, handler)
  return unlistenMock
}
const listenMock = vi.fn(defaultListen)
vi.mock('@tauri-apps/api/event', () => ({
  listen: (event: string, handler: (e: { payload: unknown }) => void, options?: unknown) =>
    listenMock(event, handler, options),
}))

const WINDOW_LABEL = 'slot-popup-1'
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ label: WINDOW_LABEL }),
}))

import { useThemeStore } from '../store/themeStore'
import { installUiSettings } from './uiSettings'

const EVT = 'ui:settings-updated'

/** 셸 푸시 흉내 — React 밖에서 오는 콜백이라 act 로 감싼다(운영에서도 Tauri 이벤트가 같은 모양이다). */
function push(theme: unknown): void {
  const h = listeners.get(EVT)
  if (!h) throw new Error(`${EVT} 리스너가 없다 — installUiSettings 가 등록에 실패했나?`)
  act(() => {
    h({ payload: { theme } })
  })
}

/** 부팅 조회(마이크로태스크 사슬)와 그 뒤 렌더까지 흘려보낸다. */
async function settle(): Promise<void> {
  await act(async () => {
    await new Promise(resolve => setTimeout(resolve, 0))
  })
}

function themeAttr(): string | null {
  return document.documentElement.getAttribute('data-theme')
}

let childMounts = 0

/** ★테마를 구독한다★ — 이 구독이 없으면 아래 리마운트 단언이 아무것도 못 잡는다(파일 머리말). */
function Child() {
  const theme = useThemeStore(s => s.theme)
  useEffect(() => {
    childMounts += 1
  }, [])
  return <div data-testid="child">{theme}</div>
}

/** App 이 하는 것과 같은 배선 — 마운트 때 한 번 걸고 언마운트 때 푼다. */
function Host() {
  useEffect(() => installUiSettings(), [])
  return <Child />
}

beforeEach(() => {
  childMounts = 0
  listeners.clear()
  listenGate = null
  unlistenMock.mockClear()
  // ★함정: `mockClear()` 는 **호출 기록만** 지우고 `mockImplementation` 은 남긴다★ — 그래서 구현까지
  //   되돌리는 `mockReset()` + 기본 구현 재설치가 필요하다. 아래쪽 재시도 소진 테스트가 거는 「listen 이
  //   항상 실패」 구현이 살아남으면, **그 뒤에 오는 테스트 전부**가 조용히 그 세계에서 돈다. 증상은
  //   「관계없어 보이는 수명 테스트 여러 개가 한꺼번에 깨진다」였다(실발생 — 세 개). 같은 함정이
  //   `invokeMock` 에도 있어 아래에서 구현을 매번 다시 건다.
  listenMock.mockReset()
  listenMock.mockImplementation(defaultListen)
  invokeMock.mockClear()
  invokeMock.mockImplementation(async () => ({ theme: 'dark' }))
  document.documentElement.removeAttribute('data-theme')
  useThemeStore.setState({ theme: 'dark' })
})

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

describe('installUiSettings — 부팅 조회', () => {
  it('파일 값이 data-theme 까지 간다', async () => {
    invokeMock.mockImplementation(async () => ({ theme: 'light' }))
    render(<Host />)
    await settle()

    expect(invokeMock).toHaveBeenCalledWith('get_ui_settings')
    expect(themeAttr()).toBe('light')
    expect(useThemeStore.getState().theme).toBe('light')
  })

  it('e-ink 도 그대로 실린다 — dark/light 로 접히지 않는다(ADR-0062)', async () => {
    invokeMock.mockImplementation(async () => ({ theme: 'e-ink' }))
    render(<Host />)
    await settle()

    expect(themeAttr()).toBe('e-ink')
  })

  it('조회가 실패하면 dark 로 간다(무테마로 남지 않는다)', async () => {
    invokeMock.mockImplementation(async () => {
      throw new Error('셸이 아직 안 떴다')
    })
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    render(<Host />)
    await settle()

    expect(themeAttr()).toBe('dark')
    expect(warn).toHaveBeenCalled()
  })

  it('모르는 이름이 오면 통과시키지 않고 dark 로 접는다', async () => {
    invokeMock.mockImplementation(async () => ({ theme: 'solarized' }))
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    render(<Host />)
    await settle()

    expect(themeAttr()).toBe('dark')
    expect(warn).toHaveBeenCalled()
  })
})

describe('installUiSettings — ui.refresh 푸시', () => {
  // ★셸은 창마다 **그 창의 값**을 보낸다★(ADR-0167). 기본 등록(`Any`)은 필터와 무관하게 전부 깨어나므로
  //   타깃을 안 걸면 이 창이 남의 창 값까지 받아 마지막에 온 것을 칠한다 — 창별 테마가 그 자리에서 무너진다.
  it('자기 창 label 로 구독한다', async () => {
    render(<Host />)
    await settle()

    expect(listenMock).toHaveBeenCalledWith(EVT, expect.any(Function), {
      target: WINDOW_LABEL,
    })
  })

  it('★값만 바뀌고 트리는 그대로 마운트돼 있다★', async () => {
    invokeMock.mockImplementation(async () => ({ theme: 'light' }))
    render(<Host />)
    await settle()
    const node = screen.getByTestId('child')
    expect(themeAttr()).toBe('light')
    expect(node.textContent).toBe('light')
    expect(childMounts).toBe(1)

    push('e-ink')

    expect(themeAttr()).toBe('e-ink')
    // 구독이 살아 있다 = 아래 두 단언이 실제로 무언가를 재고 있다는 증거.
    expect(screen.getByTestId('child').textContent).toBe('e-ink')
    expect(childMounts).toBe(1)
    // 같은 DOM 노드다 — React 가 자식을 갈아끼웠으면(키 교체·리마운트) 여기서 갈린다.
    expect(screen.getByTestId('child')).toBe(node)
    expect(node.isConnected).toBe(true)
  })

  it('푸시가 먼저 닿으면 뒤늦게 온 부팅 조회가 그것을 덮지 않는다', async () => {
    let answer: (value: unknown) => void = () => {}
    invokeMock.mockImplementation(
      () =>
        new Promise(resolve => {
          answer = resolve
        }),
    )
    render(<Host />)
    // 구독 등록은 끝나고 조회는 아직 매달려 있는 상태.
    await settle()

    push('e-ink')
    expect(themeAttr()).toBe('e-ink')

    await act(async () => {
      answer({ theme: 'light' })
      await new Promise(resolve => setTimeout(resolve, 0))
    })

    expect(themeAttr()).toBe('e-ink')
  })

  it('푸시 값이 깨져 있어도 무테마로 남지 않는다', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    render(<Host />)
    await settle()

    push(42)

    expect(themeAttr()).toBe('dark')
    expect(warn).toHaveBeenCalled()
  })
})

describe('installUiSettings — 등록과 조회의 순서', () => {
  // ★이 순서가 유실 방지의 전부다★: 둘을 나란히 띄우면 등록이 끝나기 전에 온 알림은 이 창에 도착조차 안 하고
  //   (리스너가 없다) 그 뒤 늦게 온 조회 답이 옛 값을 칠한다. `pushed` 빗장은 알림을 **받았을 때만** 서므로
  //   그 인터리브를 못 막는다.
  it('구독 등록이 끝나기 전에는 부팅 조회를 내지 않는다', async () => {
    const gate = deferred()
    listenGate = gate
    let fileTheme = 'light'
    invokeMock.mockImplementation(async () => ({ theme: fileTheme }))

    render(<Host />)
    await settle()

    expect(listeners.size).toBe(0)
    expect(invokeMock).not.toHaveBeenCalled()

    // 이 틈에 밖의 에이전트가 파일을 고치고 ui.refresh 를 돌렸다 — 리스너가 없어 알림은 이 창에 안 닿는다.
    fileTheme = 'e-ink'

    await act(async () => {
      gate.open()
      await new Promise(resolve => setTimeout(resolve, 0))
    })

    // 놓친 알림 대신, **그 뒤에** 나간 조회가 새 값을 집어 왔다.
    expect(invokeMock).toHaveBeenCalledTimes(1)
    expect(themeAttr()).toBe('e-ink')
  })

  // ★한 번 거절당하고 끝나면 그 창은 영구히 ui.refresh 를 못 받는다★ — 그런데 부팅 값은 멀쩡히 칠해져
  //   화면은 건강해 보인다(무신호 불일치). 그래서 등록에 유계 재시도를 건다.
  it('구독 등록이 한 번 거절당해도 재시도해서 결국 붙는다', async () => {
    listenMock.mockImplementationOnce(async () => {
      throw new Error('이벤트 평면 미준비')
    })
    invokeMock.mockImplementation(async () => ({ theme: 'light' }))
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})

    render(<Host />)
    await act(async () => {
      await new Promise(resolve => setTimeout(resolve, 400))
    })

    expect(listenMock.mock.calls.length).toBeGreaterThan(1)
    expect(warn).toHaveBeenCalled()
    expect(themeAttr()).toBe('light')

    // ★재시도가 실제로 붙었나★ — 붙었으면 이후 알림이 이 창에 닿는다.
    push('e-ink')
    expect(themeAttr()).toBe('e-ink')
  })

  // 재시도까지 소진되면 조용히 넘기지 않는다 — 그 뒤로는 갱신을 못 받는 것이 확정이라 warn 이 아니라 error.
  it('재시도를 다 써도 실패하면 error 로 표면화하고 부팅 조회는 그대로 나간다', async () => {
    listenMock.mockImplementation(async () => {
      throw new Error('이벤트 평면 미준비')
    })
    invokeMock.mockImplementation(async () => ({ theme: 'light' }))
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})

    render(<Host />)
    // 기본 재시도 프로필(4회 · 150ms 배수 2) 총 대기 ~1.05s — 그것보다 넉넉히 기다린다.
    await act(async () => {
      await new Promise(resolve => setTimeout(resolve, 1500))
    })

    expect(error).toHaveBeenCalled()
    expect(themeAttr()).toBe('light')
  })
})

describe('installUiSettings — 수명', () => {
  it('언마운트하면 구독을 푼다(HMR·팝아웃 재부착에서 중복 누적 금지)', async () => {
    const { unmount } = render(<Host />)
    await settle()
    expect(listenMock).toHaveBeenCalledTimes(1)

    unmount()

    expect(unlistenMock).toHaveBeenCalledTimes(1)
  })

  it('등록이 끝나기 전에 정리되면 받은 자리에서 바로 푼다', async () => {
    const dispose = installUiSettings()
    dispose()
    await settle()

    expect(unlistenMock).toHaveBeenCalledTimes(1)
  })

  // ★`off()` 만으로는 한 건이 샌다★: dispose 가 `listen()` 대기 중에 돌면 등록은 이미 끝났는데 해제는 아직
  //   전이라, 그 틈에 배달된 알림을 죽은 인스턴스가 그린다(StrictMode 첫 회 · HMR 이전 판이 살아 있는 창의
  //   테마를 덮는다). 여기서는 해제된 뒤에 콜백을 직접 때려 그 배달을 흉내낸다.
  it('정리된 뒤에 배달된 알림은 화면을 건드리지 않는다', async () => {
    invokeMock.mockImplementation(async () => ({ theme: 'light' }))
    const dispose = installUiSettings()
    await settle()
    expect(themeAttr()).toBe('light')

    dispose()

    const handler = listeners.get(EVT)
    if (!handler) throw new Error('리스너가 등록되지 않았다')
    act(() => {
      handler({ payload: { theme: 'e-ink' } })
    })

    expect(themeAttr()).toBe('light')
  })
})
