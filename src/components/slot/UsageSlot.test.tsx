// UsageSlot — 사용량 슬롯 화면(TRD S21 usage-limit-slot §4 「프론트 표시」 행 중 컴포넌트 몫).
//
// 스토어는 실물(`usageStore`)이고 값은 `setState` 로 심는다 — 받기 규칙(merge)은 `usageStore.test.ts` 가 잰다.
// command 도 실물(`usageSlot.*` 등록)이다 — ⟳·표시 토글이 command 를 거쳐 스토어·레이아웃 쓰기에 닿는 것까지 잰다.
// 시계는 둘 다 가짜다: 벽시계(`Date`)는 가짜 타이머, 받은 시각 기준 시계(`performance.now`)는 spy.
// jsdom 엔 레이아웃이 없어 폭은 가짜 ResizeObserver 로 직접 준다(슬롯 크기 · ⟳ 자리 폭 + 두 단계의 자연 폭).

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
import { formatAge, formatClock, formatResetAt, statusLine, statusSentence } from './usageFormat'

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
 * 높이는 기본으로 넉넉하다(슬롯 1000 · 2단 30).
 */
function setWidths(avail: number, s1: number, s2: number, { rootH = 1000, s2h = 30 } = {}): void {
  const refreshW = refreshButton() ? REFRESH_AREA_W : 0
  fireSizes({ rootW: avail + refreshW + SUMMARY_PAD_X_TOTAL, rootH, refreshW, s1, s2, s2h })
}

/**
 * 작은 표시의 리셋 문구 — 포맷터와 따로 세운 기대값(`Date` 의 로컬 getter). 지금(`NOW`)과 같은 로컬 날짜면 「HH:MM」,
 * 아니면 「M/D HH:MM」. 시간대에 따라 NOW + 몇 시간이 자정을 넘을 수 있어 날짜를 늘 직접 견준다.
 */
function resetText(epochSecs: number): string {
  const d = new Date(epochSecs * 1000)
  const now = new Date(NOW * 1000)
  const time = `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
  const sameDay =
    d.getFullYear() === now.getFullYear() && d.getMonth() === now.getMonth() && d.getDate() === now.getDate()
  return `리셋 ${sameDay ? time : `${d.getMonth() + 1}/${d.getDate()} ${time}`}`
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
    // 이름이 남는 자리는 표시 토글 줄뿐이다(다시 켤 자리) — 꺼진 채로.
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

  it('리셋 시각은 1·2단에만(「리셋 HH:MM」 · 다른 날이면 「리셋 M/D HH:MM」 · 요일·남은 시간 없음), 3단은 숫자만(「%」 없이)', () => {
    seed(snap('claude'))
    mount(true, false)
    const clock = (w: string) =>
      q(`[data-usage-vendor="claude"][data-usage-window="${w}"] [data-usage-reset-clock]`, summary())!.textContent
    for (const n of [1, 2] as const) {
      toStage(n)
      expect(summary().querySelectorAll('[data-usage-reset-clock]')).toHaveLength(2)
      expect(clock('five_hour')).toBe(resetText(NOW + 7_980))
      expect(clock('weekly')).toBe(resetText(NOW + 86_400 * 3))
      expect(clock('weekly')).toMatch(/^리셋 \d{1,2}\/\d{1,2} \d{2}:\d{2}$/)
      expect(clock('weekly')).not.toMatch(/[일월화수목금토]/)
      expect(summary().textContent).not.toMatch(/뒤|↻/)
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
      // 보이는 쪽과 같은 모양 — 한 격자 · 아이콘 · 리셋 시각. 갱신 중 표식은 조회가 없어도 늘 센다.
      expect((m.firstElementChild as HTMLElement).style.display).toBe('grid')
      expect(m.querySelectorAll('svg:not(.lucide)')).toHaveLength(1)
      expect(m.querySelectorAll('svg.lucide')).toHaveLength(1)
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
  it('30분 넘은 값 → 작은 표시는 숫자만 흐리게(나이 문구 없음) · 팝업은 나이를 보이고 흐리지 않는다', () => {
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
    expect(q('[data-usage-window="five_hour"] [data-usage-age]', popup)!.textContent).toBe('30분 전')
    expect(q('[data-usage-window="weekly"] [data-usage-age]', popup)!.textContent).toBe('30분 전')
    expect(q('[data-usage-stale]', popup)).toBeNull()
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

  it('「갱신 중」 = ⟳ 대기 또는 스냅숏 in_flight — 값은 그대로', () => {
    seed(snap('claude', { in_flight: true }))
    seed(snap('codex'))
    mount()
    const mark = q('[data-usage-refreshing="claude"]')!
    expect(mark.getAttribute('aria-label')).toBe('갱신 중')
    expect(valueOf('claude', 'five_hour')!.textContent).toBe('62%')
    expect(q('[data-usage-refreshing="codex"]')).toBeNull()
    act(() => useUsageStore.setState({ pending: { codex: 1 } }))
    expect(q('[data-usage-refreshing="codex"]')).not.toBeNull()
    expect(valueOf('codex', 'five_hour')!.textContent).toBe('62%')
    const popup = openPopup()
    expect(q('[data-usage-popup-vendor="codex"] [data-usage-refreshing-text]', popup)!.textContent).toBe('갱신 중')
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
    const resetAt = () => q('[data-usage-window="five_hour"] [data-usage-reset-at]', popup)!.textContent
    expect(resetAt()).toBe(`리셋 ${formatResetAt(NOW + 7_980, NOW)} (2시간 13분 뒤)`)
    expect(q('[data-usage-age]', popup)!.textContent).toBe('방금')
    const before = q('[data-usage-badge]')!.getAttribute('aria-label')
    expect(before).toBe(`Claude 조회 실패 · 다음 시도 ${formatClock(NOW + 600)}`)

    advance(60_000)
    expect(resetAt()).toBe(`리셋 ${formatResetAt(NOW + 7_980, NOW)} (2시간 12분 뒤)`)
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

  it('창별 절대 리셋 시각 · 값마다 나이 · plan 은 있을 때만 · 모델별 창은 값이 있는 것만', () => {
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
    expect(q('[data-usage-reset-at]', five)!.textContent).toBe(`리셋 ${formatResetAt(NOW + 7_980, NOW)} (2시간 13분 뒤)`)
    expect(q('[data-usage-age]', five)!.textContent).toBe('2분 전')
    expect(q('[data-usage-window="weekly"] [data-usage-reset-at]', claude)!.textContent).toContain(
      formatResetAt(NOW + 86_400 * 3, NOW),
    )
    expect(q('[data-usage-plan]', claude)!.textContent).toBe('플랜: max_20x')
    expect(q('[data-usage-window="model:Opus"] [data-usage-value]', claude)!.textContent).toBe('30%')
    expect(q('[data-usage-window="model:Sonnet"]', claude)).toBeNull()
    expect(q('[data-usage-plan]', q('[data-usage-popup-vendor="codex"]', popup)!)).toBeNull()
  })

  it('다른 날의 리셋 시각은 「M/D HH:MM」(요일 없음) — 작은 표시와 같은 규칙', () => {
    seed(snap('claude'))
    mount(true, false)
    const popup = openPopup()
    const weekly = q('[data-usage-window="weekly"] [data-usage-reset-at]', popup)!.textContent!
    expect(weekly).toBe(`${resetText(NOW + 86_400 * 3)} (3일 뒤)`)
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

  it('팝업엔 회사별 ⟳ 가 없다 — 새로고침은 작은 표시의 ⟳ 하나다', () => {
    seed(snap('claude'))
    seed(snap('codex'))
    mount()
    const popup = openPopup()
    expect(q('[data-usage-refresh]', popup)).toBeNull()
    expect(within(popup).queryAllByRole('button', { name: /새로고침/ })).toHaveLength(0)
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

  it('대상의 조회가 도는 동안의 누름은 버린다 — 「갱신 중」이 보이고, 답이 오면 다시 받는다', () => {
    seed(snap('claude'))
    seed(snap('codex'))
    mount()
    act(() => useUsageStore.setState({ pending: { codex: 1 } }))
    expect(q('[data-usage-refreshing="codex"]')).not.toBeNull()
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

  it('3단 — 쉬는 줄은 끝에 갱신 중 표식과 같은 모양·폭의 빈자리, 갱신 중인 줄은 앞의 표식만(⟳ 가 움직이지 않게)', () => {
    seed(snap('claude'))
    seed(snap('codex'))
    mount()
    toStage(3)
    const nameUnitOf = (vendor: AgentBackendKind) => q(`[data-usage-icon="${vendor}"]`, summary())!.parentElement!
    const numbersOf = (vendor: AgentBackendKind) => q('[data-usage-numbers]', nameUnitOf(vendor).parentElement!)!
    const spaceOf = (vendor: AgentBackendKind) => numbersOf(vendor).nextElementSibling as HTMLElement | null
    for (const vendor of ['claude', 'codex'] as const) {
      const line = nameUnitOf(vendor).parentElement!
      const group = numbersOf(vendor).parentElement!
      expect([...line.children]).toEqual([nameUnitOf(vendor), group])
      expect(group.style.flexWrap).toBe('wrap')
      const space = spaceOf(vendor)!
      expect([...group.children]).toEqual([numbersOf(vendor), space])
      // 드러나지 않는다 — 보조기술에도, cdp 값 표식에도.
      expect(space.getAttribute('aria-hidden')).toBe('true')
      expect(space.style.visibility).toBe('hidden')
      for (const el of [space, ...space.querySelectorAll('*')]) {
        const names = el.getAttributeNames()
        expect(names.filter(a => a.startsWith('data-usage') || a === 'role' || a === 'title' || a === 'aria-label')).toEqual(
          [],
        )
      }
      // 좁을 때 먼저 다음 줄로 넘어가 사라진다 — 넘어간 줄이 높이를 보태지 않는다.
      expect(space.style.height).toBe('0px')
      expect(space.style.flexShrink).toBe('0')
      expect(space.style.marginLeft).toBe(nameUnitOf(vendor).style.gap)
      expect(q('[data-usage-refreshing]', nameUnitOf(vendor))).toBeNull()
    }
    const idleCopy = spaceOf('claude')!.firstElementChild as HTMLElement
    act(() => useUsageStore.setState({ pending: { claude: 1 } }))
    // 갱신 중인 줄: 빈자리가 빠지고 이름 묶음에 표식이 든다 — 같은 모양(돌기만 다르다) · 같은 틈.
    expect(spaceOf('claude')).toBeNull()
    expect([...numbersOf('claude').parentElement!.children]).toEqual([numbersOf('claude')])
    const mark = q('[data-usage-refreshing="claude"]', nameUnitOf('claude'))!
    expect(mark.style.cssText).toBe(idleCopy.style.cssText)
    const cls = (el: Element) => el.querySelector('svg')!.getAttribute('class')!.replace('animate-spin', '').trim()
    expect(cls(mark)).toBe(cls(idleCopy))
    expect(idleCopy.querySelector('svg')!.getAttribute('class')).not.toContain('animate-spin')
    // 다른 회사 줄은 그대로 빈자리를 둔다.
    expect(spaceOf('codex')).not.toBeNull()
    act(() => useUsageStore.setState({ pending: {} }))
    expect(spaceOf('claude')).not.toBeNull()
    expect(q('[data-usage-refreshing]', summary())).toBeNull()
  })

  it('측정 사본은 조회 여부와 상관없이 같다 — 갱신 중 표식을 늘 세어, ⟳ 를 눌러도 단계 판정 입력이 바뀌지 않는다', () => {
    seed(snap('claude'))
    seed(snap('codex'))
    mount()
    const copies = () => ['1', '2'].map(n => q(`[data-usage-measure="${n}"]`)!.innerHTML)
    const idle = copies()
    for (const n of ['1', '2']) {
      const m = q(`[data-usage-measure="${n}"]`)!
      expect(m.querySelectorAll('svg.lucide')).toHaveLength(2)
      for (const svg of m.querySelectorAll('svg.lucide')) expect(svg.getAttribute('class')).not.toContain('animate-spin')
    }
    act(() => useUsageStore.setState({ pending: { claude: 1 } }))
    expect(q('[data-usage-refreshing="claude"]', summary())).not.toBeNull()
    expect(copies()).toEqual(idle)
    act(() => {
      useUsageStore.setState({ pending: {} })
      seed(snap('codex', { in_flight: true }))
    })
    expect(q('[data-usage-refreshing="codex"]', summary())).not.toBeNull()
    expect(copies()).toEqual(idle)
    act(() => seed(snap('codex')))
    expect(q('[data-usage-refreshing]', summary())).toBeNull()
    expect(copies()).toEqual(idle)
  })

  it('1·2단과 측정 사본엔 빈자리가 없다(막대 칸이 표식 폭을 받는다)', () => {
    seed(snap('claude'))
    mount(true, false)
    const spaces = (root: ParentNode) =>
      [...root.querySelectorAll<HTMLElement>('span[aria-hidden="true"]')].filter(
        el => el.style.visibility === 'hidden' && el.style.height === '0px',
      )
    for (const n of [1, 2] as const) {
      toStage(n)
      expect(spaces(summary())).toEqual([])
    }
    for (const n of ['1', '2']) expect(spaces(q(`[data-usage-measure="${n}"]`)!)).toEqual([])
    toStage(3)
    expect(spaces(summary())).toHaveLength(1)
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

// ── 리뷰 후속(7b) — 높이 · 리셋만 실린 창 · 뒤집기 · 포커스 · Esc · 회전 ──
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
  it('2단이 슬롯보다 키가 크면 3단 — 잘린 줄의 배지가 사라지지 않게(R31 · R3)', () => {
    seed(snap('claude', { state: { kind: 'NeedsLogin', detail: null } }))
    mount(true, false)
    // 2단 높이 + 요약 버튼의 위아래 여백(4px × 2) 과 슬롯 높이를 견준다.
    setWidths(300, 400, 200, { rootH: 53, s2h: 45 })
    expect(stage()).toBe('2')
    setWidths(300, 400, 200, { rootH: 52, s2h: 45 })
    expect(stage()).toBe('3')
    expect(q('[data-usage-badge="claude"]')).not.toBeNull()
    // 1단은 폭만 본다 — 줄 수가 3단과 같아 3단으로 가도 얻는 것이 없다.
    setWidths(500, 400, 200, { rootH: 10, s2h: 45 })
    expect(stage()).toBe('1')
  })

  it('리셋 시각만 실린 창 — 막대 없이 「—」 + 리셋 시각(1·2단) · 팝업엔 절대 리셋 시각(나이 없음)', () => {
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
    expect(q('[data-usage-reset-at]', row)!.textContent).toBe(`리셋 ${formatResetAt(NOW + 600, NOW)} (10분 뒤)`)
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

  it('갱신 중 표식은 돌고, 숨은 측정 사본은 돌지 않는다', () => {
    seed(snap('claude', { in_flight: true }))
    mount(true, false)
    expect(q('[data-usage-refreshing="claude"] svg')!.getAttribute('class')).toContain('animate-spin')
    for (const n of ['1', '2']) {
      const svg = q(`[data-usage-measure="${n}"] svg.lucide`)!
      expect(svg.getAttribute('class')).not.toContain('animate-spin')
    }
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
      expect(usage, `${theme} 사용량 줄에 색 리터럴`).not.toMatch(/#[0-9a-fA-F]{3,8}\b/)
      const base = block(theme, '--bg')
      for (const token of ['--status-danger', '--vendor-claude', '--vendor-codex']) {
        expect(base, `${theme} 바탕에 ${token} 가 없다`).toMatch(new RegExp(`${token}:\\s*[^;]+;`))
      }
    }
    const eink = block('e-ink', '--bg')
    for (const token of ['--status-danger', '--vendor-claude', '--vendor-codex']) {
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
      expect(m.querySelectorAll('svg.lucide')).toHaveLength(2)
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
      // 회사 아이콘은 같은 수 · 갱신 중 표식은 사본에만(조회가 없어도 회사마다 하나).
      expect(m[n].querySelectorAll('svg:not(.lucide)')).toHaveLength(shown.querySelectorAll('svg:not(.lucide)').length)
      expect(shown.querySelectorAll('svg.lucide')).toHaveLength(0)
      expect(m[n].querySelectorAll('svg.lucide')).toHaveLength(2)
      const placed = shown.querySelectorAll('[style*="grid-column"]').length
      expect(placed).toBeGreaterThan(0)
      expect(m[n].querySelectorAll('[style*="grid-column"]')).toHaveLength(placed)
    }
  })

  it('높이 되돌림 — 새 2단 격자가 슬롯보다 키가 크면 3단(아이콘 + 숫자만, 배지는 아이콘 옆)', () => {
    seed(snap('claude', { state: { kind: 'NeedsLogin', detail: null } }))
    seed(snap('codex', { state: { kind: 'NotInstalled', detail: null } }))
    mount()
    // 2단 높이 + 요약 버튼 위아래 여백(4px × 2) 과 슬롯 높이를 견준다.
    setWidths(300, 400, 250, { rootH: 68, s2h: 60 })
    expect(stage()).toBe('2')
    setWidths(300, 400, 250, { rootH: 67, s2h: 60 })
    expect(stage()).toBe('3')
    const shown = q('[data-usage-content]')!
    expect(q('[data-usage-grid]', shown)).toBeNull()
    expect(q('[data-usage-bar]', shown)).toBeNull()
    expect(q('[data-usage-reset-clock]', shown)).toBeNull()
    expect(shown.textContent).not.toMatch(/5시간|주간|리셋/)
    expect(shown.querySelectorAll('[data-usage-numbers]')).toHaveLength(2)
    for (const vendor of ['claude', 'codex']) {
      const badge = q(`[data-usage-badge="${vendor}"]`, shown)!
      expect(badge.previousElementSibling).toBe(q(`[data-usage-icon="${vendor}"]`, shown))
    }
    expect(shown.textContent).toBe('!62·41!62·41')
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
