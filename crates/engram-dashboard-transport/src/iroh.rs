//! Real-socket iroh adapter — behind feature `iroh` (ADR-0388). The daemon-to-daemon link: QUIC over iroh, peers authenticated by their 32-byte node key.
//!
//! Owns one iroh endpoint ([`IrohEndpoint`], built from plain values in [`IrohConfig`]), its dialing side ([`IrohDialer`]), its accepting side
//! ([`IrohListener`] with [`IrohAcceptPolicy`]) and the framing of one link onto one QUIC stream. ★iroh types never leave this file★ — consumers see plain
//! values, raw key bytes and the crate's link seams, so the adapter stays replaceable and a consumer manifest needs no iroh dependency (ADR-0388 decision 1).
//! The key's text form belongs to the consumer: this file takes and returns raw bytes, and [`dial_address`] is an adapter-private encoding no one else parses.
//!
//! ## The link (ADR-0388 decision 2)
//!
//! - One QUIC connection per link and one long-lived bi-stream on it, opened by the dialer, which writes the one-byte [`PREAMBLE`] at once (QUIC shows the
//!   peer a stream only after its first byte). Separate QUIC streams are not ordered with each other, so everything rides that one stream.
//! - Frame = kind byte (text · binary · keepalive) + u32 big-endian payload length + payload. The payload cap is the consumer's ([`IrohConfig::max_payload`],
//!   both directions). Inbound framing that breaks the format — unknown kind, length over the cap, a keepalive with a payload, text that is not UTF-8, the
//!   stream ending inside a frame, the stream reset, the stream ending at a frame boundary with no connection close within [`FIN_CLOSE_GRACE`], a wrong
//!   preamble or a stream that ends before it — closes the connection with [`CloseCode::PROTOCOL_ERROR`].
//! - ★A keepalive frame is one-way★ — nothing answers it. A supervisor silence cut over this link therefore needs both ends pinging with an `idle_timeout`
//!   above the peer's `ping_interval`; the peer link turns the supervisor keepalive off on both ends instead.
//! - ★[`LinkTx::send`] of a frame over the cap fails as a value★ (as `ws`), never a panic and never a write.
//! - One ALPN per endpoint ([`IrohConfig::alpn`]), used for both directions; a dialer offering another is refused by the QUIC handshake itself.
//! - Each side lets its peer open one bidirectional stream and no unidirectional one (QUIC stream credit), so a peer cannot park streams on us.
//!
//! ## Close mapping (ADR-0388 decision 5)
//!
//! [`CloseCode`] ↔ the QUIC application error code; the reason is cut to [`MAX_CLOSE_REASON_BYTES`]. ★[`LinkTx::close`] delivers what was written first★: it
//! ends the stream (FIN), waits until the peer has acknowledged every byte, then sends the close — QUIC stops sending stream data at `CONNECTION_CLOSE`, so
//! closing at once would drop an unsent or unacknowledged tail (a [`crate::Peer::close_after_flush`] burst, a handshake `Reject { last }`). The wait is bounded
//! by the caller's deadline (the supervisor wraps every close in one); if that cuts it, the close still goes out. After a send that was cut mid-frame it closes
//! at once — a FIN there would end the stream inside a frame. The reading side treats a FIN at a frame boundary as "the peer is closing" and waits for its
//! close. On the reading side: a peer's application close with
//! code 0 → [`LinkRead::Closed`]`(None)` — the code a QUIC stack sends when the last handle drops and when an endpoint closes, so "going away", never a refusal;
//! any other code → `Closed(Some(..))`; ★every locally detected loss — idle-out of every path, a transport error, our own close — is `Err(LinkError)`, never
//! `Closed`★ (`link.rs`: `Closed` means the peer said so). ★A write into a connection the peer closed with a code returns `Ok`★: the code is the information,
//! and it reaches the supervisor through the read side — a write error there would turn a terminal refusal into "could not connect" (`handshake_once` maps a
//! write error to unreachable). Cost: until the reader reports the close, frames written meanwhile vanish uncounted — and while the consumer is not draining its inbox (the read side held back), that window lasts until it drains. ★The link closes only when both ends are dropped★ ([`LinkTx`] contract): the ends share the connection, and the last one to go
//! closes it with code 0 before its stream is released, so a peer never sees our stream end without the close.
//!
//! ## Accepting (ADR-0388 decision 4)
//!
//! One task takes QUIC connections; each runs on its own task: QUIC + TLS handshake under [`IrohAcceptPolicy::handshake_timeout`] (ALPN and the peer key are
//! checked there; missed → the half-open connection is dropped) → the consumer's
//! [`IrohAcceptPolicy::accept_if`] with the authenticated key, ★before any stream is accepted★ — `false` closes with [`CloseCode::HANDSHAKE_REJECTED`] (the dialer
//! reads it as terminal through [`crate::RejectKind::of`]) → an accept permit ([`IrohAcceptPolicy::max_pending`]; none free → closed with
//! [`CloseCode::GOING_AWAY`], not a refusal) → bi-stream accept and the preamble under [`IrohAcceptPolicy::stream_accept_timeout`] (missed, wrong, or the
//! stream ended or reset first → closed with [`CloseCode::PROTOCOL_ERROR`], permit released) → the ready queue that [`Listener::accept`] reads, so `accept` stays cancel-safe. Neither a refused nor a
//! silent dialer holds a permit past that timeout. [`LinkInfo::peer_key`] = the authenticated 32-byte key; [`LinkInfo::remote`] = the observed address
//! (diagnostics only).
//!
//! ## Dialing
//!
//! [`IrohDialer`] reads the target from [`dial_address`]: the remote key and optional direct socket addresses; iroh verifies the key during the TLS handshake,
//! so a different node at that address fails the dial. ★No 0-RTT★ (ADR-0388 decision 3): `Connecting::into_0rtt` is never called — 0-RTT data is replayable.
//! A peer that closes with [`CloseCode::HANDSHAKE_REJECTED`] while the dial is still connecting gives [`DialFailure::Refused`] with that code, the same terminal
//! outcome the handshake would read.
//!
//! ## Guarantees / left to the consumer
//!
//! Guaranteed: the [`Frame`] round trip, close code and reason in both directions, the accept gate before any stream, raw-key identity. Left to the consumer:
//! the key's text form and storage, which keys to accept (`accept_if`), the accept loop and registry ([`Listener::accept`] → [`crate::Registry::attach`]),
//! reconnect and deadlines ([`crate::Policy`]; the dialer applies no deadline of its own), and whether to keep the supervisor's own ping / silence cut
//! ([`crate::Policy::supervisor_keepalive`] — the peer link turns it off and relies on QUIC liveness, ADR-0388 decision 6).
//!
//! ## Known limits (do not read as guarantees)
//!
//! - ★Refusals are not reported★ — unlike `ws` there is no refusal queue: a failed QUIC handshake, a full permit pool, a missed stream-accept deadline and a
//!   bad preamble only close the connection. A consumer that wants a log of refused keys logs from its `accept_if`.
//! - ★The tail before a close is delivered only within the caller's close deadline★ — past it the close goes out and unacknowledged data is lost; the peer
//!   then reads the close code without that tail.
//! - ★Half-open handshakes are not counted★ — each holds a task (no permit) until [`IrohAcceptPolicy::handshake_timeout`]; their number is not capped here.
//! - ★Drop the [`IrohListener`] only together with the endpoint★ — without it, new incoming connections are neither accepted nor refused, and their dialers
//!   wait out their own connect deadline.
//! - ★[`IrohEndpoint::close`] frees the UDP port only when every clone is gone★ — the dialers held by registries and the listener's accept task each keep one;
//!   a restart on the same fixed port must drop them first. ★A dropped endpoint without `close` also stalls our own held links★ (nothing drives them).
//! - ★mDNS ([`IrohConfig::mdns`]) and the n0 / custom helper servers ([`Helper`]) are wired but not exercised by this crate's tests★ — they contact the LAN or
//!   the internet, and the suites make no network contact.
//! - ★The accepting side does not refuse 0-RTT data from a foreign dialer★ — only our dialer's behaviour is fixed.
//!
//! ## Link properties (facts of the outside world)
//!
//! - ★Liveness★: iroh 1.4.1 sends a keepalive every 5 s and abandons a path idle for 15 s (30 s over a relay) (`iroh` `socket.rs` · `endpoint/quic.rs`); the
//!   connection idle timeout is [`QUIC_IDLE_TIMEOUT`], set here. ★Measured on loopback with one direct path★ (`tests/iroh_link.rs`, 2026-11-20): a peer that
//!   stops answering ends the link 30.0 s after it went silent — the connection idle timeout, not the 15 s path idle. Over a relay or several paths it is
//!   unmeasured.
//! - ★A close reason is cut by bytes, not characters★, first here at [`MAX_CLOSE_REASON_BYTES`] on a character boundary, then by noq to the room left in the
//!   packet (`noq-proto` `ApplicationClose::encode`); the reading side decodes it lossily.
//! - ★A write stalls under QUIC flow control★ when the peer stops reading; `Policy::write_deadline` cuts it.
//! - ★[`LinkRx::recv`] cancel safety is not load-bearing★ — every canceller drops both ends right after (`peer.rs` `drop_link` · `handshake_once` ·
//!   `shake_hands`), as for `ws`.

// ADR-0388
use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ::iroh::address_lookup::{DnsAddressLookup, PkarrPublisher, PkarrResolver};
use ::iroh::endpoint::{
    presets, BindOpts, Builder, ConnectError, ConnectingError, Connection, ConnectionError,
    IdleTimeout, Incoming, IncomingAddr, QuicTransportConfig, ReadError, ReadExactError,
    RecvStream, SendStream, VarInt, WriteError,
};
use ::iroh::{Endpoint, EndpointAddr, PublicKey, RelayMode, RelayUrl, SecretKey};
use futures_util::future::BoxFuture;
use iroh_mdns_address_lookup::MdnsAddressLookup;
use tokio::sync::mpsc;
use tokio::task::{JoinHandle, JoinSet};

use crate::clock::{with_deadline, Clock, Elapsed};
use crate::frame::{Close, CloseCode, Frame};
use crate::link::{
    Accepted, Address, DialFailure, Dialer, LinkError, LinkInfo, LinkRead, LinkRx, LinkTx,
    Listener, PermitPool,
};
use crate::mask::mask_reason;

/// The first byte the dialer writes on the link's stream — the framing version below. Anything else closes with [`CloseCode::PROTOCOL_ERROR`].
pub const PREAMBLE: u8 = 1;

const KIND_TEXT: u8 = 1;
const KIND_BINARY: u8 = 2;
const KIND_KEEPALIVE: u8 = 3;

/// Kind byte + u32 big-endian length.
const HEADER_LEN: usize = 5;

/// Bytes a close reason may carry — ★bytes, not characters★. noq cuts the reason to the room left in the closing packet (`noq-proto`
/// `ApplicationClose::encode`): about 1.1 KiB at QUIC's 1200-byte minimum datagram, less beside ACK frames, and that cut may split a character. 512 stays
/// well inside it, so the reason normally arrives whole.
pub const MAX_CLOSE_REASON_BYTES: usize = 512;

const ADDRESS_PREFIX: &str = "iroh:";

/// The QUIC connection idle timeout — how long a silent peer keeps a link. ★Set explicitly★: it is noq's default today, but noq is not pinned by
/// `iroh =1.4.1`, and the peer link's loss detection is this value (measured 30.0 s — file header 「link properties」).
pub const QUIC_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a reader waits for the connection close after the peer's FIN at a frame boundary. The closing side sends the close as soon as our ACK of its last
/// byte reaches it ([`LinkTx::close`]), so the gap is about one round trip; past this the stream end without a close is a protocol error.
pub const FIN_CLOSE_GRACE: Duration = Duration::from_secs(5);

/// A helper server choice — relay or address lookup ([`IrohConfig`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Helper {
    /// n0's public servers (iroh's defaults).
    Default,
    /// None — nothing is contacted for this role.
    Disabled,
    /// The consumer's own server at this URL.
    Url(String),
}

/// Everything the endpoint is built from — plain values only.
#[derive(Clone, PartialEq, Eq)]
pub struct IrohConfig {
    /// The node's 32-byte secret key; its public half is the node id ([`IrohEndpoint::id`]).
    pub secret_key: [u8; 32],
    /// The one ALPN this endpoint accepts and dials with (e.g. `engram/peer/1`). Must not be empty.
    pub alpn: Vec<u8>,
    /// UDP port on every interface (IPv4 required, IPv6 when available); 0 = the OS picks one per family.
    pub listen_port: u16,
    /// Bind 127.0.0.1 only (IPv4) instead of every interface — for in-process suites: no other machine can reach it and Windows raises no firewall prompt.
    pub loopback_only: bool,
    /// Relay servers. With [`Helper::Disabled`] only direct paths are tried.
    pub relay: Helper,
    /// Where this node publishes its addresses and looks peers up by id (n0 = pkarr + DNS; a URL = a pkarr relay).
    pub address_lookup: Helper,
    /// Announce and discover nodes on the LAN by mDNS (`iroh-mdns-address-lookup`). ★It announces this node's id to the whole LAN★.
    pub mdns: bool,
    /// Byte cap of one frame payload, both directions — the receive cap and the point where an outgoing frame becomes an error. Both sides must use the same
    /// value. At most `u32::MAX`.
    pub max_payload: usize,
}

/// `secret_key` is never printed.
impl fmt::Debug for IrohConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IrohConfig")
            .field("alpn", &String::from_utf8_lossy(&self.alpn))
            .field("listen_port", &self.listen_port)
            .field("loopback_only", &self.loopback_only)
            .field("relay", &self.relay)
            .field("address_lookup", &self.address_lookup)
            .field("mdns", &self.mdns)
            .field("max_payload", &self.max_payload)
            .finish_non_exhaustive()
    }
}

/// The key gate of [`IrohAcceptPolicy::accept_if`] — the peer's authenticated 32-byte key in, accept or refuse out.
pub type AcceptIf = dyn Fn(&[u8; 32]) -> bool + Send + Sync;

/// The accepting side's policy. ★No `Default`★ — like `ws::AcceptPolicy`, every value is the consumer's choice.
#[derive(Clone)]
pub struct IrohAcceptPolicy {
    /// Called once per authenticated connection with the peer's 32-byte key, before any stream is accepted; `false` closes it with
    /// [`CloseCode::HANDSHAKE_REJECTED`]. ★Runs on each incoming connection's own gate task — keep it quick and non-blocking★ (a lookup in the consumer's own list).
    pub accept_if: Arc<AcceptIf>,
    /// From the first packet to the end of the QUIC + TLS handshake; past it the half-open connection is dropped. ★QUIC has no handshake deadline of its own★ —
    /// every packet of a dialer that never finishes resets the idle timer.
    pub handshake_timeout: Duration,
    /// From an accepted key to the link's stream and preamble; past it the connection is closed and its permit released. ★The auth deadline starts where this
    /// ends ([`Accepted::upgraded_at`]) — they add up★.
    pub stream_accept_timeout: Duration,
    /// Accept permits: taken after `accept_if`, released when the registered peer reaches the operating phase (or the [`Accepted`] is dropped). The ready queue
    /// counts against them. All used = the new connection is closed with [`CloseCode::GOING_AWAY`]. 0 = accept nothing.
    pub max_pending: usize,
}

impl fmt::Debug for IrohAcceptPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IrohAcceptPolicy")
            .field("handshake_timeout", &self.handshake_timeout)
            .field("stream_accept_timeout", &self.stream_accept_timeout)
            .field("max_pending", &self.max_pending)
            .finish_non_exhaustive()
    }
}

/// The address [`IrohDialer`] reads: the remote's 32-byte key and optional direct socket addresses. ★An adapter-private encoding★ — build it here, never
/// parse or store it; the key's text form is the consumer's.
pub fn dial_address(peer_key: &[u8; 32], direct: &[SocketAddr]) -> Address {
    let mut text = String::with_capacity(ADDRESS_PREFIX.len() + 64);
    text.push_str(ADDRESS_PREFIX);
    for byte in peer_key {
        text.push_str(&format!("{byte:02x}"));
    }
    if !direct.is_empty() {
        let addrs: Vec<String> = direct.iter().map(SocketAddr::to_string).collect();
        text.push('@');
        text.push_str(&addrs.join(","));
    }
    Address::new(text)
}

/// Reads [`dial_address`]'s encoding back.
fn parse_dial_address(addr: &Address) -> Result<([u8; 32], Vec<SocketAddr>), LinkError> {
    let bad = || {
        LinkError::new(format!(
            "iroh address {:?}: not a dial_address",
            addr.as_str()
        ))
    };
    let rest = addr.as_str().strip_prefix(ADDRESS_PREFIX).ok_or_else(bad)?;
    let (hex, direct) = match rest.split_once('@') {
        Some((hex, direct)) => (hex, Some(direct)),
        None => (rest, None),
    };
    if hex.len() != 64 || !hex.is_ascii() {
        return Err(bad());
    }
    let mut key = [0u8; 32];
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| bad())?;
    }
    let addrs = match direct {
        Some(list) => list
            .split(',')
            .map(|one| SocketAddr::from_str(one).map_err(|_| bad()))
            .collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
    };
    Ok((key, addrs))
}

/// One iroh endpoint — one UDP port and one node key for both directions. Cheap to clone. ★Call [`IrohEndpoint::close`] before dropping the last clone★ —
/// it sends every open connection a close (code 0, read as going away); a dropped endpoint aborts without one and peers notice only at their idle-out.
#[derive(Clone)]
pub struct IrohEndpoint {
    inner: Arc<Shared>,
}

struct Shared {
    endpoint: Endpoint,
    alpn: Vec<u8>,
    max_payload: usize,
}

impl IrohEndpoint {
    /// Builds and binds the endpoint and starts accepting; returns it with its only listener. ★`clock` must be the registry's clock★ — it stamps
    /// [`Accepted::upgraded_at`] (where the auth deadline starts) and drives the stream-accept deadline. Call inside a tokio runtime.
    pub async fn bind(
        config: IrohConfig,
        accept: IrohAcceptPolicy,
        clock: Arc<dyn Clock>,
    ) -> Result<(Self, IrohListener), LinkError> {
        if config.alpn.is_empty() {
            return Err(LinkError::new("iroh config: the ALPN is empty"));
        }
        if u32::try_from(config.max_payload).is_err() {
            return Err(LinkError::new(format!(
                "iroh config: max_payload {} does not fit the u32 frame length",
                config.max_payload
            )));
        }
        let endpoint = builder(&config)?
            .bind()
            .await
            .map_err(|e| LinkError::new(format!("iroh bind: {}", chain(&e))))?;
        let shared = Arc::new(Shared {
            endpoint: endpoint.clone(),
            alpn: config.alpn,
            max_payload: config.max_payload,
        });
        let permits = PermitPool::new(accept.max_pending);
        // ★The ready queue never fills★ — queued length ≤ permits held ≤ this capacity.
        let (ready_tx, ready) = mpsc::channel(accept.max_pending.max(1));
        let gate = Arc::new(Gate {
            policy: accept,
            permits: permits.clone(),
            clock,
            max_payload: config.max_payload,
        });
        let driver = tokio::spawn(drive(endpoint, gate, ready_tx));
        Ok((
            Self { inner: shared },
            IrohListener {
                ready,
                permits,
                driver,
            },
        ))
    }

    /// This node's 32-byte public key — the id a peer dials and accepts.
    pub fn id(&self) -> [u8; 32] {
        *self.inner.endpoint.id().as_bytes()
    }

    /// The bound UDP sockets (with their real ports — a 0 port in the config is replaced by the OS choice).
    pub fn bound_addrs(&self) -> Vec<SocketAddr> {
        self.inner.endpoint.bound_sockets()
    }

    pub fn dialer(&self) -> IrohDialer {
        IrohDialer {
            inner: self.inner.clone(),
        }
    }

    /// Closes every connection (code 0) and the endpoint; the listener then returns `Err`. Waits briefly for the closes to be acknowledged.
    pub async fn close(&self) {
        self.inner.endpoint.close().await;
    }
}

impl fmt::Debug for IrohEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IrohEndpoint")
            .field("bound", &self.bound_addrs())
            .finish_non_exhaustive()
    }
}

/// The endpoint builder for `config` — presets per ADR-0388: `N0` only when relay and lookup are both n0's; otherwise `Minimal` plus the chosen pieces.
fn builder(config: &IrohConfig) -> Result<Builder, LinkError> {
    let mut builder = if config.relay == Helper::Default && config.address_lookup == Helper::Default
    {
        Endpoint::builder(presets::N0)
    } else {
        let mut builder = Endpoint::builder(presets::Minimal);
        builder = match &config.relay {
            Helper::Default => builder.relay_mode(RelayMode::Default),
            Helper::Disabled => builder.relay_mode(RelayMode::Disabled),
            Helper::Url(url) => {
                let url = RelayUrl::from_str(url).map_err(|e| {
                    LinkError::new(format!("iroh relay URL {url:?}: {}", chain(&e)))
                })?;
                builder.relay_mode(RelayMode::custom([url]))
            }
        };
        match &config.address_lookup {
            Helper::Default => builder
                .address_lookup(PkarrPublisher::n0_dns())
                .address_lookup(PkarrResolver::n0_dns())
                .address_lookup(DnsAddressLookup::n0_dns()),
            Helper::Disabled => builder,
            Helper::Url(url) => {
                let url = url::Url::parse(url)
                    .map_err(|e| LinkError::new(format!("iroh lookup URL {url:?}: {e}")))?;
                builder
                    .address_lookup(PkarrPublisher::builder(url.clone()))
                    .address_lookup(PkarrResolver::builder(url))
            }
        }
    };
    if config.mdns {
        builder = builder.address_lookup(MdnsAddressLookup::builder());
    }
    let ipv4 = if config.loopback_only {
        Ipv4Addr::LOCALHOST
    } else {
        Ipv4Addr::UNSPECIFIED
    };
    builder = builder
        .clear_ip_transports()
        .bind_addr(SocketAddr::from((ipv4, config.listen_port)))
        .map_err(|e| LinkError::new(format!("iroh bind address: {}", chain(&e))))?;
    if !config.loopback_only {
        builder = builder
            .bind_addr_with_opts(
                SocketAddr::from((Ipv6Addr::UNSPECIFIED, config.listen_port)),
                BindOpts::default().set_is_required(false),
            )
            .map_err(|e| LinkError::new(format!("iroh bind address: {}", chain(&e))))?;
    }
    // One bi-stream per link, opened by the dialer: the peer may open one bi and no uni stream toward us.
    let transport = QuicTransportConfig::builder()
        .max_idle_timeout(Some(
            IdleTimeout::try_from(QUIC_IDLE_TIMEOUT).expect("30 s fits a QUIC varint"),
        ))
        .max_concurrent_bidi_streams(VarInt::from_u32(1))
        .max_concurrent_uni_streams(VarInt::from_u32(0))
        .build();
    Ok(builder
        .secret_key(SecretKey::from_bytes(&config.secret_key))
        .alpns(vec![config.alpn.clone()])
        .transport_config(transport))
}

/// An error with its source chain — iroh's errors print only their top line on `Display`.
fn chain(err: &(dyn std::error::Error + 'static)) -> String {
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(next) = source {
        text.push_str(": ");
        text.push_str(&next.to_string());
        source = next.source();
    }
    text
}

fn close_code(code: CloseCode) -> VarInt {
    VarInt::from_u32(u32::from(code.0))
}

/// Cut to [`MAX_CLOSE_REASON_BYTES`] at a character boundary.
fn clamp_close_reason(reason: &str) -> &str {
    if reason.len() <= MAX_CLOSE_REASON_BYTES {
        return reason;
    }
    &reason[..reason.floor_char_boundary(MAX_CLOSE_REASON_BYTES)]
}

/// What a peer's application close reads as ([`LinkRead`] contract): 0 = a close without a code; a code beyond `u16` is not one of ours.
fn peer_close(code: u64, reason: &[u8]) -> Result<LinkRead, LinkError> {
    match code {
        0 => Ok(LinkRead::Closed(None)),
        code => match u16::try_from(code) {
            Ok(code) => Ok(LinkRead::Closed(Some(Close::new(
                CloseCode(code),
                String::from_utf8_lossy(reason).into_owned(),
            )))),
            Err(_) => Err(LinkError::new(format!(
                "iroh read: the peer closed with code {code}, outside the close-code range"
            ))),
        },
    }
}

// ── Dialing side ──────────────────────────────────────────────────────────────

/// A [`Dialer`] over the endpoint's QUIC connections; [`Address`] = [`dial_address`]. ★Failures are [`DialFailure::Unreachable`]★, except a peer close with
/// [`CloseCode::HANDSHAKE_REJECTED`] that lands while connecting ([`DialFailure::Refused`], see the file header).
#[derive(Clone)]
pub struct IrohDialer {
    inner: Arc<Shared>,
}

impl fmt::Debug for IrohDialer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IrohDialer").finish_non_exhaustive()
    }
}

impl Dialer for IrohDialer {
    fn dial(
        &self,
        addr: &Address,
    ) -> BoxFuture<'_, Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), DialFailure>> {
        let target = parse_dial_address(addr);
        Box::pin(async move {
            // ★The connection is made inside the future★ ([`Dialer`] contract: cancelled, nothing is left behind).
            let (key, direct) = target?;
            let id = PublicKey::from_bytes(&key).map_err(|e| {
                LinkError::new(format!("iroh dial: invalid peer key: {}", chain(&e)))
            })?;
            let mut remote = EndpointAddr::new(id);
            for one in direct {
                remote = remote.with_ip_addr(one);
            }
            // `connect` awaits the full handshake — `Connecting::into_0rtt` is never reached (ADR-0388 decision 3).
            let conn = match self.inner.endpoint.connect(remote, &self.inner.alpn).await {
                Ok(conn) => conn,
                Err(err) => return Err(dial_failure(&err)),
            };
            let (mut send, recv) = match conn.open_bi().await {
                Ok(streams) => streams,
                Err(err) => return Err(lost_while_dialing(&err)),
            };
            if let Err(err) = send.write_all(&[PREAMBLE]).await {
                // A peer close that already landed is read by the handshake from these ends (file header 「close mapping」).
                if !matches!(
                    err,
                    WriteError::ConnectionLost(ConnectionError::ApplicationClosed(_))
                ) {
                    return Err(LinkError::new(format!("iroh dial: preamble: {err}")).into());
                }
            }
            Ok(split_link(conn, send, recv, self.inner.max_payload))
        })
    }
}

fn dial_failure(err: &ConnectError) -> DialFailure {
    let closed = match err {
        ConnectError::Connecting {
            source:
                ConnectingError::ConnectionError {
                    source: ConnectionError::ApplicationClosed(close),
                    ..
                },
            ..
        }
        | ConnectError::Connection {
            source: ConnectionError::ApplicationClosed(close),
            ..
        } => Some(close),
        _ => None,
    };
    match closed {
        Some(close) if close.error_code == close_code(CloseCode::HANDSHAKE_REJECTED) => {
            DialFailure::Refused {
                code: CloseCode::HANDSHAKE_REJECTED,
                reason: String::from_utf8_lossy(&close.reason).into_owned(),
            }
        }
        _ => LinkError::new(format!("iroh connect: {}", chain(err))).into(),
    }
}

fn lost_while_dialing(err: &ConnectionError) -> DialFailure {
    match err {
        ConnectionError::ApplicationClosed(close)
            if close.error_code == close_code(CloseCode::HANDSHAKE_REJECTED) =>
        {
            DialFailure::Refused {
                code: CloseCode::HANDSHAKE_REJECTED,
                reason: String::from_utf8_lossy(&close.reason).into_owned(),
            }
        }
        _ => LinkError::new(format!("iroh dial: open stream: {err}")).into(),
    }
}

// ── Accepting side ────────────────────────────────────────────────────────────

/// The endpoint's only [`Listener`] ([`IrohEndpoint::bind`]): one task takes connections, each is gated on its own task, finished ones wait in a ready queue
/// (holding their permit) for [`Listener::accept`]. ★Dropping it reaps those tasks and closes the queued links★ — the endpoint keeps dialing.
pub struct IrohListener {
    ready: mpsc::Receiver<Accepted>,
    permits: PermitPool,
    driver: JoinHandle<()>,
}

impl IrohListener {
    /// Accept permits currently held (stream-accept wait, ready queue, taken-not-released).
    pub fn permits_in_use(&self) -> usize {
        self.permits.in_use()
    }

    /// [`IrohAcceptPolicy::max_pending`].
    pub fn permit_limit(&self) -> usize {
        self.permits.limit()
    }

    pub fn queued(&self) -> usize {
        self.ready.len()
    }
}

impl fmt::Debug for IrohListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IrohListener")
            .field("permits", &self.permits)
            .field("queued", &self.queued())
            .finish_non_exhaustive()
    }
}

impl Listener for IrohListener {
    fn accept(&mut self) -> BoxFuture<'_, Result<Accepted, LinkError>> {
        Box::pin(async move {
            self.ready
                .recv()
                .await
                .ok_or_else(|| LinkError::new("iroh listener: the endpoint is closed"))
        })
    }
}

impl Drop for IrohListener {
    fn drop(&mut self) {
        // The accept task holds the set of gate tasks, so they are reaped together (`drive`).
        self.driver.abort();
    }
}

struct Gate {
    policy: IrohAcceptPolicy,
    permits: PermitPool,
    clock: Arc<dyn Clock>,
    max_payload: usize,
}

/// Takes connections until the endpoint closes. ★Never waits on one connection★ — each is gated on its own task, held by `gates` so reaping this reaps them.
async fn drive(endpoint: Endpoint, gate: Arc<Gate>, ready: mpsc::Sender<Accepted>) {
    let mut gates = JoinSet::new();
    loop {
        tokio::select! {
            incoming = endpoint.accept() => match incoming {
                Some(incoming) => {
                    gates.spawn(admit(incoming, gate.clone(), ready.clone()));
                }
                None => return,
            },
            Some(_) = gates.join_next(), if !gates.is_empty() => {}
        }
    }
}

/// One connection through the gate (file header 「accepting」). Every early return drops `conn` or closes it with a code.
async fn admit(incoming: Incoming, gate: Arc<Gate>, ready: mpsc::Sender<Accepted>) {
    let remote = match incoming.remote_addr() {
        IncomingAddr::Ip(addr) => Some(addr.to_string()),
        IncomingAddr::Relay { url, .. } => Some(format!("relay {url}")),
        _ => None,
    };
    let Ok(accepting) = incoming.accept() else {
        return;
    };
    // Dropping `accepting` at the deadline abandons the half-open connection. Past it, TLS has checked the ALPN and proven the peer's key.
    let Ok(Ok(conn)) = with_deadline(&*gate.clock, gate.policy.handshake_timeout, accepting).await
    else {
        return;
    };
    let key = *conn.remote_id().as_bytes();
    if !(gate.policy.accept_if)(&key) {
        conn.close(close_code(CloseCode::HANDSHAKE_REJECTED), b"not accepted");
        return;
    }
    let Some(permit) = gate.permits.try_take() else {
        conn.close(close_code(CloseCode::GOING_AWAY), b"accept queue full");
        return;
    };
    let opened = with_deadline(
        &*gate.clock,
        gate.policy.stream_accept_timeout,
        open_stream(&conn),
    )
    .await;
    let (send, recv) = match opened {
        Ok(Opened::Link(send, recv)) => (send, recv),
        Ok(Opened::BadPreamble) => {
            conn.close(close_code(CloseCode::PROTOCOL_ERROR), b"no preamble");
            return;
        }
        // `accept_bi` fails only when the connection is gone — nothing to close.
        Ok(Opened::Gone) => return,
        Err(Elapsed) => {
            conn.close(
                close_code(CloseCode::PROTOCOL_ERROR),
                b"no link stream within the accept deadline",
            );
            return;
        }
    };
    let upgraded_at = gate.clock.now();
    let info = LinkInfo {
        remote: remote.map(|text| mask_reason(&text)),
        origin: None,
        peer_key: Some(key.to_vec()),
        socket_ends: None,
    };
    let (tx, rx) = split_link(conn, send, recv, gate.max_payload);
    let accepted = Accepted::new(tx, rx, info, upgraded_at, permit);
    // Full is unreachable (capacity comment in `IrohEndpoint::bind`); Closed = the listener was dropped. Either way the link closes here.
    let _ = ready.try_send(accepted);
}

enum Opened {
    Link(SendStream, RecvStream),
    /// A wrong preamble, or the stream ended or was reset before it (the connection may still be open).
    BadPreamble,
    /// The connection ended before a stream came.
    Gone,
}

/// The dialer's stream and its preamble.
async fn open_stream(conn: &Connection) -> Opened {
    let Ok((send, mut recv)) = conn.accept_bi().await else {
        return Opened::Gone;
    };
    let mut preamble = [0u8; 1];
    match recv.read_exact(&mut preamble).await {
        Ok(()) if preamble[0] == PREAMBLE => Opened::Link(send, recv),
        _ => Opened::BadPreamble,
    }
}

// ── The two link ends ─────────────────────────────────────────────────────────

/// What both ends of one link share. ★Dropping the last end closes the connection with code 0 before the streams go★ — a stream dropped first would send its
/// end (FIN / STOP_SENDING) ahead of the close, and the peer would read "stream ended" instead of a close.
struct LinkShared {
    conn: Connection,
    parked_send: Mutex<Option<SendStream>>,
    parked_recv: Mutex<Option<RecvStream>>,
}

impl Drop for LinkShared {
    fn drop(&mut self) {
        // A no-op if a close already went out; the parked streams drop after this, on a closed connection, so they send nothing.
        self.conn.close(VarInt::from_u32(0), b"");
    }
}

fn split_link(
    conn: Connection,
    send: SendStream,
    recv: RecvStream,
    max_payload: usize,
) -> (Box<dyn LinkTx>, Box<dyn LinkRx>) {
    let shared = Arc::new(LinkShared {
        conn,
        parked_send: Mutex::new(None),
        parked_recv: Mutex::new(None),
    });
    (
        Box::new(IrohTx {
            send: Some(send),
            shared: shared.clone(),
            max_payload,
            mid_frame: false,
        }),
        Box::new(IrohRx {
            recv: Some(recv),
            shared,
            max_payload,
            failed: false,
        }),
    )
}

/// The frame header, or `None` for a frame over `max_payload`.
fn encode_header(frame: &Frame, max_payload: usize) -> Option<[u8; HEADER_LEN]> {
    let (kind, len) = match frame {
        Frame::Text(text) => (KIND_TEXT, text.len()),
        Frame::Binary(bytes) => (KIND_BINARY, bytes.len()),
        Frame::Keepalive => (KIND_KEEPALIVE, 0),
    };
    if len > max_payload {
        return None;
    }
    let len = u32::try_from(len).ok()?;
    let mut header = [0u8; HEADER_LEN];
    header[0] = kind;
    header[1..].copy_from_slice(&len.to_be_bytes());
    Some(header)
}

/// A header read back: the kind byte and the payload length, or why the framing is broken.
fn parse_header(header: [u8; HEADER_LEN], max_payload: usize) -> Result<(u8, usize), String> {
    let mut len = [0u8; 4];
    len.copy_from_slice(&header[1..]);
    let len = u32::from_be_bytes(len) as usize;
    match header[0] {
        KIND_TEXT | KIND_BINARY if len > max_payload => Err(format!(
            "a frame of {len} bytes is over the {max_payload}-byte cap"
        )),
        KIND_TEXT | KIND_BINARY => Ok((header[0], len)),
        KIND_KEEPALIVE if len == 0 => Ok((KIND_KEEPALIVE, 0)),
        KIND_KEEPALIVE => Err(format!("a keepalive carries {len} bytes")),
        kind => Err(format!("unknown frame kind {kind}")),
    }
}

struct IrohTx {
    /// `None` only inside `drop` (parked into [`LinkShared`]).
    send: Option<SendStream>,
    shared: Arc<LinkShared>,
    max_payload: usize,
    /// A write was cut inside a frame (its future dropped) — the stream must not end there (`close`).
    mid_frame: bool,
}

impl IrohTx {
    async fn write(&mut self, frame: Frame) -> Result<(), LinkError> {
        // ★Over the cap fails as a value★ — never a write and never a panic (same reason as `ws` `WsTx::send`, ADR-0292).
        let header = encode_header(&frame, self.max_payload).ok_or_else(|| {
            LinkError::new(format!(
                "iroh write: the outgoing frame is over the {}-byte cap the receiver enforces",
                self.max_payload
            ))
        })?;
        let send = self.send.as_mut().expect("present until drop");
        self.mid_frame = true;
        let written = match send.write_all(&header).await {
            Ok(()) => match &frame {
                Frame::Text(text) => send.write_all(text.as_bytes()).await,
                Frame::Binary(bytes) => send.write_all(bytes).await,
                Frame::Keepalive => Ok(()),
            },
            Err(err) => Err(err),
        };
        self.mid_frame = false;
        match written {
            Ok(()) => Ok(()),
            // The peer said why it closed; the read side reports that code (file header 「close mapping」).
            Err(WriteError::ConnectionLost(ConnectionError::ApplicationClosed(_))) => Ok(()),
            Err(err) => Err(LinkError::new(format!("iroh write: {err}"))),
        }
    }
}

impl LinkTx for IrohTx {
    fn send(&mut self, frame: Frame) -> BoxFuture<'_, Result<(), LinkError>> {
        Box::pin(self.write(frame))
    }

    /// A keepalive frame — the peer's read side raises it as [`Frame::Keepalive`].
    fn ping(&mut self) -> BoxFuture<'_, Result<(), LinkError>> {
        Box::pin(self.write(Frame::Keepalive))
    }

    /// Delivers what was written, then closes (file header 「close mapping」). ★The close goes out even if this future is dropped★ (`CloseOnDrop`).
    fn close(&mut self, close: Close) -> BoxFuture<'_, ()> {
        // ★Built before the future★ — a future dropped before its first poll still sends the close.
        let closing = CloseOnDrop {
            conn: self.shared.conn.clone(),
            code: close_code(close.code),
            reason: clamp_close_reason(&close.reason).as_bytes().to_vec(),
        };
        Box::pin(async move {
            let _closing = closing;
            if self.mid_frame {
                return;
            }
            if let Some(send) = self.send.as_mut() {
                if send.finish().is_ok() {
                    // `Ok(None)` = every byte acknowledged; an error = the connection or stream already ended. Either way, close now.
                    let _ = send.stopped().await;
                }
            }
        })
    }
}

impl Drop for IrohTx {
    fn drop(&mut self) {
        if let Some(send) = self.send.take() {
            *self
                .shared
                .parked_send
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = Some(send);
        }
    }
}

/// Sends the close when dropped — at the end of [`IrohTx::close`] or when its caller's deadline drops that future.
struct CloseOnDrop {
    conn: Connection,
    code: VarInt,
    reason: Vec<u8>,
}

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        self.conn.close(self.code, &self.reason);
    }
}

struct IrohRx {
    /// `None` only inside `drop` (parked into [`LinkShared`]).
    recv: Option<RecvStream>,
    shared: Arc<LinkShared>,
    max_payload: usize,
    /// Reading already failed — ★keeps [`LinkRead::Closed`] meaning "the peer said so"★ ([`LinkRx::recv`] contract).
    failed: bool,
}

impl IrohRx {
    async fn read(&mut self) -> Result<LinkRead, LinkError> {
        if self.failed {
            return Err(LinkError::new("iroh read: link already failed"));
        }
        let recv = self.recv.as_mut().expect("present until drop");
        let mut header = [0u8; HEADER_LEN];
        match recv.read_exact(&mut header).await {
            Ok(()) => {}
            // The peer ended its stream at a frame boundary — it is closing; its close follows.
            Err(ReadExactError::FinishedEarly(0)) => return self.peer_finished().await,
            Err(err) => return self.read_failed(err),
        }
        let (kind, len) = match parse_header(header, self.max_payload) {
            Ok(parsed) => parsed,
            Err(why) => return Err(self.violation(&why)),
        };
        if kind == KIND_KEEPALIVE {
            return Ok(LinkRead::Frame(Frame::Keepalive));
        }
        let mut payload = vec![0u8; len];
        let recv = self.recv.as_mut().expect("present until drop");
        if let Err(err) = recv.read_exact(&mut payload).await {
            return self.read_failed(err);
        }
        if kind == KIND_BINARY {
            return Ok(LinkRead::Frame(Frame::Binary(payload)));
        }
        match String::from_utf8(payload) {
            Ok(text) => Ok(LinkRead::Frame(Frame::Text(text))),
            Err(_) => Err(self.violation("a text frame that is not UTF-8")),
        }
    }

    /// Broken framing: close with [`CloseCode::PROTOCOL_ERROR`] and fail from now on.
    fn violation(&mut self, why: &str) -> LinkError {
        self.failed = true;
        self.shared.conn.close(
            close_code(CloseCode::PROTOCOL_ERROR),
            clamp_close_reason(why).as_bytes(),
        );
        LinkError::new(format!("iroh read: {why}"))
    }

    /// A FIN at a frame boundary: wait (bounded) for the close our peer sends right after it ([`LinkTx::close`]).
    async fn peer_finished(&mut self) -> Result<LinkRead, LinkError> {
        match tokio::time::timeout(FIN_CLOSE_GRACE, self.shared.conn.closed()).await {
            Ok(lost) => self.connection_lost(lost),
            Err(_) => Err(self.violation("the link stream ended without a close")),
        }
    }

    fn read_failed(&mut self, err: ReadExactError) -> Result<LinkRead, LinkError> {
        match err {
            ReadExactError::ReadError(ReadError::ConnectionLost(lost)) => {
                self.connection_lost(lost)
            }
            // A frame boundary FIN never reaches here (`read`); our peer never resets the stream (`LinkShared`).
            ReadExactError::FinishedEarly(_) => {
                Err(self.violation("the link stream ended inside a frame"))
            }
            ReadExactError::ReadError(ReadError::Reset(code)) => {
                Err(self.violation(&format!("the link stream was reset (code {code})")))
            }
            ReadExactError::ReadError(err) => {
                self.failed = true;
                Err(LinkError::new(format!("iroh read: {err}")))
            }
        }
    }

    fn connection_lost(&mut self, lost: ConnectionError) -> Result<LinkRead, LinkError> {
        match lost {
            ConnectionError::ApplicationClosed(close) => {
                let read = peer_close(close.error_code.into_inner(), &close.reason);
                self.failed = read.is_err();
                read
            }
            // Idle-out, a transport error, our own close, a reset connection — locally detected, so never `Closed`.
            lost => {
                self.failed = true;
                Err(LinkError::new(format!("iroh read: {lost}")))
            }
        }
    }
}

impl LinkRx for IrohRx {
    fn recv(&mut self) -> BoxFuture<'_, Result<LinkRead, LinkError>> {
        Box::pin(self.read())
    }
}

impl Drop for IrohRx {
    fn drop(&mut self) {
        if let Some(recv) = self.recv.take() {
            *self
                .shared
                .parked_recv
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = Some(recv);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dial_address_reads_back_its_key_and_direct_addresses() {
        let key: [u8; 32] = std::array::from_fn(|i| (i * 7) as u8);
        let direct: Vec<SocketAddr> = vec![
            "127.0.0.1:4433".parse().unwrap(),
            "[::1]:4433".parse().unwrap(),
        ];
        assert_eq!(
            parse_dial_address(&dial_address(&key, &direct)).unwrap(),
            (key, direct)
        );
        assert_eq!(
            parse_dial_address(&dial_address(&key, &[])).unwrap(),
            (key, Vec::new())
        );
    }

    #[test]
    fn anything_but_a_dial_address_is_refused() {
        let key = [0xab; 32];
        let good = dial_address(&key, &[]).as_str().to_string();
        for bad in [
            String::new(),
            "ws://127.0.0.1:1/".to_string(),
            good.replace("iroh:", "iroh;"),
            good[..good.len() - 2].to_string(),
            format!("{good}00"),
            good.replacen('a', "g", 1),
            format!("{good}@"),
            format!("{good}@127.0.0.1"),
            format!("iroh:{}", "é".repeat(32)),
        ] {
            assert!(
                parse_dial_address(&Address::new(bad.clone())).is_err(),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn each_frame_kind_round_trips_through_its_header() {
        for (frame, kind, len) in [
            (Frame::Text("hé".into()), KIND_TEXT, 3),
            (Frame::Binary(vec![1, 2]), KIND_BINARY, 2),
            (Frame::Keepalive, KIND_KEEPALIVE, 0),
        ] {
            let header = encode_header(&frame, 16).expect("under the cap");
            assert_eq!(parse_header(header, 16), Ok((kind, len)), "{frame:?}");
        }
    }

    #[test]
    fn a_frame_at_the_cap_passes_and_one_over_it_is_refused_both_ways() {
        assert!(encode_header(&Frame::Binary(vec![0; 8]), 8).is_some());
        assert!(encode_header(&Frame::Binary(vec![0; 9]), 8).is_none());
        let header = |kind: u8, len: u32| {
            let mut h = [kind, 0, 0, 0, 0];
            h[1..].copy_from_slice(&len.to_be_bytes());
            h
        };
        assert_eq!(parse_header(header(KIND_TEXT, 8), 8), Ok((KIND_TEXT, 8)));
        assert!(parse_header(header(KIND_TEXT, 9), 8).is_err());
        assert!(parse_header(header(KIND_BINARY, u32::MAX), 8).is_err());
    }

    #[test]
    fn a_broken_header_is_refused() {
        assert!(parse_header([0, 0, 0, 0, 0], 8).is_err(), "kind 0");
        assert!(parse_header([9, 0, 0, 0, 0], 8).is_err(), "unknown kind");
        assert!(
            parse_header([KIND_KEEPALIVE, 0, 0, 0, 1], 8).is_err(),
            "keepalive with a payload"
        );
    }

    #[test]
    fn a_peer_close_with_code_zero_is_a_close_without_a_code() {
        assert!(matches!(peer_close(0, b"bye"), Ok(LinkRead::Closed(None))));
    }

    #[test]
    fn a_peer_close_carries_its_code_and_reason() {
        match peer_close(4001, b"not paired") {
            Ok(LinkRead::Closed(Some(close))) => {
                assert_eq!(close.code, CloseCode::HANDSHAKE_REJECTED);
                assert_eq!(close.reason, "not paired");
            }
            other => panic!("{other:?}"),
        }
        match peer_close(4005, &[0xff, b'x']) {
            Ok(LinkRead::Closed(Some(close))) => assert_eq!(close.reason, "\u{fffd}x"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_peer_close_code_beyond_u16_is_a_read_error() {
        assert!(peer_close(u64::from(u16::MAX) + 1, b"").is_err());
        assert!(matches!(
            peer_close(u64::from(u16::MAX), b""),
            Ok(LinkRead::Closed(Some(_)))
        ));
    }

    #[test]
    fn a_long_close_reason_is_cut_on_a_character_boundary() {
        let short = "x".repeat(MAX_CLOSE_REASON_BYTES);
        assert_eq!(clamp_close_reason(&short), short);
        let long = format!("{}€", "x".repeat(MAX_CLOSE_REASON_BYTES - 1));
        let cut = clamp_close_reason(&long);
        assert_eq!(cut.len(), MAX_CLOSE_REASON_BYTES - 1);
        assert!(cut.chars().all(|c| c == 'x'));
    }

    #[test]
    fn the_config_debug_never_shows_the_secret_key() {
        let config = IrohConfig {
            secret_key: [0x5a; 32],
            alpn: b"engram/peer/1".to_vec(),
            listen_port: 0,
            loopback_only: true,
            relay: Helper::Disabled,
            address_lookup: Helper::Disabled,
            mdns: false,
            max_payload: 8,
        };
        let shown = format!("{config:?}");
        assert!(!shown.contains("90"), "{shown}");
        assert!(!shown.contains("secret"), "{shown}");
        assert!(shown.contains("engram/peer/1"), "{shown}");
    }
}
