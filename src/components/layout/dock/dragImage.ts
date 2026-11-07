// TRD S21-slot-docking §4-2 「그림」 (PRD R29 · VS Code `applyDragImage` · research F): tab drag image = tab name label. Inside dragstart,
//   attach the label synchronously, pass it to `setDragImage`, then remove it in the next task. The look (translucent · faded) is captured from that element by the engine and drawn by the OS.
// ★`setDragImage` can only be called inside dragstart and captures the element drawn at that moment★ — so everything here is synchronous.
// ★Attach it off screen★ — with `display: none` · `visibility: hidden` the engine has no image to capture (measured on the old HTML5 drag image box).
// Failure does not leak out — it falls back to the engine default image (the dragged head) and the drag continues (PRD R2).

/** From the cursor to the label — VS Code `setDragImage(dragImage, -10, -10)` (the label stands at the lower right of the cursor). */
export const DRAG_IMAGE_OFFSET = { x: -10, y: -10 } as const

/** The look of VS Code `.monaco-drag-image` — colors are theme tokens. */
const LABEL_STYLE: Partial<CSSStyleDeclaration> = {
  position: 'fixed',
  left: '-10000px',
  top: '0px',
  display: 'inline-block',
  boxSizing: 'border-box',
  maxWidth: '120px',
  padding: '1px 7px',
  borderRadius: '10px',
  fontSize: '12px',
  fontFamily: 'var(--font-ui)',
  whiteSpace: 'nowrap',
  overflow: 'hidden',
  textOverflow: 'ellipsis',
  background: 'var(--surface-elevated)',
  color: 'var(--text)',
  pointerEvents: 'none',
}

/** Called inside dragstart. `later` = what to call in the next task (the test plugs in a fake). Does not throw. */
export function setTabDragImage(
  dt: Pick<DataTransfer, 'setDragImage'>,
  label: string,
  later: (fn: () => void) => void = fn => void setTimeout(fn, 0),
  doc: Document = document,
): void {
  let el: HTMLDivElement | null = null
  try {
    el = doc.createElement('div')
    el.setAttribute('aria-hidden', 'true')
    el.dataset.tabDragImage = ''
    el.textContent = label
    Object.assign(el.style, LABEL_STYLE)
    doc.body.append(el)
    dt.setDragImage(el, DRAG_IMAGE_OFFSET.x, DRAG_IMAGE_OFFSET.y)
  } catch (err) {
    console.warn('[dragImage] 라벨 그림 실패 — 엔진 기본 그림으로 물러난다:', err)
  } finally {
    const placed = el
    if (placed !== null) later(() => placed.remove())
  }
}
