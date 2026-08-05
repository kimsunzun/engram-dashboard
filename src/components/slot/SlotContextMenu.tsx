// ★역할★: 사람 클릭·팔레트·키바인딩·LLM 이 같은 command·같은 id 를 지난다(§5 단일 제어 표면, ADR-0055).
//   메뉴 자신은 store 를 직접 부르지 않는다(ADR-0064 불변식 — 옛 하드코딩 9항목의 viewStore 직접 호출).
//
// ★한 메뉴 컴포넌트★(ADR-0064 §5): 콘텐츠(PresetPalette/AgentList)가 자기 pane 메뉴를 소유하던 옛 구조를
//   제거하고 이 하나로 통합했다.

import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import type { CSSProperties, SyntheticEvent } from 'react'

import { fireAndForget } from '../../commands/dispatch'
import type { ResolvedSlotMenuItem } from '../../commands/slotMenu'

/** 메뉴가 창 테두리에 딱 붙지 않게. */
const MENU_MARGIN = 4

/**
 * 창 하단/우측 가장자리 우클릭 시 메뉴가 잘려 클릭 못 하던 버그(Bug1) 방지.
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

export function flyoutPosition(
  anchorLeft: number,
  anchorRight: number,
  anchorTop: number,
  fw: number,
  fh: number,
  vw: number,
  vh: number,
): { top: number; left: number } {
  const overflowRight = anchorRight + fw > vw
  const fitsLeft = anchorLeft - fw >= MENU_MARGIN
  const left = overflowRight && fitsLeft ? anchorLeft - fw : anchorRight
  // 뒤집어도 여전히 넘칠 극단 방어.
  const clampedLeft = Math.max(MENU_MARGIN, Math.min(left, Math.max(MENU_MARGIN, vw - fw - MENU_MARGIN)))
  const top =
    anchorTop + fh > vh ? Math.max(MENU_MARGIN, Math.min(anchorTop, vh - fh - MENU_MARGIN)) : anchorTop
  return { top, left: clampedLeft }
}

/** agentId 는 배정 슬롯만. */
export interface SlotMenuCtx {
  viewId: string | null
  slotId: string
  agentId?: string | null
}

interface SlotContextMenuProps {
  x: number
  y: number
  /** 이미 group·order 로 정렬되고 registry resolve 된 항목들(buildSlotMenu 산출). */
  items: ResolvedSlotMenuItem[]
  ctx: SlotMenuCtx
  onClose: () => void
}

export default function SlotContextMenu({ x, y, items, ctx, onClose }: SlotContextMenuProps) {
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState<{ top: number; left: number }>({ top: y, left: x })

  // ★페인트 전 위치 보정(Bug1)★: useLayoutEffect 라 브라우저 페인트 전에 반영돼 시각적 점프를 최소화한다
  //   (측정엔 마운트가 필요하므로 최대 1프레임 재배치는 감수 — 지시서 허용 범위).
  useLayoutEffect(() => {
    if (!ref.current) return
    const rect = ref.current.getBoundingClientRect()
    setPos(clampMenuPosition(x, y, rect.width, rect.height, window.innerWidth, window.innerHeight))
    // deps 에 items.length 포함(Codex 리뷰 LOW): 메뉴가 같은 x/y 로 열린 채 항목 수가 바뀌면(외부 콘텐츠
    //   변경) 높이가 달라져 재측정이 필요하다. items 는 매 렌더 새 배열 참조라 length 로 안정 트리거(내용만
    //   바뀌고 개수 동일하면 높이 거의 불변 → 무시 가능).
  }, [x, y, items.length])

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
          {item.separatorBefore && (
            <div style={{ height: '1px', background: 'var(--border)', margin: '2px 0' }} />
          )}
          <MenuRow item={item} ctx={ctx} onClose={onClose} />
        </div>
      ))}
    </div>
  )
}

const ROW_STYLE: CSSProperties = { padding: '6px 12px', cursor: 'pointer', color: 'var(--text)' }
function highlightOn(e: SyntheticEvent<HTMLElement>) {
  e.currentTarget.style.background = 'color-mix(in srgb, var(--accent) 20%, transparent)'
}
function highlightOff(e: SyntheticEvent<HTMLElement>) {
  e.currentTarget.style.background = 'transparent'
}

function runItem(id: string, ctx: SlotMenuCtx, onClose: () => void) {
  // ADR-0064/0055: 팔레트·키바인딩·LLM 소비자와 동일 helper 재사용 — sync throw·async reject·thenable 삼킴
  //   안전망을 재구현하지 않는다.
  fireAndForget(id, { viewId: ctx.viewId, slotId: ctx.slotId, agentId: ctx.agentId })
  onClose()
}

/**
 * 자식은 leaf 와 동일한 공유 dispatch 경로로 실행한다(§5 불변 — 서브메뉴는 presentation 일 뿐).
 */
function MenuRow({ item, ctx, onClose }: { item: ResolvedSlotMenuItem; ctx: SlotMenuCtx; onClose: () => void }) {
  const isContainer = !!item.children && item.children.length > 0
  const rowRef = useRef<HTMLDivElement>(null)
  const flyoutRef = useRef<HTMLDivElement>(null)
  const [open, setOpen] = useState(false)
  const [flyoutPos, setFlyoutPos] = useState<{ top: number; left: number } | null>(null)

  useLayoutEffect(() => {
    if (!isContainer || !open || !rowRef.current || !flyoutRef.current) return
    const anchor = rowRef.current.getBoundingClientRect()
    const fly = flyoutRef.current.getBoundingClientRect()
    setFlyoutPos(
      flyoutPosition(anchor.left, anchor.right, anchor.top, fly.width, fly.height, window.innerWidth, window.innerHeight),
    )
  }, [isContainer, open])

  if (!isContainer) {
    return (
      <div
        data-slot-menu-item={item.id}
        style={ROW_STYLE}
        onMouseEnter={highlightOn}
        onMouseLeave={highlightOff}
        onClick={e => {
          e.stopPropagation()
          runItem(item.id, ctx, onClose)
        }}
      >
        {item.title}
      </div>
    )
  }

  return (
    <div
      ref={rowRef}
      // data-attr 로 cdp/테스트가 컨테이너를 식별.
      data-slot-menu-container={item.id}
      style={{ position: 'relative' }}
      onMouseEnter={() => setOpen(true)}
      onMouseLeave={() => {
        setOpen(false)
        setFlyoutPos(null)
      }}
    >
      <div
        style={{ ...ROW_STYLE, display: 'flex', justifyContent: 'space-between', gap: '12px', alignItems: 'center' }}
        tabIndex={0}
        onFocus={() => setOpen(true)}
        onMouseEnter={highlightOn}
        onMouseLeave={highlightOff}
      >
        <span>{item.title}</span>
        <span style={{ opacity: 0.6 }}>▶</span>
      </div>
      {open && (
        <div
          ref={flyoutRef}
          data-slot-menu-flyout={item.id}
          style={{
            position: 'fixed',
            top: flyoutPos?.top ?? 0,
            left: flyoutPos?.left ?? 0,
            // 측정 전(flyoutPos=null)엔 숨겨 점프를 감춘다(clamp 완료 후 노출).
            visibility: flyoutPos ? 'visible' : 'hidden',
            background: 'var(--bg-secondary)',
            border: '1px solid var(--border)',
            borderRadius: '4px',
            zIndex: 1001,
            minWidth: '150px',
            boxShadow: '0 2px 8px rgba(0,0,0,0.3)',
          }}
        >
          {item.children!.map(child => (
            <div
              key={child.id}
              data-slot-menu-item={child.id}
              style={ROW_STYLE}
              onMouseEnter={highlightOn}
              onMouseLeave={highlightOff}
              onClick={e => {
                e.stopPropagation()
                runItem(child.id, ctx, onClose)
              }}
            >
              {child.title}
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
