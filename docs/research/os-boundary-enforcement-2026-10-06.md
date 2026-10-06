# OS 의존 코드 경계를 기계로 지키는 관행 — Rust 피어 서베이 (2026-10-06)

- **상태:** 조사 완료 · 적대 리뷰(codex BLOCK) 반영 · 결정 완료(사용자 2026-10-06 — 결과 = `crates/engram-dashboard-platform/src/lib.rs` 헤더 게이트 ④~⑥ · 「하지 않는 것」) · 정정 2건(2026-10-06 — 결론 3 의 `std::os` 금지 관행 · §4 의 `cfg_if!`)
- **방법:** `/research` medium · 설계-결정 모드. 수집 세 갈래(직접 피어 = 소스 읽기 워커 · 의존 단위 · lint/교차 타깃 = 문서형 워커) → 메인 grounding → codex 적대 리뷰(BLOCK — 8건) → 본문 정정.
- **확신도 범례:** 확실 = 1차 자료 직접 대조 + 독립 확증 · 가능성 높음 = 1차 자료 대조(단일) 또는 코드 읽기 추론 · 불확실 = 요약 · 기억 기반 · 모름 = 확인 못 함.
- **맥락:** engram 1-3 U8 이 「platform 밖 OS `cfg` 파일 명단 고정」 게이트(ci.yml `platform gate 4` — PCRE2 문법 정규식)를 세웠고, 적대 리뷰가 4라운드째 드문 꼴의 누락을 짚어 루프 상한에 닿았다. 원칙 = CLAUDE.md 「플랫폼 중립」 · ADR-0230 · ADR-0266.

## 결론 (먼저)

1. **「`cfg` 가 어디 있나」를 CI 로 재는 성숙 프로젝트로 찾은 것은 rustc 하나뿐이고, 그것도 텍스트 스캐너다**(찾은 범위 안에서 — 피어 검색은 전수가 아니다 · 아래 「쟁점 · 한계」). rustc tidy `pal` 은 `library/` 의 `.rs` 에서 `cfg(` · `cfg!(` 를 문자열로 찾아 괄호를 맞추고, 그 텍스트에 OS 낱말이 **부분 문자열로** 들어 있으면 잡으며, 경로 부분 문자열 예외 목록으로 허용한다(가능성 높음 — 소스 대조). `cfg_attr` 와 앞선 doc 주석 뒤의 `cfg` 를 놓치는 것을 알고도(코드 읽기 — 가능성 높음) 필수 게이트로 돈다. → **우리 게이트 ④ 는 비교한 항목에서 그보다 넓게 잡는다**(`cfg_attr` · 여러 줄 · 깊은 중첩 · 문자열 속 괄호 — U8 결함 주입 44건 실측). 「업계 최고 선례도 알려진 빈틈을 두고 돈다」는 것이 드문 꼴 추적을 멈춰도 되는 근거다.
2. **나머지 성숙 프로젝트는 위치 검사 대신** ① 물리적 분리(std `sys/` · mio `src/sys/` · zed 의 OS별 crate 를 타깃 조건 의존으로) ② 다른 타깃으로 `cargo check`(mio · tokio) ③ 리뷰 · 관례를 쓴다(가능성 높음). ★**②는 위치를 강제하지 않는다**★ — 거짓 갈래의 `cfg` 는 그 타깃 컴파일에서 아예 빠지므로, platform 밖의 Windows 전용 분기는 Windows 검사도 Linux 검사도 통과한다(Rust Reference — conditional compilation). ②가 잡는 것은 **조건 없이 쓴 OS 전용 코드**와 다른 OS 에서 안 서는 코드다.
3. **API 경로 · crate 를 막을 땐 해석기 기반 도구를 쓴다**: clippy `disallowed-methods`/`disallowed-types`(deno 가 crate 마다 정해진 금지 항목을 갖췄는지 메타 검사로 강제) · cargo-deny `[bans]` `wrappers`. clippy 는 `#[cfg]` 속성의 **위치**를 막지 못한다(가능성 높음 — 그런 lint 를 찾지 못했고 반증 없음).
   - ★**정정(2026-10-06): `std::os` 금지는 관행이 아니다**★ — 조사한 피어 중 이 도구로 `std::os` 경로를 막는 곳은 **0** 이다. deno 가 막는 것은 파일 시스템 함수 · `std::env::var` 이고 `std::os` 가 아니다. 위 결론이 `std::os` 금지 관행처럼 읽혔고, 메인도 처음에 그렇게 과장해 전했다. 아래 적합도 표의 clippy · Dylint 줄의 `std::os` 도 「할 수 있다」는 뜻이지 피어 선례가 아니다.
4. **그래서 층이 셋이다 — 서로 대신하지 않는다:** 위치 = 게이트 ④(+ 리뷰) · 의존 = OS API crate 를 platform 만 끌어오게(그래프 검사) · 이식성 = 다른 OS 타깃 `cargo check`(조건 없이 쓴 `use std::os::windows` 같은 것 — CI 가 Windows 뿐이라 지금은 통과한다). 뒤의 둘이 지금 없다(★갱신 2026-10-06: 의존 층은 platform 헤더 게이트 ⑤ 로 세웠다 — 지금 없는 것은 이식성 층(교차 타깃 check) 하나다★).

## 갈래별 발견

### 1. 직접 피어 — 경계를 CI 로 강제하나

| 프로젝트 | 강제 | 수단 | 범위 · 허용 | 알려진 빈틈 | 확신도 |
|---|---|---|---|---|---|
| rust-lang/rust | ○ (`library/` 만) | tidy `pal` — 문자열 스캔 + 괄호 맞추기 · OS 낱말 부분 문자열 | 경로 부분 문자열 예외 목록(`library/std/src/sys` 등 · 「FIXME: sys 로 옮길 것」 임시 예외) · 경로에 `tests`/`benches` 면 건너뜀 · `cfg` 텍스트에 `test` 가 있으면 건너뜀 · 파서가 실제로 돌았는지 자체 단언 | `cfg_attr` · 같은 줄 앞선 doc 주석 · 매크로 갈래(`cfg_select!` 류) · `std::os::*` 직접 사용 · 부분 문자열 오탐 | 가능성 높음(소스 대조) · 빈틈은 코드 읽기 |
| deno | ○ (시스템 접근) | crate 마다 `clippy.toml` 의 `disallowed-methods` 가 **정해진 함수 경로 항목**(파일 시스템 함수들 · `std::env::var` 등)을 갖췄는지 텍스트 메타 검사(`tools/lint.js` 의 `ensureDisallowedMethodsEnforced` — main `3d44d1d82fda` 기준) | `ext/*` · `libs/*` · `runtime` | 코드 안 `#[allow]` 는 감사하지 않음 · `cfg` 위치는 대상 아님 | 가능성 높음(메타 검사 소스 대조) |
| mio · tokio | ✕ 위치 / ○ 다중 타깃 컴파일 | mio `Makefile` 7개 타깃 `cargo hack check --feature-powerset` · tokio `cross-check`(`cargo check --workspace --target …`) | OS 코드는 `src/sys/{unix,windows,…}` 관례 | 위치 검사 없음 | 확실(mio 메인 대조 + 리뷰어 tokio 대조) |
| zed (gpui) | △ 구조만 | OS별 crate(`gpui_windows` 등)를 `[target.'cfg(…)'.dependencies]` 로만 끌어옴 | — | 다른 crate 에 `cfg(target_os)` 가 퍼져 있음 | 불확실(미대조) |
| rust-analyzer · servo · wezterm · alacritty · helix · nushell · uv · ruff | ✕ | 관례 · 리뷰(alacritty 는 macOS 타깃 교차 빌드) | — | — | 불확실(수집자 보고 · 미대조) |

- 공개 clippy · dylint lint 중 `cfg` 의 **위치**를 제한하는 것은 찾지 못했다(모름 — 코드 검색 속도 제한 · 전수 아님). 개인 저장소 두 곳의 Dylint 사전 확장 lint 사례가 보고됐으나 같은 작성자 계열이라 일반화하지 않는다.
- 리뷰어가 bevy · fuchsia · firefox · chromium 을 따로 찾아봤으나 pal 류 검사는 확인되지 않았다(반증 아님 — 미확인).

### 2. 의존 단위 — OS API crate 를 한 crate 에만

- **cargo-deny `[bans]` `deny = [{ crate = "…", wrappers = ["…"] }]`** — 명단의 crate 만 금지 crate 를 **직접** 의존할 수 있고 그 밖의 경로는 거부된다(확실 — 문서 원문 · 리뷰어 재대조). 검사 그래프 전체에 걸린다 — tokio · mio 같은 서드파티가 `windows-sys` 를 직접 쓰면 그들도 명단에 올라야 하고 의존성 업데이트마다 흔들린다(가능성 높음 — 문서 의미에서 도출). OS crate 제한에 쓰는 실사례는 못 찾았다(모름).
- **그래프 스크립트(`cargo tree` / `cargo metadata`)** — ★`cargo tree -i` 맨 명령은 **호스트 타깃만** 보고 노드를 워크스페이스 멤버로 거르지 않는다★(Cargo 문서 · 리뷰어 지적). 그래서 `--target all` 과 멤버 거르기를 스크립트가 해야 한다 — engram 의 기존 의존 상한 게이트가 `--target all` + 이름 접두 거르기를 쓴다(빈틈 = 접두 식별, CLAUDE.md 에 적힌 구멍). `cargo metadata` 는 멤버 id · 의존 종류 · 타깃 조건을 구조화로 주므로 출력 파싱보다 견고하다(가능성 높음 — 문서). ★**규칙의 뜻이 `wrappers` 와 다르다**★ — 멤버만 보는 규칙은 「워크스페이스 안에서 platform 만」이고, `wrappers` 는 「그래프 전체에서 명단만」이다. 우리 목적(우리 코드의 경계)에는 앞의 것이 맞다.
- Cargo 자체에는 의존 가시성 기능이 없다(불확실 — 기억 기반 · 반증 못 찾음). Bazel/Buck `visibility` 는 빌드 시스템이 강제하는 대조군이다.

### 3. API · 빌드 단위 — clippy · 교차 타깃

- **clippy `disallowed-methods` / `disallowed-types`** — 경로로 지정 · 루트 `clippy.toml` 하나 + 허용 crate 의 `#![allow(…)]` 가 현실적 배치(설정 상속 없음 — 가능성 높음). `disallowed-types` 는 **단일 항목 `use` 선언도 잡는다**(clippy 소스 — 리뷰어 지적). `disallowed-methods` 는 해석된 호출을 보며, **트레이트 메서드 경로를 주면 구현 전반의 호출이 걸린다**(특정 구현 타입으로 좁힐 수는 없음 — clippy 관리자 답변). Windows 호스트에서 `std::os::unix::…` 경로는 해석이 안 돼 `allow-invalid = true` 가 필요하다(가능성 높음). **못 하는 것:** `#[cfg]` 속성의 위치(속성은 매크로가 아님 — 가능성 높음) · `std::env::consts::OS` 같은 상수(불확실). CI 에서 `cargo clippy --all-targets -- -D warnings` 로 돌려야 효력이 있다.
- **교차 타깃 `cargo check`** — 범위를 명시해야 한다: 맨 명령은 기본 멤버 · lib/bin 타깃 · 기본 기능만 본다(Cargo 문서). 쓸 꼴 = `cargo check --workspace --exclude <셸> --all-targets --all-features --target x86_64-unknown-linux-gnu`(그리고 `aarch64-apple-darwin`). `check` 는 그 패키지의 최종 코드 생성 · 링크를 하지 않지만 **build script 는 호스트 실행 파일로 컴파일해 실행한다** — C 코드를 빌드하거나 네이티브 라이브러리를 찾는 build script 가 의존 그래프에 있으면 실패할 수 있다. 그러니 「`rustup target add` 만으로 충분」은 **그런 build script 가 없는 그래프에서만** 참이다(우리 워크스페이스 실측 전 — 셸 패키지는 Linux 에서 webkit 계열을 요구하므로 처음부터 뺀다). 비용 = 타깃별 의존 재컴파일(모름 — 미측정).
- **런타임 OS 분기**(`"USERPROFILE"` · `.exe` 문자열 · 대소문자 접기)를 막는 전용 lint 는 없다(가능성 높음). 관례는 `disallowed-methods` 로 `std::env::var*` 를 막고 env 모듈 하나에서만 허용(starship · cargo PR 사례 — 불확실, 미대조).

### 4. 텍스트 스캐너가 따로 다뤄야 하는 꼴 (리뷰어 보충)

- **`cfg_select!`**(Rust Reference 수록) — 술어를 `cfg(…)` 철자 없이 쓴다. **`cfg_aliases`**(build script 로 `#[cfg(linux)]` 같은 별칭을 만든다) · `cfg_if!` 도 같은 부류다(★정정 2026-10-06: `cfg_if!` 는 아니다 — 안에 `#[cfg(…)]` 철자를 그대로 써서 게이트 ④ 가 문다(실측). 못 무는 것은 `cfg_select!` · `cfg_aliases` 쪽이다★). engram 에는 셋 다 0건이다(실측 2026-10-06 — `rg "cfg_if!|cfg_select!|cfg_aliases"` 0). 그중 `cfg_select!` · `cfg_aliases` 는 들어오면 게이트 ④ 가 못 본다 — 헤더 한계에 적어 둘 거리(적었다 — platform 헤더 게이트 ④ 「한계」).
- **`unexpected_cfgs`**(rustc check-cfg) 는 `cfg` 이름 · 값을 검사할 뿐 위치를 강제하지 않는다.

## engram 제약 적합도

제약(정본 = CLAUDE.md · ADR): 코어 격리 · 플랫폼 중립(macOS 이식이 전체 리팩토링이 되지 않게 · 회귀망을 한 OS 에 묶지 않음 — ADR-0230) · 게이트는 CI 에서 돌고 로컬 `/qa` 바인딩이 같은 것을 돌린다 · CI 러너는 Windows 뿐 · clippy · cargo-deny 는 지금 CI 에 없다(실측 — ci.yml 0건).

| 수단 | 층 | 무엇을 막나 | 정밀도 | 도입 비용 | 적합 |
|---|---|---|---|---|---|
| 지금 게이트 ④(PCRE2 문법 정규식 · 명단 33) | 위치 | 새 파일의 OS `cfg` | 비교 항목에서 rustc tidy 이상 · 드문 꼴 몇 개 · `cfg_select!`/별칭은 못 봄 | 0(완성) | ○ |
| 그래프 검사(`cargo tree --target all` 또는 `cargo metadata`)로 OS API crate(`windows` · `windows-sys` · `libc` · `nix`) 운영 의존 = 멤버 중 platform 만 | 의존 | 다른 멤버가 OS crate 를 끌어오는 것 | 해석된 그래프 · 빈틈 = 멤버 식별 방식 | 낮음(기존 게이트 꼴) | ○ |
| 교차 타깃 `cargo check`(Linux · macOS 타깃 · 셸 제외 · `--all-targets --all-features`) | 이식성 | 조건 없이 쓴 OS 전용 코드 · 다른 OS 에서 안 서는 코드 | 컴파일러 | 중간(build script 실측 · CI 시간) | ○ (별도 단위 · 실측 먼저) |
| clippy `disallowed-*` | API | OS 전용 표준 API 호출 · 단일 항목 `use` · `std::env::var`(`std::os` 금지에 쓴 피어 0 — 정정 2026-10-06) | 해석기 기반 | 중간(CI 에 clippy 신설 · 기존 경고 정리) | △ |
| cargo-deny `wrappers` | 의존 | 그래프 전체의 직접 의존자 | 그래프 · 서드파티까지 | 중간(도구 · 서드파티 명단 유지) | ✕ |
| Dylint 커스텀 lint | 위치 · API | `cfg` 위치 · `std::os` 경로 | 컴파일러 수준 | 큼(nightly 고정 · `rustc_private` · 타깃별 실행) | ✕ |
| 정규식 게이트의 드문 꼴 추적 지속 | 위치 | 드문 `cfg` 꼴 | 수렴 안 함(리뷰 4라운드 실측) | 라운드당 코더 수십만 토큰 | ✕ |

## 거부 후보 (ADR 거부 대안 후보)

- **Dylint 커스텀 lint** — nightly 고정 · 드라이버 버전 고정 · 타깃별 실행. 지키려는 경계에 비해 유지비가 크다.
- **cargo-deny `wrappers`** — 규칙의 뜻이 「그래프 전체」라 서드파티 명단 유지가 따라온다. 우리 목적(멤버 경계)은 그래프 스크립트가 소음 없이 맞춘다.
- **정규식 게이트의 드문 꼴 추적 지속** — 찾은 최고 선례(rustc tidy)도 알려진 빈틈을 두고 돈다. 남은 큰 구멍은 위치 축이 아니라 이식성 축(단일 OS CI)이다.

## grounding (메인 외부 대조 + 리뷰어 재대조)

| 클레임 | 출처 | 판정 |
|---|---|---|
| tidy `pal` = `EXCEPTION_PATHS` 경로 부분 문자열 · `library/std/src/sys` · FIXME 임시 예외 | `rust-lang/rust` `src/tools/tidy/src/pal.rs` L38 · L57 · L60 · L86 | 지지(메인 + 리뷰어) |
| `tests`/`benches` 경로 건너뜀 · `cfg` 에 `test` 포함 시 건너뜀 · 자체 단언 | 같은 파일 L92 · L163 · L99-100 | 지지 |
| OS 낱말 부분 문자열 판정 | 같은 파일 L135-141 | 지지 |
| `cfg_attr` 을 놓친다 · 앞선 doc 주석 뒤를 건너뛴다 | 같은 파일 `parse_cfgs`(L171-) | 부분지지(코드 읽기 — 수집자 · 리뷰어 일치 · 미재현) |
| cargo-deny `wrappers` 의미 | `EmbarkStudios/cargo-deny` `docs/src/checks/bans/cfg.md` L90-96 | 지지(메인 + 리뷰어) |
| deno 메타 검사 | `denoland/deno` `tools/lint.js` `ensureDisallowedMethodsEnforced`(main `3d44d1d82fda`) | 지지 |
| mio 7개 타깃 `cargo hack check` · tokio `cross-check` | `tokio-rs/mio` `Makefile` L2 · L27-29 · tokio `ci.yml` | 지지(mio 메인 · tokio 리뷰어) |
| 교차 타깃 check 는 위치를 강제하지 못한다 | Rust Reference — conditional compilation | 지지(리뷰어 · 논리) |
| `disallowed-types` 가 `use` 를 잡는다 · 트레이트 메서드 경로가 구현 전반의 호출을 잡는다 | clippy 소스 · 관리자 답변(rust-clippy discussions #16401) | 부분지지(리뷰어 출처 · 메인 미대조) |
| clippy 는 `#[cfg]` 위치를 못 막는다 | 수집자 문서 요약 | 부분지지(반증 없음) |

## 적대 리뷰 (codex · BLOCK → 반영)

| 지적 | 결함 | 반영 |
|---|---|---|
| 「rustc 만」·「우리가 더 촘촘」 단정 | 근거 없음 · 과장 | 「찾은 범위 안에서」로 좁히고, 비교 항목(U8 결함 주입)을 명시 |
| 교차 타깃 check 를 위치 경계의 답으로 제시 | 논리 공백(치명) | 결론 2 · 4 를 「층 셋 — 서로 대신하지 않는다」로 고침 |
| `cargo check` 범위 미명시 | 도구 의미 | `--workspace --exclude <셸> --all-targets --all-features` 꼴 명시 |
| 「`rustup target add` 만으로 충분」 | 과장 | build script 조건부로 좁힘 |
| `cargo tree -i` 가 멤버만 본다 · `wrappers` 와 같은 일 | 논리 · 도구 의미 | 호스트 기본 · 거르기는 스크립트 몫 · 뜻이 다름 · `cargo metadata` 대안 |
| clippy `use` 못 잡음 · 트레이트 매칭 미확인 | 도구 의미 | `disallowed-types` 단일 `use` · 트레이트 경로 매칭으로 정정 |
| deno 서술 · 인용 위치 | 인용 정밀도 | 함수 이름 + 커밋 고정 · 「정해진 항목 대조」로 정정 |
| `cfg_select!` · `cfg_aliases` · `cargo metadata` · `unexpected_cfgs` 누락 | 완전성 | §4 신설 · 저장소 0건 실측 |

## 쟁점 · 한계

- 피어 조사는 전수가 아니다(GitHub 코드 검색 속도 제한) — 「찾은 것은 rustc 하나」 이상을 말하지 않는다.
- clippy 매칭 세부와 우리 워크스페이스의 교차 타깃 `cargo check` 가능 여부(build script)는 **실측 전**이다.
- 이 보고서 재리뷰는 돌리지 않았다(medium = 단일 패스). 정정은 리뷰어 지적을 그대로 반영했다.
