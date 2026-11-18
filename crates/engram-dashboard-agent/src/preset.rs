//! Presets — the single source of truth for the pre-spawn "cwd bookmark" list. (ADR-0061)
//!
//! Preset = a registered path shown in the background right-click "create agent" picker. It **means something different** from a profile
//! (agents.json — a spawned/reserved agent instance holding sid, epoch and restart policy): a path bookmark, not an instance, hence a
//!   separate store (presets.json) (the alternative ADR-0061 rejected).
//!
//! model/icon/inject fields are added when a real need appears (their absence now is intended).
//!
//! zero tauri imports — the same isolation rule as profile.rs.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::profile::{StoreError, StoreStatus};

pub type PresetId = Uuid;

// ── Persistent presets ───────────────────────────────────────────────────────────────

/// One preset — the unit stored in `presets.json` (ADR-0061).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub id: PresetId,
    /// Registered working directory (normalized). Without a name override, the name is derived from this path's basename (ADR-0061).
    pub cwd: PathBuf,
    /// User-set display name override (ADR-0061). `Some` → shown as is, `None` → derived from the cwd basename.
    /// `#[serde(default)]`, so an old presets.json without this field is absorbed as `None` (no migration needed).
    #[serde(default)]
    pub name: Option<String>,
}

// ── Persistence abstraction ────────────────────────────────────────────────────────

/// Preset persistence abstraction — implemented by the persistence module (mirror of FileProfileStore/ProfileStore). Trait injection lets
///   headless tests plug in an in-memory store.
pub trait PresetStore: Send + Sync + 'static {
    /// Saves the whole snapshot atomically. The implementation logs the failure — the caller only branches the flow on `Err`.
    fn save(&self, presets: &[Preset]) -> Result<(), StoreError>;
    /// Loaded once at boot. Empty list if missing, damaged or refused.
    fn load(&self) -> Vec<Preset>;
    /// Default = always usable (a store that never refuses).
    fn status(&self) -> StoreStatus {
        StoreStatus::Writable
    }
}

// ── PresetRegistry ─────────────────────────────────────────────────────────────

/// The in-memory **single owner** of presets (mirror of ProfileRegistry). Every CRUD goes through here and is persisted to the store
///   immediately on change.
///
/// Lock discipline (same as ProfileRegistry): disk IO (`store.save`) happens **while holding** the presets lock — mutate+save are one
/// critical section, so the last committed in-memory state is exactly the disk state (§5 concurrency consistency > lock-hold time).
/// Snapshot-then-unlock-then-save would let overlapping mutations leave memory and broadcast at B while the disk stays at A, and restart
/// would load the old value; an LLM/orchestrator calling rename/create/delete programmatically in succession actually hits that window (§5).
/// **No deadlock (unrelated to ADR-0006):** `store.save` takes only the store-state field (a leaf lock — no IO under it) and does not
/// re-enter the registry → lock order `presets → store-state field` one-way. The presets lock is separate from the session lock domain (ADR-0006 order). IO
/// size under the lock is the same as ProfileRegistry (re-decision right before saving · worst case = retrying while that file is locked —
///   ADR-0291 · D4).
///
/// Verbs starting with `try_` = changes with a caller — applied to a copy → saved → committed only on success; on `Err` memory is left as
/// is and the error returned (grounds = `ProfileRegistry` doc). ★Presets have no changes without a caller — so `ProfileRegistry`'s dirty
///   field is not kept here★.
// ADR-0291 R17
pub struct PresetRegistry {
    presets: Mutex<HashMap<PresetId, Preset>>,
    store: Arc<dyn PresetStore>,
}

impl PresetRegistry {
    pub fn new(store: Arc<dyn PresetStore>) -> Self {
        let loaded = store.load();
        let map = loaded.into_iter().map(|p| (p.id, p)).collect();
        Self {
            presets: Mutex::new(map),
            store,
        }
    }

    /// The common path for changes — applies to a copy and, **before releasing the lock**, saves that copy and commits only on success.
    /// Commit and persistence are one critical section, so persisted == observed (grounds = the struct lock discipline, ADR-0071).
    /// ★Every preset change has a caller (the WS preset set)★ — no internal entry applies to memory on a save `Err`, hence no dirty.
    // ADR-0291 R17
    fn try_mutate<R>(
        &self,
        f: impl FnOnce(&mut HashMap<PresetId, Preset>) -> R,
    ) -> Result<R, StoreError> {
        let mut guard = self.presets.lock().expect("presets poisoned");
        let mut draft = guard.clone();
        let result = f(&mut draft);
        let snapshot: Vec<Preset> = draft.values().cloned().collect();
        self.store.save(&snapshot)?;
        *guard = draft;
        Ok(result)
    }

    /// The store's current state — does not take the preset lock (the store-state field is the end of the lock order).
    // ADR-0291 R17 (D6)
    pub fn store_status(&self) -> StoreStatus {
        self.store.status()
    }

    pub fn list(&self) -> Vec<Preset> {
        self.presets
            .lock()
            .expect("presets poisoned")
            .values()
            .cloned()
            .collect()
    }

    /// Issues a new uuid + normalizes the cwd (`dunce::canonicalize` — on failure the input is kept as is). Commits only if saving succeeds.
    pub fn try_create(&self, cwd: PathBuf) -> Result<Preset, StoreError> {
        let preset = new_preset(cwd);
        let created = preset.clone();
        self.try_mutate(|m| {
            m.insert(preset.id, preset);
        })?;
        Ok(created)
    }

    /// Deletes a preset (for an unknown id the map stays as is — saving is still attempted). Commits only if saving succeeds. ★Deleting a
    /// preset ≠ terminating an agent★ (ADR-0061): agents already spawned from it are not touched (separate lifetimes).
    pub fn try_remove(&self, id: PresetId) -> Result<(), StoreError> {
        self.try_mutate(|m| {
            m.remove(&id);
        })
    }

    /// Sets/clears the preset display name override (ADR-0061 enrichment). `Some(name)` → save the override, `None` → clear (back to
    /// deriving from the cwd basename). `true` if it exists; for an unknown id the map stays as is and `false` (saving is still
    ///   attempted). Commits only if saving succeeds.
    /// ★Normalization is the frontend's responsibility★: trim, empty-string refusal and skipping unchanged happen before confirming (same
    /// shape as TabBar rename) — only an already-valid value or an explicit None comes here.
    pub fn try_rename(&self, id: PresetId, name: Option<String>) -> Result<bool, StoreError> {
        self.try_mutate(|m| rename_present(m, id, name))
    }
}

fn new_preset(cwd: PathBuf) -> Preset {
    // ★Why normalize★: the same folder under a different notation (case, relative path, UNC) would waver the frontend basename derivation
    //   and duplicate decision — the notation is fixed with canonicalize before saving (same policy as profile spawn).
    let cwd = dunce::canonicalize(&cwd).unwrap_or(cwd);
    Preset {
        id: Uuid::new_v4(),
        cwd,
        name: None,
    }
}

fn rename_present(m: &mut HashMap<PresetId, Preset>, id: PresetId, name: Option<String>) -> bool {
    match m.get_mut(&id) {
        Some(p) => {
            p.name = name;
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// In-memory store for tests — keeps the last saved snapshot for checking (mirror of ProfileRegistry MemStore).
    #[derive(Default)]
    struct MemStore {
        saved: Mutex<Vec<Preset>>,
        /// When on, saving returns a re-decision refusal (leaves nothing behind).
        refusing: std::sync::atomic::AtomicBool,
    }
    impl PresetStore for MemStore {
        fn save(&self, presets: &[Preset]) -> Result<(), StoreError> {
            if self.refusing.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(StoreError::Refused(crate::profile::Refusal::Unreadable));
            }
            *self.saved.lock().unwrap() = presets.to_vec();
            Ok(())
        }
        fn load(&self) -> Vec<Preset> {
            self.saved.lock().unwrap().clone()
        }
    }

    #[test]
    fn create_mints_uuid_and_persists() {
        let store = Arc::new(MemStore::default());
        let reg = PresetRegistry::new(store.clone());
        let p = reg.try_create(PathBuf::from(".")).expect("save succeeds");
        assert_eq!(reg.list().len(), 1);
        assert_eq!(store.load().len(), 1);
        assert_eq!(store.load()[0].id, p.id);
    }

    #[test]
    fn create_two_have_distinct_ids() {
        let reg = PresetRegistry::new(Arc::new(MemStore::default()));
        let a = reg.try_create(PathBuf::from(".")).expect("save succeeds");
        let b = reg.try_create(PathBuf::from(".")).expect("save succeeds");
        assert_ne!(a.id, b.id, "each create must issue a new uuid");
        assert_eq!(reg.list().len(), 2);
    }

    #[test]
    fn remove_deletes_and_persists() {
        let store = Arc::new(MemStore::default());
        let reg = PresetRegistry::new(store.clone());
        let p = reg.try_create(PathBuf::from(".")).expect("save succeeds");
        reg.try_remove(p.id).expect("save succeeds");
        assert!(reg.list().is_empty());
        assert!(
            store.load().is_empty(),
            "deletion is also persisted immediately"
        );
    }

    #[test]
    fn remove_missing_is_noop() {
        let reg = PresetRegistry::new(Arc::new(MemStore::default()));
        reg.try_create(PathBuf::from(".")).expect("save succeeds");
        reg.try_remove(Uuid::new_v4()).expect("save succeeds");
        assert_eq!(reg.list().len(), 1, "deleting an unknown id is a no-op");
    }

    #[test]
    fn load_restores_existing() {
        let store = Arc::new(MemStore::default());
        {
            let reg = PresetRegistry::new(store.clone());
            reg.try_create(PathBuf::from(".")).expect("save succeeds");
        }
        let reg2 = PresetRegistry::new(store.clone());
        assert_eq!(reg2.list().len(), 1);
    }

    // ── Name override (ADR-0061 enrichment) ──────────────────────────────────────────────

    #[test]
    fn create_starts_with_no_name_override() {
        let reg = PresetRegistry::new(Arc::new(MemStore::default()));
        let p = reg.try_create(PathBuf::from(".")).expect("save succeeds");
        assert_eq!(reg.list()[0].name, None);
        assert_eq!(p.name, None);
    }

    #[test]
    fn rename_sets_and_persists_name() {
        let store = Arc::new(MemStore::default());
        let reg = PresetRegistry::new(store.clone());
        let p = reg.try_create(PathBuf::from(".")).expect("save succeeds");
        assert!(reg
            .try_rename(p.id, Some("my preset".to_string()))
            .expect("save succeeds"));
        assert_eq!(reg.list()[0].name, Some("my preset".to_string()));
        assert_eq!(store.load()[0].name, Some("my preset".to_string()));
    }

    #[test]
    fn rename_none_clears_override() {
        let reg = PresetRegistry::new(Arc::new(MemStore::default()));
        let p = reg.try_create(PathBuf::from(".")).expect("save succeeds");
        reg.try_rename(p.id, Some("x".to_string()))
            .expect("save succeeds");
        assert!(reg.try_rename(p.id, None).expect("save succeeds"));
        assert_eq!(reg.list()[0].name, None);
    }

    #[test]
    fn rename_missing_is_noop_false() {
        let reg = PresetRegistry::new(Arc::new(MemStore::default()));
        reg.try_create(PathBuf::from(".")).expect("save succeeds");
        assert!(!reg
            .try_rename(Uuid::new_v4(), Some("y".to_string()))
            .expect("save succeeds"));
        assert_eq!(reg.list()[0].name, None);
    }

    // ── Concurrency: persisted == latest (sealing the stale-overwrite race) ──────────────────────

    /// Regression guard — in the old racy design (save outside the lock) this invariant wavered.
    #[test]
    fn save_writes_current_map_not_stale_snapshot() {
        let store = Arc::new(MemStore::default());
        let reg = PresetRegistry::new(store.clone());
        let p = reg.try_create(PathBuf::from(".")).expect("save succeeds");
        reg.try_rename(p.id, Some("final".to_string()))
            .expect("save succeeds");
        let disk = store.load();
        let mem = reg.list();
        assert_eq!(disk.len(), mem.len());
        assert_eq!(disk[0].name, Some("final".to_string()));
        assert_eq!(disk[0].name, mem[0].name, "persisted == observed");
    }

    /// Regression guard — with the old save-outside-the-lock design, A could overwrite the disk with a stale snapshot that did not see B's
    /// insert and **an entry could go missing**. Repeated create+rename widens the interleave window.
    #[test]
    fn concurrent_mutations_persisted_equals_final_map() {
        use std::thread;

        let store = Arc::new(MemStore::default());
        let reg = Arc::new(PresetRegistry::new(store.clone()));

        let mut handles = Vec::new();
        for t in 0..4 {
            let r = reg.clone();
            handles.push(thread::spawn(move || {
                for i in 0..50 {
                    let p = r.try_create(PathBuf::from(".")).expect("save succeeds");
                    r.try_rename(p.id, Some(format!("t{t}-{i}")))
                        .expect("save succeeds");
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let mem = reg.list();
        let disk = store.load();
        assert_eq!(mem.len(), 200, "200 in-memory creates");
        assert_eq!(
            disk.len(),
            mem.len(),
            "disk count == in-memory count (no entry missing through a stale snapshot)"
        );

        let mut mem_sorted: Vec<_> = mem.iter().map(|p| (p.id, p.name.clone())).collect();
        let mut disk_sorted: Vec<_> = disk.iter().map(|p| (p.id, p.name.clone())).collect();
        mem_sorted.sort();
        disk_sorted.sort();
        assert_eq!(
            disk_sorted, mem_sorted,
            "after concurrent mutations disk == latest in-memory (persisted == observed)"
        );
    }

    // ── Entries with a caller (ADR-0291 R17) ──────────────────────────────────────────

    #[test]
    fn a_caller_change_that_fails_to_save_leaves_presets_as_they_were() {
        let store = Arc::new(MemStore::default());
        let reg = PresetRegistry::new(store.clone());
        let kept = reg.try_create(PathBuf::from(".")).unwrap();
        assert_eq!(store.load(), vec![kept.clone()]);

        store
            .refusing
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(matches!(
            reg.try_create(PathBuf::from(".")),
            Err(StoreError::Refused(_))
        ));
        assert!(reg.try_rename(kept.id, Some("x".to_string())).is_err());
        assert!(reg.try_remove(kept.id).is_err());
        assert_eq!(
            reg.list(),
            vec![kept.clone()],
            "none of create, rename and delete was committed"
        );

        store
            .refusing
            .store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(reg.try_rename(kept.id, Some("x".to_string())).unwrap());
        assert!(!reg.try_rename(Uuid::new_v4(), None).unwrap());
        assert_eq!(store.load()[0].name.as_deref(), Some("x"));
        reg.try_remove(kept.id).unwrap();
        assert!(reg.list().is_empty());
        assert!(store.load().is_empty());
    }

    /// Fixture = an old presets.json without the `name` field (backward compatibility).
    #[test]
    fn deserializes_legacy_preset_without_name() {
        let legacy = r#"{ "id": "00000000-0000-0000-0000-000000000001", "cwd": "C:/proj" }"#;
        let p: Preset = serde_json::from_str(legacy).expect("legacy preset must deserialize");
        assert_eq!(p.name, None);
        assert_eq!(p.cwd, PathBuf::from("C:/proj"));
    }
}
