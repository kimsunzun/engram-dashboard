//! # engram-dashboard-platform — OS 층, 잎 crate (워크스페이스 의존 0 · 도메인 지식 0)
//!
//! OS 에 따라 달라지는 **운영** 코드를 이 crate 하나에 모은다(ADR-0266 결정 1). 여기 들일지는 OS 의존
//! 여부 하나로 판정한다 — 소비자 수가 아니다(결정 2). 부르는 쪽은 이 crate 의 함수와 핸들 타입만 부르고,
//! OS 분기(`#[cfg(windows)]` · `cfg!(windows)` · `#[cfg(unix)]` 따위)는 이 crate 안에만 둔다. 시험 · 시험
//! 하네스 bin · `examples/` 의 OS 분기는 그쪽에 남는다 — 운영 빌드에 들어가지 않는다(ADR-0266 「근거」).
//! 이 불변식의 기계 게이트는 TRD 1-3 U8 에서 선다 — 그 전까지는 리뷰가 지키고, 다른 crate 에 남은 운영
//! OS 분기는 이전 대상이다. 시한부 예외는 하나 — `src-tauri/src/fsutil.rs` 의 `cfg!(windows)`(ADR-0275 결정 15).
//!
//! 지금 입주자는 넷이다 — [`process`](PID liveness · 프로세스 시작시각과 그 세 갈래 판정 · 프로세스 표 ·
//! 자식 PID 열거 · 한 뿌리 아래 신원 목록) · [`file_holders`](이 파일을 지금 연 프로세스 — Restart Manager) ·
//! [`shell`](대화형 기본 셸 · CLI 를 콘솔 셸로 감싸기) · [`env`](홈 디렉터리 · 실행 파일 이름 · 환경변수
//! 이름 비교). 모듈 헤더와 공개 함수 문서가 그 책임의 정본이다.
//!
//! ## 들이는 규칙
//!
//! 1. **도메인 지식 0**(ADR-0266 결정 3). 「홈 디렉터리 찾기」는 여기지만 `.claude` 경로 · codex 홈의 하위
//!    폴더 · 락 파일 이름 같은 백엔드 지식은 부르는 쪽에 남는다. 그래서 공개 함수는 도메인 타입을 받지 않고
//!    원시 값(프로그램 · 인자 · PID · 경로)을 받으며, 도메인 값(종료 코드 같은 상수)도 인자로 받는다
//!    (ADR-0275 결정 12).
//! 2. **인터페이스 = 함수 + 핸들 타입, 컴파일 시점 분기**(ADR-0266 결정 5). OS 를 고르는 트레이트 객체를
//!    두지 않는다. 시험용 가짜가 필요하면 **부르는 쪽이** 작은 트레이트를 둔다 — 그래서 이 crate 의 공개
//!    API 에는 시험 seam 트레이트가 없다.
//! 3. **Windows 밖 갈래는 자리채움이다**(ADR-0266 결정 7). 이 crate 가 다른 OS 구현을 새로 만들지 않는다 —
//!    옮겨 올 때 있던 갈래(빈 답이든 `bash` · `HOME` 같은 한 줄이든)가 한 자리에 모여 보일 뿐이다. ★`#[cfg]`
//!    로 가른 그 갈래는 여기서도 컴파일 검증을 받지 않는다★ — 개발 · CI 가 Windows 뿐이다(ADR-0230 현황).
//!    `cfg!(windows)` 로 가른 갈래(`env::env_key_eq`)는 양쪽 다 타입 검사를 받지만 그 밖의 OS 에서 돌려 본
//!    적은 없다.
//! 4. **로그는 결과를 돌려줄 길이 없는 경로(`Drop`)에서만 찍는다**(TRD 1-3 §3-7 —
//!    `docs/process/S21-crate-boundaries/trd-1-3-platform-crate.md`). 나머지 실패는 값으로 돌려주고 부르는
//!    쪽이 자기 문구로 찍는다 — 도메인 낱말을 이 crate 의 로그로 가져오지 않는다(규칙 1). 찍을 때는
//!    `tracing` facade 를 직접 부른다(ADR-0266 결정 9 · ADR-0268) — 로그 때문에 base 를 끌지 않는다.
//!
//! ## 의존 (불변)
//!
//! - **워크스페이스 crate 의존 0 — base 도 아니다**(ADR-0266 「거부한 대안」 둘째 · ADR-0268). 아래 게이트
//!   ①이 그 벽이다 — 컴파일러는 잎 성질을 강제하지 않는다.
//! - **운영 의존의 서드파티는 `windows`(Windows 대상만)와 `tracing` facade 만 들인다**(TRD 1-3 §0 ①) —
//!   시험 전용 dev 의존은 이 규칙 밖이다. `windows` 는 의존 하나에 쓰는 바인딩 feature 의 합집합을 켜고, 기능별 cargo
//!   feature 로 쪼개지 않는다(ADR-0275 결정 13). `tracing` 은 규칙 4 의 `Drop` 경로가 찍는 데만 쓴다.
//! - ★**async 런타임(`tokio` 등)을 들이지 않는다**★ — 동기 crate 도 OS 층을 부른다. discovery 는 async
//!   런타임 반입을 CI 게이트(「discovery has no async-runtime ingress」)로 막고 있어, 이 crate 를 부르는
//!   순간 여기 든 런타임이 그 게이트를 빨갛게 만든다.
//!
//! ## 격리 게이트(불변) — 둘이고, 각각 다른 축이다
//!
//! **① 워크스페이스 의존 상한**(위 「의존」 첫 항의 벽):
//! `cargo tree -p engram-dashboard-platform --depth 1 --prefix none -e normal,dev,build --target all`
//! `--all-features | rg "^engram-dashboard" | sort -u` → **정확히 1줄 = 자기 자신**.
//! 서드파티는 이름 접두가 달라 세지 않는다. ★매니페스트 텍스트를 grep 으로 바꾸지 말 것★ — rename ·
//! `[dependencies.<이름>]` 테이블 형 · `[build-dependencies]` · 비활성 target · `optional` 이 전부 정상 Cargo
//! 문법으로 빠져나간다. 플래그도 줄이지 않는다(base · command · messaging · transport 상한 게이트와 같은 근거).
//! ★남는 구멍 둘★ — 멤버를 `engram-dashboard` **이름 접두**로 식별하기 때문이다(ADR-0151 「개명 함정」).
//! (a) 그 접두가 없는 워크스페이스 멤버를 의존하면 이 게이트를 그냥 통과한다. (b) 이 crate 가 그 접두를
//! 떼면 이 게이트는 빨개지지만(`-p` 가 죽거나 자기 줄이 접두를 잃는다), **다른** crate 의 상한 게이트
//! (base · messaging · command · transport)와 메시징 이름 정규식은 이 crate 로 가는 간선을 못 보게 된다 —
//! 금지된 base → platform 간선(ADR-0175 입주 조건 ② — ADR-0266 결정 8 이 다시 적는다)이 그대로
//! 빠져나간다.
//!
//! **② Tauri import 0**(ADR-0003):
//! `rg "^\s*use tauri" crates/engram-dashboard-platform/src/` → **0줄**. 돌리기 전에
//! `test -d crates/engram-dashboard-platform/src` 로 경로부터 본다 — 경로가 없어도 rg 의 매치는 0이라
//! 통과로 읽힌다.
//! ★①이 이 축을 덮는다고 읽지 말 것★ — ①은 워크스페이스 멤버만 세므로 서드파티 `tauri` 는 그대로
//! 통과한다. ADR-0003 의 불변식이 걸리는 축은 crate 이름이 아니라 **어느 바이너리에 링크되나** 이고,
//! OS 층은 창도 webview 도 없는 headless 데몬까지 모든 바이너리의 바닥에 깔리는 자리다. 패턴이 import
//! 라인 앵커인 것은 이 헤더가 자기 자신에 걸리지 않게 하기 위해서다.
//!
//! ★이 crate 는 생성물을 만들지 않는다★ — serde 도 ts-rs 도 없으므로 `bindings/` 디렉터리가 없다. CI 의
//! ts-rs sync 게이트 경로 목록에 넣지 말 것.

// ADR-0266
// ADR-0275
pub mod env;
pub mod file_holders;
pub mod process;
pub mod shell;
