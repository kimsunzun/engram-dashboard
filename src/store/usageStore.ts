// 사용량 슬롯의 값 미러(TRD S21 usage-limit-slot §1-8) — 회사별 마지막 스냅숏과, 그것을 받아들인 규칙의 기억.
//
// ★받기는 `merge` 한 곳뿐이다★ — 셸 방송(eventBus 가 잇는다)과 셸 캐시 pull 이 같은 규칙으로 들어온다. 두 길은
//   서로 순서를 보장하지 않으므로 늦게 닿은 옛 사본을 여기서 가른다. ⟳ 답(`Ack`)은 값을 싣지 않아 대기만 푼다.
// ★관심(어느 회사를 보고 싶은가)은 여기 없다★ — 셸이 레이아웃에서 계산한다. 그래서 켜지 않은 회사의 방송도 여기
//   든다 — 거르는 것은 그리는 쪽이다.
// 스토어는 `agentClient` 만 부른다(ADR-0011) — 셸 커맨드인 pull 도 transport seam 뒤에 있다(ADR-0036).

import { create } from 'zustand'

import type { UsageLimitSnapshot } from '../../crates/engram-dashboard-protocol/bindings/UsageLimitSnapshot'
import type { UsageVendorState } from '../../crates/engram-dashboard-protocol/bindings/UsageVendorState'
import { agentClient } from '../api/clientFactory'
import type { AgentBackendKind } from '../api/types'
import { retryAsync } from '../util/retryInvoke'

/**
 * 한 회사의 받아 둔 값. 키는 회사 하나다 — 스냅숏의 `account_key` 는 지금 늘 `"default"` 라 가르지 않는다(계정이
 * 늘면 revision 이 회사·계정 칸마다라 키도 넓혀야 한다).
 */
export interface UsageVendorEntry {
  snapshot: UsageLimitSnapshot
  /** 받아들인 순간의 `performance.now()`(ms). 스냅숏의 상대 시간 칸(`age_secs` 등)은 이 순간 기준이다. */
  receivedAt: number
  /**
   * 지금 소켓에서 받아들인 최고 revision. `null` = 잊었다(더 큰 `socketEpoch` 를 받았다) — 다음 한 장은 revision 과
   * 무관하게 받는다. 잊어도 `snapshot` 은 그대로 둔다.
   */
  revision: number | null
}

interface UsageState {
  vendors: Partial<Record<AgentBackendKind, UsageVendorEntry>>
  /** 받아들인 가장 큰 소켓 표식(`UsageSnapshotPull.socketEpoch`). 0 = 아직 없다. */
  socketEpoch: number
  /** ⟳ 대기 수(회사별 — 웹뷰 안 슬롯끼리 공유한다). 읽을 땐 [`useUsagePending`]. */
  pending: Partial<Record<AgentBackendKind, number>>
  merge(snapshot: UsageLimitSnapshot, socketEpoch: number): void
  /**
   * 셸 캐시를 당겨 `merge` 한다 — 유계 재시도, 최종 실패는 들고 있는 값을 그대로 둔다(throw 하지 않는다).
   * ★방송 수신이 선 뒤에만 순서가 선다★ — eventBus 가 잇기 직후 한 번 부르고, 슬롯은 마운트 때 부른다.
   */
  pull(): Promise<void>
  /** ⟳ — 끝나면(답 · 실패 · 끊김 거절) 대기를 푼다. 값은 방송으로 온다. throw 하지 않는다. */
  refresh(vendor: AgentBackendKind): Promise<void>
}

function forgetRevisions(vendors: UsageState['vendors']): UsageState['vendors'] {
  const next: UsageState['vendors'] = {}
  for (const [vendor, entry] of Object.entries(vendors) as [AgentBackendKind, UsageVendorEntry][]) {
    next[vendor] = { ...entry, revision: null }
  }
  return next
}

function addPending(
  pending: UsageState['pending'],
  vendor: AgentBackendKind,
  delta: number,
): UsageState['pending'] {
  return { ...pending, [vendor]: Math.max(0, (pending[vendor] ?? 0) + delta) }
}

export const useUsageStore = create<UsageState>()((set, get) => ({
  vendors: {},
  socketEpoch: 0,
  pending: {},

  merge: (snapshot, socketEpoch) =>
    set(state => {
      let vendors = state.vendors
      let heldEpoch = state.socketEpoch
      // ADR-0195: 소켓 표식은 셸 프로세스 안에서 되감기지 않아 대소가 곧 소켓 순서다. 작으면 밀려난 소켓의 것이다.
      //   ★revision 을 잊는 자리는 여기 하나뿐이다★ — 연결 상태 전이(`connected` 떠남)로 잊지 않는다: 그 전이는
      //   다른 길로 와 새 소켓의 payload 보다 늦게 닿을 수 있고, 그때 바닥이 지워지면 같은 소켓의 늦은 옛 사본이
      //   받아들여져 화면이 되감긴다. 0 = 표식을 모른다 — 들고 있는 표식을 바꾸지 않는다.
      if (socketEpoch !== 0) {
        if (socketEpoch < heldEpoch) return state
        if (socketEpoch > heldEpoch) {
          heldEpoch = socketEpoch
          vendors = forgetRevisions(vendors)
        }
      }
      const prev = vendors[snapshot.vendor]
      // 같은 revision 은 받는다 — 같은 상태를 더 늦게 뜬 사본이라 상대 시간 칸이 더 새롭다.
      if (prev && prev.revision !== null && snapshot.revision < prev.revision) return state
      return {
        socketEpoch: heldEpoch,
        vendors: {
          ...vendors,
          [snapshot.vendor]: { snapshot, receivedAt: performance.now(), revision: snapshot.revision },
        },
      }
    }),

  pull: async () => {
    try {
      const { socketEpoch, snapshots } = await retryAsync(() => agentClient.getUsageSnapshot(), {
        onRetry: (err, attempt) => console.warn(`[usageStore] getUsageSnapshot 재시도 #${attempt}:`, err),
      })
      for (const snapshot of snapshots) get().merge(snapshot, socketEpoch)
    } catch (err) {
      console.warn('[usageStore] getUsageSnapshot 최종 실패 — 들고 있는 값 유지:', err)
    }
  },

  refresh: async vendor => {
    set(state => ({ pending: addPending(state.pending, vendor, 1) }))
    try {
      await agentClient.refreshUsageLimits(vendor)
    } catch (err) {
      // 끊김이면 ProtocolClient 가 대기 요청을 전부 거절한다 — 그 거절이 여기서 대기를 푸는 길이다.
      console.warn(`[usageStore] refreshUsageLimits(${vendor}) 실패:`, err)
    } finally {
      set(state => ({ pending: addPending(state.pending, vendor, -1) }))
    }
  },
}))

/** 그 회사의 받아 둔 값 — 없으면 `undefined`(아직 한 장도 못 받았다). */
export function useUsageVendor(vendor: AgentBackendKind): UsageVendorEntry | undefined {
  return useUsageStore(state => state.vendors[vendor])
}

/** 그 회사의 ⟳ 가 아직 끝나지 않았나. */
export function useUsagePending(vendor: AgentBackendKind): boolean {
  return useUsageStore(state => (state.pending[vendor] ?? 0) > 0)
}

/**
 * ⟳ 를 막는 거절 = 보이는 `Rejected` 상태. 작은 표시 ⟳ 비활성과 `usageSlot.refresh` 의 대상 고르기가 이 하나를
 * 읽는다. 값이 와서 `Ready` 로 보이는 동안의 거절은 여기 안 걸린다 — 그때 ⟳ 는 데몬이 조회 없이 캐시로
 * 답한다(TRD §3 #88).
 */
export function blocksRefresh(state: UsageVendorState | undefined): boolean {
  return state?.kind === 'Rejected'
}

/**
 * 켠 회사 중 ⟳ 를 보낼 수 있는 것 — [`blocksRefresh`] 에 안 걸리는 것. 값을 아직 못 받은 회사도 든다(⟳ 가 첫
 * 값을 부르는 길이다). `usageSlot.refresh` 가 누를 때 스토어로 다시 보며 이것으로 대상을 고른다.
 */
export function refreshableVendors(
  shows: { show_claude: boolean; show_codex: boolean },
  vendors: UsageState['vendors'],
): AgentBackendKind[] {
  const shown: AgentBackendKind[] = []
  if (shows.show_claude) shown.push('claude')
  if (shows.show_codex) shown.push('codex')
  return shown.filter(vendor => !blocksRefresh(vendors[vendor]?.snapshot.state))
}
