#!/usr/bin/env node
// M15 phase B driver — direct WS client to an engram daemon (same frames as the front's ProtocolClient).
//
//   node m15b.mjs <dataDir> <cwd> <scenario> <turns> <eventsOut.jsonl>
//   scenario = tools | hops | flood
// Spawns one fresh codex (StreamJson/app-server) agent, runs <turns> turns, prints a compact log and writes
// wall-clock-stamped key events (send, ToolCall, TurnEnd, QueuedInput, Error) as JSON lines.
import { readFileSync, appendFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { randomUUID } from 'node:crypto'

const [, , dataDir, cwd, scenario, turnsArg, outPath] = process.argv
if (!outPath) {
  console.error('usage: m15b.mjs <dataDir> <cwd> <tools|hops|flood> <turns> <events.jsonl>')
  process.exit(3)
}
const TURNS = Number(turnsArg)
writeFileSync(outPath, '')
const T0 = Date.now()
const log = (...a) => console.log(`+${String(Date.now() - T0).padStart(7)}ms`, ...a)
const rec = (o) => appendFileSync(outPath, JSON.stringify({ wall: Date.now(), ...o }) + '\n')
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

const info = JSON.parse(readFileSync(join(dataDir, 'daemon.json'), 'utf8'))
const ws = new WebSocket(`ws://${info.host}:${info.port}`)
ws.binaryType = 'arraybuffer'
const pending = new Map()
let helloResolve
const hello = new Promise((r) => (helloResolve = r))
ws.onopen = () => ws.send(JSON.stringify({ Auth: { token: info.token, protocol_version: info.protocol_version } }))
ws.onerror = (e) => {
  console.error('ws error', e.message ?? e)
  process.exit(1)
}
ws.onclose = (e) => log('ws closed', e.code)

// ── live state ──
let live = false
let lastEventAt = Date.now()
let turnEndsSinceSend = 0
const outstanding = new Set() // queued input ids not yet Delivered/Dropped
let fatal = null
let textBuf = ''
let onToolCall = null
let toolCalls = 0
let turnEnds = 0

function onEvent(seq, e) {
  lastEventAt = Date.now()
  if (e.type === 'TextDelta') {
    textBuf += e.text
    return
  }
  if (textBuf) {
    log(`  text(${textBuf.length}ch): ${textBuf.replace(/\s+/g, ' ').slice(0, 110)}`)
    textBuf = ''
  }
  switch (e.type) {
    case 'ToolCall': {
      toolCalls++
      let command = ''
      try {
        command = JSON.parse(e.args_json).command ?? ''
      } catch {}
      rec({ ev: 'ToolCall', id: e.id, name: e.name, command: String(command).slice(0, 200) })
      log(`  ToolCall ${e.name} ${String(command).slice(-70)}`)
      onToolCall?.(e, command)
      break
    }
    case 'TurnEnd':
      turnEnds++
      turnEndsSinceSend++
      rec({ ev: 'TurnEnd', turn: e.turn_id, outcome: e.outcome })
      log(`  TurnEnd ${JSON.stringify(e.outcome)}`)
      if (e.outcome?.kind === 'Failed') {
        const d = String(e.outcome.detail ?? '')
        if (/rate.?limit|quota|usage limit|429/i.test(d)) fatal = `rate/quota: ${d.slice(0, 200)}`
      }
      break
    case 'QueuedInput': {
      const op = e.op
      rec({ ev: 'QueuedInput', op: { ...op, text: op.text ? op.text.slice(0, 60) : undefined } })
      if (op.kind === 'Queued') outstanding.add(op.id)
      if (op.kind === 'Delivered' || op.kind === 'Dropped') outstanding.delete(op.id)
      if (op.kind !== 'Queued' && op.kind !== 'Delivered') log(`  QueuedInput ${JSON.stringify(op).slice(0, 160)}`)
      break
    }
    case 'Error':
      rec({ ev: 'Error', message: e.message })
      log(`  Error ${e.message.slice(0, 200)}`)
      if (/rate.?limit|quota|usage limit|429/i.test(e.message)) fatal = `rate/quota: ${e.message.slice(0, 200)}`
      break
    default:
      break
  }
}

ws.onmessage = (ev) => {
  if (typeof ev.data !== 'string') {
    const buf = ev.data
    const view = new DataView(buf)
    if (view.getUint8(0) !== 1 || !live) return
    let e
    try {
      e = JSON.parse(new TextDecoder().decode(new Uint8Array(buf, 29)))
    } catch {
      return
    }
    onEvent(Number(view.getBigUint64(21, false)), e)
    return
  }
  const msg = JSON.parse(ev.data)
  const key = Object.keys(msg)[0]
  const body = msg[key]
  if (key === 'Hello') return helloResolve(body)
  if (key === 'ReplayComplete') live = true
  const rid = body && typeof body === 'object' ? body.request_id : undefined
  if (rid && pending.has(rid)) {
    const p = pending.get(rid)
    pending.delete(rid)
    if (key === 'Error') p.reject(new Error(body.message))
    else p.resolve({ key, body })
  }
}

function cmd(build) {
  const request_id = randomUUID()
  return new Promise((resolve, reject) => {
    pending.set(request_id, { resolve, reject })
    ws.send(JSON.stringify(build(request_id)))
  })
}
function send(agent_id, text, tag) {
  rec({ ev: 'send', tag, text: text.slice(0, 60) })
  return cmd((request_id) => ({ WriteStdin: { agent_id, data: Array.from(new TextEncoder().encode(text)), request_id } }))
}

// idle = saw a TurnEnd since the last prompt, nothing queued outstanding, and `quiet` ms without live events
async function waitIdle(quiet = 6000, limit = 300_000) {
  const start = Date.now()
  while (Date.now() - start < limit) {
    await sleep(250)
    if (fatal) return false
    if (turnEndsSinceSend > 0 && outstanding.size === 0 && Date.now() - lastEventAt > quiet) return true
  }
  log(`  !! waitIdle timeout (turnEndsSinceSend=${turnEndsSinceSend} outstanding=${outstanding.size})`)
  rec({ ev: 'timeout' })
  return false
}

await hello
log(`hello ok port=${info.port} scenario=${scenario} turns=${TURNS}`)
const created = await cmd((request_id) => ({
  CreateProfile: {
    name: `m15-${scenario}`,
    cwd,
    extra_args: [],
    env: [],
    auto_restore: false,
    output_format: 'StreamJson',
    backend: 'codex',
    request_id,
  },
}))
const spawned = await cmd((request_id) => ({
  SpawnProfile: { profile_id: created.body.profile.id, resume: false, request_id },
}))
const agentId = spawned.body.agent.id
log(`agent ${agentId}`)
rec({ ev: 'agent', id: agentId })
ws.send(JSON.stringify({ Subscribe: { agent_id: agentId, epoch: null, after_seq: null } }))
await sleep(1500)

const TOOL_PROMPTS = [
  'Run these four commands one at a time, sequentially, as four separate tool calls, in the current directory: ' +
    '`node flood.js 1000`, then `node flood.js 3000`, then `node flood.js 5000`, then `node flood.js 2000`. ' +
    'Do not print their output; afterwards reply with one short sentence.',
]
const PARALLEL_PROMPT =
  'Run these three commands in parallel: issue all three tool calls at once, simultaneously, not one after another ' +
  '(use parallel tool calls if you can): `node flood.js 3000 20 100`, `node flood.js 3000 20 100`, `node flood.js 3000 20 100`. ' +
  'Do not print their output; afterwards reply with one short sentence saying whether they ran in parallel.'

for (let t = 1; t <= TURNS; t++) {
  if (fatal) break
  turnEndsSinceSend = 0
  if (scenario === 'tools') {
    const parallel = t === TURNS // last turn = explicit parallel attempt
    const p = parallel ? PARALLEL_PROMPT : TOOL_PROMPTS[0]
    log(`turn ${t}${parallel ? ' (parallel attempt)' : ''}`)
    rec({ ev: 'turn', t, parallel })
    await send(agentId, p, parallel ? 'parallel' : 'tools')
  } else if (scenario === 'hops') {
    log(`turn ${t}`)
    rec({ ev: 'turn', t })
    // A then B back-to-back: B is held until turn/start's reply gives the turn id, then steered (hop=signal turn_id)
    const pa = send(agentId, `Reply with just the word OK${t}.`, 'A')
    const pb = send(agentId, `Also append the word TWO to that same reply.`, 'B')
    await Promise.all([pa, pb])
  } else if (scenario === 'flood') {
    log(`turn ${t}`)
    rec({ ev: 'turn', t })
    let fired = false
    onToolCall = (e, command) => {
      if (fired || !String(command).includes('flood.js')) return
      fired = true
      const at = [700, 1100, 1500, 1900]
      at.forEach((ms, i) =>
        setTimeout(() => {
          send(agentId, `Side note ${t}.${i + 1}: no action needed; at the very end just list the note numbers you saw.`, `steer${i + 1}`)
            .then(() => log(`  >> steer ${i + 1} (+${ms}ms)`))
            .catch((err) => log(`  >> steer ${i + 1} failed ${err.message}`))
        }, ms),
      )
    }
    await send(
      agentId,
      'Run exactly this command once in the current directory, verbatim: node flood.js 6000 40 100 . ' +
        'Do not add any redirection, pipe, variable assignment, Select-Object or Out-Null — let its full output ' +
        'print normally. Then reply with only the last line it printed.',
      'flood',
    )
  } else {
    console.error('unknown scenario')
    process.exit(3)
  }
  const ok = await waitIdle()
  if (!ok && fatal) {
    log(`FATAL ${fatal}`)
    break
  }
}
log(`done toolCalls=${toolCalls} turnEnds=${turnEnds} fatal=${fatal ?? 'none'}`)
rec({ ev: 'done', toolCalls, turnEnds, fatal })
process.exit(0)
