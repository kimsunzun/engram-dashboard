# 핸드오프: S14 T7 TauriTransport cutover — 전체 구현 진입 대기 중

> ⚠️ `.claude/continue`는 gitignored(로컬). 이건 master 브랜치 관점.

## 한 줄 상태 + 다음 첫 액션

T7 구현 **전**. 이전 세션이 §11 TRD 작성 완료(spike.md +282줄 미커밋). 이번 세션은 전 파일 리딩만 — 코드 미변경. 다음 세션은 **T7a `protocol_state.rs` 편집부터 시작**. T7a → T7b(output_channel+connection) → T7c(tauriTransport+app.emit) → T7d(GUI 실측) 순서.

## repo 상태

- HEAD = master `0723214`, origin 동기.
- **미커밋(modified):** `docs/process/S14-multi-page-layout/module1-transport-spike.md` +282줄 — §11 T7 TRD 전체. 커밋 안 됨(이전 세션 누락). **T7 구현 완료 후 함께 커밋** 권장.
- Untracked: `.claude/skills/research/study-notes/20260629-multi-viewer-terminal-buffer.md` — N1 근본 리서치 노트(사용자 이해 확인 후 삭제 규약, git 외).

## 구현 상세 (이번 세션에서 모든 파일 읽음 — 다음 세션 추가 리딩 불필요)

### T7a: `src-tauri/src/daemon_client/protocol_state.rs`

**제거:**
- `SubState.last_delivered_seq: Option<u64>` — 이 필드 자체 삭제
- `OutputDecision` enum, `decide_output` fn, `mark_delivered` fn

**추가:**
```rust
pub enum EpochDecision { Deliver, DropEpochMismatch }

pub fn decide_epoch(st: &SubState, frame_epoch: u32) -> EpochDecision {
    if let Some(cur) = st.epoch {
        if frame_epoch != cur { return EpochDecision::DropEpochMismatch; }
    }
    EpochDecision::Deliver
}
```

**변경:**
- `apply_subscribe_ack(st, epoch) -> bool` — epoch 변경 여부 bool 반환(호출자가 window seq 리셋 트리거)
- `resubscribe_params(st: &SubState, after_seq: Option<u64>) -> SubscribeParams` — 외부에서 after_seq 주입

**테스트 조정:**
- seq dedup 테스트 6개(`dedup_same_seq_dropped` 등) → 삭제(로직이 window 레벨로 이동)
- `resubscribe_uses_known_epoch_and_last_seq` → after_seq는 외부 인자로 바꿈
- epoch 관련 4개(`epoch_mismatch_dropped` 등) → `EpochDecision`으로 수정
- pending/drain/InProc/close 테스트 → 변경 없음

### T7b: `src-tauri/src/output_channel.rs`

**전체 재작성.** 현재는 30줄짜리 단순 파일 — 아래로 교체:

```rust
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use crate::output_router::WindowLabel;

pub type AgentId = engram_dashboard_protocol::AgentId; // or use protocol import

pub struct WindowEntry {
    pub channel: tauri::ipc::Channel<tauri::ipc::Response>,
    /// None = 이 창에 아직 이 agent 출력 한 번도 못 받음 (전체 replay 필요).
    /// Some(n) = 마지막으로 렌더한 seq.
    pub render_seqs: HashMap<AgentId, Option<u64>>,
}

impl WindowEntry {
    pub fn new(channel: tauri::ipc::Channel<tauri::ipc::Response>) -> Self {
        Self { channel, render_seqs: HashMap::new() }
    }
}

pub type WindowChannelRegistry = Arc<Mutex<HashMap<WindowLabel, WindowEntry>>>;

/// seq 가드 — per-window. None(첫 frame) 또는 Some(last) < frame_seq 면 배달.
pub fn should_deliver(entry: &WindowEntry, agent_id: AgentId, frame_seq: u64) -> bool {
    match entry.render_seqs.get(&agent_id) {
        None | Some(None) => true,
        Some(Some(last)) => frame_seq > *last,
    }
}

pub fn mark_rendered(entry: &mut WindowEntry, agent_id: AgentId, seq: u64) {
    entry.render_seqs.insert(agent_id, Some(seq));
}

/// epoch 변경 시 해당 agent 의 모든 창 render_seq 리셋 (None = 전체 replay 필요).
pub fn reset_all_windows_for_agent(registry: &WindowChannelRegistry, agent_id: AgentId) {
    if let Ok(mut reg) = registry.lock() {
        for entry in reg.values_mut() {
            entry.render_seqs.insert(agent_id, None);
        }
    }
}

/// resubscribe after_seq 산출 — 모든 창의 min(render_seq). 하나라도 None 이면 None(전체 replay).
pub fn min_render_seq(registry: &WindowChannelRegistry, agent_id: AgentId) -> Option<u64> {
    let Ok(reg) = registry.lock() else { return None };
    let mut min: Option<u64> = None;
    let mut any = false;
    for entry in reg.values() {
        if let Some(seq_opt) = entry.render_seqs.get(&agent_id) {
            any = true;
            match seq_opt {
                None => return None, // 하나라도 None → 전체 replay
                Some(seq) => min = Some(min.map_or(*seq, |m: u64| m.min(*seq))),
            }
        }
    }
    if any { min } else { None }
}
```

### T7b: `src-tauri/src/daemon_client/connection.rs`

**Binary arm 변경:**
```rust
// 기존:
if let OutputDecision::Deliver { seq } = decide_output(sub, epoch, seq) {
    if !labels.is_empty() && fan_out(&bytes, &labels, registry, my_gen) {
        mark_delivered(sub, seq);
    }
}

// 변경 후:
if let EpochDecision::Deliver = decide_epoch(sub, frame.epoch) {
    let labels = router.targets(frame.agent_id);
    if !labels.is_empty() {
        fan_out_per_window(&bytes, frame.agent_id, frame.seq, &labels, registry, my_gen);
    }
}
```

**SubscribeAck arm 변경:**
```rust
// 기존:
protocol_state::apply_subscribe_ack(sub, current_epoch);

// 변경 후:
if protocol_state::apply_subscribe_ack(sub, current_epoch) {
    // epoch 변경 → 모든 창 render_seq 리셋(새 스트림 전체 replay)
    output_channel::reset_all_windows_for_agent(&registry, agent_id);
}
```

**main_loop resubscribe 변경:**
```rust
// 기존:
let p = protocol_state::resubscribe_params(subs.entry(*agent_id).or_default());

// 변경 후:
let after_seq = output_channel::min_render_seq(&registry, *agent_id);
let p = protocol_state::resubscribe_params(subs.entry(*agent_id).or_default(), after_seq);
```

**Subscribe command arm 동일하게 변경** (line ~944).

**`fan_out` → `fan_out_per_window` 재작성:**
```rust
fn fan_out_per_window(
    bytes: &[u8],
    agent_id: AgentId,
    frame_seq: u64,
    labels: &[WindowLabel],
    registry: &WindowChannelRegistry,
    my_gen: u64,
) {
    let mut dead: Vec<String> = Vec::new();
    {
        let Ok(mut reg) = registry.lock() else { ... return; };
        for label in labels {
            if let Some(entry) = reg.get_mut(label) {
                if output_channel::should_deliver(entry, agent_id, frame_seq) {
                    if entry.channel.send(tauri::ipc::Response::new(bytes.to_vec())).is_err() {
                        dead.push(label.clone());
                    } else {
                        output_channel::mark_rendered(entry, agent_id, frame_seq);
                    }
                }
                // should_deliver=false → skip (이미 본 frame, 이 창엔 dedup)
            }
        }
        for label in &dead { reg.remove(label); }
    }
    if !dead.is_empty() { tracing::debug!(...); }
    // bool 반환 없음 — high-water는 이제 per-window, SubState에서 관리 안 함
}
```

**`src-tauri/src/commands/agent.rs` `subscribe_output` 변경:**
```rust
// 기존:
reg.insert(label, channel);
// 변경 후:
reg.insert(label, output_channel::WindowEntry::new(channel));
```

### T7c: Rust — `app.emit` 추가

**`connection.rs` Text arm broadcast 처리 추가:**
현재 TODO: `// TODO(emit): AppHandle 을 task 에 주입해 broadcast 를 위로 emit.`

- `run_connection` 파라미터에 `app: tauri::AppHandle` 추가
- `main_loop` 파라미터에 동일 추가  
- `DaemonClient` struct에 `app: tauri::AppHandle` 필드 추가 (lib.rs에서 주입)
- Text arm에서 broadcast variant 처리:
  ```rust
  AgentEvent::AgentListUpdated { agents } => { let _ = app.emit("agent-list-updated", agents); }
  AgentEvent::StatusChanged { agent_id, status, epoch } => { let _ = app.emit("status-changed", ...); }
  AgentEvent::RestoreResult { .. } => { let _ = app.emit("restore-result", ...); }
  AgentEvent::ProfileListUpdated { profiles } => { let _ = app.emit("profile-list-updated", profiles); }
  ```
- 연결 상태(Connected/Down/Reconnecting) 변경 시 `app.emit("daemon-connection-state", state_str)`

### T7c: Frontend — `tauriTransport.ts` + `clientFactory.ts`

**설계 결정(미확정 — 다음 세션에서 선택):**
- Option A: 새 `forward_daemon_command(cmd)` Tauri invoke 추가 → ProtocolClient.send() 유지
- Option B: TauriTransport가 AgentClient를 직접 구현(ProtocolClient bypass)

권장: Option A. ProtocolClient의 request_id 매칭이 TauriTransport에서도 필요하므로 ProtocolClient 유지가 깔끔.

**`src/api/tauriTransport.ts` 구조:**
```typescript
export class TauriTransport implements Transport {
  // subscribe_output Channel → decode frame → call messageCb({ kind: 'output', ... })
  // Tauri event 'agent-list-updated' → messageCb({ kind: 'control', event })
  // Tauri event 'status-changed' → messageCb({ kind: 'control', event })
  // Tauri event 'daemon-connection-state' → update _state + notify listeners
  
  send(cmd): void {
    // invoke('forward_daemon_command', { cmd }) — Rust이 route
    // Subscribe/Unsubscribe는 no-op (Rust이 내부 관리)
  }
  
  ensureReady() { invoke('ensure_daemon_ready') }
  start() { invoke('daemon_start') }
  close() { invoke('daemon_close') }
}
```

**`src/api/clientFactory.ts`:**
```typescript
// import { TauriTransport } from './tauriTransport'
// instance = new ProtocolClient(new TauriTransport())
```

### T7d: GUI 실측

```bash
WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223" npm run tauri dev
node scripts/cdp.mjs shot out.png
node scripts/cdp.mjs eval "window.__TAURI__.core.invoke('agent_spawn', ...)"
```

## 검증 상태

- **green(이전 세션):** `cargo test --workspace` 148 green. `cargo fmt --check` 0.
- **검증 안 됨:**
  - T7 코드 전혀 안 씀 — 빌드/테스트 미실행
  - GUI output 도달 (G2) 미실측
  - TauriTransport 구현 미결정 사항 있음 (Option A vs B)

## 실패한 접근 / do-not

- **T7c 설계 미확정:** TauriTransport.send(cmd) 구현 방식. Option A(forward_daemon_command) 권장이나 다음 세션에서 Rust 커맨드 추가 비용 고려 후 확정.
- **ProtocolClient SubState와 TauriTransport 불일치:** ProtocolClient는 여전히 JS-side lastDeliveredSeq/epoch를 갖는다. TauriTransport에서는 이게 항상 통과하므로 무해(이중 방어). 단, resubscribeAll이 Subscribe wire를 보낼 때 TauriTransport가 이를 무시해야 한다(Rust이 내부 관리) — 이 부분 T7c 구현 시 주의.
- **spike.md §11 미커밋** — 이번 세션에도 커밋 안 됨. T7 완료 후 함께 커밋 권장.

## 참조

- `src-tauri/src/daemon_client/protocol_state.rs` — SubState, decide_output, mark_delivered, resubscribe_params, 테스트 (T7a 변경 대상)
- `src-tauri/src/output_channel.rs` — 현재 30줄 단순 파일 (T7b 재작성 대상)
- `src-tauri/src/daemon_client/connection.rs` — Binary arm(L851~885), fan_out(L992~1036), main_loop resubscribe(L780~810), SubscribeAck(L832~849), Subscribe arm(L944~953) (T7b 변경 대상)
- `src-tauri/src/commands/agent.rs` — subscribe_output L132 (T7b: WindowEntry::new 변경)
- `src-tauri/src/lib.rs` — L170~180 (T7c: AppHandle 주입)
- `src/api/clientFactory.ts` — T7c 교체 대상
- `src/api/transport.ts` — Transport 인터페이스 (TauriTransport 구현 계약)
- `src/store/eventBus.ts` — broadcast 이벤트 소비자 (T7c 이후 연결)
- `docs/process/S14-multi-page-layout/module1-transport-spike.md` §11 — T7 TRD 정본 (미커밋)
