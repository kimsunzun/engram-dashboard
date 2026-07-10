// ADR-0064: 통합 슬롯 컨텍스트 메뉴 렌더러 — resolve 된 command 항목(buildSlotMenu 산출)만 그린다.
//
// ★역할★: 옛 하드코딩 9항목(viewStore 직접 호출)을 폐기하고, ViewLayoutRenderer 가 buildSlotMenu(content.type)
//   로 만든 command 참조 목록(ResolvedSlotMenuItem[])을 받아 렌더한다. 클릭 시 각 항목의 run(ctx) 를 부른다 —
//   ctx = { viewId, slotId, agentId }(command 실행 컨텍스트). 사람 클릭·팔레트·키바인딩·LLM 이 같은 command·
//   같은 id 를 지난다(§5 단일 제어 표면, ADR-0055). 메뉴 자신은 store 를 직접 부르지 않는다(ADR-0064 불변식).
//
// ★한 메뉴 컴포넌트★(ADR-0064 §5): 콘텐츠(PresetPalette/AgentList)가 자기 pane 메뉴를 소유하던 옛 구조를
//   제거하고 이 하나로 통합했다. group 경계(콘텐츠 → 구분선 → 공통)는 buildSlotMenu 가 separatorBefore 로 표시.

import { useEffect, useRef } from 'react'

import { fireAndForget } from '../../commands/dispatch'
import type { ResolvedSlotMenuItem } from '../../commands/slotMenu'

/** 메뉴 항목 클릭 시 command.run 에 넘길 실행 컨텍스트(ADR-0064). viewId/slotId 필수, agentId 는 배정 슬롯만. */
export interface SlotMenuCtx {
  viewId: string | null
  slotId: string
  agentId?: string | null
}

interface SlotContextMenuProps {
  x: number
  y: number
  /** buildSlotMenu(content.type) 산출 — 이미 group·order 로 정렬되고 registry resolve 된 항목들. */
  items: ResolvedSlotMenuItem[]
  /** command.run 에 넘길 컨텍스트(viewId/slotId/agentId). */
  ctx: SlotMenuCtx
  onClose: () => void
}

export default function SlotContextMenu({ x, y, items, ctx, onClose }: SlotContextMenuProps) {
  const ref = useRef<HTMLDivElement>(null)

  // 바깥 클릭으로 닫기 — 자기 ref 밖 mousedown 이면 닫는다(옛 가드 동형).
  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose()
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [onClose])

  return (
    <div
      ref={ref}
      style={{
        position: 'fixed',
        top: y,
        left: x,
        background: 'var(--bg-secondary)',
        border: '1px solid var(--border)',
        borderRadius: '4px',
        zIndex: 1000,
        minWidth: '150px',
        boxShadow: '0 2px 8px rgba(0,0,0,0.3)',
        fontFamily: 'var(--font-ui)',
        fontSize: '12px',
      }}
    >
      {items.map(item => (
        <div key={item.id}>
          {/* 그룹 경계(콘텐츠 → 공통) 구분선 — 변수-only 보더. */}
          {item.separatorBefore && (
            <div style={{ height: '1px', background: 'var(--border)', margin: '2px 0' }} />
          )}
          <div
            data-slot-menu-item={item.id}
            style={{ padding: '6px 12px', cursor: 'pointer', color: 'var(--text)' }}
            onMouseEnter={e => (e.currentTarget.style.background = 'color-mix(in srgb, var(--accent) 20%, transparent)')}
            onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}
            // ADR-0064/0055: 클릭 = 공유 dispatch 경로(fireAndForget)로 command 를 id 로 실행한다. 팔레트·
            //   키바인딩·LLM 소비자와 동일한 helper 를 재사용해 안전망(sync throw·async reject·thenable
            //   삼킴)을 재구현하지 않는다. item.id === 등록된 command id(buildSlotMenu resolve). ctx 는
            //   command 인자 가방(단일 객체). 메뉴는 항상 닫는다.
            // TODO(deferred): ctx.viewId===null(활성 view 없음)이면 공통 op 가 실패 — 이제 fireAndForget 이
            //   일관되게 warn 으로 로깅한다. agent_list/preset_palette 슬롯은 보통 활성 view 를 가져 저빈도 엣지.
            onClick={e => {
              e.stopPropagation()
              fireAndForget(item.id, { viewId: ctx.viewId, slotId: ctx.slotId, agentId: ctx.agentId })
              onClose()
            }}
          >
            {item.title}
          </div>
        </div>
      ))}
    </div>
  )
}
