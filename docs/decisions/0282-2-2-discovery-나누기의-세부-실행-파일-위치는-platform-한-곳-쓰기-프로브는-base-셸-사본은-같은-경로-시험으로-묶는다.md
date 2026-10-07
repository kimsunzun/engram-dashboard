# ADR-0282: 2-2 discovery 나누기의 세부 — 실행 파일 위치는 platform 한 곳, 쓰기 프로브는 base, 셸 사본은 같은 경로 시험으로 묶는다

- 상태: 확정 (2026-10-07, 근거: 사용자 위임 2026-10-07 아래의 메인 결정 · 2-2 착지 2026-10-07 (U1 `a0ec7f4` · U2 `3c9bf33` · U3 `b42e139` · U4 `d2f9822`) · 정본 설계 = `docs/process/S21-crate-boundaries/trd-2-2-discovery-split.md` §7)
- 관련: ADR-0269(결정 7 입주 조건 — 아래 결정 2) · Amends ADR-0271 (결정 2의 실행 파일 위치 계산 자리) · ADR-0273(engram CLI → 데몬 lib 임시 간선 — 아래 「영향」)

## 맥락

ADR-0271 이 discovery crate 를 나눠 데몬 몫은 데몬에, 셸 몫은 셸에 두기로 했고(결정 1 ~ 4 · 착수 = 작업 순서 2-2 · 결정 7), 내부 배치 넷을 「열린 것」으로 남겼다 — `logs\` 자리 · 실행 파일 위치 「두 곳 → 한 곳」의 모양 · 쓰기 검사 도우미(`retry_if_vanished` · `probe_write_in`) · 2-2 와 3-2 사이 `send_stop` 의 자리. 같은 ADR 은 `ci.yml` 의 `Gate: discovery has no async-runtime ingress` 를 「대상 crate 가 사라진다」며 사라지는 게이트에 넣었다.

TRD 2-2 의 현황 실측(`6fa966a`)이 그 그림과 갈린 곳:

- **실행 파일 위치 계산은 두 곳이 아니라 세 곳이었다** — 데몬 `locate_send_exe` · discovery `locate_daemon_exe` 의 첫 후보 · 데몬 `src/bin/roundtrip_smoke.rs` 의 `sibling_send_exe`(`.exe` 를 `cfg!(windows)` 로 손으로 붙였다). ADR-0271 결정 2 는 그 계산을 데몬으로 보냈는데, 셸은 데몬 crate 를 의존하지 못한다(ADR-0271 불변식 · 「거부한 대안」 둘째) — 데몬에 모으면 셸의 데몬 찾기가 그것을 부를 수 없다.
- **쓰기 프로브 도우미는 나누면 데몬 · 셸 두 곳이 쓴다** — ADR-0269 「입주시키지 않는 것」이 스스로 적은 입주 조건이다. 프로브 이름의 정적 카운터(`probe_path`)는 셸 경합 시험이 데몬 판과 셸 판을 한 프로세스에서 섞어 부르므로 하나여야 한다.
- **async 반입 게이트가 덤으로 지키던 성질 둘에 지킬 대상이 남는다** — agent 는 tokio 없는 동기 crate 이면서 platform 을 운영 의존으로 지고(platform 헤더가 async 런타임 금지의 벽으로 이 게이트를 든다), net `Cargo.toml` 은 기본 feature 의 조용한 반입을 실제로 잡는 것이 이 게이트라고 적었다.
- **discovery 의 실 WMI `#[ignore]` 시험 둘은 데몬 exe 를 운영 후보로 찾아 crate 폴더 cwd 에서 `ExeNotFound` 로 졌다**(메모 §11). 셸로 옮기면 cwd 가 `src-tauri` 라 셋째 후보가 저장소 `target\debug` 를 가리켜 우연히 풀린다(코드 읽기).

## 결정
(사용자 위임 2026-10-07 아래의 메인 결정 · crate 삭제 시점(2-2 U4)과 완료 실측은 아래 「근거」 · 「영향」에 적는다)

1. **실행 파일 위치 계산은 platform `env::sibling_exe` 한 곳이다** — ADR-0271 결정 2 가 이것을 「데몬으로」 두었던 자리를 고친다. 데몬의 CLI 찾기 · 셸의 데몬 찾기 · `roundtrip_smoke` 가 모두 그것을 부른다(데몬과 셸은 서로의 코드를 못 나눠 쓰고, 둘 다 platform 에는 닿는다).
2. **쓰기 프로브 도우미는 base `writable` 이다 — 입주를 2-2 U1 로 당긴다.** ADR-0269 결정 7 의 「사본 둘」 조건은 같은 단계 안의 U2 · U3 에서 성립한다. 당긴 이유 = 임시 사본을 만들지 않는다 · 프로브 이름 카운터(`probe_path` 의 static)를 데몬 판과 셸 판이 한 프로세스에서 공유해야 이름이 안 겹친다. 도메인 지식 0 은 시그니처로 맞췄다 — base 판 `retry_if_vanished` 는 `io::Result` 를 돌려주고, 두 호출자가 `map_err` 로 자기 오류(데몬 `DataDirUnwritable` · 셸 `DiscoveryError::DataDirUnwritable`)로 접는다(판정 표 — `Ok` · `NotFound` 뒤 한 번 재시도 · 그 밖 오류 — 는 그대로 · TRD §2-3 ①). 옮기면서 그 정리 경고의 tracing target 이 `engram_dashboard_discovery` → `engram_dashboard_base::writable` 로 바뀐다(문구 · 레벨 그대로 · 저장소 안에 그 target 을 쓰는 `RUST_LOG` 지시 0건).
3. **같은 경로 시험이 셸 사본을 데몬 정본에 묶는다** — 셸이 데몬 crate 를 운영 의존하지 않으므로(ADR-0271) 셸은 루트 찾기 · `daemon.json` 자리 · `logs\` 를 사본으로 갖고, 셸 → (dev) 데몬 간선으로 그 시험을 돌린다. 재는 것 = 루트(환경 변수 미설정 · 빈 값 · 경로 세 상태 — 변수 이름은 리터럴) · `release_data_dir` · `daemon_file` · `logs_dir`. 못 재는 것 = 루트 규칙의 release 분기 본문(시험은 늘 debug — release 1회 실측이 덮는다) · walk-up 표지 하나(`.git` · `[workspace]`)를 빠뜨린 셸 사본(이 저장소 루트에 둘 다 있다 — 표지 시험은 데몬에만 산다). 셸 → (dev) 데몬 간선을 걷거나 좁히는 날(작업 순서 2-4 이후) 이 시험과 경합 시험의 자리를 다시 정한다.
4. **환경 락은 crate 마다 하나** — `ENGRAM_DATA_DIR` 을 바꾸거나 읽는 시험은 데몬 `data_dir` 의 락 하나 · 셸 `discovery` 의 락 하나 아래에 있다(실 WMI `#[ignore]` 둘도 셸 락을 쥔다). 그 실 WMI 둘은 데몬 exe 를 시험 exe 위치에서 찾는다(운영 `locate_daemon_exe` 의 후보는 그대로).
5. **`send_stop` 의 2-2 ~ 3-2 자리 = 셸 `daemon_client/stop.rs` 글자 그대로** — 3-2(transport 셸 부착)가 다시 쓴다(ADR-0271 결정 5). 그 때문에 `pub(crate)` 로 연 discovery 내부(`check_acceptable` · `AcceptCheck` · `FileReader` 와 그 `path` · `RealLiveness`)는 discovery 밖에서는 stop.rs 만 쓴다. 셸에 `tungstenite` 직접 줄이 들지만 같은 판이 이미 lock 에 있어 새 패키지는 0. 옮긴 discovery · stop 코드의 로그 target 이 `engram_dashboard_discovery` → `engram_dashboard_lib::discovery` · `engram_dashboard_lib::daemon_client::stop` 으로 바뀐다(문구 · 레벨 그대로 · 그 target 을 쓰는 지시 0건).
6. **discovery 의 async 반입 게이트는 지우지 않고 platform 게이트 ⑦ 로 바꾼다** — 대상 = agent(`-e normal` · 동기 소비자 — base · command · platform 을 함께 덮는다) 0줄 · net 기본 feature 0줄(net `default = []` 의 성질) · 짝 = net `--features server` 1줄 이상(ADR-0278 꼴 — 패턴이 깨지면 짝이 빨개진다). 지우기만 하는 안을 버린 이유 = 그 게이트가 덤으로 지키던 platform 의 async 금지(동기 소비자 agent 가 남는다)와 net 기본 feature 축에 대상이 남는다. protocol 은 그 덮임에서 빠진다 — 2-2 뒤 소비자(데몬 · 셸 · net `server`)가 전부 async 다. 정본 = `ci.yml` 의 그 스텝 · platform `src/lib.rs` 헤더 게이트 ⑦. ★이 결정이 ADR-0271 의 두 문장을 고친다★ — 「거부한 대안」 첫 항목의 「나눈 뒤 지킬 대상이 없다」와 「영향」의 「사라지는 게이트 — `Gate: discovery has no async-runtime ingress`」(그 ADR 상태줄의 부분 폐기 도장은 결정 2 몫만 적혀 있다 — 같은 두 ADR 사이에 조항을 더하는 길이 서기 스크립트에 없다).

## 거부한 대안

- **실행 파일 위치의 「한 곳」을 데몬에 둔다(ADR-0271 결정 2 의 글자 — 결정 1).** 기각 = 셸은 데몬 crate 를 운영 의존하지 않으므로(ADR-0271 불변식) 셸의 데몬 찾기가 그것을 부를 수 없어 한 곳이 되지 않는다. 데몬 · 셸 둘 다 닿는 아래가 platform 이고 `exe_file_name` 이 이미 거기 산다(TRD D2 — ADR-0271 「열린 것」이 「현재 exe 옆 규칙 · `.exe` 붙이기는 platform 몫」으로 이미 가리켰다).
- **쓰기 프로브 도우미를 데몬 · 셸이 각자 사본으로 갖거나, 사본 둘이 생긴 뒤(U2 · U3)에 base 로 옮긴다(결정 2).** 기각 = ADR-0269 결정 7(지금 여러 곳에서 쓰이면 base) · 사본이 잠시라도 둘이 되는 중간 상태를 만들지 않는다 · 사본마다 정적 카운터가 따로면, 셸 경합 시험이 한 프로세스에서 두 판을 섞어 부를 때 같은 pid · 같은 번호의 프로브 이름이 겹친다(`create_new` 가 `AlreadyExists` 를 만나 남의 프로브를 지우고 다시 만든다 — 거짓 실패의 씨앗)(TRD §2-3).
- **프로브 파일 접두를 인자로 받는 모양(결정 2).** 택하지 않았다 — 접두 `.engram-write-probe-` 는 제품 이름이지 에이전트 런타임 · wire · 데몬 살림 지식이 아니다(base `logging` 의 로그 파일 이름 규약과 같은 부류로 보고 값 그대로 base 상수로 옮겼다). 두 호출자가 같은 값을 넘길 뿐이다(TRD §2-3 ② · §8 O4).
- **셸의 `daemon.json` 사본이 net `portfile::DAEMON_FILE` 을 빌린다(결정 3).** 기각 = net 은 3-3 에서 셸을 떠난다(TRD §2-4).
- **discovery 의 루트 시험을 제 락째 데몬으로 옮긴다 — 락 둘(결정 4).** 기각 = 한 시험 바이너리에서 락 둘이 같은 `ENGRAM_DATA_DIR` 를 짓밟는다. 데몬 `lib.rs` 의 `ENV_LOCK` 을 지우고 `data_dir` 시험 모듈의 락 하나로 모았다(TRD §2-4 — 리뷰 반영 메인 결정).
- **실 WMI 시험이 풀리도록 운영 `locate_daemon_exe` 의 후보를 고친다 · 셸 cwd 로 우연히 풀리는 것에 기댄다 · `sibling_exe` 를 쓴다(결정 4).** 기각 = 운영 동작을 시험 편의로 바꾸지 않는다(TRD D6) · 그 우연은 cargo 가 시험을 패키지 루트에서 돌린다는 동작에 기댄 코드 읽기다(TRD §1-7 · §9) · 시험 exe 는 `deps\` 에 있어 `sibling_exe` 가 맞지 않는다(TRD §2-5).
- **async 반입 게이트를 지우기만 한다(ADR-0271 「영향」의 「사라지는 게이트」 — 결정 6).** 기각 = 결정 6 의 이유(동기 소비자 agent 와 net 기본 feature 축에 대상이 남는다). TRD 1판의 두 안 — (가) 가리키는 서술만 고친다 · (나) 같은 꼴의 0줄 게이트를 platform 대상으로 옮긴다(1판 제안 = platform 은 (나) · net 은 (가)) — 도 2판에서 넓혔다: platform 단독 대상은 agent 가 다른 경로로 async 를 들이는 것을 못 보고(agent 하나로 base · command · platform 이 함께 덮인다), net 은 서술만이 아니라 기본 feature 로 직접 잰다(옛 스텝 주석이 적은 축을 그대로 잇는다). 짝이 없으면 패턴이 깨질 때 0 기대 줄 둘이 눈먼 채 통과한다(TRD §8 O1 — 리뷰 반영 메인 결정).
- **셸 `tests/stop_smoke.rs` 에 CI 스텝을 따로 세운다(결정 5).** 기각 = 시험 셋이 전부 `#[ignore]` · `#![cfg(windows)]` · 실 데몬 exe 를 띄우는 레인이라 CI 기본 실행에서 돌 것이 없고, 컴파일은 CI 워크스페이스 회귀가 이미 덮는다(TRD §3-4 5 — 메인 결정). 실행은 로컬 실 레인이다.
- **`send_stop` 의 임시 자리와 crate 삭제 시점(2-2)에는 TRD 가 저울질한 다른 안이 없다** — 지금 다시 쓰는 것만 3-2 로 미뤘다(ADR-0271 결정 5 · TRD 「지금 하지 않는 것」).

## 근거

- **TRD 2-2 2판**(`8939bdd`) — `/review trd full` 2라운드(codex blind FIX → PASS · Claude doc-aware FIX → FIX, 경미 7건 반영 · 3차 생략). 리뷰 반영 메인 결정 = 게이트 「삭제」 → 「교체」(TRD §8 O1) · 환경 락 하나(TRD §2-4) · `stop.rs` 가 쓰는 discovery 내부의 가시성(TRD §2-1).
- **현황 실측(`6fa966a`)** — 실행 파일 위치 세 벌(TRD §1-3) · `cargo tree`(TRD §1-5): `tungstenite v0.26.2` 는 Cargo.lock 에 하나뿐이고 셸 normal 그래프에 이미 있다(`tokio-tungstenite` 경유) → 셸 직접 줄의 새 패키지 0 · agent · net 기본 feature 의 async 런타임 0줄 · net `--features server` 6줄(TRD §2-6).
- **착지(2026-10-07)** — 단위마다 `/review code full`(codex + Claude) PASS · `/qa`(U1 · U2 standard · U3 · U4 full). 워크스페이스 회귀 = 시작 결과 줄 61 · 3997 통과 · 0 실패 · 31 무시 → U4 뒤 59 · 4000 · 0 · 31(TRD §3-6 과 갈린 곳 = U2 +14(TRD +13 — 리뷰에서 다리 시험 하나를 더했다) · U4 −65(TRD −64 — 다리 시험 둘이 함께 빠진다)) · 셸 lib_unit 518 → 570(무시 2 = 실 WMI). `platform gate 7` CI 블록을 로컬에서 돌려 PASS(agent 0 · net 기본 0 · net `server` 6). 실 WMI 2/2(`daemon.json` 복원 확인 · 플래그 행렬 `NO_WINDOW` ≠ 0)(U3 커밋 본문). 셸 실 레인 `stop_smoke` 의 `send_stop_makes_real_daemon_self_exit` 는 진다 — 결과가 `DaemonClosed` 대신 `Timeout` 인 기존 결함이다(`73ecdb8` 부터 · 2-2 무관 · `docs/tracking.md` T-48).
- **완료 실측(ADR-0271 결정 7 · TRD D10)** — debug GUI: 셸이 WMI 로 데몬을 띄웠다(부모 = WmiPrvSE) · 데이터 폴더 `.engram-dev` 불변 · `daemon_stop` / `daemon_start` 명령 경로 정상 · 마지막 `quit_app` 의 `outcome` 은 `Timeout`(TRD §5-1 6 의 기대 = `DaemonClosed` — 위 T-48 과 같은 결함 · 데몬은 내려갔다). release 1회: 데이터 = `target\release\data` · `daemon.json` = `<data>\daemon\run\daemon.json`(WebView2 프로필은 `WEBVIEW2_USER_DATA_FOLDER` 로 격리). 트레이 메뉴 클릭은 재지 않았다(사람 클릭 없음).

## 영향 / 불변식

- **「현재 exe 폴더 + OS 확장자」 규칙은 platform `env::sibling_exe` 에만 있다** — 존재는 보지 않는다(존재 판정 뒤 하는 일이 부르는 쪽마다 다르다 — TRD §2-2). 새 「exe 옆 실행 파일」 계산은 이것을 부른다. `find_install_root`(exe 에서 위로 걷는 다른 규칙)는 데몬 `data_dir` 에 그대로다.
- **쓰기 프로브는 base `writable` 하나다** — base 입주자 일곱 · base 게이트 ③ 의 이름 목록에 `writable`(사본 여섯). 접두 값을 바꾸면 소비자 시험의 잔여물 셈이 눈먼다(TRD §3-2 위험). ADR-0269 「입주시키지 않는 것」의 `retry_if_vanished` · `probe_write_in` 항목은 그 조건의 충족이지 개정이 아니다.
- **루트 규칙 정본 = 데몬 `crates/engram-dashboard-daemon/src/data_dir.rs`** · 셸 사본 = `src-tauri/src/discovery/layout.rs` — 같은 경로 시험이 둘을 묶는다(앵커 `// ADR-0271`). 그 시험은 셸 → (dev) 데몬 간선에 기댄다(결정 3).
- **`send_stop` = `src-tauri/src/daemon_client/stop.rs`(3-2 까지)** — `pub(crate)` 로 연 discovery 내부를 셸의 다른 모듈이 쓰기 시작하면 3-2 의 다시 쓰기가 넓어진다(TRD §3-4 위험). 셸의 `tungstenite` 직접 줄도 3-2 가 걷는다.
- **셸 dev 의존의 platform `test-support`** — `--all-targets` 처럼 lib 과 시험 타깃을 한 cargo 호출로 지으면 셸 운영 바이너리로 합쳐질 수 있다. platform 시험 기능 게이트는 그 꼴을 못 보고 매니페스트 경고로만 막는다(TRD §2-1 · §3-4 위험).
- **async 반입의 벽 = `ci.yml` 의 `platform gate 7`**(qa 바인딩 「CI와의 분담」 · platform `src/lib.rs` 헤더 게이트 ⑦ · net `Cargo.toml` 이 그것을 가리킨다). protocol 은 그 덮임 밖이다(결정 6).
- **discovery crate 는 2-2 U4(`d2f9822`)에서 지워졌다**(TRD D1 · 워크스페이스 멤버 10). CLAUDE.md 「백엔드 모듈 맵」에서 discovery 항목이 빠지고 daemon 항목이 `data_dir` 을 싣는다.
- **ADR-0271 을 고친다** — 결정 2 의 실행 파일 위치 자리(그 ADR 상태줄의 부분 폐기 도장) · 결정 6 이 적은 두 문장(본문에만 — 도장 미반영, 사용자 결정 대기).
- **옛 기록이 discovery crate 를 가리키면 이렇게 읽는다**(옛 ADR 본문은 고치지 않는다 · 찾는 법 = `git grep -n "engram-dashboard-discovery\|engram_dashboard_discovery" docs/decisions`):
  - `crates/engram-dashboard-discovery/src/lib.rs` → 데몬 찾기 · 띄우기 · 상태 · 끄기 fallback · 띄우기 전 사전 점검이면 셸 `src-tauri/src/discovery/mod.rs`, 루트 규칙 · 폴더를 만드는 쓰기 확인 · 설치 위치면 데몬 `crates/engram-dashboard-daemon/src/data_dir.rs`(셸의 루트 사본은 `src-tauri/src/discovery/layout.rs`).
  - `send_stop` 묶음 → `src-tauri/src/daemon_client/stop.rs` · 쓰기 프로브 도우미 → base `writable` · WMI 띄우기의 원시 호출은 1-3 에서 이미 platform `spawn` 으로 갔다.
  - `crates/engram-dashboard-discovery/src/layout.rs`(`DataLayout`) → 데몬 몫은 데몬 `data_dir.rs` · 셸 몫(과 `daemon.json` · `logs\` 사본)은 셸 `src-tauri/src/discovery/layout.rs`.
  - `crates/engram-dashboard-discovery/tests/stop_smoke.rs` → `src-tauri/tests/stop_smoke.rs`.
  - 예(전수 아님): `check_acceptable`(ADR-0196 · ADR-0178 「영향」의 discovery 「버전 불일치 시 데몬 거부」 경로 항목) → 셸 `src-tauri/src/discovery/mod.rs` · ADR-0230 의 `stop_smoke.rs` 포인터 → `src-tauri/tests/stop_smoke.rs` · 아래 CLI 간선.
- **engram CLI → 데몬 lib 임시 간선(U2 ~ 2-3)** — bin `engram.rs` 가 `engram_dashboard_daemon::data_dir::find_install_root` 를 부른다. ADR-0273 「맥락」의 「데몬 lib 를 한 줄도 쓰지 않는다」는 U2(`3c9bf33`)부터 2-3 까지 거짓이다. 2-3 이 이 간선을 CLI 쪽 사본으로 바꾼다(ADR-0273 결정 5) — 앵커 `// ADR-0273` 이 그 호출 줄에 있다.
