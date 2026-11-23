//! The publisher's diff, deltas and one-frame snapshots, and the copies' run / rev rules, chosen links and row checks — pure, no link.

use super::*;
use crate::link::peer::testing::pc;
use crate::link::peer::wire::{AgentState, MAX_FRAME_BYTES};

fn agent(id: &str, name: &str, state: AgentState) -> DirAgent {
    DirAgent {
        id: id.to_string(),
        name: name.to_string(),
        state,
    }
}

fn live(id: &str, name: &str) -> DirAgent {
    agent(id, name, AgentState::Live)
}

fn run(n: u128) -> Instance {
    Instance::from_u128(n)
}

fn accept_all(_: &DirAgent) -> Option<&'static str> {
    None
}

fn snap(run: Instance, rev: u64, agents: Vec<DirAgent>) -> DirSnapshot {
    DirSnapshot { run, rev, agents }
}

/// Rows whose JSON text is about `width` bytes, in id order.
fn rows(count: usize, width: usize) -> Vec<DirAgent> {
    (0..count)
        .map(|n| {
            let id = format!("id-{n:06}");
            let name = format!("{n:06}-{}", "n".repeat(width.saturating_sub(48)));
            live(&id, &name)
        })
        .collect()
}

#[test]
fn a_change_is_one_delta_with_the_next_rev_and_no_change_sends_nothing() {
    let mut publisher = Publisher::new(run(1), MAX_FRAME_BYTES);
    let first = publisher.update(vec![live("a", "alice"), live("b", "bob")]);
    let Some(Publish::Delta(delta)) = first else {
        panic!("{first:?}")
    };
    assert_eq!(
        (delta.rev, delta.upsert.len(), delta.remove.len()),
        (1, 2, 0)
    );

    assert_eq!(
        publisher.update(vec![live("b", "bob"), live("a", "alice")]),
        None,
        "the same directory in another order is no change"
    );

    // bob goes to sleep, alice is renamed, carol is created, a is gone.
    let next = publisher.update(vec![
        agent("b", "bob", AgentState::Sleeping),
        live("c", "carol"),
    ]);
    let Some(Publish::Delta(delta)) = next else {
        panic!("{next:?}")
    };
    assert_eq!(delta.rev, 2);
    assert_eq!(delta.run, run(1));
    assert_eq!(
        delta.upsert,
        vec![agent("b", "bob", AgentState::Sleeping), live("c", "carol")]
    );
    assert_eq!(delta.remove, vec!["a".to_string()]);
}

/// ADR-0411: a delta that does not fit one frame goes as a snapshot — one frame, cut to the rows that fit (in id order), never parts.
#[test]
fn a_delta_over_the_frame_cap_goes_as_one_snapshot_cut_to_the_cap() {
    let all = rows(40, 80);
    let one_row = serde_json::to_string(&all[0]).unwrap().len();
    let empty = encoded_len(&PeerMsg::DirSnapshot(snap(run(1), u64::MAX, Vec::new()))).unwrap();
    // Room for about ten rows a frame.
    let cap = empty + 10 * (one_row + 1);
    let mut publisher = Publisher::new(run(1), cap);
    let published = publisher.update(all.clone());
    let Some(Publish::Snapshot(snapshot)) = published else {
        panic!("a 40-row delta does not fit — {published:?}")
    };
    assert_eq!(snapshot.rev, 1, "the snapshot is of the new rev");
    let size = encoded_len(&PeerMsg::DirSnapshot(snapshot.clone())).unwrap();
    assert!(size <= cap, "{size} bytes over the cap {cap}");
    let n = snapshot.agents.len();
    assert!((9..=10).contains(&n), "{n} rows fit");
    assert_eq!(snapshot.agents, all[..n], "the rows that fit, in id order");
    let (again, omitted) = publisher.fitting();
    assert_eq!(
        (again, omitted),
        (snapshot.clone(), 40 - n),
        "the rest are counted as left out"
    );
    assert_eq!(
        publisher.snapshot(),
        snapshot,
        "every link gets the same frame"
    );

    // A small change afterwards fits again.
    let mut fewer = all;
    fewer.pop();
    assert!(matches!(
        publisher.update(fewer),
        Some(Publish::Delta(DirDelta { rev: 2, .. }))
    ));
}

/// ADR-0411: a directory far over the old 512 KiB frame goes whole, as one frame under the 4 MiB cap.
#[test]
fn a_large_directory_is_one_whole_frame() {
    let all = rows(20_000, 100);
    let mut publisher = Publisher::new(run(1), MAX_FRAME_BYTES);
    publisher.update(all.clone());
    let snapshot = publisher.snapshot();
    let size = encoded_len(&PeerMsg::DirSnapshot(snapshot.clone())).unwrap();
    assert!(size > 512 << 10 && size <= MAX_FRAME_BYTES, "{size} bytes");
    assert_eq!(snapshot.agents, all);
    assert_eq!(publisher.fitting().1, 0, "nothing left out");
}

#[test]
fn an_empty_directory_is_an_empty_snapshot() {
    let mut publisher = Publisher::new(run(1), MAX_FRAME_BYTES);
    let snapshot = publisher.snapshot();
    assert_eq!(snapshot.rev, 0);
    assert!(snapshot.agents.is_empty());
}

/// R16: a row is id, name and state — no folder, parent, activity, and nothing about this PC (no address).
#[test]
fn a_published_row_carries_id_name_and_state_only() {
    let mut publisher = Publisher::new(run(1), MAX_FRAME_BYTES);
    publisher.update(vec![live("a", "alice")]);
    let text = serde_json::to_string(&PeerMsg::DirSnapshot(publisher.snapshot())).unwrap();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut keys: Vec<&String> = value["agents"][0].as_object().unwrap().keys().collect();
    keys.sort_unstable();
    assert_eq!(keys, ["id", "name", "state"]);
    let mut top: Vec<&String> = value.as_object().unwrap().keys().collect();
    top.sort_unstable();
    assert_eq!(top, ["agents", "rev", "run", "type"]);
}

#[test]
fn a_snapshot_replaces_the_copy_and_deltas_follow_it() {
    let mut copies = Copies::default();
    let node = pc(1);
    let mut publisher = Publisher::new(run(7), MAX_FRAME_BYTES);
    publisher.update(vec![live("b", "bob"), live("a", "alice")]);
    assert_eq!(
        copies.snapshot(node, publisher.snapshot(), &accept_all),
        Applied::Updated
    );
    let copy = copies.get(&node).unwrap();
    assert_eq!(copy.rev, 1);
    assert_eq!(
        copy.agents,
        vec![live("a", "alice"), live("b", "bob")],
        "sorted by name"
    );

    let Some(Publish::Delta(delta)) =
        publisher.update(vec![agent("a", "alice", AgentState::Sleeping)])
    else {
        unreachable!()
    };
    assert_eq!(copies.delta(node, delta, &accept_all), Applied::Updated);
    let copy = copies.get(&node).unwrap();
    assert_eq!(copy.rev, 2);
    assert_eq!(copy.agents, vec![agent("a", "alice", AgentState::Sleeping)]);
}

#[test]
fn a_rev_gap_or_a_foreign_run_asks_for_a_snapshot() {
    let mut copies = Copies::default();
    let node = pc(1);
    let delta = |run: Instance, rev| DirDelta {
        run,
        rev,
        upsert: vec![live("x", "xavier")],
        remove: Vec::new(),
    };
    assert_eq!(
        copies.delta(node, delta(run(7), 1), &accept_all),
        Applied::Resync,
        "no copy yet"
    );
    let snap = DirSnapshot {
        run: run(7),
        rev: 3,
        agents: vec![live("a", "alice")],
    };
    copies.snapshot(node, snap, &accept_all);
    assert_eq!(
        copies.delta(node, delta(run(7), 5), &accept_all),
        Applied::Resync,
        "a gap"
    );
    assert_eq!(
        copies.delta(node, delta(run(7), 3), &accept_all),
        Applied::Resync,
        "a repeat"
    );
    assert_eq!(
        copies.delta(node, delta(run(8), 4), &accept_all),
        Applied::Resync,
        "another run"
    );
    assert_eq!(
        copies.get(&node).unwrap().agents,
        vec![live("a", "alice")],
        "the copy is kept while waiting for the snapshot"
    );
    assert_eq!(
        copies.delta(node, delta(run(7), 4), &accept_all),
        Applied::Updated
    );
}

#[test]
fn an_older_snapshot_of_the_same_run_is_ignored_and_a_new_run_replaces_the_copy() {
    let mut copies = Copies::default();
    let node = pc(1);
    let snap = |run: Instance, rev, name: &str| DirSnapshot {
        run,
        rev,
        agents: vec![live("a", name)],
    };
    copies.snapshot(node, snap(run(7), 5, "new"), &accept_all);
    assert_eq!(
        copies.snapshot(node, snap(run(7), 4, "old"), &accept_all),
        Applied::Ignored
    );
    assert_eq!(copies.get(&node).unwrap().agents[0].name, "new");
    assert_eq!(
        copies.snapshot(node, snap(run(9), 0, "restarted"), &accept_all),
        Applied::Updated,
        "the publisher restarted"
    );
    let copy = copies.get(&node).unwrap();
    assert_eq!((copy.run, copy.rev), (run(9), 0));
    assert_eq!(copy.agents[0].name, "restarted");
}

/// Only the link the actor recorded as the PC's chosen one counts; a dropped PC has none.
#[test]
fn the_chosen_link_is_the_recorded_one() {
    let mut copies = Copies::default();
    assert!(!copies.is_chosen(&pc(1), 5), "nothing chosen yet");
    copies.set_chosen(HashMap::from([(pc(1), 5), (pc(2), 6)]));
    assert!(copies.is_chosen(&pc(1), 5));
    assert!(!copies.is_chosen(&pc(1), 6));
    copies.drop_node(&pc(1));
    assert!(!copies.is_chosen(&pc(1), 5));
    assert!(copies.is_chosen(&pc(2), 6));
}

/// The re-check of the copies drops the rows the host now refuses, per PC, and reports how many.
#[test]
fn rows_the_host_now_refuses_are_dropped_from_every_copy() {
    let mut copies = Copies::default();
    copies.snapshot(
        pc(1),
        snap(run(1), 1, vec![live("x", "x"), live("a", "a")]),
        &accept_all,
    );
    copies.snapshot(pc(2), snap(run(2), 1, vec![live("b", "b")]), &accept_all);
    let dropped = copies.drop_refused(&|rows| {
        rows.iter()
            .map(|row| (row.id == "x").then_some("a local agent id"))
            .collect()
    });
    assert_eq!(dropped, vec![(pc(1), 1)]);
    assert_eq!(copies.get(&pc(1)).unwrap().agents, vec![live("a", "a")]);
    assert_eq!(copies.get(&pc(2)).unwrap().agents, vec![live("b", "b")]);
}

#[test]
fn rows_the_host_refuses_are_dropped_and_a_refused_rename_drops_the_old_row() {
    let mut copies = Copies::default();
    let node = pc(1);
    let check = |row: &DirAgent| -> Option<&'static str> {
        if row.id == "mine" {
            Some("a local agent id")
        } else if row.name.is_empty() {
            Some("a name that breaks the remote name rule")
        } else {
            None
        }
    };
    copies.snapshot(
        node,
        DirSnapshot {
            run: run(1),
            rev: 1,
            agents: vec![live("mine", "me"), live("a", ""), live("b", "bob")],
        },
        &check,
    );
    assert_eq!(copies.get(&node).unwrap().agents, vec![live("b", "bob")]);
    copies.delta(
        node,
        DirDelta {
            run: run(1),
            rev: 2,
            upsert: vec![live("b", "")],
            remove: Vec::new(),
        },
        &check,
    );
    assert!(copies.get(&node).unwrap().agents.is_empty());
}

#[test]
fn an_id_two_pcs_list_is_kept_in_both_copies() {
    let mut copies = Copies::default();
    let snap = DirSnapshot {
        run: run(1),
        rev: 1,
        agents: vec![live("same", "twin")],
    };
    copies.snapshot(pc(1), snap.clone(), &accept_all);
    copies.snapshot(pc(2), snap, &accept_all);
    assert_eq!(copies.get(&pc(1)).unwrap().agents.len(), 1);
    assert_eq!(copies.get(&pc(2)).unwrap().agents.len(), 1);
}

#[test]
fn a_dropped_copy_is_unknown_again() {
    let mut copies = Copies::default();
    copies.snapshot(
        pc(1),
        DirSnapshot {
            run: run(1),
            rev: 0,
            agents: Vec::new(),
        },
        &accept_all,
    );
    assert!(copies.get(&pc(1)).is_some(), "an empty copy is known");
    copies.drop_node(&pc(1));
    assert!(copies.get(&pc(1)).is_none());
}
