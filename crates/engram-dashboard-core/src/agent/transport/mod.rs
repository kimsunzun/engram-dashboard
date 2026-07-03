//! AgentTransport — 에이전트 1개의 데이터 채널 + 자원 제어 seam.
//!
//! transport는 바이트·이벤트를 만들어 `OutputCore::emit`/`finish`로 넘기기만 하면 된다.
//! 출력 fanout·종료 전이·구독 같은 공용 로직은 OutputCore가 담당하고, transport는
//! 자기 자원(PTY master/child/job 혹은 HTTP 스트림)의 수명만 책임진다.
//!
//! 콘솔(claude/codex/gemini)은 PtyTransport 한 벌을 공유하고, API는 ApiTransport(stage 7).
//!
//! tauri import 0.

use std::sync::Arc;

use crate::agent::output_core::OutputCore;
use crate::agent::types::{InputEvent, OutputEvent, PtyError, TransportCaps};

pub mod api;
pub mod pty;
pub mod stdio;

/// 출력 바이트 → OutputEvent 정제 seam (backend-agnostic — ADR-0004/0045).
///
/// ★왜 이 트레이트가 필요한가(ADR-0004 격리)★: transport(StdioTransport)는 **바보 파이프**라
///   자식 stdout 바이트가 무슨 스키마인지(claude stream-json / codex 프로토콜 / 평문) 몰라야 한다.
///   그런데 json 모드는 그 바이트를 구조화 OutputEvent 로 정제해야 한다 — 그 파싱 지식은 backend
///   소유다(claude 라면 `ClaudeStreamDecoder`, backend/claude.rs 단독). 그래서 파싱 로직을 이
///   트레이트 뒤에 숨겨 **transport 는 "어떤 디코더인지 모른 채" 주입받아 적용만** 한다. transport 는
///   `dyn OutputDecoder` 만 알고 claude 를 모른다(ADR-0044 무정제 불변: 통로는 스스로 파싱 안 함).
///
/// ★수명·상태(pump 스레드 단독 소유)★: decoder 는 라인 재조립을 위해 부분 라인 버퍼 등 **가변
///   상태**를 들고, pump 스레드(단일)가 `&mut` 로 배타 소유한다 — 그래서 `Send`(스레드로 move)만
///   요구하고 `Sync` 는 요구하지 않는다(공유 접근 없음). epoch 교체 = 새 transport = 새 decoder 라
///   리셋이 자동이다(재생성으로 상태가 깨끗해짐).
///
/// core 도메인 타입(OutputEvent)만 생성한다 — Serialize 무관(ADR-0003: core 는 wire 를 모른다).
pub trait OutputDecoder: Send {
    /// 임의 크기 바이트 청크를 밀어 넣고, 이번 청크로 **완성된** 이벤트만 돌려준다.
    /// 미완성 꼬리(개행 없는 부분 라인 등)는 구현체가 내부 버퍼에 남겨 다음 청크와 합친다.
    fn decode(&mut self, chunk: &[u8]) -> Vec<OutputEvent>;

    /// EOF(스트림 종료) 시 1회 호출 — 개행으로 종단되지 않은 잔여 라인을 마저 처리한다.
    /// pump 는 break(EOF/shutdown) 직후 finish **전에** 이걸 불러 잔여를 소진한다.
    fn flush(&mut self) -> Vec<OutputEvent>;
}

/// 에이전트 백엔드(PTY/API)를 추상화하는 seam. AgentSession이 `Box<dyn AgentTransport>`로 보유.
///
/// 제어 동사: start · send_input · resize · interrupt · shutdown · capabilities.
/// (reconfigure/graceful_shutdown은 API 도입 때 단계적으로 추가.)
pub trait AgentTransport: Send + Sync {
    /// 출력 pump/stream 기동 → core 연결. spawn 직후 1회 호출.
    /// PtyTransport: 보관해둔 reader를 take해 pump 스레드 spawn, core.attach_pump 호출.
    fn start(&self, core: Arc<OutputCore>);

    /// 입력 이벤트 전달. PTY=Raw 바이트를 writer로 흘려보낸다.
    fn send_input(&self, input: InputEvent) -> Result<(), PtyError>;

    /// 터미널 크기 변경. cols/rows의 보존(atomic 저장)은 AgentSession 책임.
    fn resize(&self, cols: u16, rows: u16) -> Result<(), PtyError>;

    /// ≠kill. 진행 중 작업만 중단(PTY=0x03 주입 / API=cancel). 프로세스는 살아 있다.
    fn interrupt(&self) -> Result<(), PtyError>;

    /// 자원 강제 종료(멱등). PtyTransport: shutdown flag set → child.kill+wait → job.terminate
    /// → master.take(drop). 반환 전 master drop 보장(pump read가 EOF로 깸).
    /// pump 종료 대기는 여기서 안 함(core.join_pump 몫).
    fn shutdown(&self);

    /// 이 transport(물리 채널)가 결정하는 caps만 반환 — input/output/control.
    /// session(resume)·model 은 backend 소관이라 여기서 만들지 않는다(`BackendCaps`).
    /// 최종 Capabilities 는 AgentSession 이 `Capabilities::compose(transport, backend)` 로 합성.
    fn capabilities(&self) -> TransportCaps;
}
