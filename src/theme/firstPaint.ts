// Tells the shell this webview's first screen (`webview_painted`) — the shell creates main · pop-out windows hidden and shows the window on this notice
// (`src-tauri/src/first_paint.rs` — after a time limit if no notice arrives). A notice from a window the shell does not wait for (reload · pre-handoff warm-up window ·
// `--hidden` boot) is ignored by the shell. The caller is the tab window's `WindowLayout` — it is called after the first screen (tab bar + active tab) has been committed, not
// the screen with only a background before tabs are received.
// Why state is module-scoped: the notice is once per page load (once more when a new adoption number arrives), and a reload re-runs the module and resets it together —
//   even if StrictMode runs the effect twice, it is sent only once.

import { invoke } from '@tauri-apps/api/core'

// Upper bound after which we send when frames do not arrive while waiting for two frames on a visible page — so that when the page answers visible but frame callbacks do not
//   run (visibility of a page inside a hidden window · how frames behave is [unverified]) it is not dragged all the way to the shell time limit (1500 ms). The timer of a hidden page
//   can be throttled, so it may be later than this value.
const FRAME_WAIT_CAP_MS = 100

// Adoption number of the last notice sent — `undefined` = none sent yet.
let reported: number | null | undefined = undefined

/**
 * Call it after the screen to be shown has been committed (effect). If the page is visible, send after two frames (after that screen has been drawn — after `FRAME_WAIT_CAP_MS` if no frame
 * comes), and immediately if hidden — a page inside a hidden window appears hidden so frame callbacks may not run
 * [unverified — whether WebView2 carries the parent window's hiding into page visibility], and then we send now that the DOM is committed so the first
 * frame after it becomes visible draws that DOM.
 * `generation` = the adoption number this page received (`spare:adopted` body · `window_role` answer — TRD S21-slot-docking §3-9 I26) · `null` if none.
 * It is carried in the notice — the shell door of the adopting window accepts only notices carrying that number. A page that already sent is sent once more if called with a new number
 * (the pre-adoption notice · boot-failure notice was discarded by the shell). Repeated calls with the same number · `null` do not send.
 * A failure (outside the shell · command missing) is left only as a warning — the shell shows the window after the time limit anyway.
 */
// ADR-0299
export function reportFirstPaint(generation: number | null): void {
  if (reported !== undefined && (generation === null || generation === reported)) return
  reported = generation
  let sent = false
  const send = () => {
    if (sent) return
    sent = true
    invoke<void>('webview_painted', { generation }).catch(err => console.warn('[firstPaint] webview_painted 실패:', err))
  }
  if (document.visibilityState === 'visible') {
    requestAnimationFrame(() => requestAnimationFrame(send))
    setTimeout(send, FRAME_WAIT_CAP_MS)
  } else {
    send()
  }
}

/** Test only — reset module state (isolation between tests). Do not call from production code. */
export function __resetFirstPaintForTest(): void {
  reported = undefined
}
