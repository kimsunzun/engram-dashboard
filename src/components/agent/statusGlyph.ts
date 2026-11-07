// ★Split into a shared module★: the pure mapping functions that originally lived in AgentList.tsx live here. The RichSlot header draws the same
//   status glyph, and to avoid a transitive dependency on the heavy AgentList module (react-arborist · all the tree logic),
//   only the pure helpers go in this light module. AgentList re-exports these values so existing importers/tests stay unchanged.
//
// Agent status → icon · color mapping. What ADR-0062 §38 requires is **headless fixing of every branch**, and this file keeps
//   only that — the mapping can be tested alone without AgentList, which drags in react-arborist.
//   ★It is not "zero external dependencies"★: it returns lucide icon components as values, so that import is unavoidable (a change that came
//   when ADR-0168 replaced character glyphs with icons). JSX is not put here (the drawing side = AgentList).
//
// ★A row has only one symbol (ADR-0173)★: even when axes grow (status · last failure), symbols are not placed side by side but
//   swapped. That decision is in the one `isFailureBlocked`, and the symbol · color · hover all follow it.
//   ★Iconization (ADR-0168) does not change this axis★: only the *shape* of the return changed — a set of
//   {icon · fill · shape token} came in place of the old character `⊘`, and the decision formula · color · hover wording were not touched. The observation surface
//   also only moved from textContent to the `shape` token and did not disappear (an svg leaves no text).

import { Ban, Circle, Loader, Square, X, type LucideIcon } from 'lucide-react'

import type { AgentFailureKind } from '../../api/types'

/** A set the renderer passes to the svg as is. */
export type StatusGlyphIcon = {
  Icon: LucideIcon
  /** lucide's default is `none` (outline) — whether it is filled is the *shape* difference that tells Running (filled circle) from Reserved (empty circle). */
  fill: 'currentColor' | 'none'
  /**
   * A stable token that goes out to the DOM observation surface (tests · CDP · LLM selectors). Icon components are not serialized.
   *
   * ★Do not delete or rename an existing token★: selectors hang on this string, so a rename is silent destruction.
   *   **Only growing is safe** — `blocked` grew that way (the last failure axis of ADR-0173),
   *   and `pending` is next (the presentation layer below — while waiting for an answer).
   */
  shape: 'filled-circle' | 'square' | 'x' | 'circle' | 'blocked' | 'pending'
}

/**
 * The prohibition mark the last failure uses when it swaps the status symbol (ADR-0173 — we do not add one more symbol).
 *
 * ★Why `Ban` was chosen = it is the *same figure* as the old character `⊘`★: of the three circle+slash candidates in lucide, the one that fills
 *   the circle's diameter fully and stops exactly at the border is `Ban` alone (4.929,4.929 → 19.071,19.071). In `CircleSlash` the
 *   slash is a 6-unit segment in the middle (9,15 → 15,9), so at 12px it looks like a dot, not a slash, and `CircleSlash2` is
 *   a line joining the viewBox corners (22,2 → 2,22), so it sticks out of the circle. Only the direction (↘ vs ↗) differs and the reading of "a circle struck out"
 *   is the same, so `Ban`, the standard form of the prohibition symbol, fits on both meaning (cannot open) and shape.
 * ★Fill is `none`★: filled, the slash inside the circle is buried and the silhouette overlaps Running (filled circle) — when color is
 *   neutralized in e-ink there is nothing left to tell the two apart (ADR-0062 「모양이 1차 신호」 — shape is the primary signal).
 */
const BLOCKED_GLYPH: StatusGlyphIcon = { Icon: Ban, fill: 'none', shape: 'blocked' }

/**
 * Does the last failure take this row's symbol now — **a single decision that the symbol · color · hover all follow**.
 *
 * ★Running wins (ADR-0173)★: even if it holds a last failure, if it is up we draw the original status symbol. A failure display is
 *   the signal "this entry cannot be opened right now", and putting it on a running entry would be a lie.
 * ★`Exiting` is also on the 「도는 중」 (running) side★: it is a **transitional** transition the manager emits, not a state that stays (ADR-0005).
 *   Counting it as a resting state makes the row flicker with the prohibition mark for the few seconds of a normal exit.
 * ★Why swapping loses nothing★: the display appears only for entries that are not up, and then the original symbol (square/empty circle)
 *   has no information to carry.
 */
export function isFailureBlocked(
  status: string,
  lastFailure?: AgentFailureKind | null,
): boolean {
  return Boolean(lastFailure) && status !== 'Running' && status !== 'Exiting'
}

/**
 * **Shape**, not color, carries the status, so it is distinguishable in e-ink as well.
 *
 * When `lastFailure` takes the symbol (`isFailureBlocked` above) it **swaps in** the prohibition mark regardless of status —
 * no second symbol is placed beside it (ADR-0173). Omitting the argument gives the old behavior where only the status axis runs —
 * ★Why the second argument is optional★: a caller with no notion of last failure (a spot that knows only status, like the RichSlot header)
 * must still compile as is. Making it required would make callers unrelated to this axis carry `null` too.
 *
 * Mapping (ADR-0062):
 *   - Running                → filled circle (working)
 *   - Exiting/Exited/Killed  → square    (stopped — Exiting is the transition just before terminal)
 *   - Failed                 → ✗ shape  (error)
 *   - Reserved (frontend composition)   → empty circle   (idle/unspawned empty shell)
 *   - Anything else (unknown status)      → empty circle   (safe degrade — prevents an empty field)
 *
 * ★Do not make branches whose shapes overlap★: two states that differ only in color and share a shape lose their distinction in e-ink
 *   (the e-ink block neutralizes `--status-running` to `var(--text-muted)` — the `statusGlyphColor` comment).
 *
 * ★Never return the half-filled shape that would correspond to ◐ (awaiting input)★: the backend emits no "awaiting input" signal,
 *   so that branch does not exist (ADR-0062 — being unlit is intent, not a defect). Adding a branch here when the backend
 *   emits the signal is the canonical path.
 *
 * ★Do not revert to text glyphs (`● ◻ ✗ ○ ⊘`)★: after the UI font changed from monospace to Segoe UI, each character's
 *   size · weight · baseline differed and vertical alignment with the row label went off. An icon is a 12px square regardless of the font.
 */
export function statusGlyphIcon(
  status: string,
  lastFailure?: AgentFailureKind | null,
): StatusGlyphIcon {
  // ADR-0173: look at it before the status branches — it is a swapping axis so it overrides the status mapping.
  if (isFailureBlocked(status, lastFailure)) return BLOCKED_GLYPH
  switch (status) {
    case 'Running':
      return { Icon: Circle, fill: 'currentColor', shape: 'filled-circle' }
    case 'Exiting':
    case 'Exited':
    case 'Killed':
      return { Icon: Square, fill: 'none', shape: 'square' }
    case 'Failed':
      return { Icon: X, fill: 'none', shape: 'x' }
    case 'Reserved':
      return { Icon: Circle, fill: 'none', shape: 'circle' }
    default:
      return { Icon: Circle, fill: 'none', shape: 'circle' }
  }
}

/**
 * ADR-0062 amendment: originally "status = shape (not color)", but
 * when e-ink was split off as a separate mode, color is allowed in other themes. Color does not replace shape but is *added* to it.
 * ★No color literals · variables only★ (ADR-0062 §44):
 * the green value is owned by --status-running in theme.css, and the e-ink block neutralizes it to var(--text-muted) so
 * only shape remains in e-ink (shape is still the primary signal). The warning color (--status-blocked) follows the same rule.
 * ★Calls the **same decision** as `statusGlyphIcon`★ (ADR-0173): a row whose symbol is swapped must have its color swapped too —
 * duplicating the decision makes two copies diverge and yields a combination where the prohibition mark shows in green.
 */
export function statusGlyphColor(status: string, lastFailure?: AgentFailureKind | null): string {
  // ADR-0173
  if (isFailureBlocked(status, lastFailure)) return 'var(--status-blocked)'
  return status === 'Running' ? 'var(--status-running)' : 'var(--text-muted)'
}

// ─────────────────────────────────────────────────────────────────────────────
// Presentation layer — 「답을 기다리는 중」 (waiting for an answer) · 「방금 거절당했다」 (just rejected)
//
// ★Do not grow the backend state★: `status` becomes `Running` optimistically the moment a session is created (the core's
//   OutputCore initial value) and has never meant "connected · usable". So a double-clicked row drew a green filled circle in the span before
//   the answer — saying "running" when nothing was known yet, and even after activation was
//   rejected that green stayed a few seconds more until the process was observed dead (measured 2026-08-24: rejection
//   +2.4s → green 4.2s → only then the prohibition mark).
//
// ★The direction of the fix = use the answer the frontend already holds★: the in-flight/rejection of the activation RPC is known to this screen
//   first (the moment a rejection raises the 「실패」 (failed) badge). So without touching the status axis we lay
//   a layer on top — the meaning of `isFailureBlocked` does not change by a single character.
//
// ★The property that the three (symbol · color · hover) follow one decision is kept★ (ADR-0173): `rowPhase` below is the place of that
//   one decision, and the symbol (`rowGlyphIcon`) · color (`rowGlyphColor`) · hover (the title in AgentList) receive the same value.
// ─────────────────────────────────────────────────────────────────────────────

/**
 * What this row is saying right now — **the decision order is the priority**.
 *
 * - `pending`  : a request made on this row is still running. The answer is unknown, so no status symbol is drawn.
 * - `rejected` : the activation just made was **rejected**. Before the process is observed dead `status` may still be
 *                `Running` and the 「마지막 실패」 (last failure) record also arrives a beat later — we wait for neither.
 * - `settled`  : anything else — the status · last failure axes (`statusGlyphIcon` above) as is.
 */
export type RowPhase = 'settled' | 'pending' | 'rejected'

/**
 * ★`pending` beats `rejected`★: the rejection mark is the result of a *past* attempt, so if a new attempt is in flight
 *   on the same row, that new attempt is the current fact (on retry it must switch to the spinner at once).
 */
export function rowPhase(inFlight: boolean, activationRejected: boolean): RowPhase {
  if (inFlight) return 'pending'
  if (activationRejected) return 'rejected'
  return 'settled'
}

/**
 * The symbol while waiting for an answer.
 *
 * ★Why `Loader` (8 spokes) was chosen = e-ink★: that theme neutralizes `--status-*` wholesale
 *   (`--status-running` → `var(--text-muted)`) so **only shape remains.** Of the candidates, `LoaderCircle` is a single circle open
 *   at one spot (`M21 12a9 9 0 1 1-6.219-8.56`), so at 12px its silhouette is effectively the same as Reserved's empty circle
 *   — where neither color nor motion can be trusted, that is not a signal. `Loader` is a star shape of 4 axes + 4 diagonals = eight
 *   branches extending with the center empty, so it overlaps none of the existing four (filled circle · empty circle · square · ✗) or the prohibition mark.
 *   So it reads **even when rotation stops** (reduced-motion).
 * ★Fill is `none`★: spokes are lines so there is no area to fill (filling makes lucide smear the strokes).
 * ★Rotation is owned by CSS★: the keyframes are in `agentGlyph.css` (class `engram-glyph-spin`) — no JSX inline
 *   keyframes. This module still makes neither JSX nor styles (ADR-0168).
 */
const PENDING_GLYPH: StatusGlyphIcon = { Icon: Loader, fill: 'none', shape: 'pending' }

/**
 * The symbol a row actually draws — the final choice with the presentation layer on top.
 *
 * `settled` **delegates as is** to `statusGlyphIcon` (no duplication — if two copies diverge the same state
 * shows a different shape in each place).
 */
export function rowGlyphIcon(
  phase: RowPhase,
  status: string,
  lastFailure?: AgentFailureKind | null,
): StatusGlyphIcon {
  if (phase === 'pending') return PENDING_GLYPH
  // ★Do not wait for the record★: the rejection is an answer this screen received directly, and the 「마지막 실패」 (last failure) is merely a copy of that fact
  //   coming back around through the backend. When the record arrives and it settles down to `settled`, the same prohibition mark comes out.
  if (phase === 'rejected') return BLOCKED_GLYPH
  return statusGlyphIcon(status, lastFailure)
}

/**
 * The color a row actually uses — it receives **the same phase** as the symbol (if they diverge, the prohibition mark shows in green).
 *
 * ★`pending` is muted★: green is a claim that "it is running", so it cannot be used in the span where the answer is unknown — the
 *   core of this defect was exactly that green.
 */
export function rowGlyphColor(
  phase: RowPhase,
  status: string,
  lastFailure?: AgentFailureKind | null,
): string {
  if (phase === 'pending') return 'var(--text-muted)'
  if (phase === 'rejected') return 'var(--status-blocked)'
  return statusGlyphColor(status, lastFailure)
}
