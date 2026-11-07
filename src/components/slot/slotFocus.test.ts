// ADR-0237: moves the focus held by a button in a disappearing area to the area container (tabindex -1) — does not steal focus if it was not held.

import { afterEach, describe, expect, it } from 'vitest'

import { keepFocusInSlot } from './slotFocus'

afterEach(() => {
  document.body.innerHTML = ''
})

function mount(html: string): void {
  document.body.innerHTML = html
}
const el = (id: string): HTMLElement => document.getElementById(id) as HTMLElement

describe('keepFocusInSlot', () => {
  it('버튼이 쥔 포커스를 가장 가까운 tabindex -1 조상으로 옮긴다', () => {
    mount(`
      <div id="root" tabindex="-1">
        <div id="viewport" tabindex="-1"><div><button id="b">b</button></div></div>
      </div>`)
    el('b').focus()
    keepFocusInSlot(el('b'))
    expect(document.activeElement).toBe(el('viewport'))
  })

  it('버튼이 포커스를 쥐지 않았으면 옮기지 않는다 — 다른 칸의 포커스를 빼앗지 않는다', () => {
    mount(`
      <textarea id="other"></textarea>
      <div id="root" tabindex="-1"><button id="b">b</button></div>`)
    el('other').focus()
    keepFocusInSlot(el('b'))
    expect(document.activeElement).toBe(el('other'))
  })

  it('받을 조상이 없으면 아무것도 하지 않는다', () => {
    mount(`<div tabindex="0"><button id="b">b</button></div>`)
    el('b').focus()
    keepFocusInSlot(el('b'))
    expect(document.activeElement).toBe(el('b'))
  })
})
