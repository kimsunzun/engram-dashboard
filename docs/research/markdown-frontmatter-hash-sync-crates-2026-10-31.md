# 마크다운 발췌 · SKILL.md 프런트매터 · rev 해시 · 게이트/휴지통 잠금 — 서버 쪽 외부 crate 선택 (조사 · medium)

- **상태:** medium · 2026-10-31 · 수집자 1 + 메인 grounding + codex 적대 리뷰(effort high) — 판정 FIX. TRD `docs/process/S25-preset-library/trd.md` §5-1 step 0a(구현 부분별 외부 라이브러리 조사) 중 서버 몫.
- **계기:** 프리셋 라이브러리 서버가 ① 마크다운 본문 발췌 ② SKILL.md 프런트매터 검사(검사기가 codex 와 맞아야 함) ③ 첨부 목록·파일의 rev 해시 ④ 스캔 게이트·휴지통 잠금을 만든다. 각각 crate 를 쓸지 손으로 짤지 정해야 한다. TRD §1-12 는 YAML 파서로 serde-saphyr 2.0.0 을 이미 골랐고 rev 해시는 「미정」이었다.
- **확신도 범례:** 확실 = 1차 출처 + 독립 교차확증(메인 grounding 또는 codex 재확인이 일치) · 가능성 높음 = 단일 1차 출처, 반박 없음 · 불확실 = 추정/미검 · contested = 리뷰가 반증과 함께 반박.

## 결론

| 필요 | 고름 | 이유 | 차선 |
|---|---|---|---|
| 마크다운 발췌 | **pulldown-cmark 0.13.6** (2026-10-30 · MIT · 잠금 파일에 없음 = 신규) | 이벤트 스트림이라 코드·표·HTML 을 건너뛰기 쉽다 | comrak 0.59.0 (BSD-2 · 더 무겁다). markdown-rs 1.0.0 은 느림 |
| 프런트매터 분리 | **손으로 짠 분리(~20줄, codex `extract_frontmatter` 규칙 그대로) + serde-saphyr 파싱** | codex 규칙과 달라지면 codex 가 읽는 스킬을 막거나 못 읽는 걸 통과시킨다 | 없음. gray_matter 0.3.2(yaml-rust2 끌고 옴 · 규칙 ≠ codex) · yaml-front-matter 0.1.0(2021 · serde_yaml) 기각 |
| rev 해시 | **sha2 `"0.10"` (SHA-256)** — 0.10.9 | digest · block-buffer · cpufeatures 까지 이미 잠금 파일에 있어 신규 패키지 0. 출력이 규격으로 고정 | sha1 0.10.6(데몬 그래프에 이미 있음 · tungstenite 경유 — 보안용이 아닌 토큰이면 완전 0 신규). xxhash-rust 0.8.19(BSL-1.0 · 신규) |
| 게이트 · 휴지통 잠금 | **std `Mutex` + `Condvar` 손수(각 ~30–50줄)** | 티켓 카운터가 FIFO 를 준다. 「스캔은 단독, 편집은 동시」 같은 혼합 조건은 락 공정성으로 표현이 안 된다. `notify_all` 사용 | parking_lot(`unlock_fair`). 아래 정정 참고 |

★리뷰가 강제한 정정★ (아래 본문은 정정판이다)

1. **codex 수선(repair)은 콜론 경우만이 아니라 전부 이식한다.** 수집 단계 권고는 「최소 콜론 경우」였다. 리뷰가 알 수 없는 필드의 깨진 flow 꼴 값(`tags: [next,@supabase/ssr]`)도 수선 대상임을 짚어 「전부 이식」으로 정정. 규칙은 아래 별도 절.
2. **pulldown-cmark 발췌는 메타데이터 블록을 명시적으로 건너뛴다.** `ENABLE_YAML_STYLE_METADATA_BLOCKS` 는 `---` 를 떨구는 게 아니라 텍스트를 품은 `MetadataBlock` 이벤트를 낸다. 수집 단계의 「`---` 를 버린다」는 서술을 이 점에서 고친다. 표 이벤트는 `ENABLE_TABLES` 를 켜야 나온다.
3. **parking_lot 0.13.0 은 자동 최종 공정성을 걷었고 MSRV 가 1.95 로 올랐다**(`unlock_fair` 는 남음). 0.13 을 후보로 말할 땐 이 둘을 같이 적는다.
4. 기존 조사 문서(`skill-md-recognition-claude-codex-2026-10-20.md`)가 이 둘을 빠뜨렸다는 수집자 서술은 **절반만 맞았다** — 콜론 수선과 BOM 거절은 이미 적혀 있고(40행), 빠진 건 수선 알고리즘 전체와 BOM 이 거절되는 메커니즘뿐이다.

## 1. 마크다운 발췌

| 이름 | 판 · 날짜 · 라이선스 | 잠금 파일에? | 맞음 | 확신도 | 출처 |
|---|---|---|---|---|---|
| pulldown-cmark | 0.13.6 · 2026-10-30 · MIT | 없음(신규) | 이벤트 스트림 · 코드/표/HTML 건너뛰기 · 메타데이터 블록 옵션 | 가능성 높음(crates.io + 리뷰가 v0.13.6 소스에서 옵션 동작 확인) | crates.io API · 리뷰의 `parse.rs` L2203 · `lib.rs` L209 |
| comrak | 0.59.0 · BSD-2 | 미확인 | AST 기반 · 더 무겁다 | 가능성 높음 | crates.io API |
| markdown-rs | 1.0.0 | 미확인 | 느리다 | 불확실(수집자 단독 서술) | crates.io API |

발췌 구현 주의(리뷰 반영):

- `ENABLE_YAML_STYLE_METADATA_BLOCKS` 를 켜면 프런트매터 구간이 사라지는 게 아니라 `MetadataBlock` 시작/끝 이벤트 사이에 `Text` 이벤트로 나온다. **그 하위 트리를 발췌 쪽에서 명시적으로 건너뛰어야** 프런트매터 YAML 이 발췌에 새지 않는다.
- 표 이벤트에 기대려면 `ENABLE_TABLES` 를 먼저 켠다(꺼져 있으면 표는 일반 문단으로 읽힌다).

## 2. SKILL.md 프런트매터 — codex 와 맞아야 하는 규칙

검사기는 codex 가 스킬을 읽는 규칙과 **같아야** 한다. 어긋나면 codex 가 로드하는 스킬을 우리 검사기가 막는다. 근거 = codex `codex-rs/skills/src/parser.rs`(L40–206) @ rust-v0.156.1 — 메인 grounding 이 `repair_frontmatter_scalar_fields`(L52–57 · L98 재시도)와 `extract_frontmatter`/`MissingFrontmatter` 를 대조해 지지. 리뷰가 최신 안정판 0.166.0(2026-10-29)의 `parser.rs` 가 0.156.1 과 **동일**함을 재확인했다. 두 판 모두 serde_yaml 0.9.34+deprecated 를 잠근다. **확신도 = 확실**(소스 + 메인 + 리뷰 일치).

### 2-1. 구분선 분리 (`extract_frontmatter`)

- `lines()` 로 줄을 쪼갠다(CRLF 안전).
- 첫 줄이 `trim() == "---"` 여야 시작. 다음에 `trim() == "---"` 인 줄까지가 프런트매터.
- 구분선이 없거나 줄이 0개면 `MissingFrontmatter`. **공백뿐인 블록은 MissingFrontmatter 가 아니라 YAML 단계로 넘어간다**(리뷰가 바로잡음 — 수집 단계는 「empty → Missing」으로 뭉뚱그렸다).
- **BOM 은 의도된 규칙이 아니라 `trim()` 의 부작용으로 거절된다** — `trim()` 이 U+FEFF 를 벗기지 않아 첫 줄이 `---` 로 안 보인다. 그러니 **검사 전에 BOM 을 벗기지 말 것**(벗기면 codex 가 거절하는 파일을 통과시킨다).

### 2-2. YAML 파싱과 수선(repair)

1. `serde_yaml` 로 `{name, description, metadata.short-description}` 에 역직렬화한다. 이 단계가 실패하면 **한 번만** 수선 패스를 거쳐 다시 시도한다.
2. 수선 대상(**알려진 필드만이 아니라 모든 최상위 스칼라 필드** — 리뷰의 정정):
   - 따옴표·블록 스칼라가 아닌 **무따옴표 스칼라**가 **콜론 + 임의의 공백**(`: ` 만이 아니라 콜론 뒤 아무 공백)을 포함하는 값 — 예: `description: Build for AWS: ECS`.
   - 값이 `[` `{` `@` `` ` `` 로 시작해 **flow 로 잘못 읽히는** 값 — 알 수 없는 필드도 포함. 리뷰가 든 사례: `tags: [next,@supabase/ssr]`.
3. 수선 방식: 값 전체를 **작은따옴표로 감싼다**. 값 안의 작은따옴표는 **두 개로 겹친다**(`'` → `''`). **뒤따르는 `#` 주석은 보존**한다. 블록 스칼라(`|` `>`)와 이미 따옴표 친 값은 **건드리지 않는다**.
4. 수선 뒤에도 파싱이 실패하면 **원래 오류**를 돌려준다(수선본의 오류가 아니다). 이때 `InvalidYaml`.
5. 우리 쪽 함의: serde-saphyr 로 엄격 검사만 하고 수선을 안 이식하면 **codex 가 로드하는 스킬을 잘못 막는다.** 수선 전부를 이식하고, 이식이 맞는지는 upstream 고정 시험(`codex-rs/skills/src/parser_tests.rs` L56–85 @ rust-v0.156.1)과 두 파서를 나란히 돌려 비교한다.

### 2-3. 역직렬화 이후 검사 (수선을 일으키지 않는다)

- **name:** 공백을 접는다(연속 공백 → 하나). 비었거나 없으면 **폴더 이름**으로 대체(조사 문서 기준 — 정션을 따라간 뒤의 실제 폴더). 최종 이름의 `chars().count() > 64` 면 오류(Unicode 스칼라 수 — 리뷰가 core 규칙으로 재확인).
- **description:** 공백을 접는다. 없거나 비었으면 오류. 길이 상한 없음(리뷰 재확인).
- `metadata.short-description` 이 공백뿐이면 없는 것으로 본다(리뷰 재확인).
- 위 두 검사(description 누락 · 이름 길이)는 **역직렬화 뒤에** 일어나므로 수선을 트리거하지 않는다.
- claude 쪽은 이 모두를 받아들인다(수집자 서술 · 기존 조사 문서가 정본).

### 2-4. 후보 비교

| 이름 | 판 · 날짜 · 라이선스 | 잠금 파일에? | 맞음 | 확신도 | 출처 |
|---|---|---|---|---|---|
| 손짜 분리 + serde-saphyr | serde-saphyr 2.0.0(TRD §1-12 가 이미 고름) | 별도(TRD) | codex 규칙을 그대로 따라 쓸 수 있다 | 가능성 높음 — 분리 쪽은 소스 확실, **파서 간 가장자리 차이는 미검** | codex `parser.rs` |
| gray_matter | 0.3.2 | 아니오(yaml-rust2 끌고 옴) | 규칙 ≠ codex | 가능성 높음 | crates.io API |
| yaml-front-matter | 0.1.0 · 2021 | 아니오 | serde_yaml 의존 · 오래됨 | 가능성 높음 | crates.io API |

## 3. rev 해시

| 이름 | 판 · 날짜 · 라이선스 | 잠금 파일에? | 맞음 | 확신도 | 출처 |
|---|---|---|---|---|---|
| **sha2** | 0.10.9 (`"0.10"` 고정) | 있음(+ digest · block-buffer · cpufeatures — 신규 0). **데몬 바이너리엔 새로 들어옴**(지금은 tauri/wry 경유) | 출력이 규격으로 고정 · 안정 | 확실(메인이 Cargo.lock 으로 확인 + 리뷰가 데몬 그래프 부재 재확인) | Cargo.lock · crates.io |
| sha1 | 0.10.6 | **데몬 그래프에 이미 있음**(tungstenite 경유) | 보안용이 아닌 토큰이면 신규 완전 0 | 확실(리뷰 재확인) | 〃 |
| xxhash-rust | 0.8.19 · BSL-1.0 | 아니오(신규) | 빠름 · 출력 안정 | 가능성 높음 | crates.io API |
| `DefaultHasher` · rustc-hash · fxhash | — | — | 기각: `DefaultHasher` 는 판 사이 불안정 · rustc-hash 2.0 은 알고리즘이 바뀌었고 `usize` 의존 · fxhash 는 2017 · 플랫폼 의존 | 가능성 높음(수집자 서술 · 미검) | 수집자 |

함정(수집자 서술 · 구현 설계 지침):

- **원시 바이트만 해시한다.** `std::hash::Hash` 를 거치지 않는다(구현 · 판에 따라 달라짐).
- **첨부 목록의 rev 는 정규 인코딩이 필요하다** — 길이 접두 UTF-8 + 고정 LE u64 + 기본 프리셋 슬롯 표식. 이게 없으면 `["ab","c"]` 와 `["a","bc"]` 가 충돌한다.

## 4. 게이트 · 휴지통 잠금

| 이름 | 판 · 날짜 · 라이선스 | 잠금 파일에? | 맞음 | 확신도 | 출처 |
|---|---|---|---|---|---|
| **std Mutex + Condvar 손짜기** | std | — | 티켓 카운터가 mutex 의 새치기와 무관하게 FIFO 를 준다 · 조건을 자유롭게 짤 수 있다(스캔 단독 · 편집 동시) | 가능성 높음(설계 판단 — 구현 시험에서 확인) | 수집자 |
| parking_lot | lock 0.12.5(tauri 경유) · 최신 0.13.0(2026-10-21) | 0.12.5 만. **데몬 그래프에는 없음**(신규) | `unlock_fair` 로 넘겨주기. **정정:** 0.13.0 은 자동 최종 공정성을 걷었고 MSRV 가 1.95 로 올랐다(`unlock_fair` 는 남음) | 확실(CHANGELOG 0.13.0 + 리뷰) | CHANGELOG 0.13.0 |
| spin::TicketMutex | — | — | 기각: 스핀한다 | 가능성 높음 | 수집자 |
| `ticket_lock` crate | — | — | crates.io 404 — 존재하지 않음 | 확실 | crates.io API |

공정성 근거:

- Windows(non-win7)에서 std `Mutex` 는 SRWLOCK 이 아니라 **futex(WaitOnAddress)** 다 — 넘겨주기(handoff)가 없고 새치기가 일어난다. 확신도 = **확실**(rust master 소스 + 리뷰가 rust 1.95.0 태그에서 재확인).
- 그래서 락 자체의 공정성을 믿지 않고 **티켓 카운터로 FIFO 를 직접 만든다**. 「스캔은 배타, 편집은 동시」 같은 혼합 조건은 어차피 락 공정성으로 표현이 안 된다. 깨우기는 `notify_all`(조건이 서로 다른 대기자라 하나만 깨우면 잘못된 쪽이 깰 수 있음).

## 적대 리뷰 결과 (codex · effort high · 판정 FIX)

| # | 심각도 | 지적 | 처리 |
|---|---|---|---|
| 1 | MED | 수선은 콜론 경우만이 아니라 알 수 없는 필드의 깨진 flow 값(`tags: [next,@supabase/ssr]`)도 고친다. 전체 이식 + upstream 시험(`parser_tests.rs` L56–85)과 두 파서 대조 필요 | **반영** — 2-2 에 전체 규칙을 적고 「전체 이식」으로 정정. 대조 시험은 아래 실측 목록에 |
| 2 | MED | pulldown-cmark 메타데이터 옵션은 텍스트를 품은 `MetadataBlock` 이벤트를 낸다. 하위 트리를 명시적으로 건너뛰어야 하고, 표는 `ENABLE_TABLES` 를 켜야 한다 | **반영** — 1절 구현 주의 |
| 3 | MED | parking_lot 0.13.0 이 자동 최종 공정성을 걷었고 MSRV 1.95 로 올렸다(`unlock_fair` 는 유지) | **반영** — 4절 표 + 결론 정정 3 |
| 4 | LOW | 수선은 콜론 + 임의 공백을 감지 · 뒤 주석 보존 · 작은따옴표 두 겹 · 수선 실패 시 원래 오류 반환. MissingFrontmatter = 구분선 없음 또는 0줄(공백뿐인 블록은 YAML 로 넘어감). description 누락 · 길이 검사는 역직렬화 뒤라 수선을 안 부른다 | **반영** — 2-1 ~ 2-3 |
| 5 | LOW | 기존 조사 문서(`…2026-10-20.md:40`)는 콜론 수선과 BOM 거절을 이미 적었다 — 빠진 건 수선 알고리즘 전체와 BOM 메커니즘뿐 | **반영** — 결론 정정 4. 수집자의 「문서가 빠뜨렸다」 서술을 고침. 그 문서 보강은 **남김**(이 보고서가 고치지 않는다 · TRD §1-3 에 한 줄 올릴지는 메인 몫) |

재확인(리뷰): codex 최신 안정판 0.166.0(2026-10-29) `parser.rs` = 0.156.1 과 동일 · 두 판 모두 serde_yaml 0.9.34+deprecated · 핵심 규칙(이름 ≤ 64 Unicode 스칼라 · description 길이 상한 없음 · 빈 short-description 은 없음 취급) 확인 · sha2 0.10.9 잠금 파일에 있음 · sha1 0.10.6 이 데몬 그래프(tungstenite 경유)에 있음 · sha2 · parking_lot 은 데몬 그래프에 없음 · Windows non-win7 std Mutex 가 futex/WaitOnAddress + 새치기임을 rust 1.95.0 태그에서 확인.

## 공백 · 실측 필요 (구현 파일럿)

- **serde-saphyr 와 serde_yaml 의 가장자리 차이 미검** — 중복 키 · `description: 123` 같은 형 강제 · 탭 · 앵커 · YAML 1.1 대 1.2. 기존 조사 문서의 26개 고정 시험 + 수선 사례 + upstream `parser_tests.rs` L56–85 로 **두 파서를 나란히 돌리는 골든 시험**을 만든다.
- codex 가 쓰는 serde_yaml 의 정확한 판은 잠금(0.9.34+deprecated) 까지만 확인 — 다른 판과의 동작 차이는 미검.
- 데몬에 sha2 를 새로 싣는 비용(바이너리 크기 · 빌드)은 미측정. sha1 로 대체해 완전 0 신규로 갈지는 rev 가 보안 용도가 아니라는 전제에 달렸다 — 결정 필요.
- 손짜 티켓 게이트의 FIFO · 「스캔 단독 / 편집 동시」 동작 — 시험으로 증명한다(수집자 설계 판단).
- 제안 크기 「~20줄 · ~30–50줄」 은 수집자 추정치다.

## 출처

- https://github.com/openai/codex/blob/rust-v0.156.1/codex-rs/skills/src/parser.rs (L40–206 · 수선 L52–57 · L98) · 같은 판 `codex-rs/skills/src/parser_tests.rs` L56–85 · 0.166.0 `parser.rs` 동일 확인(리뷰)
- https://github.com/rust-lang/rust/blob/master/library/std/src/sys/sync/mutex/mod.rs (· rust 1.95.0 태그 재확인)
- pulldown-cmark v0.13.6 `parse.rs` L2203 · `lib.rs` L209 (리뷰)
- parking_lot CHANGELOG 0.13.0 (2026-10-21)
- crates.io API — pulldown-cmark · comrak · markdown-rs · gray_matter · yaml-front-matter · sha2 · sha1 · xxhash-rust · parking_lot · ticket_lock(404)
- 저장소 `Cargo.lock`(sha2 0.10.9 · sha1 0.10.6) · `docs/research/skill-md-recognition-claude-codex-2026-10-20.md`(:40) · TRD `docs/process/S25-preset-library/trd.md` §1-12 · §5-1
