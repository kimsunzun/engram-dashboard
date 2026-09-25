// Phase 0 spike harness — claude CLI stream-json, mirrors engram ClaudeBackend::build_spec (JSON mode).
// usage: node claude_harness.js <scenario> <outDir> <cwd> [model] [slashCmd (M13 only, default /compact)]
'use strict';
const { spawn, execFileSync } = require('child_process');
const fs = require('fs'), path = require('path'), crypto = require('crypto');
const { performance } = require('perf_hooks');

const scenario = process.argv[2];
const outDir = process.argv[3];
const cwd = process.argv[4];
const model = process.argv[5] || 'haiku';
fs.mkdirSync(outDir, { recursive: true });
fs.mkdirSync(cwd, { recursive: true });
const stamp = new Date().toISOString().replace(/[:.]/g, '-');
const slashCmd = process.argv[6] || '/compact';
const tag = scenario === 'M13' ? '-' + slashCmd.replace(/[^a-z0-9]/gi, '') : '';
const logPath = path.join(outDir, `claude-${scenario}${tag}-${stamp}.jsonl`);
const logFd = fs.openSync(logPath, 'w');
const T0 = performance.now();
const now = () => Math.round((performance.now() - T0) * 10) / 10;
function rec(o) { fs.writeSync(logFd, JSON.stringify({ t: now(), ...o }) + '\n'); }
const uuid = () => crypto.randomUUID();
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ---- spawn (exact argv of ClaudeBackend JSON mode, no control endpoint; --model as extra_args) ----
const sid = uuid();
const args = ['--permission-mode', 'bypassPermissions', '-p', '--input-format', 'stream-json',
  '--output-format', 'stream-json', '--replay-user-messages', '--verbose', '--session-id', sid,
  '--model', model];
const env = { ...process.env, MAX_THINKING_TOKENS: '8000' };
const claudeEnvKeys = Object.keys(process.env).filter((k) => /claude/i.test(k));
for (const k of ['CLAUDECODE', 'CLAUDE_CODE_ENTRYPOINT', 'CLAUDE_CODE_SSE_PORT']) delete env[k];
rec({ dir: 'meta', argv: ['cmd.exe', '/c', 'claude', ...args], cwd, sid, inheritedClaudeEnvKeys: claudeEnvKeys });

const child = spawn('cmd.exe', ['/c', 'claude', ...args], { cwd, env, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
rec({ dir: 'meta', pid: child.pid });
console.log('CHILD_PID=' + child.pid + ' SID=' + sid + ' LOG=' + logPath);

const events = []; // parsed stdout objects with t
const waiters = [];
let chunkNo = 0, outBuf = '', lastEventT = 0, exited = false;
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
    rec({ dir: 'out', chunk: chunkNo, line: o ?? line });
    if (o) {
      const ev = { t, chunk: chunkNo, o, idx: events.length };
      events.push(ev);
      for (const w of [...waiters]) {
        if (ev.idx >= w.from && w.pred(o, ev)) { waiters.splice(waiters.indexOf(w), 1); w.resolve(ev); }
      }
    }
  }
});
child.stderr.on('data', (d) => rec({ dir: 'err', text: d.toString('utf8') }));
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
  while (now() - start < maxMs) {
    await sleep(250);
    if (now() - lastEventT >= quietMs) return;
  }
}
function writeRaw(str, note) {
  rec({ dir: 'in', note, raw: str });
  child.stdin.write(str);
}
function userLine(text, u) {
  return JSON.stringify({ type: 'user', message: { role: 'user', content: [{ type: 'text', text }] }, uuid: u }) + '\n';
}
function cancelLine(u) {
  return JSON.stringify({ type: 'control_request', request_id: 'cancel:' + u, request: { subtype: 'cancel_async_message', message_uuid: u } }) + '\n';
}
const isToolUse = (o) => o.type === 'assistant' && Array.isArray(o.message?.content) && o.message.content.some((c) => c.type === 'tool_use');
const isToolResult = (o) => o.type === 'user' && Array.isArray(o.message?.content) && o.message.content.some((c) => c.type === 'tool_result');
const isResult = (o) => o.type === 'result';
const lc = (u, st) => (o) => o.type === 'command_lifecycle' && o.command_uuid === u && (!st || o.state === st);
const terminal = ['completed', 'cancelled', 'discarded', 'refused'];
const isTerminalLc = (u) => (o) => o.type === 'command_lifecycle' && o.command_uuid === u && terminal.includes(o.state);

// ---------------- scenarios ----------------
const S = {};

// M1/M2/M4/M5: lifecycle emission, replay echo timing, init timing, transcript shape
S.M1 = async () => {
  await sleep(4000); // M4: does system/init arrive before any input?
  const A = uuid(), B = uuid();
  rec({ dir: 'meta', ids: { A, B } });
  writeRaw(userLine('Use the Bash tool to run exactly this command: sleep 20\nAfter it finishes, reply with exactly: DONE-A', A), 'A');
  const tu = await waitFor(isToolUse, 120000);
  if (!tu) return;
  await sleep(3000);
  writeRaw(userLine('When you reply, also include the word PINEAPPLE.', B), 'B mid-tool');
  await waitFor(isResult, 180000);
  await waitFor(isTerminalLc(B), 60000, 0);
  await settle(8000, 60000);
};

// M3: cancel_async_message — a) queued, c1) same-chunk write+cancel, c2) cancel-before-message, b) fold race x3
S.__trialRunner = async (delays) => {
  for (let k = 0; k < delays.length; k++) {
    const name = 'tb' + (k + 1), delayMs = delays[k];
    const A = uuid(), B = uuid();
    rec({ dir: 'meta', trial: name, delayMs, ids: { A, B } });
    const from = events.length;
    writeRaw(userLine(`Use the Bash tool to run exactly this command: sleep 4
After it finishes, reply with exactly: DONE-${name}`, A), 'A ' + name);
    const tu = await waitFor(isToolUse, 120000, from);
    if (!tu) return;
    await sleep(1500);
    writeRaw(userLine(`Also include the word WORD${name.toUpperCase()} in your reply.`, B), 'B ' + name);
    await new Promise((resolve) => {
      const w = { pred: isToolResult, from: events.length, resolve: (ev) => { setTimeout(() => writeRaw(cancelLine(B), 'cancel on tool_result+' + delayMs + 'ms ' + name + ' (tool_result seen at ' + ev.t + ')'), delayMs); resolve(); } };
      waiters.push(w);
    });
    await waitFor(isResult, 180000, from);
    await waitFor(isTerminalLc(B), 20000, from);
    await settle(4000, 30000);
  }
};

S.M3 = async () => {
  const trial = async (name, sleepSec, mode, delayMs = 0) => {
    const A = uuid(), B = uuid();
    rec({ dir: 'meta', trial: name, ids: { A, B } });
    const from = events.length;
    writeRaw(userLine(`Use the Bash tool to run exactly this command: sleep ${sleepSec}\nAfter it finishes, reply with exactly: DONE-${name}`, A), 'A ' + name);
    const tu = await waitFor(isToolUse, 120000, from);
    if (!tu) return;
    await sleep(2000);
    const word = 'WORD' + name.toUpperCase().replace(/[^A-Z0-9]/g, '');
    const bText = `Also include the word ${word} in your reply.`;
    if (mode === 'queued') {
      writeRaw(userLine(bText, B), 'B ' + name);
      await waitFor(lc(B, 'queued'), 5000);
      await sleep(1000);
      writeRaw(cancelLine(B), 'cancel ' + name);
    } else if (mode === 'samechunk') {
      writeRaw(userLine(bText, B) + cancelLine(B), 'B+cancel same write ' + name);
    } else if (mode === 'cancelfirst') {
      writeRaw(cancelLine(B), 'cancel-before-message ' + name);
      await sleep(700);
      writeRaw(userLine(bText, B), 'B after cancel ' + name);
    } else if (mode === 'foldrace') {
      writeRaw(userLine(bText, B), 'B ' + name);
      // send cancel synchronously in the same stdout handler turn that delivers the tool_result
      await new Promise((resolve) => {
        const w = { pred: isToolResult, from: events.length, resolve: (ev) => {
          const fire = () => writeRaw(cancelLine(B), 'cancel on tool_result+' + delayMs + 'ms ' + name + ' (tool_result seen at ' + ev.t + ')');
          if (delayMs === 0) fire(); else setTimeout(fire, delayMs);
          resolve(); } };
        waiters.push(w);
      });
    }
    await waitFor(isResult, 180000, from);
    await waitFor(isTerminalLc(B), 20000, from);
    await settle(4000, 30000);
  };
  await trial('a', 12, 'queued');
  await trial('c1', 12, 'samechunk');
  await trial('c2', 12, 'cancelfirst');
  const delays = [0, 150, 300, 450];
  for (let k = 0; k < delays.length; k++) await trial('b' + (k + 1), 5, 'foldrace', delays[k]);
};

// M3B: targeted fold-in-flight race (ⓑ) — cancel at tool_result + delay near the observed fold time
S.M3B = async () => {
  const delays = (process.env.M3B_DELAYS || '540,580,620,660').split(',').map(Number);
  await S.__trialRunner(delays);
};

// M7: S1 policy — hand over upon observing tool completion (tool_result line), 10 trials
S.M7 = async () => {
  for (let k = 1; k <= 10; k++) {
    const A = uuid(), B = uuid();
    rec({ dir: 'meta', trial: k, ids: { A, B } });
    const from = events.length;
    writeRaw(userLine(`Use the Bash tool to run exactly this command: sleep 2\nAfter it finishes, reply with exactly: OK-${k}`, A), 'A ' + k);
    await new Promise((resolve) => {
      const w = { pred: isToolResult, from, resolve: (ev) => { writeRaw(userLine(`Also reply with the word KIWI${k}.`, B), 'B on tool_result ' + k + ' (seen at ' + ev.t + ')'); resolve(); } };
      waiters.push(w);
      setTimeout(() => { const i = waiters.indexOf(w); if (i >= 0) { waiters.splice(i, 1); rec({ dir: 'meta', timeout: 'toolResult ' + k }); resolve(); } }, 120000);
    });
    await waitFor(isTerminalLc(B), 180000, from);
    await settle(4000, 60000);
  }
};

// M13: slash command as a user message — mid-turn (while a Bash tool runs) x2 and idle control x1
S.M13 = async () => {
  const any = (u) => (o) => o.type === 'command_lifecycle' && o.command_uuid === u;
  const midTrial = async (name) => {
    const A = uuid(), C = uuid();
    rec({ dir: 'meta', trial: name, slashCmd, ids: { A, C } });
    const from = events.length;
    writeRaw(userLine(`Use the Bash tool to run exactly this command: sleep 12
After it finishes, reply with exactly: DONE-${name}`, A), 'A ' + name);
    const tu = await waitFor(isToolUse, 120000, from);
    if (!tu) return;
    await sleep(2000);
    writeRaw(userLine(slashCmd, C), 'C slash mid-tool ' + name);
    await waitFor(any(C), 5000, from);
    await waitFor(isResult, 180000, from);
    await waitFor(isTerminalLc(C), 120000, from);
    await settle(10000, 120000);
  };
  const idleTrial = async (name) => {
    const C = uuid();
    rec({ dir: 'meta', trial: name, slashCmd, ids: { C } });
    const from = events.length;
    writeRaw(userLine(slashCmd, C), 'C slash idle ' + name);
    await waitFor(isTerminalLc(C), 120000, from);
    await settle(10000, 120000);
  };
  await midTrial('m1');
  await idleTrial('i1');
  await midTrial('m2');
};

// ---------------- driver ----------------
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
  for (let i = 0; i < 60 && !exited; i++) await sleep(250);
  if (!exited) killTree();
  await sleep(500);
  clearTimeout(hard);
  const lcs = events.filter((e) => e.o.type === 'command_lifecycle').map((e) => `${e.t} ${e.o.command_uuid.slice(0, 8)} ${e.o.state}`);
  console.log('LIFECYCLE\n' + lcs.join('\n'));
  console.log('EVENTS=' + events.length + ' LOG=' + logPath);
  fs.closeSync(logFd);
  process.exit(0);
})();
