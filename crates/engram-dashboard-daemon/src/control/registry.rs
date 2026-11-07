//! Control channel token registry (ADR-0086) — issue, validate and discard a bearer token per (AgentId, epoch).
//!
//! ★Role★: at spawn the daemon issues a 256-bit token per (AgentId, epoch), and when the spawned claude agent
//!   presents that token in the mcp-config Authorization header, it is verified here and the identity ((AgentId, epoch)) is
//!   returned. Incarnation swap (re-activation = a different marker) · kill · terminal discard the old token → a stale-epoch token is
//!   no longer valid (401). `from` (the sender identity) is always derived from this token — the token, not the payload,
//!   is the single source of identity (ADR-0086 §invariant "from is derived only from the token", blocks impersonation).
//!
//! ★Security★: the token string is not logged (tracing records only AgentId/epoch). For the reverse token↔identity lookup
//!   there is a map keyed by the token string, but Debug is not derived (prevents log leaks).
//!
//! tauri import 0 (daemon crate).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, RwLock};

use engram_dashboard_agent::types::AgentId;

// ADR-0110
use engram_dashboard_messaging::envelope::{DeliveryObservation, DeliveryObserver, EnvelopeFormat};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundIdentity {
    pub agent_id: AgentId,
    pub epoch: u32,
}

/// Everything tied to one token — identity + the mail permission **fixed at issue time** (the material for ADR-0133 decision 3's decision).
///
/// ★Why fix it at issue time (why not look it up per request)★: the mail channel is split only by backend capability and
///   **has no runtime switching or fallback** (ADR-0128 decision 1) — so the value at issue time is the truth. The token is
///   newly made per (AgentId, epoch), so re-activation fixes it again with the new value. Mail is a high-traffic
///   path (that is why ADR-0128 chose MCP), so there is also no reason to query the manager on every request.
/// ★Identity and permission go in one value, but **equality is compared by identity only**★: what session pinning compares is
///   `identity`, and it does not compare this struct whole — if the permission bit got into the equality decision, an authorization problem would
///   masquerade as a session-hijack false positive (403).
// ADR-0133
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenBinding {
    pub identity: BoundIdentity,
    /// Can the **HTTP/CLI mail entrance** (`/control/send`·`/control/messages`) be used with this credential.
    ///
    /// ★Narrower than the name — it is not "can it use mail"★: a `false` credential (= an MCP-capable backend spawn)
    ///   **writes mail normally** through `eg_send` on `/mcp`. The design is that the channel is split only by backend capability and
    ///   has no runtime fallback (ADR-0128 decision 1), and this value is used only to
    ///   close the entrance that should be closed under that design. Reading this field as
    ///   "mail permission" in general and layering the check onto the MCP tool path as well would overturn that decision.
    // ADR-0133
    // ADR-0128
    pub mail_allowed: bool,
}

/// This impl is the **only point** on the boundary that connects the two identity types (the entrance passes it as `cmd.from.into()`).
// ADR-0110
impl From<BoundIdentity> for engram_dashboard_messaging::SenderIdentity {
    fn from(b: BoundIdentity) -> Self {
        Self {
            peer_id: b.agent_id,
            epoch: b.epoch,
        }
    }
}

/// Owned by the daemon, one instance (shared via Arc). Internal RwLock — reads (validate, on every tool call) are far more frequent than
/// writes (issue/revoke, at spawn and exit).
///
/// ★ADR-0088 delivery observation slot★: `delivery_observer` is an **optional in-process sink** that
///   receives the delivery-boundary observation records of the control channel relay. Why it lives here = the registry is the only shared object already
///   threaded as an `Arc` through every control-plane path including `handle_send`, so hanging the observation sink here lets an in-proc harness
///   install observation without touching the `handle_send` signature and its call sites (minimal footprint).
///   The production daemon leaves this slot empty (observer=None) → only the existing tracing path remains and overhead is 0. A harness
///   installs it with `set_delivery_observer` and retrieves records directly **without scraping a detached daemon's logs**
///   (ADR-0088 HARD CONSTRAINT · in the vein of ADR-0012 in-process harness).
#[derive(Default)]
pub struct ControlRegistry {
    inner: RwLock<Inner>,
    /// ★Envelope format global state (ADR-0096)★ — **one daemon-global state** that switches the A→B message envelope between colon/xml
    ///   (not per recipient / per metadata presence — ADR-0096 decision 1). Why it lives here = the registry is the only shared object already
    ///   threaded as an `Arc` through `handle_send` (the MCP and CLI entrances) and WS dispatch, so hanging this
    ///   state here lets the two envelope assembly paths read the same value without newly wiring a separate state owner.
    ///   ★Owner = daemon (ADR-0029)★: the registry lives in the daemon process that owns AgentManager — src-tauri
    ///   (the client shell) does not own this state and only forwards the invoke command to the daemon.
    ///   ★AtomicU8 (no lock needed)★: it is a simple scalar toggle so RwLock is overkill — **0=Xml (default)·1=Colon**. Since `Default`
    ///   is 0, the initial value is consistently Xml (ADR-0103 — default flipped by the S18 envelope XML unification). ★Why the value mapping was
    ///   reversed★: `#[derive(Default)]` AtomicU8=0 had to be hung on the new default Xml, so Xml is assigned 0 and Colon 1
    ///   (the default = Xml holds from the derived Default alone, with no separate seed). ★Memory-only★: reset to 0 (Xml) on daemon restart
    ///   (persistence is backlog — ADR-0096 decision 4). read = on every relay (Acquire), write = the set_envelope_format
    ///   command (rare, Release) — the Release/Acquire pair makes sure the first message after the switch sees the new format
    ///   (FIX-4). It is a single scalar so there is no tearing, but cross-thread visibility is established explicitly.
    // ADR-0103
    // ADR-0096
    envelope_format: AtomicU8,
    /// RwLock: installs are rare (once at test setup) and lookups happen on every relay but are short.
    delivery_observer: RwLock<Option<Arc<dyn DeliveryObserver>>>,
    /// ★mid-send yield-seam hook (ADR-0088 Stage 1 — test-harness only)★: a test hook that fires at the **latest point** of the
    ///   resolve↔write gap in `handle_send` (just before injecting the recipient). It is for deterministically reproducing a mid-flight
    ///   epoch race — if inside the hook the same AgentId is swapped and injected as a new-epoch incarnation, resolve saw the
    ///   old incarnation but write lands on the new incarnation. ★ADR-0086 §F5 marks the race as
    ///   design-accepted★ (mail targets the **logical agent**, so landing on the new incarnation is the correct behavior)
    ///   — this seam is meant to **deterministically observe** that behavior, not to pin the epoch.
    ///   Production build = feature OFF → this field itself does not exist (the hook firing code is not even compiled,
    ///   handle_send behavior is byte-identical). RwLock: installs are rare and lookups short (same shape as observer).
    // ADR-0088
    #[cfg(feature = "test-harness")]
    mid_send_hook: RwLock<Option<Arc<dyn Fn() + Send + Sync>>>,
}

#[derive(Default)]
struct Inner {
    token_to_binding: HashMap<String, TokenBinding>,
    agent_to_token: HashMap<AgentId, String>,
    session_to_identity: HashMap<String, BoundIdentity>,
}

impl ControlRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// **Remove that AgentId's previous-epoch token** (rotation = discard — one AgentId = one live token).
    /// The token is made by the caller (provision) with a CSPRNG and handed over — this registry does no random number generation and owns
    /// only the mapping (separation of generation and mapping concerns).
    ///
    /// `mail_allowed` = whether mail requests will be accepted with this credential (ADR-0133). The decision itself belongs to the caller
    ///   (`DaemonControlChannel::provision`) alone — here the received value is only recorded (no derivation: if two places
    ///   decide, they diverge).
    /// ★Lock order (ADR-0006)★: the write lock is taken only inside this function, and no external call is made (pure map manipulation).
    pub fn issue(&self, id: AgentId, epoch: u32, token: String, mail_allowed: bool) {
        let mut inner = self.inner.write().expect("control registry poisoned");
        if let Some(old) = inner.agent_to_token.remove(&id) {
            inner.token_to_binding.remove(&old);
            // The session binding attached with the old token is also stale now — erase it together.
            inner
                .session_to_identity
                .retain(|_, ident| ident.agent_id != id);
        }
        inner.token_to_binding.insert(
            token.clone(),
            TokenBinding {
                identity: BoundIdentity {
                    agent_id: id,
                    epoch,
                },
                mail_allowed,
            },
        );
        inner.agent_to_token.insert(id, token);
        tracing::info!(agent = %id, epoch, mail_allowed, "제어 채널 토큰 발급(ADR-0086)");
    }

    /// None if absent or discarded — the caller (auth middleware) returns 401.
    ///
    /// ★Identity and mail permission come out together **in one lookup**★: if the two are looked up separately, a discard or reissue between them
    ///   can produce "identity is A but permission is B", and that combination has no defined answer.
    pub fn validate(&self, token: &str) -> Option<TokenBinding> {
        self.inner
            .read()
            .expect("control registry poisoned")
            .token_to_binding
            .get(token)
            .copied()
    }

    /// For sender liveness "observation" (not a gate — it does not block delivery and only records, user decision 2026-07-19).
    ///
    /// false = the token was discarded/replaced by kill/rotate. At relay time the original token string no longer remains, so
    ///   we trace back through agent_to_token → token_to_binding and check that the epoch matches.
    pub fn is_identity_live(&self, identity: BoundIdentity) -> bool {
        let inner = self.inner.read().expect("control registry poisoned");
        inner
            .agent_to_token
            .get(&identity.agent_id)
            .and_then(|token| inner.token_to_binding.get(token))
            .map(|bound| bound.identity.epoch == identity.epoch)
            .unwrap_or(false)
    }

    /// The identity passed in must be **one the middleware already verified** (ADR-0086). Called when the auth middleware finds
    /// Mcp-Session-Id in the initialize response. Afterwards it becomes the target of identity checks for requests coming in on that session,
    /// acceptance observation and revoke cleanup.
    ///
    /// ★no-overwrite + exact-token recheck (FIX 7 + round-2 F2)★: the session↔identity is fixed **only once, at initialize**
    ///   (identity pinning). If a binding already exists it is not overwritten — that way the middleware can detect and reject a
    ///   **cross-token takeover** where session S is opened with token A and later a request is sent to the same session with token B
    ///   (if the binding were overwritten with B, the hijack would look successful). Also, just before binding, it rechecks **whether the very token string used for
    ///   verification is still this agent's current credential** (`agent_to_token[agent] == validated_token`) —
    ///   if a revoke (token evict) or a reissue (a new token for the same agent) slips in between validate→bind, the binding is skipped and
    ///   it signals failure (returns None).
    /// ★Why recheck the **exact token** rather than the identity (round-2 F2)★: earlier it only checked whether the identity of the live token that `agent_to_token[agent]`
    ///   points to equals `identity` (id and epoch match). That relies on the **remote invariant** "the incarnation marker always
    ///   differs on every re-activation (ADR-0007)" and is safe only on the assumption that reissuing the same (id,epoch) is impossible.
    ///   To make the check **local**, it takes the verified token string itself and directly compares whether that string
    ///   is still the current credential — then even if that remote invariant breaks (even if a token is reissued with the same id·epoch),
    ///   an initialize that arrives with a stale token is not bound.
    ///   Returns: Some(identity) if newly bound, None if one already exists (duplicate init) or the token is dead or was replaced
    ///   (the caller handles accordingly — a duplicate is harmless, death/replacement is unauthorized).
    pub fn bind_session_if_absent(
        &self,
        session_id: &str,
        identity: BoundIdentity,
        validated_token: &str,
    ) -> Option<BoundIdentity> {
        let mut inner = self.inner.write().expect("control registry poisoned");
        let token_current = inner
            .agent_to_token
            .get(&identity.agent_id)
            .map(|cur| cur == validated_token)
            .unwrap_or(false);
        if !token_current {
            return None;
        }
        if inner.session_to_identity.contains_key(session_id) {
            return None;
        }
        inner
            .session_to_identity
            .insert(session_id.to_string(), identity);
        tracing::info!(
            agent = %identity.agent_id,
            epoch = identity.epoch,
            "제어 채널 세션 바인딩(ADR-0086, pinned)"
        );
        Some(identity)
    }

    pub fn identity_for_session(&self, session_id: &str) -> Option<BoundIdentity> {
        self.inner
            .read()
            .expect("control registry poisoned")
            .session_to_identity
            .get(session_id)
            .copied()
    }

    /// Called by the middleware when the client folds the session with DELETE (FIX 8). Separately from revoke-time cleanup,
    /// it keeps session_to_identity from growing without bound on the normal teardown path (repeated initialize→DELETE does not
    /// pile up entries). No-op if absent.
    pub fn unbind_session(&self, session_id: &str) {
        let mut inner = self.inner.write().expect("control registry poisoned");
        if inner.session_to_identity.remove(session_id).is_some() {
            // It is a header value the caller chose, so sanitize it when putting it in a log — this module's log field discipline has its authority in
            //   `connection_core::sanitize_for_log` (`control::agent::preview` is for **response
            //   wording** only and lets control characters through).
            tracing::info!(
                session = %crate::connection_core::sanitize_for_log(session_id),
                "제어 채널 세션 바인딩 해제(DELETE, ADR-0086)"
            );
        }
    }

    /// Discard the (AgentId, epoch) token + remove that identity's session binding. Called from terminal (reaper) / kill.
    ///
    /// ★epoch-guard★: discard **only when the requested marker matches the marker of the currently live token**. This keeps a stale terminal from
    ///   erasing a live token newly issued by re-activation (new incarnation = a different marker) (extends the spirit of ADR-0007/0084 to the token registry).
    ///   If they do not match it is a no-op (a new token has already taken the place in the meantime).
    ///   ★Do not change it to a greater/less comparison★: the marker is a random number drawn per incarnation so order has no meaning (`AgentProfile::epoch`).
    /// ★idempotent★: if already absent (double revoke — kill up front + reaper) it is a no-op. So it is safe even if kill_agent and the reaper
    ///   both call it.
    pub fn revoke(&self, id: AgentId, epoch: u32) {
        let mut inner = self.inner.write().expect("control registry poisoned");
        match inner.agent_to_token.get(&id) {
            Some(token) => {
                let cur = inner.token_to_binding.get(token).map(|b| b.identity.epoch);
                if cur != Some(epoch) {
                    return;
                }
            }
            None => return,
        }
        if let Some(token) = inner.agent_to_token.remove(&id) {
            inner.token_to_binding.remove(&token);
        }
        inner
            .session_to_identity
            .retain(|_, ident| !(ident.agent_id == id && ident.epoch == epoch));
        tracing::info!(agent = %id, epoch, "제어 채널 토큰 폐기(ADR-0086)");
    }

    /// For observation (ADR-0086 acceptance) — integration tests (a separate crate) use it too, so it is exposed publicly rather than hidden behind cfg(test)
    /// (a pure query, safe).
    pub fn live_token_count(&self) -> usize {
        self.inner
            .read()
            .expect("control registry poisoned")
            .token_to_binding
            .len()
    }

    /// For observation (acceptance) — integration tests assert "after the agent connected the daemon held the session" with this value.
    pub fn bound_session_count(&self) -> usize {
        self.inner
            .read()
            .expect("control registry poisoned")
            .session_to_identity
            .len()
    }

    pub fn set_delivery_observer(&self, observer: Arc<dyn DeliveryObserver>) {
        *self
            .delivery_observer
            .write()
            .expect("delivery observer poisoned") = Some(observer);
    }

    /// Emit a delivery observation record (ADR-0088) — `handle_send` calls it on every relay success/failure.
    /// ★Lock discipline (ADR-0006)★: clone the observer Arc, release the lock at once, then call `observe` outside the lock
    /// (no external call is made while holding the lock).
    ///
    /// ★Observation never disturbs delivery or ACK (FIX-2 — immediate push invariant)★: if the user-supplied `observe`
    ///   panics, that panic rides up through `handle_send` and after the relay (the bytes are already written) the ACK cannot be
    ///   sent, and the sender may retry causing **duplicate delivery** — delivery semantics must not change just
    ///   because observation was turned on. So the `observe` call is isolated with `catch_unwind`, the panic is swallowed
    ///   here (only a warn is left), and record_delivery always returns normally. It is caught after the clone and outside the lock, not while holding the RwLock,
    ///   so poison does not propagate either (consistent with the lock discipline).
    ///
    /// ★In a production process this catch_unwind is not reached (FIX-R2-3 · ADR-0292)★: this call is outside the containment mark so
    ///   on a panic the hook ends the process before unwinding, so this isolation is **limited to hook-less test builds**. It is
    ///   harmless now: production installs no observer (observer=None) so the `if let Some(sink)` below short-circuits before
    ///   catch_unwind. If a production observer is ever attached, the fact that its panic would terminate the daemon must be reconsidered.
    pub fn record_delivery(&self, obs: DeliveryObservation) {
        let sink = self
            .delivery_observer
            .read()
            .expect("delivery observer poisoned")
            .clone();
        if let Some(sink) = sink {
            // AssertUnwindSafe: on a panic obs is consumed and gone and we only leave a log (we do not observe a shared invariant
            //   left broken — neither sink nor obs is reused after the catch).
            let msg_id = obs.msg_id.clone();
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || sink.observe(obs)));
            if result.is_err() {
                tracing::warn!(
                    msg_id = %msg_id,
                    "ADR-0088: DeliveryObserver.observe 가 panic — 격리 삼킴(배달/ACK 는 영향 없음). 관측 싱크 버그."
                );
            }
        }
    }

    // ── Envelope format global state (ADR-0096·0103) ──────────────────────────────────────────

    /// ★Defense against unknown values (fold-unknown = Xml)★: storage is only done by set_envelope_format so it is always 0/1, but
    ///   defensively it folds **only 1 (Colon) to Colon and everything else to Xml** — default-safe = since the production normal value is xml,
    ///   fold-unknown is made consistent with Xml (the derived Default 0 also goes to Xml via this branch).
    // ADR-0103
    pub fn envelope_format(&self) -> EnvelopeFormat {
        match self.envelope_format.load(Ordering::Acquire) {
            1 => EnvelopeFormat::Colon,
            _ => EnvelopeFormat::Xml,
        }
    }

    pub fn set_envelope_format(&self, format: EnvelopeFormat) {
        let v = match format {
            EnvelopeFormat::Xml => 0u8,
            EnvelopeFormat::Colon => 1u8,
        };
        self.envelope_format.store(v, Ordering::Release);
        tracing::info!(?format, "봉투 포맷 전역 상태 전환(ADR-0096)");
    }

    #[cfg(feature = "test-harness")]
    pub fn set_mid_send_hook(&self, hook: Option<Arc<dyn Fn() + Send + Sync>>) {
        // ★Lock discipline (spirit of ADR-0006)★: plug in the new hook and drop the old hook Arc **outside the lock**. If dropped while holding
        //   the lock, the old hook's destructor (a captured Drop) deadlocks when it re-enters the registry, and if it panics the lock
        //   is poisoned — foreign code (including Drop) is not run under the lock. `_old` is dropped at the end of this function after leaving
        //   the guard scope (after the lock is released).
        let _old = {
            let mut guard = self.mid_send_hook.write().expect("mid-send hook poisoned");
            std::mem::replace(&mut *guard, hook)
        };
    }

    /// ★Lock discipline (ADR-0006)★: clone the hook Arc under a short read lock, release the lock at once, and
    ///   call it **outside the lock** — foreign code (the hook) is never called while holding the registry lock.
    #[cfg(feature = "test-harness")]
    pub fn fire_mid_send_hook(&self) {
        let hook = self
            .mid_send_hook
            .read()
            .expect("mid-send hook poisoned")
            .clone();
        if let Some(hook) = hook {
            hook();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mail permission is not the concern of the tests that use this helper — set it to the side where mail is open (the value of a non-MCP spawn).
    ///   Tests that look at that axis call `issue` directly.
    fn tok(reg: &ControlRegistry, id: AgentId, epoch: u32) -> String {
        // Deterministic test token (real provision uses a CSPRNG). Only uniqueness is kept.
        let t = format!("tok-{id}-{epoch}");
        reg.issue(id, epoch, t.clone(), true);
        t
    }

    fn current_token(reg: &ControlRegistry, id: AgentId) -> String {
        reg.inner
            .read()
            .unwrap()
            .agent_to_token
            .get(&id)
            .cloned()
            .expect("live token")
    }

    #[test]
    fn issue_then_validate_returns_identity() {
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        let t = tok(&reg, id, 0);
        let bound = reg.validate(&t).expect("valid token");
        assert_eq!(bound.identity.agent_id, id);
        assert_eq!(bound.identity.epoch, 0);
    }

    /// ★Mail permission is fixed into the credential and **fixed again on rotation** (ADR-0133 decision 2)★: if re-activation
    ///   changes the channel, the new token comes out carrying the new decision — if the old decision remained, an agent respawned on a different backend
    ///   would run with the old permission.
    #[test]
    fn issue_records_the_mail_verdict_and_rotation_replaces_it() {
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        reg.issue(id, 0, "mail-off".to_string(), false);
        assert!(
            !reg.validate("mail-off").expect("valid").mail_allowed,
            "발급 시 false 면 검증도 false"
        );
        reg.issue(id, 1, "mail-on".to_string(), true);
        assert!(reg.validate("mail-on").expect("valid").mail_allowed);
        assert!(
            reg.validate("mail-off").is_none(),
            "회전으로 옛 토큰(과 그 판정)은 폐기"
        );
    }

    #[test]
    fn unknown_token_is_none() {
        let reg = ControlRegistry::new();
        assert!(reg.validate("nope").is_none());
    }

    #[test]
    fn is_identity_live_tracks_revoke_and_rotation() {
        // ★F3 regression★ — commit-point recheck.
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        tok(&reg, id, 0);
        let ident0 = BoundIdentity {
            agent_id: id,
            epoch: 0,
        };
        assert!(reg.is_identity_live(ident0), "발급 직후 신원은 live");

        tok(&reg, id, 1);
        assert!(
            !reg.is_identity_live(ident0),
            "회전 후 옛 epoch 신원은 dead(F3)"
        );
        assert!(
            reg.is_identity_live(BoundIdentity {
                agent_id: id,
                epoch: 1
            }),
            "새 epoch 신원은 live"
        );

        reg.revoke(id, 1);
        assert!(
            !reg.is_identity_live(BoundIdentity {
                agent_id: id,
                epoch: 1
            }),
            "revoke 후 신원은 dead(F3)"
        );

        assert!(
            !reg.is_identity_live(BoundIdentity {
                agent_id: AgentId::new_v4(),
                epoch: 0
            }),
            "미발급 신원은 dead"
        );
    }

    #[test]
    fn epoch_rotation_revokes_old_token() {
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        let old = tok(&reg, id, 0);
        let new = tok(&reg, id, 1);
        assert!(
            reg.validate(&old).is_none(),
            "회전된 구 epoch 토큰은 폐기(stale) — validate None"
        );
        let bound = reg.validate(&new).expect("새 토큰 유효");
        assert_eq!(bound.identity.epoch, 1);
        assert_eq!(reg.live_token_count(), 1, "한 AgentId = 산 토큰 1개");
    }

    #[test]
    fn revoke_matching_epoch_removes_token() {
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        let t = tok(&reg, id, 3);
        reg.revoke(id, 3);
        assert!(reg.validate(&t).is_none(), "kill revoke 후 토큰 무효");
        assert_eq!(reg.live_token_count(), 0);
    }

    #[test]
    fn revoke_is_idempotent() {
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        tok(&reg, id, 0);
        reg.revoke(id, 0);
        reg.revoke(id, 0);
        assert_eq!(reg.live_token_count(), 0);
    }

    #[test]
    fn stale_revoke_does_not_kill_live_token() {
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        tok(&reg, id, 0); // old session token
        let live = tok(&reg, id, 1); // re-activation — new live token
        reg.revoke(id, 0); // delayed stale terminal
        assert!(
            reg.validate(&live).is_some(),
            "stale epoch 0 revoke 가 산 epoch 1 토큰을 지우면 안 됨(epoch-guard)"
        );
    }

    #[test]
    fn bind_and_lookup_session_identity() {
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        let t = tok(&reg, id, 0);
        let ident = BoundIdentity {
            agent_id: id,
            epoch: 0,
        };
        assert_eq!(
            reg.bind_session_if_absent("sess-abc", ident, &t),
            Some(ident),
            "첫 바인딩은 성공"
        );
        assert_eq!(reg.identity_for_session("sess-abc"), Some(ident));
        assert!(reg.identity_for_session("other").is_none());
    }

    #[test]
    fn bind_if_absent_no_overwrite_pins_identity() {
        // ★identity pinning (FIX 7)★ — the registry-side foundation for preventing cross-token takeover.
        let reg = ControlRegistry::new();
        let a = AgentId::new_v4();
        let b = AgentId::new_v4();
        let ta = tok(&reg, a, 0);
        let tb = tok(&reg, b, 0);
        let ident_a = BoundIdentity {
            agent_id: a,
            epoch: 0,
        };
        let ident_b = BoundIdentity {
            agent_id: b,
            epoch: 0,
        };
        assert_eq!(
            reg.bind_session_if_absent("sess", ident_a, &ta),
            Some(ident_a)
        );
        assert_eq!(
            reg.bind_session_if_absent("sess", ident_b, &tb),
            None,
            "이미 바인딩된 세션은 덮어쓰지 않는다(pinning)"
        );
        assert_eq!(
            reg.identity_for_session("sess"),
            Some(ident_a),
            "기존 신원 A 가 유지돼야"
        );
    }

    #[test]
    fn bind_if_absent_rejects_revoked_token() {
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        let t = tok(&reg, id, 0);
        let ident = BoundIdentity {
            agent_id: id,
            epoch: 0,
        };
        reg.revoke(id, 0); // discarded just before binding
        assert_eq!(
            reg.bind_session_if_absent("sess", ident, &t),
            None,
            "죽은 토큰의 세션은 바인딩되지 않아야(exact-token recheck)"
        );
        assert!(reg.identity_for_session("sess").is_none());
    }

    #[test]
    fn bind_if_absent_rejects_stale_token_after_same_agent_reissue() {
        // ★round-2 F2 regression★: issue can be called again with the same (id,epoch), so it reproduces even without bumping the epoch here
        //   — this is the key that proves "safe even if the epoch-always-bumps remote invariant breaks".
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        let stale = tok(&reg, id, 0); // Initial issue — assume it was validated with this token.
        let ident = BoundIdentity {
            agent_id: id,
            epoch: 0,
        };
        // Reissue with the same agent and the same epoch.
        reg.issue(id, 0, "reissued-token".to_string(), true);
        assert_ne!(
            current_token(&reg, id),
            stale,
            "재발급으로 현재 토큰이 바뀜"
        );
        assert_eq!(
            reg.bind_session_if_absent("sess", ident, &stale),
            None,
            "재발급된 뒤 stale 토큰의 세션은 바인딩되지 않아야(exact-token F2)"
        );
        assert!(reg.identity_for_session("sess").is_none());
        // Control group.
        let cur = current_token(&reg, id);
        assert_eq!(
            reg.bind_session_if_absent("sess", ident, &cur),
            Some(ident),
            "현재 크레덴셜 토큰으로는 바인딩 성공"
        );
    }

    #[test]
    fn unbind_session_prunes_binding() {
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        let t = tok(&reg, id, 0);
        reg.bind_session_if_absent(
            "sess-del",
            BoundIdentity {
                agent_id: id,
                epoch: 0,
            },
            &t,
        );
        assert_eq!(reg.bound_session_count(), 1);
        reg.unbind_session("sess-del");
        assert_eq!(reg.bound_session_count(), 0, "DELETE 후 바인딩 제거");
        reg.unbind_session("sess-del"); // no-op if absent (double safety)
    }

    #[test]
    fn revoke_clears_session_binding() {
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        let t = tok(&reg, id, 0);
        reg.bind_session_if_absent(
            "sess-x",
            BoundIdentity {
                agent_id: id,
                epoch: 0,
            },
            &t,
        );
        reg.revoke(id, 0);
        assert!(
            reg.identity_for_session("sess-x").is_none(),
            "revoke 는 세션 바인딩도 지운다"
        );
        assert_eq!(reg.bound_session_count(), 0);
    }

    // ── ADR-0096·0103: envelope format global state ────────────────────────────────────────────
    #[test]
    fn envelope_format_defaults_to_xml_and_toggles() {
        let reg = ControlRegistry::new();
        assert_eq!(
            reg.envelope_format(),
            EnvelopeFormat::Xml,
            "새 registry 기본 봉투 포맷은 xml 이어야(ADR-0103 flip)"
        );
        reg.set_envelope_format(EnvelopeFormat::Colon);
        assert_eq!(
            reg.envelope_format(),
            EnvelopeFormat::Colon,
            "set(Colon) 후 colon 으로 읽혀야(잔존 스위치)"
        );
        reg.set_envelope_format(EnvelopeFormat::Xml);
        assert_eq!(
            reg.envelope_format(),
            EnvelopeFormat::Xml,
            "set(Xml) 후 xml 로 복귀"
        );
    }

    #[test]
    fn concurrent_issue_validate_is_safe() {
        use std::sync::Arc;
        let reg = Arc::new(ControlRegistry::new());
        let mut handles = Vec::new();
        for i in 0..8u32 {
            let reg = reg.clone();
            handles.push(std::thread::spawn(move || {
                let id = AgentId::new_v4();
                let t = format!("t-{i}");
                reg.issue(id, i, t.clone(), true);
                assert_eq!(reg.validate(&t).map(|b| b.identity.epoch), Some(i));
                reg.revoke(id, i);
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(reg.live_token_count(), 0);
    }
}
