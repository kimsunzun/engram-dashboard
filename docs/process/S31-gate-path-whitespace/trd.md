# S31 TRD — 소스 정규식 게이트의 공백형 경로

- 상태: 판 1 · 2026-11-02 · 정본 ADR 없음(게이트 기대값의 정본은 각 대상 crate · 파일 헤더 — 아래 「같은 주장」).

## 범위와 PRD 관계

PRD R1–R3 · AC1–AC3 을 `.github/workflows/ci.yml` 의 게이트 셋에 적용한다.

| 게이트(스텝 이름) | 지금 패턴 | 공백형 |
|---|---|---|
| `Gate: base leaf crate — tenants must not reference each other` (`ci.yml:697`) | `(crate\|super)::(logging\|text\|…\|panic)` | `crate :: time` 통과 — 고친다 |
| `Gate: replay_flight purity — no transport/wire ingress` (`ci.yml:1267`) | `^(?:[^/]\|/[^/])*?\b(tauri\|tokio\|engram_dashboard_[a-z_]+\|std::net)::` | `tokio :: spawn` 통과 — 고친다 |
| `transport gate 3: pure layer isolation (pending/stream/machine)` (`ci.yml:1344`) | 위와 같은 꼴(`tokio\|futures_util\|futures\|…`) | 통과 — 고친다 |

- 범위 밖 — `net gate 2a`(`ci.yml:1373` 추출 · `:1394` 숨은 꼴). 공백형 `engram_dashboard_platform :: fs` 는 `:1394` 의 `(?:[^:]|:[^:]|::[^A-Za-z0-9_]|…)` 에 걸려 이미 FAIL 한다. 손대지 않는다.
- 범위 밖 — 줄을 넘는 경로(`tokio` 줄바꿈 `::spawn`). 게이트가 줄 단위다(PRD 비목표와 같은 종류).

## 바꾸는 것

- `::` 앞뒤에 `\s*` 를 허용한다 — crate 이름 뒤 `\s*::`, 여러 마디 이름(`std::net`)은 마디 사이마다 `\s*::\s*`.
  - base: `(crate|super)\s*::\s*(logging|text|time|path|sync|testing|writable|file|panic)`.
  - replay_flight: `^(?:[^/]|/[^/])*?\b(tauri|tokio|engram_dashboard_[a-z_]+|std\s*::\s*net)\s*::`.
  - transport gate 3: 같은 꼴(이름 목록은 지금 그대로).
- 접두 가드 · `\b` · 파일 존재 검사 · 판정 `case` 는 그대로 둔다(R3).
- 패턴은 bash 큰따옴표 안이다 — `\s` 는 큰따옴표에서 그대로 rg 에 간다.

## 같은 주장 — 문서 밖 사본

- 패턴 · 「`\b`+`::` 가 이웃을 가른다」를 해설하는 자리를 함께 맞춘다: `ci.yml` 의 해당 스텝 주석 · `crates/engram-dashboard-base/src/lib.rs` 헤더 · `crates/engram-dashboard-transport/src/lib.rs` 헤더 「격리 게이트」 · `src-tauri/src/daemon_client/replay_flight.rs` 헤더. 코더가 옛 패턴 문자열로 `rg` 해 사용처를 먼저 세고 보고한다.

## 시험 계획

qa 가 각 게이트 스텝을 `ci.yml` 원문 그대로 돌린다(qa 바인딩 「CI 로컬 대체」).

- AC2 — 지금 트리에서 세 게이트 PASS(수정 전과 같은 판정).
- AC1 — 대상 파일 사본(`$QA_TMP` 아래, 경로만 바꿔 같은 패턴)에 한 줄씩 넣어 FAIL: `use   tokio :: time;` · `let _ = tokio :: spawn(f);` · `tokio:: x` · `tokio ::x` · base 는 `crate :: time::now()` · `super:: sync::X`. 붙은 형태(`tokio::spawn`)도 FAIL.
- AC3 — 사본에 넣어 FAIL 하지 않음: `std::time::Instant` · `// tokio :: spawn` 꼬리 주석 · `my_tokio :: x` · base 는 `crate::other_mod`.
- 수정 전 패턴으로 AC1 공백형이 PASS 하던 것도 한 번 보여 구멍이 실재했음을 남긴다.

## 구현 단위

- 코더 1 · 단위 1(파일이 겹친다). 변경은 `ci.yml` 패턴 3줄 + 해설 사본.
- 단위 끝 = `ci.yml` YAML 파싱 OK + 위 시험을 코더가 한 번 돌려 본다(최종 판정은 qa).

## 결정

- 대상 = 게이트 셋, net gate 2a 는 이미 잡으므로 제외(위 표).
- 공백 허용 = `\s*`(탭 포함). 줄 넘김은 제외.

## 위험

- 오탐 — AC2 · AC3 이 막는다.
- `jobs:` 가 바뀌어 이 커밋의 CI 결론이 master 기준선과 다르다 — qa 바인딩 「전체 회귀 = CI」 규칙대로 판단한다.

## 리뷰 기록

(PRD+TRD 리뷰 통과 시 한 줄)
