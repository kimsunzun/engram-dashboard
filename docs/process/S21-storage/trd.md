# TRD — 저장 관리 구조: 데이터 배치 · 웹뷰 폴더 · 설정 · 화면 상태 (S21)

> 상태: **초안 6판 (2026-10-02)** — **6판 = 5판 리뷰(설계자 BLOCK · 설계자-파괴자 FIX) 반영 + 사용자 결정 D4–D8: 묻는 표면은 main 창 안 모달 · 기본 모양 예외 없음 · 답만 사본을 푼다 · `--hidden` · 쓰기 실패는 따로 다루지 않는다 + 셸 실행 잠금 · 답의 디스크 고정 · 단계 다시 자르기(§14-10). 6판 재리뷰(설계자 PASS · 설계자-파괴자 FIX) 반영 = §14-11.** 5판 = P3 착수 전 사용자 결정 D1 · D2 · D3 — F17 철회 · 크래시 사본 한 개 · 크래시 뒤 늘 묻는다(§14-9). 아래는 4판까지의 경위다.
>
> 4판까지: **리뷰 2회 반영**. 2판 = 1차 리뷰(설계자 · 파괴자 — 둘 다 FIX) 반영 · 3판 = 2차 리뷰(파괴자 FIX · 설계자 BLOCK — 같은 핵심 결함: 대기분이 디스크 확정 전에 지워진다) 반영 — 대응표 = §14. **이후 P1 구현 · 코드 리뷰 뒤 사용자 결정(2026-10-02)으로 첫 부팅 이전을 걷었다 — 옛 데이터는 옮기지 않고 버린다(§3-2 · §14-5).** P1 은 구현됐다(`fe8b552`). **4판 = P2 착수 전 사용자 결정 셋 반영 — 저장 범위 표 · `ui-settings.json` 무이전 · ~~비정상 종료 스냅숏 회전 보관~~(5판에서 철회 — §14-9)(§14-6) + 4판 리뷰(light · 설계자-파괴자 FIX) 반영과 사용자 결정 U1(사용자가 본인뿐이라 옛 값을 다루지 않는다) · U2(챗 스타일은 지금 11키만 가볍게)(§14-7). ★§14-7 반영분은 아직 재리뷰 전이다★.** **P2a 코드 리뷰(deep) 뒤 §5 설정 사양을 구현에 맞췄다(§14-8).** **P3a 착지(`41348d9`) 뒤 코드 리뷰 반영과 사용자 결정 F21(ADR-0274)에 §5-3 · §6 을 맞췄다(§14-12).** **P3b1–P3b3 착지(`5a2cf7f` · `f9db35a`) 뒤 코드 리뷰 반영과 사용자 결정 F8 · F13 에 §0 · §6 · §9-2 · §10 · §12 를 맞췄다(§14-13).** **P3c1 착지(`064b61c`) 뒤 코드 리뷰 반영(수정 5라운드)과 사용자 결정(도움말의 답 정책 · 2026-10-05) · 세션 판단(사용자 위임)에 §0 · §5-4 · §6 · §8 · §9 · §11 · §12 를 맞췄다(§14-14).** **P3c2 착지(`7b742fb`) 뒤 코드 리뷰 반영과 사용자 결정(상태 파일 안내 · 확인 목록 확정 — 2026-10-05) · 세션 판단(사용자 위임 — 안내의 화면 모양)에 §0 · §5-4 · §6 · §8 · §9 · §10 · §11 · §12 · §14 를 맞췄다(§14-15).** **후속 「안내 덮기」 착지(`125d190`) 뒤 사용자 결정(상태 파일 안내는 레이아웃을 밀지 않고 덮는다 · 가드 ⅱ 도 같은 안내를 띄운다 — 2026-10-06 · ADR-0276)과 그것을 실현한 구현(`restore.status` 에 `saves` — 세션 판단) · 코드 리뷰 반영에 §0 · §5-4 · §6 · §8 · §9-2 · §11 · §12 · §14 를 맞췄다(§14-16).** **「연결 띠 덮기」 착지(`9318b5c`) 뒤 사용자 결정(연결 끊김 안내도 레이아웃을 밀지 않고 덮는다 — main · 팝아웃 · 트리 · 2026-10-06 · ADR-0277)과 그 구현(세션 판단)에 §0 · §6-5 · §8 · §9-2 · §11 · §14 를 맞췄다(§14-17).** **P3c3 착지(`e904161`) 뒤 사용자 결정(꺼진 에이전트를 가리키는 슬롯은 부재 막 · 슬롯 활성화 단추 없음 · 「대상 없음」은 에이전트 슬롯 메뉴 그대로 · 모르는 내용 슬롯에 「비우기」 없음 · 포커스 링은 막 위 — 2026-10-06 · ADR-0280)에 §0 · §5-7 · §6-2 · §6-8 · §8 · §9-2 · §10 · §11 · §12 · §14 를 맞췄다(§14-18).** **P3d 착지(`c85b2ac`) 뒤 사용자 결정(F7 — 화면 테마 UI 없음 · 2026-10-06 / `window.setTheme` 은 지목한 창이 못 받았을 때만 오류 · 2026-10-07)과 그 구현에 §0 · §3-5 · §4 · §5-3 · §5-4 · §5-6 · §5-7 · §6-3 · §6-5 · §6-7 · §8 · §9-2 · §10 · §11 · §13 · §14 를 맞췄다(§14-19).** **사용자 결정(P3e 뺌 — 슬롯 보기 모드는 영속하지 않는다 · 2026-10-07)으로 L3 = P3a–P3d 로 닫고 §0 · §1 · §6-0 · §6-1 · §6-2 · §8 · §9 · §10 · §11 · §13 을 맞췄다(§14-20).** **P4 착지(`11f8787`) 뒤 사용자 결정(M4 (c) — 못 쓰면 기본 자리로 물러나 진행 · main 을 못 만들면 앱을 끝낸다 — 2026-10-07)과 그 구현 · 코드 리뷰 · QA 에 §0 · §4 · §6-5 · §6-7 · §8 · §9-2 · §10 · §11 · §12 를 맞췄다(§14-21 · ADR-0283).**
>
> **입력:** PRD 결정 = [`docs/research/storage-management-survey-2026-10-02.md`](../../research/storage-management-survey-2026-10-02.md) 「0. 결정」(구속) · 같은 보고서 §1–§6(근거). **판독 기준** = 브랜치 `v0.3.3/feat/storage` HEAD `d4ff3f7` · Tauri 2.11.3 / tao 0.35.3 / tauri-runtime-wry 2.11.3 / tauri-plugin-single-instance 2.4.2 = 이 PC cargo 레지스트리 소스.
>
> **앵커:** ADR-0003 · ADR-0006 · ADR-0012 · ADR-0024 · ADR-0035/0057 · ADR-0051 · ADR-0054 · ADR-0056 · ADR-0060 · ADR-0078 · ADR-0102 · ADR-0134/0135/0136/0137 · ADR-0149 · ADR-0155 · ADR-0166/0167 · ADR-0169 · ADR-0171 · ADR-0222 · ADR-0230.
>
> 표기: **[고름]** = 사용자 체감이 없는 내부 구현이라 이 문서가 고른 것. **(추천 · 세션 기본값 — 사용자 확인 대기)** = 사용자 체감 갈림길의 기본값(§10) — 「추천」이 빠진 것(F20)도 같은 뜻이고, 이 문서가 추천 근거를 내지 않은 기본값이다. **★명시 확인★** = 사용자에게 따로 올릴 항목 — 답이 없으면 기본값으로 진행하되, §9-2 단계 행에 「착수 전 … 답」이 붙은 것은 그 단계 전에 답을 받는다. **U1 · U2** = 2026-10-02 사용자 결정(§14-7). **D1 · D2 · D3** = 2026-10-02 사용자 결정(§14-9 — 크래시 사본). **D4–D8** = 2026-10-02 사용자 결정(§14-10 — 모달 · 사본 수명 · 쓰기 실패). **[미검]** = 소스 읽기만 했고 실행으로 확인하지 않은 것. **세션 판단(사용자 위임)** = 사용자가 판단을 세션에 맡긴 뒤(2026-10-03 「막힐때까지 너가 알아서해」 · 2026-10-04 「막히면 멈추고 아니면 쭉쭉」 · P3c2 「쭉 진행해」) 세션이 추천안을 고른 것 — 사용자가 고른 것이 아니고, [고름] 과 달리 사용자 체감이 남을 수 있어 따로 표시한다. **사용자 확인 2026-10-05** = 앞선 세션 판단(사용자 위임) 넷을 사용자가 「세션 판단을 그대로 둔다」로 확인한 것(§14-15).

---

## 0. 결론 (먼저)

| 무엇 | 어디 | 요지 |
|---|---|---|
| 경로 계산 | discovery `DataLayout`(신설) | 디렉터리와 둘 이상의 프로세스가 보는 파일(`daemon.json`) 경로의 단일 출처. 데몬·셸·net 은 받은 경로만 쓴다 |
| 옛 데이터 | 없음(걷음 — §3-2) | **옮기지 않고 버린다.** 새 코드는 옛 평면 파일 · `.engram-data` 를 읽지도 옮기지도 지우지도 않는다. 잠금은 `daemon\run\daemon.json` 하나 |
| 웹뷰 폴더 | 셸 | 정적 창 둘을 Rust 에서 만들고(`create:false` + `from_config`) 모든 창에 같은 `data_directory` + 같은 브라우저 인자(마무리 함수 하나 — `webview_env.rs`). 만들기 전에 쓰기 확인 한 번 — **못 쓰면 Tauri 기본 자리로 물러나 진행(M4 · 사용자 결정 2026-10-07)** · main 을 못 만들면 종료 코드 1 · ✅ P4 `11f8787` · ADR-0283 |
| 저장 범위 | §6-0 | 남기는 것 · 버리는 것을 항목별로 정했다(사용자 결정). 취향 = `settings.json` · 기록 = `state.json`. ~~슬롯 보기 모드(터미널/챗)는 프론트 메모리에서 셸 소유 슬롯 값으로 옮겨 남긴다(P3e)~~ → **P3e 를 뺐다(사용자 결정 2026-10-07 · §14-20)** — 슬롯 보기 모드 덮어쓰기는 지금처럼 프론트 메모리뿐(창마다 따로 · 재시작하면 사라짐) |
| 설정 | 셸 `settings` 모듈(신설) | 스키마 표 한 줄 = 설정 하나. 버스 명령 넷 + 같은 서비스를 부르는 Tauri 껍데기. 파일을 아는 것은 저장 계층 하나. **기본값에서 시작 — 사용자가 본인뿐이라 옛 값은 다루지 않는다(U1 · §3-5).** 챗 스타일은 지금 11키를 그대로 옮기고 `chat.style.*` 는 임시 이름공간이다(U2 · §5-1) |
| 화면 상태 | 셸 `state` 모듈(신설) | 파일 일은 **단일 인스턴스 관문 뒤 · 어느 창보다 먼저**(단일 인스턴스 바로 뒤 플러그인 — 셸 실행 잠금 `shell\run\state.lock` 을 쥔 뒤 읽는다). **`state.json` 을 쓰는 것은 부팅 첫 쓰기 뒤 기록기 스레드 하나뿐.** **비정상 종료 뒤엔 `state.json` 을 사본 `state.crash.json` 한 개로 떠 두고 기본 화면 + main 창 안 모달로 늘 묻는다**(D1–D5 · §6-5 · §6-7). 답만 사본을 풀고(D6), 답은 `state.json` 의 해시로 디스크에 붙는다. 런타임 복원은 조율자 하나가 한 커밋 지점에서 바꾼다 |
| 순서 | §9 | P1 → P2(설정) → P3(상태) → P4(웹뷰). **머지 단위 = L1(P1) · L2(P2a–c) · L3(P3a–d — ~~P3e~~ 는 뺐다, 사용자 결정 2026-10-07 · §14-20) · L4(P4), 전체가 한 릴리스.** ★P3 착수 전 게이트 — 통과(6판 재리뷰 · `ae3f6e7` — §14-11 · §9-1)★ · **P3a 착지 `41348d9`** · **P3b1–P3b3 착지 `5a2cf7f` · `f9db35a`**(정상 종료 뒤 조용한 복원이 돈다 — 크래시 사본 · 모달은 P3c) · **P3c1 착지 `064b61c`**(크래시 사본 + 답할 길 — 비정상 종료 뒤 사본 · 기본 화면 · 버스 `restore.*` 로 답한다 · 모달은 P3c2 — §9-2) · **P3c2 착지 `7b742fb`**(복원 모달 · 키 바인딩 멈춤 · 상태 파일 안내 · `restore.status` 에 `durable` · `state_file` — §9-2) · **후속 착지 `125d190`**(상태 파일 안내가 레이아웃을 덮는다 · 가드 ⅱ 안내 · `restore.status` 에 `saves` — ADR-0276 · §14-16) · **연결 띠 덮기 착지 `9318b5c`**(연결 끊김 안내도 덮는다 — main · 팝아웃 · 트리 · ADR-0277 · §14-17) · **P3c3 착지 `e904161`**(복원된 슬롯 표시 — 꺼진 에이전트는 부재 막 · 지운 에이전트는 「대상 없음」 · 모르는 내용 슬롯 — ADR-0280 · §14-18) · **P3d 착지 `c85b2ac`**(창별 테마 — 들임 = 버스 `window.setTheme` · `window.getTheme` · 걷음 = `ui.refresh` · 부팅 쓸기 · `ui_settings.rs` — §14-19) · **L3 master 머지 `79b8d09`** · **P4 착지 `11f8787`**(L4 — 웹뷰 폴더 `<root>\webview` · 정적 창을 setup ⑧ 에서 만든다 · 못 쓰면 물러남 · main 생성 실패 = 종료 — ADR-0283 · §14-21) |

## 1. 목표 · 범위

- **안(in):** 「0. 결정」의 데이터 배치(옛 데이터는 버림 — §3-2) · 웹뷰 폴더 이전 · 설정(`settings.json` + 명령 넷 + 챗 스타일 권위 이전(지금 11키 그대로 — U2) + `ui-settings.json` 폐지(옛 값은 다루지 않는다 — U1 · §3-5) + 도움말·프라이밍) · 화면 상태(`state.json` + 창 신원 + 복원 + 비정상 종료 확인) · 창별 테마의 영속 · 대상 없는 슬롯의 두 상태 표시 · **저장 범위 표(§6-0)** — ~~슬롯 보기 모드의 셸 이전 · 영속 포함(P3e)~~(뺐다 — 사용자 결정 2026-10-07 · §14-20).
- **밖(out):** §6-0 「남기지 않는 것」(펼침 · 스크롤 고정 · 선택 · 입력 초안 · 트레이 숨김) · **슬롯 보기 모드 덮어쓰기의 영속(옛 P3e — 사용자 결정 2026-10-07 · §14-20 · T-51)** · 프리셋 시스템 · 테마 프리셋 · 내장 테마 JSON 통일 · 작업 이력(T-14) · 개발 Vite 포트 공유 · T-22(명부 저장 실패의 성공 보고) · T-33(모르는 백엔드 종류로 명부 전체 손상) — **새 배치는 `agents.json` 의 자리만 바꾸므로 둘 다 건드리지 않는다** · 슬롯 종류별 설정의 코드(폴더 자리만 문서로 둔다 — §7) · 설정 화면 · 테마를 고르는 화면 메뉴(§10 F7) · **옛 값 처리 전부**(배포본(v0.1.0–v0.3.2)은 있으나 사용자가 본인뿐이라 지킬 사용자 데이터가 없다(사용자 판단 2026-10-02) — U1) · **챗 스타일 스키마 다듬기**(U2 — `chat.style.*` 는 임시).

## 2. 폴더 구조와 경로 계산

### 2-1. 최종 구조

```
<root>\              릴리스 = <exe 폴더>\data · 개발 = <워크트리>\.engram-dev(새로 시작 — 옛 .engram-data 는 건드리지 않는다, §3-2) · 테스트 = ENGRAM_DATA_DIR (현행 규칙 그대로 — discovery/src/lib.rs:84)
├─ daemon\
│  ├─ state\         지킨다: agents.json · presets.json · usage_rejects.json (+ .corrupt-*)
│  └─ run\           버려도 된다: daemon.json(잠금+발견·WS 토큰) · mcp-config\(토큰) · usage-probe\
├─ shell\
│  ├─ config\        취향(지우면 기본값): settings.json · (장래) slots\<종류>.json · themes\<이름>.json (+ .corrupt)
│  ├─ state\         기억: state.json · state.crash.json(비정상 종료 뒤 답할 때까지만 — 한 개, §6-7) (+ .corrupt — 파일마다 하나, ADR-0274)
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

그 파일은 `{"theme":…, "windows":{label:…}}` 하나이고 자리는 루트 평면(`<root>\ui-settings.json` — 옛 `ui_settings.rs:228-236` · ★그 소스 파일은 P3d 에서 통째로 지웠다 — 옛 줄은 `git show c93e664:src-tauri/src/ui_settings.rs` 로 연다★)이다. ★**배포본(v0.1.0–v0.3.2)은 있으나 사용자가 본인뿐이라 지킬 사용자 데이터가 없다(사용자 판단 2026-10-02) — U1. 값을 어디로도 옮기지 않는다**★. 칸마다 새 집이 서는 단계에서 셸이 그 칸을 **읽지 않게** 될 뿐이다.

- **P2a — 전역 칸:** `settings.json` 은 **기본값에서 시작한다.** P2a 부터 셸은 `ui-settings.json` 의 `theme` 를 **있든 없든 못 쓸 값이든 무시한다.** ★P2a 전에는 `theme` 가 없거나 못 쓸 값이면 파일 전체를 거부했다(옛 `ui_settings.rs:300,342-358` — 이 줄 번호는 TRD 초안 `c93e664` 기준이다 · `git show c93e664:src-tauri/src/ui_settings.rs` 로 연다) — 그대로 두면 L2 동안 살아 있는 창별 칸까지 함께 버려진다★. 그래서 파서가 `theme` 를 보지 않게 한다. L2 동안 이 파일은 창별 칸의 집으로 살아 있다(§5-7 · §9-1).
- **P3d — 창별 칸:** 창 테마는 모두 비어서 시작하고(`theme.default` 를 따른다) 창별 값은 `window.setTheme` 으로만 생긴다(§5-6). P3d 부터 셸은 이 파일을 읽지도 쓰지도 않는다(쓸기 · `ui.refresh` 삭제 — §5-7). 남은 파일은 따로 다루지 않는다(U1). ✅ P3d `c85b2ac` — 그 파일을 읽던 `src-tauri/src/ui_settings.rs` 를 통째로 지웠다(§14-19).
- 그래서 이전 코드(`settings/migrate.rs` · `state/migrate.rs`)는 없다.

### 3-6. (삭제 — U1 · §14-7)

옛 3·4판의 「챗 스타일 localStorage 일회 가져오기」(F5)는 사용자 결정 U1 로 걷었다 — 사용자가 본인뿐이라 지킬 사용자 값이 없다(F5 = (b)).

## 4. 웹뷰 폴더 → `<root>\webview` (P4 — ✅ 착지 `11f8787` · 결정 = ADR-0283 · 착지 기록 = §14-21)

- **사실(소스 대조):** `WebviewWindowBuilder::data_directory(PathBuf)` 는 절대 경로를 그대로 쓴다(`webview/webview_window.rs:1024` · `manager/webview.rs:534-545` — 지정이 없을 때만 `LocalData/<identifier>` 강제). 설정 키 `dataDirectory` 는 **상대 경로만** 받아 `LocalData/<label>/` 아래로 붙인다(`webview/mod.rs:392-418`).
- **결정 [고름]:** `tauri.conf.json` 두 창에 `"create": false`(`tauri-utils-2.9.3/src/config.rs:1936`) → 셸 `setup` 에서 `WebviewWindowBuilder::from_config`(`webview_window.rs:150`). Tauri 가 설정 창을 만드는 자리도 사용자 `setup` 바로 앞이다(`app.rs:2521-2531`). 창 선언은 설정에 남아 `hidden_window_labels`(`view_commands.rs`)는 그대로 돈다(이 판이 함께 들던 `declared_window_labels` 는 그것을 쓰던 부팅 쓸기와 함께 P3d 에서 걷혔다 — §6-5 ⑨). ✅ P4 — `state/placement.rs` 의 `build_static`(설정에서 그 label 의 선언을 찾아 `from_config` · 저장된 자리 · 크기를 빌더에 준다). 두 창이 `create:false` 이고 부팅이 그 둘을 다 만드는지는 시험이 지킨다(`every_config_window_is_left_to_the_boot_and_the_boot_knows_them_all`).
- **동일 환경 불변식(ADR-0054 확장):** 공통 마무리 함수 하나가 `data_directory(layout.webview_dir())` 와 `additional_browser_args(WEBVIEW2_BROWSER_ARGS)` 를 함께 붙이고 정적 창 둘과 팝아웃이 전부 그것을 지난다. 브라우저 인자 값의 정본은 Rust 상수다. ✅ P4 — 마무리 = `src-tauri/src/webview_env.rs` 의 `WebviewEnv::finish`(빌더에서 manage 하는 셸에 하나인 인스턴스 · `Clone` 없음). 붙이는 것은 넷이다 — 폴더(정했을 때) · 브라우저 인자 · 브라우저 확장 끔 · 스크롤바 모양 기본(wry 0.55.1 이 WebView2 환경에 싣는 넷 — 그 파일 머리). 정적 창 = `state/placement.rs` 의 `build_static` · 팝아웃 = `commands/popout.rs` 의 `build_window`(인스턴스가 manage 돼 있지 않으면 만들지 않고 오류). 상수는 `popout.rs` 에서 `webview_env.rs` 로 옮겼고 `tauri.conf.json` 의 `additionalBrowserArgs` 두 줄은 걷었다 — 설정 창 선언이 환경 옵션을 기본값으로 두는지는 시험이 지킨다(`webview_env.rs` 의 `config_windows_leave_every_environment_option_at_the_default`).
- **쓰기 확인(M4 — ★사용자 결정 2026-10-07: 아래 「대화상자 후 종료」 대신 기본 자리로 물러나 진행 · warn 로그 · 판정은 프로세스당 한 번 — §10 M4 행이 정본★ — 아래는 옛 안):** 창을 만들기 **전에** `webview_dir()` 에 `check_data_dir_writable`(discovery 기존 함수 — 2-2(ADR-0282) 뒤 자리 = 셸 `src-tauri/src/discovery/mod.rs` · 프로브는 base `writable`)을 돌리고, 실패하면 **네이티브 대화상자**(이미 의존하는 `tauri-plugin-dialog` 의 Rust API)로 경로와 조치를 보인 뒤 종료한다 — WebView2 는 못 쓰는 폴더에서 창 없이 조용히 실패할 수 있다(ADR-0054 의 유령 창과 같은 모양) [미검]. ~~**네트워크 공유 위 데이터 폴더는 당분간 지원하지 않는다고 문서화**한다 — ADR-0134 결정 3(공유 폴더 지원)의 개정 대상(§11).~~ ★뺀다(사용자 결정 2026-10-07 · §10 M4)★ — ADR-0134 결정 3 의 공유 폴더는 데몬끼리의 배제 문제이고 웹뷰 폴더는 클라이언트 쪽이다. ✅ P4 — 지금 모양: `WebviewEnv::chosen_dir` 가 처음 불릴 때 한 번(`OnceLock` · 동시에 불려도 한 번) `webview_dir()`(셸 `src-tauri/src/discovery/layout.rs` — 2-2 흡수 뒤 자리 · ADR-0282) 를 **절대 경로로 바꾼 뒤** 확인하고 같은 값을 넘긴다(데이터 루트는 `ENGRAM_DATA_DIR` 로 상대 경로일 수 있다) · 처음 부르는 자리 = 사용자 setup 첫머리(단일 인스턴스 관문 뒤 · 어느 창보다 앞) · 못 쓰면 폴더를 넘기지 않고 warn(사유만 — 그 오류의 데이터 루트용 조치 문구는 떼고 경로는 로그 칸) · 확인은 폴더를 만들지 않는다(만들어 봤으면 되돌린다 — 폴더는 창을 만들 때 Tauri 가 만든다).
- **덤:** `--hidden` 부팅이면 main 을 처음부터 숨긴 채 만든다(~~지금은 보였다 숨는다 — `lib.rs:165-169`~~). ✅ P4 — 옛 「보인 채 만들고 setup 끝에 숨기는」 main 의 길은 걷었다(setup 끝 숨기기 `hide_main_ui` 가 지금 숨기는 것은 ⑨ 의 팝아웃이다 — F13). ★그래도 `--hidden` 이면 그 숨기기를 늘 부른다 — 팝아웃이 없어도 건너뛰지 말 것★: `hide_main_ui` 는 창을 숨기면서 숨긴 창을 사용량 관심에서 빼고, main 은 이미 숨어 있어도 이 길로 관심에서 빠진다(숨기기 경로는 하나다 — ADR-0229 · `lib.rs` 의 그 자리 주석). ★숨긴 채 만드는 창은 포커스를 받지 않는다(`focused(false)` — `--hidden` 의 main · 설정이 숨긴 트리 창)★ — QA 가 고치기 전 보이는 부팅 3/3 에서 숨은 트리 창이 main 의 전경을 가져가는 것을 보았고, 고친 뒤 main 이 전경이다(원인이 wry 의 만들기 끝 `MoveFocus` 라는 것은 소스 독해 — `build_static` 주석 [미검]).
- **정적 창 생성 순서(I6 · §14-10):** `from_config` 로 정적 창을 만드는 자리는 상태 부팅의 실행 표식(§6-5 ⑤) 뒤다 — 사용자 setup 은 늘 그 뒤라 순서는 저절로 서고, 만들 때 저장된 main · 트리 속성(§6-5 ⑧)을 바로 입힌다. ✅ P4 — ⑧ `state::placement::restore_windows`(정적 창 → 팝아웃 → 테마 차례).
- **정적 창을 못 만들면(사용자 결정 2026-10-07 — ✅ P4):** **main = 앱을 끝낸다** — error 로그 → 시작 실패 표지 → 앱 종료 요청(`app.exit(1)`) → setup 을 거기서 끝낸다(⑩ · 복원 포트 · 트레이 없음) → `RunEvent::Exit` 가 기록기 `Final` 을 쓰고 셸 실행 잠금을 놓은 뒤 `std::process::exit(1)`(종료 요청만으로는 tao `ControlFlow::Exit` 라 종료 코드가 0 이다 — `lib.rs` 주석). **트리 창 = log 하고 그 창 없이 계속.** ★`build()` 가 `Ok` 여도 런타임이 창을 못 만든 조용한 실패를 잡는다★ — tauri-runtime-wry 2.11.3 은 메인 스레드의 창 만들기 실패(WebView2 런타임 없음 — `WebviewRuntimeNotInstalled`)를 log 만 하고 창을 돌려준다(QA 실측 — `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER` 로 런타임을 가렸을 때). 그래서 만든 직후 게터 한 번(`is_visible`)으로 확인하고 답이 오류면 실패로 친다(`confirm_created` — 사유에 `webview_version` 조회의 답을 붙인다). 릴리스 빌드에서는 그보다 먼저 Tauri 자체의 오류 대화상자 「Could not find the WebView2 Runtime.」이 뜬다(우리 것이 아니다 — tauri-runtime-wry 2.11.3 `src/lib.rs:4757-4765` · `cfg(all(not(debug_assertions), windows))`) [소스 독해 — QA 는 debug 빌드만].
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
- 원자 쓰기 = 임시 이름(`settings.json.tmp<pid>.<n>` — `<n>` 은 프로세스 전체에서 부를 때마다 하나씩 느는 번호라 같은 프로세스 안의 임시 이름은 겹치지 않는다 · ★`io` 가 막는 것은 임시 이름 충돌이 아니라 겹친 RMW 사이의 갱신 유실이다★) → `sync_all` → rename(데몬 `persistence/mod.rs:57-85` · 옛 `ui_settings.rs:687`(P3d 에서 지웠다 — 줄 번호는 `git show c93e664:src-tauri/src/ui_settings.rs` 기준) 과 같은 방식 — `write_atomic` 을 셸 공용 `fsutil.rs` 로 옮겨 설정·상태가 함께 쓴다 · P3a 에서 떠 두기도 `fsutil::copy_aside` 로 옮겼다 — 설정·상태가 같은 함수를 부르고 로그는 부르는 쪽이 낸다, §6-2). rename 이 접근 거부 · 공유 위반 · 잠금 위반(Windows 5 · 32 · 33)이면 20 ms 간격으로 5번까지 다시 한다(백신 · 색인기가 잠깐 쥐는 경우). `ReplaceFileW` 는 쓰지 않는다(§12 R7). **쓸 원문이 64 KiB(읽기 상한)를 넘으면 쓰지 않고 `INTERNAL`** — 넘는 원문을 쓰면 다음 적재가 파일 전체를 못 쓴다고 접는다.
- **항목별 관용:** 모르는 키 = 무시하되 파일에 그대로 남긴다 · 아는 키의 못 쓸 값 = 기본값으로 접고 warn, 파일에서는 덮일 때까지 그대로 · 읽는 양 상한 64 KiB(`read_capped` 재사용) · 앞머리 UTF-8 BOM 은 무시한다(메모장 · PowerShell 이 붙인다) · `$version` 은 수 1 이어야 한다(`1.0` 도 1 · 없으면 1 로 본다).
- **통째로 못 쓰는 파일**(JSON 아님 · 객체 아님 · 상한 초과 · UTF-8 아님 · 모르는 `$version`): 메모리는 기본값, **적재는 파일을 건드리지 않는다.** 그 위의 첫 쓰기가 원본을 **고정 이름 하나 `settings.json.corrupt`** 로 **떠 둔 뒤** 그 자리를 원자적으로 갈아끼운다 — 파일이 없는 순간이 없다. 다음에 다시 깨지면 **그 사본을 덮어쓴다.** 크기 한도 · 중복 생략 · 시각/번호 이름은 없다(사용자 결정 F21 2026-10-02 — Chromium `Preferences.bad` · Firefox `Invalidprefs.js` 관행 · ADR-0274). 사본은 같은 폴더의 임시 파일에 흘려 담고 `sync_all` 뒤 rename 으로 갈아끼운다 — 반쯤 쓴 사본이 앞선 사본을 덮지 않는다(`fsutil::copy_atomic` · 원본 열기도 잠김이면 rename 과 같은 규칙으로 다시 한다). 떠 두기가 실패하면 그 쓰기는 `INTERNAL` 이고 원본도 앞선 사본도 그대로다. ADR-0166 결정 9 「`.corrupt` 없음」 번복 → ADR-0265 · 이름 · 덮어쓰기 규칙 → ADR-0274(ADR-0265 결정 4 부분 폐기). ★읽기 자체의 IO 실패는 이 무리가 아니다★ — 잠깐 잠긴 멀쩡한 파일을 덮지 않도록 그때 쓰기는 실패한다(§5-2).
- **실행 중 파일을 지워도 초기화가 아니다(세션 기본값 — §10 F20).** 파일이 없으면 쓰기가 메모리에서 다시 지으므로 다음에 파일을 쓰는 호출이 화면에 보이는 값 그대로 파일을 되살린다. 반면 멀쩡한 파일에서 손으로 지운 키는 되살리지 않는다 — 그 편집이 다음 부팅에 이긴다.
- **서비스 수명:** 빌드 전에 **읽기 전용으로** 적재해 manage 한다(웹뷰 첫 invoke 전에 존재 — ADR-0102). 적재는 로거보다 먼저 돌아 로그를 내지 않고 모아 두며, `setup` 의 `enable_writes` 가 쓰기를 켜면서 그 로그를 적재 때의 수준 그대로 한 번 낸다. 쓰기는 그 뒤에만(그 전의 쓰기 = `INTERNAL`). 첫 부팅의 파일은 없다 — 옛 `ui-settings.json` 에서 채우지 않는다(§3-5).

### 5-4. 명령 넷 — 기존 버스, 셸 표 [고름]

셸 표는 하나다(`layout/commands.rs` 「레이아웃 밖의 셸 명령도 여기 선다」 · 「모듈 하나에 블록 하나」). 그 블록에 더하고 `catalog_version` 을 올린다(P2a 가 9 → 10 · P3c1 이 `restore.*` 로 10 → 11 · P3c2 가 `restore.status` 의 답 모양(`durable` · `state_file`)으로 11 → 12 · 후속 `125d190` 이 같은 답의 `saves` 로 12 → 13 · P3d 가 `window.setTheme` · `window.getTheme` 를 들이고 `ui.refresh` 를 빼 13 → 14 — 정본 = 그 파일의 `catalog_version` 과 세대 주석).

| 명령 | effect | 인자 → 답 |
|---|---|---|
| `settings.get` | Read | `{key?}` → `{rev, items:[{key, value, is_default}]}` |
| `settings.set` | Write | `{key, value}` → `{rev, key, value, changed}` |
| `settings.reset` | Write | `{key}` → `{rev, reset:[key…]}` |
| `settings.schema` | Read | `{key?}` → `{items:[{key, kind, default, choices, min, max, description}]}` |

- `key` = 정확한 키 또는 점으로 끝나는 접두. `reset` 은 키 필수(전체 초기화 없음).
- **사람 경로도 같은 서비스:** `#[tauri::command] settings_get/set/reset/schema` 껍데기(ADR-0081 결정 3 「두 껍데기, 서비스 하나」). 프론트는 이 껍데기를 쓴다.
- **셸이 떠 있어야 닿는다**(셸 표의 다른 명령 — `window.setTheme` · `restore.*` — 과 같은 성질 · 옛 `ui.refresh` 도 그랬다 · §12 R9).

### 5-5. 변경 알림 · 프론트

- 쓰기 성공 → 상태 락을 놓은 **뒤**, 알림 순서 락 아래서 `settings:changed {rev, items}` 를 **모든 웹뷰**에 — 알림 순서 = `rev` 순서. `rev` = 유효 값이 바뀐 쓰기마다 오르는 단조 번호.
- 프론트 `src/api/settingsClient.ts`(신설): 구독 먼저 → `settings_get` 당기기(`theme/uiSettings.ts` 의 순서 조항) · **적용은 키마다 가린다** — 항목은 그 키에 마지막으로 적용한 `rev` 보다 클 때만 적용하고, 처음 보는 키는 그대로 적용한다(`settings_get` 답도 항목마다 답의 `rev` 로). 답을 통째로 버리면 그 답에만 있는 키를 잃는다.
- `chatStyleStore` 는 **적용자**가 된다: `chat.style.*` 를 CSS 변수에 붙인다(`CSS_VAR_BY_KEY` 화이트리스트는 프론트 소유). 쓰기 액션은 `settings_set`. localStorage 읽기 · 쓰기는 걷는다. 개발 손잡이 `window.__engram.chatStyle` 은 걷는다(ADR-0169 「남은 갭」 해소). ADR-0035 의 프론트 전용 예외 목록(`docs/reference/architecture-overview.md:578`)에서 `chatStyleStore` 를 뺀다.
- **첫 페인트:** 비동기(§10 F6). `main.tsx:12` 의 동기 적재는 설정 클라이언트 설치로 바뀐다.

### 5-6. 테마 — 전역은 설정, 창별은 상태

- **유효 테마(창 W) = W 의 창 테마 ?? `theme.default`.** 셸이 계산하고 기존 창별 배달(`get_ui_settings` 당기기 + `ui:settings-updated` 밀기)을 그대로 쓴다 [고름]. ✅ P3d 정본 = `src-tauri/src/theme.rs`(신설) — 유효 값 `WindowTheme::resolve` · 계산 `EffectiveThemes`(출처 = 모델: 트리 창 `agent-tree` = `TreeAttrs` · 그 밖 = `ViewManager`) · 당기기 껍데기 = `commands/settings.rs` 의 `get_ui_settings` · 창 포트 = 같은 파일의 `TauriThemeWindows`(옛 자리 `commands/settings.rs:48` · `ui_settings.rs:730` 은 TRD 초안 `c93e664` 기준이다 — `git show c93e664:<경로>` 로 연다). `source` 칸은 셸이 그 파일을 놓는 P3d 에서 뺀다(파일 자체는 지우지 않는다 — §3-5) — ✅ P3d: 프론트로 가는 짐은 `{theme}` 하나이고 wire 이름 둘(`get_ui_settings` · `ui:settings-updated`)은 그대로다.
- **밀기는 한 자리에서만 계산한다:** `theme.default` 쓰기와 `window.setTheme` 쓰기는 **각자 저장을 끝낸 뒤** 같은 함수 `push_effective_themes()` 를 부르고, 그 함수가 **테마 관문 락**(가장 바깥 — 쥔 채 설정 상태 락(`state`)을 짧게 읽고 놓고 → `ViewManager`·트리 칸 락을 짧게 읽고 놓고 → 창마다 emit) 아래서 모든 창의 유효 값을 새로 계산해 민다. 마지막 밀기 = 마지막 읽기라 두 쓰기가 엇갈려도 낡은 조합이 화면에 남지 않는다(✅ P3d 정본 = `theme.rs` 의 `EffectiveThemes::push_effective_themes` — 관문 = 그 타입의 `gate` · 관문 아래서 설정 상태 · 창 명단 · 트리 칸 · `ViewManager` 를 하나씩 짧게 잡고 놓으며 보낼 때는 관문만 쥔다 · 이 판이 본보기로 든 옛 `TauriUiSettings::refresh` 의 `gate`(`c93e664` 기준 `commands/settings.rs:109`)는 P3d 에서 걷혔다). **미는 자리 넷(P3d)** = `window.setTheme` · `theme.default` 쓰기와 되돌리기(버스 · Tauri) · 부팅의 창 복원 뒤(§6-5 ⑧ ⑨) · 런타임 복원 수락(§6-7 ③ 뒤 · ④ 앞).
- **바꾼 테마가 저장된다(ADR-0167 「화면 변경 미저장」 번복):** 전역 = `settings.set theme.default <값>` · 창별 = 셸 명령 **`window.setTheme {window, theme}`**(`theme:null` = 덮어쓰기 해제 — 매크로 `Option<Option<T>>`, `macros.rs:43`) · **`window.getTheme {window}`** → `{theme, effective}`. 창별 값은 그 창의 모델 항목에 들어가 **창과 같이 죽는다**(§6-3) — 부팅 쓸기가 필요 없어진다.
- **창별 테마가 「설정 명령 넷」 밖의 다섯째 길인 이유:** 그 값은 설정이 아니라 상태이고(결정), 상태 파일은 기계가 통째로 다시 쓰므로 손 편집 경로가 될 수 없다.
- **`window.setTheme` 의 배달 실패와 답(사용자 결정 2026-10-07):** **지목한 창이 테마를 못 받았을 때만** 오류다 — 문구는 그 창의 실패를 자세히 싣고 나머지 실패는 수로 접는다. 다른 창만 못 받았으면 성공이고 warn 로그만 남는다. 이 동작은 사용자가 정했고(2026-10-07), 그 결과는 **ADR-0166 결정 6(알림이 못 나가면 성공으로 답하지 않는다)의 개정**으로 박혔다 — 결정 6 은 `window.setTheme` 에서는 지목한 창에만 걸린다(`theme.default` 쓰기에 걸지는 열려 있다 — TRD §5-6)(개정 도장 형식 = 사용자 선택 2026-10-07 · 새 ADR 없음). 구현(`theme.rs` 의 `ThemeControl::set` · `ThemeError::Undelivered` — 사용자 결정으로 적지 않는 구현 사양): 그 오류 = `INTERNAL`(「정했지만 그 창에 못 보냈다」 — 쓴 값은 되돌리지 않아 `window.getTheme` 이 새 값을 답한다) · 지목한 창이 살아 있는 웹뷰가 아니면 성공(뜰 때 당겨 간다 — ADR-0166 「알려진 잔여」의 「구독자 0 에 닿아도 성공」과 같은 성질) · 모르는 창 = `CONFLICT` · 못 쓸 테마 = `INVALID_ARGUMENT`(테마 이름은 대소문자 무시 · CLI 의 해제 낱말 `none` 은 소문자만) · 배달 실패 로그는 창 셋을 넘으면 수로 접는다(`Undelivered` · `FailedWindow`).
  - **닫힘 — 사용자 결정 2026-10-08:** ADR-0166 결정 6 을 `settings.set` · `settings.reset` 의 `theme.default` 에도 걸지 — 지금 그 길은 배달이 실패해도 성공으로 답한다(버스 · Tauri 둘 다). → **걸지 않는다 — 성공 답 유지.** 값은 파일에 남고 못 받은 창은 다음 갱신 · 재기동에 맞춰진다 · 배달 실패는 그 순간 닫히는 중이거나 죽은 창뿐이라 드물다 · 특정 창을 지목하지 않은 전역 쓰기라 `window.setTheme` 의 「다른 창만 못 받으면 성공」과 같은 모양(§14-19). 코드 변경 없음.
- **오늘 화면에는 테마를 바꾸는 UI 가 없다**(ADR-0167 결정 5 · `themeManager.apply` 호출부 = `main.tsx` · `uiSettings.ts` 뿐 — P3d 작업 트리 기준 `main.tsx:24` · `uiSettings.ts:93,128` · TRD 초안 `c93e664` 기준 = `main.tsx:20` · `uiSettings.ts:95,130`). ★P3d 에도 두지 않는다 — 명령만(사용자 결정 2026-10-06 · §10 F7 = (a))★.

### 5-7. 파일 편집 경로 폐지 · `ui.refresh` · 도움말

- `ui.refresh`(옛 `layout/commands.rs:362` — `c93e664` 기준)는 **P3d 에서 지운다**. L2 동안은 창별 칸만 다시 읽는 일로 남는다(L2 단독 머지 때 창별 제어가 끊기지 않게). ✅ P3d `c85b2ac` — 선언 · 동사 · `ThemeOrigin` · `UiRefreshOk` · `LayoutPorts.ui_settings` · 포트 `UiSettingsRefresh` 째 걷었다(§14-19).
- `prompts/engram-help.md` 구획 규칙(`:8`)이 정확히 다섯(root · mail · agent · window · theme)을 요구하고 하나라도 빠지면 파일 전체가 거부된다 → **구획 개명은 파일과 CLI 를 같은 단계에서**: `theme` → `settings`(§10 F14) · `bin/engram.rs` 의 `HELP_TOPIC_THEME`(`:243`)·필수 구획 목록·`:4331` 시험 · `help theme` 별칭 · `root` 목록 줄 · `prompts/agent-priming.md:12`. 새 바이너리 + 옛 파일 = 내장 사본(`:231` `include_str!`)이 나간다.
- 도움말 본문은 P2c(명령 넷 · 창별은 아직 파일 + `ui.refresh`) → P3c(`restore.*` · ~~슬롯 정지 상태의 `agent.spawn`~~ — ★걷음: 슬롯 활성화 단추를 두지 않아(ADR-0280) 도움말에 슬롯 쪽 활성화를 적을 것이 없다★) → P3d(`window.setTheme`/`getTheme` · `ui.refresh` 삭제)에서 고친다. ✅ P3d — `prompts/engram-help.md` 의 settings 구획을 다시 썼고 옛 `## theme` 별칭 자리는 남겼다.

## 6. 화면 상태 (`shell\state\state.json`)

### 6-0. 저장 범위 — 남기는 것 · 버리는 것 (사용자 결정 2026-10-02)

조사에서 화면에 보이는 사실 대부분이 디스크에 남지 않고 있었다. 사용자가 항목마다 남길지를 정했다(아래 두 목록).

- **가름 규칙:** config = 사람이 정한 취향(지우면 기본값) → `settings.json`(§5) · state = 프로그램이 기록한 것(「무엇이 있나 · 어땠나」) → `state.json`(§6-1). ★**사람이 고른 값이라도 창 · 탭 · 슬롯 하나에 딸려 그것과 같이 죽는 값은 state 다**★ — **도출**(사용자 결정이 아니다): 조사 「0. 결정」 17행(창별 테마 = `state.json` 의 그 창 항목) · 19행(슬롯에 꽂힌 내용 = `state.json`) · ADR-0264 결정 3(가름 기준)에서 낸 것이다. 아래 탭 이름 · 사용량 체크를 이 규칙으로 state 에 넣었다(보기 모드는 뺐다 — §14-20).

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
| ~~**슬롯 보기 모드(터미널 / 챗)**~~ | ~~state(슬롯 값)~~ → **남기지 않는다(사용자 결정 2026-10-07 · §14-20)** | `viewStore.renderModeOverride`(`src/store/viewStore.ts:96`) — **프론트 메모리뿐 · 창(웹뷰)마다 따로** — 그대로 둔다 | ~~**P3e**~~ 뺐다(T-51) |

**남기지 않는 것** — 도구 묶음 펼침 · 생각 줄 펼침 · 대기 목록 펼침 · 스크롤 고정 · 목록 선택 · 입력 초안 · 트레이로 숨긴 상태. 영속 경로를 만들지 않고 다음 부팅엔 기본값이다. 트레이 숨김은 다음 부팅이 `--hidden` 인자만 따른다(§4 덤 · §6-5 ⑨). 이 밖에 이미 뺀 것 = 화면 실측값 · 에이전트 런타임(§6-1) · 트리 창 보임(F9).

**슬롯 보기 모드 — 프론트 메모리에서 셸 소유 슬롯 값으로 (P3e)** — ★**수행하지 않는다 — P3e 를 뺐다(사용자 결정 2026-10-07 · §14-20)**★

> 사용자 사유: 「json · 터미널에서 값을 주고 있는데 아직은 필요없어 보임」 — 기본 보기는 매번 에이전트 출력 형식에서 정해지므로(JSON → 챗 · 그 밖 → 터미널) 영속할 것은 손으로 건 덮어쓰기뿐이고, 그것은 아직 필요 없다. 덮어쓰기는 지금처럼 프론트 메모리뿐이다(창마다 따로 · 재시작하면 사라진다). **아래는 2026-10-02 판의 설계 메모를 되살릴 때를 위해 남긴 것이다(T-51)** — 「바뀌는 것」 · 「P3 착수 전에 설계한다」는 어느 것도 하지 않았고 지금 계약이 그대로 선다.

- **지금 계약:** 프론트 전용 덮어쓰기다 — 결정 = ADR-0056(`:12` · `:29` 「프론트 전용 override 를 레버로 노출」) · ADR-0035 의 프론트 전용 예외 목록(`docs/reference/architecture-overview.md:578`). 권위 루프 밖에서 낙관적으로 쓰고(`viewStore.ts:89-95` JSDoc), 프론트가 그 칸을 지우는 자리가 다섯이다(`viewStore.ts:210-245` — 닫기 · 에이전트 배정 · 내용 교체 · 사용량 슬롯 설정 · **다른 창으로 옮기기**). 창(웹뷰)마다 따로라 버스에 올리지 않는다(근거 = `src/commands/renderModeCommands.ts:1-18` 머리 주석). 칸이 없으면 caps 에서 낸 기본값이다(`LayoutLeaf.tsx:159`).
- ★**에이전트 출력 형식과 다른 것이다**★ — 남기는 것은 슬롯의 **렌더러 덮어쓰기**다. 에이전트의 `output_format`(생성 때 정해 고정 — ADR-0078)은 건드리지 않는다.
- **바뀌는 것:** 셸이 슬롯 값으로 소유하고 invoke → emit 으로 쓴다. `state.json` 에는 `layout` 안의 그 슬롯과 함께 실린다(§6-1). 남길 값의 집합 = **§10 F19 ★명시 확인★**(기본 (a) 셋 그대로 — `terminal` · `rich`(챗) · `dom`, `src/components/slot/renderMode.ts:5`). P3e 착수 전에 답을 받는다(답 = (a) · 2026-10-07 — 같은 날 P3e 를 빼서 무효가 됐다 · §10 F19).
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
    { "id": "agent-tree", "kind": "tree", "theme": null, "bounds": null, "maximized": false },
    { "id": "<uuid>", "kind": "popout", "theme": null, "bounds": {…}, "maximized": false, "active_tab": "…", "tabs": [ … ] } ] }
```

- `bounds` = 마지막 보통 자리 `{x, y, w, h}`(`x` · `y` = 물리 바깥 위치 · `w` · `h` = 논리 안쪽 크기 — §6-3) **또는 `null`** = 그 창의 보통 자리를 본 적이 없다(만들 때부터 최대화 · 첫 기록 전) → 복원은 기본 자리로 연다. ★자리가 없다고 창을 빼지 않는다★ — main 이면 그 탭이 다음 부팅에 사라진다(`state/convert.rs` `to_persisted`). 읽기: 칸이 없어도 `null` 과 같다 · 객체 안 칸 하나라도 수가 아니면 그 창의 읽기 실패(건너뜀). 쓰기는 유한한 값만 싣는다 — 메모리 `WindowBounds` 가 유한하고 크기가 0 보다 큰 값만 쥔다(유한하지 않은 실수는 JSON 에 `null` 로 나가 그 창을 못 읽게 만든다). `maximized` 가 없으면 그 창의 읽기 실패다. 트리 창의 `maximized` 는 늘 `false` 다(F9 — §6-3).
- `layout` = 기존 `LayoutNode` serde(`#[serde(tag="type", rename_all="snake_case")]`) · 슬롯 `content` = 기존 `SlotContent` serde. 팝아웃 `id` = 창이 처음 생길 때 뽑는 UUID(§6-3) — label 이 아니다.
- `clean_exit` = **실행 표식**(Chromium `exit_type` 과 같은 수법 — 부팅 단계의 첫 쓰기가 `false` 로 세우고(어느 창보다 먼저) 정상 종료 쓰기가 `true` 로 내린다 · §6-5 ⑤ · §6-6). `saved_at_ms` 는 `restore.status` 가 싣는다(§6-7).
- `resolved_crash_copy` = **답한 사본의 해시**(I2) — 답(수락 · 거절) 뒤 첫 쓰기부터 사본 삭제가 성공할 때까지만 싣는다(§6-4 · §6-7). 부팅은 해시가 같은 사본을 「답함」으로 보고 묻지 않고 지운다. 해시 = 64비트 FNV-1a · 16진 문자열 [고름 — 몇 줄 직접 구현: std `DefaultHasher` 는 판마다 같은 값을 약속하지 않아 디스크에 못 싣고, 신원 확인이지 보안이 아니라 새 의존성이 필요 없다].
- ~~`seq`~~ **5판에서 뺐다** [고름]. 쓰던 곳 셋이 모두 다른 수단으로 갔다 — ① 대기분 삭제 확인 → 기록기 안의 깃발 하나(§6-4 — 쓰는 쪽이 기록기 하나이고 커밋이 요청보다 먼저라, 요청을 받은 뒤 뜬 스냅숏은 커밋을 담는다) ② 고유 임시 이름 `tmp-<seq>` → `fsutil` 의 `<이름>.tmp<pid>.<n>`(pid + 프로세스 안 번호 — P3a · §5-3) ③ 부팅 지문 → 6판에서 지문 자체가 없어졌다(셸 실행 잠금 뒤에 읽는다 — §6-5 ①). 읽는 곳이 남지 않았다.
- ~~「기본 모양」 판정(`is_default_shape`)~~ **6판에서 뺐다(사용자 결정 D5)** — 비정상 종료 뒤엔 모양과 무관하게 늘 묻는다. 사용자는 그 비교 로직을 필요 없는 복잡도로 보았다.
- ~~**슬롯 보기 모드**는 P3e 부터 셸 소유 슬롯 값이라 `layout` 안의 그 슬롯과 함께 실린다(자리 = §6-0 — P3 착수 전 설계). L3 는 한 덩어리로 머지되므로(§9-1) v1 이 나가기 전에 칸이 더해진다 — 버전을 올리지 않는다.~~ → **싣지 않는다 — P3e 를 뺐다(사용자 결정 2026-10-07 · §14-20).** v1 에 보기 모드 칸은 없다. 되살리면(T-51) 아래 「칸 더하기 규칙」을 따른다.
- **칸 더하기 규칙(세션 판단(사용자 위임) 2026-10-03 · 추천안 (a)):** 빠져도 읽히는 선택 칸을 더하는 것은 `version` 을 올리지 않는다 — 옛 빌드가 다시 저장하면 그 칸은 사라진다(내림은 지원하지 않는다 — §12 R3). 잃으면 안 되는 변경이면 `version` 을 올린다(옛 빌드는 그 파일을 버전 초과로 접는다 — §6-2). ★단 올릴 땐 앞 버전 리더를 함께 넣는다★ — 코덱은 `version == STATE_VERSION` 만 받으므로 리더 없이 올리면 새 빌드가 앞 버전 파일을 `NotStateFile` 로 접어 저장된 화면 전체를 잃는다(아래 「이름 바꾸기 규칙」과 같은 이유 · 2026-10-04 문서 리뷰 · 도출). 거부: **모르는 칸을 메모리 · IPC 타입에 실어 보존** — `WindowTabs` · `View` · IPC 타입을 바꿔야 하고, 이 문서가 모르는 것을 메모리 타입에 싣는 길을 이미 거부했다(§6-2 · §11-4 「메모리 `SlotContent::Unknown`」). 코드 쪽 정본 = `state/schema.rs` 머리(「앞 버전 리더를 함께」 단서와 아래 이름 바꾸기 규칙까지 싣는다 — P3c1 이 맞췄다).
- **이름 바꾸기 규칙(2026-10-04 문서 리뷰 · 도출 — 사용자 결정 아님):** 영속 모양이 따르는 serde 이름(변형 · 칸 · `type` 꼬리표 — serde 를 그대로 재사용하는 `SplitDir` · `SlotContent`, `state/schema.rs` 시험이 같은 JSON 으로 묶는 `LayoutNode` · `View`(영속 쪽 `PersistedNode` · `TabEntry`))을 바꾸면 옛 이름이 실린 파일이 깨진다 — 노드 꼴 · 분할 방향이 안 풀리는 탭은 통째로 건너뛰고(§6-2), 슬롯 내용 종류가 안 풀리면 그 슬롯은 모르는 내용으로 서며, `#[serde(default)]` 칸(예: `Usage` 슬롯의 `show_claude` · `show_codex`)은 모르는 내용이 되지 않고 **값이 말없이 기본값(`true`)으로 돌아간다**(옛 키는 무시된다). ★**그래서 칸 · 변형 이름을 바꿀 때는 읽는 쪽에 옛 이름을 `#[serde(alias = "옛 이름")]` 로 남긴다 — 읽기만 보면 `version` 을 올릴 필요가 없다**★ ★**단 꼬리표 키 자체(`tag = "type"` 의 `type`)는 alias 가 덮지 않는다**★(serde 의 alias 는 칸 · 변형 이름에만 붙는다) — 그 키는 바꾸지 않거나, 두 키를 다 읽는 리더와 함께 바꾼다 [미검 — 내부 꼬리표 enum(`tag = "type"`) 변형의 alias 가 먹는지는 구현 때 시험으로 확인]. ★**`version` 만 올리지 말 것**★ — 코덱은 `version == STATE_VERSION` 인 파일만 받아(`state/codec.rs` 의 `decode`) 낮은 번호를 상태 파일 아님으로 접는다 → `.corrupt` 로 떠 두고 기본 화면이라, 올린 판으로 올라가는 순간 저장된 화면 **전체**를 잃는다(옛 이름 탭만 건너뛰는 것보다 나쁘다). 올린 번호가 돕는 것은 지원하지 않는 내림뿐이다(§12 R3). `version` 은 앞 판을 읽는 리더와 함께만 올린다.
- 넣지 않는 것: 화면 실측값(`canvas`·`metrics` — `manager.rs:103-105`) · 에이전트 런타임 · §6-0 「남기지 않는 것」 전부.

### 6-2. 메모리 타입과의 대응 · 모르는 것

- 영속 DTO 는 `state/schema.rs` 에 따로 둔다 — `WindowTabs` 는 serde 가 없고(`manager.rs:98-107`) `ViewSnapshot` 은 파생값을 싣는다. `View`·`LayoutNode`·`SlotContent` 는 serde 를 재사용한다.
- **모르는 슬롯 내용(ADR-0060 요건) = 영속 DTO 에만 있다.** DTO 의 내용 칸 = `Known(SlotContent) | Unknown(원문 JSON 객체)`(전용 코덱 — 아는 변형으로 안 풀리면 원문을 통째로 쥔다). 메모리 `SlotContent` 와 IPC·버스 타입은 이 일로는 **바뀌지 않는다**(~~보기 모드 칸은 P3e 가 따로 더한다 — §6-0~~ → P3e 를 뺐다 · 사용자 결정 2026-10-07 · §14-20) — `set_slot_content`·`layout.setSlotContent` 는 모르는 종류를 지금처럼 역직렬화에서 거절하고, ts-rs 바인딩도 그대로다.
  - 복원 때 그 슬롯은 메모리에서 `Empty` 로 서고, 원문은 `ViewManager` 의 곁표(`unknown_content: slot_id → 원문`, 프론트로 안 나간다)에 남는다. **스냅숏은 그 슬롯이 아직 있고 내용이 바뀐 적 없을 때 원문을 그대로 되돌려 쓴다** — 다른 곳을 고치고 다시 저장해도 새 버전이 쓴 내용이 지워지지 않는다. 그 슬롯에 내용을 넣거나(`Empty` 포함) 닫으면 곁표 항목이 지워진다 — ★P3b1 착지: 거두는 자리는 `ViewManager` 의 내용 쓰기 문 `write_slot_content` · 뷰 지우기 문 `remove_view` · `close_slot` 셋뿐이다(`manager.rs` 머리 「모르는 슬롯 내용 곁표」)★ · `tree::assign_in_tree` 는 걷었고 트리의 내용 쓰기는 `tree::set_in_tree` 하나다. 곁표는 뷰로 먼저 갈라 탭이 다른 창으로 가도 따라가고, 슬롯을 새 창으로 빼내면 원문도 따라간다(`prepare_detached_view` — 옮기기는 교체가 아니다).
  - **그 슬롯은 「점유」다**(ADR-0059 의 빈/점유 판정 — 지금은 `SlotContent::is_empty`, `layout/types.rs:37,62`). 자동 배치가 그 원문을 덮지 않게 점유 판정을 `ViewManager::slot_is_free(view, slot)` = `is_empty() && 곁표에 없음` 하나로 모으고, `resolve_spawn_slot`(P3b1 에서 `ViewManager` 메서드가 됐다 — `manager.rs:521`, `f9db35a` 기준)이 그것을 쓴다. `tree::first_empty_slot_id` 는 트리만 봐서 곁표 슬롯을 비었다고 답하므로 자동 배치에 쓰지 않는다(`tree.rs` 의 그 함수 주석). 프론트의 같은 판정(`selectOpenTarget.ts` 의 `firstEmptySlotId` · 포커스 대상)은 `ViewSnapshot` 의 새 칸 `foreign_slots: Vec<slot id>` 로 같은 답을 낸다 — ✅ P3c3 `e904161`: `applyLayoutUpdated` 가 그 칸을 캐시(`CachedView.foreignSlots`)에 싣고, `selectOpenTarget` 은 그 슬롯을 점유로 보며 포커스된 그 슬롯도 돌려주지 않는다(트리 「열기」는 그 칸 대신 빈 슬롯에 놓는다 — §14-18 GUI). `SlotContent` 와 IPC 타입은 그대로다.
  - 화면에는 ~~「이 버전이 모르는 내용」 자리표시와 안내 한 줄(세션 선택 — ADR-0280 결정 6)~~(원문 = `8f1a00a`) → 아이콘 자리표시 하나(글 · 판 구분 · 안내 줄 · 막 없음 · 이름 「알 수 없는 내용」 — 사용자 결정 2026-10-06 · ADR-0280 결정 7 · 아이콘 `FileQuestionMark` · hover 툴팁 · `data-slot-foreign` 은 세션 판단 — 결정 6)가 서고(빈 슬롯 메뉴로 다른 내용을 **명시적으로** 놓으면 원문은 사라진다), warn 한 줄. ★그 칸의 메뉴에 「비우기」는 없다 — 새 내용으로 덮어쓰거나 슬롯을 닫는다(사용자 결정 2026-10-06 · ADR-0280 결정 4)★ · ~~열린 것(ADR-0280)~~ → 닫았다(2026-10-06 · ADR-0280 결정 7).
- **모르는 창 종류 · 못 읽는 탭:** 그 창/탭만 건너뛰고 warn.
- **못 쓸 파일** = `version` 이 아는 것보다 큼 · JSON 아님 · 상태 파일 모양 아님(머리를 못 읽음 · `version` 이 없거나 못 쓸 값) · UTF-8 아님 · 읽기 상한 **4 MiB** 초과(I4) → 기본 화면으로 시작하고 묻지 않는다. `version` 은 정수 값인 실수(`1.0`)도 그 정수로 읽는다(설정 `$version` 과 같은 관용 — §5-3) · 음수 · 소수 · `u64` 범위 밖 · 수가 아닌 값은 상태 파일이 아니다. 원본은 부팅 단계(§6-5 ④)가 고정 이름 `state.json.corrupt` 로 **떠 둔 뒤** 첫 쓰기가 그 자리를 갈아끼운다 — §5-3 과 같은 떠 두기(`fsutil::copy_aside` · 앞선 사본을 덮어쓴다 · 크기 한도 없음 — ADR-0274). 못 쓸 크래시 사본 `state.crash.json` 도 같은 규칙으로 `state.crash.json.corrupt` 에 떠 둔 뒤 지운다(§6-5 표). 떠 두기가 실패하면 log 하고 진행한다(D8) — 못 쓸 사본은 그때 지우지 않는다(§6-5 ③).
- ★**읽기 IO 실패는 못 쓸 파일이 아니다(I3)**★ — 잠김 오류(Windows 5 · 32 · 33)면 20 ms 간격으로 5번까지 다시 읽는다(쓰기 재시도와 같은 한도 — §5-3). 그래도 실패하면 log 하고 그 파일을 **없음**으로 친다 — ★**그 파일에 대한 동작(떠 두기 · 지우기 · 그 자리를 덮기)만 하지 않는다 · 다른 파일의 동작은 그 파일의 읽기대로 간다**★(파일마다 — 세션 판단(사용자 위임) 2026-10-04 · 사용자 확인 2026-10-05 · §6-5 ③). `state.json` 이 그러면 가드다 — ⑤ 건너뜀 · 기록기 없음 · 그 실행은 상태를 저장하지 않는다(§6-5 ③ · N3 · §12 R16) · 사람에게는 안내만 한다(§6-5 ③ 가드 항목 — 사용자 결정 2026-10-05). 사본이 그러면 그 사본으로는 묻지 않고, 그 때문에 떠야 할 사본을 못 뜨면 같은 가드다.
- **`ViewManager::from_persisted`**(P3b1 착지 — `state/convert.rs` 의 `impl ViewManager` · 모델을 세우는 `from_restored` 는 `manager.rs`): 순수하다(파일 · 창 · 락 0 · 실패 없음). 코덱이 모양 검사를 하지 않으므로 불변식 1–4(`manager.rs` 머리 — 양방향 소유 · 유니크 소유 · `active ∈ tabs` · main 최소 1탭)와 창 묶음의 모양을 여기서 다시 세운다. 고치거나 건너뛴 것은 `RestoreWarning` 으로 돌려주고 로그는 부르는 쪽(부팅 단계 — `Internal` 만 error · 나머지 warn)이 낸다. 아래 목록의 정본 = `state/convert.rs` 의 `RestoreWarning`:
  - **창** — id 와 종류가 어긋나면(main ≠ `main` · 트리 ≠ `agent-tree` · 팝아웃 id 가 UUID 아님) 그 창을 건너뛴다 · 같은 창 id 가 겹치면 뒤엣것을 건너뛴다(같은 UUID 의 다른 철자 — 대문자 · 중괄호 — 도 같은 창이다: 정규 철자로 적고 견준다) · 탭 없는 팝아웃은 창째 건너뛴다(label 을 받지 않는다) · 발급기가 준 label 이 이미 쓰였으면(`main` · `agent-tree` · 앞 팝아웃) 그 팝아웃을 건너뛴다 · main 항목이 없거나 탭이 하나도 안 남으면 빈 탭 하나(`View 1`)로 시작한다.
  - **탭** — 같은 view id(창 안이나 앞선 창과)면 뒤 탭을 건너뛴다 · 슬롯 · 분할 id 가 탭 안이나 앞선 탭과 겹치면 그 탭을 건너뛴다 · 활성 탭이 남은 탭에 없으면 첫 탭 · 포커스가 없거나 트리에 없는 슬롯을 가리키면 첫 슬롯(`fixup_focus` 와 같은 규칙 — 경고 없음).
  - **칸** — 범위 밖 분할 비율은 쓰기 경로(`set_split_ratio`)와 같은 범위로 자르고 유한하지 않으면 기본 비율 · 쓸 수 없는 자리(`WindowBounds::new` 가 거절 — 유한하지 않음 · 크기 0 이하)는 그 칸만 버리고 기본 자리 · 모르는 슬롯 내용은 빈 칸 + 곁표 원문.
  - ★**버려지는 창은 아무 id 도 쥐지 않는다**★ — 창 · 탭 · 노드 id 와 경고 · 자리는 그 창을 남기기로 정한 뒤에만 싣는다. 버린 창이 쥔 id 에 뒤 창(main 포함)의 탭이 겹친다고 밀려나면 살릴 수 있던 내용을 잃는다. 버린 창은 버린 이유만 남긴다.
  - 고친 입력으로도 모델을 못 세우면(이 모듈의 결함) 기본 화면으로 시작한다(`Internal`).
  - ★**결과의 `version` · `attrs_rev` 는 의미가 없다(0 근처)**★ — 부팅은 그대로 쓴다(기록기가 실행 표식 쓰기 때의 번호로 시작한다). 런타임 수락이 이 결과를 다루는 규칙은 §6-7 ③.

### 6-3. 창 신원 · 창 속성의 자리

- **영속 신원 = 창 id**(`main` · `agent-tree` 고정 · 팝아웃 = 창이 처음 생길 때 뽑는 UUID, `WindowTabs.window_id`). **runtime label 은 한 부팅 안에서만의 신원**이다 — 복원되는 팝아웃은 부팅이든 런타임 수락이든 `PopupCounter`(`popout.rs:47`, `f9db35a` 기준)에서 **새 label** 을 받는다. 그래야 런타임 복원이 지금 떠 있는 팝아웃의 label 과 부딪히지 않는다.
- **창별 테마·위치는 `WindowTabs.attrs` 묶음에 산다**(`WindowAttrs` — `theme: Option<UiTheme>`(`None` = 전역 테마를 따른다) · `bounds: Option<WindowBounds>`(마지막 보통 자리 · `None` = 아직 못 봤다) · `maximized`). 창 항목과 같이 죽고 락은 `ViewManager` 하나다. ADR-0167 이 경고한 오적용(창 밖에 label 키로 값을 둔 것)이 구조적으로 사라진다. 바꾸는 길은 setter 로만 둔다(`set_window_theme` · `observe_window_placement` — 규약이다: 칸이 `pub` 이라 컴파일러는 막지 않는다). setter 는 **값이 바뀌었을 때만** `bump_version`(`manager.rs:565`, `f9db35a` 기준 — 프론트가 낡은 알림을 거르는 레이아웃 번호)이 아니라 **`attrs_rev`** 를 올린다 — 창을 끌 때마다 레이아웃 번호가 튀지 않게 하고, 기록기는 둘 다 본다(§6-4). ★`attrs_rev` 는 `ViewManager` 의 비공개 칸이다(읽기 = `attrs_rev()`)★ — 밖에서 `attrs` 를 직접 고치면 번호가 안 올라 기록기가 그 변경을 놓친다.
- **트리 창(`agent-tree`)은 모델 밖이다**(`manager.rs` 헤더 「`agent-tree` 창은 이 모델 밖」). 그래서 `state/tree_attrs.rs` 의 `TreeAttrs` 가 따로 든다 — 칸은 `WindowAttrs` 를 쓰되 ★**테마 · 위치만 싣는다(F9 (a) — 세션 판단(사용자 위임) 2026-10-03)**★: 최대화 표식은 `set` 과 자리 기록 뒤에 늘 내리고(숨은 창에 최대화를 입히면 창이 보인다 — tao 의 최대화 = `SW_MAXIMIZE`), 보임은 복원하지 않는다(부팅마다 숨은 채 뜬다). 자기 락(잎 — 쥔 채 다른 락을 잡지 않고 OS 를 부르지 않는다)과 자기 변경 번호를 갖고, 칸을 통째로 갈아끼울 때(부팅 채우기 · 런타임 수락)도 `TreeAttrs::set` 으로 번호를 올린다 — 되돌리거나 초기화하지 않는다. `window.setTheme` 은 `agent-tree` 면 이쪽, 그 밖은 `WindowTabs` 로 간다(✅ P3d — 가르는 자리 = `theme.rs` 의 `EffectiveThemes::set_window_theme` · 그 전까지 `TreeAttrs::set_theme` 은 부르는 쪽이 없었다).
- **위치·크기 기록:** `WindowEvent::Moved/Resized` 에서 **최소화도 최대화도 아닐 때만** `bounds` 를 갱신한다 — ★**위치 = 물리 바깥 위치(`outer_position` 그대로) · 크기 = 창의 배율로 나눈 논리 안쪽 크기**★. 위치를 논리로 두면 배율이 다른 모니터가 섞일 때(100% 오른쪽에 150% — 논리 x 1280..1920 띠) 한 값이 두 모니터를 가리켜 다른 모니터로 복원된다. 위치를 물리로 적는 것은 tauri-plugin-window-state 와 같다(그 플러그인은 크기도 물리로 적는다). 버릴지 판정은 물리 공간에서 — 모니터마다 저장된 위치 · 논리 크기 × 그 모니터 배율의 사각형이 그 모니터와 겹치는지 본다. 복원은 위치를 먼저, 크기를 뒤에 놓는다(크기가 도착한 모니터의 배율로 풀리게 — 정본 = `state/placement.rs`). 최대화 여부는 `maximized` 로 따로 — 복원은 마지막 보통 크기로 만든 뒤 최대화한다. **트리 창은 최대화를 싣지 않는다**(F9 — 테마 · 위치만). ★**최대화 직전 읽기를 되돌리는 한 칸 메모(`PlacementMemo` — 창 항목 · 트리 칸과 같이 살고 영속하지 않는다)**★: 사용자가 최대화하면 tao 가 `Moved` 를 최대화 표식이 서기 전에 내서 최대화된 사각형이 보통 자리로 한 번 적힌다 — 바로 앞 읽기가 보통으로 적은 자리가 지금(최대화) 읽은 자리와 **같을 때만** 그 읽기를 되돌린다(tauri-plugin-window-state 가 `Moved` 마다 앞 위치를 밀어 두는 것과 같은 생각 · 「같은 자리」 조건을 빼면 앞의 진짜 보통 읽기를 버린다 — 정본 = `WindowAttrs::observe`). **연 창의 첫 자리**는 그 창이 모델에 든 뒤 한 번 적는다(`WindowHost::record_placement` — 한 번도 끌지 않은 창도 다음 부팅에 그 자리로 연다). 모델에 없는 label 의 사건(닫힌 뒤 늦게 온 것 · 모델에 들기 전의 새 창)은 버린다 — 팝아웃 label 은 재사용되지 않아 다른 창에 잘못 적히지 않는다. ★**창 게터(`outer_position` · `inner_size` · `scale_factor` · `is_minimized` · `is_maximized`)는 `ViewManager` 락을 잡기 전에 전부 부르고, 락 안에서는 값만 적는다**★ — 레이아웃 락 보유 중 OS 호출 금지(`layout/apply.rs` 머리 「락 규율」).
- **락 순서(명시):** 테마 관문 락(§5-6) **›** 설정 상태 락(`state`) · `ViewManager` 락 · 트리 칸 락. 관문은 가장 바깥이고 나머지 셋은 **서로 겹쳐 잡지 않는다**(관문 아래서도 하나씩 짧게). 겹쳐야 할 날이 와도 `ViewManager` → 설정 순서만 허용하고 설정 락을 쥔 채 `ViewManager` 락을 잡는 것은 금지다. 기록기는 락을 쥐고 디스크를 만지지 않는다(복사 뒤 놓는다).

### 6-4. 저장 — 쓰는 쪽은 하나

- **기록기(`state/saver.rs`) 스레드 하나**가 부팅 단계(§6-5 ①–⑥ — 한 스레드 · 기록기보다 먼저) 뒤의 `state.json` **유일한 쓰는 쪽**이고 사본 `state.crash.json` 을 **지우는 유일한 쪽**이다 [고름]. 다른 스레드는 요청을 보내고 답을 기다릴 뿐이다. 쓰는 스레드가 하나라 쓰기 락·순번 비교가 필요 없다.
- **주기 저장:** 0.5초마다 `ViewManager.version`(레이아웃 변경마다 +1 · 측정 보고는 안 올린다 — `manager.rs:292,565,1101`, `f9db35a` 기준) · `attrs_rev`(§6-3) · 트리 칸 변경 번호를 짧게 읽고, 바뀌었으면 **마지막 변경 뒤 1초 조용하거나 첫 변경 뒤 5초**에 스냅숏을 **통째로 원자적으로** 다시 쓴다(D2) — 락 안에서 복사 → 락 밖에서 직렬화 → `fsutil` 원자 쓰기(임시 `state.json.tmp<pid>.<n>` → `sync_all` → rename · 잠김 재시도 — §5-3). 기록기는 부팅 첫 쓰기(§6-5 ⑤) 때의 변경 번호를 받아 시작한다 — 그 뒤 바뀐 것은 첫 주기가 싣는다.
- **스냅숏 원천 = 포트 `SnapshotSource`** — 변경 번호 `revision()` · 창 전부 `snapshot() -> Result<Vec<WindowEntry>, String>`(파일 머리는 기록기가 채운다). **변경 번호는 같은지만 본다(대소를 보지 않는다) — 되돌리거나 초기화하지 않는다**(마지막으로 본 값과 다시 같아지면 그 사이의 변경을 못 본다). **포트 실패 = 실패한 쓰기** — log(error) · 조용함 뒤 다시 뜬다(디스크 오류와 같다) · 그때 처리 중인 `Resolve` / `Final` 의 답은 `Failed`. 포트가 지킬 것 둘(정본 = `state/saver.rs` 의 포트 문서):
  - ★**패닉하지 않는다**★ — 릴리스는 `panic = "abort"` 라 기록기 스레드의 패닉이 앱을 통째로 죽인다. 독 든 락은 되살리거나 `Err` 를 돌려준다.
  - ★**`finish` · `resolve` 를 부른 스레드를 기다리지 않는다**★ — 그 스레드는 답을 기다리며 서 있다. 특히 Tauri 창 게터는 메인 스레드 밖에서 부르면 메인이 답할 때까지 서는데, `finish` 는 `RunEvent::Exit` 에서 바로 그 메인 스레드가 부른다 — 그러면 정상 종료가 매번 마감을 넘겨 비정상 종료로 읽힌다. 그래서 위치 · 크기는 `WindowEvent` 에서 모델에 적어 두고(§6-3 · P3b3) **`snapshot()` 안에서 창 게터를 부르지 않는다.** 운영 원천 = `LiveSource`(`state/boot_plugin.rs` — 트리 칸을 읽고 놓은 뒤 레이아웃 락을 짧게 잡아 `to_persisted` · 독 든 레이아웃 락이면 `Err` — 반쯤 바뀐 모델을 쓰면 다음 부팅이 그 모양을 복원한다).
- **상한(I4) — 코덱이 쓰기를 거절하면 쓰지 않는다:** `EncodeError::TooLarge`(직렬화 결과가 4 MiB — 읽기 상한, §6-2 — 를 넘는다) · `EncodeError::Unparsable`(같은 파서로 다시 읽히지 않는다 — 닿는 원인은 serde_json 의 중첩 상한 128 단이고, 같은 슬롯을 거듭 나누면 분할 121겹에서 닿는다 · 바이트 상한은 이것을 못 잡는다). log · 요청에는 `Failed`(명령에는 `INTERNAL`). 그대로 쓰면 다음 부팅이 못 쓸 파일로 접는다. 거절된 스냅숏은 그 변경 번호를 본 것으로 치고 **다음 변경을 기다린다** — 같은 스냅숏은 또 거절된다.
- **rename 직전 확인 변형(I8):** `fsutil` 에 「rename 직전에 물어볼 함수」를 받는 변형을 둔다 — **첫 rename 전과 잠김 재시도마다 다시** 묻고, 서 있으면 임시 파일을 지우고 `Err` 가 아니라 **따로 「건너뜀」**을 돌려준다(쓰기 실패가 아니다 — 재시도 대상이 아니다). `write_atomic` 은 그 얇은 포장이 된다. 묻는 것 = 아래 `closed`.
- **요청 둘(채널 + 답):** 답 = `Written`(발행했다) · `Skipped`(닫힌 뒤라 발행하지 않았다 — 실패가 아니다) · `Failed`(스냅숏 원천 실패 · 코덱 거절 · 디스크 오류).
  - `Resolve { hash }` — 답(수락 · 거절)을 디스크에 붙인다(I2 · §6-7). ★**해결 칸은 기록기가 요청을 꺼낼 때 선다**★ — 보내는 쪽 원자 값으로 세우지 않는다(I8): 꺼낸 뒤 뜬 스냅숏만 그 답 뒤의 화면(수락이면 커밋)을 담는다. 곧바로 스냅숏을 써서(`resolved_crash_copy: hash` 를 싣는다) `Written` 또는 `Failed` 로 답하고, rename 이 성공하면 사본을 지운다 — ★**지우기 직전마다 사본을 다시 읽어(≤4 MiB) 그 해시가 칸의 해시와 같을 때만 지운다(N1)**★: 다르면(새 인스턴스가 뜬 새 사본) 지우지 않고 칸을 비우고, 없으면 칸을 비운다. 잠금 없이 진행한 길이나 2초 마감 경합에서 낡은 기록기가 새 사본을 지우지 못하게 한다. 지우기가 실패하면 칸을 남겨 **뒤의 성공 쓰기마다 다시 지우고**(그동안 해시를 계속 싣는다), 지워지면 칸을 비운다(해시도 더 싣지 않는다). 쓰기가 실패해도 칸은 남아 다음 성공 쓰기가 같은 일을 한다.
  - `Final` → `clean_exit:true` 스냅숏(해결 칸이 서 있으면 해시를 싣고 지우기도 다시 해 본다 — 같은 해시 재확인)을 쓰고 답한 뒤 스레드 종료.
  - **걷은 것(6판):** `Flush`(첫 쓰기는 부팅 단계가 직접 · 확정 쓰기는 `Resolve`) · `DropCrashCopy`(거절도 `Resolve`) · `Final{drop_crash_copy}`(D6 — 정상 종료는 사본을 지우지 않는다).
- **종료(`RunEvent::Exit`) = 마감 하나(2초 — 상수 `REPLY_DEADLINE` · `Resolve` 도 같은 마감 · 부팅의 잠금 대기 약 3초(§6-5 ①)보다 짧아야 한다):** `Final` 을 보내고 **같은 마감까지** 답을 기다린다. 마감을 넘기면 `closed` 표지(원자 값)를 세우고 돌아간다 — 기록기의 rename 은 위 변형이 `closed` 를 물어 서 있으면 「건너뜀」(발행 없음 · 사본도 안 지움). 결과는 직전 `clean_exit:false` 가 남아 다음 부팅이 묻는다. 갇힌 기록기가 나중에 깨어나도 발행하지 못한다. 기다림이 끝나면 `state.lock` 을 놓는다(§6-6).
- **쓰기 실패 = log 만 · 따로 다루지 않는다(사용자 결정 D8).** 다음 주기 저장이 통째로 다시 쓴다. 실행 표식 쓰기(§6-5 ⑤) 실패도 같다. **실패 로그는 갈래(스냅숏 · 코덱 · 디스크)마다 접는다** — 갈래가 처음이거나 바뀔 때만 제 레벨(스냅숏 · 코덱 = error · 디스크 = warn)로 남기고 같은 갈래의 되풀이는 debug(디스크 오류가 이어지면 1초마다 다시 쓰므로) · 실패 뒤 다시 성공하면 info 한 줄. 근거 = [조사](../../research/crash-session-snapshot-precedents-2026-10-02.md) 「덧붙임」 — Firefox · Chromium · VS Code · Windows Terminal 모두 로그만 남기고 계속 돈다.

### 6-5. 부팅 — 단일 인스턴스 관문 뒤 · 어느 창보다 먼저

- **사실 ①:** 단일 인스턴스 플러그인은 `.build()` 도중 자기 플러그인 setup 에서 둘째 인스턴스를 `std::process::exit(0)` 으로 끝낸다(`tauri-plugin-single-instance-2.4.2/src/platform_impl/windows.rs:55-108`). 디스크를 바꾸는 코드는 그 뒤여야 한다.
- **사실 ②(I1 근거):** 그 플러그인은 `RunEvent::Exit` 에서 뮤텍스를 놓는다(같은 파일 `:111-123`). 그리고 플러그인 사건 처리가 우리 `RunEvent::Exit` 콜백보다 **먼저** 돈다(`tauri-2.11.3/src/app.rs:1430-1432` → `on_event_loop_event` `:2644-2648`). 그래서 앞 인스턴스가 `Final`(≤2초)을 쓰는 동안 새 인스턴스가 관문을 지난다. 4판이 든 근거(앞 인스턴스의 창을 못 찾을 때 — `!hwnd.is_null()`)는 좁았다 — 뮤텍스가 풀리면 창 조회까지 가지 않는다.
- **사실 ③:** 플러그인 setup 은 `build()` 안에서 등록 순서대로 돈다(`tauri-2.11.3/src/plugin.rs:880-915` — `register` 가 등록 순서대로 쌓고 `initialize_all` 이 그 순서로 돈다 · 부르는 곳 = `app.rs:2440`). 설정 창(main · agent-tree)은 그 뒤 `RunEvent::Ready` 의 `setup()` 에서 사용자 setup **앞**에 만들어진다(`app.rs:2521-2531`). ★P4(`11f8787`)부터 두 창은 `"create": false` 라 Tauri 가 만들지 않고 사용자 setup ⑧ 이 만든다(§4) — 앞 문장은 Tauri 가 만드는 설정 창의 차례이고, 지금 이 앱의 창이 처음 생기는 자리는 ⑧ 이다★.
- **그래서 부팅의 파일 일은 단일 인스턴스 플러그인 바로 뒤에 등록한 작은 플러그인(부팅 단계)의 setup 에서 한다** [고름] — 관문 뒤이고 어느 창보다 앞이다. `LayoutState` · 복원 서비스는 지금처럼 빌더에서 빈 채로 manage 하고(ADR-0102 그대로 — 첫 invoke 전에 존재), 부팅 단계가 창이 생기기 전에 그 안을 채운다. ★잠금 대기를 빌드 전에 두지 않는 이유★: 거기는 관문 앞이라, 앞 인스턴스가 멀쩡히 떠 있을 때(트레이에 있는 앱을 다시 실행) 매번 잠금을 기다리느라 넘겨주기가 3초 늦는다. 4판의 「빌드 전 읽기 + setup 지문 재확인」은 걷었다 — 잠금 뒤에 읽으므로 앞 인스턴스의 마지막 쓰기를 본다.

**부팅 단계(플러그인 setup · 한 스레드):** ★**setup 은 `Err` 를 돌려주지 않는다**★ — 어느 단계가 실패해도 log 하고 계속한다(D8 · N7 — `Err` 면 Tauri 빌드가 멈춰 앱이 아예 안 뜬다).

0. **로그 초기화(N7)** — 지금 사용자 setup 첫머리의 `init_logging_with_file`(`lib.rs` setup)을 부팅 단계 맨 앞으로 옮긴다 [고름]. 관문 뒤라 곧 끝날 둘째 인스턴스는 여전히 로그를 열지 않는다. 그래서 상태 쪽엔 「진단을 모아 뒀다가 나중에 내기」가 없다.
   - 설정의 적재 기록 되풀이(빌드 전 적재가 모아 두고 `enable_writes` 가 낸다 — §5-3)는 그대로 둔다 — `enable_writes` 는 사용자 setup 이라 로거가 이미 서 있다. 되풀이를 걷는 길(설정 적재도 이 단계로 옮겨 로거 뒤에서 바로 낸다 — 이 단계도 첫 invoke 전이다)은 있으나 P2a 코드는 손대지 않는다.
1. **잠금 `shell\run\state.lock`(I1).** 먼저 `shell\run` 폴더를 만든다(없으면 잠금 파일을 못 연다). `std::fs::File::try_lock`(배타 · 플랫폼 중립) [고름 — 이 파일은 아무도 읽지 않으므로 net `instance.rs` 머리의 「구간 잠금이 다른 프로세스의 읽기를 막는다」 사유가 해당 없다]. 잡혀 있으면 약 3초까지 다시 시도한다(앞 인스턴스의 `Final` 마감 2초 + 여유). 그래도 못 잡으면(폴더를 못 만든 경우 포함) log 하고 잠금 없이 진행한다(D8 정신). 잡은 잠금은 프로세스 내내 쥐고 `Final` 뒤에 놓는다(§6-6) — 프로세스가 죽으면 OS 가 푼다.
2. **읽기** `state.json` · `state.crash.json`(상한 4 MiB · 잠김이면 짧게 재시도 — §6-2). 남은 임시 파일 — 상태 파일들 옆에 `fsutil` 이 만드는 `<이름>.tmp<pid>.<n>` **전부**(원자 쓰기의 `state.json.tmp<pid>.<n>` · `state.crash.json.tmp<pid>.<n>`, 그리고 떠 두기 `copy_atomic` 의 `state.json.corrupt.tmp<pid>.<n>` · `state.crash.json.corrupt.tmp<pid>.<n>`) — 은 **그 pid 가 자기거나 죽은 것만** 지운다(`engram_dashboard_platform::process::pid_alive` — 옛 `base::platform`, master 머지 2026-10-07 에 OS 층 crate 로 옮겨 갔다 · ADR-0266) — 잠금 없이 진행한 경우 살아 있는 앞 인스턴스의 쓰는 중인 파일을 지우지 않는다. ★이름에서 pid 를 꺼내는 해석(과 그 쓸기)은 이름 꼴의 주인인 `fsutil` 에 둔다 — `<이름>.tmp<pid>.<n>` 꼴을 정하는 곳이 그곳뿐이다. ✅ P3b2 — `fsutil::sweep_temps` · `temp_owner`★.
3. **판정 = 순수 함수** `decide_boot(BootInputs { state, crash_copy }) -> BootPlan { model, actions, guard, crash_copy, carry_resolved, state_aside }`(`state/boot.rs` — 뼈대 P3b2 · 사본 열 P3c1 · `state_aside` P3c2):
   - 입력 둘 = 두 파일의 읽기 `StateRead` — `Missing` · `IoFailed(사유)` · `Unusable(사유)` · `Usable { file, text, warnings }`(`text` = 읽은 원문 그대로 — 사본 뜨기 · 사본 해시 · 수락의 원천이 이 바이트다).
   - `model` = `Default` | `Restore(StateFile)` · `actions: Vec<BootAction>`(④ 가 이 순서대로 — `CopyAsideState` · `CopyAsideCrashCopy` · `RemoveUnusableCrashCopy` · `WriteCrashCopy` · `RemoveAnsweredCrashCopy { hash }`) · `crash_copy: Option<AwaitingCopy { text, hash, file }>`(`Some` = 묻는다 — 가드 ⅱ 여도 선다) · `carry_resolved: Option<hash>` · `guard: Option<Guard>` = `Guard::StateUnreadable(사유)`(ⅰ) | `Guard::CrashCopyNotWritten(사유)`(ⅱ) · `state_aside: Option<StateAside>` = 못 쓸 `state.json` 떠 두기의 결과(`CopiedAside` | `NotCopied` — 판정은 `None` 으로 두고 ④ 가 세운다 · 사본 쪽 떠 두기와 섞지 않는다 · P3c2 — ⑥ 의 `state_file` 이 읽는다). `decide_boot` 는 그대로 순수하다.
   - ★**가드 계약: 이번 실행이 상태를 쓴다 ≡ `guard.is_none()`**★ — 판정(③)이나 동작(④)이 세우고 ⑤ 는 ④ 뒤의 값을 본다.

| 사본 `state.crash.json` | `state.json` | 모델 | 동작 | 묻나 |
|---|---|---|---|---|
| 있음 · **답 없음**(해시 ≠ `resolved_crash_copy`) | 무관 | 기본 | **사본을 덮지 않는다**(D2-6) | **묻는다**(모달) |
| 있음 · **버전 초과**(새 판이 쓴 사본 — 하향) | (사본 없음으로 치고 아래 행대로) | | **지우지도 떠 두지도 덮지도 않는다**(N6) — 새 사본을 떠야 하는 행이면 못 뜬 것이라 아래 가드 | 그 사본으로는 아니오(이 판은 적용 못 한다 — §12 R3) |
| 없음 | 없음 | 기본 | — | 아니오 |
| 없음 | `clean_exit:true` | 복원(조용히 — 「항상 복원」) | — | 아니오 |
| 없음 | `clean_exit:false` | 기본 | **`state.json` 원문 → `state.crash.json`**(원자 쓰기 · ⑤보다 먼저) | **묻는다**(D5 — 모양 무관) |
| 있음 · **답함**(해시 = `resolved_crash_copy` — I2) | (사본 없음으로 치고 위 행대로) | | 지우기 직전에 다시 읽어(≤4 MiB) **해시가 같을 때만** 지운다(N1) — 새 사본을 뜰 행이면 그 쓰기가 갈아끼운다 · 못 지우면 해시를 기록기에 넘긴다(`carry_resolved`) | 그 사본으로는 아니오 |
| 못 쓸 파일(버전 초과 말고 — JSON 아님 · UTF-8 아님 · 상한 초과) | (사본 없음으로 치고 위 행대로) | | `state.crash.json.corrupt` 로 떠 둔 뒤 지운다(앞선 사본을 덮어쓴다 — ADR-0274) · **떠 두기가 실패하면 지우지 않는다**(다음 부팅이 다시 떠 둔다) — 단 새 사본을 뜰 행(`clean_exit:false`)이면 그 쓰기가 백업 없이 갈아끼운다(D8 · ADR-0274 대가 2 · error 로그) | 그 사본으로는 아니오 |
| 읽기 IO 실패(I3) | (사본 없음으로 치고 위 행대로 — 단 새 사본은 뜨지 않는다: 동작 칸) | | **그 사본은 건드리지 않는다** — `clean_exit:false` 면 덮어도 되는지 몰라 새 사본을 못 뜬다 → 가드 ⅱ(묻기는 메모리의 `state.json` 원문으로) · `clean_exit:true` + `resolved_crash_copy` 면 그 해시를 이어 싣는다(`carry_resolved`) | 그 사본으로는 아니오 |

   - **못 쓸 `state.json`(§6-2) = 사본 열과 무관하게 `state.json.corrupt` 로 떠 둔다**(⑤가 갈아끼운다 · N6). 모델 · 묻나는 「`state.json` 없음」 행대로 — 사본이 답 없음이면 기본 화면 + 모달이다.
   - ★**읽기 IO 실패(I3)는 파일마다 가른다(세션 판단(사용자 위임) 2026-10-04 — 사용자 확인 2026-10-05)**★ = 그 파일을 「없음」으로 치고 **그 파일에 대한 동작만**(떠 두기 · 지우기 · 그 자리를 덮기) 하지 않는다. 다른 파일의 동작은 그 파일의 읽기만 본다 — 둘을 묶으면 사본을 못 읽은 부팅이 못 쓸 `state.json` 을 떠 두지 않은 채 ⑤ 로 덮는다(`decide_boot` 의 주석).
     - **`state.json` IO 실패** = 가드 ⅰ · 사본 열은 제 읽기대로 간다 — 답 없는 사본이면 묻고(가드라 `durable:false`), ★못 쓸 사본이면 가드 ⅰ 아래서도 떠 둔 뒤 지운다★. 답했다는 사실(`resolved_crash_copy`)은 읽은 `state.json` 에만 있으므로 남은 사본은 답 없는 사본으로 읽힌다.
     - **사본 IO 실패** = ★못 쓸 `state.json` 은 그래도 떠 둔다(⑤ 가 갈아끼운다)★ · 나머지는 위 표의 「읽기 IO 실패」 행.
   - **표가 말하지 않던 칸 — 코드가 정했다(P3c1):** ① 사본 IO 실패 + `clean_exit:true` + `resolved_crash_copy` → 조용히 복원하고 그 해시를 이어 싣는다(버리면 아직 남았을지 모르는 답한 사본을 다음 부팅이 답 없는 사본으로 읽고 다시 묻는다) ② 떠 두기에 실패한 못 쓸 사본은 지우지 않는다(다음 부팅이 다시 떠 둔다) — `clean_exit:false` 행만 새 사본 쓰기가 갈아끼운다(D8 · ADR-0274 대가 2 · 로그가 「백업 없이 갈아끼운다」를 말한다) ③ 사본 읽기가 계속 IO 실패하는데 `state.json` 이 `clean_exit:false` 면 부팅마다 가드 ⅱ — 부팅마다 묻고 아무것도 저장하지 않는다(§12 R16 — 수용).
   - ★**가드 하나(N3)**★ — **ⅰ `state.json` 을 못 읽었거나(IO 실패 — 재시도 뒤) ⅱ 떠야 할 사본을 못 떴으면**(쓰기 실패 · 사본 읽기 IO 실패로 동작을 건너뜀) 가드를 세운다(`guard = Some(사유)` — 「쓴다」가 거짓): ⑤를 건너뛰고 기록기를 띄우지 않는다 · log. 이번 실행은 상태를 하나도 저장하지 않는다 — 덮으면 아직 사본으로 뜨지 못했을지 모르는 크래시 화면을 잃는다. 다음 부팅이 다시 판정한다. 모드가 아니라 이 조건 하나다(원칙의 예외 — 답하지 않은 크래시 화면을 지킨다).
     - ★**가드 ⅰ · 못 쓸 `state.json` 은 사람에게 안내한다(사용자 결정 2026-10-05)**★ — `state.json` 을 못 읽으면(잠김 · 권한 — 짧은 재시도 뒤) **안내만 하고 그 실행은 끝까지 저장하지 않는다** — 재시도를 늘리거나 실행 중 다시 읽지 않는다(사용자: 「가장 깔끔한 구현」). 못 쓸 파일을 `.corrupt` 로 떠 두고 기본 화면으로 시작한 경우도 같은 방식으로 안내한다(관행 — 세션 추천에 사용자 이의 없음). 자동 복구 단추는 없다. LLM 은 `restore.status` 의 `state_file` 로 같은 사실을 안다. 여기까지 사용자 결정이다. ★떠 두지 못한 경우(`corrupt_not_copied`)도 같은 방식으로 안내하는 것은 세션 판단(사용자 위임 — 위 결정에서 도출)이다★ — 사용자가 따로 고른 갈래가 아니다. 관행 조사 = [`state-file-error-notice-practice-2026-10-05.md`](../../research/state-file-error-notice-practice-2026-10-05.md)(Windows Terminal 「Temporarily using the … default settings」가 최근접 선례).
     - **상태 표면(P3c2):** `state_file` = `ok`(읽었거나 없었다 — ★가드 ⅱ 도 이 값이다★: 이 칸은 `state.json` 읽기만 싣는다) | `unreadable`(가드 ⅰ) | `corrupt_copied_aside`(떠 두고 기본 화면) | `corrupt_not_copied`(떠 두지 못하고 기본 화면 — 원본은 ⑤ 가 백업 없이 갈아끼우고, 이미 있는 `state.json.corrupt` 는 앞선 실행이 떠 둔 것이지 이번 원본의 백업이 아니다). ⑥ 이 사본 상태와 같은 `set_boot` 호출에서 정하고(사본이 없어도 선다) 그 실행 내내 바뀌지 않는다. 가르는 순서 = 가드 ⅰ 이면 `unreadable` · 아니면 `state_aside`(`CopiedAside` → `corrupt_copied_aside` · `NotCopied` → `corrupt_not_copied` · 없음 → `ok`) — `state/boot_plugin.rs` 의 `state_file_status`. ★이 실행이 저장하나는 이 칸이 아니라 `saves` 가 말한다(후속 `125d190` — 아래 가드 ⅱ 안내(사용자 결정)를 실현하려고 더한 칸이고 칸 자체는 구현 · 세션 판단이다 · 늘 값 · ⑥ 이 같은 `set_boot` 호출에서 `saves = guard.is_none()` 으로 정하고 답해도 · 되돌려도 그 실행 내내 같다 · 가드 ⅰ · ⅱ 면 `false`) · 사본을 묻는 동안의 `durable` 은 `saves` 와 같은 값이다★(§6-7 명령). P3c2 때는 그 사실을 묻는 동안의 `durable` 만 실어, 답한 뒤에는 가드 ⅱ 를 말하는 칸이 없었다(§14-15 열린 것).
     - ★**가드 ⅱ 도 같은 안내를 띄운다(2026-10-06 · ADR-0276 결정 4)**★ — 세션이 「새 결정이라기보다 10-05 결정을 이 경우에도 적용할지의 문제 — 따로 말씀이 없으면 그렇게 보고 같은 안내 띠를 띄우는 쪽으로 진행」으로 올렸고 사용자가 「그래 띠로 하던가」로 답했다. 조건 = `saves:false` 이고 `state_file:'ok'`(구현 — 세션 판단) · 문구 `i18n/ko.ts` `restore.stateFileNotSaving` · 답한 뒤에도 남는다. 가드 ⅰ 도 `saves:false` 지만 그 문구가 「저장하지 않는다」를 이미 실어 겹쳐 띄우지 않는다(`StateFileNotice.tsx` 의 `messageOf`).
     - **화면 모양:** main 레이아웃 칸 위를 덮는 닫을 수 있는 안내 줄(`src/components/layout/StateFileNotice.tsx` · 닫으면 그 실행 동안 다시 안 뜬다 · 고치는 단추 없음 · 문구 `i18n/ko.ts` `restore.stateFile*` · `restore.stateFileNotSaving`). 모달과 같은 상태 창구(`restoreClient` — §6-7 모달)를 읽는다.
       - ★**레이아웃을 밀지 않고 덮는다 · TabBar 를 가려도 된다(✕ 로 닫는다) · 여럿이면 쌓고 넘치면 스크롤 = 사용자 결정 2026-10-06(ADR-0276)**★ — 사용자: 「우리는 레이아웃을 고정시키고 그 고정된 상황에서 작업하는건데 팝업이 나오면 밀림」 · 「탭바 가려도 되지않음? 어차피 x눌르면 없어지는거잖아」 · 「일정 이상 나오면 스크롤」. P3c2(`7b742fb`)는 `ConnectionNotice` 와 같은 흐름 블록으로 밀었다 — 그 모양은 세션 판단(사용자 위임 — 「쭉 진행해」)이었고 이 결정이 대신한다. 닫을 수 있는 안내 줄이라는 모양 자체는 그대로 세션 판단이다 — 모달처럼 막지 않는다 · 가린 자리는 ✕ 전까지 못 누른다.
       - **구현(후속 `125d190`):** 덮는 층 = `src/components/layout/AppLayout.tsx` 의 감싼 `div`(★`9318b5c` 부터는 세 창 공용 `src/components/layout/NoticeOverlay.tsx` 이고 `AppLayout` 은 그것을 단다 — 아래 연결 띠 항목★ · 인라인 `position:absolute` · top/left/right 0 · z 40 · `drop-shadow`) — 그 안에 Radix ScrollArea(뷰포트 `max-h-[33vh]`). ★자리는 감싼 `div` 가 잡는다★ — Radix ScrollArea 의 Root 는 인라인 `position: relative` 를 박아 위치 클래스가 진다(코드 리뷰가 잡은 결함). 기준 = 레이아웃 칸(`flex:1` · `minHeight:0` · `position:relative`)이라 그 칸의 크기는 안내가 떠도 그대로다. 층의 높이는 안내 줄만큼이다 — 전체 높이의 투명 막을 깔면 줄 밖의 클릭까지 먹는다(구현에서 나온 세션 판단 — 사용자 결정이 아니다 · 줄 바깥 빈 영역 이야기는 사용자가 다음으로 미뤘다 — §14-16 열린 것). 층은 묻는 동안 잠기는(`inert`) 감싸개 안이고 복원 모달(z-50) 아래다. 넘침 상한 `33vh`(창 높이의 1/3 — 쌓여도 배치의 2/3 는 보인다)는 세션 판단이다.
       - ★**연결 띠(`ConnectionNotice`)도 레이아웃을 밀지 않고 덮는다 — main · 팝아웃 · 트리 창 모두 = 사용자 결정 2026-10-06(ADR-0277 · ADR-0180 개정)**★ — 사용자: 「덮기로해.」(질문의 틀 = ADR-0277 결정 1). **구현(`9318b5c` — 세션 판단):** 위 층을 공용 `NoticeOverlay` 로 뽑았다(줄 사이 1px 틈 — `gap-px` · `bg-background`) · main = 연결 띠 → 상태 파일 안내 차례로 한 층(`inert` 감싸개 안) · 팝아웃 = 내용 상자에 `position:relative` · 트리 = 머리줄 아래 새 상자(`flex:1` · `minHeight:0` · `position:relative`)가 층과 목록을 담는다 · 줄 계약 = 불투명 바탕 · `wrap-anywhere`. 정본 = ADR-0277 · 기록 = §14-17. ~~ADR-0180 대로 흐름에 남아 레이아웃을 민다(덮는 층 밖 · 레이아웃 칸 위에 쌓인다)~~ — 후속 `125d190` 까지의 모양이고, 그때 죽은 `destructive` 클래스만 테마 토큰으로 바꿨다(팝아웃 · 트리 창의 띠도 같은 컴포넌트라 함께 바뀐다).
   - **사본이 있으면 = 답하지 않은 것(D4 · D6):** 모달이 답 전 사람의 사용을 막고 정상 종료도 사본을 지우지 않으므로, 부팅에 사본이 있다는 것은 답이 없었다는 뜻이다 — 답한 뒤 지우기 전에 끝난 것(해시로 가린다)만 빼고. 그래서 `state.json` 이 무엇이든 기본 화면 + 모달이다 — 너무 새 판의 사본은 예외(N6). 5판의 「`clean_exit:true` + 사본 → 조용히 복원하고 묻는다」 행은 걷었다.
   - **비정상 = 늘 묻는다(D3 · D5):** 기본 화면으로 죽은 세션도 사본을 뜨고 묻는다. 「기본 모양」 판정은 없다(§6-1).
4. **동작 실행.** 사본 뜨기 = `state.json` 원문 바이트를 원자 쓰기로 `state.crash.json` 에(반쪽 사본 없음). 떠 두기 · 지우기 실패는 log 하고 진행한다(D8). 사본 뜨기 실패는 위 가드다. ★④ 의 결과 중 ⑤ · ⑥ 이 알아야 할 것(사본을 못 떠 서는 가드 · 못 지운 답한 사본의 해시 `carry_resolved` · 못 쓸 `state.json` 떠 두기의 결과 `state_aside` — P3c2)은 ④(`run_actions(files, &mut plan)`)가 계획에 접어 넣고 ⑤(`write_run_marker`) · ⑥ · 기록기가 그 계획을 읽는다 — 그래서 가드 로그는 ④ 뒤에 낸다(`state/boot.rs` 머리)★. 답한 사본 지우기(N1)는 다시 읽어 해시가 다르면(새 사본) · 못 쓸 내용이면 지우지 않고 해시도 넘기지 않으며, 못 지웠거나 다시 못 읽으면 해시를 넘긴다 — 기록기의 같은 정리와 같은 갈래다. 읽기와 지우기 사이는 원자적이지 않고 그 틈은 셸 실행 잠금(I1)이 막는다.
5. **첫 쓰기 = 실행 표식**(`clean_exit:false` · 판정한 모델 · `carry_resolved` 해시 — 가드면 건너뜀) — 기록기가 아직 없으므로 직접 원자 쓰기. **어느 창보다 먼저다** — 조용히 복원한 화면이 창을 만들다 앱을 죽여도 다음 부팅이 비정상으로 읽는다(Chromium `exit_type` 도 시작 때 세운다 — 조사 표). 실패는 log 하고 진행(D8).
6. **채우기:** 판정한 모델을 `LayoutState` 에 · 트리 칸을 `tree_attrs` 에 넣고, **복원 서비스 상태(`none` / `awaiting` + 사본 바이트 · 해시 · 가드 여부 · `state_file`)를 여기 한 곳에서 정하고 `restore:changed` 를 낸다(I5).** 창이 아직 없으므로 모든 창의 첫 `restore_status` 당기기가 확정된 값을 본다 — ~~사용자 setup 에서 정하면 Tauri 가 그 앞에서 만든 설정 창이 먼저 `none` 을 볼 수 있다(사실 ③)~~ ★P4 부터 창은 사용자 setup ⑧ 에서 처음 생기므로 사용자 setup 의 ⑧ 앞에서 정해도 차례는 서지만, 그 차례를 손으로 지켜야 한다 — 그래서 여기 둔다(`state/boot_plugin.rs` 머리)★. P3c1 = `RestoreService::set_boot(Option<CrashCopy { text, hash, file, durable }>)` · `durable = guard.is_none()` → P3c2 = `set_boot(copy, state_file)` — 같은 호출에서 `state_file`(③ 가드 항목)도 선다 → 후속 `125d190` = `set_boot(copy, state_file, saves)` · `saves = guard.is_none()` · `CrashCopy { text, hash, file }`(`durable` 칸을 걷었다 — 묻는 동안의 `durable` 은 서비스가 쥔 `saves` 를 같은 잠금 읽기에서 싣는다) — 알림 포트(`restore:changed` → main)는 플러그인 setup 이 부팅 단계 앞에 꽂는다(⑥ 에는 창이 없어 받는 쪽이 없다). ★**가드 ⅱ(떠야 할 사본을 못 떴다)면 `crash_copy = awaiting` 으로 정하고, 이 실행의 복원 원천은 메모리에 쥔 `state.json` 원문 바이트다** [고름 — L2]★ — 수락은 `durable:false`(기록기가 없다), 크래시 화면은 디스크에 그대로 남아 다음 부팅이 다시 뜨고 묻는다.

**사용자 `setup`(~~설정 창은 이미 있다~~ → ★P4 부터 창은 ⑧ 에서 처음 생긴다 — 그 앞 첫머리에서 웹뷰 폴더를 정한다(§4)★):**

7. 기록기 시작(⑤ 때의 변경 번호 · `carry_resolved` 를 받는다 — `StateSession::start_saver`) — 가드면 띄우지 않는다. ⑦ 전에 온 답은 답이 먼저 띄운다(§6-7 ⑤ — ~~설정 창은 사용자 setup 보다 먼저 만들어진다~~ ★P4 부터 창은 ⑦ 뒤 ⑧ 에서 생겨 오늘의 부팅 차례로는 ⑦ 전에 답이 오지 않는다 — 그 길은 방어로 남는다(`StateSession::resolve_crash_copy` 주석)★).
8. **main · 트리 창 속성 입히기(I6)** — ★P4 부터 = 정적 창을 저장된 속성으로 **만든다**(`from_config` + 웹뷰 마무리 — §4 · main 을 못 만들면 앱을 끝내고 트리 창을 못 만들면 그 창 없이 계속한다 — §4 「정적 창을 못 만들면」)★ — 저장된 위치 · 크기 · 최대화(보통 크기로 놓은 뒤 최대화 — 트리 창은 최대화를 싣지 않는다, F9) · ★**숨은 main 의 최대화는 처음 보일 때로 미룬다**(`state::placement::DeferredMaximize` — label 마다 둔다: main · 런타임 수락이 숨긴 채 만든 팝아웃(P3c1) · 보이기 경로가 입힌다 — 트레이 `show_main_ui` 는 모든 창에 · 수락의 `set_shown` · 입혀졌을 때만 거두고 못 입혔으면 다음 보이기까지 남는다 · 팝아웃 창 소멸(`Destroyed`)이 거둔다)★: 숨은 창을 최대화하면 tao 가 `SW_MAXIMIZE` 로 창을 보이고 활성화한 뒤에야 되숨겨 `--hidden` 부팅에 main 이 번쩍이고 포커스를 가져간다. 미뤄 둔 동안은 main 의 자리를 적지 않는다(숨은 동안 오는 `Moved` · `Resized` 가 최대화 표식을 내린다). ~~지금은 main 이 보인 채 만들어지고 `--hidden` 숨기기가 setup 끝이라 이 길을 타지 않는다~~ ✅ P4 — `--hidden` 부팅은 main 을 숨긴 채 만들어(§4 「덤」) 이 길을 탄다 · 보이는 부팅의 main 은 미룸을 세운 채 그 자리에서 입히고 입혀졌을 때만 거둔다(`DeferredMaximize::apply_now` · §6-7 ④) · **어느 모니터에도 안 걸치는 위치는 버린다** · 테마는 `push_effective_themes()` 한 번(§5-6 — ✅ P3d: ⑨ 의 팝아웃을 **연 뒤에** 민다 · `state/placement.rs` 의 `open_popouts_then_push_themes`(단위 시험이 붙었다) — 새 팝아웃도 받는 창 명단에 들고 못 연 팝아웃은 모델에서 빠진 뒤다). ⑨ 와 함께 `state::placement::restore_windows` 한 함수다. ★**부팅 전용이다 — 조율자 ②④는 이 함수를 다시 쓰지 않는다**★: 런타임에 부르면 떠 있는 팝아웃마다 이미 있는 label 로 창 만들기가 실패해 그 팝아웃을 모델에서 지우고 OS 창은 남긴다. **P3c1 의 런타임 경로 = 창 포트 `TauriRestoreWindows`(같은 `state/placement.rs`)** — 모니터 판정(`locate` + 로그 `land`) · 비공개 `place` · `place_main` 만 같이 쓴다(§6-7 ② ④). **실행 표식(⑤) 뒤다.** ~~L3 동안은 Tauri 가 main 을 설정 위치로 먼저 만들므로 떴다가 옮겨진다~~ — ✅ P4: 정적 창 생성(`from_config` — §4)이 이 자리로 와 저장된 속성으로 바로 만든다(저장된 보통 자리 · 크기를 빌더에 주고 만든 뒤 정확한 자리를 놓는다 — 팝아웃과 같은 길 · 어느 모니터에도 안 걸치는 자리면 설정의 자리로 연다) · 표식 뒤라는 순서는 그대로다. ★남은 것: 보이는 부팅에 저장된 최대화 main 은 약 0.5–0.8 초 보통 크기로 보인 뒤 최대화된다 — L3 와 같다(§14-21 알려진 한계)★.
9. 복원 모델의 팝아웃마다 창 생성(새 label · 저장된 위치 — **어느 모니터에도 안 걸치는 위치는 버린다(⑧과 같은 함수 · N5 — 4판 규칙 되살림)** · 저장된 최대화 · 첫 자리 기록) — 실패한 창은 모델에서 지운다(다음 주기 저장이 싣는다). ★**`--hidden` 전용 로직은 없다(사용자 결정 F13 · 2026-10-03)**★ — 팝아웃도 보이는 채 만들고(`build_runtime_window` 엔 숨김 인자가 없다) setup 끝의 숨기기(`hide_main_ui`)가 ~~main 과 함께~~ 숨기며(★P4 부터 main 은 ⑧ 이 처음부터 숨긴 채 만든다 — 그 숨기기가 숨기는 것은 이 팝아웃들이다(F13 대가 — §10). 단 그 숨기기는 숨긴 창을 사용량 관심에서도 빼므로 이미 숨은 main 도 이 길로 관심에서 빠진다 — 팝아웃이 없어도 건너뛰지 않는다(숨기기 경로는 하나 — ADR-0229 · §4 「덤」)★), `show_main_ui` 가 모든 창을 함께 보인다(`tray/actions.rs`). ~~창 생성은 죽은 창의 테마 항목 쓸기(`sweep_dead_window_entries` — setup 첫머리) 뒤다 — 그 쓸기의 전제가 「이 순간 팝아웃 창이 하나도 없다」다(ADR-0167 · P3d 까지).~~ ★P3d 에서 쓸기를 걷었다(`declared_window_labels` 포함 · ADR-0265 결정 7 — 창별 테마가 창 항목에 살아 창과 같이 죽는다) — 이 순서 제약도 함께 사라졌다★. 테마는 이 창들을 연 뒤에 민다(⑧ 끝).
10. **파생 표 재계산 = `SubscriptionSync::resync`**(라우터 재계산 + 사용량 관심 — `commands/layout.rs:74-78`) 한 번.

- 기본 레이아웃(ADR-0222 — 단일 빈 슬롯)은 **복원할 상태가 없을 때만** 쓰인다(§11).

### 6-6. 정상 종료 표식 · Windows 종료/로그오프

- **실행 표식 = `state.json` 의 `clean_exit`.** Chromium `exit_type`(시작 때 「Crashed」 · 정상 종료 때 「Normal」)과 같은 수법이라 새 표식 파일을 두지 않는다 — 부팅 단계 ⑤가 `false` 를 세우고(어느 창보다 먼저), `Final` 이 `true` 로 내린다(§6-4). 출처 = [조사](../../research/crash-session-snapshot-precedents-2026-10-02.md) 표 「비정상 종료 판별」.
- **`RunEvent::Exit`** 에서 §6-4 의 종료 절차 → 기다림이 끝나면 `state.lock` 을 놓는다(I1 — 그사이 관문을 지난 새 인스턴스는 §6-5 ①에서 기다린다). P3b3 착지 = `lib.rs` 의 `RunEvent::Exit` → `StateSession::shutdown`(`state/boot_plugin.rs`) — ★잠금을 drop 에 맡기지 않고 명시적으로 놓는다★: 이벤트 루프는 끝나면 곧장 `process::exit` 해 managed state 가 drop 되지 않고, 기대면 OS 가 핸들을 거둘 때에야 풀려 기다리는 새 인스턴스가 그만큼 더 막힌다(그 모듈 머리). 트레이 「완전 종료」(`app.exit(0)`)도 이 사건을 낸다(`tauri-2.11.3/src/app.rs:573-574`). ★가드(§6-5 ③ — 기록기 없음)면 `Final` 을 건너뛰고 잠금만 놓는다(N7)★.
- **로그오프/종료:** tao 는 `WM_ENDSESSION(wParam=TRUE)` 에서 루프를 파기하고(`tao-0.35.3/src/platform_impl/windows/event_loop.rs:2382-2391` — `WM_QUERYENDSESSION` 은 처리 안 함) tauri-runtime-wry 가 그것을 `RunEvent::Exit` 로 번역한다(`lib.rs:4185-4187`). 받는 창은 tao 의 사건 대상 창이고 메시지 전용이 아닌 최상위 창이라(`event_loop.rs:651-686`) 트레이 상태에서도 받는다 [미검 — 실제 로그오프 · §12 R2].
- 강제 종료(`taskkill /F` · 크래시 · 디버거 · 전원)는 표식을 못 남긴다 → 다음 부팅이 `state.json` 을 사본 한 개로 뜨고 기본 화면 + 모달로 묻는다(§6-5 · §6-7). 사본이 이미 있으면 덮지 않는다(D2-6).
- **정상 종료는 사본을 지우지 않는다(사용자 결정 D6)** — 모달이 떠 있는 채 트레이로 끝내도 다음 부팅이 다시 묻는다.

### 6-7. 비정상 종료 뒤 「복원할까요?」 — 모달 · 사람과 LLM 이 같은 핸들

- **크래시 사본 = `shell\state\state.crash.json` 한 개(D1 · D2 · D3 — §14-9).** 비정상 종료 뒤 부팅은 `state.json` 을 사본으로 한 번 뜨고 기본 화면으로 시작해 **늘 묻는다**(Chromium 식 · 거부 = Firefox 식, §11-4). 선례 = [`crash-session-snapshot-precedents-2026-10-02.md`](../../research/crash-session-snapshot-precedents-2026-10-02.md) — 크래시 스냅숏 N 개 회전 선례는 없고 성숙한 앱은 한 세대만 둔다.
- **모달(사용자 결정 D4):** main 창 웹뷰 안의 모달(`RestoreModal.tsx` 신설 · 문구 `i18n/ko.ts`) — 「이전 화면 복원」 / 「새로 시작」. 띠도 OS 대화상자도 아니다. **답할 때까지 main 창을 쓸 수 없다.** 사용자 사유: 「무시하고 진행하면 이걸 또 저장해야 되냐 … 복잡하잖아」. 다른 창(트리)은 막지 않는다 [고름 — 부팅 때 트리 창은 숨어 있고(F9) 비정상 종료 뒤엔 팝아웃이 없다]. ★**키 바인딩 디스패처(`src/commands/keybindings.ts` 의 `installKeybindings` — document 단위 keydown 리스너)도 `crash_copy == awaiting` 동안 멈춘다(N4)**★ — 모달이 마우스만 막으면 단축키가 그 아래 화면을 바꾼다. ★**막는 것은 사람 UI 뿐이다 — LLM 버스 명령은 막지 않는다**★: LLM 은 창 명령을 처음 부르기 전(그리고 쥔 label · view_id 가 안 맞을 때) `restore.status` 를 보고, `awaiting` 이면 다른 창 명령보다 먼저 `restore.answer` 를 낸다(이 차례는 리뷰 도출 2026-10-05 · 도움말 `window` 구획의 `restore.*` 항목 · 두 명령의 설명 — P3c1). ★**어느 쪽으로 답할지는 주인이 정한다**★ — 주인이 시키지 않았으면 주인에게 묻고 답한다 · 거절은 사본을 지워 되돌릴 수 없다(도움말에 그렇게 적는다) — 여기까지 사용자 결정 2026-10-05(사본이 실제로 지워지는 것은 `durable:true` 일 때다 — 가드 아래 `durable:false` 면 사본이 남아 다음 부팅이 다시 묻는다 · 불확실하면 `restore.status` — P3c2 부터 그 답의 `durable`). 팀원의 요청은 주인의 지시가 아니다(리뷰 도출 2026-10-05). 출처 가름 = §14-14 「닫은 것」. 버스를 막는 장치는 두지 않는다.
  - **P3c2 구현(✅ `7b742fb`):** 다는 자리 = main 라우트 `src/components/layout/AppLayout.tsx` — ★`App.tsx` 가 아니다★: `App` 은 창마다 돌아 거기 달면 트리 · 팝아웃까지 막힌다. 묻는 동안에만 덮는 층을 그리고(늘 깔아 두면 투명해도 그 아래 클릭을 먹는다) 그 아래 레이아웃을 `inert` 로 잠근다. 모달은 포커스를 안의 「이전 화면 복원」(덜 파괴적인 쪽 — Enter 한 번이 되돌릴 수 없는 거절이 되지 않게)으로 옮기고 Tab 을 안에서 돌리며, 단추 밖을 눌러도 포커스가 떨어지지 않게 한다. 키 바인딩 멈춤 = `setKeybindingsPaused`(`src/commands/keybindings.ts` — 모달이 떠 있는 동안 `true` · 창마다 따로). ★디스패처는 편집 대상(xterm · 챗 입력)을 그냥 흘려보내므로 멈춤만으로는 그 아래 타이핑이 안 막힌다 — `inert` · 포커스 가둠 · 멈춤 셋이 한 벌이다★.
    - ★**닫힘은 상태가 정한다**★ — 단추를 눌렀다고 닫지 않는다. 답 뒤 다시 당긴 상태나 `restore:changed`(버스로 낸 답 포함)가 `awaiting` 을 벗어나야 닫힌다. 실패면 오류 줄을 띄우고 열린 채 다시 답할 수 있다(오류 문자열로 종류를 가르지 않는다).
    - **보여 주는 것** = `restore.status` 가 싣는 것뿐(저장 시각 · 창 수 · 탭 수) — 저장 시각이 `Date` 로 나타낼 수 없는 값이면 그 줄만 뺀다(`formatSavedAt` — 거르지 않으면 형식화가 던져 모달이 아니라 화면 전체가 오류 경계로 넘어간다). ★**거절 경고는 `durable` 을 따른다**★ — `false` = 「이번 실행은 저장하지 않아 이전 화면이 남고 다음 실행 때 다시 묻는다」 · `true` = 「지운다 · 되돌릴 수 없다」 · 묻는 동안 `null`(오지 않는 값)이면 「지운다」 쪽에 둔다. 경고는 `state_file` 을 보지 않는다(가드 ⅱ 에서도 `ok` 다 — §6-5 ③). 문구 = `i18n/ko.ts` `restore`.
    - **상태 창구 = `src/api/restoreClient.ts`** — `settingsClient.ts` 와 같은 예외로 invoke · listen 을 직접 건다(복원 상태의 주인은 데몬이 아니라 셸 · 시험 seam = `RestoreIpc`, 가짜 = `src/api/testing/fakeRestoreIpc.ts`). `restore:changed` 구독 등록이 끝난 **뒤에** 첫 `restore_status` 를 당기고 알림마다 다시 당긴다(알림 짐에 수가 없다) · 나중에 낸 당기기의 답이 이긴다(순번) · 내린 설치(재마운트 · 언마운트)의 답은 칠하지 않는다 · 부팅 당기기는 받을 때까지 놓지 않는다(유계 재시도 뒤 2초마다 — `BOOT_REPULL_INTERVAL_MS` · 알림이 부른 당기기가 먼저 받으면 멈춘다) · 답(`restore_answer`)은 성공이든 실패든 상태를 다시 당긴 뒤에 풀린다. 모달과 상태 파일 안내(§6-5 ③ 가드 항목)가 이 한 벌을 읽는다. ★짐 읽기(후속 `125d190` · 리뷰 반영)★ — 알림만 쓰는 두 칸 `saves` · `state_file` 은 관대하게 읽는다: 없거나 모르는 값이면 `undefined`(칸마다 한 번만 경고)로 두고 짐을 살린다 — 그 칸의 안내만 안 뜬다(짐을 버리면 묻는 모달까지 안 뜬다). `crash_copy` 를 못 읽은 짐은 실패한 당기기로 친다(던진다 — 말없이 버리면 부팅 당기기가 받은 것처럼 끝나 셸이 묻고 있어도 모달이 안 뜬다 · 던지면 부팅 다시 당기기가 맞는 짐을 받을 때까지 돈다). ★알려진 잔여(셸 · 프론트 판 어긋남 전용 · 리뷰가 남긴 선재)★ — 답 뒤의 다시 당기기나 알림이 부른 당기기가 못 읽는 `crash_copy` 를 받으면 다시 시도하지 않는다. ★알려진 잔여(실측 없음)★ — `listen()` 이 거절 없이 영영 안 풀리면 당기기도 영영 나가지 않아 모달이 안 뜬다(재시도는 거절만 덮는다 · `settingsClient.ts` 와 같은 잔여).
- **`--hidden` 부팅도 따로 다루지 않는다(사용자 결정 D7)** — 모달은 숨은 main 안에 그려져 있어 트레이로 창을 열면 거기 있다.
- **묻는 조건 = 복원 서비스 상태 `awaiting` 하나** — 부팅 단계 ⑥이 한 곳에서 정한다(I5).
- 셸 명령(셸 표): **`restore.status`**(Read) → `{crash_copy, saved_at_ms, windows, tabs, durable, saves, state_file}` — `crash_copy` = `none` | `awaiting` | `answered`(I2 — 5판의 `pending` 을 대신한다) · `saved_at_ms` · `windows` · `tabs` · `durable` 은 `awaiting` 일 때 값, 아니면 `null` · ★**`durable`(P3c2) = 묻는 동안의 `saves` 와 같은 값**(후속 `125d190` — P3c2 의 「사본의 값」(`CrashCopy.durable`)을 걷고 서비스의 `saves` 를 같은 잠금 읽기에서 싣는다 — 둘이 갈라질 자리가 없다)★: `true` 면 답을 디스크에 붙이고 사본을 지우려 한다(성공은 보장하지 않는다) · `false` 면 이 실행은 아무것도 저장하지 않아(가드 ⅰ · ⅱ) 답해도 크래시 화면이 디스크에 남고(사본 또는 `clean_exit:false` 인 `state.json`) 다음 부팅이 다시 묻는다 · 답이 실제로 디스크에 붙었는지는 `restore.answer` 의 `durable`(그대로 — 답의 확정 결과) · ★**`saves`(후속 `125d190`) = 이 실행이 화면 상태를 저장하나**★ — 늘 값(사본이 없어도) · ⑥ 이 정하고 답해도 · 되돌려도 그 실행 내내 같다 · `true` = 가드가 아니다: 기록기를 띄우려 한다(띄우기 · 쓰기 성공은 보장하지 않는다 — ⑦ 에서 기록기를 못 띄운 실행도 `true`) · `false` = 가드다: 가드 ⅰ(`state_file` 이 `unreadable`) · 가드 ⅱ(`state_file` 은 `ok`) · **`state_file`(P3c2)** = 늘 값이고 그 실행 내내 같다(값 · 뜻 = §6-5 ③ 가드 항목) · **`restore.answer {accept}`**(Write) → `{restored_windows, durable}` — `awaiting` 이 아니면(사본 없음 · 이미 답함) `CONFLICT` · **다른 답이 처리 중이면 `CONFLICT`**(조율자의 처리 중 표지 하나). Tauri 껍데기 `restore_status`/`restore_answer` 가 같은 서비스를 부르고, 바뀌면 `restore:changed` 를 main 에. 5판의 `restore_prompt_shown` 신호는 없앴다(쓰던 F12 가 D6 으로 없어졌다).
  - **P3c1 구현:** 버스 = `layout/commands.rs`(`catalog_version` 11 — P3c2 가 `restore.status` 답 모양을 바꿔 12 · 후속 `125d190` 이 `saves` 로 13 · `restore.status` = `blocking_handler` · `restore.answer` = `offloaded_handler` — 블로킹 풀) · Tauri = `commands/state.rs`(`restore_status` 동기 — 잎 락 하나 · `restore_answer` = async + `tauri::async_runtime::spawn_blocking`) · ts-rs 바인딩 `CrashCopyStatus` · `RestoreStatusView` · `AnswerReply`(P3c2 가 `StateFileStatus` 를 더했다). ★**답(`RestoreCoordinator::answer`)은 메인(이벤트 루프) 스레드에서 부르지 않는다**★ — 창을 만들고 · 놓고 · 거두는 동안 이벤트 루프를 기다리고 기록기 답을 마감(2초)까지 기다려, 메인에서 부르면 창 만들기가 자기 자신을 기다린다. 버스 적용 태스크는 클라이언트 런타임 워커에 뜨므로 거기서 그대로 막아도 그 워커의 소켓 태스크가 선다.
  - ★**버스의 `crash_copy` 는 문자열이다**★ — 선언 매크로가 enum 을 serde 철자(`none` …)로 싣지 못해(PascalCase 로 나간다) `CrashCopyStatus::as_wire` 로 싣고, 시험이 그 철자를 serde 철자에 묶는다(`restore.rs` `the_status_wire_spelling_is_snake_case`). Tauri 쪽은 serde 그대로다. `state_file` 도 같은 까닭으로 `StateFileStatus::as_wire` 문자열이다(P3c2 — `the_state_file_wire_spelling_is_snake_case`).
  - **셈:** `windows` = 사본의 main + 팝아웃(트리 창은 세지 않는다 — 탭이 없고 수락이 자리만 입힌다) · `restored_windows` = main + 모델에 남은 새 팝아웃 — ★같은 방식으로 세지만 값은 다를 수 있다★: `from_persisted` 가 건너뛴 창(§6-2)과 화면을 바꾸기 전에 닫힌 팝아웃(④)은 뒤엣것에서 빠진다.
  - **오류:** `AnswerError::Conflict` → `CONFLICT` · `AnswerError::NotReady`(창 포트를 아직 꽂지 않았다 — 셸이 뜨는 중 · `awaiting` 그대로) · `AnswerError::Internal`(수락이 커밋 전에 실패 — 준비 · 창 만들기 · 커밋 거절 · `awaiting` 그대로) → 둘 다 맨 `INTERNAL`(`layout/commands.rs` `restore_error`). ★**재시도 지시를 싣지 않는다(코드의 기본 `never`)**★ — 데몬이 중계하는 실패 답의 지시를 전부 `never` 로 내려(`command_delivery` 의 `send_reply` · ADR-0159) 셸이 실어도 부르는 쪽에 닿지 않는다. 그래서 넷째 라운드 계획(`.claude/handoff/attachments/p3c1/pending-fix-round4.md` ④)의 `with_retry(…, AfterCondition)` 는 쓰지 않았고, 「다시 답하라」는 `NotReady` 의 문구(「… answer again shortly」)와 도움말이 나른다. 오류 문구로 종류를 가르지 않는다 — 실패 뒤 상태는 `restore.status` 로 다시 읽는다.
- **복원 서비스(`RestoreService` — `state/restore.rs`):** 상태 셋(`none` · `awaiting` · `answered`)과 **처리 중인 답 하나의 표지**(`AnswerTicket` — 복제할 수 없다)를 쥔다. `begin_answer` 가 표지를 세우고 `finish_answer(Answered | RolledBack)` 로 끝맺는다 — ★끝맺지 않고 버리면(패닉으로 풀린 경우 포함) `RolledBack` 과 같다★: 표지가 걸린 채 남으면 그 실행에서는 영영 답할 수 없다. `set_boot` 마다 세대 번호가 올라 앞 세대 표지는 상태를 바꾸지 못한다. 서비스 락은 잎이고 알림(`RestoreNotifier`)은 락을 놓은 뒤 부른다. ★`restore:changed` 는 ⑥ 의 결정 · `Answered` · **`RolledBack`(`awaiting` 그대로)** 에 난다★ — 그 사이 `InFlight` 로 거절당한 쪽이 다시 답할 수 있음을 알게.
- **복원 조율자(`RestoreCoordinator`)의 포트는 셸 setup 끝에 늦게 꽂는다**(`attach(RestorePorts { windows, events, subs })` — 창 포트 · 레이아웃 알림 · 구독 원천은 `AppHandle` 이 있어야 선다). ★**⑨ · ⑩ 뒤다**★ — ⑨(`open_restored_popouts`)는 모델의 팝아웃마다 창을 만드는 부팅 전용 길이라, 그보다 먼저 수락이 커밋되면 ⑨ 가 수락이 이미 만든 label 로 다시 만들려다 실패해 그 팝아웃을 모델에서 지우고 OS 창은 고아로 남긴다. 그 전의 수락 = `AnswerError::NotReady` → 맨 `INTERNAL`(`awaiting` 그대로 — 잠시 뒤 다시 답한다 · 재시도 지시는 없다 — 위 「오류」) · 거절은 포트 없이 선다. 구독 원천(`SubscriptionSource`)은 쓸 때마다(락을 잡기 전에) 라우터 · 데몬 클라이언트를 찾는다 — 꽂을 때 한 번 찾으면 그 뒤에 등록된 클라이언트를 조용히 건너뛴다. 조율자도 빌더에서 manage 한다(ADR-0102).
- **사본 수명 — 답만 사본을 푼다(사용자 결정 D6):**
  - **수락 · 거절** = 상태 `answered` → 기록기에 `Resolve { hash }` → 그 해시를 실은 `state.json` 쓰기 뒤 사본 삭제(못 지우면 뒤 쓰기마다 다시 — §6-4). **답은 그 쓰기로 디스크에 붙는다(I2)** — 사본을 지우기 전에 끝나도 다음 부팅이 해시로 알아보고 묻지 않고 지운다.
  - **정상 종료**(트레이 종료 포함 · 모달이 떠 있어도) = 사본 그대로 · 다음 부팅이 다시 묻는다.
  - **답하기 전에 다시 죽음** = 그대로 · 다시 묻는다(D2-6).
  - **못 쓸 사본** = 부팅이 떠 두고 지운다 · 묻지 않는다 — 단 **버전 초과 사본(하향)은 지우지 않고 남겨 둔 채 묻지 않는다**(이 판은 적용 못 한다 · N6 · §12 R3). 둘 다 §6-5 표.
  - **가드 아래(기록기 없음 — §6-5 ③)** = 그래도 묻는다. 답은 화면에만 먹고 디스크에 붙지 않는다 — `durable:false` · 사본 그대로 · 다음 부팅이 다시 묻는다.
- **런타임 수락 = 복원 조율자 하나**(`state/restore.rs`). 사본은 부팅 단계가 읽은 바이트를 서비스가 쥐고 있어 다시 읽지 않는다 [고름]:
  1. **준비(부수효과 없음 — 발급기 번호만 쓴다):** 쥔 사본 → `from_persisted` → 새 창 묶음(팝아웃마다 `PopupCounter` 의 새 label — 발급 순으로 줄 세운다). `RestoreWarning::Internal` · main 없음 · 새 label 이 살아 있는 모델의 창과 겹침(발급기가 런타임 팝아웃과 다르다 — 창을 하나도 만들기 전에 거른다) = 아무것도 안 바뀌고 `awaiting` 그대로 · `INTERNAL`.
  2. **새 OS 창 생성(락 밖):** 숨긴 채 · 저장된 위치가 어느 모니터에도 안 걸치면 버리고 label 의 기본 자리로 연다(§6-5 ⑧ 의 모니터 판정 `locate` · N5 — ★`open_restored_popouts` 가 아니다★: 그것도 부팅 전용이라 살아 있는 모델의 팝아웃마다 창을 만들려다 이미 있는 label 로 실패해 그 팝아웃을 모델에서 지우고 OS 창은 남긴다). P3c1 = 창 포트 `RestoreWindows::open_hidden` → `commands/popout.rs` 의 `build_hidden_runtime_window`(`build_runtime_window` 와 같은 빌더에 보임 인자만 다르다 — 부팅 ⑨ 의 빌더는 그대로 보이는 창이다, F13) 뒤 저장된 자리에 비공개 `place`. ★사본에서 최대화였던 팝아웃은 `open_hidden(label, at, maximized)` 안에서 처음 보일 때로 미루고, 그 미룸을 창을 만들기 **전에** 세운다★ — 모델에 들기 전에 서야 ③ 뒤에 오는 그 창의 `Moved` · `Resized` 가 「최대화 아님」으로 적히지 않고, 만든 뒤에 세우면 그 사이 트레이 「보이기」가 드러낸 창을 사용자가 닫았을 때 소멸 정리가 미룸보다 먼저 지나가 미룸 항목이 남는다. 창 만들기가 실패하면 세운 미룸을 거둔다. 자리를 놓은 뒤 그 창이 이미 보이면(그 사이 트레이 「보이기」가 미룬 최대화를 입혔고 방금의 위치 세터가 그것을 내렸다 — F8) 그 자리에서 다시 최대화하고 입혀졌을 때만 미룸을 거둔다. 하나라도 실패하면 만든 것을 전부 destroy 하고 끝 — 거두기가 실패하면 한 번 더, 그래도 실패하면 error 로그(모델에 없는 숨은 창이 남고 트레이 「보이기」가 드러낸다) · `awaiting` 그대로(되돌림). 기존 `move_slot_to_window` 의 phase B(창 빌드) → phase C(모델 삽입) 규율과 같고, 그 틈의 팝아웃 페이지 당기기는 기존 재시도가 덮는다(ADR-0102) [미검 — 새 창이 모델보다 먼저 뜨는 틈].
  3. **커밋(유일한 커밋 지점 · `ViewManager` 락 하나 안):** 모델 교체(main 탭 교체 · 옛 팝아웃 항목 제거 · 새 팝아웃 삽입) · `version = 지금 version + 1` · `SubscriptionSync::resync`. 이 셋을 같은 임계구역에서 한다(재계산과 발화의 순서 — `apply.rs:73-76` 의 호출 규약). ★**`from_persisted` 결과를 살아 있는 모델과 바꿔치지 않는다**(그 결과의 번호는 의미가 없다 — §6-2)★:
     - `version = 지금 version + 1` · `attrs_rev = 지금 attrs_rev + 1` 을 직접 세운다 — ★되감지 않는다★: 기록기는 같은지만 보므로 되감긴 번호가 마지막으로 본 값과 다시 같아지는 순간 그 사이의 변경을 놓친다.
     - 곁표(모르는 내용)를 View 와 함께 옮기고 옛 View 는 `remove_view` 로 지운다(그 곁표 항목을 함께 거둔다) — View 만 옮기면 그 슬롯이 빈 칸으로 저장돼 원문이 사라진다.
     - main 항목은 탭 · 속성만 바꾼다 — 통째로 바꾸면 측정값(`canvas` · `metrics`)을 웹뷰가 다시 보고할 때까지 잃는다.
     - ★트리 칸은 이 임계구역 밖이다★ — ③ 이 `ViewManager` 락을 놓은 바로 뒤(④ 앞)에 `TreeAttrs::set` 으로 갈아끼운다(그 바로 뒤가 P3d 의 테마 밀기다 — ④ 머리)(바뀌었으면 번호를 올린다 · 두 락을 겹쳐 잡지 않는다 — §6-3 락 순서). 그래서 모델 교체와 트리 칸 교체는 한 원자 단위가 아니다 — 그 틈에 기록기가 뜬 스냅숏은 새 모델 + 옛 트리 칸을 담을 수 있다. [고름 — 받아들인 틈] 해롭지 않다: 답을 디스크에 붙이는 `Resolve` 의 스냅숏은 ⑤ 에서 기록기가 요청을 꺼낸 뒤에 뜨므로(§6-4) 둘 다 담는다.
     - `attrs_rev` · `remove_view` 는 `ViewManager` 의 비공개 칸 · 함수라 main 탭 · 속성을 통째로 바꾸고 두 번호를 올리는 메서드를 `manager.rs` 에 둔다 — 밖에서 `attrs` 를 직접 고치면 기록기가 놓친다. ✅ P3c1 = **`ViewManager::adopt_restored`**: main 은 탭 · 활성 · 속성만 바꾸고(측정값 · 영속 id 는 남는다 · 최대화 메모 `PlacementMemo` 는 비운다) · 창이 쥔 옛 View 는 `remove_view` · 옛 팝아웃은 항목째 지우고 그 label 을 돌려준다(OS 창은 ④ 가 락 밖에서 거둔다) · 새 View · 곁표 · 팝아웃을 들이고 · 두 번호를 지금 값에서 하나씩 올린다. ★창이 쥐지 않은 View(옮기기 중 `prepare_detached_view` 의 임시 View)는 그대로 둔다★ — 그 일의 phase C 가 붙이거나 거둔다. 거절(main 없음 · 팝아웃 label 이 지금 창과 겹침 · View id 가 남는 View 와 겹침)이면 아무것도 바꾸지 않고 → 만든 숨은 창을 거두고 되돌린다.
     - 같은 임계구역에서 알림 재료(창마다 탭 목록 · 탭마다 레이아웃 스냅숏)를 뜨고 `resync` 한다 — 알림은 ④ 가 락 밖에서 낸다.
     - ★**`layout::apply::create_window` 가 OS 창을 만든 뒤 모델을 다시 본다**(P3c1)★ — 창을 만드는 동안 락이 풀려 있어, 그 사이 모델에서 그 창을 지운 쪽(창 닫기 · 이 커밋)의 OS 창 거두기는 아직 등록 전인 그 창을 못 찾고 지나갈 수 있다. 다시 봐서 모델에 없으면 그 OS 창을 거두고 실패로 끝낸다(옮기기 phase C 의 재검증과 같은 틈 — 고아 창).
  4. **커밋 뒤(락 밖 · 되돌리지 않음):** (✅ P3d — 그 앞에 트리 칸 교체 → **테마 밀기 한 번** · 아래 마지막 항목) 탭·레이아웃 알림(레이아웃 변경이 쓰는 그 `LayoutEvents`) → **main · 트리 창 속성을 사본 값으로**(D2 「사본 내용을 화면에」 · ★`restore_windows` 통째가 아니다★: 부팅 전용이라 런타임에 부르면 떠 있는 팝아웃 label 로 창 만들기가 실패해 그 팝아웃을 모델에서 지우고 OS 창은 고아로 남긴다) → 새 창 보이기 → 옛 팝아웃 destroy(Destroyed 정리는 이미 모델에 없는 label 이라 재계산만 한다 — `popout.rs` `drop_window_in_model`). 여기서의 실패는 로그. P3c1 구현(`restore.rs` 의 `after_commit` · 창 포트 실물 = `placement.rs` 의 `TauriRestoreWindows` — `place` 는 그 파일 안 비공개 그대로 · 포트 `RestoreWindows` 의 모양 = `app_has_focus` · `monitors` · `open_hidden(label, at, maximized)` · `visibility` · `place_main(at, maximized)` · `place` · `set_shown(label, shown)` · `record_placement` · `focus` · `destroy`(`Result` — 창이 없으면 `Ok`). ★보이기 · 최대화 · 미룸 세우기를 따로 부르는 손잡이는 없다★ — 미룸은 `open_hidden` · `place_main` 이, 보이기와 미룬 최대화 입히기는 `set_shown` 이 한 몸으로 진다 · 넷째 라운드 전 모양에 있던 `defer_maximize` 는 걷었다):
     - **main = `place_main(at, maximized)`** — 자리 판정은 `land`(어느 모니터에도 안 걸치면 지금 자리에 둔다). ★최대화는 사본을 양쪽으로 따른다(세션 판단(사용자 위임) 2026-10-04 · 사용자 확인 2026-10-05)★ — 사본이 최대화가 아니면 미뤄 둔 최대화를 거두고 풀기까지 한다. 숨은 main 의 최대화는 처음 보일 때로 미루되 ★그 미룸을 자리를 입히기 **전에** 정한다★ — 뒤에 세우면 자리 입히기가 낸 `Moved` · `Resized` 가 그 사이 「최대화 아님」으로 적힌다. 자리를 놓은 뒤 그 사이 보이게 됐으면 그 자리에서 최대화하고 입혀졌을 때만 미룸을 거둔다. ★보이는 main 도 같은 미룸을 세운 채 입힌다(`DeferredMaximize::apply_now`)★ — 미룸을 세우고 → 자리를 놓고 → 늘 최대화하고 → 입혀졌으면 거두고, 못 입혔으면 다시 세운다(그동안 main 의 자리를 적지 않고 — 아래 `record_placement` 포함 — 다음 보이기가 다시 입힌다). 사이에 보이기가 미룸을 입히고 거둬 갔어도 최대화를 부른다 — 그 뒤의 위치 세터가 최대화를 내렸다(F8). 부팅 ⑧ 도 같은 `place_main` 을 탄다.
     - **트리 = 자리만**(최대화를 싣지 않는다 — F9) → main · 트리의 지금 자리를 다시 적는다(`record_placement` — 사본의 자리를 버렸거나 못 입혔으면 모델 · 트리 칸이 창이 간 적 없는 자리를 쥔다).
     - ★**복원한 팝아웃은 main 의 보임을 따른다(ADR-0229 트레이 의미 — 세션 판단(사용자 위임) 2026-10-04 · 사용자 확인 2026-10-05)**★ — main 이 숨어 있으면(트레이 숨기기 · `--hidden` 부팅 뒤 LLM 의 답) 숨긴 채 두고 트레이 「보이기」가 함께 드러낸다. 보이기 · 숨기기는 트레이와 같은 차례로 사용량 관심에 알린다(`set_shown`). 숨긴 채 두는 최대화 팝아웃은 자리를 적지 않는다(적으면 모델에서 사본의 최대화가 지워진다).
     - ★미룬 최대화(`DeferredMaximize`)는 label 마다이고 모든 보이기 경로가 입힌다★ — 트레이 `show_main_ui`(모든 창 — 숨긴 채 둔 최대화 팝아웃도 보일 때 최대화된다 · 세션 판단(사용자 위임) 2026-10-04 · 사용자 확인 2026-10-05) · `set_shown(true)` 가 보인 직후 입히고(`apply_deferred_maximize`), ★입혀졌을 때만 미룸을 거두고 그 창의 자리를 적는다★ — 못 입혔으면 미룸을 남기고 적지 않는다: 거두고 보통 자리를 적으면 모델에서 사본의 최대화가 지워지고 다시 입힐 길도 없다. 남기면 다음 보이기가 다시 입힌다 · 팝아웃 창 소멸이 거둔다.
     - **사라진 팝아웃** — ② 와 ③ 사이에 트레이 「보이기」가 드러낸 창을 사용자가 닫았으면 그 `Destroyed` 정리는 모델에 아직 없던 label 이라 지나갔다 → 창 포트가 「창 없음」이면 `forget_vanished`(모델에서 지우고 `resync` — 창 소멸 정리처럼 프론트에는 알리지 않는다). 그 창은 `restored_windows` 에서 빠진다.
     - 옛 팝아웃 거두기 실패 = 한 번 더 · 그래도 실패하면 error 로그(모델에 없는 창이 화면에 남는다 — §14-14 「받아들이고 고치지 않은 것」).
     - **끝에 main 에 포커스** — 보인 팝아웃마다 포커스를 가져갔으니 「복원」을 누른 사람 앞에 main 을 되돌린다(트레이 보이기가 main 을 마지막에 두는 것과 같은 까닭). ★**주는 조건 = 넷 모두**★: 창을 하나라도 만지기 전(① 뒤 · ② 앞)에 표본한 「우리 앱이 앞이었다」(`app_had_focus`) · main 이 보인다 · 모델에 남은 새 팝아웃이 있다(`kept > 0`) · 부르기 바로 앞에서 다시 읽어도 우리 앱이 앞이다. 다른 앱이 앞이면 포커스 주기가 그 앱에 가짜 Alt 키를 쏴 포커스를 뺏는다(tao 0.35.3 Windows `set_focus` → `force_window_active` — `SetForegroundWindow` 가 거절될 때만 Alt 를 쏜다) — LLM 이 뒤에서 답하는 동안 사람은 다른 앱에서 일하고 있을 수 있다. 두 번 읽는 까닭: 앞 표본만으로는 ②–④ 사이에 사람이 다른 앱으로 옮긴 것을 놓치고, 뒤 표본만으로는 보인 팝아웃이 활성화를 가져가 창을 만지기 전에 다른 앱이 앞이었던 것을 놓친다.
     - **「우리 앱이 앞인가」 = 포트 `app_has_focus` → `placement.rs` 의 `foreground_is_ours` 한 함수**(OS 갈림은 OS 층 crate `platform` 의 `window::foreground_is_current_process` 안의 `cfg` 하나 — 그 함수가 `None`(답할 수단 없음)이면 셸이 포커스 게터로 판정한다 · CLAUDE.md 「플랫폼 중립」 · ADR-0266 · master 머지 2026-10-07 에 셸의 `cfg` 를 옮겼다): Windows = 전경 창(`GetForegroundWindow`)의 프로세스(`GetWindowThreadProcessId`)가 우리 PID 인가 · 그 밖 = 창의 포커스 게터(`is_focused`) 중 하나라도 `true`. ★**Windows 에서는 `is_focused` 를 쓰지 않는다**★ — wry 0.55.1 이 창의 `WM_SETFOCUS` 마다 포커스를 웹뷰로 넘겨(`MoveFocus`) 창은 곧바로 `WM_KILLFOCUS` 를 받고, tao 0.35.3 의 그 값(`is_active && is_focused`)은 웹뷰가 키보드 포커스를 쥔 동안 `false` 다 — 우리 앱이 앞이어도 「아니다」로 읽힌다(tauri-runtime-wry 2.11.3 도 그래서 창의 포커스 사건을 웹뷰의 GotFocus · LostFocus 에서 만든다) [미검 — 소스 독해]. 의존성 = `platform` 의 `cfg(windows)` `windows` 0.58 에 `Win32_UI_WindowsAndMessaging` 을 더했다(셸에는 `windows` 의존이 없다 — CI `platform gate 5` · CLAUDE.md 「의존성」).
     - ~~**테마는 밀지 않는다** — 수락 뒤 창별 테마 밀기는 창별 테마가 상태로 들어오는 P3d 몫이다.~~ → ✅ **P3d: 수락은 테마를 한 번 민다 — ③ 커밋 · 트리 칸 교체 뒤, ④ 의 레이아웃 알림 · 창 자리 입히기 · 보이기 앞**(`state/restore.rs` 의 수락 · `ThemeControl::push`). 사본의 창 테마가 모델 · 트리 칸에 든 뒤라 떠 있는 main · 트리 창과, 커밋 전(모델에 들기 전)에 첫 값을 당긴 새 팝아웃이 사본의 값을 받고, 보이기 앞이라 새 팝아웃이 옛 값으로 비치지 않는다. 커밋 전에 실패한 수락은 아무것도 밀지 않는다. 배달 실패는 로그만 남고 수락의 답을 가르지 않는다.
  5. **확정:** 상태 `answered` · `restore:changed` → 기록기에 `Resolve { hash }`(기록기가 꺼낸 뒤 뜬 스냅숏은 커밋을 담는다) → 마감(2초)까지 답을 기다린다. 답의 `durable` = 마감 안에 그 해시를 실은 쓰기가 성공했나(`Written` 일 때만 — 사본 지우기의 성패는 담지 않는다). P3c1 = `StateSession::resolve_crash_copy(hash)`(`state/boot_plugin.rs`):
     - ★기록기가 아직 없으면 먼저 ⑦(`start_saver`)을 부른다★ — ~~설정 창은 사용자 setup 보다 먼저 만들어져(§6-5 사실 ③) 그 창의 invoke 가 ⑦ 전에 올 수 있다~~ ★P4 부터 창은 ⑦ 뒤 ⑧ 에서 생겨 오늘의 부팅 차례로는 답이 ⑦ 전에 오지 않는다 — 방어로 남긴다(그 함수 주석)★. 가드 · 이미 띄움 · 종료 뒤면 그 부름은 아무것도 하지 않는다.
     - ★다른 스레드가 띄우는 중이면 실리거나 실패할 때까지 기다린다(조건 변수 — 칸 락은 놓고 선다)★ — 그 기다림과 `Resolve` 답 기다림이 마감 하나를 나눠 쓴다.
     - ★종료(`shutdown`)는 셸 실행 잠금을 놓기 **전에** 띄우는 중인 기록기의 닫힘 표지(`CloseFlag`)를 세운다★ — 안 세우면 아직 칸에 실리지 않은 기록기가 잠금을 놓은 뒤에 써서, 그사이 관문을 지난 새 인스턴스의 파일을 덮는다. 띄우는 사이 종료가 돌았으면 띄운 쪽이 그 기록기를 싣지 않고 닫는다.
     - 아무 락도 쥐지 않고 기다린다 — 기록기가 스냅숏을 뜨려고 레이아웃 · 트리 칸 락을 잡는다.
  - **거절** = ①–④ 없이 ⑤ · `restored_windows: 0`.
- **크래시 루프 격리:** 복원한 화면이 앱을 죽이면 — 확정 쓰기 전이면 사본이 남아 다시 묻고, 확정 뒤면 그 화면을 담은 `state.json`(`clean_exit:false`)이 새 사본이 되어 묻는다. 「새로 시작」으로 끊는다. 조용히 복원한 화면이 창을 만들다 죽는 경우는 표식이 어느 창보다 먼저 서서 잡는다(§6-5 ⑤).

### 6-8. 대상 에이전트가 없는 슬롯 — 꺼진 에이전트는 부재 막 · 지운 에이전트는 「대상 없음」 (✅ P3c3 `e904161` · ADR-0280)

복원은 배치와 `agent_id` 를 그대로 둔다 — 그 에이전트가 다시 뜨면 슬롯이 알아서 붙는다. 판정과 표시는 잎(`LayoutLeaf.tsx`)이 한다 — 정본 = 그 파일과 ADR-0280:

| 판정 | 표시 |
|---|---|
| 명부·프로필 목록을 둘 다 받기 전 | 「에이전트 연결 중…」 — **이 경우에만.** 목록이 오면 반드시 아래 셋 중 하나로 넘어간다(F16) |
| 프로필 있음 · 실행 중 아님(`reserved`) · 기억 없음 | **부재 막** — 공용 `SlotUnavailableVeil`(흐림 + 전원 꺼짐 아이콘뿐 — 글 없음 · 단추 없음 · 연결 끊김 · 죽음과 같은 모습). 잎이 슬롯 컴포넌트를 띄우지 않을 때만 그린다. **슬롯 활성화 단추는 없다** — 트리에서 활성화하면 슬롯이 스스로 평소 화면으로 붙는다(사용자 결정 2026-10-06 · ADR-0280 결정 1) |
| 프로필 없음(목록 수신 뒤) | **「대상 없음」** 문구 — 배치 유지(문구의 출처 = 2026-10-02 사용자 결정 「대상 없음」 대기 상태 — `docs/research/storage-management-survey-2026-10-02.md:19` · ADR-0149 (C) 의 문구를 대신한다). **원래 에이전트 슬롯 메뉴 그대로(「비우기」 포함)**: 「비우기」로 비운 뒤 내용을 놓거나 · 「에이전트 모니터링」으로 바로 덮어쓰거나 · 닫는다(메뉴 = 사용자 결정 2026-10-06 · ADR-0280 결정 3) |
| 기억 있음 + 부재(이 세션에서 죽음) | ADR-0149 (B) 그대로(흐림 · 입력 막음) |

- 트레이 종료 뒤 데몬이 에이전트를 자동으로 띄우지 않으므로 둘째 줄이 복원 직후의 흔한 경우다. 그 밖에 팝아웃 · 슬롯 id 가 바뀌어 다시 마운트될 때 · A → B → A 재배정(재배정은 기억을 버린다)도 둘째 줄이다. ADR-0149 (A) 의 「스폰 대기도 연결 중」은 첫째 줄로 대체된다 — 갓 만들었거나 갓 활성화한 에이전트의 슬롯은 명부에 오르기 전 잠깐 부재 막이 보일 수 있다(수용 = 세션 판단 · §12 R10 · ADR-0280 결정 2).
- ~~「정지됨」 + 「활성화」 단추~~(P3c3 범위 변경 전까지의 둘째 줄) → **걷었다(사용자 결정 2026-10-06 · 이유 = ADR-0280 결정 1).** 사실: 구현하자 트리 · 슬롯 활성화 상태 맞추기가 리뷰 3 라운드 동안 새 결함을 냈다(§14-18). 그 줄의 [고름](활성화 진행 표식이 트리 지역 상태)은 쓸모가 없어졌다. 미룬 것 = 슬롯 쪽 활성화 조작과 완전한 이어받기(`docs/tracking.md` T-49).
- **포커스 링은 모든 막 위에 그린다** — 링 색이 슬롯 상태로 바뀌지 않게(사용자 결정 2026-10-06 · ADR-0280 결정 5). 구현 = z 20 · `[data-slot-border]` 의 마지막 자식. 잔여(사용자 수용): 링이 반투명(강조색 40% — ADR-0168)이라 보이는 색이 바탕을 따른다 — 터미널 슬롯은 어두운 바탕 고정이라 라이트 · e-ink 에서는 산 터미널 슬롯과 막이 덮인 슬롯의 링이 다르게 보인다(다크는 같다 — §14-18).
- 모르는 내용 슬롯(곁표 슬롯)의 표시 · 메뉴 = §6-2.
- 문구 = `i18n/ko.ts`(`agent.connecting` · `agent.noTarget` · `slot.foreignContent`(아이콘의 이름 · 툴팁) · ~~`slot.foreignContentHint`~~(2026-10-06 걷음 — ADR-0280 결정 7) — `agent.noneConnected` 「연결된 에이전트가 없습니다」는 걷었다) · ADR-0149 개정 = ADR-0280(§11).

## 7. 슬롯 종류별 공통 설정 — 자리만

`shell\config\slots\<종류>.json` 은 **폴더 자리를 이 문서에만 둔다.** 코드·경로 함수·빈 파일 추상화는 만들지 않는다. 첫 `slot.<종류>.*` 키를 표에 올리는 변경이 저장 계층 안에서 그 파일로 가르는 규칙을 함께 넣는다(§5-3 — 호출부 무수정). 슬롯에 꽂힌 내용(`Usage` 의 `show_claude` 등)은 상태다.

## 8. seam · 시험 (ADR-0012)

| 모듈 | 끊는 것 | 하네스 · 새 시험 |
|---|---|---|
| `DataLayout` | 없음(순수) | root → 각 경로(옛 평면 경로 없음) |
| 잠금 | net 기존 하네스 | 새 자리 `daemon\run\daemon.json` 획득 · 새 데몬 둘 경합 = 뒤엣것은 파일을 하나도 안 건드리고 끝난다(`AlreadyRunning` 또는 — 앞엣것이 발행 전이면 — `FileBusy` 둘 다 허용 · §3-1 ③) · 옛 루트 파일(평면 `agents.json` · `daemon.json`)이 있어도 읽지도 바꾸지도 않는다 |
| 설정 | 파일 = `SettingsFiles` 트레이트 · 알림 = 포트 | 종류별 정규화 표(§5-2) · 같은 유효 값 = 파일이 목표대로면 무쓰기 · 다르면 파일만 바로잡고 `rev` · 알림 없음 · 기본값이면 키 삭제 · 모르는 키 보존 · 못 읽는 파일 무손상 + 첫 쓰기 `.corrupt` · RMW 가 남의 키 보존 · `rev` 단조 · 명령 표(`src-tauri/tests/layout_commands.rs` 가짜 포트) · ~~**L2 의 `ui-settings.json`: `theme` 가 없거나 못 쓸 값이어도 유효한 `windows` 는 그대로 적용된다(§3-5)**~~(L2 한정 — P3d 가 그 파일을 읽던 `ui_settings.rs` 와 함께 걷었다) |
| 프론트 설정 | `invoke`/`listen` 모의 | 구독 먼저 · 키마다 `rev` 로 적용(처음 보는 키는 그대로 · `settings_get` 답도 항목마다) · 챗 스타일 적용(11키 그대로 — U2) |
| fsutil | 파일 = 임시 폴더 | **rename 직전 확인 변형: 확인이 서면 첫 rename 전이든 잠김 재시도 사이든 「건너뜀」(`Err` 아님) · 임시 파일 지움 · 재시도마다 다시 묻는다(I8)** · 떠 두기 = 고정 이름 하나 · 앞선 사본을 덮어씀 · 원본 없음이면 아무것도 안 건드림 · 읽기 전용 원본도 쓸 수 있는 사본으로(ADR-0274) · 같은 대상의 임시 이름 둘이 겹치지 않음 · 읽기 잠김 재시도 5 × 20 ms(I3 — 열기 · 읽기 도중) · FNV-1a 고정 값 |
| 상태 코덱 | 없음(순수) | **`Unknown` 원문 보존 왕복: 적재 → 다른 슬롯 편집 → 재저장 → 원문 JSON 동일** · 그 슬롯에 내용을 넣으면 원문 소멸 · **곁표 슬롯 = 점유(`slot_is_free` · `resolve_spawn_slot` 이 건너뜀) · `foreign_slots` 에 실림** · 버전 초과 · 4 MiB 초과 = 못 쓸 파일 · `version` `1.0` = 1 · `u64` 범위 밖 = 상태 파일 아님 · 분할 120겹은 읽히고 121겹은 쓰기 거절(`Unparsable`) · 항목별 건너뛰기 · `from_persisted` 불변식 · `resolved_crash_copy` 왕복 · ~~**(P3e) 보기 모드 칸 왕복 · 내용이 바뀌면 그 칸이 사라짐 — 시험 모양은 P3 착수 전 설계(§6-0) 뒤 확정**~~(P3e 를 뺐다 — 사용자 결정 2026-10-07 · §14-20) |
| 잠금 `state.lock` | 시계 = 인자 | `shell\run` 이 없으면 만들고 잡는다 · 잡힌 잠금 → 약 3초 재시도 뒤 잠금 없이 진행(log) · 그 사이 놓이면 바로 잡음 · 쥔 핸들을 놓으면 풀림 · 임시 파일: 자기 pid · 죽은 pid 만 지우고 산 남의 pid 는 남김 · **가드(기록기 없음) → 종료가 `Final` 없이 잠금만 놓는다(N7)** |
| 부팅 판정 · 부팅 단계 | 없음(순수) · 파일 = 포트 | §6-5 표 전 행 · **비정상 → 사본 = 그 `state.json` 원문 바이트 · 사본 쓰기가 첫 쓰기보다 먼저** · **기본 화면으로 비정상 종료 → 사본 · 묻는다(D5)** · **사본 + `clean_exit:true` → 기본 화면 · 묻는다(D6)** · **사본이 있는 채 다시 강제 종료 → 사본 바이트 그대로 · 다시 묻는다** · **답한 사본(해시 일치) → 묻지 않고 지움 · 못 지우면 해시를 넘김** · **읽기 IO 실패 → 그 파일의 동작 0회 · 다른 파일은 제 읽기대로(I3 — 파일마다): 사본 IO 실패 + 못 쓸 `state.json` → 그래도 떠 둔 뒤 ⑤ · `state.json` IO 실패(가드 ⅰ) + 못 쓸 사본 → 떠 둔 뒤 지움 · 사본 IO 실패 + `clean_exit:true` + 해시 → 해시를 이어 실음 · 사본 IO 실패 + `clean_exit:false` → 가드 ⅱ · 묻는다** · **떠 두지 못한 못 쓸 사본 → 안 지움(새 사본을 뜰 행은 갈아끼움)** · ★**가드(N3): `state.json` IO 실패 또는 사본 못 뜸 → ⑤ 0회 · 기록기 없음**★ · **답한 사본은 지우기 직전 해시 재확인 — 다르면 안 지움(N1)** · **버전 초과 사본 → 그대로 · 묻지 않음 · 새 사본 안 뜸(N6)** · **못 쓸 `state.json` + 답 없는 사본 → 떠 둠 + 모달(N6)** · **setup 은 어느 실패에도 `Ok`(N7)** · **가드 ⅱ(사본 못 뜸) → `awaiting` · 복원 원천 = 메모리의 `state.json` 원문 · 디스크 `state.json` 바이트 그대로(L2)** · **(P3c2) `state_aside` = `state.json` 떠 두기의 결과만(사본 떠 두기와 섞지 않는다) · `state_file` = 가드 ⅰ → `unreadable` · 떠 둠 → `corrupt_copied_aside` · 못 떠 둠 → `corrupt_not_copied` · 가드 ⅱ · 읽음 · 없음 → `ok` · 사본이 없어도 ⑥ 이 정한다** · **(후속 `125d190`) `saves` = 가드가 아니다 — 못 쓸 `state.json` · 떠 두기 실패는 `true`(D8) · 가드 ⅱ 면 답한(거절) 뒤에도 `saves:false` · `state_file:'ok'`** · **팝아웃 위치가 어느 모니터에도 안 걸치면 버림(N5)** · 못 쓸 파일 → `.corrupt`(고정 이름 · 덮어씀) · 묻지 않음 · **복원 서비스 상태가 첫 쓰기와 같은 단계에서 정해진다(I5)** |
| 기록기 | 시계 · 스냅숏 원천 · 파일 = 포트 | 디바운스 1초 · 상한 5초 · 무변경 무쓰기 · **코덱 거절(4 MiB 초과 · 다시 읽히지 않는 중첩 `Unparsable`) → 안 씀 · `Failed` · 다음 변경까지 다시 안 씀** · **스냅숏 포트 실패 → 디스크 실패처럼 조용함 뒤 다시** · **실패 로그는 갈래마다 접힘** · **`Final` 마감 초과 → `closed` 뒤엔 rename 0회 · 사본 삭제 0회(갇힌 쓰기를 풀어 줘도 발행 없음)** · **쓰기와 사본 지우기 사이에 닫히면 사본 삭제 0회**(시험 있음 — P3a) · **해결 칸은 꺼낼 때 선다(I8): 보내기만 하고 아직 안 꺼냈으면 그사이 쓴 스냅숏엔 해시가 없다** · `Resolve` 쓰기 실패 → 다음 성공 쓰기가 해시 싣고 지움 · 지우기 실패 → 뒤 쓰기마다 다시 · **지우기 직전 해시 재확인: 사본 바이트가 칸 해시와 다르면(새 사본) 안 지우고 칸을 비움(N1)** · 지워지면 해시를 더 안 싣는다 · 쓰기 실패 = log 만(D8) |
| 복원 서비스 · 조율자 | 창 = `RestoreWindows` 포트(P3c1 신설 — 운영 = `TauriRestoreWindows`) · 알림 = `LayoutEvents`(기존) · 구독 = `SubscriptionSource` → `SubscriptionSync`(기존) · 기록기 = `StateSession` | 상태 셋(`none` / `awaiting` / `answered`) · `awaiting` 아니면 `answer` → `CONFLICT` · 답한 뒤 두 번째 → `CONFLICT` · **처리 중 두 번째 → `CONFLICT`** · **표지를 버리거나 패닉으로 풀리면 되돌림 · 되돌림도 알림(`awaiting`) · 알림은 락 없이** · **포트를 꽂기 전 수락 → `INTERNAL` · 거절은 선다** · **② 와 ③ 사이에 닫힌 팝아웃 → 모델에서 지움** · **main 이 숨었으면 복원 팝아웃도 숨김 · main 최대화는 사본대로** · **끝의 main 포커스는 창을 만지기 전(① 뒤 · ② 앞) · 부르기 직전 모두 우리 앱이 앞일 때만** · **커밋 거절 · 거두기 실패(한 번 더) → `awaiting`** · **버스 `crash_copy` 철자 = serde 철자** · **떠 있는 팝아웃이 같은 옛 label 을 쥔 채 수락 → 충돌 없음 · 새 label · 옛 창 destroy** · 창 생성 실패 → 되돌림 · `awaiting` 그대로 · `version` 이 정확히 +1 · resync 1회 · 수락은 main · 트리 속성도 · 수락 때 화면 밖 팝아웃 위치 버림(N5) · 가드(기록기 없음) → `durable:false` · 사본 그대로 · **가드 ⅱ: 메모리 원문으로 수락 → 화면 복원 · `durable:false` · 다음 부팅이 다시 묻는다(L2)** · **기록기 쓰기 실패 → `durable:false` · 해결 칸 유지** · **(P3c2 · 후속 `125d190`) `restore.status` 의 `durable` = `awaiting` 동안(처리 중 포함) `saves` 와 같은 값 · 그 밖 `null` · `saves` · `state_file` 은 사본이 없어도 서고 답해도 · 되돌려도 그 실행의 값 그대로 · 버스 철자 = serde 철자 · 버스 답에 `saves` 가 늘 있다(`layout_commands`)** |
| 사본 수명 | — | **정상 종료(모달이 떠 있음) → 사본 그대로**(D6) · 거절 → 해시 쓰기 → 삭제 · **답한 뒤 지우기 전에 강제 종료 → 다음 부팅이 묻지 않고 지움** |
| 프론트 모달 | vitest(`RestoreModal` · `restoreClient` · `StateFileNotice` · `keybindings` · 덮는 층 = `NoticeOverlay` · `ConnectionNotice` · `AppLayout` · `PopoutPage` · `TreePage` — IPC = 가짜 `RestoreIpc`) | `awaiting` 이면 main 을 덮고 입력을 막는다(그 아래 레이아웃 `inert`) · **그동안 키 바인딩 디스패처가 멈춘다 — 단축키가 그 아래 화면을 못 바꾼다 · 답 뒤 다시 돈다(N4)** · 답 뒤 닫힘 · `restore:changed` 로 갱신 · 첫 당기기가 `awaiting` 이면 바로 그린다 · **(P3c2) 누르는 순간 닫지 않는다(다시 당긴 상태로만) · 실패면 열린 채 다시 답한다 · 포커스를 안으로 옮기고 Tab 을 가둔다 · 거절 경고는 `durable` 로 가른다(`state_file` 을 보지 않는다) · 저장 시각이 `Date` 범위 밖이면 그 줄만 뺀다 · 창구: 구독 먼저 · 나중에 낸 당기기가 이긴다 · 내린 설치의 답은 칠하지 않는다 · 부팅 당기기는 받을 때까지 · 상태 파일 안내: `ok` · 미수신이면 없음 · 닫으면 다시 안 뜬다** · **(후속 `125d190`) 가드 ⅱ 안내(`ok` + `saves:false`) · 안내는 레이아웃 칸 위를 덮는다(층 = absolute · 칸의 자리 그대로(크기는 GUI QA) · 높이 · 아래 끝을 정하지 않음 · 넘치면 스크롤 · 층 안에서도 ✕ 로 닫힘) · ~~연결 띠는 층 밖에서 레이아웃 칸 위에 쌓인다~~(`9318b5c` 에서 걷힌 단언) · 층은 잠기는 감싸개 안이고 모달은 그 위(z-50 > 40) · 창구: `saves` · `state_file` 을 모르면 `undefined`(한 번 경고) · `crash_copy` 를 못 읽으면 실패한 당기기** · **(`9318b5c` · ADR-0277) 층의 계산된 위치 = absolute · 기준 칸의 맨 위 · 좌우 끝 · z 40 · 알림이 없으면 층이 빈다 · 여럿이면 준 차례로 쌓고 줄 사이를 1px 틈의 앱 바탕으로 가른다 · main = 두 알림이 한 층에 연결 띠 → 상태 파일 안내 차례 · 각각 ✕ 로 닫힘 · 묻는 동안 두 ✕ 도 잠김 · 세 창(main · 팝아웃 · 트리)에서 알림이 떠도 레이아웃 · 내용 · 목록 칸의 자리가 그대로(흐름 안의 자식은 그 칸뿐 — 트리는 머리줄 + 목록 칸 · 칸은 `flex:1` · `minHeight:0` · `relative` — 크기는 재지 않는다 · 크기 불변은 GUI QA §14-17) · 줄 계약 = 불투명 바탕 · `wrap-anywhere`(두 안내 모두) · 연결 띠: 닫은 뒤 같은 이유로 다시 통지돼도 안 뜬다** |
| 창 위치 | 모니터 목록 = 인자 | 최소화·최대화 중 기록 안 함 · 화면 밖 버림 · 위치·테마 변경은 `version` 을 안 올린다(`attrs_rev`) · **미룬 최대화는 label 마다 · 입혀졌을 때만 거둔다(못 입히면 다음 보이기까지 남는다) · 보이는 창은 미룸을 세운 채 놓고 입힌다**(P3c1) |
| 유효 테마 밀기 | emit = 포트 | `theme.default` 쓰기와 `window.setTheme` 이 엇갈려도 마지막 밀기 = 둘 다 반영한 값 · ✅ P3d(`theme.rs` 시험 — 창 포트 = 가짜): 창마다 자기 유효 값 · 부팅 당기기와 밀기가 같은 값 · 트리 창은 트리 칸에, 그 밖은 모델에 · 모르는 창은 읽기 · 쓰기 거절이되 부팅 당기기엔 답한다 · 지목한 창이 못 받으면 오류(값은 남는다) · 다른 창만 못 받으면 성공 · 못 받은 창은 수를 다 세되 자세히는 셋까지 · 모델 락 없이 보낸다 · 부팅 ⑨ 뒤 밀기(`state/placement.rs` `open_popouts_then_push_themes`) · 수락이 사본의 창 테마를 민다(`state/restore.rs`) |
| 슬롯 두 상태 | vitest(`ViewLayoutRenderer.test.tsx` — 잎(`LayoutLeaf`)을 렌더러째 잰다) | 미수신 / `reserved` / 프로필 없음 / 기억 있음 네 갈래 · 「연결 중」이 목록 수신 뒤 남지 않음 |
| Tauri 결합부 | — | GUI 실측만(`/qa full`): 정적 창 생성 · 웹뷰 폴더 · ~~쓰기 실패 대화상자~~ 쓰기 실패 물러남(M4) · main 생성 실패 종료(✅ P4 — §14-21 · 순수 부분 = `state/placement.rs` 의 정적 창 목록 · 차례 · `confirm_created` 와 `webview_env.rs` 의 폴더 판정은 `lib_unit` 시험) · 팝아웃 복원 · 로그오프 · **부팅 단계 플러그인이 관문 뒤 · 어느 창보다 먼저 도는지** · **모달: `taskkill /F` → 다음 기동에 모달 → `engram restore.answer` → 사본 사라짐 · 모달이 떠 있는 채 트레이 종료 → 다음 기동에 모달 다시 · `--hidden` 기동 → 트레이로 열면 모달 · 모달이 뜬 동안 단축키가 안 먹는다**(✅ P3c2 GUI G1–G8 — 트레이 메뉴 클릭 자체는 같은 처리기 `quit_app` · `show_main_ui` 를 invoke 로 불러 대신했다 · §9-2 P3c2 행) · **트레이 종료 직후 재실행 → 잠금을 기다린 뒤 조용히 복원** |

명령은 CLAUDE.md 「빌드·검증 명령」 그대로(셸 단위 = `--test lib_unit` · 데몬·discovery·base = `-- --test-threads=4`).

## 9. 단계 계획 — 매 단계 끝에 빌드·시험 초록

### 9-1. 순서 · 머지 단위

- **설정(L2)이 상태(L3)보다 먼저:** 상태 영속이 들어오면 ADR-0167 부팅 쓸기의 전제(「부팅 때 팝아웃 0개」)가 무너진다. L2 동안 창별 칸은 옛 파일 + 쓸기로 여전히 옳게 돌고, L3 가 그 칸의 집을 상태로 바꾸면서(옛 값은 다루지 않는다 — U1 · §3-5) 쓸기를 끈다.
- **웹뷰(L4)는 맨 끝에 둔다** — 3판의 근거(챗 스타일 가져오기가 옛 웹뷰 폴더 안에서만 된다)는 U1 로 사라졌다. P2b 부터 localStorage 를 아무도 읽지 않아 순서 제약은 없고, 3판 순서를 그대로 둔다 [고름].
- ★**P3 착수 전 게이트 — `/review trd` 재리뷰**★: 5판 재리뷰(설계자 BLOCK · 설계자-파괴자 FIX) 반영과 사용자 결정 D4–D8 = **6판**(§14-10). 6판 재리뷰(`/review trd full`) = 설계자(codex) PASS(Tauri 소스로 플러그인 순서 확인) · 설계자-파괴자 FIX(N1–N8) → 반영 → PASS · 마무리 문구 L1–L3 반영 — 다시 리뷰할 필요는 없다고 판정했다(`ae3f6e7` · §14-11). **★게이트 통과 — P3a 착지 `41348d9`(§9-2)★**. ★게이트는 P3 전체에 걸린다 — P3a 의 기록기 · `fsutil` 변형도 이 설계에 묶인다★. ~~같은 자리에서 슬롯 보기 모드(§6-0 ①–⑦ · F19)도 설계한다.~~(P3e 를 뺐다 — 사용자 결정 2026-10-07 · §14-20) P2 는 이 게이트와 무관하다.
  - 경위: 이 게이트는 원래 4판의 「최근 3 개 회전 보관」(F17)을 출처로 확인하고 설계를 다시 쓰려고 걸었다. 그 확인이 전제를 무너뜨려 F17 을 철회했다(§14-9).
- **P3 단계 자르기(I7 · N8):** 파일 겹침으로 순차로 자른다 — 상태 코어 + `fsutil`(P3a) → `manager.rs` 곁표(P3b1) → 순수 부팅 판정(P3b2) → `lib.rs` 배선(P3b3) → 크래시 사본 + 답할 길(P3c1 — Rust) → 복원 모달 · 키 바인딩 멈춤 · 상태 파일 안내(P3c2 — 프론트 · `restore.status` 의 칸 둘은 Rust) → 슬롯 두 상태(P3c3) → 창별 테마(P3d) → ~~보기 모드(P3e)~~(뺐다 — 사용자 결정 2026-10-07 · §14-20). ★**사본 만들기와 답할 길은 같은 단계(P3c1)다**★ — `restore.answer` 가 버스에 서면 충족된다(P3c2 의 모달 전에도 `engram restore.answer` 로 답한다). 갈리면 개발 데이터에 답할 길 없는 사본이 눌어붙는다. 그래서 P3b2–P3b3 의 비정상 종료는 한시로 「기본 화면 · 사본 없음」이다(L3 안에서만 — master 에 서지 않는다). P3a 는 `lib.rs` 에 `mod state;` 한 줄만 둔다(배선 없음) — `lib_unit` 이 그 시험을 컴파일하고 돌린다(P3c1 에서 `pub mod state;` 가 됐다 — 통합 시험 `src-tauri/tests/layout_commands.rs` 가 복원 조율자를 짓는다). 단계마다 혼자 빌드 초록.
- **머지 단위:** 단계는 코더 한 명 몫이고 매 단계 초록이지만, **master 머지는 단위 끝에서만** 한다 — L2 = P2a+P2b+P2c(명령과 도움말이 같이 나간다) · L3 = **P3a–P3d**(기록·복원·크래시 사본·창별 테마·쓸기 제거가 한 덩어리로 — ~~P3e 보기 모드~~는 뺐다(사용자 결정 2026-10-07 · §14-20) — P3d 착지로 L3 는 끝났고 지금 master 로 머지한다 · 기록만 있고 읽는 쪽이 없는 상태, 복원은 됐는데 쓸기가 남은 상태가 master 에 서지 않는다). **L1–L4 전체가 한 릴리스(v0.3.3)로 나간다.**
- 단계 안에서도 **중간에 멈춰도 빌드가 서게**: 새 모듈·타입을 쓰는 쪽 없이 먼저 넣고 호출부를 한 단위씩 옮긴다. 자료구조를 먼저 갈아엎고 호출부를 나중에 맞추는 순서는 금지.

### 9-2. 단계

| 단계 | 내용 | 파일(겹침 기준) | 크기 |
|---|---|---|---|
| **P1** (L1) 데이터 배치(옛 데이터 버림) | §2 · §3-1~3-4 | discovery `src/lib.rs` · `src/layout.rs`(신설) · `tests/stop_smoke.rs` · net `src/instance.rs`(+시험) · base `src/logging/mod.rs` · `tests/logging_fallback.rs` · `tests/logging_install_race.rs` · 데몬 `src/lib.rs` · `src/control/{mod.rs,mcp_config.rs}` · `src/bin/priming_smoke.rs` · `tests/ws_e2e.rs` · `tests/mcp_manager_lifecycle.rs` · `src-tauri/src/lib.rs`(로그 인자 한 줄) · `.gitignore` · `scripts/{engram.mjs,rebuild-run-debug.bat,rebuild-run-release.bat,run-release.bat,build-release.ps1}` · CLAUDE.md 모듈 맵 | M (이전 걷음으로 3판 추정 ~900줄보다 작다 — §14-5) |
| **P2a** (L2) 설정 코어 + 전역 테마(기본값에서 시작 — U1) | §5-1~5-4 · §5-6 전역 · §3-5 전역 | `src-tauri/src/settings/{mod,registry,store}.rs`(신설) · `src-tauri/src/fsutil.rs`(신설) · `ui_settings.rs` · `commands/settings.rs` · `commands/layout.rs`(`command_ports`) · `layout/commands.rs` · `lib.rs` · `src-tauri/tests/layout_commands.rs` · `src-tauri/bindings/*` | L (~1,000) |
| **P2b** (L2) 프론트 챗 스타일 — 지금 11키 그대로(U2) | §5-5 · §5-1 U2 | `src/api/settingsClient.ts`(신설) · `src/store/chatStyleStore.ts` · `src/main.tsx` · 해당 `*.test.ts` · `docs/reference/architecture-overview.md`(`:578` 예외 목록) | S (~250 — 가져오기(U1)와 `theme.css` 대조 시험(U2)을 걷어 3·4판 ~400 에서 줄었다) |
| **P2c** (L2) 도움말 · 프라이밍 | §5-7 1차 | `prompts/engram-help.md` · `prompts/agent-priming.md` · 데몬 `src/bin/engram.rs` · CLAUDE.md 「LLM-우선 제어」 갭 줄 | S (~200) |
| **P3a** (L3) 상태 코어 + `fsutil` — **배선 없음**(`lib.rs` 엔 `mod state;` 한 줄 — `lib_unit` 이 시험을 컴파일하고 돌린다) · ✅ **착지 `41348d9`(2026-10-03)** — 리뷰 반영 = §14-12 | §6-1 · §6-2 · §6-4 | `src-tauri/src/state/{mod,schema,codec,saver}.rs`(신설 — 기록기 요청 둘 · 해결 칸 · 4 MiB · 중첩 거절) · `src-tauri/src/fsutil.rs`(rename 직전 확인 변형 「건너뜀」 · 원자 복사 `copy_atomic` · 떠 두기 `copy_aside`(고정 이름 · 덮어씀 — ADR-0274) · 임시 이름 `<이름>.tmp<pid>.<n>` · 읽기 잠김 재시도 · FNV-1a) · `src-tauri/src/settings/store.rs`(떠 두기를 `fsutil` 에서 부른다 — F21 로 한도 · 중복 생략을 걷었다) · `src-tauri/src/lib.rs`(`mod state;`) | M–L (~800 — 실제 +3,307 / −797 줄, 시험 포함) |
| **P3b1** (L3) `manager.rs` 곁표 · 창 속성 칸 · ✅ **착지 `5a2cf7f`(2026-10-03 · P3b2 와 한 커밋)** — 리뷰 반영 = §14-13 | §6-1 · §6-2 · §6-3 | `layout/manager.rs`(`WindowTabs.window_id` · `attrs` · `attrs_rev` · 곁표 · 내용 쓰기 문 `write_slot_content` · 뷰 지우기 문 `remove_view` · `slot_is_free` · `resolve_spawn_slot` 메서드화 · `from_restored`) · `layout/tree.rs`(`assign_in_tree` 걷음 — 내용 쓰기는 `set_in_tree` 하나) · `layout/types.rs`(`ViewSnapshot.foreign_slots`) · `layout/{apply,mod}.rs`(`spawn_into` 가 메서드를 부른다 · 재노출) · `state/convert.rs`(신설 — `to_persisted` · `from_persisted` · `StateRevision`) · `state/tree_attrs.rs`(신설) · `state/{schema,codec}.rs`(영속 `bounds` = `Option`) · `state/{mod,saver}.rs` · 바인딩 `ViewSnapshot.ts` · 프론트 시험 고정물 셋(`foreign_slots` 칸 — `WindowLayout.test.tsx` · `useSplitDrag.test.tsx` · `viewStore.test.ts`) | M (~500 — 실제 = P3b2 와 합쳐 +4,113 / −181 줄, 시험 포함) |
| **P3b2** (L3) 순수 부팅 판정 · 잠금 · ✅ **착지 `5a2cf7f`(2026-10-03)** | §6-5 ①–⑤ | `state/{boot,lock}.rs`(신설 — `decide_boot` · ⑤ 실행 표식 쓰기 `write_run_marker` · 잠금 대기 · 파일 = 포트) · `fsutil.rs`(임시 이름 해석 · 쓸기 `sweep_temps` — 자기 · 죽은 pid 만 · §6-5 ②). ★한시: 사본 열이 없다 — 비정상 = 기본 화면 · 동작 없음(P3c1 이 더한다)★ | M (~400) |
| **P3b3** (L3) `lib.rs` 배선 — 조용한 복원 · 기록기 · 종료 · ✅ **착지 `f9db35a`(2026-10-03)** | §6-5 ⓪ · ⑤–⑩ · §6-6 · §6-3 기록 | `state/boot_plugin.rs`(신설 — 부팅 단계 · setup 은 늘 `Ok` · 기록기 시작 · 종료 `StateSession::shutdown` · 스냅숏 원천 `LiveSource`) · `state/placement.rs`(신설 — 자리 기록 `record` · 입히기 `restore_windows` · `open_restored_popouts` · 모니터 판정 `locate` · `DeferredMaximize`) · `lib.rs`(로그 초기화를 부팅 단계로 옮김 · 플러그인 등록(단일 인스턴스 바로 뒤) · 빈 manage · 기록기 시작 · ⑧⑨ · resync · `RunEvent::Exit` · Moved/Resized — 게터는 락 밖) · `layout/manager.rs`(`PlacementMemo` · `observe_window_placement`) · `layout/apply.rs`(`WindowHost::record_placement` — 연 창의 첫 자리) · `layout/mod.rs` · `commands/popout.rs`(창 빌더의 첫 자리 인자 · `record_placement` 구현 — 숨김 인자는 없다, F13) · `commands/layout.rs`(`record_placement` 위임) · `commands/settings.rs`(`push_themes`) · `tray/actions.rs`(미룬 최대화 입히기) · `ui_settings.rs`(쓸기 주석) · `state/{convert,lock,mod,saver,schema,tree_attrs}.rs` · `src-tauri/tests/{layout_apply,layout_commands}.rs` · CLAUDE.md 「LLM-우선 제어」의 레이아웃 영속 줄 · `docs/reference/architecture-overview.md` 소유 표 | M–L (~700 — 실제 +1,546 / −90 줄, 시험 · 문서 포함) |
| **P3c1** (L3) 크래시 사본 + 답할 길 — 사본 열 · 복원 서비스 · `restore.*` · 조율자 · ✅ **착지 `064b61c`(2026-10-05)** — 리뷰 반영 = §14-14 · `/qa full` PASS(GUI 7항목 = 크래시 → 사본 + `awaiting` · 수락 → 복원 · 거절 · 답 전 다시 크래시 → 사본 그대로 · 다른 앱이 앞일 때 수락 → 포커스를 뺏지 않음 · 트레이로 숨긴 채 수락 → 숨김을 지키고 보일 때 최대화 · 정상 종료 → 조용한 복원 — 사람 경로 `restore_answer` 는 부르지 않았다 · P3c2 몫) | §6-5 표 사본 열 · §6-7(프론트 빼고) | `state/boot.rs`(사본 열 · `BootInputs.crash_copy` · `AwaitingCopy` · `carry_resolved` · 가드 ⅱ · N1 재확인 · I3 파일마다 · 표 시험) · `state/restore.rs`(신설 — `RestoreService` · `RestoreCoordinator` · 창 포트 `RestoreWindows`) · `state/boot_plugin.rs`(⑥ `set_boot` · 알림기 · `resolve_crash_copy` · 띄우는 중 기록기의 닫힘 표지) · `state/saver.rs`(`CloseFlag` · `SaverHandle::close` · `resolve` 의 `dead_code` 허용 걷음) · `state/placement.rs`(`TauriRestoreWindows` · `place_main` · label 마다 `DeferredMaximize` · `apply_now` · `forget_deferred_maximize` · 앞 창 판정 `foreground_is_ours`) · `state/schema.rs`(머리 — 앞 버전 리더 · 이름 바꾸기 규칙) · `state/{mod,tree_attrs}.rs` · `layout/manager.rs`(`adopt_restored`) · `layout/apply.rs`(`create_window` 의 모델 재확인) · `layout/commands.rs`(`restore.*` · `catalog_version` 11 · `restore_error`) · `commands/popout.rs`(`build_hidden_runtime_window` · `try_destroy_window`) · `commands/layout.rs`(`command_ports` 의 조율자 · `OwnedEvents` 공개 · 구독 원천 `AppSubscriptions`) · `commands/state.rs`(신설) · `commands/mod.rs` · `tray/actions.rs`(모든 창에 미룬 최대화 · `with_usage_visibility` 공개) · `lib.rs`(`pub mod state;` · 서비스 · 조율자 manage · 포트 꽂기 · `Destroyed` 의 미룸 거두기 · invoke 둘) · `daemon_client/tests.rs`(포트에 조율자) · `src-tauri/tests/{layout_apply,layout_commands}.rs` · 바인딩 `src-tauri/bindings/{AnswerReply,CrashCopyStatus,RestoreStatusView}.ts`(신설) · `prompts/engram-help.md`(`window` 구획에 `restore.*`) · `src-tauri/Cargo.toml`(`cfg(windows)` `windows` 0.58) · `Cargo.lock` · (문서 — 코드 커밋 밖) CLAUDE.md 「LLM-우선 제어」의 레이아웃 영속 줄 · 「의존성」 `windows` · `docs/reference/architecture-overview.md` 소유 표 | M–L (~650 — 실제 +5,580 / −237 줄 · 26 파일, 시험 포함) |
| **P3c2** (L3) 복원 모달 · 키 바인딩 멈춤 · 상태 파일 안내 · qa 바인딩 · ✅ **착지 `7b742fb`(2026-10-06)** — 리뷰 반영 = §14-15 · `/qa full` PASS(격리 데이터 폴더 · GUI G1–G8 = G1 정상 기동 → `restore_status` 칸 여섯 · G2 크래시 → 모달 · `inert` · main 의 신뢰 클릭 · Ctrl+Tab 막힘 · 트리 · 팝아웃은 안 막힘 · G3 모달로 수락 · 거절 → `answered` · 사본 지워짐 · 단축키 되살아남 · G4 모달이 뜬 채 `quit_app` → 사본 바이트 그대로 · 다음 기동에 모달 다시 · G5 `--hidden` → `show_main_ui` → 모달 · G6 버스 `engram restore.answer --accept true` → 알림으로 모달 닫힘 · G7 쓰레기 `state.json` → `corrupt_copied_aside` 안내 · `.corrupt` 에 원문 · 닫기 · G8 `state.json` 을 공유 없이(FileShare None) 쥔 채 기동 → `unreadable` 안내 · 그 실행은 상태를 쓰지 않음) · 로컬 게이트 PASS · 0 실패(수치 = `docs/process/step-log.md` P3c2 항목 · CLAUDE.md 「빌드·검증 명령」의 `lib_unit` 줄) · CI 초록(run 37329521049 — backend · frontend · fmt+격리) · 워크스페이스 총계는 로컬에서 다시 재지 않았다(CI 로 갈음) | §6-7 프론트 몫 · §6-5 ③ 가드 ⅰ 안내 · `restore.status` 의 `durable` · `state_file` | `src/components/layout/RestoreModal.tsx`(신설) · `src/components/layout/StateFileNotice.tsx`(신설) · `src/components/layout/AppLayout.tsx`(★계획의 `src/App.tsx` 가 아니다 — §6-7 P3c2 구현★) · `src/api/restoreClient.ts`(신설) · `src/api/testing/fakeRestoreIpc.ts`(신설) · `src/commands/keybindings.ts`(`setKeybindingsPaused` — N4) · `src/i18n/ko.ts`(`restore`) · 시험(`restoreClient.test.ts` · `RestoreModal.test.tsx` · `StateFileNotice.test.tsx` · `AppLayout.test.tsx` · `keybindings.test.ts`) · `state/boot.rs`(`StateAside` · `BootPlan.state_aside` — ④ 가 세운다) · `state/boot_plugin.rs`(⑥ `set_boot(copy, state_file)` · `state_file_status`) · `state/restore.rs`(`StateFileStatus` · `RestoreStatusView` 의 `durable` · `state_file`) · `layout/commands.rs`(`restore.status` 답 · `catalog_version` 12) · `src-tauri/tests/layout_commands.rs` · 바인딩 `src-tauri/bindings/{RestoreStatusView,StateFileStatus}.ts`(`StateFileStatus` 신설) · `prompts/engram-help.md`(`restore.status` 의 두 칸) · (문서 — 코드 커밋 밖) `.claude/skill-bindings/qa.md` §full — 계획의 두 갈래 중 「기동 뒤 `crash_copy == awaiting` 이면 답한다」를 골랐다(F15 대가) · ★자동으로 답하는 것은 QA 가 띄운 스크래치 데이터 폴더에서만이고, 답은 계획의 거절(`accept:false`)이 아니라 수락이다 — 세션 판단(사용자 위임 — 「쭉 진행해」)★: 수락은 답하는 순간 파괴하지 않고 거절은 저장된 화면을 그 자리에서 되돌릴 수 없게 지운다 · 워크트리의 기본 `.engram-dev` 면 답하지 않고 멈춰 주인에게 묻는다(답 정책 — 사용자 결정 2026-10-05 · §6-7 모달) · teardown 을 `quit_app` 으로 바꾸는 갈래는 고르지 않았다: 그 함수는 데몬까지 멈춘다(까닭 = 그 절) | S–M (~350 — 실제 +2,033 / −91 줄 · 20 파일, 시험 포함) |
| **P3c2 후속** (L3) 상태 파일 안내 덮기 · 가드 ⅱ 안내 · `restore.status` 에 `saves` — ✅ **착지 `125d190`(2026-10-06)** · 기록(검증 · 열린 것) = §14-16 · 결정 = ADR-0276 | §6-5 ③ 가드 항목 · ⑥ · §6-7 명령 · 상태 창구 · §8 | (커밋 `125d190` 의 파일 목록 — 여기 베끼지 않는다) | — |
| **연결 띠 덮기** (L3) 연결 끊김 안내도 덮는다 — main · 팝아웃 · 트리 · 공용 덮는 층 — ✅ **착지 `9318b5c`(2026-10-06)** · 기록(검증 · 검증 안 된 것) = §14-17 · 결정 = ADR-0277 | §6-5 ③ 가드 항목(화면 모양) · §8 | (커밋 `9318b5c` 의 파일 목록 — 여기 베끼지 않는다) | — |
| **P3c3** (L3) 슬롯 두 상태 · 곁표 슬롯 프론트 — ✅ **착지 `e904161`(2026-10-06)** · 결정 = ADR-0280(슬롯 활성화 단추 없음 — 꺼진 에이전트는 부재 막) · 기록(리뷰 경과 · 검증 · 검증 안 된 것 · 미룬 것) = §14-18 · `/review code full`(2 family · 5 라운드) · `/qa full` PASS(스크래치 데이터 폴더 · GUI — 목록 = §14-18) · CI 초록(run 37429747731) | §6-8 · §6-2 프론트 몫 | (커밋 `e904161` 의 파일 목록 — 여기 베끼지 않는다 · 프론트만) | S–M (~300 — 실제 +332 / −66 줄 · 15 파일, 시험 포함) |
| **P3d** (L3) 창별 테마 + 은퇴(옛 값 다루지 않음 — U1) — ✅ **착지 `c85b2ac`(2026-10-07)** · 결정 = F7 (a)(사용자 결정 2026-10-06 — 화면 UI 없음) · `window.setTheme` 배달 실패의 답(사용자 결정 2026-10-07 — §5-6) · 새 ADR 없음(ADR-0265 결정 7 · 도장 확장 = §11 P3d 메모) · 기록(바뀐 것 · 검증 · 열린 것) = §14-19 | §5-6 창별 · §3-5 창별 · §5-7 | (계획) `state/tree_attrs.rs` · `layout/commands.rs`(`window.setTheme`/`getTheme` · `ui.refresh` 삭제) · `layout/manager.rs`(테마 칸 쓰기) · `commands/settings.rs` · `ui_settings.rs`(삭제) · `lib.rs`(쓸기 호출 삭제) · `src/theme/uiSettings.ts`(+시험) · `src-tauri/tests/layout_commands.rs` · `prompts/engram-help.md` — 실제 = 커밋 `c85b2ac` 의 파일 목록(`theme.rs` 신설 포함 — 여기 베끼지 않는다) | M (~600, 절반이 삭제 — 실제 +1826 / −2172 줄 · 27 파일, 시험 포함 · 커밋 직전 작업 트리 기준) |
| ~~**P3e** (L3) 슬롯 보기 모드 셸 이전 · 영속~~ — ★**뺐다(사용자 결정 2026-10-07 · §14-20 · 되살림 = T-51)**★ · ~~자리 · 명령 모양은 착수 전 설계(§6-0 ①–⑦ · §9-1) · 착수 전 F19 답~~ | §6-0 | (하지 않음 — 아래는 옛 예상) `layout/types.rs` · `layout/manager.rs` · `layout/commands.rs` · `commands/layout.rs` · 바인딩 · `state/schema.rs` · `src/store/viewStore.ts` · `src/commands/renderModeCommands.ts` · `src/components/slot/renderMode.ts` · `src/components/layout/LayoutLeaf.tsx` · `docs/reference/architecture-overview.md` · 해당 시험 | — |
| **P4** (L4) 웹뷰 폴더 — ✅ **착지 `11f8787`(2026-10-08)** · 결정 = ADR-0283 · 사용자 결정 = M4 (c)(못 쓰면 물러나 진행) · main 생성 실패는 종료(2026-10-07) · 기록(바뀐 것 · 검증 · 알려진 한계) = §14-21 · `/review code full` PASS(2인 · 재수정 3회) · `/qa full` PASS(GUI 11항목 + 혼합 DPI · 재실측 A–F) | §4 · §6-5 ⑧ | (계획) `src-tauri/tauri.conf.json` · `lib.rs` · `commands/popout.rs` · 공통 마무리 함수(`commands/popout.rs` 또는 새 `webview_env.rs`) · ★정적 창 생성은 실행 표식 뒤 — §6-5 ⑧ 자리(§4 · I6)★ — 실제 = `webview_env.rs`(신설) · `state/placement.rs` · `lib.rs` · `commands/popout.rs` · `state/boot_plugin.rs`(주석) · `tauri.conf.json` · `tauri.dev.conf.json`(주석) | S–M (~350 — 실제 +824 / −83 줄 · 7 파일, 시험 포함) |

- **P3a 가 넘긴 것(P3b1 · P3b3 착수 때 볼 것 — §14-12):**
  - **`PersistedNode` ↔ `LayoutNode` 변환은 모든 칸을 이름으로 풀어 쓴다(`..` 금지)** — 어느 쪽에 칸 · 변형이 늘면 변환이 컴파일에서 깨지게 한다(P3a 의 맞대기 시험 — `state/schema.rs` 시험 — 과 같은 수법). `..` 로 넘기면 새 칸이 조용히 영속에서 빠진다. ✅ P3b1 — `state/convert.rs` 머리(두 방향 모두 · `WindowTabs` · `WindowAttrs` 포함).
  - **`from_persisted` 는 창 묶음의 모양도 다시 세운다** — 코덱은 모양 검사를 하지 않는다(§6-2). 같은 창 id 의 중복 · id 와 `kind` 의 어긋남(`{"id":"main","kind":"tree"}` · id 가 `main` 인 팝아웃)도 다룬다. ✅ P3b1 — 수리 목록 = §6-2.
  - **창 위치는 유한한 값만 적는다**(P3b3 — `Moved` / `Resized` 기록 자리 · §6-3). 유한하지 않은 실수는 JSON 에 `null` 로 나가 다음 읽기가 그 창을 건너뛴다. ✅ 메모리 `WindowBounds::new` 가 유한하고 크기가 0 보다 큰 값만 만든다(`manager.rs`).
  - ~~★**분할 깊이 상한(`manager.rs`)은 사용자가 체감하는 동작이다 → P3b1 에서 사용자에게 묻는다**★~~ → **세션 판단(사용자 위임) 2026-10-03: 상한을 두지 않는다** — 코덱이 유일한 벽이다: 같은 슬롯을 거듭 나눈 분할 121겹부터는 쓰기를 거절하고(§6-4 `Unparsable`) 그 화면은 다음 변경까지 디스크에 가지 않는다.
- **P3b 가 넘긴 것(P3c1 · P3c3 착수 때 볼 것 — 코드 리뷰가 남김 · §14-13):**
  - ✅ **P3c1 몫 넷은 닫았다(`064b61c`) — 규칙의 정본은 각 자리다:** 런타임 수락은 부팅 전용 `restore_windows` · `open_restored_popouts` 를 통째로 쓰지 않는다(창 포트 `TauriRestoreWindows` · 숨긴 창 빌더 `build_hidden_runtime_window` — §6-7 ② ④) · 수락 커밋은 번호를 되감지 않고 `ViewManager::adopt_restored` 가 두 번호를 올린다(§6-7 ③) · 부팅 확장 지점의 `TODO(P3c1)` 와 `SaverHandle::resolve` 의 `dead_code` 허용은 모두 걷었다(계획 모양 = §6-5 ③) · 기록기 스냅숏 포트는 창 게터를 부르지 않는다 — 수락 경로는 스냅숏 원천(`LiveSource`)을 고치지 않았다(§6-4).
  - ✅ **P3c3 몫은 닫았다(`e904161`)** — `applyLayoutUpdated` 가 `foreign_slots` 를 캐시(`CachedView.foreignSlots`)에 싣고 `selectOpenTarget` 이 그 슬롯을 점유로 본다(§6-2). 옛 문안: ~~**열림 — P3c3 몫:** ★프론트 `applyLayoutUpdated`(`src/store/viewStore.ts:281`, `f9db35a` 기준)는 `foreign_slots` 를 캐시에 옮기지 않는다 → P3c3 없이 복원을 내보내지 않는다★ — 모르는 내용 슬롯이 프론트에선 빈 슬롯이다. L3 는 한 덩어리로 머지되므로(§9-1) master 에는 함께 선다. 지금 곁표가 처음 채워지는 길은 복원이 새 판이 쓴 모르는 내용을 읽을 때뿐이라 실해는 없다.~~
- **병렬 가능:** P2b ∥ P2c(겹침 0) · P3c3 ∥ P3b2 · P3b3 · P3c1(P3b1 뒤 — 프론트 대 Rust, 겹침 0). P3c2 는 P3c1 뒤(그 바인딩을 쓴다) · P3c2 와 P3c3 은 `src/i18n/ko.ts` 가 겹쳐 순차. 나머지는 순차(`lib.rs` · `layout/commands.rs` · `manager.rs` · 도움말 공유).
- 단계마다 `/review code` → `/qa`(GUI 가 걸리면 full) → 커밋. 착수 전 되돌릴 지점 = 직전 단계 커밋.
- GUI 확인 핵심: P1 빈 `.engram-dev` 로 기동 → `daemon\state\` · `daemon\run\daemon.json` 생성 · 에이전트 만들고 재시작 → 명부 유지 · 옛 `.engram-data` 가 있어도 손대지 않음 · P2a `ui-settings.json` 의 `theme` 가 못 쓸 값이어도 창별 칸은 적용(§3-5) · `settings.set theme.default light` → 모든 창 · 재시작 유지 · P2b `chat.style.fontSize` 즉시 반영 · P3a GUI 없음(시험만) · P3b1 · P3b2 GUI 스모크만(`/qa standard` — 배선 전이라 화면 상태 기능은 시험만) · P3b3(`/qa full` GUI 8단계 — §14-13) 바꾸고 1초 뒤 파일 · 트레이 종료 → `clean_exit:true` · 재시작 → 묻지 않고 탭·분할·팝아웃·위치 복원 · 트레이 종료 직후 바로 재실행 → 잠금을 기다린 뒤 같은 복원 · P3c1 화면을 꾸민 뒤 `taskkill /F` → 다음 기동에 `shell\state\state.crash.json` + 기본 화면 · `engram restore.status` → `awaiting` · `engram restore.answer`(수락) → 화면 복원 · 사본 사라짐 · (다시 `taskkill /F` 뒤) 거절 → 사본 사라짐 · 답하기 전에 `taskkill /F` → 사본 바이트 그대로 · P3c2(✅ G1–G8 — §9-2 P3c2 행) `taskkill /F` → 다음 기동에 모달(main 사용 불가 · 단축키 안 먹음) → 모달로 답 → 사본 사라짐 · 모달이 떠 있는 채 트레이 종료 → 다음 기동에 모달 다시 · `--hidden` 기동 → 트레이로 열면 모달 · 못 쓸 · 잠긴 `state.json` → 안내 · `/qa full` 은 기동 뒤 `awaiting` 이면 QA 가 띄운 스크래치 데이터 폴더에서만 수락으로 답하고 진행한다(기본 `.engram-dev` 면 멈춰 주인에게 묻는다 · qa 바인딩 — teardown 은 강제 종료 그대로) · P3c3(✅ — §14-18) 꺼진 에이전트를 가리키는 슬롯에 부재 막(단추 없음) → 트리에서 활성화하면 슬롯이 스스로 붙음 · 목록이 온 뒤 「연결 중」 없음 · 지운 에이전트 = 「대상 없음」 + 「비우기」가 있는 메뉴 · 모르는 내용 슬롯 자리표시 · 트리 「열기」가 그 칸을 덮지 않음 · 포커스 링 색이 막 유무와 같음(다크) · P3d 창별 테마 재시작 유지 · ~~P3e 보기 모드를 챗으로 바꾸고 재시작 → 유지~~(P3e 를 뺐다 — 2026-10-07 · §14-20) · P4(✅ `/qa full` — GUI 11항목 + 혼합 DPI · 재실측 A–F · §14-21) `data\webview\` · 팝아웃 유령 창 없음 · 못 쓰는 폴더 → ~~대화상자~~ 기본 자리로 물러나 warn(M4) · 런타임을 가린 실패 → 종료 코드 1 · `clean_exit:true` · 곧바로 재실행 · `--hidden` → main 숨긴 채 · 보이는 부팅의 전경 = main.

## 10. 구현 갈림길

**리뷰가 사양으로 확정한 것(갈림길 아님):** F2 · F3(옛 잠금 · 이전 실패 처리 — **옛 데이터 버림으로 무의미해졌다**, §3-2 · §14-5) · F10(모르는 내용 = 영속 DTO 전용 — §6-2) · ~~F12~~(띠가 보였으면 답 없는 정상 종료에 지운다 — **6판에서 없앴다**, D6) · F16(대상 없음 / 정지됨 두 상태 — §6-8 · ★P3c3 에서 「정지됨」 + 단추는 부재 막(글 없음 · 단추 없음)으로 바뀌었다 — 사용자 결정 2026-10-06 · ADR-0280★).

**사용자 결정 D1–D3(2026-10-02 · §14-9)으로 닫힌 것:** F17(**철회** — 「최근 3 개 회전 보관(Firefox 류)」은 출처 없는 세션 기본값이었다 · 크래시 사본은 `state.crash.json` 한 개 — §6-7) · **크래시 뒤 자동 복원 여부**(번호 없이 바로 닫힘 — **늘 묻는다, Chromium 식**. 거부 = Firefox 식(첫 크래시는 조용히 복원하고 연속 크래시만 묻는다) — 사용자가 고른 근거: 지배적 관행 · 단순 · 크래시 루프 방지. Chromium 은 크래시 루프를 피하려고 크래시 뒤엔 자동 복원하지 않는다 — [조사](../../research/crash-session-snapshot-precedents-2026-10-02.md)).

**사용자 결정 D4–D8(2026-10-02 · §14-10)으로 닫힌 것:** F11(D4 · N4 — 모달과 키 바인딩 멈춤이 사람의 사용을 막아 바꿀 화면은 크래시 뒤의 기본 화면이다 · 예외는 답 전에 LLM 이 버스로 바꾼 것뿐(버스는 막지 않는다 · 관례상 먼저 답한다) → (a) 통째로 바꾼다) · F12(D6 — 없앴다: 답만 사본을 푼다) · F15(**사용자 결정 2026-10-02** — 「개발 빌드도 예외 없이 항상 묻는다 — 나중에 짜증나면 요청」 → (a) · 대가: `/qa` GUI teardown 이 강제 종료라(`scripts/launch-detached.ps1:75`) 다음 QA 기동에 모달이 뜬다 — P3c2 가 qa 바인딩을 고친다 → ✅ `.claude/skill-bindings/qa.md` §full: 기동 뒤 `awaiting` 이면 QA 가 띄운 스크래치 데이터 폴더에서만 `restore_answer {accept:true}` 로 답하고 진행 — 수락은 답하는 순간 파괴하지 않는다 · 워크트리의 기본 `.engram-dev` 면 답하지 않고 멈춰 주인에게 묻는다(답 정책) · 스크래치 한정 · 수락 = 세션 판단(사용자 위임 — §9-2 P3c2 행) · teardown 은 그대로) · **묻는 표면**(D4 — main 창 웹뷰 안 모달) · **기본 모양 예외**(D5 — 없다) · **`--hidden` 부팅**(D7 — 따로 다루지 않는다) · **쓰기 실패**(D8 — log 만 · [조사](../../research/crash-session-snapshot-precedents-2026-10-02.md) 「덧붙임」).

**사용자 결정 U1(사용자가 본인뿐이라 옛 값을 다루지 않는다)로 닫힌 것:** F4(옛 웹뷰 폴더 — 지우지도 안내하지도 않는다, §4) · F5(챗 스타일 가져오기 = (b) 생략, §3-6) · F18(남는 `ui-settings.json` — 읽기를 그만둘 뿐 따로 다루지 않는다, §3-5).

**사용자 결정(2026-10-02 · ADR-0274)으로 닫힌 것:** F21(못 쓰는 설정 · 상태 파일 떠 두기 — 「관행 따르면 될듯」 · 「구지 쌓아야됨?」. F21 의 세션 기본값 (a)(1 MiB 넘는 원본은 떠 두지 않음 · 직전 사본과 바이트가 같으면 다시 뜨지 않음)를 걷고, §5-3 · ADR-0265 결정 4 의 시각 · 번호 이름으로 쌓는 규칙도 함께 걷어 Chromium `Preferences.bad` · Firefox `Invalidprefs.js` 관행을 따른다: 고정 이름 `<파일>.corrupt` 하나 · 다음엔 덮어쓴다 · 크기 한도 · 중복 생략 없음 — §5-3 · §6-2 · ADR-0265 결정 4 부분 폐기).

| # | 갈림길 | 선택지 | 기본값 |
|---|---|---|---|
| F1 | `settings.set` 값 싣는 법(LLM 이 보는 모양) | (a) 정규 문자열 하나(§5-2) · (b) 매크로 알파벳에 임의 JSON 추가 · (c) 종류별 선택 칸 | **(a)** (추천 · 세션 기본값 — 사용자 확인 대기) |
| F6 | 설정값 첫 페인트 | (a) 비동기 + CSS fallback = 기본값 · (b) 창 생성 때 초기값 주입(새 전역) | **(a)** (추천 · 세션 기본값 — 사용자 확인 대기) |
| F7 ★명시 확인★ | 화면의 테마 바꾸기 UI | (a) 이번엔 없음(명령만) · (b) 메뉴/팔레트 항목 | **(a) — 사용자 결정 2026-10-06** · P3d 착지(명령 = `window.setTheme` · `window.getTheme` · `settings.set theme.default` — §5-6 · §14-19) |
| F8 | 창 위치·크기 복원 | (a) 저장·복원(보통 크기만 기록 · 화면 밖 버림) · (b) 배치만 | **(a) — 사용자 결정 2026-10-03** · P3b 착지(§6-3 · §6-5 ⑧⑨) |
| F9 | 트리 창 | (a) 테마·위치만, 보임은 복원 안 함 · (b) 보임도 | **(a) — 세션 판단(사용자 위임) 2026-10-03** · 최대화도 싣지 않는다(§6-3) |
| F13 | `--hidden` 부팅 때 팝아웃 | (a) 숨긴 채 만들고 main 과 함께 보임 · (b) main 을 보일 때 만듦 | **사용자 결정 2026-10-03 — `--hidden` 전용 로직 없음**: 보이는 채 만들고 setup 끝의 숨기기가 main 과 함께 숨긴다 · 보이기는 main 과 함께(§6-5 ⑨). 그래서 `--hidden` 부팅에 복원한 팝아웃도 main 처럼 잠깐 떴다 숨는다 — P4 의 §4 「덤」(숨긴 채 만들기)은 main 만 고친다 [미검 — GUI] → ✅ P4: main 은 숨긴 채 만들고 복원한 팝아웃은 그대로다(알려진 한계 — §14-21) |
| F14 | 도움말 구획 | (a) `theme` → `settings` + `help theme` 별칭 · (b) id 유지 · 본문만 교체 | **(a)** (추천 · 세션 기본값 — 사용자 확인 대기) |
| F19 ★명시 확인★ | 남길 슬롯 보기 모드 값(§6-0) | (a) 지금 셋 그대로(`terminal` · `rich` · `dom`) · (b) `terminal` · `rich` 만(`dom` 은 남기지 않는다) | ~~**(a)** (추천 · 세션 기본값 — 사용자 확인 대기 · 있는 것을 그대로 옮겨 가장 단순 · P3e 착수 전)~~ → **(a) — 사용자 결정 2026-10-07 · 같은 날 P3e 를 빼서 무효가 됐다(§14-20)** — 되살리면(T-51) 다시 묻는다 |
| F20 | 실행 중 `settings.json` 삭제(§5-3) | (a) 초기화가 아니다 — 다음에 파일을 쓰는 호출이 메모리의 보이는 값 그대로 되살린다(멀쩡한 파일에서 손으로 지운 키는 되살리지 않는다) · (b) 초기화로 본다 | **(a)** (세션 기본값 — 사용자 확인 대기) |
| M4 ★명시 확인★ | 웹뷰 폴더를 못 쓸 때 · 네트워크 공유 | (a) 창 만들기 전에 확인 → 네이티브 대화상자 후 종료 · 공유 폴더는 당분간 미지원으로 문서화(ADR-0134 결정 3 개정) · (b) 확인 없이 기동 | **(c) 물러나 진행 — 사용자 결정 2026-10-07**(같은 날 「(a) 의 앞 절반 — 대화상자 후 종료」로 답했다가 바꿨다): 창 만들기 전 쓰기 확인 한 번 → 못 쓰면 우리 폴더를 지정하지 않아 Tauri 기본 자리(`%LOCALAPPDATA%\<식별자>`)로 진행 · warn 로그. 판정은 프로세스당 한 번이고 모든 창이 같은 판정을 받는다(ADR-0054). 대화상자 없음. 근거 = 저장 정책 「멈추지 않고 진행」(§6 D8) · 데이터 폴더 전체가 못 쓰면 창이 뜬 뒤 ADR-0134/0135 의 기존 경로가 이유를 창에 띄운다. 대가 = 그 실행만 같은 식별자의 워크트리끼리 웹뷰 폴더를 함께 쓴다(지금 그 폴더에 읽히는 값 없음 — §4 「잃는 것」). ★공유 폴더 문서화 · ADR-0134 결정 3 개정은 뺀다★ — 그 결정의 공유 폴더는 데몬끼리의 배제 문제라 클라이언트의 웹뷰 폴더와 무관하다(사용자 지적) · ✅ P4 `11f8787` · ADR-0283 |

## 11. ADR 후보 (`/adr` 가 채번 — 0264 · 0265 · 0274 · 0276 · 0277 · 0280 · 0283 은 이 브랜치가 썼다 · 0278 · 0279 · 0282 는 `v0.3.3/refactor/crate-boundaries` 가 선점해 건넜다 · 다음 번호는 채번 때 정한다)

1. **→ ADR-0264 로 박제.** 아래는 3판 후보 문안이고, 그 뒤 이전을 걷었다(§14-5 — 정본은 ADR-0264). **데이터 폴더를 컴포넌트·종류(config·state/run)로 가르고 첫 부팅에 옮긴다 · 경로 단일 출처 `DataLayout` · 옛 잠금도 쥔다 · 이전 완료 표지.** 개정 도장: **ADR-0136 결정 3**(파일 이름 규칙 원칙에 `**/data/daemon/run/` 폴더 규칙 하나를 더한다 — 경로가 구체적이라 무관한 폴더를 안 삼킨다) · ADR-0135(주소를 싣는 잠금은 여전히 하나 — 옛 자리 guard 는 배제만). 거부: **설치형(Program Files + AppData)** — 배포가 ZIP 한 덩이·한 사용자이고 설치형의 이유(여러 사용자·쓰기 보호·자동 갱신)가 지금은 해당되지 않으며 워크트리 격리를 환경변수로 따로 지켜야 한다(보고서 §2-4·§2-5) · **평면 유지** — 지킬 것·버릴 것·토큰이 섞인다 · **종류 먼저** — 사용자 결정 · **Tauri 2.12 `appDirectoriesOverride`** — 2.11.3 에 없다 · **복사 후 옛 것 유지** — 하향 바이너리가 낡은 사본으로 지운 에이전트를 되살린다 · **「둘 다 있으면 새 것」** — 실패한 이전 뒤 생긴 빈 명부가 이긴다 · **옛 자리를 지워 보는 방식** — 지운 뒤 옛 데몬이 같은 자리에 새로 만들면 둘이 뜬다 · **경로 정본을 base 에**.
2. **→ ADR-0283 으로 박제(2026-10-08 · P4 `11f8787`).** 아래는 후보 문안이다(정본 = ADR-0283). **웹뷰 데이터 폴더 = `<root>\webview` · 정적 창을 Rust 에서 · 모든 창이 같은 폴더·같은 환경 옵션(ADR-0054 확장) · 만들기 전 쓰기 확인.** 개정 도장: ~~**ADR-0134 결정 3**(네트워크 공유 폴더 — 당분간 미지원)~~(뺐다 — 사용자 결정 2026-10-07 · §10 M4) · ADR-0137 「식별자가 웹뷰 폴더도 정한다」 대가 서술(✅ 부분 폐기 도장) · **+ ADR-0054**(상수 자리 · config 사본 · 3중 동기 지점 · 불변식이 덮는 환경 옵션 범위 — ✅ 부분 폐기 도장 · 낡은 코드 포인터에 개정 표시). 도장 아닌 줄: ADR-0102 관련줄(맥락의 「웹뷰가 setup 전에 invoke 한다」는 정적 창에 더는 서지 않는다 · 결정은 그대로) · ADR-0225 「함께 바꿀 곳」(P4 가 더한 트리 창 몫 코드). 거부: 설정 키 `dataDirectory`(상대 경로만) · `appDirectoriesOverride`(업그레이드 선행) · `%LOCALAPPDATA%` 유지(포터블 원칙 위반 · 워크트리 간 localStorage 공유) · 옛 폴더 자동 삭제(F4 — U1 로 다루지 않음) · **+ 사용자 결정 2026-10-07:** 못 쓰면 대화상자 후 종료(M4 — 「멈추지 않고 진행」 D8) · main 을 못 만들어도 트레이만으로 계속(종료를 골랐다 — 사용자 사유 「이상한 것들 에러 나면 강제 종료가 맞지」(2026-10-07) · ADR-0283 거부한 대안).
3. **설정 = 셸 `settings.json` + 범용 명령 넷 + 스키마 한 줄 등록 · 쓰는 쪽은 명령 하나.** **→ ADR-0265 로 박제 · 그 결정 4 의 떠 두기 규칙(이름 · 한도 · 중복 생략)은 ADR-0274 가 부분 폐기했다(F21 — 고정 이름 `.corrupt` 하나 · 덮어쓰기 · 설정 · 상태 공통 · §5-3 · §6-2).** 폐기/개정 도장: ADR-0166 결정 1·3·9 · ADR-0167 결정 3·6·7 · ADR-0051 「권위 = 프론트」 · ADR-0169 「남은 갭」 해소. 거부: **값마다 명령**(0169 가 이미 거부) · **파일 직접 편집 + `ui.refresh`**(밖의 편집자와 앱이 한 파일을 다툼 — ADR-0167 이 남긴 갈림길 중 「쓰기를 한 곳으로」를 고른다) · **localStorage** · **데몬 소유**(표시 설정은 셸, 데몬은 에이전트 정의만 — 결정) · **설정과 상태를 한 파일에**(보고서 §2-1 · orca) · **매크로에 임의 JSON**(F1) · **빈 경로 갈래 미리 깔기**(첫 키와 함께) · **옛 값 가져오기(`ui-settings.json` · 챗 스타일 localStorage)**(사용자 결정 U1 — 배포본(v0.1.0–v0.3.2)은 있으나 사용자가 본인뿐이라 지킬 사용자 데이터가 없다(사용자 판단 2026-10-02) · §3-5 · §3-6) · **챗 스타일 키 재설계**(사용자 결정 U2 — 챗 영역 재작성과 플러그인 배치로 갈려 나갈 임시 이름공간 · §5-1).
4. **화면 상태 = 셸이 쓰는 `state.json` · 영속 창 id(UUID)와 부팅마다 새 label · 항상 복원, 비정상 종료 뒤에만 묻는다 · 창별 테마·위치는 창 항목에 · 대상 없는 슬롯 두 상태.** 폐기/개정 도장: **ADR-0167 결정 1**(표시 상태 = 「데이터 + refresh」 분류 → 상태는 명령으로 쓴다) · **ADR-0167 결정 5**(`theme.set` 류 철거의 구조 근거 「host 창만 지목 가능」은 프론트 선언 명령의 성질이었다 — 셸 선언 `window.setTheme` 은 창을 인자로 지목한다) · ADR-0167 결정 6·7(쓸기 폐지) · ADR-0166 결정 3 · ADR-0060 「영속 도입 시 요건」 이행 · **ADR-0222**(기본 레이아웃 = 복원할 상태가 없을 때만) · **ADR-0149 (A)**(스폰 대기도 「연결 중」 → 「정지됨」 상태 — ★P3c3 에서 「정지됨」 대신 부재 막 · ADR-0280 으로 박제 — 아래 P3c3 메모★). 거부: **레이아웃을 localStorage 에**(LLM-우선 충돌 · 웹뷰 폴더에 묶임) · **`tauri-plugin-window-state`**(창 기하만 · 2.11.3 에선 `app_config_dir` 고정) · **한 파일에 전부** · **지금 SQLite**(T-14) · **창별 테마를 설정 파일에**(창 수명과 떨어져 쓸기가 다시 필요) · **이름 `session.json`**(에이전트 세션과 충돌) · **기본 꺼짐 + 설정으로 켬**(사용자가 Chrome 방식을 골랐다) · **종료 때만 저장**(보고서 §2-7) · **label 을 영속 신원으로**(런타임 수락이 떠 있는 창과 부딪힌다) · **메모리 `SlotContent::Unknown`**(IPC 가 모르는 종류를 받아들이게 되고 바인딩·망라 분기 전부로 퍼진다).
   - **4판 추가(§14-6 — 문안은 P3 착수 전 재설계 뒤 확정):** 저장 범위 표(§6-0) · ~~슬롯 보기 모드 = 셸 소유 슬롯 값 · 영속~~ — ★**P3e 를 뺐다(사용자 결정 2026-10-07 · §14-20) — 아래 보기 모드 개정 도장 후보(ADR-0056 · ADR-0035 예외 목록 · 버스 제외 근거)는 지금 필요 없다. 두 결정 모두 그대로 선다(덮어쓰기는 프론트 전용 그대로)**★ · ~~개정 도장 후보:**ADR-0056** 결정(`:12` · `:29` — 렌더 모드 덮어쓰기는 프론트 전용) · ADR-0035 의 프론트 전용 예외 목록(`docs/reference/architecture-overview.md:578`) · 셸 소유가 되면 버스 제외 근거(창마다 따로인 프론트 상태)도 사라진다(§6-0 ③) · ADR-0078(출력 형식 생성 고정)은 그대로 — 남기는 것은 렌더러 덮어쓰기다~~ · ~~비정상 종료 스냅숏 최근 3 개 회전 보관 — 거부 후보: 대기분 한 파일 덮음(3판 모델 · F17)~~(5판에서 철회 — 아래 5판 추가). 거부한 대안의 근거는 사용자가 준다.
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
   - **P3c1 메모(2026-10-04):** 크래시 사본 열 · 복원 서비스 · 복원 조율자의 설계(§6-5 ③–⑥ · §6-7)는 **아직 ADR 이 없다 — 정본은 이 TRD 다.** P3c1 은 ADR 을 새로 만들지 않았다(채번은 `/adr` 몫). 그 사이 세션 판단(사용자 위임)으로 정한 것(§14-14)은 사용자가 그대로 두기로 확인했다(2026-10-05 — §14-15).
   - **P3c2 메모(2026-10-06):** P3c2 도 ADR 을 새로 만들지 않았다 — 상태 파일 안내(사용자 결정 2026-10-05)의 정본은 §6-5 ③ 가드 항목이다.
   - **안내 덮기 메모(2026-10-06 · 후속 `125d190`):** → **ADR-0276 으로 박제** — 상태 파일 안내는 레이아웃을 밀지 않고 덮는다 · TabBar 를 가려도 된다 · 여럿이면 쌓고 넘치면 스크롤(사용자 결정 2026-10-06) · 가드 ⅱ 도 같은 안내(세션 적용안에 사용자가 답 — ADR-0276 결정 4). 개정 도장: **ADR-0180**(알림이 덮는 것을 팝업 자격으로만 가르는 불변식 — 상태 파일 안내 하나만 예외 · 연결 띠는 그대로 · ★연결 띠는 같은 날 ADR-0277 이 옮겼다(아래 메모)★). 거부: **흐름 블록(미는 띠 — P3c2 의 모양)** — 사용자: 「우리는 레이아웃을 고정시키고 그 고정된 상황에서 작업하는건데 팝업이 나오면 밀림」.
   - **연결 띠 덮기 메모(2026-10-06 · `9318b5c`):** → **ADR-0277 로 박제** — 연결 끊김 안내도 레이아웃을 밀지 않고 덮는다 · 범위 = main · 팝아웃 · 트리 창(사용자 결정 2026-10-06 — 「덮기로해.」 · 질문의 틀 = ADR-0277 결정 1). 개정 도장: **ADR-0180**(같은 불변식 중 연결 띠) · **ADR-0276**(결정 5 범위 중 연결 띠). 거부: **흐름 블록(미는 띠 — ADR-0180 이 연결 띠에 둔 모양)** — ADR-0276 에서 사용자가 든 이유를 세션이 연결 띠에 걸어 추천했고 사용자가 그 추천에 답했다. 구현(공용 덮는 층 · 세 창의 자리 기준 · 줄 계약)은 세션 판단이다.
   - **P3c3 메모(2026-10-06 · `e904161`):** → **ADR-0280 으로 박제** — 꺼진 에이전트를 가리키는 슬롯(프로필 있음 · 실행 중 아님 · 기억 없음)은 부재 막으로 덮고 슬롯 활성화 단추를 두지 않는다 · 「대상 없음」은 에이전트 슬롯 메뉴 그대로(「비우기」 포함) · 모르는 내용 슬롯에 「비우기」 없음 · 포커스 링은 막 위(사용자 결정 2026-10-06). 개정 도장: **ADR-0149** — 다섯 항목(① 결정 (A) 의 스폰 대기 흡수 ② 결정 (C) 의 문구 — 프로필이 지워진 슬롯 ③ 「근거」 마지막 항목 중 예약 노드를 활성화한 직후 죽은 표시가 스침을 피해야 한다는 판단 — 기억 없는 예약 슬롯에 한해 ④ 「영향」의 알려진 한계 중 기억 없는 렌더러의 표시 ⑤ 거부한 대안 「삭제도 흐림으로 통일」의 사유 — 기억 없는 예약 슬롯에 한해 · 정본 = ADR-0280 관련줄). 막을 그리는 쪽이 넷(세 슬롯 + 잎)이 된 것은 사실 서술이라 ADR-0149 · ADR-0165 어느 쪽의 개정도 아니다 — 둘 다 ADR-0280 으로 링크만 건다(ADR-0280 「영향」). 거부: **§6-8 의 「정지됨」 + 「활성화」 단추**(사용자 사유 = ADR-0280 결정 1 · 덧붙는 사실 = 트리 · 슬롯 활성화 상태 맞추기가 리뷰 3 라운드 동안 새 결함을 냈다) · **「대상 없음」에 빈 슬롯 메뉴**(「비우기」가 사라졌다) · **모르는 내용 슬롯에 「비우기」** · **불투명한 고정 색 링**(지금은 하지 않는다). 구현(잎이 그리는 막 · 링의 z 와 자리 · 캐시 칸 · 모르는 내용 슬롯의 안내 한 줄 — ★이것은 같은 날 사용자 결정으로 걷고 아이콘 하나로 바꿨다(ADR-0280 결정 7)★)과 갓 만들었거나 갓 활성화한 에이전트의 슬롯에 막이 잠깐 스치는 것의 수용(R10)은 세션 판단이다.
   - **P3d 메모(2026-10-07 · `c85b2ac`):** 새 ADR 은 없다 — 창별 테마(상태 · `window.setTheme`/`getTheme` · 쓸기 폐지)는 ADR-0265 결정 7 이 이미 박았다. P3d 착지로 코드에서 사라진 결정을 **ADR-0265 의 부분 폐기 도장에 더했다**: **ADR-0166** 결정 1과 3과 9 → **+ 5**(답의 사유 두 갈래 — 답의 `source` 칸이 사라졌다) · **7**(파일의 미지 키 무시) · **8**(파일 값 로그의 모양 게이트) — 둘 다 그 파일 · 그것을 읽던 `ui_settings.rs` 와 함께 사라졌다 · **ADR-0167** 결정 3과 6과 7 → **+ 1**(창별 테마 = 「데이터 + refresh」 — 위 4 의 개정 도장 후보 그대로 · `window.setTheme` 이 명령이다) · **8**(쓸기 쓰기의 원자성 — 쓸기가 없어졌다). ★**ADR-0166 결정 6 은 폐기 도장에 넣지 않고 개정 도장을 받았다**★ — 사용자가 `window.setTheme` 배달 실패의 동작을 정했고(2026-10-07 · 지목한 창이 못 받았을 때만 오류 · 다른 창만 못 받으면 성공 + warn 로그 — §5-6), 그 결과를 ADR-0166 결정 6 의 개정으로 적었다(결정 6 = `window.setTheme` 이 지목한 창에만 걸린다 · 개정 도장 형식 = 사용자 선택 2026-10-07 · 새 ADR 없음). `settings.set` · `settings.reset` 의 `theme.default` 에 걸지는 열려 있다(§5-6). 위 4 의 후보 중 **ADR-0167 결정 5**(`theme.set` 류 철거의 구조 근거)는 도장에 넣지 않았다 — 결정(선언 철거)은 그대로 서고 근거 한 줄만 낡았다.

## 12. 위험 · 미확인

| # | 무엇 | 상태 |
|---|---|---|
| R1 | **웹뷰 폴더를 옮긴 뒤의 WebView2 실동작** — 첫 기동 시간 · 한 창이라도 폴더/인자가 어긋날 때 유령 창 여부 · 못 쓰는 폴더에서의 실패 모양 · 데이터 폴더 크기 증가(캐시가 포터블 폴더로 — 압축 복사·`build-release.ps1` clean) | **P4 `/qa full` PASS(2026-10-07 — §14-21):** GUI 11항목 + 혼합 DPI(항목 목록은 이 문서에 없다) · 재실측 A–F = 못 쓰는 폴더 → 기본 자리로 물러나 warn(M4) · 런타임을 가린 실패 → 종료 코드 1 · `clean_exit:true` · 곧바로 재실행 OK · 포커스 · `--hidden` · 회귀(종료 코드 0). 폴더 · 인자 어긋남은 구조로 막는다(모든 창이 한 마무리를 지난다 — §4). **남은 [미검]:** 첫 기동 시간 · 데이터 폴더 크기 증가(압축 복사 · `build-release.ps1` clean) — 이 문서에 잰 값이 없다 · 네트워크 공유 위 폴더를 WebView2 가 받는지 |
| R2 | **로그오프·종료 때 `RunEvent::Exit` 가 실제로 오고 2초 상한 안에 쓰기가 끝나는가**(§6-6 — 소스로만 확인) | [미검] — 수동 로그오프 |
| R3 | 하향 바이너리: 옛 바이너리는 옛 평면 파일만 보고 새 데몬과 잠금이 달라 함께 뜰 수 있다 — 각자 자기 명부로 돈다(§3-2) · 셸 상태(N6): 새 판이 쓴 `state.json` 은 버전 초과로 떠 두고 기본 화면 · 새 판이 쓴 사본(버전 초과)은 지우지 않고 남겨 둔 채 묻지 않는다 — 옛 판은 적용할 수 없고, 새 판으로 돌아가면 다시 묻는다 · 너무 새 사본이 있는 채 옛 판이 비정상 종료하면 그 뒤 옛 판 부팅은 모두 가드 ⅱ 에 걸린다 — ⑤가 돌지 않아 새 판이 답할 때까지 아무것도 저장되지 않는다(L3) | 지원 안 함 — 문서화 |
| R4 | 강제 종료 시 마지막 1~5초 변경 유실 · 개발 재빌드마다 복원 질문(F15 — 5판은 크래시 뒤 늘 묻는다, D3) | 설계상 수용 |
| R5 | 런타임 수락에서 새 팝아웃 창이 모델보다 먼저 뜨는 틈(§6-7 ②) · DPI 다른 모니터 사이 위치 | 틈은 P3c1 에서도 [미검] — §14-14 · 부팅 복원의 배율 다른 두 모니터(125% · 200%)는 P3b3 GUI 실측 PASS(2026-10-03) — `WM_DPICHANGED` 가 위치 세터 안에서 처리되는지는 [미검](`state/placement.rs` `place` 주석) |
| R6 | 곁표 무효화 진입점이 하나로 모이는지(§6-2) — 빠지면 사용자가 바꾼 슬롯에 옛 원문이 되살아난다 | P3b1 착지 — 거두는 자리 셋(`write_slot_content` · `remove_view` · `close_slot`)으로 모았다 · 쓰기 경로마다 곁표가 지워지는지 `manager.rs` 시험이 잰다 |
| R7 | Windows 원자 쓰기 = `rename` 교체(`ReplaceFileW` 미사용 · 보고서 §2-7). 못 읽는 파일은 옆에 떠 둔 뒤에만 덮으므로(§5-3 · ADR-0274 — 크기 한도 없음) 반쪽 파일이 덮여도 원문이 남는다 | 수용 |
| R8 | (걷음 — 이전이 없다, §14-5) | — |
| R9 | 설정·창별 테마·복원 답이 **셸이 떠 있을 때만** 명령으로 닿는다 | 「LLM-우선 제어」 갭으로 기록 |
| R10 | 갓 만들었거나 갓 활성화한 에이전트가 명부에 오르기 전 슬롯이 잠깐 부재 막(§6-8 — P3c3 전 계획의 「정지됨」 · ADR-0280) | 수용 · [미검] 체감 |
| R11 | 끝나는 중인 앞 인스턴스와의 겹침 — **실재한다**: 단일 인스턴스 플러그인이 `RunEvent::Exit` 에서 뮤텍스를 놓고 그 처리가 우리 `Final` 콜백보다 먼저 돈다(§6-5 사실 ② — 소스 확인). 4판의 근거(앞 인스턴스의 창을 못 찾을 때만 · `windows.rs:72-94`)와 「[미검] 실재 여부」는 틀렸다. 대응 = 셸 실행 잠금(I1 — 부팅 단계가 판정 전에 약 3초 기다린다). 남는 것: 3초 안에 못 잡으면 잠금 없이 진행하므로 앞 인스턴스의 `Final` 과 엇갈릴 수 있다 — 어느 쪽이 이겨도 파일은 온전하고, 앞 세션의 마지막 몇 초를 잃거나 정상 종료한 세션을 한 번 묻는다 | 수용(D8 정신) · 잠금 대기 = P3b3 GUI 실측 PASS(2026-10-03 — 잠금을 쥔 채 기동 → 기다린 뒤 복원) |
| R12 | `ReplaceFileW` 없이 rename 교체 + `closed` 확인과 rename 사이의 좁은 틈 — 그 틈에 발행되는 것은 `Final` 이 쓰려던 바로 그 내용이거나 더 이른 `clean_exit:false` 라 어느 쪽도 해롭지 않다. 6판: 이 확인은 `fsutil` 변형이 rename 마다(잠김 재시도 포함) 다시 묻고 「건너뜀」을 돌려준다(I8 · §6-4) | 수용 |
| R13 | 답하기 전에 다시 죽으면 그 세션의 화면은 잃는다(사본을 덮지 않는다 — D2-6). 모달과 키 바인딩 멈춤이 사람의 사용을 막아(D4 · N4) 그 화면은 기본 화면이다 — 남는 것은 답 전에 LLM 이 버스로 바꾼 화면뿐이다(버스는 막지 않는다 · 관례상 LLM 이 먼저 `restore.answer` 를 낸다) | 수용(원칙 — 「그런 것까지 가정하면 끝도 없음」) |
| R14 | 실행 표식 쓰기가 실패하면 그 실행의 크래시가 정상 종료로 읽혀 다음 부팅이 조용히 복원할 수 있다 | 수용 — 사용자 결정 D8(따로 다루지 않는다 · 조사 「덧붙임」: 네 앱 모두 같다) |
| R15 | (걷음 — D6 · D7: `--hidden` 부팅만 이어져도 사본은 답할 때까지 남는 것이 의도다) | — |
| R16 | `state.json` 을 못 읽었거나(잠김 IO — 재시도 뒤) 사본을 못 뜬 실행은 상태를 하나도 저장하지 않는다(가드 — §6-5 ③ · N3) — 그 실행의 화면 변경을 잃고 다음 부팅이 다시 판정한다. 6판의 「IO 실패 뒤 첫 쓰기가 덮어 크래시 화면을 잃는다」 위험은 없어졌다. ★사본 읽기가 계속 IO 실패하고 `state.json` 이 `clean_exit:false` 면 부팅마다 가드 ⅱ 다(P3c1)★ — 부팅마다 메모리의 `state.json` 원문으로 묻고(`durable:false`) 아무것도 저장하지 않아, `state.json` 이 `clean_exit:false` 로 남아 다음 부팅도 같다. 사본을 읽을 수 있게 될 때까지 이어진다. ★알리는 길(P3c2 · 후속 `125d190`)★ — 가드 ⅰ 은 main 위 안내 줄과 `restore.status` 의 `state_file` · 가드 ⅱ 도 main 위 안내 줄(ADR-0276 결정 4 · 답한 뒤에도 남는다)과 `restore.status` 의 `saves:false`(가드 ⅰ 도 `false`) · 묻는 동안은 모달의 거절 경고(`durable:false`)도 말한다(§6-5 ③ 가드 항목 — §14-15 의 열린 것을 §14-16 이 닫았다) | 수용(드묾) |
| R17 | 부팅 단계를 단일 인스턴스 바로 뒤 플러그인의 setup 에서 돌린다(§6-5) — 소스로 확인됨(tauri 2.11.3 `plugin.rs:880-915` · `app.rs:2440` · `2521` · 설계자 재리뷰도 확인) | GUI 확인만 남음 — P3b3 |

## 13. 「0. 결정」과 대조 — 다듬은 것

- **불가능한 항목은 없다.**
- 「기존 파일은 첫 부팅 때 한 번 옮긴다」 → **뒤집혔다(사용자 결정 2026-10-02)** — 옛 데이터는 옮기지 않고 버린다. 새 코드는 옛 파일을 읽지도 지우지도 않는다(§3-2 · §14-5).
- 「접근은 범용 명령 넷뿐」 → 설정에 대해서는 그대로. **창별 테마는 상태라 `window.setTheme` 이 필요하다**(§5-6).
- 「화면에서 바꾼 테마도 저장」 → 오늘 화면에 테마 UI 가 없다(ADR-0167 결정 5). 저장되는 것은 명령으로 바꾼 테마이고 화면 UI 는 F7 — **이번엔 두지 않는다(F7 = (a) · 사용자 결정 2026-10-06)**.
- 「챗 스타일을 localStorage 에서 여기로」 → 권위 이전은 그대로. **지금 11키와 기본값만 그대로 옮기고 `chat.style.*` 는 임시 이름공간이다**(U2 · §5-1). 옛 localStorage 값은 가져오지 않는다(U1 · §3-6).
- 「LLM 이 `ui-settings.json` 을 직접 고치던 경로를 폐지」 → 그대로. 사용자가 본인뿐이라 그 파일의 값은 다루지 않는다 — `settings.json` 은 기본값에서 시작한다(U1 · §3-5).
- 「슬롯: 꽂힌 내용 = `state.json`」 → ~~슬롯 보기 모드(터미널/챗 — 렌더러 덮어쓰기, ADR-0078 의 출력 형식과 별개)도 프론트 메모리에서 셸 소유 슬롯 값으로 옮겨 함께 싣는다(사용자 결정 2026-10-02 · §6-0 · P3e).~~ → **뒤집혔다(사용자 결정 2026-10-07)** — 보기 모드 덮어쓰기는 싣지 않고 프론트 메모리에 그대로 둔다. 기본 보기는 매번 출력 형식에서 정해지므로 남길 것은 덮어쓰기뿐이고 아직 필요 없다(P3e 뺌 · §6-0 · §14-20 · T-51).
- 「복원 정책 = 항상 복원, 비정상 종료 뒤에만 묻는다(Chrome 방식) — `cleanExit` 표식」 → 그대로(표식 = `state.json` 의 `clean_exit` — §6-6). 비정상 종료 뒤의 화면은 사본 `state.crash.json` **한 개**로 떠 두고 기본 화면에서 **늘**(모양 무관 — D5) 묻는다(D2 · D3). 묻는 표면은 main 창 웹뷰 안의 **모달**이고(D4) 답만 사본을 푼다(D6 · §6-7). 4판의 「최근 3 개 회전 보관」은 철회했다(D1 · §14-9).
- 「창마다 재시작해도 유지되는 id」 → 영속 id(UUID)를 따로 두고 label 은 부팅마다 새로 뽑는다(§6-3).
- 「모르는 종류 허용(ADR-0060)」 → 영속 DTO 에서만 허용하고 원문을 보존한다. 그 슬롯은 점유로 세고 아이콘 자리표시를 그린다(§6-2).
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
| R5 | L2 의 `ui-settings.json`: `theme` 는 있든 없든 못 쓸 값이든 무시 — 지금은 못 쓸 `theme` 하나가 파일 전체(창별 칸 포함)를 거부한다(`ui_settings.rs:300,342-358` — `c93e664` 기준 · 그 파일은 P3d 에서 지웠다) | §3-5 P2a · §8 설정 · GUI 확인 P2a |
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
| **못 쓰는 파일 = 옮기기 → 떠 둔 뒤 원자 교체** — 파일이 없는 순간이 없다 · 사본 실패 = `INTERNAL` · 원본 그대로 · ~~P2a 의 이름 · 한도 · 중복 생략 규칙~~ → **ADR-0274 가 대체**(사본 이름 · 덮어쓰기 · 한도의 정본 = §5-3 · §14-12) | §5-3 · §10 F21 · §12 R7 |
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
| **D2 — 크래시 사본 = 정확히 한 파일 `shell\state\state.crash.json`.** 평소: 바뀔 때마다 약 1초 디바운스로 `state.json` 을 통째로 원자 쓰기(임시 + rename · P2a 공용 `fsutil::write_atomic`) · 시작 때 「실행 중」 표식, 정상 종료 때 「정상」(Chromium `exit_type`) · 정상 종료 뒤 부팅 = 조용히 복원(「항상 복원」 그대로) · 비정상 종료 뒤 부팅 = ① 무엇이든 `state.json` 을 쓰기 **전에** `state.json` → `state.crash.json` 한 번 복사(4판 대기분 `state.pending.json` 을 통째로 대신한다) ② 기본 화면으로 시작 ③ 「복원할까요?」 — 사람과 LLM 이 한 핸들(`restore.answer` 그대로) ④ 복원 = 사본 내용을 화면에 적용하고 그 상태가 `state.json` 에 쓰인 **뒤에만** 사본 삭제(3판의 「디스크 확정 뒤에만 삭제」 원칙 그대로) ⑤ 거절 = 사본 삭제 ⑥ 부팅에 사본이 이미 있으면(답하기 전에 다시 죽었다) 더 새 `state.json` 으로 **덮지 않는다** — 앞 사본이 아직 답하지 않은 것이고 새 `state.json` 은 기본 · 부분 화면이다 · 그대로 두고 다시 묻는다 · 못 쓸 `state.json` / 사본 = 빈 채 시작 · 설정처럼 `.corrupt-<ms>` 로 떠 둠 · 묻지 않음(§5-3 과 같게) — ★떠 둘 이름은 ADR-0274 가 고정 이름 `.corrupt` 하나로 바꿨다(§14-12)★ | §2-1 · §6-1 · §6-2 · §6-4 · §6-5 · §6-6 · §6-7 · §8 · §9-2 P3a–P3c · GUI 확인 · §12 R11–R15 |
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
| **F15** — 「개발 빌드도 예외 없이 항상 묻는다 — 나중에 짜증나면 요청」(2026-10-02). 대가: `/qa` GUI teardown 이 강제 종료(`scripts/launch-detached.ps1:75`)라 다음 QA 기동에 모달이 뜬다 → `.claude/skill-bindings/qa.md` §full 을 고친다(~~기동 뒤 `crash_copy == awaiting` 이면 `restore.answer {accept:false}` · 또는 teardown 을 `quit_app` 으로~~ → P3c2 실제: QA 가 띄운 스크래치 폴더에서만 수락 · 그 밖의 폴더는 답하지 않고 주인에게 묻는다 · teardown 그대로 (§10 F15 · §9-2)) | §10 · §9-2 P3c2 · §14-11 N2 |

**결정에서 따라 나온 것:** D4 + D6 → 부팅에 사본이 있으면 늘 답하지 않은 것 → 늘 기본 화면 + 모달. 5판 「`clean_exit:true` + 사본 → 조용히 복원」 행을 걷었다. R13(둘째 세션 화면 손실)은 거의 사라졌다 — 답 전엔 앱을 못 쓰므로 남는 것은 버스(LLM) 변경뿐이다.

**지운 것(결정이 군더더기로 만들었다):** `is_default_shape`(D5) · F12 · 띠 · `restore_prompt_shown`(D6) · `Final{drop_crash_copy}` · `DropCrashCopy` · `Flush`(D6 · I2 — 요청은 `Resolve` · `Final` 둘) · 지문 재확인과 그 갈래(I1 잠금이 대신한다) · 「쓰지 않음」 모드(D8 — 사본 뜨기 실패 하나만 남겼다, 아래) · `restore.status.pending`(I2) · 5판 R15.

**세션 판단 [고름] — 오케스트레이터 지시 I1–I8**

| # | 판단 | 근거(리뷰 번호) | 반영 자리 |
|---|---|---|---|
| I1 | 셸 실행 잠금 `shell\run\state.lock` — 프로세스 내내 쥐고 `Final` 뒤에 놓는다 · 새 인스턴스는 판정 전에 약 3초 기다리고 못 잡으면 log 후 진행한다 · 임시 파일은 자기 pid · 죽은 pid 것만 지운다. 단일 인스턴스 플러그인이 `RunEvent::Exit` 에서 뮤텍스를 우리 `Final` 보다 먼저 놓는다(`windows.rs:111-123` · `app.rs:1430-1432` · `2644-2648` — 소스 확인) | 설계자 #3 · 설계자-파괴자 F3 | §2-1 · §6-5 ①② · §6-6 · §8 잠금 · §12 R11 · §13 |
| I2 | 답의 디스크 고정 — 답 뒤 첫 `state.json` 쓰기가 `resolved_crash_copy: <사본 바이트 해시>` 를 싣고, 부팅은 해시가 같은 사본을 묻지 않고 지운다 · 기록기는 지울 때까지 뒤 쓰기마다 다시 지운다 · `restore.status` 의 `pending` → `crash_copy` = `none` · `awaiting` · `answered` · 답한 뒤 `restore.answer` = `CONFLICT` | 설계자-파괴자 F1 · F12 | §6-1 · §6-4 · §6-5 표 · §6-7 · §8 |
| I3 | 읽기 IO 실패 ≠ 못 쓸 파일 — 잠김이면 5 × 20 ms 재시도 · 그래도 실패면 log 하고 「없음」으로 친다 · 그 부팅은 ~~떠 두기 · 지우기 · 사본 뜨기를 하지 않는다~~ → **그 파일의** 떠 두기 · 지우기 · 덮기만 하지 않는다(P3c1 — 파일마다 · 세션 판단(사용자 위임) 2026-10-04 · 사용자 확인 2026-10-05 · §6-5 ③ · §14-14) · 덮일 위험은 §12 에 적는다 | 설계자-파괴자 F4 | §6-2 · §6-5 · §8 · §12 R16 |
| I4 | 상한 — `state.json` 읽기 4 MiB · 기록기는 넘으면 안 쓴다(`Failed` / `INTERNAL` + log) · 떠 두기 상한 ≥ 읽기 상한 → `copy_aside_at` 을 `fsutil` 로 옮기며 상한 · 로그 `module` 을 인자로 — ★P3a 에서 떠 두기 상한은 ADR-0274 로 걷혔고(`fsutil::copy_aside` · 로그는 부르는 쪽) · 다시 읽히지 않는 중첩도 쓰기 거절이 됐다(§14-12)★ | 설계자-파괴자 F5 | §5-3 · §6-2 · §6-4 · §9-2 P3a |
| I5 | 복원 상태의 단일 출처 — 사본을 만드는 그 단계(부팅 단계 ⑥)가 서비스 상태를 정하고 `restore:changed` 를 낸다(Tauri 는 설정 창을 사용자 setup 앞에서 만든다) | 설계자-파괴자 F9 | §6-5 ⑥ · §6-7 · §8 |
| I6 | setup 순서 — main · 트리 창의 위치 · 크기 · 최대화 · 테마 입히기(⑧)는 실행 표식 뒤 · P4 의 정적 창 생성도 표식 뒤 | 설계자-파괴자 F10 | §4 · §6-5 ⑧ · §9-2 P4 |
| I7 | 단계 자르기 — P3a 는 `mod state;` 만 · 파일 겹침으로 상태 코어 + `fsutil` → `manager.rs` 곁표 → 순수 부팅 판정 → `lib.rs` 배선 · 사본 만들기와 답을 같은 단계에 · 크기 재추정 · 단계마다 혼자 초록 | 설계자 #4 · 설계자-파괴자 F11 | §9-1 · §9-2 |
| I8 | rename 직전 확인 변형은 재시도마다 다시 묻고 `Err` 아닌 「건너뜀」을 돌려준다 · 해결 칸은 기록기가 꺼낼 때 선다(보내는 쪽 원자 값 금지) + 시험 | 설계자-파괴자 7 | §6-4 · §8 기록기 · fsutil |

**세션 판단 [고름] — 이 판을 쓰며 고른 것(재리뷰가 볼 것)**

- **부팅 단계의 자리 = 단일 인스턴스 플러그인 바로 뒤에 등록한 플러그인의 setup**(§6-5) — I1 의 「판정 전에 잠금」을 지키면서, 빌드 전 대기(관문 앞이라 앱이 떠 있을 때 다시 실행하면 매번 3초)를 피한다. `LayoutState` 는 빌더에서 빈 채로 manage(ADR-0102 그대로)하고 부팅 단계가 그 안을 채운다(§12 R17).
- **실행 표식(첫 쓰기)을 부팅 단계로 옮겼다** — 어느 창보다 먼저가 되어 I6 순서가 저절로 서고, 기록기의 `Flush` 가 필요 없어졌다.
- **사본 뜨기 실패만은 그 실행의 `state.json` 쓰기를 막는다**(§6-5 ④) — 원칙의 예외(덮으면 아직 뜨지 못한 크래시 화면을 잃는다). 5판 「쓰지 않음」 모드에서 남긴 유일한 갈래다.
- **해시 = 64비트 FNV-1a 직접 구현**(§6-1) — std `DefaultHasher` 는 판마다 같은 값을 약속하지 않아 디스크에 못 싣는다. 신원 확인용이라 새 의존성을 들이지 않는다.
- ~~**상태 떠 두기 상한 16 MiB**(§6-2) — 읽기 상한(4 MiB)보다 커야 상한 초과로 못 쓴 파일도 떠 둔다.~~ → **걷었다(ADR-0274 — 떠 두기에 크기 한도 없음 · §14-12).**
- **잠금 = `File::try_lock`**(§6-5 ①) — 플랫폼 중립이고, 이 파일은 아무도 읽지 않는다.
- **수락은 main · 트리 창 속성도 사본 값으로 입힌다**(§6-7 ④ — ~~부팅 ⑧과 같은 함수~~ → ⑧ 의 main · 트리 자리 입히기 도우미만 쓴다 · 부팅 전용인 `restore_windows` 통째가 아니다, §6-5 ⑧) · **조율자는 부팅 때 읽은 사본 바이트를 쓴다**(다시 읽지 않는다).
- ~~F15 를 (a) 로 닫았다 — D3 · D5 를 개발 빌드에도 그대로 읽었다~~ → 사용자 결정으로 바뀌었다(위 결정 표 · §14-11 N2).
- **모달은 main 창만 막는다**(§6-7) — 다른 창(트리)은 막지 않는다.

### 14-11. 6판 재리뷰 반영 (2026-10-02 · 설계자 PASS · 설계자-파괴자 FIX)

설계자(codex)는 PASS(플러그인 순서를 tauri-2.11.3 소스로 확인). 설계자-파괴자의 국소 지적 8건을 반영했다 — 판은 6판 그대로다.

| # | 반영 | 자리 |
|---|---|---|
| N1 | 사본은 **지우기 직전에 다시 읽어(≤4 MiB) 칸의 해시와 같을 때만** 지운다 — 기록기 `Resolve` 재시도와 부팅 ④ 둘 다. 잠금 없이 진행한 길과 2초 마감 경합에서 낡은 기록기가 새 인스턴스의 사본을 지우지 못한다 | §6-4 · §6-5 표 · §8 |
| N2 | **F15 = 사용자 결정**(「개발 빌드도 예외 없이 항상 묻는다 — 나중에 짜증나면 요청」 · 2026-10-02 — 6판의 「세션 판단」을 바로잡음). 대가: `/qa` GUI teardown 이 강제 종료라(`scripts/launch-detached.ps1:75`) 다음 QA 기동에 모달이 뜬다 → P3c2 가 qa 바인딩 §full 을 고친다(→ P3c2: 스크래치 폴더에서만 수락 · teardown 그대로 (§10 F15 · §9-2)) | §10 · §14-10 결정 표 · §9-2 P3c2 |
| N3 | 가드를 하나로 — `state.json` 을 못 읽었거나(IO 실패) 사본을 못 떴으면 ⑤ 건너뜀 · 기록기 없음 · log. IO 실패 뒤 덮어쓰는 위험이 사라졌다 | §6-5 ③ · §12 R16 · §8 |
| N4 | 모달이 떠 있는 동안 키 바인딩 디스패처(`src/commands/keybindings.ts` 의 `installKeybindings`)도 멈춘다. 막는 것은 사람 UI 뿐 — LLM 버스는 막지 않고 관례로 먼저 `restore.answer` 를 낸다(버스 차단 장치는 만들지 않는다) | §6-7 · §8 프론트 모달 · §10 F11 · §12 R13 · §9-2 P3c2 |
| N5 | 어느 모니터에도 안 걸치는 팝아웃 위치는 버린다(4판 규칙 되살림) — 부팅 ⑨ · 조율자 ②, ⑧과 같은 함수 | §6-5 ⑧⑨ · §6-7 ② · §8 |
| N6 | 못 쓸 `state.json` 은 사본 열과 무관하게 떠 둔다 · 버전 초과 사본(하향)은 지우지 않고 남겨 둔 채 묻지 않는다 | §6-5 표 · §6-7 · §12 R3 |
| N7 | 부팅 플러그인 setup 은 `Err` 를 돌려주지 않는다 · 로그 초기화를 그 첫 단계로(상태 쪽 진단 모아 두기 삭제 · 설정의 적재 기록 되풀이는 그대로 — P2a 무수정, 걷는 길만 적었다) · 가드면 `RunEvent::Exit` 가 `Final` 을 건너뛰고 잠금만 놓는다 | §6-5 ⓪ · §6-6 · §8 |
| N8 | P3c1(~1,000)을 겹침 없는 둘로 — P3c1(사본 열 · 서비스 · 명령 · 조율자 — Rust) · P3c2(모달 · 키 바인딩 멈춤 — 프론트). 「사본은 답할 길과 함께」는 `restore.answer` 가 버스에 서는 P3c1 에서 충족된다. 옛 P3c2(슬롯 두 상태)는 P3c3 | §9-1 · §9-2 · GUI 줄 · 병렬 줄 |
| 그 밖 | R17 = 소스로 확인됨(tauri 2.11.3 `plugin.rs:880-915` · `app.rs:2440` · `2521`) — GUI 확인만 남음 · 잠금 전에 `shell\run` 을 만든다 | §12 R17 · §6-5 사실 ③ · ① |
| L1–L3 | 마무리 문구(재리뷰 PASS 뒤): ③ 「사본 있음 = 답 없음」에 너무 새 판 예외(N6) · 가드 ⅱ 면 ⑥이 `awaiting` + 메모리 `state.json` 원문을 복원 원천으로(수락 `durable:false` · 다음 부팅이 다시 묻는다 — [고름]) · R3 에 「옛 판 크래시 뒤엔 새 판이 답할 때까지 저장 없음」 | §6-5 ③⑥ · §8 · §12 R3 |

### 14-12. P3a 코드 리뷰 반영 (2026-10-03)

P3a(`41348d9` — `src-tauri/src/state/{schema,codec,saver}.rs` · `fsutil.rs` · `settings/store.rs`)의 코드 리뷰(deep · 교차 family)가 바꾼 것을 사양에 맞췄다. 같은 자리에서 사용자 결정 F21 을 ADR-0274 로 박제했다(§10 · §5-3 · §6-2 — ADR-0265 결정 4 부분 폐기). GUI 실측은 설정 쓰기 · `.corrupt` 덮어쓰기만 했다(상태 쪽은 배선 전이라 시험만).

| 바뀐 것 | 반영 자리 |
|---|---|
| **임시 이름에 프로세스 안 번호** — `<이름>.tmp<pid>.<n>`. 같은 프로세스의 두 호출이 임시 이름을 나눠 쓰면 뒤의 생성이 앞의 임시 파일을 비우고 앞의 rename 이 그 반쪽을 `Ok` 로 갈아끼운다. 설정 `io` 락의 사유도 「임시 이름 충돌」이 아니라 「겹친 RMW 사이의 갱신 유실」로 고쳐 적었다 | §5-3 · §6-1 · §6-4 · §6-5 ② |
| **떠 두기의 원본 열기도 잠김 재시도** — rename · 읽기와 같은 규칙(5 × 20 ms) | §5-3 |
| **읽기 전용 원본도 떠 둔다** — `std::fs::copy` 대신 흘려 쓴다(원본 권한을 옮기면 사본도 읽기 전용이 되어 `sync_all` 할 쓰기 핸들을 못 연다) · 시험 | §8 fsutil |
| **코덱이 다시 읽히지 않는 중첩을 거절**(`EncodeError::Unparsable` — serde_json 128 단 · 분할 121겹) · 거절된 스냅숏은 다음 변경을 기다린다 — I4 를 넓혔다 | §6-4 · §8 기록기 · 상태 코덱 |
| **상태 `version` 이 정수 값 실수(`1.0`)를 받는다**(설정과 같은 관용) · `u64` 범위 밖 = 상태 파일 아님 | §6-2 · §8 상태 코덱 |
| **영속 DTO ↔ 메모리 타입 맞대기 시험이 모든 칸을 이름으로 풀어 쓴다** — 칸 · 변형이 늘면 컴파일이 깨진다. P3b1 의 변환도 같은 수법 | §9-2 「P3a 가 넘긴 것」 |
| **스냅숏 포트가 실패할 수 있다**(`Result`) · 패닉 금지 · `finish` · `resolve` 를 부른 스레드를 기다리지 않는다(스냅숏 안 창 게터 금지)를 포트 문서에 · 변경 번호는 같은지만 보고 되돌리지 않는다 | §6-4 |
| **쓰기와 사본 지우기 사이에 닫히면 지우지 않는다** — 시험 | §8 기록기 |
| **실패 로그를 갈래(스냅숏 · 코덱 · 디스크)마다 접는다** · 회복은 info 한 줄 | §6-4 |

**받아들이고 고치지 않은 것**

- **떠 두기의 영구 실패 극단 경우** — 사본 자리가 영구히 못 쓰는 상태면(읽기 전용 · 같은 이름의 폴더 · ACL) 복사가 실패해 그 파일의 쓰기가 계속 실패한다. 사용자 원칙 「그런 것까지 가정하면 끝도 없음」(ADR-0274 대가 2).
- **R12 — `closed` 확인과 rename 사이의 틈** — 이미 수용했다(§12 R12).
- **긴 임시 이름(경로 길이)** — 고정된 짧은 파일 이름(`settings.json` · `state.json` · `state.crash.json`)에만 붙는다.

**같은 날 세션 판단(사용자 위임) 2026-10-03:** 상태 파일 칸 더하기 규칙 — 정본 = §6-1.

### 14-13. P3b 코드 리뷰 반영 (2026-10-03)

P3b1 + P3b2(`5a2cf7f` — `layout/manager.rs` 곁표 · 창 속성 · `state/{convert,tree_attrs,boot,lock}.rs` · `fsutil.rs`)와 P3b3(`f9db35a` — `state/{boot_plugin,placement}.rs` · `lib.rs` 배선)의 코드 리뷰(`/review code` — P3b1a · P3b1b full · P3b2 deep · P3b3 deep, 교차 family)가 바꾼 것을 사양에 맞췄다. P3b3 조각 2 는 재수정 3회 상한에 닿아 마지막 라운드(숨긴 main 최대화 · 문서)를 리뷰어 재실행 없이 메인 diff 확인으로 닫았다. 사용자 결정 = F8 (a) · F13(`--hidden` 전용 로직 없음) · 세션 판단(사용자 위임) = F9 (a) · 분할 깊이 상한 없음(§10 · §9-2). GUI 실측 = P3b1 · P3b2 는 스모크 · P3b3 는 8단계(첫 부팅 · 꾸미기 기록 · 정상 종료 `clean_exit:true` · 재실행 복원 · 최대화 왕복 · 잠금 대기 · `taskkill` → 기본 화면 · 배율 125% / 200% 두 모니터).

| 바뀐 것 | 반영 자리 |
|---|---|
| **영속 `bounds` = `Option`** — `null` = 보통 자리를 본 적 없다 · 자리 때문에 창을 빼지 않는다(읽기 규칙 = §6-1) | §6-1 |
| **`from_persisted` 수리 목록 확정**(`RestoreWarning`) — 버려지는 창은 아무 id 도 쥐지 않는다 · 결과의 번호는 의미가 없다 외(목록 · 자리 = §6-2) | §6-2 |
| **곁표 무효화 진입점** = 내용 쓰기 문 `write_slot_content` · 뷰 지우기 문 `remove_view` · `close_slot` — `tree::assign_in_tree` 걷음 · `resolve_spawn_slot` 메서드화 · 슬롯을 빼내면 원문이 따라간다 | §6-2 · §12 R6 |
| **창 속성 = `WindowTabs.attrs`(`WindowAttrs`) 묶음** · `attrs_rev` 는 값이 바뀐 쓰기에만 오르고 `ViewManager` 의 비공개 칸이다 · 트리 칸 `TreeAttrs` = 테마 · 위치(최대화 표식은 늘 내린다 · 보임 미복원 — F9) | §6-3 |
| **자리 단위 = 물리 바깥 위치 + 논리 안쪽 크기** · 위치 → 크기 → 최대화 순으로 입힌다(배율 다른 모니터 — tauri-plugin-window-state `3d8a3c877b` 참고 · 조각 2 FIX) | §6-3 |
| **최대화 직전 읽기를 되돌리는 한 칸 메모 `PlacementMemo`**(같은 자리일 때만 — 조각 2 FIX) · 연 창의 첫 자리 기록 `WindowHost::record_placement` · 모델에 없는 label 의 사건은 버린다 | §6-3 |
| **숨은 main 의 최대화는 처음 보일 때로 `DeferredMaximize`**(미룬 동안 main 자리를 적지 않는다 — 조각 2 FIX) → P3c1 에서 label 마다(main · 런타임 수락의 숨은 팝아웃 — §14-14) | §6-5 ⑧ |
| **`restore_windows` · `open_restored_popouts` = 부팅 전용** — 런타임 수락은 부팅 전용 함수 대신 공유 자리 도우미(`locate` · `place` · `place_main` 등)를 쓴다 | §6-5 ⑧ · §6-7 ② ④ · §9-2 「P3b 가 넘긴 것」 |
| **판정 결과의 가드 = `BootPlan.guard`**(계약 = §6-5 ③ — 설계의 `write: bool` 을 대신한다) · ④ 의 결과는 `run_actions` 가 계획에 접어 넣는다(P3c1 확장 지점) | §6-5 ③ ④ |
| **`--hidden` 은 따로 다루지 않는다(F13)** — 복원한 팝아웃도 보이는 채 만들고 setup 끝 숨기기가 함께 숨긴다 · 창 생성은 죽은 창 테마 쓸기 뒤 | §6-5 ⑨ · §10 F13 |
| **스냅숏 원천 `LiveSource`** — 창 게터 없음 · 짧은 레이아웃 락 · 독 든 락 = `Err` | §6-4 |
| **종료 = `StateSession::shutdown`** — 잠금을 명시적으로 놓는다(이벤트 루프가 끝나면 곧장 `process::exit` 해 drop 이 돌지 않는다) | §6-6 |
| **잠금 로그는 실제로 막혔을 때만 「기다려 잡았다」**(두 시각의 차는 거의 늘 0 이 아니다) | 코드만(`state/lock.rs`) |
| **수락 커밋 규칙** — 번호를 되감지 않는다 · 두 번호를 올리는 새 메서드(규칙 전부 = §6-7 ③) | §6-7 ③ · §9-2 P3c1 행 · 「P3b 가 넘긴 것」 |
| **프론트 `applyLayoutUpdated` 가 `foreign_slots` 를 버린다** → P3c3 없이 복원을 내보내지 않는다 | §9-2 P3c3 행 · 「P3b 가 넘긴 것」 |
| **분할 깊이 상한 없음**(세션 판단(사용자 위임)) — 코덱의 121겹 쓰기 거절이 유일한 벽 | §9-2 「P3a 가 넘긴 것」 · §6-4 |
| **잠금 대기 · 배율 다른 두 모니터** — GUI 실측 | §12 R11 · R5 |
| **(2026-10-04 문서 리뷰 · 도출 — 사용자 결정 아님) 이름 바꾸기 규칙** — 옛 이름은 `#[serde(alias)]` 로 읽게 남기고 `version` 만 올리지 않는다(규칙 · 까닭 = §6-1) | §6-1 · ADR-0140 주석 |
| **(같은 리뷰) 수락의 트리 칸 교체는 커밋 임계구역 밖** — 원자 단위가 아닌 틈을 받아들였다(해롭지 않은 까닭 = §6-7 ③) | §6-7 ③ |
| **(같은 리뷰) F13 의 대가** — `--hidden` 부팅에 복원한 팝아웃도 main 처럼 잠깐 떴다 숨는다 · P4 의 §4 「덤」은 main 만 고친다 [미검 — GUI] | §10 F13 |

**검증 안 된 것** — `--hidden` 부팅 + 미룬 최대화 경로(지금은 타지 않는다 — §6-5 ⑧) · `WM_DPICHANGED` 가 위치 세터 안에서 처리되는지(§12 R5) · 트레이 메뉴 클릭 자체(GUI 실측은 `quit_app` invoke 로 대신했다) · 비 Windows 전부.

### 14-14. P3c1 코드 리뷰 반영 (2026-10-04)

P3c1(크래시 사본 + 답할 길 — 슬라이스 A = 사본 열 · 복원 서비스 · B1 = 복원 조율자 · B2 = `restore.*` 명령 · 도움말 · ✅ 착지 `064b61c` 2026-10-05)의 코드 리뷰(`/review code deep` · 3렌즈 — 슬라이스마다)가 바꾼 것을 사양에 맞췄다. 착수 때 계약 = `.claude/handoff/attachments/p3c1/p3c1-interface.md`(최종 모양은 코드 — `windows` 셈 · 포트 모양이 달라졌다). 재수정 상한(3회)에 닿은 뒤 **넷째 · 다섯째 수정 라운드는 사용자 승인으로 돌았다** — 넷째의 계획 = `.claude/handoff/attachments/p3c1/pending-fix-round4.md` · 다섯째는 목록 문서가 없어 아래 「4·5라운드」 행은 커밋된 코드에서 읽었다(두 라운드를 가르지 않는다). 사용자 결정 2026-10-05 = 도움말의 답 정책 셋(아래 「닫은 것」 — 같은 자리의 리뷰 도출 둘과 가른다). 세션 판단(사용자 위임) 2026-10-04 = I3 파일마다 · main 최대화를 사본대로 양쪽 · 복원 팝아웃이 main 의 보임을 따름 · 숨긴 채 둔 최대화 팝아웃은 보일 때 최대화(D2 와 같은 규칙) — 넷 다 사용자 확인 2026-10-05(「세션 판단을 그대로 둔다」 — §14-15). 계획 · 상태 모양의 정본 = §6-5 ③(여기 되풀지 않는다).

| 바뀐 것 | 반영 자리 |
|---|---|
| **I3 은 파일마다**(세션 판단(사용자 위임) · 리뷰 3렌즈 모두 권고) — 읽기 IO 실패는 그 파일의 동작만 막는다(갈래 = §6-5 ③) | §6-2 · §6-5 ③ · §8 부팅 판정 · §14-10 I3 행 |
| **표가 말하지 않던 칸 셋을 코드가 정했다** — 해시 이어 싣기 · 떠 두지 못한 못 쓸 사본 · 사본 IO 영구 실패(내용 = §6-5 ③) | §6-5 ③ 표 · §12 R16 |
| **처리 중 표지 = 복제 불가 · 버리면(패닉 포함) 되돌림 · `set_boot` 세대 번호**(락 · 알림 규칙 = §6-7) | §6-7 복원 서비스 |
| **되돌림(`RolledBack`)도 `restore:changed`(`awaiting`)** — `InFlight` 로 거절당한 쪽이 다시 답할 수 있음을 안다 | §6-7 복원 서비스 |
| **`restore.status` 의 `windows` = main + 팝아웃(트리 제외)** — `restored_windows` 와 같은 방식(값은 다를 수 있다) | §6-7 명령 |
| **조율자 포트는 setup 끝(⑨ ⑩ 뒤)에 꽂는다** — 그 전 수락 = `INTERNAL` · 거절은 선다 · 구독 원천은 쓸 때마다 찾는다 | §6-7 조율자 포트 |
| **숨긴 창은 따로 `build_hidden_runtime_window`** · `place` 는 비공개 그대로 · 창 포트 실물은 `placement.rs` | §6-7 ② ④ · §6-5 ⑧ · §9-2 「P3b 가 넘긴 것」 |
| **`adopt_restored`** — 옮기는 중인 임자 없는 View 는 남긴다 · 거절이면 아무것도 안 바꾼다 · 최대화 메모를 비운다 | §6-7 ③ |
| **`create_window` 가 OS 창을 만든 뒤 모델을 다시 본다**(그 사이 모델에서 지워졌으면 거둔다 — 고아 창) | §6-7 ③ |
| **미룬 최대화를 label 마다**(main · 복원 팝아웃) · 모든 보이기 경로가 입힌다(숨긴 채 둔 최대화 팝아웃도 보일 때 최대화 — 세션 판단(사용자 위임)) · 팝아웃 소멸이 거둔다 | §6-5 ⑧ · §6-7 ② ④ · §14-13 |
| **main 최대화는 사본을 양쪽으로 따른다**(세션 판단(사용자 위임)) · 미룸은 자리를 입히기 전에 정한다 | §6-7 ④ |
| **복원 팝아웃은 main 의 보임을 따른다**(ADR-0229 트레이 의미 — 세션 판단(사용자 위임)) | §6-7 ④ |
| **② 와 ③ 사이에 닫힌 팝아웃 → `forget_vanished`** · 거두기 실패는 한 번 더 | §6-7 ② ④ |
| **답은 메인 스레드에서 돌지 않는다** — 버스 `offloaded_handler` · invoke async + `spawn_blocking` | §6-7 명령 |
| **`resolve_crash_copy`** — 기록기가 없으면 먼저 띄운다 · 띄우는 중이면 기다린다 · 종료가 띄우는 중 기록기를 닫는다(`CloseFlag` — 차례 · 까닭 = §6-7 ⑤) | §6-7 ⑤ · §6-5 ⑦ |
| **버스 `crash_copy` = 문자열** — 매크로 enum 철자가 serde 철자와 달라 `as_wire` · 시험이 serde 철자에 묶는다 | §6-7 명령 |
| **`schema.rs` 머리에 앞 버전 리더 단서 · 이름 바꾸기 규칙** | §6-1 |
| **(4·5라운드) 끝의 main 포커스는 우리 앱이 앞일 때만** — 창을 만지기 전(① 뒤 · ② 앞) 표본 + 부르기 직전 다시 읽기 · 판정 = `foreground_is_ours`(Windows 에서 `is_focused` 를 쓰지 않는 까닭 = §6-7 ④) · 의존성 `windows` 0.58 | §6-7 ④ · CLAUDE.md 「의존성」 |
| **(4·5라운드) 미룬 최대화는 입혀졌을 때만 거둔다** — 못 입히면 남기고 자리를 적지 않는다 · 보이는 main = `apply_now` · 팝아웃 미룸은 창을 만들기 전에 세운다(차례 · 까닭 = §6-7 ② ④) | §6-5 ⑧ · §6-7 ② ④ · §8 |
| **(4·5라운드) 창 포트 모양** — `app_has_focus` 를 더하고 `open_hidden` 이 `maximized` 를 받는다 · 따로 서던 미룸 세우기 `defer_maximize` 를 걷었다 | §6-7 ④ |
| **(4·5라운드) 셸이 뜨는 중의 수락 = `AnswerError::NotReady` → 맨 `INTERNAL`** — 계획한 재시도 표지는 싣지 않았다(까닭 = §6-7 「오류」 · ADR-0159) · 문구가 「다시 답하라」를 싣는다 | §6-7 명령 · 조율자 포트 |
| **(4·5라운드) 도움말 · 명령 설명** — 답 정책 · 차례(아래 「닫은 것」 — 사용자 결정과 리뷰 도출을 가른다) · 답한 뒤 `window.list` · `tab.list` 다시 읽기(팝아웃 label 이 새로 매겨진다) · 「`windows` 와 같은 방식으로 세지만 같지 않을 수 있다」 · `TIMEOUT` 이면 끝까지 진행됐을 수 있어 `restore.status` 로 확인 | §6-7 모달 · 명령 |
| **(4·5라운드) `layout/commands.rs` 이름 주석의 손 셈 숫자를 걷었다**(「아래는 예이고 전부가 아니다」) | 코드만 |

**받아들이고 고치지 않은 것**

- **R12 — `closed` 확인과 rename 사이의 틈** — 이미 수용했다(§12 R12).
- **두 번 거두기가 모두 실패한 창(고아)** — Tauri 런타임의 창 거두기는 이벤트 루프에 메시지를 보낼 뿐이라(tauri-runtime-wry 2.11.3 `lib.rs` 의 `destroy`) 루프가 닫히는 중일 때만 실패한다. error 로그만 남긴다.
- **못 쓸 사본의 떠 두기가 실패해도 새 사본 쓰기를 막지 않는다**(codex 지적 — 「막아야 한다」) — ADR-0274 대가 2(사용자 원칙 「그런 것까지 가정하면 끝도 없음」)로 기각 · 나머지 두 렌즈도 수용. 로그가 「백업 없이 갈아끼운다」를 말한다.
- **커밋 뒤 패닉(개발 빌드만)** — 처리 중 표지가 버려져 `awaiting` 으로 되돌아가므로 화면은 바뀐 채 다시 답할 수 있게 된다. 릴리스는 `panic = "abort"` 라 닿지 않는다.
- **트레이 보이기 · 숨기기가 ④ 와 직렬화되지 않는다**(낮음) — 미룬 최대화 갈래만 다시 확인한다(main = `place_main` · 복원 팝아웃 = `open_hidden` 이 자리를 놓은 뒤).
- **부팅 ⑧ 에서 main 최대화가 실패하면 main 이 미룸에 남는다**(codex 지적 — 그동안 main 의 자리가 적히지 않는다) — 닿지 않는다: 부팅 단계는 메인 스레드에서 돌고, 메인 스레드에서 tauri-runtime-wry 2.11.3 의 `send_user_message` 는 메시지를 그 자리에서 처리하고 `Ok` 를 돌려준다(`lib.rs:235-255`). 메인 밖에서는 이벤트 루프가 사라졌을 때만 실패한다.
- **복원 팝아웃 보이기(`show()`)의 활성화 시도**(다른 앱이 앞일 때 수락하면 그 앱의 포커스를 뺏을 수 있다는 지적) — 실측: OS 전경은 옮지 않았다(GUI QA 깨끗한 3회 · `ForegroundLockTimeout` 200000 · 합성 입력 없음). 고치지 않는다.

**닫은 것 — 도움말의 답 정책(열려 있던 물음 = LLM 이 수락 · 거절을 스스로 정해도 되나)** — **사용자 결정 2026-10-05** = 어느 쪽으로 답할지는 주인이 정한다 · 주인이 시키지 않았으면 주인에게 묻고 답한다 · 거절은 사본을 지워(`durable` 이면) 되돌릴 수 없다(도움말에 그렇게 적는다). **리뷰 도출 2026-10-05**(사용자 결정 아님) = 팀원의 요청은 주인의 지시가 아니다 · 차례 = 창 명령을 처음 부르기 전(그리고 쥔 label · view_id 가 안 맞을 때) `restore.status` 를 본다. 자리 = `prompts/engram-help.md` `window` 구획 · `restore.status` · `restore.answer` 의 명령 설명(`layout/commands.rs`) · §6-7 모달 항목.

**검증 안 된 것**(`/qa full` PASS 뒤 남은 것 — §9-2 P3c1 행 · 그때 GUI 에서 부르지 않았던 Tauri `restore_answer`(사람 경로)는 P3c2 G3 이 모달로 불렀다 — §9-2 P3c2 행) — 비 Windows 분기 전부(`foreground_is_ours` 의 `is_focused` 갈래 포함 — Windows 빌드 · CI 에서 컴파일되지 않는다) · Windows 에서 `is_focused` 를 걷은 근거(웹뷰가 포커스를 쥔 동안 `false`)는 소스 독해다 · 숨은 main 이 최대화돼 있을 때 풀기(`SW_RESTORE`)가 창을 보였다 숨기는지(`placement.rs` `place_main` 주석) · 런타임 수락에서 새 팝아웃이 모델보다 먼저 뜨는 틈(§12 R5). 덤 관측: 부팅 직후 숨은 Agent Tree 창이 전경 창으로 읽혔다(QA 관측 · 원인 미조사 · 이번 변경과 무관).

### 14-15. P3c2 코드 리뷰 반영 (2026-10-06)

P3c2(복원 모달 · 키 바인딩 멈춤 · 상태 파일 안내 · `restore.status` 에 `durable` · `state_file` — ✅ 착지 `7b742fb`)의 코드 리뷰(`/review code full` · 2 family — Claude doc-aware · codex 블라인드 · Rust 조각 1 라운드 + 프론트 · 델타 2 라운드 · PASS)가 바꾼 것을 사양에 맞췄다. **사용자 결정 2026-10-05** = 상태 파일 안내(§6-5 ③ 가드 항목 — 깨진 파일 쪽은 「관행 — 세션 추천에 이의 없음」) · 확인 목록 확정 — §14-14 의 세션 판단(사용자 위임) 넷(I3 파일마다 · 복원 팝아웃은 main 의 보임을 따른다 · main 최대화는 사본을 양쪽으로 · 숨긴 채 둔 최대화 팝아웃은 보일 때 최대화)을 「세션 판단을 그대로 둔다」로 확인했다(표시 = 「사용자 확인 2026-10-05」). **세션 판단(사용자 위임 — 「쭉 진행해」)** = 안내의 화면 모양(main 위 닫을 수 있는 안내 줄 · 막지 않음). 검증 = §9-2 P3c2 행.

| 바뀐 것 | 반영 자리 |
|---|---|
| **`restore.status` 에 `durable` 을 더했다** — 프론트가 `state_file` 로 저장 여부를 짐작하던 것을 대신한다 — `state_file` 은 `state.json` 읽기만 싣는다(가드 ⅱ 에서도 `ok`) · 거절 경고는 `durable` 만 본다 | §6-7 명령 · 모달 · §6-5 ③ 가드 항목 |
| **`state_file` 은 ⑥ 이 사본 상태와 같은 `set_boot` 호출에서 정한다** · 떠 두기의 결과는 ④ 가 `BootPlan.state_aside` 에 적는다(`decide_boot` 는 순수 그대로) | §6-5 ③ ④ ⑥ |
| **저장 시각이 `Date` 로 나타낼 수 없는 값이면 그 줄만 뺀다**(`formatSavedAt` — 거르지 않으면 화면 전체가 오류 경계로 넘어간다) | §6-7 모달 |
| **내린 설치(재마운트 · 언마운트)의 당기기 답은 칠하지 않는다** | §6-7 상태 창구 |
| **부팅 당기기는 받을 때까지 놓지 않는다** — 유계 재시도 뒤 2초마다 · 알림이 부른 당기기가 먼저 받으면 멈춘다 | §6-7 상태 창구 |
| **문구** — 도움말 · `restore.status` 명령 설명 · `i18n/ko.ts` `restore`(지금 규칙: 「못 쓰는 파일」을 손상 하나로 단정하지 않는다 · 떠 두지 못한 경우의 `.corrupt` 를 이번 실행의 백업으로 말하지 않는다 — 그 파일의 주석) | 코드만 |

**검증 안 된 것** — `durable:false` 일 때의 모달 문구 경로(GUI) · `corrupt_not_copied` 안내(GUI — 시험만) · `unreadable` 안내의 닫기(닫기는 G7 의 `corrupt_copied_aside` 에서만 실측) · 트레이 메뉴 클릭 자체(같은 처리기 `quit_app` · `show_main_ui` 를 invoke 로 불러 대신했다) · 비 Windows 전부 · 상태 창구의 알려진 잔여(`listen()` 이 영영 안 풀리면 당기기가 안 나간다 — §6-7 상태 창구) · 로컬 워크스페이스 총계(이번엔 다시 재지 않았다 — CI 초록으로 갈음).

**열린 것(결정 아님 — 사용자에게 올릴 항목)** — 가드 ⅱ(떠야 할 사본을 못 떠 이 실행이 아무것도 저장하지 않는다)는 답한 뒤 남는 안내가 없다. 그 사실을 말하는 것은 묻는 동안의 `durable:false` 와 모달의 거절 경고 하나뿐이다(§12 R16). ★→ **닫았다** — 가드 ⅱ 도 같은 안내를 띄운다(2026-10-06 — 세션 적용안에 사용자가 답했다 · ADR-0276 결정 4 · 후속 `125d190` — §14-16)★.

### 14-16. 안내 덮기 후속 — 착지 기록 (2026-10-06)

후속 「안내 덮기 + 가드 ⅱ 안내 + `saves`」 = ✅ 착지 `125d190`(CI 초록 run 37351977245 — backend · frontend · fmt+격리 · 코드 리뷰 `/review code full` PASS — 2 family · Rust 1 라운드 + 후속 확인 · 프론트 3 라운드). 결정과 그 출처의 정본 = ADR-0276(ADR-0180 을 상태 파일 안내 하나에 한해 개정) · 지금 사양 = §6-5 ③ 가드 항목 · ⑥ · §6-7 명령 · 상태 창구 · §8 — 여기 되풀지 않는다.

**검증** — 로컬 게이트 PASS · 0 실패(수치 = `docs/process/step-log.md` 의 P3c2 후속 항목 · CLAUDE.md 「빌드·검증 명령」의 `lib_unit` 줄) · 워크스페이스 총계는 로컬에서 다시 재지 않았다(CI 로 갈음). `/qa full` PASS(스크래치 데이터 폴더 · `125d190` · GUI H1–H6):

- **H1** `restore_status` 의 칸 일곱.
- **H2** 깨진 `state.json` → 안내가 덮고 TabBar 를 가린다 · 레이아웃 칸의 사각형은 안내 없는 기동과 같다.
- **H3** 안내 줄을 누른 것은 TabBar 에 닿지 않는다 · 그 아래를 누른 것은 캔버스에 닿는다 · ✕ 뒤 TabBar 가 눌린다.
- **H4** 가드 ⅱ(`state.crash.json` 자리에 같은 이름의 폴더를 미리 만듦) → 모달의 거절 경고가 `durable:false` 쪽 · 수락 → `answered` · `saves:false` · 가드 ⅱ 안내가 남는다 · ✕ 로 닫힌다 · 레이아웃을 바꿔도 `state.json` 그대로.
- **H5** 못 읽는 `state.json`(같은 이름의 폴더) → `unreadable` 문구만 뜬다(가드 ⅱ 문구를 겹쳐 띄우지 않는다) · ✕ 로 닫힌다.
- **H6** 연결 띠(합성 — `disconnect()` 뒤 `reportConnectionError` · 연결된 동안에는 그 이유를 버린다)는 흐름에 남아 레이아웃을 민다 · 두 안내가 함께 뜬다. ★`125d190` 때의 기대다 — `9318b5c` 부터 연결 띠도 덮는다(ADR-0277 · 그 GUI 확인 = §14-17)★.

**검증 안 된 것** — 가드 ⅱ 에서 「새로 시작」(거절)의 GUI 경로(셸 단위 시험 `a_run_that_could_not_write_the_crash_copy_still_says_so_after_the_answer` 가 거절 뒤 `saves:false` 를 잰다) · 실 앱의 버스로 부른 `restore.status` 의 `saves`(통합 시험 `layout_commands` 뿐) · `33vh` 스크롤(GUI — vitest 의 구조 시험뿐 · 지금 덮는 층 안의 안내는 상태 파일 안내 하나라 쌓임이 실사용되지 않는다 · ★`9318b5c` 부터 연결 띠도 이 층에 쌓인다 — 쌓임의 GUI 확인 = §14-17★) · 실제 연결 실패로 연결 띠 띄우기(H6 은 합성) · 비 Windows 전부 · 로컬 워크스페이스 총계. 덤 관측: 다크 테마에서 덮는 층의 그림자가 거의 안 보인다(고치지 않았다).

**열린 것(결정 아님)**
- ~~**연결 띠(`ConnectionNotice` — main · 팝아웃 · 트리 창)도 덮을지** — 사용자 답 대기. ADR-0180 을 뒤집는 일이라 세션이 정하지 않고 사용자에게 올렸다 — 지금은 세 창 모두 흐름(밀기) 그대로다.~~ → ★**닫혔다(사용자 결정 2026-10-06 — 「덮기로해.」) — 세 창 모두 덮는다 · ADR-0277 · 착지 `9318b5c` — §14-17**★.
- **「줄 바깥 빈 영역」** — 사용자가 다음에 얘기하자고 미뤘다. 지금은 덮는 층이 안내 줄 높이만큼만이고(구현에서 나온 세션 판단 — 사용자 결정이 아니다) 줄 밖의 클릭은 그 아래로 간다(H3). ★→ 저장 시스템을 마친 뒤 올린다(사용자 결정 2026-10-06 · §14-18 「미룬 것」 · `docs/tracking.md` T-50)★.

### 14-17. 연결 띠 덮기 — 착지 기록 (2026-10-06)

「연결 띠 덮기」 = ✅ 착지 `9318b5c`(CI 초록 run 37404870999 — backend · frontend · fmt+격리 · 코드 리뷰 `/review code full` PASS — 2 family · 리뷰 뒤 변경분 확인 PASS). 결정과 그 출처의 정본 = ADR-0277(ADR-0180 · ADR-0276 을 연결 띠에 한해 개정) · 지금 사양 = §6-5 ③ 가드 항목(화면 모양) · §8 — 여기 되풀지 않는다.

**검증** — 로컬 게이트 PASS · 0 실패(수치 = `docs/process/step-log.md` 의 연결 띠 덮기 항목 · Rust 는 바뀌지 않았다). `/qa full` PASS(스크래치 데이터 폴더 · `9318b5c` · GUI J1–J5 · 연결 띠는 창마다 그 창의 `window.__ENGRAM_AGENT__` 로 합성 — `disconnect()` 뒤 `reportConnectionError` · 조리법 = `/qa` 바인딩 「실측 조리법」 B). 잰 것(J 번호별 짝은 이 기록에 남기지 않았다):

- main — 층이 TabBar 를 덮고 레이아웃 칸의 사각형은 그대로다.
- main — 연결 띠와 상태 파일 안내가 한 층에 쌓이고, 다크 테마에서 줄 사이 1px 구분이 보인다.
- 팝아웃 — 층이 덮고 사각형은 그대로다.
- 트리 — 층이 머리줄 아래를 덮고 목록의 사각형은 그대로다. ★트리 창은 숨긴 채 쟀다★ — 보이게 할 명령이 없고 웹뷰의 `window.show` 는 권한이 없다.
- 줄 밖을 누른 것은 그 아래로 간다.

**검증 안 된 것** — `33vh` 를 넘는 스크롤(GUI — vitest 의 구조 시험뿐) · 라이트 · e-ink 테마의 1px 구분 · 실제 기동 실패로 연결 띠 띄우기(J1–J5 는 전부 합성) · 트리 창을 보이게 한 상태 · 비 Windows 전부.

**열린 것(결정 아님)**
- 연결 사건 중 ADR-0180 결정 2 · ADR-0181 이 띠 쪽으로 가른 나머지(일시적 끊김 · 재시도 중 · 유실 알림 · 명령 하나 실패)도 이 덮는 층에 넣을지 — 사용자에게 물을 것. ADR-0277 의 「연결 띠」는 오늘의 `ConnectionNotice` 하나다(ADR-0277 「영향 / 불변식」). ★→ 저장 작업 밖으로 미뤘다(사용자 결정 2026-10-06 — 「그건 현재 주제하고 관련없는 구현 아님? 별도의 챕터를 둬야되지 않음?」 · 「저장 시스템 다 하고 언급하면 될듯」 · §14-18 「미룬 것」 · `docs/tracking.md` T-48)★.
- 「줄 바깥 빈 영역」(§14-16 그대로 — 사용자가 다음으로 미뤘다). ★→ §14-18 「미룬 것」 · `docs/tracking.md` T-50★.

### 14-18. P3c3 복원된 슬롯 표시 — 착지 기록 (2026-10-06)

「P3c3」 = ✅ 착지 `e904161`(프론트만 · 15 파일 · CI 초록(run 37429747731) · 코드 리뷰 `/review code full` — 2 family · 5 라운드 · 아래). 결정과 그 출처의 정본 = ADR-0280(ADR-0149 부분 개정 — 다섯 항목: (A) 의 스폰 대기 흡수 · (C) 의 문구 · 「근거」 마지막 항목의 판단과 「삭제도 흐림」 기각 사유(둘은 기억 없는 예약 슬롯에 한해) · 알려진 한계의 표시 — 목록 정본 = ADR-0280 관련줄) · 지금 사양 = §6-8 · §6-2 — 여기 되풀지 않는다.

**리뷰 경과**(`/implement standard` → `/review code full` — Claude doc-aware · codex 블라인드):

- R1 FIX — 트리 · 슬롯의 중복 활성화 · 「대상 없음」이 「비우기」를 잃었다.
- R2 FIX — 새로고침 전에 활성화 문이 풀린다 · 트리의 거절 표식이 남는다.
- R3 FIX — 슬롯의 실패가 트리에 안 보인다 · 활성화 도중 메뉴가 켜진다 · 오류 지우기.
- **사용자 범위 변경(D1 — 아래)** — 슬롯 활성화 단추와 그것을 위한 트리 · 슬롯 공용 활성화 문을 걷었다.
- R4 codex PASS · Claude FIX — 포커스 링이 막 아래에 깔린다(→ D4) · 주석 정확성.
- R5 codex PASS · Claude FIX — 주석만 · 링의 반투명 잔여(→ 사용자 수용 — D4) → 주석 수정 반영.
- 코더 재작업 라운드가 상한 3 을 넘었다 — 범위 변경과 함께 사용자가 진행을 승인했다.

**사용자 결정(2026-10-06)** — 정본 = ADR-0280:

- **D1** — 꺼진 에이전트를 가리키는 슬롯(프로필 있음 · 실행 중 아님 · 기억 없음)은 연결 끊김 · 죽음과 같은 부재 막. 슬롯 활성화 단추를 두지 않는다(이유 = ADR-0280 결정 1). ADR-0149 의 「빈 화면 흐림은 오독된다」 사유를 이 경우에 한해 뒤집고, 지운 에이전트(「대상 없음」)는 문구로 남는다.
- **D2** — 「대상 없음」은 원래 에이전트 슬롯 메뉴 그대로(「비우기」 포함). 거부 = 빈 슬롯 메뉴로 바꾸기(중간 구현 — 「비우기」가 사라졌다).
- **D3** — 모르는 내용 슬롯에 「비우기」를 두지 않는다 — 새 내용으로 덮어쓰거나 닫으면 된다(사용자). 거부 = 「비우기」 추가.
- **D4** — 포커스 링 색이 슬롯 상태로 바뀌지 않게 링을 막 위로. 반투명 잔여는 수용(이상해 보이면 그때 요청 — 사용자). 거부(지금은) = 불투명한 고정 색 링.
- **D5** — 아래 「미룬 것」 셋은 저장 작업 밖이다 — 저장 시스템을 마친 뒤 사용자에게 올린다(사용자: 「저장 시스템 다 하고 언급하면 될듯」).
- **D6**(`8f1a00a` 뒤 보강 · 커밋 `2080a4d`) — 모르는 내용 슬롯의 자리표시는 아이콘 하나 — 판 구분 · 안내 줄 · 막 없음 · 이름 「알 수 없는 내용」(사용자: 「구지 새버전이라고 구분하지 않았으면 좋겠는데. 이게 되게 크리티컬한것도 아니고 그냥 쿨하게 없애버려도 되는 정보잖아」 · 전원 꺼짐 같은 아이콘을 요청). 거부 = 판 구분 문구 · 안내 한 줄(`e904161` 의 모습). 아래 열린 것을 닫는다 · 정본 = ADR-0280 결정 7.

**검증** — `npx tsc --noEmit` 0 · `npm test` 93 파일 2022 통과 · Rust 는 바뀌지 않았다(Rust 빌드 · 회귀 = push 의 CI — 초록(run 37429747731)). `/qa full` PASS(`src/` 가 바뀌어 standard 에서 올렸다 · 스크래치 데이터 폴더 · cdp):

- 꺼진 에이전트를 가리키는 슬롯에 막(잎의 직계 자식 · 전원 꺼짐 아이콘 · 단추 0 · 글 없음) → 트리의 예약 행을 더블클릭해 활성화 → 117 ms 에 평소 화면으로 붙었다.
- 새로고침 타임라인 — 「연결 중」은 약 300 ms 뒤 판정으로 넘어갔고, 목록이 온 뒤에는 나오지 않았다.
- 「대상 없음」 메뉴 = 에이전트 모니터링 · 에이전트 종료 · 가로 분할 · 세로 분할 · 팝업으로 분리 · 비우기 · 닫기.
- 포커스 링 픽셀이 잎의 막 · 유지된 죽은 슬롯 · 산 터미널에서 같다((36,69,108) · 다크 테마).
- 모르는 내용 슬롯(`state.json` 의 슬롯 내용을 `{"type":"future_kind"}` 로 고쳐 기동 · 그 슬롯에 포커스) → 자리표시(`e904161` 의 문구 자리표시 — D6 이 아이콘으로 바꿨다)가 서고, 트리 「열기」는 그 칸 대신 빈 슬롯에 놓았으며, 원문은 `state.json` 에 그대로 남았다.
- D6 보강 — 로컬 `npx tsc --noEmit` 0 · `npm test` 93 파일 2022 통과 · 아이콘 GUI(스크래치 데이터 폴더 · 다크) = 아이콘 가운데(40×40 · 전원 꺼짐 아이콘과 같은 색) · 보이는 글 없음 · 막 없음 · 우클릭 = 빈 슬롯 메뉴와 같음 · 트리 「열기」는 그 슬롯을 건너뛰고 원문 보존 — ★hover 툴팁은 hover 가 그 요소에 닿는 것까지만 확인 · 툴팁 창 자체는 캡처 못 해 미검★.

**검증 안 된 것** — 라이트 · e-ink 테마의 링 모습(잔여 수용 — D4) · 「연결 중」에서 「대상 없음」으로 넘어가는 순서는 한 번(7 ms)만 관측했다. 덤 관측: 슬롯 컨텍스트 메뉴가 Escape 로 닫히지 않는다(이 변경 전부터 · 고치지 않았다).

**열린 것(결정 아님)**
- ~~모르는 내용 슬롯의 안내 한 줄(`slot.foreignContentHint` — 세션 선택) 앞부분 「더 새 버전에서 저장한 내용입니다」가 ADR-0165 결정 5(상태 · 원인을 화면 문구로 늘어놓지 않는다)의 「원인 문구」에 드는지 — 사용자가 정하지 않았다(ADR-0280 결정 6 · 「영향」).~~ → **닫았다 — D6 · ADR-0280 결정 7(사용자 결정 2026-10-06).**

**미룬 것(결정 아님 — D5 · 저장 시스템을 마친 뒤 사용자에게 올린다)** — 정본 = `docs/tracking.md`:

- **T-48** — 연결 사건 중 ADR-0180 결정 2 · ADR-0181 이 띠 쪽으로 가른 나머지(일시적 끊김 · 재시도 중 · 유실 알림 · 명령 하나 실패)도 덮는 층에 넣을지(ADR-0277 의 열린 것 · §14-17) — 사용자: 「그건 현재 주제하고 관련없는 구현 아님? 별도의 챕터를 둬야되지 않음?」 → 별도 주제.
- **T-49** — 슬롯 쪽 활성화 조작과 완전한 이어받기(D1 — 별도 기능).
- **T-50** — 「줄 바깥 빈 영역」(ADR-0276 의 열린 것 · §14-16 — 사용자가 이미 미뤘다).

### 14-19. P3d 창별 테마 — 착지 기록 (2026-10-07)

「P3d」 = ✅ 착지 `c85b2ac`(셸 + 프론트 + 도움말 · 커밋 직전 작업 트리 기준 +1826 / −2172 줄 · 27 파일, 시험 포함). 새 ADR 은 없다 — 결정의 정본 = ADR-0265 결정 7(창별 테마 = 상태 · `window.setTheme`/`getTheme` · 쓸기 폐지) · 지금 사양 = §5-6 · §5-7 · §6-3 · §6-5 ⑧ ⑨ · §6-7 ③ ④ · §8 — 여기 되풀지 않는다. ADR-0166 · ADR-0167 의 도장 확장 = §11 P3d 메모.

**바뀐 것(요지 — 정본은 위 절과 코드)**

- `src-tauri/src/theme.rs` 신설 — 창 테마 배달의 한 자리(유효 값 · 계산 · 밀기 하나 · 배달 실패 타입 · 손잡이 `ThemeControl`).
- 버스 `window.setTheme {window, theme|null}` · `window.getTheme {window}` → `{theme, effective}` · `catalog_version` 13 → 14.
- 미는 자리 넷(§5-6) · 프론트로 가는 짐 = `{theme}` 하나(wire 이름 둘은 그대로).
- 걷은 것 — `ui.refresh`(선언 · 동사 · `ThemeOrigin` · `UiRefreshOk` · `LayoutPorts.ui_settings` · `UiSettingsRefresh`) · 부팅 쓸기(ADR-0167 결정 6 · 7 — `declared_window_labels` 포함) · `src-tauri/src/ui_settings.rs` 통째(그 파일 안에는 시험이 없었다 — ui_settings 를 재던 통합 시험(`tests/layout_commands.rs`)의 해당 조각을 함께 걷었다 · 옛 소스 = `git show de65cae:src-tauri/src/ui_settings.rs`).
- 도움말 — `prompts/engram-help.md` 의 settings 구획을 다시 썼고 옛 `## theme` 별칭 자리는 남겼다.

**사용자 결정**

- **F7(2026-10-06)** — P3d 에 화면의 테마 UI 를 두지 않는다 — 명령만(§10 F7 = (a)).
- **`window.setTheme` 배달 실패의 답(2026-10-07)** — 지목한 창이 테마를 못 받았을 때만 오류를 낸다. 문구는 그 핵심 실패를 자세히 싣고 나머지는 수로 접는다. 다른 창만 못 받았으면 성공 + warn 로그. 그 오류의 코드 · 살아 있지 않은 창 · 그 밖 오류의 모양은 구현 사양이다(§5-6 — 사용자 결정으로 적지 않는다).

**검증** — 로컬(2026-10-07 · 커밋 직전 코더 실측) 셸 단위 · 통합 타깃 · `engram` 바이너리 단위 · vitest 전부 통과(수치는 여기 베끼지 않는다 — 정본 = CLAUDE.md 「빌드·검증 명령」의 `lib_unit` 줄 · 그날의 실측 기록 = `docs/process/step-log.md` 의 P3d 항목). 워크스페이스 총계는 로컬에서 다시 재지 않았다(CI 로 갈음). `/review code full` 2 라운드 PASS · `/qa full` GUI 9/9 PASS(창별 변경 · 기본값 따름 · 재시작 유지 · none 해제 · 오류 · 남은 파일 무시) · CI 초록(`c85b2ac` · run 37576408596).

**열린 것(결정 아님)**

- ADR-0166 결정 6(알림이 못 나가면 성공으로 답하지 않는다)을 `settings.set` · `settings.reset` 의 `theme.default` 에도 걸지 — 지금 그 길은 배달이 실패해도 성공으로 답한다(버스 · Tauri 둘 다). → **닫힘(사용자 결정 2026-10-08): 걸지 않는다 — 성공 답 유지**(§5-6).

### 14-20. P3e 뺌 — 슬롯 보기 모드는 영속하지 않는다 (사용자 결정 2026-10-07)

「P3e」(슬롯 보기 모드 덮어쓰기 `renderModeOverride` — `terminal` · `rich` · `dom` — 를 셸 소유 슬롯 값으로 옮겨 `state.json` 에 싣기) = **S21-storage 에서 뺐다.** 코드 변경은 없다 — P3e 는 착수 전이었다.

**사용자 결정**

- **P3e 뺌(2026-10-07)** — 사유(사용자): 「json · 터미널에서 값을 주고 있는데 아직은 필요없어 보임」. 기본 보기는 매번 에이전트 출력 형식에서 정해지므로(JSON → 챗 · 그 밖 → 터미널) 영속할 것은 손으로 건 덮어쓰기뿐이고, 그것은 아직 필요 없다.
- 2026-10-02 저장 범위 결정 중 「슬롯 보기 모드를 남긴다」 한 줄을 뒤집는다(§13 · §6-0 표).
- **F19** — 같은 날 앞서 (a)(셋 그대로)로 답했으나 이 결정으로 무효가 됐다(§10).

**결과**

- 덮어쓰기는 지금처럼 **프론트 메모리뿐**이다 — 창(웹뷰)마다 따로 · 재시작하면 사라진다. ADR-0056(프론트 전용 덮어쓰기) · ADR-0035 의 프론트 전용 예외 목록은 그대로 서고, §11 4 의 보기 모드 개정 도장 후보는 필요 없다.
- **L3 = P3a–P3d.** P3d 착지(`c85b2ac` · §14-19)로 L3 가 끝났고 지금 master 로 머지한다(§9-1).
- 되살릴 자리 = `docs/tracking.md` T-51 — 설계 메모는 §6-0 「슬롯 보기 모드」 블록(①–⑦ — 수행 안 함)에 남겨 두었다.

**고친 자리** — §0 저장 범위 · 순서 · §1 안 · 밖 · §6-0 표 · 블록 · §6-1 · §6-2 · §8 상태 코덱 · §9-1 · §9-2 P3e · GUI 확인 · §10 F19 · §11 4 · §13.

### 14-21. P4 웹뷰 폴더 · 정적 창 — 착지 기록 (2026-10-08)

「P4」 = ✅ 착지 `11f8787`(셸만 — +824 / −83 줄 · 7 파일, 시험 포함). 결정 = **ADR-0283**(ADR-0054 · ADR-0137 부분 개정) · 지금 사양 = §4 · §6-5 ③ ⑥ ⑦ ⑧ ⑨ · §6-7 ⑤ — 여기 되풀지 않는다. P4 착지로 L4 가 끝났다(L1–L4 가 한 릴리스 — §9-1).

**바뀐 것(요지 — 정본은 위 절과 코드)**

- `src-tauri/src/webview_env.rs` 신설 — 모든 창(main · agent-tree · 팝아웃)이 지나는 마무리 `WebviewEnv::finish`: 폴더(정했을 때) · 브라우저 인자(정본 = Rust 상수 — `tauri.conf.json` 의 `additionalBrowserArgs` 는 걷었다) · 확장 끔 · 스크롤바 모양 기본. 폴더 판정은 프로세스당 한 번(`OnceLock`)이고 못 쓰면 폴더를 넘기지 않는다(M4).
- `tauri.conf.json` 두 창 `"create": false` — 정적 창은 setup ⑧(실행 표식 뒤)에서 `from_config` 로 만들고 저장된 자리 · 크기 · 최대화를 만들 때 입힌다(`state/placement.rs` — `StaticWindow` · `static_windows` · `open_boot_windows` · `build_static` · `confirm_created`). 이미 있는 창에 자리를 입히던 `place_saved` 는 걷었다.
- `--hidden` 부팅 → main 을 처음부터 숨긴 채 만든다 · 숨긴 채 만드는 창은 `focused(false)` · 옛 「보였다 숨는」 main 의 길과 그 한계 주석(`lib.rs`)을 걷었다.
- main 생성 실패 → 종료 코드 1(`lib.rs` — 시작 실패 표지 · `RunEvent::Exit` 에서 `std::process::exit`) · 트리 창 실패 → 계속 · 조용한 실패 = `confirm_created`(§4 「정적 창을 못 만들면」).
- 가드 시험 둘 — 설정 창이 환경 옵션을 기본값으로 둔다(`webview_env.rs`) · 설정 창은 모두 `create:false` 이고 부팅이 그 창을 모두 만든다(`placement.rs`).
- 주석 — `lib.rs` 의 ADR-0102 블록(정적 창도 setup 안에서 생긴다 · 빌더 manage 는 그대로 — ADR-0102 관련줄) · `state/boot_plugin.rs`(⑥ 의 근거 · 답이 ⑦ 전에 오는 길은 방어) · `tauri.dev.conf.json`(identifier 가 웹뷰 폴더를 정하던 서술 — ADR-0137 개정).

**사용자 결정**

- **M4 (c)(2026-10-07)** — 웹뷰 폴더를 못 쓰면 대화상자 없이 Tauri 기본 자리로 물러나 진행하고 warn 로그를 남긴다(같은 날 「대화상자 후 종료」로 답했다가 바꿨다 · 근거 = §6 D8 「멈추지 않고 진행」). 공유 폴더 문서화와 ADR-0134 결정 3 개정은 뺀다 — 그 결정의 공유 폴더는 데몬끼리의 배제 문제다(§10 M4).
- **정적 창 생성 실패(2026-10-07)** — main 을 못 만들면 앱을 끝낸다 · 트리 창은 그 창 없이 계속한다(§4). 「트레이만으로 계속」을 버린 사유 = 사용자 「이상한 것들 에러 나면 강제 종료가 맞지」(2026-10-07 · ADR-0283 거부한 대안).

**리뷰 · 검증**

- `/review code full` PASS — 2인 · 재수정 3회.
- `/qa full` PASS(2026-10-07 · 마지막 수정 라운드 전 트리) — 워크스페이스 결과 줄 61 · 4243 통과 · 0 실패 · 31 무시 · vitest 2022 · GUI 11항목 + 혼합 DPI. 마지막 수정 라운드는 셸 파일만 바꿨고 그 뒤 셸 타깃 다섯을 다시 돌렸다(2026-10-08) — `lib_unit` 787 · `layout_apply` 66 · `layout_commands` 90 · `daemon_client_pending` 4 · `daemon_client_replay` 11(수치의 정본 = CLAUDE.md 「빌드·검증 명령」 · 그날의 기록 = `docs/process/step-log.md` 의 P4 항목).
- 재실측 A–F PASS — 런타임을 가린 실패(`WEBVIEW2_BROWSER_EXECUTABLE_FOLDER`) → 종료 코드 1 · `clean_exit:true` · 곧바로 다시 띄워도 정상 · 포커스(보이는 부팅의 전경 = main — 고치기 전에는 숨은 트리 창이 3/3 가져갔다) · `--hidden` · M4 warn · 회귀(종료 코드 0).

**알려진 한계(기록 — 고치지 않음)**

1. 트리 창만 조용히 실패하면 그 죽은 등록이 Tauri 의 창 표에 남고, 런타임 복원 창 포트의 보임 판정(`TauriRestoreWindows::visibility` — 보임을 못 읽으면 「보인다」)이 그 창을 보인다고 답한다. 리뷰어 판단이 갈렸고 수용했다 — 실제로는 닿기 어렵고 트리 창은 ADR-0225 가 걷는다.
2. 저장된 최대화 main 의 보이는 부팅은 약 0.5–0.8 초 보통 크기로 보인 뒤 최대화된다 — L3 와 같다(§6-5 ⑧).
3. 런타임 복원한 팝아웃의 포커스 순서는 그대로다 · `--hidden` 부팅의 팝아웃은 여전히 잠깐 떴다 숨는다(F13 — §10).
4. 네트워크 공유 위 사용자 데이터 폴더를 WebView2 가 받는지는 확인하지 않았다(§12 R1).

**ADR-0225(트리 전용 창 걷기)와 겹침** — P4 는 트리 창 몫 코드(와 그 가드 시험)를 더했고 ADR-0225 를 구현할 때 함께 걷는다. 목록의 정본 = ADR-0225 「함께 바꿀 곳」 하나 — 여기 베끼지 않는다. 한계 1 도 그때 사라진다.

**고친 자리** — 머리 · §0 웹뷰 폴더 · 순서 · §4 · §6-5 사실 ③ · ⑥ · ⑦ · ⑧ · ⑨ · §6-7 ⑤ · §8 Tauri 결합부 · §9-2 P4 행 · GUI 확인 · §10 F13 · M4 · §11 머리 · 2 · §12 R1.
