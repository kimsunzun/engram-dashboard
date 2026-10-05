// 이 실행이 화면 상태를 제대로 다루지 못할 때 main 에 띄우는 알림 — 부팅이 `state.json` 을 제대로 못 읽었거나(TRD
// S21-storage §6-5) 이 실행이 화면 상태를 저장하지 않을 때(가드 ⅱ — 사본을 못 떴다 · 같은 종류로 띄우는 것 = 사용자
// 결정 2026-10-06). 안내만 하고 고치는 단추는 두지 않는다 = 사용자 결정 2026-10-05 · 막지 않고 닫을 수 있는 이 모양 =
// 세션 판단(사용자 위임).
//
// ★스스로 자리를 잡지 않는다★ — 레이아웃을 덮는 층은 `AppLayout` 이 깐다.
// ★닫으면 이 실행 동안 다시 안 뜬다★ — 두 값(`state_file` · `saves`)은 부팅 단계 ⑥ 이 정하고 답해도 바뀌지 않는다
// (`src-tauri/bindings/RestoreStatusView.ts`).

import { useState } from 'react'

import type { StateFileStatus } from '../../api/restoreClient'
import { t } from '../../i18n'

function messageOf(stateFile: StateFileStatus | undefined, saves: boolean | undefined): string | null {
  switch (stateFile) {
    // 가드 ⅰ 도 `saves:false` 지만 이 문구가 「저장하지 않는다」를 이미 싣는다 — 가드 ⅱ 문구를 겹쳐 띄우지 않는다.
    case 'unreadable':
      return t('restore.stateFileUnreadable')
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

/** 둘 다 `restore.status` 의 칸 — 아직 못 받았거나 모르는 값이면 `undefined`(그 칸으로는 그리지 않는다). */
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
      className="flex shrink-0 items-start gap-3 border-b border-border bg-elevated px-4 py-2 text-sm text-foreground"
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
