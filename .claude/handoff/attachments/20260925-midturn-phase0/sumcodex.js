const fs=require('fs');const f=process.argv[2];const full=process.argv[3]==='full';
for(const l of fs.readFileSync(f,'utf8').trim().split('\n')){const o=JSON.parse(l);let s='';
 if(o.dir==='out'&&typeof o.line==='object'){const x=o.line;
   if(x.method==='item/agentMessage/delta'){ if(!full) continue; s='delta '+JSON.stringify(x.params.delta);}
   else if(x.method==='item/started'||x.method==='item/completed'){const it=x.params.item||{};s=x.method+' '+it.type+' id='+(it.id||'').slice(0,12)+(it.clientId!==undefined?' clientId='+String(it.clientId).slice(0,8):'')+(it.type==='userMessage'?' text='+JSON.stringify((it.content||[]).map(c=>c.text).join('|')).slice(0,70):'')+(it.type==='agentMessage'&&x.method==='item/completed'?' text='+JSON.stringify(it.text||'').slice(0,90):'')+(it.type==='commandExecution'?' cmd='+JSON.stringify(it.command||'').slice(0,60)+' status='+it.status:'')+' turn='+String(x.params.turnId||'').slice(0,12);}
   else if(x.method){s=x.method+' '+JSON.stringify(x.params||{}).slice(0,full?600:160);}
   else s='RESP id='+x.id+' '+JSON.stringify(x.result??x.error).slice(0,full?800:200);}
 else if(o.dir==='in') s='>> '+(o.note||'')+' '+JSON.stringify(o.line).slice(0,220);
 else s=o.dir+' '+JSON.stringify(o).slice(0,300);
 console.log(o.t, s);}
