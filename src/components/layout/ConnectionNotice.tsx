// Connection failure reason notice (ADR-0134 decision 4).
//
// ★Why draw it on screen★: a failure where the daemon cannot start because the data folder is not writable reached nowhere
//   in release — the daemon has no console, and the WebView2 release build has no developer tools, so console.error cannot be
//   seen by the user. So the user saw only a causeless 'down'.
//
// ★Not a new control surface★: it only **reads** the existing connection state surface (agentClient) — it does not
//   add a window global handle and sends no command (§5 LLM-first control — the control surface is the one agentClient).
//
// ★It does not take its own place★ — the call site puts it on the layer that covers rather than pushes the layout (`NoticeOverlay`) (ADR-0277 — ADR-0180
//   amendment: the layout is fixed, and a covered area can be closed with ✕). That layer covers the top of the canvas below the title bar (tab bar · window buttons · draggable area)
//   (user decision 2026-10-26 · T-66 — before that it covered the TabBar).
// ★The ✕ and the closed memory are load-bearing★ — the area the band covers (the top of the canvas) cannot receive clicks.
//   - Without the ✕, while the reason is not cleared the covered top of the canvas (slot content · split lines) is blocked permanently.
//   - Without the closed memory the ✕ is powerless — the state surface notifies every time the transport state changes even if the reason is the same
//     (the state subscription of `ProtocolClient`), so the same reason is drawn again right away. What is remembered is the single closed-reason string, so a different reason
//     differs from it and shows, and the memory is released only when the reason is cleared (`null`) — even if A is closed, B shows, and then A comes again,
//     it does not show (see the state · subscription comments below).
//
// ★Not always-on chrome (ADR-0063)★: it draws only when there is a reason. With none it is null so the DOM is empty, and this
//   does not revive the always-on StatusBar that ADR-0063 removed.

import { useEffect, useState } from 'react'

import { agentClient } from '../../api/clientFactory'

export default function ConnectionNotice() {
  const [reason, setReason] = useState<string | null>(agentClient.connectionError)
  // Remember the closed reason itself — a **different** reason shows again (closing must not become a permanent mute).
  const [dismissed, setDismissed] = useState<string | null>(null)

  useEffect(() => {
    // The state surface also notifies when the reason changes (ProtocolClient.reportConnectionError) — so a separate
    // subscription is unneeded and this one is enough.
    return agentClient.onConnectionStateChange(() => {
      const next = agentClient.connectionError
      setReason(next)
      // ★When the reason disappears, the closed memory is dropped too (load-bearing)★: if not dropped, in "failure → close → fix →
      //   connected → later recurrence with the **identical** wording" the strings are equal and nothing shows. A repeat of the same cause
      //   is in fact the common case, so closing must not permanently mute that cause.
      if (next === null) setDismissed(null)
    })
  }, [])

  if (!reason || reason === dismissed) return null

  return (
    <div
      role="alert"
      className="flex shrink-0 items-start gap-3 border-b border-border bg-elevated px-4 py-2 text-sm text-foreground"
      style={{ borderLeft: '3px solid var(--status-danger)' }}
    >
      {/* `wrap-anywhere` = a requirement of the covering layer (the `NoticeOverlay` doc) — the reason carries a long path. */}
      <div className="min-w-0 flex-1 wrap-anywhere">
        <span className="font-semibold">데몬에 연결하지 못했습니다.</span>{' '}
        <span className="opacity-90">{reason}</span>
      </div>
      <button
        type="button"
        aria-label="알림 닫기"
        className="shrink-0 rounded px-2 opacity-70 hover:opacity-100"
        onClick={() => setDismissed(reason)}
      >
        ✕
      </button>
    </div>
  )
}
