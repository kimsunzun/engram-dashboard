// ADR-0137: dev 오버레이 헬퍼(`scripts/dev-config-json.mjs`)의 회귀 그물 — debug 셸에 dev identifier 가
//   찍히게 하는 값이 여기서 만들어진다. 이 헬퍼가 느슨해지면 구조만 멀쩡한 값(`{}`)이 통과해 tauri 가
//   release identifier 를 그대로 쓰고, 릴리즈 앱이 떠 있을 때 dev 앱이 창 없이 죽는다.
//
// ★배치 예외 — 여기가 그 근거의 단일 집이다(형제 `launcherWiring.test.ts` 몫도 포함. 거기 복제하지
//   말 것)★: 콜로케이션 규약(`vitest.config.ts` 주석: "테스트는 소스 옆 *.test.ts")의 예외다. 두 파일
//   모두 대상이 `scripts/`·`.claude/` 에 있어 옆에 둘 소스가 `src/` 에 없다. 그런데 vitest include 는
//   `src/**/*.test.ts(x)` 뿐이라 대상 옆에 두면 `npm test` 가 아예 집지 않고, include 를 넓히는 건 이
//   변경의 범위 밖이다. 그래서 "규약을 어긴 자리"가 아니라 **집히는 유일한 자리**를 택했다 —
//   콜로케이션 규약이 이 두 테스트를 수용하지 못한다는 뜻이고, 사이드카 소스는 없다.
// 이 파일이 보는 건 헬퍼의 **값 판정**뿐이다. 호출부 배선(런처·게이트가 실제로 그 경로로 짓는가)은
//   `launcherWiring.test.ts` 가, 주입이 런타임에 무효가 됐는지는 `scripts/build-client-shell.mjs` 의
//   빌드 후 산출물 대조가 본다. 셋이 각각 다른 축이라 하나로 합치면 나머지 둘이 빈다.
import { describe, expect, it } from 'vitest'

// 이 파일만 Node 표준 모듈을 쓴다. 프론트 tsconfig 는 DOM 전용(@types/node 없음)이라 필요한 심볼만
//   **이 파일 스코프로** ambient 선언한다(`src/store/chatStyleStore.test.ts` 와 같은 방식 — 전역
//   @types/node 의존을 만들지 않으려는 것). 파일 전체 억제(`@ts-nocheck`)를 쓰지 않는 이유 = 그러면
//   아래 단언들의 타입 검사까지 같이 꺼진다.
declare function require(id: string): {
  mkdtempSync(prefix: string): string
  writeFileSync(path: string, data: string, enc: string): void
  rmSync(path: string, opts: { recursive: boolean; force: boolean }): void
  readFileSync(path: string, enc: string): string
  tmpdir(): string
  join(...parts: string[]): string
  createRequire(filename: string): (id: string) => unknown
}
declare const process: { cwd(): string }

const { mkdtempSync, writeFileSync, rmSync, readFileSync } = require('node:fs')
const { tmpdir } = require('node:os')
const { join } = require('node:path')
const { createRequire } = require('node:module')

const ROOT = process.cwd()

// ★헬퍼는 Node 순정 로더로 들인다(`import`/`await import` 로 되돌리지 마라)★: 그 둘은 vite 를 거쳐
//   헬퍼까지 변환해 들이는데, 변환된 모듈 안에서는 `import.meta.url` 이 `http:` 라 헬퍼가 로드 도중
//   `fileURLToPath` 에서 죽는다(실측). Node 22.12+ 는 top-level await 없는 ESM 의 require 를 지원하고
//   이 리포는 Node 24 다. 기준점이 `import.meta.url` 이 아니라 cwd 인 것도 같은 이유(vitest root).
const nodeRequire = createRequire(join(ROOT, 'package.json'))
const { stripJsonComments, readDevConfig } = nodeRequire(
  join(ROOT, 'scripts', 'dev-config-json.mjs'),
) as {
  stripJsonComments: (text: string) => string
  readDevConfig: (
    configPath?: string,
    releaseConfigPath?: string,
  ) => { json: string; devId: string; releaseId: string }
}

// 실제 릴리즈 설정에서 읽는다 — identifier 문자열을 테스트에 박으면 정본이 셋이 된다.
const RELEASE_CONFIG = join(ROOT, 'src-tauri', 'tauri.conf.json')
const DEV_CONFIG = join(ROOT, 'src-tauri', 'tauri.dev.conf.json')

function withTempConfig(body: string, run: (path: string) => void): void {
  const dir = mkdtempSync(join(tmpdir(), 'engram-devcfg-'))
  try {
    const path = join(dir, 'overlay.json')
    writeFileSync(path, body, 'utf8')
    run(path)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
}

describe('stripJsonComments', () => {
  it('keeps `//` that lives inside a JSON string value', () => {
    // `"$schema"` 는 실제 오버레이에 있는 살아 있는 사례다 — 줄 단위로 `//` 뒤를 자르는 스트리퍼는
    //   여기서 문자열을 반토막 낸다.
    const src = '{\n  "$schema": "https://schema.tauri.app/config/2"\n}'
    expect(JSON.parse(stripJsonComments(src))).toEqual({
      $schema: 'https://schema.tauri.app/config/2',
    })
  })

  it('drops a comment whose body itself contains `//`', () => {
    const src = ['{', '  // 실측: https://example.test/a//b · `windows.rs:67`', '  "a": 1', '}'].join(
      '\n',
    )
    expect(JSON.parse(stripJsonComments(src))).toEqual({ a: 1 })
  })

  it('does not let a stripped comment glue the next line onto the previous one', () => {
    const src = '{\n  "a": 1, // note\n  "b": 2\n}'
    expect(JSON.parse(stripJsonComments(src))).toEqual({ a: 1, b: 2 })
  })

  it('keeps an escaped quote from ending the string early', () => {
    const src = '{ "a": "x\\"// still in string", "b": 1 }'
    expect(JSON.parse(stripJsonComments(src))).toEqual({ a: 'x"// still in string', b: 1 })
  })

  it('strips block comments', () => {
    const src = '{ /* https://example.test */ "a": 1 }'
    expect(JSON.parse(stripJsonComments(src))).toEqual({ a: 1 })
  })

  it('rejects an unterminated block comment instead of swallowing the rest of the file', () => {
    // 삼키면 `{}` 로 깨끗이 파싱돼 내용 없는 오버레이가 유효한 값 행세를 한다.
    expect(() => stripJsonComments('{}/*')).toThrow()
  })
})

describe('readDevConfig', () => {
  it('yields a dev identifier that differs from the release one', () => {
    const { devId, releaseId, json } = readDevConfig(DEV_CONFIG, RELEASE_CONFIG)
    expect(devId).not.toBe('')
    expect(devId).not.toBe(releaseId)
    expect(JSON.parse(json).identifier).toBe(devId)
  })

  it('rejects an overlay with no identifier', () => {
    withTempConfig('{ "productName": "x" }', (path) => {
      expect(() => readDevConfig(path, RELEASE_CONFIG)).toThrow()
    })
  })

  it('rejects an empty identifier', () => {
    withTempConfig('{ "identifier": "" }', (path) => {
      expect(() => readDevConfig(path, RELEASE_CONFIG)).toThrow()
    })
  })

  it('rejects an identifier equal to the release one', () => {
    // 이 케이스가 불변식 그 자체다 — 같으면 두 빌드가 같은 single-instance 뮤텍스를 잡는다.
    const releaseId = JSON.parse(stripJsonComments(readFileSync(RELEASE_CONFIG, 'utf8'))).identifier
    withTempConfig(JSON.stringify({ identifier: releaseId }), (path) => {
      expect(() => readDevConfig(path, RELEASE_CONFIG)).toThrow()
    })
  })

  it('rejects a malformed overlay instead of returning a partial value', () => {
    // 후행 쉼표는 일부러 지원하지 않는다 — 헬퍼 주석의 그 결정이 실제로 시끄럽게 실패하는지 본다.
    withTempConfig('{\n  // c\n  "identifier": "com.example.x",\n}\n', (path) => {
      expect(() => readDevConfig(path, RELEASE_CONFIG)).toThrow()
    })
  })
})
