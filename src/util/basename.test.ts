// ADR-0061: the cwd basename derivation rule. The single source of PresetPalette · AgentList display names.

import { describe, expect, it } from 'vitest'

import { setLanguage, t } from '../i18n'
import { basename, PATH_NAME_PLACEHOLDER } from './basename'

describe('basename (cwd display name derivation)', () => {
  it('POSIX path basename', () => {
    expect(basename('/home/me/project')).toBe('project')
  })
  it('Windows path basename', () => {
    expect(basename('C:\\work\\engram')).toBe('engram')
  })
  it('ignores a trailing separator', () => {
    expect(basename('C:/proj/')).toBe('proj')
    expect(basename('/a/b/c/')).toBe('c')
  })
  it('no segments (root etc.) → fallback to the raw cwd', () => {
    expect(basename('/')).toBe('/')
    expect(basename('projectonly')).toBe('projectonly')
  })
  it('drive-root (C:\\ / C:/) → keeps the raw cwd (no collapse to "C:")', () => {
    expect(basename('C:\\')).toBe('C:\\')
    expect(basename('C:/')).toBe('C:/')
    expect(basename('C:')).toBe('C:')
  })
  it('empty/blank-only cwd → placeholder (a stable non-empty string)', () => {
    expect(basename('')).toBe(t(PATH_NAME_PLACEHOLDER))
    expect(basename('   ')).toBe(t(PATH_NAME_PLACEHOLDER))
    expect(basename('').trim().length).toBeGreaterThan(0)
  })
  it('a UNC share derives from the last segment', () => {
    expect(basename('\\\\server\\share')).toBe('share')
    expect(basename('\\\\server\\share\\')).toBe('share')
  })
})

// ADR-0373: the blank-path name is resolved on each call, so it follows the UI language.
describe('basename — blank path in the current UI language', () => {
  it('follows a language switch', () => {
    try {
      expect(basename('')).toBe('(경로 없음)')
      setLanguage('en')
      expect(basename('')).toBe('(no path)')
    } finally {
      setLanguage('ko')
    }
  })
})
