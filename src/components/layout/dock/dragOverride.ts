// TRD S21-slot-docking §2-4 · §4-6: the source window's "swap redraw" — a pure reducer deciding when, with the cursor outside its own tab bar zone during a drag, the web shows the remaining tab instead of the dragged tab (R15; the model is unchanged), and when that display · the tab bar's order-move look (transform) is withdrawn after the drop. It also decides whether to issue the original-selection revert (`switch_tab_if(initial, expect = dragged tab)`); issuing is the caller's (the source hook).
// ★No deadline★ — four release triggers: ① that drag's `tab_drop:done` (immediately if its own invoke errored) ② after dragend, a `window:tabs-updated` that does not leave the dragged tab active ③ the next pointerdown · dragstart of that window ④ a source dragend the shell does not decide (drop in place in the same webview · a drag the engine collapsed). A deadline would show the dragged tab for a frame or two before `Drop` is applied. While waiting for the revert, the revert's error also releases (falls back to the model active — TRD §4-6).
// Caller's promise: feed events with the tab state (`WindowModel`) **after** it was applied to that window's store; feed dragend **before** sending that drag's `DragEnd` invoke, so a `done` that arrived before dragend is told apart as the receiving window's `Drop` share.

import type { DropOutcome } from '../../../../src-tauri/bindings/DropOutcome'
import type { EndEffect } from '../../../../src-tauri/bindings/EndEffect'
import { succession } from '../tabStrip/succession'

/** That window's tab state — the tab id list of store `windows[label]` and the active (`null` if unknown). */
export interface WindowModel {
  tabs: readonly string[]
  active: string | null
}

/** One drag held by the source window. `idle` = no swap redraw (drawn tab = model active · transform 0). */
export type OverrideState =
  | { phase: 'idle' }
  /**
   * A revert (`Revert`) was issued and waits for that active to reach the model — meanwhile `to` is drawn (transform 0). Releasing early would draw the model active (still the dragged tab) and flash `to` → dragged tab → `to`. If the revert never arrives, ② · ③ release — when the shell changed nothing because the condition no longer held, the notice from whoever changed the active is ②.
   */
  | { phase: 'reverting'; drag: string; view: string; to: string }
  | {
      /** `live` = dragstart ~ dragend · `ended` = after dragend, waiting for a release trigger. */
      phase: 'live' | 'ended'
      /** Source window label — the window to send the revert to. */
      window: string
      drag: string
      view: string
      initial: string | null
      /** The remaining tab to be shown outside the zone — the same as the `successor` the shell picks at drop (TRD §3-3). `null` = only one tab (no swap redraw). */
      remaining: string | null
      /** That window's tab order at dragstart — the order-move look is drawn only while this order stays as is. */
      startTabs: readonly string[]
      /** The cursor is outside its own tab bar zone (including outside the window). After dragend it stays at the value at that time. */
      out: boolean
      /** `dropEffect` of its own dragend — `null` during `live`. */
      effect: EndEffect | null
      /** Received this drag's `done{applied}` — release if the tab list differs from dragstart. */
      applied: boolean
    }

export type OverrideEvent =
  /** Source dragstart — discards the previous drag (③). `window` = source window label · `initial` = that window's model active before the press. */
  | { type: 'start'; window: string; drag: string; view: string; initial: string | null }
  /** The cursor entered (`out: false`) · left (`out: true`) its own tab bar zone — heard only until dragend. */
  | { type: 'zone'; out: boolean }
  /**
   * Source dragend. `local` = this drag's end is not sent to the shell, so it is released here (④, no revert — the pressed tab stays active, like a click): a same-place same-window drop was not sent, or dragend in the same task as dragstart = the engine could not open an OS drag and collapsed it (old HTML5 drag: 0–1 ms after dragstart).
   */
  | { type: 'end'; effect: EndEffect; local: boolean }
  | { type: 'done'; drag: string; outcome: DropOutcome }
  /** Its own `tab_drop` invoke ended with an error. */
  | { type: 'sendFailed'; drag: string }
  /** That drag's revert (`switch_tab_if`) ended with an error — the awaited active does not arrive, so it falls back to the model active (TRD §4-6). */
  | { type: 'revertFailed'; drag: string }
  /** That window's `window:tabs-updated` was applied to the store. */
  | { type: 'tabs' }
  /** That window's pointerdown. */
  | { type: 'pointerdown' }

/**
 * Revert to the original selection (R12 · R15), issued as `switch_tab_if(window, to, expect)`: the shell changes to `to` only when that window's active is still `expect` (the dragged tab the press made active), so a tab chosen by the user · LLM before the revert arrives is not overwritten.
 */
export interface Revert {
  window: string
  to: string
  expect: string
}

export interface OverrideStep {
  state: OverrideState
  /** The revert to issue — at most once per drag. */
  revert: Revert | null
}

export const IDLE: OverrideState = { phase: 'idle' }

const released = (): OverrideStep => ({ state: IDLE, revert: null })

export function stepOverride(state: OverrideState, event: OverrideEvent, model: WindowModel): OverrideStep {
  if (event.type === 'start') {
    const { window, drag, view, initial } = event
    const remaining =
      initial !== null && initial !== view && model.tabs.includes(initial) ? initial : succession(model.tabs, view)
    return {
      state: {
        phase: 'live',
        window,
        drag,
        view,
        initial,
        remaining,
        startTabs: [...model.tabs],
        out: false,
        effect: null,
        applied: false,
      },
      revert: null,
    }
  }
  if (state.phase === 'idle') return { state, revert: null }
  const same: OverrideStep = { state, revert: null }

  if (state.phase === 'reverting') {
    if (event.type === 'pointerdown') return released()
    if (event.type === 'revertFailed' && event.drag === state.drag) return released()
    // The revert arrived (active = `to`) · the LLM changed to another tab · the dragged tab left — all three mean the active is no longer the dragged tab (②).
    if (event.type === 'tabs' && model.active !== state.view) return released()
    return same
  }

  switch (event.type) {
    case 'zone':
      return state.phase === 'live' && state.out !== event.out ? { state: { ...state, out: event.out }, revert: null } : same
    case 'end':
      if (state.phase !== 'live') return same
      return event.local ? released() : { state: { ...state, phase: 'ended', effect: event.effect }, revert: null }
    case 'pointerdown':
      return released()
    case 'sendFailed':
      return event.drag === state.drag ? released() : same
    case 'revertFailed':
      return same
    case 'done': {
      if (event.drag !== state.drag) return same
      if (event.outcome === 'rejected') return released()
      if (event.outcome === 'applied') {
        // A window with only one tab has no swap redraw, so there is nothing to wait for in a list change (a moved popout keeps the same list).
        const next = { ...state, applied: true }
        return state.remaining === null || orderChanged(next, model) ? released() : { state: next, revert: null }
      }
      // Cancel · ignore. What arrived before dragend belongs to the receiving window's `Drop` — the source dragend is `move`; then the receiving side's `Drop` result follows (or none for same window, same place), and releasing now would show the dragged tab before `Drop` is applied, and reverting would differ from a click.
      if (state.phase === 'live' || state.effect === 'move') return same
      const to = revertTarget(state, model)
      return to === null
        ? released()
        : {
            state: { phase: 'reverting', drag: state.drag, view: state.view, to },
            revert: { window: state.window, to, expect: state.view },
          }
    }
    case 'tabs':
      // The dragged tab left that window — release at any time: if moved out the model active is already the remaining tab; if closed during the drag the detached head's dragend never arrives.
      if (!model.tabs.includes(state.view)) return released()
      if (state.applied && orderChanged(state, model)) return released()
      return state.phase === 'ended' && model.active !== state.view ? released() : same
  }
}

/** The tab that window draws now — the remaining tab while swap-redrawing · the tab to revert to while waiting for the revert, otherwise the model active. */
export function shownTab(state: OverrideState, model: WindowModel): string | null {
  if (state.phase === 'reverting') return model.tabs.includes(state.to) ? state.to : model.active
  if (state.phase !== 'idle' && state.out && state.remaining !== null && model.tabs.includes(state.remaining)) {
    return state.remaining
  }
  return model.active
}

/**
 * Whether the order-move look (transform) may be drawn — only while the drag is not released and that window's tab order is still as at dragstart; withdrawn on the frame the model order changes so the dropped tab does not jump (TRD §2-4 · §8-3 ④).
 */
export function stripHeld(state: OverrideState, model: WindowModel): boolean {
  return (state.phase === 'live' || state.phase === 'ended') && sameOrder(state.startTabs, model.tabs)
}

function orderChanged(state: { startTabs: readonly string[] }, model: WindowModel): boolean {
  return !sameOrder(state.startTabs, model.tabs)
}

function sameOrder(a: readonly string[], b: readonly string[]): boolean {
  return a.length === b.length && a.every((t, i) => t === b[i])
}

/**
 * The tab to revert to after cancel · ignore (TRD §4-6) — the active the press's `switch_tab` changed, back to before the press. Skipped when no `initial` · the dragged tab itself · not a tab of that window · that window's active is already not the dragged tab (user · LLM changed it — do not overwrite).
 */
function revertTarget(state: { view: string; initial: string | null }, model: WindowModel): string | null {
  const { view, initial } = state
  if (initial === null || initial === view || !model.tabs.includes(initial) || model.active !== view) return null
  return initial
}
