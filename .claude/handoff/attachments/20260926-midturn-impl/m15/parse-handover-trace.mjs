#!/usr/bin/env node
// M15 handover trace parser (TRD S21 mid-turn input queue §3-3 M15, pass line = TRD L161).
//
// Input: a daemon log written by tracing-subscriber's default `fmt` layer (engram-dashboard-base
// logging/mod.rs init_subscriber). Default "Full" format, one event per line:
//   <RFC3339 UTC micros> <LEVEL padded to 5> [<span>{<fields>}:...: ]<target>: <message> <k=v ...>
//   e.g. 2026-09-26T03:00:00.000100Z DEBUG engram::codex_handover: codex handover: 경계 신호 phase="signal" signal="tool_end" seq=0 ...
// &str fields are Debug-quoted ("..."), integers are bare, Option::None fields are absent.
// The file sink has ANSI off; the stdout copy may carry ANSI colour codes — they are stripped.
//
// Field contract = doc comment on HANDOVER_TRACE in
// crates/engram-dashboard-agent/src/backend/codex/transport.rs. Keep the names below in sync with it.
//
// Usage: node parse-handover-trace.mjs <daemon.log> [--json] [--extract <out.log>]
// Exit:  0 = PASS · 1 = FAIL · 2 = INCOMPLETE (a verdict term has no samples) · 3 = usage / IO error.
// Built-ins only.

import { readFileSync, writeFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

export const TARGET = 'engram::codex_handover';
export const LIMIT_US = 6000; // M7 window minimum (6 ms) — TRD L161
export const MIN_SAMPLES = 20; // plan §5 / TRD M15: N >= 20 per sample class
// steer hop labels. "zone" arrived in P8 (logs before it only carry the first two — still parse).
export const HOPS = ['signal', 'announce', 'zone'];

const ANSI_RE = /\x1b\[[0-9;]*[A-Za-z]/g;
const TS_RE = /^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z)\s+(TRACE|DEBUG|INFO|WARN|ERROR)\s/;
const KV_RE = /([A-Za-z_][A-Za-z0-9_]*)=("(?:[^"\\]|\\.)*"|\S+)/g;
const INT_FIELDS = new Set([
  'seq', 'lag_us', 'handle_us', 'recv_ms', 'emitted_ms', 'coalesced', 'wake_us', 'steer_us', 'hop_us',
]);

function unquote(v) {
  if (v.length >= 2 && v.startsWith('"') && v.endsWith('"')) {
    try { return JSON.parse(v); } catch { return v.slice(1, -1); }
  }
  return v;
}

/** One trace line -> { ts, level, fields } or null when the line is not ours. */
export function parseLine(raw) {
  const line = raw.replace(ANSI_RE, '').replace(/\r$/, '');
  const marker = ` ${TARGET}: `;
  let at = line.indexOf(marker);
  if (at < 0) {
    // target directly after the level, or after a span list ending in ": "
    const alt = line.indexOf(`${TARGET}: `);
    if (alt < 0) return null;
    at = alt - 1;
  }
  const rest = line.slice(at + marker.length);
  const fields = {};
  for (const m of rest.matchAll(KV_RE)) {
    const key = m[1];
    let val = unquote(m[2]);
    if (INT_FIELDS.has(key)) {
      const n = Number(val);
      val = Number.isFinite(n) ? n : NaN;
    }
    fields[key] = val;
  }
  if (!fields.phase) return null;
  const ts = TS_RE.exec(line);
  return { ts: ts ? ts[1] : null, level: ts ? ts[2] : null, fields, raw: line };
}

/** Nearest-rank percentile of a sorted array. */
function pct(sorted, p) {
  if (sorted.length === 0) return null;
  const rank = Math.max(1, Math.ceil((p / 100) * sorted.length));
  return sorted[rank - 1];
}

export function stats(values) {
  const v = values.filter((x) => Number.isFinite(x)).sort((a, b) => a - b);
  if (v.length === 0) return { count: 0, min: null, p50: null, p95: null, max: null };
  return { count: v.length, min: v[0], p50: pct(v, 50), p95: pct(v, 95), max: v[v.length - 1] };
}

/** Analyse the whole log text. Pure — returns a report object. */
export function analyze(text) {
  const lines = text.split('\n');
  const signals = [];
  const wakes = [];
  const steers = [];
  const malformed = [];
  const traceLines = [];
  lines.forEach((raw, i) => {
    const p = parseLine(raw);
    if (!p) return;
    const rec = { ...p.fields, line: i + 1, ts: p.ts };
    traceLines.push(p.raw);
    if (rec.phase === 'signal') {
      if (!Number.isFinite(rec.seq) || !Number.isFinite(rec.lag_us)) malformed.push(rec);
      else signals.push(rec);
    } else if (rec.phase === 'wake') {
      if (!Number.isFinite(rec.seq) || !Number.isFinite(rec.wake_us)) malformed.push(rec);
      else wakes.push(rec);
    } else if (rec.phase === 'steer') {
      const sigHop = rec.hop === 'signal';
      if (!Number.isFinite(rec.steer_us) || (sigHop && !Number.isFinite(rec.seq)) ||
          !HOPS.includes(rec.hop)) malformed.push(rec);
      else steers.push(rec);
    } else {
      malformed.push(rec);
    }
  });

  // seq is per channel (State::next_signal_seq starts at 0 per channel/incarnation). Duplicates mean
  // the log holds more than one codex channel — pairing becomes k-th-occurrence, i.e. ambiguous.
  const bySeq = new Map();
  for (const s of signals) {
    if (!bySeq.has(s.seq)) bySeq.set(s.seq, []);
    bySeq.get(s.seq).push(s);
  }
  const duplicateSeqs = [...bySeq.entries()].filter(([, l]) => l.length > 1).map(([k]) => k);
  const take = (seq) => {
    const l = bySeq.get(seq);
    if (!l) return null;
    return l.find((s) => !s._used) ?? null;
  };

  // Pair wakes to signals (order-independent: a wake line may precede its signal line).
  const unpairedWakes = [];
  const coveredSignals = [];
  const wakeSeqs = new Map();
  for (const w of wakes) {
    const s = take(w.seq);
    if (!s) { unpairedWakes.push(w); } else { s._used = 'wake'; w._signal = s; }
    wakeSeqs.set(w.seq, (wakeSeqs.get(w.seq) ?? 0) + 1);
    const c = Number.isFinite(w.coalesced) ? w.coalesced : 0;
    for (let k = 1; k <= c; k++) {
      const cs = take(w.seq + k);
      if (cs) { cs._used = 'coalesced'; coveredSignals.push(cs); }
    }
  }
  const unpairedSignals = signals.filter((s) => !s._used);

  const sigSteers = steers.filter((s) => s.hop === 'signal');
  const annSteers = steers.filter((s) => s.hop === 'announce');
  // hop="zone" (P8): a held answer-segment item released by an Other item starting — no boundary signal, no wake line.
  const zoneSteers = steers.filter((s) => s.hop === 'zone');
  const unpairedSteers = sigSteers.filter((s) => !wakeSeqs.has(s.seq));

  const kinds = ['tool_end', 'answer_end', 'token_usage', 'turn_id'];
  const signalCounts = Object.fromEntries(kinds.map((k) => [k, signals.filter((s) => s.signal === k).length]));
  const otherKinds = [...new Set(signals.map((s) => s.signal).filter((k) => !kinds.includes(k)))];

  const perSignal = {};
  for (const k of [...kinds, ...otherKinds]) {
    const sg = signals.filter((s) => s.signal === k);
    perSignal[k] = {
      readerLag: stats(sg.map((s) => s.lag_us)),
      handle: stats(sg.map((s) => s.handle_us)),
      vendorToPickMs: stats(sg.filter((s) => Number.isFinite(s.emitted_ms) && Number.isFinite(s.recv_ms))
        .map((s) => s.recv_ms - s.emitted_ms)),
      leg1: stats(wakes.filter((w) => w.signal === k).map((w) => w.wake_us)),
    };
  }

  const leg2Signal = stats(sigSteers.map((s) => s.steer_us));
  const leg2Announce = stats(annSteers.map((s) => s.steer_us));
  const leg2Zone = stats(zoneSteers.map((s) => s.steer_us));
  const leg2All = stats(steers.map((s) => s.steer_us));
  const hopSignal = stats(sigSteers.map((s) => s.hop_us));
  const hopZone = stats(zoneSteers.map((s) => s.hop_us));

  const verdictFor = (kind) => {
    const terms = {
      leg1: perSignal[kind]?.leg1.max ?? null,
      readerLag: perSignal[kind]?.readerLag.max ?? null,
      leg2: leg2All.max,
    };
    const missing = Object.entries(terms).filter(([, v]) => v === null).map(([k]) => k);
    if (missing.length) return { terms, sumUs: null, result: 'INCOMPLETE', missing, dominant: null };
    const sumUs = terms.leg1 + terms.readerLag + terms.leg2;
    const dominant = Object.entries(terms).sort((a, b) => b[1] - a[1])[0][0];
    return { terms, sumUs, result: sumUs < LIMIT_US ? 'PASS' : 'FAIL', missing, dominant };
  };

  const sampleWarnings = [];
  const need = (label, n) => { if (n < MIN_SAMPLES) sampleWarnings.push(`${label}: ${n} < ${MIN_SAMPLES}`); };
  need('tool_end wakes (leg 1)', perSignal.tool_end.leg1.count);
  need('signal-hop steers (leg 2, turn id hop)', leg2Signal.count);
  need('announce-hop steers (leg 2, output flood)', leg2Announce.count);

  return {
    counts: {
      traceLines: traceLines.length,
      signals: signals.length,
      wakes: wakes.length,
      coalescedWakes: wakes.filter((w) => (w.coalesced ?? 0) > 0).length,
      steersSignalHop: sigSteers.length,
      steersAnnounceHop: annSteers.length,
      steersZoneHop: zoneSteers.length,
      malformed: malformed.length,
    },
    signalCounts: { ...signalCounts, ...Object.fromEntries(otherKinds.map((k) => [k, signals.filter((s) => s.signal === k).length])) },
    perSignal,
    leg2: { signalHop: leg2Signal, announceHop: leg2Announce, zoneHop: leg2Zone, combined: leg2All, signalHopTotal: hopSignal, zoneHopTotal: hopZone },
    toolEnd: verdictFor('tool_end'),
    answerEnd: verdictFor('answer_end'), // compared with the M16 window, not a verdict
    unpaired: {
      signals: unpairedSignals.map(pick),
      wakes: unpairedWakes.map(pick),
      steers: unpairedSteers.map(pick),
    },
    coveredByCoalesce: coveredSignals.map(pick),
    duplicateSeqs,
    malformed: malformed.map(pick),
    sampleWarnings,
    traceLines,
  };
}

function pick(r) {
  return { line: r.line, phase: r.phase, signal: r.signal ?? null, seq: r.seq ?? null, hop: r.hop ?? null };
}

function fmtStats(label, s) {
  const f = (v) => (v === null ? '-' : String(v));
  return `  ${label.padEnd(34)} n=${String(s.count).padStart(4)}  min=${f(s.min).padStart(7)}  p50=${f(s.p50).padStart(7)}  p95=${f(s.p95).padStart(7)}  max=${f(s.max).padStart(7)}`;
}

export function render(r) {
  const out = [];
  out.push(`M15 handover trace — ${r.counts.traceLines} trace lines (${r.counts.signals} signal / ${r.counts.wakes} wake / ${r.counts.steersSignalHop + r.counts.steersAnnounceHop} steer, ${r.counts.malformed} malformed)`);
  out.push('');
  out.push('Signal counts:');
  for (const [k, n] of Object.entries(r.signalCounts)) out.push(`  ${k.padEnd(12)} ${n}`);
  out.push(`  coalesced wakes: ${r.counts.coalescedWakes} (signals covered by them: ${r.coveredByCoalesce.length})`);
  out.push('');
  out.push('Per signal (µs; vendor→pick in ms, cross-process wall clock — informational):');
  for (const [k, p] of Object.entries(r.perSignal)) {
    if (p.readerLag.count === 0 && p.leg1.count === 0) continue;
    out.push(` [${k}]`);
    out.push(fmtStats('reader lag (lag_us)', p.readerLag));
    out.push(fmtStats('leg 1 signal→wake (wake_us)', p.leg1));
    out.push(fmtStats('handle (handle_us)', p.handle));
    if (p.vendorToPickMs.count) out.push(fmtStats('vendor emit→pick (ms)', p.vendorToPickMs));
  }
  out.push('');
  out.push('Leg 2 wake→steer written (steer_us):');
  out.push(fmtStats('signal hop', r.leg2.signalHop));
  out.push(fmtStats('announce hop', r.leg2.announceHop));
  out.push(fmtStats('zone hop (answer→other start)', r.leg2.zoneHop));
  out.push(fmtStats('combined (max used in verdict)', r.leg2.combined));
  out.push(fmtStats('signal hop whole (hop_us)', r.leg2.signalHopTotal));
  out.push(fmtStats('zone hop whole (hop_us)', r.leg2.zoneHopTotal));
  out.push('');
  const v = (name, x, note) => {
    if (x.result === 'INCOMPLETE') {
      out.push(`${name}: INCOMPLETE — no samples for ${x.missing.join(', ')}${note}`);
    } else {
      out.push(`${name}: ${note ? 'sum' : x.result}${note} — max leg1 ${x.terms.leg1} + max reader lag ${x.terms.readerLag} + max leg2 ${x.terms.leg2} = ${x.sumUs} µs (limit < ${LIMIT_US} µs), dominant term: ${x.dominant}`);
    }
  };
  v('Tool-end reaction', r.toolEnd, '');
  v('Answer-end reaction', r.answerEnd, ' (compare with M16 window — not a verdict)');
  out.push('');
  const u = r.unpaired;
  out.push(`Unpaired: ${u.signals.length} signal, ${u.wakes.length} wake, ${u.steers.length} signal-hop steer`);
  for (const x of [...u.signals, ...u.wakes, ...u.steers]) out.push(`  line ${x.line}: phase=${x.phase} signal=${x.signal} seq=${x.seq}${x.hop ? ` hop=${x.hop}` : ''}`);
  if (r.duplicateSeqs.length) out.push(`WARNING: duplicate signal seq(s) ${r.duplicateSeqs.join(',')} — log holds more than one codex channel; pairing is by occurrence order and may be wrong. Measure with one codex agent per log.`);
  for (const w of r.sampleWarnings) out.push(`WARNING: too few samples — ${w}`);
  for (const m of r.malformed) out.push(`WARNING: malformed trace line ${m.line} (phase=${m.phase})`);
  return out.join('\n');
}

export function exitCode(r) {
  if (r.toolEnd.result === 'PASS') return 0;
  if (r.toolEnd.result === 'FAIL') return 1;
  return 2;
}

function main(argv) {
  const args = argv.slice(2);
  const json = args.includes('--json');
  const ei = args.indexOf('--extract');
  const extract = ei >= 0 ? args[ei + 1] : null;
  const file = args.find((a, i) => !a.startsWith('--') && !(ei >= 0 && i === ei + 1));
  if (!file || (ei >= 0 && !extract)) {
    console.error('usage: node parse-handover-trace.mjs <daemon.log> [--json] [--extract <out.log>]');
    return 3;
  }
  let text;
  try { text = readFileSync(file, 'utf8'); } catch (e) { console.error(`cannot read ${file}: ${e.message}`); return 3; }
  const r = analyze(text);
  if (extract) writeFileSync(extract, r.traceLines.join('\n') + '\n', 'utf8');
  if (json) { const { traceLines, ...rest } = r; console.log(JSON.stringify(rest, null, 2)); } else console.log(render(r));
  return exitCode(r);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  process.exitCode = main(process.argv);
}
