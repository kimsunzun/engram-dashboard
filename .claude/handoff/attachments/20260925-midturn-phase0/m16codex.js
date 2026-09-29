// M16 summary for codex logs (codex_harness.js scenario M16): node m16codex.cjs <log.jsonl>... [verbose]
// Per attempt: mode · verdict · gap (answer-end signal -> following tokenUsage/updated) · steer offsets.
// Per case: hits/attempts by mode, gap min/median/max, largest hit offset from the signal, smallest miss offset.
const fs = require('fs');
const args = process.argv.slice(2);
const verbose = args.includes('verbose');
const files = args.filter((a) => a !== 'verbose');
const med = (a) => { const b = [...a].sort((x, y) => x - y); return b.length ? b[Math.floor(b.length / 2)] : null; };
const byCase = {};
const extra = {}; // case -> { quiet20: [], lastDelta: [], lateOrder: [] }
for (const f of files) {
  const L = fs.readFileSync(f, 'utf8').trim().split('\n').map((l) => JSON.parse(l));
  // raw-line extras: quiet period before each signal (reader-lag input for M15) · -32600 vs turn/completed order
  const outs = L.filter((e) => e.dir === 'out');
  const trialLines = {}; let tr = null;
  for (const e of L) { if (e.trial !== undefined && e.ids) { tr = e.trial; trialLines[tr] = []; continue; } if (tr != null && e.line && typeof e.line === 'object') trialLines[tr].push(e); }
  for (const e of L) {
    if (!e.m16summary || e.m16summary.sigT == null) continue;
    const s = e.m16summary, x = (extra[s.case] = extra[s.case] || { quiet20: [], lastDelta: [], lateOrder: [] });
    const before = outs.filter((o) => o.t < s.sigT && o.t >= s.sigT - 5000);
    x.quiet20.push(before.filter((o) => o.t >= s.sigT - 20).length);
    const lastD = [...before].reverse().find((o) => /delta$/.test(o.line?.method || ''));
    if (lastD) x.lastDelta.push(Math.round((s.sigT - lastD.t) * 10) / 10);
    if (s.verdict === 'late') {
      const tl = trialLines[s.trial] || [];
      const err = tl.find((o) => o.dir === 'out' && o.line.error?.code === -32600), tc = tl.find((o) => o.dir === 'out' && o.line.method === 'turn/completed');
      if (err && tc) x.lateOrder.push(Math.round((err.t - tc.t) * 10) / 10);
    }
  }
  for (const e of L) {
    if (e.error) console.log('ERROR ' + e.error);
    if (e.cliVersion) console.log(`${f.split(/[\\/]/).pop()}: cliVersion=${e.cliVersion}`);
    if (!e.m16summary) continue;
    const s = e.m16summary;
    (byCase[s.case] = byCase[s.case] || []).push(s);
    console.log(`${s.case} #${s.trial} ${s.mode}${s.jit != null ? '+' + s.jit : ''}${s.dPlanned != null ? ' d=' + s.dPlanned : ''} ${s.verdict}` +
      ` gap=${s.gapSigToTok} steer-sig=${s.steerMinusSig} steer-tok=${s.steerMinusTok} resp=${JSON.stringify(s.steerResp ?? null).slice(0, 60)}` +
      ` echo-sig=${s.echoMinusSig} tc-sig=${s.tcMinusSig} deltas=${s.sigItemDeltas}/${s.sigItemDeltaChunks}ch sameChunk=${s.tokSameChunkAsSig} phase=${s.sigPhase} done=${s.doneAtSig} toks=${s.tokCount}`);
    if (verbose) console.log('   msgs=' + JSON.stringify(s.msgsAll) + ' postEcho=' + JSON.stringify(s.postEchoMsgs));
  }
}
for (const [cs, arr] of Object.entries(byCase)) {
  const modes = {};
  for (const s of arr) { const m = (modes[s.mode] = modes[s.mode] || { n: 0, hit: 0, v: {} }); m.n++; if (s.verdict === 'HIT') m.hit++; m.v[s.verdict] = (m.v[s.verdict] || 0) + 1; }
  const gaps = arr.map((s) => s.gapSigToTok).filter((x) => x != null);
  const hitOff = arr.filter((s) => s.verdict === 'HIT').map((s) => s.steerMinusSig);
  const missOff = arr.filter((s) => s.verdict !== 'HIT' && s.steerMinusSig != null).map((s) => s.steerMinusSig);
  const hitTok = arr.filter((s) => s.verdict === 'HIT' && s.steerMinusTok != null).map((s) => s.steerMinusTok);
  const missTok = arr.filter((s) => s.verdict !== 'HIT' && s.steerMinusTok != null).map((s) => s.steerMinusTok);
  console.log(`\n== case ${cs}: attempts=${arr.length} hits=${arr.filter((s) => s.verdict === 'HIT').length}`);
  for (const [m, v] of Object.entries(modes)) console.log(`   ${m}: ${v.hit}/${v.n} ${JSON.stringify(v.v)}`);
  if (gaps.length) console.log(`   gap sig->tok ms: min=${Math.min(...gaps)} med=${med(gaps)} max=${Math.max(...gaps)} (n=${gaps.length})`);
  if (hitOff.length) console.log(`   hit steer-sig: max=${Math.max(...hitOff)} · hit steer-tok: max=${hitTok.length ? Math.max(...hitTok) : '-'}`);
  if (missOff.length) console.log(`   miss steer-sig: min=${Math.min(...missOff)} · miss steer-tok: min=${missTok.length ? Math.min(...missTok) : '-'}`);
  // echo - tokenUsage for hits: steer landed before the tokenUsage notice vs after it (second pending check path)
  const et = (pred) => arr.filter((s) => s.verdict === 'HIT' && s.steerMinusTok != null && pred(s.steerMinusTok)).map((s) => Math.round((s.echoMinusSig - s.gapSigToTok) * 10) / 10);
  const eb = et((d) => d <= 0), ea = et((d) => d > 0);
  if (eb.length) console.log(`   hit echo-tok (steer before tok): ${Math.min(...eb)}..${Math.max(...eb)} n=${eb.length}`);
  if (ea.length) console.log(`   hit echo-tok (steer after tok):  ${Math.min(...ea)}..${Math.max(...ea)} n=${ea.length}`);
  const x = extra[cs];
  if (x) {
    console.log(`   lines in the 20 ms before the signal: max=${Math.max(...x.quiet20)} · last delta -> signal ms: min=${Math.min(...x.lastDelta)} max=${Math.max(...x.lastDelta)}`);
    if (x.lateOrder.length) console.log(`   late: -32600 minus turn/completed ms = ${JSON.stringify(x.lateOrder)} (negative = error came first)`);
  }
}
