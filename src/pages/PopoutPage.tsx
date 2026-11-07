// PopoutPage — the page drawn by an OS window created at runtime by slot pop-out (pop-out) · blank window creation (create_window).
//
// ★Tab ownership model (ADR-0057)★: not the old "fixed single View" popup but an **ordinary window that holds tabs** (D-2 "popups have tabs too").
//   It fixes its own window label from the URL key `#/popup?window=<label>` and thinly wraps WindowLayout(label) —
//   tab bar · slot canvas · tab state learning · self-close at 0 tabs are all owned by that component (§7-1). PopoutPage is
//   a shell that only extracts the label and passes it on.
//
// ★view:closed retirement (G2)★: the old "window self-exit when its own view closes" (view:closed listener) was removed. Window closing
//   is the shell's job (closing the last tab · invoke `window_control` · bus `window.close` — TRD S26-window-chrome §5-4 · §5-5),
//   and the frontend's self-close is only asking the shell via WindowLayout's window:tabs-updated{tabs:[]} (0 tabs) signal (prevents double firing · re-entry).
//   This page has no self-exit logic.
//
// ★Output subscription is automatic★: this window's own TauriTransport singleton (a separate WebView2 process, so the module graph is fresh)
//   registers subscribe_output with its own window_label on the connected transition → the agent output of this window's tabs is
//   routed to this window's Channel (the backend OutputRouter general mechanism, walking all tabs). Slots subscribe through
//   agentClient the same as main and receive replay via request_replay (gen fence, ADR-0046) — no popup-specific wiring.
//
// ★§5★: creation/binding of this window itself can be LLM-controlled through commands (`slot.popout`·`window.create` = invoke).
//   This page is a pure I/O display surface (hands and feet) — control is held by the backend side (the brain).

import { useState } from 'react'

import ConnectionNotice from '../components/layout/ConnectionNotice'
import NoticeOverlay from '../components/layout/NoticeOverlay'
import WindowLayout from '../components/layout/WindowLayout'
// ★Single source★: parsing this window's label uses viewStore's shared helper (WindowLayout · useCurrentViewId ·
//   SlotContextMenu share the same decision — §5 control surface consistency).
import { readWindowLabelFromHash } from '../store/viewStore'

export default function PopoutPage() {
  // Fixed once from URL `?window=` — unchanged for the window's lifetime. If it is not a popup route it falls back to 'main', but
  //   this page only appears on the /popup route, so the normal path is the popup label.
  const [label] = useState<string>(readWindowLabelFromHash)

  return (
    <div
      style={{
        width: '100vw',
        height: '100vh',
        background: 'var(--bg)',
        display: 'flex',
        flexDirection: 'column',
      }}
    >
      {/* Wrapping area = the spot of `WindowLayout`. The notice layer attaches not to this area but to the area below the title bar inside `WindowLayout` (the same spot on every screen). */}
      <div style={{ flex: 1, minHeight: 0, position: 'relative' }}>
        <WindowLayout
          label={label}
          // Notices appear in every window — boot runs per window and failure reasons arrive per window (ADR-0134). They cover the layout without
          //   pushing it (ADR-0277 — revision of ADR-0180). The covering spot is below the title bar — the title bar is not covered (user decision
          //   2026-10-26 · T-66).
          notices={
            <NoticeOverlay>
              <ConnectionNotice />
            </NoticeOverlay>
          }
        />
      </div>
    </div>
  )
}
