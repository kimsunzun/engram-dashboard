
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { __resetRegistryForTest, register } from './registry'
import { comboOf, installKeybindings, isEditableTarget, setKeybindingsPaused } from './keybindings'
import { useThemeStore } from '../store/themeStore'

// ─────────────────────────────────────────────────────────────────────────────
// ★jsdom does not implement HTMLElement.isContentEditable (always undefined)★ — browsers return the *effective* editability per the HTML spec; the production guard (isEditableTarget) treats isContentEditable as the authority (FIX-A), so leaning on jsdom's value would verify nothing of the contenteditable contract. Tests plant a spec-compliant getter (effective editability by walking up the contentEditable chain; plaintext-only/inheritance/"false" island boundaries per spec). Production code is untouched.
function withSpecContentEditable(el: HTMLElement): HTMLElement {
  Object.defineProperty(el, 'isContentEditable', {
    configurable: true,
    get(this: HTMLElement): boolean {
      let node: HTMLElement | null = this
      while (node) {
        const v = node.getAttribute?.('contenteditable')
        if (v != null) {
          const lc = v.toLowerCase()
          return lc === '' || lc === 'true' || lc === 'plaintext-only'
        }
        node = node.parentElement
      }
      return false
    },
  })
  return el
}

function mkCE(ce: string): HTMLElement {
  const el = document.createElement('div')
  el.setAttribute('contenteditable', ce)
  document.body.appendChild(el)
  return withSpecContentEditable(el)
}

afterEach(() => {
  document.body.innerHTML = ''
})

describe('isEditableTarget (ADR-0055 focus guard)', () => {
  it('<input> is an edit target → true', () => {
    const el = document.createElement('input')
    document.body.appendChild(el)
    expect(isEditableTarget(el)).toBe(true)
  })

  it('<textarea> is an edit target → true', () => {
    const el = document.createElement('textarea')
    document.body.appendChild(el)
    expect(isEditableTarget(el)).toBe(true)
  })

  it('<select> is an edit target → true', () => {
    const el = document.createElement('select')
    document.body.appendChild(el)
    expect(isEditableTarget(el)).toBe(true)
  })

  it('an element inside a contenteditable="true" ancestor → true', () => {
    const editable = mkCE('true')
    const inner = document.createElement('span')
    editable.appendChild(inner)
    withSpecContentEditable(inner)
    expect(isEditableTarget(inner)).toBe(true)
  })

  it('contenteditable="plaintext-only" → true (FIX-1: prevents a guard hole)', () => {
    const editable = mkCE('plaintext-only')
    expect(isEditableTarget(editable)).toBe(true)
  })

  it('an element inside a contenteditable="plaintext-only" ancestor → true', () => {
    const editable = mkCE('plaintext-only')
    const inner = document.createElement('span')
    editable.appendChild(inner)
    withSpecContentEditable(inner)
    expect(isEditableTarget(inner)).toBe(true)
  })

  it('contenteditable="false" → false (explicit non-edit allows shortcuts)', () => {
    const el = mkCE('false')
    expect(isEditableTarget(el)).toBe(false)
  })

  it('a contenteditable="false" island descendant inside an edit ancestor → false (FIX-A: the bug where closest crossed the boundary)', () => {
    const editable = mkCE('true')
    const island = document.createElement('button')
    island.setAttribute('contenteditable', 'false')
    const inner = document.createElement('span')
    island.appendChild(inner)
    editable.appendChild(island)
    withSpecContentEditable(island)
    withSpecContentEditable(inner)
    expect(isEditableTarget(inner)).toBe(false)
  })

  it('an element inside .xterm (terminal) → true (must not swallow terminal keys)', () => {
    const term = document.createElement('div')
    term.className = 'xterm'
    const row = document.createElement('div')
    term.appendChild(row)
    document.body.appendChild(term)
    expect(isEditableTarget(row)).toBe(true)
  })

  it('a plain <div> (non-edit) → false (shortcuts allowed)', () => {
    const el = document.createElement('div')
    document.body.appendChild(el)
    expect(isEditableTarget(el)).toBe(false)
  })

  it('null target → false (defence)', () => {
    expect(isEditableTarget(null)).toBe(false)
  })
})

describe('comboOf (key combination normalization)', () => {
  it('Ctrl+Shift+T → ctrl+shift+t (modifier order · lowercase normalization)', () => {
    const e = new KeyboardEvent('keydown', { key: 'T', ctrlKey: true, shiftKey: true })
    expect(comboOf(e)).toBe('ctrl+shift+t')
  })

  it('modifier keys themselves are excluded from the combo', () => {
    const e = new KeyboardEvent('keydown', { key: 'Control', ctrlKey: true })
    expect(comboOf(e)).toBe('ctrl')
  })

  it('a single key without modifiers → the key name only', () => {
    const e = new KeyboardEvent('keydown', { key: 'a' })
    expect(comboOf(e)).toBe('a')
  })
})

// ─────────────────────────────────────────────────────────────────────────────
// FIX-6: verify through the *actually installed listener*, not a pure predicate.
// ─────────────────────────────────────────────────────────────────────────────
describe('installKeybindings (installed listener wiring/lifecycle)', () => {
  let dispose: (() => void) | null = null

  beforeEach(() => {
    __resetRegistryForTest()
    // Each test registers its own command, so it depends on the side-effect import of no adapter module.
  })

  afterEach(() => {
    dispose?.()
    dispose = null
    setKeybindingsPaused(false)
    document.body.innerHTML = ''
    vi.restoreAllMocks()
  })

  function fireCtrlTab(target: EventTarget): KeyboardEvent {
    const e = new KeyboardEvent('keydown', {
      key: 'Tab',
      ctrlKey: true,
      bubbles: true,
      cancelable: true,
    })
    target.dispatchEvent(e)
    return e
  }

  it('ctrl+tab on a non-edit target (document.body) → runs the bound command + preventDefault', () => {
    const spy = vi.fn()
    // Swap the default binding (ctrl+tab → tab.next) for this test's spy command.
    register({ id: 'tab.next', title: () => 'next', run: spy })
    dispose = installKeybindings()

    const e = fireCtrlTab(document.body)
    expect(spy).toHaveBeenCalledTimes(1)
    expect(e.defaultPrevented).toBe(true)
  })

  it('the wiring reaches the store action (document.body keydown)', () => {
    // ★Do not run the real adapter★ — adapters register as a module side effect and are not re-registered after `__resetRegistryForTest()` if another test already cache-imported them (test-order dependence); register directly here and check only that "key → run → store action" connects.
    // ★themeStore is chosen because it is a **store observable without Tauri**★ — `tab.next`'s store (viewStore) goes through invoke, so measuring it would measure a mock, not the wiring.
    useThemeStore.getState().setTheme('dark')
    const THEMES = ['dark', 'light', 'e-ink'] as const
    register({
      id: 'tab.next',
      title: () => 'next',
      run: () => {
        const cur = useThemeStore.getState().theme
        const next = THEMES[(THEMES.indexOf(cur) + 1) % THEMES.length]
        useThemeStore.getState().setTheme(next)
      },
    })
    dispose = installKeybindings()
    fireCtrlTab(document.body)
    expect(useThemeStore.getState().theme).toBe('light')
  })

  it('does not run the command when the target is an <input> (the guard is wired into the listener)', () => {
    const spy = vi.fn()
    register({ id: 'tab.next', title: () => 'next', run: spy })
    dispose = installKeybindings()

    const input = document.createElement('input')
    document.body.appendChild(input)
    const e = fireCtrlTab(input)
    expect(spy).not.toHaveBeenCalled()
    expect(e.defaultPrevented).toBe(false)
  })

  it('does not run the command when the target is a .xterm descendant', () => {
    const spy = vi.fn()
    register({ id: 'tab.next', title: () => 'next', run: spy })
    dispose = installKeybindings()

    const term = document.createElement('div')
    term.className = 'xterm'
    const row = document.createElement('div')
    term.appendChild(row)
    document.body.appendChild(term)
    fireCtrlTab(row)
    expect(spy).not.toHaveBeenCalled()
  })

  it('does not run the command when the target is contenteditable="plaintext-only"', () => {
    const spy = vi.fn()
    register({ id: 'tab.next', title: () => 'next', run: spy })
    dispose = installKeybindings()

    const editable = document.createElement('div')
    editable.setAttribute('contenteditable', 'plaintext-only')
    document.body.appendChild(editable)
    withSpecContentEditable(editable)
    fireCtrlTab(editable)
    expect(spy).not.toHaveBeenCalled()
  })

  // ── Stops while the restore modal is up (TRD S21-storage §6-7 N4) ──
  it('does not fire while paused and does not preventDefault either', () => {
    const spy = vi.fn()
    register({ id: 'tab.next', title: () => 'next', run: spy })
    dispose = installKeybindings()

    setKeybindingsPaused(true)
    const e = fireCtrlTab(document.body)
    expect(spy).not.toHaveBeenCalled()
    expect(e.defaultPrevented).toBe(false)
  })

  it('fires again when turned back on', () => {
    const spy = vi.fn()
    register({ id: 'tab.next', title: () => 'next', run: spy })
    dispose = installKeybindings()

    setKeybindingsPaused(true)
    fireCtrlTab(document.body)
    setKeybindingsPaused(false)
    fireCtrlTab(document.body)
    expect(spy).toHaveBeenCalledTimes(1)
  })

  it('no longer fires after the disposer is called', () => {
    const spy = vi.fn()
    register({ id: 'tab.next', title: () => 'next', run: spy })
    const d = installKeybindings()
    d()
    fireCtrlTab(document.body)
    expect(spy).not.toHaveBeenCalled()
  })

  it('StrictMode-style install→dispose→install → fires exactly once (no duplicate registration leak)', () => {
    const spy = vi.fn()
    register({ id: 'tab.next', title: () => 'next', run: spy })
    const d1 = installKeybindings()
    d1()
    dispose = installKeybindings()
    fireCtrlTab(document.body)
    expect(spy).toHaveBeenCalledTimes(1)
  })

  it('a command bound with when:()=>false does not fire by key (FIX-5)', () => {
    const spy = vi.fn()
    register({ id: 'tab.next', title: () => 'next', when: () => false, run: spy })
    dispose = installKeybindings()

    const e = fireCtrlTab(document.body)
    expect(spy).not.toHaveBeenCalled()
    expect(e.defaultPrevented).toBe(false)
  })

  it('a command bound with when:()=>true fires normally (FIX-5)', () => {
    const spy = vi.fn()
    register({ id: 'tab.next', title: () => 'next', when: () => true, run: spy })
    dispose = installKeybindings()

    const e = fireCtrlTab(document.body)
    expect(spy).toHaveBeenCalledTimes(1)
    expect(e.defaultPrevented).toBe(true)
  })

  it('when `when` throws, the command does not run + it does not leak out of the listener + no preventDefault (FIX-B)', () => {
    const spy = vi.fn()
    register({
      id: 'tab.next',
      title: () => 'next',
      when: () => {
        throw new Error('x')
      },
      run: spy,
    })
    dispose = installKeybindings()

    // dispatchEvent returns normally if no throw leaks out of the listener (no uncaught outside the handler).
    const e = fireCtrlTab(document.body)
    expect(spy).not.toHaveBeenCalled()
    expect(e.defaultPrevented).toBe(false)
  })
})
