// Global stylesheet regression gate — no global resets outside layers (@layer).
//
// ★Why block it with a test★: in the CSS cascade, **an unlayered rule beats every layered rule.** So
//   if `*, *::before, *::after { padding: 0; margin: 0 }` is put outside layers, the Tailwind
//   spacing utilities of `@layer utilities` (all of px-*·py-*·m-*) quietly die app-wide — the classes are attached and only the computed value is 0px,
//   so it does not show unless you look at the screen, and unit tests stay green (it actually happened 2026-08-18, the spacing of 7 files
//   was being drawn as 0).
//
// ★The reset itself is not needed★: Tailwind preflight already applies the same reset inside `@layer base`
//   (node_modules/tailwindcss/preflight.css — box-sizing/margin/padding/border on `*, ::after, ::before, ::backdrop, ::file-selector-button`).
//   So writing it again here is a duplicate and only breaks the priority.
//
// Check method = source string. What this file protects is "that declaration does not exist", so it cannot
//   be observed by a jsdom render (jsdom does not compute cascade · layers).

import { describe, expect, it } from 'vitest'

// ★Read with `?raw` (not node:fs)★: this project's tsconfig has no node types (@types/node not installed),
//   so using fs · process breaks the `tsc --noEmit` gate. `?raw` is declared as string by the vite/client types and
//   Vite gives the original text as is with no CSS processing.
import cssSource from './index.css?raw'

/** Remainder after deleting whole `@layer <이름> { ... }` blocks = text left with only unlayered rules. */
function unlayeredPart(css: string): string {
  let out = ''
  let i = 0
  while (i < css.length) {
    const at = css.indexOf('@layer', i)
    if (at < 0) {
      out += css.slice(i)
      break
    }
    out += css.slice(i, at)
    const open = css.indexOf('{', at)
    if (open < 0) break // A declaration statement like `@layer a, b;` — there is no block, so stop here.
    let depth = 0
    let j = open
    for (; j < css.length; j += 1) {
      if (css[j] === '{') depth += 1
      else if (css[j] === '}') {
        depth -= 1
        if (depth === 0) break
      }
    }
    i = j + 1
  }
  return out
}

describe('index.css — 계층 밖 전역 리셋 금지', () => {
  it('계층 밖에 전역 * 선택자 padding/margin 리셋이 없다', () => {
    const unlayered = unlayeredPart(cssSource)

    // Check after stripping comments — so the comment explaining this convention is not itself a false positive.
    const code = unlayered.replace(/\/\*[\s\S]*?\*\//g, '')

    // Global reset = a selector block starting with `*`. Only the padding/margin declarations inside it are at issue.
    const globalBlocks = [...code.matchAll(/(^|[},])\s*\*[^{}]*\{([^{}]*)\}/g)].map(m => m[2])
    const offenders = globalBlocks.filter(body => /(^|;|\s)(padding|margin)\s*:/.test(body))

    expect(offenders).toEqual([])
  })

  it('Tailwind 진입점이 남아 있다(preflight 가 리셋을 대신하는 전제)', () => {
    expect(cssSource).toContain('@import "tailwindcss"')
  })
})

// ─────────────────────────────────────────────────────────────────────────────
// xterm scrollbar visibility = the same rule as the ScrollArea seam (ADR-0053)
//
// ★Why measure by source string★: jsdom does not compute the ::-webkit-scrollbar cascade so it cannot be observed by
//   rendering (same reason as the gate above). What we protect is also the "shape of the declaration" — that the thumb color turns on **only through the scrolling marker**,
//   and that a rule reviving it on hover has not reappeared.
// The side that attaches the marker (the JS half) is measured by components/ui/nativeScrollActivity.test.ts.
// ─────────────────────────────────────────────────────────────────────────────
describe('index.css — xterm 스크롤바는 스크롤 중에만 보인다', () => {
  // Strip comments — so the comment explaining this convention is not itself a false positive (same shape as the gate above).
  const code = cssSource.replace(/\/\*[\s\S]*?\*\//g, '')

  /** `.xterm-viewport ... ::-webkit-scrollbar-thumb { ... }` rules = list of [selector, body]. */
  const thumbRules = [...code.matchAll(/([^{}]*::-webkit-scrollbar-thumb)\s*\{([^{}]*)\}/g)]
    .map(m => ({ selector: m[1].trim(), body: m[2] }))
    .filter(r => r.selector.includes('xterm-viewport'))

  it('thumb 색을 켜는 규칙은 전부 스크롤 중 표식으로 게이트된다', () => {
    expect(thumbRules.length).toBeGreaterThan(0)
    const ungated = thumbRules.filter(
      r => /--scrollbar-thumb/.test(r.body) && !r.selector.includes('[data-scroll-active]'),
    )
    // If a color is given without the gate, it goes back to always shown (= the state before unification).
    expect(ungated.map(r => r.selector)).toEqual([])
  })

  it('스크롤 중에는 색을 켜는 규칙이 실제로 있다(영구 비표시 = 세 번째 룰 금지)', () => {
    const gated = thumbRules.filter(
      r => r.selector.includes('[data-scroll-active]') && /--scrollbar-thumb/.test(r.body),
    )
    expect(gated.length).toBeGreaterThan(0)
  })

  it('hover 로 스크롤바를 되살리는 규칙이 없다', () => {
    const offenders = [...code.matchAll(/[^{}]*\{[^{}]*\}/g)]
      .map(m => m[0])
      .filter(r => /xterm-viewport/.test(r) && /scrollbar/.test(r) && /:hover/.test(r))
    expect(offenders).toEqual([])
  })
})

// ─────────────────────────────────────────────────────────────────────────────
// The thickness of the thumb (.slider) xterm actually paints = the same formula as the seam thumb (ADR-0053)
//
// ★What it protects★: that the thickness is not a hard-coded number but a **derived value**. The seam's thumb is painted at the track
//   width (--scrollbar-size) minus the padding of .engram-scrollbar on both sides, and xterm's .slider must carry
//   that formula over as is to get the same thickness. Hard-coding a number makes the two quietly diverge when the token changes.
// ★Why we do not cross-check against the seam's real values (scroll-area.css·theme.css)★: @tailwindcss/vite gives the `?raw` of a CSS that is not the entry
//   (index.css) as an empty string (measured). So all this file can see is
//   index.css, and here we measure only up to **the formula's internal consistency** (subtracted width = margin×2). We do not take the detour of reading with fs
//   — @types/node is not installed so the `tsc --noEmit` gate breaks (same reason as the `?raw` comment above).
// ─────────────────────────────────────────────────────────────────────────────
describe('index.css — xterm .slider 두께는 seam thumb 과 같은 산식', () => {
  const code = cssSource.replace(/\/\*[\s\S]*?\*\//g, '')

  const sliderRule = /\.scrollbar\.vertical\s*>\s*\.slider\s*\{([^{}]*)\}/.exec(code)

  it('.slider 폭이 --scrollbar-size 파생식이고 좌우 여백이 대칭이다', () => {
    expect(sliderRule).not.toBeNull()
    const calc = /width:\s*calc\(\s*var\(--scrollbar-size,\s*[\d.]+px\)\s*-\s*([\d.]+)px\)/.exec(
      sliderRule![1],
    )
    const right = /(?:^|;|\s)right:\s*([\d.]+)px/.exec(sliderRule![1])
    expect(calc).not.toBeNull()
    expect(right).not.toBeNull()
    // Width subtracted from the track = right margin × 2 — a relationship carried over from the seam's `padding: 2px`.
    expect(Number(calc![1])).toBe(Number(right![1]) * 2)
  })

  it('폭·위치 선언은 !important 를 유지한다', () => {
    // xterm JS plants width/left as inline style (verticalScrollbarSize), so if !important is missing it quietly goes back to the default 14px
    // — a cascade failure, so type checks · render tests do not catch it.
    expect(/width:[^;]*!important/.test(sliderRule![1])).toBe(true)
    expect(/left:[^;]*!important/.test(sliderRule![1])).toBe(true)
  })

  it('트랙(.scrollbar.vertical) 자체의 폭은 건드리지 않는다', () => {
    // The track is the hit area and is paired with the value xterm uses as its reserved width, so narrowing it is a separate matter to decide.
    // What is touched here must be only the painted width.
    const offenders = [...code.matchAll(/([^{}]*)\{([^{}]*)\}/g)]
      .map(m => ({ selector: m[1].trim(), body: m[2] }))
      .filter(r => /\.scrollbar\.vertical/.test(r.selector) && !/\.slider/.test(r.selector))
      .filter(r => /(?:^|;|\s)width:/.test(r.body))
    expect(offenders.map(r => r.selector)).toEqual([])
  })
})

// ─────────────────────────────────────────────────────────────────────────────
// Area frame position = the shell rectangle rounded in CSS (ADR-0227, TRD §2e)
//
// ★What it protects★: a leaf carries only `--x0 --y0 --x1 --y1` inline, and the one rule that turns them into position · size is here alone.
//   If the rule is missing, all areas sit at the origin with no size, and if the rounding form disagrees with the shell's px boundary formula, the area px the shell computed and the
//   screen diverge. jsdom does not compute cascade · `@supports` · `round()`, so we measure by source string (same reason as the gate above).
//   Whether the frame gets this class is measured by ViewLayoutRenderer.flat.test.tsx.
// ─────────────────────────────────────────────────────────────────────────────
describe('index.css — 칸 틀 위치 규칙', () => {
  const code = cssSource.replace(/\/\*[\s\S]*?\*\//g, '')
  const SUPPORTS = '@supports (width: round(nearest, 1.5px, 1px))'

  /** The inside and end position of a block `{ ... }` starting at `from`. */
  function blockAt(css: string, from: number): { inner: string; end: number } {
    const open = css.indexOf('{', from)
    let depth = 0
    for (let j = open; j < css.length; j += 1) {
      if (css[j] === '{') depth += 1
      else if (css[j] === '}') {
        depth -= 1
        if (depth === 0) return { inner: css.slice(open + 1, j), end: j + 1 }
      }
    }
    throw new Error('블록이 닫히지 않았다')
  }

  /** The declaration list of the `.engram-slot-frame { ... }` rules (whitespace normalized). */
  function frameRules(css: string): string[][] {
    return [...css.matchAll(/\.engram-slot-frame\s*\{([^{}]*)\}/g)].map(m =>
      m[1]
        .split(';')
        .map(d => d.replace(/\s+/g, ' ').trim())
        .filter(d => d !== ''),
    )
  }

  const at = code.indexOf(SUPPORTS)
  const supports = at < 0 ? null : blockAt(code, at)
  const outside = supports === null ? code : code.slice(0, at) + code.slice(supports.end)

  it('기본 규칙은 네 경계를 반올림 없는 % 로 놓는다', () => {
    expect(frameRules(outside)).toEqual([
      [
        'left: calc(var(--x0) * 100%)',
        'top: calc(var(--y0) * 100%)',
        'width: calc((var(--x1) - var(--x0)) * 100%)',
        'height: calc((var(--y1) - var(--y0)) * 100%)',
      ],
    ])
  })

  // A declaration mixed with var() becomes unset at computed-value time, so a fallback writing the same property twice does not work — separate them with @supports.
  it('round() 지원 시 규칙은 @supports 안에서 셸의 px 경계식과 같은 모양으로 반올림한다', () => {
    expect(supports).not.toBeNull()
    expect(frameRules(supports!.inner)).toEqual([
      [
        'left: round(nearest, calc(var(--x0) * 100%), 1px)',
        'top: round(nearest, calc(var(--y0) * 100%), 1px)',
        'width: calc(round(nearest, calc(var(--x1) * 100%), 1px) - round(nearest, calc(var(--x0) * 100%), 1px))',
        'height: calc(round(nearest, calc(var(--y1) * 100%), 1px) - round(nearest, calc(var(--y0) * 100%), 1px))',
      ],
    ])
  })
})
