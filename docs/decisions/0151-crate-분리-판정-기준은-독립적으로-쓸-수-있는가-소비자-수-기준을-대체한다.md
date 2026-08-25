# ADR-0151: crate 분리 판정 기준은 독립적으로 쓸 수 있는가 — 소비자 수 기준을 대체한다

- 상태: 확정 (2026-08-17, 근거: 사용자 결정 2026-08-17 — `/research medium` + 타계열 적대 리뷰 정정 반영) · 부분 폐기 by ADR-0175 (바닥 crate 구성)
- 관련: Amends ADR-0130 (분리 판정 기준) · CLAUDE.md 「판단 기준」·「코어 격리」 · ADR-0012(모듈 격리) · ADR-0155(도구 crate 신설이 이 물음을 불렀다) · ADR-0110·ADR-0129(응집 이유를 별도로 가진 분리 선례) · Amended by ADR-0175 (바닥 crate 구성)

## 맥락

`command` crate가 **실코드 1,519줄**(전체 4,705줄 중 32%, 나머지는 테스트 48.5%·주석 15%)로 작고, **의존 방향만을 이유로 생긴 유일한 crate**라서 사용자가 물었다: *"소스 몇 개 안 되는데 이렇게 crate로 만드는 게 맞는지. 최상위 바닥이 없어서 생긴 증상 아닌가."*

기존 판정 기준은 ADR-0130의 **재론 트리거 1** — 「따로 쓸 소비자가 **실제로** 생긴다(가정이 아니라 착수된 소비처)」였다. 그 ADR이 "싸다는 게 해야 할 이유가 되지 못한다"로 제어 평면 분리를 기각할 때 세운 기준이다.

`/research medium`으로 외부 관행을 조사했고, 타계열 적대 리뷰가 **MATERIALLY-FLAWED** 판정과 HIGH 지적 6건을 냈다. 아래는 그 정정을 반영한 결과다.

## 결정

1. **crate 분리 판정 기준 = 「독립적으로 쓸 수 있는가」.** 지금 실제 소비자가 있는지는 묻지 않는다. 애매하면 쪼개는 쪽으로 기운다.
2. **독립적으로 쓸 수 없는 것은 쓰는 쪽 crate 안의 모듈로 남긴다.** 바닥 crate로 모으지 않는다.
3. **크기(줄 수)는 판단축이 아니다.** 작아서 틀렸다는 근거도, 커야 한다는 근거도 없다.
4. **★「의존 방향을 강제하려면 crate로 쪼개야 한다」를 근거로 쓰지 않는다 — 거짓이다.★**
5. **ADR-0130의 보류 결론은 유효하다.** 이 기준 교체가 그것을 자동으로 되열지 않는다(→ 영향 절).

## 거부한 대안

- **A. 「지금 두 번째 소비자가 있나」를 유지한다** — ㉠ **외부 권위 소스 어디에도 소비자 수 기반 규칙이 없다**(Cargo Book · rust-analyzer 아키텍처 문서 · matklad 3편 · users.rust-lang.org 전수 검색 — 부재 확인). ㉡ **반례가 실물로 있다:** rust-analyzer는 **소비자가 하나뿐인 `ide`를 API 경계 crate로 삼고**, 반대로 `hir-expand`·`hir-def`·`hir_ty`는 "never be an api boundary"로 **명시 배제**한다 — 경계 판정이 소비자 수로 굴러가지 않는다. ★**기각 근거 강도: 반례는 실물이지만 「규칙이 없다」 쪽은 부재 확인이라 약하다**(검색 범위 한계와 구분되지 않는다)★ — 사용자가 이 약점을 들은 상태에서 교체를 택했다(2026-08-17).
- **B. 줄 수 임계값을 둔다** — **임계값을 문서화한 프로젝트가 하나도 없었다.** 조사한 모든 사례가 「역할이 독립적으로 의미 있는가」로 판단한다.
- **C. 최상위 바닥 crate를 지금 신설한다** — 사용자 판단(「공수가 좀 있으면 나중에」). 조사는 그 자리 자체가 정석임을 지지했으므로 **기각이 아니라 유보**다.
- **D. 그 바닥 crate 이름을 `core`로 한다** — Rust 내장 `core`와 충돌해 **derive 매크로가 얽히면 깨진다**(axum#1133은 "고칠 계획 없음"으로 닫힘 · cargo#7760·rust#90960 열림). 이 저장소는 `ts-rs`·`serde` derive 의존이 많아 정면으로 걸린다. **단 이 항목은 이전 세션 조사분이며 이번 grounding 범위 밖이다.**
- **E. 계층 위치만 가리키는 이름의 바닥 crate에 도메인 중립 코드를 모은다** — Bevy `bevy_core`가 시간·이름표·태스크풀까지 빨아들이다 **해체돼 목적별로 흩어진** 선례(issue #2931). **이전 세션 조사분 — 이번 grounding 범위 밖.**

## 근거

**Microsoft Pragmatic Rust Guidelines (축자, 2026-08-17 원문 대조).**

> "You should err on the side of having too many crates rather than too few, as this leads to dramatic compile time improvements—especially during the development of these crates—and prevents cyclic component dependencies. Essentially, **if a submodule can be used independently, its contents should be moved into a separate crate.**"

- "crates are for items that can reasonably be used on their own" · 수치 임계값 **없음**.
- 같은 문서가 **되합치는 경우도 인정**한다: "In some cases, it is desirable to re-join individual crates back into a single umbrella crate."
- ★단서: **한 회사의 공개 가이드라인이고 생태계 합의가 아니다.** 근거도 그 문서 하나다. 이 결정이 그 한 출처에 기대고 있음을 숨기지 않는다.
- https://microsoft.github.io/rust-guidelines/guidelines/universal/index.html

**matklad "Fast Rust Builds" (축자 4건 원문 대조).**

- 이상적 형태: "a common vocabulary crate, a number of independent features, and a leaf crate to tie everything together"
- "The most important property of a crate is which crates it doesn't (transitively) depend on."
- 선형 체인은 순차 컴파일: "all the crates need to be compiled sequentially: A -> B -> C -> D -> E"
- → **아무것도 의존하지 않는 `command`는 이 그래프의 「공통 어휘 crate」 자리에 앉는다.** 단서: 한 저자의 설계 지침이며 측정된 보편 임계값이 아니다.

**★결정 4의 근거 — 「방향 강제엔 crate밖에 없다」가 거짓인 이유★**

- `pub(in path)`가 **지정한 조상 모듈 서브트리로 가시성을 제한**하고 그 밖의 접근은 컴파일 에러다(Rust Reference). "`pub(crate)`는 이진적이라 계층 경계를 못 만든다"는 서술은 틀렸다.
- **모듈 방향을 선언적으로 강제하고 유닛테스트로 검사하는 도구가 실재한다** — `rust_arkitect`의 `rules_for_module()` + `it_may_depend_on([...])`·`it_must_not_depend_on_anything()`(소스 파싱, `Arkitect::ensure_that(project).complies_with(rules)`). **성숙도는 낮다 — 버전 0.3.7.** `arch_test_core`도 계층 규칙(`MayNotAccess`·`MayOnlyAccess`·순환 검사)을 제공한다.
- **이 저장소는 이미 그 범주를 쓰고 있다** — CI의 `rg` 격리 게이트 + `cargo tree` 의존 상한 게이트가 정확히 「테스트로 방향 강제」다.
- 참인 것은 **「컴파일러가 강제하는 1급 수단이 없다」뿐**이고, "그래서 crate로 쪼갤 수밖에 없다"는 따라오지 않는다.

**비용 증거는 약하다(양방향 모두).**

- **통제된 A/B(동일 코드베이스, crate 수만 변수)가 존재하지 않는다.** 이것 자체가 조사의 발견이다.
- Feldera의 1,106 crate·30분→2분은 **SQL 컴파일러가 자동 생성한 코드**를 쪼갠 것이고, 저자가 직접 일반화를 경고했다: "In most Rust projects, splitting logic across dozens (or hundreds) of crates is impractical at best, a nightmare at worst."
- ruff는 **선형 체인으로 쪼개서 빌드 시간이 안 줄었다**("since `ruff_cli` depends on `ruff` this doesn't really change the total build time"). 단 같은 이슈가 라이브러리만 빌드할 때의 이득도 보고하므로 **범주적 무효가 아니라 워크로드 의존**이다.
- Cargo 메타데이터 오버헤드 28%(Bevy, bjorn3)는 2020년 단일 워크로드이고 crate 수를 분리 변수로 통제하지 않았다 → **참고 정황**으로만 쓴다.

**쟁점 — 승자를 고르지 않는다.** tokio는 서브crate들을 **하나로 병합**했고(유지보수 부담·의존 관리 복잡·"Users feel that large number of dependencies == bloat"), **같은 이슈가 분리의 semver 격리 이점도 함께 적는다**(덜 안정한 컴포넌트가 안정 코어를 깨지 않고 breaking release를 낼 수 있다). ratatui(0.30)·datafusion은 반대로 **추가 분할**했다. 둘 다 근거를 명시했고 둘 다 살아 있다.

**적대 리뷰 정정 반영.** 최초 합성본의 「1.5천 줄은 성숙 워크스페이스에서 평범하다」는 결론을 **철회했다** — 인용 8건은 축자 정확했으나 그것들을 합친 결론을 어느 출처도 함의하지 않았다(rust-analyzer 문서는 crate 개수·총 줄 수만 주고 크기 분포를 주지 않는다). 그래서 결정 3은 "작은 게 정상"이 아니라 **"크기는 축이 아니다"**로 적었다.

## 영향 / 불변식

- **★ADR-0130의 보류 결론은 유효하다 — 단 그것을 붙들고 있는 것은 근거가 아니라 위 결정 5다.★** 이 자리에 방어 논거를 세우지 않고 사실만 적는다.
  - **0130이 옵션 A(제어 평면만 분리)를 기각한 근거는 셋이다** — ㉠ **소비자 부재** ㉡ ADR-0129 결정 2가 그린 모양(응용 **전체**를 한 lib으로)과의 **불일치**(결정이 하나 더 늘어난다) ㉢ **부식 논거가 이 덩어리엔 약하다**. **㉠은 이 ADR이 방금 폐기한 바로 그 기준이다.** ㉡·㉢은 「지금 쪼개면 안 되는 이유」가 아니라 「그때 그 순서로 안 한 이유」다.
  - **0130의 나머지 근거(net 분리로 목적 달성 · 규모 착시 · "거기까지는 오버")는 옵션 A의 기각문이 아니다** — 범위 확대 전반, 특히 옵션 B(0129 완주)를 기각한 자리에 있다. 그것을 옵션 A 방어로 옮겨 쓰지 말 것.
  - **★새 기준으로 재면 0130 자신의 근거가 오히려 긍정 증거를 준다★** — `control/`은 다른 최상위 데몬 모듈로 **나가는 간선이 0인 leaf**(0130 §근거 ③)이고, 웹뷰 WS를 거치지 않는 **별개 입구**를 갖고(§근거 ④), **`net`과 맞닿지 않아** `frame_port` feature 문제를 비껴간다(§거부한 대안 옵션 A). 「독립적으로 쓸 수 있는가」에 셋 다 **예** 쪽 신호다. 규모 착시 문단마저 "그중 절반 가까이가 leaf인 제어 평면"이라고 적어 같은 방향을 가리킨다.
  - **그래서 보류를 실제로 붙들고 있는 것은 결정 5 — 사용자 판단 한 줄이다**(0130 맥락의 "거기까지는 오버"). **근거에서 도출되는 결론이 아니다.** 다음 세션이 이 줄을 논증으로 읽지 않도록 여기 명시한다.
  - **되열려면 갖춰져야 하는 것(사용자 재판정 사항)** — ① `control/`만 떼는 것과 ADR-0129 결정 2의 모양(응용 전체 한 lib)을 어떻게 화해시킬지 결정 ② `frame_port`의 feature 소속을 **착수 전에** 결정(0130 §영향 마지막 항목 — 공개 API 변경이라 미루면 비싸다) ③ 들어오는 간선 쪽 import 경로 수정 공수(세 모듈)를 지금 들일지에 대한 사용자 답. **셋 다 사람 판단이라 기계 트리거로 만들 수 없다.**
  - **재론 트리거 2(`control/`에서 나가는 production 간선)는 그대로 유효하다** — 이제는 성질이 하나 더 있다: 그것이 깨지면 leaf가 아니게 되어 **위 긍정 증거 자체가 사라진다.**
- **`command` crate의 정당화 문구를 바꾼다** — 「의존 방향 강제」가 아니라 **「독립적으로 쓸 수 있고 순환을 막는다」**. 그 crate의 `src/lib.rs` 헤더가 방향을 존재 이유로 적고 있으면 갱신 대상이다.
- **최상위 바닥 crate 후보 = `core`에 주차된 도메인 중립 약 880줄**(재생 상태기계 610 · 이름 파생 206 · 로깅 66 — 「여기 둬야 테스트가 돈다」는 이유로 거기 있다). **덩어리별로 따로 판정한다** — 셋을 뭉쳐 하나에 넣을 근거는 없고, 목적 이름을 가진 작은 crate 여러 개가 나올 수 있다(그게 matklad의 "independent features"에 맞고 `bevy_core` 실패도 피한다).
- **★개명 함정★** — 의존 상한 게이트들이 워크스페이스 멤버를 **`engram-dashboard` 이름 접두**로 식별한다. 접두 없는 이름으로 바꾸면 **그 게이트가 조용히 그 crate를 안 본다.** 같은 커밋에서 정규식을 고친다.
- **리비전 스탬프 의무** — 위 우리 쪽 수치(실코드 1,519줄 · 도메인 중립 880줄 등)는 **2026-08-17 시점값**이다. 이 수치를 근거로 쓰는 문서·ADR은 **재측정 방법을 함께** 남긴다. 안 남기면 리팩터 한 번에 거짓이 된다(적대 리뷰 지적).
  - **★이 ADR 자신의 수치를 재측정하는 방법 = ADR-0130 §근거 ⑤와 동일★** — 각 파일의 `#[cfg(test)] mod tests` **시작 줄 기준 분할**로 실코드와 테스트를 가른다(0130이 「약 8,000줄」 착시를 깰 때 쓴 그 방법). 도메인 중립 880줄은 세 덩어리(재생 상태기계 · 이름 파생 · 로깅)를 **따로** 재서 더한 값이므로 재측정도 덩어리별로 한다. ★**그 선례 밖의 세부는 기록이 없다**★ — 주석 15%를 어떤 규칙으로 떼었는지, 세 덩어리의 파일 경계가 정확히 어디였는지는 남아 있지 않다. **그래서 재측정값이 원값과 어긋나도 코드가 바뀐 것인지 세는 법이 다른 것인지 지금은 가릴 수 없다** — 다음 측정자는 쓴 기준을 수치와 함께 적어 그 구멍을 닫는다. 자기 규칙을 자기가 안 지킨 자리라 이 줄을 뒤늦게 채웠다.
- 이 결정은 **판정 기준**이다 — 어떤 crate를 언제 만들지는 여전히 사용자 결정이다.
