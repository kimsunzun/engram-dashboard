// ★One path for main · popup (D-2 "same code path")★: the main window (wrapped by AppLayout as chrome) and the popup window (thinly wrapped
// by PopoutPage) both mount this (each with its own label). The old split "AppLayout = global active render vs
// PopoutPage = fixed view render" (a D-2 violation) is removed.
//
// ★keep-alive (ADR-0056)★: mount *all* of windows[label].tabs and show only the active tab (hidden tabs
// display:none — xterm instance · buffer kept, switch instant · lossless). The WebglAddon seat only for visible slots
// (hidden tabs leave the screen and are invisible by CSS, but the instance stays alive and keeps accumulating output — the backend routes to every tab).
//
// ★Tab drag (TRD S21-slot-docking §4-3 · §4-4)★: this window's root receives our tab drag (`dock/useWindowDrop` — decision · guide · drop
// command) and draws that guide on the layer over the canvas (`DockOverlayView`). The tab bar's drag appearance belongs to `TabBar`, and the canvas shown
// while dragging in this window (R15 remaining tab) is decided by the source side `useShownTab` — keep-alive `display` follows it. File · URL drops are
// blocked by `dock/fileDropGuard.ts`.
//
// ★Waiting mode (prewarmed window — ADR-0299 · TRD S21-slot-docking §3-9)★: a window of the spare window pool (hidden spare window) is outside the tab model. That window
// draws only the title row (empty tab bar — no number badge · window buttons) + an empty canvas (empty frame), does not pull in tabs, and turns off 0-tab self-close. Measuring
// reports (canvas · metrics) are still sent. The mode is decided by `spareMode.ts`.

import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'

import type { ViewMeta, ViewSnapshot } from '../../api/layoutTypes'
import { fireAndForget } from '../../commands/dispatch'
import { selectView, useViewStore, type WindowTabsPayload } from '../../store/viewStore'
import { REPORT_DEBOUNCE_MS } from '../../util/latestValueReporter'
import { retryAsync, RetryCancelledError } from '../../util/retryInvoke'
import {
  getCanvasReport,
  observeCanvasSize,
  releaseCanvasReport,
  submitCanvasSize,
  type CanvasPx,
} from './windowCanvasReport'
import ViewLayoutRenderer from './ViewLayoutRenderer'
import TabBar from './TabBar'
import TitleRow from './titleRow/TitleRow'
import { dragProps } from './titleRow/dragRegion'
import { minWindowSizeSettled } from './titleRow/minSizeReport'
import { watchSpareMode, type WindowMode } from './spareMode'
import { installFileDropGuard } from './dock/fileDropGuard'
import { DockOverlayView } from './dock/DockOverlay'
import { windowFrame } from './dock/windowFrame'
import { useWindowDrop } from './dock/useWindowDrop'
import { useShownTab } from './tabStrip/useTabDragSource'
import { reportUiMetrics, SLOT_FRAME_BORDER } from './uiMetricsReport'
import AgentMonitoringPicker from '../slot/AgentMonitoringPicker'
import { useMonitoringPickerStore } from '../../store/monitoringPickerStore'
import { reportFirstPaint } from '../../theme/firstPaint'
import { t } from '../../i18n'

interface WindowLayoutProps {
  /** This window's label (main·slot-popup-N). */
  label: string
  /**
   * Notice layer (`NoticeOverlay`) — covers the top of the area just under the title row. ★It does not cover the title row★ — if the window buttons · draggable area are
   * covered by a notice, the window can be neither moved nor closed (user decision 2026-10-26 · T-66 · TRD S26-window-chrome §12-2). The loading · boot
   * failure screens use the same place — ★whichever screen, put it at the same place in the tree★: if the place changes, React remounts the layer and the memory of closing with ✕
   * (`ConnectionNotice` · `StateFileNotice` — what ADR-0277 said not to withdraw) is lost.
   */
  // ADR-0306 — notice band position
  notices?: ReactNode
}

// Tab list of waiting mode — the reference must be fixed so that effects hanging on the tab list (window frame report · keep-alive pull) do not rerun on every render.
const NO_TABS: ViewMeta[] = []

export default function WindowLayout({ label, notices }: WindowLayoutProps) {
  const win = useViewStore(s => s.windows[label])
  const openId = useMonitoringPickerStore(s => s.openId)
  const applyWindowTabsUpdated = useViewStore(s => s.applyWindowTabsUpdated)
  const applyLayoutUpdated = useViewStore(s => s.applyLayoutUpdated)
  const switchTab = useViewStore(s => s.switchTab)
  const createTab = useViewStore(s => s.createTab)
  const closeTab = useViewStore(s => s.closeTab)
  const renameTab = useViewStore(s => s.renameTab)

  const closingRef = useRef(false)
  const windowIdRef = useRef<string | null>(null)
  const canvasElRef = useRef<HTMLDivElement | null>(null)
  const reportCanvas = useWindowCanvasReport()
  const canvasRef = useCallback(
    (el: HTMLDivElement | null) => {
      canvasElRef.current = el
      reportCanvas(el)
    },
    [reportCanvas],
  )
  const canvasOf = useCallback(() => canvasElRef.current, [])
  const guide = useWindowDrop({ label, canvas: canvasOf })
  const titleRowRef = useRef<HTMLDivElement>(null)

  // ADR-0102: surfacing the final boot pull failure — if this window's state (win) is still not filled after retries are exhausted, it would be stuck forever on the loading
  //   placeholder (main has no event recovery path), so instead of a quiet console.warn this flag renders
  //   a visible error state. If win is filled by an emit in the meantime (race), the render below takes the normal path.
  const [bootFailed, setBootFailed] = useState(false)

  // `null` = not known yet (before the `window_role` answer) — tab pull-in · self-close run only when `normal`.
  const [mode, setMode] = useState<WindowMode | null>(null)
  const [wasSpare, setWasSpare] = useState(false)
  // The adoption number this page received (`spare:adopted` · `window_role` answer — TRD S21-slot-docking §3-9 I26). `0` is also a number.
  const [adoption, setAdoption] = useState<number | null>(null)
  const adoptionRef = useRef<number | null>(null)
  // The moment the empty frame is withdrawn relies on the order in which the shell announces the adopted window's tabs before lifting the wait — the assumption is that prewarmed window adoption (ADR-0299 · TRD
  //   S21-slot-docking §3-9 「`committed`」) announces in that order.
  // A new adoption number lowers the boot failure in the same render — a prewarmed page whose `window_role` failed and which pulled its own label, unknown to the model, in normal mode
  //   is already `bootFailed`, and since the mode stays `normal` the number must change for the pull to run again (deps
  //   below). Rendering separately would announce the failure text screen with the new number in between, and the shell would show that screen. If the same number arrives repeatedly (the answer and
  //   the event both carry it), it is not lowered — the pull does not rerun and it stays on the loading background.
  // ADR-0299
  useEffect(
    () =>
      watchSpareMode(next => {
        setMode(next.mode)
        if (next.mode === 'spare') setWasSpare(true)
        if (next.generation !== null && next.generation !== adoptionRef.current) {
          adoptionRef.current = next.generation
          setAdoption(next.generation)
          setBootFailed(false)
        }
      }),
    [],
  )
  const spare = mode === 'spare'
  const normal = mode === 'normal'
  // An adopted prewarmed window (`spare:adopted`) stays an empty frame until it receives its tabs — switching to the 「로딩」 screen (background only) makes the tab bar
  //   disappear and mount again. A prewarmed window comes up waiting under a new label, so at mount the store has no entry for that window — therefore
  //   `win` present = the tabs received at adoption.
  const framed = spare || (wasSpare && win === undefined && !bootFailed)

  useEffect(() => installFileDropGuard(), [])

  // ── Normal mode: initial pull of own window's tab state + window:tabs-updated listen (own label only) ────────────────
  // ★Subscribe first, pull later★ (same shape as F-listen): subscribe first so an emit arriving before listen registration completes
  // is not missed, then pull. If a newer emit overwrites the pull, the version guard of applyWindowTabsUpdated prevents the reversal.
  // When taken in from waiting mode (`spare:adopted`) it comes back in here and pulls the adopted tabs — if already in normal mode, a new adoption number reruns it.
  // ADR-0295
  // ADR-0299
  useEffect(() => {
    if (!normal) return
    let disposed = false
    let unlisten: (() => void) | null = null

    // ADR-0102 (FIX-2): wrap ★the whole boot path★ in one try — in the old structure the listen() await was outside this try,
    //   so if listener registration rejected during boot, the async IIFE died as unhandled and the list_tabs pull itself
    //   never even started, so bootFailed was never set (loading placeholder stuck forever — no signal). A listen() failure is also
    //   surfaced as bootFailed like a pull failure, keeping the "no silent permanent stuck" invariant (ADR-0102).
    void (async () => {
      try {
        const off = await listen<WindowTabsPayload>('window:tabs-updated', e => {
          if (e.payload.label !== label) return
          applyWindowTabsUpdated(e.payload)
        })
        if (disposed) {
          off() // If unmounted before registration completes, release immediately (leak guard)
          return
        }
        unlisten = off
        // ADR-0102: wrap the initial pull in bounded retry — main has no event recovery path (window:tabs-updated fires only on
        //   tab mutation), so if this pull is one-shot, an early failure = stuck forever.
        const payload = await retryAsync(
          () => invoke<WindowTabsPayload>('list_tabs', { window: label }),
          {
            isCancelled: () => disposed,
            onRetry: (err, attempt) =>
              console.warn(`[WindowLayout] list_tabs(${label}) 재시도 ${attempt}:`, err),
          },
        )
        if (!disposed) {
          applyWindowTabsUpdated(payload)
          setBootFailed(false)
        }
      } catch (err) {
        if (err instanceof RetryCancelledError) return
        console.error(`[WindowLayout] 부팅 실패(listen 등록 or list_tabs 재시도 소진, ${label}):`, err)
        if (!disposed) setBootFailed(true)
      }
    })()

    return () => {
      disposed = true
      if (unlisten) unlisten()
    }
  }, [label, normal, adoption, applyWindowTabsUpdated])

  // Even if this window's entry fills again during waiting (the shell lifts the wait after announcing the tabs to take in — the global mirror receives them), do not draw before the wait is lifted.
  const tabs = framed ? NO_TABS : win?.tabs
  const active = framed ? null : (win?.active ?? null)
  const tabIds = useMemo(() => tabs?.map(tab => tab.id) ?? null, [tabs])
  // Visible canvas — while dragging in this window, if outside the tab bar zone, the remaining tab (R15 · the model is unchanged — TRD S21-slot-docking §4-2).
  const shown = useShownTab(label, tabIds, active)

  // The shell builds this window hidden and shows it on the first-screen notice (`theme/firstPaint.ts`) — send that notice after the first screen (title row + the active tab's
  //   layout · boot failure text) is committed. The earlier screens (background only · the active tab's 「View 로딩」) are visible only when the notice
  //   does not arrive and the shell deadline shows the window.
  // The empty frame is not a first screen — a waiting window stays hidden until it is handed over, and an adopted window notifies with the adoption number after drawing its tabs
  //   (TRD S21-slot-docking §3-9 I26). The number is in deps so that it notifies again even if `firstScreen` stays true across adoption (lowering the boot failure
  //   and applying the tabs happen in one render) — without it, an adopted window would always be shown only by the shell deadline.
  // The notice comes after the title row's minimum window size report (`titleRow/minSizeReport.ts`) finishes — so the window is widened before the shell shows it (TRD
  //   S26-window-chrome §7-7). The wait has an upper bound. If deps change while waiting, that notice is discarded — if an old adoption number goes out after the new
  //   number, it is sent once more.
  // ADR-0299
  const activeLaidOut = useViewStore(s => active === null || selectView(s, active) !== null)
  const firstScreen = bootFailed || (!framed && tabs !== undefined && activeLaidOut)
  useEffect(() => {
    if (!firstScreen) return
    let current = true
    void minWindowSizeSettled().then(() => {
      if (current) reportFirstPaint(adoption)
    })
    return () => {
      current = false
    }
  }, [firstScreen, adoption])

  // ── keep-alive: fill the cache with the layouts of all this window's tabs (hidden tabs stay rendered, ADR-0056) ───────────
  // layout:updated is already reflected in the cache by the eventBus global subscription (per-view version guard), so here only "tabs
  // never received yet" get an initial pull.
  const tabIdsKey = tabs ? tabs.map(t => t.id).join(',') : ''
  // ★S4-F2: deps is only the tab set (tabIdsKey)★ — if win (the whole object) is put in deps, the win reference changes on every switch that changes only active,
  //   so the effect reruns and cancels · reissues the in-flight get_view (wasted pulls on every tab switch).
  //   Narrow it to the single tabIdsKey so it reruns only when the tab list actually changes (add/close). The tab id array is recovered
  //   from tabIdsKey (avoiding the win reference by a direct store lookup) and iterated.
  useEffect(() => {
    if (tabIdsKey === '') return
    let cancelled = false
    for (const tabId of tabIdsKey.split(',')) {
      if (useViewStore.getState().layouts[tabId]) continue
      // ADR-0102 (FIX-3): this pull is also wrapped in bounded retry — the old one-shot only did console.warn on a transient failure,
      //   and tabIdsKey stays the same so there is no rerun trigger, and that tab's canvas stays stuck on "View 로딩 중" until an unrelated layout:updated
      //   arrives. Retry recovers from momentary unreadiness by itself. ★Separate from the window-wide bootFailed★: the final failure
      //   of a single (usually hidden) tab is not an app-wide stuck state, so it is only left as console.error.
      void retryAsync(() => invoke<ViewSnapshot>('get_view', { viewId: tabId }), {
        isCancelled: () => cancelled,
        onRetry: (err, attempt) =>
          console.warn(`[WindowLayout] get_view(${tabId}) 재시도 ${attempt}:`, err),
      })
        .then(snap => {
          if (!cancelled) applyLayoutUpdated(snap)
        })
        .catch(err => {
          if (err instanceof RetryCancelledError) return
          console.error(`[WindowLayout] get_view(${tabId}) 최종 실패(재시도 소진):`, err)
        })
    }
    return () => {
      cancelled = true
    }
  }, [tabIdsKey, applyLayoutUpdated])

  // ── 0-tab self-close = ★defensive idempotent fallback★ (S4-F3, §5-2/G2) ─────────────────────────────────
  // ★This path is dead in the normal flow★: when the last tab of a popup is closed, the shell close_tab classifies it as WindowClosed and
  //   *directly destroys* the OS window (`layout/apply.rs` `close_tab` — the WindowClosed branch sets the emit to (None,None) so
  //   window:tabs-updated{tabs:[]} is not sent to the frontend). That is, the main path of a normal close is the shell destroy, and the frontend
  //   never receives the tabs:[] signal.
  // Why this branch is kept anyway (do not remove): if a window:tabs-updated with 0 tabs reaches the frontend by some abnormal path (defense),
  //   it asks the shell to close its own window once and cleans up the stranded window (main never reaches 0 because the backend forces an empty tab to be
  //   kept). ★Re-entry guard (closingRef)★: after asking, no-op until tabs come back.
  // ★Close is decided by the shell — the web does not close through the window API★ (TRD S26-window-chrome §5-5 · AC10 · the popout permissions have no window close):
  //   `window.close { if_empty }` does not close if that window is back in the model (a tab came in meanwhile). That decision
  //   looks up by the persistent id, so only the id received while tabs existed is passed (§5-5) — a 0-tab window is already out of the model. If there is no id
  //   received (this screen has never seen that window's tabs), it does not ask.
  // ★Only in normal mode★ — 0 tabs is normal for a waiting window (TRD S21-slot-docking §3-9). A 0-tab that arrives while the mode is unknown is looked at again after the mode is
  //   decided.
  // ADR-0295
  useEffect(() => {
    if (!normal || !win) return
    if (win.tabs.length > 0) {
      windowIdRef.current = win.windowId
      closingRef.current = false
      return
    }
    if (closingRef.current) return
    const windowId = windowIdRef.current
    if (!windowId) {
      console.warn(`[WindowLayout] 0탭 자가닫힘 — ${label} 이 탭을 가진 때의 window_id 가 없어 청하지 않는다`)
      return
    }
    closingRef.current = true
    fireAndForget('window.close', { window: windowId, if_empty: true })
  }, [win, normal, label])

  // ★The frame is the same on every screen — title row → area (first = notice layer) → screen content★. With the OS frame removed, the loading · boot failure screens also need a title row (window
  //   buttons · draggable area) so the window can be moved and closed. The notice layer must be at the same place on every screen so the memory of closing with ✕ remains (`notices` doc).
  // Screens before receiving tabs: right after boot before the pull, or no valid label found (dev reload stale popup, §3-3/G3) · boot failure (ADR-0102).
  //   Loading draws only the background — drawing text · an indicator would flash once in front of the tab screen when the shell shows the window by the deadline (user requirement
  //   2026-10-19: a window's first screen is the theme background followed directly by the real content). The area of this screen is not a canvas — it does not report size.
  let content: ReactNode
  if (!tabs) {
    content = bootFailed ? (
      <div style={{ ...centerStyle, ...FILL }} data-testid="window-boot-error">
        <span>{t('window.loadFailed', { label })}</span>
      </div>
    ) : (
      <div data-testid="window-loading" style={{ ...blankStyle, ...FILL }} />
    )
  } else {
    content = (
      <>
        {framed && <SpareCanvas />}
        {tabs.map(tab => (
          <div
            key={tab.id}
            data-testid="tab-canvas"
            data-view-id={tab.id}
            // ★keep-alive★: overlay them with absolute positioning so a switch is a display toggle, not a remount (ADR-0056).
            style={{
              position: 'absolute',
              inset: 0,
              display: tab.id === shown ? 'block' : 'none',
            }}
          >
            <TabCanvas viewId={tab.id} />
          </div>
        ))}
        {/* Drag guide — always mounted and cleared with `null` (taking it down would lose the disappear fade). This layer is the same box as the canvas, so guide coordinates =
            canvas CSS px (same origin as the decision). */}
        <div data-testid="dock-overlay-layer" style={OVERLAY_LAYER}>
          <DockOverlayView guide={guide} />
        </div>
      </>
    )
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', width: '100%', height: '100%' }}>
      <TitleRow ref={titleRowRef} label={label} number={framed ? null : (win?.number ?? null)} standby={framed}>
        {tabs ? (
          <TabBar
            label={label}
            titleRow={titleRowRef}
            tabs={tabs}
            active={active}
            onSwitch={viewId => void switchTab(label, viewId).catch(e => console.error('[switchTab]', e))}
            onCreate={() => void createTab(label).catch(e => console.error('[createTab]', e))}
            onClose={viewId => void closeTab(label, viewId).catch(e => console.error('[closeTab]', e))}
            onRename={(viewId, name) =>
              void renameTab(viewId, name).catch(e => console.error('[renameTab]', e))
            }
            canvas={canvasOf}
            // Do not pull while the mode is unknown either — for a waiting window the shell does not know that window and the call fails.
            pullFrame={normal}
          />
        ) : (
          <TabBarStandIn pullFrame={normal} />
        )}
      </TitleRow>
      {/* The canvas size report (callback ref) attaches only while tabs are drawn — it is the size of the tab content area (ADR-0227). */}
      <div ref={tabs ? canvasRef : undefined} style={{ flex: 1, position: 'relative', minHeight: 0 }}>
        {/* The notice layer attaches to the top of this area (positioned) = directly under the title row — it is out of flow, so it does not change the canvas size report (ADR-0306). */}
        {notices}
        {content}
      </div>
      {/* ADR-0067: agent monitoring search popup — one mounted per window. The slot right-click "에이전트 모니터링"
          (slot.assignRunningAgent) opens it through monitoringPickerStore, and on-select flows to assign_agent (§5).
          When closed (target=null) nothing is rendered.
          key={openId}: the key changes on every open() for a fresh remount → query/activeIndex are
          reset to the useState initial values (no stale flash, monitoringPickerStore.openId ADR-0067). */}
      {tabs && <AgentMonitoringPicker key={openId} />}
    </div>
  )
}

// The tab bar position of the screen before tabs are received — like the tab bar it takes the remaining width, puts the window buttons at the right end, and makes that area a draggable area. Window
//   facts (maximized · front window) are received like the tab bar too — so that this screen's row height · maximize button follow the window state.
function TabBarStandIn({ pullFrame }: { pullFrame: boolean }) {
  useEffect(() => (pullFrame ? windowFrame.install() : undefined), [pullFrame])
  return <div data-testid="tab-bar-standin" aria-hidden {...dragProps()} style={TAB_BAR_STAND_IN} />
}

// The same share as the tab bar root — the shrinking side · the line below (`TabBar` root). The basis width gets a unit as `0px` (jsdom drops a unitless 0).
const TAB_BAR_STAND_IN: React.CSSProperties = {
  flex: '1 1 0px',
  minWidth: 0,
  alignSelf: 'stretch',
  boxSizing: 'border-box',
  borderBottom: '1px solid var(--tab-bar-edge)',
}

const FILL: React.CSSProperties = { position: 'absolute', inset: 0 }

// The z of the drag guide layer = above the split bars · focus ring (z 20) in an area · below the connection guide (z 40) · menu (1000 and up). The canvas box and the tab canvas
//   layer do not create a stacking context, so z inside them and this layer compare in the same context.
const OVERLAY_LAYER: React.CSSProperties = { position: 'absolute', inset: 0, pointerEvents: 'none', zIndex: 30 }

/** Observation state owned by the hook instance. */
interface CanvasObserverState {
  ro: ResizeObserver | null
  timer: ReturnType<typeof setTimeout> | null
  /** The latest measurement waiting in debounce (after rounding). */
  measured: CanvasPx | null
}

// ADR-0227: measure the tab content area (window canvas) size and report to the shell only when it changes. The tabs share this area, so there is one
//   per window. Boot · popout creation · webview reload are all new mounts, so the first measurement is the report. What to send and when (one at a time
//   · merging · retry) is decided by `windowCanvasReport.ts`.
// Why a callback ref: the measured div is rendered only after win arrives (early return), so it does not yet exist in the effect at mount.
//   Why the reference is fixed: a new function on every render makes React detach and reattach, and the debounce · retry would be lost every time.
function useWindowCanvasReport(): (el: HTMLDivElement | null) => void {
  const obsRef = useRef<CanvasObserverState>({ ro: null, timer: null, measured: null })

  return useCallback((el: HTMLDivElement | null) => {
    const obs = obsRef.current
    const st = getCanvasReport()
    detachCanvasObserver(obs)
    releaseCanvasReport(st)
    if (!el) return
    // In environments without RO such as jsdom (vitest has no setupFiles), it passes without observing.
    if (typeof ResizeObserver === 'undefined') return
    const ro = new ResizeObserver(entries => {
      const rect = entries[entries.length - 1]?.contentRect
      if (!rect) return
      const size: CanvasPx = { w: Math.round(rect.width), h: Math.round(rect.height) }
      observeCanvasSize(st, size)
      obs.measured = size
      if (obs.timer !== null) clearTimeout(obs.timer)
      obs.timer = setTimeout(() => {
        obs.timer = null
        const measured = obs.measured
        obs.measured = null
        if (measured) submitCanvasSize(st, measured)
      }, REPORT_DEBOUNCE_MS)
    })
    ro.observe(el)
    obs.ro = ro
  }, [])
}

function detachCanvasObserver(obs: CanvasObserverState): void {
  obs.ro?.disconnect()
  obs.ro = null
  if (obs.timer !== null) {
    clearTimeout(obs.timer)
    obs.timer = null
  }
  obs.measured = null
}

// The empty canvas of waiting mode — draws only one area frame. There are no tabs so no leaf is mounted, so the area frame metrics (`report_ui_metrics`)
//   are sent from here — when a spare window is adopted, the shell copies these metrics into the model (TRD S21-slot-docking §3-9 `committed`).
// ADR-0295
function SpareCanvas() {
  const borderRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (borderRef.current !== null) void reportUiMetrics(borderRef.current)
  }, [])
  return (
    <div
      ref={borderRef}
      data-testid="spare-canvas"
      style={{ position: 'absolute', inset: 0, background: 'var(--bg)', boxSizing: 'border-box', ...SLOT_FRAME_BORDER }}
    />
  )
}

/**
 * Pass its own view down with viewIdOverride so the inner SlotContextMenu actions use this tab's coordinates
 * (unaware of per-window active — §3-4).
 */
function TabCanvas({ viewId }: { viewId: string }) {
  const cached = useViewStore(s => selectView(s, viewId))
  if (!cached) {
    return (
      <div style={centerStyle}>
        <span>{t('common.viewLoading')}</span>
      </div>
    )
  }
  return (
    <ViewLayoutRenderer
      node={cached.layout}
      focusedSlotId={cached.focusedSlotId}
      viewIdOverride={viewId}
      slotRects={cached.slotRects}
      splitRects={cached.splitRects}
      ratioBounds={{ min: cached.ratioMin, max: cached.ratioMax }}
      version={cached.version}
      foreignSlots={cached.foreignSlots}
    />
  )
}

const blankStyle: React.CSSProperties = {
  width: '100%',
  height: '100%',
  background: 'var(--bg)',
}

const centerStyle: React.CSSProperties = {
  width: '100%',
  height: '100%',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  background: 'var(--bg)',
  color: 'var(--text-muted)',
  fontFamily: 'var(--font-ui)',
  fontSize: '12px',
}
