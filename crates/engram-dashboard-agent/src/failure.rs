//! failure — the kind vocabulary of the 「last failure」.
//!
//! ★Home (ADR-0171's two questions)★: who must see it = every window + the control LLM (→ not in the frontend store); until when = while
//!   the daemon lives (→ not on disk). So it is a **memory-only field** of the profile struct (`AgentProfile::last_failure`).
//!
//! ★This module owns the vocabulary only★: observing a failure and enforcing record and clear is
//!   `manager::AgentManager::note_activation_result` alone, and recognizing the cause in each program's output is the `backend`'s job. The
//!   table kind → {worth retrying · text · recommended action} belongs to the actual consumer (frontend
//!   `src/components/agent/failureKinds.ts`) — do not make a second table here.
//!
//! tauri import 0 — isolation rule observed.
// ADR-0171
// ADR-0172

/// The kind of activation failure.
///
/// ★An **axis separate** from the current state (`AgentStatus`) — do not merge★: merged, 「running while holding a last failure」 cannot be
///   expressed and the screen's 「running wins」 rule has no place to stand (ADR-0172 decision 3 · ADR-0173).
/// ★No serde derive on purpose — do not bring it back★: this value must not go to disk (ADR-0172 decision 2). The profile field is
///   `#[serde(skip)]`, and without the derive, deleting that skip is a **compile error**. The wire mirror is `protocol::AgentFailureKind`.
/// ★No 「worth retrying」 decision here★: the only consumer today is the screen and the wire spec carries only the kind; a table on both
///   sides makes two copies the compiler cannot sync — move it here when a core consumer appears.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentFailureKind {
    /// The conversation to resume has no real existence — never created with the identifier we issued (an entry that died without
    /// exchanging a word). Retrying with the same identifier gives the same result.
    NoConversationToResume,
    /// The process could not be launched (executable missing · permissions · temporary resource shortage, etc.).
    SpawnFailed,
    /// It came up as a resume but exited within the observation window — everything else not recognized by the branches above.
    EarlyExitAfterResume,
    Other,
}
