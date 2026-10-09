# 패닉 처리 정책 — 피어 서베이 (2026-10-08)

- **상태:** 조사 완료 · **결정 = ADR-0288 · ADR-0290**(2026-10-10 — 요청 하나 · 에이전트 하나는 가두고, 공유 상태에서 난 패닉은 관행대로 정상 종료 · 가두지 못한 패닉은 훅이 종료를 당긴다 · abort 는 그냥 종료. 그 사이 ADR-0289(살려 두고 강한 경고)가 있었고 ADR-0290 이 폐기했다). codex 적대 리뷰 결과는 「적대 리뷰」 절.
- **방법:** /research medium · 설계-결정 모드. 갈래 셋(A 직접 피어 = 로컬 클론 + 스크래치 shallow clone 9개 · B 메커니즘 = 1차 문서 · C 사용자 대면 동작 = 제품 문서·소스) → 메인 합성 + grounding 스팟체크 → codex 적대 리뷰.
- **확신도 범례:** 확실(독립 교차확증) · 가능성 높음(단일 출처 grounding 통과) · 불확실(미검·추론).
- **계기:** 경계 리팩터링 A 범위 정리 중 「락 오염 때 패닉할지 복구할지」를 정하려다, 사용자가 「패닉이 나면 메시지를 띄워 종료를 안내하고 살려 두면 사용자가 저장할 건덕지라도 있지 않나」를 제안했다. 이 보고서는 그 제안의 선례와 비용을 잰다.

## 우리 현황 (코드 실측 · 2026-10-08 `aef34d4`)

- 워크스페이스 `[profile.release]` = `panic = "abort"`(`Cargo.toml:41`) — 데몬 · 셸 · CLI 배포판 전부. 어느 스레드든 패닉 = 그 프로세스 즉사. 그래서 운영 빌드에선 락 오염이 생기지 않는다(base `sync` 헤더 계약 3).
- 데몬 패닉 훅 = `tracing::error!` 한 줄을 남긴 뒤 이전 훅(기본 훅 — stderr 진단)을 부른다(`crates/engram-dashboard-daemon/src/lib.rs:133-147`). 사용자 고지 없음.
- 셸 = 패닉 훅 없음(`src-tauri/src` 에 `set_hook` 0).
- 데몬이 죽으면 에이전트도 전부 죽는다 — 에이전트 프로세스는 `KILL_ON_JOB_CLOSE` Job 에 든다(`crates/engram-dashboard-platform/src/group/mod.rs:38,53`). 셸이 죽으면 데몬 · 에이전트는 산다(별도 프로세스).
- 락 처리 혼재: 오염 복구 약 117곳(ADR-0275 실측) · `lock().unwrap()`/`expect` 약 250곳(시험 포함 · 2026-10-08 조사 워커 계수).

## 결론

1. **사용자 제안(어디서 난 패닉이든 프로세스 전체를 살려 두고 마무리를 안내)의 선례는 조사한 범위에 없다.** 가능성 높음 — 조사 대상 = Rust 피어 zellij · herdr · wezterm · rust-analyzer · sccache · vector · Zed · Alacritty · nushell · Deno · Tauri · atuin · helix(+ vibe-kanban) · 제품 VS Code(확장 호스트 · pty host) · Zed · tmux · zellij · Chrome · JetBrains. ★단 「요청 · 작업 하나를 가두고 서버는 산다」는 흔하다★(아래 (나) · 적대 리뷰 1). 가장 가까운 것은 JetBrains IDE(잡힌 예외를 알림으로 띄우고 계속 돈다)인데 관리형 런타임의 예외라 Rust 패닉과 성질이 다르다(지식 기반 · 미검).
2. **피어는 네 무리로 갈린다.** 확실(로컬 소스 확인) — 단 (라)와 (나)의 tower-http 는 적대 리뷰가 더했다(docs.rs 근거). herdr · atuin · helix 는 어느 무리에도 넣지 않았다 — 서버 쪽 패닉 훅 · 격리 경계가 없어 「비주 스레드 패닉은 그 스레드만 죽는다」는 std 기본 동작에 기댄다(herdr 는 가능성 높음 · 나머지 불확실).
   - **(가) 패닉 = 종료, 대신 알리고 남긴다** — zellij · Zed · Alacritty · wezterm. Deno · Tauri 저장소는 배포판이 `panic = "abort"` 라는 것만 확인했다(패닉 훅 · 실제 동작 미확인 — 불확실). zellij 서버는 어느 스레드 패닉이든 붙은 클라이언트 전부에 `Exit{Error(backtrace)}` 를 보낸 뒤 끝난다(`zellij-server/src/lib.rs:1596-1607` — 메인 확인). Alacritty 는 Windows 에서 `MessageBoxW` 「Alacritty: Runtime Error」 를 띄우고 죽는다(`alacritty/src/panic.rs:17-20` — 메인 확인). Zed 는 사이드카가 minidump 를 남기고 다음 기동 때 보고한다.
   - **(나) 요청 · 작업 하나를 가두고 산다** — rust-analyzer(읽기 전용 요청만 `catch_unwind` · 상태를 바꾸는 `on_sync_mut` 은 일부러 안 가둔다 — 「they don't modify the state, so it's OK to recover」 · 「don't make bugs :-)」, `crates/rust-analyzer/src/handlers/dispatch.rs:34-44` 메인 확인) · sccache(컴파일 요청 하나 · `src/server.rs:1530` 메인 확인) · nushell(REPL 한 바퀴) · tower-http `CatchPanicLayer`(핸들러 패닉을 HTTP 500 으로 — 공유 상태를 쥔 핸들러도 가둔다 · 적대 리뷰 1). ★「공유 상태를 안 바꾸는 단위만」은 rust-analyzer 가 명시한 기준이지 무리 전체의 규칙이 아니다★ — 웹 서버류는 그 기준 없이 가두고, 오염된 락이 남는 것은 그 위 코드의 몫으로 둔다.
   - **(다) 잡되 질서 있게 끄려고 잡는다** — vector(컴포넌트별로 잡아 topology 전체를 내린다).
   - **(라) 감독 트리** — ractor(자식 actor 의 패닉이 감독자에게 알려진다) · ractor-supervisor(재기동 정책 · 상한). Erlang 식 「죽게 두고 감독자가 다시 띄운다」의 Rust 이식. 프로세스는 살고 죽은 단위만 새로 뜬다 — 단 어디서 난 패닉이든 안전하게 되살린다는 보장은 아니다(적대 리뷰 2).
3. **살아남는 피어는 전부 unwind 빌드다.** 확실. `catch_unwind` 는 `panic = "abort"` 에서 무력하다(std 문서). 우리 배포판 설정으로는 (나)도 성립하지 않는다.
4. **성숙 제품의 주류 해법은 「살려 두기」가 아니라 「상태 주인을 버그 나기 쉬운 코드와 다른 프로세스에 두기 + 자동 재기동 + 복원」이다.** 가능성 높음.
   - VS Code: 터미널을 별도 pty host 프로세스에 두어 렌더러 · 확장 호스트가 죽어도 터미널이 산다. pty host 자신은 죽으면 재기동 횟수 상한을 두고(`_restartCount <= MaxRestarts`, `MaxRestarts = 5` — 0 부터 세므로 최대 여섯 번 · 적대 리뷰 6), 레이아웃 · cwd 로 터미널을 **새로 만든다**(옛 프로세스는 잃는다 · `ptyHostService.ts`). 확장 호스트는 5분에 3회까지 자동 재기동 후 버튼 고지(`abstractExtensionService.ts`).
   - zellij: 세션 레이아웃 · 명령을 캐시에 직렬화해 두고 재부착으로 되살린다. 되살린 명령은 Enter 를 눌러야 돈다(사고 방지).
   - tmux: 서버가 죽으면 다 죽는다 — 복원은 서드파티(tmux-resurrect).
5. **우리 구조는 tmux 쪽이다.** 데몬이 상태 주인이면서 버그가 날 수 있는 코드 전부를 품는다. 셸 쪽은 이미 VS Code 모양(셸이 죽어도 데몬 · 에이전트가 산다).

## 메커니즘 사실 (갈래 B)

| 항목 | 내용 | 확신도 |
|---|---|---|
| abort 에서도 패닉 훅은 돈다 | 그 뒤 소멸자 없이 끝난다. Windows 는 fast-fail 종료코드 `0xc0000409` | 확실 (std `process::abort` 문서 · MS fastfail 문서) |
| `catch_unwind` | unwind 패닉만 잡는다 — abort · 이중 패닉 · 외부 예외는 못 잡음. 범용 try/catch 가 아니다(문서) | 확실 |
| tokio 태스크 패닉 | unwind 면 `JoinError` 로 잡히고 런타임은 산다(기본 `UnhandledPanic::Ignore`). `ShutdownRuntime` 은 unstable · current_thread 전용 | 확실 (docs.rs) |
| std 스레드 패닉 | unwind 면 그 스레드만 끝나고 `join` 이 `Err` | 가능성 높음 (지식 기반) |
| 락 오염 | std 는 계속 오염시킨다. `std::sync::nonpoison` 은 unstable(#134645 열림). 기본 락을 Edition 2027 에서 바꾸자는 논의(#149359)는 탐색 단계 | 확실 (존재 · unstable) |
| parking_lot | 오염 없음 — 패닉 때 그냥 풀린다. wezterm mux · Zed · helix · atuin 이 쓴다 | 확실 |
| Tauri v2 | 커맨드 디스패치에 `catch_unwind` 없음(tauri 2.11.3 소스 grep — 모바일 진입점 하나뿐). 동기 커맨드는 메인 스레드에서 돈다. 패닉 정책 안내 문서 없음 | 확실(grep) · 불확실(실제 동작) |
| abort 때 사용자에게 보이는 것 | WER 대화상자 여부는 시스템 설정에 달림 — 모름 | 불확실 |

## 선택지 × 제약 적합도

| | 사용자 구출 기회 | 상태 오염 위험 | 선례 | 비용 | 락 오염 정책 귀결 |
|---|---|---|---|---|---|
| **1. 현행(abort) + 알림 보강** — 데몬 훅이 크래시 기록을 남기고, 셸이 데몬 사망을 감지해 「내부 오류로 데몬이 종료됨 · 로그 위치」를 띄운다. 재기동 뒤 복원 경로를 다듬는다 | 없음(즉사) — 대신 재기동 · 복원으로 메운다 | 없음 | (가) 다수 · zellij 가 가장 가깝다 | 작음 | 패닉(A) — 운영엔 오염이 안 생긴다 |
| **2. unwind + 요청 · 명령 하나를 가둠** (rust-analyzer · sccache · tower-http 식) — 예: 조회 요청 · 버스 명령 하나의 처리. 패닉이면 그 요청만 오류 응답 | 일부(가둔 단위에서 난 패닉만) | 낮음~중간 — 상태를 바꾸는 단위까지 가두면 반쯤 바뀐 상태가 남는다. 어디까지 가둘지가 판단의 핵심 | (나) | 중간 — 단위별 감사 | 가두지 않은 곳 = 종료 → 패닉(A). 가둔 곳의 락은 오염이 남으니 단위별로 정한다 |
| **3. unwind + 에이전트 하나를 가둠 + 감독** ((라) 식) — 에이전트 하나의 스레드 · 태스크가 패닉하면 그 에이전트만 실패로 정리하고 나머지는 산다 | 큼(다른 에이전트가 산다) | 중간 — ★펌프 자신의 패닉은 이미 펌프 안에서 가둬진다★(`crates/engram-dashboard-agent/src/transport/pty.rs:274` 「B-2」 — `catch_unwind` 뒤 `core.finish` 로 `Failed` · 같은 꼴 `stdio.rs:305`. abort 빌드라 지금은 무력하다). 열린 것은 펌프 밖의 에이전트 전용 코드(쓰기 · 출력 해석기 · 래치)다. 감독자를 세 번째 정리 호출자로 두면 ADR-0127 이 금하는 인과 분기다 | (라) · Erlang 식 | 중간 — 펌프 밖 경계 감사 · 불변식 재검토 | 에이전트 안 락 = 그 에이전트와 함께 버린다. 공유 락은 2 와 같은 판단 |
| **4. unwind + 전체 생존 + 마무리 안내** (사용자 제안) | 있음 | **높음** — 패닉한 스레드가 하던 일이 반쯤 남는다(예: 공유 명부를 바꾸다 죽으면 명부엔 있는데 돌지 않는 에이전트가 남고, 오염된 명부를 계속 쓴다 · 펌프는 위 3 처럼 이미 스스로 가둔다) | 없음 | 중간~큼 — 어떤 기능이 죽고 사는지 정의 · 셸 고지 경로 | 복구(B) 강제 |
| **5. 상태 주인 분리** — 에이전트 PTY 를 데몬과 다른 프로세스(에이전트별 호스트 등)가 소유해 데몬 패닉이 에이전트를 안 죽이게 | 있음(에이전트가 산다) | 낮음 — 단 데몬 재기동 뒤 다시 붙는 복원 경로를 따로 설계해야 한다(crash-only 설계 · 적대 리뷰 4) | VS Code pty host · Chrome | **큼** — 구조 변경. Windows 는 PTY 를 만든 프로세스가 쥐어야 한다(CLAUDE.md 「참조 구현」 합의 둘) — 호스트가 직접 만들어야 한다 | 무관 |

- **에이전트 대화 자체는 대개 패닉으로 잃지 않는다** — claude 가 대화를 저장하고 우리는 세션 id 를 영속한다(ADR-0226). 단 조건이 있다: id 는 첫 제출 뒤에만 영속되고(첫 턴 전에 죽으면 다음 활성화는 새 대화) `/clear` 추적은 best-effort 예외다(적대 리뷰 5). 패닉으로 잃는 것 = 진행 중이던 턴 · 메모리에만 있는 파킹 우편(ADR-0105) · 화면 출력.

## 거부 후보 → ADR 거부 대안 후보

- **4 (전체 생존)** — 선례 0 · 상태 오염 위험. 사용자가 고르면 거부가 아니다 — 판정은 사용자.
- **parking_lot 일괄 교체** — 오염 문제만 지우고 「패닉 뒤 반쯤 바뀐 상태」 문제는 그대로다. 단독 해법이 아니다.

## 적대 리뷰

codex(`codex exec` · high · web_search) 1회 — 판정 **FIX**, 8건 전부 반영.

1. (high · 반증) 「살아남는 피어는 공유 상태를 안 바꾸는 단위만 가둔다」 과일반화 — tower-http `CatchPanicLayer` 는 공유 상태를 쥔 핸들러도 가둔다(https://docs.rs/tower-http/latest/tower_http/catch_panic/). → (나) 서술을 고쳤다.
2. (high · 누락) 감독 트리 패턴(ractor · ractor-supervisor) 누락 → (라) · 선택지 3 추가.
3. (medium · 과장) 「선례 0」 의 대상 목록이 없다 → 결론 1 에 목록을 적고 주장을 「어디서 난 패닉이든 전체 생존」 으로 좁혔다.
4. (medium · 과장) 상태 주인 분리의 상태 오염 위험 「없음」 → 「낮음 · 복원 설계 필요」(crash-only — https://static.usenix.org/events/hotos03/tech/full_papers/candea/candea_html/index.html).
5. (medium · 과장) 대화를 잃지 않는다 → ADR-0226 조건을 붙였다.
6. (low · 반증) VS Code pty host 재기동 횟수 = 최대 여섯 번(`_restartCount <= MaxRestarts`).
7. (low · 반증) 데몬 훅은 로그 뒤 이전 훅도 부른다(`lib.rs:147`).
8. (low · 오귀속) rust PR #141828 은 `link.exe` 진단 변경이라 abort 경로의 근거가 아니다 → 인용 제거(MS fastfail 문서만 남김).

스팟 확인 통과(리뷰어): tokio 기본 `UnhandledPanic::Ignore` · `nonpoison` 추적 이슈 열림 · Tauri 생성 래퍼에 `catch_unwind` 없음 · 저장소의 release abort · Job 설정.

## 한계 · 공백

- VS Code pty host 의 원래 동기와 사용자 배너 출처는 1차 자료를 못 읽었다(검색 요약).
- Deno 의 패닉 훅 · create-tauri-app 템플릿의 기본 panic 설정 미확인.
- Warp · Windows Terminal · 에이전트 매니저류 미조사.
- 우리 데몬 안에서 (나)식으로 가둘 수 있는 단위가 실제로 얼마나 되는지는 재지 않았다.
