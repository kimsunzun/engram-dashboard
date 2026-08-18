#!/usr/bin/env node
// scripts/tauri-cli.mjs — package.json 의 `tauri` 스크립트 실체. Tauri CLI 를 그대로 대행하되
// **dev 서브커맨드에만** dev 오버레이 설정(`src-tauri/tauri.dev.conf.json`)을 끼워 넣는다.
//
// ADR-0137 — dev 서브커맨드에만 오버레이를 끼우는 이 한 겹은 identifier 분리의 주입 지점 **둘 중
//   하나**다. ★다른 하나 = `scripts/build-client-shell.mjs`★ — 그쪽은 Tauri CLI 를 지나지 않는 생
//   `cargo build -p engram-dashboard` 를 `TAURI_CONFIG` 로 덮는다(debug 런처 둘과 `/qa` 게이트가 그
//   스크립트를 부른다). 한쪽만 고치면 다른 경로가 조용히 release identifier 로 돌아간다 — 실제로
//   그렇게 났다.
// ★왜 래퍼가 필요한가(`"tauri": "tauri"` 로 되돌리지 마라)★: dev 빌드는 릴리즈와 **다른 번들
//   identifier** 로 떠야 한다 — 같으면 single-instance 뮤텍스가 겹쳐 릴리즈 앱이 떠 있는 동안
//   dev 앱이 즉시 죽는다(사유 전문 = 오버레이 파일 주석). CLI 경로에서 identifier 를 가르는 공식
//   수단은 `--config <오버레이>` 인데, 그 플래그는 **서브커맨드 소속**이라(`tauri dev -c ...`;
//   `tauri -c ... dev` 는 없다) npm 스크립트 문자열에 미리 박을 수가 없다 — npm 은 사용자 인자를
//   항상 **뒤에** 붙이므로 `npm run tauri dev` 는 `<스크립트> dev` 가 된다. 그래서 인자를 받아
//   위치를 보고 끼우는 이 한 겹이 필요하다. 개발자가 플래그를 외우지 않아도 되게 하는 것이 목적.
// ★dev 가 아닌 서브커맨드는 건드리지 않는다★ — 배포 정체성(`com.engram.dashboard`)은 그대로여야 한다.
//   CI 의 build-release.ps1 도 `npm run tauri -- build --no-bundle` 로 이 파일을 지나간다.
//   이 벽은 **두 채널 다** 막는다: 오버레이를 `dev` 에만 끼우는 것(아래 `injected`)과, `dev` 가 아닌
//   서브커맨드에서 물려받은 `TAURI_CONFIG` 를 거부하는 것(아래 가드). 후자는 `build` 만이 아니라
//   `bundle`·`android build`·`ios build` 처럼 identifier 를 소비하는 것 전부를 덮는다.
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const cli = require('@tauri-apps/cli')

const DEV_CONFIG = 'src-tauri/tauri.dev.conf.json'

const args = process.argv.slice(2)

// ★`--` 경계★: 그 뒤는 호출자가 실행 대상(러너·앱)에 넘기는 구간이라 우리가 해석하면 안 된다 — 거기 낀
//   `--config` 를 우리 걸로 오인하면 dev 오버레이를 건너뛰어 release 식별자로 조용히 되돌아간다(증상 =
//   release 앱이 떠 있으면 dev 앱이 창도 없이 즉시 종료). 그래서 서브커맨드·--config 판정 둘 다 `--`
//   앞 구간만 본다.
const sepIndex = args.indexOf('--')
const preArgs = sepIndex === -1 ? args : args.slice(0, sepIndex)

// 첫 비-플래그 토큰 = 서브커맨드. 호출자가 이미 --config 를 줬으면 그 의도가 이긴다.
const subIndex = preArgs.findIndex((a) => !a.startsWith('-'))
const hasConfig = preArgs.some((a) => a === '-c' || a === '--config' || a.startsWith('--config='))
// ★`dev` 가 **아닌** 모든 서브커맨드는 물려받은 `TAURI_CONFIG` 를 거부한다(허용 목록을 늘리지 마라)★:
//   위 "build 는 건드리지 않는다" 벽은 인자만 보므로 **환경변수 채널에 대해 눈이 멀어 있었다.** dev 값이
//   실린 셸에서 산출물을 만들면 tauri 가 그 값을 config 에 merge 해 dev identifier 가 찍힌다 — 서명·
//   업데이터·WebView2 데이터 폴더가 전부 그 값을 따라가므로 조용히 잘못된 배포판이 나간다.
//   ★금지 대상을 `build` 라는 토큰 하나로 적지 않는다★: `bundle`(이 리포가 실제로 돌렸다 — 오버레이
//   파일의 2026-08-16 실측 주석) · `android build` · `ios build` 도 전부 identifier 를 소비하는데,
//   이름 목록으로 적으면 목록에 없는 것이 그대로 새고 그 사실이 안 보인다. 대신 이 래퍼가 실제로 말할 수
//   있는 좁고 **완전한** 불변식을 쓴다 — 물려받은 `TAURI_CONFIG` 가 필요한 서브커맨드는 `dev` 뿐이다.
//   지우지 않고 **거부**하는 이유 = 이 자리에서 지우면 "왜 내 오버레이가 무시됐나"가 안 보인다.
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
  // 서브커맨드 **직후**에 넣는다 — 맨 뒤는 `--` 뒤(러너/앱 인자 구간)로 넘어갈 수 있다.
  args.splice(subIndex + 1, 0, '--config', DEV_CONFIG)
} else if (preArgs.includes('dev') && !hasConfig) {
  // 값을 받는 전역 옵션이 생기면 subIndex 오판(`tauri --opt val dev` → val 을 서브커맨드로 오인)이
  // 실제로 터진다. 지금은 그런 옵션이 없어 latent — 재구조화 대신 눈에 띄는 경고만 남긴다.
  process.stderr.write('tauri-cli: "dev" 토큰이 보이지만 --config 오버레이를 못 끼웠습니다(서브커맨드 판정 확인 요망)\n')
}

cli.run(args, 'npm run tauri').catch((err) => {
  cli.logError(err.message)
  process.exit(1)
})
