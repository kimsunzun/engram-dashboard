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

import { useUsageStore } from '../../store/usageStore'
import UsageSlot from './UsageSlot'
import { formatAge, formatClock, formatResetAt, statusSentence } from './usageFormat'

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
  fire(entries: Array<[Element, number]>) {
    this.cb(
      entries.map(([target, width]) => ({ target, contentRect: { width, height: 10 } }) as unknown as ResizeObserverEntry),
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

/** 실제 폭과 1·2단 자연 폭을 준다 — 숨은 렌더를 재는 관찰자와 같은 하나다. */
function setWidths(avail: number, s1: number, s2: number): void {
  const live = FakeResizeObserver.instances.filter(o => !o.disconnected)
  expect(live).toHaveLength(1)
  act(() =>
    live[0].fire([
      [q('[data-usage-content]')!, avail],
      [q('[data-usage-measure="1"]')!, s1],
      [q('[data-usage-measure="2"]')!, s2],
    ]),
  )
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
      expect(summary().textContent).not.toContain('Cx')
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

  it('막대 = meter(D14) — aria-valuenow 는 보이는 수', () => {
    seed(snap('claude', { five_hour: { used_pct: 80.4, resets_at: NOW + 600, age_secs: 0, expired: false } }))
    mount(true, false)
    const meters = within(summary()).getAllByRole('meter')
    expect(meters).toHaveLength(2)
    expect(meters[0].getAttribute('aria-valuenow')).toBe('19')
    expect(meters[0].getAttribute('aria-valuemin')).toBe('0')
    expect(meters[0].getAttribute('aria-valuemax')).toBe('100')
    expect(meters[0].getAttribute('aria-valuetext')).toBe('19% 남음')
    expect(meters[0].getAttribute('aria-label')).toBe('Claude 5시간 남은 양')
  })

  it('색은 토큰만 — 구간별 채움·글자색 토큰, 20% 미만은 ⚠ 를 붙인다', () => {
    seed(
      snap('claude', {
        five_hour: { used_pct: 81, resets_at: NOW + 600, age_secs: 0, expired: false },
        weekly: { used_pct: 50, resets_at: NOW + 600, age_secs: 0, expired: false },
      }),
    )
    seed(snap('codex', { five_hour: { used_pct: 10, resets_at: NOW + 600, age_secs: 0, expired: false }, weekly: null }))
    mount()
    const danger = valueOf('claude', 'five_hour')!
    expect(danger.textContent).toBe('19% ⚠')
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

  it('남은 시간은 1·2단에만, 3단은 숫자만(「%」 없이)', () => {
    seed(snap('claude'))
    mount(true, false)
    toStage(1)
    expect(summary().querySelectorAll('[data-usage-reset-in]')).toHaveLength(2)
    expect(q('[data-usage-reset-in]', summary())!.textContent).toBe('2시간 13분 뒤')
    toStage(2)
    expect(summary().querySelectorAll('[data-usage-reset-in]')).toHaveLength(2)
    toStage(3)
    expect(summary().querySelectorAll('[data-usage-reset-in]')).toHaveLength(0)
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
    expect(nameUnit.textContent).toBe('Cl!')
  })

  it('숨은 측정 사본은 역할·값 표식을 싣지 않는다(cdp·보조기술이 두 번 읽지 않게)', () => {
    seed(snap('claude', { state: { kind: 'NeedsLogin', detail: null } }))
    mount(true, false)
    for (const n of ['1', '2']) {
      const m = q(`[data-usage-measure="${n}"]`)!
      expect(m.querySelector('[data-usage-vendor]')).toBeNull()
      expect(m.querySelector('[role]')).toBeNull()
      expect(m.querySelector('[aria-label]')).toBeNull()
      expect(m.textContent).toContain('62%')
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
      const expected = statusSentence(state, 'Claude', 0, NOW)!
      for (const n of [1, 2, 3] as const) {
        toStage(n)
        const badge = q('[data-usage-badge="claude"]')
        expect(badge).not.toBeNull()
        expect(badge!.textContent).toBe('!')
        expect(badge!.textContent).not.toBe('⚠')
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
      '설치 안 됨',
      '로그인 필요 (auth_error)',
      '이 계정의 한도 정보를 받을 수 없음 (limits_unavailable)',
      `조회 실패 — Claude 응답: rate_limits_available: true, rate_limits: null · 다음 시도 ${formatClock(NOW + 300)}`,
      '거절됨 — 12분 뒤 (rate_limited)',
    ])
  })

  it('상류 원문은 번역·정화 없이 그대로 · code 는 접두 뒤 괄호', () => {
    const upstream = '{kind} 원문 $& <b>'
    seed(
      snap('codex', {
        state: { kind: 'Failed', next_attempt_in_secs: 60, detail: { kind: 'rpc_error', code: -32001, upstream } },
      }),
    )
    mount(false, true)
    expect(q('[data-usage-badge="codex"]')!.getAttribute('aria-label')).toBe(
      `조회 실패 — Codex 응답 (-32001): ${upstream} · 다음 시도 ${formatClock(NOW + 60)}`,
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
    const fresh = valueOf('claude', 'weekly')!
    expect(fresh.hasAttribute('data-usage-stale')).toBe(false)
    expect(fresh.style.opacity).toBe('')
    // 흐리게 하는 것은 숫자뿐 — 막대·이름은 그대로.
    expect(q('[data-usage-vendor="claude"][data-usage-window="five_hour"] [role="meter"]')!.style.opacity).toBe('')
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
    for (const w of ['five_hour', 'weekly']) {
      const seg = q(`[data-usage-vendor="claude"][data-usage-window="${w}"]`, summary())!
      expect(q('[data-usage-value]', seg)!.textContent).toBe('리셋됨 — 갱신 대기')
      expect(seg.textContent).not.toMatch(/%/)
      expect(q('[data-usage-reset-in]', seg)).toBeNull()
      expect(q('[role="meter"]', seg)).toBeNull()
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
    expect(q('[data-usage-reset-in]', summary())!.textContent).toBe('2시간 13분 뒤')
    expect(q('[data-usage-age]', popup)!.textContent).toBe('방금')
    const before = q('[data-usage-badge]')!.getAttribute('aria-label')
    expect(before).toBe(`조회 실패 · 다음 시도 ${formatClock(NOW + 600)}`)

    advance(60_000)
    expect(q('[data-usage-reset-in]', summary())!.textContent).toBe('2시간 12분 뒤')
    expect(q('[data-usage-age]', popup)!.textContent).toBe(formatAge(60))
    // 다음 시도 시각 자체는 같은 순간을 가리킨다(받은 순간 기준 상대 초 − 흐른 초 + 지금).
    expect(q('[data-usage-badge]')!.getAttribute('aria-label')).toBe(before)
    expect(client.getUsageSnapshot).toHaveBeenCalledTimes(1)
    expect(client.refreshUsageLimits).not.toHaveBeenCalled()
  })

  it('거절 대기 「N분 뒤」도 받은 뒤 흐른 만큼 준다', () => {
    seed(snap('claude', { state: { kind: 'Rejected', retry_in_secs: 720, detail: null } }))
    mount(true, false)
    expect(q('[data-usage-badge]')!.getAttribute('aria-label')).toBe('거절됨 — 12분 뒤')
    advance(5 * 60_000)
    expect(q('[data-usage-badge]')!.getAttribute('aria-label')).toBe('거절됨 — 7분 뒤')
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
    expect(q('[data-usage-status-line="claude"] [data-usage-status-text]', popup)!.textContent).toBe('거절됨 — 10분 뒤')
    const claudeRefresh = q('[data-usage-refresh="claude"]', popup) as HTMLButtonElement
    expect(claudeRefresh.disabled).toBe(true)
    fireEvent.click(claudeRefresh)
    expect(client.refreshUsageLimits).not.toHaveBeenCalled()
    expect((q('[data-usage-refresh="codex"]', popup) as HTMLButtonElement).disabled).toBe(false)
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
