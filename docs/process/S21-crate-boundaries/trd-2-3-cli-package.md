# TRD — 경계 리팩터링 2-3: engram CLI 를 데몬 패키지에서 떼어 독립 패키지로 둔다 (S21)

> 상태: **초안 — 사용자 검토 대기 (D4 사용자 선택 필요).** 3판(2026-10-08) — 리뷰 2라운드 반영 · 재리뷰 전. 코드는 아직 한 줄도 바뀌지 않았다.
> **리뷰 2라운드 반영(2026-10-08):** codex blind(Designer) **FIX** · Claude doc-aware(Architect-breaker) **FIX** — 둘 다 경미하고 서로 맞는다. 바뀐 것: ① 데이터 루트 조인 시험의 「체크아웃 밖이면 건너뜀」 — `CI` 가 있으면 실패 · 「안에 있나」 판정 전에 `current_exe` 와 기대 루트를 같은 `dunce::canonicalize` 로 고른다 · 로컬 확인은 `-- --show-output`(§2-4 · §3-2) ② 「표지 없음」 갈래는 임시 폴더가 아니라 판정을 인자로 받는 내부 함수 + 가짜 판정으로 잰다(옮기는 `none_when_no_marker` 도) ③ §5-3 의 `-OutDir` 는 모든 체크아웃 밖 · `launch\빌드.bat` 의 `launch\release` 는 체크아웃 안이라 폴백을 못 잰다 · `build-release.ps1:16-17` 주석은 U2 가 고친다 ④ base 폴백 함수 이름 = `checkout_root_or_exe_dir`(「설치 위치」는 데몬 · CLI 래퍼에만) · 늘 걷는다(`cfg` 없음)를 doc 에 ⑤ §7 2 에 ADR-0271 결정 2 ⑥ §3-4 수용 기준 2 의 「지금 결과」 서술 정정.
> **리뷰 1라운드 반영(2026-10-08):** `/review trd` — codex blind(Designer) **FIX** · Claude doc-aware(Architect-breaker) **FIX**. 바뀐 것: D4 를 셋(A · A′ · B)으로 다시 세우고 권고를 A → **A′** 로 옮겼다(C2) · 걸음 API 이름을 `find_checkout_root` 로 · 표지 계약을 일반 서술로(C3) · 데이터 루트 회귀 시험을 선택에서 필수로(C1) · 릴리스 빌드 두 호출을 각자의 `Invoke-Step` 으로(F1) · 릴리스 스크립트 1회를 머지 전 필수로(F2) · 빠졌던 빌드 입구 `roundtrip_smoke.rs` 안내문(F3) · 데몬 `--test-threads=4` 근거 정정(F4) · ADR-0284 가 고칠 문장 목록(F5) · 포인터 수용 rg 를 짧은 꼴까지(F6) · 버전 게이트 `:1420` 과 실패 비용 정정(F7) · 「새 crate 는 이름 알파벳에 더한다」의 bin 전용 예외(F8) · 자잘한 줄 번호 · 낱말(F9) · 두 호출의 릴리스 시간 비용(C3). 항목 표지(C1~C3 · F1~F9)는 바뀐 자리에 적었다.
> **결정 출처:** 메인(오케스트레이터) 결정 — D1~D3 · D5~D9. **D4(작업 트리 루트 찾기와 설치 위치 폴백의 자리)는 사용자 선택 대기**다(§2-4 · 권고 = A′). 그 밖의 내부 배치(함수 이름 · 시험 자리 · 단위 안 순서 · 게이트 꼴)는 이 TRD 의 「제안」이다(이름은 전부 가안).
> **범위 = 작업 순서 2-3 전부**(`docs/refactoring/architecture-discussion-2026-09-26.md` §8 결정 후보 9 · §10 2-3) — ADR-0273 결정 1~5 와 「영향」을 코드로 옮긴다. 선행 2-2(discovery 나누기)는 착지해 master 에 머지됐다(ADR-0282).
> **배치 근거:** `docs/README.md` 「새 내용을 어디에 넣나」의 「새 기능 **설계 착수** → `process/SN-name/`」. 형제(1-1 · 1-3 · 2-1 · 2-2 TRD)가 이 폴더에 있다.
> **표기:** 「실측」 = 기준 커밋 `d421fcf`(브랜치 `v0.3.3/refactor/crate-boundaries` 의 HEAD — 코드 트리는 master `79b8d09` 흡수 머지 `d9baa2d` 와 같다. 그 뒤 두 커밋은 문서뿐)에서 잰 것과 그 명령 · 「결정」 = 위 메인 결정 · 「ADR」 = 확정 ADR 본문 · 「제안」 = 이 TRD 의 안 · 「예상」 = 코드를 바꾸기 전이라 잴 수 없는 수치. 줄 번호는 따로 적지 않으면 `crates/engram-dashboard-daemon/src/bin/engram.rs` 다.
> 앵커: **ADR-0273**(이 단계의 헌장) · ADR-0282(「영향」 — CLI → 데몬 lib 임시 간선 · 결정 2 · 3) · ADR-0269(base 입주 규칙 — 결정 7) · ADR-0271(결정 2 · 4 — 설치 위치는 데몬 · 셸 사본은 같은 경로 시험) · ADR-0270(셸 → agent 운영 의존 0) · ADR-0175 결정 6(파일 하나짜리 lib 금지) · ADR-0264(경로 정본을 base 에 두지 않는다) · ADR-0094(실행 파일 이름 정렬) · ADR-0100(배포 동거) · ADR-0132 · step-log S21.

---

## 0. 결론 (먼저)

```
① 새 패키지 `crates/engram-dashboard-cli` — bin 전용 · bin 이름 `engram` 그대로. 소스 자리도 `src/bin/engram.rs` 그대로라
     옛 ADR 포인터의 뒤꼬리 · `include_str!` 깊이 · 시험 도우미의 매니페스트 깊이가 안 바뀐다(D1 · D2).
② 의존 = agent(명령 어휘) · command(요청 번호) · serde_json (+ A · A′ 면 base) · dev = uuid.
     데몬 · tokio · net · protocol · platform 직접 의존 0 — engram.exe 의 빌드 그래프가 약 194 → 약 92 패키지(예상)(D3).
③ ★D4 사용자 선택★ — exe 에서 위로 걸어 체크아웃 루트를 찾는 걸음과 설치 위치 폴백을 어디 두나.
     A′(권고): 걸음 + 설치 위치 규칙(체크아웃 루트, 없으면 exe 폴더)을 base `path` 로 — 데몬 · CLI 가 같은 함수를 부르고
              셸 · 데몬의 데이터 루트는 걸음만 쓴다. 사본도 묶이지 않은 폴백도 남지 않는다 · CLI → 데몬 간선 0.
     A: 걸음만 base — 폴백 한 줄은 데몬 · CLI 두 벌로 남고 시험으로 못 묶는다.
     B: ADR-0273 결정 5 그대로 — CLI 쪽 사본 + 같은 경로 시험(그 시험 때문에 CLI → (dev) 데몬).
④ `engram_cli.rs`(실 exe 를 띄우는 41건)는 bin 을 따라 새 패키지 `tests/` 로 — `CARGO_BIN_EXE_engram` 이 강제한다(D5).
⑤ 빌드 입구 넷(릴리스 스크립트 · rebuild 런처 둘 · `roundtrip_smoke.rs` 안내문)을 옮기는 커밋에서 고친다 — 런처 둘은 안 고치면
     engram.exe 를 조용히 안 짓는다. 릴리스 빌드는 두 호출 · 각자의 `Invoke-Step`. qa 바인딩의 빌드 명령도 같은 단위에서
     고쳐야 하지만 사용자 승인 뒤다(D6 · §8 O3).
⑥ 게이트: cli 의존 상한(이름 집합 일치) · test-support 운영 그래프 고리 둘에 cli · platform 게이트 ⑦ 에 cli 0줄 ·
     버전 게이트에 cli 매니페스트 · CI 단독 기립 스텝 · ci.yml 주석 · 「새 crate 는 이름 알파벳에 더한다」의 bin 전용 예외(D7).
⑦ 단위 셋 U1 → U3(A′). 어느 커밋에서 멈춰도 빌드 · 회귀가 초록. U2 는 쪼갤 수 없는 원자 단계 하나다 — 두 패키지가
     같은 bin 이름을 가지면 cargo 는 막지 않고 산출 파일 충돌을 경고하며 같은 `engram.exe` 를 서로 덮어쓴다.
     결과 줄 · 무시 수는 그대로 · 통과는 +2(A′ 의 새 base 시험 — 예상). 릴리스 스크립트 1회는 머지 전 필수다(F2).
     사용자가 체감하는 동작 변화는 없다(§9 끝 — 개발자가 보는 안내문 하나가 바뀐다).
```

**결정 출처:** 메인 결정(머리말) — D4 만 사용자 선택이다.

**지금 하지 않는 것:** `scripts/engram.mjs`(T-23 · ADR-0132 — D8) · CLI 어휘 상수를 agent 밖으로 빼기(ADR-0273 「영향」 — 의존 최소화는 일부만 얻는다) · 테스트용 서버 함수 정리와 셸 → (dev) 데몬 간선(2-4) · CLI 표면 · help 화면 내용 변경 · 데몬의 쓰이지 않는 `tracing-subscriber` 선언(별건 — `docs/todo/doc-drift.md:16`).

---

## 1. 현황 실측 (`d421fcf`)

### 1-1. bin 과 그 의존

| 무엇 | 자리 | 비고 |
|---|---|---|
| bin 소스 | `engram.rs` 5,508줄 — 시험 모듈 `:2806`(`#[cfg(test)]`) 아래 124건 · `#[ignore]` 0 | 시험 앞 2,805줄 |
| 워크스페이스 import | `:117-121` `engram_dashboard_agent::types::{…}`(CLI 어휘 상수) · `:177` `engram_dashboard_command::RequestId` · `:338` `engram_dashboard_daemon::data_dir::find_install_root`(앵커 `:337` · `help_file_path` `:336-340` 안) | 데몬 lib 를 쓰는 유일한 줄 = `:338`(ADR-0282 「영향」의 임시 간선) |
| 서드파티 | `serde_json`(운영) · `uuid`(시험 `:4482` 하나) · 나머지는 std(`TcpStream` 손조립 HTTP — `:113-115`) | protocol · net · platform · tokio 0. OS `cfg` · `std::os` 0(`cfg(` 는 `:2806` 하나) |
| help 본문 | `:231` `include_str!("../../../../prompts/engram-help.md")` · 시험 도우미 `help_repo_root`(`:4339` — `CARGO_MANIFEST_DIR` 의 두 단계 위) | 둘 다 `crates/<패키지>/src/bin/` · `crates/<패키지>` 깊이에 기댄다 |
| 선언 | 데몬 `Cargo.toml:33-39`(`[[bin]] name = "engram"` · 주석 `:33-36`) · 패키지 버전 `:3` = `0.3.2` | 이 bin 만을 위한 데몬 의존 줄은 없다 — `serde_json` · `uuid` 는 데몬 자신도 쓴다 |
| 실 exe 시험 | 데몬 `tests/engram_cli.rs` 1,784줄 · 41건 · `#[ignore]` 0 — `env!("CARGO_BIN_EXE_engram")`(`:122` 등) · 파일명 가드 `the_built_binary_file_name_matches_the_shared_constant`(`:604-616`) · import = agent(`:13`) · serde_json · std | `CARGO_BIN_EXE_<name>` 은 같은 패키지의 bin 만 보인다 → bin 을 따라가야 한다 |

`cargo metadata` — 데몬 타깃 = lib · bin 여섯(`engram` · `engram-dashboard-daemon` · `test-harness` 전용 넷) · 시험 일곱(`engram_cli` 포함). `CARGO_BIN_EXE_engram` 을 쓰는 시험은 `engram_cli.rs` 뿐이다(`rg CARGO_BIN_EXE crates src-tauri` — 그 밖은 `ws_e2e.rs:2329` 의 데몬 exe 하나).

### 1-2. 설치 위치 · 체크아웃 루트 찾기 — 걸음이 이미 두 벌이다

| 자리 | 내용 |
|---|---|
| 데몬 `data_dir.rs:162-168` `find_install_root` | `current_exe` → 걸음 → 못 찾으면 exe 폴더 · exe 도 못 얻으면 `None`. ★그 밖의 동작은 없다★ — 데몬 상태 · 환경 · 설정을 읽지 않는 exe 경로의 순수 함수다. 부르는 곳 = `control/priming.rs:275`(프라이밍 — `None` 이면 `.`) · CLI `:338` |
| 데몬 `data_dir.rs:170-196` `find_workspace_root` · `is_workspace_root` | 비공개. 부르는 곳 = `default_data_dir` 디버그 분기(`:72-87` · 호출 `:78`) · `find_install_root`(`:164`). 표지 시험 넷(`:415-461` · `:473-495` — `.git` · `[workspace]` · 표지 없음 · 판정 표) + 도우미 `unique_tmp`(`:404-413`) |
| 셸 `src-tauri/src/discovery/layout.rs:99-127` | 같은 두 함수의 사본(`:101-127`)과 그 위 주석 `:99`. 부르는 곳 = `default_data_dir` 디버그 분기(`:51`) 하나 → 둘 다 `#[cfg_attr(not(debug_assertions), allow(dead_code))]`(`:100` · `:116`). 표지 시험 0 — 같은 경로 시험(`:231-279`)이 루트 결과만 데몬과 견준다 |

- **두 벌의 본문은 글자까지 같다**(실측 `diff <(sed -n 170,196p 데몬 data_dir.rs) <(sed -n 101,127p 셸 layout.rs)` — 차이는 셸의 `cfg_attr` 한 줄뿐). 둘 다 canonicalize 하지 않고, 시작점이 폴더면 거기서 · 아니면 부모에서 출발해 `parent()` 가 다할 때까지 오른다(횟수 상한 없음). 표지 = `.git`(폴더든 파일이든 — `exists()`) 또는 본문에 `[workspace]` 부분 문자열을 담은 `Cargo.toml`(읽기 실패 = 아니다).
- `default_data_dir` 본문(데몬 `:67-101` · 셸 `:40-74`)도 글자까지 같다(같은 diff · 종료코드 0).
- 저장소 안에 다른 걸음은 없다(`rg '"\.git"|\[workspace\]"' crates src-tauri scripts` → 위 두 파일뿐).
- ★빈틈 하나(C1)★ — 디버그 데이터 루트를 재는 시험은 끝이 `.engram-dev` 인지만 본다: 데몬 `data_dir_empty_env_falls_through_to_default`(`:284-301`) · `default_data_dir_debug_is_local_data_dir`(`:303-317`) · 셸 같은 경로 시험의 전제(`:242-250`). 걸음이 실패해 exe 폴더 폴백(`target\debug\deps\.engram-dev`)으로 떨어져도 셋 다 초록이다(셸은 두 벌이 같은지만 보므로 함께 틀리면 못 본다).
- 저장소는 target 폴더를 체크아웃 밖으로 돌리는 경우를 안다(`CARGO_TARGET_DIR` — `scripts/build-client-shell.mjs:48` · `build-release.ps1:79` 가 그것을 처리한다). 그때는 걸음이 표지를 못 만나 운영 동작부터 exe 폴더 폴백이다.

### 1-3. 빌드 입구 — 「데몬 패키지를 지으면 CLI 도 지어진다」에 기대는 자리

| 자리 | 지금 | 2-3 뒤 그대로 두면 |
|---|---|---|
| `scripts/build-release.ps1:133-135` | 한 `Invoke-Step` 안 한 호출 `-p engram-dashboard-daemon --bin engram-dashboard-daemon --bin engram` | 빌드 실패(그 패키지에 bin `engram` 이 없다) — 요란하다 |
| `build-release.ps1:61-66` `Invoke-Step` | 블록 진입 전 `$LASTEXITCODE` 를 0 으로 두고 ★블록이 끝난 뒤 한 번만★ 본다 | 한 블록에 cargo 호출 둘을 넣으면 앞 호출의 실패를 뒤 호출의 성공이 덮는다 — 그 뒤 존재 검사(`:139-142`)는 이전 빌드의 낡은 exe 를 받아들인다(F1) |
| `ci.yml:1281-1283`(태그 릴리스만 · `if: github.ref_type == 'tag'`) | 위 스크립트를 부른다(`:1471-1477`) | 같다 — 이 스크립트와 버전 게이트는 태그 push 에서만 돈다(F2) |
| `scripts/rebuild-run-debug.bat:93` · `scripts/rebuild-run-release.bat:111` | `cargo build [--release] -p engram-dashboard-daemon` | ★조용하다★ — engram.exe 를 안 짓는다. 데몬은 남아 있는 옛 exe 를 형제로 집거나, 없으면 warn 한 줄 뒤 CLI 입구 없이 뜬다(데몬 `lib.rs:103-117` · fail-open) |
| 데몬 `src/bin/roundtrip_smoke.rs:66`(doc) · `:424`(SETUP-SKIP 안내 문자열 — 실행 중 찍힌다) | `cargo build -p engram-dashboard-daemon --features test-harness --bin engram` | 그 명령을 따라 치면 「no bin target named `engram`」로 진다(F3) |
| qa 바인딩 `.claude/skill-bindings/qa.md:320` · `:355` · `:360` | GUI 실측 전 `cargo build -p engram-dashboard-daemon` | 조용하다(런처와 같다) |
| `src/util/launcherWiring.test.ts:31` | 정규식 `-p\s+engram-dashboard(?![-\w])` 가 런처 · qa 바인딩의 생 셸 빌드를 막는다 | `-p engram-dashboard-cli` 는 뒤에 `-` 가 와 걸리지 않는다(코드 읽기) |

`build-release.ps1:49` 의 기대 exe 목록(`engram.exe`)과 `scripts/rebuild-run-debug-log.bat`(debug 런처를 부를 뿐이다 — `:23`) · `launch/빌드.bat`(그 스크립트를 부를 뿐이다 — `:4`)은 빌드 명령을 고칠 것이 없다. 단 `launch/빌드.bat:4` 의 조립 폴더 `"%~dp0release"` = `<저장소>\launch\release` 는 체크아웃 안이다 — 거기 놓인 릴리스 exe 는 걸음이 저장소 `.git` 을 만나 설치 위치 = 저장소 루트가 된다(지금 동작 · `build-release.ps1:16-17` 주석과 어긋난다 — §5-3). `rg -n "engram-dashboard-daemon[^\n]*--bin engram($|[^-\w])"` 의 지금 결과가 정확히 위 세 줄(`build-release.ps1:135` · `roundtrip_smoke.rs:66` · `:424`)이다.

### 1-4. 데몬이 CLI 를 찾는 법 — 바뀌지 않는다

- 데몬 `locate_send_exe`(`lib.rs:103-117`)와 하네스 `sibling_send_exe`(`src/bin/roundtrip_smoke.rs:902-903`)는 `platform::env::sibling_exe(CLI_EXE_NAME)` — 현재 exe 폴더 + 이름이다(ADR-0282 결정 1). 워크스페이스는 target 폴더 하나를 나눠 쓰므로 새 패키지의 `engram.exe` 도 `target\<profile>\` 에 떨어져 형제 관계가 그대로다(cargo 기본 동작 — §9).
- ★단 「데몬을 지으면 CLI 도 지어진다」가 사라진다★ — 데몬 실 레인 `cargo test -p engram-dashboard-daemon --test ws_e2e -- --ignored` 는 engram.exe 를 짓지 않게 된다. 2-2 U1 은 그 레인이 띄운 데몬의 로그에 「형제 exe 를 못 찾음」 warn 이 없는지를 봤다(2-2 TRD §3-2) — 그 확인을 다시 쓰려면 CLI 를 먼저 짓는다.

### 1-5. 게이트 · 바인딩 · 문서가 CLI 자리를 부르는 곳

- **`ci.yml`** — `:33`(경로 필터 주석의 `crates/engram-dashboard-daemon/src/bin/engram.rs:231` 포인터) · `:520-523`(메시징 이름 정규식 주석 「새 워크스페이스 crate 가 생길 때마다 여기 이름을 더하고」) · `:675` · `:767`(test-support 운영 그래프 고리 `for pkg in engram-dashboard-daemon engram-dashboard`) · `:1208-1229`(platform 게이트 ⑦ — 대상 agent · net) · `:1355-1362`(버전 게이트 주석 「5건」 · 셋째 = 데몬 `Cargo.toml` 「daemon.exe·engram.exe 를 만드는 crate」) · `:1411`(그 검사 줄) · `:1420`(`PASS — 제품 버전 선언 5건`). platform 게이트 ④ · ⑥ 은 `crates` · `src-tauri` 전체를 훑고 명단에 CLI 파일이 없다 — 두 파일에 OS `cfg` · `std::os` 가 없으므로(1-1) 명단은 그대로다(`roundtrip_smoke.rs` 는 명단에 있지만 안내문 편집은 `cfg` 를 건드리지 않는다). 게이트 ⑤ 는 `--workspace` 라 새 멤버를 저절로 본다. CI 의 crate 별 시험 스텝(base · platform · agent · command · protocol · messaging · net · transport)에 데몬 스텝은 없다(워크스페이스 회귀가 덮는다).
- **버전** — 라이브러리 crate 는 `0.1.0` 이고 버전 게이트 주석이 그것을 「단독 배송물이 없는 내부 semver」라 뺀다(`:1373-1375`). 데몬 · 셸은 제품 버전 `0.3.2`. ★CLI 는 `CARGO_PKG_VERSION` 을 읽지 않는다★(`rg CARGO_PKG_VERSION crates src-tauri/src` → agent 셋 · 데몬 `agent_conn.rs:290` 뿐) — 지금 그 값은 배송 바이트에 닿지 않는다.
- **serde_json 기능(C3)** — 데몬 그래프는 serde_json 에 `default` · `std` · `alloc` · `raw_value` 를 켜고, agent · command 그래프는 `default` · `std` 만 켠다(실측 `cargo tree --locked -p <p> -e normal,features --target all -i serde_json`). 한 cargo 호출 안에서는 고른 패키지들의 기능이 합쳐지고 따로 부르면 갈린다.
- **데몬 시험의 프로세스 띄우기(F4)** — `--test-threads=4` 를 데몬에 붙이는 근거는 CLI 스위트만이 아니다. 기본 실행(`#[ignore]` 아님) 시험이 PTY 로 실 셸을 띄운다 — `tests/control_agent.rs:406` · `:824` · `:1139` · `tests/mcp_manager_lifecycle.rs:236` · `:324`(`platform::shell::default_shell()`) · `src/connection_core.rs:2890`(`cmd.exe /c exit`) · `src/messaging_host.rs:1137`. 그 위에 `ws_e2e.rs` 의 `#[ignore]` 실 데몬 exe 분이 있다.
- **CLAUDE.md** — `:158`(base 항목 — `path`(사람이 친 경로의 철자 고르기)) · `:207`(`inventory` 가 「데몬·셸 릴리즈 바이너리」에 링크된다 — engram.exe 가 빠져 있다) · `:222`(CI 에만 있는 async 반입 게이트) · `:231`(「병렬은 테스트 바이너리마다 걸린다」의 「`-p engram-dashboard-daemon`도 해당 — 프로세스 레벨 CLI 스위트와 실 `.exe` spawn `#[ignore]` 분을 갖는다」) · `:255`(메시징 정규식) · `:256-261`(의존 상한 · 시험 기능 게이트 줄) · 「백엔드 모듈 맵」에 CLI 항목 없음.
- **qa 바인딩** — `:79-84`(게이트 ⑦ 로컬 블록) · `:138`(`--test-threads=4` 를 붙이는 crate `agent`·`daemon`·`platform`) · `:253`(메시징 정규식 「새 워크스페이스 crate가 생기면 여기에 더하고」) · `:254-261`(의존 상한 「4종」) · `:262-275`(test-support 두 블록) · `:320` · `:355` · `:360`(빌드) · `:442` · `:445`(F 절 — CLI 자리 서술).
- **문서 · 주석** — `docs/testing-strategy.md:43`(base `path`) · `:93` · `:95` · `:159`(데몬 절의 `engram_cli`) · `docs/reference/architecture-overview.md:169`(실행 산출) · `:173-197`(crate 계층 그래프) · `:384`(`crates/engram-dashboard-daemon/src/bin/engram.rs` 헤더) · 짧은 꼴 포인터(F6) — agent `src/types.rs:474`(daemon `tests/engram_cli.rs`) · `:558`(daemon `bin/engram.rs`) · 데몬 `control/catalog.rs:421` · `control/mcp_server.rs:208` · `:1636` · `:1697` · `control/priming.rs:705`(`bin/engram.rs`) · `build-release.ps1:11`(daemon tests/engram_cli.rs) · 그 밖 `control/priming.rs:234` · `build-release.ps1:127-132` · `rebuild-run-release.bat:5-8` · `scripts/README.md:12`.
- **ADR** — `src/bin/engram.rs` 를 부르는 ADR 10 개(`git grep -l "src/bin/engram.rs" -- docs/decisions` — ADR-0273 「영향」의 읽기 규칙이 덮는다) · `engram_cli` 를 부르는 ADR 2 개(ADR-0220 · ADR-0232 — 그 규칙 밖이다).

### 1-6. 기록과 어긋난 사실

| 기록 | 실제(실측 `d421fcf`) |
|---|---|
| ADR-0273 「맥락」 — 데몬 lib 를 한 줄도 쓰지 않는다 · 쓰는 것은 agent · command · discovery | discovery 는 2-2 에 사라졌고 지금은 데몬 lib 한 줄(`:338`)이다 — ADR-0282 「영향」이 이미 적었다 |
| 메모 §8 「사실」 — 비테스트 2,791줄(`a226f63`) | 지금 5,508줄 중 시험 모듈 앞 2,805줄 |
| ADR-0273 결정 5 · ADR-0271 「영향」 — CLI 쪽에 설치 위치 규칙을 「한 벌 더」 | 규칙의 본체(걸음)는 이미 데몬 · 셸 두 벌이다(1-2) — CLI 사본이면 셋째다 |
| 메모 §8 「사실」 — 셸이 데몬을 dev 의존으로 끌어온다 `src-tauri/Cargo.toml:104` | 지금 `:118`(줄이 밀렸다 — 내용은 맞다) |
| `scripts/build-release.ps1:11` — 파일명 가드 = 「daemon tests/engram_cli.rs」 | 2-3 뒤 cli 패키지(U2 가 고친다) |
| CLAUDE.md `:231` — 데몬의 `--test-threads=4` 근거 = 「프로세스 레벨 CLI 스위트와 실 `.exe` spawn `#[ignore]` 분」 | 기본 실행의 실 PTY 셸 스폰도 있다(1-5 F4) — CLI 스위트가 나가도 데몬은 그 플래그를 그대로 진다 |

---

## 2. 결정 (메인 결정 — D4 만 사용자 선택) · 설계

| | 결정 | 한 줄 이유 |
|---|---|---|
| **D1** | 패키지 이름 `engram-dashboard-cli` · 폴더 `crates/engram-dashboard-cli` | 접두는 ADR-0273 결정 2(이름 접두로 멤버를 세는 게이트). `-cli` 는 역할 낱말이다. `-engram` 은 제품 이름과 읽힘이 겹친다(기각) |
| **D2** | bin 전용 패키지 · 파일은 `src/bin/engram.rs` 그대로 · `[[bin]]` 명시(앵커 `# ADR-0273`) | 옛 ADR 포인터의 뒤꼬리 · `include_str!` 깊이 · `help_repo_root` 깊이가 그대로다. `src/main.rs`(경로 · `include_str!` 를 고친다) · lib + bin(lib 소비자 0 · ADR-0175 결정 6) 기각 |
| **D3** | 운영 의존 = agent · command · serde_json(+ A · A′ 면 base) · dev = uuid(+ B 면 daemon) | 소스가 실제로 쓰는 것만(1-1) — protocol · net · platform 직접 0 |
| **D4** | ★**사용자 선택**★ — **A′(권고)** 걸음 + 설치 위치 규칙을 base `path` 로 · **A** 걸음만 base · **B** CLI 사본 + 같은 경로 시험 | §2-4 |
| **D5** | `engram_cli.rs` → 새 패키지 `tests/` · 로컬 명령에 `-- --test-threads=4` | `CARGO_BIN_EXE_engram` 이 같은 패키지 bin 만 본다 · 41건이 실 exe 를 띄운다(ADR-0273 「영향」의 판정) |
| **D6** | 빌드 입구 넷은 옮기는 커밋에서 고친다 · 릴리스 빌드는 두 호출을 각자의 `Invoke-Step` 으로 · 릴리스 스크립트 1회는 머지 전 필수 · qa 바인딩 편집은 사용자 승인 뒤 | 런처 둘은 안 고치면 engram.exe 가 조용히 낡는다(1-3) · 한 블록 두 호출은 앞 실패를 덮는다(F1) · 그 스크립트는 CI 에서 태그 때만 돈다(F2) · 바인딩은 사용자 소관 |
| **D7** | CI · CLAUDE.md · 바인딩 세 곳 등록 — 의존 상한(이름 집합) · test-support 고리 둘 · 게이트 ⑦ cli 줄 · 버전 게이트 편입 · 단독 기립 스텝 · 주석 · 이름 알파벳의 bin 전용 예외 | ADR-0273 「영향」 — 새 패키지는 게이트를 세 곳에 등록한다 |
| **D8** | `scripts/engram.mjs` 는 범위 밖 | T-23 · ADR-0132 의 스로어웨이 스파이크 — 자기 `daemon.json` 후보를 따로 갖고 CLI 와 코드를 나누지 않는다 |
| **D9** | 새 ADR-0284(커밋 직전 번호 재확인) · 옛 포인터 읽기 규칙을 짧게 · 고쳐지는 옛 문장 목록(§7 2) | ADR-0273 의 읽기 규칙은 `engram.rs` 하나만 덮는다 — `engram_cli.rs` 를 부르는 ADR 둘이 남는다(1-5) |

### 2-1. D1 · D2 — 패키지 모양

- **매니페스트(제안):**

  ```toml
  [package]
  name = "engram-dashboard-cli"
  version = "0.3.2"        # 옮기는 순간의 데몬 `version` 값 그대로 — 버전 게이트가 잰다(D7 5)
  edition = "2021"
  description = "Engram Dashboard 제어 평면 CLI(engram.exe) — 스폰된 에이전트가 데몬 제어 라우트에 손조립 HTTP 로 붙는 최소 클라이언트. bin 전용(ADR-0273)."

  # ADR-0273
  # (데몬 Cargo.toml:33-36 의 주석을 옮긴다 — bin 이름 = agent `CLI_EXE_NAME` · 바꾸면 우편이 조용히 멈춘다 · ADR-0094)
  [[bin]]
  name = "engram"
  path = "src/bin/engram.rs"

  [dependencies]
  engram-dashboard-agent = { path = "../engram-dashboard-agent" }
  engram-dashboard-command = { path = "../engram-dashboard-command" }
  engram-dashboard-base = { path = "../engram-dashboard-base" }   # D4 = A′ · A 일 때 — 설치 위치 규칙 · 체크아웃 루트 찾기
  serde_json = "1"

  [dev-dependencies]
  uuid = { version = "1", features = ["v4"] }
  ```

- **깊이가 그대로인 둘:** `include_str!("../../../../prompts/engram-help.md")` — `crates/engram-dashboard-cli/src/bin/` 에서 넷 올라가면 저장소 루트다(지금과 같은 깊이 · 코드 읽기 — U2 컴파일이 잰다). `help_repo_root`(`:4339`)도 `CARGO_MANIFEST_DIR` 두 단계 위 = 루트 그대로.
- **자동 발견과 겹치지 않는다** — 명시 `[[bin]]` 이 `src/bin/engram.rs` 를 점유하면 같은 파일로 타깃이 되살아나지 않는다(`engram_cli.rs:598-603` 의 실측 주석 · 지금 데몬이 같은 꼴이다).
- **bin 전용이라 Doc-tests 줄이 없다**(cargo 는 lib 타깃에만 doctest 를 돈다 — 가능성 높음 · §9).
- **데몬 쪽:** `Cargo.toml:33-39`(주석 + `[[bin]]`) 를 지운다. 데몬의 다른 의존은 그대로다 — `serde_json` · `uuid` 는 데몬 자신이 쓴다. 루트 `Cargo.toml` 멤버에 한 줄 · 머리 주석 「멤버 10」 → 11.
- **거부한 대안(메인 결정):**
  - `crates/engram-dashboard-engram` — 제품 이름 「Engram Dashboard」와 읽힘이 겹쳐 「engram 패키지」가 무엇을 가리키는지 흐려진다.
  - `src/main.rs` 로 옮긴다 — 경로가 바뀌어 옛 ADR 포인터의 뒤꼬리가 깨지고(ADR-0273 「영향」은 그 경로가 새 패키지의 같은 파일을 가리킨다고 적었다) `include_str!` 의 상대 경로도 고쳐야 한다.
  - lib + bin — lib 를 부를 소비자가 없다. 지금 쓰임이 없는 lib 를 세우는 것이고, 한 파일짜리 lib 는 ADR-0175 결정 6 에 걸린다.

### 2-2. D3 — 의존과 그래프 무게

- **운영 직접 의존:** A′ · A = agent · base · command · serde_json / B = agent · command · serde_json. dev = uuid(B 는 + daemon).
- **빌드 그래프(실측 + 예상):** 데몬의 정상 그래프는 194 패키지다(실측 `cargo tree --locked -p engram-dashboard-daemon -e normal --prefix none --target all` 의 이름 · 판 중복 제거). CLI 는 agent · command · base 의 정상 그래프 합집합 + serde_json 계열 = 약 92(예상 — 구성요소를 각각 잰 합집합이다 · 새 패키지가 없어 직접 못 잰다). 빠지는 것 = tokio · axum · hyper · rmcp · tower · tokio-tungstenite · net · messaging · protocol 등. 남는 것은 ADR-0273 「영향」이 적은 그대로다(agent 의 portable-pty · windows · ts-rs · chrono · base 의 tracing-subscriber · regex · command 의 inventory).
- **async 런타임 0:** agent · command · base 각각의 `-e normal` 그래프에 `^(tokio|mio|tokio-tungstenite|futures-util) ` 가 0줄이다(실측) — CLI 그래프도 0 이어야 하고 D7 의 게이트 ⑦ 줄이 그것을 잰다.
- **Cargo.lock:** 새 `[[package]]` 는 멤버 자신 하나 · 서드파티 0(예상 — 쓰는 서드파티가 전부 이미 lock 에 있다).

### 2-3. D5 — `engram_cli.rs`

- 글자 그대로 `git mv` 한다(히스토리를 잇는다). import 는 agent · serde_json · std 뿐이라 새 패키지의 운영 의존으로 선다.
- 파일명 가드(`:604-616`)는 그대로 값어치를 갖는다 — 상수만 개명하면 이 단언이, `[[bin]] name` 만 개명하면 컴파일이 잡는다(그 주석 `:598-603`). 그 가드가 든 타깃이 조용히 사라지지 않게 CI 스텝이 이름으로 집는다(D7 1).

### 2-4. D4 — 걸음과 설치 위치 폴백의 자리 (★사용자 선택★)

**공통 사실(1-2):** 걸음은 지금 데몬 · 셸 두 벌이고 글자까지 같다. CLI 는 데몬의 `find_install_root`(걸음 + exe 폴더 폴백)를 부른다. 폴백은 개념이 둘이다 — **설치 위치**(체크아웃 루트, 없으면 exe 폴더 — 프라이밍 · CLI help 본문의 기준) · **데이터 루트 디버그 분기**(체크아웃 루트의 `.engram-dev`, 없으면 exe 폴더의 `.engram-dev` → cwd).

**API 이름(C3 · 셋 공통):** 걸음은 가안 `find_checkout_root` · 판정은 `is_checkout_root` 로 부른다 — 「workspace」는 이 저장소에서 에이전트 작업 폴더(cwd)를 가리키는 낱말과 겹친다. **표지 계약(일반 서술):** 「`start`(폴더면 그 자신, 아니면 그 부모)부터 위로 올라가며 `.git`(폴더 또는 파일)이 있거나 본문에 `[workspace]` 를 담은 `Cargo.toml` 이 있는 첫 조상 — 없으면 `None`. `[workspace]` 판정은 부분 문자열이다(TOML 을 해석하지 않는다).」 지금 주석의 저장소 사정(「repo 루트는 .git 으로도 잡힌다」)은 base 로 옮기지 않는다. 판정 동작은 글자 그대로다.

| | 걸음 벌 수 | 설치 위치 폴백 | CLI → 데몬 | 2-2 착지 코드 | ADR 개정 |
|---|---|---|---|---|---|
| **A′(권고)** | 1(base) | 1(base) | 0 | 다시 연다 | ADR-0273 결정 5 · ADR-0282 결정 3 일부 · ADR-0271 「영향」 등(§7 2) |
| **A** | 1(base) | 2(데몬 · CLI — 시험으로 못 묶는다) | 0 | 다시 연다 | 위와 같다 |
| **B** | 3 | 2(같은 경로 시험이 묶는다 — 빈틈 둘) | dev | 안 연다 | 없다(결정 5 를 채울 뿐) |

#### A′ (권고) — 걸음 + 설치 위치 규칙을 base `path` 로

- **모양(가안):** base `path` 에 공개 둘 —
  - `find_checkout_root(start: &Path) -> Option<PathBuf>` — 데몬 `data_dir.rs:170-183` 본문 글자 그대로(+ 비공개 `is_checkout_root` = `:185-196`).
  - `checkout_root_or_exe_dir(exe: &Path) -> Option<PathBuf>` = `find_checkout_root(exe)` 또는 `exe.parent()` — 데몬 `find_install_root`(`:162-168`) 본문에서 `current_exe` 읽기만 뺀 것. exe 경로를 인자로 받아 시험이 프로세스 없이 잰다. ★이름은 일반 낱말로 둔다(R3)★ — 「설치 위치」라는 Engram 쪽 이름은 그것을 부르는 데몬 `find_install_root` · CLI `install_root` 래퍼에만 남는다. **base doc 에 적을 것:** 빌드 모드와 무관하게 늘 위로 걷는다(`cfg` 없음) — 디버그 분기에서만 걷는 `default_data_dir` 와 다르다(릴리스 배포 폴더에서도 걷고, 체크아웃 안에 놓인 배포 폴더면 체크아웃 루트를 낸다 — §5-3).
- **부르는 쪽:** 데몬 `data_dir::find_install_root()` = `current_exe` → `checkout_root_or_exe_dir`(한 줄 — 프라이밍 `priming.rs:275` 는 그대로 이것을 부른다) · CLI 새 `install_root()`(가안 — 같은 한 줄 · `help_file_path` 가 부른다) · 데몬 `default_data_dir` 디버그 분기(`:78`) · 셸 `layout.rs:51` 은 **걸음만** 부르고 자기 폴백(`.engram-dev` · cwd)을 그대로 둔다. ★셸은 디버그 분기 안에서 완전 경로로 부른다★ — 모듈 위에 `use` 를 두면 릴리스 빌드가 「쓰이지 않은 import」 경고를 낸다. 지금의 `cfg_attr` 두 줄은 함수와 함께 사라진다.
- **데이터 루트가 `checkout_root_or_exe_dir` 를 안 쓰는 이유:** 두 개념의 폴백이 지금 우연히 같은 꼴일 뿐이다. 데이터 루트까지 설치 위치 함수에 올리면 설치 위치 폴백을 바꾸는 날(예: macOS 앱 번들의 리소스 폴더) 데이터 폴더가 말없이 함께 옮겨 간다 — 옛 데이터는 옮겨 주지 않는다(ADR-0264).
- **A′ 를 따로 평가한 것(C2):**
  1. **데몬 `find_install_root` 에 데몬 고유 동작이 있나** — 없다(1-2 표). `current_exe` 를 읽는 것 말고는 exe 경로의 순수 함수다. 그 한 줄은 부르는 프로세스가 「내 exe」를 넘기는 일이라 규칙이 아니다.
  2. **입주 조건 ② 가 서나** — 선다고 본다. base 함수는 「`exe` 위의 체크아웃 루트, 없으면 `exe` 의 폴더」라는 경로 계산일 뿐이고(이름도 그 계산을 그대로 말한다 — R3), 그 결과를 「설치 위치」로 읽어 리소스 기준으로 쓰는 것(`prompts/agent-priming.md` · `prompts/engram-help.md` — ADR-0100 동거)은 래퍼 둘과 그 호출부의 몫이다. Engram 고유 이름 · 환경변수 · 데이터 폴더 이름은 하나도 옮기지 않는다. 리뷰가 「exe 폴더로 물러난다」는 선택 자체를 배포 배치 지식으로 보면 A 로 내린다(§8 O2).
  3. **A 의 대가 3(묶이지 않은 폴백)이 사라지나** — 사라진다. 데몬과 CLI 가 같은 함수를 부르고, 다른 것은 각자 넘기는 exe 경로뿐이다. 그 둘이 같은 루트를 내는 것은 exe 동거(개발 = 같은 `target\<profile>\` · 배포 = 한 폴더 — ADR-0100 · `build-release.ps1` 의 기대 목록이 지킨다)가 보장한다.
  4. **OS 에 따라 달라지면** — 지금 `cfg` 0 이다. 나중에 폴백이 OS 마다 달라지면(macOS 번들) 그것은 운영 OS 분기라 platform 으로 가야 한다(CLAUDE.md 「플랫폼 중립」) — A′ 는 옮길 자리가 하나, A 는 둘이다.
- **입주 판정(ADR-0269 결정 7 · base `src/lib.rs` 헤더의 입주 조건 셋):** ① 지금 여러 곳에 복사돼 있다 — 걸음은 데몬 · 셸 두 벌, 설치 위치 규칙은 CLI 로 가면 두 벌이 된다(B · A) ② 도메인 지식 0 — 표지는 `.git` 과 Cargo `[workspace]` 뿐인 개발 체크아웃의 일반 지식이고(ADR-0282 결정 2 가 프로브 접두를 같은 논리로 base 에 들였다), 폴백은 위 2. 그래서 ADR-0269 「입주시키지 않는 것」의 「데이터 폴더 찾기(`default_data_dir`) — Engram 고유 지식」과 ADR-0264 「거부한 대안」의 「경로 정본을 base 에 — 폴더 이름은 도메인 지식」에 걸리지 않는다고 본다 — 옮기는 것은 그 규칙의 범용 부품이지 데이터 루트 규칙이 아니다. 워크스페이스 의존 0 · 새 서드파티 0(std `fs` · `path` 만) · OS `cfg` 0 ③ 입주자끼리 무참조 — std 만 쓴다. 게이트 ③ 알파벳에 `path` 가 이미 있어 사본 여섯을 고치지 않는다.
- **모듈을 `path` 로 두는 이유와 대가:** 「경로 도우미」라는 목적에 맞고 게이트 ③ 이 그대로다. 대가 = `path` 의 서술이 넓어진다 — base `src/lib.rs:6` · CLAUDE.md `:158` · `docs/testing-strategy.md:43` 의 「사람이 친 경로의 철자 고르기」에 「체크아웃 루트 · 설치 위치 찾기(파일시스템을 본다)」를 더하고, `path.rs` 에 모듈 머리(`//!` — 철자 고르기는 파일시스템을 안 보고 루트 찾기는 본다)를 단다. 새 모듈(가안 `checkout`)은 게이트 ③ 사본 여섯을 함께 고쳐야 해서 택하지 않았다(§8 O5).
- **시험:** 데몬 표지 시험 넷 + `unique_tmp` 를 base `path` 로 옮긴다(이름만 새 API 로 · 파일시스템만 쓰고 프로세스를 띄우지 않는다 — base 의 `--test-threads` 판정은 그대로). `find_install_root_yields_absolute_path`(`:463-471`)는 데몬에 남는다. CLI 쪽은 기존 `the_fixed_help_path_resolves_absolute_to_a_real_file`(`engram.rs:4377-4386` — 실제 `prompts/engram-help.md` 파일을 가리켜야 초록)이 잰다.
  - **★「표지가 없다」 갈래는 파일시스템으로 만들지 않는다(리뷰 2라운드 2)★** — 임시 폴더로 그 경우를 만들려면 OS 임시 폴더의 조상에 표지가 없어야 하는데, 그것은 시험이 정할 수 없다(사용자 임시 폴더가 어느 체크아웃 아래 있으면 걸음이 그 표지를 만난다). 그래서 **설계로 표지를 없앤다**: 걸음 본문을 판정 함수를 인자로 받는 비공개 내부 함수(가안 `walk_up_until(start, is_root)`) 하나로 두고, 공개 `find_checkout_root` 는 그것에 `is_checkout_root` 를 넘긴다(동작은 글자 그대로). `checkout_root_or_exe_dir` 도 같은 내부 함수 위에 선다. 시험은 늘 거짓인 가짜 판정을 넘겨 「끝까지 올라가 `None`」 · 「exe 폴더로 물러난다」를, 특정 조상에서만 참인 가짜 판정으로 「그 조상을 낸다」를 잰다 — 디스크도 환경도 안 본다.
  - **새 시험 둘** — `checkout_root_or_exe_dir` 의 두 갈래(가짜 판정 — 위). **옮기는 표지 시험 넷 중 `find_workspace_root_none_when_no_marker`(`:448-461`)는 같은 가정(임시 폴더 위에 표지가 없다)에 기대므로 같은 꼴(가짜 판정)로 바꾼다** — 「`[package]` 단독은 표지가 아니다」는 판정 표 시험(`is_workspace_root_distinguishes_markers` `:473-495`)이 이미 잰다. 나머지 셋(`.git` · `[workspace]` 를 찾는다)은 임시 트리 안에 만든 표지를 바깥 표지보다 먼저 만나므로 임시 폴더 위치와 무관하다.
- **★필수 — 데이터 루트 회귀 시험(C1 · 리뷰 2라운드 1)★:** 1-2 끝의 빈틈을 이 단계가 바로 지나므로 U1 에서 닫는다. 기존 세 자리의 「끝이 `.engram-dev`」 단언을 「`<체크아웃 루트>\.engram-dev` 와 같다」로 조인다 — 데몬 `data_dir_empty_env_falls_through_to_default`(`:284-301`) · `default_data_dir_debug_is_local_data_dir`(`:303-317`)(둘 다 이미 `ENV_LOCK` 아래 `ENGRAM_DATA_DIR` 를 비우거나 지운다) · 셸 같은 경로 시험의 전제(`:242-250` — 셸 판 · 데몬 판 둘 다).
  - **기대 루트** — 걸음이 아니라 `env!("CARGO_MANIFEST_DIR")` 에서 따로 낸다(데몬 = 두 단계 위 · 셸 `src-tauri` = 한 단계 위). 걸음으로 기대값을 내면 같은 결함을 두 번 셀 뿐이다. 비교 = 이름이 `.engram-dev` 인지 + 부모와 기대 루트를 **같은 canonicalize 함수로** 고른 뒤 견준다(`.engram-dev` 자체는 없을 수 있다). 함수는 저장소가 이미 쓰는 `dunce::canonicalize` 다(셸은 운영 의존 · 데몬은 dev 의존 한 줄 — 둘 다 lock 에 있는 판이라 새 패키지 0).
  - **「시험 exe 가 그 체크아웃 안에 있나」 판정** — `current_exe()` 와 기대 루트를 둘 다 같은 canonicalize 로 고른 뒤 `starts_with` 로 본다. ★고르지 않으면 틀린다★ — Windows 의 `Path::starts_with` 는 성분을 대소문자 구분해 견주므로, 드라이브 문자 · 폴더 이름의 대소문자가 갈린 두 경로(cargo 가 cwd 철자를 물려받는다)를 「밖」으로 읽는다.
  - **밖이면** — 환경변수 `CI` 가 있으면 ★실패한다★(GitHub Actions 는 `CI=true` 를 둔다 — CI 에서 건너뜀은 아무것도 실패시키지 않아 눈먼 통과가 된다). 없으면(로컬 · `CARGO_TARGET_DIR` 가 체크아웃 밖 — 그때는 운영 동작부터 exe 폴더 폴백이다 · 1-2 끝) 사유를 `eprintln!` 하고 정확 단언을 건너뛴다. ★libtest 는 통과한 시험의 출력을 가두므로 그 사유 줄은 `-- --show-output` 없이는 안 보인다★ — 그래서 U1 수용 기준은 그 플래그로 돈다(§3-2). CI 는 `CARGO_TARGET_DIR` 를 두지 않으므로(`ci.yml` 에 0회 — 실측) 정확 단언 갈래를 탄다.
  - 시험 수는 그대로다(단언만 조인다). B 를 고르면 이 편집은 선택이다(그 안은 이 분기를 안 지난다).
- **★대가★:** ① 2-2 가 막 착지한 데몬 `data_dir.rs` · 셸 `layout.rs` 를 다시 연다 — 데이터 루트 디버그 분기를 지나는 변경이다(막는 것 = base 표지 시험 · 위 필수 시험 · 셸 같은 경로 시험 · CLI help 경로 시험). ② ADR 문장 여럿이 고쳐진다 — 목록은 §7 2. ③ 메모 §5 원칙(「애매한 것은 crate 를 새로 세우거나 공용으로 두지 않고 일단 데몬으로」 · ADR-0271 「거부한 대안」 셋째 「경로 규칙만 담는 작은 공용 crate」)과 긴장한다 — 새 crate 가 아니고, 범용 도우미가 여러 벌이면 base 로라는 더 좁은 규칙(ADR-0269 결정 7)이 있으며, 2-2 가 쓰기 프로브에 같은 판단을 했다(ADR-0282 결정 2). ④ base 가 공개 함수 둘 · 시험 여섯을 더 진다.

#### A — 걸음만 base

- A′ 에서 `checkout_root_or_exe_dir` 를 빼고, 데몬 `find_install_root` 와 CLI `install_root` 가 각자 「걸음 → exe 폴더」 한 줄을 갖는다.
- **남는 것:** 그 폴백 한 줄이 두 벌이고 둘을 묶는 시험이 없다(묶으려면 CLI → 데몬 간선이 다시 선다). 갈렸을 때의 결과 = CLI 가 help 본문 파일을 못 찾아 바이너리에 박힌 사본으로 답하고 stderr 에 한 줄을 남긴다(`:222-231` · `load_help_text`) — 우편 · 제어 동사는 영향이 없다. 시험이 그 갈래를 못 타므로(늘 저장소 안에서 돈다) 릴리스 1회(§5-3)가 덮는다.
- A′ 보다 나은 점은 base 가 폴백 정책을 지지 않는다는 것 하나다(§8 O2).

#### B — ADR-0273 결정 5 그대로

- CLI `engram.rs` 에 `find_install_root` · 걸음 · 판정 사본(데몬 `data_dir.rs:162-196` 글자 그대로).
- **같은 경로 시험(가안 `the_cli_install_root_is_the_daemons`)** = CLI 판 == `engram_dashboard_daemon::data_dir::find_install_root()`. 그 시험은 **CLI bin 의 단위 시험에 산다** — bin 전용 패키지는 남이 의존할 수 없어 데몬 쪽에 둘 수 없다. 그래서 CLI → (dev) 데몬 간선이 선다. ADR-0273 결정 5 가 미룬 「그 시험이 어느 패키지에 사는지」를 이것으로 채운다.
- **빈틈(ADR-0282 결정 3 과 같은 것):** 이 저장소 루트에 표지 둘이 다 있어 표지 하나를 빠뜨린 사본도 같은 루트를 낸다 · 시험은 늘 저장소 안에서 돌아 exe 폴더 폴백 갈래를 대조하지 못한다. 표지 시험 넷을 CLI 에도 베끼면 첫째는 막힌다(+4 · 제안).
- **대가:** 걸음 세 벌 · CLI 시험 빌드가 데몬 전체(tokio · axum · rmcp …)를 다시 짓는다 · 2-4 가 다룰 dev 간선이 하나 는다. **얻는 것:** 데몬 · 셸 코드를 건드리지 않는다 · ADR 개정이 없다.

**권고 = A′** — 걸음과 설치 위치 규칙이 각각 한 벌이 되어 A 가 남기던 묶이지 않은 폴백이 사라지고, CLI 를 떼는 목적(데몬을 끌고 오지 않는다)이 시험 빌드까지 선다. 데몬 `find_install_root` 에는 데몬 고유 동작이 없어 옮겨도 잃는 것이 없다. 대가는 2-2 착지 코드를 다시 여는 것 · ADR 문장 개정 · base 가 폴백 한 줄을 지는 것이다.

### 2-5. D6 — 빌드 입구 · 바인딩

- **`build-release.ps1:133-135` → 두 호출, 각자의 `Invoke-Step`(F1 · 제안):**

  ```powershell
  Invoke-Step 'cargo build daemon (engram-dashboard-daemon)' {
      & cargo build --release --manifest-path (Join-Path $ProjectRoot 'Cargo.toml') -p engram-dashboard-daemon --bin engram-dashboard-daemon
  }
  Invoke-Step 'cargo build cli (engram-dashboard-cli)' {
      & cargo build --release --manifest-path (Join-Path $ProjectRoot 'Cargo.toml') -p engram-dashboard-cli --bin engram
  }
  ```

  ★한 블록에 둘을 넣지 않는다★ — `Invoke-Step`(`:61-66`)은 블록이 끝난 뒤 `$LASTEXITCODE` 를 한 번만 보므로 앞 호출의 실패를 뒤 호출의 성공이 덮고, 존재 검사(`:139-142`)는 이전 빌드의 낡은 exe 를 받아들인다. **두 호출인 이유:** 한 호출(`-p A -p B --bin x --bin y`)은 `--bin` 이 고른 두 패키지 모두에 걸리는 꼴이라 cargo 동작을 확인해야 하고(안 돌렸다 · §9), CLI 가 데몬의 기능 집합으로 지어진다. **비용(C3):** 두 호출이면 serde_json 이 기능 집합 둘로 따로 지어지고(데몬 = `alloc` · `raw_value` 포함 · CLI = `default` · `std` — 1-5) 그것을 쓰는 agent · command 도 CLI 호출에서 다시 지어진다 — 릴리스 프로필(`lto` · `codegen-units = 1`)이라 릴리스 시간이 는다(얼마인지는 미검). 시간이 문제가 되면 한 호출로 합치는 쪽을 다시 본다(§8 O10). 같은 편집: `:127-132` 주석(「데몬+CLI 는 … 별개 crate 의 두 [[bin]]」 → 「별개 패키지 둘」) · 헤더 `:11`(가드 시험 자리 — 전체 경로로). `--all-targets` 금지 주석은 그대로 둔다.
- **런처 둘:** `rebuild-run-debug.bat:93` → `cargo build -p engram-dashboard-daemon -p engram-dashboard-cli` · `rebuild-run-release.bat:111` → 같은 꼴 + `--release` · 두 파일의 echo 문구 · `rebuild-run-release.bat:5-8` 주석(「`tauri build` 가 데몬을 안 짓는다」에 CLI 를 더한다). 런처는 한 호출이라 기능이 합쳐져 한 번에 지어진다(개발 루프라 상관없다). ★두 파일은 CRLF 다(`.gitattributes` 의 `*.bat text eol=crlf`)★ — 편집 뒤 `file` 로 본다. `launcherWiring.test.ts` 는 그대로 초록이어야 한다(1-3).
- **`roundtrip_smoke.rs` 안내문(F3):** `:66`(doc) · `:424`(SETUP-SKIP 문자열)의 `cargo build -p engram-dashboard-daemon --features test-harness --bin engram` → `cargo build -p engram-dashboard-cli`(CLI 에는 `test-harness` 기능이 없다 · 같은 profile 이라 하네스 exe 옆에 놓인다). `:67` 의 `cargo run -p engram-dashboard-daemon --features test-harness --bin roundtrip-smoke` 는 그대로다. 실행 중 찍히는 문자열이 바뀐다 — 개발자가 보는 안내문이다(§9 끝).
- **★릴리스 스크립트 1회는 2-3 의 master 머지 전 필수(F2)★** — 이 스크립트와 버전 게이트는 CI 에서 태그 push 때만 돈다(`ci.yml:1281-1283`). 태그는 다시 쓸 수 없으므로(CLAUDE.md 「태그」 — 되돌리기 어렵고 재시도도 안 된다) 거기서 처음 깨지면 그 태그가 버려진다. U2 의 QA 가 §5-3 을 돈다.
- **qa 바인딩 편집 제안 — ★사용자 승인 뒤 적용★**(바인딩은 사용자 소관이다):
  - **U2 와 함께(빌드 명령):** `:320` · `:355` · `:360` 의 `cargo build -p engram-dashboard-daemon` → `cargo build -p engram-dashboard-daemon -p engram-dashboard-cli`(문구 「백엔드/데몬을 고쳤으면」 → 「백엔드 · 데몬 · CLI 를 고쳤으면」). ★U2 의 QA 전에 승인이 필요하다★ — 안 고치면 U2 의 GUI 실측이 옛 engram.exe 를 잰다(§8 O3).
  - **`:66` 은 그대로** — 셸 실 데몬 레인(`stop_smoke` · `real_wmi`)은 CLI 를 쓰지 않는다(데몬은 CLI 없이도 뜬다 · fail-open).
  - **U3(게이트 · 서술):** `:138` `agent`·`daemon`·`platform` → + `cli`(실 exe 41건) · `:253` 메시징 정규식 「새 워크스페이스 crate가 생기면 여기에 더하고」 뒤에 bin 전용 예외(D7 8 의 문구) · `:254-261` 의존 상한 「4종」 → 「5종」 + cli 줄(기대 = 이름 집합 — D7 2) · `:262-275` test-support 두 블록에 `-p engram-dashboard-cli` 0줄 줄 하나씩 · `:79-84` 게이트 ⑦ 블록에 cli 0줄 줄 · `:442` `crates/engram-dashboard-daemon/src/bin/engram.rs` → `crates/engram-dashboard-cli/src/bin/engram.rs` · `:445` 「데몬 패키지의 `[[bin]] engram` 이라 데몬 exe 와 같은 target 폴더에」 → 「CLI 패키지(`engram-dashboard-cli`)의 `[[bin]] engram` — 워크스페이스가 target 폴더 하나를 나눠 써 데몬 exe 옆에」.

### 2-6. D7 — 게이트 등록

1. **CI 단독 기립 스텝**(backend 잡 · agent 스텝 뒤 · 제안): `cargo test --locked -p engram-dashboard-cli --bins --test engram_cli`. 값어치 둘 — ① `-p` 로 따로 지어 데몬과의 기능 합집합 없이 서는지(이 패키지의 존재 이유) ② 파일명 가드가 든 `engram_cli` 타깃이 사라지면 「no test target」로 죽는다(셸 `--test lib_unit` 과 같은 값어치 — ADR-0094 가드가 조용히 증발하지 않는다). CI 는 `--test-threads` 를 쓰지 않는다(CLAUDE.md 「빌드·검증 명령」).
2. **의존 상한**(gates 잡 · 가안 이름 `cli gate 1: direct workspace deps — exactly agent · base · cli · command`) — 꼴은 net 게이트 3(`cargo tree --locked -p … --depth 1 --prefix none -e normal,dev,build --target all --all-features`)을 따르되 **판정은 줄 수가 아니라 이름 집합 일치**다(첫 칸을 정렬해 heredoc 과 견준다). 수만 세면 base 자리에 데몬이 들어와도 넷이라 이 단계의 요점(데몬 0)을 못 본다(§8 O6). 기대: A′ · A = agent · base · cli · command / B = agent · cli · command · daemon(dev).
3. **test-support 운영 그래프 고리 둘**(`ci.yml:675` · `:767`) — `for pkg in engram-dashboard-daemon engram-dashboard engram-dashboard-cli` · 스텝 이름 「(daemon · shell · cli = 0, agent dev >= 1)」 · 주석 「운영 바이너리를 내는 둘」 → 셋. CLI 그래프에 base · platform 이 agent 를 거쳐 있으므로 `-i` 가 rc 101 로 죽지 않는다(그래프 논리 — U3 에서 실측).
4. **platform 게이트 ⑦** — `async_ingress zero -p engram-dashboard-cli` 한 줄(`ci.yml:1227` 다음). CLI 는 동기 crate 이고 데몬을 떠나는 이유가 tokio 를 끌고 오지 않는 것이라 그 사실을 기계로 잰다 — agent 줄이 이미 덮는 것(agent · base · command · platform)에 더해 CLI 자신의 직접 서드파티(지금 serde_json)를 덮는다. 같은 편집: 스텝 이름 · platform `src/lib.rs:166-180` 헤더의 「대상 셋」 → 넷 · qa `:79-84` · CLAUDE.md `:222`(§8 O7).
5. **버전 게이트(F7)** — `check crates/engram-dashboard-cli/Cargo.toml …` 한 줄(`:1411` 다음) · 주석 「5건」 → 「6건」(`:1355`) · `:1420` 의 `PASS — 제품 버전 선언 5건` → 6건 · 셋째 「daemon.exe·engram.exe 를 만드는 crate」(`:1362`) → 「daemon.exe 를 만드는 crate」 + 새 항목 「cli — engram.exe 를 만드는 패키지. 코드는 지금 버전을 읽지 않지만 단독 배송물이 있어 「일부러 뺀 것」의 라이브러리 기준(`:1373-1375`)에 들지 않는다 — 나중에 `engram --version` 같은 것이 생기면 거짓 버전을 막는다」. 대가 = 릴리스마다 고칠 매니페스트가 하나 늘고, 빠뜨리면 ★태그를 push 한 뒤에야★ 드러나 그 태그를 버린다(§8 O4). 그래서 U3 수용 기준이 그 함수를 로컬에서 한 번 돌린다.
6. **주석** — `ci.yml:33` 의 포인터 → `crates/engram-dashboard-cli/src/bin/engram.rs:231`(U2 편집은 `:336-340` 이라 `:231` 은 그대로다 — U3 에서 줄을 다시 본다).
7. **CLAUDE.md** — 「빌드·검증 명령」에 `cargo test -p engram-dashboard-cli -- --test-threads=4`(근거 = 「병렬은 테스트 바이너리마다 걸린다」 · 실 exe 41건) · 의존 상한 줄 · 시험 기능 게이트 두 줄(`:259` · `:261`)의 「같은 꼴 `-p engram-dashboard`(셸)」 옆에 cli · `:231` 병렬 항목의 괄호(F4) — 「프로세스 레벨 CLI 스위트」는 cli 몫으로 옮겨 적고, 데몬의 근거는 「기본 실행의 실 PTY 셸 스폰 + `#[ignore]` 실 데몬 exe 시험」으로 고친다(1-5) · `:222`(⑦ 대상) · `:207`(`inventory` — 「데몬·셸 릴리즈 바이너리」 → + CLI) · `:255` 메시징 정규식 줄에 bin 전용 예외(아래 8) · 「백엔드 모듈 맵」 cli 항목(가안):
   > **cli** — 제어 평면 CLI `engram.exe` 를 뽑는 bin 전용 패키지(ADR-0273 · ADR-0284). 스폰된 에이전트가 데몬 제어 라우트에 손조립 HTTP 로 붙는다. 의존 = agent(명령 어휘) · command(요청 번호) · base(설치 위치 · 체크아웃 루트 — D4) · serde_json — 데몬 · tokio 없음(게이트 = cli 의존 상한 · platform 게이트 ⑦ 의 cli 줄). bin 이름 = agent `CLI_EXE_NAME`(바꾸면 우편이 조용히 멈춘다 · ADR-0094).
8. **이름 알파벳 규칙의 bin 전용 예외(F8)** — 메시징 정규식(`ci.yml:520-523` 주석 · qa `:253` · CLAUDE.md `:255`)은 「새 워크스페이스 crate 가 생기면 이름을 더한다」고 적는다. cli 는 **더하지 않는다** — lib 타깃이 없어 아무도 `engram_dashboard_cli::…` 로 부를 수 없고, 그 정규식이 막는 것(메시징이 다른 crate 를 부르는 것)이 성립할 자리가 없다. 문구 제안(세 곳 같게): 「단 lib 타깃이 없는 bin 전용 멤버(`engram-dashboard-cli`)는 더하지 않는다 — 부를 수 있는 crate 가 아니다(ADR-0284). 그런 멤버에 lib 를 세우는 날 여기 이름을 더한다.」 net 게이트 1 · 2b 의 이름 목록은 「모든 crate」 규칙이 아니라 고칠 것이 없다.
- **고치지 않는 게이트:** platform 게이트 ④ · ⑥ 명단(1-5) · ⑤(저절로) · 셸 게이트 1(셸 그래프 불변) · base 게이트 ①②③(A′ · A 도 알파벳 불변) · net · transport 게이트.

### 2-7. D8 — `scripts/engram.mjs`

머리(`:2`)가 스스로 「THROWAWAY 스파이크 … 롤백 예정」이라 적는 Node 스크립트다. 데몬 WS 로 붙고 `daemon.json` 후보를 자기 코드로 찾는다(`:13-`) — engram CLI(HTTP 제어 라우트)와 코드도 경로 규칙도 나누지 않는다. 그 자리는 T-23(사람용 데몬 명령줄 표면 — 사용자 결정 대기) · ADR-0132 몫이라 2-3 이 건드릴 줄이 없다.

### 2-8. D9 — 새 ADR

§7. ADR-0273 「영향」의 읽기 규칙(「그 경로는 새 패키지의 같은 파일을 가리킨다」)은 `src/bin/engram.rs` 하나만 덮는다. ADR-0282 의 목록만큼 길 필요는 없지만 `engram_cli.rs`(ADR-0220 · ADR-0232 가 부른다) · 데몬 `[[bin]] engram` · (A′ · A) 데몬 걸음을 짧게 더하고, 이 단계가 고치는 옛 문장을 명시한다(§7 2).

---

## 3. 작업 단위

**원칙(사용자 2026-10-02 — 「망가지지 않는 단위로」 · 「임시 땜빵 금지」):** 단위마다 워크스페이스가 빌드되고 회귀가 초록인 채 끊는다. A′ 는 새 자리를 먼저 세우고(U1) CLI 가 그 위로 이사한다(U2) — 그래서 CLI 가 데몬을 잠시 의존하거나 사본을 잠시 갖는 중간 상태가 없다.

### 3-1. 단위 표 (A′)

| 단위 | 범위 | 건드리는 파일 | 리뷰 · QA |
|---|---|---|---|
| **U1** | D4-A′ — base 걸음 · 설치 위치 · 데몬 · 셸 전환 · 데이터 루트 시험 조이기(CLI 무변경) | base `src/path.rs`(공개 둘 · 비공개 둘 · 시험 여섯 · 모듈 머리) · `src/lib.rs:6` · 데몬 `src/data_dir.rs`(`:78` · `:162-196` · 시험 `:284-317` · `:402-495` · 머리) · 데몬 `Cargo.toml`(dev `dunce` 한 줄) · `Cargo.lock`(데몬 항목의 의존 목록 — 새 패키지 0) · 셸 `src/discovery/layout.rs`(`:51` · `:99-127` · 머리 · 같은 경로 시험 `:204-207` · `:242-250`) · CLAUDE.md `:158` · `docs/testing-strategy.md:43` | `/implement standard` · `/review code full` · `/qa standard` |
| **U2** | ★원자★ D1 · D2 · D3 · D5 · D6 — 이사 · CLI 가 base 를 부른다 · 빌드 입구 넷 | 새 `crates/engram-dashboard-cli/Cargo.toml` · `git mv` 둘(`src/bin/engram.rs` · `tests/engram_cli.rs`) · `engram.rs:336-340`(+ 앵커) · 데몬 `Cargo.toml:33-39` · 데몬 `src/bin/roundtrip_smoke.rs:66` · `:424` · 루트 `Cargo.toml`(멤버 · 머리) · `Cargo.lock` · `build-release.ps1`(`:11` · `:16-17` · `:127-135`) · `rebuild-run-debug.bat:92-93` · `rebuild-run-release.bat`(`:5-8` · `:110-111`) · `scripts/README.md:12` · (승인 뒤) qa `:320` · `:355` · `:360` | `/implement standard` · `/review code full` · `/qa full`(§5-1 · §5-3 필수) |
| **U3** | D7 · D9 · 문서 | `ci.yml`(새 스텝 둘 · `:33` · `:520-523` · `:675` · `:767` · ⑦ · 버전 게이트 `:1355-1420`) · platform `src/lib.rs:166-180` · CLAUDE.md · (승인 뒤) qa 나머지 · `docs/testing-strategy.md` · `docs/reference/architecture-overview.md` · 코드 주석(§6 3) · ADR-0284 | `/implement standard` · `/review code full`(+ doc 렌즈) · `/qa standard` + 새 게이트 로컬 1회 |

**순서 U1 → U2 → U3.** U1 이 먼저인 이유 — U2 의 CLI 가 base 의 설치 위치 함수를 부른다. **파일 겹침:** CLAUDE.md(U1 · U3) · qa 바인딩(U2 · U3) · `engram.rs`(U2) · 데몬 `data_dir.rs`(U1) · `ci.yml`(U3) → **한 코더씩 직렬**. **critical 이 아닌 이유:** kill 인과 · finalize · 락 순서 · replay 어느 것도 지나지 않는다 — U1 은 데이터 루트 디버그 분기를 지나지만 본문을 글자 그대로 옮기고 조인 시험이 묶는다.

### 3-2. U1 — base 걸음 · 설치 위치 · 데몬 · 셸 전환

- **단위 안 순서(어디서 멈춰도 빌드가 선다):** ① base `path` 에 `find_checkout_root` · `is_checkout_root` · `checkout_root_or_exe_dir` · 내부 `walk_up_until` + 시험 여섯(「표지 없음」 둘은 가짜 판정 — §2-4) + `unique_tmp` + 모듈 머리 · base `src/lib.rs:6`(아무도 안 부른다) ② 데몬 `data_dir` — `find_install_root` 가 `checkout_root_or_exe_dir` 를, `default_data_dir` 디버그 분기가 `find_checkout_root` 를 부르고 자기 두 함수 · 표지 시험 넷 · `unique_tmp` 를 지운다 · 데이터 루트 시험 둘을 조인다(C1 — dev `dunce` 와 `Cargo.lock` 이 같은 커밋) ③ 셸 `layout.rs` 가 디버그 분기 안에서 `find_checkout_root` 를 완전 경로로 부르고 두 함수 · `cfg_attr` 두 줄 · `:99` 주석을 지운다 · 같은 경로 시험의 전제(`:242-250`)를 조이고, 주석(`:204-207`)에서 「표지 하나를 빠뜨린 사본도 못 잡는다 · 표지 시험은 데몬에만」을 지운다(걸음이 한 벌이라 그 빈틈의 대상이 없다) ④ 서술 사본(CLAUDE.md `:158` · `docs/testing-strategy.md:43`).
- **수용 기준:** ① `rg "fn (find_workspace_root|is_workspace_root|find_checkout_root|is_checkout_root|checkout_root_or_exe_dir)" crates src-tauri` → base `path.rs` 셋만 ② base 게이트 ①②③ PASS(③ 알파벳 불변) ③ 데몬 `data_dir` 시험 · 셸 같은 경로 시험 초록 — 조인 단언이 정확 비교로 돌았다: `cargo test -p engram-dashboard-daemon data_dir -- --show-output --test-threads=4` · `cargo test -p engram-dashboard --test lib_unit the_shell_copies -- --show-output` 의 출력에 건너뜀 사유 줄이 없다(★`--show-output` 를 빼면 통과한 시험의 출력이 갇혀 그 줄이 있어도 안 보인다★ · 기본 target 폴더) · CI 는 `CI` 가 있어 밖이면 실패하므로 초록 = 정확 비교다 ④ CLI 무변경(아직 데몬 `find_install_root` 를 부른다 — 그 함수가 base 를 부를 뿐이다) ⑤ 셸 `layout.rs` 의 `cfg_attr(not(debug_assertions), allow(dead_code))` 가 `LOCAL_DATA_DIR`(`:20`) 하나만 남는다.
- **게이트:** `cargo fmt --check` · `cargo test -p engram-dashboard-base` · `cargo test -p engram-dashboard-daemon -- --test-threads=4` · `cargo test -p engram-dashboard --test lib_unit` · `cargo build` · ★릴리스 분기 컴파일 1회★ `cargo check --release -p engram-dashboard-daemon -p engram-dashboard`(디버그 전용 호출이 릴리스에서 경고를 내지 않는지 — 시험은 늘 debug 다 · 분리 실행) · base 게이트 ①②③ · CI.
- **위험:** 걸음을 글자 그대로 옮기지 않으면 데이터 루트가 조용히 exe 폴더로 떨어진다 — 조인 시험이 그것을 잡는다(C1) · `path` 서술 사본 셋 중 하나를 빠뜨린다 · 조인 시험의 정확 단언이 「건너뜀」으로 늘 빠지면 시험이 이름뿐이 된다 — CI 에서는 건너뜀 대신 실패이고(`CI` 환경변수), 로컬은 수용 기준 ③ 의 `--show-output` 로 본다 · 「안에 있나」 판정을 canonicalize 없이 하면 대소문자만 다른 경로를 밖으로 읽어 로컬에서 늘 건너뛴다(§2-4).

### 3-3. U2 — 이사(원자)

- **왜 쪼갤 수 없나:** 두 패키지가 같은 bin 이름 `engram` 을 가지면 cargo 는 막지 않고 산출 파일 충돌을 경고한다 — 같은 `target\<profile>\engram.exe` 를 서로 덮어써 어느 쪽이 남는지가 빌드 순서에 달린다(F9). 릴리스 스크립트는 데몬의 `--bin engram` 이 사라지는 커밋부터 깨지므로 같은 커밋이어야 한다(ADR-0273 「영향」).
- **단위 안 순서(한 커밋):** ① 새 `Cargo.toml` + 루트 멤버 ② `git mv` 두 파일 ③ `engram.rs:336-340` — `install_root()`(`current_exe` → base `checkout_root_or_exe_dir`) · 앵커 줄을 고친다(「데몬 lib 를 부르는 유일한 줄 …」 → 「설치 위치 = base `checkout_root_or_exe_dir` — 데몬 `find_install_root` 와 같은 함수(ADR-0273 · ADR-0284)」) · `help_file_path` doc 의 「프라이밍과 같은 앵커」는 참으로 남는다 ④ 데몬 `Cargo.toml:33-39` 삭제 ⑤ 빌드 입구 넷(`build-release.ps1` 두 `Invoke-Step` · 런처 둘 · `roundtrip_smoke.rs:66` · `:424`) · `scripts/README.md:12` ⑥ `Cargo.lock` ⑦ (승인됐으면) qa 빌드 명령 셋.
- **수용 기준:** ① `rg engram_dashboard_daemon crates/engram-dashboard-cli` → 0 ② `cargo tree --locked -p engram-dashboard-cli --depth 1 --prefix none -e normal,dev,build --target all --all-features` 의 워크스페이스 줄 = agent · base · cli · command ③ `cargo test -p engram-dashboard-cli -- --test-threads=4` = bin 단위 124 · `engram_cli` 41 전부 초록 ④ `cargo test -p engram-dashboard-daemon -- --test-threads=4` 초록 ⑤ `git diff --stat -M` 에 rename 둘 ⑥ `npm test`(launcherWiring 포함) 초록 ⑦ platform 게이트 ④ · ⑤ · ⑥ PASS(명단 불변) · 셸 게이트 1 PASS ⑧ `rg -n "engram-dashboard-daemon[^\n]*--bin engram($|[^-\w])" crates src-tauri scripts .github CLAUDE.md .claude/skill-bindings docs/testing-strategy.md` → 0(F3 — 끝을 `\b` 로 적지 않는다: `--bin engram-dashboard-daemon` 의 `-` 앞도 단어 경계라 그 줄이 걸린다) ⑨ ★§5-3 릴리스 스크립트 1회 PASS★(F2 — 머지 전 필수).
- **게이트:** `cargo fmt --check` · `cargo build` · `cargo test --workspace -- --test-threads=4`(§3-6 대조) · `npm test` · CI(`--locked`) · `/qa full`(§5-1 · §5-3).
- **위험:** `.bat` CRLF · 빌드 입구를 하나 빠뜨리면 그 입구만 조용히 낡은 exe 를 쓴다(1-3) · 릴리스 두 호출을 한 블록에 두면 앞 실패가 묻힌다(F1) · qa 빌드 명령이 승인 전이면 GUI 가 낡은 exe 를 잰다(§8 O3) · `help_repo_root` · `include_str!` 의 깊이(같다 — 컴파일과 시험이 잰다).

### 3-4. U3 — 게이트 · 문서

- **단위 안 순서:** ① CI 새 스텝 둘(단독 기립 · 의존 상한) ② 고리 둘 · 게이트 ⑦ · 버전 게이트 ③ `ci.yml:33` · `:520-523` 주석 · platform 헤더 ④ CLAUDE.md · (승인 뒤) qa 나머지 ⑤ 문서 · 코드 주석 · ADR-0284 · step-log. 전부 더하는 편집이라 어느 칸에서 멈춰도 빌드가 선다.
- **수용 기준:**
  1. 새 게이트를 로컬에서 한 번 — 의존 상한 집합 일치 · 고리 셋 0줄(짝 그대로 1줄 이상) · ⑦ cli 0줄(짝 그대로) · ★버전 게이트의 `cargo_version` · `check` 를 지금 버전으로 돌려 6건 PASS★(태그에서만 도는 게이트라 여기서 미리 잰다 — F7).
  2. **옛 자리 포인터 0(F6 — 짧은 꼴까지):** `rg -nP "engram-dashboard-daemon/(src/bin/engram\.rs|tests/engram_cli\.rs)|(?<!engram-dashboard-cli/src/)\bbin/engram\.rs|(?<!engram-dashboard-cli/)\btests/engram_cli\.rs" crates src-tauri scripts .github CLAUDE.md .claude/skill-bindings docs/testing-strategy.md docs/reference/architecture-overview.md -g '!crates/engram-dashboard-cli/**'` → 남는 것은 CLAUDE.md `:229`(수치 이력 속 「`bin/engram.rs` 의 훅 구획」 — 역사 서술이라 그대로 둔다) 하나뿐. 지금(`d421fcf`) 이 명령의 결과 = 1-5 의 짧은 꼴 · 전체 꼴 포인터 목록 + CLAUDE.md `:229` + U2 가 없애는 둘(데몬 `Cargo.toml:39` 의 `path = "src/bin/engram.rs"` — 줄째 지운다 · `tests/engram_cli.rs:599` — 파일째 cli 패키지로 가 검색 범위에서 빠진다)이다(실측). ★그래서 고칠 때는 「cli `src/bin/engram.rs`」 같은 짧은 꼴을 쓰지 않고 `engram-dashboard-cli/src/bin/engram.rs` · `engram-dashboard-cli/tests/engram_cli.rs` 로 적는다★ — 짧은 꼴이면 이 명령이 다시 잡는다(데몬 crate 안에서 읽으면 데몬 자리로 오독되는 것도 그 이유다). 덧붙여 `rg -n "engram_cli" docs/testing-strategy.md` 가 새 cli 절에만 있다(데몬 절 `:93` 의 목록 꼴은 위 정규식이 못 본다). ★ADR · 과정 기록 · 핸드오프 · 날짜 박힌 스냅숏은 고치지 않는다★.
  3. `rg -n "daemon.exe·engram.exe|선언 5건|무엇을 보나 \(5건\)" .github` → 0.
- **위험:** 버전 게이트는 태그에서만 돈다 — 수용 기준 1 이 로컬로 대신 잰다 · 의존 상한을 수로 적으면 요점을 잃는다(D7 2).

### 3-5. A · B 의 차이 (압축)

| 안 | 단위 | A′ 와 다른 점 |
|---|---|---|
| **A** | U1 | `checkout_root_or_exe_dir` 와 그 시험 둘이 없다 · 데몬 `find_install_root` 는 자기 「걸음 → exe 폴더」 한 줄을 지킨다 |
| **A** | U2 | CLI `install_root()` 가 같은 한 줄을 갖는다(`current_exe` → base 걸음 → exe 폴더) |
| **B** | B-U1 | 원자 이사 — CLI 가 데몬을 normal 로 잠시 의존한다(지금 데몬 패키지 bin 이 지는 짐 그대로 · ADR-0282 「영향」의 임시 간선이 이어진다) · `:338` 그대로 · 빌드 입구 넷은 U2(A′) 와 같다 · A′ 의 U1 이 없다(데이터 루트 시험 조이기는 선택) |
| **B** | B-U2 | CLI 사본 + 같은 경로 시험 + (제안) 표지 시험 넷 사본 · 데몬 normal → dev(`engram.rs` · cli `Cargo.toml` · `Cargo.lock`) |
| **B** | B-U3 | U3 와 같다 — 의존 상한 기대 = agent · cli · command · daemon · base 서술 편집 없음 |

### 3-6. 단위마다 검증 — 회귀 수 대조

1-3 TRD §5-2 의 명령 그대로다(`cargo test --workspace -- --test-threads=4` 를 앞뒤로 돌려 결과 줄 수 · 통과 · 실패 · 무시를 견준다 · 줄 수가 줄면 타깃 소실 — 초록이어도 멈춘다). 아래는 전부 예상이다(A′).

| 단위 | 결과 줄 | 통과 | 무시 |
|---|---|---|---|
| U1 | 그대로 | **+2** — base +6(옮긴 표지 넷 + `checkout_root_or_exe_dir` 둘) · 데몬 −4 · 조인 시험은 단언만 바뀐다 | 그대로 |
| U2 | 그대로 — 데몬 −2(`unittests src/bin/engram.rs` · `tests/engram_cli.rs`) · cli +2(같은 둘 · bin 전용이라 Doc-tests 줄 없음) | 그대로 — 165(124 + 41)가 자리만 옮긴다 | 그대로(두 파일 다 `#[ignore]` 0) |
| U3 | 그대로 | 그대로 | 그대로 |
| 합 | 0 | +2 | 0 |

A: U1 통과 0(base +4 · 데몬 −4). B: B-U2 에서 +1(같은 경로 시험 · 표지 사본 넷까지면 +5) · 결과 줄 0. **기준선은 U1 착수 직전에 잰다** — 마지막 기록(CLAUDE.md 「빌드·검증 명령」 = 결과 줄 59 · 4229 통과 · 0 실패 · 31 무시 · `79b8d09` 흡수 머지 직전)을 이 TRD 는 돌려 보지 않았다. 셸 lib_unit(819)은 어느 안에서도 그대로다 — 셸은 단언만 조인다.

### 3-7. U1 착수 체크리스트

0. **전제 셋.** ① D4 사용자 선택(A′ 면 아래 · A 면 §3-5 의 A 줄을 겹쳐 · B 면 B-U1 부터). ② 새 ADR(§7 — 채번 · 링크 = `/adr` · 커밋 직전 번호 재확인 — 0283 은 이미 master 에 있다). ③ 되돌릴 지점 — 이 TRD 를 로컬 커밋해 출발점을 만든다(코드 트리 = `d421fcf`).
1. **출발 수치** — §3-6 명령(분리 실행 — qa 바인딩 「분리 실행」).
2. **손댈 파일(정확히):** §3-1 의 U1 줄.
3. **순서:** §3-2 의 ①~④.
4. **게이트 · 수치:** §3-2 게이트(릴리스 분기 `cargo check` 포함) → §3-6 대조(결과 줄 그대로 · 통과 +2).
5. `/review code full` → `/qa standard` → 게이트 초록 뒤 커밋(`S21: refactor(base): …` — 스텝 번호는 step-log 를 잇는다 · 끝에 Co-Authored-By 트레일러).

---

## 4. 의존 그래프 — 2-3 뒤

| crate | 직접 워크스페이스 의존(normal · build) 지금 | 2-3 뒤 |
|---|---|---|
| cli(신설) | — | A′ · A: agent · base · command / B: agent · command(+ dev daemon) |
| daemon | agent · base · command · messaging · net · platform · protocol | 그대로 — bin `engram` 만 나간다 |
| 셸(`engram-dashboard`) | base · command · net · platform · protocol(+ dev daemon · platform `test-support`) | 그대로 |
| base | 없음 | 없음(A′: `path` 에 걸음 · 설치 위치가 든다) |

- **engram.exe 를 짓는 그래프** — 데몬 패키지 전체(정상 194 · 실측) → agent · command · base · serde_json(약 92 · 예상).
- **사라지는 간선** — CLI → 데몬 lib(2-2 U2 가 만든 임시 간선 · ADR-0282 「영향」). A′ · A 는 0 · B 는 dev 로 남는다.
- **그대로인 게이트** — 셸 게이트 1(그 짝 「데몬 → agent ≥ 1」도 그대로 선다) · base 게이트 ①②③ · platform 게이트 ①~⑥ · net · 메시징(이름 알파벳 포함 — D7 8) · transport 게이트. **바뀌는 게이트** — §2-6.

---

## 5. GUI 실측

절차(기동 인자 · 환경변수 · PID · teardown)는 `/qa` 바인딩 §full 이 갖는다. 여기는 무엇을 볼지만 적는다.

### 5-0. 전제 (경고 — 사용자 확인)

- **데이터 폴더** — 셸이 띄운 데몬은 WMI 라 늘 `<저장소>\.engram-dev` 를 쓴다. 그 명부의 자동 복원 프로필 · 살아 있는 개발 데몬에 대한 확인은 2-2 TRD §5-0 과 같다(§8 O8).
- **★데몬과 CLI 를 함께 짓는다★** — `cargo build -p engram-dashboard-daemon -p engram-dashboard-cli`(qa 빌드 명령이 승인 전이면 손으로 · 분리 실행). `node scripts/build-client-shell.mjs` 는 둘 다 짓지 않는다.
- **릴리스 1회(§5-3)** — 조립 폴더는 모든 체크아웃 밖(§5-3 첫 항목) · `target\release` 의 데몬 · CLI exe 를 쓰고 있는 릴리스 앱이 없어야 한다(있으면 링크가 os error 5 로 진다 — qa 바인딩 「공유 데몬 바이너리 락」). 그 앱을 끄는 것은 사용자 확인 뒤다.

### 5-1. U2 뒤 (debug)

1. `target\debug\engram.exe` 가 이번 빌드 산출인지(수정 시각) — 옛 exe 를 재지 않는다.
2. **비GUI 둘(데이터 폴더를 안 건드린다):** `target\debug\engram.exe help` → stdout 에 계열 목록 · exit 0 · ★stderr 가 비어 있다★(「내장 사본으로 답한다」 줄이 없다 = 설치 위치 함수가 저장소 루트의 `prompts/engram-help.md` 를 찾았다) · 인자 없는 호출도 같은 화면.
3. **앱 기동** → 셸이 데몬을 띄운다 → 데몬 로그(`.engram-dev\logs\daemon-*-<pid>.log` — 데몬 기본 수준이 warn 이라 찍힌다)에 「제어 평면 CLI 형제 exe 를 못 찾음 — CLI 입구 비활성」(데몬 `lib.rs:114`)이 없다.
4. **스폰한 에이전트의 자격으로 CLI 를 부른다** — qa 바인딩 F 절 그대로(터미널 모드 claude 를 프롬프트 없이 띄우고 그 화신의 MCP 설정 파일에서 토큰 · URL 을 빌린다) → `<target>\engram.exe agent list` → 응답 JSON · exit 0 · 그 에이전트가 목록에 있다. (선택) 같은 꼴로 `restore.status`.
5. **정리** — 그 에이전트를 끄고 프로필을 지운다(F 절 4).

### 5-2. U1 뒤

GUI 는 필수가 아니다 — 데이터 루트 디버그 분기는 조인 시험이 덮는다(§3-2). (권장) 앱 기동 한 번으로 `daemon.json` 이 `<저장소>\.engram-dev\daemon\run\` 에 그대로 생기는지 — U2 의 5-1 과 묶어도 된다.

### 5-3. ★U2 — 릴리스 스크립트 1회 (master 머지 전 필수 · F2)★

`pwsh scripts/build-release.ps1 -OutDir <모든 체크아웃 밖의 새 폴더>`(예: `%TEMP%\engram-release-check-<시각>` · 두 `Invoke-Step` 이 된 빌드 단계 그대로 · 수 분 — 분리 실행) → 스크립트 PASS · 폴더에 exe 셋 + `prompts/` · 두 빌드 단계가 각자의 줄로 찍힌다 → 그 폴더의 `engram.exe help` 가 stderr 없이 답한다. ★CLI 의 exe 폴더 폴백 갈래는 시험이 못 탄다(늘 저장소 안에서 돈다)★ — 배포 폴더에서 설치 위치가 exe 폴더로 떨어지는 것을 재는 유일한 자리다. 앱은 띄우지 않아도 된다(데이터 폴더를 안 건드린다).

- **★`-OutDir` 는 모든 체크아웃 밖이어야 한다(리뷰 2라운드 3)★** — 걸음은 빌드 모드와 무관하게 늘 걷는다(§2-4). 폴더가 어느 체크아웃 아래 있으면 걸음이 그 `.git` 을 만나 설치 위치 = 체크아웃 루트가 되고, help 는 배포 폴더가 아니라 그 체크아웃의 `prompts/` 를 읽는다 — 폴백 갈래를 한 번도 안 타고 초록이 된다. 먼저 확인한다: 그 폴더와 모든 조상에 `.git` 도 `[workspace]` 를 담은 `Cargo.toml` 도 없다(사용자 임시 폴더가 체크아웃 아래면 다른 자리를 고른다). 끝나면 그 폴더를 지운다(사용자 확인 없이 지워도 되는 것은 이 측정이 만든 폴더뿐이다).
- **`launch\빌드.bat` 로 재지 않는다** — 그 런처는 `-OutDir "%~dp0release"`(`launch/빌드.bat:4`) = `<저장소>\launch\release` 로 조립한다. 체크아웃 안이라 위 이유로 폴백을 못 잰다. 같은 사실 때문에 `build-release.ps1:16-17` 의 주석 「릴리즈 폴더엔 .git·[workspace] 마커가 없으므로 install_root = exe 디렉토리」는 그 폴더에서 거짓이다(그 폴더에서 띄운 릴리스 앱의 프라이밍 · help 는 저장소의 `prompts/` 를 읽는다 — 2-3 이 만든 동작이 아니라 지금도 그렇다 · 동작은 이 단계가 바꾸지 않는다). U2 가 그 주석을 고친다(같은 파일을 이미 고친다 — §6 2). CI 는 이 스크립트를 태그 push 때만 돌리고 태그는 다시 쓸 수 없으므로(`ci.yml:1281-1283` · CLAUDE.md 「태그」) 여기서 먼저 재지 않으면 처음 깨지는 자리가 배포다.

---

## 6. 문서 후속 (이 TRD 는 고치지 않는다 — 각 단위가)

1. **U1** — base `src/lib.rs:6` · `path.rs` 모듈 머리(표지 계약은 §2-4 의 일반 서술) · CLAUDE.md `:158`(base 항목의 `path` 서술) · `docs/testing-strategy.md:43` · 데몬 `data_dir.rs` 머리와 `find_install_root` doc(규칙 = base `checkout_root_or_exe_dir`) · 셸 `layout.rs` 머리 · `:99` · `:204-207`.
2. **U2** — cli `Cargo.toml` 주석(데몬 `:33-36` 을 옮긴다) · `engram.rs:337` 앵커 줄 · `roundtrip_smoke.rs:66` · `:424` · `build-release.ps1:11` · `:16-17`(「릴리즈 폴더엔 마커가 없으므로」 → 「체크아웃 밖에 조립한 릴리즈 폴더면 마커가 없어 install_root = exe 디렉토리 · `launch\release` 처럼 체크아웃 안이면 체크아웃 루트다」 — §5-3) · `:127-132` · `rebuild-run-release.bat:5-8` · `scripts/README.md:12`(「데몬까지 재빌드」 → 「데몬 · CLI 까지」) · 루트 `Cargo.toml` 머리(멤버 수 · cli 한 줄).
3. **U3** — CLAUDE.md(§2-6 7 · 8) · `docs/testing-strategy.md`(데몬 절 `:93` · `:95` · `:159` 에서 `engram_cli` 를 빼고 — 데몬의 `--test-threads=4` 근거는 「기본 실행의 실 PTY 셸 스폰 + `#[ignore]` 실 데몬 exe」로(F4) — 새 cli 절: ① bin 단위 ② 프로세스 레벨 `engram_cli`(실 exe) · 실행 `cargo test -p engram-dashboard-cli -- --test-threads=4`) · `docs/reference/architecture-overview.md:169`(실행 산출 = 데몬 exe + cli 패키지의 `engram`) · `:173-197`(그래프에 cli 노드 · → agent · base · command) · `:384` · 짧은 꼴 포인터 전부(F6 — agent `src/types.rs:474` · `:558` · 데몬 `control/catalog.rs:421` · `control/mcp_server.rs:208` · `:1636` · `:1697` · `control/priming.rs:705` → `engram-dashboard-cli/src/bin/engram.rs` · `engram-dashboard-cli/tests/engram_cli.rs` 꼴로) · `control/priming.rs:234`(「같은 exe-walk-up 패턴」 — 규칙 = base `checkout_root_or_exe_dir`) · `ci.yml:33` · `:520-523` · `:1355-1420` · platform `src/lib.rs:166-180`.
4. **오케스트레이터** — step-log 착지 항목 · 메모 §8 「사실」 첫 줄 · §10 2-3 착지 표시 · 옛 ADR 문장은 본문을 고치지 않고 ADR-0284 가 링크로 적는다(§7 2).
5. **날짜 박힌 스냅숏** — `docs/reference/architecture-map-notes.md` · `architecture-map-data/*.json` 은 그 지도를 다시 뽑을 때.

---

## 7. ADR-0284 에 박을 것

> 채번 · 링크 · 도장 = `/adr`(번호는 커밋 직전에 다시 본다). 거부한 대안은 메인 · ADR · 사용자가 준 것만 옮긴다(CLAUDE.md 「결정 날조 금지」). D4 의 근거는 사용자 답이 나온 뒤 채운다.

1. **패키지 모양(D1 · D2 · D3)** — 이름 `engram-dashboard-cli` · bin 전용 · 파일 자리 그대로 · 의존 목록. 거부한 대안 = `-engram` 이름 · `src/main.rs` · lib + bin(§2-1).
2. **D4 의 결과와 고쳐지는 옛 문장(F5).** A′ 면: 걸음 = base `find_checkout_root` · 설치 위치 = base `checkout_root_or_exe_dir`(데몬 · CLI 가 부른다) · 데이터 루트는 걸음만 쓴다(이유 = §2-4) · 입주 판정 ①②③ · ADR-0269 「입주시키지 않는 것」 · ADR-0264 「거부한 대안」과의 경계. A 면 위에서 `checkout_root_or_exe_dir` 를 빼고 「CLI 폴백 한 줄은 시험으로 묶지 않는다」를 더한다. B 면: 같은 경로 시험의 자리(CLI bin 단위 · CLI → (dev) 데몬) · 범위 · 빈틈 · 2-4 와의 관계. 어느 쪽이든 거부한 둘을 그 이유와 함께. **A′ · A 일 때 고쳐지는 문장(본문은 고치지 않고 링크 · 도장 = `/adr`):**
   - ADR-0273 **결정 5** 「설치 위치 규칙은 CLI 쪽에 한 벌 더 · 같은 경로 시험으로 묶는다」 → **뒤집힌다**: 사본을 두지 않고 base 한 곳을 부른다. 이유 = 걸음이 이미 두 벌이라 사본이면 셋째 · CLI → 데몬 간선을 남기지 않는다.
   - ADR-0271 **결정 2** 「데몬으로: … 설치 위치(`find_install_root`) …」 → A′: 이름(`find_install_root`)은 데몬에 남되 규칙은 base(`checkout_root_or_exe_dir` — 데몬 함수는 자기 exe 를 넘기는 한 줄) · A: 걸음만 base. 아래 ADR-0282 「영향」 항목과 같은 말로 적는다(같은 결정 2 의 실행 파일 위치 자리는 ADR-0282 가 이미 고쳤다).
   - ADR-0271 **「영향」** 「CLI 쪽 사본과 같은 경로 시험은 ADR-0273 이 진다」 → 같은 이유로 **뒤집힌다**(그 짐이 없어진다).
   - ADR-0271 **「거부한 대안」 첫 항목** 「ADR-0273 뒤에는 설치 위치를 자기 사본으로 가져(그 결정 5) 이쪽 코드를 아예 쓰지 않는다」 → 앞 절(「자기 사본」)이 틀리게 된다 · 결론(나눈 뒤 지킬 대상의 판단)은 ADR-0282 결정 6 이 이미 고쳤다 — 이 단계는 「사본」 낱말만 바로잡는다.
   - ADR-0282 **결정 3** 의 「못 재는 것 — walk-up 표지 하나를 빠뜨린 셸 사본」 → **대상째 사라진다**(걸음이 한 벌) · 같은 결정의 나머지(루트 · `daemon.json` · `logs\` 의 같은 경로 시험)는 그대로.
   - ADR-0282 **「영향」** 「`find_install_root`(…)는 데몬 `data_dir` 에 그대로다」 → A′: 이름은 데몬에 남되 규칙은 base(데몬 함수는 `current_exe` 를 넘기는 한 줄) · A: 걸음만 base. 「루트 규칙 정본 = 데몬 `data_dir.rs`」 → **좁혀진다**: 데이터 루트 규칙(환경변수 · 분기 · 폴더 이름)의 정본은 그대로 데몬이고, 그 안의 걸음 부품만 base 다. 같은 「영향」 끝의 「2-3 이 이 간선을 CLI 쪽 사본으로 바꾼다」 → 「base 로 바꾼다」.
   - **메모 §5 원칙 · ADR-0271 「거부한 대안」 셋째(「경로 규칙만 담는 작은 공용 crate」)와의 긴장** → **뒤집지 않는다**: 새 crate 를 세우지 않고, 데이터 루트 규칙은 데몬에 남는다. base 로 가는 것은 Engram 고유 지식이 0 인 범용 부품이고, 그 판정 규칙은 ADR-0269 결정 7(지금 여러 벌이면 base)이다 — 2-2 의 쓰기 프로브(ADR-0282 결정 2)와 같은 판단. 이것을 ADR 본문에 명시한다(다음 세션이 「공용으로 두지 말라」를 근거로 되돌리지 않게).
3. **빌드 입구(D6)** — 릴리스 두 호출 · 각자의 `Invoke-Step`(한 블록이면 앞 실패가 묻힌다) · 비용(serde_json 기능 집합 차 → agent · command 재빌드) · 런처 둘 · `roundtrip_smoke.rs` 안내문 · 릴리스 스크립트 1회 = 머지 전 필수 · 거부한 대안 = 한 호출(동작 미검 · 기능 합집합) · 바인딩은 사용자 승인으로.
4. **게이트(D7)** — 의존 상한은 이름 집합 일치(수 세기를 버린 이유) · test-support 고리 셋 · 게이트 ⑦ 의 cli 줄 · 버전 게이트 편입(라이브러리 기준과의 구별 · 실패 비용 = 버려지는 태그) · 단독 기립 스텝(타깃 부재를 실패로).
5. **「새 crate 는 이름 알파벳에 더한다」의 bin 전용 예외(F8)** — 메시징 정규식(ci.yml `:520-523` · qa `:253` · CLAUDE.md `:255`)에 cli 를 더하지 않는다 · 이유 = lib 타깃이 없어 부를 수 있는 crate 가 아니다 · lib 를 세우는 날 더한다 · 세 자리의 문구는 §2-6 8. **기각 근거 자평: 보통** — 더해도 해는 없고 규칙이 단순해지는 이점이 있다. 예외를 택한 것은 「이 멤버를 부를 수 있다」는 오독을 알파벳이 만들지 않게 하려는 메인 판단이다.
6. **옛 포인터 읽기 규칙(D9 — 짧게 · 찾는 법과 함께):** ① `crates/engram-dashboard-daemon/src/bin/engram.rs` → `crates/engram-dashboard-cli/src/bin/engram.rs`(ADR-0273 「영향」이 이미 적었다 — 확인만) ② `crates/engram-dashboard-daemon/tests/engram_cli.rs` → `crates/engram-dashboard-cli/tests/engram_cli.rs`(`git grep -l engram_cli -- docs/decisions`) ③ 데몬 `Cargo.toml` 의 `[[bin]] engram` → cli `Cargo.toml` 의 `[[bin]]` ④ (A′ · A) 데몬 `data_dir` 의 `find_workspace_root` · `is_workspace_root` → base `path` 의 `find_checkout_root` · `is_checkout_root`(A′ 는 `find_install_root` 의 규칙도 `checkout_root_or_exe_dir`).
7. **착지 실측** — 회귀 수 · GUI · 릴리스 1회(착지 때 채운다).
- **코드 앵커** — `# ADR-0273`: cli `Cargo.toml` 의 `[[bin]]`(ADR-0273 「영향」이 정했다). `// ADR-0284`: base `path` 의 `find_checkout_root` · `checkout_root_or_exe_dir`(A′) · `engram.rs` 의 `install_root`(A′ · A) 또는 같은 경로 시험(B) · 데이터 루트 조인 시험 · `ci.yml` 새 스텝 둘. `// ADR-0273`: `engram.rs` 의 설치 위치 줄(그대로 남기고 문구만 고친다).

---

## 8. 열린 것

**사용자에게 올릴 것은 O1 하나다.** O2 는 O1 이 A′ · A 일 때만 열린다(A′ ↔ A 의 갈림). O3 · O8 은 U2 QA 전에 받을 확인이다. 나머지는 메인 결정이고 리뷰가 다르게 보면 바꾼다.

- **O1 — D4 사용자 선택(A′ / A / B).** 권고 A′(§2-4). A′ · A 는 2-2 의 데몬 · 셸 착지 코드를 다시 열고 ADR 문장 여럿을 고친다(§7 2). B 는 걸음이 세 벌이 되고 CLI 시험 빌드가 데몬을 다시 짓는다.
- **O2 — A′ 의 폴백이 입주 조건 ② 를 지키나(리뷰 판단).** 「체크아웃 밖이면 exe 폴더」를 범용으로 봤다(§2-4 A′ 평가 2). 리뷰가 그것을 배포 배치(ADR-0100 동거) 지식으로 보면 A 로 내린다 — 그때 묶이지 않은 폴백 한 줄이 남고 §5-3 이 그것을 덮는다.
- **O3 — qa 바인딩 편집 승인.** 빌드 명령 셋(`:320` · `:355` · `:360`)은 U2 의 QA 전에 들어가야 한다(안 그러면 GUI 가 옛 engram.exe 를 잰다). 나머지(`:79-84` · `:138` · `:253` · `:254-275` · `:442` · `:445`)는 U3.
- **O4 — 버전 = 제품 버전 + 버전 게이트 편입(메인 결정).** 대안 = `0.1.0` 으로 두고 게이트에서 뺀다(라이브러리처럼). 단독 배송물이 있다는 게이트 자신의 기준으로 편입했다. 대가 = 릴리스마다 매니페스트 하나가 늘고, 빠뜨리면 실패는 태그를 push 한 뒤에야 드러나 그 태그를 버린다(태그는 다시 쓸 수 없다). 막는 것 = U3 수용 기준 1 의 로컬 1회와 릴리스 절차의 버전 올림 커밋.
- **O5 — 모듈 = `path`(메인 결정, A′ · A 일 때).** 대안 = 새 모듈(`checkout` 등 — 게이트 ③ 알파벳 사본 여섯을 함께 고친다).
- **O6 — 의존 상한 판정 = 이름 집합(메인 결정).** 기존 net 게이트 3 의 꼴(줄 수)과 갈린다 — 줄 수로는 데몬이 들어온 것을 못 본다.
- **O7 — CLI 의 async 0 을 platform 게이트 ⑦ 에 한 줄로 둔다(메인 결정).** 대안 = 따로 `cli gate 2`. 같은 함수 · 같은 판정이라 한 스텝에 둔다 — 그 대신 platform 헤더의 「대상」 서술이 platform 소비자가 아닌 것(CLI 자신의 직접 서드파티)까지 품는다.
- **O8 — GUI · 릴리스 실측의 확인**(2-2 TRD §8 O3 와 같다 · §5-0) — U2 의 §5-1 3 과 §5-3 앞에서 묻는다.
- **O9 — ADR 번호.** 0284 는 2026-10-08 기준 다음 빈 번호다(0283 은 master · storage 브랜치에 있다). 다른 작업이 먼저 쓰면 밀린다 — 커밋 직전 재확인.
- **O10 — 릴리스 두 호출의 시간(메인 결정 = 두 호출).** serde_json 기능 집합이 갈려 agent · command 가 다시 지어진다(C3). 릴리스 시간이 문제가 되면 한 호출로 합치는 쪽(CLI 가 데몬 기능 집합으로 지어진다 · `--bin` 동작 확인 필요)을 다시 본다.

---

## 9. 미검

- **기준선 회귀 수** — 이 판에서 돌리지 않았다(§3-6 · U1 착수 직전).
- **CLI 빌드 그래프 약 92** — agent · command · base 의 그래프를 따로 잰 합집합 추정이다(새 패키지가 없다).
- **Cargo.lock 변화 = 멤버 자신 하나 · 서드파티 0** — 쓰는 서드파티가 lock 에 있다는 독해이고 돌려 보지 않았다.
- **bin 전용 패키지에 Doc-tests 줄이 없다** — cargo 동작 지식(가능성 높음). 틀리면 결과 줄이 +1 이다(회귀가 아니다).
- **target 폴더 공유로 형제 관계가 그대로** — 워크스페이스 기본 동작(확실에 가깝다 · U2 의 §5-1 3 이 잰다).
- **한 호출 `-p A -p B --bin x --bin y` 의 동작 · 두 호출의 릴리스 시간 증가폭** — 안 돌렸다(두 호출로 피한다 · O10).
- **`-p engram-dashboard-cli` 단독 빌드가 기능 합집합 없이 서는지** — U2 수용 기준 · CI 단독 기립 스텝이 잰다.
- **셸 디버그 전용 base 호출의 릴리스 경고** — U1 의 `cargo check --release` 가 잰다.
- **조인 시험의 기대 루트(`CARGO_MANIFEST_DIR` 위) == 걸음 결과** — 기본 target 폴더 · 워크트리(`.git` 파일) 둘 다에서 같다는 것은 코드 읽기다(U1 이 잰다 · 체크아웃 밖 target 은 로컬이면 건너뜀 · `CI` 면 실패). `dunce::canonicalize` 가 두 경로를 같은 철자(대소문자 포함)로 고른다는 것도 그 crate 동작 지식이다(파일시스템이 돌려주는 실제 철자 — U1 이 로컬 · CI 에서 잰다). GitHub Actions 러너가 `CI=true` 를 둔다는 것은 GitHub 문서의 기본 환경변수다(확실 · 이 저장소에서 따로 잰 적은 없다).
- **데몬 dev `dunce` 한 줄의 lock 변화** — 데몬 항목의 의존 목록만 바뀌고 새 패키지 0(dunce 는 agent · 셸이 이미 문다 — 독해).
- **`launcherWiring.test.ts` 정규식이 `-cli` 를 형제로 본다** — 코드 읽기(U2 의 `npm test` 가 잰다).
- **CLI 그래프에서 `-i engram-dashboard-base` · `-i engram-dashboard-platform` 이 rc 0** — 그래프 논리(U3 실측).
- **`include_str!` · `help_repo_root` 깊이** — 코드 읽기(U2 컴파일 · 시험이 잰다).
- **GUI 전 단계 · 릴리스 1회**(§5).
- **사용자가 체감하는 동작 변화 = 없음(코드 읽기)** — `engram.exe` 의 이름 · 자리(개발 = `target\<profile>\` · 배포 = 릴리스 폴더) · CLI 표면 · help 본문을 찾는 규칙(같은 걸음 · 같은 exe 폴더 폴백) · 배포 zip 의 파일 목록이 그대로다. 바뀌는 것은 개발자 쪽 둘이다 — ① `cargo build -p engram-dashboard-daemon` 만으로는 engram.exe 가 더 지어지지 않는다(그래서 D6) ② `roundtrip-smoke` 하네스가 `--disallow-mcp` 에서 CLI 가 없을 때 찍는 SETUP-SKIP 안내문의 빌드 명령이 바뀐다(`roundtrip_smoke.rs:424` · F3).
