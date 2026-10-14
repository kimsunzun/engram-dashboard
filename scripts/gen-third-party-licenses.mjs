#!/usr/bin/env node
// scripts/gen-third-party-licenses.mjs — 릴리즈 zip 에 실리는 서드파티 고지(THIRD_PARTY_LICENSES.txt)를 만든다.
//
// ADR-0294: 고지는 커밋하지 않고 릴리즈 빌드 때 만들며, 같은 생성을 push CI 에서도 돌린다 · 잘못되면 실패한다(fail-closed).
// 부르는 쪽 둘: `scripts/build-release.ps1`(릴리즈 빌드마다 — 긴 빌드보다 먼저) · CI gates 잡(push 마다 — 결과는 버린다).
//   push 에서도 도는 이유: 허용 밖 라이선스 · 본문 없는 패키지 같은 실패가 태그를 찍은 뒤 릴리즈 빌드에서 처음 터지지 않게.
// 사용: `node scripts/gen-third-party-licenses.mjs <출력 파일 경로>` (cwd 무관)
//   전제: 아래 CARGO_ABOUT_VERSION 의 cargo-about · `npm ci` 를 마친 node_modules(generate-license-file 도 devDependency 로
//   거기 깔린다 — 버전과 그 전이 의존을 package-lock.json 이 묶는다).
//
// 세 부분을 한 파일로 잇는다.
//   Part 1 Rust = cargo-about(설정 licenses/about.toml). JSON 으로 받아 본문을 보정한 뒤 여기서 글로 찍는다.
//   Part 2 npm  = generate-license-file(설정 licenses/glf.jsonc). 운영 의존만 고르고 NOTICE 도 스스로 싣는다.
//   Part 3      = 패키지 메타데이터로는 안 보이는 동봉물(licenses/extra/*.txt — 손으로 관리).
// cargo-about 의 handlebars 템플릿을 쓰지 않는 이유: 기본이 HTML 이스케이프라 라이선스 본문의 따옴표가 `&quot;` 로
//   깨지고, 아래 보정 · 판정에 어차피 JSON 이 필요하다.
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, isAbsolute, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawnSync } from 'node:child_process'

const ROOT = fileURLToPath(new URL('..', import.meta.url))
const CONFIG_DIR = join(ROOT, 'scripts', 'licenses')
const GLF_DIR = join(ROOT, 'node_modules', 'generate-license-file')

// 올리면 출력 형식부터 다시 본다 — npm 쪽은 도구가 찍은 글을 아래에서 쪼개 읽는다. CI 가 까는 버전(ci.yml 의
//   PIN_CARGO_ABOUT_VERSION)과 어긋나면 여기서 요란하게 실패한다.
const CARGO_ABOUT_VERSION = '0.9.2'
// ★`--features cli` 를 빼지 말 것★: 0.9 부터 실행 파일이 그 기능 뒤에 있어, 빼면 설치는 성공한 채 실행 파일만 안 생긴다(실측).
//   GitHub 릴리스의 미리 빌드된 x86_64-pc-windows-msvc 바이너리를 PATH 에 둬도 된다(CI 가 그렇게 한다).
const CARGO_ABOUT_INSTALL = `cargo install cargo-about --locked --features cli --version ${CARGO_ABOUT_VERSION}`

function fail(message) {
  process.stderr.write(`gen-third-party-licenses: ${message}\n`)
  process.exit(1)
}

function note(message) {
  process.stderr.write(`gen-third-party-licenses: ${message}\n`)
}

if (process.argv.length !== 3) fail('사용: node scripts/gen-third-party-licenses.mjs <출력 파일 경로>')
const OUTPUT = resolve(process.argv[2])

const work = mkdtempSync(join(tmpdir(), 'engram-licenses-'))
process.on('exit', () => rmSync(work, { recursive: true, force: true }))

// 라이선스 본문을 한 모양으로 — 줄바꿈은 LF(쓸 때 CRLF 로 바꾼다) · BOM 과 줄 끝 공백 제거.
function normalize(text) {
  return text
    .replace(/^﻿/, '')
    .replace(/\r\n?/g, '\n')
    .replace(/[ \t]+$/gm, '')
    .trim()
}

// about.toml 의 accepted — Rust 의 OR 대안 고르기와 npm 허용 판정이 같은 목록을 쓴다(목록은 그 파일 한 곳에만 둔다).
function readAccepted() {
  const toml = readFileSync(join(CONFIG_DIR, 'about.toml'), 'utf8')
  const block = toml.match(/^accepted\s*=\s*\[([^\]]*)\]/m)
  if (!block) fail('scripts/licenses/about.toml 에서 accepted 목록을 읽지 못했습니다')
  return [...block[1].matchAll(/"([^"]+)"/g)].map((m) => m[1])
}
const ACCEPTED = readAccepted()

// SPDX 식을 잎 판정 `leaf(id, exception)` 으로 계산한다 — OR 는 하나라도, AND 는 전부. 못 읽는 식은 null(부르는 쪽이
//   실패로 다룬다).
function evalSpdx(expr, leaf) {
  const tokens = expr.replace(/[()]/g, (p) => ` ${p} `).trim().split(/\s+/)
  let i = 0
  const primary = () => {
    if (tokens[i] === '(') {
      i++
      const v = orExpr()
      if (tokens[i++] !== ')') throw new Error('괄호')
      return v
    }
    const id = tokens[i++]
    if (id === undefined || ['AND', 'OR', 'WITH', ')'].includes(id)) throw new Error('식별자 자리')
    let exception = null
    if (tokens[i] === 'WITH') {
      exception = tokens[i + 1]
      i += 2
    }
    return leaf(id, exception)
  }
  const andExpr = () => {
    let v = primary()
    while (tokens[i] === 'AND') {
      i++
      v = primary() && v
    }
    return v
  }
  const orExpr = () => {
    let v = andExpr()
    while (tokens[i] === 'OR') {
      i++
      v = andExpr() || v
    }
    return v
  }
  try {
    const v = orExpr()
    return i === tokens.length ? v : null
  } catch {
    return null
  }
}
// 허용 목록으로 만족되는가 — `X WITH Y` 는 목록에 그 꼴 그대로 있어야 한다.
const satisfies = (expr) => evalSpdx(expr, (id, exception) => ACCEPTED.includes(exception ? `${id} WITH ${exception}` : id))
// 실을 본문(그 안에서 전문으로 판정된 라이선스 ids)이 선언한 식을 덮는가 — `X WITH Y` 는 바탕 라이선스 X 로 본다.
const coveredBy = (expr, ids) => evalSpdx(expr, (id) => ids.includes(id))

// 표준 본문 자리표시(`<year>` · `[yyyy]` …)가 남은 글 — 표준 본문을 대신 실은 항목 밖에서 나오면 실패다.
//   Apache-2.0 부록의 `Copyright [yyyy] [name of copyright owner]`(옛 판은 `{yyyy} {name of copyright owner}`)는
//   라이선스 전문의 일부라 빼고 본다.
const PLACEHOLDER =
  /<\s*(year|yyyy|copyright holders?|owner|organization|name of author)\s*>|\[\s*(year|yyyy|fullname|name of copyright owner|copyright holders?)\s*\]|\{\s*(year|yyyy|name of copyright owner)\s*\}/i
function placeholderIn(text) {
  const m = text.replace(/Copyright [[{]yyyy[\]}] [[{]name of copyright owner[\]}]/g, '').match(PLACEHOLDER)
  return m ? m[0] : null
}

const RULE = '='.repeat(80)
const THIN_RULE = '-'.repeat(80)
const PART_RULE = '#'.repeat(80)

// ── Part 1. Rust: cargo-about ────────────────────────────────────────────────────────────────

const aboutVersion = spawnSync('cargo', ['about', '--version'], { encoding: 'utf8' })
if (aboutVersion.error || aboutVersion.status !== 0) {
  fail(`cargo-about 이 없습니다 — 설치: ${CARGO_ABOUT_INSTALL}`)
}
if (aboutVersion.stdout.trim() !== `cargo-about ${CARGO_ABOUT_VERSION}`) {
  fail(`cargo-about ${CARGO_ABOUT_VERSION} 이 필요합니다(설치된 것: ${aboutVersion.stdout.trim()}) — 설치: ${CARGO_ABOUT_INSTALL}`)
}

const rustJson = join(work, 'rust.json')
note('cargo about generate ...')
const about = spawnSync(
  'cargo',
  [
    'about', 'generate', '--format', 'json', '--workspace', '--locked',
    '-c', join(CONFIG_DIR, 'about.toml'),
    '-m', join(ROOT, 'Cargo.toml'),
    // -o 필수: 표준출력으로 받으면 cargo-about 이 부모가 PowerShell 인지 보고 거부한다(인코딩 문제 회피).
    '-o', rustJson,
  ],
  { cwd: ROOT, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 },
)
if (about.error) fail(`cargo about 을 실행하지 못했습니다 - ${about.error.message}`)
if (about.status !== 0) fail(`cargo about 실패(exit ${about.status})\n${about.stderr}`)
// 우리 crate 는 license 필드가 없어 매번 경고가 난다 — 그것만 걸러 내고 나머지 경고는 보인다. 판정은 아래 JSON 으로 한다.
const aboutNoise = about.stderr.split('\n').filter((l) => l.trim() !== '' && !l.includes("'engram-dashboard"))
if (aboutNoise.length > 0) process.stderr.write(aboutNoise.join('\n') + '\n')

const rust = JSON.parse(readFileSync(rustJson, 'utf8'))

// 우리 워크스페이스 crate 를 고지에서 빼는 것은 cargo-about 의 private 무시가 아니다(그건 `publish = false` 를 봐야
//   서는데 우리 매니페스트엔 없다). license 필드가 없는 crate 는 결과에서 빠지고 경고만 남는다 — 우리 crate 는 그 덕에
//   빠지지만, 외부 crate 가 같은 꼴이면 고지에서 **조용히** 빠진다. 그래서 그 둘을 여기서 가른다.
// source 가 없음 = 경로 의존 = 이 저장소의 crate(Cargo.lock 의 외부 출처는 crates.io 하나뿐 — 실측 2026-10-15).
const unknownExternal = rust.crates.filter((c) => c.license === 'Unknown' && c.package.source != null)
if (unknownExternal.length > 0) {
  fail(
    `라이선스를 못 정한 외부 crate 가 있습니다 — about.toml 에 clarify 를 더하세요:\n` +
      unknownExternal.map((c) => `  ${c.package.name} ${c.package.version}`).join('\n'),
  )
}
const local = rust.licenses.flatMap((l) => l.used_by).filter((u) => u.crate.source == null)
if (local.length > 0) {
  fail(
    `이 저장소의 crate 가 고지에 들어왔습니다(license 필드를 달았다면 우리 crate 를 따로 빼야 합니다):\n` +
      local.map((u) => `  ${u.crate.name} ${u.crate.version}`).join('\n'),
  )
}

// ── 본문 보정 ──
// cargo-about 은 라이선스 파일을 이름 글롭으로 찾는데 `LICENSE_MIT` · `license-mit` · `COPYING` · 내용으로만 가를 수 있는
//   `LICENSE` 를 놓치면 그 crate 에 표준 본문(저작권 줄 없음)을 대신 싣고 디버그 로그만 남긴다(`--fail` 도 못 잡는다 —
//   0.9.2 src/generate.rs). 그런 항목(source_path 없음)마다 crate 루트를 직접 읽어 진짜 본문을 찾는다.
//   ① 고른 라이선스의 전문이 든 파일 — 한 라이선스만 든 파일을 먼저(여러 개가 든 파일은 그다음)
//   ② 없으면 OR 식의 다른 허용 라이선스 중 전문 파일이 있는 것으로 바꾼다(about.toml 순서) — brotli-decompressor 처럼
//      고른 쪽(MIT) 파일은 없고 다른 쪽(BSD-3-Clause) 파일만 있는 crate
//   ③ 패키지에 없으면 licenses/rust/<crate>.txt 교체 파일(상류 저장소에서 옮긴 본문 · `// crate: <이름> <버전>` 으로 버전을
//      박는다 — 버전이 어긋나면 실패. 새 버전의 본문이 바뀌었을 수 있다)
//   ④ 그래도 없으면 표준 본문을 그대로 두고 "synthesized" 로 표시한다 — licenses/synthesized-crates.txt 에 없는 crate 면 실패.
const LICENSE_FILE = /^(licen[cs]e|copying)([-_.].*)?$/i
// 파일이 어느 라이선스의 **전문**인가 — 이름이 아니라 내용으로 가른다(`LICENSE` · `COPYING` 은 이름이 아무것도 말하지
//   않는다). 전문에만 있는 문장을 본다 — "Apache License 2.0 을 따른다" 같은 선언문은 걸리지 않는다.
const MARKERS = {
  'MIT': { all: [/permission is hereby granted, free of charge/, /shall be included in all copies or substantial portions/] },
  // 제목 줄("Apache License" · "Version 2.0" · "TERMS AND CONDITIONS …")만으로는 서지 않게 본문 조항(저작권 · 특허 허락 ·
  //   재배포 조건)의 문장까지 요구한다 — 다른 라이선스 본문에 Apache 제목만 붙인 글이 통과하지 않게.
  'Apache-2.0': { all: [/apache license,? version 2\.0/, /terms and conditions for use, reproduction,? and distribution/, /grant of copyright license\. subject to the terms and conditions of this license/, /grant of patent license\. subject to the terms and conditions of this license/, /you may reproduce and distribute copies of the work or derivative works thereof/] },
  'BSD-3-Clause': { all: [/redistribution and use in source and binary forms/, /neither the name/] },
  'BSD-2-Clause': { all: [/redistribution and use in source and binary forms/], none: [/neither the name/] },
  'ISC': { all: [/permission to use, copy, modify, and\/or distribute this software for any purpose/, /provided that the above copyright notice and this permission notice appear in all copies/] },
  'Zlib': { all: [/this software is provided 'as-is'/, /altered source versions must be plainly marked/] },
  'CC0-1.0': { all: [/cc0 1\.0 universal/] },
  'MPL-2.0': { all: [/mozilla public license,? version 2\.0/] },
  'Unicode-3.0': { all: [/unicode license v3/] },
  'BSL-1.0': { all: [/boost software license - version 1\.0/] },
  // 아래 셋은 위 형제와 문장이 겹쳐 「없어야 하는 문장」으로 가른다(MIT-0 = 저작권 표시 조건 없는 MIT · 0BSD = 같은 조건 없는 ISC).
  'MIT-0': { all: [/permission is hereby granted, free of charge/], none: [/shall be included in all copies or substantial portions/] },
  '0BSD': { all: [/permission to use, copy, modify, and\/or distribute this software for any purpose with or without fee is hereby granted/], none: [/provided that the above copyright notice and this permission notice appear in all copies/] },
  'Unlicense': { all: [/this is free and unencumbered software released into the public domain/] },
}
function classify(text) {
  const flat = text.toLowerCase().replace(/\s+/g, ' ')
  return Object.entries(MARKERS)
    .filter(([, m]) => m.all.every((re) => re.test(flat)) && !(m.none ?? []).some((re) => re.test(flat)))
    .map(([id]) => id)
}
function readLicenseFiles(dir) {
  return readdirSync(dir, { withFileTypes: true })
    // `.spdx` = SPDX 메타데이터 문서(tauri 계열) · THIRD-PARTY = 그 crate 가 품은 남의 코드 고지(아래 NOTICE 절이 싣는다).
    .filter((e) => e.isFile() && LICENSE_FILE.test(e.name) && !/\.spdx$/i.test(e.name) && !/third.?party/i.test(e.name))
    .map((e) => {
      const text = normalize(readFileSync(join(dir, e.name), 'utf8'))
      return { name: e.name, text, ids: classify(text) }
    })
    .sort((a, b) => a.name.localeCompare(b.name))
}
function pickFile(files, id) {
  return files.filter((f) => f.ids.includes(id)).sort((a, b) => a.ids.length - b.ids.length)[0]
}
function orAlternatives(expr) {
  if (!expr || /[()]|\bAND\b|\bWITH\b/.test(expr)) return []
  return expr.split(/\s+OR\s+/).map((s) => s.trim())
}
function readKnownList(file) {
  return new Set(
    readFileSync(join(CONFIG_DIR, file), 'utf8')
      .split('\n')
      .map((l) => l.replace(/#.*/, '').trim())
      .filter((l) => l !== ''),
  )
}
// 손으로 둔 본문 파일(rust/ · extra/)의 꼴 — 맨 앞 `//` 줄은 관리용 주석이라 떼고, 그중 `// crate: <이름> <버전>` 은
//   그 파일이 매인 crate 다.
function readHandFile(path) {
  const lines = readFileSync(path, 'utf8').replace(/\r\n?/g, '\n').split('\n')
  let i = 0
  const ties = []
  for (; i < lines.length && lines[i].startsWith('//'); i++) {
    const tie = lines[i].match(/^\/\/\s*crate:\s*(\S+)\s+(\S+)\s*$/)
    if (tie) ties.push(`${tie[1]} ${tie[2]}`)
  }
  return { ties, text: normalize(lines.slice(i).join('\n')) }
}

const licenseNames = new Map(rust.licenses.map((l) => [l.id, l.name]))
const knownSynthesized = readKnownList('synthesized-crates.txt')
const RUST_REPLACE_DIR = join(CONFIG_DIR, 'rust')
const rustReplacements = new Map(
  (existsSync(RUST_REPLACE_DIR) ? readdirSync(RUST_REPLACE_DIR).filter((f) => f.endsWith('.txt')) : [])
    .map((f) => [f.slice(0, -'.txt'.length), { file: f, ...readHandFile(join(RUST_REPLACE_DIR, f)), used: false }]),
)
const records = []
const fixes = { file: 0, switched: 0, replaced: 0 }
const badReplacements = []
const unexpectedSynthesized = []
const synthesizedSeen = new Set()
const rustCrates = new Map()
for (const lic of rust.licenses) {
  for (const u of lic.used_by) {
    const crate = `${u.crate.name} ${u.crate.version}`
    const dir = dirname(u.crate.manifest_path)
    rustCrates.set(crate, dir)
    const record = { crate, id: lic.id, text: normalize(lic.text), synthesized: false }
    if (lic.source_path == null) {
      const files = readLicenseFiles(dir)
      const own = pickFile(files, lic.id)
      const alt = own
        ? null
        : ACCEPTED.filter((id) => id !== lic.id && orAlternatives(u.crate.license).includes(id))
            .map((id) => ({ id, file: pickFile(files, id) }))
            .find((a) => a.file)
      const replacement = own || alt ? null : rustReplacements.get(u.crate.name)
      if (own) {
        record.text = own.text
        fixes.file++
      } else if (alt) {
        record.id = alt.id
        record.text = alt.file.text
        fixes.switched++
      } else if (replacement) {
        replacement.used = true
        if (replacement.ties.length !== 1 || replacement.ties[0] !== crate) {
          badReplacements.push(`  rust/${replacement.file}: 매인 버전 ${replacement.ties.join(', ') || '(없음)'} ≠ 그래프의 ${crate}`)
        } else if (!classify(replacement.text).includes(lic.id)) {
          badReplacements.push(`  rust/${replacement.file}: ${lic.id} 전문이 아니다`)
        }
        record.text = replacement.text
        fixes.replaced++
      } else {
        record.synthesized = true
        synthesizedSeen.add(u.crate.name)
        if (!knownSynthesized.has(u.crate.name)) unexpectedSynthesized.push(`  ${crate} (${lic.id})`)
        // 전문은 없어도 선언문(저작권 줄이 든 COPYING 등)이 있으면 함께 싣는다. 경로 한 줄짜리(깨진 링크)는 뺀다.
        for (const f of files) {
          if (f.text.length >= 40 && !/^\.{1,2}[\\/]/.test(f.text)) record.text += `\n\n--- ${f.name} of the crate ---\n\n${f.text}`
        }
      }
    }
    records.push(record)
  }
}
if (unexpectedSynthesized.length > 0) {
  fail(
    `라이선스 본문 파일이 없는 crate 가 새로 들어왔습니다 — 본문을 구할 곳부터 찾으세요(패키지에 있으면 about.toml 의 ` +
      `clarify · 상류 저장소에 있으면 scripts/licenses/rust/<crate>.txt). 정말 없을 때만 synthesized-crates.txt 에 ` +
      `더합니다(표준 본문을 싣는다):\n${unexpectedSynthesized.join('\n')}`,
  )
}
// 쓰이지 않은 교체 파일 = 그 crate 가 그래프에서 빠졌거나 버전이 바뀌어(새 버전이 파일을 싣게 됐을 수도) 매인 버전과
//   어긋난 것이다 — 둘 다 그 파일을 다시 볼 일이라 실패로 올린다.
for (const [name, r] of rustReplacements) {
  if (!r.used) badReplacements.push(`  rust/${r.file}: 쓰이지 않았다 — ${name} 이 그래프에 없거나 이제 패키지가 본문을 싣는다(매인 버전 ${r.ties.join(', ')})`)
}
if (badReplacements.length > 0) {
  fail(`scripts/licenses/rust/ 의 교체 파일이 그래프와 어긋납니다 — 상류에서 그 버전의 본문을 다시 받거나 지우세요:\n${badReplacements.join('\n')}`)
}
for (const name of knownSynthesized) {
  if (!synthesizedSeen.has(name)) note(`synthesized-crates.txt 의 ${name} 은 이제 표준 본문을 쓰지 않습니다 — 목록에서 빼도 됩니다`)
}
const rustPlaceholders = records
  .filter((r) => !r.synthesized && placeholderIn(r.text))
  .map((r) => `  ${r.crate} (${r.id}): ${placeholderIn(r.text)}`)

// 같은 라이선스 · 같은 본문끼리 묶는다(같은 MIT 라도 저작권 줄이 다르면 따로 — 저작권 줄이 고지 의무의 일부다).
const rustGroups = new Map()
for (const r of records) {
  if (!rustGroups.has(r.id)) rustGroups.set(r.id, { crates: new Set(), entries: new Map() })
  const group = rustGroups.get(r.id)
  group.crates.add(r.crate)
  const key = `${r.synthesized}\0${r.text}`
  if (!group.entries.has(key)) group.entries.set(key, { synthesized: r.synthesized, text: r.text, crates: [] })
  group.entries.get(key).crates.push(r.crate)
}

const rustOut = []
for (const [id, group] of [...rustGroups].sort((a, b) => b[1].crates.size - a[1].crates.size || a[0].localeCompare(b[0]))) {
  const name = licenseNames.get(id)
  rustOut.push(RULE, `${name && name !== id ? `${name} (${id})` : id} - ${group.crates.size} crate(s)`, RULE, '')
  const entries = [...group.entries.values()]
  for (const e of entries) e.crates.sort((a, b) => a.localeCompare(b))
  // 읽는 사람이 crate 이름으로 찾게 첫 crate 이름 순으로. 표준 본문을 대신 실은 묶음은 맨 뒤.
  entries.sort((a, b) => a.synthesized - b.synthesized || a.crates[0].localeCompare(b.crates[0]))
  entries.forEach((e, i) => {
    if (i > 0) rustOut.push(THIN_RULE, '')
    rustOut.push(e.synthesized ? 'Used by (synthesized - no license file in the published crate; standard text shown):' : 'Used by:')
    for (const c of e.crates) rustOut.push(`  - ${c}`)
    rustOut.push('', e.text, '')
  })
}

// 의존이 함께 배포하라고 둔 고지 파일 — Apache-2.0 §4(d) 의 NOTICE, 그리고 COPYRIGHT · LICENSE-THIRD-PARTY(그 crate 가
//   품은 남의 코드의 저작권 · 라이선스). 어느 도구 문서에도 이 처리가 없다(npm 쪽 도구는 NOTICE 만 스스로 싣는다).
//   고른 라이선스와 무관하게 crate 루트의 것을 다 싣는다 — 실어서 손해가 없다. 단 cargo-about 이 이미 라이선스 본문으로
//   집은 파일(source_path — atomic-waker 의 LICENSE-THIRD-PARTY 가 그렇다)은 Part 1 에 있으니 다시 싣지 않는다.
const pathKey = (p) => resolve(p).toLowerCase()
const aboutSources = new Set(
  rust.licenses
    .filter((l) => l.source_path != null)
    .flatMap((l) => l.used_by.map((u) => pathKey(isAbsolute(l.source_path) ? l.source_path : join(dirname(u.crate.manifest_path), l.source_path)))),
)
const rustNotices = []
for (const [crate, dir] of rustCrates) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (aboutSources.has(pathKey(join(dir, entry.name)))) continue
    if (entry.isFile() && /^(notice|copyright|license-third-party)($|[-_.])/i.test(entry.name)) {
      rustNotices.push({ crate, file: entry.name, text: normalize(readFileSync(join(dir, entry.name), 'utf8')) })
    }
  }
}
rustNotices.sort((a, b) => a.crate.localeCompare(b.crate) || a.file.localeCompare(b.file))
if (rustNotices.length > 0) {
  rustOut.push(RULE, 'NOTICE, COPYRIGHT and third-party license files of the Rust crates above', RULE, '')
  for (const n of rustNotices) rustOut.push(`--- ${n.crate}: ${n.file} ---`, '', n.text, '')
}
for (const n of rustNotices) {
  if (placeholderIn(n.text)) rustPlaceholders.push(`  ${n.crate} ${n.file}: ${placeholderIn(n.text)}`)
}
if (rustPlaceholders.length > 0) {
  fail(`표준 본문 자리표시가 남은 글이 있습니다(저작권 줄이 비었다):\n${rustPlaceholders.join('\n')}`)
}

// ── Part 2. npm: generate-license-file ───────────────────────────────────────────────────────

const glfBin = join(GLF_DIR, 'bin', 'generate-license-file')
if (!existsSync(glfBin)) fail('node_modules 에 generate-license-file 이 없습니다 — 먼저 `npm ci`')
const GLF_VERSION = JSON.parse(readFileSync(join(GLF_DIR, 'package.json'), 'utf8')).version

const npmTxt = join(work, 'npm.txt')
note(`generate-license-file ${GLF_VERSION} ...`)
const glf = spawnSync(
  process.execPath,
  [
    glfBin,
    '--input', join(ROOT, 'package.json'),
    '--output', npmTxt,
    '--config', join(CONFIG_DIR, 'glf.jsonc'),
    '--overwrite', '--eol', 'lf', '--ci', '--no-spinner',
  ],
  { cwd: ROOT, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 },
)
if (glf.error) fail(`generate-license-file 을 실행하지 못했습니다 - ${glf.error.message}`)
if (glf.status !== 0) fail(`generate-license-file 실패(exit ${glf.status})\n${glf.stderr}`)
if (glf.stderr.trim() !== '') process.stderr.write(glf.stderr)
// 라이선스 본문을 못 찾은 패키지는 경고만 남기고 출력에서 빠진다(종료 코드 0) — 조용한 누락이라 실패로 올린다.
if (glf.stderr.includes('Unable to determine license content')) {
  fail('generate-license-file 이 라이선스를 못 찾은 패키지가 있습니다(위 경고) — scripts/licenses/glf.jsonc 의 replace 로 대 주세요')
}

// 도구가 앞뒤에 붙이는 자기 홍보 두 줄은 뗀다 — 그 글의 "This file" 이 이 파일 전체를 가리키는 것처럼 읽힌다.
const GLF_CREDIT = 'This file was generated with the generate-license-file npm package!\nhttps://www.npmjs.com/package/generate-license-file\n'
let npmText = readFileSync(npmTxt, 'utf8')
if (!npmText.startsWith(GLF_CREDIT + '\n') || !npmText.endsWith(GLF_CREDIT)) {
  fail(`generate-license-file 출력 형식이 예상과 다릅니다 — ${GLF_VERSION} 의 형식이 바뀌었으면 아래 쪼개 읽기부터 다시 볼 것`)
}
npmText = npmText.slice(GLF_CREDIT.length + 1, -GLF_CREDIT.length)

// 도구가 라이선스 본문이 아닌 것을 본문으로 싣는 꼴을 막는다 — 라이선스 파일이 없으면 package.json 의 식별자 한 줄("MIT")
//   이나 license 필드의 URL, `LICENSE.spdx` 가 있으면 그 SPDX 메타데이터 문서. 전부 허락 문구가 빠진다.
//   묶음 형식 = 「패키지 목록 → "...the following license:" → 본문 → (선택) "With the following notices:" → 구분선」.
const npmPackages = new Set()
const badBodies = []
const npmPlaceholders = []
function checkNpmBlock(packages, licenseAndNotices) {
  const body = licenseAndNotices.split('\n\nWith the following notices:')[0].trim()
  if (/^[\w.+\-() ]+$/.test(body) || /^(https?:\/\/|www\.)\S+$/.test(body) || body.startsWith('SPDXVersion:')) {
    badBodies.push(`  ${packages.join(', ')} => "${body.split('\n')[0]}"`)
  }
  // 본문 · 고지 둘 다 본다. 한 묶음이 여러 패키지의 같은 글이라 패키지 목록째 보고한다.
  const hit = placeholderIn(licenseAndNotices)
  if (hit) npmPlaceholders.push(`  ${packages.join(', ')}: ${hit}`)
}
const blocks = npmText.split('\n-----------\n').filter((b) => b.trim() !== '')
for (const block of blocks) {
  const start = block.indexOf('the following license:\n\n')
  // 패키지 목록은 본문 앞에서만 읽는다 — 라이선스 본문에도 ` - ` 로 시작하는 줄(조항 목록)이 있다.
  const packages = start < 0 ? [] : [...block.slice(0, start).matchAll(/^ - (.+)$/gm)].map((m) => m[1])
  if (packages.length === 0) fail(`generate-license-file 출력의 한 묶음을 읽지 못했습니다:\n${block.slice(0, 300)}`)
  packages.forEach((p) => npmPackages.add(p))
  checkNpmBlock(packages, block.slice(start + 'the following license:\n\n'.length))
}

// glf.jsonc 의 replace — 키 대조(아래)와 peer 뒷받침이 함께 읽는다. 줄 전체 `//` 주석만 걷어 내고 JSON 으로 읽는다
//   (그 파일 머리말의 약속).
let glfReplace
try {
  const raw = readFileSync(join(CONFIG_DIR, 'glf.jsonc'), 'utf8').split(/\r?\n/).filter((l) => !/^\s*\/\//.test(l)).join('\n')
  glfReplace = JSON.parse(raw).replace ?? {}
} catch (err) {
  fail(`scripts/licenses/glf.jsonc 를 읽지 못했습니다(주석은 줄 전체 \`//\` 만 쓸 것) - ${err.message}`)
}

// 패키지마다 선언한 라이선스(package.json 의 license) — 교체 · peer 본문 대조와 아래 허용 판정이 함께 쓴다. node_modules 를
//   직접 훑어 이름@버전 → license 를 모은다(같은 패키지가 여러 버전으로 중첩돼 깔려도 버전으로 가른다).
const npmLicenses = new Map()
const walk = (dir) => {
  if (!existsSync(dir)) return
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    if (!e.isDirectory() || e.name.startsWith('.')) continue
    const p = join(dir, e.name)
    if (e.name.startsWith('@')) {
      for (const s of readdirSync(p, { withFileTypes: true })) if (s.isDirectory()) visit(join(p, s.name))
    } else {
      visit(p)
    }
  }
}
const visit = (dir) => {
  const manifest = join(dir, 'package.json')
  if (existsSync(manifest)) {
    const pkg = JSON.parse(readFileSync(manifest, 'utf8'))
    const lic = typeof pkg.license === 'string' ? pkg.license
      : pkg.license?.type ?? (Array.isArray(pkg.licenses) ? pkg.licenses.map((l) => l.type).join(' OR ') : null)
    if (pkg.name && pkg.version) npmLicenses.set(`${pkg.name}@${pkg.version}`, lic)
  }
  walk(join(dir, 'node_modules'))
}
walk(join(ROOT, 'node_modules'))

// 위 묶음은 도구가 실은 패키지뿐이다 — 도구가 빼먹은 것은 못 본다. generate-license-file 4.2.5 는 peer 로 표시된 패키지를
//   기록 · 순회 전에 건너뛰고(설치본 dist 의 resolveNpmDependencies), 본문을 못 찾은 패키지도 경고만 하고 뺀다. 그래서
//   운영 의존 집합을 package-lock.json 에서 따로 세어 대조한다 — `dev` 만 빼고(peer 는 넣는다), 이 플랫폼에 안 깔린
//   선택 의존(다른 OS 의 바이너리 패키지)은 디스크에 없으니 뺀다. 이름 = 항목의 `name`(별칭으로 깔린 패키지는 진짜 이름을
//   여기 적는다 — 도구도 진짜 이름으로 싣는다), 없으면 경로의 마지막 `node_modules/` 뒤(중첩 · scope 포함).
const lock = JSON.parse(readFileSync(join(ROOT, 'package-lock.json'), 'utf8'))
const lockPackages = new Map()
for (const [path, entry] of Object.entries(lock.packages ?? {})) {
  if (path === '' || entry.dev) continue
  if (!existsSync(join(ROOT, path, 'package.json'))) {
    if (entry.optional || entry.devOptional) continue
    fail(`package-lock.json 의 운영 의존 ${path} 가 깔려 있지 않습니다 — 먼저 \`npm ci\``)
  }
  const name = entry.name ?? path.slice(path.lastIndexOf('node_modules/') + 'node_modules/'.length)
  lockPackages.set(`${name}@${entry.version}`, { path, peer: !!entry.peer })
}

// replace 키는 정확한 이름@버전이다(glf.jsonc 머리말). 아래 「본문 아닌 것」 · 자리표시 검사보다 먼저 판정한다 — 키가
//   어긋나면 도구가 그 패키지에 본문을 못 대 줘 그 검사들도 터지는데, 고칠 곳은 키다(도구 실행 직후의 「라이선스를 못 찾은
//   패키지」 검사는 이보다도 앞, 위에서 이미 끝났다). 설치된 운영 의존에 그 이름@버전이 없으면 = 버전이 바뀌었거나
//   패키지가 빠져 그 본문이 아무 데도 쓰이지 않는다 — 새 버전의 본문을 다시 확인하라는 신호라 실패로 올린다.
const badReplaceKeys = Object.keys(glfReplace)
  .filter((k) => !/^@?[^@]+@[^@]+$/.test(k) || !lockPackages.has(k))
  .map((k) => `  ${k}${/^@?[^@]+@[^@]+$/.test(k) ? ' — 설치된 운영 의존에 이 버전이 없다' : ' — 키에 버전이 없다'}`)
if (badReplaceKeys.length > 0) {
  fail(`scripts/licenses/glf.jsonc 의 replace 키가 설치본과 어긋납니다 — 그 버전의 본문을 다시 확인하고 키를 고치거나 지우세요:\n${badReplaceKeys.join('\n')}`)
}

// 손으로 댄 본문(replace 파일 · 아래 peer 뒷받침)이 그 패키지가 선언한 라이선스를 덮는가 — 엉뚱한 파일을 건 실수(Apache-2.0
//   패키지에 MIT 본문만 등)를 막는다. 본문 안에서 전문으로 확인된 라이선스(Rust 쪽과 같은 내용 판정)로 선언 식을 계산한다.
// `ids` 는 파일마다 따로 판정한 것의 합집합으로 받는다 — 여러 파일을 이어 붙여 한 번에 판정하면 「없어야 하는 문장」 조건
//   때문에 MIT 파일이 MIT-0 을, BSD-3 이 BSD-2 를, ISC 가 0BSD 를 가려 `MIT-0 AND MIT` 같은 정당한 식이 떨어진다.
const uncovered = []
function checkCoverage(p, ids, source) {
  const declared = npmLicenses.get(p)
  if (!declared || coveredBy(declared, ids) !== true) {
    uncovered.push(`  ${p}: 선언 ${declared ?? '(없음)'} · 본문에서 확인된 전문 ${ids.join(', ') || '(없음)'} — ${source}`)
  }
}
for (const [k, file] of Object.entries(glfReplace)) {
  checkCoverage(k, classify(normalize(readFileSync(join(CONFIG_DIR, file), 'utf8'))), `glf.jsonc → ${file}`)
}

// peer 로만 닿는 운영 의존은 도구가 아예 보지 않아 glf.jsonc 의 replace 도 소용없다 — 여기서 직접 싣는다. 본문 =
//   glf.jsonc 에 그 이름@버전 replace 가 있으면 그 파일, 없으면 패키지 폴더의 라이선스 전문 파일 전부(파일 이름 · 내용
//   판정은 Rust 쪽과 같다) · 고지 = 같은 폴더의 notice 파일. 도구가 실은 묶음과 같은 꼴 · 같은 검사를 거친다.
const missingFromNotice = []
const peerFallback = []
for (const [p, { path, peer }] of [...lockPackages].sort((a, b) => a[0].localeCompare(b[0]))) {
  if (npmPackages.has(p)) continue
  if (!peer) {
    missingFromNotice.push(`  ${p}`)
    continue
  }
  const dir = join(ROOT, path)
  let license
  if (glfReplace[p]) {
    license = normalize(readFileSync(join(CONFIG_DIR, glfReplace[p]), 'utf8'))
  } else {
    const files = readLicenseFiles(dir).filter((f) => f.ids.length > 0)
    if (files.length === 0) {
      fail(`peer 로만 닿는 운영 의존 ${p} 에 라이선스 전문 파일이 없습니다 — scripts/licenses/npm/ 에 상류 본문을 두고 glf.jsonc 의 replace 에 "${p}" 키로 거세요(이 패키지는 도구가 아니라 생성 스크립트가 그 키를 읽어 싣는다)`)
    }
    license = files.map((f) => f.text).join('\n\n')
    checkCoverage(p, [...new Set(files.flatMap((f) => f.ids))], `${path} 의 라이선스 파일`)
  }
  const notices = readdirSync(dir, { withFileTypes: true })
    .filter((e) => e.isFile() && /^notice($|[-.])/i.test(e.name))
    .map((e) => normalize(readFileSync(join(dir, e.name), 'utf8')))
  const tail = license + (notices.length > 0 ? `\n\nWith the following notices:\n\n${notices.join('\n')}` : '')
  checkNpmBlock([p], tail)
  npmPackages.add(p)
  peerFallback.push(`The following npm package may be included in this product:\n\n - ${p}\n\nThis package contains the following license:\n\n${tail}\n\n-----------\n\n`)
}
if (uncovered.length > 0) {
  fail(`손으로 댄 본문이 패키지가 선언한 라이선스를 덮지 않습니다 — 그 라이선스의 전문 파일을 대 주세요:\n${uncovered.join('\n')}`)
}
if (missingFromNotice.length > 0) {
  fail(
    `package-lock.json 의 운영 의존 중 고지에 없는 패키지가 있습니다(도구가 건너뛰었다). node_modules 가 package-lock.json 과 ` +
      `어긋났거나(낡은 설치) 링크 · 정션으로 깔린 패키지일 수 있다 — 먼저 \`npm ci\` 로 다시 깔고 돌려 볼 것:\n${missingFromNotice.join('\n')}`,
  )
}
npmText += peerFallback.join('')
if (badBodies.length > 0) {
  fail(`라이선스 본문이 아닌 것이 실린 패키지가 있습니다 — scripts/licenses/glf.jsonc 의 replace 로 대 주세요:\n${badBodies.join('\n')}`)
}
if (npmPlaceholders.length > 0) {
  fail(`표준 본문 자리표시가 남은 npm 글이 있습니다(저작권 줄이 비었다):\n${npmPlaceholders.join('\n')}`)
}
for (const p of [...npmPackages].filter((p) => !lockPackages.has(p)).sort()) {
  note(`고지에는 있는데 package-lock.json 의 운영 의존에는 없는 패키지: ${p}`)
}

// npm 쪽 도구에는 허용 목록이 없다 — 실린 패키지마다 선언 라이선스(위 npmLicenses)를 about.toml 의 accepted 로 판정한다.
const npmRejected = []
for (const p of [...npmPackages].sort()) {
  const lic = npmLicenses.get(p)
  if (lic === undefined) npmRejected.push(`  ${p}: node_modules 에서 package.json 을 찾지 못했습니다`)
  else if (!lic) npmRejected.push(`  ${p}: license 필드가 없습니다`)
  else if (satisfies(lic) !== true) npmRejected.push(`  ${p}: ${lic}`)
}
if (npmRejected.length > 0) {
  fail(
    `허용 목록(about.toml accepted) 밖 라이선스의 npm 패키지가 있습니다 — GPL · LGPL · AGPL 은 더하지 말고 끌고 온 경로부터 ` +
      `보고할 것(\`npm ls --omit=dev <패키지>\`):\n${npmRejected.join('\n')}`,
  )
}

// ── Part 3. 손으로 관리하는 동봉물 고지(licenses/extra/*.txt) ───────────────────────────────────

const EXTRA_DIR = join(CONFIG_DIR, 'extra')
const extras = []
for (const file of existsSync(EXTRA_DIR) ? readdirSync(EXTRA_DIR).filter((f) => f.endsWith('.txt')).sort() : []) {
  const { ties, text } = readHandFile(join(EXTRA_DIR, file))
  for (const tie of ties) {
    if (!rustCrates.has(tie)) {
      fail(`licenses/extra/${file} 이 매인 crate(${tie})가 그 버전으로 배포 그래프에 없습니다 — 버전이 바뀌었으면 동봉물과 고지를 다시 확인하고 그 줄을 고칠 것`)
    }
  }
  if (placeholderIn(text)) fail(`licenses/extra/${file} 에 표준 본문 자리표시가 남았습니다: ${placeholderIn(text)}`)
  extras.push(text)
}

// ── 합치기 ───────────────────────────────────────────────────────────────────────────────────

const now = new Date()
const date = [now.getFullYear(), now.getMonth() + 1, now.getDate()].map((n) => String(n).padStart(2, '0')).join('-')

const out = [
  'THIRD-PARTY SOFTWARE NOTICES',
  '============================',
  '',
  'This software includes the following third-party components. Their license',
  'texts and notices are reproduced below.',
  '',
  `Generated on ${date} with cargo-about ${CARGO_ABOUT_VERSION} (Rust crates) and`,
  `generate-license-file ${GLF_VERSION} (npm packages); Part 3 is maintained by hand.`,
  'Entries marked "synthesized" show the standard license text because the',
  'published package contains no license file.',
  'Source code of each package is available from its registry:',
  'https://crates.io/crates/<name>/<version> and https://www.npmjs.com/package/<name>/v/<version>.',
  '',
  `  Part 1. Rust crates (${rustCrates.size})`,
  `  Part 2. npm packages (${npmPackages.size})`,
  `  Part 3. Other bundled components (${extras.length})`,
  '',
  '',
  PART_RULE, 'Part 1. Rust crates', PART_RULE, '',
  ...rustOut,
  '',
  PART_RULE, 'Part 2. npm packages', PART_RULE, '',
  normalize(npmText),
  '',
  '',
  PART_RULE, 'Part 3. Other bundled components', PART_RULE, '',
  extras.join(`\n\n${RULE}\n\n`),
  '',
].join('\n')

// CRLF 로 쓴다 — Windows 배포물이다(메모장 기준).
mkdirSync(dirname(OUTPUT), { recursive: true })
writeFileSync(OUTPUT, out.replace(/\n/g, '\r\n'))
const synthesizedCount = records.filter((r) => r.synthesized).length
note(
  `완료 — Rust crate ${rustCrates.size}(본문 보정 ${fixes.file} · 대안 라이선스로 ${fixes.switched} · 교체 파일 ${fixes.replaced} · 표준 본문 ${synthesizedCount}) · ` +
    `npm 패키지 ${npmPackages.size}(lock 대조 ${lockPackages.size} · peer 직접 실음 ${peerFallback.length}) · Rust 고지 파일 ${rustNotices.length} · 동봉물 ${extras.length} → ${OUTPUT}`,
)
