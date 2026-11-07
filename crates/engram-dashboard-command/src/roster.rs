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
/// ★There is no answer 「주인이 자리 비움」 (the owner is away)★ — the disconnected owner's registration vanishes from the registry, so that name gets the same answer as a name
/// **never seen before** (what ADR-0150 accepted — the reason for leaving no trace is in [`Roster`]).
/// If that distinction is ever really needed, it is solved not by reviving the trace but by putting the connection list in the response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnerLookup {
    Available(OwnerToken),
    Unknown,
}

/// An endpoint that asks only one thing, 「이 이름의 주인은 누구인가」 (who is the owner of this name) — all that [`crate::route`] needs from the registry.
///
/// ★This is what keeps delivery from holding the registry **for the whole round trip**★: for the assembly code holding the shared registry to
/// pass a reference to `route`, it would have to hold the lock until the reply arrives, and meanwhile registration, connection cleanup and every other delivery all stop
/// (one slow peer halts the whole registry). If the implementation takes the lock **only within this call**, that halt disappears.
pub trait OwnerLookupSource: Send + Sync {
    fn lookup(&self, name: &str) -> OwnerLookup;
}

impl OwnerLookupSource for Roster {
    fn lookup(&self, name: &str) -> OwnerLookup {
        Roster::lookup(self, name)
    }
}

/// ★Does not consult the build list★ — only when the registry is filled by runtime registration alone is 「지금 부를 수 있는가」 (can it be called now) a fact
/// (TRD §3-7 prohibition clause).
///
/// ★When an owner disconnects, its entries vanish — leaving neither a trace nor a marker★ ([`Roster::disconnect`])
///
/// Do not revert to a form that leaves a trace. The owner token is new per connection (`OwnerToken`), so if the same client
/// reconnects it is a stranger to the registry, and so traces are not overwritten but **pile up**. With neither expiry nor a reclaim path,
/// once [`Roster::MAX_NAMES`] is reached every registration carrying a name not already in the registry is rejected, and — those names go out as
/// `UNKNOWN_COMMAND` (= 「재시도 말고 다시 발견하라」 (do not retry, rediscover), TRD §4-②) — nothing but a daemon restart clears it
/// (ADR-0150 decision 3).
/// ★But that alone does not bound the size★ — a name is a **string** the registration packet carried (not a build constant),
/// so a single attached owner can send any number of names. That axis is closed by count and length caps
/// ([`Roster::MAX_NAMES_PER_OWNER`] · [`Roster::MAX_NAMES`] · [`Roster::MAX_NAME_BYTES`] ·
/// [`Roster::MAX_HELP_BYTES`] · [`Roster::MAX_AGENT_ARGS`]).
/// ★One name has one owner★ — the later registrant wins. If two different owners claim the same name, the earlier
/// owner loses that name, and when the later owner disconnects, the name does not return to the earlier owner but vanishes. Several
/// owners for one name is a deferred agenda item (ADR-0150 alternative G).
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
    /// The number of names one owner can hold in the registry **cumulatively** — measured as everything already in the registry under that owner
    /// plus this packet, not the packet just sent ([`Roster::check_room_for`]).
    ///
    /// ★A name is a **string** the registration packet carried, not a build constant★ — without a cap, a single malfunctioning
    /// or malicious owner would grow the registry without bound while attached.
    /// Basis for the value: the whole current vocabulary is a bit over 60 (screen 33 · agent 5 · daemon 3 · `src-tauri` 17) and the largest
    /// owner has 33. 512 = eight times that — a spot that real use does not reach while still stopping runaway.
    pub const MAX_NAMES_PER_OWNER: usize = 512;

    /// The number of names in the whole registry — it stops here even with many owners (a per-owner cap alone would be bypassed by the number of owners).
    pub const MAX_NAMES: usize = 4096;

    /// The maximum bytes of one name.
    ///
    /// Basis for the value: the longest name that exists now is `agentlist.createTerminal` (24 bytes). 128 = a bit over five
    /// times that — not reached even if a family goes deeper, while closing the path of inflating the registry with a single name.
    pub const MAX_NAME_BYTES: usize = 128;

    /// The maximum bytes of the `help` field.
    ///
    /// ★This string stays resident while its owner is attached★ — without a cap, one exaggerated `help` eats the registry's
    /// whole memory.
    /// Basis for the value: the largest entry now is `agent.spawn` at 591 bytes (measured in `agent/bindings/commands.schema.json`).
    /// 4096 = seven times that, comfortably holding a declaration that unfolds nested structs a few more levels.
    /// Adding this cap to the name and agent-reference field caps closes the registry's resident cap at [`Roster::MAX_NAMES`] ×
    /// (128 + 4096 + 8 × 128) ≈ 20 MiB.
    pub const MAX_HELP_BYTES: usize = 4096;

    /// The number of agent-reference fields one name can mark (ADR-0308). The length of a single field name is
    /// bounded together by [`Roster::MAX_NAME_BYTES`].
    ///
    /// ★This field is also a string the registration packet carried, so it stays resident★ — without a cap, it would bypass the `help` cap to inflate the registry.
    /// Basis for the value: a command has at most three fields pointing at an agent now (`slot.assignAgent` · `layout.setSlotContent` —
    /// `assign`, the agent to put in; `agent_id`, its old name; `slot_of`, the slot pointer). 8 = a bit over twice that.
    pub const MAX_AGENT_ARGS: usize = 8;

    pub fn new() -> Self {
        Self::default()
    }

    /// All at once on attach — it is that owner's **entire set**. A reconnect resend takes the same path, so an old name this owner did not
    /// carry this time comes down from the registry (per-owner last-wins, ADR-0081 「RegisterRole 재전송」 (RegisterRole resend)).
    /// So calling it with an empty list leaves that owner's place completely empty.
    ///
    /// ★If the cap is exceeded, **not one name is entered** and `INVALID_ARGUMENT` is returned★ — if we cut off only the overflow
    /// and entered the rest, the owner would take it as success and some names would silently be missing (those names go out as `UNKNOWN_COMMAND`,
    /// though from the registering side's view there is no reason for that). The rejection is decided **before the registry is touched**, so a failed
    /// registration does not change existing state.
    /// ★Taking a name held by another is legitimate here★ — registration is an entire-set declaration tied to **the moment of attaching**
    /// (TRD §3-7 clause 1), so a new connection inheriting an old connection's names is the handover itself. The diff while attached
    /// ([`Roster::update`]) has no such anchor, so the same handover is not allowed.
    /// ★A late registration from a disconnected owner is **not blocked here**★ — the registry has no trace of that owner (it is erased on disconnect),
    /// so there is no basis to tell it apart from one sent by a live owner. That net is in **one place only**, the side that knows connection lifetime
    /// (`CommandRoster::refuse_if_detached` in `engram-dashboard-daemon`) — putting a second net here
    /// would mean holding per-owner state separately, and that list would grow without bound just like a trace.
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

    /// The **diff** while attached — a late-starting feature adds names and a switched-off feature takes names down (TRD §3-7 clause 3).
    /// A full resend is [`Roster::register`] only.
    ///
    /// ★`removed` **erases** the name — it does not leave a trace★: if it left a trace, an owner that is still connected could
    /// repeat `added`=new name · `removed`=old name and permanently fill its own share ([`Roster::MAX_NAMES_PER_OWNER`]),
    /// and then the same accident of traces permanently filling the registry would recur without even disconnecting
    /// (the other half of the axis ADR-0150 decision 3 closed for disconnects).
    /// ★`removed` erases **only names it holds itself**★ — the same rule as [`Roster::disconnect`]. If a late diff from a past
    /// owner erased the registration of the owner that just attached, a live command would disappear (reconnect overlap). Another
    /// owner's name or a nonexistent name passes silently — a diff means 「내 몫을 이렇게 바꿔라」 (change my share like this), not
    /// a claim about someone else's state.
    /// ★`added` cannot take a name held by another — this is where it parts from [`Roster::register`]★.
    /// Registration's last-wins is tied to **the moment of attaching** (TRD §3-7 clause 1), so it reads as a handover, but a diff is by definition
    /// 「붙어 있는 동안」 (while attached), so it has no such anchor. If an anchorless overwrite were allowed, a late diff from a dead connection would
    /// **overwrite with its own token** the registration of the shell that just attached, so lookups would answer the dead owner as `Available`, and the live shell would have no
    /// occasion to register again (registration happens only once on attaching), so that state would be frozen in place.
    /// ★Why it is rejected rather than passed silently★ (another owner's name in `removed` passes silently): not
    /// erasing another's name means 「내 몫」 (my share) as the diff stated it is already so, so nothing is lost, but if `added` were silently
    /// ignored, the caller would **be told success** while believing that name comes to itself. The rejection is `CONFLICT`
    /// — it answers not that the packet is wrong (`INVALID_ARGUMENT`) but that the **state** of the registry is not so.
    /// ★A diff from a disconnected owner is **not blocked here**★ — the same reason as [`Roster::register`], and the net
    /// stands in the same single place.
    /// ★A packet carrying one name in both `added` and `removed` is **rejected whole with `INVALID_ARGUMENT`**★ —
    /// we do not fix an application order. Whichever order is picked, the caller cannot know the result: adding then removing means that **even after receiving success**
    /// the name is not in the registry and goes out as `UNKNOWN_COMMAND` (the cap rejection of [`Roster::register`] blocks
    /// that very accident —
    /// 「주인은 성공으로 알고 일부 이름이 조용히 없는 상태가 된다」 (the owner takes it as success and some names silently end up missing)), and removing then adding means the instruction to take it down is silently
    /// ignored. **A success report that diverges from actual state is worse than picking the wrong order** — the caller has no means to tell a silent
    /// deletion from a successful registration. No clause fixes an order in ADR-0150 or in TRD §3-7 clause 3, so
    /// we do not fill that gap arbitrarily here.
    ///
    /// The cap is checked again counting only `added` — if exceeded, **not one name is touched** (the same contract as [`Roster::register`],
    /// and the two rejections above are the same).
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

    /// Whether one name is carried in both lists — checked **before the registry is touched** (the same contract as [`Roster::check_room_for`]).
    /// The reason for rejecting and 「적용 순서를 정하지 않는다」 (we do not fix an application order) are in the [`Roster::update`] comment.
    ///
    /// ★It does not look at the registry — so it takes no `self`★: the subject of the decision is **one packet**, and that is also why its code is
    /// `INVALID_ARGUMENT` (sending the same round trip again gives the same answer). Its sibling
    /// [`Roster::check_added_are_not_taken`] by contrast speaks about the registry's **state**, so it is `CONFLICT` and the answer changes
    /// if the state changes.
    /// ★The iteration is over `added`★ — the name quoted in the message must be one that passed [`Roster::MAX_NAME_BYTES`], and
    /// only `decls` has its length measured (`removed` is not measured). An overlapping name is by definition also in `added`, so the value
    /// this iteration quotes is always within that cap.
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

    /// Whether `added` **overwrites another's registration** — the only entries that remain in the registry have an owner (ADR-0150), so it is decided by
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
    /// ★The per-owner cap measures that owner's **cumulative** total, not one packet★: if only the packet were measured, one owner sending
    /// non-overlapping 512-name diffs one after another would fill the whole registry by itself. Then the real shell's registration would be blocked by the overall
    /// cap and `tab.*` and `window.*` would all go out as `UNKNOWN_COMMAND`, a code that means 「재시도가
    /// 무의미하니 이름을 다시 발견하라」 (retrying is pointless, rediscover the name) (TRD §4-②), so the caller would give up on a command that really exists for good.
    /// So it is blocked here so that the overall cap does not become a **shared resource with no per-owner share**. The names carried in this packet
    /// are overwrites and do not take new slots, so they are not counted twice.
    /// ★This cap is a different axis from the rejected TTL★ — what was rejected there was **time**, so the answer to the same question varied
    /// with the clock (ADR-0156 alternative C · ADR-0150 alternative F). A count cap does not change the answer for a name in the registry.
    /// ★The decision comes before the change, so this **over-measures** (left knowingly)★: names that will come down in this round trip are still
    /// in the registry, so they are counted too — [`Roster::register`]'s own names about to be replaced and `removed`
    /// of [`Roster::update`] are such cases. The cost is only that 「몫이 꽉 찬 주인은 이름 전량을 한 패킷에 갈아치우지 못한다」 (an owner whose share is full cannot replace its entire name set in one packet), and sending
    /// one empty full registration empties its own place completely and opens that path. It is not an axis along which the registry grows, so
    /// we did not fix it precisely — measuring exactly would require the cap decision to model the result of the change, and then this function's contract 「명부를 건드리기
    /// 전에 판정한다」 (decide before touching the registry) would blur.
    ///
    /// Names and `help` are strings the registration packet carried, so length is measured too — one exaggerated `help` eats the registry memory
    /// whole for as long as that owner is attached.
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

    /// The connection was cut — **erase the entries remaining under that owner along with their slot.** Since it leaves neither trace nor marker, those
    /// names get the same answer as names that were never registered ([`OwnerLookup`]).
    ///
    /// ★It takes down **only names the owner still owns**★ — so that in a reconnect overlap (the new connection registers, then the old connection's cleanup arrives)
    /// it does not erase the registration of the owner that just attached.
    /// ★This is the only place that handles disconnects★ — the only calling path is the daemon's `on_disconnect → detach`
    /// (ADR-0150), and making a second would split causality.
    /// ★Return = **all names this call erased** (ascending)★ — after erasing, the registry has no trace of the event
    /// (the reason for leaving no trace is in this type's comment), so this return is the only value that can say what disappeared. Instrumentation is
    /// done by **the side that knows connection lifetime** (`CommandRoster::detach` in `engram-dashboard-daemon`) — this crate bears
    /// no logging dependency (the workspace and third-party dependency cap = the `lib.rs` header).
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
    /// The owner token is new per connection, so when the shell reconnects, the registration (new token) comes first and the old connection's cleanup
    /// arrives late. If the cleanup took names down per name, the commands of the shell that just attached would silently vanish.
    #[test]
    fn a_superseded_owners_cleanup_does_not_take_down_the_current_registration() {
        let old = OwnerToken::new("shell-conn-1");
        let new = OwnerToken::new("shell-conn-2");
        let mut roster = Roster::new();
        roster
            .register(&old, vec![decl("tab.create")])
            .expect("상한 안");
        roster
            .register(&new, vec![decl("tab.create")])
            .expect("상한 안");

        roster.disconnect(&old);
        assert_eq!(
            roster.lookup("tab.create"),
            OwnerLookup::Available(new.clone()),
            "방금 등록한 주인이 답해야 한다"
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
            .expect("상한 안");
        roster
            .register(&second, vec![decl("tab.create")])
            .expect("상한 안");

        assert_eq!(roster.lookup("tab.create"), OwnerLookup::Available(second));
    }

    /// ★Nothing accumulates in the registry however often it reconnects★ — the owner token is new per connection, so in the form that left a trace, this repetition
    /// permanently filled the registry up to the cap (ADR-0150 decision 3).
    #[test]
    fn reconnecting_many_times_leaves_nothing_behind() {
        let mut roster = Roster::new();
        for connection in 0..50 {
            let owner = OwnerToken::new(format!("shell-conn-{connection}"));
            roster
                .register(&owner, vec![decl("tab.create"), decl("tab.close")])
                .expect("상한 안");
            roster.disconnect(&owner);
        }

        assert!(roster.is_empty(), "쌓이는 것이 없다");
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
            .expect("상한 안");
        roster
            .register(
                &second,
                vec![CommandDecl {
                    name: "tab.create".to_string(),
                    help: "{\"name\":\"tab.create\",\"since\":2}".to_string(),
                    agent_args: Vec::new(),
                }],
            )
            .expect("상한 안");

        assert_eq!(roster.len(), 1);
        let entry = roster.entries().next().expect("한 항목");
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
            .expect_err("상한을 넘는다");

        assert_eq!(err.code(), crate::ErrorCode::InvalidArgument);
        assert!(roster.is_empty(), "실패한 등록은 명부를 건드리지 않는다");
        roster
            .register(&owner, decls("fits", Roster::MAX_NAMES_PER_OWNER))
            .expect("상한 자체는 통과한다");
        assert_eq!(roster.len(), Roster::MAX_NAMES_PER_OWNER);
    }

    /// ★The cap decision comes before the change, so it also counts the owner's own names this packet will push out★
    ///
    /// It is an over-measure and left knowingly (basis = `check_room_for`). We pin down together that the cost is limited to 「몫이 꽉 찬 주인은 이름 전량을 한
    /// 패킷에 갈아치우지 못한다」 (an owner whose share is full cannot replace its entire name set in one packet), and that **one empty full registration opens that path**.
    #[test]
    fn a_full_owners_disjoint_snapshot_is_measured_before_the_old_names_come_down() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(&owner, decls("wave0", Roster::MAX_NAMES_PER_OWNER))
            .expect("첫 스냅샷은 상한 안");

        let err = roster
            .register(&owner, decls("wave1", Roster::MAX_NAMES_PER_OWNER))
            .expect_err("겹치지 않는 두 번째 스냅샷은 누적으로 상한을 넘는다");

        assert_eq!(err.code(), crate::ErrorCode::InvalidArgument);
        assert_eq!(
            roster.len(),
            Roster::MAX_NAMES_PER_OWNER,
            "실패한 등록은 명부를 건드리지 않는다"
        );

        roster.register(&owner, vec![]).expect("빈 전량 등록");
        assert!(roster.is_empty(), "전량 선언이 비면 자기 자리도 빈다");
        roster
            .register(&owner, decls("wave1", Roster::MAX_NAMES_PER_OWNER))
            .expect("자리가 비면 새 전량이 들어온다");
        assert_eq!(roster.len(), Roster::MAX_NAMES_PER_OWNER);
    }

    /// On disconnect that owner's share is completely empty — the names holding the cap vanish along with their slot (ADR-0150 decision 3).
    #[test]
    fn a_disconnect_frees_that_owners_whole_cap() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(&owner, decls("wave0", Roster::MAX_NAMES_PER_OWNER))
            .expect("상한 안");
        roster.disconnect(&owner);

        roster
            .register(&owner, decls("wave1", Roster::MAX_NAMES_PER_OWNER))
            .expect("자리를 쥐고 있는 것이 없다");

        assert_eq!(roster.len(), Roster::MAX_NAMES_PER_OWNER);
        assert_eq!(roster.lookup("wave0.0"), OwnerLookup::Unknown);
    }

    /// ★Names and `help` are strings the registration packet carried★ — if length were not measured, one exaggerated `help` would eat the registry
    /// memory whole for as long as that owner is attached.
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
            .expect("상한 자체는 통과한다");
        assert_eq!(roster.len(), 1);

        let long_name = CommandDecl {
            name: "n".repeat(Roster::MAX_NAME_BYTES + 1),
            help: "{}".to_string(),
            agent_args: Vec::new(),
        };
        let err = roster
            .register(&owner, vec![long_name])
            .expect_err("이름이 상한을 넘는다");
        assert_eq!(err.code(), crate::ErrorCode::InvalidArgument);

        let long_help = CommandDecl {
            name: "tab.create".to_string(),
            help: "h".repeat(Roster::MAX_HELP_BYTES + 1),
            agent_args: Vec::new(),
        };
        let err = roster
            .register(&owner, vec![decl("tab.close"), long_help])
            .expect_err("help 가 상한을 넘는다");
        assert_eq!(err.code(), crate::ErrorCode::InvalidArgument);

        assert_eq!(roster.len(), 1, "실패한 등록은 명부를 건드리지 않는다");
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
            .expect("상한 안");

        assert_eq!(roster.agent_args("slot.focus"), ["agent", "after"]);
        assert!(roster.agent_args("tab.close").is_empty(), "표시 없음");
        assert!(roster.agent_args("never.seen").is_empty(), "모르는 이름");

        roster
            .update(
                &owner,
                vec![
                    marked("slot.focus", &["target"]),
                    marked("tab.move", &["agent"]),
                ],
                vec!["tab.close".to_string()],
            )
            .expect("자기 이름의 차분");
        assert_eq!(
            roster.agent_args("slot.focus"),
            ["target"],
            "차분이 표시를 갈아 끼운다"
        );
        assert_eq!(roster.agent_args("tab.move"), ["agent"]);

        roster
            .register(&owner, vec![decl("slot.focus")])
            .expect("전량 재등록");
        assert!(
            roster.agent_args("slot.focus").is_empty(),
            "재등록이 표시를 뺐으면 빠진다"
        );
        assert!(roster.agent_args("tab.move").is_empty(), "내려간 이름");

        roster.disconnect(&owner);
        assert!(
            roster.agent_args("slot.focus").is_empty(),
            "끊긴 주인의 표시는 안 남는다"
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
            .expect("상한 자체는 통과한다");

        let too_many: Vec<&str> = vec!["a"; Roster::MAX_AGENT_ARGS + 1];
        let err = roster
            .register(
                &owner,
                vec![decl("tab.close"), marked("tab.move", &too_many)],
            )
            .expect_err("칸 수가 상한을 넘는다");
        assert_eq!(err.code(), crate::ErrorCode::InvalidArgument);

        let too_long = "a".repeat(Roster::MAX_NAME_BYTES + 1);
        let err = roster
            .update(
                &owner,
                vec![marked("tab.move", &[too_long.as_str()])],
                vec![],
            )
            .expect_err("칸 이름이 상한을 넘는다");
        assert_eq!(err.code(), crate::ErrorCode::InvalidArgument);

        assert_eq!(
            roster.len(),
            1,
            "실패한 등록 · 차분은 명부를 건드리지 않는다"
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
                .expect("전체 상한 안");
        }
        assert_eq!(roster.len(), Roster::MAX_NAMES);

        let newcomer = OwnerToken::new("shell-late");
        let err = roster
            .register(&newcomer, vec![decl("one.more")])
            .expect_err("전체 상한을 넘는다");
        assert_eq!(err.code(), crate::ErrorCode::InvalidArgument);
        assert_eq!(roster.len(), Roster::MAX_NAMES, "명부는 그대로다");

        // ★A name that already exists does not take a new slot★ — re-registration must pass even in a full registry.
        roster
            .register(&newcomer, vec![decl("owner0.0")])
            .expect("덮어쓰기는 크기를 안 늘린다");
        assert_eq!(roster.len(), Roster::MAX_NAMES);
    }

    /// Carrying the same name several times still takes one slot — duplicates do not burn the cap.
    #[test]
    fn duplicate_names_in_one_packet_count_once() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        let repeated = vec![decl("tab.create"); Roster::MAX_NAMES_PER_OWNER + 8];

        roster.register(&owner, repeated).expect("이름은 하나다");

        assert_eq!(roster.len(), 1);
    }

    /// ★The registration side of the vanished line of defense — the registry does not block a disconnected owner's late **registration** either★
    ///
    /// Registration is a 「붙는 순간의 전량 선언」 (entire-set declaration at the moment of attaching), so inheriting even a live owner's names is legitimate (handover), but the registry has
    /// no memory that the owner has already disconnected, so a dead connection's late registration cannot be told apart from a genuine handover.
    /// So if this packet passes, that name becomes `Available` under a **nonexistent owner**, and that connection has no further
    /// cleanup coming, so it stays frozen for the whole daemon lifetime (delivery goes out over a cut link). The blocking net is in the side that knows connection lifetime,
    /// **one place only** — `CommandRoster::refuse_if_detached` in `engram-dashboard-daemon`.
    #[test]
    fn the_roster_alone_cannot_refuse_a_late_registration_from_a_disconnected_owner() {
        let old = OwnerToken::new("shell-conn-1");
        let new = OwnerToken::new("shell-conn-2");
        let mut roster = Roster::new();
        roster
            .register(&old, vec![decl("tab.create")])
            .expect("상한 안");
        roster.disconnect(&old);
        roster
            .register(&new, vec![decl("tab.create")])
            .expect("산 주인이 그 이름을 새로 얹는다");

        roster
            .register(&old, vec![decl("tab.create")])
            .expect("명부에는 반려할 근거가 없다");

        assert_eq!(
            roster.lookup("tab.create"),
            OwnerLookup::Available(old),
            "없는 주인 앞으로 선다 — 이것을 막는 곳은 데몬 하나뿐이다"
        );
    }

    // ── Diff registration (TRD §3-7 clause 3) ────────────────────────────────────────────────────────────

    /// ★A diff erases only `removed`★ — if it left a trace, an owner that is still connected would repeat add/remove and permanently fill its own share
    /// (ADR-0150). Not touching names that are not carried is where it parts from `register`'s
    /// entire-set last-wins.
    #[test]
    fn an_update_removes_only_the_names_it_lists() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(&owner, vec![decl("tab.create"), decl("tab.close")])
            .expect("상한 안");

        roster
            .update(
                &owner,
                vec![decl("tab.split")],
                vec!["tab.close".to_string()],
            )
            .expect("상한 안");

        assert_eq!(
            roster.lookup("tab.create"),
            OwnerLookup::Available(owner.clone()),
            "차분에 안 실린 이름은 그대로다"
        );
        assert_eq!(roster.lookup("tab.split"), OwnerLookup::Available(owner));
        assert_eq!(roster.lookup("tab.close"), OwnerLookup::Unknown);
        assert_eq!(roster.len(), 2, "내린 이름은 자리째 사라진다");
    }

    /// ★A diff's `removed` empties the place★ — if left as a trace, an attached owner could keep changing names and permanently fill its own share
    /// cap, and the same unbounded growth would recur even without disconnecting (ADR-0150).
    #[test]
    fn swapping_names_forever_does_not_eat_an_owners_cap() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(&owner, decls("wave", Roster::MAX_NAMES_PER_OWNER))
            .expect("몫을 꽉 채운다");

        for round in 0..8 {
            roster
                .update(
                    &owner,
                    vec![],
                    vec![format!("wave.{}", Roster::MAX_NAMES_PER_OWNER - 1 - round)],
                )
                .expect("내리는 것은 언제나 통과한다");
            roster
                .update(&owner, vec![decl(&format!("swap.{round}"))], vec![])
                .expect("자리가 비었으므로 더할 수 있다");
        }

        assert_eq!(
            roster.len(),
            Roster::MAX_NAMES_PER_OWNER,
            "바꿔 끼워도 크기는 그대로다"
        );
    }

    /// ★Reconnect overlap — the same net must stand for the diff too★
    ///
    /// If an old connection's late diff took names down per name, the commands of the new connection that just registered the same name would silently
    /// vanish (the same accident `disconnect` prevents).
    #[test]
    fn an_update_from_a_superseded_owner_does_not_take_down_the_current_registration() {
        let old = OwnerToken::new("shell-conn-1");
        let new = OwnerToken::new("shell-conn-2");
        let mut roster = Roster::new();
        roster
            .register(&old, vec![decl("tab.create")])
            .expect("상한 안");
        roster
            .register(&new, vec![decl("tab.create")])
            .expect("상한 안");

        roster
            .update(&old, vec![], vec!["tab.create".to_string()])
            .expect("상한 안");

        assert_eq!(
            roster.lookup("tab.create"),
            OwnerLookup::Available(new),
            "방금 등록한 주인이 답해야 한다"
        );
    }

    /// ★Reconnect overlap — the same net must stand on the `added` side too★
    ///
    /// The mirror of the test above. If an old connection's late diff **adds** the same name, the entry is overwritten with a dead token,
    /// and lookup answers that dead owner as `Available`, so delivery goes out over a cut link. The live shell has no occasion to register again
    /// (registration happens once on attaching), so that state does not resolve by itself.
    #[test]
    fn an_update_from_a_superseded_owner_does_not_add_over_the_current_registration() {
        let old = OwnerToken::new("shell-conn-1");
        let new = OwnerToken::new("shell-conn-2");
        let mut roster = Roster::new();
        roster
            .register(&old, vec![decl("tab.create")])
            .expect("상한 안");
        roster
            .register(&new, vec![decl("tab.create")])
            .expect("상한 안");

        let err = roster
            .update(&old, vec![decl("tab.create")], vec![])
            .expect_err("산 남의 등록은 못 가져간다");

        assert_eq!(err.code(), crate::ErrorCode::Conflict);
        assert_eq!(
            roster.lookup("tab.create"),
            OwnerLookup::Available(new),
            "방금 등록한 주인이 답해야 한다"
        );
    }

    /// ★If one name is carried in both `added` and `removed`, it is rejected **whole packet**★
    ///
    /// If an application order is picked, the caller cannot know the result whichever way: adding then removing means that **even after receiving success**
    /// the name is not in the registry and goes out as `UNKNOWN_COMMAND` (exactly the accident `register` blocks at the cap —
    /// 「주인은 성공으로 알고 일부 이름이 조용히 없는 상태가 된다」 (the owner takes it as success and some names silently end up missing)), and removing then adding means the instruction to take it down is
    /// silently ignored. No clause fixes an order in ADR-0150 or in TRD §3-7 clause 3, so rejection is the answer.
    #[test]
    fn an_update_carrying_one_name_in_both_lists_is_refused_whole() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(&owner, vec![decl("tab.create")])
            .expect("상한 안");

        // A name already held — under the old application order it was added then erased and **vanished along with a success**.
        let err = roster
            .update(
                &owner,
                vec![decl("tab.create")],
                vec!["tab.create".to_string()],
            )
            .expect_err("같은 이름이 양쪽에 실렸다");
        assert_eq!(err.code(), crate::ErrorCode::InvalidArgument);
        assert!(
            err.message().contains("tab.create"),
            "어느 이름이 걸렸는지 문구가 말해야 한다: {}",
            err.message()
        );
        assert_eq!(
            roster.lookup("tab.create"),
            OwnerLookup::Available(owner.clone()),
            "반려는 명부를 건드리지 않는다"
        );

        // A new name — here 「넣었다가 지운다」 (add then erase) would not create a name that never existed from the start either, but the decision is the same.
        let err = roster
            .update(
                &owner,
                vec![decl("tab.split")],
                vec!["tab.split".to_string()],
            )
            .expect_err("같은 이름이 양쪽에 실렸다");
        assert_eq!(err.code(), crate::ErrorCode::InvalidArgument);
        assert_eq!(roster.lookup("tab.split"), OwnerLookup::Unknown);
        assert_eq!(roster.len(), 1);
    }

    /// Returns to the caller **what the disconnect erased** — since no trace remains in the registry (ADR-0150), this return is the only value from which that event
    /// can be reconstructed later. Instrumentation is done by the daemon, which knows connection lifetime
    /// (`CommandRoster::detach` — no logging dependency is brought into the tool crate).
    #[test]
    fn a_disconnect_reports_the_names_it_removed() {
        let owner = OwnerToken::new("shell-1");
        let other = OwnerToken::new("shell-2");
        let mut roster = Roster::new();
        roster
            .register(&owner, vec![decl("tab.create"), decl("tab.close")])
            .expect("상한 안");
        roster
            .register(&other, vec![decl("window.focus")])
            .expect("상한 안");

        let removed = roster.disconnect(&owner);

        assert_eq!(
            removed,
            vec!["tab.close".to_string(), "tab.create".to_string()],
            "자기가 쥔 이름만 실린다"
        );
        assert!(
            roster.disconnect(&owner).is_empty(),
            "두 번째 정리에는 지울 것이 없다"
        );
        assert_eq!(
            roster.lookup("window.focus"),
            OwnerLookup::Available(other),
            "남의 등록은 그대로다"
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
            .expect("상한 안");
        roster.disconnect(&old);
        roster
            .register(&new, vec![decl("tab.close")])
            .expect("상한 안");
        assert_eq!(roster.lookup("tab.create"), OwnerLookup::Unknown);

        roster
            .update(&new, vec![decl("tab.create")], vec![])
            .expect("주인 없는 이름은 가져갈 수 있다");

        assert_eq!(roster.lookup("tab.create"), OwnerLookup::Available(new));
    }

    /// The cap is measured **cumulatively** for a diff too, and if exceeded not one name enters.
    #[test]
    fn an_update_over_the_per_owner_cap_is_refused_whole() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(&owner, decls("wave0", Roster::MAX_NAMES_PER_OWNER))
            .expect("상한 안");

        let err = roster
            .update(&owner, vec![decl("one.more")], vec![])
            .expect_err("이미 쥔 이름이 몫을 다 쓰고 있다");

        assert_eq!(err.code(), crate::ErrorCode::InvalidArgument);
        assert_eq!(roster.len(), Roster::MAX_NAMES_PER_OWNER);
        assert_eq!(roster.lookup("one.more"), OwnerLookup::Unknown);
    }

    /// ★On disconnect it vanishes without a trace★ — the owner token is new per connection, so if a trace were left, the same client's
    /// reconnect would be **piling up**, not overwriting, and with neither expiry nor a reclaim path the registry would permanently fill up to the cap
    /// (ADR-0150 decision 3).
    #[test]
    fn a_disconnect_empties_that_owners_share_of_the_roster() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(&owner, vec![decl("tab.create"), decl("tab.close")])
            .expect("상한 안");

        roster.disconnect(&owner);

        assert!(roster.is_empty(), "항목이 남지 않는다");
        assert_eq!(roster.entries().count(), 0);
        assert_eq!(
            roster.lookup("tab.create"),
            OwnerLookup::Unknown,
            "모르는 이름과 같은 답이다"
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
            .expect("상한 안");
        roster.disconnect(&old);

        roster
            .register(&new, vec![decl("tab.create")])
            .expect("상한 안");

        assert_eq!(roster.len(), 1, "옛 연결의 나머지 이름은 남지 않는다");
        assert_eq!(roster.entries().next().expect("한 항목").owner, new);
        assert_eq!(roster.lookup("tab.close"), OwnerLookup::Unknown);
    }

    /// ★Pin down the vanished line of defense — the registry **cannot block** a disconnected owner's late packet★
    ///
    /// Once the trace is gone, the registry has no place to remember that owner's death. So a late diff passes as is,
    /// and that name becomes `Available` under a **nonexistent owner** and stays frozen for the whole daemon lifetime (that connection has no cleanup
    /// coming). The blocking net is in the side that knows connection lifetime, **one place only** — `CommandRoster::refuse_if_detached` in `engram-dashboard-daemon`
    /// (the list of live connections). Do not put a second net here:
    /// if the registry held per-owner state separately, expiry-less data would gain one more unboundedly growing list (the same reason
    /// the trace was discarded — ADR-0150).
    #[test]
    fn the_roster_alone_cannot_refuse_a_late_delta_from_a_disconnected_owner() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(&owner, vec![decl("tab.create")])
            .expect("상한 안");
        roster.disconnect(&owner);

        roster
            .update(&owner, vec![decl("tab.split")], vec![])
            .expect("명부에는 반려할 근거가 없다");

        assert_eq!(
            roster.lookup("tab.split"),
            OwnerLookup::Available(owner),
            "없는 주인 앞으로 선다 — 이것을 막는 곳은 데몬 하나뿐이다"
        );
    }

    #[test]
    fn reregistration_is_last_wins_and_drops_stale_names() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(&owner, vec![decl("tab.create"), decl("tab.legacy")])
            .expect("상한 안");
        roster
            .register(&owner, vec![decl("tab.create")])
            .expect("상한 안");

        assert_eq!(
            roster.lookup("tab.create"),
            OwnerLookup::Available(owner.clone())
        );
        assert_eq!(roster.lookup("tab.legacy"), OwnerLookup::Unknown);
    }
}
