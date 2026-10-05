// StateFileNotice — `state_file` 네 값과 닫기(사용자 결정 2026-10-05).

import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'

import StateFileNotice from './StateFileNotice'
import type { StateFileStatus } from '../../api/restoreClient'
import { t } from '../../i18n'

afterEach(cleanup)

describe('StateFileNotice', () => {
  it('ok 면 아무것도 그리지 않는다', () => {
    render(<StateFileNotice stateFile="ok" />)
    expect(screen.queryByRole('status')).toBeNull()
  })

  it('아직 못 받았으면(undefined) 아무것도 그리지 않는다', () => {
    render(<StateFileNotice stateFile={undefined} />)
    expect(screen.queryByRole('status')).toBeNull()
  })

  it.each<[StateFileStatus, string]>([
    ['unreadable', t('restore.stateFileUnreadable')],
    ['corrupt_copied_aside', t('restore.stateFileCorruptCopiedAside')],
    ['corrupt_not_copied', t('restore.stateFileCorruptNotCopied')],
  ])('%s 면 그 문구를 띄운다', (stateFile, text) => {
    render(<StateFileNotice stateFile={stateFile} />)
    expect(screen.getByRole('status').textContent).toContain(text)
  })

  it('unreadable 문구는 파일 이름과 「이번 실행은 저장하지 않는다」를 싣는다', () => {
    render(<StateFileNotice stateFile="unreadable" />)
    const text = screen.getByRole('status').textContent ?? ''
    expect(text).toContain('state.json')
    expect(text).toContain('저장하지 않')
  })

  it('떠 두지 못한 경우는 .corrupt 를 이번 실행의 백업이 아니라 이전 실행이 남긴 것이라고 말한다', () => {
    render(<StateFileNotice stateFile="corrupt_not_copied" />)
    const text = screen.getByRole('status').textContent ?? ''
    expect(text).toContain('보관하지 못했습니다')
    expect(text).toContain('이전 실행에서 남긴 것')
    expect(text).toContain('백업이 아닙니다')
  })

  it('떠 둔 경우는 state.json.corrupt 를 가리킨다', () => {
    render(<StateFileNotice stateFile="corrupt_copied_aside" />)
    expect(screen.getByRole('status').textContent).toContain('state.json.corrupt')
  })

  it('닫으면 사라지고, 다시 그려져도 돌아오지 않는다', () => {
    const { rerender } = render(<StateFileNotice stateFile="unreadable" />)
    fireEvent.click(screen.getByLabelText(t('restore.dismissNotice')))
    expect(screen.queryByRole('status')).toBeNull()
    rerender(<StateFileNotice stateFile="unreadable" />)
    expect(screen.queryByRole('status')).toBeNull()
  })
})
