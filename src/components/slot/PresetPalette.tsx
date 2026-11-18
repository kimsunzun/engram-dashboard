//! PresetPalette — preset (frequently used cwd bookmarks) slot content (ADR-0060 variant / ADR-0061 storage and enrichment).
//!
//! CRUD calls only the agentClient (single control surface, ADR-0011) preset methods; the screen reflects changes only via the PresetListUpdated broadcast → store mirror replacement, with no optimistic update (multi-window sync invariant, ADR-0061).
//! ★No pane menu (ADR-0064)★: add is contributed by the unified slot menu's preset.add command (folder dialog → createPreset, presetCommands.ts); a pane right-click bubbles to the parent LayoutLeaf's SlotContextMenu.
//! ★Style = variables only (theme-compliant)★: colors and fonts are CSS variable references — zero hard-coded color literals (e-ink contrast).

import { useEffect, useRef, useState } from 'react'

import { ScrollArea } from '../ui/scroll-area'
import { agentClient } from '../../api/clientFactory'
import { useAgentStore } from '../../store/agentStore'
import { refreshPresets } from '../../store/eventBus'
import { basename } from '../../util/basename'
import type { Preset } from '../../api/types'
import { t } from '../../i18n'
import { useBlurOnDragRegionPress } from '../layout/titleRow/dragRegion'

/** ADR-0061 enrichment. Derivation uses the shared `basename` util — one source with the AgentList row display name (duplicating it would diverge on win/posix and root edge cases). */
export function presetDisplayName(preset: Pick<Preset, 'cwd' | 'name'>): string {
  return preset.name ?? basename(preset.cwd)
}

type RowMenu = { x: number; y: number; presetId: string }

export default function PresetPalette() {
  const presets = useAgentStore(s => s.presets)
  // Leave the list empty until received so a new window does not flash the empty-list notice (same rule as AgentList).
  const presetsLoaded = useAgentStore(s => s.presetsLoaded)
  const rowMenuRef = useRef<HTMLDivElement>(null)
  const [rowMenu, setRowMenu] = useState<RowMenu | null>(null)

  // In-flight guard (local state only — no store optimistic update, ADR-0061): prevents a duplicate DeletePreset before Ack; released on completion either way.
  // ★ref = authoritative double-fire guard, state = visuals (disabled/opacity)★: a useState guard lets two calls both pass (the second reads a stale closure still false before the re-render commits); a ref mutates synchronously so the same-tick second call reads true.
  const deletingRef = useRef<Set<string>>(new Set())
  const [deleting, setDeleting] = useState<ReadonlySet<string>>(() => new Set())
  const renamingRef = useRef<Set<string>>(new Set())
  const [renaming, setRenaming] = useState<ReadonlySet<string>>(() => new Set())

  // ★Inline-edit local state (frontend-only draft, separate from the backend-authoritative name, TabBar pattern)★
  const [editingId, setEditingId] = useState<string | null>(null)
  const [draft, setDraft] = useState('')
  // ADR-0373: the name the edit started from — "unchanged" compares with this, not a name re-derived at commit (a language switch turns a blank-path name into another language's word).
  const [draftStart, setDraftStart] = useState('')
  // ★Stable ref★: select() once on entering edit (editingId change) — selecting via an inline callback ref re-attaches every render and breaks typing (as TabBar FIX 1).
  const inputRef = useRef<HTMLInputElement>(null)
  useEffect(() => {
    if (editingId !== null) inputRef.current?.select()
  }, [editingId])
  // Pressing the window-drag area (title row) also ends the edit — commit stays on onBlur.
  useBlurOnDragRegionPress(editingId !== null, inputRef)

  useEffect(() => {
    if (!rowMenu) return
    const h = (e: MouseEvent) => {
      const target = e.target as Node
      if (rowMenuRef.current && !rowMenuRef.current.contains(target)) setRowMenu(null)
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

  // ★Close the menu when the target disappears★: rowMenu outlives its row — if the target preset id drops out of presets (delete etc.), reset to null at once (as AgentList).
  useEffect(() => {
    if (rowMenu && !presets.some(p => p.id === rowMenu.presetId)) setRowMenu(null)
  }, [rowMenu, presets])

  // Delete: in-flight tracked per id — the same preset fires once until resolve (double-click guard); `deletingRef` (sync) is authoritative, different ids still run concurrently.
  const removePreset = (id: string): void => {
    if (deletingRef.current.has(id)) return
    deletingRef.current.add(id)
    setDeleting(prev => {
      const next = new Set(prev)
      next.add(id)
      return next
    })
    agentClient
      .deletePreset(id)
      // ★refreshPresets() safety net on success (symmetric with AgentList delete/rename)★: the list normally arrives via the PresetListUpdated broadcast, but a missed one (reconnect window, lost event) leaves only this window stale; re-pulling the authoritative list is idempotent.
      .then(() => refreshPresets())
      .catch(e => console.error('[PresetPalette] deletePreset failed:', e))
      .finally(() => {
      deletingRef.current.delete(id) // release the lock regardless of success or failure (so an error cannot lock the UI forever)
        setDeleting(prev => {
          const next = new Set(prev)
          next.delete(id)
          return next
        })
      })
  }

  const beginEdit = (preset: Preset) => {
    const name = presetDisplayName(preset)
    setEditingId(preset.id)
    setDraft(name)
    setDraftStart(name)
  }
  const cancelEdit = () => setEditingId(null)
  // ★Idempotent★: return if editingId is not this preset — Enter unmounts the input with setEditingId(null), the browser fires blur, and onBlur→commitEdit would run again; editingId is already null then, so no double renamePreset (as TabBar).
  const commitEdit = (preset: Preset) => {
    if (editingId !== preset.id) return
    const trimmed = draft.trim()
    setEditingId(null)
    if (trimmed.length === 0 || trimmed === draftStart) return
    if (renamingRef.current.has(preset.id)) return // sync authoritative guard (blocks duplicate rename submits)
    renamingRef.current.add(preset.id)
    setRenaming(prev => {
      const next = new Set(prev)
      next.add(preset.id)
      return next
    })
    agentClient
      .renamePreset(preset.id, trimmed)
      // ★refreshPresets() safety net on success (symmetric with deletePreset)★: see the delete comment above.
      .then(() => refreshPresets())
      .catch(e => console.error('[PresetPalette] renamePreset failed:', e))
      .finally(() => {
        renamingRef.current.delete(preset.id)
        setRenaming(prev => {
          const next = new Set(prev)
          next.delete(preset.id)
          return next
        })
      })
  }

  return (
    <div
      data-preset-palette="1"
      style={{
        width: '100%',
        height: '100%',
        boxSizing: 'border-box',
        display: 'flex',
        flexDirection: 'column',
        background: 'var(--bg)',
        color: 'var(--text)',
        fontFamily: 'var(--font-ui)',
        fontSize: '13px',
        overflow: 'hidden',
      }}
    >
      {/* Each placed only in the two variant components PresetPalette and AgentList, not the shared slot header
          (other slots such as the terminal are unaffected). */}
      <div
        data-slot-label="preset"
        style={{
          padding: '6px 8px',
          borderBottom: '1px solid var(--border)',
          color: 'var(--text-muted)',
          fontFamily: 'var(--font-ui)',
          fontSize: '11px',
          fontWeight: 600,
          letterSpacing: '0.03em',
          flexShrink: 0,
        }}
      >
        {t('preset.label')}
      </div>

      {/* Scroll via the shared ScrollArea seam (ADR-0053). It is a flat list (no virtualization) so wrapping in a Viewport is harmless. */}
      <ScrollArea style={{ flex: 1, minHeight: 0 }}>
        {presets.length === 0 ? (
          presetsLoaded && (
            <div style={{ padding: '12px', color: 'var(--text-muted)', fontSize: '12px' }}>
              {t('preset.empty')}
            </div>
          )
        ) : (
          presets.map(preset => {
            const isBusy = deleting.has(preset.id) || renaming.has(preset.id)
            const isEditing = editingId === preset.id
            return (
            <div
              key={preset.id}
              data-preset-id={preset.id}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: '8px',
                padding: '6px 8px',
                borderBottom: '1px solid var(--border)',
                cursor: isBusy ? 'wait' : 'default',
                opacity: isBusy ? 0.6 : 1, // in-flight visual indicator (opacity only, no color literals)
                userSelect: 'none',
              }}
              // ★Row right-click = row menu (rename, delete)★: stopPropagation so it does not leak to the parent unified slot menu (ADR-0064).
              onContextMenu={e => {
                e.preventDefault()
                e.stopPropagation()
                setRowMenu({ x: e.clientX, y: e.clientY, presetId: preset.id })
              }}
            >
              <div style={{ flex: 1, minWidth: 0 }}>
                {isEditing ? (
                  <input
                    data-preset-rename-input={preset.id}
                    value={draft}
                    autoFocus
                    ref={inputRef}
                    onChange={e => setDraft(e.target.value)}
                    onKeyDown={e => {
                      // ★Block bubbling★: keys during editing must not leak to global keybindings (same as TabBar).
                      e.stopPropagation()
                      if (e.key === 'Enter') commitEdit(preset)
                      else if (e.key === 'Escape') cancelEdit()
                    }}
                    onBlur={() => commitEdit(preset)}
                    onClick={e => e.stopPropagation()}
                    style={{
                      fieldSizing: 'content',
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
                  <div
                    data-preset-name="1"
                    style={{ color: 'var(--text)', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}
                  >
                    {presetDisplayName(preset)}
                  </div>
                )}
                <div
                  title={preset.cwd}
                  style={{
                    color: 'var(--text-muted)',
                    fontSize: '11px',
                    whiteSpace: 'nowrap',
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                  }}
                >
                  {preset.cwd}
                </div>
              </div>
            </div>
            )
          })
        )}

        {/* ── Row right-click menu (rename · delete) ─────────────────────────── */}
        {rowMenu && (
          <div
            ref={rowMenuRef}
            // ADR-0237: this Esc closes the menu — while open, a marker so the chat area's Esc does not interrupt the turn (`interruptKey.ts`).
            data-engram-overlay="1"
            style={MENU_STYLE(rowMenu.x, rowMenu.y)}
          >
            {rowMenuItems(rowMenu, presets, { beginEdit, removePreset, deleting, renaming }).map(item => (
              <div
                // ADR-0373: `id`, not the translated label.
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
      </ScrollArea>
    </div>
  )
}

function rowMenuItems(
  rowMenu: RowMenu,
  presets: Preset[],
  ctx: {
    beginEdit: (p: Preset) => void
    removePreset: (id: string) => void
    deleting: ReadonlySet<string>
    renaming: ReadonlySet<string>
  },
): Array<{ id: string; label: string; disabled: boolean; action: () => void }> {
  const preset = presets.find(p => p.id === rowMenu.presetId)
  if (!preset) return []
  const busy = ctx.deleting.has(preset.id) || ctx.renaming.has(preset.id)
  return [
    { id: 'rename', label: t('preset.rename'), disabled: busy, action: () => ctx.beginEdit(preset) },
    // The menu label is the short 'delete' (preset.deleteBtn) — distinct from the command title 'delete preset' (preset.delete).
    { id: 'delete', label: t('preset.deleteBtn'), disabled: busy, action: () => ctx.removePreset(preset.id) },
  ]
}

// Menu common style (same as the AgentList inline menu — variables only).
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
