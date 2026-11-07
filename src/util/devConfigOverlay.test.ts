// ADR-0137: regression net for the dev overlay helper (`scripts/dev-config-json.mjs`) — the value that makes the dev identifier get
//   stamped on the debug shell is made here. If this helper loosens, a value with only a sound structure (`{}`) passes and tauri
//   uses the release identifier as is, and when a release app is running the dev app dies with no window.
//
// ★Placement exception — this is the single home of that rationale (including the share of the sibling `launcherWiring.test.ts`. Do not duplicate
//   it there)★: an exception to the colocation convention (`vitest.config.ts` comment: "tests sit next to the source *.test.ts"). Both files
//   have their targets in `scripts/`·`.claude/`, so there is no source in `src/` to put them next to. But the vitest include is
//   only `src/**/*.test.ts(x)`, so if placed next to the target `npm test` would not pick them up at all, and widening include is outside the scope of this
//   change. So we chose not "the place that breaks the convention" but **the only place that gets picked up** —
//   it means the colocation convention cannot accommodate these two tests, and there is no sidecar source.
// What this file looks at is only the helper's **value decision**. The call-site wiring (whether the launcher · gate really build by that path) is seen by
//   `launcherWiring.test.ts`, and whether the injection became invalid at runtime is seen by the comparison of post-build artifacts in `scripts/build-client-shell.mjs`.
//   The three are separate axes, so merging them into one leaves the other two empty.
import { describe, expect, it } from 'vitest'

// Only this file uses Node standard modules. The frontend tsconfig is DOM-only (no @types/node), so only the needed symbols are
//   declared ambient **at this file's scope** (the same way as `src/store/chatStyleStore.test.ts` — to avoid creating a dependency on global
//   @types/node). Why we do not use whole-file suppression (`@ts-nocheck`) = that would also turn off the type checking of
//   the assertions below.
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

// ★The helper is brought in with the plain Node loader (do not revert to `import`/`await import`)★: those two pass through vite and
//   transform the helper as well, and inside a transformed module `import.meta.url` is `http:` so the helper dies in
//   `fileURLToPath` while loading (measured). Node 22.12+ supports require of ESM without top-level await, and
//   this repo is on Node 24. The reference point being cwd rather than `import.meta.url` is for the same reason (vitest root).
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

// Read from the actual release config — hard-coding the identifier string in the test makes three canonical sources.
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
    // `"$schema"` is a live case in the actual overlay — a stripper that cuts after `//` line by line
    //   splits a string in half here.
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
    // If swallowed, it parses cleanly as `{}` and an overlay with no content passes itself off as a valid value.
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
    // This case is the invariant itself — if equal, the two builds grab the same single-instance mutex.
    const releaseId = JSON.parse(stripJsonComments(readFileSync(RELEASE_CONFIG, 'utf8'))).identifier
    withTempConfig(JSON.stringify({ identifier: releaseId }), (path) => {
      expect(() => readDevConfig(path, RELEASE_CONFIG)).toThrow()
    })
  })

  it('rejects a malformed overlay instead of returning a partial value', () => {
    // Trailing commas are deliberately not supported — check that the decision in the helper comment really fails loudly.
    withTempConfig('{\n  // c\n  "identifier": "com.example.x",\n}\n', (path) => {
      expect(() => readDevConfig(path, RELEASE_CONFIG)).toThrow()
    })
  })
})
