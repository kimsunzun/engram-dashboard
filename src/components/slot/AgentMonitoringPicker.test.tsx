// ADR-0237: the agent monitoring popup's backdrop carries an overlay marker only while open — that Esc is the key that closes the popup, so
//   the chat area behind it must not interrupt the turn.

import { act, cleanup, fireEvent, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// Assignment (on-select) is not this test's concern — blocked so no shell invoke gets in.
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
