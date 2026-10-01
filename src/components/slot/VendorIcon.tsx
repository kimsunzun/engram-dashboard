// 회사 아이콘 — 사용량 슬롯의 작은 표시가 회사 이름 대신 그린다(사용자 결정 2026-09-29).
// ★모양은 자리표시다★ — 실제 모양은 확정 전이라 도형만 이 파일에 모아 둔다. 색은 테마 토큰(`--usage-vendor-*`)이
//   정하고 도형은 `currentColor` 로 칠한다(SVG 표현 속성은 `var()` 를 받지 않는다).

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
 * `label` = 보조기술 이름이자 툴팁(`role="img"`). `null` = 장식 — 옆에 이름 글자가 있거나 숨은 측정 사본이라 역할·이름을
 * 싣지 않는다.
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
