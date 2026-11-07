// ADR-0055: minimal skeleton — saving a custom keymap (localStorage) is follow-up.

import { fireAndForget } from './dispatch'
import { getCommand } from './registry'

// ─────────────────────────────────────────────────────────────────────────────
// ★LOAD-BEARING focus guard (ADR-0055 · CLAUDE.md 「LLM-우선 제어」)★
//   The global keydown must not intercept shortcuts while the user is typing. ★If this guard has a hole, shortcuts
//   intercept terminal/input typing★ — e.g. the regression where pressing 't' in the terminal changes the theme.
// ─────────────────────────────────────────────────────────────────────────────
export function isEditableTarget(target: EventTarget | null): boolean {
  if (!target || typeof (target as Element).closest !== 'function') return false
  const el = target as Element

  const tag = el.tagName
  if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return true

  // ★contenteditable is decided by isContentEditable alone★ (FIX-A, load-bearing): HTMLElement.isContentEditable returns the
  //   *effective* editability per the HTML spec — it reports true for contenteditable="" / "true" / "plaintext-only"
  //   and also for editability inherited from an ancestor, and conversely sees a contenteditable="false" island as exactly false even
  //   under an editable ancestor. We used to add a closest('[contenteditable]:not([="false"])') fallback here,
  //   but closest passes the "false" boundary and matches the editable ancestor above it → creating a bug that swallows shortcuts even over a non-editable island (e.g. a
  //   contenteditable="false" button inside an editable div). isContentEditable already
  //   handles inheritance, so the fallback is redundant and a bug source → removed. An Element that is not an HTMLElement (SVG, etc.) lacks this property, so it is
  //   undefined→falsy, and since SVG is not an HTML-contenteditable target, that is the right answer.
  if ((el as HTMLElement).isContentEditable) return true

  // Inside the xterm.js terminal — shortcuts must not swallow terminal key input (load-bearing).
  if (el.closest('.xterm')) return true

  return false
}

/** KeyboardEvent → normalized combo string (e.g. 'ctrl+shift+t'). */
export function comboOf(e: KeyboardEvent): string {
  const parts: string[] = []
  if (e.ctrlKey) parts.push('ctrl')
  if (e.altKey) parts.push('alt')
  if (e.shiftKey) parts.push('shift')
  if (e.metaKey) parts.push('meta')
  // Normalize key to lowercase (avoids uppercase with shift combinations).
  const key = e.key.toLowerCase()
  if (key !== 'control' && key !== 'alt' && key !== 'shift' && key !== 'meta') {
    parts.push(key)
  }
  return parts.join('+')
}

// Minimal skeleton — a custom keymap is follow-up.
//
// ★Do not leave an id that does not exist here★: once a combination is caught, the listener below even does `preventDefault` and then run
// throws — the key is consumed but nothing happens and the only signal is one console line. (Ctrl+Shift+T was like that —
// when ADR-0167 took down the theme command it pointed at, this line came down with it.)
const BINDINGS: Record<string, string> = {
  // ★Ctrl+Tab (D-8, ADR-0057)★: tab.next goes by the same path as a human click (switch_tab).
  'ctrl+tab': 'tab.next',
}

let paused = false

/**
 * Stops or re-enables shortcut firing for this window. Owner = main's restore modal — `true` while it is up, `false` once closed.
 * If the modal blocks only the mouse, shortcuts change the screen beneath it (TRD S21-storage §6-7 N4). Separate per window (this module is one per webview).
 */
export function setKeybindingsPaused(next: boolean): void {
  paused = next
}

/**
 * Attach the listener by an explicit call, not at import time (wiring · cleanup under control).
 * Returns = disposer (removes the listener). Call it on HMR/unmount to prevent duplicate accumulation.
 */
export function installKeybindings(): () => void {
  const onKeyDown = (e: KeyboardEvent): void => {
    if (paused) return
    // ★Focus guard first★ (the invariant above).
    if (isEditableTarget(e.target)) return

    const id = BINDINGS[comboOf(e)]
    if (!id) return

    // ★The when gate lives only here (the keybinding consumer)★ (FIX-5, VS Code semantics): when is a "UI context gate", not
    //   the command's own executability condition. A *context-triggered* consumer like keys/palette passes over the command as if it did not see it
    //   when when is false, letting the key flow through. In contrast, an explicit run('id') · __engramCmd · cdp
    //   call runs unconditionally (the caller has already decided) → registry.run() does not read when.
    const cmd = getCommand(id)
    if (cmd?.when) {
      // ★when is evaluated defensively★ (FIX-B): if a user-provided when throws, an uncaught exception
      //   leaks out of the global keydown listener (fatal because it is a load-bearing global listener). Per VS Code when-clause semantics,
      //   treat throw = "context not met = false" → skip the command and let the key pass through as is.
      let ok = false
      try {
        ok = cmd.when()
      } catch {
        ok = false
      }
      if (!ok) return
    }

    e.preventDefault()
    // ★Run as fire-and-forget★ (FIX-3/4): since we do not wait for the result, use the shared helper that swallows both sync throw and async reject
    //   (so the listener does not die). Do not call run() directly — prevents reimplementing the safety net.
    fireAndForget(id)
  }

  // ★The listener is attached to document★ (FIX-2, ADR-0055 contract): the global keydown receives bubbling at the document level.
  //   add/remove must pass the same handler reference for cleanup to hold (the disposer removes exactly this listener).
  document.addEventListener('keydown', onKeyDown)
  return () => document.removeEventListener('keydown', onKeyDown)
}
