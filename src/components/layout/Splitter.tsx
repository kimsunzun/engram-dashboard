// ADR-0227: drag handle for one split boundary. It does not know the shell — it turns the pointer into a ratio and reports it by callback; the preview state, `set_split_ratio` and reconciliation belong to the caller (the renderer using `splitPreview.ts`).
// An overlay that takes no layout space — paired with the shell's area px computation not subtracting the divider thickness (TRD §2d).

import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type PointerEvent as ReactPointerEvent,
  type RefObject,
} from 'react'

import type { SplitDir, SplitRect } from '../../api/layoutTypes'
import { ratioFromPointer } from './splitPreview'

const HIT_PX = 8
const HOVER_DELAY_MS = 300

/**
 * Callback contract: after `onDragStart` exactly one of `onRelease` and `onCancel` comes (including unmount). A click released without movement calls no callback; a press by another pointer during a press is ignored.
 */
export interface SplitterProps {
  /** The current rectangle of this split — a preview remap may have been applied. */
  split: SplitRect
  /** Allowed ratio range (`ratioRange`). null = locked — drag and double-click are both ignored. */
  range: { lo: number; hi: number } | null
  /** The flat root. On press, `getBoundingClientRect()` is taken once to fix the pointer origin and size. */
  rootRef: RefObject<HTMLElement | null>
  /**
   * Once, when it first actually moves along the split axis — carries the in-range ratio there (the preview's start value). A bare press does not call it; this value does not come again through `onPreview`.
   */
  onDragStart(ratio: number): void
  /** Ratio during the drag — at most once per frame, and only when different from the last reported value (including `onDragStart`). */
  onPreview(ratio: number): void
  /** Once on release after moving. It flushes the pending frame through `onPreview` first, so it equals the last reported value. */
  onRelease(ratio: number): void
  /**
   * During a drag: pointercancel, window blur, unmount, a pointermove with the primary button up, a new pointerdown by the same pointer (on any divider), or this split being locked, changing direction or changing split id.
   */
  onCancel(): void
  /** Double-click (when not locked). The caller sends 0.5 without a preview. */
  onReset(): void
}

interface Press {
  pointerId: number
  splitId: string
  dir: SplitDir
  el: HTMLElement
  /** The root's split-axis start coordinate and length captured on press (CSS px). */
  origin: number
  size: number
  start: number
  last: number
  moved: boolean
  frame: number | null
  previewed: number | null
}

const axisOf = (dir: SplitDir, e: { clientX: number; clientY: number }): number =>
  dir === 'left_right' ? e.clientX : e.clientY

// Drags can overlap (two dividers, two pointers). The first drag saves the original body style once and the last restores it — per-drag save/restore would let the first to end release the lock of the remaining drag, or let the last revive another drag's lock value as the "original".
const bodyLocks = new Map<object, string>()
let bodyOriginal: { cursor: string; userSelect: string } | null = null

function lockBody(owner: object, cursor: string): void {
  if (bodyLocks.size === 0) {
    bodyOriginal = { cursor: document.body.style.cursor, userSelect: document.body.style.userSelect }
  }
  bodyLocks.set(owner, cursor)
  document.body.style.cursor = cursor
  document.body.style.userSelect = 'none'
}

function unlockBody(owner: object): void {
  if (!bodyLocks.delete(owner)) return
  if (bodyLocks.size > 0) {
    const cursors = [...bodyLocks.values()]
    document.body.style.cursor = cursors[cursors.length - 1]
    return
  }
  if (bodyOriginal !== null) {
    document.body.style.cursor = bodyOriginal.cursor
    document.body.style.userSelect = bodyOriginal.userSelect
    bodyOriginal = null
  }
}

// pointerId → cancel function of the divider holding that pointer's press, shared among dividers so a new press by the same pointer on another divider can close the earlier press.
const pointerOwners = new Map<number, () => void>()

// A window-level listener must be the same function to detach, so it is created once per instance and the latest props are read through a ref.
function createDrag(latest: { current: SplitterProps }, setDragging: (v: boolean) => void) {
  let press: Press | null = null

  const ratioNow = (p: Press): number | null => {
    const { split, range } = latest.current
    if (range === null || split.split_id !== p.splitId || split.dir !== p.dir) return null
    const box0 = p.dir === 'left_right' ? split.x0 : split.y0
    const box1 = p.dir === 'left_right' ? split.x1 : split.y1
    return ratioFromPointer({ pointerNorm: (p.last - p.origin) / p.size, box0, box1, range })
  }

  const flush = (p: Press): void => {
    const r = ratioNow(p)
    if (r === null || r === p.previewed) return
    p.previewed = r
    latest.current.onPreview(r)
  }

  const end = (p: Press): void => {
    press = null
    window.removeEventListener('pointermove', onMove, true)
    window.removeEventListener('pointerup', onUp, true)
    window.removeEventListener('pointercancel', onPointerCancel, true)
    window.removeEventListener('blur', abort)
    if (p.frame !== null) cancelAnimationFrame(p.frame)
    try {
      if (p.el.hasPointerCapture?.(p.pointerId)) p.el.releasePointerCapture(p.pointerId)
    } catch {
      // The capture may already be released (e.g. after crossing a webview boundary) — if released there is nothing to do.
    }
    unlockBody(p)
    if (pointerOwners.get(p.pointerId) === abort) pointerOwners.delete(p.pointerId)
    if (p.moved) setDragging(false)
  }

  function abort(): void {
    const p = press
    if (p === null) return
    end(p)
    if (p.moved) latest.current.onCancel()
  }

  function onPointerCancel(e: PointerEvent): void {
    if (press !== null && e.pointerId === press.pointerId) abort()
  }

  function onMove(e: PointerEvent): void {
    const p = press
    if (p === null || e.pointerId !== p.pointerId) return
    // A move without the primary button means pointerup was missed or the primary was released while another button was held — no basis to commit, and following it would drag with the button released and commit an arbitrary ratio, so close it as a cancel. Per the Pointer Events spec touch and pen-tip contact also set bit 0.
    if ((e.buttons & 1) === 0) {
      abort()
      return
    }
    // pointerdown is not prevented (onPointerDown below), so the default action during the press-and-drag is prevented here.
    e.preventDefault()
    p.last = axisOf(p.dir, e)
    if (!p.moved) {
      if (p.last === p.start) return
      const r = ratioNow(p)
      if (r === null) {
        end(p)
        return
      }
      p.moved = true
      p.previewed = r
      lockBody(p, p.dir === 'left_right' ? 'col-resize' : 'row-resize')
      setDragging(true)
      latest.current.onDragStart(r)
      return
    }
    if (ratioNow(p) === null) {
      abort()
      return
    }
    if (p.frame === null) {
      p.frame = requestAnimationFrame(() => {
        p.frame = null
        if (press === p) flush(p)
      })
    }
  }

  function onUp(e: PointerEvent): void {
    const p = press
    if (p === null || e.pointerId !== p.pointerId) return
    // Released value = the last pointermove position, not the pointerup coordinates, so the commit equals the last preview (no jump on release).
    const r = p.moved ? ratioNow(p) : null
    end(p)
    if (!p.moved) return
    if (r === null) {
      latest.current.onCancel()
      return
    }
    if (r !== p.previewed) {
      p.previewed = r
      latest.current.onPreview(r)
    }
    latest.current.onRelease(r)
  }

  const onPointerDown = (e: ReactPointerEvent<HTMLElement>): void => {
    if (e.button !== 0) return
    // The same pointer cannot press again without releasing — its earlier press missed pointerup, so close it as a cancel whichever divider owns it; otherwise on another divider one pointer would drag two splits together, and on the same divider a click without movement would commit the earlier drag's last value.
    pointerOwners.get(e.pointerId)?.()
    if (press !== null) return
    const { split, range, rootRef } = latest.current
    const root = rootRef.current
    if (range === null || root === null) return
    const rect = root.getBoundingClientRect()
    const dir = split.dir
    const size = dir === 'left_right' ? rect.width : rect.height
    if (!(size > 0)) return
    // Do not preventDefault — it removes the compat mousedown, so open menus and popups that close on document mousedown (SlotContextMenu·AgentList·PresetPalette) would not close on a divider press. Text selection is blocked by the divider's `user-select: none`, the body lock during the drag and pointermove's preventDefault.
    const start = axisOf(dir, e)
    const el = e.currentTarget
    press = {
      pointerId: e.pointerId,
      splitId: split.split_id,
      dir,
      el,
      origin: dir === 'left_right' ? rect.left : rect.top,
      size,
      start,
      last: start,
      moved: false,
      frame: null,
      previewed: null,
    }
    pointerOwners.set(e.pointerId, abort)
    // Some environments such as jsdom have no pointer capture, so it is called optionally — the window-level listeners alone keep the drag running.
    try {
      el.setPointerCapture?.(e.pointerId)
    } catch {
      // A capture failure is not fatal — the window-level listeners below receive release and cancel.
    }
    // Chromium briefly loses capture even with a button held (orca `pane-divider-drag.ts`), so the drag is also held at window level through pointerup, pointercancel and blur; `lostpointercapture` is not a cancel.
    window.addEventListener('pointermove', onMove, true)
    window.addEventListener('pointerup', onUp, true)
    window.addEventListener('pointercancel', onPointerCancel, true)
    // blur is not registered in the capture phase — it would also receive blur of elements inside the window and cut the drag. Only the window's own blur is needed.
    window.addEventListener('blur', abort)
  }

  /** Called after render — if it got locked during the press or the direction or split id changed, the drag is reaped. */
  const revalidate = (): void => {
    if (press !== null && ratioNow(press) === null) abort()
  }

  return { onPointerDown, revalidate, dispose: abort }
}

export default function Splitter(props: SplitterProps) {
  const { split, range } = props
  const latest = useRef(props)
  useLayoutEffect(() => {
    latest.current = props
  })
  const [dragging, setDragging] = useState(false)
  const [hovered, setHovered] = useState(false)
  const hoverTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const [drag] = useState(() => createDrag(latest, setDragging))

  const locked = range === null
  useEffect(() => {
    drag.revalidate()
  }, [drag, split.split_id, split.dir, locked])

  useEffect(
    () => () => {
      drag.dispose()
      if (hoverTimer.current !== null) clearTimeout(hoverTimer.current)
    },
    [drag],
  )

  const leftRight = split.dir === 'left_right'
  const active = !locked && (dragging || hovered)
  const style: CSSProperties & Record<`--${string}`, number> = {
    '--x0': split.x0,
    '--y0': split.y0,
    '--x1': split.x1,
    '--y1': split.y1,
    '--at': split.at,
    position: 'absolute',
    zIndex: 20,
    touchAction: 'none',
    userSelect: 'none',
    cursor: locked ? 'default' : leftRight ? 'col-resize' : 'row-resize',
    ...(leftRight
      ? {
          left: `calc(var(--at) * 100% - ${HIT_PX / 2}px)`,
          width: HIT_PX,
          top: 'calc(var(--y0) * 100%)',
          height: 'calc((var(--y1) - var(--y0)) * 100%)',
        }
      : {
          top: `calc(var(--at) * 100% - ${HIT_PX / 2}px)`,
          height: HIT_PX,
          left: 'calc(var(--x0) * 100%)',
          width: 'calc((var(--x1) - var(--x0)) * 100%)',
        }),
  }

  return (
    <div
      role="separator"
      aria-orientation={leftRight ? 'vertical' : 'horizontal'}
      data-split-id={split.split_id}
      data-dir={split.dir}
      style={style}
      onPointerDown={drag.onPointerDown}
      onDoubleClick={() => {
        if (latest.current.range !== null) latest.current.onReset()
      }}
      onPointerEnter={() => {
        if (hoverTimer.current !== null) clearTimeout(hoverTimer.current)
        hoverTimer.current = setTimeout(() => {
          hoverTimer.current = null
          setHovered(true)
        }, HOVER_DELAY_MS)
      }}
      onPointerLeave={() => {
        if (hoverTimer.current !== null) clearTimeout(hoverTimer.current)
        hoverTimer.current = null
        setHovered(false)
      }}
    >
      <div
        aria-hidden
        style={{
          position: 'absolute',
          pointerEvents: 'none',
          background: active ? 'var(--accent)' : 'var(--border)',
          ...(leftRight
            ? { top: 0, bottom: 0, left: 'calc(50% - 0.5px)', width: 1 }
            : { left: 0, right: 0, top: 'calc(50% - 0.5px)', height: 1 }),
        }}
      />
    </div>
  )
}
