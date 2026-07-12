// PresetPalette 렌더 스모크 + 행 우클릭 메뉴(이름변경·삭제) 배선 테스트(ADR-0060/0061/0064 — 리치화).
//
// 검증 불변식:
//   1. presetDisplayName: name override ?? cwd basename 파생(win/posix 구분자·후행 슬래시·빈 세그먼트).
//   2. store.presets 를 행으로 렌더 + 표시명 = override ?? basename.
//   3. ★pane 우클릭 메뉴 없음(ADR-0064)★: 옛 pane "추가" 메뉴는 제거됐다(추가 = 통합 슬롯 메뉴의 preset.add
//      command 로 이전). PresetPalette 는 라벨/목록/행 우클릭 메뉴(이름변경·삭제)만 소유.
//   4. 행 우클릭 메뉴 "삭제" → agentClient.deletePreset(id); "이름 변경" → 인라인 편집 → renamePreset(id, name).
//   5. 스타일 = 변수-only(하드코딩 색 리터럴 없음) — 대표 요소 background 가 var(...) 참조.
//
// 전략: agentClient(clientFactory) 의 프리셋 메서드 mock. store 는 실제 useAgentStore 를 setState 로 seed
//   (= onPresetListUpdated → setPresets 반영과 동일 경로).

import { cleanup, fireEvent, render, screen } from '@testing-library/react'
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

import PresetPalette, { presetDisplayName } from './PresetPalette'
import { useAgentStore } from '../../store/agentStore'
import type { Preset } from '../../api/types'

beforeEach(() => {
  clientMock.createPreset.mockClear()
  clientMock.deletePreset.mockClear()
  clientMock.renamePreset.mockClear()
  useAgentStore.setState({ presets: [] })
})

afterEach(() => {
  cleanup()
  useAgentStore.setState({ presets: [] })
})

function seedPresets(...presets: Preset[]): void {
  useAgentStore.setState({ presets })
}

/** Preset factory — name override 는 기본 null(basename 파생). 리치화 후 필수 필드를 한 곳에 채운다. */
function preset(id: string, cwd: string, name: string | null = null): Preset {
  return { id, cwd, name }
}

// ★signature 변경(ADR-0061 리치화)★: presetDisplayName 은 이제 preset 객체({cwd, name})를 받아
//   name override 가 있으면 그대로, 없으면(null) cwd basename 을 파생한다(precedence). name=null 케이스는
//   기존 basename 파생 규칙(win/posix·root 엣지)을 그대로 검증한다.
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
    expect(emptyLabel.trim().length).toBeGreaterThan(0) // 절대 blank 아님
    expect(emptyLabel).toBe('(경로 없음)')
    // 공백-only 도 동일 fallback.
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
    // name override 없음 → basename 으로 파생(cwd 전문이 아니라).
    expect(screen.getByText('engram')).toBeTruthy()
    expect(screen.getByText('proj')).toBeTruthy()
    // 두 행 모두 마운트(data-preset-id 로 식별).
    expect(document.querySelector('[data-preset-id="pr1"]')).toBeTruthy()
    expect(document.querySelector('[data-preset-id="pr2"]')).toBeTruthy()
  })

  it('name override 있는 프리셋 → basename 대신 override 표시(ADR-0061 리치화)', () => {
    seedPresets(preset('pr1', 'C:/work/engram', '내 작업'))
    render(<PresetPalette />)
    expect(screen.getByText('내 작업')).toBeTruthy()
    // basename('engram')은 표시되지 않아야 함(override 우선).
    expect(screen.queryByText('engram')).toBeNull()
  })

  it('빈 cwd 프리셋 행도 blank 라벨을 그리지 않는다(placeholder 표시)', () => {
    seedPresets(preset('pr-empty', ''))
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

  it('행 우클릭 메뉴 "삭제" → deletePreset(id) 호출(삭제는 이제 메뉴 안, ADR-0061)', () => {
    seedPresets(preset('pr1', 'C:/work/engram'))
    render(<PresetPalette />)
    // 옛 인라인 삭제 버튼은 제거됨(메뉴로 이동).
    expect(document.querySelector('[data-preset-delete="pr1"]')).toBeNull()
    // 행 우클릭 → 메뉴 → "삭제".
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
    // trim 후 값으로 renamePreset 발화(§5 백엔드 저장 — 낙관 갱신 X).
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
    // override 가 이미 '고정' → 같은 값으로 확정하면 발화 안 함(불필요 command 억제).
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
    // background 가 CSS 변수 참조 형태여야 한다(e-ink 테마 준수 — 하드코딩 색 리터럴이면 위반).
    expect(root.style.background).toContain('var(')
  })
})
