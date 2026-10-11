# Rust 장기 실행 프로그램의 패닉 처리 관행 — 우리 안이 과한가 (2026-10-12)

- **상태:** medium · 적대 리뷰 1회 반영(codex BLOCK → 지적 7 반영) · 결정 대기 — 패닉 TRD 11판(`docs/process/S21-panic-policy/trd.md`) 결정 (가)(나)의 근거 자료.
- **계기:** 사용자 「우리가 너무 민감하지 않나. 다른 곳은 보통 어떻게 하나」(2026-10-12) — TRD 11판의 (가) 「공유 락을 쥔 동안 가드 표시를 끈다」 안을 두고.
- **방법:** 수집 갈래 셋(서버·인프라 / 데스크톱·에디터·터미널 / Rust 공식 문서·커뮤니티) 병렬 → 메인 grounding(핵심 넷을 원본 소스로 재확인: rust-analyzer · materialize · tikv · zed) → codex 적대 리뷰.
- **확신도 범례:** 확실 = 소스 원문 확인 · 가능성 높음 = 단일 출처 또는 부분 확인 · 불확실 = 확인 못 함. 줄 번호는 2026-10-12 기본 브랜치 기준이고 움직인다.
- 이전 조사 = `origin/v0.3.3/feat/panic-policy` 브랜치의 `docs/research/panic-policy-peers-2026-10-08.md`(선택지 1~4 비교). 이 문서는 그 위에 「보통은 어디까지 가두나 · 락 오염을 어떻게 다루나」를 더한다.

## 결론

(적대 리뷰 반영판 — 리뷰가 BLOCK 으로 짚은 과장 · 누락을 고쳤다. 고친 곳 = 아래 「적대 리뷰」.)

1. **「가두지 못한 패닉 = 끝」은 흔한 모양이다 — 우리 「즉시 종료」는 과민하지 않다(가능성 높음).** 표본 중 tikv(`_exit(1)`/`abort()`) · materialize(`abort()`) · zed(미니덤프 뒤 `abort()`) · zellij(세션 정리 뒤 종료)는 훅이나 서버가 끝낸다. 배포판을 아예 abort 로 두는 곳도 있다 — deno · OpenVMM · Turso, 그리고 Oxide 의 프로그램 대부분(작성자 진술). 훅이 없는 unwind 프로그램(vector · linkerd2-proxy 등)은 그 스레드만 죽고 프로세스는 남는다 — 「끝낸다」가 보편은 아니다.
2. **가두는 범위는 대개 좁고, 우리 안은 넓은 편이다(가능성 높음).** 가두는 자리는 주로 상태를 안 바꾸거나 격리된 단위다 — rust-analyzer 읽기 전용 요청 · nushell 자동완성 소스 · materialize 의 이름 붙은 자리 · herdr FFI 콜백 하나. 반례도 있다 — Qdrant 는 공유 준비 상태를 바꾸는 상태 점검 future 를 통째로 가두고 다시 돌린다. tokio 태스크 패닉 전달(기본) · tower-http `CatchPanicLayer`(기능 `catch-panic` + 명시 레이어 — 옵트인)는 공유 상태 문제를 다루지 않는다. 우리 안은 상태를 바꾸는 요청 · 에이전트까지 가두므로 표본 대부분보다 넓다.
3. **뭉치 1·2(가드 표시 + 표시 밖이면 훅이 끝냄)는 materialize 에 선례가 있다(확실).** 스레드별 카운터(`CATCHING_UNWIND`)로 「자기 `catch_unwind` 안」을 표시하고 훅이 그 밖이면 abort 한다. ★단 async 쪽은 스레드별이 아니라 **tokio 태스크별 표시**(`CATCHING_UNWIND_ASYNC` · `src/ore/src/future.rs`)를 따로 둔다★ — 멀티스레드 런타임에서 future 는 `.await` 마다 스레드를 옮길 수 있어 스레드 표시만으로는 틀린다. 우리 요청 쪽 가드(WS 명령 · 제어 HTTP · MCP)는 async 라 TRD 가 이것을 다뤄야 한다.
4. **원칙(「락을 쥔 중의 패닉 = 공유 불변식이 깨졌을 수 있다 → 계속 쓰지 말라」)은 선례가 있고, 메커니즘(락을 쥔 동안 가드 표시를 꺼서 즉시 종료)은 못 찾았다(가능성 높음).** 원칙 = std `Mutex` 문서(「likely tainted」 · `unwrap()` 으로 전파) · sunshowers 「on poisoning」(임계 구역 안 패닉 = 공유 불변식 손상 · abort 를 대응으로 논한다). 같은 글은 **읽기 전용 임계 구역 · 불변식을 깨지 않는 단순 쓰기는 다르다**고 가른다 — 「공유 락 전부」를 같은 규칙으로 묶는 것은 그보다 거칠다. rust-analyzer 는 같은 원칙을 「상태를 바꾸는 처리는 아예 안 가둔다」로 푼다.
5. **락 오염에 따로 정책을 세운 곳은 표본에 없다(가능성 높음).** 두 갈래 — ① `parking_lot`(오염 없음 — 패닉 때 말없이 풀린다 · zed · wezterm · helix · tikv) ② std 락 `.lock().unwrap()`(다음에 잡는 스레드가 패닉 — zellij 약 227곳 · herdr 약 179곳). ★둘 다 프로세스 종료를 보장하지 않는다★ — ①은 깨졌을 수 있는 데이터로 계속 가고, ②는 그 스레드를 죽일 뿐이다. 종료는 1의 훅이 따로 낸다.

**우리에게 주는 뜻(메인 해석):** 「즉시 종료」 자체는 과민하지 않다. 복잡도의 원인은 **표본 대부분보다 넓게 가두기로 한 것**(ADR-0288 「패닉에 의한 종료는 최대한 하지 않는다」)이고, (가)의 락 규칙은 그 넓은 가두기의 값이다. 고르는 축 = ① 넓게 가두고 락 규칙을 둔다(지금 안 — 원칙은 선례 있음 · 메커니즘은 우리 설계) ② rust-analyzer 처럼 공유 상태를 바꾸는 요청은 안 가둬 락 규칙을 없앤다. 어느 쪽이든 async 가드는 태스크별 표시가 필요하다(결론 3).

## 프로젝트별 (축: 배포판 panic · 가두는 범위 · 훅 · 락 오염)

### 서버 · 인프라

| 프로젝트 | 배포판 | 가두는 범위 | 훅 | 락 오염 |
|---|---|---|---|---|
| tikv | unwind 명시(`Cargo.toml:447`) 확실 | 몇 곳(범위 미확인) 불확실 | 「Exit the whole process when panic」 — 로그 → `abort()` 또는 `_exit(1)`(`components/tikv_util/src/lib.rs:525-607`) 확실 | `parking_lot` 의존 확실 · 주력인지는 가능성 높음 |
| materialize | unwind(기본) 확실 | 자기 `catch_unwind` 를 부르는 이름 붙은 자리만 가능성 높음 | 백트레이스 · Sentry → 「Instructs the entire process to abort if any thread panics」 · 자기 래퍼 안이면 예외(`src/ore/src/panic.rs:42,115-140,261`) 확실 | 모름 |
| deno | abort 확실 | — | 미확인 | 모름 |
| databend | unwind 확실 | 모름 | 로그만으로 보임 가능성 높음 | 모름 |
| sccache | unwind 확실 | 모름 | 진단용(위치 기록) 확실 | 모름 |
| vector · linkerd2-proxy | unwind 확실 | 모름 | 운영 훅 없음 가능성 높음(→ 기본 동작: 그 스레드만 죽음) | 모름 |
| tokio(라이브러리) | — | 기본 `UnhandledPanic::Ignore` — 태스크 패닉은 `JoinHandle` 로 가고 런타임은 계속(`tokio/src/runtime/builder.rs`) 확실 | — | — |
| tower-http(라이브러리) | — | `CatchPanicLayer` = 패닉 → 500 응답 · 「using panics for error handling is _not_ recommended」 · 공유 상태 언급 없음 확실 | — | — |

### 데스크톱 · 에디터 · 터미널

| 프로젝트 | 배포판 | 가두는 범위 | 훅 | 락 오염 |
|---|---|---|---|---|
| rust-analyzer | unwind 확실 | 읽기 전용 요청만 — 「they don't modify the state, so it's OK to recover」 · 상태를 바꾸는 `on_sync_mut` 은 「isn't guarded by `catch_unwind`, so, please, don't make bugs :-)」(`crates/rust-analyzer/src/handlers/dispatch.rs:34-44`) 확실 | 모름 | 모름 |
| zed | unwind 확실 | 사실상 없음 가능성 높음 | 크래시 처리 하위 프로세스로 미니덤프 → `abort()`(`crates/crashes/src/crashes.rs`) 확실 | `parking_lot` 확실 |
| wezterm | unwind 확실 | 모름 | 치명 알림 토스트 → 기본 훅 확실 | `parking_lot` 확실 |
| zellij | unwind 확실 | 없음 가능성 높음 | 메인 스레드면 보고 → `exit(1)` · 그 밖이면 서버가 모든 클라이언트를 내보내고 세션 종료(`zellij-utils/src/errors.rs:812-869`) 확실 | std 락 `.unwrap()` 약 227곳 → 연쇄 확실 |
| nushell | unwind 확실 | 자동완성 소스만 패닉 → `ShellError` 확실 | 터미널 복구 → 보고 · 종료 안 함 확실 | 모름 |
| alacritty | unwind 확실 | 없음 불확실 | Windows 오류 상자 확실 | 모름 |
| herdr | unwind(기본) 확실 | FFI 콜백 하나 확실 | 클라이언트 = 터미널 복구 → 기본 훅 확실 | `into_inner` 한 곳 · 나머지 약 179곳 `unwrap`/`expect` 확실 |
| helix | unwind 확실 | 모름 | 모름 | `parking_lot` 확실 |

## Rust 공식 문서 · 커뮤니티

- `catch_unwind` 문서: 「It is not recommended to use this function for a general try/catch mechanism」 — 예상되는 실패는 `Result`(https://doc.rust-lang.org/std/panic/fn.catch_unwind.html) 확실.
- std `Mutex` 문서: 오염 = 「likely tainted (some invariant is not being upheld)」 · 「Most usage of a mutex will simply unwrap() these results, propagating panics among threads」 · 오염은 「only advisory」(`into_inner` 로 꺼낼 수 있다)(https://doc.rust-lang.org/std/sync/struct.Mutex.html) 확실.
- 오염을 없앨지 논쟁 중 — `std::sync::nonpoison` 추적 이슈 #134645(unstable 구현 · 안정화 PR 없음) · Edition 2027 기본 락 논의 #149359(합의 없음 — 남기자 쪽 = 깨진 데이터로 계속 가는 것을 막는다 · 없애자 쪽 = abort 앱엔 값어치가 적고 운영에서 서비스를 영구 고장 낼 수 있다) 가능성 높음(요약 페이지로 읽음).
- sunshowers 「on poisoning」: 오염 찬성 · `.lock().unwrap()` 권고 · 앱은 abort 도 괜찮다(https://sunshowers.io/posts/on-poisoning/) 가능성 높음.
- `parking_lot` 월 약 8,540만 내려받기 · 직접 의존 8,610(lib.rs, 2026-10) — 오염이 없어서 고른 것인지 속도 때문인지는 구분 안 됨.
- crash-only / microreboot(Candea & Fox): 작은 격리 단위를 다시 띄우는 것은 상태가 격리되거나 밖으로 빠져 있을 때만 성립한다(https://www.usenix.org/legacy/events/osdi04/tech/full_papers/candea/candea_html/) 확실(개념) — 「공유 락을 쥔 중의 패닉 = 격리 실패 → 전체로 올림」은 그 원칙의 메인 해석이다(출처 없음).

## 공백 · 기권

- Tauri 앱의 관행 · helix 훅 · wezterm 가두는 범위 · deno 훅 내용 — 확인 못 함(GitHub code search 속도 제한).
- 배포판 abort 를 쓰는 crate 비율 통계 — 없음.
- Mara Bos · BurntSushi · matklad 의 오염 입장 — 출처를 찾지 못해 인용하지 않는다.
- Erlang 「let it crash」 1차 자료 — 읽지 않았다.

## 적대 리뷰

codex(cross-family · effort high · 웹 검색) 1회 — 판정 **BLOCK**(결론이 근거보다 넓다). 인용 넷(rust-analyzer · materialize · tikv · zellij)은 원문과 맞다고 확인했다. 지적 일곱 전부 반영(위 결론 1~5 재작성):

| 지적 | 유형 · 심각도 | 반영 |
|---|---|---|
| Qdrant 가 공유 상태를 바꾸는 future 를 가두고 다시 돈다(`src/common/health.rs`) | 반례 · high | 결론 2 「뿐」 → 「대개」 + 반례 명시 |
| sunshowers 가 임계 구역 패닉과 공유 불변식 손상을 잇고 abort 를 논한다 | 누락 선례 · high | 결론 4 = 원칙은 선례 있음 · 메커니즘은 못 찾음 |
| `parking_lot` · `unwrap()` 이 프로세스 종료를 뜻하지 않는다 | 잘못된 추론 · high | 결론 5 정정 |
| materialize 는 async 에 태스크별 표시를 따로 둔다(`src/ore/src/future.rs`) | 불완전한 비유 · high | 결론 3 에 추가 — TRD 반영 대상 |
| 배포판 abort 가 예외적이라는 추론 — OpenVMM · Turso · Oxide | 표본 누락 · medium | 결론 1 정정 |
| 읽기 전용 임계 구역 · 불변식 보존 쓰기를 구분하지 않았다 | 논리 공백 · medium | 결론 4 에 명시 |
| tower-http `CatchPanicLayer` 는 기본값이 아니다(기능 + 명시 레이어) | 사실 오류 · medium | 결론 2 정정 |

메인 재확인 안 한 것: Qdrant · OpenVMM · Turso 소스, sunshowers 글의 해당 문단(리뷰어가 읽은 것 — 확신도 가능성 높음으로 둔다).
