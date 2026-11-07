// ADR-0137: call-site wiring gate — checks by source text that **every documented path** that builds debug calls `scripts/build-client-shell.mjs`
//   and that it has not gone back to a raw `cargo build -p engram-dashboard`.
//
// ★Why this axis is needed separately (do not delete)★: the build-artifact comparison (inside that script) runs **only when someone calls the
//   script** — if the call itself disappears no signal is left. It actually regressed that way (473e82f:
//   the launcher's `npm run tauri dev` was changed to a raw `cargo build`, dropping the overlay injection entirely, and all gates
//   stayed green). This assertion is the only form that could have caught that commit.
// The decision method is the same as the other isolation gates in this repo — source text comparison (core `use tauri` · messaging · net ·
//   ADR-0130). The claim "unit tests cannot see wiring" is not true.
// ★Comment lines are not counted★: the files below write "do not go back to a raw cargo build" in comments, so
//   looking only for whether the string exists would catch that warning itself as a violation. Only executed lines are looked at.
// The placement rationale (an exception to the colocation convention) is written once in the `devConfigOverlay.test.ts` header — it is not duplicated
//   here.
import { describe, expect, it } from 'vitest'

// Only this file uses Node standard modules — a file-scope ambient declaration (the `src/store/chatStyleStore.test.ts` way).
declare function require(id: string): {
  readFileSync(path: string, enc: string): string
  join(...parts: string[]): string
}
declare const process: { cwd(): string }

const { readFileSync } = require('node:fs')
const { join } = require('node:path')

const ROOT = process.cwd()

const BUILD_SCRIPT = 'build-client-shell.mjs'
// Catches only the form that builds the client shell directly — sibling crates such as `-p engram-dashboard-daemon`·`-agent` are
//   legitimate, so it matches only when no character follows the name.
const BARE_CLIENT_BUILD = /cargo\s+build\b[^\n]*-p\s+engram-dashboard(?![-\w])/

function read(relPath: string): string {
  return readFileSync(join(ROOT, relPath), 'utf8')
}

// The executed lines of a `.bat` = lines that are not REM/:: comments.
function batExecutableLines(text: string): string {
  return text
    .split(/\r?\n/)
    .filter((line) => !/^\s*(@?rem\b|::)/i.test(line))
    .join('\n')
}

// The executed lines of Markdown = lines inside code fences with the trailing `#` comment removed. Prose paragraphs hold warnings, so
//   they are not targets.
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
    // The prose-side warning ("do not build with a raw cargo build") is outside the fence so it does not come in here.
    expect(fencedExecutableLines(read(GATE))).not.toMatch(BARE_CLIENT_BUILD)
  })
})
