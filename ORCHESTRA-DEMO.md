# Engram Dashboard — 에이전트 오케스트라 데모 런북 (디버그 모드)

여러 claude 에이전트를 한 화면에 띄우고, **LLM 제어로 스폰·배치하고 에이전트끼리 메시지를 주고받게** 하는 데모의 실행 절차. 아래 명령은 전부 **실측 검증됨**(dev/디버그 빌드, 2026-07-13).

> 대상 독자 = **데모 운영자(사람)**. 에이전트가 스스로 앱을 조종하는 방법(에이전트 대면)은 `AGENT-CONTROL-GUIDE.md` 참조.
>
> **소스를 막 받은 새 PC라면** 먼저 `SETUP-FRESH-PC.md`(툴체인·의존성 설치·첫 빌드)를 따라 부팅한 뒤 이 문서로 온다.

---

## 0. 왜 디버그 모드인가 (핵심 제약)

UI 조종(팝업·분할·슬롯 배치)은 **웹뷰 CDP 디버그 포트(9223)**를 통해서만 됩니다. 이 포트는 **dev 빌드에서만** 열립니다(릴리즈 exe엔 없음 — Tauri `dragDropEnabled`/CDP 이슈). 그래서 **오케스트라 데모 = dev(디버그) 빌드**로 합니다.

- **에이전트 스폰·메시지**(백엔드) = 데몬 WS 경유 → 빌드 무관(릴리즈도 됨).
- **UI/레이아웃 조종**(팝업·분할·배치) = 웹뷰 CDP 경유 → **dev 전용.**

---

## 1. 전제

- **dev 빌드** 실행 가능(Rust/node 툴체인). node 18+ PATH.
- WebView2 런타임(Win11 기본 탑재).
- 제어 도구 2개(리포 내장):
  - `scripts\engram.mjs` — **데몬 직결**: `list` · `spawn` · `spawn-claude` · `send` · `kill` · `reparent`. 자기 위치로 데몬을 찾아 **어느 cwd에서든** 동작.
  - `scripts\cdp.mjs` — **웹뷰 제어**: `eval "<js>"`(임의 JS·invoke) · `shot <png>` · `info`. 포트 9223 고정.

---

## 2. 실행

```bat
run-dashboard.bat
```
- 이 배치가 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223`를 걸고 `npm run tauri dev`를 띄웁니다(= 클라이언트 셸 빌드+start + vite, 데몬은 기존 것 재사용).
- 창이 뜨고 디버그 포트가 열릴 때까지 대기. 확인:
```bat
curl http://127.0.0.1:9223/json/version
```
(응답 오면 준비 완료.)

---

## 3. 데모 시나리오

### 3-1. claude 2개 스폰

```bat
node scripts\engram.mjs spawn-claude "C:/work/a"    :: → spawned claude: <A-id>
node scripts\engram.mjs spawn-claude "C:/work/b"    :: → spawned claude: <B-id>
node scripts\engram.mjs list                        :: id·이름·상태 확인
```
> `spawn-claude`가 claude 에이전트(chat UI). 일반 `spawn`은 cmd.exe 셸이라 데모엔 부적합.

### 3-2. 팝업 창 + 반 분할 + 각 칸에 배치 (선택 — 시각 연출)

위 `<A-id>`/`<B-id>`를 넣어 실행:
```bat
node scripts\cdp.mjs eval "(async()=>{const inv=window.__TAURI__.core.invoke,L=window.__engramLayout;const A='<A-id>',B='<B-id>';const flat=(n,a=[])=>{if(n.type==='slot')a.push(n);else if(n.type==='split'){flat(n.a,a);flat(n.b,a);}return a;};const label=await L.createWindow();await new Promise(r=>setTimeout(r,700));const t=await inv('list_tabs',{window:label});const v=t.active;let s=await inv('get_view',{viewId:v});const s0=flat(s.layout)[0].id;await L.split(v,s0,'horizontal');await new Promise(r=>setTimeout(r,400));s=await inv('get_view',{viewId:v});const sl=flat(s.layout).map(x=>x.id);await L.assignAgent(v,sl[0],A);await L.assignAgent(v,sl[1],B);return JSON.stringify({label,view:v,slots:sl});})()"
```
→ 팝업 창이 반으로 갈리고 좌/우에 claude A·B가 뜹니다. (세로 분할은 `'horizontal'`→`'vertical'`.)

### 3-3. 에이전트끼리 메시지 (오케스트라 핵심)

**(a) 운영자가 직접 보내기** — 대상 = 표시명 또는 id 앞자리:
```bat
node scripts\engram.mjs send <B-id> "안녕 B, 나 운영자야"
```
→ B의 입력창에 들어가고 자동 제출(`\r`). B(claude)가 응답.

**(b) 에이전트 A가 B에게 자율로 보내기** — A의 입력창에 이 지시를 타이핑(또는 `send <A-id> "..."`):
```
Bash로 실행해줘: node I:\Engram\apps\engram-dashboard\scripts\engram.mjs send <B-id> "안녕 B, 나 A야"
```
→ A(claude, Bash 툴 보유)가 그 명령을 실행 → 데몬 통해 B에 전달 → B가 수신·응답. **이게 "에이전트가 옆 에이전트와 대화"의 실물.**

**(c) 완전 자율 오케스트레이션** — A에게 가이드를 주고 알아서 하게:
```bat
node scripts\engram.mjs send <A-id> "[오케스트레이션] I:/Engram/apps/engram-dashboard/AGENT-CONTROL-GUIDE.md 를 읽고, 그 방법으로 에이전트 <B-id> 에게 '자율 테스트 성공' 이라고 보내."
```
→ A가 가이드를 Read → 스스로 제어 명령을 골라 B에게 전송(검증 시 A가 engram.mjs 실패 시 cdp.mjs로 자가복구까지 함). 사람 개입 0.

> 스폰된 claude 에이전트는 Bash를 **권한 프롬프트 없이** 실행합니다(스폰 시 권한 보유) — 승인 클릭 불필요.

---

## 4. 검증된 것 (2026-07-13 실측)

- 팝업 생성·반 분할·claude 2개 스폰·각 칸 배치 → cdp/engram으로 전부 동작(스크린샷 확인).
- 메시지 전송 → 상대 claude 화면에 렌더 + 응답.
- **자율 메타테스트**: 내부 claude가 가이드를 스스로 읽고 옆 에이전트에게 메시지 성사(실패 시 대체 경로 자가복구).

---

## 5. 알려진 제약 / 함정

- **dev 전용 UI 조종**: 릴리즈 exe엔 디버그 포트가 없어 팝업·분할·배치가 안 됨(스폰·메시지는 릴리즈도 가능). 오케스트라 풀 데모는 dev로.
- **트리 표시명 vs cwd**: 이름 rename(display_name) 안 한 에이전트는 목록/피커에서 cwd basename으로 뜸. id 앞자리로 지목하면 확실.
- **데몬 재시작**: 데몬을 끄면 에이전트 프로세스가 강제 종료됨(Job Object). 재활성화 시 기존 세션을 **resume**해 대화를 이어감(ADR-0076 — 이 동작은 코드 리빌드 후 적용).
- **드래그로 트리 계층화**: 노드를 다른 노드 **정중앙**에 떨궈야 nest(가장자리=루트 재정렬=미지원). (`dragDropEnabled:false`로 웹뷰 DnD 활성 — dev.)

---

## 6. 참조

- `SETUP-FRESH-PC.md` — 새 PC에서 소스 받고 부팅하는 절차(툴체인·의존성·첫 빌드).
- `AGENT-CONTROL-GUIDE.md` — 내부 에이전트가 앱을 조종하는 방법(에이전트 대면 레시피).
- `scripts\engram.mjs` — 데몬 CLI(백엔드 제어).
- `scripts\cdp.mjs` — 웹뷰 CDP 제어(UI/레이아웃).
- `run-dashboard.bat` / `run-dashboard-clean.bat`(데몬까지 재빌드) — dev 런처.
