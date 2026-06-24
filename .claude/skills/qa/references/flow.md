# QA — 실행 절차

`$ARGUMENTS` = 강도 `quick`|`standard`|`full` (옵션). 미지정이면 변경 범위로 추정하되 기본 standard. 어떤 강도·어떤 게이트로 도는지 호출 시 사용자에게 한 줄 명시한다.

> **명령 우선순위(1회 선언):** CLAUDE.md "빌드·검증 명령" 절 + "GUI 시각/동작 검증" 절이 **정본**이다. 이 파일 §2는 그 **현재 바인딩 스냅샷**일 뿐 — 충돌하면 CLAUDE.md를 따르고 이 파일을 고친다(rot 방지). 아래 절들은 이 원칙을 반복하지 않는다.

## 0. 강도(intensity) 정하기

강도가 **게이트 범위**를 고른다 — 단계가 아니라 단일 스케일이다(review의 강도와 평행). 실명령은 §2 — 여기는 범위·순서·escalation만.

| 강도 | 게이트 범위 | 언제 |
|---|---|---|
| **quick** | 영향 crate만 회귀(+ core crate가 닿으면 격리도, 프론트가 닿으면 타입체크도) | 국소 변경·단일 crate |
| **standard**(기본) | workspace 전회귀 + 격리 + 프론트(테스트·타입체크) | 일반 비자명 변경 |
| **full** | standard + GUI 실측(cdp) | UI·핫패스·릴리스·실제 동작 확인 필요 |

- 게이트 순서(고정): **빌드 → 테스트 → 격리 → (타입체크·프론트) → 실측**. 빌드가 깨지면 다음으로 안 넘어간다(§3). 실측(full)은 코드 게이트가 다 PASS인 뒤에만 의미 있다.
- **escalation-only(사용자 확인 없이 승격):** 시작 강도가 **하한**이다. 도중 다중 crate·UI 영향·핫패스를 발견하면 **사용자 확인 없이 상위 강도로 자동 승격하고 한 줄 알린다**. 임의로 낮추지 않는다. (강도 판정 자체가 불가할 때만 §0-1대로 사용자에게 묻는다.)
- 강도 선택 트리거: 변경 범위(crate 수) · **UI/프론트 영향 여부**(있으면 full — cdp 실측 필수) · 핫패스(spawn/kill/pump·이벤트버스) · 릴리스 직전. 무거울수록 위. 애매하면 standard. 영향 crate→강도 매핑은 §1.

## 0-1. 인자 파싱·기본값 (트리거 추정 규칙)

`/qa [quick|standard|full]` — 강도는 옵션이다. 인자를 다음 규칙으로만 해석한다(소문자만 허용):
- **인자 없음** → 변경 범위로 추정(§1)하되 기본 **standard**.
- **정확히 `quick`|`standard`|`full` 하나** → 해당 강도로(escalation 하한). 단 UI 영향이 있는데 quick/standard가 지정되면 full로 자동 승격(escalation-only).
- **그 외(알 수 없는 토큰·복수 인자·오타·대문자)** → **실행하지 말고** 받은 인자와 사용법(`/qa [quick|standard|full]`)을 사용자에게 보고한다(추정 금지).
- 애매하면 standard로 돌리되 한 줄 명시. UI 영향이 의심되면 full 여부를 사용자에게 한 줄 확인.

## 1. 변경 범위 판정 (Lead = 메인 오케스트레이터)

게이트를 돌리기 전에 무엇이 바뀌었는지부터 파악한다 — 강도·게이트 범위가 여기서 갈린다. `git diff --stat`(또는 변경 파일 목록)으로 닿은 경로를 본다.

**경로 → 강도 매핑(고정):**
- `crates/<name>/` → 해당 crate(단일이면 quick 후보)
- `src-tauri/` · 루트 `Cargo.toml` · `Cargo.lock` → **standard 이상**(workspace 영향)
- `src/` · `package.json` · `vite.config.*` · `tauri.conf.json` → **UI=full**(cdp 실측)
- `tests/` → **standard 이상**
- **판정 불가** → standard

**UI/프론트 영향 정의(이것만):** `src/` · `public/` · `index.html` · `vite.config.*` · `package*.json` · `tauri.conf.json`이 닿았거나, **Tauri command/IPC 응답 *형식* 변경**. 이에 해당하면 full(cdp 실측 필수). 그 외 백엔드만이면 standard로 충분.

**핫패스:** spawn/kill/pump·이벤트버스·transport 등 동시성·lifetime 경로(CLAUDE.md 핵심 불변식 영역)가 닿으면 full.

**core crate 격리(quick이어도 필수):** quick은 영향 crate만 돌지만, **core crate(`engram-dashboard-core`)가 닿으면 quick이어도 격리 게이트(`rg "use tauri"`)를 포함**한다 — quick의 `cargo test -p`만으론 Tauri import 회귀를 못 잡아 false PASS가 난다.

판정 결과로 강도를 확정(§0)하고, 어떤 게이트를 도는지 사용자에게 한 줄 명시한다. 도중 위 트리거를 추가로 발견하면 **escalation-only**(사용자 확인 없이 상위 승격 + 한 줄 알림, §0).

## 2. 게이트 실행 (강도별 실제 명령 — 현재 바인딩 스냅샷)

모두 **워크스페이스 루트에서** 실행한다. 게이트는 build → test → 격리 → (타입체크·프론트) → 실측 순서로 돌리고, 앞 게이트가 깨지면 멈추고 §3으로 간다(통과 위장 금지). (명령 우선순위는 상단 1회 선언 참조.)

**프론트 게이트(확정 절차):** ① `npm test`가 package.json `scripts`에 정의돼 있으면 실행(현재 `test` = `vitest run` → `npm test`). ② 타입체크는 `npm run typecheck`가 있으면 우선, **없으면 `npx tsc --noEmit`**(현재 package.json엔 typecheck 스크립트 없음 → `npx tsc --noEmit`). ③ 해당 스크립트가 아예 없으면 실행하지 말고 package.json 실제 스크립트명을 사용자에게 보고한다. **프론트 린트 게이트는 정본(CLAUDE.md·package.json)에 없음 — 임의로 lint를 추가하지 않는다.**

### quick — 영향 crate만

영향받은 멤버만 좁게 돌린다(예: core만 바뀐 경우):
```bash
cargo build -p engram-dashboard-core        # 빌드
cargo test  -p engram-dashboard-core        # 영향 crate 테스트
```
- **core crate가 닿으면 격리 게이트도 포함**(quick이어도 — §1): `rg "use tauri" crates/engram-dashboard-core/src/` → 0줄 PASS.
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
- 코어 격리 게이트(`rg "use tauri" ...`)는 **출력이 0줄일 때만 PASS** — 한 줄이라도 나오면 FAIL(코어가 Tauri를 import = 격리 위반). 이건 grep 검증이라 종료코드가 아니라 *매치 유무*로 판정한다.
- 멤버별로 좁혀 돌릴 땐 `cargo test -p engram-dashboard-core` / `cargo test -p engram-dashboard-protocol`.

### full — standard + GUI 실측 (cdp)

standard 게이트를 전부 PASS시킨 뒤, 실제 앱을 띄워 화면 동작을 확인한다(**Windows 전용** — WebView2 CDP, 포트 9223 고정):
```bash
# 1) 디버그 포트 열고 앱 실행 (백그라운드)
WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223" npm run tauri dev
# 2) 포트 뜰 때까지 대기
curl http://127.0.0.1:9223/json/version
# 3) 실측
node scripts/cdp.mjs info                   # 페이지 목록 확인
node scripts/cdp.mjs eval "<js>"            # 앱 안 JS·실제 invoke 호출 (spawn/write/interrupt/kill 등 IPC 검증)
node scripts/cdp.mjs shot out.png           # 필요시 스크린샷 → Read로 확인
```
- **검증엔 스샷보다 `eval` 텍스트가 토큰·정확도 유리**(픽셀 해석 회피) — DOM 텍스트·`window.__TAURI__.core.invoke(...)` 결과를 직접 확인. shot은 레이아웃·시각 확인이 필요할 때만.
- 변경이 닿은 동작을 실제로 한 번 통과시켜 본다(예: spawn → 출력 도착 → kill → 상태 전이). **이게 통과해야 동작 확인 = 완료**.
- 로그가 필요하면 `RUST_LOG=debug`(기본 OFF=warn)로 앱을 띄운다.

## 3. 실패 처리 (통과 위장 절대 금지)

게이트가 깨지면 **어디서·왜 깨졌는지를 정확히** 사용자에게 보고한다. 다음 게이트로 넘어가거나 실패를 숨기지 않는다.

- **어느 게이트서 막혔나** — build / test(어느 테스트) / fmt / 격리(`use tauri` 매치 줄) / tsc(타입체크) / npm(프론트 테스트) / cdp 실측(어느 동작) 중 정확히 짚는다.
- **로그·에러 핵심** — 컴파일 에러 메시지, 실패한 테스트 이름과 assert, panic 백트레이스 등 *판단에 필요한 최소*를 인용한다(전체 덤프 금지 — 핵심만).
- **재현 명령** — 사용자가 그대로 돌릴 수 있는 명령 한 줄(예: `cargo test -p engram-dashboard-core <test_name>`).
- **빌드 깨지면 멈춘다** — build FAIL이면 test/격리/실측은 의미 없으므로 돌리지 않고 build 실패만 보고한다(순서 게이트).
- 실패를 "대체로 통과"·"사소함"으로 포장하지 않는다. FAIL은 FAIL로 보고한다(가드레일).

## 4. 결과 보고 (PASS/FAIL + 게이트별)

메인이 강도·게이트별 결과를 사용자에게 보고한다.

- **종합 판정** — 전 게이트 통과면 PASS, 하나라도 깨지면 FAIL.
- **게이트별 결과** — build / test / fmt / 격리 / tsc / npm / (full이면) cdp 실측 각각 PASS/FAIL. full에서 cdp까지 PASS여야 "동작 확인 완료".
- **full에서 cdp를 못 돌린 경우**(비-Windows 등) — standard까지 PASS임을 명시하고 GUI 실측 미수행을 정직하게 알린다("동작 미확인" 상태).
- 커밋은 이 게이트(+ review) 통과 후에만. ADR·step-log 기록은 메인이 처리한다(이 스킬이 직접 쓰지 않는다).

## 프로젝트 통합 (스킬 밖 — engram 바인딩)

§2 실명령의 바인딩 출처·불변식만 여기 모은다(골격에 하드코딩 X — CLAUDE.md를 가리킨다. 명령 우선순위는 상단 1회 선언):

- **Cargo workspace 멤버** — protocol · core · discovery · daemon · src-tauri (5 멤버). `target/`·`tests/`·`cargo test`는 워크스페이스 루트.
- **코어 격리 불변식** — 코어 crate는 Tauri import 0 (ADR-0003). `rg "use tauri" crates/engram-dashboard-core/src/` → 0줄. 이게 깨지면 코어가 전송 방식에 묶인 것 = 회귀.
- **cdp 실측 환경** — WebView2(Windows 전용), 포트 9223 고정(9222=Gemini Chrome 충돌 회피, `CDP_PORT`로 변경). 실측은 `eval`(invoke/DOM) 우선, `shot`은 시각 확인용.
- **핫패스 = 불변식 영역** — spawn/kill/pump·이벤트버스·epoch·replay→live 등(CLAUDE.md 핵심 불변식)이 닿으면 full로 cdp 실측까지 — 이 경로는 test PASS만으론 race·lifetime 동작을 보장 못 한다.

## 가드레일 (앞 절에 없는 금지만)

앞 절에 박힌 규약(통과 위장 금지·빌드 깨지면 멈춤·escalation-only·강도 하향 금지·실측 전엔 미완)은 반복하지 않는다. 이 절은 **다른 곳에 안 적힌 금지**만 모은다:

- **코드 파일 수정 금지** — qa는 게이트일 뿐 .rs/.ts/.tsx를 고치지 않는다. 실패는 코더(메인이 스폰)에게 돌린다. (이 스킬은 문서·검증 전용.)
- **step-log 기록 금지** — `docs/process/step-log.md`는 메인이 쓴다. qa는 결과만 보고한다.
- **게이트 생략 금지** — 강도가 정한 게이트는 전부 돈다. test가 오래 걸린다고 quick으로 임의 강등하지 않는다(escalation-only).
- **게이트 명령 통째 복붙 금지** — §2는 CLAUDE.md의 현재 바인딩 스냅샷이다. CLAUDE.md를 베껴 늘려 두 출처가 갈리게 만들지 않는다(정본 우선순위는 상단 1회 선언).
- **"코드 통과 = 동작 확인" 착각 금지** — test/tsc PASS는 회귀 안전망이지 실제 동작 증거가 아니다. UI·핫패스는 full의 cdp 실측까지 가야 동작 확인이다.
