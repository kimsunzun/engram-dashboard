// TRD S21-slot-docking §8-2 `tabDragData`: 짓기 → 읽기 왕복 · 대소문자 · 빠진 칸 · 깨진 칸 · 다른 판 · 값 JSON · 남의 끌기 ·
//   모양 왕복.

import { describe, expect, it } from 'vitest'

import type { LayoutNode } from '../../../api/layoutTypes'
import {
  TAB_MIME,
  buildTabDragType,
  buildTabDragValue,
  parseShape,
  readTabDragType,
  readTabDragValue,
  shapeOf,
} from './tabDragData'

const APP = 'com.engram.dashboard'
const DRAG = '0f8e6a2c-5b1d-4c3e-9a7f-2d4b6c8e0a1b'

const leaf = (id: string): LayoutNode => ({ type: 'slot', id, content: { type: 'empty' } })
const split = (dir: 'left_right' | 'top_bottom', ratio: number, a: LayoutNode, b: LayoutNode): LayoutNode => ({
  type: 'split',
  id: `s-${ratio}`,
  dir,
  ratio,
  a,
  b,
})

/** 좌우(0.3) — 왼쪽 칸 · 오른쪽 위아래(1/3) 분할. */
const NESTED = split('left_right', 0.3, leaf('a'), split('top_bottom', 1 / 3, leaf('b'), leaf('c')))

describe('타입 문자열 — 짓기 → 읽기', () => {
  it('왕복 — 모든 칸', () => {
    const mime = buildTabDragType({ app: APP, drag: DRAG, original: { w: 640.4, h: 479.6 }, shape: shapeOf(NESTED) })
    expect(mime).toBe(`${TAB_MIME};app=${APP};drag=${DRAG};w=640;h=480;shape=h0.3_lv0.3333333333333333_ll`)
    const head = readTabDragType(['Files', mime], APP)
    expect(head).not.toBeNull()
    expect(head!.mime).toBe(mime)
    expect(head!.drag).toBe(DRAG)
    expect(head!.original).toEqual({ w: 640, h: 480 })
    expect(shapeOf(head!.shape!)).toBe(shapeOf(NESTED))
  })

  it('대문자 identifier 를 소문자로 짓고 소문자로 견준다(Chromium 이 형식을 소문자로 바꾼다)', () => {
    const mime = buildTabDragType({ app: 'Com.Engram.Dashboard', drag: DRAG, original: null, shape: null })
    expect(mime).toBe(mime.toLowerCase())
    expect(readTabDragType([mime], 'COM.ENGRAM.DASHBOARD')?.drag).toBe(DRAG)
    // OS 가 대문자를 그대로 넘겨도 같은 끌기다.
    expect(readTabDragType([`APPLICATION/X-ENGRAM-TAB;APP=${APP.toUpperCase()};DRAG=${DRAG}`], APP)?.drag).toBe(DRAG)
  })

  it('크기 · drag · 모양 칸이 없으면 그 칸만 null', () => {
    const head = readTabDragType([`${TAB_MIME};app=${APP}`], APP)
    expect(head).toEqual({ mime: `${TAB_MIME};app=${APP}`, drag: null, original: null, shape: null })
    expect(buildTabDragType({ app: APP, drag: DRAG, original: null, shape: null })).toBe(`${TAB_MIME};app=${APP};drag=${DRAG}`)
  })

  it('깨진 칸을 거른다 — 크기(0 · 음수 · 소수 · 한 변만) · 빈 drag · 모양 문법', () => {
    const read = (params: string) => readTabDragType([`${TAB_MIME};app=${APP};${params}`], APP)!
    expect(read('w=0;h=10').original).toBeNull()
    expect(read('w=-5;h=10').original).toBeNull()
    expect(read('w=1.5;h=10').original).toBeNull()
    expect(read('w=10').original).toBeNull()
    expect(read('w=abc;h=10').original).toBeNull()
    expect(read('drag=').drag).toBeNull()
    for (const bad of ['', 'x', 'h0.5_l', 'h0.5_lll', 'h_ll', 'hnan_ll', 'h0.5ll', 'hinfinity_ll', 'h1e999_ll']) {
      expect(read(`shape=${bad}`).shape, bad).toBeNull()
    }
    // 깨진 칸이 있어도 우리 끌기다 — 나머지 칸은 그대로 읽는다.
    expect(read(`w=0;h=10;drag=${DRAG}`).drag).toBe(DRAG)
  })

  it('다른 app 은 우리 끌기가 아니다 · app 칸이 없어도 아니다', () => {
    expect(readTabDragType([`${TAB_MIME};app=com.engram.spike;drag=${DRAG}`], APP)).toBeNull()
    expect(readTabDragType([`${TAB_MIME};drag=${DRAG}`], APP)).toBeNull()
  })

  it('Files · text/uri-list 만이면 우리 끌기가 아니다 · 머리가 같은 이름으로 시작만 하는 것도 아니다', () => {
    expect(readTabDragType(['Files'], APP)).toBeNull()
    expect(readTabDragType(['text/uri-list', 'text/plain'], APP)).toBeNull()
    expect(readTabDragType([`${TAB_MIME}-other;app=${APP}`], APP)).toBeNull()
    expect(readTabDragType([], APP)).toBeNull()
  })
})

describe('모양 — shapeOf ↔ parseShape', () => {
  it('칸 하나 = l', () => {
    expect(shapeOf(leaf('x'))).toBe('l')
    expect(parseShape('l')).toEqual({ type: 'slot', id: 'shape-slot-0', content: { type: 'empty' } })
  })

  it('분할 · 비율 f64 그대로 왕복', () => {
    for (const ratio of [0.5, 1 / 3, 0.1 + 0.2, 1e-7, 0.999999999999, 5e-324]) {
      const tree = split('top_bottom', ratio, leaf('a'), split('left_right', 1 - ratio, leaf('b'), leaf('c')))
      const back = parseShape(shapeOf(tree))
      expect(back).not.toBeNull()
      expect(shapeOf(back!)).toBe(shapeOf(tree))
      expect(back!.type === 'split' && back!.ratio).toBe(ratio)
    }
  })

  it('id 는 전위 순 · 칸 내용은 empty', () => {
    expect(parseShape(shapeOf(NESTED))).toEqual({
      type: 'split',
      id: 'shape-split-0',
      dir: 'left_right',
      ratio: 0.3,
      a: { type: 'slot', id: 'shape-slot-0', content: { type: 'empty' } },
      b: {
        type: 'split',
        id: 'shape-split-1',
        dir: 'top_bottom',
        ratio: 1 / 3,
        a: { type: 'slot', id: 'shape-slot-1', content: { type: 'empty' } },
        b: { type: 'slot', id: 'shape-slot-2', content: { type: 'empty' } },
      },
    })
  })

  it('타입 문자열 칸을 깨는 글자(; =)가 없고 소문자다', () => {
    const text = shapeOf(split('left_right', 1.5e-10, leaf('a'), leaf('b')))
    expect(text).toBe(text.toLowerCase())
    expect(text).not.toMatch(/[;=]/)
  })
})

describe('값 JSON', () => {
  const value = {
    drag: DRAG,
    view: 'view-1',
    from: 'main',
    initial: 'view-0',
    size: { w: 800, h: 600 },
    grab: { x: 37.5, y: 14 },
  }

  it('왕복 · 판 v = 1', () => {
    const raw = buildTabDragValue(value)
    expect(JSON.parse(raw).v).toBe(1)
    expect(readTabDragValue(raw)).toEqual({ v: 1, ...value })
    expect(readTabDragValue(buildTabDragValue({ ...value, initial: null, size: null }))).toEqual({
      v: 1,
      ...value,
      initial: null,
      size: null,
    })
  })

  it('JSON 깨짐 · 칸 모양이 다르면 null', () => {
    expect(readTabDragValue('')).toBeNull()
    expect(readTabDragValue('{')).toBeNull()
    expect(readTabDragValue('null')).toBeNull()
    expect(readTabDragValue('"text"')).toBeNull()
    const bad: Array<Record<string, unknown>> = [
      { ...value, v: 2 },
      { ...value, drag: 5 },
      { ...value, view: undefined },
      { ...value, from: null },
      { ...value, initial: 3 },
      { ...value, size: { w: 1 } },
      { ...value, size: { w: 'a', h: 1 } },
      { ...value, grab: null },
      { ...value, grab: { x: 1, y: Infinity } },
    ]
    for (const b of bad) {
      expect(readTabDragValue(JSON.stringify({ v: 1, ...b })), JSON.stringify(b)).toBeNull()
    }
  })
})
