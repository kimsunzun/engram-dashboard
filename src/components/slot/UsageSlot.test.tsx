// UsageSlot — 사용량 슬롯 화면(TRD S21 usage-limit-slot §4 「프론트 표시」 행 중 컴포넌트 몫).
//
// 스토어는 실물(`usageStore`)이고 값은 `setState` 로 심는다 — 받기 규칙(merge)은 `usageStore.test.ts` 가 잰다.
// 시계는 둘 다 가짜다: 벽시계(`Date`)는 가짜 타이머, 받은 시각 기준 시계(`performance.now`)는 spy.
// jsdom 엔 레이아웃이 없어 폭은 가짜 ResizeObserver 로 직접 준다(실제 폭 + 두 단계의 자연 폭).

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

// `?raw` 인 이유 = `index.css.test.ts` 와 같다(@types/node 가 없어 fs 를 쓰면 tsc 게이트가 깨진다).
import usageLinksSource from '../../../src-tauri/capabilities/usage-links.json?raw'
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

function mount(showClaude = true, showCodex = true) {
  return render(<UsageSlot content={{ type: 'usage', show_claude: showClaude, show_codex: showCodex }} />)
}

function q(selector: string, root: ParentNode = document): HTMLElement | null {
  return root.querySelector<HTMLElement>(selector)
}

function summary(): HTMLElement {
  return q('[data-usage-summary]')!
}

function valueOf(vendor: string, window: string, root: ParentNode = summary()): HTMLElement | null {
  return q(`[data-usage-vendor="${vendor}"][data-usage-window="${window}"] [data-usage-value]`, root)
}

/**
 * 실제 폭과 1·2단 자연 폭을 준다 — 숨은 렌더를 재는 관찰자와 같은 하나다. 높이는 기본으로 넉넉하다(슬롯 1000 · 2단 30).
 */
function setWidths(avail: number, s1: number, s2: number, { rootH = 1000, s2h = 30 } = {}): void {
  const live = FakeResizeObserver.instances.filter(o => !o.disconnected)
  expect(live).toHaveLength(1)
  act(() =>
    live[0].fire([
      [q('[data-usage-slot]')!, 800, rootH],
      [q('[data-usage-content]')!, avail],
      [q('[data-usage-measure="1"]')!, s1],
      [q('[data-usage-measure="2"]')!, s2, s2h],
    ]),
  )
}

/** 로컬 시각 「HH:MM」·요일 한 글자 — 포맷터와 따로 세운 기대값(`Date` 의 로컬 getter). */
function hhmm(epochSecs: number): string {
  const d = new Date(epochSecs * 1000)
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
}
function weekdayOf(epochSecs: number): string {
  return '일월화수목금토'[new Date(epochSecs * 1000).getDay()]
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

  it('두 회사 다 꺼짐 → 안내 한 줄만(R4) · 요약 버튼·팝업 없음', () => {
    seed(snap('claude'))
    mount(false, false)
    expect(q('[data-usage-hint]')!.textContent).toBe('우클릭해서 표시할 항목 고르기')
    expect(q('[data-usage-summary]')).toBeNull()
    expect(q('[data-usage-vendor]')).toBeNull()
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
    expect(q('[data-usage-refresh="codex"]', popup)).toBeNull()
    expect(popup.textContent).not.toContain('Codex')
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

  it('리셋 시각은 1·2단에만(5시간 = 「↻ HH:MM」 · 주간 = 「↻ 요일 HH:MM」 · 남은 시간 없음), 3단은 숫자만(「%」 없이)', () => {
    seed(snap('claude'))
    mount(true, false)
    const clock = (w: string) =>
      q(`[data-usage-vendor="claude"][data-usage-window="${w}"] [data-usage-reset-clock]`, summary())!.textContent
    for (const n of [1, 2] as const) {
      toStage(n)
      expect(summary().querySelectorAll('[data-usage-reset-clock]')).toHaveLength(2)
      expect(clock('five_hour')).toBe(`↻ ${hhmm(NOW + 7_980)}`)
      expect(clock('weekly')).toBe(`↻ ${weekdayOf(NOW + 86_400 * 3)} ${hhmm(NOW + 86_400 * 3)}`)
      expect(summary().textContent).not.toMatch(/뒤/)
    }
    toStage(3)
    expect(summary().querySelectorAll('[data-usage-reset-clock]')).toHaveLength(0)
    expect(summary().textContent).not.toContain('↻')
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
      expect(m.querySelectorAll('svg')).toHaveLength(1)
      expect(m.textContent).toContain(`↻ ${hhmm(NOW + 7_980)}`)
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
        expect(seg.textContent).not.toMatch(/%|↻/)
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
    // 팝업엔 토글이 없다(R8 — 켜고 끄기는 우클릭 메뉴).
    expect(popup.querySelector('input[type="checkbox"], [role="menuitemcheckbox"], [role="switch"]')).toBeNull()
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

  it('⟳ 는 팝업에만 · 켠 회사마다 하나 — 누르면 그 회사를 새로고침한다', () => {
    seed(snap('claude'))
    seed(snap('codex'))
    mount()
    expect(q('[data-usage-refresh]')).toBeNull()
    const popup = openPopup()
    expect(popup.querySelectorAll('[data-usage-refresh]')).toHaveLength(2)
    fireEvent.click(q('[data-usage-refresh="codex"]', popup)!)
    expect(client.refreshUsageLimits).toHaveBeenCalledWith('codex')
    expect(screen.getByRole('button', { name: 'Codex 새로고침' })).toBeTruthy()
  })

  it('보이는 거절 → 「거절됨 — N분 뒤」 + 그 회사 ⟳ 비활성(다른 회사는 그대로)', () => {
    seed(snap('claude', { state: { kind: 'Rejected', retry_in_secs: 600, detail: null } }))
    seed(snap('codex'))
    mount()
    const popup = openPopup()
    expect(q('[data-usage-status-line="claude"] [data-usage-status-text]', popup)!.textContent).toBe(
      'Claude 거절됨 — 10분 뒤',
    )
    const claudeRefresh = q('[data-usage-refresh="claude"]', popup) as HTMLButtonElement
    expect(claudeRefresh.getAttribute('aria-disabled')).toBe('true')
    expect(claudeRefresh.disabled).toBe(false) // 포커스를 쥘 수 있게 남긴다
    fireEvent.click(claudeRefresh)
    expect(client.refreshUsageLimits).not.toHaveBeenCalled()
    expect(q('[data-usage-refresh="codex"]', popup)!.hasAttribute('aria-disabled')).toBe(false)
  })

  it('포커스된 ⟳ 가 거절로 바뀌어도 포커스를 쥔 채 누름만 막히고, Esc 는 그대로 닫는다', () => {
    seed(snap('claude'))
    mount(true, false)
    const popup = openPopup()
    const refresh = () => q('[data-usage-refresh="claude"]', popup) as HTMLButtonElement
    act(() => refresh().focus())
    act(() => seed(snap('claude', { revision: 2, state: { kind: 'Rejected', retry_in_secs: 600, detail: null } })))
    expect(refresh().getAttribute('aria-disabled')).toBe('true')
    expect(document.activeElement).toBe(refresh())
    fireEvent.click(refresh())
    expect(client.refreshUsageLimits).not.toHaveBeenCalled()
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(document.activeElement).toBe(summary())
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
      expect(q('[data-usage-reset-clock]', seg())!.textContent).toBe(`↻ ${hhmm(NOW + 600)}`)
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
        <UsageSlot content={{ type: 'usage', show_claude: true, show_codex: false }} />
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
    expect(resetOnly[2].textContent).toBe(`↻ ${weekdayOf(NOW + 86_400)} ${hhmm(NOW + 86_400)}`)
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
      expect(m.querySelectorAll('svg')).toHaveLength(2)
      expect(m.textContent).toContain(`↻ ${weekdayOf(NOW + 86_400 * 3)} ${hhmm(NOW + 86_400 * 3)}`)
    }
    setWidths(300, 400, 250)
    expect(stage()).toBe('2')
    setWidths(240, 400, 250)
    expect(stage()).toBe('3')
    setWidths(420, 400, 250)
    expect(stage()).toBe('1')
  })

  it('관찰 대상 = 슬롯 루트 · 보이는 내용 · 두 측정 사본 — 측정 사본은 그 단계에서 보이는 것과 같은 모양이다', () => {
    seed(snap('claude', { state: { kind: 'NeedsLogin', detail: null } }))
    seed(snap('codex', { five_hour: { used_pct: null, resets_at: NOW + 600, age_secs: 0, expired: false } }))
    mount()
    const m = { 1: q('[data-usage-measure="1"]')!, 2: q('[data-usage-measure="2"]')! }
    const live = FakeResizeObserver.instances.filter(o => !o.disconnected)
    expect(live[0].observed).toEqual([q('[data-usage-slot]'), q('[data-usage-content]'), m[1], m[2]])
    for (const n of [1, 2] as const) {
      toStage(n)
      const shown = q('[data-usage-content]')!
      const shownGrid = shown.firstElementChild as HTMLElement
      const copyGrid = m[n].firstElementChild as HTMLElement
      expect(copyGrid.style.gridTemplateColumns).toBe(shownGrid.style.gridTemplateColumns)
      expect(m[n].textContent).toBe(shown.textContent)
      expect(m[n].querySelectorAll('svg')).toHaveLength(shown.querySelectorAll('svg').length)
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
    expect(shown.textContent).not.toMatch(/5시간|주간|↻/)
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
