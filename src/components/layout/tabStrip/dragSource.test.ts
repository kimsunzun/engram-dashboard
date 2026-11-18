// TRD S21-slot-docking §8-2 the source's share of `dragSource` (send · release · revert · order move confirmation) · §8-3 source window order test (fast-check `fc.scheduler()` mixes `done` · invoke responses · `window:tabs-updated` · `layout:updated` · dragend in any order). The outside is a fake port (ADR-0012); tab bar geometry exactly emulates how the tab bar draws.

import fc from 'fast-check'
import { describe, expect, it, vi } from 'vitest'

import type { DropOutcome } from '../../../../src-tauri/bindings/DropOutcome'
import type { EndEffect } from '../../../../src-tauri/bindings/EndEffect'
import type { TabDrop } from '../../../../src-tauri/bindings/TabDrop'
import type { TabDropReply } from '../../../../src-tauri/bindings/TabDropReply'
import { stripZone } from '../dock/bands'
import { shownTab, stripHeld, type WindowModel } from '../dock/dragOverride'
import { createLocalDrag, type LocalDrag, type StripGeometry } from '../dock/localDrag'
import { buildTabDragType } from '../dock/tabDragData'
import { createWindowDrop, type RootDragEvent } from '../dock/windowDrop'
import { createDragSource, type DragSource, type DragSourcePorts, type DragSourceSnapshot, type DragStart } from './dragSource'
import { stripBounds, stripZoneArea } from './measure'

const WIN = 'main'
const DRAG = 'drag-1'
/** Active A before the press · the press made B active (R2). Three tabs · usable width 480 → heads of 160 each. */
const AT_START: WindowModel = { tabs: ['A', 'B', 'C'], active: 'B' }
const START: DragStart = {
  window: WIN,
  drag: DRAG,
  view: 'B',
  initial: 'A',
  size: { w: 800, h: 600 },
  grab: { x: 80, y: 14 },
  grabOffset: 80,
  left: 160,
  x: 244,
}

/** `+` box width (gap 4 + circle 22 — `ADD_STYLE` of `TabBar.tsx`). */
const ADD_W = 26

/**
 * Exactly the order · bounds · zone the tab bar draws (computation of `TabBar` · `useTabDragSource.geometryOf`) — tab bar row x 0..`available` + `+` width ·
 * y 0..28 (no title row).
 */
function geometryOf(snap: DragSourceSnapshot, model: WindowModel, available = 480): StripGeometry {
  const motion = snap.window === WIN && stripHeld(snap.override, model) ? snap.motion : null
  const others = motion === null ? [...model.tabs] : model.tabs.filter(t => t !== motion.view)
  const order = motion === null ? others : [...others.slice(0, motion.index), motion.view, ...others.slice(motion.index)]
  const at = motion === null ? -1 : order.indexOf(motion.view)
  const bounds = stripBounds({ available, count: order.length, active: at, dragged: at >= 0 ? at : undefined })
  const m = { row: { x0: 0, y0: 0, x1: available + ADD_W, y1: 28 }, left: 0, available, addWidth: ADD_W }
  return { left: 0, bounds, available, zoneViewport: stripZone(stripZoneArea(m, bounds, null)), tabs: order }
}

interface Harness {
  src: DragSource
  local: LocalDrag
  ports: DragSourcePorts
  sends: TabDrop[]
  switches: Array<[string, string, string]>
  later: Array<() => void>
  setModel(m: WindowModel): void
  model(): WindowModel
  shown(): string | null
}

function harness(over: Partial<Pick<DragSourcePorts, 'send' | 'switchTabIf'>> = {}, available = 480): Harness {
  const local = createLocalDrag()
  const sends: TabDrop[] = []
  const switches: Array<[string, string, string]> = []
  const later: Array<() => void> = []
  let model = AT_START
  const ports: DragSourcePorts = {
    send: drop => {
      sends.push(drop)
      return over.send?.(drop) ?? Promise.resolve({ type: 'ignored', why: 'lost-drop' })
    },
    switchTabIf: (w, to, expect) => {
      switches.push([w, to, expect])
      return over.switchTabIf?.(w, to, expect) ?? Promise.resolve(true)
    },
    local,
    nextTask: fn => void later.push(fn),
  }
  const src = createDragSource(ports)
  src.model(WIN, model)
  local.subscribe(() => src.stripChanged())
  local.registerStripGeometry(() => geometryOf(src.getSnapshot(), model, available))
  return {
    src,
    local,
    ports,
    sends,
    switches,
    later,
    setModel(m) {
      model = m
      src.model(WIN, m)
    },
    model: () => model,
    shown() {
      const s = src.getSnapshot()
      return s.window === WIN ? shownTab(s.override, model) : model.active
    },
  }
}

/** Send dragstart to a past task — it is not a drag the engine collapsed. */
function started(h: Harness, s: DragStart = START): void {
  h.src.start(s)
  for (const fn of h.later.splice(0)) fn()
}

const tick = () => new Promise(resolve => setTimeout(resolve, 0))

describe('dragSource — dragstart', () => {
  it('raises the outer mark — the leaving drag · the source tab bar\'s first verdict (press point = inside the zone) · no order move drawn yet', () => {
    const h = harness()
    h.src.start(START)
    expect(h.local.getOutgoing()).toEqual({ drag: DRAG, window: WIN, view: 'B', grabOffset: 80 })
    expect(h.local.getSourceStrip()).toEqual({ index: 1, left: 160, x: 244 })
    const snap = h.src.getSnapshot()
    expect(snap.window).toBe(WIN)
    expect(snap.override.phase).toBe('live')
    expect(snap.motion).toBeNull()
    expect(h.shown()).toBe('B')
    expect(h.sends).toEqual([])
  })
})

describe('dragSource — while dragging (source tab bar verdict)', () => {
  it('outside the zone (null) = the remaining tab · when it comes back the dragged tab · the order move only after an inside-zone verdict arrives', () => {
    const h = harness()
    started(h)
    h.local.setSourceStrip(null)
    expect(h.shown()).toBe('A')
    expect(h.src.getSnapshot().motion).toBeNull()
    h.local.setSourceStrip({ index: 1, left: 170, x: 254 })
    expect(h.shown()).toBe('B')
    expect(h.src.getSnapshot().motion).toEqual({ view: 'B', index: 1, left: 170, settled: false })
  })

  // T-66 · D-a: for its own tab too, once the cursor passes the right end of `+` it is outside the zone (rejected: keep it attached up to the end of the usable width). Decided by the source window root's `windowDrop` with the tab bar geometry's zone (`stripZoneArea`).
  it('horizontal overflow — if the cursor passes the right end of + it is outside the zone (order move stops · remaining tab) · when it returns the order move stands again', () => {
    // Three heads of 160 (0..480) · + 480..506 · usable width 600 (row 0..626 — the old zone went up to 600) · canvas origin (0, 28).
    const h = harness({}, 600)
    const type = buildTabDragType({ app: 'com.engram.test', drag: DRAG, original: { w: 800, h: 600 }, shape: 'l' })
    const root = createWindowDrop({
      app: () => 'com.engram.test',
      local: h.local,
      canvasRect: () => ({ left: 0, top: 28, width: 1200, height: 800 }),
      shownTab: () => h.shown(),
      view: () => null,
      model: () => h.model(),
      send: () => Promise.resolve({ type: 'ignored', why: 'lost-drop' }),
      setTimer: () => 0,
      clearTimer: () => {},
      watchCanvas: () => () => {},
    })
    const at = (x: number): RootDragEvent => ({
      clientX: x,
      clientY: 14,
      relatedTarget: null,
      dataTransfer: { types: [type], dropEffect: 'none', getData: () => '' },
      preventDefault: () => {},
      stopPropagation: () => {},
    })
    started(h)

    root.dragEnter(at(244))
    expect(h.local.getSourceStrip()).toEqual({ index: 1, left: 164, x: 244 })
    expect(h.src.getSnapshot().motion).toEqual({ view: 'B', index: 1, left: 164, settled: false })
    // Over + is inside the zone — the dragged head stays at the end position (320).
    root.dragOver(at(500))
    expect(h.local.getSourceStrip()).toEqual({ index: 2, left: 320, x: 500 })
    expect(h.src.getSnapshot().motion?.index).toBe(2)
    // Empty space beyond the right end of + (506) — outside the zone: withdraw the order move and show the remaining tab (A, active before the press).
    root.dragOver(at(506))
    expect(h.local.getSourceStrip()).toBeNull()
    expect(h.src.getSnapshot().motion).toBeNull()
    expect(h.shown()).toBe('A')
    root.dragOver(at(590))
    expect(h.local.getSourceStrip()).toBeNull()
    // Re-entering confirms immediately (no threshold) · the dragged tab is shown again.
    root.dragOver(at(300))
    expect(h.local.getSourceStrip()).toEqual({ index: 1, left: 220, x: 300 })
    expect(h.src.getSnapshot().motion).toEqual({ view: 'B', index: 1, left: 220, settled: false })
    expect(h.shown()).toBe('B')
  })

  it('order threshold — the first verdict entering the zone is confirmed at once · after that it must exceed the threshold (16 px) from the last move point to go to the verdict position', () => {
    const h = harness()
    started(h)
    // First decision (justAttached) — position 2 is confirmed even if the cursor barely moved.
    h.local.setSourceStrip({ index: 2, left: 250, x: 250 })
    expect(h.src.getSnapshot().motion?.index).toBe(2)
    // A return within the threshold — only the head follows.
    h.local.setSourceStrip({ index: 1, left: 240, x: 240 })
    expect(h.src.getSnapshot().motion).toEqual({ view: 'B', index: 2, left: 240, settled: false })
    // Past the threshold, to that decision's position.
    h.local.setSourceStrip({ index: 1, left: 200, x: 200 })
    expect(h.src.getSnapshot().motion?.index).toBe(1)
    // Leaving the zone gives the model order · re-entering confirms immediately.
    h.local.setSourceStrip(null)
    expect(h.src.getSnapshot().motion).toBeNull()
    h.local.setSourceStrip({ index: 2, left: 205, x: 205 })
    expect(h.src.getSnapshot().motion?.index).toBe(2)
    // Even past the threshold, an unchanged position keeps the last move point (205) — moving it would put the next decision (180 − 187 = 7) inside the threshold. ★Threshold 16 is this harness's value (three tabs · 480 px) — recompute whether 187 passes it if width or tab count changes★
    h.local.setSourceStrip({ index: 2, left: 187, x: 187 })
    expect(h.src.getSnapshot().motion?.index).toBe(2)
    h.local.setSourceStrip({ index: 1, left: 180, x: 180 })
    expect(h.src.getSnapshot().motion?.index).toBe(1)
  })

  it('a verdict after dragend is not listened to — the dragged head goes to the confirmed position (settled)', () => {
    const h = harness()
    started(h)
    h.local.setSourceStrip({ index: 2, left: 330, x: 420 })
    h.src.end('B', 'move')
    expect(h.src.getSnapshot().motion).toEqual({ view: 'B', index: 2, left: 330, settled: true })
    h.local.setSourceStrip(null)
    expect(h.src.getSnapshot().motion).toEqual({ view: 'B', index: 2, left: 330, settled: true })
    expect(h.shown()).toBe('B')
  })

  it('if that tab leaves that window during the drag (LLM close · move) the swapped drawing is released', () => {
    const h = harness()
    started(h)
    h.local.setSourceStrip(null)
    h.setModel({ tabs: ['A', 'C'], active: 'A' })
    expect(h.src.getSnapshot().override.phase).toBe('idle')
    expect(h.local.getOutgoing()).toBeNull()
  })
})

describe('dragSource — dragend', () => {
  it.each(['none', 'move'] as EndEffect[])(
    'effect=%s → feeds the end to the reducer first and one NewWindow{by: DragEnd{effect}} · a second dragend is a no-op',
    effect => {
      let phaseAtSend: string | null = null
      const h = harness({
        send: () => {
          phaseAtSend = h.src.getSnapshot().override.phase
          return Promise.resolve({ type: 'ignored', why: 'accepted-ours' })
        },
      })
      started(h)
      h.src.end('B', effect)
      h.src.end('B', effect)
      expect(phaseAtSend).toBe('ended')
      expect(h.sends).toEqual([
        {
          drag: DRAG,
          view: 'B',
          from: WIN,
          initial: 'A',
          to: { type: 'new_window', by: { type: 'drag_end', effect }, size: { w: 800, h: 600 }, grab: { x: 80, y: 14 } },
        },
      ])
    },
  )

  it('same-webview in-place drop (mark) → sends nothing · release at once (order move reaped) · takes the mark', () => {
    const h = harness()
    started(h)
    h.local.setSourceStrip({ index: 2, left: 330, x: 420 })
    h.local.markLocalDrop(DRAG)
    h.src.end('B', 'move')
    expect(h.sends).toEqual([])
    expect(h.src.getSnapshot()).toEqual({ window: null, override: { phase: 'idle' }, motion: null })
    expect(h.local.getOutgoing()).toBeNull()
    expect(h.local.takeLocalDrop(DRAG)).toBe(false)
  })

  it('dragend in the same task as dragstart (the engine collapsed it) → sends nothing · release · not reverted by done afterwards', () => {
    const h = harness()
    h.src.start(START)
    h.src.end('B', 'none')
    for (const fn of h.later.splice(0)) fn()
    expect(h.sends).toEqual([])
    expect(h.src.getSnapshot().override.phase).toBe('idle')
    h.src.done({ drag: DRAG, outcome: 'canceled' })
    expect(h.switches).toEqual([])
  })

  it('dragend of another head · outside a drag is a no-op', () => {
    const h = harness()
    h.src.end('B', 'none')
    started(h)
    h.src.end('C', 'none')
    expect(h.sends).toEqual([])
    expect(h.src.getSnapshot().override.phase).toBe('live')
  })

  it('invoke error → release at once (only that drag)', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const h = harness({ send: () => Promise.reject(new Error('ipc down')) })
    started(h)
    h.local.setSourceStrip(null)
    h.src.end('B', 'none')
    expect(h.shown()).toBe('A')
    await tick()
    expect(h.src.getSnapshot().override.phase).toBe('idle')
    expect(h.shown()).toBe('B')
    vi.restoreAllMocks()
  })
})

describe('dragSource — result notice · restoring the original selection (§4-6)', () => {
  it('done{canceled} (own dragend none) → switch_tab_if(window, initial, expect = dragged tab) once · release when it lands', () => {
    const h = harness()
    started(h)
    h.src.end('B', 'none')
    h.src.done({ drag: DRAG, outcome: 'canceled' })
    h.src.done({ drag: DRAG, outcome: 'canceled' })
    expect(h.switches).toEqual([[WIN, 'A', 'B']])
    expect(h.shown()).toBe('A')
    h.setModel({ tabs: ['A', 'B', 'C'], active: 'A' })
    expect(h.src.getSnapshot().override.phase).toBe('idle')
    expect(h.local.getOutgoing()).toBeNull()
  })

  it('no revert — the active changed in between · initial = dragged tab · Rejected · own dragend move · someone else\'s drag', () => {
    const run = (outcome: DropOutcome, effect: EndEffect, setup?: (h: Harness) => void, drag = DRAG, s = START) => {
      const h = harness()
      started(h, s)
      h.src.end(s.view, effect)
      setup?.(h)
      h.src.done({ drag, outcome })
      return h.switches
    }
    expect(run('canceled', 'none', h => h.setModel({ tabs: ['A', 'B', 'C'], active: 'C' }))).toEqual([])
    expect(run('ignored', 'none', undefined, DRAG, { ...START, initial: 'B' })).toEqual([])
    expect(run('rejected', 'none')).toEqual([])
    expect(run('ignored', 'move')).toEqual([])
    expect(run('canceled', 'move')).toEqual([])
    expect(run('canceled', 'none', undefined, 'drag-0')).toEqual([])
  })

  it('revert error → caught and only logged · falls back to the model active', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const h = harness({ switchTabIf: () => Promise.reject(new Error('TabNotFound')) })
    started(h)
    h.src.end('B', 'none')
    h.src.done({ drag: DRAG, outcome: 'canceled' })
    expect(h.shown()).toBe('A')
    await tick()
    expect(warn).toHaveBeenCalled()
    expect(h.src.getSnapshot().override.phase).toBe('idle')
    expect(h.shown()).toBe('B')
    vi.restoreAllMocks()
  })

  it('a tab notice is a trigger on every apply — even with the same content (rename etc.), if after dragend the active is not the dragged tab it releases (②)', () => {
    const h = harness()
    // The press's switch echo has not come yet — model active = A.
    h.setModel({ tabs: ['A', 'B', 'C'], active: 'A' })
    started(h)
    h.src.end('B', 'move')
    expect(h.src.getSnapshot().override.phase).toBe('ended')
    h.setModel({ tabs: ['A', 'B', 'C'], active: 'A' })
    expect(h.src.getSnapshot().override.phase).toBe('idle')
  })

  it('the leaving-drag mark is lifted only when that drag is released — a mark covered by the next dragstart is not lifted by the earlier drag', () => {
    const h = harness()
    started(h)
    h.src.end('B', 'move')
    started(h, { ...START, drag: 'drag-2', view: 'C' })
    h.src.done({ drag: DRAG, outcome: 'applied' })
    expect(h.local.getOutgoing()?.drag).toBe('drag-2')
  })
})

describe('dragSource — a press before the revert lands (forceSwitch)', () => {
  /** After issuing the revert (A, expect B) of the cancelled drag B — the model active is still B. */
  function reverted(over: Partial<Pick<DragSourcePorts, 'switchTabIf'>> = {}): Harness {
    const h = harness(over)
    started(h)
    h.src.end('B', 'none')
    h.src.done({ drag: DRAG, outcome: 'canceled' })
    expect(h.switches).toEqual([[WIN, 'A', 'B']])
    return h
  }

  it('only a press on the dragged tab (expect) · only that window · true only once — also after another pointerdown that released the swapped drawing · a tab notice with the active unchanged', () => {
    const h = reverted()
    expect(h.src.forceSwitch(WIN, 'A')).toBe(false)
    expect(h.src.forceSwitch('slot-popup-1', 'B')).toBe(false)
    // A canvas press released the swap redraw — the revert is still in flight.
    h.src.pointerDown()
    expect(h.src.getSnapshot().override.phase).toBe('idle')
    h.setModel({ tabs: ['A', 'B', 'C'], active: 'B' })
    expect(h.src.forceSwitch(WIN, 'B')).toBe(true)
    expect(h.src.forceSwitch(WIN, 'B')).toBe(false)
  })

  it('when the model active leaves expect (revert landed · someone else changed) · on a revert error · with no revert it is false', async () => {
    const landed = reverted()
    landed.setModel({ tabs: ['A', 'B', 'C'], active: 'A' })
    expect(landed.src.forceSwitch(WIN, 'B')).toBe(false)

    const moved = reverted()
    moved.setModel({ tabs: ['A', 'B', 'C'], active: 'C' })
    expect(moved.src.forceSwitch(WIN, 'B')).toBe(false)

    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const failed = reverted({ switchTabIf: () => Promise.reject(new Error('TabNotFound')) })
    await tick()
    expect(failed.src.forceSwitch(WIN, 'B')).toBe(false)
    vi.restoreAllMocks()

    const none = harness()
    started(none)
    none.src.end('B', 'move')
    none.src.done({ drag: DRAG, outcome: 'canceled' })
    expect(none.switches).toEqual([])
    expect(none.src.forceSwitch(WIN, 'B')).toBe(false)
  })

  it('the next revert replaces the earlier one — the earlier revert\'s error does not reap the new one', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    let fail: (err: Error) => void = () => {}
    let calls = 0
    const h = reverted({
      switchTabIf: () => (++calls === 1 ? new Promise<boolean>((_, reject) => (fail = reject)) : new Promise<boolean>(() => {})),
    })
    started(h, { ...START, drag: 'drag-2' })
    h.src.end('B', 'none')
    h.src.done({ drag: 'drag-2', outcome: 'canceled' })
    expect(h.switches).toEqual([
      [WIN, 'A', 'B'],
      [WIN, 'A', 'B'],
    ])
    fail(new Error('TabNotFound'))
    await tick()
    expect(h.src.forceSwitch(WIN, 'B')).toBe(true)
    vi.restoreAllMocks()
  })
})

// ── order test — source window (§8-3) ──────────────────────────────────────────────────

interface Arrival {
  label: string
  /** What arrives — model notice (applied to that window's store) · `done` · `layout:updated`. */
  run(h: Harness): void
  /** Arrives only after dragend (what that dragend produced · what changed after dragend). */
  after?: boolean
}

interface Scenario {
  name: string
  effect: EndEffect
  /** Left the source tab bar zone before dragend. */
  out?: boolean
  /** Moved to position 2 inside the zone before dragend. */
  move?: boolean
  /** Same-webview in-place drop — the root drop leaves a mark. */
  local?: boolean
  /** Its own `DragEnd` invoke — the answer and the `done` after it (if none, no notice · `null` = invoke error). */
  dragEnd: { reply: TabDropReply; done: DropOutcome | null } | null
  arrivals: Arrival[]
  /** It must be released at the end. */
  releases: boolean
  reverts: number
  /** The dragged tab leaves that window — after the swap redraw, the dragged tab is not drawn for even one frame. */
  leaves?: boolean
  /** Same-window reorder — the order move is drawn until the frame where the model order changes and withdrawn on that frame. */
  reorder?: boolean
}

const tabsOf = (label: string, m: WindowModel, after = false): Arrival => ({ label, run: h => h.setModel(m), after })
const doneOf = (label: string, outcome: DropOutcome, after = false): Arrival => ({
  label,
  run: h => h.src.done({ drag: DRAG, outcome }),
  after,
})
// `layout:updated` changes only the view layout cache — it does not touch that window's tab state (`windows[label]`), so nothing reaches the source.
const layoutNoise: Arrival = { label: 'layout:updated', run: () => {} }

const SCENARIOS: Scenario[] = [
  {
    name: 'dropped on the tab bar of another window · area (the Drop of that window applied) — the dragged tab leaves',
    effect: 'move',
    out: true,
    dragEnd: { reply: { type: 'ignored', why: 'accepted-ours' }, done: 'ignored' },
    arrivals: [tabsOf('tabs:drop', { tabs: ['A', 'C'], active: 'A' }), doneOf('done:drop', 'applied'), layoutNoise],
    releases: true,
    reverts: 0,
    leaves: true,
  },
  {
    name: 'new window outside the window (DragEnd applied) — the dragged tab leaves',
    effect: 'none',
    out: true,
    dragEnd: {
      reply: {
        type: 'applied',
        window: 'slot-popup-9',
        window_id: '7d2f0b8e-1c3a-4f5e-9b6d-2a4c6e8f0a1b',
        number: 2,
        reused: false,
      },
      done: 'applied',
    },
    arrivals: [tabsOf('tabs:new-window', { tabs: ['A', 'C'], active: 'A' }, true), layoutNoise],
    releases: true,
    reverts: 0,
    leaves: true,
  },
  {
    name: 'Esc cancel — reverts to the original selection',
    effect: 'none',
    out: true,
    dragEnd: { reply: { type: 'canceled' }, done: 'canceled' },
    arrivals: [layoutNoise],
    releases: true,
    reverts: 1,
    leaves: true,
  },
  {
    name: 'same-window reorder (the Drop of this window applied)',
    effect: 'move',
    move: true,
    dragEnd: { reply: { type: 'ignored', why: 'accepted-ours' }, done: 'ignored' },
    arrivals: [tabsOf('tabs:reorder', { tabs: ['A', 'C', 'B'], active: 'B' }), doneOf('done:drop', 'applied'), layoutNoise],
    releases: true,
    reverts: 0,
    reorder: true,
  },
  {
    name: 'own DragEnd invoke error',
    effect: 'none',
    out: true,
    dragEnd: null,
    arrivals: [layoutNoise],
    releases: true,
    reverts: 0,
  },
  {
    name: 'the Drop of the receiving window lost · after dragend the LLM moves to another tab',
    effect: 'move',
    out: true,
    dragEnd: { reply: { type: 'ignored', why: 'accepted-ours' }, done: 'ignored' },
    arrivals: [tabsOf('tabs:llm', { tabs: ['A', 'B', 'C'], active: 'C' }, true)],
    releases: true,
    reverts: 0,
  },
  {
    name: 'same-webview in-place drop — sends nothing',
    effect: 'move',
    move: true,
    local: true,
    dragEnd: { reply: { type: 'duplicate' }, done: null },
    arrivals: [layoutNoise],
    releases: true,
    reverts: 0,
  },
]

interface Frame {
  label: string
  snap: DragSourceSnapshot
  model: WindowModel
}

async function play(sc: Scenario, s: fc.Scheduler) {
  const frames: Frame[] = []
  let sentWhileLive = false
  let h: Harness | null = null
  const frame = (label: string) => frames.push({ label, snap: h!.src.getSnapshot(), model: h!.model() })
  const deliver = (a: Arrival) => {
    a.run(h!)
    frame(a.label)
  }
  h = harness({
    send: drop => {
      if (h!.src.getSnapshot().override.phase === 'live') sentWhileLive = true
      const de = sc.dragEnd
      if (de === null) {
        return s.schedule(Promise.resolve(), 'reply:error').then(() => {
          throw new Error('ipc down')
        })
      }
      const outcome = de.done
      if (outcome !== null) {
        void s.schedule(Promise.resolve(), 'done:drag-end').then(() => {
          h!.src.done({ drag: drop.drag, outcome })
          frame('done:drag-end')
        })
      }
      return s.schedule(Promise.resolve(de.reply), 'reply')
    },
    // The revert round trip — when it reaches the shell, if that window's active is still `expect` it is changed and a notice follows (if it changed in the meantime, nothing happens).
    switchTabIf: (_w, to, expect) =>
      s.schedule(Promise.resolve(), 'switch_tab_if').then(() => {
        const m = h!.model()
        if (m.active !== expect || !m.tabs.includes(to)) return false
        deliver(tabsOf('tabs:revert', { tabs: m.tabs, active: to }))
        return true
      }),
  })
  started(h)
  if (sc.move) h.local.setSourceStrip({ index: 2, left: 330, x: 420 })
  if (sc.out) h.local.setSourceStrip(null)
  frame('start')

  const scheduleAll = (after: boolean) => {
    for (const a of sc.arrivals.filter(x => (x.after ?? false) === after)) {
      void s.schedule(Promise.resolve(), a.label).then(() => deliver(a))
    }
  }
  scheduleAll(false)
  void s.schedule(Promise.resolve(), 'dragend').then(() => {
    if (sc.local) h!.local.markLocalDrop(DRAG)
    h!.src.end('B', sc.effect)
    frame('dragend')
    scheduleAll(true)
  })
  await s.waitIdle()
  // Handling the invoke response (releasing on error) comes after the response is handed over, so it is not caught in the event frame — record the end state once more.
  frame('idle')
  return { h, frames, sentWhileLive }
}

const shownOf = (f: Frame) => (f.snap.window === WIN ? shownTab(f.snap.override, f.model) : f.model.active)
const drawnOf = (f: Frame) => f.snap.window === WIN && stripHeld(f.snap.override, f.model) && f.snap.motion !== null

describe('dragSource — order test (fc.scheduler · source window)', () => {
  for (const sc of SCENARIOS) {
    it(sc.name, async () => {
      const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
      try {
        await fc.assert(
          fc.asyncProperty(fc.scheduler(), async s => {
            const { h, frames, sentWhileLive } = await play(sc, s)
            const where = () => s.report().map(r => r.label).join(' → ')

            // Send — outside an in-place drop, exactly one per dragend · always after feeding the end into the reducer.
            expect(h.sends.length, where()).toBe(sc.local ? 0 : 1)
            expect(sentWhileLive, where()).toBe(false)

            // ① The branch that must be released was released at the end · ③ the tab drawn after release = the model active.
            const last = frames[frames.length - 1]
            if (sc.releases) expect(last.snap.override.phase, where()).toBe('idle')
            expect(shownOf(last), where()).toBe(last.model.active)
            if (sc.releases) expect(h.local.getOutgoing(), where()).toBeNull()

            // ② The branch where the dragged tab leaves that window — after the swap redraw, the dragged tab is not drawn for even one frame (no deadline flip).
            if (sc.leaves) for (const f of frames) expect(shownOf(f), `${f.label} @ ${where()}`).not.toBe('B')

            // ④ Same-window reorder — the order move is drawn until the frame where the model order changes and withdrawn on that frame (no jump). In this branch
            //   there is no release trigger before that — withdrawing first would return the head to the old order and then jump again to the model order.
            if (sc.reorder) {
              for (const f of frames) {
                const same = f.model.tabs.join() === AT_START.tabs.join()
                expect(drawnOf(f), `${f.label} @ ${where()}`).toBe(same)
              }
            }

            // ⑤ At most one revert · the exact count per branch.
            expect(h.switches.length, where()).toBe(sc.reverts)
          }),
          { numRuns: 150 },
        )
      } finally {
        warn.mockRestore()
      }
    })
  }

  it('⑥ if a tabs-updated (LLM) that does not keep the dragged tab active comes after dragend, the tab drawn afterwards = the model active', async () => {
    const sc = SCENARIOS.find(x => x.name.includes('LLM'))!
    await fc.assert(
      fc.asyncProperty(fc.scheduler(), async s => {
        const { frames } = await play(sc, s)
        const at = frames.findIndex(f => f.label === 'tabs:llm')
        expect(at).toBeGreaterThan(0)
        for (const f of frames.slice(at)) expect(shownOf(f)).toBe(f.model.active)
      }),
      { numRuns: 100 },
    )
  })

  it('⑤ if another tab (C) is chosen before the revert reaches the shell, C stays — the end screen = the model active', async () => {
    const sc: Scenario = {
      name: 'pick C during revert',
      effect: 'none',
      out: true,
      dragEnd: { reply: { type: 'canceled' }, done: 'canceled' },
      arrivals: [tabsOf('tabs:pick-c', { tabs: ['A', 'B', 'C'], active: 'C' }, true)],
      releases: true,
      reverts: 0,
    }
    let raced = 0
    await fc.assert(
      fc.asyncProperty(fc.scheduler(), async s => {
        const { h, frames } = await play(sc, s)
        const last = frames[frames.length - 1]
        expect(last.model.active).toBe('C')
        expect(last.snap.override.phase).toBe('idle')
        expect(h.switches.length).toBeLessThanOrEqual(1)
        if (h.switches.length === 1 && !frames.some(f => f.label === 'tabs:revert')) raced++
      }),
      { numRuns: 200 },
    )
    expect(raced).toBeGreaterThan(0)
  })
})
