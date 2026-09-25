const fs=require('fs');
const path=require('path');const p=path.join(process.env.APPDATA,'npm','node_modules','@anthropic-ai','claude-code','bin','claude.exe');
const buf=fs.readFileSync(p);
const args=process.argv.slice(2);
for (const a of args){
  const [off,len]=a.split(':').map(Number);
  console.log('=== @'+off+' len '+len);
  console.log(buf.slice(off,off+len).toString('latin1').replace(/[^\x09\x0a\x20-\x7e]/g,'.'));
}
