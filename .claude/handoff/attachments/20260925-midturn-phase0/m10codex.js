// Phase 0b summary for codex M9/M10 logs: node m10codex.js <log.jsonl> [verbose]
const fs = require('fs');
const L = fs.readFileSync(process.argv[2], 'utf8').trim().split('\n').map((l) => JSON.parse(l));
const verbose = process.argv[3] === 'verbose';
for (const e of L) {
  if (e.m10summary) {
    const s = e.m10summary;
    console.log(`trial ${s.trial} d=${s.d} ${s.verdict} tok->tc=${s.tcMinusTok}ms steerWrite-tok=${s.steerWriteMinusTok} steerResp=${JSON.stringify(s.steerResp).slice(0, 80)} echoVsTc=${s.echoVsTc} echoTurn=${(s.echoTurnId || '').slice(-6)} msgs=${s.agentMessages.length} fullFound=${s.probeFullFound} idx=${s.probeFullIdxFromNewest} walk=${s.walk ? s.walk.stoppedBy + '/' + s.walk.pages + 'p ' + s.walk.ms + 'ms found=' + JSON.stringify(s.walk.found) : '-'}`);
    if (verbose || s.verdict === 'HIT') {
      console.log('   turn=' + s.turnId.slice(-6) + ' tokChunk=' + s.tokChunk + ' tcChunk=' + s.tcChunk + ' tcT=' + s.tcT + ' echoS=' + s.echoStartedT + ' echoC=' + s.echoCompletedT + ' probeSent=' + s.probeSentT + ' probeFullResp=' + s.probeFullRespT);
      console.log('   fullTop=' + JSON.stringify(s.probeFullTop.map((x) => `${(x.turnId || '').slice(-6)}:${x.type}:${(x.clientId || '').slice(0, 8)}`)));
      if (s.walk) console.log('   walkPages=' + JSON.stringify(s.walk.pageTurns));
    }
  }
  if (e.m9summary) console.log('M9 ' + JSON.stringify(e.m9summary));
  if (e.m10tally) console.log('tally ' + JSON.stringify(e.m10tally) + ' hits=' + e.hits);
  if (e.error) console.log('ERROR ' + e.error);
}
