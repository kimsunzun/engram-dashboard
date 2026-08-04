//! engram-dashboard-net — 데몬의 **네트워크 행**(ADR-0129 결정 1, 슬라이스 1).
//!
//! ★소유하는 것★: 소켓 수락 **뒤의** 연결 살림 — WS 업그레이드 + Origin 허용목록 · 토큰 핸드셰이크 ·
//! 연결 수명 · 연결당 단일 writer · keepalive(능동 Ping·half-open 판정) · out-of-band 종료 신호 ·
//! 전-연결 팬아웃 레지스트리(`ws`) · 위층과의 프레임 포트 계약(`frame_port`) · 프로세스 살림
//! (`instance` 단일 인스턴스 가드 · `portfile` daemon.json IO/stale 판정). 모듈별 책임은 각 파일
//! 헤더가 정본이다.
//! ★accept **loop 자체**는 아직 여기 없다★: `run_accept_loop` 은 데몬 조립부에 남아 수락과 에이전트
//! 행 조립을 겸한다 — 슬라이스 3(얇은 조립 바이너리)에서 분해되며 이 crate 로 온다. 그래서 "소켓
//! 수락 뒤" 라고 경계를 그었다(수락 그 자체는 아직 조립부 소관).
//!
//! ★"유일한 구멍" 으로 줄여 적지 말 것(standing 금지)★: 그 축약이 팬아웃 경로를 한 번 가렸다. 표면이
//! 몇이고 어느 방향인지는 ADR-0129 결정 1 + 그 2026-08-04 note 가 정본이고, 실제 trait 과 구현 분담은
//! 아래 `pub mod frame_port;` 주석과 `frame_port.rs` 헤더에 있다. 여기서 개수를 다시 세지 말 것 —
//! 세 번 세어 세 번 한 칸 모자랐다.
//!
//! ★모르는 것 = 에이전트 어휘 — 단 예외 1건★: 프레임에 실린 내용이 무엇인지(명령·이벤트·출력·메시징)를
//! **타입으로도** 알지 못한다. **단 `ws.rs` 의 auth 핸드셰이크가 쓰는 `AgentCommand::Auth`·
//! `PROTOCOL_VERSION` 두 이름은 0-4 이월분으로 예외**이고(아래 ★0-4 이월★ 절), 그 결과 프레임 디코딩도
//! "전부 위층" 이 아니다 — **auth 첫 프레임 하나만** 이 crate 가 파싱한다. 그 하나를 뺀 나머지는 규칙
//! 그대로다: 프레임의 의미 해석·인코딩·디스패치는 위층(에이전트 시스템)이 소유하고, 이 crate 는
//! `frame_port` 계약을 통해서만 그것과 만난다.
//!
//! ★출신(ADR-0129 슬라이스 1)★: 네 모듈은 데몬 crate `src/` 에서 **그대로 이사**했다(로직·모듈명·
//! 타입명 무변경 — 순수 이사 + 배선 수정). 슬라이스 0 이 먼저 두 행 사이의 호출을 포트로만 흐르게
//! 끊어 뒀기 때문에(0-1 포트 계약 · 0-2 어휘 이사 · 0-5 팬아웃 포트) 이 슬라이스가 파일 이동으로
//! 끝났다. 남은 순서는 0-4(인증 이사) → 슬라이스 2(에이전트 시스템 lib) → 슬라이스 3(얇은 조립).
//!
//! ★★0-4 이월 — ADR-0129 결정 1 불변식은 이 구간만 의도적 미충족(근거 step-log S18.20)★★
//! `ws.rs` 의 auth 핸드셰이크가 `AgentCommand::Auth`/`PROTOCOL_VERSION` 을 protocol crate 에서 가져온다.
//! 인증 타입 이사(0-4)를 슬라이스 1 **뒤로** 미룬 것은 **사용자의 순서 결정**이다(step-log S18.20).
//!
//! ★막고 있던 것이 무엇이었나 — 지금은 순환이 **없다**★: 슬라이스 1 **전에는** 네트워크 행이 데몬
//! crate 안에 있었으므로 인증 타입도 `daemon` 에 놓였을 것이고, 인증 프레임을 각자 만드는 `discovery`·
//! `src-tauri` 때문에 `discovery → daemon` 이 필요해져 기존 `daemon → discovery`(경로 헬퍼)와 **순환**을
//! 이뤘다. 네트워크 행이 자기 crate 로 떨어진 **지금은 `discovery → 이 crate` 가 비순환**이다.
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
//!   명령으로는 순환 유무를 구별할 수 없다. 0-4 세션이 이 레시피에 기댈 것이라 특히 중요하다.
//! 즉 **슬라이스 1 의 완료가 바로 0-4 를 푸는 것**이다. 이 자리에 "지금도 순환이다" 라고 적지 말 것 —
//! 그 문장은 슬라이스 1 이 끝난 시점에 거짓이 된다.
//!
//! 그래도 **이 import 를 회귀로 적출하지 말 것**이고, 걷어내거나 감싸거나 추상화하지도 말 것 —
//! 0-4 가 핸드셰이크 타입을 데몬 소유로 옮기면 저절로 사라진다.
//!
//! ★격리 게이트(컴파일러가 먼저 잡고 grep 이 주석·테스트 헬퍼로 새는 경로를 잡는다 — ADR-0110 과 같은
//! 형태)★. 소스를 훑는 게이트 1·2 는 패턴을 `_(이름)`·`_(문자클래스)` **괄호 형태로** 적는다 — 맨 이름으로
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
//!     core 쪽은 게이트 2가 묶지만 **protocol 쪽은 무게이트**다(알려진 공백 — ADR-0129 Note A).
//!     ★매니페스트 **텍스트**를 grep 하지 말 것★: 위 6형태 중 다섯이 정상 Cargo 문법으로 텍스트 grep 을
//!     빠져나가고, 소스에서 이름을 안 부르면 게이트 1·2 도 통과한다. `cargo tree` 는 **해석된 패키지
//!     identity** 를 찍으므로 전부 실제 이름으로 드러난다. 플래그를 줄이면 그만큼 형태가 새므로
//!     (`-e build`↔build-deps · `--target all`↔비활성 target · `--all-features`↔optional) 줄이지 말 것.
//!     기대값을 늘리기 전에 Cargo.toml 의 의존성 상한 규칙을 먼저 읽을 것.
//!
//! ★단독 검증(ADR-0012)★: `cargo test -p engram-dashboard-net` 가 이 crate 만으로 돈다 — WS 서버·프레임
//! 포트 테스트는 실소켓(127.0.0.1:0) 또는 합성 프레임열로 돌고 에이전트 시스템 실물을 쓰지 않는다.
//! 그 crate 들이 **직접 의존으로 선언돼 있지 않다**는 것은 게이트 3(해석된 의존 그래프)이, 소스가 이름조차
//! 부르지 않는다는 것은 게이트 1이 못 박는다.
// ADR-0129

// 불투명 프레임 포트. 계약(trait)은 이 crate 가 소유하고 실물은 양쪽이 나눠 꽂는다 —
//   `FrameSink`/`FrameFanout` 은 `ws` 가, `ConnectionHandler`/`ConnectionHandlerFactory` 는 위층이.
pub mod frame_port;
pub mod instance;
pub mod portfile;
pub mod ws;
