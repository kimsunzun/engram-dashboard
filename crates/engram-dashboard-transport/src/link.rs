//! Transport seam — what opens links (`Dialer`), what accepts them (`Listener`), the two ends of an open link (`LinkTx`/`LinkRx`) and the opaque handshake over it.
//!
//! ★dyn, not generic★ — different transports (a local WS peer, a later TLS peer) must mix in one [`crate::Registry`], which a generic parameter cannot express; a dyn call per
//! frame is noise next to a syscall (ADR-0177 decision 7). ★No `async fn` in trait★ — not dyn-compatible, so futures are boxed (`BoxFuture`, as the daemon's `conn_port`).

use std::fmt;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use futures_util::future::BoxFuture;

use crate::frame::{Close, CloseCode, Frame};
use crate::mask::mask_reason;

/// The peer's address. ★The crate does not parse it★ — only the [`Dialer`] implementation knows its meaning.
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

/// Error the link itself produced. ★Human-readable text only, no code for machine branching★ — vocabularies differ per transport; the distinctions that need a branch are
/// [`crate::ConnectFailure`] (could not connect / connected but rejected) and [`DialFailure`] (at the dialing place).
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

/// What was read from the link. ★[`LinkRead::Closed`] carrying the close code is the heart of the contract★ — without it the receiver cannot tell 「the peer rejected me」 from 「the peer
/// vanished」, and ADR-0180 decision 5 (a terminal rejection does not spend reconnect budget) cannot hold in code; the code ([`RejectKind::of`]) is the material for that decision.
pub enum LinkRead {
    Frame(Frame),
    /// The peer **said it closed**: `Some` = code and message, `None` = a close without a code.
    ///
    /// ★A peer that vanished without a word never arrives here★ — a real transport yields `Err(LinkError)` (WS: `ResetWithoutClosingHandshake`), and ★the in-memory harness does the
    /// same★ (`testing::MemRx` errors on channel exhaustion; when only it raised `Closed(None)`, death-by-drop tests were green only in the harness — do not revert). Still, do not
    /// read `Closed(None)` as 「quietly died」 — it is 「said it would close without a code」. ★After [`LinkRx::recv`] has failed once this branch does not come out★ (see that rustdoc).
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

/// A failure at the dialing place — ★split by whether it spends reconnect budget★ (TRD 3 §3-0 ⑤). A [`LinkError`] raised with `?` becomes [`DialFailure::Unreachable`] (`From`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialFailure {
    /// The link did not open → spend budget and dial again.
    Unreachable(LinkError),
    /// ★The dialer already knows the peer would not accept★ (e.g. the local daemon info's version differs) → takes the 「connected but rejected」 path without a link: lands on
    /// [`crate::GaveUpReason::Rejected`] without budget, no redial. ★Always read as [`RejectKind::Terminal`]★ — for a retryable failure return [`DialFailure::Unreachable`].
    /// `code` = the rejection code the dialer chose, carried as is in [`crate::TransportEvent::Rejected`] (the consumer picks a fixed message from it; the kind is not re-decided).
    /// ★It must be a terminal code★ ([`CloseCode::HANDSHAKE_REJECTED`] = 4001): a retryable one (e.g. [`CloseCode::HANDSHAKE_RETRYABLE`]) still gives up, so event and behavior
    /// disagree (debug asserts). `reason` = local diagnostic, masked like other rejection messages ([`crate::ConnectFailure::Rejected`]).
    Refused { code: CloseCode, reason: String },
}

impl From<LinkError> for DialFailure {
    fn from(err: LinkError) -> Self {
        Self::Unreachable(err)
    }
}

/// The text field is printed masked by [`crate::mask_reason`] — a consumer dialer may carry peer-given text, and many places print this value into logs as is.
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

/// Opens a link with one address. ★The caller (the supervisor) sets the deadline★ — an implementation setting its own would put the policy value in two places. ★The returned future may be
/// cancelled★ (the supervisor drops it on `close()`): create the link **inside the future**, or an unused link is left behind.
pub trait Dialer: Send + Sync + 'static {
    fn dial(
        &self,
        addr: &Address,
    ) -> BoxFuture<'_, Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), DialFailure>>;
}

/// The link's outgoing side — ★only one supervisor task holds it (single writer)★. ★The link closes only when **both** ends are dropped★ — releasing one while holding the other hides the
/// disconnect from the peer (a socket leak). [`Dialer::dial`] gives the ends as a pair and they drop as a pair; implementations usually share one stream (WS: `BiLock`), so **the caller bears this duty.**
pub trait LinkTx: Send + 'static {
    /// ★After the returned future is cancelled, the caller does at most one [`LinkTx::close`] to this end★, then drops both ends and never calls `send` · `ping` again (operating phase:
    /// `Supervisor::drop_link` sends the close; during the handshake it drops without one — `handshake_once` / `shake_hands`). So implementations must tolerate **a `close` after a
    /// cancelled `send`**, and the cut frame's remainder may go out before it (WS does — `src/ws.rs` 「link properties」).
    fn send(&mut self, frame: Frame) -> BoxFuture<'_, Result<(), LinkError>>;

    /// Asks whether the peer is alive. ★This crate decides "when", the implementation "how"★ (WS uses its own Ping, no new frame). ★Cancellation as [`LinkTx::send`]★ — at most one `close` follows.
    fn ping(&mut self) -> BoxFuture<'_, Result<(), LinkError>>;

    /// Closes carrying a code and message; no return value (nowhere to report a failure). ★[`Close::reason`] is bounded and truncated on overflow★ — the cap is in **bytes**, not
    /// characters (Hangul · emoji fill it 3×+ faster), and its number is canonical in the adapter (WS: `MAX_CLOSE_REASON_BYTES` in `src/ws.rs`, RFC 6455 §5.5). Truncating beats not:
    /// the receiver cuts an oversize close frame with a read error, so 「connected but rejected」 would arrive as 「could not connect」 and burn budget. Branch only on [`Close::code`].
    /// ★An implementation may wait for delivery before closing (iroh waits for the peer to acknowledge the tail) and need not bound that wait★ — callers bound it with their own deadline.
    fn close(&mut self, close: Close) -> BoxFuture<'_, ()>;

    /// The TCP ends of this link as the transport observed them when it opened the link — `None` = not a TCP link or not known (the
    ///   in-memory harness, iroh). ★A fact, not a decision★: a consumer that must know who is at the other end before it sends anything
    ///   (the shell's account check, ADR-0395) reads it between [`Dialer::dial`] and its handshake.
    fn socket_ends(&self) -> Option<SocketEnds> {
        None
    }
}

/// The two TCP ends of one link — `local` = this process's socket, `peer` = the other end. Typed so a consumer can look the connection
///   up (e.g. the account that owns the other end) without parsing [`LinkInfo::remote`], which is diagnostics text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SocketEnds {
    pub local: SocketAddr,
    pub peer: SocketAddr,
}

/// The link's incoming side — ★a pair with [`LinkTx`]: the link closes only when both ends are dropped★.
pub trait LinkRx: Send + 'static {
    /// A keepalive response comes up as [`Frame::Keepalive`]. ★A link that has produced `Err` once keeps producing `Err`★ — never [`LinkRead::Closed`] after a failure (transports
    /// usually signal exhaustion with their own end-of-stream marker; WS rustdoc has the example). A "no more reads after failure" ban would give no signal when broken and would make 「the peer
    /// said it closed」 false; with this contract [`LinkRead::Closed`] means 「the peer said so」 **unconditionally**.
    fn recv(&mut self) -> BoxFuture<'_, Result<LinkRead, LinkError>>;
}

// ── Receiving side ───────────────────────────────────────────────────────────

/// Accepts incoming links — the counterpart of [`Dialer`] (TRD 3 §3-1 ① · D1). ★The consumer runs the accept loop★ — when to call `accept` and what to attach (connection cap · rate) is
/// consumer policy (ADR-0177 decision 4); the registry holds no listener. ★One receiver (`&mut self`)★. ★Dropping the listener closes links not yet taken★ (the dialer sees link loss;
/// otherwise an untaken socket would keep its upgrade permit).
pub trait Listener: Send + 'static {
    /// Waits for the next accepted link. ★Cancel-safe★ — a dropped future does not lose the link (usable on a `select!` arm against shutdown). ★`Err` is terminal★ — the listener can accept
    /// no more and keeps returning `Err` (as [`LinkRx::recv`]); ★one peer's failure (upgrade failure · permit overflow · deadline) is not `Err`★ — the listener closes that link and waits
    /// on, else one peer would end the accept loop. ★Such rejections are reported per implementation outside this trait★ (TRD 3 §5 18; `ws`: the `AcceptRefusals` returned by
    /// `WsListener::bind`).
    fn accept(&mut self) -> BoxFuture<'_, Result<Accepted, LinkError>>;
}

/// Facts attached to an accepted link, read by the consumer when building that link's handshake value (TRD 3 §3-1 ①). ★`None` = this transport does not know it★ (an in-memory link has
/// no remote address; Origin exists only in a WS upgrade). ★`#[non_exhaustive]`★ — later transports can add fields; outside the crate build it from [`LinkInfo::default`].
/// `Debug` masks `remote` · `origin` ([`crate::mask_reason`]) and prints only the length of `peer_key`.
#[derive(Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct LinkInfo {
    /// The peer's end as seen from the accepting side, for diagnostics · logs. ★Not an [`Address`]★ (a per-connection temporary port, not dialable). ★Transport-built — never put peer text in it★.
    pub remote: Option<String>,
    /// The WS upgrade's `Origin` header. ★Peer-made text★ — the decision is the accept policy's (for `ws`, a value that passed `AcceptPolicy::allowed_origins`); not printed as is.
    pub origin: Option<String>,
    /// The peer's public key confirmed by the transport (TLS key pinning · iroh node id — TRD 3 §3-5 · §4). `ws` and the in-memory harness leave it `None`; the iroh
    /// adapter fills the 32-byte node key its TLS handshake authenticated.
    pub peer_key: Option<Vec<u8>>,
    /// The TCP ends of the accepted connection (`peer` = the observed client address) — the typed accessor for a consumer that looks the
    ///   connection up, e.g. the account owning the client process (ADR-0395). `ws` fills it; iroh and the in-memory harness leave it `None`.
    pub socket_ends: Option<SocketEnds>,
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
            .field("socket_ends", &self.socket_ends)
            .finish()
    }
}

/// One accepted link from [`Listener::accept`] — both ends · [`LinkInfo`] · upgrade permit · upgrade end time.
///
/// ★The ends never leave the crate★ — only [`crate::Registry::attach`] uses them; the one exception is the test scaffold `testing::accepted_ends` behind `test-support`. If dropped, the two
/// ends drop together. ★The permit is an upgrade-wait permit★ — taken at link receipt (TCP accept), released after the receiving side takes the next (auth-wait) permit (TRD 3 §3-1 · §5 14),
/// so links waiting in the ready queue count too. The auth-wait permit does not exist yet, so the attached peer holds this one until the operating phase
/// ([`crate::Registry::attach_with_policy`]). ★The auth deadline runs from `upgraded_at`★ (TRD 3 §5 3), ready-queue time included; the stamping clock and the measuring clock must be the
/// same, so the transport gets the registry's [`crate::Clock`].
pub struct Accepted {
    pub(crate) tx: Box<dyn LinkTx>,
    pub(crate) rx: Box<dyn LinkRx>,
    pub(crate) info: LinkInfo,
    pub(crate) upgraded_at: Instant,
    pub(crate) permit: ConnPermit,
}

impl Accepted {
    /// Built by the transport. `tx` · `rx` are the pair of one link; `upgraded_at` = when the link became able to carry frames (WS: after the upgrade response).
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

/// Cap on simultaneously held [`ConnPermit`]s (TRD 3 §5 14); a clone is the same bundle. ★Counted by held handles, not events★ (TRD 3 §3-0 ②) — tied to the counted thing's lifetime, the permit
/// returns on every path (error · panic · cancel); event counting leaks on any path that misses one.
#[derive(Clone)]
pub struct PermitPool {
    count: Arc<PermitCount>,
}

struct PermitCount {
    limit: usize,
    in_use: AtomicUsize,
}

impl PermitPool {
    /// `limit` = 0 hands out no permit.
    pub fn new(limit: usize) -> Self {
        Self {
            count: Arc::new(PermitCount {
                limit,
                in_use: AtomicUsize::new(0),
            }),
        }
    }

    /// One permit; `None` = full (never waits; on overflow the new one is closed — TRD 3 §5 14).
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

#[must_use = "the permit returns the moment it is dropped — keep it tied to the lifetime of what is counted"]
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

/// Opaque handshake. ★The crate does not count round trips★ — any number of [`HandshakeStep::Send`]/[`HandshakeStep::Await`]. ★Direction-symmetric★: on the receiving side `start()` gives
/// [`HandshakeStep::Await`] and `on_frame` decides. Version · capability negotiation lives **only inside the implementation** (ADR-0178).
pub trait Handshake: Send + Sync + 'static {
    fn start(&self) -> HandshakeStep;
    fn on_frame(&self, frame: &Frame) -> HandshakeStep;
}

/// Kind of rejection — ★decides the reconnect budget★; crosses the link as a close code (TRD 3 §3-0 ⑤ · D20 ①). The rejecting side's [`HandshakeStep::Reject`] picks a kind and the crate
/// closes with its code ([`RejectKind::close_code`]); the dialing side decides from the received code ([`RejectKind::of`]) — ★never from peer-given text★ (human diagnostic, changes freely).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectKind {
    /// Redialing gives the same result (e.g. version mismatch) → [`crate::GaveUpReason::Rejected`] without budget (ADR-0180 decision 5). Code = [`CloseCode::HANDSHAKE_REJECTED`].
    Terminal,
    /// Redialing may connect (e.g. a stale credential read from rewritten daemon info) → spends budget and redials. Code = [`CloseCode::HANDSHAKE_RETRYABLE`].
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

    /// Reads the code the peer carried when it closed during the handshake; `None` = not a rejection (could not connect — spends budget). ★The normal-exit family is not a rejection★ —
    /// `1000` · `1001` and [`CloseCode::GOING_AWAY`] mean 「I am leaving」; read as rejections the dialer would give up on every clean daemon exit. `0` is QUIC's close
    /// without an application code (ADR-0388) — the iroh adapter already reads it as [`LinkRead::Closed`]`(None)`, and it is no rejection here either. ★Only
    /// [`CloseCode::HANDSHAKE_REJECTED`] is [`RejectKind::Terminal`]★; other codes are deliberate peer rejections but giving up on an unknown code would persist until user action, so they redial (spend budget).
    pub fn of(code: CloseCode) -> Option<Self> {
        if matches!(code.0, 0 | 1000 | 1001) || code == CloseCode::GOING_AWAY {
            return None;
        }
        Some(if code == CloseCode::HANDSHAKE_REJECTED {
            RejectKind::Terminal
        } else {
            RejectKind::Retryable
        })
    }
}

/// Next step of the handshake. ★[`HandshakeStep::Send`] = send and **wait for the peer's next frame**★ — the trait only has `start`/`on_frame`, so a **handshake sending two frames in a row
/// cannot be expressed** (the TRD §3-5 two-step example is: `start() = Send(auth)` → peer Hello → `on_frame = Done`). ★[`Frame::Keepalive`] is not delivered to `on_frame`★ (it would make decisions wobble).
pub enum HandshakeStep {
    /// Sends and waits for the peer's next frame.
    Send(Frame),
    /// Sends nothing and waits for the peer's next frame.
    ///
    /// To let the peer's close code decide a refusal's kind, answer the peer's refusal frame with this and wait for the close ([`RejectKind::of`] · TRD 3 §3-0 ⑤); a close without a code
    /// reads as "could not connect". A consumer whose peer sends no code may answer with [`HandshakeStep::Reject`] (the shell does — ADR-0346). The frame's text is for diagnostics only.
    Await,
    /// On to the operating phase.
    Done,
    /// ★Connected but rejected★ — sends `last` and closes with `kind`'s code ([`RejectKind::close_code`]); if `last` cannot go out within the handshake deadline the close is skipped and only the link is
    /// discarded (crate header 「close code reaches the peer in few places」). For a peer we dialed, `kind` decides: [`RejectKind::Terminal`] gives up without budget (ADR-0180 decision 5),
    /// [`RejectKind::Retryable`] spends budget and redials. ★`reason` is bounded★ — it is the close text, truncated at the transport cap (★bytes★; see [`LinkTx::close`]); put long diagnostics in
    /// the consumer's log.
    Reject {
        kind: RejectKind,
        last: Option<Frame>,
        reason: String,
    },
}

/// The frame carried prints only through [`Frame`]'s `Debug` (branch + byte length) — `Send` carries the auth token (TRD 3 §5 5). `reason` is the one line sent to the peer as close text, printed as is
/// (bounded only on the wire — ws `clamp_close_reason`).
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
        for graceful in [
            CloseCode(0),
            CloseCode(1000),
            CloseCode(1001),
            CloseCode::GOING_AWAY,
        ] {
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
        let a = pool.try_take().expect("first permit");
        let b = pool.try_take().expect("second permit");
        assert!(pool.try_take().is_none(), "exceeded the cap");
        assert_eq!(pool.in_use(), 2);
        drop(a);
        assert_eq!(pool.in_use(), 1);
        let c = pool
            .clone()
            .try_take()
            .expect("permit returned — a clone is the same bundle");
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
            socket_ends: Some(SocketEnds {
                local: "127.0.0.1:80".parse().unwrap(),
                peer: "127.0.0.1:5".parse().unwrap(),
            }),
        };
        assert_eq!(
            format!("{info:?}"),
            r#"LinkInfo { remote: Some("127.0.0.1:5[2J"), origin: Some("http://evilx"), peer_key: Some("32 bytes"), socket_ends: Some(SocketEnds { local: 127.0.0.1:80, peer: 127.0.0.1:5 }) }"#
        );
        assert_eq!(
            format!("{:?}", LinkInfo::default()),
            "LinkInfo { remote: None, origin: None, peer_key: None, socket_ends: None }"
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
