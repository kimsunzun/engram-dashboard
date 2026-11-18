// The store seeds the real useAgentStore with setState — the same path as onPresetListUpdated → setPresets reflection.

import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const clientMock = vi.hoisted(() => ({
  createPreset: vi.fn(async () => undefined),
  deletePreset: vi.fn(async () => undefined),
  renamePreset: vi.fn(async () => undefined),
}))
vi.mock('../../api/clientFactory', () => ({
  agentClient: {
    createPreset: (...args: unknown[]) => clientMock.createPreset(...(args as [])),
    deletePreset: (...args: unknown[]) => clientMock.deletePreset(...(args as [])),
    renamePreset: (...args: unknown[]) => clientMock.renamePreset(...(args as [])),
  },
  getAgentClient: vi.fn(),
}))
// refreshPresets = the safety net re-applying the authoritative list after delete/rename succeed (against a lost broadcast) — the call is verified; mocked to avoid loading the real eventBus module (agentClient/registry side effects).
const refreshPresetsMock = vi.hoisted(() => vi.fn(async () => undefined))
vi.mock('../../store/eventBus', () => ({ refreshPresets: refreshPresetsMock }))

import PresetPalette, { presetDisplayName } from './PresetPalette'
import { OVERLAY_SELECTOR } from './interruptKey'
import { useAgentStore } from '../../store/agentStore'
import type { Preset } from '../../api/types'
import { setLanguage } from '../../i18n'

beforeEach(() => {
  clientMock.createPreset.mockClear()
  clientMock.deletePreset.mockClear()
  clientMock.renamePreset.mockClear()
  refreshPresetsMock.mockClear()
  // Default is 「list received」 — a test for the not-received span lowers the flag itself.
  useAgentStore.setState({ presets: [], presetsLoaded: true })
})

afterEach(() => {
  cleanup()
  useAgentStore.setState({ presets: [] })
})

function seedPresets(...presets: Preset[]): void {
  useAgentStore.setState({ presets })
}

function preset(id: string, cwd: string, name: string | null = null): Preset {
  return { id, cwd, name }
}

const withCwd = (cwd: string) => ({ cwd, name: null })

describe('presetDisplayName (name override ?? basename derivation — ADR-0061 enrichment)', () => {
  it('with a name override it is used as is (basename derivation ignored)', () => {
    expect(presetDisplayName({ cwd: '/home/me/project', name: '내 프리셋' })).toBe('내 프리셋')
    expect(presetDisplayName({ cwd: 'C:\\work\\engram', name: '작업' })).toBe('작업')
  })
  it('name=null → cwd basename derivation (POSIX)', () => {
    expect(presetDisplayName(withCwd('/home/me/project'))).toBe('project')
  })
  it('name=null → cwd basename derivation (Windows)', () => {
    expect(presetDisplayName(withCwd('C:\\work\\engram'))).toBe('engram')
  })
  it('trailing separator ignored', () => {
    expect(presetDisplayName(withCwd('C:/proj/'))).toBe('proj')
    expect(presetDisplayName(withCwd('/a/b/c/'))).toBe('c')
  })
  it('no segment (root etc.) → falls back to the raw cwd', () => {
    expect(presetDisplayName(withCwd('/'))).toBe('/')
    expect(presetDisplayName(withCwd('projectonly'))).toBe('projectonly')
  })
  it('drive-root (C:\\ / C:/) → raw cwd kept (must not collapse to "C:")', () => {
    expect(presetDisplayName(withCwd('C:\\'))).toBe('C:\\')
    expect(presetDisplayName(withCwd('C:/'))).toBe('C:/')
    expect(presetDisplayName(withCwd('C:'))).toBe('C:') // even a separator-less drive-only avoids a misleading segment
  })
  it('empty/whitespace-only cwd (name=null) → a placeholder preventing a blank label (a non-empty stable string)', () => {
    // The label is drawn from this return value alone, so if blank the row looks like an empty cell → degrade to a placeholder.
    const emptyLabel = presetDisplayName(withCwd(''))
    expect(emptyLabel.trim().length).toBeGreaterThan(0)
    expect(emptyLabel).toBe('(경로 없음)')
    expect(presetDisplayName(withCwd('   ')).trim().length).toBeGreaterThan(0)
    expect(presetDisplayName(withCwd('   '))).toBe('(경로 없음)')
  })
  it('root-like paths (/, UNC) → do not collapse into a wrong segment', () => {
    expect(presetDisplayName(withCwd('/'))).toBe('/')
    // A UNC share has a meaningful last segment so basename derivation is allowed.
    expect(presetDisplayName(withCwd('\\\\server\\share'))).toBe('share')
    expect(presetDisplayName(withCwd('\\\\server\\share\\'))).toBe('share')
  })
})

describe('PresetPalette render', () => {
  it('empty list → notice text', () => {
    render(<PresetPalette />)
    expect(screen.getByText(/프리셋 없음/)).toBeTruthy()
  })

  it('before the first list is received → the empty-list notice is not drawn, and when an empty list is received it is drawn then', () => {
    useAgentStore.setState({ presetsLoaded: false })
    render(<PresetPalette />)
    expect(screen.queryByText(/프리셋 없음/)).toBeNull()
    act(() => useAgentStore.getState().setPresets([]))
    expect(screen.getByText(/프리셋 없음/)).toBeTruthy()
  })

  it('store.presets render as rows + display name = cwd basename (no name override)', () => {
    seedPresets(preset('pr1', 'C:/work/engram'), preset('pr2', '/home/me/proj'))
    render(<PresetPalette />)
    expect(screen.getByText('engram')).toBeTruthy()
    expect(screen.getByText('proj')).toBeTruthy()
    expect(document.querySelector('[data-preset-id="pr1"]')).toBeTruthy()
    expect(document.querySelector('[data-preset-id="pr2"]')).toBeTruthy()
  })

  it('a preset with a name override → the override is shown instead of basename (ADR-0061 enrichment)', () => {
    seedPresets(preset('pr1', 'C:/work/engram', '내 작업'))
    render(<PresetPalette />)
    expect(screen.getByText('내 작업')).toBeTruthy()
    expect(screen.queryByText('engram')).toBeNull()
  })

  it('a preset row with an empty cwd also draws no blank label (placeholder shown)', () => {
    seedPresets(preset('pr-empty', ''))
    render(<PresetPalette />)
    const nameEl = document.querySelector('[data-preset-id="pr-empty"] [data-preset-name]') as HTMLElement
    expect(nameEl).toBeTruthy()
    expect((nameEl.textContent ?? '').trim().length).toBeGreaterThan(0)
  })

  it('the top-bar text input and add button are removed (replaced by the unified slot menu)', () => {
    render(<PresetPalette />)
    expect(document.querySelector('[data-preset-input]')).toBeNull()
    expect(document.querySelector('[data-preset-add]')).toBeNull()
  })

  it('★no own pane right-click menu (ADR-0064)★ — right-clicking does not bring up the old "add" menu (moved to the unified menu)', () => {
    render(<PresetPalette />)
    const pane = document.querySelector('[data-preset-palette]') as HTMLElement
    fireEvent.contextMenu(pane)
    // "add" coverage belongs to the preset.add command of the unified slot menu (presetCommands.test).
    expect(document.querySelector('[data-preset-menu-add]')).toBeNull()
    expect(clientMock.createPreset).not.toHaveBeenCalled()
  })

  it('row right-click menu "delete" → calls deletePreset(id) (delete is now inside the menu, ADR-0061)', () => {
    seedPresets(preset('pr1', 'C:/work/engram'))
    render(<PresetPalette />)
    expect(document.querySelector('[data-preset-delete="pr1"]')).toBeNull()
    fireEvent.contextMenu(document.querySelector('[data-preset-id="pr1"]') as HTMLElement)
    fireEvent.click(screen.getByText('삭제'))
    expect(clientMock.deletePreset).toHaveBeenCalledWith('pr1')
  })

  it('the row menu carries the overlay marker only while open (ADR-0237) — it disappears with it when a document Esc closes it', () => {
    seedPresets(preset('pr1', 'C:/work/engram'))
    render(<PresetPalette />)
    expect(document.querySelector(OVERLAY_SELECTOR)).toBeNull()
    fireEvent.contextMenu(document.querySelector('[data-preset-id="pr1"]') as HTMLElement)
    expect(document.querySelector(OVERLAY_SELECTOR)?.textContent).toContain('삭제')
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(document.querySelector(OVERLAY_SELECTOR)).toBeNull()
  })

  it('row right-click menu "rename" → inline input → Enter commit → renamePreset(id, trimmed)', () => {
    seedPresets(preset('pr1', 'C:/work/engram'))
    render(<PresetPalette />)
    fireEvent.contextMenu(document.querySelector('[data-preset-id="pr1"]') as HTMLElement)
    fireEvent.click(screen.getByText('이름 변경'))
    const input = document.querySelector('[data-preset-rename-input="pr1"]') as HTMLInputElement
    expect(input).toBeTruthy()
    fireEvent.change(input, { target: { value: '  새 이름  ' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(clientMock.renamePreset).toHaveBeenCalledWith('pr1', '새 이름')
  })

  it('rename Esc → renamePreset not fired (revert)', () => {
    seedPresets(preset('pr1', 'C:/work/engram'))
    render(<PresetPalette />)
    fireEvent.contextMenu(document.querySelector('[data-preset-id="pr1"]') as HTMLElement)
    fireEvent.click(screen.getByText('이름 변경'))
    const input = document.querySelector('[data-preset-rename-input="pr1"]') as HTMLInputElement
    fireEvent.change(input, { target: { value: '바뀐이름' } })
    fireEvent.keyDown(input, { key: 'Escape' })
    expect(clientMock.renamePreset).not.toHaveBeenCalled()
    expect(document.querySelector('[data-preset-rename-input="pr1"]')).toBeNull() // edit ended
  })

  // ADR-0373 `draftStart` (the same rule as TabBar's seed): the baseline is the name the edit started from.
  it('the name changes from outside while editing, then Enter without editing → no renamePreset', () => {
    seedPresets(preset('pr1', 'C:/work/engram', '고정'))
    render(<PresetPalette />)
    fireEvent.contextMenu(document.querySelector('[data-preset-id="pr1"]') as HTMLElement)
    fireEvent.click(screen.getByText('이름 변경'))
    act(() => {
      seedPresets(preset('pr1', 'C:/work/engram', '바깥에서 바꾼 이름'))
    })
    const input = document.querySelector('[data-preset-rename-input="pr1"]') as HTMLInputElement
    expect(input.value).toBe('고정')
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(clientMock.renamePreset).not.toHaveBeenCalled()
  })

  it('rename unchanged (same as the current display name) → renamePreset not fired', () => {
    seedPresets(preset('pr1', 'C:/work/engram', '고정'))
    render(<PresetPalette />)
    fireEvent.contextMenu(document.querySelector('[data-preset-id="pr1"]') as HTMLElement)
    fireEvent.click(screen.getByText('이름 변경'))
    const input = document.querySelector('[data-preset-rename-input="pr1"]') as HTMLInputElement
    fireEvent.keyDown(input, { key: 'Enter' }) // draft = the seeded fixed name as is
    expect(clientMock.renamePreset).not.toHaveBeenCalled()
  })

  // Tauri `drag.js` blocks the default action (focus move) of pressing the window-drag area. Imitated by a listener registered earlier on the document bubble phase that also cuts later listeners (jsdom moves no focus on a press).
  it('pressing the window-drag area (title row) during rename → commits like losing focus · pressing outside the drag area keeps the edit', () => {
    const grab = document.createElement('div')
    grab.setAttribute('data-tauri-drag-region', '')
    document.body.appendChild(grab)
    const dragJs = (e: MouseEvent) => {
      if (e.target !== grab) return
      e.preventDefault()
      e.stopImmediatePropagation()
    }
    document.addEventListener('mousedown', dragJs)
    try {
      seedPresets(preset('pr1', 'C:/work/engram'))
      render(<PresetPalette />)
      fireEvent.contextMenu(document.querySelector('[data-preset-id="pr1"]') as HTMLElement)
      fireEvent.click(screen.getByText('이름 변경'))
      const input = document.querySelector('[data-preset-rename-input="pr1"]') as HTMLInputElement
      expect(document.activeElement).toBe(input)
      fireEvent.change(input, { target: { value: '  새 이름  ' } })

      fireEvent.mouseDown(document.body)
      expect(document.activeElement).toBe(input)
      expect(clientMock.renamePreset).not.toHaveBeenCalled()

      fireEvent.mouseDown(grab)
      expect(clientMock.renamePreset).toHaveBeenCalledWith('pr1', '새 이름')
      expect(document.querySelector('[data-preset-rename-input="pr1"]')).toBeNull()
    } finally {
      document.removeEventListener('mousedown', dragJs)
      grab.remove()
    }
  })

  it('style = variables only (no hard-coded color) — the root background is a var(...) reference', () => {
    render(<PresetPalette />)
    const root = document.querySelector('[data-preset-palette]') as HTMLElement
    // Compliant with the e-ink theme — a hard-coded color literal is a violation.
    expect(root.style.background).toContain('var(')
  })

  // ── Symptom 4 regression safety net (preset twin): renaming one preset does not drop sibling rows ──
  //   Same as AgentList — store.presets is left whole (no optimistic update) so sibling rows must not vanish, and rename/delete success must continue to refreshPresets (authoritative-list re-apply — symmetric against a lost broadcast).
  it('rename one of 3 presets → the other 2 rows stay + refreshPresets is called', async () => {
    seedPresets(preset('pr1', 'C:/a'), preset('pr2', 'C:/b'), preset('pr3', 'C:/c'))
    render(<PresetPalette />)
    expect(document.querySelectorAll('[data-preset-id]').length).toBe(3)

    fireEvent.contextMenu(document.querySelector('[data-preset-id="pr2"]') as HTMLElement)
    fireEvent.click(screen.getByText('이름 변경'))
    const input = document.querySelector('[data-preset-rename-input="pr2"]') as HTMLInputElement
    fireEvent.change(input, { target: { value: '새이름' } })
    await act(async () => {
      fireEvent.keyDown(input, { key: 'Enter' })
      await Promise.resolve() // renamePreset resolve → .then(refreshPresets) microtask flush
    })

    expect(clientMock.renamePreset).toHaveBeenCalledWith('pr2', '새이름')
    expect(document.querySelector('[data-preset-id="pr1"]')).toBeTruthy()
    expect(document.querySelector('[data-preset-id="pr2"]')).toBeTruthy()
    expect(document.querySelector('[data-preset-id="pr3"]')).toBeTruthy()
    expect(document.querySelectorAll('[data-preset-id]').length).toBe(3)
    expect(refreshPresetsMock).toHaveBeenCalledTimes(1)
  })

  it('delete success → refreshPresets (authoritative-list re-apply safety net) is called', async () => {
    seedPresets(preset('pr1', 'C:/a'))
    render(<PresetPalette />)
    fireEvent.contextMenu(document.querySelector('[data-preset-id="pr1"]') as HTMLElement)
    await act(async () => {
      fireEvent.click(screen.getByText('삭제'))
      await Promise.resolve() // deletePreset resolve → .then(refreshPresets) microtask flush
    })
    expect(clientMock.deletePreset).toHaveBeenCalledWith('pr1')
    expect(refreshPresetsMock).toHaveBeenCalledTimes(1)
  })
})

// ADR-0373: the "unchanged" check compares with the name the edit started from, not a name derived again in another language.
describe('PresetPalette — language switch', () => {
  afterEach(() => {
    act(() => {
      setLanguage('ko')
    })
  })

  it('a draft being edited is not saved by a switch (a blank-path name changes language under it)', () => {
    seedPresets(preset('pr1', ''))
    render(<PresetPalette />)
    fireEvent.contextMenu(document.querySelector('[data-preset-id="pr1"]') as HTMLElement)
    fireEvent.click(screen.getByText('이름 변경'))
    const input = document.querySelector('[data-preset-rename-input="pr1"]') as HTMLInputElement
    expect(input.value).toBe('(경로 없음)')

    act(() => {
      setLanguage('en')
    })
    fireEvent.keyDown(input, { key: 'Enter' })

    expect(clientMock.renamePreset).not.toHaveBeenCalled()
  })
})
