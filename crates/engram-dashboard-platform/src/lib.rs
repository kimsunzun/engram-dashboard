//! # engram-dashboard-platform — OS 층, 잎 crate (워크스페이스 의존 0 · 도메인 지식 0)
//!
//! OS 에 따라 달라지는 **운영** 코드를 이 crate 하나에 모은다(ADR-0266 결정 1). 여기 들일지는 OS 의존
//! 여부 하나로 판정한다 — 소비자 수가 아니다(결정 2). 부르는 쪽은 이 crate 의 함수와 핸들 타입만 부르고,
//! OS 분기(`#[cfg(windows)]` · `cfg!(windows)` · `#[cfg(unix)]` 따위)는 이 crate 안에만 둔다. 시험 · 시험
//! 하네스 bin · `examples/` 의 OS 분기는 그쪽에 남는다 — 운영 빌드에 들어가지 않는다(ADR-0266 「근거」).
//! 이 불변식의 기계 게이트는 아래 ④(`cfg` 위치) · ⑤(OS crate 의존) · ⑥(`std::os` 경로)다. 다른 crate 에 남은 `cfg`
//! 운영 OS 분기는 시한부 예외 하나뿐이다 —
//! `src-tauri/src/fsutil.rs` 의 `cfg!(windows)`(ADR-0275 결정 15). `cfg` 없이 런타임에 OS 규칙을 쓰는 자리는
//! 이 셈에 들지 않는다(④ 의 한계).
//!
//! 지금 입주자는 일곱이다 — [`process`](PID liveness · 프로세스 시작시각과 그 세 갈래 판정 · 프로세스 표 ·
//! 자식 PID 열거 · 한 뿌리 아래 신원 목록 · 한 프로세스 트리 끄기 — Windows `taskkill`) · [`group`](프로세스
//! 무리의 강한 주인 · 약한 손잡이 · 붙든 멤버 · 가입 알림 포트 — Windows Job Object) ·
//! [`file_holders`](이 파일을 지금 연 프로세스 — Restart Manager) ·
//! [`fs`](남의 쓰기를 막은 채 여는 열기 · 그 실패의 분류 — 공유 위반 · 접근 거부) ·
//! [`spawn`](창 없이 띄우기 · 트리 뿌리로 띄우기와 그 트리 kill 손잡이 — Windows = 멈춘 채 띄워 무리에 넣은 뒤
//! 깨우기 · 실패한 셸의 「프로그램 없음」 판정 · 이 프로세스의 Job 밖에서 띄우기 — Windows = WMI) ·
//! [`shell`](대화형 기본 셸 · CLI 를 콘솔 셸로 감싸기) ·
//! [`env`](홈 디렉터리 · 실행 파일 이름 · 환경변수 이름 비교). 모듈 헤더와 공개 함수 문서가 그 책임의 정본이다.
//!
//! ★`testing`(실프로세스 시험 도우미) · `group::GroupRef::gone`(주인이 처음부터 없는 손잡이) ·
//! `spawn::wmi_create_raw`(WMI 띄우기의 원시 호출 — 진단 시험 몫)는 cargo 기능 `test-support` 뒤다★
//! (TRD 1-3 §3-8 · 이름은 base 의 ADR-0275 결정 5 와 같다) — 선언이 `#[cfg(any(test, feature = "test-support"))]` 라 이 crate 자기 시험과 그 기능을 dev 의존으로 켠 소비자 시험만
//! 본다. 그 기능이 데몬 · 셸의 운영 의존 그래프에 없다는 것은 CI 게이트 ③ 이 잰다(0줄 기대와 짝인 1줄
//! 이상 기대) — 아래 격리 게이트 ①② 와 별개인 시험 기능 누수 게이트이고, 명령 · 기대값의 정본은 `/qa` 바인딩이다. 이 crate 의 cargo 기능은 그것 하나뿐이다(ADR-0275 결정 13).
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
//!    `cfg!(windows)` 로 가른 갈래(`env::env_key_eq` · `fs` 의 실패 분류 둘)는 양쪽 다 타입 검사를 받지만 그
//!    밖의 OS 에서 돌려 본 적은 없다.
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
//! ## 격리 게이트(불변) — ①② 는 각각 다른 축이다(시험 기능 누수 게이트 ③ 은 위 `testing` 단락 · 불변식
//! 게이트 ④~⑥ 은 맨 아래)
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
//! **④ OS 분기는 이 crate 안에만**(맨 위 불변식 · TRD 1-3 §4-4 — 게이트 꼴은 ADR-0266 「영향」이 메인 판단으로
//! 남겼고, ④~⑥ 의 꼴과 거부한 대안은 ADR-0278 이 기록한다): 이 crate 밖에서 OS `cfg` 술어를 쓰는 `.rs`
//! 파일 목록이 고정 명단과 **정확히** 같아야 한다.
//! - **어디에 있나** — 정규식(`os_cfg_re`) · 파일 찾기 함수(`os_cfg_files`) · 명단(heredoc)은
//!   `.github/workflows/ci.yml` 의 `platform gate 4` 스텝 하나에만 있다. `/qa` 바인딩 4f 블록은 그 셋을 읽되 대상
//!   범위 · 짝 · 대조는 자기 안에 따로 되풀어 적고(스텝의 그쪽을 고치면 함께 맞춘다), 같은 항목의 「저장소 전체」
//!   한 줄은 이 crate 를 빼지 않고 돌린다 — 그 셋은 여기도 다른 문서도 베끼지 않는다. 대상 = `crates` ·
//!   `src-tauri` 의 `.rs` 에서 이 crate 를 뺀 것(`src-tauri/tests/` · `build.rs` 까지 — TRD §4-4 의 `src-tauri/src`
//!   보다 넓고, 재 보면 명단은 같았다). 돌리기 전에 세 경로(`crates` · `src-tauri` · 이 crate `src`)가 있는지
//!   본다 — 없으면 rg 가 사유 없이 죽는다.
//! - **matcher = PCRE2 문법 정규식 하나(`rg -P -U -l` — rg 에 PCRE2 가 없으면 게이트가 그 사유로 FAIL 한다)**.
//!   `(?(DEFINE)…)` 로 낱말 단위(주석 · 문자열 — byte · C 접두 포함 · raw 문자열 `[bc]?r#*"…"#*` · 이름 · 그 밖의
//!   한 글자)를 정해 두고:
//!   - 술어 **밖**에서는 주석 · 문자열 · raw 문자열 · 문자 리터럴을 `(*SKIP)(*FAIL)` 로 건너뛰어 그 안의 `cfg(` 를
//!     안 문다. ★문자열 안 `\` 뒤 줄바꿈(줄 이음)을 받는 갈래를 빼지 말 것★ — 빠지면 그 뒤 따옴표 짝이 뒤집혀
//!     실제 `#[cfg(windows)]` 를 놓친다(discovery 에서 실측).
//!   - `cfg!(…)` · `cfg(…)` · `cfg_attr(…)` 의 술어를 낱말 단위로 **괄호 짝을 맞춰**(재귀) 읽고, 그 안 어느 깊이에든
//!     OS 술어 낱말이 **주석 · 문자열 밖에** 있으면 문다 — 맨 `windows` · `unix`, 또는 `target_os` ·
//!     `target_family` · `target_env` · `target_vendor` 뒤에 공백 · 줄바꿈 · 주석만 끼고 오는 `=`.
//!   - 그래서 `feature = "windows"` · 술어 안 주석의 OS 낱말 · `cfg!(test) && windows()` 의 괄호 밖 낱말 · 속성 값
//!     `windows_subsystem = "windows"` 는 안 걸리고, 몇 겹이든 묶인 술어 · 여러 줄 속성 · 값에 `[` `;` `)` 가 든
//!     문자열 · raw 문자열 · `target_os /* … */ =` · 줄이 갈린 `target_os` 와 `=` 는 걸린다.
//!   - Windows 의 rg 는 경로를 `\` 로 찍으므로 `/` 로 맞춘 뒤 정렬해 명단과 대조한다.
//! - **명단에 드는 부류** = 시험 분기뿐인 파일(`#[cfg(test)]` 모듈 · `#[cfg(all(test, …))]` 항목 아래) ·
//!   `tests/` · 시험 하네스 bin · `examples/` · 위 시한부 운영 예외.
//! - **판정은 양방향이다** — 명단 밖 파일이 걸리면(운영 분기면 이 crate 로 옮기고, 시험 · 하네스면 명단에
//!   더한다) 빨개지고, 명단의 파일이 안 걸려도(분기를 걷었거나 옮겼다 — 명단에서 뺀다) 빨개진다. 뒤쪽을
//!   실패로 두는 것은 명단이 낡지 않게 하려는 것이다.
//! - **짝** — 같은 matcher 가 이 crate 안에서는 1 파일 이상을 물어야 한다. 명단이 다 비는 날 정규식이
//!   망가지면(이스케이프 · 따옴표) 대조가 「둘 다 빈 목록」으로 눈먼다.
//! - ★**한계 — 보증으로 읽지 말 것**★:
//!   - **잡는 것은 명단 밖의 새 파일뿐이다** — 명단에 이미 있는 파일 안에 운영 분기를 더하면 못 잡는다
//!     (리뷰 몫).
//!   - `cfg` 없이 런타임에 OS 를 가르는 자리를 못 본다 — 환경변수 고르기 · 늘 대소문자를 접는 환경변수 이름
//!     비교 같은 것이고, 그 규칙의 자리는 이 crate 의 `env` 다.
//!   - 어휘 분석은 정규식 근사다 — 중첩 블록 주석(`/* /* */ */`)처럼 다루지 않는 꼴이 오면 그 뒤를 잘못 읽어
//!     `cfg` 를 놓칠 수 있다. 술어 안에서 이름 바로 뒤에 `"` · `#` 가 붙는 꼴(리터럴 접두가 아닌데)은 낱말로
//!     못 읽어 그 술어를 통째로 놓친다 — 올바른 Rust 의 `cfg` 술어에는 그런 꼴이 없다.
//!   - `cfg` 라는 이름의 함수 · 메서드 호출도 술어로 읽는다 — 인자에 OS 낱말이 있으면 오탐(파일이 명단에 들
//!     뿐이라 안전한 쪽).
//!   - ★**드문 꼴은 일부러 쫓지 않는다 — 흔한 꼴만 막는 러프한 게이트다**★(사용자 결정 2026-10-06 — 문자열
//!     수준이면 된다). 못 무는 꼴(실측): `cfg_select!`(술어를 `cfg(` 철자 없이 쓴다) · `cfg_aliases`(build
//!     script 가 만든 별칭 `#[cfg(win)]` — 별칭을 정의하는 쪽도 `cfg(` 철자가 없다) · `cfg` 와 `(` 사이의 주석
//!     (`cfg /* … */ (windows)`) · `cfg!{…}` · `cfg![…]`. `cfg_if!` 는 안의 `#[cfg(…)]` 철자로 문다. 19겹 이상
//!     중첩한 술어는 PCRE2 한도에 걸려 rg 가 오류로 끝나 게이트가 FAIL 한다 — 무음 통과는 아니다(로컬 rg
//!     14.1.1 · PCRE2 10.45 실측 — 한도는 rg 빌드마다 다를 수 있다). 이 꼴들을 잡으려고 정규식을 키우지 않는다.
//!
//! **⑤ OS crate 운영 의존은 이 crate 에만**(사용자 결정 2026-10-06 — 위 「의존」 둘째 항의 다른 멤버 쪽 벽):
//! 워크스페이스 멤버 중 OS crate 를 **운영 의존**(normal · build — 대상 OS 무관)으로 **직접** 가진 것이 이 crate
//! 하나여야 한다.
//! - **어디에 있나** — OS crate 목록(`os_dep_re`) · 판정 함수(`os_dep_gate`)는 `.github/workflows/ci.yml` 의
//!   `platform gate 5` 스텝 하나에만 있고, `/qa` 바인딩 4g 블록이 그 둘을 읽어 돌린다. 목록은 손으로 적은 명시
//!   목록이다(cargo-deny 식) — 여기 베끼지 않는다.
//! - **읽는 것** = `cargo tree --workspace --no-dedupe --depth 1 --prefix depth -e normal,build --target all
//!   --all-features` 의 뿌리(멤버)와 그 바로 아래 의존. 멤버를 손으로 적지 않아 새 멤버도 보이고, 해석된
//!   그래프라 rename(패키지 이름으로 찍힌다) · `[target.'cfg(…)'.dependencies]` · `[build-dependencies]` ·
//!   `optional` 을 다 본다(rename 을 단 대상 조건 의존과 build 의존은 결함 주입으로 실측).
//!   ★`--no-dedupe` 를 빼지 말 것★ — 빼면 다른 멤버의 의존으로 먼저 찍힌 멤버가 뿌리 자리에서 `(*)` 로 접혀
//!   자기 의존이 하나도 안 보인다(실측 — 이 crate 의 `windows` 까지).
//! - **dev 의존은 세지 않는다** — 시험 · `examples/` 몫이다(agent 의 `windows` dev = `examples/spike*.rs`).
//! - **짝** — 이 crate 가 1 개 이상 가져야 한다(지금 `windows`). 목록 정규식이나 `cargo tree` 출력 꼴이
//!   망가지면(`--no-dedupe` 를 빼는 것 포함) 판정이 「위반 0」으로 눈먼다.
//! - ★**한계**★ — 직접 의존만 본다: 다른 멤버가 OS API 를 감싼 서드파티(목록 밖)를 들이거나 목록에 없는 OS
//!   crate(`-sys` 형제 · `windows-core` 따위)를 직접 들이면 못 본다. 목록을 넓힐지는 그런 crate 가 들어올 때
//!   정한다.
//!
//! **⑥ `std::os` 의 OS 모듈은 이 crate 안에서만**(사용자 결정 2026-10-06 — 러프한 문자열 게이트): 이 crate
//! 밖에서 `std::os::windows` · `std::os::unix` 를 쓰는 `.rs` 파일 목록이 고정 명단과 **정확히** 같아야 한다.
//! `std` 의 일반 API(`std::fs` · `std::process` 따위)는 OS 에 맞게 알아서 바뀌므로 대상이 아니다.
//! - **어디에 있나** — 정규식(`std_os_re`) · 판정 함수(`std_os_gate` — 대상 범위 · 짝 · 대조 전부) ·
//!   명단(heredoc)은 `ci.yml` 의 `platform gate 6` 스텝 하나에만 있고, `/qa` 바인딩 4h 블록이 그 셋을 읽어 그
//!   함수를 그대로 부른다(④ 와 달리 4h 블록이 되풀어 적는 판정이 없다). 대상 경로 · 경로 확인 ·
//!   경로 구분자 맞춤 · 명단에 드는 부류(시험 분기뿐인 파일 · `tests/` · 시험 하네스 bin · `examples/`) ·
//!   양방향 판정 · 짝(이 crate 안에서 1 파일 이상)은 ④ 와 같다.
//! - **matcher** = 줄 머리부터 「아직 `//` 를 안 만났다」(`^(?:[^/]|/[^/])*?`) 뒤의 `std::os::windows` ·
//!   `std::os::unix`(rg 기본 엔진 — PCRE2 가 필요 없다). `//` · `///` · `//!` 주석은 안 걸리고, `use` 줄과
//!   완전 경로 호출(`::std::os::unix::…::f(…)` 포함)은 걸린다.
//! - ★**한계**★:
//!   - 명단에 이미 든 파일 안의 새 운영 쓰임은 못 잡는다(④ 와 같다 — 리뷰 몫).
//!   - 별칭 · 묶음(`use std::os as o;` · `use std::os::{windows::…};` · `use std::{os::windows::…};`)과 다른 OS
//!     모듈(`std::os::linux` · `std::os::macos` · `std::os::fd` 따위)은 못 본다.
//!   - 블록 주석 · 문자열 안의 그 경로는 걸린다(오탐 — 안전한 쪽). 같은 줄에서 그 경로 앞 문자열에 `//` 가
//!     있으면 놓친다.
//!
//! **하지 않는 것 — macOS 이식 때 다시**(사용자 결정 2026-10-06): 다른 OS 타깃의 `cargo check`(이식성 축 —
//! 조건 없이 쓴 OS 전용 코드를 잡는다. CI 가 Windows 뿐이라 지금은 그런 코드가 통과한다) · clippy
//! `disallowed-*` · Dylint 커스텀 lint · cargo-deny. ④~⑥ 은 이것들을 대신하지 않는다. 조사 =
//! `docs/research/os-boundary-enforcement-2026-10-06.md` · 결정 = ADR-0278 결정 4.
//!
//! ★이 crate 는 생성물을 만들지 않는다★ — serde 도 ts-rs 도 없으므로 `bindings/` 디렉터리가 없다. CI 의
//! ts-rs sync 게이트 경로 목록에 넣지 말 것.

// ADR-0266
// ADR-0275
// ADR-0278
pub mod env;
pub mod file_holders;
pub mod fs;
pub mod group;
pub mod process;
pub mod shell;
pub mod spawn;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
