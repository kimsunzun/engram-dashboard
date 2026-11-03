# qa 바인딩 — engram-dashboard

- 이 파일이 이 프로젝트 qa 의 규칙과 명령의 정본이다. 게이트 명단의 정본은 `.github/workflows/ci.yml` 이다.
- 전체 회귀 절차 · GUI 실측 조리법 · 창 조작은 `.claude/skill-bindings/qa-verify.md` 에 있다 — 그 일을 할 때만 연다.
- ★이 파일은 CI 시험 입력이다★ — `src/util/launcherWiring.test.ts` 가 코드 펜스 안 줄을 읽는다. 펜스 안에 `build-client-shell.mjs` 호출을 남기고, 셸 패키지를 `cargo build -p` 로 직접 짓는 줄을 넣지 않는다.

## 돌릴 것 표

메인이 바뀐 파일마다 행을 찾는다. 여러 행이 걸리면 큰 쪽을 쓴다.

| 변경 | ① 자동 테스트 | ② 실측 |
|---|---|---|
| 문서만 — `docs/**` · 루트 `*.md` · `.claude/**`(`qa.md` 제외) | 없음 | 끔 |
| 주석만 바뀐 소스(아래 「주석만 판정」) | 없음 | 끔 |
| `.claude/skill-bindings/qa.md` | 전체 회귀 | 끔 |
| 시험 파일만 — `*.test.ts(x)` · `crates/*/tests/**` · `src-tauri/tests/**` | 전체 회귀 | 끔 |
| Rust 소스 · `Cargo.toml` · `Cargo.lock` | 전체 회귀 | 끔 |
| 프론트 — `src/**` · `public/**` · `index.html` · `package*.json` · `vite.config.*` · `src-tauri/tauri.conf.json` | 전체 회귀 | 켬 · GUI |
| Tauri command · IPC 응답 형식이 바뀌는 셸 변경 | 전체 회귀 | 켬 · GUI |
| 에이전트 수명 · 출력 경로(띄우기 · 끄기 · 출력 펌프 · 이벤트 버스 · transport · epoch · replay→live) | 전체 회귀 | 켬 · GUI |
| claude · codex 레인 경로(「로컬 레인」) | 전체 회귀 + 그 레인 | 행대로 |
| `src-tauri/src/discovery/**` · `src-tauri/src/daemon_client/stop.rs` | 전체 회귀 + 셸 실 데몬 레인 | 행대로 |
| 그 밖 · 판정 불가 | 전체 회귀 | 끔 |

- 문서 · 바인딩 diff 가 명령을 바꿨으면 바뀐 명령만 문자 그대로 한 번 돌려 본다.
- 주석만 판정 — 아래 두 줄이 다 0줄이고 새 미추적 `.rs` · `.ts` · `.tsx` 가 없으면 주석만이다:
  ```bash
  git diff -U0 <기준점> -- '*.rs' | grep -E '^[+-]' | grep -vE '^(\+\+\+|---)' | grep -vE '^[+-][[:space:]]*(//|/\*|$)'
  git diff -U0 <기준점> -- '*.ts' '*.tsx' | grep -E '^[+-]' | grep -vE '^(\+\+\+|---)' | grep -vE '^[+-][[:space:]]*(//|/\*|\*|$)'
  ```
- `.rs` 에서 `*` 로 시작하는 줄은 주석이 아니다 — 역참조 대입(`*x.lock().unwrap() = …`)이다.
- 바뀐 파일에 `ts(export` 가 있으면 주석만이어도 「Rust 소스」 행이다 — 그 doc 주석이 생성물(`bindings/*.ts`)로 흘러 동기 게이트가 깨진다.
- 범위 정의:
  - **전체 회귀** = CI 세 잡(`backend` · `gates` · `frontend`)을 로컬에서 도는 것. 절차 = qa-verify 「전체 회귀 — CI 세 잡 로컬 실행」.
  - **영향 모듈** = 아래 「모듈별 명령」의 그 모듈 줄. 변이 확인 · 재현 명령 · 작은 수정 확인에 쓴다.
- 로컬 순서 = 전체 회귀 → (깨지면) 깨진 테스트 가르기 → 로컬 레인 → (레인이 깨지면) 그 실패 가르기 → 변이 확인 → GUI 실측. 로컬 단계끼리 병렬로 돌리지 않는다.
- 가르기 뒤 라벨 없는 실패가 남으면 거기서 멈춘다 — 토큰 레인(claude · codex)을 돌지 않는다.

## 전체 회귀 = 로컬

- qa 는 작업 트리를 잰다 — 커밋 · push 를 기다리지 않는다. 메인은 qa 뒤에 커밋한다.
- qa 는 시작할 때 잰 트리를 지문으로 적어 보고에 싣는다 — 인덱스 사본에 작업 트리를 통째로 올린 트리 id 와 HEAD 다(진짜 인덱스는 건드리지 않는다):
  ```bash
  cp "$(git rev-parse --git-path index)" "$QA_TMP/fp.idx"; GIT_INDEX_FILE="$QA_TMP/fp.idx" git add -A 2>/dev/null
  echo "HEAD=$(git rev-parse HEAD) TREE=$(GIT_INDEX_FILE="$QA_TMP/fp.idx" git write-tree 2>/dev/null)"
  ```
- 메인은 커밋할 것을 스테이징한 뒤 `git rev-parse HEAD` · `git write-tree` 를 그 두 값과 대조한다 — 다르면 qa 를 다시 돌리거나 잰 트리 그대로 커밋한다.
- CI 결론은 qa 판정에 넣지 않는다 — CI 는 push 뒤 통합 확인이다(`CLAUDE.md` 「브랜치·커밋」 · 읽는 법은 `docs/handbook/branch-commit.md`).
- CI 가 못 도는 것 = GUI 실측 · 실 claude 시험(CI `--skip` 목록) · `#[ignore]` 시험 → 「로컬 레인」 · 「GUI 실측」.
- 로컬과 CI 명령줄의 차이는 의도다 — 맞추지 않는다: CI 만 `--locked` · CI 만 실 claude 시험 `--skip` · 로컬만 `-- --test-threads=4` · 로컬만 분리 실행.

## 로컬 명령

### 임시 폴더

```bash
QA_TMP="$(cygpath -m "$TEMP")/engram-qa-$(date +%Y%m%d-%H%M%S)"; mkdir -p "$QA_TMP"; echo "$QA_TMP"
```

- qa 가 만드는 로그 · 스크린샷 · 사본 · 데이터 폴더는 전부 `$QA_TMP` 아래 둔다. Bash 호출이 바뀌면 처음 찍힌 경로를 그대로 다시 쓴다.

### 분리 실행

빌드 · 테스트는 전부 이 래퍼로 돈다:

```bash
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-detached.ps1 -Command "<명령 한 줄>" -WorkDir "<워크트리 루트>" -LogFile "$QA_TMP/<이름>.log"
```

- 돌아온 `PID=` · `LOG=` · `BAT=` 은 시작했다는 뜻이다. `BAT=` 경로를 적어 둔다(뒷정리).
- 완료 = 로그 마지막 줄 `__EXIT=<종료코드>`. 마커가 없으면 아직 안 끝났거나 실행 실패다 — 빈 로그 · 마커 없음은 PASS 가 아니다.
- `-WorkDir` · `-LogFile` 은 슬래시 경로로 준다 — bash 에서 `\` 뒤의 `$` 는 이스케이프되어 경로가 깨진다.
- 감싸지 않는 것: `rg` · `cargo tree … | rg` 게이트(래퍼의 cmd 에는 `rg` 가 없다) · `%` 가 든 명령(배치가 `%` 를 먹는다).
- 한글 인자는 래퍼를 거치면 `?` 로 깨진다(배치를 ASCII 로 쓴다) — vitest `-t <한글 이름>` 대신 파일 단위로 돌리고 출력에서 이름을 찾는다.
- `call` 을 손으로 붙이지 않는다 — 래퍼가 배치 대상에만 붙인다.
- Git Bash 에서 `/` 로 시작하는 인자(`taskkill /PID` 등) · `/` 가 든 `gh -q` 식 · `<ref>:<경로>` 인자는 앞에 `MSYS_NO_PATHCONV=1` 을 붙인다.

### Rust 테스트 판정

- PASS = `__EXIT=0` + 아래가 0 으로 끝남(통과 수 > 0):
  ```bash
  grep -oE 'test result: [a-zA-Z]+\. [0-9]+ passed' "$QA_TMP/<이름>.log" | awk '{s+=$4} END {print "passed=" s+0; exit !(s>0)}'
  ```
- `--exact` 로 이름을 집은 줄은 통과 수가 집은 이름 수와 같아야 PASS 다.
- 실패가 나면 로그의 `failures:` 이름 목록을 그대로 옮긴다.
- 타이밍 · flaky 실패는 다시 돌려 통과시키지 않는다 — FAIL 로 보고하고 ADR-0038(`docs/reference/debugging-conventions.md`)을 단다.

### 모듈별 명령

| 모듈 | 명령 |
|---|---|
| base · command · protocol · messaging | `cargo test -p engram-dashboard-<이름>` |
| platform · agent · daemon | `cargo test -p engram-dashboard-<이름> -- --test-threads=4` |
| cli | `cargo test -p engram-dashboard-cli --bins --test engram_cli -- --test-threads=4` |
| net | `cargo test -p engram-dashboard-net` 와 그 줄에 `--all-features` 를 붙인 줄 — 둘 다 |
| transport | `cargo test -p engram-dashboard-transport` · 그 줄에 `--all-features` · `cargo check -p engram-dashboard-transport --features ws` — 셋 다 |
| 셸(`src-tauri`) | `cargo test -p engram-dashboard --test <타깃>` — 타깃 = `layout_apply` · `layout_commands` · `daemon_client_pending` · `daemon_client_replay` · `lib_unit` |
| 프론트 | `npm test` · `npx tsc --noEmit` |
| 전체(로컬) | `cargo test --workspace -- --test-threads=4` |
| 작은 수정 확인 | `cargo build` · `npx tsc --noEmit` |

- `-- --test-threads=4` 는 실 자식 프로세스를 띄우는 시험이 있는 crate(platform · agent · daemon · cli)와 워크스페이스 전체에만 붙인다.
- `npx tsc --noEmit` 은 통과하면 아무것도 안 찍는다 — 판정은 `__EXIT=0` 으로만 한다.
- `cargo build` · `cargo test` 가 `os error 5` 로 죽으면 떠 있는 앱 · 데몬이 `target` 의 exe 를 잡은 것이다 — 그 프로세스를 끄지 않는다. `--target-dir "$QA_TMP/target"` 를 붙여 다시 돈다(분리 실행은 환경변수를 넘기지 못한다). 그 빌드의 exe 는 `$QA_TMP/target/<debug|release>/` 에 생긴다.
- 따뜻한 빌드를 다시 쓰는 자리는 이 워크트리의 `target/`(기본 — `--target-dir` 없음)뿐이다. 다른 워크트리의 `target/` 을 `--target-dir` 로 주지 않는다 — 그 트리의 `target/debug/*.exe` 를 덮는다.
- 게이트는 초록인데 앱 빌드 · 기동만 죽으면(예: `failed to read plugin permissions … (os error 3)`) diff 를 의심하기 전에 `target/` 에 다른 루트의 산출물이 섞였나 본다. 아래가 찍는 폴더가 그것이다 — 지우지 않고 그 패키지 이름과 처방 `cargo clean -p <패키지>` 를 보고한다:
  ```bash
  grep -LF "$(cygpath -w "$PWD")\\target\\" target/*/build/*/root-output
  ```

## 로컬 레인 — CI 가 못 도는 시험

### claude · codex 레인 (비용 있음 — 토큰 · 사용 한도)

- 켜는 조건은 둘 중 하나다: ① 바뀐 파일이 그 레인 경로에 걸린다 ② 설치된 CLI 버전이 기록 파일의 그 레인 마지막 PASS 버전과 다르다(기록 없음 = 다르다).
- 버전 = `claude --version` · `codex --version`(토큰 안 씀).
- 기록 파일 = `$LOCALAPPDATA/engram-qa/lanes.tsv`(PC 마다 · 저장소 밖). 줄 = `<레인>` · `<CLI 버전>` · `<SHA>` 탭 구분. `<SHA>` = 그 실행이 잰 기준 HEAD — 작업 트리에 커밋 안 된 변경이 있었으면 뒤에 ` +uncommitted` 를 붙인다. 레인이 PASS 면 그 레인 줄을 갈아 쓰고, FAIL 이면 그대로 둔다.
- 안 켠 레인은 「해당 없음 — 경로 무관 · 버전 같음(<버전>)」으로 적는다.
- CLI 없음 · 미로그인은 FAIL 이다.

| 레인 | 경로 |
|---|---|
| claude | `crates/engram-dashboard-agent/src/backend/claude/**` · `crates/engram-dashboard-agent/src/backend/mod.rs` · `crates/engram-dashboard-agent/src/usage/**` · `crates/engram-dashboard-daemon/src/control/**` · `crates/engram-dashboard-messaging/src/**` · `crates/engram-dashboard-daemon/tests/control_send.rs` · `crates/engram-dashboard-agent/tests/usage_probe_smoke.rs` |
| codex | `crates/engram-dashboard-agent/src/backend/codex/**` · `crates/engram-dashboard-agent/src/backend/mod.rs` · `crates/engram-dashboard-agent/src/usage/**` · `crates/engram-dashboard-agent/tests/backend_contract.rs` · `crates/engram-dashboard-agent/tests/usage_probe_smoke.rs` |

claude 레인 — 첫 줄의 이름 = CI `--skip` 목록에서 `#[ignore]` 가 아닌 것(목록이 바뀌면 여기도 고친다):

```bash
cargo test -p engram-dashboard-daemon --test control_send -- --test-threads=4 --exact c2_live_mid_turn_send_parks_and_delivers_after_turn_end control_send_delivery_observation_records_bytes_and_correlated_ids control_send_relays_wrapped_line_to_json_agent control_send_revoked_sender_still_delivers_observation mcp_send_message_tool_happy_and_error
cargo test -p engram-dashboard-agent --test usage_probe_smoke -- --ignored --test-threads=1 claude_
```

codex 레인:

```bash
cargo test -p engram-dashboard-agent --test usage_probe_smoke -- --ignored --test-threads=1 codex_
cargo test -p engram-dashboard-agent --test backend_contract -- --ignored --test-threads=4
```

### 비용 없는 `#[ignore]` 레인

| 레인 | 켜는 변경 | 명령 |
|---|---|---|
| 셸 실 데몬 | 표의 `discovery` · `stop.rs` 행 | `cargo build -p engram-dashboard-daemon` 뒤 `cargo test -p engram-dashboard --test stop_smoke -- --ignored --test-threads=1` |
| 실 프로세스 Job | `crates/engram-dashboard-platform/src/group/**` · `crates/engram-dashboard-agent/src/reaper.rs` | `cargo test -p engram-dashboard-daemon --test ws_e2e -- --ignored --test-threads=4` · `cargo test -p engram-dashboard-platform -- --ignored msys --test-threads=4`(Git Bash 필요) |

- 이 두 레인에 알려진 실패가 있다 — 「알려진 실패 목록」.
- `lib_unit` 의 실 WMI 시험(`--ignored real_wmi`)은 표가 켜지 않는다 — 운영 데이터 폴더 `.engram-dev` 를 건드린다. 메인이 사용자에게 돌릴지 묻고, 돌리면 앱을 띄우기 전에 `cargo test -p engram-dashboard --test lib_unit -- --ignored real_wmi --test-threads=1`.
- `lib_unit` 에 맨 `--ignored` 를 쓰지 않는다 — 실 WMI 와 골든 다시 뜨기(`regenerate_bands_golden`)가 함께 돈다.

## 알려진 실패 목록

깨진 테스트를 가를 때 이름과 메시지가 둘 다 맞으면 `FAIL(알려진 실패)` 다 — 기준점에서 재현하지 않는다. 메시지가 다르면 목록 밖이다.

- `stop_smoke` `send_stop_makes_real_daemon_self_exit` — `assert_eq` 실패 `left: Timeout` · `right: DaemonClosed`. 기록 = `docs/tracking.md` T-52.
- `ws_e2e` `real_process::ignored_daemon_kill_cleans_pty_child` — panic `… PTY child(cmd.exe)를 OS 트리 열거로 식별하지 못함`(데몬 부팅 자동 복원을 꺼 자식이 안 뜬다). 기록 = `docs/process/step-log.md` 「부팅 자동 resume OFF」.

## 추적 기록 위치

`FAIL(선재)` 는 `docs/tracking.md`(T-번호 항목)와 `docs/process/step-log.md` 에서 시험 이름으로 찾는다 — 전체 경로와 마지막 `::` 조각 둘 다(기록은 짧은 이름을 쓴다):

```bash
N='<시험 이름>'; grep -nF -e "$N" -e "${N##*::}" docs/tracking.md docs/process/step-log.md
```

- 찾으면 파일과 T-번호 · 헤딩을 적는다. 안 나오면 「기록 없음」.

## 기준점 사본 (비용 있음)

「알려진 실패 목록」에 없는 깨진 테스트만 리뷰 기준점에서 다시 돈다:

```bash
git worktree add --detach "$QA_TMP/base" <기준점>
```

- 레인이 환경 탓(CLI 없음 · 미로그인)으로 깨졌으면 가르기 전에 환경 실패로 적는다 — 기준점에서도 깨지므로 `FAIL(선재)` 로 적지 않고 여기서 다시 돌지 않는다.
- 사본 = 기준점 그대로다 — 작업 트리 변경을 얹지 않는다. 변이 확인 사본(`$QA_TMP/wt`)과 따로 두고 빌드 폴더도 따로 쓴다(`$QA_TMP/target-base`).
- Rust: 분리 실행을 `-WorkDir "$QA_TMP/base"` 로, 깨진 줄(「모듈별 명령」 · 레인 줄)에 `--target-dir "$QA_TMP/target-base"` 를 붙이고 `--` 뒤에 `--exact <깨진 이름>…` 을 더한다. 데몬 빌드가 앞에 붙은 레인은 그 빌드도 같은 `--target-dir` 로 짓는다.
- 프론트: 사본에서 분리 실행으로 `npm ci` 뒤 `npx vitest run <깨진 시험 파일>`.
- 라벨 규칙 = qa 골격 `references/flow.md` §2 「깨진 테스트 가르기」. `FAIL(선재)` 는 「추적 기록 위치」로 찾는다.

## 변이 확인 — 격리 사본

```bash
git worktree add --detach "$QA_TMP/wt" HEAD
git -c core.safecrlf=false diff HEAD --binary | git -C "$QA_TMP/wt" apply --allow-empty
git ls-files -o --exclude-standard -z | xargs -0 -r cp --parents -t "$QA_TMP/wt"
```

- 사본 = HEAD 에 작업 트리의 변경(추적 파일 diff · 무시 안 된 미추적 파일)을 얹은 것이다 — qa 는 커밋 전에 돈다.
- `--allow-empty` 는 추적 파일 변경이 없을 때(미추적 추가뿐 · 커밋 뒤 재실행) 빈 diff 를 통과시킨다 — 빼면 종료코드 128 로 죽는다.
- 그 밖에 `apply` 가 실패하면 변이 확인을 멈추고 보고한다 — 변경이 빠진 사본으로 재지 않는다.
- 줄끝만 다른 ` M` 파일은 `git diff` 에 안 나와 사본에 건너가지 않는다 — 내용은 같다. `-c core.safecrlf=false` 는 그 파일마다 찍히는 LF→CRLF 경고만 끈다(오류는 그대로 찍힌다).
- Rust: 사본 줄을 깬 뒤 분리 실행을 `-WorkDir "$QA_TMP/wt"` 로 `cargo test --target-dir "$QA_TMP/target" -p <crate> <시험 이름>` — 실패해야 한다.
- 셸 패키지 시험 중 데몬 exe 를 띄우는 것(`stop_smoke` · `lib_unit` 의 `real_wmi`)은 `<target-dir>/<profile>/` 의 데몬을 쓴다 — 그 앞에 같은 사본 · 같은 `--target-dir` 로 `cargo build -p engram-dashboard-daemon` 을 돈다. daemon crate 시험은 cargo 가 데몬을 같이 짓는다.
- 프론트: 사본에서 `npm ci` 뒤 `npx vitest run <시험 파일>`.
- 사본은 뒷정리 때 통째로 지운다. 작업 트리는 `git status --porcelain` 이 시작 때와 같아야 한다.

## GUI 실측

- 무엇을 어떤 채널로 재나(cdp · 레지스트리 · 탈출구 · 창 조작) = qa-verify 「GUI 조리법」.
- 디버그 셸은 화면을 품지 않는다 — 이 워크트리의 dev 서버(1420)에서 받는다.

### 띄우기 전 점검

```bash
powershell -NoProfile -Command "\$c = Get-NetTCPConnection -LocalPort 1420 -State Listen -ErrorAction SilentlyContinue; if (\$c) { (Get-CimInstance Win32_Process -Filter \"ProcessId=\$(\$c[0].OwningProcess)\").CommandLine }"
powershell -NoProfile -Command "\$c = Get-NetTCPConnection -LocalPort 9223 -State Listen -ErrorAction SilentlyContinue; if (\$c) { (Get-CimInstance Win32_Process -Filter \"ProcessId=\$(\$c[0].OwningProcess)\").CommandLine }"
powershell -NoProfile -Command "Get-CimInstance Win32_Process -Filter \"Name like 'engram-dashboard%'\" | Select-Object ProcessId,ExecutablePath | Format-Table -AutoSize"
WT="$(cygpath -w "$(git rev-parse --show-toplevel)")" powershell -NoProfile -Command "Get-CimInstance Win32_Process -Filter \"Name='node.exe'\" | Where-Object { \$_.CommandLine -and \$_.CommandLine.Replace('/','\').ToLower().Contains((\$env:WT + '\').ToLower()) -and \$_.CommandLine -match 'vite' } | Select-Object ProcessId,CommandLine | Format-List"
```

- 출력으로만 읽는다 — 앞 두 줄은 듣는 쪽이 없으면 아무것도 안 찍고 종료코드 1 로 끝난다. 그 1 은 「비었음」이지 실패가 아니다.
- 넷째 줄이 무엇을 찍으면 이 워크트리의 vite 가 떠 있다 — 포트와 무관하다(`--port` 로 1420 밖에 떠 있어도 잡는다).
- 1420 주인이 다른 워크트리의 vite 면 그 vite 를 쓰지 않고 끄지도 않는다 — 사용자에게 알리고 릴리스 빌드로 간다.
- 9223 을 누가 쥐고 있으면 디버그 포트를 9224 로 바꾸고 `cdp.mjs` 에 `CDP_PORT=9224` 를 준다.
- 디버그 `engram-dashboard.exe` 가 어느 워크트리에서든 떠 있으면 새 디버그 앱이 바로 죽는다(identifier 공유). 릴리스 · 설치본이 떠 있으면 릴리스 기동이 그 창으로 넘어간다. 그 앱을 끄지 않는다 — 사용자에게 알린다.
- 떠 있던 데몬 PID 를 적어 둔다 — qa 전부터 떠 있던 데몬은 건드리지 않는다.

### 빌드 — 이번 변경을 담는다

exe 경로는 빌드가 찍은 것만 쓴다 — 셸에서 경로를 계산하지 않는다(분리 실행은 셸 환경변수를 받지 않아 산출 폴더가 다를 수 있다).

디버그(기본) — 클라이언트 = `scripts/build-client-shell.mjs`(stdout 한 줄 = exe 경로), 데몬 + CLI = 손 명령 한 줄(빌드 전용 스크립트가 없다 · `scripts/rebuild-run-debug.bat` 과 같은 한 호출):

```bash
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-detached.ps1 -Command "node scripts/build-client-shell.mjs" -WorkDir "<워크트리 루트>" -LogFile "$QA_TMP/client.log"
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-detached.ps1 -Command "cargo build -p engram-dashboard-daemon -p engram-dashboard-cli --message-format=json-render-diagnostics" -WorkDir "<워크트리 루트>" -LogFile "$QA_TMP/daemon-build.log"
CLIENT_EXE="$(grep -B1 '^__EXIT=0' "$QA_TMP/client.log" | head -1 | tr -d '\r')"; echo "$CLIENT_EXE"
DAEMON_EXE="$(PYTHONIOENCODING=utf-8 py -c "import sys,json; a=[json.loads(l) for l in open(sys.argv[1],encoding='utf-8',errors='replace') if l.startswith('{')]; print([m['executable'] for m in a if (m.get('executable') or '').endswith('engram-dashboard-daemon.exe')][-1])" "$QA_TMP/daemon-build.log")"; echo "$DAEMON_EXE"
```

- `CLIENT_EXE` = `build-client-shell.mjs` 가 찍은 경로, `DAEMON_EXE` = cargo 가 이번 빌드의 산출물로 찍은 `executable`. 둘 중 하나라도 비면 빌드 실패다 — 여기서 멈춘다.

릴리스(1420 주인이 다른 워크트리의 vite 일 때) — `scripts/build-release.ps1` 이 앱 · 데몬 · CLI 를 따로 짓고 `-OutDir` 에 한 폴더로 조립한다:

```bash
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-detached.ps1 -Command "pwsh -NoProfile -File scripts/build-release.ps1 -OutDir \"$QA_TMP/release\"" -WorkDir "<워크트리 루트>" -LogFile "$QA_TMP/release-build.log"
CLIENT_EXE="$QA_TMP/release/engram-dashboard.exe"; DAEMON_EXE="$QA_TMP/release/engram-dashboard-daemon.exe"
```

- PASS = `__EXIT=0` + 로그에 `[build-release] OK`. 스크립트가 `-OutDir` 를 지우고 다시 조립하므로 두 exe 는 이번 빌드의 것이다.
- `build-release.ps1` 은 `pwsh`(PowerShell 7)로만 돈다 — Windows PowerShell 5.1 은 `-OutDir` 처리에서 죽는다. `-OutDir` 경로는 따옴표째 넘긴다(스크립트가 그 폴더를 지우고 다시 만든다).
- 전제 = cargo-about · `npm ci` 된 `node_modules` — 없어서 죽으면 도구 부재 PARTIAL 이다.

`os error 5` 로 죽으면 조각마다 다르다 — 그 exe 를 쥔 프로세스는 끄지 않는다:

- 디버그 클라이언트 — 이 워크트리의 디버그 앱이 떠 있다. 우회하지 않고(`build-client-shell.mjs` 는 인자를 받지 않는다) 사용자에게 알린다.
- 디버그 데몬 — 떠 있는 디버그 데몬이 exe 를 잡았다. 같은 줄에 `--target-dir "$QA_TMP/target"` 를 붙여 다시 돈다 — `DAEMON_EXE` 는 같은 추출 줄이 새 경로로 준다.
- 릴리스 — 이 워크트리 `target` 의 릴리스 앱 · 데몬 · CLI 중 하나가 떠 있다. 우회하지 않고 사용자에게 알린다.

- 산출물-커밋 대조: 빌드 로그가 다 `__EXIT=0` 이고 `ls -l "$CLIENT_EXE" "$DAEMON_EXE"` 가 두 파일을 다 찍는다. 두 경로는 이번 빌드의 출력에서만 나온다.
- 릴리스 앱은 띄운 뒤 URL 이 `http://tauri.localhost/` 여야 한다 — `localhost:1420` 이면 잘못된 빌드다.

### 띄우기 — 데몬 먼저, 격리 데이터 폴더

```bash
DAEMON_EXE='<빌드가 찍은 DAEMON_EXE>' powershell -NoProfile -Command "& './scripts/launch-detached.ps1' -Exe \$env:DAEMON_EXE -LogFile '$QA_TMP/daemon.log' -EnvVars 'ENGRAM_DATA_DIR=$QA_TMP/data','RUST_LOG=engram_dashboard_agent=debug,engram_dashboard_daemon=debug'"
nohup npm run dev > "$QA_TMP/vite.log" 2>&1 & disown
curl -s -o /dev/null --retry 60 --retry-delay 1 --retry-connrefused --max-time 120 http://localhost:1420
CLIENT_EXE='<빌드가 찍은 CLIENT_EXE>' powershell -NoProfile -Command "& './scripts/launch-detached.ps1' -Exe \$env:CLIENT_EXE -LogFile '$QA_TMP/app.log' -EnvVars 'WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223','ENGRAM_DATA_DIR=$QA_TMP/data','WEBVIEW2_USER_DATA_FOLDER=$QA_TMP/webview'"
curl -s -o /dev/null -w 'cdp=%{http_code}\n' --retry 60 --retry-delay 1 --retry-connrefused --max-time 120 http://127.0.0.1:9223/json/version
for i in $(seq 30); do node scripts/cdp.mjs eval "!!document.querySelector('[data-slot-id]')" 2>/dev/null | grep -qx true && break; sleep 1; done
```

- `npm run dev` · 1420 대기 줄은 디버그 빌드이고 1420 이 비었을 때만 돈다. 이 워크트리의 vite 가 이미 쥐고 있으면 그대로 쓴다. 릴리스 빌드는 두 줄을 건너뛴다.
- 기동 줄의 마지막 두 줄 `LOG=` · `PID=` 를 적어 둔다 — 뒷정리가 PID 를 쓴다. `LAUNCH_FAILED` 는 실측 FAIL 이다.
- 환경변수는 `-EnvVars` 로만 앱에 닿는다(WMI 기동이라 상속되지 않는다). `-Command` 를 `-File` 로 바꾸지 않는다 — 값 여럿이 한 값으로 뭉개진다.
- 앱이 WMI 로 띄우는 데몬은 `ENGRAM_DATA_DIR` 를 받지 않는다 — 데몬을 먼저 띄우지 않거나 그 데몬이 죽으면 기본 폴더(`.engram-dev`)의 자동복원 에이전트가 실 폴더에서 되살아난다.
- 데몬 로그 = `$QA_TMP/data/logs/`.
- 같은 데이터 폴더로 다시 띄우면 main 창이 「복원할까요?」 모달로 막힌다 — `$QA_TMP/data` 이고 이번 qa 가 복원 동작(모달 표시 · 수락 · 거절)을 재는 것이 아닐 때만 `node scripts/cdp.mjs eval "window.__TAURI__.core.invoke('restore_answer', { accept: true })"` 로 답한다. 복원 동작을 재는 qa 면 답하지 않는다. 다른 폴더의 모달에는 답하지 않고 사용자에게 묻는다.

### GUI 뒷정리

```bash
MSYS_NO_PATHCONV=1 taskkill /PID <앱 PID> /T /F
MSYS_NO_PATHCONV=1 taskkill /PID <이번에 띄운 데몬 PID> /F
MSYS_NO_PATHCONV=1 taskkill /PID <이번에 띄운 1420 주인 PID> /F
```

- 앱 → 데몬 → vite 순서다.
- `/T` 는 앱 줄에만 붙인다(앱 + 그 WebView2 렌더러). 데몬 · vite 에는 붙이지 않는다 — `/T` 는 재사용된 부모 번호 아래 남의 프로세스 나무까지 끌 수 있다(`docs/tracking.md` T-54).
- 끄기 전 `ExecutablePath` 가 이번에 띄운 exe 인지 본다 — 이미지 이름만 보고 끄지 않는다.
- 트레이 「완전 종료」(`quit_app`)로 끄지 않는다 — 떠 있던 데몬까지 멈춘다.
- 끈 뒤 남은 손자를 찾는다 — claude · codex 는 `cmd.exe /c <CLI>` 로 떠서 무리에 들기 전에 뜬 손자가 데몬 종료 뒤에도 산다(`docs/tracking.md` T-53):
  ```bash
  QA_TAG="$(basename "$QA_TMP")" powershell -NoProfile -Command "Get-CimInstance Win32_Process | Where-Object { \$_.ProcessId -ne \$PID -and \$_.CommandLine -like \"*\$env:QA_TAG*\" } | Select-Object ProcessId,Name,CommandLine | Format-List"
  ```
- 찍힌 것만 `MSYS_NO_PATHCONV=1 taskkill /PID <PID> /F` 로 하나씩 끈다. 이 조회를 돌린 셸(`bash.exe` · `powershell.exe`)은 명령줄에 경로가 있을 뿐이라 끄지 않는다. 이름만 보고 끄지 않는다.
- 남은 것이 없어진 뒤 「뒷정리」에서 `$QA_TMP` 를 지운다.
- 휴지통 명령이 `$QA_TMP` 에 `FAIL` 을 찍으면(사용 중) 조회에 안 걸린 손자가 남았을 수 있다(codex JSON 모드는 명령줄에 경로가 없다) — 끄지 말고 사용자에게 보고한다.

## 비용 있는 검사 (알리고 진행)

- claude · codex 레인 — 토큰 · 사용 한도.
- GUI 실측 — 클라이언트 · 데몬 빌드 수 분, 릴리스 빌드는 더 길다.
- `--target-dir "$QA_TMP/target"` 우회 · 변이 사본 — 찬 빌드 수 분.
- 기준점 사본 — 찬 빌드 수 분(프론트면 `npm ci` 를 더한다). claude · codex 레인 시험을 다시 돌면 토큰도 든다.

## 뒷정리

```bash
for w in wt base npm; do [ -d "$QA_TMP/$w" ] && git worktree remove --force "$QA_TMP/$w"; done
bash ~/.claude/references/recycle.sh '<분리 실행이 찍은 BAT= 경로>'… "$QA_TMP"
git worktree prune
```

- `rm -rf` 자리는 휴지통 명령으로 한다(`rm -rf` 는 권한이 막는다). 예외는 둘 — 사본을 git 에서 떼는 `git worktree remove --force` 와 아래 `robocopy /MIR` 비우기. 줄마다 `recycled` · `skip (missing)` 이면 된 것이다. 명령이 없거나 막히면 다른 명령으로 돌아가지 않고 남은 경로를 보고한다.
- `BAT=` 경로는 역슬래시 Windows 경로다 — 작은따옴표로 감싼다. 안 감싸면 bash 가 `\` 를 먹어 없는 경로가 되고 `skip (missing)` 이 된 것처럼 보인다.
- `git worktree prune` 이 휴지통으로 간 사본을 git 에서 뗀다 — `git worktree list` 에 `$QA_TMP` 아래가 남지 않아야 한다.
- `git worktree remove` 가 긴 경로로 실패하면 그 사본을 빈 폴더로 미러해 비운 뒤 지운다:
  ```bash
  mkdir -p "$QA_TMP/empty"; MSYS_NO_PATHCONV=1 robocopy "$QA_TMP/empty" "$QA_TMP/<사본>" /MIR /NFL /NDL /NJH /NJS /NP > "$QA_TMP/robocopy.log"; rc=$?; echo "robocopy rc=$rc"; [ "$rc" -lt 8 ] && bash ~/.claude/references/recycle.sh "$QA_TMP" && git worktree prune
  ```
- robocopy 종료코드 0~7 은 성공이다. 8 이상이면 `$QA_TMP` 가 남는다 — `robocopy.log` 를 읽고 보고한다.
- robocopy 출력은 로그로 돌린다 — `/NFL` 을 줘도 지운 파일마다 한 줄씩 찍는다.
- 시험이 다시 구운 생성물은 내용이 같아도 줄끝만 달라 ` M` 으로 남는다 — 대상 = `crates/engram-dashboard-protocol/bindings` · `crates/engram-dashboard-agent/bindings` · `src-tauri/bindings` · `src-tauri/Cargo.toml`. `git update-index --refresh` 로는 안 지워진다.
- 아래가 찍는 파일(` M` 인데 `git diff --numstat` 이 빈 것) 중 시작 때 `git status --porcelain` 에 없던 것만 `git checkout -- <파일>` 로 되돌린다:
  ```bash
  P="crates/engram-dashboard-protocol/bindings crates/engram-dashboard-agent/bindings src-tauri/bindings src-tauri/Cargo.toml"
  comm -23 <(git ls-files -m -- $P 2>/dev/null | sort) <(git diff --numstat -- $P 2>/dev/null | cut -f3 | sort)
  ```
