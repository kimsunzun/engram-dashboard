// TreePage — 트리 창. 연결 띠는 목록을 밀지 않고 덮는다(ADR-0277).
//
// jsdom 은 크기를 재지 못한다 — 「밀지 않는다」를 계산된 위치와 구조로 단언한다.

import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

// 목록 내부는 AgentList.test 몫이라 sentinel 로 stub.
vi.mock('../components/agent/AgentList', () => ({
  default: () => <div data-testid="agent-list" />,
}))

// 연결 띠를 띄우려고 연결 상태 표면만 대역한다(ConnectionNotice.test 와 같은 모양).
const { connection } = vi.hoisted(() => {
  const listeners = new Set<() => void>()
  const connection = {
    listeners,
    connectionError: null as string | null,
    onConnectionStateChange(cb: () => void) {
      listeners.add(cb)
      cb()
      return () => {
        listeners.delete(cb)
      }
    },
  }
  return { connection }
})
vi.mock('../api/clientFactory', async importOriginal => ({
  ...(await importOriginal<typeof import('../api/clientFactory')>()),
  agentClient: connection,
}))

import TreePage from './TreePage'

function mount(connectionError: string | null) {
  connection.connectionError = connectionError
  const r = render(<TreePage />)
  const root = r.container.firstElementChild as HTMLElement
  const list = screen.getByTestId('agent-list')
  return {
    root,
    list,
    box: list.parentElement as HTMLElement,
    overlay: screen.getByTestId('notice-overlay'),
  }
}

afterEach(() => {
  cleanup()
  connection.listeners.clear()
  connection.connectionError = null
})

describe('TreePage — 연결 띠는 목록을 덮는다', () => {
  it('연결 띠는 덮는 층 안이고, 층의 계산된 위치는 absolute 로 목록 칸의 맨 위에 붙는다', () => {
    const { box, overlay, list } = mount('폴더 문제')
    const alert = screen.getByRole('alert')
    expect(overlay.contains(alert)).toBe(true)
    const style = getComputedStyle(overlay)
    expect(style.position).toBe('absolute')
    expect([style.top, style.left, style.right]).toEqual(['0px', '0px', '0px'])
    expect(overlay.parentElement).toBe(box)
    expect(getComputedStyle(box).position).toBe('relative')
    expect(overlay.contains(list)).toBe(false)
  })

  it('연결 띠가 떠도 흐름은 머리줄 → 목록 칸 둘뿐이고, 목록 칸은 세로 스택의 나머지를 다 쓴다', () => {
    const { root, box, overlay, list } = mount('폴더 문제')
    expect(screen.getByRole('alert')).toBeTruthy()
    expect([root.style.width, root.style.height]).toEqual(['100vw', '100vh'])
    expect(root.style.display).toBe('flex')
    expect(root.style.flexDirection).toBe('column')

    const [header, second] = [...root.children]
    expect(root.childElementCount).toBe(2)
    expect(header?.textContent).toBe('Agent Tree')
    expect((header as HTMLElement).style.flexShrink).toBe('0')
    expect(second).toBe(box)

    expect(box.style.flexGrow).toBe('1')
    expect(box.style.minHeight).toBe('0px')
    expect(box.style.display).toBe('flex')
    expect(box.style.flexDirection).toBe('column')
    expect([...box.children]).toEqual([overlay, list])
  })

  it('이유가 없으면 층은 비어 있다', () => {
    mount(null)
    expect(screen.queryByRole('alert')).toBeNull()
    expect(screen.getByTestId('notice-stack').childElementCount).toBe(0)
  })

  it('층 안에서도 ✕ 로 닫힌다', () => {
    mount('폴더 문제')
    fireEvent.click(screen.getByLabelText('알림 닫기'))
    expect(screen.queryByRole('alert')).toBeNull()
  })
})
