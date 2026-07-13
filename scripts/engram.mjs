#!/usr/bin/env node
// scripts/engram.mjs — THROWAWAY 스파이크 (ADR-0014 방향 · CLI-via-Bash 실현체, 롤백 예정).
// 릴리즈-safe LLM 제어 통로 PoC: daemon.json portfile → 데몬 WS → AgentCommand JSON.
//   - dev CDP/window.__TAURI__ 는 release exe 에서 죽지만, 데몬 WS 는 빌드 무관으로 산다.
//   - 스폰된 Claude(Bash 보유)가 이 CLI 를 호출 = 앱 조종. 정식 채택 시 PRD/ADR 후 정리.
// 의존성 0: node 18+ 내장 WebSocket 만 사용(cdp.mjs 패턴 미러).
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import { fileURLToPath } from 'node:url'

// daemon.json 위치 해결 — dev(repo <root>/.engram-data) 와 release(%APPDATA%) 둘 다 커버.
// 이 후보 탐색이 "release-safe" 의 핵심: 어느 빌드든 데몬이 떠 있으면 portfile 로 붙는다.
function findPortfile() {
  const candidates = []
  if (process.env.ENGRAM_DATA_DIR) candidates.push(path.join(process.env.ENGRAM_DATA_DIR, 'daemon.json'))
  // ★스크립트 자기 위치 기준 dev 데몬★: 이 파일은 <repo>/scripts/engram.mjs 라 ../.engram-data 가 repo 의 dev
  //   portfile 이다. 호출 cwd 와 무관하게 발견된다(에이전트가 딴 cwd 에서 불러도 OK — 메타테스트에서 노출된 갭 수정).
  try {
    const scriptDir = path.dirname(fileURLToPath(import.meta.url)) // <repo>/scripts
    candidates.push(path.join(scriptDir, '..', '.engram-data', 'daemon.json'))
  } catch {}
  // dev(추가 방어): .git 있는 repo 루트까지 걸어 올라가 <root>/.engram-data/daemon.json
  let dir = process.cwd()
  for (let i = 0; i < 8; i++) {
    if (fs.existsSync(path.join(dir, '.git'))) { candidates.push(path.join(dir, '.engram-data', 'daemon.json')); break }
    const parent = path.dirname(dir)
    if (parent === dir) break
    dir = parent
  }
  // release: %APPDATA%\com.engram.dashboard\daemon.json (discovery::default_data_dir)
  if (process.env.APPDATA) candidates.push(path.join(process.env.APPDATA, 'com.engram.dashboard', 'daemon.json'))
  // 살아있는 데몬을 가리키는 첫 portfile 선택 — 죽은 dev portfile 을 건너뛴다(ENGRAM_DATA_DIR 목발 제거).
  // 이게 없으면 스테일 .engram-data/daemon.json 이 죽은 데몬을 가리켜 연결 실패한다.
  const existing = candidates.filter((c) => fs.existsSync(c))
  const isLive = (c) => {
    try {
      const info = JSON.parse(fs.readFileSync(c, 'utf8'))
      try { process.kill(info.pid, 0); return true } // 신호 0 = 존재 확인(안 죽임)
      catch (e) { return e.code === 'EPERM' } // EPERM = 존재하나 권한없음 = 살아있음
    } catch { return false }
  }
  const live = existing.find(isLive)
  if (live) return live
  if (existing.length) return existing[0] // 살아있는 게 없으면 첫 후보로(연결 시도 → 명확한 에러)
  throw new Error('daemon.json not found. 데몬이 떠 있나요? tried:\n  ' + candidates.join('\n  '))
}

const rid = () => crypto.randomUUID()

// 연결 + Auth(첫 프레임) → {send, waitFor} 반환. 실패(토큰 불일치/버전) 시 throw.
async function connect() {
  const info = JSON.parse(fs.readFileSync(findPortfile(), 'utf8'))
  const ws = new WebSocket(`ws://${info.host}:${info.port}/`)
  ws.binaryType = 'arraybuffer'
  const texts = []       // 수신한 제어(JSON Text) 메시지 누적
  const waiters = []     // {match, resolve}
  ws.onmessage = (ev) => {
    if (typeof ev.data !== 'string') return // binary = 출력 프레임(터미널 바이트) — 제어 CLI 는 무시
    let msg
    try { msg = JSON.parse(ev.data) } catch { return }
    texts.push(msg)
    for (let i = waiters.length - 1; i >= 0; i--) {
      if (waiters[i].match(msg)) { const w = waiters.splice(i, 1)[0]; w.resolve(msg) }
    }
  }
  await new Promise((res, rej) => {
    ws.onopen = () => res()
    ws.onerror = (e) => rej(new Error('ws connect error: ' + (e?.message || e)))
  })
  // 첫 프레임 = Auth. 데몬은 1s 안에 Auth 안 오면 끊는다.
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

// 에이전트 + 프로필 조인 → 트리에 보이는 표시명(label)까지 채운 목록.
// ★display_name 은 AgentProfile 에만 있고 profile.id == agent.id(spawn 후 불변, mergeTreeNodes.ts:4)★ 라 id 로
//   조인한다. label = 트리 표시명(display_name → 없으면 profile.name → 없으면 AgentInfo.name/ id 앞 8자).
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

// 표시명(label) / 전체 id / id 접두사로 에이전트 1명 지목. 모호하면 throw.
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
    // status 는 enum 객체({Running:...} 등)라 문자열이 아니면 JSON 으로.
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
    // ★claude 는 2단계★: CreateProfile(등록) → SpawnProfile(실스폰). SpawnByCwd(=spawn)는 깡통 셸(cmd.exe)이라
    //   에이전트 간 메시지 데모엔 claude 여야 한다. output_format StreamJson = 구조화 렌더(채팅 UI, ADR-0044).
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
    // \r = PTY Enter(제출) — 이게 있어야 상대 에이전트가 입력을 실제로 받는다.
    // data: serde_bytes → JSON 숫자배열(uint8 list)로 직렬화.
    const data = Array.from(Buffer.from(text + '\r', 'utf8'))
    const request_id = rid()
    conn.send({ WriteStdin: { agent_id: agent.id, data, request_id } })
    // Ack 오면 즉시, 안 오면 3s 후 종료(WriteStdin ack 여부 미확정 — 걸어놓고 넘어감).
    await conn.waitFor((m) => (m.Ack && m.Ack.request_id === request_id) || (m.Error && m.Error.request_id === request_id), 3000).catch(() => {})
    console.log('sent →', agent.label, `(${agent.id})`)
  } else if (op === 'reparent') {
    // 주의: ReparentProfile 은 profile id 대상(agent id 아님). 스파이크라 raw id 만 받는다.
    const child_id = rest[0]
    const parentArg = rest[1]
    if (!child_id) throw new Error('usage: engram reparent <childProfileId> <parentProfileId|null>')
    const parent_id = (!parentArg || parentArg === 'null') ? null : parentArg
    const request_id = rid()
    conn.send({ ReparentProfile: { child_id, parent_id, request_id } })
    await conn.waitFor((m) => (m.Ack && m.Ack.request_id === request_id) || (m.Error && m.Error.request_id === request_id), 3000).catch(() => {})
    console.log('reparent sent:', child_id, '→', parent_id)
  } else if (op === 'raw') {
    // 탈출구: 임의 AgentCommand JSON 전송 후 3s 동안 오는 제어 프레임 출력.
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
