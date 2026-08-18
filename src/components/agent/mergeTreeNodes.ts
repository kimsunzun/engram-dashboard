// "Reserved/대기"는 백엔드 상태가 아니라 프론트 합성이다: listProfiles() ∖ agents[].
// merge 키 = id (프로필 id == spawn 후 AgentInfo.id, 불변). 실행중이 우선한다.
// 백엔드 AgentStatus·protocol 무변경(§ ADR-0018 결정 2).
//
// ★계층(ADR-0072)★: 평면 concat 대신 profile.parent_id 로 자식을 부모 밑에 묶은 forest 를 반환한다.
//   1단 중첩만(A > B·C·D) — 백엔드가 "자식은 다시 부모가 될 수 없다"를 강제하지만 프론트는 방어적으로
//   *한 단계만* 중첩한다(§ nestByParent 주석). parent_id 는 AgentProfile 에만 있다(AgentInfo wire 엔
//   없음) → running 노드도 매칭 프로필에서 parent_id 를 이어받는다(display_name override 와 동형 조회).

import type { AgentInfo, AgentProfile } from '../../api/types'

/**
 * 한 id 가 지금 무엇인가 — 트리의 "실행중 vs 예약" 판정을 **id 하나**로 물어보는 형태(ADR-0148).
 *
 * - `running`   : 권위 명부(agents)에 있다. 상태가 terminal 이어도 세션이 아직 수거 안 된 것이라 여기 든다.
 * - `reserved`  : 명부엔 없고 프로필만 있다. **종료(kill) 후 reaper 가 세션을 수거한 뒤의 모습**이 이것이다
 *                 — reaper 는 명부에서 지우지만 프로필은 시체로 남긴다. 트리의 "예약" 노드와 같은 집합.
 * - `unknown`   : 둘 다 없다(트리에서 삭제됨).
 *
 * ★이 규칙을 두 곳에 각각 적지 않는다★: 트리 합성(mergeTreeNodes)과 슬롯 렌더 게이트(ViewLayoutRenderer)가
 *   같은 판정을 쓰므로, 한쪽만 고쳐 어긋나는 일이 생기지 않게 여기 한 함수로 둔다. 호출자는 명부·프로필을
 *   그대로 넘긴다(수십 건 규모 — 인덱스를 미리 만들 이유가 없다).
 *
 * 명부 자체를 아직 못 받은 구간은 이 함수로 갈리지 않는다(그때 agents 는 빈 배열이라 `reserved` 로 보인다) —
 * 호출자가 `agentsLoaded` 를 먼저 확인해야 한다.
 */
export function agentPresence(
  id: string,
  agents: AgentInfo[],
  profiles: AgentProfile[],
): 'running' | 'reserved' | 'unknown' {
  if (agents.some(a => a.id === id)) return 'running'
  if (profiles.some(p => p.id === id)) return 'reserved'
  return 'unknown'
}

export type AgentTreeNode = {
  id: string
  name: string
  cwd: string
  /**
   * ★AgentProfile.display_name 에서만 온다★: reserved 노드는 프로필 직접, running
   * 노드는 매칭 프로필이 있으면 그 override 를 이어받는다(AgentInfo wire 엔 display_name 이 없어 프로필 조회).
   */
  displayName: string | null
  status: string
  /** 'running'=실행중(또는 종료 등 세션 보유) / 'reserved'=저장만 된 깡통. */
  kind: 'running' | 'reserved'
  canInterrupt: boolean
  /**
   * 매칭 프로필 보유 여부(ADR-0072 드롭 가드용). reserved = 항상 true(프로필에서 생성), running =
   * 매칭 프로필 존재 여부(ad-hoc SpawnByCwd 셸은 프로필 없어 false). 왜: reparent 는 child·parent 둘 다
   * 실 프로필이 있어야 성립한다(백엔드가 no-profile op 를 Error 로 거부) → 프론트 드래그/드롭 pre-filter 에
   * 쓴다(false 인 노드는 드래그 불가·드롭 부모로 불가). 계층 판정(parent_id)이 아니라 프로필 유무만 담는다.
   */
  hasProfile: boolean
  /**
   * react-arborist childrenAccessor(ADR-0072). 1단 중첩만이라
   * 자식은 항상 빈 children 을 갖지만, 타입은 재귀 트리로 둔다(react-arborist 가 forest 를 순회).
   */
  children: AgentTreeNode[]
}

/**
 * 정렬(각 레벨 독립 적용): 실행중 먼저, 그다음 예약 프로필 — 사람이 활성 세션을 위에서 먼저 보게.
 *       각 그룹 내부는 결정적으로 정렬한다(MINOR-2): 백엔드 listProfiles/agents 가
 *       HashMap iteration(비결정적) 순서로 올 수 있어, 그대로 쓰면 refetch 마다 노드가 튄다.
 *       목표 = "같은 입력 집합이면 항상 같은 순서". 루트·자식 모두 같은 비교자로 정렬한다.
 */
export function mergeTreeNodes(
  profiles: AgentProfile[],
  agents: AgentInfo[],
): AgentTreeNode[] {
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
    // 매칭 프로필 존재 여부 = 드롭 가드 pre-filter(ad-hoc 셸은 프로필 없어 false → 드래그/드롭 부모 불가).
    hasProfile: profileById.has(a.id),
    children: [],
  }))

  const reservedNodes: AgentTreeNode[] = profiles
    // 예약 판정 = 위 agentPresence 와 같은 규칙(슬롯 렌더 게이트와 공유 — 두 곳에 각각 적지 않는다).
    .filter(p => agentPresence(p.id, agents, profiles) === 'reserved')
    .map(p => ({
      id: p.id,
      name: p.name || p.id.slice(0, 8),
      cwd: p.cwd,
      displayName: p.display_name ?? null,
      status: 'Reserved',
      kind: 'reserved' as const,
      canInterrupt: false,
      hasProfile: true,
      children: [],
    }))

  const flat = [...runningNodes, ...reservedNodes]
  const parentOf = (id: string): string | null => profileById.get(id)?.parent_id ?? null
  const createdAtOf = (id: string): number => profileById.get(id)?.created_at ?? 0
  return nestByParent(flat, parentOf, createdAtOf)
}

/**
 * 결정적 정렬(MINOR-2, ADR-0072 — 레벨마다 동일 적용). created_at 은 프로필 맵으로 조회(running 은
 * 매칭 프로필 없으면 0 → id 로만).
 * 목표 = "같은 입력 집합이면 항상 같은 순서"(루트·자식 동일 비교자).
 */
function sortNodes(nodes: AgentTreeNode[], createdAtOf: (id: string) => number): void {
  nodes.sort((x, y) => {
    const rankX = x.kind === 'running' ? 0 : 1
    const rankY = y.kind === 'running' ? 0 : 1
    if (rankX !== rankY) return rankX - rankY
    const cx = createdAtOf(x.id)
    const cy = createdAtOf(y.id)
    if (cx !== cy) return cx - cy
    return x.id < y.id ? -1 : x.id > y.id ? 1 : 0
  })
}

/**
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
    const parent = pid !== null && pid !== node.id ? byId.get(pid) : undefined
    const parentIsRoot = parent !== undefined && parentOf(parent.id) === null
    if (parent && parentIsRoot) {
      parent.children.push(node)
    } else {
      roots.push(node)
    }
  }

  sortNodes(roots, createdAtOf)
  for (const node of roots) {
    if (node.children.length > 0) sortNodes(node.children, createdAtOf)
  }
  return roots
}
