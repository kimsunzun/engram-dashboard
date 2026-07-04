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
///
/// ## ★등록 후 mount-즉시-replay 트리거(BLOCK-2 검증① + 근원2 — 등록 전 유실·reload 빈 화면 차단)★
/// Channel 을 registry 에 insert 한 **뒤**, 이 창(label)이 현재 보는 `(label, agent)` slot 들의 replay 를
/// actor 경유(`client.replay_slots`)로 트리거한다. layout 델타(`slots_to_replay`)는 *배정 시점* 에
/// replay 를 걸지만 창이 **나중에 mount** 되면 그 사이 on_frame 이 deliverable 게이트로 cursor advance 를
/// *멈춰* 둔다(근원2 — 미등록 창 stale advance 금지). Channel 이 *등록된 뒤* 여기서 다시 트리거하면 연결
/// task `ReplaySlots` arm 이 `resubscribe_slot`(cursor fresh 리셋 + 그 시점 버퍼 전체 replay)으로 그 멈춘
/// cursor 를 처음부터 무손실로 채운다. ★fresh 리셋이라 webview reload(같은 label 로 Channel 교체)도 빈
/// 화면이 안 된다★(옛 cursor 무시·전체 replay — 근원2). actor 경유라 on_frame 과 직렬(replay→live 순서
/// 보장, BLOCK-2). 두 트리거(배정·등록)가 함께 "Channel 등록된 뒤 전체 replay" 를 보장한다.
#[tauri::command]
pub fn subscribe_output(
    registry: State<'_, WindowChannelRegistry>,
    router: State<'_, Arc<crate::output_router::OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
    window: tauri::Window,
    channel: tauri::ipc::Channel<tauri::ipc::Response>,
) -> Result<(), String> {
    let label = window.label().to_string();
    {
        // ★ADR-0006★: registry std Mutex — insert 는 동기, 락 보유 중 await 0. 같은 라벨 재등록(창 reload)은
        //   덮어쓴다(옛 Channel 은 drop — 이미 죽은 webview 라 무해). ★cursor/seq 추적은 registry 가 아니라
        //   AgentBufferStore 가 소유(ADR-0040)★ — registry 는 순수하게 label → Channel 만 든다.
        let mut reg = registry.lock().map_err(|e| e.to_string())?;
        reg.insert(label.clone(), channel);
    } // ★registry 락 드롭 — 아래 replay_slots(actor enqueue)는 registry 락 미보유★
      // ★등록 후 replay 트리거(fresh=true — FIX-2)★: 이 창이 보는 slot 들을 actor 경유로 fresh replay.
      //   Channel 등록 = 그 창 viewer 의 (재)시작이라 **cursor fresh 리셋 + 전체 replay** 가 맞다 — webview
      //   reload(같은 label 로 Channel *교체*)면 옛 xterm 진도가 새 xterm 엔 무의미해 빈 화면이 되므로, 옛
      //   cursor 를 무시하고 버퍼 전체를 다시 줘야 한다(근원2). 배정 트리거(layout 델타 fresh=false 불가침)와
      //   달리 등록 트리거만 fresh 라, 정상 mount 에서 전체 replay 가 연속 2회 안 나간다(중복 제거 — Rust측).
      //   router 는 lock-free(ArcSwap) 조회. slots 비면 replay_slots 가 no-op.
    let slots = router.slots_for_window(&label);
    client.replay_slots(slots, true);
    Ok(())
}

/// ★slot (re)mount 시 fresh replay 재요청(remount 대화 소실 FIX)★. RichSlot/TerminalSlot 이 (re)mount
/// 되면 프론트 구독 effect 가 `invoke('resync_output', { agentId })` 로 호출한다 — 그 창(label)이 보는
/// 그 agent 의 slot 에 **fresh replay**(cursor 리셋 + 버퍼 전체 재전송)를 트리거한다.
///
/// ## ★왜 필요한가(근본원인)★
/// idle tag1(JSON) slot 을 split/재배정하면 Allotment 재귀 트리 구조 변경으로 RichSlot 이 **remount**
/// 되는데, remount 는 웹뷰 출력 Channel 재등록이 *아니라서*(`subscribe_output` 은 창 mount 시 1회) 데몬/
/// backend 가 replay 를 재전송하지 않는다 → remount 된 컴포넌트가 seq=-1·빈 messages 로 재시작하나 replay
/// 가 안 와 대화 소실 + 영구 streaming 고착. 웹뷰 reload 는 Channel 재등록으로 fresh replay 가 흘러 복원되므로,
/// 이 command 가 그 reload 복원 경로(`slots_for_window` → `replay_slots(fresh=true)` → `resubscribe_slot`)를
/// **slot (re)mount 시점에 slot 단위로 재사용**한다.
///
/// ## ★subscribe_output 과의 차이★
/// - `subscribe_output`: 창 mount 시 Channel *등록* + 그 창의 **모든** slot replay.
/// - `resync_output`: Channel 은 이미 등록됨(재등록 안 함) — 특정 **agent 의** slot 만 fresh replay.
///
/// ## ★BLOCK-1(ADR-0041) 준수 + `DaemonClient::resync()` 와 혼동 금지(FIX-E)★
/// 데몬 wire `Subscribe` 를 새로 만들지 않는다 — 여기서 부르는 것은 `client.replay_slots`(src-tauri 로컬 축 B
/// replay — 연결 actor `ReplaySlots` arm → `resubscribe_slot`)이지, 이름이 비슷한 `DaemonClient::resync()`
/// **가 아니다**. 후자는 connect handoff race(cmd_tx 저장 갭) 재동기용 `ConnectionCommand::Resync`(→ main_loop
/// resubscribe_and_sweep)로, 여기 slot 단위 mount replay 와 목적·경로가 다르다(다음 세션 혼동 방지). 로컬 축 B
/// 만 쓰므로 다중 창 replay storm 이 없고, replay→live 순서는 actor 직렬화(ADR-0043)가 보장한다. 정상 mount
/// 에서 배정 트리거 replay 와 중복될 수 있으나 프론트 seq dedup(`lastDeliveredSeq`, ADR-0037)이 흡수한다.
///
/// ★window label 자동 주입★: `tauri::Window` 를 인자로 받아 호출 webview 라벨을 얻는다(위조 불가 —
/// `subscribe_output` 동형). agent_id 만 프론트가 넘긴다.
///
/// ★필터 로직은 `router.slots_for_window_agent` 에 격리(테스트 가능성 — FIX-A)★: 이 command 는
/// `tauri::Window`/`State` 결합으로 headless 회귀가 막히므로(WebView2 DLL), "그 창이 그 agent 를 보는 slot 만
/// 고른다" 는 실제 필터 배선을 순수 메서드에 모아 두고 여기선 얇게 위임만 한다. 그 메서드의 회귀 그물 =
/// `output_router.rs` 테스트(unknown agent no-op / window 필터 / 정상 slot 선택).
#[tauri::command]
pub fn resync_output(
    router: State<'_, Arc<crate::output_router::OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
    window: tauri::Window,
    agent_id: String,
) -> Result<(), String> {
    let agent_id: AgentId = parse_uuid(&agent_id, "agent_id")?;
    let label = window.label().to_string();
    // ★그 창이 보는 slot 중 이 agent 만 필터★: router 는 lock-free(ArcSwap) 조회. 이 창(label)이 실제로
    //   그 agent 를 보는 slot 만 replay 대상으로 남긴다 — 안 보는 agent 를 넘겨도 slots 가 비어 no-op.
    let slots = router.slots_for_window_agent(&label, agent_id);
    // ★빈 slots 진단 로그(FIX-C — 조용한 성공 구분)★: 대상 slot 이 없으면(그 창이 그 agent 를 안 봄)
    //   replay_slots 가 no-op 이라 아무 일도 안 일어난다. 정상적으로 slot 이 없는 경우(remount 와 layout
    //   재배정이 엇갈린 순간 등)도 있어 에러는 아니지만, 조용히 Ok(())로 삼키면 "resync 를 불렀는데 화면이
    //   안 채워진다" 진단이 막힌다 → debug 로 남긴다(저빈도 mount 경로라 핫패스 부담 0 — logging-conventions).
    if slots.is_empty() {
        tracing::debug!(
            agent = %agent_id,
            window = %label,
            "resync_output: 그 창이 보는 해당 agent slot 이 없어 replay 미발생(no-op)"
        );
    }
    // fresh=true — Channel (재)등록/remount = viewer 재시작이라 cursor 리셋 + 전체 replay(subscribe_output
    //   등록 트리거와 동형, 근원2). slots 비면 replay_slots 가 no-op.
    client.replay_slots(slots, true);
    Ok(())
}

/// ★T7c: TauriTransport.send() 진입점★. 프론트 ProtocolClient 가 AgentCommand wire 객체를
/// JSON 으로 보내면 Rust DaemonClient 를 통해 데몬으로 전달한다.
///
/// ★계약★: `cmd` 는 AgentCommand 의 externally-tagged JSON(e.g. `{"Kill":{…}}`). 파싱 실패는
/// 에러 반환(JSON 구조 불일치).
///
/// ## ★reply 평면 (Fix-B / 안 ②)★
/// 프론트 ProtocolClient 는 reply 를 oneshot(pending 맵)으로 기다리고, 그 reply 를 `onMessage`(control
/// InboundMessage)로 받아 request_id 매칭으로 resolve 한다. 그래서 이 invoke 는 **reply 를 버리지 않고**
/// 데몬 reply(AgentEvent)를 그대로 직렬화해 invoke 반환값으로 돌려준다 — TauriTransport.send 가 그 반환을
/// `onMessage({kind:'control', event})` 로 올려, ProtocolClient.handleEvent 가 기존대로 pending 을 깬다.
/// (WsTransport 가 데몬 Text frame 을 control 로 올리는 것과 동형 — broadcast 낭비·다창 누수 없는 oneshot 경로.)
///
/// ★request_id 유무로 경로 분기 (hang 방지)★:
/// - request_id 있는 명령(spawn/kill/write/list/snapshot/profile CRUD/…) → `send_command`(reply await).
///   ProtocolClient 가 pending 을 거는 명령은 *전부* request_id 를 가지며(resizePty 만 예외=fire-and-forget),
///   데몬은 그 명령들에 reply variant(Ack/Spawned/Created/AgentList/ProfileList/Snapshot) 또는 Error 를
///   echo 한다(connection_core.rs dispatch). 즉 send_command await 가 영구 점유되는 명령은 없다.
/// - request_id 없는 명령(Resize/Subscribe/Unsubscribe) → fire-and-forget. 반환 `Ok(None)`(올릴 reply 없음).
///   (Subscribe/Unsubscribe 는 ProtocolClient 가 fire-and-forget 으로 보낸다 — pending 안 검.)
///
/// 반환 `Ok(Some(value))` = 프론트로 올릴 control event(reply), `Ok(None)` = 올릴 것 없음.
#[tauri::command]
pub async fn forward_daemon_command(
    client: tauri::State<'_, std::sync::Arc<DaemonClient>>,
    cmd: serde_json::Value,
) -> Result<Option<serde_json::Value>, String> {
    // AgentCommand 로 파싱해 request_id 유무로 경로 분기.
    let agent_cmd: engram_dashboard_protocol::AgentCommand =
        serde_json::from_value(cmd).map_err(|e| format!("AgentCommand 파싱 실패: {e}"))?;

    // ★데몬 구독 소유 = src-tauri 단독(ADR-0035/0037 — BLOCK-1)★: 프론트가 보낸 Subscribe/Unsubscribe
    //   는 데몬으로 흘리지 않고 여기서 차단(drop)한다. 데몬 구독·재구독은 layout 구독 델타(ViewManager
    //   권위, commands/layout.rs send_subscription_delta)가 `after_seq=버퍼 최신 seq`(축 A)로 단독
    //   트리거한다 — 프론트가 `Subscribe{after_seq:null}`(FromOldest)를 N창에서 forward 하면 데몬이
    //   FromOldest 를 N번 replay 해 공유 버퍼 seq 단조(무손실 전제)가 붕괴하기 때문이다. 프론트
    //   ProtocolClient 는 이미 subscribeOutput 첫 구독에서 Subscribe 를 안 보내지만, resubscribeAll
    //   (재연결 resume)·미래 carrier 변경이 다시 보낼 여지가 있어 Rust 가 무시로 2차 방어한다
    //   (프론트가 안 보내거나 Rust 가 무시 — 어느 쪽이든 데몬 직접 구독 0). reply 없는 명령이라 None 반환.
    if matches!(
        agent_cmd,
        engram_dashboard_protocol::AgentCommand::Subscribe { .. }
            | engram_dashboard_protocol::AgentCommand::Unsubscribe { .. }
    ) {
        tracing::debug!(
            cmd = ?agent_cmd,
            "forward_daemon_command: 프론트 Subscribe/Unsubscribe 차단(데몬 구독 소유=layout 델타, BLOCK-1)"
        );
        return Ok(None);
    }

    // request_id 없는 명령(Resize)은 reply 가 안 와 send_command 가 hang 이므로 fire-and-forget 으로
    // 보낸다(반환 None — 프론트로 올릴 reply 없음). (Subscribe/Unsubscribe 는 위에서 이미 차단됨.)
    if crate::daemon_client::protocol_state::command_request_id(&agent_cmd).is_none() {
        client.send_fire_and_forget(agent_cmd);
        return Ok(None);
    }

    // request_id 있는 명령 — reply 를 await 해 그대로 프론트로 돌려준다(ProtocolClient 가 pending 매칭).
    // ★reply 직렬화★: 데몬 reply(Ack/Spawned/…)는 externally-tagged AgentEvent 라, 직렬화하면
    //   `{"Ack":{"request_id":…}}` 형태가 그대로 나온다 — 프론트 handleEvent 가 기대하는 wire 형태와 동형.
    // ★끊김 처리★: send_command 가 Err(연결 끊김/응답 못 받음)면 그 메시지를 프론트로 전달해
    //   ProtocolClient.sendCommand 의 send().catch 가 해당 pending 을 reject 하게 한다(영구 hang 차단).
    //
    // ## ★reply 타임아웃 (Fix-C ③ — 영구 hang 2차 차단)★
    // send_command 의 끊김 감지는 *carrier 레벨* 끊김(소켓 죽음 → 연결 task 종료 → drain/oneshot drop)만
    // 잡는다. 그러나 "연결은 살아있는데 데몬이 그 request_id 응답만 누락"하는 경로가 있다:
    //   - 데몬 dispatch(connection_core.rs)의 Ack/Error sink.enqueue 가 SinkError 로 *무시*(`let _ =`)
    //     되는 경우(큐 포화 등, ADR-0020 R6 의도된 behavior-preserving) → reply 가 영영 안 온다.
    // 그러면 send_command 의 `reply_rx.await` 는 carrier 가 안 끊겨 영구 대기 → invoke 가 안 끝나 →
    // 프론트 ProtocolClient.pending 도 영구 hang(send().catch 도 안 불림). 이를 닫으려 reply 대기에
    // 상한을 둔다. 타임아웃 시 Err → invoke reject → 프론트 send().catch → 해당 pending reject.
    //
    // ★값(30s)★: 데몬은 명령을 enqueue 한 *직후* Ack 를 sink 로 보낸다(무거운 작업 완료를 기다리지
    //   않음 — connection_core.rs dispatch). 정상 경로의 왕복은 loopback 에서 수 ms 라 30s 에 절대 안
    //   닿는다. 넉넉히 잡아 "느린 데몬"을 타임아웃으로 오판하지 않으면서, 무한 hang 만 확정적으로 끊는다.
    //
    // ★데몬측 enqueue 실패 승격 대신 클라측 타임아웃을 택한 이유★: connection_core.rs:535 의 enqueue
    //   실패 무시를 "연결 종료로 승격"하면 ADR-0020 R6(behavior-preserving)을 깨고 데몬 crate 회귀 위험이
    //   크다(정상 단일 연결도 일시적 큐 포화로 끊길 수 있음). 타임아웃은 src-tauri(클라) 레이어에 격리돼
    //   데몬 동작을 안 건드리고, 락 순서(ADR-0006)와도 무관하다 — send_command 는 락을 잡았다 즉시 풀고
    //   Sender clone 만 반환하므로 이 timeout 래핑은 추가 락을 보유하지 않는다.
    const REPLY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
    match tokio::time::timeout(REPLY_TIMEOUT, client.send_command(agent_cmd)).await {
        Ok(Ok(event)) => {
            let value =
                serde_json::to_value(&event).map_err(|e| format!("reply 직렬화 실패: {e}"))?;
            Ok(Some(value))
        }
        Ok(Err(e)) => Err(e),
        Err(_elapsed) => Err(
            "daemon reply timeout — 연결은 살아있으나 데몬이 응답을 보내지 않음(30s 초과)"
                .to_string(),
        ),
    }
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
