// TRD S26-window-chrome §7-3: 보이는 창 번호 표식 — 글자 = 번호만. ★누르는 요소가 아니다★ — `pointer-events: none` 이라 누름이
//   부모(제목 줄 — 끌 자리)로 가고, Tab 순서 밖이다(`tabIndex` 없음). 폭은 자리 수만큼 넓어지고 자르지 않는다(`metrics.ts`).

import type { CSSProperties } from 'react'

import { MARKER_MIN_W, MARKER_PAD, SURFACE_TOP, TAB_BAR_H } from './metrics'

export default function NumberMarker({ n }: { n: number }) {
  return (
    <span data-testid="window-number" style={MARKER}>
      {String(n)}
    </span>
  )
}

// 탭바와 같은 칸(바닥에 붙은 탭바 높이 · 아래 1px 선 · 위 `SURFACE_TOP`)에 글자를 세워 탭 글자와 가운데 높이를 맞춘다.
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
