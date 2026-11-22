//! Control channel token registry (ADR-0086) — issue, validate and discard a bearer token per (AgentId, epoch).
//!
//! ★Role★: at spawn the daemon issues a 256-bit token per (AgentId, epoch); when the spawned claude agent presents it in the mcp-config Authorization header, it is verified here and the identity ((AgentId, epoch)) is returned. A new incarnation (re-activation = a different marker) · kill · terminal discards the old token, so a stale-epoch token gets 401. `from` (the sender identity) is always derived from this token, never from the payload (ADR-0086 §invariant, blocks impersonation).
//!
//! ★Security★: the token string is never logged (tracing records only AgentId/epoch). The reverse token↔identity map is keyed by the token string, so Debug is not derived.

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

/// Everything tied to one token — identity + the mail permission fixed at issue time (ADR-0133 decision 3).
///
/// ★Why fix it at issue time, not per request★: the mail channel is split only by backend capability with no runtime switching or fallback (ADR-0128 decision 1), so the value at issue time is the truth. The token is newly made per (AgentId, epoch), so re-activation fixes it again; mail is high-traffic, so no per-request manager query either.
/// ★Equality is compared by identity only★: session pinning compares `identity`, not this struct — a permission bit in the equality decision would make an authorization problem masquerade as a session-hijack false positive (403).
// ADR-0133
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenBinding {
    pub identity: BoundIdentity,
    /// Can the HTTP/CLI mail entrance (`/control/send`·`/control/messages`) be used with this credential.
    ///
    /// ★Narrower than the name — not "can it use mail"★: a `false` credential (an MCP-capable backend spawn) writes mail normally through `eg_send` on `/mcp`. The channel is split only by backend capability with no runtime fallback (ADR-0128 decision 1); this value only closes the entrance that should be closed. Layering the check onto the MCP tool path would overturn that decision.
    // ADR-0133
    // ADR-0128
    pub mail_allowed: bool,
}

/// The only point on the boundary that connects the two identity types (the entrance passes it as `cmd.from.into()`).
// ADR-0110
impl From<BoundIdentity> for engram_dashboard_messaging::SenderIdentity {
    fn from(b: BoundIdentity) -> Self {
        Self {
            peer_id: b.agent_id,
            epoch: b.epoch,
            // A control-channel caller is an agent of this PC.
            node: None,
        }
    }
}

/// Owned by the daemon, one instance (shared via Arc). Internal RwLock — reads (validate, every tool call) far outnumber writes (issue/revoke, at spawn and exit).
///
/// ★ADR-0088 delivery observation slot★: `delivery_observer` is an optional in-process sink for the control channel relay's delivery-boundary observation records. It lives here because the registry is the one shared object already threaded as an `Arc` through every control-plane path including `handle_send`, so an in-proc harness installs observation without touching the `handle_send` signature. Production leaves it empty (observer=None): only the tracing path remains, overhead 0. A harness installs it with `set_delivery_observer` and reads records directly instead of scraping a detached daemon's logs (ADR-0088 HARD CONSTRAINT · ADR-0012).
#[derive(Default)]
pub struct ControlRegistry {
    inner: RwLock<Inner>,
    /// ★Envelope format global state (ADR-0096)★ — one daemon-global state switching the A→B message envelope between colon/xml (not per recipient / per metadata presence — ADR-0096 decision 1). It lives here because the registry is the one shared object already threaded as an `Arc` through `handle_send` (MCP and CLI entrances) and WS dispatch, so both envelope assembly paths read the same value.
    ///   ★Owner = daemon (ADR-0029)★: src-tauri (the client shell) does not own this state and only forwards the invoke command.
    ///   ★AtomicU8, no lock★: a scalar toggle — 0=Xml (default)·1=Colon, so the derived `Default` gives Xml with no separate seed (ADR-0103). ★Memory-only★: reset to Xml on daemon restart (persistence is backlog — ADR-0096 decision 4). Read on every relay (Acquire), written by the rare set_envelope_format command (Release) — the pair makes the first message after a switch see the new format (FIX-4).
    // ADR-0103
    // ADR-0096
    envelope_format: AtomicU8,
    /// ★The user's UI language (`ui.language`, a shell setting) as the shell last told it★ (`AgentCommand::SetUiLanguage`) — the language
    ///   of the daemon's own user-facing text, its approval dialogs ([`crate::ask_user`]). Same reason to live here as the envelope format:
    ///   the WS dispatch already holds this object. 0 = English (the default while no app has said otherwise) · 1 = Korean. Memory only.
    // ADR-0372
    ui_language: AtomicU8,
    /// RwLock: installs are rare (test setup), lookups on every relay are short.
    delivery_observer: RwLock<Option<Arc<dyn DeliveryObserver>>>,
    /// ★Mid-send yield-seam hook (ADR-0088 Stage 1 — test-harness only)★: fires at the latest point of the resolve↔write gap in `handle_send` (just before injecting the recipient), to deterministically reproduce a mid-flight epoch race — if the hook swaps the same AgentId in as a new-epoch incarnation, resolve saw the old incarnation but write lands on the new one. ★ADR-0086 §F5 marks that race design-accepted★ (mail targets the logical agent); this seam observes that behavior, not pins the epoch.
    ///   Production build = feature OFF → the field does not exist and handle_send is byte-identical. RwLock: same shape as observer.
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

    /// Removes that AgentId's previous-epoch token (rotation = discard — one AgentId = one live token).
    /// The caller (provision) makes the token with a CSPRNG and hands it over — this registry owns only the mapping.
    ///
    /// `mail_allowed` = whether mail requests are accepted with this credential (ADR-0133). The decision belongs to `DaemonControlChannel::provision` alone; here the value is only recorded (two deciders would diverge).
    /// ★Lock order (ADR-0006)★: the write lock is taken only inside this function, with no external call.
    pub fn issue(&self, id: AgentId, epoch: u32, token: String, mail_allowed: bool) {
        let mut inner = self.inner.write().expect("control registry poisoned");
        if let Some(old) = inner.agent_to_token.remove(&id) {
            inner.token_to_binding.remove(&old);
            // The session binding attached with the old token is stale too — erase it together.
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
        tracing::info!(agent = %id, epoch, mail_allowed, "control channel token issued (ADR-0086)");
    }

    /// None if absent or discarded — the caller (auth middleware) returns 401.
    ///
    /// ★Identity and mail permission come out in one lookup★: separate lookups could straddle a discard or reissue and give "identity A, permission B", which has no defined answer.
    pub fn validate(&self, token: &str) -> Option<TokenBinding> {
        self.inner
            .read()
            .expect("control registry poisoned")
            .token_to_binding
            .get(token)
            .copied()
    }

    /// Sender liveness "observation" (not a gate — it only records, never blocks delivery).
    ///
    /// false = the token was discarded/replaced by kill/rotate. At relay time the original token string is gone, so trace agent_to_token → token_to_binding and check the epoch matches.
    pub fn is_identity_live(&self, identity: BoundIdentity) -> bool {
        let inner = self.inner.read().expect("control registry poisoned");
        inner
            .agent_to_token
            .get(&identity.agent_id)
            .and_then(|token| inner.token_to_binding.get(token))
            .map(|bound| bound.identity.epoch == identity.epoch)
            .unwrap_or(false)
    }

    /// The identity passed in must be one the middleware already verified (ADR-0086). Called when the auth middleware finds Mcp-Session-Id in the initialize response; afterwards it is the target of identity checks for requests on that session, acceptance observation and revoke cleanup.
    ///
    /// ★No-overwrite + exact-token recheck (FIX 7)★: session↔identity is fixed only once, at initialize (identity pinning). An existing binding is never overwritten, so the middleware can detect and reject a cross-token takeover (session S opened with token A, later used with token B). Just before binding it also rechecks that the very token string used for verification is still this agent's current credential (`agent_to_token[agent] == validated_token`) — a revoke or reissue slipping in between validate→bind makes it skip the binding and return None.
    /// ★Why recheck the exact token, not the identity★: checking only that the identity of the live token matches (id and epoch) relies on the remote invariant "the incarnation marker differs on every re-activation (ADR-0007)". Comparing the verified token string directly keeps the check local — even if that invariant breaks (a token reissued with the same id·epoch), an initialize arriving with a stale token is not bound.
    /// Returns Some(identity) if newly bound; None if a binding exists (duplicate init, harmless) or the token is dead or replaced (unauthorized).
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
            "control channel session bound (ADR-0086, pinned)"
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

    /// Called by the middleware when the client closes the session with DELETE (FIX 8). Keeps session_to_identity from growing without bound on the normal teardown path. No-op if absent.
    pub fn unbind_session(&self, session_id: &str) {
        let mut inner = self.inner.write().expect("control registry poisoned");
        if inner.session_to_identity.remove(session_id).is_some() {
            // A header value chosen by the caller, so sanitize it for the log — the log field discipline is `connection_core::sanitize_for_log` (`control::agent::preview` is for response wording only and lets control characters through).
            tracing::info!(
                session = %crate::connection_core::sanitize_for_log(session_id),
                "control channel session unbound (DELETE, ADR-0086)"
            );
        }
    }

    /// Discards the (AgentId, epoch) token + removes that identity's session binding. Called from terminal (reaper) / kill.
    ///
    /// ★Epoch-guard★: discard only when the requested marker matches the live token's marker, so a stale terminal cannot erase a live token newly issued by re-activation (ADR-0007/0084 spirit). On mismatch it is a no-op.
    ///   ★Do not change it to a greater/less comparison★: the marker is a random number drawn per incarnation, so order has no meaning (`AgentProfile::epoch`).
    /// ★Idempotent★: already absent (kill up front + reaper) is a no-op, so kill_agent and the reaper may both call it.
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
        tracing::info!(agent = %id, epoch, "control channel token revoked (ADR-0086)");
    }

    /// Exposed publicly (not cfg(test)) because integration tests in a separate crate use it for observation (ADR-0086 acceptance); a pure query.
    pub fn live_token_count(&self) -> usize {
        self.inner
            .read()
            .expect("control registry poisoned")
            .token_to_binding
            .len()
    }

    /// For observation — integration tests assert "after the agent connected the daemon held the session".
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

    /// Emits a delivery observation record (ADR-0088) — `handle_send` calls it on every relay success/failure.
    /// ★Lock discipline (ADR-0006)★: clone the observer Arc, release the lock, then call `observe` outside the lock.
    ///
    /// ★Observation never disturbs delivery or ACK (FIX-2)★: a panic in the user-supplied `observe` would ride up through `handle_send` after the bytes are already written, the ACK could not be sent, and the sender may retry causing duplicate delivery. So `observe` is isolated with `catch_unwind`, the panic swallowed with a warn, and record_delivery always returns normally. The catch is after the clone and outside the lock, so poison does not propagate either.
    ///
    /// ★In a production process this catch_unwind is not reached (FIX-R2-3 · ADR-0292)★: the call is outside the containment mark, so the hook ends the process on a panic before unwinding; the isolation is limited to hook-less test builds. Harmless now: production installs no observer, so `if let Some(sink)` short-circuits. If a production observer is ever attached, reconsider that its panic would terminate the daemon.
    pub fn record_delivery(&self, obs: DeliveryObservation) {
        let sink = self
            .delivery_observer
            .read()
            .expect("delivery observer poisoned")
            .clone();
        if let Some(sink) = sink {
            // AssertUnwindSafe: on a panic obs is consumed and we only log; neither sink nor obs is reused after the catch.
            let msg_id = obs.msg_id.clone();
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || sink.observe(obs)));
            if result.is_err() {
                tracing::warn!(
                    msg_id = %msg_id,
                    "ADR-0088: DeliveryObserver.observe panicked — isolated and swallowed (delivery/ACK unaffected). An observation sink bug."
                );
            }
        }
    }

    // ── Envelope format global state (ADR-0096·0103) ──────────────────────────────────────────

    /// ★Defense against unknown values (fold-unknown = Xml)★: storage is only by set_envelope_format so it is always 0/1, but defensively only 1 maps to Colon and everything else to Xml — the production normal value is xml (the derived Default 0 goes to Xml via this branch).
    // ADR-0103
    pub fn envelope_format(&self) -> EnvelopeFormat {
        match self.envelope_format.load(Ordering::Acquire) {
            1 => EnvelopeFormat::Colon,
            _ => EnvelopeFormat::Xml,
        }
    }

    /// The UI language the shell last sent — English until one does.
    pub(crate) fn ui_language(&self) -> crate::ask_user::Lang {
        match self.ui_language.load(Ordering::Acquire) {
            1 => crate::ask_user::Lang::Ko,
            _ => crate::ask_user::Lang::En,
        }
    }

    /// Records the shell's `ui.language` word; a word this daemon does not know is English.
    pub(crate) fn set_ui_language(&self, code: &str) {
        let lang = crate::ask_user::Lang::from_code(code).unwrap_or_default();
        let v = match lang {
            crate::ask_user::Lang::En => 0u8,
            crate::ask_user::Lang::Ko => 1u8,
        };
        self.ui_language.store(v, Ordering::Release);
        tracing::debug!(language = lang.code(), "the shell set the UI language");
    }

    pub fn set_envelope_format(&self, format: EnvelopeFormat) {
        let v = match format {
            EnvelopeFormat::Xml => 0u8,
            EnvelopeFormat::Colon => 1u8,
        };
        self.envelope_format.store(v, Ordering::Release);
        tracing::info!(?format, "envelope format global state switched (ADR-0096)");
    }

    #[cfg(feature = "test-harness")]
    pub fn set_mid_send_hook(&self, hook: Option<Arc<dyn Fn() + Send + Sync>>) {
        // ★Lock discipline (spirit of ADR-0006)★: plug in the new hook and drop the old hook Arc outside the lock — dropped under the lock, the old hook's destructor (a captured Drop) deadlocks on re-entering the registry or poisons the lock if it panics. `_old` drops at function end, after the guard scope.
        let _old = {
            let mut guard = self.mid_send_hook.write().expect("mid-send hook poisoned");
            std::mem::replace(&mut *guard, hook)
        };
    }

    /// ★Lock discipline (ADR-0006)★: clone the hook Arc under a short read lock, release it, and call the hook outside the lock — foreign code is never called while holding the registry lock.
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

/// The dialogs' language source — the language the shell last sent.
// ADR-0372
impl crate::ask_user::LanguageSource for ControlRegistry {
    fn language(&self) -> crate::ask_user::Lang {
        self.ui_language()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mail permission is not the concern of tests using this helper — set to the mail-open side (a non-MCP spawn). Tests on that axis call `issue` directly.
    fn tok(reg: &ControlRegistry, id: AgentId, epoch: u32) -> String {
        // Deterministic test token (real provision uses a CSPRNG); only uniqueness is kept.
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

    /// ★Mail permission is fixed into the credential and fixed again on rotation (ADR-0133 decision 2)★: if re-activation changes the channel, the new token carries the new decision — keeping the old one would run an agent respawned on a different backend with the old permission.
    #[test]
    fn issue_records_the_mail_verdict_and_rotation_replaces_it() {
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        reg.issue(id, 0, "mail-off".to_string(), false);
        assert!(
            !reg.validate("mail-off").expect("valid").mail_allowed,
            "false at issue means validation is false too"
        );
        reg.issue(id, 1, "mail-on".to_string(), true);
        assert!(reg.validate("mail-on").expect("valid").mail_allowed);
        assert!(
            reg.validate("mail-off").is_none(),
            "rotation discards the old token (and its decision)"
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
        assert!(
            reg.is_identity_live(ident0),
            "the identity is live right after issue"
        );

        tok(&reg, id, 1);
        assert!(
            !reg.is_identity_live(ident0),
            "after rotation the old epoch identity is dead (F3)"
        );
        assert!(
            reg.is_identity_live(BoundIdentity {
                agent_id: id,
                epoch: 1
            }),
            "the new epoch identity is live"
        );

        reg.revoke(id, 1);
        assert!(
            !reg.is_identity_live(BoundIdentity {
                agent_id: id,
                epoch: 1
            }),
            "after revoke the identity is dead (F3)"
        );

        assert!(
            !reg.is_identity_live(BoundIdentity {
                agent_id: AgentId::new_v4(),
                epoch: 0
            }),
            "an unissued identity is dead"
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
            "a rotated old epoch token is discarded (stale) — validate None"
        );
        let bound = reg.validate(&new).expect("the new token is valid");
        assert_eq!(bound.identity.epoch, 1);
        assert_eq!(reg.live_token_count(), 1, "one AgentId = one live token");
    }

    #[test]
    fn revoke_matching_epoch_removes_token() {
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        let t = tok(&reg, id, 3);
        reg.revoke(id, 3);
        assert!(
            reg.validate(&t).is_none(),
            "the token is invalid after kill revoke"
        );
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
            "a stale epoch 0 revoke must not delete the live epoch 1 token (epoch-guard)"
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
            "the first binding succeeds"
        );
        assert_eq!(reg.identity_for_session("sess-abc"), Some(ident));
        assert!(reg.identity_for_session("other").is_none());
    }

    #[test]
    fn bind_if_absent_no_overwrite_pins_identity() {
        // ★Identity pinning (FIX 7)★ — registry-side foundation against cross-token takeover.
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
            "an already bound session is not overwritten (pinning)"
        );
        assert_eq!(
            reg.identity_for_session("sess"),
            Some(ident_a),
            "the existing identity A must stay"
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
            "a dead token's session must not be bound (exact-token recheck)"
        );
        assert!(reg.identity_for_session("sess").is_none());
    }

    #[test]
    fn bind_if_absent_rejects_stale_token_after_same_agent_reissue() {
        // ★F2 regression★: issue can be called again with the same (id,epoch), so this reproduces without bumping the epoch — proving it is safe even if the epoch-always-bumps invariant breaks.
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
            "reissue changed the current token"
        );
        assert_eq!(
            reg.bind_session_if_absent("sess", ident, &stale),
            None,
            "after reissue a stale token's session must not be bound (exact-token F2)"
        );
        assert!(reg.identity_for_session("sess").is_none());
        // Control group.
        let cur = current_token(&reg, id);
        assert_eq!(
            reg.bind_session_if_absent("sess", ident, &cur),
            Some(ident),
            "binding with the current credential token succeeds"
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
        assert_eq!(reg.bound_session_count(), 0, "binding removed after DELETE");
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
            "revoke deletes the session binding too"
        );
        assert_eq!(reg.bound_session_count(), 0);
    }

    // ── ADR-0096·0103: envelope format global state ────────────────────────────────────────────
    #[test]
    fn the_ui_language_is_english_until_the_shell_sets_it_and_unknown_words_are_english() {
        let reg = ControlRegistry::new();
        assert_eq!(reg.ui_language(), crate::ask_user::Lang::En);
        reg.set_ui_language("ko");
        assert_eq!(reg.ui_language(), crate::ask_user::Lang::Ko);
        reg.set_ui_language("fr");
        assert_eq!(reg.ui_language(), crate::ask_user::Lang::En);
    }

    #[test]
    fn envelope_format_defaults_to_xml_and_toggles() {
        let reg = ControlRegistry::new();
        assert_eq!(
            reg.envelope_format(),
            EnvelopeFormat::Xml,
            "a new registry's default envelope format must be xml (ADR-0103 flip)"
        );
        reg.set_envelope_format(EnvelopeFormat::Colon);
        assert_eq!(
            reg.envelope_format(),
            EnvelopeFormat::Colon,
            "after set(Colon) it must read as colon (residual switch)"
        );
        reg.set_envelope_format(EnvelopeFormat::Xml);
        assert_eq!(
            reg.envelope_format(),
            EnvelopeFormat::Xml,
            "after set(Xml) it returns to xml"
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
