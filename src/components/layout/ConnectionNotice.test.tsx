// ConnectionNotice — 연결 실패 이유 표시(ADR-0134 결정 4).
//
// 이 스위트가 지키는 핵심은 "사용자가 원인을 못 보는 상태로 고착되지 않는다" 하나다.

import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

// agentClient 싱글톤을 최소 표면으로 대역한다 — 실제 transport/IPC 없이 구독 통지만 재현한다.
// ★vi.hoisted 필수★: vi.mock 팩토리는 파일 최상단으로 끌어올려져 일반 top-level 변수를 못 본다.
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

// 구독 통지는 React 밖에서 오는 이벤트라 act 로 감싸야 상태 반영이 flush 된다(운영에서도 Tauri
// 이벤트 콜백이 같은 모양이다).
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

  it('닫으면 사라진다', () => {
    render(<ConnectionNotice />)
    emit('폴더 문제')
    fireEvent.click(screen.getByLabelText('알림 닫기'))
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('닫은 뒤 다른 이유가 오면 다시 뜬다', () => {
    render(<ConnectionNotice />)
    emit('첫 번째 이유')
    fireEvent.click(screen.getByLabelText('알림 닫기'))
    emit('두 번째 이유')
    expect(screen.getByRole('alert').textContent).toContain('두 번째 이유')
  })

  // ★F3 회귀 방지★: 닫힘 기억을 이유 문자열로만 들고 있으면, 고쳐서 연결된 뒤 **똑같은 문구**로
  //   재발했을 때 아무것도 안 뜬다. 같은 원인의 재발이 오히려 흔한 경우다.
  it('닫고 → 연결 성공으로 이유가 지워진 뒤 → 같은 이유가 재발하면 다시 뜬다', () => {
    render(<ConnectionNotice />)
    const same = '데이터 폴더에 쓸 수 없음(C:\\x) — 쓰기 가능한 위치에 압축을 풀어 주세요'

    emit(same)
    fireEvent.click(screen.getByLabelText('알림 닫기'))
    expect(screen.queryByRole('alert')).toBeNull()

    emit(null) // 사용자가 폴더를 고쳐 연결 성공 → 이유 해제
    expect(screen.queryByRole('alert')).toBeNull()

    emit(same) // 나중에 같은 실패가 재발
    expect(screen.getByRole('alert')?.textContent).toContain('쓰기 가능한 위치')
  })
})
