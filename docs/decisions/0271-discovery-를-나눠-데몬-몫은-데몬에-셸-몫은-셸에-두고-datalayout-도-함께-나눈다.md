# ADR-0271: Split discovery, put the daemon's share in the daemon and the shell's share in the shell, and split DataLayout too

- 상태: 확정 (2026-10-02, basis: user decision 2026-10-02 (`docs/refactoring/architecture-discussion-2026-09-26.md` decision candidate 6 · the answer to the ADR-0264 conflict item in §11) + code cross-check 2026-10-02 (`d5ac725`)) · 부분 폐기 by ADR-0282 (the executable-location computation spot in decision 2) · 부분 폐기 by ADR-0285 (the CLI-copy item under Impact and the word "copy" in the first item of Rejected alternatives) · 부분 폐기 by ADR-0350 (결정 5 정지 클라이언트에 더하는 강제 종료 대체와 10초 시한)
- 관련: Amends ADR-0024 (the discovery single source in the data-location implementation line) · Amends ADR-0264 (the discovery DataLayout single source in decision 4) · `docs/tracking-archive.md` T-10 (discovery merge — closed: not doing it, user decision 2026-08-26 · this decision reopens it) · ADR-0029 (shell = daemon client) · ADR-0134 (data root location — unchanged) · ADR-0135 (client pre-check · lock file = connection file) · ADR-0175 (`base` third occupant) · ADR-0266 (WMI launch → platform) · ADR-0269 (`Clock` → base · occupancy rule) · ADR-0270 (the client does not know agent) · decision candidate 7 (to be fixed at the transport TRD) (removing net) · ADR-0273 (the copy of the engram CLI's install location) · memo §5 (when unsure, put it in the daemon for now) · `crates/engram-dashboard-discovery/src/lib.rs` · `crates/engram-dashboard-discovery/src/layout.rs` · step-log S21 · Amends ADR-0175 (the dependency-graph lines under Impact that contain discovery) · Amended by ADR-0282 (the executable-location computation spot in decision 2) · Amended by ADR-0285 (the CLI-copy item under Impact and the word "copy" in the first item of Rejected alternatives) · Amended by ADR-0350 (결정 5 정지 클라이언트에 더하는 강제 종료 대체와 10초 시한)

## 맥락

discovery exists because of ADR-0024 — the daemon writes the port · token · PID into `daemon.json` in the data folder and the client reads the same file to connect and authenticate, so both sides had to use the **same folder rule**. That rule (`default_data_dir`) was put in a shared crate, and the client-side finding, launching and stopping of the daemon went in with it (ADR-0024 「데이터 위치」 implementation line — 「`default_data_dir()` 단일 출처」). ADR-0264 decision 4 put a single source of paths, `DataLayout`, in the same place (directories + the file that two or more processes look at, `daemon.json`).

Actual usage (code cross-check 2026-10-02 · `d5ac725`):

| What | Used by |
|---|---|
| Root lookup `default_data_dir` · `release_data_dir` | both daemon and shell |
| `DataLayout` — `daemon_state_dir` · `daemon_run_dir` · `mcp_config_dir` · `usage_probe_dir` · `ensure_daemon_dirs` | daemon |
| `DataLayout` — `daemon_file` (the `daemon.json` location) | the daemon writes it, the shell and scripts read it |
| `DataLayout` — `shell_config_dir` · `shell_state_dir` · `shell_run_dir` · `webview_dir` | shell |
| `DataLayout` — `logs_dir` | both daemon and shell (each process has its own log file — ADR-0264 decision 2) |
| Writability check — `ensure_data_dir_writable` (creates the folder) | daemon startup |
| Writability check — `check_data_dir_writable` (does not create it) | the shell's `ensure_daemon` pre-check (ADR-0135) |
| Install location `find_install_root` | daemon (`control/priming.rs`) · the `engram` CLI inside the daemon package |
| Daemon finding · launching · status · process stopping · daemon executable lookup (`ensure_daemon` · `daemon_status` · `read_live_daemon` · `daemon_stop` · `locate_daemon_exe`) | shell |
| Stop command `send_stop` (does the WebSocket connection · authentication · stop command directly) | shell |
| Launching outside the terminal tree via WMI (`wmi_spawn` · `wmi_create_raw`) | shell (inside `ensure_daemon`) |

The reason discovery knows `net` (the authentication message) and the `protocol` command message is entirely `send_stop`. The executable-location computation (next to the current exe · appending `.exe`) is split across two places — the daemon's `locate_send_exe` (CLI) and discovery's `locate_daemon_exe` (daemon).

T-10 (2026-08-26) closed with 「discovery 를 없애지 않는다」 — the reason was 「데몬도 그 crate 를 의존하고 async 무의존 게이트가 거기 붙으며 `base` 셋째 입주자 문제(ADR-0175)가 걸린다 · 남은 동기(`src/lib.rs` 크기)는 파일 분할로」 (`docs/tracking-archive.md`).

The user (2026-10-02): with the network approach, only the daemon needs the folder rule · same-PC mode can use the **same authentication method** as remote, so there is no need to keep two split logics (convenience auto-authentication comes later) · 「지금 당장은 discovery 뽀개고 데몬 셸 각각 두고 나중에 관련해서 얘기할 때 분리」. The principle (user 2026-10-02, decision candidate 5): 「클라는 데몬이랑 아예 별도의 개념이라서 agent 를 모르는 게 좋음」.

## 결정

1. **Split the discovery crate and remove it — the daemon's share goes into the daemon crate, the shell's share goes into the shell** (user 2026-10-02).
2. **To the daemon:** the data-folder rule (`default_data_dir` · `release_data_dir`) · the writability check that creates the folder (`ensure_data_dir_writable`) · the install location (`find_install_root`) · the executable-location computation (two places → one place).
3. **To the shell:** daemon finding · launching · status · process stopping · daemon executable lookup · the pre-launch pre-check (`check_data_dir_writable` — called by `ensure_daemon`).
4. **`DataLayout` (ADR-0264) is split too** (user 2026-10-02):
   - **To the daemon:** the daemon folder paths — `daemon\state\` · `daemon\run\` (where `daemon.json` is written) · `mcp-config\` · `usage-probe\` · `ensure_daemon_dirs`.
   - **To the shell:** the shell folder paths — `shell\config\` · `shell\state\` · `shell\run\` · `webview\`.
   - **Only two things get one more copy in the shell — root lookup and the `daemon.json` location.** Because the shell does not depend on the daemon crate (client · daemon separation), the rules for those two stand on the shell side as copies. **The two copies are tied together by a test that checks they produce the same path** — the shell already pulls in the daemon package as a test-only dependency (the dev-dependency in `src-tauri/Cargo.toml`).
   - **This link is transitional** — it exists because the shell has to read `daemon.json` until the remote authentication path exists, and it goes away when remote and same-PC authentication are unified.
5. **The stop-command client (`send_stop`) goes to the shell's daemon-connection code — but at the transport stage** (work order ② · 3-2 shell attachment — together with the stop-command client and the connection-info interface). Then no small daemon client is left anywhere outside the shell.
6. **The code that launches via WMI goes to platform** (decision candidate 1). Of discovery's `Clock` (`now()` + blocking sleep), `now()` is merged into base `time`'s 「지금 읽기」 trait. The blocking sleep stays as discovery's own trait that has that trait as its supertrait (ADR-0269 decision 3) and goes to the shell together with `ensure_daemon` (decision 3 — launching and polling use that clock).
7. **Start = work order 2-2** (prerequisites 1-3 platform · 1-1 base). The completion condition is measuring the daemon startup.

## 거부한 대안

- **Leave discovery as it is (the conclusion of T-10).** Rejected = user decision (2026-10-02 — 「지금 당장은 discovery 뽀개고 데몬 셸 각각 두고」) · two new grounds (the client · daemon separation principle · preparing for remote — with the network approach only the daemon needs the folder rule). Each of T-10's closing reasons now stands like this (code cross-check):
  - 「데몬도 그 crate 를 의존」 — moving what the daemon uses into the daemon is this decision.
  - 「async 무의존 게이트가 거기 붙는다」 (`Gate: discovery has no async-runtime ingress` in `ci.yml`) — what that gate guarded is the property that tokio does not come along when discovery is built and consumed on its own (comment in discovery `Cargo.toml`). The consumers are three: the daemon, the shell, and the `engram` CLI inside the daemon package (table above — the CLI uses `find_install_root`). The daemon and the shell carry tokio (`tokio` in both `Cargo.toml`), and the CLI is a bin of the daemon package, so it is built together with that package's dependencies (tokio included) — after ADR-0273 it takes its own copy of the install location (its decision 5) and does not use this code at all. After the split there is nothing left to guard.
  - 「`base` 셋째 입주자 문제」 — ADR-0269 answered it (base accepts general-purpose code as purpose modules).
- **The shell depends on the daemon crate to get paths.** Rejected = the client · daemon separation principle (user 2026-10-02 — memo §4 「셸은 데몬 crate 를 의존하지 않으므로(클라·데몬 분리)」) + code: the daemon crate depends on agent, so if the shell pulled in the daemon as a runtime dependency, the shell → agent edge that decision candidate 5 cuts would come back.
- **Set up a small shared crate that holds only the path rules.** Rejected = user principle (memo §5 — 「애매한 건 데몬으로 몬 다음에 위에 정리되고 다시 데몬 정리를 하는 방향」 · code whose place is unclear does not get a new crate or a shared home, it goes into the daemon for now).

## 근거

- **User decision 2026-10-02** — the quotation in 「맥락」 above · the `DataLayout` split (the answer to the item 「2-2 착수 전 사용자 결정」 in memo §11).
- **Code cross-check (2026-10-02 · `d5ac725`)** — the table in 「맥락」 above (the result of running `rg "DataLayout|default_data_dir|find_install_root|engram_dashboard_discovery::"` outside discovery, and each function body) · the shell's dev-dependency (`engram-dashboard-daemon`) · the shell's runtime dependencies (`tokio` · `engram-dashboard-net` · `engram-dashboard-protocol`).
- **T-10 closing reason** — the T-10 line in `docs/tracking-archive.md`.

## 영향 / 불변식

- **Invariant: the shell's runtime dependencies do not include the daemon crate** (client · daemon separation). The invariant that the daemon and the shell converge on the same data root (release `<exe>\data` — ADR-0134 · ADR-0136 · development `<워크트리>\.engram-dev` — ADR-0264 decision 8 · test `ENGRAM_DATA_DIR`) and the same `daemon.json` location (`daemon\run\daemon.json` — ADR-0264 decision 6) stays as it is, and its basis changes from 「출처 하나」 to 「두 벌 + 같은 경로 시험」. ★Do not cite ADR-0024's 「세 exe 가 같은 빌드모드에서 같은 폴더로 수렴」 as the source of this invariant★ — those three exes (daemon · embedded · tray-host) and the 「디버그 = repo 루트 · 릴리즈 = exe 옆」 convergence were removed by ADR-0027, embedded · tray-host were gone through ADR-0029, and the folder names were decided again by ADR-0134 · ADR-0136 · ADR-0264 decision 8.
- **ADR-0024 is amended** — the 「`engram_dashboard_discovery::default_data_dir()` 단일 출처」 in the 「데이터 위치」 implementation line becomes a daemon original + a shell copy (tied by a test). The priority (① `ENGRAM_DATA_DIR` > ② debug · ③ release) and self-resolve stay as they are.
- **ADR-0264 is amended** — decision 4's 「경로의 단일 출처 = discovery 의 `DataLayout`」 becomes each of the daemon's and the shell's own paths + two copies (root · `daemon.json`). 「한 저장소만 쓰는 파일 이름은 그 저장소가 소유」 · 「net 은 잠금 파일 경로를 인자로 받는다」 · 「OS 가름은 `default_data_dir` 안에만」 stay as they are. The layout of decision 2 (folder names = disk contract) does not change.
- **T-10 is reopened** — the T-10 closing line in `docs/tracking-archive.md` points at this ADR (updated 2026-10-02). Work tracking is work order 2-2 of T-47 (crate boundary refactoring).
- **There is one more place that knows the `daemon.json` location separately** — the script `scripts/engram.mjs` (`PORTFILE_IN_DATA_DIR` — ADR-0264 decision 6). What this decision adds is one Rust copy on the shell side.
- **Open — decided when 2-2 starts (internal layout):**
  - **The `logs\` location** — both the daemon and the shell write to it (table). Whether to keep it as a shell-side copy · or put it into the same-path test.
  - **The shape of the executable-location computation 「두 곳 → 한 곳」** — the memo sends that computation to the daemon and the daemon executable lookup to the shell. Because the shell does not depend on the daemon crate, the code that finds the daemon exe stays in the shell — what gets gathered into one place (the next-to-current-exe rule · appending `.exe` belong to decision candidate 1's platform) is fixed then.
  - **The write-check helpers** (`retry_if_vanished` · `probe_write_in`) — both writability checks, the daemon's and the shell's, use them together. Splitting would make two copies, so look at whether to put them in base. ★**The decision follows the occupancy rule of ADR-0269 decision 7 (「지금 여러 곳에서 쓰이면」)**★ — ADR-0269 put `retry_if_vanished` under 「입주시키지 않는 것」 for now (a discovery-only policy), and that exclusion stands until the 2-2 split actually creates two copies.
  - **Where `send_stop` lives between 2-2 and 3-2** — decision 5 defers it to the transport stage, so it lives on for a while after 2-2. The shell already carries `net` · `protocol` as runtime dependencies — moving it into the shell as it is creates no new dependency.
- **Gates that disappear · statements that go stale:**
  - `Gate: discovery has no async-runtime ingress` in `ci.yml` — the target crate disappears.
  - net gate 1 (`rg "engram_dashboard_(daemon|messaging|discovery)" crates/engram-dashboard-net/src/`) · the name alphabet of the messaging isolation regex — `discovery` becomes a name that no longer exists (the expectation is 0, so it does not turn red — it is a cleanup target).
  - The discovery entry of CLAUDE.md 「백엔드 모듈 맵」 (including the `DataLayout` description) · the dependency-graph lines under ADR-0175's Impact that contain discovery (`discovery → base, protocol, net` etc.).
  - Comments — the head of base `logging/mod.rs` (「데이터 폴더 배치는 discovery 의 `DataLayout` 몫」) · net `instance.rs` (「discovery `DataLayout` 이 정본」) · daemon `control/mcp_config.rs` · `docs/reference/logging-conventions.md` (「호출자가 discovery `DataLayout::logs_dir()` 로 넘긴다」).
  - discovery's tests (`tests/stop_smoke.rs` · unit tests) follow their functions to the daemon and the shell — the injection seams of the decision logic (`PidLiveness` · `DaemonReader` · `Spawner` · `ProcessKiller` · `StopSender`) go with them.
- **The install location of the engram CLI** — the `find_install_root` that the CLI uses goes to the daemon. The CLI-side copy and the same-path test are ADR-0273's to carry.
- **Later (at the remote work — memo §4):** make 「데몬 주소와 토큰을 어디서 얻나」 an interface, unify the same PC onto the same authentication method · consider convenience auto-authentication. The token stays the model where the server issues it and the client holds it as a key. At that point the transitional link of decision 4 goes away.
- **Bundle the work into units that do not break things** (user 2026-10-02 — 「망가지지 않는 단위로 잘 그룹지어서 작업하라」). Within 2-2, cut at points where, for each unit, build and regression are green and the daemon comes up.
- **Code anchor = `// ADR-0271`** — the daemon's data-folder rule · the shell's root and `daemon.json` copies and the test that ties the two.
