# 성숙한 프로젝트는 문서를 어떻게 쌓는가 — 5관점 조사

> **상태:** deep tier · 5갈래 병렬 수집(주계열) + 메인 grounding + **cross-family 적대 리뷰 1회(레벨 4) 완료 — 판정 `FIX`, 반영 완료(§11)**
> **날짜:** 2026-08-26 · **계기:** `docs/` 267개(ADR 174 · process 59 · research 24 · reference 3 · handbook 1) / 3.9MB
> **전제:** 이 저장소 문서의 **1차 독자는 AI 코딩 에이전트이고 사람 팀이 2차**다. 조사 전체가 이 전제로 관련성을 걸렀다.
> **확신도 범례:** `확실` = 1차 출처 축자 확인 · `가능성 높음` = 1차 출처 지지, 독립 교차확증 없음 · `불확실` = 미지지·2차 경유 → **인용 금지**
> **이 문서는 조사이지 결정이 아니다.** 채택 여부는 사용자 결정이고, 굵은 것은 ADR로 간다.

---

## 0. 요약 — 이 조사가 뒤집은 것 셋

**① ★확정된 결정 기록에 한해서는★ "정리해서 줄인다"는 선례를 찾지 못했다.**
Oxide RFD · Python PEP · Kubernetes KEP · Rust RFC · OpenStack nova-specs — 확인한 범위에서 오래된 **결정 기록**을 통합하거나 삭제하는 절차가 없다. 통제 레버는 **개수가 아니라 ① 건당 분량 상한 ② 분할 ③ 인덱스 ④ 상태 도장**이다.
⚠️ **이 결론을 "문서 일반"으로 넓히면 틀린다** — 버전 문서(Docusaurus)와 규격(W3C)에는 능동적 축소·은퇴 절차가 있다. §2 끝의 반례 절을 함께 볼 것.

**② 나이 기준 삭제는 실제로 시행됐다가 철회됐다** (2017 설문 근거 · Queens-era 2018 정책).
OpenStack이 문서를 나이로 지우는 정책을 돌렸고, 되돌렸다. 사용자 약 60%가 업스트림 미지원 버전을 쓰고 있었는데 그들 문서가 사라져 있었다. 새 정책 = **동결 후 영구 게시**.

**③ 컨텍스트 파일이 에이전트를 돕는다는 통설은 통제 평가에서 재현되지 않았다** (미동료평가 프리프린트).
성공률 개선 없음, 추론 비용 **평균 20% 초과**. 다만 **분해가 핵심이다** — *지시는 잘 따라졌고*, 도움이 안 된 것으로 **"저장소 개요(repository overview)"**가 특정됐다.
⚠️ **적용 범위 주의** — 이 연구가 잰 것은 **벤치마크 과업에서 항상 로드되는 컨텍스트 파일**이다. 우리 267개 온디맨드 문서 코퍼스를 직접 시험한 것이 아니다(§10 범위표).

그리고 이 조사에서 가장 실행 가능한 답 하나:

**④ 성숙한 곳이 실제로 한 것은 "역사 기록"과 "현재 진실"의 분리다.**
PEP 1이 축자로 이렇게 박아 놨다 — *"해결이 나면 PEP는 살아 있는 명세가 아니라 역사 문서로 간주된다"*, 그리고 *"기대 동작의 정식 문서화는 Language Reference·Library Reference·PyPA Specifications 같은 **다른 곳**에서 유지되어야 한다."* Rust RFC도 같은 취지를 프로세스 문서에 선언한다. **우리는 이 선이 흐리다.**

---

## 1. 관점 A — 분류 체계

### 우리 문서 뭉치는 Diátaxis가 다루는 대상이 아니다

- Diátaxis 4분면(tutorial/how-to/reference/explanation)에는 **결정 기록·연구 보고·작업 로그가 들어갈 칸이 없다.** — 출처: https://diataxis.fr/start-here/ — `확실`
- 가장 자주 인용되는 도입 사례인 **Cloudflare의 실제 스타일 가이드는 4분면이 아니라 14종 콘텐츠 타입**이고 그 페이지는 Diátaxis를 언급하지 않는다. 추가된 것 중 **design guide · reference architecture · changelog**가 정확히 우리 research/reference/step-log에 대응한다. — 출처: https://developers.cloudflare.com/style-guide/documentation-content-strategy/content-types/ — `확실`
- Diátaxis 공식 입장이 **일괄 재구조화를 명시적으로 말린다** — *"거대한 난장을 치우는 중이라면 전부 헐고 새로 시작하고 싶어진다. 피하라."* 대신 요소 하나 → 평가 → 단일 다음 행동 → 반복. — 출처: https://diataxis.fr/how-to-use-diataxis/ — `확실`
- 비판도 문서화돼 있다 — Hillel Wayne: 4분면은 *도구* 문서화용으로 설계됐고 프레임워크·언어에는 안 맞으며, 빠진 것이 **Conceptual Overview**와 **Snippets/Examples**다. — 출처: https://www.hillelwayne.com/post/problems-with-the-4doc-model/ — `확실`
- **Diátaxis 도입의 정량 성과(before/after)를 어느 조직에서도 찾지 못했다.** Canonical은 수치를 안 냈고, 대신 부작용을 실토한다 — 적용 직후 문서는 **더 나빠 보인다**(문제가 숨을 곳이 없어져서). — 출처: https://canonical.com/blog/diataxis-a-new-foundation-for-canonical-documentation — `가능성 높음`

### arc42는 ADR을 자기 안 한 칸으로 흡수한다

- arc42 12절 중 **§9가 아키텍처 결정의 자리**이고 "다른 곳에 기술되지 않았다면 여기"라는 단서가 붙는다. 즉 arc42는 ADR을 대체하지 않고 흡수한다. 우리 `architecture-overview.md`가 1~8·10~12에, `docs/decisions/`가 §9에 대응한다. — 출처: https://arc42.org/overview/ — `확실`
- **C4는 다이어그램 표기법이지 문서 분류 체계가 아니다** — "267개를 어떻게 나누나"에 답을 주지 않는다. — 출처: https://c4model.com/ — `가능성 높음`

### 다섯 조직이 다섯 개의 서로 다른 "기록할 가치" 경계선을 긋는다

| 조직 | 경계 판정 기준 | 우리와의 대비 |
|---|---|---|
| **Rust RFC** | *영향 범위* — "형태는 바꾸되 의미는 안 바꾸는 것"은 RFC 불필요. "rust 개발자만 알아챌 변경"도 불필요 | 우리는 내부 리팩터도 ADR을 남긴다 |
| **Go** | **2단계로 비용을 나눈다** — 1단계 = 이슈 하나("이 시점에 설계 문서는 필요 없다"), 2단계 = 트리아지가 요청했을 때만 설계 문서 | 우리는 비싼 문서가 기본값이다 |
| **Python PEP** | *대상*으로 나눈다 — Standards Track / Informational(**"무시해도 된다"**) / Process(**"무시할 수 없다"**) | 우리는 강제 규약과 배경 설명이 같은 번호 공간에 섞여 있다 |
| **Kubernetes KEP** | **낮은 임계를 의도** — "KEP가 기본 위치가 되도록 가볍게" | 큰 번호대는 실패가 아니라 설계된 결과 (★번호 = 추적 이슈 번호이지 문서 수가 아니다★) |
| **Oxide RFD** | **가장 넓은 우산** — 기술 설계 + 회사 프로세스 + 문화 + 채용 전부 | 우리 ADR+step-log+research를 한 우산에 넣은 유일한 검증 선례 |

출처: https://github.com/rust-lang/rfcs/blob/master/README.md · https://github.com/golang/proposal/blob/master/README.md · https://peps.python.org/pep-0001/ · https://github.com/kubernetes/enhancements/blob/master/keps/README.md · https://rfd.shared.oxide.computer/rfd/0001 — 전부 `확실`

★**PEP의 Informational/Process 구분선이 우리에게 직접 걸린다**★ — 그 선은 **"무시해도 되나"**다. 우리 ADR 중 일부는 강제 규약이고 일부는 배경 설명인데 같은 번호 공간에 있다면, 에이전트는 어느 것을 반드시 따라야 하는지 못 가른다. **이 구분을 ADR에 적용한 선례는 찾지 못했다.**

---

## 2. 관점 B — 결정 기록 174개라는 규모

### 174는 통계적 극단이다

- GitHub MSR 연구(Buchgeher et al., IEEE Access 2023): **ADR을 쓰는 저장소의 약 50%가 ADR 1~5개**에 그친다 — "시도했으나 정착하지 못했다"는 신호. ADR 채택률 자체가 여전히 낮다. — 출처: https://ieeexplore.ieee.org/document/10155430/ — `가능성 높음`(검색 요약 + 독립 수집자 일치, 원문 초록 직접 대조는 페이월로 실패)
- **순수 ADR 포맷으로 100+에 도달한 공개 코퍼스를 찾지 못했다.** Backstage 자체 ADR은 15건, 정부기관 컬렉션도 수십 건대. — `가능성 높음`(전수 조사 아님)

★**결론: 참조할 동종 선례가 ADR 커뮤니티에 없다.** 번호 공간을 수백~수천으로 굴려 본 곳(RFD·KEP·PEP·RFC)이 우리 피어다.★

### 규모에 도달한 곳들이 실제로 지은 것

- ⚠️ **Oxide "500+ RFD / 140만 단어"는 내부 코퍼스에 대한 팟캐스트 구두 주장이고 독립 검증이 없다.** 공개 인덱스는 결과 80건만 노출한다(ID는 634 같은 높은 번호까지 존재 — **ID 상한 ≠ 문서 수**). — 출처: https://oxide-and-friends.transistor.fm/episodes/rfds-the-backbone-of-oxide/transcript · https://rfd.shared.oxide.computer/ — `불확실`(내부 주장 — 수치로 인용하지 말 것)
- ★**Oxide의 규모 한계 실토**★ — *"전문 검색조차 반드시 필요한 걸 다 주지는 않는 변곡점에 와 있다."* 그 전 단계는 *"RFD를 검색할 수 없다는 것이 극도로 고통스러웠다."* — 출처: 동일 — `확실`
  - 대응으로 지은 것: self-hosted Meilisearch 전문검색 · RFD 간 링크 호버 프리뷰(제목·저자·상태·갱신일) · 번호/제목 점프 · 라벨 · 저자 필터. 남은 과제로 **"컬렉션(문서 묶음) 만들기"**를 명시. — 출처: https://oxide.computer/blog/a-tool-for-discussion — `확실`
- **PEP 0 인덱스는 살아 있는 것과 죽은 것을 섹션으로 갈라 놓는다** — Open/Accepted/Provisional/Finished를 앞에, "Rejected, Superseded, and Withdrawn"을 맨 뒤 별도 섹션에. 174개를 한 덩어리로 보여주지 않는 실증 패턴. — 출처: https://peps.python.org/ — `확실`
- **Kubernetes는 디렉터리 = SIG(서브시스템) 분할이 1차 인덱스**다. — 출처: https://www.kubernetes.dev/resources/keps/ — `확실`
- **`adr-log`는 인덱스를 손유지가 아니라 마커 사이 재생성으로 푼다** — 파일에 `<!-- adrlog -->` 주석을 두면 그 구간을 통째로 갈아끼운다. 인덱스 rot의 구조적 해법. — 출처: https://github.com/adr/adr-log — `확실`

### 상태 위생을 사람에게 맡긴 곳은 없다

- **Oxide** — PR 상태와 어긋나면 **봇이 RFD 상태를 교정한다.** — 출처: https://rfd.shared.oxide.computer/rfd/0001 — `확실`
- **Ethereum EIP** — Draft/Review로 6개월 무활동이면 **봇이 자동으로 Stagnant로 내린다**(eth-bot이 실제 PR을 연다). Withdrawn은 번호 재사용 불가의 종국 상태. — 출처: https://eips.ethereum.org/EIPS/eip-1 · https://github.com/ethereum/EIP-Bot — `확실`
- ★**Kubernetes는 상태를 기계 판독 `kep.yaml`에 둔다**★ — `status`(provisional/implementable/implemented/deferred/rejected/withdrawn/**replaced**) · **`replaces`** · **`see-also`** · `stage`. presubmit이 포맷을 강제한다. — 출처: https://github.com/kubernetes/enhancements/blob/master/keps/NNNN-kep-template/kep.yaml — `확실`
  - ★**우리에게 가장 직접 이식 가능한 패턴이 이것이다.** 우리는 supersede 관계를 본문 산문에 두고 있어 grep으로만 잡힌다. YAML이면 파싱으로 잡히고 CI가 양방향 무결성을 검사할 수 있다.★
- **PEP은 양방향 도장을 강제한다** — 새 PEP에 `Replaces:`, 옛 PEP에 `Superseded-By:`. **단 PEP 1은 이 supersede를 "Informational PEP용으로 의도된 것"으로 한정해 서술한다** — 우리 규약처럼 전 종류에 적용하는 것과는 범위가 다르다. — 출처: https://peps.python.org/pep-0001/ — `확실`
- 반면 ★**MADR 4.0에서 `status` 필드가 선택 사항이 됐다**★ — 표준 템플릿조차 상태 강제를 포기했다. — 출처: https://adr.github.io/madr/ — `확실`
- `pyadr`은 propose/accept/reject만 자동화했고 **deprecate·supersede는 "not yet implemented"** — 상태 위생 도구화가 실무에서 미완이라는 증거. — 출처: https://github.com/opinionated-digital-center/pyadr — `확실`

### 입도 — 실패는 "잘게 쪼갬"이 아니라 "묶음"이었다

- ★IBM Watson Discovery 팀 2년·80+ ADR 경험 보고: *"하나의 ADR에 여러 얽힌 결정이 묶여 있었다."*★ 초기 ADR은 결정 뒤엉킴·분석 빈약·비아키텍처 세부 혼입이 반복됐고, 템플릿을 고쳐 **"품질 속성과 전략적 기술부채에 영향을 주는가"**로 기준을 좁혔다. — 출처: https://agilealliance.org/resources/experience-reports/distribute-design-authority-with-architecture-decision-records/ — `확실`
  - 같은 보고서: 2년 뒤에도 팀의 **절반만** ADR을 상시 작성했고, ADR이 2건뿐인 서비스가 12건 가까운 서비스보다 기술부채가 뚜렷이 많았다. "화이트보드 먼저, 결정이 실제로 난 뒤에만 기록" 규칙으로 리뷰 churn을 줄였다.
- Nygard 원본 기준: 문서 하나가 **1~2쪽**, 하나에 하나의 결정. — 출처: https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions — `확실`
- Fowler: 결정 기록은 *"short and to the point - typically a single page"*, 부수 자료는 링크로 뺀다. ★**append-only 코퍼스의 실질 상한은 개수가 아니라 "건당 1페이지"다.**★ — 출처: https://martinfowler.com/bliki/ArchitectureDecisionRecord.html — `확실`
- ⚠️ **Spotify는 임계를 정의하지 않고 팀에 위임한다**(*"무엇이 유의미한 영향인지는 각 팀이 정렬한다"*) — 이건 **장기근속 인간 팀의 암묵지에 의존하는 방식이고, AI 세션이 1차 독자인 환경에는 이식되지 않는다.** — 출처: https://engineering.atspotify.com/2020/04/when-should-i-write-an-architecture-decision-record — `확실`

### 소거·통합 — negative result (범위 한정)

- **확인한 결정 기록 코퍼스에서 통합·삭제 절차를 찾지 못했다.** Oxide는 `abandoned`로 남기고, PEP는 인덱스 뒤쪽 섹션으로 분리만 하며, Kubernetes에는 archive/retired 디렉터리가 없고, Rust RFC는 *"실질적 변경은 새 RFC로, 원본에는 주석을 단다"*로 새 문서를 강제한다. — `확실`(각 문서 내 부재 확인) / **"세상에 존재하지 않는다"는 전칭 주장은 `불확실`**
- Oxide 팟캐스트에는 오히려 반대 방향 발화가 있다 — *"아주 오래전에 쓴 것들 중 일부가 우리가 어디에 있었는지 이해하는 데 정말 필수적이 됐다."* — `확실`

### ★그러나 "문서 일반"으로 넓히면 반례가 있다★ (적대 리뷰가 적출)

위 부재 관찰은 **확정된 결정 기록**에만 성립한다. 다른 문서 종류에는 능동적 축소·은퇴 절차가 실재한다:

- **Docusaurus는 버전 문서의 능동 축소를 공식 권고한다** — 활성 버전을 10개 미만으로 유지하고, 버전을 **삭제**하는 절차가 있으며, 옛 버전은 외부 immutable 아카이브로 뺀다. **append-only 결론의 직접 반례.** — 출처: https://docusaurus.io/docs/versioning — `가능성 높음`
- **W3C는 "상태 도장만 하고 내용은 보존"보다 강한 은퇴 모델을 갖는다** — Discontinued / Obsolete / Superseded / Rescinded 상태와 **본문 제거**까지 규정한다. — 출처: https://www.w3.org/policies/process/ — `가능성 높음`
- ★**IETF는 이단계 수명주기다**★ — **Internet-Draft는 6개월 후 제거**되고, 발행된 RFC는 영구 보존된다. **제안 전 단계의 소거와 확정 기록의 보존을 분리한 모델**이고, 우리 step-log(진행 중 기록)와 ADR(확정 기록)의 구분에 가장 가깝다. — 출처: https://www.rfc-editor.org/info/rfc2026 · https://www.rfc-editor.org/series/rfc/ — `가능성 높음`

⚠️ **형식별로 결론이 다르다는 것이 이 절의 요점이다.** ADR·RFC·RFD·KEP·PEP를 한 모집단으로 묶어 전칭 추론하면 안 된다 — KEP는 기능·작업추적·설계 문서의 복합 단위이고, Rust RFC는 수락된 *변경 제안*이며, RFD는 회사 프로세스까지 포함하는 우산이다. 보존 규칙은 **형식마다 따로 읽어야 한다.**

---

## 3. 관점 C — drift 방지

### 실제로 드리프트를 막는 기계는 세 종류뿐이다

① **재생성 후 `git diff --exit-code`** ② **문서를 실행 가능한 테스트로** ③ **문서가 참조하는 코드 심볼·경로가 실재하는지 검사**
나머지(신선도 날짜·링크체커·프로즈 린터·CODEOWNERS)는 *알림*이거나, 게이트여도 **내용 정확성과 무관한 축**을 잰다.

### 우리가 이미 하는 것 = 업계 정본 패턴, 단 공개된 실패 사례가 있다

- HashiCorp 공식 스캐폴딩이 **문자 그대로 같은 형태**를 쓴다 — `make generate` 후 `git diff --compact-summary --exit-code || (echo "Unexpected difference..."; exit 1)`. — 출처: https://github.com/hashicorp/terraform-provider-scaffolding-framework/blob/main/.github/workflows/test.yml — `확실`
- TypeScript API Extractor는 생성물을 **리뷰 가능한 diff로 강등**시켜 커밋한다 — 재생성분과 다르면 "You have changed the public API signature"로 빌드를 깬다. — 출처: https://api-extractor.com/pages/setup/configure_api_report/ — `확실`
- ★**그 패턴을 버리자는 공개 회고**★ — Alex Eagle(2025-10): 사소한 수정 PR이 빨간 빌드로 되돌아오고, 재생성 도구가 무거워 *"10초짜리 수정에 codegen 갱신 10분"*이 들어 **기여자가 수정을 포기한다.** 결론 = 생성물을 저장소에서 빼자. — 출처: https://aspect.build/blog/stardocs-on-bcr — `확실`
  - 완화책도 있다 — `cog --check-fail-msg`는 실패 시 **"우리 프로젝트에선 이렇게 돌려라"** 메시지를 삽입한다. 위 마찰을 정확히 겨냥. — 출처: https://cog.readthedocs.io/en/latest/running.html — `확실`

### 실행 가능한 문서

- Go 공식 근거 문장이 이 분야에서 가장 강하다 — 실행 가능한 문서는 *"API가 바뀌어도 그 정보가 낡지 않음을 보장한다."* — 출처: https://go.dev/blog/examples — `확실`
- **rust-lang/book CI가 문서 게이트의 가장 밀도 높은 실제 예다** — 한 워크플로에 6개: `mdbook test`(책 안 모든 코드 컴파일·실행) · 절대경로 유출 린트 · 참조 검증 · 스펠체크 · shellcheck · linkchecker. 전부 push/PR마다. — 출처: https://github.com/rust-lang/book/blob/main/.github/workflows/main.yml — `확실`
- **Kubernetes는 문서의 YAML 예제를 실제 API 검증 함수로 테스트한다** — 문서 *자산*도 실행 테스트 대상이 될 수 있다는 실증. — 출처: https://github.com/kubernetes/website/blob/main/content/en/examples/examples_test.go — `확실`
- ★단 **그 하네스 자체가 드리프트했다**★ — 실행 스크립트가 아직 `$TRAVIS_BUILD_DIR`를 쓰고 GH Actions에 등재돼 있지 않다. **검증 하네스도 문서처럼 썩는다.** — 출처: https://github.com/kubernetes/website/blob/main/scripts/test_examples.sh — `가능성 높음`(Prow 등재 여부 미확인)
- Elixir는 doctest를 **테스트 대체가 아니라 문서 무효화 탐지기**로 한정한다. — 출처: https://hexdocs.pm/ex_unit/ExUnit.DocTest.html — `확실`
- Rust CLI 도구는 **markdown 안 CLI 세션을 그대로 테스트로 돌릴 수 있다**(trycmd) — `.md`를 테스트 케이스로 받고 `TRYCMD=overwrite`로 스냅샷 일괄 갱신. — 출처: https://docs.rs/trycmd/latest/trycmd/ — `확실`

### 링크체커는 조용히 부패한다 (실증)

- ★kubernetes/website `.htmltest.yml`은 **링크·외부·이미지 검사가 전부 꺼져 있고** GH Actions에 등재도 안 돼 있다★. `IgnoreDirectoryMissingTrailingSlash`는 true와 false로 두 번 선언돼 있다. — 출처: https://github.com/kubernetes/website/blob/main/.htmltest.yml — `확실`
- 살아남은 쪽은 **범위를 좁혔다** — rust-lang/rust 자체 linkchecker는 **상대 링크만** 검사하고 예외를 코드 안 상수로 allowlist한다. — 출처: https://github.com/rust-lang/rust/blob/main/src/tools/linkchecker/main.rs — `확실`
- ★**게이트는 좁혀야 산다**★가 이 절의 결론이다.

### "이 디렉터리가 바뀌면 문서도 바꿔라"의 실물

- ★**rust-lang/rust `triagebot.toml`**★ — 경로별 `[mentions."..."]`가 봇 코멘트를 남긴다. 예: compiletest directives를 고치면 "rustc-dev-guide에 해당 문서를 갱신하라". **강제가 아니라 조언성 알림이고 그것이 의도된 설계다.** — 출처: https://github.com/rust-lang/rust/blob/main/triagebot.toml — `확실`
- ⚠️ **CODEOWNERS는 방향이 반대다** — "문서가 바뀌면 누가 본다"만 강제하고 "코드가 바뀌면 문서도 바뀐다"는 강제 못 한다. 드리프트 방지 수단으로 오해하면 안 된다. — 출처: https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/about-code-owners — `확실`

### 단일 정본 강제의 기계화 = transclusion

- **mdBook `{{#include}}`의 앵커가 공식 권고다** — *"포함된 파일을 수정할 때 책이 깨지는 것을 피하려면 라인 번호 대신 앵커로 특정 구간을 포함할 수 있다."* 앵커 정규식은 어떤 언어의 주석에도 넣을 수 있고, `{{#rustdoc_include}}`로 포함한 구간은 **`mdbook test`가 그대로 테스트한다**. — 출처: https://rust-lang.github.io/mdBook/format/mdbook.html — `확실`
- Sphinx `literalinclude`도 같은 결론 — *"소스 파일의 태그 주석과 함께 쓰면 라인 번호를 손으로 적을 필요가 없다."* — 출처: https://www.sphinx-doc.org/en/master/usage/restructuredtext/directives.html — `확실`
- **Rust 네이티브** — `#![doc = include_str!("../README.md")]` + `#[cfg_attr(doctest, doc = include_str!(...))]`로 README 안 코드 예제를 doctest로 돌린다. — 출처: https://doc.rust-lang.org/nightly/edition-guide/rust-2024/rustdoc-nested-includes.html — `가능성 높음`
- **문서 간 중복을 기계로 탐지하는 경로도 있다** — jscpd가 마크다운을 블록 단위로 토큰화해 중복률이 임계를 넘으면 CI 실패. — 출처: https://jscpd.dev/ — `가능성 높음`
- ⚠️ 다만 **중복을 자동 검출·차단하는 게이트를 실제로 돌리는 프로젝트는 찾지 못했다.** 중복 금지는 전부 리뷰어 규범으로만 집행된다. — `가능성 높음`(부정 결과)

### 문서 존재 자체를 강제하는 게이트 — 우리에게 새로운 축

- ★**rust-lang/rust의 tidy `unstable_book` 검사는 "코드 항목 ↔ 문서 페이지"를 양방향으로 강제한다**★ — Unstable Book에 있는 절이 실제 unstable feature와 대응하지 않으면 CI 실패. **문서 쪽에 유령 항목이 남는 것을 잡는다.** 우리 ADR 인덱스/앵커 정합성 검사와 같은 축. — 출처: https://github.com/rust-lang/rust/blob/main/src/tools/tidy/src/unstable_book.rs — `확실`
- 문서 속 코드 심볼 참조 유효성은 **학술적으로도 다뤄졌다** — 3,000+ GitHub 프로젝트에서 *"대부분의 프로젝트가 이력 중 어느 시점엔 최소 하나의 낡은 코드 요소 참조를 갖는다."* — 출처: https://arxiv.org/abs/2212.01479 — `가능성 높음`
- 통계적 대표 표본 356개 저장소에 기존 일관성 검사기를 돌린 것만으로 **23.0%에서 stale 코드 요소 참조** 검출. — 출처: https://arxiv.org/abs/2606.09090 — `가능성 높음`(미동료평가 프리프린트)

---

## 4. 관점 D — 양 통제와 폐기

### 부피 통제의 실제 조합은 넷

① 중복 금지(정본 링크 강제) ② 시간축 로그의 **분할** ③ 상태 도장 + 리다이렉트 ④ **기계적** 정기 점검(깨진 링크·만료 리다이렉트)

하드 삭제를 명문화한 곳(GitLab docs·MDN·GOV.UK)조차 반드시 **리다이렉트 + 별도 아카이브**를 함께 건다.

### 삭제 / 아카이빙

- MDN은 은퇴 절차를 문서화했다 — 판정 기준 4개(기술 폐기 / 다른 곳이 더 잘 유지 / 전략 불일치 / **유지비가 사용자 가치를 초과**), 3~6개월 타임라인, 배너 → museum repo 아카이브 → 리다이렉트. *"먼저 아카이브하지 않고 영구 삭제하지 말 것."* — 출처: https://developer.mozilla.org/en-US/docs/MDN/Writing_guidelines/Howto/Retiring_content — `확실`
  - ★리다이렉트를 **홈으로 보내지 않는다**★ — 후속 문서가 있으면 그쪽, 없으면 "Retired content" 목록의 해당 앵커로. *"MDN 홈으로 리다이렉트하지 말 것 — 독자가 설명 없이 남겨진다."*
- GitLab은 **삭제에 3개월 유예를 기계로 박는다** — 제목 도장(`(deprecated)`→`(removed)`), 경고 알럿, `remove_date` 주석 래핑, `redirect_to` 프론트매터, 내비 제거, rake 태스크. **리다이렉트 자체에도 수명이 있다**(내부 3개월, 외부 1년). — 출처: https://docs.gitlab.com/development/documentation/styleguide/deprecations_and_removals/ — `확실`
- GOV.UK는 **철회와 비공개를 다른 동사로 분리**한다 — 철회: URL 유지 + 배너. 비공개: 제거 + 대체 URL 리다이렉트. 그리고 **제거하면 안 되는 사유도 명문화**한다("행사가 지나서"·"담당자가 바뀌어서"·"낡아서"는 사유가 아니다). — 출처: https://guidance.publishing.service.gov.uk/publish-update-retire-content/standard-content-types/withdraw-unpublish-standard/ — `확실`

### ★나이 기준 삭제는 시행됐다가 철회됐다★

- OpenStack: *"제안하는 변경은 docs.openstack.org에서 콘텐츠를 그 나이, 또는 적용 대상 소프트웨어의 나이, 또는 복구된 저장소의 나이에 근거해 삭제하는 것을 **중단**하는 것이다."* — 출처: https://specs.openstack.org/openstack/docs-specs/specs/queens/retention-policy.html — `확실`(축자 확인)
  - 사유: *"이 글을 쓰는 시점의 최신 사용자 설문에 따르면, 사용자의 약 60%가 업스트림에서 더 이상 지원되지 않는 OpenStack 버전을 운영 중이다."*(2017 설문) — `확실`
  - 새 정책 = **동결(freeze) 후 영구 게시.** 같은 스펙이 **아카이브 관리보다 그냥 두는 게 소규모 팀에 더 지속가능하다**고 판단한다. — `확실`

### 중복 금지

- 가장 인용 가치 있는 문장은 Kubernetes 것이다 — *"가능한 한 Kubernetes 문서는 이중 출처 콘텐츠를 호스팅하는 대신 정본 출처로 링크한다. 이중 출처 콘텐츠는 유지에 **두 배(또는 그 이상)**의 노력이 들고 **더 빨리 낡는다**."* — 출처: https://kubernetes.io/docs/contribute/style/content-guide/ — `확실`
- ⚠️ **GitLab의 SSOT는 "중복 금지"가 아니다** — *"이 정책은 콘텐츠가 문서 내 여러 곳에 중복될 수 없다는 뜻이 아니다."* **SSOT = 권위의 단일성이지 텍스트의 유일성이 아니다.** — 출처: https://docs.gitlab.com/development/documentation/styleguide/ — `확실`
- Google은 **중복 발견 시 처리 절차**를 규정한다 — *"정본 문서를 지정하라: 1차 출처를 정하고 다른 연관 문서를 그 1차 출처로 통합하라(또는 중복본을 폐기하라)."* — 출처: https://abseil.io/resources/swe-book/html/ch10.html — `확실`

### 시간순 저널(우리 step-log 59개) — 가장 직접적인 구역

- ★**Node.js는 CHANGELOG를 메이저 릴리스 라인별 파일로 쪼갰다**★ — 계기 발언: *"CHANGELOG.md가 거대하다. 로컬 마크다운 뷰어가 미리보기를 시도하다 멈춘다. 연도별로 아카이브하기 시작해야 할 것 같다."* — 출처: https://github.com/nodejs/node/issues/5533 — `확실`
  - ★**주목**★ — **연도별 아카이브 제안은 채택되지 않았고 "의미 단위(메이저 라인)별 분할"로 귀결됐다.** 시간이 아니라 *경계*로 잘랐다. — `가능성 높음`
- ★**OpenStack nova-specs가 append-only 코퍼스의 최상 구조 선례다**★ — `specs/<release>/approved`(승인·미구현) → 릴리스 종료 시 `specs/<release>/implemented`로 **이동 + 리다이렉트 생성**, 그 밖에 `specs/backlog/approved`와 `specs/abandoned`. 목적 명시: *"이 디렉터리 구조가 우리가 하려 했던 것, 하기로 한 것, 실제로 해낸 것을 보여준다."* — 출처: https://specs.openstack.org/openstack/nova-specs/readme.html — `확실`
- **Wikipedia 토론 페이지 자동 아카이빙이 "계속 자라는 시간축 로그"의 유일한 완전 기계화 선례다** — 봇이 나이/스레드 수 기준으로 잘라 번호 매긴 아카이브로 옮기고 동결·색인한다. 단 **임계값에 전역 합의가 없고 페이지별 로컬 합의로 정한다.** — 출처: https://en.wikipedia.org/wiki/Help:Archiving_a_talk_page — `가능성 높음`
- **Kubernetes SIG 연간 리포트가 "저널을 요약하는" 유일한 확인 선례**인데, **원본 로그를 대체하지 않는다** — 위에 얹히는 별도 산출물이다. — 출처: https://github.com/kubernetes/community/blob/main/committee-steering/governance/annual-reports.md — `가능성 높음`
- ★**결정 로그나 엔지니어링 저널을 "요약해서 원본을 잘라내는" 선례는 찾지 못했다.** 통제는 전부 *분할*과 *색인*이다.★ — `확실`(부정 결과, 조사 범위 내)
- Keep a Changelog에는 **보존·아카이브·분할 조항이 아예 없다.** 있는 것은 `[YANKED]` 표식뿐. 시간축 로그 표준이 부피 문제를 다루지 않는다는 것 자체가 관측이다. — 출처: https://keepachangelog.com/en/1.1.0/ — `확실`

### 문서 예산 / 상한

- ★**"페이지 하나 추가하려면 하나 지워라" 류 예산 정책은 어디서도 찾지 못했다.**★ — `확실`(부정 결과)
- **수치 상한을 성문화한 유일한 대규모 선례는 Wikipedia다** — 읽기 가능 산문 15,000단어 초과 "거의 확실히 분할", 9,000 초과 "아마도", 6,000 미만 "길이만으로는 분할 사유 아님". 근거에 **유지보수성**이 명시된다. — 출처: https://en.wikipedia.org/wiki/Wikipedia:Article_size — `확실`
- GitLab은 **헤딩 깊이를 페이지 분할 신호로 쓴다** — *"다섯 단계 넘는 헤딩이 필요하면 새 페이지로 옮겨라."* — 출처: https://docs.gitlab.com/development/documentation/styleguide/ — `가능성 높음`

### 정기 점검은 편집 감사가 아니라 기계 점검이다

- GitLab 핸드북의 월 단위 점검: stale codeowners 제거 · 깨진 외부 링크 · **만료 리다이렉트** · 링크 안 된 이미지 · 이미지 압축 · markdownlint·vale. ⚠️ **이건 현행 정책 문서가 아니라 2024-07 작업 항목 한 건에서 관찰한 실행 사례다.** — 출처: https://gitlab.com/gitlab-com/content-sites/handbook-tools/maintenance-tasks/-/issues/4 — `가능성 높음`(2024년 관찰 사례)
- Google은 **문서 신선도를 소유자에게 자동 push**한다 — freshness date + 3개월 미변경 시 소유자 메일. "Last reviewed by..." 바이라인 노출이 채택률을 올렸다. 전제: *"코드처럼 문서에도 소유자가 있어야 한다."* — 출처: https://abseil.io/resources/swe-book/html/ch10.html — `확실`
  - ⚠️ **이건 문서 담당 인력을 전제한다.** 리마인더를 받고 실제로 검토할 사람이 없으면 잡음이 된다.
- Microsoft Learn의 `ms.date`는 **"수정했으니 올린다"가 아니라 "전면 신선도 검토를 했을 때만 올린다"**로 규정돼 있다 — 오타·편집 개선으로는 갱신 금지. **날짜의 의미를 좁혀야 신호가 산다.** — 출처: https://learn.microsoft.com/en-us/contribute/content/how-to-write-major-edits — `확실`

---

## 5. 관점 E — 발견성, 그리고 AI가 1차 독자일 때

### 통설을 뒤집는 측정

- ★**"컨텍스트 파일이 에이전트를 돕는다"는 통제 평가에서 재현되지 않았다**★ — SWE-bench + 개발자 커밋 컨텍스트 파일 양쪽에서 *"컨텍스트 파일 제공이 일반적으로 과업 성공률을 개선하지 않으면서 추론 비용을 평균 20% 넘게 올린다."* 여러 LLM·여러 에이전트·LLM생성/사람작성 모두에서 유지. — 출처: https://arxiv.org/abs/2602.11988 (Gloaguen, Mündler, Müller, Raychev, Vechev — "Evaluating AGENTS.md") — `가능성 높음` **(초록 축자 대조 완료 · 단 미동료평가 프리프린트)**
  - ★**분해가 핵심**★ — *"컨텍스트 파일 안의 지시는 코딩 에이전트가 잘 따르지만, **저장소 개요는 인기 있고 모델 제공자가 권장함에도 도움이 되지 않는다**."* 유용한 것은 **비표준 코딩 관행**의 명시.
  - ⚠️ ★**범위**★ — 이 연구가 잰 것은 **벤치마크 과업 · 항상 로드되는 컨텍스트 파일**이다. **267개 온디맨드 문서 코퍼스를 시험한 것이 아니고**, "저장소 개요는 언제나 쓸모없다"는 일반 결론도 지지하지 않는다. 우리 상황으로의 이전은 **추론이지 실증이 아니다.**
- ⚠️ **반대 방향 결과도 있다** — 10 저장소·124 PR을 AGENTS.md 유/무로 실행: 중앙값 런타임 −28.6%, 출력 토큰 −16.6%, 과업 완료는 "comparable". **표본이 훨씬 작고 지표가 효율 쪽이다.** 위 연구의 "비용 20% 초과"와 방향이 어긋나므로 **둘 중 하나를 단독 인용하면 안 된다.** — 출처: https://arxiv.org/abs/2601.20404 — `가능성 높음` → **contested로 남긴다**
- **1,925 저장소·컨텍스트 파일 2,303개 실태 조사** — 이 파일들은 정적 문서가 아니라 **"잦고 작은 추가를 통해 설정 코드처럼 진화"**하며 **"복잡하고 읽기 어려운 산출물"**이 된다. 무엇이 적히는지도 편향돼 있다 — 테스트 절차 75.9% · 구현 세부 70.8% · 아키텍처 68.1% vs 보안 14.8% · 성능 14.5%. **규칙은 사고가 난 자리에만 붙는다.** — 출처: https://arxiv.org/abs/2511.12884 — `가능성 높음`(미동료평가 프리프린트)

### 벤더 공식 지침 (전부 축자 대조 완료)

- ★**크기**★ — *"Size: target under 200 lines per CLAUDE.md file. Longer files consume more context and reduce adherence."* 하드 상한은 4 MiB(초과 시 통째로 skip). — 출처: https://code.claude.com/docs/en/memory — `확실`
- ★**`/doctor` 트림 규칙이 무엇을 자를지 지목한다**★ — *"cuts content Claude can derive from the codebase, such as **directory layouts, dependency lists, and architecture overviews**, and keeps **pitfalls, rationale, and conventions that differ from tool defaults**."* — 출처: 동일 — `확실`
- **모순 규칙은 임의로 해소된다** — *"if two rules contradict each other, Claude may pick one arbitrarily."* — `확실`
- **import는 컨텍스트를 줄이지 않는다** — 최대 4홉이고 *"Splitting into `@path` imports helps organization but doesn't reduce context, since imported files load at launch."* — `확실`
- ★**267개 코퍼스에 직접 대응하는 공식 기구 = path-scoped rules**★ — `.claude/rules/*.md`에 YAML frontmatter `paths:` glob을 달면 **매칭 파일을 읽을 때만** 로드된다. 하위 디렉터리 CLAUDE.md도 런치가 아니라 온디맨드. — `확실`
- **CLAUDE.md는 시스템 프롬프트가 아니라 그 뒤의 user 메시지로 전달된다** — *"there's no guarantee of strict compliance."* 반드시 실행돼야 하는 것은 hook으로. ★**우리 「강제」 표기가 실제로 강제가 아니라는 뜻이다.**★ — `확실`
- **Claude Code는 AGENTS.md를 읽지 않는다** — *"Claude Code reads `CLAUDE.md`, not `AGENTS.md`."* 회피책 = `@AGENTS.md` import(Windows는 심링크가 관리자 권한을 요구하므로 import 권장). — `확실`
- **여러 CLAUDE.md는 병합된다** — *"All discovered files are concatenated into context rather than overriding each other."* ⚠️ **AGENTS.md 스펙 쪽은 "가장 가까운 것이 이긴다"고 적고 병합 여부가 미확정이다**(공식 저장소 이슈 #53이 메인테이너 답변 없이 열려 있음) — **크로스툴 이식성의 실제 구멍.** — 출처: https://github.com/agentsmd/agents.md/issues/53 — `확실`
- **블록 레벨 HTML 주석은 컨텍스트 주입 전에 제거된다** — 사람 유지보수자용 메모를 토큰 없이 남기는 공식 수단. — `확실`
- ⚠️ ★**Claude 하나만 보고 agent-first 로더 설계를 일반화하면 안 된다**★ — GitHub Copilot도 **path-scoped instructions**를 지원하고 `AGENTS.md`·`CLAUDE.md`·`GEMINI.md`를 함께 읽는다. 즉 같은 파일이 도구마다 **다른 탐색·우선순위·중복 로딩** 규칙을 탄다. 우리가 로더 구조를 바꾸면 **도구별 행렬을 따로 확인해야 한다.** — 출처: https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-custom-instructions — `가능성 높음`
- **진보적 공개의 실행 규칙 셋**(스킬 저작 기준) — SKILL.md 본문 500줄 미만 · **참조는 1단계 깊이만**(중첩되면 `head -100` 부분 읽기로 잘린다) · **100줄 넘는 참조 파일엔 맨 위에 목차**. — 출처: https://platform.claude.com/docs/en/agents-and-tools/agent-skills/best-practices — `확실`
- ★**우리 「손으로 베끼지 말고 `rg`로 찾아라」 규약이 벤더 권장 패턴과 같다**★ — 스킬 저작 모범사례가 SKILL.md 안에 검색 명령을 박는 패턴을 예시로 제시한다. — 출처: 동일 — `확실`

### 컨텍스트 길이·지시 개수의 열화

- **입력 길이가 늘면 성능이 떨어진다** — 대상은 **18개 LLM**(GPT-3.5·mini/nano·오픈웨이트 포함이며 "프런티어 모델 18개"가 아니다. 실험마다 참여 모델이 다르다). needle-question 의미 유사도가 낮을수록 붕괴가 빠르고, distractor는 1개만 있어도 성능을 떨어뜨린다. — 출처: https://www.trychroma.com/research/context-rot — `가능성 높음` **(Chroma 자체 발행 vendor technical report — 실험 과업 범위로 한정해 읽을 것)**
  - ⚠️ 직관에 반하는 결과 하나 — **논리적으로 일관된 haystack보다 셔플된 haystack에서 성능이 더 좋았다.** 원 그래프를 직접 못 봐서 `불확실`, **해석 확대 금지.**
- **지시 개수 자체가 준수율을 깎는다** — IFScale(지시 10~500개, 20개 모델): 최고 성능 모델도 500개 지점에서 **68%**. **앞쪽 지시 편향** 관측. — 출처: https://arxiv.org/abs/2507.11538 — `가능성 높음`(미동료평가 프리프린트)
- ⚠️ ★**구조 변수는 유의하지 않았고, 세션 진행이 유의했다**★ — 파일 크기·지시 위치·파일 구조·인접 파일 모순 4변수 요인설계(1,650 세션·16,050 관측): *"다중검정 보정 후 네 구조 변수도, 세 이원 상호작용도 탐지 가능한 대비를 만들지 않는다."* 대신 가장 강한 효과는 **세션 내 준수율 저하 — 함수 하나 생성할 때마다 준수 odds 약 5.6% 하락.** ⚠️ 단 이 효과는 **TypeScript 코드베이스 2개·모델 3개**에서의 **사후 식별 연관이고 비단조적**이다(사전 가설이 아니다). — 출처: https://arxiv.org/abs/2605.10039 — `불확실`(단일 저자·미동료평가·사후 식별)
  - ★**이 결과는 위 "200줄" 지침과 긴장 관계다.** 벤더는 크기를 줄이라 하고, 이 실험은 크기가 유의하지 않았다고 한다. **contested로 남긴다.**

### stale 문서는 무해한 잡음이 아니라 적극적 오도다

- Python 함수 시그니처 변경 17건 실험: **stale 컨텍스트만 준 경우 구버전 헬퍼 참조가 급증**(Qwen2.5-Coder-7B 15/17, gpt-4.1-mini 13/17 — current-only 대비 각각 +88.2%p, +76.5%p). **retrieval 없음은 stale 참조 0이지만 성공도 1/17.** 현재+stale 혼합은 대부분 완화됨. — 출처: https://arxiv.org/abs/2605.14478 — `가능성 높음`(표본 17건 · 진단용 설계 · 미동료평가)
- ⚠️ ★**함의를 넓히지 말 것 — 초안의 오추론을 적대 리뷰가 적출했다**★ — 이 실험이 시험한 것은 **검색되어 컨텍스트에 들어온 낡은 코드 조각**이다. **보존-대-삭제 정책도, 검색에서 제외된 아카이브 문서도 시험하지 않았다.** 따라서 여기서 "낡은 문서를 남기는 것이 안 주는 것보다 나쁘다"를 끌어낼 수 없다.
  - **끌어낼 수 있는 것은 이것뿐** — *낡은 내용이 **검색되어 들어오면** 오도한다.* 즉 위험은 **보존 자체가 아니라 검색 오염**이고, 대응 축도 삭제가 아니라 **검색 대상에서 빼는 것**(아카이브 분리·인덱스 제외)이다. 그리고 같은 실험이 **현재+stale 혼합은 대부분 완화된다**고 보고한다 — 이것도 삭제보다 분리 쪽을 가리킨다.

### llms.txt는 채택 근거가 약하다

- 스펙 실체는 명확하다(루트 `/llms.txt`, H1 → 요약 blockquote → H2 링크 목록). — 출처: https://llmstxt.org/ — `확실`
- ⚠️ **소비 증거가 거의 없다** — Ahrefs 137,000 도메인 표본에서 게시 28%, 그중 **97%가 2026년 5월 한 달간 요청 0회**. — 출처: https://ahrefs.com/blog/what-is-llms-txt/ — `가능성 높음`(단일 벤더 측정)
- 다만 `code.claude.com/docs`는 실제로 llms.txt를 인덱스로 안내한다(이번 조사 중 직접 관측). **"게시"는 사실이고 "크롤러가 소비한다"는 별개다.**

### 고전 발견성 — 우리에게 바로 쓸 것

- **고아 문서 검출을 Sphinx는 빌드 경고로 강제한다** — toctree에 없는 문서는 경고, 의도적 고아는 `:orphan:`으로 도장. **"고아 금지"를 선언이 아니라 빌드 게이트로 만든 선례.** — 출처: https://github.com/sphinx-doc/sphinx/issues/9596 — `확실`
- Docusaurus는 **깨진 링크가 기본값 `throw`**(빌드 실패). — 출처: https://docusaurus.io/docs/api/docusaurus-config — `확실`
- lychee는 마크다운·HTML의 링크뿐 아니라 **앵커 프래그먼트까지 검증**하고 JSON 출력으로 CI에 붙는다. 단 외부 링크 검사는 flaky해서 `.lycheeignore` + 캐시가 필수. — 출처: https://github.com/lycheeverse/lychee — `확실`
- ⚠️ **순수 마크다운 + 허브 파일 구성에서 "허브에서 도달 불가한 문서"를 찾는 기성 도구는 확인하지 못했다.** Sphinx는 toctree라는 명시적 그래프가 있어 되는 것이고, Docusaurus/lychee는 *깨진* 링크를 잡지 문서 *고아*를 잡지 않는다. — `가능성 높음`(부정 결과)
- **(2019 사례연구)** Spotify는 **흩어진 README도, 단일 `docs/` 폴더도 대규모에선 실패한다**고 적고 하이브리드를 만들었다. 문서 발견 가능성이 *"Spotify 엔지니어링 생산성의 3번째로 큰 장애물"*이었다. ⚠️ **2019년 내부 상태이지 현재 일반론이 아니다.** — 출처: https://engineering.atspotify.com/2019/10/solving-documentation-for-monoliths-and-monorepos — `확실`(2019 시점 사실로서)
  - ★**교훈: 콜로케이션만으로는 안 됐고, 그 위에 별도의 "발견 층"을 지어야 했다.**★ 우리 `docs/README.md` 허브가 그 자리인데, 267개를 손으로 유지하는 색인이라는 점이 Spotify가 자동화로 푼 지점과 갈린다.
- **Every Page Is Page One**(Mark Baker, 2013)이 agent-first 프레이밍에 가장 가까운 기성 이론이다 — 전제: 어느 페이지든 독자가 처음 보는 페이지일 수 있다(검색 진입). 따라서 순서가 없고 **모든 토픽이 자기완결이어야 한다.** — 출처: https://everypageispageone.com/ — `가능성 높음`(책 미독) — **다만 2013년 저작이라 LLM을 다루지 않는다.**
- ★**RAG 벤더 지침의 가장 이식성 높은 한 줄**★ — *"제품 고유 용어가 그 청크 안에 명시되지 않으면, 정답을 담고 있어도 검색되지 않는다."* — 출처: https://docs.kapa.ai/improving/writing-best-practices — `확실`(출처 성격은 규약이지 측정이 아님)
  - ⚠️ **한국어 산문 + 영어 코드 심볼이 섞인 우리 코퍼스에서 이게 어떻게 작동하는지에 대한 근거는 하나도 못 찾았다.**

### ADR 코퍼스를 LLM이 읽었을 때의 실측

- 109 저장소·980 ADR 파일에서 1,317 결정을 수집(⚠️ 이 집합은 Proposed/WIP까지 "accepted"로 매핑해 만든 것이다). LLM이 "코드가 ADR을 위반했는가"를 판정: 최고 모델 90%+ 정확도. **단 실패의 성격이 중요하다** — ⚠️**아래 비율의 분모는 1,317이 아니라 수동 검증 305건 중 오류 92건이다**: 오류의 42%가 인프라 특정 결정, **26%가 원칙성 지침**, 17%가 다중 모듈 상호작용. 실패 원인의 **28%가 "맥락 정보 누락"**. — 출처: https://arxiv.org/html/2602.07609v1 — `가능성 높음`(미동료평가 프리프린트)
  - ★**"원칙성 지침"과 "다중 모듈 상호작용"이 LLM이 가장 못 읽는 두 종류인데, 우리 CLAUDE.md 아키텍처 원칙과 핵심 불변식이 정확히 그 두 종류다.**★

---

## 6. 우리 현황과의 대조 (관찰 — 결정 아님)

| 우리가 하는 것 | 조사 결과 | 판정 |
|---|---|---|
| `// ADR-NNNN` 코드 앵커 + `rg` 게이트 | e-ADR이 유일한 형식화 시도인데 **Java 전용이고 동반 도구는 unmaintained** | ★**공개 선례보다 앞서 있다 — 베낄 대상이 없다**★ |
| 생성물 재생성 후 `git diff --exit-code` | HashiCorp·Bazel·Microsoft 공통 패턴 | 정본 패턴 ✓ (단 마찰 회고 존재) |
| `rg` 격리 게이트·의존 상한 게이트 | 업계 어휘로 "architecture fitness function" | ★**이 축에선 이미 평균 이상**★ |
| "여기가 정본, 베끼지 마라" 경고문 | Google·Kubernetes가 같은 결론에 독립 도달. **기계화 경로 = transclusion 앵커** | 방향 ✓, 기계화 미도입 |
| ADR append-only + 폐기 도장 | 보편. **단 상태를 기계 판독 필드로 두는 곳이 여럿**(kep.yaml) | 방향 ✓, 산문에 둔 것이 갈림 |
| step-log 59개 시간순 누적 | Node.js는 **의미 단위로 분할**(연도별 아카이브는 기각됨) | 분할 미도입 |
| CLAUDE.md 240줄 / 24,000자 | 벤더 권장 **200줄 미만**. `/doctor`가 자르라는 것 = 디렉터리 레이아웃·의존성 목록·아키텍처 개요 | ★**초과 + 자르라는 범주를 다수 포함**★ |
| `docs/README.md` 손유지 허브 | Spotify는 자동화로, PEP 0는 상태별 섹션 분리로 품 | 손유지가 갈림 |
| 「강제」 표기 | CLAUDE.md는 user 메시지라 **강제가 아니다**. 강제하려면 hook | 표기와 실제가 어긋남 |

---

## 7. 쟁점 (contested — 메인이 판정하지 않는다)

1. **컨텍스트 파일의 비용 방향** — 한쪽은 +20% 비용·성공률 개선 없음(대규모), 다른 쪽은 런타임 −28.6%·토큰 −16.6%(소규모). 지표가 달라 직접 모순은 아니지만 방향이 어긋난다. **어느 쪽도 단독 인용 금지.**
2. **파일 크기가 중요한가** — 벤더는 200줄 미만을 권장하고, 요인설계 실험은 구조 변수(크기 포함)가 유의하지 않았다고 보고한다. 후자는 단일 저자 프리프린트다.
3. ★**낡은 문서를 어떻게 할 것인가**★ — **초안은 이 쟁점을 "남길 것인가 지울 것인가"로 세웠는데, 적대 리뷰가 그 틀이 틀렸다고 적출했다.** 근거가 된 실험은 *검색되어 들어온* stale 내용을 시험했지 보존-대-삭제를 시험하지 않았다.
   - **다시 세운 쟁점 = 보존은 하되 무엇을 검색 대상에 둘 것인가.** 사람 독자 문헌은 보존을 강하게 지지하고(OpenStack이 삭제를 철회한 실증), 기계 독자 실험이 경고하는 것은 **검색 오염**이다. 두 결론은 충돌하지 않는다 — **아카이브를 남기되 인덱스·검색 대상에서 빼는 것**이 양쪽을 동시에 만족시키는 후보다.
   - ⚠️ **다만 그 후보를 실제로 검증한 문헌은 못 찾았다.** 우리 판단이 필요한 자리인 것은 그대로다.
4. **"문서 일반에 append-only가 보편"이라는 초안의 전칭 주장** — Docusaurus 버전 삭제와 W3C 은퇴가 이를 반증했다. 결정 기록에 한정하면 유지되고, 문서 일반으로 넓히면 거짓이다. **본문은 한정 쪽으로 고쳤다.**

---

## 8. 한계

- **이 분야 전체에 도입 성과 측정이 없다.** Diátaxis도, 콜로케이션 전환도, 문서 리오그도 before/after 수치를 낸 곳을 찾지 못했다.
- **소규모(1~3인) 저장소의 실제 관행에 대한 근거가 없다.** 확인한 정책은 전부 대규모 조직 것이다.
- **한국어 문서 코퍼스의 LLM 검색 특성에 대한 근거가 0이다.** 우리에게 가장 실질적인 위험일 수 있는데 비어 있다.
- **"허브 하나 + 267개 문서" 구성 자체를 평가한 측정이 없다.** 있는 측정은 전부 *항상 로드되는* 컨텍스트 파일에 관한 것이고, **에이전트가 온디맨드로 찾아 읽는 문서 코퍼스의 크기·구조가 성능에 미치는 영향은 측정된 문헌을 찾지 못했다.** 우리 실제 질문이 여기 있는데 이 자리가 비어 있다.
- **ADR 100+ 코퍼스의 관리 방식을 1차 출처로 확보하지 못했다.** Kafka KIP의 supersede 실측이 후보였으나 검증 실패 — **인용하지 말 것.**
- **arXiv 프리프린트 다수의 피어리뷰 상태를 확인하지 않았다**(2602.11988 · 2605.10039 · 2601.20404 · 2606.09090 · 2605.14478 · 2511.12884).
- 인용하지 않고 버린 것: "마크다운 구조가 LLM 이해도 30% 향상" 류 수치(전부 SEO/벤더 블로그, 1차 미도달) · "200 ADRs and nobody reads them"(1차 출처 특정 실패) · AGENTS.md "60,000+ 프로젝트 채택"(자체 주장, 독립 검증 없음).

## 9. 후속이 쫓을 것

1. **Oxide RFD 1차 심화** — 공개 인덱스에서 `committed` 상태 유지율·abandoned 비율·번호가 커졌을 때의 발견성 해법. 규모·성격이 가장 가까운 선례인데 이번엔 RFD 1만 읽었고, 총량 수치는 검증 못 했다.
2. **`kep.yaml` + 구조 lint 도구 실물**(`kepctl`/CI 검사) — "분류를 지침이 아니라 기계 검사로 강제한다"의 구현체.
3. **supersede 링크 무결성 CI 검사 구현** — 174개 양방향 도장 검증의 기계화 지점.
4. **한국어 + 영어 심볼 혼합 코퍼스의 임베딩 검색 성능** — 근거 0인 실질 위험.
5. **마크다운 코퍼스용 고아 문서 검출** — 기성 도구가 없어 자체 스크립트(허브에서 링크 그래프 BFS)가 필요할 가능성.
6. **"아카이브는 남기되 검색에서 뺀다"의 실증** — 쟁점 3을 푸는 후보인데 검증한 문헌을 못 찾았다.
7. **에이전트 도구별 로딩 행렬**(Claude·Copilot·기타) — 우리가 로더 구조를 바꾸면 도구마다 탐색·우선순위·중복 규칙이 달라진다.

---

## 10. load-bearing 주장의 claim/source 범위표

적대 리뷰가 요구한 표다. **결론이 어디까지 출처에 의해 지지되고 어디부터 우리 추론인지**를 분리한다.

| # | 주장 | 출처가 실제로 시험한 것 | 우리 상황으로의 이전 |
|---|---|---|---|
| 1 | 결정 기록은 통합·삭제하지 않는다 | 선택한 **확정 결정기록** 코퍼스에서의 **절차 부재 관찰** | ✅ ADR에 한정하면 지지. ❌ 문서 일반으로 넓히면 반증됨(Docusaurus·W3C) |
| 2 | 나이 기준 삭제는 철회됐다 | OpenStack 스펙이 직접 서술(2017 설문 · Queens-era 정책) | ✅ 직접 지지. 단 **날짜를 붙여 읽을 것** |
| 3 | 컨텍스트 파일이 성공률을 안 올린다 | **벤치마크 과업 · 항상 로드되는** 컨텍스트 파일의 성공률·비용 | ⚠️ **추론** — 267개 온디맨드 코퍼스도, "저장소 개요는 언제나 무용"도 시험되지 않았다 |
| 4 | stale 문서가 오도한다 | **검색되어 컨텍스트에 들어온** 낡은 코드 조각 | ⚠️ **RAG 검색 오염 위험으로만 성립.** 보존-대-삭제 정책은 시험되지 않았다 |
| 5 | 역사 기록과 현재 진실을 분리한다 | PEP 1·Rust RFC가 **명문으로 선언** | ✅ 직접 지지 |
| 6 | 로더 구조를 바꾸면 나아진다 | Claude 문서는 **로딩 메커니즘만** 설명 | ⚠️ **미검증 설계 가설.** 효과를 잰 문헌 없음 |

---

## 11. 적대 리뷰 결과 (cross-family · 레벨 4)

**판정: `FIX`** — 반영 완료. 렌즈 4종(수치 정밀·개체 혼동·시점·출처 권위) + 누락 능동 탐침 + claim/source 함의 재검증.

**적출돼 고친 것:**
- **high** — KEP·PEP **번호대를 문서 개수처럼** 썼다 → 번호와 개수를 분리
- **high** — ADR·RFC·RFD·KEP·PEP를 **한 모집단으로 묶어 전칭 추론** → 형식별로 갈라 읽도록 수정
- **high** — 미동료평가 프리프린트에 `확실` 태그 과다 → 전부 강등 + preprint 명시
- **high** — AGENTS.md 연구의 결론을 **시험 범위 밖까지** 확장 → 범위 명시 + 추론으로 강등
- **high** — stale 연구에서 **보존-대-삭제 결론을 오추론** → 삭제하고 검색 오염으로만 기술(쟁점 3 재작성)
- **med** — Oxide 수치를 팟캐스트 구두 주장인 채 `가능성 높음`으로 → `불확실`로 강등
- **med** — ADR 위반 연구 비율의 **분모가 1,317이 아니라 305/92** → 명시
- **med** — "18개 프런티어 모델 전부" 모집단 오기 → 수정 + vendor report로 분류
- **low** — OpenStack·GitLab·Spotify 사례에 **시점 미표기** → 날짜 부착

**누락으로 지적돼 추가한 것:** Docusaurus 버전 삭제 · W3C 은퇴 모델 · IETF 이단계 수명주기 · GitHub Copilot 크로스툴 로딩.

**리뷰어가 "finding 없음"으로 통과시킨 것:** Diátaxis/Django · arc42/C4 · MADR/Nygard · GitLab 제품문서/핸드북 · docs-specs/nova-specs · `include`/`rustdoc_include` · lychee/htmltest/rust linkchecker · llms.txt/AGENTS.md/CLAUDE.md 구분. 그리고 **200줄·4 MiB·path-scoped rules·AGENTS.md 미독·import 4홉·HTML 주석 제거가 2026-08-26 현재 모두 유효함을 독립 재확인**했다.

⚠️ **리뷰어가 지적했으나 이 문서가 아직 못 채운 것:** KEP·PEP의 **실제 파일 수 실측**(API·디렉터리 기준 + 스냅샷 날짜). 지금 본문은 번호대만 말하고 개수는 말하지 않는다 — 그대로 두었다.
