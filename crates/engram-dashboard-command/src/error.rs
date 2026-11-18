//! Typed error model — **one** error vocabulary whose meaning holds across hops (TRD §4-⑦).

use std::fmt;

/// Stamps the vocabulary **from one table** — variant · wire string · [`ErrorCode::ALL`] membership · retry directive come out together.
///
/// ★With two lists, adding a code fixes only one side★: the compiler demands only the new `as_str` arm, not the `ALL` entry, so a new code
///   **goes out under its own name and comes back as `INTERNAL`**
/// (the round-trip test iterates `ALL`, so it cannot see it either). Here, omitting any of the three fields **stops compilation.**
macro_rules! error_codes {
    (
        $(#[$enum_meta:meta])*
        pub enum $name:ident {
            $( $(#[$code_meta:meta])* $variant:ident => $wire:literal retry $retry:ident, )+
        }
    ) => {
        $(#[$enum_meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $( $(#[$code_meta])* $variant, )+
        }

        impl $name {
            /// The full vocabulary — exactly the table above.
            const ALL: &'static [Self] = &[ $( Self::$variant, )+ ];

            pub const fn as_str(self) -> &'static str {
                match self {
                    $( Self::$variant => $wire, )+
                }
            }

            /// `None` for an unknown code — lets the caller decide for itself 「whether to downgrade」.
            pub fn from_wire(code: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|c| c.as_str() == code)
            }

            /// The retry directive the code decides.
            ///
            /// ★Gathered in one place★: "may it be retried" is a function of **certainty** (TRD §4-④); hand-attached directives would
            ///   differ per call site for the same code.
            /// ★The table's third field★: kept outside, a catch-all arm (`_ => Never`) appears and a new code goes out as "no retry"
            ///   **without any decision**.
            // ADR-0159
            pub const fn default_retry(self) -> RetryMode {
                match self {
                    $( Self::$variant => RetryMode::$retry, )+
                }
            }
        }
    };
}

error_codes! {
    /// Transport·routing layer + common error codes.
    ///
    /// Transport·routing layer + common error codes.
    ///
    /// ★The wire form is a string and the decoder **accepts unknown codes**★ — downgraded to [`ErrorCode::Internal`] (and [`CommandError`]
    ///   deserialization downgrades `retry` to [`RetryMode::Never`]);
    /// a closed enum would break old clients on every added code. ★Adding codes is additive, changing meaning is forbidden★ (TRD §4-③).
    // ADR-0155
    pub enum ErrorCode {
        InvalidArgument => "INVALID_ARGUMENT" retry Never,
        UnknownCommand => "UNKNOWN_COMMAND" retry Never,
        /// ★No production path emits this code now — it remains only in the vocabulary★: a disconnected owner's name disappears from the
        ///   registry ([`crate::Roster::disconnect`] · ADR-0150 decision 3),
        /// so delivery answers `UNKNOWN_COMMAND`. Not removed: unknown codes are downgraded to `INTERNAL`·`retry: never` (header above),
        ///   so a counterpart still sending it would lose "retry after
        /// the condition changes"; changing its meaning is forbidden too (TRD §4-③).
        OwnerUnavailable => "OWNER_UNAVAILABLE" retry AfterCondition,
        OutcomeUnknown => "OUTCOME_UNKNOWN" retry SameRequestId,
        Timeout => "TIMEOUT" retry SameRequestId,
        RequestIdConflict => "REQUEST_ID_CONFLICT" retry Never,
        AlreadyApplied => "ALREADY_APPLIED" retry Never,
        AuthFailed => "AUTH_FAILED" retry Never,
        ProtocolMismatch => "PROTOCOL_MISMATCH" retry Never,
        NotFound => "NOT_FOUND" retry Never,
        Conflict => "CONFLICT" retry Never,
        Unsupported => "UNSUPPORTED" retry Never,
        Internal => "INTERNAL" retry Never,
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Retry directive.
///
/// ★`SameRequestId` is not "safe to re-execute"★ — whether it was applied is **unknown**, so ask again **only with the same id** (a new id
///   could apply the operation twice — TRD §4-⑥).
// ADR-0159
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RetryMode {
    Never,
    SameRequestId,
    AfterCondition,
}

impl RetryMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Never => "never",
            Self::SameRequestId => "same-request-id",
            Self::AfterCondition => "after-condition",
        }
    }

    pub fn from_wire(mode: &str) -> Option<Self> {
        [Self::Never, Self::SameRequestId, Self::AfterCondition]
            .into_iter()
            .find(|m| m.as_str() == mode)
    }
}

impl fmt::Display for RetryMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One failure — code (machine) + message (human) + retry directive. Wire shape = `{"code":"NOT_FOUND","message":"…","retry":"never"}`.
///
/// ★Do not machine-branch on `message`★: the only branching material is `code` (the CLI's string matching existed only because this
///   contract did not; TRD §4-⑦).
///
/// ★Equality is decided by **meaning** — three fields only★: `received` is a relay copy and takes no part, else a value read by
///   downgrading an unknown code and a value built directly with the same meaning
/// would differ, diverging from the contract's sameness (the flat three fields of TRD §4-⑦).
///
/// ★The three fields are private — writes go only through setters★: swapping a field directly on a value holding the raw copy would make
///   reserialization emit the raw text and **silently ignore the written value**
/// (a relay hop that lowers `retry` to cut a dangerous retry would be undone at the next hop — TRD §4-④·§4-⑦). **A written value always
///   goes out as is.**
#[derive(Debug, Clone)]
pub struct CommandError {
    code: ErrorCode,
    message: String,
    retry: RetryMode,
    /// Only what **the three typed fields cannot carry** out of the received raw text. An error built directly is `None`.
    ///
    /// ★Accepting must not be destroying★: reading an unknown code downgraded to `INTERNAL` (§4-⑦) must not **erase the raw text on
    ///   re-emission**, or a relay hop's reserialization hides the owner's code from the final caller.
    ///   Fields outside the contract are carried as is for the same reason.
    ///   ★`null` and absence are normalized **together**★ — one of the three fields arriving as `null` is read as absent and not carried
    ///   out (both mean "no value"; re-decoding gives the same value).
    ///   Fields outside the contract get no such normalization and go out as `null` — their meaning is unknown, so there is no basis to
    ///   fold them.
    /// This field is private, so it cannot be built with a struct literal — use the constructors.
    received: Option<Box<ReceivedError>>,
}

/// The two faces of a received `code` field that do not restore into the typed field — known codes are not stored here.
#[derive(Debug, Clone)]
enum ReceivedCode {
    /// That field was **absent or `null`** — not carried on the way out either (no inventing a code that was not there).
    Absent,
    /// An unknown code string — read downgraded, but carried as received on the way out.
    Unknown(String),
}

/// The raw fragments the decode holds on to — only what cannot be restored into the three fields of [`CommandError`].
#[derive(Debug, Clone)]
struct ReceivedError {
    /// `None` if it was a known code (the typed field carries it as is). The two exceptions are [`ReceivedCode`].
    code: Option<ReceivedCode>,
    /// The received raw `retry`. ★`None` = that field was **absent or `null`**★ — not carried on the way out either.
    retry: Option<String>,
    /// Whether the `message` field was carried (`null` counts as absent). Absence is remembered so as not to invent an empty message.
    message_present: bool,
    /// Fields outside the contract — carried as is so the additive extension (§4-③) does not evaporate at a relay hop.
    extra: serde_json::Map<String, serde_json::Value>,
}

impl PartialEq for CommandError {
    fn eq(&self, other: &Self) -> bool {
        self.code == other.code && self.message == other.message && self.retry == other.retry
    }
}

impl Eq for CommandError {}

impl CommandError {
    /// Derives the retry directive from the code — the form most call sites should use.
    pub fn of(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retry: code.default_retry(),
            received: None,
        }
    }

    /// Use only when a directive different from the derived value must be carried.
    pub fn with_retry(code: ErrorCode, message: impl Into<String>, retry: RetryMode) -> Self {
        Self {
            code,
            message: message.into(),
            retry,
            received: None,
        }
    }

    pub fn code(&self) -> ErrorCode {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn retry(&self) -> RetryMode {
        self.retry
    }

    /// Swaps the code — **discards the raw copy.**
    ///
    /// ★Discarding is the point★: keeping the copy would make reserialization emit the raw text for a value that received an unknown code,
    ///   ignoring the written value; the fields outside the contract and the received raw `retry`
    /// are **accessories of the old code** and would disagree with the new one.
    /// Do not call it on a value that is only relayed — the raw text must reach the final caller.
    pub fn set_code(&mut self, code: ErrorCode) {
        self.code = code;
        self.received = None;
    }

    /// Swaps the retry directive — **discards the raw copy** (same reason as [`CommandError::set_code`]).
    ///
    /// ★Do not use this to only lower the directive while relaying someone else's error★ — use [`CommandError::set_retry_for_relay`]; this
    ///   erases the received code and the fields outside the contract.
    pub fn set_retry(&mut self, retry: RetryMode) {
        self.retry = retry;
        self.received = None;
    }

    /// Swaps only the retry directive and **keeps the raw copy** — ★relay hops only★.
    ///
    /// Where it parts from [`CommandError::set_retry`]: that discards the copy and **erases the received code and the fields outside the
    ///   contract**, right when changing the code but wrong for a hop that **only** lowers the directive
    /// (it breaks [`CommandError::received`]'s "accepting must not be destroying" right at that hop).
    ///
    /// It overwrites only the copy's `retry` field — the source of the outgoing value ([`CommandError::wire_retry`]) — so **the outgoing
    ///   directive really becomes this value** while `code` · message presence · the fields outside the contract
    /// reach the final caller as received.
    /// ★It fills it even if the received error **had no** `retry` field★ — otherwise the field goes out missing and the receiver derives
    ///   it from the code, voiding the lowered directive.
    ///
    /// ★Where it is used★: a hop that must lower the directive by its own policy while **someone else made the answer**
    ///   (`command_delivery::send_reply` of `engram-dashboard-daemon`).
    pub fn set_retry_for_relay(&mut self, retry: RetryMode) {
        self.retry = retry;
        if let Some(received) = self.received.as_deref_mut() {
            received.retry = Some(retry.as_str().to_string());
        }
    }

    /// Swaps the message — leaves the copy as is: reserialization **always reads** the message from this field (no separate raw message),
    ///   so a hop appending context keeps the received code·fields outside the contract.
    pub fn set_message(&mut self, message: impl Into<String>) {
        self.message = message.into();
    }

    /// The code string carried on the way out — **as received** if an unknown code was received, `None` for an error that came **without**
    ///   a `code` field (or `null`; absent on the way out too).
    /// After [`CommandError::set_code`], the typed value goes out.
    pub fn wire_code(&self) -> Option<&str> {
        match self.received.as_deref().map(|r| &r.code) {
            Some(Some(ReceivedCode::Absent)) => None,
            Some(Some(ReceivedCode::Unknown(code))) => Some(code),
            _ => Some(self.code.as_str()),
        }
    }

    /// The retry string carried on the way out — **as received** if an unknown directive was received, `None` for an error that came
    ///   **without** a `retry` field (or `null`; absent on the way out too).
    /// After [`CommandError::set_retry`], the typed value goes out.
    pub fn wire_retry(&self) -> Option<&str> {
        match self.received.as_deref() {
            Some(received) => received.retry.as_deref(),
            None => Some(self.retry.as_str()),
        }
    }

    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::of(ErrorCode::InvalidArgument, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::of(ErrorCode::NotFound, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::of(ErrorCode::Internal, message)
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CommandError {}

#[derive(serde::Serialize)]
struct WireError<'a> {
    code: &'a str,
    message: &'a str,
    retry: &'a str,
}

#[derive(serde::Deserialize)]
struct RawError {
    /// ★Not made required★ — one error without a code (or `null`) failing decode would make the whole [`crate::CommandReply`] unreadable
    ///   and **lose even the correlation key**;
    /// the caller would see the deadline instead of the outcome already sent (TRD §4-⑤·§4-⑥).
    code: Option<String>,
    message: Option<String>,
    retry: Option<String>,
    /// Collects fields outside the contract instead of discarding them — `deny_unknown_fields` is not attached because it breaks additive
    ///   extension.
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

impl serde::Serialize for CommandError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap as _;

        let Some(received) = self.received.as_deref() else {
            return WireError {
                code: self.code.as_str(),
                message: &self.message,
                retry: self.retry.as_str(),
            }
            .serialize(s);
        };

        let mut map = s.serialize_map(None)?;
        if let Some(code) = self.wire_code() {
            map.serialize_entry("code", code)?;
        }
        if received.message_present || !self.message.is_empty() {
            map.serialize_entry("message", &self.message)?;
        }
        if let Some(retry) = &received.retry {
            map.serialize_entry("retry", retry)?;
        }
        for (key, value) in &received.extra {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

impl<'de> serde::Deserialize<'de> for CommandError {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = RawError::deserialize(d)?;
        let known_code = raw.code.as_deref().and_then(ErrorCode::from_wire);
        let known_retry = raw.retry.as_deref().and_then(RetryMode::from_wire);
        // Downgraded reading: an unknown code is INTERNAL + never (§4-⑦), an unknown directive is the code's derived value; a **missing**
        //   code folds here too (not a known code, meaning unknown).
        let code = known_code.unwrap_or(ErrorCode::Internal);
        let retry = match known_code {
            Some(_) => known_retry.unwrap_or_else(|| code.default_retry()),
            None => RetryMode::Never,
        };
        // If the three fields restore what was received as is, no copy is held — holding one needlessly would spread the "do not swap by
        //   hand" rule to common values.
        let expressible = known_code.is_some()
            && known_retry.is_some()
            && raw.message.is_some()
            && raw.extra.is_empty();
        let received = (!expressible).then(|| {
            Box::new(ReceivedError {
                code: match (known_code, raw.code) {
                    (Some(_), _) => None,
                    (None, Some(text)) => Some(ReceivedCode::Unknown(text)),
                    (None, None) => Some(ReceivedCode::Absent),
                },
                message_present: raw.message.is_some(),
                retry: raw.retry,
                extra: raw.extra,
            })
        });
        Ok(Self {
            code,
            message: raw.message.unwrap_or_default(),
            retry,
            received,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CommandReply, RequestId};

    /// ★When the vocabulary grows, outgoing and incoming names must grow together★ — a code that grows on one side only goes out under its
    ///   own name and comes back as `INTERNAL`.
    ///
    /// The table (`error_codes!`) now prevents list mismatch; the remaining risk is **two variants sharing one wire string** — `from_wire`
    ///   returns the first match, so the later line comes back under the earlier name.
    #[test]
    fn every_code_in_the_alphabet_round_trips_through_its_wire_string() {
        for code in ErrorCode::ALL {
            assert_eq!(
                ErrorCode::from_wire(code.as_str()),
                Some(*code),
                "{} does not come back",
                code.as_str()
            );
        }
    }

    /// ★The whole reply must read even without a code★ — a decode failure here breaks [`CommandReply`] entirely and **loses the
    ///   correlation key** (TRD §4-⑤).
    #[test]
    fn a_reply_whose_code_is_null_still_decodes_and_keeps_its_request_id() {
        let id = RequestId::new();
        let wire = format!(
            r#"{{"request_id":"{id}","outcome":{{"Err":{{"code":null,"message":"the owner fell over"}}}}}}"#
        );

        let decoded: CommandReply =
            serde_json::from_str(&wire).expect("decodes even when code is null");

        assert_eq!(decoded.request_id, id, "the correlation key survives");
        let err = decoded.outcome.expect_err("error reply");
        assert_eq!(err.code(), ErrorCode::Internal);
        assert_eq!(err.retry(), RetryMode::Never);
        assert_eq!(
            err.wire_code(),
            None,
            "no inventing a code that was not there"
        );
        assert_eq!(
            serde_json::to_string(&err).expect("reserialize"),
            r#"{"message":"the owner fell over"}"#
        );
    }

    /// A case with no field at all folds the same as `null` — both are 「no value」 (the same axis as the normalization rule below).
    #[test]
    fn an_error_with_no_code_field_decodes_as_internal_and_stays_codeless() {
        let decoded: CommandError =
            serde_json::from_str(r#"{"message":"x","retry":"never"}"#).expect("decode");

        assert_eq!(decoded.code(), ErrorCode::Internal);
        assert_eq!(decoded.retry(), RetryMode::Never);
        assert_eq!(
            serde_json::to_string(&decoded).expect("reserialize"),
            r#"{"message":"x","retry":"never"}"#
        );
    }

    #[test]
    fn unknown_wire_code_degrades_to_internal_never() {
        let e: CommandError =
            serde_json::from_str(r#"{"code":"FROM_THE_FUTURE","message":"x","retry":"whenever"}"#)
                .expect("unknown codes decode too");
        assert_eq!(e.code(), ErrorCode::Internal);
        assert_eq!(e.retry(), RetryMode::Never);
    }

    /// ★Accepting must not be destroying★ — a relay hop erasing an unknown code hides the original from the final caller.
    ///
    /// Byte comparison holds only when the input is already in `code`·`message`·`retry` order — reserialization stamps that order and
    ///   appends fields outside the contract after, so other orders go out as **different bytes, same meaning**.
    #[test]
    fn an_unknown_code_and_its_retry_text_survive_a_relay_hop() {
        let wire = r#"{"code":"FROM_THE_FUTURE","message":"x","retry":"whenever"}"#;
        let decoded: CommandError = serde_json::from_str(wire).expect("decode");
        assert_eq!(serde_json::to_string(&decoded).expect("reserialize"), wire);
        assert_eq!(decoded.wire_code(), Some("FROM_THE_FUTURE"));
    }

    #[test]
    fn an_unknown_retry_alone_keeps_the_known_code_and_the_original_text() {
        let wire = r#"{"code":"TIMEOUT","message":"x","retry":"tomorrow"}"#;
        let decoded: CommandError = serde_json::from_str(wire).expect("decode");
        assert_eq!(decoded.code(), ErrorCode::Timeout);
        assert_eq!(decoded.retry(), RetryMode::SameRequestId);
        assert_eq!(serde_json::to_string(&decoded).expect("reserialize"), wire);
    }

    /// ★`OWNER_UNAVAILABLE` as the example is intended★ — its emitting path is gone (ADR-0150), so this test and
    ///   [`retry_is_derived_from_code`] are the only uses keeping that code in the vocabulary (reason: the variant comment).
    #[test]
    fn known_code_keeps_wire_retry() {
        let e: CommandError = serde_json::from_str(
            r#"{"code":"OWNER_UNAVAILABLE","message":"tab.create","retry":"after-condition"}"#,
        )
        .expect("decode");
        assert_eq!(e.code(), ErrorCode::OwnerUnavailable);
        assert_eq!(e.retry(), RetryMode::AfterCondition);
    }

    #[test]
    fn retry_is_derived_from_code() {
        assert_eq!(
            CommandError::of(ErrorCode::UnknownCommand, "x").retry(),
            RetryMode::Never
        );
        assert_eq!(
            CommandError::of(ErrorCode::OwnerUnavailable, "x").retry(),
            RetryMode::AfterCondition
        );
        assert_eq!(
            CommandError::of(ErrorCode::Timeout, "x").retry(),
            RetryMode::SameRequestId
        );
    }

    #[test]
    fn round_trips_through_wire() {
        let e = CommandError::of(ErrorCode::NotFound, "no agent 'x'");
        let text = serde_json::to_string(&e).expect("serialize");
        assert_eq!(
            text,
            r#"{"code":"NOT_FOUND","message":"no agent 'x'","retry":"never"}"#
        );
        assert_eq!(
            serde_json::from_str::<CommandError>(&text).expect("decode"),
            e
        );
    }

    /// ★No inventing a field that was not there★ — filling `""` for a missing `retry` would hand the next hop a value that is no directive.
    #[test]
    fn an_absent_retry_stays_absent_across_a_relay_hop() {
        let wire = r#"{"code":"TIMEOUT","message":"x"}"#;
        let decoded: CommandError = serde_json::from_str(wire).expect("decode");

        assert_eq!(
            decoded.retry(),
            RetryMode::SameRequestId,
            "derived from the code"
        );
        assert_eq!(decoded.wire_retry(), None);
        assert_eq!(serde_json::to_string(&decoded).expect("reserialize"), wire);
    }

    #[test]
    fn an_absent_message_stays_absent_across_a_relay_hop() {
        let wire = r#"{"code":"TIMEOUT","retry":"same-request-id"}"#;
        let decoded: CommandError = serde_json::from_str(wire).expect("decode");

        assert!(decoded.message().is_empty());
        assert_eq!(serde_json::to_string(&decoded).expect("reserialize"), wire);
    }

    /// If a hop fills in the message, it is carried out — preserving absence is the rule for 「when not filled」.
    #[test]
    fn a_message_added_by_a_hop_is_serialized() {
        let mut decoded: CommandError =
            serde_json::from_str(r#"{"code":"TIMEOUT"}"#).expect("decode");
        decoded.set_message("took too long");

        assert_eq!(
            serde_json::to_string(&decoded).expect("reserialize"),
            r#"{"code":"TIMEOUT","message":"took too long"}"#
        );
    }

    /// ★The received raw text survives even when a message is appended★ — discarding the copy is the rule on the code·retry side.
    #[test]
    fn a_message_written_by_a_hop_keeps_the_relayed_code_and_extras() {
        let mut decoded: CommandError = serde_json::from_str(
            r#"{"code":"FROM_THE_FUTURE","message":"x","retry":"whenever","details":{"agent":"alpha"}}"#,
        )
        .expect("decode");
        decoded.set_message("relayed through the shell");

        assert_eq!(decoded.wire_code(), Some("FROM_THE_FUTURE"));
        assert_eq!(
            serde_json::to_string(&decoded).expect("reserialize"),
            r#"{"code":"FROM_THE_FUTURE","message":"relayed through the shell","retry":"whenever","details":{"agent":"alpha"}}"#
        );
    }

    /// ★A written value goes out as is★ — if a hop's lowered `retry` reverted to the raw text on reserialization, the next hop would keep
    ///   retrying (TRD §4-④).
    #[test]
    fn writing_a_typed_field_is_what_goes_out_even_on_a_relayed_error() {
        let wire =
            r#"{"code":"NOT_FOUND","message":"x","retry":"after-condition","details":{"tab":1}}"#;

        let mut stop: CommandError = serde_json::from_str(wire).expect("decode");
        stop.set_retry(RetryMode::Never);
        assert_eq!(stop.wire_retry(), Some("never"));
        assert_eq!(
            serde_json::to_string(&stop).expect("reserialize"),
            r#"{"code":"NOT_FOUND","message":"x","retry":"never"}"#
        );

        let mut recoded: CommandError =
            serde_json::from_str(r#"{"code":"FROM_THE_FUTURE","message":"x","retry":"whenever"}"#)
                .expect("decode");
        recoded.set_code(ErrorCode::Timeout);
        assert_eq!(recoded.wire_code(), Some("TIMEOUT"));
        assert_eq!(
            serde_json::to_string(&recoded).expect("reserialize"),
            r#"{"code":"TIMEOUT","message":"x","retry":"never"}"#
        );
    }

    /// ★`null` and absence fold together★ — re-decoding the folded form gives the same value; fields outside the contract are not folded
    ///   (meaning unknown).
    #[test]
    fn an_explicit_null_in_a_typed_field_is_canonicalized_to_absence() {
        let decoded: CommandError =
            serde_json::from_str(r#"{"code":"TIMEOUT","message":null,"retry":null,"hint":null}"#)
                .expect("decode");

        assert!(decoded.message().is_empty());
        assert_eq!(
            decoded.retry(),
            RetryMode::SameRequestId,
            "derived from the code"
        );
        assert_eq!(decoded.wire_retry(), None);
        assert_eq!(
            serde_json::to_string(&decoded).expect("reserialize"),
            r#"{"code":"TIMEOUT","hint":null}"#
        );
    }

    /// ★Fields outside the contract are additive extension (§4-③)★ — if a relay hop erases them, the final caller cannot see them.
    #[test]
    fn unknown_extra_fields_survive_a_relay_hop() {
        let wire =
            r#"{"code":"NOT_FOUND","message":"x","retry":"never","details":{"agent":"alpha"}}"#;
        let decoded: CommandError = serde_json::from_str(wire).expect("decode");

        assert_eq!(decoded.code(), ErrorCode::NotFound);
        assert_eq!(serde_json::to_string(&decoded).expect("reserialize"), wire);
    }

    /// ★Sameness is decided by the three fields★ — if a downgraded value and a value built with the same meaning came out different, the
    ///   contract (§4-⑦) and
    /// the code would speak of different sameness.
    #[test]
    fn equality_is_the_three_contract_fields_not_the_relay_copy() {
        let downgraded: CommandError =
            serde_json::from_str(r#"{"code":"FROM_THE_FUTURE","message":"x","retry":"whenever"}"#)
                .expect("decode");
        let built = CommandError::of(ErrorCode::Internal, "x");

        assert_eq!(downgraded, built);
        assert_ne!(
            serde_json::to_string(&downgraded).expect("serialize"),
            serde_json::to_string(&built).expect("serialize"),
            "the outgoing bytes differ even with the same meaning — because it holds the raw text"
        );

        let other_message: CommandError =
            serde_json::from_str(r#"{"code":"NOT_FOUND","message":"y","retry":"never"}"#)
                .expect("decode");
        assert_ne!(other_message, CommandError::not_found("x"));
    }
}
