
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { __resetRegistryForTest, register } from './registry'
import { comboOf, installKeybindings, isEditableTarget } from './keybindings'
import { useThemeStore } from '../store/themeStore'

// ─────────────────────────────────────────────────────────────────────────────
// ★jsdom 은 HTMLElement.isContentEditable 을 구현하지 않는다(항상 undefined)★ — 실제 WebView2/브라우저는
//   HTML 스펙대로 *실효* 편집 가능성을 돌려주지만 jsdom 은 스텁조차 없다. 프로덕션 가드(isEditableTarget)는
//   isContentEditable 을 권위로 삼으므로(FIX-A), 테스트가 jsdom 의 깨진 값에 기대면 contenteditable 계약을
//   전혀 검증하지 못한다. 그래서 테스트에서만 스펙 준수 isContentEditable getter 를 요소에 심어(contentEditable
//   속성 체인을 걸어올라 실효 편집성 계산) 실제 브라우저 시맨틱으로 계약을 확인한다. plaintext-only/상속/"false"
//   섬 경계까지 스펙대로 반영된다. (프로덕션 코드는 건드리지 않는다.)
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
// FIX-6: 순수 술어가 아니라 *실제 설치된 리스너* 를 통해 검증한다.
// ─────────────────────────────────────────────────────────────────────────────
describe('installKeybindings (설치된 리스너 배선/생명주기)', () => {
  let dispose: (() => void) | null = null

  beforeEach(() => {
    __resetRegistryForTest()
    // 각 테스트가 자기 command 를 등록하므로 어느 어댑터 모듈의 부수효과 import 에도 의존하지 않는다.
  })

  afterEach(() => {
    dispose?.()
    dispose = null
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
    // 기본 바인딩(ctrl+tab → tab.next)을 이 테스트용 spy command 로 갈아끼운다.
    register({ id: 'tab.next', title: 'next', run: spy })
    dispose = installKeybindings()

    const e = fireCtrlTab(document.body)
    expect(spy).toHaveBeenCalledTimes(1)
    expect(e.defaultPrevented).toBe(true)
  })

  it('배선이 store 액션까지 닿는다(document.body keydown)', () => {
    // ★진짜 어댑터를 안 태운다★ — 어댑터는 모듈 side-effect 로 register 하는데, 다른 테스트가 이미
    // 캐시-import 했으면 `__resetRegistryForTest()` 뒤 재등록되지 않는다(테스트 순서 의존). 그래서 이
    // 자리에 직접 등록해 "키 → run → store 액션" 전 구간이 닿는지만 본다.
    // ★store 로 themeStore 를 고른 것은 그것이 **Tauri 없이 관측되는 store** 라서다★ — 이 키가 실제로
    // 부르는 `tab.next` 의 store(viewStore)는 invoke 를 타므로 여기서 재려면 배선 아닌 mock 을 재게 된다.
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

    // dispatchEvent 는 리스너에서 throw 가 새어나오지 않으면 정상 반환한다(핸들러 밖 uncaught 없음).
    const e = fireCtrlTab(document.body)
    expect(spy).not.toHaveBeenCalled()
    expect(e.defaultPrevented).toBe(false)
  })
})
