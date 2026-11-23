// ADR-0050: verifies deterministic adapter behavior (mapping · absorption · filtering), and checks the leaf's internal render (chat/*) only at smoke level
//   (details like react-markdown are the leaf's own tests' job).

import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { StructuredEvent } from '../../../crates/engram-dashboard-protocol/bindings/StructuredEvent'
import { StructuredTextView } from './StructuredTextView'
import {
  StructuredEventAccumulator,
  type StructuredItem,
  type ToolCategory,
  type ToolItem,
} from './structuredAccumulator'
import { PLACEHOLDER_ERROR_MESSAGE } from '../../api/wsFrame'
import { setLanguage, t } from '../../i18n'
import { useToolGroupStore } from '../../store/toolGroupStore'

// The tool bundle expansion store is a zustand singleton, so isolate between tests.
beforeEach(() => useToolGroupStore.setState({ bySlot: {} }))
afterEach(() => cleanup())

// ── ADR-0051: rail connector clean-ends render ──────────────────────────────────────────
describe('StructuredTextView rail line clean-ends (ADR-0051)', () => {
  it('a single assistant row (single) draws no connector (dot only)', () => {
    const items: StructuredItem[] = [{ kind: 'text', text: 'solo', itemId: 0 }]
    const { container } = render(<StructuredTextView items={items} />)
    expect(container.querySelector('.w-px.bg-border')).toBeNull()
    expect(container.querySelector('.rounded-full.bg-muted')).toBeTruthy()
  })

  it('consecutive assistant rows draw a connector (w-px bg-border)', () => {
    const items: StructuredItem[] = [
      { kind: 'text', text: 'a', itemId: 0 },
      { kind: 'text', text: 'b', itemId: 1 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    expect(container.querySelector('.w-px.bg-border')).toBeTruthy()
  })
})

describe('StructuredTextView dispatch (ADR-0050)', () => {
  it('text item → rendered as the assistant markdown body', () => {
    const items: StructuredItem[] = [{ kind: 'text', text: 'hello **world**', itemId: 0 }]
    render(<StructuredTextView items={items} />)
    expect(screen.getByText('world').tagName.toLowerCase()).toBe('strong')
    expect(screen.getByText(/hello/)).toBeTruthy()
  })

  it("structured label=user → rendered as a user box (text extracted)", () => {
    const items: StructuredItem[] = [
      { kind: 'structured', label: 'user', json: JSON.stringify({ text: 'please fix it' }), itemId: 0 },
    ]
    render(<StructuredTextView items={items} />)
    expect(screen.getByText('please fix it')).toBeTruthy()
  })

  it('if structured label=user is a tool_result it is not rendered independently (absorbed into the tool OUT)', () => {
    const items: StructuredItem[] = [
      {
        kind: 'structured',
        label: 'user',
        json: JSON.stringify({ type: 'tool_result', tool_use_id: 'tu_1', content: 'RESULT_BODY' }),
        itemId: 0,
      },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    expect(screen.queryByText('RESULT_BODY')).toBeNull()
    expect(container.querySelectorAll('button').length).toBe(0)
  })

  it('structured label=thinking (with content) → rendered as ThoughtRow (title toggle), and clicking expands the body', () => {
    const items: StructuredItem[] = [
      { kind: 'structured', label: 'thinking', json: JSON.stringify({ thinking: 'let me reason' }), itemId: 0 },
    ]
    render(<StructuredTextView items={items} />)
    const toggle = screen.getByRole('button', { name: /Thought/ })
    expect(toggle).toBeTruthy()
    expect(screen.queryByText('let me reason')).toBeNull()
    fireEvent.click(toggle)
    expect(screen.getByText('let me reason')).toBeTruthy()
  })

  // Empty thinking = encrypted thinking (opus emits only the signature). rowKindOf is synced to 'skip' too so the rail calculation and
  //   DOM agree (ADR-0051).
  it('empty thinking (blank/missing) → no row is rendered (removes the empty "Thought" clutter)', () => {
    const items: StructuredItem[] = [
      { kind: 'structured', label: 'thinking', json: JSON.stringify({ thinking: '   ' }), itemId: 0 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    expect(screen.queryByText('Thought')).toBeNull()
    expect(container.querySelector('.rounded-full.bg-muted')).toBeNull()
    expect(container.querySelector('.relative.flex.px-4')).toBeNull()
  })

  it('structured other label → collapsed generic block (label header toggle)', () => {
    const items: StructuredItem[] = [
      { kind: 'structured', label: 'mystery', json: '{"a":1}', itemId: 0 },
    ]
    render(<StructuredTextView items={items} />)
    const toggle = screen.getByRole('button', { name: /mystery/ })
    expect(toggle.getAttribute('aria-expanded')).toBe('false')
    fireEvent.click(toggle)
    expect(toggle.getAttribute('aria-expanded')).toBe('true')
  })

  it('tool item → name+hint header (collapsed), clicking expands the IN(args) detail', () => {
    const items: StructuredItem[] = [
      { kind: 'tool', name: 'Read', argsJson: '{"path":"a.ts"}', id: 'tu_1', category: 'Read', resultMark: null, itemId: 0 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    const header = screen.getByRole('button', { name: /Read/ })
    expect(header.getAttribute('aria-expanded')).toBe('false')
    expect(container.querySelector('pre')).toBeNull()
    fireEvent.click(header)
    expect(header.getAttribute('aria-expanded')).toBe('true')
    expect(screen.getByText(t('chat.toolInput'))).toBeTruthy()
    const pre = container.querySelector('pre')
    expect(pre).toBeTruthy()
    expect(pre?.textContent).toContain('a.ts')
  })

  it('if a tool item has a matching tool_result, expanding also draws the OUT result', () => {
    const items: StructuredItem[] = [
      { kind: 'tool', name: 'Bash', argsJson: '{"command":"ls"}', id: 'tu_9', category: 'Command', resultMark: null, itemId: 0 },
      {
        kind: 'structured',
        label: 'user',
        json: JSON.stringify({ type: 'tool_result', tool_use_id: 'tu_9', content: 'FILE_LISTING' }),
        itemId: 1,
      },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    const header = screen.getByRole('button', { name: /Bash/ })
    fireEvent.click(header)
    expect(screen.getByText(t('chat.toolOutput'))).toBeTruthy()
    const pres = Array.from(container.querySelectorAll('pre'))
    expect(pres.some((p) => p.textContent?.includes('FILE_LISTING'))).toBe(true)
  })

  it('usage item → renders nothing (no per-message token chip)', () => {
    const items: StructuredItem[] = [{ kind: 'usage', inputTokens: 2, outputTokens: 5, itemId: 0 }]
    const { container } = render(<StructuredTextView items={items} />)
    expect(screen.queryByText(/in 2/)).toBeNull()
    expect(screen.queryByText(/out 5/)).toBeNull()
    expect(container.querySelector('.relative.px-4')).toBeNull()
  })

  it('error item → red error row (message shown)', () => {
    const items: StructuredItem[] = [{ kind: 'error', message: 'boom happened', itemId: 0 }]
    render(<StructuredTextView items={items} />)
    expect(screen.getByText('boom happened')).toBeTruthy()
  })

  // ── The three turn outcomes are told apart on screen (that distinction is the outcome field's reason to exist) ──────────────────
  it('outcome=failed → failure text + reason (detail) shown', () => {
    const items: StructuredItem[] = [
      { kind: 'outcome', outcome: 'failed', detail: 'model refused', itemId: 0 },
    ]
    render(<StructuredTextView items={items} />)
    expect(screen.getByText(t('chat.turnFailed'))).toBeTruthy()
    expect(screen.getByText('model refused')).toBeTruthy()
  })

  it('if outcome=failed has no reason no reason line is made (no empty line)', () => {
    const items: StructuredItem[] = [
      { kind: 'outcome', outcome: 'failed', detail: null, itemId: 0 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    expect(screen.getByText(t('chat.turnFailed'))).toBeTruthy()
    expect(container.querySelector('.whitespace-pre-wrap')).toBeNull()
  })

  it('outcome=interrupted is not drawn as a failure (no red tone · no failure text)', () => {
    const items: StructuredItem[] = [
      { kind: 'outcome', outcome: 'interrupted', detail: null, itemId: 0 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    expect(screen.getByText(t('chat.turnInterrupted'))).toBeTruthy()
    expect(screen.queryByText(t('chat.turnFailed'))).toBeNull()
    expect(container.querySelector('.text-red-500')).toBeNull()
    // The rail dot is not a failure tone either (error uses the bg-red-500 dot).
    expect(container.querySelector('.rounded-full.bg-red-500')).toBeNull()
  })

  it('outcome=unknown is drawn as 「unknown」, not a failure', () => {
    const items: StructuredItem[] = [
      { kind: 'outcome', outcome: 'unknown', detail: null, itemId: 0 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    expect(screen.getByText(t('chat.turnUnknown'))).toBeTruthy()
    expect(container.querySelector('.text-red-500')).toBeNull()
  })

  it('the three outcomes use different text (not smeared into one shape)', () => {
    const labels = [t('chat.turnFailed'), t('chat.turnInterrupted'), t('chat.turnUnknown')]
    expect(new Set(labels).size).toBe(3)
  })

  // ── Unknown event marker — visible, but no protocol word shows ─────────────────────────────
  it('unsupported item → a one-line notice carrying the count (no original name · JSON)', () => {
    const items: StructuredItem[] = [{ kind: 'unsupported', count: 3, itemId: 0 }]
    const { container } = render(<StructuredTextView items={items} />)
    expect(screen.getByText(t('chat.unsupportedEvent', { count: '3' }))).toBeTruthy()
    // Nothing to expand either — it does not leak into GenericItemRow (JSON code block).
    expect(container.querySelectorAll('button').length).toBe(0)
    expect(container.querySelector('pre')).toBeNull()
  })

  it('separator item → a faint vertical spacer (no border-t divider)', () => {
    const items: StructuredItem[] = [
      { kind: 'text', text: 'a', itemId: 0 },
      { kind: 'separator', itemId: 1 },
      { kind: 'text', text: 'b', itemId: 2 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    expect(container.querySelector('div[aria-hidden].border-t')).toBeNull()
    const spacer = container.querySelector('div[aria-hidden]')
    expect(spacer).toBeTruthy()
    expect(spacer?.className).toContain('h-3')
  })

  // User decision 2026-10-01: the wait indicator is the job of the row above the input box outside the list (RichSlot · `chat/WaitRow.tsx`).
  it('even when streaming no waiting indicator is drawn inside the list, and the last row does not extend the rail toward it', () => {
    const items: StructuredItem[] = [{ kind: 'text', text: 'working', itemId: 0 }]
    const { container } = render(<StructuredTextView items={items} streaming />)
    expect(screen.queryByText(t('chat.wait'))).toBeNull()
    expect(container.querySelector('[data-wait-strip]')).toBeNull()
    expect(container.querySelector('.w-px.bg-border')).toBeNull() // One row = single — no connector
  })

  // ── Margin below the last row — exactly one block of the same height (h-3) as the turn divider ──────────────────────
  const bottomSpacers = (c: HTMLElement) => c.querySelectorAll('[data-chat-bottom-spacer]')
  const lastChildClass = (c: HTMLElement) => (c.firstElementChild as HTMLElement).lastElementChild?.className ?? ''

  it('if the end is not a turn divider one spacer block (h-3) is attached — even if an error · unknown-event row is the end', () => {
    for (const last of [
      { kind: 'text', text: 'working', itemId: 1 },
      { kind: 'error', message: 'overloaded [willRetry]', itemId: 1 },
      { kind: 'unsupported', count: 2, itemId: 1 },
    ] as StructuredItem[]) {
      const { container } = render(<StructuredTextView items={[{ kind: 'text', text: 'a', itemId: 0 }, last]} streaming />)
      expect(bottomSpacers(container)).toHaveLength(1)
      expect(lastChildClass(container)).toContain('h-3')
      cleanup()
    }
  })

  it('if the end is a turn divider (even with only non-drawn rows after it) no separate spacer block is placed', () => {
    const items: StructuredItem[] = [
      { kind: 'text', text: 'done', itemId: 0 },
      { kind: 'separator', itemId: 1 },
      { kind: 'usage', inputTokens: 2, outputTokens: 5, itemId: 2 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    expect(bottomSpacers(container)).toHaveLength(0)
    expect(container.querySelectorAll('div[aria-hidden].h-3')).toHaveLength(1)
  })

  it('with no row to draw there is no spacer block either', () => {
    const { container } = render(<StructuredTextView items={[]} streaming />)
    expect(container.querySelectorAll('div[aria-hidden].h-3')).toHaveLength(0)
  })

  // ★The list height does not bounce at turn end★ — the margin block just swaps places with the divider, and there is still exactly one h-3 at the end.
  it('even when the turn ends and a divider is attached the trailing h-3 is exactly one', () => {
    const reply: StructuredItem = { kind: 'text', text: 'reply', itemId: 0 }
    const { container, rerender } = render(<StructuredTextView items={[reply]} streaming />)
    expect(container.querySelectorAll('div[aria-hidden].h-3')).toHaveLength(1)
    rerender(<StructuredTextView items={[reply, { kind: 'separator', itemId: 1 }]} />)
    expect(container.querySelectorAll('div[aria-hidden].h-3')).toHaveLength(1)
    expect(bottomSpacers(container)).toHaveLength(0)
    expect(lastChildClass(container)).toContain('h-3')
  })

  it('even if malformed json arrives it does not throw and renders the fallback (safe parser)', () => {
    const items: StructuredItem[] = [
      { kind: 'structured', label: 'thinking', json: '{bad json', itemId: 0 },
    ]
    // extractText fallback = raw json → not empty, so ThoughtRow (interactive "Thought") shows up (without a throw).
    expect(() => render(<StructuredTextView items={items} />)).not.toThrow()
    expect(screen.getByRole('button', { name: /Thought/ })).toBeTruthy()
  })

  // ── FIX 1: tool_result absorption is label-independent ──────────────────────────────────────────
  it('a structured tool_result with a NON-user label is also not rendered independently (absorbed regardless of label — FIX 1)', () => {
    const items: StructuredItem[] = [
      {
        kind: 'structured',
        label: 'mystery',
        json: JSON.stringify({ type: 'tool_result', tool_use_id: 'tu_x', content: 'HIDDEN_BODY' }),
        itemId: 0,
      },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    // Previous bug: a tool_result outside the user branch was rendered standalone as GenericItemRow.
    expect(screen.queryByText('HIDDEN_BODY')).toBeNull()
    expect(screen.queryByRole('button', { name: /mystery/ })).toBeNull()
    expect(container.querySelectorAll('button').length).toBe(0)
  })

  // ── tool id=null: safe render without OUT ──────────────────────────────────────────────
  it('if a tool item has id=null it renders only name/hint without an OUT block and does not crash', () => {
    const items: StructuredItem[] = [
      { kind: 'tool', name: 'Glob', argsJson: '{"pattern":"**/*.ts"}', id: null, category: 'Search', resultMark: null, itemId: 0 },
    ]
    expect(() => render(<StructuredTextView items={items} />)).not.toThrow()
    const header = screen.getByRole('button', { name: /Glob/ })
    fireEvent.click(header)
    expect(screen.getByText(t('chat.toolInput'))).toBeTruthy()
    expect(screen.queryByText(t('chat.toolOutput'))).toBeNull()
  })

  // ── unmatched tool_result: not rendered standalone ───────────────────────────────
  it('a tool_result whose id matches no tool is not rendered standalone', () => {
    const items: StructuredItem[] = [
      { kind: 'tool', name: 'Read', argsJson: '{"path":"a.ts"}', id: 'tu_1', category: 'Read', resultMark: null, itemId: 0 },
      {
        kind: 'structured',
        label: 'user',
        json: JSON.stringify({ type: 'tool_result', tool_use_id: 'tu_ORPHAN', content: 'ORPHAN_BODY' }),
        itemId: 1,
      },
    ]
    render(<StructuredTextView items={items} />)
    expect(screen.queryByText('ORPHAN_BODY')).toBeNull()
    // The only button = the single unmatched tool header (Read) (an orphan tool_result makes no button).
    expect(screen.getAllByRole('button').length).toBe(1)
  })

  // ── malformed json fallback (no throw) — tool args + generic json ────────────────────
  it('even if malformed argsJson · generic json arrives it renders the fallback and does not throw', () => {
    const items: StructuredItem[] = [
      { kind: 'tool', name: 'Bad', argsJson: '{not valid', id: 'tu_2', category: 'Other', resultMark: null, itemId: 0 },
      { kind: 'structured', label: 'weird', json: '{also bad', itemId: 1 },
    ]
    expect(() => render(<StructuredTextView items={items} />)).not.toThrow()
    fireEvent.click(screen.getByRole('button', { name: /Bad/ }))
    const toolPre = document.querySelector('pre')
    expect(toolPre?.textContent).toContain('{not valid')
    fireEvent.click(screen.getByRole('button', { name: /weird/ }))
    const pres = Array.from(document.querySelectorAll('pre'))
    expect(pres.some((p) => p.textContent?.includes('{also bad'))).toBe(true)
  })

  // ── FIX 2: triple backticks in tool OUT are inert (no markdown promotion) ─────────────────────
  it('even if a tool OUT has triple backticks + markdown it is inert (no markdown elements such as heading are created — FIX 2)', () => {
    const evil = '```\n# NOT_A_HEADING\n[link](http://evil.example)\n```'
    const items: StructuredItem[] = [
      { kind: 'tool', name: 'Cat', argsJson: '{"path":"x"}', id: 'tu_3', category: 'Other', resultMark: null, itemId: 0 },
      {
        kind: 'structured',
        label: 'user',
        json: JSON.stringify({ type: 'tool_result', tool_use_id: 'tu_3', content: evil }),
        itemId: 1,
      },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    fireEvent.click(screen.getByRole('button', { name: /Cat/ }))
    expect(container.querySelector('h1')).toBeNull()
    expect(container.querySelector('a')).toBeNull()
    const pres = Array.from(container.querySelectorAll('pre'))
    expect(pres.some((p) => p.textContent?.includes('# NOT_A_HEADING'))).toBe(true)
    expect(pres.some((p) => p.textContent?.includes('[link](http://evil.example)'))).toBe(true)
  })

  // ── ADR-0050: dot-rail skeleton structure ───────────────────────────────────────────────
  it('uses the dot-rail skeleton instead of a dashed (border-dashed) rail', () => {
    const items: StructuredItem[] = [
      { kind: 'text', text: 'hi', itemId: 0 },
      { kind: 'tool', name: 'Read', argsJson: '{"path":"a.ts"}', id: 'tu_1', category: 'Read', resultMark: null, itemId: 1 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    expect(container.querySelector('.border-dashed')).toBeNull()
    // A rail row is wrapped in a ChatRow wrapper (relative flex px-4) — top-padding is a CSS variable inline style (ADR-0051).
    expect(container.querySelector('.relative.flex.px-4')).toBeTruthy()
  })

  it('an assistant-side row (text) renders a left rail gutter + dot marker', () => {
    const items: StructuredItem[] = [{ kind: 'text', text: 'hello', itemId: 0 }]
    const { container } = render(<StructuredTextView items={items} />)
    // The rail-mode wrapper is a flex row (relative flex px-4) — top-padding is a CSS variable inline style (ADR-0051).
    const row = container.querySelector('.relative.flex.px-4')
    expect(row).toBeTruthy()
    expect(row?.className).toContain('flex')
    const dot = container.querySelector('.rounded-full.bg-muted')
    expect(dot).toBeTruthy()
    // The content column is flex-1 min-w-0 (prevents long-token overflow).
    expect(container.querySelector('.flex-1.min-w-0')).toBeTruthy()
    // ※ The connector (w-px bg-border) is conditional on run length (single = none) — see the separate clean-ends test.
  })

  it('rail dot color = row kind: tool is green (bg-green-500), reasoning/body is muted', () => {
    const items: StructuredItem[] = [
      { kind: 'text', text: 'hi', itemId: 0 },
      { kind: 'tool', name: 'Bash', argsJson: '{"command":"ls"}', id: 'tu_1', category: 'Command', resultMark: null, itemId: 1 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    expect(container.querySelector('.rounded-full.bg-green-500')).toBeTruthy()
    expect(container.querySelector('.rounded-full.bg-muted')).toBeTruthy()
  })

  it('a user bubble row has no rail gutter/dot marker (plain full-width)', () => {
    const items: StructuredItem[] = [
      { kind: 'structured', label: 'user', json: JSON.stringify({ text: 'ping' }), itemId: 0 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    expect(container.querySelector('.rounded-full.bg-muted')).toBeNull()
    expect(container.querySelector('.flex-1.min-w-0')).toBeNull()
    // The outer wrapper is still relative px-4 (not flex) — top-padding is a CSS variable inline style (ADR-0051).
    const row = container.querySelector('.relative.px-4')
    expect(row).toBeTruthy()
    expect(row?.className).not.toContain('flex')
  })

  it('structured label=user → rendered as an extension-look bubble (rounded-[0.75rem] border bg-elevated, inset margin)', () => {
    const items: StructuredItem[] = [
      { kind: 'structured', label: 'user', json: JSON.stringify({ text: 'do the thing' }), itemId: 0 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    const bubble = screen.getByText('do the thing')
    // bg-elevated = a background one step brighter than the page in dark (visibility).
    expect(bubble.className).toContain('rounded-[0.75rem]')
    expect(bubble.className).toContain('border')
    expect(bubble.className).toContain('bg-elevated')
    expect((bubble as HTMLElement).style.marginLeft).toBe('0.75rem')
    expect((bubble as HTMLElement).style.marginRight).toBe('0.75rem')
    expect(container.querySelectorAll('button').length).toBe(0)
  })

  it('tool item → rendered as header (icon + bold name) + bg-surface box', () => {
    const items: StructuredItem[] = [
      { kind: 'tool', name: 'Bash', argsJson: '{"command":"ls"}', id: 'tu_1', category: 'Command', resultMark: null, itemId: 0 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    const title = screen.getByText('Bash')
    expect(title.className).toContain('font-bold')
    expect(container.querySelector('.bg-surface.rounded-sm')).toBeTruthy()
  })
})

// ── ADR-0239 · ADR-0241: tool bundle · end marker (TRD S21-chat-ux §4-6 · §4-7 ⑨ frontend) ──────────────

let nextToolItemId = 1000
function toolItem(
  name: string,
  id: string | null,
  category: ToolCategory,
  extra: Partial<ToolItem> = {},
): ToolItem {
  return {
    kind: 'tool',
    name,
    argsJson: '{"path":"a.ts"}',
    id,
    category,
    resultMark: null,
    itemId: nextToolItemId++,
    ...extra,
  }
}
/** A carrier row on which claude puts the result body — not drawn (skip). */
function resultCarrier(toolUseId: string, isError: boolean): StructuredItem {
  return {
    kind: 'structured',
    label: 'user',
    json: JSON.stringify({ type: 'tool_result', tool_use_id: toolUseId, content: 'out', is_error: isError }),
    itemId: nextToolItemId++,
  }
}
const textItem = (s: string): StructuredItem => ({ kind: 'text', text: s, itemId: nextToolItemId++ })
const thoughtItem = (s: string): StructuredItem => ({
  kind: 'structured',
  label: 'thinking',
  json: JSON.stringify({ thinking: s }),
  itemId: nextToolItemId++,
})
const interruptedOutcome = (): StructuredItem => ({
  kind: 'outcome',
  outcome: 'interrupted',
  detail: null,
  itemId: nextToolItemId++,
})

const groups = (c: HTMLElement) => Array.from(c.querySelectorAll<HTMLElement>('[data-tool-group]'))
const groupHeader = (g: HTMLElement) => g.querySelector('button') as HTMLButtonElement
const railRows = (c: HTMLElement) => c.querySelectorAll('.relative.flex.px-4').length
const railDots = (c: HTMLElement) => c.querySelectorAll('.rounded-full').length
const summary = (...parts: string[]) => parts.join(' · ')

describe('StructuredTextView tool group (ADR-0239)', () => {
  it('one tool is not grouped — that row as today', () => {
    const { container } = render(
      <StructuredTextView items={[toolItem('Read', 'r1', 'Read'), textItem('done')]} />,
    )
    expect(groups(container)).toEqual([])
    expect(screen.getByRole('button', { name: 'Read' })).toBeTruthy()
  })

  it('two or more tools = one group · carrier row · continues even with an empty thought in between · DOM marker', () => {
    const items: StructuredItem[] = [
      toolItem('Read', 'r1', 'Read'),
      resultCarrier('r1', false),
      thoughtItem('   '),
      toolItem('Grep', 'g1', 'Search'),
      textItem('answer'),
    ]
    const { container } = render(<StructuredTextView items={items} />)
    const [g] = groups(container)
    expect(groups(container)).toHaveLength(1)
    expect(g.getAttribute('data-tool-group')).toBe('tool:r1')
    expect(g.getAttribute('data-tool-group-open')).toBe('0')
    expect(g.getAttribute('data-tool-group-count')).toBe('2')
    expect(groupHeader(g).textContent).toBe(
      summary(t('chat.toolGroupSearch', { count: '1' }), t('chat.toolGroupRead', { count: '1' })),
    )
    // A collapsed bundle does not draw its member rows.
    expect(screen.queryByRole('button', { name: 'Read' })).toBeNull()
  })

  it('if the first call has no backend id the key = item:<itemId>', () => {
    const first = toolItem('Read', null, 'Read')
    const { container } = render(<StructuredTextView items={[first, toolItem('Read', 'r2', 'Read')]} />)
    expect(groups(container)[0].getAttribute('data-tool-group')).toBe(`item:${first.itemId}`)
  })

  it('the rail counts by row list — group = one rail row · even expanded, members do not draw the rail (ADR-0051)', () => {
    const items: StructuredItem[] = [
      textItem('before'),
      toolItem('Read', 'r1', 'Read'),
      toolItem('Read', 'r2', 'Read'),
      toolItem('Bash', 'b1', 'Command'),
      textItem('after'),
    ]
    const { container } = render(<StructuredTextView items={items} />)
    // text · bundle · text = three rows — the three connect, so there are three connectors too (top · mid · bottom).
    expect(railRows(container)).toBe(3)
    expect(railDots(container)).toBe(3)
    expect(container.querySelectorAll('.w-px.bg-border').length).toBe(3)

    useToolGroupStore.getState().bind('s1', 'agent-1')
    cleanup()
    const expanded = render(<StructuredTextView items={items} slotId="s1" />)
    fireEvent.click(groupHeader(groups(expanded.container)[0]))
    expect(screen.getAllByRole('button', { name: 'Read' })).toHaveLength(2)
    expect(screen.getByRole('button', { name: 'Bash' })).toBeTruthy()
    expect(railRows(expanded.container)).toBe(3)
    expect(railDots(expanded.container)).toBe(3)
  })

  it('an expanded group [tool · thought · tool] — the thought row is drawn inside the group and the rail counts the group as one row', () => {
    useToolGroupStore.getState().bind('s1', 'agent-1')
    const items: StructuredItem[] = [
      textItem('before'),
      toolItem('Read', 'r1', 'Read'),
      thoughtItem('mid thought'),
      toolItem('Grep', 'g1', 'Search'),
      textItem('after'),
    ]
    const { container } = render(<StructuredTextView items={items} slotId="s1" />)
    const [g] = groups(container)
    fireEvent.click(groupHeader(g))
    expect(g.getAttribute('data-tool-group-open')).toBe('1')
    expect(g.contains(screen.getByText('Thought'))).toBe(true)
    expect(g.contains(screen.getByRole('button', { name: 'Read' }))).toBe(true)
    expect(g.contains(screen.getByRole('button', { name: 'Grep' }))).toBe(true)
    expect(railRows(container)).toBe(3)
    expect(railDots(container)).toBe(3)
  })

  // ADR-0263 decision 1: a running bundle is not expanded whole — head (progressive) + the last call running now as one line, and when done just the head (past tense).
  const readCall = (id: string) => toolItem('Read', id, 'Read', { argsJson: JSON.stringify({ path: `${id}.ts` }) })
  const shownHints = () => screen.queryAllByText(/^r\d\.ts$/).map((el) => el.textContent)

  it('the last group of a running turn is the head + one line of the last call · when calls grow that line changes · when text arrives only the head', () => {
    const calls = [readCall('r1'), readCall('r2')]
    const { container, rerender } = render(<StructuredTextView items={calls} streaming />)
    const g = groups(container)[0]
    const head = groupHeader(g)
    const live = () => groups(container)[0].getAttribute('data-tool-group-live')
    expect(g.getAttribute('data-tool-group-open')).toBe('0')
    expect(live()).toBe('1')
    expect(screen.getByText(t('chat.toolGroupRunning'))).toBeTruthy()
    expect(shownHints()).toEqual(['r2.ts'])

    const more = [...calls, thoughtItem('hmm'), readCall('r3')]
    rerender(<StructuredTextView items={more} streaming />)
    expect(groups(container)[0]).toBe(g) // Only the count grows on the same node
    expect(groupHeader(g)).toBe(head)
    expect(g.getAttribute('data-tool-group-count')).toBe('3')
    expect(g.getAttribute('data-tool-group-open')).toBe('0')
    expect(shownHints()).toEqual(['r3.ts'])

    rerender(<StructuredTextView items={[...more, textItem('here')]} streaming />)
    expect(groups(container)[0]).toBe(g)
    expect(live()).toBe('0')
    expect(screen.queryByText(t('chat.toolGroupRunning'))).toBeNull()
    expect(shownHints()).toEqual([])
  })

  it('when the turn ends the tail group is also head only — past tense', () => {
    const calls = [readCall('r1'), readCall('r2')]
    const { container, rerender } = render(<StructuredTextView items={calls} streaming />)
    rerender(<StructuredTextView items={calls} />)
    expect(groups(container)[0].getAttribute('data-tool-group-open')).toBe('0')
    expect(groups(container)[0].getAttribute('data-tool-group-live')).toBe('0')
    expect(shownHints()).toEqual([])
  })

  it('the human toggle wins — expanding while running shows everything as it grows and stays so at turn end · collapsing while running leaves only the head', () => {
    useToolGroupStore.getState().bind('s1', 'agent-1')
    const calls = [readCall('r1'), readCall('r2')]
    const { container, rerender } = render(<StructuredTextView items={calls} streaming slotId="s1" />)
    fireEvent.click(groupHeader(groups(container)[0]))
    expect(groups(container)[0].getAttribute('data-tool-group-open')).toBe('1')
    expect(shownHints()).toEqual(['r1.ts', 'r2.ts'])

    const more = [...calls, readCall('r3')]
    rerender(<StructuredTextView items={more} streaming slotId="s1" />)
    expect(shownHints()).toEqual(['r1.ts', 'r2.ts', 'r3.ts'])

    fireEvent.click(groupHeader(groups(container)[0]))
    expect(groups(container)[0].getAttribute('data-tool-group-open')).toBe('0')
    expect(shownHints()).toEqual([]) // Even while running, a chosen collapse also takes away the last call line
    fireEvent.click(groupHeader(groups(container)[0]))

    const ended = [...more, textItem('answer')]
    rerender(<StructuredTextView items={ended} slotId="s1" />)
    expect(groups(container)[0].getAttribute('data-tool-group-open')).toBe('1')
    expect(shownHints()).toEqual(['r1.ts', 'r2.ts', 'r3.ts'])
    fireEvent.click(groupHeader(groups(container)[0]))
    expect(groups(container)[0].getAttribute('data-tool-group-open')).toBe('0')
    rerender(<StructuredTextView items={[...ended]} slotId="s1" />)
    expect(groups(container)[0].getAttribute('data-tool-group-open')).toBe('0')
  })

  it('even feeding the same event sequence into a new accumulator again (replay) the chosen expansion stays attached', () => {
    const events = [
      { type: 'ToolCall', name: 'Read', args_json: '{}', id: 'r1', turn_id: null, message_id: null, category: 'Read' },
      { type: 'ToolCall', name: 'Read', args_json: '{}', id: 'r2', turn_id: null, message_id: null, category: 'Read' },
      { type: 'TextDelta', text: 'answer', turn_id: null, message_id: null },
    ]
    const snapshotOf = () => {
      const acc = new StructuredEventAccumulator()
      for (const ev of events) acc.feed(new TextEncoder().encode(JSON.stringify(ev as StructuredEvent)))
      return acc.snapshot()
    }
    useToolGroupStore.getState().bind('s1', 'agent-1')
    const { container, rerender } = render(<StructuredTextView items={snapshotOf()} slotId="s1" />)
    fireEvent.click(groupHeader(groups(container)[0]))
    rerender(<StructuredTextView items={snapshotOf()} slotId="s1" />)
    expect(groups(container)[0].getAttribute('data-tool-group')).toBe('tool:r1')
    expect(groups(container)[0].getAttribute('data-tool-group-open')).toBe('1')
  })

  it('without a slotId it stays collapsed always and pressing the head does not change it', () => {
    const calls = [toolItem('Read', 'r1', 'Read'), toolItem('Read', 'r2', 'Read')]
    const { container } = render(<StructuredTextView items={calls} />)
    const header = groupHeader(groups(container)[0])
    expect(header.disabled).toBe(true)
    fireEvent.click(header)
    expect(groups(container)[0].getAttribute('data-tool-group-open')).toBe('0')
  })

  it('expansion notice — false for a non-last group · true for the last group', () => {
    useToolGroupStore.getState().bind('s1', 'agent-1')
    const onGroupToggle = vi.fn()
    const items: StructuredItem[] = [
      toolItem('Read', 'r1', 'Read'),
      toolItem('Read', 'r2', 'Read'),
      textItem('middle'),
      toolItem('Bash', 'b1', 'Command'),
      toolItem('Bash', 'b2', 'Command'),
    ]
    const { container } = render(
      <StructuredTextView items={items} slotId="s1" onGroupToggle={onGroupToggle} />,
    )
    const [first, last] = groups(container)
    fireEvent.click(groupHeader(first))
    fireEvent.click(groupHeader(last))
    expect(onGroupToggle.mock.calls).toEqual([[false], [true]])
  })
})

describe('StructuredTextView tool end marker (ADR-0241)', () => {
  it('codex failure (resultMark failed) → red Error badge · data-tool-mark="failed" · no Out cell', () => {
    const { container } = render(
      <StructuredTextView items={[toolItem('commandExecution', 'c1', 'Command', { resultMark: 'failed' })]} />,
    )
    const row = container.querySelector('[data-tool-mark]') as HTMLElement
    expect(row.getAttribute('data-tool-mark')).toBe('failed')
    expect(screen.getByText(t('chat.toolErrorBadge')).className).toContain('text-red-500')
    fireEvent.click(screen.getByRole('button', { name: 'commandExecution' }))
    expect(screen.getByText(t('chat.toolInput'))).toBeTruthy()
    expect(screen.queryByText(t('chat.toolOutput'))).toBeNull()
  })

  it('claude vendor error (is_error) → red Error badge · no data-tool-mark (it is not a marker)', () => {
    const { container } = render(
      <StructuredTextView items={[toolItem('Bash', 'b1', 'Command'), resultCarrier('b1', true)]} />,
    )
    expect(screen.getByText(t('chat.toolErrorBadge')).className).toContain('text-red-500')
    expect(container.querySelector('[data-tool-mark]')).toBeNull()
  })

  it.each([
    ['refused', 'chat.toolRefusedReason'],
    ['declined', 'chat.toolDeclinedReason'],
  ] as const)('rejection row (%s) — orange 「rejected」 badge · reason line per source (also while collapsed) · no red marker', (mark, reasonKey) => {
    const { container } = render(
      <StructuredTextView items={[toolItem('commandExecution', 'c1', 'Command', { resultMark: mark })]} />,
    )
    const badge = screen.getByText(t('chat.toolDeclined'))
    expect(badge.style.color).toBe('var(--status-blocked)')
    expect(badge.style.borderColor).toBe('var(--status-blocked)')
    expect(screen.queryByText(t('chat.toolErrorBadge'))).toBeNull()
    expect(container.querySelector('.text-red-500, .border-red-500, .border-red-500\\/60')).toBeNull()
    // The head (tool name) is the default tone.
    expect(screen.getByText('commandExecution').className).toContain('text-foreground')

    const row = container.querySelector('[data-tool-mark]') as HTMLElement
    expect(row.getAttribute('data-tool-mark')).toBe('declined')
    const detail = screen.getByRole('button', { name: 'commandExecution' })
    expect(detail.getAttribute('aria-expanded')).toBe('false')
    const reason = container.querySelector('[data-tool-declined-reason]') as HTMLElement
    expect(reason.getAttribute('data-tool-declined-reason')).toBe(reasonKey)
    expect(reason.textContent).toBe(t(reasonKey))
  })

  it('the two sources have the same badge · color · data-tool-mark and only the reason key differs — group summary 「rejected 2」 · no error cell', () => {
    useToolGroupStore.getState().bind('s1', 'agent-1')
    const items = [
      toolItem('commandExecution', 'c1', 'Command', { resultMark: 'refused' }),
      toolItem('fileChange', 'f1', 'Edit', { resultMark: 'declined' }),
    ]
    const { container } = render(<StructuredTextView items={items} slotId="s1" />)
    const [g] = groups(container)
    expect(groupHeader(g).textContent).toBe(
      summary(
        t('chat.toolGroupEdit', { count: '1' }),
        t('chat.toolGroupCommand', { count: '1' }),
        t('chat.toolGroupDeclined', { count: '2' }),
      ),
    )
    fireEvent.click(groupHeader(g))
    const marks = Array.from(container.querySelectorAll('[data-tool-mark]')).map((el) => el.getAttribute('data-tool-mark'))
    expect(marks).toEqual(['declined', 'declined'])
    const badges = screen.getAllByText(t('chat.toolDeclined'))
    expect(badges.map((b) => b.style.color)).toEqual(['var(--status-blocked)', 'var(--status-blocked)'])
    const reasons = Array.from(container.querySelectorAll('[data-tool-declined-reason]')).map((el) =>
      el.getAttribute('data-tool-declined-reason'),
    )
    expect(reasons).toEqual(['chat.toolRefusedReason', 'chat.toolDeclinedReason'])
  })

  it('group summary order is 「error 1 · rejected 1」 — a rejection does not count toward the error count', () => {
    const items = [
      toolItem('commandExecution', 'c1', 'Command', { resultMark: 'failed' }),
      toolItem('commandExecution', 'c2', 'Command', { resultMark: 'refused' }),
    ]
    const { container } = render(<StructuredTextView items={items} />)
    expect(groupHeader(groups(container)[0]).textContent).toBe(
      summary(
        t('chat.toolGroupCommand', { count: '2' }),
        t('chat.toolGroupErrors', { count: '1' }),
        t('chat.toolGroupDeclined', { count: '1' }),
      ),
    )
  })

  it('a rejection marker wins over the vendor error body — both the badge and the head count', () => {
    useToolGroupStore.getState().bind('s1', 'agent-1')
    const items: StructuredItem[] = [
      toolItem('Read', 'r1', 'Read'),
      toolItem('Bash', 'b1', 'Command', { resultMark: 'declined' }),
      resultCarrier('b1', true),
    ]
    const { container } = render(<StructuredTextView items={items} slotId="s1" />)
    const [g] = groups(container)
    expect(groupHeader(g).textContent).toBe(
      summary(
        t('chat.toolGroupRead', { count: '1' }),
        t('chat.toolGroupCommand', { count: '1' }),
        t('chat.toolGroupDeclined', { count: '1' }),
      ),
    )
    fireEvent.click(groupHeader(g))
    expect(screen.getByText(t('chat.toolDeclined'))).toBeTruthy()
    expect(screen.queryByText(t('chat.toolErrorBadge'))).toBeNull()
  })

  it('the reason line is not a row — rail row · dot count equals the same list without a marker (ADR-0051)', () => {
    const list = (mark: ToolItem['resultMark']): StructuredItem[] => [
      textItem('before'),
      toolItem('commandExecution', 'c1', 'Command', { resultMark: mark }),
      textItem('after'),
    ]
    const plain = render(<StructuredTextView items={list(null)} />)
    const plainCounts = [railRows(plain.container), railDots(plain.container)]
    cleanup()
    const marked = render(<StructuredTextView items={list('refused')} />)
    expect(marked.container.querySelector('[data-tool-declined-reason]')).toBeTruthy()
    expect([railRows(marked.container), railDots(marked.container)]).toEqual(plainCounts)
  })

  // ★TRD §11 ⑪ · §4-7 ⑨ ⓐ pinned★: a tool failure in an interrupted turn shows as it is as a red 「error」, and the emphasized 「interrupted」
  //   row right below gives the context — failures inside an interrupted turn are not separated out.
  it('ⓐ claude (no synthetic line case) — tool_result is_error of an interrupted turn\'s tool → red 「Error」 · head 「error 1」 · interrupted row below', () => {
    useToolGroupStore.getState().bind('s1', 'agent-1')
    const items: StructuredItem[] = [
      toolItem('Read', 'r1', 'Read'),
      resultCarrier('r1', false),
      toolItem('Bash', 'b1', 'Command'),
      resultCarrier('b1', true),
      interruptedOutcome(),
      { kind: 'separator', itemId: nextToolItemId++ },
    ]
    const { container } = render(<StructuredTextView items={items} slotId="s1" />)
    const [g] = groups(container)
    expect(groupHeader(g).textContent).toBe(
      summary(
        t('chat.toolGroupRead', { count: '1' }),
        t('chat.toolGroupCommand', { count: '1' }),
        t('chat.toolGroupErrors', { count: '1' }),
      ),
    )
    fireEvent.click(groupHeader(g))
    expect(screen.getByText(t('chat.toolErrorBadge')).className).toContain('text-red-500')
    expect(container.querySelector('[data-tool-mark]')).toBeNull()
    const outcome = screen.getByText(t('chat.turnInterrupted'))
    expect(g.compareDocumentPosition(outcome) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
  })

  // ADR-0243: the real shape of claude — a synthetic interruption line comes after the dead tool's is_error result, and there is no outcome row.
  it('ⓐ claude — interrupted tool is_error → red 「Error」 · head 「error 1」 · interruption display row below · no interrupted-outcome row', () => {
    const acc = new StructuredEventAccumulator()
    const feed = (ev: unknown) => acc.feed(new TextEncoder().encode(JSON.stringify(ev as StructuredEvent)))
    const call = (name: string, id: string, category: ToolCategory) => ({
      type: 'ToolCall',
      name,
      args_json: '{}',
      id,
      turn_id: null,
      message_id: null,
      category,
    })
    const result = (id: string, isError: boolean) => ({
      type: 'Structured',
      kind: 'user',
      json: JSON.stringify({ type: 'tool_result', tool_use_id: id, content: 'out', is_error: isError }),
    })
    feed(call('Read', 'r1', 'Read'))
    feed(result('r1', false))
    feed(call('Bash', 'b1', 'Command'))
    feed(result('b1', true))
    feed({
      type: 'Structured',
      kind: 'interrupted',
      json: JSON.stringify({ text: '[Request interrupted by user for tool use]' }),
    })
    feed({ type: 'TurnEnd', turn_id: null, outcome: { kind: 'Interrupted' } })

    useToolGroupStore.getState().bind('s1', 'agent-1')
    const { container } = render(<StructuredTextView items={acc.snapshot()} slotId="s1" />)
    const [g] = groups(container)
    expect(groupHeader(g).textContent).toBe(
      summary(
        t('chat.toolGroupRead', { count: '1' }),
        t('chat.toolGroupCommand', { count: '1' }),
        t('chat.toolGroupErrors', { count: '1' }),
      ),
    )
    fireEvent.click(groupHeader(g))
    expect(screen.getByText(t('chat.toolErrorBadge')).className).toContain('text-red-500')
    const note = container.querySelector('[data-interrupt-note]') as HTMLElement
    expect(note.textContent).toBe('[Request interrupted by user for tool use]')
    expect(g.compareDocumentPosition(note) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    expect(screen.queryByText(t('chat.turnInterrupted'))).toBeNull()
  })

  it('ⓐ codex — a ToolResult{Failed} arriving late after an interrupted turn → that row 「Error」 · head 「error 1」 · interrupted row as is', () => {
    const acc = new StructuredEventAccumulator()
    const feed = (ev: unknown) => acc.feed(new TextEncoder().encode(JSON.stringify(ev as StructuredEvent)))
    const call = (id: string, category: ToolCategory) => ({
      type: 'ToolCall',
      name: 'commandExecution',
      args_json: '{}',
      id,
      turn_id: 't1',
      message_id: null,
      category,
    })
    feed(call('c1', 'Read'))
    feed(call('c2', 'Command'))
    feed({ type: 'TurnEnd', turn_id: 't1', outcome: { kind: 'Interrupted' } })
    feed({ type: 'ToolResult', id: 'c2', outcome: 'Failed' })

    useToolGroupStore.getState().bind('s1', 'agent-1')
    const { container } = render(<StructuredTextView items={acc.snapshot()} slotId="s1" />)
    const [g] = groups(container)
    expect(groupHeader(g).textContent).toBe(
      summary(
        t('chat.toolGroupRead', { count: '1' }),
        t('chat.toolGroupCommand', { count: '1' }),
        t('chat.toolGroupErrors', { count: '1' }),
      ),
    )
    fireEvent.click(groupHeader(g))
    const marks = Array.from(container.querySelectorAll('[data-tool-mark]')).map((el) => el.getAttribute('data-tool-mark'))
    expect(marks).toEqual(['failed'])
    expect(screen.getByText(t('chat.toolErrorBadge')).className).toContain('text-red-500')
    const outcome = screen.getByText(t('chat.turnInterrupted'))
    expect(g.compareDocumentPosition(outcome) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
  })

  it('if a kind exists the kind icon · if Other the name heuristic (the old daemon look as is)', () => {
    const { container, rerender } = render(<StructuredTextView items={[toolItem('Glob', 'g1', 'Search')]} />)
    const iconOf = () => container.querySelector('.mb-3 svg')?.getAttribute('class') ?? ''
    expect(iconOf()).toContain('lucide-search')
    rerender(<StructuredTextView items={[toolItem('Glob', 'g1', 'Other')]} />)
    expect(iconOf()).toContain('lucide-folder-open')
    rerender(<StructuredTextView items={[toolItem('Task', 'a1', 'Agent')]} />)
    expect(iconOf()).toContain('lucide-bot')
    rerender(<StructuredTextView items={[toolItem('mcp__x__y', 'm1', 'Mcp')]} />)
    expect(iconOf()).toContain('lucide-plug')
  })

  it('even an out-of-type kind word (an item put in by a cast) does not break rendering and goes to the name heuristic', () => {
    const odd = toolItem('Glob', 'g1', 'Weird' as unknown as ToolCategory)
    const { container } = render(<StructuredTextView items={[odd]} />)
    expect(container.querySelector('.mb-3 svg')?.getAttribute('class')).toContain('lucide-folder-open')
  })
})

// ── ADR-0237 U8: interrupt row emphasis ────────────────────────────────────────────────────────────
describe('StructuredTextView outcome row tone (ADR-0237 U8)', () => {
  const titleOf = (outcome: 'failed' | 'interrupted' | 'unknown', text: string) => {
    render(<StructuredTextView items={[{ kind: 'outcome', outcome, detail: null, itemId: 0 }]} />)
    const title = screen.getByText(text)
    return { title, line: title.parentElement as HTMLElement }
  }

  it('interrupted = bold + accent color (text-accent) · icon · text as is', () => {
    const { title, line } = titleOf('interrupted', t('chat.turnInterrupted'))
    expect(title.className).toContain('font-bold')
    expect(line.className).toContain('text-accent')
    expect(line.className).not.toContain('text-muted')
    expect(line.querySelector('svg')?.getAttribute('class')).toContain('lucide-circle-stop')
  })

  it('failed = red tone + bold as today', () => {
    const { title, line } = titleOf('failed', t('chat.turnFailed'))
    expect(title.className).toContain('font-bold')
    expect(line.className).toContain('text-red-500')
    expect(line.className).not.toContain('text-accent')
  })

  it('unknown = muted as today · not bold', () => {
    const { title, line } = titleOf('unknown', t('chat.turnUnknown'))
    expect(title.className).not.toContain('font-bold')
    expect(line.className).toContain('text-muted')
    expect(line.className).not.toContain('text-accent')
  })
})

// ── ADR-0243: claude interruption display row ───────────────────────────────────────────────────────
describe('StructuredTextView interruption display row (ADR-0243)', () => {
  const noteItem = (text: string): StructuredItem => ({ kind: 'interruptNote', text, itemId: nextToolItemId++ })

  it('the original text verbatim — markdown characters also do not become tags', () => {
    const text = '[Request interrupted by user] **bold** `code` # h'
    const { container } = render(<StructuredTextView items={[noteItem(text)]} />)
    const note = container.querySelector('[data-interrupt-note]') as HTMLElement
    expect(note.textContent).toBe(text)
    expect(note.querySelector('strong, code, h1')).toBeNull()
  })

  it('same shape as the U8 interrupted row — accent color · bold · CircleStop', () => {
    const { container } = render(<StructuredTextView items={[noteItem('[Request interrupted by user]')]} />)
    const note = container.querySelector('[data-interrupt-note]') as HTMLElement
    expect(note.className).toContain('text-accent')
    expect(screen.getByText('[Request interrupted by user]').className).toContain('font-bold')
    expect(note.querySelector('svg')?.getAttribute('class')).toContain('lucide-circle-stop')
  })

  it('the same line · text classes as the interrupted branch of the outcome row (the U8 shape does not diverge across the two rows)', () => {
    const { container } = render(
      <StructuredTextView
        items={[
          noteItem('[Request interrupted by user]'),
          { kind: 'outcome', outcome: 'interrupted', detail: null, itemId: nextToolItemId++ },
        ]}
      />,
    )
    const note = container.querySelector('[data-interrupt-note]') as HTMLElement
    const outcomeLine = screen.getByText(t('chat.turnInterrupted')).parentElement as HTMLElement
    expect(note.className).toBe(outcomeLine.className)
    expect(note.querySelector('span')?.className).toBe(outcomeLine.querySelector('span')?.className)
  })

  it('one rail row and it breaks the tool group', () => {
    const items: StructuredItem[] = [
      toolItem('Read', 'r1', 'Read'),
      toolItem('Read', 'r2', 'Read'),
      noteItem('[Request interrupted by user]'),
      toolItem('Read', 'r3', 'Read'),
      toolItem('Read', 'r4', 'Read'),
    ]
    const { container } = render(<StructuredTextView items={items} />)
    expect(groups(container)).toHaveLength(2)
    // bundle · display row · bundle = three rows.
    expect(railRows(container)).toBe(3)
    expect(railDots(container)).toBe(3)
  })
})

// ADR-0373: the dropped-output placeholder is a marker — shown in the UI language, re-rendered on a switch; other errors as they came.
describe('error row — the dropped-output placeholder', () => {
  afterEach(() => {
    act(() => {
      setLanguage('ko')
    })
  })

  // ADR-0383: only the current marker is recognised — the pre-S38 Korean one went with the PROTOCOL_VERSION 9 bump (a daemon that
  // sends it is refused). The `en` assertions tell "marker recognised" from "raw text shown".
  it('shows chat.droppedOutput instead of the marker, and follows a language switch', () => {
    const marker = PLACEHOLDER_ERROR_MESSAGE
    const items: StructuredItem[] = [{ kind: 'error', message: marker, itemId: 0 }]
    const { rerender } = render(<StructuredTextView items={items} />)
    expect(screen.getByText(t('chat.droppedOutput'))).toBeTruthy()
    expect(screen.getByText('오류')).toBeTruthy()

    act(() => {
      setLanguage('en')
    })
    rerender(<StructuredTextView items={items} />)

    expect(screen.getByText('This output could not be delivered.')).toBeTruthy()
    expect(screen.queryByText(marker)).toBeNull()
    expect(screen.getByText('Error')).toBeTruthy()
  })

  it('any other message is shown as it came, in either language', () => {
    act(() => {
      setLanguage('en')
    })
    render(<StructuredTextView items={[{ kind: 'error', message: '이 출력 사건을 싣지 못했습니다!', itemId: 0 }]} />)
    expect(screen.getByText('이 출력 사건을 싣지 못했습니다!')).toBeTruthy()
    cleanup()
    // The pre-S38 Korean marker is no longer special (ADR-0383, removed with PROTOCOL_VERSION 9): it shows as it came.
    render(<StructuredTextView items={[{ kind: 'error', message: '이 출력 사건을 싣지 못했습니다', itemId: 1 }]} />)
    expect(screen.getByText('이 출력 사건을 싣지 못했습니다')).toBeTruthy()
  })
})
