// ConnectionNotice — display of the connection failure reason (ADR-0134 decision 4).
//
// What this suite guards is just one thing: "the user is never stuck in a state where they cannot see the cause".

import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

// Stand in for the agentClient singleton with a minimal surface — reproduce only the subscription notification, with no real transport/IPC.
// ★vi.hoisted required★: the vi.mock factory is hoisted to the top of the file and cannot see ordinary top-level variables.
const { listeners, fake } = vi.hoisted(() => {
  const listeners = new Set<() => void>()
  const fake = {
    connectionError: null as string | null,
    onConnectionStateChange(cb: () => void) {
      listeners.add(cb)
      cb()
      return () => {
        listeners.delete(cb)
      }
    },
  }
  return { listeners, fake }
})

// Subscription notifications are events coming from outside React, so they must be wrapped in act for the state update to flush (in production too the Tauri
// event callback has the same shape).
function emit(next: string | null) {
  act(() => {
    fake.connectionError = next
    for (const cb of [...listeners]) cb()
  })
}

vi.mock('../../api/clientFactory', () => ({
  agentClient: fake,
}))

import ConnectionNotice from './ConnectionNotice'

afterEach(() => {
  cleanup()
  listeners.clear()
  fake.connectionError = null
})

describe('ConnectionNotice', () => {
  it('이유가 없으면 아무것도 그리지 않는다', () => {
    render(<ConnectionNotice />)
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('이유가 도착하면 그 문장을 보여준다', () => {
    render(<ConnectionNotice />)
    emit('데이터 폴더에 쓸 수 없음(C:\\x) — 쓰기 가능한 위치에 압축을 풀어 주세요')
    const alert = screen.getByRole('alert')
    expect(alert.textContent).toContain('쓰기 가능한 위치')
  })

  // The row contract of the covering layer (the `NoticeOverlay` doc) — if `wrap-anywhere` is lost, a long reason pushes the ✕ off screen.
  it('줄은 불투명 바탕이고 글 칸은 아무 데서나 줄을 바꾼다', () => {
    render(<ConnectionNotice />)
    emit('폴더 문제')
    const alert = screen.getByRole('alert')
    expect(alert.className).toMatch(/\bbg-elevated\b/)
    expect(alert.firstElementChild?.className).toMatch(/\bwrap-anywhere\b/)
  })

  it('닫으면 사라진다', () => {
    render(<ConnectionNotice />)
    emit('폴더 문제')
    fireEvent.click(screen.getByLabelText('알림 닫기'))
    expect(screen.queryByRole('alert')).toBeNull()
  })

  // The band sits on the covering layer — if the next notification of the same reason revives the band, the ✕ is powerless and the covered area is blocked again.
  it('닫은 뒤 같은 이유로 다시 통지돼도 뜨지 않는다', () => {
    render(<ConnectionNotice />)
    emit('폴더 문제')
    fireEvent.click(screen.getByLabelText('알림 닫기'))
    emit('폴더 문제')
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('닫은 뒤 다른 이유가 오면 다시 뜬다', () => {
    render(<ConnectionNotice />)
    emit('첫 번째 이유')
    fireEvent.click(screen.getByLabelText('알림 닫기'))
    emit('두 번째 이유')
    expect(screen.getByRole('alert').textContent).toContain('두 번째 이유')
  })

  // ★F3 regression guard★: if the closed memory is held only as the reason string, then when it recurs with **the identical wording** after being fixed and
  //   connected, nothing shows. A recurrence of the same cause is in fact the common case.
  it('닫고 → 연결 성공으로 이유가 지워진 뒤 → 같은 이유가 재발하면 다시 뜬다', () => {
    render(<ConnectionNotice />)
    const same = '데이터 폴더에 쓸 수 없음(C:\\x) — 쓰기 가능한 위치에 압축을 풀어 주세요'

    emit(same)
    fireEvent.click(screen.getByLabelText('알림 닫기'))
    expect(screen.queryByRole('alert')).toBeNull()

    emit(null) // The user fixes the folder and the connection succeeds → the reason is cleared
    expect(screen.queryByRole('alert')).toBeNull()

    emit(same) // Later the same failure recurs
    expect(screen.getByRole('alert')?.textContent).toContain('쓰기 가능한 위치')
  })
})
