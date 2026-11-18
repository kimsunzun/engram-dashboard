#!/usr/bin/env node
// Hangul check — code and config stay English (S38 TRD §4). Prints `file:line: text` for every line holding Hangul that no
//   rule allows, and exits 1 if there is any (0 = clean, 2 = usage or I/O error).
//
// Usage:
//   node scripts/check-hangul.mjs              every tracked file of the repository (`git ls-files -z` at the git top level)
//   node scripts/check-hangul.mjs <dir>        every file under <dir> (walked; rule paths are relative to <dir>)
//   node scripts/check-hangul.mjs <file>...    only these files (rule paths are relative to the current directory)
//
// Not checked: `docs/**` · `*.md` · `*.html` outside `src/` · `prompts/**` · `CLAUDE.md` · `.claude/**` (docs and the held
//   Korean tooling), `**/fixtures/*.jsonl` (recorded transcripts), and binary files (a binary extension or a NUL byte).
// Allowed Hangul:
//   - `src/i18n/ko.ts` — anywhere except comments (the Korean UI table).
//   - `src-tauri/src/ui_text.rs` — inside string literals on lines that contain `Lang::Ko` (the Korean arms).
//   - frontend test files (`src/**/*.test.ts(x)`) — anywhere except comments, test titles (the whole first argument of
//     `describe` / `it` / `test`) and lines with `console.`.
//   - a line carrying `hangul-ok`, and the lines between a `hangul-ok-begin` and a `hangul-ok-end` comment line (a
//     comment that starts with the marker — `// hangul-ok-begin …`, `# hangul-ok-end`; prose that only mentions a
//     marker opens nothing). An unterminated `hangul-ok-begin` is itself a hit, so a stray marker cannot open the rest
//     of a file.
//   - the path text of a file or folder name that holds Hangul, when it follows `/` or `\` (`launch\<name>.bat`) — the
//     names come from the same file list.
// Accepted residual (TRD §4): Hangul in a test file's assertion diagnostic or in a multi-line `console.*` argument passes.
//
// The rules live in `checkText` and `isSkipped`; the test (`src/util/checkHangul.test.ts`) plants each case in a temp dir.
// Node standard library only — no dependencies.

import { spawnSync } from 'node:child_process'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join, relative, resolve, sep } from 'node:path'
import { pathToFileURL } from 'node:url'

export const HANGUL = /[\u1100-\u11FF\u3130-\u318F\uA960-\uA97F\uAC00-\uD7A3\uD7B0-\uD7FF\uFFA0-\uFFDC]/
const HANGUL_G = new RegExp(HANGUL.source, 'g')

const BINARY_EXT = new Set([
  'png', 'jpg', 'jpeg', 'gif', 'bmp', 'ico', 'icns', 'webp', 'tif', 'tiff', 'psd',
  'pdf', 'zip', 'gz', 'tgz', 'xz', 'bz2', '7z', 'rar', 'tar',
  'exe', 'dll', 'lib', 'pdb', 'so', 'dylib', 'a', 'o', 'obj', 'rlib', 'wasm', 'bin', 'node',
  'woff', 'woff2', 'ttf', 'otf', 'eot',
  'mp3', 'mp4', 'wav', 'ogg', 'webm', 'mov', 'avi',
])

// Folders a directory walk never enters (they are untracked build output or VCS data).
const WALK_SKIP_DIRS = new Set(['.git', 'node_modules', 'target', 'dist'])

/** True when the file at this repository-relative path (forward slashes) is outside the check. */
export function isSkipped(rel) {
  const lower = rel.toLowerCase()
  if (rel === 'CLAUDE.md' || rel.startsWith('docs/') || rel.startsWith('prompts/') || rel.startsWith('.claude/')) return true
  if (lower.endsWith('.md')) return true
  if (lower.endsWith('.html') && !rel.startsWith('src/')) return true
  if (/(^|\/)fixtures\/[^/]+\.jsonl$/.test(rel)) return true
  const dot = lower.lastIndexOf('.')
  if (dot > lower.lastIndexOf('/') && BINARY_EXT.has(lower.slice(dot + 1))) return true
  return false
}

// ── Lexers ──────────────────────────────────────────────────────────────────────────────────────────
// Each returns a per-UTF-16-unit class array (0 = code, 1 = comment, 2 = string or template text). The JS/TS lexer also
//   returns the significant tokens, which the test-title rule reads.

const COMMENT = 1
const STRING = 2

// No `<` here: `</tag>` is a JSX closing tag (test files are often .tsx), and a regex right after `<` does not occur.
const REGEX_AFTER = new Set(['(', ',', '=', ':', '[', '!', '&', '|', '?', '{', '}', ';', '+', '-', '*', '%', '>', '~', '^'])
const REGEX_AFTER_WORDS = new Set(['return', 'typeof', 'case', 'do', 'else', 'in', 'of', 'new', 'delete', 'void', 'throw', 'instanceof', 'yield', 'await'])

export function lexJs(text) {
  const kind = new Uint8Array(text.length)
  const tokens = []
  const braceStack = [] // one entry per open `{`: true = it opened a template `${`
  let i = 0
  const n = text.length
  const prevSignificant = () => tokens[tokens.length - 1]
  const regexAllowed = () => {
    if (text[i + 1] === '>') return false // `/>` closes a JSX tag
    const p = prevSignificant()
    if (!p) return true
    if (p.type === 'punct') return REGEX_AFTER.has(p.value)
    if (p.type === 'ident') return REGEX_AFTER_WORDS.has(p.value)
    return false
  }
  // Reads template text from `i` (just after a backtick or a closing `}` of `${`) to the closing backtick or the next `${`.
  const readTemplate = (start) => {
    let j = i
    // `opens` = the piece starts the template (a backtick), not after a `${…}`; `closes` = it ends the template.
    const opens = text[start] === '`'
    while (j < n) {
      const c = text[j]
      if (c === '\\') { j += 2; continue }
      if (c === '`') {
        kind.fill(STRING, start, j + 1)
        i = j + 1
        tokens.push({ type: 'template', start, end: i, opens, closes: true })
        return
      }
      if (c === '$' && text[j + 1] === '{') {
        kind.fill(STRING, start, j + 2)
        i = j + 2
        braceStack.push(true)
        tokens.push({ type: 'template', start, end: i, opens, closes: false })
        return
      }
      j++
    }
    kind.fill(STRING, start, n)
    i = n
  }
  while (i < n) {
    const c = text[i]
    if (c === '/' && text[i + 1] === '/') {
      const e = text.indexOf('\n', i)
      const end = e === -1 ? n : e
      kind.fill(COMMENT, i, end)
      i = end
    } else if (c === '/' && text[i + 1] === '*') {
      const e = text.indexOf('*/', i + 2)
      const end = e === -1 ? n : e + 2
      kind.fill(COMMENT, i, end)
      i = end
    } else if (c === '"' || c === "'") {
      const start = i
      let j = i + 1
      while (j < n && text[j] !== c && text[j] !== '\n') j += text[j] === '\\' ? 2 : 1
      i = Math.min(j + 1, n)
      kind.fill(STRING, start, i)
      tokens.push({ type: 'string', start, end: i })
    } else if (c === '`') {
      const start = i
      i++
      readTemplate(start)
    } else if (c === '}' && braceStack.length > 0 && braceStack[braceStack.length - 1]) {
      braceStack.pop()
      const start = i
      i++
      readTemplate(start)
    } else if (c === '/' && regexAllowed()) {
      const start = i
      let j = i + 1
      let inClass = false
      let closed = false
      while (j < n && text[j] !== '\n') {
        const d = text[j]
        if (d === '\\') {
          if (text[j + 1] === '\n') break
          j += 2
          continue
        }
        if (d === '[') inClass = true
        else if (d === ']') inClass = false
        else if (d === '/' && !inClass) { closed = true; break }
        j++
      }
      if (closed) {
        j++
        while (j < n && /[a-z]/i.test(text[j])) j++
        i = j
        tokens.push({ type: 'regex', start, end: i })
      } else {
        // No closing `/` on the line — not a regex after all (a division or JSX text): read the slash as punctuation.
        tokens.push({ type: 'punct', start, end: start + 1, value: '/' })
        i = start + 1
      }
    } else if (/[A-Za-z_$]/.test(c)) {
      const start = i
      while (i < n && /[\w$]/.test(text[i])) i++
      tokens.push({ type: 'ident', start, end: i, value: text.slice(start, i) })
    } else if (/\s/.test(c)) {
      i++
    } else if (/[0-9]/.test(c)) {
      const start = i
      while (i < n && /[\w.]/.test(text[i])) i++
      tokens.push({ type: 'number', start, end: i })
    } else {
      if (c === '{') braceStack.push(false)
      else if (c === '}') braceStack.pop()
      tokens.push({ type: 'punct', start: i, end: i + 1, value: c })
      i++
    }
  }
  return { kind, tokens }
}

export function lexRust(text) {
  const kind = new Uint8Array(text.length)
  const n = text.length
  let i = 0
  while (i < n) {
    const c = text[i]
    if (c === '/' && text[i + 1] === '/') {
      const e = text.indexOf('\n', i)
      const end = e === -1 ? n : e
      kind.fill(COMMENT, i, end)
      i = end
    } else if (c === '/' && text[i + 1] === '*') {
      const start = i
      let depth = 1
      i += 2
      while (i < n && depth > 0) {
        if (text[i] === '/' && text[i + 1] === '*') { depth++; i += 2 } else if (text[i] === '*' && text[i + 1] === '/') { depth--; i += 2 } else i++
      }
      kind.fill(COMMENT, start, i)
    } else if (/[rb]/.test(c) && /^b?r#*"/.test(text.slice(i, i + 260)) && !/[\w]/.test(text[i - 1] ?? '')) {
      // raw string: r"…", r#"…"#, br#"…"#
      const m = /^b?r(#*)"/.exec(text.slice(i, i + 260))
      const close = '"' + m[1]
      const start = i
      const e = text.indexOf(close, i + m[0].length)
      i = e === -1 ? n : e + close.length
      kind.fill(STRING, start, i)
    } else if (c === '"' || (c === 'b' && text[i + 1] === '"' && !/[\w]/.test(text[i - 1] ?? ''))) {
      const start = i
      let j = c === 'b' ? i + 2 : i + 1
      while (j < n && text[j] !== '"') j += text[j] === '\\' ? 2 : 1
      i = Math.min(j + 1, n)
      kind.fill(STRING, start, i)
    } else if (c === "'") {
      // char literal or lifetime: `'x'` · `'\n'` are literals, `'a` is a lifetime
      if (text[i + 1] === '\\') {
        const e = text.indexOf("'", i + 2)
        const end = e === -1 ? n : e + 1
        kind.fill(STRING, i, end)
        i = end
      } else if (text[i + 2] === "'") {
        kind.fill(STRING, i, i + 3)
        i += 3
      } else i++
    } else i++
  }
  return kind
}

// ── Rules ───────────────────────────────────────────────────────────────────────────────────────────

const TEST_FN = new Set(['describe', 'it', 'test'])
const TEST_MOD_WITH_ARGS = new Set(['each', 'runIf', 'skipIf', 'for', 'extend'])

// Spans of the first argument of `describe` / `it` / `test` calls (with modifiers such as `.skip` · `.each(table)`).
export function testTitleSpans(tokens) {
  const spans = []
  for (let t = 0; t < tokens.length; t++) {
    const tok = tokens[t]
    if (tok.type !== 'ident' || !TEST_FN.has(tok.value)) continue
    const prev = tokens[t - 1]
    if (prev && prev.type === 'punct' && prev.value === '.') continue
    let k = t + 1
    for (;;) {
      const a = tokens[k]
      if (a && a.type === 'punct' && a.value === '.' && tokens[k + 1]?.type === 'ident') {
        const mod = tokens[k + 1].value
        k += 2
        if (TEST_MOD_WITH_ARGS.has(mod)) {
          if (tokens[k]?.type === 'template') { k++; continue } // test.each`table`
          if (tokens[k]?.type === 'punct' && tokens[k].value === '(') {
            let depth = 0
            for (; k < tokens.length; k++) {
              if (tokens[k].type !== 'punct') continue
              if (tokens[k].value === '(') depth++
              else if (tokens[k].value === ')' && --depth === 0) { k++; break }
            }
          }
        }
        continue
      }
      break
    }
    if (!(tokens[k]?.type === 'punct' && tokens[k].value === '(')) continue
    // The title is the whole first argument: up to the `,` or `)` that ends it outside any `(…)` · `[…]` · `{…}` and
    //   template `${…}` (so `'a' + '…'` and a title on its own line count).
    let depth = 0
    let tpl = 0
    let last = null
    let ended = false
    for (let m = k + 1; m < tokens.length; m++) {
      const tk = tokens[m]
      if (tk.type === 'template') {
        tpl += (tk.opens ? 0 : -1) + (tk.closes ? 0 : 1)
      } else if (tk.type === 'punct') {
        if (depth === 0 && tpl === 0 && (tk.value === ',' || tk.value === ')')) { ended = true; break }
        if (tk.value === '(' || tk.value === '[' || tk.value === '{') depth++
        else if (tk.value === ')' || tk.value === ']' || tk.value === '}') depth--
      }
      last = tk
    }
    if (ended && last) spans.push([tokens[k + 1].start, last.end])
  }
  return spans
}

/** Returns the names (file or folder) holding Hangul in a file list — the path text that may appear after `/` or `\`. */
export function hangulNames(relPaths) {
  const names = new Set()
  for (const p of relPaths) for (const seg of p.split('/')) if (HANGUL.test(seg)) names.add(seg)
  return [...names]
}

function maskPathNames(text, names) {
  let out = text
  for (const name of names) {
    let from = 0
    for (;;) {
      const at = out.indexOf(name, from)
      if (at === -1) break
      if (at > 0 && (out[at - 1] === '/' || out[at - 1] === '\\')) {
        out = out.slice(0, at) + 'x'.repeat(name.length) + out.slice(at + name.length)
      }
      from = at + name.length
    }
  }
  return out
}

// A block marker line: a comment (`//` · `#` · `/*` · `{/*` · `<!--` · `--` · `;` · `::` · `REM`) whose text starts with the
//   marker. Prose that mentions a marker name mid-comment is not one — it would silently open the rest of the file.
const BLOCK_MARK = /^\s*(?:\/\/[/!]?|#|\{?\/\*+|<!--|--|;|::|REM\b|rem\b)\s*hangul-ok-(begin|end)\b/

/**
 * Checks one file's text. `rel` = the rule path (forward slashes, relative to the checked root), `names` = Hangul path
 * names that may appear after `/` or `\`. Returns `[{ line, text }]` (1-based line, the raw line without `\r`).
 */
export function checkText(rel, text, names = []) {
  if (!HANGUL.test(text)) return []
  const lines = text.split('\n')
  const masked = maskPathNames(text, names)
  if (!HANGUL.test(masked)) return []

  const isKo = rel === 'src/i18n/ko.ts'
  const isUiText = rel === 'src-tauri/src/ui_text.rs'
  const isFrontTest = rel.startsWith('src/') && /\.test\.tsx?$/.test(rel)
  let kind = null
  let titleMask = null
  if (isKo || isFrontTest) {
    const lexed = lexJs(masked)
    kind = lexed.kind
    if (isFrontTest) {
      titleMask = new Uint8Array(masked.length)
      for (const [s, e] of testTitleSpans(lexed.tokens)) titleMask.fill(1, s, e)
    }
  } else if (isUiText) {
    kind = lexRust(masked)
  }

  const hits = []
  let offset = 0
  let blockStart = 0 // 1-based line of an open `hangul-ok-begin`, 0 = none
  for (let ln = 0; ln < lines.length; ln++) {
    const raw = lines[ln]
    const lineStart = offset
    offset += raw.length + 1
    const shown = raw.endsWith('\r') ? raw.slice(0, -1) : raw
    const mark = BLOCK_MARK.exec(raw)
    if (mark?.[1] === 'begin') { blockStart = ln + 1; continue }
    if (mark?.[1] === 'end') { blockStart = 0; continue }
    if (blockStart || raw.includes('hangul-ok')) continue
    const line = masked.slice(lineStart, lineStart + raw.length)
    if (!HANGUL.test(line)) continue
    let bad = false
    if (isKo) {
      bad = someHangul(line, (col) => kind[lineStart + col] === COMMENT)
    } else if (isUiText) {
      const koLine = line.includes('Lang::Ko')
      bad = someHangul(line, (col) => !(koLine && kind[lineStart + col] === STRING))
    } else if (isFrontTest) {
      bad = line.includes('console.') ||
        someHangul(line, (col) => kind[lineStart + col] === COMMENT || titleMask[lineStart + col] === 1)
    } else {
      bad = true
    }
    if (bad) hits.push({ line: ln + 1, text: shown })
  }
  if (blockStart) hits.push({ line: blockStart, text: 'hangul-ok-begin without a matching hangul-ok-end' })
  return hits
}

function someHangul(line, isBad) {
  HANGUL_G.lastIndex = 0
  let m
  while ((m = HANGUL_G.exec(line)) !== null) if (isBad(m.index)) return true
  return false
}

// ── File lists ──────────────────────────────────────────────────────────────────────────────────────

function gitTopLevel(cwd) {
  const r = spawnSync('git', ['rev-parse', '--show-toplevel'], { cwd, encoding: 'utf8' })
  return r.status === 0 ? r.stdout.trim() : null
}

function gitLsFiles(root) {
  const r = spawnSync('git', ['ls-files', '-z'], { cwd: root, encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 })
  if (r.status !== 0) throw new Error(`git ls-files failed in ${root}: ${r.stderr || r.error?.message || r.status}`)
  return r.stdout.split('\0').filter((p) => p !== '')
}

function walk(root) {
  const out = []
  const visit = (dir) => {
    for (const ent of readdirSync(dir, { withFileTypes: true })) {
      if (ent.isDirectory()) {
        if (!WALK_SKIP_DIRS.has(ent.name)) visit(join(dir, ent.name))
      } else if (ent.isFile()) {
        out.push(relative(root, join(dir, ent.name)).split(sep).join('/'))
      }
    }
  }
  visit(root)
  return out
}

/** Checks `relPaths` under `root`. Returns `{ hits: [{ file, line, text }], names }`. */
export function checkFiles(root, relPaths, names = hangulNames(relPaths)) {
  const hits = []
  for (const rel of relPaths) {
    if (isSkipped(rel)) continue
    let buf
    try {
      buf = readFileSync(join(root, rel))
    } catch (err) {
      if (err.code === 'ENOENT' || err.code === 'EISDIR') continue // deleted in the work tree, or a submodule
      throw err
    }
    if (buf.includes(0)) continue // binary
    const text = buf.toString('utf8')
    for (const h of checkText(rel, text, names)) hits.push({ file: rel, ...h })
  }
  return { hits }
}

/** Resolves the command-line arguments into a check and runs it. Returns `{ hits, lines, code }` without printing. */
export function run(args, cwd = process.cwd()) {
  const top = gitTopLevel(cwd)
  if (args.length === 0) {
    if (!top) throw new Error('not inside a git repository — pass a directory or files')
    const files = gitLsFiles(top)
    return finish(checkFiles(top, files).hits)
  }
  // Korean names of the enclosing repository count too, so a reference from a checked subfolder to a file outside it
  //   (`scripts/` naming `launch\<name>.bat`) is still path text.
  let repoNames = []
  if (top) {
    try {
      repoNames = hangulNames(gitLsFiles(top))
    } catch {
      // not a usable repository — only the names of the checked files count
    }
  }
  const withRepo = (names) => [...new Set([...names, ...repoNames])]
  const hits = []
  const loose = []
  for (const a of args) {
    const abs = resolve(cwd, a)
    if (statSync(abs).isDirectory()) {
      const files = walk(abs)
      const found = checkFiles(abs, files, withRepo(hangulNames(files))).hits
      hits.push(...found.map((h) => ({ ...h, file: join(a, h.file).split(sep).join('/') })))
    } else {
      loose.push(relative(cwd, abs).split(sep).join('/'))
    }
  }
  if (loose.length > 0) hits.push(...checkFiles(cwd, loose, withRepo(hangulNames(loose))).hits)
  return finish(hits)
}

function finish(hits) {
  const lines = hits.map((h) => `${h.file}:${h.line}: ${h.text.trim()}`)
  return { hits, lines, code: hits.length > 0 ? 1 : 0 }
}

const invokedAsMain =
  process.argv[1] !== undefined && pathToFileURL(resolve(process.argv[1])).href === import.meta.url
if (invokedAsMain) {
  let result
  try {
    result = run(process.argv.slice(2))
  } catch (err) {
    process.stderr.write(`check-hangul: ${err.message}\n`)
    process.exit(2)
  }
  for (const l of result.lines) process.stdout.write(l + '\n')
  if (result.code !== 0) process.stderr.write(`check-hangul: ${result.hits.length} line(s) with Hangul outside the allowlist\n`)
  process.exit(result.code)
}
