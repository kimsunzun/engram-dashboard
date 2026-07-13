// ADR-0067: 에이전트 모니터링 팝업의 실행중-필터 + 검색 로직(PURE — DOM/Tauri 의존 0).
//
// ★역할★: AgentMonitoringPicker 가 그릴 후보 목록을 순수 함수로 파생한다 — ① 실행중(Running)만 남기고
//   ② 검색어(표시명·cwd 부분일치)로 좁힌다. 순수라 headless(vitest)로 단위테스트되고(AgentList 렌더는
//   CDP 실측), 표시명은 트리(AgentList/mergeTreeNodes)와 동일 로직으로 파생한다.
//
// ★표시명 = profile.display_name(ADR-0061 — 트리와 동일 출처)★: 예전엔 cwd basename 이 유일 출처였으나,
//   트리 rename(ADR-0061)으로 표시명 override 가 프로필에 생겼다. 여기도 트리와 같게 id 로 프로필을 조인해
//   display_name 을 우선한다 — 안 그러면 "ABC"로 rename 한 에이전트가 팝업에선 cwd basename(예: "Filter
//   Library")으로 떠 트리와 어긋나고 같은 cwd 에이전트끼리 헷갈린다. 폴백 = display_name(override) →
//   basename(cwd). ★profile.name 은 안 씀★ — 이 앱 프로필은 name 이 cwd 문자열이라 전체 경로가 뜬다(트리도
//   display_name ?? basename 만). display_name·프로필은 AgentInfo wire 엔 없어 AgentProfile 에서만 온다
//   (mergeTreeNodes 와 동형 조회 — profile.id === agent.id).
//
// ★"실행중" = status.type === 'Running'★: AgentInfo.status 는 태그드 유니온({type:'Running'} 등)이라
//   문자열 비교가 아니라 태그(type)로 판정한다(types.ts AgentStatus). Exiting/Exited/Killed/Failed 는
//   모니터링 대상이 아니라 제외한다(라이브 관측이 목적).

import type { AgentInfo, AgentProfile } from '../../api/types'
import { basename } from '../../util/basename'

/** 팝업 행 1개 — 표시명 + 배정에 쓸 agentId + 원본 cwd(툴팁). */
export interface MonitoringCandidate {
  id: string
  /**
   * 표시명 = profile.display_name(ADR-0061 — 트리와 동일 출처, id 로 조인). override 없으면 cwd basename 으로
   * 폴백(프론트 파생, AgentList mergeTreeNodes 와 동일 — profile.name 은 cwd 문자열이라 미사용).
   */
  name: string
  /** 전체 cwd(muted 보조 표기·title·검색 대상). */
  cwd: string
}

/** 실행중(Running) 에이전트만 남긴다. 종료/실패/전이 상태는 모니터링 대상이 아니라 제외. */
export function runningAgents(agents: AgentInfo[]): AgentInfo[] {
  return agents.filter(a => a.status.type === 'Running')
}

/**
 * 실행중 에이전트를 검색어로 좁혀 후보 목록으로 파생한다(대소문자 무시 부분일치).
 *   - query 가 비면 실행중 전체를 후보로 낸다.
 *   - 표시명은 id 로 프로필을 조인해 파생한다(ADR-0061 — display_name → basename(cwd), profile.name 미사용).
 *   - 매칭 대상 = 파생 표시명 + 전체 cwd(경로 일부로도 찾게).
 *   - 정렬은 입력 순서 보존(호출부가 이미 결정적 순서로 넘긴다 — AgentList mergeTreeNodes 와 별개 경로라
 *     여기선 재정렬하지 않는다: agents 배열은 setAgents 교체분 그대로).
 */
export function filterMonitoringCandidates(
  agents: AgentInfo[],
  profiles: AgentProfile[],
  query: string,
): MonitoringCandidate[] {
  const q = query.trim().toLowerCase()
  // ADR-0061: 표시명 override(display_name)·프로필 기본명은 AgentProfile 에만 있다(AgentInfo wire 엔 없음).
  //   트리와 동일하게 id→profile 맵으로 조인한다(profile.id === agent.id, spawn 후 불변).
  const profileById = new Map(profiles.map(p => [p.id, p]))
  return runningAgents(agents)
    .map(a => {
      const profile = profileById.get(a.id)
      // 폴백 = display_name override → cwd basename. ★트리(AgentList)와 동일 사슬★: profile.name 은 쓰지
      //   않는다 — 이 앱의 프로필은 name 이 cwd 문자열(createClaudeProfile name=cwd)이라 전체 경로가 떠
      //   basename 보다 나쁘다. 트리도 display_name ?? basename(cwd) 로만 렌더한다(profile.name 미사용).
      const name = profile?.display_name ?? basename(a.cwd)
      return { id: a.id, name, cwd: a.cwd }
    })
    .filter(c => {
      if (q.length === 0) return true
      return c.name.toLowerCase().includes(q) || c.cwd.toLowerCase().includes(q)
    })
}
