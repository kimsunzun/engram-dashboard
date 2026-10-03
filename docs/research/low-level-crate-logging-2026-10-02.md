# 낮은 층 OS 래퍼 crate 의 로깅 — 공용 바닥 crate 를 끌지 않는 법

- **상태:** medium · 설계-결정 모드 · 2026-10-02 · 적대 리뷰 FIX 반영(아래 「적대 리뷰」) · 결정: 감싸지 않는다(사용자 2026-10-03) — ADR-0268 재작성 · platform 은 tracing 직접(ADR-0266)
- **방법:** 수집자 2(피어 코드 · 메커니즘/공식 지침) → 메인 grounding(load-bearing 5건 원문 대조) → cross-family 적대 리뷰 1회.
- **확신도 범례:** 확실(독립 교차확증) · 가능성 높음(단일 출처 grounding) · 불확실(미검증).

## 왜 조사했나

ADR-0266(OS 의존 코드 → `platform` crate, 워크스페이스 의존 0)과 ADR-0268(로그는 base 입구로만)이 부딪힌다. `platform` 으로 옮길 `RmSession` 의 `Drop` 이 실패를 경고 로그로 남긴다(`crates/engram-dashboard-agent/src/platform/file_holders.rs` 의 `impl Drop for RmSession` — 「돌려보낼 곳이 없으므로 남기는 것은 로그뿐」). 로그 하나 때문에 `platform` 이 base 전체를 의존하는 것은 말이 안 된다(사용자 2026-10-02). 그리고 base 로 보낼 원자적 쓰기(`src-tauri/src/fsutil.rs`)가 Windows 전용 재시도를 품어 base → platform 을 요구한다 — 둘이 함께면 순환이다.

## 발견

### 1. 피어의 주류 = 낮은 crate 가 로그 facade 를 직접 의존한다 — 공용 util crate 를 거치지 않는다 (확실)

- wezterm `portable-pty` → `log` 직접(`pty/Cargo.toml` — `log.workspace = true`, util 아닌 `filedescriptor` 만 의존) · rust-analyzer `stdx`(그 util crate 자체) → `tracing` 직접 · cargo `cargo-util` → `tracing` · zed `util` → `log` · ripgrep `grep-cli`/`grep-searcher` → `log` · alacritty `alacritty_terminal`(ConPTY 포함) → `log` · nushell `nu-system`/`nu-utils` → `log`. 출처 = 각 저장소 `Cargo.toml`(portable-pty · stdx 는 메인이 원문 대조).
- **facade 자체가 「작은 로깅 crate」 자리를 이미 채운다** — 피어 표본에서 util 에서 로깅만 따로 뺀 작은 crate 를 낮은 crate 가 쓰는 사례는 없었다. zed 의 `zlog`(로거 구현 + 범위 매크로)는 낮은 crate(`sum_tree` · `rope`)가 dev-dependency 로만 쓰고 운영 코드는 `log` 를 직접 부른다 — 단 더 높은 crate(`project` · `zlog_settings`)는 운영 의존으로 쓴다(적대 리뷰 정정 · 메인이 `crates/project/Cargo.toml` `[dependencies]` 로 확인). 가장 가까운 선례는 rustc 의 `rustc_log`(`tracing` 재수출 crate)이나 **검색 요약으로만 봤다(불확실)**.
- 변형: mio 는 `log` 를 기본 켜진 선택 기능으로 두고 내부 매크로가 꺼지면 아무것도 안 한다(`src/macros.rs`) · tokio 는 `tracing` 을 비기본 선택 의존으로 둔다.

### 2. 가장 낮은 핸들 층은 로그를 아예 안 찍는다 (확실)

std(`OwnedFd` · `OwnedHandle` · `File`) · wezterm `filedescriptor` · rust-analyzer `paths` · cargo `home` 은 로깅 의존이 없다. 오류는 `Result` 로 돌려주고 `Drop` 의 닫기 실패는 조용히 버린다. std `File` 문서: 「Errors detected on closing are ignored by the implementation of `Drop`. Use the method `sync_all` if these errors must be manually handled.」

### 3. `Drop` 실패 처리는 세 갈래뿐이고, 공식 지침이 둘을 권한다 (확실)

- Rust API Guidelines **C-DTOR-FAIL**(원문 대조): 「Instead of failing in a destructor, provide a separate method for checking for clean teardown, e.g. a `close` method, that returns a `Result` to signal problems. If that `close` method is not called, the `Drop` implementation should do the teardown and ignore or log/trace any errors it produces.」 — https://rust-lang.github.io/api-guidelines/dependability.html
- 피어 실물: (a) 조용히 버림 — std · wezterm `OwnedHandle`/`PseudoCon` · alacritty `Conpty` · rust-analyzer `JodChild` (b) 명시 `close() -> Result` + 최선 노력 `Drop` — tempfile `TempDir::close`(std `File` 은 닫기 메서드가 없고 `sync_all` 은 내용 동기화일 뿐이다 — 적대 리뷰 정정) (c) `Drop` 에서 facade 경고 — **더 높은 층에서만**: cargo `FileLock`(`src/util/flock.rs` — `tracing::warn!("failed to release lock: {e:?}")`, 메인 원문 대조).
- 콜백·훅으로 로그를 대신하는 낮은 crate 는 표본에서 0 이었다(부재 증명은 아니다).

### 4. 공식 facade 지침 = 라이브러리는 facade 만 링크한다 (확실)

`log`(`src/lib.rs`: 「Libraries should link only to the `log` crate…」)와 `tracing`(`tracing/src/lib.rs`: 「Libraries should link only to the `tracing` crate…」, 라이브러리는 `set_global_default` 를 부르지 않는다) 둘 다(메인 원문 대조). 무게: `tracing` 기본 기능은 proc-macro(`tracing-attributes` → syn 계열)를 끌고, `default-features = false` 면 빠진다 · `log` 는 사실상 무의존(정확한 수는 `cargo tree` 미확인 — 불확실).

### 4-1. 부르는 쪽 구독자가 `log` 사건도 받는다 (가능성 높음)

base 는 `tracing-subscriber` 기본 기능으로 `try_init()` 한다(`crates/engram-dashboard-base/src/logging/mod.rs:417` 부근 · `Cargo.toml` 은 `features = ["env-filter"]` 만 더한다). 그 기본 기능 `tracing-log` 이 켜져 있으면 `try_init()` 가 `LogTracer` 를 설치해 `log` facade 사건을 같은 구독자로 넘긴다(적대 리뷰 인용 — docs.rs `tracing-subscriber` features · `util.rs` · 메인 미대조). 즉 갈래 C 에서 `log` 를 써도 파일 로그에 닿는 길이 이미 있다.

### 5. 매크로로 감싸면 호출 자리 정보가 남는다 (가능성 높음)

`macro_rules!` 재수출·래퍼는 `module_path!()` · `file!()` · `line!()` 이 **호출 자리에서** 전개돼 호출자의 모듈 · 파일 · 줄이 남는다(근거 = `log` 의 `macros.rs` 가 `$crate` 경유 `module_path!()` 를 쓴다 · std `module_path!` 문서). 함수 래퍼는 래퍼 쪽 모듈이 찍힌다(`#[track_caller]` 는 파일 · 줄만 — 표준 의미론, 전용 출처 없음). 이것은 ADR-0268 결정 3(매크로 입구)의 근거이고, 「작은 로그 crate 가 매크로를 재수출」하는 갈래가 호출 자리 필터(`EnvFilter`)를 잃지 않는다는 뜻이다.

## 우리 제약과의 적합도

제약: ADR-0268(로그는 base 입구로만 — 「나중에 컨트롤이 안 된다」) · ADR-0266(`platform` 워크스페이스 의존 0) · ADR-0269(base 는 OS 를 모르는 범용 도우미) · 사용자 「로그 때문에 base 전체를 끄는 건 말이 안 된다」 · 리뷰 F3(원자적 쓰기의 Windows 재시도).

| 갈래 | 피어 선례 | ADR-0268 | ADR-0266 의존 0 | 원자적 쓰기 순환(F3) |
|---|---|---|---|---|
| **A. platform 은 로그를 안 찍는다** — 실패는 `Result`, 정리는 명시 `close()/end() -> Result` + 조용한 `Drop`(C-DTOR-FAIL). 로그는 부르는 쪽(agent 등)이 base 로 찍는다 | std · filedescriptor · paths · home(가장 낮은 층의 일반형) · 공식 지침 | 지킨다(platform 에 로그 없음) | 지킨다 | 풀린다 — platform 이 맨 아래, base → platform 허용(base 가 Windows 판정만 platform 에 묻는다) |
|  ↳ A 의 조건 | | | | 명시 `end()` 가 **성공 · 조기 오류 경로 모두**에서 불려야 한다 — `holders_of` 는 중간 `?` 반환이 여럿이라(`file_holders.rs:63` 부근) 조용한 `Drop` 만으로는 그 경로의 `RmEndSession` 실패가 안 보인다. Restart Manager 는 사용자 세션당 열린 세션 수가 제한된다(MS `RmStartSession` 문서 — 적대 리뷰 인용 · 메인 미대조, 가능성 높음) |
| **B. 작은 로그 전용 crate** — ADR-0268 의 입구 매크로를 base 에서 떼어 잎 crate 로, base · platform 이 그것만 의존 | rustc_log(불확실) · zed zlog 은 구현 crate 라 다르다 | **깬다(글자대로)** — ADR-0268 은 입구를 **base** 에 두고 `tracing` 직접 의존을 base 만 허용한다. 입구가 한 곳이라는 취지는 지키지만 ADR-0268 개정이 필요하다(적대 리뷰) | 깬다(로그 crate 1개 의존) — 단 그 crate 는 facade 하나뿐 | 풀린다 — 로그 crate 가 맨 아래, platform → 로그, base → platform/로그 |
| **C. platform 만 facade 직접 의존 예외** | 피어 주류(portable-pty · alacritty 등) | 예외 하나(게이트에 이름 박기) | **지킨다** — ADR-0266 은 워크스페이스 crate 의존 0 이고 게이트도 `engram-dashboard` 접두만 센다(적대 리뷰 정정) | 풀린다 — A 와 같은 방향 |
| (거부) platform → base | — | 지킨다 | 깬다 + base 전체를 끈다 | 순환(base → platform 과 함께 못 산다) |

## 거부 후보 → ADR 거부 대안

- **platform → base(로그 때문에)** — 사용자 거부(「base 전체를 포함시키는 게 말이 안 됨」) + F3 순환.
- **콜백·전역 훅으로 platform 로그를 앱이 받는다** — 거부 근거가 약하다: 피어 표본 0 은 부재 증명이 아니고 C-DTOR-FAIL 도 훅을 배제하지 않는다(적대 리뷰). 거부가 아니라 **공백**으로 둔다.

## 적대 리뷰

cross-family(codex · effort high · web_search) 1회 → **FIX**. 반영:
- 갈래 B 가 ADR-0268 을 「지킨다」는 판정은 틀렸다 — 글자대로는 깬다(개정 필요). 표 정정.
- 갈래 C 가 ADR-0266 을 「깬다」는 판정은 틀렸다 — 서드파티 facade 는 워크스페이스 의존 0 을 안 깬다. 표 정정.
- 갈래 A 는 조기 오류 경로까지 명시 `end()` 가 덮어야 실패가 보인다 — 조건 줄 추가.
- zed `zlog` 쓰임새 정정(높은 crate 는 운영 의존) · std `File::sync_all` 분류 정정 · `LogTracer` 경로 추가(§4-1) · 훅 거부를 공백으로 강등.
- 스팟 재검증 통과: API Guidelines 문구 · cargo `FileLock` `Drop` 경고 · `tracing` 매크로의 호출 자리 target.
- 리뷰어 판단: 제약을 글자대로 지키는 것은 **A**(조건 = 명시 `end()` 가 모든 경로를 덮고 정리 오류가 호출자에게 가는 길을 정한다).

## 한계 · 공백

- 피어는 표본이다(약 10 저장소 · `gh api` 원문 읽기). deno · rustup 은 못 봤다.
- 「util 에서 로그만 뗀 작은 crate」의 실사용 선례는 확인하지 못했다(rustc_log 는 검색 요약뿐).
- `log`/`tracing` 의 정확한 의존 수는 `cargo tree` 로 재지 않았다.
