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
// M16 carries its case in the log name (codex-M16-A-<stamp>.jsonl)
const logTag = scenario === 'M16' ? `M16-${(process.env.M16_CASE || 'A').toUpperCase()}` : scenario;
const logPath = path.join(outDir, `codex-${logTag}-${stamp}.jsonl`);
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
    trackTools(o); // M16: per-turn running-tool set, updated before any waiter sees this line
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
async function handshake(opts = {}) {
  // opts.experimental: opt into experimentalApi (only M16 case C needs it — `turn/start.collaborationMode`)
  const initParams = { clientInfo: { name: 'engram-dashboard', version: '0.1.0' } };
  if (opts.experimental) initParams.capabilities = { experimentalApi: true };
  const init = await request('initialize', initParams, 'initialize');
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

// ---------------------------------------------------------------------------------------------
// Phase 0b (2026-09-26) — M9 (empty-input turn/start) · M10 (turn-end "record only" branch).
// Vendor source (0.156.1): the last pending-input check is `turn.rs` run_turn (after sampling) —
// the record-only window runs from that check to `active_turn.task.take()` in
// `tasks/mod.rs` on_task_finished (stop hooks · flush_rollout in between). A steer that lands there
// is taken by `take_pending_input_for_turn_state` and recorded by `run_hooks_and_record_inputs`
// before TurnComplete is emitted. The window has no notification of its own, so we anchor on the
// final `thread/tokenUsage/updated` and busy-spin a jittered delay (setTimeout is too coarse on Windows).
function spin(ms) { const end = performance.now() + ms; while (performance.now() < end) { /* busy wait on purpose */ } }
const pick = (e) => ({ turnId: e.turnId, type: e.item?.type, id: e.item?.id, clientId: e.item?.clientId ?? null });
async function emptyTurnStart(note) {
  const from = events.length;
  const r = await request('turn/start', { threadId, input: [] }, note);
  rec({ dir: 'meta', note: note + ' response', resp: r.o });
  const turnId = r.o.result?.turn?.id ?? null;
  let text = null, done = null;
  if (turnId) {
    done = await waitFor((o) => isTurnCompleted(o) && o.params?.turn?.id === turnId, 180000, from);
    await settle(2000, 20000);
    const msgs = events.slice(from).filter((ev) => ev.o.method === 'item/completed' && itemType(ev.o) === 'agentMessage' && ev.o.params?.turnId === turnId).map((ev) => ev.o.params.item.text);
    const userItems = events.slice(from).filter((ev) => ev.o.method === 'item/started' && itemType(ev.o) === 'userMessage').map((ev) => ({ t: ev.t, turnId: ev.o.params?.turnId, clientId: ev.o.params.item.clientId ?? null, text: (ev.o.params.item.content?.[0]?.text ?? '').slice(0, 60) }));
    text = msgs;
    rec({ dir: 'meta', note: note + ' result', turnId, status: done?.o?.params?.turn?.status ?? null, turnError: done?.o?.params?.turn?.error ?? null, agentMessages: msgs, userItemsDuringEmptyTurn: userItems });
  }
  return { resp: r.o, turnId, agentMessages: text, status: done?.o?.params?.turn?.status ?? null };
}
// Walk thread/items/list desc pages. The first request is sent synchronously (callable inside a handler).
function walkItems(limit, note, prevTurnId, maxPages = 20) {
  const t0 = now();
  const pages = [];
  const go = async () => {
    let cursor = null;
    for (let p = 0; p < maxPages; p++) {
      const params = { threadId, limit, sortDirection: 'desc' };
      if (cursor) params.cursor = cursor;
      const sentT = now();
      const r = await request('thread/items/list', params, `${note} page ${p}`);
      const res = r.o.result;
      const entries = (res?.data ?? []).map(pick);
      pages.push({ p, sentT, respT: r.t, error: r.o.error ?? null, entries, nextCursor: res?.nextCursor ?? null });
      if (!res) return 'error';
      if (prevTurnId && entries.some((e) => e.turnId === prevTurnId)) return 'passed-turn';
      if (!res.nextCursor) return 'end';
      cursor = res.nextCursor;
    }
    return 'maxPages';
  };
  return go().then((stoppedBy) => ({ t0, tEnd: now(), stoppedBy, pages }));
}

S.M9 = async () => {
  // Variant B (control): empty turn/start after a normally completed turn (last user message already answered).
  await handshake();
  const from = events.length;
  const U = uuid();
  const t1 = await startTurn('Without using any tools, reply with exactly: READY-7', U, 'M9 normal turn/start');
  await waitFor((o) => isTurnCompleted(o) && o.params?.turn?.id === t1, 120000, from);
  await settle(2000, 20000);
  await itemsList('M9 after normal turn');
  const r1 = await emptyTurnStart('M9 empty turn/start (after answered turn)');
  rec({ dir: 'meta', m9summary: { variant: 'B-after-answered-turn', accepted: !!r1.turnId, error: r1.resp.error ?? null, agentMessages: r1.agentMessages, status: r1.status } });
  await itemsList('M9 final');
};

S.M10 = async () => {
  await handshake();
  const maxAttempts = Number(process.env.M10_MAX || 40), wantHits = Number(process.env.M10_HITS || 3);
  let d = Number(process.env.M10_D0 || 1.0); // spin delay after final tokenUsage/updated (ms)
  let hits = 0, prevTurnId = null, m9Done = 0;
  const tally = { HIT: 0, early: 0, late: 0, other: 0 };
  for (let k = 1; k <= maxAttempts && hits < wantHits; k++) {
    const from = events.length;
    const U = uuid(), W = uuid();
    const token = `PINEAPPLE${k}`;
    const dUsed = Math.max(0, Math.round(d * 100) / 100);
    rec({ dir: 'meta', trial: k, ids: { U, W }, d: dUsed });
    const tk = await startTurn(`Without using any tools, reply with exactly: ACK-${k}`, U, 'M10 turn/start ' + k);
    let sp = null, steerWriteT = null, tokT = null, tokChunk = null;
    // steer after a jittered spin, inside the handler that delivers the first tokenUsage/updated of this turn
    const onTok = onceSync((o) => o.method === 'thread/tokenUsage/updated' && o.params?.turnId === tk, (ev) => {
      tokT = ev.t; tokChunk = ev.chunk;
      spin(dUsed);
      steerWriteT = now();
      sp = steer(tk, `Reply with exactly the word ${token}.`, W, `M10 steer ${k} d=${dUsed} (tok at ${ev.t})`);
    }, 120000, from);
    // probe the moment turn/completed arrives: one full page (limit 40) + a limit-2 walk (design probe)
    let probeFull = null, walk = null, tcT = null, tcChunk = null, probeSentT = null;
    const tcEv = await onceSync((o) => isTurnCompleted(o) && o.params?.turn?.id === tk, (ev) => {
      tcT = ev.t; tcChunk = ev.chunk;
      probeSentT = now();
      probeFull = request('thread/items/list', { threadId, limit: 40, sortDirection: 'desc' }, `M10 probe full ${k} (in turn/completed handler)`);
      walk = walkItems(2, `M10 probe walk ${k}`, prevTurnId);
    }, 180000, from);
    await onTok;
    const steerR = sp ? await sp : null;
    const full = probeFull ? await probeFull : null;
    const w = walk ? await walk : null;
    await settle(2500, 30000);
    // collect
    const evs = events.slice(from);
    const echoS = evs.find((ev) => ev.o.method === 'item/started' && itemType(ev.o) === 'userMessage' && ev.o.params.item.clientId === W);
    const echoC = evs.find((ev) => ev.o.method === 'item/completed' && itemType(ev.o) === 'userMessage' && ev.o.params.item.clientId === W);
    const msgs = evs.filter((ev) => ev.o.method === 'item/completed' && itemType(ev.o) === 'agentMessage' && ev.o.params?.turnId === tk).map((ev) => ev.o.params.item.text);
    const toks = evs.filter((ev) => ev.o.method === 'thread/tokenUsage/updated' && ev.o.params?.turnId === tk).map((ev) => ev.t);
    const fullEntries = (full?.o?.result?.data ?? []).map(pick);
    const fIdx = fullEntries.findIndex((e) => e.clientId === W);
    let walkFound = null;
    if (w) for (const pg of w.pages) { const i = pg.entries.findIndex((e) => e.clientId === W); if (i >= 0) { walkFound = { page: pg.p, idxInPage: i, respT: pg.respT }; break; } }
    const steerErr = steerR?.o?.error ?? null;
    let verdict;
    if (!sp) verdict = 'other';
    else if (steerErr) verdict = 'late';
    else if (msgs.length >= 2 || toks.length >= 2) verdict = 'early';
    else if (msgs.length === 1) verdict = 'HIT';
    else verdict = 'other';
    tally[verdict]++;
    const summary = {
      trial: k, d: dUsed, verdict, turnId: tk,
      tokT, tokChunk, steerWriteT, steerWriteMinusTok: tokT != null ? Math.round((steerWriteT - tokT) * 10) / 10 : null,
      steerResp: steerR?.o?.result ?? steerErr, steerRespT: steerR?.t ?? null,
      tcT, tcChunk, tcMinusTok: tcT != null && tokT != null ? Math.round((tcT - tokT) * 10) / 10 : null,
      tcStatus: tcEv?.o?.params?.turn?.status ?? null,
      echoStartedT: echoS?.t ?? null, echoCompletedT: echoC?.t ?? null, echoTurnId: echoS?.o?.params?.turnId ?? null,
      echoVsTc: echoS && tcT != null ? Math.round((echoS.t - tcT) * 10) / 10 : null,
      agentMessages: msgs, tokenUsageCount: toks.length,
      probeSentT, probeFullRespT: full?.t ?? null, probeFullFound: fIdx >= 0, probeFullIdxFromNewest: fIdx, probeFullItem: fIdx >= 0 ? fullEntries[fIdx] : null,
      probeFullTop: fullEntries.slice(0, 5),
      walk: w ? { stoppedBy: w.stoppedBy, pages: w.pages.length, ms: Math.round((w.tEnd - w.t0) * 10) / 10, found: walkFound, pageTurns: w.pages.map((pg) => pg.entries.map((e) => `${(e.turnId || '').slice(-6)}:${e.type}${e.clientId === W ? '*W' : ''}`)) } : null,
    };
    rec({ dir: 'meta', m10summary: summary });
    console.log(`M10 trial ${k} d=${dUsed} ${verdict} tc-tok=${summary.tcMinusTok} echoVsTc=${summary.echoVsTc} fullFound=${summary.probeFullFound} walk=${w ? w.stoppedBy + '/' + w.pages.length + (walkFound ? ' found p' + walkFound.page : ' notfound') : '-'}`);
    // staircase on d
    if (verdict === 'early') d = d + 0.5 + Math.random() * 0.5;
    else if (verdict === 'late') d = Math.max(0, d - 0.3 - Math.random() * 0.4);
    else if (verdict === 'HIT') d = Math.max(0, d + (Math.random() - 0.5) * 0.4);
    prevTurnId = tk;
    if (verdict === 'HIT') {
      hits++;
      // settled list after the hit (is the record-only item there once things are quiet?)
      await itemsList(`M10 after hit ${k} (settled)`);
      // M9 variant A: the recorded steer is an unanswered user message in history → empty turn/start
      if (m9Done < 3) {
        m9Done++;
        const r9 = await emptyTurnStart(`M9A empty turn/start after M10 hit ${k}`);
        const answered = (r9.agentMessages || []).some((m) => String(m).includes(token));
        rec({ dir: 'meta', m9summary: { variant: 'A-after-record-only-hit', trial: k, token, accepted: !!r9.turnId, error: r9.resp.error ?? null, agentMessages: r9.agentMessages, status: r9.status, answeredToken: answered } });
        console.log(`M9A after hit ${k}: accepted=${!!r9.turnId} answered=${answered} msgs=${JSON.stringify(r9.agentMessages)}`);
        if (r9.turnId) prevTurnId = r9.turnId;
      }
    } else if (verdict === 'late' || verdict === 'other') {
      // a late steer was refused: nothing unanswered. nothing to do.
    }
  }
  rec({ dir: 'meta', m10tally: tally, hits });
  console.log('M10 tally ' + JSON.stringify(tally));
};

// ---------------------------------------------------------------------------------------------
// M16 (2026-09-26) — answer-end boundary. Does a steer written at the answer-end signal
// (`agentMessage` / `plan` `item/completed` while no tool of the turn is running) still get into the
// SAME turn's follow-up sampling?
// Vendor source (0.156.1, core/src/session/turn.rs): the answer item's `item/completed` is emitted on the
// stream's `OutputItemDone` (try_run_sampling_request); the stream then runs on to `ResponseEvent::Completed`;
// after in-flight tools drain, `send_token_count_event` (→ `thread/tokenUsage/updated`) is sent and the
// function returns; run_turn then checks `has_pending_input` (:555-567). That check closes the window —
// a steer landing after it is record-only (M10) or −32600. Plan items (`plan`) complete on the same
// `OutputItemDone` (maybe_complete_plan_item_from_message) but only exist in Plan collaboration mode,
// which needs `turn/start.collaborationMode` (experimental → initialize capabilities.experimentalApi).
// env: M16_CASE=A (tool-less long answer) | B (final answer after one tool) | C (plan-mode turn)
//      M16_N     steers inside the signal handler, alternating +0 / +1 ms busy-spin (default 10)
//      M16_TOK   steers inside the handler of the tokenUsage/updated that follows the signal (default 3)
//      M16_SWEEP setTimeout-delayed steers spread over the gap observed so far (default 6)
//      M16_K0    first attempt number (token KIWI<k>)
const NON_TOOL_ITEMS = new Set(['userMessage', 'hookPrompt', 'agentMessage', 'plan', 'reasoning', 'functionCallOutput', 'subAgentActivity', 'enteredReviewMode', 'exitedReviewMode', 'contextCompaction']);
const turnTools = new Map(); // turnId -> { running:Set<itemId>, done:number }
function trackTools(o) {
  if (o.method !== 'item/started' && o.method !== 'item/completed') return;
  const it = o.params?.item, tid = o.params?.turnId;
  if (!it || !tid || NON_TOOL_ITEMS.has(it.type)) return;
  let s = turnTools.get(tid);
  if (!s) { s = { running: new Set(), done: 0 }; turnTools.set(tid, s); }
  if (o.method === 'item/started') s.running.add(it.id); else { s.running.delete(it.id); s.done++; }
}
const toolState = (tid) => turnTools.get(tid) || { running: new Set(), done: 0 };
const median = (a) => { const b = [...a].sort((x, y) => x - y); return b.length ? b[Math.floor(b.length / 2)] : null; };
const r1 = (x) => (x == null ? null : Math.round(x * 10) / 10);

S.M16 = async () => {
  const cs = (process.env.M16_CASE || 'A').toUpperCase();
  const N1 = Number(process.env.M16_N ?? 10), N3 = Number(process.env.M16_TOK ?? 3), N2 = Number(process.env.M16_SWEEP ?? 6);
  const K0 = Number(process.env.M16_K0 || 1);
  await handshake({ experimental: cs === 'C' });
  const prompt = (k) => cs === 'A'
    ? `Without using any tools, write a detailed paragraph of about 150 words about the history of lighthouses. Finish with the line END-${k}.`
    : cs === 'B'
      ? `Run exactly this shell command: Start-Sleep -Seconds 1\nAfter it finishes, do not use any more tools: write a paragraph of about 80 words about lighthouses and finish with the line END-${k}.`
      : `Propose a short plan (3 to 5 steps) for adding a README.md file to a small project. Do not ask any questions, do not use any tools and do not inspect any files - write the final plan now inside <proposed_plan></proposed_plan> tags. Plan id P-${k}.`;
  const sigType = cs === 'C' ? 'plan' : 'agentMessage';
  const sched = [];
  for (let i = 0; i < N1; i++) sched.push({ mode: 'sig', jit: i % 2 });
  for (let i = 0; i < N3; i++) sched.push({ mode: 'tok' });
  for (let i = 0; i < N2; i++) sched.push({ mode: 'sweep', i });
  const gaps = [];
  const tally = {};
  let k = K0 - 1;
  for (const sc of sched) {
    k++;
    const from = events.length;
    const U = uuid(), W = uuid(), token = `KIWI${k}`;
    const steerText = `Also add one short final line that contains the word ${token}.`;
    rec({ dir: 'meta', trial: k, m16case: cs, mode: sc, ids: { U, W }, token });
    const params = { threadId, clientUserMessageId: U, input: txt(prompt(k)) };
    if (cs === 'C') params.collaborationMode = { mode: 'plan', settings: { model, reasoning_effort: effort, developer_instructions: null } };
    const r = await request('turn/start', params, `M16 ${cs} turn/start ${k}`);
    const tk = r.o.result?.turn?.id;
    if (!tk) { rec({ dir: 'meta', m16summary: { trial: k, case: cs, mode: sc.mode, verdict: 'start-error', error: r.o.error ?? null } }); tally['start-error'] = (tally['start-error'] || 0) + 1; break; }
    let sp = null, steerWriteT = null, sigEv = null, tokEv = null, dPlanned = null, runningAtSig = null, doneAtSig = null;
    let resolveSent; const sent = new Promise((res) => (resolveSent = res));
    const fire = (note) => { steerWriteT = now(); sp = steer(tk, steerText, W, note); resolveSent(); };
    const sigPred = (o) => o.method === 'item/completed' && o.params?.turnId === tk && itemType(o) === sigType
      && toolState(tk).running.size === 0 && (cs !== 'B' || toolState(tk).done >= 1);
    const sigRes = await onceSync(sigPred, (ev) => {
      sigEv = ev; runningAtSig = toolState(tk).running.size; doneAtSig = toolState(tk).done;
      if (sc.mode === 'sig') { if (sc.jit) spin(sc.jit); fire(`M16 ${cs} steer ${k} at signal +${sc.jit}ms (sig at ${ev.t})`); }
      else if (sc.mode === 'tok') {
        // registered synchronously so it sees the very next line
        waiters.push({ from: ev.idx + 1, pred: (o) => o.method === 'thread/tokenUsage/updated' && o.params?.turnId === tk,
          resolve: (tev) => { tokEv = tev; fire(`M16 ${cs} steer ${k} at following tokenUsage (tok at ${tev.t})`); } });
      } else {
        const g = gaps.length ? median(gaps) : 60;
        dPlanned = Math.max(0, Math.round(g * (0.3 + 1.2 * sc.i / Math.max(1, N2 - 1))));
        setTimeout(() => fire(`M16 ${cs} steer ${k} at signal + timeout ${dPlanned}ms`), dPlanned);
      }
    }, 240000, from);
    const tcEv = await waitFor((o) => isTurnCompleted(o) && o.params?.turn?.id === tk, 300000, from);
    await Promise.race([sent, sleep(3000)]);
    const steerR = sp ? await sp : null;
    await settle(2500, 30000);
    const evs = events.slice(from);
    const answerTypes = ['agentMessage', 'plan'];
    const msgsAll = evs.filter((ev) => ev.o.method === 'item/completed' && answerTypes.includes(itemType(ev.o)) && ev.o.params?.turnId === tk)
      .map((ev) => ({ t: ev.t, type: itemType(ev.o), phase: ev.o.params.item.phase ?? null, text: String(ev.o.params.item.text || '').slice(0, 90) }));
    const toks = evs.filter((ev) => ev.o.method === 'thread/tokenUsage/updated' && ev.o.params?.turnId === tk);
    let summary;
    if (!sigRes) {
      summary = { trial: k, case: cs, mode: sc.mode, verdict: 'nosignal', turnId: tk, msgsAll, tokCount: toks.length, tcStatus: tcEv?.o?.params?.turn?.status ?? null,
        itemTypes: [...new Set(evs.filter((ev) => ev.o.method === 'item/completed').map((ev) => itemType(ev.o)))] };
    } else {
      const sigIdx = sigEv.idx, sigT = sigEv.t;
      const tokAfter = tokEv || evs.find((ev) => ev.idx > sigIdx && ev.o.method === 'thread/tokenUsage/updated' && ev.o.params?.turnId === tk) || null;
      const echoS = evs.find((ev) => ev.o.method === 'item/started' && itemType(ev.o) === 'userMessage' && ev.o.params?.item?.clientId === W) || null;
      const postEcho = echoS ? evs.filter((ev) => ev.idx > echoS.idx && ev.o.method === 'item/completed' && answerTypes.includes(itemType(ev.o)) && ev.o.params?.turnId === tk) : [];
      const postEchoTok = echoS ? evs.filter((ev) => ev.idx > echoS.idx && ev.o.method === 'thread/tokenUsage/updated' && ev.o.params?.turnId === tk).length : 0;
      const answered = postEcho.some((ev) => String(ev.o.params.item.text || '').toUpperCase().includes(token));
      const deltaMethod = cs === 'C' ? 'item/plan/delta' : 'item/agentMessage/delta';
      const sigItemId = sigEv.o.params.item.id;
      const deltas = evs.filter((ev) => ev.idx < sigIdx && ev.o.method === deltaMethod && ev.o.params?.itemId === sigItemId);
      const steerErr = steerR?.o?.error ?? null;
      let verdict;
      if (!sp) verdict = 'nosteer';
      else if (steerErr) verdict = 'late';
      else if (!echoS) verdict = 'no-echo';
      else if (answered) verdict = 'HIT';
      else if (postEcho.length || postEchoTok) verdict = 'followup-ignored';
      else verdict = 'record-only';
      const gap = tokAfter ? tokAfter.t - sigT : null;
      if (gap != null) gaps.push(gap);
      summary = {
        trial: k, case: cs, mode: sc.mode, jit: sc.jit ?? null, dPlanned, verdict, turnId: tk,
        sigT, sigChunk: sigEv.chunk, sigPhase: sigEv.o.params.item.phase ?? null, runningAtSig, doneAtSig,
        sigItemDeltas: deltas.length, sigItemDeltaChunks: new Set(deltas.map((ev) => ev.chunk)).size, firstDeltaToSig: deltas.length ? r1(sigT - deltas[0].t) : null,
        tokT: tokAfter?.t ?? null, tokChunk: tokAfter?.chunk ?? null, gapSigToTok: r1(gap), tokSameChunkAsSig: tokAfter ? tokAfter.chunk === sigEv.chunk : null,
        steerWriteT, steerMinusSig: r1(steerWriteT != null ? steerWriteT - sigT : null), steerMinusTok: r1(steerWriteT != null && tokAfter ? steerWriteT - tokAfter.t : null),
        steerResp: steerR?.o?.result ?? steerErr, steerRespMinusWrite: r1(steerR && steerWriteT != null ? steerR.t - steerWriteT : null),
        echoMinusSig: r1(echoS ? echoS.t - sigT : null), echoTurnMatch: echoS ? echoS.o.params?.turnId === tk : null,
        postEchoMsgs: postEcho.map((ev) => String(ev.o.params.item.text || '').slice(0, 80)), postEchoTok, answered,
        tcMinusSig: r1(tcEv ? tcEv.t - sigT : null), tcStatus: tcEv?.o?.params?.turn?.status ?? null,
        tokCount: toks.length, msgsAll,
      };
    }
    tally[summary.verdict] = (tally[summary.verdict] || 0) + 1;
    rec({ dir: 'meta', m16summary: summary });
    console.log(`M16 ${cs} #${k} ${sc.mode}${sc.jit != null ? '+' + sc.jit : ''}${dPlanned != null ? ' d=' + dPlanned : ''} ${summary.verdict} gap=${summary.gapSigToTok} steer-sig=${summary.steerMinusSig} steer-tok=${summary.steerMinusTok} echo-sig=${summary.echoMinusSig} deltas=${summary.sigItemDeltas} msgs=${(summary.msgsAll || []).length}`);
  }
  rec({ dir: 'meta', m16tally: tally, gaps: gaps.map(r1) });
  console.log('M16 tally ' + JSON.stringify(tally));
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
