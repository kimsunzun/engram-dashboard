//! 에이전트 명령 invoke 핸들러 — request/reply 평면(S14 모듈① T6a, ADR-0036).
//!
//! 프론트(또는 §5 LLM)가 `invoke('agent_spawn'|…)` 로 부르면 여기서 `AgentCommand` 를 빌드해
//! `DaemonClient::send_command` 로 데몬에 보내고, 데몬 reply(request_id 매칭)를 프론트 친화 Result 로
//! 돌려준다. 두뇌(연결 의미론)는 `DaemonClient`, 이 레이어는 **얇은 빌더 + 주입**일 뿐이다.
//!
//! ## ★request_id 는 여기서 박는다(load-bearing)★
//! 각 핸들러가 `RequestId::new()` 로 새 키를 만들어 명령에 싣는다 — send_command 가 그 키로 reply 를
//! 매칭한다(spike §9 G1 의 "request_id 출처"는 이 레이어). idempotency 의미상 *재시도 시 같은 키*가
//! 정석이나, invoke 단발 호출은 매번 새 키로 충분(끊김 시 호출자가 재호출 = 새 키). writeStdin 중복
//! 방지는 데몬측 dedup table 책임(ids.rs RequestId 주석).
//!
//! ## OUT OF SCOPE (= T6b)
//! 출력 구독(`subscribe_output`)·window Channel registry·OutputRouter fan-out 은 T6b. 여기엔 없다.

use std::sync::Arc;

use engram_dashboard_protocol::{AgentCommand, AgentEvent, AgentId, ProfileId, RequestId};
use tauri::State;
use uuid::Uuid;

use crate::daemon_client::DaemonClient;

/// 프론트가 보낸 UUID 문자열을 파싱한다. invalid 면 명확한 Err(패닉 금지 — 잘못된 입력 방어).
fn parse_uuid(s: &str, what: &str) -> Result<Uuid, String> {
    Uuid::parse_str(s).map_err(|e| format!("{what} UUID 파싱 실패: {e}"))
}

/// send_command 결과를 프론트로 넘기기 전 공통 처리. reply 이벤트는 데몬측 의미(Ack/Spawned/…)라,
/// 성공 케이스는 호출자별로 필요한 만큼만 꺼내고 여기선 그대로 통과시킨다(각 핸들러가 변환).
type CmdResult = Result<AgentEvent, String>;

/// 새 에이전트 spawn(프로필 참조). reply = `Ack`(데몬 spawn dispatch 확인). 성공 시 `()`.
///
/// ★주의(reply 종류)★: 데몬 ws.rs 의 `Spawn{profile_id}` dispatch 는 Ack 로 응답한다(SpawnByCwd/
/// SpawnProfile 만 Spawned 로 AgentInfo 동봉). 여기선 Ack/Spawned 어느 쪽이든 성공으로 본다.
#[tauri::command]
pub async fn agent_spawn(
    client: State<'_, Arc<DaemonClient>>,
    profile_id: String,
) -> Result<(), String> {
    let profile_id: ProfileId = parse_uuid(&profile_id, "profile_id")?;
    let cmd = AgentCommand::Spawn {
        profile_id,
        request_id: RequestId::new(),
    };
    expect_ack_or_spawned(client.send_command(cmd).await)
}

/// 에이전트 종료(자원 강제 폐쇄). reply = `Ack`.
#[tauri::command]
pub async fn agent_kill(
    client: State<'_, Arc<DaemonClient>>,
    agent_id: String,
) -> Result<(), String> {
    let agent_id: AgentId = parse_uuid(&agent_id, "agent_id")?;
    let cmd = AgentCommand::Kill {
        agent_id,
        request_id: RequestId::new(),
    };
    expect_ack(client.send_command(cmd).await)
}

/// 진행 중 작업만 중단(Ctrl+C). 프로세스는 생존. reply = `Ack`.
#[tauri::command]
pub async fn agent_interrupt(
    client: State<'_, Arc<DaemonClient>>,
    agent_id: String,
) -> Result<(), String> {
    let agent_id: AgentId = parse_uuid(&agent_id, "agent_id")?;
    let cmd = AgentCommand::Interrupt {
        agent_id,
        request_id: RequestId::new(),
    };
    expect_ack(client.send_command(cmd).await)
}

/// stdin 입력 전달(raw 바이트). reply = `Ack`. `data` 는 프론트에서 byte 배열(키 입력).
#[tauri::command]
pub async fn agent_write_stdin(
    client: State<'_, Arc<DaemonClient>>,
    agent_id: String,
    data: Vec<u8>,
) -> Result<(), String> {
    let agent_id: AgentId = parse_uuid(&agent_id, "agent_id")?;
    let cmd = AgentCommand::WriteStdin {
        agent_id,
        data,
        request_id: RequestId::new(),
    };
    expect_ack(client.send_command(cmd).await)
}

/// PTY 크기 변경. ★주의★: `Resize` 는 wire 상 request_id 가 없어(데몬이 reply 안 보냄) send_command 의
/// reply 매칭 대상이 아니다 — 그래서 fire-and-forget 가 맞다. T6a 의 send_command 는 request_id 없는
/// 명령을 거르므로(영구 pending 방지), resize 는 **별도 fire-and-forget 경로**가 필요하다. T6a 에선
/// 이 핸들러를 노출만 하고 실제 송신은 T6b(구독/출력 평면)와 함께 배선한다(아래 TODO).
///
/// ★현재 동작★: 연결 여부만 확인하고 `Ok(())`(no-op). resize 미반영은 출력 화면 크기 어긋남일 뿐
/// 동작 안전엔 무해 — T6b 가 fire-and-forget 송신 경로(reply 없는 명령 enqueue)를 채울 때 실제 wire
/// 송신을 붙인다. (지금 reply 기대 경로로 보내면 영구 hang 이라 일부러 안 보낸다.)
#[tauri::command]
pub async fn agent_resize(
    _client: State<'_, Arc<DaemonClient>>,
    agent_id: String,
    _cols: u16,
    _rows: u16,
) -> Result<(), String> {
    // TODO(T6b): reply 없는 fire-and-forget 명령(Resize/Subscribe/Unsubscribe) 송신 경로 추가 후
    //   여기서 AgentCommand::Resize{agent_id, cols, rows, viewport_id:None} 를 enqueue.
    // ★silent no-op 가시화(FIX-3)★: 지금은 송신 경로가 없어 no-op 이지만, Ok(()) 만 돌려주면 T7 cutover
    //   때 "조용한 거짓"이 된다(호출자는 성공으로 믿는데 resize 가 안 감). warn 으로 미배선 상태를
    //   진단 가능하게 남긴다 — Ok(()) 반환은 유지(호출자 깨지 않게).
    tracing::warn!(
        agent_id = %agent_id,
        "agent_resize 미배선 — T6b fire-and-forget 경로 대기"
    );
    Ok(())
}

/// reply 가 Ack(void 성공)인지 확인. 그 외 event 면 예상 밖이나, 성공 reply 류는 모두 통과시킨다.
fn expect_ack(result: CmdResult) -> Result<(), String> {
    match result {
        Ok(AgentEvent::Ack { .. }) => Ok(()),
        // 데몬이 다른 성공 reply 를 줘도(있어선 안 되지만) 성공으로 본다 — 핵심은 Error 가 아니라는 것.
        // ★프로토콜 drift 가시화(FIX-5)★: Ack 가 아닌 다른 variant 를 성공으로 *조용히* 삼키면 데몬-클라
        //   계약 어긋남이 안 보인다 → 어떤 variant 였는지 warn 후 통과(반환 동작은 그대로 = 호출자 안 깨짐).
        Ok(other) => {
            tracing::warn!(
                reply = ?other,
                "expect_ack: Ack 가 아닌 reply 를 성공 처리(프로토콜 drift 의심)"
            );
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// reply 가 Ack 또는 Spawned(둘 다 spawn 성공)인지 확인.
fn expect_ack_or_spawned(result: CmdResult) -> Result<(), String> {
    match result {
        Ok(AgentEvent::Ack { .. }) | Ok(AgentEvent::Spawned { .. }) => Ok(()),
        // ★프로토콜 drift 가시화(FIX-5)★: Ack/Spawned 가 아닌 variant 를 조용히 삼키지 않고 warn.
        Ok(other) => {
            tracing::warn!(
                reply = ?other,
                "expect_ack_or_spawned: Ack/Spawned 가 아닌 reply 를 성공 처리(프로토콜 drift 의심)"
            );
            Ok(())
        }
        Err(e) => Err(e),
    }
}
