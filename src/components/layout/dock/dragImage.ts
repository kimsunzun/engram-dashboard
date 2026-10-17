// TRD S21-slot-docking §4-2: 탭 끌기 그림 — 화면 밖에 붙여 둔 상자 하나를 `dragstart` 안에서 동기로 채워
//   `setDragImage` 에 넘긴다. 모양(반투명 · 흐려짐)은 엔진이 그 상자를 떠 OS 가 그린다.
// ★`setDragImage` 는 `dragstart` 안에서만 부를 수 있고 그 순간 그려진 요소를 쓴다★ — 그래서 여기 함수는 전부
//   동기이고 셸에 묻지 않는다. 크기는 놓기 판정과 무관한 모양 값이라 웹뷰가 정한다(TRD 결정 18).
// ★끌기는 늘 이어진다(PRD R2)★ — 축소 그림이 던지면 제목만 상자로, 그것도 던지면 엔진 기본 그림(끌린 머리 요소)으로
//   물러난다. 이 파일의 어떤 실패도 `setDockDragImage` 밖으로 새지 않는다.

import type { PointCss } from '../../../../src-tauri/bindings/PointCss'

/** 논리(CSS) px 크기. */
export interface SizePx {
  w: number
  h: number
}

/** 새 창 크기에 곱하는 비율(TRD §4-2). */
export const DRAG_IMAGE_SCALE = 0.4
/** 그림 상한 — 비율을 지킨 채 이 안에 들게 줄인다. */
export const DRAG_IMAGE_MAX: Readonly<SizePx> = { w: 320, h: 240 }
/** 그림 너비 하한 — 이보다 좁으면 상한 안에서 비율대로 키운다. 제목만 상자의 너비이기도 하다. */
export const DRAG_IMAGE_MIN_W = 180
/** 위 머리 띠(탭 제목) 높이 — 제목만 상자의 높이이기도 하다. */
export const DRAG_IMAGE_TITLE_H = 28
/** 상자 안 커서 자리 — 머리 띠 왼쪽 안이라 제목이 커서 옆에 보인다. `dock_drag_start` 의 `hotspot` 으로 셸에 간다. */
export const DRAG_IMAGE_HOTSPOT: Readonly<PointCss> = { x: 16, y: 14 }
// 셸 `POPOUT_DEFAULT_SIZE`(`src-tauri/src/commands/popout.rs`)의 사본 — 메인에서 뗀 탭이 새 창이 될 때의 크기(PRD R22).
//   갈라져도 그림 크기만 어긋난다(놓기 판정 · 새 창 크기는 셸이 정한다).
const MAIN_NEW_WINDOW_SIZE: Readonly<SizePx> = { w: 720, h: 500 }

const TITLE_ONLY: Readonly<SizePx> = { w: DRAG_IMAGE_MIN_W, h: DRAG_IMAGE_TITLE_H }

/** 그 자리에 놓아 새 창이 될 때의 크기(PRD R22) — 메인이면 팝아웃 기본 크기, 팝아웃이면 그 창 안쪽 크기. */
export function newWindowSize(isMain: boolean, inner: SizePx): SizePx {
  return isMain ? { ...MAIN_NEW_WINDOW_SIZE } : { w: inner.w, h: inner.h }
}

/**
 * 끌기 그림 상자 크기(정수 CSS px) — 위 머리 띠까지 친 전체다. `newWindow` = 새 창 크기(`newWindowSize`) ·
 * `null` = 축소 그림 없이 제목만(180×28). 양수가 아닌 크기도 제목만이다.
 */
export function dragImageSize(newWindow: SizePx | null): SizePx {
  if (newWindow === null || !(newWindow.w > 0) || !(newWindow.h > 0)) return { ...TITLE_ONLY }
  let w = newWindow.w * DRAG_IMAGE_SCALE
  let h = newWindow.h * DRAG_IMAGE_SCALE
  const shrink = Math.min(1, DRAG_IMAGE_MAX.w / w, DRAG_IMAGE_MAX.h / h)
  w *= shrink
  h *= shrink
  if (w < DRAG_IMAGE_MIN_W) {
    const grow = Math.min(DRAG_IMAGE_MIN_W / w, DRAG_IMAGE_MAX.w / w, DRAG_IMAGE_MAX.h / h)
    if (grow > 1) {
      w *= grow
      h *= grow
    }
  }
  return { w: Math.round(w), h: Math.round(h) }
}

/** `setDockDragImage` 의 재료. */
export interface DragImageRequest {
  /** 끄는 탭의 제목 — 머리 띠에 쓴다. */
  title: string
  /** 그 자리에 놓아 새 창이 될 때의 크기(`newWindowSize`) — 그림 크기의 기준. */
  newWindow: SizePx
  /**
   * 끄는 탭의 캔버스 요소(`WindowLayout` 의 그 탭 `tab-canvas` div) — ★그 탭이 지금 이 창에 보이는 탭일 때만★.
   * `null` = 제목만(보이지 않던 탭의 칸은 `display:none` 이라 그릴 것이 없다 — TRD §11-1 #7). 넘긴 요소가 화면에
   * 펼쳐져 있지 않으면(크기 0) 역시 제목만이다.
   */
  canvas: HTMLElement | null
  /** 칸 자리채움에 쓸 칸 이름(`data-slot-id` 값 → 이름). 없거나 `null` 을 주면 이름 없이 칠만 한다. */
  slotLabel?: (slotId: string) => string | null
}

/** 어느 그림으로 끝났나 — `picture` = 축소 그림 · `title` = 제목만 상자 · `engine` = `setDragImage` 없이 엔진 기본 그림. */
export type DragImageKind = 'picture' | 'title' | 'engine'

export interface DragImageResult {
  kind: DragImageKind
  /** `dock_drag_start` 의 `hotspot` 으로 보낼 커서 자리 — 상자면 `DRAG_IMAGE_HOTSPOT`, 엔진 기본이면 넘겨받은 값. */
  hotspot: PointCss
}

let box: HTMLDivElement | null = null

/**
 * 화면 밖 그림 상자를 붙여 두고 돌려준다(없거나 떨어졌으면 다시 붙인다). `setDockDragImage` 가 부르므로 미리 부를
 * 필요는 없지만, 창 마운트 때 불러 두면 첫 `dragstart` 가 요소를 새로 만들지 않는다.
 */
export function ensureDragImageBox(doc: Document = document): HTMLDivElement {
  if (box !== null && box.ownerDocument === doc) {
    if (!box.isConnected) doc.body.append(box)
    return box
  }
  const el = doc.createElement('div')
  el.setAttribute('aria-hidden', 'true')
  el.dataset.dockDragImage = ''
  // 화면 밖이어도 레이아웃 · 페인트는 된다 — `display:none` · `visibility:hidden` 이면 엔진이 그릴 그림이 없다.
  Object.assign(el.style, {
    position: 'fixed',
    left: '-10000px',
    top: '0px',
    overflow: 'hidden',
    pointerEvents: 'none',
    boxSizing: 'border-box',
    background: 'var(--bg)',
  })
  doc.body.append(el)
  box = el
  return el
}

/**
 * `dragstart` 안에서 부른다 — 상자를 채우고 `dt.setDragImage` 를 부른다. 던지지 않는다.
 * `engineHotspot` = 엔진 기본 그림으로 물러났을 때 셸에 보낼 커서 자리(누른 점의 머리 안 자리 — TRD §4-2).
 */
export function setDockDragImage(
  dt: Pick<DataTransfer, 'setDragImage'>,
  req: DragImageRequest,
  engineHotspot: PointCss,
): DragImageResult {
  const hotspot = { ...DRAG_IMAGE_HOTSPOT }
  if (req.canvas !== null) {
    try {
      const el = ensureDragImageBox()
      fillPicture(el, req.canvas, req)
      dt.setDragImage(el, hotspot.x, hotspot.y)
      return { kind: 'picture', hotspot }
    } catch (err) {
      console.warn('[dragImage] 축소 그림 실패 — 제목만으로 물러난다:', err)
    }
  }
  try {
    const el = ensureDragImageBox()
    fillTitleOnly(el, req.title)
    dt.setDragImage(el, hotspot.x, hotspot.y)
    return { kind: 'title', hotspot }
  } catch (err) {
    console.warn('[dragImage] 제목만 상자 실패 — 엔진 기본 그림으로 물러난다:', err)
    return { kind: 'engine', hotspot: { x: engineHotspot.x, y: engineHotspot.y } }
  }
}

/** 테스트 전용 — 상자를 떼고 잊는다(테스트 간 격리). 프로덕션 코드에서 호출 금지. */
export function __resetDragImageForTest(): void {
  box?.remove()
  box = null
}

function fillTitleOnly(el: HTMLDivElement, title: string): void {
  el.replaceChildren()
  sizeBox(el, TITLE_ONLY)
  el.append(titleBand(el.ownerDocument, title))
}

// 실패하면 던진다 — 부르는 쪽이 제목만으로 물러난다. 상자가 반쯤 채워진 채 남아도 제목만 채우기가 비운다.
function fillPicture(el: HTMLDivElement, canvasEl: HTMLElement, req: DragImageRequest): void {
  const size = dragImageSize(req.newWindow)
  const picture: SizePx = { w: size.w, h: size.h - DRAG_IMAGE_TITLE_H }
  if (picture.h < 1) throw new Error(`그림 칸이 없다(${size.w}x${size.h})`)
  const src = canvasEl.getBoundingClientRect()
  if (!(src.width > 0 && src.height > 0)) throw new Error('캔버스가 펼쳐져 있지 않다')

  const doc = el.ownerDocument
  el.replaceChildren()
  sizeBox(el, size)
  el.append(titleBand(doc, req.title))

  const cv = doc.createElement('canvas')
  const dpr = doc.defaultView?.devicePixelRatio || 1
  cv.width = Math.max(1, Math.round(picture.w * dpr))
  cv.height = Math.max(1, Math.round(picture.h * dpr))
  Object.assign(cv.style, { display: 'block', width: `${picture.w}px`, height: `${picture.h}px` })
  const ctx = cv.getContext('2d')
  if (ctx === null) throw new Error('2d 컨텍스트 없음')
  ctx.scale(dpr, dpr)
  paintMiniature(ctx, picture, canvasEl, src, themeColors(doc), req.slotLabel)
  el.append(cv)
}

interface ThemeColors {
  bg: string
  panel: string
  border: string
  muted: string
  font: string
}

// 캔버스 2d 는 `var()` 를 못 읽어 계산된 값을 꺼낸다. 비면(테마 변수 없는 환경) 다크 테마 값으로 칠한다.
function themeColors(doc: Document): ThemeColors {
  const cs = doc.defaultView?.getComputedStyle(doc.documentElement)
  const read = (name: string, fallback: string): string => cs?.getPropertyValue(name).trim() || fallback
  return {
    bg: read('--bg', '#0a0a0a'),
    panel: read('--bg-secondary', '#141414'),
    border: read('--border', '#242424'),
    muted: read('--text-muted', '#888'),
    font: read('--font-ui', 'sans-serif'),
  }
}

// 탭 캔버스를 비율 그대로(가운데 맞춤) 줄여 칸마다 그린다. 칸은 테마 패널 색 + 칸 이름 자리채움을 먼저 칠하고 그
//   위에 칸 안의 보이는 `<canvas>`(터미널 WebGL)를 옮긴다 — ★자리채움을 먼저 칠하는 순서가 「빈 WebGL = 자리채움」을
//   지킨다★: `preserveDrawingBuffer` 없는 WebGL 캔버스를 그린 프레임 밖에서 읽으면 투명으로 빌 수 있고(가능성 높음 ·
//   S2), 그러면 밑의 자리채움이 비친다. 그린 터미널은 xterm 이 뷰포트 배경을 불투명하게 칠해 자리채움을 덮는다고
//   본다(미검 — 투명 배경 터미널이면 이름 글자가 비쳐 보인다).
function paintMiniature(
  ctx: CanvasRenderingContext2D,
  dest: SizePx,
  canvasEl: HTMLElement,
  src: DOMRect,
  colors: ThemeColors,
  slotLabel: DragImageRequest['slotLabel'],
): void {
  const s = Math.min(dest.w / src.width, dest.h / src.height)
  const ox = (dest.w - src.width * s) / 2
  const oy = (dest.h - src.height * s) / 2
  const map = (r: DOMRect) => ({
    x: ox + (r.left - src.left) * s,
    y: oy + (r.top - src.top) * s,
    w: r.width * s,
    h: r.height * s,
  })

  ctx.fillStyle = colors.bg
  ctx.fillRect(0, 0, dest.w, dest.h)
  for (const slotEl of canvasEl.querySelectorAll<HTMLElement>('[data-slot-id]')) {
    const r = slotEl.getBoundingClientRect()
    if (!(r.width > 0 && r.height > 0)) continue
    const d = map(r)
    ctx.save()
    ctx.beginPath()
    ctx.rect(d.x, d.y, d.w, d.h)
    ctx.clip()
    ctx.fillStyle = colors.panel
    ctx.fillRect(d.x, d.y, d.w, d.h)
    ctx.strokeStyle = colors.border
    ctx.lineWidth = 1
    ctx.strokeRect(d.x + 0.5, d.y + 0.5, d.w - 1, d.h - 1)
    const label = slotLabel?.(slotEl.dataset.slotId ?? '')
    if (label) {
      ctx.fillStyle = colors.muted
      ctx.font = `11px ${colors.font}`
      ctx.textAlign = 'center'
      ctx.textBaseline = 'middle'
      ctx.fillText(label, d.x + d.w / 2, d.y + d.h / 2, Math.max(1, d.w - 8))
    }
    for (const c of slotEl.querySelectorAll('canvas')) {
      const cr = c.getBoundingClientRect()
      // 크기 0 캔버스를 `drawImage` 에 넘기면 던진다 — 그릴 것도 없다.
      if (!(cr.width > 0 && cr.height > 0) || c.width === 0 || c.height === 0) continue
      const cd = map(cr)
      ctx.drawImage(c, cd.x, cd.y, cd.w, cd.h)
    }
    ctx.restore()
  }
}

function sizeBox(el: HTMLDivElement, size: SizePx): void {
  el.style.width = `${size.w}px`
  el.style.height = `${size.h}px`
}

function titleBand(doc: Document, title: string): HTMLDivElement {
  const band = doc.createElement('div')
  band.textContent = title
  Object.assign(band.style, {
    boxSizing: 'border-box',
    height: `${DRAG_IMAGE_TITLE_H}px`,
    lineHeight: `${DRAG_IMAGE_TITLE_H}px`,
    padding: '0 10px',
    background: 'var(--bg-secondary)',
    color: 'var(--text)',
    fontFamily: 'var(--font-ui)',
    fontSize: '12px',
    whiteSpace: 'nowrap',
    overflow: 'hidden',
    textOverflow: 'ellipsis',
  })
  return band
}
