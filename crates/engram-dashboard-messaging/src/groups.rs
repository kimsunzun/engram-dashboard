//! groups — `@` address resolution seam (spec §4 · ADR-0104 decision 1 · **ADR-0111 decision 4 · ADR-0112 decision 1 ·
//! ADR-0121 decision 1**).
//!
//! ★Role★: expands "`@` address → member (id · name) list". **The v1 source is a single built-in one, with two vocabulary words inside it**
//!   (ADR-0121): **`@here`** = everyone alive right now · **`@all`** = everyone in the registry (alive **+ sleeping**). The expanded members
//!   carry **the registry's id as is** (ADR-0309 decision 3 — no name → id round trip). The same profile has the same id even when launched again,
//!   so it is the same member across re-spawns, and the address (`@all`) and the response notation are names (ADR-0101).
//!
//! ★The fact that the two words read **different list pools** is the decision itself (load-bearing — ADR-0121 §impact)★: making them
//!   see the same pool would turn "all" into a false name again (the sending LLM believes "everyone was told" while missing a sleeping peer)
//!   and `@here` would lose its reason to exist. So the contract suite below asserts the source separation of the two words as an axis.
//!
//! ★A group is an **address-resolution macro**, not a send path (ADR-0111 decision 4 — this module's reason to exist)★: the expanded
//!   members flow through **exactly the same** path as directly named recipients (spec §5 multi-recipient fan-out). The group-specific
//!   delivery rules (skipping dead members · binding `(id,epoch)` at the send moment · a dedicated fan-out branch) were **all** abolished —
//!   reviving them violates ADR-0111.
//!
//! ★User-defined groups were removed (ADR-0111 decision 4 · ADR-0112 decision 1)★: the runtime-registered list (registry) ·
//!   the `group` MCP tool · the CLI `group` subcommand · ADR-0109 management semantics are all gone. Stored, named
//!   groups will be **redesigned** when real demand stands up (folder groups are the first candidate) — no storage structure for groups
//!   is laid down in advance.
//!
//! ★The resolution seam is kept (the *direction* of ADR-0104 decision 1 survives, only the sources shrink)★: the `GroupSource` extension point is left so a future source (folder `@폴더명`)
//!   can be added without touching the messaging pipeline. The v1 implementation is
//!   `BuiltinGroups` (= `@all`·`@here`) alone. **No over-build** — folder/hierarchy implementations are not built now.
//!
//! ★Pure, mechanical resolution (load-bearing boundary, ADR-0104)★: `resolve` **only expands** to a member list —
//!   alive decisions, sender exclusion, deduplication and roster matching are all the upper layer's (`MessagingService`) job. In particular
//!   **"expansion result of 0 members" is not an error here** (the empty list is returned as is): `GROUP_EMPTY` is a decision raised only when the **final recipient set** —
//!   expansion plus explicit picks combined — is empty (ADR-0114 decision 3), so this layer cannot know.
// ADR-0111
// ADR-0112

use crate::PeerId;

/// Built-in word `@all` — **everyone in the registry** (alive + sleeping) **− sender** (spec §4 · ADR-0121 decision 1).
pub const ALL_GROUP: &str = "@all";

/// Built-in word `@here` — **everyone alive right now** **− sender** (spec §4 · ADR-0121 decision 1).
///
/// ★Basis for the word choice★: Slack uses the same distinction (`@channel` = everyone / `@here` = currently active) — the LLM driving the agent
///   likely already knows that convention (ADR-0121 §rationale).
pub const HERE_GROUP: &str = "@here";

/// One member of the expansion — the id the registry gave and the name at that moment.
///
/// The upper layer picks the recipient by this id directly — resolving again by name would fold same-name twins (different agents) into one
/// recipient or turn them into `RECIPIENT_AMBIGUOUS` (ADR-0309 decision 3 · TRD S29 §4 X1).
// ADR-0309
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupMember {
    pub id: PeerId,
    pub name: String,
}

/// The **two list pools** the resolver reads (ADR-0121 decision 1) — the caller builds them from a snapshot taken outside the lock and injects them.
///
/// ★Why two pools and not one slice (load-bearing — so source knowledge does not leak into the pipeline)★: "which word
///   reads which pool" is **knowledge of the source**. If the upper layer picked a different slice per word and passed it in,
///   the pipeline would have to know `@all`/`@here`, and then a future source (folder `@폴더명`) could not pick its own pool
///   (ADR-0104 "it is a violation if source knowledge leaks into the pipeline"). So **both** pools are passed and the source chooses.
/// ★Sender exclusion arrives already applied by the caller★ (spec §4 — the sender is removed from the expansion. That the source of truth is the spec and not ADR-0111
///   is load-bearing: decision 4 of 0111 lacks that phrase, so looking only there you would miss it).
#[derive(Debug, Clone, Copy)]
pub struct MemberPools<'a> {
    /// Members alive right now — the list of `@here` and the first part of `@all`.
    pub live: &'a [GroupMember],
    /// Sleeping members (profile exists, its session is not alive) — **only `@all`** reads this too.
    ///   ★It arrives with same names not folded★: because the contract of this layer is to **return the pool the caller gave verbatim** (no sorting,
    ///   dedup or filtering) — two members with the same name are different recipients (ADR-0309 · TRD S29 §4 X1).
    pub dormant: &'a [GroupMember],
}

/// `@` address resolution error — the upper layer maps it to a wire error code (spec §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupError {
    /// `@` namespace convention violation — normalization failed.
    InvalidName { name: String },
    /// An `@이름` that is not a built-in word — a nonexistent address → `GROUP_NOT_FOUND` (the whole send unit is bounced, ADR-0114 decision 3).
    NotFound { name: String },
}

/// ★Group address normalization — a **seam-level** free function (C4 review fix F · ADR-0104 decision 1)★
///
/// ★Why a free function and not a method of a source implementation (load-bearing — seam boundary)★: the send pipeline normalizes the name **before**
///   asking the source (the envelope `to` token and the error hint use that value). If that normalization were a method of a specific source
///   implementation, the pipeline would be tied to that type, and a future source with a different name grammar would become unreachable (ADR-0104
///   "it is a violation if source knowledge leaks into the pipeline"). So the convention is placed at the seam level outside the sources and shared by all.
///
/// ★Strict convention (load-bearing — @-namespace contract)★: a group name **must start with exactly one leading `@`**.
/// If there is no leading `@` (`coders`) it is `InvalidName` — lenient correction (`coders` → `@coders`) was rejected because it blurs the contract in which the person/group
/// distinction hangs on a single `@`. If the body after `@` is empty or only whitespace, or the body has another `@`
/// (`@@x`·`@a@b`), it is likewise `InvalidName`.
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

/// Group resolution source (seam — ADR-0104 decision 1). The extension point of the "`@` address → member list" resolver.
pub trait GroupSource {
    /// `NotFound` if this source does not know that address.
    fn resolve(&self, group: &str, pools: MemberPools<'_>) -> Result<Vec<GroupMember>, GroupError>;
}

/// The only v1 source — it knows only the two built-in words (`@all`·`@here`) (no storage, computed on the spot, ADR-0111 decision 4 · ADR-0121 decision 1).
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

/// ★`GroupSource` contract suite — a safety net for durability under source swaps (user instruction 2026-07-26)★
///
/// ★Why it exists (load-bearing — this module's reason to exist)★: `GroupSource` is a seam laid down **on the premise that it will be swapped in the future**
///   (ADR-0104 decision 1 — once folders become daemon-owned, an `@폴더명` source is added). The worst case is that the new implementation
///   quietly takes on a different meaning: `@all` comes out sorted, **`@here` reads sleeping names too**,
///   an empty snapshot is folded into an error, or the `@` convention turns lenient — all of these **compile and leave existing tests green**
///   while only the broadcast meaning flips. So there is a reusable suite that asserts the resolution semantics **against the trait**, not the
///   implementation.
///
/// ★When this suite goes red★: what to fix is **the decision, not the suite**. The resolution semantics are user-decided matters set by spec §4 · ADR-0111
///   decision 4 · ADR-0114 decision 3 · ADR-0121 decision 1, so a fix that loosens the assertions here
///   is a contract change — do not relax them without user ratification.
#[cfg(any(test, feature = "test-harness"))]
pub mod contract {
    use super::{GroupError, GroupMember, GroupSource, MemberPools, ALL_GROUP, HERE_GROUP};
    use crate::PeerId;

    #[derive(Debug, Clone)]
    pub struct GroupSourceFixture {
        /// An address name this source **does not know** (the `NotFound` axis). It must already be in normalized form (`@nope`).
        pub unknown: String,
        /// Does this source **own** the built-in words (`@all`·`@here`). v1 `BuiltinGroups` = true.
        ///   A future folder source = usually false.
        pub handles_builtins: bool,
    }

    /// A different id for each member (sequence from `base`) — if several share one name, those are same-name twins.
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

    /// ★`GroupSource` resolution semantics contract — any implementation must pass this (spec §4 · ADR-0111/0114/0121)★.
    ///
    /// ★The `axisN-…:` token before assertion messages (load-bearing)★: every failure message starts with a token that is **unique per
    ///   axis**. The self-check tests confirm "the net bites" with `#[should_panic(expected = …)]`, and if a substring
    ///   overlaps across axes, the wrong axis blowing up first still comes out green. **If you change a token, fix the pin too.**
    pub fn assert_group_source_contract(src: &dyn GroupSource, fx: &GroupSourceFixture) {
        // ── 1·1b·1c·2. Built-in word axis ────────────────────────────────────────────────────────
        // Sorting would scatter the upper layer's row-order rule (explicit tokens → expansion alphabetical) across two layers, and
        //   deduping by name would fold same-name twins (different ids) into one recipient at the source stage (ADR-0309 · TRD S29 §4 X1).
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
                "axis1-all-verbatim: @all 은 live + dormant 를 그대로 이어 붙여야 한다(잠든 이름 포함·정렬·dedup·필터 금지 — spec §4 · ADR-0121 결정 1)"
            );
            assert_eq!(
                src.resolve(HERE_GROUP, pools),
                Ok(live.clone()),
                "axis1b-here-live-only: @here 는 live 스냅샷만 그대로여야 한다(잠든 이름이 끼면 '지금 여기' 가 아니다 — ADR-0121 결정 1)"
            );
            assert_ne!(
                src.resolve(ALL_GROUP, pools),
                src.resolve(HERE_GROUP, pools),
                "axis1c-vocab-split: 잠든 이름이 있으면 @all 과 @here 는 **다른 명단**이어야 한다 — 같은 소스를 보면 'all' 이 다시 거짓 이름이 된다(ADR-0121 §영향)"
            );
            assert_eq!(
                src.resolve(ALL_GROUP, empty),
                Ok(Vec::new()),
                "axis2-all-empty-ok: 명부가 빈 @all 은 빈 목록(에러 아님) — GROUP_EMPTY 는 명시 지목까지 합친 최종 집합으로 상위가 판정한다(ADR-0114 결정 3)"
            );
            assert_eq!(
                src.resolve(HERE_GROUP, empty),
                Ok(Vec::new()),
                "axis2c-here-empty-ok: 산 수신자 0명인 @here 도 빈 목록(에러 아님 — 같은 근거)"
            );
        } else {
            for builtin in [ALL_GROUP, HERE_GROUP] {
                assert!(
                    matches!(
                        src.resolve(builtin, pools),
                        Err(GroupError::NotFound { .. })
                    ),
                    "axis2b-builtin-not-owned: 내장 어휘({builtin})를 소유하지 않는 소스는 그걸 제 것인 양 해석하면 안 된다(소스 순회에서 방송 의미 가로채기 금지)"
                );
            }
        }

        // ── 3. NotFound axis ────────────────────────────────────────────────────────────────
        assert_eq!(
            src.resolve(&fx.unknown, pools),
            Err(GroupError::NotFound {
                name: fx.unknown.clone()
            }),
            "axis3-unknown-notfound: 모르는 주소는 NotFound 이고 name 은 정규화된 이름을 실어야 한다(상위가 hint 에 되쓴다)"
        );

        // ── 4. `@` namespace convention ───────────────────────────────────────────────────────
        for bad in ["", "@", "  @  ", "@@x", "@@", "@a@b", "plain-name"] {
            assert!(
                matches!(src.resolve(bad, pools), Err(GroupError::InvalidName { .. })),
                "axis4-namespace-strict: '{bad}' 은 @ 네임스페이스 규약 위반이라 InvalidName 이어야 한다(조용한 보정 금지)"
            );
        }

        // ── 5. Determinism — the same input twice, the same answer ────────────────────────────────────────────
        // If this breaks, the "one snapshot at the send moment" contract loses its meaning (two questions at the same moment see different worlds).
        assert_eq!(
            src.resolve(&fx.unknown, pools),
            src.resolve(&fx.unknown, pools),
            "axis5-deterministic: 해석은 결정적이어야 한다(순수 — 시계·내부 가변 상태 금지)"
        );
        if fx.handles_builtins {
            for builtin in [ALL_GROUP, HERE_GROUP] {
                assert_eq!(
                    src.resolve(builtin, pools),
                    src.resolve(builtin, pools),
                    "axis5-deterministic-builtin: 내장 어휘({builtin}) 해석도 결정적이어야 한다"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::contract::{assert_group_source_contract, GroupSourceFixture};
    use super::*;

    /// A random id for each member — writing the same name twice makes same-name twins.
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
            "@here 는 산 명단만 본다(잠든 멤버 불포함 — ADR-0121 결정 1)"
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
            "풀은 verbatim 이다 — 동명 쌍둥이도 id 가 달라 둘 다 남는다(ADR-0309 · TRD S29 §4 X1)"
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
            "사용자 정의 그룹은 제거됐다(ADR-0111 결정 4) — 내장 어휘 외의 @주소는 전부 주소 공간 오류"
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
                "'{bad}' 은 규약 위반이어야 한다"
            );
        }
        assert_eq!(
            normalize_group_name(" @all ").unwrap(),
            "@all",
            "trim 만 한다"
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
