//! The reusable reference walk: what a preset reaches ([`closure`]), who refers to an item ([`referrers_of`]) and how
//! many ([`reference_count`] — the `library.list` row).
//!
//! - Only library references count. A built-in (`engram:/…`) is not a library item, and a preset's folder entry is not
//!   an asset reference at all (ADR-0341) — neither is walked or counted.
//! - The preset side comes from the catalog (`used_by` — direct and through nested presets); the agent side is read
//!   from the profile registry each time (agent references are never stored — M5): an agent refers to an item when one
//!   of its attached preset paths names it by platform `same_path`.
// ADR-0341
// TRD S25 step-7 §3-5

use std::collections::HashSet;
use std::path::Path;

use engram_dashboard_agent::profile::ProfileRegistry;
use engram_dashboard_agent::types::AgentId;
use engram_dashboard_platform::fs::same_path;

use crate::catalog::{key, preset_refs, Catalog, ItemContent};
use crate::path::{AssetRef, LibPath};

/// Every library reference `preset` reaches, each once (by catalog key) as the first referencing preset spells it:
/// its lists in order (presets, instructions, skills, settings), each nested preset the catalog holds followed at once
/// by what it reaches (depth first). `preset` itself is not in it; a reference to a missing item is, but is not walked
/// into. Empty when `preset` is not a preset in the catalog.
// An explicit stack, not recursion: a long chain of nested presets would overflow the thread's stack, which aborts the
//   process.
pub fn closure(catalog: &Catalog, preset: &LibPath) -> Vec<AssetRef> {
    let mut reached = Vec::new();
    let mut seen = HashSet::from([key(preset)]);
    // Per preset being walked, its references not yet visited.
    let mut stack = vec![direct_refs(catalog, preset).into_iter()];
    while let Some(refs) = stack.last_mut() {
        let Some(target) = refs.next() else {
            stack.pop();
            continue;
        };
        if seen.insert(key(&target)) {
            reached.push(AssetRef::Library(target.clone()));
            stack.push(direct_refs(catalog, &target).into_iter());
        }
    }
    reached
}

/// The library references `preset` lists, in list order; empty when it is not a preset in the catalog.
fn direct_refs(catalog: &Catalog, preset: &LibPath) -> Vec<LibPath> {
    match catalog.get(&key(preset)).map(|item| &item.content) {
        Some(ItemContent::Preset { doc }) => preset_refs(doc).cloned().collect(),
        _ => Vec::new(),
    }
}

/// The presets that reach `item` (directly or through nested presets, `item` itself excluded), by path, and the
/// agents with a preset attached at `item`'s path (`same_path`), in id order.
pub fn referrers_of(
    catalog: &Catalog,
    registry: &ProfileRegistry,
    item: &LibPath,
) -> (Vec<LibPath>, Vec<AgentId>) {
    let presets = catalog
        .presets_reaching(&key(item))
        .into_iter()
        .map(|preset| preset.path.clone())
        .collect();
    let mut agents: Vec<AgentId> = registry
        .attachments()
        .into_iter()
        .filter(|(_, attached)| {
            attached
                .presets
                .iter()
                .any(|preset| same_path(Path::new(&preset.path), Path::new(item.as_str())))
        })
        .map(|(id, _)| id)
        .collect();
    agents.sort_unstable();
    (presets, agents)
}

/// The number of presets plus agents that refer to `item` ([`referrers_of`]).
pub fn reference_count(catalog: &Catalog, registry: &ProfileRegistry, item: &LibPath) -> usize {
    let (presets, agents) = referrers_of(catalog, registry, item);
    presets.len() + agents.len()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use engram_dashboard_agent::profile::{
        AgentCommand, AgentProfile, AttachedPreset, ProfileStore, StoreError,
    };

    use super::*;
    use crate::catalog::harness::{lp, Lib};

    /// A registry loaded with fixed profiles; saves succeed and are dropped.
    struct Fixed(Vec<AgentProfile>);

    impl ProfileStore for Fixed {
        fn save(&self, _: &[AgentProfile]) -> Result<(), StoreError> {
            Ok(())
        }
        fn load(&self) -> Vec<AgentProfile> {
            self.0.clone()
        }
    }

    fn agent(presets: &[&str]) -> AgentProfile {
        let command = AgentCommand::Shell {
            program: "sh".to_owned(),
            args: Vec::new(),
        };
        let mut profile = AgentProfile::new(
            "a".to_owned(),
            command,
            PathBuf::from("/work"),
            Vec::new(),
            false,
        );
        profile.presets = presets
            .iter()
            .map(|path| AttachedPreset {
                path: (*path).to_owned(),
            })
            .collect();
        profile
    }

    fn registry(profiles: Vec<AgentProfile>) -> ProfileRegistry {
        ProfileRegistry::new(Arc::new(Fixed(profiles)))
    }

    fn spellings(refs: &[AssetRef]) -> Vec<String> {
        refs.iter().map(AssetRef::spelling).collect()
    }

    #[test]
    fn closure_walks_nested_presets_depth_first_once_each_without_builtins_or_folders() {
        let mut lib = Lib::new();
        lib.store.file(
            "top.yaml",
            "kind: preset\nversion: 1\npresets:\n  - mid.yaml\ninstructions:\n  - engram:/priming\n  - rules/a.md\n\
             skills:\n  - skills/s\nsettings:\n  - s.yaml\nfolders:\n  - /abs/folder\n",
        );
        lib.store.file(
            "mid.yaml",
            "kind: preset\nversion: 1\npresets:\n  - top.yaml\ninstructions:\n  - rules/b.md\n  - rules/a.md\n  - rules/missing.md\n",
        );
        lib.store.file("rules/a.md", "a");
        lib.store.file("rules/b.md", "b");
        lib.scan(None);
        assert_eq!(
            spellings(&closure(&lib.catalog, &lp("top.yaml"))),
            [
                "mid.yaml",
                "rules/b.md",
                "rules/a.md",
                "rules/missing.md",
                "skills/s",
                "s.yaml"
            ]
        );
        assert_eq!(
            spellings(&closure(&lib.catalog, &lp("mid.yaml"))),
            [
                "top.yaml",
                "rules/a.md",
                "skills/s",
                "s.yaml",
                "rules/b.md",
                "rules/missing.md"
            ]
        );
        assert!(
            closure(&lib.catalog, &lp("rules/a.md")).is_empty(),
            "not a preset"
        );
    }

    #[test]
    fn closure_walks_a_long_preset_chain_without_recursion() {
        use crate::catalog::{Item, ItemKind};
        use crate::format::PresetDoc;
        use crate::path::PresetItem;
        use crate::store::ContentTag;

        // p0 -> p1 -> … -> p(N-1) -> x.md, applied straight into the catalog (a scan of that many files is slow).
        const N: usize = 20_000;
        let tag = ContentTag {
            modified: std::time::SystemTime::UNIX_EPOCH,
            size: 0,
            identity: None,
        };
        let preset = |i: usize| {
            let next = if i + 1 < N {
                format!("p{}.yaml", i + 1)
            } else {
                "x.md".to_owned()
            };
            let item = PresetItem {
                target: AssetRef::Library(lp(&next)),
                backend: None,
                line: 1,
                origin: None,
            };
            let (presets, instructions) = if i + 1 < N {
                (vec![item], vec![])
            } else {
                (vec![], vec![item])
            };
            Item {
                path: lp(&format!("p{i}.yaml")),
                kind: ItemKind::Preset,
                tag,
                read_seq: 1,
                content: ItemContent::Preset {
                    doc: PresetDoc {
                        version: 1,
                        description: None,
                        presets,
                        instructions,
                        skills: vec![],
                        settings: vec![],
                        folders: vec![],
                        unmodeled: vec![],
                        line_problems: vec![],
                    },
                },
            }
        };
        let mut catalog = Catalog::default();
        catalog.apply_items((0..N).map(preset).collect(), true);
        let reached = closure(&catalog, &lp("p0.yaml"));
        assert_eq!(reached.len(), N);
        assert_eq!(reached[0].spelling(), "p1.yaml");
        assert_eq!(reached[N - 1].spelling(), "x.md");
    }

    #[test]
    fn referrers_are_presets_direct_and_nested_plus_agents_attached_by_same_path() {
        let mut lib = Lib::new();
        lib.store.file("rules/a.md", "a");
        lib.store.file(
            "top.yaml",
            "kind: preset\nversion: 1\npresets:\n  - mid.yaml\n",
        );
        lib.store.file(
            "mid.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - rules/a.md\n",
        );
        lib.scan(None);
        let exact = agent(&["mid.yaml"]);
        let other_case = agent(&["MID.yaml"]);
        let elsewhere = agent(&["top.yaml", "engram:/default"]);
        let registry = registry(vec![exact.clone(), other_case.clone(), elsewhere.clone()]);

        let mut want_agents = vec![exact.id];
        if same_path(Path::new("MID.yaml"), Path::new("mid.yaml")) {
            want_agents.push(other_case.id);
        }
        want_agents.sort_unstable();
        let (presets, agents) = referrers_of(&lib.catalog, &registry, &lp("mid.yaml"));
        assert_eq!(presets, [lp("top.yaml")]);
        assert_eq!(agents, want_agents);
        assert_eq!(
            reference_count(&lib.catalog, &registry, &lp("mid.yaml")),
            1 + want_agents.len()
        );

        let (presets, agents) = referrers_of(&lib.catalog, &registry, &lp("rules/a.md"));
        assert_eq!(
            presets,
            [lp("mid.yaml"), lp("top.yaml")],
            "direct and through a nested preset"
        );
        assert!(
            agents.is_empty(),
            "agents attach presets, not the files they list"
        );
        assert_eq!(reference_count(&lib.catalog, &registry, &lp("top.yaml")), 1);
    }

    #[test]
    fn inserting_a_previously_missing_child_counts_its_referrers() {
        let mut lib = Lib::new();
        lib.store.file("x.md", "x");
        lib.store.file(
            "parent.yaml",
            "kind: preset\nversion: 1\npresets:\n  - child.yaml\n",
        );
        lib.scan(None);
        let registry = registry(Vec::new());
        assert_eq!(reference_count(&lib.catalog, &registry, &lp("x.md")), 0);

        lib.store.file(
            "child.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - x.md\n",
        );
        lib.scan(None);
        // The parent reaches x.md only through the child it listed while the child was missing — so the parent was
        //   found again when the child was inserted.
        let (presets, _) = referrers_of(&lib.catalog, &registry, &lp("x.md"));
        assert_eq!(presets, [lp("child.yaml"), lp("parent.yaml")]);
    }
}
