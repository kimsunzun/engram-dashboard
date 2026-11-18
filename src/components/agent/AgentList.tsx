//! ★react-arborist revival (ADR-0072)★: children nest under their parent via parent_id (one level). Drag calls onMove → reparentProfile (§5 — human drag and LLM calls share the handle). No optimistic update: the backend broadcast
//!   redraws the list (same as rename). Row click, double-click, right-click menu, inline rename and the in-flight guard behave as in the flat list, inside NodeRenderer.
//!
//! ★State = shape, not color (icon)★ (ADR-0062): state stays distinguishable on e-ink (monochrome) by choosing the shape via statusGlyphIcon, not color literals. ◐ (awaiting input) is vocabulary only (no backend signal, never lit).
//!
//! ★Display name = display_name override ?? cwd basename (frontend-derived, ADR-0061)★ (shared basename util). cwd is not shown (name only).
//!
//! ★Owns only the ROW right-click menu (ADR-0064)★: a pane-background right-click bubbles to the parent unified slot menu (agent_list "create agent" = agentlist.createAgent command + common slot ops); only a row right-click is intercepted
//! with stopPropagation to show the item-targeted menu (activate/open/kill/rename/delete). Actions flow only through agentClient / viewStore (backend-authoritative invoke→emit) — no raw invoke/ptyApi (ADR-0011).
//! Restart has no matching command → disabled as "coming soon".

import { useEffect, useRef, useState } from 'react'
import { Tree, type NodeRendererProps } from 'react-arborist'

import { agentClient } from '../../api/clientFactory'
import { useAgentStore } from '../../store/agentStore'
import { currentViewId, selectView, useViewStore } from '../../store/viewStore'
import { refreshProfiles } from '../../store/eventBus'
import { basename } from '../../util/basename'
import { mergeTreeNodes, type AgentTreeNode } from './mergeTreeNodes'
import { selectOpenTarget } from './selectOpenTarget'
import { isFailureBlocked, rowGlyphColor, rowGlyphIcon, rowPhase } from './statusGlyph'
import { failureLine } from './failureKinds'
import { useBlurOnDragRegionPress } from '../layout/titleRow/dragRegion'
import { t } from '../../i18n'
// Rotation of the "waiting for an answer" glyph (class `engram-glyph-spin`). The stylesheet owns the keyframes.
import './agentGlyph.css'

// The SSOT for the status mapping is statusGlyph.ts alone (split out so it can be measured headless without pulling in react-arborist); this re-export is only for existing `from './AgentList'` importers.
export { statusGlyphColor, statusGlyphIcon } from './statusGlyph'

/** Row right-click menu — react-arborist virtualization can unmount rows, so it holds only a primitive snapshot, not a NodeApi. */
type RowMenu = {
  x: number
  y: number
  agentId: string
  kind: 'running' | 'reserved'
}

/** A row's action failure, untranslated — the message is made where shown (`rowErrorText`) so it follows a language switch (ADR-0373). `err` is the raw error as text. */
type RowError =
  | { key: 'agent.openFailedNoSlot' | 'agent.openFailedNoEmptySlot' }
  | {
      key:
        | 'agent.activateFailed'
        | 'agent.openFailed'
        | 'agent.killFailed'
        | 'agent.cancelReservedFailed'
        | 'agent.renameFailed'
        | 'agent.reparentFailed'
      err: string
    }

function rowErrorText(error: RowError): string {
  return 'err' in error ? t(error.key, { err: error.err }) : t(error.key)
}

/** react-arborist row height and indent (px). */
const ROW_HEIGHT = 28
const INDENT = 12

export default function AgentList() {
  const rowMenuRef = useRef<HTMLDivElement>(null)
  const [rowMenu, setRowMenu] = useState<RowMenu | null>(null)
  // ★react-arborist (react-window) requires explicit width/height★: track the container's measured size with ResizeObserver; jsdom has no ResizeObserver, so the initial value is non-zero and rows draw from the first render (tests observe data-agent-row).
  const treeContainerRef = useRef<HTMLDivElement>(null)
  const [dimensions, setDimensions] = useState({ width: 240, height: 400 })
  // ★ref = authoritative double-fire guard, state (busyIds) = visuals (disabled/opacity)★: a useState guard alone lets a fast double-click pass twice before the re-render commits (stale closure with busyIds empty);
  //   a ref is synchronously mutable and blocks the same-tick second call.
  const busyRef = useRef<Set<string>>(new Set())
  const [busyIds, setBusyIds] = useState<Set<string>>(new Set())
  // Action failure message — there is no toast/StatusBar, so it shows inline next to the row.
  const [errorById, setErrorById] = useState<Record<string, RowError>>({})
  // ★Rows whose activation was refused★ — the answer this screen received directly, not a stand-in for the "last failure" record: the backend optimistically raises status to Running once a session exists and leaves it until the process death is observed,
  //   so green stayed for seconds after the refusal was in hand; this set raises the forbidden marker immediately.
  //   ★Not merged with errorById★: that one holds any action's failure (kill/open/rename), so merging would make a row whose rename failed an "unopenable" item. Only the activation path writes here.
  const [activationRejectedIds, setActivationRejectedIds] = useState<Set<string>>(new Set())

  // ★Inline-edit local state (frontend-only draft, separate from the backend-authoritative name)★:
  //   renameProfile only on commit (Enter/blur).
  const [editingId, setEditingId] = useState<string | null>(null)
  const [draft, setDraft] = useState('')
  // ADR-0373: the name the edit started from — the "unchanged" check compares with this, not with a name derived again at
  //   commit, which can change under the edit (a language switch turns a blank-path name into another language's word).
  const [draftStart, setDraftStart] = useState('')
  // ★Stable ref★: select() exactly once on entering edit — an inline callback ref re-attaches every render and breaks typing.
  const inputRef = useRef<HTMLInputElement>(null)
  useEffect(() => {
    if (editingId !== null) inputRef.current?.select()
  }, [editingId])
  // Pressing the window-drag area (title row) also ends the edit — commit stays on onBlur.
  useBlurOnDragRegionPress(editingId !== null, inputRef)

  const agents = useAgentStore(s => s.agents)
  const profiles = useAgentStore(s => s.profiles)
  const selectedAgentId = useAgentStore(s => s.selectedAgentId)
  const setSelectedAgent = useAgentStore(s => s.setSelectedAgent)
  // ★Draw nothing in the tree area until both have arrived★: an empty array cannot tell "not received yet" from "really empty" (a new window briefly showed the empty-list notice). Drawing from only one is wrong — display name and parent grouping come
  //   from profiles, and without the roster a running agent looks reserved. A daemon connection failure is shown by the window-level `ConnectionNotice` (the two flags of ADR-0148).
  const listLoaded = useAgentStore(s => s.agentsLoaded && s.profilesLoaded)

  const forest: AgentTreeNode[] = mergeTreeNodes(profiles, agents)
  // Flattened for tree presence/absence and menu staleness checks (one level, so roots + each children). react-arborist walks the forest directly.
  const flatRows: AgentTreeNode[] = forest.flatMap(n => [n, ...n.children])

  useEffect(() => {
    const el = treeContainerRef.current
    if (!el || typeof ResizeObserver === 'undefined') return
    const ro = new ResizeObserver(entries => {
      const { width, height } = entries[0].contentRect
      // Ignore 0 (before layout / at unmount) — keep the last valid size.
      if (width > 0 && height > 0) setDimensions({ width: Math.floor(width), height: Math.floor(height) })
    })
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  // Close on outside click of the row menu (mousedown outside its own ref closes it). Clicks inside the container are exempt so an
  //   item's mousedown does not close the menu first and swallow onClick (same guard as SlotContextMenu).
  useEffect(() => {
    if (!rowMenu) return
    const h = (e: MouseEvent) => {
      const t = e.target as Node
      if (rowMenuRef.current && !rowMenuRef.current.contains(t)) setRowMenu(null)
    }
    // Capture phase — pressing the window-drag area never reaches the bubble phase (`layout/titleRow/dragRegion.ts`).
    document.addEventListener('mousedown', h, true)
    return () => document.removeEventListener('mousedown', h, true)
  }, [rowMenu])

  useEffect(() => {
    if (!rowMenu) return
    const h = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setRowMenu(null)
    }
    document.addEventListener('keydown', h)
    return () => document.removeEventListener('keydown', h)
  }, [rowMenu])

  // ★Close the menu when the target disappears★: rowMenu outlives its row (kill/deleteProfile/last agent removed → empty transition); once the target leaves flatRows reset to null so a menu aimed at a departed agentId is never rendered.
  useEffect(() => {
    if (rowMenu && !flatRows.some(r => r.id === rowMenu.agentId)) setRowMenu(null)
  }, [rowMenu, flatRows])

  const beginInFlight = (id: string): boolean => {
    if (busyRef.current.has(id)) return false
    busyRef.current.add(id)
    setBusyIds(prev => new Set(prev).add(id))
    return true
  }
  const endInFlight = (id: string) => {
    busyRef.current.delete(id)
    setBusyIds(prev => {
      const next = new Set(prev)
      next.delete(id)
      return next
    })
  }
  const markActivationRejected = (id: string) =>
    setActivationRejectedIds(prev => (prev.has(id) ? prev : new Set(prev).add(id)))
  const clearActivationRejected = (id: string) =>
    setActivationRejectedIds(prev => {
      if (!prev.has(id)) return prev // keep the reference = no needless re-render.
      const next = new Set(prev)
      next.delete(id)
      return next
    })
  const setError = (id: string, error: RowError) => setErrorById(prev => ({ ...prev, [id]: error }))
  const clearError = (id: string) =>
    setErrorById(prev => {
      if (!(id in prev)) return prev
      const next = { ...prev }
      delete next[id]
      return next
    })

  // Activate a reserved node (spawnProfile) — same restore UX as AgentTree.
  // ★Make the row say what the answer says★: double-click → `pending` (spinner) until the answer arrives → on refusal `rejected` (forbidden marker) at once, on success drop the marker and return to the usual status glyph. Green only after activation actually succeeded.
  const activateReserved = (agentId: string) => {
    if (!beginInFlight(agentId)) return
    clearError(agentId)
    // Retry start = the last refusal is no longer the current fact (the spinner takes its place).
    clearActivationRejected(agentId)
    agentClient
      .spawnProfile(agentId, false)
      .then(() => refreshProfiles())
      .catch(e => {
        console.error('[spawnProfile]', e)
        setError(agentId, { key: 'agent.activateFailed', err: String(e) })
        // The very spot that raises the "failed" badge — the glyph switches at the same moment (the two disagreeing was the defect).
        markActivationRejected(agentId)
      })
      .finally(() => endInFlight(agentId))
  }

  // "Open" = assign this agent to the target slot of this window's active view (reuses the existing assign path — same as AgentTree).
  //   ★Target slot = selectOpenTarget (pure function)★: the focused slot by default; when focus is on a control slot (tree, etc.), an unknown content slot, or focus=null it falls back to the first empty slot instead of overwriting (safety net for the LayoutLeaf click-to-focus gate); with no empty slot nothing is assigned (never clobber).
  const openInFocusedSlot = (agentId: string) => {
    if (!beginInFlight(agentId)) return
    const vs = useViewStore.getState()
    const viewId = currentViewId()
    const view = selectView(vs, viewId)
    if (!viewId || !view) {
      setError(agentId, { key: 'agent.openFailedNoSlot' })
      endInFlight(agentId) // release the lock on early return too (prevents a permanent lock)
      return
    }
    const slotId = selectOpenTarget(view.layout, view.focusedSlotId, view.foreignSlots)
    if (!slotId) {
      setError(agentId, { key: 'agent.openFailedNoEmptySlot' })
      endInFlight(agentId)
      return
    }
    clearError(agentId)
    void vs
      .assignAgent(viewId, slotId, agentId)
      .catch(e => {
        console.error('[assignAgent]', e)
        setError(agentId, { key: 'agent.openFailed', err: String(e) })
      })
      .finally(() => endInFlight(agentId))
  }

  const killAgentGuarded = (agentId: string) => {
    if (!beginInFlight(agentId)) return
    clearError(agentId)
    agentClient
      .killAgent(agentId)
      .catch(e => {
        console.error('[kill]', e)
        setError(agentId, { key: 'agent.killFailed', err: String(e) })
      })
      .finally(() => endInFlight(agentId))
  }

  // Cancel reservation (delete) — remove the reserved profile from the daemon (agentClient.deleteProfile); no other UI can delete a stale reserved profile.
  const cancelReserved = (agentId: string) => {
    if (!beginInFlight(agentId)) return
    clearError(agentId)
    agentClient
      .deleteProfile(agentId)
      .then(() => refreshProfiles())
      .catch(e => {
        console.error('[deleteProfile]', e)
        setError(agentId, { key: 'agent.cancelReservedFailed', err: String(e) })
      })
      .finally(() => endInFlight(agentId))
  }

  // ★Drag reparent (ADR-0072 §5 — human drag and LLM calls share reparentProfile)★: react-arborist onMove gives dropParentId (null=root). No optimistic update — ReparentProfile → ProfileListUpdated redraws the forest. An invalid move (cycle/self/2-level/nonexistent parent)
  //   is rejected by the backend with Error → the tree keeps the old layout + inline error. A sync guard blocks duplicate fires from rapid clicks.
  const reparent = (childId: string, parentId: string | null) => {
    const node = flatRows.find(r => r.id === childId)
    const currentParentId = forest.find(root => root.children.some(c => c.id === childId))?.id ?? null
    if (!node || currentParentId === parentId) return
    if (!beginInFlight(childId)) return
    clearError(childId)
    agentClient
      .reparentProfile(childId, parentId)
      .then(() => refreshProfiles())
      .catch(e => {
        console.error('[reparentProfile]', e)
        setError(childId, { key: 'agent.reparentFailed', err: String(e) })
      })
      .finally(() => endInFlight(childId))
  }

  // Display name = display_name override ?? cwd basename (ADR-0061). Starting a rename seeds the draft and `draftStart` with it.
  const displayNameOf = (node: AgentTreeNode): string => node.displayName ?? basename(node.cwd)

  // The row menu is closed by the caller.
  const beginEdit = (node: AgentTreeNode) => {
    const name = displayNameOf(node)
    setEditingId(node.id)
    setDraft(name)
    setDraftStart(name)
  }
  const cancelEdit = () => setEditingId(null)
  // ★Idempotent★: if editingId is not this row, return at once — stops Enter from re-firing commitEdit via unmount→blur (same as TabBar). Sync duplicate-fire blocking is beginInFlight(busyRef).
  // ★refreshProfiles() on success — symmetric with spawnProfile/deleteProfile★: the display name normally arrives via RenameProfile → ProfileListUpdated (no optimistic update), but a missed broadcast (reconnect window, lost event) would leave only this window stale;
  //   activateReserved/cancelReserved already re-pull the authoritative list on success. A normal broadcast re-applies the same full list — harmless and idempotent.
  const commitEdit = (node: AgentTreeNode) => {
    if (editingId !== node.id) return
    const trimmed = draft.trim()
    setEditingId(null)
    if (trimmed.length === 0 || trimmed === draftStart) return
    if (!beginInFlight(node.id)) return
    clearError(node.id)
    agentClient
      .renameProfile(node.id, trimmed)
      .then(() => refreshProfiles())
      .catch(e => {
        console.error('[renameProfile]', e)
        setError(node.id, { key: 'agent.renameFailed', err: String(e) })
      })
      .finally(() => endInFlight(node.id))
  }

  const menuNode = rowMenu ? flatRows.find(r => r.id === rowMenu.agentId) : undefined
  // ADR-0373: `id` is the React key — a label is translated text and must not be one.
  const rowMenuItems: Array<{ id: string; label: string; disabled: boolean; action: () => void }> = !rowMenu
    ? []
    : rowMenu.kind === 'reserved'
      ? [
          {
            id: 'activate',
            label: t('agent.rowActivate'),
            disabled: busyIds.has(rowMenu.agentId),
            action: () => activateReserved(rowMenu.agentId),
          },
          {
            id: 'rename',
            label: t('agent.rowRename'),
            disabled: busyIds.has(rowMenu.agentId) || !menuNode,
            action: () => menuNode && beginEdit(menuNode),
          },
          {
            id: 'cancelReserved',
            label: t('agent.rowCancelReserved'),
            disabled: busyIds.has(rowMenu.agentId),
            action: () => cancelReserved(rowMenu.agentId),
          },
        ]
      : [
          { id: 'open', label: t('agent.rowOpen'), disabled: busyIds.has(rowMenu.agentId), action: () => openInFocusedSlot(rowMenu.agentId) },
          {
            id: 'kill',
            label: t('agent.rowKill'),
            disabled: busyIds.has(rowMenu.agentId),
            action: () => killAgentGuarded(rowMenu.agentId),
          },
          // ad-hoc (running without a profile) also opens the item, but if the backend says no-profile it shows the Error inline (not fabricated).
          {
            id: 'rename',
            label: t('agent.rowRename'),
            disabled: busyIds.has(rowMenu.agentId) || !menuNode,
            action: () => menuNode && beginEdit(menuNode),
          },
          // ★Coming soon (no backend command)★: protocolClient has no dedicated restart command (only kill→re-spawn). Do not fabricate — wire it when a real command exists (ADR-0011).
          { id: 'restart', label: t('agent.rowRestart'), disabled: true, action: () => {} },
        ]

  // NodeRenderer — kept inside the component so the select/busy/error/edit handlers are reached by closure.
  const NodeRenderer = ({ node, style, dragHandle }: NodeRendererProps<AgentTreeNode>) => {
    const data = node.data
    const isReserved = data.kind === 'reserved'
    const isBusy = busyIds.has(data.id)
    const rowError = errorById[data.id]
    const err = rowError === undefined ? undefined : rowErrorText(rowError)
    const hasChildren = data.children.length > 0
    // ADR-0173: wording comes only from the kind table (not hard-coded in the component).
    const failureTitle = isFailureBlocked(data.status, data.lastFailure)
      ? failureLine(data.lastFailure)
      : null
    // ★One verdict (phase) drives glyph, color and hover together★ (ADR-0173's property in the presentation layer): while waiting for an answer (`pending`) no status glyph is drawn; after activation is refused (`rejected`) it goes to the forbidden marker
    //   without waiting for the record or the observed process death. ★Keep a single in-flight channel★: `isBusy` is the existing marker for every request on this row (activate, kill, open, rename, move) — a second ledger could diverge from opacity/cursor.
    const phase = rowPhase(isBusy, activationRejectedIds.has(data.id))
    const glyph = rowGlyphIcon(phase, data.status, data.lastFailure)
    // Hover follows the same phase — an old failure text appearing while waiting would make glyph and words disagree.
    //   Right after a refusal `err` (inline activation failure) is already first, so the reason shows as is.
    const rowTitle =
      phase === 'pending'
        ? t('agent.glyphPending')
        : (err ?? failureTitle ?? (isReserved ? t('agent.doubleClickToActivate') : data.cwd))
    return (
      <div
        ref={dragHandle}
        data-agent-row={data.id}
        // ★Machine-readable channel for selection state★: background color alone is unreadable to CDP/LLM selectors (the "tree selection state" gap of `docs/tracking.md` T-77) and tests miss a color-only regression.
        //   ★Do not put `aria-selected` here★: this div has no role so it is invalid ARIA, and react-arborist already sets `role="treeitem" + aria-selected` on the parent row — if the two differ AT reads the outer value and the wrong state.
        data-selected={selectedAgentId === data.id ? 'true' : 'false'}
        style={{
          ...style, // react-arborist virtualization position/height (top/height). Indent is reflected by paddingLeft below per level.
          display: 'flex',
          alignItems: 'center',
          gap: '6px',
          // Fix the toggle width so a leaf without a toggle still aligns its glyph with its parent.
          paddingLeft: `${12 + node.level * INDENT}px`,
          paddingRight: '12px',
          cursor: isBusy ? 'wait' : 'pointer',
          // ★Do not use `--surface-elevated`★: in light (#ffffff) and e-ink (#f0f0f0) it equals this panel background (`--bg-secondary`), so the selection mark vanishes (only dark differs, so a visual check misses it).
          //   Mixing in `--text` works because that token contrasts with the background by definition, so a difference remains in all three themes.
          background:
            selectedAgentId === data.id
              ? 'color-mix(in srgb, var(--text) 12%, var(--bg-secondary))'
              : 'transparent',
          fontFamily: 'var(--font-ui)',
          fontSize: '12px',
          color: isReserved ? 'var(--text-muted)' : 'var(--text)',
          fontStyle: isReserved ? 'italic' : 'normal',
          opacity: isBusy ? 0.6 : 1,
          userSelect: 'none',
        }}
        onClick={() => setSelectedAgent(data.id)}
        // Running row → no-op (keeps AgentTree behavior).
        onDoubleClick={() => {
          if (data.kind === 'reserved') activateReserved(data.id)
        }}
        // ★Inline-edit keys (Enter commit / Escape cancel) are handled on this row div, not the input★: inside react-window virtualization a conditionally mounted <input>'s onKeyDown does not fire synthetic events — React 19 event delegation fails to attach a keydown
        //   listener to the re-mounted root from the 2nd time on (a react-window+jsdom interaction; click/blur/change are fine). This always-rendered row div registers keydown from the initial mount; input keys bubble here. Only while editing does stopPropagation
        //   stop react-arborist key navigation and global keybindings from leaking (same as the TabBar bubble block).
        onKeyDown={e => {
          if (editingId !== data.id) return
          e.stopPropagation()
          if (e.key === 'Enter') commitEdit(data)
          else if (e.key === 'Escape') cancelEdit()
        }}
        // ★A row whose glyph diverged diverges on hover too (ADR-0173 — the reason is one line on mouse-over)★: same verdict as the glyph (`isFailureBlocked`) so they cannot disagree; the inline action-failure message (err) is the just-pressed control's result and takes precedence.
        title={rowTitle}
        onContextMenu={e => {
          e.preventDefault()
          e.stopPropagation() // ★the row menu wins (ADR-0064)★: stop here so the parent unified slot menu does not appear.
          setSelectedAgent(data.id)
          setRowMenu({ x: e.clientX, y: e.clientY, agentId: data.id, kind: data.kind })
        }}
      >
        <span
          data-agent-toggle={hasChildren ? '1' : undefined}
          onClick={e => {
            if (!hasChildren) return
            e.stopPropagation()
            node.toggle()
          }}
          style={{
            width: '10px',
            flexShrink: 0,
            textAlign: 'center',
            fontSize: '9px',
            color: 'var(--text-muted)',
            cursor: hasChildren ? 'pointer' : 'default',
          }}
        >
          {hasChildren ? (node.isOpen ? '▾' : '▸') : ''}
        </span>
        {/* State = icon shape (primary signal). ADR-0062: active (Running) gets *supplementary* green (--status-running theme var), others muted; the e-ink block neutralizes the variable → only the shape remains. No color literals in the component, variables only (ADR-0062 §44).
            ★Color is owned by the span's CSS color★: the icon's stroke and fill are both currentColor so they follow it — lucide's `color` prop sets only stroke, so a filled circle's fill would not follow.
            data-agent-glyph-shape = DOM observation surface: keeps the state readout textContent carried in the text-glyph days (an svg leaves no text).
            ★ADR-0173: two axes ride this spot★ — a row with a last failure that is not up gets the forbidden marker (shape=`blocked`) **swapped in** instead of the state shape, and color/hover follow the same verdict. Not placing
            two glyphs side by side is the point of that decision, so do not add a second <span> here. */}
        {/* ★role+aria-label are needed for the state to be heard★: without accessibility props lucide adds aria-hidden to the svg (node_modules/lucide-react) and the icon vanishes from the accessibility tree — without this a Running row and an Exited row
            would have the same readable name. */}
        <span
          data-agent-glyph="1"
          data-agent-glyph-shape={glyph.shape}
          // Rotation is owned by the stylesheet (agentGlyph.css) — attached only while waiting.
          className={phase === 'pending' ? 'engram-glyph-spin' : undefined}
          role="img"
          // ★The heard name stays the state vocabulary as is★: the last failure's reason is carried on one line by the row `title` (hover), whose sole wording source is the kind table (failureKinds); repeating it here could diverge across two surfaces.
          //   Machine reading (tests, CDP, LLM) is carried by `data-agent-glyph-shape` above as `blocked`. ★Exception while waiting★: `status` is then an optimistic value (`Running` right after the double-click), and reading it as is would revive by sound a lie already removed from the screen.
          aria-label={phase === 'pending' ? t('agent.glyphPending') : data.status}
          aria-busy={phase === 'pending' ? true : undefined}
          style={{
            display: 'flex',
            alignItems: 'center',
            // ADR-0173: with a last failure and not up, glyph and color switch together to the forbidden marker — same verdict as `glyph` above.
            color: rowGlyphColor(phase, data.status, data.lastFailure),
            flexShrink: 0,
          }}
        >
          <glyph.Icon size={12} fill={glyph.fill} />
        </span>
        {/* cwd is not shown (title only). */}
        {editingId === data.id ? (
          <input
            data-agent-rename-input={data.id}
            value={draft}
            autoFocus
            ref={inputRef}
            onChange={e => setDraft(e.target.value)}
            // ★Enter/Escape routing is owned by the row div onKeyDown★ (comment above — avoids react-window+React19 keydown delegation); keys bubble up there. Only blur commit is owned by the input.
            onBlur={() => commitEdit(data)}
            onClick={e => e.stopPropagation()}
            onDoubleClick={e => e.stopPropagation()}
            style={{
              flex: 1,
              minWidth: '3ch',
              maxWidth: '180px',
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
            data-agent-name="1"
            // flex:1+minWidth:0 = fill the row width so ellipsis applies properly. paddingRight 2px = room for italic overhang.
            style={{
              flex: 1,
              minWidth: 0,
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
              paddingRight: '2px',
            }}
          >
            {displayNameOf(data)}
          </span>
        )}
        {err && (
          <span style={{ marginLeft: 'auto', color: 'var(--text-muted)', fontSize: '10px', flexShrink: 0 }}>
            {t('agent.rowFailedBadge')}
          </span>
        )}
      </div>
    )
  }

  return (
    // ★Structure = label (fixed) outside + only the tree in the area below★ (same as AgentMonitoringPicker, ADR-0053): the label is a flex sibling outside the scroll so it stays fixed. react-arborist (react-window) owns its virtualized scroll,
    //   so it is not wrapped in ScrollArea (scroll-ownership conflict). ★Pane-background marker = this outer flex container (data-agent-list)★: a pane-background right-click bubbles to the parent unified slot menu (ADR-0064); only a row right-click is stopPropagation'd by the row handler.
    <div
      data-agent-list="1"
      data-testid="agent-list"
      style={{
        flex: 1,
        minHeight: 0,
        height: '100%',
        display: 'flex',
        flexDirection: 'column',
        background: 'var(--bg-secondary)',
      }}
    >
      {/* Slot content label = user request. */}
      <div
        data-slot-label="agent-list"
        style={{
          flexShrink: 0,
          padding: '10px 12px 6px',
          background: 'var(--bg-secondary)',
          color: 'var(--text-muted)',
          fontFamily: 'var(--font-ui)',
          fontSize: '11px',
          fontWeight: 600,
          letterSpacing: '0.03em',
        }}
      >
        {t('agent.treeLabel')}
      </div>
      {/* ★The measuring area is always mounted★: the ResizeObserver effect above attaches only once at mount — inside a branch
          it never attaches in a window that came up unreceived / empty, and the tree stays at the initial size. */}
      <div ref={treeContainerRef} style={{ flex: 1, minHeight: 0, overflow: 'hidden' }}>
        {!listLoaded ? null : flatRows.length === 0 ? (
          // ★Empty list = a centering div instead of the tree★: react-arborist is not mounted — an empty state has no scroll/virtualization surface.
          <div
            style={{
              height: '100%',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              color: 'var(--text-muted)',
              fontFamily: 'var(--font-ui)',
              fontSize: '12px',
            }}
          >
            {t('agent.emptyList')}
          </div>
        ) : (
          <div data-testid="agent-tree">
            <Tree<AgentTreeNode>
              data={forest}
              idAccessor="id"
              childrenAccessor="children"
              width={dimensions.width}
              height={dimensions.height}
              rowHeight={ROW_HEIGHT}
              indent={INDENT}
              openByDefault
              // ★rename is our own right-click→inline input★: our draft state replaces react-arborist's built-in edit (onRename), so the built-in is off (no duplicate edit UI); multi-select is not needed → single select kept.
              disableEdit
              disableMultiSelection
              // ★Not draggable = a node without a profile (ADR-0072 drop guard)★: ad-hoc (SpawnByCwd) has no profile to reparent and the backend rejects no-profile with Error — block the drag up front (reserved and running-with-profile are draggable).
              disableDrag={(data: AgentTreeNode) => !data.hasProfile}
              // ★One-level cap + has-profile + parent-invariant UI guard (ADR-0072)★ — three things blocked in the UI:
              //   ① a drop onto a non-root parent (= under a child), or dropping a node that already has children under another node (both make two levels). A leaf also has children:[], so react-arborist sees it as isInternal and the center drop applies.
              //   ② a drop with a profile-less node (ad-hoc) as parent — nothing can attach there.
              //   ③ a drop onto the same parent as now (including root↔root reorder) — backend ordering is created_at based, so reorder is unsupported; react-arborist shows same-parent reorder as a valid drop but nothing happens, so disable it (onMove is already a no-op).
              //   ★Root-drop normalization★: on a root drop canDrop gives parentNode as the internal root pseudo-node (isRoot, level=-1, data={id:ROOT_ID}), not null; so ② and ③ apply only to a real parent (not isRoot), and root-target comparison is judged by dropParentId (root=null).
              disableDrop={({ parentNode, dragNodes }) => {
                const isRootDrop = parentNode == null || parentNode.isRoot
                const dropParentId = isRootDrop ? null : parentNode.id
                const dragParent = dragNodes[0]?.parent
                const currentParentId = dragParent && !dragParent.isRoot ? dragParent.id : null
                return (
                  (!isRootDrop && parentNode.level > 0) ||
                  (!isRootDrop && !parentNode.data.hasProfile) ||
                  dragNodes.some(n => Array.isArray(n.data.children) && n.data.children.length > 0) ||
                  currentParentId === dropParentId
                )
              }}
              // ★Drag reparent = onMove (§5)★. Fires reparentProfile with dropParentId (null=root) (no optimistic update).
              onMove={({ dragIds, parentId }) => {
                // single drag assumed via disableMultiSelection — only [0] handled (revisit this line if multi-select is re-enabled).
                if (dragIds.length !== 1) return
                const childId = dragIds[0]
                if (childId) reparent(childId, parentId)
              }}
            >
              {NodeRenderer}
            </Tree>
          </div>
        )}
      </div>

      {/* ── Row right-click menu ───────────────────────────────────────── */}
      {rowMenu && (
        <div
          ref={rowMenuRef}
          // ADR-0237: this Esc closes the menu — while open, a marker so the chat area's Esc does not interrupt the turn (`interruptKey.ts`).
          data-engram-overlay="1"
          style={MENU_STYLE(rowMenu.x, rowMenu.y)}
        >
          {rowMenuItems.map(item => (
            <div
              key={item.id}
              style={MENU_ITEM_STYLE(item.disabled)}
              onMouseEnter={e => {
                if (!item.disabled) e.currentTarget.style.background = 'color-mix(in srgb, var(--accent) 20%, transparent)'
              }}
              onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}
              onClick={e => {
                e.stopPropagation()
                if (!item.disabled) {
                  item.action()
                  setRowMenu(null)
                }
              }}
            >
              {item.label}
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

// Menu common style (same as the SlotContextMenu and AgentTree inline menus — variables only).
function MENU_STYLE(x: number, y: number): React.CSSProperties {
  return {
    position: 'fixed',
    top: y,
    left: x,
    background: 'var(--bg-secondary)',
    border: '1px solid var(--border)',
    borderRadius: '4px',
    zIndex: 1000,
    minWidth: '150px',
    boxShadow: '0 2px 8px rgba(0,0,0,0.3)',
    fontFamily: 'var(--font-ui)',
    fontSize: '12px',
  }
}
function MENU_ITEM_STYLE(disabled: boolean): React.CSSProperties {
  return {
    padding: '6px 12px',
    cursor: disabled ? 'default' : 'pointer',
    color: disabled ? 'var(--text-muted)' : 'var(--text)',
    opacity: disabled ? 0.5 : 1,
  }
}
