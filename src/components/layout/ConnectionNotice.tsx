// The connection band (TRD S36 §3-4 · ADR-0350 · ADR-0356 · ADR-0134 decision 4) — ① 「연결 안 됨」 or ② 「버전 다름」.
//
// ★Why draw it on screen★: a failure where the daemon cannot start because the data folder is not writable reached nowhere
//   in release — the daemon has no console, and the WebView2 release build has no developer tools, so console.error cannot be
//   seen by the user. So the user saw only a causeless 'down'.
//
// ★The shell decides the band★: this renders `agentClient.connectionBand` as it comes; the start cause
//   (`connectionError`) is only ①'s cause text and never raises a band on its own (`bandView`).
//
// ★「다시 연결」 only with `actions` — the main window★ (ADR-0345 · ADR-0356: on both bands, no stop button): it calls the
//   existing `DaemonControl.start()`, the same call as boot and the LLM `connection.reconnect` (`connectionCommands`) —
//   no new control surface. Popouts and the spare show the text only. While the start runs the button shows the busy
//   label, disabled; afterwards the band follows the shell.
//
// ★It does not take its own place★ — the call site puts it on the layer that covers rather than pushes the layout (`NoticeOverlay`) (ADR-0277 — ADR-0180
//   amendment: the layout is fixed, and a covered area can be closed with ✕). That layer covers the top of the canvas below the title bar (tab bar · window buttons · draggable area)
//   (user decision 2026-10-26 · T-66 — before that it covered the TabBar).
// ★The ✕ and the closed memory are load-bearing★ — the area the band covers (the top of the canvas) cannot receive clicks.
//   - Without the ✕, while the band holds the covered top of the canvas (slot content · split lines) is blocked.
//   - Without the closed memory the ✕ is powerless — the state surface notifies on every status change, so the same
//     band would be drawn again right away. The memory is the band kind + cause text: a different band or cause shows
//     again, and `connected` releases it — a recurrence of the same cause later is the common case, so closing must not
//     mute it for good.
//
// ★Not always-on chrome (ADR-0063)★: it draws only when there is a band. With none it is null so the DOM is empty, and this
//   does not revive the always-on StatusBar that ADR-0063 removed.

import { useEffect, useState } from 'react'

import { agentClient, getDaemonControl } from '../../api/clientFactory'
import { bandView } from '../../api/connectionBand'
import { t } from '../../i18n'

function readStatus() {
  return { band: agentClient.connectionBand, error: agentClient.connectionError }
}

/** `actions` = draw 「다시 연결」 — the main window only. */
export default function ConnectionNotice({ actions = false }: { actions?: boolean }) {
  const [status, setStatus] = useState(readStatus)
  const [dismissed, setDismissed] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(
    () =>
      // The state surface also notifies on a band or cause change with the word unchanged (ProtocolClient).
      agentClient.onConnectionStateChange(state => {
        setStatus(readStatus())
        if (state === 'connected') setDismissed(null)
      }),
    [],
  )

  const view = bandView(status.band, status.error)
  if (view === null) return null
  const key = `${view.kind}|${view.cause ?? ''}`
  if (key === dismissed) return null

  const reconnect = (): void => {
    setBusy(true)
    // `start()` records the cause on `connectionError`, which shows only under a plain ①; under ② or ① `refused` the
    //   guard drops it and the band itself is the signal.
    getDaemonControl()
      .start()
      .catch((err: unknown) => console.warn('[ConnectionNotice] 다시 연결 실패:', err))
      .finally(() => setBusy(false))
  }

  return (
    <div
      role="alert"
      data-band={view.kind}
      className="flex shrink-0 items-start gap-3 border-b border-border bg-elevated px-3 py-1 text-xs leading-5 text-foreground"
      style={{ borderLeft: '3px solid var(--status-danger)' }}
    >
      {/* `wrap-anywhere` = a requirement of the covering layer (the `NoticeOverlay` doc) — the cause can carry a long path. */}
      <div className="min-w-0 flex-1 wrap-anywhere">
        <span className="font-medium">{t('connection.head')}</span>
        {view.cause !== null && (
          <>
            {' '}
            <span className="opacity-90">{view.cause}</span>
          </>
        )}
      </div>
      {actions && (
        <button
          type="button"
          className="shrink-0 rounded border border-border bg-surface px-2 leading-5 hover:bg-background disabled:cursor-default disabled:opacity-50"
          disabled={busy}
          aria-busy={busy}
          onClick={reconnect}
        >
          {busy ? t('connection.reconnecting') : t('connection.reconnect')}
        </button>
      )}
      <button
        type="button"
        aria-label={t('connection.dismiss')}
        className="shrink-0 rounded px-2 opacity-70 hover:opacity-100"
        onClick={() => setDismissed(key)}
      >
        ✕
      </button>
    </div>
  )
}
