// 사용량 슬롯의 수·문구 계산(TRD S21 usage-limit-slot §1-8 · PRD R10·R21·R31·R32) — 시계는 전부 인자로 받는다.
//
// ★두 시계를 섞지 않는다★: 스냅숏의 상대 칸(`age_secs`·`next_attempt_in_secs`·`retry_in_secs`)은 받은 순간
//   (`performance.now()`) 기준이라 거기서 흐른 만큼만 더하고 뺀다 — 벽시계가 되감겨도 흔들리지 않게. 벽시계는
//   서버가 준 절대 시각(`resets_at`)과 견줄 때만 쓴다(R29).

import type { UsageStateDetail } from '../../../crates/engram-dashboard-protocol/bindings/UsageStateDetail'
import type { UsageVendorState } from '../../../crates/engram-dashboard-protocol/bindings/UsageVendorState'
import type { UsageWindow } from '../../../crates/engram-dashboard-protocol/bindings/UsageWindow'
import { t } from '../../i18n'

/** 값의 나이가 이 초를 **넘으면** 옛 값으로 보인다(R32) — 정확히 1800초는 아직 새 값이다. */
export const STALE_AFTER_SECS = 1800

/** 시각을 찍는 로캘 — 문구 테이블(`i18n/ko.ts`)이 한국어 한 장이라 같은 로캘로 맞춘다. */
export const USAGE_LOCALE = 'ko-KR'

export type UsageLevel = 'ok' | 'warn' | 'danger' | 'none'

/** 한 창을 지금 어떻게 보이나. `none` = 들고 있는 값이 없다(0% 가 아니다 — R21). */
export type WindowReading =
  | { kind: 'none' }
  | { kind: 'expired' }
  | {
      kind: 'value'
      visible: number
      level: Exclude<UsageLevel, 'none'>
      /** 지금 기준 값의 나이(초). */
      ageSecs: number
      stale: boolean
      resetsAt: number | null
      /** 리셋까지 남은 초 — `resetsAt` 이 없으면 null. 만료가 아니므로 늘 0 보다 크다. */
      resetInSecs: number | null
    }

// 부동소수 잔차(100 − 70.00000000000001 = 29.99…)가 내림에서 1 을 깎지 않게. R10 의 내림은 소수점 이하를 버리라는
//   것(남은 양을 부풀리지 않게)이지 표현 오차를 버리라는 것이 아니다.
const FLOOR_EPSILON = 1e-9

/** 쓴 양(Claude 가 주는 값)을 남은 양으로 — 모든 회사가 같은 방향(쓸수록 준다)으로 보이게(R1). */
export function leftFromUsed(usedPct: number): number {
  return 100 - usedPct
}

/** 보이는 수 = 남은 양을 0–100 으로 자른 뒤 내림(R10·D17). 색도 이 수로 정한다. */
export function visibleLeft(left: number): number {
  return Math.floor(Math.min(100, Math.max(0, left)) + FLOOR_EPSILON)
}

/** `> 50` ok · `20–50` warn(양끝 포함) · `< 20` danger · 값 없음 = none(D17). */
export function usageLevel(visible: number | null): UsageLevel {
  if (visible === null) return 'none'
  if (visible > 50) return 'ok'
  if (visible >= 20) return 'warn'
  return 'danger'
}

/** 받은 순간부터 흐른 초 — 음수로 내려가지 않는다. */
export function elapsedSecs(receivedAt: number, nowPerf: number): number {
  return Math.max(0, (nowPerf - receivedAt) / 1000)
}

/** 받은 순간 기준 상대 초(`next_attempt_in_secs` 등)의 지금 남은 초 — 0 아래로 내려가지 않는다. */
export function remainingSecs(relativeSecs: number, elapsed: number): number {
  return Math.max(0, relativeSecs - elapsed)
}

export function isStale(ageSecs: number): boolean {
  return ageSecs > STALE_AFTER_SECS
}

/** 데몬 래치(`expired`) 또는 리셋 시각이 지남(R32). */
export function isExpired(win: UsageWindow, nowWallSecs: number): boolean {
  return win.expired || (win.resets_at !== null && win.resets_at <= nowWallSecs)
}

export function readWindow(win: UsageWindow | null, elapsed: number, nowWallSecs: number): WindowReading {
  if (win === null) return { kind: 'none' }
  if (isExpired(win, nowWallSecs)) return { kind: 'expired' }
  if (win.used_pct === null) return { kind: 'none' }
  const visible = visibleLeft(leftFromUsed(win.used_pct))
  const ageSecs = win.age_secs + elapsed
  return {
    kind: 'value',
    visible,
    level: usageLevel(visible) as Exclude<UsageLevel, 'none'>,
    ageSecs,
    stale: isStale(ageSecs),
    resetsAt: win.resets_at,
    resetInSecs: win.resets_at === null ? null : win.resets_at - nowWallSecs,
  }
}

/**
 * 분 단위 남은 시간 — 올림이고 1분 아래로 내려가지 않는다(리셋이 지나면 이 문구 대신 「리셋됨」이 선다).
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
const DATE_CLOCK = new Intl.DateTimeFormat(USAGE_LOCALE, {
  month: 'numeric',
  day: 'numeric',
  weekday: 'short',
  hour: '2-digit',
  minute: '2-digit',
  hourCycle: 'h23',
})
const DAY_KEY = new Intl.DateTimeFormat(USAGE_LOCALE, { year: 'numeric', month: 'numeric', day: 'numeric' })

export function formatClock(epochSecs: number): string {
  return CLOCK.format(new Date(epochSecs * 1000))
}

/** 오늘(로컬 날짜)이면 시각만, 아니면 날짜 + 시각. */
export function formatResetAt(epochSecs: number, nowWallSecs: number): string {
  const at = new Date(epochSecs * 1000)
  const sameDay = DAY_KEY.format(at) === DAY_KEY.format(new Date(nowWallSecs * 1000))
  return sameDay ? CLOCK.format(at) : DATE_CLOCK.format(at)
}

// 이어 붙이는 문장 부호. 상태 문구 머리에 이미 「—」가 있으면(거절됨 — N분 뒤) 상류 원문은 「·」로 잇는다.
const DASH = ' — '
const DOT = ' · '

function withDetail(status: string, detail: UsageStateDetail | null, vendorName: string, upstreamJoin: string): string {
  if (detail === null) return status
  if (detail.upstream !== null) {
    const source =
      detail.code !== null
        ? t('usage.detailUpstreamCode', { vendor: vendorName, code: String(detail.code), upstream: detail.upstream })
        : t('usage.detailUpstream', { vendor: vendorName, upstream: detail.upstream })
    return `${status}${upstreamJoin}${source}`
  }
  return `${status} ${t('usage.detailKind', { kind: detail.kind })}`
}

/**
 * 비정상 상태의 한 줄 문장(R31) — 배지의 `aria-label`·`title` 과 팝업 맨 위 줄이 같은 이 문장을 쓴다. `Ready` 면 null.
 *
 * 모양 = 상태 문구 + detail(상류 원문이 있으면 「<회사> 응답[ (code)]: <원문>」, 없으면 「(<kind>)」) + 수.
 * `elapsed` = 스냅숏을 받은 뒤 흐른 초.
 */
export function statusSentence(
  state: UsageVendorState,
  vendorName: string,
  elapsed: number,
  nowWallSecs: number,
): string | null {
  switch (state.kind) {
    case 'Ready':
      return null
    case 'NotInstalled':
      return withDetail(t('usage.stateNotInstalled'), state.detail, vendorName, DASH)
    case 'NeedsLogin':
      return withDetail(t('usage.stateNeedsLogin'), state.detail, vendorName, DASH)
    case 'Unavailable':
      return withDetail(t('usage.stateUnavailable'), state.detail, vendorName, DASH)
    case 'Failed': {
      const at = nowWallSecs + remainingSecs(state.next_attempt_in_secs, elapsed)
      const head = withDetail(t('usage.stateFailed'), state.detail, vendorName, DASH)
      return `${head}${DOT}${t('usage.nextAttempt', { time: formatClock(at) })}`
    }
    case 'Rejected': {
      // 기한이 로컬로 먼저 닿아도 「0분 뒤」는 보이지 않는다 — 거절 끝은 데몬이 상태를 바꿔 방송한다.
      const minutes = Math.max(1, Math.ceil(remainingSecs(state.retry_in_secs, elapsed) / 60))
      return withDetail(t('usage.stateRejected', { minutes: String(minutes) }), state.detail, vendorName, DOT)
    }
  }
}
