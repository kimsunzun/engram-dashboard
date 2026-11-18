// ADR-0237: Esc interrupt predicate table — if any one condition is false it does not fire, and it fires only when all are true (TRD §3-2).

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
  it('if all conditions are true it fires', () => {
    expect(isInterruptEscape(key(), ctx())).toBe(true)
  })

  it.each<[string, Partial<InterruptKeyEvent>]>([
    ['another key', { key: 'Enter' }],
    ['ctrl', { ctrlKey: true }],
    ['alt', { altKey: true }],
    ['shift', { shiftKey: true }],
    ['meta', { metaKey: true }],
    ['held repeat', { repeat: true }],
    ['during IME composition (isComposing)', { nativeEvent: { isComposing: true } }],
    ['during IME composition (keyCode 229)', { keyCode: 229 }],
    ['an earlier handler consumed it (defaultPrevented)', { defaultPrevented: true }],
  ])('does not fire if even one key condition is off — %s', (_label, over) => {
    expect(isInterruptEscape(key(over), ctx())).toBe(false)
  })

  it.each<[string, Partial<InterruptKeyContext>]>([
    ['an overlay is open', { overlayOpen: true }],
    ['no turn running', { streaming: false }],
    ['interrupt already sent and waiting for the turn end (ADR-0244)', { interrupting: true }],
    ['the agent is absent now', { agentUnavailable: true }],
    ['the channel does not support interrupt', { canInterrupt: false }],
  ])('does not fire if even one area condition is off — %s', (_label, over) => {
    expect(isInterruptEscape(key(), ctx(over))).toBe(false)
  })

  it("scope 'slot' fires for an Esc from outside the input box (conversation body) too", () => {
    expect(isInterruptEscape(key({ target: body }), ctx({ scope: 'slot' }))).toBe(true)
  })

  it("scope 'input' fires only for an Esc from the input box", () => {
    expect(isInterruptEscape(key({ target: textarea }), ctx({ scope: 'input' }))).toBe(true)
    expect(isInterruptEscape(key({ target: body }), ctx({ scope: 'input' }))).toBe(false)
    expect(isInterruptEscape(key({ target: textarea }), ctx({ scope: 'input', textarea: null }))).toBe(false)
  })

  it('the scope constant is anywhere in the area (user decision U6)', () => {
    expect(ESC_SCOPE).toBe('slot')
  })
})
