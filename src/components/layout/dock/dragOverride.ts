// TRD S21-slot-docking §2-4 · §4-6: the source window's 「바꿔 그리기」 (swap redraw) — a pure reducer that decides, when the cursor is outside its own tab bar zone during a drag, the web display
//   showing the remaining tab instead of the dragged tab (R15 · the model is unchanged), and when to withdraw that display · the tab bar's order-move look (transform) after the drop.
//   It also decides here whether to issue the original-selection revert (`switch_tab_if(initial, expect = 끄는 탭)`) — issuing is done by the caller (the source hook).
// ★There is no deadline★ — there are only four triggers for release: ① that drag's `tab_drop:done` (immediately if its own invoke errored) ② after dragend, a `window:tabs-updated` that does not
//   leave the dragged tab active ③ the next pointerdown · dragstart of that window ④ a source dragend whose end the shell does not decide (drop in place in the same webview ·
//   a drag the engine collapsed). Releasing by a deadline would show the dragged tab for a frame or two before `Drop` is applied. While waiting for the revert, that
//   revert's error also releases (falls back to the model active — TRD §4-6).
// Caller's promise: feed events together with the tab state (`WindowModel`) **after** it was applied to that window's store. Feed dragend **before**
//   sending that drag's `DragEnd` invoke — so that a `done` that arrived before dragend can be told apart as the receiving window's `Drop` share.

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
   * A revert (`Revert`) was issued and it waits for that active to reach the model — meanwhile `to` is drawn (transform 0). If released, the model
   * active (still the dragged tab) would be drawn immediately and flash `to` → dragged tab → `to` (main decision 2026-10-20). If the revert never arrives, ② · ③ release it
   * — if the shell did not change anything because the condition no longer held, the notice from whoever changed the active in the meantime is ②.
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
   * Source dragend. `local` = this drag's end is not sent to the shell, so it is released here (④ · no revert — the pressed tab stays active, same as a click):
   * this webview's drop was at the same place in the same window so it was not sent · a dragend that occurred in the same task as dragstart = the engine could not open an OS drag and
   * collapsed it (measured on the old HTML5 drag — 0–1 ms after dragstart · main decision 2026-10-20 — nothing is sent to the shell).
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
 * Revert to the original selection (R12 · R15) — issued as `switch_tab_if(window, to, expect)`: the shell changes to `to` only when that window's active is still `expect` (the dragged tab
 * that the press made active). So a tab chosen by the user · LLM before the revert arrives is not overwritten (even if the web store is late).
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
      // Cancel · ignore. What arrived before dragend belongs to the receiving window's `Drop` — the source dragend is `move`. If `move`, the receiving side's `Drop` result
      //   follows or that window did not send it (same window, same place) — releasing on this would show the dragged tab before `Drop` is applied, and reverting would
      //   differ from a click.
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
      // The dragged tab left that window — release at any time (main decision 2026-10-20). If it moved out, the model active is already the remaining tab, and if it was closed
      //   during the drag, the dragend of the detached head never arrives.
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
 * Whether the tab bar's order-move look (transform) may be drawn — only while the drag is not released and that window's tab order is still as at dragstart. It must be withdrawn on the frame the model order
 * changes so the dropped tab does not jump (TRD §2-4 · §8-3 ④).
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
 * The tab to revert after cancel · ignore (TRD §4-6) — the active changed by the press's `switch_tab` back to what it was before the press. Skipped = no `initial` · the dragged tab itself ·
 * not a tab of that window · that window's active is already not the dragged tab (the user · LLM changed it in the meantime — do not overwrite).
 */
function revertTarget(state: { view: string; initial: string | null }, model: WindowModel): string | null {
  const { view, initial } = state
  if (initial === null || initial === view || !model.tabs.includes(initial) || model.active !== view) return null
  return initial
}
