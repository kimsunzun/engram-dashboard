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
    expect(readWindow(win({ used_pct: null, resets_at: null }), 0, NOW)).toEqual({
      kind: 'none',
      resetsAt: null,
      resetInSecs: null,
    })
    expect(readWindow(null, 0, NOW)).toEqual({ kind: 'none', resetsAt: null, resetInSecs: null })
  })

  it('리셋 시각만 실린 창 → none 이되 리셋 시각·남은 초는 든다', () => {
    expect(readWindow(win({ used_pct: null, resets_at: NOW + 600 }), 0, NOW)).toEqual({
      kind: 'none',
      resetsAt: NOW + 600,
      resetInSecs: 600,
    })
    // 리셋이 지났으면 값이 없어도 만료다.
    expect(readWindow(win({ used_pct: null, resets_at: NOW }), 0, NOW)).toEqual({ kind: 'expired' })
  })

  it('쓴 양 → 남은 양(R1) — 내림은 보정 없이 그대로(남은 양을 부풀리지 않는다)', () => {
    expect(leftFromUsed(80.4)).toBeCloseTo(19.6)
    expect(visibleLeft(leftFromUsed(80.4))).toBe(19)
    expect(visibleLeft(leftFromUsed(79.6))).toBe(20)
    // 경계 바로 아래 — 19.9999999999 는 20 이 아니다(노랑으로 올리지 않는다).
    expect(visibleLeft(leftFromUsed(80.0000000001))).toBe(19)
    expect(usageLevel(visibleLeft(leftFromUsed(80.0000000001)))).toBe('danger')
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
    expect(statusSentence({ kind: 'Ready' }, 0, NOW)).toBeNull()
    expect(statusSentence({ kind: 'NotInstalled', detail: null }, 0, NOW)).toBe('설치 안 됨')
    expect(statusSentence({ kind: 'NeedsLogin', detail: null }, 0, NOW)).toBe('로그인 필요')
    expect(statusSentence({ kind: 'Unavailable', detail: null }, 0, NOW)).toBe('이 계정의 한도 정보를 받을 수 없음')
    expect(statusSentence({ kind: 'Failed', next_attempt_in_secs: 300, detail: null }, 0, NOW)).toBe(
      `조회 실패 · 다음 시도 ${at(300)}`,
    )
    expect(statusSentence({ kind: 'Rejected', retry_in_secs: 720, detail: null }, 0, NOW)).toBe('거절됨 — 12분 뒤')
  })

  it('Unavailable 문구는 원인을 단정하지 않는다(§6 #20)', () => {
    const s = statusSentence({ kind: 'Unavailable', detail: null }, 0, NOW)!
    expect(s).not.toMatch(/로그아웃|API 키|한도가 없는/)
  })

  it('상류 원문은 번역 없이 그대로 · 회사 이름은 되풀지 않는다 · code 는 출처 뒤 괄호', () => {
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
    // 설치 안 됨의 원문은 회사 응답이 아니라 실행 오류일 수 있다.
    expect(
      statusSentence({ kind: 'NotInstalled', detail: { kind: 'spawn', code: null, upstream: 'program not found' } }, 0, NOW),
    ).toBe('설치 안 됨 — 오류: program not found')
  })

  it('원문이 없으면 (kind) · code 만 있으면 (kind · code)', () => {
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

  it('거절 — 머리의 대기 시간 뒤에 detail · 원문은 「·」로 잇는다(「—」가 겹치지 않게)', () => {
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

  it('거절 대기는 사람말 시간 — 「1440분 뒤」·「0분 뒤」가 없다', () => {
    const rejected = (secs: number, elapsed = 0) =>
      statusSentence({ kind: 'Rejected', retry_in_secs: secs, detail: null }, elapsed, NOW)
    expect(rejected(4_800)).toBe('거절됨 — 1시간 20분 뒤')
    expect(rejected(86_400)).toBe('거절됨 — 1일 뒤')
    expect(rejected(720, 300)).toBe('거절됨 — 7분 뒤')
    expect(rejected(60, 600)).toBe('거절됨 — 1분 뒤')
  })

  it('다음 시도는 받은 뒤 흐른 만큼 줄어든 같은 순간을 가리킨다', () => {
    expect(statusSentence({ kind: 'Failed', next_attempt_in_secs: 600, detail: null }, 240, NOW)).toBe(
      `조회 실패 · 다음 시도 ${at(360)}`,
    )
  })

  it('한 줄 = 회사 이름 + 문장 · 짧은 이름 = detail·수 없이', () => {
    expect(statusLine('Claude', '로그인 필요')).toBe('Claude 로그인 필요')
    expect(stateHead({ kind: 'Ready' }, 0)).toBeNull()
    expect(stateHead({ kind: 'Failed', next_attempt_in_secs: 60, detail: { kind: 'x', code: 1, upstream: 'y' } }, 0)).toBe(
      '조회 실패',
    )
    expect(stateHead({ kind: 'Rejected', retry_in_secs: 600, detail: null }, 0)).toBe('거절됨 — 10분 뒤')
  })
})
