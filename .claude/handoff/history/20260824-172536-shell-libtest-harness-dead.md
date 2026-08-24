# 핸드오프: 셸 단위 테스트 171개가 어디서도 안 돈다 — 원인 규명 완료, 수리는 미착수

## ★먼저 읽을 것★

1. **오늘 커밋한 문서 정리 건은 끝났다**(`accb5cb`, CI 초록). 워킹트리 클린. 이어서 정리할 잔여 없다.
2. **새로 판 것 = 셸 패키지(`src-tauri`)의 lib 테스트 타깃이 죽어서 단위 테스트 171개가 로컬·CI 어디서도 안 돈다.** 원인은 규명됐고 증명까지 됐다. **고치는 건 아직 아무것도 안 했다.**
3. **★죽은 길 셋이 실측으로 확인됐다 — 아래 「실패한 접근」을 먼저 읽어라.★** 안 읽으면 같은 삽질을 반복한다.
4. 31개 실패는 **31개 문제가 아니라 사실상 둘**이다. 규모 추정이 아래 있다.

---

## 한 줄 상태 · 다음 첫 액션

**셸 lib 테스트 타깃이 `STATUS_ENTRYPOINT_NOT_FOUND`로 죽는 원인을 끝까지 규명했다.** 매니페스트 문제이고, 실제 실행 파일에 매니페스트를 심어 **217개가 도는 것까지 확인**했다.

**다음 첫 액션 = `DaemonClient::connect()`를 읽는 것.** 실패 31개 중 30개 + 멈춤 2개가 그 함수 하나에 걸려 있다.

---

## repo 상태

- 브랜치 `v0.3.0/chore/misc`(master에서 오늘 새로 팠다), HEAD `accb5cb`, **클린**, 푸시됨
- **CI 초록 확인** — `accb5cb` 검증 3잡 전부 success(결론을 직접 읽어 확인, 종료코드 아님)
- master = `975d100` (변동 없음)
- 오늘 커밋 1건뿐: `accb5cb` 문서·주석 드리프트 정리 4건

---

## 【본론】셸 lib 테스트 타깃이 죽는 문제

### 증상

```
cargo test -p engram-dashboard --lib
  → 컴파일 44초 깨끗이 성공
  → Running unittests src\lib.rs (target\debug\deps\engram_dashboard_lib-<hash>.exe)
  → exit code: 0xc0000139, STATUS_ENTRYPOINT_NOT_FOUND
```

**2026-08-24 재현 확인.** CLAUDE.md·CI·qa 바인딩이 전부 이 패키지를 `--exclude`하는 근거가 이것이다.

### ★원인(증명됨)★

1. 그 실행 파일은 `comctl32.dll!TaskDialogIndirect`를 **정적 import**한다.
2. 로더가 물어오는 것은 `C:\Windows\System32\comctl32.dll` **v5.82**(155 exports) — **그 함수가 없다.**
3. 그 함수를 가진 v6는 WinSxS에 있고, **`RT_MANIFEST` 리소스(`Microsoft.Windows.Common-Controls 6.0.0.0`)가 있어야만** 연결된다.
4. `tauri-build`가 그 매니페스트를 **실제로 만든다**(`target/debug/build/engram-dashboard-*/out/resource.rc`).
5. ★**그런데 `embed-resource`가 그것을 `cargo:rustc-link-arg-bins=`로 넘긴다**★(`embed-resource-3.0.9/src/lib.rs:444`) — Cargo가 **`[[bin]]` 타깃에만** 적용하는 지시어다. lib 테스트 타깃은 못 받는다.
6. 그 실행 파일에는 `.rsrc` 섹션이 **아예 없다**(objdump 확인).

**결정적 증명:** 죽는 실행 파일을 복사해 `mt.exe -manifest app.manifest -outputresource:patched.exe;#1` 로 매니페스트만 심고 실행 → **정상 기동, 217개 테스트 실행.** 다른 바이트는 동일.

### 왜 통합 테스트 넷은 멀쩡한가

lib 테스트는 이 crate를 **링크 루트**로 컴파일한다 → `run()`과 `AppHandle<Wry>`를 받는 커맨드들이 전부 들어가고 Wry/muda/rfd 사슬이 딸려온다 → 그 import가 생긴다.

통합 테스트는 `engram_dashboard_lib`를 **rlib 아카이브**로 소비한다 → 필요한 심볼만 뽑아 쓴다 → `run()`을 아무도 안 부르므로 GUI 사슬이 링크에서 빠진다.

**실측:** 통합 실행 파일 넷은 import DLL이 12개이고 `comctl32`·`user32`·`ole32`가 **아예 없다**. lib 테스트는 21개. **매니페스트가 있어서 사는 게 아니라 필요가 없어서 사는 것이다.**

### ★실패한 접근 (전부 실측 — 다시 시도하지 말 것)★

| 방법 | 결과 |
|---|---|
| `build.rs`에 `cargo:rustc-link-arg={OUT_DIR}/resource.lib`(접미사 없는 형태) | **깨진다.** lib 테스트엔 닿지만 bin에도 닿아 이미 받은 것과 겹친다 → `CVT1100: duplicate resource type:VERSION` → `LNK1123`. `cargo build --bin` 실패 |
| `cargo:rustc-link-arg-tests=`(build.rs 주석이 제안하는 것) | **무의미.** 실측: 가짜 `.lib`로 확인했더니 `--test it`은 `LNK1181`로 죽고 `--lib`는 멀쩡히 링크됐다. Cargo의 `-tests`는 **`tests/` 폴더 타깃**을 뜻하지 lib 단위 테스트가 아니다 — 즉 **이미 멀쩡한 것들만 겨냥한다.** `embed_resource::compile_for_everything`도 같은 이유로 무의미(`-bins/-tests/-examples/-benches`를 내지 lib 단위 테스트는 없다) |
| `.cargo/config.toml`의 rustflags로 링크 인자 주입 | 모든 타깃에 걸려 위와 같은 중복 충돌 + 워크스페이스 전체로 샌다 |
| `#[cfg(not(test))]`를 `run()`에 걸어 Wry를 빼기 | **불충분할 가능성 높음.** `tauri::AppHandle`(=`AppHandle<Wry>`)이 17개 파일 20군데 이상 커맨드 시그니처에 있다 — `run()` 하나가 뿌리가 아니다 |

### 살아 있는 길 셋

| # | 방법 | 대가 |
|---|---|---|
| **A** | **테스트를 `tests/`로 옮긴다**(build.rs 주석이 부르는 「정공법」) | 빌드 해킹 0. 통합 타깃은 이 문제가 없음이 증명됐다. **대가 = 비공개 항목 접근** — `commands::popout::tests::*`·`tray::core::tests::*`·`daemon_client::tests::*` 등이 내부를 만져서 17개 이상 파일에 공개 범위 조정 필요. **「어디까지 열 것인가」가 실제 결정이다** |
| **B** | `build.rs`에 환경변수 스위치(`ENGRAM_LIBTEST`)로 링크 인자 주입 | **실측 동작 확인됨**(`cargo test --lib`은 bin을 안 지어서 충돌 없음). 대가 = 비표준 명령줄, `cargo test -p engram-dashboard`(전 타깃)와 동시에 쓰면 여전히 충돌, 변수 토글마다 재빌드 |
| **C** | 빌드 후 `mt.exe`로 매니페스트 심는 래퍼 스크립트 | **실 산출물로 증명됨.** 대가 = 게이트가 cargo 명령이 아니라 스크립트가 됨, Windows SDK `mt.exe` 의존(GitHub windows 러너엔 있다) |

### ★뚫어도 초록이 아니다 — 실측★

제자리 cwd로 실행한 결과:

```
217 total / 184 passed / 31 failed / 2 hung  (1.01초 — 타이밍 요소 없음)
```

**단 31개가 31개 문제가 아니다. 사실상 둘이다.**

**묶음 A+B (30개 + 멈춤 2개) — 전부 `daemon_client::tests`, 한 원인:**
- 29개가 동일 단언 실패: `connect().await` 직후 `state()`가 `Connected`여야 하는데 `Connecting`
- 1개는 정반대: 침묵하는 서버 상대로 `Timeout` 실패를 기대하는데 `Ok` 반환(`handshake_times_out_when_server_silent`)
- 멈춤 2개(`auth_sends_compiled_protocol_version_not_echo`·`close_in_flight_stays_down_no_revival`)는 mock 서버가 Auth 프레임을 **영영 못 받아** oneshot에서 무한 대기(타임아웃 없음). 90초 기다렸다 죽였다. 격리 실행에서도 재현
- **가설(미검증):** `connect()`가 핸드셰이크를 끝내지 않은 채 — 어쩌면 시작도 않은 채 — `Ok`를 돌려준다. 그러면 위 셋이 전부 설명된다. ★**조사 워커가 `connect()` 본문을 읽지 않았다** — 지시로 막았다★

**묶음 C (1개) — 무관한 모듈:**
- `layout::manager::tests::prepare_detached_view_empty_slot_is_err` — 빈 슬롯 분리가 거절돼야 하는데 `Ok`가 나온다(ADR-0064 인용). `b30f230`(08-17, 빈 슬롯을 + 아이콘으로)이 의도적으로 바꾼 결과일 수 있고, **진짜 결함일 수도 있다**(빈 팝업 창이 생긴다). 판단하려면 `prepare_detached_view`를 ADR-0064와 대조해 읽어야 한다

### 이게 제품 결함인가 — 미결

**아마 아니다(신뢰도 중상)** — 8월 18~23일 GUI 실측을 거친 커밋들이 클라이언트가 도는 걸 확인했다.

★**단 통합 테스트가 반증이 못 된다**★ — CLAUDE.md가 명시한다: **「소켓으로 나가는 것은 이 타깃이 재는 범위가 아니다」**. 통합 테스트는 핸드셰이크를 아예 안 건드린다. **그 구멍을 메우라고 있는 게 지금 죽어 있는 이 31개다.** 그러니 `connect()`를 읽기 전엔 제품 결함 가능성을 배제할 수 없다.

### 왜 썩었나 (날짜 근거)

- `src-tauri/src/daemon_client/tests.rs` — 마지막 실질 커밋 **2026-08-05**(`818cc0c`)
- `src-tauri/src/daemon_client/connection.rs` — **그 뒤로 7커밋**(8/18·18·19·19·20·21·23), S20 커맨드 버스 작업 포함
- 즉 **구현은 S20을 통과하며 계속 움직였고 테스트는 멈춰 있었으며, 그동안 한 번도 실행될 수 없었다.** 현재 형태로 단 한 번도 안 돌았을 가능성이 있다

---

## ★부수 발견 — 작은 일 6번(바인딩 게이트)에 직결★

테스트가 실제로 돌자 ts-rs가 TS 8개를 다시 구웠다. 그 결과:

1. ★**생성 위치가 cwd 상대다**★ — 워크스페이스 루트에서 돌리니 `<repo루트>/bindings/`에 떨어졌다. 정상 `cargo test`(cwd=`src-tauri/`)라면 `src-tauri/bindings/`로 간다. **게이트를 세울 때 이 경로 의존을 반드시 고정해야 한다**
2. ★**커밋본과 다르다 — 단 타입 모양은 전부 동일하고 주석만 다르다**★
   - 예: `ViewMeta`가 생성본은 `window:tabs-updated`, 커밋본은 `view:list-updated`
   - `LayoutNode`는 생성본에 doc 주석이 아예 없고 커밋본엔 있다
   - **결론: wire 계약 드리프트는 없다. 그러나 게이트를 켜면 즉시 빨간불이 난다** — 앞서 예측한 그대로다
3. **증거 보존 위치:** 재생성본 8개를 `<스크래치>/regenerated-bindings/`로 옮겨 뒀다(repo 트리는 클린). 스크래치 경로 = `C:\Users\kimsunzun\AppData\Local\Temp\claude\I--Engram-apps-engram-dashboard-wt2\9146389c-db55-4ffc-87bc-65069f83d813\scratchpad\`
   - ★**세션 스크래치라 다음 세션엔 없을 수 있다.**★ 없으면 위 A/B/C 중 하나로 다시 구우면 된다

**낡은 기록:** `src-tauri/build.rs:1-7`의 `KNOWN-ISSUE` 주석이 원인은 정확히 짚었으나, `rustc-link-arg-tests` 회피책을 거부한 사유(**"이 패키지엔 테스트 타깃이 없다"**)가 **지금은 거짓**이다(`tests/`에 4개 있다). 다만 그 회피책은 **다른 이유로** 여전히 무의미하다(위 표).

---

## 검증 상태

### 돌린 것

- `cargo test -p engram-dashboard --lib` → **재현 확인**(0xc0000139)
- PE import 대조(`llvm-objdump -p`, `C:\Program Files\LLVM\bin\`) — rustup `llvm-tools`는 **미설치**, VS `dumpbin`은 있으나 불필요했다
- `mt.exe` 매니페스트 주입 → **217개 기동 성공**
- 제자리 cwd 실행 → **184/31/2**. 스크래치 실행과 **숫자·이름 전부 동일** → 경로 아티팩트 0
- 링크 인자 3형태 실측(위 표)
- `accb5cb` CI 초록(결론 직접 읽음)

### ★검증 안 된 것★

- **`connect()` 본문을 아무도 안 읽었다** — 31개의 원인 가설은 미검증이다
- **묶음 C(빈 슬롯 분리)가 제품 결함인지 미판정**
- **A/B/C 수정안 중 무엇도 적용·검증 안 했다**
- **바인딩 주석 차이의 방향을 안 밝혔다** — 커밋본이 손으로 고쳐진 건지, Rust 주석이 바뀐 건지

---

## 정지 조건

- **`v*` 태그 push** — 사용자 승인 필수(릴리스 발행, 재시도 불가)
- **master 머지** — 매번 확인
- **`connect()` 수정** — 제품 결함으로 판정되면 그건 별건이다. 테스트 하네스 복구와 섞지 말 것
- **CLAUDE.md 수정** — 아래 backlog 2건은 사용자 결정

---

## do-not (이번·이전 세션 누적)

1. ★**`v*` 태그를 승인 없이 push하지 말 것**★
2. ★**CI 판정을 종료코드로 하지 말 것**★ — `gh run watch --exit-status`가 취소에도 0을 준다. `gh run view <id> --json conclusion`으로 직접 읽는다. 감시 중 ref에 새 커밋을 밀면 그 감시는 무효
3. ★**`gh -q` 표현식에 `/`를 쓰지 말 것**★ — Git Bash가 경로로 바꿔 먹는다(`completed/success` → `completedC:/Program Files/Git/success`). 폴링 루프가 안 끝난다. 구분자는 `" "`나 `":"`를 쓰거나 `MSYS_NO_PATHCONV=1`
4. ★**문서를 고칠 땐 그 수정이 무효화하는 다른 기록을 먼저 찾을 것**★ — 이번에 **세 번 연속** 재현됐다(백로그 목록 → 그 목록의 설명 문장 → 문장이 가리키던 줄). 매번 리뷰어가 잡았고 코더가 스스로 잡은 적이 없다
5. ★**ADR 채번을 작업 끝으로 미루지 말 것**★ — 번호를 잡으면 그 자리서 스캐폴드를 커밋·푸시해 선점
6. ★**챗 스타일을 커맨드로 만들지 말 것**★ — ADR-0169가 거부
7. ★**CDP 포트 9223을 그냥 쓰지 말 것**★ — 다른 워크트리 앱이 문다. 9224로
8. ★**릴리스 exe가 커밋보다 낡았는지 대조할 것**★ — `exe mtime` ↔ `git log -1 --format=%cd`
9. ★**곁문 명단을 문서에 적지 말 것**★ — 찾는 법 = `rg "\)\.__ENGRAM_|\)\.__engram" src/ -g '!*.test.*'`
10. **`launch/실행.bat`을 개발 중에 쓰지 말 것** — 이미지 이름으로 모든 데몬을 죽인다

---

## 남은 일

### 지금 판 것 (이게 다음 주제다)

**셸 테스트 하네스 복구** — 위 A/B/C 중 택 → 제자리 실측 → `connect()` 읽고 31개 분류 → 수리 → CI 등록. **바인딩 게이트(아래 6번)가 여기 딸려 온다.**

### 오늘 정리한 작은 것 중 남은 것

- **5. ADR 0170 빈 번호** — 처리 방식 정함: **억지로 채우지 않고 다음 새 결정을 0170으로 쓴다**(채번은 최대+1이라 그때 번호를 지정해야 한다). 지금 할 일 없음
- **6. `src-tauri/bindings/` drift 게이트** — 위 하네스 복구에 딸려 온다. 켜면 즉시 빨간불(주석 차이). 경로가 cwd 의존인 것 주의
- **7. 슬롯 점유 경로 회귀 테스트 없음** — 가장 가까운 테스트가 *다른 창의 탭*으로 실패시켜 이 경로를 안 지킨다
- **8. CDP 포트 고정 → 빈 포트 + portfile** — 런처와 `cdp.mjs` 양쪽. 데몬이 이미 쓰는 패턴

### 두꺼운 것 (이전 핸드오프에서 이어짐)

1. **`agent.spawnInto` 미배치 프로세스** — 버그가 아니라 ADR-0059가 못 박은 설계다. 프로세스는 데몬 명부에 등록돼 있어 고아가 아니다. **진짜 결함은 그 id가 한국어 산문 안에만 있어 호출자가 데이터로 못 받는 것.** 셋 중 택 — 스폰 전 미리 검사(TOCTOU로 창을 못 닫음) / 실패 시 kill(ADR-0059 번복) / **오류에 id를 데이터로 싣기(추천)**. 마지막 건 공유 `command` crate 계약을 건드린다
2. **릴리스에 무가드로 열린 전역 핸들 둘** — `src/api/clientFactory.ts:29,31`. **가드가 정말 없다**(대조군: `main.tsx:33`의 `__engram`은 DEV 가드가 있다). ★**핸들을 지워도 구멍이 안 닫힌다**★ — `withGlobalTauri: true`(`tauri.conf.json:13`)라 `window.__TAURI__.core.invoke`로 셸 명령을 직통 호출할 수 있고 거기에 spawn·kill·stdin·데몬 기동이 똑같이 노출돼 있다. 게다가 `forward_daemon_command`(`src-tauri/src/commands/agent.rs:206`)가 raw wire를 데몬에 그대로 민다. **즉 이건 "핸들 철거"가 아니라 "릴리스에서 제어 표면을 어디까지 열 것인가" 정책 결정이다.** `/qa` 바인딩(`.claude/skill-bindings/qa.md:220-223`)이 프로필 생성·삭제에 이걸 의존한다 — 대체 경로가 선행
3. **클라이언트 자작 식별자 = 주인 모델** — **몸통 작업.** 지금 데몬은 주인을 연결 번호에서 파생(`command_roster.rs:126`, `conn-<번호>`)해 재접속마다 새 주인이 된다(등록이 덮기가 아니라 쌓기). 안정 식별자를 넣으면 **「연결 하나 = 주인 하나」 전제가 깨지고** 세 군데를 같이 설계해야 한다 — 봉투 배달(`sink_of:226`)·답장 경로(`sink_for_conn:246`)·끊길 때 지우는 단위(`detach:316`, 살아 있는 다른 연결의 등록까지 지우는데 그쪽은 모른다). 코드가 스스로 「이 슬라이스가 닫아야 한다」고 적어 뒀다. 범위 = net·daemon·셸·웹 클라이언트 + wire golden, 입구 검증 값 미정
4. **`mail.*` 선언 0건** — ADR-0155 결정 1이 「데몬 = `mail.*`」로 못 박았는데 미실현. green-field 아니다: CLI 어휘 상수(`core/src/agent/types.rs`)와 MCP 도구 둘(`daemon/src/control/mcp_server.rs:198+`)과 화해해야 한다. ★**보안 구멍이 딸려 온다**★ — 우편 게이트는 **주소**로 막는데(닫힌 열거형) 표는 **이름**으로 부른다. `mail.*`이 표에 드는 날 우편이 막힌 자격증명이 전체 이름으로 `mail.send`를 부를 수 있다(step-log `:1970`). 상충하는 해법 둘이 이미 적혀 있고 **어느 쪽인지는 사용자 결정**
5. **저장 시스템** — **레이아웃은 진짜로 디스크 영속이 없다**(확인됨). ★**단 「챗 스타일이 인메모리라 날아간다」는 거짓이다**★ — `localStorage['engram.chatStyle']`에 저장돼 살아남는다(`chatStyleStore.ts:63,84-90`). ADR-0169가 말한 건 **릴리스 빌드에 그 값을 읽고 바꿀 경로가 없다**는 것. 기반은 이미 둘 있고 관례가 갈렸다 — 코어의 프로필·프리셋 JSON(원자적+스키마 버전+corrupt 사이드카) vs 셸의 `ui-settings.json`(원자적이지만 버전·사이드카 일부러 없앰). **합칠지가 ADR-0167이 남긴 미결이고 저장 시스템의 첫 결정이다**

### backlog에 기록만 한 것 (사용자 결정 필요)

- **ADR-0058 본문이 죽은 전제를 안고 있다** — 「데몬은 셸을 띄운다」. 결정 자체는 유효하고 전제만 죽었다. 전제 사이트 `:1`(제목)·`:7`·`:10`·`:14`, 낡은 앵커 `:4`·`:17`·`:23`, 제목 사본이 `docs/decisions/README.md:108`에도. **`:17` 본문은 참이다 — 문장째 지우지 말 것**
- **CLAUDE.md 「프론트 구조」의 "그런 모듈은 지금 없다"를 코드가 반증한다** — `src/theme/uiSettings.ts:15`가 `invoke`를 직접 import하고 그 파일 헤더가 스스로 예외라고 적어 뒀다

---

## 이번 세션 한 일

**커밋 `accb5cb`** — 문서·주석 드리프트 4건:
- S20 Step 4 착지를 뒤늦게 기록(step-log `S20.17` 신규 + TRD §5·§6·§9 정정 표시). **번호는 늦고 자리는 시간순**
- 구독 조건 다이어그램에서 화신 표식 제거(ADR-0164) + 같은 문서의 용어 정의 둘(`:25`·`:705`)이 「카운터」라 적던 것 수정(ADR-0163 결정 2 위반)
- 「데몬이 띄우는 건 셸」 주석 **셋** 수정 — `commands/layout.rs`는 「claude 가 아니다」라고 **명시적으로 부정**하면서 정작 claude를 띄우는 코드를 인용하고 있었다. 거절 사유를 「wire에 고를 칸이 없다」로 바꿔 적었다(기본값이 또 바뀌어도 안 낡는다)
- `/qa` 바인딩의 챗 스타일 근거 ADR-0168 → 0169

**검증:** `/review doc light` 2라운드(FIX 6 → FIX 7 → 전량 반영 + 닫힘 grep 확인), CI 초록.

**적립한 피드백:** `/qa` feedback.md 2건 — Git Bash 경로 변환이 폴링 루프를 먹은 건, 그리고 문서 정정의 연쇄 드리프트 관측(3회 연속).

---

## 읽어야 할 파일

- **`src-tauri/src/daemon_client/connection.rs`의 `connect()`** — ★다음 첫 액션★. 31개 중 30개 + 멈춤 2개가 여기 걸려 있다
- **`src-tauri/build.rs:1-7`** — `KNOWN-ISSUE` 주석. 원인은 맞고 회피책 사유는 낡았다
- **`src-tauri/src/daemon_client/tests.rs:501,572`** — 멈추는 두 테스트의 oneshot 대기 지점(타임아웃 없음)
- **`src-tauri/src/layout/manager.rs`의 `prepare_detached_view`** — 묶음 C 판정용. ADR-0064와 대조
- **`.claude/skill-bindings/qa.md`** §full — GUI 실측 조리법(포트·신선도·teardown)
