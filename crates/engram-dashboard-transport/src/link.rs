//! Transport seam — what opens links (`Dialer`) · what accepts them (`Listener`), the two ends of an open link (`LinkTx`/`LinkRx`), and the
//! opaque handshake that runs over it.
//!
//! ★Why dyn and not generic★ — **different transports must be able to be mixed** in one registry (a local WS peer and a later
//! TLS peer go into the same [`crate::Registry`]). A generic parameter cannot express that collection.
//! A frame is already a `String`/`Vec<u8>` value, so a dyn call per frame is noise next to a syscall. (ADR-0177 decision 7)
//!
//! ★Why `async fn` in trait is not used★ — it is not dyn-compatible. So it is boxed manually with `BoxFuture`
//! (the daemon's `conn_port` in the same repo has the same shape for the same reason).

use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use futures_util::future::BoxFuture;

use crate::frame::{Close, CloseCode, Frame};
use crate::mask::mask_reason;

/// The peer's address. ★The crate does not parse this string★ — only the [`Dialer`] implementation knows what it means.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Address(pub String);

impl Address {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Error the link itself produced. ★It is human-readable text only and has no code for machine branching★ — the error vocabulary differs
/// per transport, so fixing branches here would make adapters force their own reasons into ours. The distinctions that need a branch
/// (could not connect / connected but rejected) are carried by [`crate::ConnectFailure`], and the distinction at the dialing place by [`DialFailure`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkError {
    pub message: String,
}

impl LinkError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for LinkError {}

/// What was read up from the link.
///
/// ★The heart of the contract is that [`LinkRead::Closed`] carries the close code★ — without this field the receiver cannot tell 「상대가 나를
/// 거절했다」 (the peer rejected me) from 「상대가 그냥 사라졌다」 (the peer just vanished), and then ADR-0180 decision 5 (a terminal rejection does not spend the reconnect
/// budget) **does not hold in code**. A terminal rejection gives the same result on retry so it must not burn budget, and
/// the material for that decision — is it a rejection, and of which kind ([`RejectKind::of`]) — is this code.
pub enum LinkRead {
    Frame(Frame),
    /// The peer **said it closed**. If `Some`, it carries the code and message; if `None`, just the close without a code.
    ///
    /// ★A peer that vanished without a word does not arrive as this branch★ — a real transport produces it as `Err(LinkError)`
    /// (the WS adapter is the example: a socket discarded without a closing handshake is produced by tungstenite as a
    /// `ResetWithoutClosingHandshake` error). ★The in-memory harness bears the same shape too★ — if an endpoint is
    /// simply dropped, `testing::MemRx` produces channel exhaustion as an error. Once only that side raised `Closed(None)`, and
    /// **tests that mimic death by dropping an endpoint were green only in the harness** — do not revert this.
    /// Even so, do not branch on `Closed(None)` as a signal of 「조용히 죽었다」 (quietly died) — it is 「코드 없이 닫겠다고 **말했다**」 (**said** it would close without a code),
    /// and that differs from vanishing without a word.
    ///
    /// ★After [`LinkRx::recv`] has failed once, this branch does not come out★ — without that contract the sentence above
    /// would become the conditional 「첫 읽기에서만」 (only on the first read) (the reason is in that rustdoc).
    Closed(Option<Close>),
}

impl fmt::Debug for LinkRead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Frame(frame) => f.debug_tuple("Frame").field(frame).finish(),
            Self::Closed(close) => f.debug_tuple("Closed").field(close).finish(),
        }
    }
}

/// A failure that occurred at the dialing place — ★it splits by whether it spends reconnect budget or not★ (TRD 3 §3-0 ⑤).
///
/// If a [`LinkError`] is raised as is with `?`, it becomes [`DialFailure::Unreachable`] (`From`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialFailure {
    /// The link did not open → spend budget and dial again.
    Unreachable(LinkError),
    /// ★The dialer already knows the peer would not accept even if it dialed★ (e.g. the version in the local daemon info differs) → goes by the same path as
    /// 「붙었는데 거절」 (connected but rejected) without a link — it lands on [`crate::GaveUpReason::Rejected`] without budget and does not dial again. ★It is always
    /// read as [`RejectKind::Terminal`]★ — if the failure is one you want retried, return [`DialFailure::Unreachable`].
    ///
    /// `code` = the rejection code the dialer chose — it is carried as is in [`crate::TransportEvent::Rejected`] and becomes the material from which the consumer
    /// picks a fixed message (the kind is not re-decided from that value). ★It must be a terminal rejection code★ — the code that [`RejectKind::of`]
    /// reads as [`RejectKind::Terminal`] ([`CloseCode::HANDSHAKE_REJECTED`] = 4001). Even if a code that would redial (e.g.
    /// [`CloseCode::HANDSHAKE_RETRYABLE`]) is carried, this branch gives up, so the event's code and the behavior disagree (a debug
    /// build stops on an assert). `reason` = local diagnostic message — it goes through the same masking as other rejection messages
    /// ([`crate::ConnectFailure::Rejected`]).
    Refused { code: CloseCode, reason: String },
}

impl From<LinkError> for DialFailure {
    fn from(err: LinkError) -> Self {
        Self::Unreachable(err)
    }
}

/// The text field is printed masked by [`crate::mask_reason`] — a consumer dialer may carry text the peer gave, and there are many places that print this value
/// into the log as is.
impl fmt::Display for DialFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreachable(err) => write!(f, "unreachable: {}", mask_reason(&err.message)),
            Self::Refused { code, reason } => {
                write!(f, "refused ({}): {}", code.0, mask_reason(reason))
            }
        }
    }
}

impl std::error::Error for DialFailure {}

/// Opens a link with one address.
///
/// ★The caller (the supervisor task) sets the deadline★ — if the implementation sets its own deadline again, the policy value lives in two places.
///
/// ★The returned future may be cancelled★ — if the supervisor hears `close()` in the meantime it drops it as is. The implementation must create the link
/// **inside the future**; if it registers it outside, a link that nobody uses is left behind.
pub trait Dialer: Send + Sync + 'static {
    fn dial(
        &self,
        addr: &Address,
    ) -> BoxFuture<'_, Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), DialFailure>>;
}

/// The link's outgoing side. ★Only one supervisor task holds this (single writer)★.
///
/// ★The link closes only when **both** ends are dropped★ — if one end is let go and the other held, the peer cannot
/// see the disconnect, and that is a socket leak. The two ends must be given as a pair by [`Dialer::dial`] and dropped as a pair. Since implementations
/// usually share one underlying stream (the WS adapter uses a `BiLock`), **the caller bears this duty.**
pub trait LinkTx: Send + 'static {
    /// ★After the returned future is cancelled, the caller does at most one [`LinkTx::close`] to this end★ —
    /// after that it drops both ends and does not call `send` · `ping` again. In the operating phase `Supervisor::drop_link`
    /// sends the close. During the handshake it drops without a close — when the deadline cuts it (including the frame just before a rejection)
    /// `handshake_once` drops the handshake future as a whole, and if control wins, `shake_hands` does. So
    /// the implementation must tolerate **a `close` after a cancelled `send`**, and the remainder of the cut frame may go out
    /// before that close (the WS adapter does — `src/ws.rs` 「통로 성질」 (link properties)).
    fn send(&mut self, frame: Frame) -> BoxFuture<'_, Result<(), LinkError>>;

    /// Asks whether it is alive. ★This crate decides "when" and only the implementation knows "how"★ — so where the
    /// transport layer has its own Ping, as WS does, there is no need to build a new frame.
    ///
    /// ★Cancellation has the same contract as [`LinkTx::send`]★ — after being cut, all this end receives is at most one `close`.
    fn ping(&mut self) -> BoxFuture<'_, Result<(), LinkError>>;

    /// Closes carrying a code and message. Even on failure there is nowhere to report, so there is no return value.
    ///
    /// ★[`Close::reason`] is bounded and goes out truncated if it overflows★ — if the transport carries it in a control frame,
    /// there is a cap. ★The unit of the cap is bytes, not characters★, so Hangul and emoji fill it three times or more as fast.
    /// ★That **number** is not in this file★ — it differs per transport, and copying it here would make two canonical sources. For WS the value is
    /// the adapter's `MAX_CLOSE_REASON_BYTES` (`src/ws.rs`) and that constant's rustdoc gives the basis (RFC 6455 §5.5).
    /// The reason truncating beats not truncating is that **the receiving side cuts a close frame over the cap with a read
    /// error**, so 「붙었는데 거절」 (connected but rejected) arrives as 「못 붙었다」 (could not connect) and burns reconnect budget too.
    /// Do not put decision material in the message — branch only on [`Close::code`].
    fn close(&mut self, close: Close) -> BoxFuture<'_, ()>;
}

/// The link's incoming side.
///
/// ★It is a pair with [`LinkTx`] — the link closes only when both ends are dropped★ (that rustdoc gives the reason).
pub trait LinkRx: Send + 'static {
    /// A keepalive response comes up as [`Frame::Keepalive`].
    ///
    /// ★A link that has produced `Err` once keeps producing `Err`★ — after a failure the implementation does **not** produce
    /// [`LinkRead::Closed`]. The reason this, rather than forbidding further reads after a failure, was made the contract is that a ban
    /// gives no signal when not kept, and breaking it makes 「상대가 닫았다고 말했다」 (the peer said it closed) false (the transport
    /// layer usually signals exhaustion with its own end-of-stream marker — the WS adapter rustdoc gives the actual example).
    /// So on this contract [`LinkRead::Closed`] means 「상대가 말했다」 (the peer said so) **unconditionally**.
    fn recv(&mut self) -> BoxFuture<'_, Result<LinkRead, LinkError>>;
}

// ── Receiving side ───────────────────────────────────────────────────────────

/// Accepts incoming links — the counterpart of [`Dialer`] (TRD 3 §3-1 ① · D1).
///
/// ★The consumer runs the accept loop★ — when to call `accept` and whether to attach what was accepted (connection cap · rate) is consumer policy
/// (ADR-0177 decision 4). The registry does not hold the listener.
///
/// ★There is one receiver (`&mut self`)★ — there is no shape in which two tasks split the accepting of one listener.
///
/// ★If the listener is dropped, links not yet taken are closed★ — the dialer sees that as link loss. If they were held on to, a socket nobody
/// would take would remain holding an upgrade permit.
pub trait Listener: Send + 'static {
    /// Waits for the next accepted link.
    ///
    /// ★Cancel-safe★ — even if the returned future is dropped before finishing, the accepted link is not lost and the next `accept` gives it
    /// (it may be hung on a `select!` arm to race against a shutdown signal).
    ///
    /// ★`Err` is terminal★ — the listener can accept no more. A listener that has produced `Err` once keeps producing `Err`
    /// (the same contract as [`LinkRx::recv`]). ★One peer's failure (upgrade failure · permit overflow · deadline) is not `Err`★ — the
    /// listener closes that link itself and waits for the next. If it were raised as `Err`, one peer would end the accept loop.
    ///
    /// ★Such rejections are reported per implementation outside this trait★ (TRD 3 §5 18) — for `ws`, `WsListener::bind` returns the refusal
    /// receiver (`AcceptRefusals`) together with the listener. The `Ok` of `accept` is only the one accepted link ([`Accepted`]) and carries no rejection branch.
    fn accept(&mut self) -> BoxFuture<'_, Result<Accepted, LinkError>>;
}

/// Facts attached to an accepted link — the consumer reads them when building that link's handshake value (TRD 3 §3-1 ①).
///
/// ★`None` = this transport does not know it★ — an in-memory link has no remote address and Origin exists only in a WS upgrade.
/// ★`#[non_exhaustive]`★ — consumers do not break when a later transport adds fields. Outside the crate, build it by filling in
/// fields starting from [`LinkInfo::default`].
///
/// `Debug` masks `remote` · `origin` with [`crate::mask_reason`] and prints only the length of `peer_key`.
#[derive(Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct LinkInfo {
    /// The peer's end as seen from the accepting side — material for diagnostics · logs. ★It is not an [`Address`]★ — it is something like a temporary port that changes with every connection,
    /// so it is not an address you can dial.
    /// ★It is a value the transport built — do not put the peer's text in it★.
    pub remote: Option<String>,
    /// The `Origin` header value of the WS upgrade request. ★It is text the peer made up and sent★ — the decision is the accept policy's job (for `ws` it is
    /// a value that passed `AcceptPolicy::allowed_origins`), and it is not printed as is.
    pub origin: Option<String>,
    /// The peer's public key confirmed by the transport (TLS key pinning · iroh node id — TRD 3 §3-5 · §4). ★A stage-3 transport is always `None`★ — only the field
    /// is laid down early.
    pub peer_key: Option<Vec<u8>>,
}

impl fmt::Debug for LinkInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LinkInfo")
            .field("remote", &self.remote.as_deref().map(mask_reason))
            .field("origin", &self.origin.as_deref().map(mask_reason))
            .field(
                "peer_key",
                &self.peer_key.as_ref().map(|k| format!("{} bytes", k.len())),
            )
            .finish()
    }
}

/// One accepted link that [`Listener::accept`] hands out — both ends · [`LinkInfo`] · upgrade permit · upgrade end time.
///
/// ★The two ends are not taken out of the crate★ — only the place that puts it on as an accepted peer ([`crate::Registry::attach`]) uses them. The one exception is a test
/// scaffold — `testing::accepted_ends` behind the feature `test-support` hands both ends to a real-socket test, and a production build has
/// no such path. If dropped, the two ends drop together (the 「양 끝이 둘 다」 (both ends) duty of [`LinkTx`] holds here by itself).
///
/// ★The permit is an upgrade-wait permit★ — the transport takes it the moment it receives the link (TCP accept) and releases it only after the receiving side takes the next permit (auth wait)
/// (TRD 3 §3-1 · §5 14). So links sitting in the ready queue while the consumer is not calling `accept` are counted too. The auth-wait
/// permit does not exist yet, so the peer put on holds this permit until it enters the operating phase ([`crate::Registry::attach_with_policy`]).
///
/// ★The auth deadline is measured from `upgraded_at`★ (TRD 3 §5 3) — time sitting in the ready queue counts toward the deadline. The clock that stamped that moment and
/// the clock that measures the deadline must be the same, so the transport implementation is given the same [`crate::Clock`] as the registry.
pub struct Accepted {
    pub(crate) tx: Box<dyn LinkTx>,
    pub(crate) rx: Box<dyn LinkRx>,
    pub(crate) info: LinkInfo,
    pub(crate) upgraded_at: Instant,
    pub(crate) permit: ConnPermit,
}

impl Accepted {
    /// Built by the transport implementation. `tx` · `rx` are a pair of the same link. `upgraded_at` = the moment the link became able to carry frames
    /// (for WS, after sending the upgrade response).
    pub fn new(
        tx: Box<dyn LinkTx>,
        rx: Box<dyn LinkRx>,
        info: LinkInfo,
        upgraded_at: Instant,
        permit: ConnPermit,
    ) -> Self {
        Self {
            tx,
            rx,
            info,
            upgraded_at,
            permit,
        }
    }

    pub fn info(&self) -> &LinkInfo {
        &self.info
    }

    pub fn upgraded_at(&self) -> Instant {
        self.upgraded_at
    }
}

impl fmt::Debug for Accepted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Accepted")
            .field("info", &self.info)
            .field("upgraded_at", &self.upgraded_at)
            .field("permit", &self.permit)
            .finish_non_exhaustive()
    }
}

/// Cap on the number of [`ConnPermit`]s that can be held at once — the count of the connection cap (TRD 3 §5 14). A clone is the same bundle.
///
/// ★Counted not by events but by held handles★ (TRD 3 §3-0 ②) — if a [`ConnPermit`] is tied to the lifetime of what is counted, then by whichever path it
/// ends (including error · panic · cancel), the permit returns when it drops. Counting by events leaks the count on every path that missed one event.
#[derive(Clone)]
pub struct PermitPool {
    count: Arc<PermitCount>,
}

struct PermitCount {
    limit: usize,
    in_use: AtomicUsize,
}

impl PermitPool {
    /// If `limit` = 0, it hands out no permit at all.
    pub fn new(limit: usize) -> Self {
        Self {
            count: Arc::new(PermitCount {
                limit,
                in_use: AtomicUsize::new(0),
            }),
        }
    }

    /// One permit. `None` = full — it does not wait (on overflow the new one is closed — TRD 3 §5 14).
    pub fn try_take(&self) -> Option<ConnPermit> {
        let limit = self.count.limit;
        self.count
            .in_use
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < limit).then_some(n + 1)
            })
            .ok()
            .map(|_| ConnPermit {
                count: self.count.clone(),
            })
    }

    pub fn in_use(&self) -> usize {
        self.count.in_use.load(Ordering::Acquire)
    }

    pub fn limit(&self) -> usize {
        self.count.limit
    }
}

impl fmt::Debug for PermitPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PermitPool")
            .field("in_use", &self.in_use())
            .field("limit", &self.limit())
            .finish()
    }
}

/// One permit of a [`PermitPool`] — it returns when dropped.
#[must_use = "떨어뜨리는 순간 칸이 돌아간다 — 세는 대상의 수명에 묶어 둔다"]
pub struct ConnPermit {
    count: Arc<PermitCount>,
}

impl Drop for ConnPermit {
    fn drop(&mut self) {
        self.count.in_use.fetch_sub(1, Ordering::AcqRel);
    }
}

impl fmt::Debug for ConnPermit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnPermit")
            .field("limit", &self.count.limit)
            .finish_non_exhaustive()
    }
}

/// Opaque handshake. ★The crate does not count round trips★ — it accepts however many times [`HandshakeStep::Send`]/[`HandshakeStep::Await`]
/// are returned.
///
/// ★Direction-symmetric★ — on the receiving side (the daemon), `start()` gives [`HandshakeStep::Await`] and it decides in `on_frame`.
/// So the vocabulary of version · capability negotiation lives **only inside the implementation** and the crate does not know it (ADR-0178).
pub trait Handshake: Send + Sync + 'static {
    fn start(&self) -> HandshakeStep;
    fn on_frame(&self, frame: &Frame) -> HandshakeStep;
}

/// Kind of rejection — ★it decides the reconnect budget★. Over the link it crosses as a close code (TRD 3 §3-0 ⑤ · D20 ①).
///
/// When the rejecting side's [`HandshakeStep::Reject`] picks a kind, the crate carries that code ([`RejectKind::close_code`]) and
/// closes, and the dialing side decides the kind from the code it received ([`RejectKind::of`]). ★It does not decide from text the peer gave★ — the text is
/// a human diagnostic and changes without notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectKind {
    /// Dialing again gives the same result (e.g. a version mismatch) → lands on [`crate::GaveUpReason::Rejected`] without budget (ADR-0180
    /// decision 5). Code = [`CloseCode::HANDSHAKE_REJECTED`].
    Terminal,
    /// Dialing again may connect (e.g. a stale credential was read from daemon info rewritten in place) → like a failure to connect, it spends budget and
    /// dials again. Code = [`CloseCode::HANDSHAKE_RETRYABLE`].
    Retryable,
}

impl RejectKind {
    /// The close code to carry when rejecting with this kind.
    pub fn close_code(self) -> CloseCode {
        match self {
            RejectKind::Terminal => CloseCode::HANDSHAKE_REJECTED,
            RejectKind::Retryable => CloseCode::HANDSHAKE_RETRYABLE,
        }
    }

    /// Reads the code the peer carried when it closed during the handshake. `None` = not a rejection (could not connect — spends budget).
    ///
    /// ★The normal-exit family is not a rejection★ — RFC 6455's `1000` (Normal) · `1001` (Going Away) and
    /// [`CloseCode::GOING_AWAY`] mean 「나 이제 간다」 (I am leaving now). Read as a rejection, every time the daemon exits cleanly the dialer
    /// gives up without a single retry. ★Only [`CloseCode::HANDSHAKE_REJECTED`] is [`RejectKind::Terminal`]★ — the rest
    /// were deliberately chosen and sent by the peer so they are taken as rejections, but giving up on an unknown code would not clear until the user steps in, so it
    /// dials again (spends budget — it does not dial endlessly).
    pub fn of(code: CloseCode) -> Option<Self> {
        if matches!(code.0, 1000 | 1001) || code == CloseCode::GOING_AWAY {
            return None;
        }
        Some(if code == CloseCode::HANDSHAKE_REJECTED {
            RejectKind::Terminal
        } else {
            RejectKind::Retryable
        })
    }
}

/// Next step of the handshake.
///
/// ★[`HandshakeStep::Send`] means "send and **wait for the peer's next frame**"★ — the trait has no entrance that asks for the next step without a frame
/// (only `start`/`on_frame`), so after sending there is nowhere to go but waiting. So
/// **a handshake that sends two frames in a row cannot be expressed in this shape** (the two-step example TRD §3-5 gave is
/// expressed: `start() = Send(auth)` → peer Hello → `on_frame = Done`).
///
/// ★[`Frame::Keepalive`] is not delivered to `on_frame`★ — if transport-layer keepalive got
/// mixed in during the handshake, the decision would wobble.
pub enum HandshakeStep {
    /// Sends and waits for the peer's next frame.
    Send(Frame),
    /// Sends nothing and waits for the peer's next frame.
    ///
    /// To let the peer's close code decide the kind of a refusal, answer the peer's refusal frame (its error text)
    /// with this and wait for the close — the kind then comes from that code ([`RejectKind::of`] · TRD 3 §3-0 ⑤); a
    /// close without a code reads as "could not connect". A consumer whose peer never sends a code may instead
    /// answer that frame with [`HandshakeStep::Reject`] (the shell does — ADR-0346). The frame's text is for the
    /// consumer's diagnostics and logs only.
    Await,
    /// On to the operating phase.
    Done,
    /// ★Connected but rejected★ — sends `last` and closes with the code of `kind` ([`RejectKind::close_code`]). But if `last` cannot
    /// go out within the handshake deadline, the close is skipped and only the link is discarded (crate header 「닫기 코드가 상대에게 닿는 자리는
    /// 좁다」 (the place where close codes reach the peer is narrow)). If this rejecting side is a peer we dialed, it goes by `kind` — [`RejectKind::Terminal`] gives up without budget (ADR-0180
    /// decision 5) · [`RejectKind::Retryable`] spends budget and dials again.
    ///
    /// ★`reason` is bounded★ — since it goes out as the text of the close frame, it hits the transport's cap and is **truncated** on overflow
    /// (★bytes, not characters★ — the number is canonical in the adapter constant). Details and the reason are in
    /// [`LinkTx::close`]. Send long diagnostics to the consumer-side log and put here only the one line the peer will read.
    Reject {
        kind: RejectKind,
        last: Option<Frame>,
        reason: String,
    },
}

/// The frame carried is printed only through the `Debug` of [`Frame`] (branch + byte length) — `Send` carries the auth token (TRD 3 §5 5).
/// `reason` is the one line that goes out to the peer as the close text, so it is printed as is — its length is bounded only when it goes out on the wire (ws adapter `clamp_close_reason`).
impl fmt::Debug for HandshakeStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Send(frame) => f.debug_tuple("Send").field(frame).finish(),
            Self::Await => f.write_str("Await"),
            Self::Done => f.write_str("Done"),
            Self::Reject { kind, last, reason } => f
                .debug_struct("Reject")
                .field("kind", kind)
                .field("last", last)
                .field("reason", reason)
                .finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::CloseCode;

    #[test]
    fn address_is_opaque() {
        let a = Address::new("ws://127.0.0.1:1/path?x=1");
        assert_eq!(a.as_str(), "ws://127.0.0.1:1/path?x=1");
        assert_eq!(a.to_string(), "ws://127.0.0.1:1/path?x=1");
    }

    #[test]
    fn link_error_displays_its_message() {
        let e = LinkError::new("refused");
        assert_eq!(e.to_string(), "refused");
    }

    #[test]
    fn a_close_can_carry_a_code_the_reader_can_branch_on() {
        let spoken = LinkRead::Closed(Some(Close::new(
            CloseCode::HANDSHAKE_REJECTED,
            "version 3 != 4",
        )));
        let silent = LinkRead::Closed(None);
        match (&spoken, &silent) {
            (LinkRead::Closed(Some(c)), LinkRead::Closed(None)) => {
                assert_eq!(c.code, CloseCode::HANDSHAKE_REJECTED);
                assert!(c.reason.contains("version"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn handshake_step_and_link_read_debug_carry_no_frame_body() {
        let secret = r#"{"auth":"tok-SECRET"}"#;
        let n = secret.len();
        let shown = [
            (
                format!("{:?}", HandshakeStep::Send(Frame::Text(secret.into()))),
                format!("Send(Text {{ bytes: {n} }})"),
            ),
            (
                format!(
                    "{:?}",
                    HandshakeStep::Send(Frame::Binary(secret.as_bytes().to_vec()))
                ),
                format!("Send(Binary {{ bytes: {n} }})"),
            ),
            (
                format!(
                    "{:?}",
                    HandshakeStep::Reject {
                        kind: RejectKind::Retryable,
                        last: Some(Frame::Text(secret.into())),
                        reason: "stale credential".into(),
                    }
                ),
                format!(
                    "Reject {{ kind: Retryable, last: Some(Text {{ bytes: {n} }}), reason: \"stale credential\" }}"
                ),
            ),
            (
                format!("{:?}", LinkRead::Frame(Frame::Text(secret.into()))),
                format!("Frame(Text {{ bytes: {n} }})"),
            ),
        ];
        for (got, expected) in shown {
            assert!(!got.contains("SECRET"), "{got}");
            assert_eq!(got, expected);
        }
    }

    #[test]
    fn handshake_step_debug_names_each_arm() {
        assert_eq!(format!("{:?}", HandshakeStep::Await), "Await");
        assert_eq!(format!("{:?}", HandshakeStep::Done), "Done");
        assert!(format!("{:?}", HandshakeStep::Send(Frame::Keepalive)).starts_with("Send"));
        assert!(format!(
            "{:?}",
            HandshakeStep::Reject {
                kind: RejectKind::Terminal,
                last: None,
                reason: "bad version".into()
            }
        )
        .contains("bad version"));
    }

    #[test]
    fn each_reject_kind_travels_as_its_own_close_code_and_comes_back_as_itself() {
        for kind in [RejectKind::Terminal, RejectKind::Retryable] {
            assert_eq!(RejectKind::of(kind.close_code()), Some(kind), "{kind:?}");
        }
        assert_ne!(
            RejectKind::Terminal.close_code(),
            RejectKind::Retryable.close_code()
        );
    }

    #[test]
    fn only_the_terminal_code_gives_up_and_a_graceful_close_is_no_rejection() {
        for graceful in [CloseCode(1000), CloseCode(1001), CloseCode::GOING_AWAY] {
            assert_eq!(RejectKind::of(graceful), None, "{graceful:?}");
        }
        assert_eq!(
            RejectKind::of(CloseCode::HANDSHAKE_REJECTED),
            Some(RejectKind::Terminal)
        );
        // Unknown codes · other codes of the crate · the consumer range dial again.
        for other in [
            CloseCode(1008),
            CloseCode::KEEPALIVE_TIMEOUT,
            CloseCode::PROTOCOL_ERROR,
            CloseCode(CloseCode::CONSUMER_BASE),
            CloseCode(4999),
        ] {
            assert_eq!(
                RejectKind::of(other),
                Some(RejectKind::Retryable),
                "{other:?}"
            );
        }
    }

    #[test]
    fn a_dial_failure_displays_its_text_masked() {
        let hostile = "a\r\n\u{2028}b\u{202e}c\u{e0041}d\u{200b}";
        assert_eq!(
            DialFailure::Unreachable(LinkError::new(hostile)).to_string(),
            "unreachable: abcd"
        );
        assert_eq!(
            DialFailure::Refused {
                code: CloseCode::HANDSHAKE_REJECTED,
                reason: hostile.into(),
            }
            .to_string(),
            "refused (4001): abcd"
        );
    }

    // ── Receiving-side types ──
    #[test]
    fn a_permit_pool_never_hands_out_more_than_its_limit_and_takes_back_on_drop() {
        let pool = PermitPool::new(2);
        let a = pool.try_take().expect("첫 칸");
        let b = pool.try_take().expect("둘째 칸");
        assert!(pool.try_take().is_none(), "상한을 넘겼다");
        assert_eq!(pool.in_use(), 2);
        drop(a);
        assert_eq!(pool.in_use(), 1);
        let c = pool
            .clone()
            .try_take()
            .expect("돌아온 칸 — clone 은 같은 묶음이다");
        assert!(pool.try_take().is_none());
        drop((b, c));
        assert_eq!(pool.in_use(), 0);
        assert!(PermitPool::new(0).try_take().is_none());
    }

    #[test]
    fn link_info_debug_masks_the_texts_and_shows_only_the_key_length() {
        let info = LinkInfo {
            remote: Some("127.0.0.1:5\u{1b}[2J".into()),
            origin: Some("http://evil\r\n\u{202e}x".into()),
            peer_key: Some(vec![7; 32]),
        };
        assert_eq!(
            format!("{info:?}"),
            r#"LinkInfo { remote: Some("127.0.0.1:5[2J"), origin: Some("http://evilx"), peer_key: Some("32 bytes") }"#
        );
        assert_eq!(
            format!("{:?}", LinkInfo::default()),
            "LinkInfo { remote: None, origin: None, peer_key: None }"
        );
    }

    #[test]
    fn accepted_debug_carries_the_facts_but_no_link() {
        let end = crate::testing::memory_link().0;
        let permit = PermitPool::new(1).try_take().unwrap();
        let info = LinkInfo {
            origin: Some("o".into()),
            ..LinkInfo::default()
        };
        let accepted = Accepted::new(end.tx, end.rx, info.clone(), Instant::now(), permit);
        assert_eq!(accepted.info(), &info);
        let shown = format!("{accepted:?}");
        assert!(shown.starts_with("Accepted { info: LinkInfo {"), "{shown}");
        assert!(
            shown.ends_with("permit: ConnPermit { limit: 1, .. }, .. }"),
            "{shown}"
        );
    }

    #[test]
    fn a_link_error_raised_with_question_mark_is_unreachable() {
        fn dial() -> Result<(), DialFailure> {
            Err(LinkError::new("refused"))?;
            Ok(())
        }
        assert_eq!(
            dial(),
            Err(DialFailure::Unreachable(LinkError::new("refused")))
        );
    }
}
