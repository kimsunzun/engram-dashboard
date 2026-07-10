// ADR-0064 / ADR-0060 / ADR-0011: 코어 SlotContent 타입(empty·agent) 전용 command + 메뉴 기여 co-location.
//
// ★역할★: empty·agent 는 별도 feature 모듈이 없는 코어 콘텐츠 타입이라(preset_palette=presetCommands,
//   agent_list=agentCommands 처럼 각자 모듈이 있는 것과 달리) 여기 한 곳에 그 전용 command + 기여를 모은다.
//   - empty 슬롯 fill-ops(트리/팔레트 열기·에이전트 생성해 이 슬롯에 배정) → target='empty'
//   - agent 슬롯 종료(kill) → target='agent'
//   모든 항목은 command id 참조로만 메뉴에 붙는다(ADR-0064 불변식 — 메뉴 직접 store 호출 금지).
//
// import 부수효과로 등록되므로 단일 매니페스트(contributions.ts)에서 side-effect import 한다.

import { open } from '@tauri-apps/plugin-dialog'

import { agentClient } from '../api/clientFactory'
import { useViewStore } from '../store/viewStore'
import { register } from './registry'
import { registerSlotMenu } from './slotMenu'

/** (viewId, slotId) 검증 헬퍼 — fill-ops 는 모두 이 좌표로 setSlotContent/assign 한다(백엔드 권위). */
function requireCoords(
  args: { viewId?: unknown; slotId?: unknown } | undefined,
  cmd: string,
): { viewId: string; slotId: string } {
  const viewId = args?.viewId
  const slotId = args?.slotId
  if (typeof viewId !== 'string' || viewId.length === 0) throw new Error(`[${cmd}] viewId 필요`)
  if (typeof slotId !== 'string' || slotId.length === 0) throw new Error(`[${cmd}] slotId 필요`)
  return { viewId, slotId }
}

// ── empty 슬롯 fill-ops ────────────────────────────────────────────────────────

register({
  id: 'slot.fill.agentList',
  title: '에이전트 트리 열기',
  category: 'slot',
  // ADR-0063: 이 슬롯 콘텐츠를 agent_list 로 교체 → invoke(set_slot_content) → emit 반영.
  run: args => {
    const { viewId, slotId } = requireCoords(args, 'slot.fill.agentList')
    return useViewStore.getState().setSlotContent(viewId, slotId, { type: 'agent_list' })
  },
})

register({
  id: 'slot.fill.presetPalette',
  title: '프리셋 팔레트 열기',
  category: 'slot',
  run: args => {
    const { viewId, slotId } = requireCoords(args, 'slot.fill.presetPalette')
    return useViewStore.getState().setSlotContent(viewId, slotId, { type: 'preset_palette' })
  },
})

register({
  id: 'slot.createAgentHere',
  title: '에이전트 생성',
  category: 'slot',
  // ★spawn + 이 슬롯에 배정★(ADR-0011 + ADR-0035): 네이티브 폴더 다이얼로그로 cwd 를 고른 뒤(preset.add·
  //   PresetPalette 와 동일 open({directory:true}) 패턴) agentClient.spawnAgent(cwd)(데몬 권위) → 성공 id 를
  //   assignAgent(viewId, slotId, id)(레이아웃 권위 invoke)로 이 빈 슬롯에 배정한다. 두 권위가 분리돼 있다.
  //   취소(null)면 no-op. async — 호출부(cdp/메뉴)가 await 가능하게 Promise 를 흘려보낸다.
  run: async args => {
    const { viewId, slotId } = requireCoords(args, 'slot.createAgentHere')
    // 네이티브 OS 폴더 선택 창(webview 밖). directory+multiple:false → 반환은 string | null.
    const picked = await open({ directory: true, multiple: false, title: '에이전트 작업 디렉토리 선택' })
    const cwd = typeof picked === 'string' ? picked : null
    if (!cwd) return // 취소 — no-op
    const agent = await agentClient.spawnAgent(cwd)
    // ADR-0035: 배정도 백엔드 권위 invoke(assign_agent) — 낙관 갱신 없이 emit 으로 반영.
    return useViewStore.getState().assignAgent(viewId, slotId, agent.id)
  },
})

// empty 슬롯 전용 기여(group='content' — 공통 slot-ops 위에 렌더). 트리·팔레트·생성 순.
registerSlotMenu('empty', [
  { commandId: 'slot.fill.agentList', group: 'content', order: 10 },
  { commandId: 'slot.fill.presetPalette', group: 'content', order: 20 },
  { commandId: 'slot.createAgentHere', group: 'content', order: 30 },
])

// ── agent 슬롯 전용 ────────────────────────────────────────────────────────────

register({
  id: 'agent.kill',
  title: '에이전트 종료',
  category: 'agent',
  // ADR-0011: 종료 = 에이전트 명령(agentClient.killAgent). 슬롯은 그대로 두고(레이아웃 불변) agent 만 kill.
  //   agentId 는 실행 컨텍스트(ctx)에서 온다 — 메뉴가 슬롯의 배정 agentId 를 넘긴다.
  run: args => {
    const agentId = args?.agentId
    if (typeof agentId !== 'string' || agentId.length === 0) throw new Error('[agent.kill] agentId 필요')
    return agentClient.killAgent(agentId)
  },
})

registerSlotMenu('agent', [{ commandId: 'agent.kill', group: 'content', order: 10 }])
