// ADR-0055: 키바인딩 포커스 가드 단위테스트 — ★load-bearing 불변식★: 입력/터미널 타이핑 중엔
//   단축키를 가로채면 안 된다. isEditableTarget 술어(순수)와 comboOf 정규화를 jsdom 으로 검증한다.

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
      // 가장 가까운 contenteditable 지정 조상을 찾아 그 값으로 실효 편집성을 판정(스펙: false 섬에서 끊김).
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

/** contenteditable 속성 지정 + 스펙 준수 isContentEditable 심기 헬퍼(jsdom 미구현 우회). */
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
    // isContentEditable 은 상속 편집 가능성을 반영하므로 inner 도 true.
    expect(isEditableTarget(inner)).toBe(true)
  })

  it('contenteditable="false" → false (명시적 비편집은 단축키 허용)', () => {
    const el = mkCE('false')
    expect(isEditableTarget(el)).toBe(false)
  })

  it('편집 조상 안의 contenteditable="false" 섬 자손 → false (FIX-A: closest 가 경계를 넘던 버그)', () => {
    // <div contenteditable="true"><button contenteditable="false"><span target></span></button></div>
    // isContentEditable 은 "false" 섬을 정확히 비편집으로 보므로, 편집 조상 밑이어도 단축키가 발화해야 한다.
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
// FIX-6: installKeybindings 배선/생명주기 E2E — 순수 술어가 아니라 *실제 설치된 리스너* 를 통해
//   document 로 keydown 을 디스패치해 (a) 가드가 리스너에 실제로 걸렸는지 (b) disposer 가 리스너를
//   떼는지 (c) install→dispose→install 이 중복 발화하지 않는지 (d) when 게이트(FIX-5)를 검증한다.
// ─────────────────────────────────────────────────────────────────────────────
describe('installKeybindings (설치된 리스너 배선/생명주기)', () => {
  let dispose: (() => void) | null = null

  beforeEach(() => {
    __resetRegistryForTest()
    // 각 테스트가 자기 command 를 등록하므로 부수효과 import(themeCommands)에 의존하지 않는다.
  })

  afterEach(() => {
    dispose?.()
    dispose = null
    document.body.innerHTML = ''
    vi.restoreAllMocks()
  })

  // ctrl+shift+t keydown 을 지정 타겟에서 document 로 버블링해 디스패치한다.
  function fireCtrlShiftT(target: EventTarget): KeyboardEvent {
    const e = new KeyboardEvent('keydown', {
      key: 'T',
      ctrlKey: true,
      shiftKey: true,
      bubbles: true,
      cancelable: true,
    })
    target.dispatchEvent(e)
    return e
  }

  it('비편집 타겟(document.body)에서 ctrl+shift+t → 바인딩 command 실행 + preventDefault', () => {
    const spy = vi.fn()
    // 기본 바인딩(ctrl+shift+t → theme.toggle)을 이 테스트용 spy command 로 갈아끼운다.
    register({ id: 'theme.toggle', title: 'toggle', run: spy })
    dispose = installKeybindings()

    const e = fireCtrlShiftT(document.body)
    expect(spy).toHaveBeenCalledTimes(1)
    expect(e.defaultPrevented).toBe(true)
  })

  it('배선이 store 액션까지 닿는다: 테마가 순환한다(document.body keydown)', () => {
    // themeCommands 는 모듈 side-effect 로 register 하는데, 그 모듈이 이미 다른 테스트에서 캐시-import
    // 됐다면 __resetRegistryForTest() 뒤 재등록되지 않는다(테스트 순서 의존 회피). 그래서 여기서는
    // 동일한 순환 로직을 명시 등록해 "키 → run → store 액션" 전 구간이 닿는지만 확인한다.
    useThemeStore.getState().setTheme('dark')
    const THEMES = ['dark', 'light', 'e-ink'] as const
    register({
      id: 'theme.toggle',
      title: 'toggle',
      run: () => {
        const cur = useThemeStore.getState().theme
        const next = THEMES[(THEMES.indexOf(cur) + 1) % THEMES.length]
        useThemeStore.getState().setTheme(next)
      },
    })
    dispose = installKeybindings()
    fireCtrlShiftT(document.body)
    expect(useThemeStore.getState().theme).toBe('light') // dark → light 순환
  })

  it('타겟이 <input> 이면 command 실행 안 함(가드가 리스너에 배선됨)', () => {
    const spy = vi.fn()
    register({ id: 'theme.toggle', title: 'toggle', run: spy })
    dispose = installKeybindings()

    const input = document.createElement('input')
    document.body.appendChild(input)
    const e = fireCtrlShiftT(input)
    expect(spy).not.toHaveBeenCalled()
    expect(e.defaultPrevented).toBe(false) // 가드 통과 → preventDefault 안 함
  })

  it('타겟이 .xterm 자손이면 command 실행 안 함', () => {
    const spy = vi.fn()
    register({ id: 'theme.toggle', title: 'toggle', run: spy })
    dispose = installKeybindings()

    const term = document.createElement('div')
    term.className = 'xterm'
    const row = document.createElement('div')
    term.appendChild(row)
    document.body.appendChild(term)
    fireCtrlShiftT(row)
    expect(spy).not.toHaveBeenCalled()
  })

  it('타겟이 contenteditable="plaintext-only" 면 command 실행 안 함', () => {
    const spy = vi.fn()
    register({ id: 'theme.toggle', title: 'toggle', run: spy })
    dispose = installKeybindings()

    const editable = document.createElement('div')
    editable.setAttribute('contenteditable', 'plaintext-only')
    document.body.appendChild(editable)
    withSpecContentEditable(editable) // jsdom 미구현 isContentEditable 을 스펙대로 심는다.
    fireCtrlShiftT(editable)
    expect(spy).not.toHaveBeenCalled()
  })

  it('disposer 호출 후엔 더 이상 발화하지 않는다', () => {
    const spy = vi.fn()
    register({ id: 'theme.toggle', title: 'toggle', run: spy })
    const d = installKeybindings()
    d() // 리스너 제거
    fireCtrlShiftT(document.body)
    expect(spy).not.toHaveBeenCalled()
  })

  it('StrictMode 식 install→dispose→install → 정확히 1회만 발화(중복 등록 누수 없음)', () => {
    const spy = vi.fn()
    register({ id: 'theme.toggle', title: 'toggle', run: spy })
    const d1 = installKeybindings()
    d1()
    dispose = installKeybindings() // 재설치(마지막만 살아있어야 함)
    fireCtrlShiftT(document.body)
    expect(spy).toHaveBeenCalledTimes(1)
  })

  it('when:()=>false 로 바인딩된 command 는 키로 발화 안 함(FIX-5)', () => {
    const spy = vi.fn()
    register({ id: 'theme.toggle', title: 'toggle', when: () => false, run: spy })
    dispose = installKeybindings()

    const e = fireCtrlShiftT(document.body)
    expect(spy).not.toHaveBeenCalled()
    expect(e.defaultPrevented).toBe(false) // when=false → 키를 삼키지 않고 통과
  })

  it('when:()=>true 로 바인딩된 command 는 정상 발화(FIX-5)', () => {
    const spy = vi.fn()
    register({ id: 'theme.toggle', title: 'toggle', when: () => true, run: spy })
    dispose = installKeybindings()

    const e = fireCtrlShiftT(document.body)
    expect(spy).toHaveBeenCalledTimes(1)
    expect(e.defaultPrevented).toBe(true)
  })

  it('when 이 throw 하면 command 미실행 + 리스너 밖으로 안 새고 + preventDefault 안 함(FIX-B)', () => {
    const spy = vi.fn()
    register({
      id: 'theme.toggle',
      title: 'toggle',
      when: () => {
        throw new Error('x')
      },
      run: spy,
    })
    dispose = installKeybindings()

    // dispatchEvent 는 리스너에서 throw 가 새어나오지 않으면 정상 반환한다(핸들러 밖 uncaught 없음).
    const e = fireCtrlShiftT(document.body)
    expect(spy).not.toHaveBeenCalled() // when=throw → false 취급 → 미실행
    expect(e.defaultPrevented).toBe(false) // 키를 삼키지 않고 통과
  })
})
