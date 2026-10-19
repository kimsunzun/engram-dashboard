// TRD S21-slot-docking §2-5 「안내 창」: 안내 창(`DockOverlayPage`)에 셸이 민 `dock:overlay` 내용을 그린다 — 그 창의 바깥 띠 ·
//   커서 밑 칸의 띠(빗금) · 고른 띠의 붙을 사각형(강조). 판정은 하지 않는다 — 셸이 고른 사각형을 그대로 그린다.

import type { CSSProperties, ReactNode } from 'react'

import type { OverlayView } from '../../../../src-tauri/bindings/OverlayView'
import type { RectCss } from '../../../../src-tauri/bindings/RectCss'

const HATCH =
  'repeating-linear-gradient(45deg, color-mix(in srgb, var(--accent) 30%, transparent) 0 4px, transparent 4px 9px)'

function rectBox(r: RectCss): CSSProperties {
  return { position: 'absolute', left: r.x0, top: r.y0, width: r.x1 - r.x0, height: r.y1 - r.y0 }
}

/** `guide` = 셸이 민 그림(좌표 = 안내 창 뷰포트 CSS px) · `null` = 그릴 것 없음. */
export function DockOverlayView({ guide }: { guide: OverlayView | null }) {
  if (guide === null) return null
  const items: ReactNode[] = []
  guide.outer.forEach((r, i) => {
    items.push(<div key={`outer-${i}`} data-testid="dock-outer-band" style={{ ...rectBox(r), background: HATCH }} />)
  })
  guide.bands.forEach((r, i) => {
    items.push(<div key={`band-${i}`} data-testid="dock-slot-band" style={{ ...rectBox(r), background: HATCH }} />)
  })
  if (guide.preview !== null) {
    items.push(
      <div
        key="preview"
        data-testid="dock-preview"
        style={{
          ...rectBox(guide.preview),
          boxSizing: 'border-box',
          background: 'color-mix(in srgb, var(--accent) 22%, transparent)',
          border: '2px solid var(--accent)',
        }}
      />,
    )
  }
  if (items.length === 0) return null
  return (
    <div data-testid="dock-overlay" style={{ position: 'fixed', inset: 0, pointerEvents: 'none', overflow: 'hidden' }}>
      {items}
    </div>
  )
}
