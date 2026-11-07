// TRD S26-window-chrome §7-3: the visible window number badge — text = number only. ★It is not a pressable element★ — `pointer-events: none` so a press goes to
//   the parent (the title row — draggable area), and it is outside the Tab order (no `tabIndex`). The width grows with the number of digits and is not clipped (`metrics.ts`).
// It is hidden from screen readers — a bare 「1」 alone is read with no meaning, and the window identity is carried by the OS title (「<탭> - Engram:<n>」 · §4-4).

import type { CSSProperties, Ref } from 'react'

import { MARKER_MIN_W, MARKER_PAD, SURFACE_TOP, TAB_BAR_H } from './metrics'

/** `ref` = the badge element — the title row measures its width and includes it in the minimum window width (§7-7). */
export default function NumberMarker({ n, ref }: { n: number; ref?: Ref<HTMLSpanElement> }) {
  return (
    <span ref={ref} data-testid="window-number" aria-hidden style={MARKER}>
      {String(n)}
    </span>
  )
}

// Stand the text in the same area as the tab bar (tab bar height at the bottom · 1px line below · `SURFACE_TOP` above) to match the vertical center with the tab text.
const MARKER: CSSProperties = {
  flex: '0 0 auto',
  alignSelf: 'flex-end',
  height: TAB_BAR_H,
  minWidth: MARKER_MIN_W,
  boxSizing: 'border-box',
  padding: `${SURFACE_TOP}px ${MARKER_PAD}px 0`,
  borderBottom: '1px solid var(--tab-bar-edge)',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  whiteSpace: 'nowrap',
  fontSize: '12px',
  fontVariantNumeric: 'tabular-nums',
  color: 'var(--chrome-number-fg)',
  pointerEvents: 'none',
}
