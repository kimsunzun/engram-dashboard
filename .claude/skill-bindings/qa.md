# QA 바인딩 — engram

> **ADR-0004 컨벤션:** 이 파일은 소비처 프로젝트 트리(`.claude/skill-bindings/qa.md`)에 위치한다. qa 골격(`flow.md`)이 실행 착수 시 현재 프로젝트 루트 기준 cwd-상대 경로로 Read해 실값을 꺼낸다.

골격이 "프로젝트 빌드 명령"·"프로젝트 격리 게이트"·"프로젝트 코드 불변식"이라 부르는 자리에 끼우는 **engram 전용 실명령·체크리스트**다. 골격은 스택을 모른다 — 이 파일이 engram(Cargo workspace + Tauri + React)으로 바인딩한다.

> **정본 = CLAUDE.md "빌드·검증 명령" 절 + "GUI 시각/동작 검증" 절.** 이 파일은 그 **현재 바인딩 스냅샷**일 뿐이다 — 충돌하면 CLAUDE.md를 따르고 이 파일을 고친다(rot 방지). 명령을 통째 복붙해 두 출처가 갈리게 만들지 않는다.

## 프로젝트 구조 (강도·범위 매핑의 전제)

- **Cargo workspace 멤버 5개** — protocol · core(`engram-dashboard-core`) · discovery · daemon · src-tauri. `target/`·`tests/`·`cargo test`는 워크스페이스 루트.
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
- **core crate가 닿으면 격리 게이트도 포함**(quick이어도 — 아래 "코어 격리 불변식"): `rg "use tauri" crates/engram-dashboard-core/src/` → 0줄 PASS. quick의 `cargo test -p`만으론 Tauri import 회귀를 못 잡아 false PASS가 난다.
- 프론트가 닿았으면(quick 범위라도) 프론트 게이트(위 확정 절차): `npm test` + `npx tsc --noEmit`.

### standard (기본) — workspace 전회귀 + 격리 + 프론트

순서대로:
```bash
cargo build                                 # 1) 빌드 (루트, 전 workspace)
cargo test                                  # 2) 전 멤버 회귀 (core unit/통합 + protocol codec/ts-rs 등)
cargo fmt --check                           # 3) 포맷 게이트 (검사형 — rewrite 안 함)
rg "use tauri" crates/engram-dashboard-core/src/   # 4) 코어 격리 게이트 → 0줄이어야 PASS (ADR-0003)
npx tsc --noEmit                            # 5) 프론트 타입체크 (package.json에 typecheck 스크립트 없음)
npm test                                    # 6) 프론트 테스트 (vitest run)
```
- 코어 격리 게이트(`rg "use tauri" ...`)는 **출력이 0줄일 때만 PASS** — 한 줄이라도 나오면 FAIL(코어가 Tauri를 import = 격리 위반). 종료코드가 아니라 *매치 유무*로 판정한다.
- 멤버별로 좁혀 돌릴 땐 `cargo test -p engram-dashboard-core` / `cargo test -p engram-dashboard-protocol`.

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
- **비-Windows에선 cdp 불가** → standard까지가 한계 + "동작 미확인" 정직 보고(골격 §4).

## 실패 보고 시 게이트 명칭 (골격 §3에 주입)

어디서 막혔는지 짚을 때 쓰는 게이트 이름: build / test(어느 테스트) / fmt / 격리(`use tauri` 매치 줄) / tsc(타입체크) / npm(프론트 테스트) / cdp 실측(어느 동작).

## flaky·타이밍·perf 실패 = 매직넘버로 통과 금지 (ADR-0038)

flaky/타이밍/perf 실패를 상수·임계값·재시도 튜닝으로 통과시키려 하면 중단하고 `docs/reference/debugging-conventions.md`(OSS 조사 전환)를 적용한다. (이 규약의 *발화 지점* — qa가 신호를 잡는 곳.)

## 코어 격리 불변식 (정본 = ADR-0003 + 코드의 `// ADR-` 앵커)

코어 crate(`engram-dashboard-core`)는 **Tauri import 0** — `rg "use tauri" crates/engram-dashboard-core/src/` → 0줄. 이게 깨지면 코어가 전송 방식에 묶인 것 = 회귀. (근거·거부 대안은 ADR-0003.)
