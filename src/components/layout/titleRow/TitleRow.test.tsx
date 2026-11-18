// The title row's share of TRD S26-window-chrome §10-3 — layout order · draggable area attributes · number badge · window buttons (registry commands · state · accessibility ·
//   main/popout third) · dimming · waiting mode · chrome shape. The tab bar itself is a placeholder child (the tab bar's share = `TabBar.test.tsx`).

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

import '../../../commands/tabCommands' // Registration of `window.*` that the buttons call (in production `commands/contributions.ts`)
import type { WindowFrame, WindowFrameSource } from '../dock/windowFrame'
import { __resetMinSizeReportForTest, minWindowSizeSettled } from './minSizeReport'
import TitleRow from './TitleRow'

// Reads the theme.css source with Node fs — the reason and declaration shape are the same as `store/chatStyleStore.test.ts`.
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

describe('TitleRow — layout · drag spot', () => {
  it('row order = tab bar → empty drag spot → number marker → window buttons (minimize → maximize → third)', () => {
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

  it('drag spot = the row itself · empty drag spot (a valueless attribute — itself only) · absent on marker · button · tab bar spot', () => {
    renderRow()
    expect(row().getAttribute(DRAG)).toBe('')
    expect(screen.getByTestId('title-grab-gap').getAttribute(DRAG)).toBe('')
    for (const id of ['window-number', 'caption-buttons', 'caption-minimize', 'caption-maximize', 'caption-close', 'tab-bar']) {
      expect(screen.getByTestId(id).hasAttribute(DRAG), id).toBe(false)
    }
  })

  it('only the tab bar shrinks — empty drag spot (42) · marker · buttons do not shrink', () => {
    renderRow()
    const gap = screen.getByTestId('title-grab-gap').style
    expect([gap.flexGrow, gap.flexShrink, gap.flexBasis]).toEqual(['0', '0', '42px'])
    for (const id of ['window-number', 'caption-buttons']) {
      const s = screen.getByTestId(id).style
      expect([s.flexGrow, s.flexShrink], id).toEqual(['0', '0'])
    }
  })

  it('height = normal top margin 6 + tab bar 28 · maximized margin 0 · data-maximized · left margin of the chrome shape', () => {
    const f = fakeFrame()
    renderRow({ frame: f.source })
    expect([row().style.height, row().dataset.maximized, row().style.paddingLeft]).toEqual(['34px', 'false', '0px'])
    f.set({ ...FRAME, maximized: true, caption: { ...FRAME.caption, leading_inset: 70 } })
    expect([row().style.height, row().dataset.maximized, row().style.paddingLeft]).toEqual(['28px', 'true', '70px'])
  })

  it('before receiving the window fact = normal height · front window · draws buttons', () => {
    renderRow({ frame: fakeFrame(null).source })
    expect([row().style.height, row().dataset.windowActive]).toEqual(['34px', 'true'])
    expect(screen.getByTestId('caption-buttons')).toBeTruthy()
  })
})

describe('TitleRow — number marker', () => {
  it('text = number only · outside Tab order · hidden from the screen reader · does not take a press (goes to the parent drag spot)', () => {
    renderRow({ number: 7 })
    const marker = screen.getByTestId('window-number')
    expect(marker.textContent).toBe('7')
    expect(marker.hasAttribute('tabindex')).toBe(false)
    expect(marker.getAttribute('aria-hidden')).toBe('true')
    expect(marker.style.pointerEvents).toBe('none')
    expect(marker.style.fontVariantNumeric).toBe('tabular-nums')
  })

  it('width = minimum 28 · 8 on each side of the text — widens by the number of digits and does not clip', () => {
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

describe('TitleRow — window buttons', () => {
  it('all three type=button · accessible name = tooltip — main = minimize · maximize · hide all windows', () => {
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

  it('pressing → registry window.* command → shell invoke window_control (this window label) — main third = window.close (the shell hides all)', () => {
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

  it('maximized state = second button 「restore previous size」 (overlapping boxes) → window.unmaximize · redraws when the fact changes', () => {
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

  it('main third = eye cover · faint hover (not red) / popout third = 「close window」 × · red hover → window.close', () => {
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

  it('hover · text color only by class — no inline color · background (inline would beat hover) · focus border', () => {
    renderRow()
    for (const id of ['caption-minimize', 'caption-maximize', 'caption-close'] as const) {
      const b = button(id)
      expect([b.style.color, b.style.background, b.style.backgroundColor], id).toEqual(['', '', ''])
      expect(b.className.split(' '), id).toEqual(
        expect.arrayContaining(['text-[color:var(--caption-fg)]', 'focus-visible:outline', 'focus-visible:outline-accent']),
      )
    }
  })

  // TRD §8-4 — the details of reporting · hover are covered by `snapOverlay.test.ts`. Here, whether the window fact field opens and closes that path.
  it('snap overlay — if caption.snap_overlay is false (default build) it does not report the maximize button rectangle · if true it reports and withdraws on release', () => {
    const snapCalls = () => invokeMock.mock.calls.filter(c => c[0] === 'report_max_button_rect')
    const off = renderRow()
    expect(snapCalls()).toEqual([])
    expect(button('caption-maximize').dataset.snapHover).toBe('false')
    expect(button('caption-maximize').className.split(' ')).toContain('data-[snap-hover=true]:bg-[var(--caption-hover-bg)]')
    off.unmount()
    expect(snapCalls()).toEqual([])

    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue(DOMRect.fromRect({ x: 900, y: 0, width: 46, height: 34 }))
    const on = renderRow({ frame: fakeFrame({ ...FRAME, caption: { ...FRAME.caption, snap_overlay: true } }).source })
    expect(snapCalls()).toEqual([['report_max_button_rect', { rect: { x: 900, y: 0, width: 46, height: 34 } }]])
    on.unmount()
    expect(snapCalls().slice(-1)[0]).toEqual(['report_max_button_rect', { rect: null }])
  })

  it('if the chrome shape leaves buttons to the OS (caption.buttons false) it does not draw them — the marker as is', () => {
    renderRow({ frame: fakeFrame({ ...FRAME, caption: { ...FRAME.caption, buttons: false } }).source })
    expect(screen.queryByTestId('caption-buttons')).toBeNull()
    expect(screen.getByTestId('window-number')).toBeTruthy()
  })
})

describe('TitleRow — dimming · waiting mode', () => {
  it('window fact focused → data-window-active · follows when it changes', () => {
    const f = fakeFrame()
    renderRow({ frame: f.source })
    expect(row().dataset.windowActive).toBe('true')
    f.set({ ...FRAME, focused: false })
    expect(row().dataset.windowActive).toBe('false')
    f.set({ ...FRAME, focused: true })
    expect(row().dataset.windowActive).toBe('true')
  })

  it('title row tokens exist in all three themes · dimming swaps the tab text · number · button glyph tokens to the dim color', () => {
    const css = require('node:fs').readFileSync(`${process.cwd()}/src/styles/theme.css`, 'utf8')
    for (const theme of ['dark', 'light', 'e-ink']) {
      const m = css.match(new RegExp(`:root\\[data-theme='${theme}'\\]\\s*\\{\\s*--tab-text:[^}]*\\}`))
      expect(m, `${theme} has no title row block`).not.toBeNull()
      for (const token of [
        '--tab-text',
        '--chrome-number-fg',
        '--chrome-inactive-fg',
        '--caption-fg',
        '--caption-hover-bg',
        '--caption-close-hover-bg',
        '--caption-close-hover-fg',
      ]) {
        expect(m![0], `${theme} has no ${token}`).toMatch(new RegExp(`${token}:\\s*[^;]+;`))
      }
    }
    const dim = css.match(/\[data-window-active='false'\]\s*\{[^}]*\}/)
    expect(dim, 'no dim rule').not.toBeNull()
    for (const token of ['--tab-text', '--tab-text-inactive', '--chrome-number-fg', '--caption-fg']) {
      expect(dim![0]).toContain(`${token}: var(--chrome-inactive-fg);`)
    }
  })

  it('waiting mode (standby · number null) = only row · empty drag spot without marker · buttons — the tab bar stays as a child', () => {
    renderRow({ number: null, standby: true })
    expect([...row().children].map(el => (el as HTMLElement).dataset.testid)).toEqual(['tab-bar', 'title-grab-gap'])
    expect(row().getAttribute(DRAG)).toBe('')
  })

  // Loading · boot failure screens — with no OS frame, this row is the only place to move and close the window (`WindowLayout`).
  it('a normal screen with no number yet = draws the window buttons, only the marker omitted', () => {
    renderRow({ number: null })
    expect([...row().children].map(el => (el as HTMLElement).dataset.testid)).toEqual([
      'tab-bar',
      'title-grab-gap',
      'caption-buttons',
    ])
  })
})

// TRD S26-window-chrome §7-7 — the numbers of the computation are covered by `metrics.test.ts`. Here, what is measured and when it is sent.
describe('TitleRow — minimum window size report', () => {
  // Sequence number · epoch are covered by `minSizeReport.test.ts` — here only the size.
  const minReports = () =>
    invokeMock.mock.calls
      .filter(c => c[0] === 'report_window_min_width')
      .map(c => {
        const { width, height } = c[1] as { width: number; height: number }
        return { width, height }
      })
  /** Until all the reports in hand have gone out — if the next value is taken in the meantime, the earlier value is not sent. */
  const settle = () => act(() => minWindowSizeSettled())

  // Badge width = 7 px per character × number of digits + 8 on each side (as drawn — jsdom does no layout).
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

  it('sends at mount — measures the marker width and carries three buttons · redrawing with the same value does not resend', async () => {
    const f = fakeFrame()
    renderRow({ frame: f.source })
    await settle()
    f.set({ ...FRAME, focused: false })
    await settle()
    expect(minReports()).toEqual([{ width: 286, height: 64 }])
  })

  it('if the number of digits changes it re-measures the marker width and sends (one · two · three digits)', async () => {
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

  // The shell withdraws the open mark set when it opened the window at that window's next report — if a report during waiting withdraws the adoption-time mark first, the adopted window is not
  //   pulled wider (`TitleRow` head · head of `src-tauri/src/chrome/book.rs`).
  it('with no number (waiting · loading · boot failure) it does not send · once a number is received it sends the first time carrying marker · buttons', async () => {
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

  it('if the chrome shape changes it sends again — if the OS draws the buttons the button share is subtracted · the left margin is added', async () => {
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

  it('even when maximized the sent value is the same (based on the normal state — the shell applies it only to a normal-size window)', async () => {
    const f = fakeFrame()
    render(rowWith(1, f.source))
    await settle()
    f.set({ ...FRAME, maximized: true })
    await settle()
    expect(minReports()).toEqual([{ width: 286, height: 64 }])
  })
})
