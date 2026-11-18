//! The daemon identity — `identity.json` in the daemon state folder, holding `daemon_id` (a lowercase hyphenated
//! uuid v4) that clients key their drafts on. The asset library carries the value (`LibraryValues::daemon_id`); it
//! never reads this file.
//!
//! Base data-file rules (ADR-0291 · ADR-0376): a missing file gets a new id written; a corrupt one — including one
//! whose `daemon_id` is not a uuid v4 (nil, another version) — is copied aside to `<name>.corrupt` first and a new id
//! written; a v4 is kept, read back in the lowercase hyphenated spelling. A newer or unreadable file, or one that cannot
//! be written, gives an id for this run only, not written, with a `warn` — that run's client drafts are orphaned (the
//! least harm).
// ADR-0264
// ADR-0291
// ADR-0311
// ADR-0376

use std::path::Path;

use engram_dashboard_agent::persistence::LOAD_HOOKS;
use engram_dashboard_base::file::{self, Claim, Loaded, Parsed, Spec, WritePolicy};
use engram_dashboard_platform::process::pid_alive;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::file_hooks::OS_HOOKS;

/// The file name, joined onto the daemon state folder (ADR-0264 decision 4: the store that uses a file name owns it).
pub const IDENTITY_FILE: &str = "identity.json";

const SPEC: Spec = Spec {
    version_key: "schema_version",
    current: 1,
    cap: 64 * 1024,
    shape,
};

fn shape(_version: u64, doc: &Map<String, Value>) -> Result<(), String> {
    id_of(doc).map(drop)
}

/// The `daemon_id` of `doc`, spelled lowercase hyphenated; anything but a uuid v4 is refused (a corrupt file).
fn id_of(doc: &Map<String, Value>) -> Result<String, String> {
    doc.get("daemon_id")
        .and_then(Value::as_str)
        .and_then(|id| Uuid::parse_str(id).ok())
        .filter(|id| id.get_version() == Some(uuid::Version::Random))
        .map(|id| id.hyphenated().to_string())
        .ok_or_else(|| "`daemon_id` is not a uuid v4".to_owned())
}

/// This run's `daemon_id` from `identity.json` under `state_dir`, creating it when missing or corrupt (module header).
/// Never fails.
pub fn load_or_create(state_dir: &Path) -> String {
    match file::sweep_temps(state_dir, &[IDENTITY_FILE], pid_alive) {
        Ok(swept) => {
            for (path, removed) in swept {
                if let Err(e) = removed {
                    tracing::debug!(path = %path.display(), "identity.json temp not removed: {e}");
                }
            }
        }
        Err(e) => {
            tracing::debug!(dir = %state_dir.display(), "identity.json temp sweep failed: {e}")
        }
    }
    let path = state_dir.join(IDENTITY_FILE);
    // ADR-0291 R14: a lock left after the long load budget gives a run-only id.
    let loaded = file::load(&path, &SPEC, LOAD_HOOKS);
    let id = Uuid::new_v4().hyphenated().to_string();
    let run_only = |why: String| {
        tracing::warn!(path = %path.display(), "identity.json unusable; daemon_id for this run only: {why}");
        id.clone()
    };
    match &loaded {
        Loaded::Parsed(Parsed::Usable { doc, .. }) => match id_of(doc) {
            Ok(id) => return id,
            Err(why) => return run_only(why),
        },
        Loaded::Parsed(Parsed::Newer { found }) => {
            return run_only(format!("newer version (schema_version {found})"))
        }
        Loaded::Failed(e) => return run_only(format!("read failed: {e}")),
        Loaded::Missing | Loaded::Parsed(Parsed::Unusable(_)) => {}
    }
    if loaded.write_policy(Claim::NotYet) == WritePolicy::CopyAsideFirst {
        match file::copy_aside(&path, OS_HOOKS) {
            Ok(to) => tracing::warn!(to = %to.display(), "identity.json corrupt; copied aside"),
            Err(e) => return run_only(format!("copy aside failed: {e}")),
        }
    }
    let bytes =
        serde_json::to_vec_pretty(&json!({ "schema_version": SPEC.current, "daemon_id": id }))
            .expect("a JSON object serializes");
    match file::write_atomic(&path, &bytes, OS_HOOKS) {
        Ok(()) => {
            tracing::info!(daemon_id = %id, "identity.json written");
            id
        }
        Err(e) => run_only(format!("write failed: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::log_capture::capture_loud;

    /// A fresh folder under the temp folder, removed on drop.
    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn scratch() -> Scratch {
        let dir = std::env::temp_dir().join(format!("engram-identity-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn file_id(dir: &Path) -> Option<String> {
        let text = std::fs::read_to_string(dir.join(IDENTITY_FILE)).ok()?;
        let doc: Value = serde_json::from_str(&text).unwrap();
        Some(doc["daemon_id"].as_str().unwrap().to_owned())
    }

    /// `load_or_create` inside a log capture (every call of its callsites runs inside one — `log_capture` "Observation
    /// conditions" ①), with the WARN and ERROR lines it made.
    fn run(dir: &Path) -> (String, Vec<String>) {
        capture_loud(|| load_or_create(dir))
    }

    #[test]
    fn a_missing_file_gets_a_new_v4_id_written_and_the_next_start_reads_it_back() {
        let dir = scratch();
        let (id, loud) = run(&dir.0);
        let parsed = Uuid::parse_str(&id).unwrap();
        assert_eq!(parsed.get_version_num(), 4);
        assert_eq!(id, parsed.hyphenated().to_string(), "lowercase hyphenated");
        assert_eq!(file_id(&dir.0), Some(id.clone()));
        assert!(loud.is_empty(), "{loud:?}");
        assert_eq!(run(&dir.0).0, id);
    }

    #[test]
    fn a_corrupt_file_is_copied_aside_and_replaced_by_a_new_id() {
        let dir = scratch();
        std::fs::write(dir.0.join(IDENTITY_FILE), "not json").unwrap();
        let (id, _) = run(&dir.0);
        assert_eq!(file_id(&dir.0), Some(id));
        assert_eq!(
            std::fs::read_to_string(dir.0.join("identity.json.corrupt")).unwrap(),
            "not json"
        );
    }

    #[test]
    fn an_id_that_is_not_a_uuid_v4_is_corrupt_and_replaced_while_a_v4_is_kept_in_lowercase() {
        for (bad, version) in [
            ("00000000-0000-0000-0000-000000000000", "nil"),
            ("c232ab00-9414-11ec-b3c8-9f6bdeced846", "v1"),
        ] {
            let dir = scratch();
            let text = json!({ "schema_version": 1, "daemon_id": bad }).to_string();
            std::fs::write(dir.0.join(IDENTITY_FILE), &text).unwrap();
            let (id, _) = run(&dir.0);
            assert_ne!(id, bad, "{version}");
            assert_eq!(
                Uuid::parse_str(&id).unwrap().get_version_num(),
                4,
                "{version}"
            );
            assert_eq!(file_id(&dir.0), Some(id), "{version}");
            assert_eq!(
                std::fs::read_to_string(dir.0.join("identity.json.corrupt")).unwrap(),
                text,
                "{version}"
            );
        }
        let dir = scratch();
        let v4 = Uuid::new_v4();
        let upper = v4.hyphenated().to_string().to_uppercase();
        std::fs::write(
            dir.0.join(IDENTITY_FILE),
            json!({ "schema_version": 1, "daemon_id": upper }).to_string(),
        )
        .unwrap();
        assert_eq!(run(&dir.0).0, v4.hyphenated().to_string());
    }

    #[test]
    fn a_newer_or_unreadable_file_gives_a_run_only_id_with_a_warn_and_is_not_written() {
        let newer = scratch();
        let text = r#"{"schema_version": 2, "daemon_id": "x"}"#;
        std::fs::write(newer.0.join(IDENTITY_FILE), text).unwrap();
        let unreadable = scratch();
        std::fs::create_dir(unreadable.0.join(IDENTITY_FILE)).unwrap();
        for dir in [&newer.0, &unreadable.0] {
            let (first, loud) = run(dir);
            let (second, _) = run(dir);
            assert_ne!(first, second, "a fresh id each run");
            assert!(
                loud.iter().any(|line| line.contains("for this run only")),
                "{loud:?}"
            );
        }
        assert_eq!(
            std::fs::read_to_string(newer.0.join(IDENTITY_FILE)).unwrap(),
            text
        );
        assert!(unreadable.0.join(IDENTITY_FILE).is_dir());
    }

    #[test]
    fn an_unwritable_place_gives_a_run_only_id_with_a_warn() {
        let dir = scratch();
        let state = dir.0.join("state");
        std::fs::write(&state, "a file where the folder should be").unwrap();
        let (id, loud) = run(&state);
        assert!(Uuid::parse_str(&id).is_ok());
        assert!(
            loud.iter().any(|line| line.contains("for this run only")),
            "{loud:?}"
        );
    }
}
