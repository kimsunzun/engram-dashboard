// TRD S21-slot-docking §5-6 · §11 「TS tabStrip」 · PRD AC40: 닫기 모드 순수 상태 줄의 표 시험 — 경계(39/40 · 59/60 · 위 ·
//   시작 쪽 0) · 300 ms(가짜 시계 = 입력의 `now`) · 밖 누름 · 맨 끝 탭 · 고정 폭.

import { describe, expect, it } from 'vitest'

import { TAB_MIN_ACTIVE_W, TAB_MIN_INACTIVE_W, TAB_STANDARD_W, tabStripLayout } from './layout'
import {
  CLOSE_LEAVE_DELAY_MS,
  CLOSE_SLOP_BELOW_PX,
  CLOSE_SLOP_END_PX,
  CLOSING_OFF,
  type ClosingInput,
  type ClosingMode,
  type Rect,
  closingBounds,
  frozenAvailable,
  inClosingBounds,
  layoutAvailable,
  stepClosingMode,
} from './closingMode'

const A = TAB_MIN_ACTIVE_W
const I = TAB_MIN_INACTIVE_W
const S = TAB_STANDARD_W

const BAR: Rect = { x0: 0, y0: 0, x1: 500, y1: 28 }

function run(mode: ClosingMode, inputs: ClosingInput[]): ClosingMode {
  return inputs.reduce(stepClosingMode, mode)
}

describe('경계 — 아래 40 · 끝 쪽 60 · 위 · 시작 쪽 0 · 반열림(AC40)', () => {
  it('closingBounds', () => {
    expect(closingBounds(BAR)).toEqual({ x0: 0, y0: 0, x1: 500 + CLOSE_SLOP_END_PX, y1: 28 + CLOSE_SLOP_BELOW_PX })
    expect(CLOSE_SLOP_END_PX).toBe(60)
    expect(CLOSE_SLOP_BELOW_PX).toBe(40)
  })

  const CASES: { name: string; x: number; y: number; inside: boolean }[] = [
    { name: '탭바 안', x: 250, y: 14, inside: true },
    { name: '왼쪽 위 모서리(포함)', x: 0, y: 0, inside: true },
    { name: '아래 39 = 안', x: 250, y: 28 + 39, inside: true },
    { name: '아래 39.99 = 안', x: 250, y: 28 + 39.99, inside: true },
    { name: '아래 40 = 밖', x: 250, y: 28 + 40, inside: false },
    { name: '아래 41 = 밖', x: 250, y: 28 + 41, inside: false },
    { name: '끝 쪽 59 = 안', x: 500 + 59, y: 14, inside: true },
    { name: '끝 쪽 60 = 밖', x: 500 + 60, y: 14, inside: false },
    { name: '끝 쪽 61 = 밖', x: 500 + 61, y: 14, inside: false },
    { name: '끝 · 아래 모서리 59/39 = 안', x: 559, y: 67, inside: true },
    { name: '위는 여유 없음 — 바로 밖', x: 250, y: -0.5, inside: false },
    { name: '시작 쪽은 여유 없음 — 바로 밖', x: -0.5, y: 14, inside: false },
  ]
  for (const c of CASES) {
    it(c.name, () => {
      expect(inClosingBounds(BAR, c.x, c.y)).toBe(c.inside)
    })
  }

  it('탭바가 창 원점에 있지 않아도 탭바 기준', () => {
    const bar = { x0: 30, y0: 6, x1: 330, y1: 34 }
    expect(inClosingBounds(bar, 29.9, 20)).toBe(false)
    expect(inClosingBounds(bar, 30, 5.9)).toBe(false)
    expect(inClosingBounds(bar, 389, 73)).toBe(true)
    expect(inClosingBounds(bar, 390, 20)).toBe(false)
    expect(inClosingBounds(bar, 100, 74)).toBe(false)
  })
})

describe('frozenAvailable — 지금 폭 − 빠지는 폭', () => {
  const CASES: { name: string; current: number; widths: number[]; closed: number; next?: number; out: number }[] = [
    { name: '비활성 하나를 뺌', current: 400, widths: [100, 100, 100, 100], closed: 1, out: 300 },
    { name: '바탕은 폭 합이 아니라 넘긴 지금 폭', current: 330, widths: [100, 100, 100], closed: 1, out: 230 },
    { name: '단계 A 에서 활성을 뺌 — 닫힌 폭(다음 활성을 줘도)', current: 301, widths: [101, 100, 100], closed: 0, next: 1, out: 200 },
    { name: '단계 B 에서 활성을 뺌 — ★다음 활성 탭의 지금 폭★', current: 146, widths: [I + 3, A, I + 3, I + 2], closed: 1, next: 2, out: 111 },
    { name: '단계 B 라도 비활성을 빼면 닫힌 폭', current: 146, widths: [I + 3, A, I + 3, I + 2], closed: 2, out: 111 },
    { name: '단계 B · 다음 활성이 숨긴 탭이어도 그 계산 폭', current: 106, widths: [A, I, I], closed: 0, next: 2, out: 74 },
    { name: '숨긴 탭을 빼도 그 계산 폭(Chrome 의 보임 = 고정 탭 · 그룹뿐)', current: 135, widths: [A, I, I, I, I], closed: 3, out: 103 },
    { name: '넘김점 그대로(모두 A) = 단계 A', current: 126, widths: [A, A, A], closed: 1, next: 2, out: 84 },
  ]
  for (const c of CASES) {
    it(c.name, () => {
      expect(frozenAvailable(c.current, c.widths, c.closed, c.next)).toBe(c.out)
    })
  }

  it('빠진 자리가 마지막이거나 범위 밖 · 다음 활성 자리가 범위 밖이거나 빠진 자리와 같으면 RangeError', () => {
    expect(() => frozenAvailable(300, [100, 100, 100], 2)).toThrow(RangeError)
    expect(() => frozenAvailable(300, [100, 100, 100], -1)).toThrow(RangeError)
    expect(() => frozenAvailable(300, [100, 100, 100], 0.5)).toThrow(RangeError)
    expect(() => frozenAvailable(300, [100, 100, 100], 0, 3)).toThrow(RangeError)
    expect(() => frozenAvailable(146, [I + 3, A, I + 3, I + 2], 1, 1)).toThrow(RangeError)
  })
})

describe('stepClosingMode — 들어감 · 탭이 빠짐', () => {
  const closed = (
    widths: number[],
    at: number,
    measured: number | null,
    opts: { mouse?: boolean; next?: number } = {},
  ): ClosingInput => ({ kind: 'closed', mouse: opts.mouse ?? true, widths, closed: at, measured, nextActive: opts.next })

  const CASES: { name: string; from: ClosingMode; input: ClosingInput; out: ClosingMode }[] = [
    {
      name: '마우스로 닫음 → 들어감 · 바탕 = min(n·S, 잰 폭)',
      from: CLOSING_OFF,
      input: closed([100, 100, 100], 1, 300),
      out: { on: true, available: 200, deadline: null },
    },
    {
      name: '재지 못함 → 바탕 = n·S(480) — 380 ≥ 2·S 라 들어가지 않음',
      from: CLOSING_OFF,
      input: closed([100, 100, 100], 1, null),
      out: CLOSING_OFF,
    },
    {
      name: '모두 표준 폭 — 고정 폭이 남은 탭 표준 폭 합 이상이면 들어가지 않음',
      from: CLOSING_OFF,
      input: closed([S, S, S], 0, 1000),
      out: CLOSING_OFF,
    },
    {
      name: '표준 폭 합보다 1 px 좁으면 들어감',
      from: CLOSING_OFF,
      input: closed([S, S - 1, S], 0, 479),
      out: { on: true, available: 2 * S - 1, deadline: null },
    },
    {
      name: '마지막 탭(모드 밖) → 그때 폭으로 들어감 · 빼지 않는다(tci #1435)',
      from: CLOSING_OFF,
      input: closed([100, 100, 100], 2, 300),
      out: { on: true, available: 300, deadline: null },
    },
    {
      name: '마지막 탭(모드 중) → 모드 · 고정 폭 · 기한 그대로(tci #1435)',
      from: { on: true, available: 280, deadline: 1234 },
      input: closed([100, 100, 100], 2, 300),
      out: { on: true, available: 280, deadline: 1234 },
    },
    {
      name: '마지막 탭 · 그때 폭이 남은 탭 표준 폭 합 이상 → 들어가지 않음(tci #1263)',
      from: CLOSING_OFF,
      input: closed([S, S, 100], 2, 420),
      out: CLOSING_OFF,
    },
    {
      name: '숨긴 탭도 센다 — 마지막이 아닌 숨긴 탭이 빠지면 그 계산 폭을 뺀다(700 − 32)',
      from: { on: true, available: 700, deadline: null },
      input: closed([A, I, I, I, I, I, I, I, I, I], 4, 135, { mouse: false }),
      out: { on: true, available: 668, deadline: null },
    },
    {
      name: '보이는 마지막이라도 뒤에 숨긴 탭이 있으면 마지막 탭이 아니다 — 뺀다',
      from: CLOSING_OFF,
      input: closed([A, I, I, I, I], 2, 135),
      out: { on: true, available: 103, deadline: null },
    },
    { name: '탭 하나 → 들어가지 않음', from: CLOSING_OFF, input: closed([100], 0, 300), out: CLOSING_OFF },
    {
      name: '탭 하나 → 모드 중이면 나감',
      from: { on: true, available: 300, deadline: null },
      input: closed([100], 0, 300, { mouse: false }),
      out: CLOSING_OFF,
    },
    {
      name: '키보드 · 명령으로 닫음 → 들어가지 않음',
      from: CLOSING_OFF,
      input: closed([100, 100, 100], 1, 300, { mouse: false }),
      out: CLOSING_OFF,
    },
    {
      name: '모드 중 다른 길로 닫음 → 폭만 고침(Chrome)',
      from: { on: true, available: 300, deadline: null },
      input: closed([100, 100, 100], 0, 500, { mouse: false }),
      out: { on: true, available: 200, deadline: null },
    },
    {
      name: '모드 중 바탕 = 고정 폭 — 폭 합 300 · 잰 폭 500 이 아니라 330(tci #1470-1473)',
      from: { on: true, available: 330, deadline: null },
      input: closed([100, 100, 100], 1, 500),
      out: { on: true, available: 230, deadline: null },
    },
    {
      name: '모드 중 이어 닫음 → 기한은 그대로 둔다',
      from: { on: true, available: 300, deadline: 1234 },
      input: closed([100, 100, 100], 0, 300),
      out: { on: true, available: 200, deadline: 1234 },
    },
    {
      name: '단계 B 에서 활성을 닫음 — 다음 활성 폭을 뺀다',
      from: CLOSING_OFF,
      input: closed([I + 3, A, I + 3, I + 2], 1, 146, { next: 2 }),
      out: { on: true, available: 2 * I + 5 + A, deadline: null },
    },
  ]
  for (const c of CASES) {
    it(c.name, () => {
      expect(stepClosingMode(c.from, c.input)).toEqual(c.out)
    })
  }

  it('빠진 자리가 범위 밖이면 RangeError', () => {
    expect(() => stepClosingMode(CLOSING_OFF, closed([100, 100], -1, 200))).toThrow(RangeError)
    expect(() => stepClosingMode(CLOSING_OFF, closed([100, 100], 2, 200))).toThrow(RangeError)
  })
})

describe('stepClosingMode — 나감(300 ms · 밖 누름)', () => {
  const ON: ClosingMode = { on: true, available: 200, deadline: null }
  const move = (x: number, y: number, now: number): ClosingInput => ({ kind: 'move', x, y, bar: BAR, now })
  const press = (x: number, y: number): ClosingInput => ({ kind: 'press', x, y, bar: BAR })
  const tick = (now: number): ClosingInput => ({ kind: 'tick', now })

  it('300 ms', () => {
    expect(CLOSE_LEAVE_DELAY_MS).toBe(300)
  })

  it('탭바 안에 있는 동안은 시간이 지나도 그대로', () => {
    expect(run(ON, [move(250, 14, 0), tick(10_000), move(300, 28 + 39, 20_000), tick(60_000)])).toEqual(ON)
  })

  it('아래 40 으로 나가면 300 ms 뒤 나감 — 299 는 아직', () => {
    const left = run(ON, [move(250, 28 + 40, 1000)])
    expect(left).toEqual({ ...ON, deadline: 1300 })
    expect(stepClosingMode(left, tick(1299))).toEqual(left)
    expect(stepClosingMode(left, tick(1300))).toEqual(CLOSING_OFF)
  })

  it('끝 쪽 60 으로 나가도 같다 · 59 는 안', () => {
    expect(run(ON, [move(559, 14, 0), tick(5000)])).toEqual(ON)
    expect(run(ON, [move(560, 14, 0), tick(300)])).toEqual(CLOSING_OFF)
  })

  it('위 · 시작 쪽은 경계 바로 밖이면 나감', () => {
    expect(run(ON, [move(250, -1, 0), tick(300)])).toEqual(CLOSING_OFF)
    expect(run(ON, [move(-1, 14, 0), tick(300)])).toEqual(CLOSING_OFF)
  })

  it('300 ms 안에 돌아오면 취소 — 모드가 이어진다', () => {
    expect(run(ON, [move(250, 100, 0), move(250, 14, 299), tick(300), tick(5000)])).toEqual(ON)
  })

  it('밖에서 더 움직여도 기한은 처음 나간 때부터', () => {
    const out = run(ON, [move(250, 100, 0), move(250, 120, 200), move(250, 140, 290)])
    expect(out).toEqual({ ...ON, deadline: 300 })
    expect(stepClosingMode(out, tick(300))).toEqual(CLOSING_OFF)
  })

  it('나갔다 돌아온 뒤 다시 나가면 새 기한', () => {
    expect(run(ON, [move(250, 100, 0), move(250, 14, 100), move(250, 100, 1000)])).toEqual({ ...ON, deadline: 1300 })
  })

  it('창 밖으로 나가면 300 ms 뒤 · 그 안에 돌아오면 취소', () => {
    expect(run(ON, [{ kind: 'leaveWindow', now: 50 }, tick(349)])).toEqual({ ...ON, deadline: 350 })
    expect(run(ON, [{ kind: 'leaveWindow', now: 50 }, tick(350)])).toEqual(CLOSING_OFF)
    expect(run(ON, [{ kind: 'leaveWindow', now: 50 }, move(250, 14, 200), tick(350)])).toEqual(ON)
  })

  it('경계 밖으로 움직인 뒤 창 밖으로 나가도 기한은 그대로', () => {
    expect(run(ON, [move(250, 100, 0), { kind: 'leaveWindow', now: 250 }])).toEqual({ ...ON, deadline: 300 })
  })

  it('밖에서 누르면 곧바로 나감', () => {
    expect(stepClosingMode(ON, press(250, 28 + 40))).toEqual(CLOSING_OFF)
    expect(stepClosingMode({ ...ON, deadline: 900 }, press(560, 14))).toEqual(CLOSING_OFF)
  })

  it('안에서 누르면 그대로 · 걸린 기한은 취소', () => {
    expect(stepClosingMode(ON, press(250, 14))).toEqual(ON)
    expect(stepClosingMode({ ...ON, deadline: 900 }, press(250, 28 + 39))).toEqual(ON)
  })

  it('탭이 더해지면 곧바로 나감', () => {
    expect(stepClosingMode(ON, { kind: 'inserted' })).toEqual(CLOSING_OFF)
  })

  it('모드 밖에서는 아무 입력도 상태를 바꾸지 않는다', () => {
    const inputs: ClosingInput[] = [move(250, 100, 0), press(900, 900), { kind: 'leaveWindow', now: 0 }, tick(10_000), { kind: 'inserted' }]
    for (const input of inputs) expect(stepClosingMode(CLOSING_OFF, input)).toEqual(CLOSING_OFF)
  })
})

describe('연달아 닫기 — 들어감부터 나감까지', () => {
  it('세 번 이어 닫은 뒤 떠나 300 ms 에 다시 배치', () => {
    // 같은 폭 다섯(넘치지 않고 표준 폭보다 좁음) · 잰 폭 500 · 커서는 탭바 위 같은 자리.
    const close = (widths: number[], at: number): ClosingInput => ({ kind: 'closed', mouse: true, widths, closed: at, measured: 500 })
    let mode = stepClosingMode(CLOSING_OFF, close([100, 100, 100, 100, 100], 1))
    expect(mode).toEqual({ on: true, available: 400, deadline: null })
    mode = stepClosingMode(mode, close([100, 100, 100, 100], 1))
    expect(mode).toEqual({ on: true, available: 300, deadline: null })
    mode = stepClosingMode(mode, close([100, 100, 100], 1))
    expect(mode).toEqual({ on: true, available: 200, deadline: null })
    expect(layoutAvailable(mode, 500)).toBe(200)
    mode = run(mode, [{ kind: 'move', x: 150, y: 80, bar: BAR, now: 7000 }, { kind: 'tick', now: 7300 }])
    expect(mode).toEqual(CLOSING_OFF)
    expect(layoutAvailable(mode, 500)).toBe(500)
  })

  it('이어 닫다 마지막 탭을 닫으면 고정 폭 그대로 — 남은 탭이 그 안에서 넓어진다', () => {
    const close = (widths: number[], at: number): ClosingInput => ({ kind: 'closed', mouse: true, widths, closed: at, measured: 400 })
    let mode = stepClosingMode(CLOSING_OFF, close([100, 100, 100, 100], 1))
    expect(mode).toEqual({ on: true, available: 300, deadline: null })
    mode = stepClosingMode(mode, close([100, 100, 100], 2))
    expect(mode).toEqual({ on: true, available: 300, deadline: null })
    expect(tabStripLayout(400, 2, 0, layoutAvailable(mode, 400)).widths).toEqual([150, 150])
  })

  it('넘친 탭바에서 같은 자리를 세 번 닫음 — 고정 103 → 71 → 71 · 남은 폭 [A, I](Chromium 재현)', () => {
    // 잰 폭 135 · 탭 다섯 · 활성 0 · 마우스로 자리 2 를 세 번 닫는다. 폭은 상한(고정 폭)으로 · 보임은 잰 폭으로.
    const measured = 135
    let n = 5
    let mode: ClosingMode = CLOSING_OFF
    const seen: number[] = []
    for (let k = 0; k < 3; k += 1) {
      const { widths } = tabStripLayout(measured, n, 0, layoutAvailable(mode, measured))
      mode = stepClosingMode(mode, { kind: 'closed', mouse: true, widths, closed: 2, measured })
      n -= 1
      seen.push(mode.on ? mode.available : -1)
    }
    expect(seen).toEqual([103, 71, 71])
    expect(tabStripLayout(measured, n, 0, layoutAvailable(mode, measured))).toEqual({ widths: [A, I], visible: [true, true] })
  })
})

describe('layoutAvailable — 모드 중이면 min(고정 폭, 잰 폭)', () => {
  const ON: ClosingMode = { on: true, available: 300, deadline: null }
  const CASES: { name: string; mode: ClosingMode; measured: number | null; out: number | null }[] = [
    { name: '모드 밖 = 잰 폭', mode: CLOSING_OFF, measured: 500, out: 500 },
    { name: '모드 밖 · 재지 못함 = null', mode: CLOSING_OFF, measured: null, out: null },
    { name: '모드 중 = 고정 폭', mode: ON, measured: 500, out: 300 },
    { name: '모드 중 창이 줄면 잰 폭', mode: ON, measured: 250, out: 250 },
    { name: '모드 중 재지 못함 = 고정 폭', mode: ON, measured: null, out: 300 },
    { name: '모드 중 유한하지 않은 잰 폭 = 고정 폭', mode: ON, measured: Number.NaN, out: 300 },
  ]
  for (const c of CASES) {
    it(c.name, () => {
      expect(layoutAvailable(c.mode, c.measured)).toBe(c.out)
    })
  }
})
