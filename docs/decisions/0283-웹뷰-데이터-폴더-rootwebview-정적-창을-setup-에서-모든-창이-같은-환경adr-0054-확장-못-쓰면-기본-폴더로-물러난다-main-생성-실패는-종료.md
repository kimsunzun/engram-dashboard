# ADR-0283: 웹뷰 데이터 폴더 = `<root>\webview` · 정적 창을 setup 에서 · 모든 창이 같은 환경(ADR-0054 확장) · 못 쓰면 기본 폴더로 물러난다 · main 생성 실패는 종료

- 상태: 확정 (2026-10-08, 근거: TRD `docs/process/S21-storage/trd.md` §4 · §6-5 ⑧ · §10 M4 · §11-2 + 사용자 결정 2026-10-07 (M4 (c) — 못 쓰면 기본 자리로 물러나 진행 · 대화상자 없음 / main 을 못 만들면 앱을 끝낸다) + P4 구현 `11f8787` · `/review code full` PASS · `/qa full` PASS(GUI 실측 — TRD §14-21))
- 관련: ADR-0264(데이터 배치 — 그 결정이 `<root>\webview\` 를 자리로만 두고 옮기는 일을 이 결정에 넘겼다) · ADR-0134 결정 4(데이터 루트를 못 쓰면 명확히 실패 — 웹뷰 폴더의 물러남은 그 경로가 창에 사유를 띄울 수 있게 한다, 아래 「근거」) · ADR-0135 · ADR-0102(빌더 manage — ★그 맥락의 「웹뷰가 setup 전에 invoke 한다」는 정적 창에 더는 서지 않는다 · 결정은 그대로 옳다(그 ADR 관련줄)★) · ADR-0225(트리 전용 창 걷기 — 이 결정이 더한 트리 몫 코드도 그때 함께 걷는다, 아래 「영향」) · ADR-0271(`DataLayout` 을 데몬 몫 · 셸 몫으로 나눈다 — `webview_dir()` 의 자리가 옮겨 갈 수 있다) · TRD `docs/process/S21-storage/trd.md` §4 · §6-5 · §14-21 · `src-tauri/src/webview_env.rs` · `src-tauri/src/state/placement.rs`(`restore_windows` · `build_static` · `confirm_created`) · `src-tauri/src/lib.rs`(setup ⑧ · `RunEvent::Exit` 의 종료 코드) · `src-tauri/tauri.conf.json` · step-log S21 · Amends ADR-0054 (상수 자리와 config 사본과 3중 동기 지점, 불변식이 덮는 환경 옵션 범위) · Amends ADR-0137 (영향의 WebView2 데이터 폴더 대가 서술)

## 맥락

- **웹뷰 데이터 폴더가 데이터 루트 밖에 있었다.** 창의 WebView2 사용자 데이터 폴더는 Tauri 기본 자리(`%LOCALAPPDATA%\<identifier>`)였다. ADR-0264 가 `<root>\webview\` 를 자리로 두고 옮기는 일은 뒤따르는 결정에 넘겼다.
- **설정만으로는 그 폴더를 가리킬 수 없다(Tauri 2.11.3 소스 대조 — TRD §4).** 빌더의 `data_directory(PathBuf)` 는 절대 경로를 그대로 쓰지만 설정 키 `dataDirectory` 는 상대 경로만 받아 `LocalData/<label>/` 아래 붙인다(`webview/mod.rs:392-418`). 그래서 설정이 선언한 창(main · agent-tree)도 Rust 빌더를 거쳐야 한다.
- **같은 폴더의 창은 환경 옵션이 같아야 한다(ADR-0054).** 어긴 창은 `build()` 가 `Ok` 인데 OS 창이 없다(유령 창). 브라우저 인자는 설정(창 두 곳)과 Rust 상수(`popout.rs`) 세 곳에 따로 적혀 손으로 맞추고 있었다. 폴더를 옮기면 폴더 자체도 맞춰야 할 환경 옵션이 된다.
- **설정 창은 Tauri 가 사용자 setup 앞에서 만들었다**(`app.rs:2521-2531`). 그래서 저장된 main 자리는 창이 설정 자리로 뜬 뒤에 옮겨졌고, `--hidden` 부팅의 main 은 보였다가 숨었다(TRD §6-5 ⑧ · §4).
- 남은 물음 둘 — 웹뷰 폴더에 쓸 수 없을 때(TRD §10 M4) · 창을 못 만들었을 때.

## 결정

1. **웹뷰 데이터 폴더 = `<root>\webview`**(`DataLayout::webview_dir()`). 절대 경로로 바꾼 뒤 확인하고 같은 값을 넘긴다 — 데이터 루트는 `ENGRAM_DATA_DIR` 로 상대 경로일 수 있다.
2. **정적 창은 셸이 setup 에서 만든다.** `tauri.conf.json` 의 두 창은 선언만 두고 `"create": false` 라 Tauri 가 만들지 않는다. 부팅 setup ⑧(상태 부팅의 실행 표식 ⑤ 뒤 — TRD §6-5 I6)이 `WebviewWindowBuilder::from_config` 로 만들고 저장된 자리 · 크기 · 최대화를 만들 때 입힌다. `--hidden` 부팅이면 main 을 처음부터 숨긴 채 만든다. 숨긴 채 만드는 창은 포커스를 받지 않는다(`focused(false)`).
3. **모든 창이 같은 환경 — ADR-0054 확장.** 창을 만드는 모든 코드(정적 창 · 런타임 팝아웃)는 마무리 함수 하나 `WebviewEnv::finish`(`src-tauri/src/webview_env.rs`)를 지나고, 그 값은 셸에 하나인 인스턴스에서 나온다. 마무리가 붙이는 것 = 데이터 폴더(정했을 때) · 브라우저 인자 · 브라우저 확장 끔 · 스크롤바 모양 기본 — wry 가 WebView2 환경에 싣는 넷이다. 브라우저 인자의 정본은 Rust 상수 `WEBVIEW2_BROWSER_ARGS` 하나이고 설정은 그 값을 싣지 않는다(`additionalBrowserArgs` 를 걷었다).
4. **못 쓰면 기본 폴더로 물러난다(사용자 결정 2026-10-07 — M4 (c)).** 창을 만들기 전에 쓰기 확인을 **프로세스당 한 번** 하고 모든 창이 그 답을 받는다. 못 쓰면 데이터 폴더를 넘기지 않아 Tauri 기본 자리(`%LOCALAPPDATA%\<identifier>`)로 진행하고 warn 로그를 남긴다(사유만 — 경로는 로그 칸). 대화상자는 없다.
5. **main 을 못 만들면 앱을 끝낸다 · 트리 창은 계속(사용자 결정 2026-10-07).** main 실패 = error 로그 → 표지 → 앱 종료 요청 → `RunEvent::Exit` 가 화면 상태 `Final` 을 쓰고 셸 실행 잠금을 놓은 뒤 `std::process::exit(1)`. 트리 창 실패는 log 하고 그 창 없이 계속한다. `build()` 가 `Ok` 여도 런타임이 창을 못 만든 조용한 실패(WebView2 런타임 없음 등)는 만든 직후 게터 한 번으로 확인해 실패로 친다(`confirm_created`).

## 거부한 대안

- **설정 키 `dataDirectory`** — 상대 경로만 받아 `LocalData/<label>/` 아래 붙는다(Tauri 소스 — 위 「맥락」). `<root>\webview` 를 가리킬 수 없다. 기각 근거 = 코드.
- **Tauri 2.12 `appDirectoriesOverride`** — 업그레이드가 먼저다(2.11.3 에 없다 — TRD §11-1). 기각 근거 = 코드(지금 버전에 없음).
- **`%LOCALAPPDATA%` 유지** — 데이터 루트 밖이라 포터블 배치(ADR-0134)를 어기고, 같은 식별자의 워크트리끼리 localStorage 를 함께 쓴다(TRD §11-2).
- **옛 폴더(`%LOCALAPPDATA%\com.engram.dashboard[.dev]`) 자동 삭제** — 사용자 결정 U1(사용자가 본인뿐이라 옛 값을 다루지 않는다 — TRD §10 F4) · 같은 식별자의 다른 워크트리 · 옛 빌드가 쓰고 있을 수 있고 쓰는 중이면 반쯤만 지워진다(TRD §4).
- **못 쓰면 네이티브 대화상자 후 종료**(M4 (a) 의 앞 절반 — 같은 날 한 번 그렇게 답했다가 바꿨다) — 사용자 결정 2026-10-07: 저장 정책 「멈추지 않고 진행」(TRD §6 D8). 데이터 루트 전체를 못 쓰면 창이 뜬 뒤 ADR-0134 결정 4 · ADR-0135 의 기존 경로가 사유를 창에 띄운다 — 종료하면 그 창도 없다.
- **네트워크 공유 위 데이터 폴더를 당분간 미지원으로 문서화(ADR-0134 결정 3 개정)** — 사용자 결정 2026-10-07: ADR-0134 결정 3 의 공유 폴더는 데몬끼리의 배제 문제이고 웹뷰 폴더는 클라이언트 쪽이라 무관하다. ADR-0134 결정 3 은 개정하지 않는다.
- **main 을 못 만들어도 트레이만으로 계속** — 사용자 결정 2026-10-07(종료를 골랐다). 사용자 사유: 「이상한 것들 에러 나면 강제 종료가 맞지」. 계속 돌면 창 없이 단일 인스턴스 잠금과 `state.lock` 만 쥔 프로세스가 남아, 다시 실행해도 그 프로세스에 넘겨질 뿐 아무 일도 안 일어난다(유저 눈엔 멈춘 앱 — 리뷰 적출) · P4 전에 Tauri setup 패닉으로 끝난 것은 창 만들기가 `Err` 를 돌려준 경우뿐이었고, 런타임의 조용한 실패(`build()` 는 `Ok`)는 창 없이 계속 돌았다 — 바로 앞의 잠금만 쥔 프로세스가 그것이다.

## 근거

- **소스 대조(Tauri 2.11.3 · tao 0.35.3 · tauri-runtime-wry 2.11.3 · wry 0.55.1):** `data_directory` 는 절대 경로를 그대로 쓰고 지정이 없을 때만 `LocalData/<identifier>` 를 쓴다(`webview/webview_window.rs:1024` · `manager/webview.rs:534-545`) · `create` 플래그(`tauri-utils-2.9.3/src/config.rs:1936`) · `from_config`(`webview_window.rs:150`) · wry `create_environment` 가 환경에 싣는 옵션 넷(`webview_env.rs` 머리) · 종료 요청은 `ControlFlow::Exit`(tao = 코드 0)로 끝나 종료 코드 1 은 `RunEvent::Exit` 에서 직접 낸다(`lib.rs`).
- **조용한 실패 실측(QA 2026-10-07):** `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER` 로 런타임을 가리면 tauri-runtime-wry 는 창 만들기 실패(`WebviewRuntimeNotInstalled`)를 log 만 하고 `build()` 는 `Ok` 를 돌려준다. 확인을 넣은 뒤 = 종료 코드 1 · `clean_exit:true` · 곧바로 다시 띄워도 정상. 릴리스 빌드에서는 그보다 먼저 Tauri 자체의 오류 대화상자 「Could not find the WebView2 Runtime.」이 뜬다(우리 것이 아니다 — tauri-runtime-wry 2.11.3 `src/lib.rs:4757-4765` · `cfg(all(not(debug_assertions), windows))`) [소스 독해 — QA 는 debug 빌드만].
- **포커스 실측(QA 2026-10-07):** 고치기 전 보이는 부팅 3/3 에서 숨은 트리 창이 전경을 가져갔다 · `focused(false)` 뒤 main 이 전경이다. 원인이 wry 의 만들기 끝 `MoveFocus` 라는 것은 소스 독해다(`placement.rs` 의 `build_static` 주석 — [미검]).
- **게이트:** `/review code full`(2인 · 재수정 3회) PASS · `/qa full` PASS — 워크스페이스 · 셸 타깃 · vitest · GUI 11항목 + 혼합 DPI · 재실측 A–F(수치 = TRD §14-21 · step-log).
- **M4 (c) 의 대가(사용자 결정에 딸린 것 — TRD §10 M4):** 물러난 실행은 같은 식별자의 워크트리끼리 웹뷰 폴더를 함께 쓴다 — 지금 그 폴더에서 읽는 값은 없다(TRD §4 「잃는 것」).

## 영향 / 불변식

- **창을 만드는 코드는 전부 `WebviewEnv::finish` 를 지난다** — 건너뛴 창은 다른 창과 환경이 갈려 유령 창이 된다(ADR-0054). 팝아웃 빌더는 그 인스턴스가 manage 돼 있지 않으면 창을 만들지 않고 오류를 낸다(`commands/popout.rs` 의 `build_window`). `// ADR-0054` 앵커 = `webview_env.rs` 머리 · `placement.rs` 의 `build_static`.
- **설정에 환경 옵션을 다시 적지 말 것 · `"create": false` 를 걷지 말 것** — 시험이 지킨다: `webview_env.rs` 의 `config_windows_leave_every_environment_option_at_the_default` · `placement.rs` 의 `every_config_window_is_left_to_the_boot_and_the_boot_knows_them_all`. ADR-0054 가 「후속 하드닝」으로 남긴 설정 ↔ 상수 동기 문제는 설정이 그 값을 싣지 않게 되면서 사라졌다.
- **폴더 판정은 프로세스당 한 번이다** — 창마다 다시 물으면 한 프로세스의 창들이 다른 폴더를 받는다(`OnceLock` · `WebviewEnv` 에 `Clone` 을 달지 않는다 — 그 파일 주석).
- **정적 창 생성은 실행 표식(⑤) 뒤 setup ⑧ 이다** — 창을 만들다 죽어도 다음 부팅이 비정상으로 읽는다(TRD §6-5 I6). 이 순서 덕에 창이 처음 당기는 복원 상태는 부팅 단계 ⑥ 이 정한 값이다.
- **알려진 한계(고치지 않음 — TRD §14-21):** ① 트리 창만 조용히 실패하면 그 죽은 등록이 Tauri 의 창 표에 남아 보임 판정이 「보인다」로 답한다(리뷰어 판단이 갈렸다 · 실제로는 닿기 어렵고 트리 창은 ADR-0225 가 걷는다 — 수용) ② 저장된 최대화 main 의 보이는 부팅은 약 0.5–0.8 초 보통 크기로 보인 뒤 최대화된다(L3 와 같다) ③ 런타임 복원한 팝아웃의 포커스 순서는 그대로 · `--hidden` 부팅의 팝아웃은 여전히 잠깐 떴다 숨는다(F13) ④ 네트워크 공유 위 사용자 데이터 폴더를 WebView2 가 받는지는 확인하지 않았다.
- **ADR-0225 와 겹침:** 이 결정이 트리 창 몫 코드(와 그 가드 시험)를 더했고 ADR-0225 를 구현할 때 함께 걷는다. 목록의 정본 = ADR-0225 「함께 바꿀 곳」 하나 — 여기 베끼지 않는다.
