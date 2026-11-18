// StateFileNotice — the five `state_file` values · guard ⅱ (`saves:false` · `ok`) · close (user decision 2026-10-05 · 2026-10-06).

import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'

import StateFileNotice from './StateFileNotice'
import type { StateFileStatus } from '../../api/restoreClient'
import { t } from '../../i18n'

afterEach(cleanup)

describe('StateFileNotice', () => {
  it('if ok and a saving run it draws nothing', () => {
    render(<StateFileNotice stateFile="ok" saves={true} />)
    expect(screen.queryByRole('status')).toBeNull()
  })

  it('if not received yet (undefined) it draws nothing', () => {
    render(<StateFileNotice stateFile={undefined} saves={undefined} />)
    expect(screen.queryByRole('status')).toBeNull()
  })

  // A pair of the shell contract — unreadable · newer are guard ⅰ so they are always `saves:false`.
  it.each<[StateFileStatus, boolean, string]>([
    ['unreadable', false, t('restore.stateFileUnreadable')],
    ['newer', false, t('restore.stateFileNewer')],
    ['corrupt_copied_aside', true, t('restore.stateFileCorruptCopiedAside')],
    ['corrupt_not_copied', true, t('restore.stateFileCorruptNotCopied')],
  ])('%s shows that text', (stateFile, saves, text) => {
    render(<StateFileNotice stateFile={stateFile} saves={saves} />)
    expect(screen.getByRole('status').textContent).toContain(text)
  })

  // The row contract of the covering layer (the `NoticeOverlay` doc) — if `wrap-anywhere` is lost, long wording pushes the ✕ off screen.
  it('the row has an opaque background and the text field wraps anywhere', () => {
    render(<StateFileNotice stateFile="unreadable" saves={false} />)
    const row = screen.getByRole('status')
    expect(row.className).toMatch(/\bbg-elevated\b/)
    expect(row.firstElementChild?.className).toMatch(/\bwrap-anywhere\b/)
  })

  it('the unreadable text carries the file name and 「this run does not save」', () => {
    render(<StateFileNotice stateFile="unreadable" saves={false} />)
    const text = screen.getByRole('status').textContent ?? ''
    expect(text).toContain('state.json')
    expect(text).toContain('저장하지 않')
  })

  // ADR-0291: a file saved by a newer version is separate from the corruption wording — since we do not copy it aside, we do not mention .corrupt.
  it('the newer text carries the file name · 「does not save」 · 「leaves it as is」 and does not mention .corrupt', () => {
    render(<StateFileNotice stateFile="newer" saves={false} />)
    expect(screen.getAllByRole('status')).toHaveLength(1)
    const text = screen.getByRole('status').textContent ?? ''
    expect(text).toContain('state.json')
    expect(text).toContain('새 버전')
    expect(text).toContain('저장하지 않')
    expect(text).toContain('그대로 둡니다')
    expect(text).not.toContain('.corrupt')
    expect(text).not.toContain(t('restore.stateFileNotSaving'))
  })

  it('when it could not leave a copy, it says .corrupt was left by an earlier run, not this run\'s backup', () => {
    render(<StateFileNotice stateFile="corrupt_not_copied" saves={true} />)
    const text = screen.getByRole('status').textContent ?? ''
    expect(text).toContain('보관하지 못했습니다')
    expect(text).toContain('이전 실행에서 남긴 것')
    expect(text).toContain('백업이 아닙니다')
  })

  it('when it left a copy it points to state.json.corrupt', () => {
    render(<StateFileNotice stateFile="corrupt_copied_aside" saves={true} />)
    expect(screen.getByRole('status').textContent).toContain('state.json.corrupt')
  })

  it('it disappears on close, and does not return even when redrawn', () => {
    const { rerender } = render(<StateFileNotice stateFile="unreadable" saves={false} />)
    fireEvent.click(screen.getByLabelText(t('restore.dismissNotice')))
    expect(screen.queryByRole('status')).toBeNull()
    rerender(<StateFileNotice stateFile="unreadable" saves={false} />)
    expect(screen.queryByRole('status')).toBeNull()
  })
})

describe('StateFileNotice — guard ⅱ (could not make a copy so this run does not save)', () => {
  it('ok with saves:false shows 「does not save · asks again」', () => {
    render(<StateFileNotice stateFile="ok" saves={false} />)
    const notice = screen.getByRole('status')
    expect(notice.textContent).toContain(t('restore.stateFileNotSaving'))
    expect(notice.textContent).toContain('저장하지 않')
    expect(notice.textContent).toContain('다시 묻')
    expect(notice.getAttribute('data-saves')).toBe('false')
  })

  it('if state_file is unknown (version mismatch with the shell) it does not draw even with saves:false — it is unknown which text applies', () => {
    render(<StateFileNotice stateFile={undefined} saves={false} />)
    expect(screen.queryByRole('status')).toBeNull()
  })

  it('ok with saves unknown (version mismatch with the shell) does not draw', () => {
    render(<StateFileNotice stateFile="ok" saves={undefined} />)
    expect(screen.queryByRole('status')).toBeNull()
  })

  it('with several causes it does not blame state.json', () => {
    render(<StateFileNotice stateFile="ok" saves={false} />)
    expect(screen.getByRole('status').textContent).not.toContain('state.json')
  })

  it('unreadable (guard ⅰ — also saves:false) shows only that one text — guard ⅱ text does not overlap', () => {
    render(<StateFileNotice stateFile="unreadable" saves={false} />)
    expect(screen.getAllByRole('status')).toHaveLength(1)
    const text = screen.getByRole('status').textContent ?? ''
    expect(text).toContain(t('restore.stateFileUnreadable'))
    expect(text).not.toContain(t('restore.stateFileNotSaving'))
  })

  it('it disappears on close, and does not repeat even when redrawn', () => {
    const { rerender } = render(<StateFileNotice stateFile="ok" saves={false} />)
    fireEvent.click(screen.getByLabelText(t('restore.dismissNotice')))
    expect(screen.queryByRole('status')).toBeNull()
    rerender(<StateFileNotice stateFile="ok" saves={false} />)
    expect(screen.queryByRole('status')).toBeNull()
  })
})
