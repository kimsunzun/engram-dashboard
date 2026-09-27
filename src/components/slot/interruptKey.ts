// ADR-0237: 채팅 칸(RichSlot) 안의 키 하나가 「도는 턴 끊기」인지 가르는 순수 술어(DOM 0).
//   발화 조건의 정본 = `docs/process/S21-chat-ux/trd.md` §3-2 — 이 파일은 그 목록을 그대로 옮긴 것이다.
//   T-36(단축키 시스템 — 보류)이 키바인딩 표를 세우면 칸 조건(`ctx` 쪽)이 그 표 한 줄의 `when` 이 된다. ★술어가
//   통째로 옮겨 가지는 않는다★ — `when` 은 이벤트를 받지 않는 `() => boolean` 이고(`commands/registry.ts`) 전역
//   리스너는 편집 대상(입력창)을 건너뛰므로, 키 모양 판정(키 · 수식키 · 반복 · IME · `defaultPrevented` · 대상)은
//   그 표의 매처나 지금의 지역 capture 에 남는다. 끊는 동작은 명령 `agent.interrupt` 에 있어 따라 옮길 것이 없다.

/** Esc 가 먹히는 범위 — `'slot'` = 칸 안 어디든(입력창 + 대화 본문) · `'input'` = 입력창에 포커스일 때만. */
export type InterruptEscScope = 'slot' | 'input'

/** 사용자 결정 U6 = 칸 안 어디든. */
export const ESC_SCOPE: InterruptEscScope = 'slot'

/**
 * 열려 있는 동안 Esc 가 턴을 끊지 않는 오버레이의 표지. 문서 전역 Esc 로 닫히는 셋(에이전트 고르기 ·
 * 트리 행 메뉴 · 프리셋 행 메뉴)과 우클릭 메뉴가 뿌리에 단다 — 그 Esc 는 오버레이를 닫는 키다.
 */
export const OVERLAY_SELECTOR = '[data-engram-overlay]'

/** 술어가 읽는 칸만 — React `KeyboardEvent` 가 그대로 맞는다. */
export interface InterruptKeyEvent {
  key: string
  ctrlKey: boolean
  altKey: boolean
  shiftKey: boolean
  metaKey: boolean
  repeat: boolean
  keyCode: number
  defaultPrevented: boolean
  target: EventTarget | null
  nativeEvent: { isComposing: boolean }
}

export interface InterruptKeyContext {
  /** 칸이 응답을 기다리거나 받는 중으로 보인다(RichSlot 의 `streaming`). */
  streaming: boolean
  /** 부재 막이 선 조건(종료 · 연결 끊김 · 구독 정지). */
  agentUnavailable: boolean
  /** 그 에이전트의 통로가 끊기를 지원한다(`capabilities.control.interrupt`). */
  canInterrupt: boolean
  /** 문서에 [`OVERLAY_SELECTOR`] 가 있다 — 부르는 쪽이 재서 넘긴다. */
  overlayOpen: boolean
  scope: InterruptEscScope
  /** 칸의 입력창 노드 — `scope === 'input'` 일 때만 읽는다. */
  textarea: EventTarget | null
}

export function isInterruptEscape(e: InterruptKeyEvent, ctx: InterruptKeyContext): boolean {
  if (e.key !== 'Escape') return false
  if (e.ctrlKey || e.altKey || e.shiftKey || e.metaKey) return false
  if (e.repeat) return false
  // 조합 중 Esc 는 조합만 취소한다 — 입력창 Enter 의 IME 판정과 같은 식이다(WebView2 는 keyCode 229 로 싣는다).
  if (e.nativeEvent.isComposing || e.keyCode === 229) return false
  // Radix 레이어는 문서 capture 에서 먼저 먹는다 — 그 Esc 는 레이어를 닫는 키다.
  if (e.defaultPrevented) return false
  if (ctx.overlayOpen) return false
  if (!ctx.streaming || ctx.agentUnavailable || !ctx.canInterrupt) return false
  if (ctx.scope === 'input' && e.target !== ctx.textarea) return false
  return true
}
