// TRD S21-slot-docking §4-2 · §4-6: tab head = the source of an OS (HTML5) drag. Converts head events (press · dragstart · dragend) into values and passes them to
//   `dragSource` (where decisions are made — release · revert · send), and registers this tab bar's decision geometry in `localDrag`. The subscriptions that connect the result notice
//   (`tab_drop:done`) · that window's pointerdown · source window tab bar decision changes to `dragSource` are also attached and detached here (tab bar lifetime).
// ★Do not remove the dragged head from under the pressed point inside dragstart★ — after handling dragstart, Chromium re-hit-tests the pressed point and if that head is not
//   there it does not open the OS drag and immediately emits dragend(none) (measured on the old HTML5 drag 15/15 · other DOM changes did not end the drag
//   25/25). So even if it is redrawn with the state dragstart changed, the tab bar DOM stays as is (`DragSourceSnapshot.motion` does not exist yet).
// ★Stop only the dragstart of our drag★ — the HTML5 backend of the agent tree (react-arborist → react-dnd) cancels, with a `window` bubble listener, a dragstart that its own
//   source did not receive (`react-dnd-html5-backend` `handleTopDragStart` — TRD §4-2). dragend is not stopped.

import { getIdentifier } from '@tauri-apps/api/app'
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type DragEvent as ReactDragEvent,
  type MutableRefObject,
  type PointerEvent as ReactPointerEvent,
  type RefObject,
} from 'react'

import type { EndEffect } from '../../../../src-tauri/bindings/EndEffect'
import type { Insets } from '../../../../src-tauri/bindings/Insets'
import type { PointCss } from '../../../../src-tauri/bindings/PointCss'
import type { SizeCss } from '../../../../src-tauri/bindings/SizeCss'
import { MAIN_WINDOW_LABEL, useViewStore } from '../../../store/viewStore'
import { stripZone } from '../dock/bands'
import { setTabDragImage } from '../dock/dragImage'
import { shownTab } from '../dock/dragOverride'
import { dragGrab, dragSizes } from '../dock/dragSize'
import { localDrag, type StripGeometry } from '../dock/localDrag'
import { tabDropClient } from '../dock/tabDrop'
import { buildTabDragType, buildTabDragValue, shapeOf } from '../dock/tabDragData'
import { windowFrame } from '../dock/windowFrame'
import { titleRowHeight } from '../titleRow/metrics'
import { createAppIdentifier } from './appIdentifier'
import { createDragSource, type DragSource, type DragSourcePorts, type DragSourceSnapshot, type DragStart } from './dragSource'
import { measureStrip, stripZoneArea } from './measure'
import { grabOffset, type StripBounds } from './reorderGeometry'

const ports: DragSourcePorts = {
  send: drop => tabDropClient.send(drop),
  switchTabIf: (window, to, expect) => useViewStore.getState().switchTabIf(window, to, expect),
  local: localDrag,
  nextTask: fn => void setTimeout(fn, 0),
}

let source: DragSource = createDragSource(ports)

/** One per webview. */
export function dragSource(): DragSource {
  return source
}

/** Test only — reset module state (isolation between tests). Do not call from production code. */
export function __resetDragSourceForTest(): void {
  source = createDragSource(ports)
}

// One per webview — the contract of the two functions is the same-named methods of `AppIdentifier`.
const appIdentifier = createAppIdentifier(getIdentifier)

export function currentAppIdentifier(): string | null {
  return appIdentifier.current()
}

export function askAppIdentifier(): void {
  appIdentifier.ask()
}

const CAPTURE: AddEventListenerOptions = { capture: true, passive: true }
const NO_INSETS: Insets = { t: 0, r: 0, b: 0, l: 0 }

/** What the tab bar is drawing now — written on every render. */
export interface DrawnStrip {
  /** Drawn order (during a drag = the order carrying the confirmed order move). */
  order: readonly string[]
  bounds: StripBounds
  /** Measured 「머리들이 쓸 수 있는 폭」 (`null` = could not be measured). */
  available: number | null
}

export interface UseTabDragSourceOptions {
  label: string
  /** Tab ids in model order — a new array on every apply to that window's store · otherwise the same reference (change = a tab notice — `DragSource.model`). */
  tabs: readonly string[]
  /** Model active — the value at press time is that drag's `initial`. */
  active: string | null
  /** The tab being name-edited — not dragged. */
  editing: string | null
  /** Drag image label. */
  nameOf: (view: string) => string
  /** Tab bar row (`measure.ts` `StripDom.row`). */
  row: RefObject<HTMLElement | null>
  add: RefObject<HTMLElement | null>
  /** Tab canvas area — measures chrome (viewport − canvas). If absent or size 0, chrome is taken as 0. */
  canvas?: () => Element | null
  /**
   * Receive window facts (`window_frame` · `window:frame`) — default `true`. A waiting window is outside the shell window model, so it is `false` (the shell does not know that window,
   * so the pull fails with `WindowNotFound`). Pulled anew every time it turns `true`.
   */
  pullFrame?: boolean
  /**
   * The title row holding this tab bar (TRD S26-window-chrome §7-9 · R14 · D9) — if present, the vertical extent of the tab bar zone base is from the top to the bottom of this row (even a drop on the top margin above the tabs · `+`
   * still falls in the tab bar). Horizontally, present or not, it is the left of the first tab ~ the right end of `+` — above the empty draggable area · number badge · window buttons is
   * not the zone (`measure.ts` `stripZoneArea`). If absent, the vertical extent is the tab bar row.
   */
  titleRow?: RefObject<HTMLElement | null>
}

export interface TabDragSourceHandle {
  snapshot: DragSourceSnapshot
  /** `true` from the press until that press ends, and until this window's drag (including swap redraw) is released — defers matching the head DOM order. */
  engaged: boolean
  draggable(view: string): boolean
  /** The tab bar writes it on every render — the dragstart computation · decision geometry read it. */
  drawn: MutableRefObject<DrawnStrip | null>
  /**
   * Capture phase of head pointerdown — before the press-moment switch (`switch_tab`) (records the active before the press) · even if the close button inside the head stops bubbling, that press is recorded
   * (material to block a drag started there).
   */
  onPointerDown(e: ReactPointerEvent<HTMLElement>, view: string): void
  onDragStart(e: ReactDragEvent<HTMLElement>, view: string): void
  onDragEnd(e: ReactDragEvent<HTMLElement>, view: string): void
  /** This window's `DragSource.forceSwitch`. */
  forceSwitch(view: string): boolean
}

/** One press — recorded at head pointerdown and used at dragstart. Coordinates = viewport CSS px. */
interface Press {
  view: string
  initial: string | null
  target: EventTarget | null
  point: PointCss
  /** Drawn horizontal range of the head at press time — the basis of the grabbed width ratio (Chrome measures at press). */
  head: { x0: number; x1: number }
}

// ADR-0296
export function useTabDragSource(o: UseTabDragSourceOptions): TabDragSourceHandle {
  const src = source
  const snapshot = useSyncExternalStore(src.subscribe, src.getSnapshot)
  const live = useRef(o)
  useLayoutEffect(() => {
    live.current = o
  })
  const drawn = useRef<DrawnStrip | null>(null)
  const press = useRef<Press | null>(null)

  // Tab state that window's store applied — fed with that rendered value (the window layout draws the same value). The store sets the tab list to a new
  //   array on every apply (`applyWindowTabsUpdated`), so that reference changing is exactly 「알림이 왔다」.
  useLayoutEffect(() => {
    src.model(o.label, { tabs: o.tabs, active: o.active })
  }, [src, o.label, o.tabs, o.active])

  const pullFrame = o.pullFrame ?? true
  useEffect(() => (pullFrame ? windowFrame.install() : undefined), [pullFrame])

  useEffect(() => {
    askAppIdentifier()
    const offDone = tabDropClient.onDone(d => src.done(d))
    const offStrip = localDrag.subscribe(() => src.stripChanged())
    const offGeometry = localDrag.registerStripGeometry(() => geometryOf(live.current, drawn.current))
    const onDown = (): void => src.pointerDown()
    window.addEventListener('pointerdown', onDown, CAPTURE)
    return () => {
      offDone()
      offStrip()
      offGeometry()
      window.removeEventListener('pointerdown', onDown, CAPTURE)
    }
  }, [src])

  const [pressing, setPressing] = usePressing()

  const onPointerDown = useCallback(
    (e: ReactPointerEvent<HTMLElement>, view: string) => {
      if (e.button !== 0) return
      const r = e.currentTarget.getBoundingClientRect()
      press.current = {
        view,
        initial: live.current.active,
        target: e.target,
        point: { x: e.clientX, y: e.clientY },
        head: { x0: r.left, x1: r.right },
      }
      setPressing(true)
    },
    [setPressing],
  )

  const onDragStart = useCallback(
    (e: ReactDragEvent<HTMLElement>, view: string) => {
      const p = press.current?.view === view ? press.current : null
      press.current = null
      // The drag takes over the press — if the engine goes to an OS drag, the pointerup of that press may not arrive.
      setPressing(false)
      const opts = live.current
      const head = e.currentTarget
      const dt = e.dataTransfer
      // A drag started from the name input · close button is not a tab drag (text drag · button press) — the engine emits that drag on the draggable ancestor (the head),
      //   so the pressed spot is looked at separately. It is not our drag so it is not stopped.
      if (opts.editing === view || onControl(head, e.target) || (p !== null && onControl(head, p.target)) || !dt) {
        e.preventDefault()
        return
      }
      const at = Number.isFinite(e.clientX) && Number.isFinite(e.clientY) ? { x: e.clientX, y: e.clientY } : p?.point
      const begin = at === undefined ? null : plan(opts, drawn.current, view, p, at)
      if (begin === null) {
        e.preventDefault()
        return
      }
      e.stopPropagation()
      // ★Carry only this one format★ (head of `tabDragData.ts`) — empty the engine default data first.
      dt.clearData()
      dt.setData(begin.type, begin.value)
      dt.effectAllowed = 'move'
      setTabDragImage(dt, opts.nameOf(view))
      src.start(begin.start)
    },
    [src, setPressing],
  )

  const onDragEnd = useCallback(
    (e: ReactDragEvent<HTMLElement>, view: string) => {
      const effect: EndEffect = e.dataTransfer?.dropEffect === 'move' ? 'move' : 'none'
      src.end(view, effect)
    },
    [src],
  )

  const mine = snapshot.window === o.label && snapshot.override.phase !== 'idle'
  const single = o.label === MAIN_WINDOW_LABEL && o.tabs.length === 1
  return {
    snapshot,
    engaged: pressing || mine,
    // Q35 — the last tab of main does not start a drag (a single-tab popout does — Q34).
    draggable: view => view !== o.editing && !single,
    drawn,
    onPointerDown,
    onDragStart,
    onDragEnd,
    forceSwitch: view => src.forceSwitch(o.label, view),
  }
}

/**
 * The tab that the window layout · tab bar show in that window — while the source window swap-redraws, the remaining tab (R15) · while waiting for the revert, the tab to revert to,
 * otherwise the model active. If `tabs` = `null` (that window's tabs not known yet), the model active.
 */
export function useShownTab(label: string, tabs: readonly string[] | null, active: string | null): string | null {
  const src = source
  return useSyncExternalStore(src.subscribe, () => {
    const s = src.getSnapshot()
    return tabs !== null && s.window === label ? shownTab(s.override, { tabs, active }) : active
  })
}

// A press ends on its release · cancel — listening is attached immediately at press (if attached by effect, a release arriving before would be missed).
function usePressing(): [boolean, (on: boolean) => void] {
  const [pressing, setState] = useState(false)
  const off = useRef<(() => void) | null>(null)
  const set = useCallback((on: boolean) => {
    off.current?.()
    off.current = null
    setState(on)
    if (!on) return
    const up = (): void => set(false)
    window.addEventListener('pointerup', up, CAPTURE)
    window.addEventListener('pointercancel', up, CAPTURE)
    off.current = () => {
      window.removeEventListener('pointerup', up, CAPTURE)
      window.removeEventListener('pointercancel', up, CAPTURE)
    }
  }, [])
  useEffect(() => () => off.current?.(), [])
  return [pressing, set]
}

/** Carrying · size · starting values of dragstart. If they cannot be carried (tab bar unknown · no identifier), `null` — the drag is not opened. */
function plan(
  o: UseTabDragSourceOptions,
  d: DrawnStrip | null,
  view: string,
  p: Press | null,
  at: PointCss,
): { type: string; value: string; start: DragStart } | null {
  const app = appIdentifier.current()
  const row = o.row.current
  const i = d === null ? -1 : d.order.indexOf(view)
  if (app === null || row === null || d === null || i < 0) {
    console.warn('[useTabDragSource] 끌기 자료를 지을 수 없다 — 끌기를 열지 않는다:', { app, row: row !== null, view })
    return null
  }
  const b = d.bounds.tabs[i]
  const stripLeft = row.getBoundingClientRect().left
  const pressed = p?.point ?? at
  const head = p?.head ?? { x0: stripLeft + b.x0, x1: stripLeft + b.x1 }
  const initial = p?.initial ?? o.active
  const w = head.x1 - head.x0
  const ratio = w > 0 ? Math.min(1, Math.max(0, (pressed.x - head.x0) / w)) : 0

  const frame = windowFrame.current()
  const restored = frame?.restored ?? null
  const maximized = frame?.maximized ?? false
  const single = o.label !== MAIN_WINDOW_LABEL && o.tabs.length === 1
  const viewport: SizeCss = { w: window.innerWidth, h: window.innerHeight }
  // The new window (for a single-tab popout, that window being restored) comes up in normal state — the maximized source's title row has no top margin (TRD S26-window-chrome
  //   §7-6), so the new window's chrome and the vertical position of the tab bar are larger and lower by that difference. If not subtracted, the new window comes up shifted from the cursor by that amount.
  const rowGrowth = o.titleRow?.current == null ? 0 : titleRowHeight(false) - titleRowHeight(maximized)
  const sizes = dragSizes({
    restored,
    maximized,
    insets: frame?.insets ?? NO_INSETS,
    viewport,
    work: { w: window.screen.availWidth, h: window.screen.availHeight },
    chrome: chromeOf(o.canvas?.() ?? null, viewport, rowGrowth),
    single,
  })
  // W1 of `dragGrab` = the width of a single-tab head in the new window's tab bar — the new window's (popout) tab bar row is taken to be the whole inner width of that window and `+` the same
  //   width as this window's (a popout has no chrome left or right of the tab bar — `PopoutPage` · [untested] TRD §9-2 · M2 measures it). A maximized single-tab popout is
  //   computed with the restored window width (TRD §3-5).
  const newWidth = sizes.size !== null ? sizes.size.w : maximized && restored !== null ? restored.w : null
  const addW = o.add.current?.getBoundingClientRect().width ?? 0
  const headRoom = newWidth === null ? null : Math.max(0, newWidth - addW)
  const grab = dragGrab({ pressed: { x: pressed.x, y: pressed.y + rowGrowth }, head, stripLeft, headRoom, single, maximized })

  const drag = crypto.randomUUID()
  const layout = useViewStore.getState().layouts[view]?.layout
  return {
    type: buildTabDragType({ app, drag, original: sizes.original, shape: layout === undefined ? null : shapeOf(layout) }),
    value: buildTabDragValue({ drag, view, from: o.label, initial, size: sizes.size, grab }),
    start: {
      window: o.label,
      drag,
      view,
      initial,
      size: sizes.size,
      grab,
      grabOffset: grabOffset(ratio, b.x1 - b.x0),
      left: b.x0,
      x: at.x - stripLeft,
    },
  }
}

/** New window chrome = viewport − tab canvas (negative becomes 0) · `rowGrowth` is added vertically. If the canvas cannot be measured, it is 0. */
function chromeOf(canvas: Element | null, viewport: SizeCss, rowGrowth: number): SizeCss {
  const c = canvas?.getBoundingClientRect()
  if (c === undefined || !(c.width > 0 && c.height > 0)) return { w: 0, h: 0 }
  return { w: Math.max(0, viewport.w - c.width), h: Math.max(0, viewport.h - c.height) + rowGrowth }
}

/** Decision material of this tab bar — the row is measured at that time (so it is right even if the window size changes). */
function geometryOf(o: UseTabDragSourceOptions, d: DrawnStrip | null): StripGeometry | null {
  const row = o.row.current
  if (row === null || d === null) return null
  const m = measureStrip({ row, add: o.add.current })
  const title = o.titleRow?.current?.getBoundingClientRect()
  // Base = first tab ~ `+` × title row (the tab bar row if absent) — to its right is not the zone (`stripZoneArea` · T-66).
  const area = stripZoneArea(m, d.bounds, title === undefined ? null : { top: title.top, bottom: title.bottom })
  return { left: m.left, bounds: d.bounds, available: d.available, zoneViewport: stripZone(area), tabs: d.order }
}

function onControl(head: Element, target: EventTarget | null): boolean {
  const el = target instanceof Element ? target : target instanceof Node ? target.parentElement : null
  const control = el?.closest('input, button')
  return control != null && head.contains(control)
}
