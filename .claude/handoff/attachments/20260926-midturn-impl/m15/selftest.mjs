// Self-test for parse-handover-trace.mjs against fixture-sample.log. Run: node selftest.mjs
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import assert from 'node:assert/strict';
import { analyze, parseLine, render, exitCode, stats } from './parse-handover-trace.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const r = analyze(readFileSync(join(here, 'fixture-sample.log'), 'utf8'));

// counts — the header and the unrelated-target line are ignored
assert.equal(r.counts.traceLines, 14);
assert.equal(r.counts.signals, 6);
assert.equal(r.counts.wakes, 4);
assert.equal(r.counts.coalescedWakes, 1);
assert.equal(r.counts.steersSignalHop, 2);
assert.equal(r.counts.steersAnnounceHop, 2);
assert.equal(r.counts.malformed, 0);
assert.deepEqual(r.signalCounts, { tool_end: 3, answer_end: 1, token_usage: 1, turn_id: 1 });

// reader lag / leg 1 per signal
assert.deepEqual(r.perSignal.tool_end.readerLag, { count: 3, min: 100, p50: 300, p95: 1200, max: 1200 });
assert.deepEqual(r.perSignal.tool_end.leg1, { count: 2, min: 150, p50: 150, p95: 400, max: 400 });
assert.equal(r.perSignal.answer_end.leg1.max, 90);
assert.equal(r.perSignal.turn_id.leg1.max, 60);
assert.equal(r.perSignal.turn_id.vendorToPickMs.count, 0); // response line: no emitted_ms
assert.equal(r.perSignal.tool_end.vendorToPickMs.max, 3);

// leg 2
assert.deepEqual(r.leg2.signalHop, { count: 2, min: 700, p50: 700, p95: 1100, max: 1100 });
assert.deepEqual(r.leg2.announceHop, { count: 2, min: 500, p50: 500, p95: 900, max: 900 });
assert.equal(r.leg2.combined.max, 1100);
assert.equal(r.leg2.signalHopTotal.max, 1180);

// verdicts
assert.equal(r.toolEnd.result, 'PASS');
assert.equal(r.toolEnd.sumUs, 400 + 1200 + 1100);
assert.equal(r.toolEnd.dominant, 'readerLag');
assert.equal(r.answerEnd.sumUs, 90 + 80 + 1100);
assert.equal(exitCode(r), 0);

// pairing: seq 2 covered by the coalesced wake of seq 1; seq 5 unpaired; wake-before-signal (seq 3) pairs
assert.deepEqual(r.coveredByCoalesce.map((x) => x.seq), [2]);
assert.deepEqual(r.unpaired.signals.map((x) => x.seq), [5]);
assert.equal(r.unpaired.wakes.length, 0);
assert.equal(r.unpaired.steers.length, 0);
assert.deepEqual(r.duplicateSeqs, []);

// too few samples is reported (fixture is tiny)
assert.equal(r.sampleWarnings.length, 3);

// ANSI-coloured stdout copy parses the same as the file sink
const ansi = '\x1b[2m2026-09-26T03:00:01.000300Z\x1b[0m \x1b[34mDEBUG\x1b[0m \x1b[2mengram::codex_handover\x1b[0m\x1b[2m:\x1b[0m codex handover: 라이터 깸 \x1b[3mphase\x1b[0m\x1b[2m=\x1b[0m"wake" \x1b[3msignal\x1b[0m\x1b[2m=\x1b[0m"tool_end" \x1b[3mseq\x1b[0m\x1b[2m=\x1b[0m7 \x1b[3mcoalesced\x1b[0m\x1b[2m=\x1b[0m0 \x1b[3mlag_us\x1b[0m\x1b[2m=\x1b[0m1 \x1b[3mwake_us\x1b[0m\x1b[2m=\x1b[0m42\r';
const p = parseLine(ansi);
assert.equal(p.ts, '2026-09-26T03:00:01.000300Z');
assert.equal(p.fields.phase, 'wake');
assert.equal(p.fields.wake_us, 42);

// FAIL path and INCOMPLETE path
const failText = [
  '2026-09-26T00:00:00.000000Z DEBUG engram::codex_handover: m phase="signal" signal="tool_end" seq=0 lag_us=5000 handle_us=1 recv_ms=1',
  '2026-09-26T00:00:00.000001Z DEBUG engram::codex_handover: m phase="wake" signal="tool_end" seq=0 coalesced=0 lag_us=5000 wake_us=500',
  '2026-09-26T00:00:00.000002Z DEBUG engram::codex_handover: m phase="steer" hop="announce" steer_us=600',
].join('\n');
const f = analyze(failText);
assert.equal(f.toolEnd.result, 'FAIL');
assert.equal(f.toolEnd.dominant, 'readerLag');
assert.equal(exitCode(f), 1);
const inc = analyze(failText.split('\n').slice(0, 2).join('\n'));
assert.equal(inc.toolEnd.result, 'INCOMPLETE');
assert.deepEqual(inc.toolEnd.missing, ['leg2']);
assert.equal(exitCode(inc), 2);

// hop="zone" (P8): counted apart, folded into the combined leg 2, no wake pairing; unknown hop stays malformed
const zoneText = [
  failText.split('\n')[0].replace('lag_us=5000', 'lag_us=100'),
  failText.split('\n')[1].replace('lag_us=5000', 'lag_us=100'),
  '2026-09-26T00:00:00.000003Z DEBUG engram::codex_handover: m phase="steer" hop="zone" steer_us=1500 hop_us=1900',
  '2026-09-26T00:00:00.000004Z DEBUG engram::codex_handover: m phase="steer" hop="bogus" steer_us=1',
].join('\n');
const z = analyze(zoneText);
assert.equal(z.counts.steersZoneHop, 1);
assert.equal(z.counts.malformed, 1);
assert.deepEqual(z.leg2.zoneHop, { count: 1, min: 1500, p50: 1500, p95: 1500, max: 1500 });
assert.equal(z.leg2.zoneHopTotal.max, 1900);
assert.equal(z.leg2.combined.max, 1500);
assert.equal(z.unpaired.steers.length, 0);
assert.equal(r.counts.steersZoneHop, 0); // pre-P8 fixture: no zone lines

assert.deepEqual(stats([]), { count: 0, min: null, p50: null, p95: null, max: null });
assert.ok(render(r).includes('Tool-end reaction: PASS'));

console.log('selftest OK');
