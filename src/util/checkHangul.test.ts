// S38 TRD §4: regression net for the Hangul check (`scripts/check-hangul.mjs`) — code and config stay English, and the only
//   Hangul that passes is the allowlist (the Korean UI tables, frontend test values, recorded fixtures, `hangul-ok` markers,
//   Korean file-name paths). Each case is planted in a temp dir and the script runs over that dir, so the rule paths
//   (`src/i18n/ko.ts` · `src-tauri/src/ui_text.rs` · `src/**/*.test.ts`) are the real ones.
// ★Every planted Hangul character is built from `\u` escapes★ — this file must itself pass the check it tests, and a
//   literal would also hide which string is meant to be a hit.
// Placement exception = the same as `devConfigOverlay.test.ts` (the target is in `scripts/`, and vitest only collects `src/**`).
import { afterAll, beforeAll, describe, expect, it } from 'vitest'

// Node standard modules, declared at this file's scope only (the same way as `devConfigOverlay.test.ts`).
declare function require(id: string): {
  mkdtempSync(prefix: string): string
  mkdirSync(path: string, opts: { recursive: boolean }): void
  writeFileSync(path: string, data: string | Uint8Array, enc?: string): void
  rmSync(path: string, opts: { recursive: boolean; force: boolean }): void
  tmpdir(): string
  join(...parts: string[]): string
  dirname(path: string): string
  createRequire(filename: string): (id: string) => unknown
  spawnSync(cmd: string, args: string[], opts: { cwd: string }): { status: number | null }
}
declare const process: { cwd(): string }

const { mkdtempSync, mkdirSync, writeFileSync, rmSync } = require('node:fs')
const { tmpdir } = require('node:os')
const { join, dirname } = require('node:path')
const { createRequire } = require('node:module')
const { spawnSync } = require('node:child_process')

// The plain Node loader, not `import` (vite would transform the script — see `devConfigOverlay.test.ts`).
const nodeRequire = createRequire(join(process.cwd(), 'package.json'))
type Hit = { file: string; line: number; text: string }
const { run, checkText } = nodeRequire(join(process.cwd(), 'scripts', 'check-hangul.mjs')) as {
  run: (args: string[], cwd?: string) => { hits: Hit[]; lines: string[]; code: number }
  checkText: (rel: string, text: string, names?: string[]) => { line: number; text: string }[]
}

// "hangul", "test" and "build" in Hangul — any Hangul syllables do.
const KO = '\uD55C\uAE00'
const KO2 = '\uC2DC\uD5D8'
const KO_FILE = '\uBE4C\uB4DC'

let root = ''
let result: { hits: Hit[]; lines: string[]; code: number }

function plant(rel: string, body: string | Uint8Array): void {
  const path = join(root, rel)
  mkdirSync(dirname(path), { recursive: true })
  writeFileSync(path, body)
}

// Hit lines per planted file, as `file:line`.
function hitsOf(file: string): number[] {
  return result.hits.filter((h) => h.file.endsWith(file)).map((h) => h.line)
}

beforeAll(() => {
  root = mkdtempSync(join(tmpdir(), 'engram-hangul-'))
  // Hits: a Rust comment, a `console.warn` in frontend code, a plain string in a config file.
  plant('crates/x/src/lib.rs', ['fn a() {}', `// ${KO} comment`, 'fn b() {}'].join('\n'))
  plant('src/api/thing.ts', ['export const x = 1', `console.warn('${KO}')`].join('\n'))
  plant('config.toml', `name = "${KO}"\n`)
  // ko.ts: a value passes, a comment is a hit.
  plant('src/i18n/ko.ts', [`export const ko = {`, `  a: '${KO}',`, `  // ${KO2}`, `  b: \`${KO} \${x}\`,`, '}'].join('\n'))
  // ui_text.rs: a Korean arm passes; Hangul off a `Lang::Ko` line or outside a literal is a hit.
  plant(
    'src-tauri/src/ui_text.rs',
    [
      `        (Lang::Ko, MenuAction::Quit) => "${KO}",`,
      `        (Lang::En, MenuAction::Quit) => "${KO}",`,
      `        (Lang::Ko, MenuAction::Stop) => "x", // ${KO2}`,
    ].join('\n'),
  )
  // A frontend test file: assertion values pass; titles, comments and `console.` lines are hits.
  plant(
    'src/a/b.test.ts',
    [
      `describe('${KO} title', () => {`,
      `  it.each([1, 2])('${KO2} %s', (n) => {`,
      `    expect(f(n)).toBe('${KO}')`,
      `    // ${KO}`,
      `    console.log('${KO}')`,
      `  })`,
      `  test(\`${KO} \${1}\`, () => {})`,
      `})`,
    ].join('\n'),
  )
  // A .tsx test file: a comment after a JSX closing or self-closing tag is still a comment (not read as a regex), JSX
  //   text is a value, and a title is the whole first argument (a concatenation, or on its own line).
  plant(
    'src/c/d.test.tsx',
    [
      `render(<p>x</p>) // ${KO}`,
      `render(<Foo bar={x} />) // ${KO}`,
      `render(<p>${KO}</p>)`,
      `it('a ' + '${KO2}', () => {})`,
      `it(`,
      `  '${KO}',`,
      `  () => {},`,
      `)`,
    ].join('\n'),
  )
  // A .test.ts outside src/ is not a frontend test file — its values are hits too.
  plant('crates/w/a.test.ts', `expect(x).toBe('${KO}')\n`)
  // Markers: a trailing marker and a block pass; an unterminated block is a hit at its opening line.
  plant('src/marked.ts', [`const a = '${KO}' // hangul-ok`, '// hangul-ok-begin', `const b = '${KO}'`, '// hangul-ok-end', `const c = '${KO}'`].join('\n'))
  plant('src/open.ts', ['// hangul-ok-begin', `const d = '${KO}'`].join('\n'))
  // Block markers in other comment styles; prose that only mentions a marker name opens no block.
  plant('config/a.yml', ['# hangul-ok-begin', `k: ${KO}`, '# hangul-ok-end', `j: ${KO}`].join('\n'))
  plant('src/e.tsx', ['{/* hangul-ok-begin */}', `<p>${KO}</p>`, '{/* hangul-ok-end */}', `<p>${KO}</p>`].join('\n'))
  plant('crates/z/src/lib.rs', ['// a marker name in prose: hangul-ok-begin', `// ${KO}`, '// hangul-ok-end'].join('\n'))
  // A Korean file name: its path text after `/` or `\` passes; the bare word does not.
  plant(`launch/${KO_FILE}.bat`, '@echo off\n')
  plant('scripts/run.ps1', [`# see launch\\${KO_FILE}.bat and launch/${KO_FILE}.bat`, `# ${KO_FILE} alone`].join('\n'))
  // Not checked at all: docs, markdown (CLAUDE.md included), HTML outside src/, prompts, .claude, fixtures, binaries.
  plant('docs/a.txt', KO)
  plant('notes.md', KO)
  plant('CLAUDE.md', KO)
  plant('index.html', KO)
  plant('src/page.html', `<p>${KO}</p>\n`)
  plant('prompts/p.txt', KO)
  plant('.claude/x.json', KO)
  plant('crates/y/tests/fixtures/run.jsonl', `{"t":"${KO}"}\n`)
  // `**/fixtures/*.jsonl` = directly inside a fixtures folder; a deeper one is checked.
  plant('crates/y/tests/fixtures/sub/deep.jsonl', `{"t":"${KO}"}\n`)
  plant('assets/pic.png', KO)
  plant('assets/blob.dat', new Uint8Array([0x41, 0x00, 0xed, 0x95, 0x9c]))
  result = run([root])
})

afterAll(() => {
  if (root !== '') rmSync(root, { recursive: true, force: true })
})

describe('check-hangul', () => {
  it('flags a comment in a .rs file', () => {
    expect(hitsOf('crates/x/src/lib.rs')).toEqual([2])
  })

  it('flags a console.warn in frontend code and a string in a config file', () => {
    expect(hitsOf('src/api/thing.ts')).toEqual([2])
    expect(hitsOf('config.toml')).toEqual([1])
  })

  it('allows a ko.ts value (also inside a template) but flags a ko.ts comment', () => {
    expect(hitsOf('src/i18n/ko.ts')).toEqual([3])
  })

  it('allows only string literals on Lang::Ko lines in ui_text.rs', () => {
    expect(hitsOf('src-tauri/src/ui_text.rs')).toEqual([2, 3])
  })

  it('allows test values but flags test titles, comments and console lines', () => {
    expect(hitsOf('src/a/b.test.ts')).toEqual([1, 2, 4, 5, 7])
  })

  it('flags a comment after a JSX tag and a concatenated or multi-line title, but allows JSX text', () => {
    expect(hitsOf('src/c/d.test.tsx')).toEqual([1, 2, 4, 6])
  })

  it('treats a .test.ts outside src/ as ordinary code', () => {
    expect(hitsOf('crates/w/a.test.ts')).toEqual([1])
  })

  it('allows marker lines and marker blocks, and flags an unterminated block', () => {
    expect(hitsOf('src/marked.ts')).toEqual([5])
    expect(hitsOf('src/open.ts')).toEqual([1])
  })

  it('accepts block markers in other comment styles, but a marker named in prose opens no block', () => {
    expect(hitsOf('config/a.yml')).toEqual([4])
    expect(hitsOf('src/e.tsx')).toEqual([4])
    expect(hitsOf('crates/z/src/lib.rs')).toEqual([2])
  })

  it('allows the path text of a Korean file name but not the bare word', () => {
    expect(hitsOf('scripts/run.ps1')).toEqual([2])
    expect(hitsOf(`launch/${KO_FILE}.bat`)).toEqual([])
  })

  it('skips docs, markdown, HTML outside src/, prompts, .claude, fixtures and binaries', () => {
    for (const f of ['docs/a.txt', 'notes.md', 'CLAUDE.md', 'index.html', 'prompts/p.txt', '.claude/x.json', 'run.jsonl', 'pic.png', 'blob.dat']) {
      expect(hitsOf(f)).toEqual([])
    }
    expect(hitsOf('src/page.html')).toEqual([1])
    expect(hitsOf('fixtures/sub/deep.jsonl')).toEqual([1])
  })

  it('checks only the given files, with rule paths relative to the current directory', () => {
    const r = run(['src/i18n/ko.ts', 'config.toml'], root)
    expect(r.hits.map((h) => `${h.file}:${h.line}`)).toEqual(['src/i18n/ko.ts:3', 'config.toml:1'])
  })

  it('checks only the tracked files of the repository by default (git ls-files)', () => {
    const repo = mkdtempSync(join(tmpdir(), 'engram-hangul-git-'))
    try {
      writeFileSync(join(repo, 'tracked.rs'), `// ${KO}\n`)
      writeFileSync(join(repo, 'untracked.rs'), `// ${KO}\n`)
      mkdirSync(join(repo, 'launch'), { recursive: true })
      writeFileSync(join(repo, 'launch', `${KO_FILE}.bat`), '@echo off\n')
      writeFileSync(join(repo, 'ref.ps1'), `# calls launch\\${KO_FILE}.bat\n`)
      expect(spawnSync('git', ['init', '-q'], { cwd: repo }).status).toBe(0)
      expect(spawnSync('git', ['add', 'tracked.rs', 'launch', 'ref.ps1'], { cwd: repo }).status).toBe(0)
      expect(run([], repo).hits.map((h) => `${h.file}:${h.line}`)).toEqual(['tracked.rs:1'])
    } finally {
      rmSync(repo, { recursive: true, force: true })
    }
    // Spawns git three times plus the checker — ~0.5 s alone, ~9 s under the full parallel suite, past the 5 s default.
  }, 30_000)

  it('reports file:line: text and exits 1 when anything is flagged', () => {
    expect(result.code).toBe(1)
    expect(result.lines).toContain(`${join(root, 'crates/x/src/lib.rs').split('\\').join('/')}:2: // ${KO} comment`)
  })

  it('exits 0 on clean text', () => {
    expect(checkText('src/clean.ts', 'export const a = 1\n')).toEqual([])
    const clean = mkdtempSync(join(tmpdir(), 'engram-hangul-clean-'))
    try {
      writeFileSync(join(clean, 'a.rs'), 'fn a() {}\n')
      expect(run([clean]).code).toBe(0)
    } finally {
      rmSync(clean, { recursive: true, force: true })
    }
    // Spawns the checker as a child process — slow under the full parallel suite (see the git ls-files test).
  }, 30_000)
})
