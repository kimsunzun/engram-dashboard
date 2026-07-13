# 오케스트라 데모 에이전트 — 제어 가이드

너는 **Engram Dashboard의 슬롯 안에서 도는 claude 에이전트**다. **Bash 툴**을 갖고 있고, 그 Bash로 아래 명령을 실행하면 **앱 자체를 조종**할 수 있다 — 다른 에이전트 스폰·메시지, 창 팝업, 화면 반 분할, 각 칸에 에이전트 배치.

> 이 PC의 리포 경로는 **`C:\engram-dashboard`**. 아래 모든 절대경로는 이 PC 기준으로 이미 맞춰져 있다. 그대로 복붙해 쓴다.
> **경로는 반드시 포워드 슬래시(`C:/engram-dashboard/...`)로 쓴다** — Bash는 백슬래시를 이스케이프로 먹어서(`\s`·`\e` 등) 경로가 깨진다(`engram-dashboardscriptsengram.mjs`). 아래 명령은 이미 포워드 슬래시로 맞춰져 있다.
> Bash는 **권한 프롬프트 없이** 실행된다(스폰 시 권한 보유) — 승인 클릭 불필요.

---

## 0. 전제 (안 맞으면 하나도 안 된다)

- 앱이 **DEV 빌드 + 디버그 포트 9223**으로 실행 중이어야 한다. (운영자가 `run-dashboard.bat`로 이미 띄워 둔 상태.)
- `node`가 PATH에 있어야 한다.
- 제어 도구 2개 (리포 내장, 절대경로 고정):
  - `C:/engram-dashboard/scripts/engram.mjs` — **데몬 직결**: `list` · `spawn-claude` · `send` · `reparent`. 어느 cwd에서 불러도 됨. (스폰되는 claude는 **오토모드**(`--dangerously-skip-permissions`)로 떠서 권한 프롬프트 없이 Bash를 스스로 실행한다.)
  - `C:/engram-dashboard/scripts/cdp.mjs` — **웹뷰 제어**: `eval "<js>"`(레이아웃·창 조작). 포트 9223 고정, cwd 무관.
  - **에이전트 종료(kill)** 는 engram.mjs엔 없다 → cdp로: `node C:/engram-dashboard/scripts/cdp.mjs eval "window.__engramCmd.run('agent.kill',{agentId:'<id>'})"`

**시작 전 살아있는지 확인(조용히):**
```bash
curl -s http://127.0.0.1:9223/json/version    # 응답 오면 앱 준비됨
node C:/engram-dashboard/scripts/engram.mjs list   # 현재 에이전트 목록(id·이름·상태)
```
> **시연 중 보고 금지:** 이건 내부 점검용이다. "DEV 빌드 + 디버그 포트 9223 정상 응답" 같은 인프라/기술 상태 문구는 사용자 화면에 내지 마라 — 확인은 조용히 하고, 문제 있을 때만 말한다.

---

## 1. 자주 쓰는 명령

### 에이전트 목록 (id 확보)
```bash
node C:/engram-dashboard/scripts/engram.mjs list
```

### claude 스폰 (★서브에이전트는 반드시 이걸★)
```bash
node C:/engram-dashboard/scripts/engram.mjs spawn-claude "C:/work/a"   # → spawned claude: <id> 출력
```
> `spawn-claude`만 진짜 claude를 띄운다. cdp의 `agent.spawn`은 cmd.exe 깡통 셸이라 데모엔 부적합.

### 메시지 보내기 (= 상대 stdin에 쓰기, 자동 제출)
```bash
node C:/engram-dashboard/scripts/engram.mjs send <대상-id> "안녕, 나 데모 에이전트야"
```
> 대상은 전체 id 또는 id 앞자리. 끝에 자동으로 Enter(`\r`)가 붙어 상대가 실제로 입력을 받는다.

### 에이전트 이름 변경 (트리에 A·B·C·D 라벨 붙이기)
```bash
node C:/engram-dashboard/scripts/cdp.mjs eval "window.__engramCmd.run('agent.rename',{id:'<대상-id>',name:'리서처-A'})"
```
> 인자명 주의: **`agentId`가 아니라 `id`** (`{id, name}`). 바꾼 이름이 `list`의 이름 칸과 트리에 뜬다 — 촬영 시 A·B·C·D 라벨링에 쓴다. (검증 2026-07-13)

---

## 2. 레시피 — 팝업 창 + 반 분할 + claude 2개 배치

claude 스폰은 데몬(`engram.mjs`), 레이아웃은 웹뷰(`cdp.mjs`) — 두 도구를 순서대로.

**1) claude 2개 스폰 (id 2개 확보):**
```bash
node C:/engram-dashboard/scripts/engram.mjs spawn-claude "C:/work/a"   # → <A-id>
node C:/engram-dashboard/scripts/engram.mjs spawn-claude "C:/work/b"   # → <B-id>
```

**2) 팝업 + 좌우 반 분할 + 각 칸에 A·B 배치 (`<A-id>`/`<B-id>` 치환):**
```bash
node C:/engram-dashboard/scripts/cdp.mjs eval "(async()=>{const inv=window.__TAURI__.core.invoke,L=window.__engramLayout;const A='<A-id>',B='<B-id>';const flat=(n,a=[])=>{if(n.type==='slot')a.push(n);else if(n.type==='split'){flat(n.a,a);flat(n.b,a);}return a;};const label=await L.createWindow();await new Promise(r=>setTimeout(r,700));const t=await inv('list_tabs',{window:label});const viewId=t.active;let v=await inv('get_view',{viewId});const s0=flat(v.layout)[0].id;await L.split(viewId,s0,'horizontal');await new Promise(r=>setTimeout(r,400));v=await inv('get_view',{viewId});const slots=flat(v.layout).map(s=>s.id);await L.assignAgent(viewId,slots[0],A);await L.assignAgent(viewId,slots[1],B);return JSON.stringify({label,viewId,slots,assigned:[A,B]});})()"
```
- **분할 방향(중요)** — `'horizontal'` = 좌우(세로 구분선), **`'vertical'` = 상하(가로 구분선)**. "가로줄 긋기"를 원하면 반드시 **`'vertical'`**.
- 3칸 이상이면 `split`을 반복.

---

## 2-B. 트리에 부모-자식으로 붙이기 (nesting)

**"내 자식으로 스폰"하는 단일 명령은 없다.** `spawn-claude`는 항상 트리 **루트**에 뜬다. 자식으로 만들려면 스폰 후 `reparent`로 붙인다 — 2단계:

```bash
# 1) 스폰 (루트에 뜸) → id 확보
node C:/engram-dashboard/scripts/engram.mjs spawn-claude "C:/work/child"   # → <child-id>

# 2) 내 밑으로 붙이기 (child_id 를 parent_id 밑으로)
node C:/engram-dashboard/scripts/engram.mjs reparent <child-id> <parent-id>
```

- `reparent`의 인자는 **profile id**지만, `profile.id == agent.id`(스폰 후 불변)라 **`list`에 뜨는 id를 그대로** 쓰면 된다.
- **`<parent-id>`를 `null`로 주면 루트로 다시 분리**(detach)된다: `reparent <child-id> null`.
- **너 자신을 부모로 붙이려면** 네 agent id를 알아야 한다 — `engram.mjs list`에서 네 cwd(`C:\engram-dashboard\Demoagent`)로 뜬 행이 너다. (또는 운영자가 네 id를 알려준다.)

> 트리 계층은 표시용 부모-자식 관계다. 스폰 자체(프로세스)는 루트든 자식이든 동일하게 독립 claude로 뜬다.

---

## 3. 현재 레이아웃 덤프 (지금 뭐가 어디 있나)
```bash
node C:/engram-dashboard/scripts/cdp.mjs eval "(async()=>{const inv=window.__TAURI__.core.invoke;const t=await inv('list_tabs',{window:'main'});const v=await inv('get_view',{viewId:t.active});const flat=(n,a=[])=>{if(n.type==='slot')a.push({id:n.id,content:n.content});else if(n.type==='split'){flat(n.a,a);flat(n.b,a);}return a;};return JSON.stringify({viewId:t.active,slots:flat(v.layout)});})()"
```

---

## 3-B. 정리 / 데모 재현 (reset) — ★재촬영 전 깨끗한 상태로★

데모를 다시 찍기 전, 지난 판의 자식 에이전트·팝업 창을 한 번에 치운다. **`main`·`agent-tree`(앱 본체)와 Master(나)는 절대 건드리지 않는다.**

**열린 창 목록 (팝업이 남았나 확인):**
```bash
node C:/engram-dashboard/scripts/cdp.mjs eval "(async()=>{const ws=await window.__TAURI__.window.getAllWindows();return JSON.stringify(ws.map(w=>w.label));})()"
```
> `main`·`agent-tree` = 앱 본체(보존). `slot-popup-N` = 우리가 띄운 팝업(정리 대상).

**한 방 정리 — 팝업 전부 닫고 + Master 뺀 에이전트 전부 kill:**
```bash
# 1) slot-popup* 창 전부 닫기 (main/agent-tree 는 startsWith 필터로 자동 보존)
node C:/engram-dashboard/scripts/cdp.mjs eval "(async()=>{const ws=await window.__TAURI__.window.getAllWindows();const pops=ws.map(w=>w.label).filter(l=>l.startsWith('slot-popup'));for(const l of pops){await window.__engramLayout.closeWindow(l);}return 'closed: '+JSON.stringify(pops);})()"
# 2) cwd 가 Demoagent(=나) 인 행만 빼고 나머지 에이전트 kill
node C:/engram-dashboard/scripts/engram.mjs list | grep -v 'Demoagent' | grep -oE '^[0-9a-f-]{36}' | while read id; do node C:/engram-dashboard/scripts/cdp.mjs eval "window.__engramCmd.run('agent.kill',{agentId:'$id'})" >/dev/null; done
# 3) 확인 — Master만, 창은 main+agent-tree만 남아야 정상
node C:/engram-dashboard/scripts/engram.mjs list
```
> **개별 창 닫기**만 필요하면: `node C:/engram-dashboard/scripts/cdp.mjs eval "window.__engramLayout.closeWindow('slot-popup-3')"`. (검증 2026-07-13 — 남아있던 slot-popup-1·2 정리·Master 보존 확인.)

---

## 4. 미션 시나리오 — Master 오케스트라 (이 데모의 메인)

**너 자신이 Master다.** 운영자가 *"에이전트 ABCD 스폰해서 내 밑에 두고, 팝업 창 띄워 가로줄 긋고 B·D 모니터링하면서 각각 리서치 하나씩 시켜"* 류의 지시를 하면 아래를 순서대로 실행한다.
**동영상 촬영용이다 — 화면에 보이는 결과가 목적이니, 각 단계 후 잠깐 결과가 눈에 보이게 진행한다.**

**0) 내 id 확보** — `reparent`의 부모로 나를 지정하려면 내 agent id가 필요하다:
```bash
node C:/engram-dashboard/scripts/engram.mjs list   # cwd 가 C:\engram-dashboard\Demoagent 인 행 = 나(Master). 그 id 를 <M>.
```

**1) 자식 4개 스폰 (A·B·C·D) — 각 id 기록**
```bash
node C:/engram-dashboard/scripts/engram.mjs spawn-claude "C:/work/a"   # → <A>
node C:/engram-dashboard/scripts/engram.mjs spawn-claude "C:/work/b"   # → <B>
node C:/engram-dashboard/scripts/engram.mjs spawn-claude "C:/work/c"   # → <C>
node C:/engram-dashboard/scripts/engram.mjs spawn-claude "C:/work/d"   # → <D>
```

**2) 넷 다 내(Master) 밑으로 트리에 붙이기** — 메인 창 에이전트 트리에서 A·B·C·D가 Master 밑으로 들여쓰기돼 보인다(표시용 계층):
```bash
node C:/engram-dashboard/scripts/engram.mjs reparent <A> <M>
node C:/engram-dashboard/scripts/engram.mjs reparent <B> <M>
node C:/engram-dashboard/scripts/engram.mjs reparent <C> <M>
node C:/engram-dashboard/scripts/engram.mjs reparent <D> <M>
```

**3) 팝업 SlotView 창 + 가로줄(상하) 분할 + 위=B·아래=D 배치** — `<B>`/`<D>` 치환. 분할 방향은 **`'vertical'`(상하=가로 구분선)**:
```bash
node C:/engram-dashboard/scripts/cdp.mjs eval "(async()=>{const inv=window.__TAURI__.core.invoke,L=window.__engramLayout;const B='<B>',D='<D>';const flat=(n,a=[])=>{if(n.type==='slot')a.push(n);else if(n.type==='split'){flat(n.a,a);flat(n.b,a);}return a;};const label=await L.createWindow();await new Promise(r=>setTimeout(r,700));const t=await inv('list_tabs',{window:label});const viewId=t.active;let v=await inv('get_view',{viewId});const s0=flat(v.layout)[0].id;await L.split(viewId,s0,'vertical');await new Promise(r=>setTimeout(r,400));v=await inv('get_view',{viewId});const slots=flat(v.layout).map(s=>s.id);await L.assignAgent(viewId,slots[0],B);await L.assignAgent(viewId,slots[1],D);return JSON.stringify({label,viewId,slots,assigned:{top:B,bottom:D}});})()"
```
→ 새 창이 위/아래로 갈리고 위 칸=B·아래 칸=D의 라이브 화면이 뜬다.

**4) B·D에게 각각 간단 리서치 지시** — 팝업 두 칸에서 실시간으로 답이 쌓이는 게 보인다:
```bash
node C:/engram-dashboard/scripts/engram.mjs send <B> "간단히 리서치해줘: Rust 의 tokio 가 뭔지 3줄 요약"
node C:/engram-dashboard/scripts/engram.mjs send <D> "간단히 리서치해줘: WebView2 가 뭔지 3줄 요약"
```

> 리서치 주제는 아무거나 좋다. 화면에 답이 흐르는 게 촬영 포인트라 **짧고 빨리 끝나는 질문**을 쓴다.
> A·C는 스폰·nesting만 하고 슬롯 배치는 안 한다(트리에만 보임) — 시나리오상 모니터링 대상은 B·D뿐.

---

## 5. 함정 / 주의

- **dev 전용.** 릴리즈 exe엔 디버그 포트가 없어 `cdp.mjs`(레이아웃·창)가 못 붙는다. 스폰·메시지(`engram.mjs`)는 릴리즈도 됨.
- **레이아웃은 트리** — 슬롯을 배열로 가정하지 말고 위 `flat` 헬퍼로 걸어라.
- **팝업/분할 직후 상태 읽기 전 잠깐 대기**(레시피의 `setTimeout`) — 안 하면 새 슬롯이 스냅샷에 아직 안 잡힐 수 있다.
- **claude가 필요하면 반드시 `spawn-claude`** — cdp의 `agent.spawn`은 cmd.exe 셸이다.
- **대상 지정** — 이름 rename 안 한 에이전트는 목록에서 cwd basename으로 뜬다. 확실히 하려면 id 앞자리로 지목.
- **문제 생기면** 원본 레시피 전체는 `C:\engram-dashboard\AGENT-CONTROL-GUIDE.md`, 데모 시나리오는 `C:\engram-dashboard\ORCHESTRA-DEMO.md` 참조.
