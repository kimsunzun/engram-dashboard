// 비정상 종료 뒤 「이전 화면을 복원할까요?」 — main 창 전용 모달(TRD S21-storage §6-7 · 사용자 결정 D4).
//
// ★떠 있는 동안 main 을 쓸 수 없다★ — 그 아래 레이아웃은 `AppLayout` 이 `inert` 로 잠그고, 이 컴포넌트는 포커스를
//   안으로 옮기고 Tab 을 안에 가두며 단축키 디스패처를 멈춘다(N4). 디스패처는 편집 대상(xterm · 챗 입력)을 그냥
//   흘려보내므로 그것을 멈추는 것만으로는 그 아래 타이핑이 막히지 않는다 — 잠금 셋은 한 벌이다.
// ★닫힘은 상태가 정한다★ — 단추를 눌렀다고 닫지 않는다. 답 뒤 다시 당긴 상태나 `restore:changed`(버스로 낸 답 포함)
//   가 `awaiting` 이 아니게 되면 `AppLayout` 이 이것을 걷는다. 실패면 `awaiting` 그대로라 다시 답할 수 있다.
// ★막는 것은 사람 UI 뿐이다★ — 다른 창(트리 · 팝아웃)과 LLM 버스 명령은 막지 않는다.
// ★보여 주는 것은 버스 `restore.status` 가 싣는 것뿐이다★ — 저장 시각 · 창 수 · 탭 수.

import { useEffect, useId, useLayoutEffect, useRef, useState } from 'react'
import type { KeyboardEvent as ReactKeyboardEvent, MouseEvent as ReactMouseEvent } from 'react'

import type { RestoreView } from '../../api/restoreClient'
import { setKeybindingsPaused } from '../../commands/keybindings'
import { t } from '../../i18n'

const SAVED_AT = new Intl.DateTimeFormat('ko-KR', {
  year: 'numeric',
  month: 'numeric',
  day: 'numeric',
  hour: '2-digit',
  minute: '2-digit',
  hourCycle: 'h23',
})

/**
 * 유닉스 밀리초 → 이 PC 시간대의 날짜·시각. `Date` 로 나타낼 수 없는 값(±8.64e15 밖 · 유한하지 않음)이면 `null`.
 */
export function formatSavedAt(ms: number): string | null {
  const at = new Date(ms)
  // ★값을 거르지 않고 넘기면 `format` 이 RangeError 를 던져 모달이 아니라 화면 전체가 오류 경계로 넘어간다★ — 셸은 사본의
  //   `saved_at_ms` 를 u64 그대로 싣고 사본은 디스크 바이트 그대로라, 손상되거나 손으로 고친 파일이면 이 범위를 넘는다.
  return Number.isNaN(at.getTime()) ? null : SAVED_AT.format(at)
}

const BUTTON =
  'rounded border border-border bg-surface px-3 py-1 hover:bg-background disabled:cursor-default disabled:opacity-50'

interface Props {
  /** `crash_copy === 'awaiting'` 인 상태. */
  status: RestoreView
  /** 답을 내고 상태를 다시 당긴 뒤 풀린다 — 실패는 reject(`RestoreClient.answer`). */
  onAnswer: (accept: boolean) => Promise<unknown>
}

export default function RestoreModal({ status, onAnswer }: Props) {
  const [busy, setBusy] = useState(false)
  const [failed, setFailed] = useState(false)
  const dialogRef = useRef<HTMLDivElement>(null)
  const acceptRef = useRef<HTMLButtonElement>(null)
  const titleId = useId()
  const descId = useId()

  // 커밋 직후 · 페인트 전에 멈춘다 — 그 사이의 키가 그 아래 화면을 바꾸지 않게.
  useLayoutEffect(() => {
    setKeybindingsPaused(true)
    return () => setKeybindingsPaused(false)
  }, [])

  // 처음 뜰 때, 그리고 답이 끝나 단추가 다시 살아날 때 포커스가 밖(body · 비활성 단추가 놓친 자리)에 있으면 데려온다.
  // 덜 파괴적인 「이전 화면 복원」에 둔다 — Enter 한 번이 되돌릴 수 없는 거절이 되지 않게.
  useEffect(() => {
    if (busy) return
    if (!dialogRef.current?.contains(document.activeElement)) acceptRef.current?.focus()
  }, [busy])

  const answer = (accept: boolean): void => {
    setBusy(true)
    setFailed(false)
    onAnswer(accept)
      .catch(e => {
        console.warn('[restore] restore_answer 실패:', e)
        setFailed(true)
      })
      .finally(() => setBusy(false))
  }

  // 아래 레이아웃이 `inert` 라 Tab 이 거기로 가지는 않지만, 마지막 단추 뒤에서 웹뷰 밖으로 빠지지 않게 안에서 돌린다.
  const onKeyDown = (e: ReactKeyboardEvent<HTMLDivElement>): void => {
    if (e.key !== 'Tab') return
    const dialog = dialogRef.current
    if (!dialog) return
    const items = [...dialog.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')]
    if (items.length === 0) {
      e.preventDefault()
      return
    }
    const first = items[0]
    const last = items[items.length - 1]
    if (document.activeElement === (e.shiftKey ? first : last)) {
      e.preventDefault()
      ;(e.shiftKey ? last : first).focus()
    }
  }

  // 단추 밖을 눌러도 포커스가 body 로 떨어지지 않게 한다.
  const onMouseDown = (e: ReactMouseEvent<HTMLDivElement>): void => {
    if (!(e.target as Element).closest('button')) e.preventDefault()
  }

  const { windows, tabs } = status
  const savedAt = status.saved_at_ms === null ? null : formatSavedAt(status.saved_at_ms)
  // `null` 은 묻는 동안 오지 않는 값이다 — 오면 「지운다 · 되돌릴 수 없다」 쪽으로 둔다. 남는다고 말했는데 지워지는
  //   것이 그 반대보다 나쁘다.
  const warning = status.durable === false ? t('restore.rejectWarningNotSaving') : t('restore.rejectWarning')

  return (
    <div
      data-restore-overlay=""
      className="fixed inset-0 z-50 flex items-center justify-center p-4"
      style={{ background: 'rgba(0, 0, 0, 0.55)' }}
      onMouseDown={onMouseDown}
    >
      <div
        ref={dialogRef}
        role="alertdialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={descId}
        onKeyDown={onKeyDown}
        className="flex w-full max-w-md flex-col gap-3 rounded border border-border bg-elevated p-5 text-sm text-foreground shadow-lg"
      >
        <h2 id={titleId} className="text-base font-semibold">
          {t('restore.title')}
        </h2>
        <p id={descId}>{t('restore.intro')}</p>
        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1">
          {savedAt !== null && (
            <>
              <dt className="text-muted">{t('restore.savedAt')}</dt>
              <dd data-restore-saved-at="">{savedAt}</dd>
            </>
          )}
          {windows !== null && (
            <>
              <dt className="text-muted">{t('restore.windows')}</dt>
              <dd data-restore-windows="">{t('restore.count', { count: String(windows) })}</dd>
            </>
          )}
          {tabs !== null && (
            <>
              <dt className="text-muted">{t('restore.tabs')}</dt>
              <dd data-restore-tabs="">{t('restore.count', { count: String(tabs) })}</dd>
            </>
          )}
        </dl>
        <p data-restore-warning="" className="text-muted">
          {warning}
        </p>
        {failed && (
          <p role="alert" style={{ color: 'var(--status-danger)' }}>
            {t('restore.answerFailed')}
          </p>
        )}
        <div className="flex items-center justify-end gap-2">
          {busy && <span className="mr-auto text-muted">{t('restore.working')}</span>}
          <button ref={acceptRef} type="button" className={BUTTON} disabled={busy} onClick={() => answer(true)}>
            {t('restore.accept')}
          </button>
          <button type="button" className={BUTTON} disabled={busy} onClick={() => answer(false)}>
            {t('restore.reject')}
          </button>
        </div>
      </div>
    </div>
  )
}
