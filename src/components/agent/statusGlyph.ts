// ★공유 모듈로 분리★: 원래 AgentList.tsx 에 있던 두 순수 함수를 여기로 옮겼다. RichSlot 헤더가 같은
//   상태 글리프를 그리는데, 무거운 AgentList 모듈(react-arborist·트리 로직 전부) 전이 의존을 피하려
//   순수 헬퍼만 이 경량 모듈에 둔다. AgentList 는 이 값을 re-export 해 기존 importer/test 를 그대로 유지한다.

/**
 * 색이 아닌 모양이 상태를 담아 e-ink 에서도 구분된다.
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
export function statusGlyph(status: string): string {
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
 * e-ink 에선 모양만 남는다(모양이 여전히 1차 신호).
 */
export function statusGlyphColor(status: string): string {
  return status === 'Running' ? 'var(--status-running)' : 'var(--text-muted)'
}
