// The band model (TRD S36 §3-2) — what each shell band says, and where a start's cause shows.

import { afterEach, describe, expect, it } from 'vitest'

import { setLanguage } from '../i18n'
import { bandText, bandView } from './connectionBand'

const HEAD = '데몬에 연결하지 못했습니다.'
const REFUSED = '데몬이 이 앱의 인증을 거절했습니다'
const VERSION = '실행 중인 데몬이 이 앱과 버전이 다릅니다. 데몬을 끈 뒤 다시 연결하세요'
const OTHER_ACCOUNT = '이 데몬은 다른 Windows 계정의 것입니다. 계정마다 데이터 폴더를 따로 쓰세요'
const UNVERIFIED =
  '데몬의 계정을 확인하지 못했습니다. 앱과 Engram을 같은 방식으로(둘 다 관리자 권한 또는 둘 다 일반 권한) 실행하세요'

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

  // ADR-0395: the shell refused the daemon listener's OS account itself — its own words, never the start cause.
  it.each([
    [false, 'other_account', OTHER_ACCOUNT],
    [true, 'account_unverified', UNVERIFIED],
  ] as const)('the account band (unverified %s) shows its own cause and ignores the start cause', (unverified, kind, text) => {
    const view = bandView({ kind: 'account', unverified }, 'cannot write to the folder')
    expect(view).toEqual({ kind: 'account', cause: { kind } })
    expect(bandText(view!)).toBe(`${HEAD} ${text}`)
  })

  it('no band = no view, even with a start cause', () => {
    expect(bandView(null, 'cannot write to the folder')).toBeNull()
  })
})

// ADR-0399: the start was refused before a spawn for the data folder's drive — the typed cause in the language of the moment, never
//   the start's English text.
describe('the volume band', () => {
  afterEach(() => {
    setLanguage('ko')
  })

  it.each([
    [false, 'unprotected_volume', 'Engram 폴더가 파일 권한을 저장하지 못하는 드라이브(FAT 또는 exFAT)에 있습니다. Engram을 종료하고 Engram 폴더(포터블 사본이면 exe가 든 폴더 전체)를 NTFS 드라이브로 옮긴 뒤 다시 시작하세요', 'The Engram folder is on a drive that cannot keep file permissions (FAT or exFAT). Exit Engram, move the Engram folder (for a portable copy, the whole folder with the exe) to an NTFS drive, then start it again.'],
    [true, 'volume_unreadable', 'Engram 폴더가 있는 드라이브의 속성을 읽지 못했습니다. Engram을 종료하고 Engram 폴더(포터블 사본이면 exe가 든 폴더 전체)를 NTFS 드라이브로 옮긴 뒤 다시 시작하세요', 'Could not read the properties of the drive the Engram folder is on. Exit Engram, move the Engram folder (for a portable copy, the whole folder with the exe) to an NTFS drive, then start it again.'],
  ] as const)('(unreadable %s) shows its own cause in ko and en and ignores the start cause', (unreadable, kind, ko, en) => {
    const view = bandView({ kind: 'volume', unreadable }, 'the data folder (E:/x) is on a drive that does not store file permissions (FAT or exFAT)')
    expect(view).toEqual({ kind: 'volume', cause: { kind } })
    expect(bandText(view!)).toBe(`${HEAD} ${ko}`)
    setLanguage('en')
    expect(bandText(view!)).toBe(`Could not connect to the daemon. ${en}`)
  })
})
