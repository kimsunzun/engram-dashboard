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
    claude_session_id: null,
    old_session_ids: [],
    epoch: 0,
    auto_restore: false,
    restart_policy: 'Never',
    restart_count: 0,
    failed_reason: null,
    created_at: 0,
    last_active: 0,
    last_start_at: null,
  }
}

describe('runningAgents — 실행중만', () => {
  it('status.type==="Running" 만 남기고 종료/실패/전이는 제외한다', () => {
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

describe('filterMonitoringCandidates — 실행중 ∩ 검색어', () => {
  it('빈 검색어 → 실행중 전체(프로필 없으면 표시명 = cwd basename)', () => {
    const list = [
      agent('a', 'C:/work/alpha'),
      agent('b', 'C:/work/beta', { type: 'Exited', code: 0 }),
      agent('c', 'C:/proj/gamma'),
    ]
    const out = filterMonitoringCandidates(list, [], '')
    expect(out.map(c => c.id)).toEqual(['a', 'c'])
    expect(out.map(c => c.name)).toEqual(['alpha', 'gamma'])
  })

  it('검색어는 표시명·cwd 부분일치(대소문자 무시)', () => {
    const list = [
      agent('a', 'C:/work/alpha'),
      agent('b', 'C:/work/beta'),
      agent('c', 'C:/other/alphabet'),
    ]
    expect(filterMonitoringCandidates(list, [], 'ALPHA').map(c => c.id)).toEqual(['a', 'c'])
    expect(filterMonitoringCandidates(list, [], 'work').map(c => c.id)).toEqual(['a', 'b'])
    expect(filterMonitoringCandidates(list, [], 'zzz')).toEqual([])
  })

  it('실행중 없으면 빈 배열(검색어 무관)', () => {
    const list = [agent('a', 'C:/x', { type: 'Killed' })]
    expect(filterMonitoringCandidates(list, [], '')).toEqual([])
    expect(filterMonitoringCandidates(list, [], 'x')).toEqual([])
  })

  it('입력 순서를 보존한다(재정렬 없음)', () => {
    const list = [agent('z', 'C:/work/zed'), agent('a', 'C:/work/apex')]
    expect(filterMonitoringCandidates(list, [], '').map(c => c.id)).toEqual(['z', 'a'])
  })

  // ADR-0061: 표시명은 트리(mergeTreeNodes)와 동일하게 프로필에서 파생한다.
  it('프로필의 display_name 이 있으면 그 이름을 쓴다(cwd basename 아님)', () => {
    // rename 시나리오.
    const list = [agent('a', 'C:/repos/Filter Library')]
    const profiles = [profile('a', { display_name: 'ABC', name: 'orig', cwd: 'C:/repos/Filter Library' })]
    expect(filterMonitoringCandidates(list, profiles, '').map(c => c.name)).toEqual(['ABC'])
    expect(filterMonitoringCandidates(list, profiles, 'abc').map(c => c.id)).toEqual(['a'])
  })

  it('display_name 이 없으면 cwd basename 폴백(profile.name 은 cwd 문자열이라 미사용 — 트리와 동일)', () => {
    const list = [
      agent('a', 'C:/work/alpha'), // 프로필 있으나 display_name=null
      agent('b', 'C:/work/beta'), // 매칭 프로필 없는 ad-hoc
    ]
    // profile.name 을 일부러 cwd 전체 경로로 둔다 — 실제 createClaudeProfile 동작.
    const profiles = [profile('a', { display_name: null, name: 'C:/work/alpha', cwd: 'C:/work/alpha' })]
    expect(filterMonitoringCandidates(list, profiles, '').map(c => c.name)).toEqual(['alpha', 'beta'])
  })
})
