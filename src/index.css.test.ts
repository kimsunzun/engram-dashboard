// 전역 스타일시트 회귀 게이트 — 계층(@layer) 밖 전역 리셋 금지.
//
// ★왜 테스트로 막나★: CSS 캐스케이드에서 **계층 밖(unlayered) 규칙은 모든 계층 규칙을 이긴다.** 그래서
//   `*, *::before, *::after { padding: 0; margin: 0 }` 를 계층 밖에 두면 `@layer utilities` 의 Tailwind
//   여백 유틸(px-*·py-*·m-* 전부)이 앱 전역에서 조용히 죽는다 — 클래스는 붙어 있고 계산값만 0px 이라
//   화면을 보지 않으면 드러나지 않고, 단위테스트는 초록으로 남는다(실발생 2026-08-18, 7개 파일 여백이
//   0 으로 그려지고 있었다).
//
// ★리셋 자체가 필요 없다★: Tailwind preflight 가 `@layer base` 안에서 같은 리셋을 이미 적용한다
//   (node_modules/tailwindcss/preflight.css — `*, ::after, ::before, ::backdrop, ::file-selector-button`
//   에 box-sizing/margin/padding/border). 그래서 여기 다시 쓰면 중복이면서 우선순위만 망가뜨린다.
//
// 검사 방식 = 소스 문자열. 이 파일이 지키려는 것은 "그 선언이 존재하지 않음" 이라 jsdom 렌더로는
//   관측할 수 없다(jsdom 은 캐스케이드·계층을 계산하지 않는다).

import { describe, expect, it } from 'vitest'

// ★`?raw` 로 읽는다(node:fs 아님)★: 이 프로젝트 tsconfig 에는 node 타입이 없어(@types/node 미설치)
//   fs·process 를 쓰면 `tsc --noEmit` 게이트가 깨진다. `?raw` 는 vite/client 타입이 string 으로 선언하고
//   Vite 가 CSS 처리 없이 원문을 그대로 준다.
import cssSource from './index.css?raw'

/** `@layer <이름> { ... }` 블록을 통째로 지운 나머지 = 계층 밖 규칙만 남은 텍스트. */
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
    if (open < 0) break // `@layer a, b;` 같은 선언문 — 블록이 없으니 여기서 멈춘다.
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

    // 주석 제거 후 검사 — 이 규약을 설명하는 주석 자체가 오탐되지 않게 한다.
    const code = unlayered.replace(/\/\*[\s\S]*?\*\//g, '')

    // 전역 리셋 = `*` 로 시작하는 선택자 블록. 그 안의 padding/margin 선언만 문제 삼는다.
    const globalBlocks = [...code.matchAll(/(^|[},])\s*\*[^{}]*\{([^{}]*)\}/g)].map(m => m[2])
    const offenders = globalBlocks.filter(body => /(^|;|\s)(padding|margin)\s*:/.test(body))

    expect(offenders).toEqual([])
  })

  it('Tailwind 진입점이 남아 있다(preflight 가 리셋을 대신하는 전제)', () => {
    expect(cssSource).toContain('@import "tailwindcss"')
  })
})

// ─────────────────────────────────────────────────────────────────────────────
// xterm 스크롤바 가시성 = ScrollArea seam 과 같은 룰 (ADR-0053)
//
// ★왜 소스 문자열로 재나★: jsdom 은 ::-webkit-scrollbar 캐스케이드를 계산하지 않아 렌더로는 관측이
//   안 된다(위 게이트와 같은 사유). 지키려는 것도 "선언의 형태" 다 — thumb 색이 **스크롤 중 표식으로만**
//   켜지는지, hover 로 되살리는 규칙이 다시 생기지 않았는지.
// 표식을 붙이는 쪽(JS 절반)은 components/ui/nativeScrollActivity.test.ts 가 잰다.
// ─────────────────────────────────────────────────────────────────────────────
describe('index.css — xterm 스크롤바는 스크롤 중에만 보인다', () => {
  // 주석 제거 — 이 규약을 설명하는 주석 자체가 오탐되지 않게 한다(위 게이트와 동형).
  const code = cssSource.replace(/\/\*[\s\S]*?\*\//g, '')

  /** `.xterm-viewport ... ::-webkit-scrollbar-thumb { ... }` 규칙 = [선택자, 본문] 목록. */
  const thumbRules = [...code.matchAll(/([^{}]*::-webkit-scrollbar-thumb)\s*\{([^{}]*)\}/g)]
    .map(m => ({ selector: m[1].trim(), body: m[2] }))
    .filter(r => r.selector.includes('xterm-viewport'))

  it('thumb 색을 켜는 규칙은 전부 스크롤 중 표식으로 게이트된다', () => {
    expect(thumbRules.length).toBeGreaterThan(0)
    const ungated = thumbRules.filter(
      r => /--scrollbar-thumb/.test(r.body) && !r.selector.includes('[data-scroll-active]'),
    )
    // 게이트 없이 색을 주면 상시 표시(= 통일 전 상태)로 되돌아간다.
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
