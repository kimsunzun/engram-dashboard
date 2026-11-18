// The connection band (TRD S36 §3-4 · ADR-0350 · ADR-0356 · ADR-0134 decision 4) — ① "not connected" or ② "version mismatch".
// ★Why on screen★: a daemon that cannot start because the data folder is not writable reached nowhere in release — no daemon console, no WebView2 release devtools — so the user saw only a causeless 'down'.
// ★The shell decides the band★: this renders `agentClient.connectionBand` as it comes; the start cause (`connectionError`) is only ①'s cause text and never raises a band on its own (`bandView`).
// ★"Reconnect" only with `actions` — the main window★ (ADR-0345 · ADR-0356: no stop button on either band): it calls the existing `DaemonControl.start()`, same as boot and the LLM `connection.reconnect` (`connectionCommands`) — no new control surface. Popouts and the spare show the text only. While start runs the button shows the busy label, disabled; afterwards the band follows the shell.
// ★It takes no place of its own★ — the call site puts it on the layer that covers rather than pushes the layout (`NoticeOverlay`; ADR-0277 — ADR-0180 amendment: layout fixed, covered area closable with ✕). That layer covers the top of the canvas below the title bar (T-66).
// ★The ✕ and the closed memory are load-bearing★ — the area the band covers cannot receive clicks. Without the ✕ the covered top of the canvas (slot content · split lines) is blocked while the band holds; without the closed memory the ✕ is powerless — the state surface notifies on every status change, so the same band would redraw at once. The memory is the band kind + the untranslated cause (ADR-0373 — a language switch must not bring a closed band back): a different band or cause shows again, `connected` releases it, and a later recurrence of the same cause is the common case, so closing must not mute it for good.
// ★Not always-on chrome (ADR-0063)★: it draws only when there is a band; with none it is null, so the DOM is empty and the StatusBar ADR-0063 removed is not revived.

import { useEffect, useState } from 'react'

import { agentClient, getDaemonControl } from '../../api/clientFactory'
import { bandView, causeText } from '../../api/connectionBand'
import { t } from '../../i18n'

function readStatus() {
  return { band: agentClient.connectionBand, error: agentClient.connectionError }
}

/** `actions` = draw 「reconnect」 — the main window only. */
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
  // `bandView` builds the object in a fixed field order, so equal views give equal keys.
  const key = JSON.stringify(view)
  if (key === dismissed) return null

  const reconnect = (): void => {
    setBusy(true)
    // `start()` records the cause on `connectionError`, which shows only under a plain ①; under ② or ① `refused` the
    //   guard drops it and the band itself is the signal.
    getDaemonControl()
      .start()
      .catch((err: unknown) => console.warn('[ConnectionNotice] reconnect failed:', err))
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
            <span className="opacity-90">{causeText(view.cause)}</span>
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
