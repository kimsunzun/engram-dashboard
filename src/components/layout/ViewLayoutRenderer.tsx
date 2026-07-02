// ViewLayoutRenderer — 백엔드 권위 레이아웃(ViewManager) 트리를 그리는 최소 렌더러(ADR-0035 수직 슬라이스).
//
// ★기존 LayoutRenderer 와 별도★: LayoutRenderer 는 slotStore 의 LayoutNode(number id + content union,
// 프론트 내부 상태)를 그린다. 이건 wire LayoutNode(string UUID id + agent_id, src-tauri/bindings)를 그린다.
// 이번 슬라이스의 목표는 split 루프 실증이라 slotStore 렌더를 갈아엎지 않고 *추가*만 한다 — 전면 이주는
// 다음 슬라이스(보고 참조).

import { Allotment } from 'allotment'

import type { LayoutNode } from '../../api/layoutTypes'
import { useViewStore } from '../../store/viewStore'
import TerminalSlot from '../slot/TerminalSlot'
import RichSlot from '../slot/RichSlot'

function nodeKey(node: LayoutNode): string {
  if (node.type === 'slot') return `s${node.id}`
  return `p[${node.dir}:${nodeKey(node.a)},${nodeKey(node.b)}]`
}

export default function ViewLayoutRenderer({
  node,
  focusedSlotId,
}: {
  node: LayoutNode
  focusedSlotId: string | null
}) {
  // ★M0 스파이크(임시) — ADR-0044★: 이 slot 이 RichSlot(fixture 구동 JSON 모드) 오버레이인지.
  // 프론트 전용(백엔드 wire LayoutNode 엔 없음) — M2 에서 transport caps 기반 분기로 대체될 자리.
  const richSlots = useViewStore(s => s.richSlots)
  const mountRich = useViewStore(s => s.mountRich)

  if (node.type === 'slot') {
    const isFocused = node.id === focusedSlotId
    // agent_id 우선(실 터미널). agent 없는 빈 슬롯에만 rich 스파이크가 적용된다.
    const isRich = node.agent_id == null && !!richSlots[node.id]
    const hasContent = node.agent_id != null || isRich
    // ★known-limitation: 같은 webview 안에서 동일 agentId 를 두 슬롯에 배정하면 ProtocolClient.subs 가
    // agentId 당 단일 콜백이라 두 번째 마운트가 첫 슬롯 구독을 덮어써 첫 슬롯이 빈다. 이번 슬라이스
    // 범위 밖 — 별도 슬라이스에서 콜백 레지스트리를 agentId 당 다중 구독으로 확장해 해소 예정.
    // (옛 "unmount 시 형제 구독 동반 제거"는 ProtocolClient owner-token 가드로 이미 해소.)
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
      >
        {node.agent_id != null ? (
          <TerminalSlot key={node.agent_id} agentId={node.agent_id} />
        ) : isRich ? (
          <RichSlot />
        ) : (
          <>
            <span>Slot {node.id.slice(0, 8)}</span>
            <span>— empty —</span>
            {/* ★M0 스파이크(임시) — ADR-0044★: 빈 슬롯에서 JSON 모드 렌더를 눈으로 보게 하는 dev 버튼.
                window.__richslot(§5 LLM 경로)와 같은 mountRich 액션을 흔든다 — M2 에서 제거 예정. */}
            <button
              onClick={() => mountRich(node.id)}
              style={{
                marginTop: '4px',
                cursor: 'pointer',
                background: 'transparent',
                border: '1px solid var(--border)',
                color: 'var(--text-muted)',
                borderRadius: '4px',
                padding: '2px 8px',
                fontSize: '11px',
              }}
            >
              JSON 스파이크
            </button>
          </>
        )}
      </div>
    )
  }
  // split: a/b 두 자식을 방향대로 분할. dir='vertical' = 상하(allotment vertical).
  return (
    <div style={{ height: '100%' }}>
      <Allotment vertical={node.dir === 'vertical'}>
        <Allotment.Pane key={nodeKey(node.a)}>
          <ViewLayoutRenderer node={node.a} focusedSlotId={focusedSlotId} />
        </Allotment.Pane>
        <Allotment.Pane key={nodeKey(node.b)}>
          <ViewLayoutRenderer node={node.b} focusedSlotId={focusedSlotId} />
        </Allotment.Pane>
      </Allotment>
    </div>
  )
}
