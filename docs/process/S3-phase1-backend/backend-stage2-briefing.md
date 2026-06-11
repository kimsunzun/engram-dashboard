# 백엔드 2단계 코딩 브리핑

**대상:** 코더 에이전트 (새 세션)  
**작성:** ed12, 2026-06-11

---

## 읽어야 할 파일 (순서대로)

1. `docs/backend-lld-stage1.md` — **설계 계약서. 이게 기준.**
2. `CLAUDE.md` — 프로젝트 현재 상태
3. 이 파일

---

## 네가 할 일

`src-tauri/` 아래 Rust 백엔드를 모듈별로 구현.  
LLD의 시그니처/타입/의사코드를 실제 코드로. **LLD에서 벗어나면 안 됨.**

---

## 구현 순서 (이 순서대로)

### 1. `src-tauri/Cargo.toml` 셋업
LLD §2 의존성 그대로. 버전 절대 임의 변경 금지.

### 2. `src-tauri/src/pty/types.rs`
LLD §3 전체. `AgentId`, `AgentStatus`, `PtyChunk`, `PtyEvent`, `AgentInfo`, `PtyError`, `OutputSink` trait, `StatusSink` trait.

### 3. `src-tauri/src/pty/session.rs`
LLD §4. `PtySession`, `ReplayBuffer`. 필드별 Mutex 구조 정확히.

### 4. `src-tauri/src/pty/drain.rs`
LLD §6. drain thread + 즉시 send + 종료 시퀀스.  
**주의:** `completion channel`(`std::sync::mpsc`) drain_done_tx/rx 포함.

### 5. `src-tauri/src/pty/platform/windows.rs`
LLD §13. `JobObjectHandle` wrapper + `create_and_assign`.

### 6. `src-tauri/src/pty/manager.rs`
LLD §5. `PtyManager` — `sessions: Arc<RwLock<...>>`, `status_sink: Arc<dyn StatusSink>`.  
`spawn_agent`, `subscribe`, `unsubscribe`, `write_stdin`, `resize`, `kill_agent`, `list_agents`, `get_snapshot`, `shutdown_all`.

### 7. `src-tauri/src/logging/mod.rs`
LLD §14. `init_logging`, `set_log_level`. env `ENGRAM_LOG`.

### 8. `src-tauri/src/commands/agent.rs` + `pty.rs`
LLD §8. thin wrapper만. 비즈니스 로직 없음.  
`subscribe_agent_output` → `SinkId` 반환 필수.  
`unsubscribe_agent_output` 추가.

### 9. `src-tauri/src/lib.rs`
`AppState { manager: Arc<PtyManager> }`.  
Tauri Channel 기반 `OutputSink` impl here (commands 층).  
Tauri AppHandle 기반 `StatusSink` impl here.

---

## 코드 작성 규칙

- **주석 많이.** 특히 동시성 관련 (왜 이 순서인지, 왜 이 lock인지).
- 각 함수 위에 LLD 참조 표기: `// LLD §6 drain loop`
- 락 획득 순서 규칙 위반 금지 (LLD §10 참조).
- `pty/` 하위 파일에 `use tauri::` 절대 금지.
- `#[cfg(windows)]` 분기는 플랫폼 모듈로 격리.

---

## 검증 포인트 (각 모듈 완성 후 확인)

| 모듈 | 검증 |
|---|---|
| types.rs | `cargo check` 통과 |
| session.rs | `cargo check` 통과 |
| drain.rs | headless test: spawn → 출력 수신 → kill → join 즉시 완료 |
| manager.rs | headless test: PtyManager::new(MockStatusSink) 직접 생성 가능 |
| commands/ | `cargo build` 통과 |
| 전체 | `npm run tauri dev` → Claude Code 실제 실행 확인 |

---

## 2단계 첫 스파이크 (코드 짜기 전 먼저)

`src-tauri/src/pty/spike.rs` 임시 파일로:
```rust
// Windows 실측 검증
// 1. portable-pty spawn → stdout 수신 → child.kill() → master.take() → join 즉시 완료?
// 2. 완료 시간 측정 (5초 이내여야 함)
```
통과하면 본 구현 진행. 실패하면 ed12(manager)에 보고.

---

## 완료 후

각 모듈 완성할 때마다 `orch 12 "⟁ed12 [모듈명] 완료"` 보고.  
전체 완료 후 `npm run tauri dev` 로 실제 동작 확인.
