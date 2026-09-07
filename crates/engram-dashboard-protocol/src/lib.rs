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
//! ## ★ 이름 충돌 메모:
//! 이 crate 의 [`AgentCommand`] = **클→데몬 요청 envelope**(설계 §3 명칭).
//! 기존 `agent(profile.rs)::AgentCommand` = **spawn 종류**(Claude/Shell) — 그쪽의 wire 미러는 이 enum 이
//! 아니라 [`AgentSpawnCommand`] 다. crate 를 빼고 "AgentCommand" 라 부르면 뜻이 안 정해진다.
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
    AgentBackendKind, AgentFailureKind, AgentInfo, AgentProfile, AgentSpawnCommand, AgentStatus,
    Capabilities, ClaudeOutputFormat, ControlCaps, EnvelopeFormat, InputCaps, ModelCaps,
    OutputCaps, Preset, RestartPolicy, RestoreOutcome, RestoreReport, SessionCaps, SnapshotChunk,
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
///
/// v4: codex 배선 — [`AgentSpawnCommand::Codex`] **변형 추가** + `SpawnByCwd`·`CreateProfile` 에
/// `backend`([`AgentBackendKind`]) 칸 신설.
/// ★변형을 더한 것이라 v3 과 같은 **비관용** 축이다★ — `AgentSpawnCommand` 는 `#[serde(tag = "kind")]`
/// 라 모르는 `kind` 는 관용되는 미지 **필드**가 아니라 역직렬화 **실패**다. 「기존 변형 안에
/// `#[serde(default)]` 칸을 더하면 버전 유지」(v2·v3 사이에 여러 번 그렇게 지나갔다)는 이 변경에
/// 해당하지 않는다. 두 방향이 각각 조용히 깨진다:
///   - **신데몬 + 구셸**: codex 프로필이 하나라도 명부에 있으면 `ProfileList` 의
///     `AgentProfile.command` 에서 디코드가 실패하고, 실패 단위가 **응답 전체**라 구셸은 그 한 행이
///     아니라 **명부를 통째로** 잃는다.
///   - **구데몬 + 신셸**: 새 `backend` 칸은 `#[serde(default)]` 라 구데몬이 그것을 **버리고** 옛
///     하드코딩(claude·StreamJson)을 띄운다 — 사람이 「코덱스 터미널」을 골랐는데 codex 라벨이 붙은
///     노드 뒤에서 claude 가 돈다. [`AgentBackendKind`] 가 기본값을 안 두기로 한 결정이 막으려던 바로
///     그 실패인데, 그 결정을 모르는 데몬에는 그 결정이 닿지 않는다.
/// ★출시 뒤가 아니라 로컬 개발에서 먼저 온다★ — 데몬은 설계상 셸 재빌드보다 오래 산다(셸을 다시 지어도
/// 떠 있던 데몬에 그대로 붙는다). 그래서 위 두 조합은 개발 중에 일상적으로 만들어진다. 「출시 전이라
/// 지켜 줄 상대가 없다」로 이 bump 를 건너뛰려던 판단이 틀렸던 지점이 여기다.
/// bump 가 둘 다 시끄럽게 만든다: auth 의 version check(`net` 의 `ws.rs`) + discovery 의
/// version-mismatch 거부(`discovery` 의 `check_acceptable`)가 짝이 안 맞는 데몬을 **재사용하지 않고
/// 거부/재기동**한다. 그 강제를 재는 자리 = discovery 의
/// `version_mismatch_live_daemon_errors_without_spawn`.
pub const PROTOCOL_VERSION: u32 = 4;
