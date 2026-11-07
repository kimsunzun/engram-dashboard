// ADR-0242: scroll-follow decision — a pure function (no DOM) that takes one input and the current state and returns the next state and the position to write.
//   Canonical rule table = `docs/process/S21-chat-ux/trd.md` §2-2 (the numbers are that table's numbers — `followCore.test.ts` is that table).
//
// ★Pinned (`pinned`) changes only on scroll events and upward-going inputs — a size change (`resize`) does not re-measure it★:
//   re-measuring reads streaming growth as 「위로 올렸다」 (scrolled up) (alternative rejected in ADR-0242). Explicit `pin`·`unpin` are the exception.
// ★While pinned, `unseenGrowth` is always false★ — the paths that pin (rule 4 re-pin · `pin`) clear it too.

/** ADR-0242: the middle of the peer range 40–64 (t3code 40 · paseo 64). A distance from the bottom within this counts as 「바닥」 (bottom). */
export const FOLLOW_THRESHOLD_PX = 48

/** Width within which a scroll event is recognized as our own write (rule 3). */
const OWN_WRITE_TOLERANCE_PX = 1

/**
 * Within this width it already counts as at that position (rule 7) — we neither write nor announce. The browser emits no scroll
 * event for an in-place write, so leaving an announcement would mistake a later user scroll for ours.
 */
const SAME_POSITION_PX = 0.5

/** One measurement of the viewport (px) — `scrollTop` · `scrollHeight` · `clientHeight`. */
export interface Metrics {
  top: number
  height: number
  client: number
}

export interface FollowState {
  pinned: boolean
  /** The viewport is inside `display:none` (`client === 0` — ADR-0056 keep-alive tab). Values measured meanwhile are invalid. */
  frozen: boolean
  lastTop: number
  /** The position we wrote that no scroll event has yet shown us. `null` = no write pending. */
  expectTop: number | null
  /** Position recorded at freeze time — restored if it thaws while detached. `null` = nothing to restore (initial · after thaw). */
  savedTop: number | null
  /** The size changed while detached. Read only by the button condition `'whenUnseen'` (U3 alternative). */
  unseenGrowth: boolean
}

export type FollowInput =
  | { type: 'attach'; m: Metrics }
  | { type: 'scroll'; m: Metrics }
  | { type: 'resize'; m: Metrics }
  | { type: 'intentUp'; m: Metrics }
  | { type: 'pin' }
  | { type: 'unpin' }

/** If `scrollTo` is not `null`, the caller writes that value to `scrollTop`. */
export interface FollowStep {
  state: FollowState
  scrollTo: number | null
}

/** Rule 8. */
export function initialFollowState(): FollowState {
  return {
    pinned: true,
    frozen: false,
    lastTop: 0,
    expectTop: null,
    savedTop: null,
    unseenGrowth: false,
  }
}

const stay = (state: FollowState): FollowStep => ({ state, scrollTo: null })

const bottomOf = (m: Metrics): number => Math.max(0, m.height - m.client)

/**
 * Rule 7. ★`lastTop` moves to where the write will land★ — if the user drags up before the write's scroll event arrives,
 * that event arrives only once, merged with our write. With the old position as the baseline, the dragged spot is below it, so 「위로」 (up) is
 * not read and it stays pinned, and the next growth pulls it back down. The write's own event is recognized separately by `expectTop`.
 */
function writeTo(s: FollowState, m: Metrics, target: number): FollowStep {
  if (Math.abs(m.top - target) <= SAME_POSITION_PX) return stay(s)
  return { state: { ...s, lastTop: target, expectTop: target }, scrollTo: target }
}

/**
 * Is there anything to scroll — the 「맨 아래로」 (scroll to bottom) button condition (the hook supplies it). The pinned decision does not use it (rule 5 stays
 * `height > client` — the two are the same at integer px).
 */
export function isScrollable(m: Metrics): boolean {
  return m.height - m.client > SAME_POSITION_PX
}

/**
 * Rule 2. ★`expectTop` is cleared too★ — the scroll event of a position written before hiding was dropped while frozen, and
 * the position was re-measured here, so a leftover announcement would only make a later user scroll look like ours.
 */
function thaw(s: FollowState, m: Metrics): FollowStep {
  const base: FollowState = { ...s, frozen: false, lastTop: m.top, expectTop: null, savedTop: null }
  if (s.pinned) return writeTo(base, m, bottomOf(m))
  if (s.savedTop !== null && m.top !== s.savedTop) return writeTo(base, m, s.savedTop)
  return stay(base)
}

/** Rules 3 · 4. */
function onScroll(s: FollowState, m: Metrics): FollowStep {
  if (s.expectTop !== null && Math.abs(m.top - s.expectTop) <= OWN_WRITE_TOLERANCE_PX) {
    return stay({ ...s, lastTop: m.top, expectTop: null })
  }
  // ★A scroll that does not match also discards the announcement★: the browser merges one frame's position changes into one event —
  //   if the user moved after our write, that write's event does not arrive separately.
  const next: FollowState = { ...s, lastTop: m.top, expectTop: null }
  const distance = m.height - m.top - m.client
  if (s.pinned) {
    // An upward move within the threshold may be a clamp caused by shrunk content — do not unpin.
    if (distance > FOLLOW_THRESHOLD_PX && m.top < s.lastTop) next.pinned = false
  } else if (m.top > s.lastTop && distance <= FOLLOW_THRESHOLD_PX) {
    // ★Upward-intent latch★: re-pinning happens only on a scroll that moved down into the threshold — so after a small up-wheel unpins by rule 5,
    //   the up scroll (within the threshold) that wheel produced does not re-pin right away.
    next.pinned = true
    next.unseenGrowth = false
  }
  return stay(next)
}

export function step(s: FollowState, i: FollowInput): FollowStep {
  // Rule 10 — even while frozen only pinned changes (rule 1). The write comes from the measurement the hook feeds next or from the thaw (rule 2).
  if (i.type === 'pin') return stay({ ...s, pinned: true, unseenGrowth: false })
  if (i.type === 'unpin') return stay({ ...s, pinned: false })

  const { m } = i
  if (s.frozen) {
    if (m.client > 0 && (i.type === 'resize' || i.type === 'attach')) return thaw(s, m)
    return stay(s)
  }
  if (m.client === 0) {
    // Rule 1 — `m.top` at this moment may already be invalid, so record the last value seen as valid.
    return stay({ ...s, frozen: true, savedTop: s.expectTop ?? s.lastTop })
  }

  switch (i.type) {
    case 'attach': {
      // Rule 9 — a new node. Events of positions written to the old node do not arrive here.
      const base: FollowState = { ...s, lastTop: m.top, expectTop: null }
      return s.pinned ? writeTo(base, m, bottomOf(m)) : stay(base)
    }
    case 'scroll':
      return onScroll(s, m)
    case 'resize':
      if (s.pinned) return writeTo(s, m, bottomOf(m))
      return stay(s.unseenGrowth ? s : { ...s, unseenGrowth: true })
    case 'intentUp':
      return stay(s.pinned && m.height > m.client ? { ...s, pinned: false } : s)
  }
}
