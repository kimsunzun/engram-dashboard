// dragImage 단위테스트 — 크기 규칙(TRD §4-2 · §9 프론트 행의 예)과 물러나기(축소 그림 → 제목만 → 엔진 기본).
// jsdom 은 2d 캔버스 · 레이아웃이 없어 `getContext` · `getBoundingClientRect` 를 갈아 끼운다. 실제 그림 모양 · WebGL 이
//   비어 읽히는지는 여기서 못 잰다(S2 · `/qa full` 몫).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import {
  __resetDragImageForTest,
  dragImageSize,
  DRAG_IMAGE_HOTSPOT,
  ensureDragImageBox,
  newWindowSize,
  setDockDragImage,
  type DragImageRequest,
} from './dragImage'

function rect(x: number, y: number, w: number, h: number): DOMRect {
  return { x, y, left: x, top: y, right: x + w, bottom: y + h, width: w, height: h, toJSON: () => ({}) } as DOMRect
}

function withRect<T extends Element>(el: T, r: DOMRect): T {
  el.getBoundingClientRect = () => r
  return el
}

function fakeCtx() {
  return {
    scale: vi.fn(),
    fillRect: vi.fn(),
    strokeRect: vi.fn(),
    save: vi.fn(),
    restore: vi.fn(),
    beginPath: vi.fn(),
    rect: vi.fn(),
    clip: vi.fn(),
    fillText: vi.fn(),
    drawImage: vi.fn(),
    fillStyle: '',
    strokeStyle: '',
    lineWidth: 1,
    font: '',
    textAlign: '',
    textBaseline: '',
  }
}

const ENGINE_HOTSPOT = { x: 7, y: 9 }

let ctx: ReturnType<typeof fakeCtx>
let getContext: ReturnType<typeof vi.spyOn>

beforeEach(() => {
  ctx = fakeCtx()
  getContext = vi
    .spyOn(HTMLCanvasElement.prototype, 'getContext')
    .mockImplementation(() => ctx as unknown as CanvasRenderingContext2D)
  vi.spyOn(console, 'warn').mockImplementation(() => {})
})
afterEach(() => {
  __resetDragImageForTest()
  document.body.replaceChildren()
  vi.restoreAllMocks()
})

// 탭 캔버스(뷰포트 (0, 28) · 720×472) 하나에 칸 둘 — 왼쪽 칸에만 터미널 캔버스가 있다.
function tabCanvas() {
  const root = withRect(document.createElement('div'), rect(0, 28, 720, 472))
  const left = withRect(document.createElement('div'), rect(0, 28, 360, 472))
  left.dataset.slotId = 'slot-a'
  const term = withRect(document.createElement('canvas'), rect(1, 29, 358, 470))
  left.append(term)
  const right = withRect(document.createElement('div'), rect(360, 28, 360, 472))
  right.dataset.slotId = 'slot-b'
  root.append(left, right)
  document.body.append(root)
  return { root, term }
}

function request(over: Partial<DragImageRequest> = {}): DragImageRequest {
  return { title: '탭 하나', newWindow: { w: 720, h: 500 }, canvas: null, ...over }
}

describe('dragImageSize', () => {
  it.each([
    [{ w: 720, h: 500 }, { w: 288, h: 200 }],
    [{ w: 1600, h: 1000 }, { w: 320, h: 200 }],
    [{ w: 400, h: 300 }, { w: 180, h: 135 }],
  ])('새 창 %o → %o', (win, want) => {
    expect(dragImageSize(win)).toEqual(want)
  })

  it('제목만 = 180×28 — 크기가 없거나 양수가 아니어도', () => {
    expect(dragImageSize(null)).toEqual({ w: 180, h: 28 })
    expect(dragImageSize({ w: 0, h: 500 })).toEqual({ w: 180, h: 28 })
    expect(dragImageSize({ w: Number.NaN, h: 500 })).toEqual({ w: 180, h: 28 })
  })

  it('너비 하한까지 키우는 것은 상한 안에서만이다 — 좁고 긴 창은 높이 상한에서 멈춘다', () => {
    expect(dragImageSize({ w: 200, h: 1000 })).toEqual({ w: 48, h: 240 })
  })

  it('메인에서 떼면 팝아웃 기본 크기가 기준이다 — 팝아웃은 그 창 안쪽 크기', () => {
    expect(newWindowSize(true, { w: 1920, h: 1080 })).toEqual({ w: 720, h: 500 })
    expect(newWindowSize(false, { w: 1600, h: 1000 })).toEqual({ w: 1600, h: 1000 })
  })
})

describe('setDockDragImage', () => {
  it('보이지 않던 탭(캔버스 없음) = 제목만 상자 · 커서 자리 (16, 14)', () => {
    const dt = { setDragImage: vi.fn() }
    const res = setDockDragImage(dt, request(), ENGINE_HOTSPOT)

    expect(res).toEqual({ kind: 'title', hotspot: { x: 16, y: 14 } })
    const box = ensureDragImageBox()
    expect(dt.setDragImage).toHaveBeenCalledWith(box, DRAG_IMAGE_HOTSPOT.x, DRAG_IMAGE_HOTSPOT.y)
    expect(box.style.width).toBe('180px')
    expect(box.style.height).toBe('28px')
    expect(box.textContent).toBe('탭 하나')
    expect(box.querySelector('canvas')).toBeNull()
  })

  it('보이는 탭 = 축소 그림 — 칸마다 자리채움을 칠한 뒤 터미널 캔버스를 비율 그대로 옮긴다', () => {
    const { root, term } = tabCanvas()
    const dt = { setDragImage: vi.fn() }
    const labels = vi.fn((id: string) => (id === 'slot-b' ? '챗' : null))

    const res = setDockDragImage(dt, request({ canvas: root, slotLabel: labels }), ENGINE_HOTSPOT)

    expect(res).toEqual({ kind: 'picture', hotspot: { x: 16, y: 14 } })
    const box = ensureDragImageBox()
    expect(dt.setDragImage).toHaveBeenCalledWith(box, 16, 14)
    expect(box.style.width).toBe('288px')
    expect(box.style.height).toBe('200px')
    const cv = box.querySelector('canvas')
    expect(cv?.style.height).toBe('172px')
    expect(labels.mock.calls.map(c => c[0])).toEqual(['slot-a', 'slot-b'])
    expect(ctx.fillText).toHaveBeenCalledTimes(1)
    expect(ctx.fillText.mock.calls[0][0]).toBe('챗')

    // 288×172 안에 720×472 를 비율 그대로 → s = 172/472 · 가로 가운데 맞춤.
    const s = 172 / 472
    const ox = (288 - 720 * s) / 2
    expect(ctx.drawImage).toHaveBeenCalledTimes(1)
    const [img, x, y, w, h] = ctx.drawImage.mock.calls[0]
    expect(img).toBe(term)
    expect(x).toBeCloseTo(ox + 1 * s)
    expect(y).toBeCloseTo(1 * s)
    expect(w).toBeCloseTo(358 * s)
    expect(h).toBeCloseTo(470 * s)
    // 빈 WebGL 이 자리채움을 비치게 하는 순서 — 그 칸의 자리채움 칠이 캔버스 옮기기보다 먼저다.
    const slotFill = ctx.fillRect.mock.invocationCallOrder[1]
    expect(slotFill).toBeLessThan(ctx.drawImage.mock.invocationCallOrder[0])
  })

  it('크기 0 캔버스(펼쳐지지 않은 xterm)는 건너뛴다 — 던지지 않고 자리채움만 남는다', () => {
    const { root, term } = tabCanvas()
    withRect(term, rect(0, 0, 0, 0))
    const res = setDockDragImage({ setDragImage: vi.fn() }, request({ canvas: root }), ENGINE_HOTSPOT)
    expect(res.kind).toBe('picture')
    expect(ctx.drawImage).not.toHaveBeenCalled()
  })

  it.each([
    ['2d 컨텍스트가 없다', () => getContext.mockImplementation(() => null)],
    [
      '캔버스 옮기기가 던진다',
      () =>
        ctx.drawImage.mockImplementation(() => {
          throw new Error('boom')
        }),
    ],
  ])('축소 그림 실패(%s) = 제목만 상자로 물러난다', (_why, breakIt) => {
    const { root } = tabCanvas()
    breakIt()
    const dt = { setDragImage: vi.fn() }

    const res = setDockDragImage(dt, request({ canvas: root }), ENGINE_HOTSPOT)

    expect(res).toEqual({ kind: 'title', hotspot: { x: 16, y: 14 } })
    const box = ensureDragImageBox()
    expect(dt.setDragImage).toHaveBeenCalledTimes(1)
    expect(box.style.width).toBe('180px')
    expect(box.style.height).toBe('28px')
    expect(box.querySelector('canvas')).toBeNull()
  })

  it('넘긴 탭 캔버스가 펼쳐져 있지 않으면(크기 0) 제목만이다', () => {
    const { root } = tabCanvas()
    withRect(root, rect(0, 0, 0, 0))
    const res = setDockDragImage({ setDragImage: vi.fn() }, request({ canvas: root }), ENGINE_HOTSPOT)
    expect(res.kind).toBe('title')
  })

  it('축소 그림의 setDragImage 가 던지면 제목만으로 다시 부른다', () => {
    const { root } = tabCanvas()
    const dt = {
      setDragImage: vi.fn().mockImplementationOnce(() => {
        throw new Error('no')
      }),
    }
    expect(setDockDragImage(dt, request({ canvas: root }), ENGINE_HOTSPOT).kind).toBe('title')
    expect(dt.setDragImage).toHaveBeenCalledTimes(2)
  })

  it('제목만 상자까지 실패하면 엔진 기본 그림 — 셸에 보낼 커서 자리는 넘겨받은 값이다', () => {
    const { root } = tabCanvas()
    const dt = {
      setDragImage: vi.fn(() => {
        throw new Error('no')
      }),
    }
    expect(setDockDragImage(dt, request({ canvas: root }), ENGINE_HOTSPOT)).toEqual({
      kind: 'engine',
      hotspot: ENGINE_HOTSPOT,
    })
  })
})

describe('ensureDragImageBox', () => {
  it('화면 밖에 하나만 붙이고, 떨어졌으면 다시 붙인다 — 숨기지 않는다(엔진이 그릴 수 있게)', () => {
    const box = ensureDragImageBox()
    expect(ensureDragImageBox()).toBe(box)
    expect(document.querySelectorAll('[data-dock-drag-image]')).toHaveLength(1)
    expect(box.style.position).toBe('fixed')
    expect(box.style.display).not.toBe('none')
    expect(box.style.visibility).not.toBe('hidden')

    box.remove()
    expect(ensureDragImageBox()).toBe(box)
    expect(box.isConnected).toBe(true)
  })
})
