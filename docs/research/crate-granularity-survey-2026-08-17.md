# 조사 보고 — 소형 crate 분할이 옳은 방향인가 (Rust 워크스페이스)

| | |
|---|---|
| 상태 | 완료 (medium — 적대 리뷰 1회 반영) |
| 날짜 | 2026-08-17 |
| 방법 | 주계열 수집 4갈래(병렬) → 메인 grounding(클레임↔출처 함의) → cross-family 적대 리뷰 → 반증 3건 메인 재대조 |
| 리뷰 판정 | **MATERIALLY-FLAWED** — HIGH 지적 6건. 아래 내용은 그 지적을 **반영해 수정한 뒤**의 것이다 |
| 확신도 범례 | **확실**(grounding 지지 + 독립 확증) · **가능성 높음**(지지, 단일 출처) · **contested**(반증 존재) · **불확실**(보류) |

---

## 결론

**crate로 뺀 것 자체는 관행에 어긋나지 않는다. 크기는 판단축이 아니다.** 다만 우리가 근거로 삼았던 두 전제가 조사에서 무너졌다 — "방향 강제엔 crate밖에 없다"와 "두 번째 소비자가 있어야 한다". 둘 다 외부 근거가 없거나 반례가 있다.

**기준을 바꿔야 한다: 「크기」·「방향 강제」가 아니라 「독립적으로 쓸 수 있는가」.** 이것이 유일하게 발견된 권위 있는 산업 가이드의 기준이고, 우리 현행 기준보다 넓다.

---

## F1. 줄 수는 판단축이 아니다 — **확실**(부재 확인) / "1.5천 줄이 평범하다"는 **불확실**

- **줄 수 임계값을 문서화한 프로젝트는 하나도 없었다.** 모든 사례가 "역할이 독립적으로 의미 있는가"로 판단한다.
- ★**수정:** 최초 합성본은 "1.5천 줄은 성숙 워크스페이스에서 평범한 하위 범위"라고 결론했는데, **리뷰가 이를 정확히 때렸고 맞다.** 근거로 든 것은 rust-analyzer의 crate 32개·총 20만 줄과 "Some crates consist only of a single-file"(원문 축자 확인)인데, **이 인용의 문맥은 디렉터리 레이아웃이고 crate 크기 분포를 제시하지 않는다.** 인용은 정확했지만 결론을 함의하지 않았다.
  - https://matklad.github.io/2021/08/22/large-rust-workspaces.html
- **강등 항목:** tower-layer 425줄(LOC 조회 실패) · Zed 231 crate/130만 줄(2차 출처 DeepWiki, 원본 미대조) → 둘 다 **불확실**
- 남는 것: **"작아서 틀렸다"는 근거도 없다.** 크기는 질문이 아니다.

## F2. ★권위 있는 산업 가이드는 정반대로, 강하게 말한다 — **확실**(축자 확인)

Microsoft Pragmatic Rust Guidelines:

- **"If in doubt, split the crate"**
- 기준: **"Essentially, if a submodule can be used independently, its contents should be moved into a separate crate."**
- 근거: **"as this leads to dramatic compile time improvements—especially during the development of these crates—and prevents cyclic component dependencies."**
- **"crates are for items that can reasonably be used on their own"**
- 수치 임계값: 없음
- https://microsoft.github.io/rust-guidelines/guidelines/universal/index.html

★이것은 **최초 합성본이 통째로 놓친 항목**이며, 적대 리뷰가 누락 렌즈로 찾아냈다. 방향이 우리 결론과 정반대라 특히 중요하다.

**우리 `command`에 대입:** 워크스페이스 의존 0으로 독립 사용 가능하고, 순환·방향 때문에 뺐다 → 이 가이드의 기준을 **정면으로 충족한다.**

## F3. 이상적 그래프 형태 — 바닥 crate는 정석 위치다 — **확실**(축자 확인 4건)

matklad "Fast Rust Builds":

- **"a common vocabulary crate, a number of independent features, and a leaf crate to tie everything together"**
- **"The most important property of a crate is which crates it doesn't (transitively) depend on."**
- 선형 체인은 이득 없음: **"This is slow to compile, as all the crates need to be compiled sequentially: A -> B -> C -> D -> E"**
- **"you want to keep serialization at the boundary of the system, in the leaf crates"**
- https://matklad.github.io/2021/09/04/fast-rust-builds.html

**대입:** 아무것도 의존하지 않는 `command`는 이 그래프의 "공통 어휘 crate" 자리에 정확히 앉는다. **사용자가 의심한 「최상위 바닥이 없다」는 진단을 이 인용이 지지한다** — 이상적 형태의 출발점이 바로 그 자리다.

단서: 이는 **한 저자의 설계 지침이며 측정된 보편 임계값이 아니다**(리뷰 지적, 타당).

## F4. ★"방향 강제엔 crate밖에 없다"는 전제는 거짓이다 — **확실**(반증 확인)

최초 합성본의 핵심 주장이 리뷰에 **반증됐고, 내가 재대조해 확인했다.**

1. **`pub(in path)`는 crate 안에서도 세분 가시성을 만든다.** "pub(crate)는 이진적이라 계층 경계를 못 만든다"는 서술은 **틀렸다.** `pub(in crate::foo)`는 지정한 조상 모듈 서브트리로 가시성을 제한하고 그 밖의 접근은 컴파일 에러다.
   - https://doc.rust-lang.org/reference/visibility-and-privacy.html
2. **모듈 방향을 선언적으로 강제하는 도구가 실재한다.** `rust_arkitect`는 `rules_for_module()`로 `it_may_depend_on([...])`·`it_must_not_depend_on_anything()`을 선언하고 **유닛테스트로 강제**한다(소스 파싱, `Arkitect::ensure_that(project).complies_with(rules)`). 성숙도는 낮다 — **버전 0.3.7**. `arch_test_core`도 계층 규칙(`MayNotAccess`·`MayOnlyAccess`·순환 검사)을 제공한다.
   - https://docs.rs/crate/rust_arkitect/latest/source/README.md · https://docs.rs/arch_test_core/latest/arch_test_core/
3. ★**그리고 우리 리포는 이미 이 범주를 쓰고 있다** — CI의 `rg` 격리 게이트 + `cargo tree` 의존 상한 게이트가 정확히 "테스트로 방향 강제"다.

**결론: 컴파일러가 강제하는 1급 수단은 없다는 것만 참이다.** "그래서 crate로 쪼갤 수밖에 없다"는 따라오지 않는다.

정정: 최초 합성본은 `cargo-deny`·`machete`·`udeps`·`geiger`가 "전부 외부 crate 그래프만 본다"고 묶었는데, `geiger`는 소스를 스캔하고 `machete`는 소스+매니페스트를 본다. **어느 것도 모듈 방향 검사기는 아니라는 결론만 남는다**(리뷰 지적, 타당).

## F5. 방향은 한쪽이 아니다 — **확실**, 단 양쪽 다 단서가 붙는다

**되돌린 쪽 — tokio.** "rfc: collapse Tokio sub crates into single `tokio` crate"(closed). 축자 사유: "Maintaining a large number of crates comes with an increased maintainership burden." · "Maintaining correct dependencies between crates is complex." · "Users feel that large number of dependencies == bloat."
- https://github.com/tokio-rs/tokio/issues/1318
- ★**리뷰가 잡은 선택적 누락:** 같은 이슈가 **분리의 이점도 기록한다** — 덜 안정한 컴포넌트가 안정 코어를 깨지 않고 breaking release를 낼 수 있다(semver 격리). 이걸 빼면 사례가 한쪽으로 왜곡된다. **타당한 지적.**
- ★**귀속 정정:** "버전 조율 실패"·"사용자가 public/internal 서브crate 혼동"은 **이 이슈 본문에 없다** — 2차 블로그 서술이다.

**더 쪼갠 쪽 — ratatui**(0.30부터). 축자: "improve modularity, reduce compilation times, enable more flexible dependency management, and provide better API stability for third-party widget libraries."
- https://github.com/ratatui/ratatui/blob/main/ARCHITECTURE.md
- 단서: 문서는 **동기를 밝힐 뿐 각 효과가 실제로 개선됐다는 측정은 없다**(리뷰 지적, 타당).

**datafusion**도 추가 분할(사유 = 코드 관리·의존성 추론 용이성) — https://github.com/apache/datafusion/issues/1750

## F6. 비용 증거는 약하다 — **확실**(부재 확인)

- **통제된 A/B(동일 코드베이스, crate 수만 변수)는 존재하지 않는다.** 이것 자체가 발견이다.
- **Feldera 30분→2분은 우리에게 일반화되지 않는다.** 1,106 crate, 64코어/128스레드. 결정적으로 **쪼갠 대상이 SQL 컴파일러가 자동 생성한 코드**다("Since the Rust code is entirely auto-generated from this structure, we had total control over how to split it up."). 저자 자신의 경고 축자: **"In most Rust projects, splitting logic across dozens (or hundreds) of crates is impractical at best, a nightmare at worst."**
  - https://www.feldera.com/blog/cutting-down-rust-compile-times-from-30-to-2-minutes-with-one-thousand-crates
- **선형 체인 분할은 효과 없었다(ruff).** 축자: "since `ruff_cli` depends on `ruff` this doesn't really change the total build time" — https://github.com/astral-sh/ruff/issues/1820
  - 단서(리뷰): 같은 이슈가 **라이브러리만 빌드할 때는 이득**(약 100개 CLI 의존 회피)도 보고한다 → **범주적 무효가 아니라 워크로드 의존**. 타당.
- **순환 의존이 추가 분할을 막았다(ruff, 같은 이슈).** 축자: "the rule implementations depend on `ast::Checker` and `ast::Checker` depends on the rule implementations."
- **Cargo 메타데이터 오버헤드 28%(Bevy, bjorn3)** — 축자 확인. 단서(리뷰): **2020년 단일 워크로드이고 crate 수를 분리 변수로 통제하지 않았다.** 이후 Cargo에 리졸버 캐싱·성능 개선이 들어갔다 → **강등: 가능성 높음 → 참고 정황.**
- **orphan rule 실사례(Rust-for-Linux)** — 분할 시도가 orphan rule에 걸렸다: https://github.com/rust-lang/rust/issues/136979
  - ★**정정 2건(리뷰):** 우회가 "free function"이 아니라 **추가 trait**이다 · **`extern_impl`(RFC PR #3482)은 "unstable feature"가 아니라 닫힌 설계 제안**이다 — 기술적으로 사용 가능한 것처럼 쓰면 안 된다.
- **feature unification** — **contested / 불확실.** 최초 합성본은 cargo#12676을 "cargo가 회귀로 인정"으로 적었는데 **확인되지 않았다**(이슈는 S-triage, 메인테이너 판정 없음). 리뷰는 "재현자가 `resolver = "2"`를 안 써서 v1의 문서화된 동작일 뿐"이라 반박했는데, **재현자가 리졸버를 명시하지 않은 것은 확인됐지만 메인테이너 판정은 여전히 없다.** 양쪽 다 미확립.
  - https://github.com/rust-lang/cargo/issues/12676 · https://doc.rust-lang.org/cargo/reference/resolver.html
- **monomorphization 중복** — 강등: 특정 의존 토폴로지(형제 crate diamond)와 최적화 설정에 달렸고 `-Zshare-generics`로 공유 가능 → **조건부, 불확실.**
- **`pub(crate)`→`pub` 확대의 실피해 사례** — 못 찾음 → **불확실.**

## F7. "두 번째 소비자" 기준은 외부에 없고, 반례가 있다 — **가능성 높음**

- Cargo Book·rust-analyzer 문서·matklad·포럼 어디에도 **소비자 수 기반 규칙이 없다**(부재 확인).
- ★**반례(리뷰 지적, 타당):** rust-analyzer는 `ide`를 **소비자가 하나뿐인데도** API 경계 crate로 삼고, 반대로 `hir-expand`·`hir-def`·`hir_ty`는 **"never be an api boundary"로 명시 배제**한다. 즉 경계 판정이 소비자 수로 굴러가지 않는다.
  - https://rust-analyzer.github.io/book/contributing/architecture.html
- proc-macro는 분리가 **강제**되는 경우다. 단 "유일하게 강제되는 경우"라는 전칭은 근거 없음(리뷰 지적, 타당) — 다른 빌드·링크·타겟 제약을 배제하지 못한다.

## F8. 진행 중인 변화 (최초 합성본 누락 — 리뷰가 찾음)

전부 **미안정**이므로 지금 결정의 근거로 쓰지 않되, "나중에 그림이 바뀔 수 있다"로만 기록한다.

- Cargo `resolver.feature-unification`(nightly) — 패키지별 통합 모드 — 추적 이슈 #14774
- Cargo open namespaces(nightly, #13576) — 여러 패키지가 한 API 이름공간에 참여
- Cargo 워크스페이스 일괄 배포 지원 — 다중 crate 릴리스 부담 일부 경감. **단 원자적이지 않다**
- Rust project goals: "relink, don't rebuild" · incremental system rethought
- "Inline crates" 제안은 **저자 자신이 추진 의사 없음을 명시**했고 컴파일러 대개편을 전제한다 → 근미래 해법 아님

---

## 우리 리포에 대입 (사실 대조)

| 축 | 우리 상태 | 조사가 말하는 것 |
|---|---|---|
| 크기 | `command` 실코드 1,519줄(전체 4,705 중 32%) | 판단축이 아니다 |
| 독립 사용성 | 워크스페이스 의존 0 | **Microsoft 기준을 충족** → 분리 정당 |
| 그래프 위치 | 아무것도 의존하지 않는 바닥 | matklad 이상형의 "공통 어휘 crate" 자리 |
| 분리 동기 | 의존 방향 강제 | **그 이유만으론 crate가 필요 없다**(F4) |
| 소비자 수 | core + protocol(예정) = 2 | 외부 기준 아님, 반례 있음(F7) |
| 컴파일 이득 | 미측정 | 우리 규모에서 증거 없음(F6) |
| 죽은 코드 | route·link 약 185줄 호출자 0 · trait 구현·호출 0 | 구조 결정과 무관하게 순이득 |
| 도메인 중립 주차 | `core`에 약 880줄 | 최상위 바닥의 실질 근거(F3) |

★**단서(리뷰 지적, 타당):** 이 표의 우리 쪽 수치들은 **리비전 스탬프와 재현 쿼리가 없다.** 리팩터 한 번에 거짓이 될 수 있으므로 결정 기록에 박을 때 커밋 해시와 산출 명령을 같이 남겨야 한다.

---

## 쟁점 (승자를 고르지 않음)

1. **tokio(병합) vs ratatui·datafusion(분할)** — 둘 다 근거를 명시했고 둘 다 살아 있다. tokio 이슈 자체가 분리의 semver 격리 이점도 함께 적는다.
2. **우리 ADR-0130의 "두 번째 소비자" 기준 vs Microsoft의 "독립적으로 쓸 수 있으면"** — 후자가 훨씬 넓다. 우리 기준을 유지할지 교체할지는 **사용자 결정.**
3. **feature unification이 살아 있는 대가인지** — 양쪽 다 미확립.

## 한계 / 공백

- 부재 확인 3건이 결론에 실려 있다(줄 수 임계값 없음 · 소비자 수 규칙 없음 · 통제된 A/B 없음). 부재 확인은 검색 범위 한계와 구분되지 않는다.
- rustc 자신의 crate 분할 근거 1차 문서, Rust API Guidelines 해당 조항 — 도달 실패.
- tower-layer LOC · Zed 규모 — 미대조.
- 이전 세션 조사분(`core` 이름 금지 · bevy_core 해체)은 **이번 grounding 범위 밖**이다.
- **적대 리뷰는 1회**(medium). 리뷰가 놓친 것에 대한 백스톱은 없다.
