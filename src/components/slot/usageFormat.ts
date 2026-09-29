// 사용량 슬롯의 수·문구 계산(TRD S21 usage-limit-slot §1-8 · PRD R10·R21·R31·R32) — 시계는 전부 인자로 받는다.
//
// ★두 시계를 섞지 않는다★: 스냅숏의 상대 칸(`age_secs`·`next_attempt_in_secs`·`retry_in_secs`)은 받은 순간
//   (`performance.now()`) 기준이라 거기서 흐른 만큼만 더하고 뺀다 — 벽시계가 되감겨도 흔들리지 않게. 벽시계는
//   서버가 준 절대 시각(`resets_at`)과 견줄 때만 쓴다(R29).
// ★문장의 어순·이음 부호는 전부 `t()` 틀에 있다(R28)★ — 여기서는 어느 틀을 고를지만 정한다.

import type { UsageStateDetail } from '../../../crates/engram-dashboard-protocol/bindings/UsageStateDetail'
import type { UsageVendorState } from '../../../crates/engram-dashboard-protocol/bindings/UsageVendorState'
import type { UsageWindow } from '../../../crates/engram-dashboard-protocol/bindings/UsageWindow'
import { t } from '../../i18n'

/** 값의 나이가 이 초를 **넘으면** 옛 값으로 보인다(R32) — 정확히 1800초는 아직 새 값이다. */
export const STALE_AFTER_SECS = 1800

/** 시각을 찍는 로캘 — 문구 테이블(`i18n/ko.ts`)이 한국어 한 장이라 같은 로캘로 맞춘다. */
export const USAGE_LOCALE = 'ko-KR'

export type UsageLevel = 'ok' | 'warn' | 'danger' | 'none'

/**
 * 한 창을 지금 어떻게 보이나. `none` = 들고 있는 값이 없다(0% 가 아니다 — R21) — 리셋 시각만 실린 창이면 그 시각은
 * 들고 있다.
 */
export type WindowReading =
  | {
      kind: 'none'
      resetsAt: number | null
      /** 리셋까지 남은 초 — `resetsAt` 이 없으면 null. 만료가 아니므로 늘 0 보다 크다. */
      resetInSecs: number | null
    }
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

/** 쓴 양(Claude 가 주는 값)을 남은 양으로 — 모든 회사가 같은 방향(쓸수록 준다)으로 보이게(R1). */
export function leftFromUsed(usedPct: number): number {
  return 100 - usedPct
}

/**
 * 보이는 수 = 남은 양을 0–100 으로 자른 뒤 내림(R10·D17) — 남은 양을 부풀리지 않는다(19.9999… 는 19). 색도 이 수로
 * 정한다.
 */
export function visibleLeft(left: number): number {
  return Math.floor(Math.min(100, Math.max(0, left)))
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
 * 분 단위 남은 시간 — 올림이고 1분 아래로 내려가지 않는다(리셋이 지나면 이 문구 대신 「리셋됨」이, 거절이 끝나면
 * 데몬이 바꾼 상태가 선다).
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

// 날짜는 `Date` 의 로컬 getter 로 가른다 — 시각을 찍는 `CLOCK` 도 같은 기본 시간대라 둘이 어긋나지 않는다.
function sameLocalDay(a: Date, b: Date): boolean {
  return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate()
}

// ADR-0257: 오늘이 아닌 리셋은 요일이 아니라 날짜 — 「리셋 수」 는 횟수로 읽히고, 다음 주 같은 요일이면 모호하다.
/**
 * 리셋 시각(사용자 결정 2026-09-29) — 지금과 같은 로컬 날짜면 「HH:MM」, 아니면 「M/D HH:MM」(요일 없음 · 월·일 0 채움
 * 없음). 작은 표시와 팝업이 같은 규칙을 쓴다.
 */
export function formatResetAt(epochSecs: number, nowWallSecs: number): string {
  const at = new Date(epochSecs * 1000)
  const time = CLOCK.format(at)
  if (sameLocalDay(at, new Date(nowWallSecs * 1000))) return time
  return t('usage.resetDateTime', { month: String(at.getMonth() + 1), day: String(at.getDate()), time })
}

/** 작은 표시의 리셋 시각 — 남은 시간은 팝업에만 둔다. */
export function formatResetClock(epochSecs: number, nowWallSecs: number): string {
  return t('usage.resetClock', { time: formatResetAt(epochSecs, nowWallSecs) })
}

/**
 * detail 을 상태 문구에 잇는다. `afterWait` = 문구 머리가 이미 대기 시간을 「—」로 달고 있다(거절됨 — N분 뒤) — 그때는
 * 상류 원문을 다른 틀로 이어 「—」가 겹치지 않게 한다.
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

/** 거절 대기 — 받은 뒤 흐른 만큼 줄인 사람말 시간. */
function rejectedHead(retryInSecs: number, elapsed: number): string {
  return t('usage.stateRejected', { duration: formatDuration(remainingSecs(retryInSecs, elapsed)) })
}

/**
 * 비정상 상태의 짧은 이름(detail·다음 시도 없이) — 요약 버튼의 접근성 이름처럼 짧게 말할 자리용. `Ready` 면 null.
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
 * 비정상 상태의 문장(R31) — 회사 이름 없이. 모양 = 상태 문구 + detail(상류 원문이 있으면 「응답[ (code)]: <원문>」,
 * 없으면 「(kind[ · code])」) + 수. 상류 원문은 번역하지 않고 그대로 끼운다. `Ready` 면 null.
 *
 * ★출처 낱말★: `NotInstalled` 의 원문은 회사 응답이 아니라 OS 의 실행 오류일 수 있어 「오류」로 부른다.
 * `elapsed` = 스냅숏을 받은 뒤 흐른 초.
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
 * 회사 이름을 앞에 단 한 줄 — 배지의 `aria-label`·`title` 과 팝업 맨 위 줄이 이 같은 문자열을 쓴다(R31). 문장 안에서는
 * 회사 이름을 되풀지 않는다(「Claude 조회 실패 — 응답: …」).
 */
export function statusLine(vendorName: string, sentence: string): string {
  return t('usage.statusLine', { vendor: vendorName, sentence })
}
