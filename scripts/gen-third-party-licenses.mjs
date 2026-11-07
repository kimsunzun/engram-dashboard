#!/usr/bin/env node
// scripts/gen-third-party-licenses.mjs — builds the third-party notice (THIRD_PARTY_LICENSES.txt) shipped in the release zip.
//
// ADR-0294: the notice is not committed but built at release build time, and the same generation also runs in push CI · it fails if wrong (fail-closed).
// Two callers: `scripts/build-release.ps1` (every release build — before the long build) · the CI gates job (every push — the result is discarded).
//   Why it runs on push too: so that failures such as a license outside the allowed set or a package with no license text do not first blow up in the release build after the tag is cut.
// Usage: `node scripts/gen-third-party-licenses.mjs <출력 파일 경로>` (cwd-independent)
//   Prerequisites: cargo-about at the CARGO_ABOUT_VERSION below · node_modules after `npm ci` (generate-license-file is also installed
//   there as a devDependency — package-lock.json pins its version and its transitive dependencies).
//
// Joins three parts into one file.
//   Part 1 Rust = cargo-about (config licenses/about.toml). Receives JSON, corrects the texts, and prints them as text here.
//   Part 2 npm  = generate-license-file (config licenses/glf.jsonc). Picks only production dependencies and includes NOTICE by itself.
//   Part 3      = bundled items not visible from package metadata (licenses/extra/*.txt — managed by hand).
// Why the handlebars template of cargo-about is not used: the default is HTML escaping, so quotes in the license text
//   break into `&quot;`, and the corrections and decisions below need JSON anyway.
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, isAbsolute, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawnSync } from 'node:child_process'

const ROOT = fileURLToPath(new URL('..', import.meta.url))
const CONFIG_DIR = join(ROOT, 'scripts', 'licenses')
const GLF_DIR = join(ROOT, 'node_modules', 'generate-license-file')

// If raised, look at the output format again first — on the npm side the text the tool printed is split and read below. If it diverges from the version CI installs (PIN_CARGO_ABOUT_VERSION in ci.yml),
//   it fails loudly here.
const CARGO_ABOUT_VERSION = '0.9.2'
// ★Do not drop `--features cli`★: since 0.9 the executable is behind that feature, so if dropped the install succeeds while only the executable is not produced (measured).
//   You may also put GitHub release's prebuilt x86_64-pc-windows-msvc binary on PATH (CI does that).
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

// Puts the license text in one shape — line breaks are LF (changed to CRLF when written) · BOM and trailing spaces removed.
function normalize(text) {
  return text
    .replace(/^﻿/, '')
    .replace(/\r\n?/g, '\n')
    .replace(/[ \t]+$/gm, '')
    .trim()
}

// The accepted of about.toml — the Rust OR-alternative selection and the npm allow decision use the same list (the list lives in that one file only).
function readAccepted() {
  const toml = readFileSync(join(CONFIG_DIR, 'about.toml'), 'utf8')
  const block = toml.match(/^accepted\s*=\s*\[([^\]]*)\]/m)
  if (!block) fail('scripts/licenses/about.toml 에서 accepted 목록을 읽지 못했습니다')
  return [...block[1].matchAll(/"([^"]+)"/g)].map((m) => m[1])
}
const ACCEPTED = readAccepted()

// Evaluates an SPDX expression with the leaf decision `leaf(id, exception)` — OR needs any one, AND needs all. An expression it cannot read is null (the caller
//   treats it as a failure).
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
// Whether it is satisfied by the allow list — `X WITH Y` must be in the list in exactly that form.
const satisfies = (expr) => evalSpdx(expr, (id, exception) => ACCEPTED.includes(exception ? `${id} WITH ${exception}` : id))
// Whether the text to ship (the license ids decided as full text within it) covers the declared expression — `X WITH Y` is treated as the base license X.
const coveredBy = (expr, ids) => evalSpdx(expr, (id) => ids.includes(id))

// Text where a standard-text placeholder (`<year>` · `[yyyy]` ...) remains — if it appears outside an item that shipped the standard text instead, it fails.
//   The Apache-2.0 appendix `Copyright [yyyy] [name of copyright owner]` (the old edition has `{yyyy} {name of copyright owner}`) is
//   part of the license full text, so this check ignores it.
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
    // -o is required: if output goes to stdout, cargo-about checks whether the parent is PowerShell and refuses (works around an encoding problem).
    '-o', rustJson,
  ],
  { cwd: ROOT, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 },
)
if (about.error) fail(`cargo about 을 실행하지 못했습니다 - ${about.error.message}`)
if (about.status !== 0) fail(`cargo about 실패(exit ${about.status})\n${about.stderr}`)
// Our crates have no license field so a warning appears every time — only that one is filtered out and the other warnings stay visible. The decision is made from the JSON below.
const aboutNoise = about.stderr.split('\n').filter((l) => l.trim() !== '' && !l.includes("'engram-dashboard"))
if (aboutNoise.length > 0) process.stderr.write(aboutNoise.join('\n') + '\n')

const rust = JSON.parse(readFileSync(rustJson, 'utf8'))

// Dropping our workspace crates from the notice is not cargo-about's private ignore (that works only by looking at `publish = false`
//   and our manifests do not have it). A crate with no license field drops out of the result and only a warning remains — our crates drop out thanks to that,
//   but if an external crate had the same shape it would drop out of the notice **silently**. So we tell those two apart here.
// No source = a path dependency = a crate of this repository (the only external origin in Cargo.lock is crates.io — measured 2026-10-15).
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

// ── Text correction ──
// cargo-about finds license files by name glob, and if it misses `LICENSE_MIT` · `license-mit` · `COPYING` · a `LICENSE` that can be told apart only by content,
//   it ships the standard text (with no copyright line) for that crate and leaves only a debug log (`--fail` does not catch it either —
//   0.9.2 src/generate.rs). For each such item (no source_path) it reads the crate root directly to find the real text.
//   ① a file that holds the full text of the chosen license — files holding a single license first (files holding several next)
//   ② if none, switch to another allowed license in the OR expression that has a full-text file (about.toml order) — a crate like brotli-decompressor
//      with no file for the chosen side (MIT) but only a file for the other side (BSD-3-Clause)
//   ③ if the package has none, the licenses/rust/<crate>.txt replacement file (text moved from the upstream repository · the version is pinned with `// crate: <이름> <버전>` —
//      it fails if the version diverges. The new version's text may have changed)
//   ④ if still none, leave the standard text as is and mark it "synthesized" — it fails if the crate is not in licenses/synthesized-crates.txt.
const LICENSE_FILE = /^(licen[cs]e|copying)([-_.].*)?$/i
// Which license's **full text** a file is — told apart by content, not by name (the name of `LICENSE` · `COPYING` says nothing).
//   We look at sentences found only in the full text — a declaration like "follows Apache License 2.0" does not match.
const MARKERS = {
  'MIT': { all: [/permission is hereby granted, free of charge/, /shall be included in all copies or substantial portions/] },
  // So that title lines ("Apache License" · "Version 2.0" · "TERMS AND CONDITIONS ...") alone do not suffice, it also requires sentences of the body clauses (copyright · patent grant ·
  //   redistribution conditions) — so that text of another license with just an Apache title stuck on does not pass.
  'Apache-2.0': { all: [/apache license,? version 2\.0/, /terms and conditions for use, reproduction,? and distribution/, /grant of copyright license\. subject to the terms and conditions of this license/, /grant of patent license\. subject to the terms and conditions of this license/, /you may reproduce and distribute copies of the work or derivative works thereof/] },
  'BSD-3-Clause': { all: [/redistribution and use in source and binary forms/, /neither the name/] },
  'BSD-2-Clause': { all: [/redistribution and use in source and binary forms/], none: [/neither the name/] },
  'ISC': { all: [/permission to use, copy, modify, and\/or distribute this software for any purpose/, /provided that the above copyright notice and this permission notice appear in all copies/] },
  'Zlib': { all: [/this software is provided 'as-is'/, /altered source versions must be plainly marked/] },
  'CC0-1.0': { all: [/cc0 1\.0 universal/] },
  'MPL-2.0': { all: [/mozilla public license,? version 2\.0/] },
  'Unicode-3.0': { all: [/unicode license v3/] },
  'BSL-1.0': { all: [/boost software license - version 1\.0/] },
  // The three below overlap in sentences with their siblings above, so they are told apart by 「없어야 하는 문장」 (sentences that must be absent) (MIT-0 = MIT without the copyright notice condition · 0BSD = ISC without the same condition).
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
    // `.spdx` = SPDX metadata document (tauri family) · THIRD-PARTY = the notice of others' code that crate contains (the NOTICE section below ships it).
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
// The shape of hand-placed text files (rust/ · extra/) — the leading `//` lines are management comments and are stripped, and of them `// crate: <이름> <버전>` is
//   the crate that file is bound to.
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
        // Even without a full text, if there is a declaration (such as a COPYING holding a copyright line) it is shipped together. A one-line path file (a broken link) is excluded.
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
// An unused replacement file = that crate dropped out of the graph or the version changed (the new version may have started shipping the file) and diverged from the bound version
//   — both mean the file needs another look, so it is raised as a failure.
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

// Groups by same license · same text (even the same MIT is separate if the copyright line differs — the copyright line is part of the notice obligation).
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
  // In order of the first crate name so the reader can look up by crate name. The group that shipped the standard text instead goes last.
  entries.sort((a, b) => a.synthesized - b.synthesized || a.crates[0].localeCompare(b.crates[0]))
  entries.forEach((e, i) => {
    if (i > 0) rustOut.push(THIN_RULE, '')
    rustOut.push(e.synthesized ? 'Used by (synthesized - no license file in the published crate; standard text shown):' : 'Used by:')
    for (const c of e.crates) rustOut.push(`  - ${c}`)
    rustOut.push('', e.text, '')
  })
}

// Notice files a dependency asks to be distributed together — the NOTICE of Apache-2.0 §4(d), and COPYRIGHT · LICENSE-THIRD-PARTY (the copyright · license of
//   others' code that crate contains). No tool's documentation covers this handling (the npm-side tool ships only NOTICE by itself).
//   Everything at the crate root is shipped regardless of the chosen license — there is no harm in shipping. But a file cargo-about already
//   picked up as license text (source_path — atomic-waker's LICENSE-THIRD-PARTY is one) is in Part 1, so it is not shipped again.
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
// A package whose license text was not found leaves only a warning and drops out of the output (exit code 0) — a silent omission, so it is raised as a failure.
if (glf.stderr.includes('Unable to determine license content')) {
  fail('generate-license-file 이 라이선스를 못 찾은 패키지가 있습니다(위 경고) — scripts/licenses/glf.jsonc 의 replace 로 대 주세요')
}

// The two self-promotion lines the tool attaches before and after are stripped — the "This file" in that text would read as if it pointed at this whole file.
const GLF_CREDIT = 'This file was generated with the generate-license-file npm package!\nhttps://www.npmjs.com/package/generate-license-file\n'
let npmText = readFileSync(npmTxt, 'utf8')
if (!npmText.startsWith(GLF_CREDIT + '\n') || !npmText.endsWith(GLF_CREDIT)) {
  fail(`generate-license-file 출력 형식이 예상과 다릅니다 — ${GLF_VERSION} 의 형식이 바뀌었으면 아래 쪼개 읽기부터 다시 볼 것`)
}
npmText = npmText.slice(GLF_CREDIT.length + 1, -GLF_CREDIT.length)

// Prevents the tool from shipping something that is not license text as text — if there is no license file, the one-line identifier in package.json ("MIT")
//   or the URL in the license field, or if `LICENSE.spdx` exists, that SPDX metadata document. All of them lack the permission wording.
//   Group format = 「패키지 목록 → "...the following license:" → 본문 → (선택) "With the following notices:" → 구분선」 (package list → "...the following license:" → text → (optional) "With the following notices:" → separator line).
const npmPackages = new Set()
const badBodies = []
const npmPlaceholders = []
function checkNpmBlock(packages, licenseAndNotices) {
  const body = licenseAndNotices.split('\n\nWith the following notices:')[0].trim()
  if (/^[\w.+\-() ]+$/.test(body) || /^(https?:\/\/|www\.)\S+$/.test(body) || body.startsWith('SPDXVersion:')) {
    badBodies.push(`  ${packages.join(', ')} => "${body.split('\n')[0]}"`)
  }
  // Looks at both text and notices. One group is the same text for several packages, so it reports the whole package list.
  const hit = placeholderIn(licenseAndNotices)
  if (hit) npmPlaceholders.push(`  ${packages.join(', ')}: ${hit}`)
}
const blocks = npmText.split('\n-----------\n').filter((b) => b.trim() !== '')
for (const block of blocks) {
  const start = block.indexOf('the following license:\n\n')
  // The package list is read only before the text — license text also has lines starting with ` - ` (clause lists).
  const packages = start < 0 ? [] : [...block.slice(0, start).matchAll(/^ - (.+)$/gm)].map((m) => m[1])
  if (packages.length === 0) fail(`generate-license-file 출력의 한 묶음을 읽지 못했습니다:\n${block.slice(0, 300)}`)
  packages.forEach((p) => npmPackages.add(p))
  checkNpmBlock(packages, block.slice(start + 'the following license:\n\n'.length))
}

// The replace of glf.jsonc — read by both the key check (below) and the peer backing. Only whole-line `//` comments are stripped and it is read as JSON
//   (the promise in that file's preamble).
let glfReplace
try {
  const raw = readFileSync(join(CONFIG_DIR, 'glf.jsonc'), 'utf8').split(/\r?\n/).filter((l) => !/^\s*\/\//.test(l)).join('\n')
  glfReplace = JSON.parse(raw).replace ?? {}
} catch (err) {
  fail(`scripts/licenses/glf.jsonc 를 읽지 못했습니다(주석은 줄 전체 \`//\` 만 쓸 것) - ${err.message}`)
}

// The license each package declared (license in package.json) — used by both the replacement · peer text check and the allow decision below. It scans node_modules
//   directly to collect name@version → license (even if the same package is installed nested in several versions, they are told apart by version).
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

// The groups above are only the packages the tool shipped — we cannot see what the tool left out. generate-license-file 4.2.5 skips packages marked peer
//   before recording · traversing (resolveNpmDependencies in the installed dist), and also drops a package whose text it could not find with only a warning. So
//   we count the production dependency set separately from package-lock.json and compare — dropping only `dev` (peer is included), and optional
//   dependencies not installed on this platform (binary packages for other OSes) are dropped because they are not on disk. Name = the entry's `name` (for a package installed under an alias, write
//   the real name here — the tool also ships under the real name), otherwise what follows the last `node_modules/` in the path (nested · scope included).
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

// The replace key is the exact name@version (glf.jsonc preamble). It is decided before the 「본문 아닌 것」 (not text) · placeholder checks below — if the key
//   diverges the tool cannot supply the text for that package and those checks blow up too, but the place to fix is the key (the 「라이선스를 못 찾은
//   패키지」 (package with no license found) check right after the tool runs is even earlier than this, already done above). If the installed production dependencies have no such name@version = the version changed or
//   the package dropped out so that text is used nowhere — a signal to re-check the new version's text, so it is raised as a failure.
const badReplaceKeys = Object.keys(glfReplace)
  .filter((k) => !/^@?[^@]+@[^@]+$/.test(k) || !lockPackages.has(k))
  .map((k) => `  ${k}${/^@?[^@]+@[^@]+$/.test(k) ? ' — 설치된 운영 의존에 이 버전이 없다' : ' — 키에 버전이 없다'}`)
if (badReplaceKeys.length > 0) {
  fail(`scripts/licenses/glf.jsonc 의 replace 키가 설치본과 어긋납니다 — 그 버전의 본문을 다시 확인하고 키를 고치거나 지우세요:\n${badReplaceKeys.join('\n')}`)
}

// Whether the hand-attached text (replace file · peer backing below) covers the license that package declared — prevents the mistake of attaching the wrong file (only MIT text for an Apache-2.0
//   package, etc.). It evaluates the declared expression with the licenses confirmed as full text within the text (the same content decision as the Rust side).
// `ids` is taken as the union of what was decided per file — if several files were joined and decided at once, the 「없어야 하는 문장」 (sentences that must be absent) condition
//   would let an MIT file hide MIT-0, BSD-3 hide BSD-2 and ISC hide 0BSD, so a legitimate expression like `MIT-0 AND MIT` would fail.
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

// A production dependency reached only through peer is not seen by the tool at all, so the replace in glf.jsonc is useless too — it is shipped directly here. Text =
//   that file if glf.jsonc has a replace for that name@version, otherwise all the license full-text files in the package folder (file name · content
//   decision is the same as the Rust side) · notices = the notice files in the same folder. It goes through the same shape · the same checks as the groups the tool shipped.
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

// The npm-side tool has no allow list — each shipped package's declared license (npmLicenses above) is decided with the accepted of about.toml.
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

// ── Part 3. Hand-managed bundled-item notices (licenses/extra/*.txt) ───────────────────────────────────

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

// ── Merging ───────────────────────────────────────────────────────────────────────────────────

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

// Written with CRLF — it is a Windows distribution (by Notepad's convention).
mkdirSync(dirname(OUTPUT), { recursive: true })
writeFileSync(OUTPUT, out.replace(/\n/g, '\r\n'))
const synthesizedCount = records.filter((r) => r.synthesized).length
note(
  `완료 — Rust crate ${rustCrates.size}(본문 보정 ${fixes.file} · 대안 라이선스로 ${fixes.switched} · 교체 파일 ${fixes.replaced} · 표준 본문 ${synthesizedCount}) · ` +
    `npm 패키지 ${npmPackages.size}(lock 대조 ${lockPackages.size} · peer 직접 실음 ${peerFallback.length}) · Rust 고지 파일 ${rustNotices.length} · 동봉물 ${extras.length} → ${OUTPUT}`,
)
