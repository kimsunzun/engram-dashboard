import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// ── Tauri / transport layer stubs ────────────────────────────────────────────────
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => undefined),
  Channel: class {
    onmessage: unknown = null
  },
}))
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => vi.fn()),
}))

// ── agentClient / clientFactory stub ─────────────────────────────────────────
// TerminalSlot imports agentClient internally, but this test mocks TerminalSlot itself,
// so no real call happens. clientFactory also uses Tauri invoke, so it is stubbed.
// killAgent is stubbed too — the SlotContextMenu 'kill agent' path calls it.
const clientMock = vi.hoisted(() => ({
  spawnAgent: vi.fn(async () => ({ id: 'spawned-agent-id' })),
  killAgent: vi.fn(async () => undefined),
}))
vi.mock('../../api/clientFactory', () => ({
  agentClient: {
    subscribeOutput: vi.fn(async () => ({ unsubscribe: vi.fn() })),
    writeStdin: vi.fn(async () => undefined),
    resizePty: vi.fn(async () => undefined),
    spawnAgent: (...args: unknown[]) => clientMock.spawnAgent(...(args as [])),
    killAgent: (...args: unknown[]) => clientMock.killAgent(...(args as [])),
    connectionState: 'down',
    // Connection-state surface — the slot reads it for the absence-veil verdict (notifies once on registration + returns a disposer).
    onConnectionStateChange: (cb: (s: string) => void) => {
      cb('down')
      return () => {}
    },
  },
  getAgentClient: vi.fn(),
}))

// ── agentStore stub — the LayoutLeaf drawn by ViewLayoutRenderer looks up caps via `useAgentStore(s => s.agents)`. ──
// FIX 1 (ADR-0041): the renderer branch depends on the store's AgentInfo presence and caps, so the test must control agents.
// A mutable holder is made with vi.hoisted and fed to the selector (TerminalSlot/RichSlot are stubs so
// their own useAgentStore calls are harmless). Reset in afterEach.
// ADR-0148: the three-way gate (roster not received / profile alive / profile absent) looks at agentsLoaded and profiles together.
const agentStoreState = vi.hoisted(() => ({
  agents: [] as unknown[],
  agentsLoaded: false,
  profiles: [] as unknown[],
  profilesLoaded: false,
  presets: [] as unknown[],
}))
vi.mock('../../store/agentStore', () => ({
  useAgentStore: Object.assign(
    (selector: (s: typeof agentStoreState) => unknown) => selector(agentStoreState),
    { getState: () => agentStoreState },
  ),
}))

// ── Native folder dialog stub (ADR-0064) — slot.createAgentHere / preset.add / agentlist.createAgent call
//    @tauri-apps/plugin-dialog open. The return (path/null) is swapped per test. ──
const dialogMock = vi.hoisted(() => ({ open: vi.fn(async () => null as string | null) }))
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: (...args: unknown[]) => dialogMock.open(...(args as [])),
}))

// ── Three slot stubs — only check mount presence and the props passed, with no real subscription/xterm ─────────────────
// ★Draw every prop passed★: what the parent passes down is this renderer's contract, so if props are not drawn,
//   drift is invisible in the DOM. ★If the epoch is revived here that value becomes a remount trigger itself,
//   and the moment the roster reaps on exit the value drops and the conversation being preserved is erased★ (regression case below).
vi.mock('../slot/TerminalSlot', () => ({
  default: ({ viewId, agentId }: { viewId: string; agentId: string }) => (
    <div data-testid="terminal-slot" data-agent-id={agentId} data-view-id={viewId} />
  ),
}))

vi.mock('../slot/RichSlot', () => ({
  default: ({ viewId, agentId }: { viewId?: string; agentId: string }) => (
    <div data-testid="rich-slot" data-agent-id={agentId} data-view-id={viewId} />
  ),
}))

vi.mock('../slot/DomSlot', () => ({
  default: ({ viewId, agentId }: { viewId: string; agentId: string }) => (
    <div data-testid="dom-slot" data-agent-id={agentId} data-view-id={viewId} />
  ),
}))

// ── PresetPalette stub (ADR-0060/0061) — only checks the preset_palette variant mounts, without preset CRUD wiring ──
vi.mock('../slot/PresetPalette', () => ({
  default: () => <div data-testid="preset-palette" />,
}))
// ── UsageSlot stub — only checks the usage variant mounts and the toggles and slot coordinates passed down (the screen is UsageSlot.test's job) ──
vi.mock('../slot/UsageSlot', () => ({
  default: ({
    content,
    viewId,
    slotId,
  }: {
    content: { show_claude: boolean; show_codex: boolean }
    viewId: string | null
    slotId: string
  }) => (
    <div
      data-testid="usage-slot"
      data-show-claude={String(content.show_claude)}
      data-show-codex={String(content.show_codex)}
      data-view-id={String(viewId)}
      data-slot-id={slotId}
    />
  ),
}))
// ── AgentList stub (ADR-0060/0062) — only checks the agent_list variant mounts (internal wiring is AgentList.test's job) ──
vi.mock('../agent/AgentList', () => ({
  default: () => <div data-testid="agent-list" />,
}))

// ── @xterm stub — TerminalSlot is not actually rendered, but this guards import resolution ────
vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    loadAddon = vi.fn()
    open = vi.fn()
    reset = vi.fn()
    write = vi.fn()
    onData = vi.fn(() => ({ dispose: vi.fn() }))
    dispose = vi.fn()
    cols = 80
    rows = 24
  },
}))
vi.mock('@xterm/addon-fit', () => ({
  FitAddon: class {
    fit = vi.fn()
  },
}))

// ── Subject under test ────────────────────────────────────────────────────────────────
// ADR-0064: the unified menu resolves command references with buildSlotMenu(content.type) → the commands and contributions must be
//   registered in the registry. The manifest is side-effect imported to register the same as boot.
import '../../commands/contributions'
import ViewLayoutRenderer from './ViewLayoutRenderer'
import { setLanguage } from '../../i18n'
// The menu placement anchor gap (ADR-0143 decision 4) belongs to SlotContextMenu — here the coordinate expectations derive from that constant.
import { ANCHOR_GAP } from '../slot/SlotContextMenu'
import type { LayoutNode, SlotContent, SplitDir } from '../../api/layoutTypes'
import type { AgentInfo, Capabilities } from '../../api/types'
import { useViewStore } from '../../store/viewStore'
import { rectsFor } from './testing/rects'
import { __resetUiMetricsReportForTest } from './uiMetricsReport'

// When a leaf mounts it sends the area frame metrics once per webview (module state) — cleared every time so test order does not decide whether it was reported.
beforeEach(() => {
  __resetUiMetricsReportForTest()
})

afterEach(() => {
  cleanup()
  useViewStore.setState({ renderModeOverride: {} })
  agentStoreState.agents = []
  agentStoreState.agentsLoaded = false
  agentStoreState.profiles = []
  agentStoreState.profilesLoaded = false
})

// ── Helpers ──────────────────────────────────────────────────────────────────────
function slotNode(id: string, agentId: string | null): LayoutNode {
  return {
    type: 'slot',
    id,
    content: agentId != null ? { type: 'agent', agent_id: agentId } : { type: 'empty' },
  }
}

function splitNode(id: string, a: LayoutNode, b: LayoutNode, ratio = 0.5, dir: SplitDir = 'left_right'): LayoutNode {
  return { type: 'split', id, dir, ratio, a, b }
}

/**
 * Empty-slot placeholder = a `+` icon (ADR-0141 replaced the old `Slot <id8>` / `— empty —` text).
 * Since ADR-0143 it is a pure picture, not a button, so it has no role or accessible name — the svg directly under the area border element is the only surface.
 * ADR-0227: an area is split into a frame (`[data-slot-id]` — position·events) and the border element inside it (`[data-slot-border]` — border·alignment·content).
 *   Looking directly under the frame always gives an empty array so a 「no icon」 assertion would pass vacuously.
 */
function emptyIcons(): HTMLElement[] {
  return Array.from(document.querySelectorAll<HTMLElement>('[data-slot-border] > svg'))
}

/** Area frame (`[data-slot-id]`) — the outer element carrying the rectangle position, click and right-click. */
function frameOf(slotId: string): HTMLElement {
  const el = document.querySelector<HTMLElement>(`[data-slot-id="${slotId}"]`)
  if (el === null) throw new Error(`area frame ${slotId} is missing`)
  return el
}

/** Area border element — must be a direct child of the frame (outside the frame it would not be clipped by the frame's overflow:hidden). */
function borderOf(slotId: string): HTMLElement {
  const el = frameOf(slotId).querySelector<HTMLElement>(':scope > [data-slot-border]')
  if (el === null) throw new Error(`the border element of area ${slotId} is not directly under the frame`)
  return el
}

/**
 * The absence veil the leaf draws itself (`SlotUnavailableVeil`) — looks only directly under the border element. The slot component (a stub here) draws
 * its own veil inside itself, so it is narrowed to direct children to tell them apart.
 */
function leafVeil(slotId: string): HTMLElement | null {
  return borderOf(slotId).querySelector<HTMLElement>(':scope > [data-slot-dead="1"]')
}

/** The four rectangle custom properties carried on the frame (numbers). Throws if missing so an empty value is not read as 0 and matches vacuously. */
function frameRect(slotId: string): { x0: number; y0: number; x1: number; y1: number } {
  const s = frameOf(slotId).style
  const read = (name: string): number => {
    const raw = s.getPropertyValue(name)
    if (raw.trim() === '') throw new Error(`area frame ${slotId} has no ${name}`)
    return Number(raw)
  }
  return { x0: read('--x0'), y0: read('--y0'), x1: read('--x1'), y1: read('--y1') }
}

function contentSlotNode(id: string, content: SlotContent): LayoutNode {
  return { type: 'slot', id, content }
}

// Only caps matter so the other fields are minimal. structured=true → RichSlot, false → TerminalSlot branch.
function caps(structured: boolean): Capabilities {
  return {
    input: { raw: true, message: false, attachment: false },
    output: { terminal_bytes: !structured, structured, markdown: false, tool_events: false, usage: false },
    control: { resize: true, interrupt: true, cancel: false, graceful_shutdown: true },
    session: { resume: true, snapshot: false, cwd_env: true },
    model: { select: false, temperature: false, max_tokens: false },
  }
}

/** epoch is not used by this renderer's verdict but AgentInfo requires it, so it is filled (the value has no meaning). */
function agentInfo(id: string, structured: boolean, epoch = 2): AgentInfo {
  return {
    id,
    name: id,
    cwd: '/tmp',
    status: { type: 'Running' },
    cols: 80,
    rows: 24,
    epoch,
    capabilities: caps(structured),
  }
}

/** Seed AgentInfo into the store (FIX 1: a concrete renderer mounts only after caps arrive, so most tests need this). */
function seedAgents(...infos: AgentInfo[]): void {
  agentStoreState.agents = infos
}

// ── Test cases ─────────────────────────────────────────────────────────────

describe('ViewLayoutRenderer — slot branch', () => {
  it('slot with agent_id (non-structured caps) → TerminalSlot mounts and the agentId prop is passed', () => {
    const agentId = 'aaaa-bbbb-cccc-dddd'
    seedAgents(agentInfo(agentId, false))
    render(<ViewLayoutRenderer node={slotNode('s1', agentId)} focusedSlotId={null} />)
    const terminal = screen.getByTestId('terminal-slot')
    expect(terminal).toBeTruthy()
    expect(terminal.getAttribute('data-agent-id')).toBe(agentId)
  })

  it('slot with null agent_id → the `+` icon placeholder shows and there is no TerminalSlot (no old text)', () => {
    render(<ViewLayoutRenderer node={slotNode('s1', null)} focusedSlotId={null} />)
    expect(screen.queryByTestId('terminal-slot')).toBeNull()
    expect(emptyIcons()).toHaveLength(1)
    // Guards against a regression of the old placeholder text (ADR-0141).
    expect(screen.queryByText(/^Slot /)).toBeNull()
    expect(screen.queryByText('— empty —')).toBeNull()
  })

  it('focusedSlotId == node.id → the focus-ring overlay (inset box-shadow, accent 40%) shows above the content', () => {
    render(<ViewLayoutRenderer node={slotNode('s1', null)} focusedSlotId="s1" />)
    const border = borderOf('s1')
    // ADR-0066 (focus-ring): the ring is drawn as an absolute overlay *above* the content, not as the border (fixes the bug where a child filling 100% of an
    //   overflow:hidden slot covered the inset box-shadow). The anchor is the border element (ADR-0227).
    const overlay = [...border.querySelectorAll('div')].find(d => d.style.boxShadow.includes('accent'))
    expect(overlay).toBeTruthy()
    expect(overlay!.parentElement).toBe(border)
    // ★Actually measure the strength★: the old assertion was only `includes('accent')` so it passed even if reverted to 65% — a test that
    //   could not catch a value-changing regression while only its title advertised the strength (review finding 2026-08-23). Value = ADR-0168 decision 1.
    expect(overlay!.style.boxShadow).toContain('40%')
    expect(overlay!.style.pointerEvents).toBe('none')
    expect(overlay!.style.position).toBe('absolute')
    // The border color does not change with focus — the ring is the overlay's job.
    expect(border.style.borderColor).toBe('var(--border)')
    expect(border.style.borderColor).not.toContain('accent')
  })

  it('focusedSlotId != node.id → not focused: no ring overlay', () => {
    render(<ViewLayoutRenderer node={slotNode('s1', null)} focusedSlotId="s-other" />)
    const border = borderOf('s1')
    expect(border.style.borderColor).toBe('var(--border)')
    expect(border.style.borderColor).not.toContain('accent')
    const overlay = [...frameOf('s1').querySelectorAll('div')].find(d => d.style.boxShadow.includes('accent'))
    expect(overlay).toBeUndefined()
  })

  // ── ADR-0060/0061/0062: preset_palette·agent_list variant → each real renderer mounts (hasContent=true) ──
  it('content.type=preset_palette slot → PresetPalette mounts', () => {
    render(<ViewLayoutRenderer node={contentSlotNode('s1', { type: 'preset_palette' })} focusedSlotId={null} />)
    expect(screen.getByTestId('preset-palette')).toBeTruthy()
    // The preset palette is real content (hasContent=true) — there must be no centering flex or the palette layout breaks.
    //   Alignment is carried by the border element (ADR-0227) — the empty-slot positive pair below guards that the targeting is right.
    expect(borderOf('s1').style.justifyContent).not.toBe('center')
    expect(borderOf('s1').style.overflow).toBe('hidden')
  })

  it('content.type=agent_list slot (Slice C) → AgentList mounts (hasContent=true)', () => {
    render(<ViewLayoutRenderer node={contentSlotNode('s1', { type: 'agent_list' })} focusedSlotId={null} />)
    expect(screen.getByTestId('agent-list')).toBeTruthy()
    // A real renderer, not the empty placeholder — there must be no centering flex or the list layout breaks.
    expect(emptyIcons()).toHaveLength(0)
    expect(borderOf('s1').style.justifyContent).not.toBe('center')
    expect(borderOf('s1').style.overflow).toBe('hidden')
  })

  it('content.type=usage slot → UsageSlot receives the slot content (show_*) and mounts (hasContent=true)', () => {
    render(
      <ViewLayoutRenderer
        node={contentSlotNode('s1', { type: 'usage', show_claude: true, show_codex: false })}
        focusedSlotId={null}
      />,
    )
    const usage = screen.getByTestId('usage-slot')
    expect(usage.getAttribute('data-show-claude')).toBe('true')
    expect(usage.getAttribute('data-show-codex')).toBe('false')
    expect(emptyIcons()).toHaveLength(0)
    expect(borderOf('s1').style.justifyContent).not.toBe('center')
    expect(borderOf('s1').style.overflow).toBe('hidden')
  })

  // ★Positive pair★: the `!== 'center'` assertions above and below pass vacuously if the targeted element is wrong. With the same targeting the empty slot
  //   is also measured to actually show centering.
  it('the border element of an empty slot centers the placeholder (positive pair to confirm the targeting)', () => {
    render(<ViewLayoutRenderer node={slotNode('s1', null)} focusedSlotId={null} />)
    const border = borderOf('s1')
    expect(border.style.display).toBe('flex')
    expect(border.style.justifyContent).toBe('center')
    expect(border.style.alignItems).toBe('center')
    expect(emptyIcons()).toHaveLength(1)
    expect(emptyIcons()[0].parentElement).toBe(border)
  })

  // TRD S21-storage §6-2: an unknown content slot is carried as `empty` but is not empty — draw an icon placeholder instead of `+`
  //   (no text, notice line or veil — user decision 2026-10-06).
  it('a slot in foreignSlots → an 「unknown content」 icon placeholder (no `+`, text or veil), other empty slots stay `+`', () => {
    const node = splitNode('p', slotNode('foreign', null), slotNode('plain', null))
    const { slotRects, splitRects } = rectsFor(node)
    render(
      <ViewLayoutRenderer
        node={node}
        focusedSlotId={null}
        slotRects={slotRects}
        splitRects={splitRects}
        foreignSlots={['foreign']}
      />,
    )
    const foreign = borderOf('foreign')
    const mark = foreign.querySelector<HTMLElement>(':scope > [data-slot-foreign]')
    expect(mark).not.toBeNull()
    expect(mark!.getAttribute('role')).toBe('img')
    expect(mark!.getAttribute('aria-label')).toBe('알 수 없는 내용')
    expect(mark!.getAttribute('title')).toBe('알 수 없는 내용')
    expect(mark!.querySelector('svg')?.getAttribute('aria-hidden')).toBe('true')
    expect(foreign.textContent).toBe('')
    expect(foreign.querySelector(':scope > svg')).toBeNull()
    expect(foreign.querySelector('[data-slot-dead]')).toBeNull()
    expect(foreign.style.justifyContent).toBe('center')
    expect(borderOf('plain').querySelector(':scope > svg')).not.toBeNull()
    expect(borderOf('plain').querySelector('[data-slot-foreign]')).toBeNull()
    // The icon does not cut pointer-events, so a right-click on it also bubbles to the frame and must open the empty-slot menu.
    fireEvent.contextMenu(mark!)
    expect(screen.getByText('새 콘텐츠')).toBeTruthy()
  })

  it('the data-slot-id attribute is set to node.id (invariant for cdp verification)', () => {
    const id = 'test-slot-uuid'
    render(<ViewLayoutRenderer node={slotNode(id, null)} focusedSlotId={null} />)
    expect(document.querySelector(`[data-slot-id="${id}"]`)).toBeTruthy()
  })

  it('the wrapper of a slot with agent_id (caps arrived) has no centering flex (prevents polluting the terminal layout)', () => {
    seedAgents(agentInfo('some-agent-id', false))
    render(<ViewLayoutRenderer node={slotNode('s1', 'some-agent-id')} focusedSlotId={null} />)
    expect(screen.getByTestId('terminal-slot')).toBeTruthy()
    const border = borderOf('s1')
    expect(border.style.justifyContent).not.toBe('center')
    expect(border.style.alignItems).not.toBe('center')
    expect(border.style.overflow).toBe('hidden')
  })

  // ── FIX 1 (ADR-0041 replay ownership): do not mount a concrete renderer before caps arrive ──────────────
  it('agent assigned but no AgentInfo in the store → "agent connecting…" placeholder (no TerminalSlot/RichSlot)', () => {
    // With the store empty (= caps not arrived), only a neutral placeholder must show to avoid losing replay on a swap.
    render(<ViewLayoutRenderer node={slotNode('s1', 'not-in-store')} focusedSlotId={null} />)
    expect(screen.getByText('에이전트 연결 중…')).toBeTruthy()
    expect(screen.queryByTestId('terminal-slot')).toBeNull()
    expect(screen.queryByTestId('rich-slot')).toBeNull()
  })

  // ── ADR-0148: split "an agent not in the roster" three ways ────────────────────────────────
  //
  // One state (agent==null) carried three meanings. The real outcome of exit (kill) is that the reaper reaps the session,
  // **removes it from the roster and leaves the profile**, so the old gate drew a dead slot as 「connecting」 and tore
  // the view down — the daemon discards the replay ring with the session, so at that moment the conversation was permanently lost.
  describe('ADR-0148 — three branches of a slot without an agent', () => {
    const GONE = 'gone-agent'

    function profile(id: string, epoch = 3): unknown {
      return { id, name: id, cwd: '/tmp', display_name: null, parent_id: null, created_at: 0, epoch }
    }

    /** Mount once with a live agent to create the "memory". */
    function mountAlive(structured = true, epoch = 3) {
      seedAgents(agentInfo(GONE, structured, epoch))
      agentStoreState.agentsLoaded = true
      agentStoreState.profilesLoaded = true
      agentStoreState.profiles = [profile(GONE, epoch)]
      return render(<ViewLayoutRenderer node={slotNode('s1', GONE)} focusedSlotId={null} />)
    }

    // TRD S21-storage §6-8 · user decision 2026-10-06: replaces ADR-0149 (A)'s 「spawn pending is also connecting」 — once the list is
    //   received there is no indefinite 「connecting」, and the same absence veil as a disconnected or dead slot is drawn (no text or button).
    it('profile present + no mount memory → absence veil (no text or button, not 「connecting」)', () => {
      // This is the state right after restore following a tray exit: the profile exists and the daemon did not bring that agent up.
      agentStoreState.agentsLoaded = true
      agentStoreState.profilesLoaded = true
      agentStoreState.profiles = [profile(GONE)]
      render(<ViewLayoutRenderer node={slotNode('s1', GONE)} focusedSlotId={null} />)

      const veil = leafVeil('s1')
      expect(veil).not.toBeNull()
      expect(veil!.textContent).toBe('')
      expect(screen.queryByRole('button')).toBeNull()
      expect(borderOf('s1').textContent).toBe('')
      expect(screen.queryByText('에이전트 연결 중…')).toBeNull()
      expect(screen.queryByText('대상 없음')).toBeNull()
      // A slot that has never been up, so no subscription is made with a dead agent.
      expect(screen.queryByTestId('rich-slot')).toBeNull()
      expect(screen.queryByTestId('terminal-slot')).toBeNull()
    })

    // User decision 2026-10-06: the focus-ring color does not vary with slot state — the ring stands above the veil. jsdom does not paint, so only the
    //   inputs of the stacking rule (same z · tree order) are measured. The real color is GUI QA's job.
    it('focused slot without memory: the focus ring stands above the leaf absence veil', () => {
      agentStoreState.agentsLoaded = true
      agentStoreState.profilesLoaded = true
      agentStoreState.profiles = [profile(GONE)]
      render(<ViewLayoutRenderer node={slotNode('s1', GONE)} focusedSlotId="s1" />)

      const veil = leafVeil('s1')
      const ring = borderOf('s1').querySelector<HTMLElement>(':scope > [data-slot-focus-ring]')
      expect(veil).not.toBeNull()
      expect(ring).not.toBeNull()
      expect(veil!.className.split(' ')).toContain('z-20')
      expect(Number(ring!.style.zIndex)).toBe(20)
      expect(borderOf('s1').lastElementChild).toBe(ring)
      expect(ring!.style.pointerEvents).toBe('none')
    })

    it('even while the roster and profile list themselves are not received it is 「agent connecting…」 (does not leak into 「no target」)', () => {
      // profilesLoaded=false = refreshProfiles not arrived or failed. Leaking into 「no target」 here would
      //   unmount early the view that is preserving the conversation.
      agentStoreState.agentsLoaded = true
      agentStoreState.profilesLoaded = false
      agentStoreState.profiles = []
      render(<ViewLayoutRenderer node={slotNode('s1', GONE)} focusedSlotId={null} />)

      expect(screen.getByText('에이전트 연결 중…')).toBeTruthy()
      expect(screen.queryByText('대상 없음')).toBeNull()
    })

    it('even if the profile list is received, without the roster it is 「agent connecting…」 (does not leak into the absence veil)', () => {
      // With the roster not received agents=[] so even a running agent looks reserved.
      agentStoreState.agentsLoaded = false
      agentStoreState.profilesLoaded = true
      agentStoreState.profiles = [profile(GONE)]
      render(<ViewLayoutRenderer node={slotNode('s1', GONE)} focusedSlotId={null} />)

      expect(screen.getByText('에이전트 연결 중…')).toBeTruthy()
      expect(leafVeil('s1')).toBeNull()
    })

    it('when the list arrives it moves from 「connecting」 to the absence veil', () => {
      agentStoreState.profiles = [profile(GONE)]
      const { rerender } = render(<ViewLayoutRenderer node={slotNode('s1', GONE)} focusedSlotId={null} />)
      expect(screen.getByText('에이전트 연결 중…')).toBeTruthy()
      expect(leafVeil('s1')).toBeNull()

      agentStoreState.agentsLoaded = true
      agentStoreState.profilesLoaded = true
      rerender(<ViewLayoutRenderer node={slotNode('s1', GONE)} focusedSlotId={null} />)
      expect(screen.queryByText('에이전트 연결 중…')).toBeNull()
      expect(leafVeil('s1')).not.toBeNull()
    })

    // Activating in the tree puts it in the roster, and at that moment the slot attaches to the usual screen (the slot does not call activation).
    it('absence veil → when that agent enters the roster, the usual screen (no veil)', () => {
      agentStoreState.agentsLoaded = true
      agentStoreState.profilesLoaded = true
      agentStoreState.profiles = [profile(GONE)]
      const { rerender } = render(<ViewLayoutRenderer node={slotNode('s1', GONE)} focusedSlotId={null} />)
      expect(leafVeil('s1')).not.toBeNull()

      seedAgents(agentInfo(GONE, false))
      rerender(<ViewLayoutRenderer node={slotNode('s1', GONE)} focusedSlotId={null} />)
      expect(screen.getByTestId('terminal-slot').getAttribute('data-agent-id')).toBe(GONE)
      expect(leafVeil('s1')).toBeNull()
    })

    // ★F1 regression★: if slot props shake at exit a remount runs and the conversation being preserved is erased on the spot
    //   (the daemon ring also vanishes with the session so even resubscribing cannot revive it). Judged by "is the same DOM node still there" —
    //   looking only at text presence cannot tell a remount from a re-render.
    //   ★The defense is not passing the shaking value at all★ — so the case below guards "what is passed is only viewId and agentId"
    //   rather than "it does not shake whatever is passed" (bottom case).
    it('profile present + memory present → keeps the same node (conversation preserved)', () => {
      const { rerender } = mountAlive(true, 3)
      const before = screen.getByTestId('rich-slot')

      // Exit = disappears from the roster and only the profile remains.
      agentStoreState.agents = []
      rerender(<ViewLayoutRenderer node={slotNode('s1', GONE)} focusedSlotId={null} />)

      const after = screen.getByTestId('rich-slot')
      expect(Object.is(before, after)).toBe(true) // not remounted = conversation preserved
      expect(screen.queryByText('에이전트 연결 중…')).toBeNull()
      expect(screen.queryByText('대상 없음')).toBeNull()
      // An absence with memory is not the leaf's absence veil — it is ADR-0149 (B) as is, which keeps the view (the veil is drawn by the slot component).
      expect(leafVeil('s1')).toBeNull()
    })

    it('terminal mode is the same — node kept', () => {
      const { rerender } = mountAlive(false, 5)
      const before = screen.getByTestId('terminal-slot')

      agentStoreState.agents = []
      rerender(<ViewLayoutRenderer node={slotNode('s1', GONE)} focusedSlotId={null} />)

      const after = screen.getByTestId('terminal-slot')
      expect(Object.is(before, after)).toBe(true)
    })

    // ★Do not put the epoch back into the slot★: that value comes only from the live roster, and exit means disappearing from the roster,
    //   so the moment it is passed down as a prop **exit = prop change = remount = conversation loss**.
    //   Incarnation rotation is judged by the subscription against the authoritative roster and emitted as a clear signal (protocolClient.observeRoster).
    it('what is passed to the slot is only viewId and agentId (the epoch is not passed down)', () => {
      mountAlive(true, 3)
      const slot = screen.getByTestId('rich-slot')
      expect(slot.getAttribute('data-view-id')).toBe('s1')
      expect(slot.getAttribute('data-agent-id')).toBe(GONE)
      expect(slot.getAttribute('data-epoch')).toBeNull()
    })

    // ★F2 regression★: the memory belongs to that agent. Assigning another agent must not skip the caps gate —
    //   if B is in terminal mode but RichSlot appears from A's memory, tag0 bytes are dropped.
    it('assigning another agent to the slot does not use the memory (caps gate kept)', () => {
      const { rerender } = mountAlive(true, 3)
      expect(screen.getByTestId('rich-slot')).toBeTruthy()

      // B assigned — not in the roster yet (caps not arrived).
      const OTHER = 'other-agent'
      agentStoreState.profiles = [profile(GONE), profile(OTHER)]
      rerender(<ViewLayoutRenderer node={slotNode('s1', OTHER)} focusedSlotId={null} />)

      expect(screen.queryByTestId('rich-slot')).toBeNull()
      expect(screen.queryByTestId('terminal-slot')).toBeNull()
      expect(leafVeil('s1')).not.toBeNull()
    })

    // ★G1 regression★: if it exits without having received the profile list (a one-shot pull failed or was delayed) presence is 'unknown'.
    //   Tearing the view down on that permanently loses the conversation being preserved — "no profile" is trusted only after the list is received.
    it('even if it exits without having received the profile list, the view is kept if there is memory', () => {
      // The combination where the roster was received (the agent really came up) and only the profile list was not.
      seedAgents(agentInfo(GONE, true, 4))
      agentStoreState.agentsLoaded = true
      agentStoreState.profilesLoaded = false
      agentStoreState.profiles = []
      const { rerender } = render(<ViewLayoutRenderer node={slotNode('s1', GONE)} focusedSlotId={null} />)
      const before = screen.getByTestId('rich-slot')

      agentStoreState.agents = []
      rerender(<ViewLayoutRenderer node={slotNode('s1', GONE)} focusedSlotId={null} />)

      const after = screen.getByTestId('rich-slot')
      expect(Object.is(before, after)).toBe(true)
      expect(screen.queryByText('에이전트 연결 중…')).toBeNull()
    })

    // ★G2 regression★: if the memory is not erased it revives when the assignment is removed and put back. In between, the view has already unmounted
    //   and there is no conversation, so a revived memory produces "empty screen + dim + subscription with a reaped agent".
    it('moving the assignment to another agent and back does not revive the memory', () => {
      const OTHER = 'other-agent'
      const { rerender } = mountAlive(true, 3)
      expect(screen.getByTestId('rich-slot')).toBeTruthy()

      // Exit → view kept (normal).
      agentStoreState.agents = []
      rerender(<ViewLayoutRenderer node={slotNode('s1', GONE)} focusedSlotId={null} />)
      expect(screen.getByTestId('rich-slot')).toBeTruthy()

      // Assign B to the same slot → A's view unmounts (the conversation state dies here too) and the memory must also be dropped.
      agentStoreState.profiles = [profile(GONE), profile(OTHER)]
      rerender(<ViewLayoutRenderer node={slotNode('s1', OTHER)} focusedSlotId={null} />)
      expect(screen.queryByTestId('rich-slot')).toBeNull()

      // Assign A again — there is no conversation to revive, so no empty view is brought up.
      rerender(<ViewLayoutRenderer node={slotNode('s1', GONE)} focusedSlotId={null} />)
      expect(screen.queryByTestId('rich-slot')).toBeNull()
      expect(leafVeil('s1')).not.toBeNull()
    })

    // ★Leaf key = slot id (ADR-0227)★: when a different slot id comes at the same place the leaf is built anew. If the memory passed to s2,
    //   s2 would 「preserve」 a view it never brought up and subscribe with a reaped agent.
    it('when the slot at the same place changes to a different slot id the memory is not carried over (leaf remount)', () => {
      const { rerender } = mountAlive(true, 3)
      expect(screen.getByTestId('rich-slot')).toBeTruthy()

      // Exit — disappears from the roster and the profile stays (reserved). With the same s1 this is the combination that keeps the view.
      agentStoreState.agents = []
      rerender(<ViewLayoutRenderer node={slotNode('s2', GONE)} focusedSlotId={null} />)

      expect(screen.queryByTestId('rich-slot')).toBeNull()
      expect(leafVeil('s2')).not.toBeNull()
    })

    it('no profile → 「no target」 (the view that was being preserved also comes down here — intended)', () => {
      const { rerender } = mountAlive(true, 3)
      expect(screen.getByTestId('rich-slot')).toBeTruthy()

      // After exit, deleting the profile too = the user's explicit cleanup action.
      agentStoreState.agents = []
      agentStoreState.profiles = []
      rerender(<ViewLayoutRenderer node={slotNode('s1', GONE)} focusedSlotId={null} />)

      expect(screen.getByText('대상 없음')).toBeTruthy()
      expect(screen.queryByTestId('rich-slot')).toBeNull()
      expect(leafVeil('s1')).toBeNull()
    })
  })

  it('agent in the store and structured caps → RichSlot (no TerminalSlot)', () => {
    const agentId = 'struct-agent'
    seedAgents(agentInfo(agentId, true))
    render(<ViewLayoutRenderer node={slotNode('s1', agentId)} focusedSlotId={null} />)
    expect(screen.getByTestId('rich-slot')).toBeTruthy()
    expect(screen.queryByTestId('terminal-slot')).toBeNull()
    expect(screen.queryByText('에이전트 연결 중…')).toBeNull()
  })

  // ── Default RenderMode derivation (defaultRenderMode): with no override the renderer is decided by caps ──────────
  it('no override + structured=true caps → RichSlot by default derivation (no TerminalSlot)', () => {
    const agentId = 'derive-rich'
    seedAgents(agentInfo(agentId, true))
    render(<ViewLayoutRenderer node={slotNode('s1', agentId)} focusedSlotId={null} />)
    expect(screen.getByTestId('rich-slot')).toBeTruthy()
    expect(screen.queryByTestId('terminal-slot')).toBeNull()
  })

  it('no override + structured=false caps → TerminalSlot by default derivation (no RichSlot)', () => {
    const agentId = 'derive-terminal'
    seedAgents(agentInfo(agentId, false))
    render(<ViewLayoutRenderer node={slotNode('s1', agentId)} focusedSlotId={null} />)
    expect(screen.getByTestId('terminal-slot')).toBeTruthy()
    expect(screen.queryByTestId('rich-slot')).toBeNull()
  })

  // ── An override beats the default (setRenderMode) ────────────────────────────────────────────────
  it('setRenderMode(id,"terminal") overrides the structured default (rich) and mounts TerminalSlot', () => {
    const agentId = 'override-terminal'
    seedAgents(agentInfo(agentId, true))
    useViewStore.getState().setRenderMode('s1', 'terminal')
    render(<ViewLayoutRenderer node={slotNode('s1', agentId)} focusedSlotId={null} />)
    expect(screen.getByTestId('terminal-slot')).toBeTruthy()
    expect(screen.queryByTestId('rich-slot')).toBeNull()
  })

  it('setRenderMode(id,"rich") overrides the non-structured default (terminal) and mounts RichSlot', () => {
    const agentId = 'override-rich'
    seedAgents(agentInfo(agentId, false))
    useViewStore.getState().setRenderMode('s1', 'rich')
    render(<ViewLayoutRenderer node={slotNode('s1', agentId)} focusedSlotId={null} />)
    expect(screen.getByTestId('rich-slot')).toBeTruthy()
    expect(screen.queryByTestId('terminal-slot')).toBeNull()
  })

  // ── DOM override (§5 observation): takes priority over the caps default renderer, caps-ready gating kept ──────────────
  it('slot with renderModeOverride=dom (caps arrived) → DomSlot mounts and Terminal/Rich do not', () => {
    const agentId = 'dom-agent'
    seedAgents(agentInfo(agentId, false)) // even non-structured (terminal default), DOM mode must take priority
    useViewStore.getState().setRenderMode('s1', 'dom')
    render(<ViewLayoutRenderer node={slotNode('s1', agentId)} focusedSlotId={null} />)
    const dom = screen.getByTestId('dom-slot')
    expect(dom).toBeTruthy()
    expect(dom.getAttribute('data-agent-id')).toBe(agentId)
    expect(screen.queryByTestId('terminal-slot')).toBeNull()
    expect(screen.queryByTestId('rich-slot')).toBeNull()
  })

  it('renderModeOverride=dom takes priority over the structured caps default (rich) (DomSlot, not RichSlot)', () => {
    const agentId = 'dom-struct-agent'
    seedAgents(agentInfo(agentId, true))
    useViewStore.getState().setRenderMode('s1', 'dom')
    render(<ViewLayoutRenderer node={slotNode('s1', agentId)} focusedSlotId={null} />)
    expect(screen.getByTestId('dom-slot')).toBeTruthy()
    expect(screen.queryByTestId('rich-slot')).toBeNull()
  })

  it('even with renderModeOverride=dom, if caps have not arrived DomSlot does not show and "agent connecting…" shows (replay gating kept)', () => {
    // If caps have not arrived no concrete renderer is mounted even with an override (prevents losing pre-swap bytes — replay ownership).
    useViewStore.getState().setRenderMode('s1', 'dom')
    render(<ViewLayoutRenderer node={slotNode('s1', 'not-in-store')} focusedSlotId={null} />)
    expect(screen.queryByTestId('dom-slot')).toBeNull()
    expect(screen.getByText('에이전트 연결 중…')).toBeTruthy()
  })

  it('releasing the override with clearRenderMode(id) returns to the caps-derived default', () => {
    const agentId = 'clear-agent'
    seedAgents(agentInfo(agentId, false)) // default = terminal
    useViewStore.getState().setRenderMode('s1', 'dom')
    useViewStore.getState().clearRenderMode('s1')
    render(<ViewLayoutRenderer node={slotNode('s1', agentId)} focusedSlotId={null} />)
    expect(screen.getByTestId('terminal-slot')).toBeTruthy()
    expect(screen.queryByTestId('dom-slot')).toBeNull()
  })
})

// ── Split tree (ADR-0227 flat renderer) ─────────────────────────────────────────────────────────
// A split tree is not unrolled recursively — the area frames are laid side by side under one root using the shell snapshot's rectangles, with dividers behind them.
//   Rectangles are made by a test-only fixture (`testing/rects.ts`). Here we only see that the renderer carries the rectangles it received as they are,
//   and do not check whether the values equal the shell's (geometry = the Rust `geometry.rs` tests' job). Remount and order are measured by `.flat.test.tsx`.
describe('ViewLayoutRenderer — split branch (shell rectangles)', () => {
  function renderTree(node: LayoutNode) {
    const { slotRects, splitRects } = rectsFor(node)
    return render(
      <ViewLayoutRenderer node={node} focusedSlotId={null} slotRects={slotRects} splitRects={splitRects} />,
    )
  }

  it('left_right split → two area frames are drawn with left/right rectangles (--x*·--y*) and one divider is attached', () => {
    const node = splitNode('sp', slotNode('s1', null), slotNode('s2', null), 0.5, 'left_right')
    renderTree(node)
    expect(frameRect('s1')).toEqual({ x0: 0, y0: 0, x1: 0.5, y1: 1 })
    expect(frameRect('s2')).toEqual({ x0: 0.5, y0: 0, x1: 1, y1: 1 })
    const divider = document.querySelector<HTMLElement>('[data-split-id="sp"][data-dir="left_right"]')
    expect(divider).not.toBeNull()
    // The divider is a direct sibling of the root, not inside an area frame (an overlay that takes no layout space).
    expect(divider!.parentElement).toBe(frameOf('s1').parentElement)
    expect(divider!.getAttribute('aria-orientation')).toBe('vertical')
  })

  // The dir → screen direction mapping (ADR-0140) is resolved by the shell with rectangles — the renderer carries the received y boundaries as they are.
  it('top_bottom split → two area frames are drawn with top/bottom rectangles and the divider data-dir is top_bottom', () => {
    const node = splitNode('sp', slotNode('s1', null), slotNode('s2', null), 0.5, 'top_bottom')
    renderTree(node)
    expect(frameRect('s1')).toEqual({ x0: 0, y0: 0, x1: 1, y1: 0.5 })
    expect(frameRect('s2')).toEqual({ x0: 0, y0: 0.5, x1: 1, y1: 1 })
    const divider = document.querySelector<HTMLElement>('[data-split-id="sp"]')
    expect(divider?.getAttribute('data-dir')).toBe('top_bottom')
    expect(divider?.getAttribute('aria-orientation')).toBe('horizontal')
  })

  it('carries the received rectangle values as they are (ratio 0.2 fixture → first area x1 = 0.2)', () => {
    const node = splitNode('sp', slotNode('s1', null), slotNode('s2', null), 0.2)
    const { slotRects } = rectsFor(node)
    renderTree(node)
    for (const r of slotRects) {
      expect(frameRect(r.slot_id)).toEqual({ x0: r.x0, y0: r.y0, x1: r.x1, y1: r.y1 })
    }
    expect(frameRect('s1').x1).toBe(0.2)
  })

  it('nested split → three areas and two dividers are laid flat under one root', () => {
    const node = splitNode('outer', slotNode('s1', null), splitNode('inner', slotNode('s2', null), slotNode('s3', null), 0.5, 'top_bottom'))
    renderTree(node)
    const root = frameOf('s1').parentElement!
    for (const id of ['s2', 's3']) expect(frameOf(id).parentElement).toBe(root)
    expect(root.querySelectorAll(':scope > [data-split-id]')).toHaveLength(2)
    expect(frameRect('s2')).toEqual({ x0: 0.5, y0: 0, x1: 1, y1: 0.5 })
    expect(frameRect('s3')).toEqual({ x0: 0.5, y0: 0.5, x1: 1, y1: 1 })
  })

  it('a split child with agent_id (caps arrived) mounts TerminalSlot only in that area', () => {
    const agentId = 'zzzz-agent'
    seedAgents(agentInfo(agentId, false))
    renderTree(splitNode('sp', slotNode('s1', agentId), slotNode('s2', null)))
    const terminals = screen.getAllByTestId('terminal-slot')
    expect(terminals).toHaveLength(1)
    expect(terminals[0].getAttribute('data-agent-id')).toBe(agentId)
    expect(terminals[0].getAttribute('data-view-id')).toBe('s1')
    expect(borderOf('s1').contains(terminals[0])).toBe(true)
    expect(emptyIcons()).toHaveLength(1)
    expect(borderOf('s2').contains(emptyIcons()[0])).toBe(true)
  })

  // Not in the production path (the snapshot always carries rectangles) — the screen does not compute tree geometry itself, so it does not draw but reports.
  it('a root that is a split without rectangles leaves an error and draws nothing', () => {
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {})
    try {
      const { container } = render(
        <ViewLayoutRenderer node={splitNode('sp', slotNode('s1', null), slotNode('s2', null))} focusedSlotId={null} />,
      )
      expect(container.innerHTML).toBe('')
      expect(consoleError).toHaveBeenCalled()
    } finally {
      consoleError.mockRestore()
    }
  })

  it('a root that is a slot without rectangles is drawn as the full box (0,0)-(1,1)', () => {
    render(<ViewLayoutRenderer node={slotNode('s1', null)} focusedSlotId={null} />)
    expect(frameRect('s1')).toEqual({ x0: 0, y0: 0, x1: 1, y1: 1 })
    expect(document.querySelectorAll('[data-split-id]')).toHaveLength(0)
  })
})

// ── ★click-to-focus gate (control-slot focus excluded — ADR-0066 refinement)★ ─────────────────────────────
// ★What this suite blocks★: a click on the tree (agent_list) or palette (preset_palette) slot pane must not call focusSlot
// (they are not work slots, so not focus targets). That was the root of the pre-existing UX bug where the following right-click "open" took that control slot as target and
// overwrote the tree with an agent terminal. Content slots (empty/agent) call focusSlot as before.
//
// Strategy: inject a focusSlot spy into the real viewStore (human click = LLM = single surface) and fill windows["main"].active so the
//   targetViewId fallback holds (same setup as the context-menu suite).
describe('ViewLayoutRenderer — click-to-focus gate (control-slot focus excluded)', () => {
  const FOCUS_VIEW = 'focus-view-1'
  const focusSlotSpy = vi.fn(async () => undefined)
  const origHash = window.location.hash

  beforeEach(() => {
    focusSlotSpy.mockClear()
    window.location.hash = '#/'
    useViewStore.setState({
      windows: {
        main: { windowId: 'main', number: 1, tabs: [{ id: FOCUS_VIEW, name: 'View', named: true }], active: FOCUS_VIEW, version: 1 },
      },
      focusSlot: focusSlotSpy,
    })
  })
  afterEach(() => {
    window.location.hash = origHash
  })

  function clickSlot(content: SlotContent): void {
    render(<ViewLayoutRenderer node={contentSlotNode('s1', content)} focusedSlotId={null} />)
    fireEvent.click(document.querySelector('[data-slot-id="s1"]') as HTMLElement)
  }

  it('empty slot click → calls focusSlot(viewId, slotId) (content slot = focus target)', () => {
    clickSlot({ type: 'empty' })
    expect(focusSlotSpy).toHaveBeenCalledWith(FOCUS_VIEW, 's1')
  })

  // ADR-0144: a left-click on an empty slot does not open the menu — focus movement is unchanged.
  it('left-click on an empty slot calls only focusSlot and does not open the menu', () => {
    clickSlot({ type: 'empty' })
    expect(focusSlotSpy).toHaveBeenCalledWith(FOCUS_VIEW, 's1')
    expect(screen.queryByText('가로 분할')).toBeNull()
  })

  // ★The icon does not swallow clicks★: a click on the icon must also reach the container for focusSlot to be called. Without this,
  //   re-attaching interaction to the icon leaves the suite green and click-to-focus quietly dies (with left-click no longer
  //   opening a menu there is no visible signal other than the focus ring, ADR-0144).
  it('a click on the `+` icon also reaches the container and calls focusSlot', () => {
    render(<ViewLayoutRenderer node={contentSlotNode('s1', { type: 'empty' })} focusedSlotId={null} />)
    fireEvent.click(emptyIcons()[0], { clientX: 5, clientY: 5 })
    expect(focusSlotSpy).toHaveBeenCalledWith(FOCUS_VIEW, 's1')
  })

  it('agent slot click → calls focusSlot (content slot)', () => {
    seedAgents(agentInfo('a-focus', false)) // caps arrived (harmless — the gate looks only at content.type)
    render(<ViewLayoutRenderer node={contentSlotNode('s1', { type: 'agent', agent_id: 'a-focus' })} focusedSlotId={null} />)
    fireEvent.click(document.querySelector('[data-slot-id="s1"]') as HTMLElement)
    expect(focusSlotSpy).toHaveBeenCalledWith(FOCUS_VIEW, 's1')
  })

  it('agent_list (tree) slot click → focusSlot not called (control slot = focus excluded)', () => {
    clickSlot({ type: 'agent_list' })
    expect(focusSlotSpy).not.toHaveBeenCalled()
  })

  it('preset_palette (palette) slot click → focusSlot not called (control slot = focus excluded)', () => {
    clickSlot({ type: 'preset_palette' })
    expect(focusSlotSpy).not.toHaveBeenCalled()
  })

  // The focus gate is an allowlist (isContentSlot), so a new variant is excluded without being singled out.
  it('usage slot click → focusSlot not called', () => {
    clickSlot({ type: 'usage', show_claude: true, show_codex: true })
    expect(focusSlotSpy).not.toHaveBeenCalled()
  })
})

// ── ★Right-click unified context menu (§5, ADR-0064)★ ─────────────────────────────────────────────────
// ★What this suite actually blocks★: right-clicking a canvas slot mounts the unified SlotContextMenu (the output of buildSlotMenu(content.type))
// and each item click flows (viewId, slotId, agentId) through its command.run(ctx) to viewStore/agentClient.
// Menu items = command id references (ADR-0064) — content-specific (kill agent · open tree/palette · create) +
// common '*' (split horizontal/vertical · detach popup · clear · close) coexist in one menu.
//
// Strategy: spies for split/closeSlot/assignAgent/setSlotContent/moveSlotToWindow are injected into the real viewStore
// (a command calls them via useViewStore.getState().split(...) → physically identical to LLM/__engramCmd).
// 'kill agent' (agent.kill) continues to agentClient.killAgent.
describe('ViewLayoutRenderer — right-click context menu (§5 single control surface)', () => {
  const ACTIVE_VIEW = 'active-view-9'
  const splitSpy = vi.fn(async () => 'new-slot')
  const closeSlotSpy = vi.fn(async () => undefined)
  const assignAgentSpy = vi.fn(async () => undefined)
  const setSlotContentSpy = vi.fn(async () => undefined)
  const moveSlotToWindowSpy = vi.fn(async () => ({
    window: 'slot-popup-1',
    tab: 'v-new',
    window_id: '7d2f0b8e-1c3a-4f5e-9b6d-2a4c6e8f0a1b',
    number: 2,
  }))
  const origHash = window.location.hash

  beforeEach(() => {
    splitSpy.mockClear()
    closeSlotSpy.mockClear()
    assignAgentSpy.mockClear()
    setSlotContentSpy.mockClear()
    moveSlotToWindowSpy.mockClear()
    clientMock.spawnAgent.mockClear()
    clientMock.killAgent.mockClear()
    dialogMock.open.mockClear()
    dialogMock.open.mockResolvedValue(null)
    // ★Tab ownership model (ADR-0057)★: without viewIdOverride, ViewLayoutRenderer falls back to useCurrentViewId() (this webview window's
    //   active tab) to fill ctx.viewId. Set the main window (#/) context and inject windows["main"].active=ACTIVE_VIEW
    //   → the fallback path picks ACTIVE_VIEW. Layout actions are also injected into the store (single surface).
    window.location.hash = '#/'
    useViewStore.setState({
      windows: {
        main: { windowId: 'main', number: 1, tabs: [{ id: ACTIVE_VIEW, name: 'View', named: true }], active: ACTIVE_VIEW, version: 1 },
      },
      split: splitSpy,
      closeSlot: closeSlotSpy,
      assignAgent: assignAgentSpy,
      setSlotContent: setSlotContentSpy,
      moveSlotToWindow: moveSlotToWindowSpy,
    })
  })

  afterEach(() => {
    window.location.hash = origHash
  })

  /** If viewIdOverride is passed, render with that view (Fix 3). */
  function openMenu(
    slotId: string,
    agentId: string | null,
    viewIdOverride?: string | null,
  ): HTMLElement {
    render(
      <ViewLayoutRenderer
        node={slotNode(slotId, agentId)}
        focusedSlotId={null}
        viewIdOverride={viewIdOverride}
      />,
    )
    const wrapper = document.querySelector(`[data-slot-id="${slotId}"]`) as HTMLElement
    fireEvent.contextMenu(wrapper)
    return wrapper
  }

  const POPUP_VIEW = 'popup-view-77'

  /** ADR-0065: empty-slot fill-ops folded into a "new content" container — the flyout must be opened by hover for the children to show. */
  function openNewContentFlyout(): void {
    fireEvent.mouseEnter(screen.getByText('새 콘텐츠'))
  }

  it('right-click on an empty slot → top level = agent monitoring + new content (container) + common slot ops, fills inside the flyout (ADR-0067/0065)', () => {
    openMenu('s1', null)
    expect(screen.getByText('에이전트 모니터링')).toBeTruthy()
    expect(screen.getByText('새 콘텐츠')).toBeTruthy()
    expect(screen.getByText('가로 분할')).toBeTruthy()
    expect(screen.getByText('세로 분할')).toBeTruthy()
    expect(screen.getByText('닫기')).toBeTruthy()
    expect(screen.queryByText('비우기')).toBeNull()
    expect(screen.getByText('팝업으로 분리')).toBeTruthy() // ADR-0228
    expect(screen.queryByText('에이전트 트리 열기')).toBeNull()
    openNewContentFlyout()
    expect(screen.getByText('에이전트 트리 열기')).toBeTruthy()
    expect(screen.getByText('프리셋 팔레트 열기')).toBeTruthy()
    // ADR-0067
    expect(screen.queryByText('에이전트 생성')).toBeNull()
  })

  it('there is no menu before a right-click (state-based mount after preventDefault)', () => {
    render(<ViewLayoutRenderer node={slotNode('s1', null)} focusedSlotId={null} />)
    expect(screen.queryByText('가로 분할')).toBeNull()
  })

  // ★Leaf key = slot id (ADR-0227)★: the leaf holds the menu state — a different slot id at the same place is a new leaf so the menu does not follow.
  it('when the slot at the same place changes to a different slot id the open menu closes (leaf remount)', () => {
    const { rerender } = render(<ViewLayoutRenderer node={slotNode('s1', null)} focusedSlotId={null} />)
    const before = document.querySelector('[data-slot-id="s1"]') as HTMLElement
    fireEvent.contextMenu(before)
    expect(screen.getByText('가로 분할')).toBeTruthy()

    rerender(<ViewLayoutRenderer node={slotNode('s2', null)} focusedSlotId={null} />)

    expect(screen.queryByText('가로 분할')).toBeNull()
    expect(document.querySelector('[data-slot-id="s2"]')).not.toBeNull()
    expect(before.isConnected).toBe(false)
  })

  // ── ★The menu is right-click only (ADR-0144)★ ────────────────────────────────────────────────────────
  // ADR-0141/0143 made a left-click on an empty slot open the menu too, but after reports of "a menu appears on every click"
  // ADR-0144 reverted it — a left-click on an empty slot now only moves focus. The following three cases verify the invariants of the
  // right-click-opened menu (no re-anchoring · outside-click handling · command path).
  /** The menu div (position:fixed container) — for coordinate assertions. */
  function openedMenu(): HTMLElement {
    return screen.getByText('가로 분할').closest('[style*="position: fixed"]') as HTMLElement
  }

  /** A left-click on blank margin that is not the icon = the slot wrapper is the target (after ADR-0144 it does not open the menu). */
  function leftClickSlot(slotId: string, x: number, y: number): void {
    fireEvent.click(document.querySelector(`[data-slot-id="${slotId}"]`) as HTMLElement, {
      clientX: x,
      clientY: y,
    })
  }

  /** The menu opener is right-click only (ADR-0144) — cases that need coordinates (re-anchor · outside click) use this instead of openMenu(). */
  function rightClickSlot(slotId: string, x: number, y: number): void {
    fireEvent.contextMenu(document.querySelector(`[data-slot-id="${slotId}"]`) as HTMLElement, {
      clientX: x,
      clientY: y,
    })
  }

  // ★Do not re-attach interaction (ADR-0143 §Impact)★: an empty slot must have no role, tabindex or button —
  //   re-attaching would overlap the container handler and open twice, and revive a path to a menu that cannot be left by keyboard.
  //   ★Limit★: React delegates handlers to the root, so whether an onClick was re-attached to the icon cannot be seen in the DOM
  //   (with both using the same coordinates they cannot be told apart by behavior either). That clause is left to review.
  it('an empty slot has no interactive element (no role · tabindex · button)', () => {
    render(<ViewLayoutRenderer node={slotNode('slot-inert', null)} focusedSlotId={null} />)
    const wrapper = document.querySelector('[data-slot-id="slot-inert"]') as HTMLElement
    expect(wrapper.querySelector('button, [role], [tabindex]')).toBeNull()
    const icon = emptyIcons()[0]
    expect(icon.hasAttribute('tabindex')).toBe(false)
    expect(icon.hasAttribute('role')).toBe(false)
    expect(icon.hasAttribute('aria-label')).toBe(false)
    // ADR-0143 decision 2: a pure picture — pointer-events is cut so clicks always reach the container (still valid after ADR-0144).
    expect(getComputedStyle(icon).pointerEvents).toBe('none')
  })

  // ★Icon size = over 32px (ADR-0143 decision 3)★: Tailwind `size-N` = N×0.25rem = N×4px (default scale), so
  //   px is recovered from the class. jsdom does not apply Tailwind so it cannot be seen by computed value (measured in the GUI).
  it('the `+` icon is larger than 32px', () => {
    render(<ViewLayoutRenderer node={slotNode('slot-size', null)} focusedSlotId={null} />)
    const sizeClass = [...emptyIcons()[0].classList].map(c => /^size-(\d+)$/.exec(c)).find(m => m != null)
    expect(sizeClass, 'no size-N utility').not.toBeUndefined()
    expect(Number(sizeClass![1]) * 4).toBeGreaterThan(32)
  })

  // ★An open menu is not re-anchored (a click *inside* the menu)★: SlotContextMenu is mounted inside the slot wrapper, not a portal,
  //   so a click on a submenu container row bubbles to the wrapper. Re-anchoring would make the menu jump under the cursor, the anchor would overlap
  //   the menu and the following click would run the item under the cursor. Even after a left-click stopped opening the menu (ADR-0144)
  //   this invariant must still hold for clicks inside a menu opened by right-click.
  it('a click on the "new content" row of an open menu does not re-anchor the menu', () => {
    render(<ViewLayoutRenderer node={slotNode('slot-reanchor', null)} focusedSlotId={null} />)
    rightClickSlot('slot-reanchor', 42, 77)
    fireEvent.click(screen.getByText('새 콘텐츠'), { clientX: 300, clientY: 400 })
    expect(openedMenu().style.left).toBe(`${42 + ANCHOR_GAP}px`)
    expect(openedMenu().style.top).toBe(`${77 + ANCHOR_GAP}px`)
  })

  // ★A left-click *outside* the menu (slot margin) only closes the menu and does not reopen it at new coordinates (ADR-0144)★: SlotContextMenu's
  //   outside-close still runs first on `mousedown` and clears the state (same as in the ADR-0143 days). What changed is what comes
  //   after — a left-click no longer opens the menu (the "menu is right-click only" section above), so after an outside click the menu is not
  //   moved open at new coordinates and stays closed.
  it('a left-click outside the open menu (slot margin) closes the menu and does not reopen it at new coordinates', () => {
    render(<ViewLayoutRenderer node={slotNode('slot-move', null)} focusedSlotId={null} />)
    rightClickSlot('slot-move', 42, 77)
    expect(openedMenu().style.left).toBe(`${42 + ANCHOR_GAP}px`)
    const wrapper = document.querySelector('[data-slot-id="slot-move"]') as HTMLElement
    fireEvent.mouseDown(wrapper, { clientX: 300, clientY: 400 })
    leftClickSlot('slot-move', 300, 400)
    expect(screen.queryByText('가로 분할')).toBeNull()
  })

  it('"split horizontally" of a menu opened by right-click also calls split(…, "top_bottom") through the same command path', () => {
    render(<ViewLayoutRenderer node={slotNode('slot-lc2', null)} focusedSlotId={null} />)
    rightClickSlot('slot-lc2', 8, 9)
    fireEvent.click(screen.getByText('가로 분할'))
    expect(splitSpy).toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-lc2', 'top_bottom')
  })

  // ★A left-click opens no menu for any slot type (ADR-0144)★: a menu over terminal input, tree node selection or palette item
  //   clicks would cover the click meaning those slots have. An empty slot now follows the same rule (that
  //   case is verified with focusSlot by the click-to-focus gate suite above) and the three cases below are the control
  //   slots' share ('split horizontally' is a common item independent of content kind, so it serves as the discriminator of "did a menu appear").
  it('a left-click on an agent (terminal) slot does not open a menu', () => {
    seedAgents(agentInfo('a-lc', false))
    render(<ViewLayoutRenderer node={slotNode('slot-agent-lc', 'a-lc')} focusedSlotId={null} />)
    leftClickSlot('slot-agent-lc', 20, 30)
    expect(screen.queryByText('가로 분할')).toBeNull()
  })

  it('a left-click on an agent_list (tree) slot does not open a menu', () => {
    render(<ViewLayoutRenderer node={contentSlotNode('slot-tree-lc', { type: 'agent_list' })} focusedSlotId={null} />)
    leftClickSlot('slot-tree-lc', 20, 30)
    expect(screen.queryByText('가로 분할')).toBeNull()
  })

  it('a left-click on a preset_palette (palette) slot does not open a menu', () => {
    render(
      <ViewLayoutRenderer node={contentSlotNode('slot-pal-lc', { type: 'preset_palette' })} focusedSlotId={null} />,
    )
    leftClickSlot('slot-pal-lc', 20, 30)
    expect(screen.queryByText('가로 분할')).toBeNull()
  })

  // ★Label ↔ direction wiring (ADR-0140 = vim convention)★: a horizontal line appears and splits up/down → top_bottom. If reversed
  //   the convention the user chose breaks (regresses to the tmux axis convention).
  it('"split horizontally" → calls split(viewId, slotId, "top_bottom") (§5 command path)', () => {
    openMenu('slot-A', null)
    fireEvent.click(screen.getByText('가로 분할'))
    expect(splitSpy).toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-A', 'top_bottom')
  })

  it('"split vertically" → calls split(viewId, slotId, "left_right")', () => {
    openMenu('slot-B', null)
    fireEvent.click(screen.getByText('세로 분할'))
    expect(splitSpy).toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-B', 'left_right')
  })

  it('"close" → calls closeSlot(viewId, slotId)', () => {
    openMenu('slot-C', null)
    fireEvent.click(screen.getByText('닫기'))
    expect(closeSlotSpy).toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-C')
  })

  // ── ★empty fill-ops (ADR-0063/0064/0065)★: inside the "new content" flyout → setSlotContent(view, slot, {type}) ──
  it('"open agent tree" (flyout) → setSlotContent(viewId, slotId, {type:agent_list})', () => {
    openMenu('slot-T', null)
    openNewContentFlyout()
    fireEvent.click(screen.getByText('에이전트 트리 열기'))
    expect(setSlotContentSpy).toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-T', { type: 'agent_list' })
  })

  it('"open preset palette" (flyout) → setSlotContent(viewId, slotId, {type:preset_palette})', () => {
    openMenu('slot-U', null)
    openNewContentFlyout()
    fireEvent.click(screen.getByText('프리셋 팔레트 열기'))
    expect(setSlotContentSpy).toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-U', { type: 'preset_palette' })
  })

  it('"open usage limits" (flyout) → setSlotContent(viewId, slotId, {type:usage, both companies on})', () => {
    openMenu('slot-W', null)
    openNewContentFlyout()
    fireEvent.click(screen.getByText('사용량 한도 열기'))
    expect(setSlotContentSpy).toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-W', {
      type: 'usage',
      show_claude: true,
      show_codex: true,
    })
  })

  // ── Usage slot — the right-click menu has no usage items (user decision 2026-09-29 — ⟳ is a small indicator, display toggles are in the popup) ──
  it('right-click on a usage slot → only common slot ops without usage items (⟳ · company display) · UsageSlot receives this slot coordinates', () => {
    render(
      <ViewLayoutRenderer
        node={contentSlotNode('slot-U1', { type: 'usage', show_claude: true, show_codex: false })}
        focusedSlotId={null}
      />,
    )
    fireEvent.contextMenu(document.querySelector('[data-slot-id="slot-U1"]') as HTMLElement)
    expect(screen.queryByText('사용량 새로고침')).toBeNull()
    expect(screen.queryByText('Claude 표시')).toBeNull()
    expect(screen.queryByText('Codex 표시')).toBeNull()
    expect(document.querySelector('[data-slot-menu-item^="usageSlot."]')).toBeNull()
    expect(document.querySelector('[data-slot-menu-item="slot.close"]')).not.toBeNull()
    const usage = screen.getByTestId('usage-slot')
    expect(usage.getAttribute('data-view-id')).toBe(ACTIVE_VIEW)
    expect(usage.getAttribute('data-slot-id')).toBe('slot-U1')
  })

  it('an empty slot has no "clear" (ADR-0065 hideOn:["empty"] trim — re-clearing an already empty slot is a no-op)', () => {
    openMenu('slot-V', null)
    expect(screen.queryByText('비우기')).toBeNull()
  })

  it('with viewIdOverride, "open agent tree" (flyout) calls setSlotContent with the override view', () => {
    openMenu('slot-to', null, POPUP_VIEW)
    openNewContentFlyout()
    fireEvent.click(screen.getByText('에이전트 트리 열기'))
    expect(setSlotContentSpy).toHaveBeenCalledWith(POPUP_VIEW, 'slot-to', { type: 'agent_list' })
  })

  // ADR-0067: the slot content-fill menu has no spawn path; direct command routing is covered by slotCommands.test.ts.

  // ── ★agent slot: content-specific "kill agent" + common ops coexist (ADR-0064)★ ──────────────────────
  it('right-click on an agent-assigned slot → clicking "kill agent" (content) calls killAgent(that agentId)', () => {
    openMenu('slot-E', 'assigned-agent')
    fireEvent.click(screen.getByText('에이전트 종료'))
    expect(clientMock.killAgent).toHaveBeenCalledWith('assigned-agent')
  })

  it('an agent slot also shows the common slot ops (close · split · popup) (prevents the common-loss bug)', () => {
    openMenu('slot-E2', 'some-agent')
    expect(screen.getByText('에이전트 종료')).toBeTruthy()
    expect(screen.getByText('닫기')).toBeTruthy()
    expect(screen.getByText('팝업으로 분리')).toBeTruthy()
  })

  it('an empty-slot menu has no "kill agent" (agent-only content item)', () => {
    openMenu('slot-empty-x', null)
    expect(screen.queryByText('에이전트 종료')).toBeNull()
  })

  // ── TRD S21-storage §6-8: 「no target」 · absence-veil slot = assignment kept + the agent slot menu as is (user decision 2026-10-06) ──
  it('right-click on a 「no target」 slot → the agent slot menu as is — "clear" empties that slot', () => {
    agentStoreState.agentsLoaded = true
    agentStoreState.profilesLoaded = true
    openMenu('slot-NT', 'deleted-agent')
    expect(screen.getByText('대상 없음')).toBeTruthy()
    expect(screen.getByText('에이전트 모니터링')).toBeTruthy()
    expect(screen.getByText('에이전트 종료')).toBeTruthy()
    expect(screen.queryByText('새 콘텐츠')).toBeNull()
    fireEvent.click(screen.getByText('비우기'))
    expect(setSlotContentSpy).toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-NT', { type: 'empty' })
  })

  it('the menu of an absence-veil slot (profile present · not running) is also the agent slot menu as is ("clear" present)', () => {
    agentStoreState.agentsLoaded = true
    agentStoreState.profilesLoaded = true
    agentStoreState.profiles = [{ id: 'stopped-x', name: 'stopped-x', cwd: '/tmp', display_name: null, parent_id: null, created_at: 0, epoch: 1 }]
    openMenu('slot-ST', 'stopped-x')
    expect(leafVeil('slot-ST')).not.toBeNull()
    expect(screen.getByText('에이전트 종료')).toBeTruthy()
    expect(screen.getByText('비우기')).toBeTruthy()
    expect(screen.queryByText('새 콘텐츠')).toBeNull()
  })

  // TRD S21-storage §6-2: an unknown content slot can also have other content placed **explicitly** via the empty-slot menu (discarding the original is the shell's job).
  it('right-click on an unknown content slot → empty-slot menu → the fill is placed on that slot', () => {
    render(<ViewLayoutRenderer node={slotNode('slot-F', null)} focusedSlotId={null} foreignSlots={['slot-F']} />)
    fireEvent.contextMenu(frameOf('slot-F'))
    expect(screen.getByText('에이전트 모니터링')).toBeTruthy()
    openNewContentFlyout()
    fireEvent.click(screen.getByText('프리셋 팔레트 열기'))
    expect(setSlotContentSpy).toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-F', { type: 'preset_palette' })
  })

  // ── ★"detach to popup" = common (ADR-0064)★: shown regardless of content kind including empty (ADR-0228) and moves (viewId, slotId). ──
  it('"detach to popup" (common, non-empty) → calls moveSlotToWindow(viewId, slotId)', () => {
    openMenu('slot-P', 'agent-p')
    fireEvent.click(screen.getByText('팝업으로 분리'))
    expect(moveSlotToWindowSpy).toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-P')
  })

  it('"detach to popup" of an empty slot → calls moveSlotToWindow(viewId, slotId) (ADR-0228)', () => {
    openMenu('slot-PE', null)
    fireEvent.click(screen.getByText('팝업으로 분리'))
    expect(moveSlotToWindowSpy).toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-PE')
  })

  // ── ★viewIdOverride threading★ — the popup window path acts on the view passed in instead of activeViewId(=main) ──
  // ViewLayoutRenderer assembles ctx.viewId = viewIdOverride ?? currentViewId and passes it to command.run.
  it('with viewIdOverride, "split horizontally" calls split with the override view', () => {
    openMenu('slot-po', null, POPUP_VIEW)
    fireEvent.click(screen.getByText('가로 분할'))
    expect(splitSpy).toHaveBeenCalledWith(POPUP_VIEW, 'slot-po', 'top_bottom')
    expect(splitSpy).not.toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-po', 'top_bottom')
  })

  it('with viewIdOverride, "close" calls closeSlot with the override view', () => {
    openMenu('slot-pc', null, POPUP_VIEW)
    fireEvent.click(screen.getByText('닫기'))
    expect(closeSlotSpy).toHaveBeenCalledWith(POPUP_VIEW, 'slot-pc')
  })

  it('with viewIdOverride, "detach to popup" moves with the override view through moveSlotToWindow', () => {
    openMenu('slot-pp', 'po-agent', POPUP_VIEW)
    fireEvent.click(screen.getByText('팝업으로 분리'))
    expect(moveSlotToWindowSpy).toHaveBeenCalledWith(POPUP_VIEW, 'slot-pp')
  })

  it('without viewIdOverride (main window path) it falls back to activeViewId as before (backward compatible)', () => {
    openMenu('slot-main', null)
    fireEvent.click(screen.getByText('가로 분할'))
    expect(splitSpy).toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-main', 'top_bottom')
  })
})

// ── ADR-0373: a language switch re-renders the memo leaves (no remount) ─────────────────────────────
// Nothing above the leaf re-renders here (no App), so only `useLanguage()` in the memo boundaries can bring the new text.
describe('ViewLayoutRenderer — language switch', () => {
  afterEach(() => {
    act(() => {
      setLanguage('ko')
    })
  })

  it('a rendered SlotBody shows the new language, on the same node', () => {
    render(<ViewLayoutRenderer node={slotNode('s1', 'not-in-store')} focusedSlotId={null} />)
    const before = screen.getByText('에이전트 연결 중…')

    act(() => {
      setLanguage('en')
    })

    expect(screen.getByText('Connecting to agent…')).toBeTruthy()
    expect(screen.queryByText('에이전트 연결 중…')).toBeNull()
    // Same DOM node, re-rendered — not a remount.
    expect(before.isConnected).toBe(true)
    expect(before.textContent).toBe('Connecting to agent…')
  })

  it('the open slot menu shows translated command and container titles, and an open flyout stays open', () => {
    render(<ViewLayoutRenderer node={slotNode('s1', null)} focusedSlotId={null} />)
    fireEvent.contextMenu(frameOf('s1'))
    fireEvent.mouseEnter(screen.getByText('새 콘텐츠'))
    expect(screen.getByText('에이전트 트리 열기')).toBeTruthy()
    const flyout = document.querySelector('[data-slot-menu-flyout="container:slot.newContent"]')
    expect(flyout).not.toBeNull()

    act(() => {
      setLanguage('en')
    })

    expect(screen.getByText('New content')).toBeTruthy()
    expect(screen.getByText('Split horizontally')).toBeTruthy()
    expect(screen.getByText('Close')).toBeTruthy()
    // The flyout survived the switch (same node, still open) and its children are translated too.
    expect(document.querySelector('[data-slot-menu-flyout="container:slot.newContent"]')).toBe(flyout)
    expect(screen.getByText('Open agent tree')).toBeTruthy()
    expect(screen.queryByText('새 콘텐츠')).toBeNull()
  })
})
