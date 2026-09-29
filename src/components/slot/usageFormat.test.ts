// usageFormat — 사용량 슬롯의 수·문구(TRD S21 usage-limit-slot §4 「프론트 표시」 행 중 순수 계산 몫).

import { describe, expect, it } from 'vitest'

import type { UsageWindow } from '../../../crates/engram-dashboard-protocol/bindings/UsageWindow'
import {
  STALE_AFTER_SECS,
  USAGE_LOCALE,
  elapsedSecs,
  formatAge,
  formatClock,
  formatDuration,
  formatResetAt,
  isExpired,
  leftFromUsed,
  readWindow,
  statusSentence,
  usageLevel,
  visibleLeft,
} from './usageFormat'

const NOW = 1_900_000_000

function win(over: Partial<UsageWindow> = {}): UsageWindow {
  return { used_pct: 38, resets_at: NOW + 3600, age_secs: 0, expired: false, ...over }
}

// ── 보이는 수·색 경계(R10·D17 — 남은 양 기준) ──
describe('보이는 수와 색 구간', () => {
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
    it(`남은 ${left} → ${visible} · ${level}`, () => {
      expect(visibleLeft(left)).toBe(visible)
      expect(usageLevel(visibleLeft(left))).toBe(level)
    })
  }

  it('값 없음 → none(0% 가 아니다 — R21)', () => {
    expect(usageLevel(null)).toBe('none')
    expect(readWindow(win({ used_pct: null }), 0, NOW)).toEqual({ kind: 'none' })
    expect(readWindow(null, 0, NOW)).toEqual({ kind: 'none' })
  })

  it('쓴 양 → 남은 양(R1) — 부동소수 잔차가 내림에서 1 을 깎지 않는다', () => {
    expect(leftFromUsed(80.4)).toBeCloseTo(19.6)
    expect(visibleLeft(leftFromUsed(80.4))).toBe(19)
    expect(visibleLeft(leftFromUsed(79.6))).toBe(20)
    expect(visibleLeft(leftFromUsed(70.00000000000001))).toBe(30)
    expect(visibleLeft(29.9)).toBe(29)
  })
})

// ── 나이·30분 경계·만료(R32) ──
describe('나이 · 오래됨 · 만료', () => {
  it('나이 = age_secs + 받은 뒤 흐른 초', () => {
    expect(elapsedSecs(1_000, 61_000)).toBe(60)
    expect(elapsedSecs(5_000, 1_000)).toBe(0)
    const r = readWindow(win({ age_secs: 100 }), 60, NOW)
    expect(r.kind === 'value' && r.ageSecs).toBe(160)
  })

  it('30분 경계 — 정확히 1800초는 아직 새 값, 넘으면 오래됨', () => {
    const at = (age: number) => readWindow(win({ age_secs: age }), 0, NOW)
    expect(at(STALE_AFTER_SECS)).toMatchObject({ kind: 'value', stale: false })
    expect(at(STALE_AFTER_SECS + 1)).toMatchObject({ kind: 'value', stale: true })
    // 받은 뒤 로컬로 흐른 시간도 나이에 든다.
    expect(readWindow(win({ age_secs: STALE_AFTER_SECS - 10 }), 11, NOW)).toMatchObject({ stale: true })
  })

  it('만료 = 데몬 래치 또는 리셋 시각이 지남(같은 순간 포함)', () => {
    expect(isExpired(win({ expired: true, resets_at: NOW + 999 }), NOW)).toBe(true)
    expect(isExpired(win({ resets_at: NOW }), NOW)).toBe(true)
    expect(isExpired(win({ resets_at: NOW + 1 }), NOW)).toBe(false)
    expect(isExpired(win({ resets_at: null }), NOW)).toBe(false)
    expect(readWindow(win({ resets_at: NOW - 1 }), 0, NOW)).toEqual({ kind: 'expired' })
  })

  it('남은 시간 = resets_at − 벽시계', () => {
    expect(readWindow(win({ resets_at: NOW + 7_980 }), 0, NOW)).toMatchObject({ resetInSecs: 7_980 })
  })
})

// ── 문구 ──
describe('시간 문구', () => {
  it('남은 시간 — 분 올림 · 1분 아래로 안 내려감', () => {
    expect(formatDuration(1)).toBe('1분')
    expect(formatDuration(60)).toBe('1분')
    expect(formatDuration(61)).toBe('2분')
    expect(formatDuration(3_600)).toBe('1시간')
    expect(formatDuration(7_980)).toBe('2시간 13분')
    expect(formatDuration(86_400 * 3 + 7_200)).toBe('3일 2시간')
    expect(formatDuration(86_400)).toBe('1일')
  })

  it('나이 — 방금 · N분 전 · N시간 전 · N일 전', () => {
    expect(formatAge(59)).toBe('방금')
    expect(formatAge(60)).toBe('1분 전')
    expect(formatAge(3_599)).toBe('59분 전')
    expect(formatAge(3_600 * 5)).toBe('5시간 전')
    expect(formatAge(86_400 * 2)).toBe('2일 전')
  })

  it('시각은 Intl 로캘 포맷(R28) — 오늘이면 시각만, 아니면 날짜까지', () => {
    const clock = new Intl.DateTimeFormat(USAGE_LOCALE, { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' })
    expect(formatClock(NOW)).toBe(clock.format(new Date(NOW * 1000)))
    expect(formatResetAt(NOW + 60, NOW)).toBe(clock.format(new Date((NOW + 60) * 1000)))
    const far = formatResetAt(NOW + 86_400 * 3, NOW)
    expect(far).not.toBe(clock.format(new Date((NOW + 86_400 * 3) * 1000)))
    expect(far).toContain(clock.format(new Date((NOW + 86_400 * 3) * 1000)))
  })
})

// ── 상태 문장(R31 · §6 #20) ──
describe('상태 문장', () => {
  const at = (secs: number) => formatClock(NOW + secs)

  it('다섯 상태 문구 — Ready 는 문장이 없다', () => {
    expect(statusSentence({ kind: 'Ready' }, 'Claude', 0, NOW)).toBeNull()
    expect(statusSentence({ kind: 'NotInstalled', detail: null }, 'Claude', 0, NOW)).toBe('설치 안 됨')
    expect(statusSentence({ kind: 'NeedsLogin', detail: null }, 'Claude', 0, NOW)).toBe('로그인 필요')
    expect(statusSentence({ kind: 'Unavailable', detail: null }, 'Claude', 0, NOW)).toBe(
      '이 계정의 한도 정보를 받을 수 없음',
    )
    expect(statusSentence({ kind: 'Failed', next_attempt_in_secs: 300, detail: null }, 'Claude', 0, NOW)).toBe(
      `조회 실패 · 다음 시도 ${at(300)}`,
    )
    expect(statusSentence({ kind: 'Rejected', retry_in_secs: 720, detail: null }, 'Claude', 0, NOW)).toBe(
      '거절됨 — 12분 뒤',
    )
  })

  it('Unavailable 문구는 원인을 단정하지 않는다(§6 #20)', () => {
    const s = statusSentence({ kind: 'Unavailable', detail: null }, 'Claude', 0, NOW)!
    expect(s).not.toMatch(/로그아웃|API 키|한도가 없는/)
  })

  it('상류 원문은 번역 없이 그대로 · 없으면 (kind) · code 는 접두 뒤 괄호', () => {
    const upstream = 'rate_limits_available: true, rate_limits: null {name} $&'
    expect(
      statusSentence(
        { kind: 'Failed', next_attempt_in_secs: 60, detail: { kind: 'rate_limits_null', code: null, upstream } },
        'Claude',
        0,
        NOW,
      ),
    ).toBe(`조회 실패 — Claude 응답: ${upstream} · 다음 시도 ${at(60)}`)
    expect(
      statusSentence(
        { kind: 'Failed', next_attempt_in_secs: 60, detail: { kind: 'timeout', code: null, upstream: null } },
        'Claude',
        0,
        NOW,
      ),
    ).toBe(`조회 실패 (timeout) · 다음 시도 ${at(60)}`)
    expect(
      statusSentence(
        { kind: 'NeedsLogin', detail: { kind: 'rpc_error', code: -32600, upstream: 'Unauthorized' } },
        'Codex',
        0,
        NOW,
      ),
    ).toBe('로그인 필요 — Codex 응답 (-32600): Unauthorized')
    expect(
      statusSentence(
        { kind: 'Rejected', retry_in_secs: 120, detail: { kind: 'rate_limited', code: null, upstream: null } },
        'Claude',
        0,
        NOW,
      ),
    ).toBe('거절됨 — 2분 뒤 (rate_limited)')
    expect(
      statusSentence(
        { kind: 'Rejected', retry_in_secs: 120, detail: { kind: 'rate_limited', code: 429, upstream: 'Too Many' } },
        'Claude',
        0,
        NOW,
      ),
    ).toBe('거절됨 — 2분 뒤 · Claude 응답 (429): Too Many')
  })

  it('다음 시도·거절 대기는 받은 뒤 흐른 만큼 줄어든다', () => {
    expect(statusSentence({ kind: 'Failed', next_attempt_in_secs: 600, detail: null }, 'Claude', 240, NOW)).toBe(
      `조회 실패 · 다음 시도 ${at(360)}`,
    )
    expect(statusSentence({ kind: 'Rejected', retry_in_secs: 720, detail: null }, 'Claude', 300, NOW)).toBe(
      '거절됨 — 7분 뒤',
    )
    // 기한이 로컬로 먼저 닿아도 「0분 뒤」는 보이지 않는다.
    expect(statusSentence({ kind: 'Rejected', retry_in_secs: 60, detail: null }, 'Claude', 600, NOW)).toBe(
      '거절됨 — 1분 뒤',
    )
  })
})
