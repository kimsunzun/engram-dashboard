# S31 PRD — 소스 정규식 게이트가 공백 낀 경로(`tokio :: time`)를 놓치는 구멍

## 1. 무엇 · 왜

- CI 의 소스 정규식 게이트 몇 개가 crate 경로를 `\b(<이름>)::` 로 찾는다. Rust 는 경로 마디 사이에 공백을 허용하므로 `use   tokio :: time;` · `tokio :: spawn(…)` 은 컴파일되면서 이 정규식에 안 걸린다.
- 보통은 `cargo fmt --check`(ci.yml `gates` 잡)가 먼저 막는다 — rustfmt 가 공백형을 `tokio::time` 으로 고쳐 쓰므로 공백형 그대로는 그 스텝에서 FAIL 한다.
- 남는 구멍 = rustfmt 가 손대지 않는 자리 — 중괄호 매크로 호출의 본문(`m! { tokio :: spawn(f) }` — 단순한 `macro_rules!` 본문은 rustfmt 가 고쳐 쓴다)과 `#[rustfmt::skip]` 구간. 그 자리의 공백형은 fmt 게이트도, 정규식 게이트도 통과한다. 대상 게이트 중 일부는 컴파일러 backstop 이 없어 정규식이 유일한 벽이다(ci.yml 주석).
- 출처 = `docs/todo/doc-drift.md` 「게이트 정규식이 못 닫은 구멍」.

## 2. 범위 / 비목표

- 범위 = `.github/workflows/ci.yml` 의 소스 정규식 검사 중 경로(crate 경로 · `std::os` · 도구 attribute `rustfmt::skip` · `clippy::restriction`)를 `::` 로 찾는 것. 목록은 TRD 가 확정한다.
- 비목표 — 블록 주석(`/* */`) 안 오탐 · 줄을 넘는 경로 · 공백이 아닌 다른 이름 꼴(`use tokio as rt;` · `use std::{net::TcpStream, time::Instant};` · `use std::net;` — 새 할 일로 돌린다) · rustfmt 설정 변경 · 게이트를 컴파일러 검사로 바꾸는 것.

## 3. 요구사항

- R1 — 대상 검사가 `::` 앞 · 뒤 · 여러 마디 사이의 공백형(`tokio :: time` · `tokio ::time` · `std :: net :: X` · `super:: sync` · 탭 포함)을 붙은 형태와 똑같이 잡는다.
- R2 — 지금 트리에서 대상 게이트의 판정이 바뀌지 않는다(오탐 0).
- R3 — 각 게이트의 기존 성질(주석 밖만 · `\b` 경계 · 정당한 이웃 `std::time` 통과 · 파일 존재 검사)을 유지한다.

## 4. 수용 기준

- AC1 — 대상 게이트마다, 대상 파일 사본에 공백형 위반 한 줄을 넣으면 FAIL, 붙은 형태 위반도 여전히 FAIL(실행 확인).
- AC2 — 지금 트리에 대상 게이트를 돌리면 수정 전과 같은 판정(PASS)이 나온다(실행 확인).
- AC3 — 정당한 이웃 표본(`std::time::Instant` · 주석 안 자기인용 · `my_tokio::x`)이 FAIL 하지 않는다(실행 확인).

## 5. 미결 결정

- 없음(방향은 todo 가 정했다 — 정규식을 넓힌다). 대상 게이트 목록은 TRD 에서 확정하고 사용자에게 보인다.

## 6. 위험

- 정규식을 넓혀 정당한 코드가 걸리는 오탐 — AC2 · AC3 이 막는다.
- 문자열 리터럴 안의 `"tokio :: x"` 는 지금도 붙은 형태와 같은 취급이다(새 위험 아님).
