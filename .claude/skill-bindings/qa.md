# QA 바인딩 — engram

> **ADR-0004 컨벤션:** 이 파일은 소비처 프로젝트 트리(`.claude/skill-bindings/qa.md`)에 위치한다. qa 골격(`flow.md`)이 실행 착수 시 현재 프로젝트 루트 기준 cwd-상대 경로로 Read해 실값을 꺼낸다.

골격이 "프로젝트 빌드 명령"·"프로젝트 격리 게이트"·"프로젝트 코드 불변식"이라 부르는 자리에 끼우는 **engram 전용 실명령·체크리스트**다. 골격은 스택을 모른다 — 이 파일이 engram(Cargo workspace + Tauri + React)으로 바인딩한다.

> **정본 = CLAUDE.md "빌드·검증 명령" 절 + "GUI 시각/동작 검증" 절.** 이 파일은 그 **현재 바인딩 스냅샷**일 뿐이다 — 충돌하면 CLAUDE.md를 따르고 이 파일을 고친다(rot 방지). 명령을 통째 복붙해 두 출처가 갈리게 만들지 않는다.

## 프로젝트 구조 (강도·범위 매핑의 전제)

- **Cargo workspace** — 멤버 목록의 정본은 루트 `Cargo.toml`의 `[workspace] members`(여기 베끼면 갈린다). `target/`·`tests/`·실행 cwd는 워크스페이스 루트 — 단 **루트 bare `cargo test` 금지**(아래 standard 2번).
- **프론트** — `src/`(React 19 + TS + Vite), `package.json`, `vite.config.*`, `tauri.conf.json`.

**경로 → 강도 매핑(골격 §1 "변경 범위 판정"에 주입):**
- `crates/<name>/` → 해당 crate(단일이면 quick 후보)
- `src-tauri/` · 루트 `Cargo.toml` · `Cargo.lock` → **standard 이상**(workspace 영향)
- `src/` · `public/` · `index.html` · `package*.json` · `vite.config.*` · `tauri.conf.json` → **UI=full**(cdp 실측)
- `tests/` → **standard 이상**
- **판정 불가** → standard

**UI/프론트 영향 정의(이것만):** 위 프론트 경로가 닿았거나 **Tauri command/IPC 응답 *형식* 변경**. 이에 해당하면 full(cdp 실측 필수), 그 외 백엔드만이면 standard로 충분.

**핫패스 = 불변식 영역:** spawn/kill/pump·이벤트버스·transport·epoch·replay→live 등 동시성·lifetime 경로(CLAUDE.md "핵심 불변식")가 닿으면 full — 이 경로는 test PASS만으론 race·lifetime 동작을 보장 못 한다. **정직 note:** full의 cdp 실측 **1회 통과도 race-free 증명이 아니다** — smoke(존재 증거)일 뿐, 핫패스는 1회 관찰로 race를 배제하지 못한다(과청구 금지).

## 강도별 실명령 (골격 §2 "게이트 실행"에 주입)

모두 **워크스페이스 루트에서** 실행한다. 게이트 순서(빌드 → 테스트 → 격리 → 타입체크·프론트 → 실측)·실패 시 멈춤은 골격이 강제한다.

**프론트 게이트 확정 절차:** ① `npm test`(package.json `scripts.test` = `vitest run`). ② 타입체크는 `npm run typecheck`가 있으면 우선, **없으면 `npx tsc --noEmit`**(현재 package.json엔 typecheck 스크립트 없음 → `npx tsc --noEmit`). ③ 스크립트가 아예 없으면 실행하지 말고 package.json 실제 스크립트명을 사용자에게 보고한다. **프론트 린트 게이트는 정본(CLAUDE.md·package.json)에 없음 — 임의로 lint를 추가하지 않는다.**

### quick — 영향 crate만

영향받은 멤버만 좁게 돌린다(예: core만 바뀐 경우):
```bash
cargo build -p engram-dashboard-core        # 빌드
cargo test  -p engram-dashboard-core        # 영향 crate 테스트
```
- **core crate가 닿으면 격리 게이트도 포함**(quick이어도 — 아래 "코어 격리 불변식"): `rg "^\s*use tauri" crates/engram-dashboard-core/src/` → 0줄 PASS. quick의 `cargo test -p`만으론 Tauri import 회귀를 못 잡아 false PASS가 난다.
- 프론트가 닿았으면(quick 범위라도) 프론트 게이트(위 확정 절차): `npm test` + `npx tsc --noEmit`.

### standard (기본) — workspace 전회귀 + 격리 + 프론트

순서대로:
```bash
cargo build                                 # 1) 빌드 (루트, 전 workspace)
cargo test --workspace --exclude engram-dashboard   # 2) 전 멤버 회귀 — src-tauri 패키지(`engram-dashboard`)만 뺀다. 루트 bare cargo test 금지(src-tauri lib 타깃이 0xc0000139 STATUS_ENTRYPOINT_NOT_FOUND 로 죽는다 — 실측 2026-08-05, 정본 CLAUDE.md·2026-07-19 드리프트 수정)
cargo fmt --check                           # 3) 포맷 게이트 (검사형 — rewrite 안 함)
rg "^\s*use tauri" crates/engram-dashboard-core/src/   # 4) 코어 격리 게이트 → 0줄이어야 PASS (ADR-0003)
npx tsc --noEmit                            # 5) 프론트 타입체크 (package.json에 typecheck 스크립트 없음)
npm test                                    # 6) 프론트 테스트 (vitest run)
```
- 코어 격리 게이트(`rg "^\s*use tauri" ...`)는 **출력이 0줄일 때만 PASS** — 한 줄이라도 나오면 FAIL(코어가 Tauri를 import = 격리 위반). 종료코드가 아니라 *매치 유무*로 판정한다. 패턴은 import 라인 앵커(`^\s*`) — 게이트 규칙을 자기 인용한 문서 주석(`//!`)이 오탐되는 것 방지(실측 2026-07-13).
- 멤버별로 좁혀 돌릴 땐 `cargo test -p <멤버>`.
- **메시징 커널 격리 게이트(ADR-0110 — messaging crate가 닿으면 필수):** `rg "engram_dashboard_(core|daemon|protocol|discovery)" crates/engram-dashboard-messaging/src/` → 0줄 PASS. 이 crate는 워크스페이스 crate 무의존이 불변식이라 위반은 컴파일 에러로 먼저 잡히지만, 주석·테스트 헬퍼 이름으로 새는 경로는 grep이 잡는다.
- **네트워크 행 격리 게이트(ADR-0129 — net crate가 닿으면 필수, quick이어도):** 아래를 **전부** 돌린다. 기대값·근거의 정본은 `crates/engram-dashboard-net/src/lib.rs` 헤더이고, 기대값을 늘리기 전에 그 헤더와 그 crate `Cargo.toml`의 의존 상한 규칙을 먼저 읽는다.
  ```bash
  rg "engram_dashboard_(daemon|messaging|discovery)" crates/engram-dashboard-net/src/          # 게이트1 소스 참조 → 0줄 PASS
  rg -o --no-filename "engram_dashboard_core::[A-Za-z0-9_:]+" crates/engram-dashboard-net/src/ | sort -u   # 게이트2 core 심볼 allowlist → 정확히 2줄 PASS
  cargo tree -p engram-dashboard-net --depth 1 --prefix none -e normal,dev,build --target all --all-features | rg "^engram-dashboard" | sort -u   # 게이트3 직접 워크스페이스 의존 상한 → 정확히 3줄 PASS
  rg "(A)gentCommand|(P)ROTOCOL_VERSION" crates/engram-dashboard-net/src/   # 게이트4 auth 어휘 재유입 금지 → 0줄 PASS
  ```
  게이트1·4는 매치 유무로 판정하고(0줄이어야 PASS — 코어 `use tauri` 게이트와 같은 규칙), 게이트2·3은 줄 수로 판정한다. 어느 쪽도 종료코드로 판정하지 않는다. 게이트2의 기대값은 **심볼 단위**다 — "`portfile.rs`만"처럼 파일 이름으로 바꾸면 그 파일 안에 새 import가 들어와도 통과한다. 게이트3은 **해석된 의존 그래프**를 읽는다 — `Cargo.toml` 텍스트 grep으로 바꾸지 말고(rename·`[dependencies.<이름>]` 테이블 형·들여쓴 선언·`[build-dependencies]`·비활성 target·`optional`이 빠져나간다 — 실측) 플래그도 줄이지 않는다. 게이트4의 패턴을 `_(이름)` 괄호 형태에서 풀어 쓰지 않는다 — 그 형태의 근거(자기일치 함정, 실측 기록)는 net crate 헤더의 게이트4 절이 정본이다.
- **공유 데몬 바이너리 락(실발동 2026-07-08):** 실행 중인 `engram-dashboard-daemon.exe`(공유 인프라 — 타 에이전트 호스팅 가능)가 있으면 daemon bin을 빌드하는 루트 `cargo build`·`cargo test`가 os error 5로 FAIL한다 — 코드 결함 아님. **데몬 강제 종료 금지.** 우회 = daemon bin을 안 빌드하는 패키지 스코프(`cargo build/test -p <영향 crate들>`)로 좁혀 회귀 확인, 워크스페이스 전체 게이트는 **PARTIAL로 정직 보고**(못 돌린 범위 명시).

### full — standard + GUI 실측 (cdp)

standard 게이트를 전부 PASS시킨 뒤, 실제 앱을 띄워 화면 동작을 확인한다(**Windows 전용** — WebView2 CDP, 포트 9223 고정):
```powershell
# 1) 디버그 포트 열고 앱 실행 (백그라운드) — PowerShell (bash면: WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223" npm run tauri dev)
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=9223"; npm run tauri dev
# 2) 포트 뜰 때까지 대기
curl http://127.0.0.1:9223/json/version
# 3) 실측
node scripts/cdp.mjs info                   # 페이지 목록 확인
node scripts/cdp.mjs eval "<js>"            # 앱 안 JS·실제 invoke 호출 (spawn/write/interrupt/kill 등 IPC 검증)
node scripts/cdp.mjs shot out.png           # 필요시 스크린샷 → Read로 확인
```
- 포트 9223 고정(9222=Gemini Chrome 충돌 회피, `CDP_PORT`로 변경).
- **검증엔 스샷보다 `eval` 텍스트가 토큰·정확도 유리**(픽셀 해석 회피) — DOM 텍스트·`window.__TAURI__.core.invoke(...)` 결과를 직접 확인. shot은 레이아웃·시각 확인이 필요할 때만.
- 변경이 닿은 동작을 실제로 한 번 통과시켜 본다(예: spawn → 출력 도착 → kill → 상태 전이). **이게 통과해야 동작 확인 = 완료**.
- 로그가 필요하면 `$env:RUST_LOG = "debug"`(기본 OFF=warn — bash면 `RUST_LOG=debug` 접두)로 앱을 띄운다.
- **teardown — 자기가 띄운 건 자기가 치운다(실발동 2026-07-10):** 실측 종료 시 1)에서 띄운 런처의 **PID 트리째** 강제 종료한다(`taskkill /PID <런처PID> /T /F` — 자식 `engram-dashboard.exe`·vite watcher까지). 런처 PID를 실측 시작 시 기록해 둔다. **경계: 공유 데몬(`engram-dashboard-daemon.exe`)은 불가침** — qa가 띄운 게 아니면 죽이지 않는다(persist 모델·타 에이전트 호스팅 가능).
- **비-Windows에선 cdp 불가** → standard까지가 한계 + "동작 미확인" 정직 보고(골격 §4).

## 실패 보고 시 게이트 명칭 (골격 §3에 주입)

어디서 막혔는지 짚을 때 쓰는 게이트 이름: build / test(어느 테스트) / fmt / 격리(어느 crate의 어느 게이트 + 매치 줄 또는 실제 줄 수) / tsc(타입체크) / npm(프론트 테스트) / cdp 실측(어느 동작).

## flaky·타이밍·perf 실패 = 매직넘버로 통과 금지 (ADR-0038)

flaky/타이밍/perf 실패를 상수·임계값·재시도 튜닝으로 통과시키려 하면 중단하고 `docs/reference/debugging-conventions.md`(OSS 조사 전환)를 적용한다. (이 규약의 *발화 지점* — qa가 신호를 잡는 곳.)

## 코어 격리 불변식 (정본 = ADR-0003 + 코드의 `// ADR-` 앵커)

코어 crate(`engram-dashboard-core`)는 **Tauri import 0** — `rg "^\s*use tauri" crates/engram-dashboard-core/src/` → 0줄. 이게 깨지면 코어가 전송 방식에 묶인 것 = 회귀. (근거·거부 대안은 ADR-0003.)
