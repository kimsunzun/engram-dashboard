// The `ui.language` applier (ADR-0373) — a shell setting value switches this window's language and its per-window memory;
//   anything but `en` | `ko` changes nothing. The settings client is the real one over a fake IPC.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ label: 'main' }),
}))

import { createSettingsClient, type SettingsIpc } from '../api/settingsClient'
import { getLanguage, setLanguage } from './index'
import { installLanguageApplier, LANGUAGE_SETTING_KEY } from './languageSetting'

const MEMORY_KEY = 'engram.lang.main'

/** A client whose notices the test pushes; the boot pull answers nothing. */
function harness() {
  let handler: ((payload: unknown) => void) | undefined
  let rev = 0
  const ipc: SettingsIpc = {
    async listen(_event, h) {
      handler = h
      return () => {}
    },
    invoke: async <T,>(cmd: string) => {
      if (cmd === 'settings_get') return { rev, items: [] } as T
      throw new Error(`unexpected command ${cmd}`)
    },
  }
  const client = createSettingsClient(ipc)
  return {
    client,
    push(items: Array<[string, string]>) {
      if (!handler) throw new Error('not listening yet')
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
  disposers = []
  sessionStorage.clear()
})

afterEach(() => {
  for (const d of disposers) d()
  setLanguage('ko')
  vi.restoreAllMocks()
})

describe('installLanguageApplier', () => {
  it('a value switches the language, <html lang> and the window memory', async () => {
    const h = harness()
    disposers.push(installLanguageApplier(h.client), h.client.install())
    await flush()

    h.push([[LANGUAGE_SETTING_KEY, 'en']])

    expect(getLanguage()).toBe('en')
    expect(document.documentElement.lang).toBe('en')
    expect(sessionStorage.getItem(MEMORY_KEY)).toBe('en')

    h.push([[LANGUAGE_SETTING_KEY, 'ko']])

    expect(getLanguage()).toBe('ko')
    expect(sessionStorage.getItem(MEMORY_KEY)).toBe('ko')
  })

  it('a value received before installing is applied on install (the client replays it)', async () => {
    const h = harness()
    disposers.push(h.client.install())
    await flush()
    h.push([[LANGUAGE_SETTING_KEY, 'en']])
    expect(getLanguage()).toBe('ko')

    disposers.push(installLanguageApplier(h.client))

    expect(getLanguage()).toBe('en')
    expect(sessionStorage.getItem(MEMORY_KEY)).toBe('en')
  })

  it.each(['fr', 'EN', 'ko-KR', ''])('ignores %j — no language change, no memory write', async value => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const h = harness()
    disposers.push(installLanguageApplier(h.client), h.client.install())
    await flush()

    h.push([[LANGUAGE_SETTING_KEY, value]])

    expect(getLanguage()).toBe('ko')
    expect(document.documentElement.lang).toBe('ko')
    expect(sessionStorage.length).toBe(0)
    expect(warn).toHaveBeenCalledTimes(1)
  })

  it('other keys are not read', async () => {
    const h = harness()
    disposers.push(installLanguageApplier(h.client), h.client.install())
    await flush()

    h.push([
      ['theme.default', 'en'],
      ['ui.languages', 'en'],
    ])

    expect(getLanguage()).toBe('ko')
    expect(sessionStorage.length).toBe(0)
  })

  it('after unsubscribing a value changes nothing', async () => {
    const h = harness()
    const off = installLanguageApplier(h.client)
    disposers.push(h.client.install())
    await flush()
    off()

    h.push([[LANGUAGE_SETTING_KEY, 'en']])

    expect(getLanguage()).toBe('ko')
  })
})
