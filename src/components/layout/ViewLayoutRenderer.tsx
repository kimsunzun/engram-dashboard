// ViewLayoutRenderer — 백엔드 권위 레이아웃(ViewManager) 트리를 그리는 메인 캔버스 렌더러(ADR-0035).
//
// ★유일한 레이아웃 렌더러★(Brick 1): 옛 프론트 전용 slotStore/LayoutRenderer(number id + content union)는
// 제거됐다. 이 렌더러는 wire LayoutNode(string UUID id + content: SlotContent, ADR-0060, src-tauri/bindings)만 그린다 —
// 사람 우클릭(SlotContextMenu)이든 LLM(window.__engramLayout)이든 같은 invoke→emit 권위 루프로 갱신된다.

import { useState } from 'react'
import { Allotment } from 'allotment'

import type { LayoutNode } from '../../api/layoutTypes'
import { useCurrentViewId, useViewStore } from '../../store/viewStore'
import { useAgentStore } from '../../store/agentStore'
import TerminalSlot from '../slot/TerminalSlot'
import RichSlot from '../slot/RichSlot'
import DomSlot from '../slot/DomSlot'
import PresetPalette from '../slot/PresetPalette'
import AgentList from '../agent/AgentList'
import SlotContextMenu from '../slot/SlotContextMenu'
import { buildSlotMenu } from '../../commands/slotMenu'
import { defaultRenderMode } from '../slot/renderMode'

export default function ViewLayoutRenderer({
  node,
  focusedSlotId,
  viewIdOverride,
}: {
  node: LayoutNode
  focusedSlotId: string | null
  // ★이 렌더러가 그리는 View id 오버라이드(선택).★ WindowLayout(main·팝업)이 각 탭 캔버스에 그 탭 view 를
  //   넘겨(ADR-0057) 내부 SlotContextMenu 의 액션 좌표를 그 탭 view 로 고정한다. 없으면 메뉴가
  //   useCurrentViewId(이 웹뷰 창의 active 탭) 폴백. 재귀 split 렌더에도 전파해 하위 슬롯 메뉴까지 같은 view.
  viewIdOverride?: string | null
}) {
  // ★렌더 모드 오버라이드(§5)★: caps 유도 기본(defaultRenderMode) 대신 강제할 slot node.id → RenderMode.
  const renderModeOverride = useViewStore(s => s.renderModeOverride)
  // ★M2 caps 분기(ADR-0044)★: agent 배정 슬롯의 렌더러는 그 agent 의 output caps 로 고른다.
  // structured(NDJSON 캐리어=StdioTransport)면 라이브 RichSlot, 아니면 TerminalSlot(xterm). caps 는
  // AgentInfo 로 이미 wire 를 건너와 store 에 있다(M1) — 여기선 조회만(추가 배선 불필요).
  const agents = useAgentStore(s => s.agents)
  // ★우클릭 슬롯 메뉴 상태(§5)★: 슬롯 하나당 이 렌더러 인스턴스가 하나라(재귀 렌더) 여기 useState 는
  //   그 슬롯 전용 메뉴 좌표다. 열림 시 SlotContextMenu 를 이 렌더러 안에서 직접 마운트한다 — 옛
  //   LayoutRenderer→SlotPane 래핑 경로가 Brick 1 에서 삭제돼 메뉴가 캔버스에서 닿지 않던 갭을 메운다.
  //   ★hooks 무조건 호출★: split/slot 분기 이전에 부른다(조건부 호출 금지).
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number } | null>(null)
  // ★이 메뉴가 조작할 View 좌표(ADR-0064)★: WindowLayout 이 넘긴 탭 오버라이드가 있으면 그걸, 없으면 이
  //   웹뷰 창의 active 탭(useCurrentViewId, §3-4). command.run(ctx) 의 viewId 로 흘러 분할/닫기/배정이 이
  //   좌표를 쓴다. 옛 SlotContextMenu 내부 폴백을 여기(ctx 조립처)로 끌어올렸다.
  const currentViewId = useCurrentViewId()
  const targetViewId = viewIdOverride ?? currentViewId

  if (node.type === 'slot') {
    const isFocused = node.id === focusedSlotId
    // ADR-0060: 슬롯 점유자 = SlotContent 태그드 유니온. Agent variant 만 실 렌더러가 붙는다(Empty = 플레이스홀더).
    const slotAgentId = node.content.type === 'agent' ? node.content.agent_id : null
    const agent = slotAgentId != null ? (agents.find(a => a.id === slotAgentId) ?? null) : null
    // ★caps 도착 후에만 구체 렌더러를 마운트한다(ADR-0041 replay 소유권)★: 데몬 replay 는 slot-assign
    //   델타((window,agent) 키)에서 단 1회만 발화하고, 컴포넌트 스왑(TerminalSlot→RichSlot)엔 재발화하지
    //   않는다. 그래서 caps 미도착 상태에서 TerminalSlot 을 먼저 띄웠다가 caps 도착 후 RichSlot 으로 갈아끼면,
    //   스왑된 RichSlot 이 빈 채로 마운트돼 스왑 전 바이트가 영구 유실된다. 대신 caps(=AgentInfo) 도착 전엔
    //   중립 플레이스홀더만 두고(아래 '에이전트 연결 중…'), 첫 구체 렌더러를 caps 확정 후 마운트해 assign
    //   시점 replay 를 온전히 받게 한다. (터미널 에이전트는 보통 assign 전에 AgentInfo 가 오므로 이 플레이스
    //   홀더는 일시적 엣지 상태다 — 터미널 replay 경로는 종전과 동일.)
    // 구조화 출력(NDJSON) = 라이브 RichSlot, 아니면 TerminalSlot(xterm) 분기 근거(ADR-0002/0044).
    const capsReady = slotAgentId != null && agent != null
    // caps-ready 슬롯의 렌더러: 오버라이드가 있으면 그걸, 없으면 caps 에서 유도한 기본(defaultRenderMode).
    // agent 는 capsReady 분기(아래 agent != null)에서만 사용되므로 여기선 null 병합만 걸어둔다.
    const mode = agent != null ? (renderModeOverride[node.id] ?? defaultRenderMode(agent)) : null
    // hasContent = 구체 렌더러를 그리는 경우만(래퍼를 100% 채움). caps 대기 플레이스홀더는 empty 슬롯처럼
    // 중앙정렬 스타일로 둔다(hasContent=false). ADR-0060/0061: preset_palette variant 도 슬롯을 100% 채우는
    // 실 렌더러(PresetPalette)라 hasContent=true(중앙정렬 스타일이 팔레트 레이아웃을 깨지 않게).
    // preset_palette·agent_list variant 도 슬롯을 100% 채우는 실 렌더러라 hasContent=true(중앙정렬
    //   플레이스홀더 스타일이 이들 레이아웃을 깨지 않게, ADR-0060/0061/0062).
    const isPresetPalette = node.content.type === 'preset_palette'
    const isAgentList = node.content.type === 'agent_list'
    const hasContent = capsReady || isPresetPalette || isAgentList
    // ★ADR-0046: 버그 B 구조 해소★: ProtocolClient.subs 가 이제 viewId(slot id) 키라 같은 agentId 를 두
    //   슬롯에 배정해도 각 슬롯이 독립 구독·독립 진도를 갖는다(옛 agentId-당-단일-콜백 덮어쓰기 소멸).
    //   슬롯은 아래 viewId={node.id} 로 자기 slot id 를 구독 키로 넘긴다.
    return (
      <div
        style={{
          height: '100%',
          background: 'var(--bg)',
          // border 폭을 항상 1px 고정해 포커스 이동 시 layout shift 제거.
          // 포커스 표시는 inset box-shadow(65% 반투명 accent) — 세 테마 모두 color-mix 로 자동 적응.
          //   ★강도 65%★: GUI 에디터의 너무 약한 포커스 표시가 반복 UX 불만이라(VS Code #24586 등, /research)
          //   "은은하되 확실히 식별"되게 40%→65%로 올림(사용자 결정). WebView2 최신 Chromium = color-mix 지원.
          border: '1px solid var(--border)',
          boxShadow: isFocused ? 'inset 0 0 0 1px color-mix(in srgb, var(--accent) 65%, transparent)' : 'none',
          boxSizing: 'border-box',
          // 콘텐츠(터미널/rich) 있을 때: 슬롯을 100% 채우도록 여백·정렬 제거(center 정렬 끼면 깨짐).
          // 빈 슬롯(empty): 플레이스홀더를 중앙정렬하는 flex 유지.
          ...(hasContent
            ? { overflow: 'hidden' }
            : {
                display: 'flex',
                flexDirection: 'column',
                alignItems: 'center',
                justifyContent: 'center',
                color: 'var(--text-muted)',
                fontFamily: 'var(--font-ui)',
                fontSize: '12px',
                gap: '4px',
              }),
        }}
        // 슬롯 식별용 data 속성 — cdp eval 에서 DOM 으로 split 결과(슬롯 수)를 셀 수 있게.
        data-slot-id={node.id}
        // ADR-0066: click-to-focus — 슬롯 pane 클릭 시 이 슬롯을 포커스로 지정한다. viewStore.focusSlot →
        //   invoke(focus_slot) → emit(layout:updated) 단일 제어 표면(사람 클릭 = LLM = slot.focus command, §5).
        //   ★낙관 갱신 X★: 링(isFocused)은 백엔드 emit 스냅샷으로만 갱신된다(권위 = src-tauri, ADR-0035).
        //   ★버블 허용(stopPropagation/preventDefault 안 함)★: 내부 상호작용(터미널 포커스·AgentList 버튼 등)을
        //   가로채지 않는다 — pane 어디를 눌러도 그 슬롯이 focus 되고 내부 핸들러도 그대로 발화한다.
        //   targetViewId 미확정(부팅 직후 탭 상태 미도착)이면 no-op(잘못된 view 로 focus 유출 방지).
        onClick={() => {
          if (targetViewId) void useViewStore.getState().focusSlot(targetViewId, node.id)
        }}
        // ADR-0035: 우클릭 → SlotContextMenu 마운트. 메뉴 액션(분할/닫기/배정)은 viewStore(=window.__engramLayout)
        //   단일 제어 표면으로만 흐른다(사람 클릭 = LLM 이 한 표면, §5). 기본 컨텍스트 메뉴는 막는다.
        onContextMenu={e => {
          e.preventDefault()
          setContextMenu({ x: e.clientX, y: e.clientY })
        }}
      >
        {node.content.type === 'agent' ? (
          agent == null ? (
            // ★caps-ready 게이팅(replay 소유권)★: caps(AgentInfo) 미도착 시 중립 플레이스홀더만(위 replay
            // 소유권 주석 참조). 구체 렌더러(DomSlot/RichSlot/TerminalSlot) 마운트를 caps 확정까지 미뤄
            // assign 시점 replay 를 온전히 받게 한다. 래퍼의 empty 슬롯 스타일(중앙정렬·muted)을 상속.
            <span>에이전트 연결 중…</span>
          ) : (
            // caps 도착 후에만 도달 — mode 는 여기서 non-null(위 defaultRenderMode ?? 오버라이드).
            // 오버라이드가 있으면 그 렌더러, 없으면 caps 유도 기본. 이 switch 는 위 caps-ready 게이팅 안에
            // 있어 replay 소유권을 그대로 지킨다(caps 도착 전엔 마운트 안 함 → assign replay 온전).
            (() => {
              // ★viewId = node.id(slot id, ADR-0046)★: 슬롯이 자기 slot id 로 구독한다 — 같은 agentId 두
              //   슬롯도 독립 진도(버그 B 해소). key 도 slot id 로 두어(옛 agent_id 키는 같은 agent 두 슬롯이
              //   같은 React key 가 돼 remount 가 꼬였다) 슬롯 정체성을 slot 단위로 고정한다.
              switch (mode) {
                case 'dom':
                  // ★DOM 모드(§5 관측)★: 같은 출력 스트림을 평문 <pre> 로 그려 CDP eval/innerText 로 읽히게
                  // 한다(터미널 xterm 은 canvas 라 관측 불가). agentId = capsReady 확정된 slotAgentId(non-null).
                  return <DomSlot key={node.id} viewId={node.id} agentId={slotAgentId!} epoch={agent.epoch} />
                case 'rich':
                  // 라이브 RichSlot — 실스트림 구독([agentId,epoch]). epoch 은 재spawn 재구독 트리거.
                  return <RichSlot key={node.id} viewId={node.id} agentId={slotAgentId!} epoch={agent.epoch} />
                case 'terminal':
                default:
                  return <TerminalSlot key={node.id} viewId={node.id} agentId={slotAgentId!} />
              }
            })()
          )
        ) : node.content.type === 'preset_palette' ? (
          // ADR-0060/0061: 프리셋 팔레트 variant — 슬롯을 100% 채우는 실 렌더러(hasContent=true).
          //   목록/추가/삭제는 PresetPalette 내부에서 agentClient(단일 제어 표면)로 흐른다.
          <PresetPalette />
        ) : node.content.type === 'agent_list' ? (
          // ADR-0060/0062: 에이전트 목록 variant(Slice C) — 슬롯을 100% 채우는 실 렌더러(hasContent=true).
          //   사이드바 고정 마운트와 동일 컴포넌트(향후 슬롯 배치도 이 케이스로 렌더). 조작은 AgentList
          //   내부에서 agentClient/viewStore(단일 제어 표면)로 흐른다(§5).
          <AgentList />
        ) : (
          // empty 슬롯 플레이스홀더 — 중앙정렬 스타일(hasContent=false) 상속.
          <>
            <span>Slot {node.id.slice(0, 8)}</span>
            <span>— empty —</span>
          </>
        )}
        {contextMenu && (
          // ADR-0064: 통합 슬롯 메뉴 — buildSlotMenu(content.type) 로 (콘텐츠 전용 ∪ 공통 '*') command 참조를
          //   결정적 정렬·resolve 해 항목을 만들고, ctx(viewId/slotId/agentId)를 넘겨 각 command.run 이 백엔드
          //   권위 경로(viewStore/agentClient)로 흐르게 한다(§5 단일 제어 표면). content 종류가 가시성 게이트.
          <SlotContextMenu
            x={contextMenu.x}
            y={contextMenu.y}
            items={buildSlotMenu(node.content.type)}
            ctx={{ viewId: targetViewId, slotId: node.id, agentId: slotAgentId }}
            onClose={() => setContextMenu(null)}
          />
        )}
      </div>
    )
  }
  // split: a/b 두 자식을 방향대로 분할. dir='vertical' = 상하(allotment vertical).
  // ★ratio 초기 사이징(ADR-0063)★: node.ratio = a(왼/위) 자식의 비율. ★Allotment 의 `defaultSizes` 는
  //   비율이 아니라 *픽셀*이다★ — [0.2,0.8] 을 주면 0.2px/0.8px 로 먹어 split-view 가 ~1px 로 붕괴하고
  //   자식들이 흐름 밖으로 쌓인다(실측 스샷으로 확인한 회귀). 대신 첫 pane(a=왼/위)에 `preferredSize` 를
  //   *퍼센트 문자열*로 줘 컨테이너 대비 비율로 배치한다(b 는 나머지 채움). 0.2 → "20%" = 20/80,
  //   0.5 → "50%" = 50/50. 컨테이너 실측 픽셀을 몰라도 되고 높이는 Allotment 가 컨테이너로 채운다.
  //   ★초기 사이징만★: 드래그 리사이즈→백엔드 ratio 되쓰기는 이 슬라이스 범위 밖(ADR-0063).
  // TODO(follow-up): drag→backend ratio writeback (layout:updated 권위 루프 = 별도 슬라이스)
  // ★Allotment.Pane key = 위치 고정(pane-a/pane-b), 콘텐츠 파생 금지★: 옛 key 는 nodeKey(node.a) 로
  //   *서브트리 구조*에서 파생됐다 — 어느 pane 안의 슬롯이 split 으로 재구조화되면 그 pane 의 nodeKey 가
  //   바뀌어 React 가 Pane 을 unmount+remount 했고, Allotment 는 pane 이탈+합류로 보아 전 pane 을 균등
  //   재분배(형제의 비율 소실 — 예: 왼 20% → 50% 점프)했다. split 은 항상 a/b 두 자식을 이 순서로만
  //   가지므로 위치 기반 안정 key("pane-a"/"pane-b")를 쓴다 → 콘텐츠 재구조화에도 Pane 이 마운트 유지 →
  //   Allotment 가 사이즈를 보존한다. (형제 2개 사이에서만 유일하면 됨 — 중첩 Allotment 는 각자 짝을 가짐.)
  //   preferredSize(=ratio 파생 초기 사이징 %)는 첫 pane(a)에만 — 마운트 시 1회 적용·이후 보존(ADR-0063).
  return (
    <div style={{ height: '100%' }}>
      <Allotment vertical={node.dir === 'vertical'}>
        <Allotment.Pane key="pane-a" preferredSize={`${Math.round(node.ratio * 100)}%`}>
          <ViewLayoutRenderer node={node.a} focusedSlotId={focusedSlotId} viewIdOverride={viewIdOverride} />
        </Allotment.Pane>
        <Allotment.Pane key="pane-b">
          <ViewLayoutRenderer node={node.b} focusedSlotId={focusedSlotId} viewIdOverride={viewIdOverride} />
        </Allotment.Pane>
      </Allotment>
    </div>
  )
}
