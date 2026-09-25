// Phase 0 spike harness — codex app-server (stdio JSON-RPC), mirrors engram CodexBackend app-server handshake.
// usage: node codex_harness.js <scenario> <outDir> <cwd> [model] [effort]
'use strict';
const { spawn, execFileSync } = require('child_process');
const fs = require('fs'), path = require('path'), crypto = require('crypto');
const { performance } = require('perf_hooks');

const scenario = process.argv[2];
const outDir = process.argv[3];
const cwd = process.argv[4];
const model = process.argv[5] || 'gpt-6-luna';
const effort = process.argv[6] || 'low';
fs.mkdirSync(outDir, { recursive: true });
fs.mkdirSync(cwd, { recursive: true });
const stamp = new Date().toISOString().replace(/[:.]/g, '-');
const logPath = path.join(outDir, `codex-${scenario}-${stamp}.jsonl`);
const logFd = fs.openSync(logPath, 'w');
const T0 = performance.now();
const now = () => Math.round((performance.now() - T0) * 10) / 10;
function rec(o) { fs.writeSync(logFd, JSON.stringify({ t: now(), ...o }) + '\n'); }
const uuid = () => crypto.randomUUID();
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// argv = ours (`codex app-server --stdio`) + extra_args (-c overrides: cheap model, low effort, no notify side effect)
const args = ['app-server', '--stdio', '-c', `model=${model}`, '-c', `model_reasoning_effort=${effort}`, '-c', 'notify=[]'];
rec({ dir: 'meta', argv: ['cmd.exe', '/c', 'codex', ...args], cwd });
const child = spawn('cmd.exe', ['/c', 'codex', ...args], { cwd, env: process.env, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
rec({ dir: 'meta', pid: child.pid });
console.log('CHILD_PID=' + child.pid + ' LOG=' + logPath);

const events = [];
const waiters = [];
const pendingReq = new Map(); // id -> {method, resolve}
let nextId = 0, outBuf = '', lastEventT = 0, exited = false, chunkNo = 0;

function send(obj, note) { rec({ dir: 'in', note, line: obj }); child.stdin.write(JSON.stringify(obj) + '\n'); }
function request(method, params, note) {
  const id = nextId++;
  const p = new Promise((resolve) => pendingReq.set(id, { method, resolve }));
  send({ id, method, params }, note);
  return p;
}
child.stdout.on('data', (d) => {
  chunkNo++;
  outBuf += d.toString('utf8');
  let i;
  while ((i = outBuf.indexOf('\n')) >= 0) {
    const line = outBuf.slice(0, i).replace(/\r$/, '');
    outBuf = outBuf.slice(i + 1);
    if (!line.trim()) continue;
    let o = null;
    try { o = JSON.parse(line); } catch (_) { }
    const t = now();
    lastEventT = t;
    // do not log agentMessage deltas in full (noise) — keep a compact form
    if (o && o.method === 'item/agentMessage/delta') rec({ dir: 'out', chunk: chunkNo, line: { method: o.method, params: { itemId: o.params?.itemId, delta: o.params?.delta } } });
    else rec({ dir: 'out', chunk: chunkNo, line: o ?? line });
    if (!o) continue;
    if (o.id !== undefined && o.method !== undefined) {
      // server -> client request: refuse like engram Reader::refuse (-32601)
      send({ id: o.id, error: { code: -32601, message: `engram-dashboard 는 \`${o.method}\` 를 처리하지 않는다` } }, 'refuse server request');
    } else if (o.id !== undefined && (o.result !== undefined || o.error !== undefined)) {
      const p = pendingReq.get(o.id);
      if (p) { pendingReq.delete(o.id); p.resolve({ t, o }); }
    }
    const ev = { t, chunk: chunkNo, o, idx: events.length };
    events.push(ev);
    for (const w of [...waiters]) {
      if (ev.idx >= w.from && w.pred(o, ev)) { waiters.splice(waiters.indexOf(w), 1); w.resolve(ev); }
    }
  }
});
child.stderr.on('data', (d) => rec({ dir: 'err', text: d.toString('utf8').slice(0, 2000) }));
child.on('exit', (code, sig) => { exited = true; rec({ dir: 'meta', exit: code, sig }); });

function waitFor(pred, timeoutMs, from = events.length) {
  for (const ev of events) if (ev.idx >= from && pred(ev.o, ev)) return Promise.resolve(ev);
  return new Promise((resolve) => {
    const w = { pred, from, resolve };
    waiters.push(w);
    setTimeout(() => { const k = waiters.indexOf(w); if (k >= 0) { waiters.splice(k, 1); rec({ dir: 'meta', timeout: String(pred).slice(0, 120) }); resolve(null); } }, timeoutMs);
  });
}
// run cb synchronously inside the stdout handler that delivers the matching event
function onceSync(pred, cb, timeoutMs, from = events.length) {
  return new Promise((resolve) => {
    const w = { pred, from, resolve: (ev) => { cb(ev); resolve(ev); } };
    waiters.push(w);
    setTimeout(() => { const k = waiters.indexOf(w); if (k >= 0) { waiters.splice(k, 1); rec({ dir: 'meta', timeout: 'onceSync ' + String(pred).slice(0, 100) }); resolve(null); } }, timeoutMs);
  });
}
async function settle(quietMs, maxMs) {
  const start = now();
  while (now() - start < maxMs) { await sleep(250); if (now() - lastEventT >= quietMs) return; }
}
const isN = (m) => (o) => o.method === m;
const itemType = (o) => o.params?.item?.type;
const isCmdStarted = (o) => o.method === 'item/started' && itemType(o) === 'commandExecution';
const isCmdCompleted = (o) => o.method === 'item/completed' && itemType(o) === 'commandExecution';
const isTurnCompleted = isN('turn/completed');
const txt = (text) => [{ type: 'text', text }];

let threadId = null;
async function handshake() {
  const init = await request('initialize', { clientInfo: { name: 'engram-dashboard', version: '0.1.0' } }, 'initialize');
  rec({ dir: 'meta', initializeResult: init.o.result ?? init.o.error });
  console.log('USER_AGENT=' + (init.o.result?.userAgent ?? JSON.stringify(init.o.error)));
  send({ method: 'initialized' }, 'initialized');
  const st = await request('thread/start', { cwd, approvalPolicy: 'on-request', sandbox: 'workspace-write' }, 'thread/start');
  threadId = st.o.result?.thread?.id;
  rec({ dir: 'meta', threadId, cliVersion: st.o.result?.thread?.cliVersion });
  if (!threadId) throw new Error('thread/start failed ' + JSON.stringify(st.o));
}
async function startTurn(text, clientUserMessageId, note) {
  const r = await request('turn/start', { threadId, clientUserMessageId, input: txt(text) }, note);
  const turnId = r.o.result?.turn?.id;
  rec({ dir: 'meta', note: 'turn started', turnId, cumid: clientUserMessageId });
  return turnId;
}
function steer(turnId, text, id, note) {
  return request('turn/steer', { threadId, clientUserMessageId: id, input: txt(text), expectedTurnId: turnId }, note);
}
async function itemsList(note) {
  const r = await request('thread/items/list', { threadId, limit: 40, sortDirection: 'desc' }, note);
  const data = r.o.result?.data ?? [];
  const users = data.filter((e) => e.item?.type === 'userMessage').map((e) => ({ turnId: e.turnId, id: e.item.id, clientId: e.item.clientId ?? null, text: (e.item.content?.[0]?.text ?? '').slice(0, 60) }));
  rec({ dir: 'meta', note: 'items/list userMessages ' + note, users, error: r.o.error ?? null });
  return users;
}

const S = {};
S.M6 = async () => {
  await handshake();
  // T1: two steers during a running commandExecution
  let from = events.length;
  const U1 = uuid(), X = uuid(), Y = uuid();
  rec({ dir: 'meta', trial: 'T1', ids: { U1, X, Y } });
  const t1 = await startTurn('Run exactly this shell command: Start-Sleep -Seconds 20\nAfter it finishes, reply with exactly: DONE-1', U1, 'T1 turn/start');
  const c1 = await waitFor(isCmdStarted, 120000, from);
  if (c1) {
    await sleep(2000);
    const sx = steer(t1, 'Also include the word MANGO in your final reply.', X, 'T1 steer X');
    await sleep(1000);
    const sy = steer(t1, 'Also include the word LEMON in your final reply.', Y, 'T1 steer Y');
    rec({ dir: 'meta', steerX: (await sx).o, steerY: (await sy).o });
  }
  // steer the moment turn/completed arrives (turn just ended) -> expect -32600
  let lateSteer = null;
  await onceSync(isTurnCompleted, () => { lateSteer = steer(t1, 'late steer after completion', uuid(), 'T1 steer at turn/completed (sync)'); }, 180000, from);
  if (lateSteer) rec({ dir: 'meta', lateSteerResp: (await lateSteer).o });
  await settle(3000, 20000);
  await itemsList('after T1');

  // T2: steer then interrupt
  from = events.length;
  const U2 = uuid(), Z = uuid();
  rec({ dir: 'meta', trial: 'T2', ids: { U2, Z } });
  const t2 = await startTurn('Run exactly this shell command: Start-Sleep -Seconds 20\nAfter it finishes, reply with exactly: DONE-2', U2, 'T2 turn/start');
  const c2 = await waitFor(isCmdStarted, 120000, from);
  if (c2) {
    await sleep(2000);
    const sz = steer(t2, 'Also include the word PAPAYA in your final reply.', Z, 'T2 steer Z');
    rec({ dir: 'meta', steerZ: (await sz).o });
    await sleep(1500);
    const ir = await request('turn/interrupt', { threadId, turnId: t2 }, 'T2 interrupt');
    rec({ dir: 'meta', interruptResp: ir.o });
  }
  await waitFor(isTurnCompleted, 60000, from);
  await settle(4000, 20000);
  await itemsList('after T2');

  // T3: end-of-turn race — steer synchronously on agentMessage item/completed (no-tool turn)
  for (const [name, trig] of [['T3', (o) => o.method === 'item/completed' && itemType(o) === 'agentMessage'], ['T4', isN('thread/tokenUsage/updated')]]) {
    from = events.length;
    const U = uuid(), W = uuid();
    rec({ dir: 'meta', trial: name, ids: { U, W } });
    const tt = await startTurn('Without using any tools, write exactly two short sentences about the moon.', U, name + ' turn/start');
    let sp = null;
    await onceSync(trig, () => { sp = steer(tt, 'Also add a third sentence that contains the word COMET.', W, name + ' steer at end-of-sampling (sync)'); }, 120000, from);
    if (sp) rec({ dir: 'meta', steerResp: (await sp).o });
    await waitFor(isTurnCompleted, 120000, from);
    await settle(4000, 30000);
  }
  await itemsList('final');
};

S.M7 = async () => {
  await handshake();
  for (let k = 1; k <= 10; k++) {
    const from = events.length;
    const U = uuid(), B = uuid();
    rec({ dir: 'meta', trial: k, ids: { U, B } });
    const tk = await startTurn(`Run exactly this shell command: Start-Sleep -Seconds 2\nAfter it finishes, reply with exactly: OK-${k}`, U, 'turn/start ' + k);
    let sp = null;
    await onceSync(isCmdCompleted, (ev) => { sp = steer(tk, `Also reply with the word KIWI${k}.`, B, `steer on commandExecution completed ${k} (seen at ${ev.t})`); }, 120000, from);
    if (sp) rec({ dir: 'meta', trial: k, steerResp: (await sp).o });
    await waitFor(isTurnCompleted, 180000, from);
    await settle(3000, 30000);
  }
};

function killTree() {
  try { execFileSync('taskkill', ['/PID', String(child.pid), '/T', '/F'], { stdio: 'ignore' }); rec({ dir: 'meta', killed: child.pid }); } catch (e) { rec({ dir: 'meta', killErr: String(e.message).slice(0, 200) }); }
}
(async () => {
  const hard = setTimeout(() => { rec({ dir: 'meta', hardTimeout: true }); killTree(); process.exit(3); }, 20 * 60 * 1000);
  try {
    await (S[scenario] || (async () => { throw new Error('unknown scenario ' + scenario); }))();
  } catch (e) { rec({ dir: 'meta', error: String(e.stack || e) }); }
  child.stdin.end();
  rec({ dir: 'meta', stdinClosed: true });
  for (let i = 0; i < 40 && !exited; i++) await sleep(250);
  if (!exited) killTree();
  await sleep(500);
  clearTimeout(hard);
  console.log('EVENTS=' + events.length + ' LOG=' + logPath);
  fs.closeSync(logFd);
  process.exit(0);
})();
