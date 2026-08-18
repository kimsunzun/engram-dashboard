#!/usr/bin/env node
// scripts/dev-config-json.mjs — dev 오버레이(`src-tauri/tauri.dev.conf.json`)를 한 줄 JSON 으로 찍는다.
// 소비자 = `scripts/build-client-shell.mjs`(그 값을 `TAURI_CONFIG` 로 담아 `cargo build -p
// engram-dashboard` 에 흘린다). 사람이 손으로 값을 볼 때만 CLI 로 직접 실행한다.
//
// ★유효한 JSON 이라는 것만으로 통과시키지 않는다(느슨하게 되돌리지 마라)★: 소비자가 볼 수 있는 실패
//   신호는 "stdout 이 비었나" 하나뿐이라, 구조만 멀쩡하고 identifier 가 없는 값(`{}`)을 흘리면 tauri 는
//   release identifier 를 그대로 쓰고 소비자는 성공으로 읽는다 — 이 헬퍼가 막으려는 바로 그 사고가
//   한 층 위로 옮겨갈 뿐이다. 그래서 여기서 **불변식 자체**를 본다: identifier 가 있고, 그것이
//   release(`src-tauri/tauri.conf.json`)와 다르다. 리터럴을 여기 박지 않으므로 양쪽 파일이 정본으로
//   남는다.
//
// ADR-0137 — 오버레이 파일이 dev identifier 의 단일 출처다. 호출부가 그 문자열을 복제하지 않게 하려고
//   이 헬퍼를 둔다(복제하면 한쪽만 고쳐져 조용히 어긋난다).
// ★값은 파일 경로가 아니라 인라인 JSON 이어야 한다★: 받는 쪽이 `serde_json::from_str` 로 파싱한 뒤
//   config 에 merge 한다(실측 — tauri-build 2.6.3 `src/lib.rs:487`, tauri-codegen 2.6.3
//   `src/lib.rs:83`). 둘 다 `cargo:rerun-if-env-changed=TAURI_CONFIG` 를 선언해 값이 생기거나 바뀌거나
//   사라지면 재빌드된다.
// ★실패하면 stdout 에 아무것도 쓰지 않고 종료한다(되살리지 마라)★: 호출부는 stdout 이 비었는지로
//   실패를 판정해 빌드를 중단한다. 여기서 기본값을 대신 찍으면 release identifier 로 조용히 돌아가고,
//   그 증상은 빌드에도 런타임에도 안 보인다 — 릴리즈 앱이 떠 있을 때만 dev 앱이 창 없이 즉시 죽는다.
import { readFileSync } from 'node:fs'
import { fileURLToPath, pathToFileURL } from 'node:url'

// cwd 가 아니라 이 파일 기준으로 잡는다 — 호출부의 cwd 를 전제하지 않는다.
const DEV_CONFIG_PATH = fileURLToPath(new URL('../src-tauri/tauri.dev.conf.json', import.meta.url))
const RELEASE_CONFIG_PATH = fileURLToPath(new URL('../src-tauri/tauri.conf.json', import.meta.url))

// 입력은 JSONC 다 — `//` 주석이 있고, 그 주석 본문에도 `//` 가 들어 있다(URL·`windows.rs:67` 같은 경로).
// 게다가 `"$schema"` 는 `//` 를 품은 진짜 JSON 문자열 값이다. 그래서 "`//` 뒤를 자른다" 식 스트리퍼는
// 파일을 깨뜨린다 — 문자열 안/밖과 이스케이프를 추적해야 한다.
// 후행 쉼표는 일부러 처리하지 않는다 — JSONC 를 더 흉내 내는 대신 JSON.parse 가 시끄럽게 실패하게 둔다.
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
      // 줄바꿈은 남긴다 — 뒤따르는 줄이 앞줄에 붙지 않게.
      while (i < text.length && text[i] !== '\n') i += 1
      continue
    }
    if (c === '/' && text[i + 1] === '*') {
      i += 2
      while (i < text.length && !(text[i] === '*' && text[i + 1] === '/')) i += 1
      // 닫히지 않은 블록 주석은 조용히 "끝까지 주석"으로 삼키면 안 된다 — `{}/*` 같은 잘린 파일이
      // `{}` 로 깨끗이 파싱돼 내용 없는 오버레이가 유효한 값 행세를 한다.
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

// 파싱까지 하는 이유 = 깨진 오버레이를 여기서 죽이기 위해서다. 그대로 흘리면 cargo 가 build script 안에서
// 죽고, 그 에러는 identifier 와 무관해 보인다.
// 계약: 성공하면 `{ json, devId, releaseId }`. json = `TAURI_CONFIG` 에 넣을 인라인 JSON 한 줄이고,
//   두 id 는 빌드 산출물을 대조하는 쪽(`build-client-shell.mjs`)이 쓴다. 실패는 전부 throw 이며
//   반환값으로 신호하지 않는다 — 호출부가 부분 성공을 볼 수 없어야 한다.
// 두 경로 인자는 대조군 주입용(테스트) — 기본값이 실제 설정 파일 둘이다.
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
    // 읽기 실패와 불변식 위반을 한 메시지로 묶는다 — 호출부엔 어차피 "값이 없다" 하나로 도착한다.
    process.stderr.write(`dev-config-json: ${DEV_CONFIG_PATH} 를 쓸 수 없습니다 - ${err.message}\n`)
    process.exit(1)
  }
}
