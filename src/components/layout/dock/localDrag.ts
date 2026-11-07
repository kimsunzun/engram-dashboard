// TRD S21-slot-docking §4-2 · §4-3: the state between hooks of the tab drag, one per webview (JS context) — holds only what the drag source (`tabStrip/useTabDragSource`) and
//   the root receiver (`dock/useWindowDrop`) exchange **within the same webview**. Nothing crosses between windows — all another window
//   knows is the type string of the drag data (ADR-0296 · TRD §2-2).
// One writer per field: `outgoing` = the source (set at dragstart and withdrawn when swap redraw is released) · `sourceStrip` = the source window's root
//   dragover (only the first value is placed by the source dragstart — `setSourceStrip` below) · `targetMarker` = another window's root dragover (drawn by `TabBar`) ·
//   in-place drop mark = written by the root drop and taken by the source dragend · tab bar geometry = registered by `TabBar` and read by the root.
// ★A drag is not an LLM command★ (TRD §3-6) — it is human input and its measurement, so it is not put on a global handle · bus.

import { useSyncExternalStore } from 'react'

import type { StripBounds } from '../tabStrip/reorderGeometry'
import type { RectCss } from './overlayTypes'

/** The drag that left this webview — exists only in the source window. */
export interface OutgoingDrag {
  drag: string
  /** Source window label. */
  window: string
  /** The dragged tab. */
  view: string
  /** `JudgeStrip.source.grabOffset` — the value computed once at dragstart (`reorderGeometry.grabOffset`). */
  grabOffset: number
}

/**
 * Decision on the source window's own tab bar (the `strip` answer of `judge`) — tab bar coordinates (0 = `StripGeometry.left`). `index` = position in the list without the dragged tab ·
 * `left` = left of the dragged head (`draggedLeft`) · `x` = cursor x of that dragover (material for the order threshold `moveDue` — `left` is
 * clipped at the end of the drag area and cannot be recovered from the cursor).
 */
export interface SourceStripMove {
  index: number
  left: number
  x: number
}

/**
 * Decision material for this webview's tab bar — exactly the order `TabBar` is drawing now (during a drag = the order carrying the confirmed order move). Coordinates = viewport
 * CSS px. The position to drop on the source window's own tab bar is `tabs.indexOf(끄는 탭)` (the confirmed position — not the raw answer of `judge`).
 */
export interface StripGeometry {
  /** Tab bar coordinate 0 = the viewport x of the left of the tab drag area (left of the first tab). */
  left: number
  /** Ideal bounds of the order drawn now (including the dragged tab · the dragged tab is counted as a visible tab even when hidden). */
  bounds: StripBounds
  /** `available` of `dragAreaWidth` — the measured value (`null` = could not be measured). */
  available: number | null
  /**
   * Tab bar zone = `STRIP_ZONE_PX` (`bands.stripZone`) above and below the base rectangle (left of the first tab ~ right end of `+` × the title row if the tab bar is inside the title row · otherwise the tab bar row — `tabStrip/measure.ts`
   * `stripZoneArea`) — viewport coordinates.
   */
  zoneViewport: RectCss
  /** Tab ids in `bounds.tabs` order. */
  tabs: readonly string[]
}

/** The field subscribers read — a new object each time the field changes. */
export interface LocalDragSnapshot {
  outgoing: OutgoingDrag | null
  sourceStrip: SourceStripMove | null
  targetMarker: number | null
}

export interface LocalDrag {
  getOutgoing(): OutgoingDrag | null
  setOutgoing(d: OutgoingDrag | null): void
  /**
   * On every source window root dragover, record that decision — the `strip` answer if inside its own tab bar zone · `null` if outside (including a dragleave leaving the window).
   * The source dragstart places the first value of 「누른 점 = 구역 안」 — so `null` always means 「구역을 떠남」.
   */
  setSourceStrip(m: SourceStripMove | null): void
  getSourceStrip(): SourceStripMove | null
  /** x of the 2 px vertical line of another window's tab bar (tab bar coordinates · `dropMarkerX`) — `null` = none. */
  setTargetMarker(x: number | null): void
  getTargetMarker(): number | null
  /** An in-place drop (same window, same place) where the root drop sent nothing (TRD §2-4 ④) — the source dragend takes it with `takeLocalDrop`. */
  markLocalDrop(drag: string): void
  /** If that drag had the mark, clear it and return `true` (only once). */
  takeLocalDrop(drag: string): boolean
  /** Return = detach — the last one registered is read. */
  registerStripGeometry(fn: () => StripGeometry | null): () => void
  /** `null` if there is no tab bar. */
  readStripGeometry(): StripGeometry | null
  /** Every time the field changes (not called if the same value is written again). Return = detach. */
  subscribe(cb: () => void): () => void
  getSnapshot(): LocalDragSnapshot
}

export function createLocalDrag(): LocalDrag {
  let snap: LocalDragSnapshot = { outgoing: null, sourceStrip: null, targetMarker: null }
  let localDrop: string | null = null
  const geometries: Array<{ fn: () => StripGeometry | null }> = []
  const listeners = new Set<() => void>()

  const write = <K extends keyof LocalDragSnapshot>(key: K, value: LocalDragSnapshot[K]): void => {
    if (Object.is(snap[key], value)) return
    snap = { ...snap, [key]: value }
    for (const cb of [...listeners]) cb()
  }

  return {
    getOutgoing: () => snap.outgoing,
    setOutgoing: d => write('outgoing', d),
    setSourceStrip: m => write('sourceStrip', m),
    getSourceStrip: () => snap.sourceStrip,
    setTargetMarker: x => write('targetMarker', x),
    getTargetMarker: () => snap.targetMarker,
    markLocalDrop: drag => {
      localDrop = drag
    },
    takeLocalDrop: drag => {
      if (localDrop !== drag) return false
      localDrop = null
      return true
    },
    registerStripGeometry(fn) {
      const entry = { fn }
      geometries.push(entry)
      return () => {
        const at = geometries.indexOf(entry)
        if (at >= 0) geometries.splice(at, 1)
      }
    },
    readStripGeometry: () => geometries[geometries.length - 1]?.fn() ?? null,
    subscribe(cb) {
      const entry = (): void => cb()
      listeners.add(entry)
      return () => {
        listeners.delete(entry)
      }
    },
    getSnapshot: () => snap,
  }
}

/** One per webview. */
export const localDrag: LocalDrag = createLocalDrag()

/** `select` must return the field as is — creating a new object makes it read as a changed value on every draw. */
export function useLocalDrag<T>(select: (s: LocalDragSnapshot) => T, store: LocalDrag = localDrag): T {
  return useSyncExternalStore(store.subscribe, () => select(store.getSnapshot()))
}
