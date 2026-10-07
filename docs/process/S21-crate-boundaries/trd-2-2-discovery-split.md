# TRD — 경계 리팩터링 2-2: discovery 를 나눠 데몬 · 셸에 두고 crate 를 지운다 (S21)

> 상태: **2판(2026-10-07) — 2인 TRD 리뷰(FIX) 반영 · 재리뷰 전.** 코드는 아직 한 줄도 바뀌지 않았다. 리뷰 반영의 메인 결정은 해당 D · § 본문과 §8 「메인 처리」에 적었다.
> **결정 출처:** 사용자 위임 2026-10-07 아래의 메인 결정 — D1~D10. 그 밖의 내부 배치(모듈 이름 · 파일 자리 · 시험 자리 · 단위 안 순서)는 이 TRD 의 「제안」이다(이름은 전부 가안).
> **범위 = 작업 순서 2-2 전부**(`docs/refactoring/architecture-discussion-2026-09-26.md` §4 결정 후보 6 · §10 2-2) — ADR-0271 결정 1~7 을 코드로 옮긴다. 선행 1-1(base) · 1-3(platform)은 착지했다.
> **배치 근거:** `docs/README.md` 「새 내용을 어디에 넣나」의 「새 기능 **설계 착수** → `process/SN-name/`」. 형제(1-1 · 1-3 · 2-1 TRD)가 이 폴더에 있다.
> **표기:** 「실측」 = 기준 커밋 `6fa966a`(브랜치 `v0.3.3/refactor/crate-boundaries` 의 HEAD — master `175800b` 와 코드가 같다. 차이는 `.claude/handoff/` 두 파일뿐)에서 잰 것과 그 명령 · 「결정」 = 위 메인 결정 · 「ADR」 = 확정 ADR 본문 · 「제안」 = 이 TRD 의 안. 줄 번호는 따로 적지 않으면 `crates/engram-dashboard-discovery/src/lib.rs` 다.
> 앵커: **ADR-0271**(이 단계의 헌장) · ADR-0264(`DataLayout` · 배치) · ADR-0024(데이터 위치 — ADR-0271 이 고쳤다) · ADR-0269(base 입주 규칙 — 결정 7) · ADR-0266 · ADR-0275(platform) · ADR-0273(engram CLI 의 설치 위치 사본 — 2-3) · ADR-0134 · ADR-0135(데이터 폴더 · 잠금 파일 = 접속 파일) · ADR-0029(셸 = 데몬 클라이언트) · step-log S21.

---

## 0. 결론 (먼저)

```
① crate 를 2-2 에서 지운다. 데몬 몫(루트 규칙 · 폴더를 만드는 쓰기 확인 · 설치 위치 · DataLayout 데몬 절반)은 데몬 `data_dir`,
     셸 몫(찾기 · 띄우기 · 상태 · 끄기 · exe 찾기 · 사전 점검 · DataLayout 셸 절반)은 셸 `discovery` 모듈로 간다.
     셸 소비자의 경로 `crate::discovery::…` 는 그대로다 — 재수출 한 줄이 모듈 선언으로 바뀐다(D1).
② send_stop 묶음은 셸 `daemon_client/stop.rs` 로 글자 그대로 — 3-2 가 다시 쓸 때까지의 임시 거처.
     셸에 `tungstenite` 직접 줄이 서고 Cargo.lock 새 패키지는 0(실측 — 이미 셸 그래프에 있다)(D1).
③ 셸이 한 벌 더 갖는 것 = 루트 찾기 · daemon.json 자리 · logs\. 셸 lib_unit 의 같은 경로 시험 하나가 데몬 정본과 묶는다(D3 · D5).
④ 「현재 exe 옆 실행 파일」 = platform `env::sibling_exe` 한 곳. 소비자 셋 — 데몬 CLI 찾기 · 셸 데몬 찾기 첫 후보 · roundtrip 하네스(D2).
⑤ 쓰기 프로브 도우미 넷 = base 새 모듈 `writable`. cfg 0 · 도메인 지식은 오류 타입 하나라 io::Result 로 돌려준다(D4).
⑥ discovery async 반입 게이트는 crate 삭제 커밋에서 같은 꼴 새 대상으로 갈아 끼운다(platform 게이트 ⑦) — agent(동기 소비자 ·
     base · command · platform 을 한 번에)와 net 기본 feature = 0줄 · 짝 net `server` = 1줄 이상(D7 · §8 O1).
⑦ 단위 넷 U1 → U4. 어느 커밋에서 멈춰도 빌드 · 회귀가 초록. U3 은 쪼갤 수 없는 원자 단계 하나(이름 전환)를 품는다.
     U3 · U4 는 /qa full — 셸이 실 데몬을 띄우는 것을 GUI 로 잰다(완료 조건 · D10).
```

**결정 출처:** 사용자 위임 2026-10-07 아래의 메인 결정이다(머리말).

**지금 하지 않는 것:** `send_stop` 다시 쓰기 · 접속 정보 인터페이스(3-2) · 원격 · 같은 PC 인증 통일(나중 — ADR-0271 「영향」) · engram CLI 의 설치 위치 사본(2-3 · ADR-0273 결정 5) · 셸 → (dev) 데몬 간선 정리(2-4) · `DiscoveryError` 개명 · 소비자 0 인 셸 경로(`shell_state_dir` 등)에 쓰임 만들기 · 트레이 메뉴 클릭 자동화.

---

## 1. 현황 실측 (`6fa966a`)

### 1-1. crate 의 내용과 갈 곳

| 조각 | 자리 | 지금 쓰는 쪽 | 갈 곳 |
|---|---|---|---|
| 루트 찾기 `default_data_dir` · `data_dir_env_override` · `release_data_dir` + 상수 셋 | `:49-150` | 데몬 · 셸 | 데몬 정본 + 셸 사본(D5) |
| 쓰기 프로브 `write_probe_payload` · `probe_path` · `probe_write_in` · `probe_write_at` · `retry_if_vanished` + 상수 둘 | `:67-77` · `:152-238` | 아래 두 확인 함수 | base `writable`(D4) |
| `ensure_data_dir_writable`(폴더를 만든다) | `:240-255` | 데몬 `lib.rs:469` | 데몬 |
| `check_data_dir_writable` + `_once`(만들지 않고 되돌린다) | `:257-348` | 셸 `ensure_daemon` 사전 점검(`:960-968`) | 셸 |
| 설치 위치 `find_install_root` · `find_workspace_root` · `is_workspace_root` | `:350-392` | 데몬 `control/priming.rs:276` · `bin/engram.rs:337` | 데몬 |
| `DiscoveryError` · 주입 trait 넷 · 시계 `Clock`(base 상위 트레이트) · `ensure_with` · status · read_live · `daemon_stop` · `TaskKiller` · `ensure_daemon` · `locate_daemon_exe` · `FileReader` · `RealLiveness` · `WmiSpawner` | `:394-727` · `:915-1071` | 셸 | 셸 `discovery` |
| `send_stop` 묶음(`StopOutcome` · `StopSender` · `stop_with_sender` · `build_stop_command` · `build_auth_command` · `TungsteniteStopSender` · `STOP_WS_TIMEOUT`) | `:729-913` | 셸(트레이) | 셸 `daemon_client/stop.rs`(D1) |
| `DataLayout` — `daemon_state_dir` · `daemon_run_dir` · `daemon_file` · `mcp_config_dir` · `usage_probe_dir` · `logs_dir` · `ensure_daemon_dirs` | `layout.rs:34-101` | 데몬 | 데몬 |
| `DataLayout` — `resolve` · `shell_config_dir` · `shell_state_dir` · `shell_run_dir` · `webview_dir` · `daemon_file`(사본) · `logs_dir`(사본) | 같은 파일 | 셸 | 셸 |
| 시험 — 단위 65(그중 `#[ignore]` 2) · `layout.rs` 2 · `tests/stop_smoke.rs` 3(전부 `#[ignore]` · `#![cfg(windows)]`) | `:1075-2644` 등 | — | 함수를 따라간다(§3-5) |

OS `cfg` 는 운영 코드에 없다 — `rg "cfg\(|cfg!\("` 는 `:98` · `:116`(`debug_assertions` — OS 술어 아님)과 시험의 `#[cfg(windows)]` 셋(`:1305` · `:2481` · `:2557`)뿐이다. `std::os::` 0줄.

### 1-2. 소비자

- **데몬** — 매니페스트 `Cargo.toml:89-90`(normal). `lib.rs:37` import · `:68-70` `resolve_data_dir` · `:450` `DataLayout::new` · `:455` `logs_dir` · `:469` `ensure_data_dir_writable` · `:474` `ensure_daemon_dirs` · `:496` `daemon_file` · `:584` `mcp_config_dir` · `:212-215` `daemon_state_dir` · `usage_probe_dir` · `control/priming.rs:276` · `bin/engram.rs:337` · `tests/ws_e2e.rs:2326`(`DataLayout` — `:2392` · `:2428` · `:2606`) · 다리 시험 `lib.rs:1116-1135`(`resolve_data_dir` == discovery 판).
- **셸** — 매니페스트 `src-tauri/Cargo.toml:76`. `src/lib.rs:13` 이 crate 를 `crate::discovery` 로 재수출하고 대부분이 그 경로를 쓴다 — `lib.rs:73`(`DataLayout::resolve().shell_config_dir`) · `:87` · `:92`(`logs_dir`) · `commands/discovery.rs:11` · `:88-110` · `:167-191` · `tray/mod.rs:28`(`StopOutcome`) · `:153-155` · `:201-217` · `tray/actions.rs:182-183`(`send_stop`) · `:203-204`. 재수출을 거치지 않는 둘 — `ui_settings.rs:240`(`default_data_dir().join(UI_SETTINGS_FILE)`) · `daemon_client/mod.rs:124-133`(`RealDiscovery`).
- 셸 쪽 `DataLayout` 의 코드 소비자는 `resolve` · `shell_config_dir` · `logs_dir` 셋이다. `shell_state_dir` · `shell_run_dir` · `webview_dir` 와 데몬 쪽 `daemon_run_dir` 의 바깥 호출은 0(`rg` — `daemon_run_dir` 은 같은 파일 안 넷이 쓴다).

### 1-3. 실행 파일 위치 — 세 벌

| 자리 | 규칙 |
|---|---|
| 데몬 `lib.rs:101-118` `locate_send_exe` | `current_exe` 폴더 + `platform::env::exe_file_name(CLI_EXE_NAME)` · `is_file` · 못 찾으면 warn + `None` |
| discovery `:983-999` `locate_daemon_exe` | 첫 후보 = 같은 규칙(`"engram-dashboard-daemon"`) · 그 뒤 `<cwd>\target\debug` · `<cwd>\..\target\debug` → `locate_in`(`:1001-1011`) |
| 데몬 `src/bin/roundtrip_smoke.rs:902-911` `sibling_send_exe` | 같은 규칙인데 `.exe` 를 `cfg!(windows)` 로 손으로 붙인다(`:905`) |

`find_install_root`(`:359-365`)는 exe 에서 위로 걸어 작업 루트를 찾는 다른 규칙이다.

### 1-4. 쓰기 프로브 도우미 — cfg · 도메인 판정 근거

- **cfg 0**(1-1 끝).
- **도메인이 닿는 자리 셋:** ① `retry_if_vanished`(`:227-238`)가 `DiscoveryError` 를 돌려준다 — 접기는 `unwritable`(`:160-165`)이 한다. ② 프로브 파일 이름 접두 `.engram-write-probe-`(`:73`) · 내용 `engram-write-probe`(`:77`) — 제품 이름이다. ③ doc 이 트레이 「데몬 켜기」와 부팅 ensure 의 경합을 사례로 든다(`:69-72` · `:221-226`).
- `probe_write_at` 은 `tracing::warn!` 을 직접 부른다(`:213-216`) — base 는 이미 `tracing` 을 의존한다(ADR-0269 결정 4 · ADR-0268).
- `:167-173` 의 doc(「`dir` **안에** 파일을 만들어 … 남길 수 있다(계약)」)은 `probe_write_in` 의 것인데 `probe_path` 위에 붙어 있다.

### 1-5. 의존 · tungstenite (실측 `cargo tree`)

- 셸 직접 워크스페이스 의존(normal · `--depth 1`) = base · command · discovery · net · protocol. platform 은 **discovery 경유 하나로만** 닿는다(`cargo tree -p engram-dashboard -e normal -i engram-dashboard-platform`).
- `tungstenite v0.26.2` 는 Cargo.lock 에 하나뿐이고 셸 normal 그래프에 이미 두 경로로 있다 — discovery · `tokio-tungstenite v0.26.2`(`cargo tree -p engram-dashboard -e normal -i tungstenite`). `tempfile 3.27.0` 도 lock 에 있다.
- `cargo tree -p <p> -e normal --prefix none --target all | rg -i "^(tokio|mio|tokio-tungstenite|futures-util) "` → platform · net(기본 feature) · discovery · **agent** 넷 다 0줄(transport 는 tokio · futures-util). ★agent 는 tokio 없는 동기 crate 이면서 platform 을 운영 의존으로 진다★ — platform 의 「async 런타임 금지」가 지키는 소비자가 2-2 뒤에도 남는다(§8 O1).
- `-e normal,build,features` 는 dev 의존이 켠 기능을 세지 않는다 — agent(platform 을 normal 로 · `test-support` 를 dev 로 진다)에서 그 꼴이 0줄, `-e normal,dev,features` 로 1줄(실측). 셸이 같은 모양이 되어도 platform 시험 기능 게이트는 0줄로 선다.

### 1-6. 게이트 · 바인딩이 discovery 를 부르는 자리

`ci.yml` — 메시징 이름 정규식 `:527` · base Tauri 게이트 주석 `:619` · platform 시험 기능 게이트 주석 `:747-753`(소비 crate 「agent · discovery」 · 「셸은 platform 을 직접 의존하지 않고 전이로만 닿는다」) · 셸 게이트 주석 `:806` · gate 4 명단 `:870`(`discovery/src/lib.rs`) · `:882`(`discovery/tests/stop_smoke.rs`) · net gate 1 `:1087` · `:1091` · async 반입 게이트 `:1191-1212` · 버전 게이트 주석 `:1358`. `.claude/skill-bindings/qa.md` — `:73-74` · `:148` · `:165` · `:242` · `:251` · `:258` · `:267` · `:332` · `:428`. base 게이트 ③ 알파벳(`(logging|text|time|path|sync|testing)`)의 사본 여섯 — base `lib.rs:51` · `CLAUDE.md:251` · `ci.yml:653` · `qa.md:149` · `docs/testing-strategy.md:50` · `:169`.

### 1-7. 기록과 어긋난 사실

| 기록 | 실제(실측 `6fa966a`) |
|---|---|
| 메모 §4 · §11 — 실행 파일 위치가 두 곳 `daemon/src/lib.rs:96,146` · `discovery lib.rs:981` | **세 곳**(1-3). 옛 줄 둘은 낡았다 — `:96` 은 `locate_send_exe` doc 중간 · `:146` 은 panic hook 끝이다. 실제 = 데몬 `:101` · discovery `:983` · roundtrip `:902` |
| ADR-0271 「맥락」 — 실행 파일 위치 계산은 두 곳 | 세 곳(위) |
| ADR-0271 표 — 셸이 `shell_state_dir` · `shell_run_dir` · `webview_dir` 를 쓴다 · 데몬이 `daemon_run_dir` 를 쓴다 | 코드 소비자 0(1-2). 결정 4 대로 옮기되 옮기는 것은 쓰임이 아니라 ADR-0264 결정 2 의 배치 지도다 |
| 조사 입력 — discovery 가 net · protocol · tungstenite · serde_json 을 의존하는 이유는 `send_stop` 하나 | net · tungstenite · serde_json(운영)은 맞다. **protocol 은 아니다** — `DaemonInfo` · `PROTOCOL_VERSION` 을 판정(`check_acceptable` `:452-462`)과 읽기(`FileReader` `:1015-1030`)가 쓴다. ADR-0271 의 「protocol **명령 메시지**」(`AgentCommand` · `RequestId` — `:807-813`)는 맞다 |
| 메모 §11 — 실 WMI `#[ignore]` 둘이 `cargo test -p engram-dashboard-discovery` 로 `ExeNotFound` | 맞다(코드 읽기 — 후보가 시험 exe 폴더 `deps\` 와 cwd = crate 폴더 기준). 셸로 옮기면 cwd 가 `src-tauri` 라 셋째 후보 `<cwd>\..\target\debug` 가 저장소 `target\debug` 를 가리켜 **우연히 풀린다**(코드 읽기 · 미검). D6 은 그 우연에 기대지 않는다 |
| 셸 `commands/discovery.rs:161` 「taskkill /F」 | `/F /T` — `TaskKiller`(`:915-929`) → platform `process::kill_tree` |
| 셸 `lib.rs:11` 「tray-host 와 공유」 · discovery `Cargo.toml` description 「Embedded(src-tauri)와 tray-host 가 공통 의존」 | embedded · tray-host 는 ADR-0029 로 사라졌다 — 소비자는 데몬 · 셸 |
| `docs/testing-strategy.md:97` — src-tauri ① 단위에 「`ensure_with` OS/WMI/clock trait 주입 순수 검증」 | 지금 그 시험은 discovery 에 있다(`:1526-2059`). U3 뒤에 참이 된다 |
| `docs/testing-strategy.md:55` — 실 WMI `#[ignore]` 둘은 「discovery 에 남는다」 · 1-1 표의 `:167-173` doc 위치 | U3 이 셸로 옮긴다 · doc 은 U1 이 제자리(`probe_write_in`)로 |
| net `Cargo.toml:59-64` · platform `src/lib.rs:54-55` — 자기 성질의 기계 벽으로 「discovery has no async-runtime ingress」를 가리킨다 | 그 게이트는 D7 로 대상이 바뀐다(platform 게이트 ⑦) — 두 서술은 새 게이트를 가리키게 U4 가 고친다(§2-6) |

---

## 2. 결정 (사용자 위임 2026-10-07 아래의 메인 결정) · 설계

| | 결정 | 한 줄 이유 |
|---|---|---|
| **D1** | crate 를 2-2 에서 지운다 · `send_stop` 묶음은 셸 데몬 연결 코드 자리로 글자 그대로(3-2 까지의 임시 거처) · 셸에 `tungstenite` 직접 줄(새 패키지 0) | ADR-0271 결정 1 · 결정 5 의 「2-2 와 3-2 사이 자리」(열린 것)를 채운다 |
| **D2** | 실행 파일 위치 「두 곳 → 한 곳」 = platform `env` 도우미 하나 — 데몬 `locate_send_exe` · 셸 `locate_daemon_exe` 첫 후보 · `sibling_send_exe` 가 부른다 · `find_install_root` 는 그대로 | 셸은 데몬을 의존하지 못하므로 한 곳은 둘 다 아래의 platform 이다 · `exe_file_name` 이 이미 거기 산다 |
| **D3** | `logs\` 는 셸이 사본을 갖고 같은 경로 시험에 넣는다 | 둘 다 쓴다 · 로그 폴더가 갈리면 기동 실패를 쫓을 두 로그가 흩어진다(셸 `src/lib.rs:84-86` 주석) |
| **D4** | 쓰기 프로브 도우미 넷을 base 새 목적 모듈로 — 게이트 ③ 알파벳을 같은 단위에서 · `ensure_data_dir_writable` = 데몬 · `check_data_dir_writable` = 셸, 둘 다 그 위에 | ADR-0269 결정 7 — 나누면 쓰는 곳이 둘이다(ADR-0269 「입주시키지 않는 것」이 스스로 적은 조건) |
| **D5** | `ui-settings.json` 자리 불변 — 그것이 부르는 셸 루트 사본이 같은 경로 시험에 든다 | ADR-0271 결정 4 의 사본 둘(루트 · `daemon.json`) |
| **D6** | 실 WMI 시험은 시험 쪽만 고친다(현재 exe 기준 도우미) — 운영 후보는 그대로 | 운영 동작을 시험 편의로 바꾸지 않는다 |
| **D7** | discovery async 반입 게이트는 지우지 않고 대상을 갈아 끼운다 — agent · net 기본 = 0줄 + 짝 net `server` ≥ 1줄(platform 게이트 ⑦ · 리뷰 반영 메인 결정) | discovery 는 사라지지만 그 게이트가 지키던 축 둘이 남는다 — 동기 crate 로의 async 유입에는 소비자 agent 가 남고(§1-5), net 기본 feature 의 조용한 반입은 2-2 뒤 동기 소비자가 없으므로(셸은 tokio 를 진다 · §4) net 자신의 `default = []` 설계 성질로 잰다 |
| **D8** | 게이트 명단 편집은 그 파일을 옮기는 단위에서 · 메모 §11 의 낡은 포인터와 `taskkill` 주석도 그 작업에서 | gate 4 는 양방향이라 어긋나면 그 커밋부터 CI 가 빨갛다 |
| **D9** | 단위 순서 = ① platform 도우미 + base 프로브 ② 데몬 사본 ③ 셸 사본 + 같은 경로 시험 ④ crate 삭제 · 게이트 · 문서 | 갈 곳을 먼저 세우고 옛 자리를 지운다 — 어디서 멈춰도 빌드가 선다 |
| **D10** | 완료 = GUI 에서 실 데몬 기동 실측(2-1 TRD §5 꼴) | ADR-0271 결정 7 |

### 2-1. D1 — crate 삭제 · 셸 `discovery` 모듈 · `send_stop` 임시 거처

- **셸 자리(제안):** `src-tauri/src/discovery/{mod.rs, layout.rs, tests.rs}`. `src/lib.rs:13` 의 `pub use engram_dashboard_discovery as discovery;` 가 `pub mod discovery;` 로 바뀌어 **셸 소비자 경로가 그대로다**(1-2 의 재수출 경유 자리 전부). 「discovery = 데몬 찾기 · 띄우기」라는 이름은 셸에 남는 몫과 맞고, 프론트 주석(`src/api/agentClient.ts:177` 의 `discovery::ensure_daemon`) · `tray/core.rs:3` 같은 개념 서술도 그대로 참이다. `commands::discovery`(Tauri 명령)와는 경로가 달라 겹치지 않는다. 시험은 `tests.rs` 로 뗀다(`daemon_client/tests.rs` 와 같은 꼴) — 그러면 OS `cfg` 파일은 `tests.rs` 하나다(gate 4).
- **`send_stop` 묶음 → `src-tauri/src/daemon_client/stop.rs`(가안 · `pub mod stop`):** `:729-913` 의 본문을 글자 그대로 옮긴다. 바뀌는 것은 경로와 가시성뿐이다 — 타입만 여는 것으로는 모자란다(리뷰 지적). ① 운영: `crate::discovery` 가 `check_acceptable` · `AcceptCheck` · `FileReader` · `RealLiveness` 를 `pub(crate)` 로 열고, **`FileReader` 의 필드 `path` 도 `pub(crate)`** 로 연다 — `send_stop` 이 구조체 식 `FileReader { path: … }`(`:779-781`)로 만들기 때문이다(생성자를 새로 두지 않는다 — 글자 그대로 원칙). `DaemonReader` · `PidLiveness` · `DiscoveryError` 는 이미 `pub`. ② 시험: `discovery/mod.rs` 의 선언이 `#[cfg(test)] pub(crate) mod tests;` 이고, 그 안에서 `FakeReader` 와 **연관 함수 `FakeReader::new`**(`:1461-1466`) · `FakeLiveness` 와 **필드 `dead`**(`:1447-1449` — 옮긴 시험이 구조체 식 `FakeLiveness { dead: … }` 로 만든다) · `info`(`:1436`)를 `pub(crate)` 로 올린다. `FakeReader` 의 `seq` · `calls` 는 비공개 그대로다(stop 시험은 안 만진다 — `calls` 는 같은 모듈의 ensure 시험만 읽는다). `CountingStopSender`(`:2191-2206`)는 stop 시험에서만 쓰여 함께 옮긴다. 시험 9(`send_stop_*` 7 · `build_*` 2)가 따라간다. 머리 주석 = 「임시 거처 — 3-2(transport 셸 부착)가 정지 명령 클라이언트를 다시 쓴다(ADR-0271 결정 5)」 + `// ADR-0271`. 트레이 두 파일의 세 자리(`tray/mod.rs:28` · `:217` · `tray/actions.rs:183`)가 이 경로를 부른다.
- **의존:** `src-tauri/Cargo.toml` 에 `tungstenite = "0.26"`(주석: `send_stop` 의 동기 WS · 3-2 가 걷는다) · `engram-dashboard-platform`(normal — `WmiSpawner` · `RealLiveness` · `TaskKiller` · `locate_daemon_exe` 가 부른다) · dev 에 platform `test-support`(`real_wmi_spawn_flag_matrix` 의 `spawn::wmi_create_raw`) · dev `tempfile`(`stop_smoke`). discovery 줄 삭제. ★dev platform `test-support` 줄은 discovery `Cargo.toml:57-59` 의 주석을 함께 옮기고 경고 한 줄을 더한다★ — `--all-targets` 처럼 lib 과 시험 타깃을 **한 cargo 호출**로 지으면 그 기능이 셸 운영 바이너리로 합쳐진다(데몬 `Cargo.toml:176-182` 의 self-dev-dependency 와 같은 위험군). platform 시험 기능 게이트(③)는 그 꼴을 못 본다(그 게이트의 알려진 한계 — `ci.yml` 같은 스텝 주석). ★Cargo.lock 새 `[[package]]` 0★ — 셸 항목의 이름 목록만 바뀐다(+ platform · tungstenite · tempfile · − discovery)(1-5 실측). U4 에서 discovery `[[package]]` 가 빠진다.
- **데몬 자리(제안):** `crates/engram-dashboard-daemon/src/data_dir.rs`(`pub mod data_dir`) — `default_data_dir` · `release_data_dir` · `data_dir_env_override`(비공개) · `ensure_data_dir_writable` · `find_install_root`(+ 두 도우미) · `DataLayout`(데몬 절반 — `resolve` 는 소비자가 없어 빼고 데몬은 `resolve_data_dir` 로 연다). 공개 범위 = 셸 시험이 dev 의존으로 부르는 셋(`default_data_dir` · `release_data_dir` · `ensure_data_dir_writable`) + `DataLayout` · `find_install_root`(부르는 쪽 = lib `control/priming.rs:276` · bin `engram.rs:337`). ★bin `engram.rs` 가 새로 데몬 lib(`engram_dashboard_daemon::data_dir::find_install_root`)를 부르게 된다★ — ADR-0273 「맥락」의 「데몬 lib 를 한 줄도 쓰지 않는다」가 낡는다. 그 호출 자리에 `// ADR-0273` 앵커와 한 줄(「2-3 이 이 간선을 CLI 쪽 사본으로 바꾼다 — ADR-0273 결정 5」)을 단다(§6 2 · §7 6). 쓰기 확인 실패는 데몬 자기 오류 타입(가안 `DataDirUnwritable { path, reason }`)으로 내고 **문구는 지금 글 그대로**(`:410` — 「데이터 폴더에 쓸 수 없음(…): … — 쓰기 가능한 위치에 압축을 풀어 주세요」). 앵커 `// ADR-0271`(루트 규칙의 정본) · `// ADR-0264`.
- **override 주석(`:32-47`)은 가른다** — 규칙 부분(테스트 격리 탈출구 · 배포 노브 아님 · 인스턴스 스코프)은 데몬 정본으로, 「WMI 경로엔 닿지 않는다」는 셸(WMI 로 띄우는 쪽)로. 셸 사본 머리는 「데몬 `data_dir` 의 사본 — 같은 경로 시험이 묶는다 · 설명 정본은 데몬」 한 단락만 둔다.

### 2-2. D2 — `platform::env::sibling_exe`

- **모양(가안):** `pub fn sibling_exe(stem: &str) -> Option<PathBuf>` = `current_exe()` 의 폴더 + `exe_file_name(stem)`. `None` 은 `current_exe` · 부모를 못 얻을 때뿐이고 **존재는 보지 않는다** — 존재 판정 뒤 하는 일이 부르는 쪽마다 다르다(데몬 = info/warn 로그 · 셸 = 후보 목록의 첫 칸 · 하네스 = `filter(is_file)`). platform 들이는 규칙 1(원시 값만 · 도메인 0) · 3(cfg 없음 — `EXE_SUFFIX` 는 std 상수)을 지킨다. 시험 1(`current_exe` 폴더 + `exe_file_name` 과 같다).
- **소비자 셋:** 데몬 `locate_send_exe` 본문(`:104-113`) · 셸 `locate_daemon_exe` 의 첫 후보(`:986-991`) · `sibling_send_exe`(`:902-911` — `cfg!(windows)` 가 사라진다). `roundtrip_smoke.rs` 는 시험 모듈(`:1114` 아래 `:1208` 등)에 `cfg!(windows)` 가 남아 **gate 4 명단에 그대로 남는다**(실측) — U1 은 명단을 안 건드린다.
- `find_install_root` 는 바꾸지 않는다(walk-up — 다른 규칙). 셸 `locate_daemon_exe` 의 cwd 후보 둘(`:993-996`)도 그대로다(D6).
- ADR-0271 결정 2 는 「실행 파일 위치 계산(두 곳 → 한 곳)」을 **데몬으로** 보냈다 — 한 곳이 platform 이 되는 것은 그 조각의 개정이다(같은 ADR 「열린 것」이 「현재 exe 옆 규칙 · `.exe` 붙이기는 platform 몫」으로 이미 가리켰다) → §7 1.

### 2-3. D4 — base `writable` (가안)

- **판정(1-4):** cfg 0 — 통과. 도메인이 닿는 셋 중 ① 은 시그니처로 걷힌다 — base 판 `retry_if_vanished(once) -> io::Result<()>` 는 `NotFound` 면 한 번 더 부르고 그 결과를, 아니면 첫 오류를 그대로 돌려준다. 접기(`DataDirUnwritable`)는 두 호출자가 `map_err` 로 한다 — 판정 표(`Ok` · `NotFound` 뒤 재시도 · 그 밖 오류)가 지금과 같다. ② 접두 `.engram-write-probe-` 는 제품 이름이지 에이전트 런타임 · wire · 데몬 살림 지식이 아니다 — base `logging` 의 로그 파일 이름 규약(`<종류>-<UTC>-<pid>.log`)과 같은 부류로 보고 **값 그대로** base 상수로 옮긴다(→ §8 O4). ③ doc 사례(트레이 · 부팅 ensure)는 「여러 프로세스가 같은 폴더를 동시에 검사할 수 있다」로 일반화하고 사례는 셸 `check_data_dir_writable` doc 에 둔다. → **입주한다**(결정 그대로).
- **공개 API(가안):** `probe_write_in(dir) -> io::Result<()>` · `retry_if_vanished(once) -> io::Result<()>` · `WRITE_PROBE_PREFIX`(소비자 시험이 잔여물을 센다 — `probe_leftovers` `:1164-1176`). `probe_write_at` · `probe_path` · 내용 상수 · `write_probe_payload` 는 비공개. 입주자끼리 무참조(조건 ③) — `std` 와 `tracing` 만 쓴다.
- **시험:** 옮김 2(`writable_probe_rejects_a_name_it_cannot_create` · `writable_probe_recovers_from_a_leftover_probe_file` — `probe_write_at` 을 직접 부른다) + 제안 1(`retry_if_vanished` 판정 표 — 지금은 경합 시험이 확률적으로만 덮는다).
- **같은 단위에서 고칠 사본:** 게이트 ③ 알파벳 여섯(1-6) → `(logging|text|time|path|sync|testing|writable)` · 입주자 수 「여섯」 → 「일곱」(base `lib.rs:3-7` · `CLAUDE.md` 「백엔드 모듈 맵」 base 항목 · base `Cargo.toml` description · 루트 `Cargo.toml:13-14` · `docs/testing-strategy.md:43`). ★알파벳은 이름 접두로 문다(경계 없음)★ — `writable` 로 시작하는 다른 이름을 base 안 `crate::` 뒤에 쓰지 않는다.
- **ADR-0269 와의 관계:** 「입주시키지 않는 것」의 `retry_if_vanished` · `probe_write_in` 항목은 「2-2 의 나누기가 사본 둘을 만들면 base 로」라고 스스로 조건을 적었다 — 개정이 아니라 그 조건의 충족이다(링크만 · §7 2). ★단 입주는 둘째 사본이 생기기 **전**(U1)에 당긴다★ — 이유 둘: ① 사본이 잠시라도 둘이 되는 중간 상태를 안 만든다 ② 프로브 이름의 정적 카운터(`probe_path` 의 `COUNTER` · `:175-180`)는 하나여야 한다 — 셸 lib_unit 경합 시험이 데몬 판(dev 의존)과 셸 판을 한 프로세스에서 섞어 부르므로, 사본마다 카운터가 따로면 같은 pid · 같은 번호의 프로브 이름이 겹친다(`create_new` 가 `AlreadyExists` 를 만나 남의 프로브를 지우고 다시 만든다 — 거짓 실패의 씨앗).

### 2-4. D3 · D5 — 셸 사본과 같은 경로 시험

- **셸 사본(`discovery/layout.rs` — 사본이 한 파일에 모인다):** 루트 찾기 셋 + 상수 셋(`:53` · `:55` · `:65`) + **walk-up 도우미 둘 `find_workspace_root` · `is_workspace_root`**(`:367-392` — 디버그 분기가 부른다 `:104`. 셸에서는 그 분기만 부르므로 둘 다 `#[cfg_attr(not(debug_assertions), allow(dead_code))]` — `LOCAL_DATA_DIR`(`:52`)과 같은 꼴) · `daemon.json` 자리(`DAEMON_FILE` · `daemon` · `run` 이름 — net `portfile::DAEMON_FILE` 을 빌리지 않는다. net 은 3-3 에서 셸을 떠난다) · `logs` 이름 · 셸 폴더 셋 · `webview`. `data_dir_env_override` 는 `pub(crate)` — 셸 사전 점검(`:961`)이 본다.
- **같은 경로 시험(가안 `the_shell_copies_resolve_the_paths_the_daemon_writes`)** — 셸 lib_unit(`discovery/layout.rs` 시험 · 셸 → (dev) 데몬 `src-tauri/Cargo.toml:105`):
  1. `ENGRAM_DATA_DIR` 세 상태(없음 · 빈 값 · 임시 경로)에서 셸 `default_data_dir()` == 데몬 `data_dir::default_data_dir()`.
  2. `release_data_dir(<표본>)` 같다.
  3. 1 의 각 루트에서 `DataLayout::new(r)` 의 `daemon_file()` · `logs_dir()` 이 데몬 판과 같다.
  - 환경은 모듈 락 아래서 바꾸고 단언 **전에** 되돌린다(`:1089-1101` 의 꼴). 셸 lib_unit 에서 `ENGRAM_DATA_DIR` 를 읽는 다른 시험은 지금 없다(`rg "ENGRAM_DATA_DIR|in_data_dir\(\)|default_data_dir\(\)" src-tauri/src` — 시험 안 0).
  - **환경 락 — 데몬 쪽(리뷰 반영 · 메인 결정 = 락 하나):** 데몬 lib 시험은 `ENGRAM_DATA_DIR` 를 `lib.rs` 의 `ENV_LOCK`(`:1151`) 아래서 바꾸고(`:1117` · `:1138`), 데몬으로 옮겨 오는 discovery 루트 시험 넷은 따로 정적 락(`:1087`)을 쥔다 — 그대로 옮기면 한 시험 바이너리에서 락 둘이 같은 환경을 짓밟는다. **결정:** `lib.rs` 의 `resolve_data_dir_*` 두 시험(다리 시험 포함)을 `data_dir` 시험 모듈로 옮기고 그 모듈의 락 **하나**만 쓴다 — `lib.rs` 의 `ENV_LOCK` 은 지운다(옮긴 시험은 `crate::resolve_data_dir()` 를 부른다 — 루트의 비공개 함수는 자손 모듈에서 보인다). 데몬 `src` 의 다른 `ENV_LOCK`(`control/ingress.rs` · `control/mod.rs` · `control/priming.rs` · `bin/engram.rs`)은 `ENGRAM_DATA_DIR` 를 만지지 않는다(실측 rg).
  - **환경 락 — 셸 쪽(U3 · 확인 결과 = 같은 문제 없음):** 루트 시험 넷은 데몬으로만 간다(§3-6) — 셸 lib_unit 에서 `ENGRAM_DATA_DIR` 를 **바꾸는** 시험은 같은 경로 시험 하나뿐이라 락이 하나다. **읽는** 시험은 `#[ignore]` 실 WMI 둘(`default_data_dir()` — `:2489` · `:2568`)뿐이고, `ensure_daemon_missing_exe_is_exe_not_found`(`:2466`)는 canonicalize 에서 먼저 실패해 사전 점검의 override 읽기(`:961`)까지 가지 않는다(코드 읽기). `--ignored real_wmi` 실행에는 같은 경로 시험이 안 끼지만 `--include-ignored` 에서는 겹치므로 **실 WMI 둘도 같은 락을 쥔다**(D6 과 같은 시험 쪽 편집 — 락은 `src-tauri/src/discovery/mod.rs` 의 `#[cfg(test)] pub(crate) static` 하나 — 실 WMI 둘은 형제 모듈 `discovery::tests` 에 살아 `layout` 시험 모듈 안에 두면 닿지 않는다. `layout` 의 같은 경로 시험과 실 WMI 둘이 그것을 쥔다).
  - **한계:** 릴리스 분기 본문(`:116-128`)은 대조하지 못한다 — 시험은 늘 debug 다(도우미 `release_data_dir` 만 잰다 — §8 O6). 셸이 사본 밖에서 경로를 조립하는 것도 못 잡는다(리뷰 몫). walk-up 표지 하나(`.git` · `[workspace]`)를 빠뜨린 사본도 못 잡는다 — 이 저장소 루트에 둘 다 있어 어느 하나로도 같은 루트가 나온다(실측). 표지 시험은 데몬에만 산다(§7 3).
- `ui-settings.json`(`ui_settings.rs:240`)은 셸 루트 사본을 부르므로 위 1 이 덮는다 — 자리 불변.
- **경합 시험(`:1245-1275`)은 셸 lib_unit 으로** — 「만들고 되돌리는 셸 점검 × 만들고 남기는 데몬 기동」이 실제로 겹치는 두 주체라, 데몬 판은 dev 의존으로 `data_dir::ensure_data_dir_writable` 을 부르고 결과를 `Result<(), String>` 으로 맞춘다.
- **다리:** U2 ~ U3 사이에는 데몬의 다리 시험(지금 `lib.rs:1116-1135` — 데몬 판 == discovery 판 · U2 가 `data_dir` 시험 모듈로 옮긴다 — 위 환경 락)이 두 벌을 묶는다. 그래서 U2 는 데몬 매니페스트의 discovery 줄을 지우지 않고 `[dev-dependencies]` 로 내린다 — 운영 그래프에서는 U2 에 빠지고 그 시험만 쓴다. U4 가 줄과 시험을 함께 지운다.

### 2-5. D6 — 실 WMI 시험의 데몬 exe

- 시험 쪽 도우미(가안 `test_daemon_exe()` — `tests.rs` 안) = `current_exe()` 의 할아버지 폴더(`target\<profile>\deps\…` → `target\<profile>`) + `platform::env::exe_file_name("engram-dashboard-daemon")`. `stop_smoke.rs:27-33` 의 `daemon_exe_path` 와 같은 꼴이다. `sibling_exe` 는 맞지 않는다 — 시험 exe 는 `deps\` 에 있다.
- 두 시험(`:2484` · `:2560`)의 `locate_daemon_exe()` 호출만 이것으로 바꾼다. 운영 `locate_daemon_exe` 후보는 그대로다.

### 2-6. D7 — async 반입 게이트 교체(platform 게이트 ⑦)

- **교체(리뷰 반영 메인 결정 — 지우기만 하지 않는다):** 스텝 `ci.yml:1191-1212`(「Gate: discovery has no async-runtime ingress」)를 **같은 자리에서** 새 스텝(가안 이름 `platform gate 7: no async-runtime ingress (agent · net default = 0, net server >= 1)`)으로 갈아 끼운다. 꼴은 옛 스텝 그대로다 — `cargo tree --locked … -e normal --prefix none --target all` · `cargo tree` 종료코드가 0 이 아니면 FAIL · 패턴 `rg -i "^(tokio|mio|tokio-tungstenite|futures-util) "`(뒤 공백 포함 앵커 — 옛 주석의 「빼지 말 것」을 옮긴다. ★discovery `Cargo.toml:49-50` 의 괄호 「지금은 앵커를 떼도 매치 0 이다」는 옮기지 않는다★ — 대상이 agent 면 앵커 없는 패턴이 `termios v0.2.2` 에 걸린다) · rg 종료코드 판정은 0 기대 줄(1 · 2)이 `1` = PASS · `0` = FAIL, 짝(3)이 그 반대(`0` = PASS · `1` = FAIL)이고, 둘 다 그 밖 = FAIL.
  1. `-p engram-dashboard-agent` → **0줄**. agent 가 실제 동기 소비자이고 base · command · platform 을 운영 의존으로 함께 지므로 셋이 한 줄로 덮인다(실측 §1-5 — 지금 0줄).
  2. `-p engram-dashboard-net`(기본 feature) → **0줄**. 옛 스텝 주석(`:1191-1195`)이 적은 축 — 「feature 0개 소비자에게 async 런타임이 조용히 딸려오는 것」(net `default = []` 결정을 부른 회귀) — 을 소비자 대신 net 자신으로 잰다.
  3. **짝**(ADR-0278 꼴): `-p engram-dashboard-net --features server` → **1줄 이상**. 패턴이 깨지거나 `cargo tree` 출력 꼴이 바뀌어 0 기대 둘이 눈먼 채 통과하는 것을 빨갛게 만든다(`server` 가 tokio · tokio-tungstenite · futures-util 을 켠다 — net `Cargo.toml` `[features]`).
  - 실측(`6fa966a` · 세 명령 다 `--locked` rc 0): agent 0줄 · net 기본 0줄 · net `--features server` **6줄**.
- 자리는 옛 스텝 자리(net 게이트 뒤 — 그 주석이 net 게이트 5 의 짝으로 적혀 있다). 대상 crate 가 없으면 옛 스텝의 `cargo tree -p` 가 죽어 FAIL 이므로 crate 삭제와 **같은 커밋**이다.
- **사본 · 가리키는 서술(지우지 않고 새 대상으로):** `qa.md:73-74`(「CI와의 분담」 블록 — 명령 셋과 기대값) · `CLAUDE.md:222`(「CI에만 있고 이 목록엔 없는 게이트」의 「discovery async 반입」 → 「async 반입(agent · net 기본 — platform 게이트 ⑦)」) · platform `src/lib.rs:54-55`(async 런타임 금지의 벽 = **게이트 ⑦ · 명령 · 기대값 정본 = `ci.yml` 의 `platform gate 7` 스텝** — 헤더 「격리 게이트」 머리의 번호 안내(`:58-59`)에도 ⑦ 을 더한다) · net `Cargo.toml:59-64`(「소비자 쪽 트리 실측 — discovery 의 게이트」 → 같은 스텝의 net 기본 feature 줄).

---

## 3. 작업 단위

**원칙(사용자 2026-10-02 — 「망가지지 않는 단위로」 · 「임시 땜빵 금지」):** 단위마다 워크스페이스가 빌드되고 회귀가 초록인 채 끊는다. 갈 곳을 먼저 세우고 옛 자리를 지운다. U2 ~ U4 사이 데몬 · 셸 사본과 discovery 원본이 함께 서는 것은 땜빵이 아니라 순서다(다리 시험 · 같은 경로 시험이 그 사이를 묶는다).

### 3-1. 단위 표

| 단위 | 범위 | 건드리는 파일 | 리뷰 · QA |
|---|---|---|---|
| **U1** | D2 · D4 · D8(실행 파일 줄 · base 알파벳) | platform `src/env.rs` · `src/lib.rs`(헤더 env 서술) · daemon `src/lib.rs`(`:84` 주석 · `:101-118`) · `Cargo.toml:84-86` 주석 · `src/bin/roundtrip_smoke.rs:902-911` · discovery `src/lib.rs`(`:152-238` 삭제 · `:250-255` · `:289-291` · `:983-991` · 시험 `:1164-1236` 일부) · base `src/writable.rs`(새) · `src/lib.rs` · `Cargo.toml` · 루트 `Cargo.toml:13-14` · `ci.yml:653` · `qa.md:149` · `CLAUDE.md`(`:251` · base · platform 항목) · `docs/testing-strategy.md`(`:43` · `:50` · `:55` env · `:169`) · 메모 §4 · §11 실행 파일 줄 | `/implement standard` · `/review code full` · `/qa standard` + ws_e2e `#[ignore]` 실 데몬 레인(데몬 기동 경로의 `locate_send_exe` 가 바뀐다) |
| **U2** | D1 데몬 몫 · D3 · D5 데몬 쪽 | daemon `src/data_dir.rs`(새) · `src/lib.rs`(`:37` · `:62-70` · `:450-476` · `:470` 주석 · 시험 `:1115-1151` → `data_dir` 시험 모듈 · `ENV_LOCK` 삭제) · `src/control/priming.rs:233-234` · `:276` · `src/control/mcp_config.rs:42` · `src/bin/engram.rs:337`(+ `// ADR-0273`) · `tests/ws_e2e.rs:2326` · `Cargo.toml:89-90`(normal → dev) | `/implement standard` · `/review code full` · `/qa standard` + ws_e2e `#[ignore]` 실프로세스 |
| **U3** | D1 셸 몫 · D3 · D5 · D6 · 같은 경로 시험 · D8(gate 4 +2 · platform 시험 기능 서술 · taskkill 주석) | 셸 `src/discovery/{mod.rs, layout.rs, tests.rs}`(새) · `src/daemon_client/stop.rs`(새) · `src/daemon_client/mod.rs`(`pub mod stop` · `:101-134`) · `src/lib.rs:11-13` · `src/ui_settings.rs:240` · `src/tray/mod.rs`(`:28` · `:217` · `:240-246` 주석) · `src/tray/actions.rs:183` · `src/commands/discovery.rs:161` · `tests/stop_smoke.rs`(새) · `Cargo.toml` · `Cargo.lock` · `ci.yml`(`:747-753` · gate 4 명단) · `qa.md`(`:20` · `:53` · `:138` · `:143` · `:161-163` · 새 2g · `:258` · `:332` · 「CI와의 분담」 실 레인) · `CLAUDE.md`(src-tauri 항목 · `:231` · `:239` · `:240` · platform 시험 기능 줄) · `docs/testing-strategy.md`(src-tauri 절 · `:55`) | `/implement standard` · `/review code full`(+ doc 렌즈) · `/qa full`(GUI §5-1 · 실 레인 §5-2) |
| **U4** | D1 삭제 · D7 · D8 나머지 · 문서 | `crates/engram-dashboard-discovery/` 삭제 · 루트 `Cargo.toml:2-8` · `:23` · `Cargo.lock` · daemon `Cargo.toml`(dev 줄) · `src/data_dir.rs`(다리 시험) · `ci.yml`(`:527` · `:619` · `:747-753` · `:806` · `:870` · `:882` · `:1087-1091` · `:1191-1212` 교체 · `:1358`) · `qa.md`(`:73-74` 교체 · 나머지) · `CLAUDE.md`(`:222` 교체 · 나머지) · platform `src/lib.rs:54-59` · net `Cargo.toml:59-64` · §6 의 U4 몫 | `/implement standard` · `/review code full`(+ doc 렌즈) · `/qa full`(완료 실측 §5-3) |

**순서 U1 → U2 → U3 → U4.** U1 이 먼저인 이유 — U2 · U3 의 두 확인 함수가 base 판 위에 서야 사본이 프로브를 다시 베끼지 않는다. U2 가 U3 보다 앞인 이유 — 같은 경로 시험 · 경합 시험이 데몬 판(`data_dir`)을 부른다. **파일 겹침:** discovery `src/lib.rs`(U1 편집 · U4 삭제) · `CLAUDE.md` · `qa.md` · `ci.yml`(U1 · U3 · U4) · daemon `Cargo.toml`(U1 주석 · U2 · U4) · daemon `src/lib.rs`(U1 · U2) · platform `src/lib.rs`(U1 · U4) → **한 코더씩 직렬**.

### 3-2. U1 — platform 도우미 · base 프로브

- **단위 안 순서:** ① platform `env::sibling_exe` + 시험 + 헤더 env 서술(아무도 안 부른다) ② 소비자 셋 전환 + 데몬 주석 ③ base `writable`(글자 그대로 + `retry_if_vanished` 시그니처 하나 · doc 제자리) + 시험 + base 헤더 · `mod` · 게이트 ③ 사본 여섯 · 입주자 수 ④ discovery 의 두 확인 함수가 base 를 부르고 옛 도우미 넷 · 시험 2 를 지운다(`probe_leftovers` 는 base 상수를 본다) ⑤ 메모 §4 · §11 실행 파일 줄.
- **수용 기준:** ① 「현재 exe 옆 + OS 확장자」 규칙이 `sibling_exe` 하나에만 있다 — `rg 'cfg!\(windows\)' crates/engram-dashboard-daemon/src/bin/roundtrip_smoke.rs` 에서 `:905` 줄이 사라진다 ② 프로브 넷이 base 에만 있다(`rg "fn (probe_path|probe_write_in|probe_write_at|retry_if_vanished)" crates src-tauri` → base 만) ③ 오류 문구 · 재시도 한 번 · 남김 계약이 같다(discovery 시험 그대로 초록) ④ discovery 공개 API 불변 — 데몬 · 셸 무변경 ⑤ 게이트 ③ 사본 여섯이 같은 알파벳.
- **게이트:** `cargo fmt --check` · `cargo test -p engram-dashboard-platform -- --test-threads=4` · `-p engram-dashboard-base` · `-p engram-dashboard-discovery` · `-p engram-dashboard-daemon -- --test-threads=4` · `cargo build -p engram-dashboard-daemon --features test-harness --bin roundtrip-smoke` · base 게이트 ①②③ · platform 게이트 4f~4h(명단 불변) · CI.
- **실 데몬 레인(분리 실행):** `cargo test -p engram-dashboard-daemon --test ws_e2e -- --ignored --nocapture --test-threads=4`(`docs/testing-strategy.md:158` 과 같은 플래그). U1 은 데몬 기동 경로의 `locate_send_exe` 본문을 바꾸지만 ★이 레인이 재는 것은 「데몬이 뜬다」뿐이다★ — 그 함수는 fail-open(못 찾으면 warn + `None` · 데몬은 CLI 입구 없이 그대로 뜬다)이라 바뀐 본문이 틀려도 레인은 초록이다. ADR-0271 의 「단위마다 … 데몬이 뜨는 채로 끊는다」(「영향」)만 이 레인이 잰다 — 시험 하네스는 실 데몬 exe 를 띄운다(`ENGRAM_DATA_DIR` 격리). ★그래서 확인 하나를 더한다★: 그 레인이 띄운 데몬의 로그(격리 데이터 폴더의 `logs\daemon-*-<pid>.log` — 데몬 기본 수준이 warn 이라 찍힌다)에 `locate_send_exe` 의 warn 「제어 평면 CLI 형제 exe 를 못 찾음 — CLI 입구 비활성(MCP 입구는 정상, ADR-0086 F1)」(데몬 `lib.rs:114-117`)이 없다 — 데몬 exe 옆에 `engram.exe` 가 지어져 있어야 하는 조건이다(없으면 옛 본문에서도 찍힌다 — 착수 전 기준선에서 먼저 본다).
- **critical 이 아닌 이유:** 규칙 한 줄과 도우미 넷을 자리만 옮긴다 — kill 인과 · finalize · 락 순서 · replay 어느 것도 지나지 않는다.
- **위험:** 알파벳 사본 하나를 빠뜨리면 로컬 · CI 가 갈린다(같은 커밋에서 여섯) · `retry_if_vanished` 의 오류 갈래를 바꾸면 경합 시험이 확률적으로만 잡는다(제안 시험이 결정적으로 잰다) · 프로브 접두 값을 바꾸면 소비자 시험의 잔여물 셈이 눈먼다.

### 3-3. U2 — 데몬 사본

- **단위 안 순서:** ① `data_dir.rs` — 루트 규칙 · `ensure_data_dir_writable`(base 위) · 설치 위치 · `DataLayout` 데몬 절반 + 시험 13 의 사본(아무도 안 부른다 · 환경 락은 그 시험 모듈의 하나) ② `lib.rs` 의 `resolve_data_dir_*` 두 시험을 그 모듈로 옮기고 `lib.rs` `ENV_LOCK` 삭제(§2-4 환경 락) — ★① 과 ② 는 한 커밋이다★: ① 만 커밋하면 그 중간 커밋의 데몬 lib 시험 바이너리에 `ENGRAM_DATA_DIR` 를 바꾸는 락이 둘(`lib.rs` `ENV_LOCK` · `data_dir` 시험 락) 서서 같은 환경을 짓밟는다 ③ 소비자 전환(`lib.rs` · `priming.rs` · `bin/engram.rs` + `// ADR-0273` · `ws_e2e.rs`) · `:470` 주석의 `DiscoveryError` ④ 매니페스트 discovery 줄 → `[dev-dependencies]`(다리 시험만 쓴다).
- **수용 기준:** ① `rg engram_dashboard_discovery crates/engram-dashboard-daemon` → 다리 시험(`data_dir.rs` 시험 모듈) 한 곳 ② 데몬이 같은 폴더 · 같은 잠금 파일을 쓴다 — 다리 시험 초록 + ws_e2e `#[ignore]` 실프로세스(`ENGRAM_DATA_DIR` 격리) 초록 ③ 쓰기 확인 실패 문구가 같다(옮긴 `writable_probe_rejects_child_of_a_file` 의 「쓰기 가능한 위치」 단언) ④ 셸 무변경 ⑤ **환경 락 하나** — `rg -n "ENGRAM_DATA_DIR|DATA_DIR_ENV" crates/engram-dashboard-daemon/src` 의 `set_var` · `remove_var` 줄이 전부 `data_dir` 시험 모듈의 한 락 아래다(`DATA_DIR_ENV` 를 함께 거는 이유 = 옮긴 discovery 시험은 문자열이 아니라 상수로 부른다).
- **게이트:** `cargo fmt --check` · `cargo test -p engram-dashboard-daemon -- --test-threads=4` · `cargo test -p engram-dashboard-daemon --test ws_e2e -- --ignored --nocapture --test-threads=4`(실 데몬 exe · 분리 실행) · `cargo build` · CI(`--locked` — lock 은 normal/dev 를 가르지 않아 바뀌지 않는다 · 확인).
- **위험:** 루트 규칙 사본이 한 글자 갈리면 데몬과 셸이 다른 폴더를 본다 — 다리 시험이 U2 ~ U3 을, 같은 경로 시험이 U3 뒤를 잡는다 · `find_install_root` 를 잘못 옮기면 프라이밍이 조용히 비활성된다(`priming.rs:233-234` 의 옛 사고 — `find_install_root_yields_absolute_path` 가 절대성만 잰다).

### 3-4. U3 — 셸 사본 · `send_stop` 이사 · 같은 경로 시험

- **단위 안 순서(어디서 멈춰도 빌드가 선다):**
  1. 셸 매니페스트에 platform · tungstenite · dev platform `test-support` · dev tempfile 을 더한다 — 쓰지 않는 의존일 뿐이다. ★1 을 따로 커밋하면 `Cargo.lock` 도 그 커밋에 든다★ — 셸 항목의 의존 목록이 바뀌고(+ platform · tungstenite · tempfile), CI 는 `--locked` 라 lock 을 빠뜨린 커밋부터 빨갛다.
  2. ★**원자 단계 — 쪼갤 수 없다**★: `discovery/{mod.rs, layout.rs, tests.rs}` · `daemon_client/stop.rs` 를 세우고 `lib.rs:13` 재수출을 `pub mod discovery;` 로 바꾸고 트레이의 `send_stop` · `StopOutcome` 경로를 `daemon_client::stop` 으로 돌린다. **같은 커밋에 gate 4 명단 `src-tauri/src/discovery/tests.rs` 한 줄.** 이름 `crate::discovery` 를 재수출과 새 모듈이 동시에 가질 수 없어서 원자적이다 — 대신 기계적(글자 그대로)이고 소비자 경로가 안 바뀐다. 시험은 경합 시험을 뺀 셸 몫 전부가 함께 온다(`public_entry_points_…` 의 `ensure_daemon_dirs()` 한 줄은 `create_dir_all(daemon_file 의 부모)` 로). **같은 단계의 가시성(§2-1 — 빠지면 이 단계가 컴파일되지 않는다):** 운영 `check_acceptable` · `AcceptCheck` · `FileReader` 와 그 필드 `path` · `RealLiveness` = `pub(crate)` · `discovery/mod.rs` 의 `#[cfg(test)] pub(crate) mod tests;` · 그 안 `FakeReader` + `FakeReader::new` · `FakeLiveness` + 필드 `dead` · `info` = `pub(crate)`.
  3. 같은 경로 시험 · 경합 시험(데몬 판 dev 호출) · `layout` 셸 절반 시험.
  4. 직접 부르던 둘(`ui_settings.rs:240` · `daemon_client/mod.rs:124-133`)을 `crate::discovery::` 로 → 매니페스트 discovery 줄 삭제 · `Cargo.lock`.
  5. `tests/stop_smoke.rs`(글자 그대로 · import 만 `engram_dashboard_lib::…`) + gate 4 명단 한 줄 · ★머리 주석 둘도 고친다★ — 실행 줄(`:11` `cargo test -p engram-dashboard-discovery --test stop_smoke -- --ignored`) → `cargo test -p engram-dashboard --test stop_smoke -- --ignored --test-threads=1` · `:2` 의 `discovery::send_stop` → 새 경로 `daemon_client::stop::send_stop` · 실 WMI 시험의 exe 도우미(D6)와 환경 락(§2-4 — `src-tauri/src/discovery/mod.rs` 의 `#[cfg(test)] pub(crate) static` 하나 · 같은 경로 시험과 같은 락). `stop_smoke` 는 CI 에 따로 스텝을 세우지 않는다(메인 결정) — 시험 셋이 전부 `#[ignore]` · `#![cfg(windows)]` · 실 데몬 exe 를 띄우는 레인이라 CI 기본 실행에서 돌 것이 없고, 컴파일은 CI 워크스페이스 회귀가 이미 덮는다(테스트 타깃을 전부 짓는다). 실행은 로컬 실 레인(§5-2)이다.
  6. 주석 · 문서(§6 U3 몫) · `commands/discovery.rs:161` → 「`/F /T`(platform `process::kill_tree`)」.
- **수용 기준:** ① `rg engram_dashboard_discovery src-tauri` → 0 ② 셸 직접 의존 = base · command · net · platform · protocol(`cargo tree --depth 1`) ③ 셸 게이트 1 · platform 시험 기능 게이트(셸 0줄) · gate 4(명단 +2) · gate 5 · 6 PASS ④ 옮긴 시험이 전부 초록 · 같은 경로 시험 초록 ⑤ 프론트가 받는 글(`DiscoveryError` 문구 · `DaemonInfoDto`)이 같다 ⑥ 환경 락 하나 — `rg -n "ENGRAM_DATA_DIR|DATA_DIR_ENV" src-tauri/src` 의 `set_var` · `remove_var` 가 전부 `discovery/mod.rs` 의 시험 락 아래고, 실 WMI 둘도 그 락을 쥔다(§2-4) ⑦ GUI §5-1.
- **게이트:** `cargo fmt --check` · `cargo build` · `cargo test -p engram-dashboard --test lib_unit` · `--test layout_apply` · `--test layout_commands` · `--test daemon_client_pending` · `--test daemon_client_replay` · `--test stop_smoke`(컴파일 — 시험은 `#[ignore]`) · `cargo test -p engram-dashboard-daemon -- --test-threads=4`(다리 시험) · 셸 게이트 1 · platform 게이트 3~6 · CI. 실 레인(§5-2)과 GUI(§5-1) — 둘 다 §5-0 의 사용자 확인(O3) 뒤.
- **critical 이 아닌 이유:** 끄기(`daemon_stop` → `kill_tree` · `send_stop`)와 띄우기(WMI)는 본문을 바꾸지 않고 옮긴다 — kill 인과 · 단일 인스턴스 판정의 코드가 같다. 그 대신 실 데몬으로 GUI 실측한다.
- **위험:** 원자 단계가 크다(그 하나는 한 코더에게) · gate 4 명단 한 줄을 빠뜨리면 그 커밋부터 CI 가 빨갛다 · `pub(crate)` 로 연 내부를 셸 다른 모듈이 쓰기 시작하면 3-2 의 다시 쓰기가 넓어진다(연 것 — 넷과 `FileReader.path` — 외 사용 금지를 `stop.rs` 머리에 적는다) · 같은 경로 시험이 바꾼 환경이 병렬 시험에 새면 그 시험이 엉뚱한 폴더를 본다(단언 전 복원 · 읽는 실 WMI 둘도 같은 락 — §2-4) · 셸 lib_unit 에 프로세스를 만드는 줄이 처음 든다(`#[ignore]` 실 WMI 둘 — 기본 실행은 그대로 0) · dev platform `test-support` 가 `--all-targets` 한 호출 빌드로 셸 운영 바이너리에 합쳐질 수 있다(§2-1 의존 — 게이트 ③ 이 못 본다 · 매니페스트 경고로만 막는다).

### 3-5. U4 — crate 삭제 · 게이트 · 문서

- **단위 안 순서:** ① 데몬 dev 줄과 다리 시험(`data_dir.rs` 시험 모듈) 삭제 → `rg engram_dashboard_discovery crates src-tauri` 가 crate 자신만. ★① 을 따로 커밋하면 `Cargo.lock` 도 그 커밋에 든다★ — lock 은 dev 의존도 패키지 의존 목록에 적으므로 데몬 항목에서 discovery 가 빠지고, CI 는 `--locked` 라 lock 을 빠뜨린 커밋부터 빨갛다 ② ★한 커밋★: 워크스페이스 멤버 줄 · crate 폴더 · `Cargo.lock` · gate 4 명단 discovery 두 줄(`:870` · `:882`) · async 반입 스텝 **교체**(`:1191-1212` → platform 게이트 ⑦ · §2-6) · `qa.md:73-74` 교체 — 대상이 사라지면 옛 스텝과 gate 4 가 그 커밋부터 FAIL 이다. `CLAUDE.md:222` · platform `src/lib.rs:54-59` · net `Cargo.toml:59-64` 의 새 게이트 가리킴도 같은 커밋이 깔끔하다(④ 로 미뤄도 빌드는 선다) ③ 이름 알파벳 게이트 — 메시징(`:527`)과 net gate 1(`:1087` 이름 · `:1091`)에서 `discovery` 를 빼고 사본(CLAUDE.md · `qa.md` · `docs/testing-strategy.md` · `docs/reference/architecture-overview.md:492` · net `src/lib.rs:59`)을 함께(§8 O2) ④ 문서 · 주석(§6 U4 몫).
- **수용 기준:** ① `rg -n "engram_dashboard_discovery|engram-dashboard-discovery" crates src-tauri .github CLAUDE.md .claude/skill-bindings docs/testing-strategy.md docs/reference/structure docs/reference/logging-conventions.md docs/reference/architecture-overview.md scripts` → 0 ② 결과 줄 −3 (§3-6) ③ CI 전 게이트 초록 — 새 `platform gate 7` 이 세 줄 다 PASS(agent 0 · net 기본 0 · net `server` ≥ 1) ④ `rg -n "discovery has no async-runtime ingress|discovery async 반입" .github CLAUDE.md .claude/skill-bindings crates` → 0(옛 게이트를 가리키는 서술이 남지 않았다) ⑤ GUI §5-3.
- **게이트:** `cargo fmt --check` · `cargo build` · `cargo test --workspace -- --test-threads=4`(§3-6 대조) · 새 게이트 세 명령을 로컬에서 한 번(`qa.md` 「CI와의 분담」 블록 — 기대값 위 ③) · CI(`--locked`).
- **위험:** 정규식 게이트는 0 기대라 `discovery` 를 빼지 않아도 초록이다 — 빠뜨려도 안 보인다(수용 기준 ① 의 rg 가 잰다) · 새 게이트의 패턴을 옛 스텝에서 옮기며 `^…␣` 앵커를 빠뜨리면 0 기대 줄이 ter(mio)s · tungstenite 같은 이름에 걸린다(옛 스텝 주석 `:1194-1195`) — 짝 줄은 그 반대(패턴이 아무것도 못 잡음)만 잡는다 · net 헤더의 순환 서술(`:31-50`)을 지우면 0-4 가 왜 그 crate 였는지가 사라진다(역사로 남기고 현재 서술만 고친다).

### 3-6. 단위마다 검증 — 회귀 수 대조

1-3 TRD §5-2 의 명령 그대로다(`cargo test --workspace -- --test-threads=4` 를 앞뒤로 돌려 결과 줄 수 · 통과 · 실패 · 무시를 견준다 · 줄 수가 줄면 타깃 소실 — 초록이어도 멈춘다. U4 의 −3 은 의도된 소실이다).

| 단위 | 결과 줄 | 통과 | 무시 | 분포 |
|---|---|---|---|---|
| U1 | 그대로 | +1(platform) +2(base) −2(discovery) = **+1**(제안 retry 시험을 넣으면 +2) | 그대로 | 프로브 시험 discovery → base |
| U2 | 그대로 | **+13** — 데몬 사본: 루트 4 · 프로브 2 · 작업 루트 · 설치 위치 5 · `layout` 2 | 그대로 | — |
| U3 | **+1**(`src-tauri/tests/stop_smoke.rs`) | **+52** — lib_unit: discovery 셸 몫 41(43 중 실 WMI 2 는 무시) · `layout` 셸 절반 1 · 같은 경로 1 · `stop.rs` 9 | **+5**(실 WMI 2 · stop_smoke 3) | — |
| U4 | **−3**(discovery 단위 · `stop_smoke` · Doc-tests) | **−64** — discovery 61(U1 뒤 63 중 무시 2 제외) · `layout` 2 · 데몬 다리 시험 1 | **−5** | — |
| 합 | −2 | +2(+3) | 0 | — |

**기준선은 U1 착수 직전에 잰다** — 마지막 기록(CLAUDE.md 「빌드·검증 명령」 = 결과 줄 59 · 3822 통과 · 30 무시, `cac9ee0`)은 1-1 · 1-3 · 2-1 머지로 낡았다. 이 TRD 는 돌려 보지 않았다. 셸 테스트 타깃은 다섯 → 여섯, lib_unit 은 +52 통과 · 무시 0 → 2 가 된다.

### 3-7. U1 착수 체크리스트

0. **전제 둘.** ① 새 ADR(§7 — 채번 · 링크 = `/adr`)을 박는다 — D2 가 ADR-0271 결정 2 의 한 조각을 고친다(CLAUDE.md 「설계 결정 기록」). ② 되돌릴 지점 — 이 TRD 를 로컬 커밋해 출발점을 만든다(코드 트리 = `6fa966a`).
1. **출발 수치** — §3-6 명령(터미널이 죽으면 `scripts/run-detached.ps1` — qa 바인딩 「분리 실행」).
2. **손댈 파일(정확히):** platform `src/env.rs`(함수 + 시험 · `// ADR-NNNN`) · `src/lib.rs`(입주자 줄의 env 서술) · daemon `src/lib.rs`(`:84` 주석 · `:101-118` 본문) · `Cargo.toml:84-86` 주석 · `src/bin/roundtrip_smoke.rs:902-911` · discovery `src/lib.rs`(`:67-77` 상수 둘 · `:152-238` · `:250-255` · `:289-291` · `:983-991` · 시험 `:1164-1236`) · base `src/writable.rs`(새 · `// ADR-0269` · `// ADR-NNNN`) · `src/lib.rs`(헤더 입주자 · 게이트 ③ · `pub mod writable;`) · `Cargo.toml` description · 루트 `Cargo.toml:13-14` · `ci.yml:653` · `qa.md:149` · `CLAUDE.md:251` 과 base · platform 항목 · `docs/testing-strategy.md:43` · `:50` · `:55` · `:169` · 메모 §4 · §11 실행 파일 줄(처리 표시 · 실제 포인터).
3. **순서:** §3-2 의 ①~⑤.
4. **게이트 · 수치:** §3-2 게이트 · 실 데몬 레인(ws_e2e `--ignored` · `--test-threads=4`) → §3-6 대조(결과 줄 그대로 · 통과 +1 또는 +2).
5. `/review code full` → `/qa standard` → 게이트 초록 뒤 커밋(`S21: refactor(platform|base): …` — 스텝 번호는 step-log 를 잇는다 · 끝에 Co-Authored-By 트레일러).

---

## 4. 의존 그래프 — 2-2 뒤

| crate | 직접 워크스페이스 의존(normal · build) 지금 | 2-2 뒤 |
|---|---|---|
| 셸(`engram-dashboard`) | base · command · discovery · net · protocol | base · command · net · **platform** · protocol (+ 서드파티 직접 `tungstenite`) |
| 셸 dev | + daemon | + daemon · platform(`test-support`) · tempfile |
| daemon | agent · base · command · discovery · messaging · net · platform · protocol | discovery 만 빠진다(U2 에 dev 로 · U4 에 삭제) |
| discovery | base · net · platform · protocol | 없음 |

- **platform 의 운영 소비자** — discovery 가 빠지고 셸이 직접 든다. 그래서 platform 시험 기능 게이트의 소비 crate 서술이 「agent · 셸」이 되고 「셸은 platform 을 직접 의존하지 않는다」(`ci.yml:747-753` · `CLAUDE.md` · `qa.md:258`)가 U3 에 거짓이 된다 — U3 이 고친다. 게이트 명령 자체는 그대로 선다(1-5 — dev 기능은 `-e normal,build` 로 안 센다 · agent 로 실측).
- **net 기본 feature 의 소비자** — discovery · 셸 → 셸 하나(`auth::AuthFrame` — `daemon_client/connection.rs` · 새 `stop.rs`). 셸은 tokio 를 지므로 소비자 쪽에서 지킬 것은 없다 — 그래도 net 자신의 기본 feature 무반입(`default = []` 결정)은 새 게이트가 net 을 대상으로 직접 잰다(§2-6 2).
- **ADR-0271 불변식 「셸의 운영 의존에 데몬 crate 가 없다」** — 셸 게이트 1 이 간접으로 잰다: 데몬이 agent 를 의존하는 동안 셸 → 데몬 normal 간선이 생기면 agent 줄이 뜬다(확실 — 그래프 논리).
- **사라지는 게이트:** 없다. ★단 덮임 하나가 빠진다★ — protocol 은 지금 discovery 경유로 async 반입 게이트에 덮였는데, 새 대상(agent · net)은 protocol 을 운영 의존으로 지지 않아 2-2 뒤 그 덮임 밖이다. 괜찮은 이유 = 2-2 뒤 protocol 의 소비자(데몬 · 셸 · net `server`)가 전부 async 라 protocol 에 async 런타임이 들어와도 조용히 번질 동기 소비자가 없다. **바뀌는 게이트:** async 반입 — 대상 discovery → agent · net 기본 = 0줄 + 짝 net `server` ≥ 1줄(D7 · U4 · platform 게이트 ⑦) · gate 4 명단(U3 +2 · U4 −2) · 메시징 · net gate 1 알파벳(U4) · base 게이트 ③ 알파벳(U1). **그대로:** base · platform 시험 기능 게이트(대상이 셸 그래프에 남는다 — base 직접 · platform 은 U3 부터 직접) · 셸 게이트 1 · platform 게이트 5 · 6 · net 게이트 2 · 3.

---

## 5. GUI 실측

절차(기동 인자 · 환경변수 · PID · teardown)는 `/qa` 바인딩 §full 이 갖는다. ★이번 측정 대상은 「셸이 데몬을 띄운다」라서 §full 의 격리 절차(데몬을 먼저 띄우고 `ENGRAM_DATA_DIR` 로 가른다)를 쓰지 않는다★ — WMI 로 뜬 데몬은 환경을 물려받지 않아 늘 `<저장소>\.engram-dev` 를 쓴다(override 주석 `:42-47`). 앱 로그는 `RUST_LOG=info`(`-EnvVars`) — 데몬은 그 값을 못 받아 기본 warn 이므로 데몬 쪽은 파일 존재와 `daemon.json` 으로 잰다.

### 5-0. 전제 (경고 — 사용자 확인)

★**사용자 확인(§8 O3)은 §5-2 실 레인 직전에 받는다 — 그 답 하나가 §5-2 · §5-1 · §5-3(1 의 GUI 재실측 · 2 의 릴리스 1회)을 함께 연다**★(리뷰 반영 메인 결정 — 실 레인의 실 WMI 시험이 GUI 보다 먼저 `.engram-dev` 와 그 자동 복원 프로필을 건드린다). U3 실행 순서 = 확인 → §5-2(살아 있는 데몬이 있으면 실 WMI 시험이 skip 하므로 앱보다 먼저) → §5-1.

1. **자동 복원 프로필** — 이 워크트리 `.engram-dev\daemon\state\agents.json` 에 `auto_restore: true` 가 1건 있다(실측 2026-10-07 · 셈만 봤다). 셸이 띄운 데몬도, 실 WMI 시험이 띄운 데몬도 그것을 실 cwd 에서 되살린다(둘 다 WMI 라 늘 `.engram-dev`) → §8 O3.
2. **살아 있는 개발 데몬** — 있으면 그것을 끄는 일이 그 자식 에이전트를 끝낸다. 측정 전 사용자 확인.
3. **데몬 exe** — `cargo build -p engram-dashboard-daemon`. 셸은 자기 형제(`sibling_exe`)를 첫 후보로 찾는다 — `build-client-shell.mjs` 가 낸 셸 exe 와 같은 폴더인지 본다.

### 5-1. U3 — 셸이 데몬을 띄운다

1. **전** — `.engram-dev` 1단 목록(지금 `daemon` · `logs`) · `logs\` 파일 수 · `daemon\run\daemon.json` 유무 · 개발 데몬 없음(tasklist + 파일 pid).
2. **기동** — §full 0) ~ 2)(`-EnvVars 'WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223','RUST_LOG=info'`). 부팅 연결(`daemon_connect` → `ensure_daemon`)이 데몬을 WMI 로 띄운다.
3. **판정** — 앱 로그에 「데몬 spawn」(`ensure_with` `:536`)과 「앱 로그 파일 결정 data_dir=<저장소>\.engram-dev」 · `.engram-dev\daemon\run\daemon.json` 에 새 pid · `.engram-dev\logs\daemon-*-<그 pid>.log` 가 생김 · 그 로그에 warn 「제어 평면 CLI 형제 exe 를 못 찾음 — CLI 입구 비활성(MCP 입구는 정상, ADR-0086 F1)」(데몬 `lib.rs:114-117` — `locate_send_exe` 는 fail-open 이라 데몬 기동만으로는 U1 의 `sibling_exe` 전환이 안 잰다)이 없다 · `node scripts/cdp.mjs eval "window.__TAURI__.core.invoke('daemon_status')"` → `alive:true` · 같은 pid. (선택) 그 pid 의 부모가 셸이 아니다 — `Get-CimInstance Win32_Process` 의 `ParentProcessId`.
4. **데이터 폴더 불변** — 1 과 견준다: 새로 생길 수 있는 것은 ADR-0264 배치 안의 것(셸이 설정 · 창 항목을 쓴 경우의 `shell\` · 루트 `ui-settings.json`)뿐 · `target\debug\data` · `src-tauri\.engram-dev` · `crates\*\.engram-dev` 없음.
5. **트레이 상태 · 끄기 · 켜기(명령 경로)** — `eval "window.__TAURI__.event.listen('daemon-status-changed', e => (window.__ds ||= []).push(e.payload)); 'ok'"` → `invoke('daemon_stop')`(끄기 fallback — `TaskKiller` → `kill_tree`) → pid 반환 · 3초(옵저버 주기 `tray/mod.rs:145`) 안에 `window.__ds` 끝이 `false` · `daemon_status.alive=false` → `invoke('daemon_start')`(ensure — WMI) → 새 pid · `__ds` 끝이 `true`. 트레이 메뉴 「데몬 끄기 · 켜기」 클릭은 같은 함수(`send_stop` · `ensure_daemon`)를 부른다 — 사람이 누르거나 미검.
6. **graceful 끄기(옮긴 `send_stop`)** — 맨 끝에 `invoke('quit_app')` → 앱 로그 「[tray] quit_app: 데몬 graceful stop 발사(best-effort)」의 `outcome` 이 `DaemonClosed` · 데몬 pid 사라짐(tasklist) · 앱 종료.

### 5-2. U3 — 실 레인(비GUI · 분리 실행 · §5-0 확인 뒤)

- `cargo test -p engram-dashboard --test stop_smoke -- --ignored --test-threads=1` — `send_stop` 이 실 데몬을 스스로 내리게 한다(데몬 exe 필요). 이 시험 자체는 `Command` 로 띄워 환경이 상속되므로 임시 폴더로 격리된다(`stop_smoke.rs:6-7` — `.engram-dev` 를 안 건드린다). 그래도 같은 레인이라 확인 뒤에 돈다.
- `cargo test -p engram-dashboard --test lib_unit -- --ignored real_wmi --test-threads=1` — ★`.engram-dev` 를 건드린다(백업 · 복원) · 살아 있는 데몬이 있으면 skip 이므로 앱을 띄우기 **전에**★. `--test-threads=1` 은 빼지 않는다 — 두 시험이 같은 `daemon.json` 백업과 단일 인스턴스를 나눠 쓴다(병렬이면 한쪽 백업이 다른 쪽 데몬의 기록을 담는다). D6 의 exe 도우미가 cwd 와 무관하게 데몬 exe 를 찾는지가 여기서 선다. 플래그 행렬의 기대(`None=0 · NEW_CONSOLE=0 · NO_WINDOW≠0`)는 1-3 U7 실측과 같아야 한다.

### 5-3. U4 — 완료 실측(ADR-0271 결정 7)

1. crate 를 지운 빌드로 5-1 의 2 ~ 4 를 다시 — 완료 기록.
2. (권장) 릴리스 1회 — `npm run tauri build -- --no-bundle`(4분대 · §full 경고) → 앱이 띄운 데몬이 `target\release\data\daemon\run\daemon.json` 에 발행. 릴리스 분기 본문은 시험이 못 재는 유일한 자리다(§8 O6). 같은 5-0 전제가 그 데이터 폴더에도 걸리고, **§5-0 의 사용자 확인 없이는 돌리지 않는다**.

---

## 6. 문서 후속 (이 TRD 는 고치지 않는다 — 각 단위가)

`rg -i discovery` 를 `docs/`(과정 기록 `docs/process/**` · ADR 본문 · 연구 `docs/research/**` · 핸드오프 제외) · `CLAUDE.md` · 바인딩 · 스크립트 · 코드 주석에 돌린 결과다. 개념 낱말로서의 「discovery」(셸 모듈 이름 · protocol `src/discovery.rs` · `DaemonDiscovery` 트레이트 · CLAUDE.md `:139` 의 「discovery·portfile 축」)는 그대로 참이라 뺐다.

1. **U1** — base `src/lib.rs:3-7` · `:51` · base `Cargo.toml` description · 루트 `Cargo.toml:13-14` · `CLAUDE.md:251` 과 「백엔드 모듈 맵」 base(입주자 일곱) · platform(`env` 서술) 항목 · `ci.yml:653` · `qa.md:149` · `docs/testing-strategy.md:43` · `:50` · `:55`(env 시험) · `:169` · 데몬 `Cargo.toml:84-86` · `lib.rs:84` 주석 · 메모 §4 「사실」 넷째 줄 · §11 「실행 파일 위치 계산이 두 곳」 줄(처리 표시 · 실제 세 곳).
2. **U2** — 데몬 `src/lib.rs:62-64` · `:470` · `control/priming.rs:233-234` · `control/mcp_config.rs:42` 주석 · `bin/engram.rs:337` 호출 자리의 `// ADR-0273` 앵커와 한 줄(CLI 가 데몬 lib 를 쓰는 유일한 줄 · 2-3 이 CLI 쪽 사본으로 바꾼다 — §2-1 · §7 6).
3. **U3** — `CLAUDE.md` 「백엔드 모듈 맵」 src-tauri 항목(데몬 찾기 · 띄우기 · 끄기 = `discovery` 모듈 · 루트 · `daemon.json` · `logs` 사본과 같은 경로 시험 — ADR-0271) · 「빌드·검증 명령」의 셸 시험 타깃 — ★프로세스를 만드는 줄이 셸에 처음 든다(실 WMI 시험의 `taskkill` `Command` · stop_smoke) · 단 전부 `#[ignore]` 라 기본 실행은 여전히 아무것도 안 띄운다★: `CLAUDE.md:231`(「셸 패키지의 테스트 타깃 다섯은 하나도 안 붙는다 — 프로세스를 만드는 줄 자체가 없다(`rg "Command::new|std::process|\.spawn\(\)" src-tauri/src/` → 0줄)」 → 여섯 · 그 rg 는 이제 0 이 아니다 — 시험 안 `#[ignore]` 분뿐) · `:239`(통합 타깃 명단에 `cargo test -p engram-dashboard --test stop_smoke` 실행 줄 — 기본 실행은 컴파일 + 무시 3 · `--ignored` 는 §5-2 의 `--test-threads=1`) · `:240`(lib_unit 줄 「이 스위트는 자식 프로세스를 하나도 안 띄운다」 → 「기본 실행은 안 띄운다 — `#[ignore]` 실 WMI 둘은 `--ignored real_wmi --test-threads=1`」) · `qa.md:20` · `:53`(「셸 패키지 타깃 다섯(2b~2f) … 프로세스를 만드는 줄이 없다」) · `:138`(2) 줄 끝의 같은 서술) · `:143`(2f 「실 자식 프로세스는 하나도 안 띄우므로」) · `:161-163`(2b~2f · 「다섯 줄」 → 2b~2g · 여섯) · 새 2g 줄(`--test stop_smoke`) · ★stop_smoke 에 CI 스텝을 따로 세우지 않는다(메인 결정 — §3-4 5)★ · platform 시험 기능 게이트 줄(소비 crate · 「셸은 platform 을 직접 의존하지 않고」) · `ci.yml:747-753` · `qa.md:258` · `:332`(override 주석 자리 → 셸 `discovery`) · `qa.md` 「CI와의 분담」 실 레인에 §5-2 두 명령(스레드 상한 포함 · §5-0 확인 전제) · `docs/testing-strategy.md` src-tauri 절(① 단위 · 「자식 프로세스를 하나도 안 띄운다」 · 실 레인) · `:55` · 셸 `src/lib.rs:11-12` · `tray/mod.rs:240-246` · `daemon_client/mod.rs:101-103` · `:115-116` · `commands/discovery.rs:161` · 메모 §11 의 `taskkill` 줄 · 실 WMI 줄(처리 표시).
4. **U4 — CLAUDE.md** — 「백엔드 모듈 맵」 discovery 항목(`:165`) 삭제 · daemon 항목에 「데이터 폴더 규칙 · `DataLayout` 데몬 몫 · 설치 위치(`data_dir` — ADR-0271)」 · `:207` `windows` 항목의 「discovery 에서 옮겨 왔고 … 의존이 없다」 · `:222` 「discovery async 반입」 → 새 대상(「async 반입(agent · net 기본 — platform 게이트 ⑦)」 · §2-6 — 지우지 않는다) · `:250`(base 가 링크되는 곳) · `:254` 메시징 정규식 · `:273` net gate 1. ★「플랫폼 중립」 절은 discovery 를 부르지 않는다(rg 실측) — 고칠 것 없음★(그 절이 가리키는 OS cfg 명단의 정본은 `ci.yml` 이고 U3 · U4 가 바꾼다). 「의존성」 절에는 `tungstenite` 가 없다 — 셸 직접 간선이 하나 느는 것은 커밋 본문 · step-log 로 보고한다(새 패키지 0).
5. **U4 — 바인딩 · 전략 문서** — `qa.md:73-74`(새 게이트 세 명령 · 기대값으로 교체 — §2-6) · `:148` · `:165` · `:242` · `:251` · `:258`(discovery 빼기) · `:267` · `:428` · `docs/testing-strategy.md:75` · `:80` · `:83` · `:93`(「stale discovery」 — 데몬 시험 이름이라 그대로 둬도 참) · `:130`.
6. **U4 — 코드 주석(ADR-0271 「영향」의 낡는 서술 포함)** — base `src/lib.rs:46` · `src/logging/mod.rs:15` · `:344`(배치 = 데몬 · 셸 각자의 `DataLayout`) · net `src/lib.rs:10` · `:31-50`(순환 서술 — 현재형만 고치고 0-4 의 역사는 남긴다) · `:59` · net `Cargo.toml:13` · `:25` · `:30` · `:45` · `:59-64`(벽 = 새 게이트의 net 기본 feature 줄 — §2-6) · `:75` · `:82` · net `src/instance.rs:195` · `src/auth.rs:12`(트레이 stop 경로 = 셸 `daemon_client::stop`) · platform `src/lib.rs:54-55`(벽 = 게이트 ⑦ · `:58-59` 번호 안내에 ⑦ — §2-6) · `src/process.rs:6`(소비자 = 셸) · `src/spawn.rs:303` · protocol `src/lib.rs:85-86` · `:104`(재는 시험의 자리 = 셸 `discovery`) · 데몬 `tests/ws_e2e.rs:2576` · `ci.yml:619` · `:806` · `:1358`.
7. **U4 — 참조 문서 · 스크립트** — `docs/reference/logging-conventions.md:3` · `:9` · `:13` · `docs/reference/architecture-overview.md:75` · `:100` · `:175-191`(그래프에서 discovery 노드 삭제 · 데몬 → base · 셸 → platform) · `:492` · `docs/reference/structure/agent-backend.md:241` 와 짝 `agent-backend.html:646`(같은 커밋) · `scripts/engram.mjs:25` · `:43`(정본 = 데몬 `data_dir::DataLayout::daemon_file` · 셸 사본은 같은 경로 시험) · `:63` · `scripts/rebuild-run-debug.bat:28` · `scripts/launch-detached.ps1:134` · `scripts/run-detached.ps1:145`.
8. **오케스트레이터** — ADR-0175 「영향」 의존 그래프 줄(`:54` 의 `discovery → …` · `daemon → … discovery`)은 ADR-0271 이 이미 Amends 로 걸었다 — 본문은 고치지 않는다 · ADR-0273 「맥락」의 「데몬 lib 를 한 줄도 쓰지 않는다」는 U2 부터 2-3 까지 거짓이다(CLI → 데몬 `data_dir::find_install_root` 한 줄) — 본문은 고치지 않고 새 ADR 이 링크로 적는다(§7 6) · `docs/tracking.md:49` · `docs/tracking-archive.md:11`(T-10 — ADR-0271 이 다시 연 것의 종결 표시) · step-log 착지 항목 · 메모 §10 2-2 착지 표시.
9. **날짜 박힌 스냅숏** — `docs/reference/architecture-map-notes.md` · `architecture-map-data/*.json` · `docs/reference/structure/session-path-ownership.md:813` 은 그 지도를 다시 뽑을 때.

---

## 7. ADR-NNNN 에 박을 것

> 채번 · 링크 · 도장 = `/adr`. 거부한 대안은 메인 · ADR 이 준 것만 옮긴다(CLAUDE.md 「결정 날조 금지」). `docs/decisions/` 는 다른 작업이 동시에 쓸 수 있어 이 TRD 는 손대지 않았다.

1. **ADR-0271 결정 2 의 한 조각 개정(D2)** — 「실행 파일 위치 계산(두 곳 → 한 곳) → 데몬」을 「→ platform `env`」로. 사유 = 셸이 데몬 crate 를 의존하지 않는다(ADR-0271 자신의 「열린 것」 · 「거부한 대안」 둘째) · 실제 사본이 세 벌이었다(§1-7). `find_install_root` 는 그대로 데몬.
2. **ADR-0271 「열린 것」 넷을 채운다(개정 아님)** — ① `logs\` = 셸 사본 + 같은 경로 시험(D3) ② 실행 파일 「한 곳」의 모양(위 1) ③ 쓰기 프로브 = base `writable` — ADR-0269 「입주시키지 않는 것」 항목이 적은 조건의 충족(그 항목에 링크) · ★입주를 둘째 사본이 생기기 전(같은 단계의 U1)으로 당긴 것과 그 이유 둘 — 임시 중복을 만들지 않는다 · 프로브 이름의 정적 카운터가 하나여야 한다(셸 경합 시험이 데몬 판 · 셸 판을 한 프로세스에서 섞는다 — §2-3)★ · 오류 타입을 걷는 시그니처 · 접두 값 유지와 그 판단(§2-3) ④ 2-2 ~ 3-2 의 `send_stop` 자리 = 셸 `daemon_client/stop.rs` 글자 그대로 · `tungstenite` 직접 줄 · 새 패키지 0(D1).
3. **같은 경로 시험의 범위와 한계(D5)** — 루트 세 환경 상태 · 릴리스 도우미 · `daemon.json` · `logs` · 경합 시험의 자리 · 환경 락(데몬 · 셸 각 하나 — §2-4) · 못 재는 것: 릴리스 분기 본문 · ★walk-up 표지 하나(`.git` · `[workspace]`)를 빠뜨린 셸 사본 — 이 저장소 루트에 둘 다 있어 같은 경로가 나온다. 표지 시험(`find_workspace_root_detects_*`)은 데몬에만 산다★ · ★셸 → (dev) 데몬 간선에 기댄다 — 그 간선을 걷거나 좁히는 날(작업 순서 2-4 이후) 두 시험의 자리를 다시 정한다★.
4. **async 반입 게이트 교체(D7)** — 대상 discovery → agent(동기 소비자 — base · command · platform 을 함께 덮는다) · net 기본 feature = 0줄 + 짝 net `server` ≥ 1줄(ADR-0278 꼴)을 platform 게이트 ⑦ 로 · 지우기만 하는 안을 버린 이유 = 그 게이트가 덤으로 지키던 platform 의 async 금지와 net `default = []` 축에 소비자 · 대상이 남는다(§8 O1) · 그것을 벽으로 가리키던 net · platform 서술은 새 게이트를 가리킨다.
5. **crate 삭제 시점 = 2-2(D1)** · 완료 실측 결과(D10 · 착지 때 채운다).
6. **engram CLI → 데몬 lib 임시 간선(U2 ~ 2-3)** — bin `engram.rs` 가 `engram_dashboard_daemon::data_dir::find_install_root` 를 부른다. ADR-0273 「맥락」의 「데몬 lib 를 한 줄도 쓰지 않는다」가 그 사이 거짓이 된다(링크) · 2-3 이 이 간선을 CLI 쪽 사본으로 바꾼다(ADR-0273 결정 5) · 앵커 `// ADR-0273` 이 그 줄에 있다.
- **코드 앵커** — `// ADR-0271`: 데몬 `data_dir.rs` 머리(루트 규칙 정본) · 셸 `discovery/layout.rs` 머리(사본)와 같은 경로 시험 · 셸 `daemon_client/stop.rs` 머리(임시 거처). `// ADR-NNNN`: platform `env::sibling_exe` · base `writable.rs` 머리(`// ADR-0269` 와 함께). `// ADR-0264`: 두 `DataLayout`. `// ADR-0273`: bin `engram.rs` 의 `find_install_root` 호출(위 6).

---

## 8. 열린 것

결정 D1~D10 은 다시 열지 않았다 — 단 D7 은 2판에서 메인이 리뷰 반영으로 「삭제」를 「교체」로 넓혔다(O1). 아래는 결정을 그대로 둔 채 사실로 드러난 것 · 이 TRD 가 정하지 않은 것이다.

**메인 처리(사용자 위임 2026-10-07 아래의 메인 결정 · 2판에서 리뷰 반영으로 O1 · O3 갱신):** O1 = **옛 게이트를 지우지 않고 갈아 끼운다**(1판의 「platform (나) · net (가)」를 대신한다) — U4 가 crate 삭제 커밋에서 같은 꼴 스텝 `platform gate 7` 을 세운다: agent · net(기본 feature) = 0줄 + 짝 net `--features server` ≥ 1줄 · `qa.md:73-74` · `CLAUDE.md:222` 는 새 대상으로 · platform 헤더 · net `Cargo.toml` 서술은 그 스텝을 가리킨다(§2-6) · O2 = U4 에서 `net` · `transport` 를 더한다 · O3 = **사용자 확인 대기** — U3 의 **§5-2 실 레인 직전**에 묻는다(실 WMI 시험이 GUI 보다 먼저 `.engram-dev` 를 건드린다 · 그 답이 §5-1 과 §5-3 1 · 2 도 연다 · 그 전 단위는 진행) · O4 = 결정 그대로 · O5 = 그대로(머리 주석이 정본) · O6 = 릴리스 1회 실측을 §5-3 2 대로 한다.

- **O1 — async 반입 게이트를 지우기만 하면 기계 벽을 잃는 서술 둘(사실 · 메인 판단 — 2판에서 닫힘).** net `Cargo.toml:59-64` 는 기본 feature 의 조용한 반입을 「실제로 잡는 것은 소비자 쪽 트리 실측 — discovery 의 게이트」라 적고, platform `src/lib.rs:54-55` 는 async 런타임 금지의 근거로 같은 게이트를 든다. 지금 둘 다 0줄이다(실측 §1-5). ★D7 의 이유(「셸이 이미 tokio 를 지므로 이을 목적이 없다」)는 discovery 자신의 성질에 대해서는 맞지만, 그 게이트가 덤으로 지키던 platform 의 성질에는 지킬 소비자가 남는다★ — agent 는 tokio 없는 동기 crate 이고 platform 을 운영 의존으로 진다(실측 §1-5). platform 에 async 런타임이 들면 agent 로 조용히 번지고, D7 뒤에는 그것을 잡는 게이트가 없다. 1판 선택지: (가) 서술만 고친다 (나) 같은 꼴의 0줄 게이트를 `-p engram-dashboard-platform` 대상으로 옮긴다 — 1판 제안은 platform (나) · net (가). **2판 메인 결정(리뷰 반영)은 그보다 넓다** — 대상은 platform 이 아니라 **agent**(실제 동기 소비자 — platform 단독 대상은 agent 가 다른 경로로 async 를 들이는 것을 못 본다 · agent 하나로 base · command · platform 이 함께 덮인다)이고, net 은 서술만이 아니라 **기본 feature 로 직접** 잰다(옛 스텝 주석 `:1191-1195` 이 적은 축을 그대로 잇는다), 그리고 패턴이 눈멀지 않게 **짝**(net `server` ≥ 1줄)을 둔다. D7 은 「삭제」에서 「교체」로 바뀌었다(§2 표 · §2-6).
- **O2 — 메시징 이름 게이트에 `net` · `transport` 가 없다(메모 §11 · 「다음에 그 게이트를 손볼 때 사본 다섯을 한 번에」).** U4 가 바로 그 게이트를 손본다(`discovery` 빼기) → 같은 편집에서 두 이름을 더하자고 제안한다(messaging 소스에 두 이름 0건 — 2026-10-05 실측 · 더해도 초록). 메인 확인.
- **O3 — 실 레인 · GUI 실측이 사용자 데이터를 건드린다.** 셸이 띄운 데몬과 실 WMI 시험(§5-2)이 띄운 데몬은 늘 `<저장소>\.engram-dev` 를 쓰고(WMI · 환경 미상속), 그 명부의 `auto_restore: true` 1건이 실 cwd 에서 되살아난다(§5-0). 실 WMI 시험이 GUI 보다 먼저 돌므로 확인은 §5-2 앞이다. 격리 우회 — exe 둘을 저장소 밖 폴더에 복사해 띄우면 루트가 exe 옆 `.engram-dev` 로 갈리지만 그것은 루트 찾기의 다른 갈래(작업 루트를 못 찾은 경우)라 「데이터 폴더 불변」을 재지 못한다. → U3 §5-2 실 레인 전에 사용자 확인(되살아나도 되나 · 측정 동안 그 프로필을 끌 것인가 — 명부를 이 작업이 고치지 않는다). 같은 답이 §5-3 의 GUI 재실측 · 릴리스 1회도 연다.
- **O4 — 프로브 파일 접두 `.engram-write-probe-` 의 제품 이름을 도메인 지식으로 보지 않았다(D4 · §2-3 ②).** 리뷰가 다르게 보면 접두를 인자로 받는 모양으로 바꾼다(두 호출자가 같은 값을 넘긴다). 불가능은 아니다 — 결정 그대로 진행.
- **O5 — 셸 lib_unit 이 프로세스를 만드는 시험(`#[ignore]` 실 WMI 둘 — WMI 띄우기 · `taskkill`)을 처음 품는다.** 기본 실행은 그대로 0 이라 `--test-threads` 판정은 안 바뀐다. `--ignored` 로 돌릴 때만 해당(`--test-threads=1` — §5-2) — 그 시험 머리 주석이 정본이 된다(§6 3).
- **O6 — 릴리스 분기(`not(debug_assertions)`)의 루트 찾기 본문은 두 벌 다 시험이 대조하지 못한다.** §5-3 2 의 릴리스 1회 실측을 권한다(4분대). 안 하면 §9 에 남긴다.

---

## 9. 미검

- **기준선 회귀 수** — 이 판에서 돌리지 않았다(§3-6 · U1 착수 직전).
- **`--locked` 아래 lock 변화** — U2(normal → dev)가 lock 을 안 바꾼다 · U3 의 셸 항목 이름 목록만 바뀐다는 것은 lock 구조 독해이고 돌려 보지 않았다(새 `[[package]]` 0 의 근거인 tungstenite · tempfile 존재는 실측).
- **셸에서 실 WMI 시험이 cwd 후보로 우연히 풀린다는 것**(§1-7) — cargo 가 시험을 패키지 루트에서 돌린다는 동작에 기댄 코드 읽기다.
- **`cargo test -p engram-dashboard-daemon` 이 `roundtrip-smoke` bin 을 함께 짓는지** — 매니페스트 주석(자기 dev 의존이 `test-harness` 를 켠다) 독해 · 그래서 U1 게이트에 `--bin roundtrip-smoke` 빌드를 따로 넣었다.
- **GUI 전 단계**(§5) — 데몬 로그 파일 이름에 pid 가 들어가는 것(`<종류>-<UTC>-<pid>.log` — base `logging` 머리 서술) · `daemon-status-changed` 를 CDP 로 듣는 수법 · 트레이 메뉴 클릭(사람) · `quit_app` 의 `outcome` 이 로그에 찍히는 꼴(`tray/actions.rs:183-187` 독해).
- **실 WMI 플래그 행렬이 셸에서 같은 값을 내는지**(§5-2) — 1-3 U7 의 실측은 discovery 시험 바이너리였다.
- **릴리스 분기**(O6).
