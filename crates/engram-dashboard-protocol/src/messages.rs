//! wire 메시지 — UI→core [`AgentCommand`], core→UI [`AgentEvent`].
//! 둘 다 externally-tagged JSON(serde 기본). 단 고-throughput TerminalBytes 출력은
//! JSON 이 아닌 binary frame(`codec`)으로 흐른다(설계 §1-2).

use ts_rs::TS;

use crate::domain::{AgentInfo, AgentStatus, Capabilities, RestoreReport};
use crate::ids::{AgentId, ProfileId, RequestId};

/// UI→core 요청 envelope(설계 §3). side-effect 명령은 `request_id` 로 idempotent.
/// (Profile CRUD 는 phase 1 에서 core profile 타입 합류 후 추가 — 지금은 보류.)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub enum AgentCommand {
    /// 새 에이전트 spawn. 프로필 참조.
    Spawn {
        #[ts(type = "string")]
        profile_id: ProfileId,
        request_id: RequestId,
    },
    /// 에이전트 종료(자원 강제 폐쇄).
    Kill {
        #[ts(type = "string")]
        agent_id: AgentId,
        request_id: RequestId,
    },
    /// 진행 중 작업만 중단(Ctrl+C). 프로세스는 생존.
    Interrupt {
        #[ts(type = "string")]
        agent_id: AgentId,
        request_id: RequestId,
    },
    /// stdin 입력 전달. raw 바이트(키 입력). idempotency 키 필수(중복=입력 중복).
    WriteStdin {
        #[ts(type = "string")]
        agent_id: AgentId,
        #[serde(with = "serde_bytes")]
        #[ts(type = "number[]")]
        data: Vec<u8>,
        request_id: RequestId,
    },
    /// PTY 크기 변경. viewport_id 는 멀티뷰 중 어느 뷰가 요청했는지(ControlLease 판정용).
    Resize {
        #[ts(type = "string")]
        agent_id: AgentId,
        cols: u16,
        rows: u16,
        viewport_id: Option<String>,
    },
    /// 출력 구독. epoch/after_seq 로 재연결 resume(설계 §1-3).
    /// 둘 다 None = 처음부터(oldest 부터) 받겠다는 신규 구독.
    Subscribe {
        #[ts(type = "string")]
        agent_id: AgentId,
        epoch: Option<u32>,
        #[ts(type = "number | null")]
        after_seq: Option<u64>,
    },
    /// 구독 해제.
    Unsubscribe {
        #[ts(type = "string")]
        agent_id: AgentId,
    },
    /// 전체 에이전트 목록 조회(연결 직후 데몬이 자동 push 도 하지만 명시 조회도 허용).
    ListAgents,
    /// 데몬 종료(§5 LLM 제어). force=true 면 활성 에이전트 있어도 종료, kill_agents=true 면 함께 정리.
    StopDaemon {
        force: bool,
        kill_agents: bool,
        request_id: RequestId,
    },
}

/// core→UI 이벤트 envelope(설계 §3, JSON 경로). TerminalBytes 출력은 여기 없음(binary frame).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub enum AgentEvent {
    /// 연결 직후 핸드셰이크. 버전·capability 통보.
    Hello {
        protocol_version: u32,
        daemon_version: String,
        /// 데몬 전체 capability(에이전트별 capability 는 AgentInfo 에).
        capabilities: Option<Capabilities>,
    },
    /// side-effect command 수신/처리 확인(request_id 에코).
    Ack { request_id: RequestId },
    /// Subscribe 응답 — replay 방식과 범위(설계 §1-3).
    SubscribeAck {
        #[ts(type = "string")]
        agent_id: AgentId,
        action: SubscribeAction,
        current_epoch: u32,
        #[ts(type = "number")]
        oldest_seq: u64,
        #[ts(type = "number")]
        latest_seq: u64,
        /// 이 seq+1 부터 replay 를 보낸다(클라가 dedup 기준).
        #[ts(type = "number")]
        replay_from: u64,
        /// ring 밖으로 밀려 일부 손실(clear+tail). UI "output truncated" 표시.
        truncated: bool,
    },
    /// 저빈도 구조화 출력(TextDelta/Usage/ToolCall 등). TerminalBytes 는 binary frame 으로 감.
    Output {
        #[ts(type = "string")]
        agent_id: AgentId,
        epoch: u32,
        #[ts(type = "number")]
        seq: u64,
        chunk: OutputChunk,
    },
    /// replay 구간 끝 — 이후는 라이브(C4 원자 전환의 클라측 신호).
    ReplayComplete {
        #[ts(type = "string")]
        agent_id: AgentId,
        epoch: u32,
    },
    /// 상태 변경. epoch 동봉(옛 세션 stale 알림 방어).
    StatusChanged {
        #[ts(type = "string")]
        agent_id: AgentId,
        status: AgentStatus,
        epoch: u32,
    },
    /// 전체 목록 갱신. terminal 판정은 이걸로(status_changed 아님 — 설계 불변식).
    AgentListUpdated { agents: Vec<AgentInfo> },
    /// 복원 시도 결과.
    RestoreResult { report: RestoreReport },
    /// 오류 통지. request_id 있으면 특정 command 실패.
    Error {
        request_id: Option<RequestId>,
        message: String,
    },
}

/// Subscribe 결과 분기(설계 §1-3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub enum SubscribeAction {
    /// epoch 불일치 → 완전 초기화 후 oldest 부터.
    Reset,
    /// epoch 일치 & after_seq<oldest → oldest 부터(앞부분 손실, clear+tail).
    TruncatedReplay,
    /// epoch 일치 & after_seq>=oldest → after_seq+1 부터 무손실 이어받기.
    Resume,
}

/// 출력 청크 — 종류 불가지(설계 §2). TerminalBytes 는 binary frame(codec)으로,
/// 나머지 구조화 variant 는 JSON(AgentEvent::Output)으로 흐른다.
/// (구조화 turn 단위 출력은 TUI↔구조화 스위칭 모드 설계 때 실제 채움 — 지금은 형태만 연다.)
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub enum OutputChunk {
    /// 콘솔 raw 바이트(현 유일 실사용). JSON 경로엔 안 실림 — codec binary frame 전용.
    TerminalBytes(
        #[serde(with = "serde_bytes")]
        #[ts(type = "number[]")]
        Vec<u8>,
    ),
    /// API/구조화 텍스트 증분.
    TextDelta(String),
    /// 토큰 사용량.
    Usage {
        input_tokens: u64,
        output_tokens: u64,
    },
    /// 도구 호출(이름+직렬화 인자).
    ToolCall { name: String, args_json: String },
    /// 임의 구조화 페이로드(forward-compat 탈출구).
    Structured { kind: String, json: String },
}
