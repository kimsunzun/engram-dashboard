import { describe, expect, it } from 'vitest'

import type { AgentCommand, AgentInfo, AgentProfile, AgentStatus } from '../../api/types'
import { filterMonitoringCandidates, runningAgents } from './monitoringPickerFilter'

function agent(id: string, cwd: string, status: AgentStatus = { type: 'Running' }): AgentInfo {
  return {
    id,
    name: '',
    cwd,
    status,
    cols: 80,
    rows: 24,
    epoch: 0,
    capabilities: {
      input: { raw: true, message: false, attachment: false },
      output: { terminal_bytes: true, structured: false, markdown: false, tool_events: false, usage: false },
      control: { resize: true, interrupt: true, cancel: false, graceful_shutdown: false },
      session: { resume: false, snapshot: false, cwd_env: true },
      model: { select: false, temperature: false, max_tokens: false },
    },
  }
}

function profile(
  id: string,
  { display_name = null, name = '', cwd = 'C:/x' }: { display_name?: string | null; name?: string; cwd?: string } = {},
): AgentProfile {
  const command: AgentCommand = { kind: 'Shell', program: 'sh', args: [] }
  return {
    id,
    name,
    display_name,
    parent_id: null,
    command,
    cwd,
    env: [],
    backend_session_id: null,
    old_session_ids: [],
    epoch: 0,
    auto_restore: false,
    restart_policy: 'Never',
    restart_count: 0,
    failed_reason: null,
    last_failure: null,
    created_at: 0,
    last_active: 0,
    last_start_at: null,
  }
}

describe('runningAgents — running only', () => {
  it('keeps only status.type==="Running" and excludes exited/failed/transitional', () => {
    const list = [
      agent('a', 'C:/work/alpha', { type: 'Running' }),
      agent('b', 'C:/work/beta', { type: 'Exited', code: 0 }),
      agent('c', 'C:/work/gamma', { type: 'Killed' }),
      agent('d', 'C:/work/delta', { type: 'Failed', message: 'x' }),
      agent('e', 'C:/work/epsilon', { type: 'Exiting' }),
      agent('f', 'C:/work/zeta', { type: 'Running' }),
    ]
    expect(runningAgents(list).map(a => a.id)).toEqual(['a', 'f'])
  })
})

describe('filterMonitoringCandidates — running ∩ query', () => {
  it('empty query → all running (display name = cwd basename if no profile)', () => {
    const list = [
      agent('a', 'C:/work/alpha'),
      agent('b', 'C:/work/beta', { type: 'Exited', code: 0 }),
      agent('c', 'C:/proj/gamma'),
    ]
    const out = filterMonitoringCandidates(list, [], '')
    expect(out.map(c => c.id)).toEqual(['a', 'c'])
    expect(out.map(c => c.name)).toEqual(['alpha', 'gamma'])
  })

  it('the query is a partial match on display name · cwd (case-insensitive)', () => {
    const list = [
      agent('a', 'C:/work/alpha'),
      agent('b', 'C:/work/beta'),
      agent('c', 'C:/other/alphabet'),
    ]
    expect(filterMonitoringCandidates(list, [], 'ALPHA').map(c => c.id)).toEqual(['a', 'c'])
    expect(filterMonitoringCandidates(list, [], 'work').map(c => c.id)).toEqual(['a', 'b'])
    expect(filterMonitoringCandidates(list, [], 'zzz')).toEqual([])
  })

  it('with none running an empty array (regardless of query)', () => {
    const list = [agent('a', 'C:/x', { type: 'Killed' })]
    expect(filterMonitoringCandidates(list, [], '')).toEqual([])
    expect(filterMonitoringCandidates(list, [], 'x')).toEqual([])
  })

  it('preserves input order (no re-sorting)', () => {
    const list = [agent('z', 'C:/work/zed'), agent('a', 'C:/work/apex')]
    expect(filterMonitoringCandidates(list, [], '').map(c => c.id)).toEqual(['z', 'a'])
  })

  // ADR-0061: the display name is derived from the profile the same as the tree (mergeTreeNodes).
  it('if the profile has display_name that name is used (not the cwd basename)', () => {
    // rename scenario.
    const list = [agent('a', 'C:/repos/Filter Library')]
    const profiles = [profile('a', { display_name: 'ABC', name: 'orig', cwd: 'C:/repos/Filter Library' })]
    expect(filterMonitoringCandidates(list, profiles, '').map(c => c.name)).toEqual(['ABC'])
    expect(filterMonitoringCandidates(list, profiles, 'abc').map(c => c.id)).toEqual(['a'])
  })

  it('without display_name cwd basename fallback (profile.name is a cwd string so unused — same as the tree)', () => {
    const list = [
      agent('a', 'C:/work/alpha'), // Has a profile but display_name=null
      agent('b', 'C:/work/beta'), // ad-hoc with no matching profile
    ]
    // profile.name is deliberately set to the full cwd path — the real createClaudeProfile behavior.
    const profiles = [profile('a', { display_name: null, name: 'C:/work/alpha', cwd: 'C:/work/alpha' })]
    expect(filterMonitoringCandidates(list, profiles, '').map(c => c.name)).toEqual(['alpha', 'beta'])
  })
})
