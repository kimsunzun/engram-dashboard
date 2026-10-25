// TRD S26-window-chrome §10-3 의 제목 줄 몫 — 배치 순서 · 끌 자리 속성 · 번호 표식 · 창 단추(레지스트리 명령 · 상태 · 접근성 ·
//   main/팝아웃 셋째) · 흐림 · 대기 모드 · 크롬 모양. 탭바 자체는 자리표시 자식이다(탭바 몫 = `TabBar.test.tsx`).

import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invokeMock = vi.hoisted(() =>
  vi.fn(async (..._args: unknown[]): Promise<unknown> => ({
    deferred: false,
    pending: false,
    shown_all: false,
    focused: false,
    close: 'closed',
  })),
)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ label: 'main' }) }))

import '../../../commands/tabCommands' // 단추가 부르는 `window.*` 등록(운영은 `commands/contributions.ts`)
import type { WindowFrame, WindowFrameSource } from '../dock/windowFrame'
import { __resetMinSizeReportForTest, minWindowSizeSettled } from './minSizeReport'
import TitleRow from './TitleRow'

// theme.css 원문을 Node fs 로 읽는다 — 까닭과 선언 모양은 `store/chatStyleStore.test.ts` 와 같다.
declare function require(id: string): { readFileSync(p: string, enc: string): string }
declare const process: { cwd(): string }

const FRAME: WindowFrame = {
  restored: { w: 1200, h: 800 },
  maximized: false,
  insets: { t: 0, r: 0, b: 0, l: 0 },
  focused: true,
  caption: { buttons: true, leading_inset: 0, snap_overlay: false },
}

function fakeFrame(initial: WindowFrame | null = FRAME) {
  let frame = initial
  const fns = new Set<() => void>()
  const source: WindowFrameSource = {
    current: () => frame,
    subscribe: fn => {
      fns.add(fn)
      return () => {
        fns.delete(fn)
      }
    },
    install: () => () => {},
  }
  return {
    source,
    set: (next: WindowFrame | null) =>
      act(() => {
        frame = next
        fns.forEach(fn => fn())
      }),
  }
}

function renderRow(o: { label?: string; number?: number | null; standby?: boolean; frame?: WindowFrameSource } = {}) {
  return render(
    <TitleRow
      label={o.label ?? 'main'}
      number={o.number === undefined ? 1 : o.number}
      standby={o.standby}
      frame={o.frame ?? fakeFrame().source}
    >
      <div data-testid="tab-bar" />
    </TitleRow>,
  )
}

const row = () => screen.getByTestId('title-row')
const button = (id: 'caption-minimize' | 'caption-maximize' | 'caption-close') => screen.getByTestId(id)
const controlCalls = () => invokeMock.mock.calls.filter(c => c[0] === 'window_control')
const DRAG = 'data-tauri-drag-region'

beforeEach(() => {
  invokeMock.mockClear()
  __resetMinSizeReportForTest()
})
afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

describe('TitleRow — 배치 · 끌 자리', () => {
  it('줄 순서 = 탭바 → 끌 빈 자리 → 번호 표식 → 창 단추(최소화 → 최대화 → 셋째)', () => {
    renderRow({ number: 4 })
    expect([...row().children].map(el => (el as HTMLElement).dataset.testid)).toEqual([
      'tab-bar',
      'title-grab-gap',
      'window-number',
      'caption-buttons',
    ])
    expect([...screen.getByTestId('caption-buttons').children].map(el => (el as HTMLElement).dataset.testid)).toEqual([
      'caption-minimize',
      'caption-maximize',
      'caption-close',
    ])
  })

  it('끌 자리 = 줄 자신 · 끌 빈 자리(값 없는 속성 — 자기 자신만) · 표식 · 단추 · 탭바 자리에는 없다', () => {
    renderRow()
    expect(row().getAttribute(DRAG)).toBe('')
    expect(screen.getByTestId('title-grab-gap').getAttribute(DRAG)).toBe('')
    for (const id of ['window-number', 'caption-buttons', 'caption-minimize', 'caption-maximize', 'caption-close', 'tab-bar']) {
      expect(screen.getByTestId(id).hasAttribute(DRAG), id).toBe(false)
    }
  })

  it('탭바만 줄어든다 — 끌 빈 자리(42) · 표식 · 단추는 줄지 않는다', () => {
    renderRow()
    const gap = screen.getByTestId('title-grab-gap').style
    expect([gap.flexGrow, gap.flexShrink, gap.flexBasis]).toEqual(['0', '0', '42px'])
    for (const id of ['window-number', 'caption-buttons']) {
      const s = screen.getByTestId(id).style
      expect([s.flexGrow, s.flexShrink], id).toEqual(['0', '0'])
    }
  })

  it('높이 = 보통 위 여백 6 + 탭바 28 · 최대화면 여백 0 · data-maximized · 크롬 모양의 왼 여백', () => {
    const f = fakeFrame()
    renderRow({ frame: f.source })
    expect([row().style.height, row().dataset.maximized, row().style.paddingLeft]).toEqual(['34px', 'false', '0px'])
    f.set({ ...FRAME, maximized: true, caption: { ...FRAME.caption, leading_inset: 70 } })
    expect([row().style.height, row().dataset.maximized, row().style.paddingLeft]).toEqual(['28px', 'true', '70px'])
  })

  it('창 사실을 받기 전 = 보통 높이 · 앞 창 · 단추를 그린다', () => {
    renderRow({ frame: fakeFrame(null).source })
    expect([row().style.height, row().dataset.windowActive]).toEqual(['34px', 'true'])
    expect(screen.getByTestId('caption-buttons')).toBeTruthy()
  })
})

describe('TitleRow — 번호 표식', () => {
  it('글자 = 번호만 · Tab 순서 밖 · 화면 낭독기에서 감춘다 · 누름을 받지 않는다(부모 끌 자리로 간다)', () => {
    renderRow({ number: 7 })
    const marker = screen.getByTestId('window-number')
    expect(marker.textContent).toBe('7')
    expect(marker.hasAttribute('tabindex')).toBe(false)
    expect(marker.getAttribute('aria-hidden')).toBe('true')
    expect(marker.style.pointerEvents).toBe('none')
    expect(marker.style.fontVariantNumeric).toBe('tabular-nums')
  })

  it('폭 = 최소 28 · 글자 양옆 8 — 자리 수만큼 넓어지고 자르지 않는다', () => {
    for (const n of [7, 42, 123]) {
      const view = renderRow({ number: n })
      const s = screen.getByTestId('window-number').style
      expect(screen.getByTestId('window-number').textContent).toBe(String(n))
      expect([s.minWidth, s.paddingLeft, s.paddingRight, s.whiteSpace, s.overflow, s.width], String(n)).toEqual([
        '28px',
        '8px',
        '8px',
        'nowrap',
        '',
        '',
      ])
      view.unmount()
    }
  })
})

describe('TitleRow — 창 단추', () => {
  it('셋 다 type=button · 접근성 이름 = 툴팁 — main = 최소화 · 최대화 · 모든 창 숨기기', () => {
    renderRow()
    expect(
      (['caption-minimize', 'caption-maximize', 'caption-close'] as const).map(id => [
        button(id).getAttribute('type'),
        button(id).getAttribute('aria-label'),
        button(id).getAttribute('title'),
      ]),
    ).toEqual([
      ['button', '최소화', '최소화'],
      ['button', '최대화', '최대화'],
      ['button', '모든 창 숨기기', '모든 창 숨기기'],
    ])
  })

  it('누르면 레지스트리 window.* 명령 → 셸 invoke window_control(이 창 label) — main 셋째 = window.close(셸이 모두 숨긴다)', () => {
    renderRow()
    fireEvent.click(button('caption-minimize'))
    fireEvent.click(button('caption-maximize'))
    fireEvent.click(button('caption-close'))
    expect(controlCalls()).toEqual([
      ['window_control', { window: 'main', action: 'minimize', ifEmpty: null }],
      ['window_control', { window: 'main', action: 'maximize', ifEmpty: null }],
      ['window_control', { window: 'main', action: 'close', ifEmpty: false }],
    ])
  })

  it('최대화 상태 = 둘째 단추 「이전 크기로 복원」(겹친 상자) → window.unmaximize · 사실이 바뀌면 다시 그린다', () => {
    const f = fakeFrame()
    renderRow({ frame: f.source })
    expect(button('caption-maximize').querySelector('svg.lucide-square')).toBeTruthy()
    f.set({ ...FRAME, maximized: true })
    expect(button('caption-maximize').getAttribute('aria-label')).toBe('이전 크기로 복원')
    expect(button('caption-maximize').getAttribute('title')).toBe('이전 크기로 복원')
    expect(button('caption-maximize').querySelector('svg.lucide-copy')).toBeTruthy()
    fireEvent.click(button('caption-maximize'))
    expect(controlCalls()).toEqual([['window_control', { window: 'main', action: 'unmaximize', ifEmpty: null }]])
    f.set({ ...FRAME, maximized: false })
    expect(button('caption-maximize').getAttribute('aria-label')).toBe('최대화')
  })

  it('main 셋째 = 눈 가림 · 옅은 맴돎(빨강 아님) / 팝아웃 셋째 = 「창 닫기」 × · 빨간 맴돎 → window.close', () => {
    const main = renderRow({ label: 'main' })
    const hideAll = button('caption-close')
    expect(hideAll.querySelector('svg.lucide-eye-off')).toBeTruthy()
    expect(hideAll.className.split(' ')).toContain('hover:bg-[var(--caption-hover-bg)]')
    expect(hideAll.className).not.toContain('--caption-close-hover-bg')
    main.unmount()

    renderRow({ label: 'slot-popup-3', number: 2 })
    const close = button('caption-close')
    expect([close.getAttribute('aria-label'), close.getAttribute('title')]).toEqual(['창 닫기', '창 닫기'])
    expect(close.querySelector('svg.lucide-x')).toBeTruthy()
    expect(close.className.split(' ')).toEqual(
      expect.arrayContaining(['hover:bg-[var(--caption-close-hover-bg)]', 'hover:text-[color:var(--caption-close-hover-fg)]']),
    )
    expect(button('caption-minimize').querySelector('svg.lucide-minus')).toBeTruthy()
    fireEvent.click(close)
    expect(controlCalls()).toEqual([['window_control', { window: 'slot-popup-3', action: 'close', ifEmpty: false }]])
  })

  it('맴돎 · 글자색은 클래스로만 — 인라인 색 · 바탕이 없다(인라인이 맴돎을 이긴다) · 포커스 테두리', () => {
    renderRow()
    for (const id of ['caption-minimize', 'caption-maximize', 'caption-close'] as const) {
      const b = button(id)
      expect([b.style.color, b.style.background, b.style.backgroundColor], id).toEqual(['', '', ''])
      expect(b.className.split(' '), id).toEqual(
        expect.arrayContaining(['text-[color:var(--caption-fg)]', 'focus-visible:outline', 'focus-visible:outline-accent']),
      )
    }
  })

  it('크롬 모양이 단추를 OS 몫으로 두면(caption.buttons 거짓) 그리지 않는다 — 표식은 그대로', () => {
    renderRow({ frame: fakeFrame({ ...FRAME, caption: { ...FRAME.caption, buttons: false } }).source })
    expect(screen.queryByTestId('caption-buttons')).toBeNull()
    expect(screen.getByTestId('window-number')).toBeTruthy()
  })
})

describe('TitleRow — 흐림 · 대기 모드', () => {
  it('창 사실 focused → data-window-active · 바뀌면 따라간다', () => {
    const f = fakeFrame()
    renderRow({ frame: f.source })
    expect(row().dataset.windowActive).toBe('true')
    f.set({ ...FRAME, focused: false })
    expect(row().dataset.windowActive).toBe('false')
    f.set({ ...FRAME, focused: true })
    expect(row().dataset.windowActive).toBe('true')
  })

  it('제목 줄 토큰은 세 테마 모두 있다 · 흐림은 탭 글자 · 번호 · 단추 글리프 토큰을 흐림 색으로 갈아 끼운다', () => {
    const css = require('node:fs').readFileSync(`${process.cwd()}/src/styles/theme.css`, 'utf8')
    for (const theme of ['dark', 'light', 'e-ink']) {
      const m = css.match(new RegExp(`:root\\[data-theme='${theme}'\\]\\s*\\{\\s*--tab-text:[^}]*\\}`))
      expect(m, `${theme} 의 제목 줄 블록이 없다`).not.toBeNull()
      for (const token of [
        '--tab-text',
        '--chrome-number-fg',
        '--chrome-inactive-fg',
        '--caption-fg',
        '--caption-hover-bg',
        '--caption-close-hover-bg',
        '--caption-close-hover-fg',
      ]) {
        expect(m![0], `${theme} 에 ${token} 가 없다`).toMatch(new RegExp(`${token}:\\s*[^;]+;`))
      }
    }
    const dim = css.match(/\[data-window-active='false'\]\s*\{[^}]*\}/)
    expect(dim, '흐림 규칙이 없다').not.toBeNull()
    for (const token of ['--tab-text', '--tab-text-inactive', '--chrome-number-fg', '--caption-fg']) {
      expect(dim![0]).toContain(`${token}: var(--chrome-inactive-fg);`)
    }
  })

  it('대기 모드(standby · number null) = 표식 · 단추 없이 줄 · 끌 빈 자리만 — 탭바는 그대로 자식', () => {
    renderRow({ number: null, standby: true })
    expect([...row().children].map(el => (el as HTMLElement).dataset.testid)).toEqual(['tab-bar', 'title-grab-gap'])
    expect(row().getAttribute(DRAG)).toBe('')
  })

  // 로딩 · 부팅 실패 화면 — OS 틀이 없어 창을 옮기고 닫는 자리가 이 줄뿐이다(`WindowLayout`).
  it('번호가 아직 없는 보통 화면 = 표식만 빼고 창 단추는 그린다', () => {
    renderRow({ number: null })
    expect([...row().children].map(el => (el as HTMLElement).dataset.testid)).toEqual([
      'tab-bar',
      'title-grab-gap',
      'caption-buttons',
    ])
  })
})

// TRD S26-window-chrome §7-7 — 셈의 숫자는 `metrics.test.ts` 가 본다. 여기는 무엇을 재서 언제 보내나.
describe('TitleRow — 최소 창 크기 보고', () => {
  // 순번 · epoch 는 `minSizeReport.test.ts` 가 본다 — 여기는 크기만.
  const minReports = () =>
    invokeMock.mock.calls
      .filter(c => c[0] === 'report_window_min_width')
      .map(c => {
        const { width, height } = c[1] as { width: number; height: number }
        return { width, height }
      })
  /** 맡은 보고가 다 나갈 때까지 — 그 사이 다음 값을 맡으면 앞 값은 보내지 않는다. */
  const settle = () => act(() => minWindowSizeSettled())

  // 표식 폭 = 글자 7 px × 자리 수 + 양옆 8(그린 그대로 — jsdom 은 배치를 하지 않는다).
  function measureMarkerByDigits(): void {
    const real = HTMLElement.prototype.getBoundingClientRect
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
      if (this.dataset.testid !== 'window-number') return real.call(this)
      const width = 16 + 7 * (this.textContent ?? '').length
      return { left: 0, top: 0, width, height: 28, right: width, bottom: 28, x: 0, y: 0 } as DOMRect
    })
  }

  function rowWith(number: number | null, frame: WindowFrameSource = fakeFrame().source, standby = false) {
    return (
      <TitleRow label="slot-popup-1" number={number} standby={standby} frame={frame}>
        <div data-testid="tab-bar" />
      </TitleRow>
    )
  }

  it('마운트 때 보낸다 — 표식 폭은 재고 단추 셋을 싣는다 · 다시 그려도 같은 값은 다시 보내지 않는다', async () => {
    const f = fakeFrame()
    renderRow({ frame: f.source })
    await settle()
    f.set({ ...FRAME, focused: false })
    await settle()
    expect(minReports()).toEqual([{ width: 286, height: 64 }])
  })

  it('번호 자리 수가 바뀌면 표식 폭을 다시 재서 보낸다(한 · 두 · 세 자리)', async () => {
    measureMarkerByDigits()
    const view = render(rowWith(9))
    await settle()
    view.rerender(rowWith(10))
    await settle()
    view.rerender(rowWith(100))
    await settle()
    expect(minReports()).toEqual([
      { width: 286, height: 64 },
      { width: 288, height: 64 },
      { width: 295, height: 64 },
    ])
  })

  // 셸은 창을 열 때 세운 열기 표시를 그 창의 다음 보고에서 거둔다 — 대기 중 보고가 들일 때의 표시를 먼저 거두면 들인 창이
  //   넓혀 당겨지지 않는다(`TitleRow` 머리 · `src-tauri/src/chrome/book.rs` 머리).
  it('번호가 없으면(대기 · 로딩 · 부팅 실패) 보내지 않는다 · 번호를 받으면 표식 · 단추를 실어 처음 보낸다', async () => {
    const f = fakeFrame()
    const view = render(rowWith(null, f.source, true))
    await settle()
    view.rerender(rowWith(null, f.source))
    await settle()
    expect(minReports()).toEqual([])
    view.rerender(rowWith(3, f.source))
    await settle()
    expect(minReports()).toEqual([{ width: 286, height: 64 }])
  })

  it('크롬 모양이 바뀌면 다시 보낸다 — 단추를 OS 가 그리면 단추 몫을 빼고 · 왼 여백은 더한다', async () => {
    const f = fakeFrame()
    render(rowWith(1, f.source))
    await settle()
    f.set({ ...FRAME, caption: { ...FRAME.caption, buttons: false } })
    await settle()
    f.set({ ...FRAME, caption: { ...FRAME.caption, leading_inset: 80 } })
    await settle()
    expect(minReports()).toEqual([
      { width: 286, height: 64 },
      { width: 148, height: 64 },
      { width: 366, height: 64 },
    ])
  })

  it('최대화해도 보내는 값은 그대로다(보통 상태 기준 — 셸은 보통 크기 창에만 입힌다)', async () => {
    const f = fakeFrame()
    render(rowWith(1, f.source))
    await settle()
    f.set({ ...FRAME, maximized: true })
    await settle()
    expect(minReports()).toEqual([{ width: 286, height: 64 }])
  })
})
