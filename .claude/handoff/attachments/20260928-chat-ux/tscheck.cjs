// Scratch type-check harness: runs the repo's tsc program with in-memory file overrides.
// Scenarios: baseline (== tsc --noEmit), "after" (generated StructuredEvent gains ToolResult + category),
// "drift" (generated ToolOutcome has an extra word), "required" (tool item fields made required).
const path = require('path')
const REPO = 'I:/Engram/apps/engram-dashboard-wt1'
const ts = require(path.join(REPO, 'node_modules/typescript'))
const fs = require('fs')

const cfg = ts.readConfigFile(path.join(REPO, 'tsconfig.json'), ts.sys.readFile)
const parsed = ts.parseJsonConfigFileContent(cfg.config, ts.sys, REPO)
const norm = (p) => path.resolve(p).toLowerCase().replace(/\\/g, '/')

function run(label, overrides) {
  const ov = new Map(Object.entries(overrides).map(([k, v]) => [norm(k), v]))
  const host = ts.createCompilerHost(parsed.options)
  const read = host.readFile.bind(host)
  const exists = host.fileExists.bind(host)
  const getSource = host.getSourceFile.bind(host)
  host.readFile = (f) => (ov.has(norm(f)) ? ov.get(norm(f)) : read(f))
  host.fileExists = (f) => ov.has(norm(f)) || exists(f)
  host.getSourceFile = (f, lang, onErr, create) =>
    ov.has(norm(f)) ? ts.createSourceFile(f, ov.get(norm(f)), lang) : getSource(f, lang, onErr, create)
  const program = ts.createProgram(parsed.fileNames, parsed.options, host)
  const diags = ts.getPreEmitDiagnostics(program)
  console.log(`== ${label}: ${diags.length} diagnostics`)
  for (const d of diags) {
    const msg = ts.flattenDiagnosticMessageText(d.messageText, ' ').slice(0, 220)
    if (d.file) {
      const { line } = d.file.getLineAndCharacterOfPosition(d.start)
      console.log(`  ${path.relative(REPO, d.file.fileName)}:${line + 1} TS${d.code} ${msg}`)
    } else console.log(`  TS${d.code} ${msg}`)
  }
}

function replaceOnce(src, from, to) {
  if (!src.includes(from)) throw new Error('anchor not found: ' + from.slice(0, 60))
  return src.replace(from, to)
}

const BIND = path.join(REPO, 'crates/engram-dashboard-protocol/bindings')
const seFile = path.join(BIND, 'StructuredEvent.ts')
const se = fs.readFileSync(seFile, 'utf8')
let seAfter = replaceOnce(
  se,
  'id: string | null, turn_id: string | null, message_id: string | null, } | { "type": "Usage"',
  'id: string | null, turn_id: string | null, message_id: string | null, category?: ToolCategory, } | { "type": "Usage"',
)
seAfter = replaceOnce(
  seAfter,
  '| { "type": "QueuedInput", op: QueuedInputEvent, };',
  '| { "type": "QueuedInput", op: QueuedInputEvent, } | { "type": "ToolResult", id: string, outcome: ToolOutcome, };',
)
seAfter = replaceOnce(
  seAfter,
  'import type { TurnOutcome } from "./TurnOutcome";',
  'import type { TurnOutcome } from "./TurnOutcome";\nimport type { ToolCategory } from "./ToolCategory";\nimport type { ToolOutcome } from "./ToolOutcome";',
)
const catFile = path.join(BIND, 'ToolCategory.ts')
const outFile = path.join(BIND, 'ToolOutcome.ts')
const cat =
  'export type ToolCategory = "Read" | "Search" | "List" | "Edit" | "Command" | "Web" | "Agent" | "Mcp" | "Other";\n'
const out = 'export type ToolOutcome = "Completed" | "Failed" | "Declined" | "Refused";\n'
const outDrift = 'export type ToolOutcome = "Completed" | "Failed" | "Declined" | "Refused" | "TimedOut";\n'

const accFile = path.join(REPO, 'src/components/slot/structuredAccumulator.ts')
const acc = fs.readFileSync(accFile, 'utf8')
let accRequired = replaceOnce(acc, '      category?: ToolCategory\n', '      category: ToolCategory\n')
accRequired = replaceOnce(accRequired, '      resultMark?: ToolResultMark | null\n', '      resultMark: ToolResultMark | null\n')

// Negative control: reading a field inside the guard branch must be a tsc error BEFORE the variant exists.
const accReadInGuard = replaceOnce(
  acc,
  'if (isToolResultEvent(ev)) return this.consumeToolResult(ev)',
  'if (isToolResultEvent(ev)) { console.log(ev.id); return this.consumeToolResult(ev) }',
)

run('baseline (tree as is)', {})
run('after (generated ToolResult + category)', { [seFile]: seAfter, [catFile]: cat, [outFile]: out })
run('drift (generated ToolOutcome has extra word)', { [seFile]: seAfter, [catFile]: cat, [outFile]: outDrift })
run('required tool fields (contract shape)', { [accFile]: accRequired })
run('neg control: field read inside guard, before', { [accFile]: accReadInGuard })
run('neg control: field read inside guard, after', {
  [accFile]: accReadInGuard,
  [seFile]: seAfter,
  [catFile]: cat,
  [outFile]: out,
})
