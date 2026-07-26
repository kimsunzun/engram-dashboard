import fs from 'node:fs';

const PORT = '9223';
const BASE = `http://127.0.0.1:${PORT}`;

async function targets() {
  const r = await fetch(`${BASE}/json/list`);
  return await r.json();
}

function pickMain(list) {
  const pages = list.filter(t => t.type === 'page');
  const main = pages.find(t => /\/(index\.html)?(#\/?)?$/.test(t.url) && !/popup|tree/.test(t.url))
            || pages.find(t => !/popup|tree/.test(t.url))
            || pages[0];
  return main;
}

function cdp(wsUrl) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(wsUrl);
    let id = 0;
    const pending = new Map();
    ws.onopen = () => resolve({
      send(method, params = {}) {
        return new Promise((res, rej) => {
          const mid = ++id;
          pending.set(mid, { res, rej });
          ws.send(JSON.stringify({ id: mid, method, params }));
        });
      },
      close() { ws.close(); }
    });
    ws.onerror = (e) => reject(new Error('ws error: ' + (e.message || e)));
    ws.onmessage = (ev) => {
      const msg = JSON.parse(ev.data);
      if (msg.id && pending.has(msg.id)) {
        const { res, rej } = pending.get(msg.id);
        pending.delete(msg.id);
        if (msg.error) rej(new Error(JSON.stringify(msg.error)));
        else res(msg.result);
      }
    };
  });
}

const [cmd, argRaw] = process.argv.slice(2);
const arg = (argRaw && argRaw.startsWith('@file:')) ? fs.readFileSync(argRaw.slice(6), 'utf8').trim() : argRaw;

const list = await targets();
const main = pickMain(list);
if (!main) { console.error('no main page'); process.exit(1); }

console.error('Using page:', main.url);

const c = await cdp(main.webSocketDebuggerUrl);

if (cmd === 'eval') {
  const { result, exceptionDetails } = await c.send('Runtime.evaluate', {
    expression: arg,
    returnByValue: true,
    awaitPromise: true,
    allowUnsafeEvalBlockedByCSP: true,
  });
  if (exceptionDetails) {
    console.log(JSON.stringify({ error: exceptionDetails.text, detail: exceptionDetails.exception?.description }));
  } else {
    console.log(typeof result.value === 'string' ? result.value : JSON.stringify(result.value, null, 2));
  }
} else if (cmd === 'call') {
  // call method with JSON args from file
  const { methodName, args } = JSON.parse(fs.readFileSync(argRaw, 'utf8'));
  const expr = `(async () => { const r = await window.__ENGRAM_AGENT__.${methodName}(${args.map(a => JSON.stringify(a)).join(',')}); return JSON.stringify(r); })()`;
  const { result, exceptionDetails } = await c.send('Runtime.evaluate', {
    expression: expr, returnByValue: true, awaitPromise: true
  });
  if (exceptionDetails) console.log(JSON.stringify({ error: exceptionDetails.text, detail: exceptionDetails.exception?.description }));
  else console.log(typeof result.value === 'string' ? result.value : JSON.stringify(result.value, null, 2));
} else if (cmd === 'info') {
  console.log(JSON.stringify(list.map(t => ({ type: t.type, title: t.title, url: t.url })), null, 2));
}
c.close();
