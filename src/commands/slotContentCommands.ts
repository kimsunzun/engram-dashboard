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
import { useMonitoringPickerStore } from '../store/monitoringPickerStore'
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

// ADR-0067: slot 우클릭 "에이전트 모니터링" — 검색 팝업(실행중 에이전트)을 우클릭한 그 slot 을 타깃으로 연다.
register({
  id: 'slot.assignRunningAgent',
  title: '에이전트 모니터링',
  category: 'slot',
  // ★배치 타깃 = 우클릭한 slot(명시)★(ADR-0067): ctx.viewId/slotId 가 우클릭한 slot 좌표다(포커스 비의존
  //   — focus-steal 원천 차단). 여기선 팝업을 그 좌표로 열기만 하고, 실제 배치(assign_agent)는 팝업의
  //   on-select 가 viewStore.assignAgent 로 흘린다(§5 단일 제어 표면 — 별도 배치 상태 없음). 팝업 열기 자체는
  //   focused_slot_id 를 건드리지 않는다(우클릭 포커스 불변식). requireCoords 로 좌표를 검증(fail-loud).
  run: args => {
    const { viewId, slotId } = requireCoords(args, 'slot.assignRunningAgent')
    useMonitoringPickerStore.getState().open(viewId, slotId)
  },
})

// empty 슬롯 전용 기여(group='content' — 공통 slot-ops 위에 렌더).
// ★"새 콘텐츠 ▶" 1단 서브메뉴(ADR-0065)★: 콘텐츠-채움 항목(트리·팔레트)을 컨테이너 하나로 접어
//   빈 슬롯 메뉴를 정돈한다("이 칸에 뭘 넣나" = 콘텐츠 평면 분리). 자식은 기존 상대 순서(트리→팔레트)
//   유지. command 는 registry 단일소스로 그대로 직접 호출 가능(§5 불변) — 서브메뉴는 presentation 일 뿐이다.
//   향후 백엔드 타입(codex/gemini) 추가 시 자식으로 붙는 확장 자리.
// ★ADR-0067: "생성"(slot.createAgentHere) 제거★ — 스폰은 트리 소관으로 이관(reserved 프로필 더블클릭 +
//   agent_list 슬롯 메뉴 agentlist.createAgent). command 정의 자체는 남긴다(직접 호출·향후 재사용 가능).
registerSlotMenu('empty', [
  {
    title: '새 콘텐츠',
    group: 'content',
    order: 10,
    children: [
      { commandId: 'slot.fill.agentList', group: 'content', order: 10 },
      { commandId: 'slot.fill.presetPalette', group: 'content', order: 20 },
    ],
  },
])

// ADR-0067: "에이전트 모니터링" 기여 — empty·agent 슬롯에 노출(우클릭한 slot 에 실행중 에이전트 배정).
//   ★hideOn = ['agent_list','preset_palette']★: 소스 슬롯(트리·팔레트)에서 "여기에 에이전트를 모니터링"
//   은 무의미하다(그 슬롯은 콘텐츠 자체가 소스 UI). '*' 보편 등록으로 empty·agent 를 함께 덮되 소스 두
//   타입만 subtraction 으로 뺀다(ADR-0065 hideOn — allowlist 아님, 공통 단일소스 유지). group='content'.
registerSlotMenu('*', [
  {
    commandId: 'slot.assignRunningAgent',
    group: 'content',
    order: 5,
    hideOn: ['agent_list', 'preset_palette'],
  },
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
