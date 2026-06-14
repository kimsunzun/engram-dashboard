# Phase 2 데몬 구현 설계 — consult 교차검증 병합 (2026-06-14)

근거: `/consult` 3종(GPT·Gemini·Claude-opus) 블라인드 + judge. 원자료: `agents/web-runner/shared/20260614-151528-consult-daemon-phase2/`. 코드 직접 검증 병합. judge가 GPT 기여를 Gemini로 오귀속 → 매니저 정정(아래는 정정 반영).

## 모델별 기여·오류 (un-blind)
- **GPT(가장 신뢰, 오류 0):** ★ReplayBuffer **event-count cap** 부재 위험(1B 청크 폭주→replay event 폭증→신규 구독자 mpsc 즉시 full→재연결마다 slow-consumer 영구 끊김) + ★**WsStatusSink** 누락(status/list/restore_result도 WS text frame 필수, 안 그러면 agent list 갱신 불능→종료 판정 불가). 둘 다 코드-실재.
- **Claude(오류 0, co-best):** WMI **2층 구조** 분리(상위 Tauri→데몬 WMI 1회 / 하위 데몬→PTY portable-pty N회, breakaway 불필요) + ACL **동일 SID 무력성**(같은 사용자 프로세스는 ACL로 못 막음, 진짜 방어=커맨드라인 토큰 노출 차단) + replay가 raw 저장임을 정확 인지.
- **Gemini(오류 2 — 폐기):** ①split 구조에서 "Control Lane 데드락"은 실재 안 함(read/write 독립 task라 write backpressure가 read 안 막음) ②WMI race·CREATE_SUSPENDED를 PTY child spawn에 오적용(PTY child는 portable-pty 직접 spawn이라 WMI 무관) ③ReplayBuffer가 base64 저장이라는 오인(코드상 raw 저장). 고유 기여: Stale Port Hijacking/Graceful Shutdown/Malicious Flood 하네스 케이스(유효).

## 3종 합의 (judge 확인, 채택)
1. **"코어 변경 0" = 거짓.** 현 emit이 raw→base64 인코딩 후 PtyEvent로만 fanout(sink엔 base64만 감, raw는 ReplayBuffer에만). WS는 raw 필요 → decode·re-encode 왕복. **raw-first로 전환 필수.**
2. **try_send를 std thread(동기)에서 호출 안전** — non-async·non-blocking. 금지는 send().await/blocking_send.
3. **core는 protocol 무의존, 데몬이 번역.** SpawnSpec 개명(core profile::AgentCommand→SpawnSpec, serde variant 유지로 디스크 호환).
4. **연결당 단일 writer 큐(conn_tx).** SplitSink 동시 write 불가. 출력 frame·control JSON 모두 단일 conn_tx 합류.
5. **연결 종료 시 그 연결의 모든 (agentId, SinkId) 일괄 unsubscribe**(누수 방지) — 연결별 레지스트리.
6. **SubscribeAck→replay→live 순서 race 실재** — 단일 conn_tx enqueue 순서로 직렬화.

## 병합 구현 설계 (착수 가능)

### Step 1 — 코어 raw 경계화 (★최우선, 코드-검증 가능)
- `core/pty/types.rs`: `OutputSink::send(&self, chunk: &OutputChunk)` 로 시그니처 변경(현 `PtyEvent` → raw `OutputChunk{seq, data:Vec<u8>}`).
- `core/pty/output_core.rs`: emit에서 base64 인코딩 제거, ReplayBuffer push·fanout 모두 raw `&OutputChunk`. subscribe replay 동일.
- `src-tauri/lib.rs` ChannelOutputSink: send 안에서 base64::encode → PtyEvent → Tauri Channel(Embedded 동작 보존). **base64 책임을 sink로 이동** = 코어가 transport-agnostic.
- WsOutputSink(데몬, phase2 Step4): raw → `protocol::encode_terminal_frame`(디코드 왕복 0).
- 검증: core unit 38 + GUI E2E(Embedded base64 디코드 회귀, phase1b와 동일 방식).
- ★주의: PtyEvent(base64)는 Embedded 전용 타입으로 ChannelOutputSink 내부 격리. protocol wire와 무관.

### Step 2 — SpawnSpec 개명
core `profile::AgentCommand`(Claude/Shell) → `SpawnSpec`. 참조처(profile/backend/manager) 일괄. serde variant 표현 유지(agents.json 호환). protocol `AgentCommand`(envelope) 무변경.

### Step 3 — protocol 보강
- **Auth 첫 frame 추가**(현 protocol AgentCommand에 없음 — 누락). `ClientHello{token, protocol_version}` 또는 별도 auth frame.
- ReplayBuffer **max_events cap 추가**(GPT): `max_bytes`(2MB)와 함께 `max_events`(예 4096). evict 조건 `total_bytes>max_bytes || events.len()>max_events`. 불변식: **replay_max_events ≤ ws_out_queue_cap − control_slack**(예 4096 ≤ 4608−512) — 신규 구독자가 replay만으로 큐 넘쳐 끊기는 것 방지.
- seq u64의 JS 표현(number 2^53 한계) — **사용자 결정**(현 number 매핑).

### Step 4 — engram-dashboard-daemon bin
- `#[tokio::main]`. `Local\EngramDashboardDaemon-{user_sid_hash}` named mutex(ERROR_ALREADY_EXISTS=이미 실행). bind 127.0.0.1:0 → daemon.json{port,pid,token,protocolVersion} atomic(persistence tmp+rename).
- `Arc<AgentManager>` 소유. spawn_agent/restore_all 등 블로킹 작업은 **spawn_blocking**(restore_all은 sleep+3s 윈도라 필수).
- 연결당: `accept_hdr_async`(Origin allowlist, upgrade 전) → 1s 내 Auth frame(없으면 close) → `ws.split()`.
  - **read_task**(독립): AgentCommand JSON 파싱 → manager 호출. Subscribe 시 WsOutputSink(conn_tx clone) 등록 + (agentId,SinkId) 레지스트리 기록. client binary frame은 protocol error close. **read는 write backpressure와 독립**(Gemini 데드락 우려는 여기서 자동 해소).
  - **write_task**(독립, 단일 writer): conn_tx(bounded)에서 `WsOutbound::{Text(AgentEvent), Binary(frame), Close}` 받아 SinkHalf write. SubscribeAck/StatusChanged/AgentListUpdated/RestoreResult 모두 Text로 이 큐 통과(순서 보존).
  - **supervisor**: read/write 중 하나 종료 시 cleanup_connection(레지스트리 순회 unsubscribe 전부).
- **WsOutputSink**: sync `send(&OutputChunk)` → `encode_terminal_frame`(raw) → conn_tx.try_send. Full→Err(코어 dead-sink 제거) + `mark_backpressured`(conn closing swap + Close 신호 = 그 연결 전체 정리).
- **WsStatusSink**(GPT): status_changed/agent_list_updated/restore_result → `WsOutbound::Text(AgentEvent::...)` try_send. 빠지면 list 갱신 불능.
- **WMI 2층(Claude)**: WMI Win32_Process.Create는 **Tauri→데몬 spawn에만**(breakaway 우회). 데몬→PTY child는 기존 portable-pty+`job.assign`(platform/windows.rs) 그대로 — WMI/CREATE_SUSPENDED 불필요. AssignProcessToJobObject 실패 시 child 즉시 kill+Err(GPT, orphan 방지).

### Step 5 — Tauri→데몬 discovery/spawn (DaemonClient는 phase4)
daemon.json 읽기→없/stale(pid 죽음)면 WMI로 데몬 spawn→port/token 회수. WMI Create는 fully-qualified exe path + cwd/quoting 검증 필수(GPT).

### Step 6 — 격리 하네스 (integration test + harness bin 하이브리드)
- Rust integration test(`daemon/tests/`): codec/auth/subscribe-order/backpressure/멀티agent 역다중화/연결종료 unsubscribe/base64-제거 raw회귀/epoch mismatch/event-cap.
- 별도 harness bin: 데몬 kill→PTY child 정리/WMI spawn path/long-running high throughput/ACL cross-user(--ignored). assert_cmd+tempfile+tokio-tungstenite로 데몬을 child process로.
- 추가 케이스(GPT/Gemini): Auth timeout 1s/Auth가 binary면 reject/malformed JSON/oversized frame/protocol_version mismatch/Stale Port Hijacking/Graceful Shutdown(StopDaemon)/Malicious Flood(auth 전 정크 바이너리).

## 미해결 — 사용자 결정
1. **SubscribeAck 순서 보장 방식**: (a) 데몬 단일 conn_tx enqueue 순서(코어 변경 0 유지) vs (b) OutputCore에 `SubscriptionSink::send_subscribe_ack` 추가(더 견고하나 코어 변경). 권장 (a)(코어 최소 변경 원칙).
2. **seq u64 JS 표현**: number(현재, 2^53 한계) vs bigint/string.
3. **port.json ACL 강도**: 명시 DACL(현 사용자+SYSTEM+Administrators vs 현 사용자 only) vs LOCALAPPDATA 상속 의존. 동일 SID는 ACL로 못 막으므로 실이득 제한 — **보안 담당 결정 영역**.
4. **송신 큐 모델**: 연결당 단일 conn_tx(단순, HOL 가능) vs agent별 큐+select! 다중화(HOL 완화). 권장 후자(단일 writer 유지).
5. **Tauri WebView2 실제 Origin** 문자열 실측(allowlist 등록용).
