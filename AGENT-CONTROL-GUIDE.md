# Engram Dashboard — 내부 에이전트 제어 가이드

너(이 문서를 읽는 에이전트)는 **Engram Dashboard의 슬롯 안에서 도는 claude 에이전트**다. **Bash 툴**을 갖고 있고, 그 Bash로 아래 명령을 실행하면 **앱 자체를 조종**할 수 있다 — 다른 에이전트 스폰·메시지, 창 팝업, 화면 반 분할, 각 칸에 에이전트 배치 등.

> 이 문서의 모든 명령은 **실측 검증됨**(dev 빌드, 2026-07-13). 그대로 복붙해 쓴다.

---

## 0. 전제 (안 맞으면 하나도 안 된다)

- 앱이 **DEV(디버그) 빌드로, 디버그 포트 9223이 열린 채** 실행 중이어야 한다. (`run-dashboard.bat`가 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223`로 띄운다. **릴리즈 빌드엔 이 포트가 없어 아래가 전부 안 먹는다.**)
- `node`가 PATH에 있어야 한다.
- 제어 도구 경로(고정): `I:\Engram\apps\engram-dashboard\scripts\cdp.mjs`

---

## 1. 기본 호출 형태

```bash
node I:\Engram\apps\engram-dashboard\scripts\cdp.mjs eval "<JS>"
```

- `<JS>`는 **앱 메인 웹뷰 안에서** 실행된다. 결과는 JSON으로 stdout에 찍힌다.
- **Promise는 자동 await**된다 → `async` IIFE로 감싸 `await` 자유롭게 써도 된다.
- 반환값은 JSON 직렬화 가능한 값이어야 한다(객체 OK, DOM 노드 X). 안전하게 `JSON.stringify(...)`로 감싸 반환하라.
- **따옴표:** 바깥은 `"`, JS 안 문자열은 `'`(작은따옴표)로 써서 충돌을 피한다.

---

## 2. 제어 표면 (검증된 API)

웹뷰 전역에 3개가 노출돼 있다(dev 전용):
- `window.__engramCmd` — 명령 레지스트리. `.list()`(전체 id), `.run(id, args)`.
- `window.__engramLayout` — 레이아웃 조작 래퍼(전부 Promise).
- `window.__engram` — 스토어 핸들(`.agent`, `.theme`, `.chatStyle`) — 상태 읽기용.
- `window.__TAURI__.core.invoke(name, args)` — 백엔드 커맨드 직접 호출(상태 읽기·stdin 쓰기).

### 자주 쓰는 것

| 하고 싶은 것 | 명령 |
|---|---|
| 에이전트 목록(id·이름) | `window.__engram.agent.getState().agents` |
| 에이전트 스폰 (**셸**) | `window.__engramCmd.run('agent.spawn', {cwd})` → **cmd.exe 깡통 셸**(claude 아님!) |
| 에이전트 스폰 (**claude**) | `node ...\engram.mjs spawn-claude <cwd>` → claude 스폰, id 출력. ★서브에이전트 데모는 이걸 써라★ |
| 에이전트 죽이기 | `window.__engramCmd.run('agent.kill', {agentId})` |
| **메시지 보내기(=상대 stdin에 쓰기)** | `invoke('agent_write_stdin', {agentId, data})` (data=바이트 배열, 아래 레시피) |
| 현재 탭/뷰 목록 | `invoke('list_tabs', {window:'main'})` → `{tabs, active(viewId)}` |
| 뷰의 슬롯 구조 | `invoke('get_view', {viewId})` → `{layout(트리), ...}` |
| 팝업 창 열기 | `window.__engramLayout.createWindow()` → label(`'slot-popup-1'`) |
| 슬롯 반 분할 | `window.__engramLayout.split(viewId, slotId, 'horizontal'\|'vertical')` → 새 slotId |
| 슬롯에 에이전트 배치 | `window.__engramLayout.assignAgent(viewId, slotId, agentId)` |
| 슬롯 포커스 | `window.__engramLayout.focusSlot(viewId, slotId)` |

전체 명령 id는 `node ...cdp.mjs eval "JSON.stringify(window.__engramCmd.list().map(c=>c.id))"`로 확인.

### 레이아웃 트리 구조 (중요)

`get_view`의 `layout`은 **재귀 트리**다. 슬롯은 배열이 아니라 트리를 걸어야 나온다:
- `{type:'slot', id, content:{type:'empty'|'agent_list'|'agent', agent_id?}}` — 잎(실제 칸)
- `{type:'split', dir, ratio, a:{...}, b:{...}}` — 분할 노드(a/b 재귀)

슬롯 평탄화 헬퍼: `const flat=(n,acc=[])=>{if(n.type==='slot')acc.push(n);else if(n.type==='split'){flat(n.a,acc);flat(n.b,acc);}return acc;};`

---

## 3. 레시피 (복붙)

### 3-1. 메시지 보내기 (표시명 또는 id로 대상 지정)

```bash
node I:\Engram\apps\engram-dashboard\scripts\cdp.mjs eval "(async()=>{const inv=window.__TAURI__.core.invoke;const s=window.__engram.agent.getState();const pById=new Map(s.profiles.map(p=>[p.id,p]));const label=a=>{const p=pById.get(a.id);return (p&&(p.display_name||p.name))||a.name||a.id.slice(0,8);};const needle='DEF';const text='안녕 DEF, 나 ACB야';const hit=s.agents.find(a=>a.id===needle)||s.agents.filter(a=>label(a).toLowerCase()===needle.toLowerCase())[0]||s.agents.filter(a=>a.id.startsWith(needle))[0];if(!hit)return 'NOT FOUND: '+needle;const data=Array.from(new TextEncoder().encode(text+'\r'));await inv('agent_write_stdin',{agentId:hit.id,data});return 'sent -> '+label(hit)+' ('+hit.id+')';})()"
```

- `needle`(대상)·`text`(내용)만 바꾼다. 대상 = 표시명(예 `DEF`), 전체 id, 또는 id 앞자리.
- 끝의 `\r`가 Enter(제출)라 상대가 실제로 입력을 받는다. 안 붙이면 입력창에 글자만 들어가고 제출 안 됨.

### 3-2. 팝업 열고 → 반 분할 → claude 2개 스폰 → 각 칸에 배치

**(실측 검증됨.)** claude 스폰은 데몬(engram.mjs), 레이아웃은 웹뷰(cdp) — 두 도구를 순서대로.

**1) claude 2개 스폰 (id 2개 확보):**
```bash
node I:\Engram\apps\engram-dashboard\scripts\engram.mjs spawn-claude "C:/work/a"   # → spawned claude: <A-id>
node I:\Engram\apps\engram-dashboard\scripts\engram.mjs spawn-claude "C:/work/b"   # → spawned claude: <B-id>
```

**2) 팝업 + 반 분할 + 각 칸에 배치 (`A`/`B`에 위 id를 넣는다):**
```bash
node I:\Engram\apps\engram-dashboard\scripts\cdp.mjs eval "(async()=>{const inv=window.__TAURI__.core.invoke,L=window.__engramLayout;const A='<A-id>',B='<B-id>';const flat=(n,a=[])=>{if(n.type==='slot')a.push(n);else if(n.type==='split'){flat(n.a,a);flat(n.b,a);}return a;};const label=await L.createWindow();await new Promise(r=>setTimeout(r,700));const t=await inv('list_tabs',{window:label});const viewId=t.active;let v=await inv('get_view',{viewId});const s0=flat(v.layout)[0].id;await L.split(viewId,s0,'horizontal');await new Promise(r=>setTimeout(r,400));v=await inv('get_view',{viewId});const slots=flat(v.layout).map(s=>s.id);await L.assignAgent(viewId,slots[0],A);await L.assignAgent(viewId,slots[1],B);return JSON.stringify({label,viewId,slots,assigned:[A,B]});})()"
```

세로 분할이면 `'horizontal'`→`'vertical'`. 3칸 이상이면 `split` 반복. **셸 아닌 claude가 필요하면 반드시 1)의 `spawn-claude`를 쓴다 — cdp의 `agent.spawn`은 cmd.exe 셸이다.**

### 3-3. 현재 레이아웃 덤프 (지금 뭐가 어디 있나)

```bash
node I:\Engram\apps\engram-dashboard\scripts\cdp.mjs eval "(async()=>{const inv=window.__TAURI__.core.invoke;const t=await inv('list_tabs',{window:'main'});const v=await inv('get_view',{viewId:t.active});const flat=(n,a=[])=>{if(n.type==='slot')a.push({id:n.id,content:n.content});else if(n.type==='split'){flat(n.a,a);flat(n.b,a);}return a;};return JSON.stringify({viewId:t.active,slots:flat(v.layout)});})()"
```

---

## 4. 함정 / 주의

- **표시명(ACB/DEF)은 profile.display_name에서 온다.** ad-hoc 스폰(cwd만) 에이전트는 프로필이 없어 `name`이 cwd다 → 그런 에이전트는 **id로 지목**하라. profile.id == agent.id(스폰 후 불변)라 3-1의 label 헬퍼가 조인한다.
- **레이아웃은 트리** — 슬롯을 배열로 가정하지 말고 위 `flat` 헬퍼로 걸어라.
- **팝업/분할 직후 상태 읽기 전 잠깐 대기**(위 레시피의 `setTimeout`) — 안 하면 새 슬롯이 아직 스냅샷에 안 잡힐 수 있다.
- **dev 전용.** 릴리즈 exe엔 디버그 포트가 없어 `cdp.mjs`가 못 붙는다.
- **대안 도구 `scripts\engram.mjs`(데몬 직결):** 웹뷰 없이 데몬에 직접 붙어 `list`/`spawn`/`spawn-claude`/`send`/`kill`. **스크립트 자기 위치로 데몬을 찾으니 어느 cwd에서 불러도 된다.** ★claude 스폰은 이 도구뿐★(cdp로는 불가). 단 **레이아웃·창 조작은 cdp.mjs만**(데몬은 UI를 모름 — ADR-0035).
- **cdp.mjs도 cwd 무관**(127.0.0.1:9223 고정 접속). 메시지는 두 도구 다 되지만, 자율 실행 검증에선 cdp 경로(3-1)가 안정적이었다.
