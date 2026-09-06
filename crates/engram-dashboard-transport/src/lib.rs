//! # engram-dashboard-transport — 재사용 전송 lib (ADR-0177)
//!
//! ★소유하는 것★: 프레임 경계 · 요청/응답 상관 · 역할 구분 · 스트림 살림(세대·순번·이어받기·중복
//! 제거·**구멍 찾기와 그 지점부터의 재요청**) · 연결 수립과 **불투명 핸드셰이크** · **양방향 keepalive** ·
//! **쓰기 시한** · **백오프+지터 재연결** · **상대 N 개 명부와 상대마다 감독** · 팬아웃 · 유계 큐와 배압
//! 신호 · **연결 사건 구조화 방출**(ADR-0182).
//!
//! ★모르는 것 = 소비자의 패킷 어휘★ — [`Wire`] 의 연관 타입 뒤로 전부 가려져 있다. 봉투도 짝짓기 번호
//! 생성도 주소 파싱도 버전 협상도 이 crate 밖이다(ADR-0177 결정 4·5).
//!
//! ## 정본 요약
//!
//! ```text
//! ① 타입은 제네릭, 자원은 dyn.
//!      제네릭 = Wire(소비자의 패킷 어휘)
//!      dyn    = Dialer/Link(전송) · Clock(시간)
//! ② 순수 층과 구동 층을 파일로 가른다(아래 게이트 3 이 그 벽이다).
//! ③ 상대는 처음부터 N 개다.
//! ④ 위로 올라가는 것은 Incoming 한 줄뿐이다 — 옆 채널을 두지 않는다.
//! ```
//!
//! ## 진입점
//!
//! [`Registry::new`] 가 명부와 [`Inbound`] 를 낸다 · [`Registry::add`] 가 상대를 올리고 [`Peer`] 를 준다 ·
//! [`Peer::request`]/[`Peer::notify`]/[`Peer::resume_stream`]/[`Peer::reconnect_now`]/[`Peer::close`] 가
//! 조작 표면 · [`Inbound::recv`] 가 유일한 수신 표면.
//!
//! ★[`machine`]·[`pending`]·[`stream`] 이 공개인 것은 의도★ — 순수 층이라 소비자가 자기 하네스에서
//! 그대로 돌려 볼 수 있다(ADR-0012). **지지되는 표면**은 위 진입점이고, 순수 층은 관측용이다.
//!
//! ## 격리 게이트 — 넷이고 각각 다른 축이다
//!
//! **① 직접 워크스페이스 의존 상한**(해석된 의존 그래프 범위):
//! `cargo tree -p engram-dashboard-transport --depth 1 --prefix none -e normal,dev,build --target all`
//! `--all-features | rg "^engram-dashboard" | sort -u` → **정확히 1줄 = 자기 자신**.
//! ★매니페스트 텍스트를 grep 으로 바꾸지 말 것★ — rename · `[dependencies.<이름>]` 테이블 형 ·
//! `[build-dependencies]` · 비활성 target · `optional` 이 전부 정상 Cargo 문법으로 빠져나간다. 플래그도
//! 줄이지 않는다(`command`·`base`·`messaging` 상한 게이트와 같은 근거).
//! ★공통으로 남는 구멍★ — 멤버를 `engram-dashboard` **이름 접두**로 식별하므로 다른 이름을 단 멤버는
//! 그냥 통과한다. 그래서 이 crate 의 패키지 이름에서 그 접두를 떼면 게이트가 조용히 눈을 감는다.
//!
//! **② Tauri import 0**(ADR-0003):
//! `rg "^\s*use tauri" crates/engram-dashboard-transport/src/` → **0줄**.
//! ★①이 이 축을 덮는다고 읽지 말 것★ — ①은 워크스페이스 멤버만 세므로 서드파티 `tauri` 는 그대로
//! 통과한다. 두 게이트는 겹치지 않는다.
//!
//! **③ 순수 층 격리**(`pending.rs`·`stream.rs`·`machine.rs` 범위):
//! `rg "^(?:[^/]|/[^/])*?\b(tokio|futures_util|futures|engram_dashboard_[a-z_]+|std::net)::"`
//! `crates/engram-dashboard-transport/src/pending.rs crates/engram-dashboard-transport/src/stream.rs`
//! `crates/engram-dashboard-transport/src/machine.rs` → **0줄**.
//! ★`use` 라인만 앵커하면 인라인 완전경로 한 줄에 뚫린다★ — `tokio::spawn(…)` 처럼 본문에 완전경로로
//! 쓰면 import 가 안 생긴다. 접두 `(?:[^/]|/[^/])*?` 는 「아직 `//` 를 안 만났다」를 뜻해 **주석 밖**만
//! 잡고, 그래서 이 헤더의 자기인용이 오탐되지 않는다(선례 = `replay_flight` 순수성 게이트, ADR-0175 결정 3).
//! ★이 게이트에는 컴파일러 backstop 이 없다★ — 같은 crate 안이라 `tokio` 한 줄이 그냥 컴파일된다.
//! 이 정규식이 **유일한 벽**이다. ★파일이 사라져도 매치는 0이라 통과로 읽힌다★ — 이름을 바꾸거나
//! 옮긴 뒤엔 `test -f` 로 세 파일의 존재를 먼저 본다.
//! ★TRD §9-2 의 초안은 `machine.rs` 만 겨눴다★ — 파일표는 셋 다 ★순수★로 표시하므로 셋으로 넓혔다.
//!
//! **④ 두 feature 조합이 각각 컴파일된다**(`-p` 범위):
//! `cargo test -p engram-dashboard-transport` → **성공해야 PASS**(feature 0개)
//! `cargo test -p engram-dashboard-transport --all-features` → **성공해야 PASS**(`test-support` 포함)
//! ★판정 방식이 위 셋과 다르다★ — ①·③ 은 줄 수로, ②는 매치 유무로 읽지만 이건 **성공 여부**로 읽는다.
//! 가려진 코드는 오류를 내지 않으므로 한 줄로 줄이지 말 것.
//!
//! ## 알려진 한계 (보증으로 읽지 말 것)
//!
//! - ★**늦은 답장의 오배달을 불가능하게 만들지 못한다 — 그리고 만들려 하지 않는다**★. 소비자가
//!   [`Wire::Tag`] 를 **되쓰는** 타입으로 고르면 이 crate 는 그것을 막을 수 없다. 기제: 시한 만료로
//!   걷힌 번호를 **FIFO 유계 링**([`Policy::retired_tags`], 기본 256)에 넣어 두고 다시 오면
//!   [`RequestError::TagRetired`] 로 거절한다. 그 뒤로 256개가 더 은퇴하면 가장 오래된 번호는
//!   **잊히고**, ★그 뒤 도착한 그 번호의 늦은 답장은 같은 번호를 쓴 **새 요청**의 짝으로 배달된다★
//!   (상관 키가 그 번호뿐이라 둘을 가를 재료가 wire 에 없다).
//!   ★**잊는 것은 결함이 아니라 의도된 거래다**★ — 잊지 않으면 그 번호의 **정당한** 재사용도 영구히
//!   막히고, 상한 없는 기억은 무한히 자란다. **두 리뷰어가 이 지점에서 갈렸고, 유계 링을 지금 모양
//!   그대로 두는 것이 사용자 결정이다**(2026-09-06) — 우리 실제 `Tag` 는 UUID 라 정당한 충돌이 안
//!   나고, 남는 구멍은 **소비자 버그 + 링 하나 분량만큼 늦은 답장**이 동시에 필요하다.
//!   ★다음 세션이 이것을 실수로 읽고 「고치지」 말 것★ — 상세는 [`Pending`] rustdoc.
//! - ★**살아 있는 번호가 겹치면 양쪽이 다 실패한다**★ — 옛 대기자는 [`RequestError::Superseded`],
//!   새 요청은 [`RequestError::TagRetired`] 다. 상관 키가 그 번호 하나뿐이라 **먼저 나간 요청의 늦은
//!   답장을 나중 요청의 짝과 구별할 수단이 wire 에 없고**, 잘못 배달하느니 둘 다 실패시킨다.
//!   ★TRD §4-1 의 「새 것이 승계」와 다른 지점이다★.
//! - ★**닫기 코드가 상대에게 닿는 자리는 좁다**★ — RFC 6455 §7.4.1 이 `1006` 을 와이어로 못 보내는
//!   예약값으로 못박는다. 혼잡한 소켓에서 끊을 때(큐 포화·쓰기 시한)는 닫기 프레임을 밀어 넣을 자리가
//!   없다. 코드가 실제로 전달되는 자리는 **핸드셰이크 거절**이고, [`LinkRead::Closed`] 가 그 코드를
//!   나르는 유일한 통로다.
//! - ★**운영 중 끊김은 사유 **문구**를 안 나른다**★ — [`TransportEvent::Disconnected`] 가 싣는 것은
//!   [`DisconnectCause`] 뿐이라 통로가 준 문구도, 상대가 실은 닫기 코드도 갈 곳이 없다.
//! - ★**`tracing` 이 없다**★ — 사건 스트림([`TransportEvent`])이 그 자리다. **소비자가 사건을 안 읽는
//!   동안은 아무 흔적도 안 남는다.** 다리는 소비자가 놓는다.
//! - ★**지터는 수동 시계 아래에서 결정적이다**★ — 난수원이 시계 하위 나노초 + 상대별 씨앗이라, 하네스가
//!   원하는 성질인 대신 지터의 *분산*은 실소켓에서만 관측된다.
//! - ★**[`TransportEvent::StreamTruncated`] 의 `oldest_valid` 는 상대가 말해 준 값이 아니라 관측값이다**★ —
//!   [`Wire`] 의 여섯 칸에 「지금 유효한 가장 오래된 번호」를 실어 오는 입구가 없다. 재요청을 낸 뒤
//!   그 답(`after + 1` 번 조각)을 기다리는 동안 도착한 순서 밖 조각을 **붙들어 두고**, 창이 닫히면
//!   (시한 초과 · [`Policy::resume_buffer`] 포화 · 스트림 축출) **그 첫 번호부터 순서대로 내보내면서**
//!   그 번호를 올린다. 그래서 값이 참이다 — 실제로 그 번호부터 배달한다. 상대의 권위 값(우리 wire 의
//!   `SubscribeAck.oldest_seq`)을 쓰려면 `Wire` 에 칸이 하나 늘어야 하고, ★그건 소비자 전부를 깨는
//!   계약 변경이라 사용자 승인 사항이다★.
//! - ★**붙드는 동안은 배달이 밀린다**★ — 순서를 지키려고 창이 닫힐 때까지 그 스트림의 조각을 안 올린다.
//!   최대 지연 = [`Policy::resume_timeout`], 최대 적재 = [`Policy::resume_buffer`] 개.
//! - ★**스트림 장부는 [`Policy::max_streams`] 에서 잘린다**★ — 넘치면 **가장 오래 안 쓴** 것부터 잊고
//!   [`TransportEvent::StreamForgotten`] 으로 신고한다(붙들고 있던 조각은 함께 돌려준다).
//!   ★잊은 스트림이 나중에 다시 나타나면 장부가 처음부터 시작되므로 **진도가 되감긴다**★ — 그것을
//!   막으려면 잊은 것을 기억해야 하고, 그러면 상한을 두는 이유가 사라진다.
//! - ★**재연결 예산은 축이 둘이다**★ — 「못 붙는다」(연결 성공이 0 으로 되돌린다)와 「붙어도 안
//!   버틴다」([`Policy::live_dwell`] 을 못 채운 연속 횟수). 어느 쪽이 바닥나도 포기하고, 둘 다
//!   [`ReconnectPolicy::max_attempts`] 를 상한으로 쓴다.
//! - ★**공유 줄 포화는 연결을 안 끊는다**★ — [`Policy::inbound_queue`] 는 상대 전부가 함께 쓰므로 그것이
//!   찼다고 어떤 상대를 끊으면 시끄러운 상대가 조용한 상대를 죽인다. 세어 두었다가
//!   [`TransportEvent::Dropped`] 로 신고할 뿐이고, **그 사이 도착한 패킷은 실제로 사라진다.**
//!   연결마다 따로 재는 자원은 [`Policy::frame_buffer`] 쪽이다.
//! - ★**[`Peer::close`] 에 완료 신호가 없다**★ — 돌아온 시점에 감독이 아직 통로를 닫는 중일 수 있다.
//!   같은 이름표를 다시 올릴 때 그 겹침이 해롭지 않은 것은 **세대 바닥값** 덕이지 순서 덕이 아니다.
//! - ★**하네스는 current_thread 런타임 전용이다**★ — [`testing::settle`] 이 협조적 양보로만 동기화하고,
//!   그 위반을 스스로 패닉으로 잡는다.
//! - ★**받는 쪽(accept)이 아직 없다**★ — [`Handshake`] 는 이미 방향 대칭이지만 `Listener` 는 이 범위에
//!   없다. 들어오면 [`Registry::new`] 시그니처가 바뀐다(TRD §12-7 의 열린 질문).
//! - ★**WS 어댑터가 아직 없다**★ — [`Dialer`]/[`LinkTx`]/[`LinkRx`] 구현체가 이 crate 에 하나도 없고,
//!   있는 것은 `test-support` 뒤의 인메모리 하네스뿐이다. 그래서 **실소켓 검증(쓰기 시한이 커널 버퍼
//!   앞에서 실제로 잘라내나)은 아직 아무도 안 잰다.**
//!
//! ## 단독 검증 (ADR-0012)
//!
//! `cargo test -p engram-dashboard-transport --all-features` 가 이 crate 만으로 돈다 — 소켓도 실시간도
//! 자식 프로세스도 없다. 백오프·keepalive·쓰기 시한은 [`testing::ManualClock`] 위에서 **실제로 기다리지
//! 않고** 잰다(ADR-0177 결정 8).
//!
//! ★`-- --test-threads=4` 를 붙이지 않는다★ — 이 crate 는 자식 프로세스를 하나도 안 띄운다.
//! CLAUDE.md 「빌드·검증 명령」의 판정 규칙 그대로이고 `command`·`protocol`·`messaging`·`net` 과 같은 처지다.

// ADR-0177
// ADR-0180
// ADR-0181
// ADR-0182
// ADR-0183
pub mod clock;
pub mod event;
pub mod frame;
pub mod link;
pub mod machine;
pub mod peer;
pub mod pending;
pub mod policy;
pub mod registry;
pub mod stream;
pub mod wire;

#[cfg(any(test, feature = "test-support"))]
pub mod testing;

pub use clock::{with_deadline, Clock, Elapsed, SystemClock};
pub use event::{Arrival, Direction, Incoming, PeerId, TransportEvent};
pub use frame::{Close, CloseCode, Frame};
pub use link::{Address, Dialer, Handshake, HandshakeStep, LinkError, LinkRx, LinkTx};
pub use machine::{
    Action, ConnectFailure, DisconnectCause, GaveUpReason, Generation, Input, Machine,
    MachineEvent, PeerState,
};
pub use peer::{Peer, SendError};
pub use pending::{Insert, Pending, RequestError};
pub use policy::{DecodeFailurePolicy, Policy, QueueFullPolicy, ReconnectPolicy};
pub use registry::{Inbound, Registry};
pub use stream::{StreamMark, Streams, Verdict};
pub use wire::Wire;
