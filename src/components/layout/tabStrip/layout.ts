// Tab bar head width · hiding = exactly the Chromium rules (Q22, TRD S21-slot-docking R24).
// Source = chromium/src main: width `chrome/browser/ui/views/tabs/tab_strip_layout.cc` (`CalculateSpaceFractionAvailable` · `TabSizer::CalculateTabWidth` · `AllocateExtraSpace` · `TabSizer::TabAcceptsExtraSpace`, read 2026-10-17); minimum width `tab_width_constraints.cc` `GetMinimumWidth`; hiding `chrome/browser/ui/views/tabs/tab_container_impl.cc` `ShouldTabBeVisible` (fetched 2026-10-18 #1677-1733).
// ★Our share is only the scale of the values★ — no pinned tabs · no `tab_overlap`, so overlap is read as 0. Do not invent rules (name-length width · pulling the active tab into view · overflow menu were removed by Q22). Numbers 1–5 in the body = stages of the computation.

/** Width of one head when there is room (`S`, CSS px, `border-box` — padding · border included): our scale of Chrome `TabStyle::GetStandardWidth()` (256 dp). */
export const TAB_STANDARD_W = 160

/** Circle diameter of the close (×) · new tab (+) buttons (CSS px — ADR-0298 decision 5). The active minimum width holds the × circle, so it lives here (`TabBar.tsx` draws with it). */
export const TAB_BUTTON_SIZE = 22

/** Gap before the `+` circle (CSS px) so it does not touch the separator (±2.5) straddling the last head's right edge. `+` box width = this gap + `TAB_BUTTON_SIZE` (`TabBar.tsx` `ADD_STYLE` · minimum window width `titleRow/metrics.ts`). */
export const TAB_ADD_GAP = 4

/** Left and right padding of the head content area (CSS px — `HEAD_CONTENT` of `TabBar.tsx`). */
export const TAB_HEAD_PAD = 12

/** Gap between the name and × in the head content area (CSS px — `HEAD_CONTENT` of `TabBar.tsx`). */
export const TAB_HEAD_GAP = 6

/** Minimum width of an active head (`A`) = the width in which the close button fits (Chrome `GetMinimumActiveWidth()`): left padding + name 0 + gap + close circle + right padding (ADR-0298 decision 5; the head has no border). */
export const TAB_MIN_ACTIVE_W = TAB_HEAD_PAD + 0 + TAB_HEAD_GAP + TAB_BUTTON_SIZE + TAB_HEAD_PAD

/** Minimum width of an inactive head (`I`) = Chrome `GetMinimumInactiveWidth()` ≈ 32 dp: only the name field left after removing left/right padding (`TAB_HEAD_PAD` × 2); the close button does not fit (a head narrower than `A` hides it). */
export const TAB_MIN_INACTIVE_W = 32

export interface TabStripLayout {
  /** Computed width per head (CSS px · integer). A hidden head also carries a computed width — the visibility decision measures the right end with this width (0 when drawn). */
  widths: number[]
  /** Whether each head is visible. Hidden ones are always the tail — visible = the first `k`. */
  visible: boolean[]
}

/**
 * Width · visibility of `n` heads. `available` = measured "CSS px — tab bar row width − `+` width, floored to an integer"; `null` / non-finite = unmeasurable (before measuring · no `ResizeObserver`) = no limit.
 * `widthCap` = a cap on the width computation only — the fixed width of closing mode (`closingMode.ts` `layoutAvailable`). ★Visibility is measured by `available`, not the cap★: Chrome computes width as `min(frozen, measured)` (tab_container_impl.cc #1189 · #1220-1227) and visibility by the measured width (#1688-1691 · #1732). With neither, all are `S` · nothing hidden.
 * `activeIndex` = position of the head the tab bar draws as active (among input heads — the caller removes collapsed ones). Out of range (including `-1`) = no active (all use inactive minimum `I`; the active condition of hiding does not apply).
 */
export function tabStripLayout(
  available: number | null,
  n: number,
  activeIndex: number,
  widthCap?: number | null,
): TabStripLayout {
  const count = Math.max(0, Math.floor(n))
  const room = floorOrNull(available)
  const cap = floorOrNull(widthCap)
  const avail = room === null ? cap : cap === null ? room : Math.min(room, cap)
  if (avail === null) {
    return { widths: Array<number>(count).fill(TAB_STANDARD_W), visible: Array<boolean>(count).fill(true) }
  }
  const active = activeIndex >= 0 && activeIndex < count ? activeIndex : -1
  const S = TAB_STANDARD_W
  const A = TAB_MIN_ACTIVE_W
  const I = TAB_MIN_INACTIVE_W

  // 1. Minimum · overflow point (all heads `A`) · preferred (all heads `S`).
  const mins = Array.from({ length: count }, (_, i) => (i === active ? A : I))
  const minimum = mins.reduce((s, m) => s + m, 0)
  const crossover = count * A
  const preferred = count * S

  let widths: number[]
  const phaseB = avail < crossover
  let f: number
  if (!phaseB) {
    // 2. Stage A — all the same width, between `A` and `S`.
    f = preferred === crossover ? 1 : clamp01((avail - crossover) / (preferred - crossover))
    widths = Array<number>(count).fill(Math.floor(A + f * (S - A)))
  } else {
    // 3. Stage B — the active stays at `A` · only inactive ones go from `A` toward `I`.
    f = crossover === minimum ? 0 : clamp01((avail - minimum) / (crossover - minimum))
    widths = mins.map(m => Math.floor(m + f * (A - m)))
  }

  // 4. Leftover px (`AllocateExtraSpace`) — 1 px each from the left to heads that accept it; a share beyond the accepting count is unused (overflowed → `extra ≤ 0`, none). Accepting (`TabAcceptsExtraSpace`): ratio 0 · 1 → nobody; stage B = heads with minimum < overflow point (inactive — active's minimum equals `A`); stage A = heads with overflow point < preferred (all).
  const accepts = (i: number) => f !== 0 && f !== 1 && (phaseB ? mins[i] < A : A < S)
  let extra = avail - widths.reduce((s, w) => s + w, 0)
  for (let i = 0; i < count && extra > 0; i += 1) {
    if (!accepts(i)) continue
    widths[i] += 1
    extra -= 1
  }

  // 5. Visibility (by the measured width `room`) — hidden if the right end overflows; if the active is after this head, also hidden if the right end of "the active at this index or before" (= left end + active width) overflows, so the visible set does not wobble when the active changes (Chrome's stable set).
  const visible: boolean[] = []
  let x1 = 0
  for (let i = 0; i < count; i += 1) {
    x1 += widths[i]
    if (room === null) visible.push(true)
    else if (x1 > room) visible.push(false)
    else if (active > i) visible.push(x1 + (widths[active] - widths[i]) <= room)
    else visible.push(true)
  }
  return { widths, visible }
}

function floorOrNull(v: number | null | undefined): number | null {
  return v === null || v === undefined || !Number.isFinite(v) ? null : Math.floor(v)
}

function clamp01(f: number): number {
  return Math.min(1, Math.max(0, f))
}
