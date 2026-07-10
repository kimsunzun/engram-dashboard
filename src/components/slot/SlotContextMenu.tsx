// ADR-0064: 통합 슬롯 컨텍스트 메뉴 렌더러 — resolve 된 command 항목(buildSlotMenu 산출)만 그린다.
//
// ★역할★: 옛 하드코딩 9항목(viewStore 직접 호출)을 폐기하고, ViewLayoutRenderer 가 buildSlotMenu(content.type)
//   로 만든 command 참조 목록(ResolvedSlotMenuItem[])을 받아 렌더한다. 클릭 시 각 항목의 run(ctx) 를 부른다 —
//   ctx = { viewId, slotId, agentId }(command 실행 컨텍스트). 사람 클릭·팔레트·키바인딩·LLM 이 같은 command·
//   같은 id 를 지난다(§5 단일 제어 표면, ADR-0055). 메뉴 자신은 store 를 직접 부르지 않는다(ADR-0064 불변식).
//
// ★한 메뉴 컴포넌트★(ADR-0064 §5): 콘텐츠(PresetPalette/AgentList)가 자기 pane 메뉴를 소유하던 옛 구조를
//   제거하고 이 하나로 통합했다. group 경계(콘텐츠 → 구분선 → 공통)는 buildSlotMenu 가 separatorBefore 로 표시.

import { useEffect, useLayoutEffect, useRef, useState } from 'react'

import { fireAndForget } from '../../commands/dispatch'
import type { ResolvedSlotMenuItem } from '../../commands/slotMenu'

/** 뷰포트 가장자리 최소 여백(px) — 메뉴가 창 테두리에 딱 붙지 않게. */
const MENU_MARGIN = 4

/**
 * 커서 좌표(x,y)에 뜬 메뉴(w×h)가 뷰포트(vw×vh) 밖으로 넘치면 안쪽으로 clamp 한 {top,left} 를 돌려준다.
 * 창 하단/우측 가장자리 우클릭 시 메뉴가 잘려 클릭 못 하던 버그(Bug1) 방지 — 넘치면 사실상 위/왼쪽으로 밀어
 * 전체가 보이게 한다. 순수 함수(측정값만 받음)라 컴포넌트 밖에서 단위테스트한다.
 *   bottom: y + h > vh → top = clamp(y, MARGIN, vh - h - MARGIN)
 *   right : x + w > vw → left = clamp(x, MARGIN, vw - w - MARGIN)
 * 메뉴가 뷰포트보다 큰 극단(h > vh)에서도 top 은 최소 MARGIN 으로 상단 고정(음수 방지).
 */
export function clampMenuPosition(
  x: number,
  y: number,
  w: number,
  h: number,
  vw: number,
  vh: number,
): { top: number; left: number } {
  const left = x + w > vw ? Math.max(MENU_MARGIN, Math.min(x, vw - w - MENU_MARGIN)) : x
  const top = y + h > vh ? Math.max(MENU_MARGIN, Math.min(y, vh - h - MENU_MARGIN)) : y
  return { top, left }
}

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
  // ★뷰포트 clamp 된 실제 위치(Bug1)★: 초기값 = 커서 좌표(x,y) — 첫 페인트는 여기 뜨고, 아래 useLayoutEffect
  //   가 마운트 직후(페인트 전) 실측 크기로 넘침을 보정해 안쪽으로 밀어넣는다. 넘치지 않으면 그대로.
  const [pos, setPos] = useState<{ top: number; left: number }>({ top: y, left: x })

  // ★페인트 전 위치 보정(Bug1)★: 메뉴를 실제로 렌더한 뒤 getBoundingClientRect 로 크기를 재고, 창 하단/우측을
  //   넘으면 뷰포트 안으로 clamp 한다. useLayoutEffect 라 브라우저 페인트 전에 반영돼 시각적 점프를 최소화한다
  //   (측정엔 마운트가 필요하므로 최대 1프레임 재배치는 감수 — 지시서 허용 범위). x/y 가 바뀌면 재측정.
  useLayoutEffect(() => {
    if (!ref.current) return
    const rect = ref.current.getBoundingClientRect()
    setPos(clampMenuPosition(x, y, rect.width, rect.height, window.innerWidth, window.innerHeight))
    // deps 에 items.length 포함(Codex 리뷰 LOW): 메뉴가 같은 x/y 로 열린 채 항목 수가 바뀌면(외부 콘텐츠
    //   변경) 높이가 달라져 재측정이 필요하다. items 는 매 렌더 새 배열 참조라 length 로 안정 트리거(내용만
    //   바뀌고 개수 동일하면 높이 거의 불변 → 무시 가능).
  }, [x, y, items.length])

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
        top: pos.top,
        left: pos.left,
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
