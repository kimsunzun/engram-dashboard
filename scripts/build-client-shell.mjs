#!/usr/bin/env node
// scripts/build-client-shell.mjs — the **only** place that builds the debug client shell (`engram-dashboard.exe`).
// It ties dev overlay injection and artifact verification into one body.
//
// ADR-0137 — dev/release identifier separation. There are only two injection paths and this file is one of them (the other =
//   `tauri dev --config` in `scripts/tauri-cli.mjs`, for the Tauri CLI path only).
// ★Do not go back to a raw `cargo build -p engram-dashboard`★: that command does not pass through the Tauri CLI so the overlay is not
//   applied, and the debug exe gets stamped with the **release identifier**. The symptom does not show in the build — only while a release app is
//   running does the dev app die at once with no window (single-instance mutex collision. Full mechanism =
//   the comment in `src-tauri/tauri.dev.conf.json`). It really regressed that way.
// ★There are three consumers, so it was pulled out into a script★: the two debug launcher `.bat` files and the GUI measurement procedure of `/engram-qa`
//   (`.claude/skill-bindings/qa.md` 「GUI 실측」). If the three each copy the injection, nobody knows when one drops it —
//   in fact the qa procedure was left without it. When a new consumer appears, do not copy the injection; call this script.
//   **Do not optimize the call away as "already built, skip it"** — the cost of one more link step each run is
//   a known cost (accepted in review), and skipping it removes the verification too.
// ★The post-build verification (do not delete)★: this is where we catch the injection breaking **at runtime**. If tauri
//   changes its `TAURI_CONFIG` support or renames it, the injection code stays the same and silently becomes void, and only this verification that looks at
//   the artifact itself catches that class. But it cannot see for itself the case where **nobody calls this script any more** — that axis (call-site
//   wiring) is covered by the source-text assertions in `src/util/launcherWiring.test.ts`. Both are needed to cover it.
//
// Output contract (the call sites depend on it):
//   stdout = on success **exactly one line**, the absolute path of the built exe. On failure it writes nothing.
//   stderr = human-readable progress and failure messages + cargo output.
//   ★The launchers decide failure by whether stdout is empty, and launch the app from that path★ — if the path is hard-coded separately,
//   where the build went and where the launch happens diverge (see the target directory measurement comment below).
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { join } from 'node:path'
import { spawnSync } from 'node:child_process'

import { readDevConfig } from './dev-config-json.mjs'

const ROOT = fileURLToPath(new URL('..', import.meta.url))

function fail(message) {
  process.stderr.write(`build-client-shell: ${message}\n`)
  process.exit(1)
}

function note(message) {
  process.stderr.write(`build-client-shell: ${message}\n`)
}

// Takes no arguments — the verification below assumes the debug profile path, so if an argument like `--release` came in,
// it would pass while looking at the wrong artifact.
if (process.argv.length > 2) fail(`인자를 받지 않습니다(받은 것: ${process.argv.slice(2).join(' ')})`)

// ★Do not hard-code the output directory (`target/debug` is only the default)★: `CARGO_TARGET_DIR` or
//   `build.target-dir` in `.cargo/config.toml` redirects the output elsewhere. If hard-coded, the build goes to the new path and the verification
//   reads **a stale exe at the old path** and passes — right now there really is an exe with the dev identifier at that spot,
//   so the verification would print "success" and the launcher would launch that stale binary. `scripts/build-release.ps1`
//   already solved the same trap this way (measured via cargo metadata).
function resolveTargetDir() {
  const meta = spawnSync(
    'cargo',
    ['metadata', '--format-version', '1', '--no-deps', '--manifest-path', join(ROOT, 'Cargo.toml')],
    { cwd: ROOT, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 },
  )
  if (meta.error) fail(`cargo metadata 를 실행하지 못했습니다 - ${meta.error.message}`)
  if (meta.status !== 0) fail(`cargo metadata 실패(exit ${meta.status})\n${meta.stderr ?? ''}`)
  let targetDirectory
  try {
    targetDirectory = JSON.parse(meta.stdout).target_directory
  } catch (err) {
    fail(`cargo metadata 출력을 파싱하지 못했습니다 - ${err.message}`)
  }
  if (typeof targetDirectory !== 'string' || targetDirectory === '') {
    fail('cargo metadata 에 target_directory 가 없습니다')
  }
  // If CARGO_BUILD_TARGET (a non-host triple) is set, the artifact goes down to `<target>/<triple>/debug`.
  //   The metadata target_directory does not include that segment, so it is inserted here.
  const triple = (process.env.CARGO_BUILD_TARGET ?? '').trim()
  if (triple !== '') note(`CARGO_BUILD_TARGET=${triple} 감지 — 산출 경로에 triple 세그먼트 반영`)
  return triple === '' ? join(targetDirectory, 'debug') : join(targetDirectory, triple, 'debug')
}

let config
try {
  config = readDevConfig()
} catch (err) {
  fail(`dev 오버레이를 쓸 수 없습니다 - ${err.message}`)
}

const exePath = join(
  resolveTargetDir(),
  `engram-dashboard${process.platform === 'win32' ? '.exe' : ''}`,
)

// Environment variables go to the child only — exporting them to this process or the calling shell would taint even the release build that runs afterwards
// (`scripts/tauri-cli.mjs` rejects that case, but not leaking in the first place comes first).
// cargo writes progress and errors to stderr, so it does not break the stdout contract (one path line).
const build = spawnSync('cargo', ['build', '-p', 'engram-dashboard'], {
  cwd: ROOT,
  env: { ...process.env, TAURI_CONFIG: config.json },
  stdio: 'inherit',
})
if (build.error) fail(`cargo 를 실행하지 못했습니다 - ${build.error.message}`)
if (build.status !== 0) process.exit(build.status ?? 1)

let image
try {
  // latin1 = bytes 1:1 — the identifier is ASCII so this is enough, and 22MB is not decoded as UTF-8.
  image = readFileSync(exePath).toString('latin1')
} catch (err) {
  fail(`빌드는 성공했는데 산출물을 읽지 못했습니다(${exePath}) - ${err.message}`)
}

if (!image.includes(config.devId)) {
  fail(
    `산출물에 dev identifier("${config.devId}")가 없습니다 — 오버레이 주입이 끊겼습니다.\n` +
      `  대상: ${exePath}\n` +
      `  TAURI_CONFIG 를 tauri 가 아직 읽는지 확인할 것(tauri-build/tauri-codegen 의 config 병합).\n` +
      `  이대로 띄우면 릴리즈 앱이 떠 있을 때 창 없이 즉시 죽습니다.`,
  )
}

// If the dev id starts with the release id (`com.engram.dashboard` + `.dev`), a plain substring check cannot tell
// the two apart — the trailing tail must be excluded with a negative lookahead so that only the "bare release id" is caught.
const escape = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
const bareRelease = config.devId.startsWith(config.releaseId)
  ? new RegExp(`${escape(config.releaseId)}(?!${escape(config.devId.slice(config.releaseId.length))})`)
  : new RegExp(escape(config.releaseId))
if (bareRelease.test(image)) {
  fail(
    `산출물에 release identifier("${config.releaseId}")가 그대로 남아 있습니다 — 오버레이가 부분만 먹었습니다.\n` +
      `  대상: ${exePath}`,
  )
}

note(`OK - identifier "${config.devId}" 확인 (${exePath})`)
process.stdout.write(`${exePath}\n`)
