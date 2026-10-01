// B5 processor — raw `b5codex.cjs` log -> README-conformant fixture lines, or a redacted copy of the whole log
// (`crates/engram-dashboard-agent/src/backend/codex/fixtures/README.md` 「공통 가공」).
// usage:
//   node b5fixture.cjs fixture <log.jsonl> <out.jsonl> <fromLine> <toLine> <cwd> [outsideDir]
//     fromLine/toLine = 0-based line numbers of the raw log (inclusive) — the README 「줄 범위」.
//     Keeps `dir:"out"` lines only (the `line` payload), drops the README noise methods.
//   node b5fixture.cjs log <log.jsonl> <out.jsonl> <cwd> [outsideDir]
//     Whole log, every envelope kept, one output line per input line (line numbers stay valid).
// Redaction (both modes, inside every string / key):
//   cwd -> C:\work\proj · outsideDir -> C:\work\outside · OS user home -> C:\home\user · host name -> HOST
//   (paths separator-agnostic and case-insensitive; a path written with doubled backslashes keeps them doubled)
//   key `installationId` -> zero uuid · key `planType` -> "redacted"
// Fails (exit 2, nothing written) if any identifying leftover survives: user name · host name · session scratchpad ·
// installation id · e-mail address.
'use strict';
const fs = require('fs'), os = require('os'), path = require('path');
const mode = process.argv[2];
let logPath, outPath, from, to, cwd, outside;
if (mode === 'fixture') {
  [logPath, outPath] = process.argv.slice(3, 5);
  from = Number(process.argv[5]); to = Number(process.argv[6]);
  [cwd, outside] = process.argv.slice(7);
} else if (mode === 'log') {
  [logPath, outPath, cwd, outside] = process.argv.slice(3);
} else {
  console.error('mode must be fixture | log'); process.exit(1);
}
const NOISE = new Set(['mcpServer/startupStatus/updated', 'skills/changed', 'account/updated', 'account/rateLimits/updated', 'remoteControl/status/changed', 'thread/started']);
const ZERO_UUID = '00000000-0000-0000-0000-000000000000';

const home = os.homedir();
const host = os.hostname();
const rules = [[cwd, 'C:\\work\\proj']];
if (outside) rules.push([outside, 'C:\\work\\outside']);
rules.push([home, 'C:\\home\\user']);
const esc = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
const compiled = rules.map(([src, dst]) => {
  const segs = path.resolve(src).split(/[\\/]+/).filter(Boolean).map(esc);
  // capture the first separator run so the replacement reuses the same style (\ · \\ · /)
  const re = new RegExp(segs[0] + '([\\\\/]+)' + segs.slice(1).join('[\\\\/]+'), 'gi');
  const dstSegs = dst.split('\\');
  return (s) => s.replace(re, (_m, sep) => dstSegs.join(sep));
});
const hostRe = new RegExp(esc(host), 'gi');
compiled.push((s) => s.replace(hostRe, 'HOST'));
const seenInstallIds = new Set();
const fix = (v, key) => {
  if (key === 'installationId' && typeof v === 'string') { seenInstallIds.add(v); return ZERO_UUID; }
  if (key === 'planType' && typeof v === 'string') return 'redacted';
  if (typeof v === 'string') return compiled.reduce((s, f) => f(s), v);
  if (Array.isArray(v)) return v.map((x) => fix(x));
  if (v && typeof v === 'object') { const o = {}; for (const [k, x] of Object.entries(v)) o[k] = fix(x, k); return o; }
  return v;
};

const raw = fs.readFileSync(logPath, 'utf8').split('\n');
const out = [];
const kept = [];
if (mode === 'fixture') {
  for (let i = from; i <= to; i++) {
    if (!raw[i] || !raw[i].trim()) continue;
    const e = JSON.parse(raw[i]);
    if (e.dir !== 'out' || typeof e.line !== 'object') continue;
    if (e.line.method && NOISE.has(e.line.method)) continue;
    out.push(JSON.stringify(fix(e.line)));
    kept.push(i);
  }
} else {
  for (let i = 0; i < raw.length; i++) {
    if (!raw[i].trim()) { out.push(raw[i]); continue; }
    out.push(JSON.stringify(fix(JSON.parse(raw[i]))));
    kept.push(i);
  }
}
const text = mode === 'fixture' ? out.join('\n') + '\n' : out.join('\n');
const low = text.toLowerCase();
const words = [os.userInfo().username, host, 'scratchpad', 'I--Engram', ...seenInstallIds];
if (mode === 'fixture') words.push('AppData');
const leftovers = words.filter((w) => w && low.includes(w.toLowerCase()));
const mail = text.match(/[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}/g);
if (mail) leftovers.push('e-mail: ' + [...new Set(mail)].join(', '));
if (leftovers.length) { console.error('LEFTOVER identifying text: ' + leftovers.join(' | ')); process.exit(2); }
fs.writeFileSync(outPath, text);
console.log(`${mode}: wrote ${mode === 'fixture' ? out.length : raw.length} lines -> ${outPath} (raw lines ${kept[0]}..${kept[kept.length - 1]}) · identifying-value check passed`);
