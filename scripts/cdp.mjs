// Raw CDP helper — node 18+ 전역 WebSocket로 9223(WebView2)에 직접 붙는다. MCP 불필요.
// 사용:
//   node scripts/cdp.mjs shot <out.png>     메인 페이지 스크린샷 저장(파일명만 주면 _wip/shots/로 라우팅)
//   node scripts/cdp.mjs eval "<js>"        메인 페이지에서 JS 평가 후 결과 출력(JSON)
//   node scripts/cdp.mjs info               타깃 목록
// 환경: CDP_PORT(기본 9223), CDP_MATCH(기본 메인=hash 없는 localhost:1420/).
import fs from 'node:fs';

const PORT = process.env.CDP_PORT || '9223';
const BASE = `http://127.0.0.1:${PORT}`;

async function targets() {
  const r = await fetch(`${BASE}/json/list`);
  return await r.json();
}

// 메인 페이지 선택: popup/tree 제외, page 타입.
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

const [cmd, arg] = process.argv.slice(2);
const list = await targets();
if (cmd === 'info') { console.log(JSON.stringify(list.map(t => ({ type: t.type, title: t.title, url: t.url })), null, 2)); process.exit(0); }

const main = pickMain(list);
if (!main) { console.error('메인 페이지 없음'); process.exit(1); }
const c = await cdp(main.webSocketDebuggerUrl);

if (cmd === 'shot') {
  // 스크린샷 기본 보관함 = _wip/shots/ (gitignore됨). 디렉토리 없이 파일명만 주면 그쪽으로 라우팅해 repo 루트 오염 방지.
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
