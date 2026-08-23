// 에이전트 상태 → 아이콘·색 매핑. ADR-0062 §38 이 요구하는 것은 **headless 전 분기 고정**이고, 이 파일은
//   그것만 지킨다 — react-arborist 를 끌고 오는 AgentList 없이 매핑을 단독 테스트할 수 있다.
//   ★"외부 의존 0"은 아니다★: lucide 아이콘 컴포넌트를 값으로 돌려주므로 그 import 는 불가피하다(ADR-0168 로
//   문자 글리프에서 아이콘으로 교체되며 생긴 변화). JSX 는 여기 두지 않는다(그리는 쪽 = AgentList).

import { Circle, Square, X, type LucideIcon } from 'lucide-react'

/** 렌더러가 그대로 svg 에 넘기는 한 벌. */
export type StatusGlyphIcon = {
  Icon: LucideIcon
  /** lucide 기본은 `none`(외곽선) — 채움 여부가 Running(채운 원)과 Reserved(빈 원)를 가르는 *모양* 차이다. */
  fill: 'currentColor' | 'none'
  /** DOM 관측 표면(테스트·CDP·LLM 셀렉터)에 나가는 안정 토큰. 아이콘 컴포넌트는 직렬화되지 않는다. */
  shape: 'filled-circle' | 'square' | 'x' | 'circle'
}

/**
 * 색이 아닌 **모양**이 상태를 담아 e-ink 에서도 구분된다.
 *
 * 매핑(ADR-0062):
 *   - Running                → 채운 원 (작업중)
 *   - Exiting/Exited/Killed  → 사각    (멈춤 — Exiting 은 terminal 직전 전이)
 *   - Failed                 → ✗ 모양  (에러)
 *   - Reserved(프론트 합성)   → 빈 원   (유휴/미spawn 깡통)
 *   - 그 외(미지 status)      → 빈 원   (안전 degrade — 빈 칸 방지)
 *
 * ★모양이 겹치는 분기를 만들지 말 것★: 색만 다르고 모양이 같은 두 상태는 e-ink 에서 구분이 사라진다
 *   (e-ink 블록이 `--status-running` 을 `var(--text-muted)` 로 중립화한다 — `statusGlyphColor` 주석).
 *
 * ★◐(입력대기)에 해당하는 반쯤 채운 모양은 절대 반환하지 않는다★: 백엔드가 "입력 대기" 신호를 내지
 *   않으므로 그 분기가 없다(ADR-0062 — 미점등은 결함이 아니라 의도). 백엔드가 신호를 낼 때 여기 분기를
 *   추가하는 것이 정규 경로.
 *
 * ★텍스트 글리프(`● ◻ ✗ ○`)로 되돌리지 말 것★: UI 폰트가 monospace 에서 Segoe UI 로 바뀐 뒤 글자마다
 *   크기·굵기·baseline 이 달라 행 라벨과 세로 정렬이 어긋났다. 아이콘은 폰트와 무관하게 12px 정사각이다.
 */
export function statusGlyphIcon(status: string): StatusGlyphIcon {
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
 * ADR-0062 개정: 원래 "상태=모양(색 아님)"이었으나
 * e-ink 를 별도 모드로 분리하기로 하며 다른 테마에선 색 허용. 색은 모양을 대체하지 않고 *부가*한다.
 * ★색 리터럴 금지·변수만★(ADR-0062 §44):
 * green 값은 theme.css 의 --status-running 이 소유하고, e-ink 블록이 이를 var(--text-muted) 로 중립화해
 * e-ink 에선 모양만 남는다(모양이 여전히 1차 신호).
 */
export function statusGlyphColor(status: string): string {
  return status === 'Running' ? 'var(--status-running)' : 'var(--text-muted)'
}
