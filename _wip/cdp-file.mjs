// Raw CDP helper — attaches directly to 9223 (WebView2) with the node 18+ global WebSocket. No MCP needed.
// Usage:
//   node scripts/cdp.mjs shot <out.png>     save a main page screenshot (a bare file name is routed to _wip/shots/)
//   node scripts/cdp.mjs eval "<js>"        evaluate JS in the main page and print the result (JSON)
//   node scripts/cdp.mjs info               target list
// Environment: CDP_PORT (default 9223), CDP_MATCH (default main = localhost:1420/ without a hash).
import fs from 'node:fs';

const PORT = process.env.CDP_PORT || '9223';
const BASE = `http://127.0.0.1:${PORT}`;

async function targets() {
  const r = await fetch(`${BASE}/json/list`);
  return await r.json();
}

// Main page selection: popup/tree excluded, page type.
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

const [cmd, _argRaw] = process.argv.slice(2);
const arg = (_argRaw && _argRaw.startsWith('@file:')) ? fs.readFileSync(_argRaw.slice(6), 'utf8').trim() : _argRaw;
const list = await targets();
if (cmd === 'info') { console.log(JSON.stringify(list.map(t => ({ type: t.type, title: t.title, url: t.url })), null, 2)); process.exit(0); }

const main = pickMain(list);
if (!main) { console.error('no main page'); process.exit(1); }
const c = await cdp(main.webSocketDebuggerUrl);

if (cmd === 'shot') {
  // Default screenshot store = _wip/shots/ (gitignored). A bare file name without a directory is routed there so the repo root is not polluted.
  let out = arg || 'cdp-shot.png';
  if (!/[\\/]/.test(out)) {
    fs.mkdirSync('_wip/shots', { recursive: true });
    out = `_wip/shots/${out}`;
  }
  const { data } = await c.send('Page.captureScreenshot', { format: 'png' });
  fs.writeFileSync(out, Buffer.from(data, 'base64'));
  console.error('saved →', out);
  console.log('saved ' + out + ' (' + main.url + ')');
} else if (cmd === 'eval') {
  const { result, exceptionDetails } = await c.send('Runtime.evaluate', {
    expression: arg, returnByValue: true, awaitPromise: true
  });
  if (exceptionDetails) console.log(JSON.stringify({ error: exceptionDetails.text, detail: exceptionDetails.exception?.description }));
  else console.log(typeof result.value === 'string' ? result.value : JSON.stringify(result.value, null, 2));
} else {
  console.error('usage: shot <png> | eval <js> | info');
  process.exit(1);
}
c.close();
process.exit(0);
