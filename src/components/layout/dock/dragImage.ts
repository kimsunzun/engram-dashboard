// TRD S21-slot-docking §4-2 「그림」(PRD R29 · VS Code `applyDragImage` · 조사 F): 탭 끌기 그림 = 탭 이름 라벨. dragstart 안에서
//   동기로 라벨을 붙이고 `setDragImage` 에 넘긴 뒤 다음 task 에 뗀다. 모양(반투명 · 흐려짐)은 엔진이 그 요소를 떠 OS 가 그린다.
// ★`setDragImage` 는 dragstart 안에서만 부를 수 있고 그 순간 그려진 요소를 뜬다★ — 그래서 여기는 전부 동기다.
// ★화면 밖에 붙인다★ — `display: none` · `visibility: hidden` 이면 엔진이 뜰 그림이 없다(옛 HTML5 끌기 그림 상자 실측).
// 실패는 밖으로 새지 않는다 — 엔진 기본 그림(끌린 머리)으로 물러나고 끌기는 이어진다(PRD R2).

/** 커서에서 라벨까지 — VS Code `setDragImage(dragImage, -10, -10)`(라벨이 커서 오른쪽 아래에 선다). */
export const DRAG_IMAGE_OFFSET = { x: -10, y: -10 } as const

/** VS Code `.monaco-drag-image` 의 꼴 — 색은 테마 토큰. */
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

/** dragstart 안에서 부른다. `later` = 다음 task 에 부를 것(시험이 가짜를 꽂는다). 던지지 않는다. */
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
