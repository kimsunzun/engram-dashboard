// "Reserved/waiting" is a frontend composition, not a backend state: listProfiles() ∖ agents[]. Merge key = id (profile id == AgentInfo.id after spawn, immutable); running takes priority. Backend AgentStatus · protocol unchanged (ADR-0018 decision 2).
//
// ★Hierarchy (ADR-0072)★: returns a forest grouping children under their parent by profile.parent_id. Only 1 level (A > B·C·D) — the backend enforces "a child cannot become a parent" but the frontend nests defensively *one level only* (see nestByParent). parent_id exists only on AgentProfile (not the AgentInfo wire), so a running node inherits it from the matching profile (like the display_name override).

import type { AgentFailureKind, AgentInfo, AgentProfile } from '../../api/types'

/**
 * What a single id is right now — the tree's "running vs reserved" decision asked by **a single id** (ADR-0148).
 * - `running`: in the authoritative registry (agents), even if the status is terminal (session not yet reaped).
 * - `reserved`: not in the registry, only a profile — **the shape after kill and after the reaper removes the session** (the profile stays as a corpse). Same set as the tree's "reserved" node.
 * - `unknown`: neither exists (deleted from the tree).
 *
 * ★One rule for both the tree composition (mergeTreeNodes) and the slot render gate (LayoutLeaf)★ — kept in this one function so fixing one side cannot cause a mismatch. Callers pass the registry and profiles as is.
 * Not told apart here: the span before the registry is received (agents is empty so it looks `reserved`) — the caller must check `agentsLoaded` first.
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
  /** ★Only from AgentProfile.display_name★: a reserved node reads the profile directly; a running node inherits it from the matching profile (the AgentInfo wire has no display_name). */
  displayName: string | null
  status: string
  /** ★Separate axis from status★ (ADR-0172): the matching profile's "last failure". A running node can hold it, and "running wins" hides it on screen (decided in one place, `statusGlyph.isFailureBlocked`). An ad-hoc running node with no profile is null. */
  lastFailure: AgentFailureKind | null
  kind: 'running' | 'reserved'
  canInterrupt: boolean
  /** Whether a matching profile is held (ADR-0072 drop guard): reserved = always true; running = whether a profile exists (an ad-hoc SpawnByCwd shell has none → false). Reparent needs a real profile on both child and parent (the backend rejects otherwise), so a false node cannot be dragged or be a drop parent. */
  hasProfile: boolean
  /** react-arborist childrenAccessor (ADR-0072). Only 1 level, so a child's children is always empty, but the type stays recursive (react-arborist walks the forest). */
  children: AgentTreeNode[]
}

/**
 * Sorting (per level): running first, then reserved profiles. Within each group, deterministic (MINOR-2): backend listProfiles/agents may arrive in HashMap order, which would make nodes jump on every refetch — the same input set always gives the same order (same comparator for roots and children).
 */
export function mergeTreeNodes(
  profiles: AgentProfile[],
  agents: AgentInfo[],
): AgentTreeNode[] {
  // display_name and parent_id exist only on AgentProfile (not the AgentInfo wire): an id→profile map lets a running node inherit them (reserved maps the profile directly).
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

/** Deterministic sort (MINOR-2, ADR-0072 — same at every level). created_at comes via the profile map (running with no profile gets 0 → by id only). */
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
 * ★Only 1 level★: a node is plugged in as a child only when parent_id points at an existing node that is itself a root. The backend enforces this, but on out-of-step data (parent is itself a child · parent_id of a nonexistent id · self-parent) the frontend promotes the node to root — never 2+ levels (guards cycles · infinite depth).
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
