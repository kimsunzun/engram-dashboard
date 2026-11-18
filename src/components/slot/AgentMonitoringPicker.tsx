// ADR-0067: agent-monitoring search popup (command-palette style) — assigns a running agent to the right-clicked slot.
// ★Role★: slot right-click "agent monitoring" (slot.assignRunningAgent) opens this popup, with monitoringPickerStore.open(viewId, slotId) carrying the target; it draws a search box + running-agent filter list, and picking one assigns it. No placement state of its own — on-select flows through viewStore.assignAgent (assign_agent), the §5 single control surface (ADR-0067 invariant).
// ★Target = the right-clicked slot (explicit)★: the (viewId, slotId) in the store, independent of focus (focus-steal blocked at the source); opening does not touch focused_slot_id (ADR-0067 focus invariant).
// ★No optimistic update (ADR-0035)★: assignAgent only invokes assign_agent; the popup closes right after sending and the slot swap is drawn by the backend-authoritative layout:updated emit.
// ★Single mount per window★: WindowLayout mounts one and it renders nothing when target is null (same fixed-overlay pattern as SlotContextMenu). ★Style = variables only★ (same discipline as PresetPalette/AgentList — e-ink contrast, zero color literals).

import { useEffect, useRef, useState } from 'react'

import { ScrollArea } from '../ui/scroll-area'
import { useAgentStore } from '../../store/agentStore'
import { useMonitoringPickerStore } from '../../store/monitoringPickerStore'
import { useViewStore } from '../../store/viewStore'
import { filterMonitoringCandidates } from './monitoringPickerFilter'
import { t } from '../../i18n'

export default function AgentMonitoringPicker() {
  const target = useMonitoringPickerStore(s => s.target)
  const close = useMonitoringPickerStore(s => s.close)
  const agents = useAgentStore(s => s.agents)
  // ADR-0061: the display name derives from the profile's display_name like the tree (filterMonitoringCandidates joins); profiles come from the same store as agents (single source with mergeTreeNodes).
  const profiles = useAgentStore(s => s.profiles)

  const [query, setQuery] = useState('')
  // Keyboard-selection highlight (Enter to assign) — a list index. Reset to 0 when the query changes.
  const [activeIndex, setActiveIndex] = useState(0)
  const inputRef = useRef<HTMLInputElement>(null)

  const candidates = filterMonitoringCandidates(agents, profiles, query)

  // Auto-focus the search box after mount. key={openId} makes every open() a fresh remount (monitoringPickerStore.openId), so useState initial values reset query/activeIndex — only focus here.
  useEffect(() => {
    if (!target) return
    // The ref is settled at this point even without requestAnimationFrame.
    inputRef.current?.focus()
  }, [target])

  useEffect(() => {
    setActiveIndex(0)
  }, [query])

  // Close on Esc — attach the listener only while open (prevents leaks). Separate from the input keydown (also catches a global Esc outside the backdrop).
  useEffect(() => {
    if (!target) return
    const h = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close()
    }
    document.addEventListener('keydown', h)
    return () => document.removeEventListener('keydown', h)
  }, [target, close])

  if (!target) return null

  // ADR-0067: on-select placement = viewStore.assignAgent (right-clicked slot coordinates) → invoke(assign_agent) → emit; no optimistic update. Failure is a console warning (no toast, like AgentList openInFocusedSlot); the popup closes either way.
  const selectAgent = (agentId: string) => {
    void useViewStore
      .getState()
      .assignAgent(target.viewId, target.slotId, agentId)
      .catch(e => console.error('[AgentMonitoringPicker] assignAgent failed:', e))
    close()
  }

  const onInputKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      setActiveIndex(i => Math.min(i + 1, Math.max(0, candidates.length - 1)))
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      setActiveIndex(i => Math.max(i - 1, 0))
    } else if (e.key === 'Enter') {
      e.preventDefault()
      const chosen = candidates[activeIndex]
      if (chosen) selectAgent(chosen.id)
    }
    // Escape is handled by the global listener above.
  }

  return (
    // Backdrop — click closes (the popup body stops propagation). Fixed full cover + translucent rgba (a lightness dim, theme-independent, so no variable). zIndex above SlotContextMenu (1000/1001).
    <div
      data-monitoring-picker-backdrop="1"
      // ADR-0237: this Esc closes the popup — while open, a marker so the chat area's Esc does not interrupt the turn (`interruptKey.ts`).
      data-engram-overlay="1"
      onClick={close}
      style={{
        position: 'fixed',
        inset: 0,
        background: 'rgba(0,0,0,0.4)',
        zIndex: 2000,
        display: 'flex',
        alignItems: 'flex-start',
        justifyContent: 'center',
        paddingTop: '12vh',
      }}
    >
      <div
        data-monitoring-picker="1"
        onClick={e => e.stopPropagation()}
        style={{
          width: 'min(480px, 90vw)',
          maxHeight: '60vh',
          display: 'flex',
          flexDirection: 'column',
          background: 'var(--bg-secondary)',
          color: 'var(--text)',
          border: '1px solid var(--border)',
          borderRadius: '6px',
          boxShadow: '0 8px 32px rgba(0,0,0,0.4)',
          fontFamily: 'var(--font-ui)',
          fontSize: '13px',
          overflow: 'hidden',
        }}
      >
        {/* Label — assign a running agent to the right-clicked slot */}
        <div
          style={{
            padding: '6px 10px',
            borderBottom: '1px solid var(--border)',
            color: 'var(--text-muted)',
            fontSize: '11px',
            fontWeight: 600,
            letterSpacing: '0.03em',
            flexShrink: 0,
          }}
        >
          {t('agent.monitoringLabel')}
        </div>

        {/* Search box — auto-focus (effect above) */}
        <input
          ref={inputRef}
          data-monitoring-picker-search="1"
          value={query}
          onChange={e => setQuery(e.target.value)}
          onKeyDown={onInputKeyDown}
          placeholder={t('agent.monitoringSearch')}
          style={{
            margin: '8px',
            padding: '6px 8px',
            background: 'var(--bg)',
            color: 'var(--text)',
            border: '1px solid var(--border)',
            borderRadius: '4px',
            fontFamily: 'var(--font-ui)',
            fontSize: '13px',
            outline: 'none',
            flexShrink: 0,
          }}
        />

        {/* Candidate list — running filter + query narrowing. Flat list scrolled through the shared ScrollArea seam (ADR-0053). */}
        <ScrollArea style={{ flex: 1, minHeight: 0 }}>
          {candidates.length === 0 ? (
            <div
              data-monitoring-picker-empty="1"
              style={{ padding: '12px', color: 'var(--text-muted)', fontSize: '12px' }}
            >
              {agents.some(a => a.status.type === 'Running')
                ? t('agent.noCandidates')
                : t('agent.noRunning')}
            </div>
          ) : (
            candidates.map((c, i) => (
              <div
                key={c.id}
                data-monitoring-candidate={c.id}
                onClick={() => selectAgent(c.id)}
                onMouseEnter={() => setActiveIndex(i)}
                title={c.cwd}
                style={{
                  padding: '6px 10px',
                  cursor: 'pointer',
                  borderBottom: '1px solid var(--border)',
                  // accent mix without color literals.
                  background:
                    i === activeIndex ? 'color-mix(in srgb, var(--accent) 20%, transparent)' : 'transparent',
                }}
              >
                <div
                  style={{ color: 'var(--text)', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}
                >
                  {c.name}
                </div>
                <div
                  style={{
                    color: 'var(--text-muted)',
                    fontSize: '11px',
                    whiteSpace: 'nowrap',
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                  }}
                >
                  {c.cwd}
                </div>
              </div>
            ))
          )}
        </ScrollArea>
      </div>
    </div>
  )
}
