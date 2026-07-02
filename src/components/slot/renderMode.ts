// RenderMode — slot 렌더러 선택을 나타내는 프론트 전용 union(코드베이스 "enum" 관례 = TS union, 예: AgentStatus/SplitDir).
//
// ★프론트 전용★: 백엔드 wire 는 이 개념을 모른다(layoutTypes 재노출 대상 아님). 슬롯이 어떤 렌더러
// (xterm 터미널 / 구조화 RichSlot / 평문 DomSlot)를 마운트할지 고르는 순수 프론트 결정이다.

import type { AgentInfo } from '../../api/types'

/**
 * 유효한 RenderMode 값의 단일 출처(union 과 런타임 가드가 공유).
 * ★as const★: 아래 RenderMode union 을 이 배열에서 파생시켜, setRenderMode 의 런타임 가드
 * (isRenderMode)와 타입이 한 곳에서 갈라진다 — 새 모드를 추가하면 union·가드가 함께 늘어난다.
 */
export const RENDER_MODES = ['terminal', 'rich', 'dom'] as const

/** 슬롯 렌더러 종류 — 'terminal'=xterm, 'rich'=구조화 RichSlot, 'dom'=평문 관측 DomSlot(§5). */
export type RenderMode = (typeof RENDER_MODES)[number]

/**
 * 런타임 RenderMode 가드 — 미타입 JS(window.__engramLayout)에서 온 값을 store 에 쓰기 전 검증.
 * setRenderMode 가 이걸로 걸러 잘못된 mode 가 오버라이드로 새는 걸 막는다(ViewLayoutRenderer switch 가
 * 알 수 없는 값을 조용히 terminal 로 떨어뜨리는 걸 방지 — 무효 입력은 no-op 이 맞다).
 */
export function isRenderMode(mode: unknown): mode is RenderMode {
  return typeof mode === 'string' && (RENDER_MODES as readonly string[]).includes(mode)
}

/**
 * agent 의 output caps 로부터 유도하는 기본 렌더 모드(오버라이드 없을 때).
 * structured(NDJSON 캐리어) → 'rich', 아니면 'terminal'(xterm). 근거 ADR-0002/0044.
 *
 * ★wire boolean 을 저장하지 않고 매번 유도한다★: capabilities.output.structured 는 wire 권위 값이라
 * 프론트가 복제·보관하면 드리프트 원천이 된다 — 렌더 시점에 그 값에서 파생만 한다(오버라이드는 별도 저장).
 */
export function defaultRenderMode(agent: AgentInfo): RenderMode {
  return agent.capabilities.output.structured ? 'rich' : 'terminal'
}
