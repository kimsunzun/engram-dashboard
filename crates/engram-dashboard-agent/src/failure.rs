//! failure — the kind vocabulary of the 「마지막 실패」 (last failure).
//!
//! ★Home (the answer to ADR-0171's two questions)★: who must see it = every window + the control LLM (→ it cannot be carried in the frontend store) ·
//!   until when must it live = while the daemon lives (→ not kept on disk). So this value lives as a
//!   **memory-only field** of the profile struct (`AgentProfile::last_failure`).
//!
//! ★This module owns the vocabulary only★: observing what is a failure and enforcing record and clear is
//!   `manager::AgentManager::note_activation_result` alone, and recognizing the cause in each program's output
//!   is the `backend`'s job. The table kind → {worth retrying · text · recommended action} is carried by the side that actually consumes that value
//!   (frontend `src/components/agent/failureKinds.ts`) — do not make a second table here.
//!
//! tauri import 0 — isolation rule observed.
// ADR-0171
// ADR-0172

/// The kind of activation failure.
///
/// ★An **axis separate** from the current state (`AgentStatus`) — do not merge them★: if merged, the combination "running while holding a last failure"
///   cannot be expressed, and without that combination the screen's 「도는 중이 이긴다」 (running wins) rule has no place to stand
///   (ADR-0172 decision 3 · ADR-0173).
/// ★The serde derive was left off on purpose — do not bring it back★: this value must not go out to disk (ADR-0172 decision 2).
///   The profile field is `#[serde(skip)]` so it does not go out even now, but without the derive, a change that deletes that skip becomes a
///   **compile error**. The mirror that goes out as the wire spec is carried separately by `protocol::AgentFailureKind`.
/// ★The 「다시 해볼 가치가 있나」 (is it worth retrying) decision is not put here★: today the only consumer reading that value is the screen
///   (click blocking · 「새 대화로 시작」 (start as a new conversation) are follow-ups), and the wire spec carries only the kind. Putting a table on both sides makes two copies the compiler
///   cannot keep in sync — when the consumer comes to the core, it is moved here then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentFailureKind {
    /// The conversation to resume has no real existence — a conversation was never created with the identifier we issued (an entry that died without exchanging
    /// a single word). Retrying with the same identifier gives the same result.
    NoConversationToResume,
    /// The process could not be launched (executable missing · permissions · temporary resource shortage, etc.).
    SpawnFailed,
    /// It came up as a resume but exited within the observation window — everything else not recognized by the branches above.
    EarlyExitAfterResume,
    /// Not classified into any branch.
    Other,
}
