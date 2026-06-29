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
//! ## T6b 추가 (출력 평면)
//! - `subscribe_output(channel)` — 창 mount 시 호출, 그 창의 출력 Channel 을 registry 에 등록한다.
//! - `agent_resize` — fire-and-forget(`send_fire_and_forget(Resize)`)로 배선(reply 없는 명령).

use std::sync::Arc;

use engram_dashboard_protocol::{AgentCommand, AgentEvent, AgentId, ProfileId, RequestId};
use tauri::State;
use uuid::Uuid;

use crate::daemon_client::DaemonClient;
use crate::output_channel::WindowChannelRegistry;

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
/// reply 매칭 대상이 아니다 — 그래서 **fire-and-forget**(`send_fire_and_forget`)가 정답이다(reply 기대
/// 경로로 보내면 영구 hang). T6b 가 그 송신 경로를 깔아 여기서 실제로 wire 송신한다.
///
/// ★fire-and-forget 의미★: enqueue 만 하고 ack 를 안 기다린다(resize 미반영=화면 크기 어긋남일 뿐
/// 동작 안전엔 무해). 비연결이면 DaemonClient 가 조용히 no-op — Resize 는 구독 델타가 아니라 단발 명령이라
/// connect 시 자동 재동기되지 않는다(다음 resize 입력이 새 치수로 갱신). 그래서 `Ok(())` 는 "송신 시도함"
/// 이지 "데몬 반영 확인"이 아니다.
#[tauri::command]
pub async fn agent_resize(
    client: State<'_, Arc<DaemonClient>>,
    agent_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let agent_id: AgentId = parse_uuid(&agent_id, "agent_id")?;
    client.send_fire_and_forget(AgentCommand::Resize {
        agent_id,
        cols,
        rows,
        viewport_id: None,
    });
    Ok(())
}

/// ★출력 Channel 등록(T6b)★. 창 mount 시 프론트가 `invoke('subscribe_output', { channel })` 로 호출한다 —
/// 그 창의 출력 Channel 을 window_label → Channel registry 에 넣는다. 연결 task 가 라우팅 표를 보고 이
/// Channel 로 그 창의 모든 agent 출력을 fan-out 한다(프레임에 agent_id 태그 내장).
///
/// ★window label 자동 주입★: `tauri::Window` 를 인자로 받으면 Tauri 가 **호출한 webview** 를 주입한다 →
/// `window.label()` 로 라벨을 얻는다(프론트가 라벨을 안 넘겨도 됨, 위조 불가). Channel 도 호출 webview 에
/// 태생 바인딩된다(spike §7) — 그래서 라벨↔Channel 짝이 항상 정합한다.
///
/// ★raw byte(spike §7)★: registry 타입이 `Channel<tauri::ipc::Response>` 라 연결 task 가
/// `Response::new(bytes)` 로 raw 바이트를 보낸다(`Channel<Vec<u8>>` 의 JSON 직렬화 함정 회피).
#[tauri::command]
pub fn subscribe_output(
    registry: State<'_, WindowChannelRegistry>,
    window: tauri::Window,
    channel: tauri::ipc::Channel<tauri::ipc::Response>,
) -> Result<(), String> {
    let label = window.label().to_string();
    // ★ADR-0006★: registry std Mutex — insert 는 동기, 락 보유 중 await 0. 같은 라벨 재등록(창 reload)은
    //   덮어쓴다(옛 Channel 은 drop — 이미 죽은 webview 라 무해).
    let mut reg = registry.lock().map_err(|e| e.to_string())?;
    reg.insert(label, channel);
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
