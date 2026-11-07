// Number · phrasing computation for the usage slot (TRD S21 usage-limit-slot §1-8 · PRD R10·R21·R31·R32) — all clocks are taken as arguments.
//
// ★Do not mix the two clocks★: the relative fields of the snapshot (`age_secs`·`next_attempt_in_secs`·`retry_in_secs`) are based on the moment of receipt
//   (`performance.now()`), so only the time elapsed since then is added or subtracted — so they do not wobble even if the wall clock is rewound. The wall clock is
//   used only when comparing with the absolute time the server gave (`resets_at`) (R29).
// ★Sentence word order and joining marks all live in the `t()` templates (R28)★ — here we only decide which template to pick.

import type { UsageStateDetail } from '../../../crates/engram-dashboard-protocol/bindings/UsageStateDetail'
import type { UsageVendorState } from '../../../crates/engram-dashboard-protocol/bindings/UsageVendorState'
import type { UsageWindow } from '../../../crates/engram-dashboard-protocol/bindings/UsageWindow'
import { t } from '../../i18n'

/** If a value's age **exceeds** this many seconds it is shown as an old value (R32) — exactly 1800 seconds is still a new value. */
export const STALE_AFTER_SECS = 1800

/** Locale for printing times — the phrase table (`i18n/ko.ts`) is a single Korean sheet, so match the same locale. */
export const USAGE_LOCALE = 'ko-KR'

export type UsageLevel = 'ok' | 'warn' | 'danger' | 'none'

/**
 * How a window looks right now. `none` = no value held (it is not 0% — R21) — for a window carrying only a reset time, that time is
 * held.
 */
export type WindowReading =
  | {
      kind: 'none'
      resetsAt: number | null
      /** Seconds until reset — null if there is no `resetsAt`. Not expired, so always greater than 0. */
      resetInSecs: number | null
    }
  | { kind: 'expired' }
  | {
      kind: 'value'
      visible: number
      level: Exclude<UsageLevel, 'none'>
      /** Age of the value (seconds) as of now. */
      ageSecs: number
      stale: boolean
      resetsAt: number | null
      /** Seconds until reset — null if there is no `resetsAt`. Not expired, so always greater than 0. */
      resetInSecs: number | null
    }

/** Convert the used amount (the value Claude gives) to the remaining amount — so every company looks the same direction (it decreases as it is used) (R1). */
export function leftFromUsed(usedPct: number): number {
  return 100 - usedPct
}

/**
 * Visible number = the remaining amount clamped to 0–100 then floored (R10·D17) — do not inflate the remaining amount (19.9999… is 19). Color is
 * decided by this number too.
 */
export function visibleLeft(left: number): number {
  return Math.floor(Math.min(100, Math.max(0, left)))
}

/** `> 50` ok · `20–50` warn (both ends included) · `< 20` danger · no value = none (D17). */
export function usageLevel(visible: number | null): UsageLevel {
  if (visible === null) return 'none'
  if (visible > 50) return 'ok'
  if (visible >= 20) return 'warn'
  return 'danger'
}

/** Seconds elapsed since the moment of receipt — never goes negative. */
export function elapsedSecs(receivedAt: number, nowPerf: number): number {
  return Math.max(0, (nowPerf - receivedAt) / 1000)
}

/** Seconds remaining now of a relative seconds value as of the moment of receipt (`next_attempt_in_secs` etc.) — never goes below 0. */
export function remainingSecs(relativeSecs: number, elapsed: number): number {
  return Math.max(0, relativeSecs - elapsed)
}

export function isStale(ageSecs: number): boolean {
  return ageSecs > STALE_AFTER_SECS
}

/** Oldest age (seconds) among windows that have a value — null if there is no such window. Expired windows hold no value so they are not counted. */
export function oldestAgeSecs(readings: WindowReading[]): number | null {
  let oldest: number | null = null
  for (const r of readings) {
    if (r.kind === 'value' && (oldest === null || r.ageSecs > oldest)) oldest = r.ageSecs
  }
  return oldest
}

/** Daemon latch (`expired`) or the reset time has passed (R32). */
export function isExpired(win: UsageWindow, nowWallSecs: number): boolean {
  return win.expired || (win.resets_at !== null && win.resets_at <= nowWallSecs)
}

export function readWindow(win: UsageWindow | null, elapsed: number, nowWallSecs: number): WindowReading {
  if (win === null) return { kind: 'none', resetsAt: null, resetInSecs: null }
  if (isExpired(win, nowWallSecs)) return { kind: 'expired' }
  const resetInSecs = win.resets_at === null ? null : win.resets_at - nowWallSecs
  if (win.used_pct === null) return { kind: 'none', resetsAt: win.resets_at, resetInSecs }
  const visible = visibleLeft(leftFromUsed(win.used_pct))
  const ageSecs = win.age_secs + elapsed
  return {
    kind: 'value',
    visible,
    level: usageLevel(visible) as Exclude<UsageLevel, 'none'>,
    ageSecs,
    stale: isStale(ageSecs),
    resetsAt: win.resets_at,
    resetInSecs,
  }
}

/**
 * Time remaining in minutes — rounded up and never goes below 1 minute (once the reset passes, 「리셋됨」 (reset) stands in place of this phrase, and when the refusal ends
 * the state the daemon changed takes over).
 */
export function formatDuration(secs: number): string {
  const total = Math.max(1, Math.ceil(secs / 60))
  const days = Math.floor(total / 1440)
  const hours = Math.floor((total % 1440) / 60)
  const minutes = total % 60
  if (days > 0) {
    return hours > 0
      ? t('usage.durationDaysHours', { days: String(days), hours: String(hours) })
      : t('usage.durationDays', { days: String(days) })
  }
  if (hours > 0) {
    return minutes > 0
      ? t('usage.durationHoursMinutes', { hours: String(hours), minutes: String(minutes) })
      : t('usage.durationHours', { hours: String(hours) })
  }
  return t('usage.durationMinutes', { minutes: String(minutes) })
}

export function formatAge(ageSecs: number): string {
  if (ageSecs < 60) return t('usage.ageJustNow')
  const minutes = Math.floor(ageSecs / 60)
  if (minutes < 60) return t('usage.ageMinutes', { minutes: String(minutes) })
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return t('usage.ageHours', { hours: String(hours) })
  return t('usage.ageDays', { days: String(Math.floor(hours / 24)) })
}

const CLOCK = new Intl.DateTimeFormat(USAGE_LOCALE, { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' })

export function formatClock(epochSecs: number): string {
  return CLOCK.format(new Date(epochSecs * 1000))
}

// Dates are told apart by `Date`'s local getters — `CLOCK`, which prints the time, is in the same default time zone, so the two do not disagree.
function sameLocalDay(a: Date, b: Date): boolean {
  return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate()
}

// ADR-0258: a reset that is not today shows a date instead of a weekday.
/**
 * Reset time (user decision 2026-09-29) — on the same local date as now 「HH:MM」, otherwise 「M/D HH:MM」 (no weekday · no zero padding of
 * month · day). The small display and the popup use the same rule.
 */
export function formatResetAt(epochSecs: number, nowWallSecs: number): string {
  const at = new Date(epochSecs * 1000)
  const time = CLOCK.format(at)
  if (sameLocalDay(at, new Date(nowWallSecs * 1000))) return time
  return t('usage.resetDateTime', { month: String(at.getMonth() + 1), day: String(at.getDate()), time })
}

/** Name of the reset time (tooltip · assistive tech) — in the visible place an hourglass stands in place of the word (ADR-0259). */
export function formatResetClock(epochSecs: number, nowWallSecs: number): string {
  return t('usage.resetClock', { time: formatResetAt(epochSecs, nowWallSecs) })
}

/**
 * Append detail to the status phrase. `afterWait` = the phrase head already carries the wait time with a 「—」 (거절됨 — N분 뒤) — then
 * join the upstream raw text with a different template so the 「—」 does not double up.
 */
function withDetail(status: string, detail: UsageStateDetail | null, source: string, afterWait: boolean): string {
  if (detail === null) return status
  if (detail.upstream !== null) {
    const upstream = detail.upstream
    if (detail.code !== null) {
      const code = String(detail.code)
      return afterWait
        ? t('usage.withUpstreamCodeAfterWait', { status, source, code, upstream })
        : t('usage.withUpstreamCode', { status, source, code, upstream })
    }
    return afterWait
      ? t('usage.withUpstreamAfterWait', { status, source, upstream })
      : t('usage.withUpstream', { status, source, upstream })
  }
  return detail.code !== null
    ? t('usage.withKindCode', { status, kind: detail.kind, code: String(detail.code) })
    : t('usage.withKind', { status, kind: detail.kind })
}

/** Refusal wait — human-language time reduced by what elapsed since receipt. */
function rejectedHead(retryInSecs: number, elapsed: number): string {
  return t('usage.stateRejected', { duration: formatDuration(remainingSecs(retryInSecs, elapsed)) })
}

/**
 * Short name of an abnormal state (without detail · next attempt) — for a place that must speak briefly, like the accessibility name of the summary button. null for `Ready`.
 */
export function stateHead(state: UsageVendorState, elapsed: number): string | null {
  switch (state.kind) {
    case 'Ready':
      return null
    case 'NotInstalled':
      return t('usage.stateNotInstalled')
    case 'NeedsLogin':
      return t('usage.stateNeedsLogin')
    case 'Unavailable':
      return t('usage.stateUnavailable')
    case 'Failed':
      return t('usage.stateFailed')
    case 'Rejected':
      return rejectedHead(state.retry_in_secs, elapsed)
  }
}

/**
 * Sentence of an abnormal state (R31) — without the company name. Shape = status phrase + detail (with upstream raw text 「응답[ (code)]: <원문>」,
 * without it 「(kind[ · code])」) + number. The upstream raw text is inserted as is, untranslated. null for `Ready`.
 *
 * ★Source word★: the raw text of `NotInstalled` may be the OS's execution error, not the company's response, so it is called 「오류」 (error).
 * `elapsed` = seconds elapsed since the snapshot was received.
 */
export function statusSentence(state: UsageVendorState, elapsed: number, nowWallSecs: number): string | null {
  const response = t('usage.sourceResponse')
  switch (state.kind) {
    case 'Ready':
      return null
    case 'NotInstalled':
      return withDetail(t('usage.stateNotInstalled'), state.detail, t('usage.sourceError'), false)
    case 'NeedsLogin':
      return withDetail(t('usage.stateNeedsLogin'), state.detail, response, false)
    case 'Unavailable':
      return withDetail(t('usage.stateUnavailable'), state.detail, response, false)
    case 'Failed': {
      const at = nowWallSecs + remainingSecs(state.next_attempt_in_secs, elapsed)
      const sentence = withDetail(t('usage.stateFailed'), state.detail, response, false)
      return t('usage.withNextAttempt', { sentence, time: formatClock(at) })
    }
    case 'Rejected':
      return withDetail(rejectedHead(state.retry_in_secs, elapsed), state.detail, response, true)
  }
}

/**
 * One line with the company name in front — the badge's `aria-label`·`title` and the top line of the popup use this same string (R31). Within a sentence
 * the company name is not repeated (「Claude 조회 실패 — 응답: …」).
 */
export function statusLine(vendorName: string, sentence: string): string {
  return t('usage.statusLine', { vendor: vendorName, sentence })
}
