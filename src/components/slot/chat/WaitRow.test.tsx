// 대기 표시 줄 시험 — 늘 서 있는 고정 높이 줄 · 표시는 streaming 동안만 · 경과 초의 정체성(사용자 결정 2026-10-01 ·
//   ADR-0244 「중단하는 중」).

import { act, cleanup, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { t } from '../../../i18n'
import { WaitStrip } from './WaitRow'

afterEach(() => {
  cleanup()
  vi.useRealTimers()
})

const strip = (c: HTMLElement) => c.querySelector('[data-wait-strip="1"]') as HTMLElement

describe('WaitStrip — 자리', () => {
  it('streaming 이 아니어도 줄은 서 있고 비어 있다 — 높이는 변수 하나로 고정', () => {
    const { container, rerender } = render(<WaitStrip streaming={false} interrupting={false} />)
    const node = strip(container)
    expect(node).not.toBeNull()
    expect(node.childElementCount).toBe(0)
    expect(node.style.height).toBe('var(--chat-wait-strip-h)')

    rerender(<WaitStrip streaming interrupting={false} />)
    expect(strip(container)).toBe(node)
    expect(screen.getByText('Wait')).toBeTruthy()
    expect(node.style.height).toBe('var(--chat-wait-strip-h)')

    rerender(<WaitStrip streaming={false} interrupting={false} />)
    expect(strip(container)).toBe(node)
    expect(node.childElementCount).toBe(0)
  })

  it('라벨은 오른쪽 칸 — 대기 글 뒤에 서고, 대기 표시가 켜지고 꺼져도 같은 노드다', () => {
    const label = <span data-test-label="1">agent</span>
    const { container, rerender } = render(<WaitStrip streaming={false} interrupting={false} label={label} />)
    const node = container.querySelector('[data-test-label="1"]') as HTMLElement
    expect(strip(container).contains(node)).toBe(true)

    rerender(<WaitStrip streaming interrupting={false} label={label} />)
    expect(container.querySelector('[data-test-label="1"]')).toBe(node)
    expect(screen.getByText('Wait').compareDocumentPosition(node) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()

    rerender(<WaitStrip streaming={false} interrupting={false} label={label} />)
    expect(container.querySelector('[data-test-label="1"]')).toBe(node)
  })
})

// ── ADR-0244: 끊기를 보낸 뒤 「중단하는 중」 ──────────────────────────────────────
describe('WaitStrip — 중단하는 중(ADR-0244)', () => {
  it('interrupting 이면 Wait 대신 CircleStop + 「중단하는 중…」 을 그린다', () => {
    const { container } = render(<WaitStrip streaming interrupting />)
    const row = container.querySelector('[data-wait-interrupting="1"]') as HTMLElement
    expect(row).not.toBeNull()
    expect(row.textContent).toBe(t('chat.interrupting'))
    expect(row.querySelector('svg')?.getAttribute('class')).toContain('lucide-circle-stop')
    expect(screen.queryByText('Wait')).toBeNull()
  })

  it('streaming 이 아니면 interrupting 이어도 아무것도 그리지 않는다', () => {
    const { container } = render(<WaitStrip streaming={false} interrupting />)
    expect(container.querySelector('[data-wait-interrupting="1"]')).toBeNull()
    expect(screen.queryByText(t('chat.interrupting'))).toBeNull()
  })
})

describe('WaitStrip — 경과 초', () => {
  it('중단하는 중을 오가도 다시 마운트되지 않는다 — 경과 초가 이어진다', () => {
    vi.useFakeTimers()
    const { rerender } = render(<WaitStrip streaming interrupting={false} />)
    act(() => vi.advanceTimersByTime(3000))
    expect(screen.getByText('3s')).toBeTruthy()
    rerender(<WaitStrip streaming interrupting />)
    expect(screen.queryByText('Wait')).toBeNull()
    act(() => vi.advanceTimersByTime(2000))
    rerender(<WaitStrip streaming interrupting={false} />)
    expect(screen.getByText('Wait')).toBeTruthy()
    expect(screen.getByText('5s')).toBeTruthy()
  })

  it('streaming 이 꺼졌다 다시 켜지면(새 턴) 0 부터 센다', () => {
    vi.useFakeTimers()
    const { rerender } = render(<WaitStrip streaming interrupting={false} />)
    act(() => vi.advanceTimersByTime(4000))
    expect(screen.getByText('4s')).toBeTruthy()
    rerender(<WaitStrip streaming={false} interrupting={false} />)
    act(() => vi.advanceTimersByTime(1000))
    rerender(<WaitStrip streaming interrupting={false} />)
    expect(screen.getByText('0s')).toBeTruthy()
    act(() => vi.advanceTimersByTime(1000))
    expect(screen.getByText('1s')).toBeTruthy()
  })
})
