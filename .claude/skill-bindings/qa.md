# QA 바인딩 — engram

> **ADR-0004 컨벤션:** 이 파일은 소비처 프로젝트 트리(`.claude/skill-bindings/qa.md`)에 위치한다. qa 골격(`flow.md`)이 실행 착수 시 현재 프로젝트 루트 기준 cwd-상대 경로로 Read해 실값을 꺼낸다.

골격이 "프로젝트 빌드 명령"·"프로젝트 격리 게이트"·"프로젝트 코드 불변식"이라 부르는 자리에 끼우는 **engram 전용 실명령·체크리스트**다. 골격은 스택을 모른다 — 이 파일이 engram(Cargo workspace + Tauri + React)으로 바인딩한다.

> **역할 분담 — 셋이 각각 다른 것의 정본이다.** ① **CLAUDE.md 「빌드·검증 명령」 = *규칙*의 정본**(무엇을 반드시 지키나). ② **이 파일 = *실행*의 정본**(어떤 명령을, 어느 강도에서, 무엇을 기대하며 돌리나 — 명령줄·플래그·기대 줄 수·절차·실측 근거). ③ **`.github/workflows/ci.yml` = *게이트 명단*의 정본**(무엇이 게이트인가).
>
> CLAUDE.md는 명령줄·기대값을 싣지 않으므로 그쪽에서 명령을 찾지 말 것이고, **규칙과 충돌하면 CLAUDE.md를 따르고 이 파일을 고친다.** ★**CI에 있는데 여기 없는 게이트를 발견하면 그건 드리프트다 — 이 파일을 채운다**★(실발생 2026-08-19: 메시징 격리 정규식이 CI보다 약했고, 의존 상한 게이트 2종이 빠져 있었으며, 바인딩 sync fallback이 옛 명령이었다. 셋 다 *실행되는* 사본만 뒤처진 방향이었다).
>
> **예외 — net 격리 게이트(ADR-0129):** 그 다섯 게이트의 명령 텍스트·기대값·근거 정본은 **`crates/engram-dashboard-net/src/lib.rs` 헤더**이고, 충돌하면 그 헤더를 따른다(역할 분담의 서술 = `docs/testing-strategy.md` §net). ★**command crate 헤더는 예외가 아니다**★ — `crates/engram-dashboard-command/src/lib.rs` 불변식 1이 적어 둔 게이트는 매니페스트 텍스트 정규식(`rg "path\s*=" … Cargo.toml`)인데, CI는 그것이 못 보는 형태(따옴표 종류·`[build-dependencies]`) 때문에 **처음부터 `cargo tree` 상한 게이트로 세웠다.** 그 헤더 줄은 CI·이 파일보다 약하므로 **실행은 아래 「의존 상한 게이트 2종」을 쓴다**(헤더 수정은 코드 변경이라 별건). **예외 — GUI 실측 절차:** CLAUDE.md 「GUI 실측」 절은 **금지 조항만** 싣고 절차를 이 파일로 내렸다. 기동·환경변수·PID·teardown의 정본은 아래 §full이다 — 그쪽으로 되올리지 않는다.

## 프로젝트 구조 (강도·범위 매핑의 전제)

- **Cargo workspace** — 멤버 목록의 정본은 루트 `Cargo.toml`의 `[workspace] members`(여기 베끼면 갈린다). `target/`·`tests/`·실행 cwd는 워크스페이스 루트 — 단 **루트 bare `cargo test` 금지**(아래 standard 2번).
- **프론트** — `src/`(React 19 + TS + Vite), `package.json`, `vite.config.*`, `tauri.conf.json`.

**경로 → 강도 매핑(골격 §1 "변경 범위 판정"에 주입):**
- `crates/<name>/` → 해당 crate(단일이면 quick 후보)
- `src-tauri/` · 루트 `Cargo.toml` · `Cargo.lock` → **standard 이상**(workspace 영향). ★`src-tauri/`는 standard 2번이 통째로 빼는 유일한 패키지다★ — 이 행이 집어 오는 실제 커버리지는 **standard 2b·2c·2d**(아래 — `src-tauri/tests/`의 통합 타깃 전부)이고, 그 줄들이 빠지면 이 경로 변경은 게이트가 0이다. ★**`src-tauri/tests/`에 파일이 늘면 이 목록도 함께 늘린다**★ — 워크스페이스 회귀가 그 패키지를 통째로 빼므로 새 스위트는 등재되기 전까지 로컬 신호가 0이다(실발생: `daemon_client_pending`이 CI에만 있었다).
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
- **★로컬과 CI의 명령줄은 바이트 단위로 같지 않다 — 셋 다 의도된 차이다★.** 드리프트로 보고 맞추지 말 것(게이트의 *내용*은 같다):
  1. **CI는 `--locked`를 붙인다** — 커밋된 `Cargo.lock`이 낡았을 때 조용히 재해석하지 않고 실패시킨다.
  2. **CI는 실 claude 의존 테스트를 이름으로 `--skip` 한다** — 러너에 claude가 없다. **그 `--skip` 목록이 정본**이고, 그 축의 CI 커버리지는 0이라 로컬(claude 있음)에서만 검증된다.
  3. **로컬은 모든 빌드·테스트를 `scripts/run-detached.ps1`로 돌리지만 CI는 그냥 돈다** — 러너는 출력을 이미 자기 잡 로그에 담고 그 로그를 읽어 갈 세션 터미널도 없으니, 감싸 봐야 얻는 게 없고 CI만 느려진다(로컬에서 감싸는 이유 = 아래 「분리 실행」).
- **CI 미커버 3건 — 로컬 몫이다:** ① GUI 실측(창 필요) ② 실 claude 의존 테스트(워크플로가 `--skip`으로 제외하며 **그 목록이 정본**) ③ ADR-0130 재론 트리거(게이트가 아니라 알림이라 CI에 못 얹는다 — daemon crate가 닿으면 로컬에서 돌 것).
- **★아래 강도별 목록에 없고 CI에만 있는 게이트★**(개수를 세지 않는다 — 세던 숫자는 게이트가 늘 때마다 뒤처진다). 로컬 fallback으로 돌 때 빠뜨리면 CI보다 약하다:
  ```bash
  # ts-rs 바인딩 sync — protocol·core 테스트를 돌린 **직후**(양쪽 다 생성물을 다시 굽는다)
  git add -N -f -- crates/engram-dashboard-protocol/bindings/ crates/engram-dashboard-core/bindings/
  git diff --exit-code -- crates/engram-dashboard-protocol/bindings/ crates/engram-dashboard-core/bindings/
  # discovery async 반입 → `^(tokio|mio|tokio-tungstenite|futures-util) ` 매치 0줄이어야 PASS
  cargo tree --locked -p engram-dashboard-discovery -e normal --prefix none --target all
  ```
  ★**`git add -N`(intent-to-add)를 빼지 말 것**★ — `git diff`는 **untracked 파일을 보지 않는다.** 처음 생성되는 `.ts`는 비교 대상 자체가 없어 매치 없이 조용히 통과했다(`SlotContent.ts` 사건 — `docs/process/S20-command-bus/trd.md` §5). intent-to-add로 두 디렉터리를 **내용 없이** 인덱스에 올려 두면 신규 파일도 "전체가 추가된 diff"로 잡힌다 — 빈 blob 등록일 뿐이라 이후 커밋 내용에는 영향이 없다. ★**`-f`도 빼지 말 것**★ — `git add`는 무시된 경로를 조용히 건너뛰므로, 어느 crate의 `.gitignore`에 `bindings/`·`*.ts` 같은 넓은 줄이 생기면 생성물이 인덱스에 안 올라가 **게이트가 소리 없이 해제된다**(지금은 두 디렉터리 다 무시 대상이 아니다). ★**경로를 하나로 줄이지 말 것**★ — 선언이 crate마다로 흩어져 생성 지점이 둘이고, 새 생성 지점이 생기면 이 목록도 함께 늘어난다.

## 강도별 실명령 (골격 §2 "게이트 실행"에 주입)

모두 **워크스페이스 루트에서** 실행한다. 게이트 순서(빌드 → 테스트 → 격리 → 타입체크·프론트 → 실측)·실패 시 멈춤은 골격이 강제한다.

### ★분리 실행 — 아래 명령은 하나도 셸에서 직접 돌리지 않는다★

**빌드·테스트를 포함해 이 절의 모든 명령은 `scripts/run-detached.ps1`을 거친다.** 예외 없다. **규칙 자체의 정본은 CLAUDE.md 「빌드·검증 명령」**이고, **근거·실측·사용법의 정본은 이 절과 그 스크립트 헤더다.**

- **왜 분리 실행인가 — 이유는 출력 처리다.** 출력이 **파일로만** 떨어져 **판정에 필요한 줄만 골라 읽는다.** 셸에서 직접 돌리면 빌드 로그 전체가 도구 결과로 거슬러 올라와 컨텍스트를 통째로 먹는다.
- ★**크래시 회피는 이 규칙의 이유가 아니다 — 그렇게 적혀 있던 옛 문장은 오진이었다**★(정정 2026-08-19). 세션을 함께 데려가던 터미널 크래시의 실제 원인·증거·해법은 아래 「터미널 크래시」 항목이 갖는다. **`start`·백그라운드 잡·`nohup`은 로그 파일 + 아래 `__EXIT` 마커 계약을 주지 않으므로 대체재가 아니다.**

```bash
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-detached.ps1 -Command "<아래 명령 한 줄>" -WorkDir "<워크스페이스 루트>" -LogFile "<로그경로>.log"
```

- 즉시 `PID=`·`LOG=`·`BAT=`을 찍고 돌아온다 — **반환 = 시작했다는 뜻이지 끝났다는 뜻이 아니다.**
- ★**완료 판정 = 로그 마지막 줄의 `__EXIT=<종료코드>` 마커**★ — 그 마커가 나타날 때까지 폴링한다. **프로세스가 사라진 것으로 판정하지 말 것**(래퍼 `cmd`가 자식보다 먼저 빠질 수 있다). PASS/FAIL은 그 종료코드로 가른다.
- **예외 — vitest(`npm test`)는 `__EXIT`가 안정적으로 안 붙는다**(자식이 래퍼보다 오래 산다). 그 줄만 **로그에 찍힌 vitest 자신의 pass/fail 요약**으로 판정한다.
- 판정이 끝나면 로그를 읽되 **필요한 줄만 인용한다** — 출력이 파일로 떨어지는 덕에 빌드 로그 전체를 삼키지 않아도 된다.
- **`rg`·`cargo tree` 격리 게이트처럼 출력이 몇 줄뿐인 단발 조회는 굳이 감쌀 필요가 없다** — 그대로 받아도 컨텍스트를 먹지 않고 판정도 그 몇 줄로 끝난다. 다만 감싸도 무해하니, 애매하면 감싼다.
- **§full의 「앱을 셸에서 직접 띄우지 않는다」와 같은 규칙이다** — 앱·빌드·테스트, **우리가 띄우는 것은 전부 프로세스 트리 밖 + 출력은 파일로만.** 대상별 절차만 다르다(앱 = `launch-detached.ps1`, 빌드·테스트 = `run-detached.ps1`).
- ★**옛 규칙은 폐기했다 — 되살리지 말 것**★(2026-08-19). 옛 규칙 = **libtest 테스트 스레드 수 상한 플래그**(워크스페이스·core·daemon 회귀 뒤에 `4`를 박아 두던 그 인자 — ★이 문서에서 그 플래그를 쓰는 자리는 이제 없다★). 원인이 규명된 뒤 폐기 사유는 **더 분명해졌다:**
  1. **실제 원인에 닿지도 않는다.** 원인은 터미널 자신의 프로세스 트리 재귀(아래 「터미널 크래시」)이고 그 재귀는 **단기 프로세스가 대량으로 나고 죽을 때** 걸리는데, 그 플래그는 *테스트*가 몇 개 동시에 도는지만 정한다. 2026-08-19 12:58 터미널은 테스트가 하나도 돌기 전 **컴파일·링크 단계**에서 죽었다(그 시점 디스크의 테스트 바이너리는 전날 것이고 라이브러리 산출물만 갓 쓰여 있었다) — 플래그가 닿은 적 없는 자리다. *컴파일러* 프로세스 수는 cargo의 build jobs가 정한다.
  2. **두 번째 후보(프로세스 트리 위치)도 원인이 아니었다.** 분리 실행으로 옮긴 뒤에도 터미널은 죽었다 — 15:18:51 분리 실행 `cargo test` 기동, 15:18:59 터미널 사망(실측). 즉 테스트 동시성도, 트리 위치도 레버가 아니었다. 실제 수정은 **터미널 업그레이드**다(아래 「터미널 크래시」).
- **터미널 크래시(`0xc00000fd`) — 원인 규명·수정 완료(2026-08-19).** ★**사내 보안 에이전트 탓이 아니다**★:
  - **증상:** 그날 세 번(12:58 · 15:18 · 15:31) `wezterm-gui.exe`가 `0xc00000fd`(STACK_OVERFLOW)로 죽고 **세션까지 함께 내려갔다** — 돌던 게이트가 통째로 날아갔다.
  - **원인:** `wezterm-gui`는 각 pane에서 무엇이 돌고 있나(탭 제목·상태)를 판정하려고 **시스템 전역** 프로세스 표(`CreateToolhelp32Snapshot`)를 떠서 부모→자식 트리를 **재귀로** 만든다. 그 재귀에 **순환 가드가 없었다.** 단기 프로세스가 대량으로 나고 죽으면 PID 재사용으로 부모 맵에 순환이 생기고, 재귀가 끝없이 내려가 스레드 스택이 넘친다.
  - **상류 증거:** wezterm issue **#7705** "Stack overflow in build_proc due to process tree cycle on Windows"(2026-04-01 개설, 2026-06-07 종결) → **PR #7706**이 `build_proc`에 `visited` 집합을 넣어 고쳤다. 보고자 환경이 이 머신과 일치하고(Windows 11 Enterprise 10.0.26200 · `0xc00000fd`), 트리거도 "WezTerm 탭에서 Claude Code 세션 여럿을 돌려 서브프로세스를 대량 생성"이었다. 다른 보고자는 24시간에 3회를 기록했는데 이 머신도 그날 정확히 3회다.
  - **로컬 증거:** 크래시 덤프 3개가 모두 같은 반복 스택 패턴이고 재귀 깊이가 약 780단이다.
  - **왜 첫 분석이 보안 에이전트를 범인으로 짚었나:** 스택 오버플로는 **가드 페이지를 건드린 프레임**이 범인 모듈로 찍힐 뿐이다 — 한 번은 `ntdll.dll`, 한 번은 SentinelOne 에이전트의 `InProcessClient64.dll`이었다. 그 DLL이 모든 프로세스에 주입돼 있다는 것은 여전히 사실이지만 **이 크래시의 원인은 아니다**(무고한 구경꾼).
  - **왜 분리 실행이 못 막았나:** 그 스냅샷은 **시스템 전역**이라, 일을 wezterm 서브트리 밖으로 옮겨도 훑는 표에서 그 프로세스들이 빠지지 않는다. 직접 실측 — 15:18:51 분리 실행 `cargo test` 기동, 15:18:59 터미널 사망.
  - ★**버전 경계 — 다시 만나면 여기부터 본다**★: dated release는 `20240203`까지 **버그판**이고(이 머신에 설치돼 있던 것 = `20240203-110809-5046fc22`, 수정보다 두 해 이상 오래됐다), 수정은 **2026-06-07**에 들어갔으며 그 뒤 dated release가 없어 **nightly에만 실린다.** **`wezterm-gui.exe`에서 `0xc00000fd`를 또 만나면 가장 먼저 확인할 것 = 지금 돌고 있는 wezterm 빌드가 무엇인가.**
  - **적용된 해법:** nightly `20260819-012343-33891b4a`를 `I:\Engram\tools\wezterm_new\`에 풀어 그것으로 쓰고 있다(config·런처 동봉 — 런처는 자기 폴더 기준 상대 경로). 옛 폴더 `I:\Engram\tools\wezterm\`는 손대지 않아 2024 빌드 그대로다.

**프론트 게이트 확정 절차:** ① `npm test`(package.json `scripts.test` = `vitest run`). ② 타입체크는 `npm run typecheck`가 있으면 우선, **없으면 `npx tsc --noEmit`**(현재 package.json엔 typecheck 스크립트 없음 → `npx tsc --noEmit`). ③ 스크립트가 아예 없으면 실행하지 말고 package.json 실제 스크립트명을 사용자에게 보고한다. **프론트 린트 게이트는 정본(CLAUDE.md·package.json)에 없음 — 임의로 lint를 추가하지 않는다.**

### quick — 영향 crate만

영향받은 멤버만 좁게 돌린다(예: core만 바뀐 경우):
```bash
cargo build -p engram-dashboard-core        # 빌드
cargo test  -p engram-dashboard-core        # 영향 crate 테스트
```
- **★좁혀 돌릴 때도 분리 실행이다★** — 위 「분리 실행」 절은 quick에도 그대로 걸린다. 한 crate짜리 명령이라고 셸에서 직접 돌리지 않는다(좁혀 돌려도 컴파일 로그는 길다 — 이유는 크래시 회피가 아니라 **출력 처리**이고 근거는 그 절).
- **core crate가 닿으면 격리 게이트도 포함**(quick이어도 — 명령·판정은 아래 standard 4번): quick의 `cargo test -p`만으론 Tauri import 회귀를 못 잡아 false PASS가 난다.
- 프론트가 닿았으면(quick 범위라도) 프론트 게이트(위 확정 절차): `npm test` + `npx tsc --noEmit`.

### standard (기본) — workspace 전회귀 + 격리 + 프론트

순서대로:
```bash
cargo build                                 # 1) 빌드 (루트, 전 workspace). ★이걸로 지어진 engram-dashboard.exe 는 띄우지 않는다★ — TAURI_CONFIG 없이 도는 빌드라 debug 셸에 release identifier 를 다시 찍는다(ADR-0137, 정본 CLAUDE.md 「빌드·검증 명령」). 띄울 exe 는 아래 full 의 build-client-shell.mjs 가 만든다
cargo test --workspace --exclude engram-dashboard   # 2) 전 멤버 회귀 — src-tauri 패키지(`engram-dashboard`)만 뺀다. 루트 bare cargo test 금지(src-tauri lib 타깃이 0xc0000139 STATUS_ENTRYPOINT_NOT_FOUND 로 죽는다 — 실측 2026-08-05). ★옛 `-- --test-threads=4` 는 폐기했다 — 되살리지 말 것★(2026-08-19: 그 플래그는 실제 원인에 닿지도 않았다. 사유의 정본 = 아래 「분리 실행」)
cargo test -p engram-dashboard --test layout_apply                      # 2b) 2번이 뺀 그 패키지의 **통합** 타깃 — 죽는 건 lib 타깃뿐이고 통합 타깃은 정상 기립한다(실측 2026-08-17). 2번이 이 스위트를 못 보므로 이 줄이 유일한 실행 경로다(아래 둘째 항목)
cargo test -p engram-dashboard --test layout_commands                   # 2c) 같은 패키지의 또 다른 **통합** 타깃 — layout_apply 는 적용 서비스 자체, 이쪽은 명령 선언(`layout::commands`) + 인바운드 수신기(데몬 명령을 적용 서비스로 라우팅)를 잰다. 판정 사유는 2b 와 동일(0xc0000139 는 lib 타깃뿐, 통합 타깃은 정상 기립·실측 2026-08-17) — 2번이 이 스위트도 못 보므로 이 줄이 유일한 실행 경로다
cargo test -p engram-dashboard --test daemon_client_pending             # 2d) 같은 패키지의 셋째 **통합** 타깃 — 겹친 `request_id` 가 연결 태스크를 패닉시키지 않고 옛 대기자가 영구 hang 대신 오류로 깨어나며 새 요청이 그 번호를 승계함을 잰다(GUI 실측 2026-08-18 결함의 회귀망). 판정 사유는 2b 와 동일(0xc0000139 는 lib 타깃뿐, 통합 타깃은 정상 기립) — 2번이 이 스위트도 못 보므로 이 줄이 유일한 실행 경로다
cargo test -p engram-dashboard --test daemon_client_replay              # 2e) 같은 패키지의 넷째 **통합** 타깃 — 거절당한 구독이 슬롯을 풀고 병합된 다음 세대 `Subscribe` 가 실제로 **만들어져 보낼 명령으로 돌아오며** 이미 acked 된 구독은 거절로 풀리지 않음을 잰다(2026-08-19 출력 두절 결함의 회귀망). ★**소켓으로 나가는 것까지는 이 스위트가 안 잰다**★ — 돌려받은 명령을 실 소켓에 미는 줄은 무검증 잔여로 남아 있다(그 파일 헤더 「무엇이 안 덮이나」가 정본). 판정 사유는 2b 와 동일(0xc0000139 는 lib 타깃뿐, 통합 타깃은 정상 기립·실측 2026-08-20 재확인) — 2번이 이 스위트도 못 보므로 이 줄이 유일한 실행 경로다
cargo fmt --check                           # 3) 포맷 게이트 (검사형 — rewrite 안 함)
rg "^\s*use tauri" crates/engram-dashboard-core/src/   # 4) 코어 격리 게이트 → 0줄이어야 PASS (ADR-0003)
npx tsc --noEmit                            # 5) 프론트 타입체크 (package.json에 typecheck 스크립트 없음)
npm test                                    # 6) 프론트 테스트 (vitest run)
```
- **★위 블록의 빌드·테스트 줄(1·2·2b~2e·3·5·6)은 전부 「분리 실행」 절을 거쳐 돈다★**(4번 `rg` 는 대상 밖 — 그 절의 판정 규칙). 근거·실측은 그 절이 갖는다.
- **★2b~2e의 `--test`를 `-p` 단독이나 `--tests`로 넓히지 말 것★** — 둘 다 죽는 lib 타깃(`0xc0000139`)을 도로 끌어와 스텝이 통째로 실패한다. **`cargo build`·2번 어느 쪽도 이 타깃들을 컴파일하지 않는다** — `build`는 테스트 타깃을 안 굽고 2번은 패키지를 뺀다. 그래서 이 줄들이 빠지면 그 스위트가 깨진 것조차 안 보인다 — **2d·2e가 이 목록에 없던 동안 실제로 그랬다**(정정 2026-08-21). 이 목록이 곧 로컬 실행 명단이라 타깃이 늘면 여기에도 줄을 늘린다.
- 코어 격리 게이트(`rg "^\s*use tauri" ...`)는 **출력이 0줄일 때만 PASS** — 한 줄이라도 나오면 FAIL(코어가 Tauri를 import = 격리 위반). 종료코드가 아니라 *매치 유무*로 판정한다. 패턴은 import 라인 앵커(`^\s*`) — 게이트 규칙을 자기 인용한 문서 주석(`//!`)이 오탐되는 것 방지(실측 2026-07-13).
- 멤버별로 좁혀 돌릴 땐 `cargo test -p <멤버>`.
- **메시징 커널 격리 게이트(ADR-0110 — messaging crate가 닿으면 필수):** `rg "engram_dashboard_(core|daemon|protocol|discovery|command)" crates/engram-dashboard-messaging/src/` → 0줄 PASS. 이 crate는 워크스페이스 crate 무의존이 불변식이라 위반은 컴파일 에러로 먼저 잡히지만, 주석·테스트 헬퍼 이름으로 새는 경로는 grep이 잡는다. ★**괄호 안 이름 목록을 줄이지 말 것 — 새 워크스페이스 crate가 생기면 여기에 더한다**★(`command` 누락 상태로 한동안 돌았다 — CI 쪽에만 있어 로컬이 더 약했다).
- **의존 상한 게이트 2종 — standard에서 항상 돌린다**(해당 crate가 닿으면 quick에서도 필수). 위 정규식이 **소스 텍스트**만 봐서 못 잡는 형태(따옴표 종류·`[build-dependencies]`·rename·비활성 target·`optional`)를 **해석된 의존 그래프**로 덮는다:
  ```bash
  cargo tree -p engram-dashboard-messaging --depth 1 --prefix none -e normal,dev,build --target all --all-features | rg "^engram-dashboard" | sort -u   # → 정확히 1줄(자기 자신) PASS — ADR-0110 무의존 불변식
  cargo tree -p engram-dashboard-command   --depth 1 --prefix none -e normal,dev,build --target all --all-features | rg "^engram-dashboard" | sort -u   # → 정확히 1줄(자기 자신) PASS — ADR-0155 도구 crate 무의존
  ```
  줄 수로 판정한다(매치 유무가 아니다). **플래그를 줄이지 말 것** — net 게이트 3과 같은 이유로 그만큼 형태가 샌다. ★**정규식 게이트의 가장 큰 구멍이 이것을 부른 계기다**★ — 정규식은 crate 이름 알파벳을 손으로 박아 두므로 **새 crate는 누가 그 알파벳에 이름을 더할 때까지 아예 안 보인다**. ★**두 게이트에 공통으로 남는 구멍**★ — 둘 다 워크스페이스 멤버를 `engram-dashboard` **이름 접두**로 식별하므로, 다른 이름을 단 멤버는 양쪽 다 그냥 통과한다. command crate가 워크스페이스 의존 0을 지키는 것은 **벽**이지 그 crate가 존재하는 *이유*는 아니다(이유 = 독립적으로 쓸 수 있고 순환을 막는다 — CLAUDE.md 「백엔드 모듈 맵」 command 항목 · ADR-0151 결정 4).
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
  rg -U -n "(crate|(super::)+)::[^;]*\b($(rg -o '^(pub )?mod ([a-z_0-9]+);' -r '$2' crates/engram-dashboard-daemon/src/lib.rs | rg -v '^control$' | paste -sd'|'))\b" crates/engram-dashboard-daemon/src/control/   # 재개 조건② — 0줄이면 보류 유지, 매치가 나오면 ADR-0130 재론
  ```
  **★매치가 나와도 바로 재론이 아니다 — 한 단계 걸러라★:** 그 줄이 **그 파일의 `#[cfg(test)]` 시작 줄보다 뒤면 테스트 픽스처라 합법**이다(ADR-0130 §영향이 옆걸음을 명시적으로 허용 — grep 은 이걸 못 가른다). 앞이면 production 간선 후보 = 재론. `rg -n "#\[cfg\(test\)\]" crates/engram-dashboard-daemon/src/control/` 로 경계 줄을 뽑아 대조한다.
  **패턴 주의(2026-08-05 리뷰 2라운드):** ① 경로 접두를 `crate::` 만으로 좁히지 말 것 — `control/mod.rs` 에서 `super::` 는 크레이트 루트라 같은 간선이 빠져나간다. ② **`-U` 를 빼지 말 것** — rustfmt 가 쪼갠 그룹 import(`use crate::{`⏎`  connection_core::A,`)는 접두와 모듈명이 다른 줄이라 단일행 패턴이 못 문다. ④ **형제 모듈 이름을 손으로 박지 말 것** — 박아 둔 명단은 새 모듈이 생겨도 안 늘어 그 간선이 **아예 안 보인다**(실측 2026-08-19: 박힌 네 이름 판이 `control/commands.rs → command_delivery` 와 `control/mcp_server.rs → command_roster` 둘을 놓친 채 0줄을 냈다 — 트리거가 관측하려던 성질이 그동안 관측 불가였다). 그래서 명단은 `lib.rs` 의 모듈 선언에서 **파생**한다(`control` 자신만 뺀다). 그 파생을 다시 상수 목록으로 되돌리지 말 것. ③ **양성 대조(경로를 `src/` 로 넓히기)로 접두 축소를 승인하지 말 것** — 현재 실간선은 전부 `crate::` 형태라 `super::` 갈래가 죽어도 그 대조는 통과한다.
  **커버리지는 조건 ②뿐이다.** ①(제어 평면·중계 층을 따로 쓸 소비자가 실제로 생김)은 사람 판단이라 기계화 대상이 아니고, ③(production 의존 그래프의 순환)은 단발 명령이 없다 — 재는 법의 정본은 ADR-0130 §영향 조건 3. **이 블록이 0줄이라고 "순환 없음"으로 읽지 말 것.** ★**등록 상태 자체의 정본은 여기가 아니라 ADR-0130 §영향의 "등록 상태" 줄이다**★ — ③이 나중에 등록되면 그 줄만 갱신되고 이 문단은 낡는다. 상태를 판단할 땐 거기를 볼 것.
- **공유 데몬 바이너리 락(실발동 2026-07-08):** 실행 중인 `engram-dashboard-daemon.exe`(공유 인프라 — 타 에이전트 호스팅 가능)가 있으면 daemon bin을 빌드하는 루트 `cargo build`·`cargo test`가 os error 5로 FAIL한다 — 코드 결함 아님. **데몬 강제 종료 금지.** 우회 = daemon bin을 안 빌드하는 패키지 스코프(`cargo build/test -p <영향 crate들>`)로 좁혀 회귀 확인, 워크스페이스 전체 게이트는 **PARTIAL로 정직 보고**(못 돌린 범위 명시).

### full — standard + GUI 실측 (cdp)

standard 게이트를 전부 PASS시킨 뒤, 실제 앱을 띄워 화면 동작을 확인한다(**Windows 전용** — WebView2 CDP, 포트 9223 고정).

**★앱을 셸에서 직접 띄우지 않는다★** — 셸에서 띄우면 그 호출이 앱 수명에 매달리고 **앱 출력이 파이프를 거슬러 계속 올라온다.** 분리 실행은 앱을 프로세스 트리 밖에 두고 **출력을 로그 파일로만** 보내므로, 판정에 필요한 줄만 골라 읽는다. `start`·백그라운드 잡·`nohup`은 그 로그 파일·PID 계약을 주지 않으므로 대체재가 아니다. **위 「분리 실행」과 같은 규칙의 앱 쪽 절차다** — 원칙은 하나이고 도구만 갈린다(앱 = `launch-detached.ps1`(exe 경로) · 빌드·테스트 = `run-detached.ps1`(명령줄)).

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

#### 실측 조리법 — 위 3)에서 무엇을 어떤 채널로 관측하나

> 위 「스샷보다 `eval` 텍스트」 불릿의 **실행 세부**다(그 판정을 되풀이하지 않는다). 이 네 갈래를 워커마다 다시 알아냈다 — 2026-08-21 한 세션에서 둘이 **독립적으로** 같은 것을 발굴했고 한쪽은 그 발굴을 "이번에 가장 비쌌던 작업"(도구 호출 84회)으로 지목했다.
> ★**검증 표시가 항목마다 붙어 있다 — 이 절을 쓴 세션은 앱을 띄우지 않았다.**★ `코드 파생`(파일:줄이 근거) · `이전 세션 실측`(날짜) · `미검증`(아무도 돌려 본 기록이 없다) 셋으로 갈린다. 미검증 항목이 틀리면 그 자리를 고치고 표시를 올릴 것.

**A. cdp.mjs가 하는 일은 셋뿐이고, 영역 지정 캡처는 없다** (코드 파생)
- 서브커맨드 = `info`(타깃 목록) · `eval "<js>"`(결과 JSON 출력, `awaitPromise`라 `await` 가능) · `shot <png>` — 그 밖은 usage + exit 1 (`scripts/cdp.mjs:62`·`64`·`106`·`112`). 환경변수 둘: `CDP_PORT`(기본 9223) · `CDP_MATCH`(url·title 정규식). 기본 타깃은 메인 창(popup·tree 제외, `:20`)이고 `CDP_MATCH`로 팝아웃·트리 창을 집는다 — 매치 0건·정규식 무효 = 타깃 목록 + exit 1, 다건 = 첫 번째 선택 (`:66`~`:91`).
- ★**영역·요소 범위 캡처는 지원하지 않는다**★ — `shot`은 `Page.captureScreenshot`에 `{ format: 'png' }`만 넘겨 **창 전체**를 찍는다(`scripts/cdp.mjs:102`). CDP 프로토콜엔 `clip`이 있지만 **이 스크립트가 노출하지 않으므로 없는 플래그를 짐작해 쓰지 말 것.** 좁히는 실수단은 둘이다: ① 좌표·치수·색·가시성은 `eval` + `getBoundingClientRect()`/`getComputedStyle()`로 **숫자로** 받는다(대개 픽셀을 읽을 이유가 사라진다 — 아래 D) ② 대상이 팝아웃 창에 있으면 `CDP_MATCH`로 그 창을 찍는다(창이 작아 캡처도 작다). 파일명에 디렉토리가 없으면 `_wip/shots/`로 라우팅된다(`:98`).

**B. 프로필 생성·삭제는 command 레지스트리에 없다 → window에 노출된 실 client를 쓴다** (코드 파생)
- 레지스트리에 **있는** 것: `agent.spawn`·`agent.rename`·`agent.kill`·`agentlist.createAgent|createTerminal|createJson`·`preset.list|create|delete|rename|add`·`slot.*`·`tab.*`·`window.create|close`·`layout.setSlotContent`·`agent.spawnInto`. ★`theme.set|toggle`은 2026-08-23 에 회수됐다 — 테마는 이제 설정 파일 데이터다(ADR-0167)★. 정상 제어 표면(`window.__engramCmd.run(id, args)` — `src/store/eventBus.ts:140`)으로 되는 일은 여기서 끝낸다.
- **없는** 것: 프로필 삭제(`deleteProfile`)·자동복원 토글(`setProfileAutoRestore`)에 대응하는 command가 없다. **둘의 처지는 다르니 한 문장으로 묶어 읽지 말 것** — 삭제는 사람 경로(트리 행 메뉴)가 있고(`src/components/agent/AgentList.tsx:213`), **자동복원 토글은 `src/` 안에 호출자가 0건**이라 사람 경로조차 없다(선언만 둘 — `src/api/agentClient.ts:209`·`src/api/protocolClient.ts:907`). 그래서 토글은 아래 탈출구로 client 를 직접 부르는 것 말고 실측 수단이 없다. 생성 쪽 command는 있으나 cdp로는 못 쓴다: 셋 다 `createReservedProfile`을 지나 **폴더 선택 다이얼로그를 먼저 띄우고**(`src/commands/agentCommands.ts:19`) `autoRestore`를 `false`로 박아 넘긴다(`:22`) — ★`agentlist.create*`에는 `autoRestore` 인자가 없다★.
- **탈출구 = `window.__ENGRAM_AGENT__`**(단일 `ProtocolClient`. 노출 지점 `src/api/clientFactory.ts:29` — ★DEV 가드가 없어 릴리스 빌드에도 있다★). 앱 소스 모듈을 `import()`할 필요가 없다.
  ```bash
  node scripts/cdp.mjs eval "window.__ENGRAM_AGENT__.createClaudeProfile('<name>','<cwd>',[],[],true,'Terminal').then(p=>p.id)"
  node scripts/cdp.mjs eval "window.__ENGRAM_AGENT__.deleteProfile('<id>')"
  ```
  인자 순서 = `(name, cwd, extraArgs, env, autoRestore, outputFormat?)` (`src/api/agentClient.ts:199`~`206`, wire 매핑 `src/api/protocolClient.ts:877`). ★**기본값 함정**★ — 생략 시 client는 `'Terminal'`(`protocolClient.ts:883`), command 경로는 `'StreamJson'`(`agentCommands.ts:35`)이다. 같다고 보고 인자를 빼면 **반대 렌더 모드**가 나온다.
- 나머지 관측 핸들: `window.__engramLayout`(`eventBus.ts:86`) · `window.__engramChat`(`:124`) · store 스냅샷 `window.__engram.agent.getState()`(`src/main.tsx:24` — ★`import.meta.env.DEV` 가드가 있어 **릴리스 빌드엔 없다**★, `:23`. 위 `__ENGRAM_AGENT__`와 갈리는 지점이니 릴리스로 실측할 때 헷갈리지 말 것).

**C. 터미널 화면 텍스트는 DOM에 없다 → fiber를 타고 `Terminal` 인스턴스의 버퍼를 읽는다**
- 왜(코드 파생): 보이는 슬롯엔 WebGL 렌더러가 붙어(ADR-0056 — `src/components/slot/TerminalSlot.tsx:170`~`222`) 글리프가 canvas로 그려지고, `screenReaderMode`도 안 켜져 있다(생성 옵션 = `fontFamily`·`fontSize`·`theme` 셋뿐, `TerminalSlot.tsx:78`~`82`). 그래서 `.xterm` 하위 `innerText`가 빈 문자열이다(이전 세션 실측 2026-08-21).
- 잡는 경로(코드 파생): 인스턴스를 들고 있는 것은 `terminalRef` 하나이고 window에 노출되지 않는다(`TerminalSlot.tsx:20`·`:90`). ★**시작점은 `.xterm`이 아니라 그 부모다**★ — fiber 키(`__reactFiber$*`)가 붙는 것은 React가 렌더한 컨테이너 div(`TerminalSlot.tsx:341`)이고 `.xterm`은 `term.open()`이 그 안에 만든 것이라(`:85`) 키가 없다. 슬롯이 여럿이면 `[data-slot-id]`로 먼저 좁힌다(`src/components/layout/ViewLayoutRenderer.tsx:159`).
- ★**훅 인덱스로 세지 말 것**★ — `terminalRef`는 현재 두 번째 `useRef`지만(`TerminalSlot.tsx:19`~`32`) 훅 순서는 편집 한 번에 밀린다. **모양으로 찾는다**(`.current.buffer`를 가진 훅). ★미검증 스케치 — fiber 내부 구조에 기대므로 이 저장소엔 선례가 없다(`rg reactFiber|memoizedState` → 0건)★:
  ```js
  const host = document.querySelector('.xterm').parentElement
  const k = Object.keys(host).find(s => s.startsWith('__reactFiber$'))
  let term = null
  for (let f = host[k]; f && !term; f = f.return)
    for (let h = f.memoizedState; h && !term; h = h.next)
      if (h.memoizedState?.current?.buffer) term = h.memoizedState.current
  const b = term.buffer.active
  Array.from({length: b.length}, (_, i) => b.getLine(i).translateToString(true)).join('\n')
  ```
  `buffer.active`·`translateToString`은 xterm 공개 API다(미검증 — 이 저장소에 호출 선례 0).

**D. 스크롤바 관측은 두 갈래로 갈린다 — 채널을 먼저 고른다**
- ★**진짜 네이티브 thumb는 어떤 조회로도 안 보인다**★ — DOM 요소가 아니라 selector·`getComputedStyle`이 그 픽셀을 말해 주지 않는다. **이 경우에만** 캡처가 유일 채널이고, 위 A대로 그건 창 전체다. (**미검증** — 이 저장소에 관측 선례 0. 네이티브 스크롤바가 DOM 밖에 그려진다는 일반 사실에 기댄 추론이다.)
- ★**터미널에서 화면에 보이는 슬라이더는 네이티브가 아니다 — DOM이라 조회된다**★(이전 세션 실측 2026-08-21). xterm 6.0(`package.json:27`)은 VS Code의 ScrollableElement를 쓰고 실물은 `.xterm .xterm-scrollable-element > .scrollbar.vertical > .slider`다(`src/index.css:33`~`35`·`91`~`95`). 폭·위치·가시성은 `eval` + `getBoundingClientRect()`/`getComputedStyle()`로 숫자로 받는다 — **이걸 스크린샷으로 재지 말 것.** 폭·`left`는 xterm JS가 인라인 style로 박아 우리 규칙이 `!important`로 덮는다(`index.css:83`~`92`).
  - 이 갈래 차이가 곧 `index.css`의 `::-webkit-scrollbar` 블록(`index.css:59`~`79`)이 지금 **죽은 코드**인 근거다 — thumb 색을 빨강으로 강제하고 전체 캡처에서 일치 픽셀을 세어 0을 얻었다(이전 세션 실측 2026-08-21 — 그 관측을 적어 둔 주석이 `index.css:37`~`41`이고, 블록 자체는 그보다 아래다). ★"스크린샷만이 관측 수단"으로 뭉뚱그리면 이 갈래를 놓치고 전창 캡처를 반복하게 된다.★
- **넘침 강제(이전 세션 실측 2026-08-21):** `.xterm-viewport`에 **높이 있는 빈 자식을 넣는다.** `.xterm-screen`의 높이 조작은 안 듣는다 — v6의 `.xterm-viewport`는 스크롤을 더 이상 받지 않고(스크롤백이 쌓여도 `scrollHeight === clientHeight`) 불투명한 `.xterm-scrollable-element`가 그 위를 덮기 때문이다(`src/index.css:33`~`37`).
  - ★그렇게 드러나는 것은 **덮인 네이티브 스크롤바**이고 사용자가 보는 그것이 아니다★ — 위 첫 갈래로 간다. (코드 파생 — `src/index.css:33`~`41`)
  - ★**넘침만 만들면 한 픽셀도 안 칠해진다**★ — thumb 색이 평소 transparent이고 `[data-scroll-active]`가 붙은 동안만 칠해진다(`index.css:66`~`79`). 정적 캡처를 뜨려면 표식을 손으로 붙인다(`el.setAttribute('data-scroll-active','1')`). (코드 파생) 실제 스크롤로 붙이면 마지막 스크롤 **500 ms** 뒤 자동으로 떨어진다(`SCROLL_HIDE_DELAY_MS` = `src/components/ui/scroll-area.tsx:36`, 붙이는 쪽 = `src/components/ui/nativeScrollActivity.ts`).
  - 거꾸로 **보이는 `.slider`는 DOM 주입으로 못 띄운다** — 기하가 xterm 내부 버퍼 상태에서 나오므로 실제 스크롤백을 쌓아야 한다(에이전트 출력 또는 위 C로 잡은 인스턴스에 쓰기). (미검증 — 추론)

## 실패 보고 시 게이트 명칭 (골격 §3에 주입)

어디서 막혔는지 짚을 때 쓰는 게이트 이름: build / test(어느 테스트) / fmt / 격리(어느 crate의 어느 게이트 + 매치 줄 또는 실제 줄 수) / tsc(타입체크) / npm(프론트 테스트) / cdp 실측(어느 동작).

## flaky·타이밍·perf 실패 = 매직넘버로 통과 금지 (ADR-0038)

flaky/타이밍/perf 실패를 상수·임계값·재시도 튜닝으로 통과시키려 하면 중단하고 `docs/reference/debugging-conventions.md`(OSS 조사 전환)를 적용한다. (이 규약의 *발화 지점* — qa가 신호를 잡는 곳.)

## 코어 격리 불변식 (정본 = ADR-0003 + 코드의 `// ADR-` 앵커)

코어 crate(`engram-dashboard-core`)는 **Tauri import 0** — `rg "^\s*use tauri" crates/engram-dashboard-core/src/` → 0줄. 이게 깨지면 코어가 전송 방식에 묶인 것 = 회귀. (근거·거부 대안은 ADR-0003.)
