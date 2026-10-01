import { readFileSync } from 'node:fs'
const [, , events, log] = process.argv
const ev = readFileSync(events, 'utf8').trim().split('\n').map((l) => JSON.parse(l))
const lines = readFileSync(log, 'utf8').split('\n')
const toolEnds = lines.filter((l) => l.includes('phase="signal"') && l.includes('signal="tool_end"'))
  .map((l) => Number(/recv_ms=(\d+)/.exec(l)[1]))
const firstDelta = lines.find((l) => l.includes('commandExecution/outputDelta'))
const fd = firstDelta ? Date.parse(firstDelta.slice(0, 27)) : null
let turn = null
const rows = []
for (const e of ev) {
  if (e.ev === 'turn') turn = { t: e.t, sends: [] }
  if (e.ev === 'ToolCall' && turn) turn.tool = e.wall
  if (e.ev === 'send' && turn && String(e.tag).startsWith('steer')) turn.sends.push(e.wall)
  if (e.ev === 'TurnEnd' && turn && turn.tool) {
    const te = toolEnds.find((x) => x > turn.tool)
    rows.push({ t: turn.t, sendsAfterTool: turn.sends.map((s) => s - turn.tool), toolEndAfterTool: te - turn.tool,
      allBeforeToolEnd: turn.sends.every((s) => s < te), marginMs: te - Math.max(...turn.sends) })
    turn = null
  }
}
console.log(JSON.stringify(rows))
if (fd) console.log('firstOutputDelta(turn1) after ToolCall ms =', fd - ev.find((e) => e.ev === 'ToolCall').wall)
