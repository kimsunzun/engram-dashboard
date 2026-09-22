import { describe, expect, it } from 'vitest'

import { matchDeclaredSpelling } from './enumArg'

const DECLARED = ['Terminal', 'StreamJson'] as const

describe('matchDeclaredSpelling', () => {
  it('완전 일치는 그대로 돌려준다', () => {
    expect(matchDeclaredSpelling('Terminal', DECLARED)).toBe('Terminal')
    expect(matchDeclaredSpelling('StreamJson', DECLARED)).toBe('StreamJson')
  })

  // ★핵심★: 반환은 호출자가 준 문자열이 아니라 선언 철자다 — 이것이 wire 값의 바이트 동일성을 진다.
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

  // 공백·주변 문자는 안 다듬는다 — 접는 축은 대소문자 하나뿐이다.
  it('trim 은 하지 않는다', () => {
    expect(matchDeclaredSpelling(' Terminal', DECLARED)).toBeUndefined()
    expect(matchDeclaredSpelling('Terminal ', DECLARED)).toBeUndefined()
  })

  // 대소문자만 다른 두 철자가 선언되는 날에도 정확히 쓴 호출자는 자기 철자를 받는다.
  it('완전 일치가 접기보다 먼저다', () => {
    const ambiguous = ['id', 'ID'] as const
    expect(matchDeclaredSpelling('ID', ambiguous)).toBe('ID')
    expect(matchDeclaredSpelling('id', ambiguous)).toBe('id')
  })
})
