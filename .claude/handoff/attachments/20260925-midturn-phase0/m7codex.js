const fs=require('fs');const L=fs.readFileSync(process.argv[2],'utf8').trim().split('\n').map(JSON.parse);
let trials=[],cur=null;
for(const e of L){
 if(e.dir==='meta'&&e.trial!==undefined&&e.ids){cur={k:e.trial,B:e.ids.B,msgs:[]};trials.push(cur);continue;}
 if(!cur)continue;
 if(e.dir==='in'&&e.line&&e.line.method==='turn/steer')cur.steer=e.t;
 if(e.dir!=='out'||typeof e.line!=='object')continue;const x=e.line;
 if(x.method==='item/completed'&&x.params.item.type==='commandExecution'&&!cur.cmdDone)cur.cmdDone=e.t;
 if(x.method==='item/started'&&x.params.item.type==='userMessage'&&x.params.item.clientId===cur.B)cur.echo=e.t;
 if(cur.cmdDone&&x.method==='item/started'&&['agentMessage','reasoning'].includes(x.params.item.type)&&!cur.nextStart)cur.nextStart=e.t;
 if(x.method==='item/completed'&&x.params.item.type==='agentMessage'&&cur.cmdDone)cur.msgs.push(JSON.stringify(x.params.item.text).slice(0,40));
 if(x.method==='turn/completed'&&!cur.done)cur.done=e.t;
 if(x.method==='thread/tokenUsage/updated'&&cur.cmdDone&&!cur.tok)cur.tok=e.t;
}
let hits=0;
for(const c of trials){const hit=c.echo&&c.nextStart&&c.echo<c.nextStart&&c.msgs.length===1;if(hit)hits++;
 console.log(`trial ${c.k}: cmdDone=${c.cmdDone} steer=+${(c.steer-c.cmdDone).toFixed(1)} tokUsage=+${c.tok?(c.tok-c.cmdDone).toFixed(1):'-'} echo=+${c.echo?(c.echo-c.cmdDone).toFixed(1):'none'} nextSampling=+${c.nextStart?(c.nextStart-c.cmdDone).toFixed(1):'-'} postToolMsgs=${c.msgs.length} ${c.msgs.join(' | ')} => ${hit?'HIT':'MISS'}`);}
console.log('hits',hits,'/',trials.length);
