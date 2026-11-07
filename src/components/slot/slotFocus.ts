// ADR-0237 (U6 「칸 안 어디든」 (anywhere in the area)): if a button in the area disappears or locks after being pressed, focus falls to document body, and that area's Esc
//   does not reach the area root's capture (interrupt becomes unresponsive). Moves the focus the button held to the nearest 「클릭으로 포커스를
//   받는 칸 컨테이너」 (area container that takes focus on click; tabindex -1 — scroll viewport · RichSlot root).

/**
 * Moves only if `button` currently holds focus — if not, there is nothing to lose, and stealing another area's focus makes this
 * area's turn get interrupted by Esc instead of that area's. If there is no ancestor to receive it, does nothing. Call after the button's action (inside the same handler).
 */
export function keepFocusInSlot(button: HTMLElement): void {
  if (button.ownerDocument.activeElement !== button) return
  button.parentElement?.closest<HTMLElement>('[tabindex="-1"]')?.focus({ preventScroll: true })
}
