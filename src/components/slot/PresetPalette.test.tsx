// PresetPalette 렌더 스모크 + 프리셋 삭제 배선 테스트(ADR-0060/0061/0064).
//
// 검증 불변식:
//   1. presetDisplayName: cwd basename 파생(win/posix 구분자·후행 슬래시·빈 세그먼트).
//   2. store.presets 를 행으로 렌더 + 표시명 = basename(이름 미저장 — 프론트 파생).
//   3. ★pane 우클릭 메뉴 없음(ADR-0064)★: 옛 pane "추가" 메뉴는 제거됐다(추가 = 통합 슬롯 메뉴의 preset.add
//      command 로 이전 — presetCommands.test 가 배선 단언). PresetPalette 는 라벨/목록/삭제만 소유.
//   4. 행 삭제 → agentClient.deletePreset(id) 호출.
//   5. 스타일 = 변수-only(하드코딩 색 리터럴 없음) — 대표 요소 background 가 var(...) 참조.
//
// 전략: agentClient(clientFactory) 의 프리셋 메서드 mock. store 는 실제 useAgentStore 를 setState 로 seed
//   (= onPresetListUpdated → setPresets 반영과 동일 경로).

import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const clientMock = vi.hoisted(() => ({
  createPreset: vi.fn(async () => undefined),
  deletePreset: vi.fn(async () => undefined),
}))
vi.mock('../../api/clientFactory', () => ({
  agentClient: {
    createPreset: (...args: unknown[]) => clientMock.createPreset(...(args as [])),
    deletePreset: (...args: unknown[]) => clientMock.deletePreset(...(args as [])),
  },
  getAgentClient: vi.fn(),
}))

import PresetPalette, { presetDisplayName } from './PresetPalette'
import { useAgentStore } from '../../store/agentStore'
import type { Preset } from '../../api/types'

beforeEach(() => {
  clientMock.createPreset.mockClear()
  clientMock.deletePreset.mockClear()
  useAgentStore.setState({ presets: [] })
})

afterEach(() => {
  cleanup()
  useAgentStore.setState({ presets: [] })
})

function seedPresets(...presets: Preset[]): void {
  useAgentStore.setState({ presets })
}

describe('presetDisplayName (basename 파생 — ADR-0061)', () => {
  it('POSIX 경로 basename', () => {
    expect(presetDisplayName('/home/me/project')).toBe('project')
  })
  it('Windows 경로 basename', () => {
    expect(presetDisplayName('C:\\work\\engram')).toBe('engram')
  })
  it('후행 구분자 무시(trailing separator)', () => {
    expect(presetDisplayName('C:/proj/')).toBe('proj')
    expect(presetDisplayName('/a/b/c/')).toBe('c')
  })
  it('세그먼트 없음(루트 등) → cwd 원본 fallback', () => {
    expect(presetDisplayName('/')).toBe('/')
    expect(presetDisplayName('projectonly')).toBe('projectonly')
  })
  it('drive-root(C:\\ / C:/) → raw cwd 유지("C:" 로 붕괴 금지)', () => {
    expect(presetDisplayName('C:\\')).toBe('C:\\')
    expect(presetDisplayName('C:/')).toBe('C:/')
    expect(presetDisplayName('C:')).toBe('C:') // 구분자 없는 drive-only 도 misleading 세그먼트 방지
  })
  it('빈/공백-only cwd → blank 라벨 방지 placeholder(비어있지 않은 안정적 문자열)', () => {
    // 라벨은 이 반환값 하나로만 그려지므로 blank 면 행이 빈 칸으로 보인다 → placeholder 로 degrade.
    const emptyLabel = presetDisplayName('')
    expect(emptyLabel.trim().length).toBeGreaterThan(0) // 절대 blank 아님
    expect(emptyLabel).toBe('(경로 없음)')
    // 공백-only 도 동일 fallback.
    expect(presetDisplayName('   ').trim().length).toBeGreaterThan(0)
    expect(presetDisplayName('   ')).toBe('(경로 없음)')
  })
  it('root-like 경로(/, UNC) → 잘못된 세그먼트로 붕괴하지 않음', () => {
    expect(presetDisplayName('/')).toBe('/')
    // UNC share 는 마지막 세그먼트가 의미 있으므로 basename 파생 허용.
    expect(presetDisplayName('\\\\server\\share')).toBe('share')
    expect(presetDisplayName('\\\\server\\share\\')).toBe('share')
  })
  it('일반 경로(normal path) → basename', () => {
    expect(presetDisplayName('/home/me/project')).toBe('project')
    expect(presetDisplayName('C:\\work\\engram')).toBe('engram')
  })
})

describe('PresetPalette 렌더', () => {
  it('빈 목록 → 안내 문구', () => {
    render(<PresetPalette />)
    expect(screen.getByText(/프리셋 없음/)).toBeTruthy()
  })

  it('store.presets 를 행으로 렌더 + 표시명 = cwd basename(이름 미저장)', () => {
    seedPresets({ id: 'pr1', cwd: 'C:/work/engram' }, { id: 'pr2', cwd: '/home/me/proj' })
    render(<PresetPalette />)
    // 표시명은 basename 으로 파생(cwd 전문이 아니라).
    expect(screen.getByText('engram')).toBeTruthy()
    expect(screen.getByText('proj')).toBeTruthy()
    // 두 행 모두 마운트(data-preset-id 로 식별).
    expect(document.querySelector('[data-preset-id="pr1"]')).toBeTruthy()
    expect(document.querySelector('[data-preset-id="pr2"]')).toBeTruthy()
  })

  it('빈 cwd 프리셋 행도 blank 라벨을 그리지 않는다(placeholder 표시)', () => {
    seedPresets({ id: 'pr-empty', cwd: '' })
    render(<PresetPalette />)
    const nameEl = document.querySelector('[data-preset-id="pr-empty"] [data-preset-name]') as HTMLElement
    expect(nameEl).toBeTruthy()
    expect((nameEl.textContent ?? '').trim().length).toBeGreaterThan(0) // 행 라벨이 blank 가 아님
  })

  it('탑바 텍스트 입력·추가 버튼은 제거됨(통합 슬롯 메뉴로 대체)', () => {
    render(<PresetPalette />)
    // 옛 탑바 요소가 더 이상 존재하지 않아야 한다(회귀 방지).
    expect(document.querySelector('[data-preset-input]')).toBeNull()
    expect(document.querySelector('[data-preset-add]')).toBeNull()
  })

  it('★pane 우클릭 자체 메뉴 없음(ADR-0064)★ — 우클릭해도 옛 "추가" 메뉴가 뜨지 않는다(통합 메뉴로 이전)', () => {
    render(<PresetPalette />)
    const pane = document.querySelector('[data-preset-palette]') as HTMLElement
    fireEvent.contextMenu(pane)
    // 옛 pane 메뉴("추가")는 제거됨 — 추가는 이제 통합 슬롯 메뉴의 preset.add command(presetCommands.test 담당).
    expect(document.querySelector('[data-preset-menu-add]')).toBeNull()
    expect(clientMock.createPreset).not.toHaveBeenCalled()
  })

  it('행 삭제 → deletePreset(id) 호출', () => {
    seedPresets({ id: 'pr1', cwd: 'C:/work/engram' })
    render(<PresetPalette />)
    fireEvent.click(document.querySelector('[data-preset-delete="pr1"]') as HTMLElement)
    expect(clientMock.deletePreset).toHaveBeenCalledWith('pr1')
  })

  it('스타일 = 변수-only(하드코딩 색 없음) — 루트 background 가 var(...) 참조', () => {
    render(<PresetPalette />)
    const root = document.querySelector('[data-preset-palette]') as HTMLElement
    // background 가 CSS 변수 참조 형태여야 한다(e-ink 테마 준수 — 하드코딩 색 리터럴이면 위반).
    expect(root.style.background).toContain('var(')
  })
})
