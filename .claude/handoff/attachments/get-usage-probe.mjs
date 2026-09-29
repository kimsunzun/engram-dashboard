// get_usage probe: spawn ONE claude in stream-json mode, send control requests only (never a user message).
// usage: node probe.mjs <noinit|init> <logfile>
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import readline from 'node:readline';
import { fileURLToPath } from 'node:url';

const mode = process.argv[2];
const logFile = process.argv[3];
if (!['noinit', 'init'].includes(mode) || !logFile) {
  console.error('usage: node probe.mjs <noinit|init> <logfile>');
  process.exit(2);
}
const TIMEOUT_MS = 60_000;
const exe = path.join(process.env.APPDATA, 'npm', 'node_modules', '@anthropic-ai', 'claude-code', 'bin', 'claude.exe');
const cwd = path.dirname(fileURLToPath(import.meta.url));

// Our backend's StreamJson args (crates/engram-dashboard-agent/src/backend/claude/mod.rs:155-182), no --session-id,
// plus side-effect reducers mirroring t3code's probe (no hooks, no MCP, no transcript).
const args = [
  '--permission-mode', 'bypassPermissions',
  '-p', '--input-format', 'stream-json', '--output-format', 'stream-json',
  '--replay-user-messages', '--verbose',
  '--no-session-persistence',
  '--settings', JSON.stringify({ disableAllHooks: true }),
  '--strict-mcp-config', '--mcp-config', JSON.stringify({ mcpServers: {} }),
];

// Child env: drop this Claude Code session's own CLAUDE* vars (the daemon would not have them).
const env = {};
for (const [k, v] of Object.entries(process.env)) {
  if (/^CLAUDE/i.test(k)) continue;
  env[k] = v;
}
env.MAX_THINKING_TOKENS = '8000';
env.ENABLE_CLAUDEAI_MCP_SERVERS = 'false';
env.CLAUDE_CODE_AUTO_CONNECT_IDE = '0';
env.CLAUDE_CODE_IDE_SKIP_AUTO_INSTALL = '1';

// Redaction applied BEFORE anything is written to disk.
const SENSITIVE_KEY = /(email|account|org|uuid|user_?id|user_?name|full_?name|^name$|login|phone|session_id|cwd|path|token|secret|key)/i;
const SAFE_KEY = /^(display_name|subtype|type|request_id|model_usage|rate_limits|five_hour|seven_day.*|extra_usage|model_scoped)$/i;
function redactStr(s) {
  return s
    .replace(/[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}/g, '<redacted-email>')
    .replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi, '<redacted-uuid>')
    .replace(/[A-Za-z]:\\\\?Users\\\\?[^\\"\s]+/g, '<redacted-path>');
}
function redact(v, key = '') {
  if (v === null || v === undefined) return v;
  if (typeof v === 'string') {
    if (key && SENSITIVE_KEY.test(key) && !SAFE_KEY.test(key)) return '<redacted>';
    return redactStr(v);
  }
  if (Array.isArray(v)) return v.map((x) => redact(x, key));
  if (typeof v === 'object') {
    const o = {};
    for (const [k, x] of Object.entries(v)) o[k] = redact(x, k);
    return o;
  }
  return v;
}

const log = fs.openSync(logFile, 'w');
const t0 = performance.now();
const ms = () => Math.round(performance.now() - t0);
function rec(kind, payload) {
  fs.writeSync(log, JSON.stringify({ t_ms: ms(), kind, ...payload }) + '\n');
}

rec('meta', { mode, args, exe: '<claude.exe>', cwd: '<scratch>' });
const child = spawn(exe, args, { cwd, env, stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true });
rec('spawned', { pid_present: !!child.pid });

let firstStdout = null;
let getUsageSent = 0;
let done = false;
const pending = new Map(); // request_id -> subtype
const summary = { mode, first_stdout_ms: null, init_response_ms: null, get_usage_response_ms: null, get_usage_calls: 0, event_types: {} };

function send(obj) {
  const line = JSON.stringify(obj) + '\n';
  child.stdin.write(line);
  rec('stdin', { line: obj });
}
function sendGetUsage() {
  if (getUsageSent >= 1) return; // one per process (global budget enforced by the operator: max 3)
  getUsageSent++;
  summary.get_usage_calls = getUsageSent;
  const id = 'gu' + Math.random().toString(36).slice(2, 10);
  pending.set(id, 'get_usage');
  send({ type: 'control_request', request_id: id, request: { subtype: 'get_usage', skip_behaviors: true } });
}
function sendInitialize() {
  const id = 'in' + Math.random().toString(36).slice(2, 10);
  pending.set(id, 'initialize');
  // Mirrors the SDK's initialize payload with every optional field left undefined (dropped by JSON.stringify).
  send({ type: 'control_request', request_id: id, request: { subtype: 'initialize' } });
}

function finish(reason) {
  if (done) return;
  done = true;
  summary.end_reason = reason;
  summary.end_ms = ms();
  rec('finish', { reason });
  try { child.stdin.end(); } catch {}
  try { child.kill(); } catch {}
  setTimeout(() => {
    fs.writeSync(log, JSON.stringify({ t_ms: ms(), kind: 'summary', summary }) + '\n');
    fs.closeSync(log);
    process.stdout.write(JSON.stringify(summary) + '\n');
    process.exit(0);
  }, 1500);
}

const rl = readline.createInterface({ input: child.stdout });
rl.on('line', (line) => {
  if (firstStdout === null) { firstStdout = ms(); summary.first_stdout_ms = firstStdout; }
  let obj = null;
  try { obj = JSON.parse(line); } catch {}
  if (!obj) { rec('stdout_raw', { line: redactStr(line).slice(0, 4000) }); return; }
  const typeKey = obj.type + (obj.subtype ? ':' + obj.subtype : '') + (obj.response?.subtype ? ':' + obj.response.subtype : '');
  summary.event_types[typeKey] = (summary.event_types[typeKey] || 0) + 1;
  rec('stdout', { obj: redact(obj) });
  if (obj.type === 'control_response') {
    const r = obj.response || {};
    const sub = pending.get(r.request_id);
    if (sub === 'initialize') {
      summary.init_response_ms = ms();
      summary.init_response_subtype = r.subtype;
      if (r.subtype === 'error') summary.init_error = redactStr(String(r.error));
      sendGetUsage();
    } else if (sub === 'get_usage') {
      summary.get_usage_response_ms = ms();
      summary.get_usage_response_subtype = r.subtype;
      if (r.subtype === 'error') summary.get_usage_error = redactStr(String(r.error));
      finish('get_usage_response');
    }
  }
  if (/rate.?limit|429/i.test(line) && obj.type !== 'control_response') summary.rate_limit_hint = true;
});
const rle = readline.createInterface({ input: child.stderr });
rle.on('line', (line) => {
  rec('stderr', { line: redactStr(line).slice(0, 4000) });
  if (/rate.?limit|429/i.test(line)) summary.rate_limit_hint = true;
});
child.on('exit', (code, signal) => { rec('exit', { code, signal }); summary.exit_code = code; finish('child_exit'); });
child.on('error', (e) => { rec('spawn_error', { message: String(e) }); finish('spawn_error'); });

if (mode === 'noinit') sendGetUsage();
else sendInitialize();

setTimeout(() => finish('timeout_60s'), TIMEOUT_MS);
