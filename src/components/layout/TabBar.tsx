// TabBar — one window's (label) tab list + active-tab marker + [+] new tab + per-tab close + inline name edit (ADR-0057, §7-2).
//
// ★§5 hands/brain split★: human actions (tab switch/add/close/rename) call only the viewStore actions (switchTab/createTab/closeTab/
// renameTab) — physically the same
// surface as the LLM (window.__engramCmd) driving the same commands. The real state change is made by the backend ViewManager (authority) and reflected through the window:tabs-updated emit (no optimistic update).
//
// ★Tab drag (TRD S21-slot-docking §4-2 · §4-4) = the head is the OS (HTML5) drag source · `tabStrip/useTabDragSource`★ — one hook
//   per tab bar. Drawing rules:
//   - The row (`tab-strip-row`) holds only the heads and `+` (`measure.ts` measures row width − `+` width). The vertical line for a drag from another window
//     is drawn over that row and exists only then.
//   - Head width · hidden = `stripBounds` · position = absolute x (`translateX`). While dragging inside its own tab bar zone, the order move changes only
//     the transform — the dragged head follows the cursor and neighbors step aside to their positions in the confirmed order (`DragSourceSnapshot.motion`). ★The dragged head is
//     drawn even when hidden★ — if the source node is detached, that drag's dragend never arrives.
//   - The head DOM order (= tab key · screen reader order) matches the visible order at rest, and during press · drag · the position transition after drop · name edit
//     it keeps the order already drawn — if React moves an element (detach and reattach), the CSS transition in progress is cut, so neighbors · the dragged tab
//     jump instead of sliding (not measured on WebView2), the name input being edited loses focus, and for the dragged head the OS drag breaks.

import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type RefObject,
} from 'react'

import { Plus, X } from 'lucide-react'
import type { ViewMeta } from '../../api/layoutTypes'
import { t } from '../../i18n'
import { shownTab, stripHeld } from './dock/dragOverride'
import { useLocalDrag } from './dock/localDrag'
import { TAB_ADD_GAP, TAB_BUTTON_SIZE, TAB_HEAD_GAP, TAB_HEAD_PAD, TAB_MIN_ACTIVE_W, TAB_STANDARD_W } from './tabStrip/layout'
import { measureStrip, stripBounds, tabsEnd } from './tabStrip/measure'
import { EASE_OUT_CSS, slideMs, slideTargets } from './tabStrip/reorderGeometry'
import { useTabDragSource } from './tabStrip/useTabDragSource'
import { dragProps, useBlurOnDragRegionPress } from './titleRow/dragRegion'
import { SURFACE_TOP, TAB_BAR_H } from './titleRow/metrics'

interface TabBarProps {
  /** Label of the window this tab bar belongs to (main·slot-popup-N). */
  label: string
  tabs: ViewMeta[]
  /** The model's active tab — `null` = none (the empty frame in waiting mode). */
  active: string | null
  onSwitch: (viewId: string) => void
  onCreate: () => void
  onClose: (viewId: string) => void
  onRename: (viewId: string, name: string) => void
  /** This window's tab canvas area — measures the chrome of the drag size (viewport − canvas). Without it, chrome is taken as 0. */
  canvas?: () => Element | null
  /** `UseTabDragSourceOptions.pullFrame`. */
  pullFrame?: boolean
  /** `UseTabDragSourceOptions.titleRow`. */
  titleRow?: RefObject<HTMLElement | null>
}

/** One head to draw — `x` = CSS px from the left of the tab bar row. */
interface DrawnHead {
  id: string
  x: number
  w: number
  transition: string
}

// TODO(U7c): once close mode (R26 · `closingMode.layoutAvailable`) is wired in, this value becomes that fixed width — the hook and the drawing receive the same value.
const WIDTH_CAP: number | null = null

// ADR-0295
export default function TabBar({
  label,
  tabs,
  active,
  onSwitch,
  onCreate,
  onClose,
  onRename,
  canvas,
  pullFrame,
  titleRow,
}: TabBarProps) {
  const [editingId, setEditingId] = useState<string | null>(null)
  const [draft, setDraft] = useState('')
  // ★Stable ref★: if a new callback ref is attached to the input on every render, React re-attaches it each time → select() runs
  //   on every keystroke and selects the whole text again → the next character overwrites everything ("New"→"w"). A stable ref + effect
  //   fix it so select() runs exactly once, only when edit is entered (editingId changes).
  const inputRef = useRef<HTMLInputElement>(null)
  useEffect(() => {
    if (editingId !== null) inputRef.current?.select()
  }, [editingId])

  // The name at the moment the edit was opened — commit compares against this, not the current name. Even if the LLM renames during the edit or an auto name changes,
  //   an untouched field does not write back the old name. It is compared with the committed text (`draft.trim()`), so it is trimmed — so that for a
  //   name with surrounding whitespace, an untouched field does not send the trimmed text as a 「바뀐 글자」, which would lock in an auto name or overwrite a name the LLM changed.
  const seed = useRef('')
  const beginEdit = (tab: ViewMeta) => {
    seed.current = tab.name.trim()
    setEditingId(tab.id)
    setDraft(tab.name)
  }
  const cancelEdit = () => setEditingId(null)
  // The tab that the last head press turned active (a press that changed nothing = `null`) — a narrow inactive head has no close button, so that tab's close
  //   button may have appeared under the cursor on the head widened after that press.
  const activatedBy = useRef<string | null>(null)
  // The blur that fires when the input is removed after Enter does not reach React — React 19 turns off event delivery during commit
  //   (`_enabled=false` in react-dom `prepareForCommit`), and a later blur fires on a detached node that does not reach the root listener.
  //   The `editingId` check on the first line is still a safeguard so that onRename is not sent twice when commitEdit is called twice.
  // `deliberate` = Enter. The same text is sent only by Enter — committing an auto-named tab (`named` false) with that text unchanged via Enter means
  //   the user chose that name (TRD S26-window-chrome §3-1), while focus loss · a press on a covered neighbor are not meant to commit and send only changed text —
  //   otherwise opening and closing an edit alone would lock in an auto name. For a tab with a chosen name, the same text has nothing to change.
  const commitEdit = (tab: ViewMeta, deliberate: boolean) => {
    if (editingId !== tab.id) return
    const trimmed = draft.trim()
    if (trimmed.length > 0 && (trimmed !== seed.current || (deliberate && !tab.named))) onRename(tab.id, trimmed)
    setEditingId(null)
  }
  // Window draggable area = empty part of the tab strip · title row · empty draggable area — a press there also ends the edit (the commit rule stays as in onBlur).
  useBlurOnDragRegionPress(editingId !== null, inputRef)

  const rowRef = useRef<HTMLDivElement>(null)
  const addRef = useRef<HTMLButtonElement>(null)
  const available = useStripAvailable(rowRef, addRef)
  const ids = useMemo(() => tabs.map(tab => tab.id), [tabs])
  // If the tab being edited leaves the model (closed · moved to another window), the edit also ends — a leftover `editingId` blocks matching the head DOM order.
  useEffect(() => {
    if (editingId !== null && !ids.includes(editingId)) setEditingId(null)
  }, [editingId, ids])
  const metaById = new Map(tabs.map(tab => [tab.id, tab]))
  const source = useTabDragSource({
    label,
    tabs: ids,
    active,
    editing: editingId,
    nameOf: id => metaById.get(id)?.name ?? '',
    row: rowRef,
    add: addRef,
    canvas,
    pullFrame,
    titleRow,
  })
  const { snapshot } = source
  const model = { tabs: ids, active }
  const mine = snapshot.window === label
  // The painted active = the tab whose canvas this window shows (including the R15 remaining tab · the tab waiting to be reverted — same value as `WindowLayout`).
  const shown = mine ? shownTab(snapshot.override, model) : active
  const motion = mine && stripHeld(snapshot.override, model) ? snapshot.motion : null
  const dragged =
    mine && (snapshot.override.phase === 'live' || snapshot.override.phase === 'ended') ? snapshot.override.view : null

  const order = motion === null ? ids : placeAt(ids, motion.view, motion.index)
  const at = motion === null ? -1 : order.indexOf(motion.view)
  const draggedAt = dragged === null ? -1 : order.indexOf(dragged)
  const bounds = stripBounds({
    available,
    count: order.length,
    active: motion !== null ? at : shown === null ? -1 : order.indexOf(shown),
    dragged: draggedAt >= 0 ? draggedAt : undefined,
    widthCap: WIDTH_CAP,
  })
  useLayoutEffect(() => {
    source.drawn.current = { order, bounds, available }
  })
  // Outside a drag too, the position-transition property stays `transform` and only the duration is 0 — by CSS Transitions rules, if only the duration changes, a transition in
  //   progress (the dragged tab · neighbors going to their position after drop) runs to the end, while removing the property cuts it on the spot (measured on WebView2 — M-run part 1 C2).
  //   Keep the duration until the frame where the drag is released — heads returning to the model order after cancel · rejection slide there.
  const dragOwned = mine && snapshot.override.phase !== 'idle'
  const wasDragOwned = useRef(false)
  useLayoutEffect(() => {
    wasDragOwned.current = dragOwned
  })
  const slide = `transform ${dragOwned || wasDragOwned.current ? slideMs(reducedMotion(), false) : 0}ms ${EASE_OUT_CSS}`
  const targets = motion === null ? null : slideTargets(bounds, at)
  const heads: DrawnHead[] = []
  order.forEach((id, i) => {
    let x =
      motion === null || targets === null
        ? bounds.visible[i]
          ? bounds.tabs[i].x0
          : null
        : i === at
          ? motion.settled
            ? bounds.tabs[i].x0
            : motion.left
          : targets[i]
    if (x === null) return
    let w = bounds.tabs[i].x1 - bounds.tabs[i].x0
    // A head being name-edited is widened to the standard width and drawn over its neighbors — in a narrow head, the name field ·
    //   input field did not fit inside the `overflow: hidden` of the head content area (measured — with the old ≈ 11 px close button, at head widths 42 · 62 · 86 px they came to 0 · 19 · 43 px · it was not
    //   re-measured with the 22 px circle). Pull it left so the right end does not exceed the usable width, but stop at the left end of the row (0) — if the usable width is
    //   narrower than the standard width, the head can run 0..160 past that width and cover `+`, and if hidden tabs put `+` inside the usable width, it can
    //   cover part of `+`. A press on a covered neighbor head = `coveredHead`.
    if (motion === null && id === editingId && w < TAB_STANDARD_W) {
      w = TAB_STANDARD_W
      if (available !== null) x = Math.max(0, Math.min(x, available - w))
    }
    const following = motion !== null && i === at && !motion.settled
    heads.push({ id, x, w, transition: following ? 'none' : slide })
  })
  // `+` position — the same value as the right end of the tab bar zone (`stripZoneArea`).
  const end = tabsEnd(bounds)
  const marker = useLocalDrag(s => s.targetMarker)

  const domOrder = useDomOrder(
    heads.map(h => h.id),
    source.engaged,
    editingId !== null,
    rowRef,
  )
  const headById = new Map(heads.map(h => [h.id, h]))

  const activate = (view: string, button: number) => {
    const switching = button === 0 && metaById.has(view) && (view !== active || source.forceSwitch(view))
    activatedBy.current = switching ? view : null
    if (switching) onSwitch(view)
  }
  // A press in the area covered by the head (`id`) widened during name edit is a press on the head under it — if not passed on, a covered neighbor cannot be pressed until
  //   the edit ends. `null` where there is no head under its own pre-widening area (`+` · empty row).
  const coveredHead = (id: string, clientX: number): string | null => {
    const row = rowRef.current
    if (motion !== null || row === null) return null
    const x = clientX - row.getBoundingClientRect().left
    const under = (i: number) => bounds.visible[i] && x >= bounds.tabs[i].x0 && x < bounds.tabs[i].x1
    if (under(order.indexOf(id))) return null
    const j = order.findIndex((_, i) => under(i))
    return j < 0 ? null : order[j]
  }

  // Even after the order move is withdrawn, it is drawn over neighbors while going to its position — kept until the next drag (no difference when they do not overlap).
  const lastDragged = useRef<string | null>(null)
  useLayoutEffect(() => {
    if (motion !== null) lastDragged.current = motion.view
  })
  const topId = motion?.view ?? lastDragged.current

  // While an order move is drawn, hover is not painted — the hover from the press moment stays visible on a head that steps aside (whether the head
  //   lifts up during an OS drag is untested — TRD §2-4 · M11).
  const [hovered, setHovered] = useState<string | null>(null)
  const activeId = shown
  const lit = (id: string) => id === activeId || (motion === null && id === hovered)
  // A head within `SEPARATOR_FADE_MS` of its hover ending — only the separators on both its sides come back gradually (`SEPARATOR_FADE_MS` head comment).
  const [hoverEnding, setHoverEnding] = useState<ReadonlySet<string>>(() => new Set())
  const hoverEndTimers = useRef(new Map<string, ReturnType<typeof setTimeout>>())
  useEffect(() => {
    const timers = hoverEndTimers.current
    return () => timers.forEach(clearTimeout)
  }, [])
  const endHover = (id: string) => {
    setHovered(h => (h === id ? null : h))
    clearTimeout(hoverEndTimers.current.get(id))
    setHoverEnding(s => new Set(s).add(id))
    hoverEndTimers.current.set(
      id,
      setTimeout(() => {
        hoverEndTimers.current.delete(id)
        setHoverEnding(s => {
          const next = new Set(s)
          next.delete(id)
          return next
        })
      }, SEPARATOR_FADE_MS),
    )
  }
  // The head that puts a separator on its left edge — one between each pair of drawn neighbors. It is visible only when neither head on either side is active · hovered,
  //   and otherwise it disappears (not removed — the transition of fading in must show when it reappears) (Chrome).
  const leftNeighbour = new Map(heads.slice(1).map((h, k) => [h.id, heads[k].id]))
  const separatorShown = (id: string) => !lit(leftNeighbour.get(id)!) && !lit(id)
  const separatorFadesIn = (id: string) => hoverEnding.has(id) || hoverEnding.has(leftNeighbour.get(id)!)
  // One on the right edge of the last drawn head too — without it, the × of the last inactive head and + looked stuck together (ADR-0298 decision 4).
  //   It hangs inside the head, so the row width · + position stay the same. Visible · fading = looks at that one head only.
  const trailingId = heads.length > 0 ? heads[heads.length - 1].id : null

  return (
    <div
      data-testid="tab-bar"
      data-window-label={label}
      style={{
        display: 'flex',
        alignItems: 'stretch',
        height: TAB_BAR_H,
        // In the title row only the tab bar shrinks — when the row narrows, the measured width (`useStripAvailable`) shrinks and tabs shrink first (TRD S26-window-chrome §7-1).
        flex: '1 1 0px',
        minWidth: 0,
        // Safeguard — even when the row is narrower than the heads · `+`, nothing is drawn over what is to its right (empty draggable area · number badge · window buttons). Clip horizontally only —
        //   `hidden` also clips vertically and would erase the active surface · flare (`bottom: -1`) that straddle the 1px line under the tab bar.
        overflowX: 'clip',
        // ★Surface rule = the active tab shares the surface of the content (`--tab-active-bg`), so it continues into the canvas below, while the bar (`--tab-bar-bg`) is a surface whose
        //   brightness differs from the content★ — Chrome style (user decision 2026-10-19 · values per theme in `styles/theme.css`). Light · e-ink = the bar is a surface
        //   one step darker than the content · dark = the bar is lighter than the content so the active tab is the darkest (Zed Ayu Dark style — ADR-0298 decision 1).
        //   ☞ A layout with the active tab darker than the bar was reverted as 「활성 탭이 구멍처럼 읽힌다」 (user judgment 2026-08-23), and that judgment
        //   stays for light · e-ink — for dark, ADR-0298 reversed it (with a #000 bar the active tab was invisible). The option of mixing in `--text`
        //   to lift the bar became a gray band at odds with the tree panel, so it was reverted (user judgment 2026-08-23).
        background: 'var(--tab-bar-bg)',
        // 1px line under the bar — broken only where the active surface · flare covers it. Only in e-ink does the line separate bar and content; in dark · light the surface
        //   brightness difference separates them, so it is transparent (the line's space remains so the row height is unchanged). If the border is removed the row height grows by 1px, so the active surface's
        //   `bottom: -1` must be fixed together.
        borderBottom: '1px solid var(--tab-bar-edge)',
        fontFamily: 'var(--font-ui)',
        fontSize: '12px',
        userSelect: 'none',
      }}
    >
      <div
        ref={rowRef}
        data-testid="tab-strip-row"
        // Empty space between tabs · after the last tab = window draggable area (itself only — not the heads · `+` · TRD S26-window-chrome §7-2).
        {...dragProps()}
        style={{ position: 'relative', flex: '1 1 0', minWidth: 0 }}
      >
        {domOrder.map(id => {
          const head = headById.get(id)!
          // `placeAt` also puts a dragged tab that is not in the model into the row — that head draws only position · width, without a name. The order move is drawn only when the tab list at drag start
          //   equals the model (`stripHeld`), so for now it appears unreachable (untested).
          const tab = metaById.get(id) ?? { id, name: '', named: false }
          const isActive = id === activeId
          const editing = editingId === id
          // Name · × share one foreground color — when active · hovered, `--tab-text` (= `--text` · ADR-0298 decision 8). One foreground color is Chrome style (`tab.cc`
          //   — same foreground for tab title and close button), and hover → `--text` is our choice (Chrome keeps a separate hover foreground —
          //   `kColorTabForegroundInactiveHoverFrameActive` · `tab_style.cc`). The two tokens are swapped by the title row for the dimmed look
          //   when the window is not the front window (`styles/theme.css` · TRD S26-window-chrome §7-5).
          const fg = lit(id) ? 'var(--tab-text)' : 'var(--tab-text-inactive)'
          return (
            <div
              key={id}
              data-testid="tab"
              data-view-id={id}
              data-active={isActive ? 'true' : 'false'}
              draggable={source.draggable(id)}
              // The press is recorded in the capture phase — even if the close button stops bubbling, a drag started there can be told apart, and since it comes before the switch (the bubble
              //   handler below), the active tab before the press becomes that drag's `initial`.
              onPointerDownCapture={e => source.onPointerDown(e, id)}
              // Active at the moment of press (TRD §2-3 ①). A press on a head being edited is not a drag candidate, and a press on a covered neighbor's area commits the edit and
              //   only switches to that neighbor.
              onPointerDown={e => {
                if (editing) {
                  const inInput = e.target instanceof Element && e.target.closest('input') !== null
                  const under = e.button === 0 && !inInput ? coveredHead(id, e.clientX) : null
                  if (under === null) return
                  commitEdit(tab, false)
                  activate(under, e.button)
                  return
                }
                activate(id, e.button)
              }}
              onDragStart={e => source.onDragStart(e, id)}
              onDragEnd={e => source.onDragEnd(e, id)}
              // Anywhere on the head (including a narrow active head whose name field width is 0) — over the close button only when it is a click that button does not close.
              onDoubleClick={e => {
                if (editing) return
                if (e.target instanceof Element && e.target.closest('button') !== null && activatedBy.current !== id) return
                beginEdit(tab)
              }}
              onPointerEnter={() => setHovered(id)}
              onPointerLeave={() => endHover(id)}
              style={{
                position: 'absolute',
                top: 0,
                bottom: 0,
                left: 0,
                width: head.w,
                transform: `translateX(${head.x}px)`,
                transition: head.transition,
                zIndex: editing ? 2 : id === topId ? 1 : undefined,
                boxSizing: 'border-box',
                cursor: 'pointer',
                color: fg,
              }}
            >
              {/* The surface · flare · separator only draw (`pointer-events: none`) — the head box (press · grab decision) is unchanged, and even if a flare sticks out over a neighbor
                  head, a press there belongs to the neighbor. ★Do not bring back the accent underline★ — the 2px blue line stood out most in the frame and was the main culprit
                  of the "high contrast" (user judgment 2026-08-23, compared with Orca). */}
              {isActive ? (
                <>
                  <span aria-hidden data-testid="tab-surface" style={ACTIVE_SURFACE} />
                  <span aria-hidden data-testid="tab-flare-left" style={FLARE_LEFT} />
                  <span aria-hidden data-testid="tab-flare-right" style={FLARE_RIGHT} />
                </>
              ) : editing ? (
                // Hide it so the neighbor's text covered by the widened edit head does not show through.
                <span aria-hidden style={EDIT_COVER} />
              ) : (
                lit(id) && <span aria-hidden data-testid="tab-hover" style={HOVER_SURFACE} />
              )}
              {leftNeighbour.has(id) && (
                <span aria-hidden data-testid="tab-separator" style={separatorStyle(separatorShown(id), separatorFadesIn(id))}>
                  <span style={SEPARATOR_LINE} />
                </span>
              )}
              {id === trailingId && (
                <span
                  aria-hidden
                  data-testid="tab-separator-trailing"
                  style={{ ...separatorStyle(!lit(id), hoverEnding.has(id)), left: undefined, right: -SEPARATOR_SLOT / 2 }}
                >
                  <span style={SEPARATOR_LINE} />
                </span>
              )}
              <div style={HEAD_CONTENT}>
                {editing ? (
                  <input
                    data-testid="tab-rename-input"
                    data-view-id={id}
                    value={draft}
                    autoFocus
                    // Calling select() from an inline callback ref here would rerun on every render and break typing (FIX 1).
                    ref={inputRef}
                    onChange={e => setDraft(e.target.value)}
                    onKeyDown={e => {
                      // ★Bubble block★: a key during editing must not leak into the global key bindings.
                      e.stopPropagation()
                      if (e.key === 'Enter') commitEdit(tab, true)
                      else if (e.key === 'Escape') cancelEdit()
                    }}
                    onBlur={() => commitEdit(tab, false)}
                    style={{
                      // ★Fit to content width★: with field-sizing:content the input takes only the width of the draft text
                      //   (a default input is fixed at the size attribute width ~160px, so it spreads wide even for a short name). The browser
                      //   measures the actual glyph width, so CJK · proportional fonts are exact too. Supported by WebView2 (Chromium 123+).
                      //   minWidth only guards against being too narrow when empty / one character, maxWidth is the upper limit.
                      fieldSizing: 'content',
                      minWidth: '3ch',
                      maxWidth: '160px',
                      font: 'inherit',
                      color: 'var(--text)',
                      background: 'var(--bg)',
                      border: '1px solid var(--accent)',
                      borderRadius: '2px',
                      padding: '0 2px',
                      outline: 'none',
                    }}
                  />
                ) : (
                  <span
                    data-testid="tab-name"
                    style={{ flex: '1 1 auto', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis' }}
                  >
                    {tab.name}
                  </span>
                )}
                {/* Like Chrome, an inactive head narrower than the active minimum width hides the close button (`layout.ts` `TAB_MIN_ACTIVE_W`). */}
                {(isActive || head.w >= TAB_MIN_ACTIVE_W) && (
                  <button
                    type="button"
                    aria-label={t('tab.close', { name: tab.name })}
                    data-testid="tab-close"
                    // A close button press is not a head press (switch · drag).
                    onPointerDown={e => e.stopPropagation()}
                    // The close button of the tab being name-edited does not take input field focus on press — if it did, the blur commit would redraw the head
                    //   (the widened head shrinks · the button right after the input field moves to the head's right end), the button would slip out from under the cursor, and that
                    //   click could not close. A press on another tab's close button commits the edit as usual.
                    onMouseDown={editing ? e => e.preventDefault() : undefined}
                    onClick={e => {
                      // In the same run of clicks (detail ≥ 2), the earlier press made this tab active and this button just appeared under the cursor — do not close
                      //   (double-clicking an inactive head in a narrow tab bar closed the tab 4/7 — M-run part 1 anomaly 3). The close button of the
                      //   next tab pushed in while closing by repeated clicks does close.
                      if (e.detail >= 2 && activatedBy.current === id) return
                      activatedBy.current = null
                      onClose(id)
                    }}
                    className={BUTTON_HOVER}
                    style={{ ...CLOSE_STYLE, color: fg }}
                  >
                    <X aria-hidden size={BUTTON_ICON} strokeWidth={2} />
                  </button>
                )}
              </div>
            </div>
          )
        })}
        <button
          ref={addRef}
          type="button"
          aria-label={t('tab.create')}
          data-testid="tab-add"
          onClick={onCreate}
          className="group/add"
          style={{ ...ADD_STYLE, transform: `translateX(${end}px)`, transition: slide }}
        >
          <span aria-hidden data-testid="tab-add-circle" className={ADD_HOVER} style={ADD_CIRCLE}>
            <Plus size={BUTTON_ICON} strokeWidth={2} />
          </span>
        </button>
        {marker !== null && (
          <span
            aria-hidden
            data-testid="tab-drop-marker"
            style={{ ...MARKER_STYLE, transform: `translateX(${marker - MARKER_W / 2}px)` }}
          />
        )}
      </div>
    </div>
  )
}

// Close · `+` buttons = a 13 px icon inside a `TAB_BUTTON_SIZE`(22) px circle (ADR-0298 decision 5 · user decision 2026-10-21). `+` circle = Chrome
//   `tab_strip_control_button.cc` `kButtonSize` 28 scaled by the head-height ratio 28/34 (the calculation is ≈ 23 — 22 is the user's decision). × circle =
//   the same 22 — a user decision to make it larger than the visible size of Chrome's close button (hover shape = `layout_constants.cc` `kTabCloseButtonSize` 16 dp · `tab.cc`
//   #545-548), so do not shrink it by the Chrome ratio. Both icons = Chrome `tab_strip_control_button.cc`
//   `kIconSize` 16 × 28/34 ≈ 13. Hover circle = Chrome ink drop highlight opacity 0.16 (`--tab-button-hover-bg`).
const BUTTON_ICON = 13
// ★Keep class names as whole strings★ — Tailwind scans source text to generate classes, so joining fragments produces none. Do not put the hover
//   color in inline `background` — inline beats the class and the hover circle disappears.
const BUTTON_HOVER = 'bg-transparent hover:bg-[var(--tab-button-hover-bg)]'
const ADD_HOVER = 'group-hover/add:bg-[var(--tab-button-hover-bg)]'

const CLOSE_STYLE: CSSProperties = {
  flexShrink: 0,
  width: TAB_BUTTON_SIZE,
  height: TAB_BUTTON_SIZE,
  borderRadius: '50%',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  padding: 0,
  border: 'none',
  cursor: 'pointer',
}

// Do not add horizontal margin — the usable width is row width − the `+` border box width (`measure.ts`). The gap before the circle is therefore left padding, not margin
//   (box = gap 4 + circle 22). The hover circle is painted only on the circle inside, not on the box (Tailwind `group-hover/add`).
const ADD_STYLE: CSSProperties = {
  position: 'absolute',
  top: SURFACE_TOP,
  bottom: 0,
  left: 0,
  display: 'flex',
  alignItems: 'center',
  background: 'transparent',
  border: 'none',
  // Same token as the resting gray of ×.
  color: 'var(--tab-text-inactive)',
  cursor: 'pointer',
  padding: `0 0 0 ${TAB_ADD_GAP}px`,
}

const ADD_CIRCLE: CSSProperties = {
  width: TAB_BUTTON_SIZE,
  height: TAB_BUTTON_SIZE,
  borderRadius: '50%',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
}

const REDUCED_MOTION = '(prefers-reduced-motion: reduce)'

/** Chrome's rich animations off — step-aside time 0 (`reorderGeometry.slideMs`). */
function reducedMotion(): boolean {
  return typeof window.matchMedia === 'function' && window.matchMedia(REDUCED_MOTION).matches
}

/** The order made by removing `view` from `ids` and inserting it at position `index` of that list (out of range = the nearest end). */
function placeAt(ids: readonly string[], view: string, index: number): string[] {
  const others = ids.filter(id => id !== view)
  const k = Math.min(Math.max(index, 0), others.length)
  return [...others.slice(0, k), view, ...others.slice(k)]
}

// Active tab shape = exactly Chrome (PRD R27 · ADR-0295 decision 1 · Chromium `tab_style.cc` `GetTopCornerRadius` ·
//   `GetBottomCornerRadius`) — the top corner radius and the bottom outer flare radius are different values.
const CORNER_RADIUS = 10
const FLARE_RADIUS = 12

// The flare (`FLARE_*`) sticks out of the head, so the head itself has no `overflow: hidden` — clipping is done by this content area.
const HEAD_CONTENT: CSSProperties = {
  position: 'absolute',
  top: SURFACE_TOP,
  bottom: 0,
  left: 0,
  right: 0,
  boxSizing: 'border-box',
  overflow: 'hidden',
  display: 'flex',
  alignItems: 'center',
  gap: TAB_HEAD_GAP,
  padding: `0 ${TAB_HEAD_PAD}px`,
  whiteSpace: 'nowrap',
}

// `bottom: -1` = also covers the border (1px) under the bar so it joins the content without a break.
const ACTIVE_SURFACE: CSSProperties = {
  position: 'absolute',
  top: SURFACE_TOP,
  bottom: -1,
  left: 0,
  right: 0,
  borderRadius: `${CORNER_RADIUS}px ${CORNER_RADIUS}px 0 0`,
  background: 'var(--tab-active-bg)',
  pointerEvents: 'none',
}

// Concave flares at the two bottom corners of the active surface — in a square of the outside-radius width outside the head, paint only outside the circle centered on the outer top corner.
const flare = (side: 'left' | 'right'): CSSProperties => ({
  position: 'absolute',
  bottom: -1,
  left: side === 'left' ? -FLARE_RADIUS : undefined,
  right: side === 'right' ? -FLARE_RADIUS : undefined,
  width: FLARE_RADIUS,
  height: FLARE_RADIUS,
  background: `radial-gradient(circle at ${side === 'left' ? '0 0' : '100% 0'}, transparent ${FLARE_RADIUS - 0.5}px, var(--tab-active-bg) ${FLARE_RADIUS}px)`,
  pointerEvents: 'none',
})
const FLARE_LEFT = flare('left')
const FLARE_RIGHT = flare('right')

// Leave the bottom 2px empty — so the hover surface does not cover the flare of the neighboring active tab.
const HOVER_SURFACE: CSSProperties = {
  position: 'absolute',
  top: SURFACE_TOP,
  bottom: 2,
  left: 2,
  right: 2,
  borderRadius: '6px',
  background: 'var(--tab-hover-bg)',
  pointerEvents: 'none',
}

const EDIT_COVER: CSSProperties = {
  position: 'absolute',
  inset: 0,
  background: 'var(--tab-bar-bg)',
  pointerEvents: 'none',
}

// Insertion position for a drag from another window — VS Code's 2 px vertical line (TRD §4-3 · W13 · U2 ② straddling the ends of both heads by 1 px each). x = tab bar coordinates
//   (`localDrag` `targetMarker` — written by that window's root dragover).
const MARKER_W = 2
const MARKER_STYLE: CSSProperties = {
  position: 'absolute',
  top: SURFACE_TOP,
  bottom: 0,
  left: 0,
  width: MARKER_W,
  background: 'var(--accent)',
  pointerEvents: 'none',
  zIndex: 3,
}

// Separator = Chrome style (PRD R27 · Chromium `tab_style.cc` `kSeparator*` · `horizontal_tab_style_views.cc`
//   `GetSeparatorBounds`) — side margin 2 · rounding = half the thickness · top and bottom = the center of the head height unchanged, and ★thickness 1 · height 13 is
//   ADR-0298 decision 3★ (thickness = thinner than Chrome's 2 · height = the Chrome ratio of tab 34 to separator 16 applied to our head 28). The margin ·
//   thickness · margin area is hung at the middle of the boundary with the neighbor head — Chrome puts that area in the region where tabs overlap (18), but our heads do not
//   overlap (`tabStrip/layout.ts` head).
const SEPARATOR_W = 1
const SEPARATOR_H = 13
const SEPARATOR_MARGIN = 2
const SEPARATOR_SLOT = SEPARATOR_MARGIN + SEPARATOR_W + SEPARATOR_MARGIN
// Disappearing · appearing = exactly Chrome source (PRD R27 · Q23) — the separator next to a tab whose background is visible (active ·
//   hovered) disappears immediately (0 is produced before interpolating the hover value — `horizontal_tab_style_views.cc`
//   `GetSeparatorOpacity`). Fading is only for coming back, and only on both sides of a head whose hover has ended — separator opacity = 1 − that head's
//   hover value (`GetHoverInterpolatedSeparatorOpacity`), so it appears while the hover value falls (`glow_hover_controller.h` default 200 ms ·
//   `Tween::EASE_IN` when turning off). A separator that comes back because the active tab moved appears immediately.
const SEPARATOR_FADE_MS = 200
// `Tween::EASE_IN` `t²` (tween.cc#37-38) in CSS — the same form as `EASE_OUT_CSS`.
const EASE_IN_CSS = 'cubic-bezier(0.3333, 0, 0.6667, 0.3333)'
const SEPARATOR_SLOT_STYLE: CSSProperties = {
  position: 'absolute',
  top: 0,
  bottom: 0,
  left: -SEPARATOR_SLOT / 2,
  width: SEPARATOR_SLOT,
  boxSizing: 'border-box',
  padding: `0 ${SEPARATOR_MARGIN}px`,
  display: 'flex',
  alignItems: 'center',
  pointerEvents: 'none',
}
const SEPARATOR_LINE: CSSProperties = {
  width: SEPARATOR_W,
  height: SEPARATOR_H,
  flexShrink: 0,
  borderRadius: SEPARATOR_W / 2,
  background: 'var(--tab-separator)',
}
const separatorStyle = (shown: boolean, fadesIn: boolean): CSSProperties => ({
  ...SEPARATOR_SLOT_STYLE,
  opacity: shown ? 1 : 0,
  transition: shown && fadesIn ? `opacity ${SEPARATOR_FADE_MS}ms ${EASE_IN_CSS}` : undefined,
})

/**
 * Width the heads can use (row width − `+` width — the same formula the hook measures at drag time) — re-measured every time the row size changes. If the row height is 0,
 * there is no layout (before measuring · jsdom · `display: none`) — `null` = no limit. ★Width 0 is a measured value★ — if the window is
 * narrower than the title row's minimum width, the row width becomes 0, and treating that as `null` would stand every head at the standard width and cover the empty draggable area · number badge · window buttons.
 */
function useStripAvailable(
  row: RefObject<HTMLElement | null>,
  add: RefObject<HTMLElement | null>,
): number | null {
  const [available, setAvailable] = useState<number | null>(null)
  useLayoutEffect(() => {
    const el = row.current
    if (el === null) return
    const measure = () => {
      const m = measureStrip({ row: el, add: add.current })
      setAvailable(m.row.y1 > m.row.y0 ? m.available : null)
    }
    measure()
    if (typeof ResizeObserver === 'undefined') return
    const ro = new ResizeObserver(measure)
    ro.observe(el)
    return () => ro.disconnect()
  }, [row, add])
  return available
}

/**
 * Head DOM order (head comment 「DOM 순서」) — at rest it is the visible order (`visual`); otherwise the heads already drawn stay in place and a new head is
 * appended at the end. At rest = no name edit, the drag hook has released the press (`engaged`), and after that all the tab bar's transitions have ended.
 */
function useDomOrder(
  visual: string[],
  engaged: boolean,
  editing: boolean,
  row: RefObject<HTMLElement | null>,
): string[] {
  // `false` while the drag hook holds the press · `true` once it is released and transitions have finished and it is time to match.
  const [synced, setSynced] = useState(true)
  const committed = useRef<string[]>([])

  useEffect(() => {
    if (engaged) {
      setSynced(false)
      return
    }
    if (synced || editing) return
    let live = true
    const step = (): void => {
      if (!live) return
      // `getAnimations` updates styles first, so it also counts the position transition just drawn (Web Animations spec · not measured on WebView2). One that ended and
      //   remains as fill has `finished` already resolved, so counting it makes the wait spin in place. jsdom has none (= no transition).
      const running = (row.current?.getAnimations?.({ subtree: true }) ?? []).filter(
        a => a.playState === 'running' || a.pending,
      )
      if (running.length === 0) setSynced(true)
      else void Promise.allSettled(running.map(a => a.finished)).then(step)
    }
    // Match after the click following the release has gone — if heads are moved in between (detach and reattach), the click may not reach the head and double-click rename
    //   may break (not measured on WebView2).
    const timer = setTimeout(step, 0)
    return () => {
      live = false
      clearTimeout(timer)
    }
  }, [engaged, editing, synced, row])

  let order: string[]
  if (synced && !engaged && !editing) {
    order = visual
  } else {
    const drawn = new Set(visual)
    order = committed.current.filter(id => drawn.has(id))
    const kept = new Set(order)
    for (const id of visual) if (!kept.has(id)) order.push(id)
  }
  useLayoutEffect(() => {
    committed.current = order
  })
  return order
}
