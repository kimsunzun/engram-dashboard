// usageFormat — numbers and phrasing of the usage slot (the pure-computation share of the 「frontend display」 rows of TRD S21 usage-limit-slot §4).

import { afterEach, describe, expect, it, vi } from 'vitest'

import type { UsageWindow } from '../../../crates/engram-dashboard-protocol/bindings/UsageWindow'
import {
  STALE_AFTER_SECS,
  elapsedSecs,
  formatAge,
  formatClock,
  formatDuration,
  formatResetAt,
  formatResetClock,
  isExpired,
  isStale,
  leftFromUsed,
  oldestAgeSecs,
  readWindow,
  stateHead,
  statusLine,
  statusSentence,
  usageLevel,
  visibleLeft,
} from './usageFormat'

const NOW = 1_900_000_000

function win(over: Partial<UsageWindow> = {}): UsageWindow {
  return { used_pct: 38, resets_at: NOW + 3600, age_secs: 0, expired: false, ...over }
}

// ── Visible number · color boundaries (R10·D17 — based on the remaining amount) ──
describe('visible number and color band', () => {
  const cases: Array<[left: number, visible: number, level: string]> = [
    [19.6, 19, 'danger'],
    [20.4, 20, 'warn'],
    [50.9, 50, 'warn'],
    [51, 51, 'ok'],
    [50, 50, 'warn'],
    [20, 20, 'warn'],
    [0, 0, 'danger'],
    [100, 100, 'ok'],
    [-5, 0, 'danger'],
    [130, 100, 'ok'],
  ]
  for (const [left, visible, level] of cases) {
    it(`remaining ${left} → ${visible} · ${level}`, () => {
      expect(visibleLeft(left)).toBe(visible)
      expect(usageLevel(visibleLeft(left))).toBe(level)
    })
  }

  it('no value → none (not 0% — R21)', () => {
    expect(usageLevel(null)).toBe('none')
    expect(readWindow(win({ used_pct: null, resets_at: null }), 0, NOW)).toEqual({
      kind: 'none',
      resetsAt: null,
      resetInSecs: null,
    })
    expect(readWindow(null, 0, NOW)).toEqual({ kind: 'none', resetsAt: null, resetInSecs: null })
  })

  it('a window carrying only the reset time → none but it holds the reset time · remaining seconds', () => {
    expect(readWindow(win({ used_pct: null, resets_at: NOW + 600 }), 0, NOW)).toEqual({
      kind: 'none',
      resetsAt: NOW + 600,
      resetInSecs: 600,
    })
    // If the reset has passed, it is expired even without a value.
    expect(readWindow(win({ used_pct: null, resets_at: NOW }), 0, NOW)).toEqual({ kind: 'expired' })
  })

  it('used amount → remaining amount (R1) — rounding down as is without correction (does not inflate the remaining amount)', () => {
    expect(leftFromUsed(80.4)).toBeCloseTo(19.6)
    expect(visibleLeft(leftFromUsed(80.4))).toBe(19)
    expect(visibleLeft(leftFromUsed(79.6))).toBe(20)
    // Just below the boundary — 19.9999999999 is not 20 (it is not raised to yellow).
    expect(visibleLeft(leftFromUsed(80.0000000001))).toBe(19)
    expect(usageLevel(visibleLeft(leftFromUsed(80.0000000001)))).toBe('danger')
    expect(visibleLeft(29.9)).toBe(29)
  })
})

// ── Age · 30-minute boundary · expiry (R32) ──
describe('age · stale · expired', () => {
  it('age = age_secs + seconds elapsed since receipt', () => {
    expect(elapsedSecs(1_000, 61_000)).toBe(60)
    expect(elapsedSecs(5_000, 1_000)).toBe(0)
    const r = readWindow(win({ age_secs: 100 }), 60, NOW)
    expect(r.kind === 'value' && r.ageSecs).toBe(160)
  })

  it('30-minute boundary — exactly 1800 seconds is still a fresh value, beyond it stale', () => {
    const at = (age: number) => readWindow(win({ age_secs: age }), 0, NOW)
    expect(at(STALE_AFTER_SECS)).toMatchObject({ kind: 'value', stale: false })
    expect(at(STALE_AFTER_SECS + 1)).toMatchObject({ kind: 'value', stale: true })
    // Time that elapsed locally after receipt also counts toward the age.
    expect(readWindow(win({ age_secs: STALE_AFTER_SECS - 10 }), 11, NOW)).toMatchObject({ stale: true })
  })

  it('isStale — amber (popup head) and dimming (small display) use the same boundary: 1800 seconds is usual · 1801 seconds is stale', () => {
    expect(isStale(STALE_AFTER_SECS)).toBe(false)
    expect(isStale(STALE_AFTER_SECS + 1)).toBe(true)
    expect(STALE_AFTER_SECS).toBe(1800)
  })

  it('oldest age — the maximum among windows with a value · windows with no value or expired are not counted · null if there is no such window', () => {
    const at = (age: number) => readWindow(win({ age_secs: age }), 0, NOW)
    expect(oldestAgeSecs([at(60), at(125)])).toBe(125)
    expect(oldestAgeSecs([at(125), at(60)])).toBe(125)
    // Even if one window drops out (no value), it is the age of the remaining window.
    expect(oldestAgeSecs([at(60), readWindow(null, 0, NOW)])).toBe(60)
    expect(oldestAgeSecs([readWindow(win({ used_pct: null, age_secs: 9_999 }), 0, NOW), at(30)])).toBe(30)
    // An expired window holds no value.
    expect(oldestAgeSecs([readWindow(win({ age_secs: 9_999, expired: true }), 0, NOW), at(30)])).toBe(30)
    expect(oldestAgeSecs([readWindow(null, 0, NOW), readWindow(win({ used_pct: null }), 0, NOW)])).toBeNull()
    expect(oldestAgeSecs([])).toBeNull()
    // It is the age that includes the seconds elapsed since receipt.
    expect(oldestAgeSecs([readWindow(win({ age_secs: 100 }), 60, NOW)])).toBe(160)
  })

  it('expired = daemon latch or the reset time has passed (including the same instant)', () => {
    expect(isExpired(win({ expired: true, resets_at: NOW + 999 }), NOW)).toBe(true)
    expect(isExpired(win({ resets_at: NOW }), NOW)).toBe(true)
    expect(isExpired(win({ resets_at: NOW + 1 }), NOW)).toBe(false)
    expect(isExpired(win({ resets_at: null }), NOW)).toBe(false)
    expect(readWindow(win({ resets_at: NOW - 1 }), 0, NOW)).toEqual({ kind: 'expired' })
  })

  it('remaining time = resets_at − wall clock', () => {
    expect(readWindow(win({ resets_at: NOW + 7_980 }), 0, NOW)).toMatchObject({ resetInSecs: 7_980 })
  })
})

// ── Phrasing ──
describe('time text', () => {
  it('remaining time — rounds up by minute · does not go below 1 minute', () => {
    expect(formatDuration(1)).toBe('1분')
    expect(formatDuration(60)).toBe('1분')
    expect(formatDuration(61)).toBe('2분')
    expect(formatDuration(3_600)).toBe('1시간')
    expect(formatDuration(7_980)).toBe('2시간 13분')
    expect(formatDuration(86_400 * 3 + 7_200)).toBe('3일 2시간')
    expect(formatDuration(86_400)).toBe('1일')
  })

  it('age — just now · N minutes ago · N hours ago · N days ago', () => {
    expect(formatAge(59)).toBe('방금')
    expect(formatAge(60)).toBe('1분 전')
    expect(formatAge(3_599)).toBe('59분 전')
    expect(formatAge(3_600 * 5)).toBe('5시간 전')
    expect(formatAge(86_400 * 2)).toBe('2일 전')
  })

  it('time is the Intl locale format (R28) — 24-hour 「HH:MM」', () => {
    const clock = new Intl.DateTimeFormat('ko-KR', { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' })
    expect(formatClock(NOW)).toBe(clock.format(new Date(NOW * 1000)))
  })
})

// ── Reset time — same local date "HH:MM", otherwise "M/D HH:MM" (no weekday · no zero padding) ──
describe('reset time', () => {
  // Built from local time so the same text comes out regardless of time zone. Now = passed as an argument (the real clock is not read).
  const at = (month: number, day: number, hour: number, minute: number, year = 2026) =>
    new Date(year, month - 1, day, hour, minute).getTime() / 1000

  it('same day → time only · another day → month/day + time (no weekday)', () => {
    expect(formatResetAt(at(9, 30, 21, 5), at(9, 30, 9, 0))).toBe('21:05')
    expect(formatResetAt(at(10, 1, 1, 48), at(9, 30, 9, 0))).toBe('10/1 01:48')
    expect(formatResetAt(at(10, 3, 1, 48), at(9, 30, 9, 0))).toBe('10/3 01:48')
    expect(formatResetAt(at(10, 1, 1, 48), at(9, 30, 9, 0))).not.toMatch(/[일월화수목금토]/)
  })

  it('around midnight — if the date changes it attaches the date · until the end of the same day only the time', () => {
    const now = at(9, 30, 23, 50)
    expect(formatResetAt(at(9, 30, 23, 59), now)).toBe('23:59')
    expect(formatResetAt(at(10, 1, 0, 0), now)).toBe('10/1 00:00')
    expect(formatResetAt(at(10, 1, 0, 58), now)).toBe('10/1 00:58')
    // If now has just passed midnight, a reset within that day shows the time only.
    expect(formatResetAt(at(10, 1, 0, 58), at(10, 1, 0, 0))).toBe('00:58')
    expect(formatResetAt(at(10, 1, 23, 59), at(10, 1, 0, 0))).toBe('23:59')
  })

  it('month · year boundary — month · day are not zero-padded', () => {
    expect(formatResetAt(at(11, 1, 9, 5), at(10, 31, 22, 0))).toBe('11/1 09:05')
    expect(formatResetAt(at(1, 1, 0, 5, 2027), at(12, 31, 23, 0))).toBe('1/1 00:05')
    // Even with the same month and day, a different year is a different day.
    expect(formatResetAt(at(9, 30, 10, 0, 2027), at(9, 30, 9, 0))).toBe('9/30 10:00')
  })

  it('the name of the reset time (tooltip · assistive tech) = 「reset 」 + the same rule — common to small display · popup (regardless of window kind · no ↻ symbol)', () => {
    const now = at(9, 30, 22, 0)
    expect(formatResetClock(at(9, 30, 23, 58), now)).toBe('리셋 23:58')
    expect(formatResetClock(at(10, 1, 1, 48), now)).toBe('리셋 10/1 01:48')
    expect(formatResetClock(at(10, 1, 1, 48), now)).not.toContain('↻')
  })
})

// ── Status sentence (R31 · §6 #20) ──
describe('status sentence', () => {
  const at = (secs: number) => formatClock(NOW + secs)

  it('five state phrases — Ready has no sentence', () => {
    expect(statusSentence({ kind: 'Ready' }, 0, NOW)).toBeNull()
    expect(statusSentence({ kind: 'NotInstalled', detail: null }, 0, NOW)).toBe('설치 안 됨')
    expect(statusSentence({ kind: 'NeedsLogin', detail: null }, 0, NOW)).toBe('로그인 필요')
    expect(statusSentence({ kind: 'Unavailable', detail: null }, 0, NOW)).toBe('이 계정의 한도 정보를 받을 수 없음')
    expect(statusSentence({ kind: 'Failed', next_attempt_in_secs: 300, detail: null }, 0, NOW)).toBe(
      `조회 실패 · 다음 시도 ${at(300)}`,
    )
    expect(statusSentence({ kind: 'Rejected', retry_in_secs: 720, detail: null }, 0, NOW)).toBe('거절됨 — 12분 뒤')
  })

  it('the Unavailable phrase does not assert a cause (§6 #20)', () => {
    const s = statusSentence({ kind: 'Unavailable', detail: null }, 0, NOW)!
    expect(s).not.toMatch(/로그아웃|API 키|한도가 없는/)
  })

  it('upstream raw text as is without translation · company name not repeated · code in parentheses after the source', () => {
    const upstream = 'rate_limits_available: true, rate_limits: null {status} $&'
    expect(
      statusSentence(
        { kind: 'Failed', next_attempt_in_secs: 60, detail: { kind: 'rate_limits_null', code: null, upstream } },
        0,
        NOW,
      ),
    ).toBe(`조회 실패 — 응답: ${upstream} · 다음 시도 ${at(60)}`)
    expect(
      statusSentence(
        { kind: 'NeedsLogin', detail: { kind: 'rpc_error', code: -32600, upstream: 'Unauthorized' } },
        0,
        NOW,
      ),
    ).toBe('로그인 필요 — 응답 (-32600): Unauthorized')
    // The raw text of not-installed may be an execution error, not the company's response.
    expect(
      statusSentence({ kind: 'NotInstalled', detail: { kind: 'spawn', code: null, upstream: 'program not found' } }, 0, NOW),
    ).toBe('설치 안 됨 — 오류: program not found')
  })

  it('without raw text (kind) · with only code (kind · code)', () => {
    expect(
      statusSentence(
        { kind: 'Failed', next_attempt_in_secs: 60, detail: { kind: 'timeout', code: null, upstream: null } },
        0,
        NOW,
      ),
    ).toBe(`조회 실패 (timeout) · 다음 시도 ${at(60)}`)
    expect(
      statusSentence({ kind: 'NeedsLogin', detail: { kind: 'rpc_error', code: -32001, upstream: null } }, 0, NOW),
    ).toBe('로그인 필요 (rpc_error · -32001)')
  })

  it('refusal — detail after the head\'s wait time · raw text joined with 「·」 (so 「—」 does not overlap)', () => {
    expect(
      statusSentence(
        { kind: 'Rejected', retry_in_secs: 120, detail: { kind: 'rate_limited', code: null, upstream: null } },
        0,
        NOW,
      ),
    ).toBe('거절됨 — 2분 뒤 (rate_limited)')
    expect(
      statusSentence(
        { kind: 'Rejected', retry_in_secs: 120, detail: { kind: 'rate_limited', code: 429, upstream: 'Too Many' } },
        0,
        NOW,
      ),
    ).toBe('거절됨 — 2분 뒤 · 응답 (429): Too Many')
    expect(
      statusSentence(
        { kind: 'Rejected', retry_in_secs: 120, detail: { kind: 'rate_limited', code: null, upstream: 'slow down' } },
        0,
        NOW,
      ),
    ).toBe('거절됨 — 2분 뒤 · 응답: slow down')
  })

  it('refusal wait is human-language time — there is no 「in 1440 min」·「in 0 min」', () => {
    const rejected = (secs: number, elapsed = 0) =>
      statusSentence({ kind: 'Rejected', retry_in_secs: secs, detail: null }, elapsed, NOW)
    expect(rejected(4_800)).toBe('거절됨 — 1시간 20분 뒤')
    expect(rejected(86_400)).toBe('거절됨 — 1일 뒤')
    expect(rejected(720, 300)).toBe('거절됨 — 7분 뒤')
    expect(rejected(60, 600)).toBe('거절됨 — 1분 뒤')
  })

  it('the next attempt points to the same moment reduced by the time elapsed since receipt', () => {
    expect(statusSentence({ kind: 'Failed', next_attempt_in_secs: 600, detail: null }, 240, NOW)).toBe(
      `조회 실패 · 다음 시도 ${at(360)}`,
    )
  })

  it('one line = company name + sentence · short name = without detail·number', () => {
    expect(statusLine('Claude', '로그인 필요')).toBe('Claude 로그인 필요')
    expect(stateHead({ kind: 'Ready' }, 0)).toBeNull()
    expect(stateHead({ kind: 'Failed', next_attempt_in_secs: 60, detail: { kind: 'x', code: 1, upstream: 'y' } }, 0)).toBe(
      '조회 실패',
    )
    expect(stateHead({ kind: 'Rejected', retry_in_secs: 600, detail: null }, 0)).toBe('거절됨 — 10분 뒤')
  })
})

// ADR-0373: the clock formatter follows the UI language — made per language when used, never once at module load. The
//   24-hour 「HH:MM」 text is the same in both locales, so the test reads the locale each formatter is made with.
describe('clock — locale of the UI language', () => {
  afterEach(() => {
    vi.restoreAllMocks()
    vi.resetModules()
  })

  it('formatClock / formatResetAt build their formatter in the current language, after a switch too', async () => {
    vi.resetModules()
    const Real = Intl.DateTimeFormat
    const made = vi.spyOn(Intl, 'DateTimeFormat').mockImplementation(function (
      ...args: ConstructorParameters<typeof Intl.DateTimeFormat>
    ) {
      return new Real(...args)
    } as typeof Intl.DateTimeFormat)
    const i18n = await import('../../i18n')
    const fresh = await import('./usageFormat')
    i18n.setLanguage('ko')
    expect(made).not.toHaveBeenCalled()

    fresh.formatClock(NOW)
    expect(made.mock.calls.map(c => c[0])).toEqual(['ko-KR'])

    i18n.setLanguage('en')
    fresh.formatResetAt(NOW, NOW)
    fresh.formatClock(NOW)
    expect(made.mock.calls.map(c => c[0])).toEqual(['ko-KR', 'en-US'])
  })
})
