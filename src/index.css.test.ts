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
