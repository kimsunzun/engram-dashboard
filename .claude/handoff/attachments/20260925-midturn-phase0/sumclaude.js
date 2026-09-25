const fs=require('fs');const f=process.argv[2];
const short=u=>(u||'').slice(0,8);
for(const l of fs.readFileSync(f,'utf8').trim().split('\n')){const o=JSON.parse(l);let s='';
 if(o.dir==='out'&&typeof o.line==='object'){const x=o.line;
  if(x.type==='system'&&['thinking_tokens','hook_started','hook_response'].includes(x.subtype))continue;
  if(x.type==='rate_limit_event')continue;
  if(x.type==='command_lifecycle')s='LC '+x.state+' '+short(x.command_uuid);
  else if(x.type==='control_response')s='CTRL_RESP '+JSON.stringify(x.response);
  else if(x.type==='user'){const c=x.message.content;s='user'+(x.isReplay?' REPLAY '+short(x.uuid):'')+' '+(Array.isArray(c)?c.map(b=>b.type==='tool_result'?'tool_result':('text:'+JSON.stringify(b.text).slice(0,60))).join(','):JSON.stringify(c).slice(0,60));}
  else if(x.type==='assistant'){s='assistant '+x.message.content.map(b=>b.type==='text'?'text:'+JSON.stringify(b.text).slice(0,80):b.type==='tool_use'?'tool_use:'+JSON.stringify(b.input.command||b.input).slice(0,40):b.type).join(',');}
  else if(x.type==='result')s='RESULT '+x.subtype+' '+JSON.stringify(x.result||'').slice(0,60);
  else s=x.type+'/'+(x.subtype||'')+' '+JSON.stringify(x).slice(0,160);
  s='chunk'+o.chunk+' '+s;}
 else if(o.dir==='in') s='>> '+o.note;
 else s=o.dir+' '+JSON.stringify(o).slice(0,260);
 console.log(o.t, s);}
