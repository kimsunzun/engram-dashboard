#!/usr/bin/env node
// scripts/engram.mjs — THROWAWAY 스파이크 (ADR-0014 방향 · CLI-via-Bash 실현체, 롤백 예정).
// 릴리즈-safe LLM 제어 통로 PoC: daemon.json portfile → 데몬 WS → AgentCommand JSON.
//   - dev CDP/window.__TAURI__ 는 release exe 에서 죽지만, 데몬 WS 는 빌드 무관으로 산다.
//   - 스폰된 Claude(Bash 보유)가 이 CLI 를 호출 = 앱 조종. 정식 채택 시 PRD/ADR 후 정리.
// 의존성 0: node 18+ 내장 WebSocket 만 사용(cdp.mjs 패턴 미러).
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import net from 'node:net'
import { fileURLToPath } from 'node:url'

// daemon.json 위치 해결 — dev(repo <root>/.engram-data) 와 release(<exe 폴더>/data) 둘 다 커버.
// 이 후보 탐색이 "release-safe" 의 핵심: 어느 빌드든 데몬이 떠 있으면 portfile 로 붙는다.
//
// ★ENGRAM_DATA_DIR 로 릴리스 데몬을 가리키게 하지 말 것★: ADR-0134 이후 그 변수는 단일 인스턴스
//   스코프이기도 해서, 설정한 채로 데몬이 뜨면 다른 폴더를 잡아 조용히 두 번째 데몬이 된다. 이 함수가
//   알아서 찾는다.
// ★찢긴 읽기를 견뎌야 한다(ADR-0135)★: 데몬은 daemon.json 을 붙잡은 채 **제자리에** 쓴다(임시 파일 +
//   rename 이 불가능하다 — 우리가 삭제 공유를 닫고 잡고 있다). 그래서 길이 0으로 줄인 직후나 쓰는
//   도중에 읽으면 빈 파일·반쪽 JSON 이 보인다. 그건 손상이 아니라 **아직 준비 안 됨**이므로 짧게 다시
//   읽는다. ★JSON.parse 를 맨몸으로 부르지 마라★ — 데몬이 멀쩡한데 SyntaxError 로 죽는다.
//   열기 실패(제3자가 좁은 공유로 잠깐 여는 경우)도 같은 취급이다.
const PORTFILE_READ_ATTEMPTS = 10   // 총 대기 상한 ≈ 500ms — 데몬 발행은 ms 단위라 넉넉하다.
const PORTFILE_READ_DELAY_MS = 50   // discovery 폴링 주기와 같은 값.

function sleepSync(ms) { Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms) }

// 성공하면 파싱된 레코드, 예산 안에 온전한 내용을 못 보면 null(던지지 않는다).
// ★후보 추리기(findPortfile 의 생존 진단)는 예산을 짧게 준다★: 후보마다 전체 예산을 쓰면 죽은 후보
//   몇 개만으로 CLI 가 몇 초씩 멈춘다. 붙을 파일을 확정한 뒤(connect)에만 넉넉히 기다린다.
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
  // release: <exe 폴더>/data/daemon.json (ADR-0134 · discovery::default_data_dir).
  //   릴리스 exe 는 repo 밖에도 풀릴 수 있으므로 repo 기준 후보(target/release)와 cwd 기준 후보를 함께 둔다.
  try {
    const scriptDir = path.dirname(fileURLToPath(import.meta.url)) // <repo>/scripts
    candidates.push(path.join(scriptDir, '..', 'target', 'release', 'data', 'daemon.json'))
  } catch {}
  candidates.push(path.join(process.cwd(), 'data', 'daemon.json'))
  // 살아있는 데몬을 가리키는 첫 portfile 선택 — 죽은 dev portfile 을 건너뛴다(ENGRAM_DATA_DIR 목발 제거).
  // 이게 없으면 스테일 .engram-data/daemon.json 이 죽은 데몬을 가리켜 연결 실패한다.
  // ★후보별 진단을 전부 계산해 둔다★: 예전엔 첫 살아있는 후보에서 멈췄지만, 그 뒤 연결이 실패했을 때
  //   "왜 이 파일을 골랐고 다른 후보는 어땠나"를 보여주려면 전체 진단이 필요하다(연결 실패 메시지가 씀).
  //   후보 수가 적어(보통 4개 이하) 전부 계산해도 비용은 무시할 만하다.
  const diagnostics = candidates.map((c) => {
    const exists = fs.existsSync(c)
    let alive = false
    if (exists) {
      const info = readPortfile(c, 2)
      if (info) {
        try { process.kill(info.pid, 0); alive = true } // 신호 0 = 존재 확인(안 죽임)
        catch (e) { alive = e.code === 'EPERM' } // EPERM = 존재하나 권한없음 = 살아있음
      }
    }
    return { path: c, exists, alive }
  })
  const existing = diagnostics.filter((d) => d.exists)
  const live = existing.find((d) => d.alive)
  // reason = 이 portfile 을 왜 골랐나(연결 실패 시 사용자에게 설명하는 용도).
  if (live) return { path: live.path, reason: 'live-pid-match', candidates: diagnostics }
  if (existing.length) return { path: existing[0].path, reason: 'fallback-no-live-candidate', candidates: diagnostics } // 살아있는 게 없으면 첫 후보로(연결 시도 → 명확한 에러)
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

// 화면 표시 전용 중복 제거 — 선택 로직(findPortfile 의 live/existing[0])은 원본 순서를 그대로 쓰고
//   이 함수는 안 건드린다. 스크립트-상대 후보와 .git 워크업 후보가 같은 파일로 수렴하는 경우가 있어
//   (Windows 는 대소문자 무시) 정규화한 절대경로 기준 첫 등장만 남긴다.
function dedupeForDisplay(candidates) {
  const seen = new Set()
  return candidates.filter((c) => {
    const key = path.resolve(c.path).toLowerCase()
    if (seen.has(key)) return false
    seen.add(key)
    return true
  })
}

// portfile 선택 정보(found)를 실패 메시지에 붙인다 — 어느 portfile 을 왜 골랐고 다른 후보는 어땠는지.
function withCandidateDiagnostics(headerLines, found) {
  return [...headerLines, `사용한 portfile: ${found.path} (${reasonText(found)})`, 'candidates:', dedupeForDisplay(found.candidates).map(formatCandidateLine).join('\n')].join('\n')
}

// ★raw TCP 로 원인을 보충한다★: WHATWG ErrorEvent(.error, undici 기반) 는 message 가 비어 있어(실측,
//   node v24) 아무 정보가 없다 — 그대로 문자열 결합하면 '[object ErrorEvent]' 로만 찍힌다(실측 증상).
//   이미 실패가 확정된 뒤라, 같은 host:port 로 raw socket 을 한 번 더 열어 OS 에러(code/message, 예:
//   ECONNREFUSED)를 얻는다. 성공 경로는 절대 안 탄다 — connect() 가 open 직후 onerror 를 떼어낸다(아래).
// 반환 kind 셋: 'connected'(TCP 는 열렸다 — 실패는 WS 핸드셰이크 단계) · 'error'(OS 에러, text 에 담김) ·
//   'timeout'(그 무엇도 안 왔다). 예전엔 성공·타임아웃이 둘 다 null 로 뭉개져 호출부가 못 갈랐다.
function probeTcpReason(host, port, ms = 800) {
  return new Promise((resolve) => {
    let done = false
    let sock
    const finish = (result) => { if (done) return; done = true; sock?.destroy(); resolve(result) }
    try {
      sock = net.createConnection({ host, port })
    } catch (e) {
      // 기형 host(JSON 은 파싱됐지만 값이 이상 — 예: "host:123")는 net.createConnection 이 비동기
      // 'error' 대신 여기서 동기로 던진다. 못 잡으면 이 executor 의 throw 가 그대로 promise reject 로
      // 번져 describeWsError → connect() 의 onerror(비동기 함수) 안에서 죽고, 바깥 auth 프라미스가
      // 영영 안 정착한다(무한 대기 → 잘못된 종료).
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

// 연결 + Auth(첫 프레임) → {send, waitFor} 반환. 실패(토큰 불일치/버전) 시 throw.
async function connect() {
  const chosen = findPortfile() // 이름을 waitFor 내부의 지역 found 와 겹치지 않게 chosen 으로.
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
    ws.onerror = async (e) => {
      let reason
      try {
        reason = await describeWsError(e, info.host, info.port)
      } catch (probeErr) {
        // 진단 경로 자체가 무엇을 던지든(동기 throw 포함) 이 프라미스는 반드시 reject 로 정착해야
        // 한다 — 안 그러면 onerror 가 안에서 죽어 connect() 가 영영 안 끝난다.
        reason = `${String(e)} (원인 진단 실패: ${probeErr.message})`
      }
      rej(new Error(withCandidateDiagnostics(
        [`daemon 연결 실패: ${endpoint}`, `원인: ${reason}`],
        chosen,
      )))
    }
  })
  // ★open 이후 onerror 를 떼어낸다★: 안 그러면 나중 소켓 리셋이 이 핸들러를 다시 태워 이미 정착된 위
  //   프라미스에 헛rej 하면서 probeTcpReason(최대 800ms)까지 또 돌아 Hello 대기만 지연시킨다.
  ws.onerror = null
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
    // data: serde_bytes → JSON 숫자배열(uint8 list)로 직렬화.
    // ★`\r` = PTY Enter. 이 한 버퍼 방식은 셸·JSON 에이전트에는 통하지만 **claude TUI 에는 제출되지
    //   않는다**(실측 2026-08-17 — TUI 는 본문과 CR 이 한 write 로 오면 텍스트를 입력창에 담아 둔다).
    //   ★그렇다고 여기서 두 프레임으로 쪼개지 마라★: 프레임마다 세션 인코더가 따로 감싸므로 JSON
    //   에이전트에게 CR 만 든 **빈 턴이 하나 더** 생긴다. 제출 판정은 코어(백엔드별 seam)가 갖는다 —
    //   CLI 는 상대가 무엇인지 모르고, 알 필요도 없다. CLI 로 터미널 claude 를 제출까지 시키는 건 미지원.
    const data = Array.from(Buffer.from(text + '\r', 'utf8'))
    const request_id = rid()
    conn.send({ WriteStdin: { agent_id: agent.id, data, request_id } })
    // ★ack 없이 성공이라 말하지 않는다★: 타임아웃·연결 끊김은 "갔는지 모른다" 이지 "갔다" 가 아니다.
    const reply = await conn.waitFor((m) => (m.Ack && m.Ack.request_id === request_id) || (m.Error && m.Error.request_id === request_id), 3000).catch(() => null)
    if (!reply) throw new Error(`WriteStdin: 3s 안에 응답 없음 — 전달 여부 미확인 (${agent.label})`)
    if (reply.Error) throw new Error('WriteStdin failed: ' + (reply.Error.message ?? JSON.stringify(reply.Error)))
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
