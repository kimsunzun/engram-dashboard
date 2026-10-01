# TRD — 저장 관리 구조: 데이터 배치 · 웹뷰 폴더 · 설정 · 화면 상태 (S21)

> 상태: **초안 1판 · 리뷰 전 (2026-10-02)** · 코드 무변경.
>
> **입력:** PRD 결정 = [`docs/research/storage-management-survey-2026-10-02.md`](../../research/storage-management-survey-2026-10-02.md) 「0. 결정」(구속) · 같은 보고서 §1–§6(근거). **판독 기준** = 브랜치 `v0.3.3/feat/storage` HEAD `d4ff3f7` · Tauri 2.11.3 / tao 0.35.3 / tauri-runtime-wry 2.11.3 = 이 PC cargo 레지스트리 소스.
>
> **앵커:** ADR-0003 · ADR-0006 · ADR-0012 · ADR-0024 · ADR-0035/0057 · ADR-0051 · ADR-0054 · ADR-0060 · ADR-0102 · ADR-0134/0135/0136/0137 · ADR-0149 · ADR-0155 · ADR-0166/0167 · ADR-0169 · ADR-0171 · ADR-0230.
>
> 표기: **[고름]** = 사용자 체감이 없는 내부 구현이라 이 문서가 고른 것. **(추천 · 세션 기본값 — 사용자 확인 대기)** = 사용자 체감 갈림길의 기본값(§10). **[미검]** = 소스 읽기만 했고 실행으로 확인하지 않은 것.

---

## 0. 결론 (먼저)

| 무엇 | 어디 | 요지 |
|---|---|---|
| 경로 계산 | discovery `DataLayout`(신설) | 디렉터리와 프로세스 경계를 넘는 파일(`daemon.json`) 경로의 단일 출처. 데몬·셸·net 은 받은 경로만 쓴다 |
| 데몬 이전 | 데몬 기동, 잠금 획득 직후 | `store\` 3파일은 rename, `run\` 것은 옛 사본을 지운다(재생성물). 파일 단위 독립이라 중간에 죽어도 재기동이 이어 한다 |
| 웹뷰 폴더 | 셸 | 정적 창 둘을 Rust 에서 만들고(`create:false` + `from_config`) 모든 창에 같은 `data_directory` + 같은 브라우저 인자 |
| 설정 | 셸 `settings` 모듈(신설) | 스키마 표 한 줄 = 설정 하나. 버스 명령 넷 + 같은 서비스를 부르는 Tauri 껍데기. 파일을 아는 것은 저장 계층 하나 |
| 화면 상태 | 셸 `state` 모듈(신설) | 레이아웃 권위(`ViewManager`) 변경 번호를 보고 디바운스 저장. 부팅 전에 읽어 복원. 비정상 종료면 따로 떼어 두고 묻는다(LLM 도 답할 수 있다) |
| 순서 | §9 | P1 데이터 배치 → P2 설정 → P3 상태 → P4 웹뷰. **설정이 상태보다 먼저, 웹뷰가 맨 끝** — 사유 §9-1 |

## 1. 목표 · 범위

- **안(in):** 「0. 결정」의 데이터 배치 · 첫 부팅 이전 · 웹뷰 폴더 이전 · 설정(`settings.json` + 명령 넷 + 챗 스타일 이주 + `ui-settings.json` 폐지 + 도움말·프라이밍) · 화면 상태(`state.json` + 창 신원 + 복원 + 비정상 종료 확인) · 창별 테마의 영속 · 슬롯 종류별 설정 seam(경로 규칙만).
- **밖(out):** 프리셋 시스템 · 테마 프리셋 · 내장 테마 JSON 통일 · 작업 이력(T-14) · 개발 Vite 포트 공유 · T-22(명부 저장 실패의 성공 보고) · T-33(모르는 백엔드 종류로 명부 전체 손상). **T-22/T-33 은 이전이 건드리지 않는다** — 이전은 `agents.json` 을 파싱하지 않고 rename 만 한다.
- **밖(이번에 안 만드는 사용자 UI):** 설정 화면 · 테마 고르는 화면 메뉴(§10 F7).

## 2. 폴더 구조와 경로 계산

### 2-1. 최종 구조

```
<root>\              릴리스 = <exe 폴더>\data · 개발 = <워크트리>\.engram-data · 테스트 = ENGRAM_DATA_DIR (현행 규칙 그대로 — discovery/src/lib.rs:84)
├─ daemon\
│  ├─ store\         지킨다: agents.json · presets.json · usage_rejects.json (+ 그 .corrupt-* · .legacy-*)
│  └─ run\           버려도 된다: daemon.json(잠금+발견·WS 토큰) · mcp-config\(토큰) · usage-probe\
├─ shell\
│  ├─ store\         settings.json · state.json · state.pending.json(비정상 종료 뒤 답 전까지만) · slots\<종류>.json(필요한 종류만 — 지금 0개)
│  └─ run\           지금 없음(생길 때만 만든다)
├─ webview\          WebView2 사용자 데이터 폴더
└─ logs\             daemon-*.log · app-*.log (위치 현행 그대로)
```

### 2-2. `DataLayout` — 경로의 단일 출처 [고름]

- **자리 = `engram-dashboard-discovery` 의 새 모듈 `layout`.** 그 crate 가 이미 `default_data_dir` 의 단일 출처이고(ADR-0024 · `lib.rs:84`) 데몬·셸이 둘 다 의존한다. **base 는 기각** — 입주 조건 「도메인 지식 0」에 어긋나고(폴더 이름이 도메인 지식이다) 셋째 입주자는 ADR-0175 「거부한 대안」 재심을 부른다. **net 은 discovery 를 의존할 수 없어**(discovery → net 방향) 경로를 인자로 받는다.
- 모양: `DataLayout::new(root)` · `DataLayout::resolve()`(= `new(default_data_dir())`) · `root()` · `daemon_store_dir()` · `daemon_run_dir()` · `daemon_file()` · `mcp_config_dir()` · `usage_probe_dir()` · `shell_store_dir()` · `shell_run_dir()` · `slots_dir()` · `webview_dir()` · `logs_dir()` · `ensure_daemon_dirs()` · `legacy()`(이전 전용 — 옛 평면 경로 묶음).
- **소유 규칙:** 디렉터리와 **둘 이상의 프로세스가 같이 보는 파일**(`daemon.json` — 데몬이 쓰고 셸·스크립트가 읽는다)은 `DataLayout` 이 계산한다. **한 저장소만 쓰는 파일의 이름은 그 저장소가 소유**하고 받은 디렉터리 안에서만 붙인다 — `agents.json`(`agent/src/persistence/mod.rs:22`) · `presets.json`(`presets.rs:19`) · `usage_rejects.json`(`reject_store.rs:20`) · `settings.json`·`state.json`(셸 저장 계층). 이름까지 끌어오면 agent crate 가 discovery 를 의존해야 한다.
- 바뀌는 시그니처: net `instance::acquire(data_dir)` → `acquire(lock_file: &Path)`(지금은 안에서 `data_dir.join(DAEMON_FILE)` — `instance.rs:229`) · 데몬 `mcp_config::*_path(data_dir, …)` → `(mcp_dir, …)`(지금은 `join(MCP_CONFIG_SUBDIR)` — `mcp_config.rs:48-52,186-187`) · base `logging::init_logging_with_file(data_dir, kind)` → `(logs_dir, kind)`(지금은 안에서 `LOG_SUBDIR` — `logging/mod.rs:87,355`). discovery 공개 함수(`ensure_daemon`·`daemon_status`·`read_live_daemon`·`daemon_stop`·`send_stop`)는 **root 를 그대로 받고** 안에서 `DataLayout` 을 쓴다 — 셸 호출부(`commands/discovery.rs` · `daemon_client/mod.rs` · `tray/`)는 손대지 않는다.
- **플랫폼 중립:** OS 가름은 지금처럼 `default_data_dir` 안에만 있다. `DataLayout` 은 `Path::join` 뿐이고 구분자 리터럴이 없다. `webview_dir()` 은 모든 OS 에서 계산하고, 쓰는 쪽(§4)도 `#[cfg]` 없이 넘긴다 — 그 값을 쓰느냐는 Tauri 의 플랫폼 구현 몫이다(Tauri 는 Windows·Linux 에서만 `data_directory` 를 강제한다 — `tauri-2.11.3/src/manager/webview.rs:534-545`).

## 3. 이전(migration)

### 3-1. 데몬 — 누가, 언제

**데몬이 한다.** 명부·프리셋·거절 기록의 유일한 쓰는 쪽이고 단일 인스턴스 잠금을 쥔다. 셸은 데몬 파일을 건드리지 않는다(ADR-0134 「클라이언트는 daemon.json 을 지우지 않는다」 — `discovery/src/lib.rs:454-472` 의 원칙 연장).

`run()` 의 새 순서(지금 `daemon/src/lib.rs:461-659`):

1. `layout = DataLayout::resolve()` → 로그(`layout.logs_dir()`).
2. `ensure_data_dir_writable(root)` → `layout.ensure_daemon_dirs()`.
3. **옛 잠금 정리**(§3-2 · §10 F2): `<root>\daemon.json` 이 있으면 지운다. 공유 위반(32)으로 못 지우면 진단 읽기 — 살아 있는 레코드면 「옛 데몬이 이 폴더를 쥐고 있다」 로그 후 정상 종료(exit 0 · 현행 `AlreadyRunning` 과 같은 양보), 아니면 `FileBusy` 와 같은 갈래(exit 1).
4. `acquire(layout.daemon_file())`.
5. **store 이전** — 잠금을 쥔 뒤라 같은 폴더의 다른 새 데몬과 겹치지 않는다. 파일마다 독립: 옛 파일이 있고 새 자리가 비었으면 `rename`. 새 자리도 있으면 §10 F3(기본 = 옛 것을 `daemon\store\<이름>.legacy-<ms>` 로 보존 + warn). 옛 `<이름>.corrupt-*` 는 이름 그대로 store 로 옮기고, 옛 `<이름>.tmp` 는 지운다.
6. **run 정리** — 옛 `<root>\mcp-config\` · `<root>\usage-probe\` 를 통째로 지운다. 옮기지 않는 이유: 부팅 시점의 mcp-config 는 정의상 죽은 자격증명이고(`daemon/src/lib.rs:582-585` 스윕의 근거와 같다) usage-probe 는 기동 때 쓸리는 임시물이다(`:213-218`). 실패는 warn 하고 계속.
7. 새 자리 스윕(`layout.mcp_config_dir()` · `layout.usage_probe_dir()`) → 배선(`FileProfileStore::new(layout.daemon_store_dir())` 등 — 지금 `:226-235`).

- **멱등:** 모든 동작이 「옛 것이 있으면」 조건부라 두 번째 기동은 아무것도 안 한다.
- **중간에 죽어도:** 파일 하나의 이동 = 같은 볼륨 안 `rename` 한 번이다(`<root>` 아래라 볼륨이 갈리지 않는다). 파일 사이에 불변식이 없어(명부와 프리셋은 서로를 참조하지 않는다) 일부만 옮겨진 상태도 정상 상태이고, 다음 기동이 나머지를 옮긴다.
- **구현 seam:** 계획(`plan(listing) -> Vec<Action>`)은 순수 함수, 실행은 실제 임시 폴더로 시험한다(파일 시스템이 싸고 결정적이다).

### 3-2. 옛/새 바이너리가 섞일 때

| 조합 | 일어나는 일 | 처리 |
|---|---|---|
| 새 셸 + 옛 데몬(실행 중) | 새 경로에 `daemon.json` 없음 | **discovery 의 읽기가 새 경로 → 옛 경로 순으로 본다**(`FileReader` 하나만 고친다 — `ensure_with`·`status_with`·`stop_with`·`stop_with_sender` 가 다 그 reader 를 쓴다, `lib.rs:475,621,702,780`). 버전이 같으면 옛 데몬에 붙고 다르면 기존 `VersionMismatch`. **끄기·상태도 같은 읽기라 옛 데몬을 트레이에서 끌 수 있다** — 끈 뒤 뜨는 새 데몬이 이전한다 |
| 옛 데몬 실행 중에 새 데몬 기동 | §3-1 ③에서 옛 `daemon.json` 지우기가 32 | 살아 있으면 양보 종료. 이것이 **한 폴더에 데몬 둘**을 막는 유일한 장치다 — 없으면 새 데몬은 새 잠금을 문제없이 얻는다 |
| 옛 셸 + 새 데몬(실행 중) | 옛 셸이 루트 `daemon.json` 을 못 찾아 spawn → 새 exe 면 `AlreadyRunning` 양보 → 옛 셸 5초 시간 초과 | **지원 안 함(하향).** 알려진 한계로 적는다 |
| 이전된 폴더에서 옛 데몬 기동 | 루트에 명부 없음 → 빈 명부로 시작, 루트에 새로 씀 | **지원 안 함.** 데이터는 `daemon\store\` 에 그대로 있고, 다음 새 데몬이 F3 규칙으로 루트 것을 `.legacy-*` 로 보존한다. 개발 중 이 워크트리를 옛 커밋으로 되돌려 띄우면 여기 걸린다(§12 R3) |

### 3-3. `.gitignore` · 스크립트 · 시험

- `.gitignore` [고름]: `**/data/daemon/run/` · `**/data/daemon/store/agents.json` 추가. 옛 두 줄(`**/data/daemon.json` · `**/data/agents.json` — `.gitignore:23-24`)은 **남긴다** — 이전 안 된 옛 배포판 폴더가 추적 폴더 안에 풀릴 수 있다. `.engram-data/`(`:13`)는 그대로 전부 덮는다.
- 스크립트: `scripts/engram.mjs:45-66` 후보 경로에 새 경로를 앞에 더하고 옛 경로는 뒤에 남긴다 · `rebuild-run-debug.bat:36` · `rebuild-run-release.bat:44` · `run-release.bat` 의 포트파일 경로를 새 경로로(없으면 옛 경로) · `build-release.ps1:151-152` 주석 갱신.
- 시험: `daemon/tests/ws_e2e.rs:2390,2437,2613` · `discovery/tests/stop_smoke.rs:178` 이 루트 경로를 직접 쓴다 → 새 경로로. 그리고 **ws_e2e 에 이전 사례 하나를 더한다**(루트 평면 폴더 → 기동 → 명부 보존 + 파일 이동).
- CLAUDE.md 「백엔드 모듈 맵」 discovery 줄에 `DataLayout` 을 더한다.

### 3-4. 셸 — `ui-settings.json` 을 둘로 나눠 은퇴시킨다

지금 파일은 `{"theme":…, "windows":{label:…}}` 하나다(`layout/commands.rs:346-348`). 두 칸이 다른 집으로 가고, 각 칸의 새 집이 서는 단계에서 옮긴다.

- **P2a — 전역 칸:** 셸 부팅 때 `<root>\ui-settings.json` 의 `theme` 가 유효하고 기본값(`dark`)과 다르며 `settings.json` 에 `theme.default` 가 없으면 기록한다 → `ui-settings.json` 을 `theme` 없이 원자적으로 다시 쓴다. 파서는 `theme` 를 선택 칸으로 바꾼다(지금은 없으면 파일 전체 거부 — `ui_settings.rs` 「`theme` 키가 없다」).
- **P3d — 창별 칸:** `windows.main` · `windows["agent-tree"]` 를 그 창의 상태 항목 테마로(이미 있으면 상태가 이긴다), `slot-popup-*` 는 버린다(부팅 시점에 정의상 죽은 항목 — ADR-0167 결정 6). 그 뒤 파일을 지운다.
- 두 단계 모두 멱등(옮길 것이 남아 있을 때만 움직인다)이고, 새 파일을 원자적으로 쓴 **뒤에** 옛 파일을 고친다.

### 3-5. 챗 스타일 localStorage 일회 가져오기 (P2b · §10 F5)

- main 창 부팅 때 `localStorage['engram.chatStyle']` 가 있으면, 그 안의 **알려진 키 중 기본값과 다른 값**만 `settings.set` 으로 보내고 **전부 성공하면** 그 localStorage 키를 지운다(그래서 멱등). 하나라도 실패하면 지우지 않고 다음 부팅에 다시 한다.
- ★**옛 웹뷰 폴더 안에서만 성립한다**★ — P4 가 폴더를 옮기면 그 값은 새 폴더에서 안 보인다. 그래서 **P2b 가 P4 보다 먼저**다. 그리고 **릴리스에는 가져올 값이 원래 없다** — 릴리스에서 이 값을 쓰는 경로가 없고(`chatStyleStore.ts:6-10` · ADR-0169), dev/release 는 식별자가 달라 웹뷰 폴더가 다르다(ADR-0137). 이 가져오기가 실제로 옮기는 것은 개발 빌드 손잡이로 넣은 값뿐이다.

## 4. 웹뷰 폴더 → `<root>\webview` (P4)

- **사실(소스 대조):** `WebviewWindowBuilder::data_directory(PathBuf)` 는 절대 경로를 그대로 쓴다(`tauri-2.11.3/src/webview/webview_window.rs:1024` · `manager/webview.rs:534-545` — 지정이 없을 때만 `LocalData/<identifier>` 를 강제). 설정 키 `dataDirectory` 는 **상대 경로만** 받고 `LocalData/<label>/` 아래로 붙는다(`webview/mod.rs:392-418`). 그러니 설정만으로는 `<root>` 에 못 간다.
- **결정 [고름]:** `tauri.conf.json` 의 두 창에 `"create": false` 를 넣고(`tauri-utils-2.9.3/src/config.rs:1936` — 기본 true) 셸 `setup` 첫머리에서 `WebviewWindowBuilder::from_config(app, cfg)`(`webview_window.rs:150`)로 만든다. Tauri 가 설정 창을 만드는 자리도 사용자 `setup` 바로 앞이라(`app.rs:2521-2531`) 시점이 거의 같다. 창 선언(label·크기·`visible`)은 설정에 남으므로 `declared_window_labels`·`hidden_window_labels` 같은 설정 읽기는 그대로 돈다.
- **동일 환경 불변식(ADR-0054 확장):** 같은 사용자 데이터 폴더를 쓰는 모든 웹뷰는 같은 환경 옵션을 써야 한다(MS — 보고서 §2-6). 그래서 **공통 마무리 함수 하나**가 `data_directory(layout.webview_dir())` 와 `additional_browser_args(WEBVIEW2_BROWSER_ARGS)` 를 함께 붙이고, 정적 창 둘과 팝아웃(`popout.rs:111-117`)이 전부 그것을 지난다. 설정 파일의 `additionalBrowserArgs` 는 남기되 값의 정본은 Rust 상수다.
- **덤:** `--hidden` 부팅이면 main 을 처음부터 숨긴 채 만든다 — 지금은 보였다가 숨는다(`lib.rs:165-169` 의 자인한 깜빡임).
- **옛 폴더(`%LOCALAPPDATA%\com.engram.dashboard[.dev]`)는 지우지 않는다**(§10 F4 기본값) — 같은 식별자를 쓰는 다른 배포판 사본·다른 워크트리가 아직 쓰고 있을 수 있고, 쓰는 중이면 반쯤만 지워진다. 릴리스 노트에 「옛 폴더는 손으로 지워도 된다」를 적는다.
- **잃는 것:** 옛 폴더의 localStorage(오늘 쓰는 키는 챗 스타일 하나 — `rg localStorage src/` 실측) — §3-5 가 먼저 옮긴다.

## 5. 설정 (`shell\store\settings.json`)

### 5-1. 스키마 표 [고름]

`src-tauri/src/settings/registry.rs` 의 정적 표 한 줄 = 설정 하나:

```rust
SettingDef { key: "theme.default", kind: Kind::Choice(&["dark", "light", "e-ink"]), default: "dark",
             desc: "창별 덮어쓰기가 없는 창의 테마" },
SettingDef { key: "chat.style.fontSize", kind: Kind::CssLength { min: 8.0, max: 48.0 }, default: "13px",
             desc: "챗 기본 글자 크기" },
```

- 키 = 소문자 이름공간 + 점(`theme.*` · `chat.style.*` · 장래 `slot.<종류>.*`). 마지막 마디는 기존 철자 유지(`chat.style.fontSize`).
- 종류 = `Choice` · `CssLength`(숫자 + `px|rem|em`, 크기 상한) · `CssNumber`(단위 없는 수) · `Bool` · `Int{min,max}`. 오늘 쓰는 것은 앞 셋이다.
- **초기 키 12개:** `theme.default` + `chat.style.*` 11개(`railRowPt` · `plainRowPt` · `userPy` · `userPx` · `userMy` · `railGutter` · `railLineOffset` · `railDotTop` · `fontSize` · `lineHeight` · `waitStripH` — 기본값은 `chatStyleStore.ts:37-49` 를 그대로 옮긴다). `lineHeight` 만 `CssNumber`, `railLineOffset` 은 음수를 허용한다.
- **기본값의 정본 = 이 표.** 프론트의 `CHAT_STYLE_DEFAULTS` 는 걷는다. `theme.css` 의 `--chat-*` fallback(첫 페인트용)은 남기고, 표를 `src-tauri/bindings/settings_registry.json` 으로 내보내는 시험(ts-rs 바인딩과 같은 방식 → CI 생성물 sync 게이트가 이미 덮는 디렉터리)을 두어 vitest 가 `theme.css` fallback · `CSS_VAR_BY_KEY` 와 대조한다.

### 5-2. 저장 계층 — 파일을 아는 유일한 자리 [고름]

- `settings/store.rs`: `route(key) -> FileSlot`. 오늘 규칙 = `slot.<종류>.*` → `shell\store\slots\<종류>.json`(§7), 나머지 → `shell\store\settings.json`. 나중에 `chat.*` 를 떼어도 바뀌는 것은 이 함수뿐이다.
- 파일 모양 = **평평한 점 키 객체 + 예약 키 하나**: `{"$version":1, "theme.default":"light", "chat.style.fontSize":"15px"}`. **기본값과 다른 것만** 적는다 — `set` 이 기본값과 같은 값을 받으면 그 키를 지운다.
- **쓰기 = 읽고-고치고-쓰기(RMW), 서비스 락 하나 아래.** 쓸 때마다 파일을 다시 읽어 **그 키만** 바꾼다 — 사람이 앱 밖에서 다른 키를 고쳐 둔 것을 지우지 않는다(실행 중엔 반영 안 되고 다음 부팅에 읽힌다).
- 원자 쓰기 = 임시 파일 → `sync_all` → rename(데몬 `persistence/mod.rs:57-85` · `ui_settings.rs:687` 과 같은 방식 — 기존 `write_atomic` 을 셸 공용 모듈로 옮겨 설정·상태가 함께 쓴다). `ReplaceFileW` 는 쓰지 않는다(§12 R7).
- **항목별 관용:** 모르는 키 = 무시하되 **파일에 그대로 남긴다**(새 버전이 쓴 키를 옛 버전이 지우지 않게) · 아는 키의 못 쓸 값 = 기본값으로 접고 warn, 파일에서는 `set`/`reset` 이 덮을 때까지 그대로 · 읽는 양 상한 64 KiB(`read_capped` 재사용 — `ui_settings.rs` 「원문 크기 상한」).
- **통째로 못 읽는 파일(JSON 아님·상한 초과):** 메모리는 기본값으로 서고 **파일은 건드리지 않는다**. 첫 `set`/`reset` 이 그 파일을 `settings.json.corrupt-<ms>` 로 옮긴 뒤 새로 쓴다 — 제자리 덮어쓰기는 없고 사본이 남는다(ADR-0166 결정 9 「`.corrupt` 없음」 번복 — 이제 앱이 쓰는 파일이라 사람 손 편집본을 잃으면 되살릴 곳이 없다).

### 5-3. 명령 넷 — 기존 버스, 셸 표 [고름]

셸 표는 하나다(`layout/commands.rs` 「레이아웃 밖의 셸 명령도 여기 선다」 · 매크로 제약 「모듈 하나에 블록 하나」). 그 블록에 더하고 `catalog_version` 을 올린다(지금 9 — `:76`).

| 명령 | effect | 인자 → 답 |
|---|---|---|
| `settings.get` | Read | `{key?}` → `{rev, items:[{key, value, is_default}]}` |
| `settings.set` | Write | `{key, value}` → `{rev, key, value, changed}` · `NOT_FOUND`(모르는 키) · `INVALID_ARGUMENT`(형식·범위) · `INTERNAL`(쓰기 실패) |
| `settings.reset` | Write | `{key}` → `{rev, reset:[key…]}` |
| `settings.schema` | Read | `{key?}` → `{items:[{key, kind, default, choices, min, max, description}]}` |

- `key` 는 정확한 키 또는 점으로 끝나는 접두(`chat.style.`). `reset` 은 키가 필수다(전체 초기화 명령은 없다).
- **값은 문자열로 싣는다**(§10 F1 기본값) — 선언 매크로의 타입 알파벳에 임의 JSON 값이 없다(`command/src/macros.rs:37-47` — 원시형·`Vec`·`Option`·블록 선언 타입뿐). 서비스가 키의 종류대로 파싱·정규화하고, 파일에는 종류에 맞는 JSON 타입으로 적는다.
- **사람 경로도 같은 서비스:** `#[tauri::command] settings_get/set/reset/schema` 껍데기가 같은 함수를 부른다(레이아웃의 「두 껍데기, 서비스 하나」 — ADR-0081 결정 3). 프론트는 이 껍데기를 쓴다.
- **셸이 떠 있어야 닿는다** — 셸 표 명령이라 대시보드가 꺼져 있으면 `settings.*` 도 없다(오늘 `ui.refresh` 와 같은 성질 · §12 R9).

### 5-4. 변경 알림 · 프론트

- 쓰기가 성공하면 서비스 락을 놓은 **뒤** `settings:changed {rev, items}` 를 **모든 웹뷰**에 민다(챗은 어느 창에나 있다). `rev` 는 서비스가 쓰기마다 올리는 단조 번호다.
- 프론트 `src/api/settingsClient.ts`(신설): 구독을 먼저 걸고 그 **뒤에** `settings_get` 으로 당긴다(`theme/uiSettings.ts` 의 순서 조항 그대로) · 당긴 답의 `rev` 가 이미 받은 알림보다 낮으면 버린다.
- `chatStyleStore` 는 권위가 아니라 **적용자**가 된다: `chat.style.*` 값을 받아 CSS 변수에 붙인다(`CSS_VAR_BY_KEY` 화이트리스트는 프론트가 계속 소유). 쓰기 액션은 `settings_set` 을 부른다. 개발 손잡이 `window.__engram.chatStyle` 은 걷는다 — 쓰는 길이 버스에 생겼다(ADR-0169 「남은 갭」 해소).
- **첫 페인트:** 값은 비동기로 온다(§10 F6 기본값). `theme.css` fallback = 표의 기본값이라 깜빡임은 기본값과 다른 키에만 생긴다. `main.tsx:12` 의 동기 `loadAndApplyChatStyle()` 은 설정 클라이언트 설치로 바뀐다.

### 5-5. 테마 — 전역은 설정, 창별은 상태

- **유효 테마(창 W) = 상태의 W 항목 테마 ?? `theme.default`.** 계산은 셸이 하고, 창마다 자기 값을 받는 기존 배달(`get_ui_settings` 당기기 + `ui:settings-updated` 밀기 — `commands/settings.rs:48` · `ui_settings.rs:730`)을 그대로 쓴다 [고름 — 이름을 바꾸면 프론트·시험만 늘어난다]. `source` 칸은 파일이 사라지는 P3d 에서 뺀다.
- **화면/LLM 이 바꾼 테마가 저장된다(ADR-0167 「화면 변경 미저장」 번복):** 전역 = `settings.set theme.default <값>` · 창별 = 새 셸 명령 **`window.setTheme {window, theme}`**(`theme` = `null` 이면 덮어쓰기 해제 — 매크로의 `Option<Option<T>>` 가 그 자리다, `macros.rs:43`)와 **`window.getTheme {window}`** → `{theme, effective}`. 창별 값은 상태 파일의 그 창 항목에 들어가 **창과 같이 죽는다** — 부팅 쓸기(ADR-0167 결정 6)가 필요 없어진다.
- ★**창별 테마가 「설정 명령 넷」 밖의 다섯째 길인 이유**★: 그 값은 설정이 아니라 상태다(결정 「창별 테마 = state.json 의 그 창 항목」). 상태 파일은 기계가 통째로 다시 쓰므로 손 편집 경로가 될 수 없고, 그래서 명령이 유일한 쓰기 길이다.
- **오늘 화면에는 테마를 바꾸는 UI 가 없다**(ADR-0167 결정 5 가 `theme.set`·`theme.toggle` 을 내렸다 · `themeManager.apply` 호출부 = `main.tsx:20` · `uiSettings.ts:95,130` 뿐). 「화면에서 바꾼」 길을 새로 만들지는 §10 F7.

### 5-6. 파일 편집 경로 폐지 · `ui.refresh` · 도움말

- `ui.refresh`(`layout/commands.rs:362`)는 **P3d 에서 지운다** — 다시 읽을 파일이 없어진다. P2 동안은 창별 칸(`ui-settings.json` 의 `windows`)을 다시 읽는 일로 남는다 [고름 — 중간 머지 때 창별 테마 제어가 끊기지 않게].
- `prompts/engram-help.md`: 구획 규칙(`:8`)이 「정확히 다섯 — root · mail · agent · window · theme」를 요구하고 하나라도 빠지면 파일 전체가 거부된다. 그래서 **구획 이름 바꾸기는 파일과 CLI 를 같은 변경에서 고친다**: `theme` → `settings`(§10 F14) · `bin/engram.rs` 의 `HELP_TOPIC_THEME`(`:243`)·필수 구획 목록·`:4331` 시험 · `help theme` 는 같은 구획을 여는 별칭 · `root` 구획의 목록 줄 · `prompts/agent-priming.md:12`. 새 바이너리 + 옛 파일 = 필수 구획 누락 → 내장 사본(`:231` `include_str!`)이 나가므로 어긋남이 깨짐으로 번지지 않는다.
- 도움말 본문은 P2c(설정 명령 넷 · 창별은 아직 파일 + `ui.refresh`) → P3c(`restore.*` 를 window 구획에) → P3d(`window.setTheme`/`getTheme` · `ui.refresh` 삭제) 세 번 고친다.

## 6. 화면 상태 (`shell\store\state.json`)

### 6-1. 스키마 v1 [고름 — 필드 철자는 기존 serde 와 같게 snake_case]

```json
{ "version": 1, "saved_at_ms": 1759400000000, "clean_exit": false,
  "windows": [
    { "id": "main", "kind": "main", "theme": "light", "bounds": {"x":80,"y":60,"w":1280,"h":800,"maximized":false},
      "active_tab": "<uuid>", "tabs": [ { "id": "<uuid>", "name": "View 1", "focused_slot_id": "<uuid>|null",
                                          "layout": { "type": "split", "id": "…", "dir": "left_right", "ratio": 0.5, "a": {…}, "b": {…} } } ] },
    { "id": "agent-tree", "kind": "tree", "theme": null, "bounds": {…} },
    { "id": "slot-popup-3", "kind": "popout", "theme": null, "bounds": {…}, "active_tab": "…", "tabs": [ … ] } ] }
```

- `layout` = 기존 `LayoutNode` serde 그대로(`layout/types.rs` — `#[serde(tag="type", rename_all="snake_case")]`) · 슬롯 `content` = 기존 `SlotContent` serde(`{type:"agent", agent_id}` · `{type:"usage", show_claude, show_codex}` …).
- 넣지 않는 것: 화면 실측값(`canvas`·`metrics` — `manager.rs:103-105` · 버전을 안 올리는 측정이다), 렌더 모드 덮어쓰기(웹뷰 소유 상태 — 별건), 에이전트 런타임.

### 6-2. 메모리 타입과의 대응 · 모르는 것

- 영속 DTO 는 `state/schema.rs` 에 따로 둔다 — `WindowTabs` 는 serde 가 없고(`manager.rs:98-107`) `ViewSnapshot` 은 파생값을 싣는다. `View`·`LayoutNode`·`SlotContent` 는 serde 를 재사용한다.
- **모르는 슬롯 내용(ADR-0060 요건):** 메모리 `SlotContent` 에 `Unknown { kind: String, raw: String }` 을 더한다(§10 F10 기본값). 읽을 때 아는 변형으로 안 풀리는 `content` 는 원문 JSON 을 `raw` 에 담고, 쓸 때 `raw` 를 그대로 되돌려 써서 **새 버전이 쓴 내용을 옛 버전이 지우지 않는다**. 화면은 「알 수 없는 콘텐츠(<kind>)」 자리표시를 그리고 슬롯 명령(비우기·닫기)은 그대로 먹는다. 바인딩(`src-tauri/bindings/SlotContent.ts`)이 바뀌므로 Rust·TS 의 망라 `match`/`switch` 에 갈래를 더한다(셸 쪽 사용처 = `output_router.rs` · `daemon_client/usage_interest.rs` · `layout/*` · 프론트 = `LayoutLeaf.tsx` · `tabCommands.ts` · `selectOpenTarget.ts`).
- **모르는 창 종류 · 못 읽는 노드 · 잘린 탭:** 그 창/탭만 건너뛰고 warn(항목별 관용). **`version` 이 아는 것보다 크거나 파일 전체가 못 읽히면** 기본 화면으로 시작하고 원본을 `state.json.unreadable-<ms>` 로 옮겨 둔다 — 덮어쓰지 않는다.
- **복원 검증 = `ViewManager::from_persisted`**(신설): 유니크 소유 · `active ∈ tabs` · main 최소 1탭(`manager.rs` 불변식 1–4)을 다시 세운다 — 중복 view id 는 뒤엣것을 버리고, `active` 가 없으면 첫 탭, main 에 남은 탭이 없으면 기본 탭 하나, 탭이 없는 팝아웃은 창째 버린다.

### 6-3. 창 신원 = 지금의 label 을 그대로 영속 [고름]

- `main` · `agent-tree` 는 고정. 팝아웃은 `slot-popup-<n>`(`popout.rs:31`) 그대로 저장하고 복원 때 **같은 label 로** 다시 만든다. `PopupCounter`(`popout.rs:44`)는 복원된 팝아웃 번호의 최댓값에서 시작한다.
- 닫힌 창의 번호가 다음 실행에 재사용될 수 있지만 **무해하다** — 창에 딸린 것(탭·테마·위치)이 전부 그 창 항목 안에 있고 창을 닫으면 항목이 함께 사라진다. ADR-0167 이 경고한 오적용은 「label 을 키로 창 밖에 값을 둔」 데서 났다.
- UUID 신원을 기각한 이유: label 이 이미 Tauri 창 신원이고, 권한 glob(`capabilities/popup.json` 의 `slot-popup-*`)·Destroyed 정리 게이트(`popout.rs` `is_popup_label`)·탭 이름(`Popup 3`)이 그 모양에 기대며, LLM 은 `window.list` 의 label 로 창을 지목한다.

### 6-4. 저장 — 언제

- **기록기(`state/saver.rs`) 스레드 하나** [고름]: 0.5초마다 `ViewManager.version`(변경마다 +1 — `manager.rs:137,245`, 측정 보고는 안 올린다 — `:1921`)과 창 속성(테마·위치) 변경 번호를 짧은 락으로 읽는다. 바뀌었으면 **마지막 변경 뒤 1초 조용하거나 첫 변경 뒤 5초**가 되면 스냅숏(락 안에서 복사 → 락 밖에서 직렬화·쓰기 — ADR-0006 방식)을 원자 쓰기한다. 적용 서비스 함수 열댓 개마다 알림을 심는 대신 이미 있는 단일 변경 카운터를 본다 — 빠뜨린 경로가 생길 수 없다.
- 창 위치·크기 = `WindowEvent::Moved/Resized` 에서 창 속성으로(논리 좌표로 저장 — 배율이 다른 모니터 사이 이동에 견디게).
- **부팅 직후 한 번 `clean_exit:false` 로 쓴다** — 그 뒤 아무 변경이 없다가 죽어도 「비정상」으로 읽히게.

### 6-5. 복원 — 부팅 흐름

1. **빌드 전(`run()` 안 · `builder.manage` 앞):** `state.json` 을 읽어 `ViewManager` 를 만들고 그것으로 `LayoutState` 를 manage 한다(지금 `lib.rs:59` 의 `LayoutState::new()` 자리). **ADR-0102 그대로** — 웹뷰의 첫 invoke 전에 복원된 모델이 이미 있다. 진단 줄은 들고 있다가 로그가 선 뒤(`lib.rs:70`) 낸다.
2. 파일 없음 → 기본(`ViewManager::new()`). `clean_exit:true` → 복원. `clean_exit:false` → `state.json` 을 `state.pending.json` 으로 옮기고(이미 있으면 덮는다 — 가장 최근의 비정상 세션) 기본 화면으로 시작한 뒤 묻는다(§6-7).
3. **`setup` 안:** 복원된 팝아웃마다 창을 만든다(`build_runtime_window` 에 위치·크기·보임 인자 추가 · `--hidden` 이면 숨긴 채 — `show_main_ui` 가 이미 모든 창을 보인다, `tray/actions.rs:64-77`). 만들기에 실패한 창은 모델에서 그 창을 지운다. 위치가 어느 모니터에도 안 걸치면 위치를 버린다(순수 함수로 시험). main 의 위치·크기는 P4 전에는 만든 뒤 setter 로, P4 뒤에는 빌더에서 준다.
4. **모델에서 파생되는 표를 한 번 다시 계산한다** — 출력 라우팅 표(`output_router::rebuild` · 그 호출 계약)와 사용량 관심(`daemon_client/usage_interest.rs`). 복원은 변형 경로를 안 지나므로 이 한 번이 없으면 복원된 에이전트 슬롯이 구독을 못 받는다 [미검 — 구현 때 진입점 확인].

### 6-6. 정상 종료 표식 · Windows 종료/로그오프

- **`RunEvent::Exit` 에서 동기로** 마지막 스냅숏을 `clean_exit:true` 로 쓴다(지금 `lib.rs:267` 은 빈 핸들러). 트레이 「완전 종료」(`app.exit(0)`)도 이 사건을 낸다(`tauri-2.11.3/src/app.rs:573-574`).
- **로그오프/종료:** tao 는 `WM_ENDSESSION(wParam=TRUE)` 에서 이벤트 루프를 파기하고(`tao-0.35.3/src/platform_impl/windows/event_loop.rs:2382-2391` — `WM_QUERYENDSESSION` 은 처리하지 않는다) tauri-runtime-wry 가 그것을 `RunEvent::Exit` 로 번역한다(`tauri-runtime-wry-2.11.3/src/lib.rs:4185-4187`). 그 메시지를 받는 창은 tao 의 사건 대상 창인데 **메시지 전용 창이 아닌 최상위 창**이라(`event_loop.rs:651-686`) 창을 전부 숨긴 트레이 상태에서도 받는다. 그래서 같은 핸들러 하나가 둘 다 덮는다 [미검 — 실제 로그오프로 확인 필요 · §12 R2].
- 강제 종료(`taskkill /F` · 크래시 · 디버거 중지 · 전원)는 표식을 못 남긴다 → 다음 부팅이 묻는다. 개발 재빌드 스크립트가 앱을 죽이면 매번 묻는다(§10 F15).

### 6-7. 비정상 종료 뒤 「복원할까요?」 — 사람과 LLM 이 같은 핸들

- 셸 명령 둘(셸 표): **`restore.status`**(Read) → `{pending, saved_at_ms, windows, tabs}` · **`restore.answer {accept}`**(Write) → `{restored_windows}`, 대기분이 없으면 `CONFLICT`. Tauri 껍데기 `restore_status`/`restore_answer` 가 같은 서비스를 부르고, 상태가 바뀌면 `restore:changed` 를 main 에 민다.
- 프론트: main 창 상단 띠(`src/components/layout/RestoreBanner.tsx` 신설 · 문구는 `i18n/ko.ts`) — 「이전 화면 복원」 / 「새로 시작」.
- `accept=true` → 대기분으로 `from_persisted` → **지금 모델을 통째로 바꾼다**(지금 떠 있는 팝아웃은 닫고 복원분을 연다 — §10 F11) → 파생 표 재계산 → `window:tabs-updated` 알림. `false` → 대기분 삭제. **답 없이 정상 종료하면 대기분을 지운다**(§10 F12).
- 크래시 루프 격리: 복원한 화면이 다시 앱을 죽이면 다음 부팅도 비정상이라 또 묻는다 — 사용자가 「새로 시작」 으로 끊을 수 있다(보고서 §6 「크래시 루프 격리 출처 못 찾음」의 답을 이 흐름이 대신한다).

### 6-8. 대상 에이전트가 없는 슬롯

- 복원은 배치를 그대로 두고 `agent_id` 도 그대로 둔다 — 그 에이전트가 다시 뜨면(트리에서 활성화) 슬롯이 알아서 붙는다. 이것이 「대기」다.
- 화면 판정은 기존 ADR-0149 세 상태 그대로다(`LayoutLeaf.tsx:194-260`): 프로필도 없으면 「연결된 에이전트가 없습니다」(= 결정의 「대상 없음」) · 목록 미수신이면 「에이전트 연결 중…」. **바꾸지 않는다** — 복원 직후 웹뷰는 기억이 없어(A) 갈래로 들어가고, 목록이 오면 (C)로 정해진다. 프로필은 있는데 에이전트가 안 떠 있는 슬롯은 계속 「연결 중…」으로 보인다(§10 F16).

## 7. 슬롯 종류별 공통 설정 (`shell\store\slots\<종류>.json`)

- **seam 만 깐다:** 설정 키 `slot.<종류>.<이름>` → 저장 계층 `route()` 가 `slots\<종류>.json` 으로 보낸다(§5-2). 명령·스키마·알림은 일반 설정과 같은 길이다.
- **지금 구현되는 것 = 그 경로 규칙과 그 시험뿐.** 등록된 키가 0개라 파일은 생기지 않는다. 슬롯에 꽂힌 내용(`Usage` 의 `show_claude` 등)은 상태(`state.json`)다 — 종류별 「공통」 설정만 여기 온다.

## 8. seam · 시험 (ADR-0012)

| 모듈 | 끊는 것 | 하네스 · 새 시험 |
|---|---|---|
| `DataLayout` | 없음(순수) | root → 각 경로 단언 · 옛 경로 묶음 단언(`cargo test -p engram-dashboard-discovery`) |
| 데몬 이전 | 파일 시스템 = 임시 폴더 · 계획 = 순수 함수 | 계획 표 시험 · **동작 앞부분만 실행 → 재실행 = 같은 결과**(중간 죽음) · 충돌 보존 · `.corrupt-*`/`.tmp` 처리 · 옛 잠금 판정(`#[cfg(windows)]` — 쥔 핸들로 지우기 32 재현, net 의 기존 방식) · ws_e2e 평면 폴더 기동 |
| discovery 대체 읽기 | `DaemonReader` 트레이트(기존) | 새 경로 우선 · 새 경로 없음 → 옛 경로 · 둘 다 없음 |
| 설정 표 · 저장 · 서비스 | 파일 = `SettingsFiles` 트레이트(메모리 구현) · 알림 = 포트 | 종류별 검증 · 기본값과 같으면 키 삭제 · 모르는 키 보존 · 못 읽는 파일 무손상 + 첫 쓰기 때 `.corrupt` · RMW 가 남의 키 보존 · `rev` 단조 · 명령 표(`src-tauri/tests/layout_commands.rs` 에 가짜 포트) · `settings_registry.json` 내보내기 |
| 프론트 설정 | `invoke`/`listen` 모의 | 구독 먼저 → 당기기 · 낡은 `rev` 버림 · 챗 스타일 적용 · 일회 가져오기(성공 시만 지움) · `theme.css` fallback 대조 |
| 상태 코덱 | 없음(순수) | 왕복 · `Unknown` 원문 보존 · 버전 초과 · 항목별 건너뛰기 · `from_persisted` 불변식(중복 view · 빠진 active · main 0탭 · 빈 팝아웃) |
| 기록기 | 시계 · 스냅숏 원천 · 쓰기 = 포트 | 디바운스 1초 · 상한 5초 · 무변경 무쓰기 · 종료 플러시 `clean_exit:true` · 부팅 직후 `false` |
| 부팅 판정 · 대기분 | 파일 = 포트 | 없음/깨끗/비정상 → 동작 표 · `answer` 수락/거절/대기분 없음 · 정상 종료 시 대기분 삭제 |
| 창 위치 | 모니터 목록 = 인자 | 화면 밖 위치 버림 · 최대화 |
| Tauri 결합부 | — | GUI 실측만(`/qa full`): 정적 창 생성 · 웹뷰 폴더 · 팝아웃 복원 · 로그오프 |

수치·명령은 CLAUDE.md 「빌드·검증 명령」 그대로 — 셸 단위 시험은 `cargo test -p engram-dashboard --test lib_unit`, 데몬·discovery 는 `-- --test-threads=4`.

## 9. 단계 계획 — 매 단계 끝에 빌드·시험 초록

### 9-1. 순서의 이유

- **설정(P2)이 상태(P3)보다 먼저:** 상태 영속이 들어오는 순간 ADR-0167 의 부팅 쓸기 전제(「부팅 때 팝아웃은 0개」)가 무너진다. 설정을 먼저 하면 P2 동안 창별 칸은 옛 파일 + 쓸기로 **여전히 옳게** 돌고, 그 칸은 상태가 선 뒤(P3d) 한 번에 옮긴다.
- **웹뷰(P4)가 맨 끝:** 챗 스타일 가져오기(P2b)가 옛 웹뷰 폴더 안에서만 된다(§3-5).
- **각 단계 안에서도 중간에 멈춰도 빌드가 서게** — 새 모듈·새 타입을 쓰는 쪽 없이 먼저 넣고(경고만), 호출부를 한 단위씩 옮긴다. 자료구조를 먼저 갈아엎고 호출부를 나중에 맞추는 순서는 금지다.

### 9-2. 단계

| 단계 | 내용 | 파일(겹침 기준) | 크기 |
|---|---|---|---|
| **P1** 데이터 배치 + 데몬 이전 | §2 · §3-1~3-3 | discovery `src/lib.rs` · `src/layout.rs`(신설) · `tests/stop_smoke.rs` · net `src/instance.rs`(+시험) · base `src/logging/mod.rs` · 데몬 `src/lib.rs` · `src/data_migration.rs`(신설) · `src/control/{mod.rs,mcp_config.rs}` · `src/bin/priming_smoke.rs` · `tests/ws_e2e.rs` · `src-tauri/src/lib.rs`(로그 인자 한 줄) · `.gitignore` · `scripts/{engram.mjs,rebuild-run-debug.bat,rebuild-run-release.bat,run-release.bat,build-release.ps1}` · CLAUDE.md 모듈 맵 | M–L (~800줄) |
| **P2a** 설정 코어 + 전역 테마 | §5-1~5-3 · §5-5 전역 · §3-4 전역 · 셸 공용 원자 쓰기 | `src-tauri/src/settings/{mod,registry,store,migrate}.rs`(신설) · `src-tauri/src/fsutil.rs`(신설 — `write_atomic`·`read_capped` 이사) · `ui_settings.rs` · `commands/settings.rs` · `commands/layout.rs`(`command_ports`) · `layout/commands.rs` · `lib.rs` · `src-tauri/tests/layout_commands.rs` · `src-tauri/bindings/*` | L (~1,000) |
| **P2b** 프론트 챗 스타일 | §5-4 · §3-5 | `src/api/settingsClient.ts`(신설) · `src/store/chatStyleStore.ts` · `src/main.tsx` · 해당 `*.test.ts` | S–M (~400) |
| **P2c** 도움말 · 프라이밍 | §5-6 1차 | `prompts/engram-help.md` · `prompts/agent-priming.md` · 데몬 `src/bin/engram.rs` · CLAUDE.md 「LLM-우선 제어」 갭 줄(챗 스타일) | S (~200) |
| **P3a** 상태 기록 | §6-1 · §6-2 · §6-4 · §6-6 | `src-tauri/src/state/{mod,schema,saver,attrs}.rs`(신설) · `layout/types.rs`(`Unknown`) · `layout/manager.rs`(스냅숏 내보내기) · `Unknown` 갈래가 필요한 곳(`output_router.rs` · `daemon_client/usage_interest.rs` · `layout/{apply,commands,tree,spatial,geometry}.rs`) · `lib.rs`(기록기 · `RunEvent::Exit` · Moved/Resized) · `src/components/layout/LayoutLeaf.tsx` · `src/commands/tabCommands.ts` · `src/components/agent/selectOpenTarget.ts` · `src/i18n/ko.ts` · 바인딩 | L (~1,100) |
| **P3b** 부팅 복원 | §6-3 · §6-5 (비정상이면 대기분으로 떼고 묻지는 않는다) | `state/restore.rs`(신설) · `layout/manager.rs`(`from_persisted`) · `layout/mod.rs` · `commands/popout.rs` · `lib.rs` · CLAUDE.md 「레이아웃은 디스크 영속이 없다」 줄 | M–L (~800) |
| **P3c** 비정상 종료 확인 | §6-7 | `state/pending.rs`(신설) · `layout/commands.rs` · `commands/state.rs`(신설) · `commands/mod.rs` · `lib.rs` · `src/components/layout/RestoreBanner.tsx`(신설) · `src/App.tsx` · `src/i18n/ko.ts` · `prompts/engram-help.md`(window 구획) · `src-tauri/tests/layout_commands.rs` | M (~600) |
| **P3d** 창별 테마 이주 + `ui-settings.json` 은퇴 | §5-5 창별 · §3-4 창별 · §5-6 | `state/attrs.rs` · `state/migrate.rs`(신설) · `layout/commands.rs`(`window.setTheme`/`getTheme` · `ui.refresh` 삭제) · `commands/settings.rs` · `ui_settings.rs`(삭제) · `lib.rs`(쓸기 호출 삭제) · `src/theme/uiSettings.ts`(+시험) · `src-tauri/tests/layout_commands.rs` · `prompts/engram-help.md` | M (~600, 절반이 삭제) |
| **P4** 웹뷰 폴더 | §4 | `src-tauri/tauri.conf.json` · `lib.rs` · `commands/popout.rs` · 공통 마무리 함수(`commands/popout.rs` 또는 새 `webview_env.rs`) | S–M (~300) |

- **병렬 가능:** P2b ∥ P2c(파일 겹침 0). 나머지는 순차 — `lib.rs` · `layout/commands.rs` · `prompts/engram-help.md` 를 여럿이 공유한다.
- 단계마다 `/review code` → `/qa`(GUI 가 걸린 단계는 full) → 커밋. 각 단계 착수 전 되돌릴 지점 = 직전 단계 커밋.
- 단계별 GUI 확인 핵심: P1 실제 `.engram-data` 사본으로 기동 → 명부 보존 · P2a `settings.set theme.default light` → 모든 창 전환 · 재시작 유지 · P2b `chat.style.fontSize` 즉시 반영 · P3a 레이아웃 바꾸고 1초 뒤 파일 · 트레이 종료 → `clean_exit:true` · P3b 탭·분할·팝아웃·위치 복원 · P3c `taskkill /F` 후 띠 · `engram restore.answer` · P3d 창별 테마 재시작 유지 · P4 `data\webview\` 생성 · 팝아웃 유령 창 없음.

## 10. 구현 갈림길

| # | 갈림길 | 선택지 | 기본값 |
|---|---|---|---|
| F1 | `settings.set` 값 싣는 법(LLM 이 보는 모양) | (a) 문자열 하나, 키 종류대로 파싱 · (b) 매크로 알파벳에 임의 JSON 추가(command crate · ADR-0155 영향) · (c) 종류별 선택 칸 | **(a)** (추천 · 세션 기본값 — 사용자 확인 대기) |
| F2 | 새 데몬이 옛 루트 `daemon.json` 을 다루는 법 | (a) 지워 보고 못 지우면(=쥔 데몬 있음) 양보 · (b) 평생 함께 쥐어 하향 데몬을 막는다(루트에 0바이트 파일이 영구히 남는다) | **(a)** (추천 · 세션 기본값 — 사용자 확인 대기) |
| F3 | 옛 위치와 새 위치에 같은 저장 파일이 둘 다 있을 때 | (a) 새 것 유지 · 옛 것을 `.legacy-<ms>` 로 보존 · (b) 수정 시각이 새 것을 채택 · (c) 기동 거부 | **(a)** (추천 · 세션 기본값 — 사용자 확인 대기) |
| F4 | 옛 웹뷰 폴더 `%LOCALAPPDATA%\<식별자>` | (a) 그대로 두고 릴리스 노트에 적는다 · (b) 릴리스만 한 번 지운다 · (c) 둘 다 지운다 | **(a)** (추천 · 세션 기본값 — 사용자 확인 대기) |
| F5 | 챗 스타일 localStorage 가져오기 | (a) 일회 가져오기 구현(개발 빌드에서만 실효) · (b) 생략 | **(a)** (추천 · 세션 기본값 — 사용자 확인 대기) |
| F6 | 설정값 첫 페인트 | (a) 비동기 당기기 + CSS fallback = 기본값 · (b) 창 생성 때 초기값 주입(새 전역 하나) | **(a)** (추천 · 세션 기본값 — 사용자 확인 대기) |
| F7 | 화면의 테마 바꾸기 UI | (a) 이번엔 없음(명령만) · (b) 메뉴/팔레트 항목 추가 | **(a)** (추천 · 세션 기본값 — 사용자 확인 대기) |
| F8 | 창 위치·크기 복원 | (a) 저장·복원 + 화면 밖이면 버림 · (b) 배치만(팝아웃은 계단식) | **(a)** (추천 · 세션 기본값 — 사용자 확인 대기) |
| F9 | 트리 창(`agent-tree`) | (a) 테마·위치만 · 보임은 복원 안 함(부팅 땐 숨김) · (b) 보임도 복원 | **(a)** (추천 · 세션 기본값 — 사용자 확인 대기) |
| F10 | 모르는 슬롯 내용 | (a) `Unknown` 변형 + 원문 보존 + 자리표시 · (b) 빈 슬롯으로(손실) | **(a)** (추천 · 세션 기본값 — 사용자 확인 대기) |
| F11 | 「복원」을 누르면 | (a) 지금 화면을 통째로 바꾼다 · (b) 복원분을 탭으로 덧붙인다 | **(a)** (추천 · 세션 기본값 — 사용자 확인 대기) |
| F12 | 답하지 않은 대기분 | (a) 다음 정상 종료 때 버림(Chrome) · (b) 답할 때까지 보존 | **(a)** (추천 · 세션 기본값 — 사용자 확인 대기) |
| F13 | `--hidden` 부팅 때 팝아웃 | (a) 숨긴 채 만들고 main 과 함께 보인다 · (b) main 을 보일 때 만든다 | **(a)** (추천 · 세션 기본값 — 사용자 확인 대기) |
| F14 | 도움말 구획 | (a) `theme` → `settings` 개명 + `help theme` 별칭 · (b) id `theme` 유지, 본문만 교체 | **(a)** (추천 · 세션 기본값 — 사용자 확인 대기) |
| F15 | 강제 종료(개발 재빌드 스크립트 포함) 뒤 매번 묻기 | (a) 그대로 묻는다 · (b) 개발 빌드는 묻지 않고 복원 · (c) 환경변수로 끈다 | **(a)** (추천 · 세션 기본값 — 사용자 확인 대기) |
| F16 | 복원된 슬롯의 에이전트가 프로필만 있고 안 떠 있을 때 | (a) 기존 ADR-0149 문구(「연결 중…」) 그대로 · (b) 「정지됨」 새 상태 | **(a)** (추천 · 세션 기본값 — 사용자 확인 대기) |

## 11. ADR 후보 (`/adr` 가 채번 — 다음 = 0264 부근)

1. **데이터 폴더를 컴포넌트·종류(store/run)로 가르고 첫 부팅에 옮긴다 · 경로 단일 출처 `DataLayout`.** 거부: **설치형(Program Files + AppData)** — 배포가 ZIP 한 덩이·한 사용자이고, 설치형의 이유(여러 사용자·쓰기 보호 폴더·자동 갱신)가 지금은 해당되지 않으며, 옮기면 워크트리 격리를 환경변수로 따로 지켜야 한다(보고서 §2-4 · §2-5) · **평면 유지** — 지킬 것과 버릴 것·토큰 파일이 한 폴더에 섞여 지워도 되는 것을 가를 수 없다 · **종류 먼저(`store\daemon\`)** — 사용자 결정은 컴포넌트 먼저 · **Tauri 2.12 `appDirectoriesOverride`** — 2.11.3 에 없어 업그레이드가 선행된다(보고서 §2-6) · **복사 후 옛 것 유지** — 하향 바이너리가 낡은 사본으로 지운 에이전트를 되살린다 · **경로 정본을 base 에** — 도메인 지식이고 ADR-0175 재심을 부른다.
2. **웹뷰 데이터 폴더 = `<root>\webview` · 정적 창을 Rust 에서 만든다 · 모든 창이 같은 폴더·같은 환경 옵션(ADR-0054 확장).** 거부: 설정 키 `dataDirectory`(상대 경로만 · `LocalData/<label>` 아래) · `appDirectoriesOverride`(업그레이드 선행) · `%LOCALAPPDATA%` 유지(ADR-0134 포터블 원칙 위반 · 워크트리 간 localStorage 공유) · 옛 폴더 자동 삭제(F4). ADR-0137 「식별자가 웹뷰 폴더도 정한다」는 대가 서술이 낡는다.
3. **설정 = 셸 `settings.json` + 범용 명령 넷 + 스키마 한 줄 등록 · 쓰는 쪽은 명령 하나뿐.** ADR-0166 결정 1·3·9 와 ADR-0167 결정 3·6·7 번복(폐기 도장은 옛 ADR 본문에), ADR-0051 「권위 = 프론트」 최종 번복(0169 가 부분 개정), ADR-0169 의 「남은 갭」 해소. 거부: **값마다 명령**(ADR-0169 가 이미 거부 — 유지) · **파일 직접 편집 + `ui.refresh`**(밖의 편집자와 앱이 한 파일을 다툰다 — ADR-0167 이 남긴 갈림길 ①/② 중 ② 「쓰기를 한 곳으로」를 고른다) · **localStorage**(셸·LLM 이 못 읽고 웹뷰 폴더에 묶인다) · **데몬 소유**(표시 설정은 셸 몫, 데몬은 에이전트를 띄우는 정의만 — 결정) · **설정과 상태를 한 파일에**(목적별 분리 — 보고서 §2-1 · orca 가 보조 파일을 떼야 했다) · **매크로에 임의 JSON**(F1).
4. **화면 상태 = 셸이 쓰는 `state.json` · 창 신원 = 영속하는 label · 항상 복원, 비정상 종료 뒤에만 묻는다 · 창별 테마는 창 항목에.** ADR-0060 「영속 도입 시 요건」 이행, ADR-0167 결정 6·7(부팅 쓸기) 폐기, ADR-0166 결정 3 「화면 토글 미저장」 번복. 거부: **레이아웃을 프론트 localStorage 에**(LLM-우선 제어 충돌 · 출처·웹뷰 폴더에 묶임 — 웹 우선 피어만의 길) · **`tauri-plugin-window-state`**(창 기하만 · 2.11.3 에선 `app_config_dir` 고정) · **한 파일에 전부** · **지금 SQLite**(상태가 작다 — 저장 엔진은 T-14) · **창별 테마를 설정 파일에**(창 수명과 떨어져 쓸기가 다시 필요하다) · **이름 `session.json`**(에이전트 세션과 충돌) · **기본 꺼짐 + 설정으로 켬**(Windows Terminal 관행 — 사용자가 Chrome 방식을 골랐다) · **종료 때만 저장**(크래시·강제 종료에 진다 — 보고서 §2-7) · **UUID 창 신원**(§6-3).

## 12. 위험 · 미확인

| # | 무엇 | 상태 |
|---|---|---|
| R1 | **웹뷰 폴더를 옮긴 뒤의 WebView2 실동작** — 첫 기동 시간 · 한 창이라도 폴더/인자가 어긋나면 유령 창(ADR-0054 의 그 실패가 `data_directory` 축에서도 나는지) · 폴더가 네트워크 공유·OneDrive 위일 때(ADR-0134 결정 3 이 지원한다는 배치) · 데이터 폴더 크기 증가(캐시가 포터블 폴더로 들어온다 — 압축 복사·`build-release.ps1` clean 에 영향) | [미검] — P4 GUI 실측 |
| R2 | **로그오프·종료 때 `RunEvent::Exit` 가 실제로 오고 쓰기가 끝나는가** — tao 경로는 소스로만 확인(§6-6). Windows 가 기다려 주는 시간 안에 끝나는지, 숨긴 창만 있을 때도 오는지 | [미검] — 수동 로그오프 시험 |
| R3 | **옛/새 바이너리 혼용**(§3-2) — 이 워크트리를 옛 커밋으로 되돌려 띄우면 빈 명부로 보이고(데이터는 `daemon\store\` 에 있다) 옛 셸은 새 데몬에 못 붙는다 | 지원 안 함 — 문서화 |
| R4 | 강제 종료 시 마지막 1~5초의 화면 변경 유실 · 개발 재빌드마다 복원 질문(F15) | 설계상 수용 |
| R5 | **복원 뒤 파생 표 재계산의 진입점** — 라우팅·사용량 관심을 한 번 다시 계산할 함수가 무엇인지 · DPI 가 다른 모니터 사이의 위치 · 팝아웃 생성 실패 처리 | [미검] — P3b |
| R6 | `SlotContent::Unknown` 이 바인딩과 모든 망라 분기에 퍼진다(§6-2 목록은 `rg` 결과이고 전수 확인 아님) | P3a 컴파일러가 잡는다 |
| R7 | Windows 원자 쓰기 — `rename` 교체를 쓰고 `ReplaceFileW` 는 안 쓴다(보고서 §2-7 지적). 못 읽는 파일은 덮어쓰지 않으므로 반쪽 파일이 기본값 덮어쓰기로 번지지는 않는다 | 수용 |
| R8 | 데몬 이전 중 `rename` 이 제3자(백신·인덱서) 핸들로 실패 | 실패한 파일만 옛 자리에 남고 다음 기동이 다시 한다 · 그 기동은 그 파일이 빈 상태로 돈다 [미검] |
| R9 | 설정·창별 테마·복원 답이 **셸이 떠 있을 때만** 명령으로 닿는다 — 대시보드가 꺼져 있으면 LLM 이 바꿀 길이 없다 | 「LLM-우선 제어」 갭으로 기록 |
| R10 | 단일 인스턴스 뮤텍스는 식별자 단위라(`lib.rs` 플러그인 주석) 개발 워크트리 둘의 셸은 지금도 동시에 못 뜬다 — 웹뷰 폴더 분리는 「번갈아 쓸 때의 공유」만 없앤다 | 사실 기록 |

## 13. 「0. 결정」과 대조 — 다듬은 것 · 막힌 것

- **불가능한 항목은 없다.** 다듬은 것만 있다.
- 「기존 파일은 첫 부팅 때 한 번 옮긴다」 → **store 3파일만 옮기고, run 것(`daemon.json` · `mcp-config\` · `usage-probe\`)은 옛 사본을 지운다** — 셋 다 기동마다 새로 만들거나 쓸어내는 것이라 옮길 내용이 없다(§3-1). 그중 `daemon.json` 은 잠금이라 **옛 데몬이 살아 있으면 지울 수 없고, 그 실패가 곧 중복 데몬 방지 장치**다.
- 「접근은 범용 명령 넷뿐」 → 설정에 대해서는 그대로. **창별 테마는 상태라 다섯째 길(`window.setTheme`)이 필요하다**(§5-5) — 상태 파일은 기계가 통째로 다시 쓰므로 손 편집 경로가 될 수 없다.
- 「화면에서 바꾼 테마도 저장」 → **오늘 화면에 테마 UI 가 없다**(ADR-0167 결정 5). 이번 변경 뒤 저장되는 것은 명령으로 바꾼 테마다. 화면 UI 는 F7.
- 「챗 스타일을 localStorage 에서 여기로」 → 권위 이전은 그대로. **값 가져오기는 옛 웹뷰 폴더 안에서만 되고**, 릴리스에는 가져올 값이 원래 없다(§3-5).
- 「모르는 종류 허용(ADR-0060)」 → 슬롯 내용에 `Unknown` 변형이 필요하고 그것이 바인딩·프론트까지 퍼진다(F10).
- 「Windows 종료/로그오프 알림을 정상 종료로」 → 별도 훅 없이 `RunEvent::Exit` 하나로 된다(tao 가 `WM_ENDSESSION` 을 번역) — 단 실행 확인 전(R2).
- 「`shell\{store,run}` — run 은 생길 때만」 → 이번 변경에 `shell\run\` 에 들어갈 것이 없다.
