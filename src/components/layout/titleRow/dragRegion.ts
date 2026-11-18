// ADR-0303 — Chrome-style title bar
// TRD S26-window-chrome §7-2: window draggable area = Tauri `data-tauri-drag-region` (Q5 = ①). Press · double press (maximize toggle) are passed to the OS by Tauri `drag.js` — not through the bus (D7). Permission = `core:window:allow-start-dragging` (main · popout — §9).
// ★A valueless attribute = drags only when that element itself is pressed★ — walking upward from the pressed element, if the first mark met is not the element itself it does not drag (`drag.js` `isDragRegion`), so child tab heads · `+` · buttons do not start a drag. `"deep"` would make a tab head press a window drag.
// ★Do not switch to CSS `app-region: drag`★ — hit testing in that region goes to the top-level window (HWND), reaches the file-only drop receiver of tao, and a tab dropped on it ends with no receiver (measured).
// ★A press on the draggable area does not reach the document's bubble-phase `mousedown`★ — `drag.js` (tauri 2.11.3 `src/window/scripts/drag.js`) attaches a bubble-phase `mousedown` to document first as an initialization script and for a left press on a draggable area calls `preventDefault` and `stopImmediatePropagation`. So bubble-phase `mousedown` listeners on document · window attached later are not called, and since the default action (focus move) does not happen either, an input field keeps focus. The side that closes on an outside press listens in the capture phase (`addEventListener(…, true)`); `pointerdown` is not blocked.

import { useEffect, type RefObject } from 'react'

const DRAG_REGION_ATTR = 'data-tauri-drag-region'
const DRAG_REGION = { [DRAG_REGION_ATTR]: '' } as const

export function dragProps(): typeof DRAG_REGION {
  return DRAG_REGION
}

/** Is the pressed element a draggable area — only the valueless mark is used, so only an element carrying the mark itself counts (a press on a child is not). */
function isDragRegionTarget(target: EventTarget | null): boolean {
  return target instanceof Element && target.hasAttribute(DRAG_REGION_ATTR)
}

/**
 * While `editing`, pressing a window draggable area takes focus off the input `input` — that press does not move focus, so a field whose edit commits on focus loss would not end without this. The commit is done by that field's blur handler. Capture phase, so it listens before `drag.js`.
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
