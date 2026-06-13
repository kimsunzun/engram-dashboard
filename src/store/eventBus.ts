// 앱 전역 Tauri 이벤트 리스너 — 앱 시작 시 1회 호출(App.tsx).
// HMR 재평가 시 기존 리스너 해제 후 재등록(중복 누적 방지).

import { listen } from '@tauri-apps/api/event'

import type { AgentInfo, AgentStatus, RestoreReport } from '../api/types'
import { useAgentStore } from './agentStore'
import { useSlotStore } from './slotStore'

let unlistenFns: (() => void)[] = []
// StrictMode 이중마운트 레이스 방지 — 진행 중인 promise가 있으면 재사용
let initPromise: Promise<void> | null = null

export function initEventBus(): Promise<void> {
  if (initPromise) return initPromise

  initPromise = (async () => {
    try {
      // §5: 레이아웃 제어 표면(dispatch)을 window에 노출 → LLM(cdp eval 등)이 사람 UI와
      // 동일한 단일 진입점을 호출할 수 있다. 정식 control surface 전까지의 임시 경로.
      ;(globalThis as Record<string, unknown>).__engramLayout = {
        dispatch: useSlotStore.getState().dispatch,
      }

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
        await listen<{ id: string; status: AgentStatus; epoch: number }>(
          'agent-status-changed',
          e => {
            useAgentStore.getState().onStatusChanged(e.payload.id, e.payload.status)
          },
        ),
      )

      // agent-restore-result: 부팅 복원 결과(S9). 현재는 로그만 — UX 배너는 추후.
      unlistenFns.push(
        await listen<RestoreReport>('agent-restore-result', e => {
          const { agent_id, outcome } = e.payload
          console.info('[restore]', agent_id, outcome.type, outcome)
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
