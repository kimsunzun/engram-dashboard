// basename 단위테스트 — cwd basename 파생 규칙(ADR-0061). PresetPalette·AgentList 표시명 단일 출처.
//
// 검증: win/posix 구분자·후행 슬래시·root/drive-root/UNC 엣지·빈/공백 placeholder.

import { describe, expect, it } from 'vitest'

import { basename, PATH_NAME_PLACEHOLDER } from './basename'

describe('basename (cwd 표시명 파생)', () => {
  it('POSIX 경로 basename', () => {
    expect(basename('/home/me/project')).toBe('project')
  })
  it('Windows 경로 basename', () => {
    expect(basename('C:\\work\\engram')).toBe('engram')
  })
  it('후행 구분자 무시', () => {
    expect(basename('C:/proj/')).toBe('proj')
    expect(basename('/a/b/c/')).toBe('c')
  })
  it('세그먼트 없음(루트 등) → cwd 원본 fallback', () => {
    expect(basename('/')).toBe('/')
    expect(basename('projectonly')).toBe('projectonly')
  })
  it('drive-root(C:\\ / C:/) → raw cwd 유지("C:" 붕괴 금지)', () => {
    expect(basename('C:\\')).toBe('C:\\')
    expect(basename('C:/')).toBe('C:/')
    expect(basename('C:')).toBe('C:')
  })
  it('빈/공백-only cwd → placeholder(비어있지 않은 안정적 문자열)', () => {
    expect(basename('')).toBe(PATH_NAME_PLACEHOLDER)
    expect(basename('   ')).toBe(PATH_NAME_PLACEHOLDER)
    expect(basename('').trim().length).toBeGreaterThan(0)
  })
  it('UNC share 는 마지막 세그먼트 파생', () => {
    expect(basename('\\\\server\\share')).toBe('share')
    expect(basename('\\\\server\\share\\')).toBe('share')
  })
})
