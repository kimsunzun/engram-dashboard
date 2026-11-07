import { describe, expect, it } from 'vitest'

import { matchDeclaredSpelling } from './enumArg'

const DECLARED = ['Terminal', 'StreamJson'] as const

describe('matchDeclaredSpelling', () => {
  it('완전 일치는 그대로 돌려준다', () => {
    expect(matchDeclaredSpelling('Terminal', DECLARED)).toBe('Terminal')
    expect(matchDeclaredSpelling('StreamJson', DECLARED)).toBe('StreamJson')
  })

  // ★Key★: the return is the declared spelling, not the string the caller gave — this is what bears the byte-identity of the wire value.
  it('대소문자가 달라도 선언 철자로 접어 돌려준다', () => {
    expect(matchDeclaredSpelling('terminal', DECLARED)).toBe('Terminal')
    expect(matchDeclaredSpelling('TERMINAL', DECLARED)).toBe('Terminal')
    expect(matchDeclaredSpelling('streamjson', DECLARED)).toBe('StreamJson')
    expect(matchDeclaredSpelling('STREAMJSON', DECLARED)).toBe('StreamJson')
    expect(matchDeclaredSpelling('sTrEaMjSoN', DECLARED)).toBe('StreamJson')
  })

  it('모르는 낱말·문자열 아닌 값 → undefined(유효 집합은 안 넓어진다)', () => {
    expect(matchDeclaredSpelling('Termnal', DECLARED)).toBeUndefined()
    expect(matchDeclaredSpelling('stream_json', DECLARED)).toBeUndefined()
    expect(matchDeclaredSpelling('', DECLARED)).toBeUndefined()
    expect(matchDeclaredSpelling(7, DECLARED)).toBeUndefined()
    expect(matchDeclaredSpelling(undefined, DECLARED)).toBeUndefined()
    expect(matchDeclaredSpelling(null, DECLARED)).toBeUndefined()
    expect(matchDeclaredSpelling({ toString: () => 'Terminal' }, DECLARED)).toBeUndefined()
  })

  // Whitespace and surrounding characters are not trimmed — the only axis folded is letter case.
  it('trim 은 하지 않는다', () => {
    expect(matchDeclaredSpelling(' Terminal', DECLARED)).toBeUndefined()
    expect(matchDeclaredSpelling('Terminal ', DECLARED)).toBeUndefined()
  })

  // Even on the day two spellings that differ only in case get declared, a caller who writes it exactly receives their own spelling.
  it('완전 일치가 접기보다 먼저다', () => {
    const ambiguous = ['id', 'ID'] as const
    expect(matchDeclaredSpelling('ID', ambiguous)).toBe('ID')
    expect(matchDeclaredSpelling('id', ambiguous)).toBe('id')
  })
})
