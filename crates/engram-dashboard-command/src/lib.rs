//! # engram-dashboard-command — 명령 버스 **도구** (명령 0개)
//!
//! 봉투 · 오류 어휘 · 선언 매크로 · 표 · 라우팅만 담는다. 명령 자체는 **생산자 모듈 옆**에서 선언한다
//! (`core` = `agent.*`, `daemon` = `mail.*`, `src-tauri` = `window/tab/slot`).
//!
//! ## 불변식 셋 (어기면 이 crate 의 존재 이유가 사라진다)
//!
//! 1. **워크스페이스 crate 의존 0.** 게이트 = `rg "path\s*=" crates/engram-dashboard-command/Cargo.toml`
//!    → 0줄. 남이 이 crate 를 의존하는 것은 이 불변식과 무관하다 — 화살표는 한 방향뿐이다.
//!    이 벽이 있어야 `core` 가 명령을 선언하면서도 wire 계약(`protocol`)을 안 볼 수 있다.
//! 2. **명령 0개.** 이 파일 트리에 [`declare_commands!`] 호출이 있으면 위반이다. 도구가 어휘를 갖는 순간
//!    「선언은 생산자 옆」이 무너지고 중앙 카탈로그가 다른 문으로 돌아온다.
//! 3. **★T-1★ 링커 수집이 담는 것은 [`CommandSpec`] 까지.** `static`·링커 수집이
//!    `Arc<dyn CommandHandler>` 를 담으면 테스트가 가짜 의존을 꽂을 자리가 없어져 단위 테스트가 실물
//!    (프로세스)을 띄운다. 핸들러 실물은 각 모듈의 `make_table(deps)` 가 조립 때 주입한다.
//!
//! ## 진입점
//!
//! [`declare_commands!`] 선언 · [`CommandTable`] 내 표 · [`Roster`] 명부 · [`route`] 배달 3단계 ·
//! [`CommandTable::check_args`] 입구 전용 인자 검문 · [`command_specs`] 이 바이너리에 링크된 선언 전량.
//!
//! ## 알려진 예정 사항 (Step 2 착수 전에 읽을 것)
//!
//! - **입구 검문은 배선이 부르지 않는다(ADR-0142).** [`CommandTable::check_args`] 는 **사람·LLM 이 치는
//!   표면**에서만 부른다 — [`route`] 안에 넣으면 버전이 앞선 호출자가 실은 신규 칸이 옛 주인을 하드
//!   실패시켜 additive 진화가 죽는다(TRD §4-③). 표면별 안내 문구는 어댑터가 덧붙인다.
//!
//! - **봉투 타입들은 ts-rs 를 구현하지 않는다.** `protocol` 의 wire 메시지는 전부 `#[derive(TS)]` 라
//!   [`CommandEnvelope`]·[`CommandReply`]·[`RequestId`]·[`CommandDecl`] 을 그 enum 에 additive variant 로
//!   싣는 순간 **바인딩 생성이 컴파일 에러로 멈춘다.** 지금 ts-rs 를 안 들인 것은 의도다(외부 의존 최소) —
//!   Step 2 에서 「도구 crate 에 ts-rs 추가」와 「protocol 쪽에서 `#[ts(type = …)]` 로 덮기」 중 하나를
//!   골라야 하고, 그건 wire 계약 결정이라 그때 판단한다.
//! - **패닉 그물은 릴리즈 프로필에서 실효가 없다** — 워크스페이스 릴리즈는 `panic = "abort"` 다.
// ADR-0140
// ADR-0141

mod coerce;
mod envelope;
mod error;
mod link;
mod macros;
mod roster;
mod route;
mod spec;
mod table;

#[cfg(any(test, feature = "test-support"))]
pub mod testing;

pub use envelope::{CommandEnvelope, CommandReply, OwnerToken, RequestId};
pub use error::{CommandError, ErrorCode, RetryMode};
pub use link::{CommandLink, InboundCommands, ReplySink};
pub use macros::transmitted;
pub use roster::{OwnerLookup, OwnerLookupSource, Roster, RosterEntry};
pub use route::route;
pub use spec::{
    catalog_json, command_specs, duplicate_command_names, lint_spec, spec_item_json, spec_of,
    CommandDecl, CommandSpec, Effect, LinkedSpec, COMMON_ERRORS,
};
pub use table::{blocking_handler, CommandFuture, CommandHandler, CommandTable, TableError};

/// 선언 매크로가 링커 수집 항목을 만들 때 쓴다 — 소비 crate 가 `inventory` 를 직접 의존하지 않게 한다.
#[doc(hidden)]
pub use inventory;
