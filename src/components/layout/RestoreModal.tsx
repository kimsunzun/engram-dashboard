// 「이전 화면을 복원할까요?」 (restore the previous screen?) after an abnormal exit — a modal for the main window only (TRD S21-storage §6-7 · user decision D4).
//
// ★While up, main cannot be used★ — the layout beneath is locked by `AppLayout` with `inert`, and this component moves focus
//   inside, traps Tab inside, and stops the shortcut dispatcher (N4). The dispatcher lets edit targets (xterm · chat input)
//   pass through, so merely stopping it does not block typing beneath — the three locks are one set.
// ★Closing is decided by state★ — it does not close because a button was pressed. When the state pulled again after the answer or `restore:changed` (including an answer sent over the bus)
//   is no longer `awaiting`, `AppLayout` withdraws this. On failure it stays `awaiting` so it can be answered again.
// ★Only the human UI is blocked★ — other windows (tree · popout) and LLM bus commands are not blocked.
// ★What it shows is only what the bus `restore.status` carries★ — save time · number of windows · number of tabs.

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
 * Unix milliseconds → date · time in this PC's time zone. `null` for a value that cannot be represented as a `Date` (outside ±8.64e15 · not finite).
 */
export function formatSavedAt(ms: number): string | null {
  const at = new Date(ms)
  // ★If the value is passed on unfiltered `format` throws a RangeError and not the modal but the whole screen goes to the error boundary★ — the shell carries the copy's
  //   `saved_at_ms` as a u64 as is and the copy is the disk bytes as is, so a corrupted or hand-edited file exceeds this range.
  return Number.isNaN(at.getTime()) ? null : SAVED_AT.format(at)
}

const BUTTON =
  'rounded border border-border bg-surface px-3 py-1 hover:bg-background disabled:cursor-default disabled:opacity-50'

interface Props {
  /** The state where `crash_copy === 'awaiting'`. */
  status: RestoreView
  /** Resolves after sending the answer and pulling the state again — failure is a reject (`RestoreClient.answer`). */
  onAnswer: (accept: boolean) => Promise<unknown>
}

export default function RestoreModal({ status, onAnswer }: Props) {
  const [busy, setBusy] = useState(false)
  const [failed, setFailed] = useState(false)
  const dialogRef = useRef<HTMLDivElement>(null)
  const acceptRef = useRef<HTMLButtonElement>(null)
  const titleId = useId()
  const descId = useId()

  // Stops right after commit · before paint — so keys in between do not change the screen beneath.
  useLayoutEffect(() => {
    setKeybindingsPaused(true)
    return () => setKeybindingsPaused(false)
  }, [])

  // When it first appears, and when the answer ends and the buttons come alive again, bring focus in if it is outside (body · the spot a disabled button dropped).
  // Put it on the less destructive 「이전 화면 복원」 (restore previous screen) — so a single Enter does not become an irreversible refusal.
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

  // The layout beneath is `inert` so Tab does not go there, but cycle inside so it does not leave the webview after the last button.
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

  // Make sure pressing outside a button does not drop focus to body.
  const onMouseDown = (e: ReactMouseEvent<HTMLDivElement>): void => {
    if (!(e.target as Element).closest('button')) e.preventDefault()
  }

  const { windows, tabs } = status
  const savedAt = status.saved_at_ms === null ? null : formatSavedAt(status.saved_at_ms)
  // `null` is a value that does not come while asking — if it comes, put it on the 「지운다 · 되돌릴 수 없다」 (deletes · cannot be undone) side. Saying it stays and then deleting
  //   is worse than the opposite.
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
