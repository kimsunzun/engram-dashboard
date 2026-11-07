// ADR-0227: pure computation of divider drag — preview state and reconcile rules, allowed ratio range, linear remapping of shell rectangles.
//   It knows neither React nor invoke. Token issuing · the `set_split_ratio` call · the pull timer are borne by the hook on the renderer side.
// ★The frontend does not hold a second tree geometry★ — the preview only moves the rectangles the shell gave and does not unfold the tree again.
//   Last-digit differences are covered by the confirmed snapshot (TRD §2f).

import type { SlotRect, SplitRatioOutcome, SplitRect } from '../../api/layoutTypes'

/** Minimum area size (CSS px). It is a policy constant of the screen and the only source of truth — the shell receives it through `report_ui_metrics` and uses it. */
export const MIN_PANE_PX = 30

/** Time to wait for that version's snapshot after an `Applied` response. If exceeded, `get_view` is pulled once (against event loss). */
export const COMMIT_PULL_DELAY_MS = 500

export type PreviewPhase = 'drag' | 'commit'

/**
 * One preview per view. `ratio` = the share of the a-side (left/top) area (ADR-0140).
 * `awaitVersion` exists only after `commit` has received an `Applied` response — it is the version of the snapshot in which that write was announced.
 */
export interface SplitPreview {
  token: number
  splitId: string
  ratio: number
  phase: PreviewPhase
  awaitVersion?: number
}

export type PreviewState = SplitPreview | null

/**
 * `token` = a value the caller draws anew for every drag. Responses · failures carry that drag's token.
 * `response.cachedVersion` = the version of this view's cache at the moment the response was received.
 * `snapshot` = every time this view's cache adopts a new snapshot — `splitIds` is all the split ids of that snapshot.
 */
export type PreviewEvent =
  | { type: 'dragStart'; token: number; splitId: string; ratio: number }
  | { type: 'dragMove'; token: number; ratio: number }
  | { type: 'dragCancel'; token: number }
  | { type: 'release'; token: number }
  | { type: 'response'; token: number; outcome: SplitRatioOutcome; version: number; cachedVersion: number }
  | { type: 'failure'; token: number }
  | { type: 'snapshot'; version: number; splitIds: ReadonlySet<string> }

/**
 * Reconcile rules (TRD §2f 1~5). null = no preview (screen = cache = shell value).
 * - `dragStart` always replaces with a new drag. The other drag events are ignored if their token differs from the current one (rule 5).
 * - `snapshot` during `drag`: null if the split id being dragged has disappeared (drag cancelled), otherwise kept (rule 1).
 * - `release` → `commit`. `dragMove` · `dragCancel` are taken only in `drag`, `response` · `failure` only in `commit`.
 * - `response`: `Unchanged` · `TooSmall` → null immediately. `Applied` → `awaitVersion = version`, null if the cache is already at or above it (rule 2).
 * - `snapshot` during `commit`: null if `version >= awaitVersion`, kept if before the response or below it (rule 2 · 3).
 * - `failure` → null (rule 4 — the caller leaves the log).
 */
export function reducePreview(state: PreviewState, ev: PreviewEvent): PreviewState {
  if (ev.type === 'dragStart') {
    return { token: ev.token, splitId: ev.splitId, ratio: ev.ratio, phase: 'drag' }
  }
  if (state === null) return null
  if (ev.type === 'snapshot') {
    if (state.phase === 'drag') return ev.splitIds.has(state.splitId) ? state : null
    return state.awaitVersion !== undefined && ev.version >= state.awaitVersion ? null : state
  }
  if (ev.token !== state.token) return state
  switch (ev.type) {
    case 'dragMove':
      return state.phase === 'drag' ? { ...state, ratio: ev.ratio } : state
    case 'dragCancel':
      return state.phase === 'drag' ? null : state
    case 'release':
      return state.phase === 'drag' ? { ...state, phase: 'commit' } : state
    case 'response':
      if (state.phase !== 'commit') return state
      if (ev.outcome !== 'Applied') return null
      if (ev.cachedVersion >= ev.version) return null
      return { ...state, awaitVersion: ev.version }
    case 'failure':
      return state.phase === 'commit' ? null : state
  }
}

/**
 * Whether this is a state in which to set a pull timer after commit — true only when it is `commit` and the awaited version is above the cache.
 * Before the response (no `awaitVersion`) and for an unchanged response it does not wait for a snapshot, so it is false.
 */
export function needsPull(state: PreviewState, cachedVersion: number): boolean {
  return (
    state !== null &&
    state.phase === 'commit' &&
    state.awaitVersion !== undefined &&
    state.awaitVersion > cachedVersion
  )
}

/**
 * Allowed ratio range of the split being dragged. null = locked (neither side can be given `minPanePx` — `L < 2·minPanePx`).
 * `axisPx` = that axis length of the integer canvas this window reported to the shell, `boxExtent` = that axis width of the snapshot split box (`x1 - x0` or `y1 - y0`).
 * ★The same formula · same operation order as the px clamp of shell `ViewManager::set_split_ratio` (`src-tauri/src/layout/manager.rs`) ④★
 * — with the same input the range is bit-for-bit equal, so the confirmed value is not clipped again in the shell (no jump on release).
 * To change the formula, change it together with the shell.
 */
export function ratioRange(p: {
  ratioMin: number
  ratioMax: number
  minPanePx: number
  axisPx: number
  boxExtent: number
}): { lo: number; hi: number } | null {
  const edge = p.minPanePx / (p.axisPx * p.boxExtent)
  const lo = Math.max(p.ratioMin, edge)
  const hi = Math.min(p.ratioMax, 1 - edge)
  // The shell tells an empty range by `lo > hi`. Written in negated form to send NaN to the lock too.
  if (!(lo <= hi)) return null
  return { lo, hi }
}

/**
 * Convert the pointer position (normalized relative to the root, split axis) into a ratio inside the split box and clamp it into `range`.
 * If the computation is NaN (box width 0, etc.) return `range.lo` — so NaN does not leak into the preview · command.
 */
export function ratioFromPointer(p: {
  pointerNorm: number
  box0: number
  box1: number
  range: { lo: number; hi: number }
}): number {
  const r = (p.pointerNorm - p.box0) / (p.box1 - p.box0)
  if (Number.isNaN(r)) return p.range.lo
  return Math.min(p.range.hi, Math.max(p.range.lo, r))
}

type Axis = 'x' | 'y'

/**
 * Apply the preview to the shell rectangles — inside the box of the split being dragged (the areas · splits of that subtree) only that axis coordinate is linearly remapped.
 * Rectangles outside the box are returned as the same object — since the reference is the same, the caller can skip re-rendering that area.
 * The input array is not changed.
 * Remapping is a function of coordinate values, so boundaries that were bit-for-bit equal in the input are equal in the result. Box edges stay in place
 * (except for degenerate input where a boundary sticks to the box end — the shell's write path does not create such splits).
 * If `preview` is null, or that split id is absent, or the ratio is not finite, the input is returned as is.
 */
export function remapRects(
  slotRects: SlotRect[],
  splitRects: SplitRect[],
  preview: SplitPreview | null,
): { slotRects: SlotRect[]; splitRects: SplitRect[] } {
  if (preview === null || !Number.isFinite(preview.ratio)) return { slotRects, splitRects }
  const target = splitRects.find(s => s.split_id === preview.splitId)
  if (target === undefined) return { slotRects, splitRects }

  const axis: Axis = target.dir === 'left_right' ? 'x' : 'y'
  const X0 = axis === 'x' ? target.x0 : target.y0
  const X1 = axis === 'x' ? target.x1 : target.y1
  const B = target.at
  // The same shape as the shell geometry boundary formula (`x0 + (x1 - x0) * ratio`). Going outside the box would cover neighbors, so it is clamped.
  const B2 = Math.min(X1, Math.max(X0, X0 + (X1 - X0) * preview.ratio))
  if (B2 === B) return { slotRects, splitRects }

  const map = (v: number): number => {
    if (v === B) return B2
    // The shell keeps split boundaries strictly inside the box (`SplitTooDeep`), so the two denominators are not 0 — if one comes in, nothing moves.
    if (v < B) return B === X0 ? v : X0 + ((v - X0) * (B2 - X0)) / (B - X0)
    return B === X1 ? v : X1 - ((X1 - v) * (X1 - B2)) / (X1 - B)
  }
  const inBox = (r: { x0: number; y0: number; x1: number; y1: number }): boolean =>
    r.x0 >= target.x0 && r.x1 <= target.x1 && r.y0 >= target.y0 && r.y1 <= target.y1

  const nextSlots = slotRects.map(r => {
    if (!inBox(r)) return r
    return axis === 'x' ? { ...r, x0: map(r.x0), x1: map(r.x1) } : { ...r, y0: map(r.y0), y1: map(r.y1) }
  })
  const nextSplits = splitRects.map(s => {
    if (!inBox(s)) return s
    const moved = axis === 'x' ? { ...s, x0: map(s.x0), x1: map(s.x1) } : { ...s, y0: map(s.y0), y1: map(s.y1) }
    // `at` is the axis coordinate of that split itself — only the boundaries of splits divided along the same axis move.
    if (s.dir === target.dir) moved.at = map(s.at)
    return moved
  })
  return { slotRects: nextSlots, splitRects: nextSplits }
}
