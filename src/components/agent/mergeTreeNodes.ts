// 트리 노드 머지 — 저장 프로필(예약/깡통) ∪ 실행중 에이전트 (ADR-0018) → parent_id 로 계층화(ADR-0072).
//
// "Reserved/대기"는 백엔드 상태가 아니라 프론트 합성이다: listProfiles() ∖ agents[].
// merge 키 = id (프로필 id == spawn 후 AgentInfo.id, 불변). 실행중이 우선한다.
// 백엔드 AgentStatus·protocol 무변경(§ ADR-0018 결정 2).
//
// ★계층(ADR-0072)★: 평면 concat 대신 profile.parent_id 로 자식을 부모 밑에 묶은 forest 를 반환한다.
//   1단 중첩만(A > B·C·D) — 백엔드가 "자식은 다시 부모가 될 수 없다"를 강제하지만 프론트는 방어적으로
//   *한 단계만* 중첩한다(§ nestByParent 주석). parent_id 는 AgentProfile 에만 있다(AgentInfo wire 엔
//   없음) → running 노드도 매칭 프로필에서 parent_id 를 이어받는다(display_name override 와 동형 조회).

import type { AgentInfo, AgentProfile } from '../../api/types'

/** 트리에 그릴 노드 1개. kind 로 실행중/예약을 구분 — 시각·더블클릭 동작이 갈린다. */
export type AgentTreeNode = {
  id: string
  name: string
  /** 작업 디렉토리(cwd). AgentList 가 표시명 override 없으면 이 값의 basename 으로 파생한다. */
  cwd: string
  /**
   * 사용자 지정 표시명 override(ADR-0061 리치화 — 트리 rename). Some → 그대로 표시, null → cwd basename
   * 파생(기존 동작 불변). ★AgentProfile.display_name 에서만 온다★: reserved 노드는 프로필 직접, running
   * 노드는 매칭 프로필이 있으면 그 override 를 이어받는다(AgentInfo wire 엔 display_name 이 없어 프로필 조회).
   */
  displayName: string | null
  /** 'running' = AgentStatus.type 문자열, 'reserved' = 깡통(미spawn 프로필). */
  status: string
  /** 'running'=실행중(또는 종료 등 세션 보유) / 'reserved'=저장만 된 깡통. */
  kind: 'running' | 'reserved'
  /** 우클릭 중단 가능 여부(running 전용). reserved 는 항상 false. */
  canInterrupt: boolean
  /**
   * 자식 노드(ADR-0072 — react-arborist childrenAccessor). 항상 배열(빈 배열 = leaf). 1단 중첩만이라
   * 자식은 항상 빈 children 을 갖지만, 타입은 재귀 트리로 둔다(react-arborist 가 forest 를 순회).
   */
  children: AgentTreeNode[]
}

/**
 * 실행중 에이전트와 저장 프로필을 id 기준으로 머지한 뒤 parent_id 로 계층화한다(ADR-0018 + ADR-0072).
 *
 * 머지(ADR-0018):
 * - agents 에 있으면 → 실행중 노드(기존 status 노드). 같은 id 의 프로필은 흡수(중복 없음).
 * - profiles 에만 있으면 → 예약(Reserved) 노드.
 * - 프로필 없이 실행중인 ad-hoc 셸(SpawnByCwd)은 agents 에만 있으므로 그대로 표시.
 *
 * 정렬(각 레벨 독립 적용): 실행중 먼저, 그다음 예약 프로필 — 사람이 활성 세션을 위에서 먼저 보게.
 *       각 그룹 내부는 결정적으로 정렬한다(MINOR-2): 백엔드 listProfiles/agents 가
 *       HashMap iteration(비결정적) 순서로 올 수 있어, 그대로 쓰면 refetch 마다 노드가 튄다.
 *       - reserved: created_at 오름차순(생성 순) → tiebreaker id. profile 은 created_at 보유.
 *       - running: AgentInfo 엔 created_at 이 없으므로 id 오름차순(안정 키).
 *       목표 = "같은 입력 집합이면 항상 같은 순서". 루트·자식 모두 같은 비교자로 정렬한다.
 *
 * 계층(ADR-0072): 평면 노드 목록을 만든 뒤 parent_id 로 자식을 부모 children 에 꽂는다(nestByParent).
 */
export function mergeTreeNodes(
  profiles: AgentProfile[],
  agents: AgentInfo[],
): AgentTreeNode[] {
  const runningIds = new Set(agents.map(a => a.id))
  // 표시명 override(display_name)와 parent_id 는 AgentProfile 에만 있다(AgentInfo wire 엔 없음). running
  //   노드가 매칭 프로필의 값을 이어받게 id→profile 맵을 만든다(reserved 는 프로필을 직접 매핑).
  const profileById = new Map(profiles.map(p => [p.id, p]))

  const runningNodes: AgentTreeNode[] = agents.map(a => ({
    id: a.id,
    name: a.name || a.id.slice(0, 8),
    cwd: a.cwd,
    // ad-hoc(SpawnByCwd)은 프로필이 없을 수 있다 → 맵 미스 시 null(basename 파생, 기존 동작 불변).
    displayName: profileById.get(a.id)?.display_name ?? null,
    status: a.status.type,
    kind: 'running' as const,
    canInterrupt: a.capabilities?.control?.interrupt ?? false,
    children: [],
  }))

  // 실행중에 이미 있는 id 의 프로필은 제외(실행중 우선). 남은 프로필만 예약 노드로.
  const reservedNodes: AgentTreeNode[] = profiles
    .filter(p => !runningIds.has(p.id))
    .map(p => ({
      id: p.id,
      name: p.name || p.id.slice(0, 8),
      cwd: p.cwd,
      displayName: p.display_name ?? null, // 프로필 override 직접 매핑.
      status: 'Reserved',
      kind: 'reserved' as const,
      canInterrupt: false,
      children: [],
    }))

  // 평면 노드 전체(계층화 입력) — parent_id·created_at 조회는 profileById 로. running 노드도 매칭 프로필의
  //   parent_id 를 이어받는다(ad-hoc 은 프로필 없음 → parent_id 없음 = 루트). created_at 도 매칭 프로필에서
  //   (running AgentInfo 엔 created_at 없음 → 0 = 정렬 시 id tiebreaker 로 떨어짐, 기존 동작 동형).
  const flat = [...runningNodes, ...reservedNodes]
  const parentOf = (id: string): string | null => profileById.get(id)?.parent_id ?? null
  const createdAtOf = (id: string): number => profileById.get(id)?.created_at ?? 0
  return nestByParent(flat, parentOf, createdAtOf)
}

/**
 * 결정적 정렬(MINOR-2, ADR-0072 — 레벨마다 동일 적용). running 먼저 → reserved 뒤, 각 그룹 안은 created_at
 * 오름차순 → id tiebreaker. created_at 은 프로필 맵으로 조회(running 은 매칭 프로필 없으면 0 → id 로만).
 * 목표 = "같은 입력 집합이면 항상 같은 순서"(루트·자식 동일 비교자).
 */
function sortNodes(nodes: AgentTreeNode[], createdAtOf: (id: string) => number): void {
  nodes.sort((x, y) => {
    // running(0) 이 reserved(1) 보다 먼저.
    const rankX = x.kind === 'running' ? 0 : 1
    const rankY = y.kind === 'running' ? 0 : 1
    if (rankX !== rankY) return rankX - rankY
    // 같은 그룹: created_at 오름차순.
    const cx = createdAtOf(x.id)
    const cy = createdAtOf(y.id)
    if (cx !== cy) return cx - cy
    // tiebreaker: id 오름차순(안정 키).
    return x.id < y.id ? -1 : x.id > y.id ? 1 : 0
  })
}

/**
 * 평면 노드 목록을 parent_id 로 1단 forest 로 접는다(ADR-0072). 루트·각 부모의 children 을 결정적 정렬.
 *
 * ★1단 중첩만★: parent_id 가 존재하는 노드를 가리키고, 그 부모 자신이 루트(부모의 parent_id 가 없음)일
 *   때만 자식으로 꽂는다. 백엔드가 "자식은 부모가 될 수 없다"를 강제하지만, 프론트는 데이터가 어긋나도
 *   (부모가 또 자식이거나, parent_id 가 존재하지 않는 id 를 가리키거나, self-parent) 안전하게 그 노드를
 *   루트로 승격시킨다 — 절대 2단 이상 중첩하지 않는다(cycle·무한 depth 방어).
 */
function nestByParent(
  flat: AgentTreeNode[],
  parentOf: (id: string) => string | null,
  createdAtOf: (id: string) => number,
): AgentTreeNode[] {
  const byId = new Map(flat.map(n => [n.id, n]))

  const roots: AgentTreeNode[] = []
  for (const node of flat) {
    const pid = parentOf(node.id)
    // 루트 판정: parent_id 없음 · self · 존재하지 않는 부모 · 부모가 또 자식(2단 방지) → 루트로 승격.
    const parent = pid !== null && pid !== node.id ? byId.get(pid) : undefined
    const parentIsRoot = parent !== undefined && parentOf(parent.id) === null
    if (parent && parentIsRoot) {
      parent.children.push(node)
    } else {
      roots.push(node)
    }
  }

  // 결정적 정렬: 루트 먼저, 그다음 각 부모의 children(1단이라 children 의 children 은 항상 빈 배열).
  sortNodes(roots, createdAtOf)
  for (const node of roots) {
    if (node.children.length > 0) sortNodes(node.children, createdAtOf)
  }
  return roots
}
