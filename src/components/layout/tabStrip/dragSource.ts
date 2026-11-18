// TRD S21-slot-docking §2-4 · §4-2 · §4-6: the one place that drives a tab drag from the source webview to its end — swap redraw release · original selection revert · sending at dragend · the own tab bar order-move look. Decisions rest only on the drag id and model state, not on deadline/event arrival order (release · revert rules = the pure reducer `dock/dragOverride.ts`; this feeds it and issues its answer). Knows no DOM — `useTabDragSource` converts head events into values.
// One per webview — the tab bar (order move · active paint) and the window layout (keep-alive visible tab) read the same snapshot.
// ★No shell call during a drag (dragstart ~ dragend)★ (ADR-0296 · ADR-0297) — the shell calls are the `tab_drop` at dragend and one revert after `done` (`switch_tab_if`), both after the drag ended.

import type { EndEffect } from '../../../../src-tauri/bindings/EndEffect'
import type { PointCss } from '../../../../src-tauri/bindings/PointCss'
import type { SizeCss } from '../../../../src-tauri/bindings/SizeCss'
import type { TabDrop } from '../../../../src-tauri/bindings/TabDrop'
import type { TabDropDone } from '../../../../src-tauri/bindings/TabDropDone'
import type { TabDropReply } from '../../../../src-tauri/bindings/TabDropReply'
import {
  IDLE,
  stepOverride,
  type OverrideEvent,
  type OverrideState,
  type Revert,
  type WindowModel,
} from '../dock/dragOverride'
import type { LocalDrag, SourceStripMove } from '../dock/localDrag'
import { moveDue, reorderThreshold } from './reorderGeometry'

/** The outside: `useTabDragSource` fills it with Tauri · the store in production; tests plug in a fake (ADR-0012). */
export interface DragSourcePorts {
  send(drop: TabDrop): Promise<TabDropReply>
  /** The shell changes to `to` only when that window's active is still `expect` (TRD §4-6). */
  switchTabIf(window: string, to: string, expect: string): Promise<boolean>
  local: LocalDrag
  /** Call in the next task — it must not be a microtask (the same-task decision of `DragStart`). */
  nextTask(fn: () => void): void
}

/** What the source computed at dragstart. Coordinates = tab bar coordinates (0 = left of the tab drag area). */
export interface DragStart {
  window: string
  drag: string
  view: string
  /** That window's model active before the press. */
  initial: string | null
  /** Size · cursor position of `DropTo.new_window` (`dragSize.ts`). */
  size: SizeCss | null
  grab: PointCss
  grabOffset: number
  /** Left of the dragged head · cursor x at dragstart — the first decision value on its own tab bar (the pressed point is inside the tab bar zone). */
  left: number
  x: number
}

/** The order-move look inside its own tab bar zone. */
export interface StripMotion {
  view: string
  /** Confirmed position — based on the list without the dragged tab (= the position in the result list after the drop). It changes only past the order threshold (`moveDue`). */
  index: number
  /** Left of the dragged head (tab bar coordinates) — not used if `settled`. */
  left: number
  /** After dragend — the dragged head goes to position `index` instead of the cursor. */
  settled: boolean
}

export interface DragSourceSnapshot {
  /** Window holding a swap redraw — `null` if `override` is `idle`. */
  window: string | null
  override: OverrideState
  /**
   * The order move to draw — exists only after the first decision of that drag (`setSourceStrip`) came inside its own tab bar zone. At the dragstart spot it is `null`, so the tab bar draws the same DOM as at rest (if the dragged head left from under the pressed point inside dragstart, the engine would collapse the drag). The caller still withdraws it with `stripHeld` if the model order differs from dragstart.
   */
  motion: StripMotion | null
}

export interface DragSource {
  getSnapshot(): DragSourceSnapshot
  subscribe(cb: () => void): () => void
  /** Tab state after `window:tabs-updated` (· `list_tabs` pull) was applied to that window's store — once per apply; even an unchanged notice is the release trigger (TRD §2-4 ②). That window's first value is not counted as a notice. */
  model(window: string, m: WindowModel): void
  /** That window's pointerdown (including outside heads). */
  pointerDown(): void
  start(s: DragStart): void
  /** A field of `localDrag` changed — reads the source window tab bar decision (`sourceStrip`). */
  stripChanged(): void
  /** Source head's dragend. `effect` = its `dropEffect` (outside `move` it is `none`). */
  end(view: string, effect: EndEffect): void
  /** The `tab_drop:done` that came to this window. */
  done(d: TabDropDone): void
  /**
   * Whether a head press in that window must send `switch_tab` even when it presses the model active tab (`view`): `true` if a revert with `expect = view` was issued to that window and its model active is still `view`. Otherwise the later-arriving revert would overwrite this press (the shell applies in received order). Returning `true` forgets that revert.
   */
  forceSwitch(window: string, view: string): boolean
}

const EMPTY: WindowModel = { tabs: [], active: null }

/** One drag — the table that decides what dragend sends. */
interface Flight {
  s: DragStart
  /** Same task as dragstart — a dragend inside it means the engine could not open an OS drag and collapsed it. */
  sameTask: boolean
  ended: boolean
}

/** Computation of the order move on its own tab bar (Chrome `DraggingTabsSession` threshold · last move point). */
interface Track {
  /** Confirmed position — `null` = model order (it was outside the zone). */
  index: number | null
  left: number
  /** Cursor x where the position last actually changed. */
  lastMove: number
  /** The first decision on (re-)entering the zone — computed without the threshold. */
  justAttached: boolean
  inZone: boolean
  /** The root's first decision has arrived. */
  tracking: boolean
}

export function createDragSource(ports: DragSourcePorts): DragSource {
  const models = new Map<string, WindowModel>()
  let owner: string | null = null
  let state: OverrideState = IDLE
  let flight: Flight | null = null
  let track: Track | null = null
  let lastStrip: SourceStripMove | null = null
  let snap: DragSourceSnapshot = { window: null, override: IDLE, motion: null }
  const listeners = new Set<() => void>()
  // The last revert issued per window (`forceSwitch`) — withdrawn when that window's model active leaves `expect` (arrived · changed by someone else) or its invoke errors. Held apart from the swap redraw (`reverting`), which any pointerdown of that window releases while the revert is still in flight.
  const reverts = new Map<string, Revert>()

  const modelOf = (w: string | null): WindowModel => (w === null ? EMPTY : (models.get(w) ?? EMPTY))

  function motionOf(): StripMotion | null {
    if (track === null || !track.inZone || !track.tracking) return null
    if (state.phase !== 'live' && state.phase !== 'ended') return null
    const index = track.index ?? modelOf(owner).tabs.indexOf(state.view)
    if (index < 0) return null
    return { view: state.view, index, left: track.left, settled: state.phase === 'ended' }
  }

  function publish(): void {
    const motion = motionOf()
    if (snap.window === owner && snap.override === state && sameMotion(snap.motion, motion)) return
    snap = { window: owner, override: state, motion }
    for (const cb of [...listeners]) cb()
  }

  function dispatch(event: OverrideEvent): void {
    const window = event.type === 'start' ? event.window : owner
    const before = state
    const step = stepOverride(state, event, modelOf(window))
    owner = window
    state = step.state
    if (state.phase === 'idle') {
      owner = null
      track = null
      // Only the released drag's own is cleared — the path where a new drag's dragstart discards the previous drag does not pass here (`start` covers it).
      const out = ports.local.getOutgoing()
      if (out !== null && before.phase !== 'idle' && out.drag === before.drag) ports.local.setOutgoing(null)
    }
    if (step.revert !== null) revert(step.revert)
  }

  function revert(r: Revert): void {
    const drag = state.phase === 'reverting' ? state.drag : null
    reverts.set(r.window, r)
    ports.switchTabIf(r.window, r.to, r.expect).then(
      changed => {
        // Not changed = the active changed in the meantime — that notice (`tabs`) releases it.
        if (!changed) console.debug('[dragSource] switch_tab_if: the active tab changed, not restoring', r)
      },
      err => {
        // A normal race (the tab to revert was closed in the meantime, etc.) — only a log is left and it falls back to the model active.
        console.warn('[dragSource] switch_tab_if failed — falling back to the active tab of the model:', err)
        if (reverts.get(r.window) === r) reverts.delete(r.window)
        if (drag === null) return
        dispatch({ type: 'revertFailed', drag })
        publish()
      },
    )
  }

  function send(f: Flight, effect: EndEffect): void {
    const { s } = f
    const drop: TabDrop = {
      drag: s.drag,
      view: s.view,
      from: s.window,
      initial: s.initial,
      to: { type: 'new_window', by: { type: 'drag_end', effect }, size: s.size, grab: s.grab },
    }
    ports.send(drop).then(
      reply => console.debug('[dragSource] tab_drop(drag_end) reply:', reply),
      err => {
        console.warn('[dragSource] tab_drop(drag_end) failed — releasing the swapped drawing at once:', err)
        dispatch({ type: 'sendFailed', drag: s.drag })
        publish()
      },
    )
  }

  // Chrome order threshold — the confirmed position moves to the decision's position only when `moveDue` is true (`judge` ignores the threshold · TRD §7 U3). The tab bar geometry is the order drawn now (carrying the confirmed position), the same basis as the decision's position.
  function follow(t: Track, view: string, m: SourceStripMove): void {
    const g = ports.local.readStripGeometry()
    const d = g === null ? -1 : g.tabs.indexOf(view)
    if (g !== null && d >= 0 && Number.isInteger(m.index) && m.index >= 0 && m.index < g.bounds.tabs.length) {
      const due = moveDue(t.lastMove, m.x, reorderThreshold(g.bounds, m.index), t.justAttached)
      if (due && m.index !== d) {
        t.index = m.index
        t.lastMove = m.x
      }
    }
    t.justAttached = false
  }

  return {
    getSnapshot: () => snap,

    subscribe(cb) {
      const entry = (): void => cb()
      listeners.add(entry)
      return () => {
        listeners.delete(entry)
      }
    },

    model(window, m) {
      const prev = models.get(window)
      models.set(window, m)
      if (m.active !== reverts.get(window)?.expect) reverts.delete(window)
      if (prev === undefined) return
      if (owner !== window) return
      dispatch({ type: 'tabs' })
      publish()
    },

    pointerDown() {
      if (state.phase === 'idle') return
      dispatch({ type: 'pointerdown' })
      publish()
    },

    start(s) {
      const f: Flight = { s, sameTask: true, ended: false }
      flight = f
      ports.nextTask(() => {
        f.sameTask = false
      })
      dispatch({ type: 'start', window: s.window, drag: s.drag, view: s.view, initial: s.initial })
      track = { index: null, left: s.left, lastMove: s.x, justAttached: true, inZone: true, tracking: false }
      // ★Put the first decision before the outgoing drag mark★ — that mark's notice calls `stripChanged`, and a remaining last decision of the previous drag (`null` etc.) would make the new drag read as "outside the zone".
      const first: SourceStripMove = { index: modelOf(s.window).tabs.indexOf(s.view), left: s.left, x: s.x }
      lastStrip = first
      ports.local.setSourceStrip(first)
      ports.local.setOutgoing({ drag: s.drag, window: s.window, view: s.view, grabOffset: s.grabOffset })
      publish()
    },

    stripChanged() {
      const m = ports.local.getSourceStrip()
      if (m === lastStrip) return
      lastStrip = m
      const f = flight
      const t = track
      if (f === null || f.ended || t === null || state.phase !== 'live' || state.drag !== f.s.drag) return
      if (m === null) {
        t.inZone = false
        t.index = null
        t.justAttached = true
      } else {
        follow(t, f.s.view, m)
        t.inZone = true
        t.left = m.left
        t.tracking = true
      }
      const out = m === null
      if (out !== state.out) dispatch({ type: 'zone', out })
      publish()
    },

    end(view, effect) {
      const f = flight
      if (f === null || f.ended || f.s.view !== view) return
      f.ended = true
      // Neither sends the end to the shell (TRD §2-4 ④ · U2 ⑨): a same-place same-window drop was not sent, and a drag the engine collapsed (old HTML5 drag: dragend(none) in the same task as dragstart) would be read by the shell as a drop at the last point and open a new window. The mark is always taken — left over, the next drag reads it as its own.
      const marked = ports.local.takeLocalDrop(f.s.drag)
      const local = marked || f.sameTask
      // ★Feed the reducer first, then send★ (U2 ⑦) — a `done` that arrived before dragend is split off as the receiving window's `Drop` share.
      dispatch({ type: 'end', effect, local })
      publish()
      if (!local) send(f, effect)
    },

    done(d) {
      if (state.phase === 'idle' || state.drag !== d.drag) return
      dispatch({ type: 'done', drag: d.drag, outcome: d.outcome })
      publish()
    },

    forceSwitch(window, view) {
      if (reverts.get(window)?.expect !== view) return false
      reverts.delete(window)
      return true
    },
  }
}

function sameMotion(a: StripMotion | null, b: StripMotion | null): boolean {
  if (a === null || b === null) return a === b
  return a.view === b.view && a.index === b.index && a.left === b.left && a.settled === b.settled
}
