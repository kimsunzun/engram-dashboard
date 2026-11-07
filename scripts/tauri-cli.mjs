#!/usr/bin/env node
// scripts/tauri-cli.mjs — the real body of the `tauri` script in package.json. It stands in for the Tauri CLI as is, but
// inserts the dev overlay config (`src-tauri/tauri.dev.conf.json`) **only for the dev subcommand**.
//
// ADR-0137 — this one layer that inserts the overlay only for the dev subcommand is **one of two** injection points of the identifier
//   separation. ★The other = `scripts/build-client-shell.mjs`★ — it covers a raw
//   `cargo build -p engram-dashboard`, which does not pass through the Tauri CLI, with `TAURI_CONFIG` (the two debug launchers and `/engram-qa` call that
//   script). If only one side is fixed, the other path silently returns to the release identifier — it really
//   happened that way.
// ★Why a wrapper is needed (do not go back to `"tauri": "tauri"`)★: a dev build must come up with a **different bundle
//   identifier** from release — if the same, the single-instance mutex collides and the dev app dies at once while the release app
//   is running (full reason = the overlay file comment). The official means to separate the identifier on the CLI path is
//   `--config <오버레이>`, but that flag **belongs to the subcommand** (`tauri dev -c ...`;
//   there is no `tauri -c ... dev`) so it cannot be hard-coded into the npm script string — npm always appends user arguments
//   **at the end**, so `npm run tauri dev` becomes `<스크립트> dev`. So this one layer that takes the arguments and
//   inserts by position is needed. The purpose is that developers need not memorize the flag.
// ★Subcommands other than dev are not touched★ — the distribution identity (`com.engram.dashboard`) must stay as is.
//   The CI's build-release.ps1 also goes through this file with `npm run tauri -- build --no-bundle`.
//   This wall blocks **both channels**: inserting the overlay only into `dev` (`injected` below), and rejecting an inherited `TAURI_CONFIG`
//   from a non-`dev` subcommand (the guard below). The latter covers not only `build` but everything that consumes the identifier
//   such as `bundle`·`android build`·`ios build`.
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const cli = require('@tauri-apps/cli')

const DEV_CONFIG = 'src-tauri/tauri.dev.conf.json'

const args = process.argv.slice(2)

// ★The `--` boundary★: what comes after it is the section the caller hands to the execution target (runner · app) so we must not interpret it — if we mistake the
//   `--config` caught there for ours we skip the dev overlay and quietly go back to the release identifier (symptom =
//   if a release app is running the dev app exits at once with no window). So both the subcommand and --config decisions look
//   only at the section before `--`.
const sepIndex = args.indexOf('--')
const preArgs = sepIndex === -1 ? args : args.slice(0, sepIndex)

// The first non-flag token = the subcommand. If the caller already gave --config, that intent wins.
const subIndex = preArgs.findIndex((a) => !a.startsWith('-'))
const hasConfig = preArgs.some((a) => a === '-c' || a === '--config' || a.startsWith('--config='))
// ★Every subcommand that is **not** `dev` rejects an inherited `TAURI_CONFIG` (do not extend the allow list)★:
//   the "do not touch build" wall above looks only at arguments so it was **blind to the environment variable channel.** If an artifact is built in a shell
//   carrying the dev value, tauri merges that value into the config and the dev identifier is stamped — signing,
//   updater and the WebView2 data folder all follow that value, so a wrong distribution quietly goes out.
//   ★Do not write the forbidden target as the single token `build`★: `bundle` (this repo actually ran it — the 2026-08-16 measured comment
//   in the overlay file) · `android build` · `ios build` all consume the identifier too,
//   and if written as a name list, whatever is not on the list leaks through and that fact is not visible. Instead we write the narrow and **complete**
//   invariant this wrapper can actually state — the only subcommand that needs an inherited `TAURI_CONFIG` is `dev`.
//   The reason to **reject** rather than erase = erasing here would hide "why was my overlay ignored".
if (subIndex !== -1 && preArgs[subIndex] !== 'dev' && (process.env.TAURI_CONFIG ?? '') !== '') {
  process.stderr.write(
    `tauri-cli: "${preArgs[subIndex]}" 는 TAURI_CONFIG 가 설정된 환경에서 돌 수 없습니다 — 산출물 identifier 가 오염됩니다.\n` +
      '  이 값은 debug 셸 빌드 전용입니다(scripts/build-client-shell.mjs 가 자식 프로세스에만 넘깁니다).\n' +
      '  변수를 지운 셸에서 다시 실행하세요.\n',
  )
  process.exit(1)
}

const injected = subIndex !== -1 && preArgs[subIndex] === 'dev' && !hasConfig
if (injected) {
  // Inserted **right after** the subcommand — the very end could end up past `--` (the runner/app argument section).
  args.splice(subIndex + 1, 0, '--config', DEV_CONFIG)
} else if (preArgs.includes('dev') && !hasConfig) {
  // If a global option that takes a value appears, a subIndex misjudgment (`tauri --opt val dev` → val mistaken for the subcommand)
  // really blows up. Today no such option exists so it is latent — instead of restructuring we leave only a conspicuous warning.
  process.stderr.write('tauri-cli: "dev" 토큰이 보이지만 --config 오버레이를 못 끼웠습니다(서브커맨드 판정 확인 요망)\n')
}

cli.run(args, 'npm run tauri').catch((err) => {
  cli.logError(err.message)
  process.exit(1)
})
