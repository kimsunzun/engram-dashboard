// UsageSlot screen (component share of the "frontend display" row of TRD S21 usage-limit-slot §4).
// Real `usageStore` with values planted by `setState` (the merge rule is measured in `usageStore.test.ts`) and the real `usageSlot.*` commands — measures that ⟳ · display toggle reach store and layout writes through the command.
// Both clocks are fake: `Date` by fake timer, `performance.now` by spy. jsdom has no layout, so widths come from a fake ResizeObserver (slot width · ⟳ area width + natural widths of two stages).

import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { UsageLimitSnapshot } from '../../../crates/engram-dashboard-protocol/bindings/UsageLimitSnapshot'
import type { UsageVendorState } from '../../../crates/engram-dashboard-protocol/bindings/UsageVendorState'
import type { AgentBackendKind } from '../../api/types'

const client = vi.hoisted(() => ({
  getUsageSnapshot: vi.fn(async () => ({ socketEpoch: 0, snapshots: [] })),
  refreshUsageLimits: vi.fn(async (_vendor: string) => undefined),
}))
vi.mock('../../api/clientFactory', () => ({ agentClient: client }))

const opener = vi.hoisted(() => ({ openUrl: vi.fn(async (_url: string) => undefined) }))
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: opener.openUrl }))

// The display toggle flows for real through command → viewStore → shell invoke, and is measured by the invoke argument at the end (same as `usageCommands.test`).
const invokeMock = vi.hoisted(() => vi.fn(async (_cmd: string, _args?: unknown) => undefined as unknown))
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => vi.fn()) }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ close: vi.fn(async () => undefined), label: () => 'main' }),
}))

// Why `?raw` = same as `index.css.test.ts` (there is no @types/node, so using fs would break the tsc gate).
import usageLinksSource from '../../../src-tauri/capabilities/usage-links.json?raw'
import '../../commands/usageCommands' // side-effect: usageSlot.* registration
import { useUsageStore } from '../../store/usageStore'
import UsageSlot, { USAGE_PAGE_URL } from './UsageSlot'
import { ANCHOR_GAP } from './SlotContextMenu'
import { formatAge, formatClock, formatResetAt, STALE_AFTER_SECS, statusLine, statusSentence } from './usageFormat'

// Reads the theme.css source with Node fs — the reason and declaration shape are the same as `store/chatStyleStore.test.ts`.
declare function require(id: string): { readFileSync(p: string, enc: string): string }
declare const process: { cwd(): string }

const NOW = 1_900_000_000 // Wall clock (seconds)
let perfNow = 50_000 // performance.now()(ms)

class FakeResizeObserver {
  static instances: FakeResizeObserver[] = []
  observed: Element[] = []
  disconnected = false
  private readonly cb: ResizeObserverCallback
  constructor(cb: ResizeObserverCallback) {
    this.cb = cb
    FakeResizeObserver.instances.push(this)
  }
  observe(el: Element) {
    this.observed.push(el)
  }
  unobserve() {}
  disconnect() {
    this.disconnected = true
    this.observed = []
  }
  fire(entries: Array<[Element, number, number?]>) {
    this.cb(
      entries.map(
        ([target, width, height = 10]) => ({ target, contentRect: { width, height } }) as unknown as ResizeObserverEntry,
      ),
      this as unknown as ResizeObserver,
    )
  }
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval', 'Date'] })
  vi.setSystemTime(NOW * 1000)
  perfNow = 50_000
  vi.spyOn(performance, 'now').mockImplementation(() => perfNow)
  FakeResizeObserver.instances = []
  globalThis.ResizeObserver = FakeResizeObserver as unknown as typeof ResizeObserver
  useUsageStore.setState({ vendors: {}, socketEpoch: 0, pending: {} })
  invokeMock.mockClear()
  client.getUsageSnapshot.mockClear()
  client.refreshUsageLimits.mockClear()
  opener.openUrl.mockClear()
})

afterEach(() => {
  cleanup()
  vi.useRealTimers()
  vi.restoreAllMocks()
  delete (globalThis as { ResizeObserver?: unknown }).ResizeObserver
})

function snap(vendor: AgentBackendKind, over: Partial<UsageLimitSnapshot> = {}): UsageLimitSnapshot {
  return {
    vendor,
    account_key: 'default',
    five_hour: { used_pct: 38, resets_at: NOW + 7_980, age_secs: 0, expired: false },
    weekly: { used_pct: 59, resets_at: NOW + 86_400 * 3, age_secs: 0, expired: false },
    model_scoped: [],
    plan: null,
    in_flight: false,
    state: { kind: 'Ready' },
    revision: 1,
    ...over,
  }
}

function seed(snapshot: UsageLimitSnapshot, receivedAt = perfNow): void {
  useUsageStore.setState(s => ({
    vendors: { ...s.vendors, [snapshot.vendor]: { snapshot, receivedAt, revision: snapshot.revision } },
  }))
}

function usage(showClaude: boolean, showCodex: boolean) {
  return { type: 'usage' as const, show_claude: showClaude, show_codex: showCodex }
}

function mount(showClaude = true, showCodex = true) {
  return render(<UsageSlot content={usage(showClaude, showCodex)} viewId="v1" slotId="s1" />)
}

function q(selector: string, root: ParentNode = document): HTMLElement | null {
  return root.querySelector<HTMLElement>(selector)
}

function summary(): HTMLElement {
  return q('[data-usage-summary]')!
}

/** The ⟳ of the small display (outside the summary button). */
function refreshButton(): HTMLButtonElement | null {
  return q('[data-usage-slot] [data-usage-refresh]') as HTMLButtonElement | null
}

function valueOf(vendor: string, window: string, root: ParentNode = summary()): HTMLElement | null {
  return q(`[data-usage-vendor="${vendor}"][data-usage-window="${window}"] [data-usage-value]`, root)
}

/** The width of the ⟳ area (⟳ + right margin) plus the left/right margins of the summary button — a fake size matched to real values. */
const REFRESH_AREA_W = 26
const SUMMARY_PAD_X_TOTAL = 12

function liveObserver(): FakeResizeObserver {
  const live = FakeResizeObserver.instances.filter(o => !o.disconnected)
  expect(live).toHaveLength(1)
  return live[0]
}

/** Gives the observer the size it receives as is — slot root · ⟳ area · stage 1·2 measurement copies. */
function fireSizes(s: { rootW: number; rootH: number; refreshW: number; s1: number; s2: number; s2h: number }): void {
  act(() =>
    liveObserver().fire([
      [q('[data-usage-slot]')!, s.rootW, s.rootH],
      [q('[data-usage-refresh-area]')!, s.refreshW, s.refreshW === 0 ? 0 : 22],
      [q('[data-usage-measure="1"]')!, s.s1],
      [q('[data-usage-measure="2"]')!, s.s2, s.s2h],
    ]),
  )
}

/**
 * Gives the width the content will use (`avail`) and the natural widths of stages 1·2 — the slot width is built backwards as `avail` + ⟳ area + summary button margins.
 * Height is generous by default (slot 1000 · stage 2 30) — stages do not look at height (ADR-0259) but the observer carries height too.
 */
function setWidths(avail: number, s1: number, s2: number, { rootH = 1000, s2h = 30 } = {}): void {
  const refreshW = refreshButton() ? REFRESH_AREA_W : 0
  fireSizes({ rootW: avail + refreshW + SUMMARY_PAD_X_TOTAL, rootH, refreshW, s1, s2, s2h })
}

/**
 * The visible text of the reset time — an expected value built apart from the formatter (the local getters of `Date`). If it is the same local date as now (`NOW`) 「HH:MM」,
 * otherwise 「M/D HH:MM」. Depending on the time zone NOW + a few hours can cross midnight, so the date is always compared directly.
 */
function resetText(epochSecs: number): string {
  const d = new Date(epochSecs * 1000)
  const now = new Date(NOW * 1000)
  const time = `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
  const sameDay =
    d.getFullYear() === now.getFullYear() && d.getMonth() === now.getMonth() && d.getDate() === now.getDate()
  return sameDay ? time : `${d.getMonth() + 1}/${d.getDate()} ${time}`
}

/** The name of the reset time (tooltip · assistive tech) — in the visible spot an hourglass stands instead of a word (ADR-0259). */
function resetName(epochSecs: number): string {
  return `리셋 ${resetText(epochSecs)}`
}

/** The reset spot of the popup window row — the hourglass marker (tooltip = name) and the remaining time after it. */
function popupReset(row: ParentNode): { mark: HTMLElement; text: string; name: string | null; after: string } {
  const at = q('[data-usage-reset-at]', row)!
  const mark = q('[role="img"]', at)!
  return {
    mark,
    text: mark.textContent!,
    name: mark.getAttribute('title'),
    after: q('[data-usage-reset-in]', at)!.textContent!,
  }
}

/** The company head of the popup — the first line of the company section. */
function popupHead(popup: ParentNode, vendor: string): HTMLElement {
  return q(`[data-usage-popup-vendor="${vendor}"]`, popup)!.firstElementChild as HTMLElement
}

function vendorRefresh(popup: ParentNode, vendor: string): HTMLButtonElement {
  return q(`[data-usage-vendor-refresh="${vendor}"]`, popup) as HTMLButtonElement
}

/** Grid cell list — whitespace inside `minmax(…, …)` does not separate cells. */
function tracks(template: string): string[] {
  return template.match(/minmax\([^)]*\)|\S+/g) ?? []
}

function stage(): string | null {
  return q('[data-usage-content]')!.getAttribute('data-usage-stage')
}

const STAGE_WIDTHS: Record<1 | 2 | 3, number> = { 1: 500, 2: 300, 3: 50 }
function toStage(n: 1 | 2 | 3): void {
  setWidths(STAGE_WIDTHS[n], 400, 200)
  expect(stage()).toBe(String(n))
}

function openPopup(): HTMLElement {
  fireEvent.click(summary())
  return screen.getByRole('dialog')
}

function advance(ms: number): void {
  act(() => {
    perfNow += ms
    vi.advanceTimersByTime(ms)
  })
}

const FIVE_STATES: Array<[string, UsageVendorState]> = [
  ['NotInstalled', { kind: 'NotInstalled', detail: null }],
  ['NeedsLogin', { kind: 'NeedsLogin', detail: { kind: 'auth_error', code: null, upstream: null } }],
  ['Unavailable', { kind: 'Unavailable', detail: { kind: 'limits_unavailable', code: null, upstream: null } }],
  [
    'Failed',
    {
      kind: 'Failed',
      next_attempt_in_secs: 300,
      detail: { kind: 'rate_limits_null', code: null, upstream: 'rate_limits_available: true, rate_limits: null' },
    },
  ],
  ['Rejected', { kind: 'Rejected', retry_in_secs: 720, detail: { kind: 'rate_limited', code: 429, upstream: null } }],
]

// ── Mount · enabled company ──
describe('mount and enabled companies', () => {
  it('on mount it pulls the shell cache once (TRD §3 #82)', () => {
    mount()
    expect(client.getUsageSnapshot).toHaveBeenCalledTimes(1)
  })

  it('both companies off → one notice line (R4) · no ⟳ · pressing the notice opens a popup that has only the display toggle row', () => {
    seed(snap('claude'))
    mount(false, false)
    const hint = q('[data-usage-hint]')!
    expect(hint.textContent).toBe('클릭해서 표시할 항목 고르기')
    expect(summary().contains(hint)).toBe(true)
    expect(summary().textContent).toBe('클릭해서 표시할 항목 고르기')
    expect(q('[data-usage-vendor]')).toBeNull()
    expect(refreshButton()).toBeNull()
    const popup = openPopup()
    expect([...popup.children].map(c => c.hasAttribute('data-usage-show-toggles'))).toEqual([true])
    expect(q('[data-usage-popup-vendor]', popup)).toBeNull()
    expect(q('[data-usage-status-line]', popup)).toBeNull()
    expect(within(popup).getAllByRole('checkbox').map(c => (c as HTMLInputElement).checked)).toEqual([false, false])
  })

  it('a disabled company is absent from the small display, badge and popup — even if the store has values', () => {
    seed(snap('claude'))
    seed(snap('codex', { state: { kind: 'NeedsLogin', detail: null } }))
    mount(true, false)
    for (const n of [1, 2, 3] as const) {
      toStage(n)
      expect(q('[data-usage-vendor="codex"]')).toBeNull()
      expect(q('[data-usage-badge]')).toBeNull()
      expect(summary().textContent).not.toContain('Codex')
      expect(q('[data-usage-icon="codex"]', summary())).toBeNull()
    }
    const popup = openPopup()
    expect(q('[data-usage-popup-vendor="codex"]', popup)).toBeNull()
    expect(q('[data-usage-status-line]', popup)).toBeNull()
    expect(vendorRefresh(popup, 'codex')).toBeNull()
    expect(vendorRefresh(popup, 'claude')).not.toBeNull()
    // The only place where the name remains is the display toggle row (the place to turn it back on) — while off (ADR-0259 decision 7).
    const toggles = q('[data-usage-show-toggles]', popup)!
    for (const child of popup.children) {
      if (child !== toggles) expect(child.textContent).not.toContain('Codex')
    }
    expect((q('[data-usage-show="codex"]', toggles) as HTMLInputElement).checked).toBe(false)
  })

  it('a company that has not received a single value yet → gray "—" in both windows · no badge', () => {
    mount(true, false)
    expect(valueOf('claude', 'five_hour')!.textContent).toBe('—')
    expect(valueOf('claude', 'weekly')!.textContent).toBe('—')
    expect(q('[data-usage-badge]')).toBeNull()
    expect(q('[data-usage-bar]')).toBeNull()
    expect(screen.queryAllByRole('meter')).toHaveLength(0)
  })
})

// ── Value · color · bar ──
describe('value · color · bar', () => {
  it('remaining amount as DOM text (R27) — Claude usage 38 → 62%', () => {
    seed(snap('claude'))
    mount(true, false)
    expect(valueOf('claude', 'five_hour')!.textContent).toBe('62%')
    expect(valueOf('claude', 'weekly')!.textContent).toBe('41%')
  })

  it('the small display bar is hidden from assistive tech and the summary button carries the short name (button children are flattened)', () => {
    seed(snap('claude', { state: { kind: 'NeedsLogin', detail: null } }))
    seed(snap('codex', { weekly: null, in_flight: true }))
    mount()
    const bars = [...summary().querySelectorAll<HTMLElement>('[data-usage-bar]')]
    expect(bars).toHaveLength(3)
    for (const bar of bars) {
      expect(bar.getAttribute('aria-hidden')).toBe('true')
      expect(bar.hasAttribute('role')).toBe(false)
    }
    expect(within(summary()).queryAllByRole('meter', { hidden: true })).toHaveLength(0)
    expect(summary().getAttribute('aria-label')).toBe(
      'Claude 5시간 62% 남음, 주간 41% 남음 (로그인 필요) · Codex 5시간 62% 남음, 주간 값 없음 (갱신 중)',
    )
  })

  it('bar = meter (D14 — popup) — aria-valuenow is the visible number', () => {
    seed(snap('claude', { five_hour: { used_pct: 80.4, resets_at: NOW + 600, age_secs: 0, expired: false } }))
    mount(true, false)
    const meters = within(openPopup()).getAllByRole('meter')
    expect(meters).toHaveLength(2)
    expect(meters[0].getAttribute('aria-valuenow')).toBe('19')
    expect(meters[0].getAttribute('aria-valuemin')).toBe('0')
    expect(meters[0].getAttribute('aria-valuemax')).toBe('100')
    expect(meters[0].getAttribute('aria-valuetext')).toBe('19% 남음')
    expect(meters[0].getAttribute('aria-label')).toBe('Claude 5시간 남은 양')
  })

  it('color is tokens only — per-band fill and text color tokens, even under 20% only color with no symbol (R10 revision 2026-09-29)', () => {
    seed(
      snap('claude', {
        five_hour: { used_pct: 81, resets_at: NOW + 600, age_secs: 0, expired: false },
        weekly: { used_pct: 50, resets_at: NOW + 600, age_secs: 0, expired: false },
      }),
    )
    seed(snap('codex', { five_hour: { used_pct: 10, resets_at: NOW + 600, age_secs: 0, expired: false }, weekly: null }))
    mount()
    const danger = valueOf('claude', 'five_hour')!
    expect(danger.textContent).toBe('19%')
    expect(danger.style.color).toBe('var(--usage-danger)')
    const fills = [...summary().querySelectorAll<HTMLElement>('[data-usage-fill]')].map(el => el.style.background)
    expect(fills).toEqual(['var(--usage-danger-fill)', 'var(--usage-warn-fill)', 'var(--usage-ok-fill)'])
    expect(valueOf('claude', 'weekly')!.style.color).toBe('var(--usage-warn)')
    expect(valueOf('codex', 'five_hour')!.style.color).toBe('var(--usage-ok)')
    const none = valueOf('codex', 'weekly')!
    expect(none.textContent).toBe('—')
    expect(none.style.color).toBe('var(--usage-none)')
  })

  it('Unavailable is gray "—" even if a value is carried (a state that carries no value)', () => {
    seed(snap('claude', { state: { kind: 'Unavailable', detail: null } }))
    mount(true, false)
    expect(valueOf('claude', 'five_hour')!.textContent).toBe('—')
    expect(valueOf('claude', 'weekly')!.textContent).toBe('—')
    expect(within(summary()).queryAllByRole('meter')).toHaveLength(0)
  })

  it('even on failure or refusal it draws the value it holds (R22)', () => {
    seed(snap('claude', { state: { kind: 'Failed', next_attempt_in_secs: 60, detail: null } }))
    seed(snap('codex', { state: { kind: 'Rejected', retry_in_secs: 60, detail: null } }))
    mount()
    expect(valueOf('claude', 'five_hour')!.textContent).toBe('62%')
    expect(valueOf('codex', 'five_hour')!.textContent).toBe('62%')
  })
})

// ── Width stages (R3·D16) ──
describe('width stage', () => {
  it('widest stage that fits — one line per company → one line per window → numbers only', () => {
    seed(snap('claude'))
    mount(true, false)
    expect(stage()).toBe('1') // The most detailed stage before measuring
    setWidths(400, 400, 200)
    expect(stage()).toBe('1')
    setWidths(399, 400, 200)
    expect(stage()).toBe('2')
    setWidths(200, 400, 200)
    expect(stage()).toBe('2')
    setWidths(199, 400, 200)
    expect(stage()).toBe('3')
    // When the natural width shrinks (values·text change and the hidden render is measured again), back to the wide stage.
    setWidths(199, 150, 100)
    expect(stage()).toBe('1')
  })

  it('reset time only in stages 1·2 — hourglass + 「HH:MM」 (on another day 「M/D HH:MM」 · no weekday, remaining time or word) · name = 「reset …」, stage 3 numbers only (without 「%」)', () => {
    seed(snap('claude'))
    mount(true, false)
    const cell = (w: string) =>
      q(`[data-usage-vendor="claude"][data-usage-window="${w}"] [data-usage-reset-clock]`, summary())!
    for (const n of [1, 2] as const) {
      toStage(n)
      expect(summary().querySelectorAll('[data-usage-reset-clock]')).toHaveLength(2)
      expect(cell('five_hour').textContent).toBe(resetText(NOW + 7_980))
      expect(cell('weekly').textContent).toBe(resetText(NOW + 86_400 * 3))
      expect(cell('weekly').textContent).toMatch(/^\d{1,2}\/\d{1,2} \d{2}:\d{2}$/)
      expect(cell('weekly').textContent).not.toMatch(/[일월화수목금토]/)
      expect(cell('five_hour').getAttribute('title')).toBe(resetName(NOW + 7_980))
      expect(cell('weekly').getAttribute('title')).toBe(resetName(NOW + 86_400 * 3))
      // The word 「reset」 is only in the tooltip — it is absent from the DOM text (ADR-0259 decision 3).
      expect(summary().textContent).not.toMatch(/리셋|뒤|↻/)
    }
    toStage(3)
    expect(summary().querySelectorAll('[data-usage-reset-clock]')).toHaveLength(0)
    expect(summary().textContent).not.toContain('리셋')
    expect(q('[data-usage-numbers]')!.textContent).toBe('62·41')
    expect(within(summary()).queryAllByRole('meter')).toHaveLength(0)
  })

  it('narrower than stage 3 only the number side is ellipsized — no line wrap or scroll, name and badge do not shrink', () => {
    seed(snap('claude', { state: { kind: 'NeedsLogin', detail: null } }))
    mount(true, false)
    toStage(3)
    const numbers = q('[data-usage-numbers]')!
    expect(numbers.style.textOverflow).toBe('ellipsis')
    expect(numbers.style.whiteSpace).toBe('nowrap')
    expect(numbers.style.overflow).toBe('hidden')
    const nameUnit = q('[data-usage-badge]')!.parentElement!
    expect(nameUnit.style.flexShrink).toBe('0')
    expect(nameUnit.contains(numbers)).toBe(false)
    expect(q('[data-usage-icon="claude"]', nameUnit)).not.toBeNull()
    expect(nameUnit.textContent).toBe('!')
  })

  it('the hidden measuring copy carries no role or value marker (so cdp and assistive tech do not read twice)', () => {
    seed(snap('claude', { state: { kind: 'NeedsLogin', detail: null } }))
    mount(true, false)
    for (const n of ['1', '2']) {
      const m = q(`[data-usage-measure="${n}"]`)!
      expect(m.querySelector('[data-usage-vendor]')).toBeNull()
      expect(m.querySelector('[role]')).toBeNull()
      expect(m.querySelector('[aria-label]')).toBeNull()
      expect(m.querySelector('[title]')).toBeNull()
      expect(m.textContent).toContain('62%')
      // Same shape as the visible side — one grid · icon · reset time.
      expect((m.firstElementChild as HTMLElement).style.display).toBe('grid')
      expect(m.querySelectorAll('svg:not(.lucide)')).toHaveLength(1)
      expect(m.querySelectorAll('svg.lucide-refresh-cw')).toHaveLength(0)
      expect(m.querySelectorAll('svg.lucide-hourglass')).toHaveLength(2)
      expect(m.textContent).toContain(resetText(NOW + 7_980))
    }
    expect(q('[data-usage-measure="1"]')!.parentElement!.style.visibility).toBe('hidden')
  })
})

// ── Status badge (R31) ──
describe('status badge and sentence', () => {
  for (const [name, state] of FIVE_STATES) {
    it(`${name} — at every width stage a 「!」 badge next to the name · aria-label = title = the popup's top line`, () => {
      seed(snap('claude', { state }))
      mount(true, false)
      const expected = statusLine('Claude', statusSentence(state, 0, NOW)!)
      for (const n of [1, 2, 3] as const) {
        toStage(n)
        const badge = q('[data-usage-badge="claude"]')
        expect(badge).not.toBeNull()
        expect(badge!.textContent).toBe('!')
        expect(badge!.previousElementSibling).toBe(q('[data-usage-icon="claude"]', summary()))
        expect(badge!.getAttribute('aria-label')).toBe(expected)
        expect(badge!.getAttribute('title')).toBe(expected)
      }
      const popup = openPopup()
      const top = q('[data-usage-status-line]', popup)!
      expect(popup.firstElementChild!.contains(top)).toBe(true)
      expect(q('[data-usage-status-text]', top)!.textContent).toBe(q('[data-usage-badge="claude"]')!.getAttribute('aria-label'))
    })
  }

  it('Ready has no badge', () => {
    seed(snap('claude'))
    mount(true, false)
    expect(q('[data-usage-badge]')).toBeNull()
    expect(q('[data-usage-status-line]', openPopup())).toBeNull()
  })

  it('five state phrases (Korean · Unavailable does not assert a cause)', () => {
    const texts: string[] = []
    for (const [, state] of FIVE_STATES) {
      cleanup()
      useUsageStore.setState({ vendors: {} })
      seed(snap('claude', { state }))
      mount(true, false)
      texts.push(q('[data-usage-badge]')!.getAttribute('aria-label')!)
    }
    expect(texts).toEqual([
      'Claude 설치 안 됨',
      'Claude 로그인 필요 (auth_error)',
      'Claude 이 계정의 한도 정보를 받을 수 없음 (limits_unavailable)',
      `Claude 조회 실패 — 응답: rate_limits_available: true, rate_limits: null · 다음 시도 ${formatClock(NOW + 300)}`,
      'Claude 거절됨 — 12분 뒤 (rate_limited · 429)',
    ])
  })

  it('upstream raw text as is without translation or sanitizing · code in parentheses after the source · company name once at the line head', () => {
    const upstream = '{kind} 원문 $& <b>'
    seed(
      snap('codex', {
        state: { kind: 'Failed', next_attempt_in_secs: 60, detail: { kind: 'rpc_error', code: -32001, upstream } },
      }),
    )
    mount(false, true)
    expect(q('[data-usage-badge="codex"]')!.getAttribute('aria-label')).toBe(
      `Codex 조회 실패 — 응답 (-32001): ${upstream} · 다음 시도 ${formatClock(NOW + 60)}`,
    )
  })
})

// ── Stale · expired · refreshing (R32) ──
describe('stale · expired · refreshing', () => {
  it('a value older than 30 minutes → the small display dims only the number (no age text) · the popup does not dim and shows the company head age in amber', () => {
    seed(
      snap('claude', {
        five_hour: { used_pct: 38, resets_at: NOW + 7_980, age_secs: 1_801, expired: false },
        weekly: { used_pct: 59, resets_at: NOW + 7_980, age_secs: 1_800, expired: false },
      }),
    )
    mount(true, false)
    const stale = valueOf('claude', 'five_hour')!
    expect(stale.hasAttribute('data-usage-stale')).toBe(true)
    expect(stale.style.opacity).toBe('0.5')
    expect(stale.getAttribute('title')).toBe('30분 넘게 새로 들어오지 않은 값')
    const fresh = valueOf('claude', 'weekly')!
    expect(fresh.hasAttribute('data-usage-stale')).toBe(false)
    expect(fresh.style.opacity).toBe('')
    // Only the number is dimmed — the bar and name stay as they are.
    expect(q('[data-usage-vendor="claude"][data-usage-window="five_hour"] [data-usage-bar]')!.style.opacity).toBe('')
    expect(summary().textContent).not.toMatch(/분 전|방금/)

    const popup = openPopup()
    expect(q('[data-usage-value][data-usage-stale]', popup)).toBeNull()
    expect(q('[data-usage-window] [data-usage-age]', popup)).toBeNull()
    expect(popup.querySelectorAll('[data-usage-age]')).toHaveLength(1)
    // The head's age = the older of the two windows (1801 seconds) — past 30 minutes it is amber (ADR-0259 decision 4).
    const age = q('[data-usage-age]', popupHead(popup, 'claude'))!
    expect(age.textContent).toBe('30분 전')
    expect(age.hasAttribute('data-usage-stale')).toBe(true)
    expect(age.style.color).toBe('var(--usage-stale)')
    expect(age.getAttribute('title')).toBe('30분 넘게 새로 들어오지 않은 값')
  })

  it('after the reset time passes (or daemon latch) hides % and remaining time and shows 「reset — waiting for refresh」', () => {
    seed(
      snap('claude', {
        five_hour: { used_pct: 38, resets_at: NOW, age_secs: 0, expired: false },
        weekly: { used_pct: 59, resets_at: NOW + 999, age_secs: 0, expired: true },
      }),
    )
    mount(true, false)
    for (const n of [1, 2] as const) {
      toStage(n)
      for (const w of ['five_hour', 'weekly']) {
        const seg = q(`[data-usage-vendor="claude"][data-usage-window="${w}"]`, summary())!
        expect(q('[data-usage-value]', seg)!.textContent).toBe('리셋됨 — 갱신 대기')
        expect(seg.textContent).not.toMatch(/%/)
        expect(q('[data-usage-reset-clock]', seg)).toBeNull()
        expect(q('[data-usage-bar]', seg)).toBeNull()
      }
    }
    toStage(3)
    expect(q('[data-usage-numbers]')!.textContent).toBe('리셋됨·리셋됨')
  })

  it('when the local wall clock passes the reset time it becomes expired without a request', () => {
    seed(snap('claude', { five_hour: { used_pct: 38, resets_at: NOW + 90, age_secs: 0, expired: false } }))
    mount(true, false)
    expect(valueOf('claude', 'five_hour')!.textContent).toBe('62%')
    advance(120_000)
    expect(valueOf('claude', 'five_hour')!.textContent).toBe('리셋됨 — 갱신 대기')
    expect(client.getUsageSnapshot).toHaveBeenCalledTimes(1)
  })

  it('querying = ⟳ waiting or snapshot in_flight — the ⟳ marker carries the company, and the value stays as is (ADR-0261)', () => {
    seed(snap('claude', { in_flight: true }))
    seed(snap('codex'))
    mount()
    expect(refreshButton()!.getAttribute('data-usage-refreshing')).toBe('claude')
    expect(valueOf('claude', 'five_hour')!.textContent).toBe('62%')
    act(() => useUsageStore.setState({ pending: { codex: 1 } }))
    expect(refreshButton()!.getAttribute('data-usage-refreshing')).toBe('claude codex')
    expect(valueOf('codex', 'five_hour')!.textContent).toBe('62%')
    const popup = openPopup()
    expect(vendorRefresh(popup, 'codex').getAttribute('data-usage-refreshing')).toBe('codex')
    expect(q('[data-usage-refreshing-text]', popup)).toBeNull()
    act(() => {
      useUsageStore.setState({ pending: {} })
      seed(snap('claude', { revision: 2 }))
    })
    expect(refreshButton()!.hasAttribute('data-usage-refreshing')).toBe(false)
    expect(vendorRefresh(popup, 'codex').hasAttribute('data-usage-refreshing')).toBe(false)
  })

  it('age · remaining time · next attempt flow locally by one minute tick (no request)', () => {
    seed(
      snap('claude', {
        weekly: null,
        state: { kind: 'Failed', next_attempt_in_secs: 600, detail: null },
      }),
    )
    mount(true, false)
    const popup = openPopup()
    const reset = () => popupReset(q('[data-usage-window="five_hour"]', popup)!)
    expect(reset().text).toBe(formatResetAt(NOW + 7_980, NOW))
    expect(reset().after).toBe('(2시간 13분 뒤)')
    expect(q('[data-usage-age]', popup)!.textContent).toBe('방금')
    const before = q('[data-usage-badge]')!.getAttribute('aria-label')
    expect(before).toBe(`Claude 조회 실패 · 다음 시도 ${formatClock(NOW + 600)}`)

    advance(60_000)
    expect(reset().after).toBe('(2시간 12분 뒤)')
    expect(q('[data-usage-age]', popup)!.textContent).toBe(formatAge(60))
    // The next attempt time itself points to the same instant (relative seconds from the receipt moment − elapsed seconds + now).
    expect(q('[data-usage-badge]')!.getAttribute('aria-label')).toBe(before)
    expect(client.getUsageSnapshot).toHaveBeenCalledTimes(1)
    expect(client.refreshUsageLimits).not.toHaveBeenCalled()
  })

  it('refusal wait 「in N min」 also gives the time that elapsed since receipt', () => {
    seed(snap('claude', { state: { kind: 'Rejected', retry_in_secs: 720, detail: null } }))
    mount(true, false)
    expect(q('[data-usage-badge]')!.getAttribute('aria-label')).toBe('Claude 거절됨 — 12분 뒤')
    advance(5 * 60_000)
    expect(q('[data-usage-badge]')!.getAttribute('aria-label')).toBe('Claude 거절됨 — 7분 뒤')
  })
})

// ── Popup (R5–R7·D8·D14) ──
describe('popup', () => {
  it('the summary area is a native button — opens by click (Enter·Space are called by the browser as the same click) and moves focus', () => {
    seed(snap('claude'))
    mount(true, false)
    const button = summary()
    expect(button.tagName).toBe('BUTTON')
    expect(button.getAttribute('type')).toBe('button')
    expect(button.getAttribute('aria-expanded')).toBe('false')
    const popup = openPopup()
    expect(button.getAttribute('aria-expanded')).toBe('true')
    expect(popup.style.position).toBe('fixed')
    expect(document.activeElement).toBe(popup)
  })

  it('closes with Esc and returns focus to the summary button', () => {
    seed(snap('claude'))
    mount(true, false)
    openPopup()
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(document.activeElement).toBe(summary())
  })

  it('closes on an outside press · a press or click inside the popup does not close · pressing the summary button again closes', () => {
    seed(snap('claude'))
    mount(true, false)
    const popup = openPopup()
    fireEvent.mouseDown(popup)
    fireEvent.click(popup)
    expect(screen.queryByRole('dialog')).not.toBeNull()
    fireEvent.mouseDown(document.body)
    expect(screen.queryByRole('dialog')).toBeNull()
    openPopup()
    fireEvent.mouseDown(summary())
    fireEvent.click(summary())
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  it('absolute reset time per window · age once at the company head (not on window lines) · plan only when present · per-model windows only those with a value', () => {
    seed(
      snap('claude', {
        plan: 'max_20x',
        five_hour: { used_pct: 38, resets_at: NOW + 7_980, age_secs: 125, expired: false },
        model_scoped: [
          { label: 'Opus', window: { used_pct: 70, resets_at: NOW + 86_400 * 2, age_secs: 0, expired: false } },
          { label: 'Sonnet', window: { used_pct: null, resets_at: NOW + 86_400 * 2, age_secs: 0, expired: false } },
        ],
      }),
    )
    seed(snap('codex'))
    mount()
    const popup = openPopup()
    const claude = q('[data-usage-popup-vendor="claude"]', popup)!
    const five = q('[data-usage-window="five_hour"]', claude)!
    expect(q('[data-usage-value]', five)!.textContent).toBe('62%')
    expect(popupReset(five).text).toBe(formatResetAt(NOW + 7_980, NOW))
    expect(popupReset(five).after).toBe('(2시간 13분 뒤)')
    expect(popupReset(q('[data-usage-window="weekly"]', claude)!).text).toBe(formatResetAt(NOW + 86_400 * 3, NOW))
    expect(q('[data-usage-window] [data-usage-age]', claude)).toBeNull()
    expect(claude.querySelectorAll('[data-usage-age]')).toHaveLength(1)
    expect(q('[data-usage-age]', popupHead(popup, 'claude'))!.textContent).toBe('2분 전')
    expect(q('[data-usage-plan]', claude)!.textContent).toBe('플랜: max_20x')
    expect(q('[data-usage-window="model:Opus"] [data-usage-value]', claude)!.textContent).toBe('30%')
    expect(q('[data-usage-window="model:Sonnet"]', claude)).toBeNull()
    expect(q('[data-usage-plan]', q('[data-usage-popup-vendor="codex"]', popup)!)).toBeNull()
  })

  it('a reset time on another day is 「M/D HH:MM」 (no weekday) — same rule as the small display', () => {
    seed(snap('claude'))
    mount(true, false)
    const popup = openPopup()
    const weekly = popupReset(q('[data-usage-window="weekly"]', popup)!)
    expect(weekly.text).toBe(resetText(NOW + 86_400 * 3))
    expect(weekly.text).toMatch(/^\d{1,2}\/\d{1,2} \d{2}:\d{2}$/)
    expect(weekly.name).toBe(resetName(NOW + 86_400 * 3))
    expect(weekly.after).toBe('(3일 뒤)')
  })

  it('「usage page ↗」 → openUrl with the exact address · title states the destination', () => {
    seed(snap('claude'))
    seed(snap('codex'))
    mount()
    const popup = openPopup()
    const claudeLink = q('[data-usage-link="claude"]', popup)!
    const codexLink = q('[data-usage-link="codex"]', popup)!
    expect(claudeLink.textContent).toBe('사용량 페이지 ↗')
    expect(claudeLink.getAttribute('title')).toContain('https://claude.ai/settings/usage')
    fireEvent.click(claudeLink)
    fireEvent.click(codexLink)
    expect(opener.openUrl.mock.calls.map(c => c[0])).toEqual([
      'https://claude.ai/settings/usage',
      'https://chatgpt.com/codex/settings/usage',
    ])
  })

  it('one ⟳ per company in the popup — name 「<company> refresh usage」 · the small display\'s global ⟳ stays outside the popup (ADR-0259 decision 2)', () => {
    seed(snap('claude'))
    seed(snap('codex'))
    mount()
    const popup = openPopup()
    expect(q('[data-usage-refresh]', popup)).toBeNull()
    expect(within(popup).getAllByRole('button', { name: /새로고침/ })).toEqual([
      vendorRefresh(popup, 'claude'),
      vendorRefresh(popup, 'codex'),
    ])
    expect(within(popup).getByRole('button', { name: 'Claude 사용량 새로고침' })).toBe(vendorRefresh(popup, 'claude'))
    expect(within(popup).getByRole('button', { name: 'Codex 사용량 새로고침' })).toBe(vendorRefresh(popup, 'codex'))
    for (const vendor of ['claude', 'codex']) {
      const button = vendorRefresh(popup, vendor)
      expect(popupHead(popup, vendor).contains(button)).toBe(true)
      expect(button.getAttribute('type')).toBe('button')
      expect(q('svg.lucide-refresh-cw', button)).not.toBeNull()
    }
    expect(refreshButton()).not.toBeNull()
    expect(popup.contains(refreshButton())).toBe(false)
  })

  it('visible refusal → popup top 「refused — in N min」', () => {
    seed(snap('claude', { state: { kind: 'Rejected', retry_in_secs: 600, detail: null } }))
    seed(snap('codex'))
    mount()
    const popup = openPopup()
    expect(q('[data-usage-status-line="claude"] [data-usage-status-text]', popup)!.textContent).toBe(
      'Claude 거절됨 — 10분 뒤',
    )
  })

  it('details only for enabled companies — no section or status line for a disabled company', () => {
    seed(snap('claude'))
    seed(snap('codex', { state: { kind: 'NeedsLogin', detail: null } }))
    mount(true, false)
    const popup = openPopup()
    expect([...popup.querySelectorAll('[data-usage-popup-vendor]')].map(e => e.getAttribute('data-usage-popup-vendor'))).toEqual([
      'claude',
    ])
    expect(q('[data-usage-status-line]', popup)).toBeNull()
  })

  it('both companies abnormal → one line per company at the popup top, each line = that company badge sentence', () => {
    seed(snap('claude', { state: { kind: 'NeedsLogin', detail: null } }))
    seed(snap('codex', { state: { kind: 'NotInstalled', detail: null } }))
    mount()
    const popup = openPopup()
    const lines = popup.firstElementChild!.querySelectorAll('[data-usage-status-line]')
    expect([...lines].map(l => l.getAttribute('data-usage-status-line'))).toEqual(['claude', 'codex'])
    for (const vendor of ['claude', 'codex']) {
      expect(q(`[data-usage-status-line="${vendor}"] [data-usage-status-text]`, popup)!.textContent).toBe(
        q(`[data-usage-badge="${vendor}"]`)!.getAttribute('aria-label'),
      )
    }
  })
})

// ── Popup company head (ADR-0259 decisions 1·2·4·5) — 「icon, name, plan: max, 1 minute ago」 ──
describe('popup company head', () => {
  const REJECTED = { kind: 'Rejected' as const, retry_in_secs: 600, detail: null }

  it('head = icon·name · plan · 「·」 · age + ⟳ (attached) · even while refreshing no text is added — no clock icon', () => {
    seed(
      snap('claude', {
        plan: 'max',
        five_hour: { used_pct: 38, resets_at: NOW + 7_980, age_secs: 60, expired: false },
        weekly: { used_pct: 59, resets_at: NOW + 86_400 * 3, age_secs: 0, expired: false },
      }),
    )
    mount(true, false)
    const popup = openPopup()
    const head = popupHead(popup, 'claude')
    expect(head.children).toHaveLength(4)
    const [name, plan, sep, group] = [...head.children] as HTMLElement[]
    expect(name.textContent).toBe('Claude')
    expect(plan.hasAttribute('data-usage-plan')).toBe(true)
    expect(plan.textContent).toBe('플랜: max')
    expect(sep.textContent).toBe('·')
    expect(sep.getAttribute('aria-hidden')).toBe('true')
    expect([...group.children]).toEqual([q('[data-usage-age]', head), vendorRefresh(popup, 'claude')])
    expect(q('[data-usage-age]', head)!.textContent).toBe('1분 전')
    // The lucide icon is only ⟳ — no clock icon is attached to the age.
    expect([...head.querySelectorAll('svg.lucide')].map(svg => svg.closest('button'))).toEqual([
      vendorRefresh(popup, 'claude'),
    ])
    act(() => useUsageStore.setState({ pending: { claude: 1 } }))
    expect(head.children).toHaveLength(4)
    expect(head.textContent).not.toContain('갱신 중')
    expect(vendorRefresh(popup, 'claude').getAttribute('data-usage-refreshing')).toBe('claude')
    expect(vendorRefresh(popup, 'claude').parentElement).toBe(group)
  })

  it('without a plan the age comes right after the name — no 「·」 either', () => {
    seed(snap('codex'))
    mount(false, true)
    const head = popupHead(openPopup(), 'codex')
    expect(q('[data-usage-plan]', head)).toBeNull()
    expect(head.children).toHaveLength(2)
    expect(head.textContent).toBe('Codex방금')
  })

  it('age = the oldest of 5-hour and weekly — an older per-model window is not included (amber is the same)', () => {
    seed(
      snap('claude', {
        five_hour: { used_pct: 38, resets_at: NOW + 7_980, age_secs: 60, expired: false },
        weekly: { used_pct: 59, resets_at: NOW + 86_400 * 3, age_secs: 600, expired: false },
        model_scoped: [
          { label: 'Opus', window: { used_pct: 70, resets_at: NOW + 86_400 * 2, age_secs: 7_200, expired: false } },
        ],
      }),
    )
    mount(true, false)
    const popup = openPopup()
    const age = q('[data-usage-age]', popupHead(popup, 'claude'))!
    expect(age.textContent).toBe('10분 전')
    expect(age.hasAttribute('data-usage-stale')).toBe(false)
    expect(q('[data-usage-window="model:Opus"] [data-usage-value]', popup)!.textContent).toBe('30%')
  })

  it('if one window is missing the remaining window\'s age · if both windows have no value no age (even if a per-model window has a value · Unavailable too) · ⟳ remains', () => {
    seed(snap('claude', { weekly: null, five_hour: { used_pct: 38, resets_at: NOW + 7_980, age_secs: 300, expired: false } }))
    mount(true, false)
    expect(q('[data-usage-age]', popupHead(openPopup(), 'claude'))!.textContent).toBe('5분 전')

    cleanup()
    seed(
      snap('claude', {
        revision: 2,
        plan: 'max',
        five_hour: null,
        weekly: { used_pct: null, resets_at: NOW + 600, age_secs: 0, expired: false },
        model_scoped: [
          { label: 'Opus', window: { used_pct: 70, resets_at: NOW + 86_400, age_secs: 0, expired: false } },
        ],
      }),
    )
    mount(true, false)
    const popup = openPopup()
    const head = popupHead(popup, 'claude')
    expect(q('[data-usage-age]', head)).toBeNull()
    expect(head.textContent).not.toContain('·')
    expect(vendorRefresh(popup, 'claude')).not.toBeNull()
    expect(q('[data-usage-window="model:Opus"]', popup)).not.toBeNull()

    cleanup()
    seed(snap('claude', { revision: 3, state: { kind: 'Unavailable', detail: null } }))
    mount(true, false)
    expect(q('[data-usage-age]', popupHead(openPopup(), 'claude'))).toBeNull()
  })

  it('amber boundary — 1800 seconds is the usual color, 1801 seconds is amber (`--usage-stale`) · the same constant as the small display dimming', () => {
    const at = (age: number, revision: number) =>
      snap('claude', {
        revision,
        five_hour: { used_pct: 38, resets_at: NOW + 7_980, age_secs: age, expired: false },
        weekly: { used_pct: 59, resets_at: NOW + 86_400 * 3, age_secs: 0, expired: false },
      })
    seed(at(STALE_AFTER_SECS, 1))
    mount(true, false)
    const fresh = q('[data-usage-age]', popupHead(openPopup(), 'claude'))!
    expect(fresh.textContent).toBe('30분 전')
    expect(fresh.hasAttribute('data-usage-stale')).toBe(false)
    expect(fresh.hasAttribute('title')).toBe(false)
    expect(fresh.style.color).toBe('var(--text-muted)')

    cleanup()
    seed(at(STALE_AFTER_SECS + 1, 2))
    mount(true, false)
    const stale = q('[data-usage-age]', popupHead(openPopup(), 'claude'))!
    expect(stale.hasAttribute('data-usage-stale')).toBe(true)
    expect(stale.style.color).toBe('var(--usage-stale)')
    expect(stale.getAttribute('title')).toBe('30분 넘게 새로 들어오지 않은 값')
    // In the small display only the number is dimmed — the same window goes stale at the same moment.
    expect(valueOf('claude', 'five_hour')!.hasAttribute('data-usage-stale')).toBe(true)
  })

  it('after crossing 30 minutes by a minute tick it becomes amber without a request', () => {
    seed(
      snap('claude', {
        five_hour: { used_pct: 38, resets_at: NOW + 7_980, age_secs: STALE_AFTER_SECS - 30, expired: false },
        weekly: { used_pct: 59, resets_at: NOW + 86_400 * 3, age_secs: 0, expired: false },
      }),
    )
    mount(true, false)
    const popup = openPopup()
    const age = () => q('[data-usage-age]', popupHead(popup, 'claude'))!
    expect(age().hasAttribute('data-usage-stale')).toBe(false)
    advance(60_000)
    expect(age().hasAttribute('data-usage-stale')).toBe(true)
    expect(age().textContent).toBe('30분 전')
    expect(client.refreshUsageLimits).not.toHaveBeenCalled()
    expect(client.getUsageSnapshot).toHaveBeenCalledTimes(1)
  })

  it('per-company ⟳ = refreshes only that company · the popup stays open · focus on ⟳ · pressing again while spinning is dropped', async () => {
    seed(snap('claude'))
    seed(snap('codex'))
    mount()
    const popup = openPopup()
    const codex = vendorRefresh(popup, 'codex')
    fireEvent.mouseDown(codex)
    act(() => codex.focus())
    fireEvent.click(codex)
    expect(client.refreshUsageLimits.mock.calls.map(c => c[0])).toEqual(['codex'])
    expect(screen.getByRole('dialog')).toBe(popup)
    expect(document.activeElement).toBe(vendorRefresh(popup, 'codex'))
    // Only that company is 「refreshing」 — another company's ⟳ can be pressed.
    expect(vendorRefresh(popup, 'codex').getAttribute('aria-busy')).toBe('true')
    expect(vendorRefresh(popup, 'claude').hasAttribute('aria-busy')).toBe(false)
    fireEvent.click(vendorRefresh(popup, 'codex'))
    expect(client.refreshUsageLimits).toHaveBeenCalledTimes(1)
    await act(async () => {})
    expect(vendorRefresh(popup, 'codex').hasAttribute('aria-busy')).toBe(false)
  })

  it('on a visible refusal only that company\'s ⟳ is aria-disabled (not disabled) · a press does not reach the command', () => {
    seed(snap('claude', { state: REJECTED }))
    seed(snap('codex'))
    mount()
    const popup = openPopup()
    const claude = vendorRefresh(popup, 'claude')
    expect(claude.getAttribute('aria-disabled')).toBe('true')
    expect(claude.disabled).toBe(false)
    expect(vendorRefresh(popup, 'codex').hasAttribute('aria-disabled')).toBe(false)
    const warn = vi.spyOn(console, 'warn')
    fireEvent.click(claude)
    expect(client.refreshUsageLimits).not.toHaveBeenCalled()
    expect(warn).not.toHaveBeenCalled()
    expect(screen.getByRole('dialog')).toBe(popup)
  })

  it('while that company\'s query runs (⟳ waiting · the other\'s in_flight) a press is dropped — received again when it ends', async () => {
    seed(snap('claude', { in_flight: true }))
    seed(snap('codex'))
    mount()
    const popup = openPopup()
    expect(vendorRefresh(popup, 'claude').getAttribute('aria-busy')).toBe('true')
    fireEvent.click(vendorRefresh(popup, 'claude'))
    act(() => {
      seed(snap('claude', { revision: 2, in_flight: false }))
      useUsageStore.setState({ pending: { claude: 1 } })
    })
    fireEvent.click(vendorRefresh(popup, 'claude'))
    expect(client.refreshUsageLimits).not.toHaveBeenCalled()
    act(() => useUsageStore.setState({ pending: {} }))
    fireEvent.click(vendorRefresh(popup, 'claude'))
    expect(client.refreshUsageLimits.mock.calls.map(c => c[0])).toEqual(['claude'])
    await act(async () => {})
  })

  it('even if the focused company ⟳ turns into a refusal, focus is held and only the press is blocked · the popup stays open', () => {
    seed(snap('claude'))
    mount(true, false)
    const popup = openPopup()
    act(() => vendorRefresh(popup, 'claude').focus())
    act(() => seed(snap('claude', { revision: 2, state: REJECTED })))
    expect(vendorRefresh(popup, 'claude').getAttribute('aria-disabled')).toBe('true')
    expect(document.activeElement).toBe(vendorRefresh(popup, 'claude'))
    fireEvent.click(vendorRefresh(popup, 'claude'))
    expect(client.refreshUsageLimits).not.toHaveBeenCalled()
    expect(screen.getByRole('dialog')).toBe(popup)
  })

  it('opening and closing the popup gives 0 queries — pull is once at mount only (ADR-0259 decision 5)', () => {
    seed(snap('claude'))
    seed(snap('codex'))
    mount()
    for (let i = 0; i < 3; i++) {
      openPopup()
      fireEvent.click(summary())
      expect(screen.queryByRole('dialog')).toBeNull()
    }
    openPopup()
    fireEvent.keyDown(document, { key: 'Escape' })
    openPopup()
    fireEvent.mouseDown(document.body)
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(client.refreshUsageLimits).not.toHaveBeenCalled()
    expect(client.getUsageSnapshot).toHaveBeenCalledTimes(1)
  })
})

// ── Hourglass (ADR-0259 decision 3) — the word 「reset」 only in the name · no arrow icon ──
describe('reset notation = hourglass', () => {
  const NOT_HOURGLASS = /lucide-(refresh|rotate|history|repeat|undo|redo|arrow|clock|alarm|calendar)/

  /** `scope` = the element that holds the marker — find it by name and check it is the same element (`title` becomes the name). */
  function expectHourglass(scope: HTMLElement, mark: HTMLElement, at: number): void {
    expect(mark.getAttribute('role')).toBe('img')
    expect(mark.getAttribute('title')).toBe(resetName(at))
    expect(mark.hasAttribute('aria-label')).toBe(false)
    expect(within(scope).getByRole('img', { name: resetName(at) })).toBe(mark)
    expect(mark.textContent).toBe(resetText(at))
    const svgs = [...mark.querySelectorAll('svg')]
    expect(svgs).toHaveLength(1)
    expect(svgs[0].getAttribute('class')).toContain('lucide-hourglass')
    expect(svgs[0].getAttribute('class')).not.toMatch(NOT_HOURGLASS)
    expect(svgs[0].getAttribute('aria-hidden')).toBe('true')
  }

  it('the reset spot of the small display (stages 1·2) and the popup has the same shape — the tooltip 「reset HH:MM」 is itself the name (no aria-label) · the icon is hidden from assistive tech', () => {
    seed(snap('claude'))
    mount(true, false)
    for (const n of [1, 2] as const) {
      toStage(n)
      for (const [w, at] of [
        ['five_hour', NOW + 7_980],
        ['weekly', NOW + 86_400 * 3],
      ] as const) {
        const seg = q(`[data-usage-vendor="claude"][data-usage-window="${w}"]`, summary())!
        expectHourglass(seg, q('[data-usage-reset-clock]', seg)!, at)
      }
    }
    const popup = openPopup()
    for (const [w, at] of [
      ['five_hour', NOW + 7_980],
      ['weekly', NOW + 86_400 * 3],
    ] as const) {
      const row = q(`[data-usage-window="${w}"]`, popup)!
      const reset = popupReset(row)
      expectHourglass(row, reset.mark, at)
      // The remaining time is outside the marker that carries the name — it stays as visible text (assistive tech reads it too) and is not part of the name.
      expect(reset.mark.contains(q('[data-usage-reset-in]', row))).toBe(false)
      expect(reset.name).not.toContain('뒤')
      expect(q('[data-usage-reset-at]', row)!.textContent).toBe(`${resetText(at)} ${reset.after}`)
    }
  })

  it('the reset marker has no overlap of name and description — no marker carries both aria-label and title (small display · popup · per-model window)', () => {
    seed(
      snap('claude', {
        model_scoped: [
          { label: 'Opus', window: { used_pct: 70, resets_at: NOW + 86_400 * 2, age_secs: 0, expired: false } },
        ],
      }),
    )
    mount(true, false)
    openPopup()
    const marks = [
      ...document.querySelectorAll<HTMLElement>('[data-usage-reset-clock], [data-usage-reset-at] [role="img"]'),
    ]
    expect(marks).toHaveLength(5)
    for (const mark of marks) {
      expect(mark.hasAttribute('title')).toBe(true)
      expect(mark.hasAttribute('aria-label')).toBe(false)
    }
  })

  it('the ↻ shape (RefreshCw) is only on the ⟳ button — no arrow icon at the reset spot and none in the measuring copy (ADR-0261)', () => {
    seed(snap('claude', { in_flight: true }))
    mount(true, false)
    openPopup()
    const resets = [...document.querySelectorAll<HTMLElement>('[data-usage-reset-clock], [data-usage-reset-at]')]
    expect(resets.length).toBeGreaterThan(0)
    for (const el of resets) {
      for (const svg of el.querySelectorAll('svg')) expect(svg.getAttribute('class')).not.toMatch(NOT_HOURGLASS)
    }
    const icons = [...document.querySelectorAll('svg.lucide-refresh-cw')]
    expect(icons.length).toBeGreaterThan(0)
    for (const svg of icons) expect(svg.closest('[data-usage-refresh], [data-usage-vendor-refresh]')).not.toBeNull()
    expect(document.querySelectorAll('[data-usage-measure] svg.lucide-refresh-cw')).toHaveLength(0)
  })
})

// ── The ⟳ of the small display — enabled companies at once · command path · separate from the popup ──
describe('⟳ of the small display', () => {
  it('at all three width stages one on the right of the grid — a bordered icon button · name 「refresh usage」', () => {
    seed(snap('claude'))
    seed(snap('codex'))
    mount()
    for (const n of [1, 2, 3] as const) {
      toStage(n)
      const buttons = document.querySelectorAll('[data-usage-slot] [data-usage-refresh]')
      expect(buttons).toHaveLength(1)
      const refresh = refreshButton()!
      expect(refresh.tagName).toBe('BUTTON')
      expect(refresh.getAttribute('type')).toBe('button')
      expect(refresh.parentElement!.previousElementSibling).toBe(summary())
      expect(refresh.style.border).toBe('1px solid var(--border)')
      expect(q('svg', refresh)).not.toBeNull()
      expect(refresh.textContent).toBe('')
    }
    expect(screen.getByRole('button', { name: '사용량 새로고침' })).toBe(refreshButton())
  })

  it('it does not go inside the summary button — interactive elements do not overlap and there is no button or input inside the summary button', () => {
    seed(snap('claude'))
    mount(true, false)
    for (const n of [1, 2, 3] as const) {
      toStage(n)
      expect(summary().contains(refreshButton())).toBe(false)
      expect(summary().querySelector('button, input, a, [tabindex]')).toBeNull()
    }
  })

  it('pressing it does not open the popup', () => {
    seed(snap('claude'))
    mount(true, false)
    fireEvent.mouseDown(refreshButton()!)
    fireEvent.click(refreshButton()!)
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(summary().getAttribute('aria-expanded')).toBe('false')
  })

  it('pressing makes the command refresh only enabled companies that are not a visible refusal (disabled companies · companies under refusal drop out)', () => {
    seed(snap('claude'))
    seed(snap('codex'))
    mount(true, true)
    fireEvent.click(refreshButton()!)
    expect(client.refreshUsageLimits.mock.calls.map(c => c[0])).toEqual(['claude', 'codex'])

    cleanup()
    client.refreshUsageLimits.mockClear()
    useUsageStore.setState({ pending: {} })
    seed(snap('claude', { state: { kind: 'Rejected', retry_in_secs: 600, detail: null } }))
    mount(true, true)
    expect(refreshButton()!.hasAttribute('aria-disabled')).toBe(false)
    fireEvent.click(refreshButton()!)
    expect(client.refreshUsageLimits.mock.calls.map(c => c[0])).toEqual(['codex'])

    cleanup()
    client.refreshUsageLimits.mockClear()
    useUsageStore.setState({ pending: {} })
    mount(false, true)
    fireEvent.click(refreshButton()!)
    expect(client.refreshUsageLimits.mock.calls.map(c => c[0])).toEqual(['codex'])
  })

  it('a press while the target\'s query runs is dropped — ⟳ spins, and it is received again when the answer comes', () => {
    seed(snap('claude'))
    seed(snap('codex'))
    mount()
    act(() => useUsageStore.setState({ pending: { codex: 1 } }))
    expect(refreshButton()!.getAttribute('data-usage-refreshing')).toBe('codex')
    expect(refreshButton()!.getAttribute('aria-busy')).toBe('true')
    fireEvent.click(refreshButton()!)
    expect(client.refreshUsageLimits).not.toHaveBeenCalled()
    // A query the other side runs by itself (in_flight) is the same.
    act(() => {
      useUsageStore.setState({ pending: {} })
      seed(snap('claude', { revision: 2, in_flight: true }))
    })
    fireEvent.click(refreshButton()!)
    expect(client.refreshUsageLimits).not.toHaveBeenCalled()
    act(() => seed(snap('claude', { revision: 3, in_flight: false })))
    expect(refreshButton()!.hasAttribute('aria-busy')).toBe(false)
    fireEvent.click(refreshButton()!)
    expect(client.refreshUsageLimits.mock.calls.map(c => c[0])).toEqual(['claude', 'codex'])
  })

  it('if every enabled company is a visible refusal then aria-disabled (not disabled) · a press does not reach the command', () => {
    const rejected = { kind: 'Rejected' as const, retry_in_secs: 600, detail: null }
    seed(snap('claude', { state: rejected }))
    seed(snap('codex', { state: rejected }))
    mount()
    const refresh = refreshButton()!
    expect(refresh.getAttribute('aria-disabled')).toBe('true')
    expect(refresh.disabled).toBe(false) // Left so it can hold focus
    const warn = vi.spyOn(console, 'warn')
    fireEvent.click(refresh)
    expect(client.refreshUsageLimits).not.toHaveBeenCalled()
    expect(warn).not.toHaveBeenCalled() // It is not that it went all the way to the command and was refused
  })

  it('even if the focused ⟳ turns into a refusal, focus is held and only the press is blocked', () => {
    seed(snap('claude'))
    mount(true, false)
    act(() => refreshButton()!.focus())
    act(() => seed(snap('claude', { revision: 2, state: { kind: 'Rejected', retry_in_secs: 600, detail: null } })))
    expect(refreshButton()!.getAttribute('aria-disabled')).toBe('true')
    expect(document.activeElement).toBe(refreshButton())
    fireEvent.click(refreshButton()!)
    expect(client.refreshUsageLimits).not.toHaveBeenCalled()
  })

  it('pressing ⟳ with the popup open leaves the popup as is · refresh happens · Esc on ⟳ closes but keeps focus on ⟳', () => {
    seed(snap('claude'))
    mount(true, false)
    openPopup()
    fireEvent.mouseDown(refreshButton()!)
    act(() => refreshButton()!.focus())
    fireEvent.click(refreshButton()!)
    expect(screen.queryByRole('dialog')).not.toBeNull()
    expect(client.refreshUsageLimits).toHaveBeenCalledWith('claude')
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(document.activeElement).toBe(refreshButton())
  })

  it('with both companies off there is none', () => {
    mount(false, false)
    expect(refreshButton()).toBeNull()
  })
})

// ── The popup's display toggle row ──
describe('display toggle row of the popup', () => {
  function toggles(popup: HTMLElement): HTMLInputElement[] {
    return within(q('[data-usage-show-toggles]', popup)!).getAllByRole('checkbox') as HTMLInputElement[]
  }
  /** The argument of a display field write that went out to the shell — other invokes are filtered out. */
  function usageWrites(): unknown[] {
    return invokeMock.mock.calls.filter(([cmd]) => cmd === 'set_usage_slot').map(([, args]) => args)
  }

  it('at the very bottom 「Show in slot:」 + a named checkbox per company — enabled company ☑ · disabled company ☐', () => {
    seed(snap('claude'))
    mount(true, false)
    const popup = openPopup()
    const row = q('[data-usage-show-toggles]', popup)!
    expect(popup.lastElementChild).toBe(row)
    expect(row.getAttribute('role')).toBe('group')
    expect(within(popup).getByRole('group', { name: '슬롯에 표시:' })).toBe(row)
    expect(row.style.flexWrap).toBe('wrap')
    expect(toggles(popup).map(c => c.getAttribute('data-usage-show'))).toEqual(['claude', 'codex'])
    expect(toggles(popup).map(c => c.checked)).toEqual([true, false])
    expect(within(popup).getByRole('checkbox', { name: 'Claude' })).toBe(toggles(popup)[0])
    expect(within(popup).getByRole('checkbox', { name: 'Codex' })).toBe(toggles(popup)[1])
  })

  it('pressing runs that company\'s toggle command with the slot coordinates — writes only the one value with that cell flipped', () => {
    mount(true, false)
    fireEvent.click(within(openPopup()).getByRole('checkbox', { name: 'Codex' }))
    expect(usageWrites()).toEqual([{ viewId: 'v1', slotId: 's1', showClaude: null, showCodex: true }])
  })

  it('consecutive presses of different companies before the echo — Codex on → Claude off goes out as two single-cell writes and whichever echo order only Codex remains', () => {
    mount(true, false)
    const popup = openPopup()
    fireEvent.click(within(popup).getByRole('checkbox', { name: 'Codex' }))
    fireEvent.click(within(popup).getByRole('checkbox', { name: 'Claude' }))
    expect(usageWrites()).toEqual([
      { viewId: 'v1', slotId: 's1', showClaude: null, showCodex: true },
      { viewId: 'v1', slotId: 's1', showClaude: false, showCodex: null },
    ])
  })

  it('a press after the echo flips from the incoming content', () => {
    const { rerender } = mount(true, false)
    const popup = openPopup()
    fireEvent.click(within(popup).getByRole('checkbox', { name: 'Codex' }))
    rerender(<UsageSlot content={usage(true, true)} viewId="v1" slotId="s1" />)
    fireEvent.click(within(popup).getByRole('checkbox', { name: 'Codex' }))
    expect(usageWrites()).toEqual([
      { viewId: 'v1', slotId: 's1', showClaude: null, showCodex: true },
      { viewId: 'v1', slotId: 's1', showClaude: null, showCodex: false },
    ])
  })

  it('if the write is refused only a log remains and the check stays as the slot content', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    invokeMock.mockRejectedValueOnce(new Error('ipc dropped'))
    mount(true, false)
    const popup = openPopup()
    const codex = () => within(popup).getByRole('checkbox', { name: 'Codex' }) as HTMLInputElement
    fireEvent.click(codex())
    await act(async () => {})
    expect(warn).toHaveBeenCalledWith(expect.stringMatching(/usageSlot\.toggleCodex/), expect.any(Error))
    expect(codex().checked).toBe(false)
  })

  it('the check follows the slot content — a press alone does not change it, it changes when the content returns · the popup stays open', () => {
    seed(snap('claude'))
    seed(snap('codex'))
    const { rerender } = mount(true, false)
    const popup = openPopup()
    const codex = () => within(popup).getByRole('checkbox', { name: 'Codex' }) as HTMLInputElement
    fireEvent.click(codex())
    expect(codex().checked).toBe(false)
    rerender(<UsageSlot content={usage(true, true)} viewId="v1" slotId="s1" />)
    expect(screen.getByRole('dialog')).toBe(popup)
    expect(codex().checked).toBe(true)
    expect(q('[data-usage-popup-vendor="codex"]', popup)).not.toBeNull()
  })

  it('even if the last company is turned off inside the popup the popup and focus remain — the small display becomes the notice, the popup only the toggle row', () => {
    seed(snap('claude'))
    const { rerender } = mount(true, false)
    const popup = openPopup()
    const claude = within(popup).getByRole('checkbox', { name: 'Claude' })
    act(() => claude.focus())
    fireEvent.click(claude)
    rerender(<UsageSlot content={usage(false, false)} viewId="v1" slotId="s1" />)
    expect(screen.getByRole('dialog')).toBe(popup)
    expect([...popup.children].map(c => c.hasAttribute('data-usage-show-toggles'))).toEqual([true])
    expect(document.activeElement).toBe(claude)
    expect(q('[data-usage-hint]', summary())).not.toBeNull()
    expect(refreshButton()).toBeNull()
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(document.activeElement).toBe(summary())
  })

  it('if the coordinates are unknown (no viewId) the command refuses and only a log remains — the layout is not written', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    render(<UsageSlot content={usage(true, false)} viewId={null} slotId="s1" />)
    fireEvent.click(within(openPopup()).getByRole('checkbox', { name: 'Codex' }))
    expect(usageWrites()).toEqual([])
    expect(warn).toHaveBeenCalledWith(expect.stringMatching(/usageSlot\.toggleCodex/), expect.any(Error))
  })
})

// ── Stage picking and ⟳ — ⟳ is right next to the content · the width for the stage decision is slot width − ⟳ area ──
describe('stage measuring and ⟳', () => {
  it('lines = only the summary button · the ⟳ spot (does not shrink) — ⟳ is not inside the measuring copy or the summary button (putting it in would subtract the ⟳ share twice)', () => {
    seed(snap('claude'))
    seed(snap('codex'))
    mount()
    const row = q('[data-usage-row]')!
    const area = q('[data-usage-refresh-area]')!
    expect(row.style.display).toBe('flex')
    expect([...row.children]).toEqual([summary(), area])
    expect([...area.children]).toEqual([refreshButton()])
    expect(area.style.flexGrow).toBe('0')
    expect(area.style.flexShrink).toBe('0')
    expect(summary().style.flexGrow).toBe('1')
    expect(summary().style.flexShrink).toBe('1')
    expect(parseFloat(summary().style.minWidth)).toBe(0)
    for (const n of ['1', '2']) expect(q(`[data-usage-measure="${n}"] [data-usage-refresh]`)).toBeNull()
    expect(q('[data-usage-content] [data-usage-refresh]')).toBeNull()
  })

  it('⟳ is right next to the content at all stages — in stages 1·2 the bar cell (1fr) fills the summary button, in stage 3 the line shrinks to the content width (slot width cap)', () => {
    seed(snap('claude'))
    seed(snap('codex'))
    mount()
    const row = q('[data-usage-row]')!
    for (const n of [1, 2] as const) {
      toStage(n)
      expect(row.style.width).toBe('')
      const grid = q('[data-usage-grid]', summary())!
      expect(tracks(grid.style.gridTemplateColumns)).toContain('minmax(4em, 1fr)')
      expect(grid.style.display).toBe('grid')
    }
    toStage(3)
    expect(row.style.width).toBe('fit-content')
    expect(row.style.maxWidth).toBe('100%')
    // Even when the line shrinks the summary button must be able to get narrower than the slot — cut the content and keep the ⟳ area.
    expect(parseFloat(summary().style.minWidth)).toBe(0)
    expect(q('[data-usage-content]')!.style.overflow).toBe('hidden')
    toStage(1)
    expect(row.style.width).toBe('')
  })

  it('stage verdict width = slot width − ⟳ spot − summary button margin — if the ⟳ spot widens the threshold also moves', () => {
    seed(snap('claude'))
    mount(true, false)
    const at = (rootW: number, refreshW = REFRESH_AREA_W) => {
      fireSizes({ rootW, rootH: 1000, refreshW, s1: 400, s2: 200, s2h: 30 })
      return stage()
    }
    expect(at(400 + REFRESH_AREA_W + SUMMARY_PAD_X_TOTAL)).toBe('1')
    expect(at(400 + REFRESH_AREA_W + SUMMARY_PAD_X_TOTAL - 1)).toBe('2')
    expect(at(200 + REFRESH_AREA_W + SUMMARY_PAD_X_TOTAL)).toBe('2')
    expect(at(200 + REFRESH_AREA_W + SUMMARY_PAD_X_TOTAL - 1)).toBe('3')
    expect(at(400 + REFRESH_AREA_W + SUMMARY_PAD_X_TOTAL, REFRESH_AREA_W + 1)).toBe('2')
  })

  it('the chosen stage does not return to its own input — the content width shrunk to stage 3 is not measured, so when widened it goes back to stage 1', () => {
    seed(snap('claude'))
    seed(snap('codex'))
    mount()
    // Elements whose width changes with the stage (line · summary button · visible content) are not observation targets.
    const observed = liveObserver().observed
    for (const sel of ['[data-usage-row]', '[data-usage-summary]', '[data-usage-content]']) {
      expect(observed).not.toContain(q(sel))
    }
    const wide = { rootW: 500, rootH: 1000, refreshW: REFRESH_AREA_W, s1: 400, s2: 200, s2h: 30 }
    fireSizes(wide)
    expect(stage()).toBe('1')
    fireSizes({ ...wide, rootW: 120 })
    expect(stage()).toBe('3')
    // After being laid out again with the stage-3 picture, even if the same size is notified again it stays at stage 3 (it does not go back and forth).
    fireSizes({ ...wide, rootW: 120 })
    expect(stage()).toBe('3')
    // A notification from an element that is not an observation target (content shrunk at stage 3) does not enter the decision.
    act(() => liveObserver().fire([[q('[data-usage-content]')!, 40]]))
    expect(stage()).toBe('3')
    fireSizes(wide)
    expect(stage()).toBe('1')
    act(() => liveObserver().fire([[q('[data-usage-content]')!, 40]]))
    expect(stage()).toBe('1')
  })

  it('notice text (no enabled company) — the ⟳ spot is empty and width 0 · the line fills the slot width so the whole notice is the pressing spot', () => {
    mount(false, false)
    const area = q('[data-usage-refresh-area]')!
    expect(area.children).toHaveLength(0)
    expect(liveObserver().observed).toContain(area)
    fireSizes({ rootW: 300, rootH: 1000, refreshW: 0, s1: 0, s2: 0, s2h: 0 })
    expect(stage()).toBe('1')
    expect(q('[data-usage-row]')!.style.width).toBe('')
    expect(summary().style.flexGrow).toBe('1')
  })

  it('even when querying switches on and off the small display content and measuring copy are the same — the only querying indicator is the ⟳ rotation (ADR-0261)', () => {
    seed(snap('claude'))
    seed(snap('codex'))
    mount()
    const copies = () => ['1', '2'].map(n => q(`[data-usage-measure="${n}"]`)!.innerHTML)
    const idle = copies()
    for (const n of [1, 2, 3] as const) {
      toStage(n)
      const shown = q('[data-usage-content]')!.innerHTML
      act(() => useUsageStore.setState({ pending: { claude: 1 } }))
      expect(q('[data-usage-content]')!.innerHTML).toBe(shown)
      expect(copies()).toEqual(idle)
      expect(q('svg.lucide-refresh-cw', summary())).toBeNull()
      act(() => useUsageStore.setState({ pending: {} }))
    }
    for (const n of ['1', '2']) expect(q(`[data-usage-measure="${n}"] svg.lucide-refresh-cw`)).toBeNull()
  })

  it('even moving between stages ⟳ remains the same element (its width does not vary by stage)', () => {
    seed(snap('claude'))
    mount(true, false)
    const first = refreshButton()
    for (const n of [3, 2, 1, 3] as const) {
      toStage(n)
      expect(refreshButton()).toBe(first)
    }
  })
})

// ── Low slot (stage by width only) · window carrying only reset · popup place · focus · Esc · rotation ──
function rect(left: number, top: number, width: number, height: number): DOMRect {
  return { left, top, width, height, right: left + width, bottom: top + height, x: left, y: top, toJSON() {} } as DOMRect
}

/** Gives the on-screen rectangles of the summary and popup (jsdom has no layout). */
function stubRects(summaryRect: DOMRect, popupRect: DOMRect): void {
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.hasAttribute('data-usage-popup')) return popupRect
    if (this.hasAttribute('data-usage-summary')) return summaryRect
    return rect(0, 0, 0, 0)
  })
}

describe('height · windows carrying only a reset', () => {
  it('stages are by width only — even if stage 2 is taller than the slot, if the width allows it is stage 2 and the slot root clips below (ADR-0259 decision 6)', () => {
    seed(snap('claude', { state: { kind: 'NeedsLogin', detail: null } }))
    mount(true, false)
    setWidths(300, 400, 200, { rootH: 53, s2h: 45 })
    expect(stage()).toBe('2')
    for (const rootH of [52, 20, 1]) {
      setWidths(300, 400, 200, { rootH, s2h: 45 })
      expect(stage()).toBe('2')
    }
    expect(q('[data-usage-slot]')!.style.overflow).toBe('hidden')
    // Even when it is cut off and not visible, the badge is still in the DOM (the price of accepting — it is only hidden, not gone).
    expect(q('[data-usage-badge="claude"]')).not.toBeNull()
    setWidths(500, 400, 200, { rootH: 10, s2h: 45 })
    expect(stage()).toBe('1')
    setWidths(199, 400, 200, { rootH: 1000, s2h: 45 })
    expect(stage()).toBe('3')
  })

  it('a window carrying only a reset time — no bar, 「—」 + reset time (stages 1·2) · the popup has the absolute reset time (no age on the window line)', () => {
    seed(snap('claude', { five_hour: { used_pct: null, resets_at: NOW + 600, age_secs: 0, expired: false } }))
    mount(true, false)
    const seg = () => q('[data-usage-vendor="claude"][data-usage-window="five_hour"]', summary())!
    for (const n of [1, 2] as const) {
      toStage(n)
      expect(q('[data-usage-value]', seg())!.textContent).toBe('—')
      expect(q('[data-usage-bar]', seg())).toBeNull()
      expect(q('[data-usage-reset-clock]', seg())!.textContent).toBe(resetText(NOW + 600))
    }
    toStage(3)
    expect(q('[data-usage-numbers]')!.textContent).toBe('—·41')
    const row = q('[data-usage-window="five_hour"]', openPopup())!
    expect(q('[data-usage-bar]', row)).toBeNull()
    expect(q('[role="meter"]', row)).toBeNull()
    expect(popupReset(row).text).toBe(formatResetAt(NOW + 600, NOW))
    expect(popupReset(row).after).toBe('(10분 뒤)')
    expect(q('[data-usage-age]', row)).toBeNull()
  })
})

describe('popup position · focus · Esc', () => {
  it('if it fits below it opens below the summary', () => {
    seed(snap('claude'))
    mount(true, false)
    stubRects(rect(10, 10, 300, 40), rect(0, 0, 240, 200))
    const popup = openPopup()
    expect(popup.style.top).toBe(`${50 + ANCHOR_GAP}px`)
  })

  it('if it does not fit below it opens upward from the summary\'s top edge — it does not cover the summary', () => {
    seed(snap('claude'))
    mount(true, false)
    const vh = window.innerHeight
    const summaryTop = vh - 100
    stubRects(rect(10, summaryTop, 300, 60), rect(0, 0, 240, 200))
    const popup = openPopup()
    expect(popup.style.top).toBe(`${summaryTop - ANCHOR_GAP - 200}px`)
    expect(parseFloat(popup.style.top) + 200).toBeLessThanOrEqual(summaryTop)
  })

  it('if the summary grows while open (enabling a company adds a line) it re-measures so the grown summary is not covered either', () => {
    seed(snap('claude'))
    seed(snap('codex'))
    let summaryRect = rect(10, 10, 300, 40)
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
      if (this.hasAttribute('data-usage-popup')) return rect(0, 0, 240, 200)
      if (this.hasAttribute('data-usage-summary')) return summaryRect
      return rect(0, 0, 0, 0)
    })
    const { rerender } = mount(true, false)
    const popup = openPopup()
    expect(popup.style.top).toBe(`${50 + ANCHOR_GAP}px`)
    summaryRect = rect(10, 10, 300, 58)
    rerender(<UsageSlot content={usage(true, true)} viewId="v1" slotId="s1" />)
    expect(screen.getByRole('dialog')).toBe(popup)
    expect(popup.style.top).toBe(`${68 + ANCHOR_GAP}px`)
    expect(parseFloat(popup.style.top)).toBeGreaterThanOrEqual(summaryRect.bottom)
  })

  it('focus moves only after the position is set and shown — Chromium refuses focus on an invisible element', () => {
    seed(snap('claude'))
    mount(true, false)
    const seen: string[] = []
    const original = HTMLElement.prototype.focus
    vi.spyOn(HTMLElement.prototype, 'focus').mockImplementation(function (this: HTMLElement, opts?: FocusOptions) {
      if (this.hasAttribute('data-usage-popup')) seen.push(this.style.visibility)
      original.call(this, opts)
    })
    const popup = openPopup()
    expect(seen).toEqual(['visible'])
    expect(document.activeElement).toBe(popup)
  })

  it('if focus leaves the popup and summary it closes, and focus stays where it went', () => {
    seed(snap('claude'))
    render(
      <>
        <UsageSlot content={usage(true, false)} viewId="v1" slotId="s1" />
        <button data-testid="outside">outside</button>
      </>,
    )
    openPopup()
    act(() => summary().focus())
    expect(screen.queryByRole('dialog')).not.toBeNull()
    const outside = screen.getByTestId('outside')
    act(() => outside.focus())
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(document.activeElement).toBe(outside)
  })

  it('Esc closes when focus is in the popup or summary', () => {
    seed(snap('claude'))
    mount(true, false)
    openPopup()
    act(() => summary().focus())
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(document.activeElement).toBe(summary())
  })

  it('even when focus is nowhere (body) Esc closes and returns focus to the summary — the case where the focus inside the popup dropped out', () => {
    seed(snap('claude'))
    mount(true, false)
    const popup = openPopup()
    act(() => popup.blur())
    expect(document.activeElement).toBe(document.body)
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(document.activeElement).toBe(summary())
  })

  it('while querying ⟳ spins, and even after the query ends it spins until that turn\'s boundary (animationiteration) · a press during the tail = a new query + from the start (ADR-0261)', async () => {
    seed(snap('claude', { in_flight: true }))
    mount(true, false)
    const svg = () => q('svg.lucide-refresh-cw', refreshButton()!)!
    // jsdom has no AnimationEvent so React listens under the prefixed name — fire both names.
    const roundEnds = () => {
      fireEvent.animationIteration(svg())
      fireEvent(svg(), new Event('webkitAnimationIteration', { bubbles: true }))
    }
    const expectTail = () => {
      expect(svg().getAttribute('class')).toContain('animate-spin')
      expect(refreshButton()!.hasAttribute('data-usage-refreshing')).toBe(false)
      expect(refreshButton()!.hasAttribute('aria-busy')).toBe(false)
    }
    expect(svg().getAttribute('class')).toContain('animate-spin')
    // It does not stop at a boundary while the query is running.
    roundEnds()
    expect(svg().getAttribute('class')).toContain('animate-spin')
    act(() => seed(snap('claude', { revision: 2 })))
    expectTail()
    roundEnds()
    expect(svg().getAttribute('class')).not.toContain('animate-spin')

    // Pressing before the boundary = an ordinary press — one command · attach the icon anew and start from the beginning.
    act(() => seed(snap('claude', { revision: 3, in_flight: true })))
    act(() => seed(snap('claude', { revision: 4 })))
    expectTail()
    const tail = svg()
    fireEvent.click(refreshButton()!)
    expect(client.refreshUsageLimits.mock.calls.map(c => c[0])).toEqual(['claude'])
    expect(svg()).not.toBe(tail)
    expect(svg().getAttribute('class')).toContain('animate-spin')
    await act(async () => {})
  })

  it('pressing starts the spin · a re-press while querying only restarts the rotation without a query (icon swap) · on refusal it does not spin (ADR-0261)', () => {
    seed(snap('claude'))
    mount(true, false)
    const svg = () => q('svg.lucide-refresh-cw', refreshButton()!)!
    expect(svg().getAttribute('class')).not.toContain('animate-spin')
    fireEvent.click(refreshButton()!)
    expect(client.refreshUsageLimits).toHaveBeenCalledTimes(1)
    const first = svg()
    expect(first.getAttribute('class')).toContain('animate-spin')
    act(() => useUsageStore.setState({ pending: { claude: 1 } }))
    fireEvent.click(refreshButton()!)
    expect(client.refreshUsageLimits).toHaveBeenCalledTimes(1)
    expect(svg()).not.toBe(first)
    expect(svg().getAttribute('class')).toContain('animate-spin')

    cleanup()
    seed(snap('claude', { revision: 3, state: { kind: 'Rejected', retry_in_secs: 600, detail: null } }))
    useUsageStore.setState({ pending: {} })
    mount(true, false)
    fireEvent.click(refreshButton()!)
    expect(svg().getAttribute('class')).not.toContain('animate-spin')
  })
})

// ── Small display overhaul — company icon · one-grid vertical alignment · no ⚠ ──
describe('company icon', () => {
  it('at all width stages an icon instead of the name — role=img · aria-label = title = company name · color is the company token', () => {
    seed(snap('claude'))
    seed(snap('codex'))
    mount()
    for (const n of [1, 2, 3] as const) {
      toStage(n)
      for (const [vendor, name] of [
        ['claude', 'Claude'],
        ['codex', 'Codex'],
      ] as const) {
        const icon = q(`[data-usage-icon="${vendor}"]`, summary())!
        expect(icon.getAttribute('role')).toBe('img')
        expect(icon.getAttribute('aria-label')).toBe(name)
        expect(icon.getAttribute('title')).toBe(name)
        expect(icon.style.color).toBe(`var(--usage-vendor-${vendor})`)
        expect(q('svg', icon)!.getAttribute('width')).toBe('16')
      }
      // The name text was dropped from the small display — only the summary button's accessibility name carries the name.
      expect(summary().textContent).not.toMatch(/Claude|Codex|Cl|Cx/)
      expect(summary().getAttribute('aria-label')).toMatch(/^Claude .* · Codex /)
    }
  })

  it('shape = placeholder figure — Claude is an eight-ray sun + center dot, Codex is a hexagon outline + center dot', () => {
    seed(snap('claude'))
    seed(snap('codex'))
    mount()
    const claude = q('[data-usage-icon="claude"] svg', summary())!
    expect(claude.querySelectorAll('line')).toHaveLength(8)
    expect(claude.querySelectorAll('circle')).toHaveLength(1)
    const codex = q('[data-usage-icon="codex"] svg', summary())!
    expect(codex.querySelector('polygon')!.getAttribute('fill')).toBe('none')
    expect(codex.querySelectorAll('circle')).toHaveLength(1)
  })

  it('the badge is right next to the icon at all width stages — both companies, outside the number ellipsis even at stage 3', () => {
    seed(snap('claude', { state: { kind: 'NeedsLogin', detail: null } }))
    seed(snap('codex', { state: { kind: 'NotInstalled', detail: null }, in_flight: true }))
    mount()
    for (const n of [1, 2, 3] as const) {
      toStage(n)
      for (const vendor of ['claude', 'codex']) {
        const badge = q(`[data-usage-badge="${vendor}"]`, summary())!
        expect(badge.previousElementSibling).toBe(q(`[data-usage-icon="${vendor}"]`, summary()))
        expect(badge.parentElement!.style.flexShrink).toBe('0')
        expect(badge.closest('[data-usage-numbers]')).toBeNull()
      }
    }
  })

  it('the popup is icon + company name text — the icon is decorative (so the name is not read twice)', () => {
    seed(snap('claude'))
    seed(snap('codex'))
    mount()
    const popup = openPopup()
    for (const [vendor, name] of [
      ['claude', 'Claude'],
      ['codex', 'Codex'],
    ] as const) {
      const section = q(`[data-usage-popup-vendor="${vendor}"]`, popup)!
      const icon = q(`[data-usage-icon="${vendor}"]`, section)!
      expect(icon.getAttribute('aria-hidden')).toBe('true')
      expect(icon.hasAttribute('role')).toBe(false)
      expect(icon.nextElementSibling!.textContent).toBe(name)
    }
  })

  it('the usage color tokens in all three themes point only at base tokens (no literals — for color presets) · the e-ink base is the body color', () => {
    const css = require('node:fs').readFileSync(`${process.cwd()}/src/styles/theme.css`, 'utf8')
    // Two declaration blocks per theme line — the base (starting with `--bg`) and usage (starting with `--usage-ok`).
    const block = (theme: string, first: string): string => {
      const m = css.match(new RegExp(`:root\\[data-theme='${theme}'\\]\\s*\\{\\s*${first}[^}]*\\}`))
      expect(m, `${theme} has no ${first} block`).not.toBeNull()
      return m![0]
    }
    for (const theme of ['dark', 'light', 'e-ink']) {
      const usage = block(theme, '--usage-ok')
      expect(usage).toContain('--usage-danger: var(--status-danger);')
      expect(usage).toContain('--usage-vendor-claude: var(--vendor-claude);')
      expect(usage).toContain('--usage-vendor-codex: var(--vendor-codex);')
      // The old age in the popup head (amber — ADR-0259 decision 4).
      expect(usage).toContain('--usage-stale: var(--status-blocked);')
      expect(usage, `${theme} usage row has a color literal`).not.toMatch(/#[0-9a-fA-F]{3,8}\b/)
      const base = block(theme, '--bg')
      for (const token of ['--status-danger', '--status-blocked', '--vendor-claude', '--vendor-codex']) {
        expect(base, `${theme} base has no ${token}`).toMatch(new RegExp(`${token}:\\s*[^;]+;`))
      }
    }
    const eink = block('e-ink', '--bg')
    // Amber in e-ink is the body color — it only gets darker than the dim text with no color cue (TRD §6 #30 undecided).
    for (const token of ['--status-danger', '--status-blocked', '--vendor-claude', '--vendor-codex']) {
      expect(eink).toContain(`${token}: var(--text);`)
    }
  })
})

describe('one-grid vertical alignment', () => {
  /** The cells of one window (= the children of the `display: contents` that carries the value marker). */
  function cells(vendor: string, window: string): HTMLElement[] {
    const seg = q(`[data-usage-vendor="${vendor}"][data-usage-window="${window}"]`, summary())!
    expect(seg.style.display).toBe('contents')
    return [...seg.children] as HTMLElement[]
  }

  it('stage 1 — two company lines share the same nine cells of one grid (icon | name·bar·%·reset × 2)', () => {
    seed(snap('claude', { five_hour: { used_pct: 99, resets_at: NOW + 600, age_secs: 0, expired: false } }))
    seed(snap('codex'))
    mount()
    toStage(1)
    const grids = summary().querySelectorAll<HTMLElement>('[data-usage-grid]')
    expect(grids).toHaveLength(1)
    const grid = grids[0]
    expect(grid.style.display).toBe('grid')
    expect(grid.style.fontVariantNumeric).toBe('tabular-nums')
    expect(tracks(grid.style.gridTemplateColumns)).toHaveLength(9)
    for (const [vendor, row] of [
      ['claude', '1'],
      ['codex', '2'],
    ] as const) {
      const iconCell = q(`[data-usage-icon="${vendor}"]`, summary())!.parentElement!
      expect(iconCell.parentElement).toBe(grid)
      expect(`${iconCell.style.gridRow}:${iconCell.style.gridColumn}`).toBe(`${row}:1`)
      expect(grid.contains(q(`[data-usage-vendor="${vendor}"][data-usage-window="weekly"]`, summary()))).toBe(true)
    }
    // The same cell of the same window is the same column, and a different row for each company.
    for (const window of ['five_hour', 'weekly']) {
      const a = cells('claude', window)
      const b = cells('codex', window)
      expect(a.map(c => c.style.gridColumn)).toEqual(b.map(c => c.style.gridColumn))
      expect(a.every(c => c.style.gridRow === '1')).toBe(true)
      expect(b.every(c => c.style.gridRow === '2')).toBe(true)
    }
    expect(cells('claude', 'five_hour').map(c => c.style.gridColumn)).toEqual(['2', '3', '4', '5'])
    expect(cells('claude', 'weekly').map(c => c.style.gridColumn)).toEqual(['6', '7', '8', '9'])
    // The % cell is right-aligned — the ends line up even if 1% and 62% have different digit counts.
    for (const vendor of ['claude', 'codex']) {
      expect(valueOf(vendor, 'five_hour')!.parentElement!.style.textAlign).toBe('right')
    }
    expect(valueOf('claude', 'five_hour')!.textContent).toBe('1%')
  })

  it('stage 1 — the bar↔% gap is narrower than between other cells (the grid gap as is, the rest add margin)', () => {
    seed(snap('claude'))
    mount(true, false)
    const grid = q('[data-usage-grid]', summary())!
    expect(grid.style.columnGap).toBe('4px')
    const [label, bar, pct, reset] = cells('claude', 'five_hour')
    expect(pct.style.marginLeft).toBe('')
    for (const cell of [label, bar, reset]) expect(parseFloat(cell.style.marginLeft)).toBeGreaterThan(0)
  })

  it('stage 1 — an expired window\'s one cell 「reset — waiting for refresh」 covers the three cells bar·%·reset · a window carrying only a reset is 「—」 + time', () => {
    seed(
      snap('claude', {
        five_hour: { used_pct: 38, resets_at: NOW, age_secs: 0, expired: false },
        weekly: { used_pct: null, resets_at: NOW + 86_400, age_secs: 0, expired: false },
      }),
    )
    mount(true, false)
    const expired = cells('claude', 'five_hour')
    expect(expired).toHaveLength(2)
    expect(expired[1].style.gridColumn).toBe('3 / span 3')
    const resetOnly = cells('claude', 'weekly')
    expect(resetOnly.map(c => c.style.gridColumn)).toEqual(['6', '8', '9'])
    expect(resetOnly[1].textContent).toBe('—')
    expect(resetOnly[2].textContent).toBe(resetText(NOW + 86_400))
  })

  it('stage 2 — one line per window in one grid (icon | name·bar·%·reset), the icon spans that company\'s two lines', () => {
    seed(snap('claude'))
    seed(snap('codex'))
    mount()
    toStage(2)
    const grids = summary().querySelectorAll<HTMLElement>('[data-usage-grid]')
    expect(grids).toHaveLength(1)
    expect(tracks(grids[0].style.gridTemplateColumns)).toHaveLength(5)
    expect(q('[data-usage-icon="claude"]', summary())!.parentElement!.style.gridRow).toBe('1 / span 2')
    expect(q('[data-usage-icon="codex"]', summary())!.parentElement!.style.gridRow).toBe('3 / span 2')
    const rows = (vendor: string) =>
      ['five_hour', 'weekly'].map(w => cells(vendor, w).map(c => `${c.style.gridRow}:${c.style.gridColumn}`))
    expect(rows('claude')).toEqual([
      ['1:2', '1:3', '1:4', '1:5'],
      ['2:2', '2:3', '2:4', '2:5'],
    ])
    expect(rows('codex')).toEqual([
      ['3:2', '3:3', '3:4', '3:5'],
      ['4:2', '4:3', '4:4', '4:5'],
    ])
  })
})

// ── Popup as one table (ADR-0261 · TRD §3 #108) — jsdom does not compute layout, so assert by structure · column number ──
describe('popup one table', () => {
  /** The columns (DOM order) of the cells whose column position is pinned in a popup window row. */
  function columns(row: HTMLElement): string[] {
    return [...row.querySelectorAll<HTMLElement>('*')].filter(el => el.style.gridColumn !== '').map(el => el.style.gridColumn)
  }

  it('both companies\' window lines are in one table · the table is tabular-nums · value line = name 1 · bar 2 · % 3 (right-aligned) · reset 4 · remaining time 5', () => {
    seed(snap('claude', { model_scoped: [{ label: 'Fable', window: { used_pct: 0, resets_at: NOW + 86_400 * 3, age_secs: 0, expired: false } }] }))
    seed(snap('codex', { model_scoped: [{ label: 'gpt-reserve', window: { used_pct: 0, resets_at: NOW + 86_400 * 7, age_secs: 0, expired: false } }] }))
    mount()
    const popup = openPopup()
    const tables = popup.querySelectorAll<HTMLElement>('[data-usage-popup-table]')
    expect(tables).toHaveLength(1)
    const table = tables[0]
    expect(table.style.display).toBe('grid')
    expect(table.style.fontVariantNumeric).toBe('tabular-nums')
    const rows = [...popup.querySelectorAll<HTMLElement>('[data-usage-window]')]
    expect(rows.map(r => `${r.getAttribute('data-usage-vendor')}:${r.getAttribute('data-usage-window')}`)).toEqual([
      'claude:five_hour',
      'claude:weekly',
      'claude:model:Fable',
      'codex:five_hour',
      'codex:weekly',
      'codex:model:gpt-reserve',
    ])
    for (const row of rows) {
      expect(row.closest('[data-usage-popup-table]')).toBe(table)
      expect(columns(row)).toEqual(['1', '2', '3', '4', '5'])
      const pct = q('[data-usage-value]', row)!.parentElement!
      expect(pct.style.gridColumn).toBe('3')
      expect(pct.style.textAlign).toBe('right')
    }
  })

  it('a line without a value keeps % at column 3 as is · a line without a reset has columns 4·5 empty · an expired line covers from column 2 to the end', () => {
    seed(
      snap('claude', {
        five_hour: { used_pct: null, resets_at: NOW + 600, age_secs: 0, expired: false },
        weekly: { used_pct: 59, resets_at: null, age_secs: 0, expired: false },
      }),
    )
    seed(snap('codex', { five_hour: { used_pct: 38, resets_at: NOW, age_secs: 0, expired: false } }))
    mount()
    const popup = openPopup()
    const row = (vendor: string, window: string) => q(`[data-usage-vendor="${vendor}"][data-usage-window="${window}"]`, popup)!
    const noValue = row('claude', 'five_hour')
    expect(columns(noValue)).toEqual(['1', '3', '4', '5'])
    expect(q('[data-usage-value]', noValue)!.textContent).toBe('—')
    expect(q('[data-usage-value]', noValue)!.parentElement!.style.gridColumn).toBe('3')
    expect(columns(row('claude', 'weekly'))).toEqual(['1', '2', '3'])
    const expired = row('codex', 'five_hour')
    expect(columns(expired)).toEqual(['1', '2 / -1'])
    expect(q('[data-usage-value]', expired)!.textContent).toBe('리셋됨 — 갱신 대기')
  })
})

describe('no ⚠ (R10 revision 2026-09-29)', () => {
  it('even under 20% there is no ⚠ anywhere in the small display (all stages · measuring copy) or the popup', () => {
    const low = { used_pct: 99.5, resets_at: NOW + 600, age_secs: 0, expired: false }
    seed(snap('claude', { five_hour: low, weekly: low, model_scoped: [{ label: 'Opus', window: low }] }))
    seed(snap('codex', { five_hour: low, weekly: low }))
    mount()
    for (const n of [1, 2, 3] as const) {
      toStage(n)
      expect(valueOf('claude', 'five_hour')!.getAttribute('data-usage-level')).toBe('danger')
      expect(document.body.textContent).not.toContain('⚠')
    }
    const popup = openPopup()
    expect(q('[data-usage-window="model:Opus"] [data-usage-value]', popup)!.textContent).toBe('0%')
    expect(document.body.textContent).not.toContain('⚠')
  })
})

describe('measuring copy and stage choosing', () => {
  it('the stage is chosen while the measuring copy draws the new shape (grid · icon · reset time)', () => {
    seed(snap('claude'))
    seed(snap('codex'))
    mount()
    const m1 = q('[data-usage-measure="1"]')!
    const m2 = q('[data-usage-measure="2"]')!
    expect(tracks((m1.firstElementChild as HTMLElement).style.gridTemplateColumns)).toHaveLength(9)
    expect(tracks((m2.firstElementChild as HTMLElement).style.gridTemplateColumns)).toHaveLength(5)
    for (const m of [m1, m2]) {
      expect(m.querySelectorAll('svg:not(.lucide)')).toHaveLength(2)
      expect(m.querySelectorAll('svg.lucide-refresh-cw')).toHaveLength(0)
      expect(m.querySelectorAll('svg.lucide-hourglass')).toHaveLength(4)
      expect(m.textContent).toContain(resetText(NOW + 86_400 * 3))
    }
    setWidths(300, 400, 250)
    expect(stage()).toBe('2')
    setWidths(240, 400, 250)
    expect(stage()).toBe('3')
    setWidths(420, 400, 250)
    expect(stage()).toBe('1')
  })

  it('observed targets = slot root · ⟳ spot · two measuring copies — a measuring copy has the same shape as what is visible at that stage', () => {
    seed(snap('claude', { state: { kind: 'NeedsLogin', detail: null } }))
    seed(snap('codex', { five_hour: { used_pct: null, resets_at: NOW + 600, age_secs: 0, expired: false } }))
    mount()
    const m = { 1: q('[data-usage-measure="1"]')!, 2: q('[data-usage-measure="2"]')! }
    const live = FakeResizeObserver.instances.filter(o => !o.disconnected)
    expect(live[0].observed).toEqual([q('[data-usage-slot]'), q('[data-usage-refresh-area]'), m[1], m[2]])
    for (const n of [1, 2] as const) {
      toStage(n)
      const shown = q('[data-usage-content]')!
      const shownGrid = shown.firstElementChild as HTMLElement
      const copyGrid = m[n].firstElementChild as HTMLElement
      expect(copyGrid.style.gridTemplateColumns).toBe(shownGrid.style.gridTemplateColumns)
      expect(m[n].textContent).toBe(shown.textContent)
      expect(m[n].querySelectorAll('svg:not(.lucide)')).toHaveLength(shown.querySelectorAll('svg:not(.lucide)').length)
      expect(m[n].querySelectorAll('svg.lucide-hourglass')).toHaveLength(shown.querySelectorAll('svg.lucide-hourglass').length)
      const placed = shown.querySelectorAll('[style*="grid-column"]').length
      expect(placed).toBeGreaterThan(0)
      expect(m[n].querySelectorAll('[style*="grid-column"]')).toHaveLength(placed)
    }
  })

  it('no height revert — even if the stage-2 grid is taller than the slot it stays stage 2 (two companies · badge next to the icon · ADR-0259 decision 6)', () => {
    seed(snap('claude', { state: { kind: 'NeedsLogin', detail: null } }))
    seed(snap('codex', { state: { kind: 'NotInstalled', detail: null } }))
    mount()
    setWidths(300, 400, 250, { rootH: 68, s2h: 60 })
    expect(stage()).toBe('2')
    setWidths(300, 400, 250, { rootH: 67, s2h: 60 })
    expect(stage()).toBe('2')
    const shown = q('[data-usage-content]')!
    expect(q('[data-usage-grid]', shown)).not.toBeNull()
    expect(shown.querySelectorAll('[data-usage-reset-clock]')).toHaveLength(4)
    for (const vendor of ['claude', 'codex']) {
      const badge = q(`[data-usage-badge="${vendor}"]`, shown)!
      expect(badge.previousElementSibling).toBe(q(`[data-usage-icon="${vendor}"]`, shown))
    }
  })
})

// ── 「usage page」 destination = shell opener allow list — if even one query or trailing slash differs the shell refuses to open it ──
describe('usage page address = shell allow list (src-tauri/capabilities/usage-links.json)', () => {
  it('per-company address equals the allow list down to the character · no address remains in the list either', () => {
    const cap = JSON.parse(usageLinksSource) as {
      permissions: Array<string | { identifier: string; allow?: Array<{ url: string }> }>
    }
    const grant = cap.permissions.find(
      (p): p is { identifier: string; allow?: Array<{ url: string }> } =>
        typeof p === 'object' && p.identifier === 'opener:allow-open-url',
    )
    const allowed = (grant?.allow ?? []).map(a => a.url).sort()
    expect(Object.values(USAGE_PAGE_URL).sort()).toEqual(allowed)
  })
})
