// Rich slot backend notation regression — do title · marker · color tone tell claude and codex apart?
// Why: rich mode drew the two backends identically, so the screen gave no signal of who you talk to. The basis is the profile's `command.kind` alone, and **with no profile the claude notation stays** (regression-free default — ad-hoc sessions and mocks that do not fill the profile take that path).
// Observation surface: empty-state title text · the marker's `data-rich-brand` · the root's `data-rich-brand` + tone class. ★The root token is measured separately★: once a conversation starts the title and marker fold away together (ADR-0145) and the root attribute is the only DOM telling the backend.
// Strategy: stub agentClient (clientFactory) and agentStore as in RichSlot.test.tsx, and fire the restore-complete signal ('live') to open the empty state.

import { act, cleanup, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { OutputChunk, ViewPhase } from '../../api/agentClient'

// ── subscribeOutput callback capture (only the state callback is used — the signal that opens the empty-state gate). ──
const captured = vi.hoisted(() => ({
  onState: null as ((s: ViewPhase) => void) | null,
}))

vi.mock('../../api/clientFactory', () => ({
  agentClient: {
    // ADR-0046 signature (viewId, agentId, onChunk, onState?, onReset?).
    subscribeOutput: vi.fn(
      async (
        _viewId: string,
        _agentId: string,
        _onChunk: (c: OutputChunk) => void,
        onState?: (s: ViewPhase) => void,
      ) => {
        captured.onState = onState ?? null
        return { unsubscribe: vi.fn() }
      },
    ),
    writeStdin: vi.fn(async () => undefined),
    resizePty: vi.fn(async () => undefined),
    connectionState: 'connected',
    // Notifies the current state once on registration + returns a disposer (the real contract).
    onConnectionStateChange: (cb: (s: string) => void) => {
      cb('connected')
      return () => {}
    },
  },
  getAgentClient: vi.fn(),
}))

// ── agentStore stub — the absent decision (agents·agentsLoaded) and the notation decision (profiles) read here. ──
const agentStoreState = vi.hoisted(() => ({
  agents: [] as unknown[],
  agentsLoaded: false,
  profiles: undefined as unknown[] | undefined,
}))
vi.mock('../../store/agentStore', () => ({
  useAgentStore: (selector: (s: typeof agentStoreState) => unknown) => selector(agentStoreState),
}))

// ── Test subject ────────────────────────────────────────────────────────────────
import RichSlot from './RichSlot'
import { richBranding } from './richBranding'

const AGENT = 'aaaa-bbbb-cccc-dddd'

/** Minimal profile shape — fills only the fields the slot reads (id·command·cwd·display_name). */
function profile(kind: 'Claude' | 'Codex' | 'Shell'): unknown {
  const command =
    kind === 'Shell'
      ? { kind: 'Shell', program: 'pwsh', args: [] }
      : { kind, extra_args: [], output_format: 'StreamJson' }
  return { id: AGENT, name: 'C:/work', display_name: null, cwd: 'C:/work', command }
}

async function flush(): Promise<void> {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
}

/** Mount + restore-complete signal → a screen with the empty state (marker·title) open. */
async function renderEmpty(): Promise<void> {
  render(<RichSlot viewId="v1" agentId={AGENT} />)
  await flush()
  act(() => captured.onState!('live'))
}

function root(): HTMLElement {
  const el = document.querySelector('[data-rich-live="1"]')
  if (!el) throw new Error('the RichSlot root is missing')
  return el as HTMLElement
}

function mascot(): HTMLElement | null {
  return document.querySelector('[data-rich-mascot="1"]')
}

beforeEach(() => {
  captured.onState = null
  agentStoreState.agents = []
  agentStoreState.agentsLoaded = false
  agentStoreState.profiles = undefined
})

afterEach(() => {
  cleanup()
})

describe('richBranding — pure mapping', () => {
  it('only a Codex profile is told apart as codex notation', () => {
    const b = richBranding('Codex')
    expect(b.brand).toBe('codex')
    expect(b.title).toBe('Codex')
    expect(b.tintClass).toBe('rich-tint-codex')
  })

  it('a Claude profile is the existing notation as is (title string unchanged — a product name so it does not go through i18n)', () => {
    const b = richBranding('Claude')
    expect(b.brand).toBe('claude')
    expect(b.title).toBe('Claude Code')
    // No tone = the look until now. If a class appears here, every existing screen diverges.
    expect(b.tintClass).toBe('')
  })

  // ★Default convention★: what is not known is not changed to a neutral notation — a screen that changes with nothing learned is worse.
  it('profile absence (null·undefined) and unknown kinds fall to claude notation', () => {
    expect(richBranding(null).brand).toBe('claude')
    expect(richBranding(undefined).brand).toBe('claude')
    expect(richBranding('Shell').brand).toBe('claude')
  })
})

describe('RichSlot — empty state notation', () => {
  it('codex profile: title · marker · tint all go codex', async () => {
    agentStoreState.profiles = [profile('Codex')]
    await renderEmpty()

    expect(screen.getByText('Codex')).toBeTruthy()
    expect(screen.queryByText('Claude Code')).toBeNull()
    expect(mascot()?.getAttribute('data-rich-brand')).toBe('codex')
    expect(root().className).toContain('rich-tint-codex')
    expect(root().getAttribute('data-rich-brand')).toBe('codex')
  })

  it('claude profile: no notation changes at all (no tint class)', async () => {
    agentStoreState.profiles = [profile('Claude')]
    await renderEmpty()

    expect(screen.getByText('Claude Code')).toBeTruthy()
    expect(mascot()?.getAttribute('data-rich-brand')).toBe('claude')
    expect(root().className).not.toContain('rich-tint-codex')
    expect(root().getAttribute('data-rich-brand')).toBe('claude')
  })

  it('without a profile claude notation — no tint attached either (current look kept)', async () => {
    await renderEmpty() // profiles = undefined (unit test mock · ad-hoc session path)
    expect(screen.getByText('Claude Code')).toBeTruthy()
    expect(mascot()?.getAttribute('data-rich-brand')).toBe('claude')
    expect(root().className).not.toContain('rich-tint-codex')
  })

  // The marker is pure decoration so it is hidden from the accessibility tree (ADR-0146 invariant) — the codex marker bears the same discipline as its sibling.
  it('the codex marker is also aria-hidden and still hits the existing marker selector', async () => {
    agentStoreState.profiles = [profile('Codex')]
    await renderEmpty()
    expect(mascot()).not.toBeNull()
    expect(mascot()?.getAttribute('aria-hidden')).toBe('true')
  })
})

describe('RichSlot — the backend is also observed during a conversation', () => {
  // When the empty state closes the title and marker disappear together — the only basis in that span is the root token.
  it('even when not the empty state the root token · tint remain', async () => {
    agentStoreState.profiles = [profile('Codex')]
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    act(() => captured.onState!('buffering')) // Restoring = the span where the empty-state gate is closed

    expect(mascot()).toBeNull()
    expect(screen.queryByText('Codex')).toBeNull()
    expect(root().getAttribute('data-rich-brand')).toBe('codex')
    expect(root().className).toContain('rich-tint-codex')
  })
})
