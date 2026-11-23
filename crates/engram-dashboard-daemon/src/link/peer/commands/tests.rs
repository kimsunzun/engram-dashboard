//! The `peer.*` commands through the same order `call_daemon_command` drives them (argument check → approval gate → handler, first poll
//!   only), over fake trust operations, a fake dialog and a manual clock. The typed caller from the real entrances is pinned in
//!   `control::commands`' tests.

use std::sync::mpsc::Receiver;

use engram_dashboard_base::time::ManualClock;
use engram_dashboard_command::lint_spec;
use futures_util::FutureExt as _;
use serde_json::{json, Value};

use super::*;
use crate::ask_user::fakes::{FakeApp, FakeDialogs, FakeLanguage, Shown};
use crate::link::peer::identity::Unreadable;
use crate::link::peer::testing::{knock, not_started, pc, source, FakeOps};

struct Rig {
    commands: PeerCommands,
    ops: Arc<FakeOps>,
    _shown: Receiver<Shown>,
}

fn rig() -> Rig {
    let ops = Arc::new(FakeOps::default());
    let (dialogs, shown) = FakeDialogs::new();
    // The app shows the slot, so no dialog opens — these tests answer by command.
    let app = Arc::new(FakeApp::default());
    app.set_shows(true);
    let approvals = Approvals::start(
        source(ops.clone()),
        dialogs,
        app,
        Arc::new(FakeLanguage::default()),
        Arc::new(ManualClock::new()),
    );
    Rig {
        commands: PeerCommands::new(approvals),
        ops,
        _shown: shown,
    }
}

fn human() -> PeerCaller {
    PeerCaller::Human
}

fn agent() -> PeerCaller {
    PeerCaller::Agent(Requester {
        id: "6f1c2a90-0000-4000-8000-000000000001".to_string(),
        name: Some("lead".to_string()),
    })
}

/// What `call_daemon_command` does for a `peer.*` name: argument check, gate, then the handler — which must finish on its first poll.
fn run(rig: &Rig, name: &str, args: Value, caller: &PeerCaller) -> Result<Value, CommandError> {
    let mut args = args;
    rig.commands.check_args(name, &args)?;
    match rig.commands.call(name, &mut args, caller)? {
        PeerCall::Answered(answer) => Ok(answer),
        PeerCall::Run(future) => future
            .now_or_never()
            .expect("a peer.* handler finishes on its first poll"),
    }
}

fn id_text(byte: u8) -> String {
    pc(byte).to_text()
}

fn office(rig: &Rig) {
    run(
        rig,
        ADD,
        json!({ "id": id_text(0x11), "name": "office" }),
        &human(),
    )
    .expect("a human's add applies");
}

#[test]
fn the_table_declares_every_peer_command_and_each_declaration_lints() {
    let rig = rig();
    let mut names: Vec<&str> = rig.commands.specs().map(|spec| spec.name).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "peer.add",
            "peer.approve",
            "peer.deny",
            "peer.edit",
            "peer.list",
            "peer.regenerateKey",
            "peer.remove",
            "peer.setNetwork"
        ]
    );
    for spec in COMMAND_SPECS {
        lint_spec(spec).unwrap_or_else(|e| panic!("{}: {e}", spec.name));
    }
}

/// ADR-0394 d1: a human's gated call applies at once.
#[test]
fn a_human_add_applies_at_once() {
    let rig = rig();
    let answer = run(
        &rig,
        ADD,
        json!({ "id": id_text(0x11), "name": "office", "address": "192.168.0.7:47120" }),
        &human(),
    )
    .unwrap();
    assert_eq!(answer["outcome"], "applied");
    assert_eq!(rig.ops.calls(), ["add office"]);
    let added = rig.ops.state.lock().unwrap().peers[0].clone();
    assert_eq!(added.address, Some("192.168.0.7:47120".parse().unwrap()));
}

/// ADR-0394 d1 · d4: an agent's gated call records a request and is answered at once — nothing applied.
#[test]
fn an_agents_gated_calls_only_request_and_return_at_once() {
    let rig = rig();
    office(&rig);
    for (name, args) in [
        (ADD, json!({ "id": id_text(0x22), "name": "laptop" })),
        (SET_NETWORK, json!({ "relay": "none" })),
        (REGENERATE_KEY, json!({})),
        (EDIT, json!({ "name": "office", "new_name": "work" })),
    ] {
        let answer = run(&rig, name, args, &agent()).unwrap();
        assert_eq!(answer["outcome"], "requested", "{name}: {answer}");
        let request = answer["request"].as_str().expect("a request id");
        assert!(
            answer["message"].as_str().unwrap().starts_with(&format!(
                "requested — waiting for the user's approval (request {request})"
            )),
            "{answer}"
        );
    }
    assert_eq!(
        rig.ops.calls(),
        ["add office"],
        "nothing an agent asked for was applied"
    );
    let listed = run(&rig, LIST, json!({}), &agent()).unwrap();
    let kinds: Vec<&str> = listed["requests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["add", "setNetwork", "regenerateKey", "rename"]);
    assert_eq!(listed["requests"][0]["agent_name"], "lead");
}

/// ADR-0394 d1: an address edit and a removal are open to agents.
#[test]
fn an_agents_address_edit_and_removal_apply() {
    let rig = rig();
    office(&rig);
    let edited = run(
        &rig,
        EDIT,
        json!({ "name": "OFFICE", "address": "10.0.0.9:5000" }),
        &agent(),
    )
    .unwrap();
    assert_eq!(edited["outcome"], "applied");
    let removed = run(&rig, REMOVE, json!({ "id": id_text(0x11) }), &agent()).unwrap();
    assert_eq!(removed["saved"], true);
    assert_eq!(removed["name"], "office");
    assert_eq!(
        rig.ops.calls(),
        vec![
            "add office".to_string(),
            format!("edit {}", pc(0x11)),
            format!("remove {}", pc(0x11))
        ]
    );
}

/// ADR-0394 d1 · d7: approving and denying are human-only.
#[test]
fn an_agent_cannot_approve_or_deny() {
    let rig = rig();
    let request = run(&rig, REGENERATE_KEY, json!({}), &agent()).unwrap()["request"]
        .as_str()
        .unwrap()
        .to_string();
    for name in [APPROVE, DENY] {
        let refused = run(&rig, name, json!({ "request": request }), &agent()).unwrap_err();
        assert_eq!(refused.code(), ErrorCode::Conflict, "{name}");
        assert!(
            refused
                .message()
                .starts_with("only a human answers a request"),
            "{}",
            refused.message()
        );
    }
    assert!(rig.ops.calls().is_empty());
    let listed = run(&rig, LIST, json!({}), &agent()).unwrap();
    assert_eq!(
        listed["requests"].as_array().unwrap().len(),
        1,
        "still waiting"
    );
}

#[test]
fn a_human_approves_by_request_id_once() {
    let rig = rig();
    let request = run(
        &rig,
        ADD,
        json!({ "id": id_text(0x22), "name": "laptop" }),
        &agent(),
    )
    .unwrap()["request"]
        .as_str()
        .unwrap()
        .to_string();
    let approved = run(&rig, APPROVE, json!({ "request": request }), &human()).unwrap();
    assert_eq!(approved["outcome"], "applied");
    assert_eq!(rig.ops.calls(), ["add laptop"]);
    let again = run(&rig, APPROVE, json!({ "request": request }), &human()).unwrap_err();
    assert_eq!(again.code(), ErrorCode::NotFound, "{}", again.message());
}

#[test]
fn a_human_deny_with_stop_asking_refuses_that_agents_later_requests() {
    let rig = rig();
    let request = run(&rig, REGENERATE_KEY, json!({}), &agent()).unwrap()["request"]
        .as_str()
        .unwrap()
        .to_string();
    let denied = run(
        &rig,
        DENY,
        json!({ "request": request, "stop_asking": true }),
        &human(),
    )
    .unwrap();
    assert_eq!(denied["outcome"], "denied");
    let refused = run(&rig, SET_NETWORK, json!({ "mdns": false }), &agent()).unwrap_err();
    assert_eq!(refused.code(), ErrorCode::Conflict);
    assert!(
        refused.message().contains("until the daemon restarts"),
        "{}",
        refused.message()
    );
    assert!(rig.ops.calls().is_empty());
}

#[test]
fn a_ninth_agent_request_is_refused_with_a_hint_to_ask_the_user() {
    let rig = rig();
    for byte in 1..=MAX_PENDING as u8 {
        run(
            &rig,
            ADD,
            json!({ "id": id_text(byte), "name": format!("pc{byte}") }),
            &agent(),
        )
        .unwrap();
    }
    let refused = run(
        &rig,
        ADD,
        json!({ "id": id_text(0x40), "name": "pc40" }),
        &agent(),
    )
    .unwrap_err();
    assert_eq!(refused.code(), ErrorCode::Conflict);
    assert!(
        refused.message().contains("ask the user"),
        "{}",
        refused.message()
    );
}

/// ADR-0390 d4: the removal holds for the run even when the save is refused — and the answer says so.
#[test]
fn remove_reports_a_refused_save_and_the_removal_holds() {
    let rig = rig();
    office(&rig);
    rig.ops.with(|state| state.save_fails = true);
    let removed = run(&rig, REMOVE, json!({ "name": "office" }), &agent()).unwrap();
    assert_eq!(removed["saved"], false);
    let message = removed["message"].as_str().unwrap();
    assert!(
        message.contains("could not be saved (the disk is full)"),
        "{message}"
    );
    assert!(rig.ops.state.lock().unwrap().peers.is_empty(), "revoked");
}

#[test]
fn a_pc_is_named_by_exactly_one_of_name_and_id_and_a_miss_names_candidates() {
    let rig = rig();
    office(&rig);
    let both = run(
        &rig,
        REMOVE,
        json!({ "name": "office", "id": id_text(0x11) }),
        &human(),
    )
    .unwrap_err();
    assert_eq!(both.code(), ErrorCode::InvalidArgument);
    let neither = run(&rig, REMOVE, json!({}), &human()).unwrap_err();
    assert_eq!(neither.code(), ErrorCode::InvalidArgument);
    let missing = run(&rig, REMOVE, json!({ "name": "home" }), &human()).unwrap_err();
    assert_eq!(missing.code(), ErrorCode::NotFound);
    assert!(
        missing.message().ends_with("candidates: \"office\""),
        "{}",
        missing.message()
    );
}

#[test]
fn add_refuses_what_it_can_tell_is_wrong() {
    let rig = rig();
    office(&rig);
    let cases = [
        (
            json!({ "id": "not-an-id", "name": "x" }),
            ErrorCode::InvalidArgument,
            "id:",
        ),
        (
            json!({ "id": id_text(0xAA), "name": "me" }),
            ErrorCode::InvalidArgument,
            "this PC's own ID",
        ),
        (
            json!({ "id": id_text(0x33), "name": "a@b" }),
            ErrorCode::InvalidArgument,
            "name:",
        ),
        (
            json!({ "id": id_text(0x33), "name": "Office" }),
            ErrorCode::Conflict,
            "already used",
        ),
        (
            json!({ "id": id_text(0x11), "name": "other" }),
            ErrorCode::Conflict,
            "already trusted",
        ),
        (
            json!({ "id": id_text(0x33), "name": "x", "address": "nowhere" }),
            ErrorCode::InvalidArgument,
            "address:",
        ),
    ];
    for (args, code, wording) in cases {
        for caller in [human(), agent()] {
            let refused = run(&rig, ADD, args.clone(), &caller).unwrap_err();
            assert_eq!(refused.code(), code, "{args} {caller:?}");
            assert!(
                refused.message().contains(wording),
                "{args}: {}",
                refused.message()
            );
        }
    }
    let unknown_field = run(
        &rig,
        ADD,
        json!({ "id": id_text(0x33), "nmae": "x" }),
        &human(),
    )
    .unwrap_err();
    assert_eq!(
        unknown_field.code(),
        ErrorCode::InvalidArgument,
        "the entrance check runs first"
    );
    assert_eq!(rig.ops.calls(), ["add office"]);
}

#[test]
fn set_network_replaces_only_the_given_fields() {
    let rig = rig();
    run(
        &rig,
        SET_NETWORK,
        json!({ "port": "5000", "mdns": false }),
        &human(),
    )
    .unwrap();
    let network = rig.ops.state.lock().unwrap().network.clone();
    assert_eq!(network.port, Some(5000));
    assert!(!network.mdns);
    assert_eq!(network.relay, Helper::Default, "kept");
    run(
        &rig,
        SET_NETWORK,
        json!({ "port": "default", "relay": "https://relay.example.com" }),
        &human(),
    )
    .unwrap();
    let network = rig.ops.state.lock().unwrap().network.clone();
    assert_eq!(network.port, None);
    assert_eq!(
        network.relay,
        Helper::Url("https://relay.example.com".to_string())
    );
    assert!(!network.mdns, "kept");
    for args in [
        json!({}),
        json!({ "port": "0" }),
        json!({ "relay": "ftp://x" }),
    ] {
        let refused = run(&rig, SET_NETWORK, args.clone(), &human()).unwrap_err();
        assert_eq!(refused.code(), ErrorCode::InvalidArgument, "{args}");
    }
}

#[test]
fn regenerate_answers_the_new_id_and_says_every_pc_must_pair_again() {
    let rig = rig();
    let answer = run(&rig, REGENERATE_KEY, json!({}), &human()).unwrap();
    assert_eq!(answer["outcome"], "applied");
    assert_eq!(answer["id"], id_text(0xBB));
    let message = answer["message"].as_str().unwrap();
    assert!(message.contains(&pc(0xBB).grouped()), "{message}");
    assert!(
        message.contains("Every other PC must remove this PC's old ID"),
        "{message}"
    );
}

/// T18: one query for the area; the filter narrows it to one PC.
#[test]
fn peer_list_answers_the_whole_area_and_narrows_to_one_pc() {
    let rig = rig();
    office(&rig);
    run(
        &rig,
        ADD,
        json!({ "id": id_text(0x22), "name": "laptop" }),
        &human(),
    )
    .unwrap();
    rig.ops.with(|state| state.knocks.push(knock(pc(0x55))));
    run(&rig, SET_NETWORK, json!({ "relay": "none" }), &agent()).unwrap();

    let listed = run(&rig, LIST, json!({}), &agent()).unwrap();
    let keys: Vec<&str> = listed
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    let mut keys = keys;
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "actions",
            "addresses",
            "endpoint",
            "endpoint_error",
            "id",
            "knocks",
            "network",
            "pcs",
            "reason",
            "requests",
            "state"
        ]
    );
    assert_eq!(listed["state"], "ready");
    assert_eq!(listed["reason"], Value::Null);
    assert_eq!(listed["id"], id_text(0xAA));
    assert_eq!(listed["endpoint"], "off");
    assert_eq!(listed["network"]["relay"], "default");
    assert_eq!(listed["pcs"].as_array().unwrap().len(), 2);
    assert_eq!(listed["pcs"][0]["online"], "offline");
    assert_eq!(listed["knocks"][0]["id"], id_text(0x55));
    assert_eq!(listed["requests"][0]["kind"], "setNetwork");
    let actions: Vec<&str> = listed["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect();
    assert!(
        actions
            .iter()
            .any(|a| a.contains("owner confirms its ID another way")),
        "{actions:?}"
    );
    assert!(
        actions
            .iter()
            .any(|a| a.starts_with("1 request(s) from agents wait")),
        "{actions:?}"
    );

    let one = run(&rig, LIST, json!({ "name": "Laptop" }), &agent()).unwrap();
    assert_eq!(one["pcs"].as_array().unwrap().len(), 1);
    assert_eq!(one["pcs"][0]["name"], "laptop");
    assert!(one["knocks"].as_array().unwrap().is_empty());
    let missing = run(&rig, LIST, json!({ "id": id_text(0x77) }), &agent()).unwrap_err();
    assert_eq!(missing.code(), ErrorCode::NotFound);
}

/// Every state that needs a human shows in `peer.list` with the command that resolves it.
#[test]
fn peer_list_names_each_state_that_needs_a_human() {
    let rig = rig();
    let cases: [(PeerStatus, &str, &str); 4] = [
        (
            PeerStatus::KeyUnreadable(Unreadable::MissingUnderList),
            "key_unreadable",
            "peer.regenerateKey",
        ),
        (
            PeerStatus::ListChangedOutside,
            "list_changed_outside",
            "accepts the trusted list",
        ),
        (
            PeerStatus::AdoptOrFresh {
                machines: vec!["m".to_string()],
            },
            "adopt_or_fresh",
            "found under another machine id",
        ),
        (
            PeerStatus::IdentityInUse,
            "identity_in_use",
            "stop that copy",
        ),
    ];
    for (status, word, action) in cases {
        rig.ops.with(|state| state.status = status.clone());
        let listed = run(&rig, LIST, json!({}), &agent()).unwrap();
        assert_eq!(listed["state"], word);
        assert_eq!(listed["reason"], status.to_string());
        assert!(
            listed["actions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a.as_str().unwrap().contains(action)),
            "{word}: {}",
            listed["actions"]
        );
    }
    rig.ops.with(|state| {
        state.status = PeerStatus::Ready;
        state.endpoint = EndpointView::Error(EndpointError::HelperRefused("bad host".to_string()));
    });
    let listed = run(&rig, LIST, json!({}), &agent()).unwrap();
    assert_eq!(listed["endpoint"], "error");
    assert!(listed["endpoint_error"]
        .as_str()
        .unwrap()
        .contains("bad host"));
    assert!(listed["actions"][0]
        .as_str()
        .unwrap()
        .contains("peer.setNetwork"));
}

#[test]
fn my_addresses_show_a_bound_address_as_is() {
    let rig = rig();
    rig.ops.with(|state| {
        state.endpoint = EndpointView::Up {
            addrs: vec![
                "127.0.0.1:47121".parse().unwrap(),
                "127.0.0.1:47121".parse().unwrap(),
            ],
        }
    });
    let listed = run(&rig, LIST, json!({}), &agent()).unwrap();
    assert_eq!(listed["addresses"], json!(["127.0.0.1:47121"]));
    assert_eq!(listed["endpoint"], "up");
}

#[test]
fn before_the_peer_link_starts_list_says_so_and_changes_are_refused() {
    let (dialogs, _shown) = FakeDialogs::new();
    let approvals = Approvals::start(
        not_started(),
        dialogs,
        Arc::new(FakeApp::default()),
        Arc::new(FakeLanguage::default()),
        Arc::new(ManualClock::new()),
    );
    let rig = Rig {
        commands: PeerCommands::new(approvals),
        ops: Arc::new(FakeOps::default()),
        _shown,
    };
    let listed = run(&rig, LIST, json!({}), &human()).unwrap();
    assert_eq!(listed["state"], "not_started");
    for (name, args) in [
        (ADD, json!({ "id": id_text(1), "name": "x" })),
        (REGENERATE_KEY, json!({})),
    ] {
        for caller in [human(), agent()] {
            let refused = run(&rig, name, args.clone(), &caller).unwrap_err();
            assert_eq!(refused.code(), ErrorCode::Conflict, "{name}");
            assert!(refused
                .message()
                .starts_with("the peer link is not running"));
        }
    }
}

/// ADR-0394 d1 (B1): every declared name has an agent rule — a name without one would be refused for agents, and this pins that none
///   is missing.
#[test]
fn every_declared_name_has_an_agent_rule() {
    for spec in COMMAND_SPECS {
        assert!(
            agent_policy(spec.name).is_some(),
            "{} has no agent rule",
            spec.name
        );
    }
    assert_eq!(
        agent_policy("peer.somethingNew"),
        None,
        "an unknown name is not open"
    );
}

/// ADR-0389 d3: the first `peer.*` call of a PC without an identity makes it — `peer.list` then shows an ID to pair with.
#[test]
fn the_first_peer_call_creates_this_pcs_identity() {
    let rig = rig();
    rig.ops.with(|state| {
        state.status = PeerStatus::NoIdentityYet;
        state.id = None;
    });
    let listed = run(&rig, LIST, json!({}), &agent()).unwrap();
    assert_eq!(listed["state"], "ready");
    assert_eq!(listed["id"], id_text(0xAA));
    assert_eq!(rig.ops.calls(), ["ensureIdentity"]);
    run(&rig, LIST, json!({}), &agent()).unwrap();
    assert_eq!(rig.ops.calls(), ["ensureIdentity"], "made once");
}

/// A PC that only knocked can be looked at by its ID.
#[test]
fn peer_list_by_id_finds_a_pc_that_only_knocked() {
    let rig = rig();
    rig.ops.with(|state| state.knocks.push(knock(pc(0x55))));
    let one = run(&rig, LIST, json!({ "id": id_text(0x55) }), &agent()).unwrap();
    assert_eq!(one["knocks"][0]["id"], id_text(0x55));
    assert!(one["pcs"].as_array().unwrap().is_empty());
}

/// The adopt request's third answer through the command: `peer.approve --start_fresh true`; on any other request it is refused and the
///   request stays.
#[test]
fn approve_with_start_fresh_answers_the_adopt_request() {
    let rig = rig();
    let request = run(
        &rig,
        ADD,
        json!({ "id": id_text(0x22), "name": "laptop" }),
        &agent(),
    )
    .unwrap()["request"]
        .as_str()
        .unwrap()
        .to_string();
    let refused = run(
        &rig,
        APPROVE,
        json!({ "request": request, "start_fresh": true }),
        &human(),
    )
    .unwrap_err();
    assert_eq!(refused.code(), ErrorCode::InvalidArgument);
    rig.ops.with(|state| {
        state.status = PeerStatus::AdoptOrFresh {
            machines: vec!["m".to_string()],
        }
    });
    rig.commands.approvals.sync_state(&rig.ops.snapshot());
    let listed = run(&rig, LIST, json!({}), &human()).unwrap();
    let adopt = listed["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["kind"] == "adopt")
        .expect("the adopt request")["request"]
        .as_str()
        .unwrap()
        .to_string();
    let actions = listed["actions"].to_string();
    assert!(
        actions.contains(&format!("answers request {adopt}")),
        "{actions}"
    );
    let fresh = run(
        &rig,
        APPROVE,
        json!({ "request": adopt, "start_fresh": true }),
        &human(),
    )
    .unwrap();
    assert_eq!(fresh["id"], id_text(0xDD));
    assert!(rig.ops.calls().contains(&"startFresh".to_string()));
}

/// A held-back question is not pointed at: `actions` says the user put it off.
#[test]
fn actions_do_not_point_at_a_question_the_user_put_off() {
    let rig = rig();
    rig.ops
        .with(|state| state.status = PeerStatus::ListChangedOutside);
    rig.commands.approvals.sync_state(&rig.ops.snapshot());
    let listed = run(&rig, LIST, json!({}), &human()).unwrap();
    let request = listed["requests"][0]["request"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(listed["actions"].to_string().contains(&request));
    run(&rig, DENY, json!({ "request": request }), &human()).unwrap();
    let listed = run(&rig, LIST, json!({}), &human()).unwrap();
    let actions = listed["actions"].to_string();
    assert!(actions.contains("put the question off"), "{actions}");
    assert!(!actions.contains(&request), "{actions}");
}

/// An agent's network request with a server the link refuses is refused at once, not queued for the human.
#[test]
fn a_network_request_the_link_would_refuse_is_refused_at_once() {
    let rig = rig();
    let refused = run(
        &rig,
        SET_NETWORK,
        json!({ "relay": "https://xn--a.example" }),
        &agent(),
    )
    .unwrap_err();
    assert_eq!(
        refused.code(),
        ErrorCode::InvalidArgument,
        "{}",
        refused.message()
    );
    let listed = run(&rig, LIST, json!({}), &agent()).unwrap();
    assert!(listed["requests"].as_array().unwrap().is_empty());
}

/// E1: while the current settings are not known (the list file could not be read), a network request is refused — the question could
///   not show a true "from", and the approval could not tell what moved.
#[test]
fn a_network_request_is_refused_while_the_settings_are_unknown() {
    let rig = rig();
    rig.ops.with(|state| {
        state.status = PeerStatus::ListRefused(
            crate::link::peer::trusted::ListRefusal::ReadFailed("locked".to_string()),
        );
        state.network_unknown = true;
    });
    for caller in [agent(), human()] {
        let refused = run(&rig, SET_NETWORK, json!({ "relay": "none" }), &caller).unwrap_err();
        assert_eq!(refused.code(), ErrorCode::Conflict, "{caller:?}");
        assert!(
            refused
                .message()
                .starts_with("the current network settings are not known"),
            "{}",
            refused.message()
        );
    }
    assert!(run(&rig, LIST, json!({}), &agent()).unwrap()["requests"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(rig.ops.calls().is_empty());
}

/// ADR-0391: each PC's agents from its directory copy, sorted by name, with the copy's state; `--agents false` leaves them out.
#[test]
fn peer_list_shows_each_pcs_agents_and_leaves_them_out_on_request() {
    use crate::link::peer::book::Online;
    use crate::link::peer::wire::{AgentState, DirAgent};
    let rig = rig();
    office(&rig);
    for (byte, name) in [(0x22, "laptop"), (0x33, "nas")] {
        run(
            &rig,
            ADD,
            json!({ "id": id_text(byte), "name": name }),
            &human(),
        )
        .unwrap();
    }
    let row = |id: &str, name: &str, state| DirAgent {
        id: id.to_string(),
        name: name.to_string(),
        state,
    };
    rig.ops.with(|state| {
        state.online.insert(pc(0x11), Online::Online);
        state.agents.insert(
            pc(0x11),
            vec![
                row("id-a", "alice", AgentState::Live),
                row("id-b", "bob", AgentState::Sleeping),
            ],
        );
        state
            .agents
            .insert(pc(0x22), vec![row("id-c", "carol", AgentState::Live)]);
    });

    let listed = run(&rig, LIST, json!({}), &agent()).unwrap();
    let pcs = listed["pcs"].as_array().unwrap();
    assert_eq!(pcs[0]["directory"], "connected");
    assert_eq!(
        pcs[0]["agents"],
        json!([
            { "name": "alice", "id": "id-a", "state": "live" },
            { "name": "bob", "id": "id-b", "state": "sleeping" },
        ])
    );
    assert_eq!(
        pcs[1]["directory"], "disconnected",
        "a copy kept while its PC is offline"
    );
    assert_eq!(pcs[1]["agents"][0]["name"], "carol");
    assert_eq!(pcs[2]["directory"], "unknown", "no copy this run");
    assert_eq!(pcs[2]["agents"], json!([]));

    let bare = run(&rig, LIST, json!({ "agents": false }), &agent()).unwrap();
    assert!(bare["pcs"]
        .as_array()
        .unwrap()
        .iter()
        .all(|pc| pc["agents"] == json!([])));
    assert_eq!(
        bare["pcs"][0]["directory"], "connected",
        "the state stays without the rows"
    );
    let one = run(&rig, LIST, json!({ "name": "office" }), &agent()).unwrap();
    assert_eq!(one["pcs"].as_array().unwrap().len(), 1);
    assert_eq!(one["pcs"][0]["agents"].as_array().unwrap().len(), 2);
}
