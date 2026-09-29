// ★역할★: 사람 클릭·팔레트·키바인딩·LLM 이 같은 command·같은 id 를 지난다(§5 단일 제어 표면, ADR-0055).
//   메뉴 자신은 store 를 직접 부르지 않는다(ADR-0064 불변식 — 옛 하드코딩 9항목의 viewStore 직접 호출).
//
// ★한 메뉴 컴포넌트★(ADR-0064 §5): 콘텐츠(PresetPalette/AgentList)가 자기 pane 메뉴를 소유하던 옛 구조를
//   제거하고 이 하나로 통합했다.

import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import type { CSSProperties, SyntheticEvent } from 'react'

import { fireAndForget } from '../../commands/dispatch'
import type { CommandArgs } from '../../commands/registry'
import { SLOT_MENU_ORIGIN, type ResolvedSlotMenuItem, type SlotMenuCtx } from '../../commands/slotMenu'

/** 메뉴가 창 테두리에 딱 붙지 않게. */
const MENU_MARGIN = 4

/**
 * 앵커(여는 지점)와 메뉴 사각형 사이 간격 — ★더블클릭 인식 반경보다 크게★(ADR-0143 결정 4).
 *
 * 유도: Windows 는 첫 클릭 주위 사각형 안에 둘째 클릭이 떨어지면 더블클릭으로 본다. 그 사각형의 *폭·높이*가
 * `SM_CXDOUBLECLK`/`SM_CYDOUBLECLK`(기본 4px)이므로 첫 클릭에서의 허용 *반경*은 그 절반 = 2px. 여기서 4배를
 * 잡아 사용자가 허용치를 두 배로 키워도(레지스트리 `DoubleClickWidth`) 둘째 클릭이 메뉴에 닿지 않게 한다.
 *
 * 이 간격이 0 이 되면 앵커가 메뉴 모서리에 딱 붙어, 빈 슬롯을 더블클릭하면 둘째 클릭이 항목 위에 떨어져
 * 실행된다 — 아래쪽 슬롯에선 메뉴가 위로 뒤집혀 맨 아래 `닫기`가 커서 밑에 오고, 레이아웃은 디스크에 없어
 * 되돌릴 수단이 없다.
 */
export const ANCHOR_GAP = 8

/**
 * 창 가장자리에서 메뉴가 잘려 클릭 못 하던 버그(Bug1) 방지. 앵커(여는 지점)는 어느 방향으로 배치되든 메뉴
 * 바깥 `ANCHOR_GAP` 만큼 떨어진 자리에 남는다.
 */
export function clampMenuPosition(
  x: number,
  y: number,
  w: number,
  h: number,
  vw: number,
  vh: number,
): { top: number; left: number } {
  return { top: placeAlongAxis(y, h, vh), left: placeAlongAxis(x, w, vw) }
}

// ★밀지 말고 뒤집는다★: 밀어 넣으면 앵커(여는 지점)가 메뉴 *안*으로 들어가 커서 밑에 항목이 놓인다.
//   빈 슬롯이 좌클릭으로 같은 메뉴를 열게 된 뒤로는(ADR-0141·0142) 그 상태에서 더블클릭 한 번이 커서 밑
//   항목을 실행해 버린다 — 닫기가 걸리면 슬롯이 사라진다. 뒤집으면 앵커는 어느 방향이든 메뉴 밖에 남는다.
//   ★두 방향 모두 ANCHOR_GAP 을 얹는다(ADR-0143 결정 4)★: 모서리에 붙기만 해서는 더블클릭 둘째 클릭이
//   항목에 떨어진다 — 간격을 한 곳(이 helper)에서 주므로 두 축·두 방향이 같은 규칙을 받는다.
//   뒤집어도 화면을 벗어나는 퇴화 케이스(메뉴가 뷰포트보다 큼)에서만 밀기로 떨어진다 — 그때는
//   "화면 안에 있다"가 앵커 규칙(간격 포함)보다 우선이다.
function placeAlongAxis(anchor: number, size: number, viewport: number): number {
  const forward = anchor + ANCHOR_GAP
  if (forward + size <= viewport) return forward
  const flipped = anchor - ANCHOR_GAP - size
  if (flipped >= 0) return flipped
  return Math.max(MENU_MARGIN, Math.min(anchor, viewport - size - MENU_MARGIN))
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

  // ★역할은 그리기만이다 — 키보드 이동(화살표·Home/End·타입어헤드)은 없다★(알려진 갭 · 이 메뉴 전부).
  return (
    <div
      ref={ref}
      role="menu"
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
        <div key={item.id} role="none">
          {item.separatorBefore && (
            <div role="separator" style={{ height: '1px', background: 'var(--border)', margin: '2px 0' }} />
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

const DISABLED_ROW_STYLE: CSSProperties = { ...ROW_STYLE, cursor: 'default', opacity: 0.4 }

function commandArgs(ctx: SlotMenuCtx): CommandArgs {
  return { viewId: ctx.viewId, slotId: ctx.slotId, agentId: ctx.agentId, content: ctx.content, origin: SLOT_MENU_ORIGIN }
}

function runItem(id: string, ctx: SlotMenuCtx, onClose: () => void) {
  // ADR-0064/0055: 팔레트·키바인딩·LLM 소비자와 동일 helper 재사용 — sync throw·async reject·thenable 삼킴
  //   안전망을 재구현하지 않는다.
  fireAndForget(id, commandArgs(ctx))
  onClose()
}

// ★렌더 중에 부르므로 throw 를 흘리지 않는다★(slotMenu FIX-1 과 같은 fail-loud but crash-free) — error boundary 가
//   없으면 기여 하나의 throw 가 화면을 통째로 비운다. 판정 실패 = 끔 · 비활성.
function evaluate(item: ResolvedSlotMenuItem, what: 'checked' | 'enabled', fn: () => boolean): boolean {
  try {
    return fn()
  } catch (err) {
    console.error(`[SlotContextMenu] ${what} 판정 실패 — "${item.id}" 를 ${what === 'checked' ? '끔' : '비활성'}으로 그린다:`, err)
    return false
  }
}

/** 실행 항목 한 줄 — 최상위와 서브메뉴 자식이 같은 것을 그린다. */
function LeafRow({ item, ctx, onClose }: { item: ResolvedSlotMenuItem; ctx: SlotMenuCtx; onClose: () => void }) {
  const checkedFn = item.checked
  const enabledFn = item.enabled
  const checked = checkedFn ? evaluate(item, 'checked', () => checkedFn(ctx)) : undefined
  const enabled = enabledFn ? evaluate(item, 'enabled', () => enabledFn(ctx)) : true
  return (
    <div
      data-slot-menu-item={item.id}
      {...(checked === undefined ? { role: 'menuitem' } : { role: 'menuitemcheckbox', 'aria-checked': checked })}
      {...(enabled ? {} : { 'aria-disabled': true, 'data-slot-menu-disabled': '' })}
      style={enabled ? ROW_STYLE : DISABLED_ROW_STYLE}
      onMouseEnter={enabled ? highlightOn : undefined}
      onMouseLeave={enabled ? highlightOff : undefined}
      onClick={e => {
        e.stopPropagation()
        if (enabled) runItem(item.id, ctx, onClose)
      }}
    >
      {checked !== undefined && (
        <span aria-hidden="true" style={{ display: 'inline-block', width: '1.2em' }}>
          {checked ? '✓' : ''}
        </span>
      )}
      {item.title}
    </div>
  )
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

  if (!isContainer) return <LeafRow item={item} ctx={ctx} onClose={onClose} />

  return (
    <div
      ref={rowRef}
      // data-attr 로 cdp/테스트가 컨테이너를 식별.
      data-slot-menu-container={item.id}
      role="none"
      style={{ position: 'relative' }}
      onMouseEnter={() => setOpen(true)}
      onMouseLeave={() => {
        setOpen(false)
        setFlyoutPos(null)
      }}
    >
      <div
        role="menuitem"
        aria-haspopup="menu"
        aria-expanded={open}
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
          role="menu"
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
            <LeafRow key={child.id} item={child} ctx={ctx} onClose={onClose} />
          ))}
        </div>
      )}
    </div>
  )
}
