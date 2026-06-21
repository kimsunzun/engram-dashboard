// 앱 전역 에이전트 이벤트 배선 — 앱 시작 시 1회 호출(App.tsx).
// HMR 재평가 시 기존 구독 해제 후 재등록(중복 누적 방지).
//
// Tauri 이벤트를 직접 듣지 않고 agentClient(Embedded/Daemon 싱글톤)의 이벤트 구독 메서드를
// 소비한다 — 두 모드 공통 표면이라 데몬 모드(WS 이벤트)에서도 동일하게 트리·상태바가 갱신된다.

import { agentClient } from '../api/clientFactory'
import { useAgentStore } from './agentStore'
import { useSlotStore } from './slotStore'

let unlistenFns: (() => void)[] = []
// StrictMode 이중마운트 레이스 방지 — 진행 중인 promise가 있으면 재사용
let initPromise: Promise<void> | null = null

/**
 * 프로필 목록 갱신(ADR-0018). 백엔드에 프로필 변경 이벤트가 없으므로,
 * 부팅 1회 + create/delete/activate(spawnProfile) 직후 명시적으로 호출해 store 를 동기화한다.
 * (실행중 전환 자체는 기존 agent-list-updated 가 처리 — 여기선 예약 목록만 새로 받는다.)
 */
export async function refreshProfiles(): Promise<void> {
  try {
    const profiles = await agentClient.listProfiles()
    useAgentStore.getState().setProfiles(profiles)
  } catch (err) {
    console.warn('[eventBus] refreshProfiles failed:', err)
  }
}

/**
 * 재연결 직후 목록/프로필 재동기화(Q2). connected *재*전이에서만 호출(첫 연결 제외 — initEventBus
 * 의 lastState 가드). 권위 목록을 다시 끌어와 store 를 새로 쓴다 → 끊긴 동안 변경(spawn/kill/프로필)
 * 반영. 출력 resubscribe(ProtocolClient.resubscribeAll)는 건드리지 않는다(이미 자동 동작).
 */
async function resyncAfterReconnect(): Promise<void> {
  try {
    const agents = await agentClient.getAgents()
    useAgentStore.getState().setAgents(agents)
  } catch (err) {
    console.warn('[eventBus] resync getAgents failed:', err)
  }
  await refreshProfiles()
}

export function initEventBus(): Promise<void> {
  if (initPromise) return initPromise

  initPromise = (async () => {
    try {
      // §5: 레이아웃 제어 표면(dispatch)을 window에 노출 → LLM(cdp eval 등)이 사람 UI와
      // 동일한 단일 진입점을 호출할 수 있다. 정식 control surface 전까지의 임시 경로.
      ;(globalThis as Record<string, unknown>).__engramLayout = {
        dispatch: useSlotStore.getState().dispatch,
      }

      // HMR 재평가 시 기존 구독 먼저 해제
      if (unlistenFns.length > 0) {
        unlistenFns.forEach(fn => fn())
        unlistenFns = []
      }

      // 등록은 sync(disposer 즉시 반환) — await 불필요. agentClient 가 모드별 transport 를 숨긴다.

      // 권위 있는 목록 교체(존재/제거 판정 기준, T-4)
      unlistenFns.push(
        agentClient.onAgentListUpdated(agents => {
          useAgentStore.getState().setAgents(agents)
        }),
      )

      // 개별 status 갱신(뱃지 표시용, 목록 제거 안 함, T-4)
      unlistenFns.push(
        agentClient.onStatusChanged((id, status) => {
          useAgentStore.getState().onStatusChanged(id, status)
        }),
      )

      // 부팅 복원 결과(S9). 현재는 로그만 — UX 배너는 추후.
      unlistenFns.push(
        agentClient.onRestoreResult(report => {
          console.info('[restore]', report.agent_id, report.outcome.type, report.outcome)
        }),
      )

      // 프로필 목록 라이브 갱신(깡통/예약, ADR-0018 후속). daemon 모드는 ProfileListUpdated
      // broadcast 로 동작, embedded 는 후속 backend broadcast 흡수 자리(현재 미도달). store 미러 교체.
      unlistenFns.push(
        agentClient.onProfileListUpdated(profiles => {
          useAgentStore.getState().setProfiles(profiles)
        }),
      )

      // 재연결 시 목록/프로필 재동기화(Q2). 출력 스트림은 ProtocolClient.resubscribeAll 로 자동
      // 복구되나, 에이전트 트리·프로필 목록은 재동기화 트리거가 없어 stale 이 된다(끊긴 동안의
      // spawn/kill/프로필 변경 broadcast 를 놓침). connected 로 *재*전이할 때만 권위 목록을 다시
      // 끌어와 store 를 새로 쓴다. ★첫 connected 는 스킵★ — App.tsx 부팅 로드(getAgents/
      // refreshProfiles 1회)와 중복 방지. lastState 가드는 ProtocolClient.lastState 패턴과 동일
      // (prev!=='connected' && cur==='connected'), 초기값은 현재 상태로 둬 첫 통지가 connected 여도
      // 재전이로 오인하지 않는다.
      let lastConn = agentClient.connectionState
      unlistenFns.push(
        agentClient.onConnectionStateChange(state => {
          const prev = lastConn
          lastConn = state
          if (state === 'connected' && prev !== 'connected') {
            void resyncAfterReconnect()
          }
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
