// ADR-0067: 에이전트 모니터링 검색 팝업의 열림/타깃 상태(프론트 전용 UI 상태).
//
// ★역할★: slot 우클릭 "에이전트 모니터링" → 검색 팝업이 어느 slot 을 타깃으로 열려 있는지만 담는다.
//   배치 자체(assign_agent)는 여기 담지 않는다 — 배치 코어는 viewStore.assignAgent 하나(§5 단일 제어
//   표면, ADR-0067). 이 스토어는 "팝업이 열려 있고 타깃이 (viewId, slotId) 다"라는 순수 표시 상태만
//   보유한다(별도 배치 상태 금지 — ADR-0067 불변식).
//
// ★타깃 = 우클릭한 slot(명시)★: open(viewId, slotId) 에 좌표를 그대로 실어 배치가 포커스에 의존하지
//   않게 한다(focus-steal 원천 차단, ADR-0067). 우클릭은 focused_slot_id 를 건드리지 않으므로 이
//   스토어도 포커스와 무관하다.
//
// ★창별 인스턴스★: zustand 모듈 스토어라 각 웹뷰 창(WebView2)이 자기 인스턴스를 갖는다 — 팝업을 연
//   창에서만 picker 가 뜬다(command 는 우클릭한 slot 을 소유한 그 창에서 실행되므로 일치한다).

import { create } from 'zustand'

/** 팝업이 타깃하는 slot 좌표(우클릭한 slot). 닫혀 있으면 null. */
export interface MonitoringPickerTarget {
  /** 우클릭한 slot 이 속한 view id(WindowLayout 이 넘긴 탭 오버라이드에서 옴). */
  viewId: string
  /** 우클릭한 slot 의 node id(SlotContent 배정 대상). */
  slotId: string
}

interface MonitoringPickerState {
  /** 열려 있으면 타깃 좌표, 닫혀 있으면 null. picker 는 이 값으로 마운트 여부를 정한다. */
  target: MonitoringPickerTarget | null
  /** open() 호출마다 +1 되는 단조 증가 카운터. WindowLayout 이 `key={openId}` 로 내려꽂아
   *  매 open 마다 AgentMonitoringPicker 를 fresh remount 한다(stale query/activeIndex 플래시 방지, ADR-0067).
   *  닫힘 상태에서도 값이 보존되어 다음 open 때 key 가 바뀐다는 점이 핵심이다. */
  openId: number
  /** 팝업 열기 — 우클릭한 slot 좌표를 실어 타깃을 고정한다(§5 command 에서 호출). */
  open: (viewId: string, slotId: string) => void
  /** 팝업 닫기(선택·Esc·backdrop). 타깃을 비운다. */
  close: () => void
}

export const useMonitoringPickerStore = create<MonitoringPickerState>((set, get) => ({
  target: null,
  openId: 0,
  open: (viewId, slotId) => set({ target: { viewId, slotId }, openId: get().openId + 1 }),
  close: () => set({ target: null }),
}))
