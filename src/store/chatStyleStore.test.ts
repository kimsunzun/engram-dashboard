// chatStyleStore — whether the shell setting `chat.style.*` reaches the :root CSS variables · whether anything outside the whitelist is blocked · whether it does not
//   touch storage (ADR-0265). The settings client plugs a fake IPC into the real one.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { createSettingsClient, type SettingsClient, type SettingsIpc } from '../api/settingsClient'
import {
  installChatStyleApplier,
  resetChatStyle,
  setChatStyle,
  type ChatStyleKey,
} from './chatStyleStore'

// Reads the theme.css source at runtime (vitest=Node). The frontend tsconfig is DOM-only (no @types/node) and
//   vitest treats .css as an empty module so `?raw`/glob is an empty string, so we read it directly with Node fs. Only the Node symbols needed here are declared as a minimal ambient
//   (avoiding a dependency on global @types/node — scoped to this test file only).
declare function require(id: string): { readFileSync(p: string, enc: string): string }
declare const process: { cwd(): string }

function rootVar(name: string): string {
  return document.documentElement.style.getPropertyValue(name).trim()
}

/** A client where the test can push one notice directly — the pull answer is empty. */
function harness() {
  let handler: ((payload: unknown) => void) | undefined
  let rev = 0
  const invoke = vi.fn(async (cmd: string, args?: Record<string, unknown>): Promise<unknown> => {
    if (cmd === 'settings_get') return { rev, items: [] }
    if (cmd === 'settings_set') return { rev: ++rev, key: args?.key, value: args?.value, changed: true }
    if (cmd === 'settings_reset') return { rev, reset: [args?.key], changed: [] }
    throw new Error(`예상 밖 명령 ${cmd}`)
  })
  const ipc: SettingsIpc = {
    async listen(_event, h) {
      handler = h
      return () => {}
    },
    invoke: <T,>(cmd: string, args?: Record<string, unknown>) => invoke(cmd, args) as Promise<T>,
  }
  const client: SettingsClient = createSettingsClient(ipc)
  return {
    client,
    invoke,
    push(items: Array<[string, string]>) {
      if (!handler) throw new Error('리스너가 아직 없다')
      rev += 1
      handler({ rev, items: items.map(([key, value]) => ({ key, value, is_default: false })) })
    },
  }
}

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i++) await Promise.resolve()
}

let disposers: Array<() => void> = []

beforeEach(() => {
  document.documentElement.removeAttribute('style')
  disposers = []
})

afterEach(() => {
  for (const d of disposers) d()
  vi.restoreAllMocks()
})

async function installed() {
  const h = harness()
  disposers.push(installChatStyleApplier(h.client), h.client.install())
  await flush()
  return h
}

describe('chatStyleStore — 적용자', () => {
  it('chat.style.* 항목은 그 CSS 변수에 칠해진다', async () => {
    const h = await installed()
    h.push([
      ['chat.style.fontSize', '16px'],
      ['chat.style.railLineOffset', '-1rem'],
    ])
    expect(rootVar('--chat-font-size')).toBe('16px')
    expect(rootVar('--chat-rail-line-offset')).toBe('-1rem')
  })

  it('설치 전에 받은 값도 설치하자마자 칠한다', async () => {
    const h = harness()
    disposers.push(h.client.install())
    await flush()
    h.push([['chat.style.lineHeight', '1.8']])
    expect(rootVar('--chat-line-height')).toBe('')

    disposers.push(installChatStyleApplier(h.client))
    expect(rootVar('--chat-line-height')).toBe('1.8')
  })

  // ★Do not measure by whether the style attribute is empty★ — if a prototype key passes, a function · object goes in the variable name slot and
  //   setProperty quietly leaves nothing, so even if the guard is weakened to `in` that assertion stays green. Count the calls themselves.
  it('화이트리스트 밖 키 · 챗 스타일이 아닌 키 · 프로토타입 키는 setProperty 를 한 번도 부르지 않는다', async () => {
    const h = await installed()
    const setProperty = vi.spyOn(document.documentElement.style, 'setProperty')
    h.push([
      ['chat.style.bogus', '1px'],
      ['chat.style.constructor', '2px'],
      ['chat.style.__proto__', '3px'],
      ['chat.style.toString', '4px'],
      ['chat.style.hasOwnProperty', '5px'],
      ['theme.default', 'light'],
      ['fontSize', '6px'],
    ])
    expect(setProperty).not.toHaveBeenCalled()
  })

  it('구독을 풀면 더는 칠하지 않는다', async () => {
    const h = harness()
    const off = installChatStyleApplier(h.client)
    disposers.push(h.client.install())
    await flush()
    off()
    h.push([['chat.style.fontSize', '20px']])
    expect(rootVar('--chat-font-size')).toBe('')
  })
})

describe('chatStyleStore — 쓰기는 셸 설정으로', () => {
  it('setChatStyle 은 settings_set chat.style.<k> 로 간다', async () => {
    const h = harness()
    await setChatStyle('fontSize', '15px', h.client)
    expect(h.invoke).toHaveBeenCalledWith('settings_set', { key: 'chat.style.fontSize', value: '15px' })
  })

  it('resetChatStyle 은 한 키 또는 chat.style. 접두 전부를 settings_reset 으로 보낸다', async () => {
    const h = harness()
    await resetChatStyle('userPy', h.client)
    await resetChatStyle(undefined, h.client)
    expect(h.invoke).toHaveBeenNthCalledWith(1, 'settings_reset', { key: 'chat.style.userPy' })
    expect(h.invoke).toHaveBeenNthCalledWith(2, 'settings_reset', { key: 'chat.style.' })
  })

  it('설치 · 적용 · 쓰기 어디서도 localStorage 를 건드리지 않는다', async () => {
    const getItem = vi.spyOn(Storage.prototype, 'getItem')
    const setItem = vi.spyOn(Storage.prototype, 'setItem')
    const removeItem = vi.spyOn(Storage.prototype, 'removeItem')

    const h = await installed()
    h.push([['chat.style.fontSize', '16px']])
    await setChatStyle('fontSize', '17px', h.client)
    await resetChatStyle('fontSize', h.client)

    expect(rootVar('--chat-font-size')).toBe('17px')
    expect(getItem).not.toHaveBeenCalled()
    expect(setItem).not.toHaveBeenCalled()
    expect(removeItem).not.toHaveBeenCalled()
  })
})

// ── Whitelist ↔ theme.css fallback ─────────────
//   The first paint is handled by `--chat-*` in theme.css `:root`. We only check that the set of variables the applier paints and the set of those declarations are the same
//   — the values (whether they match the defaults in the shell schema table) are not measured (ADR-0265 「U2 의 대가」 (U2's cost)).
describe('chatStyleStore 화이트리스트 ↔ theme.css', () => {
  const KEYS: ChatStyleKey[] = [
    'railRowPt',
    'plainRowPt',
    'userPy',
    'userPx',
    'userMy',
    'railGutter',
    'railLineOffset',
    'railDotTop',
    'fontSize',
    'lineHeight',
    'waitStripH',
  ]

  it('11키를 모두 받으면 theme.css 에 선언된 --chat-* 변수 전부를, 그것만 칠한다', async () => {
    const h = await installed()
    h.push(KEYS.map(k => [`chat.style.${k}`, '1px']))

    const painted = Array.from(document.documentElement.style)
      .filter(name => name.startsWith('--chat-'))
      .sort()

    const css = require('node:fs').readFileSync(`${process.cwd()}/src/styles/theme.css`, 'utf8')
    // Strip the comments first so a variable-name citation inside a comment (`--chat-rail-row-pt(커플링)` etc.) is not counted as a declaration.
    const declared = Array.from(
      css.replace(/\/\*[\s\S]*?\*\//g, '').matchAll(/(--chat-[a-z0-9-]+)\s*:/g),
      (m: RegExpMatchArray) => m[1],
    )
    expect(declared.length).toBeGreaterThan(0)
    expect(painted).toEqual([...new Set(declared)].sort())
  })
})
