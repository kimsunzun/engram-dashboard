// useTabDrag 단위테스트 — 컨트롤러(`TabDragController`)를 가짜 invoke · 가짜 재기 · jsdom 포인터 · 키 사건으로 몬다:
//   10 px 문턱(그 아래는 click · dblclick 그대로) · 뗌을 놓친 누름(단추 놓인 움직임 · 새 누름 · 창 blur) · 누른 머리 사각형의
//   잡은 비율 · begin 인자와 답 넷 · 답 기다리는 동안 붙든 끝 · 순서 이동 seq 와 끝의 `at` · 모델 메아리 맞추기 · 구역 나감 →
//   leave 답 셋(not_ready 면 구역 밖도 가로로 몬다) · 답 전 모델에서 빠진 끄는 탭 · 캡처 잃음 · 키(탭을 쥔 창이 받고 끊는다) ·
//   셸 입력(attach · move · release) · `dock:drag{ended}` 거두기(뗀 끌기 · 창째 끌기의 남은 캡처 놓기 포함) · 누름부터 끝까지의
//   `engaged`. 끝에 훅 배선(재기 · `dock:feed` · `dock:drag` · 언마운트)을 실물 `dockSession` 위에서 잰다.
// 탭바(기대값의 손셈): 줄 x 100..800 · y 40..68 · `+` 40 → 쓸 수 있는 폭 660 · 탭 넷이면 모두 표준 폭 160 → 끌기 영역 좌표의
//   경계 0 · 160 · 320 · 480 · 640 · 순서 문턱 16 · 끄는 탭 왼쪽 상한 660 − 160 = 500. 탭 a(머리 x 100..260)를 x 180(머리 안
//   80 = 비율 0.5 → 잡은 오프셋 80)에서 누른다. 움직임은 주 단추를 쥔 채(`buttons` 1)다.

import { act, cleanup, renderHook } from '@testing-library/react'
import type { PointerEvent as ReactPointerEvent } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invokeMock = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}))
const listeners = new Map<string, (e: { payload: unknown }) => void>()
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, handler: (e: { payload: unknown }) => void) => {
    listeners.set(event, handler)
    return () => listeners.delete(event)
  }),
}))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ label: 'main' }) }))

import type { BeginReply } from '../../../../src-tauri/bindings/BeginReply'
import type { LeaveReply } from '../../../../src-tauri/bindings/LeaveReply'
import type { RectCss } from '../../../../src-tauri/bindings/RectCss'
import type { TabDragFeed } from '../../../../src-tauri/bindings/TabDragFeed'
import { CMD_TAB_DRAG_END, EVT_DRAG, EVT_FEED, watchDockSession } from '../dock/dockSession'
import type { StripMeasure } from './measure'
import {
  CMD_TAB_DRAG_BEGIN,
  CMD_TAB_DRAG_LEAVE,
  CMD_TAB_DRAG_REORDER,
  TabDragController,
  useTabDrag,
  type TabDragView,
} from './useTabDrag'

const ticks = async (): Promise<void> => {
  for (let i = 0; i < 20; i++) await Promise.resolve()
}

interface Pending<T> {
  resolve: (v: T) => void
  reject: (e: unknown) => void
}
let begins: Pending<BeginReply>[]
let leaves: Pending<LeaveReply>[]

const calls = (cmd: string) => invokeMock.mock.calls.filter(c => c[0] === cmd).map(c => (c[1] as { args: unknown }).args)
const reorders = () => calls(CMD_TAB_DRAG_REORDER)
const ends = () => calls(CMD_TAB_DRAG_END)

const ZONE: RectCss = { x0: 100, y0: 25, x1: 760, y1: 83 }
const MEASURE: StripMeasure = { row: { x0: 100, y0: 40, x1: 800, y1: 68 }, left: 100, available: 660, canvas: null }

let row: HTMLDivElement
let head: HTMLDivElement
let captured: Set<number>
let views: (TabDragView | null)[]
let engaged: boolean[]
let ctl: TabDragController

const lastView = () => views[views.length - 1] ?? null

function pointer(type: string, x: number, y: number, pointerId = 1, buttons = type === 'pointermove' ? 1 : 0): PointerEvent {
  const e = new PointerEvent(type, { clientX: x, clientY: y, pointerId, buttons, bubbles: true, cancelable: true })
  row.dispatchEvent(e)
  return e
}
function box(el: Element, left: number, top: number, width: number, height: number): void {
  el.getBoundingClientRect = () =>
    ({ left, top, width, height, right: left + width, bottom: top + height, x: left, y: top }) as DOMRect
}
const move = (x: number, y: number) => pointer('pointermove', x, y)
const up = (x: number, y: number) => pointer('pointerup', x, y)
function lost(pointerId = 1): void {
  row.dispatchEvent(new PointerEvent('lostpointercapture', { pointerId, bubbles: true }))
}
function key(k: string): KeyboardEvent {
  const e = new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true })
  document.body.dispatchEvent(e)
  return e
}

function press(view = 'a', x = 180, y = 50, extra: { button?: number; target?: EventTarget } = {}): void {
  ctl.press({ view, pointerId: 1, button: extra.button ?? 0, x, y, target: extra.target ?? head, head })
}

/** a 를 누르고 문턱을 넘긴 뒤 begin 답 `strip{7}` 까지. */
async function strip(): Promise<void> {
  press()
  move(191, 50)
  begins[begins.length - 1].resolve({ type: 'strip', session: 7, zone: ZONE })
  await ticks()
}

const feed = (step: TabDragFeed['step'], session = 9, view = 'n'): void => ctl.feed({ session, view, step })
const attachStep = (x = 420): TabDragFeed['step'] => ({
  type: 'attach',
  zone: ZONE,
  grab: { x_ratio: 0.25, y: 10, single_x: 40 },
  x,
  y: 50,
})

beforeEach(() => {
  begins = []
  leaves = []
  invokeMock.mockReset()
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === CMD_TAB_DRAG_BEGIN) return new Promise((resolve, reject) => begins.push({ resolve, reject }))
    if (cmd === CMD_TAB_DRAG_LEAVE) return new Promise((resolve, reject) => leaves.push({ resolve, reject }))
    return Promise.resolve(undefined)
  })
  row = document.createElement('div')
  head = document.createElement('div')
  row.append(head)
  document.body.append(row)
  box(head, 100, 40, 160, 28)
  captured = new Set()
  row.setPointerCapture = vi.fn((id: number) => void captured.add(id))
  row.releasePointerCapture = vi.fn((id: number) => void captured.delete(id))
  row.hasPointerCapture = (id: number) => captured.has(id)
  views = []
  engaged = []
  ctl = new TabDragController({
    row: () => row,
    measure: () => MEASURE,
    reducedMotion: () => false,
    onView: v => views.push(v),
    onEngaged: e => engaged.push(e),
  })
  ctl.setModel(['a', 'b', 'c', 'd'], 'b', null)
})
afterEach(() => {
  ctl.dispose()
  row.remove()
  vi.restoreAllMocks()
})

// ── 문턱 ──

describe('누름 · 10 px 문턱', () => {
  it('10 px 이하로 움직이고 떼면 아무것도 보내지 않고 사건을 막지 않는다(click · dblclick 그대로)', async () => {
    press()
    expect(move(189, 50).defaultPrevented).toBe(false)
    move(180, 60) // 정확히 10 = 아님
    expect(up(180, 60).defaultPrevented).toBe(false)
    move(300, 50) // 뗀 뒤
    await ticks()
    expect(invokeMock).not.toHaveBeenCalled()
    expect(row.setPointerCapture).not.toHaveBeenCalled()
    expect(views).toEqual([])
  })

  it('넘는 순간 탭바 줄에 캡처 · 포커스를 놓고 begin 을 한 번 — 누른 점 · 끌기 영역 · 잡은 자리 · 누르기 전 활성', async () => {
    const input = document.createElement('input')
    document.body.append(input)
    input.focus()
    press()
    move(191, 50)
    move(195, 50)
    expect(row.setPointerCapture).toHaveBeenCalledWith(1)
    expect(document.activeElement).not.toBe(input)
    expect(calls(CMD_TAB_DRAG_BEGIN)).toEqual([
      {
        view: 'a',
        down: { x: 180, y: 50 },
        area: { x0: 100, y0: 40, x1: 760, y1: 68 },
        grab: { x_ratio: 0.5, y: 10, single_x: 80 },
        initial_active: 'b',
      },
    ])
    input.remove()
  })

  it('왼쪽 단추가 아니거나 머리 안 닫기 단추 · 이름 입력에서 시작한 누름은 끌기가 아니다', () => {
    const close = document.createElement('button')
    const rename = document.createElement('input')
    head.append(close, rename)
    press('a', 180, 50, { button: 2 })
    move(300, 50)
    press('a', 180, 50, { target: close })
    move(300, 50)
    press('a', 180, 50, { target: rename })
    move(300, 50)
    expect(invokeMock).not.toHaveBeenCalled()
  })

  it('잡은 비율은 누른 순간 머리의 그려진 사각형에서 잰다(폭 0 이면 0) — 잡은 오프셋도 그 비율이다', async () => {
    box(head, 100, 40, 200, 28)
    press()
    box(head, 0, 0, 0, 0) // 누른 뒤 바뀐 사각형은 보지 않는다
    move(191, 50)
    expect(calls(CMD_TAB_DRAG_BEGIN)[0]).toMatchObject({ grab: { x_ratio: 0.4, single_x: 64 } })
    begins[0].resolve({ type: 'strip', session: 7, zone: ZONE })
    await ticks()
    move(300, 50) // 200 − 오프셋 64
    expect(lastView()?.left).toBe(136)

    ctl.dragEvent({ session: 7, phase: 'ended' })
    press()
    move(191, 50)
    expect(calls(CMD_TAB_DRAG_BEGIN)[1]).toMatchObject({ grab: { x_ratio: 0, single_x: 0 } })
  })
})

describe('뗌을 놓친 누름', () => {
  it('주 단추가 놓인 움직임은 누름을 버린다 — 그 뒤 문턱을 넘어도 begin 없음 · 다른 포인터의 움직임은 보지 않는다', async () => {
    press()
    move(185, 50)
    pointer('pointermove', 300, 50, 1, 0)
    move(310, 50)
    up(310, 50)
    await ticks()
    expect(invokeMock).not.toHaveBeenCalled()
    expect(row.setPointerCapture).not.toHaveBeenCalled()

    press()
    pointer('pointermove', 185, 50, 2, 0)
    move(191, 50)
    expect(calls(CMD_TAB_DRAG_BEGIN)).toHaveLength(1)
  })

  it('누름 중 머리의 새 누름은 새 점에서 다시 누른다', () => {
    press('a', 180, 50)
    move(185, 50)
    press('b', 400, 50)
    move(405, 50) // 앞 점에서는 문턱 밖 · 새 점에서는 5 px
    expect(invokeMock).not.toHaveBeenCalled()
    move(411, 50)
    expect(calls(CMD_TAB_DRAG_BEGIN)).toEqual([expect.objectContaining({ view: 'b', down: { x: 400, y: 50 } })])
  })

  it('누름 중 어디서든 pointerdown 이 오면 누름을 버린다', () => {
    press()
    move(185, 50)
    document.body.dispatchEvent(new PointerEvent('pointerdown', { pointerId: 1, buttons: 1, bubbles: true }))
    move(300, 50)
    expect(invokeMock).not.toHaveBeenCalled()
  })

  it('창 blur 는 누름을 버린다 · 요소의 blur(문턱 때 포커스 놓기 포함)는 버리지 않는다', async () => {
    const input = document.createElement('input')
    document.body.append(input)
    press()
    move(185, 50)
    window.dispatchEvent(new Event('blur'))
    move(300, 50)
    expect(invokeMock).not.toHaveBeenCalled()

    input.focus()
    press()
    input.blur()
    input.focus()
    move(191, 50)
    up(191, 50)
    begins[0].resolve({ type: 'strip', session: 7, zone: ZONE })
    await ticks()
    expect(ends()).toEqual([{ session: 7, how: 'drop', src: 'pointer', at: 0 }])
    input.remove()
  })
})

// ── begin 답 ──

describe('begin 답', () => {
  it('strip — 탭바 안을 몬다(끄는 탭 x · 순서 이동)', async () => {
    await strip()
    move(300, 50)
    expect(reorders()).toEqual([{ session: 7, seq: 1, index: 1 }])
    expect(lastView()).toEqual({
      view: 'a',
      order: ['b', 'a', 'c', 'd'],
      targets: [0, null, 320, 480],
      left: 120,
      slideMs: 200,
      settling: false,
    })
  })

  it('whole_window — 그리지 않고 포인터 사건 · 키를 버린다(키는 앱 전체 키 감시 몫)', async () => {
    press()
    move(191, 50)
    begins[0].resolve({ type: 'whole_window', session: 3 })
    await ticks()
    move(300, 50)
    up(300, 50)
    expect(key('Escape').defaultPrevented).toBe(false)
    await ticks()
    expect(lastView()).toBeNull()
    expect(invokeMock.mock.calls.map(c => c[0])).toEqual([CMD_TAB_DRAG_BEGIN])
    expect(row.releasePointerCapture).not.toHaveBeenCalled()
  })

  it.each<[string, BeginReply | Error]>([
    ['busy', { type: 'busy' }],
    ['rejected', { type: 'rejected', reason: 'no' }],
    ['오류', new Error('gone')],
  ])('%s — 끌기를 버리고 캡처를 놓는다', async (_name, reply) => {
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    press()
    move(191, 50)
    if (reply instanceof Error) begins[0].reject(reply)
    else begins[0].resolve(reply)
    await ticks()
    expect(row.releasePointerCapture).toHaveBeenCalledWith(1)
    move(300, 50)
    expect(key('a').defaultPrevented).toBe(false)
    await ticks()
    expect(invokeMock.mock.calls.map(c => c[0])).toEqual([CMD_TAB_DRAG_BEGIN])
    expect(views.every(v => v === null)).toBe(true)
  })
})

describe('begin 답을 기다리는 동안', () => {
  it('뗌을 붙들었다 strip 답 뒤 drop · pointer — 뒤따른 캡처 잃음은 버린다', async () => {
    press()
    move(191, 50)
    up(191, 50)
    lost()
    expect(ends()).toEqual([])
    begins[0].resolve({ type: 'strip', session: 7, zone: ZONE })
    await ticks()
    expect(ends()).toEqual([{ session: 7, how: 'drop', src: 'pointer', at: 0 }])
    expect(lastView()?.settling).toBe(true)
  })

  it('처음 키 하나만 붙든다 — 키는 그 자리에서 끊는다', async () => {
    press()
    move(191, 50)
    const a = key('a')
    const esc = key('Escape')
    expect(a.defaultPrevented && esc.defaultPrevented).toBe(true)
    begins[0].resolve({ type: 'strip', session: 7, zone: ZONE })
    await ticks()
    expect(ends()).toEqual([{ session: 7, how: 'drop', src: 'key', at: 0 }])
  })

  it('whole_window 답이면 붙든 키 끝만 보낸다(at 없음) · 붙든 뗌은 버린다', async () => {
    press()
    move(191, 50)
    key('Escape')
    begins[0].resolve({ type: 'whole_window', session: 3 })
    await ticks()
    expect(ends()).toEqual([{ session: 3, how: 'cancel', src: 'key', at: null }])

    invokeMock.mockClear()
    ctl.dragEvent({ session: 3, phase: 'ended' })
    press()
    move(191, 50)
    up(191, 50)
    begins[1].resolve({ type: 'whole_window', session: 4 })
    await ticks()
    expect(ends()).toEqual([])
  })

  it('그 사이 움직임은 마지막 점만 — 답 뒤 그 점으로 몬다', async () => {
    press()
    move(191, 50)
    move(250, 50)
    move(300, 50)
    begins[0].resolve({ type: 'strip', session: 7, zone: ZONE })
    await ticks()
    expect(reorders()).toEqual([{ session: 7, seq: 1, index: 1 }])
  })

  it('마지막 점이 구역 밖이면 답 뒤 곧바로 leave', async () => {
    press()
    move(191, 50)
    move(300, 120)
    begins[0].resolve({ type: 'strip', session: 7, zone: ZONE })
    await ticks()
    expect(calls(CMD_TAB_DRAG_LEAVE)).toEqual([{ session: 7 }])
  })
})

// ── 순서 이동 ──

describe('탭바 안 순서 이동', () => {
  it('seq 는 세션 안 1 부터 단조 · 놓을 때 지금 자리를 at 으로 · 끄는 탭은 자기 자리로 간다', async () => {
    await strip()
    move(300, 50)
    move(460, 50)
    move(465, 50) // 문턱 안 · 같은 자리
    expect(reorders()).toEqual([
      { session: 7, seq: 1, index: 1 },
      { session: 7, seq: 2, index: 2 },
    ])
    expect(lastView()).toMatchObject({ order: ['b', 'c', 'a', 'd'], targets: [0, 160, null, 480], left: 285 })
    up(465, 50)
    expect(ends()).toEqual([{ session: 7, how: 'drop', src: 'pointer', at: 2 }])
    expect(lastView()).toMatchObject({ left: 320, settling: true, slideMs: 200 })
    move(100, 50)
    await ticks()
    expect(reorders()).toHaveLength(2)
  })

  it('줄인 움직임이면 비켜남 0', async () => {
    ctl = new TabDragController({
      row: () => row,
      measure: () => MEASURE,
      reducedMotion: () => true,
      onView: v => views.push(v),
      onEngaged: () => {},
    })
    ctl.setModel(['a', 'b', 'c', 'd'], 'b', null)
    await strip()
    move(300, 50)
    expect(lastView()?.slideMs).toBe(0)
  })

  it('메아리 — 보낸 자리의 메아리는 지우고(앞선 메아리로 되돌아가지 않는다) · 보낸 것이 없을 때 다른 자리는 따른다', async () => {
    await strip()
    move(300, 50)
    move(460, 50)
    ctl.setModel(['b', 'a', 'c', 'd'], 'a', null)
    expect(lastView()?.order).toEqual(['b', 'c', 'a', 'd'])
    ctl.setModel(['b', 'c', 'a', 'd'], 'a', null)
    expect(lastView()?.order).toEqual(['b', 'c', 'a', 'd'])
    ctl.setModel(['a', 'b', 'c', 'd'], 'a', null)
    expect(lastView()?.order).toEqual(['a', 'b', 'c', 'd'])
    const n = views.length
    ctl.setModel(['a', 'b', 'c', 'd'], 'a', null)
    expect(views).toHaveLength(n)
  })
})

// ── 떠나기 ──

describe('구역 나감 → leave', () => {
  it('detached — 그리지 않고 그 뒤 포인터 사건 · 캡처 잃음 · 키를 버린다(캡처는 놓지 않는다)', async () => {
    await strip()
    move(300, 100)
    move(400, 100)
    expect(calls(CMD_TAB_DRAG_LEAVE)).toEqual([{ session: 7 }])
    expect(reorders()).toEqual([])
    leaves[0].resolve('detached')
    await ticks()
    expect(lastView()).toBeNull()
    move(300, 50)
    up(300, 50)
    lost()
    expect(key('a').defaultPrevented).toBe(false)
    await ticks()
    expect(reorders()).toEqual([])
    expect(ends()).toEqual([])
    expect(calls(CMD_TAB_DRAG_LEAVE)).toHaveLength(1)
    expect(row.releasePointerCapture).not.toHaveBeenCalled()
  })

  it('not_ready — 구역 밖 점도 가로로 몰고(끌기 영역 끝에 잘려 선다) · 답으로는 다시 청하지 않고 구역 밖 다음 움직임에서 청한다', async () => {
    await strip()
    move(600, 100)
    expect(reorders()).toEqual([])
    leaves[0].resolve('not_ready')
    await ticks()
    expect(reorders()).toEqual([{ session: 7, seq: 1, index: 3 }])
    expect(lastView()).toMatchObject({ order: ['b', 'c', 'd', 'a'], left: 420, settling: false })
    expect(calls(CMD_TAB_DRAG_LEAVE)).toHaveLength(1)

    move(2000, 100)
    expect(calls(CMD_TAB_DRAG_LEAVE)).toHaveLength(2)
    leaves[1].resolve('not_ready')
    await ticks()
    expect(lastView()).toMatchObject({ order: ['b', 'c', 'd', 'a'], left: 500 })
    expect(calls(CMD_TAB_DRAG_LEAVE)).toHaveLength(2)

    move(300, 50)
    expect(reorders()).toEqual([
      { session: 7, seq: 1, index: 3 },
      { session: 7, seq: 2, index: 1 },
    ])
    expect(calls(CMD_TAB_DRAG_LEAVE)).toHaveLength(2)
  })

  it('not_ready — 기다리는 동안 구역 안으로 돌아온 점은 답 뒤 몬다', async () => {
    await strip()
    move(300, 100)
    move(300, 50)
    leaves[0].resolve('not_ready')
    await ticks()
    expect(reorders()).toEqual([{ session: 7, seq: 1, index: 1 }])
  })

  it('ended · 오류 — 끌기를 버리고 캡처를 놓는다', async () => {
    await strip()
    move(300, 100)
    leaves[0].resolve('ended')
    await ticks()
    expect(lastView()).toBeNull()
    expect(row.releasePointerCapture).toHaveBeenCalledWith(1)
    expect(key('a').defaultPrevented).toBe(false)

    vi.spyOn(console, 'warn').mockImplementation(() => {})
    await strip()
    move(300, 100)
    leaves[1].reject(new Error('gone'))
    await ticks()
    expect(lastView()).toBeNull()
    expect(ends()).toEqual([])
  })

  it('기다리는 동안의 뗌 — not_ready 면 그 점까지 몬 자리로 drop · pointer, detached 면 버린다', async () => {
    await strip()
    move(300, 100)
    up(300, 100)
    leaves[0].resolve('not_ready')
    await ticks()
    expect(reorders()).toEqual([{ session: 7, seq: 1, index: 1 }])
    expect(ends()).toEqual([{ session: 7, how: 'drop', src: 'pointer', at: 1 }])

    ctl.dragEvent({ session: 7, phase: 'ended' })
    invokeMock.mockClear()
    await strip()
    move(300, 100)
    up(300, 100)
    leaves[1].resolve('detached')
    await ticks()
    expect(ends()).toEqual([])
  })

  it('기다리는 동안의 키 — detached 면 키 끝만 at 없이', async () => {
    await strip()
    move(300, 100)
    key('Enter')
    leaves[0].resolve('detached')
    await ticks()
    expect(ends()).toEqual([{ session: 7, how: 'drop', src: 'key', at: null }])
  })

  it('기다리는 동안 모델에서 끄는 탭이 빠지면(셸이 떼었다) 답 전에 그리지 않고 · not_ready 답이면 다시 몬다', async () => {
    await strip()
    move(300, 50)
    move(300, 100)
    expect(lastView()).not.toBeNull()
    ctl.setModel(['b', 'c', 'd'], 'b', null)
    expect(lastView()).toBeNull()
    move(320, 110)
    expect(lastView()).toBeNull()

    leaves[0].resolve('not_ready')
    await ticks()
    expect(lastView()).toMatchObject({ view: 'a', order: ['b', 'a', 'c', 'd'], settling: false })
  })

  it('기다리는 동안 같은 탭바에 다시 붙으면(메아리 · feed attach) 붙은 몰기를 그리고 늦게 온 detached 답은 버린다', async () => {
    await strip()
    move(300, 100)
    ctl.setModel(['b', 'c', 'd'], 'b', null)
    expect(lastView()).toBeNull()
    ctl.setModel(['b', 'a', 'c', 'd'], 'a', null)
    feed(attachStep(), 7, 'a')
    const attached = lastView()
    expect(attached).toMatchObject({ view: 'a', settling: false })

    leaves[0].resolve('detached')
    await ticks()
    expect(lastView()).toBe(attached)
  })
})

describe('뗀 끌기 · 창째 끌기 뒤 남은 캡처', () => {
  it('detached — 놓지 않고 있다가 그 세션의 ended 에서 탭바 줄의 캡처를 놓는다', async () => {
    await strip()
    move(300, 100)
    leaves[0].resolve('detached')
    await ticks()
    expect(captured.has(1)).toBe(true)
    ctl.dragEvent({ session: 7, phase: 'ended' })
    expect(row.releasePointerCapture).toHaveBeenCalledWith(1)
    expect(captured.has(1)).toBe(false)
  })

  it('whole_window — 그 세션의 ended 에서 놓는다', async () => {
    press()
    move(191, 50)
    begins[0].resolve({ type: 'whole_window', session: 3 })
    await ticks()
    expect(captured.has(1)).toBe(true)
    ctl.dragEvent({ session: 3, phase: 'ended' })
    expect(captured.has(1)).toBe(false)
  })

  it('떠나기 답 전에 같은 탭바에 다시 붙은 끌기도 ended 에서 붙기 전의 캡처를 놓는다', async () => {
    await strip()
    move(300, 100)
    feed(attachStep(), 7, 'a')
    ctl.dragEvent({ session: 7, phase: 'ended' })
    expect(captured.has(1)).toBe(false)
  })

  it('ended 전에 새 누름이 왔으면 옛 세션의 ended 는 놓지 않는다 — 그 누름이 문턱에서 건 캡처가 그대로다', async () => {
    await strip()
    move(300, 100)
    leaves[0].resolve('detached')
    await ticks()
    press('b', 400, 50)
    move(411, 50)
    ctl.dragEvent({ session: 7, phase: 'ended' })
    expect(row.releasePointerCapture).not.toHaveBeenCalled()
    expect(captured.has(1)).toBe(true)
    expect(calls(CMD_TAB_DRAG_BEGIN)).toHaveLength(2)
  })
})

// ── 끝 ──

describe('끝 — 캡처 잃음 · 키', () => {
  it('떠나기 전 캡처를 잃으면 cancel · capture_lost', async () => {
    await strip()
    move(300, 50)
    lost()
    expect(ends()).toEqual([{ session: 7, how: 'cancel', src: 'capture_lost', at: 1 }])
  })

  it('Escape = cancel · key / 그 밖 = drop · key — 지금 자리를 싣고 키를 window capture 에서 끊는다', async () => {
    const watcher = vi.fn()
    document.addEventListener('keydown', watcher, true)
    try {
      await strip()
      move(300, 50)
      const esc = key('Escape')
      expect(esc.defaultPrevented).toBe(true)
      expect(watcher).not.toHaveBeenCalled()
      expect(ends()).toEqual([{ session: 7, how: 'cancel', src: 'key', at: 1 }])
      expect(row.releasePointerCapture).toHaveBeenCalledWith(1)
      // 끝을 보낸 뒤의 키는 감시 몫이다.
      key('b')
      expect(watcher).toHaveBeenCalledTimes(1)

      ctl.dragEvent({ session: 7, phase: 'ended' })
      invokeMock.mockClear()
      await strip()
      key('Shift')
      expect(ends()).toEqual([{ session: 7, how: 'drop', src: 'key', at: 0 }])
    } finally {
      document.removeEventListener('keydown', watcher, true)
    }
  })
})

// ── 셸 입력 ──

describe('셸 입력 — dock:feed', () => {
  beforeEach(() => {
    ctl.setModel(['x', 'y', 'z'], 'y', null)
  })

  it('attach — 문턱 없이 곧바로 자리를 셈해 보내고 이웃은 곧바로 선다 · 모델에 아직 없는 탭도 그린다', () => {
    feed(attachStep())
    expect(reorders()).toEqual([{ session: 9, seq: 1, index: 2 }])
    expect(lastView()).toEqual({
      view: 'n',
      order: ['x', 'y', 'n', 'z'],
      targets: [0, 160, null, 480],
      left: 280,
      slideMs: 0,
      settling: false,
    })
  })

  it('move 로 몰고 release 에 지금 자리로 drop · release', () => {
    feed(attachStep())
    feed({ type: 'move', x: 430, y: 50 })
    expect(lastView()).toMatchObject({ left: 290, slideMs: 200 })
    feed({ type: 'move', x: 620, y: 50 })
    expect(reorders()).toEqual([
      { session: 9, seq: 1, index: 2 },
      { session: 9, seq: 2, index: 3 },
    ])
    feed({ type: 'release' })
    expect(ends()).toEqual([{ session: 9, how: 'drop', src: 'release', at: 3 }])
    expect(lastView()).toMatchObject({ settling: true, left: 480 })
  })

  it('붙은 창도 키를 받아 지금 자리로 끝낸다 · 포인터 사건은 버린다', () => {
    feed(attachStep())
    move(100, 50)
    up(100, 50)
    expect(key('a').defaultPrevented).toBe(true)
    expect(ends()).toEqual([{ session: 9, how: 'drop', src: 'key', at: 2 }])
  })

  it('구역 밖 move → leave · 기다리는 동안 온 release 는 not_ready 뒤 drop · release', async () => {
    feed(attachStep())
    feed({ type: 'move', x: 420, y: 120 })
    expect(calls(CMD_TAB_DRAG_LEAVE)).toEqual([{ session: 9 }])
    feed({ type: 'release' })
    expect(ends()).toEqual([])
    leaves[0].resolve('not_ready')
    await ticks()
    expect(ends()).toEqual([{ session: 9, how: 'drop', src: 'release', at: 2 }])
  })

  it('다른 세션 · 다른 탭의 move · release 는 버린다', () => {
    feed(attachStep())
    feed({ type: 'move', x: 620, y: 50 }, 8)
    feed({ type: 'release' }, 9, 'other')
    expect(reorders()).toHaveLength(1)
    expect(ends()).toEqual([])
  })

  it('떼어 낸 창(away)에 다시 붙으면 같은 세션의 seq 를 잇는다', async () => {
    ctl.setModel(['a', 'b', 'c', 'd'], 'b', null)
    await strip()
    move(300, 50)
    move(300, 100)
    leaves[0].resolve('detached')
    await ticks()
    ctl.setModel(['b', 'c', 'd'], 'b', null)
    feed(attachStep(), 7, 'a')
    expect(reorders()).toEqual([
      { session: 7, seq: 1, index: 1 },
      { session: 7, seq: 2, index: 2 },
    ])
  })
})

// ── 끝남 ──

describe('dock:drag{ended}', () => {
  it('그 세션이면 거둔다 — 그리기 null · 캡처 놓기 · 키 감시 해제 · 다른 세션은 그대로', async () => {
    await strip()
    move(300, 50)
    ctl.dragEvent({ session: 6, phase: 'ended' })
    ctl.dragEvent({ session: 7, phase: 'active' })
    expect(lastView()).not.toBeNull()
    ctl.dragEvent({ session: 7, phase: 'ended' })
    expect(lastView()).toBeNull()
    expect(row.releasePointerCapture).toHaveBeenCalledWith(1)
    expect(key('a').defaultPrevented).toBe(false)
    move(460, 50)
    expect(reorders()).toHaveLength(1)
  })

  it('끝을 기다리는 창째 끌기(away)도 그 세션의 ended 로 거두고 새 누름을 받는다', async () => {
    press()
    move(191, 50)
    begins[0].resolve({ type: 'whole_window', session: 3 })
    await ticks()
    ctl.dragEvent({ session: 3, phase: 'ended' })
    press()
    move(191, 50)
    expect(calls(CMD_TAB_DRAG_BEGIN)).toHaveLength(2)
  })
})

describe('engaged', () => {
  it('누름부터 — 문턱 전 뗌은 그 자리에서, 끌기는 begin 을 기다리는 동안의 뗌 · 놓은 뒤를 지나 그 세션의 ended 에서 끝난다', async () => {
    press()
    up(185, 50)
    expect(engaged).toEqual([true, false])

    press()
    move(191, 50)
    up(191, 50)
    begins[0].resolve({ type: 'strip', session: 7, zone: ZONE })
    await ticks()
    expect(ends()).toHaveLength(1)
    expect(engaged).toEqual([true, false, true])
    ctl.dragEvent({ session: 7, phase: 'ended' })
    expect(engaged).toEqual([true, false, true, false])
  })

  it('창째 끌기 · 뗀 뒤는 그 세션의 ended 까지 · 셸이 붙인 끌기는 붙은 때부터', async () => {
    press()
    move(191, 50)
    begins[0].resolve({ type: 'whole_window', session: 3 })
    await ticks()
    expect(engaged).toEqual([true])
    ctl.dragEvent({ session: 3, phase: 'ended' })
    expect(engaged).toEqual([true, false])
    feed(attachStep())
    expect(engaged).toEqual([true, false, true])
  })
})

// ── 훅 배선 ──

describe('useTabDrag — 훅 배선', () => {
  const emit = (event: string, payload: unknown) => act(() => listeners.get(event)!({ payload }))

  let stopWatch: () => void
  beforeEach(async () => {
    listeners.clear()
    box(row, 100, 40, 700, 28)
    window.matchMedia = vi.fn(() => ({ matches: false }) as MediaQueryList)
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === CMD_TAB_DRAG_BEGIN) return new Promise((resolve, reject) => begins.push({ resolve, reject }))
      if (cmd === 'window_role') return Promise.resolve({ role: 'model', drag: null })
      return Promise.resolve(undefined)
    })
    stopWatch = watchDockSession(() => {})
    await ticks()
  })
  afterEach(() => {
    cleanup()
    stopWatch()
  })

  function mount() {
    const add = document.createElement('button')
    box(add, 760, 40, 40, 28)
    return renderHook(
      ({ tabs }: { tabs: string[] }) =>
        useTabDrag({ tabs, active: 'b', row: { current: row }, add: { current: add } }),
      { initialProps: { tabs: ['a', 'b', 'c', 'd'] } },
    )
  }
  const down = (x: number, y: number) =>
    ({ pointerId: 1, button: 0, clientX: x, clientY: y, target: head, currentTarget: head }) as unknown as ReactPointerEvent<HTMLElement>

  it('누름 → 문턱 → begin(잰 끌기 영역) → strip 답 → 몰기 모습', async () => {
    const { result } = mount()
    act(() => result.current.onPointerDown(down(180, 50), 'a'))
    move(191, 50)
    expect(calls(CMD_TAB_DRAG_BEGIN)[0]).toMatchObject({ area: { x0: 100, y0: 40, x1: 760, y1: 68 } })
    await act(async () => {
      begins[0].resolve({ type: 'strip', session: 7, zone: ZONE })
      await ticks()
    })
    act(() => void move(300, 50))
    expect(result.current.drag).toMatchObject({ view: 'a', order: ['b', 'a', 'c', 'd'], left: 120 })
    emit(EVT_DRAG, { session: 7, phase: 'ended' })
    expect(result.current.drag).toBeNull()
  })

  it('dock:feed attach → 붙은 탭바 몰기 · 그릴 때마다 새 탭 배열이어도 다시 그리지 않는다', () => {
    const { result, rerender } = mount()
    emit(EVT_FEED, { session: 9, view: 'n', step: attachStep() })
    expect(result.current.drag).toMatchObject({ view: 'n', order: ['a', 'b', 'n', 'c', 'd'] })
    const shown = result.current.drag
    rerender({ tabs: ['a', 'b', 'c', 'd'] })
    expect(result.current.drag).toBe(shown)
  })

  it('언마운트 뒤에는 dock:feed 를 받지 않고 키를 건드리지 않는다', () => {
    const { result, unmount } = mount()
    emit(EVT_FEED, { session: 9, view: 'n', step: attachStep() })
    expect(result.current.drag).not.toBeNull()
    unmount()
    invokeMock.mockClear()
    emit(EVT_FEED, { session: 9, view: 'n', step: { type: 'move', x: 620, y: 50 } })
    expect(key('a').defaultPrevented).toBe(false)
    expect(invokeMock).not.toHaveBeenCalled()
  })
})
