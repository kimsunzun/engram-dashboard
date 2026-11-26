import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// ── react-arborist (react-window) jsdom shim ──────────────────────────────────────
// jsdom has no ResizeObserver — the Tree container measuring hook references it, so a no-op stub.
//   (The callback never runs, but AgentList's dimensions initial value is non-zero so the tree draws rows from the first render.)
globalThis.ResizeObserver ||= class {
  observe() {}
  unobserve() {}
  disconnect() {}
} as unknown as typeof ResizeObserver

// ── clientFactory stub ────────────────────────────────────────────────────────
// ADR-0064: spawn goes through the unified slot menu's agentlist.createAgent command; AgentList does not call registry.run.
const clientMock = vi.hoisted(() => ({
  spawnProfile: vi.fn(async () => ({ id: 'a' })),
  killAgent: vi.fn(async () => undefined),
  deleteProfile: vi.fn(async () => undefined),
  renameProfile: vi.fn(async () => undefined),
  reparentProfile: vi.fn(async () => undefined),
}))
vi.mock('../../api/clientFactory', () => ({
  agentClient: {
    spawnProfile: (...args: unknown[]) => clientMock.spawnProfile(...(args as [])),
    killAgent: (...args: unknown[]) => clientMock.killAgent(...(args as [])),
    deleteProfile: (...args: unknown[]) => clientMock.deleteProfile(...(args as [])),
    renameProfile: (...args: unknown[]) => clientMock.renameProfile(...(args as [])),
    reparentProfile: (...args: unknown[]) => clientMock.reparentProfile(...(args as [])),
  },
  getAgentClient: vi.fn(),
}))
// refreshProfiles is the safety net that re-applies the authoritative list after rename/activate/cancel succeed (against a lost broadcast).
const refreshProfilesMock = vi.hoisted(() => vi.fn(async () => undefined))
vi.mock('../../store/eventBus', () => ({ refreshProfiles: refreshProfilesMock }))
// viewStore only references assignAgent (the open path) — accessed via getState, so the real store is shallowly stubbed.
//   assignAgent is kept in one hoisted place so getState and selectors return the same instance (for call verification).
const assignAgentMock = vi.hoisted(() => vi.fn(async () => undefined))
// selectView returns CachedView({layout, focusedSlotId, foreignSlots}) — openInFocusedSlot picks the target slot with
//   selectOpenTarget(layout, focusedSlotId, foreignSlots). By default the focused slot ('slot-1') is an
//   empty content slot so the default path (assign to the focused slot) holds. The control-slot / no-empty-slot edge cases are verified by the safety-net
//   suite swapping the selectViewMock return (hoisted, so it can change per test).
const selectViewMock = vi.hoisted(() =>
  vi.fn<() => { layout: LayoutNode; focusedSlotId: string | null; foreignSlots: string[] } | null>(() => ({
    layout: { type: 'slot', id: 'slot-1', content: { type: 'empty' } },
    focusedSlotId: 'slot-1',
    foreignSlots: [],
  })),
)
const currentViewIdMock = vi.hoisted(() => vi.fn(() => 'main-view' as string | null))
vi.mock('../../store/viewStore', () => ({
  useViewStore: Object.assign(
    (sel: (s: unknown) => unknown) => sel({ assignAgent: assignAgentMock }),
    { getState: () => ({ assignAgent: assignAgentMock }) },
  ),
  currentViewId: () => currentViewIdMock(),
  selectView: () => selectViewMock(),
}))

// Ban = the forbidden marker of the last-failure axis (ADR-0173) — unlike the status axis's 4 shapes (Circle/Square/X) it appears only when `lastFailure`
//   exists. So it is not in the 「nothing outside the 4 shapes comes out」 allowlist below (status axis called alone).
import { Ban, Circle, Loader, Square, X } from 'lucide-react'

import AgentList, { statusGlyphColor, statusGlyphIcon } from './AgentList'
// The presentation layer is imported directly from statusGlyph, the mapping SSOT — AgentList's re-export is
//   only a path for old importers, so no new axis is added there.
import { rowGlyphColor, rowGlyphIcon, rowPhase } from './statusGlyph'
import { useAgentStore } from '../../store/agentStore'
import { setLanguage, t } from '../../i18n'
import { useLanguage } from '../../i18n/react'
import { OVERLAY_SELECTOR } from '../slot/interruptKey'
import type { AgentInfo, AgentProfile, Capabilities } from '../../api/types'
import type { LayoutNode } from '../../api/layoutTypes'

const caps = (): Capabilities => ({
  input: { raw: true, message: false, attachment: false },
  output: { terminal_bytes: true, structured: false, markdown: false, tool_events: false, usage: false },
  control: { resize: true, interrupt: true, cancel: false, graceful_shutdown: false },
  session: { resume: true, snapshot: false, cwd_env: true },
  model: { select: false, temperature: false, max_tokens: false },
})

function agent(id: string, cwd: string, status: AgentInfo['status'] = { type: 'Running' }): AgentInfo {
  return { id, name: '', cwd, status, cols: 80, rows: 24, epoch: 1, capabilities: caps() }
}
function profile(
  id: string,
  cwd: string,
  createdAt = 0,
  displayName: string | null = null,
  parentId: string | null = null,
  lastFailure: AgentProfile['last_failure'] = null,
): AgentProfile {
  return {
    id, name: '', display_name: displayName, parent_id: parentId,
    command: { kind: 'Claude', extra_args: [], output_format: 'Terminal' },
    cwd, env: [], backend_session_id: null, old_session_ids: [], epoch: 0, auto_restore: false,
    restart_policy: 'Never', restart_count: 0, failed_reason: null, last_failure: lastFailure,
    created_at: createdAt,
    last_active: 0, last_start_at: null,
  }
}

beforeEach(() => {
  clientMock.spawnProfile.mockClear()
  clientMock.killAgent.mockClear()
  clientMock.deleteProfile.mockClear()
  clientMock.renameProfile.mockClear()
  clientMock.reparentProfile.mockClear()
  refreshProfilesMock.mockClear()
  assignAgentMock.mockClear()
  // Reset to the default selectView/currentViewId (isolates what the safety-net suite swapped per test).
  selectViewMock.mockReset()
  selectViewMock.mockReturnValue({
    layout: { type: 'slot', id: 'slot-1', content: { type: 'empty' } },
    focusedSlotId: 'slot-1',
    foreignSlots: [],
  })
  currentViewIdMock.mockReset()
  currentViewIdMock.mockReturnValue('main-view')
  // Default is 「roster and profiles both received」 — a test for the not-received span lowers the flag itself.
  useAgentStore.setState({
    agents: [], agentsLoaded: true, profiles: [], profilesLoaded: true, selectedAgentId: null,
  })
})
afterEach(() => {
  cleanup()
  useAgentStore.setState({ agents: [], profiles: [], selectedAgentId: null })
})

// ── statusGlyphIcon: every branch (pure, ADR-0062) ─────────────────────────────────
// Icon identity is measured by **component reference** — so the mapping test holds even if lucide's class-name convention changes.
//   Class names are seen only in the render suite below (did a real svg appear).
describe('statusGlyphIcon (pure, all branches)', () => {
  it('Running → filled circle (working)', () => {
    const g = statusGlyphIcon('Running')
    expect(g.Icon).toBe(Circle)
    expect(g.fill).toBe('currentColor')
    expect(g.shape).toBe('filled-circle')
  })
  it('Exiting → square (transition to stopped)', () => expect(statusGlyphIcon('Exiting').Icon).toBe(Square))
  it('Exited → square (stopped)', () => expect(statusGlyphIcon('Exited').Icon).toBe(Square))
  it('Killed → square (stopped)', () => expect(statusGlyphIcon('Killed').Icon).toBe(Square))
  it('Failed → ✗ shape (error)', () => expect(statusGlyphIcon('Failed').Icon).toBe(X))
  it('Reserved → empty circle (idle/not spawned)', () => {
    const g = statusGlyphIcon('Reserved')
    expect(g.Icon).toBe(Circle)
    expect(g.fill).toBe('none')
    expect(g.shape).toBe('circle')
  })
  it('unknown status → empty circle (degrade, avoids an empty cell)', () => {
    const g = statusGlyphIcon('???')
    expect(g.Icon).toBe(Circle)
    expect(g.shape).toBe('circle')
  })

  // ★Successor of the old "does not return ◐" test★ (ADR-0062 — awaiting input is vocabulary only, never lit): since character comparison
  //   is gone, it is measured by a **range allowlist**. If a new branch returns anything outside the 4 shapes below — a half-filled shape (CircleDashed, PieChart…) or whatever —
  //   this breaks here. InputWaiting/Idle/Waiting are mixed into the input list because
  //   if such a branch were really attached it would be attached under these names (if not attached they fall to default and pass).
  it('nothing outside the 4 shapes (including half-filled shapes like ◐) comes out for any input', () => {
    const inputs = [
      'Running', 'Exiting', 'Exited', 'Failed', 'Killed', 'Reserved', 'unknown', '',
      'InputWaiting', 'Idle', 'Waiting',
    ]
    const allowedIcons = [Circle, Square, X]
    const allowedShapes = ['filled-circle', 'square', 'x', 'circle']
    for (const s of inputs) {
      const g = statusGlyphIcon(s)
      expect(allowedIcons).toContain(g.Icon)
      expect(allowedShapes).toContain(g.shape)
      expect(['currentColor', 'none']).toContain(g.fill)
    }
  })
})

// ── statusGlyphColor: color supplement (pure, ADR-0062 color revision) ─────────────────────────
// Color is variables only (no literals) — e-ink neutralizes --status-running to var(--text-muted) in theme.css,
// so the logic here is single regardless of theme.
describe('statusGlyphColor (pure)', () => {
  it('Running → var(--status-running) (active green)', () =>
    expect(statusGlyphColor('Running')).toBe('var(--status-running)'))
  it('Exited → var(--text-muted)', () => expect(statusGlyphColor('Exited')).toBe('var(--text-muted)'))
  it('Killed → var(--text-muted)', () => expect(statusGlyphColor('Killed')).toBe('var(--text-muted)'))
  it('Failed → var(--text-muted)', () => expect(statusGlyphColor('Failed')).toBe('var(--text-muted)'))
  it('Exiting → var(--text-muted)', () => expect(statusGlyphColor('Exiting')).toBe('var(--text-muted)'))
  it('Reserved → var(--text-muted)', () => expect(statusGlyphColor('Reserved')).toBe('var(--text-muted)'))
  it('unknown status → var(--text-muted) (default)', () =>
    expect(statusGlyphColor('???')).toBe('var(--text-muted)'))
})

// ── The last failure swaps the glyph (ADR-0173) ────────────────────────────────
// No extra glyph is added — the forbidden marker **replaces** the status glyph's place and the color changes with it.
//
// ★Assert the `shape` token★ (stable, also goes out to the DOM), not the icon component — even if lucide swaps the component,
//   the token still carries what the glyph reads as.
describe('statusGlyphIcon/Color — last-failure axis', () => {
  const reserved = 'Reserved'
  it('an item at rest is swapped to the forbidden marker regardless of status', () => {
    for (const s of [reserved, 'Exited', 'Killed', 'Failed', '???']) {
      const g = statusGlyphIcon(s, 'NoConversationToResume')
      expect(g.shape).toBe('blocked')
      expect(g.Icon).toBe(Ban)
      // If filled, its silhouette overlaps Running (filled circle) and the distinction vanishes on e-ink.
      expect(g.fill).toBe('none')
      expect(statusGlyphColor(s, 'NoConversationToResume')).toBe('var(--status-blocked)')
    }
  })
  it('★running wins★ — even holding a last failure, Running keeps the original glyph and color', () => {
    expect(statusGlyphIcon('Running', 'NoConversationToResume').shape).toBe('filled-circle')
    expect(statusGlyphColor('Running', 'NoConversationToResume')).toBe('var(--status-running)')
  })
  it('Exiting is transitional so it counts on the running side (it must not flicker for the few seconds of a normal exit)', () => {
    expect(statusGlyphIcon('Exiting', 'NoConversationToResume')).toEqual(statusGlyphIcon('Exiting'))
    expect(statusGlyphColor('Exiting', 'NoConversationToResume')).toBe(statusGlyphColor('Exiting'))
  })
  it('with no last failure the existing mapping does not change a bit', () => {
    for (const s of ['Running', 'Exiting', 'Exited', 'Killed', 'Failed', 'Reserved', '???']) {
      expect(statusGlyphIcon(s, null)).toEqual(statusGlyphIcon(s))
      expect(statusGlyphColor(s, null)).toBe(statusGlyphColor(s))
    }
  })
  it('even with different kinds the glyph is one (no per-kind glyphs are added)', () => {
    const kinds = ['NoConversationToResume', 'SpawnFailed', 'EarlyExitAfterResume', 'Other'] as const
    const shapes = new Set(kinds.map(k => statusGlyphIcon(reserved, k).shape))
    expect(shapes.size).toBe(1)
  })
})

// ── Presentation layer: 「waiting for an answer」 · 「pure」 ────────────────────────
// The backend status becomes Running optimistically the moment a session exists, so it does not mean "connected". So
//   the span before an answer arrives and the span right after a refusal are laid **on top of** the status axis — the meaning of `isFailureBlocked` is unchanged.
describe('rowPhase / rowGlyphIcon / rowGlyphColor (presentation layer, pure)', () => {
  it('priority = pending > rejected > settled (a retry beats a past refusal)', () => {
    expect(rowPhase(true, false)).toBe('pending')
    expect(rowPhase(true, true)).toBe('pending')
    expect(rowPhase(false, true)).toBe('rejected')
    expect(rowPhase(false, false)).toBe('settled')
  })

  it('pending = spoke shape + a new token (the existing five are neither reused nor renamed)', () => {
    const g = rowGlyphIcon('pending', 'Running', null)
    expect(g.Icon).toBe(Loader)
    expect(g.shape).toBe('pending')
    // Spokes are lines with no area to fill — filling would let lucide smear the strokes.
    expect(g.fill).toBe('none')
  })

  it('★no green while waiting★ — even with an optimistic Running the color is pressed down to muted', () => {
    // The core of this defect was that green: claiming "running" in the span where the answer is unknown.
    for (const s of ['Running', 'Exiting', 'Exited', 'Failed', 'Killed', 'Reserved', '???']) {
      expect(rowGlyphColor('pending', s, null)).toBe('var(--text-muted)')
      expect(rowGlyphColor('pending', s, null)).not.toBe('var(--status-running)')
      expect(rowGlyphIcon('pending', s, null).shape).toBe('pending')
    }
  })

  it('★a refusal waits for neither the record nor the observed death★ — forbidden marker even with lastFailure=null and status=Running', () => {
    // The exact span of the measured lie: the RPC already refused but the process is still alive so status is Running.
    const g = rowGlyphIcon('rejected', 'Running', null)
    expect(g.shape).toBe('blocked')
    expect(g.Icon).toBe(Ban)
    expect(rowGlyphColor('rejected', 'Running', null)).toBe('var(--status-blocked)')
    // 「ADR-0173」 is a rule about the record of a *past* attempt so it does not apply here —
    //   the refusal is the answer this screen just received directly.
    expect(rowGlyphIcon('rejected', 'Running', null)).not.toEqual(statusGlyphIcon('Running', null))
  })

  it('settled does not change the status axis a bit (delegation — no copy is made)', () => {
    for (const s of ['Running', 'Exiting', 'Exited', 'Killed', 'Failed', 'Reserved', '???']) {
      for (const f of [null, 'NoConversationToResume'] as const) {
        expect(rowGlyphIcon('settled', s, f)).toEqual(statusGlyphIcon(s, f))
        expect(rowGlyphColor('settled', s, f)).toBe(statusGlyphColor(s, f))
      }
    }
  })

  it('the status axis alone (statusGlyphIcon) never yields pending (the layers do not mix)', () => {
    for (const s of ['Running', 'Exiting', 'Exited', 'Killed', 'Failed', 'Reserved', '???', '']) {
      expect(statusGlyphIcon(s).shape).not.toBe('pending')
      expect(statusGlyphIcon(s, 'SpawnFailed').shape).not.toBe('pending')
    }
  })
})

// ── Before the first list is received (new window) — no intermediate screen is drawn ──────────────────────────
describe('AgentList before the first list is received', () => {
  beforeEach(() => {
    useAgentStore.setState({ agentsLoaded: false, profilesLoaded: false })
  })

  it('neither received → neither the empty-list notice nor the tree (label only)', () => {
    render(<AgentList />)
    expect(screen.queryByText(/에이전트 없음/)).toBeNull()
    expect(document.querySelector('[data-testid="agent-tree"]')).toBeNull()
    expect(document.querySelector('[data-slot-label="agent-list"]')).toBeTruthy()
  })

  it('only the roster received, empty (profile pending) → the empty-list notice is not drawn yet', () => {
    render(<AgentList />)
    act(() => useAgentStore.getState().setAgents([]))
    expect(screen.queryByText(/에이전트 없음/)).toBeNull()
  })

  it('only profiles received (roster pending) → reserved rows are not drawn first', () => {
    render(<AgentList />)
    act(() => useAgentStore.getState().setProfiles([profile('p1', 'C:/r')]))
    expect(document.querySelector('[data-agent-row="p1"]')).toBeNull()
  })

  it('both received empty → then the empty-list notice', () => {
    render(<AgentList />)
    act(() => {
      useAgentStore.getState().setAgents([])
      useAgentStore.getState().setProfiles([])
    })
    expect(screen.getByText(/에이전트 없음/)).toBeTruthy()
  })

  it('both received + non-empty → tree rows, and the empty-list notice never appears', () => {
    render(<AgentList />)
    act(() => {
      useAgentStore.getState().setAgents([agent('a1', 'C:/w')])
      useAgentStore.getState().setProfiles([profile('p1', 'C:/r')])
    })
    const tree = document.querySelector('[data-testid="agent-tree"]')
    expect(tree?.querySelector('[data-agent-row="a1"]')).toBeTruthy()
    expect(tree?.querySelector('[data-agent-row="p1"]')).toBeTruthy()
    expect(screen.queryByText(/에이전트 없음/)).toBeNull()
  })
})

// ── Tree render (react-arborist, ADR-0072) ────────────────────────────────────────
describe('AgentList tree render', () => {
  it('empty list → notice text', () => {
    render(<AgentList />)
    expect(screen.getByText(/에이전트 없음/)).toBeTruthy()
  })

  // ★The empty-state notice is a flex-1 centering div outside the tree (react-arborist)★ — with no nodes to draw the tree (virtualized scroll) is not
  //   mounted and only the notice text is vertically centered (keeps the old FIX-A intent — an empty state has no scroll surface).
  it('empty list → the notice text is outside the tree container and the tree is not mounted', () => {
    render(<AgentList />)
    const tree = document.querySelector('[data-testid="agent-tree"]')
    expect(tree).toBeNull()
    const empty = screen.getByText(/에이전트 없음/)
    expect(empty.closest('[data-testid="agent-tree"]')).toBeNull()
    expect(empty.closest('[data-agent-list]')).toBeTruthy()
  })

  it('non-empty list → the tree container mounts + rows are inside it', () => {
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')] })
    render(<AgentList />)
    const tree = document.querySelector('[data-testid="agent-tree"]')
    expect(tree).toBeTruthy()
    expect(tree?.querySelector('[data-agent-row="a1"]')).toBeTruthy()
  })

  it('running ∪ reserved rows render + display name = cwd basename (name not stored)', () => {
    useAgentStore.setState({
      agents: [agent('a1', 'C:/work/engram')],
      profiles: [profile('p1', '/home/me/reserved-proj')],
    })
    render(<AgentList />)
    expect(document.querySelector('[data-agent-row="a1"]')).toBeTruthy()
    expect(document.querySelector('[data-agent-row="p1"]')).toBeTruthy()
    expect(screen.getByText('engram')).toBeTruthy()
    expect(screen.getByText('reserved-proj')).toBeTruthy()
  })

  // Measure both observation surfaces together: data-agent-glyph-shape (stable token) and the real lucide svg (class + fill).
  //   A regression where only the attribute is right but the icon is not drawn cannot be caught by the token alone.
  it('status shows as an icon shape (Failed=✗ / reserved=empty circle)', () => {
    useAgentStore.setState({
      agents: [agent('a1', 'C:/w', { type: 'Failed', message: 'x' })],
      profiles: [profile('p1', 'C:/r')],
    })
    render(<AgentList />)
    const failGlyph = document.querySelector('[data-agent-row="a1"] [data-agent-glyph]') as HTMLElement
    const resGlyph = document.querySelector('[data-agent-row="p1"] [data-agent-glyph]') as HTMLElement
    expect(failGlyph.dataset.agentGlyphShape).toBe('x')
    expect(resGlyph.dataset.agentGlyphShape).toBe('circle')
    expect(failGlyph.querySelector('svg.lucide-x')).toBeTruthy()
    expect(resGlyph.querySelector('svg.lucide-circle')).toBeTruthy()
  })

  // Running and Reserved are the same circle so **fill is the only shape difference** (on e-ink color is neutralized and only this
  //   remains — ADR-0062). So fill is pinned directly in the DOM.
  it('Running=filled circle / Reserved=empty circle — fill tells the two states apart', () => {
    useAgentStore.setState({
      agents: [agent('a1', 'C:/w')],
      profiles: [profile('p1', 'C:/r')],
    })
    render(<AgentList />)
    const runSvg = document.querySelector('[data-agent-row="a1"] [data-agent-glyph] svg') as SVGElement
    const resSvg = document.querySelector('[data-agent-row="p1"] [data-agent-glyph] svg') as SVGElement
    expect(runSvg.classList.contains('lucide-circle')).toBe(true)
    expect(resSvg.classList.contains('lucide-circle')).toBe(true)
    expect(runSvg.getAttribute('fill')).toBe('currentColor')
    expect(resSvg.getAttribute('fill')).toBe('none')
  })

  // Hanging the selection mark on background color alone silently vanishes per theme — the trigger was a real incident (2026-08-23) where `--surface-elevated` was used and
  //   in light · e-ink became the same color as the panel background so the selection was invisible. jsdom does not resolve CSS variables so it cannot measure the *color itself*,
  //   so the machine-readable channel (`data-selected`) carries the regression net instead.
  it('the selected row is identified not by color but also by data-selected (theme-independent channel)', () => {
    useAgentStore.setState({ agents: [agent('a1', 'C:/w'), agent('a2', 'C:/x')] })
    render(<AgentList />)
    fireEvent.click(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    expect(document.querySelector('[data-agent-row="a1"]')?.getAttribute('data-selected')).toBe('true')
    expect(document.querySelector('[data-agent-row="a2"]')?.getAttribute('data-selected')).toBe('false')
  })

  // If the selection background regresses to *the same token* as the panel background token it is invisible depending on theme. jsdom cannot
  //   compare values so it is pinned in the weak form "does not use the same var".
  it('the selection background does not use the panel background (--bg-secondary) as is', () => {
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')] })
    render(<AgentList />)
    fireEvent.click(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    const bg = (document.querySelector('[data-agent-row="a1"]') as HTMLElement).style.background
    expect(bg).not.toBe('var(--bg-secondary)')
    expect(bg).not.toBe('var(--surface-elevated)')
    expect(bg).toContain('var(')
  })

  it('reserved row double-click → spawnProfile (restore UX kept)', () => {
    useAgentStore.setState({ profiles: [profile('p1', 'C:/r')] })
    render(<AgentList />)
    fireEvent.doubleClick(document.querySelector('[data-agent-row="p1"]') as HTMLElement)
    expect(clientMock.spawnProfile).toHaveBeenCalledWith('p1', false)
  })

  it('style = variables only (the root background is a var reference)', () => {
    render(<AgentList />)
    const root = document.querySelector('[data-agent-list]') as HTMLElement
    expect(root.style.background).toContain('var(')
  })
})

// ── ★Background (bg) menu removed = bubbles to the unified slot menu (ADR-0064)★ ──────────────────────
describe('background right-click = no own menu (bubbles to the unified slot menu)', () => {
  it('background right-click → the old bg "create agent" menu/picker does not appear (removed)', () => {
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-list]') as HTMLElement)
    expect(document.querySelector('[data-agent-create]')).toBeNull()
    expect(document.querySelector('[data-agent-picker]')).toBeNull()
  })

  it('a background right-click event does not stopPropagation (bubbles so the parent unified slot menu receives it)', () => {
    render(<AgentList />)
    const pane = document.querySelector('[data-agent-list]') as HTMLElement
    // Regression safety net for removing the old stopPropagation.
    const ev = new MouseEvent('contextmenu', { bubbles: true, cancelable: true })
    const stopSpy = vi.spyOn(ev, 'stopPropagation')
    pane.dispatchEvent(ev)
    expect(stopSpy).not.toHaveBeenCalled()
  })
})

// ── Row right-click menu: kill · rename wired / restart disabled (ADR-0061 enrichment) ──────────
describe('row right-click menu', () => {
  it('carries the overlay marker only while open (ADR-0237) — it disappears with it when a document Esc closes it', () => {
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')] })
    render(<AgentList />)
    expect(document.querySelector(OVERLAY_SELECTOR)).toBeNull()
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    expect(document.querySelector(OVERLAY_SELECTOR)?.textContent).toContain('종료')
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(document.querySelector(OVERLAY_SELECTOR)).toBeNull()
  })

  it('running row: kill calls killAgent, restart is "coming soon" disabled (no-op)', () => {
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')] })
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    fireEvent.click(screen.getByText('재시작 (준비 중)'))
    expect(clientMock.killAgent).not.toHaveBeenCalled()
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    fireEvent.click(screen.getByText('종료'))
    expect(clientMock.killAgent).toHaveBeenCalledWith('a1')
  })

  // ★Inline-edit keys (Enter/Escape) fire on the ROW div★: the keydown handler is on the always-rendered row div
  //   (not the input). The onKeyDown of an input conditionally mounted inside react-window virtualization does not get a synthetic event because React 19 event
  //   delegation cannot attach a keydown listener to the root re-mounted from the 2nd time on (a react-window+jsdom
  //   interaction — click/blur/change fine, only keydown). So the component raised keydown to the row div, and the test also
  //   fires keydown at the element that really owns the handler (the row div) (input value changes are still verified by change).
  it('running row rename → inline input → Enter commit → calls renameProfile(id, trimmed)', () => {
    // An ad-hoc running row without a profile has no rename target, but here a matching profile is placed to verify the override save path.
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')], profiles: [profile('a1', 'C:/w')] })
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    fireEvent.click(screen.getByText('이름 변경'))
    const input = document.querySelector('[data-agent-rename-input="a1"]') as HTMLInputElement
    expect(input).toBeTruthy()
    fireEvent.change(input, { target: { value: '  내 에이전트  ' } })
    fireEvent.keyDown(document.querySelector('[data-agent-row="a1"]') as HTMLElement, { key: 'Enter' })
    expect(clientMock.renameProfile).toHaveBeenCalledWith('a1', '내 에이전트')
  })

  it('reserved row rename → Esc cancel → renameProfile not fired (revert)', () => {
    useAgentStore.setState({ profiles: [profile('p1', 'C:/r')] })
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-row="p1"]') as HTMLElement)
    fireEvent.click(screen.getByText('이름 변경'))
    const input = document.querySelector('[data-agent-rename-input="p1"]') as HTMLInputElement
    fireEvent.change(input, { target: { value: '바뀐이름' } })
    fireEvent.keyDown(document.querySelector('[data-agent-row="p1"]') as HTMLElement, { key: 'Escape' })
    expect(clientMock.renameProfile).not.toHaveBeenCalled()
    expect(document.querySelector('[data-agent-rename-input="p1"]')).toBeNull()
  })

  // ADR-0373 `draftStart` (the same rule as TabBar's seed): the baseline is the name the edit started from.
  it('the name changes from outside while editing, then Enter without editing → no renameProfile', () => {
    useAgentStore.setState({ profiles: [profile('p2', 'C:/x', 0, '고정명')] })
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-row="p2"]') as HTMLElement)
    fireEvent.click(screen.getByText('이름 변경'))
    act(() => {
      useAgentStore.setState({ profiles: [profile('p2', 'C:/x', 0, '바깥에서 바꾼 이름')] })
    })
    expect((document.querySelector('[data-agent-rename-input="p2"]') as HTMLInputElement).value).toBe('고정명')
    fireEvent.keyDown(document.querySelector('[data-agent-row="p2"]') as HTMLElement, { key: 'Enter' })
    expect(clientMock.renameProfile).not.toHaveBeenCalled()
  })

  it('rename unchanged (same as the display name) → renameProfile not fired', () => {
    useAgentStore.setState({ profiles: [profile('p2', 'C:/x', 0, '고정명')] })
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-row="p2"]') as HTMLElement)
    fireEvent.click(screen.getByText('이름 변경'))
    // The seeded draft = the current display name (the fixed-name fixture).
    fireEvent.keyDown(document.querySelector('[data-agent-row="p2"]') as HTMLElement, { key: 'Enter' })
    expect(clientMock.renameProfile).not.toHaveBeenCalled()
  })

  // Tauri `drag.js` blocks the default action (focus move) of pressing the window-drag area. The imitation = a listener registered earlier on the document bubble phase — it also cuts the
  //   later listeners (jsdom does not move focus on a press so the default-action side is already the same).
  it('pressing the window-drag area (title row) during rename → commits like losing focus · pressing outside the drag area keeps the edit', () => {
    const grab = document.createElement('div')
    grab.setAttribute('data-tauri-drag-region', '')
    document.body.appendChild(grab)
    const dragJs = (e: MouseEvent) => {
      if (e.target !== grab) return
      e.preventDefault()
      e.stopImmediatePropagation()
    }
    document.addEventListener('mousedown', dragJs)
    try {
      useAgentStore.setState({ profiles: [profile('p1', 'C:/r')] })
      render(<AgentList />)
      fireEvent.contextMenu(document.querySelector('[data-agent-row="p1"]') as HTMLElement)
      fireEvent.click(screen.getByText('이름 변경'))
      const input = () => document.querySelector('[data-agent-rename-input="p1"]') as HTMLInputElement
      expect(document.activeElement).toBe(input())
      // The virtual list may re-attach the input field, so the field is re-found each time.
      fireEvent.change(input(), { target: { value: '  새이름  ' } })

      fireEvent.mouseDown(document.body)
      expect(document.activeElement).toBe(input())
      expect(clientMock.renameProfile).not.toHaveBeenCalled()

      fireEvent.mouseDown(grab)
      expect(clientMock.renameProfile).toHaveBeenCalledWith('p1', '새이름')
      expect(document.querySelector('[data-agent-rename-input="p1"]')).toBeNull()
    } finally {
      document.removeEventListener('mousedown', dragJs)
      grab.remove()
    }
  })
})

// ── Symptom 4 regression safety net: operating one node does not drop sibling nodes ────────────────────
//   Targets the bug report ("with 3 in the tree, operating one makes the other 2 disappear"). The rename path
//   leaves the store mirror (the whole agents/profiles) as is without an optimistic update, so the sibling rows drawn by mergeTreeNodes
//   must not vanish from the render. Also a rename success must continue to refreshProfiles (the authoritative-list re-apply safety net — against a lost
//   broadcast) symmetric with spawn/delete. If the old regression (overwriting with a partial list) recurs this assertion breaks.
describe('symptom 4 regression: renaming one node does not drop sibling nodes', () => {
  it('rename one of 3 nodes (running 1 + reserved 2) → the other 2 node rows stay + refreshProfiles is called', () => {
    useAgentStore.setState({
      agents: [agent('a1', 'C:/work/aaa')],
      profiles: [profile('p1', 'C:/r/bbb', 1), profile('p2', 'C:/r/ccc', 2)],
    })
    render(<AgentList />)
    expect(document.querySelector('[data-agent-row="a1"]')).toBeTruthy()
    expect(document.querySelector('[data-agent-row="p1"]')).toBeTruthy()
    expect(document.querySelector('[data-agent-row="p2"]')).toBeTruthy()

    fireEvent.contextMenu(document.querySelector('[data-agent-row="p1"]') as HTMLElement)
    fireEvent.click(screen.getByText('이름 변경'))
    const input = document.querySelector('[data-agent-rename-input="p1"]') as HTMLInputElement
    fireEvent.change(input, { target: { value: '새이름' } })
    // Enter commit is owned by the row div keydown (comment above — avoiding react-window+React19). Fire on the row div.
    fireEvent.keyDown(document.querySelector('[data-agent-row="p1"]') as HTMLElement, { key: 'Enter' })

    expect(clientMock.renameProfile).toHaveBeenCalledWith('p1', '새이름')
    expect(document.querySelector('[data-agent-row="a1"]')).toBeTruthy()
    expect(document.querySelector('[data-agent-row="p1"]')).toBeTruthy()
    expect(document.querySelector('[data-agent-row="p2"]')).toBeTruthy()
    expect(document.querySelectorAll('[data-agent-row]').length).toBe(3)
  })

  it('rename success → refreshProfiles (authoritative-list re-apply safety net) is called — symmetric with spawn/delete', async () => {
    useAgentStore.setState({ profiles: [profile('p1', 'C:/r/bbb', 1)] })
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-row="p1"]') as HTMLElement)
    fireEvent.click(screen.getByText('이름 변경'))
    const input = document.querySelector('[data-agent-rename-input="p1"]') as HTMLInputElement
    fireEvent.change(input, { target: { value: '새이름' } })
    await act(async () => {
      // Enter commit is owned by the row div keydown (comment above). Fire on the row div.
      fireEvent.keyDown(document.querySelector('[data-agent-row="p1"]') as HTMLElement, { key: 'Enter' })
      // Flush the microtask in which .then(refreshProfiles) runs after the renameProfile Promise resolves.
      await Promise.resolve()
    })
    expect(refreshProfilesMock).toHaveBeenCalledTimes(1)
  })
})

// ── FIX#1: sync in-flight guard (useRef) — rapid clicks collapse into one fire ─────────────
describe('sync in-flight guard (useRef, FIX#1)', () => {
  // Hold spawnProfile as an unresolved Promise so the second doubleClick arrives while still in flight.
  //   With useState alone both pass via a stale closure before the re-render commits, but the busyRef sync guard blocks at once.
  it('reserved row double-click spam → spawnProfile only once (busyRef sync block)', () => {
    let resolveSpawn: (() => void) | undefined
    clientMock.spawnProfile.mockImplementationOnce(
      () => new Promise<{ id: string }>(res => { resolveSpawn = () => res({ id: 'p1' }) }),
    )
    useAgentStore.setState({ profiles: [profile('p1', 'C:/r')] })
    render(<AgentList />)
    const row = document.querySelector('[data-agent-row="p1"]') as HTMLElement
    fireEvent.doubleClick(row)
    fireEvent.doubleClick(row)
    expect(clientMock.spawnProfile).toHaveBeenCalledTimes(1)
    resolveSpawn?.() // cleanup (avoid an unresolved handle)
  })

  it('running row kill spam → killAgent only once (busyRef sync block)', () => {
    clientMock.killAgent.mockImplementationOnce(() => new Promise<undefined>(() => {})) // never resolves
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')] })
    render(<AgentList />)
    // menu → click kill (the menu closes on click, so reopen each time).
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    fireEvent.click(screen.getByText('종료'))
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    fireEvent.click(screen.getByText('종료'))
    expect(clientMock.killAgent).toHaveBeenCalledTimes(1)
  })

  it('open spam → assignAgent only once (busyRef sync block)', () => {
    assignAgentMock.mockImplementationOnce(() => new Promise<undefined>(() => {})) // never resolves
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')] })
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    fireEvent.click(screen.getByText('열기'))
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    fireEvent.click(screen.getByText('열기'))
    expect(assignAgentMock).toHaveBeenCalledTimes(1)
  })
})

// ── ★A double-clicked row does not claim a state until the answer arrives★ ────────────────────────────
//   A measured defect (2026-08-24): 2.4s after the double-click the activation RPC had already **refused**, yet the row drew a
//   green filled circle for 4.2 more seconds — next to the 「failed」 badge. Green is drawn following status, and that status becomes
//   Running optimistically the moment the session exists (core) and stays until the process death is observed.
//   The four properties pinned here: ① in-flight = spinner ② refusal = immediate forbidden marker ③ success = usual glyph
//   ④ while waiting, green never appears by any path.
//
//   ★No time is measured★: this repo has precedent of tests made flaky by wall-clock assertions. Instead the spawnProfile
//   Promise is held directly (unresolved/rejected/resolved) and only microtasks are flushed to make the span deterministic.
describe('activation progress/refusal drives the row glyph (green-lie regression safety net)', () => {
  const rowOf = (id: string) => document.querySelector(`[data-agent-row="${id}"]`) as HTMLElement
  const glyphOf = (id: string) =>
    document.querySelector(`[data-agent-row="${id}"] [data-agent-glyph]`) as HTMLElement

  /** Deterministically drain the microtasks that run through .catch → .finally after reject/resolve (not a timer). */
  async function flush(): Promise<void> {
    await act(async () => {
      await Promise.resolve()
      await Promise.resolve()
      await Promise.resolve()
    })
  }

  it('①④ after a double-click before the answer → spinner, and even if Running rises in the roster meanwhile it is not green', () => {
    let resolveSpawn: (() => void) | undefined
    clientMock.spawnProfile.mockImplementationOnce(
      () => new Promise<{ id: string }>(res => { resolveSpawn = () => res({ id: 'p1' }) }),
    )
    useAgentStore.setState({ profiles: [profile('p1', 'C:/r')] })
    render(<AgentList />)
    fireEvent.doubleClick(rowOf('p1'))

    expect(glyphOf('p1').dataset.agentGlyphShape).toBe('pending')
    expect(glyphOf('p1').querySelector('svg.lucide-loader')).toBeTruthy()
    // Rotation is owned by the stylesheet (agentGlyph.css) — the component only attaches the class.
    expect(glyphOf('p1').classList.contains('engram-glyph-spin')).toBe(true)
    expect(glyphOf('p1').style.color).not.toBe('var(--status-running)')
    // Hover also follows the same verdict — if an old failure text/cwd appeared while waiting the glyph and words would disagree.
    expect(rowOf('p1').getAttribute('title')).toBe('진행 중…')

    // ★The exact span of the defect★: the backend raises an optimistic Running into the roster together with session creation. Even so
    //   it is not yet the answer, so it must not switch to green.
    act(() => { useAgentStore.setState({ agents: [agent('p1', 'C:/r')] }) })
    expect(glyphOf('p1').dataset.agentGlyphShape).toBe('pending')
    expect(glyphOf('p1').dataset.agentGlyphShape).not.toBe('filled-circle')
    expect(glyphOf('p1').style.color).not.toBe('var(--status-running)')

    resolveSpawn?.() // cleanup (avoid an unresolved handle)
  })

  it('② refusal → the forbidden marker already before the lastFailure record arrives (the same moment as the badge)', async () => {
    clientMock.spawnProfile.mockImplementationOnce(() =>
      Promise.reject(new Error('no conversation to resume')),
    )
    // last_failure = null — the record not having arrived yet is this test's premise.
    useAgentStore.setState({ profiles: [profile('p1', 'C:/r')] })
    render(<AgentList />)
    fireEvent.doubleClick(rowOf('p1'))
    await flush()

    expect(useAgentStore.getState().profiles[0].last_failure).toBeNull()
    expect(glyphOf('p1').dataset.agentGlyphShape).toBe('blocked')
    expect(glyphOf('p1').querySelector('svg.lucide-ban')).toBeTruthy()
    expect(glyphOf('p1').style.color).toBe('var(--status-blocked)')
    // The 「failed」 badge = proof the frontend already held the answer. The glyph switches at the same moment.
    expect(rowOf('p1').textContent).toContain('실패')
  })

  it('② after a refusal, even if the process is still alive with status=Running it does not return to green', async () => {
    clientMock.spawnProfile.mockImplementationOnce(() => Promise.reject(new Error('rejected')))
    useAgentStore.setState({ profiles: [profile('p1', 'C:/r')] })
    render(<AgentList />)
    fireEvent.doubleClick(rowOf('p1'))
    await flush()
    // The span where a dying process stays in the roster as Running (measured +2.4s ~ +6.6s).
    act(() => { useAgentStore.setState({ agents: [agent('p1', 'C:/r')] }) })

    expect(glyphOf('p1').dataset.agentGlyphShape).toBe('blocked')
    expect(glyphOf('p1').style.color).not.toBe('var(--status-running)')
  })

  it('③ success → the usual status glyph (the green filled circle comes only here)', async () => {
    clientMock.spawnProfile.mockImplementationOnce(async () => ({ id: 'p1' }))
    useAgentStore.setState({ profiles: [profile('p1', 'C:/r')] })
    render(<AgentList />)
    fireEvent.doubleClick(rowOf('p1'))
    await flush()
    act(() => {
      useAgentStore.setState({ agents: [agent('p1', 'C:/r')], profiles: [profile('p1', 'C:/r')] })
    })

    expect(glyphOf('p1').dataset.agentGlyphShape).toBe('filled-circle')
    expect(glyphOf('p1').querySelector('svg.lucide-circle')).toBeTruthy()
    expect(glyphOf('p1').style.color).toBe('var(--status-running)')
    expect(glyphOf('p1').classList.contains('engram-glyph-spin')).toBe(false)
  })

  it('retry after a refusal → the spinner beats the forbidden marker (the past answer is not the current fact)', async () => {
    clientMock.spawnProfile.mockImplementationOnce(() => Promise.reject(new Error('rejected')))
    useAgentStore.setState({ profiles: [profile('p1', 'C:/r')] })
    render(<AgentList />)
    fireEvent.doubleClick(rowOf('p1'))
    await flush()
    expect(glyphOf('p1').dataset.agentGlyphShape).toBe('blocked')

    let resolveSpawn: (() => void) | undefined
    clientMock.spawnProfile.mockImplementationOnce(
      () => new Promise<{ id: string }>(res => { resolveSpawn = () => res({ id: 'p1' }) }),
    )
    fireEvent.doubleClick(rowOf('p1'))
    expect(glyphOf('p1').dataset.agentGlyphShape).toBe('pending')
    resolveSpawn?.()
  })
})

// ── ★"open" safety net: control-slot focus excluded (selectOpenTarget integration, ADR-0066 refinement)★ ──────────────
//   The click-to-focus gate will no longer seat focus on a control slot, but even in existing/edge backend states (the tree
//   slot already focused or focus=null) "open" must not overwrite a control slot and must fall back to an empty slot.
//   With no empty slot either, nothing is assigned (inline failure, no clobbering).
describe('open safety net — control-slot focus excluded (selectOpenTarget)', () => {
  function openRow(agentId: string): void {
    fireEvent.contextMenu(document.querySelector(`[data-agent-row="${agentId}"]`) as HTMLElement)
    fireEvent.click(screen.getByText('열기'))
  }

  it('focus on the tree (agent_list) + an empty slot exists → assignAgent to the empty slot instead of the tree', () => {
    selectViewMock.mockReturnValue({
      layout: {
        type: 'split', id: 'root', dir: 'left_right', ratio: 0.3,
        a: { type: 'slot', id: 'tree', content: { type: 'agent_list' } },
        b: { type: 'slot', id: 'empty', content: { type: 'empty' } },
      },
      focusedSlotId: 'tree',
      foreignSlots: [],
    })
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')] })
    render(<AgentList />)
    openRow('a1')
    expect(assignAgentMock).toHaveBeenCalledWith('main-view', 'empty', 'a1')
  })

  // TRD S21-storage §6-2: an unknown content slot is carried as `empty` — if the cache's foreignSlots is not passed it would overwrite that original.
  it('focus on an unknown content slot + an empty slot exists → assignAgent to the empty slot instead of that slot', () => {
    selectViewMock.mockReturnValue({
      layout: {
        type: 'split', id: 'root', dir: 'left_right', ratio: 0.5,
        a: { type: 'slot', id: 'foreign', content: { type: 'empty' } },
        b: { type: 'slot', id: 'empty', content: { type: 'empty' } },
      },
      focusedSlotId: 'foreign',
      foreignSlots: ['foreign'],
    })
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')] })
    render(<AgentList />)
    openRow('a1')
    expect(assignAgentMock).toHaveBeenCalledWith('main-view', 'empty', 'a1')
  })

  it('focus on the usage slot (usage) + no empty slot → assignAgent not called + failure inline badge', () => {
    selectViewMock.mockReturnValue({
      layout: {
        type: 'split', id: 'root', dir: 'left_right', ratio: 0.5,
        a: { type: 'slot', id: 'usage', content: { type: 'usage', show_claude: true, show_codex: true } },
        b: { type: 'slot', id: 'busy', content: { type: 'agent', agent_id: 'other' } },
      },
      focusedSlotId: 'usage',
      foreignSlots: [],
    })
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')] })
    render(<AgentList />)
    openRow('a1')
    expect(assignAgentMock).not.toHaveBeenCalled()
    const row = document.querySelector('[data-agent-row="a1"]') as HTMLElement
    expect(row.textContent).toContain('실패')
  })

  it('no active view/cache (selectView=null) → assignAgent not called + failure inline', () => {
    selectViewMock.mockReturnValue(null)
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')] })
    render(<AgentList />)
    openRow('a1')
    expect(assignAgentMock).not.toHaveBeenCalled()
    const row = document.querySelector('[data-agent-row="a1"]') as HTMLElement
    expect(row.textContent).toContain('실패')
  })
})

// ── FIX#3: reserved row "delete" → deleteProfile (regression restored) ────────────────────────
//   The menu label is 'delete' (rowCancelReserved). The behavior is deleteProfile as is.
describe('delete (deleteProfile, FIX#3)', () => {
  it('reserved row right-click → "delete" → calls deleteProfile(id)', () => {
    useAgentStore.setState({ profiles: [profile('p1', 'C:/r')] })
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-row="p1"]') as HTMLElement)
    fireEvent.click(screen.getByText('삭제'))
    expect(clientMock.deleteProfile).toHaveBeenCalledWith('p1')
  })

  it('a running row menu has no "delete" (reserved only)', () => {
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')] })
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    expect(screen.queryByText('삭제')).toBeNull()
  })
})

// ── The ROW menu keeps stopPropagation (item-targeted, ADR-0064) ─────────────────
describe('row right-click stopPropagation (the row menu wins over the unified menu)', () => {
  it('a row right-click event does stopPropagation (so the parent unified slot menu does not appear)', () => {
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')] })
    render(<AgentList />)
    const row = document.querySelector('[data-agent-row="a1"]') as HTMLElement
    const ev = new MouseEvent('contextmenu', { bubbles: true, cancelable: true })
    const stopSpy = vi.spyOn(ev, 'stopPropagation')
    row.dispatchEvent(ev)
    expect(stopSpy).toHaveBeenCalled()
  })
})

// ── Close an open row menu with Escape ──────────────────────────────────────────────
describe('close the row menu with Escape', () => {
  it('row menu open → Escape → closed', () => {
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')] })
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    expect(screen.getByText('열기')).toBeTruthy()
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(screen.queryByText('열기')).toBeNull()
  })
})

// ── The row menu closes when the target disappears from the list (stale-target guard) ──────────────────
//   rowMenu state outlives its row: even if the list changes and the target agentId drops out of rows (kill/delete/
//   last agent removed) rowMenu stays. The tree container unmounts on the empty transition so the menu vanishes briefly, and
//   when a new agent appears and it remounts non-empty a menu aimed at the departed agentId revives at stale coordinates.
describe('row menu closes when the target disappears (stale-target guard)', () => {
  it('row menu open → target agent removed (list empty) → menu disappears', () => {
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')] })
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    expect(screen.getByText('열기')).toBeTruthy()
    // Wrap in act to flush the reset effect.
    act(() => useAgentStore.setState({ agents: [] }))
    expect(screen.queryByText('열기')).toBeNull()
  })

  it('after removing the target another agent appears (non-empty remount) → the stale menu does not revive', () => {
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')] })
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    expect(screen.getByText('열기')).toBeTruthy()
    // Flush each transition with act.
    act(() => useAgentStore.setState({ agents: [] }))
    act(() => useAgentStore.setState({ agents: [agent('a2', 'C:/x')] }))
    expect(screen.queryByText('열기')).toBeNull()
    expect(document.querySelector('[data-agent-row="a2"]')).toBeTruthy()
  })

  it('among several rows only the target removed (list stays non-empty) → menu disappears', () => {
    useAgentStore.setState({ agents: [agent('a1', 'C:/w'), agent('a2', 'C:/x')] })
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    expect(screen.getByText('열기')).toBeTruthy()
    // Flush with act.
    act(() => useAgentStore.setState({ agents: [agent('a2', 'C:/x')] }))
    expect(screen.queryByText('열기')).toBeNull()
  })
})

// ── Hierarchical nesting render + collapse/expand (react-arborist, ADR-0072) ──────────────────────────
describe('hierarchical nesting (parent_id → child under parent, ADR-0072)', () => {
  it('a child (parent_id=A) renders nested under parent A + the child row is indented more', () => {
    // Rendered with the children also expanded since openByDefault.
    useAgentStore.setState({
      profiles: [profile('A', 'C:/a', 1), profile('B', 'C:/b', 2, null, 'A')],
    })
    render(<AgentList />)
    const parentRow = document.querySelector('[data-agent-row="A"]') as HTMLElement
    const childRow = document.querySelector('[data-agent-row="B"]') as HTMLElement
    expect(parentRow).toBeTruthy()
    expect(childRow).toBeTruthy()
    expect(parentRow.querySelector('[data-agent-toggle="1"]')).toBeTruthy()
    expect(childRow.querySelector('[data-agent-toggle="1"]')).toBeNull()
    const parentPad = parseInt(parentRow.style.paddingLeft || '0', 10)
    const childPad = parseInt(childRow.style.paddingLeft || '0', 10)
    expect(childPad).toBeGreaterThan(parentPad)
  })

  it('parent toggle click → children collapse (disappear), click again → expand', () => {
    useAgentStore.setState({
      profiles: [profile('A', 'C:/a', 1), profile('B', 'C:/b', 2, null, 'A')],
    })
    render(<AgentList />)
    expect(document.querySelector('[data-agent-row="B"]')).toBeTruthy()
    const toggle = document.querySelector('[data-agent-row="A"] [data-agent-toggle="1"]') as HTMLElement
    act(() => { fireEvent.click(toggle) })
    expect(document.querySelector('[data-agent-row="B"]')).toBeNull()
    // The toggle is a re-lookup — the parent row stays in the collapsed state.
    const toggle2 = document.querySelector('[data-agent-row="A"] [data-agent-toggle="1"]') as HTMLElement
    act(() => { fireEvent.click(toggle2) })
    expect(document.querySelector('[data-agent-row="B"]')).toBeTruthy()
  })
})


// ── ADR-0373: a language switch neither freezes a shown error nor saves an untouched draft ──────────────────
describe('AgentList — language switch', () => {
  afterEach(() => {
    act(() => {
      setLanguage('ko')
    })
  })

  const rowTitle = (id: string) =>
    (document.querySelector(`[data-agent-row="${id}"]`) as HTMLElement).getAttribute('title')

  /** Stands in for `App`, which re-renders the tree on a switch — AgentList itself does not subscribe. */
  function Root() {
    useLanguage()
    return <AgentList />
  }

  it('an error shown before the switch is shown in the new language after it', () => {
    selectViewMock.mockReturnValue(null)
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')] })
    render(<Root />)
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    fireEvent.click(screen.getByText('열기'))
    expect(rowTitle('a1')).toBe(t('agent.openFailedNoSlot'))
    expect(rowTitle('a1')).not.toBe('Open failed: no active view or focused slot.')

    act(() => {
      setLanguage('en')
    })

    expect(rowTitle('a1')).toBe('Open failed: no active view or focused slot.')
  })

  it('an error with a raw cause keeps the cause and translates the frame', async () => {
    clientMock.killAgent.mockRejectedValueOnce(new Error('boom'))
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')] })
    render(<Root />)
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    fireEvent.click(screen.getByText('종료'))
    await act(async () => {
      for (let i = 0; i < 5; i++) await Promise.resolve()
    })
    expect(rowTitle('a1')).toBe(t('agent.killFailed', { err: 'Error: boom' }))

    act(() => {
      setLanguage('en')
    })

    expect(rowTitle('a1')).toBe('Stop failed: Error: boom')
  })

  it('a draft being edited is not saved by a switch (a blank-path name changes language under it)', () => {
    useAgentStore.setState({ profiles: [profile('p1', '')] })
    render(<Root />)
    fireEvent.contextMenu(document.querySelector('[data-agent-row="p1"]') as HTMLElement)
    fireEvent.click(screen.getByText('이름 변경'))
    const input = document.querySelector('[data-agent-rename-input="p1"]') as HTMLInputElement
    expect(input.value).toBe('(경로 없음)')

    act(() => {
      setLanguage('en')
    })
    fireEvent.keyDown(document.querySelector('[data-agent-row="p1"]') as HTMLElement, { key: 'Enter' })

    expect(clientMock.renameProfile).not.toHaveBeenCalled()
  })
})
