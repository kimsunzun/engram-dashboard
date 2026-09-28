// ADR-0237: Esc 끊기 술어 표 — 조건 하나씩 거짓이면 발화하지 않고, 전부 참일 때만 발화한다(TRD §3-2).

import { describe, expect, it } from 'vitest'

import {
  ESC_SCOPE,
  isInterruptEscape,
  type InterruptKeyContext,
  type InterruptKeyEvent,
} from './interruptKey'

const textarea = new EventTarget()
const body = new EventTarget()

function key(over: Partial<InterruptKeyEvent> = {}): InterruptKeyEvent {
  return {
    key: 'Escape',
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    metaKey: false,
    repeat: false,
    keyCode: 27,
    defaultPrevented: false,
    target: textarea,
    nativeEvent: { isComposing: false },
    ...over,
  }
}

function ctx(over: Partial<InterruptKeyContext> = {}): InterruptKeyContext {
  return {
    streaming: true,
    interrupting: false,
    agentUnavailable: false,
    canInterrupt: true,
    overlayOpen: false,
    scope: 'slot',
    textarea,
    ...over,
  }
}

describe('isInterruptEscape', () => {
  it('조건이 전부 참이면 발화한다', () => {
    expect(isInterruptEscape(key(), ctx())).toBe(true)
  })

  it.each<[string, Partial<InterruptKeyEvent>]>([
    ['다른 키', { key: 'Enter' }],
    ['ctrl', { ctrlKey: true }],
    ['alt', { altKey: true }],
    ['shift', { shiftKey: true }],
    ['meta', { metaKey: true }],
    ['누른 채 반복', { repeat: true }],
    ['IME 조합 중(isComposing)', { nativeEvent: { isComposing: true } }],
    ['IME 조합 중(keyCode 229)', { keyCode: 229 }],
    ['앞선 처리기가 먹었다(defaultPrevented)', { defaultPrevented: true }],
  ])('키 조건 하나만 어긋나도 발화하지 않는다 — %s', (_label, over) => {
    expect(isInterruptEscape(key(over), ctx())).toBe(false)
  })

  it.each<[string, Partial<InterruptKeyContext>]>([
    ['오버레이가 열려 있다', { overlayOpen: true }],
    ['턴이 안 돈다', { streaming: false }],
    ['끊기를 이미 보내고 턴 끝을 기다린다(ADR-0244)', { interrupting: true }],
    ['에이전트가 지금 없다', { agentUnavailable: true }],
    ['통로가 끊기를 지원하지 않는다', { canInterrupt: false }],
  ])('칸 조건 하나만 어긋나도 발화하지 않는다 — %s', (_label, over) => {
    expect(isInterruptEscape(key(), ctx(over))).toBe(false)
  })

  it("범위 'slot' 은 입력창 밖(대화 본문)에서 온 Esc 도 발화한다", () => {
    expect(isInterruptEscape(key({ target: body }), ctx({ scope: 'slot' }))).toBe(true)
  })

  it("범위 'input' 은 입력창에서 온 Esc 만 발화한다", () => {
    expect(isInterruptEscape(key({ target: textarea }), ctx({ scope: 'input' }))).toBe(true)
    expect(isInterruptEscape(key({ target: body }), ctx({ scope: 'input' }))).toBe(false)
    expect(isInterruptEscape(key({ target: textarea }), ctx({ scope: 'input', textarea: null }))).toBe(false)
  })

  it('범위 상수는 칸 안 어디든이다(사용자 결정 U6)', () => {
    expect(ESC_SCOPE).toBe('slot')
  })
})
