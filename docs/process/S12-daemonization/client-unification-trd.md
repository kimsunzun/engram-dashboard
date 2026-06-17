# TRD — 클라이언트/백엔드 경로 통합 (ADR-0020 구현)

결정·거부대안은 ADR-0020. 이 문서는 **어떻게**(단계·인터페이스·테스트). 4단계, 각 단계 = 코더(opus)→reviewer-deep→QA(build/test+cdp)→게이트 후 커밋. 단계마다 WS 테스트 green 유지(behavior-preserving), 단위테스트 누적 갱신.

## 절단선 요약
```
바이트/소켓 만지는 모든 것 = 어댑터 (carrier별)
AgentCommand → ConnectionCore.dispatch → AgentManager → Outbound = core (공유)
```

## Stage 1 — 백엔드: ConnectionCore 추출 (behavior-preserving, WS green 유지)
**목표:** `ws.rs`의 dispatch 로직을 transport-중립 ConnectionCore로 빼되 동작 0 변경. WS 어댑터는 그 위에서 구동.

- **신규 trait/타입(daemon crate):**
  - `trait OutboundSink { fn enqueue(&self, out: Outbound) -> Result<(), SinkError>; }` — 응답/이벤트/출력 송신 추상.
  - `enum Outbound { Event(AgentEvent), Binary(Vec<u8>), Close }` — carrier-중립(WsOutbound의 상위 개념). 인코딩(JSON/frame)은 sink 구현이 소유.
  - `struct ConnectionSession { conn_id, subs: HashMap<AgentId,SinkId>, owned_viewports: Vec<(AgentId,String)> }` — per-conn 수명 상태.
  - `struct ConnectionCore { manager, multiview }` — `async fn dispatch(&self, cmd, session, sink: &dyn OutboundSink) -> DispatchFlow`. `enum DispatchFlow { Continue, Close }`(현 bool 대체).
- **이동 대상(ws.rs→connection_core.rs):** dispatch match arm 전부, handle_subscribe(TOCTOU-safe), reply/send_error/event_json(Outbound 경유로 변경), *_to_wire 변환, MultiViewState.
- **WS 어댑터(handle_connection 잔류):** handshake/auth/Origin/protocol_version, ws.split, read_task(frame→cmd), write_task(Outbound→Message), keepalive, close_signal. `WsOutboundSink`가 `OutboundSink` 구현(conn_tx에 push, AgentEvent→JSON text/binary frame 인코딩, 큐 포화 시 SinkError→close_signal).
- **단위테스트:** ConnectionCore를 `MockOutboundSink`(Vec 기록)로 직접 구동 — R1 [Ack,Binary,ReplayComplete] 순서, dispatch arm별 manager 호출. 기존 ws_e2e 44 그대로 green.
- **게이트:** ws_e2e green, core/protocol 무회귀, clippy0, `rg "use tauri" core`=0.

## Stage 2 — 백엔드: Tauri 어댑터 (embedded over 프로토콜)
**목표:** 로컬도 AgentCommand로 말하게. invoke 20개 → generic 1개.

- `#[tauri::command] async fn agent_command(state, cmd: AgentCommand)` — state의 per-conn **inbound mpsc**에 enqueue. **단일 command loop task**가 순서대로 꺼내 `ConnectionCore.dispatch` 호출(★결정2 보강: invoke racing 직렬화★).
- `TauriOutboundSink`가 `OutboundSink` 구현 — `Outbound::Event`→Channel emit, `Outbound::Binary`→base64 PtyEvent Channel(기존 인코딩 유지). 출력 구독은 기존 subscribe_agent_output 경로를 ConnectionCore.Subscribe로 흡수하거나 sink 라우팅.
- src-tauri setup: ConnectionCore 1개 + conn_id 1개 등록(single client) + command loop 기동.
- **단위/통합:** embedded conformance — 같은 ConnectionCore를 Tauri sink로 구동, WS와 동일 명령→동일 manager 효과 단언. racing 테스트(병렬 invoke N개 → 순서 보존).
- **게이트:** 위 + 기존 src-tauri 빌드.

## Stage 3 — 프론트: ProtocolClient over Transport
**목표:** client 1벌 + transport 2개.

- `interface Transport { send(payload): void; onMessage(cb); readonly state; onStateChange(cb); close() }`.
- `WsTransport`(DaemonClient에서 WS 부분 추출: openSocket/Auth/Hello/scheduleReconnect/ws.send·onmessage) / `InProcTransport`(invoke('agent_command',{cmd}) + Channel 수신, state 항상 connected, 재연결 no-op).
- `ProtocolClient implements AgentClient` — request_id pending map, seq high-water dedup, epoch 가드, resubscribe resume를 transport 무관하게 보유(DaemonClient 로직 승격). InProc에선 dedup/재연결이 무해 no-op 수렴.
- `clientFactory`: mode → transport 선택 → `new ProtocolClient(transport)`. EmbeddedClient/DaemonClient 클래스 은퇴.
- **vitest 갱신:** transport mock으로 dedup(replay_from≠high-water)·epoch 리셋·resume·request_id 매칭. InProc mock으로 no-op 수렴. #13133/순서 보존.
- **게이트:** vitest green, tsc0.

## Stage 4 — 옛 경로 삭제 + 실측
- 삭제/축소: 옛 EmbeddedClient invoke 경로, ptyApi(또는 InProcTransport 내부로 흡수), 옛 Tauri commands(agent/profile/pty dispatch — agent_command로 대체). discovery 등 비-에이전트 command는 유지.
- **QA cdp 실측(필수):** embedded·daemon 두 모드 각각 spawn→write echo→interrupt→kill→subscribe replay→profile CRUD 전 경로 WebView2 실측(코드 green ≠ 동작).
- **게이트:** 전체 test + cdp PASS → step-log/ADR 인덱스 갱신.

## 회귀 가드 (ADR-0020 R1~R7)
R1 FIFO·R2 dedup·R3 resume(baseline 커버 확인)·R4 finalize 1회·R5 kill 2동사·R6 close_signal(어댑터 잔류)·R7 코어 격리. 각 단계 QA에서 해당 R 단언.
