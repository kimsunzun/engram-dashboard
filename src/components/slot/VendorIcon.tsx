// Company icon — the small display of the usage slot draws it instead of the company name (user decision 2026-09-29).
// ★The shapes are placeholders★ — the real shapes are not yet decided, so only the figures are gathered in this file. Color is
//   set by the theme tokens (`--usage-vendor-*`) and figures are painted with `currentColor` (SVG presentation attributes do not accept `var()`).

import type { HTMLAttributes } from 'react'

import type { AgentBackendKind } from '../../api/types'

const SIZE_PX = 16

const COLOR: Record<AgentBackendKind, string> = {
  claude: 'var(--usage-vendor-claude)',
  codex: 'var(--usage-vendor-codex)',
}

const CLAUDE_RAYS: Array<[number, number, number, number]> = [
  [8, 1.4, 8, 4.3],
  [8, 11.7, 8, 14.6],
  [1.4, 8, 4.3, 8],
  [11.7, 8, 14.6, 8],
  [3.2, 3.2, 5.1, 5.1],
  [10.9, 10.9, 12.8, 12.8],
  [12.8, 3.2, 10.9, 5.1],
  [5.1, 10.9, 3.2, 12.8],
]

function Shape({ vendor }: { vendor: AgentBackendKind }) {
  if (vendor === 'claude') {
    return (
      <>
        <g stroke="currentColor" strokeWidth={1.6} strokeLinecap="round">
          {CLAUDE_RAYS.map(([x1, y1, x2, y2]) => (
            <line key={`${x1},${y1}`} x1={x1} y1={y1} x2={x2} y2={y2} />
          ))}
        </g>
        <circle cx={8} cy={8} r={2.1} fill="currentColor" />
      </>
    )
  }
  return (
    <>
      <polygon
        points="8,1.4 13.6,4.75 13.6,11.25 8,14.6 2.4,11.25 2.4,4.75"
        fill="none"
        stroke="currentColor"
        strokeWidth={1.4}
        strokeLinejoin="round"
      />
      <circle cx={8} cy={8} r={2} fill="currentColor" />
    </>
  )
}

/**
 * `label` = the assistive tech name and also the tooltip (`role="img"`). `null` = decoration — there is a name text next to it or it is a hidden measurement copy, so role · name are not
 * carried.
 */
export default function VendorIcon({
  vendor,
  label,
  style,
  ...rest
}: { vendor: AgentBackendKind; label: string | null } & HTMLAttributes<HTMLSpanElement>) {
  const a11y = label === null ? { 'aria-hidden': true } : { role: 'img', 'aria-label': label, title: label }
  return (
    <span {...rest} {...a11y} style={{ display: 'inline-flex', flex: 'none', color: COLOR[vendor], ...style }}>
      <svg width={SIZE_PX} height={SIZE_PX} viewBox="0 0 16 16" aria-hidden="true" focusable="false">
        <Shape vendor={vendor} />
      </svg>
    </span>
  )
}
