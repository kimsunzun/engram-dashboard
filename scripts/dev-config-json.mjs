#!/usr/bin/env node
// scripts/dev-config-json.mjs — prints the dev overlay (`src-tauri/tauri.dev.conf.json`) as one-line JSON.
// Consumer = `scripts/build-client-shell.mjs` (it puts that value into `TAURI_CONFIG` and feeds it to `cargo build -p
// engram-dashboard`). Run it directly from the CLI only when a person wants to see the value by hand.
//
// ★Do not pass it just because it is valid JSON (do not loosen this back)★: the only failure
//   signal the consumer can see is "is stdout empty", so if a value with a sound structure but no identifier (`{}`) leaked through, tauri would use the
//   release identifier as is and the consumer would read it as success — the very accident this helper tries to prevent
//   just moves up one layer. So here we check **the invariant itself**: an identifier exists, and it
//   differs from the release one (`src-tauri/tauri.conf.json`). No literal is hard-coded here, so both files
//   stay canonical.
//
// ADR-0137 — the overlay file is the single source of the dev identifier. This helper exists so that call sites do not duplicate that string
//   (if duplicated, only one side gets fixed and they silently drift apart).
// ★The value must be inline JSON, not a file path★: the receiving side parses it with `serde_json::from_str` and then merges it into
//   the config (measured — tauri-build 2.6.3 `src/lib.rs:487`, tauri-codegen 2.6.3
//   `src/lib.rs:83`). Both declare `cargo:rerun-if-env-changed=TAURI_CONFIG`, so a rebuild happens when the value appears, changes or
//   disappears.
// ★On failure it writes nothing to stdout and exits (do not revive it)★: the call site decides failure by whether stdout is empty
//   and aborts the build. If a default were printed here instead, it would silently return to the release identifier,
//   and that symptom shows neither in the build nor at runtime — only while a release app is running does the dev app die at once with no window.
import { readFileSync } from 'node:fs'
import { fileURLToPath, pathToFileURL } from 'node:url'

// Resolved relative to this file, not the cwd — it does not assume the caller's cwd.
const DEV_CONFIG_PATH = fileURLToPath(new URL('../src-tauri/tauri.dev.conf.json', import.meta.url))
const RELEASE_CONFIG_PATH = fileURLToPath(new URL('../src-tauri/tauri.conf.json', import.meta.url))

// The input is JSONC — it has `//` comments, and the comment bodies themselves contain `//` (URLs, paths like `windows.rs:67`).
// On top of that, `"$schema"` is a real JSON string value that holds `//`. So a "cut after `//`" style stripper
// breaks the file — it must track inside/outside of strings and escapes.
// Trailing commas are deliberately not handled — instead of imitating JSONC further, we let JSON.parse fail loudly.
export function stripJsonComments(text) {
  let out = ''
  let i = 0
  let inString = false
  while (i < text.length) {
    const c = text[i]
    if (inString) {
      if (c === '\\') {
        out += c + (text[i + 1] ?? '')
        i += 2
        continue
      }
      if (c === '"') inString = false
      out += c
      i += 1
      continue
    }
    if (c === '"') {
      inString = true
      out += c
      i += 1
      continue
    }
    if (c === '/' && text[i + 1] === '/') {
      // Line breaks are kept — so the following line does not stick to the previous one.
      while (i < text.length && text[i] !== '\n') i += 1
      continue
    }
    if (c === '/' && text[i + 1] === '*') {
      i += 2
      while (i < text.length && !(text[i] === '*' && text[i + 1] === '/')) i += 1
      // An unclosed block comment must not be silently swallowed as "a comment to the end" — a truncated file like `{}/*`
      // would parse cleanly as `{}` and an overlay with no content would pose as a valid value.
      if (i >= text.length) throw new Error('닫히지 않은 블록 주석(`/*`)')
      i += 2
      continue
    }
    out += c
    i += 1
  }
  return out
}

function readIdentifier(configPath) {
  const parsed = JSON.parse(stripJsonComments(readFileSync(configPath, 'utf8')))
  const id = parsed.identifier
  if (typeof id !== 'string' || id === '') {
    throw new Error(`${configPath} 에 쓸 수 있는 "identifier" 가 없습니다`)
  }
  return { parsed, id }
}

// The reason it parses too = to kill a broken overlay here. If passed through as is, cargo would die inside the build script,
// and that error looks unrelated to the identifier.
// Contract: on success `{ json, devId, releaseId }`. json = the one-line inline JSON to put into `TAURI_CONFIG`, and
//   the two ids are used by the side that verifies the build artifact (`build-client-shell.mjs`). Every failure is a throw and
//   is never signaled through a return value — the call site must not be able to see a partial success.
// The two path arguments are for injecting controls (tests) — the defaults are the two real config files.
export function readDevConfig(configPath = DEV_CONFIG_PATH, releaseConfigPath = RELEASE_CONFIG_PATH) {
  const { parsed, id: devId } = readIdentifier(configPath)
  const { id: releaseId } = readIdentifier(releaseConfigPath)
  if (devId === releaseId) {
    throw new Error(
      `dev identifier 가 release 와 같습니다("${devId}") — 두 빌드가 같은 single-instance 뮤텍스를 잡습니다`,
    )
  }
  return { json: JSON.stringify(parsed), devId, releaseId }
}

export function readDevConfigJson(configPath = DEV_CONFIG_PATH, releaseConfigPath = RELEASE_CONFIG_PATH) {
  return readDevConfig(configPath, releaseConfigPath).json
}

const invokedAsMain =
  process.argv[1] !== undefined && pathToFileURL(process.argv[1]).href === import.meta.url
if (invokedAsMain) {
  try {
    process.stdout.write(readDevConfigJson() + '\n')
  } catch (err) {
    // Folds read failure and invariant violation into one message — to the call site it arrives as "there is no value" either way.
    process.stderr.write(`dev-config-json: ${DEV_CONFIG_PATH} 를 쓸 수 없습니다 - ${err.message}\n`)
    process.exit(1)
  }
}
