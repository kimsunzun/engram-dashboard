//! Owner registry — filled **by runtime registration alone**, and when an owner disconnects, its share vanishes without a trace
//! (TRD §3-7 · ADR-0150).

use std::collections::{BTreeMap, BTreeSet};

use crate::{CommandDecl, CommandError, ErrorCode, OwnerToken};

/// A registry entry for one name — the registry holds **only names that have an owner** (ADR-0150).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterEntry {
    pub name: String,
    /// The shape the registration packet carried. ★An opaque string★ — no parsing, validation or branching (ADR-0156).
    pub help: String,
    pub owner: OwnerToken,
}

/// The two answers when asked about one name.
///
/// ★No "the owner is away" answer★ — a disconnected owner's registration vanishes, so its name answers like one **never seen** (accepted
///   in ADR-0150; reason in [`Roster`]).
/// If the distinction is ever needed, put the connection list in the response instead of reviving the trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnerLookup {
    Available(OwnerToken),
    Unknown,
}

/// An endpoint that asks only "who is the owner of this name" — all [`crate::route`] needs from the registry.
///
/// ★Keeps delivery from holding the registry **for the whole round trip**★: passing the shared registry by reference to `route` would hold
///   the lock until the reply arrives,
/// halting registration, cleanup and every other delivery behind one slow peer. Implementations take the lock **only within this call**.
pub trait OwnerLookupSource: Send + Sync {
    fn lookup(&self, name: &str) -> OwnerLookup;
}

impl OwnerLookupSource for Roster {
    fn lookup(&self, name: &str) -> OwnerLookup {
        Roster::lookup(self, name)
    }
}

/// ★Does not consult the build list★ — only when the registry is filled by runtime registration alone is "can it be called now" a fact
///   (TRD §3-7 prohibition clause).
///
/// ★When an owner disconnects, its entries vanish — no trace, no marker★ ([`Roster::disconnect`]). Do not revert to leaving a trace: the
///   owner token is new per connection (`OwnerToken`),
/// so a reconnecting client is a stranger and traces **pile up** with no expiry or reclaim; at [`Roster::MAX_NAMES`] every new name is
///   rejected as `UNKNOWN_COMMAND`
/// ("do not retry", TRD §4-②) until a daemon restart (ADR-0150 decision 3).
/// ★That alone does not bound the size★ — a name is a **string** from the packet, so one attached owner can send any number; count and
///   length caps close that axis
/// ([`Roster::MAX_NAMES_PER_OWNER`] · [`Roster::MAX_NAMES`] · [`Roster::MAX_NAME_BYTES`] · [`Roster::MAX_HELP_BYTES`] ·
///   [`Roster::MAX_AGENT_ARGS`]).
/// ★One name has one owner★ — the later registrant wins; the earlier owner loses the name for good, even after the later one disconnects.
///   Several owners per name is deferred (ADR-0150 alternative G).
// ADR-0156
// ADR-0150
#[derive(Debug, Default)]
pub struct Roster {
    /// Name → the owner that registered that name **last**. A disconnected owner's entries do not remain.
    entries: BTreeMap<String, Registration>,
}

#[derive(Debug, Clone)]
struct Registration {
    owner: OwnerToken,
    help: String,
    /// The agent-reference fields the registration packet marked — delivery reads them before relaying ([`Roster::agent_args`]).
    agent_args: Vec<String>,
}

impl Roster {
    /// The number of names one owner can hold **cumulatively** — everything already in the registry under that owner plus this packet
    ///   ([`Roster::check_room_for`]).
    ///
    /// ★A name is a **string** from the packet★ — without a cap one faulty owner could grow the registry without bound while attached.
    /// Value basis: the whole vocabulary is a bit over 60 (screen 33 · agent 5 · daemon 3 · `src-tauri` 17), the largest owner 33; 512 =
    ///   eight times that.
    pub const MAX_NAMES_PER_OWNER: usize = 512;

    /// The number of names in the whole registry — it stops here even with many owners (a per-owner cap alone would be bypassed by the
    ///   number of owners).
    pub const MAX_NAMES: usize = 4096;

    /// The maximum bytes of one name. Basis: the longest now is `agentlist.createTerminal` (24 bytes); 128 = a bit over five times that.
    pub const MAX_NAME_BYTES: usize = 128;

    /// The maximum bytes of the `help` field. ★Resident while its owner is attached★ — an uncapped `help` eats the registry's memory.
    /// Basis: the largest now is `agent.spawn` at 591 bytes (`agent/bindings/commands.schema.json`); 4096 = seven times that. With the
    ///   name and agent-reference caps the resident cap is
    /// [`Roster::MAX_NAMES`] × (128 + 4096 + 8 × 128) ≈ 20 MiB.
    pub const MAX_HELP_BYTES: usize = 4096;

    /// The number of agent-reference fields one name can mark (ADR-0308); a single field name is bounded by [`Roster::MAX_NAME_BYTES`].
    ///
    /// ★Also a resident packet string★ — without a cap it would bypass the `help` cap. Basis: at most three now (`slot.assignAgent` ·
    ///   `layout.setSlotContent`: `assign`, `agent_id`, `slot_of`); 8 = a bit over twice that.
    pub const MAX_AGENT_ARGS: usize = 8;

    pub fn new() -> Self {
        Self::default()
    }

    /// All at once on attach — that owner's **entire set**. A reconnect resend takes the same path, so an old name not carried this time
    ///   comes down (per-owner last-wins, ADR-0081 "RegisterRole resend");
    /// an empty list leaves that owner's place completely empty.
    ///
    /// ★If the cap is exceeded, **not one name is entered** and `INVALID_ARGUMENT` is returned★ — entering the rest would read as success
    ///   while some names silently go out as `UNKNOWN_COMMAND`.
    /// The rejection comes **before the registry is touched**, so a failed registration changes nothing.
    /// ★Taking a name held by another is legitimate here★ — registration is tied to **the moment of attaching** (TRD §3-7 clause 1), so
    ///   inheriting an old connection's names is the handover; [`Roster::update`] has no such anchor.
    /// ★A late registration from a disconnected owner is **not blocked here**★ — the registry keeps no trace of that owner, so it cannot
    ///   tell it from a live one. The net lives **in one place only**, the side that knows
    /// connection lifetime (`CommandRoster::refuse_if_detached` in `engram-dashboard-daemon`); a second net here would need per-owner
    ///   state that grows without bound like a trace.
    // ADR-0150
    pub fn register(
        &mut self,
        owner: &OwnerToken,
        decls: Vec<CommandDecl>,
    ) -> Result<(), CommandError> {
        self.check_room_for(owner, &decls)?;
        // It is an entire-set declaration, so this owner's names not carried this time come down along with their slot.
        self.remove_owner(owner);
        for decl in decls {
            self.entries.insert(
                decl.name,
                Registration {
                    owner: owner.clone(),
                    help: decl.help,
                    agent_args: decl.agent_args,
                },
            );
        }
        Ok(())
    }

    /// The **diff** while attached — a late-starting feature adds names, a switched-off one takes names down (TRD §3-7 clause 3). A full
    ///   resend is [`Roster::register`] only.
    ///
    /// ★`removed` **erases** the name, leaving no trace★: a trace would let a connected owner repeat add/remove and permanently fill its
    ///   own share ([`Roster::MAX_NAMES_PER_OWNER`]) — the other half of the axis ADR-0150 decision 3 closed for disconnects.
    /// ★`removed` erases **only names it holds itself**★ (as [`Roster::disconnect`]): a late diff from a past owner must not erase the
    ///   registration of the owner that just attached (reconnect overlap).
    /// Another owner's or a nonexistent name passes silently — a diff means "change my share", not a claim about others.
    /// ★`added` cannot take a name held by another — where it parts from [`Roster::register`]★: last-wins there is anchored to the moment
    ///   of attaching; a diff is "while attached", so a late diff from a dead connection
    /// would **overwrite the just-attached shell's registration with its dead token**, lookups would answer `Available` for the dead
    ///   owner, and the live shell never re-registers (once per attach) — frozen.
    /// ★Why rejected rather than ignored★: a silently ignored `added` would **tell the caller success** while it believes the name is its
    ///   own. The rejection is `CONFLICT` — the registry's **state** is not so, not that the packet is wrong (`INVALID_ARGUMENT`).
    /// ★A diff from a disconnected owner is **not blocked here**★ — same reason and same single net as [`Roster::register`].
    /// ★A packet carrying one name in both `added` and `removed` is **rejected whole with `INVALID_ARGUMENT`**★ — no application order is
    ///   fixed: add-then-remove leaves the name absent **after a success**
    /// (the accident the cap rejection of [`Roster::register`] blocks), remove-then-add silently ignores the take-down. **A success report
    ///   that diverges from actual state is worse than picking the wrong order**;
    /// neither ADR-0150 nor TRD §3-7 clause 3 fixes an order, so none is invented here.
    ///
    /// The cap is checked again counting only `added` — if exceeded, **not one name is touched** (same contract as [`Roster::register`]).
    // ADR-0150
    pub fn update(
        &mut self,
        owner: &OwnerToken,
        added: Vec<CommandDecl>,
        removed: Vec<String>,
    ) -> Result<(), CommandError> {
        // Pass the length caps first — the rejections below quote the name in their message (same order and
        //   same reason as inside `check_room_for`).
        self.check_room_for(owner, &added)?;
        Self::check_lists_are_disjoint(&added, &removed)?;
        self.check_added_are_not_taken(owner, &added)?;
        for decl in added {
            self.entries.insert(
                decl.name,
                Registration {
                    owner: owner.clone(),
                    help: decl.help,
                    agent_args: decl.agent_args,
                },
            );
        }
        for name in removed {
            if self
                .entries
                .get(&name)
                .is_some_and(|reg| &reg.owner == owner)
            {
                self.entries.remove(&name);
            }
        }
        Ok(())
    }

    /// Whether one name is carried in both lists — checked **before the registry is touched** (same contract as
    ///   [`Roster::check_room_for`]); why it is rejected: [`Roster::update`].
    ///
    /// ★Does not look at the registry — takes no `self`★: it judges **one packet**, hence `INVALID_ARGUMENT` (the same round trip gets the
    ///   same answer); its sibling
    /// [`Roster::check_added_are_not_taken`] speaks about registry **state**, hence `CONFLICT`.
    /// ★Iterates over `added`★ — the quoted name must have passed [`Roster::MAX_NAME_BYTES`], and only `decls` has its length measured; an
    ///   overlapping name is also in `added`.
    fn check_lists_are_disjoint(
        added: &[CommandDecl],
        removed: &[String],
    ) -> Result<(), CommandError> {
        let dropped: BTreeSet<&str> = removed.iter().map(|name| name.as_str()).collect();
        for decl in added {
            if dropped.contains(decl.name.as_str()) {
                return Err(CommandError::invalid_argument(format!(
                    "'{}' is in both added and removed — this packet does not say which one wins",
                    decl.name
                )));
            }
        }
        Ok(())
    }

    /// Whether `added` **overwrites another's registration** — the only entries that remain in the registry have an owner (ADR-0150), so
    ///   it is decided by
    /// one owner comparison.
    fn check_added_are_not_taken(
        &self,
        owner: &OwnerToken,
        decls: &[CommandDecl],
    ) -> Result<(), CommandError> {
        for decl in decls {
            let taken = self
                .entries
                .get(&decl.name)
                .is_some_and(|reg| &reg.owner != owner);
            if taken {
                return Err(CommandError::of(
                    ErrorCode::Conflict,
                    format!("'{}' is registered to another owner", decl.name),
                ));
            }
        }
        Ok(())
    }

    /// Whether this registration is within the cap — checked **before the registry is touched**.
    ///
    /// ★The per-owner cap measures that owner's **cumulative** total, not one packet★: otherwise one owner sending non-overlapping
    ///   512-name diffs would fill the whole registry, the real shell's registration
    /// would be blocked by the overall cap, and `tab.*` / `window.*` would go out as `UNKNOWN_COMMAND` (TRD §4-②, "retrying is pointless")
    ///   so callers give up on commands that exist.
    /// Names carried in this packet are overwrites and are not counted twice.
    /// ★A different axis from the rejected TTL★ — that was **time**, so the answer varied with the clock (ADR-0156 alternative C ·
    ///   ADR-0150 alternative F); a count cap does not change the answer for a name in the registry.
    /// ★The decision comes before the change, so this **over-measures** (left knowingly)★: names about to come down in this round trip
    ///   ([`Roster::register`]'s own replaced names, `removed` of [`Roster::update`]) are still counted.
    /// The cost is only that an owner with a full share cannot replace its entire set in one packet; one empty full registration opens
    ///   that path.
    /// Measuring exactly would force the decision to model the change and blur "decide before touching".
    ///
    /// Names and `help` are strings from the packet, so length is measured too.
    fn check_room_for(
        &self,
        owner: &OwnerToken,
        decls: &[CommandDecl],
    ) -> Result<(), CommandError> {
        for decl in decls {
            // Measure the name first — that way the `help` error below can quote the name safely.
            if decl.name.len() > Self::MAX_NAME_BYTES {
                return Err(CommandError::invalid_argument(format!(
                    "a command name may be at most {} bytes (got {})",
                    Self::MAX_NAME_BYTES,
                    decl.name.len()
                )));
            }
            if decl.help.len() > Self::MAX_HELP_BYTES {
                return Err(CommandError::invalid_argument(format!(
                    "the shape sent with '{}' may be at most {} bytes (got {})",
                    decl.name,
                    Self::MAX_HELP_BYTES,
                    decl.help.len()
                )));
            }
            if decl.agent_args.len() > Self::MAX_AGENT_ARGS {
                return Err(CommandError::invalid_argument(format!(
                    "'{}' may mark at most {} agent-reference arguments (got {})",
                    decl.name,
                    Self::MAX_AGENT_ARGS,
                    decl.agent_args.len()
                )));
            }
            if decl
                .agent_args
                .iter()
                .any(|arg| arg.len() > Self::MAX_NAME_BYTES)
            {
                return Err(CommandError::invalid_argument(format!(
                    "an agent-reference argument name marked by '{}' may be at most {} bytes",
                    decl.name,
                    Self::MAX_NAME_BYTES
                )));
            }
        }
        let names: BTreeSet<&str> = decls.iter().map(|d| d.name.as_str()).collect();
        let kept = self
            .entries
            .iter()
            .filter(|(name, reg)| &reg.owner == owner && !names.contains(name.as_str()))
            .count();
        let held = kept + names.len();
        if held > Self::MAX_NAMES_PER_OWNER {
            return Err(CommandError::invalid_argument(format!(
                "a single owner may hold at most {} command names ({kept} still on the roster from this owner, {} in this packet)",
                Self::MAX_NAMES_PER_OWNER,
                names.len()
            )));
        }
        let added = names
            .iter()
            .filter(|name| !self.entries.contains_key(**name))
            .count();
        if self.entries.len() + added > Self::MAX_NAMES {
            return Err(CommandError::invalid_argument(format!(
                "the roster holds at most {} command names ({} in use, {added} new)",
                Self::MAX_NAMES,
                self.entries.len()
            )));
        }
        Ok(())
    }

    /// The connection was cut — **erase the entries remaining under that owner along with their slot.** No trace or marker is left, so
    ///   those names answer like never-registered ones ([`OwnerLookup`]).
    ///
    /// ★Takes down **only names the owner still owns**★ — in a reconnect overlap (new connection registers, then the old one's cleanup
    ///   arrives) it must not erase the just-attached owner's registration.
    /// ★The only place that handles disconnects★ — the only caller is the daemon's `on_disconnect → detach` (ADR-0150); a second would
    ///   split causality.
    /// ★Return = **all names this call erased** (ascending)★ — nothing else records the event, so this is the only value that can say what
    ///   disappeared. Instrumentation belongs to the side that knows
    /// connection lifetime (`CommandRoster::detach` in `engram-dashboard-daemon`); this crate takes no logging dependency (`lib.rs` header).
    // ADR-0150
    pub fn disconnect(&mut self, owner: &OwnerToken) -> Vec<String> {
        self.remove_owner(owner)
    }

    pub fn lookup(&self, name: &str) -> OwnerLookup {
        match self.entries.get(name) {
            Some(reg) => OwnerLookup::Available(reg.owner.clone()),
            None => OwnerLookup::Unknown,
        }
    }

    /// The agent-reference fields the owner of this name marked (registration order). Unknown name · no marking = empty slice.
    // ADR-0308
    pub fn agent_args(&self, name: &str) -> &[String] {
        self.entries
            .get(name)
            .map(|reg| reg.agent_args.as_slice())
            .unwrap_or(&[])
    }

    pub fn entries(&self) -> impl Iterator<Item = RosterEntry> + '_ {
        self.entries.iter().map(|(name, reg)| RosterEntry {
            name: name.clone(),
            help: reg.help.clone(),
            owner: reg.owner.clone(),
        })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Return = the names taken down (ascending — `BTreeMap` iteration order). [`Roster::register`] does not use that value
    /// (an entire-set declaration replacing its own place is not an event but the definition of that call).
    fn remove_owner(&mut self, owner: &OwnerToken) -> Vec<String> {
        let mut removed = Vec::new();
        self.entries.retain(|name, reg| {
            if &reg.owner == owner {
                removed.push(name.clone());
                false
            } else {
                true
            }
        });
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(name: &str) -> CommandDecl {
        CommandDecl {
            name: name.to_string(),
            help: format!("{{\"name\":\"{name}\"}}"),
            agent_args: Vec::new(),
        }
    }

    /// ★Reconnect overlap — an old connection's cleanup must not take down a new registration★
    ///
    /// The owner token is new per connection, so the new registration comes first and the old cleanup arrives late; per-name takedown
    ///   would silently vanish the just-attached shell's commands.
    #[test]
    fn a_superseded_owners_cleanup_does_not_take_down_the_current_registration() {
        let old = OwnerToken::new("shell-conn-1");
        let new = OwnerToken::new("shell-conn-2");
        let mut roster = Roster::new();
        roster
            .register(&old, vec![decl("tab.create")])
            .expect("within the cap");
        roster
            .register(&new, vec![decl("tab.create")])
            .expect("within the cap");

        roster.disconnect(&old);
        assert_eq!(
            roster.lookup("tab.create"),
            OwnerLookup::Available(new.clone()),
            "the owner just registered must answer"
        );

        roster.disconnect(&new);
        assert_eq!(roster.lookup("tab.create"), OwnerLookup::Unknown);
    }

    #[test]
    fn the_latest_registration_answers_while_both_are_up() {
        let first = OwnerToken::new("shell-1");
        let second = OwnerToken::new("shell-2");
        let mut roster = Roster::new();
        roster
            .register(&first, vec![decl("tab.create")])
            .expect("within the cap");
        roster
            .register(&second, vec![decl("tab.create")])
            .expect("within the cap");

        assert_eq!(roster.lookup("tab.create"), OwnerLookup::Available(second));
    }

    /// ★Nothing accumulates in the registry however often it reconnects★ — the owner token is new per connection, so in the form that left
    ///   a trace, this repetition
    /// permanently filled the registry up to the cap (ADR-0150 decision 3).
    #[test]
    fn reconnecting_many_times_leaves_nothing_behind() {
        let mut roster = Roster::new();
        for connection in 0..50 {
            let owner = OwnerToken::new(format!("shell-conn-{connection}"));
            roster
                .register(&owner, vec![decl("tab.create"), decl("tab.close")])
                .expect("within the cap");
            roster.disconnect(&owner);
        }

        assert!(roster.is_empty(), "nothing accumulates");
        assert_eq!(roster.entries().count(), 0);
        assert_eq!(roster.lookup("tab.create"), OwnerLookup::Unknown);
    }

    /// The `help` of the owner that registered later is the shape of that name — one name has one entry.
    #[test]
    fn the_surviving_entry_carries_the_latest_shape() {
        let first = OwnerToken::new("shell-1");
        let second = OwnerToken::new("shell-2");
        let mut roster = Roster::new();
        roster
            .register(&first, vec![decl("tab.create")])
            .expect("within the cap");
        roster
            .register(
                &second,
                vec![CommandDecl {
                    name: "tab.create".to_string(),
                    help: "{\"name\":\"tab.create\",\"since\":2}".to_string(),
                    agent_args: Vec::new(),
                }],
            )
            .expect("within the cap");

        assert_eq!(roster.len(), 1);
        let entry = roster.entries().next().expect("one item");
        assert_eq!(entry.help, "{\"name\":\"tab.create\",\"since\":2}");
        assert_eq!(entry.owner, second);
    }

    #[test]
    fn never_seen_name_is_unknown() {
        assert_eq!(Roster::new().lookup("tab.create"), OwnerLookup::Unknown);
    }

    fn decls(prefix: &str, count: usize) -> Vec<CommandDecl> {
        (0..count).map(|i| decl(&format!("{prefix}.{i}"))).collect()
    }

    /// ★An attached owner cannot grow the registry without bound★ — a name is a string the registration packet sent, and until that owner
    /// disconnects there is no reclaim, so the cap is the only brake.
    #[test]
    fn a_registration_over_the_per_owner_cap_is_refused_whole() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();

        let err = roster
            .register(&owner, decls("flood", Roster::MAX_NAMES_PER_OWNER + 1))
            .expect_err("exceeds the cap");

        assert_eq!(err.code(), crate::ErrorCode::InvalidArgument);
        assert!(
            roster.is_empty(),
            "a failed registration does not touch the registry"
        );
        roster
            .register(&owner, decls("fits", Roster::MAX_NAMES_PER_OWNER))
            .expect("the cap itself passes");
        assert_eq!(roster.len(), Roster::MAX_NAMES_PER_OWNER);
    }

    /// ★The cap decision comes before the change, so it also counts the owner's own names this packet will push out★
    ///
    /// An over-measure left knowingly (basis = `check_room_for`): the cost is limited to "a full-share owner cannot replace its entire set
    ///   in one packet", and **one empty full registration opens that path**.
    #[test]
    fn a_full_owners_disjoint_snapshot_is_measured_before_the_old_names_come_down() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(&owner, decls("wave0", Roster::MAX_NAMES_PER_OWNER))
            .expect("the first snapshot is within the cap");

        let err = roster
            .register(&owner, decls("wave1", Roster::MAX_NAMES_PER_OWNER))
            .expect_err("a second, non-overlapping snapshot exceeds the cap cumulatively");

        assert_eq!(err.code(), crate::ErrorCode::InvalidArgument);
        assert_eq!(
            roster.len(),
            Roster::MAX_NAMES_PER_OWNER,
            "a failed registration does not touch the registry"
        );

        roster
            .register(&owner, vec![])
            .expect("empty full registration");
        assert!(
            roster.is_empty(),
            "when the full declaration is empty, its own spot empties too"
        );
        roster
            .register(&owner, decls("wave1", Roster::MAX_NAMES_PER_OWNER))
            .expect("once the spot is empty, a new full set comes in");
        assert_eq!(roster.len(), Roster::MAX_NAMES_PER_OWNER);
    }

    /// On disconnect that owner's share is completely empty — the names holding the cap vanish along with their slot (ADR-0150 decision 3).
    #[test]
    fn a_disconnect_frees_that_owners_whole_cap() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(&owner, decls("wave0", Roster::MAX_NAMES_PER_OWNER))
            .expect("within the cap");
        roster.disconnect(&owner);

        roster
            .register(&owner, decls("wave1", Roster::MAX_NAMES_PER_OWNER))
            .expect("nothing is holding the spot");

        assert_eq!(roster.len(), Roster::MAX_NAMES_PER_OWNER);
        assert_eq!(roster.lookup("wave0.0"), OwnerLookup::Unknown);
    }

    /// ★Names and `help` are strings from the packet★ — unmeasured length would let one exaggerated `help` eat the registry memory.
    #[test]
    fn an_oversized_name_or_help_is_refused_whole() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();

        let at_cap = CommandDecl {
            name: "n".repeat(Roster::MAX_NAME_BYTES),
            help: "h".repeat(Roster::MAX_HELP_BYTES),
            agent_args: Vec::new(),
        };
        roster
            .register(&owner, vec![at_cap])
            .expect("the cap itself passes");
        assert_eq!(roster.len(), 1);

        let long_name = CommandDecl {
            name: "n".repeat(Roster::MAX_NAME_BYTES + 1),
            help: "{}".to_string(),
            agent_args: Vec::new(),
        };
        let err = roster
            .register(&owner, vec![long_name])
            .expect_err("the name exceeds the cap");
        assert_eq!(err.code(), crate::ErrorCode::InvalidArgument);

        let long_help = CommandDecl {
            name: "tab.create".to_string(),
            help: "h".repeat(Roster::MAX_HELP_BYTES + 1),
            agent_args: Vec::new(),
        };
        let err = roster
            .register(&owner, vec![decl("tab.close"), long_help])
            .expect_err("help exceeds the cap");
        assert_eq!(err.code(), crate::ErrorCode::InvalidArgument);

        assert_eq!(
            roster.len(),
            1,
            "a failed registration does not touch the registry"
        );
        assert_eq!(roster.lookup("tab.close"), OwnerLookup::Unknown);
    }

    fn marked(name: &str, agent_args: &[&str]) -> CommandDecl {
        CommandDecl {
            agent_args: agent_args.iter().map(|arg| arg.to_string()).collect(),
            ..decl(name)
        }
    }

    // ── Agent-reference fields (ADR-0308) ───────────────────────────────────────────────────

    /// The value delivery reads before relaying — registration · diff · re-registration all swap that name's marking.
    #[test]
    fn agent_args_follow_register_and_update() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(
                &owner,
                vec![marked("slot.focus", &["agent", "after"]), decl("tab.close")],
            )
            .expect("within the cap");

        assert_eq!(roster.agent_args("slot.focus"), ["agent", "after"]);
        assert!(roster.agent_args("tab.close").is_empty(), "no marks");
        assert!(roster.agent_args("never.seen").is_empty(), "unknown name");

        roster
            .update(
                &owner,
                vec![
                    marked("slot.focus", &["target"]),
                    marked("tab.move", &["agent"]),
                ],
                vec!["tab.close".to_string()],
            )
            .expect("diff of its own name");
        assert_eq!(
            roster.agent_args("slot.focus"),
            ["target"],
            "the diff swaps the marks"
        );
        assert_eq!(roster.agent_args("tab.move"), ["agent"]);

        roster
            .register(&owner, vec![decl("slot.focus")])
            .expect("full re-registration");
        assert!(
            roster.agent_args("slot.focus").is_empty(),
            "if the re-registration dropped the marks, they are gone"
        );
        assert!(roster.agent_args("tab.move").is_empty(), "removed name");

        roster.disconnect(&owner);
        assert!(
            roster.agent_args("slot.focus").is_empty(),
            "a disconnected owner's marks do not remain"
        );
    }

    /// The marking fields are also resident packet strings — count and length are measured, and if exceeded the packet is rejected whole.
    #[test]
    fn oversized_agent_args_are_refused_whole() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();

        let at_cap: Vec<String> = (0..Roster::MAX_AGENT_ARGS)
            .map(|i| format!("{i}{}", "a".repeat(Roster::MAX_NAME_BYTES - 1)))
            .collect();
        roster
            .register(
                &owner,
                vec![CommandDecl {
                    agent_args: at_cap,
                    ..decl("slot.focus")
                }],
            )
            .expect("the cap itself passes");

        let too_many: Vec<&str> = vec!["a"; Roster::MAX_AGENT_ARGS + 1];
        let err = roster
            .register(
                &owner,
                vec![decl("tab.close"), marked("tab.move", &too_many)],
            )
            .expect_err("the field count exceeds the cap");
        assert_eq!(err.code(), crate::ErrorCode::InvalidArgument);

        let too_long = "a".repeat(Roster::MAX_NAME_BYTES + 1);
        let err = roster
            .update(
                &owner,
                vec![marked("tab.move", &[too_long.as_str()])],
                vec![],
            )
            .expect_err("the field name exceeds the cap");
        assert_eq!(err.code(), crate::ErrorCode::InvalidArgument);

        assert_eq!(
            roster.len(),
            1,
            "a failed registration · diff does not touch the registry"
        );
        assert_eq!(
            roster.agent_args("slot.focus").len(),
            Roster::MAX_AGENT_ARGS
        );
    }

    /// A per-owner cap alone is bypassed by the number of owners — the overall cap closes that axis.
    #[test]
    fn the_total_cap_holds_across_owners() {
        let mut roster = Roster::new();
        let owners = Roster::MAX_NAMES / Roster::MAX_NAMES_PER_OWNER;
        for i in 0..owners {
            let owner = OwnerToken::new(format!("shell-{i}"));
            roster
                .register(
                    &owner,
                    decls(&format!("owner{i}"), Roster::MAX_NAMES_PER_OWNER),
                )
                .expect("within the total cap");
        }
        assert_eq!(roster.len(), Roster::MAX_NAMES);

        let newcomer = OwnerToken::new("shell-late");
        let err = roster
            .register(&newcomer, vec![decl("one.more")])
            .expect_err("exceeds the total cap");
        assert_eq!(err.code(), crate::ErrorCode::InvalidArgument);
        assert_eq!(roster.len(), Roster::MAX_NAMES, "the registry is unchanged");

        // ★A name that already exists does not take a new slot★ — re-registration must pass even in a full registry.
        roster
            .register(&newcomer, vec![decl("owner0.0")])
            .expect("an overwrite does not increase the size");
        assert_eq!(roster.len(), Roster::MAX_NAMES);
    }

    /// Carrying the same name several times still takes one slot — duplicates do not burn the cap.
    #[test]
    fn duplicate_names_in_one_packet_count_once() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        let repeated = vec![decl("tab.create"); Roster::MAX_NAMES_PER_OWNER + 8];

        roster.register(&owner, repeated).expect("it is one name");

        assert_eq!(roster.len(), 1);
    }

    /// ★The registration side of the vanished line of defense — the registry does not block a disconnected owner's late **registration** either★
    ///
    /// Registration inherits even a live owner's names legitimately (handover), and the registry cannot tell a dead connection's late
    ///   registration from one. If it passes, the name becomes `Available`
    /// under a **nonexistent owner** and stays frozen for the daemon's lifetime (delivery over a cut link). The net is **in one place
    ///   only** — `CommandRoster::refuse_if_detached` in `engram-dashboard-daemon`.
    #[test]
    fn the_roster_alone_cannot_refuse_a_late_registration_from_a_disconnected_owner() {
        let old = OwnerToken::new("shell-conn-1");
        let new = OwnerToken::new("shell-conn-2");
        let mut roster = Roster::new();
        roster
            .register(&old, vec![decl("tab.create")])
            .expect("within the cap");
        roster.disconnect(&old);
        roster
            .register(&new, vec![decl("tab.create")])
            .expect("a live owner puts that name anew");

        roster
            .register(&old, vec![decl("tab.create")])
            .expect("the registry has no basis to bounce it");

        assert_eq!(
            roster.lookup("tab.create"),
            OwnerLookup::Available(old),
            "it stands addressed to a nonexistent owner — the daemon is the only place that prevents this"
        );
    }

    // ── Diff registration (TRD §3-7 clause 3) ────────────────────────────────────────────────────────────

    /// ★A diff erases only `removed`★ — if it left a trace, an owner that is still connected would repeat add/remove and permanently fill
    ///   its own share
    /// (ADR-0150). Not touching names that are not carried is where it parts from `register`'s
    /// entire-set last-wins.
    #[test]
    fn an_update_removes_only_the_names_it_lists() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(&owner, vec![decl("tab.create"), decl("tab.close")])
            .expect("within the cap");

        roster
            .update(
                &owner,
                vec![decl("tab.split")],
                vec!["tab.close".to_string()],
            )
            .expect("within the cap");

        assert_eq!(
            roster.lookup("tab.create"),
            OwnerLookup::Available(owner.clone()),
            "names not carried in the diff are unchanged"
        );
        assert_eq!(roster.lookup("tab.split"), OwnerLookup::Available(owner));
        assert_eq!(roster.lookup("tab.close"), OwnerLookup::Unknown);
        assert_eq!(
            roster.len(),
            2,
            "a removed name vanishes along with its spot"
        );
    }

    /// ★A diff's `removed` empties the place★ — a trace would let an attached owner fill its own share cap by changing names (ADR-0150).
    #[test]
    fn swapping_names_forever_does_not_eat_an_owners_cap() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(&owner, decls("wave", Roster::MAX_NAMES_PER_OWNER))
            .expect("fills the share completely");

        for round in 0..8 {
            roster
                .update(
                    &owner,
                    vec![],
                    vec![format!("wave.{}", Roster::MAX_NAMES_PER_OWNER - 1 - round)],
                )
                .expect("removing always passes");
            roster
                .update(&owner, vec![decl(&format!("swap.{round}"))], vec![])
                .expect("the spot is empty, so it can be added");
        }

        assert_eq!(
            roster.len(),
            Roster::MAX_NAMES_PER_OWNER,
            "swapping keeps the size the same"
        );
    }

    /// ★Reconnect overlap — the same net must stand for the diff too★ — an old connection's late diff must not take down names the new
    ///   connection just registered (as `disconnect`).
    #[test]
    fn an_update_from_a_superseded_owner_does_not_take_down_the_current_registration() {
        let old = OwnerToken::new("shell-conn-1");
        let new = OwnerToken::new("shell-conn-2");
        let mut roster = Roster::new();
        roster
            .register(&old, vec![decl("tab.create")])
            .expect("within the cap");
        roster
            .register(&new, vec![decl("tab.create")])
            .expect("within the cap");

        roster
            .update(&old, vec![], vec!["tab.create".to_string()])
            .expect("within the cap");

        assert_eq!(
            roster.lookup("tab.create"),
            OwnerLookup::Available(new),
            "the owner just registered must answer"
        );
    }

    /// ★Reconnect overlap — the same net must stand on the `added` side too★
    ///
    /// Mirror of the test above: an old connection's late diff **adding** the same name would overwrite the entry with a dead token,
    ///   lookups would answer the dead owner as `Available`, and the live shell never re-registers.
    #[test]
    fn an_update_from_a_superseded_owner_does_not_add_over_the_current_registration() {
        let old = OwnerToken::new("shell-conn-1");
        let new = OwnerToken::new("shell-conn-2");
        let mut roster = Roster::new();
        roster
            .register(&old, vec![decl("tab.create")])
            .expect("within the cap");
        roster
            .register(&new, vec![decl("tab.create")])
            .expect("within the cap");

        let err = roster
            .update(&old, vec![decl("tab.create")], vec![])
            .expect_err("cannot take a live other's registration");

        assert_eq!(err.code(), crate::ErrorCode::Conflict);
        assert_eq!(
            roster.lookup("tab.create"),
            OwnerLookup::Available(new),
            "the owner just registered must answer"
        );
    }

    /// ★If one name is carried in both `added` and `removed`, it is rejected **whole packet**★ — either application order leaves the
    ///   caller unable to know the result
    /// (add-then-remove: absent **after a success**; remove-then-add: take-down silently ignored); no clause of ADR-0150 or TRD §3-7
    ///   clause 3 fixes an order.
    #[test]
    fn an_update_carrying_one_name_in_both_lists_is_refused_whole() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(&owner, vec![decl("tab.create")])
            .expect("within the cap");

        // A name already held — under the old application order it was added then erased and **vanished along with a success**.
        let err = roster
            .update(
                &owner,
                vec![decl("tab.create")],
                vec!["tab.create".to_string()],
            )
            .expect_err("the same name was carried on both sides");
        assert_eq!(err.code(), crate::ErrorCode::InvalidArgument);
        assert!(
            err.message().contains("tab.create"),
            "the message must say which name was caught: {}",
            err.message()
        );
        assert_eq!(
            roster.lookup("tab.create"),
            OwnerLookup::Available(owner.clone()),
            "a bounce does not touch the registry"
        );

        // A new name — here 「add then erase」 would not create a name that never existed from the start either, but the decision is the same.
        let err = roster
            .update(
                &owner,
                vec![decl("tab.split")],
                vec!["tab.split".to_string()],
            )
            .expect_err("the same name was carried on both sides");
        assert_eq!(err.code(), crate::ErrorCode::InvalidArgument);
        assert_eq!(roster.lookup("tab.split"), OwnerLookup::Unknown);
        assert_eq!(roster.len(), 1);
    }

    /// Returns **what the disconnect erased** — no trace remains in the registry (ADR-0150), so this is the only value that reconstructs
    ///   the event; instrumentation is the daemon's (`CommandRoster::detach`).
    #[test]
    fn a_disconnect_reports_the_names_it_removed() {
        let owner = OwnerToken::new("shell-1");
        let other = OwnerToken::new("shell-2");
        let mut roster = Roster::new();
        roster
            .register(&owner, vec![decl("tab.create"), decl("tab.close")])
            .expect("within the cap");
        roster
            .register(&other, vec![decl("window.focus")])
            .expect("within the cap");

        let removed = roster.disconnect(&owner);

        assert_eq!(
            removed,
            vec!["tab.close".to_string(), "tab.create".to_string()],
            "only names it holds are carried"
        );
        assert!(
            roster.disconnect(&owner).is_empty(),
            "the second cleanup has nothing to erase"
        );
        assert_eq!(
            roster.lookup("window.focus"),
            OwnerLookup::Available(other),
            "others' registrations are unchanged"
        );
    }

    /// A name the disconnected owner held becomes a name **nobody holds** — it does not take away a live registration, so the diff
    /// takes it as is.
    #[test]
    fn an_update_may_claim_a_name_whose_owner_has_disconnected() {
        let old = OwnerToken::new("shell-conn-1");
        let new = OwnerToken::new("shell-conn-2");
        let mut roster = Roster::new();
        roster
            .register(&old, vec![decl("tab.create")])
            .expect("within the cap");
        roster.disconnect(&old);
        roster
            .register(&new, vec![decl("tab.close")])
            .expect("within the cap");
        assert_eq!(roster.lookup("tab.create"), OwnerLookup::Unknown);

        roster
            .update(&new, vec![decl("tab.create")], vec![])
            .expect("a name without an owner can be taken");

        assert_eq!(roster.lookup("tab.create"), OwnerLookup::Available(new));
    }

    /// The cap is measured **cumulatively** for a diff too, and if exceeded not one name enters.
    #[test]
    fn an_update_over_the_per_owner_cap_is_refused_whole() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(&owner, decls("wave0", Roster::MAX_NAMES_PER_OWNER))
            .expect("within the cap");

        let err = roster
            .update(&owner, vec![decl("one.more")], vec![])
            .expect_err("the names already held use up the share");

        assert_eq!(err.code(), crate::ErrorCode::InvalidArgument);
        assert_eq!(roster.len(), Roster::MAX_NAMES_PER_OWNER);
        assert_eq!(roster.lookup("one.more"), OwnerLookup::Unknown);
    }

    /// ★On disconnect it vanishes without a trace★ — the owner token is new per connection, so a trace would **pile up** per reconnect up
    ///   to the cap (ADR-0150 decision 3).
    #[test]
    fn a_disconnect_empties_that_owners_share_of_the_roster() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(&owner, vec![decl("tab.create"), decl("tab.close")])
            .expect("within the cap");

        roster.disconnect(&owner);

        assert!(roster.is_empty(), "no item remains");
        assert_eq!(roster.entries().count(), 0);
        assert_eq!(
            roster.lookup("tab.create"),
            OwnerLookup::Unknown,
            "the same answer as an unknown name"
        );
    }

    /// A reconnect is a **completely new entry** — old names the new connection did not carry remain nowhere.
    #[test]
    fn a_reconnect_registers_as_a_wholly_new_entry() {
        let old = OwnerToken::new("shell-conn-1");
        let new = OwnerToken::new("shell-conn-2");
        let mut roster = Roster::new();
        roster
            .register(&old, vec![decl("tab.create"), decl("tab.close")])
            .expect("within the cap");
        roster.disconnect(&old);

        roster
            .register(&new, vec![decl("tab.create")])
            .expect("within the cap");

        assert_eq!(
            roster.len(),
            1,
            "the rest of the old connection's names do not remain"
        );
        assert_eq!(roster.entries().next().expect("one item").owner, new);
        assert_eq!(roster.lookup("tab.close"), OwnerLookup::Unknown);
    }

    /// ★Pin down the vanished line of defense — the registry **cannot block** a disconnected owner's late packet★
    ///
    /// With the trace gone the registry cannot remember the owner's death, so a late diff passes and the name becomes `Available` under a
    ///   **nonexistent owner**, frozen for the daemon's lifetime.
    /// The net is in the side that knows connection lifetime, **one place only** — `CommandRoster::refuse_if_detached` in
    ///   `engram-dashboard-daemon`.
    /// Do not put a second net here: it would add another unboundedly growing per-owner list (why the trace was discarded — ADR-0150).
    #[test]
    fn the_roster_alone_cannot_refuse_a_late_delta_from_a_disconnected_owner() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(&owner, vec![decl("tab.create")])
            .expect("within the cap");
        roster.disconnect(&owner);

        roster
            .update(&owner, vec![decl("tab.split")], vec![])
            .expect("the registry has no basis to bounce it");

        assert_eq!(
            roster.lookup("tab.split"),
            OwnerLookup::Available(owner),
            "it stands addressed to a nonexistent owner — the daemon is the only place that prevents this"
        );
    }

    #[test]
    fn reregistration_is_last_wins_and_drops_stale_names() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(&owner, vec![decl("tab.create"), decl("tab.legacy")])
            .expect("within the cap");
        roster
            .register(&owner, vec![decl("tab.create")])
            .expect("within the cap");

        assert_eq!(
            roster.lookup("tab.create"),
            OwnerLookup::Available(owner.clone())
        );
        assert_eq!(roster.lookup("tab.legacy"), OwnerLookup::Unknown);
    }
}
