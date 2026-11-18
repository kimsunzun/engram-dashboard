// Strategy: the sibling (slotCommands.test.ts) swaps store actions for spies and measures only "what was called"; here the ★real viewStore★ is used and renderModeOverride asserted directly — ① the toggle needs a round trip (dom→default→dom), and with a spy the decision lives inside the store and cannot be measured; ② "an invalid argument does not reach the store" is stronger as "state unchanged + the store's warn guard did not fire" than "spy not called".
// ★Do not import `./renderModeCommands` directly — it comes through the manifest★: the app attaches commands via the one `./contributions` line (`App.tsx`); a direct import would keep tests green even if that line disappeared (it registers by itself). Only the manifest path exercises "it shows up in __engramCmd.list". The price is that every command module loads, so the stub set is the same as busCommands.test.ts.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => vi.fn()) }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ label: 'main', close: vi.fn(async () => undefined) }),
}))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(async () => null) }))
vi.mock('../api/clientFactory', () => ({
  agentClient: new Proxy({}, { get: () => vi.fn(async () => undefined) }),
  getAgentClient: vi.fn(),
  bootstrapDaemonIfNeeded: vi.fn(async () => undefined),
}))

import './contributions' // side-effect register — ★no direct import (header above)★
import { list, run } from './registry'
import { useViewStore } from '../store/viewStore'

const IDS = [
  'slot.renderMode.set',
  'slot.renderMode.clear',
  'slot.domMode.enable',
  'slot.domMode.disable',
  'slot.domMode.toggle',
]

const override = (): Record<string, string> => useViewStore.getState().renderModeOverride

beforeEach(() => useViewStore.setState({ renderModeOverride: {} }))
afterEach(() => vi.restoreAllMocks())

describe('render mode command routing', () => {
  // ★Manifest wiring lock★: deleting `import './renderModeCommands'` from contributions.ts breaks this first (all routing tests below fail with "unknown command id" too).
  it('the five ids go into the registry via the manifest (contributions) — the list __engramCmd.list sees', () => {
    const ids = list().map(c => c.id)
    for (const id of IDS) expect(ids, `${id}: is there a module import in contributions.ts`).toContain(id)
  })

  it('slot.renderMode.set → records the mode for that slot', () => {
    run('slot.renderMode.set', { slotId: 's1', mode: 'rich' })
    expect(override()['s1']).toBe('rich')
    run('slot.renderMode.set', { slotId: 's1', mode: 'terminal' })
    expect(override()['s1']).toBe('terminal')
  })

  // ★Case-insensitive acceptance + declared spelling normalization★: what lands in the store is an element of RENDER_MODES, not the caller's spelling — else the switch in LayoutLeaf silently falls to terminal.
  it('mode accepted case-insensitively → normalized to the declared spelling and recorded', () => {
    run('slot.renderMode.set', { slotId: 's1', mode: 'RICH' })
    expect(override()['s1']).toBe('rich')
    run('slot.renderMode.set', { slotId: 's1', mode: 'Terminal' })
    expect(override()['s1']).toBe('terminal')
    run('slot.renderMode.set', { slotId: 's1', mode: 'DoM' })
    expect(override()['s1']).toBe('dom')
  })

  it('slot.renderMode.clear → removes only that slot entry (other slots unchanged)', () => {
    run('slot.renderMode.set', { slotId: 's1', mode: 'dom' })
    run('slot.renderMode.set', { slotId: 's2', mode: 'rich' })
    run('slot.renderMode.clear', { slotId: 's1' })
    expect(override()['s1']).toBeUndefined()
    expect(override()['s2']).toBe('rich')
  })

  it('clear on a slot with no override is idempotent (no throw)', () => {
    expect(() => run('slot.renderMode.clear', { slotId: 'never-set' })).not.toThrow()
    expect(override()).toEqual({})
  })

  it('slot.domMode.enable → dom', () => {
    run('slot.domMode.enable', { slotId: 's1' })
    expect(override()['s1']).toBe('dom')
  })

  // disable = clearRenderMode, so an override that is not dom is withdrawn too (the real meaning of the alias — "return to default").
  it('slot.domMode.disable → removes the entry (a rich override is cleared too)', () => {
    run('slot.renderMode.set', { slotId: 's1', mode: 'rich' })
    run('slot.domMode.disable', { slotId: 's1' })
    expect(override()['s1']).toBeUndefined()
  })

  it('slot.domMode.toggle round trip: dom → default → dom', () => {
    run('slot.domMode.toggle', { slotId: 's1' })
    expect(override()['s1']).toBe('dom')
    run('slot.domMode.toggle', { slotId: 's1' })
    expect(override()['s1']).toBeUndefined()
    run('slot.domMode.toggle', { slotId: 's1' })
    expect(override()['s1']).toBe('dom')
  })

  it('slot.domMode.toggle: from a non-dom override (rich) it goes up to dom', () => {
    run('slot.renderMode.set', { slotId: 's1', mode: 'rich' })
    run('slot.domMode.toggle', { slotId: 's1' })
    expect(override()['s1']).toBe('dom')
  })

  // slotId is a globally unique UUID across windows (ADR-0035) so it does not take viewId — but it must be able to take the slot menu ctx bag
  //   as is, so extra keys are ignored.
  it('a ctx bag mixed with an extra viewId passes as is', () => {
    run('slot.domMode.enable', { viewId: 'v1', slotId: 's1' })
    expect(override()['s1']).toBe('dom')
  })
})

describe('invalid argument refusal (no silent no-op)', () => {
  it('slotId missing → all five throw + store unchanged', () => {
    for (const id of IDS) {
      expect(() => run(id, { mode: 'dom' }), id).toThrow(/slotId/)
    }
    expect(override()).toEqual({})
  })

  it('slotId not a string or an empty string → throw', () => {
    expect(() => run('slot.domMode.enable', { slotId: 42 })).toThrow(/slotId/)
    expect(() => run('slot.domMode.enable', { slotId: '' })).toThrow(/slotId/)
    expect(override()).toEqual({})
  })

  // ★Key★: the store's setRenderMode warns and ignores an invalid mode — flowing down that path would look like success to the caller; the command layer rejects first, so it never reaches the store's warn guard.
  it('mode invalid/missing → throw + store unchanged + the store warn guard not reached', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    expect(() => run('slot.renderMode.set', { slotId: 's1', mode: 'bogus' })).toThrow(/mode/)
    expect(() => run('slot.renderMode.set', { slotId: 's1' })).toThrow(/mode/)
    expect(() => run('slot.renderMode.set', { slotId: 's1', mode: 7 })).toThrow(/mode/)
    expect(override()['s1']).toBeUndefined()
    expect(warn).not.toHaveBeenCalled()
  })

  it('even when only mode is invalid, no partial application on the slotId side remains', () => {
    run('slot.renderMode.set', { slotId: 's1', mode: 'dom' })
    expect(() => run('slot.renderMode.set', { slotId: 's1', mode: 'bogus' })).toThrow(/mode/)
    expect(override()['s1']).toBe('dom') // Old value kept — an invalid call neither clears nor overwrites it.
  })
})
