// ADR-0303 — Chrome-style title bar
// TRD S26-window-chrome §7-2: window draggable area = Tauri `data-tauri-drag-region` (Q5 = ①). Press · double press (maximize toggle) are passed to the OS by Tauri
//   `drag.js` — they do not go through the bus (D7). Permission = `core:window:allow-start-dragging` (main · popout — §9).
// ★An attribute without a value = drags only when that element itself is pressed★ — walking upward from the pressed element, if the first mark met is not the element itself, it does not drag
//   (`drag.js` `isDragRegion`). So child tab heads · `+` · buttons do not start a drag. Changing to `"deep"` would make a tab head
//   press a window drag.
// ★Do not switch to CSS `app-region: drag`★ — hit testing in that region goes to the top-level window (HWND), reaches the drop-receiving side of tao that accepts only files,
//   and a tab dropped on it ends with no receiver (measured in a debug session 2026-10-24).
// ★A press on the draggable area does not reach the document's bubble-phase `mousedown`★ — `drag.js` (tauri 2.11.3 `src/window/scripts/drag.js`) attaches a bubble-phase `mousedown`
//   to document first as an initialization script, and for a left press on a draggable area it calls `preventDefault` and
//   `stopImmediatePropagation`. So the document · window bubble-phase `mousedown` attached after it are not called, and since the default
//   action (focus move) does not happen either, an input field does not lose focus. The side that closes on an outside press listens in the capture phase (`addEventListener(…, true)`) —
//   `pointerdown` is not blocked.

import { useEffect, type RefObject } from 'react'

const DRAG_REGION_ATTR = 'data-tauri-drag-region'
const DRAG_REGION = { [DRAG_REGION_ATTR]: '' } as const

export function dragProps(): typeof DRAG_REGION {
  return DRAG_REGION
}

/** Is the pressed element a draggable area — only the valueless mark is used, so this is the case where the element itself carries the mark (a press on a child is not a draggable area). */
function isDragRegionTarget(target: EventTarget | null): boolean {
  return target instanceof Element && target.hasAttribute(DRAG_REGION_ATTR)
}

/**
 * While `editing`, pressing a window draggable area takes focus off the input field `input` — that press does not move focus (head), so a field whose edit is
 * committed by focus loss would not end on a draggable area press without this. The commit itself is done by that field's blur handler. It is the capture phase, so it listens
 * before `drag.js`.
 */
export function useBlurOnDragRegionPress(editing: boolean, input: RefObject<HTMLInputElement | null>): void {
  useEffect(() => {
    if (!editing) return
    const onDown = (e: MouseEvent): void => {
      if (isDragRegionTarget(e.target)) input.current?.blur()
    }
    document.addEventListener('mousedown', onDown, true)
    return () => document.removeEventListener('mousedown', onDown, true)
  }, [editing, input])
}
