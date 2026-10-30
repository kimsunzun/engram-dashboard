// TRD S21-slot-docking §4-4 「안내」: 끌기 안내를 그 창 캔버스 위 층에 그린다 — 커서 밑 칸의 구역 선(가운데 사각형 + 칸
//   모서리에서 그 모서리로 가는 대각선 넷) · 바깥 띠에 들면 레이아웃 틀 · 고른 띠의 붙을 사각형(옅은 채움 + 얇은 테두리).
//   판정은 하지 않는다 — 받은 `OverlayView` 를 그대로 그린다. 빗금 · 짙은 채움을 쓰지 않는다 — 안내가 칸 내용을 가리지 않게
//   (사용자 결정 2026-10-19).
// 움직임 = VS Code(TRD §9-4 — 나타남 · 사라짐 150 ms 페이드 · 자리 옮김 70 ms). 사라지는 것은 마지막 자리에 남겨 두고 흐려지고,
//   처음 나타나는 붙을 사각형은 옛 자리에서 미끄러져 오지 않고 제자리에서 나타난다. `prefers-reduced-motion: reduce` 면 곧바로.

import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from 'react'

import type { OverlayView, RectCss, SlotZones } from './overlayTypes'

const ACCENT = 'var(--accent)'
const FADE_MS = 150
const MOVE_MS = 70
const REDUCED_MOTION = '(prefers-reduced-motion: reduce)'

function reducedMotion(): boolean {
  return typeof window.matchMedia === 'function' && window.matchMedia(REDUCED_MOTION).matches
}

function rectBox(r: RectCss): CSSProperties {
  return { position: 'absolute', left: r.x0, top: r.y0, width: r.x1 - r.x0, height: r.y1 - r.y0 }
}

/** `value` 가 `null` 이 돼도 마지막 값을 `last` 로 쥔다 · `moving` = 직전 값도 있었다(자리 옮김을 움직일 때). */
function useLast<T>(value: T | null): { last: T | null; moving: boolean } {
  const [s, setS] = useState({ value, last: value, moving: false })
  if (value === s.value) return s
  const next = { value, last: value ?? s.last, moving: value !== null && s.value !== null }
  setS(next)
  return next
}

/**
 * 끌기 안내 — `guide` = 그릴 것(좌표 = 이 층 CSS px) · `null` = 치운다(150 ms 흐려진 뒤 층을 걷는다). 뿌리는
 * `position: absolute; inset: 0` 이라 위치를 가진 상자(그 창 캔버스) 안에 둔다. 늘 마운트해 두고 `null` 로 치운다 — 조건부로
 * 내리면 사라짐 페이드가 없다.
 */
export function DockOverlayView({ guide }: { guide: OverlayView | null }) {
  const reduced = reducedMotion()
  const shown = guide !== null
  const { last } = useLast(guide)
  const [faded, setFaded] = useState(!shown)
  useEffect(() => {
    if (shown) {
      setFaded(false)
      return
    }
    const t = window.setTimeout(() => setFaded(true), FADE_MS)
    return () => window.clearTimeout(t)
  }, [shown])
  const view = guide ?? (reduced || faded ? null : last)
  if (view === null) return null
  return <GuideLayer view={view} shown={shown} fadeMs={reduced ? 0 : FADE_MS} moveMs={reduced ? 0 : MOVE_MS} />
}

function GuideLayer({ view, shown, fadeMs, moveMs }: { view: OverlayView; shown: boolean; fadeMs: number; moveMs: number }) {
  const ref = useRef<HTMLDivElement>(null)
  const [entered, setEntered] = useState(false)
  // 불투명도 0 을 브라우저가 한 번 셈한 뒤 1 로 올린다 — 셈 없이 한 스타일 바뀜 안에서 0 → 1 이면 전환이 일지 않는다.
  useLayoutEffect(() => {
    ref.current?.getBoundingClientRect()
    setEntered(true)
  }, [])
  const slot = useLast(view.slot ?? null)
  const preview = useLast(view.preview)
  const fade = `opacity ${fadeMs}ms ease-out`
  const move = ['left', 'top', 'width', 'height'].map(p => `${p} ${moveMs}ms ease-out`).join(', ')
  return (
    <div
      ref={ref}
      data-testid="dock-overlay"
      style={{
        position: 'absolute',
        inset: 0,
        pointerEvents: 'none',
        overflow: 'hidden',
        opacity: shown && entered ? 1 : 0,
        transition: fade,
      }}
    >
      <svg
        data-testid="dock-slot-lines"
        width="100%"
        height="100%"
        style={{
          position: 'absolute',
          inset: 0,
          stroke: ACCENT,
          strokeOpacity: 0.45,
          strokeWidth: 1,
          fill: 'none',
          opacity: view.slot != null ? 1 : 0,
          transition: fade,
        }}
      >
        {slot.last !== null && <SlotLines zones={slot.last} />}
      </svg>
      <div
        data-testid="dock-frame"
        style={{
          position: 'absolute',
          inset: 0,
          boxSizing: 'border-box',
          border: `2px solid ${ACCENT}`,
          opacity: view.outer.length > 0 ? 1 : 0,
          transition: fade,
        }}
      />
      <div
        data-testid="dock-preview"
        style={{
          ...rectBox(preview.last ?? { x0: 0, y0: 0, x1: 0, y1: 0 }),
          boxSizing: 'border-box',
          background: `color-mix(in srgb, ${ACCENT} 18%, transparent)`,
          border: `1px solid ${ACCENT}`,
          opacity: view.preview !== null ? 1 : 0,
          transition: preview.moving ? `${move}, ${fade}` : fade,
        }}
      />
    </div>
  )
}

function SlotLines({ zones }: { zones: SlotZones }) {
  const { slot: s, centre: c } = zones
  const diagonals = [
    [s.x0, s.y0, c.x0, c.y0],
    [s.x1, s.y0, c.x1, c.y0],
    [s.x1, s.y1, c.x1, c.y1],
    [s.x0, s.y1, c.x0, c.y1],
  ]
  // 가운데는 `<rect>` 가 아니라 닫힌 경로 — 좁은 칸에서 폭 0 이 되면 `<rect>` 는 아무것도 그리지 않는다.
  const centre = `M${c.x0} ${c.y0}H${c.x1}V${c.y1}H${c.x0}Z`
  return (
    <>
      <path data-testid="dock-slot-centre" d={centre} />
      {diagonals.map(([x1, y1, x2, y2], i) => (
        <line key={i} data-testid="dock-slot-diagonal" x1={x1} y1={y1} x2={x2} y2={y2} />
      ))}
    </>
  )
}
