//! # engram-dashboard-protocol — 경계 계약 (linchpin)
//!
//! UI(프론트) ↔ core/daemon 사이 wire 프로토콜. 두 모드 공통:
//!   - Embedded: Tauri invoke/Channel 이 이 타입을 실어 나름.
//!   - Daemon: 127.0.0.1 WS 가 이 타입을 실어 나름.
//!
//! ## 설계 근거 (daemon-design.md)
//! - §1-1 단일 WS 연결·단일 수신루프(lane 분리 금지) — control 과 output 이 같은 연결.
//! - §1-2 wire codec: **output hot path = 커스텀 고정헤더 binary frame**(`codec`), control = JSON.
//!   그래서 `AgentEvent`(JSON enum)에는 고-throughput TerminalBytes 가 안 실린다 — 그건 binary frame.
//!   저빈도 구조화 출력(TextDelta/Usage/ToolCall)만 JSON `AgentEvent::Output` 으로 흐른다.
//! - §1-3 replay 기점: epoch 불일치=Reset / afterSeq<oldest=TruncatedReplay / 그 외=Resume(`SubscribeAction`).
//!
//! ## Tauri import 금지. 도메인 로직 금지(순수 타입·serde·codec 만).
//!
//! ## ★ 이름 충돌 메모 (phase 1 reconcile):
//! 이 crate 의 [`AgentCommand`] = **UI→core 요청 envelope**(설계 §3 명칭).
//! 기존 `core(profile.rs)::AgentCommand` = **spawn 종류**(Claude/Shell).
//! 둘은 다른 개념이라 phase 1(core 가 이 crate 의존) 시 spawn 종류를 `SpawnSpec` 등으로 개명해야
//! TS 생성 바인딩 충돌(동명 export)을 막는다. 지금은 독립 crate 라 충돌 없음.
//!
//! ## seq 의 TS 매핑
//! u64 seq 는 ts-rs 기본 매핑이 `bigint` 이지만, 기존 프론트(`PtyEvent.seq: number`)와
//! 정합 + JSON number 한계(2^53) 내 현실 안전(초당 수만 청크라도 수천년)으로 `#[ts(type="number")]`
//! 고정. binary frame 의 seq 는 JS `DataView.getBigUint64` 로 받으므로 무관(JSON 경로만 number).

mod codec;
mod discovery;
mod domain;
mod ids;
mod messages;

pub use codec::{
    decode_frame, encode_structured_frame, encode_terminal_frame, CodecError, DecodedFrame,
    FRAME_HEADER_LEN, FRAME_TAG_STRUCTURED_EVENT, FRAME_TAG_TERMINAL_BYTES,
};
pub use discovery::DaemonInfo;
pub use domain::{
    AgentInfo, AgentProfile, AgentSpawnCommand, AgentStatus, Capabilities, ClaudeOutputFormat,
    ControlCaps, InputCaps, ModelCaps, OutputCaps, Preset, RestartPolicy, RestoreOutcome,
    RestoreReport, SessionCaps, SnapshotChunk,
};
pub use ids::{AgentId, PresetId, ProfileId, RequestId};
pub use messages::{AgentCommand, AgentEvent, OutputChunk, StructuredEvent, SubscribeAction};

/// 프로토콜 버전. 깨지는 변경(필드 의미 변경·제거)에서만 +1(설계 결정 #6: 버전 처리 deferred,
/// 지금은 상수만 두고 Hello 에 실어 보냄 — 불일치 시 팝업 가이드는 나중).
///
/// v2: ListAgents/ListProfiles 조회 응답을 broadcast(AgentListUpdated/ProfileListUpdated) 편승
/// 매칭에서 request_id 동봉 전용 reply(AgentList/ProfileList)로 전환 + Snapshot 에 request_id 추가.
/// ListAgents/ListProfiles 커맨드도 unit→request_id 동봉으로 변경(reply 계약 변경). 구데몬(v1)은
/// 구 응답만 보내 신클라가 무한 대기할 수 있으므로 version mismatch 로 거부한다(자동재기동 정책은 별건).
pub const PROTOCOL_VERSION: u32 = 2;
