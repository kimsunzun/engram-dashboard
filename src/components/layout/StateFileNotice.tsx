// 이 실행의 부팅이 화면 상태 파일(`state.json`)을 제대로 못 읽었을 때 main 위에 띄우는 알림(TRD S21-storage §6-5).
// 안내만 하고 고치는 단추는 두지 않는다 = 사용자 결정 2026-10-05 · 막지 않고 닫을 수 있는 이 모양 = 세션 판단(사용자 위임).
//
// ★덮지 않는다★: `ConnectionNotice` 와 같은 일반 흐름 블록이다 — 오버레이는 TabBar 클릭을 먹는다(그 파일 머리).
// ★닫으면 이 실행 동안 다시 안 뜬다★ — 값은 부팅 단계 ⑥ 이 정하고 실행 내내 바뀌지 않는다
// (`src-tauri/bindings/StateFileStatus.ts`).

import { useState } from 'react'

import type { StateFileStatus } from '../../api/restoreClient'
import { t } from '../../i18n'

function messageOf(stateFile: StateFileStatus | undefined): string | null {
  switch (stateFile) {
    case 'unreadable':
      return t('restore.stateFileUnreadable')
    case 'corrupt_copied_aside':
      return t('restore.stateFileCorruptCopiedAside')
    case 'corrupt_not_copied':
      return t('restore.stateFileCorruptNotCopied')
    default:
      return null
  }
}

/** `stateFile` = 아직 못 받았으면 `undefined`(그리지 않는다). */
export default function StateFileNotice({ stateFile }: { stateFile: StateFileStatus | undefined }) {
  const [dismissed, setDismissed] = useState(false)
  const message = messageOf(stateFile)
  if (dismissed || message === null) return null

  return (
    <div
      role="status"
      data-state-file={stateFile}
      className="flex shrink-0 items-start gap-3 border-b border-border bg-surface px-4 py-2 text-sm text-foreground"
      style={{ borderLeft: '3px solid var(--status-blocked)' }}
    >
      <div className="min-w-0 flex-1">{message}</div>
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
