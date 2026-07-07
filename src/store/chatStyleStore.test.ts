// ADR-0051: chatStyleStore 단위테스트 — 채팅 스타일 control surface(간격·폰트)의 권위·영속·CSS 적용을
//   검증한다. 순수 로직 + jsdom(localStorage/document 존재). 사람 UI·LLM 공통 진입점(setValue/patch/reset).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import {
  CHAT_STYLE_DEFAULTS,
  loadChatStyle,
  useChatStyleStore,
  type ChatStyleKey,
  type ChatStyleValues,
} from './chatStyleStore'

// FIX-3: theme.css 원문을 런타임에 읽는다(vitest=Node). 프론트 tsconfig 는 DOM 전용(@types/node 없음)이고
//   vitest 는 .css 를 빈 모듈로 처리해 `?raw`/glob 이 빈 문자열이라, Node fs 로 직접 읽는다. 여기 필요한
//   Node 심볼만 최소 ambient 선언(전역 @types/node 의존 회피 — 이 테스트 파일 스코프 한정).
declare function require(id: string): { readFileSync(p: string, enc: string): string }
declare const process: { cwd(): string }

const STORAGE_KEY = 'engram.chatStyle'

function rootVar(name: string): string {
  return document.documentElement.style.getPropertyValue(name).trim()
}

beforeEach(() => {
  localStorage.clear()
  // store 를 기본값으로 리셋(zustand 싱글톤이라 테스트 간 격리). setState 로 직접 초기화.
  useChatStyleStore.setState({ values: { ...CHAT_STYLE_DEFAULTS } })
  // :root 인라인 스타일 정리(이전 테스트 잔류 방지).
  document.documentElement.removeAttribute('style')
})

afterEach(() => {
  localStorage.clear()
})

describe('chatStyleStore (ADR-0051)', () => {
  it('loadChatStyle: 저장값 부재 → 기본값 fallback', () => {
    expect(loadChatStyle()).toEqual(CHAT_STYLE_DEFAULTS)
  })

  it('loadChatStyle: 손상된 JSON → 기본값 fallback(throw 없음)', () => {
    localStorage.setItem(STORAGE_KEY, '{not valid json')
    expect(loadChatStyle()).toEqual(CHAT_STYLE_DEFAULTS)
  })

  it('loadChatStyle: 부분 저장값은 기본값 위에 병합(누락 키는 기본값 유지)', () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ fontSize: '15px' }))
    const loaded = loadChatStyle()
    expect(loaded.fontSize).toBe('15px')
    expect(loaded.railRowPt).toBe(CHAT_STYLE_DEFAULTS.railRowPt) // 누락 키 = 기본값
  })

  it('loadChatStyle: 문자열이 아닌 값은 무시하고 기본값 유지(신뢰 못할 저장값 방어)', () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ fontSize: 42, lineHeight: '1.7' }))
    const loaded = loadChatStyle()
    expect(loaded.fontSize).toBe(CHAT_STYLE_DEFAULTS.fontSize) // number → 무시
    expect(loaded.lineHeight).toBe('1.7') // string → 채택
  })

  it('init: localStorage 로드 → CSS 변수 적용', () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ railRowPt: '2rem' }))
    useChatStyleStore.getState().init()
    expect(useChatStyleStore.getState().values.railRowPt).toBe('2rem')
    expect(rootVar('--chat-rail-row-pt')).toBe('2rem')
    // 누락 키도 기본값으로 :root 에 적용된다.
    expect(rootVar('--chat-font-size')).toBe(CHAT_STYLE_DEFAULTS.fontSize)
  })

  it('setValue: 값 갱신 → store + CSS 변수 + localStorage 3자 반영', () => {
    useChatStyleStore.getState().setValue('fontSize', '16px')
    expect(useChatStyleStore.getState().values.fontSize).toBe('16px')
    expect(rootVar('--chat-font-size')).toBe('16px')
    const saved = JSON.parse(localStorage.getItem(STORAGE_KEY)!)
    expect(saved.fontSize).toBe('16px')
  })

  it('영속 round-trip: setValue 후 새 로드(loadChatStyle)가 그 값을 복원한다', () => {
    useChatStyleStore.getState().setValue('lineHeight', '1.8')
    // 새로고침 시뮬레이션 — localStorage 에서 다시 로드.
    const reloaded = loadChatStyle()
    expect(reloaded.lineHeight).toBe('1.8')
  })

  it('patch: 여러 키 부분 갱신(다른 키는 유지)', () => {
    const before = useChatStyleStore.getState().values.railGutter
    useChatStyleStore.getState().patch({ fontSize: '14px', lineHeight: '1.6' })
    const v = useChatStyleStore.getState().values
    expect(v.fontSize).toBe('14px')
    expect(v.lineHeight).toBe('1.6')
    expect(v.railGutter).toBe(before) // 안 건드린 키 유지
    expect(rootVar('--chat-font-size')).toBe('14px')
    expect(rootVar('--chat-line-height')).toBe('1.6')
  })

  it('reset: 기본값으로 복귀 + CSS/localStorage 갱신', () => {
    useChatStyleStore.getState().setValue('fontSize', '20px')
    useChatStyleStore.getState().reset()
    expect(useChatStyleStore.getState().values).toEqual(CHAT_STYLE_DEFAULTS)
    expect(rootVar('--chat-font-size')).toBe(CHAT_STYLE_DEFAULTS.fontSize)
    const saved = JSON.parse(localStorage.getItem(STORAGE_KEY)!)
    expect(saved.fontSize).toBe(CHAT_STYLE_DEFAULTS.fontSize)
  })
})

// ── FIX-2: 런타임 키 화이트리스트(control surface 는 무신뢰 경계) ─────────────────────────
describe('chatStyleStore runtime key whitelist (ADR-0051 FIX-2)', () => {
  it('patch({ bogus }): 낯선 키는 store·localStorage 를 오염시키지 않고 bogus 로 setProperty 하지 않는다', () => {
    const spy = vi.spyOn(document.documentElement.style, 'setProperty')
    // 캐스트로 TS 를 우회(런타임 외부 호출 = LLM/CDP 재현). 알려진 키 하나 + 낯선 키 하나를 섞는다.
    useChatStyleStore
      .getState()
      .patch({ fontSize: '17px', bogus: 'x' } as unknown as Partial<ChatStyleValues>)

    // 알려진 키는 반영, 낯선 키는 store 에 없다.
    const v = useChatStyleStore.getState().values as Record<string, unknown>
    expect(v.fontSize).toBe('17px')
    expect('bogus' in v).toBe(false)

    // localStorage 에도 낯선 키가 없다.
    const saved = JSON.parse(localStorage.getItem(STORAGE_KEY)!)
    expect('bogus' in saved).toBe(false)
    expect(saved.fontSize).toBe('17px')

    // setProperty 는 'bogus' 이름(또는 --bogus)으로 절대 호출되지 않는다.
    const propertyNames = spy.mock.calls.map(c => String(c[0]))
    expect(propertyNames.some(n => n.includes('bogus'))).toBe(false)
    spy.mockRestore()
  })

  it('setValue(낯선 키): store·localStorage 무변경(no-op)', () => {
    const before = { ...useChatStyleStore.getState().values }
    // 캐스트로 TS 우회(런타임 외부 호출 재현).
    const setValue = useChatStyleStore.getState().setValue as (k: string, v: string) => void
    setValue('nope', 'y')
    expect(useChatStyleStore.getState().values).toEqual(before)
    // 저장 자체가 일어나지 않아야 한다(키 오염 없음).
    const raw = localStorage.getItem(STORAGE_KEY)
    if (raw) expect('nope' in JSON.parse(raw)).toBe(false)
  })
})

// ── ADR-0051: 프로토타입 키 화이트리스트 우회 방어(isChatStyleKey own-key 판정) ──────────────
//   `key in CHAT_STYLE_DEFAULTS` 는 프로토타입 체인을 타서 constructor·__proto__·toString 등
//   Object.prototype 상속 키가 화이트리스트를 통과했다. LLM/CDP 등 런타임 외부 호출이 이 이름들을
//   set/patch 하면 store·localStorage 가 오염되므로, 고정 10키만 통과하는지 검증한다.
describe('chatStyleStore prototype-key bypass (ADR-0051)', () => {
  const POLLUTING_KEYS = ['__proto__', 'constructor', 'toString', 'valueOf', 'hasOwnProperty']

  it('setValue: 프로토타입 상속 키(__proto__/constructor)는 store·localStorage 에 못 들어간다', () => {
    const before = { ...useChatStyleStore.getState().values }
    const objProtoBefore = Object.getPrototypeOf({})
    // 캐스트로 TS 우회(런타임 외부 호출 = LLM/CDP 재현).
    const setValue = useChatStyleStore.getState().setValue as (k: string, v: string) => void

    setValue('__proto__', '1rem')
    setValue('constructor', 'x')
    setValue('toString', 'y')

    // store values 에 오염 키가 own-key 로 들어가지 않는다.
    const v = useChatStyleStore.getState().values as Record<string, unknown>
    for (const k of POLLUTING_KEYS) {
      expect(Object.prototype.hasOwnProperty.call(v, k)).toBe(false)
    }
    expect(useChatStyleStore.getState().values).toEqual(before)

    // 저장 자체가 일어나지 않는다(모두 no-op → localStorage 미기록).
    expect(localStorage.getItem(STORAGE_KEY)).toBeNull()

    // Object.prototype 이 오염되지 않았다.
    expect(Object.getPrototypeOf({})).toBe(objProtoBefore)
    expect(({} as Record<string, unknown>).toString).toBe(Object.prototype.toString)
  })

  it('patch({ __proto__, constructor, toString }): 프로토타입 키 전부 거른다(오염·prototype 변형 없음)', () => {
    const before = { ...useChatStyleStore.getState().values }
    const objProtoBefore = Object.getPrototypeOf({})

    useChatStyleStore
      .getState()
      .patch({ __proto__: 'a', constructor: 'b', toString: 'c' } as unknown as Partial<ChatStyleValues>)

    const v = useChatStyleStore.getState().values as Record<string, unknown>
    for (const k of POLLUTING_KEYS) {
      expect(Object.prototype.hasOwnProperty.call(v, k)).toBe(false)
    }
    expect(useChatStyleStore.getState().values).toEqual(before)
    // patch 는 걸러낸 뒤에도 next(=기존 값)를 persist 한다 → 저장 JSON 에 오염 키가 없기만 하면 된다.
    const saved = JSON.parse(localStorage.getItem(STORAGE_KEY)!)
    for (const k of POLLUTING_KEYS) {
      expect(Object.prototype.hasOwnProperty.call(saved, k)).toBe(false)
    }

    // Object.prototype·글로벌 프로토타입 오염 없음.
    expect(Object.getPrototypeOf({})).toBe(objProtoBefore)
  })

  it('patch({ fontSize, __proto__ }): 유효 키는 적용, 프로토타입 키는 드롭', () => {
    useChatStyleStore
      .getState()
      .patch({ fontSize: '15px', __proto__: 'x' } as unknown as Partial<ChatStyleValues>)

    const v = useChatStyleStore.getState().values as Record<string, unknown>
    expect(v.fontSize).toBe('15px') // 유효 키 적용
    expect(Object.prototype.hasOwnProperty.call(v, '__proto__')).toBe(false)
    expect(rootVar('--chat-font-size')).toBe('15px')

    // localStorage 에 유효 키만 반영, 오염 키 없음.
    const saved = JSON.parse(localStorage.getItem(STORAGE_KEY)!)
    expect(saved.fontSize).toBe('15px')
    expect(Object.prototype.hasOwnProperty.call(saved, '__proto__')).toBe(false)
  })
})

// ── FIX-3: 이중 출처 기본값 drift 감지(theme.css :root ↔ CHAT_STYLE_DEFAULTS) ─────────────
//   기본값이 두 곳에 산다(store + theme.css :root fallback). 한쪽만 바뀌면 부팅 첫 프레임과 store 적용이
//   어긋난다. 하나에서 다른 하나를 유도하지 않고(각자 정본), 둘이 같은지만 싸게 검증해 조용한 drift 를 잡는다.
describe('theme.css ↔ CHAT_STYLE_DEFAULTS drift (ADR-0051 FIX-3)', () => {
  // store 키 ↔ theme.css :root 변수명 매핑(chatStyleStore.ts CSS_VAR_BY_KEY 와 짝 — 여기 재선언해 독립 검증).
  const CSS_VAR_BY_KEY: Record<ChatStyleKey, string> = {
    railRowPt: '--chat-rail-row-pt',
    plainRowPt: '--chat-plain-row-pt',
    userPy: '--chat-user-py',
    userPx: '--chat-user-px',
    userMy: '--chat-user-my',
    railGutter: '--chat-rail-gutter',
    railLineOffset: '--chat-rail-line-offset',
    railDotTop: '--chat-rail-dot-top',
    fontSize: '--chat-font-size',
    lineHeight: '--chat-line-height',
  }

  it('theme.css :root 의 10개 chat 변수 기본값이 CHAT_STYLE_DEFAULTS 와 일치한다', () => {
    // vitest 는 프로젝트 루트에서 실행 → cwd 기준 상대 경로로 theme.css 원문을 읽는다.
    const css = require('node:fs').readFileSync(`${process.cwd()}/src/styles/theme.css`, 'utf8')

    for (const key of Object.keys(CHAT_STYLE_DEFAULTS) as ChatStyleKey[]) {
      const varName = CSS_VAR_BY_KEY[key]
      // `--chat-xxx: <value>;` 선언에서 값만 추출(주석·공백 무시). 변수명은 리터럴로 escape.
      const escaped = varName.replace(/[-]/g, '\\-')
      const m = css.match(new RegExp(`${escaped}\\s*:\\s*([^;]+);`))
      expect(m, `theme.css 에 ${varName} 선언이 없다`).not.toBeNull()
      const cssValue = m![1].trim()
      expect(cssValue, `${varName}(=${key}) 기본값이 theme.css 와 store 에서 어긋남`).toBe(
        CHAT_STYLE_DEFAULTS[key],
      )
    }
  })
})
