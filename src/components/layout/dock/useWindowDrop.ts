// TRD S21-slot-docking §4-3: receiving drags at the window root — connects this document's dragenter · dragover · dragleave · drop and mousemove to
//   `windowDrop` (where decisions are made) and returns its guide. Filling the outside (store · tab bar geometry · Tauri) through ports is also done here.
// ★Drag listeners are in the `document` bubble phase★ — the agent tree (react-arborist → react-dnd HTML5 backend) overwrites the `dropEffect` of a dragover that is not its own with `none` through a `window` bubble listener
//   (TRD §4-2). Our drag is stopped at `document` so it does not reach there —
//   attached to `window`, the registration order of the same node wins and stopping does not hold, and stopping at `document` capture means the handlers of elements below do not receive it (the same clause of the old
//   HTML5 drag `dockDrag.ts`). The file · URL drop block (`fileDropGuard`) lives on separately as is — the two do not look at each other's drag.
// ★mousemove is `window` capture★ — the guide layer does not receive the pointer, so 「덮개 위 이동」 arrives at the element below it · it is seen even if an app handler stops propagation.

import { useEffect, useLayoutEffect, useMemo, useRef, useSyncExternalStore } from 'react'

import type { ViewMeta } from '../../../api/layoutTypes'
import { useViewStore } from '../../../store/viewStore'
import { askAppIdentifier, currentAppIdentifier, dragSource } from '../tabStrip/useTabDragSource'
import { shownTab } from './dragOverride'
import { localDrag } from './localDrag'
import type { OverlayView } from './overlayTypes'
import { tabDropClient } from './tabDrop'
import { createWindowDrop, type DropModel, type WindowDrop, type WindowDropPorts } from './windowDrop'

const MOVE_LISTEN: AddEventListenerOptions = { capture: true, passive: true }

export interface UseWindowDropOptions {
  label: string
  /** Tab canvas area — the box the guide layer covers. */
  canvas: () => Element | null
}

/** The drag guide to draw in this window now — the `guide` of `DockOverlayView`. */
// ADR-0296
export function useWindowDrop(o: UseWindowDropOptions): OverlayView | null {
  const live = useRef(o)
  useLayoutEffect(() => {
    live.current = o
  })
  const ctl = useMemo(() => createWindowDrop(portsOf(live)), [])

  useEffect(() => {
    askAppIdentifier()
    const off = attach(ctl)
    return () => {
      off()
      ctl.reset()
    }
  }, [ctl])

  return useSyncExternalStore(ctl.subscribe, ctl.getGuide)
}

function attach(ctl: WindowDrop): () => void {
  const enter = (e: DragEvent): void => ctl.dragEnter(e)
  const over = (e: DragEvent): void => ctl.dragOver(e)
  const leave = (e: DragEvent): void => ctl.dragLeave(e)
  const drop = (e: DragEvent): void => ctl.drop(e)
  const move = (e: MouseEvent): void => ctl.mouseMove(e.buttons)
  document.addEventListener('dragenter', enter)
  document.addEventListener('dragover', over)
  document.addEventListener('dragleave', leave)
  document.addEventListener('drop', drop)
  window.addEventListener('mousemove', move, MOVE_LISTEN)
  return () => {
    document.removeEventListener('dragenter', enter)
    document.removeEventListener('dragover', over)
    document.removeEventListener('dragleave', leave)
    document.removeEventListener('drop', drop)
    window.removeEventListener('mousemove', move, MOVE_LISTEN)
  }
}

function portsOf(live: { readonly current: UseWindowDropOptions }): WindowDropPorts {
  // The store sets the tab list to a new array on every apply — if it is the same array, the id list returns the same array too.
  let ids: { from: ViewMeta[]; tabs: string[] } | null = null
  const model = (): DropModel => {
    const w = useViewStore.getState().windows[live.current.label]
    if (w === undefined) return { tabs: [], active: null }
    if (ids === null || ids.from !== w.tabs) ids = { from: w.tabs, tabs: w.tabs.map(t => t.id) }
    return { tabs: ids.tabs, active: w.active }
  }
  return {
    app: currentAppIdentifier,
    local: localDrag,
    canvasRect: () => live.current.canvas()?.getBoundingClientRect() ?? null,
    // The tab that `WindowLayout` draws (same computation as `useShownTab`).
    shownTab: () => {
      const m = model()
      const s = dragSource().getSnapshot()
      return s.window === live.current.label ? shownTab(s.override, m) : m.active
    },
    view: id => useViewStore.getState().layouts[id] ?? null,
    model,
    send: drop => tabDropClient.send(drop),
    setTimer: (fn, ms) => setTimeout(fn, ms),
    clearTimer: h => clearTimeout(h as ReturnType<typeof setTimeout>),
    watchCanvas: onResize => {
      const el = live.current.canvas()
      // In environments without RO such as jsdom (vitest has no setupFiles), it does not observe.
      if (el === null || typeof ResizeObserver === 'undefined') return () => {}
      const ro = new ResizeObserver(() => onResize())
      ro.observe(el)
      return () => ro.disconnect()
    },
  }
}
