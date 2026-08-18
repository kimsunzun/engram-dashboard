// ADR-0137: 호출부 배선 게이트 — debug 를 짓는 **모든 문서화된 경로**가 `scripts/build-client-shell.mjs`
//   를 부르는지, 그리고 생 `cargo build -p engram-dashboard` 로 되돌아가지 않았는지 소스 텍스트로 본다.
//
// ★왜 이 축이 따로 필요한가(지우지 마라)★: 빌드 산출물 대조(그 스크립트 안)는 **누가 스크립트를 부를
//   때만** 돈다 — 호출 자체가 사라지면 아무 신호도 남지 않는다. 실제로 그렇게 회귀했다(473e82f:
//   런처의 `npm run tauri dev` 가 생 `cargo build` 로 바뀌면서 오버레이 주입이 통째로 빠졌고, 전 게이트가
//   초록인 채였다). 그 커밋을 잡을 수 있었던 유일한 형태가 이 단언이다.
// 판정 방식은 이 리포의 다른 격리 게이트와 같다 — 소스 텍스트 대조(코어 `use tauri`·messaging·net·
//   ADR-0130). "단위테스트로는 배선을 못 본다" 는 말은 사실이 아니다.
// ★주석 줄은 세지 않는다★: 아래 파일들은 "생 cargo build 로 되돌리지 마라" 를 주석으로 적고 있어서,
//   문자열이 있는지만 보면 그 경고문 자체가 위반으로 잡힌다. 실행되는 줄만 본다.
// 배치 근거(콜로케이션 규약의 예외)는 `devConfigOverlay.test.ts` 헤더에 한 번만 적는다 — 여기 복제하지
//   않는다.
import { describe, expect, it } from 'vitest'

// 이 파일만 Node 표준 모듈을 쓴다 — 파일 스코프 ambient 선언(`src/store/chatStyleStore.test.ts` 방식).
declare function require(id: string): {
  readFileSync(path: string, enc: string): string
  join(...parts: string[]): string
}
declare const process: { cwd(): string }

const { readFileSync } = require('node:fs')
const { join } = require('node:path')

const ROOT = process.cwd()

const BUILD_SCRIPT = 'build-client-shell.mjs'
// 클라이언트 셸을 직접 짓는 형태만 잡는다 — `-p engram-dashboard-daemon`·`-core` 등 형제 crate 는
//   정당하므로 이름 뒤에 이어지는 문자가 없을 때만 매치한다.
const BARE_CLIENT_BUILD = /cargo\s+build\b[^\n]*-p\s+engram-dashboard(?![-\w])/

function read(relPath: string): string {
  return readFileSync(join(ROOT, relPath), 'utf8')
}

// `.bat` 의 실행 줄 = REM/:: 주석이 아닌 줄.
function batExecutableLines(text: string): string {
  return text
    .split(/\r?\n/)
    .filter((line) => !/^\s*(@?rem\b|::)/i.test(line))
    .join('\n')
}

// 마크다운의 실행 줄 = 코드 펜스 안의 줄에서 트레일링 `#` 주석을 뗀 것. 산문 문단은 경고문을 담으므로
//   대상이 아니다.
function fencedExecutableLines(markdown: string): string {
  const lines = markdown.split(/\r?\n/)
  const out: string[] = []
  let inFence = false
  for (const line of lines) {
    if (/^\s*```/.test(line)) {
      inFence = !inFence
      continue
    }
    if (inFence) out.push(line.replace(/#.*$/, ''))
  }
  return out.join('\n')
}

const LAUNCHERS = ['scripts/run-debug.bat', 'scripts/rebuild-run-debug.bat']

describe('debug launchers build the client shell through the overlay-injecting script', () => {
  for (const relPath of LAUNCHERS) {
    it(`${relPath} calls ${BUILD_SCRIPT}`, () => {
      expect(batExecutableLines(read(relPath))).toContain(BUILD_SCRIPT)
    })

    it(`${relPath} has no bare client-shell cargo build`, () => {
      expect(batExecutableLines(read(relPath))).not.toMatch(BARE_CLIENT_BUILD)
    })
  }
})

describe('the /qa GUI gate procedure builds the client shell the same way', () => {
  const GATE = '.claude/skill-bindings/qa.md'

  it(`${GATE} step 0 calls ${BUILD_SCRIPT}`, () => {
    expect(fencedExecutableLines(read(GATE))).toContain(BUILD_SCRIPT)
  })

  it(`${GATE} runs no bare client-shell cargo build`, () => {
    // 산문 쪽 경고문("생 cargo build 로 짓지 말 것")은 펜스 밖이라 여기 안 들어온다.
    expect(fencedExecutableLines(read(GATE))).not.toMatch(BARE_CLIENT_BUILD)
  })
})
