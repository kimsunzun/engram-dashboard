# 주석 컨벤션 — engram 살아있는 캐논

> **위치 잠정.** 이 문서 위치(`docs/reference/`)는 잠정이다. ADR 등 문서 프로세스가 정립되면 재조정한다(ADR-0032에 명시).
> **이 문서는 진화형 캐논이다 — 제자리 수정, ADR 아님.** 결정의 *왜*는 ADR-0032, 근거 풀셋은 `docs/research/code-commenting-conventions-research-2026-06-23.md`. 이 문서는 *실천 규약(정설)*만 담는다.

CLAUDE.md `## 컨벤션` 절이 이 캐논을 가리킨다. 주석을 새로 달거나 정리할 때 본문이 여기 있다.

## 핵심 한 줄

주석은 *줄이는* 게 아니라 **책임을 좁히는** 것이다. `what`(무엇)은 이름·타입·구조·테스트가 맡고, 주석은 `why`(의도·배경·제약)와 `load-bearing 의미`(시그니처로 안 보이는 불변식)만 진다. "한눈 이해"는 인라인 verbose가 아니라 **파일 overview 헤더(별도 계층)** 가 담당한다.

## 4층위 모델 — 어느 층위가 무엇을 지나

리서치 §3.1. 같은 정보를 두 층위에 중복하지 않는다 — 각 층위는 담을 것과 피할 것이 다르다.

| 층위 | 형식 | 담을 것 | 피할 것 |
|---|---|---|---|
| 코드 내부 local | `//`, `/* */` | 비직관 이유·invariant·sentinel·동시성/lifetime·workaround 근거 | 코드 한 줄 직역·죽은 코드·오래된 TODO |
| API doc-comment | rustdoc `///`/`//!`, JSDoc, docstring | 사용법·contract·params/return 의미·errors/panics/safety·examples·deprecation | signature와 중복되는 타입 설명·template filler |
| 설계 결정 | ADR (+ 코드 `// ADR-NNNN` 앵커) | context·decision·**거부 대안**·consequences·status·supersede | 회의록·사소한 구현 선택 |
| 사용자/에이전트 문서 | Diataxis 4유형 · CLAUDE.md | 학습/작업/조회/이해 분리 · 에이전트 불변식·금지·검증절차 | 한 문서에 입문+절차+API+철학 혼재 |

## 2계층 핵심 규약 (선택지 B — 채택)

이 캐논의 운영 규약 두 가지다(ADR-0032).

### (1) 인라인은 why/intent/invariant/load-bearing으로 좁힌다 (기존 engram 규약)

코드가 *how*를 보여주므로 주석은 *why*만 말한다. 비직관 이유·불변식·sentinel·동시성/lifetime·workaround 근거를 박고, 코드 한 줄 직역·죽은 주석·오래된 TODO는 쓰지 않는다. 일일이 다는 건 노이즈 — *의도가 섞인 지점*만 깊게 단다.

특히 **시그니처·타입만 봐선 안 보이는 load-bearing 의미**는 빠짐없이 박는다. "이 변수가 사실 테스트 격리 탈출구다", "이 분기가 어떤 race를 막는다", "이건 detached여야 한다" 같은 의미는 빠뜨리면 다음 세션이 모르고 "불필요"로 지우거나 잘못 바꾼다(실제 사례: `ENGRAM_DATA_DIR` 오삭제 — CLAUDE.md `## 컨벤션` 참조).

### (2) load-bearing 파일은 overview 헤더(`//!`) 권장 (soft — boy-scout로 점진) — 신규 계층

load-bearing 파일(동시성·kill·보안·세션복원 등 핵심 책임을 지는 파일)은 모듈 overview 헤더(Rust `//!`)를 둔다. 게이트(hard guardrail)가 아니라 **점진 권고(soft)** 다 — 그 파일을 만질 때 곁다리로 단다(boy-scout, 아래 §점진 정리 권고). 처음 보는 사람·에이전트가 1번 줄에서 파악할 것:

- **역할·책임** — 이 파일이 무엇을 지는가(여러 책임이면 나눠서).
- **핵심 불변식** — 어기면 무엇이 깨지나.
- **진입점** — 어디서부터 읽나(주요 함수·타입).
- **"시그니처로 안 보이는 load-bearing 의미"** — 자동인가 호출자 책임인가, best-effort인가 보장인가, 1회인가 멱등인가 등.

이건 사용자 "한눈에 이해" 니즈를 증거가 지지하는 형태로 충족한다 — verbose 인라인이 아니라 overview 별도 계층(리서치 §3.3). redundant 주석(noise)이 아니라 *시그니처로 안 보이는 의미*를 박는 것이라 why-not-what·load-bearing 원칙과 정합한다.

#### overview 헤더 예시 (리서치 §4 — `logging/mod.rs`, 코드 미적용 예시)

**Before:** 첫 줄 = `use std::sync::OnceLock;`. "이 파일이 무슨 책임을 지는지", 특히 **"마스킹이 자동이 아니라 호출자 책임"** 이라는 비자명 불변식이 한눈에 안 보임 → 다음 세션이 production 로그에 PTY 텍스트를 추가하며 `mask_secrets`를 빠뜨릴 위험(= `ENGRAM_DATA_DIR` 오삭제 사고와 같은 부류).

**After (이런 `//!` 헤더를 달면):**

```rust
//! logging — tracing-subscriber 전역 설정 + 로그 비밀값 마스킹.
//!
//! ## 두 책임
//! - 로그 초기화·레벨 제어: init_logging(부팅 1회, 멱등) → RELOAD_HANDLE → set_log_level.
//! - 비밀값 마스킹: mask_secrets가 API 키·Bearer 토큰을 ***로 치환(T-1).
//!
//! ## load-bearing (시그니처만 봐선 안 보이는 의미)
//! - 마스킹은 *호출자* 책임 — 자동 적용이 아니다. 이 모듈은 mask_secrets를 제공만 한다.
//!   (tracking T-1/D-6 — production PTY 로그 추가 시 필수)
//! - 마스킹은 best-effort. AWS Secret Key·generic api_key= 는 못 잡는다. "통과 = 비밀 0" 아님.
//! - 전역 상태(OnceLock)라 init·regex 컴파일은 정확히 1회. 중복 init/set은 no-op.
```

**개선점:** 처음 보는 사람이 1번 줄에서 ① 두 책임(로그 제어 / 비밀 마스킹) ② load-bearing 불변식(마스킹=호출자 책임·best-effort·전역 1회)을 즉시 파악.

## ADR 앵커 (`// ADR-NNNN`) — 점진 확대

load-bearing 코드에는 그 결정을 박은 ADR을 `// ADR-NNNN` 한 줄 앵커로 가리킨다. 앵커는 코드와 한 몸이라 인덱스 리스트처럼 rot하지 않는다(다음 세션이 `rg "ADR-"`로 만지는 코드의 결정을 찾는다). 신규·수정분부터 점진 확대한다.

## 점진 정리 권고 (boy-scout) — soft, 대량정리 금지

rot(거짓·죽은 주석)는 **대량 정리하지 않는다.** 그 파일을 만질 때 곁다리로 고친다(boy-scout rule). overview 헤더도 마찬가지 — 신규·수정 파일부터 점진 확대한다.

대량 일괄 정리를 피하는 이유: 이 캐논은 강제(hard guardrail)가 아니라 soft context이고, 리서치가 짚었듯 한 번에 훑는 대량 정리는 표본편향(작업 맥락 없이 "불필요해 보여서" 삭제) 위험이 있다. rot는 주석 *길이*가 아니라 *갱신 누락*의 함수이므로, 해법은 "짧게 써라"가 아니라 "코드 변경과 함께 갱신하라"다.

## engram 현황 — 대공사 아님

이 캐논은 새 규율 도입이 아니라 **기존 기조의 명문화**다. 대부분의 코어 파일이 이미 `//!` 헤더를 보유한다(`rg '^//!' crates/engram-dashboard-core/src`로 현황 확인). 갭은 헤더 없는 소수 파일 + ADR 앵커 부분성이며, 이마저도 boy-scout로 점진 채운다. (조사 당시 수치 스냅샷은 `docs/research/code-commenting-conventions-research-2026-06-23.md`.)

## 역링크

- **왜 이 규약인가(결정·거부 대안):** `../decisions/0032-주석컨벤션-2계층-overview헤더.md`
- **근거 풀셋(리서치):** `../research/code-commenting-conventions-research-2026-06-23.md`
- **상위 라우터:** `../../CLAUDE.md` `## 컨벤션`
