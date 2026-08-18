//! # engram-dashboard-protocol — 경계 계약 (linchpin)
//!
//! UI(프론트) ↔ daemon 사이 wire 프로토콜(daemon-only 단일 경로 — ADR-0029 embedded 모드 제거,
//! ADR-0036 창↔데몬 직결 없이 src-tauri 가 단일 데몬 클라이언트로 중계).
//! 두 구간이 이 타입을 실어 나른다:
//!   - 프론트 ↔ src-tauri(로컬 IPC): Tauri invoke(명령) · 이벤트(브로드캐스트) · Channel(출력 frame, raw byte).
//!   - src-tauri(DaemonClient) ↔ daemon: 127.0.0.1 WS.
//!
//! ## 설계 근거 (daemon-design.md)
//! - §1-1 단일 WS 연결·단일 수신루프(lane 분리 금지) — control 과 output 이 같은 연결.
//! - §1-2 wire codec: **output hot path = 커스텀 고정헤더 binary frame**(`codec`), control = JSON.
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
    ControlCaps, EnvelopeFormat, InputCaps, ModelCaps, OutputCaps, Preset, RestartPolicy,
    RestoreOutcome, RestoreReport, SessionCaps, SnapshotChunk,
};
pub use ids::{AgentId, PresetId, ProfileId, RequestId};
pub use messages::{
    command_request_id, event_reply_request_id, AgentCommand, AgentEvent, CommandListEntry,
    OutputChunk, StructuredEvent, SubscribeAction,
};

/// 깨지는 변경(필드 의미 변경·제거)에서만 +1(설계 결정 #6: 버전 처리 deferred,
/// 지금은 상수만 두고 Hello 에 실어 보냄 — 불일치 시 팝업 가이드는 나중).
///
/// v2: ListAgents/ListProfiles 조회 응답을 broadcast(AgentListUpdated/ProfileListUpdated) 편승
/// 매칭에서 request_id 동봉 전용 reply(AgentList/ProfileList)로 전환 + Snapshot 에 request_id 추가.
/// ListAgents/ListProfiles 커맨드도 unit→request_id 동봉으로 변경(reply 계약 변경). 구데몬(v1)은
/// 구 응답만 보내 신클라가 무한 대기할 수 있으므로 version mismatch 로 거부한다(자동재기동 정책은 별건).
///
/// v3: `AgentCommand::SetEnvelopeFormat`(UI→daemon 봉투 포맷 스위치, ADR-0096) 추가. 이 커맨드는
/// **비관용 additive** — 같은 v2 데몬이라도 이 variant 를 디코드할 수 없어(unknown externally-tagged
/// 키) deserialize 에서 막힌다. 그러면 신클라가 Ack 를 기다리며 무한 대기할 수 있으므로(v2 bump 사유와
/// 동일한 시나리오), auth 의 version check(ws.rs) + discovery 의 version-mismatch 거부가 구 데몬을
/// **재사용하지 않고 거부/재기동**하게 강제한다.
pub const PROTOCOL_VERSION: u32 = 3;
