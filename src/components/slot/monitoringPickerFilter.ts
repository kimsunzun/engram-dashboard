// ADR-0067: 에이전트 모니터링 팝업의 실행중-필터 + 검색 로직(PURE — DOM/Tauri 의존 0).
//
// ★역할★: AgentMonitoringPicker 가 그릴 후보 목록을 순수 함수로 파생한다 — ① 실행중(Running)만 남기고
//   ② 검색어(cwd basename 부분일치)로 좁힌다. 순수라 headless(vitest)로 단위테스트되고(AgentList 렌더는
//   CDP 실측), 표시명은 공용 basename 유틸로 파생해 AgentList/PresetPalette 와 단일 출처를 공유한다.
//
// ★"실행중" = status.type === 'Running'★: AgentInfo.status 는 태그드 유니온({type:'Running'} 등)이라
//   문자열 비교가 아니라 태그(type)로 판정한다(types.ts AgentStatus). Exiting/Exited/Killed/Failed 는
//   모니터링 대상이 아니라 제외한다(라이브 관측이 목적).

import type { AgentInfo } from '../../api/types'
import { basename } from '../../util/basename'

/** 팝업 행 1개 — 표시명(basename) + 배정에 쓸 agentId + 원본 cwd(툴팁). */
export interface MonitoringCandidate {
  id: string
  /** 표시명 = cwd basename(프론트 파생 — 이름 미저장, AgentList 와 단일 출처). */
  name: string
  /** 전체 cwd(muted 보조 표기·title). */
  cwd: string
}

/** 실행중(Running) 에이전트만 남긴다. 종료/실패/전이 상태는 모니터링 대상이 아니라 제외. */
export function runningAgents(agents: AgentInfo[]): AgentInfo[] {
  return agents.filter(a => a.status.type === 'Running')
}

/**
 * 실행중 에이전트를 검색어로 좁혀 후보 목록으로 파생한다(대소문자 무시 부분일치).
 *   - query 가 비면 실행중 전체를 후보로 낸다.
 *   - 매칭 대상 = 표시명(basename) + 전체 cwd(경로 일부로도 찾게).
 *   - 정렬은 입력 순서 보존(호출부가 이미 결정적 순서로 넘긴다 — AgentList mergeTreeNodes 와 별개 경로라
 *     여기선 재정렬하지 않는다: agents 배열은 setAgents 교체분 그대로).
 */
export function filterMonitoringCandidates(agents: AgentInfo[], query: string): MonitoringCandidate[] {
  const q = query.trim().toLowerCase()
  return runningAgents(agents)
    .map(a => ({ id: a.id, name: basename(a.cwd), cwd: a.cwd }))
    .filter(c => {
      if (q.length === 0) return true
      return c.name.toLowerCase().includes(q) || c.cwd.toLowerCase().includes(q)
    })
}
