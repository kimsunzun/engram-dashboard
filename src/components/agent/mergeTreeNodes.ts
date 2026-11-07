// "Reserved/waiting" is not a backend state but a frontend composition: listProfiles() ∖ agents[].
// merge key = id (profile id == AgentInfo.id after spawn, immutable). Running takes priority.
// Backend AgentStatus · protocol unchanged (§ ADR-0018 decision 2).
//
// ★Hierarchy (ADR-0072)★: instead of a flat concat, returns a forest that groups children under their parent by profile.parent_id.
//   Only 1-level nesting (A > B·C·D) — the backend enforces "a child cannot become a parent again" but the frontend defensively nests
//   *only one level* (see the § nestByParent comment). parent_id exists only on AgentProfile (not on the AgentInfo wire)
//   → a running node also inherits parent_id from the matching profile (a lookup isomorphic to the display_name override).

import type { AgentFailureKind, AgentInfo, AgentProfile } from '../../api/types'

/**
 * What a single id is right now — the form that asks the tree's "running vs reserved" decision by **a single id** (ADR-0148).
 *
 * - `running`   : in the authoritative registry (agents). Even if the status is terminal it belongs here, since the session is not yet reaped.
 * - `reserved`  : not in the registry, only a profile. **This is the shape after kill and after the reaper reaps the session**
 *                 — the reaper deletes it from the registry but leaves the profile as a corpse. The same set as the tree's "reserved" node.
 * - `unknown`   : neither exists (deleted from the tree).
 *
 * ★We do not write this rule separately in two places★: the tree composition (mergeTreeNodes) and the slot render gate (LayoutLeaf) use
 *   the same decision, so it is kept in this one function so that fixing only one side cannot cause a mismatch. Callers pass the registry and profiles
 *   as they are (tens of entries — no reason to build an index in advance).
 *
 * The span where the registry itself has not yet been received is not told apart by this function (agents is then an empty array so it looks `reserved`) —
 * the caller must check `agentsLoaded` first.
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
   * ★Comes only from AgentProfile.display_name★: a reserved node reads the profile directly, and a running
   * node inherits that override if there is a matching profile (the AgentInfo wire has no display_name, so look up the profile).
   */
  displayName: string | null
  status: string
  /**
   * ★An axis separate from status★ (ADR-0172): the matching profile's 「마지막 실패」 (last failure). A running node can hold it too, and then
   * 「도는 중이 이긴다」 (running wins) means it does not show on the screen (the decision is in one place, `statusGlyph.isFailureBlocked`).
   * An ad-hoc running node with no profile is always null.
   */
  lastFailure: AgentFailureKind | null
  /** 'running' = running (or holding a session, e.g. exited) / 'reserved' = an empty shell that is only saved. */
  kind: 'running' | 'reserved'
  canInterrupt: boolean
  /**
   * Whether a matching profile is held (for the ADR-0072 drop guard). reserved = always true (created from a profile), running =
   * whether a matching profile exists (an ad-hoc SpawnByCwd shell has no profile so false). Why: reparent holds only when both child and parent
   * have a real profile (the backend rejects a no-profile op with Error) → used for the frontend drag/drop pre-filter
   * (a false node cannot be dragged · cannot be a drop parent). It carries only whether a profile exists, not the hierarchy decision (parent_id).
   */
  hasProfile: boolean
  /**
   * react-arborist childrenAccessor (ADR-0072). With only 1-level nesting
   * a child always has empty children, but the type is kept as a recursive tree (react-arborist walks the forest).
   */
  children: AgentTreeNode[]
}

/**
 * Sorting (applied independently at each level): running first, then reserved profiles — so a human sees active sessions at the top first.
 *       Within each group, sort deterministically (MINOR-2): the backend listProfiles/agents can arrive in
 *       HashMap iteration (non-deterministic) order, so using it as is makes nodes jump on every refetch.
 *       Goal = "the same input set always gives the same order". Roots and children are sorted with the same comparator.
 */
export function mergeTreeNodes(
  profiles: AgentProfile[],
  agents: AgentInfo[],
): AgentTreeNode[] {
  // The display name override (display_name) and parent_id exist only on AgentProfile (not on the AgentInfo wire). Build an id→profile map so a running
  //   node inherits the matching profile's values (reserved maps the profile directly).
  const profileById = new Map(profiles.map(p => [p.id, p]))

  const runningNodes: AgentTreeNode[] = agents.map(a => ({
    id: a.id,
    name: a.name || a.id.slice(0, 8),
    cwd: a.cwd,
    // An ad-hoc (SpawnByCwd) may have no profile → null on a map miss (basename derivation, existing behavior unchanged).
    displayName: profileById.get(a.id)?.display_name ?? null,
    status: a.status.type,
    // ADR-0172: an axis absent from the AgentInfo wire, so like display_name it is inherited from the matching profile.
    lastFailure: profileById.get(a.id)?.last_failure ?? null,
    kind: 'running' as const,
    canInterrupt: a.capabilities?.control?.interrupt ?? false,
    // Whether a matching profile exists = the drop guard pre-filter (an ad-hoc shell has no profile so false → cannot be a drag/drop parent).
    hasProfile: profileById.has(a.id),
    children: [],
  }))

  const reservedNodes: AgentTreeNode[] = profiles
    // The reserved decision = the same rule as agentPresence above (shared with the slot render gate — not written separately in two places).
    .filter(p => agentPresence(p.id, agents, profiles) === 'reserved')
    .map(p => ({
      id: p.id,
      name: p.name || p.id.slice(0, 8),
      cwd: p.cwd,
      displayName: p.display_name ?? null,
      status: 'Reserved',
      lastFailure: p.last_failure ?? null,
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
 * Deterministic sort (MINOR-2, ADR-0072 — applied identically at every level). created_at is looked up via the profile map (running with
 * no matching profile gets 0 → by id only).
 * Goal = "the same input set always gives the same order" (same comparator for roots and children).
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
 * ★Only 1-level nesting★: only when parent_id points at an existing node and that parent itself is a root (the parent has no parent_id)
 *   is it plugged in as a child. The backend enforces "a child cannot become a parent", but the frontend safely promotes that node to
 *   root even if data is out of step (the parent is itself a child, parent_id points at a nonexistent id, or self-parent) — it never nests
 *   2 or more levels (guards against cycles · infinite depth).
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
