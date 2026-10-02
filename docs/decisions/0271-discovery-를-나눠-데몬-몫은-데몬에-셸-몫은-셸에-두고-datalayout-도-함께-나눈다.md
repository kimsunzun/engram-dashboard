# ADR-0271: discovery 를 나눠 데몬 몫은 데몬에 셸 몫은 셸에 두고 DataLayout 도 함께 나눈다

- 상태: 확정 (2026-10-02, 근거: 사용자 결정 2026-10-02 (`docs/refactoring/architecture-discussion-2026-09-26.md` 결정 후보 6 · §11 의 ADR-0264 충돌 항목에 대한 답) + 코드 대조 2026-10-02 (`d5ac725`))
- 관련: Amends ADR-0024 (데이터 위치 구현 줄의 discovery 단일 출처) · Amends ADR-0264 (결정 4의 discovery DataLayout 단일 출처) · `docs/tracking-archive.md` T-10(discovery 통합 — 종결: 안 한다, 사용자 결정 2026-08-26 · 이 결정이 다시 연다) · ADR-0029(셸 = 데몬 클라이언트) · ADR-0134(데이터 루트 위치 — 그대로) · ADR-0135(클라이언트 사전 점검 · 잠금 파일 = 접속 파일) · ADR-0175(`base` 셋째 입주자) · ADR-0266(WMI 띄우기 → platform) · ADR-0269(`Clock` → base · 입주 규칙) · ADR-0270(클라는 agent 를 모른다) · 결정 후보 7(transport TRD 때 박는다)(net 걷기) · ADR-0273(engram CLI 의 설치 위치 사본) · 메모 §5(애매한 것은 일단 데몬으로) · `crates/engram-dashboard-discovery/src/lib.rs` · `crates/engram-dashboard-discovery/src/layout.rs` · step-log S21 · Amends ADR-0175 (영향의 의존 그래프 중 discovery 가 든 줄)

## 맥락

discovery 가 생긴 이유는 ADR-0024 다 — 데몬이 데이터 폴더의 `daemon.json` 에 포트 · 토큰 · PID 를 적고 클라이언트가 같은 파일을 읽어 접속 · 인증하므로 양쪽이 **같은 폴더 규칙**을 써야 했고, 그 규칙(`default_data_dir`)을 공용 crate 에 두면서 클라이언트 쪽 데몬 찾기 · 띄우기 · 끄기를 함께 넣었다(ADR-0024 「데이터 위치」 구현 줄 — 「`default_data_dir()` 단일 출처」). ADR-0264 결정 4 는 같은 자리에 경로의 단일 출처 `DataLayout` 을 새로 두었다(디렉터리 + 둘 이상의 프로세스가 보는 파일 `daemon.json`).

실제 쓰임(코드 대조 2026-10-02 · `d5ac725`):

| 무엇 | 쓰는 쪽 |
|---|---|
| 루트 찾기 `default_data_dir` · `release_data_dir` | 데몬 · 셸 둘 다 |
| `DataLayout` — `daemon_state_dir` · `daemon_run_dir` · `mcp_config_dir` · `usage_probe_dir` · `ensure_daemon_dirs` | 데몬 |
| `DataLayout` — `daemon_file`(`daemon.json` 자리) | 데몬이 쓰고 셸 · 스크립트가 읽는다 |
| `DataLayout` — `shell_config_dir` · `shell_state_dir` · `shell_run_dir` · `webview_dir` | 셸 |
| `DataLayout` — `logs_dir` | 데몬 · 셸 둘 다(로그 파일은 프로세스마다 따로 — ADR-0264 결정 2) |
| 쓰기 가능 확인 — `ensure_data_dir_writable`(폴더를 만든다) | 데몬 기동 |
| 쓰기 가능 확인 — `check_data_dir_writable`(만들지 않는다) | 셸 `ensure_daemon` 의 사전 점검(ADR-0135) |
| 설치 위치 `find_install_root` | 데몬(`control/priming.rs`) · 데몬 패키지 안의 `engram` CLI |
| 데몬 찾기 · 띄우기 · 상태 · 프로세스 끄기 · 데몬 실행 파일 찾기(`ensure_daemon` · `daemon_status` · `read_live_daemon` · `daemon_stop` · `locate_daemon_exe`) | 셸 |
| 정지 명령 `send_stop`(WebSocket 접속 · 인증 · 정지 명령을 직접 한다) | 셸 |
| WMI 로 터미널 트리 밖에 띄우기(`wmi_spawn` · `wmi_create_raw`) | 셸(`ensure_daemon` 안) |

discovery 가 `net`(인증 메시지)과 `protocol` 명령 메시지를 아는 이유는 전부 `send_stop` 이다. 실행 파일 위치 계산(현재 exe 옆 · `.exe` 붙이기)은 두 곳에 갈려 있다 — 데몬 `locate_send_exe`(CLI) · discovery `locate_daemon_exe`(데몬).

T-10(2026-08-26)이 「discovery 를 없애지 않는다」로 종결했다 — 사유는 「데몬도 그 crate 를 의존하고 async 무의존 게이트가 거기 붙으며 `base` 셋째 입주자 문제(ADR-0175)가 걸린다 · 남은 동기(`src/lib.rs` 크기)는 파일 분할로」다(`docs/tracking-archive.md`).

사용자(2026-10-02): 네트워크 방식이 되면 폴더 규칙은 데몬만 필요하다 · 같은 PC 모드도 원격과 **같은 인증 방식**으로 하면 되고 로직을 갈라 둘 필요가 없다(편의용 자동 인증은 나중) · 「지금 당장은 discovery 뽀개고 데몬 셸 각각 두고 나중에 관련해서 얘기할 때 분리」. 원칙(사용자 2026-10-02, 결정 후보 5): 「클라는 데몬이랑 아예 별도의 개념이라서 agent 를 모르는 게 좋음」.

## 결정

1. **discovery crate 를 나눠 없앤다 — 데몬 몫은 데몬 crate 안으로, 셸 몫은 셸 안으로**(사용자 2026-10-02).
2. **데몬으로:** 데이터 폴더 규칙(`default_data_dir` · `release_data_dir`) · 폴더를 만드는 쓰기 가능 확인(`ensure_data_dir_writable`) · 설치 위치(`find_install_root`) · 실행 파일 위치 계산(두 곳 → 한 곳).
3. **셸로:** 데몬 찾기 · 띄우기 · 상태 · 프로세스 끄기 · 데몬 실행 파일 찾기 · 띄우기 전 사전 점검(`check_data_dir_writable` — `ensure_daemon` 이 부른다).
4. **`DataLayout`(ADR-0264)도 나눈다**(사용자 2026-10-02):
   - **데몬으로:** 데몬 폴더 경로 — `daemon\state\` · `daemon\run\`(`daemon.json` 을 쓰는 자리) · `mcp-config\` · `usage-probe\` · `ensure_daemon_dirs`.
   - **셸로:** 셸 폴더 경로 — `shell\config\` · `shell\state\` · `shell\run\` · `webview\`.
   - **셸에 한 벌 더 두는 것은 둘뿐이다 — 루트 찾기와 `daemon.json` 자리.** 셸은 데몬 crate 를 의존하지 않으므로(클라 · 데몬 분리) 그 둘의 규칙이 셸 쪽에 사본으로 선다. **두 벌이 같은 경로를 내는지 재는 시험으로 묶는다** — 셸은 데몬 패키지를 테스트 전용 의존으로 이미 끌어온다(`src-tauri/Cargo.toml` 의 dev-dependency).
   - **이 연결은 과도기다** — 원격 인증 경로가 생기기 전까지 셸이 `daemon.json` 을 읽어야 해서 있는 것이고, 원격 · 같은 PC 인증을 통일할 때 사라진다.
5. **정지 명령 클라이언트(`send_stop`)는 셸의 데몬 연결 코드로 간다 — 단 transport 단계에서**(작업 순서 ② · 3-2 셸 부착 — 정지 명령 클라이언트 · 접속 정보 인터페이스와 함께). 그러면 셸 밖 어디에도 작은 데몬 클라이언트가 안 남는다.
6. **WMI 로 띄우는 코드는 platform 으로 간다**(결정 후보 1). discovery 의 `Clock` 은 base `time` 으로 합친다(결정 후보 4).
7. **착수 = 작업 순서 2-2**(선행 1-3 platform · 1-1 base). 데몬 기동 실측이 완료 조건이다.

## 거부한 대안

- **discovery 를 그대로 둔다(T-10 의 결론).** 기각 = 사용자 결정(2026-10-02 — 「지금 당장은 discovery 뽀개고 데몬 셸 각각 두고」) · 새 근거 둘(클라 · 데몬 분리 원칙 · 원격 대비 — 네트워크 방식이면 폴더 규칙은 데몬만 필요하다). T-10 의 종결 사유는 각각 이렇게 된다(코드 대조):
  - 「데몬도 그 crate 를 의존」 — 데몬이 쓰는 것을 데몬 안으로 옮기는 것이 이 결정이다.
  - 「async 무의존 게이트가 거기 붙는다」(`ci.yml` 의 `Gate: discovery has no async-runtime ingress`) — 그 게이트가 지키던 것은 discovery 를 단독으로 빌드 · 소비할 때 tokio 가 딸려 오지 않는다는 성질이다(discovery `Cargo.toml` 주석). 소비자는 데몬 · 셸 · 데몬 패키지 안의 `engram` CLI 셋이다(위 표 — CLI 는 `find_install_root`). 데몬 · 셸은 tokio 를 지고(두 `Cargo.toml` 의 `tokio`), CLI 는 데몬 패키지의 bin 이라 그 패키지의 의존(tokio 포함)과 함께 빌드된다 — ADR-0273 뒤에는 설치 위치를 자기 사본으로 가져(그 결정 5) 이쪽 코드를 아예 쓰지 않는다. 나눈 뒤 지킬 대상이 없다.
  - 「`base` 셋째 입주자 문제」 — ADR-0269 이 답했다(base 는 범용 코드를 목적 모듈로 받는다).
- **셸이 경로를 얻으려고 데몬 crate 를 의존한다.** 기각 = 클라 · 데몬 분리 원칙(사용자 2026-10-02 — 메모 §4 「셸은 데몬 crate 를 의존하지 않으므로(클라·데몬 분리)」) + 코드: 데몬 crate 는 agent 를 의존하므로 셸이 운영 의존으로 데몬을 들이면 결정 후보 5 가 끊는 셸 → agent 간선이 도로 생긴다.
- **경로 규칙만 담는 작은 공용 crate 를 세운다.** 기각 = 사용자 원칙(메모 §5 — 「애매한 건 데몬으로 몬 다음에 위에 정리되고 다시 데몬 정리를 하는 방향」 · 자리가 애매한 코드는 crate 를 새로 세우거나 공용으로 두지 않고 일단 데몬 안으로 넣는다).

## 근거

- **사용자 결정 2026-10-02** — 위 「맥락」 인용 · `DataLayout` 나누기(메모 §11 의 「2-2 착수 전 사용자 결정」 항목에 대한 답).
- **코드 대조(2026-10-02 · `d5ac725`)** — 위 「맥락」 표(`rg "DataLayout|default_data_dir|find_install_root|engram_dashboard_discovery::"` 를 discovery 밖에서 돌린 결과와 각 함수 본문) · 셸의 dev-dependency(`engram-dashboard-daemon`) · 셸의 운영 의존(`tokio` · `engram-dashboard-net` · `engram-dashboard-protocol`).
- **T-10 종결 사유** — `docs/tracking-archive.md` 의 T-10 줄.

## 영향 / 불변식

- **불변식: 셸의 운영 의존에 데몬 crate 가 없다**(클라 · 데몬 분리). 데몬 · 셸이 같은 데이터 루트(릴리스 `<exe>\data` — ADR-0134 · ADR-0136 · 개발 `<워크트리>\.engram-dev` — ADR-0264 결정 8 · 시험 `ENGRAM_DATA_DIR`)와 같은 `daemon.json` 자리(`daemon\run\daemon.json` — ADR-0264 결정 6)로 수렴한다는 불변식은 그대로이고, 그 근거가 「출처 하나」에서 「두 벌 + 같은 경로 시험」으로 바뀐다. ★ADR-0024 의 「세 exe 가 같은 빌드모드에서 같은 폴더로 수렴」을 이 불변식의 출처로 들지 않는다★ — 그 세 exe(daemon · embedded · tray-host)와 「디버그 = repo 루트 · 릴리즈 = exe 옆」 수렴은 ADR-0027 이 걷었고, embedded · tray-host 는 ADR-0029 로 사라졌으며, 폴더 이름은 ADR-0134 · ADR-0136 · ADR-0264 결정 8 이 다시 정했다.
- **ADR-0024 를 고친다** — 「데이터 위치」 구현 줄의 「`engram_dashboard_discovery::default_data_dir()` 단일 출처」가 데몬 정본 + 셸 사본(시험으로 묶음)이 된다. 우선순위(① `ENGRAM_DATA_DIR` > ② 디버그 · ③ 릴리스)와 self-resolve 는 그대로다.
- **ADR-0264 를 고친다** — 결정 4 의 「경로의 단일 출처 = discovery 의 `DataLayout`」이 데몬 · 셸 각자의 경로 + 사본 둘(루트 · `daemon.json`)이 된다. 「한 저장소만 쓰는 파일 이름은 그 저장소가 소유」 · 「net 은 잠금 파일 경로를 인자로 받는다」 · 「OS 가름은 `default_data_dir` 안에만」은 그대로다. 결정 2 의 배치(폴더 이름 = 디스크 계약)는 바뀌지 않는다.
- **T-10 을 다시 연다** — `docs/tracking-archive.md` 의 T-10 종결 줄이 이 ADR 을 가리킨다(2026-10-02 갱신). 작업 추적은 T-47(크레이트 경계 리팩터링)의 작업 순서 2-2 다.
- **`daemon.json` 자리를 따로 아는 곳이 하나 더 있다** — 스크립트 `scripts/engram.mjs`(`PORTFILE_IN_DATA_DIR` — ADR-0264 결정 6). 이 결정이 늘리는 것은 셸 쪽 Rust 사본 하나다.
- **열린 것 — 2-2 착수 때 정한다(내부 배치):**
  - **`logs\` 자리** — 데몬 · 셸이 둘 다 쓴다(표). 셸 쪽 사본으로 둘지 · 같은 경로 시험에 넣을지.
  - **실행 파일 위치 계산 「두 곳 → 한 곳」의 모양** — 메모는 그 계산을 데몬으로, 데몬 실행 파일 찾기를 셸로 보낸다. 셸은 데몬 crate 를 의존하지 않으므로 데몬 exe 를 찾는 코드는 셸에 남는다 — 한 곳으로 모이는 것이 무엇인지(현재 exe 옆 규칙 · `.exe` 붙이기는 결정 후보 1 의 platform 몫)를 그때 확정한다.
  - **쓰기 검사 도우미**(`retry_if_vanished` · `probe_write_in`) — 데몬 · 셸 쓰기 가능 확인 둘이 함께 쓴다. 나누면 사본 둘이 되므로 base 에 둘지 본다. ★**판정은 ADR-0269 결정 7 의 입주 규칙(「지금 여러 곳에서 쓰이면」)을 따른다**★ — ADR-0269 는 지금 `retry_if_vanished` 를 「입주시키지 않는 것」에 두었고(discovery 전용 정책), 2-2 의 나누기가 실제로 사본 둘을 만들 때까지 그 제외가 선다.
  - **2-2 와 3-2 사이 `send_stop` 의 자리** — 결정 5 가 transport 단계로 미루므로 2-2 뒤에도 잠시 산다. 셸은 이미 `net` · `protocol` 을 운영 의존으로 진다 — 셸로 그대로 옮겨도 새 의존이 생기지 않는다.
- **사라지는 게이트 · 낡는 서술:**
  - `ci.yml` 의 `Gate: discovery has no async-runtime ingress` — 대상 crate 가 사라진다.
  - net gate 1(`rg "engram_dashboard_(daemon|messaging|discovery)" crates/engram-dashboard-net/src/`) · 메시징 격리 정규식의 이름 알파벳 — `discovery` 가 없는 이름이 된다(0 기대라 빨개지지는 않는다 — 정리 대상).
  - CLAUDE.md 「백엔드 모듈 맵」 discovery 항목(`DataLayout` 서술 포함) · ADR-0175 영향의 의존 그래프 중 discovery 가 든 줄(`discovery → base, protocol, net` 등).
  - 주석 — base `logging/mod.rs` 머리(「데이터 폴더 배치는 discovery 의 `DataLayout` 몫」) · net `instance.rs`(「discovery `DataLayout` 이 정본」) · 데몬 `control/mcp_config.rs` · `docs/reference/logging-conventions.md`(「호출자가 discovery `DataLayout::logs_dir()` 로 넘긴다」).
  - discovery 의 시험(`tests/stop_smoke.rs` · 단위 시험)은 함수를 따라 데몬 · 셸로 간다 — 판정 로직의 주입 seam(`PidLiveness` · `DaemonReader` · `Spawner` · `ProcessKiller` · `StopSender`)도 함께 간다.
- **engram CLI 의 설치 위치** — CLI 가 쓰는 `find_install_root` 는 데몬으로 간다. CLI 쪽 사본과 같은 경로 시험은 ADR-0273 이 진다.
- **나중(원격 작업 때 — 메모 §4):** 클라이언트가 「데몬 주소와 토큰을 어디서 얻나」를 인터페이스로 두고, 같은 PC 도 같은 인증 방식으로 통일 · 편의용 자동 인증 검토. 토큰은 서버가 발급하고 클라이언트가 열쇠로 들고 있는 모델 그대로. 그때 결정 4 의 과도기 연결이 사라진다.
- **작업은 망가지지 않는 단위로 묶는다**(사용자 2026-10-02 — 「망가지지 않는 단위로 잘 그룹지어서 작업하라」). 2-2 안에서 단위마다 빌드 · 회귀가 초록이고 데몬이 뜨는 채로 끊는다.
- **코드 앵커 = `// ADR-0271`** — 데몬의 데이터 폴더 규칙 · 셸의 루트 · `daemon.json` 사본과 그 둘을 묶는 시험.
