// 삭제된 고정 크롬 파일의 dangling import 도 이 스위트가 잡는다 — 셸이 그것들을 import 하면
// 단언 이전에 이 테스트 파일이 로드 실패한다.

import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

// 실제 탭/캔버스 배선 커버리지는 WindowLayout.test 담당이라 여기선 sentinel 로 stub.
vi.mock('./WindowLayout', () => ({
  default: ({ label }: { label: string }) => <div data-testid="window-layout" data-label={label} />,
}))

import AppLayout from './AppLayout'
import { MAIN_WINDOW_LABEL } from '../../store/viewStore'

afterEach(cleanup)

describe('AppLayout — 슬롯화된 셸(ADR-0063)', () => {
  it('main 창 WindowLayout 을 label="main" 으로 마운트한다', () => {
    render(<AppLayout />)
    const wl = screen.getByTestId('window-layout')
    expect(wl).toBeTruthy()
    expect(wl.getAttribute('data-label')).toBe(MAIN_WINDOW_LABEL)
  })

  it('옛 고정 크롬(Sidebar/DiffPanel/StatusBar) 잔재가 없다', () => {
    render(<AppLayout />)
    expect(screen.queryByText('Agent Tree')).toBeNull() // 옛 Sidebar 헤더
    expect(screen.queryByText('Ready')).toBeNull() // 옛 StatusBar
    expect(screen.queryByText('Accept')).toBeNull() // 옛 DiffPanel
    expect(screen.queryByText('▶')).toBeNull() // 옛 사이드바 재열기 토글
  })
})
