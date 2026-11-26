//! Profile persistence — the store of the profile file ([`PROFILES_FILE`]).
//!
//! ★The old version's `agents.json` is not this store's file★ (ADR-0314 decision 5 — old data is dropped without asking) — it is
//! never read, changed or deleted, and its temp-file names are not swept. So a folder holding only the old file starts with an empty
//! roster, and downgrading to an old build and back leaves each version's file untouched — that is why there is no version marker ·
//! migration · backup. Do not make the old name read again — old-version profiles (old agents) would mix into the new roster.
//!
//! The rules are base `file`'s data-file rules (ADR-0291):
//! - Writes are atomic (temp in the same folder → rename) — no half file on a crash.
//! - ★「may it be overwritten」 is decided on every save by re-reading the current file right before it★ — a file from a newer edition or an
//!   unreadable file is neither overwritten nor copied aside, and that save is refused. A damaged file is copied aside to `<name>.corrupt`
//!   before overwriting; if the copy-aside fails, that save fails too (original left as is). Loading does not change the file.
//! - ★A load-time refusal (newer edition · unreadable) means nothing is saved for the whole run★ — the in-memory registry did not come
//!   from that file, so even if the file later becomes readable, writing an empty registry over it would lose the whole registry.
//!
//! ★Serializing saves is the caller's job★ — the registry saves while holding its own map lock (ADR-0071) and is the only production caller.
//! This module holds one leaf lock (the store-state field) and does no IO, logging or outside calls under it — lock order =
//! `latch → expected field → profiles → store-state field`.
// ADR-0291
// ADR-0314

use std::io;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use engram_dashboard_base::file::{
    self, Claim, Loaded, OsHooks, Parsed, Refused, WritePolicy, BUSY_PAUSE, BUSY_RETRIES,
};
use engram_dashboard_base::sync;
use engram_dashboard_platform::fs::{retry_busy, sync_dir, Retry};
use engram_dashboard_platform::process::pid_alive;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::profile::{AgentProfile, ProfileStore, Refusal, StoreError, StoreStatus};

/// File name of the profile store — the text that tells the caller of a save refusal or failure names this file.
/// ★Change the same-named line in `.gitignore` with it★ — profiles carry plaintext `env` (ADR-0264 decision 7).
pub const PROFILES_FILE: &str = "agent-profiles.json";

const PROFILES: FileKind = FileKind {
    name: PROFILES_FILE,
    list_key: "profiles",
    spec: file::Spec {
        version_key: "schema_version",
        current: 1,
        // ADR-0291 R6: about ten thousand times the measured size (1,747 B on this PC) — never reached in normal use, a line that protects
        //   memory from garbage files.
        cap: 16 * 1024 * 1024,
        shape: profiles_shape,
    },
};

#[derive(Serialize)]
struct ProfilesFile<'a> {
    schema_version: u64,
    profiles: &'a [AgentProfile],
}

pub struct FileProfileStore {
    inner: FileStore,
}

impl FileProfileStore {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            inner: FileStore::new(dir, PROFILES),
        }
    }
}

impl ProfileStore for FileProfileStore {
    fn save(&self, profiles: &[AgentProfile]) -> Result<(), StoreError> {
        warn_if_secret(profiles);
        self.inner.save(
            profiles.len(),
            &ProfilesFile {
                schema_version: PROFILES.spec.current,
                profiles,
            },
        )
    }

    fn load(&self) -> Vec<AgentProfile> {
        self.inner.load()
    }

    fn status(&self) -> StoreStatus {
        self.inner.status()
    }
}

fn profiles_shape(version: u64, doc: &Map<String, Value>) -> Result<(), String> {
    list_shape::<AgentProfile>(&PROFILES, version, doc)
}

/// Warns if env has keys that look like credentials (security). Does not block persisting; the log tells about the plaintext storage risk.
fn warn_if_secret(profiles: &[AgentProfile]) {
    const NEEDLES: [&str; 4] = ["KEY", "TOKEN", "SECRET", "PASSWORD"];
    for p in profiles {
        for (k, _) in &p.env {
            let upper = k.to_uppercase();
            if NEEDLES.iter().any(|n| upper.contains(n)) {
                tracing::warn!(
                    agent = %p.id,
                    env_key = %k,
                    file = PROFILES_FILE,
                    "profile env has a key that looks like a credential — stored in plaintext in the profile file. Do not put credentials in env."
                );
            }
        }
    }
}

// ── OS share ────────────────────────────────────────────────────────────────────────

const DATA_HOOKS: OsHooks = OsHooks {
    retry: retry_data,
    sync_dir,
};

/// The load-read budget — longer than the data-file one. Every data file the daemon reads once at boot uses it: this
/// store's file, asset-library `library.json` and daemon `identity.json`.
// ADR-0291 R14 (D7): a lock still held after it makes the file read-only for the whole run (R17), so it is long enough
//   that one brief hold does not bind the run. The upper end is the shell's daemon-discovery deadline (5 s — on
//   failure `Down` with no retry): the boot loads run before the connection record is published, about 1 s each in
//   the worst case, about 3 s for the three files, which leaves the rest of the boot its share.
pub const LOAD_HOOKS: OsHooks = OsHooks {
    retry: retry_load,
    sync_dir,
};
const LOAD_RETRIES: u32 = 19;
const LOAD_PAUSE: Duration = Duration::from_millis(50);

fn retry_data(attempt: &mut dyn FnMut() -> io::Result<()>) -> io::Result<()> {
    retry_busy(
        Retry {
            retries: BUSY_RETRIES,
            pause: BUSY_PAUSE,
        },
        attempt,
    )
}

fn retry_load(attempt: &mut dyn FnMut() -> io::Result<()>) -> io::Result<()> {
    retry_busy(
        Retry {
            retries: LOAD_RETRIES,
            pause: LOAD_PAUSE,
        },
        attempt,
    )
}

// ── Common store ──────────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct FileKind {
    name: &'static str,
    /// The top-level field carrying the list.
    list_key: &'static str,
    spec: file::Spec,
}

struct FileStore {
    dir: PathBuf,
    kind: FileKind,
    /// The store-state field — a leaf lock. Held only at the moment of reading and writing (not held during decisions, disk IO or logging).
    // ADR-0291 R12
    state: Mutex<StoreState>,
}

struct StoreState {
    loaded: bool,
    /// The refusal of the load decision — stays for the whole run (R17).
    load_refusal: Option<Refusal>,
    /// The re-decision refusal of the last save — erased before writing when the next re-decision accepts (D6 — a later write failure is not a
    /// refusal). While the same reason continues, the log level is lowered (R15).
    last_refusal: Option<Refusal>,
    /// `NotYet` before the first load, or if the load found nothing or damage and nothing has been written yet (D14).
    claim: Claim,
}

/// What the first load leaves in the state field.
enum LoadVerdict {
    /// The registry came from that file.
    Adopted,
    /// Missing · damaged — the registry did not come from that file.
    NotAdopted,
    Refused(Refusal),
}

impl FileStore {
    fn new(dir: PathBuf, kind: FileKind) -> Self {
        Self {
            dir,
            kind,
            state: Mutex::new(StoreState {
                loaded: false,
                load_refusal: None,
                last_refusal: None,
                claim: Claim::NotYet,
            }),
        }
    }

    fn path(&self) -> PathBuf {
        self.dir.join(self.kind.name)
    }

    /// Reads the file and returns the list — an empty list if missing, damaged or refused. Does not change the file.
    ///
    /// ★The load refusal and `Claim` are set only on the first call★ — later calls only read, and do not lower a `Claim` raised by an
    /// earlier successful write. The first call also sweeps leftover temp files (this process's too), so call it when no save of the same
    ///   file is in progress (registry creation).
    fn load<T: DeserializeOwned>(&self) -> Vec<T> {
        let first = !std::mem::replace(&mut sync::lock(&self.state).loaded, true);
        if first {
            // Assumes one store per path in a process (production: the daemon assembly creates one per path). A second store on the same
            //   path could have its first load remove the temp file the first store is writing (this pid's numbered form).
            self.sweep_temps();
        }
        let name = self.kind.name;
        let (items, verdict) = match file::load(&self.path(), &self.kind.spec, LOAD_HOOKS) {
            Loaded::Missing => {
                tracing::debug!(
                    file = name,
                    "the file does not exist — starting with an empty list"
                );
                (Vec::new(), LoadVerdict::NotAdopted)
            }
            Loaded::Parsed(Parsed::Usable { doc, .. }) => match read_list(&doc, self.kind.list_key)
            {
                Ok(items) => (items, LoadVerdict::Adopted),
                // The shape check already passed the same function — getting here means the two disagree, so the file is not claimed as ours
                //   (copied aside before the first save overwrites it).
                Err(reason) => {
                    tracing::error!(
                        file = name,
                        %reason,
                        "could not read the list of a file that passed the shape check — starting with an empty list"
                    );
                    (Vec::new(), LoadVerdict::NotAdopted)
                }
            },
            Loaded::Parsed(Parsed::Unusable(reason)) => {
                tracing::error!(
                    file = name,
                    %reason,
                    "the file is damaged — starting with an empty list; it is copied aside to .corrupt before the first save overwrites it"
                );
                (Vec::new(), LoadVerdict::NotAdopted)
            }
            // Only the first load sets read-only — later loads are just reads, so they do not emit that text.
            Loaded::Parsed(Parsed::Newer { found }) => {
                let current = self.kind.spec.current;
                if first {
                    tracing::error!(
                        file = name,
                        found,
                        current,
                        "the file was written by a newer edition than this daemon — not read and not overwritten. Starting with an empty list and not saving during this \
                         run (restarting the daemon clears it)"
                    );
                } else {
                    tracing::debug!(
                        file = name,
                        found,
                        current,
                        "not read because the file was written by a newer edition"
                    );
                }
                (Vec::new(), LoadVerdict::Refused(Refusal::Newer { found }))
            }
            Loaded::Failed(error) => {
                if first {
                    tracing::error!(
                        file = name,
                        %error,
                        "could not read the file — not overwriting. Starting with an empty list and not saving during this run \
                         (restarting the daemon clears it)"
                    );
                } else {
                    tracing::debug!(file = name, %error, "could not read the file");
                }
                (Vec::new(), LoadVerdict::Refused(Refusal::Unreadable))
            }
        };
        if first {
            let mut state = sync::lock(&self.state);
            match verdict {
                LoadVerdict::Adopted => state.claim = Claim::Adopted,
                LoadVerdict::NotAdopted => {}
                LoadVerdict::Refused(refusal) => state.load_refusal = Some(refusal),
            }
        }
        items
    }

    /// Re-decides the current file right before writing, then writes. [`StoreError::ReadOnly`] on a load refusal · [`StoreError::Refused`]
    /// on a re-decision refusal (no write) · [`StoreError::Io`] on a serialization, cap, copy-aside or write failure (file left as is).
    ///   Logs are emitted here.
    // ADR-0291 R8 · R15 · R16 · R17
    fn save(&self, count: usize, payload: &impl Serialize) -> Result<(), StoreError> {
        let name = self.kind.name;
        let (load_refusal, claim) = {
            let state = sync::lock(&self.state);
            (state.load_refusal, state.claim)
        };
        if let Some(refusal) = load_refusal {
            tracing::debug!(
                file = name,
                %refusal,
                "not saving because the file was refused at load (restarting the daemon clears it)"
            );
            return Err(StoreError::ReadOnly(refusal));
        }

        let failed = |what: &str, error: io::Error| {
            tracing::error!(file = name, %error, "{what} — not saved (the file is left as is)");
            StoreError::Io(error)
        };
        let bytes = serde_json::to_vec_pretty(payload).map_err(|e| {
            failed(
                "serialization failed",
                io::Error::new(io::ErrorKind::InvalidData, e),
            )
        })?;
        file::check_cap(bytes.len(), &self.kind.spec)
            .map_err(|e| failed("the text to write exceeds the read cap", e))?;

        let path = self.path();
        let current = file::load(&path, &self.kind.spec, DATA_HOOKS);
        let policy = current.write_policy(claim);
        if let WritePolicy::Refuse(refused) = policy {
            return Err(self.refused(refused, &current));
        }
        // ★If the re-decision accepted, the refusal marker is cleared before writing★ — a later copy-aside or write failure is `Io`, not a
        //   refusal; clearing only on write success would leave `agent.list` showing a stale `Refusing` during that failure (D6).
        self.accepted();
        if policy == WritePolicy::CopyAsideFirst {
            let what = match current {
                Loaded::Parsed(Parsed::Usable { .. }) => {
                    "a usable file placed while this run had not written yet"
                }
                _ => "the damaged file",
            };
            match file::copy_aside(&path, DATA_HOOKS) {
                Ok(to) => tracing::warn!(
                    file = name,
                    to = %to.display(),
                    "copied aside {what} before overwriting"
                ),
                Err(error) => return Err(failed(&format!("could not copy aside {what}"), error)),
            }
        }

        file::write_atomic(&path, &bytes, DATA_HOOKS)
            .map_err(|e| failed("could not write the file", e))?;
        sync::lock(&self.state).claim = Claim::Adopted;
        tracing::debug!(file = name, count, "saved");
        Ok(())
    }

    /// The re-decision accepted — erases the earlier re-decision refusal, and if there was one, records once that it cleared (R15).
    fn accepted(&self) {
        let cleared = sync::lock(&self.state).last_refusal.take();
        if let Some(refusal) = cleared {
            tracing::info!(
                file = self.kind.name,
                %refusal,
                "this re-decision accepted the file that was refused earlier — writing now"
            );
        }
    }

    // ADR-0291 R15
    fn refused(&self, refused: Refused, current: &Loaded) -> StoreError {
        let name = self.kind.name;
        let refusal = match refused {
            Refused::Newer { found } => Refusal::Newer { found },
            Refused::ReadFailed => Refusal::Unreadable,
        };
        let repeated = sync::lock(&self.state).last_refusal.replace(refusal) == Some(refusal);
        if repeated {
            tracing::debug!(file = name, %refusal, "refused the save again — not overwriting");
        } else if let Loaded::Failed(error) = current {
            tracing::error!(
                file = name,
                %error,
                "cannot read the current file, so not overwriting — refusing this save (the next save decides again)"
            );
        } else {
            tracing::error!(
                file = name,
                %refusal,
                "cannot overwrite the current file — refusing this save (the next save decides again)"
            );
        }
        StoreError::Refused(refusal)
    }

    fn status(&self) -> StoreStatus {
        let state = sync::lock(&self.state);
        match (state.load_refusal, state.last_refusal) {
            (Some(refusal), _) => StoreStatus::ReadOnly(refusal),
            (None, Some(refusal)) => StoreStatus::Refusing(refusal),
            (None, None) => StoreStatus::Writable,
        }
    }

    fn sweep_temps(&self) {
        let name = self.kind.name;
        match file::sweep_temps(&self.dir, &[name], pid_alive) {
            Ok(swept) => {
                for (path, removed) in swept {
                    match removed {
                        Ok(()) => tracing::debug!(
                            file = name,
                            path = %path.display(),
                            "removed a leftover temp file"
                        ),
                        Err(error) => tracing::warn!(
                            file = name,
                            path = %path.display(),
                            %error,
                            "could not remove a leftover temp file"
                        ),
                    }
                }
            }
            Err(error) => tracing::warn!(
                file = name,
                %error,
                "could not find leftover temp files — proceeding without cleanup"
            ),
        }
    }
}

/// The owner's shape check (`Spec::shape`) — loading gets the value through the same [`read_list`], so the two definitions of damage
///   cannot diverge.
// ADR-0291 R7 (D1)
fn list_shape<T: DeserializeOwned>(
    kind: &FileKind,
    version: u64,
    doc: &Map<String, Value>,
) -> Result<(), String> {
    // There is no reader for earlier editions — when the edition is raised, fix this line together with that edition's reader (R3).
    if version != kind.spec.current {
        return Err(format!(
            "no reader for {} {version} (this build is {})",
            kind.spec.version_key, kind.spec.current
        ));
    }
    read_list::<T>(doc, kind.list_key).map(drop)
}

/// Reads the list field (`key`). The version key is not required — `file::parse` has already looked at the edition (no key = edition 1).
fn read_list<T: DeserializeOwned>(doc: &Map<String, Value>, key: &str) -> Result<Vec<T>, String> {
    let Some(list) = doc.get(key) else {
        return Err(format!("the `{key}` field is missing"));
    };
    let Some(items) = list.as_array() else {
        return Err(format!("the `{key}` field is not an array"));
    };
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            T::deserialize(item).map_err(|e| {
                // The serde_json error text is not carried — it copies field values (「invalid type: string "…"」), so logging it would leak
                //   values like the profile env. Line and column are always 0 (read from the value tree), so the item number is carried instead.
                format!(
                    "could not read item {index} of `{key}` (category {:?})",
                    e.classify()
                )
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::profile::{AgentCommand, AttachedPreset};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("engram-persist-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn sample() -> AgentProfile {
        AgentProfile::new(
            "t".into(),
            AgentCommand::Shell {
                program: "cmd.exe".into(),
                args: vec![],
            },
            PathBuf::from("."),
            vec![],
            true,
        )
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = temp_dir("roundtrip");
        let store = FileProfileStore::new(dir.clone());
        let p = sample();
        let id = p.id;
        store.save(&[p]).unwrap();

        let loaded = store.load();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, id);
        let _ = fs::remove_dir_all(&dir);
    }

    /// The `epoch` key is a leftover placeholder no reader reads — fix this test too when removing it (rationale = the
    /// `AgentProfile::epoch` comment).
    #[test]
    fn the_profile_file_carries_a_zero_epoch_placeholder_not_the_live_tag() {
        let dir = temp_dir("epoch-key");
        let store = FileProfileStore::new(dir.clone());
        let mut p = sample();
        p.epoch = 0xDEAD_BEEF;
        store.save(&[p]).unwrap();

        let raw = fs::read(dir.join(PROFILES.name)).expect("read the profile file");
        let v: serde_json::Value = serde_json::from_slice(&raw).expect("JSON parse");
        assert_eq!(
            v["profiles"][0].get("epoch"),
            Some(&serde_json::json!(0)),
            "the placeholder key is carried as 0, not as the live marker: {v}"
        );

        assert_eq!(
            store.load()[0].epoch,
            0,
            "even on re-reading, the marker does not come from the file (the next spawn issues a new one)"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_missing_is_empty() {
        let dir = temp_dir("missing");
        let store = FileProfileStore::new(dir.clone());
        assert!(store.load().is_empty());
        assert_eq!(store.status(), StoreStatus::Writable);
    }

    // ADR-0264
    #[test]
    fn the_repo_gitignore_covers_the_profile_file_and_its_side_copies() {
        const GITIGNORE: &str = include_str!("../../../../.gitignore");
        let rule = format!("**/data/daemon/state/{PROFILES_FILE}*");
        assert!(
            GITIGNORE.lines().any(|line| line.trim() == rule),
            "`.gitignore` has no `{rule}` line"
        );
    }

    // ── old-version file · new fields ──

    /// The shape of the old version's `agents.json` — as that version's reader read it (no new fields).
    const OLD_AGENTS_JSON: &str = r#"{
  "schema_version": 1,
  "profiles": [
    {
      "id": "00000000-0000-0000-0000-0000000000a1",
      "name": "C:\\work\\old",
      "display_name": "old",
      "parent_id": null,
      "command": { "kind": "Claude", "extra_args": [], "output_format": "Terminal" },
      "cwd": "C:\\work\\old",
      "env": [],
      "backend_session_id": "00000000-0000-0000-0000-0000000000b1",
      "old_session_ids": [],
      "epoch": 0,
      "auto_restore": true,
      "restart_policy": "Always",
      "restart_count": 0,
      "failed_reason": null,
      "created_at": 1,
      "last_active": 1,
      "last_start_at": null
    }
  ]
}"#;

    #[test]
    fn only_the_old_agents_json_present_starts_an_empty_roster_and_leaves_it_byte_for_byte() {
        let dir = temp_dir("old-only");
        put(&dir, "agents.json", OLD_AGENTS_JSON);
        let before = fs::read(dir.join("agents.json")).unwrap();

        let store = FileProfileStore::new(dir.clone());
        assert!(
            store.load().is_empty(),
            "an old-version agent mixed into the new roster"
        );
        assert_eq!(store.status(), StoreStatus::Writable);
        let p = sample();
        store.save(std::slice::from_ref(&p)).unwrap();

        assert_eq!(
            fs::read(dir.join("agents.json")).unwrap(),
            before,
            "the old-version file is neither read nor changed"
        );
        assert!(!dir.join("agents.json.corrupt").exists());
        let ours = FileProfileStore::new(dir.clone()).load();
        assert_eq!(ours.len(), 1);
        assert_eq!(ours[0].id, p.id);

        // The downgrade-and-upgrade path — even if the old-version file changes in between, the new roster neither mixes nor empties.
        put(
            &dir,
            "agents.json",
            &OLD_AGENTS_JSON.replace(r#""auto_restore": true"#, r#""auto_restore": false"#),
        );
        let reloaded = FileProfileStore::new(dir.clone()).load();
        assert_eq!(reloaded.len(), 1);
        assert_eq!(reloaded[0].id, p.id);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_new_fields_survive_a_save_and_reload_and_the_cwd_key_keeps_its_spelling() {
        let dir = temp_dir("new-fields");
        let mut p = sample();
        p.default_preset = false;
        p.presets = vec![
            AttachedPreset {
                path: "프리셋/리뷰.yaml".into(), // hangul-ok — non-ASCII path fixture
            },
            AttachedPreset {
                path: "공용/코딩 습관.yaml".into(), // hangul-ok — non-ASCII path fixture
            },
        ];
        p.last_applied_instructions_hash = Some("ab12".into());
        FileProfileStore::new(dir.clone())
            .save(std::slice::from_ref(&p))
            .unwrap();

        let raw: Value =
            serde_json::from_slice(&fs::read(dir.join(PROFILES.name)).unwrap()).unwrap();
        assert!(raw["profiles"][0].get("cwd").is_some(), "{raw}");

        let back = &FileProfileStore::new(dir.clone()).load()[0];
        assert!(!back.default_preset);
        assert_eq!(back.presets, p.presets, "attach order kept too");
        assert_eq!(back.last_applied_instructions_hash.as_deref(), Some("ab12"));
        assert_eq!(back.cwd, p.cwd);
        let _ = fs::remove_dir_all(&dir);
    }

    /// The notice fingerprint survives a daemon restart — a recorded empty set stays distinct from "nothing recorded",
    /// and an item without the key reads as nothing recorded.
    // ADR-0341
    #[test]
    fn the_notice_fingerprint_survives_a_save_and_reload() {
        let dir = temp_dir("notice-fingerprint");
        let mut shown = sample();
        shown.notice_fingerprint = Some("[\"omitted\",{}]".into());
        let mut empty = sample();
        empty.notice_fingerprint = Some(String::new());
        let never = sample();
        FileProfileStore::new(dir.clone())
            .save(&[shown.clone(), empty.clone(), never.clone()])
            .unwrap();

        let back = FileProfileStore::new(dir.clone()).load();
        let of = |id| {
            back.iter()
                .find(|p| p.id == id)
                .unwrap()
                .notice_fingerprint
                .clone()
        };
        assert_eq!(of(shown.id), shown.notice_fingerprint);
        assert_eq!(of(empty.id), Some(String::new()));
        assert_eq!(of(never.id), None);

        let _ = fs::remove_dir_all(&dir);

        let without_key = temp_dir("notice-fingerprint-missing");
        put(&without_key, PROFILES.name, OLD_AGENTS_JSON);
        assert_eq!(
            FileProfileStore::new(without_key.clone()).load()[0].notice_fingerprint,
            None
        );
        let _ = fs::remove_dir_all(&without_key);
    }

    /// An item without the new fields is not damage either — it reads as freshly created.
    #[test]
    fn an_item_without_the_new_fields_reads_as_freshly_created() {
        let dir = temp_dir("new-fields-missing");
        put(&dir, PROFILES.name, OLD_AGENTS_JSON);

        let store = FileProfileStore::new(dir.clone());
        let loaded = store.load();
        assert_eq!(loaded.len(), 1);
        assert!(loaded[0].default_preset);
        assert!(loaded[0].presets.is_empty());
        assert_eq!(loaded[0].last_applied_instructions_hash, None);
        store.save(&loaded).unwrap();
        assert!(
            !dir.join(aside_name()).exists(),
            "a loaded file is ours, so no side copy is made"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    // ── Damaged · newer edition · read failure ──

    fn file_text(profiles: &[AgentProfile]) -> String {
        serde_json::to_string_pretty(&ProfilesFile {
            schema_version: 1,
            profiles,
        })
        .unwrap()
    }

    fn put(dir: &std::path::Path, name: &str, text: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(name), text).unwrap();
    }

    fn read(dir: &std::path::Path, name: &str) -> String {
        fs::read_to_string(dir.join(name)).unwrap()
    }

    fn aside_name() -> String {
        format!("{}.corrupt", PROFILES.name)
    }

    #[test]
    fn a_corrupt_file_stays_in_place_until_the_first_save_copies_it_aside() {
        let dir = temp_dir("corrupt");
        put(&dir, PROFILES.name, "{ not valid json");

        let store = FileProfileStore::new(dir.clone());
        assert!(store.load().is_empty());
        assert_eq!(
            read(&dir, PROFILES.name),
            "{ not valid json",
            "loading does not change the file"
        );
        assert!(!dir.join(aside_name()).exists());
        assert_eq!(store.status(), StoreStatus::Writable);

        let p = sample();
        store.save(std::slice::from_ref(&p)).unwrap();
        assert_eq!(read(&dir, &aside_name()), "{ not valid json");
        assert_eq!(FileProfileStore::new(dir.clone()).load()[0].id, p.id);
        let _ = fs::remove_dir_all(&dir);
    }

    /// ★Does not keep holding the load decision★ — otherwise, after the first save, every save would copy aside our healthy file and erase
    ///   the real damaged copy.
    #[test]
    fn after_the_first_save_the_file_is_not_copied_aside_again() {
        let dir = temp_dir("corrupt-once");
        put(&dir, PROFILES.name, "{ not valid json");
        let store = FileProfileStore::new(dir.clone());
        store.load();
        store.save(&[sample()]).unwrap();
        assert_eq!(read(&dir, &aside_name()), "{ not valid json");

        store.save(&[sample(), sample()]).unwrap();
        assert_eq!(
            read(&dir, &aside_name()),
            "{ not valid json",
            "the second save copied aside our file and overwrote the damaged copy"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_with_one_unreadable_item_is_corrupt_and_copied_aside_before_the_first_save() {
        let dir = temp_dir("bad-kind");
        let mut doc: Value = serde_json::from_str(&file_text(&[sample(), sample()])).unwrap();
        doc["profiles"][1]["command"]["kind"] = Value::from("NoSuchBackend");
        let text = doc.to_string();
        put(&dir, PROFILES.name, &text);

        let store = FileProfileStore::new(dir.clone());
        assert!(
            store.load().is_empty(),
            "if one item cannot be read, the whole file is damaged"
        );
        store.save(&[sample()]).unwrap();
        assert_eq!(read(&dir, &aside_name()), text);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_newer_file_is_neither_read_nor_overwritten_for_the_whole_run() {
        let dir = temp_dir("newer");
        let newer = r#"{"schema_version":999,"profiles":[]}"#;
        put(&dir, PROFILES.name, newer);

        let store = FileProfileStore::new(dir.clone());
        assert!(store.load().is_empty());
        let refusal = Refusal::Newer { found: 999 };
        assert_eq!(store.status(), StoreStatus::ReadOnly(refusal));
        assert!(matches!(
            store.save(&[sample()]),
            Err(StoreError::ReadOnly(r)) if r == refusal
        ));
        assert_eq!(read(&dir, PROFILES.name), newer);
        assert!(
            !dir.join(aside_name()).exists(),
            "a newer-edition file is not even copied aside"
        );

        // The in-memory registry did not come from that file — even if the file becomes usable, this run does not write over it.
        let ours = file_text(&[sample()]);
        put(&dir, PROFILES.name, &ours);
        assert!(store.save(&[]).is_err());
        assert_eq!(read(&dir, PROFILES.name), ours);
        assert_eq!(store.status(), StoreStatus::ReadOnly(refusal));

        assert_eq!(
            FileProfileStore::new(dir.clone()).load().len(),
            1,
            "cleared by restarting"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unreadable_file_keeps_the_store_read_only_even_after_it_goes_away() {
        let dir = temp_dir("unreadable");
        fs::create_dir_all(dir.join(PROFILES.name)).unwrap();

        let store = FileProfileStore::new(dir.clone());
        assert!(store.load().is_empty());
        let refusal = Refusal::Unreadable;
        assert_eq!(store.status(), StoreStatus::ReadOnly(refusal));

        fs::remove_dir(dir.join(PROFILES.name)).unwrap();
        assert!(matches!(
            store.save(&[sample()]),
            Err(StoreError::ReadOnly(r)) if r == refusal
        ));
        assert!(
            !dir.join(PROFILES.name).exists(),
            "even after it is gone, an empty registry must not be written"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_rejudge_refusal_shows_as_refusing_until_a_save_succeeds() {
        let dir = temp_dir("refusing");
        let store = FileProfileStore::new(dir.clone());
        store.load();
        store.save(&[sample()]).unwrap();

        let newer = r#"{"schema_version":2,"profiles":[]}"#;
        put(&dir, PROFILES.name, newer);
        let refusal = Refusal::Newer { found: 2 };
        assert!(matches!(
            store.save(&[sample()]),
            Err(StoreError::Refused(r)) if r == refusal
        ));
        assert_eq!(store.status(), StoreStatus::Refusing(refusal));
        assert_eq!(read(&dir, PROFILES.name), newer);

        put(&dir, PROFILES.name, &file_text(&[]));
        let p = sample();
        store.save(std::slice::from_ref(&p)).unwrap();
        assert_eq!(store.status(), StoreStatus::Writable);
        assert_eq!(FileProfileStore::new(dir.clone()).load()[0].id, p.id);
        let _ = fs::remove_dir_all(&dir);
    }

    /// ★A write failure after the re-decision accepted is not a refusal★ — the marker is cleared at that re-decision and does not stay
    ///   `Refusing` during `Io` (D6). A folder placed at the copy-aside location makes the damaged file's copy-aside fail.
    // ADR-0291 R17 (D6)
    #[test]
    fn an_io_failure_after_an_accepting_rejudge_does_not_leave_the_store_refusing() {
        let dir = temp_dir("refusing-then-io");
        let store = FileProfileStore::new(dir.clone());
        store.load();
        store.save(&[sample()]).unwrap();

        put(&dir, PROFILES.name, r#"{"schema_version":2,"profiles":[]}"#);
        assert!(matches!(
            store.save(&[sample()]),
            Err(StoreError::Refused(_))
        ));
        assert!(matches!(store.status(), StoreStatus::Refusing(_)));

        put(&dir, PROFILES.name, "{broken");
        fs::create_dir_all(dir.join(aside_name())).unwrap();
        assert!(matches!(store.save(&[sample()]), Err(StoreError::Io(_))));
        assert_eq!(store.status(), StoreStatus::Writable);
        assert_eq!(
            read(&dir, PROFILES.name),
            "{broken",
            "the file is left as is"
        );

        fs::remove_dir(dir.join(aside_name())).unwrap();
        store.save(&[sample()]).unwrap();
        assert_eq!(store.status(), StoreStatus::Writable);
        let _ = fs::remove_dir_all(&dir);
    }

    /// If someone restored a file that was missing at load while the run was going, the first save copies it aside before overwriting —
    ///   only once.
    #[test]
    fn a_usable_file_placed_after_a_missing_load_is_copied_aside_once() {
        let dir = temp_dir("not-yet");
        let store = FileProfileStore::new(dir.clone());
        assert!(store.load().is_empty());

        let restored = file_text(&[sample()]);
        put(&dir, PROFILES.name, &restored);
        store.save(&[sample(), sample()]).unwrap();
        assert_eq!(read(&dir, &aside_name()), restored);

        fs::remove_file(dir.join(aside_name())).unwrap();
        store.save(&[sample()]).unwrap();
        assert!(
            !dir.join(aside_name()).exists(),
            "the file after one write is ours"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    // ── Read rules ──

    #[test]
    fn a_bom_prefixed_file_loads() {
        let dir = temp_dir("bom");
        let p = sample();
        put(
            &dir,
            PROFILES.name,
            &format!("\u{feff}{}", file_text(std::slice::from_ref(&p))),
        );
        assert_eq!(FileProfileStore::new(dir.clone()).load()[0].id, p.id);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_without_a_version_key_loads_as_the_first_version() {
        let dir = temp_dir("no-version");
        let p = sample();
        let mut doc: Value = serde_json::from_str(&file_text(std::slice::from_ref(&p))).unwrap();
        doc.as_object_mut().unwrap().remove("schema_version");
        put(&dir, PROFILES.name, &doc.to_string());

        let store = FileProfileStore::new(dir.clone());
        assert_eq!(store.load()[0].id, p.id);
        store.save(&[p]).unwrap();
        assert!(
            !dir.join(aside_name()).exists(),
            "a loaded file is ours, so it is not copied aside"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn version_zero_is_corrupt_not_refused() {
        let dir = temp_dir("version-zero");
        put(&dir, PROFILES.name, r#"{"schema_version":0,"profiles":[]}"#);
        let store = FileProfileStore::new(dir.clone());
        assert!(store.load().is_empty());
        assert_eq!(store.status(), StoreStatus::Writable);
        store.save(&[]).unwrap();
        assert!(dir.join(aside_name()).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_shape_failure_reason_does_not_echo_field_values() {
        let mut doc: Value = serde_json::from_str(&file_text(&[sample()])).unwrap();
        doc["profiles"][0]["auto_restore"] = Value::from("TOP-SECRET-VALUE");
        match file::parse(&doc.to_string(), &PROFILES.spec) {
            Parsed::Unusable(reason) => {
                assert!(!reason.contains("TOP-SECRET"), "{reason}");
                assert!(reason.contains("item 0"), "{reason}");
            }
            other => panic!("must be damaged: {other:?}"),
        }
    }

    // ── Startup sweep ──

    /// ★Old-version temp names are not swept★ — the old version's `agents.json` side is not touched by a single byte (module head).
    #[test]
    fn the_first_load_sweeps_our_own_numbered_temps_but_not_the_old_files() {
        let dir = temp_dir("sweep");
        let me = std::process::id();
        let own = format!("{}.tmp{me}.7", PROFILES.name);
        let old_numbered = format!("agents.json.tmp{me}.7");
        put(&dir, &own, "half");
        put(&dir, &format!("{}.tmpx", PROFILES.name), "not ours");
        put(&dir, "agents.json.tmp", "old fixed temp");
        put(&dir, &old_numbered, "old numbered temp");

        let store = FileProfileStore::new(dir.clone());
        store.load();
        assert!(!dir.join(&own).exists());
        assert!(
            dir.join(format!("{}.tmpx", PROFILES.name)).exists(),
            "names not in our form are left alone"
        );
        assert_eq!(read(&dir, "agents.json.tmp"), "old fixed temp");
        assert_eq!(read(&dir, &old_numbered), "old numbered temp");

        // Later loads do not sweep — removing the temp file of a save in progress in between would make its rename fail.
        put(&dir, &own, "in flight");
        store.load();
        assert!(dir.join(&own).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    // ── Production OS share ──

    /// ★Passing a retry that does nothing still compiles★ — this test catches that mistake.
    #[test]
    fn both_os_hooks_really_retry_a_locked_attempt() {
        for (label, hooks) in [("data", DATA_HOOKS), ("load read", LOAD_HOOKS)] {
            let mut attempts = 0;
            let outcome = (hooks.retry)(&mut || {
                attempts += 1;
                if attempts == 1 {
                    Err(io::Error::from(io::ErrorKind::PermissionDenied))
                } else {
                    Ok(())
                }
            });
            assert!(outcome.is_ok(), "{label}: {outcome:?}");
            assert_eq!(attempts, 2, "{label}");
        }
    }

    /// Catches the mistake of passing the same budget to both — if the lock is not released, each retries for its own budget and gives up.
    #[test]
    fn the_load_read_hook_waits_longer_than_the_data_hook() {
        let attempts_until_given_up = |hooks: OsHooks| {
            let mut attempts = 0;
            let outcome = (hooks.retry)(&mut || {
                attempts += 1;
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            });
            assert!(outcome.is_err());
            attempts
        };
        assert_eq!(attempts_until_given_up(DATA_HOOKS), BUSY_RETRIES + 1);
        assert_eq!(attempts_until_given_up(LOAD_HOOKS), LOAD_RETRIES + 1);
        assert!(LOAD_PAUSE * LOAD_RETRIES > BUSY_PAUSE * BUSY_RETRIES);
    }
}
