// TRD S21-slot-docking §4-1 · §8-5 M10 · PRD AC22: if a file · URL is dragged and dropped onto our window, the webview navigates to it and the app screen disappears — that drop is
//   blocked. Always on (independent of the drag session).
// ★Only drags carrying a file · URL are blocked★ — those two are the only ones the webview navigates on drop in a window with the drop handler off (Chromium
//   `DragData::ContainsURL`). Blocking other drags too would reject the default behavior of dropping text into an input (chat composer, etc.).
// ★It does not stop propagation (no `stopPropagation`)★ — the react-dnd HTML5 backend of the agent tree receives its own drag through a bubble listener on the root
//   `window`.

/** Attach the block to this document. Return = detach. */
export function installFileDropGuard(): () => void {
  // ★`dragenter` is blocked too★ — on a step where the element under the cursor changed, Chromium emits only `dragenter` (`EventHandler::UpdateDragAndDrop`).
  //   If that is not blocked, during that step the document is treated as not accepting the drag, so copy stands for file · URL, and releasing in that gap (~16–60 ms) goes
  //   to navigation without a `drop` event (`PerformDrop`) — the `drop` block below does not run.
  const onEnterOver = (e: DragEvent): void => {
    const dt = navigatingTransfer(e)
    if (dt === null) return
    e.preventDefault()
    dt.dropEffect = 'none'
  }
  const onDrop = (e: DragEvent): void => {
    if (navigatingTransfer(e) !== null) e.preventDefault()
  }
  document.addEventListener('dragenter', onEnterOver)
  document.addEventListener('dragover', onEnterOver)
  document.addEventListener('drop', onDrop)
  return () => {
    document.removeEventListener('dragenter', onEnterOver)
    document.removeEventListener('dragover', onEnterOver)
    document.removeEventListener('drop', onDrop)
  }
}

function navigatingTransfer(e: DragEvent): DataTransfer | null {
  const dt = e.dataTransfer
  if (dt === null) return null
  const types = Array.from(dt.types)
  return types.includes('Files') || types.includes('text/uri-list') ? dt : null
}
