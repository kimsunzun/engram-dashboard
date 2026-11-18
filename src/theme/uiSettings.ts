// Attaches the per-window theme decided by the shell: window theme (`window.setTheme`) ?? shell setting `theme.default`, computed by the shell (ADR-0265). Only a single theme field today.
// ★Changes only the value★: pours the received theme into themeManager and swaps one `data-theme`. No remount · key swap · router navigation on this path — chat is component state, so a slot remount loses the conversation permanently (ADR-0149).
// ★Calls invoke/listen directly★: an exception to "components · stores depend only on agentClient" — a backend-authority surface uses Tauri directly (same as `viewStore`'s layout; the value's owner is the shell, not the daemon).
// ★Read-only path★: touching the theme on screen saves nowhere and the shell's next push overwrites it; the only ways to change it are the shell commands global `settings.set theme.default` · per-window `window.setTheme`.

import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

import { isThemeName, type ThemeName } from '../store/themeStore'
import { retryAsync, RetryCancelledError } from '../util/retryInvoke'
import { themeManager } from './ThemeManager'

// The notice the shell pushes. ★Each window gets its own value★ — the shell sends it naming the destination (`src-tauri/src/commands/settings.rs`).
const EVT_UI_SETTINGS_UPDATED = 'ui:settings-updated'
// Boot query; same payload shape as the push (`{theme}`). ★No window argument★ — Tauri supplies the asking window on the shell side (a wrongly written label would take another window's theme).
const CMD_GET_UI_SETTINGS = 'get_ui_settings'

const FALLBACK: ThemeName = 'dark'

// ★The shell already folds it into one of three values — this check is a net for a broken contract★: a name not in `data-theme` would be planted and styles would not attach. `null` = unusable → the caller goes to `keepCurrent`.
function themeOf(payload: unknown, where: string): ThemeName | null {
  const raw = (payload as { theme?: unknown } | null | undefined)?.theme
  if (isThemeName(raw)) {
    return raw
  }
  console.warn(`[uiSettings] ${where}: unusable theme ${JSON.stringify(raw)} — keeping the current theme`)
  return null
}

// Shell value not received — keep the attached theme (planted by the shell at window creation or received earlier). ★Do not overwrite with FALLBACK★: a light · e-ink window would turn dark. FALLBACK applies only when nothing is attached (no theme → text · border tokens empty → black on black, `main.tsx`).
function keepCurrent(): void {
  if (!isThemeName(document.documentElement.dataset.theme)) {
    themeManager.apply(FALLBACK)
  }
}

/**
 * One boot query + the update subscription; returns a disposer (prevents listener accumulation on unmount/HMR). Called once per window — main · pop-out · tree each pull their own value.
 */
export function installUiSettings(): () => void {
  let disposed = false
  let unlisten: UnlistenFn | undefined
  // ★Latch so the boot query cannot overwrite a push★: a push (`window.setTheme` · `theme.default` write) during the query round trip makes the query answer staler; letting arrival order win would make the command succeed while the screen does not change.
  let pushed = false

  void (async () => {
    // ★Issue the boot query **after** the subscription registration finishes★: side by side, a push arriving before registration completes never reaches this window (no listener) and the late query answer paints the old value; the `pushed` latch stands only on a received notice, so only order stops it. Nothing is lost: the query reads the shell's current value after registration.
    // ★Once refused, the window stays permanently deaf — so a bounded retry★: if only the subscription fails, the first value paints fine and the screen looks healthy, but the window misses every later push and diverges with no signal. The event plane not being ready is a transient failure right after boot (same judgment as the boot pull in `App.tsx` — ADR-0102). If retries are exhausted it is **surfaced as an error**, not a warn — this window not receiving updates is then certain.
    // ★The retry covers **the subscription only**★: the boot query below is a bare `invoke`; if it fails once it keeps the current theme and the next push sets the value right.
    // ★The retry covers **refusal** only, not no response (hang)★ (known residual): with no timeout, a `listen()` that never resolves keeps the loop on the first attempt and blocks the boot query with it (order dependence). Do not read it as "registration eventually happens".
    try {
      // ★★Do not drop `target`★★ — otherwise this window receives **other windows' values too** and paints the last one: `listen()` defaults to `Any`, and Tauri wakes `Any` listeners regardless of the filter (`match_any_or_filter`); the shell sends each window its own value with `emit_to(label, …)`, so one push over three windows would deliver three values here. ★The label comes from Tauri★ (the shell uses the same value — `Window::label()` in `commands/settings.rs`).
      const off = await retryAsync(
        () =>
          listen<unknown>(
            EVT_UI_SETTINGS_UPDATED,
            e => {
              // ★Discard anything arriving after cleanup★: if dispose runs while `listen()` is pending, registration is done but not yet at `off()`; delivery in that gap lets a **dead instance** (first StrictMode round · previous HMR edition) overwrite the live window's theme, which `off()` alone cannot stop.
              if (disposed) return
              pushed = true
              const theme = themeOf(e.payload, EVT_UI_SETTINGS_UPDATED)
              if (theme === null) {
                keepCurrent()
              } else {
                themeManager.apply(theme)
              }
            },
            { target: getCurrentWindow().label },
          ),
        {
          isCancelled: () => disposed,
          onRetry: (err, attempt) => {
            console.warn(`[uiSettings] retrying the ${EVT_UI_SETTINGS_UPDATED} subscription #${attempt}:`, err)
          },
        },
      )
      // Dispose before registration finishes: the handle is not received yet so unlisten cannot be called — release it where it is received (same clause in viewStore).
      if (disposed) {
        off()
        return
      }
      unlisten = off
    } catch (e) {
      // Issue the boot query even if the subscription dies — even if updates cannot be received, the first value must be right.
      if (!(e instanceof RetryCancelledError)) {
        console.error(
          `[uiSettings] subscribing to ${EVT_UI_SETTINGS_UPDATED} failed — this window will not receive later theme pushes (window.setTheme · theme.default writes):`,
          e,
        )
      }
    }

    let theme: ThemeName | null = null
    try {
      theme = themeOf(await invoke<unknown>(CMD_GET_UI_SETTINGS), CMD_GET_UI_SETTINGS)
    } catch (e) {
      console.warn(`[uiSettings] ${CMD_GET_UI_SETTINGS} failed — keeping the current theme:`, e)
    }
    if (disposed || pushed) return
    if (theme === null) {
      keepCurrent()
    } else {
      themeManager.apply(theme)
    }
  })()

  return () => {
    disposed = true
    unlisten?.()
    unlisten = undefined
  }
}
