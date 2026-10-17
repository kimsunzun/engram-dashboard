// TRD S21-slot-docking §4-3: 끄는 동안 이 창의 가이드를 그린다 — 커서 밑 칸의 띠 · 바깥 띠(빗금) · 고른 띠의 결과
//   사각형(강조 — 바깥이면 `landing`) · 탭바 끼울 자리 막대 · 전환 대기 진행 표시. 판정은 하지 않는다 — 입력은
//   `dockDrag.ts` 가 낸 `DockGuide` 하나다.
// ★창 뷰포트에 고정해 그린다(`position: fixed`)★ — 영역의 탭바 사각형은 뷰포트 CSS px 이고 칸 · 바깥 띠는 캔버스
//   원점(`canvas_origin`) 기준이라, 캔버스 div 안에 두면 그 밖의 탭바 막대를 못 그린다. 그래서 창 어디에 마운트해도
//   된다 — 단 조상에 `transform` · `filter` 가 있으면 `fixed` 의 기준이 그 조상으로 바뀌어 어긋난다.

import type { CSSProperties, ReactNode } from 'react'

import type { DockRegions } from '../../../../src-tauri/bindings/DockRegions'
import type { PxRectWire } from '../../../../src-tauri/bindings/PxRectWire'
import { HOVER_SWITCH_MS, useDockGuide, type DockGuide } from './dockDrag'

const HATCH =
  'repeating-linear-gradient(45deg, color-mix(in srgb, var(--accent) 30%, transparent) 0 4px, transparent 4px 9px)'

const HOVER_KEYFRAMES = `@keyframes engram-dock-hover { from { transform: scaleX(0) } to { transform: scaleX(1) } }`

function canvasBox(r: DockRegions, rect: PxRectWire): CSSProperties {
  return {
    position: 'absolute',
    left: r.canvas_origin.x + rect.x0,
    top: r.canvas_origin.y + rect.y0,
    width: rect.x1 - rect.x0,
    height: rect.y1 - rect.y0,
  }
}

/** 고른 띠의 결과 사각형 — 바깥 변이면 `landing`, 칸 띠면 그 띠 사각형(PRD R3 — 띠 = 결과). */
function landingOf(guide: DockGuide): PxRectWire | null {
  const { regions: r, shown } = guide
  if (shown.type !== 'seam') return null
  if (shown.anchor === null) return r.outer.find(o => o.side === shown.side)?.landing ?? null
  const slot = r.slots.find(s => s.slot_id === shown.anchor)
  return slot?.bands.find(b => b.side === shown.side)?.rect ?? null
}

export function DockOverlayView({ guide }: { guide: DockGuide | null }) {
  if (guide === null) return null
  const { regions: r, shown } = guide
  const items: ReactNode[] = []

  if (guide.canvas) {
    for (const o of r.outer) {
      items.push(
        <div
          key={`outer-${o.side}`}
          data-testid="dock-outer-band"
          data-side={o.side}
          style={{ ...canvasBox(r, o.strip), background: HATCH }}
        />,
      )
    }
    const slot = guide.cursorSlot === null ? undefined : r.slots.find(s => s.slot_id === guide.cursorSlot)
    for (const b of slot?.bands ?? []) {
      items.push(
        <div
          key={`band-${b.side}`}
          data-testid="dock-slot-band"
          data-side={b.side}
          style={{ ...canvasBox(r, b.rect), background: HATCH }}
        />,
      )
    }
    const landing = landingOf(guide)
    if (landing !== null) {
      items.push(
        <div
          key="landing"
          data-testid="dock-landing"
          style={{
            ...canvasBox(r, landing),
            boxSizing: 'border-box',
            background: 'color-mix(in srgb, var(--accent) 22%, transparent)',
            border: '2px solid var(--accent)',
          }}
        />,
      )
    }
  }

  const bar = r.tab_bar
  if (bar !== null && shown.type === 'tab_bar') {
    const x = bar.insert_xs[shown.index]
    if (x !== undefined) {
      items.push(
        <div
          key="insert"
          data-testid="dock-tab-insert"
          style={{
            position: 'absolute',
            left: bar.rect.x0 + x - 1,
            top: bar.rect.y0,
            width: 2,
            height: bar.rect.y1 - bar.rect.y0,
            background: 'var(--accent)',
          }}
        />,
      )
    }
  }

  const head = guide.hover && bar ? bar.heads.find(h => h.view_id === guide.hover?.viewId) : undefined
  if (head && guide.hover) {
    items.push(
      <div
        key={`hover-${guide.hover.seq}`}
        data-testid="dock-hover-progress"
        data-view-id={head.view_id}
        style={{
          position: 'absolute',
          left: head.rect.x0,
          top: head.rect.y1 - 2,
          width: head.rect.x1 - head.rect.x0,
          height: 2,
          background: 'var(--accent)',
          transformOrigin: 'left center',
          animation: `engram-dock-hover ${HOVER_SWITCH_MS}ms linear forwards`,
        }}
      />,
    )
  }

  if (items.length === 0) return null
  return (
    <div
      data-testid="dock-overlay"
      style={{ position: 'fixed', inset: 0, pointerEvents: 'none', zIndex: 900, overflow: 'hidden' }}
    >
      <style>{HOVER_KEYFRAMES}</style>
      {items}
    </div>
  )
}

/** 이 창의 지금 가이드를 그린다 — 창마다 하나 마운트한다(`WindowLayout`). */
export default function DockOverlay() {
  return <DockOverlayView guide={useDockGuide()} />
}
