# 모듈 1 — pty/types.rs 브리핑 (담당: dcs24, Sonnet)

발신: ed12 (매니저)
근거 문서: `docs/backend-lld-stage1.md` §3 (타입 정의). 반드시 §3 원문을 읽고 그대로 따른다.

## 목표

`src-tauri/src/pty/types.rs` 작성. PTY 백엔드 전체가 공유하는 타입·trait 정의.

## 필수 포함 (LLD §3)

```rust
pub type AgentId = uuid::Uuid;
pub type SinkId  = uuid::Uuid;

// 프론트엔드가 discriminated union으로 받도록 internally-tagged 필수
#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "type")]
pub enum AgentStatus {
    Running,
    Exiting,
    Exited { code: Option<i32> },
    Failed { message: String },
    Killed,
}

// drain 내부 전달용 (raw 바이트)
pub struct PtyChunk { pub seq: u64, pub data: Vec<u8> }

// 프론트로 나가는 wire 포맷 (base64 인코딩 문자열)
#[derive(Clone, serde::Serialize)]
pub struct PtyEvent { pub agent_id: AgentId, pub seq: u64, pub data_b64: String }

#[derive(Clone, serde::Serialize)]
pub struct AgentInfo { pub id: AgentId, pub cwd: String, pub status: AgentStatus, pub cols: u16, pub rows: u16 }

// 추상화 trait — 이 파일에 tauri import 절대 금지 (headless 테스트 가능해야 함)
pub trait OutputSink: Send + Sync + 'static {
    fn send(&self, event: PtyEvent) -> Result<(), SinkError>;
    fn sink_id(&self) -> SinkId;
}
pub trait StatusSink: Send + Sync + 'static {
    fn status_changed(&self, id: AgentId, status: AgentStatus);
    fn agent_list_updated(&self, agents: Vec<AgentInfo>);
}

#[derive(thiserror::Error, Debug)]
pub enum SinkError { #[error("sink closed")] Closed, /* 필요시 추가 */ }
```

## ReplayBuffer (LLD §3 — 늦게 붙는 창을 위한 ring buffer)

- 용량 상한 **2MB**. 초과 시 앞에서부터 버림(ring).
- `push(chunk: PtyChunk)`, `snapshot() -> Vec<PtyChunk>` (clone) 제공.
- 내부 구조는 `VecDeque<PtyChunk>` + 누적 바이트 카운터. 구현 자유, 단 2MB 상한 정확히.
- LLD §3에 시그니처 명시돼 있으면 그대로 따를 것.

## 불변 규칙 (리뷰 필수 항목)

- **이 파일에 `use tauri` / tauri 타입 0개.** (`grep "use tauri" src/pty/types.rs` → 0줄)
- `AgentStatus` 에 `#[serde(tag = "type")]` 필수.
- 파생 매크로(Clone/Debug/Serialize)는 LLD에 맞춰. 과하게 달지 말 것.

## 코드 품질 (10년 유지보수)

- 각 타입 위에 **무엇을 위한 타입인지 1줄 한국어 주석**. 자명한 필드엔 주석 금지.
- `cargo fmt` 통과. 논리 블록 사이 빈 줄.

## 빌드 확인 & 보고

- `pty/mod.rs` 에 `pub mod types;` 추가하고 `lib.rs` 에서 `mod pty;` 가 잡히게 최소 연결.
- `cargo build` 통과 확인 (Cargo.toml 의존성은 dco23이 이미 추가함 — uuid/thiserror/serde 사용 가능).
- 완료 보고: `orch 12 "⟁dcs24 types.rs 완료 — cargo build OK"` (실패 시 에러 발췌 첨부)

막히면 30분 내 중간보고. 버전/스펙 변경 필요하면 임의 변경 말고 ed12에 보고.
