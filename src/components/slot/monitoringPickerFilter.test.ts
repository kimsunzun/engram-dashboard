// ADR-0067: 모니터링 팝업 필터/검색 순수 로직 단위테스트(headless — DOM/Tauri 의존 0).
//
// ★검증 불변식★:
//   1. runningAgents = status.type==='Running' 만(종료/실패/전이 제외).
//   2. filterMonitoringCandidates = 실행중 ∩ 검색어(basename·cwd 부분일치, 대소문자 무시).
//   3. 빈 검색어 → 실행중 전체. 표시명 = cwd basename.
//   4. 입력 순서 보존(재정렬 없음).

import { describe, expect, it } from 'vitest'

import type { AgentInfo, AgentStatus } from '../../api/types'
import { filterMonitoringCandidates, runningAgents } from './monitoringPickerFilter'

/** 테스트용 AgentInfo 최소 팩토리 — 필터가 보는 필드(id/cwd/status)만 채우고 나머지는 더미. */
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
  it('빈 검색어 → 실행중 전체(표시명 = cwd basename)', () => {
    const list = [
      agent('a', 'C:/work/alpha'),
      agent('b', 'C:/work/beta', { type: 'Exited', code: 0 }),
      agent('c', 'C:/proj/gamma'),
    ]
    const out = filterMonitoringCandidates(list, '')
    expect(out.map(c => c.id)).toEqual(['a', 'c']) // b(Exited) 제외
    expect(out.map(c => c.name)).toEqual(['alpha', 'gamma']) // basename 파생
  })

  it('검색어는 basename·cwd 부분일치(대소문자 무시)', () => {
    const list = [
      agent('a', 'C:/work/alpha'),
      agent('b', 'C:/work/beta'),
      agent('c', 'C:/other/alphabet'),
    ]
    // 'ALPHA' → alpha, alphabet 매칭(대소문자 무시).
    expect(filterMonitoringCandidates(list, 'ALPHA').map(c => c.id)).toEqual(['a', 'c'])
    // 경로 세그먼트로도 매칭('work' 는 basename 이 아니라 cwd 부분).
    expect(filterMonitoringCandidates(list, 'work').map(c => c.id)).toEqual(['a', 'b'])
    // 매칭 없음.
    expect(filterMonitoringCandidates(list, 'zzz')).toEqual([])
  })

  it('실행중 없으면 빈 배열(검색어 무관)', () => {
    const list = [agent('a', 'C:/x', { type: 'Killed' })]
    expect(filterMonitoringCandidates(list, '')).toEqual([])
    expect(filterMonitoringCandidates(list, 'x')).toEqual([])
  })

  it('입력 순서를 보존한다(재정렬 없음)', () => {
    const list = [agent('z', 'C:/work/zed'), agent('a', 'C:/work/apex')]
    expect(filterMonitoringCandidates(list, '').map(c => c.id)).toEqual(['z', 'a'])
  })
})
