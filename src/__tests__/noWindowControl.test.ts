// @vitest-environment node
// ADR-0302 — window state command
// AC10 gate (TRD S26-window-chrome §10-4) — the web never calls the Tauri window control API. Closing, minimizing, maximizing, showing and focusing a window all go through the shell invoke `window_control` (registry `window.*` — §6-4 · 0-tab self-close — §5-5). Two layers — ① a source type check ② a runtime backstop that no capability grants that permission to the web (§9 — the "capability" section below).
// ★Do not turn this into a one-line rg★: string matching is bypassed by line breaks, aliases (`const w = getCurrentWindow(); w.close()`) and destructuring. So the project tsconfig type-checks `src` and finds accesses whose receiver type is `Window` or `WebviewWindow` of `@tauri-apps/api` (including child classes and type parameters constrained by them) — property access, literal-key access, and destructuring (declarations · parameters · assignments).
// ★Pair★ — put one virtual file containing a violating form into the same program and check it is caught exactly, and that the real code also shows a receiver of that type (`getCurrentWindow().label`). If the receiver check goes blind (package path or type name changes, or the type resolves to `any`), zero hits would pass unchanged, so that must fail red.
// Remaining holes (① cannot see them) — computed names (`w[name]()`) · `Reflect.get` · types erased to `any` or `unknown` · nested assignment destructuring · structural upcasts to a local or port type (`const p: { close(): Promise<void> } = getCurrentWindow()`) · mapped types like `Pick` or `Omit` · a `Webview` receiver (`getCurrentWebview()` has its own close, hide, show and focus) · calling the plugin command (`plugin:window|close`) directly through invoke. ② is the backstop: without the permission the shell rejects the call (structural upcasts, `Pick`, `Omit` and `Webview` give 0 hits in ①).
// Holes in ② — capabilities outside `src-tauri/capabilities` (inline in a config file · overlaid at build time through `TAURI_CONFIG`) and bundles like `core:default` that later come to include window control.

import ts from 'typescript'
import { describe, expect, it } from 'vitest'

const BANNED = new Set([
  'close',
  'destroy',
  'minimize',
  'maximize',
  'unmaximize',
  'toggleMaximize',
  'show',
  'hide',
  'setFocus',
  'unminimize',
])
const RECEIVERS = new Set(['Window', 'WebviewWindow'])
const TEST_FILE = /\.test\.tsx?$/

const slash = (p: string): string => p.replace(/\\/g, '/')
const samePath = (a: string, b: string): boolean => slash(a).toLowerCase() === slash(b).toLowerCase()
const isTauriApiFile = (fileName: string): boolean => slash(fileName).includes('/node_modules/@tauri-apps/api/')

const ROOT = slash(ts.sys.getCurrentDirectory())
const PROBE = `${ROOT}/src/__no_window_control_probe__.ts`
const PROBE_SOURCE = `
import { getCurrentWindow, Window } from '@tauri-apps/api/window'
import { getCurrentWebviewWindow, type WebviewWindow } from '@tauri-apps/api/webviewWindow'

const w = getCurrentWindow()
w.close()
getCurrentWindow()
  .minimize()
const { hide } = getCurrentWindow()
const { show: reveal } = w
w['maximize']()
getCurrentWebviewWindow().setFocus()
function takesWindow({ destroy }: Window) {
  return destroy
}
function maybeWindow(m: Window | null) {
  return m?.toggleMaximize()
}
function genericWindow<T extends Window>(g: T) {
  return g.minimize()
}
function genericWebviewWindow<T extends WebviewWindow>({ hide: h }: T) {
  return h
}
class SubWindow extends Window {
  shut() {
    return this.close()
  }
}
let unminimizeRef: unknown
;({ unminimize: unminimizeRef } = w)
const label = w.label
window.close()
export { hide, reveal, takesWindow, maybeWindow, genericWindow, genericWebviewWindow, SubWindow, unminimizeRef, label }
`
const PROBE_HITS = [
  'close',
  'minimize',
  'hide',
  'show',
  'maximize',
  'setFocus',
  'destroy',
  'toggleMaximize',
  'minimize',
  'hide',
  'close',
  'unminimize',
]

function isTauriWindowType(type: ts.Type, checker: ts.TypeChecker, seen = new Set<ts.Type>()): boolean {
  if (seen.has(type)) return false
  seen.add(type)
  if (type.isUnionOrIntersection()) return type.types.some(t => isTauriWindowType(t, checker, seen))
  // Generic receivers (`<T extends Window>(w: T)`) and `this` inside a child class are judged by their constraint — both are type parameters, so symbol name does not catch them.
  if (type.flags & ts.TypeFlags.Instantiable) {
    const constraint = checker.getBaseConstraintOfType(type)
    return constraint !== undefined && isTauriWindowType(constraint, checker, seen)
  }
  const symbol = type.aliasSymbol ?? type.getSymbol()
  // Tell them apart by class declaration — `@tauri-apps/api/event` adds a field to the DOM global `Window` (`declare global`), so a declaration with that name also appears in the same package; by file alone, `window.close()` (DOM) gets caught (measured).
  if (
    symbol !== undefined &&
    RECEIVERS.has(symbol.getName()) &&
    (symbol.declarations ?? []).some(d => ts.isClassDeclaration(d) && isTauriApiFile(d.getSourceFile().fileName))
  ) {
    return true
  }
  return type.isClassOrInterface() && checker.getBaseTypes(type).some(b => isTauriWindowType(b, checker, seen))
}

function literalName(expr: ts.Expression, checker: ts.TypeChecker): string | undefined {
  if (ts.isStringLiteralLike(expr)) return expr.text
  const type = checker.getTypeAtLocation(expr)
  return type.isStringLiteral() ? type.value : undefined
}

function keyName(key: ts.PropertyName | ts.BindingName, checker: ts.TypeChecker): string | undefined {
  if (ts.isIdentifier(key) || ts.isPrivateIdentifier(key) || ts.isStringLiteralLike(key) || ts.isNumericLiteral(key)) {
    return key.text
  }
  if (ts.isComputedPropertyName(key)) return literalName(key.expression, checker)
  return undefined
}

interface Scan {
  /** `<file>:<line> .<name>` */
  hits: string[]
  /** Number of `.label` accesses whose receiver is a Tauri window type — the pair that checks whether that type resolves in the real code. */
  labelReads: number
}

function scan(sf: ts.SourceFile, checker: ts.TypeChecker): Scan {
  const out: Scan = { hits: [], labelReads: 0 }
  const file = slash(sf.fileName).slice(ROOT.length + 1)
  const check = (receiver: ts.Node, name: string | undefined, at: ts.Node): void => {
    if (name === undefined || !BANNED.has(name)) return
    if (!isTauriWindowType(checker.getTypeAtLocation(receiver), checker)) return
    const { line } = sf.getLineAndCharacterOfPosition(at.getStart(sf))
    out.hits.push(`${file}:${line + 1} .${name}`)
  }
  const visit = (node: ts.Node): void => {
    if (ts.isPropertyAccessExpression(node)) {
      check(node.expression, node.name.text, node.name)
      if (node.name.text === 'label' && isTauriWindowType(checker.getTypeAtLocation(node.expression), checker)) {
        out.labelReads += 1
      }
    } else if (ts.isElementAccessExpression(node)) {
      check(node.expression, literalName(node.argumentExpression, checker), node.argumentExpression)
    } else if (ts.isObjectBindingPattern(node)) {
      for (const el of node.elements) {
        if (!el.dotDotDotToken) check(node, keyName(el.propertyName ?? el.name, checker), el)
      }
    } else if (
      ts.isBinaryExpression(node) &&
      node.operatorToken.kind === ts.SyntaxKind.EqualsToken &&
      ts.isObjectLiteralExpression(node.left)
    ) {
      for (const p of node.left.properties) {
        if (ts.isShorthandPropertyAssignment(p) || ts.isPropertyAssignment(p)) check(node.right, keyName(p.name, checker), p)
      }
    }
    ts.forEachChild(node, visit)
  }
  visit(sf)
  return out
}

function buildProgram(): ts.Program {
  const configPath = ts.findConfigFile(ROOT, ts.sys.fileExists, 'tsconfig.json')
  if (configPath === undefined) throw new Error(`tsconfig.json not found(${ROOT})`)
  const read = ts.readConfigFile(configPath, ts.sys.readFile)
  if (read.error) throw new Error(ts.flattenDiagnosticMessageText(read.error.messageText, '\n'))
  const parsed = ts.parseJsonConfigFileContent(read.config, ts.sys, slash(configPath).replace(/\/[^/]*$/, ''))
  const host = ts.createCompilerHost(parsed.options, true)
  const getSourceFile = host.getSourceFile.bind(host)
  const fileExists = host.fileExists.bind(host)
  const readFile = host.readFile.bind(host)
  host.getSourceFile = (name, language, onError, shouldCreate) =>
    samePath(name, PROBE)
      ? ts.createSourceFile(name, PROBE_SOURCE, language, true)
      : getSourceFile(name, language, onError, shouldCreate)
  host.fileExists = name => samePath(name, PROBE) || fileExists(name)
  host.readFile = name => (samePath(name, PROBE) ? PROBE_SOURCE : readFile(name))
  const roots = parsed.fileNames.filter(f => !TEST_FILE.test(f))
  return ts.createProgram({ rootNames: [...roots, PROBE], options: parsed.options, host })
}

describe('AC10 — the web does not call the Tauri window control API', () => {
  it('in src (test files excluded) there is no window control access whose receiver is a Tauri Window · WebviewWindow · a hypothetical violation file is all caught', () => {
    const program = buildProgram()
    const checker = program.getTypeChecker()
    const real: Scan = { hits: [], labelReads: 0 }
    let probe: Scan | undefined
    let scanned = 0
    for (const sf of program.getSourceFiles()) {
      if (sf.isDeclarationFile || !slash(sf.fileName).toLowerCase().startsWith(`${ROOT.toLowerCase()}/src/`)) continue
      const result = scan(sf, checker)
      if (samePath(sf.fileName, PROBE)) {
        probe = result
        continue
      }
      scanned += 1
      real.hits.push(...result.hits)
      real.labelReads += result.labelReads
    }

    expect(probe?.hits.map(hit => hit.replace(/^.* \./, ''))).toEqual(PROBE_HITS)
    expect(scanned).toBeGreaterThan(0)
    expect(real.labelReads).toBeGreaterThan(0)
    expect(real.hits).toEqual([])
  }, 120_000)
})

// ── runtime backstop — capability (shell ACL) ──
// Window control calls from the web (forms the type check cannot see, including invoke `plugin:window|…`) are rejected by the shell without that permission, so we separately measure that no capability gives the web window or webview control permission. `core:window:allow-start-dragging` (title bar drag — U8a) is allowed since it does not change window state.
const CAPABILITY_DIR = `${ROOT}/src-tauri/capabilities`
const BANNED_PERMISSIONS = new Set([
  ...['close', 'destroy', 'minimize', 'maximize', 'unmaximize', 'toggle-maximize', 'show', 'hide', 'set-focus', 'unminimize'].map(
    name => `core:window:allow-${name}`,
  ),
  ...['webview-close', 'webview-hide', 'webview-show', 'set-webview-focus'].map(name => `core:webview:allow-${name}`),
])

function isBannedPermission(id: string): boolean {
  return BANNED_PERMISSIONS.has(id) || (/^core:(window|webview):/.test(id) && id.includes('*'))
}

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v)

interface CapabilityRead {
  capabilities: { file: string; identifier: unknown; permissions: string[] }[]
  /** Files that could not be read — counted as failures so this gate does not pass without having seen them. */
  unreadable: string[]
}

function readCapabilities(): CapabilityRead {
  const out: CapabilityRead = { capabilities: [], unreadable: [] }
  for (const path of ts.sys.readDirectory(CAPABILITY_DIR)) {
    const file = slash(path).slice(ROOT.length + 1)
    if (!file.endsWith('.json')) {
      out.unreadable.push(`${file}: not JSON (this gate reads only .json)`)
      continue
    }
    let doc: unknown
    try {
      doc = JSON.parse(ts.sys.readFile(path) ?? '')
    } catch (e) {
      out.unreadable.push(`${file}: ${String(e)}`)
      continue
    }
    // The three forms of a Tauri capability file — one capability · a list of them · `{ capabilities: [...] }`.
    const list = Array.isArray(doc) ? doc : isRecord(doc) && Array.isArray(doc.capabilities) ? doc.capabilities : [doc]
    for (const cap of list) {
      if (!isRecord(cap) || !Array.isArray(cap.permissions)) {
        out.unreadable.push(`${file}: an entry without a permissions array`)
        continue
      }
      const permissions: string[] = []
      for (const p of cap.permissions) {
        // A permission entry = a name string or `{ identifier, allow, deny }` with a scope attached.
        const id = typeof p === 'string' ? p : isRecord(p) && typeof p.identifier === 'string' ? p.identifier : undefined
        if (id === undefined) out.unreadable.push(`${file}: an unreadable permission ${JSON.stringify(p)}`)
        else permissions.push(id)
      }
      out.capabilities.push({ file, identifier: cap.identifier, permissions })
    }
  }
  return out
}

describe('AC10 support — no capability gives the web window · webview control permission', () => {
  it('the self-test of the verdict — forbidden names 14 · wildcard is caught and title row drag · the default bundle pass', () => {
    expect(BANNED_PERMISSIONS.size).toBe(14)
    for (const id of [...BANNED_PERMISSIONS, 'core:window:allow-*', 'core:webview:allow-*', 'core:window:*']) {
      expect(isBannedPermission(id), id).toBe(true)
    }
    for (const id of ['core:window:allow-start-dragging', 'core:window:default', 'core:default', 'dialog:allow-open']) {
      expect(isBannedPermission(id), id).toBe(false)
    }
  })

  it('the capabilities of src-tauri/capabilities have no forbidden permission, and the popup capability was read', () => {
    const { capabilities, unreadable } = readCapabilities()
    expect(unreadable).toEqual([])
    expect(capabilities.length).toBeGreaterThan(0)
    expect(capabilities.map(c => c.identifier)).toContain('popup')
    const granted = capabilities.flatMap(c =>
      c.permissions.filter(isBannedPermission).map(p => `${c.file} (${String(c.identifier)}): ${p}`),
    )
    expect(granted).toEqual([])
  })
})
