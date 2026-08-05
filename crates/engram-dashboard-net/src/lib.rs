//! engram-dashboard-net — 데몬의 **네트워크 행**(ADR-0129 결정 1, 슬라이스 1).
//!
//! ★소유하는 것★: 소켓 수락 **뒤의** 연결 살림 — WS 업그레이드 + Origin 허용목록 · 토큰 핸드셰이크 ·
//! 연결 수명 · 연결당 단일 writer · keepalive(능동 Ping·half-open 판정) · out-of-band 종료 신호 ·
//! 전-연결 팬아웃 레지스트리(`ws`) · 위층과의 프레임 포트 계약(`frame_port`) · 프로세스 살림
//! (`instance` 단일 인스턴스 가드 · `portfile` daemon.json IO/stale 판정). 모듈별 책임은 각 파일
//! 헤더가 정본이다.
//! ★그 전부가 `server` feature 뒤에 있고 **기본은 비어 있다**★: 켜지 않으면 남는 것은 `auth` 하나 —
//! **핸드셰이크 프레임 모양만 필요한 소비자**(`discovery` · `src-tauri`)가 async 런타임을 지지 않게
//! 하는 경계다. 서버 행을 쓰는 쪽이 `features = ["server"]` 로 명시해 켠다(데몬 crate). 어떤 의존이 그
//! 뒤로 들어갔는지·왜 기본을 비웠는지·이득의 범위가 어디까지인지는 **Cargo.toml 의 `[features]` 주석이
//! 정본**이다(여기서 목록을 다시 세지 말 것).
//! ★accept **loop 자체**는 아직 여기 없다★: `run_accept_loop` 은 데몬 조립부에 남아 수락과 에이전트
//! 행 조립을 겸한다 — 슬라이스 3(얇은 조립 바이너리)에서 분해되며 이 crate 로 온다. 그래서 "소켓
//! 수락 뒤" 라고 경계를 그었다(수락 그 자체는 아직 조립부 소관).
//!
//! ★"유일한 구멍" 으로 줄여 적지 말 것(standing 금지)★: 그 축약이 팬아웃 경로를 한 번 가렸다. 표면이
//! 몇이고 어느 방향인지는 ADR-0129 결정 1 + 그 2026-08-04 note 가 정본이고, 실제 trait 과 구현 분담은
//! 아래 `pub mod frame_port;` 주석과 `frame_port.rs` 헤더에 있다. 여기서 개수를 다시 세지 말 것 —
//! 세 번 세어 세 번 한 칸 모자랐다.
//!
//! ★모르는 것 = 에이전트 어휘 — 예외 0(0-4 완료)★: 프레임에 실린 내용이 무엇인지(명령·이벤트·출력·
//! 메시징)를 **타입으로도** 알지 못한다. 다만 프레임 디코딩이 "전부 위층" 이라는 뜻은 아니다 —
//! **auth 첫 프레임 하나만** 이 crate 가 파싱한다. 그건 어휘 예외가 아니라 **이 crate 가 모양을 소유한
//! 프레임**이기 때문이다(`auth::AuthFrame` — 토큰 인증은 소켓을 살릴지 판정하는 네트워크 살림이다).
//! 그 하나를 뺀 나머지는 규칙 그대로다: 프레임의 의미 해석·인코딩·디스패치는 위층(에이전트 시스템)이
//! 소유하고, 이 crate 는 `frame_port` 계약을 통해서만 그것과 만난다.
//!
//! ★출신(ADR-0129 슬라이스 1)★: 네 모듈은 데몬 crate `src/` 에서 **그대로 이사**했다(로직·모듈명·
//! 타입명 무변경 — 순수 이사 + 배선 수정). 슬라이스 0 이 먼저 두 행 사이의 호출을 포트로만 흐르게
//! 끊어 뒀기 때문에(0-1 포트 계약 · 0-2 어휘 이사 · 0-5 팬아웃 포트) 이 슬라이스가 파일 이동으로
//! 끝났다. 그 뒤 0-4(인증 이사)가 마지막 어휘 구멍을 닫았다. 남은 순서는 슬라이스 2(에이전트 시스템
//! lib) → 슬라이스 3(얇은 조립).
//!
//! ★★0-4 완료 — 결정 1 불변식이 이제 성립한다★★
//! 옛 상태: `ws.rs` 의 auth 핸드셰이크가 위층 명령 enum 의 auth variant 와 버전 상수를 protocol crate
//! 에서 가져왔다(슬라이스 1 구간의 의도적 이월 — 근거 step-log S18.20, 사용자의 순서 결정).
//! 지금: 프레임 모양은 `auth::AuthFrame`(이 crate 소유), 기대 버전은 `handle_connection` 이 **값으로
//! 주입**받는다. 그래서 두 이름 다 `src/` 에 없고, 그 상태를 **게이트 4** 가 못 박는다.
//! ★버전 상수 자체는 protocol 에 남는다★ — daemon.json 의 `DaemonInfo` 계약이기도 해서 discovery 가
//! portfile 로 읽는다. 여기로 끌고 오지 말 것(그건 인증과 무관한 별개 계약이다).
//!
//! ★왜 이 crate 였나 — 순환 때문★: 슬라이스 1 **전에는** 네트워크 행이 데몬 crate 안에 있었으므로
//! 인증 타입도 `daemon` 에 놓였을 것이고, 인증 프레임을 각자 만드는 `discovery`·`src-tauri` 때문에
//! `discovery → daemon` 이 필요해져 기존 `daemon → discovery`(경로 헬퍼)와 **순환**을 이뤘다. 네트워크
//! 행이 자기 crate 로 떨어져 `discovery → 이 crate` 가 비순환이 된 것이 0-4 를 푼 열쇠다(실제로 0-4 가
//! `discovery → 이 crate` · `engram-dashboard → 이 crate` 두 간선을 새로 놨다).
//! ★확인 레시피 — **워크스페이스-로컬 forward 폐포**를 볼 것★:
//!   `cargo tree -p engram-dashboard-net -e normal --target all --all-features --prefix none`
//!   `| rg "^engram-dashboard" | sort -u`
//!   → 자기 자신 + `core` + `protocol`. 목록에 `discovery` 가 없다 = **이 crate 는 discovery 에 도달하지
//!   않는다**. 그래서 `discovery → 이 crate` 를 더해도 닫히는 고리가 없다.
//!   ★`--target all --all-features` 를 빼지 말 것★: 호스트 target 만 보면 `cfg(unix)` 같은 **비활성 target**
//!   의존과 **optional** 의존이 출력에서 사라진다(실측). 외부 crate 까지 다 찍히면 수백 줄이라 워크스페이스
//!   것만 걸러 본다. `core → protocol` 은 core 의 dev-의존이라 `-e normal` 엔 나오지 않는데, 결론에는
//!   영향이 없다 — 없는 간선은 도달을 늘리지 못한다.
//!   ★`--invert` 를 쓰지 말 것(unsound)★: invert 는 이 crate 의 **조상**을 나열하는데, 순환 성립 조건은
//!   `이 crate →* discovery` 이므로 순환이 있는 세계에서도 invert 출력은 **바이트 동일**하다 — 즉 그
//!   명령으로는 순환 유무를 구별할 수 없다.
//!
//! ★격리 게이트(컴파일러가 먼저 잡고 grep 이 주석·테스트 헬퍼로 새는 경로를 잡는다 — ADR-0110 과 같은
//! 형태)★. 소스를 훑는 게이트 1·2·4 는 패턴을 `_(이름)`·`_(문자클래스)` **괄호 형태로** 적는다 — 맨 이름으로
//! 적으면 이 헤더가 게이트에 자기 자신을 물리기 때문이다(같은 함정을 코어 tauri 게이트가 이미 밟았다,
//! 2026-07-13). 게이트 3 은 파일을 읽지 않고 해석된 의존 그래프를 보므로 그 문제가 아예 없다.
//! 게이트를 손보면 **편집 후 비자기일치 성질을 다시 실측할 것**:
//!   · **게이트 1 — 소스 참조**(`src/` 범위):
//!     `rg "engram_dashboard_(daemon|messaging|discovery)" crates/engram-dashboard-net/src/` → **0줄**
//!   · **게이트 2 — core 심볼 allowlist**(`src/` 범위):
//!     `rg -o --no-filename "engram_dashboard_core::[A-Za-z0-9_:]+" crates/engram-dashboard-net/src/`
//!     `| sort -u` → **정확히 2줄**, 둘 다 `agent::platform::` 아래 프로세스 liveness 헬퍼
//!     (`pid_alive_with_start_time` · `current_process_start_time` — portfile 의 stale 판정 전용).
//!     ★기대값을 **파일 이름**으로 두지 말 것★: "`portfile.rs` 만" 은 그 파일 **안에** 에이전트 어휘
//!     import 를 새로 넣어도 여전히 참이라 게이트가 통과한다. 불변식은 파일 단위가 아니라 **심볼 단위**다
//!     (ADR-0129 — 격리는 crate 단위가 아니라 타입 단위).
//!   · **게이트 3 — 직접 워크스페이스 의존 상한**(해석된 의존 그래프 범위):
//!     `cargo tree -p engram-dashboard-net --depth 1 --prefix none -e normal,dev,build --target all`
//!     `--all-features | rg "^engram-dashboard" | sort -u`
//!     → **정확히 3줄** = 자기 자신 · `engram-dashboard-core` · `engram-dashboard-protocol`.
//!     ★이 게이트가 닫는 것(딱 이만큼)★: **선언된 직접 워크스페이스 의존**이 매니페스트 문법에 관계없이
//!     드러난다 — rename · `[dependencies.<이름>]` 테이블 형 · 들여쓴 선언 · `[build-dependencies]` ·
//!     비활성 target(`cfg(unix)`) · `optional` 6형태를 **주입→관측→원복으로 실측**해 전부 잡는 것을 확인했다.
//!     ★닫지 **않는** 것★: 허용된 간선(protocol·core) **안에서** 어떤 심볼을 쓰는지는 보지 않는다 —
//!     core 쪽은 게이트 2가 묶고, protocol 쪽은 게이트 4가 **두 이름만** 못 박는다. 즉 protocol 심볼
//!     **일반**의 allowlist 는 여전히 없다(알려진 공백 — ADR-0129 Note A, 의도적 유보). 게이트 4를
//!     그 일반 allowlist 로 키우지 말 것 — 그건 별도 결정이다(0-4 범위 밖).
//!     ★매니페스트 **텍스트**를 grep 하지 말 것★: 위 6형태 중 다섯이 정상 Cargo 문법으로 텍스트 grep 을
//!     빠져나가고, 소스에서 이름을 안 부르면 게이트 1·2 도 통과한다. `cargo tree` 는 **해석된 패키지
//!     identity** 를 찍으므로 전부 실제 이름으로 드러난다. 플래그를 줄이면 그만큼 형태가 새므로
//!     (`-e build`↔build-deps · `--target all`↔비활성 target · `--all-features`↔optional) 줄이지 말 것.
//!     기대값을 늘리기 전에 Cargo.toml 의 의존성 상한 규칙을 먼저 읽을 것.
//!   · **게이트 4 — auth 어휘 재유입 금지**(`src/` 범위, 0-4 가 추가):
//!     `rg "(A)gentCommand|(P)ROTOCOL_VERSION" crates/engram-dashboard-net/src/` → **0줄**
//!     ★무엇을 지키나★: 0-4 가 닫은 마지막 어휘 구멍(핸드셰이크가 위층 명령 enum 과 버전 상수를 빌려
//!     쓰던 것)이 **다시 열리는 것**을 막는다. 프레임 모양은 `auth::AuthFrame` 이 소유하고 기대 버전은
//!     `ws::handle_connection` 이 주입받으므로, 이 두 이름이 `src/` 에 다시 나타난다 = 되돌아간 것이다.
//!     ★괄호 형태인 이유★: 맨 이름으로 적으면 **이 줄이 게이트에 걸려** 영원히 1줄이 나온다(위 서두의
//!     자기일치 함정 — 코어 tauri 게이트가 2026-07-13 에 실제로 밟았다). 편집하면 비자기일치를 다시 실측할 것.
//!     ★주석·테스트까지 범위다(의도)★: 컴파일러는 `#[cfg(test)]` 안의 재유입도 잡지만 주석은 못 잡고,
//!     테스트가 위층 타입으로 형태를 대조하기 시작하면 그게 곧 재유입의 첫 걸음이다(그래서 `auth.rs` 의
//!     형태 테스트는 **golden JSON 문자열**로 되어 있다 — 타입 대조가 아니다).
//!     ★부분 문자열까지 문다(의도 — 좁히지 말 것)★: 단어 경계를 안 걸어서 `TEST_(P)ROTOCOL_VERSION` 같은
//!     **지역 이름**도 걸린다(실측 2026-08-05 — 이 crate 의 테스트 상수가 처음 그 이름이었다). 넓은 게
//!     맞다: 이 crate 안엔 프로토콜 버전 비슷한 것조차 두지 않겠다는 불변식이고, 걸리면 이름을 바꾸는
//!     쪽이 대응이다(`ws.rs` 의 `TEST_WIRE_VERSION`). `\b` 로 좁히면 그 규율이 사라진다.
//!     ★이 문단도 괄호 형태로 적혀 있다★ — 게이트를 **설명하는 글**조차 게이트에 걸린다(실측: 처음엔
//!     맨 이름으로 적었다가 그대로 물렸다). 서두의 자기일치 경고가 예시 하나를 더 얻은 셈이다.
//!   · **게이트 5 — 두 feature 조합이 각각 컴파일된다**(`-p` 범위, `default = []` 전환이 추가):
//!     `cargo test -p engram-dashboard-net` → **성공해야 PASS**(feature 0개 = `auth` 단독 + golden 테스트)
//!     `cargo test -p engram-dashboard-net --all-features` → **성공해야 PASS**(`server` 행)
//!     ★판정 방식이 위 넷과 다르다★: 1·4 는 매치 유무로, 2·3 은 줄 수로 읽지만 이건 **성공 여부**로 읽는다.
//!     ★왜 두 줄인가★: 기본이 비어 있어 맨 명령은 feature 0개만 컴파일하고, `#[cfg(feature = "server")]`
//!     아래 모듈은 **컴파일 대상에서 빠진다**(가려진 코드는 오류를 내지 않는다) — 실측 2026-08-05 로 맨
//!     명령은 **6개**, `--all-features` 는 **31개**를 돌린다. 반대 방향으로, 워크스페이스 스코프 명령은
//!     데몬이 `server` 를 켜므로 항상 ON 쪽만 본다 — `-p` 범위에서 ON 쪽을 컴파일하는 명령은 둘째 줄뿐이다.
//!     각 줄이 상대가 못 보는 쪽을 맡으므로 한 줄로 줄이지 말 것.
//!     ★`--features server` 가 아니라 `--all-features` 인 이유★: 게이트 3 이 이미 그 플래그를 쓰므로 net
//!     게이트의 ON 쪽 표기를 하나로 맞춘다. feature 가 `server` 하나뿐인 지금 두 형태는 같은 집합이다.
//!     ★`build` 가 아니라 `test` 인 이유★: dev-의존 경로까지 문다 — `auth.rs` 의 golden 테스트가 쓰는
//!     `serde_json` 은 `[dependencies]` 쪽에선 optional 이라 feature 0개 빌드에 없다.
//!     ★이 게이트가 닫지 **않는** 것★: 새 의존을 `optional` 없이 적어 feature 0개 소비자에게 조용히
//!     지우는 형태는 컴파일을 깨지 않으므로 여기서 안 걸린다 — 근거·대안 게이트는 Cargo.toml 의
//!     `[features]` 주석.
//!
//! ★단독 검증(ADR-0012)★: `cargo test -p engram-dashboard-net --all-features` 가 이 crate 만으로 돈다 —
//! WS 서버·프레임 포트 테스트는 실소켓(127.0.0.1:0) 또는 합성 프레임열로 돌고 에이전트 시스템 실물을 쓰지
//! 않는다. ★`--all-features` 를 빼지 말 것★: 기본이 비어 있어 맨 `cargo test -p engram-dashboard-net` 은
//! 서버 행을 아예 컴파일하지 않고 초록을 낸다(게이트 5 의 첫 줄이 바로 그 조합이다 — 서로 다른 것을 본다).
//! 그 crate 들이 **직접 의존으로 선언돼 있지 않다**는 것은 게이트 3(해석된 의존 그래프)이, 소스가 이름조차
//! 부르지 않는다는 것은 게이트 1이 못 박는다.
// ADR-0129

// 토큰 핸드셰이크 프레임(0-4). 이 crate 가 **모양을 소유**하는 유일한 프레임이고, 발신자(트레이 stop ·
//   CLI · 데몬 클라이언트 셸 · 프론트 손조립 JSON)가 전부 이 모양을 만든다 — wire 는 얼려 있다.
pub mod auth;
// ↓ 여기부터 서버 행 — `server` feature 가 켠다(기본은 비어 있고, 데몬 crate 가 명시로 켠다). `#[cfg]` 로
//   가린 코드는 **가려진 조합에서 컴파일 오류가 보이지 않는다**는 성질이 있으므로, 이 모듈들의 컴파일은
//   feature 를 켜고 도는 경로가 지킨다 — 워크스페이스 스코프(`cargo build` 루트 · `cargo test --workspace`)와
//   `-p` 스코프의 게이트 5 둘째 줄(위 헤더).
// 불투명 프레임 포트. 계약(trait)은 이 crate 가 소유하고 실물은 양쪽이 나눠 꽂는다 —
//   `FrameSink`/`FrameFanout` 은 `ws` 가, `ConnectionHandler`/`ConnectionHandlerFactory` 는 위층이.
#[cfg(feature = "server")]
pub mod frame_port;
#[cfg(feature = "server")]
pub mod instance;
#[cfg(feature = "server")]
pub mod portfile;
#[cfg(feature = "server")]
pub mod ws;
