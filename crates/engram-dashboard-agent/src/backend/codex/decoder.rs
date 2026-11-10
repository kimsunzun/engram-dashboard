//! codex app-server **notification** → neutral [`OutputEvent`] translator.
//!
//! ★The only door in is [`OutputDecoder`]★ — it takes byte chunks of arbitrary size, and line reassembly is also done
//!   here. The path branches on the envelope (a request carries a duty to respond, a response carries a waiting ticket), so it parses that line
//!   once and feeds its **original bytes as they are** back into this door — each notification line costs one more
//!   parse.
//!   ★Why that cost is accepted (this is the spot where you will want to build a second door again)★: the pump **moves** the decoder
//!   as `Box<dyn OutputDecoder>` (`transport/stdio.rs`). So a door outside the trait is
//!   **unreachable** on the production path, and using it would take the line reassembly · cap · resync · UTF-8 discipline here, and the tests
//!   that guard them, wholly off the shipping path. The route where the path builds its own instance separately
//!   also splits the same value into two copies (the observation map and the "once per name" guard diverge). **One parse is cheaper than that.**
//!
//! ★The translation (notification → event) itself is pure★ — no I/O and no clock, and the same notification always
//!   yields the same event (the only global state is the usage interpretation's 「로그 한 번」 (log once) marker, and it does not touch the output). ★But **framing
//!   depends on call history**★: the state an instance holds is the partial-line buffer · the resync flag · three diagnostic counters ·
//!   the usage observation the pump clears line by line, and the first two change the output (even for the same chunk, depending on earlier calls
//!   an event may or may not come out). So goldens are applied to a **new instance**.
//!
//! ★The usage limit notification goes out through [`OutputDecoder::take_usage`], not as an event★ — it is account-level state, so
//!   it must not go through per-agent replay · subscriber fan-out (interpretation = this folder's `usage`).
//!
//! ★Unknown things are dropped — do not use `Structured` as an **outlet**★ (TRD §6-2). If that escape hatch is used as the default
//!   path, the frontend prints `kind` as a label and draws the payload as a code block, ending up **showing codex method
//!   names and protocol JSON on the user's screen.** Instead **everything dropped is left as counts · logs**
//!   (`observe`). The price = unknown signals move from the screen to the log (TRD §4-7 bears that accounting).
//!   ★Exception = fields **inside** the usage limit notification★: once the envelope is read, if just one field (including the window position) is wrong, only that field is quietly
//!   dropped. What is left in the log is only when a bucket · window length is dropped and when no field is left in a window, and even that is without counting,
//!   once per process (this folder's `usage`).
//!   ★But it is not that `Structured` is never emitted at all★ — one echoed user message goes out in that vocabulary.
//!   That is **not letting unknown things leak but carrying a known thing into a neutral marker**, so it is not caught by the discipline
//!   above (there is no codex vocabulary inside `kind="user"`).
//!
//! ★The turn boundary is emitted by `turn/completed` **alone**★ — that one notification line is [`OutputEvent::TurnEnd`] exactly
//!   once, whatever status value it carries (even none) (TRD §6-1). Emitting on every `item/completed` would split one turn
//!   into several boundaries. `turn/started` is not translated.
//!   ★**Live** translation does not emit [`OutputEvent::MessageDone`]★ — that one means "one message closed"
//!   and what is needed here is "one turn closed **with this outcome**". A turn has several completed items, so the two
//!   are not the same event (measured).
//!   ★But **history restore uses that vocabulary — do not read this as 「이 번역기는 MessageDone 을 안 낸다」 (this translator never emits MessageDone)**★:
//!   a page carries no outcome, so there is no value to fill into `TurnEnd` ([`history_turn_boundary`] is the source of truth for that
//!   choice). The two vocabularies split by origin, and the consumer's render path is one.
//!   ★But **whether to raise that boundary to the screen is not decided here**★ — this translator has no material to answer 「이 종료가 우리 턴의
//!   것인가」 (is this end that of our turn): our thread · turn id. That decision is carried by the path that classifies envelopes, so
//!   the `TurnEnd` emitted here may be blocked or deferred (source of truth = `note_turn` in this folder's `transport`).
//!
//! ★Known overlap — the decoder of `backend/claude/mod.rs` also does NDJSON line reassembly★. ★But it is not the same
//!   code★: that one appends a whole chunk to the buffer, drains from the front line by line, measures the cap **after
//!   appending**, and empties with `clear()`. This one walks only forward over a shrinking slice, measures the cap **before
//!   appending**, and returns even the capacity with `mem::take`. `flush` also diverged (that one parses the tail, this one discards
//!   — the reason is in that impl's comment). So "de-duplication" is not a move-and-paste but **first deciding which algorithm
//!   to unify on**, and it touches both the shared transport layer and the claude implementation.

// chunk 1 only sets up the types and the translator and does no wiring at all. Plugging this decoder into the pipe
//   for real is chunk 2, and this allow is removed then.
#![allow(dead_code)]

use std::collections::{BTreeMap, VecDeque};

use engram_dashboard_base::logging::mask_secrets;
use serde_json::Value;

use super::protocol::{self, method, Inbound};
use super::usage;
use crate::transport::OutputDecoder;
use crate::types::{OutputEvent, QueuedInputEvent, ToolCategory, ToolOutcome, TurnOutcome};
use crate::usage::UsageObservation;

/// Upper bound (in characters) on the peer's string carried in one log line. ★Error bodies reaching 4KB have been measured★
/// (an unknown-method error lists all 160 valid methods) — if not truncated, the log is covered by it.
const LOG_STRING_LIMIT: usize = 512;

/// Upper bound (in characters) applied to **each piece the peer gave** in an error string that goes to the screen.
///
/// ★Not merging first and cutting at the end is intentional★ — in concatenation order the most actionable information (`steer`
/// = the sentence to feed as-is into the next turn) comes last, and cutting at the end would lose it first.
/// ★All six pieces must pass this cap for the total to be bounded★ — if even one is missed, "the total is bounded" becomes false.
/// Pieces = `message` · the `codexErrorInfo` label · `additionalDetails` · `errorType`
/// · `detailedExplanation` · `steer.message`.
const ERROR_PART_LIMIT: usize = 1024;

/// Maximum bytes one line may use. ★No normal line larger than this is known★ — the largest measured is an error body of about 4KB.
/// Without a cap, a peer that never sends a newline takes memory through this one buffer.
const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;

/// Upper bound (in characters) on one piece of **transcript body** that goes to the screen.
///
/// ★The purpose of this cap is **length**, not masking★ — so these pieces pass through [`clip`] and not [`sanitize`]
/// (reason = the [`clip`] doc).
/// ★Without it, one screen wipes out that agent's entire replay★: if a line grown up to [`MAX_LINE_BYTES`] becomes
/// one event, that event's weight alone exceeds the replay ring's **single-event cap (2MiB)**, and the ring evicts all the rest to fit
/// that one (`output_core::Ring::push`). That is, one paste erases
/// the rewind of that conversation.
/// ★Basis of the value★: the body a person pastes at once is below this (a piece of a log file · one source file),
/// and even in the worst case (4-byte characters + everything escaped) it does not exceed a quarter of the ring cap.
const MAX_TRANSCRIPT_CHARS: usize = 64 * 1024;

/// Maximum bytes of the identifier strings used for comparison · correlation.
///
/// ★They are **filtered**, not truncated★ — these values are not for people to read but **tokens to compare for equality** later,
/// so if stored truncated, two different long ones would read as the same (the same judgment is made by this folder's `transport`
/// `MAX_TURN_ID_BYTES` — the value is the same too). Observed ids are UUIDv7 strings (36 bytes).
/// ★This cap is also used to filter when remembering an approval item the path rejected★ — for an id over this cap no result goes out, so the memory cannot
/// find its pair (ADR-0241).
pub(super) const MAX_ID_BYTES: usize = 128;

/// The maximum bytes `args_json` of [`OutputEvent::ToolCall`] may use.
///
/// ★The contract of this field is 「backend 스키마 그대로」 (the backend schema as is), and that contract holds **only within this cap**★ — if exceeded,
/// an abbreviated version keeping only the item's `type` · `id` is carried instead (truncated JSON cannot even be parsed, so it is not truncated).
/// The reason for the cap is the same as [`MAX_TRANSCRIPT_CHARS`] — one big patch item wipes out the replay.
const MAX_TOOL_ARGS_BYTES: usize = 256 * 1024;

/// Maximum number of distinct keys the diagnostic map keeps (the total, including drift · defects).
const MAX_UNTRANSLATED_KEYS: usize = 256;

/// The share that **routine** (known name · known variant) keys may use.
///
/// ★The reason for the split = so that the cap does not switch off the detector itself★: with one pool, routine
/// names alone fill the cap, and when drift actually arrives there is no entry left to record it. With the shares separated, routine keys
/// cannot grow beyond [`MAX_ROUTINE_KEYS`], so **a drift · defect share remains.**
/// ★The routine ceiling is not written here as a number★ — a counted value goes stale whenever the notification list · item axis changes.
/// **What proves that property is `routine_keys_cannot_crowd_out_drift_keys_in_any_arrival_order`**, and
/// if the share is removed, that test turns red.
const MAX_ROUTINE_KEYS: usize = 128;

/// How many already-raised echoed user message item ids to remember.
///
/// ★What the memory blocks is the **`started`/`completed` pair of the same item**, and the two come adjacent within one turn★ —
/// what can come between them is only the rest of that turn's items, so a few entries are enough. Even if generous, it must not grow as the session
/// gets longer, so there is a cap (the oldest are pushed out first).
/// ★If an id pushed out beyond the cap comes again late, that message stays on the screen twice★ — that is the price of making memory
/// bounded. The frontend filters once more by `uuid` (the json contract of `user_message_event`), so that case is also caught
/// on the screen.
const MAX_EMITTED_USER_ITEMS: usize = 64;

/// Server notification names we **knowingly do not translate** (of the 81 kinds of `ServerNotification` in schema 0.154.0,
/// 73 kinds, excluding the 8 this translator handles — plus names measured later, marked where they stand).
///
/// ★The purpose of this list is only one — to tell the grades apart★: a name here is "not carried over yet", so it is
/// routine (debug), and a name **not** here was newly made upstream, so it is a drift signal (warn).
/// Without the list the two mix in one pool, and the signal that should be seen is buried in routine noise.
/// ★Going stale is the safe direction★ — if upstream removes a name, only a dead entry stays here, and if upstream
/// adds a name, it is not in the list, so it becomes a warn as intended.
const KNOWN_UNTRANSLATED_METHODS: &[&str] = &[
    "account/login/completed",
    "account/updated",
    "app/list/updated",
    "autoApprovalReview/strictReviewRequired",
    "command/exec/outputDelta",
    "configWarning",
    "externalAgentConfig/import/completed",
    "externalAgentConfig/import/progress",
    "fs/changed",
    "fuzzyFileSearch/sessionCompleted",
    "fuzzyFileSearch/sessionUpdated",
    "guardianWarning",
    "hook/completed",
    "hook/started",
    "item/autoApprovalReview/completed",
    "item/autoApprovalReview/started",
    "item/commandExecution/outputDelta",
    "item/commandExecution/terminalInteraction",
    "item/fileChange/outputDelta",
    "item/fileChange/patchUpdated",
    "item/mcpToolCall/progress",
    "item/plan/delta",
    "item/reasoning/summaryPartAdded",
    "item/reasoning/summaryTextDelta",
    "item/reasoning/textDelta",
    "mcpServer/event/stream/notification",
    "mcpServer/oauthLogin/completed",
    "mcpServer/startupStatus/updated",
    "model/rerouted",
    "model/safetyBuffering/updated",
    "model/verification",
    "modelProvider/authRecoveryCompleted",
    "modelProvider/authRecoveryStarted",
    "process/exited",
    "process/outputDelta",
    "project/changed",
    "remoteControl/status/changed",
    "serverRequest/resolved",
    "skills/changed",
    "thread/archived",
    "thread/closed",
    "thread/compacted",
    "thread/deleted",
    "thread/environment/connected",
    "thread/environment/disconnected",
    "thread/goal/cleared",
    "thread/goal/updated",
    "thread/name/updated",
    "thread/project/updated",
    "thread/queue/changed",
    // ADR-0337: comes with the `experimentalApi` opt-in (measured 0.163.0 — M3), not in the 0.154.0 schema.
    "thread/readState/changed",
    "thread/realtime/closed",
    "thread/realtime/error",
    "thread/realtime/item/completed",
    "thread/realtime/item/started",
    "thread/realtime/item/transcript/delta",
    "thread/realtime/itemAdded",
    "thread/realtime/outputAudio/delta",
    "thread/realtime/sdp",
    "thread/realtime/started",
    "thread/realtime/transcript/delta",
    "thread/realtime/transcript/done",
    "thread/reverted",
    "thread/settings/updated",
    "thread/started",
    "thread/status/changed",
    "thread/unarchived",
    "turn/diff/updated",
    "turn/moderationMetadata",
    "turn/plan/updated",
    "turn/started",
    "warning",
    "windows/worldWritableWarning",
    "windowsSandbox/setupCompleted",
];

/// The 19 values of the `type` of the `ThreadItem` union (all of schema 0.154.0). The same purpose as the list above —
/// ★a `type` not here is a variant upstream added, and that is how this union grows★.
///
/// The TRD's translation table lists this union as 20 variants, but the 0.154.0 schema has 19 — the document side is to be corrected, and
/// this list is the generator output as the source of truth.
const KNOWN_ITEM_TYPES: &[&str] = &[
    "userMessage",
    "hookPrompt",
    "agentMessage",
    "functionCallOutput",
    "plan",
    "reasoning",
    "commandExecution",
    "fileChange",
    "mcpToolCall",
    "dynamicToolCall",
    "collabAgentToolCall",
    "subAgentActivity",
    "webSearch",
    "imageView",
    "sleep",
    "imageGeneration",
    "enteredReviewMode",
    "exitedReviewMode",
    "contextCompaction",
];

/// The `ThreadItem` variant by which codex **echoes our input back**.
///
/// ★Not dropping this is the decision★ — an echoed user item is **the material that revives the screen on reattach**
/// (TRD §10-A row 2, user decision). The translator marks it 「우리가 보낸 것」 (what we sent) and lets it through, and the frontend reads only that
/// marker — the backend name never reaches the screen (ADR-0004).
const USER_MESSAGE_ITEM_TYPE: &str = "userMessage";

/// The `ThreadItem` variant that carries the whole assistant body.
///
/// ★In live this variant is **not translated — and that is right**★: the same body has already streamed as `item/agentMessage/delta`
///   and been drawn on the screen, so emitting it again here would leave one answer twice.
/// ★But history restore has no such deltas★ — what a page response carries is only completed items, so if this variant
///   is not carried over, **the restored screen has not one line of the assistant's words** (measured sample: 5 of 120 items are this).
///   So it is carried over only for [`ItemOrigin::History`] — the two live branches are byte-for-byte unchanged.
const AGENT_MESSAGE_ITEM_TYPE: &str = "agentMessage";

/// **Which door did this item come in by.** Three share the same `ThreadItem` union, but the emit rules differ.
///
/// ★Widening the axis from `bool` to this enum is all that history restore added★ — the vocabulary tables ([`KNOWN_ITEM_TYPES`]
///   ·[`TOOL_ITEM_TYPES`]) and the per-variant translators ([`tool_call`]·[`user_message_event`]) are still one set,
///   and what differs is only 「이 출처에서 이 변형을 내나」 (does this origin emit this variant) (ADR-0203: no second vocabulary table is made).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ItemOrigin {
    /// `item/started` — live. The emit point of a tool call.
    Started,
    /// `item/completed` — live. Does **not emit the tool call again** (`Started` already did) — if the end is a failure · refusal, it emits only its
    ///   result ([`OutputEvent::ToolResult`]) (ADR-0241).
    Completed,
    /// `thread/items/list` page — the past record right after resume (ADR-0203). It is a finished item, so it carries both the call and the result.
    History,
}

/// The `ThreadItem` variants carried over as tool calls. The rest have no place in our neutral vocabulary
/// (reasoning · plan · review-mode switch) or are already carried by other notifications.
///
/// ★The path's tool-end trigger also reads this table ([`is_tool_item`])★ — it is where the text held at the `item/completed` of a variant listed here
///   is handed over to the running turn. To widen it, grounds (source or M7-style measurement) must come first that the vendor's pending-input check comes after that variant's end —
///   if handed over at an end without grounds, that text misses the check and is one boundary late (it is not lost — the turn end collects it).
const TOOL_ITEM_TYPES: &[&str] = &[
    "mcpToolCall",
    "dynamicToolCall",
    "collabAgentToolCall",
    "commandExecution",
    "fileChange",
    "webSearch",
];

/// Is this variant a tool item — [`TOOL_ITEM_TYPES`]. The path does not copy the vocabulary separately but calls this (its end is the tool-end
/// trigger — ADR-0235).
// ADR-0231
pub(super) fn is_tool_item(item_type: &str) -> bool {
    TOOL_ITEM_TYPES.contains(&item_type)
}

/// The end-status (`status`) words of a tool item — each variant has its own schema enum (`CommandExecutionStatus` · `PatchApplyStatus` ·
/// `McpToolCallStatus` · `DynamicToolCallStatus` · `CollabAgentToolCallStatus` · read from the t3code generated schema). `webSearch` has
/// no status field.
// ADR-0241
mod tool_status {
    pub(super) const IN_PROGRESS: &str = "inProgress";
    pub(super) const COMPLETED: &str = "completed";
    pub(super) const FAILED: &str = "failed";
    pub(super) const DECLINED: &str = "declined";
    pub(super) const INTERRUPTED: &str = "interrupted";

    /// That variant's schema vocabulary. `webSearch`, which has no status field, and variants outside [`super::TOOL_ITEM_TYPES`] give an empty list — ★empty list =
    /// 「이 변형엔 status 칸이 없다」 (this variant has no status field)★, and the reader of the end status uses it to tell whether the field's absence is routine or a defect.
    pub(super) fn of(kind: &str) -> &'static [&'static str] {
        match kind {
            "commandExecution" | "fileChange" => &[IN_PROGRESS, COMPLETED, FAILED, DECLINED],
            "mcpToolCall" | "dynamicToolCall" => &[IN_PROGRESS, COMPLETED, FAILED],
            "collabAgentToolCall" => &[IN_PROGRESS, COMPLETED, FAILED, INTERRUPTED],
            _ => &[],
        }
    }
}

/// The answer of [`CodexAppServerDecoder::tool_outcome`] — the outcome to carry, and whether its end status was already recorded as anomalous.
// ADR-0241
struct ToolEnd {
    /// The outcome to carry. `None` = nothing to carry (still running · interrupted · no field · anomalous).
    outcome: Option<ToolOutcome>,
    /// The end status was already recorded as drift · defect — ★if set, the caller does not add the `item:` routine count★. If one observation is
    /// counted at two grades, the drift is caught once more in the 「아는 변형, 번역 안 함」 (known variant, not translated) field too.
    noted: bool,
}

impl ToolEnd {
    fn quiet(outcome: Option<ToolOutcome>) -> Self {
        Self {
            outcome,
            noted: false,
        }
    }

    fn noted(outcome: Option<ToolOutcome>) -> Self {
        Self {
            outcome,
            noted: true,
        }
    }
}

/// Is this variant an echoed user message — [`USER_MESSAGE_ITEM_TYPE`].
// ADR-0226
pub(super) fn is_user_message(item_type: &str) -> bool {
    item_type == USER_MESSAGE_ITEM_TYPE
}

/// Grade of an observation that could not be carried over — decides the log level and wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Observed {
    /// A known name · known variant that has not been carried over yet. Routine, so debug.
    Routine,
    /// A name not in the schema (0.154.0) — suspected upstream drift, so warn.
    Drift,
    /// A **known** word the schema did not list in that place, translated as meant — not dropped, but suspected upstream drift, so warn.
    /// Where it is used today = end statuses outside a tool variant's schema vocabulary (`declined` of `mcpToolCall`, etc. — [`tool_status::of`]).
    DriftTranslated,
    /// A known name but the shape differs or it cannot be read — a defect, so warn.
    ///
    /// ★That it is warn is the key★: `RUST_LOG` does not reach the release daemon and the default level is warn, so
    /// if left at debug, **a failure that empties the screen is not recorded at all in the shipping configuration.**
    Malformed,
}

/// codex app-server notification translator.
#[derive(Debug, Default)]
pub(crate) struct CodexAppServerDecoder {
    /// Bytes of the incomplete line after the last `\n`.
    ///
    /// ★Invariant★: **no UTF-8 decoding before a complete line is fixed.** The pump throws arbitrary chunks that know neither line boundaries nor
    /// character boundaries, so a multibyte character can be cut at a chunk boundary.
    /// If we join only as bytes and decode only the line a `\n` arrived for, that cut is naturally absorbed (newline `0x0A` cannot
    /// appear as a UTF-8 continuation byte, so the boundary search is safe at the byte level).
    /// ★The length never exceeds [`MAX_LINE_BYTES`]★ — decided **before** appending.
    buffer: Vec<u8>,

    /// Is it **discarding the whole tail** of a line that exceeded the cap, **up to the next `\n`**.
    ///
    /// ★Merely emptying the buffer is not enough★ — the remaining bytes of that line would be parsed as a "new line" up to the next `\n`
    /// and could produce fake events. Lose only the one polluted line and recover from where that line ends.
    discarding: bool,

    /// Observations that could not be carried over → counts.
    ///
    /// ★A key is **axis prefix + the rest**, and a string made by the peer is always placed only **after** the prefix★ —
    /// so whatever name the peer invents and sends, it cannot leave its own axis and land in another axis's field.
    /// Axes = `method:` (notification name) · `item:` (notification name + item type) · `turn-status:` (turn outcome value)
    /// · `tool-status:` (tool end status — item type + value. drift · missing-field defect, and ★the only **translated** observation in this map★ —
    /// a known word outside the schema vocabulary — [`Observed::DriftTranslated`]) · `shape:`·`params:`·`item-type:`·`item-id:`·`tool:`
    /// (defects) · `line:`·`envelope:` (line · envelope) · `deprecation:`.
    /// Whatever characters the content after the prefix holds, the first segment decides the axis.
    untranslated: BTreeMap<String, u64>,

    /// The count of **observations that could not be recorded** because the map above used up its share. ★It is not the number of distinct keys★ — if the same
    /// name comes ten times, ten is added. If it is not 0, do not read the map above as exhaustive.
    untranslated_dropped: u64,

    /// Item ids for which an echoed user message was already raised — [`MAX_EMITTED_USER_ITEMS`] entries, with the oldest pushed out first.
    ///
    /// ★The reason this field exists = the emit point cannot be pinned to `item/started` alone★: whether that notification also comes for this
    /// item kind is **unmeasured** (if it does not, the user's utterance never appears), and on the other side `item/completed` is measured not to come at all for an
    /// interrupted item (TRD §2 L10). So **whichever comes first** raises it, and the later one is
    /// caught by this memory and quietly dropped.
    /// ★It holds only ids, and only ids that passed [`MAX_ID_BYTES`]★ — whether a body or an id without a cap,
    /// it is a string whose length the peer decides, so this cap that only counts entries does not bound the total.
    emitted_user_items: VecDeque<String>,

    /// Usage observations picked up from `account/rateLimits/updated` — the pump empties them line by line with `take_usage`.
    usage_observations: Vec<UsageObservation>,
}

impl CodexAppServerDecoder {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Observations that could not be carried over and their counts (diagnostic only — it takes no part in the translation result).
    ///
    /// ★A key is a value that **already went through** masking · truncation **at storage time**★ ([`Self::observe`]) — so it is lent out
    /// as is without filtering again here. Putting masking at the storage point rather than the boundary is stronger:
    /// not only the path out through this map but also the path out through the log is covered by the same single pass.
    pub(crate) fn untranslated_observations(&self) -> &BTreeMap<String, u64> {
        &self.untranslated
    }

    /// The **number of observations** that could not be recorded because the share was used up (not the number of keys).
    pub(crate) fn untranslated_dropped(&self) -> u64 {
        self.untranslated_dropped
    }

    /// Count one observation, and log its key **only the first time it is seen**.
    ///
    /// ★The grade decision stands **before** the map state★ — how full the map is must not change the answer to "is this drift".
    /// And routine keys cannot grow beyond [`MAX_ROUTINE_KEYS`], so they cannot
    /// eat into the drift · defect share.
    fn observe(&mut self, key: String, what: Observed, detail: &str) {
        // ★Masking comes **before** truncation★: the pattern of `mask_secrets` requires 20 or more characters after `sk-`/`sk-proj-`,
        //   so if truncated first, a credential straddling the boundary is cut below that quantifier and **escapes
        //   masking**, remaining in the log and this map. Truncation is a length defense (the peer taking memory through name
        //   length), and that defense holds just the same when applied after masking.
        let key = sanitize(&key, LOG_STRING_LIMIT);
        let budget = if what == Observed::Routine {
            MAX_ROUTINE_KEYS
        } else {
            MAX_UNTRANSLATED_KEYS
        };
        match self.untranslated.get(&key).copied() {
            Some(seen) => {
                self.untranslated.insert(key, seen.saturating_add(1));
                return; // A key already logged — it ends here (spam prevention).
            }
            None if self.untranslated.len() < budget => {
                self.untranslated.insert(key.clone(), 1);
            }
            None => {
                self.untranslated_dropped = self.untranslated_dropped.saturating_add(1);
                if self.untranslated_dropped == 1 {
                    tracing::warn!(
                        cap = MAX_UNTRANSLATED_KEYS,
                        "codex app-server: 관측 이름이 몫을 넘었다 — 이후는 개수만 센다"
                    );
                }
                return;
            }
        }

        // `key` already went through masking · truncation above. Only `detail` is filtered here.
        match what {
            Observed::Routine => {
                tracing::debug!(observed = %key, "codex app-server: 아는 이름인데 번역하지 않는다 — 버린다")
            }
            Observed::Drift => tracing::warn!(
                observed = %key,
                reason = %sanitize(detail, LOG_STRING_LIMIT),
                "codex app-server: 상류 드리프트 신호 — 버린다"
            ),
            Observed::DriftTranslated => tracing::warn!(
                observed = %key,
                reason = %sanitize(detail, LOG_STRING_LIMIT),
                "codex app-server: 상류 드리프트 신호 — 아는 낱말이라 뜻대로 옮겼다"
            ),
            Observed::Malformed => tracing::warn!(
                observed = %key,
                reason = %sanitize(detail, LOG_STRING_LIMIT),
                "codex app-server: 읽을 수 없는 줄 — 버린다"
            ),
        }
    }

    /// A notification with a known name but a different shape is a **defect** — drop it but leave that fact.
    ///
    /// ★The log is squashed to once per key★ — `item/agentMessage/delta` is the most frequent
    /// notification in this protocol, so if upstream changes one field, a warning goes to the synchronous file sink **on every token delta**.
    /// A serde error message carries the offending value as is (`invalid type: string "…"`), so it goes through the same
    /// masking · truncation as a body.
    fn parse<T: serde::de::DeserializeOwned>(
        &mut self,
        params: Option<&Value>,
        method_name: &str,
    ) -> Option<T> {
        let Some(params) = params else {
            // Every notification we translate has params as required — if it is absent altogether, it is a defect just like a different shape.
            self.observe(
                format!("params:{method_name}"),
                Observed::Malformed,
                "params 가 없다",
            );
            return None;
        };
        match serde::Deserialize::deserialize(params) {
            Ok(v) => Some(v),
            Err(err) => {
                self.observe(
                    format!("shape:{method_name}"),
                    Observed::Malformed,
                    &err.to_string(),
                );
                None
            }
        }
    }

    /// Carry one notification over into a neutral event. If there is no vocabulary to carry it into, it is an **empty vector** — not an error.
    fn translate(&mut self, method_name: &str, params: Option<&Value>) -> Vec<OutputEvent> {
        match method_name {
            method::ITEM_AGENT_MESSAGE_DELTA => self.text_delta(params),
            method::ITEM_STARTED => self.item(params, method_name, ItemOrigin::Started),
            method::THREAD_TOKEN_USAGE_UPDATED => self.usage(params),
            method::ACCOUNT_RATE_LIMITS_UPDATED => self.rate_limits_updated(params),
            method::ERROR => self.error(params),
            method::TURN_COMPLETED => self.turn_completed(params),

            // ★Only two events come out of this side★ — the echoed user message (if `item/started` came first, it does not come out —
            //   reason = the `item`·`user_message` doc) and the result when a tool end is a failure · refusal (ADR-0241). A tool **call** is
            //   `item/started` alone — if emitted from both sides, one call shows up twice on the screen. Even so the item `type` is
            //   looked at: a new variant comes here too. And if a turn boundary were emitted here, one turn would be split into as many boundaries as there are items
            //   (TRD §6-1) — the boundary is `turn/completed` alone.
            method::ITEM_COMPLETED => self.item(params, method_name, ItemOrigin::Completed),

            // An early signal of upstream drift, so it leaves not only the name but also the body (TRD §4-7 item 2).
            // ★Squashing into one key is intentional★ — every other warning originating from the peer in this file is once per key,
            //   and if only this one fired every time — the daemon default level is warn and the file sink writes synchronously on every event —
            //   this one line would reopen the same flood. The price = the **body** of the second and later notices
            //   is not kept (the arrival count is kept). How often it arrives in 0.154.0 is unmeasured.
            method::DEPRECATION_NOTICE => {
                let payload = params.map(|v| v.to_string()).unwrap_or_default();
                self.observe("deprecation:notice".to_string(), Observed::Drift, &payload);
                Vec::new()
            }

            other => {
                let what = if KNOWN_UNTRANSLATED_METHODS.contains(&other) {
                    Observed::Routine
                } else {
                    Observed::Drift
                };
                self.observe(
                    format!("method:{other}"),
                    what,
                    "스키마(0.154.0) 선언에 없는 알림 이름",
                );
                Vec::new()
            }
        }
    }

    /// `item/agentMessage/delta` → [`OutputEvent::TextDelta`].
    ///
    /// ★Unlike claude, `turn_id`/`message_id` are filled★ — codex carries both values in the notification.
    /// What is lost is only `threadId` — our vocabulary has no thread axis.
    fn text_delta(&mut self, params: Option<&Value>) -> Vec<OutputEvent> {
        let Some(n) = self.parse::<protocol::AgentMessageDeltaNotification>(
            params,
            method::ITEM_AGENT_MESSAGE_DELTA,
        ) else {
            return Vec::new();
        };
        vec![OutputEvent::TextDelta {
            // For a body only length is filtered (no masking — [`clip`]); identifiers are **filtered** by length ([`bounded_id`]).
            text: clip(&n.delta, MAX_TRANSCRIPT_CHARS),
            turn_id: bounded_id(&n.turn_id),
            message_id: bounded_id(&n.item_id),
        }]
    }

    /// `item/started`·`item/completed` — the **notification**-side door that carries the same `ThreadItem` union.
    ///
    /// ★In live, the emit point of a tool call is `item/started` alone★ — if emitted from both sides, one call shows up on the screen
    /// twice. `item/completed` is the place that emits that call's **result**, not the call (only for failure · refusal — [`Self::tool_item`]).
    /// ★But it is unverified whether codex emits `started` for every item★ — if there is an item kind for which only `completed` comes,
    /// that call disappears: if the end is a failure · refusal only the result goes out, and with no row to point at the consumer drops it,
    /// and any other end is left only in the `item:` routine count.
    ///
    /// ★Only the echoed user message does not follow that rule — **whichever comes first** raises it★: for that one, if we hang it on one side,
    /// what is lost is 「유저 자신이 친 말」 (what the user typed themselves), so the cost differs, and both directions have failure cases — whether `started` comes for
    /// this kind is unmeasured, and `completed` is measured not to come at all for an interrupted item
    /// (TRD §2 L10). The second notification is caught by [`Self::emitted_user_items`] and quietly dropped.
    ///
    /// ★The branch table itself is carried by [`Self::item_core`]★ — this door only unwraps the envelope. The history-side door
    /// ([`Self::history_item`]) uses the same table, and that sharing is the real thing behind the ADR-0203 rule 「두 번째 어휘표를 만들지
    /// 않는다」 (do not make a second vocabulary table).
    fn item(
        &mut self,
        params: Option<&Value>,
        method_name: &str,
        origin: ItemOrigin,
    ) -> Vec<OutputEvent> {
        let Some(n) = self.parse::<protocol::ItemNotification>(params, method_name) else {
            return Vec::new();
        };
        self.item_core(&n.turn_id, &n.item, method_name, origin)
    }

    /// One past item received as a page right after resume → 0 to n neutral events (ADR-0203).
    ///
    /// ★This is the only place where the door in is not [`OutputDecoder`] — we state exactly the scope of that exception★:
    ///   a page is **a response, not a notification**, so it does not arrive on the line stream (the path itself carries the envelope-stripped `data[]` over).
    ///   So this door is not **bypassing** the line reassembly · cap · resync discipline but
    ///   **outside the axis that discipline applies to in the first place** — that discipline still covers the entire live stream.
    /// ★An item that comes in here gets the [`ItemOrigin::History`] rules★ — a tool emits the call together with its end result
    ///   (`[ToolCall, ToolResult?]` — [`Self::tool_item`]) and the assistant body is also emitted (`AGENT_MESSAGE_ITEM_TYPE` doc).
    // ADR-0203
    pub(crate) fn history_item(&mut self, turn_id: &str, item: &Value) -> Vec<OutputEvent> {
        self.item_core(
            turn_id,
            item,
            method::THREAD_ITEMS_LIST,
            ItemOrigin::History,
        )
    }

    /// Carry one envelope-stripped item over — the **single branch table** shared by the two doors above.
    fn item_core(
        &mut self,
        turn_id: &str,
        item: &Value,
        method_name: &str,
        origin: ItemOrigin,
    ) -> Vec<OutputEvent> {
        let Some(kind) = item.get("type").and_then(|v| v.as_str()).map(String::from) else {
            self.observe(
                format!("item-type:{method_name}"),
                Observed::Malformed,
                "item 에 `type` 문자열이 없다",
            );
            return Vec::new();
        };
        if kind.len() > MAX_ID_BYTES {
            // ★`type` is both the branch key and the tool-name field★ — without a cap, that string goes straight out as the `name` of
            //   [`OutputEvent::ToolCall`] and is also used as an observation key.
            self.observe(
                format!("item-type:{method_name}"),
                Observed::Malformed,
                "item `type` 이 상한을 넘었다",
            );
            return Vec::new();
        }
        if kind == USER_MESSAGE_ITEM_TYPE {
            // ★Not dropped — this is the material that revives the screen after reattach★: on our side there is no other material
            //   to restore this conversation's user utterances (since no input-time synthetic echo is declared — that
            //   spot in this folder's `mod.rs` · ADR-0193).
            return self.user_message(item, method_name, origin);
        }
        // ★History-only arm — the two live branches do not enter here★ (source of truth for the reason = [`AGENT_MESSAGE_ITEM_TYPE`]).
        if origin == ItemOrigin::History && kind == AGENT_MESSAGE_ITEM_TYPE {
            let Some(text) = item.get("text").and_then(|v| v.as_str()) else {
                self.observe(
                    format!("item-text:{method_name}#{kind}"),
                    Observed::Malformed,
                    "어시스턴트 메시지 item 에 required `text` 가 없다",
                );
                return Vec::new();
            };
            if text.is_empty() {
                // Do not create an empty speech bubble — the consumer also drops an empty delta with the same decision.
                self.observe(format!("item:{method_name}#{kind}"), Observed::Routine, "");
                return Vec::new();
            }
            // ★Emitting as `TextDelta` is **the same precedent** as the claude-side restore★ (ADR-0079: the past assistant
            //   body is carried as one delta). The consumer joins consecutive text, so even when a turn has several body items
            //   it ends up shaped like deltas continuing in live.
            // ★The body passes only [`clip`] — it is not masked★ (reason = the same spot in `user_message_event`:
            //   this is not a diagnostic string but the record of that conversation).
            return vec![OutputEvent::TextDelta {
                text: clip(text, MAX_TRANSCRIPT_CHARS),
                turn_id: bounded_id(turn_id),
                message_id: item.get("id").and_then(|v| v.as_str()).and_then(bounded_id),
            }];
        }
        if TOOL_ITEM_TYPES.contains(&kind.as_str()) {
            return self.tool_item(item, &kind, turn_id, method_name, origin);
        }
        // ★This is where a new variant comes in★ — if no signal is left, when upstream widens the union the only diagnostic that could show
        //   the drift closes its eyes at exactly the point where drift appears.
        let what = if KNOWN_ITEM_TYPES.contains(&kind.as_str()) {
            Observed::Routine
        } else {
            Observed::Drift
        };
        self.observe(format!("item:{method_name}#{kind}"), what, "");
        Vec::new()
    }

    /// One tool-variant item — the emit differs per door.
    ///
    /// - `Started` = one [`OutputEvent::ToolCall`]. The end status is not read.
    /// - `Completed` = one [`OutputEvent::ToolResult`] only when the end is a failure · refusal. Otherwise 0 events + the `item:` routine count — but if the end
    ///   status was already recorded as anomalous ([`ToolEnd::noted`]), the routine count is not added.
    /// - `History` = `[ToolCall, ToolResult?]` in this order — the history page has no start notification, so one finished item carries both.
    ///
    /// ★Normal completion is not emitted — only failure · refusal are carried★: the screen uses only error · refusal counts and the badge, so emitting at every end
    ///   just adds one event per call and gains nothing (the old shell leaves a 「표시할 수 없는 신호」 (a signal that cannot be displayed) line for each such event).
    /// ★The result's `id` passes the same filter as the call ([`bounded_id`])★ — if it was filtered out there is no row to point at, so no result is emitted
    ///   and it is counted as a defect.
    // ADR-0241
    fn tool_item(
        &mut self,
        item: &Value,
        kind: &str,
        turn_id: &str,
        method_name: &str,
        origin: ItemOrigin,
    ) -> Vec<OutputEvent> {
        let mut events = Vec::new();
        if origin != ItemOrigin::Completed {
            match tool_call(item, kind, turn_id) {
                Some(call) => events.push(call),
                None => {
                    // ★A tool variant with no call coming out is not "not translated" but a defect★ — if it is sent off as a routine count (debug),
                    //   an item with a missing required field becomes quiet noise.
                    self.observe(
                        format!("tool:{method_name}#{kind}"),
                        Observed::Malformed,
                        "도구 item 에 `tool` 문자열이 없다",
                    );
                    return events;
                }
            }
        }
        if origin == ItemOrigin::Started {
            return events;
        }
        let end = self.tool_outcome(item, kind);
        match end.outcome {
            Some(outcome @ (ToolOutcome::Failed | ToolOutcome::Declined)) => {
                match item.get("id").and_then(Value::as_str).and_then(bounded_id) {
                    Some(id) => events.push(OutputEvent::ToolResult { id, outcome }),
                    None => self.observe(
                        format!("item-id:{method_name}#{kind}"),
                        Observed::Malformed,
                        "끝 결과를 붙일 도구 item 의 `id` 가 없거나 상한을 넘었다",
                    ),
                }
            }
            Some(ToolOutcome::Completed | ToolOutcome::Refused) | None => {
                // ★Do not add the routine count to an end already recorded as anomalous★ — one observation is counted at one grade only (the same as the defect id case).
                if origin == ItemOrigin::Completed && !end.noted {
                    self.observe(format!("item:{method_name}#{kind}"), Observed::Routine, "");
                }
            }
        }
        events
    }

    /// A tool item's vendor end status (`status`) → neutral outcome. Shared by the two doors `Completed` · `History` (ADR-0203 「두 번째
    /// 어휘표를 만들지 않는다」 (do not make a second vocabulary table)).
    ///
    /// Outcome `None` = no outcome to carry — still running (`inProgress` · a running item on a history page) · interrupted (`interrupted` · that turn's
    ///   outcome row already shows the interruption) · no field (`webSearch` has no status field) · anomalous (below).
    /// ★An anomaly is recorded here and told through [`ToolEnd::noted`]★ — an unknown value · a non-string value (drift) · a missing field in a variant
    ///   that should have a status field (defect · warn — if the field name changes upstream, all failure · refusal badges quietly disappear. Same decision =
    ///   the missing `turn.status` of `turn_completed`).
    /// ★The decision is `status` alone — do not read `exitCode`★: if we re-classify a non-zero exit as failure ourselves, a decision different from the vendor's
    ///   stands. In measurement (codex-cli 0.156.1 · fixture `tool_fail_u2`) a non-zero exit comes as `failed`. `mcpToolCall.error` ·
    ///   `dynamicToolCall.success` are not read either, for the same reason.
    /// ★A command we refused also closes as `failed` (measured fixture `refuse_u2a`)★ — the only field that tells it apart from a real non-zero exit is
    ///   `exitCode` (refusal = `null`), and `aggregatedOutput` is `null` in both. We do not tell them apart by that shape — there are other paths
    ///   that produce the same shape (the approval callback being dropped, etc.) and a false reason would be attached. So this function does not emit `Refused`,
    ///   and attributing our own refusals is the job of the path that remembers the refused item.
    /// ★A known word outside the per-variant schema vocabulary (e.g. `declined` of `mcpToolCall`) is carried over as meant but counted as drift★.
    // ADR-0241
    fn tool_outcome(&mut self, item: &Value, kind: &str) -> ToolEnd {
        let vocabulary = tool_status::of(kind);
        let Some(status) = item.get("status") else {
            if vocabulary.is_empty() {
                return ToolEnd::quiet(None); // `webSearch` — a variant with no field.
            }
            self.observe(
                format!("tool-status:{kind}#<없음>"),
                Observed::Malformed,
                "도구 item 에 required `status` 가 없다",
            );
            return ToolEnd::noted(None);
        };
        // ★A peer string is used as a key only when it is a known value★ (reason = the same spot in `turn_completed`) — unknown values are
        //   squashed into one entry per variant, and the actual value goes out in the log body.
        let unknown_key = || format!("tool-status:{kind}#<모르는 값>");
        let Some(word) = status.as_str() else {
            self.observe(
                unknown_key(),
                Observed::Drift,
                &format!("문자열이 아닌 도구 끝 상태: {status}"),
            );
            return ToolEnd::noted(None);
        };
        let outcome = match word {
            tool_status::FAILED => Some(ToolOutcome::Failed),
            tool_status::DECLINED => Some(ToolOutcome::Declined),
            tool_status::COMPLETED => Some(ToolOutcome::Completed),
            tool_status::IN_PROGRESS | tool_status::INTERRUPTED => None,
            other => {
                self.observe(
                    unknown_key(),
                    Observed::Drift,
                    &format!("모르는 도구 끝 상태: {other}"),
                );
                return ToolEnd::noted(None);
            }
        };
        if !vocabulary.contains(&word) {
            self.observe(
                format!("tool-status:{kind}#{word}"),
                Observed::DriftTranslated,
                "이 도구 변형의 스키마 어휘에 없는 끝 상태",
            );
            return ToolEnd::noted(outcome);
        }
        ToolEnd::quiet(outcome)
    }

    /// One echoed user message item → 0 to 1 「우리가 보낸 것」 (what we sent) marker events.
    ///
    /// ★Whichever of `item/started` or `item/completed` comes **first** raises it★ (reason = [`Self::item`] doc).
    ///   The second notification is dropped by the item id comparison, and that comparison is an **exact match** — the schema marks that `id` as required
    ///   (0.154.0 `UserMessageThreadItem`).
    /// ★If there is no id, only the `item/started` side raises it★ — with no key to compare, raising both would leave the same words on the screen
    ///   twice, and the frontend's dedup key is also that id, so it does not catch there either. It means a required field is empty, so it is
    ///   counted as a defect.
    /// ★The second notification dropped as the same one is not counted as an 「옮기지 못한 관측」 (observation that could not be carried over)★ — that map means **what was dropped**,
    ///   and this was already raised. Mixing it in would make that map stop being a drift detector.
    /// ★If `clientId` (the id we carried as `clientUserMessageId`) exists, we emit
    ///   `QueuedInput(Delivered{clientId})` **right before** the bubble — in the two live doors only★. The moment the vendor put that text into the
    ///   history is the receipt (TRD §5-5 「소비 에코」 (consumption echo)). The history door does not emit it: past records are not receipt events,
    ///   and the core door that carries history chunks does not accept list events.
    // ADR-0231
    fn user_message(
        &mut self,
        item: &Value,
        method_name: &str,
        origin: ItemOrigin,
    ) -> Vec<OutputEvent> {
        let at_item_start = origin != ItemOrigin::Completed;
        let id = item.get("id").and_then(|v| v.as_str());
        // It is a comparison token, so it is filtered, not truncated (reason = [`MAX_ID_BYTES`]) — if filtered out, it goes by the item id as it does today.
        let client_id = item
            .get("clientId")
            .and_then(|v| v.as_str())
            .filter(|c| c.len() <= MAX_ID_BYTES);
        // ★What is stored in the memory and compared is only an item id that passed the **same cap as the wire**★ — if the raw form were stored, the entry-count
        //   cap ([`MAX_EMITTED_USER_ITEMS`]) could not bound the length, and a string whose length the peer decides would stay for the whole session.
        // ★This key differs from the bubble's `uuid`★ — `uuid` is `clientId` (one that passed the same cap) first, and only when that is absent is it this
        //   key (see the [`user_message_event`] call below). So if the item id exceeds the cap, this memory cannot catch the `started`/`completed`
        //   pair and the bubble goes out twice, and if there is a `clientId`, `Delivered` also goes out twice. What follows is decided by `clientId`
        //   — if present, the `uuid` of the two bubbles is the same and the frontend's `uuid` dedup folds them into one, and if absent (below the minimum codex version ·
        //   input that came in without an identifier · text we did not put in) `uuid` is empty and two copies remain. The reason for not changing the length decision to truncation is the same as
        //   [`bounded_id`] (if two truncated ones become equal, different words would be folded into one).
        let dedup_key = id.filter(|s| s.len() <= MAX_ID_BYTES);
        if dedup_key.is_some_and(|seen| self.emitted_user_items.iter().any(|k| k == seen)) {
            return Vec::new();
        }
        if id.is_none() {
            self.observe(
                format!("item-id:{method_name}#{USER_MESSAGE_ITEM_TYPE}"),
                Observed::Malformed,
                "유저 메시지 item 에 required `id` 가 없다",
            );
            if !at_item_start {
                return Vec::new();
            }
        }
        let delivered = match (origin, client_id) {
            (ItemOrigin::History, _) | (_, None) => None,
            (_, Some(c)) => Some(OutputEvent::QueuedInput(QueuedInputEvent::Delivered {
                id: c.to_string(),
            })),
        };
        // A user message with no text piece at all (only images · audio · mention) — no empty bubble is made and it is
        //   sent off as a routine count. We have no vocabulary to carry that input kind. A receipt is still a receipt.
        let Some(event) = user_message_event(item, client_id.or(dedup_key)) else {
            self.observe(
                format!("item:{method_name}#{USER_MESSAGE_ITEM_TYPE}"),
                Observed::Routine,
                "",
            );
            return delivered.into_iter().collect();
        };
        if let Some(id) = dedup_key {
            // Push out the oldest first — this memory must not grow just because the session gets longer.
            if self.emitted_user_items.len() >= MAX_EMITTED_USER_ITEMS {
                self.emitted_user_items.pop_front();
            }
            self.emitted_user_items.push_back(id.to_string());
        }
        delivered.into_iter().chain([event]).collect()
    }

    /// `thread/tokenUsage/updated` → [`OutputEvent::Usage`].
    ///
    /// ★`last` is used, not `total`★ — the schema does not explain the meaning of the two fields, so we read them by name,
    /// and the consumer **does not accumulate `Usage` but draws one row per event**. If cumulative values were carried repeatedly,
    /// rows with the same growing number would pile up, and if `last` is carried each row reads alone as "the amount the request that just ended used".
    /// The price = we do not emit a turn's total (you have to add up the rows to get it).
    fn usage(&mut self, params: Option<&Value>) -> Vec<OutputEvent> {
        let Some(n) = self.parse::<protocol::ThreadTokenUsageUpdatedNotification>(
            params,
            method::THREAD_TOKEN_USAGE_UPDATED,
        ) else {
            return Vec::new();
        };
        vec![OutputEvent::Usage {
            // The wire is `int64` and our vocabulary field is `u64` — a negative has no meaning as a token count, so it is clamped to 0.
            //   ★That is better than dropping the whole notification★ (choosing that would lose that turn's usage).
            input_tokens: n.token_usage.last.input_tokens.max(0) as u64,
            output_tokens: n.token_usage.last.output_tokens.max(0) as u64,
            turn_id: bounded_id(&n.turn_id),
        }]
    }

    /// `account/rateLimits/updated` → 0 events. Readable observations are piled up in [`Self::usage_observations`] (interpretation rules =
    /// this folder's `usage`). If the envelope cannot be read, it is left as a defect and dropped like the other arms.
    fn rate_limits_updated(&mut self, params: Option<&Value>) -> Vec<OutputEvent> {
        if let Some(n) = self.parse::<protocol::AccountRateLimitsUpdatedNotification>(
            params,
            method::ACCOUNT_RATE_LIMITS_UPDATED,
        ) {
            self.usage_observations
                .extend(usage::observation_from_rate_limits_updated(&n));
        }
        Vec::new()
    }

    /// `error` notification → [`OutputEvent::Error`]. ★This is not a turn boundary★ — it is a retryable stream error, so
    /// the same turn continues after this line (the boundary is `turn/completed` alone).
    ///
    /// ★What each of the two vocabularies means — this is where consumers diverge★:
    ///   - [`OutputEvent::Error`] = **an incident that happened inside a turn**. The turn continues. If this vocabulary is read as a turn end,
    ///     one turn is split as many times as there are incidents, and the real boundary that comes after is attached to an already ended turn.
    ///   - [`OutputEvent::TurnEnd`] = **the turn has ended**. How it ended is carried by the outcome field inside it
    ///     (including `Failed` — the boundary of a failed turn also arrives in this vocabulary).
    /// ★So 「재시도되나」 (is it retried) is not sent out as a separate wire field★ — **the event type** already carries that distinction,
    ///   and if one more field is made, the same fact lives in two places and one goes stale (the same decision is made by
    ///   [`error_message`] for that string). And this notification has no `threadId`·`turnId` at all
    ///   (`protocol::ErrorNotification`), so **this layer cannot even attribute which turn the incident belongs to** —
    ///   even to promote it to a boundary, there is no material.
    fn error(&mut self, params: Option<&Value>) -> Vec<OutputEvent> {
        let Some(n) = self.parse::<protocol::ErrorNotification>(params, method::ERROR) else {
            return Vec::new();
        };
        vec![OutputEvent::Error(error_message(&n.error, n.will_retry))]
    }

    /// `turn/completed` → [`OutputEvent::TurnEnd`] **exactly once**. The outcome is also carried inside that event.
    ///
    /// ★Do not split the event in two★ — if the failure reason is put ahead as a separate [`OutputEvent::Error`], the 「턴 경계」 (turn boundary) and
    ///   「어떻게 끝났나」 (how it ended) split into two events and consumers get one more ordering contract. The reason a type with an outcome field
    ///   exists is to avoid that split.
    /// ★Emit the event whatever is carried★ — even if there is no status, even if the value is unknown. This one line is the only
    ///   「턴이 끝났다」 (the turn has ended) the peer tells us, so if it is quietly dropped, that conversation's waiting indicator spins forever. So
    ///   only here we do not use `parse::<T>()` and scan fields one by one — even if one field is off, the boundary must stand.
    ///   ★「낸다」 (emit) does not mean it reaches the screen★ — attribution is decided by the path, and it may be blocked or deferred there
    ///   (this file's header). And in the give-up path where the peer **never sends** this line, the path emits its own
    ///   `TurnEnd` — so this spot is not the only emit point of the boundary.
    /// ★Read `status` **as a string — do not receive it as a closed Rust `enum`**★: the day upstream adds a fifth value, the
    ///   **whole** line vanishes with a deserialization failure, the boundary disappears entirely, and the waiting indicator spins forever.
    /// ★The failure decision is an **allowlist — do not flip it to the complement**★: only `failed` counts as failure and
    ///   everything else is not failure. The same spot on the claude side was once a complement (`!= "success"`), and that form
    ///   stamped **a turn the user normally interrupted with Esc** as a failure — in this repository an interrupt is a first-class normal path
    ///   (`TerminalReason::Interrupted` exists separately. The record of that reversal = `backend/claude/mod.rs`'s
    ///   `is_error` spot). The complement also automatically makes any non-error value upstream adds in future a failure.
    /// ★Do not fold `inProgress` into one of the known three★ — we do not know what the combination of that value **arriving on a completion notification**
    ///   means (TRD §6-1 leaves that field as `[미확인]` (unconfirmed)). If we guess a mapping the screen draws a false
    ///   outcome, so it goes to `Unknown` — the turn still ends.
    // ADR-0004
    fn turn_completed(&mut self, params: Option<&Value>) -> Vec<OutputEvent> {
        let turn = params.and_then(|p| p.get("turn"));
        let turn_id = turn
            .and_then(|t| t.get("id"))
            .and_then(|v| v.as_str())
            .and_then(bounded_id);
        let status = turn.and_then(|t| t.get("status")).and_then(|v| v.as_str());

        let outcome = match status {
            Some(protocol::turn_status::COMPLETED) => TurnOutcome::Completed,
            Some(protocol::turn_status::INTERRUPTED) => TurnOutcome::Interrupted,
            Some(protocol::turn_status::FAILED) => {
                // The schema writes 「Only populated when the Turn's status is failed」 on `Turn.error`
                //   (0.154.0). It is still optional so may be absent, and if absent it emits only the failure without a reason.
                let detail = turn
                    .and_then(|t| t.get("error"))
                    .and_then(|v| serde_json::from_value::<protocol::TurnError>(v.clone()).ok())
                    // `willRetry` is not in this field — it exists only in the `error` **notification**, so it is fixed to false.
                    .map(|e| error_message(&e, false));
                TurnOutcome::Failed { detail }
            }
            Some(other) => {
                let what = if protocol::turn_status::ALL.contains(&other) {
                    // A value that is in the declaration but whose meaning in this notification is unknown — not drift but unmeasured.
                    Observed::Routine
                } else {
                    Observed::Drift
                };
                // ★A peer string is used as a key **only when it is a known value**★ — this map uses a bounded
                //   field that never empties, so if an arbitrary string became a key, the peer could burn that budget with different statuses and
                //   **real drift after that would leave only a count, without a name**. Unknown values are squashed into one
                //   entry and the actual value goes out in the log body — that body being squashed to once per key is the same discipline as every other
                //   peer-originated warning in this file (`observe`).
                let key = match what {
                    Observed::Routine => format!("turn-status:{other}"),
                    _ => "turn-status:<모르는 값>".to_string(),
                };
                self.observe(
                    key,
                    what,
                    &format!("이 알림에서의 뜻을 모르는 TurnStatus 값: {other}"),
                );
                TurnOutcome::Unknown
            }
            None => {
                self.observe(
                    format!("shape:{}", method::TURN_COMPLETED),
                    Observed::Malformed,
                    "turn.status 문자열이 없다",
                );
                TurnOutcome::Unknown
            }
        };
        vec![OutputEvent::TurnEnd { turn_id, outcome }]
    }

    /// One complete line (newline excluded) → zero or more events.
    fn consume_line(&mut self, line: &[u8], events: &mut Vec<OutputEvent>) {
        // ★UTF-8 is decoded for the first time here★ (the `buffer` invariant above).
        let text = match std::str::from_utf8(line) {
            Ok(t) => t.trim(),
            Err(_) => {
                self.observe("line:non-utf8".to_string(), Observed::Malformed, "");
                return;
            }
        };
        if text.is_empty() {
            return;
        }

        match protocol::classify(text) {
            Ok(Inbound::Notification { method, params }) => {
                events.extend(self.translate(&method, params.as_ref()));
            }
            // ★This door cannot answer a request — the decoder has no write handle★. So if a request comes in by this
            //   path, that agent **freezes permanently** (the peer waits for an answer).
            //   The path must split envelopes first and take requests itself.
            Ok(Inbound::Request { method, .. }) => {
                self.observe("envelope:request".to_string(), Observed::Malformed, &method)
            }
            Ok(Inbound::Response { .. }) | Ok(Inbound::Error { .. }) => self.observe(
                "envelope:response".to_string(),
                Observed::Malformed,
                "통로가 응답 봉투를 번역기로 넘겼다",
            ),
            // ★Keys are split per reason★ — with one pool, the first broken line met permanently silences every later
            //   envelope defect. The reason this path exists is that a failure that empties the screen must be recorded in the shipping configuration,
            //   and that purpose would end at the first line.
            Err(err) => self.observe(
                format!("envelope:{}", envelope_cause(&err)),
                Observed::Malformed,
                &err.to_string(),
            ),
        }
    }
}

impl OutputDecoder for CodexAppServerDecoder {
    /// Push in a byte chunk of arbitrary size and carry over only the **lines completed** by this chunk. A tail without a newline
    /// stays in the buffer and merges with the next chunk — a notification is not lost even if it is cut at a read boundary.
    ///
    /// A line that cannot be decoded is skipped with only a record left. ★Not treating it as fatal is a defense★: the stdout of 0.154.0
    /// was measured to be only NDJSON (4 runs, 0 banner or non-JSON lines — all errors went to stderr), but there is no guarantee
    /// another version is the same, and if a parse failure cut the stream the agent would die.
    fn decode(&mut self, chunk: &[u8]) -> Vec<OutputEvent> {
        let mut events = Vec::new();
        let mut rest = chunk;

        // If it is discarding a line that exceeded the cap, discard everything up to the newline that ends that line.
        if self.discarding {
            match rest.iter().position(|&b| b == b'\n') {
                Some(nl) => {
                    self.discarding = false;
                    rest = &rest[nl + 1..];
                }
                None => return events,
            }
        }

        // ★`rest` only moves forward★ — it neither drains from the front nor rescans from the start, so a chunk's
        //   total scan cost is linear in the chunk length (even a chunk of 4096 newlines alone makes 0 allocations).
        while let Some(nl) = rest.iter().position(|&b| b == b'\n') {
            let head = &rest[..nl];
            rest = &rest[nl + 1..];

            if self.buffer.is_empty() {
                // The common case — read the slice inside the chunk as it is, without going through the buffer.
                if head.len() > MAX_LINE_BYTES {
                    events.push(overflow_event(head.len()));
                    continue;
                }
                self.consume_line(head, &mut events);
            } else {
                // ★Either way we take the whole buffer and empty it★ — this line ends here, and since ownership
                //   moves away, not only the length but the **capacity** is also given back.
                let mut line = std::mem::take(&mut self.buffer);
                if line.len() + head.len() > MAX_LINE_BYTES {
                    let dropped = line.len() + head.len();
                    drop(line);
                    events.push(overflow_event(dropped));
                    continue;
                }
                line.extend_from_slice(head);
                self.consume_line(&line, &mut events);
            }
        }

        if !rest.is_empty() {
            // ★Decide **before** appending★ — if measured after appending, the cap has already been exceeded at that moment.
            if self.buffer.len() + rest.len() > MAX_LINE_BYTES {
                let dropped = self.buffer.len() + rest.len();
                self.buffer = Vec::new(); // Give back capacity — `clear()` makes only the length 0.
                self.discarding = true;
                events.push(overflow_event(dropped));
            } else {
                self.buffer.extend_from_slice(rest);
            }
        }
        events
    }

    /// Once at stream end.
    ///
    /// ★The shared trait doc says "process the remaining line to the end" and here we **discard** — an intended divergence★:
    /// a line not ended by a newline is **truncated JSON**, so "processing it to the end" would mean inventing the missing rest.
    /// The claude-side implementation is a spot that need not be line-delimited JSON and so processes per that doc, so
    /// the trait doc is left alone. The loss itself is a fact that must be observed, so it is left as a warning.
    fn flush(&mut self) -> Vec<OutputEvent> {
        if !self.buffer.is_empty() {
            tracing::warn!(
                bytes = self.buffer.len(),
                "codex app-server: 개행 없이 스트림이 끝났다 — 잘린 마지막 줄을 버린다"
            );
            self.buffer = Vec::new();
        }
        self.discarding = false;
        Vec::new()
    }

    fn take_usage(&mut self) -> Vec<UsageObservation> {
        std::mem::take(&mut self.usage_observations)
    }
}

/// Stable name for the reason an envelope could not be read. ★The reason the Display of [`ParseError`] is not used as a key★ =
/// its string has the serde error body mixed in, so the same reason becomes a different key every time.
fn envelope_cause(err: &protocol::ParseError) -> &'static str {
    use protocol::ParseError as E;
    match err {
        E::NotJson(_) => "not-json",
        E::NotObject => "not-object",
        E::MethodNotString => "method-not-string",
        E::BadId => "bad-id",
        E::BadError(_) => "bad-error",
        E::MissingId(_) => "missing-id",
        E::ConflictingEnvelope => "conflicting",
        E::UnknownShape => "unknown-shape",
    }
}

/// One `TurnError` → the error string that goes to the screen. ★Both callers must produce the same shape★ — the `error`
/// notification and the `Turn.error` of `turn/completed` are the same type, and if drawn differently the same reason gets two faces.
///
/// ★Do not write 「턴을 끝낸 실패인가」 (is it a failure that ended the turn) into this string as a token★ — **the event type** carries that distinction
///   (the outcome field of [`OutputEvent::TurnEnd`] vs [`OutputEvent::Error`]). If it is written into the string too, the same fact lives in two
///   places and one goes stale.
///
/// ★The schema's only backpressure vocabulary arrives as `codexErrorInfo`★ — `serverOverloaded`
/// ·`rateLimitExceeded` are strings in this field, not numeric error codes. So that label is attached to the body:
/// if not attached, "overload being retried" and "real failure" look the same on the screen.
///
/// ★Every token we add is exactly a protocol field name★ (`willRetry`·`misalignment`·`steer`) —
/// this string is drawn on the screen as is without going through `src/i18n`, so if a sentence is composed here it would become an
/// untranslated UI phrase.
///
/// ★Masking is done **here**, not at the boundary★ — this string goes to the screen and to the log, and over the wire
/// also out as the reason field of [`TurnOutcome::Failed`]. If every outgoing door filtered again, one of them would
/// surely be forgotten (another instance of the same discipline = the error envelope spot of this folder's `transport`). `mask_secrets` is not applied
/// automatically; the caller calls it explicitly (`docs/reference/logging-conventions.md`).
///
/// `will_retry` = the field of the same name in the `error` notification. `Turn.error` has no such field, so it arrives as false.
fn error_message(error: &protocol::TurnError, will_retry: bool) -> String {
    // ★Every piece whose length the peer decides **must pass** [`sanitize`], with none missed★ — only then is the total
    //   bounded (truncation) and no credential carried (masking). If even one piece is reverted to a bare [`truncate`], only that
    //   piece skips masking.
    let mut message = sanitize(&error.message, ERROR_PART_LIMIT);
    if let Some(label) = error.codex_error_info.as_ref().and_then(|v| v.as_str()) {
        message = format!("{message} ({})", sanitize(label, ERROR_PART_LIMIT));
    }
    if will_retry {
        message = format!("{message} [willRetry]");
    }
    if let Some(details) = error.additional_details.as_deref() {
        message = format!("{message}\n{}", sanitize(details, ERROR_PART_LIMIT));
    }
    // ★`message` alone tells neither why it was blocked nor what to do★ — the field that carries those two
    //   is here. `steer` is the "sentence to feed as-is into the next turn" that codex gave, so we do not compose it.
    if let Some(m) = error.misalignment.as_ref() {
        if let Some(kind) = m.error_type.as_deref() {
            message = format!(
                "{message}\nmisalignment: {}",
                sanitize(kind, ERROR_PART_LIMIT)
            );
        }
        if let Some(explanation) = m.detailed_explanation.as_deref() {
            message = format!("{message}\n{}", sanitize(explanation, ERROR_PART_LIMIT));
        }
        if let Some(steer) = m.steer.as_ref() {
            message = format!(
                "{message}\nsteer: {}",
                sanitize(&steer.message, ERROR_PART_LIMIT)
            );
        }
    }
    message
}

/// Raise to the screen the fact that one line was lost for exceeding the cap.
///
/// ★It is a token, not prose★ — this string is drawn as is without going through `src/i18n`, so if a sentence is
/// composed it becomes an untranslated UI phrase (the same discipline is written in [`error_message`]).
fn overflow_event(seen: usize) -> OutputEvent {
    // ★`seen` is **the bytes seen up to that point** and not the whole line length★ — when handed over without a newline,
    //   the rest is quietly swallowed by resync, so nobody counts that total. Writing it as `dropped=` would
    //   read as the line size.
    OutputEvent::Error(format!(
        "codex-decoder: lineOverflow seen={seen}B limit={MAX_LINE_BYTES}B"
    ))
}

/// One echoed user message item → a [`OutputEvent::Structured`] marked 「우리가 보낸 것」 (what we sent).
///
/// `None` = this item has no text piece at all (only images · audio · mention) — the caller sends it off as a count.
///
/// ★The marker = a `kind` of `"user"`, and that alone★ — that is the only marker of 「우리가 보낸 것」 (what we sent), and the consumer
///   reads **only that marker**. No codex vocabulary is carried in the value, so even if another backend uses the same marker the consumer
///   does not diverge (ADR-0004).
/// ★The json shape is a consumer contract★ — `{"type":"text","text":…,"uuid":…}`. Only when `type` is `"text"`
///   does the consumer pick `uuid` as the dedup key. So it must be **identical character for character** to the same shape on the claude side,
///   and that fact is no reason to merge the two copies into one function — merging would force that function to live outside the two backend
///   folders. (That hoist was tried once and reverted.)
/// ★`uuid` = the dedup key chosen by the caller★ — the item's `clientId` (the id we carried as `clientUserMessageId` —
///   the same value as the path's synthetic bubble · list entry, so they fold into one) if present, otherwise **codex's item
///   id**. The schema marks that `id` as required (0.154.0 `UserMessageThreadItem`), so in practice it is always
///   carried, but if absent the field is left empty — a missing key is not a dedup target and is kept as is.
/// ★Several text pieces are joined with newlines★ — the schema makes `content` an array and does not decide a separator between pieces.
///   Writing them together would run the words together, so we split lines (our choice · not a measured convention).
// ADR-0004
// ADR-0045
// ADR-0193
fn user_message_event(item: &Value, uuid: Option<&str>) -> Option<OutputEvent> {
    let text = item
        .get("content")
        .and_then(|v| v.as_array())?
        .iter()
        .filter(|part| part.get("type").and_then(|v| v.as_str()) == Some("text"))
        .filter_map(|part| part.get("text").and_then(|v| v.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() {
        return None;
    }
    Some(user_bubble(&text, uuid))
}

/// One 「우리가 보낸 것」 (what we sent) bubble — the echoed user message and the path's synthetic bubble (Direct) use the same shape.
///
/// ★The json shape is a consumer contract★ (reason = [`user_message_event`] doc). `uuid` is the dedup key, so the caller
///   passes a value already filtered by [`MAX_ID_BYTES`].
/// ★Length is filtered with [`clip`] but **not masked — do not revive masking**★: this is not a diagnostic string but
/// exactly what the user typed, and masking would alter that record (the claude-side user block is also carried on the same carrier without
/// masking — `input_echo_event` of `backend/claude/mod.rs`). What the masking discipline targets is
/// **diagnostic strings**.
// ADR-0231
pub(super) fn user_bubble(text: &str, uuid: Option<&str>) -> OutputEvent {
    let text = clip(text, MAX_TRANSCRIPT_CHARS);

    #[derive(serde::Serialize)]
    struct TextBlock<'a> {
        #[serde(rename = "type")]
        kind: &'static str,
        text: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        uuid: Option<&'a str>,
    }
    let block = TextBlock {
        kind: "text",
        text: &text,
        uuid,
    };
    OutputEvent::Structured {
        kind: "user".to_string(),
        // to_string does not fail for this shape — unwrap_or_default is defensive.
        json: serde_json::to_string(&block).unwrap_or_default(),
    }
}

/// One tool-variant item → [`OutputEvent::ToolCall`].
///
/// `None` means not "not a tool" but **the shape is broken** — the caller filters with [`TOOL_ITEM_TYPES`]
/// first, so an item reaching here is already a tool variant.
fn tool_call(item: &Value, kind: &str, turn_id: &str) -> Option<OutputEvent> {
    let name = match kind {
        // For these three the `tool` field is the tool name itself. The `server` of `mcpToolCall` is not mixed into the name —
        //   that would be inventing a separator, and that value stays as it is in `args_json` below.
        "mcpToolCall" | "dynamicToolCall" | "collabAgentToolCall" => {
            item.get("tool")?.as_str()?.to_string()
        }
        // These three have no tool-name field — the item kind itself is the name.
        _ => kind.to_string(),
    };
    Some(OutputEvent::ToolCall {
        // ★This field is a **display label**, so it is cut, not filtered★ — it is not used for comparison, so even if two cut ones become
        //   equal nothing is lost. The same number is read as a **character count**, so even in the worst case of 4-byte characters it is 512B.
        name: clip(&name, MAX_ID_BYTES),
        args_json: bounded_args_json(item, kind),
        id: item.get("id").and_then(|v| v.as_str()).and_then(bounded_id),
        turn_id: bounded_id(turn_id),
        // A codex item has no concept of a message-group id — the call identifier is carried by the `id` above.
        message_id: None,
        category: tool_category(item, kind),
    })
}

/// Tool-variant item → neutral kind.
///
/// ★Decide from the intact item — do not re-read from `args_json`★: that field, if it exceeds [`MAX_TOOL_ARGS_BYTES`], is
///   a `type`·`id` abbreviation and has no `commandActions`, so one big command would lose its kind.
// ADR-0239
fn tool_category(item: &Value, kind: &str) -> ToolCategory {
    match kind {
        "commandExecution" => command_category(item),
        "fileChange" => ToolCategory::Edit,
        "webSearch" => ToolCategory::Web,
        "mcpToolCall" => ToolCategory::Mcp,
        "collabAgentToolCall" => ToolCategory::Agent,
        "dynamicToolCall" => ToolCategory::Other,
        // The caller filters with [`TOOL_ITEM_TYPES`] first, so what comes here is the case where that table was widened but this decision was not updated.
        _ => ToolCategory::Other,
    }
}

/// The kind of `commandExecution` — decided by `commandActions[].type`, in which codex spells out the command (read vocabulary `read` · `listFiles` ·
/// `search` · `unknown`).
///
/// All `read` = Read · all `listFiles` = List · only `read`·`listFiles` mixed = Read · `search` present and the rest only these
/// three = Search. ★If anything at all is outside the three, it is Command★ — `unknown` is a command codex could not unpack (the `Start-Sleep` of the measured
/// fixture is that), and an unknown word · a missing `type` are treated the same (if wrongly folded into read, a command with side effects
/// would be summarized as 「읽기」 (read)). Command too if the array is absent or empty.
// ADR-0239
fn command_category(item: &Value) -> ToolCategory {
    let Some(actions) = item.get("commandActions").and_then(Value::as_array) else {
        return ToolCategory::Command;
    };
    if actions.is_empty() {
        return ToolCategory::Command;
    }
    let (mut read, mut search) = (false, false);
    for action in actions {
        match action.get("type").and_then(Value::as_str) {
            Some("read") => read = true,
            Some("listFiles") => {}
            Some("search") => search = true,
            _ => return ToolCategory::Command,
        }
    }
    if search {
        ToolCategory::Search
    } else if read {
        ToolCategory::Read
    } else {
        ToolCategory::List
    }
}

/// The whole item as `args_json` — but only within [`MAX_TOOL_ARGS_BYTES`].
///
/// ★Carrying the whole item is the default★ — "what is the argument" differs per variant (command line · patch · query · arguments), so
/// picking out one field would lose the rest at that moment. The contract of this field is "the backend schema as is".
/// ★If exceeded, it is not cut but **replaced with an abbreviation**★ — JSON cut in the middle cannot even be parsed and becomes a 「깨진
/// 값」 (broken value) to the consumer, but an object keeping only `type`·`id` is still a subset of the same schema. So what is lost is
/// **only the arguments**, and 「이 도구가 불렸다」 (this tool was called) remains.
/// ★The reason for the cap itself is the same as [`MAX_TRANSCRIPT_CHARS`]★ — if one entry exceeds the replay ring's single-event cap
/// the ring evicts all the rest.
/// ★This warning is not squashed to once per key★ — that squashing belongs to [`CodexAppServerDecoder::observe`] and this
/// function does not hold `&mut self`. A big item is not the kind that pours in like token deltas, so it was left as it is.
fn bounded_args_json(item: &Value, kind: &str) -> String {
    let full = item.to_string();
    if full.len() <= MAX_TOOL_ARGS_BYTES {
        return full;
    }
    tracing::warn!(
        bytes = full.len(),
        cap = MAX_TOOL_ARGS_BYTES,
        kind = %sanitize(kind, LOG_STRING_LIMIT),
        "codex app-server: 도구 item 이 상한을 넘어 인자를 뺀 축약본을 싣는다"
    );
    let mut reduced = serde_json::Map::new();
    reduced.insert("type".to_string(), Value::String(kind.to_string()));
    if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
        if id.len() <= MAX_ID_BYTES {
            reduced.insert("id".to_string(), Value::String(id.to_string()));
        }
    }
    Value::Object(reduced).to_string()
}

/// The boundary event that closes one history turn (ADR-0203).
///
/// ★The decision is that it is **not** the [`OutputEvent::TurnEnd`] that live uses★: that vocabulary requires an outcome ([`TurnOutcome`]),
///   and a page response does not carry **how** a past turn ended. Two choices remain,
///   and both are bad — `Unknown` is honest but the consumer draws a 「모름」 (unknown) marker at every boundary, so something we never asked
///   about covers the restored screen with markers. Folding to `Completed` is an **invented outcome**, which the doc of that
///   type forbids head-on.
/// ★[`OutputEvent::MessageDone`] closes the turn without claiming an outcome★ — the consumer draws it as one divider.
///   The claude restore also closes with the same vocabulary (ADR-0079 — its transcript has no outcome line either).
/// ★★The closing itself is load-bearing — do not remove it★★: if the restored part ends with only progress signals (text · tool · user), the consumer's
///   derivation is `!turnDone && items>0`, so **the waiting indicator spins forever though nothing was sent**
///   (a bug measured on the claude side 2026-08-17 · the same spot in ADR-0079).
/// ★It is unrelated to observation (ADR-0113)★ — this event goes out through the door that does not record turn observation
///   ([`crate::output_core::OutputCore::emit_without_turn_observation`]). So busy/idle is
///   not bootstrapped by it.
/// ★The argument is an `Option` to accept a **caller that has already filtered out a turn id over the cap**★ — that filtering is
///   exactly the [`MAX_ID_BYTES`] rule (dropped, not truncated), and `None` means 「이 턴의 id 를 실을 수 없다」 (this turn's id cannot be carried).
///   The boundary itself still stands — not closing the turn because the id could not be carried errs toward the waiting indicator spinning.
// ADR-0203
pub(crate) fn history_turn_boundary(turn_id: Option<&str>) -> OutputEvent {
    OutputEvent::MessageDone {
        turn_id: turn_id.and_then(bounded_id),
        message_id: None,
    }
}

/// Turn a peer-made string into a form that can go into a log — credential masking, then length truncation.
fn sanitize(s: &str, limit: usize) -> String {
    truncate(&mask_secrets(s), limit)
}

/// Bring the **transcript body**, whose length the peer decides, within the cap — ★not masked. Do not revive masking★.
///
/// ★This is where it differs from [`sanitize`]★: that one is for **diagnostic strings** (log · error body · observation key),
///   so masking is the purpose and length is a bonus. This one is for **conversation records**, so its purpose is length alone — if what the user typed and
///   what the agent said were altered by masking, that record would become false (by the same judgment the claude-side user block is also
///   carried on the same carrier without masking).
/// ★So [`truncate`] has two doors, and only those two★ — at the spot where one wants to make a third, one must first answer 「이 문자열은
///   진단인가 기록인가」 (is this string a diagnostic or a record). The item that measures that property in the source is
///   `truncate_is_reachable_only_through_its_two_named_doors`.
fn clip(s: &str, limit: usize) -> String {
    truncate(s, limit)
}

/// **Filter** identifiers used for comparison by length — do not truncate (reason = [`MAX_ID_BYTES`]).
fn bounded_id(s: &str) -> Option<String> {
    (s.len() <= MAX_ID_BYTES).then(|| s.to_string())
}

fn truncate(s: &str, limit: usize) -> String {
    match s.char_indices().nth(limit) {
        Some((cut, _)) => format!("{}…", &s[..cut]),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★All goldens go through the shipping path (`decode`)★ — so that framing is measured together.
    fn decode_line(line: &str) -> Vec<OutputEvent> {
        CodexAppServerDecoder::new().decode(format!("{line}\n").as_bytes())
    }

    fn notification(method: &str, params: Value) -> String {
        serde_json::json!({
            "method": method,
            "params": params,
            "emittedAtMs": 1_764_000_000_000i64,
        })
        .to_string()
    }

    fn notification_line(method: &str, params: Value) -> String {
        format!("{}\n", notification(method, params))
    }

    fn decode_notification(method: &str, params: Value) -> Vec<OutputEvent> {
        CodexAppServerDecoder::new().decode(notification_line(method, params).as_bytes())
    }

    fn delta_line(text: &str) -> String {
        notification(
            "item/agentMessage/delta",
            serde_json::json!({"threadId": "t", "turnId": "u", "itemId": "i", "delta": text}),
        )
    }

    fn text_of(events: &[OutputEvent]) -> Vec<String> {
        events
            .iter()
            .map(|e| match e {
                OutputEvent::TextDelta { text, .. } => text.clone(),
                other => panic!("TextDelta 가 아니다: {other:?}"),
            })
            .collect()
    }

    // ── Line reassembly (trait contract) ───────────────────────────────────────────

    /// ★That a notification is cut at a 4096B read boundary is the reason this contract exists★ — if a piece is dropped,
    /// there is no compile error and no failing test, only output that disappears.
    #[test]
    fn one_notification_split_across_two_chunks_still_emits() {
        let line = format!("{}\n", delta_line("안녕하세요"));
        let bytes = line.as_bytes();
        let cut = bytes.len() / 2;
        let mut d = CodexAppServerDecoder::new();
        assert!(
            d.decode(&bytes[..cut]).is_empty(),
            "줄이 안 끝났는데 이벤트가 났다"
        );
        assert_eq!(text_of(&d.decode(&bytes[cut..])), vec!["안녕하세요"]);
    }

    /// A multibyte character must survive even if cut at a chunk boundary.
    #[test]
    fn a_multibyte_character_split_across_chunks_survives() {
        let line = format!("{}\n", delta_line("한글"));
        let bytes = line.as_bytes();
        // Cut in the middle of the 3 bytes of "한".
        let cut = line.find("한").expect("payload") + 1;
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(&bytes[..cut]).is_empty());
        assert_eq!(text_of(&d.decode(&bytes[cut..])), vec!["한글"]);
    }

    #[test]
    fn two_notifications_in_one_chunk_both_emit() {
        let chunk = format!("{}\n{}\n", delta_line("first"), delta_line("second"));
        let mut d = CodexAppServerDecoder::new();
        assert_eq!(
            text_of(&d.decode(chunk.as_bytes())),
            vec!["first", "second"]
        );
    }

    #[test]
    fn a_complete_line_plus_a_partial_tail_emits_only_the_complete_one() {
        let chunk = format!("{}\n{}", delta_line("done"), &delta_line("partial")[..20]);
        let mut d = CodexAppServerDecoder::new();
        assert_eq!(text_of(&d.decode(chunk.as_bytes())), vec!["done"]);
    }

    /// A line that arrives without a newline is **held** — it comes out when the newline arrives.
    #[test]
    fn a_line_without_a_terminator_is_held_until_one_arrives() {
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(delta_line("later").as_bytes()).is_empty());
        assert_eq!(text_of(&d.decode(b"\n")), vec!["later"]);
    }

    /// A truncated last line is dropped without parsing — no fake event from half a JSON.
    #[test]
    fn flush_drops_a_truncated_trailing_line() {
        let partial = &delta_line("never finished")[..30];
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(partial.as_bytes()).is_empty());
        assert!(d.flush().is_empty());
        // After dropping, the state is clean — the next line does not tangle with that tail.
        let line = format!("{}\n", delta_line("after"));
        assert_eq!(text_of(&d.decode(line.as_bytes())), vec!["after"]);
    }

    #[test]
    fn flush_on_a_clean_stream_is_empty() {
        let mut d = CodexAppServerDecoder::new();
        assert!(d.flush().is_empty());
        let line = format!("{}\n", delta_line("x"));
        assert_eq!(d.decode(line.as_bytes()).len(), 1);
        assert!(d.flush().is_empty());
    }

    /// In a chunk full of newlines, empty lines make neither events nor observations, and the state afterward is clean.
    /// ★This test does not measure cost (whether it is quadratic)★ — measuring needs allocation · time instrumentation, and that property is
    /// carried here by the "only moves forward" comment in the `decode` body.
    #[test]
    fn a_chunk_of_only_newlines_produces_nothing_and_leaves_clean_state() {
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(&vec![b'\n'; 4096]).is_empty());
        let line = format!("{}\n", delta_line("after"));
        assert_eq!(text_of(&d.decode(line.as_bytes())), vec!["after"]);
    }

    /// ★A huge line ending in a newline must also hit the cap★ — if measured only after appending, this path passes.
    #[test]
    fn an_oversized_line_that_ends_in_a_newline_is_still_capped() {
        let mut d = CodexAppServerDecoder::new();
        let mut chunk = vec![b'x'; MAX_LINE_BYTES + 1];
        chunk.push(b'\n');
        let events = d.decode(&chunk);
        assert!(
            matches!(events.as_slice(), [OutputEvent::Error(m)] if m.contains("lineOverflow")),
            "{events:?}"
        );
        // It is a line ended by a newline, so no resync is needed — the next line comes out right away.
        let line = format!("{}\n", delta_line("recovered"));
        assert_eq!(text_of(&d.decode(line.as_bytes())), vec!["recovered"]);
    }

    #[test]
    fn an_unterminated_oversized_line_resyncs_at_the_next_newline() {
        let mut d = CodexAppServerDecoder::new();
        let events = d.decode(&vec![b'x'; MAX_LINE_BYTES + 1]);
        assert!(
            matches!(events.as_slice(), [OutputEvent::Error(m)] if m.contains("lineOverflow")),
            "{events:?}"
        );
        let tail = format!("more junk\n{}\n", delta_line("recovered"));
        assert_eq!(text_of(&d.decode(tail.as_bytes())), vec!["recovered"]);
    }

    /// A line growing across several chunks is also cut before it exceeds the cap — the buffer has never existed beyond the cap.
    #[test]
    fn a_line_growing_across_chunks_is_capped_before_it_exceeds_the_limit() {
        let mut d = CodexAppServerDecoder::new();
        let mib = vec![b'x'; 1024 * 1024];
        // 4 times is exactly equal to the cap and does not exceed it yet — the fifth is what exceeds.
        for _ in 0..5 {
            let _ = d.decode(&mib);
            assert!(d.buffer.len() <= MAX_LINE_BYTES, "{}", d.buffer.len());
        }
        assert!(d.discarding, "상한을 넘겼는데 resync 로 들어가지 않았다");
        assert_eq!(d.buffer.capacity(), 0, "용량이 반납되지 않았다");
    }

    // ── Notifications we translate ────────────────────────────────────────────────

    #[test]
    fn agent_message_delta_becomes_a_text_delta_with_both_ids() {
        match decode_line(&delta_line("안녕")).as_slice() {
            [OutputEvent::TextDelta {
                text,
                turn_id,
                message_id,
            }] => {
                assert_eq!(text, "안녕");
                assert_eq!(turn_id.as_deref(), Some("u"));
                assert_eq!(message_id.as_deref(), Some("i"));
            }
            other => panic!("TextDelta 하나가 아니다: {other:?}"),
        }
    }

    #[test]
    fn token_usage_becomes_usage_from_the_last_breakdown() {
        let events = decode_notification(
            "thread/tokenUsage/updated",
            serde_json::json!({
                "threadId": "t-1", "turnId": "u-1",
                "tokenUsage": {
                    "last": {"inputTokens": 120, "cachedInputTokens": 100, "outputTokens": 7,
                             "reasoningOutputTokens": 3, "totalTokens": 127},
                    "total": {"inputTokens": 9999, "cachedInputTokens": 0, "outputTokens": 8888,
                              "reasoningOutputTokens": 0, "totalTokens": 18887},
                    "modelContextWindow": 272000
                }
            }),
        );
        match events.as_slice() {
            [OutputEvent::Usage {
                input_tokens,
                output_tokens,
                turn_id,
            }] => {
                assert_eq!(*input_tokens, 120);
                assert_eq!(*output_tokens, 7);
                assert_eq!(turn_id.as_deref(), Some("u-1"));
            }
            other => panic!("Usage 하나가 아니다: {other:?}"),
        }
    }

    /// One negative must not make that turn's whole usage disappear.
    #[test]
    fn negative_token_counts_clamp_instead_of_dropping_the_event() {
        let events = decode_notification(
            "thread/tokenUsage/updated",
            serde_json::json!({
                "threadId": "t", "turnId": "u",
                "tokenUsage": {
                    "last": {"inputTokens": -5, "cachedInputTokens": 0, "outputTokens": 3,
                             "reasoningOutputTokens": 0, "totalTokens": 0},
                    "total": {"inputTokens": 0, "cachedInputTokens": 0, "outputTokens": 0,
                              "reasoningOutputTokens": 0, "totalTokens": 0}
                }
            }),
        );
        match events.as_slice() {
            [OutputEvent::Usage {
                input_tokens,
                output_tokens,
                ..
            }] => {
                assert_eq!(*input_tokens, 0);
                assert_eq!(*output_tokens, 3);
            }
            other => panic!("Usage 하나가 아니다: {other:?}"),
        }
    }

    #[test]
    fn error_notification_carries_the_codex_label_and_the_retry_token() {
        let events = decode_notification(
            "error",
            serde_json::json!({
                "threadId": "t-1", "turnId": "u-1", "willRetry": true,
                "error": {"message": "upstream busy", "codexErrorInfo": "serverOverloaded"}
            }),
        );
        match events.as_slice() {
            [OutputEvent::Error(message)] => {
                assert_eq!(message, "upstream busy (serverOverloaded) [willRetry]");
            }
            other => panic!("Error 하나가 아니다: {other:?}"),
        }
    }

    /// ★Do not mix localized sentences into strings the core emits★ — this value is drawn as is on the screen without going through `src/i18n`.
    /// Pin it with a golden (a loose assertion lets prose through).
    #[test]
    fn our_own_tokens_are_protocol_field_names_not_prose() {
        let events = decode_notification(
            "error",
            serde_json::json!({"error": {"message": "x"}, "willRetry": true}),
        );
        match events.as_slice() {
            [OutputEvent::Error(message)] => assert_eq!(message, "x [willRetry]"),
            other => panic!("Error 하나가 아니다: {other:?}"),
        }
    }

    /// The cap-exceeded notice carries the same discipline — it must be a token, not a sentence.
    #[test]
    fn the_overflow_event_is_tokens_not_prose() {
        match overflow_event(7) {
            OutputEvent::Error(message) => {
                assert_eq!(
                    message,
                    format!("codex-decoder: lineOverflow seen=7B limit={MAX_LINE_BYTES}B")
                );
            }
            other => panic!("Error 가 아니다: {other:?}"),
        }
    }

    #[test]
    fn misalignment_details_and_steer_reach_the_error_event() {
        let events = decode_notification(
            "error",
            serde_json::json!({
                "error": {
                    "message": "blocked",
                    "misalignment": {
                        "errorType": "policyViolation",
                        "detailedExplanation": "그 요청은 수행할 수 없습니다",
                        "steer": {"message": "다르게 물어보세요"}
                    }
                }
            }),
        );
        match events.as_slice() {
            [OutputEvent::Error(message)] => {
                assert!(message.contains("policyViolation"), "{message}");
                assert!(
                    message.contains("그 요청은 수행할 수 없습니다"),
                    "{message}"
                );
                assert!(message.contains("steer: 다르게 물어보세요"), "{message}");
            }
            other => panic!("Error 하나가 아니다: {other:?}"),
        }
    }

    /// ★The total is bounded only if **all** six pieces are cut★ — if even one is missed, one field breaches the whole.
    /// And the place of cutting must not be wrong: cutting at the end loses the most actionable `steer` first.
    #[test]
    fn every_server_supplied_part_of_an_error_is_capped_and_the_steer_survives() {
        let huge = "z".repeat(50_000);
        let events = decode_notification(
            "error",
            serde_json::json!({
                "willRetry": true,
                "error": {
                    "message": huge,
                    "codexErrorInfo": huge,
                    "additionalDetails": huge,
                    "misalignment": {
                        "errorType": huge,
                        "detailedExplanation": huge,
                        "steer": {"message": "try asking differently"}
                    }
                }
            }),
        );
        match events.as_slice() {
            [OutputEvent::Error(message)] => {
                // Six pieces that get cut + a little of our tokens · separators.
                let bound = ERROR_PART_LIMIT * 6 + 200;
                assert!(
                    message.chars().count() < bound,
                    "총량이 유계가 아니다: {} (상한 {bound})",
                    message.chars().count()
                );
                assert!(
                    message.ends_with("steer: try asking differently"),
                    "steer 가 잘려 나갔다"
                );
            }
            other => panic!("Error 하나가 아니다: {other:?}"),
        }
    }

    #[test]
    fn command_execution_item_becomes_a_tool_call_named_after_the_item_kind() {
        let events = decode_notification(
            "item/started",
            serde_json::json!({
                "threadId": "t-1", "turnId": "u-1", "startedAtMs": 1764000000000i64,
                "item": {"type": "commandExecution", "id": "i-9", "command": "cargo test",
                         "cwd": "C:/w", "commandActions": [{"type": "unknown", "command": "cargo test"}],
                         "status": "inProgress"}
            }),
        );
        match events.as_slice() {
            [OutputEvent::ToolCall {
                name,
                args_json,
                id,
                turn_id,
                message_id,
                category,
            }] => {
                assert_eq!(name, "commandExecution");
                assert_eq!(*category, ToolCategory::Command);
                assert_eq!(id.as_deref(), Some("i-9"));
                assert_eq!(turn_id.as_deref(), Some("u-1"));
                assert!(message_id.is_none());
                let back: Value = serde_json::from_str(args_json).unwrap();
                assert_eq!(back["command"], "cargo test");
                assert_eq!(back["cwd"], "C:/w");
            }
            other => panic!("ToolCall 하나가 아니다: {other:?}"),
        }
    }

    #[test]
    fn mcp_tool_call_item_is_named_after_its_tool_field() {
        let events = decode_notification(
            "item/started",
            serde_json::json!({
                "threadId": "t-1", "turnId": "u-1", "startedAtMs": 1i64,
                "item": {"type": "mcpToolCall", "id": "i-3", "server": "fs", "tool": "read_file",
                         "arguments": {"path": "a.txt"}, "status": "inProgress"}
            }),
        );
        match events.as_slice() {
            [OutputEvent::ToolCall {
                name, args_json, ..
            }] => {
                assert_eq!(name, "read_file");
                let back: Value = serde_json::from_str(args_json).unwrap();
                assert_eq!(back["server"], "fs", "server 는 args_json 에 남아야 한다");
            }
            other => panic!("ToolCall 하나가 아니다: {other:?}"),
        }
    }

    #[test]
    fn web_search_and_file_change_items_are_tool_calls() {
        for (item, expected) in [
            (
                serde_json::json!({"type": "webSearch", "id": "i-1", "query": "rust ndjson"}),
                "webSearch",
            ),
            (
                serde_json::json!({"type": "fileChange", "id": "i-2", "status": "inProgress",
                                   "changes": [{"path": "a.rs", "diff": "-x\n+y", "kind": {"type": "update"}}]}),
                "fileChange",
            ),
        ] {
            let events = decode_notification(
                "item/started",
                serde_json::json!({"threadId": "t", "turnId": "u", "startedAtMs": 1i64, "item": item}),
            );
            match events.as_slice() {
                [OutputEvent::ToolCall { name, .. }] => assert_eq!(name, expected),
                other => panic!("{expected}: ToolCall 하나가 아니다: {other:?}"),
            }
        }
    }

    // ── ADR-0239: tool item → neutral kind ─────────────────────────────────────────────

    /// Also measures whether live (`item/started`) and the history door give **the same kind**, and returns that kind.
    fn category_through_both_doors(item: Value) -> ToolCategory {
        let (started, _, history) = through_each_door(item);
        let category_of = |events: &[OutputEvent]| match events {
            [OutputEvent::ToolCall { category, .. }] => *category,
            other => panic!("ToolCall 하나가 아니다: {other:?}"),
        };
        let live = category_of(&started);
        assert_eq!(category_of(&history), live, "이력과 라이브의 종류가 갈렸다");
        live
    }

    /// ★The kind of `commandExecution` is decided by the combination of `commandActions[].type`★ — it is a command if there is even one `unknown` or the array is
    /// absent or empty, and an unknown word · a missing `type` is also a command.
    // ADR-0239
    #[test]
    fn command_actions_decide_the_category_of_a_command_execution() {
        let action = |t: &str| serde_json::json!({"type": t, "command": "x"});
        let cases: Vec<(&str, Option<Value>, ToolCategory)> = vec![
            (
                "read",
                Some(serde_json::json!([action("read")])),
                ToolCategory::Read,
            ),
            (
                "read×2",
                Some(serde_json::json!([action("read"), action("read")])),
                ToolCategory::Read,
            ),
            (
                "listFiles",
                Some(serde_json::json!([action("listFiles")])),
                ToolCategory::List,
            ),
            (
                "listFiles×2",
                Some(serde_json::json!([
                    action("listFiles"),
                    action("listFiles")
                ])),
                ToolCategory::List,
            ),
            (
                "search",
                Some(serde_json::json!([action("search")])),
                ToolCategory::Search,
            ),
            (
                "search+read",
                Some(serde_json::json!([action("search"), action("read")])),
                ToolCategory::Search,
            ),
            (
                "listFiles+search",
                Some(serde_json::json!([action("listFiles"), action("search")])),
                ToolCategory::Search,
            ),
            (
                "read+listFiles",
                Some(serde_json::json!([action("read"), action("listFiles")])),
                ToolCategory::Read,
            ),
            (
                "unknown",
                Some(serde_json::json!([action("unknown")])),
                ToolCategory::Command,
            ),
            (
                "search+unknown",
                Some(serde_json::json!([action("search"), action("unknown")])),
                ToolCategory::Command,
            ),
            (
                "read+unknown",
                Some(serde_json::json!([action("read"), action("unknown")])),
                ToolCategory::Command,
            ),
            ("empty", Some(serde_json::json!([])), ToolCategory::Command),
            ("missing", None, ToolCategory::Command),
            ("null", Some(Value::Null), ToolCategory::Command),
            (
                "unknown word",
                Some(serde_json::json!([action("write")])),
                ToolCategory::Command,
            ),
            (
                "no type",
                Some(serde_json::json!([{"command": "x"}])),
                ToolCategory::Command,
            ),
        ];
        for (label, actions, want) in cases {
            let mut item =
                serde_json::json!({"type": "commandExecution", "id": "c1", "command": "x"});
            if let Some(actions) = actions {
                item["commandActions"] = actions;
            }
            assert_eq!(category_through_both_doors(item), want, "{label}");
        }
    }

    /// ★`commandActions: [{type:"unknown"}]` of the measured fixture (M7 — codex-cli 0.156.1) is a command★ — both the live line as is and
    /// that item put into the history door.
    // ADR-0239
    #[test]
    fn the_tool_end_fixture_command_with_an_unknown_action_is_a_command() {
        let lines: Vec<&str> = include_str!("fixtures/tool_end_m7.jsonl")
            .lines()
            .filter(|l| l.contains(r#""method":"item/started""#) && l.contains("commandExecution"))
            .collect();
        assert!(!lines.is_empty(), "fixture 에 도구 시작 줄이 없다");
        for line in lines {
            match decode_line(line).as_slice() {
                [OutputEvent::ToolCall { category, .. }] => {
                    assert_eq!(*category, ToolCategory::Command)
                }
                other => panic!("ToolCall 하나가 아니다: {other:?}"),
            }
            let v: Value = serde_json::from_str(line).unwrap();
            assert_eq!(
                category_through_both_doors(v["params"]["item"].clone()),
                ToolCategory::Command
            );
        }
    }

    /// A tool variant other than command is decided by the item type alone.
    // ADR-0239
    #[test]
    fn each_other_tool_item_type_has_its_own_category() {
        for (item, want) in [
            (
                serde_json::json!({"type": "fileChange", "id": "i-1", "changes": []}),
                ToolCategory::Edit,
            ),
            (
                serde_json::json!({"type": "webSearch", "id": "i-2", "query": "q"}),
                ToolCategory::Web,
            ),
            (
                serde_json::json!({"type": "mcpToolCall", "id": "i-3", "server": "fs", "tool": "read_file"}),
                ToolCategory::Mcp,
            ),
            (
                serde_json::json!({"type": "collabAgentToolCall", "id": "i-4", "tool": "spawn"}),
                ToolCategory::Agent,
            ),
            (
                serde_json::json!({"type": "dynamicToolCall", "id": "i-5", "tool": "custom"}),
                ToolCategory::Other,
            ),
        ] {
            let kind = item["type"].as_str().unwrap().to_owned();
            assert_eq!(category_through_both_doors(item), want, "{kind}");
        }
    }

    /// ★Even an item over the size cap has its kind decided from the intact item★ — `args_json` is an abbreviation without `commandActions`, but
    /// the kind is the value that saw the `search` inside it.
    // ADR-0239
    #[test]
    fn an_item_over_the_args_cap_still_gets_its_category_from_the_whole_item() {
        let item = serde_json::json!({
            "type": "commandExecution", "id": "c1",
            "command": "x".repeat(MAX_TOOL_ARGS_BYTES + 1),
            "commandActions": [{"type": "search", "command": "rg x", "query": "x"}]
        });
        let (started, _, history) = through_each_door(item);
        for events in [started, history] {
            match events.as_slice() {
                [OutputEvent::ToolCall {
                    args_json,
                    category,
                    ..
                }] => {
                    let back: Value = serde_json::from_str(args_json).unwrap();
                    assert!(
                        back.get("commandActions").is_none(),
                        "상한을 넘었는데 축약본이 아니다: {args_json:.200}"
                    );
                    assert_eq!(*category, ToolCategory::Search);
                }
                other => panic!("ToolCall 하나가 아니다: {other:?}"),
            }
        }
    }

    // ── ADR-0241: tool end result ─────────────────────────────────────────────────

    /// Feed one tool item to a new translator as `item/completed`, and return the events that came out and that translator (to look at the observation map).
    fn complete_tool(item: Value) -> (Vec<OutputEvent>, CodexAppServerDecoder) {
        let mut d = CodexAppServerDecoder::new();
        let events = d.decode(
            notification_line(
                method::ITEM_COMPLETED,
                serde_json::json!({"threadId": "t", "turnId": "u", "completedAtMs": 2i64, "item": item}),
            )
            .as_bytes(),
        );
        (events, d)
    }

    /// ★If the end is a failure · refusal, one result — the call is not emitted again★. The decision is the vendor `status` alone (neither the command's `exitCode`
    /// nor MCP's `error` is read), and the translator does not emit `Refused` (attributing our refusals belongs to the path).
    // ADR-0241
    #[test]
    fn a_failed_or_declined_tool_end_becomes_one_tool_result_on_the_completed_door() {
        for (item, want) in [
            (
                serde_json::json!({"type": "commandExecution", "id": "i-1", "command": "false",
                                   "commandActions": [], "status": "failed", "exitCode": 1}),
                ToolOutcome::Failed,
            ),
            (
                serde_json::json!({"type": "fileChange", "id": "i-1", "changes": [], "status": "declined"}),
                ToolOutcome::Declined,
            ),
            (
                serde_json::json!({"type": "mcpToolCall", "id": "i-1", "server": "fs", "tool": "read_file",
                                   "arguments": {}, "status": "failed", "error": {"message": "boom"}}),
                ToolOutcome::Failed,
            ),
            (
                serde_json::json!({"type": "dynamicToolCall", "id": "i-1", "tool": "t", "status": "failed",
                                   "success": false}),
                ToolOutcome::Failed,
            ),
            (
                serde_json::json!({"type": "collabAgentToolCall", "id": "i-1", "tool": "spawn",
                                   "status": "failed"}),
                ToolOutcome::Failed,
            ),
        ] {
            let (events, d) = complete_tool(item.clone());
            assert_eq!(events.len(), 1, "{item}: 결과 하나가 아니다: {events:?}");
            match &events[0] {
                OutputEvent::ToolResult { id, outcome } => {
                    assert_eq!(id, "i-1", "{item}");
                    assert_eq!(*outcome, want, "{item}");
                }
                other => panic!("{item}: ToolResult 가 아니다: {other:?}"),
            }
            assert!(
                d.untranslated_observations().is_empty(),
                "{item}: 옮긴 끝이 「번역 안 함」 계수로 새었다: {:?}",
                d.untranslated_observations()
            );
        }
    }

    /// ★Normal completion · still running · interrupted · no status field are 0 events★ — even a `completed` carrying a non-zero `exitCode` (the decision is
    /// `status` alone). Today's routine count (`item:`) stays as it is.
    // ADR-0241
    #[test]
    fn other_tool_ends_emit_nothing_and_keep_the_routine_count() {
        for item in [
            serde_json::json!({"type": "commandExecution", "id": "i-1", "command": "grep x",
                               "commandActions": [], "status": "completed", "exitCode": 1}),
            serde_json::json!({"type": "commandExecution", "id": "i-1", "command": "x",
                               "commandActions": [], "status": "inProgress"}),
            serde_json::json!({"type": "collabAgentToolCall", "id": "i-1", "tool": "spawn",
                               "status": "interrupted"}),
            serde_json::json!({"type": "webSearch", "id": "i-1", "query": "q"}),
        ] {
            let (events, d) = complete_tool(item.clone());
            assert!(events.is_empty(), "{item}: {events:?}");
            let kind = item["type"].as_str().unwrap();
            assert_eq!(
                d.untranslated_observations(),
                &BTreeMap::from([(format!("item:item/completed#{kind}"), 1)]),
                "{item}"
            );
        }
    }

    /// ★An unknown end status is 0 events + one drift entry★ — a peer string does not become a key (two different unknown values pile up in the same
    /// entry). A non-string value goes to the same entry.
    // ADR-0241
    #[test]
    fn an_unknown_tool_status_emits_nothing_and_is_counted_as_drift_under_one_key() {
        let mut d = CodexAppServerDecoder::new();
        for status in [
            serde_json::json!("quantumCollapsed"),
            serde_json::json!("singularity"),
            Value::Null,
            serde_json::json!(7),
        ] {
            let events = d.decode(
                notification_line(
                    method::ITEM_COMPLETED,
                    serde_json::json!({"threadId": "t", "turnId": "u", "item":
                        {"type": "commandExecution", "id": "i-1", "command": "x", "status": status}}),
                )
                .as_bytes(),
            );
            assert!(events.is_empty(), "{status}: {events:?}");
        }
        let seen = d.untranslated_observations();
        assert_eq!(
            seen.get("tool-status:commandExecution#<모르는 값>"),
            Some(&4),
            "{seen:?}"
        );
        assert!(
            !seen.keys().any(|k| k.contains("quantumCollapsed")),
            "상대 문자열이 키가 됐다: {seen:?}"
        );
        assert!(
            seen.get("item:item/completed#commandExecution").is_none(),
            "드리프트가 일상 계수로도 한 번 더 잡혔다: {seen:?}"
        );
    }

    /// A known word the schema did not list for that variant (`declined` of `mcpToolCall`) is carried over as meant but counted as drift.
    // ADR-0241
    #[test]
    fn a_known_status_outside_the_variant_vocabulary_is_translated_and_counted_as_drift() {
        let (events, d) = complete_tool(serde_json::json!({
            "type": "mcpToolCall", "id": "i-1", "server": "fs", "tool": "read_file", "status": "declined"
        }));
        assert!(
            matches!(
                events.as_slice(),
                [OutputEvent::ToolResult {
                    outcome: ToolOutcome::Declined,
                    ..
                }]
            ),
            "{events:?}"
        );
        assert_eq!(
            d.untranslated_observations(),
            &BTreeMap::from([("tool-status:mcpToolCall#declined".to_string(), 1)]),
            "드리프트 하나만 — 일상 계수가 겹쳐 붙으면 안 된다"
        );
    }

    /// ★If the end status was already recorded as anomalous, the routine count (`item:`) is not added★ — one observation is counted at one grade only. All of: unknown value ·
    /// non-string value · known word outside the schema vocabulary (including the no-outcome side · even a status carried on `webSearch`).
    // ADR-0241
    #[test]
    fn a_tool_end_already_counted_as_drift_is_not_counted_again_as_routine() {
        for (item, want_key) in [
            (
                serde_json::json!({"type": "commandExecution", "id": "i-1", "command": "x",
                                   "status": "quantumCollapsed"}),
                "tool-status:commandExecution#<모르는 값>",
            ),
            (
                serde_json::json!({"type": "commandExecution", "id": "i-1", "command": "x", "status": 7}),
                "tool-status:commandExecution#<모르는 값>",
            ),
            (
                serde_json::json!({"type": "commandExecution", "id": "i-1", "command": "x",
                                   "status": "interrupted"}),
                "tool-status:commandExecution#interrupted",
            ),
            (
                serde_json::json!({"type": "webSearch", "id": "i-1", "query": "q", "status": "completed"}),
                "tool-status:webSearch#completed",
            ),
        ] {
            let (events, d) = complete_tool(item.clone());
            assert!(events.is_empty(), "{item}: {events:?}");
            assert_eq!(
                d.untranslated_observations(),
                &BTreeMap::from([(want_key.to_string(), 1)]),
                "{item}"
            );
        }
    }

    /// ★If the field is missing in a variant that should have a status field, it is one defect entry — not passed over quietly★: if upstream renames the field, failure ·
    /// refusal badges all disappear, and if the only signal left then is the debug routine count, nothing is left in the shipping configuration. The routine count is
    /// not added. The history door is the same (the call row still stands). Only `webSearch`, which has no field, is routine (`other_tool_ends…` above).
    // ADR-0241
    #[test]
    fn a_status_less_tool_end_on_a_status_bearing_variant_is_a_defect() {
        for item in [
            serde_json::json!({"type": "commandExecution", "id": "i-1", "command": "x", "commandActions": []}),
            serde_json::json!({"type": "fileChange", "id": "i-1", "changes": []}),
            serde_json::json!({"type": "mcpToolCall", "id": "i-1", "server": "fs", "tool": "read_file"}),
            serde_json::json!({"type": "dynamicToolCall", "id": "i-1", "tool": "t"}),
            serde_json::json!({"type": "collabAgentToolCall", "id": "i-1", "tool": "spawn"}),
        ] {
            let kind = item["type"].as_str().unwrap().to_string();
            let (events, d) = complete_tool(item.clone());
            assert!(events.is_empty(), "{kind}: {events:?}");
            assert_eq!(
                d.untranslated_observations(),
                &BTreeMap::from([(format!("tool-status:{kind}#<없음>"), 1)]),
                "{kind}: 결함 한 칸만 — 일상 계수가 붙거나 조용히 넘어가면 안 된다"
            );

            let mut h = CodexAppServerDecoder::new();
            let events = h.history_item("tu", &item);
            assert!(
                matches!(events.as_slice(), [OutputEvent::ToolCall { .. }]),
                "{kind}: 이력 문은 호출만 낸다: {events:?}"
            );
            assert_eq!(
                h.untranslated_observations()
                    .get(&format!("tool-status:{kind}#<없음>")),
                Some(&1),
                "{kind}"
            );
        }
    }

    /// ★Empty schema vocabulary = 「status 칸이 없는 변형」 (a variant with no status field)★ — the decision that reads a missing status as routine leans on this empty list. In the tool-variant
    /// table the empty list must be `webSearch` alone: if a new variant is added to the table without writing its vocabulary, that variant's missing status quietly
    /// becomes routine.
    // ADR-0241
    #[test]
    fn every_tool_variant_but_web_search_has_a_status_vocabulary() {
        for kind in TOOL_ITEM_TYPES {
            if *kind == "webSearch" {
                assert!(tool_status::of(kind).is_empty(), "{kind}");
            } else {
                assert!(
                    !tool_status::of(kind).is_empty(),
                    "{kind}: 도구 변형인데 스키마 어휘가 비었다"
                );
            }
        }
    }

    /// ★If there is no call to point at, no result is emitted and it is counted as a defect★ — an id that is absent or over the cap (the call also
    /// loses its id through the same filtering).
    // ADR-0241
    #[test]
    fn a_failed_tool_end_without_a_usable_id_emits_nothing_and_is_a_defect() {
        for id in [None, Some("i".repeat(MAX_ID_BYTES + 1))] {
            let mut item =
                serde_json::json!({"type": "commandExecution", "command": "x", "status": "failed"});
            if let Some(id) = &id {
                item["id"] = Value::String(id.clone());
            }
            let (events, d) = complete_tool(item);
            assert!(events.is_empty(), "{events:?}");
            let seen = d.untranslated_observations();
            assert_eq!(
                seen.get("item-id:item/completed#commandExecution"),
                Some(&1),
                "{seen:?}"
            );
            assert!(
                seen.get("item:item/completed#commandExecution").is_none(),
                "결함이 「아는 변형, 번역 안 함」 계수로 새어 들어갔다: {seen:?}"
            );
        }
        let long = "i".repeat(MAX_ID_BYTES);
        let (events, _) = complete_tool(serde_json::json!({
            "type": "commandExecution", "id": long, "command": "x", "status": "failed"
        }));
        assert!(
            matches!(events.as_slice(), [OutputEvent::ToolResult { id, .. }] if *id == long),
            "상한 안의 id 가 걸러졌다: {events:?}"
        );
    }

    /// ★The history door emits `[ToolCall, ToolResult]` from one finished item in this order★ — the accumulator attaches the result to the preceding call row, so
    /// if the order is reversed the result is dropped. They point at the same id, and for a normal completion there is only the call.
    // ADR-0241
    #[test]
    fn the_history_door_emits_the_call_then_its_result() {
        for (status, want) in [
            ("failed", Some(ToolOutcome::Failed)),
            ("declined", Some(ToolOutcome::Declined)),
            ("completed", None),
            ("inProgress", None),
        ] {
            let item = serde_json::json!({"type": "commandExecution", "id": "c1", "command": "x",
                                          "commandActions": [], "status": status});
            let (started, _, history) = through_each_door(item);
            assert!(
                matches!(started.as_slice(), [OutputEvent::ToolCall { .. }]),
                "{status}: 시작 문은 끝 상태를 안 읽는다: {started:?}"
            );
            match (history.as_slice(), want) {
                (
                    [OutputEvent::ToolCall { id: call_id, .. }, OutputEvent::ToolResult { id, outcome }],
                    Some(want),
                ) => {
                    assert_eq!(call_id.as_deref(), Some("c1"), "{status}");
                    assert_eq!(id, "c1", "{status}");
                    assert_eq!(*outcome, want, "{status}");
                }
                ([OutputEvent::ToolCall { .. }], None) => {}
                (other, _) => panic!("{status}: {other:?}"),
            }
        }
    }

    /// If the history door also cannot use the id, it emits only the call and counts the result as a defect — the call row stands even without an id (as today).
    // ADR-0241
    #[test]
    fn the_history_door_without_a_usable_id_emits_only_the_call() {
        let mut d = CodexAppServerDecoder::new();
        let events = d.history_item(
            "tu",
            &serde_json::json!({"type": "fileChange", "changes": [], "status": "declined"}),
        );
        assert!(
            matches!(events.as_slice(), [OutputEvent::ToolCall { id: None, .. }]),
            "{events:?}"
        );
        assert_eq!(
            d.untranslated_observations()
                .get("item-id:thread/items/list#fileChange"),
            Some(&1)
        );
    }

    /// Pick only notification lines and feed them to one translator in order — what the path hands to the translator is notifications only (responses · server requests carry an `id`).
    fn decode_fixture_notifications(fixture: &str) -> Vec<OutputEvent> {
        let mut d = CodexAppServerDecoder::new();
        fixture
            .lines()
            .filter(|l| {
                let v: Value = serde_json::from_str(l).expect("fixture 줄이 JSON 이 아니다");
                v.get("method").is_some() && v.get("id").is_none()
            })
            .flat_map(|l| d.decode(format!("{l}\n").as_bytes()))
            .collect()
    }

    /// ★Measured (codex-cli 0.156.1 · fixture `tool_fail_u2` — `exit 7`)★: a non-zero exit closes as `status: failed`, and the translator
    /// emits exactly one `ToolResult{Failed}` of the same id after that command's call.
    // ADR-0241
    #[test]
    fn the_nonzero_exit_fixture_yields_the_call_then_exactly_one_failed_result() {
        let events = decode_fixture_notifications(include_str!("fixtures/tool_fail_u2.jsonl"));
        let tools: Vec<&OutputEvent> = events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    OutputEvent::ToolCall { .. } | OutputEvent::ToolResult { .. }
                )
            })
            .collect();
        match tools.as_slice() {
            [OutputEvent::ToolCall {
                id: Some(call_id), ..
            }, OutputEvent::ToolResult { id, outcome }] => {
                assert_eq!(call_id, "exec-c8b4b685-9f78-4680-99a0-3d560fab9fd1");
                assert_eq!(id, call_id);
                assert_eq!(*outcome, ToolOutcome::Failed);
            }
            other => panic!("호출 하나 뒤 실패 결과 하나가 아니다: {other:?}"),
        }
    }

    /// The three existing measured fixtures all have tool ends `completed`, so there are no result events at all — the event sequence is as today.
    // ADR-0241
    #[test]
    fn the_older_fixtures_whose_tools_all_completed_yield_no_tool_result() {
        for (name, fixture) in [
            ("steer_m6", include_str!("fixtures/steer_m6.jsonl")),
            ("tool_end_m7", include_str!("fixtures/tool_end_m7.jsonl")),
            (
                "empty_turn_m9",
                include_str!("fixtures/empty_turn_m9.jsonl"),
            ),
        ] {
            let events = decode_fixture_notifications(fixture);
            assert!(
                !events
                    .iter()
                    .any(|e| matches!(e, OutputEvent::ToolResult { .. })),
                "{name}: {events:?}"
            );
        }
    }

    // ── Echoed user message (`userMessage` of `item/*`) ────────────────────────

    /// ★Do not drop it — this is the only path that puts the user's utterance on the screen★: this backend does not declare an input-time synthetic
    /// echo (ADR-0193), so if this item is dropped, what the user typed never appears on the screen and
    /// is not restored after reattach either.
    #[test]
    fn a_rebounded_user_message_is_marked_as_ours_and_carries_its_text() {
        let events = decode_notification(
            "item/started",
            serde_json::json!({"threadId": "t", "turnId": "u", "startedAtMs": 1i64,
                               "item": {"type": "userMessage", "id": "i-7",
                                        "content": [{"type": "text", "text": "안녕 codex"}]}}),
        );
        match events.as_slice() {
            [OutputEvent::Structured { kind, json }] => {
                assert_eq!(kind, "user", "소비자가 읽는 표식은 이 한 칸이다");
                let v: Value = serde_json::from_str(json).unwrap();
                assert_eq!(v["type"], "text", "소비자가 이 값으로 dedup 키를 집는다");
                assert_eq!(v["text"], "안녕 codex");
                assert_eq!(v["uuid"], "i-7");
            }
            other => panic!("Structured{{kind:\"user\"}} 하나가 아니다: {other:?}"),
        }
    }

    /// Carry one echoed user message item identically in two notifications.
    fn user_item_line(method: &str, id: &str, text: &str) -> String {
        notification_line(
            method,
            serde_json::json!({"threadId": "t", "turnId": "u", "startedAtMs": 1i64,
                               "completedAtMs": 2i64,
                               "item": {"type": "userMessage", "id": id,
                                        "content": [{"type": "text", "text": text}]}}),
        )
    }

    /// ★**Whichever comes first** raises it and the second drops out — in either order★. If we pin it to one notification, what is lost is
    /// 「유저 자신이 친 말」 (what the user typed themselves), and both directions have failure cases: whether `item/started` comes for this kind is
    /// unmeasured, and `item/completed` is measured not to come at all for an interrupted item (TRD §2 L10).
    /// ★The point is measuring with the same instance★ — the comparison memory lives inside that instance. If two lines are fed to a new instance,
    /// this item measures nothing.
    #[test]
    fn the_rebounded_user_message_lands_once_whichever_notification_arrives_first() {
        for order in [
            ["item/started", "item/completed"],
            ["item/completed", "item/started"],
        ] {
            let mut d = CodexAppServerDecoder::new();
            let mut events = Vec::new();
            for method in order {
                events.extend(d.decode(user_item_line(method, "i-7", "hi").as_bytes()));
            }
            match events.as_slice() {
                [OutputEvent::Structured { kind, json }] => {
                    assert_eq!(kind, "user", "{order:?}");
                    let v: Value = serde_json::from_str(json).unwrap();
                    assert_eq!(v["text"], "hi", "{order:?}");
                    assert_eq!(v["uuid"], "i-7", "{order:?}");
                }
                other => panic!("{order:?}: 말풍선이 하나가 아니다: {other:?}"),
            }
        }
    }

    /// ★A different item id means different words★ — if the comparison is flattened into 「유저 메시지를 이미 한 번 올렸나」 (has a user message already been raised once), from the
    /// second question of a conversation on, nothing appears on the screen.
    #[test]
    fn two_different_user_items_both_land() {
        let mut d = CodexAppServerDecoder::new();
        let mut events = d.decode(user_item_line("item/started", "i-1", "첫 질문").as_bytes());
        events.extend(d.decode(user_item_line("item/completed", "i-2", "둘째 질문").as_bytes()));
        assert_eq!(events.len(), 2, "{events:?}");
    }

    /// ★The comparison memory does not grow with session length★ — even if the peer makes items endlessly, this field stands at the cap.
    /// The price = if an id pushed out beyond the cap comes again late, that bubble stays twice (the frontend `uuid`
    /// dedup takes that case).
    #[test]
    fn the_user_item_memory_is_bounded() {
        let mut d = CodexAppServerDecoder::new();
        for i in 0..(MAX_EMITTED_USER_ITEMS * 3) {
            let _ = d.decode(user_item_line("item/started", &format!("i-{i}"), "x").as_bytes());
        }
        assert_eq!(d.emitted_user_items.len(), MAX_EMITTED_USER_ITEMS);
    }

    /// ★An id over the cap is not kept in the memory★ — the `uuid` going out on the wire is filtered at the same cap, and if only this field held the raw form,
    /// under a cap that counts only entries (64), a string as large as the line cap would stay for the whole session.
    /// The price is that bubble goes up in two copies (there is no key left to compare) — this item measures both together.
    #[test]
    fn an_oversize_user_item_id_is_not_retained_in_the_dedup_memory() {
        let long_id = "i".repeat(MAX_ID_BYTES + 1);
        let mut d = CodexAppServerDecoder::new();
        let started = d.decode(user_item_line("item/started", &long_id, "hi").as_bytes());
        let completed = d.decode(user_item_line("item/completed", &long_id, "hi").as_bytes());

        assert!(
            d.emitted_user_items.is_empty(),
            "상한 밖 id 가 기억에 남았다: {:?}",
            d.emitted_user_items
        );
        assert_eq!(started.len(), 1, "{started:?}");
        assert_eq!(completed.len(), 1, "{completed:?}");
    }

    /// ★An id **within** the cap still blocks the second notification as it is★ — measures that the filter of the item above did not switch off dedup itself
    /// (the comparison is an exact match either way).
    #[test]
    fn a_bounded_user_item_id_still_suppresses_the_second_notification() {
        let id = "i".repeat(MAX_ID_BYTES);
        let mut d = CodexAppServerDecoder::new();
        let started = d.decode(user_item_line("item/started", &id, "hi").as_bytes());
        let completed = d.decode(user_item_line("item/completed", &id, "hi").as_bytes());

        assert_eq!(started.len(), 1, "{started:?}");
        assert!(
            completed.is_empty(),
            "둘째 알림이 또 올라갔다: {completed:?}"
        );
        assert_eq!(d.emitted_user_items.len(), 1, "{:?}", d.emitted_user_items);
    }

    /// ★If there is no id there is no key to compare★ — so only then do we hang it on the `item/started` side alone. If both are raised,
    /// the same words remain twice, and the frontend dedup key is also that id, so it does not catch there either. A field the schema marks required being empty
    /// is counted as a defect.
    #[test]
    fn a_user_item_without_an_id_lands_only_at_item_start_and_is_counted_as_malformed() {
        let item = serde_json::json!({"type": "userMessage",
                                      "content": [{"type": "text", "text": "hi"}]});
        let params = serde_json::json!({"threadId": "t", "turnId": "u", "item": item});
        let mut d = CodexAppServerDecoder::new();
        let started = d.decode(notification_line("item/started", params.clone()).as_bytes());
        let completed = d.decode(notification_line("item/completed", params).as_bytes());
        assert_eq!(started.len(), 1, "{started:?}");
        assert!(completed.is_empty(), "{completed:?}");
        assert_eq!(
            d.untranslated_observations()
                .get("item-id:item/started#userMessage"),
            Some(&1)
        );
    }

    /// If there are several text pieces they are joined with lines — written together the words run together. And non-text input
    /// (images · mention etc.) has no vocabulary to carry it and drops out (a knowingly accepted loss).
    #[test]
    fn user_message_text_parts_are_joined_and_non_text_inputs_are_dropped() {
        let events = decode_notification(
            "item/started",
            serde_json::json!({"threadId": "t", "turnId": "u", "startedAtMs": 1i64,
                               "item": {"type": "userMessage", "id": "i-8",
                                        "content": [{"type": "text", "text": "첫 줄"},
                                                    {"type": "localImage", "path": "a.png"},
                                                    {"type": "text", "text": "둘째 줄"}]}}),
        );
        match events.as_slice() {
            [OutputEvent::Structured { json, .. }] => {
                let v: Value = serde_json::from_str(json).unwrap();
                assert_eq!(v["text"], "첫 줄\n둘째 줄");
            }
            other => panic!("Structured 하나가 아니다: {other:?}"),
        }
    }

    /// A user message with no text at all **does not make an empty bubble** — instead it is left as a count, so the fact that we have no
    /// vocabulary for that input kind is not silent.
    /// ★An echoed user message is a field whose length the peer decides★ — without a cap, one big paste alone exceeds the replay
    /// ring's single-event cap, and to fit that one entry the ring **evicts all the rest of that agent's
    /// rewind**. ★Even so it is not masked — do not revive masking★: this is not a diagnostic string but
    /// exactly what the user typed (source of truth for that decision = `user_message_event` doc).
    #[test]
    fn a_huge_pasted_user_message_is_capped_but_not_masked() {
        let secret = "sk-proj-ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let text = format!("{} {secret}", "가".repeat(MAX_TRANSCRIPT_CHARS * 2));
        let events = decode_notification(
            "item/started",
            serde_json::json!({"threadId": "t", "turnId": "u-1",
                "item": {"type": "userMessage", "id": "i-1",
                         "content": [{"type": "text", "text": text}]}}),
        );
        let json = match events.as_slice() {
            [OutputEvent::Structured { kind, json }] if kind == "user" => json.clone(),
            other => panic!("되울린 유저 메시지가 아니다: {other:?}"),
        };
        let carried = serde_json::from_str::<Value>(&json).expect("json")["text"]
            .as_str()
            .expect("text")
            .to_string();
        // The cap + one ellipsis character. What was sent over was twice that.
        assert_eq!(
            carried.chars().count(),
            MAX_TRANSCRIPT_CHARS + 1,
            "상한 없이 실렸다"
        );

        // A short credential is within the cap and must remain as it is — if masking is turned on, this assertion breaks.
        let short = decode_notification(
            "item/started",
            serde_json::json!({"threadId": "t", "turnId": "u-2",
                "item": {"type": "userMessage", "id": "i-2",
                         "content": [{"type": "text", "text": format!("내 키는 {secret} 야")}]}}),
        );
        match short.as_slice() {
            [OutputEvent::Structured { json, .. }] => assert!(
                json.contains(secret),
                "전사 본문이 마스킹됐다 — 그 기록이 거짓이 된다: {json}"
            ),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_user_message_without_text_makes_no_empty_bubble_but_is_counted() {
        let line = notification_line(
            "item/started",
            serde_json::json!({"threadId": "t", "turnId": "u", "startedAtMs": 1i64,
                               "item": {"type": "userMessage", "id": "i-9",
                                        "content": [{"type": "localImage", "path": "a.png"}]}}),
        );
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(line.as_bytes()).is_empty());
        assert_eq!(
            d.untranslated_observations()
                .get("item:item/started#userMessage"),
            Some(&1)
        );
    }

    // ── Consumption echo (`clientId`) — ADR-0231 ────────────────────────────────────

    fn client_item(client_id: Option<&str>) -> Value {
        let mut item = serde_json::json!({"type": "userMessage", "id": "codex-item-1",
                                          "content": [{"type": "text", "text": "hi"}]});
        if let Some(c) = client_id {
            item["clientId"] = Value::String(c.to_string());
        }
        item
    }

    fn bubble_uuid(event: &OutputEvent) -> Option<String> {
        match event {
            OutputEvent::Structured { kind, json } if kind == "user" => {
                let v: Value = serde_json::from_str(json).unwrap();
                v["uuid"].as_str().map(str::to_string)
            }
            other => panic!("유저 말풍선이 아니다: {other:?}"),
        }
    }

    /// ★The receipt is **right before** the bubble★ — the accumulator sets the list entry's bubble at the `Delivered` spot, and the echo that follows
    /// folds into one since it has the same uuid. If the uuid stays as the codex item id, two bubbles stand.
    #[test]
    fn an_echo_carrying_our_client_id_is_delivered_right_before_a_bubble_keyed_by_it() {
        for method_name in [method::ITEM_STARTED, method::ITEM_COMPLETED] {
            let events = decode_notification(
                method_name,
                serde_json::json!({"threadId": "t", "turnId": "u", "item": client_item(Some("ours-1"))}),
            );
            match events.as_slice() {
                [OutputEvent::QueuedInput(QueuedInputEvent::Delivered { id }), bubble] => {
                    assert_eq!(id, "ours-1", "{method_name}");
                    assert_eq!(
                        bubble_uuid(bubble).as_deref(),
                        Some("ours-1"),
                        "{method_name}"
                    );
                }
                other => panic!("{method_name}: 받음 + 말풍선이 아니다: {other:?}"),
            }
        }
    }

    /// If there is no `clientId` it is as today — one bubble with the codex item id, without a receipt.
    #[test]
    fn an_echo_without_a_client_id_is_today_s_bubble_and_no_delivery() {
        let events = decode_notification(
            method::ITEM_STARTED,
            serde_json::json!({"threadId": "t", "turnId": "u", "item": client_item(None)}),
        );
        match events.as_slice() {
            [bubble] => assert_eq!(bubble_uuid(bubble).as_deref(), Some("codex-item-1")),
            other => panic!("말풍선 하나가 아니다: {other:?}"),
        }
    }

    /// Two notifications of one item are **one set** of receipt · bubble — the second is caught by the item id comparison and dropped entirely.
    #[test]
    fn the_second_notification_of_an_echo_repeats_neither_delivery_nor_bubble() {
        let mut d = CodexAppServerDecoder::new();
        let params = serde_json::json!({"threadId": "t", "turnId": "u", "item": client_item(Some("ours-1"))});
        let first = d.decode(notification_line(method::ITEM_STARTED, params.clone()).as_bytes());
        let second = d.decode(notification_line(method::ITEM_COMPLETED, params).as_bytes());
        assert_eq!(first.len(), 2, "{first:?}");
        assert!(second.is_empty(), "{second:?}");
    }

    /// ★The history door does not emit a receipt★ — past records are not receipt events, and the core door that carries history chunks does not accept list
    /// events (an assertion fires in a debug build). The bubble uuid follows the same rule as live.
    #[test]
    fn the_history_door_keys_the_bubble_by_client_id_but_delivers_nothing() {
        let (_, _, history) = through_each_door(client_item(Some("ours-1")));
        match history.as_slice() {
            [bubble] => assert_eq!(bubble_uuid(bubble).as_deref(), Some("ours-1")),
            other => panic!("말풍선 하나가 아니다: {other:?}"),
        }
    }

    /// A `clientId` over the cap is the same as none — it is a comparison token, so it is filtered, not truncated.
    #[test]
    fn an_oversize_client_id_falls_back_to_today_s_bubble() {
        let long = "c".repeat(MAX_ID_BYTES + 1);
        let events = decode_notification(
            method::ITEM_STARTED,
            serde_json::json!({"threadId": "t", "turnId": "u", "item": client_item(Some(&long))}),
        );
        match events.as_slice() {
            [bubble] => assert_eq!(bubble_uuid(bubble).as_deref(), Some("codex-item-1")),
            other => panic!("말풍선 하나가 아니다: {other:?}"),
        }
    }

    // ── Turn boundary (`turn/completed`) ────────────────────────────────────────

    /// ★Only this one event turns off the waiting indicator★ — if no boundary is emitted at any outcome, that conversation's
    /// screen spins forever. So **even for unknown values** it is once per line.
    #[test]
    fn every_turn_status_ends_the_turn_exactly_once() {
        for (status, expected) in [
            ("completed", TurnOutcome::Completed),
            ("interrupted", TurnOutcome::Interrupted),
            ("failed", TurnOutcome::Failed { detail: None }),
            ("inProgress", TurnOutcome::Unknown),
            ("quantumCollapsed", TurnOutcome::Unknown),
        ] {
            let events = decode_notification(
                "turn/completed",
                serde_json::json!({
                    "threadId": "t-1",
                    "turn": {"id": "u-1", "items": [], "status": status}
                }),
            );
            match events.as_slice() {
                [OutputEvent::TurnEnd { turn_id, outcome }] => {
                    assert_eq!(turn_id.as_deref(), Some("u-1"), "status={status}");
                    assert_eq!(outcome, &expected, "status={status}");
                }
                other => panic!("status={status}: TurnEnd 하나가 아니다: {other:?}"),
            }
        }
    }

    /// The outcome and reason come as **one event** — if split, consumers get one more ordering contract.
    #[test]
    fn a_failed_turn_carries_its_reason_inside_the_outcome() {
        let events = decode_notification(
            "turn/completed",
            serde_json::json!({
                "threadId": "t-1",
                "turn": {"id": "u-1", "items": [], "status": "failed",
                         "error": {"message": "model refused",
                                   "codexErrorInfo": "serverOverloaded"}}
            }),
        );
        match events.as_slice() {
            [OutputEvent::TurnEnd {
                turn_id,
                outcome: TurnOutcome::Failed { detail },
            }] => {
                assert_eq!(turn_id.as_deref(), Some("u-1"));
                assert_eq!(detail.as_deref(), Some("model refused (serverOverloaded)"));
            }
            other => panic!("Failed 결말을 실은 TurnEnd 하나가 아니다: {other:?}"),
        }
    }

    /// `Turn.error` is optional — even without a reason, the fact of failure must remain.
    #[test]
    fn a_failed_turn_without_an_error_payload_still_ends_the_turn_as_failed() {
        let events = decode_notification(
            "turn/completed",
            serde_json::json!({"turn": {"id": "u-1", "items": [], "status": "failed"}}),
        );
        assert_eq!(
            events.len(),
            1,
            "사유가 없다고 이벤트 수가 달라지면 안 된다: {events:?}"
        );
        assert!(matches!(
            events.as_slice(),
            [OutputEvent::TurnEnd {
                outcome: TurnOutcome::Failed { detail: None },
                ..
            }]
        ));
    }

    /// ★Do not revive it — this is the very misclassification that actually blew up on the claude side and was reverted★: if the failure decision is
    /// written as 「완료가 아닌 것 전부」 (everything that is not completed) (the complement), a turn the user normally interrupted with Esc is stamped as a failure.
    /// In this repository an interrupt is a first-class normal path.
    #[test]
    fn an_interrupted_turn_is_not_a_failure_allowlist_not_denylist() {
        let events = decode_notification(
            "turn/completed",
            serde_json::json!({
                "threadId": "t-1",
                "turn": {"id": "u-1", "items": [], "status": "interrupted", "durationMs": 4437}
            }),
        );
        assert!(
            !events.iter().any(|e| matches!(
                e,
                OutputEvent::Error(_)
                    | OutputEvent::TurnEnd {
                        outcome: TurnOutcome::Failed { .. },
                        ..
                    }
            )),
            "중단은 실패가 아니다: {events:?}"
        );
        assert!(matches!(
            events.as_slice(),
            [OutputEvent::TurnEnd {
                outcome: TurnOutcome::Interrupted,
                ..
            }]
        ));
    }

    /// Even if upstream adds a fifth value, the boundary must stand — if dropped, that screen spins forever. And that value is
    /// counted as **drift** (since it is a name not in the declaration).
    ///
    /// ★But that count **does not use the peer string as a key**★ — this map is a bounded field that never empties, so
    /// if an arbitrary status were used as a key the budget would fill by that alone and the drift that should be seen would leave only a count, without a name.
    #[test]
    fn an_unknown_turn_status_still_ends_the_turn_and_is_counted_under_one_bounded_key() {
        let mut d = CodexAppServerDecoder::new();
        let events = d.decode(
            notification_line(
                "turn/completed",
                serde_json::json!({"turn": {"id": "u-1", "items": [], "status": "quantumCollapsed"}}),
            )
            .as_bytes(),
        );
        assert!(matches!(
            events.as_slice(),
            [OutputEvent::TurnEnd {
                outcome: TurnOutcome::Unknown,
                ..
            }]
        ));
        // Two different unknown values pile up in the **same entry** — the budget is not burned.
        let _ = d.decode(
            notification_line(
                "turn/completed",
                serde_json::json!({"turn": {"id": "u-2", "items": [], "status": "singularity"}}),
            )
            .as_bytes(),
        );
        let seen = d.untranslated_observations();
        assert_eq!(seen.get("turn-status:<모르는 값>"), Some(&2), "{seen:?}");
        assert!(
            !seen.keys().any(|k| k.contains("quantumCollapsed")),
            "상대 문자열이 키가 됐다: {seen:?}"
        );

        // A value that **is** in the declaration keeps its name as is — that set is bounded, so the budget is not burned.
        let mut d = CodexAppServerDecoder::new();
        let _ = d.decode(
            notification_line(
                "turn/completed",
                serde_json::json!({"turn": {"id": "u-3", "items": [], "status": "inProgress"}}),
            )
            .as_bytes(),
        );
        assert_eq!(
            d.untranslated_observations().get("turn-status:inProgress"),
            Some(&1)
        );
    }

    /// ★Do not guess `inProgress` and fold it into one of the known outcomes★ — we do not know what the combination of that value **arriving on a completion notification**
    /// means (the `[미확인]` field of TRD §6-1). If folded, the screen draws a false outcome.
    #[test]
    fn an_in_progress_status_on_a_completion_is_not_guessed() {
        let line = notification_line(
            "turn/completed",
            serde_json::json!({"turn": {"id": "u-1", "items": [], "status": "inProgress"}}),
        );
        let mut d = CodexAppServerDecoder::new();
        assert!(matches!(
            d.decode(line.as_bytes()).as_slice(),
            [OutputEvent::TurnEnd {
                outcome: TurnOutcome::Unknown,
                ..
            }]
        ));
        assert_eq!(
            d.untranslated_observations().get("turn-status:inProgress"),
            Some(&1),
            "선언에 있는 값이라도 뜻을 모르면 계수는 남아야 한다"
        );
    }

    /// Even if one field is off the boundary stands — so only this notification is not run through `parse::<T>()`.
    #[test]
    fn turn_completed_without_a_readable_status_still_ends_the_turn() {
        let line = notification_line("turn/completed", serde_json::json!({"threadId": "t-1"}));
        let mut d = CodexAppServerDecoder::new();
        match d.decode(line.as_bytes()).as_slice() {
            [OutputEvent::TurnEnd { turn_id, outcome }] => {
                assert!(turn_id.is_none());
                assert_eq!(outcome, &TurnOutcome::Unknown);
            }
            other => panic!("TurnEnd 하나가 아니다: {other:?}"),
        }
        assert_eq!(
            d.untranslated_observations().get("shape:turn/completed"),
            Some(&1)
        );
    }

    /// ★`MessageDone` is not this translator's vocabulary★ — if turn boundary and message boundary are merged into one vocabulary, the boundary splits in this backend where one turn has
    /// several completed items. If it comes back, this goes red.
    #[test]
    fn this_translator_never_emits_the_message_boundary_vocabulary() {
        let mut d = CodexAppServerDecoder::new();
        let mut all = Vec::new();
        for (method, params) in [
            (
                "turn/completed",
                serde_json::json!({"turn": {"id": "u-1", "items": [], "status": "completed"}}),
            ),
            (
                "item/completed",
                serde_json::json!({"threadId": "t", "turnId": "u", "completedAtMs": 1i64,
                                   "item": {"type": "agentMessage", "id": "i-1", "text": "hi"}}),
            ),
            (
                "item/agentMessage/delta",
                serde_json::json!({"threadId": "t", "turnId": "u", "itemId": "i", "delta": "x"}),
            ),
        ] {
            all.extend(d.decode(notification_line(method, params).as_bytes()));
        }
        assert!(
            !all.iter()
                .any(|e| matches!(e, OutputEvent::MessageDone { .. })),
            "{all:?}"
        );
    }

    /// `turn/started` is not a translation target at this stage — only the grade is left as routine.
    #[test]
    fn turn_started_stays_untranslated() {
        let line = notification_line(
            "turn/started",
            serde_json::json!({"threadId": "t", "turn": {"id": "u", "items": [], "status": "inProgress"}}),
        );
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(line.as_bytes()).is_empty());
        assert_eq!(
            d.untranslated_observations().get("method:turn/started"),
            Some(&1)
        );
    }

    /// `thread/readState/changed` comes with the `experimentalApi` opt-in (measured 0.163.0 — M3). It is a known
    /// untranslated name, so it is routine, not drift: once the routine share is full it is only counted as dropped,
    /// where a drift name is still recorded.
    // ADR-0337
    #[test]
    fn thread_read_state_changed_is_observed_as_routine_not_drift() {
        let line = notification_line(
            "thread/readState/changed",
            serde_json::json!({"threadId": "t"}),
        );
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(line.as_bytes()).is_empty());
        assert_eq!(
            d.untranslated_observations()
                .get("method:thread/readState/changed"),
            Some(&1)
        );

        let mut full = CodexAppServerDecoder::new();
        for i in 0..MAX_ROUTINE_KEYS {
            full.observe(format!("method:routine/{i}"), Observed::Routine, "");
        }
        assert!(full.decode(line.as_bytes()).is_empty());
        assert!(
            full.untranslated_observations()
                .get("method:thread/readState/changed")
                .is_none(),
            "recorded past the routine share — it was observed as drift"
        );
        let drift = notification_line("brand/new", serde_json::json!({}));
        assert!(full.decode(drift.as_bytes()).is_empty());
        assert_eq!(
            full.untranslated_observations().get("method:brand/new"),
            Some(&1),
            "control: a drift name is still recorded"
        );
    }

    // ── What is dropped — but the observation is kept ─────────────────────────────────────────

    /// ★This is where upstream grows the union★ — if silent here, the drift diagnostic closes its eyes at exactly the point where
    /// drift appears.
    #[test]
    fn an_unknown_item_type_is_counted_under_a_distinguishable_key() {
        let line = notification_line(
            "item/started",
            serde_json::json!({
                "threadId": "t", "turnId": "u", "startedAtMs": 1i64,
                "item": {"type": "quantumEntanglement", "id": "i-1", "spookiness": 9}
            }),
        );
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(line.as_bytes()).is_empty());
        assert_eq!(
            d.untranslated_observations()
                .get("item:item/started#quantumEntanglement"),
            Some(&1)
        );
        // It does not mix with the method-name axis.
        assert!(d
            .untranslated_observations()
            .get("item:item/started")
            .is_none());
    }

    /// The body is carried by the delta notification — if the item side emitted it too, the same text would show twice. The observation is still kept.
    #[test]
    fn agent_message_item_emits_nothing_but_is_observed() {
        let line = notification_line(
            "item/started",
            serde_json::json!({
                "threadId": "t", "turnId": "u", "startedAtMs": 1i64,
                "item": {"type": "agentMessage", "id": "i-1", "text": "hello"}
            }),
        );
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(line.as_bytes()).is_empty());
        assert_eq!(
            d.untranslated_observations()
                .get("item:item/started#agentMessage"),
            Some(&1)
        );
    }

    #[test]
    fn item_completed_emits_nothing_but_still_inspects_the_item_type() {
        let line = notification_line(
            "item/completed",
            serde_json::json!({
                "threadId": "t", "turnId": "u", "completedAtMs": 2i64,
                "item": {"type": "commandExecution", "id": "i-9", "command": "cargo test",
                         "cwd": "C:/w", "commandActions": [], "status": "completed"}
            }),
        );
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(line.as_bytes()).is_empty());
        assert_eq!(
            d.untranslated_observations()
                .get("item:item/completed#commandExecution"),
            Some(&1)
        );
    }

    #[test]
    fn unknown_notification_method_emits_nothing_and_is_counted_once_per_name() {
        let line = notification_line("thread/realtime/sdp", serde_json::json!({"sdp": "v=0"}));
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(line.as_bytes()).is_empty());
        assert!(d.decode(line.as_bytes()).is_empty());
        assert_eq!(
            d.untranslated_observations()
                .get("method:thread/realtime/sdp"),
            Some(&2)
        );
        assert_eq!(d.untranslated_observations().len(), 1);
    }

    /// Our vocabulary has no reasoning axis — it is dropped, but the fact that it was dropped is kept.
    #[test]
    fn reasoning_delta_is_dropped_but_observed() {
        let line = notification_line(
            "item/reasoning/textDelta",
            serde_json::json!({"threadId": "t", "turnId": "u", "itemId": "i", "delta": "musing"}),
        );
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(line.as_bytes()).is_empty());
        assert_eq!(
            d.untranslated_observations()
                .get("method:item/reasoning/textDelta"),
            Some(&1)
        );
    }

    /// ★Warnings the peer sends are also squashed to once per key★ — the same discipline as every other peer-originated warn in this file.
    /// If emitted every time, it opens the same flood into the daemon's synchronous file sink.
    #[test]
    fn deprecation_notice_emits_nothing_and_is_throttled_like_every_other_peer_warning() {
        let line = notification_line("deprecationNotice", serde_json::json!({"message": "gone"}));
        let mut d = CodexAppServerDecoder::new();
        for _ in 0..50 {
            assert!(d.decode(line.as_bytes()).is_empty());
        }
        let seen = d.untranslated_observations();
        assert_eq!(seen.get("deprecation:notice"), Some(&50));
        assert_eq!(seen.len(), 1, "키 하나로 모여야 한다(로그는 첫 1 회뿐)");
    }

    // ── Defects are recorded as defects (not mixed into routine noise) ────────────────────

    #[test]
    fn known_method_with_a_wrong_shape_emits_nothing_and_is_recorded_as_a_defect() {
        let line = notification_line(
            "item/agentMessage/delta",
            serde_json::json!({"threadId": "t", "turnId": "u", "itemId": "i"}),
        );
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(line.as_bytes()).is_empty());
        assert_eq!(
            d.untranslated_observations()
                .get("shape:item/agentMessage/delta"),
            Some(&1)
        );
    }

    /// ★If the shape of the most frequent notification changes, a warning goes out on every delta★ — it must be squashed to once per key.
    #[test]
    fn a_repeated_shape_defect_is_counted_but_logged_once() {
        let line = notification_line(
            "item/agentMessage/delta",
            serde_json::json!({"threadId": "t", "turnId": "u", "itemId": "i"}),
        );
        let mut d = CodexAppServerDecoder::new();
        for _ in 0..100 {
            assert!(d.decode(line.as_bytes()).is_empty());
        }
        assert_eq!(
            d.untranslated_observations()
                .get("shape:item/agentMessage/delta"),
            Some(&100),
            "키 하나로 모여야 한다(로그는 첫 1 회뿐)"
        );
        assert_eq!(d.untranslated_observations().len(), 1);
    }

    #[test]
    fn known_method_without_params_is_recorded_as_a_defect() {
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(b"{\"method\":\"error\"}\n").is_empty());
        assert_eq!(d.untranslated_observations().get("params:error"), Some(&1));
    }

    /// ★A tool variant with a missing required field is a defect★ — if sent off as a routine count it becomes quiet noise.
    #[test]
    fn a_tool_item_missing_its_tool_field_is_recorded_as_a_defect() {
        let line = notification_line(
            "item/started",
            serde_json::json!({
                "threadId": "t", "turnId": "u", "startedAtMs": 1i64,
                "item": {"type": "mcpToolCall", "id": "i-1", "server": "fs", "status": "inProgress"}
            }),
        );
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(line.as_bytes()).is_empty());
        assert_eq!(
            d.untranslated_observations()
                .get("tool:item/started#mcpToolCall"),
            Some(&1)
        );
        assert!(
            d.untranslated_observations()
                .get("item:item/started#mcpToolCall")
                .is_none(),
            "결함이 「아는 변형, 번역 안 함」 계수로 새어 들어갔다"
        );
    }

    #[test]
    fn an_item_without_a_type_string_is_recorded_as_a_defect() {
        let line = notification_line(
            "item/started",
            serde_json::json!({"threadId": "t", "turnId": "u", "startedAtMs": 1i64, "item": {"id": "i-1"}}),
        );
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(line.as_bytes()).is_empty());
        assert_eq!(
            d.untranslated_observations().get("item-type:item/started"),
            Some(&1)
        );
    }

    /// ★A failure that empties the screen must be recorded in the shipping configuration★ — if these two are silent, it cannot be told apart from "the agent is
    /// idle".
    #[test]
    fn unreadable_lines_are_recorded_as_defects_not_silence() {
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(b"Warning: something happened\n").is_empty());
        assert!(d.decode(&[0xff, 0xfe, 0x00, b'\n']).is_empty());
        let seen = d.untranslated_observations();
        assert_eq!(seen.get("envelope:not-json"), Some(&1));
        assert_eq!(seen.get("line:non-utf8"), Some(&1));
    }

    #[test]
    fn envelopes_the_transport_should_have_kept_are_recorded() {
        let mut d = CodexAppServerDecoder::new();
        assert!(d
            .decode(b"{\"id\":7,\"method\":\"item/tool/call\",\"params\":{}}\n")
            .is_empty());
        assert!(d
            .decode(b"{\"id\":1,\"result\":{\"thread\":{\"id\":\"t\"}}}\n")
            .is_empty());
        let seen = d.untranslated_observations();
        assert_eq!(seen.get("envelope:request"), Some(&1));
        assert_eq!(seen.get("envelope:response"), Some(&1));
    }

    #[test]
    fn blank_lines_are_not_recorded() {
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(b"\n   \n\n").is_empty());
        assert!(d.untranslated_observations().is_empty());
    }

    // ── Cap on growth the peer decides ────────────────────────────────────────────

    /// ★Routine names must not eat into the drift share★ — if they do, the detector switches itself off.
    #[test]
    fn routine_keys_cannot_crowd_out_drift_keys_in_any_arrival_order() {
        // ★Push in **more** distinct routine keys than the total cap★ — with the real notification-name list
        //   there are only 75 distinct keys, which does not even get near the cap, and then this
        //   test stays green even if the share is removed entirely (that was the defect of the old form).
        let flood = MAX_UNTRANSLATED_KEYS + 50;
        for drift_first in [false, true] {
            let mut d = CodexAppServerDecoder::new();
            if drift_first {
                d.observe("method:early/drift".to_string(), Observed::Drift, "");
            }
            for i in 0..flood {
                d.observe(format!("method:routine/{i}"), Observed::Routine, "");
            }

            let routine = d
                .untranslated_observations()
                .keys()
                .filter(|k| k.starts_with("method:routine/"))
                .count();
            assert!(
                routine <= MAX_ROUTINE_KEYS,
                "일상 키가 자기 몫을 넘겼다: {routine} (drift_first={drift_first})"
            );

            // No matter how much is pushed in as routine, the drift entry remains. ★Recorded = that key was seen for the first
            //   time, and that is exactly the condition under which the log (warn) goes out★ — there is no means to capture the log itself here,
            //   so recording is measured instead.
            d.observe("method:late/drift".to_string(), Observed::Drift, "");
            assert_eq!(
                d.untranslated_observations().get("method:late/drift"),
                Some(&1),
                "일상 키가 드리프트 칸을 먹었다 (drift_first={drift_first})"
            );
            if drift_first {
                assert_eq!(
                    d.untranslated_observations().get("method:early/drift"),
                    Some(&1),
                    "먼저 온 드리프트 키가 밀려났다"
                );
            }
        }
    }

    /// ★If a method name made by the peer fell into an item-axis entry, detection of that entry would die★ — the prefix
    /// structurally prevents it.
    #[test]
    fn a_forged_method_name_cannot_land_in_the_item_axis() {
        let line = notification_line(
            "item/started",
            serde_json::json!({
                "threadId": "t", "turnId": "u", "startedAtMs": 1i64,
                "item": {"type": "agentMessage", "id": "i-1", "text": "hello"}
            }),
        );
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(line.as_bytes()).is_empty());
        // The peer sends a method name **identical character for character** to that item-axis key.
        d.translate("item/started#agentMessage", None);

        let seen = d.untranslated_observations();
        assert_eq!(seen.get("item:item/started#agentMessage"), Some(&1));
        assert_eq!(seen.get("method:item/started#agentMessage"), Some(&1));
    }

    #[test]
    fn distinct_names_are_capped_and_the_excess_is_counted() {
        let mut d = CodexAppServerDecoder::new();
        for i in 0..(MAX_UNTRANSLATED_KEYS + 10) {
            d.translate(&format!("drift/name/{i}"), None);
        }
        assert_eq!(d.untranslated_observations().len(), MAX_UNTRANSLATED_KEYS);
        assert_eq!(d.untranslated_dropped(), 10);
        // Even after reaching the cap, **keys already known** are still counted.
        d.translate("drift/name/0", None);
        assert_eq!(
            d.untranslated_observations().get("method:drift/name/0"),
            Some(&2)
        );
    }

    /// The peer can take memory through name length too — the count share alone cannot prevent it.
    #[test]
    fn an_absurdly_long_name_is_bounded_before_it_is_stored() {
        let mut d = CodexAppServerDecoder::new();
        d.translate(&"n".repeat(100_000), None);
        let seen = d.untranslated_observations();
        let (key, _) = seen.iter().next().unwrap();
        assert!(key.chars().count() <= LOG_STRING_LIMIT + 1, "{}", key.len());
    }

    // ── Determinism ───────────────────────────────────────────────────────────────

    /// The same line always emits the same event — pin the content too (measuring only sameness passes even when always empty).
    #[test]
    fn the_same_line_always_yields_the_same_events() {
        let line = format!("{}\n", delta_line("x"));
        let mut d = CodexAppServerDecoder::new();
        let first = format!("{:?}", d.decode(line.as_bytes()));
        let second = format!("{:?}", d.decode(line.as_bytes()));
        assert_eq!(first, second);
        assert!(first.contains("TextDelta"), "{first}");
        assert!(first.contains("\"x\""), "{first}");
    }

    // ── Log · diagnostic hygiene ───────────────────────────────────────────────

    /// ★If masking is removed it must go red★ — an assertion that measured only the cap passes even without `mask_secrets`.
    #[test]
    fn sanitize_masks_credentials() {
        let secret = "sk-proj-ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let out = sanitize(&format!("token={secret} trailing"), LOG_STRING_LIMIT);
        assert!(!out.contains(secret), "자격증명이 마스킹되지 않았다: {out}");
        assert!(out.contains("***"), "{out}");
        let bearer = sanitize("Authorization: Bearer abcdefghijklmnop", LOG_STRING_LIMIT);
        assert!(!bearer.contains("abcdefghijklmnop"), "{bearer}");
    }

    /// ★A failure reason the peer wrote goes to the screen, to the log and **to the wire** — masking is done not in front of those three but at the place
    /// where it is made★. This reason is carried in the outcome field of `TurnEnd` and flows through the daemon to the frontend, so if it is
    /// missed here, the credential stays along that whole path.
    #[test]
    fn a_secret_inside_a_failed_turns_reason_never_leaves_this_module() {
        let secret = "sk-proj-ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let events = decode_notification(
            "turn/completed",
            serde_json::json!({
                "threadId": "t-1",
                "turn": {"id": "u-1", "items": [], "status": "failed",
                         "error": {"message": format!("bad config token={secret}"),
                                   "additionalDetails": format!("retry with {secret}"),
                                   "misalignment": {"steer": {"message": format!("use {secret}")}}}}
            }),
        );
        match events.as_slice() {
            [OutputEvent::TurnEnd {
                outcome:
                    TurnOutcome::Failed {
                        detail: Some(detail),
                    },
                ..
            }] => {
                assert!(
                    !detail.contains(secret),
                    "자격증명이 그대로 실렸다: {detail}"
                );
                assert!(detail.contains("***"), "{detail}");
            }
            other => panic!("사유를 실은 Failed 결말이 아니다: {other:?}"),
        }
    }

    /// Another caller that uses the same string maker — the `error` notification must also pass through the same door.
    #[test]
    fn a_secret_inside_an_error_notification_never_leaves_this_module() {
        let secret = "sk-proj-ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let events = decode_notification(
            "error",
            serde_json::json!({"error": {"message": format!("boom token={secret}")},
                               "willRetry": false}),
        );
        match events.as_slice() {
            [OutputEvent::Error(message)] => {
                assert!(
                    !message.contains(secret),
                    "자격증명이 그대로 실렸다: {message}"
                );
                assert!(message.contains("***"), "{message}");
            }
            other => panic!("Error 하나가 아니다: {other:?}"),
        }
    }

    /// ★[`truncate`] has **only two named doors**★ — [`sanitize`] (truncate after masking · for diagnostic strings) and
    /// [`clip`] (truncate only · for transcript body). If called from anywhere else, it means nobody has ever answered 「이 문자열은 진단인가 기록인가」 (is this string a diagnostic or a record),
    /// and if it was a diagnostic it skips masking and goes out to the screen · log · wire (that actually
    /// happened — the error string maker only truncated and skipped masking).
    ///
    /// This item measures that property **in the source** — keeping one credential item per leaking path does not last,
    /// since a path without one becomes the leaking path (this folder's `transport` uses the same form).
    /// ★But this axis cannot see **length**★ — a path that uses neither of the two doors is structurally invisible here.
    /// That is covered by `no_translated_notification_can_produce_an_event_that_evicts_the_replay_ring`.
    #[test]
    fn truncate_is_reachable_only_through_its_two_named_doors() {
        let src = include_str!("decoder.rs");
        // ★Do not split by `#[cfg(test)]`★ — that attribute is also inside the production section, so if cut there this item
        //   scans only the front part and does not see the rest. ★Do not use a marker containing a line break either★ — the checkout may be CRLF.
        let production = src.split("mod tests {").next().expect("운영 구획");
        let offenders: Vec<&str> = production
            .lines()
            .map(|l| l.trim())
            .filter(|l| l.contains("truncate("))
            .filter(|l| !l.starts_with("//") && !l.starts_with("///"))
            // Allowed = the definition itself + the bodies of the two doors (`sanitize` · `clip`). To add a third, the question in the doc above
            //   must be answered first.
            .filter(|l| {
                !l.starts_with("fn truncate(")
                    && !l.starts_with("truncate(&mask_secrets(")
                    && !l.starts_with("truncate(s, limit)")
            })
            .collect();
        assert!(
            offenders.is_empty(),
            "이름 붙은 두 문 밖에서 절단을 부른다: {offenders:?}"
        );
    }

    /// The weight the replay ring allows for **one event** (= `max_bytes` of `output_core::Ring`). That ring evicts everything else to fit one entry that
    /// exceeds this value alone.
    const RING_SINGLE_EVENT_LIMIT: usize = 2 * 1024 * 1024;

    /// Use the very axis the ring counts (`output_core::estimate_cost_bytes`) — if copied separately here, the two diverge and what this
    /// item measures differs from what the ring counts.
    fn event_weight(e: &OutputEvent) -> usize {
        crate::output_core::estimate_cost_bytes(e)
    }

    /// ★Catch the unbounded emit path **in the output**★ — the source item above structurally cannot see a path that uses neither of the two
    /// doors. Such a spot actually existed: the echoed user message carried `content[].text` as is,
    /// and one big paste emptied that agent's whole replay (with neither cap nor masking, both items
    /// closed their eyes). Instead, this item fills the field whose length the peer decides hugely for every notification we translate,
    /// and measures whether the emitted event is under the ring's single-event cap.
    ///
    /// ★If the table below disagrees with the arm list of `translate`, it goes red★ — if we start translating a new notification without
    /// putting on a cap, that spot is caught here. Growing the table is itself answering that question.
    #[test]
    fn no_translated_notification_can_produce_an_event_that_evicts_the_replay_ring() {
        let huge = "x".repeat(2_500_000);
        let long_id = "i".repeat(MAX_ID_BYTES + 64);
        let usage_cell = serde_json::json!({
            "inputTokens": 1, "cachedInputTokens": 0, "outputTokens": 2,
            "reasoningOutputTokens": 0, "totalTokens": 3
        });
        let cases: Vec<(&str, &str, Value)> = vec![
            (
                "ITEM_AGENT_MESSAGE_DELTA",
                method::ITEM_AGENT_MESSAGE_DELTA,
                serde_json::json!({"threadId": "t", "turnId": long_id, "itemId": long_id, "delta": huge}),
            ),
            (
                "ITEM_STARTED",
                method::ITEM_STARTED,
                serde_json::json!({"threadId": "t", "turnId": long_id,
                    "item": {"type": "commandExecution", "id": long_id, "command": huge}}),
            ),
            (
                "ITEM_COMPLETED",
                method::ITEM_COMPLETED,
                serde_json::json!({"threadId": "t", "turnId": long_id,
                    "item": {"type": "userMessage", "id": long_id,
                             "content": [{"type": "text", "text": huge}]}}),
            ),
            (
                "THREAD_TOKEN_USAGE_UPDATED",
                method::THREAD_TOKEN_USAGE_UPDATED,
                serde_json::json!({"threadId": "t", "turnId": long_id,
                    "tokenUsage": {"last": usage_cell, "total": usage_cell}}),
            ),
            (
                "ERROR",
                method::ERROR,
                serde_json::json!({"error": {"message": huge, "additionalDetails": "d"}, "willRetry": true}),
            ),
            (
                "TURN_COMPLETED",
                method::TURN_COMPLETED,
                serde_json::json!({"threadId": "t",
                    "turn": {"id": long_id, "status": "failed", "error": {"message": huge}}}),
            ),
            (
                "DEPRECATION_NOTICE",
                method::DEPRECATION_NOTICE,
                serde_json::json!({"note": huge}),
            ),
            (
                "ACCOUNT_RATE_LIMITS_UPDATED",
                method::ACCOUNT_RATE_LIMITS_UPDATED,
                serde_json::json!({"rateLimits": {"limitId": huge, "limitName": huge,
                    "primary": {"usedPercent": 1, "windowDurationMins": 300}}}),
            ),
        ];

        let src = include_str!("decoder.rs");
        let production = src.split("mod tests {").next().expect("운영 구획");
        let mut arms: Vec<&str> = production
            .lines()
            .map(|l| l.trim())
            .filter(|l| l.starts_with("method::") && l.contains("=>"))
            .map(|l| {
                l.trim_start_matches("method::")
                    .split(' ')
                    .next()
                    .expect("arm 이름")
            })
            .collect();
        arms.sort_unstable();
        let mut covered: Vec<&str> = cases.iter().map(|c| c.0).collect();
        covered.sort_unstable();
        assert_eq!(arms, covered, "translate 의 arm 과 이 표가 어긋난다");

        for (name, method_name, params) in cases {
            for event in decode_notification(method_name, params) {
                let weight = event_weight(&event);
                assert!(
                    weight <= RING_SINGLE_EVENT_LIMIT,
                    "{name}: 이벤트 하나가 {weight}B — 링 상한({RING_SINGLE_EVENT_LIMIT}B)을 넘어 replay 를 비운다"
                );
            }
        }
    }

    // ── Usage limit notification (`account/rateLimits/updated`) — the interpretation rules themselves are measured by the `usage.rs` tests ──

    fn rate_limits_line(snapshot: Value) -> String {
        notification_line(
            method::ACCOUNT_RATE_LIMITS_UPDATED,
            serde_json::json!({ "rateLimits": snapshot }),
        )
    }

    fn default_bucket() -> Value {
        serde_json::json!({
            "limitId": "codex",
            "primary": {"usedPercent": 37, "windowDurationMins": 300, "resetsAt": 1_790_424_706i64},
            "secondary": {"usedPercent": 58, "windowDurationMins": 10080, "resetsAt": 1_790_739_378i64},
        })
    }

    /// ★0 screen events + 1 observation★ — if emitted as an event, account state would be carried in per-agent replay.
    #[test]
    fn rate_limits_update_is_collected_without_output_events() {
        let line = rate_limits_line(default_bucket());
        let mut d = CodexAppServerDecoder::new();
        // Even if a chunk boundary falls in the middle of a line, it is picked up only from complete lines.
        let (head, tail) = line.as_bytes().split_at(line.len() / 2);
        assert!(d.decode(head).is_empty());
        assert!(d.take_usage().is_empty(), "줄이 안 끝났는데 주웠다");
        assert!(
            d.decode(tail).is_empty(),
            "사용량 알림이 화면 이벤트를 냈다"
        );

        let taken = d.take_usage();
        assert_eq!(taken.len(), 1);
        let obs = &taken[0];
        assert_eq!(obs.vendor, usage::USAGE_VENDOR);
        assert_eq!(obs.five_hour.and_then(|w| w.used_pct), Some(37.0));
        assert_eq!(obs.weekly.and_then(|w| w.used_pct), Some(58.0));
        assert!(d.take_usage().is_empty(), "take_usage 는 비운다");
        assert!(d.flush().is_empty());
        assert!(d.take_usage().is_empty());
    }

    /// Several lines of the same chunk are gathered in arrival order.
    #[test]
    fn several_updates_in_one_chunk_arrive_in_order() {
        let mut first = default_bucket();
        first["primary"]["usedPercent"] = 10.into();
        let mut second = default_bucket();
        second["primary"]["usedPercent"] = 20.into();
        let chunk = format!("{}{}", rate_limits_line(first), rate_limits_line(second));
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(chunk.as_bytes()).is_empty());
        let pcts: Vec<_> = d
            .take_usage()
            .iter()
            .map(|o| o.five_hour.and_then(|w| w.used_pct))
            .collect();
        assert_eq!(pcts, vec![Some(10.0), Some(20.0)]);
    }

    /// ★It is no longer counted as a 「알면서 안 옮기는 이름」 (name knowingly not translated)★ — if a notification we translate keeps occupying the routine observation field, that share shrinks.
    #[test]
    fn rate_limits_update_is_no_longer_an_untranslated_observation() {
        let mut d = CodexAppServerDecoder::new();
        d.decode(rate_limits_line(default_bucket()).as_bytes());
        assert!(
            d.untranslated_observations().is_empty(),
            "{:?}",
            d.untranslated_observations()
        );
    }

    /// A dropped observation yields neither event nor observation, and only an envelope defect is left as a defect.
    #[test]
    fn unusable_rate_limits_updates_yield_nothing() {
        let mut d = CodexAppServerDecoder::new();
        for snapshot in [
            serde_json::json!({"limitId": "base_model_inference",
                "primary": {"usedPercent": 0, "windowDurationMins": 10080}}),
            serde_json::json!({"primary": null, "secondary": null}),
            serde_json::json!({"primary": {"usedPercent": 9, "windowDurationMins": 43200}}),
            serde_json::json!({}),
        ] {
            assert!(d
                .decode(rate_limits_line(snapshot.clone()).as_bytes())
                .is_empty());
            assert!(d.take_usage().is_empty(), "{snapshot}");
        }
        assert!(
            d.untranslated_observations().is_empty(),
            "모양이 맞는 봉투는 결함이 아니다"
        );

        // If `rateLimits` is absent or not an object, it is an envelope defect — left under the same key as the other arms.
        for params in [
            serde_json::json!({}),
            serde_json::json!({"rateLimits": null}),
            serde_json::json!({"rateLimits": "x"}),
            serde_json::json!("x"),
        ] {
            let line = notification_line(method::ACCOUNT_RATE_LIMITS_UPDATED, params.clone());
            assert!(d.decode(line.as_bytes()).is_empty(), "{params}");
            assert!(d.take_usage().is_empty(), "{params}");
        }
        assert!(
            d.untranslated_observations()
                .contains_key("shape:account/rateLimits/updated"),
            "{:?}",
            d.untranslated_observations()
        );
    }

    /// ★Do not read an array in field order to make a plausible observation★ — all three became observations carrying windows without an object check
    /// (params array → 5 hours 11 · snapshot array → two windows · snapshot array led by `null` → 5 hours 14).
    #[test]
    fn non_object_rate_limits_envelopes_are_not_read_positionally() {
        for params in [
            serde_json::json!([{"limitId": "codex",
                "primary": {"usedPercent": 11, "windowDurationMins": 300}}]),
            serde_json::json!({"rateLimits": ["codex",
                {"usedPercent": 12, "windowDurationMins": 300},
                {"usedPercent": 13, "windowDurationMins": 10080}]}),
            serde_json::json!({"rateLimits": [null,
                {"usedPercent": 14, "windowDurationMins": 300}]}),
        ] {
            let mut d = CodexAppServerDecoder::new();
            let line = notification_line(method::ACCOUNT_RATE_LIMITS_UPDATED, params.clone());
            assert!(d.decode(line.as_bytes()).is_empty(), "{params}");
            assert!(d.take_usage().is_empty(), "{params}");
            assert!(
                d.untranslated_observations()
                    .contains_key("shape:account/rateLimits/updated"),
                "봉투 결함으로 남아야 한다: {params} → {:?}",
                d.untranslated_observations()
            );
        }
    }

    // ── History restore translation (ADR-0203) ───────────────────────────────────────────────

    /// Feed the same item to each of the three doors and look only at **what comes out**.
    fn through_each_door(item: Value) -> (Vec<OutputEvent>, Vec<OutputEvent>, Vec<OutputEvent>) {
        let params = serde_json::json!({"threadId": "th", "turnId": "tu", "item": item});
        (
            decode_notification(method::ITEM_STARTED, params.clone()),
            decode_notification(method::ITEM_COMPLETED, params),
            CodexAppServerDecoder::new().history_item("tu", &item),
        )
    }

    /// ★★The assistant body is carried **only from history**★★ — in live the same body has already streamed as
    /// `item/agentMessage/delta` and been drawn on the screen, so emitting it again here would leave one answer twice.
    /// But the page response has no such delta, so if it is not carried, **the restored screen has not one line of
    /// the assistant's words** (in the measured sample 5 of 120 items are this variant).
    #[test]
    fn an_agent_message_item_becomes_text_only_on_the_history_door() {
        let (started, completed, history) = through_each_door(
            serde_json::json!({"type": "agentMessage", "id": "m1", "text": "hello"}),
        );
        assert!(
            started.is_empty(),
            "라이브가 델타와 겹쳐 두 벌을 낸다: {started:?}"
        );
        assert!(completed.is_empty(), "{completed:?}");
        match history.as_slice() {
            [OutputEvent::TextDelta {
                text,
                turn_id,
                message_id,
            }] => {
                assert_eq!(text, "hello");
                assert_eq!(turn_id.as_deref(), Some("tu"));
                assert_eq!(message_id.as_deref(), Some("m1"));
            }
            other => panic!("어시스턴트 본문 하나가 아니다: {other:?}"),
        }
    }

    /// ★Tool calls also come out of history — the emit axis is not `started` alone but 「`completed` 가 아닐 것」 (not being completed)★.
    /// In the measured sample this variant is 44 of 120, so if it were missing the restored screen would be practically empty.
    #[test]
    fn a_tool_item_is_emitted_on_the_history_door_just_like_item_started() {
        let (started, completed, history) = through_each_door(serde_json::json!({
            "type": "commandExecution", "id": "c1", "command": "ls"
        }));
        assert_eq!(started.len(), 1, "{started:?}");
        assert!(
            completed.is_empty(),
            "라이브 `completed` 는 여전히 안 낸다: {completed:?}"
        );
        assert_eq!(history.len(), 1, "{history:?}");
        assert!(
            matches!(history[0], OutputEvent::ToolCall { .. })
                && matches!(started[0], OutputEvent::ToolCall { .. }),
            "이력과 `started` 의 산출이 갈렸다: {history:?} / {started:?}"
        );
    }

    /// The echoed user utterance comes out of all three doors — in history it is the only material for 「누가 무엇을 물었나」 (who asked what).
    #[test]
    fn a_user_message_item_comes_through_the_history_door_too() {
        let (_, _, history) = through_each_door(serde_json::json!({
            "type": "userMessage", "id": "u1",
            "content": [{"type": "text", "text": "안녕"}]
        }));
        match history.as_slice() {
            [OutputEvent::Structured { kind, json }] => {
                assert_eq!(kind, "user");
                assert!(json.contains("\"uuid\":\"u1\""), "{json}");
                assert!(json.contains("안녕"), "{json}");
            }
            other => panic!("유저 말풍선 하나가 아니다: {other:?}"),
        }
    }

    /// ★A variant live drops is dropped in history too★ — if a history-only arm is added for reasoning · sub-agent activity and the like,
    /// the two screens draw different conversations.
    #[test]
    fn the_history_door_drops_exactly_what_the_live_doors_drop() {
        for kind in ["reasoning", "subAgentActivity", "contextCompaction", "plan"] {
            let (started, completed, history) =
                through_each_door(serde_json::json!({"type": kind, "id": "x"}));
            assert!(started.is_empty(), "{kind}: {started:?}");
            assert!(completed.is_empty(), "{kind}: {completed:?}");
            assert!(history.is_empty(), "{kind}: {history:?}");
        }
    }

    /// ★One history event cannot empty the replay ring★ — this is the same wall the sibling item
    /// (`no_translated_notification_can_produce_an_event_that_evicts_the_replay_ring`) puts on the notification side, and the history door is **outside**
    /// that item's reach, so it is measured separately here.
    #[test]
    fn no_history_item_can_produce_an_event_that_evicts_the_replay_ring() {
        let huge = "가".repeat(MAX_LINE_BYTES / 4);
        let long_id = "i".repeat(MAX_ID_BYTES + 1);
        let cases = [
            serde_json::json!({"type": "agentMessage", "id": long_id, "text": huge}),
            serde_json::json!({"type": "userMessage", "id": long_id,
                "content": [{"type": "text", "text": huge}]}),
            serde_json::json!({"type": "commandExecution", "id": long_id, "command": huge}),
        ];
        for item in cases {
            for event in CodexAppServerDecoder::new().history_item(&long_id, &item) {
                let weight = event_weight(&event);
                assert!(
                    weight <= RING_SINGLE_EVENT_LIMIT,
                    "이력 이벤트 하나가 {weight}B — 링 상한({RING_SINGLE_EVENT_LIMIT}B)을 넘어 replay 를 비운다"
                );
            }
        }
    }

    /// ★★The history turn boundary **does not claim an outcome**★★ — a page does not carry how the past turn ended,
    /// so there is no value to fill into `TurnEnd`. `Unknown` draws a 「모름」 (unknown) marker at every boundary, and
    /// `Completed` is an invented outcome (that type's doc forbids it).
    #[test]
    fn a_history_turn_boundary_closes_without_claiming_an_outcome() {
        match history_turn_boundary(Some("tu")) {
            OutputEvent::MessageDone {
                turn_id,
                message_id,
            } => {
                assert_eq!(turn_id.as_deref(), Some("tu"));
                assert_eq!(message_id, None);
            }
            other => panic!("경계가 `MessageDone` 이 아니다 — 결말을 지어내고 있다: {other:?}"),
        }
        // It is a comparison token, so over the cap it is filtered, not truncated (the same decision = `bounded_id`).
        match history_turn_boundary(Some(&"t".repeat(MAX_ID_BYTES + 1))) {
            OutputEvent::MessageDone { turn_id, .. } => assert_eq!(turn_id, None),
            other => panic!("경계가 `MessageDone` 이 아니다: {other:?}"),
        }
        // Even if the id cannot be carried the boundary stands — if it is not closed, the waiting indicator spins forever.
        match history_turn_boundary(None) {
            OutputEvent::MessageDone { turn_id, .. } => assert_eq!(turn_id, None),
            other => panic!("경계가 `MessageDone` 이 아니다: {other:?}"),
        }
    }

    /// The caller may print the diagnostic map anywhere — it must go out masked at the boundary.
    #[test]
    fn the_observation_map_hands_back_masked_keys() {
        let secret = "sk-proj-ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let mut d = CodexAppServerDecoder::new();
        d.translate(&format!("drift/{secret}"), None);
        let seen = d.untranslated_observations();
        let (key, _) = seen.iter().next().unwrap();
        assert!(
            !key.contains(secret),
            "마스킹되지 않은 키가 새어 나갔다: {key}"
        );
    }

    /// ★If truncated then masked, a credential straddling the boundary survives★ — the pattern of `mask_secrets` requires
    /// 20 or more characters after `sk-proj-`, so a piece cut first is not caught by that quantifier.
    #[test]
    fn a_credential_straddling_the_truncation_boundary_is_still_masked() {
        // ★Pass the key to `observe` **directly**★ — if it goes through `translate`, the `method:` prefix pushes the truncation
        //   point, and the credential, instead of straddling the boundary, is cut off whole (then this test
        //   measures nothing).
        // Fill so that the truncation point falls after `sk-proj-` 8 characters + a 4-character tail — that 4-character tail
        //   falls short of the 20-character quantifier `mask_secrets` requires.
        let padding = "a".repeat(LOG_STRING_LIMIT - 12);
        let key = format!("{padding}sk-proj-{}", "B".repeat(40));
        let mut d = CodexAppServerDecoder::new();
        d.observe(key, Observed::Drift, "");

        let seen = d.untranslated_observations();
        let (key, _) = seen.iter().next().unwrap();
        assert!(
            !key.contains("sk-proj-"),
            "경계에 걸친 자격증명이 마스킹을 빠져나갔다: {key}"
        );
        assert!(key.chars().count() <= LOG_STRING_LIMIT + 1, "{}", key.len());
    }

    #[test]
    fn sanitize_bounds_the_length() {
        let out = sanitize(&"a".repeat(LOG_STRING_LIMIT * 4), LOG_STRING_LIMIT);
        assert!(out.chars().count() <= LOG_STRING_LIMIT + 1, "{}", out.len());
    }

    #[test]
    fn truncate_cuts_on_character_boundaries() {
        assert_eq!(truncate(&"한".repeat(10), 3), "한한한…");
    }

    // ── Vocabulary of the tool-end trigger (ADR-0231 · ADR-0235) ───────────────────

    /// Tool = only the six tool-call vocabularies — output (`agentMessage`·`reasoning`·`plan`) · echo · known variants with no grounds to be tools ·
    /// unknown variants · the empty string are all not (their ends are not triggers).
    #[test]
    fn is_tool_item_reads_the_tool_vocabulary_and_nothing_else() {
        let table: &[(&str, bool)] = &[
            ("commandExecution", true),
            ("fileChange", true),
            ("mcpToolCall", true),
            ("dynamicToolCall", true),
            ("collabAgentToolCall", true),
            ("webSearch", true),
            ("agentMessage", false),
            ("plan", false),
            ("reasoning", false),
            ("userMessage", false),
            ("hookPrompt", false),
            ("contextCompaction", false),
            ("enteredReviewMode", false),
            ("exitedReviewMode", false),
            ("imageView", false),
            ("imageGeneration", false),
            ("sleep", false),
            ("subAgentActivity", false),
            ("functionCallOutput", false),
            ("someFutureVariant", false),
            ("", false),
        ];
        for (kind, tool) in table {
            assert_eq!(is_tool_item(kind), *tool, "{kind}");
        }
        // The table covers every known variant — so a new schema variant does not slip quietly out of this table.
        for kind in KNOWN_ITEM_TYPES {
            assert!(
                table.iter().any(|(k, _)| k == kind),
                "{kind} 가 분류 표에 없다"
            );
        }
        for kind in TOOL_ITEM_TYPES {
            assert!(
                KNOWN_ITEM_TYPES.contains(kind),
                "{kind} 가 스키마 어휘에 없다"
            );
        }
    }
}
