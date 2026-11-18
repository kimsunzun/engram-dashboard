// ADR-0237: a pure predicate (zero DOM) deciding whether one key inside the chat area (RichSlot) means "interrupt the running turn". Canonical firing conditions = `docs/process/S21-chat-ux/trd.md` §3-2 — this file carries that list over as is.
// When T-42 (shortcut system — on hold) sets up a keybinding table, the area condition (the `ctx` side) becomes the `when` of one line there. ★The predicate does not move over wholesale★ — `when` is a `() => boolean` that receives no event (`commands/registry.ts`) and the global listener skips the edit target (the input box), so the key-shape verdict (key · modifier · repeat · IME · `defaultPrevented` · target) stays in that table's matcher or the current local capture. The interrupt action lives in the command `agent.interrupt`, so nothing else carries over.

/** Scope where Esc takes effect — `'slot'` = anywhere in the area (input box + conversation body) · `'input'` = only when the input box has focus. */
export type InterruptEscScope = 'slot' | 'input'

/** U6 = anywhere in the area. */
export const ESC_SCOPE: InterruptEscScope = 'slot'

/**
 * Marker of an overlay under which Esc does not interrupt the turn while open. The three closed by a document-global Esc (agent picker · tree row menu · preset row menu) and the right-click menu attach it to their root — that Esc closes the overlay.
 */
export const OVERLAY_SELECTOR = '[data-engram-overlay]'

/** Only the fields the predicate reads — a React `KeyboardEvent` fits as is. */
export interface InterruptKeyEvent {
  key: string
  ctrlKey: boolean
  altKey: boolean
  shiftKey: boolean
  metaKey: boolean
  repeat: boolean
  keyCode: number
  defaultPrevented: boolean
  target: EventTarget | null
  nativeEvent: { isComposing: boolean }
}

export interface InterruptKeyContext {
  /** The area looks like it is waiting for or receiving a response (RichSlot's `streaming`). */
  streaming: boolean
  /**
   * An interrupt was already sent and awaits that turn's end (`store/interruptStore.ts`). ADR-0244: an Esc meanwhile is ignored — a resent interrupt could land on the turn opened by the next text that was waiting.
   */
  interrupting: boolean
  /** The condition under which the absence veil stands (exited · disconnected · subscription stopped). */
  agentUnavailable: boolean
  /** That agent's channel supports interrupt (`capabilities.control.interrupt`). */
  canInterrupt: boolean
  /** The document has [`OVERLAY_SELECTOR`] — the caller measures and passes it. */
  overlayOpen: boolean
  scope: InterruptEscScope
  /** The area's input box node — read only when `scope === 'input'`. */
  textarea: EventTarget | null
}

export function isInterruptEscape(e: InterruptKeyEvent, ctx: InterruptKeyContext): boolean {
  if (e.key !== 'Escape') return false
  if (e.ctrlKey || e.altKey || e.shiftKey || e.metaKey) return false
  if (e.repeat) return false
  // Esc during composition cancels only the composition — the same formula as the IME verdict of the input box Enter (WebView2 carries it as keyCode 229).
  if (e.nativeEvent.isComposing || e.keyCode === 229) return false
  // A Radix layer takes it first in the document capture — that Esc is the key that closes the layer.
  if (e.defaultPrevented) return false
  if (ctx.overlayOpen) return false
  if (!ctx.streaming || ctx.interrupting || ctx.agentUnavailable || !ctx.canInterrupt) return false
  if (ctx.scope === 'input' && e.target !== ctx.textarea) return false
  return true
}
