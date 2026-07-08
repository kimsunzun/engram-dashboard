// PopoutPage 자가종료(self-close) 단위테스트 — Finding 1 재작업 회귀 안전망.
//
// ★이 스위트가 막는 것★: 팝업 창은 자기 백킹 View 가 *실제로 닫혔을 때만*(view:closed 의 id 가 자기
// ?view= id 와 정확히 일치) 창을 자가종료해야 한다. 옛 버그(view:list-updated 목록에서 자기 view 가
// 빠지면 닫기)는 view_metas 필터가 팝업 view 를 항상 제외하므로 모든 팝업이 첫 emit 에 자가종료·연쇄
// 붕괴했다. 그래서 여기서 검증하는 불변식:
//   1. view:closed{id: 자기 view} → getCurrentWindow().close() 호출(자가종료).
//   2. view:closed{id: 다른 view} → close() 안 부름(무관 view 는 무시).
//   3. view:list-updated 는 자가종료 트리거가 아니다 — 어떤 payload 든 close() 안 부름.

import { cleanup, render, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// ── listen mock: 이벤트명별 핸들러를 보관해 테스트가 직접 emit 을 흉내낸다 ──
const listeners = new Map<string, (e: { payload: unknown }) => void>()
const unlistenMock = vi.fn()
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, handler: (e: { payload: unknown }) => void) => {
    listeners.set(event, handler)
    return unlistenMock
  }),
}))

// ── invoke mock: get_view(초기 pull) 응답 ──
const invokeMock = vi.fn(async (_cmd: string, ..._rest: unknown[]) => undefined as unknown)
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, ...rest: unknown[]) => invokeMock(cmd, ...rest),
  Channel: class {
    onmessage: unknown = null
  },
}))

// ── getCurrentWindow().close() mock — 자가종료 여부만 관측 ──
const closeMock = vi.fn(async () => undefined)
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ close: closeMock }),
}))

// ── ViewLayoutRenderer stub — 렌더 자체는 이 테스트 관심 밖(자가종료 로직만 본다) ──
vi.mock('../components/layout/ViewLayoutRenderer', () => ({
  default: () => <div data-testid="view-layout-renderer" />,
}))

import PopoutPage from './PopoutPage'
import type { ViewSnapshot } from '../api/layoutTypes'

const OWN_VIEW = 'popup-own-view-uuid'

function slotSnap(viewId: string, version: number): ViewSnapshot {
  return {
    view_id: viewId,
    layout: { type: 'slot', id: 's1', agent_id: null },
    focused_slot_id: 's1',
    version,
  }
}

/** listen 이 등록한 핸들러로 payload 를 흘려보낸다(백엔드 emit 흉내). */
function emit(event: string, payload: unknown): void {
  const h = listeners.get(event)
  if (!h) throw new Error(`no listener for ${event} — 구독 등록 전인가?`)
  h({ payload })
}

const origHash = window.location.hash

beforeEach(() => {
  listeners.clear()
  unlistenMock.mockClear()
  closeMock.mockClear()
  invokeMock.mockReset()
  // 초기 pull(get_view)은 자기 view 스냅샷을 준다.
  invokeMock.mockImplementation(async (cmd: string) => {
    if (cmd === 'get_view') return slotSnap(OWN_VIEW, 1)
    return undefined
  })
  // 팝업 컨텍스트 hash 설정 — readViewIdFromHash 가 OWN_VIEW 를 파싱하게.
  window.location.hash = `#/popup?view=${OWN_VIEW}`
})

afterEach(() => {
  cleanup()
  window.location.hash = origHash
})

describe('PopoutPage 자가종료(Finding 1: view:closed 양성 신호)', () => {
  it('view:closed{id: 자기 view} → 창을 자가종료한다(close 호출)', async () => {
    render(<PopoutPage />)
    // 구독 등록 완료(listen 은 async)까지 대기.
    await waitFor(() => expect(listeners.has('view:closed')).toBe(true))
    emit('view:closed', { id: OWN_VIEW })
    await waitFor(() => expect(closeMock).toHaveBeenCalledTimes(1))
  })

  it('view:closed{id: 다른 view} → 자가종료하지 않는다(close 미호출)', async () => {
    render(<PopoutPage />)
    await waitFor(() => expect(listeners.has('view:closed')).toBe(true))
    emit('view:closed', { id: 'some-other-view' })
    // 마이크로태스크 flush 후에도 close 는 안 불려야 한다.
    await Promise.resolve()
    expect(closeMock).not.toHaveBeenCalled()
  })

  it('view:list-updated 는 자가종료 트리거가 아니다 — 자기 view 가 목록에 없어도 close 안 부름(옛 버그 회귀 안전망)', async () => {
    render(<PopoutPage />)
    await waitFor(() => expect(listeners.has('view:closed')).toBe(true))
    // 팝업은 view:list-updated 를 아예 구독하지 않는다 — 그래서 listeners 에 없어야 한다(옛 트리거 제거 확인).
    expect(listeners.has('view:list-updated')).toBe(false)
    expect(closeMock).not.toHaveBeenCalled()
  })
})
