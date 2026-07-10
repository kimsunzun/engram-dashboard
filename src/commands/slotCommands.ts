// ADR-0064 / ADR-0055 / ADR-0035: 공통 슬롯 ops command 어댑터 + '*' 기여(중앙 1파일).
//
// ★역할★: 옛 SlotContextMenu 하드코딩(가로/세로 분할·팝업 분리·비우기·닫기)을 registry command 로 승격한다.
//   각 command 는 실행 컨텍스트(viewId·slotId)를 run(args) 로 받아 viewStore 액션으로 라우팅만 한다(새 상태
//   경로 0 — 레이아웃 권위는 백엔드 ViewManager, ADR-0035). 사람 우클릭·팔레트·키바인딩·LLM(__engramCmd)이
//   모두 같은 command 를 컨텍스트 인자로 실행한다(§5 단일 제어 표면).
//
// ★공통 = '*' 단일소스★(ADR-0064 불변식): 이 항목들은 registerSlotMenu('*', …) 로 모든 슬롯에 붙는다 —
//   콘텐츠 컴포넌트가 재선언하지 않는다(재선언 = drift, 리뷰 reject). 콘텐츠 전용 항목은 각 콘텐츠 모듈에서.
//
// ★팝업 분리 = 공통으로 승격(ADR-0064)★: 옛 메뉴는 '팝업으로 분리'를 라이브 agent 有일 때만 활성화했지만,
//   슬롯 콘텐츠 유니온(ADR-0060) 이후엔 콘텐츠 종류와 무관하게 슬롯을 다른 창으로 옮길 수 있어야 한다 →
//   agent 게이팅을 제거하고 '*'(공통)으로 옮긴다.
//
// import 부수효과로 등록되므로 단일 매니페스트(contributions.ts)에서 side-effect import 한다.

import { useViewStore } from '../store/viewStore'
import type { SplitDir } from '../api/layoutTypes'
import { register } from './registry'
import { registerSlotMenu } from './slotMenu'

/** 공통 slot-op command 의 실행 컨텍스트 인자(단일 가방, ADR-0055). viewId·slotId 필수. */
interface SlotCtx {
  viewId?: unknown
  slotId?: unknown
}

/** args 에서 (viewId, slotId) 를 검증해 뽑는다 — 둘 다 문자열이어야 한다(백엔드 권위 좌표계). */
function requireCoords(args: SlotCtx | undefined, cmd: string): { viewId: string; slotId: string } {
  const viewId = args?.viewId
  const slotId = args?.slotId
  if (typeof viewId !== 'string' || viewId.length === 0) throw new Error(`[${cmd}] viewId 필요`)
  if (typeof slotId !== 'string' || slotId.length === 0) throw new Error(`[${cmd}] slotId 필요`)
  return { viewId, slotId }
}

function registerSplit(id: string, title: string, dir: SplitDir): void {
  register({
    id,
    title,
    category: 'slot',
    // ADR-0035: 분할 = viewStore.split(viewId, slotId, dir) → invoke(split_slot) → emit 반영(낙관 갱신 X).
    run: args => {
      const { viewId, slotId } = requireCoords(args, id)
      return useViewStore.getState().split(viewId, slotId, dir)
    },
  })
}

registerSplit('slot.split.h', '가로 분할', 'horizontal')
registerSplit('slot.split.v', '세로 분할', 'vertical')

register({
  id: 'slot.focus',
  title: '포커스',
  category: 'slot',
  // ADR-0066: click-to-focus — 슬롯 pane 클릭·팔레트·키바인딩·LLM(__engramCmd)이 모두 이 command 를 통해
  //   viewStore.focusSlot → invoke(focus_slot) → emit(layout:updated) 로 링을 갱신한다(낙관 갱신 X, §5 단일
  //   제어 표면). ViewLayoutRenderer 의 pane onClick 도 같은 viewStore.focusSlot 을 부른다(동일 핸들).
  run: args => {
    const { viewId, slotId } = requireCoords(args, 'slot.focus')
    return useViewStore.getState().focusSlot(viewId, slotId)
  },
})

register({
  id: 'slot.popout',
  title: '팝업으로 분리',
  category: 'slot',
  // ADR-0057/0064: 슬롯을 새 런타임 팝업 창의 새 탭으로 MOVE(detach) — viewStore.moveSlotToWindow →
  //   invoke(move_slot_to_window, toWindow=null). 콘텐츠 종류 무관(agent 게이팅 제거, ADR-0064).
  run: args => {
    const { viewId, slotId } = requireCoords(args, 'slot.popout')
    return useViewStore.getState().moveSlotToWindow(viewId, slotId)
  },
})

register({
  id: 'slot.empty',
  title: '비우기',
  category: 'slot',
  // ADR-0063: 슬롯 콘텐츠를 empty 로 교체 = viewStore.setSlotContent(…,{type:'empty'}) → invoke → emit.
  run: args => {
    const { viewId, slotId } = requireCoords(args, 'slot.empty')
    return useViewStore.getState().setSlotContent(viewId, slotId, { type: 'empty' })
  },
})

register({
  id: 'slot.close',
  title: '닫기',
  category: 'slot',
  // ADR-0035: 닫기 = viewStore.closeSlot(viewId, slotId) → invoke(close_slot)(형제 승격).
  run: args => {
    const { viewId, slotId } = requireCoords(args, 'slot.close')
    return useViewStore.getState().closeSlot(viewId, slotId)
  },
})

// ★공통 슬롯 ops 기여 = '*'(모든 슬롯) 단일소스★(ADR-0064). group='slot-ops'(콘텐츠 항목 아래에 렌더).
//   order 는 분할→팝업→비우기→닫기 순(닫기는 관례상 맨 아래라 99).
// ★hideOn 트림(ADR-0065)★: slot.popout(빈 칸 팝아웃 실익 낮음)·slot.empty(빈 슬롯 재비우기 = no-op)는
//   빈 슬롯에서 무의미하므로 hideOn:['empty'] 로 제외한다. '*' 보편 등록은 유지(공통 ops 단일소스 불변식) —
//   콘텐츠 타입별 재선언이 아니라 subtraction 필터일 뿐이다(ADR-0065 거부 대안 참조).
registerSlotMenu('*', [
  { commandId: 'slot.split.h', group: 'slot-ops', order: 10 },
  { commandId: 'slot.split.v', group: 'slot-ops', order: 20 },
  { commandId: 'slot.popout', group: 'slot-ops', order: 30, hideOn: ['empty'] },
  { commandId: 'slot.empty', group: 'slot-ops', order: 40, hideOn: ['empty'] },
  { commandId: 'slot.close', group: 'slot-ops', order: 99 },
])
