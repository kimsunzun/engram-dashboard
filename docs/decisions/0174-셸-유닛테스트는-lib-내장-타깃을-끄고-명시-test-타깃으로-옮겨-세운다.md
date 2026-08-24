# ADR-0174: 셸 유닛테스트는 lib 내장 타깃을 끄고 명시 [[test]] 타깃으로 옮겨 세운다

- 상태: 확정 (2026-08-24, 근거: 실측 — `mt.exe` 매니페스트 주입으로 217건 기동 증명 + 링크 인자 4형태 실측)
- 관련: CLAUDE.md 「빌드·검증 명령」 · `src-tauri/Cargo.toml` `[lib]`/`[[test]]` · `src-tauri/build.rs` · ADR-0012(모듈 격리·단독 하네스)

## 맥락

셸 패키지(`src-tauri`)의 `src/**` 안 `#[cfg(test)]` 유닛테스트 217건이 **로컬에서도 CI에서도 한 번도 실행된 적이 없다.** lib 내장 테스트 타깃이 프로세스 로드 단계에서 `0xC0000139 STATUS_ENTRYPOINT_NOT_FOUND`로 즉사했기 때문이다.

원인은 매니페스트 리소스의 링크 경로다. 그 테스트 exe는 Wry/muda/rfd 사슬을 타고 `comctl32.dll!TaskDialogIndirect`를 정적 import하는데, 그 export는 comctl32 **v6**에만 있고 v6는 바이너리가 `RT_MANIFEST`(`Microsoft.Windows.Common-Controls 6.0.0.0`)를 품을 때만 side-by-side로 바인딩된다. `tauri_build`가 그 매니페스트를 `$OUT_DIR`에 만들기는 하는데, `embed-resource`가 그것을 `cargo:rustc-link-arg-bins=`로만 흘리고 cargo는 그 인자를 `[[bin]]` 타깃에만 적용한다. 매니페스트를 못 받은 exe는 System32의 v5.82를 물어 진입점을 못 찾고 `main` 전에 죽는다.

그 즉사가 **다른 결정 40여 곳의 근거로 인용돼 있었다** — 워크스페이스 회귀의 `--exclude engram-dashboard`, "이 패키지에 새로 쓰는 테스트는 `tests/`로 간다"는 배치 규칙, 알려진 결함의 수정 보류, `src-tauri/bindings/`를 "손으로 관리되는 생성물"로 부른 판정까지. 즉 이건 테스트 하나가 안 도는 문제가 아니라 **검증 지형 전체가 기대던 전제**였다.

## 결정

세 다리를 함께 세운다.

1. `src-tauri/Cargo.toml` — `[lib] test = false`(+ `doctest = false`)로 lib 내장 테스트 타깃을 끈다.
2. `src-tauri/Cargo.toml` — 같은 소스를 `[[test]] name = "lib_unit", path = "src/lib.rs"`로 **다시 선언**한다.
3. `src-tauri/build.rs` — `$OUT_DIR`에서 리소스 아카이브를 찾아 `cargo:rustc-link-arg-tests=`로 흘린다.

핵심은 2번이다. cargo의 `-tests` 링크 인자는 **kind=test 타깃**(= `[[test]]` 선언분)에 적용되는데 lib 내장 유닛테스트 타깃은 거기 들지 않는다. 같은 `src/lib.rs`를 명시 `[[test]]`로 한 번 더 선언하면 그 인자를 받는 타깃이 된다.

부르는 명령은 `cargo test -p engram-dashboard --test lib_unit` 하나다.

## 거부한 대안

- **접미사 없는 `cargo:rustc-link-arg=`** — 실측 탈락. lib 테스트엔 닿지만 bin에도 닿아 이미 `-bins`로 받은 것과 겹친다 → `CVT1100: duplicate resource type:VERSION` → `LNK1123`. `cargo build --bin`이 깨진다.
- **`cargo:rustc-link-arg-tests=` 단독**(옛 `build.rs` 주석이 제안하던 것) — 실측 탈락. 가짜 `.lib`로 확인했더니 `--test <이름>`은 `LNK1181`로 죽고 `--lib`은 멀쩡히 링크됐다 — 즉 그 인자는 **이미 멀쩡한 타깃만** 겨냥한다. `embed_resource::compile_for_everything`도 `-bins/-tests/-examples/-benches`를 낼 뿐 lib 유닛테스트 타깃은 없어 같은 이유로 무의미하다. **본 결정이 이 대안을 *되살린* 것이 아니다** — 인자는 그대로 두고 그 인자가 닿는 **타깃을 새로 만든** 것이 차이다.
- **`.cargo/config.toml` rustflags 주입** — 모든 타깃에 걸려 위와 같은 중복 충돌이 나고, 워크스페이스 전체로 샌다.
- **`#[cfg(not(test))]`로 `run()`에서 Wry 사슬 제거** — 코드 근거로 탈락. `tauri::AppHandle`(=`AppHandle<Wry>`)이 17개 파일 20군데 이상 커맨드 시그니처에 있어 `run()` 하나가 뿌리가 아니다.
- **테스트를 `src-tauri/tests/`로 이관**(옛 `build.rs` 주석이 「정공법」이라 부르던 것) — 코드 근거로 탈락. 통합 타깃은 이 문제가 없지만, 옮기려면 `commands::popout::tests`·`tray::core::tests`·`daemon_client::tests` 등이 만지는 비공개 항목 때문에 17개 이상 파일의 공개 범위를 열어야 한다. **테스트를 위해 제품 API 표면을 넓히는 대가**이고, 채택안은 소스 변경 0으로 같은 것을 얻는다.
- **`build.rs` 환경변수 스위치(`ENGRAM_LIBTEST`)** — 실측으론 동작했으나 비표준 명령줄이 되고, 패키지 전체 `cargo test`와 같이 쓰면 여전히 충돌하며, 변수를 토글할 때마다 재빌드가 돈다.
- **빌드 후 `mt.exe`로 매니페스트 주입하는 래퍼 스크립트** — 실 산출물로 증명됐으나(아래 「근거」) 게이트가 cargo 명령이 아니라 스크립트가 되고 Windows SDK 의존이 붙는다. **증명 도구로만 쓰고 해법으로는 쓰지 않았다.**

## 근거

- **원인 확정 = `mt.exe` 주입 실험.** 죽는 exe를 복사해 매니페스트만 심고(`mt.exe -manifest app.manifest -outputresource:patched.exe;#1`) 실행하니 **정상 기동해 217건이 돌았다.** 다른 바이트는 동일 — 매니페스트 부재가 유일 원인임이 이걸로 닫혔다.
- **왜 통합 타깃 넷은 멀쩡한가.** lib 테스트는 이 crate를 **링크 루트**로 컴파일해 `run()`과 `AppHandle<Wry>` 커맨드가 전부 들어가고 Wry 사슬이 딸려온다. 통합 타깃은 `engram_dashboard_lib`를 **rlib으로 소비**해 필요한 심볼만 뽑으므로 그 사슬이 링크에서 빠진다. 실측: 통합 exe 넷은 import DLL 12개이고 `comctl32`·`user32`·`ole32`가 아예 없다(lib 테스트는 21개). **매니페스트가 있어서 사는 게 아니라 필요가 없어서 사는 것이다.**
- **릴리스 무영향 실측.** `-tests` 인자는 kind=test 타깃에만 닿는다. 변경 후 `cargo build -p engram-dashboard`로 지은 exe가 `.rsrc`·VERSIONINFO·Common-Controls 6.0 매니페스트를 그대로 갖는 것을 확인했다. `Cargo.lock`도 불변이라 CI의 `--locked` 스텝 전부 무영향.
- **적대 리뷰 2라운드 · 4인**(Claude 2 + GPT 2, 교차 family). 1라운드 BLOCK → 2라운드 FIX. 두 family가 독립적으로 겹쳐 짚은 둘(`#[ignore]` 대신 대기 상한 · gnu 타깃의 `libresource.a`)은 그대로 반영했다.

## 영향 / 불변식

- ★**세 다리는 함께 서야 한다**★ — 하나만 빠지면 **조용히** 무너진다. `[[test]]`를 지우면 217건이 통째로 증발하고, `build.rs` 방출을 지우면 `0xC0000139`가 그대로 돌아온다. `[lib] test = false`를 지우면 죽는 타깃이 기본 선택으로 되돌아온다.
- **그 셋을 지키는 것이 CI 게이트다** — `cargo test --locked -p engram-dashboard --test lib_unit -- export_bindings_`(8건). 그 8건이 돈다는 사실 자체가 세 다리의 존재 증명이다(`[[test]]` 부재 → cargo가 "no test target named `lib_unit`", 링크 인자 부재 → exe가 뜨기도 전에 죽음). 같은 스텝이 `src-tauri/bindings/`를 생성물 동기 게이트에 처음으로 편입시킨다.
- ★**`--lib`·`--all-targets`는 여전히 죽는다**★ — `[lib] test = false`는 **기본 선택**에서만 빼므로 명시 호출은 매니페스트 없는 내장 타깃을 그대로 고른다. 매니페스트로 닫을 수 있는 구멍이 아니라 **호출 형태로 피한다**.
- **`src/lib.rs`가 두 번 컴파일된다** — cargo가 "present in multiple build targets" 경고를 상시 낸다. 미관 문제지만, 그걸 "정리"하려고 `[[test]]`를 지우면 위 회귀가 난다.
- **`build.rs`는 리소스 아카이브를 정확히 하나 못 찾으면 panic한다** — 0개면 `[[bin]]`도 매니페스트를 못 받았다는 뜻(= 릴리스 바이너리가 이미 깨져 있다), 2개 이상이면 stale+신규가 함께 링크돼 `CVT1100`으로 이 패키지 테스트 타깃 다섯이 전부 깨진다. **어느 쪽이든 조용히 성공하면 안 되는 빌드다.** msvc(`resource.lib`)와 gnu(`libresource.a`) 양쪽 이름을 받는다.
- **이 결정은 스위트를 초록으로 만들지 않는다** — 217건 중 32건이 실패하고 전부 `daemon_client::tests::`의 한 원인(`start_connection`의 `app: None` 단락)에서 나온다. **제품 결함이 아니다**(운영 경로는 항상 `app: Some`). 그래서 스위트 전체는 아직 CI에 등재하지 않았다. 그 하네스를 어떤 모양으로 고칠지는 **미결이고 사용자 결정**이다.
