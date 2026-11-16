// A notice raised in main when this launch cannot handle the screen state properly — when boot could not read `state.json` properly (TRD
// S21-storage §6-5) or when this launch does not save the screen state (guard ⅱ — the copy could not be copied aside · raised as the same kind = user
// decision 2026-10-06). It only informs and has no fix button = user decision 2026-10-05 · this shape that does not block like a modal (the covered area cannot be pressed until ✕) but can be closed =
// session judgment (delegated by the user).
//
// ADR-0276: this notice covers the layout · guard ⅱ is also raised as the same kind.
// ★It does not take its own place★ — the layer that covers the layout (`NoticeOverlay`) is laid by `AppLayout`.
// ★Once closed it does not show again during this launch★ — the two values (`state_file` · `saves`) are decided by boot step ⑥ and do not change even when answered
// (`src-tauri/bindings/RestoreStatusView.ts`).

import { useState } from 'react'

import type { StateFileStatus } from '../../api/restoreClient'
import { t } from '../../i18n'

function messageOf(stateFile: StateFileStatus | undefined, saves: boolean | undefined): string | null {
  switch (stateFile) {
    // Guard ⅰ (unreadable · newer edition) is also `saves:false`, but these two wordings already carry 「저장하지 않는다」 (does not save) — we do not
    //   raise the guard ⅱ wording on top of them.
    case 'unreadable':
      return t('restore.stateFileUnreadable')
    case 'newer':
      return t('restore.stateFileNewer')
    case 'corrupt_copied_aside':
      return t('restore.stateFileCorruptCopiedAside')
    case 'corrupt_not_copied':
      return t('restore.stateFileCorruptNotCopied')
    case 'ok':
      return saves === false ? t('restore.stateFileNotSaving') : null
    default:
      return null
  }
}

/** Both are fields of `restore.status` — `undefined` if not yet received or an unknown value (nothing is drawn for that field). */
export default function StateFileNotice({
  stateFile,
  saves,
}: {
  stateFile: StateFileStatus | undefined
  saves: boolean | undefined
}) {
  const [dismissed, setDismissed] = useState(false)
  const message = messageOf(stateFile, saves)
  if (dismissed || message === null) return null

  return (
    <div
      role="status"
      data-state-file={stateFile}
      data-saves={saves === undefined ? undefined : String(saves)}
      className="flex shrink-0 items-start gap-3 border-b border-border bg-elevated px-3 py-1 text-xs leading-5 text-foreground"
      style={{ borderLeft: '3px solid var(--status-blocked)' }}
    >
      <div className="min-w-0 flex-1 wrap-anywhere">{message}</div>
      <button
        type="button"
        aria-label={t('restore.dismissNotice')}
        className="shrink-0 rounded px-2 opacity-70 hover:opacity-100"
        onClick={() => setDismissed(true)}
      >
        ✕
      </button>
    </div>
  )
}
