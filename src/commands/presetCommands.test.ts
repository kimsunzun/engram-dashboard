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
// preset.add 가 네이티브 폴더 다이얼로그를 부른다 — import 부수효과로 register 만 하므로 open 은 no-op mock.
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(async () => null) }))
// slotMenu registerSlotMenu 부수효과는 무해하나, registry 중복 등록 회피를 위해 side-effect import 는 1회만.

import './presetCommands' // side-effect register
import { run } from './registry'

beforeEach(() => {
  clientMock.createPreset.mockClear()
  clientMock.deletePreset.mockClear()
  clientMock.renamePreset.mockClear()
})

describe('preset.rename 라우팅(§5 — ADR-0061 리치화)', () => {
  it('id + name → renamePreset(id, trimmed)', () => {
    run('preset.rename', { id: '  pr1  ', name: '  내 프리셋  ' })
    expect(clientMock.renamePreset).toHaveBeenCalledWith('pr1', '내 프리셋')
  })
  it('name 생략/빈문자열 → null(override 해제)', () => {
    run('preset.rename', { id: 'pr1' })
    expect(clientMock.renamePreset).toHaveBeenCalledWith('pr1', null)
    clientMock.renamePreset.mockClear()
    run('preset.rename', { id: 'pr1', name: '   ' })
    expect(clientMock.renamePreset).toHaveBeenCalledWith('pr1', null)
  })
  it('빈 id → throw(조용한 no-op 금지)', () => {
    expect(() => run('preset.rename', { name: 'x' })).toThrow(/id 가 비어 있음/)
    expect(clientMock.renamePreset).not.toHaveBeenCalled()
  })
})

describe('preset.delete 라우팅(기존 배선 회귀 가드)', () => {
  it('id → deletePreset(trim 된 id)', () => {
    run('preset.delete', { id: '  pr1  ' })
    expect(clientMock.deletePreset).toHaveBeenCalledWith('pr1')
  })
  it('빈 id → throw', () => {
    expect(() => run('preset.delete', {})).toThrow(/id 가 비어 있음/)
  })
})
