// ★Display name = profile.display_name (ADR-0061 — same source as the tree)★: formerly cwd basename was the only source, but
//   with tree rename (ADR-0061) a display name override came to exist on the profile. If it is not the same as the tree, an agent renamed to "ABC"
//   shows in the popup as the cwd basename (e.g. "Filter Library"), diverging from the tree, and agents with the same cwd get
//   confused. ★profile.name is not used★ — in this app's profiles name is the cwd string so the full path would show (the tree too uses only
//   display_name ?? basename). display_name and the profile are not in the AgentInfo wire and come only from AgentProfile.
//
// ★"실행중"★ (running): Exiting/Exited/Killed/Failed are not monitoring targets and are excluded (the purpose is live observation).

import type { AgentInfo, AgentProfile } from '../../api/types'
import { basename } from '../../util/basename'

export interface MonitoringCandidate {
  id: string
  /**
   * Display name = profile.display_name (ADR-0061 — same source as the tree, joined by id). With no override falls back to cwd basename
   * (frontend-derived, same as AgentList mergeTreeNodes — profile.name is a cwd string so it is not used).
   */
  name: string
  cwd: string
}

/** Exit/failure/transition states are not monitoring targets and are excluded. */
export function runningAgents(agents: AgentInfo[]): AgentInfo[] {
  return agents.filter(a => a.status.type === 'Running')
}

/**
 * Sorting preserves input order (the caller already passes a deterministic order — a path separate from AgentList mergeTreeNodes, so
 * it is not re-sorted here: the agents array is the setAgents replacement as it is).
 */
export function filterMonitoringCandidates(
  agents: AgentInfo[],
  profiles: AgentProfile[],
  query: string,
): MonitoringCandidate[] {
  const q = query.trim().toLowerCase()
  // ADR-0061: profile.id === agent.id is invariant after spawn.
  const profileById = new Map(profiles.map(p => [p.id, p]))
  return runningAgents(agents)
    .map(a => {
      const profile = profileById.get(a.id)
      // ★The same chain as the tree (AgentList)★: profile.name is not used — in this app's profiles name is the cwd
      //   string (createClaudeProfile name=cwd) so the full path would show, which is worse than basename.
      const name = profile?.display_name ?? basename(a.cwd)
      return { id: a.id, name, cwd: a.cwd }
    })
    .filter(c => {
      if (q.length === 0) return true
      return c.name.toLowerCase().includes(q) || c.cwd.toLowerCase().includes(q)
    })
}
