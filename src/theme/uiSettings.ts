// Wiring that attaches the theme the shell decided per window to the screen — that window's value = window theme (`window.setTheme`) ?? shell setting
// `theme.default`, and the shell computes it for us (ADR-0265). What is carried today is a single theme field.
//
// ★Changes only the value★: pours the received theme into themeManager and swaps one `data-theme`. Do not put a remount · key swap · router
// navigation onto this path — chat is component state, so if a slot is remounted the conversation is permanently lost (ADR-0149).
//
// ★Calls invoke/listen directly★: an exception to 「컴포넌트·스토어는 agentClient 에만 의존한다」 (components · stores depend only on agentClient), a
// surface whose authority is the backend uses Tauri directly (the same exception layout uses in `viewStore` — the owner of this value is the shell, not the daemon, so
// agentClient is not what should carry it).
//
// ★This path is read-only★: touching the theme on screen saves it nowhere and the shell's next push overwrites it. The ways to
// change the value are only two shell commands — global `settings.set theme.default` · per-window `window.setTheme`.
// ADR-0149

import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

import { isThemeName, type ThemeName } from '../store/themeStore'
import { retryAsync, RetryCancelledError } from '../util/retryInvoke'
import { themeManager } from './ThemeManager'

// The notice the shell pushes. ★Each window gets its own value★ — the shell sends it naming the destination (`src-tauri/src/commands/settings.rs`).
const EVT_UI_SETTINGS_UPDATED = 'ui:settings-updated'
// Boot query. Returns the same shape as the push (`{theme}`) — the shell uses the same payload struct in both places.
// ★No window argument is passed★ — Tauri supplies which window asked on the shell side (if the webview declared it, one wrongly written label
// would take another window's theme).
const CMD_GET_UI_SETTINGS = 'get_ui_settings'

const FALLBACK: ThemeName = 'dark'

// ★The shell already folds it into one of three values before giving it — this check is a net for when that contract breaks★. If it passed silently here,
//   a name not in `data-theme` would be planted and styles would not attach at all. `null` = unusable — the caller goes to `keepCurrent`.
function themeOf(payload: unknown, where: string): ThemeName | null {
  const raw = (payload as { theme?: unknown } | null | undefined)?.theme
  if (isThemeName(raw)) {
    return raw
  }
  console.warn(`[uiSettings] ${where}: 쓸 수 없는 테마 ${JSON.stringify(raw)} — 지금 테마를 둔다`)
  return null
}

// When the shell value was not received — keep the currently attached theme as is (the window's theme the shell planted when creating the window · a value received earlier). ★Do not overwrite with FALLBACK★ —
//   a light · e-ink window would turn dark and show colors that are not that window's theme. FALLBACK applies only when nothing is attached
//   (with no theme the text · border color tokens are entirely empty, giving black text on a black background — `main.tsx`).
function keepCurrent(): void {
  if (!isThemeName(document.documentElement.dataset.theme)) {
    themeManager.apply(FALLBACK)
  }
}

/**
 * Does one boot query + hooks the update subscription. The return value is a disposer (prevents listener duplicate accumulation on unmount/HMR).
 *
 * Called once per window — main · pop-out · tree each pull their own value.
 */
export function installUiSettings(): () => void {
  let disposed = false
  let unlisten: UnlistenFn | undefined
  // ★The latch that keeps the boot query from overwriting a push★: if a push (`window.setTheme` · `theme.default` write) comes in during the query round trip,
  //   the query answer is the staler value. Letting arrival order win makes that command succeed only in its answer while the screen does not change —
  //   a reasonless non-response, so the caller wanders suspecting the command · setting.
  let pushed = false

  void (async () => {
    // ★Issue the boot query **after** the subscription registration finishes — do not launch the two side by side★.
    //   Side by side, a push (`window.setTheme` · `theme.default` write) notice that arrives while registration is not yet done **does not even arrive**
    //   at this window (no listener) and the query answer that comes late afterwards paints the old value. The `pushed` latch above stands **only when a notice is received**,
    //   so it cannot stop that interleave — only order stops it.
    //   Reversing the order loses nothing: even if a notice was missed before registration, the query reads the shell's current value **after that**, so
    //   it fetches the same (or a newer) value.
    // ★Once refused, that window becomes permanently deaf — so a bounded retry is put on★.
    //   If the query succeeds and only the subscription fails, the first value is painted fine and **the screen looks healthy**. After that, that window alone misses every
    //   push (`window.setTheme` · `theme.default` write) and diverges from other windows forever with no signal.
    //   The event plane not being ready is a transient failure right after boot, the kind that retry cures (by the same judgment the boot pull in `App.tsx` uses this helper
    //   — ADR-0102).
    //   If retries are exhausted too, **it is not passed over silently but surfaced as an error** — from then on this window not receiving
    //   updates is certain, so it is not a warn.
    // ★The retry covers **the subscription only**★ — the boot query below is a bare `invoke`. If it fails once it leaves the current theme
    //   as is, and the next push (`window.setTheme` · `theme.default` write) sets the value right (the subscription is already up by then).
    // ★What this retry covers is **refusal** only — it cannot cover no response (hang)★ (known residual). With no timeout,
    //   if `listen()` never resolves the retry loop stays stuck at the first attempt, and the boot query below is blocked with it
    //   (order dependence — the item right above). Do not read it as 「재시도가 있으니 등록은 결국 된다」 (there is a retry so registration eventually happens).
    try {
      // ★★Do not drop `target` — otherwise this window receives **other windows' values too** and paints the last one★★
      //   The default target of `listen()` is `Any`, and Tauri wakes listeners registered as `Any` **all of them regardless of the filter**
      //   (`match_any_or_filter`). The shell sends each window its own value with `emit_to(label, …)`, so with a default
      //   registration one push over three windows makes this window receive three values and **the earlier ones become meaningless**.
      //   ★The label comes from Tauri★ — the shell uses the same value (`Window::label()` in `commands/settings.rs`).
      const off = await retryAsync(
        () =>
          listen<unknown>(
            EVT_UI_SETTINGS_UPDATED,
            e => {
              // ★Anything that arrives after cleanup is discarded★: if dispose runs while `listen()` is pending, a gap arises where registration is already done but
              //   it has not yet reached the `off()` below. If delivered in that gap, a **dead instance**
              //   (the first StrictMode round · a previous HMR edition) overwrites the live window's theme. `off()` alone
              //   cannot stop that one case.
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
            console.warn(`[uiSettings] ${EVT_UI_SETTINGS_UPDATED} 구독 재시도 #${attempt}:`, err)
          },
        },
      )
      // If dispose runs before registration finishes, the handle has not been received yet so unlisten cannot be called — release it right at the spot it is received
      // (the same clause in viewStore).
      if (disposed) {
        off()
        return
      }
      unlisten = off
    } catch (e) {
      // Issue the boot query even if the subscription dies — even if updates cannot be received, the first value must be right.
      if (!(e instanceof RetryCancelledError)) {
        console.error(
          `[uiSettings] ${EVT_UI_SETTINGS_UPDATED} 구독 실패 — 이 창은 이후 테마 밀기(window.setTheme · theme.default 쓰기)를 못 받는다:`,
          e,
        )
      }
    }

    let theme: ThemeName | null = null
    try {
      theme = themeOf(await invoke<unknown>(CMD_GET_UI_SETTINGS), CMD_GET_UI_SETTINGS)
    } catch (e) {
      console.warn(`[uiSettings] ${CMD_GET_UI_SETTINGS} 실패 — 지금 테마를 둔다:`, e)
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
