# 서버 쪽 YAML 파싱 · JSON Schema 검증 · HTTPS 가져오기 — 라이브러리 조사 (serde-saphyr · jsonschema · ureq)

- **상태:** medium · 2026-10-31 · 수집자 1(worker-scout) + 메인 grounding + codex 적대 리뷰(effort high) — 판정 FIX. TRD `docs/process/S25-preset-library/trd.md` §5-1 step 0a(구현 부분별 외부 라이브러리 조사)의 서버 갈래.
- **계기:** S25 프리셋 라이브러리 서버가 ① 프리셋 YAML 읽기 ② 외부 JSON Schema(claude · codex 설정 스키마) 검증 ③ 스키마를 HTTPS 로 가져와 갱신하는 데 쓸 crate 를 고르는 단계. TRD §1-12 는 jsonschema 0.59.0 을 박아 뒀는데 지금 판은 0.60.2.
- **확신도 범례:** 확실 = 1차 출처 + 독립 교차확증(메인 grounding 또는 codex 재확인이 일치) · 가능성 높음 = 단일 1차 출처, 반박 없음 · 불확실 = 추정/미검 · contested = 리뷰가 반증과 함께 반박.

## 결론

★리뷰(FIX)가 강제한 정정 여섯은 해당 항목에 「리뷰로 정정」으로 표시했다.★

| 필요 | 선택 | 이유 | 차점 |
|---|---|---|---|
| YAML 읽기 | **serde-saphyr 2.0.0** (`default-features = false`, `["deserialize"]`) | 오류에 줄·칸 + 발췌, 중복 키 기본 거부, 예산 상한(billion-laughs), unsafe 없음, 퍼징됨. 판 확인: 2026-10-13(메인 grounding) | saphyr 0.3.3(YAML 1.2 · 위치 있음 · serde 없음) |
| JSON Schema 검증 | **jsonschema 0.60.2** (`default-features = false`, `["resolve-file","idna"]`) | 기본 feature 가 reqwest + rustls + aws-lc 를 끌고 오는 것을 막는다. 드래프트 2020-12/2019-09/7/6/4 · `validator_for` 자동 감지 · `iter_errors` + `instance_path()` | boon 0.6.1(2025-01) |
| HTTPS 가져오기 | **ureq 3.5.0** 블로킹 (`default-features = false`, `["rustls","platform-verifier","gzip"]`) | 갱신이 tokio 가 아니라 평범한 데몬 스레드 하나에서 돌고, async 반입 게이트(⑦)를 그대로 지키며, 의존 트리가 작다 | reqwest 0.13.5 |

- **정정 1(리뷰로 정정 — ureq 인증서):** `platform-verifier` feature 만으로는 모자란다. 에이전트에 `TlsConfig::builder().root_certs(RootCerts::PlatformVerifier)` 를 **명시**해야 한다. 안 하면 Mozilla 루트가 기본으로 남는다. 확실(codex 가 docs.rs ureq 3.5.0 으로 반증 + 수집자 feature 서술과 합치).
- **정정 2(리뷰로 정정 — 포맷 검증):** 「포맷 검증은 `should_validate_formats(true)` 를 주기 전엔 꺼져 있다」는 틀렸다. 기본값이 **드래프트에 따라 갈린다.** 두 대상 스키마가 draft-07 이므로 정책(켬/끔)을 **코드에서 명시**하고, 모르는 포맷을 어떻게 다룰지는 별도 설정으로 정한다.
- **정정 3(리뷰로 정정 — 시스템 프록시):** Windows 시스템 프록시(`win-system-proxy`)는 HKCU 의 `ProxyEnable`/`ProxyServer` 만 읽어 **HTTP 프록시 URI 하나**로 만든다. PAC · WPAD · `ProxyOverride` 는 안 된다 → 「제한된 수동 프록시 발견」으로 적는다(수집자의 막연한 「미상」을 좁힘).
- **정정 4(리뷰로 정정 — 본문 크기):** ureq 본문 기본 상한 10 MiB 는 `read_to_string`·`read_to_vec`·`read_json` 에만 걸리고 reader API 는 **무제한**이다. 쓸 읽기 API 를 이름으로 정하고 스키마 크기 상한(클레임 스키마 약 231 KB 기준)을 **명시적으로** 둔다.
- **정정 5(리뷰로 정정 — reqwest):** reqwest 에도 블로킹 클라이언트(`reqwest::blocking`)와 `rustls-no-provider` + ring 조합이 있다. 「async 라서 탈락」은 과했다. ureq 가 이기는 근거는 **의존 수와 런타임 불필요**로 좁힌다.
- **정정 6(리뷰로 정정 — 누락 후보):** `yaml_serde` 가 비교에서 빠져 있었다. 실제 비교는 되지 않았다 → 아래 「공백」에 「비교 필요」로 올린다(판정 없음).
- **전제(메인 grounding 이 지지):** 이 저장소 `Cargo.lock` 에는 rustls · native-tls · schannel · ring · aws-lc · webpki-roots · ureq · YAML crate · jsonschema 가 **하나도 없다.** 어떤 HTTPS 클라이언트를 고르든 새 TLS 스택이 들어온다.

## 1. YAML 읽기

| 이름 | 판 · 날짜 | 라이선스 | 잠금 파일에? | 맞음 | 확신도 | 출처 |
|---|---|---|---|---|---|---|
| **serde-saphyr** | 2.0.0 · 2026-10-13 | 입력에 없음 | 없음 | 선택. 오류 줄·칸 + 발췌 · `Spanned<T>`(값마다 위치) · 중복 키 기본 거부 · 예산 상한 · unsafe 없음 · 퍼징. 파서 = granit-parser(saphyr 포크). MSRV 1.89 | 확실(crates.io 판 = 메인 grounding · MSRV · deserialize 전용 구성 · 중복 키 거부 · strict boolean = codex 재확인) | crates.io · github bourumir-wyngs/serde-saphyr |
| saphyr | 0.3.3 | 입력에 없음 | 없음 | 차점. YAML 1.2 · 위치 있음 · serde 없음 | 가능성 높음 | crates.io |
| yaml_serde | 0.10.7(리뷰가 제시) | MIT/Apache | 없음 | **미비교** — YAML 조직이 관리하는 serde_yaml 포크, MSRV 1.82, 파서 libyaml-rs | 불확실(수집자 미평가 · 리뷰의 서술만) | docs.rs yaml_serde · github yaml/yaml-serde `Cargo.toml` |
| serde_yaml | 0.9.34+deprecated | — | 없음 | 탈락 — 보관됨(dtolnay/serde-yaml archived = true, 메인 grounding 이 지지) | 확실 | github |
| serde_yml | 0.0.13 | — | 없음 | 탈락 — 보관됨 · noyalib 로 가는 deprecated 껍데기. RUSTSEC-2025-0068(unsound, ≤0.0.12) | 확실(RustSec) | rustsec RUSTSEC-2025-0068 |
| noyalib | 0.0.58 | 미확인 | 없음 | 탈락 — serde_yml 과 같은 저자, 0.0.x | 가능성 높음 | crates.io |
| serde_norway | 0.9.42 | — | 없음 | 탈락 — 갱신 정체 | 가능성 높음 | crates.io |
| saphyr-serde | 0.0.0 | — | 없음 | 탈락 — 자리표시자 | 가능성 높음 | crates.io |
| yaml-rust2 | 0.13.0 | — | 없음 | 탈락 — serde 없음 | 가능성 높음 | crates.io |
| marked-yaml | 0.8.0 | — | 없음 | 탈락 — 활동 적음 | 가능성 높음 | crates.io |

주의(serde-saphyr):

- **YAML 1.1 꼴:** 대상이 bool 이면 `yes/no/on/off` 가 bool 로 읽힌다. `Options { strict_booleans: true }` 권장(strict boolean 동작은 codex 재확인).
- **`Spanned<T>` 한계:** untagged · flatten 안에서는 위치가 사라진다. M14 의 kind 판별을 untagged enum 으로 구현하면 오류 위치가 안 나온다(가능성 높음 — 실측 전).
- **기능 구성:** `comments` 기본 on. 읽기 쪽은 `default-features = false` + `["deserialize"]`.

## 2. JSON Schema 검증

| 이름 | 판 · 날짜 | 라이선스 | 잠금 파일에? | 맞음 | 확신도 | 출처 |
|---|---|---|---|---|---|---|
| **jsonschema** | 0.60.2 · 2026-10-26 | 입력에 없음 | 없음 | 선택. 드래프트 2020-12/2019-09/7/6/4 · 자동 감지 · `iter_errors` · `instance_path()`. feature `resolve-file`·`idna`·`resolve-http`·`tls-aws-lc-rs`·`tls-ring` 존재 | 확실(판 = 메인 grounding · feature 이름과 기본값 = codex 재확인) | docs.rs jsonschema 0.60.2 · github Stranger6667/jsonschema |
| boon | 0.6.1 · 2025-01 | 입력에 없음 | 없음 | 차점 | 가능성 높음 | crates.io |
| valico | 4.0.0 | — | 없음 | 탈락 — 낡음 | 가능성 높음 | crates.io |

실제 스키마 관련:

- **claude 설정 스키마**(schemastore `claude-code-settings.json`): `$schema` 는 draft-07인데 정의는 `$defs`(2019-09 키워드)에 있고 참조는 `#/$defs/...`, `if` 5개. 0.59 에서 스파이크 통과 → 0.60.2 에서 같은 시험을 **테스트로 고정**해야 한다. 확실(draft-07 `$schema` = 메인 grounding · 나머지 = 수집자 확인).
- **서버 응답:** ETag · Last-Modified 를 주고 `Cache-Control: max-age=600` → 조건부 GET 가능. 약 231 KB. `json.schemastore.org` → `www.schemastore.org` 리다이렉트. 가능성 높음(수집자 curl).
- **codex 쪽:** main 의 `core/config.schema.json` 은 draft-07 이고 `definitions` 를 쓴다. 릴리스 자산 경로는 **미확인.**
- **TOML 파싱:** 이 저장소에 이미 있는 toml 1.1.2 · toml_edit 0.25 를 재사용(새 crate 없음).
- **TRD 표류:** §1-12 의 0.59.0 → 0.60.2 로 고쳐야 한다.

## 3. HTTPS 가져오기

| 이름 | 판 · 날짜 | 라이선스 | 잠금 파일에? | 맞음 | 확신도 | 출처 |
|---|---|---|---|---|---|---|
| **ureq** | 3.5.0 · 2026-10-21 | 입력에 없음 | 없음 | 선택. 블로킹 · rustls(ring — C 컴파일러만, NASM/CMake 불필요) · 시스템 프록시 환경변수 지원. Windows 시스템 프록시는 선택 의존 `win-system-proxy`(winreg) | 확실(판 = 메인 grounding · 블로킹 · ring · 선택 winreg = codex 재확인). `win-system-proxy` 가 4g 게이트(직접 의존만 셈)에 안 걸린다는 점 = 가능성 높음 | docs.rs ureq 3.5.0 · aws-lc-rs windows requirements · rustls#1913 |
| reqwest | 0.13.5 | 입력에 없음 | 0.13.4 가 셸 그래프에(TLS 없음), 0.12.28 은 데몬 dev 전용(TLS 없음) | 차점 — async. 기본 TLS = rustls + aws-lc-rs(Windows 에서 NASM 필요, 미리 빌드가 있으면 예외) · 시스템 프록시 기본 on. 블로킹 클라이언트도 있음(정정 5). tokio 위에서 돌거나 데몬 HTTPS 가 더 늘 때 | 가능성 높음 | crates.io · docs.rs |
| native-tls · schannel | — | — | 없음 | 비권장 — 플랫폼마다 갈림 | 가능성 높음 | 수집자 판단 |

설정 지침:

- **인증서:** 정정 1 대로 `RootCerts::PlatformVerifier` 를 명시(Windows 인증서 저장소 — 기업 프록시 대응).
- **시간 제한:** 명시적으로 설정한다.
- **프록시:** 환경변수는 따른다. Windows 시스템 프록시는 정정 3 의 한계가 있다.
- **조건부 GET:** 직접 구현(`If-None-Match` · 304). 304 처리 동작은 **미확인.**
- **본문 크기:** 정정 4 — 읽기 API 를 정하고 상한을 둔다.

## 적대 리뷰 결과 (codex · effort high · FIX)

| # | 등급 | 지적 | 처리 |
|---|---|---|---|
| 1 | MED | ureq `platform-verifier` feature 만으론 부족 — `RootCerts::PlatformVerifier` 명시 필요 | 반영(정정 1) |
| 2 | MED | 포맷 검증 기본값이 드래프트에 따라 다름 | 반영(정정 2) — 정책 명시를 구현 요건으로 |
| 3 | MED(누락) | `yaml_serde` 미비교 | **남김** — 입력에 비교 결과가 없어 판정 없이 공백으로 올림 |
| 4 | MED | win-system-proxy 는 HKCU 수동 프록시 하나만 | 반영(정정 3) — ureq 3.5.0 `src/proxy.rs` 235–255 |
| 5 | LOW | reqwest 에도 블로킹 클라이언트 · `rustls-no-provider` + ring | 반영(정정 5) |
| 6 | LOW | ureq 본문 상한은 일부 API 에만 | 반영(정정 4) — `src/body/mod.rs` |

재확인으로 확인된 것: serde-saphyr MSRV 1.89 · deserialize 전용 구성 · 중복 키 거부 · strict boolean; jsonschema 0.60.2 feature 이름과 기본값; ureq 블로킹 · ring · 선택 winreg.

## 공백 · 실측 필요

- **yaml_serde 비교 필요**(0.10.7 · MIT/Apache · MSRV 1.82 · 파서 libyaml-rs — 리뷰가 서술한 것뿐이다). 판정 없음 — serde-saphyr 와 오류 위치 · 중복 키 · bool 처리 · 의존 크기 · 유지 상태를 비교해야 한다.
- jsonschema 0.60.2 에서 claude 스키마(draft-07 + `$defs` 혼용) 통과 시험 — 아직 안 돌렸다.
- codex 릴리스 자산 스키마 경로 — 미확인.
- ureq 304 응답 처리 동작 — 미확인. 본문 상한 API 선택 후 크기 초과 동작 시험.
- Windows 에서 `win-system-proxy` 의 실제 동작 · PAC/WPAD 환경은 지원 안 됨(정정 3)이 실사용에 걸리는지.
- Spanned vs M14 untagged 파싱 — 위치가 실제로 사라지는지 구현 파일럿에서 측정.
- noyalib 라이선스 — 미확인(탈락 후보라 우선순위 낮음).
- 각 선택 crate 의 라이선스(serde-saphyr · jsonschema · ureq)는 입력에 없다 — 도입 전 `Cargo.toml` · LICENSE 로 확인.
- 저장소 상태: serde_json 1.0.150 · tokio 1.52.3 · hyper 1.10.1 · tungstenite 0.26.2 · toml 1.1.2 + toml_edit 0.25 · walkdir 2.5.0 이미 있음. 툴체인 1.95.0 (serde-saphyr MSRV 1.89 충족).

## 출처

- crates.io: serde-saphyr 2.0.0 · jsonschema 0.60.2 · ureq 3.5.0 (판 · 날짜 = 메인 grounding)
- github: bourumir-wyngs/serde-saphyr · Stranger6667/jsonschema · dtolnay/serde-yaml(archived) · yaml/yaml-serde(`Cargo.toml`)
- rustsec RUSTSEC-2025-0068
- docs.rs: jsonschema 0.60.2(`ValidationOptions::should_validate_formats`) · ureq 3.5.0(`#platform-verifier`) · yaml_serde
- ureq 3.5.0 소스: `src/proxy.rs` 235–255 · `src/body/mod.rs`
- schemastore `claude-code-settings.json`
- aws-lc-rs windows requirements · rustls#1913
