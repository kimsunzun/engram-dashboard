// ★표시명 = profile.display_name(ADR-0061 — 트리와 동일 출처)★: 예전엔 cwd basename 이 유일 출처였으나,
//   트리 rename(ADR-0061)으로 표시명 override 가 프로필에 생겼다. 트리와 같게 안 하면 "ABC"로 rename 한
//   에이전트가 팝업에선 cwd basename(예: "Filter Library")으로 떠 트리와 어긋나고 같은 cwd 에이전트끼리
//   헷갈린다. ★profile.name 은 안 씀★ — 이 앱 프로필은 name 이 cwd 문자열이라 전체 경로가 뜬다(트리도
//   display_name ?? basename 만). display_name·프로필은 AgentInfo wire 엔 없어 AgentProfile 에서만 온다.
//
// ★"실행중"★: Exiting/Exited/Killed/Failed 는 모니터링 대상이 아니라 제외한다(라이브 관측이 목적).

import type { AgentInfo, AgentProfile } from '../../api/types'
import { basename } from '../../util/basename'

export interface MonitoringCandidate {
  id: string
  /**
   * 표시명 = profile.display_name(ADR-0061 — 트리와 동일 출처, id 로 조인). override 없으면 cwd basename 으로
   * 폴백(프론트 파생, AgentList mergeTreeNodes 와 동일 — profile.name 은 cwd 문자열이라 미사용).
   */
  name: string
  cwd: string
}

/** 종료/실패/전이 상태는 모니터링 대상이 아니라 제외. */
export function runningAgents(agents: AgentInfo[]): AgentInfo[] {
  return agents.filter(a => a.status.type === 'Running')
}

/**
 * 정렬은 입력 순서 보존(호출부가 이미 결정적 순서로 넘긴다 — AgentList mergeTreeNodes 와 별개 경로라
 * 여기선 재정렬하지 않는다: agents 배열은 setAgents 교체분 그대로).
 */
export function filterMonitoringCandidates(
  agents: AgentInfo[],
  profiles: AgentProfile[],
  query: string,
): MonitoringCandidate[] {
  const q = query.trim().toLowerCase()
  // ADR-0061: profile.id === agent.id 는 spawn 후 불변.
  const profileById = new Map(profiles.map(p => [p.id, p]))
  return runningAgents(agents)
    .map(a => {
      const profile = profileById.get(a.id)
      // ★트리(AgentList)와 동일 사슬★: profile.name 은 쓰지 않는다 — 이 앱의 프로필은 name 이 cwd
      //   문자열(createClaudeProfile name=cwd)이라 전체 경로가 떠 basename 보다 나쁘다.
      const name = profile?.display_name ?? basename(a.cwd)
      return { id: a.id, name, cwd: a.cwd }
    })
    .filter(c => {
      if (q.length === 0) return true
      return c.name.toLowerCase().includes(q) || c.cwd.toLowerCase().includes(q)
    })
}
