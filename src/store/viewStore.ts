// viewStore — the frontend mirror of the layout authority (src-tauri ViewManager, ADR-0035/0057) + control surface (§5).
//
// ★Authority = backend★: this store does not mutate the layout directly. Actions only call the matching invoke; state updates only by listening to the layout:updated / window:tabs-updated the backend emits
// (no optimistic updates), so a human click and the LLM (cdp eval → window.__engramCmd) pass the same invoke→emit loop (§5).
//
// ★The layout does not go through the agentClient/ProtocolClient seam (ADR-0011)★ — that is for agent commands (daemon authority); the layout is src-tauri authority (ADR-0035), so @tauri-apps/api invoke/listen are called directly.
//
// Tab ownership model (ADR-0057): one window (label) owns a tab list (several Views) and switches within it. The frontend mirrors per-window tab state as `windows: label → {tabs, active, version}`
// (consuming window:tabs-updated), and rendering follows "the active tab of this webview window" (useCurrentViewId): main = windows["main"].active, popup = windows[label].active of ?window=<label>.
//
// ★Per-view_id cache model★ (core invariant): the layout is a view_id → {layout,focus,geometry,version} cache.
//   - The backend ViewManager.version is a *global monotonic counter* shared by all views (manager.rs bump_version), so "always adopt when the view_id differs" is wrong:
//     a late emit of another view (lower global version) could overwrite it as stale (F1+F4).
//   - Independent entries per view_id keep versions of different views from colliding; within a view the version is globally monotonic, so monotonic comparison discards stale emits exactly.
//   - keep-alive (ADR-0056): a window mounts all its tabs and shows only the active one, so the cache holds the layouts of all tabs of that window; each slot canvas reads its own view_id cache.

import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { create } from 'zustand'

import type {
  LayoutNode,
  NewWindow,
  SlotContent,
  SlotRect,
  SplitDir,
  SplitRatioApplied,
  SplitRect,
  ViewMeta,
  ViewSnapshot,
  WindowControlAction,
  WindowControlOutcome,
} from '../api/layoutTypes'
import { isRenderMode, type RenderMode } from '../components/slot/renderMode'
import { retryAsync } from '../util/retryInvoke'

/** Main window label (mirror of the backend MAIN_WINDOW_LABEL). Window decision fallback · default target. */
export const MAIN_WINDOW_LABEL = 'main'

/** Display fields to write on a usage slot — missing fields are kept by the shell at that slot's current value (`setUsageSlot`). */
export type UsageShows = Partial<Pick<Extract<SlotContent, { type: 'usage' }>, 'show_claude' | 'show_codex'>>

/** window:tabs-updated / list_tabs payload — mirror of `WindowTabsPayload` in layout/apply.rs. The field names are the wire contract (ADR-0057). */
export interface WindowTabsPayload {
  label: string
  /** The persistent identity of that window — main = `main`, popout = UUID. */
  window_id: string
  /** Window number — main = 1, popouts = 2 and up (TRD S26-window-chrome §4-1). */
  number: number
  /** OS window title — putting it on the OS window is the shell's job, so the web does not read it (TRD S26-window-chrome §4-4). */
  title: string
  tabs: ViewMeta[]
  active: string
  version: number
}

/** The answer of `move_slot_to_window` — mirror of `SlotMove` in layout/apply.rs. `tab` = the tab created in the window moved to. */
export type SlotMove = NewWindow & { tab: string }

/** Tab state of one window (frontend mirror). version = global monotonic (for discarding stale emits). `windowId` · `number` = as in the payload. */
export interface WindowTabs {
  windowId: string
  number: number
  tabs: ViewMeta[]
  active: string
  version: number
}

export interface CachedView {
  layout: LayoutNode
  focusedSlotId: string | null
  /** Area and split rectangles the shell computed (as in the snapshot — tree pre-order). The contract is `ViewSnapshot.slot_rects`·`split_rects`. */
  slotRects: SlotRect[]
  splitRects: SplitRect[]
  /** The shell's split ratio limits (`ViewSnapshot.ratio_min`/`ratio_max`) — taken from the snapshot so the screen does not keep its own constants. */
  ratioMin: number
  ratioMax: number
  /** Slot ids holding content this version does not know (the snapshot's `foreign_slots`). Their `content` is carried as `empty` but they are not empty — the empty-slot pick treats them as occupied (same as shell `ViewManager::slot_is_free`). */
  foreignSlots: string[]
  /** The global version that last adopted this entry (stale emit guard — monotonic comparison within the same view). */
  version: number
}

interface ViewState {
  /** view_id → cached layout. A window canvas renders the cache of each view_id of its own window's tabs (keep-alive). */
  layouts: Record<string, CachedView>
  /** window label → that window's tab state (window:tabs-updated mirror). The single source for the tab bar and useCurrentViewId. */
  windows: Record<string, WindowTabs>

  /**
   * ★Render mode override (§5)★: slot node.id → forced RenderMode, overriding the caps-derived default (defaultRenderMode: structured→'rich' / else→'terminal'); an unspecified slot falls to `?? defaultRenderMode`.
   * ★Frontend only★ — the backend wire LayoutNode does not know render modes, so this field does not ride the invoke→emit loop (a frontend-only exception to "no optimistic updates").
   */
  renderModeOverride: Record<string, RenderMode>

  // ── tab/window commands ──────────────────────────
  /** Adds a new empty-slot tab to window `window` and activates it → returns the new view_id. */
  createTab: (window: string, name?: string) => Promise<string>
  closeTab: (window: string, viewId: string) => Promise<void>
  /** Changes the active tab of window `window` (that window only). */
  switchTab: (window: string, viewId: string) => Promise<void>
  /**
   * Switches to `viewId` only when the active tab of window `window` is `expect` — `true` if switched, `false` if the active tab differs. Errors as `switchTab`. Only for restoring the original selection of a tab drag
   * (TRD S21-slot-docking §4-6 — does not overwrite a tab the user or LLM picked before the restore arrives). ★Not put on the LLM surface★ — the LLM uses `tab.switch` (TRD §3-6).
   */
  switchTabIf: (window: string, viewId: string, expect: string) => Promise<boolean>
  /**
   * Replaces a tab (View) name. view_id is globally unique, so no window is taken (derived from the backend view_owner). The name updates only through the window:tabs-updated emit (no optimistic update — name authority = src-tauri, ADR-0035).
   * §5: the TabBar double-click edit and the LLM (tab.rename) converge here; name normalization (trim/blank refusal) is the caller boundary's job (TabBar · tab.rename).
   */
  renameTab: (viewId: string, name: string) => Promise<void>
  /** Creates an empty new window (1 empty tab) (D-6) → its `{window, window_id, number}` (binding `NewWindow` — TRD S26-window-chrome §5-3). Point at the window again with `window_id` — numbers reuse those of closed windows. */
  createWindow: () => Promise<NewWindow>
  /**
   * One window state action — invoke `window_control` (TRD S26-window-chrome §6-4; same service as the bus `window.*`). `window` is the argument the shell resolves (window_id or a label within the same run — numbers are refused);
   * `hide_all` · `show_all` pick no window. `ifEmpty` is only for `close` — then `window` is that window's window_id (§5-5). Refusal = rejects with the shell text (a string, not an Error); the registry `window.*` adds the command name.
   * ★Closing a window also goes this way★ — main's `close` hides all windows (answer `close: 'hidden_all'`).
   */
  controlWindow: (
    window: string,
    action: WindowControlAction,
    ifEmpty?: boolean,
  ) => Promise<WindowControlOutcome>

  // ── operations inside a View (view_id is globally unique, so the signature stays — the owning window is derived from the backend view_owner) ──
  /** Splits a slot → returns the new slot_id. */
  split: (viewId: string, slotId: string, dir: SplitDir) => Promise<string>
  /**
   * Writes a split ratio (ADR-0227 — divider confirm · double-click 50:50). `ratio` = the share of the a side (left/top) area (ADR-0140). Returns the shell-clamped applied value and the outcome (`version` is meaningful only for `Applied`);
   * on failure rejects with the shell's error string. No optimistic update — the screen changes through layout:updated and the divider preview reconciliation (`splitPreview.ts`).
   * ★Not on the frontend command registry (`window.__engramCmd`)★ — the LLM path is the bus `split.setRatio` alone; the same id on two surfaces makes what they take and give diverge (the `slot.popout` precedent).
   */
  setSplitRatio: (viewId: string, splitId: string, ratio: number) => Promise<SplitRatioApplied>
  /** Sets a slot as the focus (click-to-focus — ADR-0066). Invokes backend focus_slot; the ring updates only through layout:updated (focused_slot_id authority = src-tauri, ADR-0035/0066). Human clicks, the palette, keybindings and the LLM (slot.focus) share this handle (§5). */
  focusSlot: (viewId: string, slotId: string) => Promise<void>
  /** Closes a slot (the sibling is promoted). */
  closeSlot: (viewId: string, slotId: string) => Promise<void>
  assignAgent: (viewId: string, slotId: string, agentId: string) => Promise<void>
  /**
   * Replaces a slot's content with any of the SlotContent union (ADR-0063): tree (agent_list), usage (usage) or clear (empty). Invokes backend set_slot_content; the update comes only through layout:updated (no optimistic update, ADR-0035).
   * §5: the LLM also calls it through `layout.setSlotContent`.
   */
  setSlotContent: (viewId: string, slotId: string, content: SlotContent) => Promise<void>
  /**
   * Writes the display fields of a usage slot field by field — only given fields change; the shell keeps missing fields at the slot's current value inside the lock (a non-usage slot becomes usage with missing fields on). Reflected only through emit.
   * ★To change one field, do not build a whole content with `setSlotContent`★ — this webview's content is stale before the broadcast and would revert another field's earlier write.
   */
  setUsageSlot: (viewId: string, slotId: string, shows: UsageShows) => Promise<void>
  /**
   * MOVEs the slot's agent to a new tab of another window (detach, not mirror). Backend move_slot_to_window creates a new View → moves the agent → inserts a new tab in the target window (a new popup window when unspecified) → removes the source slot,
   * reflected by layout:updated (no optimistic update, ADR-0035). Returns {window, tab} (G4) + the destination's window_id · number (as `createWindow`).
   * §5: the LLM calls it through `slot.popout` — ★that command does not pass toWindow, so it is always a new window★; `toWindow` has no production caller and only `viewStore.test.ts` measures it. The existing-window branch is held by the shell's
   * bus declaration (`to_window` of `slot.popout`); the note that the two surfaces differ in what they take and give is next to the registration in `commands/slotCommands.ts`.
   *
   * ★Dropped out of the command table, not unreachable★: with `withGlobalTauri` (`src-tauri/tauri.conf.json`) `window.__TAURI__.core.invoke` exists in release too, so a CDP caller can get a view id from `list_tabs` and pass `toWindow`
   * to `move_slot_to_window` directly. What was lost is only that those branches do not pass the command table.
   */
  moveSlotToWindow: (
    viewId: string,
    slotId: string,
    toWindow?: string,
  ) => Promise<SlotMove>

  // ── ★render mode override (§5)★ set/clear (frontend only, no invoke) ──────────────────────────
  /** Forces a slot's renderer to mode (overrides the caps-derived default). */
  setRenderMode: (nodeId: string, mode: RenderMode) => void
  /** Clears a slot's override (back to the caps-derived default renderer). */
  clearRenderMode: (nodeId: string) => void

  // ── DOM mode aliases (§5 observation — plain <pre>, so output is readable with CDP innerText) ──────────────────────
  /** Puts a slot in DOM mode (= setRenderMode(id,'dom')). */
  enableDomMode: (nodeId: string) => void
  /** Clears a slot's DOM mode (= clearRenderMode(id)). */
  disableDomMode: (nodeId: string) => void
  /** Toggles a slot's DOM mode (dom ↔ default). */
  toggleDomMode: (nodeId: string) => void

  // ── emit receive handlers (called by eventBus/WindowLayout in their listen callbacks) ───────────────────────────
  /** On layout:updated — updates that view_id cache entry when it passes the version guard. */
  applyLayoutUpdated: (snap: ViewSnapshot) => void
  /** On window:tabs-updated / list_tabs — updates that window's tab list/active (version stale defence). */
  applyWindowTabsUpdated: (payload: WindowTabsPayload) => void
}

export const useViewStore = create<ViewState>((set, get) => ({
  layouts: {},
  windows: {},
  renderModeOverride: {}, // the only update paths = set/clearRenderMode (+DOM aliases) (the contract is the field JSDoc).

  createTab: (window, name) => invoke<string>('create_tab', { window, name: name ?? null }),
  closeTab: (window, viewId) => invoke<void>('close_tab', { window, view: viewId }),
  switchTab: (window, viewId) => invoke<void>('switch_tab', { window, view: viewId }),
  switchTabIf: (window, viewId, expect) => invoke<boolean>('switch_tab_if', { window, view: viewId, expect }),
  createWindow: () => invoke<NewWindow>('create_window'),
  controlWindow: (window, action, ifEmpty) =>
    invoke<WindowControlOutcome>('window_control', { window, action, ifEmpty: ifEmpty ?? null }),

  split: (viewId, slotId, dir) => invoke<string>('split_slot', { viewId, slotId, dir }),
  setSplitRatio: (viewId, splitId, ratio) =>
    invoke<SplitRatioApplied>('set_split_ratio', { viewId, splitId, ratio }),
  // ADR-0057/0035: no optimistic update — only invoke; the name is reflected through the window:tabs-updated emit. §5: human double-click + LLM tab.rename converge here; trim/blank refusal is the caller's (TabBar · tab.rename).
  renameTab: (viewId, name) => invoke<void>('rename_tab', { viewId, name }),
  // ADR-0066: no optimistic update — only invoke is called and the ring is updated only through the layout:updated emit (backend authority, ADR-0035).
  focusSlot: (viewId, slotId) => invoke<void>('focus_slot', { viewId, slotId }),
  closeSlot: (viewId, slotId) => {
    // The slot goes away, so its render override entry is removed right away (leak prevention — renderModeOverride is frontend-only state outside the invoke→emit loop).
    get().clearRenderMode(slotId)
    return invoke<void>('close_slot', { viewId, slotId })
  },
  assignAgent: (viewId, slotId, agentId) => {
    // A slot UUID is stable across reassignment (only agent_id changes) → an override set for the previous agent must not silently apply to the new one; clear it so the new agent starts with the caps-derived default.
    get().clearRenderMode(slotId)
    return invoke<void>('assign_agent', { viewId, slotId, agentId })
  },
  setSlotContent: (viewId, slotId, content) => {
    // The slot content changes entirely (agent→tree etc.), so its render override is cleaned up right away (same as assignAgent); the content replacement itself is reflected by the layout:updated emit (ADR-0035).
    get().clearRenderMode(slotId)
    return invoke<void>('set_slot_content', { viewId, slotId, content })
  },
  setUsageSlot: (viewId, slotId, shows) => {
    // If the slot was not usage, the content changes here — the override cleanup is for the same reason as setSlotContent.
    get().clearRenderMode(slotId)
    // Missing field = null (the shell's `None` — that field keeps its current value).
    return invoke<void>('set_usage_slot', {
      viewId,
      slotId,
      showClaude: shows.show_claude ?? null,
      showCodex: shows.show_codex ?? null,
    })
  },
  moveSlotToWindow: (viewId, slotId, toWindow) => {
    // The slot leaves the source window (MOVE), so its render override entry is cleaned up right away (leak prevention). Slot removal and the target tab insertion arrive by emit (window:tabs-updated of both windows + layout:updated of the source).
    get().clearRenderMode(slotId)
    return invoke<SlotMove>('move_slot_to_window', {
      viewId,
      slotId,
      toWindow: toWindow ?? null,
    })
  },

  // ★Render mode override = frontend-only exception★: no invoke, only frontend state (the backend wire LayoutNode does not know render modes).
  setRenderMode: (nodeId, mode) => {
    // ★Out-of-type entry guard (FIX-4)★: an invalid mode leaked into the override would silently drop to terminal (default) in the LayoutLeaf switch, against the intent — filter before writing, leave only a warning.
    // The registry entry point (`slot.renderMode.set`) throws before reaching here; this is the net behind it.
    if (!isRenderMode(mode)) {
      console.warn(`[viewStore] setRenderMode: ignoring an invalid mode — ${JSON.stringify(mode)}`)
      return
    }
    set(state => ({ renderModeOverride: { ...state.renderModeOverride, [nodeId]: mode } }))
  },
  clearRenderMode: nodeId =>
    set(state => {
      const next = { ...state.renderModeOverride }
      delete next[nodeId] // the renderer goes back to the caps-derived default via ?? defaultRenderMode.
      return { renderModeOverride: next }
    }),

  // ★Aliases stay wrappers over set/clearRenderMode★ — their own state would split the places LayoutLeaf reads (renderModeOverride must stay the only source).
  enableDomMode: nodeId => get().setRenderMode(nodeId, 'dom'),
  disableDomMode: nodeId => get().clearRenderMode(nodeId),
  toggleDomMode: nodeId =>
    get().renderModeOverride[nodeId] === 'dom'
      ? get().clearRenderMode(nodeId)
      : get().setRenderMode(nodeId, 'dom'),

  applyLayoutUpdated: snap => {
    // Update only when larger than that view_id entry's version (globally monotonic, so within-view comparison holds); first receipt has no entry (version === undefined) and is adopted.
    // ★Versions are not compared with other view entries★ — entries are independent; an "always adopt when the view_id differs" branch was the root of F1+F4.
    const prev = get().layouts[snap.view_id]
    if (prev && snap.version <= prev.version) return
    set(state => ({
      layouts: {
        ...state.layouts,
        [snap.view_id]: {
          layout: snap.layout,
          focusedSlotId: snap.focused_slot_id,
          slotRects: snap.slot_rects,
          splitRects: snap.split_rects,
          ratioMin: snap.ratio_min,
          ratioMax: snap.ratio_max,
          foreignSlots: snap.foreign_slots,
          version: snap.version,
        },
      },
    }))
  },

  applyWindowTabsUpdated: payload => {
    // Mirrors that window's tab list and active. ★version stale defence (G10)★: a lower/equal global version for the same window is discarded; the first receipt is adopted. Versions are not compared between windows (independent entries, same as the layout cache).
    const prev = get().windows[payload.label]
    if (prev && payload.version <= prev.version) return
    set(state => ({
      windows: {
        ...state.windows,
        [payload.label]: {
          windowId: payload.window_id,
          number: payload.number,
          tabs: payload.tabs,
          active: payload.active,
          version: payload.version,
        },
      },
    }))
  },
}))

export function selectView(state: ViewState, viewId: string | null): CachedView | null {
  return viewId ? (state.layouts[viewId] ?? null) : null
}

// ── deciding which window this webview is (§3-3/§3-4, G7) ────────────────────────────────────────────────
/**
 * Decides this webview window's label from the URL hash: for the popup route (`#/popup?window=<label>`) that label, for any other hash `"main"`.
 *
 * ★Why the URL★: the popup output Channel subscribes with getCurrentWindow().label() (agent.rs), but the frontend needs its path window label → windows[label].active to know its own active tab; `?window=` is the SSOT (§3-3).
 * ★Single source★: WindowLayout, useCurrentViewId and SlotContextMenu use this same function.
 */
export function readWindowLabelFromHash(): string {
  const hash = window.location.hash
  const qIndex = hash.indexOf('?')
  if (qIndex < 0) return MAIN_WINDOW_LABEL
  // ★Route scoping★: `?window=` is trusted only on the popup route; any other hash (even the unreachable main `#/?window=x`) falls back to main.
  const path = hash.slice(0, qIndex)
  if (path !== '#/popup') return MAIN_WINDOW_LABEL
  const params = new URLSearchParams(hash.slice(qIndex + 1))
  return params.get('window') ?? MAIN_WINDOW_LABEL
}

/**
 * ★The current active tab view id of this webview window (§3-4, G7).★ Derived from the per-window state: main = windows["main"].active, popup = windows[label].active of ?window=<label>.
 * null when it cannot be found (that window's tab state has not arrived yet — right after boot, before the pull).
 * Human clicks (SlotContextMenu) and LLM commands share this decision and act on "the active tab of their own window" — a call inside a popup does not touch an unrelated main view.
 */
export function useCurrentViewId(): string | null {
  const label = readWindowLabelFromHash()
  return useViewStore(s => s.windows[label]?.active ?? null)
}

/** Queries the current window's active tab outside hooks (event handlers). The non-hook version of useCurrentViewId. */
export function currentViewId(): string | null {
  const label = readWindowLabelFromHash()
  return useViewStore.getState().windows[label]?.active ?? null
}

// ── emit subscription registration (called once from eventBus; the HMR/duplicate guard is managed by eventBus with dispose) ──────────
// Returns `{ dispose, ready }` synchronously: the caller holds dispose *immediately* without await, and runs the init pull only after awaiting ready (F-listen).
// listen() is async, so an emit arriving before registration finishes has no handler and is lost → ready exposes registration completion so the pull runs after it.
//
// ★Why a synchronous dispose (prevents the dead-branch leak)★: if the disposer were returned after awaiting registration, a cleanup (HMR dispose/re-init) during that await would not have it yet and could not unlisten
// → a late-completing listener leaks permanently. Like RxJS Subscription/useSyncExternalStore, the teardown handle is returned synchronously and registration starts in the background.
// Rationale: docs/research/async-subscribe-cleanup-race-2026-06-28.md.
//
// ★Tab ownership model (ADR-0057)★: listens to window:tabs-updated (per window) instead of the old global view:list-updated; layout:updated stays. Each WindowLayout also listens with its own label filter, but this global
// subscription fills the store with the main window's tab state so AppLayout/AgentTree react immediately (mirrors every label into the store).
export function subscribeViewEvents(): { dispose: () => void; ready: Promise<void> } {
  let disposed = false
  // Unlisten handles whose registration completed (only those not released yet). dispose calls only what is held here → idempotent guaranteed.
  const handles: UnlistenFn[] = []

  const store = useViewStore.getState()

  // The single path receiving freshly registered handles. If dispose was already called, release immediately (the real branch of a dispose before ready — guards late handles from leaking); otherwise gather them for dispose.
  const adopt = (fn: UnlistenFn): void => {
    if (disposed) {
      fn()
    } else {
      handles.push(fn)
    }
  }

  // Registration starts in the background — dispose returns immediately without waiting (the core of the leak guard). ready resolves after both registrations settle (F-listen); if one fails, Promise.all rejects *immediately* (no hang).
  //   Even if dispose comes first, adopt releases arriving handles immediately, so ready ends normally.
  const ready = Promise.all([
    listen<ViewSnapshot>('layout:updated', e => {
      store.applyLayoutUpdated(e.payload)
    }).then(adopt),
    listen<WindowTabsPayload>('window:tabs-updated', e => {
      store.applyWindowTabsUpdated(e.payload)
    }).then(adopt),
  ]).then(() => undefined)

  const dispose = (): void => {
    disposed = true
    // Releases and clears all held handles → idempotent (a second call finds an empty array); handles arriving late before ready are released by adopt's disposed branch.
    while (handles.length > 0) handles.pop()!()
  }

  return { dispose, ready }
}

// ── boot init — main window tab state + active tab layout pull (read-only) ──────────────────────────
/**
 * Pulls the backend's main window tab state and puts the active tab layout in the cache. eventBus calls it once right after registering the subscriptions (after ready).
 *
 * Why: ViewManager creates the default View at boot *before* anything listens, so it never reaches us by emit. read-only list_tabs("main")/get_view pull current tabs + active layout so the screen draws at once.
 * ★Without creating a ghost View★ — pull only (no version bump, no broadcast). Called after the subscriptions are set up, so a newer emit that came during init makes the cache/window version guard filter out the (older) pull.
 */
export async function initMainWindowFromBackend(): Promise<void> {
  // ADR-0102: both boot pulls use bounded retries — the main window has no event recovery path (window:tabs-updated fires only on tab mutations), so a single transient failure would leave the screen never filled.
  //   When retries are exhausted it throws to the caller (eventBus), which surfaces it with console.error (not swallowed).
  const payload = await retryAsync(() =>
    invoke<WindowTabsPayload>('list_tabs', { window: MAIN_WINDOW_LABEL }),
  )
  useViewStore.getState().applyWindowTabsUpdated(payload)
  // The active tab layout is pulled too so the canvas draws at boot (other tabs are filled by WindowLayout with get_view on mount — keep-alive).
  const snap = await retryAsync(() =>
    invoke<ViewSnapshot>('get_view', { viewId: payload.active }),
  )
  useViewStore.getState().applyLayoutUpdated(snap)
}
