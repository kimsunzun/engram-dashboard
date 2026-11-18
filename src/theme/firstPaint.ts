// Tells the shell this webview's first screen (`webview_painted`) — the shell creates main · pop-out windows hidden and shows them on this notice (`src-tauri/src/first_paint.rs`; after a time limit if none arrives). A notice from a window the shell does not wait for (reload · pre-handoff warm-up window · `--hidden` boot) is ignored. The caller is the tab window's `WindowLayout`, after the first screen (tab bar + active tab) is committed, not the background-only screen before tabs arrive.
// State is module-scoped: the notice is once per page load (once more when a new adoption number arrives); a reload re-runs the module and resets it, and StrictMode's double effect still sends once.

import { invoke } from '@tauri-apps/api/core'

// Upper bound for sending when frames do not arrive while waiting two frames on a visible page — a page in a hidden window may answer visible while frame callbacks do not run (visibility of a page inside a hidden window · frame behavior [unverified]), so it is not dragged to the shell time limit (1500 ms). A hidden page's timer can be throttled, so it may be later than this.
const FRAME_WAIT_CAP_MS = 100

// Adoption number of the last notice sent — `undefined` = none sent yet.
let reported: number | null | undefined = undefined

/**
 * Call after the screen to be shown is committed (effect). Visible page → send after two frames (after that screen is drawn; after `FRAME_WAIT_CAP_MS` if no frame comes); hidden → send immediately — a page inside a hidden window appears hidden so frame callbacks may not run [unverified — whether WebView2 carries the parent window's hiding into page visibility], and sending now that the DOM is committed lets the first frame after it becomes visible draw that DOM.
 * `generation` = the adoption number this page received (`spare:adopted` body · `window_role` answer — TRD S21-slot-docking §3-9 I26) · `null` if none; carried in the notice — the adopting window's shell door accepts only notices carrying it. A page that already sent sends once more with a new number (the pre-adoption · boot-failure notice was discarded by the shell); repeated calls with the same number · `null` do not send.
 * A failure (outside the shell · command missing) leaves only a warning — the shell shows the window after the time limit anyway.
 */
// ADR-0299
export function reportFirstPaint(generation: number | null): void {
  if (reported !== undefined && (generation === null || generation === reported)) return
  reported = generation
  let sent = false
  const send = () => {
    if (sent) return
    sent = true
    invoke<void>('webview_painted', { generation }).catch(err => console.warn('[firstPaint] webview_painted failed:', err))
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
