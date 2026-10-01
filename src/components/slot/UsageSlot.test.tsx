// UsageSlot — 사용량 슬롯 화면(TRD S21 usage-limit-slot §4 「프론트 표시」 행 중 컴포넌트 몫).
//
// 스토어는 실물(`usageStore`)이고 값은 `setState` 로 심는다 — 받기 규칙(merge)은 `usageStore.test.ts` 가 잰다.
// command 도 실물(`usageSlot.*` 등록)이다 — ⟳·표시 토글이 command 를 거쳐 스토어·레이아웃 쓰기에 닿는 것까지 잰다.
// 시계는 둘 다 가짜다: 벽시계(`Date`)는 가짜 타이머, 받은 시각 기준 시계(`performance.now`)는 spy.
// jsdom 엔 레이아웃이 없어 폭은 가짜 ResizeObserver 로 직접 준다(슬롯 폭 · ⟳ 자리 폭 + 두 단계의 자연 폭).

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

// 표시 토글은 command → viewStore → 셸 invoke 까지 실물로 흐르고, 그 끝의 invoke 인자로 잰다(`usageCommands.test` 와 같다).
const invokeMock = vi.hoisted(() => vi.fn(async (_cmd: string, _args?: unknown) => undefined as unknown))
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => vi.fn()) }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ close: vi.fn(async () => undefined), label: () => 'main' }),
}))

// `?raw` 인 이유 = `index.css.test.ts` 와 같다(@types/node 가 없어 fs 를 쓰면 tsc 게이트가 깨진다).
import usageLinksSource from '../../../src-tauri/capabilities/usage-links.json?raw'
import '../../commands/usageCommands' // side-effect: usageSlot.* 등록
import { useUsageStore } from '../../store/usageStore'
import UsageSlot, { USAGE_PAGE_URL } from './UsageSlot'
import { ANCHOR_GAP } from './SlotContextMenu'
import { formatAge, formatClock, formatResetAt, STALE_AFTER_SECS, statusLine, statusSentence } from './usageFormat'

// theme.css 원문을 Node fs 로 읽는다 — 까닭과 선언 모양은 `store/chatStyleStore.test.ts` 와 같다.
declare function require(id: string): { readFileSync(p: string, enc: string): string }
declare const process: { cwd(): string }

const NOW = 1_900_000_000 // 벽시계(초)
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

/** 작은 표시의 ⟳(요약 버튼 밖). */
function refreshButton(): HTMLButtonElement | null {
  return q('[data-usage-slot] [data-usage-refresh]') as HTMLButtonElement | null
}

function valueOf(vendor: string, window: string, root: ParentNode = summary()): HTMLElement | null {
  return q(`[data-usage-vendor="${vendor}"][data-usage-window="${window}"] [data-usage-value]`, root)
}

/** ⟳ 자리의 폭(⟳ + 오른쪽 여백)과 요약 버튼의 좌우 여백 합 — 실물 값에 맞춘 가짜 크기. */
const REFRESH_AREA_W = 26
const SUMMARY_PAD_X_TOTAL = 12

function liveObserver(): FakeResizeObserver {
  const live = FakeResizeObserver.instances.filter(o => !o.disconnected)
  expect(live).toHaveLength(1)
  return live[0]
}

/** 관찰자가 받는 크기를 그대로 준다 — 슬롯 루트 · ⟳ 자리 · 1·2단 측정 사본. */
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
 * 내용이 쓸 폭(`avail`)과 1·2단 자연 폭을 준다 — 슬롯 폭은 `avail` + ⟳ 자리 + 요약 버튼 여백으로 거꾸로 세운다.
 * 높이는 기본으로 넉넉하다(슬롯 1000 · 2단 30) — 단계는 높이를 보지 않지만(ADR-0259) 관찰자는 높이도 싣는다.
 */
function setWidths(avail: number, s1: number, s2: number, { rootH = 1000, s2h = 30 } = {}): void {
  const refreshW = refreshButton() ? REFRESH_AREA_W : 0
  fireSizes({ rootW: avail + refreshW + SUMMARY_PAD_X_TOTAL, rootH, refreshW, s1, s2, s2h })
}

/**
 * 리셋 시각의 보이는 글자 — 포맷터와 따로 세운 기대값(`Date` 의 로컬 getter). 지금(`NOW`)과 같은 로컬 날짜면 「HH:MM」,
 * 아니면 「M/D HH:MM」. 시간대에 따라 NOW + 몇 시간이 자정을 넘을 수 있어 날짜를 늘 직접 견준다.
 */
function resetText(epochSecs: number): string {
  const d = new Date(epochSecs * 1000)
  const now = new Date(NOW * 1000)
  const time = `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
  const sameDay =
    d.getFullYear() === now.getFullYear() && d.getMonth() === now.getMonth() && d.getDate() === now.getDate()
  return sameDay ? time : `${d.getMonth() + 1}/${d.getDate()} ${time}`
}

/** 리셋 시각의 이름(툴팁·보조기술) — 보이는 자리엔 낱말 대신 모래시계가 선다(ADR-0259). */
function resetName(epochSecs: number): string {
  return `리셋 ${resetText(epochSecs)}`
}

/** 팝업 창 줄의 리셋 자리 — 모래시계 표식(툴팁 = 이름)과 그 뒤의 남은 시간. */
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

/** 팝업의 회사 머리 — 회사 섹션의 첫 줄. */
function popupHead(popup: ParentNode, vendor: string): HTMLElement {
  return q(`[data-usage-popup-vendor="${vendor}"]`, popup)!.firstElementChild as HTMLElement
}

function vendorRefresh(popup: ParentNode, vendor: string): HTMLButtonElement {
  return q(`[data-usage-vendor-refresh="${vendor}"]`, popup) as HTMLButtonElement
}

/** 격자 칸 목록 — `minmax(…, …)` 안의 공백은 칸을 가르지 않는다. */
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

// ── 마운트 · 켠 회사 ──
describe('마운트와 켠 회사', () => {
  it('마운트하면 셸 캐시를 한 번 당긴다(TRD §3 #82)', () => {
    mount()
    expect(client.getUsageSnapshot).toHaveBeenCalledTimes(1)
  })

  it('두 회사 다 꺼짐 → 안내 한 줄(R4) · ⟳ 없음 · 안내를 누르면 표시 토글 줄만 있는 팝업이 열린다', () => {
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

  it('끈 회사는 작은 표시·배지·팝업 어디에도 없다 — 스토어에 값이 있어도', () => {
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
    // 이름이 남는 자리는 표시 토글 줄뿐이다(다시 켤 자리) — 꺼진 채로(ADR-0259 결정 7).
    const toggles = q('[data-usage-show-toggles]', popup)!
    for (const child of popup.children) {
      if (child !== toggles) expect(child.textContent).not.toContain('Codex')
    }
    expect((q('[data-usage-show="codex"]', toggles) as HTMLInputElement).checked).toBe(false)
  })

  it('아직 한 장도 못 받은 회사 → 두 창 모두 회색 "—" · 배지 없음', () => {
    mount(true, false)
    expect(valueOf('claude', 'five_hour')!.textContent).toBe('—')
    expect(valueOf('claude', 'weekly')!.textContent).toBe('—')
    expect(q('[data-usage-badge]')).toBeNull()
    expect(q('[data-usage-bar]')).toBeNull()
    expect(screen.queryAllByRole('meter')).toHaveLength(0)
  })
})

// ── 값 · 색 · 막대 ──
describe('값·색·막대', () => {
  it('남은 양을 DOM 텍스트로(R27) — Claude 사용률 38 → 62%', () => {
    seed(snap('claude'))
    mount(true, false)
    expect(valueOf('claude', 'five_hour')!.textContent).toBe('62%')
    expect(valueOf('claude', 'weekly')!.textContent).toBe('41%')
  })

  it('작은 표시의 막대는 보조기술에서 숨기고 요약 버튼이 짧은 이름을 진다(버튼 자식은 평평해진다)', () => {
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

  it('막대 = meter(D14 — 팝업) — aria-valuenow 는 보이는 수', () => {
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

  it('색은 토큰만 — 구간별 채움·글자색 토큰, 20% 미만도 기호 없이 색만(R10 개정 2026-09-29)', () => {
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

  it('Unavailable 은 실린 값이 있어도 회색 "—"(값을 싣지 않는 상태)', () => {
    seed(snap('claude', { state: { kind: 'Unavailable', detail: null } }))
    mount(true, false)
    expect(valueOf('claude', 'five_hour')!.textContent).toBe('—')
    expect(valueOf('claude', 'weekly')!.textContent).toBe('—')
    expect(within(summary()).queryAllByRole('meter')).toHaveLength(0)
  })

  it('실패·거절이어도 들고 있는 값을 그린다(R22)', () => {
    seed(snap('claude', { state: { kind: 'Failed', next_attempt_in_secs: 60, detail: null } }))
    seed(snap('codex', { state: { kind: 'Rejected', retry_in_secs: 60, detail: null } }))
    mount()
    expect(valueOf('claude', 'five_hour')!.textContent).toBe('62%')
    expect(valueOf('codex', 'five_hour')!.textContent).toBe('62%')
  })
})

// ── 폭 단계(R3·D16) ──
describe('폭 단계', () => {
  it('들어가는 가장 넓은 단계 — 회사당 한 줄 → 창마다 한 줄 → 숫자만', () => {
    seed(snap('claude'))
    mount(true, false)
    expect(stage()).toBe('1') // 재기 전엔 가장 자세한 단계
    setWidths(400, 400, 200)
    expect(stage()).toBe('1')
    setWidths(399, 400, 200)
    expect(stage()).toBe('2')
    setWidths(200, 400, 200)
    expect(stage()).toBe('2')
    setWidths(199, 400, 200)
    expect(stage()).toBe('3')
    // 자연 폭이 줄면(값·문구가 바뀌어 숨은 렌더가 다시 재어지면) 다시 넓은 단계로.
    setWidths(199, 150, 100)
    expect(stage()).toBe('1')
  })

  it('리셋 시각은 1·2단에만 — 모래시계 + 「HH:MM」(다른 날이면 「M/D HH:MM」 · 요일·남은 시간·낱말 없음) · 이름 = 「리셋 …」, 3단은 숫자만(「%」 없이)', () => {
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
      // 낱말 「리셋」 은 툴팁에만 — DOM 글자에서는 빠진다(ADR-0259 결정 3).
      expect(summary().textContent).not.toMatch(/리셋|뒤|↻/)
    }
    toStage(3)
    expect(summary().querySelectorAll('[data-usage-reset-clock]')).toHaveLength(0)
    expect(summary().textContent).not.toContain('리셋')
    expect(q('[data-usage-numbers]')!.textContent).toBe('62·41')
    expect(within(summary()).queryAllByRole('meter')).toHaveLength(0)
  })

  it('3단보다 좁으면 숫자 쪽만 말줄임 — 줄바꿈·스크롤 없음, 이름·배지는 줄지 않는다', () => {
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

  it('숨은 측정 사본은 역할·값 표식을 싣지 않는다(cdp·보조기술이 두 번 읽지 않게)', () => {
    seed(snap('claude', { state: { kind: 'NeedsLogin', detail: null } }))
    mount(true, false)
    for (const n of ['1', '2']) {
      const m = q(`[data-usage-measure="${n}"]`)!
      expect(m.querySelector('[data-usage-vendor]')).toBeNull()
      expect(m.querySelector('[role]')).toBeNull()
      expect(m.querySelector('[aria-label]')).toBeNull()
      expect(m.querySelector('[title]')).toBeNull()
      expect(m.textContent).toContain('62%')
      // 보이는 쪽과 같은 모양 — 한 격자 · 아이콘 · 리셋 시각.
      expect((m.firstElementChild as HTMLElement).style.display).toBe('grid')
      expect(m.querySelectorAll('svg:not(.lucide)')).toHaveLength(1)
      expect(m.querySelectorAll('svg.lucide-refresh-cw')).toHaveLength(0)
      expect(m.querySelectorAll('svg.lucide-hourglass')).toHaveLength(2)
      expect(m.textContent).toContain(resetText(NOW + 7_980))
    }
    expect(q('[data-usage-measure="1"]')!.parentElement!.style.visibility).toBe('hidden')
  })
})

// ── 상태 배지(R31) ──
describe('상태 배지와 문장', () => {
  for (const [name, state] of FIVE_STATES) {
    it(`${name} — 모든 폭 단계에서 이름 옆 「!」 배지 · aria-label = title = 팝업 맨 위 줄`, () => {
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

  it('Ready 는 배지가 없다', () => {
    seed(snap('claude'))
    mount(true, false)
    expect(q('[data-usage-badge]')).toBeNull()
    expect(q('[data-usage-status-line]', openPopup())).toBeNull()
  })

  it('다섯 상태 문구(한국어 · Unavailable 은 원인을 단정하지 않는다)', () => {
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

  it('상류 원문은 번역·정화 없이 그대로 · code 는 출처 뒤 괄호 · 회사 이름은 줄 머리에 한 번', () => {
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

// ── 오래됨·만료·갱신 중(R32) ──
describe('오래됨 · 만료 · 갱신 중', () => {
  it('30분 넘은 값 → 작은 표시는 숫자만 흐리게(나이 문구 없음) · 팝업은 흐리지 않고 회사 머리의 나이를 호박색으로', () => {
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
    // 흐리게 하는 것은 숫자뿐 — 막대·이름은 그대로.
    expect(q('[data-usage-vendor="claude"][data-usage-window="five_hour"] [data-usage-bar]')!.style.opacity).toBe('')
    expect(summary().textContent).not.toMatch(/분 전|방금/)

    const popup = openPopup()
    expect(q('[data-usage-value][data-usage-stale]', popup)).toBeNull()
    expect(q('[data-usage-window] [data-usage-age]', popup)).toBeNull()
    expect(popup.querySelectorAll('[data-usage-age]')).toHaveLength(1)
    // 머리의 나이 = 두 창 가운데 오래된 쪽(1801초) — 30분을 넘어 호박색이다(ADR-0259 결정 4).
    const age = q('[data-usage-age]', popupHead(popup, 'claude'))!
    expect(age.textContent).toBe('30분 전')
    expect(age.hasAttribute('data-usage-stale')).toBe(true)
    expect(age.style.color).toBe('var(--usage-stale)')
    expect(age.getAttribute('title')).toBe('30분 넘게 새로 들어오지 않은 값')
  })

  it('리셋 시각이 지나면(또는 데몬 래치) %·남은 시간을 숨기고 「리셋됨 — 갱신 대기」', () => {
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

  it('로컬 벽시계가 리셋 시각을 넘으면 요청 없이 만료로 바뀐다', () => {
    seed(snap('claude', { five_hour: { used_pct: 38, resets_at: NOW + 90, age_secs: 0, expired: false } }))
    mount(true, false)
    expect(valueOf('claude', 'five_hour')!.textContent).toBe('62%')
    advance(120_000)
    expect(valueOf('claude', 'five_hour')!.textContent).toBe('리셋됨 — 갱신 대기')
    expect(client.getUsageSnapshot).toHaveBeenCalledTimes(1)
  })

  it('조회 중 = ⟳ 대기 또는 스냅숏 in_flight — ⟳ 의 표식이 회사를 싣고, 값은 그대로(ADR-0261)', () => {
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

  it('나이·남은 시간·다음 시도가 분 tick 하나로 로컬로 흐른다(요청 없음)', () => {
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
    // 다음 시도 시각 자체는 같은 순간을 가리킨다(받은 순간 기준 상대 초 − 흐른 초 + 지금).
    expect(q('[data-usage-badge]')!.getAttribute('aria-label')).toBe(before)
    expect(client.getUsageSnapshot).toHaveBeenCalledTimes(1)
    expect(client.refreshUsageLimits).not.toHaveBeenCalled()
  })

  it('거절 대기 「N분 뒤」도 받은 뒤 흐른 만큼 준다', () => {
    seed(snap('claude', { state: { kind: 'Rejected', retry_in_secs: 720, detail: null } }))
    mount(true, false)
    expect(q('[data-usage-badge]')!.getAttribute('aria-label')).toBe('Claude 거절됨 — 12분 뒤')
    advance(5 * 60_000)
    expect(q('[data-usage-badge]')!.getAttribute('aria-label')).toBe('Claude 거절됨 — 7분 뒤')
  })
})

// ── 팝업(R5–R7·D8·D14) ──
describe('팝업', () => {
  it('요약 영역은 네이티브 버튼 — 클릭(Enter·Space 는 브라우저가 같은 click 으로 부른다)으로 열고 포커스를 옮긴다', () => {
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

  it('Esc 로 닫고 포커스를 요약 버튼으로 돌린다', () => {
    seed(snap('claude'))
    mount(true, false)
    openPopup()
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(document.activeElement).toBe(summary())
  })

  it('바깥 누름으로 닫힌다 · 팝업 안 누름·클릭은 닫지 않는다 · 요약 버튼을 다시 누르면 닫힌다', () => {
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

  it('창별 절대 리셋 시각 · 나이는 회사 머리에 한 번(창 줄엔 없음) · plan 은 있을 때만 · 모델별 창은 값이 있는 것만', () => {
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

  it('다른 날의 리셋 시각은 「M/D HH:MM」(요일 없음) — 작은 표시와 같은 규칙', () => {
    seed(snap('claude'))
    mount(true, false)
    const popup = openPopup()
    const weekly = popupReset(q('[data-usage-window="weekly"]', popup)!)
    expect(weekly.text).toBe(resetText(NOW + 86_400 * 3))
    expect(weekly.text).toMatch(/^\d{1,2}\/\d{1,2} \d{2}:\d{2}$/)
    expect(weekly.name).toBe(resetName(NOW + 86_400 * 3))
    expect(weekly.after).toBe('(3일 뒤)')
  })

  it('「사용량 페이지 ↗」 → openUrl 에 정확한 주소 · title 이 목적지를 말한다', () => {
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

  it('팝업엔 회사마다 ⟳ 하나 — 이름 「<회사> 사용량 새로고침」 · 작은 표시의 전역 ⟳ 는 팝업 밖에 그대로(ADR-0259 결정 2)', () => {
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

  it('보이는 거절 → 팝업 맨 위 「거절됨 — N분 뒤」', () => {
    seed(snap('claude', { state: { kind: 'Rejected', retry_in_secs: 600, detail: null } }))
    seed(snap('codex'))
    mount()
    const popup = openPopup()
    expect(q('[data-usage-status-line="claude"] [data-usage-status-text]', popup)!.textContent).toBe(
      'Claude 거절됨 — 10분 뒤',
    )
  })

  it('상세는 켠 회사만 — 끈 회사의 섹션·상태 줄은 없다', () => {
    seed(snap('claude'))
    seed(snap('codex', { state: { kind: 'NeedsLogin', detail: null } }))
    mount(true, false)
    const popup = openPopup()
    expect([...popup.querySelectorAll('[data-usage-popup-vendor]')].map(e => e.getAttribute('data-usage-popup-vendor'))).toEqual([
      'claude',
    ])
    expect(q('[data-usage-status-line]', popup)).toBeNull()
  })

  it('두 회사 비정상 → 팝업 맨 위에 회사별 한 줄씩, 각 줄 = 그 회사 배지 문장', () => {
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

// ── 팝업 회사 머리(ADR-0259 결정 1·2·4·5) — 「<아이콘> <이름> 플랜: max · 1분 전 ⟳」 ──
describe('팝업 회사 머리', () => {
  const REJECTED = { kind: 'Rejected' as const, retry_in_secs: 600, detail: null }

  it('머리 = 아이콘·이름 · 플랜 · 「·」 · 나이 + ⟳(붙어 있다) · 갱신 중이어도 글자를 더하지 않는다 — 시계 아이콘 없음', () => {
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
    // lucide 아이콘은 ⟳ 하나뿐 — 나이에 시계 아이콘을 붙이지 않는다.
    expect([...head.querySelectorAll('svg.lucide')].map(svg => svg.closest('button'))).toEqual([
      vendorRefresh(popup, 'claude'),
    ])
    act(() => useUsageStore.setState({ pending: { claude: 1 } }))
    expect(head.children).toHaveLength(4)
    expect(head.textContent).not.toContain('갱신 중')
    expect(vendorRefresh(popup, 'claude').getAttribute('data-usage-refreshing')).toBe('claude')
    expect(vendorRefresh(popup, 'claude').parentElement).toBe(group)
  })

  it('플랜이 없으면 이름 뒤에 곧바로 나이 — 「·」 도 없다', () => {
    seed(snap('codex'))
    mount(false, true)
    const head = popupHead(openPopup(), 'codex')
    expect(q('[data-usage-plan]', head)).toBeNull()
    expect(head.children).toHaveLength(2)
    expect(head.textContent).toBe('Codex방금')
  })

  it('나이 = 5시간·주간 가운데 가장 오래된 것 — 더 묵은 모델별 창은 넣지 않는다(호박색도 같다)', () => {
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

  it('한 창이 빠지면 남은 창의 나이 · 두 창 다 값이 없으면 나이 없음(모델별 창에 값이 있어도 · Unavailable 도) · ⟳ 는 남는다', () => {
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

  it('호박색 경계 — 1800초는 평소 색, 1801초는 호박색(`--usage-stale`) · 작은 표시의 흐림과 같은 상수', () => {
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
    // 작은 표시는 숫자만 흐리다 — 같은 창이 같은 순간에 오래됨으로 넘어간다.
    expect(valueOf('claude', 'five_hour')!.hasAttribute('data-usage-stale')).toBe(true)
  })

  it('분 tick 으로 30분을 넘으면 요청 없이 호박색이 된다', () => {
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

  it('회사별 ⟳ = 그 회사만 새로고침 · 팝업은 열린 채 · 포커스는 ⟳ 에 · 도는 동안 다시 누르면 버린다', async () => {
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
    // 그 회사만 「갱신 중」 — 다른 회사의 ⟳ 는 누를 수 있다.
    expect(vendorRefresh(popup, 'codex').getAttribute('aria-busy')).toBe('true')
    expect(vendorRefresh(popup, 'claude').hasAttribute('aria-busy')).toBe(false)
    fireEvent.click(vendorRefresh(popup, 'codex'))
    expect(client.refreshUsageLimits).toHaveBeenCalledTimes(1)
    await act(async () => {})
    expect(vendorRefresh(popup, 'codex').hasAttribute('aria-busy')).toBe(false)
  })

  it('보이는 거절이면 그 회사 ⟳ 만 aria-disabled(disabled 아님) · 누름은 command 에 닿지 않는다', () => {
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

  it('그 회사의 조회가 도는 동안(⟳ 대기 · 상대의 in_flight) 누름은 버린다 — 끝나면 다시 받는다', async () => {
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

  it('포커스된 회사 ⟳ 가 거절로 바뀌어도 포커스를 쥔 채 누름만 막힌다 · 팝업은 열린 채', () => {
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

  it('팝업을 열고 닫아도 조회 0 — pull 은 마운트 때 한 번뿐(ADR-0259 결정 5)', () => {
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

// ── 모래시계(ADR-0259 결정 3) — 낱말 「리셋」 은 이름에만 · 화살표 아이콘 없음 ──
describe('리셋 표기 = 모래시계', () => {
  const NOT_HOURGLASS = /lucide-(refresh|rotate|history|repeat|undo|redo|arrow|clock|alarm|calendar)/

  /** `scope` = 표식을 품은 요소 — 이름으로 찾아 같은 요소인지 본다(`title` 이 이름이 된다). */
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

  it('작은 표시(1·2단)와 팝업의 리셋 자리가 같은 모양 — 툴팁 「리셋 HH:MM」 이 곧 이름(aria-label 없음) · 아이콘은 보조기술에 숨김', () => {
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
      // 남은 시간은 이름을 진 표식 밖 — 보이는 글자로 남고(보조기술도 읽는다) 이름엔 들지 않는다.
      expect(reset.mark.contains(q('[data-usage-reset-in]', row))).toBe(false)
      expect(reset.name).not.toContain('뒤')
      // DOM 글자로 읽어도 시각과 남은 시간이 떨어져 있다(R27).
      expect(q('[data-usage-reset-at]', row)!.textContent).toBe(`${resetText(at)} ${reset.after}`)
    }
  })

  it('리셋 표식은 이름과 설명이 겹치지 않는다 — aria-label·title 을 함께 단 표식이 없다(작은 표시·팝업·모델별 창)', () => {
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
    // 작은 표시 두 창 + 팝업 세 줄(5시간·주간·Opus).
    expect(marks).toHaveLength(5)
    for (const mark of marks) {
      expect(mark.hasAttribute('title')).toBe(true)
      expect(mark.hasAttribute('aria-label')).toBe(false)
    }
  })

  it('↻ 모양(RefreshCw)은 ⟳ 버튼에만 — 리셋 자리에 화살표 아이콘이 없고 측정 사본에도 없다(ADR-0261)', () => {
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

// ── 작은 표시의 ⟳(사용자 결정 2026-09-29) — 켠 회사를 한 번에 · command 경로 · 팝업과 따로 ──
describe('작은 표시의 ⟳', () => {
  it('세 폭 단계 모두 격자 오른쪽에 하나 — 테두리 있는 아이콘 버튼 · 이름 「사용량 새로고침」', () => {
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

  it('요약 버튼 안에 들어가지 않는다 — 대화형 요소가 겹치지 않고, 요약 버튼 안엔 버튼·입력이 없다', () => {
    seed(snap('claude'))
    mount(true, false)
    for (const n of [1, 2, 3] as const) {
      toStage(n)
      expect(summary().contains(refreshButton())).toBe(false)
      expect(summary().querySelector('button, input, a, [tabindex]')).toBeNull()
    }
  })

  it('누르면 팝업을 열지 않는다', () => {
    seed(snap('claude'))
    mount(true, false)
    fireEvent.mouseDown(refreshButton()!)
    fireEvent.click(refreshButton()!)
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(summary().getAttribute('aria-expanded')).toBe('false')
  })

  it('누르면 command 가 켠 회사 중 보이는 거절이 아닌 것만 새로고침한다(끈 회사 · 거절 중인 회사는 빠진다)', () => {
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

  it('대상의 조회가 도는 동안의 누름은 버린다 — ⟳ 가 돌고, 답이 오면 다시 받는다', () => {
    seed(snap('claude'))
    seed(snap('codex'))
    mount()
    act(() => useUsageStore.setState({ pending: { codex: 1 } }))
    expect(refreshButton()!.getAttribute('data-usage-refreshing')).toBe('codex')
    expect(refreshButton()!.getAttribute('aria-busy')).toBe('true')
    fireEvent.click(refreshButton()!)
    expect(client.refreshUsageLimits).not.toHaveBeenCalled()
    // 상대가 스스로 도는 조회(in_flight)도 같다.
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

  it('켠 회사가 전부 보이는 거절이면 aria-disabled(disabled 아님) · 누름은 command 에 닿지 않는다', () => {
    const rejected = { kind: 'Rejected' as const, retry_in_secs: 600, detail: null }
    seed(snap('claude', { state: rejected }))
    seed(snap('codex', { state: rejected }))
    mount()
    const refresh = refreshButton()!
    expect(refresh.getAttribute('aria-disabled')).toBe('true')
    expect(refresh.disabled).toBe(false) // 포커스를 쥘 수 있게 남긴다
    const warn = vi.spyOn(console, 'warn')
    fireEvent.click(refresh)
    expect(client.refreshUsageLimits).not.toHaveBeenCalled()
    expect(warn).not.toHaveBeenCalled() // command 까지 가서 거절당한 것이 아니다
  })

  it('포커스된 ⟳ 가 거절로 바뀌어도 포커스를 쥔 채 누름만 막힌다', () => {
    seed(snap('claude'))
    mount(true, false)
    act(() => refreshButton()!.focus())
    act(() => seed(snap('claude', { revision: 2, state: { kind: 'Rejected', retry_in_secs: 600, detail: null } })))
    expect(refreshButton()!.getAttribute('aria-disabled')).toBe('true')
    expect(document.activeElement).toBe(refreshButton())
    fireEvent.click(refreshButton()!)
    expect(client.refreshUsageLimits).not.toHaveBeenCalled()
  })

  it('팝업이 열린 채 ⟳ 를 누르면 팝업은 그대로 · 새로고침은 된다 · ⟳ 에서 Esc 는 닫되 포커스는 ⟳ 에 둔다', () => {
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

  it('두 회사를 다 끄면 없다', () => {
    mount(false, false)
    expect(refreshButton()).toBeNull()
  })
})

// ── 팝업의 표시 토글 줄(사용자 결정 2026-09-29) ──
describe('팝업의 표시 토글 줄', () => {
  function toggles(popup: HTMLElement): HTMLInputElement[] {
    return within(q('[data-usage-show-toggles]', popup)!).getAllByRole('checkbox') as HTMLInputElement[]
  }
  /** 셸로 나간 표시 칸 쓰기의 인자 — 다른 invoke 는 거른다. */
  function usageWrites(): unknown[] {
    return invokeMock.mock.calls.filter(([cmd]) => cmd === 'set_usage_slot').map(([, args]) => args)
  }

  it('맨 아래 「슬롯에 표시:」 + 회사마다 이름 붙은 체크박스 — 켠 회사 ☑ · 끈 회사 ☐', () => {
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

  it('누르면 그 회사의 토글 command 가 슬롯 좌표로 돈다 — 그 칸을 뒤집은 값 하나만 쓴다', () => {
    mount(true, false)
    fireEvent.click(within(openPopup()).getByRole('checkbox', { name: 'Codex' }))
    expect(usageWrites()).toEqual([{ viewId: 'v1', slotId: 's1', showClaude: null, showCodex: true }])
  })

  it('반향 전의 다른 회사 연달은 누름 — Codex 켬 → Claude 끔이 칸 하나씩 두 번 가서 어느 반향 순서로도 Codex 만 남는다', () => {
    mount(true, false)
    const popup = openPopup()
    fireEvent.click(within(popup).getByRole('checkbox', { name: 'Codex' }))
    fireEvent.click(within(popup).getByRole('checkbox', { name: 'Claude' }))
    expect(usageWrites()).toEqual([
      { viewId: 'v1', slotId: 's1', showClaude: null, showCodex: true },
      { viewId: 'v1', slotId: 's1', showClaude: false, showCodex: null },
    ])
  })

  it('반향 뒤의 누름은 들어온 내용에서 뒤집는다', () => {
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

  it('쓰기가 거절되면 로그만 남고 체크는 슬롯 내용 그대로다', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    invokeMock.mockRejectedValueOnce(new Error('ipc 끊김'))
    mount(true, false)
    const popup = openPopup()
    const codex = () => within(popup).getByRole('checkbox', { name: 'Codex' }) as HTMLInputElement
    fireEvent.click(codex())
    await act(async () => {})
    expect(warn).toHaveBeenCalledWith(expect.stringMatching(/usageSlot\.toggleCodex/), expect.any(Error))
    expect(codex().checked).toBe(false)
  })

  it('체크는 슬롯 내용을 따른다 — 누름만으로 바뀌지 않고, 내용이 돌아오면 바뀐다 · 팝업은 열린 채', () => {
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

  it('팝업 안에서 마지막 회사를 꺼도 팝업·포커스가 남는다 — 작은 표시는 안내로, 팝업은 토글 줄만', () => {
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

  it('좌표를 모르면(viewId 없음) command 가 거절하고 로그만 남는다 — 레이아웃을 쓰지 않는다', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    render(<UsageSlot content={usage(true, false)} viewId={null} slotId="s1" />)
    fireEvent.click(within(openPopup()).getByRole('checkbox', { name: 'Codex' }))
    expect(usageWrites()).toEqual([])
    expect(warn).toHaveBeenCalledWith(expect.stringMatching(/usageSlot\.toggleCodex/), expect.any(Error))
  })
})

// ── 단계 고르기와 ⟳ — ⟳ 는 내용 바로 옆 · 단계 판정의 폭은 슬롯 폭 − ⟳ 자리 ──
describe('단계 측정과 ⟳', () => {
  it('줄 = 요약 버튼 · ⟳ 자리(줄지 않음) 둘뿐 — ⟳ 는 측정 사본·요약 버튼 안에 없다(넣으면 ⟳ 몫을 두 번 뺀다)', () => {
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

  it('⟳ 는 모든 단계에서 내용 바로 옆 — 1·2단은 막대 칸(1fr)이 요약 버튼을 채우고, 3단은 줄이 내용 폭으로 줄어든다(슬롯 폭 상한)', () => {
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
    // 줄이 줄어도 요약 버튼은 슬롯보다 좁아질 수 있어야 한다 — 내용을 자르고 ⟳ 자리는 남긴다.
    expect(parseFloat(summary().style.minWidth)).toBe(0)
    expect(q('[data-usage-content]')!.style.overflow).toBe('hidden')
    toStage(1)
    expect(row.style.width).toBe('')
  })

  it('단계 판정 폭 = 슬롯 폭 − ⟳ 자리 − 요약 버튼 여백 — ⟳ 자리가 넓어지면 문턱도 움직인다', () => {
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

  it('고른 단계가 제 입력으로 되돌아오지 않는다 — 3단으로 줄어든 내용 폭을 재지 않아, 넓어지면 1단으로 돌아간다', () => {
    seed(snap('claude'))
    seed(snap('codex'))
    mount()
    // 단계에 따라 폭이 바뀌는 요소(줄 · 요약 버튼 · 보이는 내용)는 관찰 대상이 아니다.
    const observed = liveObserver().observed
    for (const sel of ['[data-usage-row]', '[data-usage-summary]', '[data-usage-content]']) {
      expect(observed).not.toContain(q(sel))
    }
    const wide = { rootW: 500, rootH: 1000, refreshW: REFRESH_AREA_W, s1: 400, s2: 200, s2h: 30 }
    fireSizes(wide)
    expect(stage()).toBe('1')
    fireSizes({ ...wide, rootW: 120 })
    expect(stage()).toBe('3')
    // 3단 그림으로 다시 배치된 뒤 같은 크기를 또 알려도 3단에 머문다(오가지 않는다).
    fireSizes({ ...wide, rootW: 120 })
    expect(stage()).toBe('3')
    // 관찰 대상이 아닌 요소(3단에서 줄어든 내용)의 알림은 판정에 들지 않는다.
    act(() => liveObserver().fire([[q('[data-usage-content]')!, 40]]))
    expect(stage()).toBe('3')
    fireSizes(wide)
    expect(stage()).toBe('1')
    act(() => liveObserver().fire([[q('[data-usage-content]')!, 40]]))
    expect(stage()).toBe('1')
  })

  it('안내 문구(켠 회사 없음) — ⟳ 자리는 비어 있고 폭 0 · 줄은 슬롯 폭을 채워 안내 전체가 누르는 자리다', () => {
    mount(false, false)
    const area = q('[data-usage-refresh-area]')!
    expect(area.children).toHaveLength(0)
    expect(liveObserver().observed).toContain(area)
    fireSizes({ rootW: 300, rootH: 1000, refreshW: 0, s1: 0, s2: 0, s2h: 0 })
    expect(stage()).toBe('1')
    expect(q('[data-usage-row]')!.style.width).toBe('')
    expect(summary().style.flexGrow).toBe('1')
  })

  it('조회가 켜지고 꺼져도 작은 표시 내용·측정 사본이 같다 — 조회 중 표시는 ⟳ 의 회전뿐(ADR-0261)', () => {
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

  it('단계를 오가도 ⟳ 는 같은 요소로 남는다(폭이 단계에 따라 달라지지 않는다)', () => {
    seed(snap('claude'))
    mount(true, false)
    const first = refreshButton()
    for (const n of [3, 2, 1, 3] as const) {
      toStage(n)
      expect(refreshButton()).toBe(first)
    }
  })
})

// ── 낮은 슬롯(단계는 폭으로만) · 리셋만 실린 창 · 팝업 자리 · 포커스 · Esc · 회전 ──
function rect(left: number, top: number, width: number, height: number): DOMRect {
  return { left, top, width, height, right: left + width, bottom: top + height, x: left, y: top, toJSON() {} } as DOMRect
}

/** 요약·팝업의 화면 사각형을 준다(jsdom 엔 레이아웃이 없다). */
function stubRects(summaryRect: DOMRect, popupRect: DOMRect): void {
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.hasAttribute('data-usage-popup')) return popupRect
    if (this.hasAttribute('data-usage-summary')) return summaryRect
    return rect(0, 0, 0, 0)
  })
}

describe('높이 · 리셋만 실린 창', () => {
  it('단계는 폭으로만 — 2단이 슬롯보다 키가 커도 폭이 허락하면 2단이고 슬롯 루트가 아래를 자른다(ADR-0259 결정 6)', () => {
    seed(snap('claude', { state: { kind: 'NeedsLogin', detail: null } }))
    mount(true, false)
    setWidths(300, 400, 200, { rootH: 53, s2h: 45 })
    expect(stage()).toBe('2')
    for (const rootH of [52, 20, 1]) {
      setWidths(300, 400, 200, { rootH, s2h: 45 })
      expect(stage()).toBe('2')
    }
    expect(q('[data-usage-slot]')!.style.overflow).toBe('hidden')
    // 잘려 보이지 않아도 배지는 DOM 에 그대로 있다(받아들인 대가 — 가려질 뿐 사라지지 않는다).
    expect(q('[data-usage-badge="claude"]')).not.toBeNull()
    setWidths(500, 400, 200, { rootH: 10, s2h: 45 })
    expect(stage()).toBe('1')
    setWidths(199, 400, 200, { rootH: 1000, s2h: 45 })
    expect(stage()).toBe('3')
  })

  it('리셋 시각만 실린 창 — 막대 없이 「—」 + 리셋 시각(1·2단) · 팝업엔 절대 리셋 시각(창 줄엔 나이 없음)', () => {
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

describe('팝업 자리 · 포커스 · Esc', () => {
  it('아래에 들어가면 요약 아래로 편다', () => {
    seed(snap('claude'))
    mount(true, false)
    stubRects(rect(10, 10, 300, 40), rect(0, 0, 240, 200))
    const popup = openPopup()
    expect(popup.style.top).toBe(`${50 + ANCHOR_GAP}px`)
  })

  it('아래에 안 들어가면 요약의 위쪽 변에서 위로 편다 — 요약을 덮지 않는다', () => {
    seed(snap('claude'))
    mount(true, false)
    const vh = window.innerHeight
    const summaryTop = vh - 100
    stubRects(rect(10, summaryTop, 300, 60), rect(0, 0, 240, 200))
    const popup = openPopup()
    expect(popup.style.top).toBe(`${summaryTop - ANCHOR_GAP - 200}px`)
    expect(parseFloat(popup.style.top) + 200).toBeLessThanOrEqual(summaryTop)
  })

  it('열린 채 요약이 커지면(회사를 켜 줄이 늚) 다시 재어 늘어난 요약도 덮지 않는다', () => {
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

  it('포커스는 자리를 잡아 보인 뒤에만 옮긴다 — Chromium 은 보이지 않는 요소의 focus 를 거절한다', () => {
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

  it('포커스가 팝업·요약 밖으로 나가면 닫고, 포커스는 간 자리에 둔다', () => {
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

  it('Esc 는 포커스가 팝업·요약에 있을 때 닫는다', () => {
    seed(snap('claude'))
    mount(true, false)
    openPopup()
    act(() => summary().focus())
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(document.activeElement).toBe(summary())
  })

  it('포커스가 아무 데도 없을 때(body)도 Esc 로 닫고 포커스를 요약으로 돌린다 — 팝업 안의 포커스가 빠진 경우', () => {
    seed(snap('claude'))
    mount(true, false)
    const popup = openPopup()
    act(() => popup.blur())
    expect(document.activeElement).toBe(document.body)
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(document.activeElement).toBe(summary())
  })

  it('조회 중엔 ⟳ 가 돌고, 조회가 끝나도 그 바퀴의 경계(animationiteration)까지는 돈다 · 꼬리 중 누름은 새 조회 + 처음부터(ADR-0261)', async () => {
    seed(snap('claude', { in_flight: true }))
    mount(true, false)
    const svg = () => q('svg.lucide-refresh-cw', refreshButton()!)!
    // jsdom 엔 AnimationEvent 가 없어 React 가 접두 붙은 이름으로 듣는다 — 두 이름을 다 쏜다.
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
    // 조회가 도는 동안의 경계에선 멈추지 않는다.
    roundEnds()
    expect(svg().getAttribute('class')).toContain('animate-spin')
    // 꼬리 — 조회는 끝났고 돌던 바퀴를 마저 돈다. 조회 중이 아니므로 속성·aria-busy 가 없다.
    act(() => seed(snap('claude', { revision: 2 })))
    expectTail()
    roundEnds()
    expect(svg().getAttribute('class')).not.toContain('animate-spin')

    // 경계 전에 누르면 = 보통 누름 — 명령 하나 · 아이콘을 새로 붙여 처음부터.
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

  it('누르면 돌기 시작한다 · 조회 중 재누름은 조회 없이 회전만 처음부터(아이콘 교체) · 거절이면 돌지 않는다(ADR-0261)', () => {
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

// ── 작은 표시 개편(사용자 결정 2026-09-29) — 회사 아이콘 · 한 격자 세로 정렬 · ⚠ 없음 ──
describe('회사 아이콘', () => {
  it('모든 폭 단계에서 이름 대신 아이콘 — role=img · aria-label = title = 회사 이름 · 색은 회사 토큰', () => {
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
      // 이름 글자는 작은 표시에서 빠졌다 — 요약 버튼의 접근성 이름만 이름을 진다.
      expect(summary().textContent).not.toMatch(/Claude|Codex|Cl|Cx/)
      expect(summary().getAttribute('aria-label')).toMatch(/^Claude .* · Codex /)
    }
  })

  it('모양 = 자리표시 도형 — Claude 는 여덟 갈래 햇살 + 가운데 점, Codex 는 육각 윤곽 + 가운데 점', () => {
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

  it('배지는 모든 폭 단계에서 아이콘 바로 옆 — 두 회사 모두, 3단에서도 숫자 말줄임 밖', () => {
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

  it('팝업은 아이콘 + 회사 이름 글자 — 아이콘은 장식(이름을 두 번 읽지 않게)', () => {
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

  it('사용량 색 토큰은 세 테마 모두 바탕 토큰만 가리킨다(리터럴 없음 — 색 프리셋 대비) · e-ink 바탕은 본문색', () => {
    const css = require('node:fs').readFileSync(`${process.cwd()}/src/styles/theme.css`, 'utf8')
    // 테마 줄마다 선언 블록 둘 — 바탕(`--bg` 로 시작) 과 사용량(`--usage-ok` 로 시작).
    const block = (theme: string, first: string): string => {
      const m = css.match(new RegExp(`:root\\[data-theme='${theme}'\\]\\s*\\{\\s*${first}[^}]*\\}`))
      expect(m, `${theme} 의 ${first} 블록이 없다`).not.toBeNull()
      return m![0]
    }
    for (const theme of ['dark', 'light', 'e-ink']) {
      const usage = block(theme, '--usage-ok')
      expect(usage).toContain('--usage-danger: var(--status-danger);')
      expect(usage).toContain('--usage-vendor-claude: var(--vendor-claude);')
      expect(usage).toContain('--usage-vendor-codex: var(--vendor-codex);')
      // 팝업 머리의 오래된 나이(호박색 — ADR-0259 결정 4).
      expect(usage).toContain('--usage-stale: var(--status-blocked);')
      expect(usage, `${theme} 사용량 줄에 색 리터럴`).not.toMatch(/#[0-9a-fA-F]{3,8}\b/)
      const base = block(theme, '--bg')
      for (const token of ['--status-danger', '--status-blocked', '--vendor-claude', '--vendor-codex']) {
        expect(base, `${theme} 바탕에 ${token} 가 없다`).toMatch(new RegExp(`${token}:\\s*[^;]+;`))
      }
    }
    const eink = block('e-ink', '--bg')
    // e-ink 의 호박색은 본문색이다 — 색 단서 없이 흐린 글자보다 진해질 뿐이다(TRD §6 #30 미정).
    for (const token of ['--status-danger', '--status-blocked', '--vendor-claude', '--vendor-codex']) {
      expect(eink).toContain(`${token}: var(--text);`)
    }
  })
})

describe('한 격자 세로 정렬', () => {
  /** 한 창의 칸(= 값 표식을 단 `display: contents` 의 자식들). */
  function cells(vendor: string, window: string): HTMLElement[] {
    const seg = q(`[data-usage-vendor="${vendor}"][data-usage-window="${window}"]`, summary())!
    expect(seg.style.display).toBe('contents')
    return [...seg.children] as HTMLElement[]
  }

  it('1단 — 두 회사 줄이 한 격자의 같은 아홉 칸을 나눠 쓴다(아이콘 | 이름·막대·%·리셋 × 2)', () => {
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
    // 같은 창의 같은 칸은 같은 열, 회사마다 다른 줄.
    for (const window of ['five_hour', 'weekly']) {
      const a = cells('claude', window)
      const b = cells('codex', window)
      expect(a.map(c => c.style.gridColumn)).toEqual(b.map(c => c.style.gridColumn))
      expect(a.every(c => c.style.gridRow === '1')).toBe(true)
      expect(b.every(c => c.style.gridRow === '2')).toBe(true)
    }
    expect(cells('claude', 'five_hour').map(c => c.style.gridColumn)).toEqual(['2', '3', '4', '5'])
    expect(cells('claude', 'weekly').map(c => c.style.gridColumn)).toEqual(['6', '7', '8', '9'])
    // % 칸은 오른쪽 정렬 — 1% 와 62% 의 자리 수가 달라도 끝이 맞는다.
    for (const vendor of ['claude', 'codex']) {
      expect(valueOf(vendor, 'five_hour')!.parentElement!.style.textAlign).toBe('right')
    }
    expect(valueOf('claude', 'five_hour')!.textContent).toBe('1%')
  })

  it('1단 — 막대↔% 틈이 다른 칸 사이보다 좁다(격자 틈 그대로, 나머지는 여백을 더한다)', () => {
    seed(snap('claude'))
    mount(true, false)
    const grid = q('[data-usage-grid]', summary())!
    expect(grid.style.columnGap).toBe('4px')
    const [label, bar, pct, reset] = cells('claude', 'five_hour')
    expect(pct.style.marginLeft).toBe('')
    for (const cell of [label, bar, reset]) expect(parseFloat(cell.style.marginLeft)).toBeGreaterThan(0)
  })

  it('1단 — 만료 창은 「리셋됨 — 갱신 대기」 한 칸이 막대·%·리셋 세 칸을 덮는다 · 리셋만 실린 창은 「—」 + 시각', () => {
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

  it('2단 — 한 격자에 창마다 한 줄(아이콘 | 이름·막대·%·리셋), 아이콘은 그 회사의 두 줄에 걸친다', () => {
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

// ── 팝업 한 표(ADR-0261 · TRD §3 #108) — jsdom 은 배치를 계산하지 않아 구조·열 번호로 단언한다 ──
describe('팝업 한 표', () => {
  /** 팝업 창 줄에서 열 자리를 박은 칸들의 열(DOM 순서). */
  function columns(row: HTMLElement): string[] {
    return [...row.querySelectorAll<HTMLElement>('*')].filter(el => el.style.gridColumn !== '').map(el => el.style.gridColumn)
  }

  it('두 회사의 창 줄이 한 표에 든다 · 표는 tabular-nums · 값 줄 = 이름 1 · 막대 2 · % 3(오른쪽 맞춤) · 리셋 4 · 남은 시간 5', () => {
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

  it('값 없는 줄은 % 가 그대로 3 열 · 리셋 없는 줄은 4·5 열이 빈다 · 만료 줄은 2 열부터 끝까지 덮는다', () => {
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

describe('⚠ 없음(R10 개정 2026-09-29)', () => {
  it('20% 미만이어도 작은 표시(모든 단계·측정 사본)와 팝업 어디에도 ⚠ 가 없다', () => {
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

describe('측정 사본과 단계 고르기', () => {
  it('측정 사본이 새 모양(격자·아이콘·리셋 시각)을 그린 채로 단계가 골라진다', () => {
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

  it('관찰 대상 = 슬롯 루트 · ⟳ 자리 · 두 측정 사본 — 측정 사본은 그 단계에서 보이는 것과 같은 모양이다', () => {
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
      // 회사 아이콘·모래시계는 같은 수.
      expect(m[n].querySelectorAll('svg:not(.lucide)')).toHaveLength(shown.querySelectorAll('svg:not(.lucide)').length)
      expect(m[n].querySelectorAll('svg.lucide-hourglass')).toHaveLength(shown.querySelectorAll('svg.lucide-hourglass').length)
      const placed = shown.querySelectorAll('[style*="grid-column"]').length
      expect(placed).toBeGreaterThan(0)
      expect(m[n].querySelectorAll('[style*="grid-column"]')).toHaveLength(placed)
    }
  })

  it('높이 되돌림 없음 — 2단 격자가 슬롯보다 키가 커도 2단 그대로(두 회사 · 배지는 아이콘 옆 · ADR-0259 결정 6)', () => {
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

// ── 「사용량 페이지 ↗」 목적지 = 셸 opener 허용 목록 — 쿼리·끝 슬래시 하나라도 다르면 셸이 여는 것을 거절한다 ──
describe('사용량 페이지 주소 = 셸 허용 목록(src-tauri/capabilities/usage-links.json)', () => {
  it('회사별 주소가 허용 목록과 글자까지 같다 · 목록에 남는 주소도 없다', () => {
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
