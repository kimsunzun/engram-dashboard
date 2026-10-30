// StateFileNotice — `state_file` 다섯 값 · 가드 ⅱ(`saves:false` · `ok`) · 닫기(사용자 결정 2026-10-05 · 2026-10-06).

import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'

import StateFileNotice from './StateFileNotice'
import type { StateFileStatus } from '../../api/restoreClient'
import { t } from '../../i18n'

afterEach(cleanup)

describe('StateFileNotice', () => {
  it('ok 이고 저장하는 실행이면 아무것도 그리지 않는다', () => {
    render(<StateFileNotice stateFile="ok" saves={true} />)
    expect(screen.queryByRole('status')).toBeNull()
  })

  it('아직 못 받았으면(undefined) 아무것도 그리지 않는다', () => {
    render(<StateFileNotice stateFile={undefined} saves={undefined} />)
    expect(screen.queryByRole('status')).toBeNull()
  })

  // 셸 계약의 짝 — unreadable · newer 는 가드 ⅰ 이라 늘 `saves:false` 다.
  it.each<[StateFileStatus, boolean, string]>([
    ['unreadable', false, t('restore.stateFileUnreadable')],
    ['newer', false, t('restore.stateFileNewer')],
    ['corrupt_copied_aside', true, t('restore.stateFileCorruptCopiedAside')],
    ['corrupt_not_copied', true, t('restore.stateFileCorruptNotCopied')],
  ])('%s 면 그 문구를 띄운다', (stateFile, saves, text) => {
    render(<StateFileNotice stateFile={stateFile} saves={saves} />)
    expect(screen.getByRole('status').textContent).toContain(text)
  })

  // 덮는 층의 줄 계약(`NoticeOverlay` doc) — `wrap-anywhere` 를 잃으면 긴 문구가 ✕ 를 화면 밖으로 민다.
  it('줄은 불투명 바탕이고 글 칸은 아무 데서나 줄을 바꾼다', () => {
    render(<StateFileNotice stateFile="unreadable" saves={false} />)
    const row = screen.getByRole('status')
    expect(row.className).toMatch(/\bbg-elevated\b/)
    expect(row.firstElementChild?.className).toMatch(/\bwrap-anywhere\b/)
  })

  it('unreadable 문구는 파일 이름과 「이번 실행은 저장하지 않는다」를 싣는다', () => {
    render(<StateFileNotice stateFile="unreadable" saves={false} />)
    const text = screen.getByRole('status').textContent ?? ''
    expect(text).toContain('state.json')
    expect(text).toContain('저장하지 않')
  })

  // ADR-0291: 새 버전이 저장한 파일은 손상 문구가 아니라 따로 — 떠 두지 않으므로 .corrupt 를 말하지 않는다.
  it('newer 문구는 파일 이름 · 「저장하지 않는다」 · 「그대로 둔다」를 싣고 .corrupt 를 말하지 않는다', () => {
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

  it('떠 두지 못한 경우는 .corrupt 를 이번 실행의 백업이 아니라 이전 실행이 남긴 것이라고 말한다', () => {
    render(<StateFileNotice stateFile="corrupt_not_copied" saves={true} />)
    const text = screen.getByRole('status').textContent ?? ''
    expect(text).toContain('보관하지 못했습니다')
    expect(text).toContain('이전 실행에서 남긴 것')
    expect(text).toContain('백업이 아닙니다')
  })

  it('떠 둔 경우는 state.json.corrupt 를 가리킨다', () => {
    render(<StateFileNotice stateFile="corrupt_copied_aside" saves={true} />)
    expect(screen.getByRole('status').textContent).toContain('state.json.corrupt')
  })

  it('닫으면 사라지고, 다시 그려져도 돌아오지 않는다', () => {
    const { rerender } = render(<StateFileNotice stateFile="unreadable" saves={false} />)
    fireEvent.click(screen.getByLabelText(t('restore.dismissNotice')))
    expect(screen.queryByRole('status')).toBeNull()
    rerender(<StateFileNotice stateFile="unreadable" saves={false} />)
    expect(screen.queryByRole('status')).toBeNull()
  })
})

describe('StateFileNotice — 가드 ⅱ(사본을 못 떠 이 실행이 저장하지 않는다)', () => {
  it('ok 인데 saves:false 면 「저장하지 않는다 · 다시 묻는다」를 띄운다', () => {
    render(<StateFileNotice stateFile="ok" saves={false} />)
    const notice = screen.getByRole('status')
    expect(notice.textContent).toContain(t('restore.stateFileNotSaving'))
    expect(notice.textContent).toContain('저장하지 않')
    expect(notice.textContent).toContain('다시 묻')
    expect(notice.getAttribute('data-saves')).toBe('false')
  })

  it('state_file 을 모르면(셸과 판이 어긋남) saves:false 여도 그리지 않는다 — 어느 문구인지 모른다', () => {
    render(<StateFileNotice stateFile={undefined} saves={false} />)
    expect(screen.queryByRole('status')).toBeNull()
  })

  it('ok 인데 saves 를 모르면(셸과 판이 어긋남) 그리지 않는다', () => {
    render(<StateFileNotice stateFile="ok" saves={undefined} />)
    expect(screen.queryByRole('status')).toBeNull()
  })

  it('원인이 여럿이라 state.json 을 탓하지 않는다', () => {
    render(<StateFileNotice stateFile="ok" saves={false} />)
    expect(screen.getByRole('status').textContent).not.toContain('state.json')
  })

  it('unreadable(가드 ⅰ — 역시 saves:false)이면 그 문구 하나만 — 가드 ⅱ 문구를 겹치지 않는다', () => {
    render(<StateFileNotice stateFile="unreadable" saves={false} />)
    expect(screen.getAllByRole('status')).toHaveLength(1)
    const text = screen.getByRole('status').textContent ?? ''
    expect(text).toContain(t('restore.stateFileUnreadable'))
    expect(text).not.toContain(t('restore.stateFileNotSaving'))
  })

  it('닫으면 사라지고, 다시 그려져도 돌아오지 않는다', () => {
    const { rerender } = render(<StateFileNotice stateFile="ok" saves={false} />)
    fireEvent.click(screen.getByLabelText(t('restore.dismissNotice')))
    expect(screen.queryByRole('status')).toBeNull()
    rerender(<StateFileNotice stateFile="ok" saves={false} />)
    expect(screen.queryByRole('status')).toBeNull()
  })
})
