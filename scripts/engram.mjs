#!/usr/bin/env node
// scripts/engram.mjs — THROWAWAY spike (direction of ADR-0014 · CLI-via-Bash realization, to be rolled back).
// Release-safe LLM control path PoC: daemon.json portfile → daemon WS → AgentCommand JSON.
//   - dev CDP/window.__TAURI__ dies on a release exe, but the daemon WS lives regardless of build.
//   - a spawned Claude (with Bash) calling this CLI = driving the app. If formally adopted, clean up after PRD/ADR.
// Zero dependencies: uses only the node 18+ built-in WebSocket (mirrors the cdp.mjs pattern).
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import net from 'node:net'
import { fileURLToPath } from 'node:url'

// Resolves the daemon.json location — covers both dev (repo <root>/.engram-dev) and release (<exe folder>/data).
// This candidate search is the core of "release-safe": whichever build it is, if a daemon is up, it attaches through the portfile.
//
// ★Do not point at a release daemon with ENGRAM_DATA_DIR★: since ADR-0134 that variable is also the single-instance
//   scope, so if a daemon starts with it set it picks a different folder and quietly becomes a second daemon. This function
//   finds it on its own.
// ★It must tolerate torn reads (ADR-0135)★: the daemon holds daemon.json and writes **in place** (temp file +
//   rename is impossible — we hold it with delete sharing closed). So reading right after it is truncated to length 0 or in the middle of
//   a write shows an empty file or half a JSON. That is not corruption but **not ready yet**, so it re-reads briefly.
//   ★Do not call JSON.parse bare★ — it dies of a SyntaxError even when the daemon is fine.
//   A failure to open (a third party briefly opening it with narrow sharing) is treated the same.
const PORTFILE_READ_ATTEMPTS = 10   // Total wait cap ≈ 500ms — daemon publishing takes milliseconds, so this is generous.
const PORTFILE_READ_DELAY_MS = 50   // The same value as the polling period of the shell's discovery module.

function sleepSync(ms) { Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms) }

// The parsed record on success, null if intact content is not seen within the budget (it does not throw).
// ★Narrowing candidates (findPortfile's liveness diagnosis) gets a short budget★: if every candidate used the full budget, a few dead candidates
//   alone would stall the CLI for seconds at a time. Only after the file to attach to is settled (connect) does it wait generously.
function readPortfile(p, attempts = PORTFILE_READ_ATTEMPTS) {
  for (let i = 0; i < attempts; i++) {
    try {
      const info = JSON.parse(fs.readFileSync(p, 'utf8'))
      if (info && typeof info.port === 'number' && typeof info.token === 'string') return info
    } catch {}
    if (i < attempts - 1) sleepSync(PORTFILE_READ_DELAY_MS)
  }
  return null
}

// The portfile location inside the data folder (canonical = daemon `data_dir::DataLayout::daemon_file` — ADR-0264 · ADR-0271).
//   ★This line is a hand copy that no test ties down★ — unlike the shell copy there is no same-path test, so if the canonical one is changed, fix this one by hand too.
const PORTFILE_IN_DATA_DIR = path.join('daemon', 'run', 'daemon.json')

function findPortfile() {
  const candidates = []
  if (process.env.ENGRAM_DATA_DIR) candidates.push(path.join(process.env.ENGRAM_DATA_DIR, PORTFILE_IN_DATA_DIR))
  // ★The dev daemon relative to the script's own location★: this file is <repo>/scripts/engram.mjs so ../.engram-dev is the repo's dev
  //   data folder. It is found regardless of the calling cwd (fine even if an agent calls it from another cwd — a gap exposed in the meta-test, now fixed).
  try {
    const scriptDir = path.dirname(fileURLToPath(import.meta.url)) // <repo>/scripts
    candidates.push(path.join(scriptDir, '..', '.engram-dev', PORTFILE_IN_DATA_DIR))
  } catch {}
  // dev (extra defense): walk up to the repo root that has .git and use <root>/.engram-dev
  let dir = process.cwd()
  for (let i = 0; i < 8; i++) {
    if (fs.existsSync(path.join(dir, '.git'))) { candidates.push(path.join(dir, '.engram-dev', PORTFILE_IN_DATA_DIR)); break }
    const parent = path.dirname(dir)
    if (parent === dir) break
    dir = parent
  }
  // release: <exe folder>/data (ADR-0134 · the daemon's data_dir::default_data_dir).
  //   A release exe can also be unpacked outside the repo, so both a repo-relative candidate (target/release) and a cwd-relative candidate are kept.
  try {
    const scriptDir = path.dirname(fileURLToPath(import.meta.url)) // <repo>/scripts
    candidates.push(path.join(scriptDir, '..', 'target', 'release', 'data', PORTFILE_IN_DATA_DIR))
  } catch {}
  candidates.push(path.join(process.cwd(), 'data', PORTFILE_IN_DATA_DIR))
  // Picks the first portfile that points at a live daemon — skips dead dev portfiles (removes the ENGRAM_DATA_DIR crutch).
  // Without it a stale dev portfile points at a dead daemon and the connection fails.
  // ★Compute the diagnosis for every candidate★: it used to stop at the first live candidate, but to show
  //   "why this file was chosen and how the other candidates fared" when a later connection fails, the full diagnosis is needed (the connection failure message uses it).
  //   The number of candidates is small (usually 4 or fewer), so computing all of them costs next to nothing.
  const diagnostics = candidates.map((c) => {
    const exists = fs.existsSync(c)
    let alive = false
    if (exists) {
      const info = readPortfile(c, 2)
      if (info) {
        try { process.kill(info.pid, 0); alive = true } // signal 0 = existence check (does not kill)
        catch (e) { alive = e.code === 'EPERM' } // EPERM = exists but no permission = alive
      }
    }
    return { path: c, exists, alive }
  })
  const existing = diagnostics.filter((d) => d.exists)
  const live = existing.find((d) => d.alive)
  // reason = why this portfile was chosen (used to explain to the user when the connection fails).
  if (live) return { path: live.path, reason: 'live-pid-match', candidates: diagnostics }
  if (existing.length) return { path: existing[0].path, reason: 'fallback-no-live-candidate', candidates: diagnostics } // If none is alive, use the first candidate (attempt to connect → a clear error)
  throw new Error('daemon.json not found. 데몬이 떠 있나요? tried:\n  ' + candidates.join('\n  '))
}

function reasonText(found) {
  return found.reason === 'live-pid-match'
    ? '기록된 pid 가 살아있어 선택됨'
    : '살아있는 후보가 없어 첫 존재 후보로 fallback'
}

function formatCandidateLine(c) {
  if (!c.exists) return `  ${c.path}  [missing]`
  return `  ${c.path}  [exists, pid ${c.alive ? 'alive' : 'dead/unreadable'}]`
}

// De-duplication for display only — the selection logic (findPortfile's live/existing[0]) uses the original order as is and
//   this function does not touch it. A script-relative candidate and a .git walk-up candidate can converge on the same file
//   (Windows ignores case), so only the first appearance by normalized absolute path is kept.
function dedupeForDisplay(candidates) {
  const seen = new Set()
  return candidates.filter((c) => {
    const key = path.resolve(c.path).toLowerCase()
    if (seen.has(key)) return false
    seen.add(key)
    return true
  })
}

// Attaches the portfile selection info (found) to the failure message — which portfile was chosen and why, and how the other candidates fared.
function withCandidateDiagnostics(headerLines, found) {
  return [...headerLines, `사용한 portfile: ${found.path} (${reasonText(found)})`, 'candidates:', dedupeForDisplay(found.candidates).map(formatCandidateLine).join('\n')].join('\n')
}

// ★Supplement the cause with raw TCP★: a WHATWG ErrorEvent (.error, undici-based) has an empty message (measured,
//   node v24) so it carries no information — concatenated into a string as is it prints only '[object ErrorEvent]' (the observed symptom).
//   Since the failure is already settled, we open a raw socket once more to the same host:port to get the OS error (code/message, e.g.
//   ECONNREFUSED). The success path never takes this — connect() detaches onerror right after open (below).
// Three return kinds: 'connected' (TCP opened — the failure is at the WS handshake stage) · 'error' (OS error, carried in text) ·
//   'timeout' (nothing came at all). It used to lump success and timeout together as null so the call site could not tell them apart.
function probeTcpReason(host, port, ms = 800) {
  return new Promise((resolve) => {
    let done = false
    let sock
    const finish = (result) => { if (done) return; done = true; sock?.destroy(); resolve(result) }
    try {
      sock = net.createConnection({ host, port })
    } catch (e) {
      // A malformed host (the JSON parsed but the value is odd — e.g. "host:123") makes net.createConnection throw synchronously here instead of an async
      // 'error'. If not caught, the throw of this executor spreads as a promise reject,
      // dies inside describeWsError → the onerror (an async function) of connect(), and the outer auth promise
      // never settles (infinite wait → wrong exit).
      finish({ kind: 'error', text: e.code ? `${e.message} (${e.code})` : e.message })
      return
    }
    sock.once('connect', () => finish({ kind: 'connected' }))
    sock.once('error', (e) => finish({ kind: 'error', text: e.code ? `${e.message} (${e.code})` : e.message }))
    setTimeout(() => finish({ kind: 'timeout' }), ms)
  })
}

async function describeWsError(e, host, port) {
  const err = e?.error
  const msg = err?.message || e?.message
  const code = err?.code
  if (msg) return code ? `${msg} (${code})` : msg
  const probed = await probeTcpReason(host, port)
  if (probed.kind === 'connected') return 'TCP 연결은 됐지만 WebSocket 핸드셰이크가 거부됨(Origin 검사·HTTP 4xx 등 의심 — ErrorEvent 는 그 이상을 안 줌)'
  if (probed.kind === 'timeout') return `TCP 연결도 응답 없음(${host}:${port})`
  return probed.text || String(e)
}

const rid = () => crypto.randomUUID()

// Connect + Auth (first frame) → returns {send, waitFor}. Throws on failure (token mismatch/version).
async function connect() {
  const chosen = findPortfile() // Named chosen so it does not collide with the local found inside waitFor.
  const portfile = chosen.path
  const info = readPortfile(portfile)
  if (!info) {
    throw new Error(withCandidateDiagnostics(
      [`daemon.json 을 온전히 읽지 못했습니다(쓰는 중이거나 다른 프로그램이 붙들고 있음)`],
      chosen,
    ))
  }
  const endpoint = `ws://${info.host}:${info.port}/`
  const ws = new WebSocket(endpoint)
  ws.binaryType = 'arraybuffer'
  const texts = []       // Accumulates received control (JSON Text) messages
  const waiters = []     // {match, resolve}
  ws.onmessage = (ev) => {
    if (typeof ev.data !== 'string') return // binary = output frames (terminal bytes) — the control CLI ignores them
    let msg
    try { msg = JSON.parse(ev.data) } catch { return }
    texts.push(msg)
    for (let i = waiters.length - 1; i >= 0; i--) {
      if (waiters[i].match(msg)) { const w = waiters.splice(i, 1)[0]; w.resolve(msg) }
    }
  }
  await new Promise((res, rej) => {
    ws.onopen = () => res()
    ws.onerror = async (e) => {
      let reason
      try {
        reason = await describeWsError(e, info.host, info.port)
      } catch (probeErr) {
        // Whatever the diagnosis path itself throws (including a synchronous throw), this promise must settle as a reject
        // — otherwise onerror dies inside and connect() never finishes.
        reason = `${String(e)} (원인 진단 실패: ${probeErr.message})`
      }
      rej(new Error(withCandidateDiagnostics(
        [`daemon 연결 실패: ${endpoint}`, `원인: ${reason}`],
        chosen,
      )))
    }
  })
  // ★Detach onerror after open★: otherwise a later socket reset fires this handler again, rejecting in vain on the already
  //   settled promise above and also running probeTcpReason (up to 800ms) again, which only delays the Hello wait.
  ws.onerror = null
  // First frame = Auth. The daemon drops the connection if Auth does not arrive within its AUTH_TIMEOUT (daemon link/server.rs).
  ws.send(JSON.stringify({ Auth: { token: info.token, protocol_version: info.protocol_version } }))
  const waitFor = (match, ms = 5000) => new Promise((res, rej) => {
    const found = texts.find(match)
    if (found) return res(found)
    let t
    const w = { match, resolve: (m) => { clearTimeout(t); res(m) } }
    waiters.push(w)
    t = setTimeout(() => { const i = waiters.indexOf(w); if (i >= 0) waiters.splice(i, 1); rej(new Error('timeout waiting for reply')) }, ms)
  })
  const hello = await waitFor((m) => m.Hello || m.Error)
  if (hello.Error) throw new Error('auth failed: ' + JSON.stringify(hello.Error))
  return { ws, waitFor, send: (obj) => ws.send(JSON.stringify(obj)) }
}

async function listAgents(conn) {
  const request_id = rid()
  conn.send({ ListAgents: { request_id } })
  const reply = await conn.waitFor((m) => m.AgentList && m.AgentList.request_id === request_id)
  return reply.AgentList.agents
}

async function listProfiles(conn) {
  const request_id = rid()
  conn.send({ ListProfiles: { request_id } })
  const reply = await conn.waitFor((m) => m.ProfileList && m.ProfileList.request_id === request_id)
  return reply.ProfileList.profiles
}

// Agent + profile join → a list filled in down to the display name (label) shown in the tree.
// ★display_name exists only in AgentProfile and profile.id == agent.id (immutable after spawn, mergeTreeNodes.ts:4)★ so it joins
//   by id. label = the tree display name (display_name → else profile.name → else AgentInfo.name / the first 8 characters of the id).
async function fetchAgents(conn) {
  const agents = await listAgents(conn)
  const profiles = await listProfiles(conn)
  const pById = new Map(profiles.map((p) => [p.id, p]))
  return agents.map((a) => {
    const p = pById.get(a.id)
    const label = (p && (p.display_name || p.name)) || a.name || a.id.slice(0, 8)
    return { id: a.id, cwd: a.cwd, status: a.status, label }
  })
}

// Points at 1 agent by display name (label) / full id / id prefix. Throws if ambiguous.
function resolveAgent(list, needle) {
  const byId = list.find((a) => a.id === needle)
  if (byId) return byId
  const byLabel = list.filter((a) => (a.label || '').toLowerCase() === needle.toLowerCase())
  if (byLabel.length === 1) return byLabel[0]
  if (byLabel.length > 1) throw new Error(`이름 모호 "${needle}" — ${byLabel.length}명 매칭. id 로 지목하세요.`)
  const byPrefix = list.filter((a) => a.id.startsWith(needle))
  if (byPrefix.length === 1) return byPrefix[0]
  if (byPrefix.length > 1) throw new Error(`id 접두사 모호 "${needle}" — ${byPrefix.length}명.`)
  throw new Error(`agent not found: "${needle}"`)
}

const [op, ...rest] = process.argv.slice(2)

let conn
try {
  conn = await connect()
  if (op === 'list') {
    const agents = await fetchAgents(conn)
    if (!agents.length) console.log('(no agents)')
    // status is an enum object ({Running:...} etc.), so if it is not a string, use JSON.
    const st = (s) => (typeof s === 'string' ? s : JSON.stringify(s))
    for (const a of agents) console.log(`${a.id}\t${a.label}\t${st(a.status)}\t${a.cwd}`)
  } else if (op === 'spawn') {
    const cwd = rest[0]
    if (!cwd) throw new Error('usage: engram spawn <cwd>')
    const request_id = rid()
    conn.send({ SpawnByCwd: { cwd, request_id } })
    const reply = await conn.waitFor((m) => m.Spawned && m.Spawned.request_id === request_id)
    console.log('spawned:', reply.Spawned.agent.id, reply.Spawned.agent.name)
  } else if (op === 'spawn-claude') {
    // ★claude takes 2 steps★: CreateProfile (register) → SpawnProfile (real spawn). SpawnByCwd (= spawn) is an empty shell (cmd.exe),
    //   so an agent-to-agent message demo needs claude. output_format StreamJson = structured rendering (chat UI, ADR-0044).
    const cwd = rest[0]
    if (!cwd) throw new Error('usage: engram spawn-claude <cwd>')
    const rid1 = rid()
    conn.send({ CreateProfile: { name: cwd, cwd, extra_args: [], env: [], auto_restore: false, output_format: 'StreamJson', request_id: rid1 } })
    const created = await conn.waitFor((m) => (m.Created && m.Created.request_id === rid1) || (m.Error && m.Error.request_id === rid1))
    if (created.Error) throw new Error('CreateProfile failed: ' + created.Error.message)
    const profileId = created.Created.profile.id
    const rid2 = rid()
    conn.send({ SpawnProfile: { profile_id: profileId, resume: false, request_id: rid2 } })
    const spawned = await conn.waitFor((m) => (m.Spawned && m.Spawned.request_id === rid2) || (m.Error && m.Error.request_id === rid2))
    if (spawned.Error) throw new Error('SpawnProfile failed: ' + spawned.Error.message)
    console.log('spawned claude:', spawned.Spawned.agent.id)
  } else if (op === 'send') {
    const target = rest[0]
    const text = rest.slice(1).join(' ')
    if (!target || !text) throw new Error('usage: engram send <name|id> <text...>')
    const agent = resolveAgent(await fetchAgents(conn), target)
    // data: serde_bytes → serialized as a JSON number array (uint8 list).
    // ★`\r` = PTY Enter. This one-buffer approach works for shells and JSON agents but is **not submitted to the claude
    //   TUI** (measured 2026-08-17 — if the body and CR arrive in one write, the TUI holds the text in the input box).
    //   ★Even so, do not split it into two frames here★: each frame is wrapped separately by the session encoder, so the JSON
    //   agent gets **one extra empty turn** holding only the CR. The submit decision belongs to the core (the per-backend seam) —
    //   the CLI does not know what the other side is and does not need to. Making the CLI submit a terminal claude is unsupported.
    const data = Array.from(Buffer.from(text + '\r', 'utf8'))
    const request_id = rid()
    conn.send({ WriteStdin: { agent_id: agent.id, data, request_id } })
    // ★Do not say success without an ack★: a timeout or disconnect means "do not know whether it went", not "it went".
    const reply = await conn.waitFor((m) => (m.Ack && m.Ack.request_id === request_id) || (m.Error && m.Error.request_id === request_id), 3000).catch(() => null)
    if (!reply) throw new Error(`WriteStdin: 3s 안에 응답 없음 — 전달 여부 미확인 (${agent.label})`)
    if (reply.Error) throw new Error('WriteStdin failed: ' + (reply.Error.message ?? JSON.stringify(reply.Error)))
    console.log('sent →', agent.label, `(${agent.id})`)
  } else if (op === 'reparent') {
    // Note: ReparentProfile targets a profile id (not an agent id). It is a spike so it accepts only a raw id.
    const child_id = rest[0]
    const parentArg = rest[1]
    if (!child_id) throw new Error('usage: engram reparent <childProfileId> <parentProfileId|null>')
    const parent_id = (!parentArg || parentArg === 'null') ? null : parentArg
    const request_id = rid()
    conn.send({ ReparentProfile: { child_id, parent_id, request_id } })
    await conn.waitFor((m) => (m.Ack && m.Ack.request_id === request_id) || (m.Error && m.Error.request_id === request_id), 3000).catch(() => {})
    console.log('reparent sent:', child_id, '→', parent_id)
  } else if (op === 'raw') {
    // Escape hatch: send an arbitrary AgentCommand JSON and print the control frames that arrive for 3s.
    const json = rest.join(' ')
    if (!json) throw new Error('usage: engram raw \'{"VariantName":{...}}\'')
    conn.send(JSON.parse(json))
    await new Promise((r) => setTimeout(r, 3000))
  } else {
    console.log('engram <list | spawn <cwd> | spawn-claude <cwd> | send <name|id> <text...> | reparent <childProfileId> <parentProfileId|null> | raw <json>>')
  }
  process.exit(0)
} catch (e) {
  console.error('error:', e.message)
  process.exit(1)
} finally {
  conn?.ws?.close()
}
