# S31 TRD — 소스 정규식 검사의 공백형 경로

- 상태: 판 2 · 2026-11-02 · 정본 ADR 없음(게이트 기대값의 정본은 각 대상 crate · 파일 헤더 — 아래 「같은 주장」). 판 2 = PRD+TRD 리뷰(Claude · codex) 반영.
- 브랜치 기준 = `v0.5.0/chore/skill-apply`(바인딩 · CLAUDE.md 정리분 위). 통합 순서 = skill-apply 다음.

## 범위와 PRD 관계

PRD R1–R3 · AC1–AC3 을 `.github/workflows/ci.yml` 의 검사 여섯에 적용한다(줄 번호 = 이 브랜치 기준).

| 검사(스텝 이름) | 줄 | 지금 패턴 | 공백형 |
|---|---|---|---|
| `Gate: base leaf crate — tenants must not reference each other` | 698 | `(crate\|super)::(logging\|…\|panic)` | `super:: sync` · `crate :: time` 통과 |
| `Gate: replay_flight purity — no transport/wire ingress` | 1262 | `…\b(tauri\|tokio\|engram_dashboard_[a-z_]+\|std::net)::` | `tokio :: spawn` · `std :: net :: X` 통과 |
| `transport gate 3: pure layer isolation (pending/stream/machine)` | 1339 | 같은 꼴(`tokio\|futures_util\|futures\|…`) | 통과 |
| `platform gate 6` (`std_os_re`) | 1197 | `…\bstd::os::(?:windows\|unix)\b` | `std :: os :: windows` 통과 — 명단 밖 파일이 안 잡힌다 |
| shell gate 2 — lint 끄기 부재 | 1028 | `…(?:wildcard_enum_match_arm\|clippy::restriction)` | `clippy :: restriction` 통과 |
| shell gate 2 — rustfmt 건너뛰기 부재 | 1031 | `rustfmt::skip\|rustfmt_skip` | `#[rustfmt :: skip]` 통과 — rustfmt 미적용 구역을 막는 가드 자체가 샌다 |

- 범위 밖 — `net gate 2a`. 첫 마디 공백(`engram_dashboard_platform :: fs`)은 숨은 꼴 검사(`:1389`)가, 깊은 마디 공백(`engram_dashboard_platform::fs :: f`)은 추출(`:1368`) 집합 불일치가 FAIL 시킨다. 시험에 표본 하나를 넣어 이 주장을 실측으로 남긴다.
- 범위 밖 — `:964` · `:987` · `:1026` 의 존재 검사(공백형이 들면 실패하는 안전한 쪽) · 줄을 넘는 경로.
- 범위 밖 · 새 할 일 — 공백이 아닌 다른 이름 꼴(`use tokio as rt;` · `extern crate tokio as rt;` · `use std::net;` · 여러 항목 묶음 `use std::{net::TcpStream, time::Instant};`)도 replay_flight · transport gate 3 을 통과한다(옛 · 새 패턴 모두). 한 항목 묶음은 rustfmt 가 풀어 써 게이트에 걸린다. 이번에 넓히지 않고 `docs/todo/observed-only.md` 에 새 항목으로 올리고, 「알려진 한계」 한 줄을 각 게이트의 정본 자리에만 적는다(replay_flight = `ci.yml` 스텝 주석 · transport gate 3 = transport `lib.rs` 헤더).

## 바꾸는 것

- `::` 앞뒤에 `\s*` 를 허용한다 — 이름 뒤 `\s*::`, 여러 마디 이름은 마디 사이마다 `\s*::\s*`.
  - base: `(crate|super)\s*::\s*(logging|text|time|path|sync|testing|writable|file|panic)`.
  - replay_flight: `^(?:[^/]|/[^/])*?\b(tauri|tokio|engram_dashboard_[a-z_]+|std\s*::\s*net)\s*::`.
  - transport gate 3: 같은 꼴(이름 목록은 그대로).
  - platform gate 6: `^(?:[^/]|/[^/])*?\bstd\s*::\s*os\s*::\s*(?:windows|unix)\b`.
  - shell gate 2: `clippy\s*::\s*restriction` · `rustfmt\s*::\s*skip|rustfmt_skip`.
- 접두 가드 · `\b` · 파일 존재 검사 · 판정 `case` 는 그대로 둔다(R3).
- 패턴은 bash 큰따옴표 · 작은따옴표 안이다 — `\s` 는 그대로 rg 에 간다.

## 같은 주장 — 문서 밖 사본

- 옛 패턴 문자열이 이 브랜치에서 살아 있는 자리(`rg -F` 로 확인 — `.github` 는 숨김 폴더라 `--hidden`):
  - `ci.yml` 의 각 스텝 해설 주석.
  - `crates/engram-dashboard-base/src/lib.rs:55` 헤더.
  - `crates/engram-dashboard-transport/src/lib.rs:48` 헤더 「격리 게이트」.
  - `crates/engram-dashboard-platform/src/lib.rs:169-171` matcher 해설(+ `:174` 「알려진 한계」 목록).
  - `src-tauri/src/daemon_client/replay_flight.rs:51-54` 헤더.
  - `docs/testing-strategy.md:50` · `:179`.
- 역사 기록(`docs/process/**` 다른 S · `docs/decisions/*` · `.claude/handoff/*` · `docs/refactoring/*` · `docs/research/*`)은 손대지 않는다.
- ★base 게이트는 주석 가드가 없다★ — base `lib.rs` 헤더를 고칠 때 `crate :: time` 같은 예시 경로를 쓰면 게이트가 자기 헤더에 걸린다(헤더 `:66` 경고). 패턴은 괄호째 인용하는 지금 방식을 유지한다.
- 코더가 위 목록을 출발점으로 옛 패턴 문자열을 다시 세고, 목록과 다르면 구현 전에 보고한다.

## 시험 계획

qa 가 각 검사를 `ci.yml` 원문 스텝 그대로 돌린다(qa-verify 「gates 잡」 추출). 표본은 대상 파일에 직접 넣지 않는다 — scratch worktree(`$QA_TMP/wt`)의 실파일에 한 줄을 넣고 그 worktree 에서 스텝을 그대로 돈다.
- scratch worktree 는 커밋 전 작업 트리의 변경(특히 새 `ci.yml`)을 실어야 한다 — `git diff` 를 그 worktree 에 적용한 뒤 돈다. HEAD 만 떠 오면 옛 패턴을 돌려 AC2 · AC3 이 아무것도 증명하지 못한 채 PASS 한다.
- 검사가 여럿 든 스텝(shell gate 2 · platform gate 6 · net gate 2a)은 표본을 다른 검사를 건드리지 않는 자리(예: 다른 0열 item 앞)에 넣고, FAIL 메시지가 대상 검사의 것인지 확인한다 — shell gate 2 「lint 를 끄는 attribute」 · 「rustfmt 건너뛰기」, platform gate 6 「명단 밖인데 걸린 파일」, net gate 2a 「심볼 꼴이 아닌 모양」.

- AC2 — 이 브랜치 트리에서 여섯 검사 모두 수정 전과 같은 판정.
- AC1(넣으면 FAIL) — 수정 전 패턴으로는 통과하던 것만 고른다:
  - replay_flight · transport: `use   tokio :: time;` · `let _ = tokio ::spawn(f);` · `tokio	::	x`(탭) · `let _ = std :: net :: TcpStream::connect(a);` · `let _ = std::net :: TcpStream::connect(a);` · `let _ = engram_dashboard_protocol :: X;`
  - base: `crate :: time::now()` · `super:: sync::X`
  - platform gate 6: 명단 밖 `.rs` 에 `use std :: os :: windows::io::AsRawHandle;`
  - shell gate 2: 대상 파일에 `#[rustfmt :: skip]` · `#[allow(clippy :: restriction)]`
  - 붙은 형태(`tokio::spawn`)도 여전히 FAIL.
- AC3(넣어도 FAIL 안 함): `std::time::Instant` · `std :: time :: Instant` · `// tokio :: spawn` 꼬리 주석 · `my_tokio :: x` · base 는 `crate::other_mod`.
- net gate 2a 표본(범위 밖 주장 실측): `use engram_dashboard_platform :: fs::open_deny_write;` → FAIL.
- 수정 전 패턴으로 AC1 표본이 PASS 하던 것도 한 번 보여 구멍이 실재했음을 남긴다.

## 구현 단위

- 코더 1 · 단위 1(파일이 겹친다). 변경 = `ci.yml` 패턴 6곳 + 해설 사본 + 「알려진 한계」 한 줄(게이트 정본 자리) + `docs/todo/doc-drift.md`(이 항목 닫기) · `docs/todo/observed-only.md`(새 항목).
- 단위 끝 = `ci.yml` YAML 파싱 OK + 코더가 AC1 · AC3 표본 몇 개를 rg 로 한 번 돌려 본다(최종 판정은 qa).

## 결정

- 대상 = 검사 여섯(리뷰가 platform gate 6 · shell gate 2 둘을 더했다). net gate 2a 는 이미 잡으므로 제외.
- 공백 허용 = `\s*`(탭 포함). 줄 넘김은 제외.
- 다른 이름 꼴 = 새 할 일 + 알려진 한계 기록(S31 에 합치지 않는다 — 구멍이 더 넓고 rustfmt 를 통과한 정상 코드에서도 나와 별도 판단이 필요하다).
- 브랜치 기준 = skill-apply(정리 뒤 바인딩 · 문서 기준으로 사본을 맞춘다).

## 위험

- 오탐 — AC2 · AC3 이 막는다(리뷰어 사전 실측: 넓힌 패턴을 지금 트리에 돌린 결과가 옛 패턴과 같다).
- `jobs:` 가 바뀌어 이 커밋의 CI 결론이 master 기준선과 다르다 — qa 바인딩 「전체 회귀 = CI」 규칙대로 판단한다.

## 리뷰 기록

- 2026-11-02 · PRD+TRD 리뷰 full(Claude worker-reviewer + codex xhigh) · 2라운드 PASS · 판 2 커밋 뒤 권장 3(줄 번호 · scratch worktree 에 변경 싣기 · FAIL 메시지 귀속) 반영.
- 2026-11-02 · 코드 리뷰 full(Claude worker-reviewer + codex) · PASS(must 0) · 권장 라운드 재리뷰 PASS. 그 반영으로 새 할 일 자리(`observed-only.md`) · 한계 기록 자리(게이트 정본 한 곳) · 묶음 예시(여러 항목)를 이 TRD 에도 맞췄다 — 설계는 그대로다.
