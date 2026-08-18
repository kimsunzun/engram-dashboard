# QA 바인딩 — engram

> **ADR-0004 컨벤션:** 이 파일은 소비처 프로젝트 트리(`.claude/skill-bindings/qa.md`)에 위치한다. qa 골격(`flow.md`)이 실행 착수 시 현재 프로젝트 루트 기준 cwd-상대 경로로 Read해 실값을 꺼낸다.

골격이 "프로젝트 빌드 명령"·"프로젝트 격리 게이트"·"프로젝트 코드 불변식"이라 부르는 자리에 끼우는 **engram 전용 실명령·체크리스트**다. 골격은 스택을 모른다 — 이 파일이 engram(Cargo workspace + Tauri + React)으로 바인딩한다.

> **정본 = CLAUDE.md 「빌드·검증 명령」 절.** 이 파일은 그 **현재 바인딩 스냅샷**일 뿐이다 — 충돌하면 CLAUDE.md를 따르고 이 파일을 고친다(rot 방지). 명령을 통째 복붙해 두 출처가 갈리게 만들지 않는다. **예외 — net 격리 게이트(ADR-0129):** CLAUDE.md 의 그 대목은 스스로 "발췌"라 밝히고 게이트 1~3만 싣는다. 그 다섯 게이트의 명령 텍스트·기대값·근거 정본은 **`crates/engram-dashboard-net/src/lib.rs` 헤더**이고, 충돌하면 그 헤더를 따른다(역할 분담의 서술 = `docs/testing-strategy.md` §net). **예외 — GUI 실측 절차:** CLAUDE.md 「GUI 실측」 절은 **금지 조항만** 싣고 절차를 이 파일로 내렸다. 기동·환경변수·PID·teardown의 정본은 아래 §full이다 — 그쪽으로 되올리지 않는다.

## 프로젝트 구조 (강도·범위 매핑의 전제)

- **Cargo workspace** — 멤버 목록의 정본은 루트 `Cargo.toml`의 `[workspace] members`(여기 베끼면 갈린다). `target/`·`tests/`·실행 cwd는 워크스페이스 루트 — 단 **루트 bare `cargo test` 금지**(아래 standard 2번).
- **프론트** — `src/`(React 19 + TS + Vite), `package.json`, `vite.config.*`, `tauri.conf.json`.

**경로 → 강도 매핑(골격 §1 "변경 범위 판정"에 주입):**
- `crates/<name>/` → 해당 crate(단일이면 quick 후보)
- `src-tauri/` · 루트 `Cargo.toml` · `Cargo.lock` → **standard 이상**(workspace 영향)
- `src/` · `public/` · `index.html` · `package*.json` · `vite.config.*` · `src-tauri/tauri.conf.json` → **UI=full**(cdp 실측)
- `tests/` → **standard 이상**
- **산문 문서만 바뀐 경우** → **테스트 게이트 없음.** 테스트는 대상 자체가 없다(실측 2026-08-06: 소스 0 변경에 전체 회귀를 5회 돌려 매번 동일 결과 — 정보량 0). 판정·절차는 아래 「산문 문서 전용」 절.
- **판정 불가** → standard
- **★행이 여럿 걸리면 무거운 쪽이 이긴다★** — 위 목록은 평평하다. 한 변경이 `docs/`와 `crates/`를 같이 건드렸으면 문서 행이 아니라 crate 행으로 판정한다.

### 산문 문서 전용 (테스트 게이트 없음)

**적용 조건 — 셋 다 참일 때만:**
1. **커밋 범위**에 `.rs`·`.ts`·`.tsx`·`.toml`·`.json`·`.html`·`.css`가 **한 파일도 없다.** ★워킹트리가 아니라 커밋 범위로 판정한다★ — `git status --short`만 보면 문서를 먼저 커밋한 뒤엔 트리가 비어 무조건 참이 된다(실발동 2026-08-06). 확인 = `git diff --name-only origin/master...HEAD` + 스테이징분.
2. 바뀐 파일이 `docs/` · 루트 `*.md` · `.claude/skill-bindings/` 안에 있다. **`.claude/settings*.json`·`tauri.conf.json`·`package.json`·`Cargo.toml`은 "설정"이지만 여기 안 든다** — 위 표의 제 행으로 간다.
3. 그 변경이 **다른 문서가 실행하는 명령**을 건드리지 않았거나, 건드렸으면 아래 ②를 했다.

**할 것:** ① 조건 1을 기계로 확인한다. ② **변경된 명령만** 문자 그대로 실행해 도는지 본다(문서에 실린 전체 명령을 다 돌리는 게 아니다 — 이번 diff가 손댄 것만). 오탈자 하나로 게이트가 조용히 죽는다.

**★문서 결함을 잡는 게이트는 테스트가 아니라 `/review doc`이다★** — 그쪽은 생략하지 않는다.

**UI/프론트 영향 정의(이것만):** 위 프론트 경로가 닿았거나 **Tauri command/IPC 응답 *형식* 변경**. 이에 해당하면 full(cdp 실측 필수), 그 외 백엔드만이면 standard로 충분.

**핫패스 = 불변식 영역:** spawn/kill/pump·이벤트버스·transport·epoch·replay→live 등 동시성·lifetime 경로(CLAUDE.md "핵심 불변식")가 닿으면 full — 이 경로는 test PASS만으론 race·lifetime 동작을 보장 못 한다. **정직 note:** full의 cdp 실측 **1회 통과도 race-free 증명이 아니다** — smoke(존재 증거)일 뿐, 핫패스는 1회 관찰로 race를 배제하지 못한다(과청구 금지).

## CI와의 분담

**어느 브랜치든 push하면 CI(`.github/workflows/ci.yml`)가 standard 범위를 windows 러너에서 돌린다.** 그래서 **로컬에서 같은 것을 선행 반복하지 않는다**(사용자 결정) — 아래 강도 판정 규칙은 그대로 두고, 그 강도의 **build/test 부분을 CI 결과로 갈음**한다.

- **강도 하향이 아니다.** 경로→강도 매핑도 escalation-only도 그대로다. 바뀐 것은 *누가 돌리나*뿐이다.
- **게이트 성립 = CI 초록.** push 후 결과를 확인한다 — 초록을 못 봤으면 그 변경은 아직 게이트를 통과한 게 아니다. CI가 못 도는 상황(오프라인·워크플로 자체 수정·CI 장애)이면 아래 강도별 실명령을 로컬에서 그대로 돈다.
- **CI 미커버 3건 — 로컬 몫이다:** ① GUI 실측(창 필요) ② 실 claude 의존 테스트(워크플로가 `--skip`으로 제외하며 **그 목록이 정본**) ③ ADR-0130 재론 트리거(게이트가 아니라 알림이라 CI에 못 얹는다 — daemon crate가 닿으면 로컬에서 돌 것).
- **★아래 목록에 없고 CI에만 있는 게이트 2건★** — ts-rs 바인딩 sync(`git diff --exit-code -- crates/engram-dashboard-protocol/bindings/`, protocol 테스트 **직후**)와 discovery async 반입(`cargo tree --locked -p engram-dashboard-discovery -e normal --prefix none --target all` → `^(tokio|mio|tokio-tungstenite|futures-util) ` 0줄). 로컬 fallback으로 돌 때 이 둘을 빠뜨리면 CI보다 약하다.

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
cargo build                                 # 1) 빌드 (루트, 전 workspace). ★이걸로 지어진 engram-dashboard.exe 는 띄우지 않는다★ — TAURI_CONFIG 없이 도는 빌드라 debug 셸에 release identifier 를 다시 찍는다(ADR-0137, 정본 CLAUDE.md 「빌드·검증 명령」). 띄울 exe 는 아래 full 의 build-client-shell.mjs 가 만든다
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
  cargo test -p engram-dashboard-net                    # 게이트5a feature 0개(auth 단독 + golden) → 성공해야 PASS
  cargo test -p engram-dashboard-net --all-features     # 게이트5b server 행 → 성공해야 PASS
  ```
  게이트1·4는 매치 유무로 판정하고(0줄이어야 PASS — 코어 `use tauri` 게이트와 같은 규칙), 게이트2·3은 줄 수로 판정한다. **게이트5만 성공 여부로 판정한다**(앞 넷과 다르다 — 출력을 읽지 않는다). 게이트5가 두 줄인 이유: net의 기본 feature가 비어 있어 맨 명령은 `server` 아래 모듈을 **컴파일조차 하지 않고**, 반대로 워크스페이스 스코프 명령은 데몬이 `server`를 켜므로 항상 ON 쪽만 본다 — 각 줄이 상대가 못 보는 조합을 맡으므로 한 줄로 줄이지 않는다. `build`가 아니라 `test`인 이유는 dev-의존 경로(`auth.rs` golden이 쓰는 `serde_json`)까지 무는 것이다. 게이트2의 기대값은 **심볼 단위**다 — "`portfile.rs`만"처럼 파일 이름으로 바꾸면 그 파일 안에 새 import가 들어와도 통과한다. 게이트3은 **해석된 의존 그래프**를 읽는다 — `Cargo.toml` 텍스트 grep으로 바꾸지 말고(rename·`[dependencies.<이름>]` 테이블 형·들여쓴 선언·`[build-dependencies]`·비활성 target·`optional`이 빠져나간다 — 실측) 플래그도 줄이지 않는다. 게이트4의 패턴을 `_(이름)` 괄호 형태에서 풀어 쓰지 않는다 — 그 형태의 근거(자기일치 함정, 실측 기록)는 net crate 헤더의 게이트4 절이 정본이다.
- **★ADR-0130 재론 트리거 — 게이트가 아니다(판정 규칙이 반대)★:** 아래는 *어기면 FAIL* 인 격리 게이트가 **아니라**, 매치가 나오면 **진행을 멈추고 ADR-0130 의 재개 조건을 재론하라**는 알림이다. 매치를 회귀로 보고 되돌리지 말 것 — `control/` 이 형제 모듈을 부르는 것 자체는 결함이 아니다(근거 = ADR-0130 §영향). daemon crate 가 닿으면 필수(quick이어도). **이것이 살아 돌아가는 사본**이고 현행 판정은 여기를 쓴다. ADR-0130 §근거 ③에 같은 명령이 있는데 **그 명령줄(정규식·플래그·경로)은 이 사본과 같아야 한다** — 갈리면 날짜 차이가 아니라 **둘 중 하나가 틀린 것이니 맞춘다**(명령은 도구라 옳고 그름이 날짜와 무관하다. 얼려도 되는 것은 *결과*뿐). 규칙은 명령줄에만 걸린다 — 양쪽의 설명 주석·산문은 독자가 달라 같을 필요가 없다.
  ```bash
  rg -U -n "(crate|(super::)+)::[^;]*\b(connection_core|agent_conn|status_fanout|messaging_host)\b" crates/engram-dashboard-daemon/src/control/   # 재개 조건② — 0줄이면 보류 유지, 매치가 나오면 ADR-0130 재론
  ```
  **★매치가 나와도 바로 재론이 아니다 — 한 단계 걸러라★:** 그 줄이 **그 파일의 `#[cfg(test)]` 시작 줄보다 뒤면 테스트 픽스처라 합법**이다(ADR-0130 §영향이 옆걸음을 명시적으로 허용 — grep 은 이걸 못 가른다). 앞이면 production 간선 후보 = 재론. `rg -n "#\[cfg\(test\)\]" crates/engram-dashboard-daemon/src/control/` 로 경계 줄을 뽑아 대조한다.
  **패턴 주의(2026-08-05 리뷰 2라운드):** ① 경로 접두를 `crate::` 만으로 좁히지 말 것 — `control/mod.rs` 에서 `super::` 는 크레이트 루트라 같은 간선이 빠져나간다. ② **`-U` 를 빼지 말 것** — rustfmt 가 쪼갠 그룹 import(`use crate::{`⏎`  connection_core::A,`)는 접두와 모듈명이 다른 줄이라 단일행 패턴이 못 문다. ③ **양성 대조(경로를 `src/` 로 넓히기)로 접두 축소를 승인하지 말 것** — 현재 실간선은 전부 `crate::` 형태라 `super::` 갈래가 죽어도 그 대조는 통과한다.
  **커버리지는 조건 ②뿐이다.** ①(제어 평면·중계 층을 따로 쓸 소비자가 실제로 생김)은 사람 판단이라 기계화 대상이 아니고, ③(production 의존 그래프의 순환)은 단발 명령이 없다 — 재는 법의 정본은 ADR-0130 §영향 조건 3. **이 블록이 0줄이라고 "순환 없음"으로 읽지 말 것.** ★**등록 상태 자체의 정본은 여기가 아니라 ADR-0130 §영향의 "등록 상태" 줄이다**★ — ③이 나중에 등록되면 그 줄만 갱신되고 이 문단은 낡는다. 상태를 판단할 땐 거기를 볼 것.
- **공유 데몬 바이너리 락(실발동 2026-07-08):** 실행 중인 `engram-dashboard-daemon.exe`(공유 인프라 — 타 에이전트 호스팅 가능)가 있으면 daemon bin을 빌드하는 루트 `cargo build`·`cargo test`가 os error 5로 FAIL한다 — 코드 결함 아님. **데몬 강제 종료 금지.** 우회 = daemon bin을 안 빌드하는 패키지 스코프(`cargo build/test -p <영향 crate들>`)로 좁혀 회귀 확인, 워크스페이스 전체 게이트는 **PARTIAL로 정직 보고**(못 돌린 범위 명시).

### full — standard + GUI 실측 (cdp)

standard 게이트를 전부 PASS시킨 뒤, 실제 앱을 띄워 화면 동작을 확인한다(**Windows 전용** — WebView2 CDP, 포트 9223 고정).

**★앱을 셸에서 직접 띄우지 않는다★** — 셸에서 띄우면 앱이 터미널의 자손이 되고 **앱 출력이 그 사슬을 거슬러 올라간다.** 그 조합에서 터미널이 반복 크래시해 실측이 통째로 날아간다(실측 2026-08-16). **끊어야 할 조건이 둘이다 — 프로세스 트리 밖 + 출력은 파일로만.** `start`·백그라운드 잡·`nohup`은 둘 다 못 끊으므로 대체재가 아니다.

**사람이 손으로 볼 때는 `scripts/`의 `run-*.bat` 런처를 쓴다**(목록·용도 = README) — 빌드·dev 서버·분리 실행을 한 번에 한다. 아래는 그 런처가 하는 일을 게이트에서 단계별로 돌리는 형태다.

**★게이트에서는 런처를 쓰지 말고 아래 단계를 쓴다★** — 디버그 런처는 dev 서버를 자식으로 남기므로, 출력을 파이프로 받으면 **dev 서버가 죽을 때까지 파이프가 안 닫혀 호출이 매달린다**(실측 2026-08-17). 사람이 창에서 쓸 땐 무해하다.

아래는 **Git Bash 한 셸에서 전부 돈다**(실측 2026-08-17). PowerShell로 옮겨 쓰지 말 것 — teardown이 Git Bash 전용 접두를 쓴다.

```bash
# 0) 이번 변경을 담은 빌드를 만든다 + dev 서버를 띄운다(디버그 빌드는 화면을 품지 않는다)
export CLIENT_EXE="$(node scripts/build-client-shell.mjs)"   # ★클라이언트 셸은 이걸로만★(ADR-0137 — 아래 첫 불릿). ★빈 값이면 빌드 실패다 → 여기서 멈춘다★(경로는 stdout 한 줄, 진행·에러는 stderr). 백엔드/데몬을 고쳤으면 `cargo build -p engram-dashboard-daemon` 도 — ★조건부다★: 데몬이 떠 있으면 그 명령은 os error 5 로 하드 FAIL 한다(아래 "공유 데몬 바이너리 락"), 그러니 무조건 붙여 돌리지 말 것
# ★1420이 떠 있어도 그냥 재사용하지 말 것 — 그게 이 워크트리 것인지 먼저 확인한다★(아래 절)
powershell -NoProfile -Command "\$c = Get-NetTCPConnection -LocalPort 1420 -State Listen -ErrorAction SilentlyContinue; if (\$c) { (Get-CimInstance Win32_Process -Filter \"ProcessId=\$(\$c[0].OwningProcess)\").CommandLine }"
nohup npm run dev > /tmp/engram-vite.log 2>&1 & disown   # 위가 빈 출력일 때만(= 아무도 안 잡고 있을 때만)
curl -s -o /dev/null --retry 60 --retry-delay 1 --retry-connrefused --max-time 120 http://localhost:1420
# 1) 분리 실행으로 기동 — 스케줄러가 새 프로세스를 만들어 터미널 트리 밖에 둔다
powershell -NoProfile -Command "& './scripts/launch-detached.ps1' -Exe \$env:CLIENT_EXE -EnvVars 'WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223'"
#    성공 시 stdout 마지막 두 줄 = LOG=<로그경로> · PID=<pid>   ★PID를 기록한다★(teardown이 이걸 쓴다)
#    로그까지 켜려면 값을 콤마로 잇는다 — 반드시 작은따옴표 각각:
#      ... -EnvVars 'WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223','RUST_LOG=debug'
# 2) 포트 뜰 때까지 대기 — ★재시도 필수★(PID 반환 ≠ 포트 준비. 단발 curl은 첫 거부와 기동 실패를 못 가른다)
curl -s -o /dev/null -w 'cdp=%{http_code}\n' --retry 60 --retry-delay 1 --retry-connrefused --max-time 120 http://127.0.0.1:9223/json/version
# 3) 실측
node scripts/cdp.mjs info                   # 페이지 목록 확인
node scripts/cdp.mjs eval "<js>"            # 앱 안 JS·실제 invoke 호출 (spawn/write/interrupt/kill 등 IPC 검증)
node scripts/cdp.mjs shot out.png           # 필요시 스크린샷 → Read로 확인
```
- **★띄우는 exe 경로도 하드코딩하지 말 것★** — `target/debug`는 기본값일 뿐이라 `CARGO_TARGET_DIR`·`.cargo/config.toml`의 `build.target-dir`이 산출물을 딴 데로 돌린다. 그러면 옛 경로에 **남아 있는 낡은 exe**가 존재 검사를 통과해 그걸 띄우고, 이번 변경이 안 담긴 바이너리로 실측하고도 통과로 오판한다. 0)이 `cargo metadata`로 실측한 경로를 `$CLIENT_EXE`로 물려주므로 그대로 쓴다(`\$env:` 이스케이프는 Git Bash가 `$env`를 자기 변수로 먹는 것을 막는 것 — 값은 PowerShell이 환경에서 직접 읽으므로 경로에 작은따옴표가 있어도 안전하다).
- **★클라이언트 셸을 생 `cargo build -p engram-dashboard`로 짓지 말 것(ADR-0137)★** — 그 명령은 Tauri CLI를 지나지 않아 dev 오버레이(`src-tauri/tauri.dev.conf.json`)가 안 먹고 **release identifier가 찍힌다.** `TAURI_CONFIG`는 `cargo:rerun-if-env-changed`라 *변수를 빼는 것만으로도 재빌드가 돌아* 멀쩡하던 exe를 되돌려 놓는다(실측). 그러면 릴리즈 앱이 떠 있는 동안 실측 대상이 **창도 없이 즉시 죽어** 게이트가 앱 결함으로 오판한다. `scripts/build-client-shell.mjs`가 주입과 산출물 대조를 함께 하며 **런처·이 게이트가 같은 구현을 쓴다**(위 "런처를 쓰지 말 것"은 그대로 유효 — 저건 dev 서버 자식 때문이고, 이 스크립트는 앱을 띄우지 않는다).
- **★1420을 남이 잡고 있으면 디버그 경로를 쓰지 않는다 — 릴리스로 간다★(실측 2026-08-17):** 디버그 빌드는 화면을 품지 않고 1420에서 받아오는데 그 포트는 **먼저 잡은 워크트리 것**이다. 남의 vite를 재사용하면 **남의 화면을 측정하고 통과로 오판한다** — 앱은 정상으로 보이므로 눈치챌 단서가 없다. 위 0)의 명령줄 출력에 **지금 워크트리 경로가 아닌 다른 경로**가 보이면(예: `...engram-dashboard-wt2\...\vite.js`) 그 vite를 쓰지 말고, 사용자에게 그 프로세스를 알리고 릴리스 경로로 전환한다(릴리스 exe는 화면을 품어 포트가 필요 없다). **남의 vite를 죽이지 않는다** — 그 워크트리에서 다른 작업이 돌고 있을 수 있다.
- **★릴리스로 갈 땐 순수 `cargo build --release`가 아니다★(실측 2026-08-18):** 그렇게 만든 exe는 여전히 `localhost:1420`을 로드해 **같은 함정에 그대로 빠진다.** 화면을 품은 exe는 `npm run tauri build -- --no-bundle`이 만든다(근거·경고 = `scripts/build-release.ps1` 헤더). 띄운 뒤 앱 안에서 URL을 확인해 `http://tauri.localhost/`인지 본다 — `localhost:1420`이면 잘못된 빌드를 측정하는 것이다.
- **★`-Command`를 `-File`로 바꾸지 말 것★** — `-File`은 뒤 인자를 전부 문자열 리터럴로 넘겨 `'A=B','C=D'`가 **한 값 `A=B,C=D`로 뭉개진다.** 그러면 포트 인자가 오염돼 **9223이 안 열리는데 스크립트는 PID를 정상 반환**한다 — 게이트가 조용히 죽는 경로다(실측 2026-08-17). 경로에 `\`를 쓰는 것도 금지 — Git Bash가 먹어서 `exe not found`가 난다. 슬래시로 쓴다.
- **★환경변수는 상속되지 않는다★** — 새 프로세스를 스케줄러가 만들어서 현재 셸의 `$env:...`가 안 넘어간다. 디버그 포트·`RUST_LOG`는 반드시 `-EnvVars`로 넘긴다. 빠뜨리면 포트가 안 열려 "왜 9223이 안 뜨지"로 헤맨다.
- **★실측 대상이 이번 변경을 담은 빌드인지 먼저 확인한다★** — 분리 실행은 **이미 만들어진 exe**를 띄운다. 소스를 고치고 재빌드 없이 띄우면 옛 바이너리로 실측하고 **통과로 오판한다**. `node scripts/build-client-shell.mjs`는 **데몬을 빌드하지 않는다** — 백엔드를 고쳤으면 `-p engram-dashboard-daemon`도 돌린다(안 그러면 옛 데몬에 붙어 Rust 변경이 조용히 무효가 된다, ADR-0029).
- **디버그 대신 릴리스로 볼 수도 있다** — `target/release/engram-dashboard.exe`는 화면을 품고 있어 dev 서버가 필요 없고 렌더가 즉시다. 대신 빌드가 4분대다(실측 2026-08-17). 반복 확인엔 디버그가 낫다 — dev 서버를 살려두면 재기동 렌더가 1초 안쪽이다.
- **★`target/release/`와 `release/`는 다른 배포판이다★** — 전자는 위 런처가 만들고, 후자는 `scripts/build-release.ps1`이 조립하는 portable 폴더다. 데이터 폴더도 각자라 섞으면 엉뚱한 데몬·엉뚱한 로스터를 본다.
- **★다른 배포판 앱이 떠 있는 것은 정상이다(ADR-0137) — 막아야 할 것은 그게 아니다★** — 스크립트는 이미지 이름이 아니라 **exe 경로**로 새 PID를 가리므로 남의 워크트리·release 앱은 애초에 후보에 안 든다. 실제 제약은 **이 배포판의 다른 프로세스가 20초 폴링 창 안에 뜨면 안 된다**는 것뿐이다(여럿이면 첫 번째만 쓴다). 같은 배포판의 잔여 앱을 먼저 확인한다.
- **기동 실패 판정** — 20초 안에 이 배포판의 새 프로세스를 못 찾으면 `LAUNCH_FAILED (no new ... within 20s). log: <경로>` + 로그 꼬리 20줄, 종료코드 1. 실측 실패로 보고한다.
- **앱 출력은 화면에 안 나온다** — 전부 `LOG=` 파일로 리다이렉트된다(기본 `%TEMP%\detached-<exe이름>-<이번 실행 태그>.log` — ★실행마다 새 파일이다★. exe 이름만으로 가르면 워크트리·debug/release가 전부 같은 파일에 몰려 먼저 뜬 앱이 다음 앱의 기동을 막는다. 경로를 짐작하지 말고 `LOG=`·`LAUNCH_FAILED` 줄에 찍힌 것을 쓴다). `RUST_LOG` 미설정이면 앱이 stdout에 아무것도 안 써서 **로그 0바이트가 정상**이다 — 빈 로그를 기동 실패로 읽지 말 것. **거꾸로 `RUST_LOG=debug`를 넘겼는데도 0바이트면 위 `-EnvVars` 문법이 깨진 것이다**(정상이면 수 KB — 실측 2026-08-17: 0 B → 1672 B).
- 포트 9223 고정(9222=Gemini Chrome 충돌 회피, `CDP_PORT`로 변경).
- **검증엔 스샷보다 `eval` 텍스트가 토큰·정확도 유리**(픽셀 해석 회피) — DOM 텍스트·`window.__TAURI__.core.invoke(...)` 결과를 직접 확인. shot은 레이아웃·시각 확인이 필요할 때만.
- 변경이 닿은 동작을 실제로 한 번 통과시켜 본다(예: spawn → 출력 도착 → kill → 상태 전이). **이게 통과해야 동작 확인 = 완료**.
- **teardown — 자기가 띄운 건 자기가 치운다(실발동 2026-07-10):** 1)에서 받은 **PID**로 종료한다 — `MSYS_NO_PATHCONV=1 taskkill /PID <기록한PID> /T /F`(Git Bash면 접두 필수 — 안 붙이면 `/PID`가 경로로 변환된다). **`/T`가 잡는 것 = 앱 + 그 WebView2 렌더러 자식들**뿐이다(옛 "런처 트리" 모델이 아니다 — 분리 실행엔 런처 부모가 없다). 앱을 감싼 임시 `cmd`는 자식이 아니라 **부모**라 `/T`가 안 건드리고, 앱이 끝나면 스스로 빠진다.
- **★dev 서버(vite)도 `/T`에 안 걸린다 — 데몬과 같은 소유권 규칙으로 따로 치운다★(누락 적출 2026-08-18):** vite는 앱과 무관한 별도 프로세스라 **앱을 닫아도 1420을 계속 잡고 있다.** 위 0)에서 **자기가 띄웠으면** 자기가 끈다(그 `npm run dev` 잡의 pid). **실측 시작 전부터 떠 있던 것은 불가침** — 사람이나 다른 워크트리 세션 것일 수 있다. 남기면 다음 실측이 그걸 재사용해 남의 화면을 측정한다(위 「1420을 남이 잡고 있으면」).
- **★데몬은 위 `/T`에 안 걸린다 — 따로 판단한다★** — 앱이 데몬을 WMI(`Win32_Process.Create`)로 띄워 부모가 `WmiPrvSE.exe`가 되기 때문이다(근거 = `crates/engram-dashboard-discovery/src/lib.rs` `wmi_spawn` 주석 · 실측 2026-08-17). 처리는 **소유권으로 갈린다:**
  - **실측 시작 전부터 떠 있던 데몬 = 불가침.** 죽이지 않는다(persist 모델·타 에이전트 호스팅 가능 — 에이전트는 데몬의 자식이라 죽이면 진행 중인 작업이 날아간다). 그래서 **기동 전에 데몬 유무를 기록해 둔다**(위 "잔여 프로세스 확인"과 같은 단계).
  - **이번 실측이 띄운 데몬 = 자기가 치운다.** 남기면 그 배포판의 데몬 exe가 잠겨 **다음 재빌드가 하드 실패한다**(`scripts/build-release.ps1`이 "앱과 데몬을 완전히 종료한 뒤 다시 실행하세요"로 멈춘다). 죽이기 전 `ExecutablePath`가 이번에 띄운 배포판(`target/debug/` 또는 `target/release/`) 것인지 **반드시 확인한다** — 이미지 이름만 보고 죽이면 남의 배포판 데몬을 죽인다(ADR-0139).
- **비-Windows에선 cdp 불가** → standard까지가 한계 + "동작 미확인" 정직 보고(골격 §4).

## 실패 보고 시 게이트 명칭 (골격 §3에 주입)

어디서 막혔는지 짚을 때 쓰는 게이트 이름: build / test(어느 테스트) / fmt / 격리(어느 crate의 어느 게이트 + 매치 줄 또는 실제 줄 수) / tsc(타입체크) / npm(프론트 테스트) / cdp 실측(어느 동작).

## flaky·타이밍·perf 실패 = 매직넘버로 통과 금지 (ADR-0038)

flaky/타이밍/perf 실패를 상수·임계값·재시도 튜닝으로 통과시키려 하면 중단하고 `docs/reference/debugging-conventions.md`(OSS 조사 전환)를 적용한다. (이 규약의 *발화 지점* — qa가 신호를 잡는 곳.)

## 코어 격리 불변식 (정본 = ADR-0003 + 코드의 `// ADR-` 앵커)

코어 crate(`engram-dashboard-core`)는 **Tauri import 0** — `rg "^\s*use tauri" crates/engram-dashboard-core/src/` → 0줄. 이게 깨지면 코어가 전송 방식에 묶인 것 = 회귀. (근거·거부 대안은 ADR-0003.)
