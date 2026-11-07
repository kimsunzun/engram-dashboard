// TRD S21-slot-docking §4-3 · §2-4: the receiving side of a drag in one window (webview) — decides (`judge`) our tab drag that arrived at the root and records the guide (preview · zone
//   lines · layout frame) · another window's tab bar vertical line · the source window's own tab bar decision, and issues one drop command at drop. Deciding happens only here and
//   it does not know the DOM — connecting document events to these values is `useWindowDrop`.
// ★The drag command is sent only at drop★ (ADR-0296 · ADR-0297) — the only shell call of this module is the `tab_drop` at drop, and for the same window, same place
//   even that is absent. During a drag the outside (window size · tabs · layout) is not blocked and keeps flowing, so the decision always uses current values — the shell looks up the target
//   again at drop.
// ★The guide stands for one drag round only★ — a round is opened by dragenter and closed by drop · window leave (dragleave · no related target) · `GUIDE_IDLE_MS` without dragover ·
//   `GUIDE_IDLE_MS` after mousemove. A dragover draws only for an open round — a dragover arriving late after closing does not revive the guide · vertical line without a new
//   dragenter (TRD §8-3). The closing deadline is fired holding the round it was set for and decided by a single 「지금 회차가 그 회차인가」 —
//   it does not rely on the order in which events · deadlines arrive. Accepting (`preventDefault`) is always done regardless of the round — so that drop arrives.
// Cheaply (S1 handler ≤ 0.5 ms) — bands · validity are recomputed only when the visible tab · its cache · canvas size · tab bar zone changes, and per dragover only
//   tab bar geometry reading and point choosing are done. The canvas is re-measured only at dragenter and on size changes during a round (`watchCanvas`) — on a change it re-decides with the last
//   point, so the preview follows the new size without waiting for dragover.
// ★A drag is not an LLM command★ (TRD §3-6) — it is human input and its measurement, so it is not put on a global handle · bus.

import type { LayoutNode } from '../../../api/layoutTypes'
import type { DropTo } from '../../../../src-tauri/bindings/DropTo'
import type { PointCss } from '../../../../src-tauri/bindings/PointCss'
import type { TabDrop } from '../../../../src-tauri/bindings/TabDrop'
import type { TabDropReply } from '../../../../src-tauri/bindings/TabDropReply'
import { windowBands, zoneInCanvas, type WindowBands } from './bands'
import type { CanvasPx } from './dockedTree'
import { judge, type JudgeStrip, type Judgement } from './judge'
import type { LocalDrag, OutgoingDrag } from './localDrag'
import type { OverlayView, RectCss } from './overlayTypes'
import { readTabDragType, readTabDragValue, type TabDragHead } from './tabDragData'

/** Deadline to clear the guide · vertical line — this long without dragover · this long after mousemove (VS Code style · a value that exists only on the web — TRD §4-5). */
export const GUIDE_IDLE_MS = 300

/** Cache of the visible tab — material for the band computation (fields of `viewStore.CachedView`). It must be the same object for the same cache so bands are not recomputed. */
export interface DropView {
  layout: LayoutNode
  ratioMin: number
  ratioMax: number
}

/** This window's model tab state (store) — `tabs` = tab ids in model order. */
export interface DropModel {
  tabs: readonly string[]
  active: string | null
}

/** The outside — in production `useWindowDrop` fills it with the store · tab bar geometry · Tauri, and tests plug in a fake (ADR-0012). */
export interface WindowDropPorts {
  /** Tauri identifier (lowercase) — `null` = not known yet (no drag is read as ours). */
  app(): string | null
  local: LocalDrag
  /** Tab canvas area (viewport CSS px) — `null` = none. */
  canvasRect(): { left: number; top: number; width: number; height: number } | null
  /** The tab this window's canvas shows now (including the source window's R15 remaining tab — the value `WindowLayout` draws). */
  shownTab(): string | null
  view(id: string): DropView | null
  model(): DropModel
  send(drop: TabDrop): Promise<TabDropReply>
  setTimer(fn: () => void, ms: number): unknown
  clearTimer(handle: unknown): void
  /**
   * Observe canvas size changes — `onResize` is called when the size changes (it may also be called with the same size, e.g. at first observation). The returned function
   * stops it. If there is no canvas, nothing is observed.
   */
  watchCanvas(onResize: () => void): () => void
}

/** Root event — a DOM `DragEvent` fits as is. Coordinates = viewport CSS px. */
export interface RootDragEvent {
  readonly clientX: number
  readonly clientY: number
  readonly relatedTarget: EventTarget | null
  readonly dataTransfer: Pick<DataTransfer, 'types' | 'getData' | 'dropEffect'> | null
  preventDefault(): void
  stopPropagation(): void
}

export interface WindowDrop {
  dragEnter(e: RootDragEvent): void
  dragOver(e: RootDragEvent): void
  dragLeave(e: RootDragEvent): void
  drop(e: RootDragEvent): void
  /** This document's mousemove — ignored if `buttons` is not 0. */
  mouseMove(buttons: number): void
  /** The guide to draw now — a new object only when it changes. */
  getGuide(): OverlayView | null
  subscribe(cb: () => void): () => void
  /** Close the open round (detach) — usable after that too. */
  reset(): void
}

/** Canvas — `size` = rounded size (`null` = no canvas or size 0). If the re-measured value is the same, the object is not replaced (band cache key). */
interface Canvas {
  origin: PointCss
  size: CanvasPx | null
}

/** Band computation of the visible tab — reused if the key (visible tab · its cache · whether it is the dragged tab itself · tab bar zone · canvas) is the same. */
interface Bands {
  shown: string | null
  view: DropView | null
  self: boolean
  zone: RectCss | null
  canvas: Canvas
  /** `null` = no bands (the canvas where the dragged tab itself is visible — `SelfTarget` · no cache of the visible tab · no canvas). */
  bands: WindowBands | null
}

/** One decision and the state used at that time — drop builds the command from this (preview = result — R4 · TRD §4-3). */
interface Decision {
  j: Judgement
  /** That drag that left this webview (this window = source) · otherwise `null`. */
  source: OutgoingDrag | null
  /** The tab bar decided on was the source's own tab bar holding the dragged tab. */
  ownStrip: boolean
  /** Viewport x of tab bar coordinate 0. */
  stripLeft: number
  bands: Bands
}

interface Session {
  head: TabDragHead
  canvas: Canvas
  bands: Bands | null
  /** Last decision — a round is opened by the decision of dragenter, so one always exists in an open round. */
  last: Decision | null
  /** Point of the last decision — if the canvas size changes, it is re-decided with this point. */
  point: PointCss | null
  idle: unknown | null
  mouse: unknown | null
  unwatch: (() => void) | null
}

const NO_CANVAS: Canvas = { origin: { x: 0, y: 0 }, size: null }

// ADR-0296
export function createWindowDrop(ports: WindowDropPorts): WindowDrop {
  const { local } = ports
  let session: Session | null = null
  let guide: OverlayView | null = null
  let guideKey: { bands: WindowBands; type: Judgement['type']; band: number | null; zones: number | null } | null = null
  const listeners = new Set<() => void>()

  function publish(next: OverlayView | null): void {
    if (next === guide) return
    guide = next
    for (const cb of [...listeners]) cb()
  }

  // Is it our drag — if it is the same type string as the open round, it is not re-read (the app key was matched when that round was opened).
  function ours(e: RootDragEvent): TabDragHead | null {
    const dt = e.dataTransfer
    const app = ports.app()
    if (dt === null || app === null) return null
    if (session !== null && dt.types.includes(session.head.mime)) return session.head
    return readTabDragType(dt.types, app)
  }

  function accept(e: RootDragEvent): void {
    e.preventDefault()
    if (e.dataTransfer !== null) e.dataTransfer.dropEffect = 'move'
    e.stopPropagation()
  }

  function measure(): Canvas {
    const r = ports.canvasRect()
    if (r === null) return NO_CANVAS
    const size = { w: Math.round(r.width), h: Math.round(r.height) }
    return { origin: { x: r.left, y: r.top }, size: size.w > 0 && size.h > 0 ? size : null }
  }

  function open(head: TabDragHead): Session {
    return { head, canvas: measure(), bands: null, last: null, point: null, idle: null, mouse: null, unwatch: null }
  }

  // If the re-measured canvas differs, replace it and return `true`.
  function remeasure(s: Session): boolean {
    const c = measure()
    const a = s.canvas
    const same =
      c.origin.x === a.origin.x &&
      c.origin.y === a.origin.y &&
      (c.size === null || a.size === null ? c.size === a.size : c.size.w === a.size.w && c.size.h === a.size.h)
    if (!same) s.canvas = c
    return !same
  }

  function resized(s: Session): void {
    if (session !== s || !remeasure(s) || s.point === null) return
    step(s, s.point)
  }

  function bandsOf(s: Session, source: OutgoingDrag | null, zone: RectCss | null): Bands {
    const shown = ports.shownTab()
    const view = shown === null ? null : ports.view(shown)
    const self = source !== null && shown === source.view
    const c = s.bands
    if (c !== null && c.shown === shown && c.view === view && c.self === self && c.canvas === s.canvas && sameRect(c.zone, zone)) {
      return c
    }
    const size = s.canvas.size
    const bands =
      view === null || self || size === null
        ? null
        : windowBands({
            canvas: size,
            layout: view.layout,
            ratio: { min: view.ratioMin, max: view.ratioMax },
            incoming: s.head.shape,
            original: s.head.original,
            zone,
          })
    const next = { shown, view, self, zone, canvas: s.canvas, bands }
    s.bands = next
    return next
  }

  function decide(s: Session, point: PointCss): Decision {
    // ★Treat it as the source only after comparing the drag id★ — the outgoing drag mark stays until swap redraw is released after dragend.
    const out = local.getOutgoing()
    const source = out !== null && s.head.drag !== null && out.drag === s.head.drag ? out : null
    const g = local.readStripGeometry()
    const zone = g === null ? null : zoneInCanvas(g.zoneViewport, s.canvas.origin)
    const b = bandsOf(s, source, zone)
    let strip: JudgeStrip | null = null
    let dragged = -1
    if (g !== null) {
      if (source !== null) {
        dragged = g.tabs.indexOf(source.view)
        strip = {
          role: 'source',
          left: g.left,
          bounds: g.bounds,
          dragged,
          grabOffset: source.grabOffset,
          available: g.available,
        }
      } else {
        strip = { role: 'other', left: g.left, bounds: g.bounds }
      }
    }
    const j = judge(
      { canvasOrigin: s.canvas.origin, tabBar: strip === null || zone === null ? null : { zone, strip }, bands: b.bands },
      point,
    )
    return { j, source, ownStrip: dragged >= 0, stripLeft: g?.left ?? 0, bands: b }
  }

  function step(s: Session, point: PointCss): void {
    let d = decide(s, point)
    const { j } = d
    if (d.source !== null) {
      // Source window root decision — always written as a new object (the same object is not counted as changed) · `null` = left its own tab bar zone.
      local.setSourceStrip(
        j.type === 'strip' && j.left !== null ? { index: j.index, left: j.left, x: point.x - d.stripLeft } : null,
      )
    }
    local.setTargetMarker(j.type === 'strip' ? j.marker : null)
    // If the write above changed the source window's visible tab (R15 — the remaining tab after leaving the zone · the dragged tab after coming back), re-decide with that tab's bands — so it is not
    //   drawn one dragover late. The tab bar zone decision is independent of the visible tab, so there is nothing to look at again.
    if (j.type !== 'strip' && ports.shownTab() !== d.bands.shown) d = decide(s, point)
    s.last = d
    s.point = point
    publish(guideOf(d))
  }

  function guideOf(d: Decision): OverlayView | null {
    const wb = d.bands.bands
    const { j } = d
    if (wb === null || j.type === 'strip') {
      guideKey = null
      return null
    }
    const band = j.type === 'band' ? j.band : null
    const k = guideKey
    if (k !== null && k.bands === wb && k.type === j.type && k.band === band && k.zones === j.zones) return guide
    guideKey = { bands: wb, type: j.type, band, zones: j.zones }
    const slot = j.zones === null ? null : wb.zones[j.zones]
    if (band === null) return slot === null ? null : { bands: [], outer: [], preview: null, slot }
    const b = wb.bands[band]
    // The layout frame only when an outer band is chosen — the frame stands when this list is not empty (user decision 2026-10-19).
    const outer = b.anchor.type === 'outer' ? wb.bands.filter(x => x.anchor.type === 'outer').flatMap(x => x.areas) : []
    return { bands: [], outer, preview: b.landing, slot }
  }

  function arm(s: Session, slot: 'idle' | 'mouse'): void {
    const prev = s[slot]
    if (prev !== null) ports.clearTimer(prev)
    s[slot] = ports.setTimer(() => {
      if (session === s) end(s)
    }, GUIDE_IDLE_MS)
  }

  function end(s: Session): void {
    if (session !== s) return
    session = null
    if (s.idle !== null) ports.clearTimer(s.idle)
    if (s.mouse !== null) ports.clearTimer(s.mouse)
    s.unwatch?.()
    s.idle = null
    s.mouse = null
    s.unwatch = null
    local.setTargetMarker(null)
    guideKey = null
    publish(null)
  }

  function deliver(head: TabDragHead, d: Decision, dt: Pick<DataTransfer, 'getData'>): void {
    const { j } = d
    let to: DropTo | null = null
    if (j.type === 'strip') {
      let index = j.index
      if (d.ownStrip && d.source !== null) {
        // Source's own tab bar — position to drop = the dragged tab's position in the order drawn now (the position confirmed past the order threshold · not the raw position of the decision).
        //   Re-read at drop — the position the last dragover confirmed is carried into the geometry only in a later drawing.
        const view = d.source.view
        const drawn = local.readStripGeometry()?.tabs.indexOf(view) ?? -1
        if (drawn >= 0) index = drawn
        // ★Whether it is in place is decided by the current store★ (ADR-0297) — if after the last decision the dragged tab went to another window (LLM), the model seen then is still
        //   「제자리」, so not sending would leave the tab in that window. If it does not hold, send it and the shell looks it up again by id.
        if (index === ports.model().tabs.indexOf(view)) {
          // Same window, same place — do not send, and record it so the source dragend releases immediately (TRD §2-4 ④).
          local.markLocalDrop(d.source.drag)
          return
        }
      }
      to = { type: 'tab_bar', index }
    } else if (j.type === 'band') {
      const b = d.bands.bands?.bands[j.band]
      const target = d.bands.shown
      if (b === undefined || target === null) {
        console.warn('[windowDrop] 띠 놓기의 대상을 모른다 — 보내지 않는다:', { target })
        return
      }
      to = { type: 'seam', target, anchor: b.anchor, side: b.side, share: b.share }
    }
    const value = readTabDragValue(dt.getData(head.mime))
    if (value === null) {
      console.warn('[windowDrop] 끌기 자료의 값을 못 읽어 놓기를 보내지 않는다:', head.mime)
      return
    }
    to ??= { type: 'new_window', by: { type: 'drop' }, size: value.size, grab: value.grab }
    ports.send({ drag: value.drag, view: value.view, from: value.from, initial: value.initial, to }).then(
      // The result is received by the source as `tab_drop:done` (TRD §4-6) — here we only leave it.
      reply => console.debug('[windowDrop] tab_drop(drop) 답:', reply),
      err => console.warn('[windowDrop] tab_drop(drop) 실패:', err),
    )
  }

  return {
    dragEnter(e) {
      const head = ours(e)
      if (head === null) return
      accept(e)
      let s = session
      if (s !== null && s.head.mime !== head.mime) {
        end(s)
        s = null
      }
      if (s === null) {
        const opened = open(head)
        s = opened
        session = s
        opened.unwatch = ports.watchCanvas(() => resized(opened))
      } else {
        remeasure(s)
      }
      step(s, { x: e.clientX, y: e.clientY })
      arm(s, 'idle')
    },

    dragOver(e) {
      const head = ours(e)
      if (head === null) return
      accept(e)
      const s = session
      if (s === null || s.head.mime !== head.mime) return
      step(s, { x: e.clientX, y: e.clientY })
      arm(s, 'idle')
    },

    dragLeave(e) {
      const head = ours(e)
      if (head === null) return
      e.stopPropagation()
      // If there is a related target, it crossed between elements in the document — Chromium emits the new element's dragenter first.
      if (e.relatedTarget !== null) return
      const out = local.getOutgoing()
      if (out !== null && head.drag !== null && out.drag === head.drag) local.setSourceStrip(null)
      if (session !== null) end(session)
    },

    drop(e) {
      const head = ours(e)
      if (head === null) return
      // ★For our drag the first thing is `preventDefault`★ — so that the source dragend is `move` (TRD §4-3). Before it there is only reading the type string.
      e.preventDefault()
      e.stopPropagation()
      const s = session
      // If there is no open round (a deadline · mousemove closed it), decide that position now — do not discard that drop.
      const d = s !== null && s.head.mime === head.mime && s.last !== null ? s.last : decide(open(head), { x: e.clientX, y: e.clientY })
      if (s !== null) end(s)
      if (e.dataTransfer !== null) deliver(head, d, e.dataTransfer)
    },

    mouseMove(buttons) {
      const s = session
      // Filter out moves with the primary button held — they are synthetic · delayed moves during a drag (M11).
      if (s === null || buttons !== 0 || s.mouse !== null) return
      arm(s, 'mouse')
    },

    getGuide: () => guide,

    subscribe(cb) {
      const entry = (): void => cb()
      listeners.add(entry)
      return () => {
        listeners.delete(entry)
      }
    },

    reset() {
      if (session !== null) end(session)
    },
  }
}

function sameRect(a: RectCss | null, b: RectCss | null): boolean {
  if (a === null || b === null) return a === b
  return a.x0 === b.x0 && a.y0 === b.y0 && a.x1 === b.x1 && a.y1 === b.y1
}
