// The band model (TRD S36 §3-2) — what each shell band says, and where a start's cause shows.

import { describe, expect, it } from 'vitest'

import { bandText, bandView } from './connectionBand'

const HEAD = '데몬에 연결하지 못했습니다.'
const REFUSED = '데몬이 이 앱의 인증을 거절했습니다'
const VERSION = '실행 중인 데몬이 이 앱과 버전이 다릅니다. 데몬을 끈 뒤 다시 연결하세요'

describe('bandView · bandText', () => {
  it('① plain with no cause = the head alone', () => {
    const view = bandView({ kind: 'not_connected', refused: false }, null)
    expect(view).toEqual({ kind: 'not_connected', cause: null })
    expect(bandText(view!)).toBe(HEAD)
  })

  it("① plain carries a failed start's cause", () => {
    const view = bandView({ kind: 'not_connected', refused: false }, 'cannot write to the folder')
    expect(view).toEqual({ kind: 'not_connected', cause: { kind: 'error', message: 'cannot write to the folder' } })
    expect(bandText(view!)).toBe(`${HEAD} cannot write to the folder`)
  })

  it('① refused shows the refusal, not the start cause', () => {
    const view = bandView({ kind: 'not_connected', refused: true }, 'cannot write to the folder')
    expect(view).toEqual({ kind: 'not_connected', cause: { kind: 'refused' } })
    expect(bandText(view!)).toBe(`${HEAD} ${REFUSED}`)
  })

  it.each([true, false])('② (can_stop %s) shows the version text and ignores the start cause', canStop => {
    const view = bandView({ kind: 'version', can_stop: canStop }, 'cannot write to the folder')
    expect(view).toEqual({ kind: 'version', cause: { kind: 'version' } })
    expect(bandText(view!)).toBe(`${HEAD} ${VERSION}`)
  })

  it('no band = no view, even with a start cause', () => {
    expect(bandView(null, 'cannot write to the folder')).toBeNull()
  })
})
