
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => vi.fn()) }))

import { invoke } from '@tauri-apps/api/core'

import './tabCommands' // side-effect register
import { run } from './registry'
import { useViewStore } from '../store/viewStore'
import type { WindowTabsPayload } from '../store/viewStore'
import type { LayoutNode, WindowControlAction, WindowControlOutcome } from '../api/layoutTypes'

const POPUP = 'slot-popup-0a1b2c3d-1'
const POPUP_ID = '7d2f0b8e-1c3a-4f5e-9b6d-2a4c6e8f0a1b'

function payload(label: string, window_id: string, number: number, ids: string[], active: string): WindowTabsPayload {
  return {
    label,
    window_id,
    number,
    title: '',
    tabs: ids.map(id => ({ id, name: id.toUpperCase(), named: true })),
    active,
    version: 1,
  }
}

/**
 * Imitates the shell `list_tabs` — resolves the window argument like the shell resolver (digits only = number refused · persistent id · label). Refuses with a string, like the shell.
 */
function shellWith(...windows: WindowTabsPayload[]): void {
  vi.mocked(invoke).mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd !== 'list_tabs') return undefined
    const arg = String((args as { window: string }).window).trim()
    if (/^\d+$/.test(arg)) throw 'a window number is not accepted as a window argument — numbers are reused. Find that number\'s window_id in the details of window.list and pass it, or pass the window (label) of a layout.get row'
    const hit = windows.find(w => w.label === arg || w.window_id === arg)
    if (!hit) throw `window not found: ${arg}`
    return hit
  })
}

const createTabSpy = vi.fn(async () => 'new-view')
const switchTabSpy = vi.fn(async () => undefined)
const closeTabSpy = vi.fn(async () => undefined)
const NEW_WINDOW = { window: 'slot-popup-0a1b2c3d-1', window_id: '7d2f0b8e-1c3a-4f5e-9b6d-2a4c6e8f0a1b', number: 2 }
const createWindowSpy = vi.fn(async () => NEW_WINDOW)
/** Imitates the shell `window_control` answer — every field is true, so it shows whether each command trimmed it to the bus answer shape. `close` is main = hide. */
const controlWindowSpy = vi.fn(
  async (window: string, action: WindowControlAction, ifEmpty?: boolean): Promise<WindowControlOutcome> => ({
    deferred: true,
    pending: true,
    shown_all: true,
    focused: true,
    close: action !== 'close' ? null : window === 'main' ? 'hidden_all' : ifEmpty ? 'kept' : 'closed',
  }),
)
const renameTabSpy = vi.fn(async () => undefined)

const origHash = window.location.hash

beforeEach(() => {
  vi.mocked(invoke).mockReset()
  vi.mocked(invoke).mockImplementation(async () => undefined)
  createTabSpy.mockClear()
  switchTabSpy.mockClear()
  closeTabSpy.mockClear()
  createWindowSpy.mockClear()
  controlWindowSpy.mockClear()
  renameTabSpy.mockClear()
  window.location.hash = '#/' // main window context
  useViewStore.setState({
    windows: {},
    createTab: createTabSpy,
    switchTab: switchTabSpy,
    closeTab: closeTabSpy,
    createWindow: createWindowSpy,
    controlWindow: controlWindowSpy,
    renameTab: renameTabSpy,
  })
})
afterEach(() => {
  window.location.hash = origHash
})

describe('tabCommands routing (window resolution)', () => {
  it('tab.create (window omitted) → createTab(this window=main, undefined)', () => {
    run('tab.create')
    expect(createTabSpy).toHaveBeenCalledWith('main', undefined)
  })

  it('tab.create (window/name given) → createTab(that window, name)', () => {
    run('tab.create', { window: 'slot-popup-2', name: 'My Tab' })
    expect(createTabSpy).toHaveBeenCalledWith('slot-popup-2', 'My Tab')
  })

  it('tab.create (name with leading/trailing spaces) → createTab(that window, trimmed name)', () => {
    run('tab.create', { window: 'slot-popup-2', name: '  My Tab  ' })
    expect(createTabSpy).toHaveBeenCalledWith('slot-popup-2', 'My Tab')
  })

  it('tab.create (blank name) → throw (createTab not called)', () => {
    expect(() => run('tab.create', { name: '   ' })).toThrow()
    expect(createTabSpy).not.toHaveBeenCalled()
  })

  it('tab.create (name omitted · window given) → createTab(that window, undefined) — automatic name', () => {
    run('tab.create', { window: 'slot-popup-2' })
    expect(createTabSpy).toHaveBeenCalledWith('slot-popup-2', undefined)
  })

  it('tab.switch (view given) → switchTab(this window, view)', () => {
    run('tab.switch', { view: 'v7' })
    expect(switchTabSpy).toHaveBeenCalledWith('main', 'v7')
  })

  it('tab.switch (view missing) → throw', () => {
    expect(() => run('tab.switch')).toThrow()
  })

  it('tab.close (view given) → closeTab(this window, view)', () => {
    run('tab.close', { view: 'v3' })
    expect(closeTabSpy).toHaveBeenCalledWith('main', 'v3')
  })

  // ── ★S4-F1★ — view omitted = the active tab of the window the shell resolved ──
  it('tab.close (window label · view omitted) → closes the active tab of that window (not the current webview active)', async () => {
    shellWith(payload('main', 'main', 1, ['mv'], 'mv'), payload(POPUP, POPUP_ID, 2, ['pv'], 'pv'))
    await run('tab.close', { window: POPUP })
    // ★Had it been the bug, it would close with main's active (mv), giving a (popup, mv) mismatch → backend ViewNotFound★.
    expect(closeTabSpy).toHaveBeenCalledWith(POPUP, 'pv')
  })

  it('tab.close (window_id · view omitted) → closes with the label the shell resolved and the active tab of that window', async () => {
    shellWith(payload('main', 'main', 1, ['mv'], 'mv'), payload(POPUP, POPUP_ID, 2, ['pv'], 'pv'))
    await run('tab.close', { window: POPUP_ID })
    expect(invoke).toHaveBeenCalledWith('list_tabs', { window: POPUP_ID })
    expect(closeTabSpy).toHaveBeenCalledWith(POPUP, 'pv')
  })

  it('tab.close (window and view both omitted) → closes the active tab of this webview window (main)', async () => {
    shellWith(payload('main', 'main', 1, ['mv'], 'mv'))
    await run('tab.close')
    expect(closeTabSpy).toHaveBeenCalledWith('main', 'mv')
  })

  it('tab.close (window number · view omitted) → throws the shell refusal (does not close)', async () => {
    shellWith(payload('main', 'main', 1, ['mv'], 'mv'), payload(POPUP, POPUP_ID, 2, ['pv'], 'pv'))
    await expect(run('tab.close', { window: '2' })).rejects.toThrow(/^\[tab\.close\] a window number/)
    expect(closeTabSpy).not.toHaveBeenCalled()
  })

  it('tab.close (unknown window · view omitted) → throws the shell refusal', async () => {
    shellWith(payload('main', 'main', 1, ['mv'], 'mv'))
    await expect(run('tab.close', { window: 'slot-popup-9' })).rejects.toThrow(/window not found: slot-popup-9/)
    expect(closeTabSpy).not.toHaveBeenCalled()
  })

  // ── ★tab.rename(ADR-0057)★ ──
  it('tab.rename (view/name given) → renameTab(view, trimmed name)', () => {
    run('tab.rename', { view: 'v7', name: '  My Tab  ' })
    expect(renameTabSpy).toHaveBeenCalledWith('v7', 'My Tab')
  })

  it('tab.rename (view missing) → throw (renameTab not called)', () => {
    expect(() => run('tab.rename', { name: 'x' })).toThrow()
    expect(renameTabSpy).not.toHaveBeenCalled()
  })

  it('tab.rename (name missing) → throw', () => {
    expect(() => run('tab.rename', { view: 'v1' })).toThrow()
    expect(renameTabSpy).not.toHaveBeenCalled()
  })

  it('tab.rename (blank name) → throw (loud fail before invoke)', () => {
    expect(() => run('tab.rename', { view: 'v1', name: '   ' })).toThrow()
    expect(renameTabSpy).not.toHaveBeenCalled()
  })

  it('window.create → createWindow() · answer = {window, window_id, number}', async () => {
    await expect(run('window.create')).resolves.toEqual(NEW_WINDOW)
    expect(createWindowSpy).toHaveBeenCalledTimes(1)
  })

  it('window.close (window omitted) → window_control close(this window=main) · answer = bus-spelled outcome', async () => {
    await expect(run('window.close')).resolves.toEqual({ outcome: 'HiddenAll' })
    expect(controlWindowSpy).toHaveBeenCalledWith('main', 'close', false)
  })

  it('window.close (popout window_id) → answer outcome = Closed', async () => {
    await expect(run('window.close', { window: POPUP_ID })).resolves.toEqual({ outcome: 'Closed' })
    expect(controlWindowSpy).toHaveBeenCalledWith(POPUP_ID, 'close', false)
  })

  it('window.close (if_empty = true · window_id) → sends ifEmpty along · answer outcome = Kept', async () => {
    await expect(run('window.close', { window: POPUP_ID, if_empty: true })).resolves.toEqual({ outcome: 'Kept' })
    expect(controlWindowSpy).toHaveBeenCalledWith(POPUP_ID, 'close', true)
  })

  it('window.close (if_empty = true · window omitted) → throw — falling back to this window label, the shell finds nothing', () => {
    for (const w of [undefined, null]) {
      expect(() => run('window.close', { window: w, if_empty: true })).toThrow(/^\[window\.close\] if_empty requires window/)
    }
    expect(controlWindowSpy).not.toHaveBeenCalled()
  })

  it('window.close (if_empty not a boolean) → throw', () => {
    for (const ifEmpty of ['true', 1]) {
      expect(() => run('window.close', { window: POPUP_ID, if_empty: ifEmpty })).toThrow(
        /^\[window\.close\] if_empty must be a boolean/,
      )
    }
    expect(controlWindowSpy).not.toHaveBeenCalled()
  })

  // ── Window state commands — the same ids and the same answer shapes as the shell bus (TRD S26-window-chrome §6-1 · §6-4) ──
  it.each([
    ['window.minimize', 'minimize', {}],
    ['window.maximize', 'maximize', { deferred: true }],
    ['window.unmaximize', 'unmaximize', { deferred: true }],
    ['window.raise', 'raise', { shown_all: true, focused: true, pending: true }],
  ] as const)('%s → window_control %s (that window) · answer = only the bus answer fields', async (id, action, answer) => {
    await expect(run(id, { window: POPUP_ID })).resolves.toEqual(answer)
    expect(controlWindowSpy).toHaveBeenCalledWith(POPUP_ID, action)
    await run(id)
    expect(controlWindowSpy).toHaveBeenLastCalledWith('main', action)
  })

  it.each([
    ['window.hideAll', 'hide_all', {}],
    ['window.showAll', 'show_all', { focused: true }],
  ] as const)('%s → window_control %s · does not pick a window by the window argument and fills this window label', async (id, action, answer) => {
    window.location.hash = `#/popup?window=${POPUP}`
    for (const w of [undefined, null, 'main', POPUP_ID]) {
      controlWindowSpy.mockClear()
      await expect(run(id, { window: w })).resolves.toEqual(answer)
      expect(controlWindowSpy).toHaveBeenCalledWith(POPUP, action)
    }
  })

  it.each(['window.hideAll', 'window.showAll'])('%s also throws when the given window is empty or not a string', id => {
    for (const w of ['', '   ', 2]) {
      expect(() => run(id, { window: w })).toThrow(new RegExp(`^\\[${id.replace('.', '\\.')}\\] window must be a non-empty`))
    }
    expect(controlWindowSpy).not.toHaveBeenCalled()
  })

  it('window state commands do not fold an empty window into this window either', () => {
    expect(() => run('window.maximize', { window: '  ' })).toThrow(/^\[window\.maximize\] window must be a non-empty/)
    expect(controlWindowSpy).not.toHaveBeenCalled()
  })

  it.each(['window.minimize', 'window.maximize', 'window.unmaximize', 'window.raise', 'window.hideAll', 'window.showAll'])(
    '%s: a shell refusal (string) is thrown as an Error carrying the command name',
    async id => {
      controlWindowSpy.mockRejectedValueOnce('a window number is not accepted as a window argument')
      const err = await Promise.resolve(run(id, { window: POPUP_ID })).catch((e: unknown) => e)
      expect(err).toBeInstanceOf(Error)
      expect((err as Error).message).toBe(`[${id}] a window number is not accepted as a window argument`)
    },
  )

  it('a shell refusal (string) of window.close is also thrown as an Error carrying the command name', async () => {
    controlWindowSpy.mockRejectedValueOnce('window not found: w-9')
    const err = await Promise.resolve(run('window.close', { window: 'w-9', if_empty: true })).catch((e: unknown) => e)
    expect(err).toBeInstanceOf(Error)
    expect((err as Error).message).toBe('[window.close] window not found: w-9')
  })

  // ── An empty window argument is not folded into this window ──
  it('window: undefined → falls back to this window (main)', () => {
    run('window.close', { window: undefined })
    expect(controlWindowSpy).toHaveBeenCalledWith('main', 'close', false)
  })

  it('window: null → falls back to this window (main) like undefined', () => {
    run('window.close', { window: null })
    expect(controlWindowSpy).toHaveBeenCalledWith('main', 'close', false)
  })

  it('throws an error with the command prefix even when window is a BigInt or a cyclic object', () => {
    const circular: Record<string, unknown> = {}
    circular.self = circular
    for (const w of [1n, circular]) {
      expect(() => run('tab.create', { window: w })).toThrow(/^\[tab\.create\] window/)
    }
    expect(createTabSpy).not.toHaveBeenCalled()
  })

  it('window empty string · blank only → throw (not folded into this window)', () => {
    for (const w of ['', '   ']) {
      expect(() => run('window.close', { window: w })).toThrow(/^\[window\.close\] window must be a non-empty string/)
      expect(() => run('tab.create', { window: w })).toThrow(/^\[tab\.create\] window/)
    }
    expect(controlWindowSpy).not.toHaveBeenCalled()
    expect(createTabSpy).not.toHaveBeenCalled()
  })

  it('throws when window is not a string (a number)', async () => {
    expect(() => run('tab.switch', { window: 2, view: 'v1' })).toThrow(/^\[tab\.switch\] window/)
    await expect(run('tab.close', { window: 2 })).rejects.toThrow(/^\[tab\.close\] window/)
    await expect(run('tab.next', { window: 2 })).rejects.toThrow(/^\[tab\.next\] window/)
    expect(switchTabSpy).not.toHaveBeenCalled()
    expect(closeTabSpy).not.toHaveBeenCalled()
    expect(invoke).not.toHaveBeenCalled()
  })
})

// ── ★layout.setSlotContent variant shape validation (FIX LOW)★ ─────────────────────────────────────────
// What this suite blocks: a malformed value like {type:'agent'} (agent_id missing) passing the registry and blowing up late in Rust
// deserialization — loud fail before invoke.
describe('layout.setSlotContent variant shape validation', () => {
  const setSlotContentSpy = vi.fn(async () => undefined)
  beforeEach(() => {
    setSlotContentSpy.mockClear()
    useViewStore.setState({ setSlotContent: setSlotContentSpy })
  })

  it('agent variant without agent_id → throw (loud fail at the registry boundary, before invoke)', () => {
    expect(() =>
      run('layout.setSlotContent', { viewId: 'v1', slotId: 's1', content: { type: 'agent' } }),
    ).toThrow(/agent_id/)
    expect(setSlotContentSpy).not.toHaveBeenCalled()
  })

  it('agent variant whose agent_id is a non-string (number) → throw', () => {
    expect(() =>
      run('layout.setSlotContent', { viewId: 'v1', slotId: 's1', content: { type: 'agent', agent_id: 42 } }),
    ).toThrow(/agent_id/)
    expect(setSlotContentSpy).not.toHaveBeenCalled()
  })

  it('agent variant whose agent_id is an empty string → throw', () => {
    expect(() =>
      run('layout.setSlotContent', { viewId: 'v1', slotId: 's1', content: { type: 'agent', agent_id: '' } }),
    ).toThrow(/agent_id/)
    expect(setSlotContentSpy).not.toHaveBeenCalled()
  })

  it('an unknown type still throws (whitelist)', () => {
    expect(() =>
      run('layout.setSlotContent', { viewId: 'v1', slotId: 's1', content: { type: 'bogus' } }),
    ).toThrow(/SlotContent variant/)
    expect(setSlotContentSpy).not.toHaveBeenCalled()
  })

  it('a valid agent variant (agent_id string) → routed to setSlotContent', () => {
    run('layout.setSlotContent', { viewId: 'v1', slotId: 's1', content: { type: 'agent', agent_id: 'a-1' } })
    expect(setSlotContentSpy).toHaveBeenCalledWith('v1', 's1', { type: 'agent', agent_id: 'a-1' })
  })

  it('unit variants (empty/agent_list/preset_palette) pass without extra fields', () => {
    run('layout.setSlotContent', { viewId: 'v1', slotId: 's1', content: { type: 'empty' } })
    run('layout.setSlotContent', { viewId: 'v1', slotId: 's2', content: { type: 'agent_list' } })
    run('layout.setSlotContent', { viewId: 'v1', slotId: 's3', content: { type: 'preset_palette' } })
    expect(setSlotContentSpy).toHaveBeenCalledWith('v1', 's1', { type: 'empty' })
    expect(setSlotContentSpy).toHaveBeenCalledWith('v1', 's2', { type: 'agent_list' })
    expect(setSlotContentSpy).toHaveBeenCalledWith('v1', 's3', { type: 'preset_palette' })
  })

  // ★Case-insensitive acceptance + normalization to the declared spelling★: the tag flowing to the store and backend must be the declared spelling of the SlotContent union, not the caller's (wire bytes identical).
  it('type accepted case-insensitively → normalized to the declared spelling and routed', () => {
    run('layout.setSlotContent', { viewId: 'v1', slotId: 's1', content: { type: 'EMPTY' } })
    run('layout.setSlotContent', { viewId: 'v1', slotId: 's2', content: { type: 'Agent_List' } })
    run('layout.setSlotContent', { viewId: 'v1', slotId: 's3', content: { type: 'PRESET_PALETTE' } })
    expect(setSlotContentSpy).toHaveBeenCalledWith('v1', 's1', { type: 'empty' })
    expect(setSlotContentSpy).toHaveBeenCalledWith('v1', 's2', { type: 'agent_list' })
    expect(setSlotContentSpy).toHaveBeenCalledWith('v1', 's3', { type: 'preset_palette' })
  })

  it('agent variant folds only the tag and carries agent_id as is', () => {
    run('layout.setSlotContent', { viewId: 'v1', slotId: 's1', content: { type: 'Agent', agent_id: 'A-1' } })
    expect(setSlotContentSpy).toHaveBeenCalledWith('v1', 's1', { type: 'agent', agent_id: 'A-1' })
  })

  // ── Usage slot (TRD S21 usage-limit-slot §1-8) — show_* is a boolean when present · a missing field is that slot's current value (the shell serde default when none) ──
  it('usage variant — show_* booleans as is · missing fields of an unknown slot are passed on unfilled · the tag is folded', () => {
    run('layout.setSlotContent', {
      viewId: 'v1',
      slotId: 's1',
      content: { type: 'Usage', show_claude: false, show_codex: true },
    })
    run('layout.setSlotContent', { viewId: 'v1', slotId: 's2', content: { type: 'usage', show_codex: false } })
    run('layout.setSlotContent', { viewId: 'v1', slotId: 's3', content: { type: 'usage' } })
    expect(setSlotContentSpy).toHaveBeenCalledWith('v1', 's1', { type: 'usage', show_claude: false, show_codex: true })
    expect(setSlotContentSpy).toHaveBeenCalledWith('v1', 's2', { type: 'usage', show_codex: false })
    expect(setSlotContentSpy).toHaveBeenCalledWith('v1', 's3', { type: 'usage' })
  })

  // Must give the same result as the bus `layout.setSlotContent` (merge inside the shell lock) — so the two LLM paths do not diverge (TRD §3 #20).
  it('usage variant — missing show fields of a slot that is already usage are filled with the current value · given fields as is', () => {
    const layout: LayoutNode = {
      type: 'split',
      id: 'sp',
      dir: 'left_right',
      ratio: 0.5,
      a: { type: 'slot', id: 'u1', content: { type: 'usage', show_claude: false, show_codex: false } },
      b: { type: 'slot', id: 'e1', content: { type: 'empty' } },
    }
    useViewStore.setState({ layouts: { v1: { layout } as never } })
    try {
      run('layout.setSlotContent', { viewId: 'v1', slotId: 'u1', content: { type: 'usage', show_codex: true } })
      run('layout.setSlotContent', { viewId: 'v1', slotId: 'u1', content: { type: 'usage' } })
      // A slot that is not usage now is not merged — the shell reads missing fields as true.
      run('layout.setSlotContent', { viewId: 'v1', slotId: 'e1', content: { type: 'usage', show_claude: false } })
      expect(setSlotContentSpy.mock.calls).toEqual([
        ['v1', 'u1', { type: 'usage', show_claude: false, show_codex: true }],
        ['v1', 'u1', { type: 'usage', show_claude: false, show_codex: false }],
        ['v1', 'e1', { type: 'usage', show_claude: false }],
      ])
    } finally {
      useViewStore.setState({ layouts: {} })
    }
  })

  it('usage variant — throw before invoke when show_* is not a boolean', () => {
    for (const bad of ['false', 0, null, {}]) {
      expect(() =>
        run('layout.setSlotContent', { viewId: 'v1', slotId: 's1', content: { type: 'usage', show_claude: bad } }),
      ).toThrow(/show_claude of the usage variant must be a boolean/)
      expect(() =>
        run('layout.setSlotContent', { viewId: 'v1', slotId: 's1', content: { type: 'usage', show_codex: bad } }),
      ).toThrow(/show_codex of the usage variant must be a boolean/)
    }
    expect(setSlotContentSpy).not.toHaveBeenCalled()
  })

  it('only the letter case is folded — a different spelling still throws', () => {
    expect(() =>
      run('layout.setSlotContent', { viewId: 'v1', slotId: 's1', content: { type: 'agentlist' } }),
    ).toThrow(/SlotContent variant/)
    expect(() =>
      run('layout.setSlotContent', { viewId: 'v1', slotId: 's1', content: { type: 'Emptyy' } }),
    ).toThrow(/SlotContent variant/)
    expect(setSlotContentSpy).not.toHaveBeenCalled()
  })
})

// Tab order and active tab are based on the shell answer (list_tabs) — the web store is not consulted.
describe('tab.next (Ctrl+Tab cycling)', () => {
  it('several tabs → cycles to the right (switch to the tab after active)', async () => {
    shellWith(payload('main', 'main', 1, ['a', 'b', 'c'], 'a'))
    await run('tab.next')
    expect(invoke).toHaveBeenCalledWith('list_tabs', { window: 'main' })
    expect(switchTabSpy).toHaveBeenCalledWith('main', 'b')
  })

  it('last tab active → wraps to the first tab', async () => {
    shellWith(payload('main', 'main', 1, ['a', 'b'], 'b'))
    await run('tab.next')
    expect(switchTabSpy).toHaveBeenCalledWith('main', 'a')
  })

  it('1 tab → no-op (switch not called)', async () => {
    shellWith(payload('main', 'main', 1, ['a'], 'a'))
    await run('tab.next')
    expect(switchTabSpy).not.toHaveBeenCalled()
  })

  it('moves by the shell answer even when the web store does not have that window', async () => {
    useViewStore.setState({ windows: {} })
    shellWith(payload('main', 'main', 1, ['a', 'b'], 'a'))
    await run('tab.next')
    expect(switchTabSpy).toHaveBeenCalledWith('main', 'b')
  })

  it('window label → moves in that window', async () => {
    shellWith(payload('main', 'main', 1, ['a', 'b'], 'a'), payload(POPUP, POPUP_ID, 2, ['p', 'q'], 'q'))
    await run('tab.next', { window: POPUP })
    expect(switchTabSpy).toHaveBeenCalledWith(POPUP, 'p')
  })

  it('window_id → moves by the label the shell resolved', async () => {
    shellWith(payload('main', 'main', 1, ['a', 'b'], 'a'), payload(POPUP, POPUP_ID, 2, ['p', 'q'], 'p'))
    await run('tab.next', { window: POPUP_ID })
    expect(invoke).toHaveBeenCalledWith('list_tabs', { window: POPUP_ID })
    expect(switchTabSpy).toHaveBeenCalledWith(POPUP, 'q')
  })

  it('window number → throws the shell refusal (does not move)', async () => {
    shellWith(payload('main', 'main', 1, ['a', 'b'], 'a'), payload(POPUP, POPUP_ID, 2, ['p', 'q'], 'p'))
    await expect(run('tab.next', { window: '2' })).rejects.toThrow(/^\[tab\.next\] a window number/)
    expect(switchTabSpy).not.toHaveBeenCalled()
  })

  it('unknown window → throws the shell refusal', async () => {
    shellWith(payload('main', 'main', 1, ['a', 'b'], 'a'))
    await expect(run('tab.next', { window: 'slot-popup-9' })).rejects.toThrow(/window not found: slot-popup-9/)
    expect(switchTabSpy).not.toHaveBeenCalled()
  })
})
