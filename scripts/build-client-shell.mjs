#!/usr/bin/env node
// scripts/build-client-shell.mjs — debug 클라이언트 셸(`engram-dashboard.exe`)을 빌드하는 **유일한**
// 자리. dev 오버레이 주입과 산출물 대조를 한 몸으로 묶는다.
//
// ADR-0137 — dev/release identifier 분리. 주입 경로는 둘뿐이고 이 파일이 그중 하나다(다른 하나 =
//   `scripts/tauri-cli.mjs` 의 `tauri dev --config`, Tauri CLI 경로 전용).
// ★생 `cargo build -p engram-dashboard` 로 되돌리지 마라★: 그 명령은 Tauri CLI 를 지나지 않아 오버레이가
//   적용되지 않고, debug exe 가 **release identifier** 로 찍힌다. 증상이 빌드에 안 나온다 — 릴리즈 앱이
//   떠 있을 때만 dev 앱이 창도 없이 즉시 죽는다(single-instance 뮤텍스 충돌. 기전 전문 =
//   `src-tauri/tauri.dev.conf.json` 주석). 실제로 그렇게 회귀했다.
// ★소비자가 셋이라 스크립트로 뽑았다★: debug 런처 `.bat` 둘과 `/qa` 게이트의 GUI 실측 절차
//   (`.claude/skill-bindings/qa.md` §full). 셋이 각자 주입을 베끼면 하나가 빠져도 아무도 모른다 —
//   실제로 qa 절차가 빠진 채였다. 새 소비자가 생기면 주입을 베끼지 말고 이 스크립트를 부른다.
//   **호출을 "이미 빌드했으니 생략" 으로 최적화하지 말 것** — 매번 도는 대가로 링크가 한 번 더 도는 건
//   알려진 비용이고(리뷰에서 수용), 건너뛰면 대조도 같이 사라진다.
// ★빌드 뒤 대조(지우지 마라)★: 주입이 **런타임에** 끊겼는지 잡는 자리다. tauri 가 `TAURI_CONFIG` 지원을
//   바꾸거나 이름을 바꾸면 주입 코드는 그대로인 채 조용히 무효가 되는데, 산출물을 직접 보는 이 대조만
//   그 부류를 잡는다. 다만 **이 스크립트를 아무도 안 부르게 된 경우**는 스스로 못 본다 — 그 축(호출부
//   배선)은 `src/util/launcherWiring.test.ts` 의 소스 텍스트 단언이 맡는다. 둘 다 있어야 덮인다.
//
// 출력 계약(호출부가 의존한다):
//   stdout = 성공 시 **정확히 한 줄**, 빌드된 exe 의 절대 경로. 실패하면 아무것도 쓰지 않는다.
//   stderr = 사람이 읽는 진행·실패 메시지 + cargo 출력.
//   ★런처는 stdout 이 비었는지로 실패를 판정하고, 그 경로로 앱을 띄운다★ — 경로를 따로 하드코딩하면
//   빌드가 간 곳과 띄우는 곳이 갈린다(아래 target 디렉토리 실측 주석).
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

// 인자를 받지 않는다 — 아래 대조가 debug 프로파일 경로를 전제하므로, `--release` 같은 인자가 들어오면
// 엉뚱한 산출물을 보면서 통과한다.
if (process.argv.length > 2) fail(`인자를 받지 않습니다(받은 것: ${process.argv.slice(2).join(' ')})`)

// ★산출 디렉토리를 하드코딩하지 마라(`target/debug` 는 기본값일 뿐)★: `CARGO_TARGET_DIR` 이나
//   `.cargo/config.toml` 의 `build.target-dir` 이 출력을 딴 데로 돌린다. 하드코딩하면 빌드는 새 경로로
//   가고 대조는 **옛 경로의 낡은 exe** 를 읽어 통과한다 — 지금 그 자리엔 dev identifier 를 가진 exe 가
//   실제로 있어서, 대조가 "성공" 을 찍고 런처는 그 stale 바이너리를 띄운다. `scripts/build-release.ps1`
//   이 같은 함정을 이미 이렇게 풀었다(cargo metadata 실측).
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
  // CARGO_BUILD_TARGET(비-호스트 triple)이 걸리면 산출물이 `<target>/<triple>/debug` 로 내려간다.
  //   metadata 의 target_directory 는 그 세그먼트를 포함하지 않으므로 여기서 끼워 넣는다.
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

// 환경변수는 자식에게만 준다 — 이 프로세스나 호출한 셸에 export 하면 뒤이어 도는 릴리즈 빌드까지
// 물들인다(`scripts/tauri-cli.mjs` 가 그 경우를 거부하지만, 애초에 새지 않게 하는 쪽이 먼저다).
// cargo 는 진행·에러를 stderr 로 쓰므로 stdout 계약(경로 한 줄)을 깨지 않는다.
const build = spawnSync('cargo', ['build', '-p', 'engram-dashboard'], {
  cwd: ROOT,
  env: { ...process.env, TAURI_CONFIG: config.json },
  stdio: 'inherit',
})
if (build.error) fail(`cargo 를 실행하지 못했습니다 - ${build.error.message}`)
if (build.status !== 0) process.exit(build.status ?? 1)

let image
try {
  // latin1 = 바이트 1:1 — identifier 는 ASCII 라 이걸로 충분하고, 22MB 를 UTF-8 로 디코딩하지 않는다.
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

// dev id 가 release id 로 시작하면(`com.engram.dashboard` + `.dev`) 단순 부분문자열 검사로는 둘을 못
// 가른다 — 뒤따르는 꼬리를 부정 전방탐색으로 배제해야 "맨 release id" 만 잡힌다.
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
