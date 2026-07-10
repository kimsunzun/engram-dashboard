// AppLayout 셸 렌더 단위테스트 — ADR-0063 고정 크롬 제거 회귀 안전망.
//
// ★이 스위트가 막는 것★: AppLayout 이 이제 main 창의 WindowLayout 만 감싸는 얇은 셸인지 —
//   옛 고정 Sidebar / 하단 더미 DiffPanel·StatusBar 가 되살아나지 않는지(삭제 파일 dangling import 도
//   포함 — 셸이 그것들을 import 하면 이 테스트 파일이 아예 로드 실패한다). WindowLayout 은 sentinel 로 stub.

import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

// WindowLayout stub — 자기 label 로 마운트되는지만 확인(실제 탭/캔버스 배선은 WindowLayout.test 담당).
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
    // Sidebar 헤더 라벨·StatusBar 문구·DiffPanel 버튼이 전부 사라졌다.
    expect(screen.queryByText('Agent Tree')).toBeNull() // 옛 Sidebar 헤더
    expect(screen.queryByText('Ready')).toBeNull() // 옛 StatusBar
    expect(screen.queryByText('Accept')).toBeNull() // 옛 DiffPanel
    // 사이드바 재열기 토글 버튼(▶)도 없다.
    expect(screen.queryByText('▶')).toBeNull()
  })
})
