//! groups — `@` address resolution seam (spec §4, ADR-0104 decision 1, ADR-0111 decision 4, ADR-0112 decision 1, ADR-0121 decision 1).
//!
//! ★Role★: expands "`@` address → member (id, name) list". The v1 source is a single built-in one with two words (ADR-0121): **`@here`** =
//!   everyone alive right now, **`@all`** = everyone in the registry
//! (alive + sleeping). Members carry **the registry's id as is** (ADR-0309 decision 3 — no name → id round trip); the same profile keeps
//!   its id across re-spawns; the address and response notation are names (ADR-0101).
//! ★The two words reading **different pools** is the decision itself (ADR-0121 §Impact)★: one shared pool would make "all" a false name
//!   again (the sender believes "everyone was told" while a sleeping peer is missed)
//! and `@here` pointless; the contract suite below asserts the source separation as an axis.
//! ★A group is an **address-resolution macro**, not a send path (ADR-0111 decision 4)★: expanded members flow through **exactly the same**
//!   path as directly named recipients (spec §5 fan-out); group-specific
//! delivery rules (skipping dead members, binding `(id,epoch)` at send, a fan-out branch) were all abolished — reviving them violates ADR-0111.
//! ★User-defined groups were removed (ADR-0111 decision 4, ADR-0112 decision 1)★: registry, `group` MCP tool, CLI `group` subcommand,
//!   ADR-0109 management are gone; stored named groups will be redesigned on real
//! demand (folder groups first) — no storage structure is laid down in advance.
//! ★The resolution seam is kept★: `GroupSource` lets a future source (folder `@folder-name`) be added without touching the pipeline; v1 is
//!   `BuiltinGroups` alone — **no over-build**.
//! ★Pure, mechanical resolution (ADR-0104)★: `resolve` **only expands**; alive decisions, sender exclusion, dedup and roster matching are
//!   `MessagingService`'s. ★"0 members" is not an error here★ (returned as is):
//! `GROUP_EMPTY` is raised only when the **final recipient set** (expansion + explicit picks) is empty (ADR-0114 decision 3), which this
//!   layer cannot know.
// ADR-0111, ADR-0112

use crate::PeerId;

/// Built-in word `@all` — **everyone in the registry** (alive + sleeping) **− sender** (spec §4, ADR-0121 decision 1).
pub const ALL_GROUP: &str = "@all";

/// Built-in word `@here` — **everyone alive right now** **− sender** (spec §4, ADR-0121 decision 1). ★Word choice★: Slack's same
///   distinction (`@channel` / `@here`) is likely known to the driving LLM (ADR-0121 §Rationale).
pub const HERE_GROUP: &str = "@here";

/// One member of the expansion — the id the registry gave and the name at that moment. The upper layer picks the recipient by this id
///   directly; re-resolving by name would fold same-name twins into one recipient or
/// `RECIPIENT_AMBIGUOUS` (ADR-0309 decision 3, TRD S29 §4 X1).
// ADR-0309
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupMember {
    pub id: PeerId,
    pub name: String,
}

/// The **two list pools** the resolver reads (ADR-0121 decision 1) — built by the caller from a snapshot taken outside the lock. ★Two
///   pools, not one slice (load-bearing)★: "which word reads which pool" is **source
/// knowledge**; if the upper layer picked a slice per word the pipeline would know `@all`/`@here` and a future source (folder
///   `@folder-name`) could not pick its own pool (ADR-0104: source knowledge must not leak into
/// the pipeline). So both are passed and the source chooses. ★Sender exclusion arrives already applied★ (spec §4 — the spec is canonical;
///   ADR-0111 decision 4 lacks that phrase).
#[derive(Debug, Clone, Copy)]
pub struct MemberPools<'a> {
    /// Members alive right now — the pool of `@here` and the first part of `@all`.
    pub live: &'a [GroupMember],
    /// Sleeping members (profile exists, session not alive) — read **only by `@all`**. ★Same names are not folded★: this layer returns the
    ///   caller's pool verbatim (no sorting, dedup or filtering); two members with one name
    ///   are different recipients (ADR-0309, TRD S29 §4 X1).
    pub dormant: &'a [GroupMember],
}

/// `@` address resolution error — the upper layer maps it to a wire error code (spec §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupError {
    /// `@` namespace convention violation — normalization failed.
    InvalidName { name: String },
    /// An `@name` that is not a built-in word → `GROUP_NOT_FOUND` (the whole send is bounced, ADR-0114 decision 3).
    NotFound { name: String },
}

/// ★Group address normalization — a **seam-level** free function (C4 review fix F, ADR-0104 decision 1)★. ★Why not a source method (seam
///   boundary)★: the pipeline normalizes the name **before** asking the source (the envelope
/// `to` token and error hint use it); a method of one source would tie the pipeline to that type and make a future source with a different
///   name grammar unreachable (ADR-0104). So the convention sits outside the sources.
/// ★Strict convention (@-namespace contract)★: a group name **must start with exactly one leading `@`**. No leading `@` (`coders`) is
///   `InvalidName` — lenient correction (`coders` → `@coders`) was rejected because the
/// person/group distinction hangs on one `@`. An empty/whitespace body or another `@` in the body (`@@x`, `@a@b`) is likewise `InvalidName`.
// ADR-0104
pub fn normalize_group_name(name: &str) -> Result<String, GroupError> {
    let trimmed = name.trim();
    let Some(body) = trimmed.strip_prefix('@') else {
        return Err(GroupError::InvalidName {
            name: name.to_string(),
        });
    };
    if body.is_empty() || body.chars().all(char::is_whitespace) || body.contains('@') {
        return Err(GroupError::InvalidName {
            name: name.to_string(),
        });
    }
    Ok(format!("@{body}"))
}

/// Group resolution source (seam — ADR-0104 decision 1): the extension point of the "`@` address → member list" resolver.
pub trait GroupSource {
    /// `NotFound` if this source does not know that address.
    fn resolve(&self, group: &str, pools: MemberPools<'_>) -> Result<Vec<GroupMember>, GroupError>;
}

/// The only v1 source — knows only the two built-in words (`@all`·`@here`); no storage, computed on the spot (ADR-0111 decision 4,
///   ADR-0121 decision 1).
#[derive(Debug, Default, Clone, Copy)]
pub struct BuiltinGroups;

impl GroupSource for BuiltinGroups {
    // ADR-0121
    fn resolve(&self, group: &str, pools: MemberPools<'_>) -> Result<Vec<GroupMember>, GroupError> {
        let norm = normalize_group_name(group)?;
        if norm == HERE_GROUP {
            return Ok(pools.live.to_vec());
        }
        if norm == ALL_GROUP {
            let mut members = Vec::with_capacity(pools.live.len() + pools.dormant.len());
            members.extend_from_slice(pools.live);
            members.extend_from_slice(pools.dormant);
            return Ok(members);
        }
        Err(GroupError::NotFound { name: norm })
    }
}

/// ★`GroupSource` contract suite — a safety net for durability under source swaps★. ★Why (load-bearing)★: `GroupSource` is a seam laid
///   down on the premise it will be swapped (ADR-0104 decision 1 — an
/// `@folder-name` source once folders are daemon-owned). The worst case is a new implementation quietly changing meaning — `@all` sorted,
///   **`@here` reading sleeping names too**, an empty snapshot folded into an error, a
/// lenient `@` convention — which compiles and leaves existing tests green while the broadcast meaning flips. So this reusable suite
///   asserts the semantics **against the trait**.
/// ★When it goes red★: fix **the decision, not the suite** — the semantics are user-decided (spec §4, ADR-0111 decision 4, ADR-0114
///   decision 3, ADR-0121 decision 1); loosening the assertions is a contract change
/// needing user ratification.
#[cfg(any(test, feature = "test-harness"))]
pub mod contract {
    use super::{GroupError, GroupMember, GroupSource, MemberPools, ALL_GROUP, HERE_GROUP};
    use crate::PeerId;

    #[derive(Debug, Clone)]
    pub struct GroupSourceFixture {
        /// An address name this source **does not know** (the `NotFound` axis); must already be normalized (`@nope`).
        pub unknown: String,
        /// Does this source **own** the built-in words (`@all`·`@here`). v1 `BuiltinGroups` = true; a future folder source usually false.
        pub handles_builtins: bool,
    }

    /// A different id per member (sequence from `base`) — several sharing one name are same-name twins.
    fn members(base: u128, names: &[&str]) -> Vec<GroupMember> {
        names
            .iter()
            .zip(base..)
            .map(|(name, n)| GroupMember {
                id: PeerId::from_u128(n),
                name: name.to_string(),
            })
            .collect()
    }

    /// ★`GroupSource` resolution semantics contract — any implementation must pass this (spec §4, ADR-0111/0114/0121)★. ★The `axisN-…:`
    ///   token before assertion messages (load-bearing)★: each failure message
    ///   starts with a token unique per axis; self-check tests confirm "the net bites" via `#[should_panic(expected = …)]`, and
    ///   overlapping substrings would let the wrong axis blow up first and still pass. If you change a token, fix the pin too.
    pub fn assert_group_source_contract(src: &dyn GroupSource, fx: &GroupSourceFixture) {
        // ── 1·1b·1c·2. Built-in word axis ────────────────────────────────────────────────────────
        // Sorting would scatter the upper layer's row-order rule (explicit tokens → expansion alphabetical) across two layers, and name
        //   dedup would fold same-name twins at the source stage (ADR-0309, TRD S29 §4 X1).
        let live = members(1, &["zeta", "alpha", "zeta", "mid"]);
        let dormant = members(100, &["sleepy", "twin", "twin"]);
        let pools = MemberPools {
            live: &live,
            dormant: &dormant,
        };
        let empty = MemberPools {
            live: &[],
            dormant: &[],
        };
        if fx.handles_builtins {
            let mut all_expected = live.clone();
            all_expected.extend(dormant.clone());
            assert_eq!(
                src.resolve(ALL_GROUP, pools),
                Ok(all_expected),
                "axis1-all-verbatim: @all must join live + dormant as is (asleep names included · no sorting·dedup·filtering — spec §4 · ADR-0121 decision 1)"
            );
            assert_eq!(
                src.resolve(HERE_GROUP, pools),
                Ok(live.clone()),
                "axis1b-here-live-only: @here must be the live snapshot only, as is (with asleep names in it, it is not 'here now' — ADR-0121 decision 1)"
            );
            assert_ne!(
                src.resolve(ALL_GROUP, pools),
                src.resolve(HERE_GROUP, pools),
                "axis1c-vocab-split: when there are asleep names, @all and @here must be **different lists** — looking at the same source makes 'all' a false name again (ADR-0121 §Impact)"
            );
            assert_eq!(
                src.resolve(ALL_GROUP, empty),
                Ok(Vec::new()),
                "axis2-all-empty-ok: @all with an empty registry is an empty list (not an error) — GROUP_EMPTY is decided upstream on the final set including explicit targets (ADR-0114 decision 3)"
            );
            assert_eq!(
                src.resolve(HERE_GROUP, empty),
                Ok(Vec::new()),
                "axis2c-here-empty-ok: @here with 0 live recipients is an empty list too (not an error — same basis)"
            );
        } else {
            for builtin in [ALL_GROUP, HERE_GROUP] {
                assert!(
                    matches!(
                        src.resolve(builtin, pools),
                        Err(GroupError::NotFound { .. })
                    ),
                    "axis2b-builtin-not-owned: a source that does not own the built-in word ({builtin}) must not resolve it as if it were its own (no hijacking of broadcast meaning in source iteration)"
                );
            }
        }

        // ── 3. NotFound axis ────────────────────────────────────────────────────────────────
        assert_eq!(
            src.resolve(&fx.unknown, pools),
            Err(GroupError::NotFound {
                name: fx.unknown.clone()
            }),
            "axis3-unknown-notfound: an unknown address is NotFound and name must carry the normalized name (upstream writes it back into the hint)"
        );

        // ── 4. `@` namespace convention ───────────────────────────────────────────────────────
        for bad in ["", "@", "  @  ", "@@x", "@@", "@a@b", "plain-name"] {
            assert!(
                matches!(src.resolve(bad, pools), Err(GroupError::InvalidName { .. })),
                "axis4-namespace-strict: '{bad}' violates the @ namespace convention, so it must be InvalidName (no silent correction)"
            );
        }

        // ── 5. Determinism — the same input twice, the same answer ────────────────────────────────────────────
        // If this breaks, "one snapshot at send time" loses its meaning (two questions at one moment see different worlds).
        assert_eq!(
            src.resolve(&fx.unknown, pools),
            src.resolve(&fx.unknown, pools),
            "axis5-deterministic: resolution must be deterministic (pure — no clock·internal mutable state)"
        );
        if fx.handles_builtins {
            for builtin in [ALL_GROUP, HERE_GROUP] {
                assert_eq!(
                    src.resolve(builtin, pools),
                    src.resolve(builtin, pools),
                    "axis5-deterministic-builtin: resolving the built-in word ({builtin}) must be deterministic too"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::contract::{assert_group_source_contract, GroupSourceFixture};
    use super::*;

    /// A random id per member — writing one name twice makes same-name twins.
    fn members(v: &[&str]) -> Vec<GroupMember> {
        v.iter()
            .map(|s| GroupMember {
                id: PeerId::new_v4(),
                name: s.to_string(),
            })
            .collect()
    }

    fn pools<'a>(live: &'a [GroupMember], dormant: &'a [GroupMember]) -> MemberPools<'a> {
        MemberPools { live, dormant }
    }

    #[test]
    fn here_resolves_to_the_live_snapshot_verbatim() {
        let live = members(&["zeta", "alpha", "zeta"]);
        let dormant = members(&["sleepy"]);
        assert_eq!(
            BuiltinGroups.resolve("@here", pools(&live, &dormant)),
            Ok(live.clone()),
            "@here sees only the live list (asleep members not included — ADR-0121 decision 1)"
        );
    }

    #[test]
    fn all_appends_the_dormant_members_to_the_live_snapshot() {
        let live = members(&["zeta", "alpha"]);
        let dormant = members(&["sleepy", "twin", "twin"]);
        let mut expected = live.clone();
        expected.extend(dormant.clone());
        assert_eq!(
            BuiltinGroups.resolve("@all", pools(&live, &dormant)),
            Ok(expected),
            "the pool is verbatim — same-named twins have different ids, so both remain (ADR-0309 · TRD S29 §4 X1)"
        );
    }

    #[test]
    fn builtin_vocabularies_with_no_names_are_empty_lists_not_errors() {
        assert_eq!(
            BuiltinGroups.resolve("@all", pools(&[], &[])),
            Ok(Vec::new())
        );
        assert_eq!(
            BuiltinGroups.resolve("@here", pools(&[], &[])),
            Ok(Vec::new())
        );
    }

    #[test]
    fn unknown_at_address_is_not_found() {
        let live = members(&["alice"]);
        assert_eq!(
            BuiltinGroups.resolve("@coders", pools(&live, &[])),
            Err(GroupError::NotFound {
                name: "@coders".to_string()
            }),
            "user-defined groups were removed (ADR-0111 decision 4) — every @address other than the built-in words is an address space error"
        );
    }

    #[test]
    fn namespace_rules_are_strict() {
        for bad in ["", "@", " @ ", "@@x", "@a@b", "coders"] {
            assert!(
                matches!(
                    normalize_group_name(bad),
                    Err(GroupError::InvalidName { .. })
                ),
                "'{bad}' must be a convention violation"
            );
        }
        assert_eq!(
            normalize_group_name(" @all ").unwrap(),
            "@all",
            "only trims"
        );
    }

    #[test]
    fn builtin_source_satisfies_the_group_source_contract() {
        assert_group_source_contract(&BuiltinGroups, &builtin_fixture());
    }

    fn builtin_fixture() -> GroupSourceFixture {
        GroupSourceFixture {
            unknown: "@nope".to_string(),
            handles_builtins: true,
        }
    }

    struct SortingSource;
    impl GroupSource for SortingSource {
        fn resolve(
            &self,
            group: &str,
            pools: MemberPools<'_>,
        ) -> Result<Vec<GroupMember>, GroupError> {
            let norm = normalize_group_name(group)?;
            if norm == ALL_GROUP || norm == HERE_GROUP {
                let mut v = pools.live.to_vec();
                if norm == ALL_GROUP {
                    v.extend_from_slice(pools.dormant);
                }
                v.sort_by(|a, b| a.name.cmp(&b.name));
                return Ok(v);
            }
            Err(GroupError::NotFound { name: norm })
        }
    }

    #[test]
    #[should_panic(expected = "axis1-all-verbatim")]
    fn contract_suite_catches_a_source_that_sorts_all() {
        assert_group_source_contract(&SortingSource, &builtin_fixture());
    }

    /// ★ADR-0309 net★ — a source that folds by name (one of the same-name twins vanishes from the expansion).
    struct NameFoldingSource;
    impl GroupSource for NameFoldingSource {
        fn resolve(
            &self,
            group: &str,
            pools: MemberPools<'_>,
        ) -> Result<Vec<GroupMember>, GroupError> {
            let norm = normalize_group_name(group)?;
            if norm == ALL_GROUP || norm == HERE_GROUP {
                let dormant: &[GroupMember] = if norm == ALL_GROUP {
                    pools.dormant
                } else {
                    &[]
                };
                let mut v: Vec<GroupMember> = Vec::new();
                for m in pools.live.iter().chain(dormant) {
                    if !v.iter().any(|seen| seen.name == m.name) {
                        v.push(m.clone());
                    }
                }
                return Ok(v);
            }
            Err(GroupError::NotFound { name: norm })
        }
    }

    #[test]
    #[should_panic(expected = "axis1-all-verbatim")]
    fn contract_suite_catches_a_source_that_folds_namesakes() {
        assert_group_source_contract(&NameFoldingSource, &builtin_fixture());
    }

    /// ★ADR-0121 net★ — regression of the old `@all` (reading only the roster).
    struct LiveOnlyAllSource;
    impl GroupSource for LiveOnlyAllSource {
        fn resolve(
            &self,
            group: &str,
            pools: MemberPools<'_>,
        ) -> Result<Vec<GroupMember>, GroupError> {
            let norm = normalize_group_name(group)?;
            if norm == ALL_GROUP || norm == HERE_GROUP {
                return Ok(pools.live.to_vec());
            }
            Err(GroupError::NotFound { name: norm })
        }
    }

    #[test]
    #[should_panic(expected = "axis1-all-verbatim")]
    fn contract_suite_catches_an_all_that_ignores_dormant_names() {
        assert_group_source_contract(&LiveOnlyAllSource, &builtin_fixture());
    }

    /// ★ADR-0121 net (opposite direction)★ — regression where `@here` reads dormant.
    struct HereReadsDormantSource;
    impl GroupSource for HereReadsDormantSource {
        fn resolve(
            &self,
            group: &str,
            pools: MemberPools<'_>,
        ) -> Result<Vec<GroupMember>, GroupError> {
            let norm = normalize_group_name(group)?;
            if norm == ALL_GROUP || norm == HERE_GROUP {
                let mut v = pools.live.to_vec();
                v.extend_from_slice(pools.dormant);
                return Ok(v);
            }
            Err(GroupError::NotFound { name: norm })
        }
    }

    #[test]
    #[should_panic(expected = "axis1b-here-live-only")]
    fn contract_suite_catches_a_here_that_reads_dormant_names() {
        assert_group_source_contract(&HereReadsDormantSource, &builtin_fixture());
    }

    /// Regression net for the old `GroupError::Empty` — axis2 must bite (catch it).
    struct EmptyRejectingSource;
    impl GroupSource for EmptyRejectingSource {
        fn resolve(
            &self,
            group: &str,
            pools: MemberPools<'_>,
        ) -> Result<Vec<GroupMember>, GroupError> {
            let norm = normalize_group_name(group)?;
            if norm == ALL_GROUP || norm == HERE_GROUP {
                if pools.live.is_empty() && pools.dormant.is_empty() {
                    return Err(GroupError::NotFound { name: norm });
                }
                let mut v = pools.live.to_vec();
                if norm == ALL_GROUP {
                    v.extend_from_slice(pools.dormant);
                }
                return Ok(v);
            }
            Err(GroupError::NotFound { name: norm })
        }
    }

    #[test]
    #[should_panic(expected = "axis2-all-empty-ok")]
    fn contract_suite_catches_a_source_that_rejects_an_empty_snapshot() {
        assert_group_source_contract(&EmptyRejectingSource, &builtin_fixture());
    }
}
