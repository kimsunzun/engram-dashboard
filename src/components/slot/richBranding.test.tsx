// Rich slot backend notation regression — do title · marker · color tone tell claude and codex apart?
//
// Background: rich (structured) mode drew the two backends identically, so the screen had no signal of who you are talking to. The basis for telling them apart is
//   the profile's `command.kind` alone, and the convention is that **with no profile the claude notation stays**
//   (a regression-free default — ad-hoc sessions and mocks that do not fill the profile really take that path).
//
// Observation surface: empty-state title text · the marker's `data-rich-brand` · the root's `data-rich-brand` + tone class.
//   ★Why the root token is measured separately★: when a conversation starts the title and marker fold away together (ADR-0145) and then the
//   only DOM that tells the backend is the root attribute.
//
// Strategy: stub agentClient (clientFactory) and agentStore with the same pattern as RichSlot.test.tsx,
//   and fire the restore-complete signal ('live') to open the empty state.

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
  if (!el) throw new Error('RichSlot 루트가 없다')
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

describe('richBranding — 순수 매핑', () => {
  it('Codex 프로필만 codex 표기로 갈린다', () => {
    const b = richBranding('Codex')
    expect(b.brand).toBe('codex')
    expect(b.title).toBe('Codex')
    expect(b.tintClass).toBe('rich-tint-codex')
  })

  it('Claude 프로필은 기존 표기 그대로다(제목 문자열 불변 — 제품명이라 i18n 을 타지 않는다)', () => {
    const b = richBranding('Claude')
    expect(b.brand).toBe('claude')
    expect(b.title).toBe('Claude Code')
    // No tone = the look until now. If a class appears here, every existing screen diverges.
    expect(b.tintClass).toBe('')
  })

  // ★Default convention★: what is not known is not changed to a neutral notation — a screen that changes with nothing learned is worse.
  it('프로필 부재(null·undefined)와 미지 종류는 claude 표기로 떨어진다', () => {
    expect(richBranding(null).brand).toBe('claude')
    expect(richBranding(undefined).brand).toBe('claude')
    expect(richBranding('Shell').brand).toBe('claude')
  })
})

describe('RichSlot — 빈 상태 표기', () => {
  it('codex 프로필: 제목·표식·색조가 모두 codex 로 간다', async () => {
    agentStoreState.profiles = [profile('Codex')]
    await renderEmpty()

    expect(screen.getByText('Codex')).toBeTruthy()
    expect(screen.queryByText('Claude Code')).toBeNull()
    expect(mascot()?.getAttribute('data-rich-brand')).toBe('codex')
    expect(root().className).toContain('rich-tint-codex')
    expect(root().getAttribute('data-rich-brand')).toBe('codex')
  })

  it('claude 프로필: 표기가 하나도 바뀌지 않는다(색조 클래스 없음)', async () => {
    agentStoreState.profiles = [profile('Claude')]
    await renderEmpty()

    expect(screen.getByText('Claude Code')).toBeTruthy()
    expect(mascot()?.getAttribute('data-rich-brand')).toBe('claude')
    expect(root().className).not.toContain('rich-tint-codex')
    expect(root().getAttribute('data-rich-brand')).toBe('claude')
  })

  it('프로필이 없으면 claude 표기 — 색조도 붙지 않는다(현행 모습 유지)', async () => {
    await renderEmpty() // profiles = undefined (unit test mock · ad-hoc session path)
    expect(screen.getByText('Claude Code')).toBeTruthy()
    expect(mascot()?.getAttribute('data-rich-brand')).toBe('claude')
    expect(root().className).not.toContain('rich-tint-codex')
  })

  // The marker is pure decoration so it is hidden from the accessibility tree (ADR-0146 invariant) — the codex marker bears the same discipline as its sibling.
  it('codex 표식도 aria-hidden 이고 기존 표식 셀렉터에 그대로 걸린다', async () => {
    agentStoreState.profiles = [profile('Codex')]
    await renderEmpty()
    expect(mascot()).not.toBeNull()
    expect(mascot()?.getAttribute('aria-hidden')).toBe('true')
  })
})

describe('RichSlot — 대화 중에도 백엔드가 관측된다', () => {
  // When the empty state closes the title and marker disappear together — the only basis in that span is the root token.
  it('빈 상태가 아니어도 루트 토큰·색조는 남는다', async () => {
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
