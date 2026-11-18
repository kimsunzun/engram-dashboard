import { describe, expect, it, vi, beforeEach } from 'vitest'

const clientMock = vi.hoisted(() => ({
  createPreset: vi.fn(async () => undefined),
  deletePreset: vi.fn(async () => undefined),
  renamePreset: vi.fn(async () => undefined),
  listPresets: vi.fn(async () => []),
}))
vi.mock('../api/clientFactory', () => ({
  agentClient: {
    createPreset: (...args: unknown[]) => clientMock.createPreset(...(args as [])),
    deletePreset: (...args: unknown[]) => clientMock.deletePreset(...(args as [])),
    renamePreset: (...args: unknown[]) => clientMock.renamePreset(...(args as [])),
    listPresets: (...args: unknown[]) => clientMock.listPresets(...(args as [])),
  },
  getAgentClient: vi.fn(),
}))
// preset.add calls the native folder dialog — the import side effect only registers, so open is a no-op mock.
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(async () => null) }))
// The slotMenu registerSlotMenu side effect is harmless, but the side-effect import happens once only to avoid duplicate registry registration.

import './presetCommands' // side-effect register
import { run } from './registry'

beforeEach(() => {
  clientMock.createPreset.mockClear()
  clientMock.deletePreset.mockClear()
  clientMock.renamePreset.mockClear()
})

describe('preset.rename routing (§5 — ADR-0061 enrichment)', () => {
  it('id + name → renamePreset(id, trimmed)', () => {
    run('preset.rename', { id: '  pr1  ', name: '  my preset  ' })
    expect(clientMock.renamePreset).toHaveBeenCalledWith('pr1', 'my preset')
  })
  it('name omitted/empty string → null (clear the override)', () => {
    run('preset.rename', { id: 'pr1' })
    expect(clientMock.renamePreset).toHaveBeenCalledWith('pr1', null)
    clientMock.renamePreset.mockClear()
    run('preset.rename', { id: 'pr1', name: '   ' })
    expect(clientMock.renamePreset).toHaveBeenCalledWith('pr1', null)
  })
  it('empty id → throw (no silent no-op)', () => {
    expect(() => run('preset.rename', { name: 'x' })).toThrow(/id is empty/)
    expect(clientMock.renamePreset).not.toHaveBeenCalled()
  })
})

describe('preset.delete routing (regression guard for the existing wiring)', () => {
  it('id → deletePreset(trimmed id)', () => {
    run('preset.delete', { id: '  pr1  ' })
    expect(clientMock.deletePreset).toHaveBeenCalledWith('pr1')
  })
  it('empty id → throw', () => {
    expect(() => run('preset.delete', {})).toThrow(/id is empty/)
  })
})
