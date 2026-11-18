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
  it('moves the focus the button held to the nearest tabindex -1 ancestor', () => {
    mount(`
      <div id="root" tabindex="-1">
        <div id="viewport" tabindex="-1"><div><button id="b">b</button></div></div>
      </div>`)
    el('b').focus()
    keepFocusInSlot(el('b'))
    expect(document.activeElement).toBe(el('viewport'))
  })

  it('does not move the focus if the button did not hold focus', () => {
    mount(`
      <textarea id="other"></textarea>
      <div id="root" tabindex="-1"><button id="b">b</button></div>`)
    el('other').focus()
    keepFocusInSlot(el('b'))
    expect(document.activeElement).toBe(el('other'))
  })

  it('if there is no ancestor to receive it does nothing', () => {
    mount(`<div tabindex="0"><button id="b">b</button></div>`)
    el('b').focus()
    keepFocusInSlot(el('b'))
    expect(document.activeElement).toBe(el('b'))
  })
})
