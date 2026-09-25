const fs=require('fs'),path=require('path');
const p=path.join(process.env.APPDATA,'npm','node_modules','@anthropic-ai','claude-code','bin','claude.exe');
const buf=fs.readFileSync(p);
const [needle,before,after,max]=[process.argv[2],+process.argv[3]||200,+process.argv[4]||600,+process.argv[5]||3];
let i=-1,n=0;
while(n<max && (i=buf.indexOf(needle,i+1,'latin1'))!==-1){n++;
 console.log('=== @'+i);
 console.log(buf.slice(Math.max(0,i-before),i+after).toString('latin1').replace(/[^\x09\x0a\x20-\x7e]/g,'.'));}
console.log('hits shown',n);
