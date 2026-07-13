# Engram Dashboard — 새 PC 부팅 가이드 (오케스트라 데모 재현용)

이 문서는 **git으로 소스를 막 받은 새 PC**에서 오케스트라 데모(`ORCHESTRA-DEMO.md`)를 돌아가게 만드는 절차다. **이 PC(새 PC)에서 도는 claude 에이전트가 그대로 따라 실행**하도록 쓰였다.

> 데모 자체(스폰·분할·메시지 시나리오)는 `ORCHESTRA-DEMO.md`, 내부 에이전트가 앱을 조종하는 레시피는 `AGENT-CONTROL-GUIDE.md`. 이 문서는 **그 앞단(부팅)만** 담당한다.

---

## 0. 왜 이 문서가 필요한가

데모는 **dev(디버그) 빌드**에서만 완전 동작한다(웹뷰 CDP 디버그 포트 9223이 dev 빌드에만 열림). 그래서 새 PC에선 릴리즈 exe를 받는 게 아니라 **소스를 받아 dev 빌드로 띄운다.** 아래는 그 툴체인 준비 + 빌드 + 실행 순서다.

`node_modules/`·`target/`·`.engram-data/`는 git에 없다(gitignore) → 새 PC에서 **의존성 설치와 첫 빌드가 필요**하다.

---

## 1. 전제 (툴체인 — 없으면 빌드/실행 자체가 안 됨)

Windows 11 기준. 아래 4개가 PATH에 있어야 한다:

1. **Rust toolchain** — `rustup`으로 설치(`rustc`/`cargo`). Tauri v2 backend + `windows` crate(Job Object) 컴파일에 필수.
2. **Microsoft C++ Build Tools** (MSVC, "Desktop development with C++") — Rust MSVC 타깃 링크에 필수. 없으면 `cargo build`가 링커 에러로 실패.
3. **Node.js 18+** / npm — 프론트(Vite/React) 빌드 + 제어 스크립트(`engram.mjs`/`cdp.mjs`) 실행.
4. **`claude` CLI** — PATH에 있어야 함. 데모의 `spawn-claude`가 **실제 claude 프로세스**를 띄우므로, 없으면 스폰 대상이 안 뜬다.

WebView2 런타임은 Win11에 기본 탑재(별도 설치 불필요).

**설치 확인:**
```bat
rustc --version
cargo --version
node --version   :: v18 이상
npm --version
claude --version
```
하나라도 없으면 그것부터 설치한 뒤 진행.

---

## 2. 소스 받기

> **경로 권장:** 가능하면 `I:\Engram\apps\engram-dashboard` **동일 경로로 clone**한다. `AGENT-CONTROL-GUIDE.md`의 내부-에이전트 명령들이 스크립트를 이 절대경로로 하드코딩하고 있어서다. 다른 경로에 받으면 §6(경로 치환)을 반드시 읽을 것.

```bat
:: 최초
git clone https://github.com/kimsunzun/engram-dashboard.git
:: 이미 있으면
git pull
```

---

## 3. 의존성 설치 + 첫 빌드

repo 루트에서:

```bat
npm install
cargo build
```

- `npm install` — 프론트 의존성(node_modules 생성).
- `cargo build` — Rust workspace 전체 빌드. **첫 빌드는 의존성 컴파일로 수 분 걸린다**(정상). 이게 통과하면 backend 코드가 이 PC에서 문제없이 컴파일된다는 뜻.
- (선택) 게이트 확인: `cargo test -p engram-dashboard-core` · `npm test` · `npx tsc --noEmit`.

---

## 4. 실행 (dev 빌드 + 디버그 포트)

```bat
run-dashboard.bat
```

- 이 배치가 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223`를 걸고 `npm run tauri dev`를 띄운다(클라이언트 셸 빌드+start + Vite, 데몬은 클라이언트가 자동 spawn).
- 창이 뜨고 디버그 포트가 열릴 때까지 대기. 확인:
```bat
curl http://127.0.0.1:9223/json/version
```
응답이 오면 준비 완료.

> 새 PC엔 기존 에이전트가 없다(`.engram-data/`는 로컬 전용, git에 없음). 데모에서 `spawn-claude`로 새로 띄우면 된다 — 정상.

---

## 5. 데모 진행

여기부터는 `ORCHESTRA-DEMO.md` 3장 시나리오를 그대로 따른다:
1. `node scripts\engram.mjs spawn-claude "<cwd>"` — claude 2개 스폰(id 확보)
2. `node scripts\cdp.mjs eval "<js>"` — 팝업 창 + 반 분할 + 각 칸에 배치
3. `node scripts\engram.mjs send <id> "<메시지>"` — 에이전트 간 메시지

내부 에이전트가 스스로 조종하게 하려면 `AGENT-CONTROL-GUIDE.md`를 그 에이전트에게 읽힌다.

---

## 6. 경로 치환 (동일 경로로 clone하지 않았을 때만)

`AGENT-CONTROL-GUIDE.md`의 내부-에이전트 명령들은 스크립트를 `I:\Engram\apps\engram-dashboard\scripts\cdp.mjs`(또는 `engram.mjs`) **절대경로**로 부른다(슬롯 안 에이전트의 cwd가 제각각이라 절대경로 필요). 다른 경로에 clone했다면:

- 그 절대경로들을 **이 PC의 실제 clone 경로**로 바꿔서 에이전트에게 준다.
- 운영자가 직접 치는 `ORCHESTRA-DEMO.md`의 명령들은 `node scripts\engram.mjs ...`처럼 **상대경로**라 repo 루트에서 실행하면 그대로 동작한다(치환 불필요).

---

## 7. 트러블슈팅

- **`cargo build` 링커 에러** → MSVC C++ Build Tools 미설치. §1-2 설치.
- **`curl` 9223 무응답** → dev 빌드가 아니거나 창이 아직 안 뜸. `run-dashboard.bat`로 띄웠는지, 창이 실제로 보이는지 확인. 릴리즈 exe엔 포트 없음.
- **`spawn-claude` 했는데 claude가 아니라 빈 셸** → `claude` CLI가 PATH에 없음(§1-4). `agent.spawn`(cdp)은 원래 cmd.exe 셸이니 claude가 필요하면 반드시 `spawn-claude`(engram.mjs).
- **`cdp.mjs`가 못 붙음** → 디버그 포트 미개방(dev 아님) 또는 포트 9222 충돌(Gemini Chrome). `CDP_PORT`로 변경 가능하나 데모는 9223 고정 전제.
- **데몬 관련 이상** → 데몬은 클라이언트 셸이 자동으로 띄운다. 꼬이면 `run-dashboard-clean.bat`(데몬까지 재빌드)로 재시작.

---

## 8. 참조

- `ORCHESTRA-DEMO.md` — 데모 시나리오(운영자 대면).
- `AGENT-CONTROL-GUIDE.md` — 내부 에이전트가 앱을 조종하는 레시피(에이전트 대면).
- `CLAUDE.md` — 프로젝트 아키텍처 원칙·빌드/검증 명령.
