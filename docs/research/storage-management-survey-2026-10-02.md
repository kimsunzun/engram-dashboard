# 저장 관리 구조 서베이 — 화면 상태 복원 · 사용자 설정 · 백엔드 저장 (2026-10-02)

- **상태:** 조사 완료 · PRD 수준 결정 완료(2026-10-02 사용자 결정 — 아래 「0. 결정」) · TRD = `docs/process/S21-storage/trd.md`

## 0. 결정 (2026-10-02 사용자 결정 · 대화)

- **데이터 루트 = 포터블 유지**(릴리스 `<exe>\data\` · 개발 `<워크트리>\.engram-dev\`). 설치형(Program Files + AppData)은 성숙 후·보안 필요 시 재론.
- **개발 데이터 루트 이름 = `.engram-dev`**(옛 `.engram-data` — 2026-10-02 사용자 결정 · ADR-0136 결정 2 번복 · ADR-0264). 관행 = 릴리스 이름 + `-dev`(herdr `herdr-dev` · orca `orca-dev`). `engram` 은 저장소 루트·검색·ignore 에서 겹치지 않게 남기고, 개발 식별자 `com.engram.dashboard.dev` 와도 맞는다. 이름을 바꾸지 않는다 — `.engram-dev` 는 새로 시작한다(아래 「옛 데이터」).
- **폴더 구조 = 컴포넌트 먼저, 종류 다음:** `daemon\{state,run}\` · `shell\{config,state,run}\`(run 은 생길 때만) · `webview\` · `logs\`. 규칙 = `config\` · `state\` 는 지키고 `run\` 은 버려도 된다. 토큰 든 파일(`daemon.json` · `mcp-config\`)은 `daemon\run\`.
  - **폴더 이름 = 업계 표준**(2026-10-02 사용자 결정) — systemd `ConfigurationDirectory=` · `StateDirectory=` · `RuntimeDirectory=` · XDG `CONFIG_HOME` · `STATE_HOME` · `RUNTIME_DIR`(https://specifications.freedesktop.org/basedir/latest/ · systemd.exec man page). 가름 기준: **config = 취향**(「어떻게」 — 지우면 기본값으로 돌아간다) · **state = 프로그램이 기억하는 것**(「무엇이 있나 · 어땠나」 — 에이전트 명부는 사용자가 만들었어도 여기다. Docker 의 `/var/lib` 와 같다). 데몬은 아직 config 가 없다(장래 조정값 → `daemon\config\`). 옛 이름 `store` 는 표준 용어가 아니라 버렸다.
  - `daemon\state\` = `agents.json` · `presets.json` · `usage_rejects.json` · `daemon\run\` = `daemon.json` · `mcp-config\` · `usage-probe\`. ~~기존 파일은 첫 부팅 때 한 번 옮긴다.~~ → 아래 「옛 데이터」로 뒤집혔다.
- **옛 데이터는 옮기지 않고 버린다**(2026-10-02 사용자 결정 — 「아직 데이터 정립 안 됐으니」 · ADR-0264 결정 5 · 6). 새 코드는 옛 평면 파일(릴리스 `<exe>\data\*`)과 개발 `.engram-data` 를 읽지도 옮기지도 지우지도 않는다. 첫 부팅 이전 · 완료 표지 · 옛 자리 잠금은 없고, 잠금은 `daemon\run\daemon.json` 하나다. 옛/새 바이너리는 파일을 공유하지 않아 간섭하지 않는다(옛 데이터가 새 바이너리에 안 보이는 것은 의도).
- **웹뷰 데이터를 `webview\` 로**(지금 `%LOCALAPPDATA%\<identifier>` — 포터블 원칙 위반 해소 · 워크트리 간 localStorage 공유 해소).
- **설정 = `shell\config\settings.json`** — 사람·LLM 이 정하는 값, 기본값에서 바꾼 것만. 키는 이름공간(`theme.default` · `chat.style.*` …). 접근은 범용 명령 넷(`settings.get` · `set` · `reset` · `schema`)뿐 — 스키마에 한 줄 등록으로 설정 추가. LLM 이 `ui-settings.json` 을 직접 고치던 경로를 폐지하고 `prompts/engram-help.md` 의 theme 구획·프라이밍을 함께 고친다. 저장 계층 한 곳만 파일을 안다(나중에 `chat.*` 등을 별도 파일로 떼도 호출부 무수정). 챗 스타일을 localStorage 에서 여기로.
- **화면 상태 = `shell\state\state.json`**(이름 = Windows Terminal 관행 · `session` 은 에이전트 세션과 충돌해 기피) — 창 → 탭 → 분할 트리 → 슬롯 → `content{kind, agentId / 옵션}`. 창마다 재시작해도 유지되는 id. 버전 봉투 + 모르는 종류 허용(ADR-0060).
- **복원 정책 = 항상 복원, 비정상 종료 뒤에만 묻는다**(Chrome 방식) — `cleanExit` 표식 · Windows 종료/로그오프 알림을 정상 종료로 처리.
- **화면에서 바꾼 테마도 저장**(ADR-0167 의 「화면 변경 미저장」 번복) — 전체 기본 = `settings.json` · **창별 테마 = `state.json` 의 그 창 항목**(창과 수명이 같아 부팅 쓸기 불필요).
- **테마는 이름으로 참조** — 내장 셋(dark · light · e-ink)은 CSS 유지. 사용자 테마는 나중에 `shell\config\themes\<이름>.json`(`extends` + 바꾼 색). 내장도 JSON 으로 통일할지는 테마 프리셋 착수 때(이번 범위 밖).
- **슬롯:** 꽂힌 내용 = `state.json` · 슬롯 종류별 공통 설정 = `shell\config\slots\<종류>.json`(필요한 종류만). 대상 에이전트가 없는 슬롯은 배치를 유지하고 「대상 없음」 대기 상태로 복원.
- **프리셋 시스템은 이번 범위 아님** — 에이전트를 띄울 때 들어가는 정의는 데몬 쪽이라는 원칙만. 기존 `presets.json` 은 자리만 `daemon\state\` 로 정하고(옛 파일은 옮기지 않는다) 새 프리셋 구조는 만들지 않는다(2026-10-02).
- **로그 = 최상위 `logs\` 유지**(2026-10-02) — 프로세스마다 파일이 따로다. 데몬/클라이언트가 다른 기계로 갈라지면 각 기계의 `logs\` 에 제 종류만 남는다.
- **범위 밖(별건):** 개발 Vite 포트(1420 고정 · 재사용)를 워크트리끼리 공유하는 문제 · 작업 내용 이력(T-14).
- **TRD 4판의 뒤따른 결정(2026-10-02 사용자 결정 — 상세 = TRD §6-0 · §14-6 · §14-7):**
  - **저장 범위** — 남김: 팝아웃 창 · 탭(순서 · 활성 · 이름) · 분할 트리(구조 · 방향 · 비율) · 슬롯 내용 · 포커스 · 창 위치 · 크기 · 최대화 · 사용량 슬롯 체크 · 테마(전역 = 설정 · 창별 = 상태) · 챗 스타일(설정). 안 남김: 도구 묶음 · 생각 줄 · 대기 목록 펼침 · 스크롤 고정 · 목록 선택 · 입력 초안 · 트레이 숨김.
  - **슬롯 보기 모드(터미널/챗)도 남긴다** — 프론트 메모리뿐이던 렌더러 덮어쓰기를 셸 소유 슬롯 값으로 옮긴다(에이전트 출력 형식과는 별개 — ADR-0078).
  - **옛 값은 다루지 않는다(U1)** — 배포본(v0.1.0–v0.3.2)은 있으나 사용자가 본인뿐이라 지킬 사용자 데이터가 없다(사용자 판단 2026-10-02). `ui-settings.json` 은 옮기지 않고 셸이 읽기를 그만둘 뿐이다. 챗 스타일 localStorage 값도 가져오지 않는다. `settings.json` 은 기본값에서 시작한다.
  - **챗 스타일은 지금 11키만 가볍게(U2)** — `chat.style.*` 는 임시 이름공간이다(챗 영역 재작성 · 플러그인 배치로 갈려 나갈 예정).
  - **비정상 종료 스냅숏은 덮어쓰지 않고 최근 3 개를 돌려 보관한다** — 세션 기본값(사용자 확인 대기 · TRD §10 F17). P3 착수 전에 설계를 다시 쓰고 재리뷰한다.
- **방법:** research medium · 설계-결정 모드. 갈래 3(동종 앱 피어 · Tauri/WebView2 메커니즘 · 설정/상태 분리 패턴) + 현행 인벤토리 1. 메인 grounding(로컬 클론·cargo 레지스트리 소스 대조) + cross-family(codex) 적대 리뷰 1회(판정 BLOCK → 아래 정정 반영).
- **범위:** 작업 내용 이력(대화·출력 기록)은 제외 — T-14 갈래.
- **사용자 기준:** 「업계 표준이 우선」(2026-10-02). 에이전트 도구에 한정하지 않고 일반 소프트웨어 관행도 채용한다.
- **확신도:** 확실 = 1차 출처 + 독립 교차확증 · 가능성 높음 = 1차 출처 단독 · 불확실 = 미지지.

## 1. 현행 (인벤토리 · 소스 읽기로 확인, 실행 실측 아님)

저장소가 셋이다.

| | 위치 | 갈리는 축 | 들어 있는 것 |
|---|---|---|---|
| 데몬 데이터 | 디버그 = 워크트리 루트 `.engram-data` · 릴리스 = `<exe>/data` (`discovery/src/lib.rs:84` · ADR-0134/0136) | 워크트리 · dev/release | `agents.json` · `presets.json` · `daemon.json`(잠금+발견) · `usage_rejects.json` · `mcp-config/` · `usage-probe/` · 데몬 로그 |
| 셸 데이터 | 데몬과 **같은 폴더** | 데몬과 같음 | `ui-settings.json`(테마 — 에이전트가 쓰고 셸은 부팅 때 죽은 창 항목만 쓸어낸다 · ADR-0166/0167) · 앱 로그. **레이아웃·창·탭은 메모리뿐**(`src-tauri/src/layout/mod.rs:26-35`) |
| 웹뷰 데이터 | Tauri 가 식별자로 강제하는 `%LOCALAPPDATA%\<identifier>` (`tauri-2.11.3/src/manager/webview.rs:534-545` — 확인) | dev/release(식별자) · 출처(포트) — **워크트리로는 안 갈림** | localStorage 의 챗 스타일 하나(`src/store/chatStyleStore.ts`) |

- 릴리스의 **포터블 원칙(ADR-0134 「폴더를 지우면 흔적 없음」)은 웹뷰 데이터가 이미 깨고 있다** — 웹뷰 폴더가 `%LOCALAPPDATA%` 로 간다.
- dev 앱은 Vite `localhost:1420`(`strictPort`)을 띄우고 실행 스크립트가 이미 떠 있는 Vite 를 재사용한다 — 워크트리 둘의 dev 앱이 같은 프론트 코드·같은 출처를 볼 수 있다(설정 읽기 추론 · 실측 안 함).
- 알려진 결함: T-22(`agents.json` 저장 실패가 성공으로 보고) · T-33(모르는 백엔드 종류 하나에 파일 전체가 손상 처리).
- 결합: ADR-0167 의 부팅 쓸기는 「레이아웃이 복원되지 않는다」를 전제로 한다. ADR-0060 은 레이아웃 영속에 버전 봉투 · `Unknown` 변형 · 마이그레이션을 요구한다.

## 2. 발견

### 2-1. 설정과 상태를 목적별로 가른다 — 가능성 높음~확실
- 사람이 고치는 **설정**과 기계가 쓰는 **상태**(창 배치·열린 탭)를 다른 저장소에 둔다: VS Code(`settings.json` ↔ `storage.json`·`state.vscdb` — https://code.visualstudio.com/docs/editor/settings) · Windows Terminal(`settings.json` ↔ `state.json` — `ApplicationState.cpp:16-17`) · JetBrains(`.idea/*.xml` ↔ `workspace.xml` — https://www.jetbrains.com/help/idea/configure-project-settings.html) · Zed(`settings.json` ↔ `db.sqlite`) · herdr(`config.toml` ↔ `session.json`, 클론 확인).
- ★**정정(적대 리뷰):** 「두 파일 이름·두 위치」가 보편 규칙인 것은 아니다. 보편적인 것은 **목적별 분리**다(Zed 상태는 SQLite·Local, Electron `userData` 기본은 Roaming — https://www.electronjs.org/docs/latest/api/app).

### 2-2. 화면 상태는 네이티브 쪽이 파일로 쓴다 — 확실(일반 소프트웨어) · 반례 있음(웹 우선 에이전트 도구)
- 네이티브/서버가 씀: VS Code(메인 프로세스 `windowsState`) · Windows Terminal · herdr · zellij(`session-layout.kdl`) · orca(메인 프로세스 `orca-data.json` 의 `workspaceSession`).
- 프론트 웹 저장소에 씀: t3code(`t3code:ui-state:v1` — `apps/web/src/uiStateStore.ts:6`, 클론 확인) · paseo(`workspace-layout-state` — `workspace-layout-store.ts:1724`, 클론 확인). 둘 다 웹 우선 구조.
- 우리 원칙(LLM-우선 제어 · 레이아웃 권위 = 셸, ADR-0035/0057)과 맞는 쪽은 네이티브 쪽이다.

### 2-3. 복원은 기본 꺼짐 + 설정으로 켠다 — 가능성 높음
- Windows Terminal 의 `firstWindowPreference` 기본값은 `defaultProfile`(새로 시작)이고, 저장된 레이아웃 복원은 사용자가 켠다(https://learn.microsoft.com/en-us/windows/terminal/customize-settings/startup). Chrome 도 「이어서 열기」가 설정이고, 비정상 종료 뒤에는 조용히 복원하지 않고 묻는다.
- 창 복원에는 **창 신원**이 필요하다 — Terminal 은 창 이름을 배치·탭과 함께 저장한다. 닫힌 창 · 사라진 에이전트 · 중복 팝아웃을 어떻게 다룰지를 스키마보다 먼저 정해야 한다.

### 2-4. 위치 — 기본은 OS 표준 폴더, 포터블은 배포 형태에 딸린 선택 — 가능성 높음
- 기본: VS Code · Zed(`%LOCALAPPDATA%\Zed`) · herdr(설정 `%APPDATA%` · 상태 `%LOCALAPPDATA%`) · orca(`%APPDATA%\orca`).
- 포터블: VS Code 는 **ZIP 배포에서만** `data/` 폴더를 두면 켜진다(https://code.visualstudio.com/docs/editor/portable). Windows Terminal 은 **별도 포터블 배포**에서 `.portable` 표식 → exe 옆 `settings/`(https://learn.microsoft.com/en-us/windows/terminal/distributions). 에이전트 오케스트레이터 피어 중 포터블 모드가 있는 곳은 못 찾았다.
- ★**정정(적대 리뷰):** 「설정 = Roaming, 상태 = Local」을 표준으로 박는 것은 과장이다. MS 지침은 Roaming = 기기 사이를 따라다녀야 할 작은 선호값, Local = 기기 고유 데이터다(https://learn.microsoft.com/en-us/uwp/api/Windows.Storage.ApplicationData). Terminal 도 `settings.json` 을 Local 에 둔다.
- ★**정정(적대 리뷰):** 피어가 OS 폴더를 쓴다는 것만으로 우리 포터블 배포를 바꿀 근거가 되지 않는다 — 우리 배포는 ZIP 하나이고, VS Code 의 ZIP 배포는 포터블을 지원한다.

### 2-5. dev/release 분리 = 시작 때 다른 루트 이름 + 환경변수 — 확실
- herdr `herdr-dev`(`src/config/io.rs:22`, 클론 확인) · orca `orca-dev`(`configure-process.ts:153-184`, 클론 확인) · t3code `dev/` ↔ `userdata/`(`apps/server/src/config.ts:132`, 클론 확인) · Zed `0-<channel>`. 환경변수 재지정은 공통(T3CODE_HOME · PASEO_HOME · ORCA_USER_DATA_PATH · Zed `--user-data-dir`).
- **워크트리별 격리를 하는 피어는 없다** — orca 는 dev 워크트리들이 `orca-dev` 를 함께 쓴다고 주석에 적어 둔다. 우리 `.engram-data` 가 피어보다 더 격리돼 있다. OS 표준 폴더로 가면 이 격리는 환경변수 없이는 사라진다(적대 리뷰 지적).

### 2-6. Tauri 메커니즘 — 확실(소스 대조)
- 우리 버전: tauri 2.11.3 · wry 0.55.1 · webview2-com 0.38.2(`Cargo.lock`).
- 창 단위 웹뷰 폴더 지정: `WebviewWindowBuilder::data_directory(PathBuf)` 있음(`tauri-2.11.3/src/webview/webview_window.rs:1024`). 설정 키 `dataDirectory` 는 상대 경로만 받는다.
- 환경 옵션(`additionalBrowserArgs` 등)이 다른 웹뷰는 다른 웹뷰 폴더를 써야 한다(MS — https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/user-data-folder). 팝아웃이 `additional_browser_args` 를 쓴다(`popout.rs:117`).
- 웹뷰 폴더에는 캐시만이 아니라 쿠키 · 권한 · **localStorage** · IndexedDB 가 든다(MS 같은 문서). 폴더를 옮기면 챗 스타일 값이 옛 폴더에 남는다. 세션이 살아 있는 동안 그 폴더를 지울 수 없다.
- ★**적대 리뷰가 낸 사실 · 버전 정정:** Tauri 에 **앱 디렉터리 일괄 재지정 설정 `app > appDirectoriesOverride`** 가 있다 — 앱 경로·플러그인·**기본 웹뷰 폴더**를 함께 옮긴다(https://v2.tauri.app/reference/config/#appdirectoriesoverride). 리뷰는 「2.10 에 들어왔다」고 했지만, **우리 2.11.3 소스에는 없다**(`tauri-2.11.3/src/path/desktop.rs:238-260` — 식별자만 붙인다). 이 기능이 처음 나온 릴리스는 **tauri 2.12.0**(tauri-utils 2.10.0)이다(`gh api repos/tauri-apps/tauri/releases` 본문 검색). → 쓰려면 **Tauri 2.12 업그레이드가 선행**이다. 실행 파일 기준 상대 경로는 Program Files 같은 쓰기 불가 위치에서 실패한다는 Tauri 경고도 있다.
- 공식 플러그인: `tauri-plugin-window-state` 는 크기·위치·최대화·표시·장식·전체화면만 `app_config_dir/.window-state.json` 에 저장한다(창 라벨이 키). `tauri-plugin-store` 는 AppData 기준 JSON. 둘 다 지금 의존성에 없다. 위 재지정 설정이 있으면 둘 다 그 루트를 따른다.

### 2-7. 파일 견고성 — 가능성 높음
- 항목 하나가 나쁘다고 파일 전체를 버리지 않는다 — Zed 는 `settings.json` 의 값 하나가 틀리면 설정 전체를 기본값으로 되돌린다(zed#12389 · #9989). 우리 T-33 이 같은 부류다. 항목별로 파싱하고, 파싱에 실패한 파일은 덮어쓰지 않는다.
- 원자 쓰기: 데몬은 tmp → fsync → rename 을 쓴다. ★**정정(적대 리뷰):** Windows 에서는 이것이 완전한 명세가 아니다 — `ReplaceFileW`(백업 동작 포함)를 쓸지와 실패 시 복구를 정해야 한다(https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew).
- 저장 시점: 종료 때 한 번 저장만으로는 크래시·강제 종료에 약하다 — Zed 는 주기적으로 스로틀해 저장하고, Firefox 는 살아 있는 복구 파일 + 회전 백업을 둔다. 자주 바뀌는 값은 레이아웃 덩어리에서 뺀다(zed#64823).
- 도구와 사람이 같은 설정 파일을 고칠 때: 주석을 보존하며 한 속성만 고치는 편집(`jsonc-parser`) · 파일 감시 리로드. 우리에겐 「모든 쓰기를 같은 명령 표면으로 보내 쓰는 쪽을 하나로」가 자연스럽다(추론 — 출처 없음).

## 3. 결정 축과 선택지

| # | 축 | 선택지 | 표준 근거 |
|---|---|---|---|
| 1 | **데이터 루트 위치** | (가) OS 표준 폴더 기본 + 포터블 표식으로 exe 옆 · (나) 포터블 기본 유지 + 웹뷰 폴더도 그 안으로 · (다) OS 표준 폴더만 | VS Code ZIP · Windows Terminal = (가) 꼴 · 우리 현행 = (나)에 가까움 |
| 2 | 화면 상태 소유 | 셸이 상태 파일을 쓴다 vs 프론트 localStorage | 일반 소프트웨어 = 네이티브. 프론트는 웹 우선 도구뿐 |
| 3 | 복원 정책 | 기본 꺼짐 + 설정으로 켬 vs 항상 복원 · 크래시 뒤 묻기 | Windows Terminal · Chrome = 기본 꺼짐/설정 |
| 4 | 설정 파일 | 설정(사람·LLM 이 고침)과 상태(기계가 씀)를 다른 파일로 · 챗 스타일을 localStorage 에서 설정 파일로 | 목적별 분리(2-1) |
| 5 | 워크트리 격리 | 유지(루트를 워크트리마다) vs dev 하나로 공유 | 피어는 공유. 우리는 격리가 더 강함 |
| 6 | 데몬 데이터 견고성 | T-22 · T-33 수정 · Windows 원자 쓰기 명세 | 2-7 |

- **1 이 4 · 5 를 가른다:** (가)·(다)면 워크트리 격리를 환경변수로 따로 지켜야 하고, 웹뷰 폴더 이전은 Tauri 2.12 의 재지정 설정으로 한 번에 풀린다. (나)면 지금 구조를 유지하면서 웹뷰 폴더만 창마다 `data_directory` 로 옮긴다(2.11.3 에서 가능).

## 4. 거부 후보 (ADR 거부 대안 후보)

- **레이아웃을 프론트 localStorage 에** — LLM-우선 제어와 충돌(셸·LLM 이 못 읽음), 출처·웹뷰 폴더에 묶여 dev/release·포트가 바뀌면 잃는다. 웹 우선 피어(t3code·paseo)만 이 길이다.
- **`tauri-plugin-window-state` 를 그대로** — 창 기하만 저장하고 탭·슬롯·분할을 모른다. 2.11.3 에선 위치가 `app_config_dir` 고정이라 포터블·워크트리 루트를 못 따른다(2.12 재지정 설정이면 따른다).
- **한 파일에 전부** — orca 는 자주 바뀌는 데이터 때문에 보조 파일을 따로 떼야 했다.
- **지금 SQLite** — 저장할 상태가 작다. 저장 엔진은 T-14 갈래에서 정한다.

## 5. 적대 리뷰 결과 (codex · 판정 BLOCK)

반영한 지적: Roaming/Local 과장 · 포터블 → OS 폴더 전환의 논리 공백 · 웹뷰 폴더에 localStorage 가 듦 · 「흔적 없음」 과장(환경변수·정책이 웹뷰 폴더를 재지정할 수 있고 옛 폴더는 자동으로 지워지지 않음) · 복원 정책 누락 · 창 신원 누락 · 워크트리 격리 논리 공백 · Windows 원자 쓰기 명세 · 실행 파일 상대 경로의 쓰기 권한.

**부분 반박:** 「`appDirectoriesOverride` 가 2.10 에 들어와 2.11.3 에 있다」 — 기능은 실재하지만 우리 2.11.3 소스에 없다(`path/desktop.rs:238-260`). 처음 나온 릴리스는 tauri 2.12.0. → 「업그레이드 선행」으로 적었다.

## 6. 한계 · 공백

- WezTerm · Warp · superset · Obsidian · electron-store 는 보지 않았다.
- 버전 봉투·마이그레이션 실무의 1차 출처를 못 찾았다.
- 크래시 루프 격리(복원하다 죽은 레이아웃을 다시 복원하지 않는 장치)의 출처를 못 찾았다.
- 웹뷰 폴더 내부 형식(LevelDB) · 하위 폴더 이름(`EBWebView`)은 확인하지 않았다.
- 두 창이 서로 다른 웹뷰 폴더를 한 프로세스에서 쓸 때의 실동작은 확인하지 않았다.
- Tauri 2.12 업그레이드 비용은 재지 않았다.
