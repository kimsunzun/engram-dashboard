// ADR-0227: divider-drag wiring — attaches gesture token issuing, the `set_split_ratio` commit, the post-commit pull timer and the allowed range to the preview state (`splitPreview.ts`) and hands it out as `Splitter` props. Reconcile rules live in `reducePreview` (TRD §2f). Two decisions added here: on release, do not commit if another drag has taken the preview; tie the preview and the commit's reply and pull to the view that started them.
// Not on the frontend command registry (`window.__engramCmd`) — the LLM path is the bus command `split.setRatio`.

import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
  useSyncExternalStore,
  type RefObject,
} from 'react'
import { invoke } from '@tauri-apps/api/core'

import type { SlotRect, SplitRect, ViewSnapshot } from '../../api/layoutTypes'
import { useViewStore } from '../../store/viewStore'
import { retryAsync, RetryCancelledError } from '../../util/retryInvoke'
import type { SplitterProps } from './Splitter'
import {
  COMMIT_PULL_DELAY_MS,
  MIN_PANE_PX,
  needsPull,
  ratioRange,
  reducePreview,
  remapRects,
  type PreviewEvent,
  type PreviewState,
} from './splitPreview'
import { getCanvasReport, subscribeCanvasReport, type CanvasPx } from './windowCanvasReport'

/**
 * Inputs are all this view's cache (`CachedView`) as is — the shell snapshot is the authority.
 * ★Pass `slotRects` and `splitRects` as the cache's arrays as they are (MUST), never sorted or copied★: a changed array reference is treated as a new snapshot and reconciled, and the result's guarantee "unchanged rects and arrays keep the input's reference" holds only if input references are stable. For drawing order (id order), sort this hook's *result*.
 */
export interface UseSplitDragArgs {
  viewId: string
  slotRects: SlotRect[]
  splitRects: SplitRect[]
  ratioMin: number
  ratioMax: number
  /** Cache version — when it changes, it is treated as a new snapshot and reconciled. */
  version: number
  /** Flat root. Before the canvas report, the axis length of the allowed range is measured here. */
  rootRef: RefObject<HTMLElement | null>
}

export interface UseSplitDragResult {
  /** Rects to draw — values with the preview remap applied. Unchanged rects and arrays are the same references as the input. */
  slotRects: SlotRect[]
  splitRects: SplitRect[]
  /** `split` = one item of `splitRects` above (the remapped value); spread it onto `<Splitter key={split.split_id} …/>`. `range` is null (locked) if the snapshot has no such split id. */
  splitterProps(split: SplitRect): SplitterProps
}

type Range = ReturnType<typeof ratioRange>

const readCanvas = (): CanvasPx | null => getCanvasReport().lastSent

/** The preview and the view it belongs to. An event carries the view that emitted it; one not from the current view is dropped. */
interface Scoped {
  viewId: string
  preview: PreviewState
}

type ScopedAction = { type: 'event'; viewId: string; ev: PreviewEvent } | { type: 'reset'; viewId: string }

// If nothing changes it returns the same object — `send` uses that to filter out no-op dispatches.
function reduceScoped(s: Scoped, a: ScopedAction): Scoped {
  if (a.type === 'reset') {
    return s.viewId === a.viewId && s.preview === null ? s : { viewId: a.viewId, preview: null }
  }
  if (a.viewId !== s.viewId) return s
  const preview = reducePreview(s.preview, a.ev)
  return preview === s.preview ? s : { viewId: s.viewId, preview }
}

const initScoped = (viewId: string): Scoped => ({ viewId, preview: null })

/**
 * Divider drag of one view — one preview per view.
 * - On release it calls `set_split_ratio` once — unless a new drag of another divider (two pointers) took the preview meanwhile: the value on screen then belongs to that drag, and sending would commit a value never shown.
 * - A double-click sends 0.5 with no preview.
 * - If no snapshot of that version arrives within `COMMIT_PULL_DELAY_MS` after `Applied`, it pulls `get_view` once.
 * - Axis length of the allowed range = the integer canvas this window reported to the shell (before a report, the root size rounded) × the snapshot split box width — the same input as the shell px clamp, so the committed value is not clipped again.
 * - If `viewId` changes on the same instance, the preview and any gesture in progress are discarded; the reply and pull of an old view's commit never reach the new view.
 */
export function useSplitDrag(args: UseSplitDragArgs): UseSplitDragResult {
  const { viewId, slotRects, splitRects, ratioMin, ratioMax, version, rootRef } = args

  const [scoped, dispatch] = useReducer(reduceScoped, viewId, initScoped)
  // Even in the render where the view has just changed (before the reset effect below), the old view's preview is not drawn.
  const preview = scoped.viewId === viewId ? scoped.preview : null
  // Folds sent actions right away = the state React will eventually draw (same pure function, same order). Being ahead of render lets release decide "does this gesture still own the preview?" without waiting for a render.
  // ★Never call `dispatch` directly; always go through `send`★ — bypassing it even once silently throws off that ownership decision.
  const latest = useRef<Scoped>(scoped)
  const send = useCallback((a: ScopedAction) => {
    const next = reduceScoped(latest.current, a)
    // An action that changes nothing is not sent — dropping no-change steps from the ordered fold of a pure function leaves the later state the same.
    if (next === latest.current) return
    latest.current = next
    dispatch(a)
  }, [])

  const nextToken = useRef(0)
  // split id → in-progress gesture token of that divider; held per divider so overlapping drags of two pointers or an unmount cancel do not touch someone else's preview.
  const gestures = useRef(new Map<string, number>())

  // Only decides whether to cancel the pull; the commit reply is not blocked by it — with effects cut but state alive (`<Activity mode="hidden">`), discarding the reply would trap the preview in `commit`.
  const mounted = useRef(false)
  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
    }
  }, [])

  // Runs before snapshot reconcile (declaration order) — so the changed view's snapshot is not fed to the old preview.
  useLayoutEffect(() => {
    if (latest.current.viewId === viewId) return
    gestures.current.clear()
    send({ type: 'reset', viewId })
  }, [viewId, send])

  // Cache version read when the reply arrives — it must include snapshots adopted since, not the value at release time.
  const cachedVersion = useRef(version)
  // Layout effect: reconciles before paint so a stale preview is not drawn for a frame over a new snapshot.
  useLayoutEffect(() => {
    cachedVersion.current = version
    const cur = latest.current
    // With no preview a snapshot does not change the state — no need to build the split id set either.
    if (cur.viewId !== viewId || cur.preview === null) return
    send({
      type: 'event',
      viewId,
      ev: { type: 'snapshot', version, splitIds: new Set(splitRects.map(s => s.split_id)) },
    })
  }, [viewId, version, splitRects, send])

  const pullToken = preview !== null && needsPull(preview, version) ? preview.token : null
  useEffect(() => {
    if (pullToken === null) return
    const timer = setTimeout(
      () => pullView(viewId, () => !mounted.current || latest.current.viewId !== viewId),
      COMMIT_PULL_DELAY_MS,
    )
    return () => clearTimeout(timer)
  }, [pullToken, viewId])

  const canvas = useSyncExternalStore(subscribeCanvasReport, readCanvas)
  const [measured, setMeasured] = useState<CanvasPx | null>(null)
  // Measured only before the canvas report. Re-measured on every render, but the state is not replaced if the size is the same.
  // Known limit: root size changes while this hook's host is not re-rendering go unseen — once a report arrives (debounce 100ms) that is used.
  useLayoutEffect(() => {
    if (canvas !== null) return
    const el = rootRef.current
    if (el === null) return
    const r = el.getBoundingClientRect()
    const next = { w: Math.round(r.width), h: Math.round(r.height) }
    setMeasured(prev => (prev !== null && prev.w === next.w && prev.h === next.h ? prev : next))
  })
  const axis = canvas ?? measured
  const axisW = axis?.w
  const axisH = axis?.h

  const display = useMemo(
    () => remapRects(slotRects, splitRects, preview),
    [slotRects, splitRects, preview],
  )

  const ranges = useMemo(() => {
    const m = new Map<string, Range>()
    for (const s of splitRects) {
      const leftRight = s.dir === 'left_right'
      const axisPx = leftRight ? axisW : axisH
      m.set(
        s.split_id,
        axisPx === undefined
          ? null
          : ratioRange({
              ratioMin,
              ratioMax,
              minPanePx: MIN_PANE_PX,
              axisPx,
              // Snapshot box before remap — must be the same input as the shell, bit for bit.
              boxExtent: leftRight ? s.x1 - s.x0 : s.y1 - s.y0,
            }),
      )
    }
    return m
  }, [splitRects, ratioMin, ratioMax, axisW, axisH])

  const splitterProps = useCallback(
    (split: SplitRect): SplitterProps => {
      const splitId = split.split_id
      // Attaches the view that drew this divider — what arrives after the view changed (an old commit's reply etc.) is dropped by `reduceScoped`.
      const toView = (ev: PreviewEvent): void => send({ type: 'event', viewId, ev })
      return {
        split,
        range: ranges.get(splitId) ?? null,
        rootRef,
        onDragStart(ratio) {
          nextToken.current += 1
          const token = nextToken.current
          gestures.current.set(splitId, token)
          toView({ type: 'dragStart', token, splitId, ratio })
        },
        onPreview(ratio) {
          const token = gestures.current.get(splitId)
          if (token !== undefined) toView({ type: 'dragMove', token, ratio })
        },
        onCancel() {
          const token = gestures.current.get(splitId)
          if (token === undefined) return
          gestures.current.delete(splitId)
          toView({ type: 'dragCancel', token })
        },
        onRelease(ratio) {
          const token = gestures.current.get(splitId)
          if (token === undefined) return
          gestures.current.delete(splitId)
          const cur = latest.current
          const owns =
            cur.viewId === viewId &&
            cur.preview !== null &&
            cur.preview.token === token &&
            cur.preview.phase === 'drag'
          toView({ type: 'release', token })
          if (!owns) return
          // The reply is not blocked by unmount state (see the `mounted` comment above). A dispatch after a real unmount is ignored by React.
          useViewStore
            .getState()
            .setSplitRatio(viewId, splitId, ratio)
            .then(
              res =>
                toView({
                  type: 'response',
                  token,
                  outcome: res.outcome,
                  version: res.version,
                  cachedVersion: cachedVersion.current,
                }),
              err => {
                console.error(`[useSplitDrag] set_split_ratio(${splitId}, ${ratio}) failed:`, err)
                toView({ type: 'failure', token })
              },
            )
        },
        onReset() {
          useViewStore
            .getState()
            .setSplitRatio(viewId, splitId, 0.5)
            .catch(err => console.error(`[useSplitDrag] set_split_ratio(${splitId}, 0.5) failed:`, err))
        },
      }
    },
    [ranges, rootRef, send, viewId],
  )

  return { slotRects: display.slotRects, splitRects: display.splitRects, splitterProps }
}

// Same shape as the tab cache pull in `WindowLayout.tsx` — bounded retries then adopt into the cache; on cancel (unmount, view switch) it stops retrying and drops an in-flight reply. What is adopted is filtered again by the cache's version guard.
function pullView(viewId: string, isCancelled: () => boolean): void {
  void retryAsync(() => invoke<ViewSnapshot>('get_view', { viewId }), {
    isCancelled,
    onRetry: (err, attempt) => console.warn(`[useSplitDrag] get_view(${viewId}) retry ${attempt}:`, err),
  })
    .then(snap => {
      if (!isCancelled()) useViewStore.getState().applyLayoutUpdated(snap)
    })
    .catch(err => {
      if (err instanceof RetryCancelledError) return
      console.error(`[useSplitDrag] get_view(${viewId}) final failure (retries exhausted):`, err)
    })
}
