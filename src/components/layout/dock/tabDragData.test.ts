// TRD S21-slot-docking §8-2 `tabDragData`: build → read round trip · letter case · missing field · broken field · other version · value JSON · someone else's drag ·
//   shape round trip.

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

/** Left-right (0.3) — left area · right top-bottom (1/3) split. */
const NESTED = split('left_right', 0.3, leaf('a'), split('top_bottom', 1 / 3, leaf('b'), leaf('c')))

describe('type string — build → read', () => {
  it('round trip — every field', () => {
    const mime = buildTabDragType({ app: APP, drag: DRAG, original: { w: 640.4, h: 479.6 }, shape: shapeOf(NESTED) })
    expect(mime).toBe(`${TAB_MIME};app=${APP};drag=${DRAG};w=640;h=480;shape=h0.3_lv0.3333333333333333_ll`)
    const head = readTabDragType(['Files', mime], APP)
    expect(head).not.toBeNull()
    expect(head!.mime).toBe(mime)
    expect(head!.drag).toBe(DRAG)
    expect(head!.original).toEqual({ w: 640, h: 480 })
    expect(shapeOf(head!.shape!)).toBe(shapeOf(NESTED))
  })

  it('builds an uppercase identifier in lowercase and compares in lowercase (Chromium lowercases the format)', () => {
    const mime = buildTabDragType({ app: 'Com.Engram.Dashboard', drag: DRAG, original: null, shape: null })
    expect(mime).toBe(mime.toLowerCase())
    expect(readTabDragType([mime], 'COM.ENGRAM.DASHBOARD')?.drag).toBe(DRAG)
    // Even if the OS passes uppercase as is, it is the same drag.
    expect(readTabDragType([`APPLICATION/X-ENGRAM-TAB;APP=${APP.toUpperCase()};DRAG=${DRAG}`], APP)?.drag).toBe(DRAG)
  })

  it('if the size · drag · shape field is missing only that field is null', () => {
    const head = readTabDragType([`${TAB_MIME};app=${APP}`], APP)
    expect(head).toEqual({ mime: `${TAB_MIME};app=${APP}`, drag: null, original: null, shape: null })
    expect(buildTabDragType({ app: APP, drag: DRAG, original: null, shape: null })).toBe(`${TAB_MIME};app=${APP};drag=${DRAG}`)
  })

  it('filters broken fields — size (0 · negative · fractional · one side only) · empty drag · shape grammar', () => {
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
    // Even with a broken field it is our drag — the remaining fields are read as is.
    expect(read(`w=0;h=10;drag=${DRAG}`).drag).toBe(DRAG)
  })

  it('another app is not our drag · neither is one without an app field', () => {
    expect(readTabDragType([`${TAB_MIME};app=com.engram.spike;drag=${DRAG}`], APP)).toBeNull()
    expect(readTabDragType([`${TAB_MIME};drag=${DRAG}`], APP)).toBeNull()
  })

  it('only Files · text/uri-list is not our drag · nor one whose head merely starts with the same name', () => {
    expect(readTabDragType(['Files'], APP)).toBeNull()
    expect(readTabDragType(['text/uri-list', 'text/plain'], APP)).toBeNull()
    expect(readTabDragType([`${TAB_MIME}-other;app=${APP}`], APP)).toBeNull()
    expect(readTabDragType([], APP)).toBeNull()
  })
})

describe('shape — shapeOf ↔ parseShape', () => {
  it('one area = l', () => {
    expect(shapeOf(leaf('x'))).toBe('l')
    expect(parseShape('l')).toEqual({ type: 'slot', id: 'shape-slot-0', content: { type: 'empty' } })
  })

  it('split · ratio f64 round trip as is', () => {
    for (const ratio of [0.5, 1 / 3, 0.1 + 0.2, 1e-7, 0.999999999999, 5e-324]) {
      const tree = split('top_bottom', ratio, leaf('a'), split('left_right', 1 - ratio, leaf('b'), leaf('c')))
      const back = parseShape(shapeOf(tree))
      expect(back).not.toBeNull()
      expect(shapeOf(back!)).toBe(shapeOf(tree))
      expect(back!.type === 'split' && back!.ratio).toBe(ratio)
    }
  })

  it('ids in pre-order · area content is empty', () => {
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

  it('no characters that break the type string field (; =) and lowercase', () => {
    const text = shapeOf(split('left_right', 1.5e-10, leaf('a'), leaf('b')))
    expect(text).toBe(text.toLowerCase())
    expect(text).not.toMatch(/[;=]/)
  })
})

describe('value JSON', () => {
  const value = {
    drag: DRAG,
    view: 'view-1',
    from: 'main',
    initial: 'view-0',
    size: { w: 800, h: 600 },
    grab: { x: 37.5, y: 14 },
  }

  it('round trip · version v = 1', () => {
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

  it('broken JSON · a different field shape gives null', () => {
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
