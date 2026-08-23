// ★공유 모듈로 분리★: 원래 AgentList.tsx 에 있던 순수 글리프 함수들이 여기 산다. RichSlot 헤더가 같은
//   상태 글리프를 그리는데, 무거운 AgentList 모듈(react-arborist·트리 로직 전부) 전이 의존을 피하려
//   순수 헬퍼만 이 경량 모듈에 둔다. AgentList 는 이 값을 re-export 해 기존 importer/test 를 그대로 유지한다.
//
// ★한 행의 기호는 하나뿐이다(ADR-0162)★: 축이 늘어도(상태 · 마지막 실패) 기호를 나란히 두지 않고
//   갈아끼운다. 그 판정은 `isFailureBlocked` 한 곳에 있고 기호·색·hover 가 함께 그것을 따른다.

import type { AgentFailureKind } from '../../api/types'

/** 마지막 실패가 상태 기호를 갈아끼울 때 쓰는 금지 표식(ADR-0162 — 기호를 하나 더 늘리지 않는다). */
const BLOCKED_GLYPH = '⊘'

/**
 * 마지막 실패가 지금 이 행의 기호를 차지하나 — **기호·색·hover 가 함께 따르는 단일 판정**.
 *
 * ★도는 중이 이긴다(ADR-0162)★: 마지막 실패를 들고 있어도 떠 있으면 원래 상태 기호를 그린다. 실패 표시는
 *   "지금 이 항목을 못 연다" 는 신호라, 도는 항목에 얹으면 거짓말이 된다.
 * ★`Exiting` 도 「도는 중」 쪽이다★: 그건 매니저가 내는 **과도기** 전이지 머무는 상태가 아니다(ADR-0005).
 *   쉬는 상태로 세면 정상 종료 몇 초 동안 행이 금지 표식으로 깜빡인다.
 * ★갈아끼워도 손실이 없는 이유★: 표시가 뜨는 경우는 안 떠 있는 항목뿐이고, 그때 원래 기호(◻/○)는
 *   나를 정보가 없다.
 */
export function isFailureBlocked(
  status: string,
  lastFailure?: AgentFailureKind | null,
): boolean {
  return Boolean(lastFailure) && status !== 'Running' && status !== 'Exiting'
}

/**
 * 색이 아닌 모양이 상태를 담아 e-ink 에서도 구분된다.
 *
 * `lastFailure` 가 기호를 차지하면(위 `isFailureBlocked`) 상태와 무관하게 금지 표식을 **갈아끼운다** —
 * 두 번째 기호를 나란히 두지 않는다(ADR-0162).
 *
 * 매핑(ADR-0062):
 *   - Running               → ● (작업중)
 *   - Exiting/Exited/Killed  → ◻ (멈춤 — Exiting 은 terminal 직전 전이)
 *   - Failed                → ✗ (에러)
 *   - Reserved(프론트 합성)   → ○ (유휴/미spawn 깡통)
 *   - 그 외(미지 status)      → ○ (안전 degrade — 빈 칸 방지)
 *
 * ★◐(입력대기)는 어휘로만 존재 — 절대 점등하지 않는다★: 백엔드가 "입력 대기" 신호를 내지 않으므로
 *   이 함수는 ◐ 를 반환하는 분기가 없다(ADR-0062 — 미점등은 결함이 아니라 의도). 백엔드가 신호를 낼 때
 *   이 함수에 분기를 추가하는 것이 정규 경로.
 */
export function statusGlyph(status: string, lastFailure?: AgentFailureKind | null): string {
  // ADR-0162
  if (isFailureBlocked(status, lastFailure)) return BLOCKED_GLYPH
  switch (status) {
    case 'Running':
      return '●'
    case 'Exiting':
    case 'Exited':
    case 'Killed':
      return '◻'
    case 'Failed':
      return '✗'
    case 'Reserved':
      return '○'
    default:
      return '○'
  }
}

/**
 * ADR-0062 개정: 원래 "상태=모양(색 아님)"이었으나
 * e-ink 를 별도 모드로 분리하기로 하며 다른 테마에선 색 허용. 색은 모양을 대체하지 않고 *부가*한다.
 * ★색 리터럴 금지·변수만★(ADR-0062 §44):
 * green 값은 theme.css 의 --status-running 이 소유하고, e-ink 블록이 이를 var(--text-muted) 로 중립화해
 * e-ink 에선 모양만 남는다(모양이 여전히 1차 신호). 경고색(--status-blocked)도 같은 규칙을 따른다.
 */
export function statusGlyphColor(status: string, lastFailure?: AgentFailureKind | null): string {
  // ADR-0162
  if (isFailureBlocked(status, lastFailure)) return 'var(--status-blocked)'
  return status === 'Running' ? 'var(--status-running)' : 'var(--text-muted)'
}
