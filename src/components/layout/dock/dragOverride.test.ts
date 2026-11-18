// TRD S21-slot-docking §8-2 `dragOverride` (reducer table — triggers ① ② ③ ④ · no deadline · revert) · §8-3 source window order test (fast-check `fc.scheduler()` mixes `done` · invoke responses · `window:tabs-updated` · `layout:updated` · dragend in any order).

import fc from 'fast-check'
import { describe, expect, it } from 'vitest'

import type { DropOutcome } from '../../../../src-tauri/bindings/DropOutcome'
import type { EndEffect } from '../../../../src-tauri/bindings/EndEffect'
import {
  IDLE,
  type OverrideEvent,
  type OverrideState,
  type Revert,
  type WindowModel,
  shownTab,
  stepOverride,
  stripHeld,
} from './dragOverride'

const WIN = 'main'
const DRAG = 'drag-1'
/** Active A before the press · the press made B active (R2). */
const AT_START: WindowModel = { tabs: ['A', 'B', 'C'], active: 'B' }

/** Feed the events in turn — the last state and the list of `to` of the reverts issued. */
function run(events: Array<[OverrideEvent, WindowModel]>, from: OverrideState = IDLE) {
  let state = from
  const reverts: string[] = []
  for (const [ev, model] of events) {
    const out = stepOverride(state, ev, model)
    state = out.state
    if (out.revert !== null) reverts.push(out.revert.to)
  }
  return { state, reverts }
}

const start = (initial: string | null = 'A', view = 'B'): [OverrideEvent, WindowModel] => [
  { type: 'start', window: WIN, drag: DRAG, view, initial },
  AT_START,
]
const out = (m: WindowModel = AT_START): [OverrideEvent, WindowModel] => [{ type: 'zone', out: true }, m]
const end = (effect: EndEffect, local = false, m: WindowModel = AT_START): [OverrideEvent, WindowModel] => [
  { type: 'end', effect, local },
  m,
]
const done = (outcome: DropOutcome, m: WindowModel = AT_START, drag = DRAG): [OverrideEvent, WindowModel] => [
  { type: 'done', drag, outcome },
  m,
]
const tabs = (m: WindowModel): [OverrideEvent, WindowModel] => [{ type: 'tabs' }, m]

describe('dragOverride — while dragging', () => {
  it('outside the zone the remaining tab (= initial ≠ view) · when it comes back the dragged tab', () => {
    let s = run([start()]).state
    expect(shownTab(s, AT_START)).toBe('B')
    s = run([out()], s).state
    expect(shownTab(s, AT_START)).toBe('A')
    s = run([[{ type: 'zone', out: false }, AT_START]], s).state
    expect(shownTab(s, AT_START)).toBe('B')
  })

  it('initial = view · if none, the remaining tab = succession (right → left)', () => {
    expect(shownTab(run([start('B'), out()]).state, AT_START)).toBe('C')
    expect(shownTab(run([start(null), out()]).state, AT_START)).toBe('C')
    const lastTab: WindowModel = { tabs: ['A', 'B', 'C'], active: 'C' }
    const s = run([[{ type: 'start', window: WIN, drag: DRAG, view: 'C', initial: 'C' }, lastTab], out(lastTab)]).state
    expect(shownTab(s, lastTab)).toBe('B')
  })

  it('with only one tab there is no swapped drawing', () => {
    const one: WindowModel = { tabs: ['P'], active: 'P' }
    const s = run([[{ type: 'start', window: WIN, drag: DRAG, view: 'P', initial: 'P' }, one], out(one)]).state
    expect(shownTab(s, one)).toBe('P')
  })

  it('if the remaining tab was closed in the meantime it draws the model active', () => {
    const s = run([start(), out()]).state
    expect(shownTab(s, { tabs: ['B', 'C'], active: 'B' })).toBe('B')
  })

  it('draws the order-move look while dragging · when the order changes it is reaped in that frame', () => {
    const s = run([start()]).state
    expect(stripHeld(s, AT_START)).toBe(true)
    expect(stripHeld(s, { tabs: ['B', 'A', 'C'], active: 'B' })).toBe(false)
    expect(stripHeld(IDLE, AT_START)).toBe(false)
  })
})

describe('dragOverride — release triggers (no deadline)', () => {
  it('① done{rejected} · own invoke error → release (no revert)', () => {
    expect(run([start(), out(), end('none'), done('rejected')])).toEqual({ state: IDLE, reverts: [] })
    expect(run([start(), out(), end('none'), [{ type: 'sendFailed', drag: DRAG }, AT_START]])).toEqual({
      state: IDLE,
      reverts: [],
    })
  })

  it('① done{applied} releases after the tab list differs from the time of dragstart — at whichever of the two arrives later', () => {
    const moved: WindowModel = { tabs: ['A', 'C'], active: 'A' }
    // done first → still unchanged → released on the list change.
    let s = run([start(), out(), end('move'), done('applied')]).state
    expect(s.phase).toBe('ended')
    expect(shownTab(s, AT_START)).toBe('A')
    expect(run([tabs(moved)], s).state).toBe(IDLE)
    // List first (the dragged tab leaves) → released on the spot.
    s = run([start(), out(), end('move'), tabs(moved)]).state
    expect(s).toBe(IDLE)
    // Same-window reorder — the dragged tab stays active and only the order changes → released only when done{applied} arrives.
    const reordered: WindowModel = { tabs: ['B', 'A', 'C'], active: 'B' }
    s = run([start(), end('move'), tabs(reordered)]).state
    expect(s.phase).toBe('ended')
    expect(run([done('applied', reordered)], s).state).toBe(IDLE)
  })

  it('① done{applied} — a window with only one tab (a moved popout) releases even if the list is unchanged', () => {
    const one: WindowModel = { tabs: ['P'], active: 'P' }
    const s = run([
      [{ type: 'start', window: WIN, drag: DRAG, view: 'P', initial: 'P' }, one],
      out(one),
      end('none', false, one),
    ]).state
    expect(run([done('applied', one)], s).state).toBe(IDLE)
  })

  it('① done · error of another drag is not listened to', () => {
    const s = run([start(), out(), end('none')]).state
    expect(run([done('canceled', AT_START, 'drag-0')], s).state).toBe(s)
    expect(run([[{ type: 'sendFailed', drag: 'drag-0' }, AT_START]], s).state).toBe(s)
  })

  it('if own dragend is move, done{ignored} · done{canceled} neither release nor revert', () => {
    for (const outcome of ['ignored', 'canceled'] as const) {
      const r = run([start(), out(), end('move'), done(outcome)])
      expect(r.state.phase).toBe('ended')
      expect(r.reverts).toEqual([])
      expect(shownTab(r.state, AT_START)).toBe('A')
    }
  })

  it('a done{ignored} arriving before dragend (the receiving window\'s Drop — no-cursor) is the same as move', () => {
    const r = run([start(), out(), done('ignored'), end('move')])
    expect(r.state.phase).toBe('ended')
    expect(r.reverts).toEqual([])
  })

  it('② a tabs-updated after dragend that does not keep the dragged tab active → release · a notice with the dragged tab active does not release', () => {
    let s = run([start(), out(), end('move')]).state
    s = run([tabs(AT_START)], s).state
    expect(s.phase).toBe('ended')
    s = run([tabs({ tabs: ['A', 'B', 'C'], active: 'C' })], s).state
    expect(s).toBe(IDLE)
  })

  it('② is not before dragend — even if the LLM changes the active while dragging the swapped drawing stays', () => {
    const s = run([start(), out(), tabs({ tabs: ['A', 'B', 'C'], active: 'C' })]).state
    expect(s.phase).toBe('live')
  })

  it('if the dragged tab leaves that window (including a close while dragging) it releases at any time', () => {
    expect(run([start(), out(), tabs({ tabs: ['A', 'C'], active: 'C' })]).state).toBe(IDLE)
  })

  it('③ the next pointerdown · dragstart releases (including a drag that lost dragend)', () => {
    expect(run([start(), out(), [{ type: 'pointerdown' }, AT_START]]).state).toBe(IDLE)
    const next = run([start(), out(), [{ type: 'start', window: WIN, drag: 'drag-2', view: 'C', initial: 'B' }, AT_START]]).state
    expect(next.phase === 'live' && next.drag).toBe('drag-2')
    expect(next.phase === 'live' && next.out).toBe(false)
  })

  it('④ same-webview in-place drop → release right at dragend · the following done{ignored} is a no-op', () => {
    const r = run([start(), end('move', true), done('ignored')])
    expect(r).toEqual({ state: IDLE, reverts: [] })
    expect(stripHeld(r.state, AT_START)).toBe(false)
  })

  it('④ a drag the engine collapsed (dragend in the same task as dragstart · not sent) → release at once · no revert', () => {
    const r = run([start(), end('none', true)])
    expect(r).toEqual({ state: IDLE, reverts: [] })
    expect(shownTab(r.state, AT_START)).toBe('B')
  })

  it('the zone after dragend is not listened to', () => {
    const s = run([start(), out(), end('none')]).state
    expect(run([[{ type: 'zone', out: false }, AT_START]], s).state).toBe(s)
  })
})

describe('dragOverride — restoring the original selection (§4-6)', () => {
  it('own dragend none + done{canceled|ignored} → switch_tab(initial) once · draws initial until it lands', () => {
    for (const outcome of ['canceled', 'ignored'] as const) {
      const r = run([start(), out(), end('none'), done(outcome)])
      expect(r.reverts).toEqual(['A'])
      expect(r.state).toEqual({ phase: 'reverting', drag: DRAG, view: 'B', to: 'A' })
      // The model still has the pressed tab (B) active — even so, draw initial (no flash).
      expect(shownTab(r.state, AT_START)).toBe('A')
      expect(stripHeld(r.state, AT_START)).toBe(false)
    }
  })

  it('restore = switch_tab_if(source window, initial, expect = dragged tab)', () => {
    let state: OverrideState = IDLE
    let revert: Revert | null = null
    for (const [ev, model] of [start(), out(), end('none'), done('canceled')]) {
      ;({ state, revert } = stepOverride(state, ev, model))
    }
    expect(revert).toEqual({ window: WIN, to: 'A', expect: 'B' })
  })

  it('even if cancelled inside the zone, initial is drawn while waiting for the restore', () => {
    const r = run([start(), end('none'), done('canceled')])
    expect(shownTab(r.state, AT_START)).toBe('A')
  })

  it('waiting for the restore — release when the active reaches initial · not by a notice with the dragged tab active · done · zone', () => {
    const s = run([start(), out(), end('none'), done('canceled')]).state
    expect(run([tabs(AT_START)], s).state).toBe(s)
    expect(run([done('applied'), done('rejected'), [{ type: 'sendFailed', drag: DRAG }, AT_START]], s).state).toBe(s)
    expect(run([[{ type: 'zone', out: false }, AT_START]], s).state).toBe(s)
    const back: WindowModel = { tabs: ['A', 'B', 'C'], active: 'A' }
    expect(run([tabs(back)], s)).toEqual({ state: IDLE, reverts: [] })
  })

  it('waiting for the restore — nets: LLM changes to another tab (②) · the dragged tab leaves · pointerdown (③) · next dragstart', () => {
    const s = run([start(), out(), end('none'), done('ignored')]).state
    const llm: WindowModel = { tabs: ['A', 'B', 'C'], active: 'C' }
    expect(run([tabs(llm)], s).state).toBe(IDLE)
    expect(shownTab(IDLE, llm)).toBe('C')
    expect(run([tabs({ tabs: ['A', 'C'], active: 'A' })], s).state).toBe(IDLE)
    expect(run([[{ type: 'pointerdown' }, AT_START]], s).state).toBe(IDLE)
    expect(run([[{ type: 'start', window: WIN, drag: 'drag-2', view: 'C', initial: 'B' }, AT_START]], s).state.phase).toBe('live')
  })

  it('waiting for the restore — on an error of that restore, release to the model active · errors of someone else\'s drag · error notices during the drag are no-ops', () => {
    const s = run([start(), out(), end('none'), done('canceled')]).state
    expect(shownTab(s, AT_START)).toBe('A')
    expect(run([[{ type: 'revertFailed', drag: 'drag-0' }, AT_START]], s).state).toBe(s)
    const failed = run([[{ type: 'revertFailed', drag: DRAG }, AT_START]], s)
    expect(failed).toEqual({ state: IDLE, reverts: [] })
    expect(shownTab(failed.state, AT_START)).toBe('B')
    const live = run([start(), out()]).state
    expect(run([[{ type: 'revertFailed', drag: DRAG }, AT_START]], live).state).toBe(live)
  })

  it('if the tab to restore was closed in the meantime it draws the model active', () => {
    const s = run([start(), out(), end('none'), done('canceled')]).state
    expect(shownTab(s, { tabs: ['B', 'C'], active: 'B' })).toBe('B')
  })

  it('skips — no initial · initial = view · not a tab of that window · that window\'s active is already not view · rejected · applied', () => {
    expect(run([start(null), end('none'), done('canceled')]).reverts).toEqual([])
    expect(run([start('B'), end('none'), done('canceled')]).reverts).toEqual([])
    expect(run([start('Z'), end('none'), done('canceled')]).reverts).toEqual([])
    const llm: WindowModel = { tabs: ['A', 'B', 'C'], active: 'C' }
    expect(run([start(), end('none'), done('canceled', llm)]).reverts).toEqual([])
    expect(run([start(), end('none'), done('rejected')]).reverts).toEqual([])
    expect(run([start(), end('none'), done('applied', { tabs: ['A', 'C'], active: 'A' })]).reverts).toEqual([])
  })

  it('at most once — a second done of the same drag is a no-op', () => {
    expect(run([start(), end('none'), done('ignored'), done('canceled'), done('applied')]).reverts).toEqual(['A'])
    const back: WindowModel = { tabs: ['A', 'B', 'C'], active: 'A' }
    expect(run([start(), end('none'), done('ignored'), tabs(back), done('canceled', back)]).reverts).toEqual(['A'])
  })
})

// ── §8-3 order test ──────────────────────────────────────────────────────────────

/** One thing that reaches the source window. `after` = what arises only after the source dragend (the result of the `DragEnd` sent by that dragend). */
interface Arrival {
  label: string
  event: OverrideEvent | null
  /** When this arrives, that window's store becomes this value (`window:tabs-updated`). */
  model?: WindowModel
  after?: boolean
}

interface Scenario {
  name: string
  initial: string | null
  /** Drop location — whether the cursor was outside its own tab bar zone. */
  out: boolean
  effect: EndEffect
  local?: boolean
  arrivals: Arrival[]
  /** Whether the end state must be idle (the branch that is released without the net). */
  releases: boolean
  /** The dragged tab leaves that window — after the swap redraw, the dragged tab is not drawn for even one frame (②). */
  leaves?: boolean
  /** Same-window reorder — the frame that withdraws the transform = the frame where the model order changes (④). */
  reorder?: boolean
  /** Revert count (exact). `undefined` = check only ≤ 1. */
  reverts?: number
  /** The revert's `switch_tab_if` is rejected or lost so that notice does not arrive. */
  revertLost?: boolean
  /** After the swap redraw, the dragged tab (B) is not drawn for even one frame — no revert flash. */
  noFlash?: boolean
}

const doneOf = (outcome: DropOutcome, after = false): Arrival => ({
  label: `done:${outcome}${after ? '(DragEnd)' : '(Drop)'}`,
  event: { type: 'done', drag: DRAG, outcome },
  after,
})
const tabsOf = (label: string, model: WindowModel, after = false): Arrival => ({ label, event: { type: 'tabs' }, model, after })
/** Noise that does not change state — `layout:updated` · invoke response (not an error). */
const noise = (label: string, after = false): Arrival => ({ label, event: null, after })

const MOVED_OUT: WindowModel = { tabs: ['A', 'C'], active: 'A' }

const SCENARIOS: Scenario[] = [
  {
    name: 'dropped on another window (Drop applied · DragEnd ignored)',
    initial: 'A',
    out: true,
    effect: 'move',
    arrivals: [
      tabsOf('tabs:moved-out', MOVED_OUT),
      noise('layout:updated'),
      doneOf('applied'),
      noise('reply:drop'),
      doneOf('ignored', true),
      noise('reply:drag-end', true),
    ],
    releases: true,
    leaves: true,
    reverts: 0,
  },
  {
    name: 'dropped on another window — drags an already active tab (remaining tab = succession)',
    initial: 'B',
    out: true,
    effect: 'move',
    arrivals: [tabsOf('tabs:moved-out', { tabs: ['A', 'C'], active: 'C' }), doneOf('applied'), doneOf('ignored', true)],
    releases: true,
    leaves: true,
    reverts: 0,
  },
  {
    name: 'same-window seam (attached to remaining tab A)',
    initial: 'A',
    out: true,
    effect: 'move',
    arrivals: [tabsOf('tabs:docked', MOVED_OUT), noise('layout:updated'), doneOf('applied'), doneOf('ignored', true)],
    releases: true,
    leaves: true,
    reverts: 0,
  },
  {
    name: 'new window outside the window (DragEnd applied)',
    initial: 'A',
    out: true,
    effect: 'none',
    arrivals: [tabsOf('tabs:placed', MOVED_OUT, true), doneOf('applied', true), noise('reply:drag-end', true)],
    releases: true,
    leaves: true,
    reverts: 0,
  },
  {
    name: 'same-window reorder (own tab bar)',
    initial: 'A',
    out: false,
    effect: 'move',
    arrivals: [
      tabsOf('tabs:reordered', { tabs: ['B', 'A', 'C'], active: 'B' }),
      noise('layout:updated'),
      doneOf('applied'),
      doneOf('ignored', true),
    ],
    releases: true,
    reorder: true,
    reverts: 0,
  },
  {
    name: 'Esc cancel → revert (lands late — no flash)',
    initial: 'A',
    out: true,
    effect: 'none',
    arrivals: [doneOf('canceled', true), noise('reply:drag-end', true), noise('layout:updated', true)],
    releases: true,
    reverts: 1,
    noFlash: true,
  },
  {
    name: 'lost drop (ignored) → the revert never lands — the pointerdown net releases it',
    initial: 'A',
    out: true,
    effect: 'none',
    arrivals: [
      doneOf('ignored', true),
      noise('reply:drag-end', true),
      { label: 'pointerdown', event: { type: 'pointerdown' }, after: true },
    ],
    releases: true,
    revertLost: true,
  },
  {
    name: 'lost drop (ignored) — meanwhile the LLM sets the active to C',
    initial: 'A',
    out: true,
    effect: 'none',
    arrivals: [doneOf('ignored', true), tabsOf('tabs:llm-switch', { tabs: ['A', 'B', 'C'], active: 'C' })],
    releases: true,
  },
  {
    name: 'Drop refused (tab in place · the pressed tab stays active)',
    initial: 'A',
    out: true,
    effect: 'move',
    arrivals: [doneOf('rejected'), doneOf('ignored', true), noise('reply:drop')],
    releases: true,
    reverts: 0,
  },
  {
    name: 'Drop ignored (no-cursor) — the net (pointerdown) releases it',
    initial: 'A',
    out: true,
    effect: 'move',
    arrivals: [
      doneOf('ignored'),
      doneOf('ignored', true),
      { label: 'pointerdown', event: { type: 'pointerdown' }, after: true },
    ],
    releases: true,
    reverts: 0,
  },
  {
    name: 'same-webview in-place drop',
    initial: 'A',
    out: false,
    effect: 'move',
    local: true,
    arrivals: [doneOf('ignored', true), noise('reply:drag-end', true)],
    releases: true,
    reverts: 0,
  },
  {
    name: 'own DragEnd invoke error',
    initial: 'A',
    out: true,
    effect: 'none',
    arrivals: [{ label: 'reply:error', event: { type: 'sendFailed', drag: DRAG }, after: true }],
    releases: true,
    reverts: 0,
  },
]

interface Frame {
  label: string
  state: OverrideState
  model: WindowModel
}

/** Run one scenario in the order the scheduler picks — record state · store at every frame (after feeding one event). */
async function play(sc: Scenario, s: fc.Scheduler) {
  let model = AT_START
  let state = run([[{ type: 'start', window: WIN, drag: DRAG, view: 'B', initial: sc.initial }, AT_START]]).state
  if (sc.out) state = run([out()], state).state
  const frames: Frame[] = [{ label: 'start', state, model }]
  const reverts: string[] = []
  let ended = false

  const deliver = (a: Arrival) => {
    if (a.model !== undefined) model = a.model
    if (a.event !== null) {
      const step = stepOverride(state, a.event, model)
      state = step.state
      const revert = step.revert
      if (revert !== null) {
        reverts.push(revert.to)
        // The revert's `switch_tab_if` round trip — when it reaches the shell, if that window's active is still `expect`, change it to `to` and a notice follows.
        //   If another tab was chosen in the meantime, the shell changes nothing and does not notify.
        if (!sc.revertLost) {
          void s.schedule(Promise.resolve(), 'switch_tab_if').then(() => {
            if (model.active !== revert.expect || !model.tabs.includes(revert.to)) return
            deliver(tabsOf('tabs:revert', { tabs: model.tabs, active: revert.to }))
          })
        }
      }
    }
    frames.push({ label: a.label, state, model })
  }

  const scheduleAll = (after: boolean) => {
    for (const a of sc.arrivals.filter(x => (x.after ?? false) === after)) {
      void s.schedule(Promise.resolve(), a.label).then(() => deliver(a))
    }
  }

  scheduleAll(false)
  void s.schedule(Promise.resolve(), 'dragend').then(() => {
    ended = true
    deliver({ label: 'dragend', event: { type: 'end', effect: sc.effect, local: sc.local ?? false } })
    scheduleAll(true)
  })
  await s.waitIdle()
  expect(ended).toBe(true)
  return { frames, reverts, state, model }
}

describe('dragOverride — order test (fc.scheduler)', () => {
  for (const sc of SCENARIOS) {
    it(sc.name, async () => {
      await fc.assert(
        fc.asyncProperty(fc.scheduler(), async s => {
          const { frames, reverts, state, model } = await play(sc, s)
          const where = () => `${s.report().map(r => r.label).join(' → ')}`

          // ① The branch that must be released was released at the end · ③ the tab drawn after release = the model active.
          if (sc.releases) expect(state, where()).toBe(IDLE)
          expect(shownTab(state, model), where()).toBe(model.active)

          // ② The branch where the dragged tab leaves that window — after the swap redraw, the dragged tab is not drawn for even one frame (no deadline flip).
          if (sc.leaves) {
            for (const f of frames) expect(shownTab(f.state, f.model), `${f.label} @ ${where()}`).not.toBe('B')
          }

          // ④ Same-window reorder — the transform stays until the frame where the model order changes, and is withdrawn on that frame.
          if (sc.reorder) {
            for (const f of frames) {
              const sameOrder = f.model.tabs.join() === AT_START.tabs.join()
              expect(stripHeld(f.state, f.model), `${f.label} @ ${where()}`).toBe(sameOrder)
            }
          }

          // The dragged tab is not drawn while waiting for the revert — after the net (pointerdown) releases, the model active (the pressed tab) is it, so it is excluded.
          if (sc.noFlash) {
            for (const f of frames) {
              if (f.label === 'pointerdown') break
              expect(shownTab(f.state, f.model), `${f.label} @ ${where()}`).not.toBe('B')
            }
          }
          // Even if the revert does not arrive, the net (pointerdown) releases — if done came before that, a revert was issued and initial was drawn until then.
          if (sc.revertLost) {
            const at = (label: string) => frames.findIndex(f => f.label === label)
            const doneFirst = at('done:ignored(DragEnd)') < at('pointerdown')
            expect(reverts.length, where()).toBe(doneFirst ? 1 : 0)
            for (const f of frames.slice(0, at('pointerdown'))) {
              expect(shownTab(f.state, f.model), `${f.label} @ ${where()}`).not.toBe('B')
            }
          }

          // ⑤ At most one revert · the exact count per branch.
          expect(reverts.length, where()).toBeLessThanOrEqual(1)
          if (sc.reverts !== undefined) expect(reverts.length, where()).toBe(sc.reverts)
        }),
        { numRuns: 200 },
      )
    })
  }

  it('⑤ if the LLM\'s active change arrives before done it does not revert · if later once — the final screen is always the model active', async () => {
    const sc = SCENARIOS.find(x => x.name.includes('meanwhile the LLM'))!
    await fc.assert(
      fc.asyncProperty(fc.scheduler(), async s => {
        const { reverts, state, model, frames } = await play(sc, s)
        const llmFirst =
          frames.findIndex(f => f.label === 'tabs:llm-switch') < frames.findIndex(f => f.label.startsWith('done:'))
        expect(reverts.length).toBe(llmFirst ? 0 : 1)
        expect(state).toBe(IDLE)
        expect(shownTab(state, model)).toBe(model.active)
      }),
      { numRuns: 200 },
    )
  })

  it('⑤ if another tab (C) is chosen before the revert reaches the shell, C stays — the revert does not overwrite', async () => {
    const pick: WindowModel = { tabs: ['A', 'B', 'C'], active: 'C' }
    const sc: Scenario = {
      name: 'pick C during revert',
      initial: 'A',
      out: true,
      effect: 'none',
      arrivals: [doneOf('canceled', true), tabsOf('tabs:pick-c', pick, true)],
      releases: true,
    }
    let raced = 0
    await fc.assert(
      fc.asyncProperty(fc.scheduler(), async s => {
        const { reverts, state, model, frames } = await play(sc, s)
        const where = () => `${s.report().map(r => r.label).join(' → ')}`
        expect(state, where()).toBe(IDLE)
        expect(model.active, where()).toBe('C')
        expect(shownTab(state, model), where()).toBe('C')
        // The branch where a revert was issued and C arrived before that round trip — with the old unconditional `switch_tab` it would have been overwritten with A.
        if (reverts.length === 1 && !frames.some(f => f.label === 'tabs:revert')) raced++
      }),
      { numRuns: 200 },
    )
    expect(raced).toBeGreaterThan(0)
  })

  it('⑥ if a tabs-updated that does not keep the dragged tab active comes after dragend, the tab drawn afterwards = the model active (a late swapped drawing does not overwrite)', async () => {
    const llm: WindowModel = { tabs: ['A', 'B', 'C'], active: 'C' }
    const sc: Scenario = {
      name: 'LLM switch after dragend',
      initial: 'A',
      out: true,
      effect: 'move',
      // The receiving window's Drop was lost — done is only the ignoring of DragEnd.
      arrivals: [doneOf('ignored', true), tabsOf('tabs:llm-switch', llm, true)],
      releases: true,
      reverts: 0,
    }
    await fc.assert(
      fc.asyncProperty(fc.scheduler(), async s => {
        const { frames } = await play(sc, s)
        const at = frames.findIndex(f => f.label === 'tabs:llm-switch')
        for (const f of frames.slice(at)) expect(shownTab(f.state, f.model)).toBe(f.model.active)
      }),
      { numRuns: 100 },
    )
  })
})
