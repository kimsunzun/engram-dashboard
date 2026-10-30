// @vitest-environment node
// ADR-0302 — 창 상태 명령
// AC10 게이트(TRD S26-window-chrome §10-4) — 웹은 Tauri 창 제어 API 를 부르지 않는다. 창 닫기 · 최소화 · 최대화 · 보이기 ·
//   포커스는 전부 셸 invoke `window_control` 로 간다(레지스트리 `window.*` — §6-4 · 0탭 자가닫힘 — §5-5). 두 층이다 —
//   ① 소스 타입 검사 ② 그 권한을 어느 capability 도 웹에 주지 않는다는 실행 시점 받침(§9 — 아래 「capability」 구획).
//
// ★rg 한 줄로 바꾸지 말 것★: 문자열 매치는 줄바꿈 · 별칭(`const w = getCurrentWindow(); w.close()`) · 구조 분해에 뚫린다
//   (리뷰 Claude ⑦). 그래서 프로젝트 tsconfig 로 `src` 를 타입 검사해 받는 쪽 타입이 `@tauri-apps/api` 의 `Window` ·
//   `WebviewWindow`(그 자식 클래스 · 그것을 제약으로 둔 타입 매개변수 포함)인 접근을 찾는다 — 속성 접근 · 리터럴 키 접근 ·
//   구조 분해(선언 · 매개변수 · 대입).
// ★짝★ — 같은 프로그램에 위반 꼴을 담은 가상 파일을 하나 넣어 그것이 정확히 잡히는지, 실제 코드에서도 그 타입의 받는
//   쪽(`getCurrentWindow().label`)이 보이는지 본다. 받는 쪽 판정이 눈멀면(패키지 경로 · 타입 이름이 바뀌거나 타입이 `any`
//   로 풀리면) 0 건이 그대로 통과하므로 그것을 빨갛게 만든다.
// 남는 구멍(① 이 못 본다) — 계산된 이름(`w[name]()`) · `Reflect.get` · `any` · `unknown` 으로 지운 타입 · 중첩 대입 구조
//   분해 · 로컬 · 포트 타입으로의 구조적 업캐스트(`const p: { close(): Promise<void> } = getCurrentWindow()` — 받는 쪽이
//   익명 타입이다) · `Pick` · `Omit` 같은 매핑 타입 · `Webview` 받는 쪽(`getCurrentWebview()` — 닫기 · 숨기기 · 보이기 ·
//   포커스를 따로 가진다) · invoke 로 플러그인 명령(`plugin:window|close`)을 직접 부르는 것. 그 꼴들의 받침이 ② 다 —
//   권한이 없으면 셸이 그 호출을 거절한다(실측 2026-10-25 — 구조적 업캐스트 · `Pick` · `Omit` · `Webview` 는 ① 에서 0 건).
//   ② 의 구멍 — `src-tauri/capabilities` 밖의 capability(설정 파일 안 인라인 · 빌드 때 `TAURI_CONFIG` 로 덧대는 것)와
//   `core:default` 같은 묶음이 뒤에 창 제어를 품게 되는 것.

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
  // 제네릭 받는 쪽(`<T extends Window>(w: T)`)과 자식 클래스 안의 `this` 는 그 제약으로 본다 — 둘 다 타입 매개변수라
  //   심볼 이름으로는 안 걸린다.
  if (type.flags & ts.TypeFlags.Instantiable) {
    const constraint = checker.getBaseConstraintOfType(type)
    return constraint !== undefined && isTauriWindowType(constraint, checker, seen)
  }
  const symbol = type.aliasSymbol ?? type.getSymbol()
  // 클래스 선언으로 가른다 — `@tauri-apps/api/event` 가 DOM 의 전역 `Window` 에 칸을 덧대(`declare global`) 그 이름의
  //   선언이 같은 패키지에도 생긴다. 파일만 보면 `window.close()`(DOM)가 잡힌다(실측).
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
  /** `<파일>:<줄> .<이름>` */
  hits: string[]
  /** 받는 쪽이 Tauri 창 타입인 `.label` 접근 수 — 실제 코드에서 그 타입이 풀리는지 보는 짝. */
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
  if (configPath === undefined) throw new Error(`tsconfig.json 을 찾지 못했다(${ROOT})`)
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

describe('AC10 — 웹은 Tauri 창 제어 API 를 부르지 않는다', () => {
  it('src(시험 파일 제외)에 받는 쪽이 Tauri Window · WebviewWindow 인 창 제어 접근이 없다 · 가상 위반 파일은 다 잡힌다', () => {
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

// ── 실행 시점 받침 — capability(셸 ACL) ──
// 웹의 창 제어 호출(위 타입 검사가 못 보는 꼴 · invoke `plugin:window|…` 포함)은 그 권한이 없으면 셸이 거절한다. 그래서
//   어느 capability 도 웹에 창 · 웹뷰 제어 권한을 주지 않는다는 것을 따로 잰다. `core:window:allow-start-dragging`(제목 줄
//   끌기 — U8a)은 창 상태를 바꾸지 않아 허용한다.
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
  /** 읽지 못한 것 — 이 게이트가 그 파일을 못 본 채 통과하지 않게 실패로 센다. */
  unreadable: string[]
}

function readCapabilities(): CapabilityRead {
  const out: CapabilityRead = { capabilities: [], unreadable: [] }
  for (const path of ts.sys.readDirectory(CAPABILITY_DIR)) {
    const file = slash(path).slice(ROOT.length + 1)
    if (!file.endsWith('.json')) {
      out.unreadable.push(`${file}: JSON 이 아니다(이 게이트는 .json 만 읽는다)`)
      continue
    }
    let doc: unknown
    try {
      doc = JSON.parse(ts.sys.readFile(path) ?? '')
    } catch (e) {
      out.unreadable.push(`${file}: ${String(e)}`)
      continue
    }
    // Tauri capability 파일의 세 꼴 — capability 하나 · 그 목록 · `{ capabilities: [...] }`.
    const list = Array.isArray(doc) ? doc : isRecord(doc) && Array.isArray(doc.capabilities) ? doc.capabilities : [doc]
    for (const cap of list) {
      if (!isRecord(cap) || !Array.isArray(cap.permissions)) {
        out.unreadable.push(`${file}: permissions 배열이 없는 항목`)
        continue
      }
      const permissions: string[] = []
      for (const p of cap.permissions) {
        // 권한 항목 = 이름 문자열 또는 범위를 단 `{ identifier, allow, deny }`.
        const id = typeof p === 'string' ? p : isRecord(p) && typeof p.identifier === 'string' ? p.identifier : undefined
        if (id === undefined) out.unreadable.push(`${file}: 읽지 못한 권한 ${JSON.stringify(p)}`)
        else permissions.push(id)
      }
      out.capabilities.push({ file, identifier: cap.identifier, permissions })
    }
  }
  return out
}

describe('AC10 받침 — 어느 capability 도 웹에 창 · 웹뷰 제어 권한을 주지 않는다', () => {
  it('판정 자기 시험 — 금지 이름 14 · 와일드카드는 걸리고 제목 줄 끌기 · 기본 묶음은 지나간다', () => {
    expect(BANNED_PERMISSIONS.size).toBe(14)
    for (const id of [...BANNED_PERMISSIONS, 'core:window:allow-*', 'core:webview:allow-*', 'core:window:*']) {
      expect(isBannedPermission(id), id).toBe(true)
    }
    for (const id of ['core:window:allow-start-dragging', 'core:window:default', 'core:default', 'dialog:allow-open']) {
      expect(isBannedPermission(id), id).toBe(false)
    }
  })

  it('src-tauri/capabilities 의 capability 전부에 금지 권한이 없다 · popup capability 를 읽었다', () => {
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
