#!/usr/bin/env node
// scripts/tauri-cli.mjs — package.json 의 `tauri` 스크립트 실체. Tauri CLI 를 그대로 대행하되
// **dev 서브커맨드에만** dev 오버레이 설정(`src-tauri/tauri.dev.conf.json`)을 끼워 넣는다.
//
// ★왜 래퍼가 필요한가(`"tauri": "tauri"` 로 되돌리지 마라)★: dev 빌드는 릴리즈와 **다른 번들
//   identifier** 로 떠야 한다 — 같으면 single-instance 뮤텍스가 겹쳐 릴리즈 앱이 떠 있는 동안
//   dev 앱이 즉시 죽는다(사유 전문 = 오버레이 파일 주석). identifier 를 가르는 공식 수단은
//   `--config <오버레이>` 뿐인데, 그 플래그는 **서브커맨드 소속**이라(`tauri dev -c ...`;
//   `tauri -c ... dev` 는 없다) npm 스크립트 문자열에 미리 박을 수가 없다 — npm 은 사용자 인자를
//   항상 **뒤에** 붙이므로 `npm run tauri dev` 는 `<스크립트> dev` 가 된다. 그래서 인자를 받아
//   위치를 보고 끼우는 이 한 겹이 필요하다. 개발자가 플래그를 외우지 않아도 되게 하는 것이 목적.
// ★build 는 건드리지 않는다★ — 배포 정체성(`com.engram.dashboard`)은 그대로여야 한다.
//   CI 의 build-release.ps1 도 `npm run tauri -- build --no-bundle` 로 이 파일을 지나간다.
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const cli = require('@tauri-apps/cli')

const DEV_CONFIG = 'src-tauri/tauri.dev.conf.json'

const args = process.argv.slice(2)

// 첫 비-플래그 토큰 = 서브커맨드. 호출자가 이미 --config 를 줬으면 그 의도가 이긴다.
const subIndex = args.findIndex((a) => !a.startsWith('-'))
const hasConfig = args.some((a) => a === '-c' || a === '--config' || a.startsWith('--config='))
if (subIndex !== -1 && args[subIndex] === 'dev' && !hasConfig) {
  // 서브커맨드 **직후**에 넣는다 — 맨 뒤는 `--` 뒤(러너/앱 인자 구간)로 넘어갈 수 있다.
  args.splice(subIndex + 1, 0, '--config', DEV_CONFIG)
}

cli.run(args, 'npm run tauri').catch((err) => {
  cli.logError(err.message)
  process.exit(1)
})
