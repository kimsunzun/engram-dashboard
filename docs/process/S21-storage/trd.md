# TRD — 저장 관리 구조: 데이터 배치 · 웹뷰 폴더 · 설정 · 화면 상태 (S21)

> 상태: **초안 6판 (2026-10-02)** — **6판 = 5판 리뷰(설계자 BLOCK · 설계자-파괴자 FIX) 반영 + 사용자 결정 D4–D8: 묻는 표면은 main 창 안 모달 · 기본 모양 예외 없음 · 답만 사본을 푼다 · `--hidden` · 쓰기 실패는 따로 다루지 않는다 + 셸 실행 잠금 · 답의 디스크 고정 · 단계 다시 자르기(§14-10). 6판 재리뷰(설계자 PASS · 설계자-파괴자 FIX) 반영 = §14-11.** 5판 = P3 착수 전 사용자 결정 D1 · D2 · D3 — F17 철회 · 크래시 사본 한 개 · 크래시 뒤 늘 묻는다(§14-9). 아래는 4판까지의 경위다.
>
> 4판까지: **리뷰 2회 반영**. 2판 = 1차 리뷰(설계자 · 파괴자 — 둘 다 FIX) 반영 · 3판 = 2차 리뷰(파괴자 FIX · 설계자 BLOCK — 같은 핵심 결함: 대기분이 디스크 확정 전에 지워진다) 반영 — 대응표 = §14. **이후 P1 구현 · 코드 리뷰 뒤 사용자 결정(2026-10-02)으로 첫 부팅 이전을 걷었다 — 옛 데이터는 옮기지 않고 버린다(§3-2 · §14-5).** P1 은 구현됐다(`fe8b552`). **4판 = P2 착수 전 사용자 결정 셋 반영 — 저장 범위 표 · `ui-settings.json` 무이전 · ~~비정상 종료 스냅숏 회전 보관~~(5판에서 철회 — §14-9)(§14-6) + 4판 리뷰(light · 설계자-파괴자 FIX) 반영과 사용자 결정 U1(사용자가 본인뿐이라 옛 값을 다루지 않는다) · U2(챗 스타일은 지금 11키만 가볍게)(§14-7). ★§14-7 반영분은 아직 재리뷰 전이다★.** **P2a 코드 리뷰(deep) 뒤 §5 설정 사양을 구현에 맞췄다(§14-8).**
>
> **입력:** PRD 결정 = [`docs/research/storage-management-survey-2026-10-02.md`](../../research/storage-management-survey-2026-10-02.md) 「0. 결정」(구속) · 같은 보고서 §1–§6(근거). **판독 기준** = 브랜치 `v0.3.3/feat/storage` HEAD `d4ff3f7` · Tauri 2.11.3 / tao 0.35.3 / tauri-runtime-wry 2.11.3 / tauri-plugin-single-instance 2.4.2 = 이 PC cargo 레지스트리 소스.
>
> **앵커:** ADR-0003 · ADR-0006 · ADR-0012 · ADR-0024 · ADR-0035/0057 · ADR-0051 · ADR-0054 · ADR-0056 · ADR-0060 · ADR-0078 · ADR-0102 · ADR-0134/0135/0136/0137 · ADR-0149 · ADR-0155 · ADR-0166/0167 · ADR-0169 · ADR-0171 · ADR-0222 · ADR-0230.
>
> 표기: **[고름]** = 사용자 체감이 없는 내부 구현이라 이 문서가 고른 것. **(추천 · 세션 기본값 — 사용자 확인 대기)** = 사용자 체감 갈림길의 기본값(§10) — 「추천」이 빠진 것(F20 · F21)도 같은 뜻이고, 이 문서가 추천 근거를 내지 않은 기본값이다. **★명시 확인★** = 사용자에게 따로 올릴 항목 — 답이 없으면 기본값으로 진행하되, §9-2 단계 행에 「착수 전 … 답」이 붙은 것은 그 단계 전에 답을 받는다. **U1 · U2** = 2026-10-02 사용자 결정(§14-7). **D1 · D2 · D3** = 2026-10-02 사용자 결정(§14-9 — 크래시 사본). **D4–D8** = 2026-10-02 사용자 결정(§14-10 — 모달 · 사본 수명 · 쓰기 실패). **[미검]** = 소스 읽기만 했고 실행으로 확인하지 않은 것.

---

## 0. 결론 (먼저)

| 무엇 | 어디 | 요지 |
|---|---|---|
| 경로 계산 | discovery `DataLayout`(신설) | 디렉터리와 둘 이상의 프로세스가 보는 파일(`daemon.json`) 경로의 단일 출처. 데몬·셸·net 은 받은 경로만 쓴다 |
| 옛 데이터 | 없음(걷음 — §3-2) | **옮기지 않고 버린다.** 새 코드는 옛 평면 파일 · `.engram-data` 를 읽지도 옮기지도 지우지도 않는다. 잠금은 `daemon\run\daemon.json` 하나 |
| 웹뷰 폴더 | 셸 | 정적 창 둘을 Rust 에서 만들고(`create:false` + `from_config`) 모든 창에 같은 `data_directory` + 같은 브라우저 인자. 만들기 전에 쓰기 확인 |
| 저장 범위 | §6-0 | 남기는 것 · 버리는 것을 항목별로 정했다(사용자 결정). 취향 = `settings.json` · 기록 = `state.json`. 슬롯 보기 모드(터미널/챗)는 프론트 메모리에서 셸 소유 슬롯 값으로 옮겨 남긴다(P3e) |
| 설정 | 셸 `settings` 모듈(신설) | 스키마 표 한 줄 = 설정 하나. 버스 명령 넷 + 같은 서비스를 부르는 Tauri 껍데기. 파일을 아는 것은 저장 계층 하나. **기본값에서 시작 — 사용자가 본인뿐이라 옛 값은 다루지 않는다(U1 · §3-5).** 챗 스타일은 지금 11키를 그대로 옮기고 `chat.style.*` 는 임시 이름공간이다(U2 · §5-1) |
| 화면 상태 | 셸 `state` 모듈(신설) | 파일 일은 **단일 인스턴스 관문 뒤 · 어느 창보다 먼저**(단일 인스턴스 바로 뒤 플러그인 — 셸 실행 잠금 `shell\run\state.lock` 을 쥔 뒤 읽는다). **`state.json` 을 쓰는 것은 부팅 첫 쓰기 뒤 기록기 스레드 하나뿐.** **비정상 종료 뒤엔 `state.json` 을 사본 `state.crash.json` 한 개로 떠 두고 기본 화면 + main 창 안 모달로 늘 묻는다**(D1–D5 · §6-5 · §6-7). 답만 사본을 풀고(D6), 답은 `state.json` 의 해시로 디스크에 붙는다. 런타임 복원은 조율자 하나가 한 커밋 지점에서 바꾼다 |
| 순서 | §9 | P1 → P2(설정) → P3(상태) → P4(웹뷰). **머지 단위 = L1(P1) · L2(P2a–c) · L3(P3a–e) · L4(P4), 전체가 한 릴리스.** ★P3 착수 전 게이트 — 6판 재리뷰 FIX 반영(§14-11) 확인 대기(§9-1)★ |

## 1. 목표 · 범위

- **안(in):** 「0. 결정」의 데이터 배치(옛 데이터는 버림 — §3-2) · 웹뷰 폴더 이전 · 설정(`settings.json` + 명령 넷 + 챗 스타일 권위 이전(지금 11키 그대로 — U2) + `ui-settings.json` 폐지(옛 값은 다루지 않는다 — U1 · §3-5) + 도움말·프라이밍) · 화면 상태(`state.json` + 창 신원 + 복원 + 비정상 종료 확인) · 창별 테마의 영속 · 대상 없는 슬롯의 두 상태 표시 · **저장 범위 표(§6-0) — 슬롯 보기 모드의 셸 이전 · 영속 포함(P3e)**.
- **밖(out):** §6-0 「남기지 않는 것」(펼침 · 스크롤 고정 · 선택 · 입력 초안 · 트레이 숨김) · 프리셋 시스템 · 테마 프리셋 · 내장 테마 JSON 통일 · 작업 이력(T-14) · 개발 Vite 포트 공유 · T-22(명부 저장 실패의 성공 보고) · T-33(모르는 백엔드 종류로 명부 전체 손상) — **새 배치는 `agents.json` 의 자리만 바꾸므로 둘 다 건드리지 않는다** · 슬롯 종류별 설정의 코드(폴더 자리만 문서로 둔다 — §7) · 설정 화면 · 테마를 고르는 화면 메뉴(§10 F7) · **옛 값 처리 전부**(배포본(v0.1.0–v0.3.2)은 있으나 사용자가 본인뿐이라 지킬 사용자 데이터가 없다(사용자 판단 2026-10-02) — U1) · **챗 스타일 스키마 다듬기**(U2 — `chat.style.*` 는 임시).

## 2. 폴더 구조와 경로 계산

### 2-1. 최종 구조

```
<root>\              릴리스 = <exe 폴더>\data · 개발 = <워크트리>\.engram-dev(새로 시작 — 옛 .engram-data 는 건드리지 않는다, §3-2) · 테스트 = ENGRAM_DATA_DIR (현행 규칙 그대로 — discovery/src/lib.rs:84)
├─ daemon\
│  ├─ state\         지킨다: agents.json · presets.json · usage_rejects.json (+ .corrupt-*)
│  └─ run\           버려도 된다: daemon.json(잠금+발견·WS 토큰) · mcp-config\(토큰) · usage-probe\
├─ shell\
│  ├─ config\        취향(지우면 기본값): settings.json · (장래) slots\<종류>.json · themes\<이름>.json
│  ├─ state\         기억: state.json · state.crash.json(비정상 종료 뒤 답할 때까지만 — 한 개, §6-7) (+ .corrupt-*)
│  └─ run\           state.lock(셸 실행 잠금 — 쥔 동안만 의미 · §6-5 ①)
├─ webview\          WebView2 사용자 데이터 폴더
└─ logs\             daemon-*.log · app-*.log (위치 현행 그대로)
```

### 2-2. `DataLayout` — 경로의 단일 출처 [고름]

- **자리 = `engram-dashboard-discovery` 의 새 모듈 `layout`.** 그 crate 가 이미 `default_data_dir` 의 단일 출처이고(ADR-0024 · `lib.rs:84`) 데몬·셸이 둘 다 의존한다. **base 기각** — 폴더 이름은 도메인 지식이라 입주 조건 위반이고 셋째 입주자는 ADR-0175 재심을 부른다. **net 은 discovery 를 의존할 수 없어**(discovery → net) 경로를 인자로 받는다.
- 모양: `DataLayout::new(root)` · `resolve()`(= `new(default_data_dir())`) · `root()` · `daemon_state_dir()` · `daemon_run_dir()` · `daemon_file()` · `mcp_config_dir()` · `usage_probe_dir()` · `shell_config_dir()` · `shell_state_dir()` · `shell_run_dir()` · `webview_dir()` · `logs_dir()` · `ensure_daemon_dirs()`. **옛 평면 경로 함수는 두지 않는다**(§3-2).
- **소유 규칙:** 디렉터리와 **둘 이상의 프로세스가 보는 파일**(`daemon.json` — 데몬이 쓰고 셸·스크립트가 읽는다)은 `DataLayout` 이 계산한다. **한 저장소만 쓰는 파일 이름은 그 저장소가 소유**하고 받은 디렉터리 안에서만 붙인다 — `agents.json`(`agent/src/persistence/mod.rs:22`) · `presets.json`(`presets.rs:19`) · `usage_rejects.json`(`reject_store.rs:20`) · `settings.json`·`state.json`(셸 저장 계층). 이름까지 끌어오면 agent crate 가 discovery 를 의존해야 한다.
- 바뀌는 시그니처: net `instance::acquire(data_dir)` → `acquire(lock_file: &Path)`(지금 `instance.rs:229` 가 안에서 join) · 데몬 `mcp_config::*(data_dir, …)` → `(mcp_dir, …)`(`mcp_config.rs:48-52,186-187`) · base `logging::init_logging_with_file(data_dir, kind)` → `(logs_dir, kind)`(`logging/mod.rs:87,355`). discovery 공개 함수(`ensure_daemon`·`daemon_status`·`read_live_daemon`·`daemon_stop`·`send_stop`)는 **root 를 그대로 받고** 안에서 `DataLayout` 을 쓴다 — 셸 호출부(`commands/discovery.rs` · `daemon_client/mod.rs` · `tray/`)는 손대지 않는다.
- **플랫폼 중립:** OS 가름은 지금처럼 `default_data_dir` 안에만 있다. `DataLayout` 은 `Path::join` 뿐이다. `webview_dir()` 은 모든 OS 에서 계산하고 쓰는 쪽(§4)도 `#[cfg]` 없이 넘긴다 — 그 값을 쓰느냐는 Tauri 의 플랫폼 구현 몫이다(`tauri-2.11.3/src/manager/webview.rs:534-545` — Windows·Linux 만 강제).

## 3. 데몬 기동 · 옛 데이터 · 셸 옛 파일

### 3-1. 데몬 기동 순서

셸은 데몬 파일을 건드리지 않는다(`discovery/src/lib.rs:454-472` 「클라이언트는 daemon.json 을 지우지 않는다」의 연장).

`run()` 의 새 순서(지금 `daemon/src/lib.rs:461-659`):

1. `layout = DataLayout::resolve()` → 로그(`layout.logs_dir()`).
2. `ensure_data_dir_writable(root)` → `layout.ensure_daemon_dirs()`.
3. **잠금 하나** `acquire(layout.daemon_file())` — 현행 세 갈래 그대로(`AlreadyRunning` = 양보 exit 0 · `FileBusy`/`AccessDenied` = exit 1 · `lib.rs:498-532`). ★**먼저 뜬 새 데몬이 아직 레코드를 발행하기 전(잠금 획득 ~ 발행)이면 뒤엣것은 `AlreadyRunning` 이 아니라 `FileBusy` 로 나간다**★ — 진단 읽기가 빈 파일을 보기 때문이고 오늘도 같은 구간이 있다. 그래서 `FileBusy` 로그 문구를 「중복 데몬 아님」에서 **「다른 프로그램, 또는 아직 발행 전인 다른 데몬이 쥐고 있음」**으로 고친다(`daemon/src/lib.rs:512`).
4. 새 자리 스윕(`mcp_config_dir()` · `usage_probe_dir()`) → 배선(`FileProfileStore::new(layout.daemon_state_dir())` 등 — 지금 `:226-235`).

### 3-2. 옛 데이터 — 옮기지 않고 버린다 (사용자 결정 2026-10-02 · ADR-0264 결정 5 · 6)

- **새 코드는 옛 파일을 읽지도 옮기지도 지우지도 않는다.** 대상 = 릴리스 `<exe>\data\` 의 평면 파일(`agents.json` · `presets.json` · `usage_rejects.json` · 루트 `daemon.json` · `mcp-config\` · `usage-probe\`)과 개발 루트 `.engram-data\` 통째. 근거 = 데이터가 아직 정립되지 않았다(사용자 결정).
- **없는 것:** 첫 부팅 이전 · 완료 표지(`.migrated-v1`) · 옮겨 들이기/보존(`.legacy-*`) · 대소문자 변형 처리 · 옛 루트 `daemon.json` 배제 잠금 · 옛 run 폴더 정리 · discovery 두 자리 읽기(`pick`) · 0바이트 옛 잠금 해석 · `.engram-data` → `.engram-dev` 이름 바꾸기. 개발 루트 `.engram-dev` 는 새로 시작한다.
- **잠금은 하나**(`daemon\run\daemon.json`). discovery · 스크립트는 새 자리 하나만 읽는다.
- **옛/새 바이너리:** 상태 · 잠금 · 토큰 파일을 하나도 공유하지 않아 서로 간섭하지 않는다 — 같은 루트에서 함께 돌 수도 있고(각자 자기 명부), 옛 데이터는 새 바이너리에 보이지 않는다(의도). 하향 · 섞어 쓰기는 지원하지 않는다. 같이 쓰는 자리는 최상위 `logs\` 뿐이다.

### 3-3. (걷음 — §14-5)

옛 3판의 「state 이전 규칙」은 §3-2 로 대체됐다.

### 3-4. `.gitignore` · 스크립트 · 시험 [고름]

- `.gitignore`: `**/data/daemon/run/`(폴더 규칙 — ADR-0136 결정 3 개정, ADR-0264 결정 7) · `**/data/daemon/state/agents.json*`(`.corrupt-*` 사본도 평문 env 를 싣는다) · 개발 루트 `/.engram-dev/` 통째. **옛 줄(`.engram-data/` · `**/data/daemon.json` · `**/data/agents.json*`)은 남긴다** — 새 코드가 옛 파일을 지우지 않으므로 다른 워크트리 · 옛 배포판에 남은 폴더와 토큰 든 평면 파일이 계속 무시되어야 한다.
- 스크립트: `scripts/engram.mjs:45-66` 후보를 새 경로 하나로 · `rebuild-run-debug.bat:36` · `rebuild-run-release.bat:44` · `run-release.bat` 포트파일 경로 · `build-release.ps1:151-152` 주석 — 옛 경로 대비 갈래를 두지 않는다.
- 시험 경로 갱신: `daemon/tests/ws_e2e.rs:2390,2437,2613` · `daemon/tests/mcp_manager_lifecycle.rs:373` · `discovery/tests/stop_smoke.rs:178` · discovery `real_wmi_spawn_*` 의 운영 `daemon.json` 백업/복원(`discovery/src/lib.rs:2739,2816`) · base `tests/logging_fallback.rs` · `tests/logging_install_race.rs`(로그 인자). 이전 사례는 더하지 않는다.
- CLAUDE.md 「백엔드 모듈 맵」 discovery 줄에 `DataLayout`.

### 3-5. 셸 — `ui-settings.json` 은 읽기를 그만둘 뿐이다 (사용자 결정 2026-10-02 · U1)

지금 파일은 `{"theme":…, "windows":{label:…}}` 하나이고 자리는 루트 평면(`<root>\ui-settings.json` — `ui_settings.rs:228-236`)이다. ★**배포본(v0.1.0–v0.3.2)은 있으나 사용자가 본인뿐이라 지킬 사용자 데이터가 없다(사용자 판단 2026-10-02) — U1. 값을 어디로도 옮기지 않는다**★. 칸마다 새 집이 서는 단계에서 셸이 그 칸을 **읽지 않게** 될 뿐이다.

- **P2a — 전역 칸:** `settings.json` 은 **기본값에서 시작한다.** P2a 부터 셸은 `ui-settings.json` 의 `theme` 를 **있든 없든 못 쓸 값이든 무시한다.** ★지금은 `theme` 가 없거나 못 쓸 값이면 파일 전체를 거부한다(`ui_settings.rs:300,342-358`) — 그대로 두면 L2 동안 살아 있는 창별 칸까지 함께 버려진다★. 그래서 파서가 `theme` 를 보지 않게 한다. L2 동안 이 파일은 창별 칸의 집으로 살아 있다(§5-7 · §9-1).
- **P3d — 창별 칸:** 창 테마는 모두 비어서 시작하고(`theme.default` 를 따른다) 창별 값은 `window.setTheme` 으로만 생긴다(§5-6). P3d 부터 셸은 이 파일을 읽지도 쓰지도 않는다(쓸기 · `ui.refresh` 삭제 — §5-7). 남은 파일은 따로 다루지 않는다(U1).
- 그래서 이전 코드(`settings/migrate.rs` · `state/migrate.rs`)는 없다.

### 3-6. (삭제 — U1 · §14-7)

옛 3·4판의 「챗 스타일 localStorage 일회 가져오기」(F5)는 사용자 결정 U1 로 걷었다 — 사용자가 본인뿐이라 지킬 사용자 값이 없다(F5 = (b)).

## 4. 웹뷰 폴더 → `<root>\webview` (P4)

- **사실(소스 대조):** `WebviewWindowBuilder::data_directory(PathBuf)` 는 절대 경로를 그대로 쓴다(`webview/webview_window.rs:1024` · `manager/webview.rs:534-545` — 지정이 없을 때만 `LocalData/<identifier>` 강제). 설정 키 `dataDirectory` 는 **상대 경로만** 받아 `LocalData/<label>/` 아래로 붙인다(`webview/mod.rs:392-418`).
- **결정 [고름]:** `tauri.conf.json` 두 창에 `"create": false`(`tauri-utils-2.9.3/src/config.rs:1936`) → 셸 `setup` 에서 `WebviewWindowBuilder::from_config`(`webview_window.rs:150`). Tauri 가 설정 창을 만드는 자리도 사용자 `setup` 바로 앞이다(`app.rs:2521-2531`). 창 선언은 설정에 남아 `declared_window_labels`·`hidden_window_labels` 는 그대로 돈다.
- **동일 환경 불변식(ADR-0054 확장):** 공통 마무리 함수 하나가 `data_directory(layout.webview_dir())` 와 `additional_browser_args(WEBVIEW2_BROWSER_ARGS)` 를 함께 붙이고 정적 창 둘과 팝아웃(`popout.rs:111-117`)이 전부 그것을 지난다. 브라우저 인자 값의 정본은 Rust 상수다.
- **쓰기 확인(M4 · ★명시 확인★):** 창을 만들기 **전에** `webview_dir()` 에 `check_data_dir_writable`(discovery 기존 함수)을 돌리고, 실패하면 **네이티브 대화상자**(이미 의존하는 `tauri-plugin-dialog` 의 Rust API)로 경로와 조치를 보인 뒤 종료한다 — WebView2 는 못 쓰는 폴더에서 창 없이 조용히 실패할 수 있다(ADR-0054 의 유령 창과 같은 모양) [미검]. **네트워크 공유 위 데이터 폴더는 당분간 지원하지 않는다고 문서화**한다 — ADR-0134 결정 3(공유 폴더 지원)의 개정 대상(§11).
- **덤:** `--hidden` 부팅이면 main 을 처음부터 숨긴 채 만든다(지금은 보였다 숨는다 — `lib.rs:165-169`).
- **정적 창 생성 순서(I6 · §14-10):** `from_config` 로 정적 창을 만드는 자리는 상태 부팅의 실행 표식(§6-5 ⑤) 뒤다 — 사용자 setup 은 늘 그 뒤라 순서는 저절로 서고, 만들 때 저장된 main · 트리 속성(§6-5 ⑧)을 바로 입힌다.
- **옛 폴더(`%LOCALAPPDATA%\com.engram.dashboard[.dev]`)는 다루지 않는다**(§10 F4 — U1 로 닫힘) — 지우지 않는다(같은 식별자의 다른 워크트리 · 옛 빌드가 쓰고 있을 수 있고 쓰는 중이면 반쯤만 지워진다). 사용자가 본인뿐이라 릴리스 노트 안내도 두지 않는다(U1).
- **잃는 것:** 없다. 옛 폴더의 localStorage 에 있는 키는 챗 스타일 하나뿐인데(`rg localStorage src/`) P2b 부터 아무도 읽지 않는다(§5-5) — 사용자가 본인뿐이라 옮기지 않는다(U1).

## 5. 설정 (`shell\config\settings.json`)

### 5-1. 스키마 표 [고름]

`src-tauri/src/settings/registry.rs` 의 정적 표 한 줄 = 설정 하나:

```rust
SettingDef { key: "theme.default", kind: Kind::Choice(&["dark", "light", "e-ink"]), default: "dark",
             desc: "창별 덮어쓰기가 없는 창의 테마" },
SettingDef { key: "chat.style.fontSize",
             kind: Kind::CssLength { px: (8.0, 48.0), rem: Some((0.5, 3.0)) }, default: "13px",
             desc: "챗 기본 글자 크기" },
```

- **`CssLength` 범위는 단위마다 따로 선언한다** — `px: (min, max)` 는 필수, `rem: Option<(min, max)>` 은 그 키가 rem·em 을 받을 때만(em 은 rem 과 같은 범위). 선언 안 된 단위는 `INVALID_ARGUMENT`. 숫자 범위 하나를 단위와 무관하게 대면 `48rem` 이 통과한다. 초기 표: `fontSize` px 8–48 · rem 0.5–3 / 여백·간격 8키 px 0–200 · rem 0–12.5 / `railLineOffset` px −200–200 · rem −12.5–12.5 (기본값이 rem 인 키가 있으므로 — `chatStyleStore.ts:37-49` — rem 을 막지 않는다).

- 키 = 소문자 이름공간 + 점(`theme.*` · `chat.style.*`). 마지막 마디는 기존 철자(`chat.style.fontSize`).
- **초기 키 12개:** `theme.default` + `chat.style.*` 11개(`railRowPt` · `plainRowPt` · `userPy` · `userPx` · `userMy` · `railGutter` · `railLineOffset` · `railDotTop` · `fontSize` · `lineHeight` · `waitStripH` — 기본값은 `chatStyleStore.ts:37-49` 그대로). `lineHeight` 만 `CssNumber`, `railLineOffset` 은 음수 허용.
- **기본값의 정본 = 이 표.** 프론트 `CHAT_STYLE_DEFAULTS` 는 걷는다. `theme.css` 의 `--chat-*` fallback(첫 페인트용)은 남기고, 둘을 맞추라는 지금 주석(`chatStyleStore.ts:35-36` — ADR-0051)을 표 옆으로 옮긴다 [고름 — U2]. 3판의 「표를 `settings_registry.json` 으로 내보내 vitest 가 대조」는 걷었다 — 임시 이름공간에 새 시험 장치를 더하지 않는다.
- ★**`chat.style.*` 는 임시 이름공간이다(사용자 결정 2026-10-02 · U2)**★ — 지금 11키와 기본값을 **그대로** 옮기고 가볍게 구현한다. 챗 영역은 나중에 쪼개 다시 짜고, 결국 챗 배치는 플러그인으로 자유롭게 짜게 되므로 이 키들은 갈려 나갈 예정이다. 그래서 챗 스키마 다듬기(키 이름 · 묶음 · 범위 조정)는 범위 밖이다 — 위 범위 표는 지금 값이 통과하는 넉넉한 묶음 셋에서 더 손대지 않는다.
  - **받아들인 대가(U2):** 대조 시험을 걷었으므로 (a) `theme.css` fallback 이 표의 기본값과 같은지 (b) 표의 11키가 프론트 `CSS_VAR_BY_KEY`(`chatStyleStore.ts:52`)와 같은지를 아무것도 확인하지 않는다. 키 집합이 어긋나면 `settings.set chat.style.X` 가 `changed:true` 로 답하는데 화면에는 아무 변화가 없다.
  - **나가는 길이 명령 표면에 없다:** 이 키들을 표에서 뺀 뒤에는 파일에 남은 값을 명령으로 지울 수 없다 — 저장 계층은 모르는 키를 남기고(§5-3) 모르는 키의 `reset` 은 `NOT_FOUND` 다(§5-2). 그래서 이 키들을 걷는 챗 재작성이 파일에서 지우는 일까지 맡는다.

### 5-2. wire 계약 — 값은 언제나 정규 문자열 [고름 · F1]

| 종류 | 받는 입력 | 정규형(get·set 답·알림이 싣는 값) | 파일에 적는 JSON |
|---|---|---|---|
| `Choice` | 선택지 낱말, 대소문자 무시 | 표의 철자 그대로(`e-ink`) | 문자열 |
| `CssLength` | 앞뒤 공백 허용 · 수 + 그 키가 선언한 단위(대소문자 무시) · 그 단위의 범위 안 | 최단 십진 + 소문자 단위(`15px` · `0.9rem` · `-1rem`), `+` 없음 | 문자열 |
| `CssNumber` | 단위 없는 수 | 최단 십진(`1.45`) | 문자열 |
| `Bool`(장래) | `true`/`false` 대소문자 무시 | `true`/`false` | bool |
| `Int`(장래) | 십진 정수 | 십진 정수 | 수 |

- **오류:** 모르는 키 = `NOT_FOUND` · 형식·범위 위반 = `INVALID_ARGUMENT`(기대 형식을 문구에) · 디스크 쓰기 실패 = `INTERNAL`(메모리 · `rev` 불변 · 알림 없음).
- **쓸지는 디스크로, `changed` 는 유효 값으로 가른다** — 유효 값이 지금과 같으면 `changed:false` · `rev` 그대로 · 알림 없음이되, 파일이 목표와 다르면(밖에서 고친 값 · 접힌 못 쓸 값 · 정규형이 아닌 철자) 파일만 바로잡아 쓴다. 유효 값이 바뀌는데 파일이 이미 목표대로면 쓰지 않고 `changed:true` · 알림만 낸다. 파일을 못 읽으면(읽기 IO 실패 · 통째로 못 쓰는 파일) 유효 값이 바뀔 때만 쓴다(읽기 IO 실패면 그 쓰기는 `INTERNAL`). 기본값과 같은 값을 받으면 파일에서 그 키를 지운다(「바꾼 것만」). `reset` 도 같은 가름이다 — 선택자의 키가 파일에 하나라도 있으면 지워 쓰고, 유효 값이 바뀐 키에만 알림을 낸다.
- **한 호출 = 한 파일.** 오늘은 모든 키가 `settings.json` 이라 자동으로 성립한다. 장래 접두가 두 파일에 걸치는 `reset` 은 `INVALID_ARGUMENT` 로 거절한다(부분 실패 의미를 만들지 않는다).
- 값을 문자열 하나로 싣는 이유: 선언 매크로의 타입 알파벳에 임의 JSON 이 없다(`command/src/macros.rs:37-47`).

### 5-3. 저장 계층 — 파일을 아는 유일한 자리 [고름]

- `settings/store.rs` 만 파일 이름·위치를 안다. **경로 갈래 함수는 지금 두지 않는다** — 키가 한 파일로 다 간다. 다른 파일이 필요해지면 이 모듈 안에서만 가르므로 호출부는 무수정이다.
- 파일 = 평평한 점 키 객체 + 예약 키: `{"$version":1, "theme.default":"light"}`.
- **쓰기 = 읽고-고치고-쓰기(RMW), 쓰기 직렬화 락(`io`) 아래.** 매번 파일을 다시 읽어 받은 키만 바꾼다 — 앱 밖에서 고친 다른 키와 모르는 키를 지우지 않는다(실행 중엔 반영 안 됨 · 다음 부팅에 읽힘). **파일이 없거나 통째로 못 쓰면 새 파일을 메모리의 유효 덮어쓰기 전부에서 다시 짓는다** — 안 그러면 쓰기 한 번이 화면에 아직 보이는 다른 값을 디스크에서 지운다.
- **락 셋 — 순서 = `io` → 알림 순서(`announce`) → 상태(`state`).** `io` 는 RMW 와 `sync_all` 동안 쥔다. `state`(메모리 값 · `rev`)는 잎이고, 읽기(`get` · 유효 값 · 테마 밀기)는 이것만 잡아 진행 중인 쓰기의 디스크 대기 뒤에 줄 서지 않는다. 확정(`rev` 발급)과 알림(`events.changed`)은 `announce` 아래 한 덩이라 알림 순서 = `rev` 순서이고, 그때 `io` 는 이미 놓았다. 테마 밀기(§5-6)는 설정의 어느 락도 쥐지 않은 채 부른다.
- 원자 쓰기 = 임시 이름(`settings.json.tmp<pid>` — 같은 프로세스 안의 겹침은 `io` 가 막는다) → `sync_all` → rename(데몬 `persistence/mod.rs:57-85` · `ui_settings.rs:687` 과 같은 방식 — `write_atomic` 을 셸 공용 `fsutil.rs` 로 옮겨 설정·상태가 함께 쓴다 · P3a 에서 떠 두기 `copy_aside_at` 도 `fsutil` 로 옮긴다 — 상한 · 로그 `module` 을 인자로 받고 설정 쪽 값은 그대로, §6-2). rename 이 접근 거부 · 공유 위반 · 잠금 위반(Windows 5 · 32 · 33)이면 20 ms 간격으로 5번까지 다시 한다(백신 · 색인기가 잠깐 쥐는 경우). `ReplaceFileW` 는 쓰지 않는다(§12 R7). **쓸 원문이 64 KiB(읽기 상한)를 넘으면 쓰지 않고 `INTERNAL`** — 넘는 원문을 쓰면 다음 적재가 파일 전체를 못 쓴다고 접는다.
- **항목별 관용:** 모르는 키 = 무시하되 파일에 그대로 남긴다 · 아는 키의 못 쓸 값 = 기본값으로 접고 warn, 파일에서는 덮일 때까지 그대로 · 읽는 양 상한 64 KiB(`read_capped` 재사용) · 앞머리 UTF-8 BOM 은 무시한다(메모장 · PowerShell 이 붙인다) · `$version` 은 수 1 이어야 한다(`1.0` 도 1 · 없으면 1 로 본다).
- **통째로 못 쓰는 파일**(JSON 아님 · 객체 아님 · 상한 초과 · UTF-8 아님 · 모르는 `$version`): 메모리는 기본값, **적재는 파일을 건드리지 않는다.** 그 위의 첫 쓰기가 원본을 `settings.json.corrupt-<ms>` 로 **떠 둔 뒤**(이름이 겹치면 `-<n>` 을 덧붙인다 · 앞서 떠 둔 사본을 덮지 않는다 · 사본은 `sync_all` 까지) 그 자리를 원자적으로 갈아끼운다 — 파일이 없는 순간이 없다. 떠 두기가 실패하면 그 쓰기는 `INTERNAL` 이고 원본은 그대로다. 1 MiB 를 넘는 원본은 떠 두지 않고(error 뒤 갈아끼운다 · 상한은 뜨는 도중에도 지킨다) · 이 프로세스가 직전에 떠 둔 사본이 그 자리에 남아 있고 지금 원본과 바이트가 같으면(길이 · 수정 시각은 먼저 거르는 값일 뿐 — 갈아끼우기가 실패해 같은 원본 위에 다시 쓰는 경우) 다시 떠 두지 않는다(세션 기본값 — §10 F21). ADR-0166 결정 9 「`.corrupt` 없음」 번복 → ADR-0265. ★읽기 자체의 IO 실패는 이 무리가 아니다★ — 잠깐 잠긴 멀쩡한 파일을 덮지 않도록 그때 쓰기는 실패한다(§5-2).
- **실행 중 파일을 지워도 초기화가 아니다(세션 기본값 — §10 F20).** 파일이 없으면 쓰기가 메모리에서 다시 지으므로 다음에 파일을 쓰는 호출이 화면에 보이는 값 그대로 파일을 되살린다. 반면 멀쩡한 파일에서 손으로 지운 키는 되살리지 않는다 — 그 편집이 다음 부팅에 이긴다.
- **서비스 수명:** 빌드 전에 **읽기 전용으로** 적재해 manage 한다(웹뷰 첫 invoke 전에 존재 — ADR-0102). 적재는 로거보다 먼저 돌아 로그를 내지 않고 모아 두며, `setup` 의 `enable_writes` 가 쓰기를 켜면서 그 로그를 적재 때의 수준 그대로 한 번 낸다. 쓰기는 그 뒤에만(그 전의 쓰기 = `INTERNAL`). 첫 부팅의 파일은 없다 — 옛 `ui-settings.json` 에서 채우지 않는다(§3-5).

### 5-4. 명령 넷 — 기존 버스, 셸 표 [고름]

셸 표는 하나다(`layout/commands.rs` 「레이아웃 밖의 셸 명령도 여기 선다」 · 「모듈 하나에 블록 하나」). 그 블록에 더하고 `catalog_version` 을 올린다(지금 9 — `:76`).

| 명령 | effect | 인자 → 답 |
|---|---|---|
| `settings.get` | Read | `{key?}` → `{rev, items:[{key, value, is_default}]}` |
| `settings.set` | Write | `{key, value}` → `{rev, key, value, changed}` |
| `settings.reset` | Write | `{key}` → `{rev, reset:[key…]}` |
| `settings.schema` | Read | `{key?}` → `{items:[{key, kind, default, choices, min, max, description}]}` |

- `key` = 정확한 키 또는 점으로 끝나는 접두. `reset` 은 키 필수(전체 초기화 없음).
- **사람 경로도 같은 서비스:** `#[tauri::command] settings_get/set/reset/schema` 껍데기(ADR-0081 결정 3 「두 껍데기, 서비스 하나」). 프론트는 이 껍데기를 쓴다.
- **셸이 떠 있어야 닿는다**(오늘 `ui.refresh` 와 같은 성질 · §12 R9).

### 5-5. 변경 알림 · 프론트

- 쓰기 성공 → 상태 락을 놓은 **뒤**, 알림 순서 락 아래서 `settings:changed {rev, items}` 를 **모든 웹뷰**에 — 알림 순서 = `rev` 순서. `rev` = 유효 값이 바뀐 쓰기마다 오르는 단조 번호.
- 프론트 `src/api/settingsClient.ts`(신설): 구독 먼저 → `settings_get` 당기기(`theme/uiSettings.ts` 의 순서 조항) · **적용은 키마다 가린다** — 항목은 그 키에 마지막으로 적용한 `rev` 보다 클 때만 적용하고, 처음 보는 키는 그대로 적용한다(`settings_get` 답도 항목마다 답의 `rev` 로). 답을 통째로 버리면 그 답에만 있는 키를 잃는다.
- `chatStyleStore` 는 **적용자**가 된다: `chat.style.*` 를 CSS 변수에 붙인다(`CSS_VAR_BY_KEY` 화이트리스트는 프론트 소유). 쓰기 액션은 `settings_set`. localStorage 읽기 · 쓰기는 걷는다. 개발 손잡이 `window.__engram.chatStyle` 은 걷는다(ADR-0169 「남은 갭」 해소). ADR-0035 의 프론트 전용 예외 목록(`docs/reference/architecture-overview.md:578`)에서 `chatStyleStore` 를 뺀다.
- **첫 페인트:** 비동기(§10 F6). `main.tsx:12` 의 동기 적재는 설정 클라이언트 설치로 바뀐다.

### 5-6. 테마 — 전역은 설정, 창별은 상태

- **유효 테마(창 W) = W 의 창 테마 ?? `theme.default`.** 셸이 계산하고 기존 창별 배달(`get_ui_settings` 당기기 + `ui:settings-updated` 밀기 — `commands/settings.rs:48` · `ui_settings.rs:730`)을 그대로 쓴다 [고름]. `source` 칸은 셸이 그 파일을 놓는 P3d 에서 뺀다(파일 자체는 지우지 않는다 — §3-5).
- **밀기는 한 자리에서만 계산한다:** `theme.default` 쓰기와 `window.setTheme` 쓰기는 **각자 저장을 끝낸 뒤** 같은 함수 `push_effective_themes()` 를 부르고, 그 함수가 **테마 관문 락**(가장 바깥 — 쥔 채 설정 상태 락(`state`)을 짧게 읽고 놓고 → `ViewManager`·트리 칸 락을 짧게 읽고 놓고 → 창마다 emit) 아래서 모든 창의 유효 값을 새로 계산해 민다. 마지막 밀기 = 마지막 읽기라 두 쓰기가 엇갈려도 낡은 조합이 화면에 남지 않는다(지금 `TauriUiSettings::refresh` 의 `gate` 와 같은 수법 — `commands/settings.rs:109`).
- **바꾼 테마가 저장된다(ADR-0167 「화면 변경 미저장」 번복):** 전역 = `settings.set theme.default <값>` · 창별 = 셸 명령 **`window.setTheme {window, theme}`**(`theme:null` = 덮어쓰기 해제 — 매크로 `Option<Option<T>>`, `macros.rs:43`) · **`window.getTheme {window}`** → `{theme, effective}`. 창별 값은 그 창의 모델 항목에 들어가 **창과 같이 죽는다**(§6-3) — 부팅 쓸기가 필요 없어진다.
- **창별 테마가 「설정 명령 넷」 밖의 다섯째 길인 이유:** 그 값은 설정이 아니라 상태이고(결정), 상태 파일은 기계가 통째로 다시 쓰므로 손 편집 경로가 될 수 없다.
- **오늘 화면에는 테마를 바꾸는 UI 가 없다**(ADR-0167 결정 5 · `themeManager.apply` 호출부 = `main.tsx:20` · `uiSettings.ts:95,130` 뿐). 화면 UI 는 §10 F7.

### 5-7. 파일 편집 경로 폐지 · `ui.refresh` · 도움말

- `ui.refresh`(`layout/commands.rs:362`)는 **P3d 에서 지운다**. L2 동안은 창별 칸만 다시 읽는 일로 남는다(L2 단독 머지 때 창별 제어가 끊기지 않게).
- `prompts/engram-help.md` 구획 규칙(`:8`)이 정확히 다섯(root · mail · agent · window · theme)을 요구하고 하나라도 빠지면 파일 전체가 거부된다 → **구획 개명은 파일과 CLI 를 같은 단계에서**: `theme` → `settings`(§10 F14) · `bin/engram.rs` 의 `HELP_TOPIC_THEME`(`:243`)·필수 구획 목록·`:4331` 시험 · `help theme` 별칭 · `root` 목록 줄 · `prompts/agent-priming.md:12`. 새 바이너리 + 옛 파일 = 내장 사본(`:231` `include_str!`)이 나간다.
- 도움말 본문은 P2c(명령 넷 · 창별은 아직 파일 + `ui.refresh`) → P3c(`restore.*` · 슬롯 정지 상태의 `agent.spawn`) → P3d(`window.setTheme`/`getTheme` · `ui.refresh` 삭제)에서 고친다.

## 6. 화면 상태 (`shell\state\state.json`)

### 6-0. 저장 범위 — 남기는 것 · 버리는 것 (사용자 결정 2026-10-02)

조사에서 화면에 보이는 사실 대부분이 디스크에 남지 않고 있었다. 사용자가 항목마다 남길지를 정했다(아래 두 목록).

- **가름 규칙:** config = 사람이 정한 취향(지우면 기본값) → `settings.json`(§5) · state = 프로그램이 기록한 것(「무엇이 있나 · 어땠나」) → `state.json`(§6-1). ★**사람이 고른 값이라도 창 · 탭 · 슬롯 하나에 딸려 그것과 같이 죽는 값은 state 다**★ — **도출**(사용자 결정이 아니다): 조사 「0. 결정」 17행(창별 테마 = `state.json` 의 그 창 항목) · 19행(슬롯에 꽂힌 내용 = `state.json`) · ADR-0264 결정 3(가름 기준)에서 낸 것이다. 아래 탭 이름 · 사용량 체크 · 보기 모드를 이 규칙으로 state 에 넣었다.

**남기는 것**

| 항목 | 파일 | 지금 사는 곳 | 단계 |
|---|---|---|---|
| 팝아웃 창 | state | `ViewManager` 의 창 항목(메모리뿐) | P3a · 복원 P3b1–P3b3 |
| 탭 · 탭 순서 · 활성 탭 | state | `WindowTabs.tabs` · `.active`(`layout/manager.rs:98-107`) | P3a |
| 탭 이름 | state | `View.name`(`layout/types.rs:109`) | P3a |
| 분할 트리 구조 · 방향 · 비율 | state | `LayoutNode::Split { dir, ratio }`(`layout/types.rs:88-99`) | P3a |
| 슬롯 내용 | state | `SlotContent`(`layout/types.rs:36-55`) | P3a |
| 포커스 | state | `View.focused_slot_id`(`layout/types.rs:113`) | P3a |
| 창 위치 · 크기 · 최대화 | state | 없음 — 새로 기록한다(§6-3) | P3b1–P3b3 |
| 사용량 슬롯 claude/codex 체크 | state | `SlotContent::Usage { show_claude, show_codex }`(`layout/types.rs:49-54`) — 슬롯 내용과 함께 실린다 | P3a |
| 테마 — 전역 | config `theme.default` | `ui-settings.json` 의 `theme` — 옛 값은 다루지 않는다(U1 · §3-5) | P2a |
| 테마 — 창별 | state(창 항목 `theme`) | `ui-settings.json` 의 `windows` — 옛 값은 다루지 않는다(U1 · §3-5) | P3d |
| 챗 스타일 | config `chat.style.*`(임시 이름공간 — U2 · §5-1) | 웹뷰 localStorage `engram.chatStyle`(`src/store/chatStyleStore.ts:66`) — 옛 값은 다루지 않는다(U1) | P2b |
| **슬롯 보기 모드(터미널 / 챗)** | state(슬롯 값) | `viewStore.renderModeOverride`(`src/store/viewStore.ts:96`) — **프론트 메모리뿐 · 창(웹뷰)마다 따로** | **P3e** |

**남기지 않는 것** — 도구 묶음 펼침 · 생각 줄 펼침 · 대기 목록 펼침 · 스크롤 고정 · 목록 선택 · 입력 초안 · 트레이로 숨긴 상태. 영속 경로를 만들지 않고 다음 부팅엔 기본값이다. 트레이 숨김은 다음 부팅이 `--hidden` 인자만 따른다(§4 덤 · §6-5 ⑨). 이 밖에 이미 뺀 것 = 화면 실측값 · 에이전트 런타임(§6-1) · 트리 창 보임(F9).

**슬롯 보기 모드 — 프론트 메모리에서 셸 소유 슬롯 값으로 (P3e)**

- **지금 계약:** 프론트 전용 덮어쓰기다 — 결정 = ADR-0056(`:12` · `:29` 「프론트 전용 override 를 레버로 노출」) · ADR-0035 의 프론트 전용 예외 목록(`docs/reference/architecture-overview.md:578`). 권위 루프 밖에서 낙관적으로 쓰고(`viewStore.ts:89-95` JSDoc), 프론트가 그 칸을 지우는 자리가 다섯이다(`viewStore.ts:210-245` — 닫기 · 에이전트 배정 · 내용 교체 · 사용량 슬롯 설정 · **다른 창으로 옮기기**). 창(웹뷰)마다 따로라 버스에 올리지 않는다(근거 = `src/commands/renderModeCommands.ts:1-18` 머리 주석). 칸이 없으면 caps 에서 낸 기본값이다(`LayoutLeaf.tsx:159`).
- ★**에이전트 출력 형식과 다른 것이다**★ — 남기는 것은 슬롯의 **렌더러 덮어쓰기**다. 에이전트의 `output_format`(생성 때 정해 고정 — ADR-0078)은 건드리지 않는다.
- **바뀌는 것:** 셸이 슬롯 값으로 소유하고 invoke → emit 으로 쓴다. `state.json` 에는 `layout` 안의 그 슬롯과 함께 실린다(§6-1). 남길 값의 집합 = **§10 F19 ★명시 확인★**(기본 (a) 셋 그대로 — `terminal` · `rich`(챗) · `dom`, `src/components/slot/renderMode.ts:5`). P3e 착수 전에 답을 받는다.
- **P3 착수 전에 설계한다(이 판에서는 정하지 않는다):**
  - ① **자리** — 후보 둘. `SlotContent::Agent` 의 선택 칸이면 배정 · 내용 교체 · 사용량 설정 때 내용과 함께 사라져 지금 정리 자리 넷(닫기 포함)과 맞는다. 다른 후보는 잎 노드의 칸이다. 옮기기는 ⑥.
  - ② **wire 변경** — `SlotContent`/`LayoutNode` 와 ts-rs 바인딩이 바뀐다. §6-2 의 「IPC 타입은 바뀌지 않는다」는 모르는 내용에 대한 말이고 이 칸과는 별개다.
  - ③ **쓰기 경로 · 버스 승선 — ★①과 묶인다★.** `SlotContent::Agent` 안에 두면 `layout.setSlotContent`(내용 통째 쓰기)가 둘째 쓰기 경로가 된다 — ①의 답이 ③을 정한다. 셸 소유가 되면 버스 제외 근거(창마다 따로인 프론트 상태)가 사라진다. 올릴지는 「LLM-우선 제어」와 함께 정한다.
  - ④ **개정 도장** — ADR-0056 결정(프론트 전용 override) · ADR-0035 프론트 전용 예외 목록(§11-4).
  - ⑤ **모르는 모드 값**(새 빌드가 쓴 값) — 그 칸만 버리고(기본 유도로) 슬롯 내용은 살린다. 그냥 두면 §6-2 코덱의 원문 폴백이 슬롯 내용 통째를 「모르는 내용」으로 만든다.
  - ⑥ **다른 창으로 옮기기** — 지금은 지운다(창마다 따로라서 — `viewStore.ts:241-245`). 셸 소유가 되면 내용과 함께 따라갈 수 있다. 따라 보낼지 지울지를 정한다.
  - ⑦ **다시 쓸 주석 · 문서** — `src/commands/renderModeCommands.ts:1-18` 머리 · `src/store/viewStore.ts:89-95` · `src/components/slot/renderMode.ts:1` · `docs/reference/architecture-overview.md:575,578`(그 밖 = `rg renderModeOverride src/ docs/`).

### 6-1. 스키마 v1 [고름 — 기존 serde 와 같은 snake_case]

```json
{ "version": 1, "saved_at_ms": 1759400000000, "clean_exit": false,
  "windows": [
    { "id": "main", "kind": "main", "theme": "light",
      "bounds": {"x":80,"y":60,"w":1280,"h":800}, "maximized": false,
      "active_tab": "<uuid>", "tabs": [ { "id": "<uuid>", "name": "View 1", "focused_slot_id": "<uuid>|null", "layout": { … LayoutNode … } } ] },
    { "id": "agent-tree", "kind": "tree", "theme": null, "bounds": {…}, "maximized": false },
    { "id": "<uuid>", "kind": "popout", "theme": null, "bounds": {…}, "maximized": false, "active_tab": "…", "tabs": [ … ] } ] }
```

- `layout` = 기존 `LayoutNode` serde(`#[serde(tag="type", rename_all="snake_case")]`) · 슬롯 `content` = 기존 `SlotContent` serde. 팝아웃 `id` = 창이 처음 생길 때 뽑는 UUID(§6-3) — label 이 아니다.
- `clean_exit` = **실행 표식**(Chromium `exit_type` 과 같은 수법 — 부팅 단계의 첫 쓰기가 `false` 로 세우고(어느 창보다 먼저) 정상 종료 쓰기가 `true` 로 내린다 · §6-5 ⑤ · §6-6). `saved_at_ms` 는 `restore.status` 가 싣는다(§6-7).
- `resolved_crash_copy` = **답한 사본의 해시**(I2) — 답(수락 · 거절) 뒤 첫 쓰기부터 사본 삭제가 성공할 때까지만 싣는다(§6-4 · §6-7). 부팅은 해시가 같은 사본을 「답함」으로 보고 묻지 않고 지운다. 해시 = 64비트 FNV-1a · 16진 문자열 [고름 — 몇 줄 직접 구현: std `DefaultHasher` 는 판마다 같은 값을 약속하지 않아 디스크에 못 싣고, 신원 확인이지 보안이 아니라 새 의존성이 필요 없다].
- ~~`seq`~~ **5판에서 뺐다** [고름]. 쓰던 곳 셋이 모두 다른 수단으로 갔다 — ① 대기분 삭제 확인 → 기록기 안의 깃발 하나(§6-4 — 쓰는 쪽이 기록기 하나이고 커밋이 요청보다 먼저라, 요청을 받은 뒤 뜬 스냅숏은 커밋을 담는다) ② 고유 임시 이름 `tmp-<seq>` → `fsutil::write_atomic` 의 pid 이름(같은 프로세스 안의 쓰는 쪽이 하나) ③ 부팅 지문 → 6판에서 지문 자체가 없어졌다(셸 실행 잠금 뒤에 읽는다 — §6-5 ①). 읽는 곳이 남지 않았다.
- ~~「기본 모양」 판정(`is_default_shape`)~~ **6판에서 뺐다(사용자 결정 D5)** — 비정상 종료 뒤엔 모양과 무관하게 늘 묻는다. 사용자는 그 비교 로직을 필요 없는 복잡도로 보았다.
- **슬롯 보기 모드**는 P3e 부터 셸 소유 슬롯 값이라 `layout` 안의 그 슬롯과 함께 실린다(자리 = §6-0 — P3 착수 전 설계). L3 는 한 덩어리로 머지되므로(§9-1) v1 이 나가기 전에 칸이 더해진다 — 버전을 올리지 않는다.
- 넣지 않는 것: 화면 실측값(`canvas`·`metrics` — `manager.rs:103-105`) · 에이전트 런타임 · §6-0 「남기지 않는 것」 전부.

### 6-2. 메모리 타입과의 대응 · 모르는 것

- 영속 DTO 는 `state/schema.rs` 에 따로 둔다 — `WindowTabs` 는 serde 가 없고(`manager.rs:98-107`) `ViewSnapshot` 은 파생값을 싣는다. `View`·`LayoutNode`·`SlotContent` 는 serde 를 재사용한다.
- **모르는 슬롯 내용(ADR-0060 요건) = 영속 DTO 에만 있다.** DTO 의 내용 칸 = `Known(SlotContent) | Unknown(원문 JSON 객체)`(전용 코덱 — 아는 변형으로 안 풀리면 원문을 통째로 쥔다). 메모리 `SlotContent` 와 IPC·버스 타입은 이 일로는 **바뀌지 않는다**(보기 모드 칸은 P3e 가 따로 더한다 — §6-0) — `set_slot_content`·`layout.setSlotContent` 는 모르는 종류를 지금처럼 역직렬화에서 거절하고, ts-rs 바인딩도 그대로다.
  - 복원 때 그 슬롯은 메모리에서 `Empty` 로 서고, 원문은 `ViewManager` 의 곁표(`unknown_content: slot_id → 원문`, 프론트로 안 나간다)에 남는다. **스냅숏은 그 슬롯이 아직 있고 내용이 바뀐 적 없을 때 원문을 그대로 되돌려 쓴다** — 다른 곳을 고치고 다시 저장해도 새 버전이 쓴 내용이 지워지지 않는다. 그 슬롯에 내용을 넣거나 닫으면 곁표 항목이 지워진다(내용 변경 지점에서 무효화 [미검 — 구현 때 `tree` 의 내용 쓰기 진입점 하나로 모은다]).
  - **그 슬롯은 「점유」다**(ADR-0059 의 빈/점유 판정 — 지금은 `SlotContent::is_empty`, `layout/types.rs:37,62`). 자동 배치가 그 원문을 덮지 않게 점유 판정을 `ViewManager::slot_is_free(view, slot)` = `is_empty() && 곁표에 없음` 하나로 모으고, `resolve_spawn_slot`(`manager.rs:797,805` — `tree::first_empty_slot_id` 포함)이 그것을 쓴다. 프론트의 같은 판정(`selectOpenTarget.ts` 의 `firstEmptySlotId` · 포커스 대상)은 `ViewSnapshot` 의 새 칸 `foreign_slots: Vec<slot id>` 로 같은 답을 낸다 — `SlotContent` 와 IPC 타입은 그대로다.
  - 화면에는 「이 버전이 모르는 내용」 자리표시가 서고(빈 슬롯 메뉴로 다른 내용을 **명시적으로** 놓으면 원문은 사라진다), warn 한 줄.
- **모르는 창 종류 · 못 읽는 탭:** 그 창/탭만 건너뛰고 warn.
- **못 쓸 파일** = `version` 이 아는 것보다 큼 · JSON 아님 · UTF-8 아님 · 읽기 상한 **4 MiB** 초과(I4) → 기본 화면으로 시작하고 묻지 않는다. 원본은 부팅 단계(§6-5 ④)가 `state.json.corrupt-<ms>` 로 **떠 둔 뒤** 첫 쓰기가 그 자리를 갈아끼운다 — §5-3 과 같은 떠 두기(P3a 에서 `fsutil` 로 옮긴 같은 함수 · 이름 규칙 `-<n>` · 앞선 사본을 덮지 않음). 사본 `state.crash.json` 도 같다. 상태 쪽 떠 두기 상한 = 16 MiB [고름 — 읽기 상한보다 커야 상한 초과로 못 쓴 파일도 떠 둔다 · 설정 쪽은 1 MiB 그대로]. 떠 두기가 실패하면 log 하고 진행한다(D8).
- ★**읽기 IO 실패는 못 쓸 파일이 아니다(I3)**★ — 잠김 오류(Windows 5 · 32 · 33)면 20 ms 간격으로 5번까지 다시 읽는다(쓰기 재시도와 같은 한도 — §5-3). 그래도 실패하면 log 하고 그 파일을 **없음**으로 친다 — 떠 두기 · 지우기 · 사본 뜨기를 하나도 하지 않는다(§6-5 ③). `state.json` 이 그러면 가드다 — ⑤ 건너뜀 · 기록기 없음 · 그 실행은 상태를 저장하지 않는다(§6-5 ③ · N3 · §12 R16). 사본이 그러면 그 사본으로는 묻지 않고, 그 때문에 떠야 할 사본을 못 뜨면 같은 가드다.
- **`ViewManager::from_persisted`**(신설 · 순수): 유니크 소유 · `active ∈ tabs` · main 최소 1탭(`manager.rs` 불변식 1–4)을 다시 세운다 — 중복 view id 는 뒤엣것 버림 · `active` 가 없으면 첫 탭 · main 탭이 없으면 기본 탭 · 탭 없는 팝아웃은 창째 버림.

### 6-3. 창 신원 · 창 속성의 자리

- **영속 신원 = 창 id**(`main` · `agent-tree` 고정 · 팝아웃 = 창이 처음 생길 때 뽑는 UUID, `WindowTabs.window_id`). **runtime label 은 한 부팅 안에서만의 신원**이다 — 복원되는 팝아웃은 부팅이든 런타임 수락이든 `PopupCounter`(`popout.rs:44`)에서 **새 label** 을 받는다. 그래야 런타임 복원이 지금 떠 있는 팝아웃의 label 과 부딪히지 않는다.
- **창별 테마·위치는 `WindowTabs` 에 산다**(`theme: Option<UiTheme>` · `bounds` · `maximized`). 창 항목과 같이 죽고 락은 `ViewManager` 하나다. ADR-0167 이 경고한 오적용(창 밖에 label 키로 값을 둔 것)이 구조적으로 사라진다. 바꿀 때는 `bump_version`(`manager.rs:245` — 프론트가 낡은 알림을 거르는 레이아웃 번호)이 아니라 **새 `attrs_rev`** 를 올린다 — 창을 끌 때마다 레이아웃 번호가 튀지 않게 하고, 기록기는 둘 다 본다(§6-4).
- **트리 창(`agent-tree`)은 모델 밖이다**(`manager.rs` 헤더 「`agent-tree` 창은 이 모델 밖」). 그래서 `state/tree_attrs.rs` 의 작은 칸(테마·위치)이 따로 들고, 자기 락(잎 — 쥔 채 다른 락을 잡지 않는다)과 자기 변경 번호를 갖는다. `window.setTheme` 은 `agent-tree` 면 이쪽, 그 밖은 `WindowTabs` 로 간다.
- **위치·크기 기록:** `WindowEvent::Moved/Resized` 에서 **최소화도 최대화도 아닐 때만** `bounds` 를 갱신한다(논리 좌표). 최대화 여부는 `maximized` 로 따로 — 복원은 마지막 보통 크기로 만든 뒤 최대화한다. ★**창 게터(`outer_position` · `inner_size` · `scale_factor` · `is_minimized` · `is_maximized`)는 `ViewManager` 락을 잡기 전에 전부 부르고, 락 안에서는 값만 적는다**★ — 레이아웃 락 보유 중 OS 호출 금지(`layout/apply.rs` 머리 「락 규율」).
- **락 순서(명시):** 테마 관문 락(§5-6) **›** 설정 상태 락(`state`) · `ViewManager` 락 · 트리 칸 락. 관문은 가장 바깥이고 나머지 셋은 **서로 겹쳐 잡지 않는다**(관문 아래서도 하나씩 짧게). 겹쳐야 할 날이 와도 `ViewManager` → 설정 순서만 허용하고 설정 락을 쥔 채 `ViewManager` 락을 잡는 것은 금지다. 기록기는 락을 쥐고 디스크를 만지지 않는다(복사 뒤 놓는다).

### 6-4. 저장 — 쓰는 쪽은 하나

- **기록기(`state/saver.rs`) 스레드 하나**가 부팅 단계(§6-5 ①–⑥ — 한 스레드 · 기록기보다 먼저) 뒤의 `state.json` **유일한 쓰는 쪽**이고 사본 `state.crash.json` 을 **지우는 유일한 쪽**이다 [고름]. 다른 스레드는 요청을 보내고 답을 기다릴 뿐이다. 쓰는 스레드가 하나라 쓰기 락·순번 비교가 필요 없다.
- **주기 저장:** 0.5초마다 `ViewManager.version`(레이아웃 변경마다 +1 · 측정 보고는 안 올린다 — `manager.rs:137,245,1921`) · `attrs_rev`(§6-3) · 트리 칸 변경 번호를 짧게 읽고, 바뀌었으면 **마지막 변경 뒤 1초 조용하거나 첫 변경 뒤 5초**에 스냅숏을 **통째로 원자적으로** 다시 쓴다(D2) — 락 안에서 복사 → 락 밖에서 직렬화 → `fsutil` 원자 쓰기(임시 `state.json.tmp<pid>` → `sync_all` → rename · 잠김 재시도 — §5-3). 기록기는 부팅 첫 쓰기(§6-5 ⑤) 때의 변경 번호를 받아 시작한다 — 그 뒤 바뀐 것은 첫 주기가 싣는다.
- **상한(I4):** 직렬화 결과가 4 MiB(읽기 상한 — §6-2)를 넘으면 쓰지 않는다 — log · 요청에는 `Failed`(명령에는 `INTERNAL`). 넘는 파일을 쓰면 다음 부팅이 못 쓸 파일로 접는다.
- **rename 직전 확인 변형(I8):** `fsutil` 에 「rename 직전에 물어볼 함수」를 받는 변형을 둔다 — **첫 rename 전과 잠김 재시도마다 다시** 묻고, 서 있으면 임시 파일을 지우고 `Err` 가 아니라 **따로 「건너뜀」**을 돌려준다(쓰기 실패가 아니다 — 재시도 대상이 아니다). `write_atomic` 은 그 얇은 포장이 된다. 묻는 것 = 아래 `closed`.
- **요청 둘(채널 + 답):**
  - `Resolve { hash }` — 답(수락 · 거절)을 디스크에 붙인다(I2 · §6-7). ★**해결 칸은 기록기가 요청을 꺼낼 때 선다**★ — 보내는 쪽 원자 값으로 세우지 않는다(I8): 꺼낸 뒤 뜬 스냅숏만 그 답 뒤의 화면(수락이면 커밋)을 담는다. 곧바로 스냅숏을 써서(`resolved_crash_copy: hash` 를 싣는다) `Written` 또는 `Failed` 로 답하고, rename 이 성공하면 사본을 지운다 — ★**지우기 직전마다 사본을 다시 읽어(≤4 MiB) 그 해시가 칸의 해시와 같을 때만 지운다(N1)**★: 다르면(새 인스턴스가 뜬 새 사본) 지우지 않고 칸을 비우고, 없으면 칸을 비운다. 잠금 없이 진행한 길이나 2초 마감 경합에서 낡은 기록기가 새 사본을 지우지 못하게 한다. 지우기가 실패하면 칸을 남겨 **뒤의 성공 쓰기마다 다시 지우고**(그동안 해시를 계속 싣는다), 지워지면 칸을 비운다(해시도 더 싣지 않는다). 쓰기가 실패해도 칸은 남아 다음 성공 쓰기가 같은 일을 한다.
  - `Final` → `clean_exit:true` 스냅숏(해결 칸이 서 있으면 해시를 싣고 지우기도 다시 해 본다 — 같은 해시 재확인)을 쓰고 답한 뒤 스레드 종료.
  - **걷은 것(6판):** `Flush`(첫 쓰기는 부팅 단계가 직접 · 확정 쓰기는 `Resolve`) · `DropCrashCopy`(거절도 `Resolve`) · `Final{drop_crash_copy}`(D6 — 정상 종료는 사본을 지우지 않는다).
- **종료(`RunEvent::Exit`) = 마감 하나(2초):** `Final` 을 보내고 **같은 마감까지** 답을 기다린다. 마감을 넘기면 `closed` 표지(원자 값)를 세우고 돌아간다 — 기록기의 rename 은 위 변형이 `closed` 를 물어 서 있으면 「건너뜀」(발행 없음 · 사본도 안 지움). 결과는 직전 `clean_exit:false` 가 남아 다음 부팅이 묻는다. 갇힌 기록기가 나중에 깨어나도 발행하지 못한다. 기다림이 끝나면 `state.lock` 을 놓는다(§6-6).
- **쓰기 실패 = log 만 · 따로 다루지 않는다(사용자 결정 D8).** 다음 주기 저장이 통째로 다시 쓴다. 실행 표식 쓰기(§6-5 ⑤) 실패도 같다. 근거 = [조사](../../research/crash-session-snapshot-precedents-2026-10-02.md) 「덧붙임」 — Firefox · Chromium · VS Code · Windows Terminal 모두 로그만 남기고 계속 돈다.

### 6-5. 부팅 — 단일 인스턴스 관문 뒤 · 어느 창보다 먼저

- **사실 ①:** 단일 인스턴스 플러그인은 `.build()` 도중 자기 플러그인 setup 에서 둘째 인스턴스를 `std::process::exit(0)` 으로 끝낸다(`tauri-plugin-single-instance-2.4.2/src/platform_impl/windows.rs:55-108`). 디스크를 바꾸는 코드는 그 뒤여야 한다.
- **사실 ②(I1 근거):** 그 플러그인은 `RunEvent::Exit` 에서 뮤텍스를 놓는다(같은 파일 `:111-123`). 그리고 플러그인 사건 처리가 우리 `RunEvent::Exit` 콜백보다 **먼저** 돈다(`tauri-2.11.3/src/app.rs:1430-1432` → `on_event_loop_event` `:2644-2648`). 그래서 앞 인스턴스가 `Final`(≤2초)을 쓰는 동안 새 인스턴스가 관문을 지난다. 4판이 든 근거(앞 인스턴스의 창을 못 찾을 때 — `!hwnd.is_null()`)는 좁았다 — 뮤텍스가 풀리면 창 조회까지 가지 않는다.
- **사실 ③:** 플러그인 setup 은 `build()` 안에서 등록 순서대로 돈다(`tauri-2.11.3/src/plugin.rs:880-915` — `register` 가 등록 순서대로 쌓고 `initialize_all` 이 그 순서로 돈다 · 부르는 곳 = `app.rs:2440`). 설정 창(main · agent-tree)은 그 뒤 `RunEvent::Ready` 의 `setup()` 에서 사용자 setup **앞**에 만들어진다(`app.rs:2521-2531`).
- **그래서 부팅의 파일 일은 단일 인스턴스 플러그인 바로 뒤에 등록한 작은 플러그인(부팅 단계)의 setup 에서 한다** [고름] — 관문 뒤이고 어느 창보다 앞이다. `LayoutState` · 복원 서비스는 지금처럼 빌더에서 빈 채로 manage 하고(ADR-0102 그대로 — 첫 invoke 전에 존재), 부팅 단계가 창이 생기기 전에 그 안을 채운다. ★잠금 대기를 빌드 전에 두지 않는 이유★: 거기는 관문 앞이라, 앞 인스턴스가 멀쩡히 떠 있을 때(트레이에 있는 앱을 다시 실행) 매번 잠금을 기다리느라 넘겨주기가 3초 늦는다. 4판의 「빌드 전 읽기 + setup 지문 재확인」은 걷었다 — 잠금 뒤에 읽으므로 앞 인스턴스의 마지막 쓰기를 본다.

**부팅 단계(플러그인 setup · 한 스레드):** ★**setup 은 `Err` 를 돌려주지 않는다**★ — 어느 단계가 실패해도 log 하고 계속한다(D8 · N7 — `Err` 면 Tauri 빌드가 멈춰 앱이 아예 안 뜬다).

0. **로그 초기화(N7)** — 지금 사용자 setup 첫머리의 `init_logging_with_file`(`lib.rs` setup)을 부팅 단계 맨 앞으로 옮긴다 [고름]. 관문 뒤라 곧 끝날 둘째 인스턴스는 여전히 로그를 열지 않는다. 그래서 상태 쪽엔 「진단을 모아 뒀다가 나중에 내기」가 없다.
   - 설정의 적재 기록 되풀이(빌드 전 적재가 모아 두고 `enable_writes` 가 낸다 — §5-3)는 그대로 둔다 — `enable_writes` 는 사용자 setup 이라 로거가 이미 서 있다. 되풀이를 걷는 길(설정 적재도 이 단계로 옮겨 로거 뒤에서 바로 낸다 — 이 단계도 첫 invoke 전이다)은 있으나 P2a 코드는 손대지 않는다.
1. **잠금 `shell\run\state.lock`(I1).** 먼저 `shell\run` 폴더를 만든다(없으면 잠금 파일을 못 연다). `std::fs::File::try_lock`(배타 · 플랫폼 중립) [고름 — 이 파일은 아무도 읽지 않으므로 net `instance.rs` 머리의 「구간 잠금이 다른 프로세스의 읽기를 막는다」 사유가 해당 없다]. 잡혀 있으면 약 3초까지 다시 시도한다(앞 인스턴스의 `Final` 마감 2초 + 여유). 그래도 못 잡으면(폴더를 못 만든 경우 포함) log 하고 잠금 없이 진행한다(D8 정신). 잡은 잠금은 프로세스 내내 쥐고 `Final` 뒤에 놓는다(§6-6) — 프로세스가 죽으면 OS 가 푼다.
2. **읽기** `state.json` · `state.crash.json`(상한 4 MiB · 잠김이면 짧게 재시도 — §6-2). 남은 임시 파일(`state.json.tmp<pid>` · `state.crash.json.tmp<pid>`)은 **그 pid 가 자기거나 죽은 것만** 지운다(`engram_dashboard_base::platform::pid_alive`) — 잠금 없이 진행한 경우 살아 있는 앞 인스턴스의 쓰는 중인 파일을 지우지 않는다.
3. **판정 = 순수 함수** `decide_boot(state, crash) -> BootPlan { model: Default | Restore(doc), actions, crash_copy: None | Awaiting, carry_resolved: Option<hash>, write: bool }`:

| 사본 `state.crash.json` | `state.json` | 모델 | 동작 | 묻나 |
|---|---|---|---|---|
| 있음 · **답 없음**(해시 ≠ `resolved_crash_copy`) | 무관 | 기본 | **사본을 덮지 않는다**(D2-6) | **묻는다**(모달) |
| 있음 · **버전 초과**(새 판이 쓴 사본 — 하향) | (사본 없음으로 치고 아래 행대로) | | **지우지도 떠 두지도 덮지도 않는다**(N6) — 새 사본을 떠야 하는 행이면 못 뜬 것이라 아래 가드 | 그 사본으로는 아니오(이 판은 적용 못 한다 — §12 R3) |
| 없음 | 없음 | 기본 | — | 아니오 |
| 없음 | `clean_exit:true` | 복원(조용히 — 「항상 복원」) | — | 아니오 |
| 없음 | `clean_exit:false` | 기본 | **`state.json` 원문 → `state.crash.json`**(원자 쓰기 · ⑤보다 먼저) | **묻는다**(D5 — 모양 무관) |
| 있음 · **답함**(해시 = `resolved_crash_copy` — I2) | (사본 없음으로 치고 위 행대로) | | 지우기 직전에 다시 읽어(≤4 MiB) **해시가 같을 때만** 지운다(N1) — 새 사본을 뜰 행이면 그 쓰기가 갈아끼운다 · 못 지우면 해시를 기록기에 넘긴다(`carry_resolved`) | 그 사본으로는 아니오 |
| 못 쓸 파일(버전 초과 말고 — JSON 아님 · UTF-8 아님 · 상한 초과) | (사본 없음으로 치고 위 행대로) | | `state.crash.json.corrupt-<ms>` 로 떠 둔 뒤 지운다 | 그 사본으로는 아니오 |

   - **못 쓸 `state.json`(§6-2) = 사본 열과 무관하게 `state.json.corrupt-<ms>` 로 떠 둔다**(⑤가 갈아끼운다 · N6). 모델 · 묻나는 「`state.json` 없음」 행대로 — 사본이 답 없음이면 기본 화면 + 모달이다.
   - **읽기 IO 실패(I3)** = 그 파일을 「없음」으로 치고, 그 부팅의 동작은 하나도 하지 않는다(사본 뜨기 · 떠 두기 · 지우기 전부).
   - ★**가드 하나(N3)**★ — **ⅰ `state.json` 을 못 읽었거나(IO 실패 — 재시도 뒤) ⅱ 떠야 할 사본을 못 떴으면**(쓰기 실패 · 사본 읽기 IO 실패로 동작을 건너뜀) `write = false`: ⑤를 건너뛰고 기록기를 띄우지 않는다 · log. 이번 실행은 상태를 하나도 저장하지 않는다 — 덮으면 아직 사본으로 뜨지 못했을지 모르는 크래시 화면을 잃는다. 다음 부팅이 다시 판정한다. 모드가 아니라 이 조건 하나다(원칙의 예외 — 답하지 않은 크래시 화면을 지킨다).
   - **사본이 있으면 = 답하지 않은 것(D4 · D6):** 모달이 답 전 사람의 사용을 막고 정상 종료도 사본을 지우지 않으므로, 부팅에 사본이 있다는 것은 답이 없었다는 뜻이다 — 답한 뒤 지우기 전에 끝난 것(해시로 가린다)만 빼고. 그래서 `state.json` 이 무엇이든 기본 화면 + 모달이다 — 너무 새 판의 사본은 예외(N6). 5판의 「`clean_exit:true` + 사본 → 조용히 복원하고 묻는다」 행은 걷었다.
   - **비정상 = 늘 묻는다(D3 · D5):** 기본 화면으로 죽은 세션도 사본을 뜨고 묻는다. 「기본 모양」 판정은 없다(§6-1).
4. **동작 실행.** 사본 뜨기 = `state.json` 원문 바이트를 원자 쓰기로 `state.crash.json` 에(반쪽 사본 없음). 떠 두기 · 지우기 실패는 log 하고 진행한다(D8). 사본 뜨기 실패는 위 가드다.
5. **첫 쓰기 = 실행 표식**(`clean_exit:false` · 판정한 모델 · `carry_resolved` 해시 — 가드면 건너뜀) — 기록기가 아직 없으므로 직접 원자 쓰기. **어느 창보다 먼저다** — 조용히 복원한 화면이 창을 만들다 앱을 죽여도 다음 부팅이 비정상으로 읽는다(Chromium `exit_type` 도 시작 때 세운다 — 조사 표). 실패는 log 하고 진행(D8).
6. **채우기:** 판정한 모델을 `LayoutState` 에 · 트리 칸을 `tree_attrs` 에 넣고, **복원 서비스 상태(`none` / `awaiting` + 사본 바이트 · 해시 · 가드 여부)를 여기 한 곳에서 정하고 `restore:changed` 를 낸다(I5).** 창이 아직 없으므로 모든 창의 첫 `restore_status` 당기기가 확정된 값을 본다 — 사용자 setup 에서 정하면 Tauri 가 그 앞에서 만든 설정 창이 먼저 `none` 을 볼 수 있다(사실 ③). ★**가드 ⅱ(떠야 할 사본을 못 떴다)면 `crash_copy = awaiting` 으로 정하고, 이 실행의 복원 원천은 메모리에 쥔 `state.json` 원문 바이트다** [고름 — L2]★ — 수락은 `durable:false`(기록기가 없다), 크래시 화면은 디스크에 그대로 남아 다음 부팅이 다시 뜨고 묻는다.

**사용자 `setup`(설정 창은 이미 있다):**

7. 기록기 시작(⑤ 때의 변경 번호 · `carry_resolved` 를 받는다) — 가드면 띄우지 않는다.
8. **main · 트리 창 속성 입히기(I6)** — 저장된 위치 · 크기 · 최대화(보통 크기로 놓은 뒤 최대화) · **어느 모니터에도 안 걸치는 위치는 버린다** · 테마는 `push_effective_themes()` 한 번(§5-6). 이 함수 하나를 ⑨ · 조율자 ②④가 같이 쓴다. **실행 표식(⑤) 뒤다.** L3 동안은 Tauri 가 main 을 설정 위치로 먼저 만들므로 떴다가 옮겨진다 — ★P4 부터는 정적 창 생성(`from_config` — §4)이 이 자리로 와 저장된 속성으로 바로 만든다 · 표식 뒤라는 순서는 그대로다★.
9. 복원 모델의 팝아웃마다 창 생성(새 label · 저장된 위치 — **어느 모니터에도 안 걸치는 위치는 버린다(⑧과 같은 함수 · N5 — 4판 규칙 되살림)** · `--hidden` 이면 숨긴 채 — `show_main_ui` 가 모든 창을 보인다, `tray/actions.rs:64-77`) — 실패한 창은 모델에서 지운다(다음 주기 저장이 싣는다).
10. **파생 표 재계산 = `SubscriptionSync::resync`**(라우터 재계산 + 사용량 관심 — `commands/layout.rs:74-78`) 한 번.

- 기본 레이아웃(ADR-0222 — 단일 빈 슬롯)은 **복원할 상태가 없을 때만** 쓰인다(§11).

### 6-6. 정상 종료 표식 · Windows 종료/로그오프

- **실행 표식 = `state.json` 의 `clean_exit`.** Chromium `exit_type`(시작 때 「Crashed」 · 정상 종료 때 「Normal」)과 같은 수법이라 새 표식 파일을 두지 않는다 — 부팅 단계 ⑤가 `false` 를 세우고(어느 창보다 먼저), `Final` 이 `true` 로 내린다(§6-4). 출처 = [조사](../../research/crash-session-snapshot-precedents-2026-10-02.md) 표 「비정상 종료 판별」.
- **`RunEvent::Exit`** 에서 §6-4 의 종료 절차 → 기다림이 끝나면 `state.lock` 을 놓는다(I1 — 그사이 관문을 지난 새 인스턴스는 §6-5 ①에서 기다린다). 지금 `lib.rs:267` 은 빈 핸들러. 트레이 「완전 종료」(`app.exit(0)`)도 이 사건을 낸다(`tauri-2.11.3/src/app.rs:573-574`). ★가드(§6-5 ③ — 기록기 없음)면 `Final` 을 건너뛰고 잠금만 놓는다(N7)★.
- **로그오프/종료:** tao 는 `WM_ENDSESSION(wParam=TRUE)` 에서 루프를 파기하고(`tao-0.35.3/src/platform_impl/windows/event_loop.rs:2382-2391` — `WM_QUERYENDSESSION` 은 처리 안 함) tauri-runtime-wry 가 그것을 `RunEvent::Exit` 로 번역한다(`lib.rs:4185-4187`). 받는 창은 tao 의 사건 대상 창이고 메시지 전용이 아닌 최상위 창이라(`event_loop.rs:651-686`) 트레이 상태에서도 받는다 [미검 — 실제 로그오프 · §12 R2].
- 강제 종료(`taskkill /F` · 크래시 · 디버거 · 전원)는 표식을 못 남긴다 → 다음 부팅이 `state.json` 을 사본 한 개로 뜨고 기본 화면 + 모달로 묻는다(§6-5 · §6-7). 사본이 이미 있으면 덮지 않는다(D2-6).
- **정상 종료는 사본을 지우지 않는다(사용자 결정 D6)** — 모달이 떠 있는 채 트레이로 끝내도 다음 부팅이 다시 묻는다.

### 6-7. 비정상 종료 뒤 「복원할까요?」 — 모달 · 사람과 LLM 이 같은 핸들

- **크래시 사본 = `shell\state\state.crash.json` 한 개(D1 · D2 · D3 — §14-9).** 비정상 종료 뒤 부팅은 `state.json` 을 사본으로 한 번 뜨고 기본 화면으로 시작해 **늘 묻는다**(Chromium 식 · 거부 = Firefox 식, §11-4). 선례 = [`crash-session-snapshot-precedents-2026-10-02.md`](../../research/crash-session-snapshot-precedents-2026-10-02.md) — 크래시 스냅숏 N 개 회전 선례는 없고 성숙한 앱은 한 세대만 둔다.
- **모달(사용자 결정 D4):** main 창 웹뷰 안의 모달(`RestoreModal.tsx` 신설 · 문구 `i18n/ko.ts`) — 「이전 화면 복원」 / 「새로 시작」. 띠도 OS 대화상자도 아니다. **답할 때까지 main 창을 쓸 수 없다.** 사용자 사유: 「무시하고 진행하면 이걸 또 저장해야 되냐 … 복잡하잖아」. 다른 창(트리)은 막지 않는다 [고름 — 부팅 때 트리 창은 숨어 있고(F9) 비정상 종료 뒤엔 팝아웃이 없다]. ★**키 바인딩 디스패처(`src/commands/keybindings.ts:97` — document 단위 keydown 리스너)도 `crash_copy == awaiting` 동안 멈춘다(N4)**★ — 모달이 마우스만 막으면 단축키가 그 아래 화면을 바꾼다. ★**막는 것은 사람 UI 뿐이다 — LLM 버스 명령은 막지 않는다**★: LLM 은 관례로 다른 명령보다 먼저 `restore.answer` 를 낸다(도움말 `restore.*` 항목에 적는다 — P3c1). 버스를 막는 장치는 두지 않는다.
- **`--hidden` 부팅도 따로 다루지 않는다(사용자 결정 D7)** — 모달은 숨은 main 안에 그려져 있어 트레이로 창을 열면 거기 있다.
- **묻는 조건 = 복원 서비스 상태 `awaiting` 하나** — 부팅 단계 ⑥이 한 곳에서 정한다(I5).
- 셸 명령(셸 표): **`restore.status`**(Read) → `{crash_copy, saved_at_ms, windows, tabs}` — `crash_copy` = `none` | `awaiting` | `answered`(I2 — 5판의 `pending` 을 대신한다 · 뒤 셋은 `awaiting` 일 때 사본의 값, 아니면 `null`) · **`restore.answer {accept}`**(Write) → `{restored_windows, durable}` — `awaiting` 이 아니면(사본 없음 · 이미 답함) `CONFLICT` · **다른 답이 처리 중이면 `CONFLICT`**(조율자의 처리 중 표지 하나). Tauri 껍데기 `restore_status`/`restore_answer` 가 같은 서비스를 부르고, 바뀌면 `restore:changed` 를 main 에. 5판의 `restore_prompt_shown` 신호는 없앴다(쓰던 F12 가 D6 으로 없어졌다).
- **사본 수명 — 답만 사본을 푼다(사용자 결정 D6):**
  - **수락 · 거절** = 상태 `answered` → 기록기에 `Resolve { hash }` → 그 해시를 실은 `state.json` 쓰기 뒤 사본 삭제(못 지우면 뒤 쓰기마다 다시 — §6-4). **답은 그 쓰기로 디스크에 붙는다(I2)** — 사본을 지우기 전에 끝나도 다음 부팅이 해시로 알아보고 묻지 않고 지운다.
  - **정상 종료**(트레이 종료 포함 · 모달이 떠 있어도) = 사본 그대로 · 다음 부팅이 다시 묻는다.
  - **답하기 전에 다시 죽음** = 그대로 · 다시 묻는다(D2-6).
  - **못 쓸 사본** = 부팅이 떠 두고 지운다 · 묻지 않는다 — 단 **버전 초과 사본(하향)은 지우지 않고 남겨 둔 채 묻지 않는다**(이 판은 적용 못 한다 · N6 · §12 R3). 둘 다 §6-5 표.
  - **가드 아래(기록기 없음 — §6-5 ③)** = 그래도 묻는다. 답은 화면에만 먹고 디스크에 붙지 않는다 — `durable:false` · 사본 그대로 · 다음 부팅이 다시 묻는다.
- **런타임 수락 = 복원 조율자 하나**(`state/restore.rs`). 사본은 부팅 단계가 읽은 바이트를 서비스가 쥐고 있어 다시 읽지 않는다 [고름]:
  1. **준비(부수효과 없음):** 쥔 사본 → `from_persisted` → 새 창 묶음(팝아웃마다 `PopupCounter` 의 새 label). 실패 = 아무것도 안 바뀌고 `awaiting` 그대로 · `INTERNAL`.
  2. **새 OS 창 생성(락 밖):** 숨긴 채 · 저장된 위치가 어느 모니터에도 안 걸치면 버린다(§6-5 ⑧ 과 같은 함수 · N5). 하나라도 실패하면 만든 것을 전부 destroy 하고 끝 — `awaiting` 그대로(되돌림). 기존 `move_slot_to_window` 의 phase B(창 빌드) → phase C(모델 삽입) 규율과 같고, 그 틈의 팝아웃 페이지 당기기는 기존 재시도가 덮는다(ADR-0102) [미검 — 새 창이 모델보다 먼저 뜨는 틈].
  3. **커밋(유일한 커밋 지점 · `ViewManager` 락 하나 안):** 모델 교체(main 탭 교체 · 옛 팝아웃 항목 제거 · 새 팝아웃 삽입) · `version = 지금 version + 1` · `SubscriptionSync::resync`. 이 셋을 같은 임계구역에서 한다(재계산과 발화의 순서 — `apply.rs:73-76` 의 호출 규약).
  4. **커밋 뒤(락 밖 · 되돌리지 않음):** 탭·레이아웃 알림 → **main · 트리 창 속성을 사본 값으로**(§6-5 ⑧ 과 같은 함수 — D2 「사본 내용을 화면에」) → 새 창 보이기 → 옛 팝아웃 destroy(Destroyed 정리는 이미 모델에 없는 label 이라 재계산만 한다 — `popout.rs` `drop_window_in_model`). 여기서의 실패는 로그.
  5. **확정:** 상태 `answered` · `restore:changed` → 기록기에 `Resolve { hash }`(기록기가 꺼낸 뒤 뜬 스냅숏은 커밋을 담는다) → 마감(2초)까지 답을 기다린다. 답의 `durable` = 마감 안에 그 해시를 실은 쓰기가 성공했나.
  - **거절** = ①–④ 없이 ⑤ · `restored_windows: 0`.
- **크래시 루프 격리:** 복원한 화면이 앱을 죽이면 — 확정 쓰기 전이면 사본이 남아 다시 묻고, 확정 뒤면 그 화면을 담은 `state.json`(`clean_exit:false`)이 새 사본이 되어 묻는다. 「새로 시작」으로 끊는다. 조용히 복원한 화면이 창을 만들다 죽는 경우는 표식이 어느 창보다 먼저 서서 잡는다(§6-5 ⑤).

### 6-8. 대상 에이전트가 없는 슬롯 — 결정 그대로 두 상태 (구속)

복원은 배치와 `agent_id` 를 그대로 둔다 — 그 에이전트가 다시 뜨면 슬롯이 알아서 붙는다. 표시는 `LayoutLeaf.tsx:194-260` 의 판정을 이렇게 바꾼다(`agentPresence` — `mergeTreeNodes.ts:27-34`):

| 판정 | 표시 |
|---|---|
| 명부·프로필 목록 미수신 | 「에이전트 연결 중…」 — **이 경우에만.** 목록이 오면 반드시 아래 둘 중 하나로 넘어간다 |
| 프로필 있음 · 실행 중 아님(`reserved`) · 기억 없음 | **「정지됨」 + 「활성화」 단추** — 단추는 트리 활성화와 같은 `agentClient.spawnProfile`(LLM 은 기존 `agent.spawn`). 누르면 단추 자리 진행 표시 → 명부에 오르면 평소 화면, 실패면 사유 |
| 프로필 없음(목록 수신 뒤) | **「대상 없음」** 대기 상태 — 배치 유지 · 빈 슬롯 메뉴로 다른 내용을 놓을 수 있다 |
| 기억 있음 + 부재(이 세션에서 죽음) | ADR-0149 (B) 그대로(흐림 · 입력 막음) |

- 트레이 종료 뒤 데몬이 에이전트를 자동으로 띄우지 않으므로 둘째 줄이 복원 직후의 흔한 경우다. ADR-0149 (A) 의 「스폰 대기도 연결 중」은 이 줄로 대체된다 — 갓 만든 에이전트가 명부에 오르기 전 잠깐 「정지됨」이 보일 수 있다 [고름 — 활성화 진행 표식은 트리 컴포넌트 지역 상태(`AgentList.tsx:136`)라 슬롯이 못 본다].
- 문구는 `i18n/ko.ts` · ADR-0149 개정(§11).

## 7. 슬롯 종류별 공통 설정 — 자리만

`shell\config\slots\<종류>.json` 은 **폴더 자리를 이 문서에만 둔다.** 코드·경로 함수·빈 파일 추상화는 만들지 않는다. 첫 `slot.<종류>.*` 키를 표에 올리는 변경이 저장 계층 안에서 그 파일로 가르는 규칙을 함께 넣는다(§5-3 — 호출부 무수정). 슬롯에 꽂힌 내용(`Usage` 의 `show_claude` 등)은 상태다.

## 8. seam · 시험 (ADR-0012)

| 모듈 | 끊는 것 | 하네스 · 새 시험 |
|---|---|---|
| `DataLayout` | 없음(순수) | root → 각 경로(옛 평면 경로 없음) |
| 잠금 | net 기존 하네스 | 새 자리 `daemon\run\daemon.json` 획득 · 새 데몬 둘 경합 = 뒤엣것은 파일을 하나도 안 건드리고 끝난다(`AlreadyRunning` 또는 — 앞엣것이 발행 전이면 — `FileBusy` 둘 다 허용 · §3-1 ③) · 옛 루트 파일(평면 `agents.json` · `daemon.json`)이 있어도 읽지도 바꾸지도 않는다 |
| 설정 | 파일 = `SettingsFiles` 트레이트 · 알림 = 포트 | 종류별 정규화 표(§5-2) · 같은 유효 값 = 파일이 목표대로면 무쓰기 · 다르면 파일만 바로잡고 `rev` · 알림 없음 · 기본값이면 키 삭제 · 모르는 키 보존 · 못 읽는 파일 무손상 + 첫 쓰기 `.corrupt` · RMW 가 남의 키 보존 · `rev` 단조 · 명령 표(`src-tauri/tests/layout_commands.rs` 가짜 포트) · **L2 의 `ui-settings.json`: `theme` 가 없거나 못 쓸 값이어도 유효한 `windows` 는 그대로 적용된다(§3-5)** |
| 프론트 설정 | `invoke`/`listen` 모의 | 구독 먼저 · 키마다 `rev` 로 적용(처음 보는 키는 그대로 · `settings_get` 답도 항목마다) · 챗 스타일 적용(11키 그대로 — U2) |
| fsutil | 파일 = 임시 폴더 | **rename 직전 확인 변형: 확인이 서면 첫 rename 전이든 잠김 재시도 사이든 「건너뜀」(`Err` 아님) · 임시 파일 지움 · 재시도마다 다시 묻는다(I8)** · 떠 두기 상한 · 로그 `module` 인자(설정 쪽 값 그대로 — 기존 설정 시험 초록) · 읽기 잠김 재시도 5 × 20 ms(I3) · FNV-1a 고정 값 |
| 상태 코덱 | 없음(순수) | **`Unknown` 원문 보존 왕복: 적재 → 다른 슬롯 편집 → 재저장 → 원문 JSON 동일** · 그 슬롯에 내용을 넣으면 원문 소멸 · **곁표 슬롯 = 점유(`slot_is_free` · `resolve_spawn_slot` 이 건너뜀) · `foreign_slots` 에 실림** · 버전 초과 · 4 MiB 초과 = 못 쓸 파일 · 항목별 건너뛰기 · `from_persisted` 불변식 · `resolved_crash_copy` 왕복 · **(P3e) 보기 모드 칸 왕복 · 내용이 바뀌면 그 칸이 사라짐 — 시험 모양은 P3 착수 전 설계(§6-0) 뒤 확정** |
| 잠금 `state.lock` | 시계 = 인자 | `shell\run` 이 없으면 만들고 잡는다 · 잡힌 잠금 → 약 3초 재시도 뒤 잠금 없이 진행(log) · 그 사이 놓이면 바로 잡음 · 쥔 핸들을 놓으면 풀림 · 임시 파일: 자기 pid · 죽은 pid 만 지우고 산 남의 pid 는 남김 · **가드(기록기 없음) → 종료가 `Final` 없이 잠금만 놓는다(N7)** |
| 부팅 판정 · 부팅 단계 | 없음(순수) · 파일 = 포트 | §6-5 표 전 행 · **비정상 → 사본 = 그 `state.json` 원문 바이트 · 사본 쓰기가 첫 쓰기보다 먼저** · **기본 화면으로 비정상 종료 → 사본 · 묻는다(D5)** · **사본 + `clean_exit:true` → 기본 화면 · 묻는다(D6)** · **사본이 있는 채 다시 강제 종료 → 사본 바이트 그대로 · 다시 묻는다** · **답한 사본(해시 일치) → 묻지 않고 지움 · 못 지우면 해시를 넘김** · **읽기 IO 실패 → 동작 0회(I3)** · ★**가드(N3): `state.json` IO 실패 또는 사본 못 뜸 → ⑤ 0회 · 기록기 없음**★ · **답한 사본은 지우기 직전 해시 재확인 — 다르면 안 지움(N1)** · **버전 초과 사본 → 그대로 · 묻지 않음 · 새 사본 안 뜸(N6)** · **못 쓸 `state.json` + 답 없는 사본 → 떠 둠 + 모달(N6)** · **setup 은 어느 실패에도 `Ok`(N7)** · **가드 ⅱ(사본 못 뜸) → `awaiting` · 복원 원천 = 메모리의 `state.json` 원문 · 디스크 `state.json` 바이트 그대로(L2)** · **팝아웃 위치가 어느 모니터에도 안 걸치면 버림(N5)** · 못 쓸 파일 → `.corrupt-<ms>` · 묻지 않음 · **복원 서비스 상태가 첫 쓰기와 같은 단계에서 정해진다(I5)** |
| 기록기 | 시계 · 스냅숏 원천 · 파일 = 포트 | 디바운스 1초 · 상한 5초 · 무변경 무쓰기 · **4 MiB 초과 → 안 씀 · `Failed`** · **`Final` 마감 초과 → `closed` 뒤엔 rename 0회 · 사본 삭제 0회(갇힌 쓰기를 풀어 줘도 발행 없음)** · **해결 칸은 꺼낼 때 선다(I8): 보내기만 하고 아직 안 꺼냈으면 그사이 쓴 스냅숏엔 해시가 없다** · `Resolve` 쓰기 실패 → 다음 성공 쓰기가 해시 싣고 지움 · 지우기 실패 → 뒤 쓰기마다 다시 · **지우기 직전 해시 재확인: 사본 바이트가 칸 해시와 다르면(새 사본) 안 지우고 칸을 비움(N1)** · 지워지면 해시를 더 안 싣는다 · 쓰기 실패 = log 만(D8) |
| 복원 서비스 · 조율자 | 창 = `WindowHost` 포트(기존) · 구독 = `SubscriptionSync`(기존) · 기록기 = 포트 | 상태 셋(`none` / `awaiting` / `answered`) · `awaiting` 아니면 `answer` → `CONFLICT` · 답한 뒤 두 번째 → `CONFLICT` · **처리 중 두 번째 → `CONFLICT`** · **떠 있는 팝아웃이 같은 옛 label 을 쥔 채 수락 → 충돌 없음 · 새 label · 옛 창 destroy** · 창 생성 실패 → 되돌림 · `awaiting` 그대로 · `version` 이 정확히 +1 · resync 1회 · 수락은 main · 트리 속성도 · 수락 때 화면 밖 팝아웃 위치 버림(N5) · 가드(기록기 없음) → `durable:false` · 사본 그대로 · **가드 ⅱ: 메모리 원문으로 수락 → 화면 복원 · `durable:false` · 다음 부팅이 다시 묻는다(L2)** · **기록기 쓰기 실패 → `durable:false` · 해결 칸 유지** |
| 사본 수명 | — | **정상 종료(모달이 떠 있음) → 사본 그대로**(D6) · 거절 → 해시 쓰기 → 삭제 · **답한 뒤 지우기 전에 강제 종료 → 다음 부팅이 묻지 않고 지움** |
| 프론트 모달 | vitest(`RestoreModal`) | `awaiting` 이면 main 을 덮고 입력을 막는다 · **그동안 키 바인딩 디스패처가 멈춘다 — 단축키가 그 아래 화면을 못 바꾼다 · 답 뒤 다시 돈다(N4)** · 답 뒤 닫힘 · `restore:changed` 로 갱신 · 첫 당기기가 `awaiting` 이면 바로 그린다 |
| 창 위치 | 모니터 목록 = 인자 | 최소화·최대화 중 기록 안 함 · 화면 밖 버림 · 위치·테마 변경은 `version` 을 안 올린다(`attrs_rev`) |
| 유효 테마 밀기 | emit = 포트 | `theme.default` 쓰기와 `window.setTheme` 이 엇갈려도 마지막 밀기 = 둘 다 반영한 값 |
| 슬롯 두 상태 | vitest(`LayoutLeaf`) | 미수신 / `reserved` / 프로필 없음 / 기억 있음 네 갈래 · 「연결 중」이 목록 수신 뒤 남지 않음 |
| Tauri 결합부 | — | GUI 실측만(`/qa full`): 정적 창 생성 · 웹뷰 폴더 · 쓰기 실패 대화상자 · 팝아웃 복원 · 로그오프 · **부팅 단계 플러그인이 관문 뒤 · 어느 창보다 먼저 도는지** · **모달: `taskkill /F` → 다음 기동에 모달 → `engram restore.answer` → 사본 사라짐 · 모달이 떠 있는 채 트레이 종료 → 다음 기동에 모달 다시 · `--hidden` 기동 → 트레이로 열면 모달 · 모달이 뜬 동안 단축키가 안 먹는다** · **트레이 종료 직후 재실행 → 잠금을 기다린 뒤 조용히 복원** |

명령은 CLAUDE.md 「빌드·검증 명령」 그대로(셸 단위 = `--test lib_unit` · 데몬·discovery·base = `-- --test-threads=4`).

## 9. 단계 계획 — 매 단계 끝에 빌드·시험 초록

### 9-1. 순서 · 머지 단위

- **설정(L2)이 상태(L3)보다 먼저:** 상태 영속이 들어오면 ADR-0167 부팅 쓸기의 전제(「부팅 때 팝아웃 0개」)가 무너진다. L2 동안 창별 칸은 옛 파일 + 쓸기로 여전히 옳게 돌고, L3 가 그 칸의 집을 상태로 바꾸면서(옛 값은 다루지 않는다 — U1 · §3-5) 쓸기를 끈다.
- **웹뷰(L4)는 맨 끝에 둔다** — 3판의 근거(챗 스타일 가져오기가 옛 웹뷰 폴더 안에서만 된다)는 U1 로 사라졌다. P2b 부터 localStorage 를 아무도 읽지 않아 순서 제약은 없고, 3판 순서를 그대로 둔다 [고름].
- ★**P3 착수 전 게이트 — `/review trd` 재리뷰**★: 5판 재리뷰(설계자 BLOCK · 설계자-파괴자 FIX) 반영과 사용자 결정 D4–D8 = **6판**(§14-10). 6판 재리뷰 = 설계자 PASS · 설계자-파괴자 FIX(국소 8건) → §14-11 반영. **남은 것 = §14-11 반영분 확인**. ★게이트는 P3 전체에 걸린다 — P3a 의 기록기 · `fsutil` 변형도 이 설계에 묶인다★. 같은 자리에서 슬롯 보기 모드(§6-0 ①–⑦ · F19)도 설계한다. P2 는 이 게이트와 무관하다.
  - 경위: 이 게이트는 원래 4판의 「최근 3 개 회전 보관」(F17)을 출처로 확인하고 설계를 다시 쓰려고 걸었다. 그 확인이 전제를 무너뜨려 F17 을 철회했다(§14-9).
- **P3 단계 자르기(I7 · N8):** 파일 겹침으로 순차로 자른다 — 상태 코어 + `fsutil`(P3a) → `manager.rs` 곁표(P3b1) → 순수 부팅 판정(P3b2) → `lib.rs` 배선(P3b3) → 크래시 사본 + 답할 길(P3c1 — Rust) → 복원 모달 · 키 바인딩 멈춤(P3c2 — 프론트) → 슬롯 두 상태(P3c3) → 창별 테마(P3d) → 보기 모드(P3e). ★**사본 만들기와 답할 길은 같은 단계(P3c1)다**★ — `restore.answer` 가 버스에 서면 충족된다(P3c2 의 모달 전에도 `engram restore.answer` 로 답한다). 갈리면 개발 데이터에 답할 길 없는 사본이 눌어붙는다. 그래서 P3b2–P3b3 의 비정상 종료는 한시로 「기본 화면 · 사본 없음」이다(L3 안에서만 — master 에 서지 않는다). P3a 는 `lib.rs` 에 `mod state;` 한 줄만 둔다(배선 없음) — `lib_unit` 이 그 시험을 컴파일하고 돌린다. 단계마다 혼자 빌드 초록.
- **머지 단위:** 단계는 코더 한 명 몫이고 매 단계 초록이지만, **master 머지는 단위 끝에서만** 한다 — L2 = P2a+P2b+P2c(명령과 도움말이 같이 나간다) · L3 = P3a–P3e(기록·복원·크래시 사본·창별 테마·쓸기 제거·보기 모드가 한 덩어리로 — 기록만 있고 읽는 쪽이 없는 상태, 복원은 됐는데 쓸기가 남은 상태가 master 에 서지 않는다). **L1–L4 전체가 한 릴리스(v0.3.3)로 나간다.**
- 단계 안에서도 **중간에 멈춰도 빌드가 서게**: 새 모듈·타입을 쓰는 쪽 없이 먼저 넣고 호출부를 한 단위씩 옮긴다. 자료구조를 먼저 갈아엎고 호출부를 나중에 맞추는 순서는 금지.

### 9-2. 단계

| 단계 | 내용 | 파일(겹침 기준) | 크기 |
|---|---|---|---|
| **P1** (L1) 데이터 배치(옛 데이터 버림) | §2 · §3-1~3-4 | discovery `src/lib.rs` · `src/layout.rs`(신설) · `tests/stop_smoke.rs` · net `src/instance.rs`(+시험) · base `src/logging/mod.rs` · `tests/logging_fallback.rs` · `tests/logging_install_race.rs` · 데몬 `src/lib.rs` · `src/control/{mod.rs,mcp_config.rs}` · `src/bin/priming_smoke.rs` · `tests/ws_e2e.rs` · `tests/mcp_manager_lifecycle.rs` · `src-tauri/src/lib.rs`(로그 인자 한 줄) · `.gitignore` · `scripts/{engram.mjs,rebuild-run-debug.bat,rebuild-run-release.bat,run-release.bat,build-release.ps1}` · CLAUDE.md 모듈 맵 | M (이전 걷음으로 3판 추정 ~900줄보다 작다 — §14-5) |
| **P2a** (L2) 설정 코어 + 전역 테마(기본값에서 시작 — U1) | §5-1~5-4 · §5-6 전역 · §3-5 전역 | `src-tauri/src/settings/{mod,registry,store}.rs`(신설) · `src-tauri/src/fsutil.rs`(신설) · `ui_settings.rs` · `commands/settings.rs` · `commands/layout.rs`(`command_ports`) · `layout/commands.rs` · `lib.rs` · `src-tauri/tests/layout_commands.rs` · `src-tauri/bindings/*` | L (~1,000) |
| **P2b** (L2) 프론트 챗 스타일 — 지금 11키 그대로(U2) | §5-5 · §5-1 U2 | `src/api/settingsClient.ts`(신설) · `src/store/chatStyleStore.ts` · `src/main.tsx` · 해당 `*.test.ts` · `docs/reference/architecture-overview.md`(`:578` 예외 목록) | S (~250 — 가져오기(U1)와 `theme.css` 대조 시험(U2)을 걷어 3·4판 ~400 에서 줄었다) |
| **P2c** (L2) 도움말 · 프라이밍 | §5-7 1차 | `prompts/engram-help.md` · `prompts/agent-priming.md` · 데몬 `src/bin/engram.rs` · CLAUDE.md 「LLM-우선 제어」 갭 줄 | S (~200) |
| **P3a** (L3) 상태 코어 + `fsutil` — **배선 없음**(`lib.rs` 엔 `mod state;` 한 줄 — `lib_unit` 이 시험을 컴파일하고 돌린다) | §6-1 · §6-2 · §6-4 | `src-tauri/src/state/{mod,schema,codec,saver}.rs`(신설 — 기록기 요청 둘 · 해결 칸 · 4 MiB) · `src-tauri/src/fsutil.rs`(rename 직전 확인 변형 「건너뜀」 · 떠 두기 `copy_aside_at` 이사(상한 · `module` 인자) · 읽기 잠김 재시도 · FNV-1a) · `src-tauri/src/settings/store.rs`(떠 두기를 `fsutil` 에서 부른다 — 값 그대로) · `src-tauri/src/lib.rs`(`mod state;`) | M–L (~800) |
| **P3b1** (L3) `manager.rs` 곁표 · 창 속성 칸 | §6-2 · §6-3 | `layout/manager.rs`(`WindowTabs` 칸 · `attrs_rev` · 곁표 · `slot_is_free` · 스냅숏 · `resolve_spawn_slot` · `from_persisted`) · `layout/tree.rs`(내용 쓰기 진입점의 곁표 무효화) · `layout/types.rs`(`ViewSnapshot.foreign_slots`) · `state/tree_attrs.rs`(신설) · 바인딩 | M (~500) |
| **P3b2** (L3) 순수 부팅 판정 · 잠금 | §6-5 ①–④ | `state/{boot,lock}.rs`(신설 — `decide_boot` · 잠금 대기 · 임시 파일 pid 판정 · 파일 = 포트). ★한시: 사본 열이 없다 — 비정상 = 기본 화면 · 동작 없음(P3c1 이 더한다)★ | M (~400) |
| **P3b3** (L3) `lib.rs` 배선 — 조용한 복원 · 기록기 · 종료 | §6-5 ⓪ · ⑤–⑩ · §6-6 · §6-3 기록 | `state/boot_plugin.rs`(신설 — 부팅 단계 · setup 은 늘 `Ok`) · `lib.rs`(로그 초기화를 부팅 단계로 옮김 · 플러그인 등록(단일 인스턴스 바로 뒤) · 빈 manage · 기록기 시작 · ⑧ · 팝아웃 · resync · `RunEvent::Exit` + 잠금 해제 · Moved/Resized — 게터는 락 밖) · `layout/mod.rs` · `commands/popout.rs`(위치·숨김 인자) · CLAUDE.md 「레이아웃은 디스크 영속이 없다」 줄 | M–L (~700) |
| **P3c1** (L3) 크래시 사본 + 답할 길 — 사본 열 · 복원 서비스 · `restore.*` · 조율자 | §6-5 표 사본 열 · §6-7(프론트 빼고) | `state/{boot,boot_plugin,restore}.rs`(사본 열 · 해결 해시 · N1 재확인 · 서비스 셋 상태 · 조율자) · `layout/commands.rs`(`restore.*` · `catalog_version`) · `commands/state.rs`(신설) · `commands/mod.rs` · `lib.rs` · `prompts/engram-help.md`(`restore.*` · 「`awaiting` 이면 다른 명령보다 먼저 답한다」 관례) · `src-tauri/tests/layout_commands.rs` · 바인딩 | M–L (~650) |
| **P3c2** (L3) 복원 모달 · 키 바인딩 멈춤 · qa 바인딩 | §6-7 프론트 몫 | `src/components/layout/RestoreModal.tsx`(신설) · `src/App.tsx` · `src/commands/keybindings.ts`(`awaiting` 동안 멈춤 — N4) · `src/i18n/ko.ts` · 해당 시험 · `.claude/skill-bindings/qa.md` §full(기동 뒤 `crash_copy == awaiting` 이면 `restore.answer {accept:false}` 로 답하거나, teardown 을 강제 종료 대신 `quit_app` 으로 — F15 대가) | S–M (~350) |
| **P3c3** (L3) 슬롯 두 상태 · 곁표 슬롯 프론트 | §6-8 · §6-2 프론트 몫 | `src/components/layout/LayoutLeaf.tsx` · `src/components/agent/selectOpenTarget.ts`(`foreign_slots`) · `src/i18n/ko.ts` · 해당 시험 | S–M (~300) |
| **P3d** (L3) 창별 테마 + 은퇴(옛 값 다루지 않음 — U1) | §5-6 창별 · §3-5 창별 · §5-7 | `state/tree_attrs.rs` · `layout/commands.rs`(`window.setTheme`/`getTheme` · `ui.refresh` 삭제) · `layout/manager.rs`(테마 칸 쓰기) · `commands/settings.rs` · `ui_settings.rs`(삭제) · `lib.rs`(쓸기 호출 삭제) · `src/theme/uiSettings.ts`(+시험) · `src-tauri/tests/layout_commands.rs` · `prompts/engram-help.md` | M (~600, 절반이 삭제) |
| **P3e** (L3) 슬롯 보기 모드 셸 이전 · 영속 — ★자리 · 명령 모양은 착수 전 설계(§6-0 ①–⑦ · §9-1) · 착수 전 F19 답★ | §6-0 | (설계 뒤 확정 — 예상) `layout/types.rs` · `layout/manager.rs` · `layout/commands.rs` · `commands/layout.rs` · 바인딩 · `state/schema.rs` · `src/store/viewStore.ts` · `src/commands/renderModeCommands.ts` · `src/components/slot/renderMode.ts` · `src/components/layout/LayoutLeaf.tsx` · `docs/reference/architecture-overview.md` · 해당 시험 | M (추정 전 — 설계 뒤) |
| **P4** (L4) 웹뷰 폴더 | §4 | `src-tauri/tauri.conf.json` · `lib.rs` · `commands/popout.rs` · 공통 마무리 함수(`commands/popout.rs` 또는 새 `webview_env.rs`) · ★정적 창 생성은 실행 표식 뒤 — §6-5 ⑧ 자리(§4 · I6)★ | S–M (~350) |

- **병렬 가능:** P2b ∥ P2c(겹침 0) · P3c3 ∥ P3b2 · P3b3 · P3c1(P3b1 뒤 — 프론트 대 Rust, 겹침 0). P3c2 는 P3c1 뒤(그 바인딩을 쓴다) · P3c2 와 P3c3 은 `src/i18n/ko.ts` 가 겹쳐 순차. 나머지는 순차(`lib.rs` · `layout/commands.rs` · `manager.rs` · 도움말 공유).
- 단계마다 `/review code` → `/qa`(GUI 가 걸리면 full) → 커밋. 착수 전 되돌릴 지점 = 직전 단계 커밋.
- GUI 확인 핵심: P1 빈 `.engram-dev` 로 기동 → `daemon\state\` · `daemon\run\daemon.json` 생성 · 에이전트 만들고 재시작 → 명부 유지 · 옛 `.engram-data` 가 있어도 손대지 않음 · P2a `ui-settings.json` 의 `theme` 가 못 쓸 값이어도 창별 칸은 적용(§3-5) · `settings.set theme.default light` → 모든 창 · 재시작 유지 · P2b `chat.style.fontSize` 즉시 반영 · P3a · P3b1 · P3b2 GUI 없음(시험만) · P3b3 바꾸고 1초 뒤 파일 · 트레이 종료 → `clean_exit:true` · 재시작 → 묻지 않고 탭·분할·팝아웃·위치 복원 · 트레이 종료 직후 바로 재실행 → 잠금을 기다린 뒤 같은 복원 · P3c1 화면을 꾸민 뒤 `taskkill /F` → 다음 기동에 `shell\state\state.crash.json` + 기본 화면 · `engram restore.status` → `awaiting` · `engram restore.answer`(수락) → 화면 복원 · 사본 사라짐 · (다시 `taskkill /F` 뒤) 거절 → 사본 사라짐 · 답하기 전에 `taskkill /F` → 사본 바이트 그대로 · P3c2 `taskkill /F` → 다음 기동에 모달(main 사용 불가 · 단축키 안 먹음) → 모달로 답 → 사본 사라짐 · 모달이 떠 있는 채 트레이 종료 → 다음 기동에 모달 다시 · `--hidden` 기동 → 트레이로 열면 모달 · `/qa full` teardown 뒤 다음 기동이 막히지 않음(qa 바인딩) · P3c3 슬롯 「정지됨」→ 활성화 · P3d 창별 테마 재시작 유지 · P3e 보기 모드를 챗으로 바꾸고 재시작 → 유지 · P4 `data\webview\` · 팝아웃 유령 창 없음 · 못 쓰는 폴더 → 대화상자.

## 10. 구현 갈림길

**리뷰가 사양으로 확정한 것(갈림길 아님):** F2 · F3(옛 잠금 · 이전 실패 처리 — **옛 데이터 버림으로 무의미해졌다**, §3-2 · §14-5) · F10(모르는 내용 = 영속 DTO 전용 — §6-2) · ~~F12~~(띠가 보였으면 답 없는 정상 종료에 지운다 — **6판에서 없앴다**, D6) · F16(대상 없음 / 정지됨 두 상태 — §6-8).

**사용자 결정 D1–D3(2026-10-02 · §14-9)으로 닫힌 것:** F17(**철회** — 「최근 3 개 회전 보관(Firefox 류)」은 출처 없는 세션 기본값이었다 · 크래시 사본은 `state.crash.json` 한 개 — §6-7) · **크래시 뒤 자동 복원 여부**(번호 없이 바로 닫힘 — **늘 묻는다, Chromium 식**. 거부 = Firefox 식(첫 크래시는 조용히 복원하고 연속 크래시만 묻는다) — 사용자가 고른 근거: 지배적 관행 · 단순 · 크래시 루프 방지. Chromium 은 크래시 루프를 피하려고 크래시 뒤엔 자동 복원하지 않는다 — [조사](../../research/crash-session-snapshot-precedents-2026-10-02.md)).

**사용자 결정 D4–D8(2026-10-02 · §14-10)으로 닫힌 것:** F11(D4 · N4 — 모달과 키 바인딩 멈춤이 사람의 사용을 막아 바꿀 화면은 크래시 뒤의 기본 화면이다 · 예외는 답 전에 LLM 이 버스로 바꾼 것뿐(버스는 막지 않는다 · 관례상 먼저 답한다) → (a) 통째로 바꾼다) · F12(D6 — 없앴다: 답만 사본을 푼다) · F15(**사용자 결정 2026-10-02** — 「개발 빌드도 예외 없이 항상 묻는다 — 나중에 짜증나면 요청」 → (a) · 대가: `/qa` GUI teardown 이 강제 종료라(`scripts/launch-detached.ps1:75`) 다음 QA 기동에 모달이 뜬다 — P3c2 가 qa 바인딩을 고친다) · **묻는 표면**(D4 — main 창 웹뷰 안 모달) · **기본 모양 예외**(D5 — 없다) · **`--hidden` 부팅**(D7 — 따로 다루지 않는다) · **쓰기 실패**(D8 — log 만 · [조사](../../research/crash-session-snapshot-precedents-2026-10-02.md) 「덧붙임」).

**사용자 결정 U1(사용자가 본인뿐이라 옛 값을 다루지 않는다)로 닫힌 것:** F4(옛 웹뷰 폴더 — 지우지도 안내하지도 않는다, §4) · F5(챗 스타일 가져오기 = (b) 생략, §3-6) · F18(남는 `ui-settings.json` — 읽기를 그만둘 뿐 따로 다루지 않는다, §3-5).

| # | 갈림길 | 선택지 | 기본값 |
|---|---|---|---|
| F1 | `settings.set` 값 싣는 법(LLM 이 보는 모양) | (a) 정규 문자열 하나(§5-2) · (b) 매크로 알파벳에 임의 JSON 추가 · (c) 종류별 선택 칸 | **(a)** (추천 · 세션 기본값 — 사용자 확인 대기) |
| F6 | 설정값 첫 페인트 | (a) 비동기 + CSS fallback = 기본값 · (b) 창 생성 때 초기값 주입(새 전역) | **(a)** (추천 · 세션 기본값 — 사용자 확인 대기) |
| F7 ★명시 확인★ | 화면의 테마 바꾸기 UI | (a) 이번엔 없음(명령만) · (b) 메뉴/팔레트 항목 | **(a)** (추천 · 세션 기본값 — 사용자 확인 대기) |
| F8 | 창 위치·크기 복원 | (a) 저장·복원(보통 크기만 기록 · 화면 밖 버림) · (b) 배치만 | **(a)** (추천 · 세션 기본값 — 사용자 확인 대기) |
| F9 | 트리 창 | (a) 테마·위치만, 보임은 복원 안 함 · (b) 보임도 | **(a)** (추천 · 세션 기본값 — 사용자 확인 대기) |
| F13 | `--hidden` 부팅 때 팝아웃 | (a) 숨긴 채 만들고 main 과 함께 보임 · (b) main 을 보일 때 만듦 | **(a)** (추천 · 세션 기본값 — 사용자 확인 대기) |
| F14 | 도움말 구획 | (a) `theme` → `settings` + `help theme` 별칭 · (b) id 유지 · 본문만 교체 | **(a)** (추천 · 세션 기본값 — 사용자 확인 대기) |
| F19 ★명시 확인★ | 남길 슬롯 보기 모드 값(§6-0) | (a) 지금 셋 그대로(`terminal` · `rich` · `dom`) · (b) `terminal` · `rich` 만(`dom` 은 남기지 않는다) | **(a)** (추천 · 세션 기본값 — 사용자 확인 대기 · 있는 것을 그대로 옮겨 가장 단순 · P3e 착수 전) |
| F20 | 실행 중 `settings.json` 삭제(§5-3) | (a) 초기화가 아니다 — 다음에 파일을 쓰는 호출이 메모리의 보이는 값 그대로 되살린다(멀쩡한 파일에서 손으로 지운 키는 되살리지 않는다) · (b) 초기화로 본다 | **(a)** (세션 기본값 — 사용자 확인 대기) |
| F21 | 못 쓰는 설정 파일 떠 두기의 한도(§5-3) | (a) 1 MiB 넘는 원본은 떠 두지 않는다(error 뒤 갈아끼운다 · 뜨는 도중에도 지킨다) · 이 프로세스가 직전에 떠 둔 사본이 남아 있고 바이트가 같으면 다시 떠 두지 않는다 · (b) 크기 · 횟수와 무관하게 매번 떠 둔다 | **(a)** (세션 기본값 — 사용자 확인 대기) |
| M4 ★명시 확인★ | 웹뷰 폴더를 못 쓸 때 · 네트워크 공유 | (a) 창 만들기 전에 확인 → 네이티브 대화상자 후 종료 · 공유 폴더는 당분간 미지원으로 문서화(ADR-0134 결정 3 개정) · (b) 확인 없이 기동 | **(a)** (추천 · 세션 기본값 — 사용자 확인 대기) |

## 11. ADR 후보 (`/adr` 가 채번 — 0264 는 이 브랜치가 썼다 · 다음 번호는 채번 때 정한다)

1. **→ ADR-0264 로 박제.** 아래는 3판 후보 문안이고, 그 뒤 이전을 걷었다(§14-5 — 정본은 ADR-0264). **데이터 폴더를 컴포넌트·종류(config·state/run)로 가르고 첫 부팅에 옮긴다 · 경로 단일 출처 `DataLayout` · 옛 잠금도 쥔다 · 이전 완료 표지.** 개정 도장: **ADR-0136 결정 3**(파일 이름 규칙 원칙에 `**/data/daemon/run/` 폴더 규칙 하나를 더한다 — 경로가 구체적이라 무관한 폴더를 안 삼킨다) · ADR-0135(주소를 싣는 잠금은 여전히 하나 — 옛 자리 guard 는 배제만). 거부: **설치형(Program Files + AppData)** — 배포가 ZIP 한 덩이·한 사용자이고 설치형의 이유(여러 사용자·쓰기 보호·자동 갱신)가 지금은 해당되지 않으며 워크트리 격리를 환경변수로 따로 지켜야 한다(보고서 §2-4·§2-5) · **평면 유지** — 지킬 것·버릴 것·토큰이 섞인다 · **종류 먼저** — 사용자 결정 · **Tauri 2.12 `appDirectoriesOverride`** — 2.11.3 에 없다 · **복사 후 옛 것 유지** — 하향 바이너리가 낡은 사본으로 지운 에이전트를 되살린다 · **「둘 다 있으면 새 것」** — 실패한 이전 뒤 생긴 빈 명부가 이긴다 · **옛 자리를 지워 보는 방식** — 지운 뒤 옛 데몬이 같은 자리에 새로 만들면 둘이 뜬다 · **경로 정본을 base 에**.
2. **웹뷰 데이터 폴더 = `<root>\webview` · 정적 창을 Rust 에서 · 모든 창이 같은 폴더·같은 환경 옵션(ADR-0054 확장) · 만들기 전 쓰기 확인.** 개정 도장: **ADR-0134 결정 3**(네트워크 공유 폴더 — 당분간 미지원) · ADR-0137 「식별자가 웹뷰 폴더도 정한다」 대가 서술. 거부: 설정 키 `dataDirectory`(상대 경로만) · `appDirectoriesOverride`(업그레이드 선행) · `%LOCALAPPDATA%` 유지(포터블 원칙 위반 · 워크트리 간 localStorage 공유) · 옛 폴더 자동 삭제(F4 — U1 로 다루지 않음).
3. **설정 = 셸 `settings.json` + 범용 명령 넷 + 스키마 한 줄 등록 · 쓰는 쪽은 명령 하나.** 폐기/개정 도장: ADR-0166 결정 1·3·9 · ADR-0167 결정 3·6·7 · ADR-0051 「권위 = 프론트」 · ADR-0169 「남은 갭」 해소. 거부: **값마다 명령**(0169 가 이미 거부) · **파일 직접 편집 + `ui.refresh`**(밖의 편집자와 앱이 한 파일을 다툼 — ADR-0167 이 남긴 갈림길 중 「쓰기를 한 곳으로」를 고른다) · **localStorage** · **데몬 소유**(표시 설정은 셸, 데몬은 에이전트 정의만 — 결정) · **설정과 상태를 한 파일에**(보고서 §2-1 · orca) · **매크로에 임의 JSON**(F1) · **빈 경로 갈래 미리 깔기**(첫 키와 함께) · **옛 값 가져오기(`ui-settings.json` · 챗 스타일 localStorage)**(사용자 결정 U1 — 배포본(v0.1.0–v0.3.2)은 있으나 사용자가 본인뿐이라 지킬 사용자 데이터가 없다(사용자 판단 2026-10-02) · §3-5 · §3-6) · **챗 스타일 키 재설계**(사용자 결정 U2 — 챗 영역 재작성과 플러그인 배치로 갈려 나갈 임시 이름공간 · §5-1).
4. **화면 상태 = 셸이 쓰는 `state.json` · 영속 창 id(UUID)와 부팅마다 새 label · 항상 복원, 비정상 종료 뒤에만 묻는다 · 창별 테마·위치는 창 항목에 · 대상 없는 슬롯 두 상태.** 폐기/개정 도장: **ADR-0167 결정 1**(표시 상태 = 「데이터 + refresh」 분류 → 상태는 명령으로 쓴다) · **ADR-0167 결정 5**(`theme.set` 류 철거의 구조 근거 「host 창만 지목 가능」은 프론트 선언 명령의 성질이었다 — 셸 선언 `window.setTheme` 은 창을 인자로 지목한다) · ADR-0167 결정 6·7(쓸기 폐지) · ADR-0166 결정 3 · ADR-0060 「영속 도입 시 요건」 이행 · **ADR-0222**(기본 레이아웃 = 복원할 상태가 없을 때만) · **ADR-0149 (A)**(스폰 대기도 「연결 중」 → 「정지됨」 상태). 거부: **레이아웃을 localStorage 에**(LLM-우선 충돌 · 웹뷰 폴더에 묶임) · **`tauri-plugin-window-state`**(창 기하만 · 2.11.3 에선 `app_config_dir` 고정) · **한 파일에 전부** · **지금 SQLite**(T-14) · **창별 테마를 설정 파일에**(창 수명과 떨어져 쓸기가 다시 필요) · **이름 `session.json`**(에이전트 세션과 충돌) · **기본 꺼짐 + 설정으로 켬**(사용자가 Chrome 방식을 골랐다) · **종료 때만 저장**(보고서 §2-7) · **label 을 영속 신원으로**(런타임 수락이 떠 있는 창과 부딪힌다) · **메모리 `SlotContent::Unknown`**(IPC 가 모르는 종류를 받아들이게 되고 바인딩·망라 분기 전부로 퍼진다).
   - **4판 추가(§14-6 — 문안은 P3 착수 전 재설계 뒤 확정):** 저장 범위 표(§6-0) · 슬롯 보기 모드 = 셸 소유 슬롯 값 · 영속 — 개정 도장 후보: **ADR-0056** 결정(`:12` · `:29` — 렌더 모드 덮어쓰기는 프론트 전용) · ADR-0035 의 프론트 전용 예외 목록(`docs/reference/architecture-overview.md:578`) · 셸 소유가 되면 버스 제외 근거(창마다 따로인 프론트 상태)도 사라진다(§6-0 ③) · ADR-0078(출력 형식 생성 고정)은 그대로 — 남기는 것은 렌더러 덮어쓰기다 · ~~비정상 종료 스냅숏 최근 3 개 회전 보관 — 거부 후보: 대기분 한 파일 덮음(3판 모델 · F17)~~(5판에서 철회 — 아래 5판 추가). 거부한 대안의 근거는 사용자가 준다.
   - **5판 추가(§14-9 — 사용자 결정 D1 · D2 · D3):** 크래시 사본 = `shell\state\state.crash.json` **한 개** · 실행 표식 = `state.json` 의 `clean_exit`(Chromium `exit_type` 수법 — 부팅 첫 쓰기가 세우고 정상 종료가 내린다) · **크래시 뒤엔 늘 묻는다(자동 복원 없음)** · 답하기 전에 다시 죽으면 앞 사본을 지킨다 · 사본은 복원분이 디스크에 확정된 뒤에만 지운다. 선례 = [`crash-session-snapshot-precedents-2026-10-02.md`](../../research/crash-session-snapshot-precedents-2026-10-02.md). 거부:
     - **Firefox 식**(첫 크래시는 조용히 복원하고 연속 크래시(`max_resumed_crashes=1` 초과)만 묻는다) — 사용자 결정 D3: 지배적 관행이고 단순하며 크래시 루프를 막는다(Chromium 은 크래시 루프를 피하려고 크래시 뒤 자동 복원하지 않는다).
     - **최근 N 개(3) 회전 보관**(4판 F17) — 철회(D1): 출처 없는 세션 기본값이었고 선례가 없다 — Firefox 의 「3」은 업그레이드 백업에만 걸리고 성숙한 앱은 한 세대만 둔다.
     - **새 비정상 세션으로 사본을 덮기**(3판 대기분 모델 — 기본 모양이 아니면 덮었다) — 사용자 결정 D2-6: 앞 사본은 사용자가 아직 답하지 않은 화면이고, 그 뒤의 `state.json` 은 기본 화면에서 시작한 부분 화면이다.
   - **6판 추가(§14-10 — 사용자 결정 D4–D8):** 묻는 표면 = main 창 웹뷰 안의 **모달**(답 전 사용 불가 · 사람과 LLM 이 `restore.answer` 하나) · 답만 사본을 푼다 · 답은 `state.json` 의 해시로 디스크에 붙는다 · 셸 실행 잠금 `shell\run\state.lock`. 거부:
     - **띠(배너)** — 사용자 결정 D4: 「무시하고 진행하면 이걸 또 저장해야 되냐 … 복잡하잖아」.
     - **네이티브 OS 대화상자** — 사용자 결정 D4(웹뷰 안 모달로 정했다 · 따로 사유를 들지 않았다).
     - **기본 모양 예외**(기본 화면으로 죽은 세션은 묻지 않음 — 4·5판) — 사용자 결정 D5: 그 비교 로직은 필요 없는 복잡도다.
     - **정상 종료 때 사본 지우기**(띠가 보였으면 — F12) — 사용자 결정 D6: 답(`restore.answer`)만 사본을 푼다.
     - **`--hidden` 부팅 특별 처리** — 사용자 결정 D7: 숨은 main 안에 모달이 이미 있다.
     - **쓰기 실패 특별 처리**(「쓰지 않음」 모드 · 실행 표식 실패 대응) — 사용자 결정 D8: Firefox · Chromium · VS Code · Windows Terminal 모두 로그만 남기고 계속 돈다(조사 「덧붙임」) · 원칙 「그런 것까지 가정하면 끝도 없음」.

## 12. 위험 · 미확인

| # | 무엇 | 상태 |
|---|---|---|
| R1 | **웹뷰 폴더를 옮긴 뒤의 WebView2 실동작** — 첫 기동 시간 · 한 창이라도 폴더/인자가 어긋날 때 유령 창 여부 · 못 쓰는 폴더에서의 실패 모양 · 데이터 폴더 크기 증가(캐시가 포터블 폴더로 — 압축 복사·`build-release.ps1` clean) | [미검] — P4 GUI |
| R2 | **로그오프·종료 때 `RunEvent::Exit` 가 실제로 오고 2초 상한 안에 쓰기가 끝나는가**(§6-6 — 소스로만 확인) | [미검] — 수동 로그오프 |
| R3 | 하향 바이너리: 옛 바이너리는 옛 평면 파일만 보고 새 데몬과 잠금이 달라 함께 뜰 수 있다 — 각자 자기 명부로 돈다(§3-2) · 셸 상태(N6): 새 판이 쓴 `state.json` 은 버전 초과로 떠 두고 기본 화면 · 새 판이 쓴 사본(버전 초과)은 지우지 않고 남겨 둔 채 묻지 않는다 — 옛 판은 적용할 수 없고, 새 판으로 돌아가면 다시 묻는다 · 너무 새 사본이 있는 채 옛 판이 비정상 종료하면 그 뒤 옛 판 부팅은 모두 가드 ⅱ 에 걸린다 — ⑤가 돌지 않아 새 판이 답할 때까지 아무것도 저장되지 않는다(L3) | 지원 안 함 — 문서화 |
| R4 | 강제 종료 시 마지막 1~5초 변경 유실 · 개발 재빌드마다 복원 질문(F15 — 5판은 크래시 뒤 늘 묻는다, D3) | 설계상 수용 |
| R5 | 런타임 수락에서 새 팝아웃 창이 모델보다 먼저 뜨는 틈(§6-7 ②) · DPI 다른 모니터 사이 위치 | [미검] — P3c1 |
| R6 | 곁표 무효화 진입점이 하나로 모이는지(§6-2) — 빠지면 사용자가 바꾼 슬롯에 옛 원문이 되살아난다 | P3b1 시험이 잡는다 |
| R7 | Windows 원자 쓰기 = `rename` 교체(`ReplaceFileW` 미사용 · 보고서 §2-7). 못 읽는 파일은 옆에 떠 둔 뒤에만 덮으므로(§5-3 — 1 MiB 넘는 원본은 예외, F21) 반쪽 파일이 덮여도 원문이 남는다 | 수용 |
| R8 | (걷음 — 이전이 없다, §14-5) | — |
| R9 | 설정·창별 테마·복원 답이 **셸이 떠 있을 때만** 명령으로 닿는다 | 「LLM-우선 제어」 갭으로 기록 |
| R10 | 갓 만든 에이전트가 명부에 오르기 전 슬롯이 잠깐 「정지됨」(§6-8) | 수용 · [미검] 체감 |
| R11 | 끝나는 중인 앞 인스턴스와의 겹침 — **실재한다**: 단일 인스턴스 플러그인이 `RunEvent::Exit` 에서 뮤텍스를 놓고 그 처리가 우리 `Final` 콜백보다 먼저 돈다(§6-5 사실 ② — 소스 확인). 4판의 근거(앞 인스턴스의 창을 못 찾을 때만 · `windows.rs:72-94`)와 「[미검] 실재 여부」는 틀렸다. 대응 = 셸 실행 잠금(I1 — 부팅 단계가 판정 전에 약 3초 기다린다). 남는 것: 3초 안에 못 잡으면 잠금 없이 진행하므로 앞 인스턴스의 `Final` 과 엇갈릴 수 있다 — 어느 쪽이 이겨도 파일은 온전하고, 앞 세션의 마지막 몇 초를 잃거나 정상 종료한 세션을 한 번 묻는다 | 수용(D8 정신) · [미검] 잠금 실측 — P3b3 GUI |
| R12 | `ReplaceFileW` 없이 rename 교체 + `closed` 확인과 rename 사이의 좁은 틈 — 그 틈에 발행되는 것은 `Final` 이 쓰려던 바로 그 내용이거나 더 이른 `clean_exit:false` 라 어느 쪽도 해롭지 않다. 6판: 이 확인은 `fsutil` 변형이 rename 마다(잠김 재시도 포함) 다시 묻고 「건너뜀」을 돌려준다(I8 · §6-4) | 수용 |
| R13 | 답하기 전에 다시 죽으면 그 세션의 화면은 잃는다(사본을 덮지 않는다 — D2-6). 모달과 키 바인딩 멈춤이 사람의 사용을 막아(D4 · N4) 그 화면은 기본 화면이다 — 남는 것은 답 전에 LLM 이 버스로 바꾼 화면뿐이다(버스는 막지 않는다 · 관례상 LLM 이 먼저 `restore.answer` 를 낸다) | 수용(원칙 — 「그런 것까지 가정하면 끝도 없음」) |
| R14 | 실행 표식 쓰기가 실패하면 그 실행의 크래시가 정상 종료로 읽혀 다음 부팅이 조용히 복원할 수 있다 | 수용 — 사용자 결정 D8(따로 다루지 않는다 · 조사 「덧붙임」: 네 앱 모두 같다) |
| R15 | (걷음 — D6 · D7: `--hidden` 부팅만 이어져도 사본은 답할 때까지 남는 것이 의도다) | — |
| R16 | `state.json` 을 못 읽었거나(잠김 IO — 재시도 뒤) 사본을 못 뜬 실행은 상태를 하나도 저장하지 않는다(가드 — §6-5 ③ · N3) — 그 실행의 화면 변경을 잃고 다음 부팅이 다시 판정한다. 6판의 「IO 실패 뒤 첫 쓰기가 덮어 크래시 화면을 잃는다」 위험은 없어졌다 | 수용(드묾) |
| R17 | 부팅 단계를 단일 인스턴스 바로 뒤 플러그인의 setup 에서 돌린다(§6-5) — 소스로 확인됨(tauri 2.11.3 `plugin.rs:880-915` · `app.rs:2440` · `2521` · 설계자 재리뷰도 확인) | GUI 확인만 남음 — P3b3 |

## 13. 「0. 결정」과 대조 — 다듬은 것

- **불가능한 항목은 없다.**
- 「기존 파일은 첫 부팅 때 한 번 옮긴다」 → **뒤집혔다(사용자 결정 2026-10-02)** — 옛 데이터는 옮기지 않고 버린다. 새 코드는 옛 파일을 읽지도 지우지도 않는다(§3-2 · §14-5).
- 「접근은 범용 명령 넷뿐」 → 설정에 대해서는 그대로. **창별 테마는 상태라 `window.setTheme` 이 필요하다**(§5-6).
- 「화면에서 바꾼 테마도 저장」 → 오늘 화면에 테마 UI 가 없다(ADR-0167 결정 5). 저장되는 것은 명령으로 바꾼 테마이고 화면 UI 는 F7.
- 「챗 스타일을 localStorage 에서 여기로」 → 권위 이전은 그대로. **지금 11키와 기본값만 그대로 옮기고 `chat.style.*` 는 임시 이름공간이다**(U2 · §5-1). 옛 localStorage 값은 가져오지 않는다(U1 · §3-6).
- 「LLM 이 `ui-settings.json` 을 직접 고치던 경로를 폐지」 → 그대로. 사용자가 본인뿐이라 그 파일의 값은 다루지 않는다 — `settings.json` 은 기본값에서 시작한다(U1 · §3-5).
- 「슬롯: 꽂힌 내용 = `state.json`」 → 슬롯 보기 모드(터미널/챗 — 렌더러 덮어쓰기, ADR-0078 의 출력 형식과 별개)도 프론트 메모리에서 셸 소유 슬롯 값으로 옮겨 함께 싣는다(사용자 결정 2026-10-02 · §6-0 · P3e).
- 「복원 정책 = 항상 복원, 비정상 종료 뒤에만 묻는다(Chrome 방식) — `cleanExit` 표식」 → 그대로(표식 = `state.json` 의 `clean_exit` — §6-6). 비정상 종료 뒤의 화면은 사본 `state.crash.json` **한 개**로 떠 두고 기본 화면에서 **늘**(모양 무관 — D5) 묻는다(D2 · D3). 묻는 표면은 main 창 웹뷰 안의 **모달**이고(D4) 답만 사본을 푼다(D6 · §6-7). 4판의 「최근 3 개 회전 보관」은 철회했다(D1 · §14-9).
- 「창마다 재시작해도 유지되는 id」 → 영속 id(UUID)를 따로 두고 label 은 부팅마다 새로 뽑는다(§6-3).
- 「모르는 종류 허용(ADR-0060)」 → 영속 DTO 에서만 허용하고 원문을 보존한다. 그 슬롯은 점유로 세고 「모르는 내용」 자리표시를 그린다(§6-2).
- 「Windows 종료/로그오프 알림을 정상 종료로」 → `RunEvent::Exit` 하나로 된다 — 실행 확인 전(R2).
- 「`shell\run` — 생길 때만」 → 6판에서 셸 실행 잠금 `state.lock` 이 든다(I1 · §6-5 ①).

## 14. 리뷰 반영 (A = 설계자 · B = 파괴자)

### 14-1. 1차 → 2판 (2026-10-02 · 둘 다 FIX)

| 지적 | 반영 자리 |
|---|---|
| F16 대상 없음 / 정지됨 두 상태, 무기한 「연결 중」 금지 | §6-8 · §8 · P3c · §11-4(ADR-0149 개정) |
| F3 실패한 이전 뒤 파일이 이기지 않게 · 이동 재시도 후 중단 · 완료 표지 | §3-3 · §8 · §11-1 |
| F2 옛 자리도 `acquire` 로 쥐고 평생 보유 · 0바이트 gitignore | §3-1 ④ · §3-2 · §3-4 · §11-1 |
| F4 · F7 · M4 명시 확인 · M4 기본값(쓰기 확인 + 대화상자 · 공유 미지원) | §4 · §10 · §11-2 |
| 1 (B-H1) 빌드 전 읽기 전용 · 파일 변경은 관문 뒤 · 판정 순수 함수 | §6-5 · §5-3 「서비스 수명」 · §3-5 |
| 2 (A, B-H2) 단일 쓰는 쪽 · 순번 · 종료 join 상한 · 고유 임시 이름 | §6-4 · §8 |
| 3 (B-H3, A) 런타임 수락 = 새 label · 영속 id 분리 · version+1 · 한 임계구역 resync · 조율자 · 되돌림 · 시험 | §6-3 · §6-7 · §8 |
| 4 (A, B-M6, B-L4) 머지 단위 L2·L3 · 한 릴리스 · 쓸기 제거와 복원 동시 | §9-1 · §9-2 |
| 5 (A, B-M5) 모르는 내용 = DTO 전용 · IPC 거절 유지 · 왕복 시험 | §6-2 · §8 · §11-4 |
| 6 (A) 설정 wire 정규형 · 오류 · 한 호출 한 파일 | §5-2 |
| 7 (A) 빈 슬롯 설정 경로 추상화 삭제 | §7 · §5-3 |
| 8 (B-M2) 창 테마·위치 = `WindowTabs` · 트리 창 별도 · 락 순서 | §6-3 · §5-6 |
| 9 (B-L1) 기본에서 안 바뀐 세션은 대기분을 덮지 않음 · 띠가 보였을 때만 지움 | §6-1(3판에서 `is_default_shape` 로 대체) · §6-5 표 · §6-7 |
| 10 (B-L3) 최소화·최대화 중 위치 기록 안 함 | §6-3 · §8 |
| 11 (B-L5) discovery 두 경로 · 산 쪽 우선 | §3-2 · §8 |
| 12 (B-L2) ADR-0167 결정 1·5 · ADR-0136 결정 3 · ADR-0222 도장 · `agents.json*` | §11 · §3-4 |
| 13 (B-L6) P1 파일 목록 보강 | §3-4 · §9-2 P1 |

### 14-2. 2판 → 3판 (2026-10-02 · 파괴자 FIX · 설계자 BLOCK)

| 지적 | 반영 자리 |
|---|---|
| 1 (BLOCK) 수락 시 대기분이 디스크 확정 전에 지워짐 | §6-7 ⑤(기록기가 커밋을 담은 스냅숏을 rename 한 뒤에만 삭제 · 실패면 유지 · `durable`) · §6-4 `Flush` · §8 |
| 2 `changed` 가 창 위치·테마·포커스 사건에 오염 | §6-1(`origin`/`changed` 칸 삭제 → 내용에서 내는 `is_default_shape`) · §6-5 표 · §8(기본 부팅 → 첫 `Moved`/`Resized` → 강제 종료 → 대기분 보존) |
| 3 부팅 옮기기 실패의 경계 없음 | §6-5 실행기 ②(원본 보존 · error · 기록기 「쓰지 않음」) · §8 |
| 4 P3a 단독이 GUI 확인 중 저장분을 덮음 | §9-2(P3a 는 실행 배선 없음 · 기록기 배선을 P3b 로) |
| 5 종료 경로 · 마감 하나 · 마감 뒤 발행 금지 · `tmp-*` 청소 | §6-4(쓰는 스레드 하나로 단순화 · `Final` + 2초 마감 · `closed` 를 rename 직전 확인) · §6-5 공통 행 · §12 R12 |
| 6 `CssLength` 범위가 단위를 안 봄 | §5-1(단위별 범위 · 초기 표) · §5-2 |
| 7 `restore.answer` 동시 처리 | §6-7(처리 중이면 `CONFLICT`) · §8 |
| 8 곁표 슬롯이 자동 배치에 빈 칸으로 보임 | §6-2(`slot_is_free` · `foreign_slots`) · §8 · §9-2 P3a/P3c |
| 9 끝나는 앞 인스턴스와의 부팅 계획 엇갈림 | §6-5 실행기 ①(지문 재확인 → 덮지 않고 대기분) · §6-1 `seq` · §12 R11 |
| 10 discovery 두 경로 — 한쪽 깨짐이 다른 쪽 산 레코드를 가림 | §3-2(`pick` 우선순위) · §8 |
| 11 경합하는 새 데몬은 발행 전이면 `FileBusy` | §3-1 ③(로그 문구 수정) · §8(두 결과 허용) |
| 12 유효 테마 밀기의 엇갈림 | §5-6(`push_effective_themes` · 테마 관문 락) · §6-3 락 순서 · §8 |
| 13 `Moved`/`Resized` 게터는 레이아웃 락 밖 | §6-3 |

### 14-3. 3판 → 구현 (P1 코드 리뷰 · 2026-10-02)

> ★이 절의 이전 관련 항목은 전부 §14-5 가 걷었다 — 기록으로만 남긴다★.

- **§3-3 「표지 뒤 낙오는 보존만」을 뒤집었다** — 표지가 있어도 state 에 그 파일이 없으면 옮겨 들이고, 둘 다 있을 때만 `.legacy-<ms>` 로 보존한다. 근거 = 코드 리뷰 두 리뷰어가 독립 적출(새 배포판을 한 번 띄운 뒤 옛 `data\*` 를 복사해 넣으면 빈 명부로 뜨고 진짜 명부가 백업 이름 아래 숨는다). **사용자 확인 대기**(사용자가 체감하는 동작 — 세션이 데이터 손실을 막는 쪽으로 잠정 채택). 정본 = ADR-0264 결정 7.
- 이동 실패 재시도는 공유 위반만 · 옛 자리 0바이트 자르기 실패는 기동 중단 · 옛 run 폴더는 우리가 만든 이름만 지운다 — ADR-0264 결정 6 · 7 · 8.

### 14-4. P1 코드 리뷰 뒤 사용자 결정 (2026-10-02)

| 결정 | 반영 자리 |
|---|---|
| 폴더 이름 = 업계 표준(systemd `StateDirectory=` · `RuntimeDirectory=` · `ConfigurationDirectory=` · XDG `STATE_HOME` · `RUNTIME_DIR` · `CONFIG_HOME`) — `daemon\store\` → `daemon\state\` · 셸은 `shell\config\`(settings.json · slots\) 와 `shell\state\`(state.json) 로 가른다 · `run\` 그대로. 기준 = config 는 취향(지우면 기본값) · state 는 프로그램이 기억하는 것(에이전트 명부도). 데몬 config 는 아직 없음(장래 `daemon\config\`). 거부 = `store` 유지(표준 용어 아님) | §0 · §2-1 · §3-1 · §3-3 · §3-4 · §5 · §6 · §7 · §11-1 · ADR-0264 결정 2 · 3 |
| 개발 루트 `.engram-data` → `.engram-dev`(릴리스 이름 + `-dev` 관행 · 개발 식별자 `com.engram.dashboard.dev` 와 맞춤). 첫 부팅에 `.engram-dev` 가 없고 `.engram-data` 가 있으면 이름을 바꾸고 state/run 이전을 잇는다. ADR-0136 결정 2 번복. 거부 = `.engram-data` 유지(디버그 여부를 이름으로 못 앎) · `.data-dev`(흔한 이름이라 검색 · ignore 겹침) | §2-1 · §3-1 ⓪ · §3-4 · §9-2 GUI 확인 · ~~ADR-0264 결정 11~~ → ADR-0264 재작성 뒤 **결정 8** |
| 기존 `presets.json` 은 자리만 옮긴다 — 새 프리셋 구조 없음 | §1 「밖」 그대로 · ADR-0264 결정 3 |
| 로그는 최상위 `logs\` 유지(프로세스마다 파일이 따로 · 갈라지면 기계마다 제 종류만) | §2-1 · ADR-0264 결정 2 |

- ~~**미정(구현 몫):** 코드 식별자~~ **해소** — 코드 식별자는 `daemon_state_dir()` · `shell_config_dir()` · `shell_state_dir()` 로 정해졌고 §2-2 · §3-1 을 그 이름으로 고쳤다. 이 절에 있던 다른 두 미정(이름 바꾸기의 실행 주체 · 옛 `.engram-data/` 줄 존치)은 §14-5 가 닫았다 — 이름을 바꾸지 않고, 옛 줄은 남긴다. 위 표 둘째 줄의 「첫 부팅에 이름을 바꾸고 state/run 이전을 잇는다」도 §14-5 로 걷혔다.

### 14-5. 옛 데이터 버림 · P1 단순화 (사용자 결정 2026-10-02)

- **결정:** 「옛 데몬 데이터는 신경 쓰지 마, 날려도 됨, 아직 데이터 정립 안 됐으니」. 새 코드는 옛 평면 파일(릴리스 `<exe>\data\*`)과 개발 `.engram-data` 를 **읽지도 옮기지도 지우지도 않는다**. 이 워크트리의 `.engram-data` 는 사용자가 손으로 지웠다. 정본 = ADR-0264 결정 5 · 6 · 8.
- **걷은 것:** 첫 부팅 이전 · 완료 표지 `.migrated-v1` · 옮겨 들이기/보존(`.legacy-*`) · 대소문자 변형 규칙 · 옛 루트 `daemon.json` 배제 잠금(잠금은 `daemon\run\daemon.json` 하나) · 옛 run 폴더 정리 · discovery 두 자리 읽기(`pick`)와 0바이트 해석 · 스크립트의 옛 경로 갈래 · `.engram-data` → `.engram-dev` 이름 바꾸기(`.engram-dev` 는 새로 시작) · 데몬 `src/data_migration.rs` · ws_e2e 이전 사례. 옛/새 바이너리는 파일을 공유하지 않으므로 간섭하지 않는다(옛 데이터가 새 바이너리에 안 보이는 것은 의도).
- **P1 단순화 경위:** P1 은 3판대로 이전까지 구현됐고 코드 리뷰 3회 동안 경계 사례(대소문자 변형 · FAT 이름 바꾸기 · 링크 · 섞인 바이너리의 잠금)가 이어졌다. 그 리뷰 뒤 이 결정으로 P1 을 배치 + 단일 잠금으로 줄인다. 반영 자리 = §0 · §1 · §2-1 · §2-2 · §3-1~3-4 · §8 · §9-2 P1 · §10 · §12 R3 · R8 · §13 · ADR-0264.
- **유지:** 컴포넌트 먼저의 config/state/run 배치 · systemd/XDG 이름과 가름 기준 · `DataLayout` 단일 출처 · `daemon\run\` 내용 · `.gitignore` 규칙(옛 줄 존치 — §3-4) · `.engram-dev` 이름.
- ~~**P3 세션 기본값 — 비정상 종료 스냅숏은 덮어쓰지 않는다.** 대기분 한 파일(`state.pending.json` 덮음 — §6-5 · §6-7) 대신 **최근 N 개(예: 3)를 돌려 보관한다(Firefox 식)**. ★세션 기본값 · 설계 재리뷰 필요★ — §6-5 · §6-7 · §8 의 대기분 수명 · `restore.status` 모양 · 삭제 확인 조건을 P3 착수 전에 다시 짠다(이 판에서는 §6-5 에 가리킴만 두었다). → 4판이 N = 3 으로 적고 P3 자리에 걸었다(§14-6).~~ → **5판에서 철회(§14-9)** — 이 항목은 사용자 결정이 아니라 세션이 지어낸 기본값이었고 출처가 없었다. 크래시 사본은 한 개다.

### 14-6. P2 착수 전 반영 — 4판 (사용자 결정 2026-10-02)

| 결정 | 반영 자리 |
|---|---|
| **저장 범위** — 조사 뒤 항목별로 남길 것 · 버릴 것을 정했다. 가름 = config 는 사람이 정한 취향 · state 는 프로그램 기록이고, 인스턴스에 딸린 값은 state 다(도출 — §6-0). 슬롯 보기 모드(`renderModeOverride` — 프론트 메모리)는 셸 소유 슬롯 값으로 옮겨 영속한다(P3e · 자리 · 명령 모양은 P3 착수 전 설계) | §0 · §1 · §6-0(신설) · §6-1 · §6-2 · §8 상태 코덱 · §9-1 · §9-2 P3e(신설) · GUI 확인 P3e · §11-4 · §13 |
| **`ui-settings.json` 은 옮기지 않는다.** `settings.json` 은 기본값에서 시작하고 창별 테마도 빈 채 시작한다. 이전 코드(`settings/migrate.rs` · `state/migrate.rs`)를 걷었다. ~~P3d 뒤 남는 파일은 지우지 않는다(세션 기본값 F18)~~ → §14-7 U1 | §0 · §1 · §3 제목 · §3-5 · §5-3 · §5-6 · §8 설정 · §9-1 · §9-2 P2a · P3d · GUI 확인 · §11-3 · §13 |
| ~~챗 스타일 localStorage 일회 가져오기(§3-6)를 열린 항목으로 올렸다(F5)~~ → §14-7 U1 로 (b) 생략 확정 | §3-6 · §4 · §8 프론트 설정 · §9-1 · §9-2 P2b · §13 |
| ~~**비정상 종료 스냅숏 = 덮어쓰지 않고 최근 3 개 회전 보관**(Firefox 세션 복구 백업 류 — 실제 메커니즘은 재설계 때 출처로 확인). 세션 기본값이고 **P3 착수 전 설계 재작성 + 재리뷰 필수** — 이 판은 다시 짤 자리만 적었다~~ → **5판에서 철회(§14-9 · D1)** — 표의 「사용자 결정」 제목 아래 있었으나 사용자 결정이 아니라 세션 기본값이었다 | §0 · §6-5 ★ · §6-6 · §6-7 머리 · §9-1 게이트 · §9-2 P3b · §10 F17 · §11-4 |

- **P1 절은 손대지 않았다**(구현 `fe8b552`). P2(설정)는 이 판으로 착수할 수 있다.

### 14-7. 4판 리뷰(light · 설계자-파괴자 FIX) 반영 + 사용자 결정 U1 · U2 (2026-10-02)

**사용자 결정**

| 결정 | 반영 자리 |
|---|---|
| **U1** — 「애초에 기존데이터라는게 따로없어. 아직 릴리즈 출시하지 않아서. 그냥 배제하고 작업하면 될듯」 → 재리뷰가 「출시 전」 전제를 바로잡았다(GitHub 릴리스 v0.1.0–v0.3.2 가 공개돼 있다 · v0.3.2 = 2026-09-27) → 사용자 판단(2026-10-02) 「본인뿐임. 챙길필요없어」. 그래서 U1 = **배포본(v0.1.0–v0.3.2)은 있으나 사용자가 본인뿐이라 지킬 사용자 데이터가 없다(사용자 판단 2026-10-02)** → 옛 값 처리를 계획에서 모두 뺀다. F5 = (b) 생략(챗 스타일 가져오기 삭제) · F18 = 따로 다루지 않음(셸이 읽기를 그만둘 뿐) · F4 = 옛 웹뷰 폴더를 다루지 않음(지우지도 안내하지도 않는다 — ADR-0264 「업그레이드 안내」 줄에도 같은 메모). P1 의 실질은 그대로다(이미 옛 데이터를 버린다) | §0 · §1 · §3-5 · §3-6(삭제) · §4 · §5-5 · §6-0 표 · §8 설정 · 프론트 설정 · §9-1 · §9-2 P2a · P2b · P3d · GUI 확인 · §10 머리 · §11-2 · §11-3 · §13 · ADR-0264 영향 |
| **U2** — 「기존것만 잘 옮겨 깊게하지는 말고」 · 「가볍게 구현해 현재 있는것만」. 챗 스타일은 지금 11키와 기본값을 `chat.style.*` 로 그대로 옮긴다. `chat.style.*` 는 **임시 이름공간** — 챗 영역은 나중에 쪼개 다시 짜고, 결국 챗 배치는 플러그인으로 자유롭게 짠다. 챗 스키마 다듬기는 범위 밖. `settings_registry.json` 내보내기 + `theme.css` 대조 시험을 걷었다 [고름]. P2b 재추정 = S (~250) | §0 · §1 · §5-1 · §5-5 · §6-0 표 · §8 · §9-2 P2b · §11-3 · §13 |

**리뷰 지적**(번호 = 리뷰어 번호 — §12 의 R 과 무관)

| # | 지적 | 반영 자리 |
|---|---|---|
| R1 | P3 게이트를 P3a 에도 — P3a 가 `Flush{delete_pending}` · `seq` 를 싣는다 | §9-1 게이트(「P3 전체」) · §9-2 P3a — 5판: `delete_pending` → `drop_crash_copy` · `seq` 삭제(§14-9) · 게이트는 그대로 P3 전체 |
| R2 | F11 · F12 를 「확정 사양」이라 부르면 안 된다 — F17 재설계 대상 | §10 머리(F12) · F11 행 — 5판: F17 철회로 그 표지를 걷었다 · F12 는 사본으로 옮겨 재리뷰 대기(§14-9) |
| R3 | 보기 모드 개정 도장 = ADR-0167 이 아니라 **ADR-0056**(`:12` · `:29`) + ADR-0035 프론트 전용 예외 목록(`architecture-overview.md:578`) · 다시 쓸 주석 목록 | §6-0 지금 계약 · ④ · ⑦ · §9-2 P3e 파일 · §11-4 · 앵커 |
| R4 | 보기 모드 값 집합을 [고름] 에서 갈림길로 | §6-0 바뀌는 것 · §10 F19(신설 ★명시 확인★) · §9-2 P3e |
| R5 | L2 의 `ui-settings.json`: `theme` 는 있든 없든 못 쓸 값이든 무시 — 지금은 못 쓸 `theme` 하나가 파일 전체(창별 칸 포함)를 거부한다(`ui_settings.rs:300,342-358`) | §3-5 P2a · §8 설정 · GUI 확인 P2a |
| R6 | 보기 모드 설계 질문 — ①↔③ 결합(`layout.setSlotContent` 가 둘째 쓰기 경로) · ⑤ 모르는 모드 값은 칸만 버린다 | §6-0 ③ · ⑤ |
| R7 | 지금 정리 자리는 다섯 — 사용량 설정 · 옮기기 포함(`viewStore.ts:210-245`). 옮기기는 설계 질문으로 | §6-0 지금 계약 · ① · ⑥ |
| R8 | 「인스턴스에 딸린 값은 state」는 사용자 결정이 아니라 도출 | §6-0 가름 규칙 · §14-6 |
| R9 | 출처 없는 「25개 중 5개」 삭제 | §6-0 머리 · §14-6 |
| R10 | `state.pending.json` 에 F17 표지 · §6-7 머리 재설계 목록 확장 | §0 화면 상태 · §2-1 · §6-7 머리(§6-1 `seq` · §2-1 · §8 복원 조율자 · F11 · F12) · §8 복원 조율자 행 — 5판: `state.pending.json` 자체가 없어졌다(→ `state.crash.json` · §14-9) |
| R11 | 표기 · 게이트 문구 일치 | 머리 표기(「추천」 없는 세션 기본값 = ~~F17~~(5판 철회 — §14-9) — F19 는 근거(가장 단순)를 대므로 「추천」 · ★명시 확인★ 의 「착수 전 답」 · U1/U2) |
| R12 | 4판 결정을 조사 「0. 결정」에 짧게 비춘다 | `docs/research/storage-management-survey-2026-10-02.md` 「0. 결정」 끝 |
| R13 | 보기 모드 ≠ 출력 형식(ADR-0078) · Firefox 언급을 「류 — 출처로 확인」으로 | §6-0 · §11-4 · §13 · §6-5 ★ · §6-7 머리 · §10 F17 · §14-6 — 5판: 출처로 확인한 결과 그 Firefox 언급은 틀렸다(「3」은 업그레이드 백업) → F17 철회(§14-9) |
| R14 | §14-6 반영 자리에 P2b · P3e GUI 줄 · §11 머리의 「다음 = 0264」 | §14-6 · §11 머리 |

- §14-7 반영분은 아직 재리뷰 전이다. P2 는 이 판으로 착수할 수 있다 — P2 를 막는 사용자 답은 없고, 단계 착수 전에 답이 필요한 것은 P3 쪽(~~F17~~ · F19)이다. — 5판: F17 은 철회됐다(§14-9).

### 14-8. P2a 코드 리뷰(deep) 반영 (2026-10-02)

설정 사양을 구현 코드(`src-tauri/src/settings/{mod,store}.rs` · `fsutil.rs`)에 맞췄다. 정본 = ADR-0265 결정 3 · 4 · 6. GUI 실측은 아직 없다.

| 사양 변경 | 반영 자리 |
|---|---|
| **쓸지는 디스크로, `changed` 는 유효 값으로** — 유효 값이 같아도 파일이 목표와 다르면 파일만 바로잡는다(`rev` · 알림 없음) · 유효 값이 바뀌는데 파일이 이미 목표대로면 쓰지 않고 알림만 · 파일을 못 읽으면 유효 값이 바뀔 때만 쓴다 | §5-2 · §8 설정 |
| **락 셋 `io` → `announce` → `state`** — 읽기는 `state` 만 · 알림은 `announce` 아래(알림 순서 = `rev` 순서) · 테마 밀기는 설정의 모든 락 밖 · 테마 관문 아래 잡는 것은 `state` | §5-3 · §5-5 · §5-6 · §6-3 락 순서 |
| **파일이 없거나 통째로 못 쓰면 메모리의 유효 덮어쓰기 전부에서 다시 짓는다** — 실행 중 삭제 ≠ 초기화(F20) · 멀쩡한 파일에서 손으로 지운 키는 되살리지 않는다 | §5-3 · §10 F20 |
| **못 쓰는 파일 = 옮기기 → 떠 둔 뒤 원자 교체** — 파일이 없는 순간이 없다 · 사본 실패 = `INTERNAL` · 원본 그대로 · `-<n>` · 앞선 사본을 덮지 않는다 · 1 MiB 초과 원본(error · 뜨는 도중에도 지킨다)과 직전 사본이 남아 있고 바이트가 같은 원본은 떠 두지 않는다(F21) | §5-3 · §10 F21 · §12 R7 |
| 쓸 원문 64 KiB 초과 = `INTERNAL` · rename 재시도(5 · 32 · 33 — 20 ms × 5) · 앞머리 BOM 무시 · `$version` 은 수 1(`1.0` 포함) | §5-3 |
| 적재 로그는 로거 전이라 모아 두고 `enable_writes` 가 한 번 낸다 | §5-3 서비스 수명 |
| 소비자는 키마다 `rev` 로 적용 · 처음 보는 키는 그대로 · `settings_get` 답도 항목마다(통째로 버리지 않는다) | §5-5 · §8 프론트 설정 |

### 14-9. P3 착수 전 크래시 사본 재설계 — 5판 (사용자 결정 D1 · D2 · D3 · 2026-10-02)

> ★**이 절의 반영분은 아직 재리뷰 전이다**★ — P3 착수 전 게이트(§9-1)의 남은 일 = `/review trd` 재리뷰.
>
> ★**6판(§14-10)이 이 절의 일부를 걷거나 바꿨다** — 기본 모양 예외 · F12 · 지문 재확인 · 「쓰지 않음」 모드(사본 뜨기 실패만 남김) · `Flush` · `DropCrashCopy` · `Final{drop_crash_copy}` · 띠 · `pending`. 이 절은 5판의 기록이다★.

**경위 — 철회한 전제는 사용자가 아니라 세션에서 나왔다.** 4판의 F17 「비정상 종료 스냅숏은 덮어쓰지 않고 최근 3 개를 돌려 보관한다(Firefox 류)」는 **사용자 결정이 아니라 앞 세션이 기억으로 지어낸 세션 기본값**이었고 출처가 없었다 — 그런데도 §14-6 의 「사용자 결정」 표 안에 실렸다. 4판 리뷰(§14-7 R13)가 「류 — 출처로 확인」으로 낮췄고, P3 착수 전 게이트(§9-1)가 출처 확인을 걸었다. 그 확인 = 조사 [`crash-session-snapshot-precedents-2026-10-02.md`](../../research/crash-session-snapshot-precedents-2026-10-02.md)가 전제를 무너뜨렸다: **크래시 스냅숏 N 개 회전 선례는 없다** — Firefox 의 「3」은 업그레이드 백업에만 걸리고, 성숙한 앱(Firefox · Chromium · VS Code · Windows Terminal)은 살아 있는 파일 + 한 세대만 둔다.

**사용자 결정**

| 결정 | 반영 자리 |
|---|---|
| **D1 — F17 철회.** 「최근 3 개 회전 보관(Firefox 류)」을 걷는다 | 머리 · §0 · §2-1 · §6-5 ★ 삭제 · §6-6 · §6-7 머리 · §8 · §9-1 게이트 · §10(F17 행 삭제 → 닫힌 것) · §11-4 · §13 · §14-5 · §14-6 · §14-7 · 조사 「0. 결정」 |
| **D2 — 크래시 사본 = 정확히 한 파일 `shell\state\state.crash.json`.** 평소: 바뀔 때마다 약 1초 디바운스로 `state.json` 을 통째로 원자 쓰기(임시 + rename · P2a 공용 `fsutil::write_atomic`) · 시작 때 「실행 중」 표식, 정상 종료 때 「정상」(Chromium `exit_type`) · 정상 종료 뒤 부팅 = 조용히 복원(「항상 복원」 그대로) · 비정상 종료 뒤 부팅 = ① 무엇이든 `state.json` 을 쓰기 **전에** `state.json` → `state.crash.json` 한 번 복사(4판 대기분 `state.pending.json` 을 통째로 대신한다) ② 기본 화면으로 시작 ③ 「복원할까요?」 — 사람과 LLM 이 한 핸들(`restore.answer` 그대로) ④ 복원 = 사본 내용을 화면에 적용하고 그 상태가 `state.json` 에 쓰인 **뒤에만** 사본 삭제(3판의 「디스크 확정 뒤에만 삭제」 원칙 그대로) ⑤ 거절 = 사본 삭제 ⑥ 부팅에 사본이 이미 있으면(답하기 전에 다시 죽었다) 더 새 `state.json` 으로 **덮지 않는다** — 앞 사본이 아직 답하지 않은 것이고 새 `state.json` 은 기본 · 부분 화면이다 · 그대로 두고 다시 묻는다 · 못 쓸 `state.json` / 사본 = 빈 채 시작 · 설정처럼 `.corrupt-<ms>` 로 떠 둠 · 묻지 않음(§5-3 과 같게) | §2-1 · §6-1 · §6-2 · §6-4 · §6-5 · §6-6 · §6-7 · §8 · §9-2 P3a–P3c · GUI 확인 · §12 R11–R15 |
| **D3 — 크래시 뒤엔 늘 묻는다(Chromium 식) · 비정상 종료 뒤 자동 복원 없음.** 거부 = Firefox 식(첫 크래시는 조용히 복원하고 연속 크래시만 묻는다). 근거(사용자가 고름) = 지배적 관행 · 단순 · 크래시 루프 방지 — Chromium 은 크래시 루프를 피하려고 크래시 뒤 자동 복원하지 않는다(조사 표 「묻나」) | §6-5 · §6-7 · §10 닫힌 것 · §11-4 거부 |

**세션이 고른 것 [고름] — 재리뷰가 볼 것**

- **실행 표식 = 4판 `clean_exit` 재사용**(새 표식 파일 없음) — 이미 Chromium 수법과 같다. **고친 것 하나: 부팅 첫 쓰기(`clean_exit:false`)를 팝아웃 창 생성 앞으로 당겼다**(§6-5 ③ — 4판은 맨 끝). 4판 순서면 조용히 복원한 화면이 창을 만들다 앱을 죽일 때 `clean_exit:true` 가 남아 다음 부팅이 또 조용히 복원한다.
- **`seq` 삭제**(§6-1) — 지문은 원문 바이트 비교 · 삭제 확인은 기록기 깃발 · 임시 이름은 `write_atomic` 의 pid.
- **`Flush{delete_pending}` → `Flush{drop_crash_copy}` + `DropCrashCopy` + `Final{drop_crash_copy}`**(§6-4) — 이름만 바꾼 것이 아니라 순번 비교를 깃발로 줄였다. 개념은 남긴다: 「확정 뒤에만 삭제 · 실패해도 다음 성공 쓰기가 지운다」가 D2-④ 를 지키는 수단이기 때문이다(없으면 확정 쓰기가 실패한 뒤 정상 종료하면 사본이 남아, 다음 부팅이 이미 복원한 화면을 또 묻는다).
- **`write_atomic` 에 rename 직전 확인 변형**(§6-4) — 마감 뒤 발행 금지(`closed`)가 rename 직전 확인을 요구한다.
- **기본 모양으로 죽은 세션은 사본을 만들지 않는다**(§6-5 표) — D2-① 의 글자(「한 번 복사」)보다 좁다. 4판 규칙(§14-1 #9 · B-L1 「물을 것이 없다」)을 옮겨 왔다.
- **지문이 바뀌었고 사본까지 있으면 이번 실행은 「쓰지 않음」**(§6-5 ①) — D2-⑥ 과 4판의 「지문이 다르면 덮지 않는다」를 함께 지키는 길.
- **못 쓸 파일의 해석**(§6-5 표) — 못 쓸 `state.json` 이어도 쓸 만한 사본이 있으면 묻는다 · 못 쓸 사본은 떠 둔 뒤 「사본 없음」으로 치고 나머지 행을 따른다(그래서 그 부팅의 비정상 `state.json` 이 새 사본이 될 수 있다).
- **F12 를 사본에 그대로 옮겼다**(§6-7 사본 수명) — 답 없는 정상 종료는 띠가 보였을 때만 지운다.

**4판 대기분 모델과 견준 성질 — 잃은 것 · 다시 세운 것**

- **잃음 — 답하기 전에 다시 죽은 세션의 화면**(§12 R13): 4판은 그 세션이 기본 모양이 아니면 새 화면으로 대기분을 덮었다. 5판은 앞 사본을 지키므로 둘째 세션에서 띠를 무시하고 꾸민 화면은 잃는다. D2-⑥ 은 그 세션을 「기본 · 부분 화면」으로 보았다.
- **잃음 — 겹친 부팅에서 그 실행의 저장**(§12 R11): 지문이 바뀌었고 사본이 있으면 「쓰지 않음」.
- **다시 세움 — 「디스크 확정 뒤에만 삭제」**(3판 BLOCK 의 답 · §14-2 #1): `drop_crash_copy` 깃발로 같게.
- **다시 세움 — 기본 모양 세션은 묻지 않음**(§14-1 #9): 「대기분을 덮지 않음」 쪽 역할은 D2-⑥ 이 모든 경우로 넓혀 대신하고, 「물을 것이 없다」 쪽 역할만 남겼다.
- **다시 세움 — 지문 재확인**(§14-2 #9): 순번 대신 원문 바이트.
- **다시 세움 — 크래시 루프 격리**(§6-7 끝): 4판은 「묻는다」에 기댔고 5판은 거기에 D3(크래시 뒤 자동 복원 없음)과 표식 앞당김을 더했다. 단 표식은 첫 쓰기 성공에 기댄다(§12 R14).
- **바뀌지 않음:** `restore.status` · `restore.answer` 명령 모양 · 조율자 ①–④ · 띠 · `--hidden` 보존 · 마감 하나(2초)와 `closed` · 단일 쓰는 쪽.

**건드리지 않은 것:** §6-0 저장 범위 표 · P3d 창별 테마 · P3e 보기 모드 · F19 — 대기분을 가리키던 자리(§6-0 의 §6-5 단계 번호)만 고쳤다.

### 14-10. 5판 리뷰(설계자 BLOCK · 설계자-파괴자 FIX) 반영 + 사용자 결정 D4–D8 — 6판 (2026-10-02)

> ★6판 재리뷰(설계자 PASS · 설계자-파괴자 FIX)를 거쳤다 — 반영 = §14-11★.

**원칙(사용자):** 「그런 것까지 가정하면 끝도 없음」 — 결정을 채우는 가장 단순한 설계를 고른다. 드문 디스크 · 백신 · 겹침 경계에는 **답하지 않은 크래시 사본을 잃는 경우가 아니면** 모드 · 가드를 더하지 않는다. 결정이 군더더기로 만든 장치는 지운다.

**사용자 결정**

| 결정 | 반영 자리 |
|---|---|
| **D4** — 「복원할까요?」는 main 창 웹뷰 안의 **모달**이다(띠도 OS 대화상자도 아니다). 답할 때까지 앱을 쓸 수 없다. 사람과 LLM 이 같은 `restore.answer` 로 답한다. 사유: 「무시하고 진행하면 이걸 또 저장해야 되냐 … 복잡하잖아」 | §0 · §6-7 · §8 프론트 모달 · GUI · §9-2 P3c1 · P3c2 · §10 · §11-4 · §13 |
| **D5** — 비정상 종료 뒤엔 늘 묻는다 · **기본 모양 예외 없음.** `is_default_shape` 를 크래시 처리에서 통째로 뺀다(비교 로직은 필요 없는 복잡도) | §6-1 · §6-5 표 · §8 · §10 · §11-4 · §13 |
| **D6** — 모달이 떠 있는 채 트레이 종료(또는 어떤 정상 종료)해도 **사본을 지키고** 다음 부팅이 다시 묻는다. 사본을 푸는 것은 `restore.answer` 뿐이다. F12 「띠가 보였으면 지운다」를 없앤다 | §6-4 · §6-5 · §6-6 · §6-7 · §8 · §10 · §11-4 |
| **D7** — `--hidden`(자동 시작 · 트레이만) 부팅은 **따로 다루지 않는다** — 모달은 숨은 main 안에 그려져 있어 창을 열면 거기 있다 | §6-7 · §8 GUI · §11-4 · §12 R15 |
| **D8** — 쓰기 실패(실행 표식 쓰기 실패 포함)는 **따로 다루지 않는다** — log 하고 계속, 다음 저장이 다시 쓴다. 근거 = [조사](../../research/crash-session-snapshot-precedents-2026-10-02.md) 「덧붙임」(Firefox · Chromium · VS Code · Windows Terminal 모두 같다) | §6-4 · §6-5 · §11-4 · §12 R14 |
| **F15** — 「개발 빌드도 예외 없이 항상 묻는다 — 나중에 짜증나면 요청」(2026-10-02). 대가: `/qa` GUI teardown 이 강제 종료(`scripts/launch-detached.ps1:75`)라 다음 QA 기동에 모달이 뜬다 → `.claude/skill-bindings/qa.md` §full 을 고친다(기동 뒤 `crash_copy == awaiting` 이면 `restore.answer {accept:false}` · 또는 teardown 을 `quit_app` 으로) | §10 · §9-2 P3c2 · §14-11 N2 |

**결정에서 따라 나온 것:** D4 + D6 → 부팅에 사본이 있으면 늘 답하지 않은 것 → 늘 기본 화면 + 모달. 5판 「`clean_exit:true` + 사본 → 조용히 복원」 행을 걷었다. R13(둘째 세션 화면 손실)은 거의 사라졌다 — 답 전엔 앱을 못 쓰므로 남는 것은 버스(LLM) 변경뿐이다.

**지운 것(결정이 군더더기로 만들었다):** `is_default_shape`(D5) · F12 · 띠 · `restore_prompt_shown`(D6) · `Final{drop_crash_copy}` · `DropCrashCopy` · `Flush`(D6 · I2 — 요청은 `Resolve` · `Final` 둘) · 지문 재확인과 그 갈래(I1 잠금이 대신한다) · 「쓰지 않음」 모드(D8 — 사본 뜨기 실패 하나만 남겼다, 아래) · `restore.status.pending`(I2) · 5판 R15.

**세션 판단 [고름] — 오케스트레이터 지시 I1–I8**

| # | 판단 | 근거(리뷰 번호) | 반영 자리 |
|---|---|---|---|
| I1 | 셸 실행 잠금 `shell\run\state.lock` — 프로세스 내내 쥐고 `Final` 뒤에 놓는다 · 새 인스턴스는 판정 전에 약 3초 기다리고 못 잡으면 log 후 진행한다 · 임시 파일은 자기 pid · 죽은 pid 것만 지운다. 단일 인스턴스 플러그인이 `RunEvent::Exit` 에서 뮤텍스를 우리 `Final` 보다 먼저 놓는다(`windows.rs:111-123` · `app.rs:1430-1432` · `2644-2648` — 소스 확인) | 설계자 #3 · 설계자-파괴자 F3 | §2-1 · §6-5 ①② · §6-6 · §8 잠금 · §12 R11 · §13 |
| I2 | 답의 디스크 고정 — 답 뒤 첫 `state.json` 쓰기가 `resolved_crash_copy: <사본 바이트 해시>` 를 싣고, 부팅은 해시가 같은 사본을 묻지 않고 지운다 · 기록기는 지울 때까지 뒤 쓰기마다 다시 지운다 · `restore.status` 의 `pending` → `crash_copy` = `none` · `awaiting` · `answered` · 답한 뒤 `restore.answer` = `CONFLICT` | 설계자-파괴자 F1 · F12 | §6-1 · §6-4 · §6-5 표 · §6-7 · §8 |
| I3 | 읽기 IO 실패 ≠ 못 쓸 파일 — 잠김이면 5 × 20 ms 재시도 · 그래도 실패면 log 하고 「없음」으로 친다 · 그 부팅은 떠 두기 · 지우기 · 사본 뜨기를 하지 않는다 · 덮일 위험은 §12 에 적는다 | 설계자-파괴자 F4 | §6-2 · §6-5 · §8 · §12 R16 |
| I4 | 상한 — `state.json` 읽기 4 MiB · 기록기는 넘으면 안 쓴다(`Failed` / `INTERNAL` + log) · 떠 두기 상한 ≥ 읽기 상한 → `copy_aside_at` 을 `fsutil` 로 옮기며 상한 · 로그 `module` 을 인자로 | 설계자-파괴자 F5 | §5-3 · §6-2 · §6-4 · §9-2 P3a |
| I5 | 복원 상태의 단일 출처 — 사본을 만드는 그 단계(부팅 단계 ⑥)가 서비스 상태를 정하고 `restore:changed` 를 낸다(Tauri 는 설정 창을 사용자 setup 앞에서 만든다) | 설계자-파괴자 F9 | §6-5 ⑥ · §6-7 · §8 |
| I6 | setup 순서 — main · 트리 창의 위치 · 크기 · 최대화 · 테마 입히기(⑧)는 실행 표식 뒤 · P4 의 정적 창 생성도 표식 뒤 | 설계자-파괴자 F10 | §4 · §6-5 ⑧ · §9-2 P4 |
| I7 | 단계 자르기 — P3a 는 `mod state;` 만 · 파일 겹침으로 상태 코어 + `fsutil` → `manager.rs` 곁표 → 순수 부팅 판정 → `lib.rs` 배선 · 사본 만들기와 답을 같은 단계에 · 크기 재추정 · 단계마다 혼자 초록 | 설계자 #4 · 설계자-파괴자 F11 | §9-1 · §9-2 |
| I8 | rename 직전 확인 변형은 재시도마다 다시 묻고 `Err` 아닌 「건너뜀」을 돌려준다 · 해결 칸은 기록기가 꺼낼 때 선다(보내는 쪽 원자 값 금지) + 시험 | 설계자-파괴자 7 | §6-4 · §8 기록기 · fsutil |

**세션 판단 [고름] — 이 판을 쓰며 고른 것(재리뷰가 볼 것)**

- **부팅 단계의 자리 = 단일 인스턴스 플러그인 바로 뒤에 등록한 플러그인의 setup**(§6-5) — I1 의 「판정 전에 잠금」을 지키면서, 빌드 전 대기(관문 앞이라 앱이 떠 있을 때 다시 실행하면 매번 3초)를 피한다. `LayoutState` 는 빌더에서 빈 채로 manage(ADR-0102 그대로)하고 부팅 단계가 그 안을 채운다(§12 R17).
- **실행 표식(첫 쓰기)을 부팅 단계로 옮겼다** — 어느 창보다 먼저가 되어 I6 순서가 저절로 서고, 기록기의 `Flush` 가 필요 없어졌다.
- **사본 뜨기 실패만은 그 실행의 `state.json` 쓰기를 막는다**(§6-5 ④) — 원칙의 예외(덮으면 아직 뜨지 못한 크래시 화면을 잃는다). 5판 「쓰지 않음」 모드에서 남긴 유일한 갈래다.
- **해시 = 64비트 FNV-1a 직접 구현**(§6-1) — std `DefaultHasher` 는 판마다 같은 값을 약속하지 않아 디스크에 못 싣는다. 신원 확인용이라 새 의존성을 들이지 않는다.
- **상태 떠 두기 상한 16 MiB**(§6-2) — 읽기 상한(4 MiB)보다 커야 상한 초과로 못 쓴 파일도 떠 둔다.
- **잠금 = `File::try_lock`**(§6-5 ①) — 플랫폼 중립이고, 이 파일은 아무도 읽지 않는다.
- **수락은 main · 트리 창 속성도 사본 값으로 입힌다**(§6-7 ④ — 부팅 ⑧과 같은 함수) · **조율자는 부팅 때 읽은 사본 바이트를 쓴다**(다시 읽지 않는다).
- ~~F15 를 (a) 로 닫았다 — D3 · D5 를 개발 빌드에도 그대로 읽었다~~ → 사용자 결정으로 바뀌었다(위 결정 표 · §14-11 N2).
- **모달은 main 창만 막는다**(§6-7) — 다른 창(트리)은 막지 않는다.

### 14-11. 6판 재리뷰 반영 (2026-10-02 · 설계자 PASS · 설계자-파괴자 FIX)

설계자(codex)는 PASS(플러그인 순서를 tauri-2.11.3 소스로 확인). 설계자-파괴자의 국소 지적 8건을 반영했다 — 판은 6판 그대로다.

| # | 반영 | 자리 |
|---|---|---|
| N1 | 사본은 **지우기 직전에 다시 읽어(≤4 MiB) 칸의 해시와 같을 때만** 지운다 — 기록기 `Resolve` 재시도와 부팅 ④ 둘 다. 잠금 없이 진행한 길과 2초 마감 경합에서 낡은 기록기가 새 인스턴스의 사본을 지우지 못한다 | §6-4 · §6-5 표 · §8 |
| N2 | **F15 = 사용자 결정**(「개발 빌드도 예외 없이 항상 묻는다 — 나중에 짜증나면 요청」 · 2026-10-02 — 6판의 「세션 판단」을 바로잡음). 대가: `/qa` GUI teardown 이 강제 종료라(`scripts/launch-detached.ps1:75`) 다음 QA 기동에 모달이 뜬다 → P3c2 가 qa 바인딩 §full 을 고친다 | §10 · §14-10 결정 표 · §9-2 P3c2 |
| N3 | 가드를 하나로 — `state.json` 을 못 읽었거나(IO 실패) 사본을 못 떴으면 ⑤ 건너뜀 · 기록기 없음 · log. IO 실패 뒤 덮어쓰는 위험이 사라졌다 | §6-5 ③ · §12 R16 · §8 |
| N4 | 모달이 떠 있는 동안 키 바인딩 디스패처(`src/commands/keybindings.ts:97`)도 멈춘다. 막는 것은 사람 UI 뿐 — LLM 버스는 막지 않고 관례로 먼저 `restore.answer` 를 낸다(버스 차단 장치는 만들지 않는다) | §6-7 · §8 프론트 모달 · §10 F11 · §12 R13 · §9-2 P3c2 |
| N5 | 어느 모니터에도 안 걸치는 팝아웃 위치는 버린다(4판 규칙 되살림) — 부팅 ⑨ · 조율자 ②, ⑧과 같은 함수 | §6-5 ⑧⑨ · §6-7 ② · §8 |
| N6 | 못 쓸 `state.json` 은 사본 열과 무관하게 떠 둔다 · 버전 초과 사본(하향)은 지우지 않고 남겨 둔 채 묻지 않는다 | §6-5 표 · §6-7 · §12 R3 |
| N7 | 부팅 플러그인 setup 은 `Err` 를 돌려주지 않는다 · 로그 초기화를 그 첫 단계로(상태 쪽 진단 모아 두기 삭제 · 설정의 적재 기록 되풀이는 그대로 — P2a 무수정, 걷는 길만 적었다) · 가드면 `RunEvent::Exit` 가 `Final` 을 건너뛰고 잠금만 놓는다 | §6-5 ⓪ · §6-6 · §8 |
| N8 | P3c1(~1,000)을 겹침 없는 둘로 — P3c1(사본 열 · 서비스 · 명령 · 조율자 — Rust) · P3c2(모달 · 키 바인딩 멈춤 — 프론트). 「사본은 답할 길과 함께」는 `restore.answer` 가 버스에 서는 P3c1 에서 충족된다. 옛 P3c2(슬롯 두 상태)는 P3c3 | §9-1 · §9-2 · GUI 줄 · 병렬 줄 |
| 그 밖 | R17 = 소스로 확인됨(tauri 2.11.3 `plugin.rs:880-915` · `app.rs:2440` · `2521`) — GUI 확인만 남음 · 잠금 전에 `shell\run` 을 만든다 | §12 R17 · §6-5 사실 ③ · ① |
| L1–L3 | 마무리 문구(재리뷰 PASS 뒤): ③ 「사본 있음 = 답 없음」에 너무 새 판 예외(N6) · 가드 ⅱ 면 ⑥이 `awaiting` + 메모리 `state.json` 원문을 복원 원천으로(수락 `durable:false` · 다음 부팅이 다시 묻는다 — [고름]) · R3 에 「옛 판 크래시 뒤엔 새 판이 답할 때까지 저장 없음」 | §6-5 ③⑥ · §8 · §12 R3 |
