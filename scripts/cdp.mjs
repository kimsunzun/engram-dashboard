// Raw CDP helper — attaches directly to 9223 (WebView2) with the node 18+ global WebSocket. No MCP needed.
// Usage:
//   node scripts/cdp.mjs shot <out.png>     save a screenshot of the target page (a bare file name is routed to _wip/shots/)
//   node scripts/cdp.mjs eval "<js>"        evaluate JS in the target page and print the result (JSON)
//   node scripts/cdp.mjs info               target list
// Environment: CDP_PORT (default 9223), CDP_MATCH (regex string for url·title partial match).
//   CDP_MATCH unset = existing behavior (main, excluding popup/tree). 0 matches = exit 1 + print the target list (no fallback).
//   Multiple matches = pick the first match in list order and show the rest on stderr too. An invalid regex itself is handled the same.
import fs from 'node:fs';

const PORT = process.env.CDP_PORT || '9223';
const BASE = `http://127.0.0.1:${PORT}`;

async function targets() {
  const r = await fetch(`${BASE}/json/list`);
  return await r.json();
}

// Main page selection (default, when CDP_MATCH is unset): excludes popup/tree, page type.
function pickMain(list) {
  const pages = list.filter(t => t.type === 'page');
  const main = pages.find(t => /\/(index\.html)?(#\/?)?$/.test(t.url) && !/popup|tree/.test(t.url))
            || pages.find(t => !/popup|tree/.test(t.url))
            || pages[0];
  return main;
}

// CDP_MATCH selection: every page target whose url or title matches the regex (order = list order).
function pickByMatch(list, pattern) {
  const re = new RegExp(pattern);
  return list.filter(t => t.type === 'page' && (re.test(t.url) || re.test(t.title)));
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

const [cmd, arg] = process.argv.slice(2);
const list = await targets();
if (cmd === 'info') { console.log(JSON.stringify(list.map(t => ({ type: t.type, title: t.title, url: t.url })), null, 2)); process.exit(0); }

const match = process.env.CDP_MATCH;
let target;
if (match) {
  let matches;
  try {
    matches = pickByMatch(list, match);
  } catch (e) {
    // The SyntaxError that new RegExp(pattern) throws on an invalid CDP_MATCH — handled as the header promises (message +
    // target list + exit 1). So it does not leak out as a raw stack trace.
    console.error(`CDP_MATCH="${match}" 정규식 무효: ${e.message}. 사용 가능한 타깃:`);
    console.error(JSON.stringify(list.map(t => ({ type: t.type, title: t.title, url: t.url })), null, 2));
    process.exit(1);
  }
  if (matches.length === 0) {
    console.error(`CDP_MATCH="${match}" 매치 없음. 사용 가능한 타깃:`);
    console.error(JSON.stringify(list.map(t => ({ type: t.type, title: t.title, url: t.url })), null, 2));
    process.exit(1);
  }
  if (matches.length > 1) {
    console.error(`CDP_MATCH="${match}" ${matches.length}건 매치 — 첫 번째 선택:`);
    console.error(JSON.stringify(matches.map(t => ({ title: t.title, url: t.url })), null, 2));
  }
  target = matches[0];
} else {
  target = pickMain(list);
}
if (!target) { console.error('대상 페이지 없음'); process.exit(1); }
const c = await cdp(target.webSocketDebuggerUrl);

if (cmd === 'shot') {
  // Default screenshot store = _wip/shots/ (gitignored). A bare file name with no directory is routed there to avoid polluting the repo root.
  let out = arg || 'cdp-shot.png';
  if (!/[\\/]/.test(out)) {
    fs.mkdirSync('_wip/shots', { recursive: true });
    out = `_wip/shots/${out}`;
  }
  const { data } = await c.send('Page.captureScreenshot', { format: 'png' });
  fs.writeFileSync(out, Buffer.from(data, 'base64'));
  console.error('saved →', out);
  console.log('saved ' + out + ' (' + target.url + ')');
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
