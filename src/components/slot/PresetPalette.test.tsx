// store 는 실제 useAgentStore 를 setState 로 seed — onPresetListUpdated → setPresets 반영과 동일 경로.

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
// refreshPresets = delete/rename 성공 후 권위 목록 재적용 안전망(broadcast 유실 대비) — 호출을 검증한다.
//   eventBus 실모듈 로드(agentClient/registry 등 부작용)를 피하려 mock 한다.
const refreshPresetsMock = vi.hoisted(() => vi.fn(async () => undefined))
vi.mock('../../store/eventBus', () => ({ refreshPresets: refreshPresetsMock }))

import PresetPalette, { presetDisplayName } from './PresetPalette'
import { useAgentStore } from '../../store/agentStore'
import type { Preset } from '../../api/types'

beforeEach(() => {
  clientMock.createPreset.mockClear()
  clientMock.deletePreset.mockClear()
  clientMock.renamePreset.mockClear()
  refreshPresetsMock.mockClear()
  useAgentStore.setState({ presets: [] })
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

describe('presetDisplayName (name override ?? basename 파생 — ADR-0061 리치화)', () => {
  it('name override 있으면 그대로(basename 파생 무시)', () => {
    expect(presetDisplayName({ cwd: '/home/me/project', name: '내 프리셋' })).toBe('내 프리셋')
    expect(presetDisplayName({ cwd: 'C:\\work\\engram', name: '작업' })).toBe('작업')
  })
  it('name=null → cwd basename 파생(POSIX)', () => {
    expect(presetDisplayName(withCwd('/home/me/project'))).toBe('project')
  })
  it('name=null → cwd basename 파생(Windows)', () => {
    expect(presetDisplayName(withCwd('C:\\work\\engram'))).toBe('engram')
  })
  it('후행 구분자 무시(trailing separator)', () => {
    expect(presetDisplayName(withCwd('C:/proj/'))).toBe('proj')
    expect(presetDisplayName(withCwd('/a/b/c/'))).toBe('c')
  })
  it('세그먼트 없음(루트 등) → cwd 원본 fallback', () => {
    expect(presetDisplayName(withCwd('/'))).toBe('/')
    expect(presetDisplayName(withCwd('projectonly'))).toBe('projectonly')
  })
  it('drive-root(C:\\ / C:/) → raw cwd 유지("C:" 로 붕괴 금지)', () => {
    expect(presetDisplayName(withCwd('C:\\'))).toBe('C:\\')
    expect(presetDisplayName(withCwd('C:/'))).toBe('C:/')
    expect(presetDisplayName(withCwd('C:'))).toBe('C:') // 구분자 없는 drive-only 도 misleading 세그먼트 방지
  })
  it('빈/공백-only cwd(name=null) → blank 라벨 방지 placeholder(비어있지 않은 안정적 문자열)', () => {
    // 라벨은 이 반환값 하나로만 그려지므로 blank 면 행이 빈 칸으로 보인다 → placeholder 로 degrade.
    const emptyLabel = presetDisplayName(withCwd(''))
    expect(emptyLabel.trim().length).toBeGreaterThan(0)
    expect(emptyLabel).toBe('(경로 없음)')
    expect(presetDisplayName(withCwd('   ')).trim().length).toBeGreaterThan(0)
    expect(presetDisplayName(withCwd('   '))).toBe('(경로 없음)')
  })
  it('root-like 경로(/, UNC) → 잘못된 세그먼트로 붕괴하지 않음', () => {
    expect(presetDisplayName(withCwd('/'))).toBe('/')
    // UNC share 는 마지막 세그먼트가 의미 있으므로 basename 파생 허용.
    expect(presetDisplayName(withCwd('\\\\server\\share'))).toBe('share')
    expect(presetDisplayName(withCwd('\\\\server\\share\\'))).toBe('share')
  })
})

describe('PresetPalette 렌더', () => {
  it('빈 목록 → 안내 문구', () => {
    render(<PresetPalette />)
    expect(screen.getByText(/프리셋 없음/)).toBeTruthy()
  })

  it('store.presets 를 행으로 렌더 + 표시명 = cwd basename(name override 없음)', () => {
    seedPresets(preset('pr1', 'C:/work/engram'), preset('pr2', '/home/me/proj'))
    render(<PresetPalette />)
    expect(screen.getByText('engram')).toBeTruthy()
    expect(screen.getByText('proj')).toBeTruthy()
    expect(document.querySelector('[data-preset-id="pr1"]')).toBeTruthy()
    expect(document.querySelector('[data-preset-id="pr2"]')).toBeTruthy()
  })

  it('name override 있는 프리셋 → basename 대신 override 표시(ADR-0061 리치화)', () => {
    seedPresets(preset('pr1', 'C:/work/engram', '내 작업'))
    render(<PresetPalette />)
    expect(screen.getByText('내 작업')).toBeTruthy()
    expect(screen.queryByText('engram')).toBeNull()
  })

  it('빈 cwd 프리셋 행도 blank 라벨을 그리지 않는다(placeholder 표시)', () => {
    seedPresets(preset('pr-empty', ''))
    render(<PresetPalette />)
    const nameEl = document.querySelector('[data-preset-id="pr-empty"] [data-preset-name]') as HTMLElement
    expect(nameEl).toBeTruthy()
    expect((nameEl.textContent ?? '').trim().length).toBeGreaterThan(0)
  })

  it('탑바 텍스트 입력·추가 버튼은 제거됨(통합 슬롯 메뉴로 대체)', () => {
    render(<PresetPalette />)
    expect(document.querySelector('[data-preset-input]')).toBeNull()
    expect(document.querySelector('[data-preset-add]')).toBeNull()
  })

  it('★pane 우클릭 자체 메뉴 없음(ADR-0064)★ — 우클릭해도 옛 "추가" 메뉴가 뜨지 않는다(통합 메뉴로 이전)', () => {
    render(<PresetPalette />)
    const pane = document.querySelector('[data-preset-palette]') as HTMLElement
    fireEvent.contextMenu(pane)
    // "추가" 커버리지는 통합 슬롯 메뉴의 preset.add command 쪽(presetCommands.test) 담당.
    expect(document.querySelector('[data-preset-menu-add]')).toBeNull()
    expect(clientMock.createPreset).not.toHaveBeenCalled()
  })

  it('행 우클릭 메뉴 "삭제" → deletePreset(id) 호출(삭제는 이제 메뉴 안, ADR-0061)', () => {
    seedPresets(preset('pr1', 'C:/work/engram'))
    render(<PresetPalette />)
    expect(document.querySelector('[data-preset-delete="pr1"]')).toBeNull()
    fireEvent.contextMenu(document.querySelector('[data-preset-id="pr1"]') as HTMLElement)
    fireEvent.click(screen.getByText('삭제'))
    expect(clientMock.deletePreset).toHaveBeenCalledWith('pr1')
  })

  it('행 우클릭 메뉴 "이름 변경" → 인라인 입력 → Enter 확정 → renamePreset(id, trimmed)', () => {
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

  it('이름 변경 Esc → renamePreset 미발화(revert)', () => {
    seedPresets(preset('pr1', 'C:/work/engram'))
    render(<PresetPalette />)
    fireEvent.contextMenu(document.querySelector('[data-preset-id="pr1"]') as HTMLElement)
    fireEvent.click(screen.getByText('이름 변경'))
    const input = document.querySelector('[data-preset-rename-input="pr1"]') as HTMLInputElement
    fireEvent.change(input, { target: { value: '바뀐이름' } })
    fireEvent.keyDown(input, { key: 'Escape' })
    expect(clientMock.renamePreset).not.toHaveBeenCalled()
    expect(document.querySelector('[data-preset-rename-input="pr1"]')).toBeNull() // 편집 종료
  })

  it('이름 변경 미변경(현재 표시명과 동일) → renamePreset 미발화', () => {
    seedPresets(preset('pr1', 'C:/work/engram', '고정'))
    render(<PresetPalette />)
    fireEvent.contextMenu(document.querySelector('[data-preset-id="pr1"]') as HTMLElement)
    fireEvent.click(screen.getByText('이름 변경'))
    const input = document.querySelector('[data-preset-rename-input="pr1"]') as HTMLInputElement
    fireEvent.keyDown(input, { key: 'Enter' }) // draft = 시드된 '고정' 그대로
    expect(clientMock.renamePreset).not.toHaveBeenCalled()
  })

  it('스타일 = 변수-only(하드코딩 색 없음) — 루트 background 가 var(...) 참조', () => {
    render(<PresetPalette />)
    const root = document.querySelector('[data-preset-palette]') as HTMLElement
    // e-ink 테마 준수 — 하드코딩 색 리터럴이면 위반.
    expect(root.style.background).toContain('var(')
  })

  // ── 증상4 회귀 안전망(프리셋 트윈): 한 프리셋 rename 이 형제 행을 떨어뜨리지 않는다 ──────────
  //   AgentList 와 동형 — 낙관 갱신 없이 store.presets 전체를 그대로 두므로 형제 행이 사라지면 안 되고,
  //   rename/delete 성공은 refreshPresets(권위 목록 재적용 안전망)로 이어져야 한다(broadcast 유실 대비 대칭).
  it('3프리셋 중 하나 rename → 나머지 2행 유지 + refreshPresets 호출', async () => {
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

  it('delete 성공 → refreshPresets(권위 목록 재적용 안전망) 호출', async () => {
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
