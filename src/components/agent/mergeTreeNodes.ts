// 트리 노드 머지 — 저장 프로필(예약/깡통) ∪ 실행중 에이전트 (ADR-0018).
//
// "Reserved/대기"는 백엔드 상태가 아니라 프론트 합성이다: listProfiles() ∖ agents[].
// merge 키 = id (프로필 id == spawn 후 AgentInfo.id, 불변). 실행중이 우선한다.
// 백엔드 AgentStatus·protocol 무변경(§ ADR-0018 결정 2).

import type { AgentInfo, AgentProfile } from '../../api/types'

/** 트리에 그릴 노드 1개. kind 로 실행중/예약을 구분 — 시각·더블클릭 동작이 갈린다. */
export type AgentTreeNode = {
  id: string
  name: string
  /** 작업 디렉토리(cwd). AgentList 가 이 값의 basename 으로 표시명을 파생한다(이름 미저장 정책). */
  cwd: string
  /** 'running' = AgentStatus.type 문자열, 'reserved' = 깡통(미spawn 프로필). */
  status: string
  /** 'running'=실행중(또는 종료 등 세션 보유) / 'reserved'=저장만 된 깡통. */
  kind: 'running' | 'reserved'
  /** 우클릭 중단 가능 여부(running 전용). reserved 는 항상 false. */
  canInterrupt: boolean
}

/**
 * 실행중 에이전트와 저장 프로필을 id 기준으로 머지한다.
 *
 * - agents 에 있으면 → 실행중 노드(기존 status 노드). 같은 id 의 프로필은 흡수(중복 없음).
 * - profiles 에만 있으면 → 예약(Reserved) 노드.
 * - 프로필 없이 실행중인 ad-hoc 셸(SpawnByCwd)은 agents 에만 있으므로 그대로 표시.
 *
 * 정렬: 실행중 먼저, 그다음 예약 프로필 — 사람이 활성 세션을 위에서 먼저 보게.
 *       각 그룹 내부는 결정적으로 정렬한다(MINOR-2): 백엔드 listProfiles/agents 가
 *       HashMap iteration(비결정적) 순서로 올 수 있어, 그대로 쓰면 refetch 마다 노드가 튄다.
 *       - reserved: created_at 오름차순(생성 순) → tiebreaker id. profile 은 created_at 보유.
 *       - running: AgentInfo 엔 created_at 이 없으므로 id 오름차순(안정 키).
 *       목표 = "같은 입력 집합이면 항상 같은 순서".
 */
export function mergeTreeNodes(
  profiles: AgentProfile[],
  agents: AgentInfo[],
): AgentTreeNode[] {
  const runningIds = new Set(agents.map(a => a.id))

  const runningNodes: AgentTreeNode[] = agents
    .map(a => ({
      id: a.id,
      name: a.name || a.id.slice(0, 8),
      cwd: a.cwd,
      status: a.status.type,
      kind: 'running' as const,
      canInterrupt: a.capabilities?.control?.interrupt ?? false,
    }))
    // AgentInfo 엔 안정 시간키가 없다 → id 로 결정적 정렬.
    .sort((x, y) => (x.id < y.id ? -1 : x.id > y.id ? 1 : 0))

  // 실행중에 이미 있는 id 의 프로필은 제외(실행중 우선). 남은 프로필만 예약 노드로.
  // 매핑 전에 원본 프로필을 결정적으로 정렬한다(created_at → id) → AgentTreeNode 에
  // created_at 을 싣지 않고도 안정 순서 확보.
  const reservedNodes: AgentTreeNode[] = profiles
    .filter(p => !runningIds.has(p.id))
    .sort((x, y) =>
      x.created_at !== y.created_at
        ? x.created_at - y.created_at
        : x.id < y.id ? -1 : x.id > y.id ? 1 : 0,
    )
    .map(p => ({
      id: p.id,
      name: p.name || p.id.slice(0, 8),
      cwd: p.cwd,
      status: 'Reserved',
      kind: 'reserved' as const,
      canInterrupt: false,
    }))

  return [...runningNodes, ...reservedNodes]
}
