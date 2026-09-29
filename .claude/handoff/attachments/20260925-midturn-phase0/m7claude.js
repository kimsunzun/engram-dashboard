const fs=require('fs');const L=fs.readFileSync(process.argv[2],'utf8').trim().split('\n').map(JSON.parse);
let trials=[],cur=null;
for(const e of L){
 if(e.dir==='meta'&&e.trial!==undefined&&e.ids){cur={k:e.trial,B:e.ids.B,A:e.ids.A};trials.push(cur);continue;}
 if(!cur)continue;
 if(e.dir==='in'&&/^B on tool_result/.test(e.note||''))cur.write=e.t;
 if(e.dir!=='out'||typeof e.line!=='object')continue;const x=e.line;
 const isTR=x.type==='user'&&Array.isArray(x.message?.content)&&x.message.content.some(c=>c.type==='tool_result');
 if(isTR&&!cur.tr)cur.tr=e.t;
 if(x.type==='command_lifecycle'&&x.command_uuid===cur.B){cur[x.state]=cur[x.state]||e.t;}
 if(x.type==='user'&&x.isReplay&&x.uuid===cur.B)cur.replay=e.t;
 if(cur.tr&&x.type==='assistant'&&!cur.nextAsst)cur.nextAsst=e.t;
 if(x.type==='result'){cur.results=(cur.results||0)+1;if(!cur.firstResult)cur.firstResult=e.t;}
}
let hits=0;const f=(v,b)=>v?('+'+(v-b).toFixed(1)):'-';
for(const c of trials){const hit=c.started&&c.nextAsst&&c.started<c.nextAsst;if(hit)hits++;
 console.log(`trial ${c.k}: tool_result=${c.tr} write=${f(c.write,c.tr)} queued=${f(c.queued,c.tr)} replay=${f(c.replay,c.tr)} started=${f(c.started,c.tr)} nextAssistant=${f(c.nextAsst,c.tr)} firstResult=${f(c.firstResult,c.tr)} results=${c.results} => ${hit?'HIT':'MISS'}`);}
console.log('hits',hits,'/',trials.length);
