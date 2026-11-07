// TRD S21-slot-docking §4-4 「안내」: draws the drag guide on the layer over that window's canvas — the zone lines of the area under the cursor (a center rectangle + four diagonals from the area
//   corners to that corner) · the layout frame when inside an outer band · the rectangle to attach for the chosen band (light fill + thin border).
//   It does not decide — it draws the received `OverlayView` as is. No hatching · dark fill — so the guide does not hide area content
//   (user decision 2026-10-19).
// Motion = VS Code (TRD §9-4 — appear · disappear 150 ms fade · move 70 ms). A disappearing one stays at its last position and fades,
//   and a rectangle to attach appearing for the first time does not slide in from an old position but appears in place. With `prefers-reduced-motion: reduce`, immediately.

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

/** Even when `value` becomes `null`, the last value is held as `last` · `moving` = a previous value also existed (when animating a move). */
function useLast<T>(value: T | null): { last: T | null; moving: boolean } {
  const [s, setS] = useState({ value, last: value, moving: false })
  if (value === s.value) return s
  const next = { value, last: value ?? s.last, moving: value !== null && s.value !== null }
  setS(next)
  return next
}

/**
 * Drag guide — `guide` = what to draw (coordinates = this layer's CSS px) · `null` = clear (the layer is withdrawn after the 150 ms fade). The root is
 * `position: absolute; inset: 0`, so place it inside a positioned box (that window's canvas). Keep it always mounted and clear with `null` — taking it down
 * conditionally loses the disappear fade.
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
  // After the browser has computed opacity 0 once, raise it to 1 — if it goes 0 → 1 within one style change without that computation, no transition occurs.
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
  // The center is a closed path, not a `<rect>` — if width becomes 0 in a narrow area, a `<rect>` draws nothing.
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
