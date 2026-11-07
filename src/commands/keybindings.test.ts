
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { __resetRegistryForTest, register } from './registry'
import { comboOf, installKeybindings, isEditableTarget, setKeybindingsPaused } from './keybindings'
import { useThemeStore } from '../store/themeStore'

// ─────────────────────────────────────────────────────────────────────────────
// ★jsdom does not implement HTMLElement.isContentEditable (always undefined)★ — real WebView2/browsers
//   return the *effective* editability per the HTML spec, but jsdom has not even a stub. The production guard (isEditableTarget)
//   treats isContentEditable as the authority (FIX-A), so if a test leaned on jsdom's broken value it would verify
//   the contenteditable contract not at all. So only in tests we plant a spec-compliant isContentEditable getter on the element (computing effective
//   editability by walking up the contentEditable attribute chain) to check the contract with real browser semantics. plaintext-only/inheritance/"false"
//   island boundaries are reflected per spec too. (Production code is not touched.)
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

describe('isEditableTarget (ADR-0055 포커스 가드)', () => {
  it('<input> 은 편집 대상 → true', () => {
    const el = document.createElement('input')
    document.body.appendChild(el)
    expect(isEditableTarget(el)).toBe(true)
  })

  it('<textarea> 는 편집 대상 → true', () => {
    const el = document.createElement('textarea')
    document.body.appendChild(el)
    expect(isEditableTarget(el)).toBe(true)
  })

  it('<select> 는 편집 대상 → true', () => {
    const el = document.createElement('select')
    document.body.appendChild(el)
    expect(isEditableTarget(el)).toBe(true)
  })

  it('contenteditable="true" 조상 안의 요소 → true', () => {
    const editable = mkCE('true')
    const inner = document.createElement('span')
    editable.appendChild(inner)
    withSpecContentEditable(inner)
    expect(isEditableTarget(inner)).toBe(true)
  })

  it('contenteditable="plaintext-only" → true (FIX-1: 가드 구멍 방지)', () => {
    const editable = mkCE('plaintext-only')
    expect(isEditableTarget(editable)).toBe(true)
  })

  it('contenteditable="plaintext-only" 조상 안의 요소 → true', () => {
    const editable = mkCE('plaintext-only')
    const inner = document.createElement('span')
    editable.appendChild(inner)
    withSpecContentEditable(inner)
    expect(isEditableTarget(inner)).toBe(true)
  })

  it('contenteditable="false" → false (명시적 비편집은 단축키 허용)', () => {
    const el = mkCE('false')
    expect(isEditableTarget(el)).toBe(false)
  })

  it('편집 조상 안의 contenteditable="false" 섬 자손 → false (FIX-A: closest 가 경계를 넘던 버그)', () => {
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

  it('.xterm(터미널) 안의 요소 → true (터미널 키를 삼키면 안 됨)', () => {
    const term = document.createElement('div')
    term.className = 'xterm'
    const row = document.createElement('div')
    term.appendChild(row)
    document.body.appendChild(term)
    expect(isEditableTarget(row)).toBe(true)
  })

  it('평범한 <div>(비편집) → false (단축키 허용)', () => {
    const el = document.createElement('div')
    document.body.appendChild(el)
    expect(isEditableTarget(el)).toBe(false)
  })

  it('null 타겟 → false (방어)', () => {
    expect(isEditableTarget(null)).toBe(false)
  })
})

describe('comboOf (키 조합 정규화)', () => {
  it('Ctrl+Shift+T → ctrl+shift+t (수식키 순서·소문자 정규화)', () => {
    const e = new KeyboardEvent('keydown', { key: 'T', ctrlKey: true, shiftKey: true })
    expect(comboOf(e)).toBe('ctrl+shift+t')
  })

  it('수식키 자체는 combo 에서 제외', () => {
    const e = new KeyboardEvent('keydown', { key: 'Control', ctrlKey: true })
    expect(comboOf(e)).toBe('ctrl')
  })

  it('수식키 없는 단일 키 → 키 이름만', () => {
    const e = new KeyboardEvent('keydown', { key: 'a' })
    expect(comboOf(e)).toBe('a')
  })
})

// ─────────────────────────────────────────────────────────────────────────────
// FIX-6: verify through the *actually installed listener*, not a pure predicate.
// ─────────────────────────────────────────────────────────────────────────────
describe('installKeybindings (설치된 리스너 배선/생명주기)', () => {
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

  it('비편집 타겟(document.body)에서 ctrl+tab → 바인딩 command 실행 + preventDefault', () => {
    const spy = vi.fn()
    // Swap the default binding (ctrl+tab → tab.next) for this test's spy command.
    register({ id: 'tab.next', title: 'next', run: spy })
    dispose = installKeybindings()

    const e = fireCtrlTab(document.body)
    expect(spy).toHaveBeenCalledTimes(1)
    expect(e.defaultPrevented).toBe(true)
  })

  it('배선이 store 액션까지 닿는다(document.body keydown)', () => {
    // ★Do not run the real adapter★ — adapters register as a module side effect, and if another test already
    // cache-imported it, it is not re-registered after `__resetRegistryForTest()` (test-order dependence). So we register directly in this
    // place and only check that the whole "key → run → store action" span connects.
    // ★themeStore is chosen as the store because it is a **store observable without Tauri**★ — the store (viewStore) of the `tab.next`
    // that this key actually calls goes through invoke, so measuring it here would measure a mock, not the wiring.
    useThemeStore.getState().setTheme('dark')
    const THEMES = ['dark', 'light', 'e-ink'] as const
    register({
      id: 'tab.next',
      title: 'next',
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

  it('타겟이 <input> 이면 command 실행 안 함(가드가 리스너에 배선됨)', () => {
    const spy = vi.fn()
    register({ id: 'tab.next', title: 'next', run: spy })
    dispose = installKeybindings()

    const input = document.createElement('input')
    document.body.appendChild(input)
    const e = fireCtrlTab(input)
    expect(spy).not.toHaveBeenCalled()
    expect(e.defaultPrevented).toBe(false)
  })

  it('타겟이 .xterm 자손이면 command 실행 안 함', () => {
    const spy = vi.fn()
    register({ id: 'tab.next', title: 'next', run: spy })
    dispose = installKeybindings()

    const term = document.createElement('div')
    term.className = 'xterm'
    const row = document.createElement('div')
    term.appendChild(row)
    document.body.appendChild(term)
    fireCtrlTab(row)
    expect(spy).not.toHaveBeenCalled()
  })

  it('타겟이 contenteditable="plaintext-only" 면 command 실행 안 함', () => {
    const spy = vi.fn()
    register({ id: 'tab.next', title: 'next', run: spy })
    dispose = installKeybindings()

    const editable = document.createElement('div')
    editable.setAttribute('contenteditable', 'plaintext-only')
    document.body.appendChild(editable)
    withSpecContentEditable(editable)
    fireCtrlTab(editable)
    expect(spy).not.toHaveBeenCalled()
  })

  // ── Stops while the restore modal is up (TRD S21-storage §6-7 N4) ──
  it('멈춘 동안에는 발화하지 않고 preventDefault 도 안 한다', () => {
    const spy = vi.fn()
    register({ id: 'tab.next', title: 'next', run: spy })
    dispose = installKeybindings()

    setKeybindingsPaused(true)
    const e = fireCtrlTab(document.body)
    expect(spy).not.toHaveBeenCalled()
    expect(e.defaultPrevented).toBe(false)
  })

  it('다시 켜면 발화한다', () => {
    const spy = vi.fn()
    register({ id: 'tab.next', title: 'next', run: spy })
    dispose = installKeybindings()

    setKeybindingsPaused(true)
    fireCtrlTab(document.body)
    setKeybindingsPaused(false)
    fireCtrlTab(document.body)
    expect(spy).toHaveBeenCalledTimes(1)
  })

  it('disposer 호출 후엔 더 이상 발화하지 않는다', () => {
    const spy = vi.fn()
    register({ id: 'tab.next', title: 'next', run: spy })
    const d = installKeybindings()
    d()
    fireCtrlTab(document.body)
    expect(spy).not.toHaveBeenCalled()
  })

  it('StrictMode 식 install→dispose→install → 정확히 1회만 발화(중복 등록 누수 없음)', () => {
    const spy = vi.fn()
    register({ id: 'tab.next', title: 'next', run: spy })
    const d1 = installKeybindings()
    d1()
    dispose = installKeybindings()
    fireCtrlTab(document.body)
    expect(spy).toHaveBeenCalledTimes(1)
  })

  it('when:()=>false 로 바인딩된 command 는 키로 발화 안 함(FIX-5)', () => {
    const spy = vi.fn()
    register({ id: 'tab.next', title: 'next', when: () => false, run: spy })
    dispose = installKeybindings()

    const e = fireCtrlTab(document.body)
    expect(spy).not.toHaveBeenCalled()
    expect(e.defaultPrevented).toBe(false)
  })

  it('when:()=>true 로 바인딩된 command 는 정상 발화(FIX-5)', () => {
    const spy = vi.fn()
    register({ id: 'tab.next', title: 'next', when: () => true, run: spy })
    dispose = installKeybindings()

    const e = fireCtrlTab(document.body)
    expect(spy).toHaveBeenCalledTimes(1)
    expect(e.defaultPrevented).toBe(true)
  })

  it('when 이 throw 하면 command 미실행 + 리스너 밖으로 안 새고 + preventDefault 안 함(FIX-B)', () => {
    const spy = vi.fn()
    register({
      id: 'tab.next',
      title: 'next',
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
