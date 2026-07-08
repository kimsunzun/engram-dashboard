// ViewLayoutRenderer — 백엔드 권위 레이아웃(ViewManager) 트리를 그리는 메인 캔버스 렌더러(ADR-0035).
//
// ★유일한 레이아웃 렌더러★(Brick 1): 옛 프론트 전용 slotStore/LayoutRenderer(number id + content union)는
// 제거됐다. 이 렌더러는 wire LayoutNode(string UUID id + agent_id, src-tauri/bindings)만 그린다 —
// 사람 우클릭(SlotContextMenu)이든 LLM(window.__engramLayout)이든 같은 invoke→emit 권위 루프로 갱신된다.

import { useState } from 'react'
import { Allotment } from 'allotment'

import type { LayoutNode } from '../../api/layoutTypes'
import { useViewStore } from '../../store/viewStore'
import { useAgentStore } from '../../store/agentStore'
import TerminalSlot from '../slot/TerminalSlot'
import RichSlot from '../slot/RichSlot'
import DomSlot from '../slot/DomSlot'
import SlotContextMenu from '../slot/SlotContextMenu'
import { defaultRenderMode } from '../slot/renderMode'

function nodeKey(node: LayoutNode): string {
  if (node.type === 'slot') return `s${node.id}`
  return `p[${node.dir}:${nodeKey(node.a)},${nodeKey(node.b)}]`
}

export default function ViewLayoutRenderer({
  node,
  focusedSlotId,
  viewIdOverride,
}: {
  node: LayoutNode
  focusedSlotId: string | null
  // ★Fix 3: 이 렌더러가 그리는 View id 오버라이드(선택).★ 팝업 창(PopoutPage)이 자기 고정 view 를 넘겨
  //   내부 SlotContextMenu 의 액션 좌표를 그 view 로 고정한다. 메인 창은 안 넘김 → 메뉴가 activeViewId 폴백
  //   (하위호환). 재귀 split 렌더에도 그대로 전파해 하위 슬롯 메뉴까지 같은 view 를 쓰게 한다.
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

  if (node.type === 'slot') {
    const isFocused = node.id === focusedSlotId
    // agent_id 있는 슬롯만 실 렌더러가 붙는다(빈 슬롯 = 플레이스홀더).
    const agent = node.agent_id != null ? (agents.find(a => a.id === node.agent_id) ?? null) : null
    // ★caps 도착 후에만 구체 렌더러를 마운트한다(ADR-0041 replay 소유권)★: 데몬 replay 는 slot-assign
    //   델타((window,agent) 키)에서 단 1회만 발화하고, 컴포넌트 스왑(TerminalSlot→RichSlot)엔 재발화하지
    //   않는다. 그래서 caps 미도착 상태에서 TerminalSlot 을 먼저 띄웠다가 caps 도착 후 RichSlot 으로 갈아끼면,
    //   스왑된 RichSlot 이 빈 채로 마운트돼 스왑 전 바이트가 영구 유실된다. 대신 caps(=AgentInfo) 도착 전엔
    //   중립 플레이스홀더만 두고(아래 '에이전트 연결 중…'), 첫 구체 렌더러를 caps 확정 후 마운트해 assign
    //   시점 replay 를 온전히 받게 한다. (터미널 에이전트는 보통 assign 전에 AgentInfo 가 오므로 이 플레이스
    //   홀더는 일시적 엣지 상태다 — 터미널 replay 경로는 종전과 동일.)
    // 구조화 출력(NDJSON) = 라이브 RichSlot, 아니면 TerminalSlot(xterm) 분기 근거(ADR-0002/0044).
    const capsReady = node.agent_id != null && agent != null
    // caps-ready 슬롯의 렌더러: 오버라이드가 있으면 그걸, 없으면 caps 에서 유도한 기본(defaultRenderMode).
    // agent 는 capsReady 분기(아래 agent != null)에서만 사용되므로 여기선 null 병합만 걸어둔다.
    const mode = agent != null ? (renderModeOverride[node.id] ?? defaultRenderMode(agent)) : null
    // hasContent = 구체 렌더러를 그리는 경우만(래퍼를 100% 채움). caps 대기 플레이스홀더는 empty 슬롯처럼
    // 중앙정렬 스타일로 둔다(hasContent=false).
    const hasContent = capsReady
    // ★ADR-0046: 버그 B 구조 해소★: ProtocolClient.subs 가 이제 viewId(slot id) 키라 같은 agentId 를 두
    //   슬롯에 배정해도 각 슬롯이 독립 구독·독립 진도를 갖는다(옛 agentId-당-단일-콜백 덮어쓰기 소멸).
    //   슬롯은 아래 viewId={node.id} 로 자기 slot id 를 구독 키로 넘긴다.
    return (
      <div
        style={{
          height: '100%',
          background: 'var(--bg)',
          border: isFocused ? '2px solid var(--accent)' : '1px solid var(--border)',
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
        // ADR-0035: 우클릭 → SlotContextMenu 마운트. 메뉴 액션(분할/닫기/배정)은 viewStore(=window.__engramLayout)
        //   단일 제어 표면으로만 흐른다(사람 클릭 = LLM 이 한 표면, §5). 기본 컨텍스트 메뉴는 막는다.
        onContextMenu={e => {
          e.preventDefault()
          setContextMenu({ x: e.clientX, y: e.clientY })
        }}
      >
        {node.agent_id != null ? (
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
                  // 한다(터미널 xterm 은 canvas 라 관측 불가).
                  return <DomSlot key={node.id} viewId={node.id} agentId={node.agent_id} epoch={agent.epoch} />
                case 'rich':
                  // 라이브 RichSlot — 실스트림 구독([agentId,epoch]). epoch 은 재spawn 재구독 트리거.
                  return <RichSlot key={node.id} viewId={node.id} agentId={node.agent_id} epoch={agent.epoch} />
                case 'terminal':
                default:
                  return <TerminalSlot key={node.id} viewId={node.id} agentId={node.agent_id} />
              }
            })()
          )
        ) : (
          <>
            <span>Slot {node.id.slice(0, 8)}</span>
            <span>— empty —</span>
          </>
        )}
        {contextMenu && (
          // ADR-0035: 이 슬롯의 우클릭 메뉴 — slotId=node.id, agentId=node.agent_id 를 넘겨
          //   viewStore(백엔드 권위) 경로로 분할/닫기/배정을 흘린다(§5 단일 제어 표면).
          <SlotContextMenu
            x={contextMenu.x}
            y={contextMenu.y}
            slotId={node.id}
            agentId={node.agent_id}
            viewIdOverride={viewIdOverride}
            onClose={() => setContextMenu(null)}
          />
        )}
      </div>
    )
  }
  // split: a/b 두 자식을 방향대로 분할. dir='vertical' = 상하(allotment vertical).
  return (
    <div style={{ height: '100%' }}>
      <Allotment vertical={node.dir === 'vertical'}>
        <Allotment.Pane key={nodeKey(node.a)}>
          <ViewLayoutRenderer node={node.a} focusedSlotId={focusedSlotId} viewIdOverride={viewIdOverride} />
        </Allotment.Pane>
        <Allotment.Pane key={nodeKey(node.b)}>
          <ViewLayoutRenderer node={node.b} focusedSlotId={focusedSlotId} viewIdOverride={viewIdOverride} />
        </Allotment.Pane>
      </Allotment>
    </div>
  )
}
