// ADR-0237: 에이전트 모니터링 팝업의 백드롭은 열린 동안만 오버레이 표지를 단다 — 그 Esc 는 팝업을 닫는 키라
//   뒤의 채팅 칸이 턴을 끊으면 안 된다.

import { act, cleanup, fireEvent, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// 배정(on-select)은 이 시험의 관심사가 아니다 — 셸 invoke 를 들이지 않게 막아 둔다.
vi.mock('../../store/viewStore', () => ({
  useViewStore: { getState: () => ({ assignAgent: vi.fn(async () => undefined) }) },
}))

import AgentMonitoringPicker from './AgentMonitoringPicker'
import { OVERLAY_SELECTOR } from './interruptKey'
import { useMonitoringPickerStore } from '../../store/monitoringPickerStore'
import { useAgentStore } from '../../store/agentStore'

const backdrop = (): Element | null => document.querySelector('[data-monitoring-picker-backdrop="1"]')

beforeEach(() => {
  useAgentStore.setState({ agents: [], profiles: [] })
  useMonitoringPickerStore.setState({ target: null })
})
afterEach(() => {
  cleanup()
  useMonitoringPickerStore.setState({ target: null })
})

describe('AgentMonitoringPicker — 오버레이 표지(ADR-0237)', () => {
  it('닫혀 있으면 표지가 없고, 열면 백드롭이 표지를 단다', () => {
    render(<AgentMonitoringPicker />)
    expect(document.querySelector(OVERLAY_SELECTOR)).toBeNull()
    act(() => useMonitoringPickerStore.getState().open('v1', 's1'))
    expect(backdrop()).not.toBeNull()
    expect(document.querySelector(OVERLAY_SELECTOR)).toBe(backdrop())
  })

  it('문서 Esc 로 닫히면 표지도 함께 사라진다', () => {
    render(<AgentMonitoringPicker />)
    act(() => useMonitoringPickerStore.getState().open('v1', 's1'))
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(useMonitoringPickerStore.getState().target).toBeNull()
    expect(document.querySelector(OVERLAY_SELECTOR)).toBeNull()
  })

  it('백드롭을 눌러 닫혀도 표지가 사라진다', () => {
    render(<AgentMonitoringPicker />)
    act(() => useMonitoringPickerStore.getState().open('v1', 's1'))
    fireEvent.click(backdrop()!)
    expect(document.querySelector(OVERLAY_SELECTOR)).toBeNull()
  })
})
