import { describe, expect, it } from 'vitest'

import { matchDeclaredSpelling } from './enumArg'

const DECLARED = ['Terminal', 'StreamJson'] as const

describe('matchDeclaredSpelling', () => {
  it('returns an exact match as is', () => {
    expect(matchDeclaredSpelling('Terminal', DECLARED)).toBe('Terminal')
    expect(matchDeclaredSpelling('StreamJson', DECLARED)).toBe('StreamJson')
  })

  // ★Key★: the return is the declared spelling, not the string the caller gave — this is what bears the byte-identity of the wire value.
  it('folds to the declared spelling even when the case differs', () => {
    expect(matchDeclaredSpelling('terminal', DECLARED)).toBe('Terminal')
    expect(matchDeclaredSpelling('TERMINAL', DECLARED)).toBe('Terminal')
    expect(matchDeclaredSpelling('streamjson', DECLARED)).toBe('StreamJson')
    expect(matchDeclaredSpelling('STREAMJSON', DECLARED)).toBe('StreamJson')
    expect(matchDeclaredSpelling('sTrEaMjSoN', DECLARED)).toBe('StreamJson')
  })

  it('unknown word · non-string value → undefined (the valid set does not widen)', () => {
    expect(matchDeclaredSpelling('Termnal', DECLARED)).toBeUndefined()
    expect(matchDeclaredSpelling('stream_json', DECLARED)).toBeUndefined()
    expect(matchDeclaredSpelling('', DECLARED)).toBeUndefined()
    expect(matchDeclaredSpelling(7, DECLARED)).toBeUndefined()
    expect(matchDeclaredSpelling(undefined, DECLARED)).toBeUndefined()
    expect(matchDeclaredSpelling(null, DECLARED)).toBeUndefined()
    expect(matchDeclaredSpelling({ toString: () => 'Terminal' }, DECLARED)).toBeUndefined()
  })

  // Whitespace and surrounding characters are not trimmed — the only axis folded is letter case.
  it('does not trim', () => {
    expect(matchDeclaredSpelling(' Terminal', DECLARED)).toBeUndefined()
    expect(matchDeclaredSpelling('Terminal ', DECLARED)).toBeUndefined()
  })

  // Even on the day two spellings that differ only in case get declared, a caller who writes it exactly receives their own spelling.
  it('an exact match comes before folding', () => {
    const ambiguous = ['id', 'ID'] as const
    expect(matchDeclaredSpelling('ID', ambiguous)).toBe('ID')
    expect(matchDeclaredSpelling('id', ambiguous)).toBe('id')
  })
})
