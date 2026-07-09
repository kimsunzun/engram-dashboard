# 핸드오프: T7c BLOCK — reply/출력 경로 누락, Fix-B 진행 대기

> ⚠️ `.claude/continue`는 gitignored(로컬). 이건 master 브랜치 관점.

## 한 줄 상태 + 다음 첫 액션

T7a+T7b 완료, T7c 구현됐으나 리뷰어 BLOCK. 다음 세션은 **T7c BLOCK 수정(Fix-B)부터** — Tauri 레이어에서 reply variant(`Ack`/`Spawned`/`Error` 등)를 `app.emit`으로 WebView에 올리고, `subscribeOutput`이 Tauri Channel invoke를 호출하도록 수정.

## repo 상태

- HEAD = master `0723214`, origin 동기.
- **전체 미커밋(T7a/b/c 전부 unstaged):**
  - `src-tauri/src/daemon_client/protocol_state.rs` — T7a
  - `src-tauri/src/output_channel.rs` — T7b
  - `src-tauri/src/daemon_client/connection.rs` — T7b+T7c
  - `src-tauri/src/commands/agent.rs` — T7b
  - `src-tauri/src/daemon_client/mod.rs` — T7c
  - `src-tauri/src/lib.rs` — T7c
  - `src-tauri/src/commands/discovery.rs` — T7c (신규)
  - `src/api/tauriTransport.ts` — T7c (신규)
  - `src/api/clientFactory.ts` — T7c
  - `docs/process/S14-multi-page-layout/module1-transport-spike.md` — §11 TRD +282줄
- **BLOCK 수정 완료 후 T7a~T7c 전체 한 번에 커밋** 권장.
- `C:\Users\kimsunzun\.claude\settings.json` — `model: sonnet → opus` 변경(로컬, git 외).

## T7c BLOCK 결함 상세

### [HIGH-1] 출력 frame 전멸

`ProtocolClient.subscribeOutput`은 JS 내부 콜백만 등록하고 `subscribe_output` invoke(Tauri Channel 열기)를 호출하지 않음. `TauriTransport`는 Binary output을 `onMessage`로 올리지 않고 Tauri Channel로만 보냄 → 터미널 출력이 WebView에 전혀 안 보임.

**Fix-B 방향:** `TauriTransport`에 `subscribeOutput(agentId, channel)` 메서드 추가 → `invoke('subscribe_output', ...)` 호출. `ProtocolClient.subscribeOutput`이 transport에 위임.

또는 `TauriTransport.send({Subscribe: ...})`를 가로채서 내부적으로 `subscribe_output` invoke로 라우팅.

### [HIGH-2] JS pending 영구 hang

`emit_broadcast`가 `Ack`/`Spawned`/`Error` 등 reply variant를 `_ => {}` no-op으로 버림. `ProtocolClient.sendCommand`가 pending에 request_id를 넣고 영원히 대기 → `spawnAgent`/`killAgent` 등 모든 명령 hang.

**Fix-B 방향:** `connection.rs`의 `emit_broadcast`에 reply variant 추가:
```rust
AgentEvent::Created { request_id, agent_id } => {
    let _ = app.emit("agent-reply", json!({ "requestId": request_id, "result": { "Created": { "agentId": agent_id } } }));
}
AgentEvent::Ack { request_id } => { ... }
AgentEvent::Error { request_id, message } => { ... }
// 실제 variant 이름은 protocol crate 확인 필요
```
`TauriTransport`에서 `listen('agent-reply', ...)` → `onMessage({ kind: 'reply', ... })` → `ProtocolClient`가 pending resolve.

### [MED] close 후 리스너 유실

`TauriTransport.close()` 후 재연결 시 `listen` 재등록 경로 없음 → control 이벤트 전부 유실. `start()`/`ensureReady()` 호출 시 리스너 재등록 필요.

### [MED] app:None silent Ok

테스트 경로에서 `app: None`이면 연결 task 미spawn인데 `Ok(())`로 반환. 통합 테스트 위양성 가능성.

## 검증 상태

- **green:** `cargo test --workspace` 145개 통과 (T7a+T7b 완료 시점). `npm test` 111개 통과. `npx tsc --noEmit` 오류 없음.
- **검증 안 됨:**
  - T7c BLOCK 미수정 — 수정 후 재빌드/테스트 필요
  - GUI 출력 실측(T7d) 미실행
  - reply 경로 실제 동작 미확인

## 실패한 접근 / do-not

- **T7c Option A(forward_daemon_command)만으로 완결 불가** — reply 경로를 별도로 뚫어야 함(emit). 코더가 reply를 버렸음.
- **ProtocolClient에 transport 종류 분기 넣기 금지** — transport가 무엇인지 ProtocolClient가 알면 안 됨. Fix-B처럼 Tauri 레이어에서 emit, ProtocolClient는 이벤트만 소비.

## 참조

- `src-tauri/src/daemon_client/connection.rs` — `emit_broadcast` 헬퍼 (reply variant 추가 대상)
- `src-tauri/src/daemon_client/protocol_state.rs` — 프로토콜 event variant 확인
- `crates/engram-dashboard-protocol/src/` — `AgentEvent` 실제 variant 목록 확인 필수
- `src/api/tauriTransport.ts` — `listen('agent-reply', ...)` 추가 대상
- `src/api/protocolClient.ts` — `subscribeOutput`, `sendCommand`, pending 맵 (변경 최소화)
- `docs/process/S14-multi-page-layout/module1-transport-spike.md` §11 — T7 TRD 정본

## 기타 메모

- 용어 통일 작업 TODO: "Rust"/"프론트" 대신 "Tauri 레이어"/"WebView" 등으로 통일. 이번 작업 완료 후 별도 진행.
- settings.json `model: opus` — 다음 세션부터 Opus로 실행됨. 현 세션 재시작 필요.
