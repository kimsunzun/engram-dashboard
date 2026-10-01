// B5 capture driver (S21 chat-ux · TRD §4-7 ⑨) — real `codex app-server --stdio`, same handshake/refusal as
// `../20260925-midturn-phase0/codex_harness.js` (approvalPolicy on-request · sandbox workspace-write · every server
// request refused with -32601 like engram `Reader::refuse`).
// usage: node b5codex.cjs <scenario> <outDir> <cwd> [outsideDir]
//   A = one turn, shell command that exits nonzero           (-> fixtures/tool_fail_u2.jsonl)
//   B = sleep-then-fail command, turn/interrupt mid-run, wait for the late item/completed (record only)
//   C = our refusal: (a) shell write outside cwd  (b) edit-tool write outside cwd (-> fixtures/refuse_u2a.jsonl)
//   R = resume B5_THREAD in a new app-server, read the history page (thread/items/list from itemsBackwardsCursor)
// env: B5_MODEL (default gpt-6-luna) · B5_EFFORT (default low) · B5_THREAD (scenario R) · B5_CFG (extra -c, ';'-separated)
// ★TMPDIR is removed from the child env★ — Git Bash exports it and codex adds $TMPDIR to the workspace-write
// writable roots, which would make the scratchpad "outside" dir writable (no approval request). A daemon spawn
// (WMI) does not carry it.
'use strict';
const { spawn, execFileSync } = require('child_process');
const fs = require('fs'), path = require('path'), crypto = require('crypto');
const { performance } = require('perf_hooks');

const scenario = process.argv[2];
const outDir = process.argv[3];
const cwd = process.argv[4];
const outside = process.argv[5] || null;
const model = process.env.B5_MODEL || 'gpt-6-luna';
const effort = process.env.B5_EFFORT || 'low';
fs.mkdirSync(outDir, { recursive: true });
fs.mkdirSync(cwd, { recursive: true });
const stamp = new Date().toISOString().replace(/[:.]/g, '-');
const logPath = path.join(outDir, `codex-B5${scenario}-${stamp}.jsonl`);
const logFd = fs.openSync(logPath, 'w');
const T0 = performance.now();
const now = () => Math.round((performance.now() - T0) * 10) / 10;
function rec(o) { fs.writeSync(logFd, JSON.stringify({ t: now(), ...o }) + '\n'); }
const uuid = () => crypto.randomUUID();
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const args = ['app-server', '--stdio', '-c', `model=${model}`, '-c', `model_reasoning_effort=${effort}`, '-c', 'notify=[]'];
// B5_CFG = extra `-c` overrides separated by ';' (capture C: `sandbox_workspace_write.exclude_tmpdir_env_var=true`,
// because the scratchpad lives under %TEMP% and codex otherwise treats that as a writable root on Windows —
// the first C run auto-approved the out-of-cwd patch there without any approval request)
for (const c of (process.env.B5_CFG || '').split(';').map((x) => x.trim()).filter(Boolean)) args.push('-c', c);
const env = { ...process.env };
delete env.TMPDIR;
rec({ dir: 'meta', argv: ['cmd.exe', '/c', 'codex', ...args], cwd, outside, tmpdirRemoved: true });
const child = spawn('cmd.exe', ['/c', 'codex', ...args], { cwd, env, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
rec({ dir: 'meta', pid: child.pid });
console.log('CHILD_PID=' + child.pid + ' LOG=' + logPath);

const events = [];
const waiters = [];
const pendingReq = new Map();
const serverReqs = []; // server -> client requests we refused
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
    // same compact delta form as codex_harness.js (the existing fixtures carry it)
    if (o && o.method === 'item/agentMessage/delta') rec({ dir: 'out', chunk: chunkNo, line: { method: o.method, params: { itemId: o.params?.itemId, delta: o.params?.delta } } });
    else rec({ dir: 'out', chunk: chunkNo, line: o ?? line });
    if (!o) continue;
    if (o.id !== undefined && o.method !== undefined) {
      serverReqs.push({ t, method: o.method, id: o.id, params: o.params });
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
async function settle(quietMs, maxMs) {
  const start = now();
  while (now() - start < maxMs) { await sleep(250); if (now() - lastEventT >= quietMs) return; }
}
const itemType = (o) => o.params?.item?.type;
const isCmdStarted = (o) => o.method === 'item/started' && itemType(o) === 'commandExecution';
const turnDone = (tid) => (o) => o.method === 'turn/completed' && (!tid || o.params?.turn?.id === tid);
const txt = (text) => [{ type: 'text', text }];

let threadId = null;
async function initialize() {
  const init = await request('initialize', { clientInfo: { name: 'engram-dashboard', version: '0.1.0' } }, 'initialize');
  rec({ dir: 'meta', initializeResult: init.o.result ?? init.o.error });
  console.log('USER_AGENT=' + (init.o.result?.userAgent ?? JSON.stringify(init.o.error)));
  send({ method: 'initialized' }, 'initialized');
}
async function handshake() {
  await initialize();
  const st = await request('thread/start', { cwd, approvalPolicy: 'on-request', sandbox: 'workspace-write' }, 'thread/start');
  threadId = st.o.result?.thread?.id;
  rec({ dir: 'meta', threadId, cliVersion: st.o.result?.thread?.cliVersion });
  console.log('THREAD=' + threadId + ' CLI=' + st.o.result?.thread?.cliVersion);
  if (!threadId) throw new Error('thread/start failed ' + JSON.stringify(st.o));
}
async function startTurn(text, note) {
  const r = await request('turn/start', { threadId, clientUserMessageId: uuid(), input: txt(text) }, note);
  const turnId = r.o.result?.turn?.id;
  rec({ dir: 'meta', note: 'turn started ' + note, turnId });
  return turnId;
}
function summarizeTurn(from, label) {
  const evs = events.slice(from).map((e) => e.o);
  const tools = evs.filter((o) => (o.method === 'item/started' || o.method === 'item/completed') && !['userMessage', 'agentMessage', 'reasoning'].includes(itemType(o)))
    .map((o) => ({ m: o.method, type: itemType(o), id: o.params.item.id, status: o.params.item.status ?? null, exitCode: o.params.item.exitCode, turnId: o.params.turnId }));
  const reqs = evs.filter((o) => o.id !== undefined && o.method !== undefined).map((o) => ({ method: o.method, id: o.id, itemId: o.params?.itemId, approvalId: o.params?.approvalId ?? null, net: o.params?.networkApprovalContext ?? null }));
  const errs = evs.filter((o) => o.method === 'error').map((o) => o.params);
  const tc = evs.filter((o) => o.method === 'turn/completed').map((o) => ({ id: o.params.turn.id, status: o.params.turn.status, error: o.params.turn.error }));
  const msgs = evs.filter((o) => o.method === 'item/completed' && itemType(o) === 'agentMessage').map((o) => String(o.params.item.text || '').slice(0, 200));
  const s = { label, tools, reqs, errs, tc, msgs };
  rec({ dir: 'meta', b5summary: s });
  console.log('SUMMARY ' + JSON.stringify(s));
}

const S = {};
S.A = async () => {
  await handshake();
  const from = events.length;
  const tid = await startTurn('Run exactly this shell command, once, with no changes: exit 7\nIt is expected to fail with exit code 7; do not retry it. After it finishes, reply with exactly: DONE-A', 'A turn/start');
  await waitFor(turnDone(tid), 240000, from);
  await settle(3000, 15000);
  summarizeTurn(from, 'A');
};

S.B = async () => {
  await handshake();
  const from = events.length;
  const tid = await startTurn('Run exactly this shell command, once, with no changes: Start-Sleep -Seconds 8; exit 5\nIt is expected to fail with exit code 5; do not retry it. After it finishes, reply with exactly: DONE-B', 'B turn/start');
  const c = await waitFor(isCmdStarted, 180000, from);
  if (!c) { rec({ dir: 'meta', note: 'no command started' }); summarizeTurn(from, 'B'); return; }
  const cmdId = c.o.params.item.id;
  rec({ dir: 'meta', note: 'command started', cmdId, turnId: c.o.params.turnId });
  await sleep(2000);
  const ir = await request('turn/interrupt', { threadId, turnId: tid }, 'B interrupt');
  const tInt = now();
  rec({ dir: 'meta', interruptResp: ir.o, tInt });
  await waitFor(turnDone(tid), 60000, from);
  // idle wait: command would finish ~6 s after the interrupt; wait >= 30 s beyond that
  const late = await waitFor((o) => o.method === 'item/completed' && o.params?.item?.id === cmdId, 45000, from);
  rec({ dir: 'meta', note: 'late end (idle wait)', found: !!late, dt: late ? Math.round(late.t - tInt) : null });
  // keep watching >= 30 s past the command's natural end for anything else (a second end, an error)
  if (late) { const extraFrom = events.length; await sleep(32000); rec({ dir: 'meta', note: 'lines in the 32 s after the late end', n: events.length - extraFrom }); }
  if (!late) {
    // no end while idle — see whether a follow-up turn flushes it
    const f2 = events.length;
    const t2 = await startTurn('Without using any tools, reply with exactly: READY-B', 'B follow-up turn/start');
    await waitFor(turnDone(t2), 120000, f2);
    const late2 = await waitFor((o) => o.method === 'item/completed' && o.params?.item?.id === cmdId, 30000, from);
    rec({ dir: 'meta', note: 'late end (after follow-up)', found: !!late2, dt: late2 ? Math.round(late2.t - tInt) : null });
  }
  await settle(3000, 15000);
  summarizeTurn(from, 'B');
};

S.C = async () => {
  if (!outside) throw new Error('scenario C needs outsideDir');
  fs.mkdirSync(outside, { recursive: true });
  const cmdTarget = path.join(outside, 'cmd_write.txt');
  const patchTarget = path.join(outside, 'patch_target.txt');
  try { fs.rmSync(cmdTarget, { force: true }); } catch (_) { }
  fs.writeFileSync(patchTarget, 'ORIGINAL\n');
  rec({ dir: 'meta', cmdTarget, patchTarget });
  await handshake();
  // (a) shell command writing outside cwd -> command approval request
  let from = events.length;
  const ta = await startTurn(
    `Using a single shell command (do NOT use apply_patch or any file-editing tool), write the text HELLO into the file ${cmdTarget}\n` +
    'That path is outside the workspace, so run the command with escalated permissions (request approval for it). Do not try any alternative if it is not allowed. ' +
    'Afterwards reply with one short sentence saying whether the file was written.', 'Ca turn/start');
  await waitFor(turnDone(ta), 240000, from);
  await settle(3000, 15000);
  summarizeTurn(from, 'Ca');
  // (b) edit tool changing a file outside cwd -> file change approval request
  from = events.length;
  const tb = await startTurn(
    `Using your file-editing tool (apply_patch — do NOT use any shell command), change the line ORIGINAL to EDITED in the file ${patchTarget}\n` +
    'That file is outside the workspace; if the edit is not allowed, do not try any alternative. ' +
    'Afterwards reply with one short sentence saying whether the file was changed.', 'Cb turn/start');
  await waitFor(turnDone(tb), 240000, from);
  await settle(3000, 15000);
  summarizeTurn(from, 'Cb');
  rec({ dir: 'meta', files: { cmdTargetExists: fs.existsSync(cmdTarget), patchTargetContent: fs.readFileSync(patchTarget, 'utf8') } });
  console.log('FILES cmdTargetExists=' + fs.existsSync(cmdTarget) + ' patchTarget=' + JSON.stringify(fs.readFileSync(patchTarget, 'utf8')));
};

S.R = async () => {
  const tid = process.env.B5_THREAD;
  if (!tid) throw new Error('scenario R needs B5_THREAD');
  await initialize();
  const r = await request('thread/resume', { threadId: tid, cwd, approvalPolicy: 'on-request', sandbox: 'workspace-write', excludeTurns: true }, 'thread/resume');
  threadId = r.o.result?.thread?.id;
  const cursor = r.o.result?.itemsBackwardsCursor ?? null;
  rec({ dir: 'meta', resumed: threadId, cursor, error: r.o.error ?? null });
  const params = { threadId: tid, limit: 40, sortDirection: 'desc' };
  if (cursor) params.cursor = cursor;
  const page = await request('thread/items/list', params, 'thread/items/list');
  const data = page.o.result?.data ?? [];
  const items = data.map((e) => ({ turnId: e.turnId, type: e.item?.type, id: e.item?.id, status: e.item?.status ?? null, exitCode: e.item?.exitCode, text: e.item?.type === 'agentMessage' ? String(e.item.text || '').slice(0, 80) : undefined }));
  rec({ dir: 'meta', historyItems: items, nextCursor: page.o.result?.nextCursor ?? null, error: page.o.error ?? null });
  console.log('HISTORY ' + JSON.stringify(items));
};

function killTree() {
  try { execFileSync('taskkill', ['/PID', String(child.pid), '/T', '/F'], { stdio: 'ignore' }); rec({ dir: 'meta', killed: child.pid }); } catch (e) { rec({ dir: 'meta', killErr: String(e.message).slice(0, 200) }); }
}
(async () => {
  const hard = setTimeout(() => { rec({ dir: 'meta', hardTimeout: true }); killTree(); process.exit(3); }, 9 * 60 * 1000);
  try {
    await (S[scenario] || (async () => { throw new Error('unknown scenario ' + scenario); }))();
  } catch (e) { rec({ dir: 'meta', error: String(e.stack || e) }); console.log('ERROR ' + String(e.stack || e)); }
  rec({ dir: 'meta', serverReqs });
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
