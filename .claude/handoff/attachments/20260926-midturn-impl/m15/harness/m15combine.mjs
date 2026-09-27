#!/usr/bin/env node
// Combine several M15 daemon logs (one codex channel each) using the parser's own functions.
//   node m15combine.mjs <parser.mjs> <label=log> [<label=log> ...]
// Per-log: parser analyze() (pairing, unpaired, malformed, per-log verdict).
// Combined: union of raw values -> same stats() + same verdict formula (max leg1 + max lag + max leg2 < LIMIT_US).
import { readFileSync } from 'node:fs'
import { pathToFileURL } from 'node:url'

const [, , parserPath, ...pairs] = process.argv
const P = await import(pathToFileURL(parserPath).href)
const runs = pairs.map((p) => {
  const i = p.indexOf('=')
  return { label: p.slice(0, i), path: p.slice(i + 1) }
})

const raw = { signals: [], wakes: [], steers: [] }
const perLog = []
for (const r of runs) {
  const text = readFileSync(r.path, 'utf8')
  const a = P.analyze(text)
  perLog.push({
    label: r.label,
    counts: a.counts,
    signalCounts: a.signalCounts,
    toolEnd: a.toolEnd,
    answerEnd: a.answerEnd,
    unpaired: { signals: a.unpaired.signals.length, wakes: a.unpaired.wakes.length, steers: a.unpaired.steers?.length },
    duplicateSeqs: a.duplicateSeqs?.length ?? a.warnings?.duplicateSeqs?.length,
  })
  for (const line of text.split('\n')) {
    const p = P.parseLine(line)
    if (!p) continue
    const f = { ...p.fields, run: r.label, ts: p.ts }
    if (f.phase === 'signal') raw.signals.push(f)
    else if (f.phase === 'wake') raw.wakes.push(f)
    else if (f.phase === 'steer') raw.steers.push(f)
  }
}

const kinds = ['tool_end', 'answer_end', 'token_usage', 'turn_id']
const per = {}
for (const k of kinds) {
  const sg = raw.signals.filter((s) => s.signal === k)
  per[k] = {
    signals: sg.length,
    readerLag: P.stats(sg.map((s) => s.lag_us)),
    handle: P.stats(sg.map((s) => s.handle_us)),
    leg1: P.stats(raw.wakes.filter((w) => w.signal === k).map((w) => w.wake_us)),
    vendorToPickMs: P.stats(
      sg.filter((s) => Number.isFinite(s.emitted_ms) && Number.isFinite(s.recv_ms)).map((s) => s.recv_ms - s.emitted_ms),
    ),
  }
}
const sig = raw.steers.filter((s) => s.hop === 'signal')
const ann = raw.steers.filter((s) => s.hop === 'announce')
const leg2 = {
  signalHop: P.stats(sig.map((s) => s.steer_us)),
  signalHopTurnId: P.stats(sig.filter((s) => s.signal === 'turn_id').map((s) => s.steer_us)),
  signalHopBySignal: Object.fromEntries(kinds.map((k) => [k, sig.filter((s) => s.signal === k).length])),
  announceHop: P.stats(ann.map((s) => s.steer_us)),
  announceByRun: Object.fromEntries(runs.map((r) => [r.label, ann.filter((s) => s.run === r.label).length])),
  combined: P.stats(raw.steers.map((s) => s.steer_us)),
  hopUs: P.stats(sig.map((s) => s.hop_us)),
  hopUsTurnId: P.stats(sig.filter((s) => s.signal === 'turn_id').map((s) => s.hop_us)),
}
const verdict = (k) => {
  const terms = { leg1: per[k].leg1.max, readerLag: per[k].readerLag.max, leg2: leg2.combined.max }
  if (Object.values(terms).some((v) => v === null)) return { terms, result: 'INCOMPLETE' }
  const sumUs = terms.leg1 + terms.readerLag + terms.leg2
  const dominant = Object.entries(terms).sort((a, b) => b[1] - a[1])[0][0]
  return { terms, sumUs, result: sumUs < P.LIMIT_US ? 'PASS' : 'FAIL', dominant }
}
// top readerLag / leg1 lines for tool_end (evidence)
const top = (arr, f, n = 5) =>
  [...arr].sort((a, b) => f(b) - f(a)).slice(0, n).map((x) => ({ run: x.run, ts: x.ts, signal: x.signal, seq: x.seq, lag_us: x.lag_us, handle_us: x.handle_us, wake_us: x.wake_us, steer_us: x.steer_us, hop: x.hop }))
const out = {
  perLog,
  combined: { per, leg2, toolEnd: verdict('tool_end'), answerEnd: verdict('answer_end') },
  evidence: {
    topToolEndLag: top(raw.signals.filter((s) => s.signal === 'tool_end'), (s) => s.lag_us),
    topToolEndLeg1: top(raw.wakes.filter((w) => w.signal === 'tool_end'), (w) => w.wake_us),
    topSteer: top(raw.steers, (s) => s.steer_us),
    topAnswerEndLag: top(raw.signals.filter((s) => s.signal === 'answer_end'), (s) => s.lag_us),
  },
}
console.log(JSON.stringify(out, null, 1))
