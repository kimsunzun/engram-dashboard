// 앱 전역 Tauri 이벤트 리스너 — 앱 시작 시 1회 호출(App.tsx).
// HMR 재평가 시 기존 리스너 해제 후 재등록(중복 누적 방지).

import { listen } from '@tauri-apps/api/event'

import type { AgentInfo, AgentStatus } from '../api/types'
import { useAgentStore } from './agentStore'

let unlistenFns: (() => void)[] = []
// StrictMode 이중마운트 레이스 방지 — 진행 중인 promise가 있으면 재사용
let initPromise: Promise<void> | null = null

export function initEventBus(): Promise<void> {
  if (initPromise) return initPromise

  initPromise = (async () => {
    try {
      // HMR 재평가 시 기존 리스너 먼저 해제
      if (unlistenFns.length > 0) {
        unlistenFns.forEach(fn => fn())
        unlistenFns = []
      }

      // agent-list-updated: 권위 있는 목록 교체(존재/제거 판정 기준, T-4)
      unlistenFns.push(
        await listen<AgentInfo[]>('agent-list-updated', e => {
          useAgentStore.getState().setAgents(e.payload)
        }),
      )

      // agent-status-changed: 개별 status 갱신(뱃지 표시용, 목록 제거 안 함, T-4)
      unlistenFns.push(
        await listen<{ id: string; status: AgentStatus }>('agent-status-changed', e => {
          useAgentStore.getState().onStatusChanged(e.payload.id, e.payload.status)
        }),
      )

      // Vite HMR 모듈 교체 시 리스너 해제 + promise 초기화
      if (import.meta.hot) {
        import.meta.hot.dispose(() => {
          unlistenFns.forEach(fn => fn())
          unlistenFns = []
          initPromise = null
        })
      }
    } catch (err) {
      console.error('[eventBus] init failed:', err)
      initPromise = null // 고착 방지 — 다음 호출 시 재시도 허용
      throw err
    }
  })()

  return initPromise
}
