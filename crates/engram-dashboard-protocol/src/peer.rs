//! The peer state the app's PC list slot shows — the wire copy of the daemon's `peer.list` answer, carried by
//!   [`crate::AgentEvent::PeerStateUpdated`] to connections that subscribed ([`crate::AgentCommand::PeerSubscribe`]).
//!
//! ★Same field names and words as `peer.list`★ (the daemon's `PeerListOk` in `link/peer/commands.rs`): the slot reads one shape
//!   whether it came from this broadcast or from its own `peer.list` call, and a daemon test holds the two serializations equal.
//! ★The word fields are not raised to enums★ (the `QueuedInputRow` rule): when the daemon grows a word, an old app shows that one
//!   row with an unknown word instead of failing to read the whole state.
//! Daemon ↔ daemon vocabulary is not here (it lives in the daemon — ADR-0310 D2) — only what the app shows.

// ADR-0394
use ts_rs::TS;

/// The whole peer area — `peer.list` without a filter.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct PeerState {
    /// ready | not_started | no_identity | key_unreadable | key_transient | adopt_or_fresh | identity_in_use | list_refused |
    /// list_changed_outside | list_corrupt | unavailable.
    pub state: String,
    pub reason: Option<String>,
    /// This PC's ID (its full text — the app groups it for reading).
    pub id: Option<String>,
    /// This PC's direct `ip:port` addresses another PC can type in.
    pub addresses: Vec<String>,
    pub network: Option<PeerStateNetwork>,
    /// off | up | error.
    pub endpoint: String,
    pub endpoint_error: Option<String>,
    pub pcs: Vec<PeerStatePc>,
    pub knocks: Vec<PeerStateKnock>,
    pub requests: Vec<PeerStateRequest>,
    /// What a human must do — sentences for an LLM reader; the app shows its own text per state.
    pub actions: Vec<String>,
}

/// This PC's network settings.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct PeerStateNetwork {
    /// The fixed listen port; absent = the build's default (`listen_port` says which).
    #[ts(type = "number | null")]
    pub port: Option<u16>,
    #[ts(type = "number")]
    pub listen_port: u16,
    /// default | none | a URL.
    pub relay: String,
    /// default | none | a URL.
    pub address_lookup: String,
    pub mdns: bool,
}

/// One trusted PC.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct PeerStatePc {
    pub id: String,
    /// Chosen by a human or an agent's approved request — render as text only.
    pub name: String,
    pub address: Option<String>,
    /// online | offline | refused | version_mismatch | duplicate_identity | endpoint_error.
    pub online: String,
    pub warning: Option<String>,
    /// Its agent list here: connected | disconnected | unknown (ADR-0391). Absent from a daemon built before it (read as empty).
    #[serde(default)]
    pub directory: String,
    /// Its agents, sorted by name — names and ids are that PC's, render as text only. Absent from a daemon built before them. The
    /// daemon's broadcast leaves them empty (`peer.list --agents false`) until the app shows them.
    #[serde(default)]
    pub agents: Vec<PeerStateAgent>,
}

/// An agent on a trusted PC, as that PC's directory lists it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct PeerStateAgent {
    pub name: String,
    pub id: String,
    /// live | sleeping.
    pub state: String,
}

/// A PC refused at the accept gate this run (not on the trusted list).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct PeerStateKnock {
    pub id: String,
    #[ts(type = "number")]
    pub count: u64,
    #[ts(type = "number")]
    pub last_at_ms: i64,
}

/// One request waiting for a human's answer — an agent's trust change or a question the peer state raised.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct PeerStateRequest {
    /// The id `peer.approve` / `peer.deny` take.
    pub request: String,
    /// add | setNetwork | regenerateKey | rename | acceptList | adopt.
    pub kind: String,
    /// One line saying what is asked (outside strings quoted by the daemon) — render as text only.
    pub summary: String,
    /// The requesting agent — absent for a question the peer state raised.
    pub agent_id: Option<String>,
    /// Agent-supplied — render as text only.
    pub agent_name: Option<String>,
    /// The PC whose mail most recently reached that agent (a hint, not a boundary).
    pub origin: Option<String>,
    #[ts(type = "number")]
    pub asked_at_ms: i64,
    #[ts(type = "number | null")]
    pub expires_in_s: Option<u64>,
    /// The question as the daemon's approval dialog shows it, in the UI language the shell told the daemon: title, heading and
    /// the body's lines (an empty line separates paragraphs). Daemon-written; outside strings in it are already quoted and made
    /// visible — render as text only. Absent from a daemon built before them (read as empty).
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub heading: String,
    #[serde(default)]
    pub lines: Vec<String>,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn sample() -> PeerState {
        PeerState {
            state: "ready".into(),
            reason: None,
            id: Some("abc".into()),
            addresses: vec!["192.168.0.2:47120".into()],
            network: Some(PeerStateNetwork {
                port: None,
                listen_port: 47120,
                relay: "default".into(),
                address_lookup: "none".into(),
                mdns: true,
            }),
            endpoint: "up".into(),
            endpoint_error: None,
            pcs: vec![PeerStatePc {
                id: "def".into(),
                name: "office".into(),
                address: None,
                online: "online".into(),
                warning: None,
                directory: "connected".into(),
                agents: vec![PeerStateAgent {
                    name: "worker".into(),
                    id: "a1".into(),
                    state: "live".into(),
                }],
            }],
            knocks: vec![PeerStateKnock {
                id: "ghi".into(),
                count: 2,
                last_at_ms: 1_700_000_000_000,
            }],
            requests: vec![PeerStateRequest {
                request: "0123456789ab".into(),
                kind: "add".into(),
                summary: "add PC \"office\"".into(),
                agent_id: Some("a1".into()),
                agent_name: Some("worker".into()),
                origin: None,
                asked_at_ms: 1_700_000_000_000,
                expires_in_s: Some(86_000),
                title: "Engram".into(),
                heading: "An agent asks to trust another PC".into(),
                lines: vec![
                    "ID: abcd".into(),
                    String::new(),
                    "Request 0123456789ab.".into(),
                ],
            }],
            actions: vec![],
        }
    }

    #[test]
    fn peer_state_json_golden_and_roundtrip() {
        let json = serde_json::to_value(sample()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "state": "ready",
                "reason": null,
                "id": "abc",
                "addresses": ["192.168.0.2:47120"],
                "network": {
                    "port": null,
                    "listen_port": 47120,
                    "relay": "default",
                    "address_lookup": "none",
                    "mdns": true
                },
                "endpoint": "up",
                "endpoint_error": null,
                "pcs": [{
                    "id": "def",
                    "name": "office",
                    "address": null,
                    "online": "online",
                    "warning": null,
                    "directory": "connected",
                    "agents": [{ "name": "worker", "id": "a1", "state": "live" }]
                }],
                "knocks": [{ "id": "ghi", "count": 2, "last_at_ms": 1_700_000_000_000_i64 }],
                "requests": [{
                    "request": "0123456789ab",
                    "kind": "add",
                    "summary": "add PC \"office\"",
                    "agent_id": "a1",
                    "agent_name": "worker",
                    "origin": null,
                    "asked_at_ms": 1_700_000_000_000_i64,
                    "expires_in_s": 86000,
                    "title": "Engram",
                    "heading": "An agent asks to trust another PC",
                    "lines": ["ID: abcd", "", "Request 0123456789ab."]
                }],
                "actions": []
            })
        );
        let back: PeerState = serde_json::from_value(json).unwrap();
        assert_eq!(back, sample());
    }
}
