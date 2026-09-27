import { readFileSync } from 'node:fs'
const c = JSON.parse(readFileSync(process.argv[2], 'utf8'))
const f = (s) => (s.count ? [s.count, s.min, s.p50, s.p95, s.max].join('/') : '0')
for (const l of c.perLog) console.log(l.label, JSON.stringify(l.counts), JSON.stringify(l.signalCounts), 'unpaired', JSON.stringify(l.unpaired), 'dup', l.duplicateSeqs, l.toolEnd.result, l.toolEnd.sumUs)
for (const [k, v] of Object.entries(c.combined.per)) console.log(k, 'n=' + v.signals, 'lag', f(v.readerLag), 'leg1', f(v.leg1), 'handle', f(v.handle), 'v2p', f(v.vendorToPickMs))
const L = c.combined.leg2
console.log('leg2 sig', f(L.signalHop), 'sigTurnId', f(L.signalHopTurnId), JSON.stringify(L.signalHopBySignal), 'ann', f(L.announceHop), JSON.stringify(L.announceByRun), 'comb', f(L.combined), 'hop', f(L.hopUs), 'hopTid', f(L.hopUsTurnId))
console.log('TOOL', JSON.stringify(c.combined.toolEnd))
console.log('ANS', JSON.stringify(c.combined.answerEnd))
for (const [k, v] of Object.entries(c.evidence)) console.log(k, JSON.stringify(v))
