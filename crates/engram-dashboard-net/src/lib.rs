//! engram-dashboard-net — 데몬의 **네트워크 행**(ADR-0129 결정 1, 슬라이스 1).
//!
//! ★소유하는 것★: 소켓 수락 **뒤의** 연결 살림 — WS 업그레이드 + Origin 허용목록 · 토큰 핸드셰이크 ·
//! 연결 수명 · 연결당 단일 writer · keepalive(능동 Ping·half-open 판정) · out-of-band 종료 신호 ·
//! 전-연결 팬아웃 레지스트리(`ws`) · 위층과의 프레임 포트 계약(`frame_port`) · 프로세스 살림
//! (`instance` 단일 인스턴스 가드 · `portfile` daemon.json 쓰기/읽기·stale 판정 — ADR-0135 이후
//! **둘이 같은 파일 하나**를 다룬다: 가드가 붙잡는 파일이 곧 클라이언트가 읽는 접속 파일이다).
//! 모듈별 책임은 각 파일 헤더가 정본이다.
//! ★그 전부가 `server` feature 뒤에 있고 **기본은 비어 있다**★: 켜지 않으면 남는 것은 `auth` 하나 —
//! **핸드셰이크 프레임 모양만 필요한 소비자**(`discovery` · `src-tauri`)가 async 런타임을 지지 않게
//! 하는 경계다. 서버 행을 쓰는 쪽이 `features = ["server"]` 로 명시해 켠다(데몬 crate). 어떤 의존이 그
//! 뒤로 들어갔는지·왜 기본을 비웠는지·이득의 범위가 어디까지인지는 **Cargo.toml 의 `[features]` 주석이
//! 정본**이다(여기서 목록을 다시 세지 말 것).
//! ★accept **loop 자체**는 아직 여기 없다★: `run_accept_loop` 은 데몬 조립부에 남아 수락과 에이전트
//! 행 조립을 겸한다 — 슬라이스 3(얇은 조립 바이너리)에서 이 crate 로 올 예정이었으나 **그 슬라이스는
//! ADR-0130 으로 보류됐다**(예정이 아니라 재개 시의 목표 모양이다). 그래서 "소켓
//! 수락 뒤" 라고 경계를 그었다(수락 그 자체는 아직 조립부 소관).
//!
//! ★"유일한 구멍" 으로 줄여 적지 말 것(standing 금지)★: 그 축약이 팬아웃 경로를 한 번 가렸다. 표면이
//! 몇이고 어느 방향인지는 ADR-0129 결정 1 + 그 2026-08-04 note 가 정본이고, 실제 trait 과 구현 분담은
//! `frame_port.rs` 헤더에 있다. 여기서 개수를 다시 세지 말 것 —
//! 세 번 세어 세 번 한 칸 모자랐다.
//!
//! ★모르는 것 = 에이전트 어휘 — 예외 0(0-4 완료)★: 프레임에 실린 내용이 무엇인지(명령·이벤트·출력·
//! 메시징)를 **타입으로도** 알지 못한다. 다만 프레임 디코딩이 "전부 위층" 이라는 뜻은 아니다 —
//! **auth 첫 프레임 하나만** 이 crate 가 파싱한다. 그건 어휘 예외가 아니라 **이 crate 가 모양을 소유한
//! 프레임**이기 때문이다(`auth::AuthFrame` — 토큰 인증은 소켓을 살릴지 판정하는 네트워크 살림이다).
//! 그 하나를 뺀 나머지는 규칙 그대로다: 프레임의 의미 해석·인코딩·디스패치는 위층(에이전트 시스템)이
//! 소유하고, 이 crate 는 `frame_port` 계약을 통해서만 그것과 만난다.
//!
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
//!   → 자기 자신 + `base` + `protocol`. 목록에 `discovery` 가 없다 = **이 crate 는 discovery 에 도달하지
//!   않는다**. 그래서 `discovery → 이 crate` 를 더해도 닫히는 고리가 없다.
//!   ★`--target all --all-features` 를 빼지 말 것★: 호스트 target 만 보면 `cfg(unix)` 같은 **비활성 target**
//!   의존과 **optional** 의존이 출력에서 사라진다(실측). 외부 crate 까지 다 찍히면 수백 줄이라 워크스페이스
//!   것만 걸러 본다. `base` 는 워크스페이스 crate 를 하나도 의존하지 않는 잎이라(ADR-0175) 그 아래로
//!   폐포가 더 자라지 않는다.
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
//!   · **게이트 2 — 두 조각이다**(`src/` 범위). ADR-0175 결정 1 이 liveness 헬퍼를 잎 crate 로 옮기면서
//!     기대값이 한 crate 이름에서 다른 crate 이름으로 **자리를 옮겼다** — 총량은 그대로 2 다:
//!     · **2a — base 심볼 allowlist**:
//!     `rg -o --no-filename "engram_dashboard_base::[A-Za-z0-9_:]+" crates/engram-dashboard-net/src/`
//!     `| sort -u` → **정확히 2줄**, 둘 다 `platform::` 아래 프로세스 liveness 헬퍼
//!     (`pid_alive_with_start_time` · `current_process_start_time` — portfile 의 stale 판정 전용).
//!     · **2b — 에이전트 런타임 재유입 금지**:
//!     `rg "engram_dashboard_(a)gent" crates/engram-dashboard-net/src/` → **0줄**
//!     (이 crate 는 에이전트 런타임을 의존으로도 모른다)
//!     ★2b 는 **맨 crate 이름**을 문다 — 심볼 꼬리를 붙이지 말 것★: 0 기대 게이트는 심볼 모양을 요구할
//!     이유가 없고, 꼬리를 붙이면 `::` 뒤가 문자클래스 밖일 때 안 물어 **가장 흔한 두 형태를 놓친다** —
//!     중괄호 전개(`use …::{platform::pid_alive, …}`)와 별칭(`use … as ag;`)이 그대로 빠져나갔다(실측).
//!     ★그 대신 자기일치 방어를 **명시**로 진다★: 이름만 물게 되면 이 헤더가 게이트에 자기 자신을 물리므로
//!     게이트 4 와 같은 `_(첫 글자)` 괄호 형태를 쓴다. 꼬리가 있던 옛 형태는 `::` 뒤에 `[` 가 오는 **우연**
//!     덕에만 자기일치를 면했고, 여기 예시 심볼을 하나 적는 순간 죽는 방어였다 — 그래서 우연을 규칙으로
//!     바꿨다. 2a 는 아직 그 우연 위에 서 있다(꼬리가 문자클래스라 필요하다 — 개수를 세는 게이트다).
//!     ★2b 가 사는 값어치를 부풀리지 말 것★: 살아 있는 벽은 게이트 3 이다 — net 은 의존을 선언하지 않고는
//!     에이전트 심볼을 부를 수조차 없어 그쪽이 먼저 빨개진다. 2b 가 맡는 것은 **주석·테스트 헬퍼로 어휘가
//!     먼저 새는 것**과, 게이트가 제 이름값(`0줄`)대로 도는 것이다.
//!     ★그래도 2b 를 **지우지 말 것**★: 그것만 떼면 base 쪽 2 기대가 홀로 남아 "심볼이 사라진 게 아니라
//!     옮겨 갔다" 를 아무도 단언하지 않는다. 짝으로 서야 벽이다.
//!     ★기대값을 **파일 이름**으로 두지 말 것★: "`portfile.rs` 만" 은 그 파일 **안에** 에이전트 어휘
//!     import 를 새로 넣어도 여전히 참이라 게이트가 통과한다. 불변식은 파일 단위가 아니라 **심볼 단위**다
//!     (ADR-0129 — 격리는 crate 단위가 아니라 타입 단위).
//!   · **게이트 3 — 직접 워크스페이스 의존 상한**(해석된 의존 그래프 범위):
//!     `cargo tree -p engram-dashboard-net --depth 1 --prefix none -e normal,dev,build --target all`
//!     `--all-features | rg "^engram-dashboard" | sort -u`
//!     → **정확히 3줄** = 자기 자신 · `engram-dashboard-base` · `engram-dashboard-protocol`.
//!     ★ADR-0175 결정 1 뒤에도 기대값은 3 그대로다★ — 둘째 자리가 `agent` 에서 `base` 로 갈렸을 뿐
//!     개수는 안 변했다. 줄이려 들지 말 것.
//!     ★이 게이트가 닫는 것(딱 이만큼)★: **선언된 직접 워크스페이스 의존**이 매니페스트 문법에 관계없이
//!     드러난다 — rename · `[dependencies.<이름>]` 테이블 형 · 들여쓴 선언 · `[build-dependencies]` ·
//!     비활성 target(`cfg(unix)`) · `optional` 6형태를 **주입→관측→원복으로 실측**해 전부 잡는 것을 확인했다.
//!     ★닫지 **않는** 것★: 허용된 간선(protocol·base) **안에서** 어떤 심볼을 쓰는지는 보지 않는다 —
//!     base 쪽은 게이트 2가 묶고, protocol 쪽은 게이트 4가 **두 이름만** 못 박는다. 즉 protocol 심볼
//!     **일반**의 allowlist 는 여전히 없다(알려진 공백 — ADR-0129 §영향/불변식의 2026-08-05 note 안
//!     ★알려진 공백★ 절, 의도적 유보. 그 ADR 에 "Note A" 라는 라벨은 없다). 게이트 4를
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
//!     ★주석·테스트까지 범위다(의도)★: 컴파일러는 `#[cfg(test)]` 안의 재유입도 잡지만 주석은 못 잡고,
//!     테스트가 위층 타입으로 형태를 대조하기 시작하면 그게 곧 재유입의 첫 걸음이다.
//!     ★부분 문자열까지 문다(의도 — 좁히지 말 것)★: 단어 경계를 안 걸어서 `TEST_(P)ROTOCOL_VERSION` 같은
//!     **지역 이름**도 걸린다(실측 2026-08-05 — 이 crate 의 테스트 상수가 처음 그 이름이었다). 넓은 게
//!     맞다: 이 crate 안엔 프로토콜 버전 비슷한 것조차 두지 않겠다는 불변식이고, 걸리면 이름을 바꾸는
//!     쪽이 대응이다(`ws.rs` 의 `TEST_WIRE_VERSION`). `\b` 로 좁히면 그 규율이 사라진다.
//!   · **게이트 5 — 두 feature 조합이 각각 컴파일된다**(`-p` 범위, `default = []` 전환이 추가):
//!     `cargo test -p engram-dashboard-net` → **성공해야 PASS**(feature 0개 = `auth` 단독 + golden 테스트)
//!     `cargo test -p engram-dashboard-net --all-features` → **성공해야 PASS**(`server` 행)
//!     ★판정 방식이 위 넷과 다르다★: 1·4 는 매치 유무로, 2·3 은 줄 수로 읽지만 이건 **성공 여부**로 읽는다.
//!     ★왜 두 줄인가★: 기본이 비어 있어 맨 명령은 feature 0개만 컴파일하고, `#[cfg(feature = "server")]`
//!     아래 모듈은 **컴파일 대상에서 빠진다**(가려진 코드는 오류를 내지 않는다) — 실측 2026-08-14 로 맨
//!     명령은 **6개**, `--all-features` 는 **42개**를 돌린다. 반대 방향으로, 워크스페이스 스코프 명령은
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
//! 않는다.

pub mod auth;
#[cfg(feature = "server")]
pub mod frame_port;
#[cfg(feature = "server")]
pub mod instance;
#[cfg(feature = "server")]
pub mod portfile;
#[cfg(feature = "server")]
pub mod ws;
