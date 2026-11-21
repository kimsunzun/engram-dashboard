//! The peer state over real files in a temp folder, with the OS behind a fake: sealing tags each blob with its account (another account
//!   cannot open it, as with DPAPI), unseal errors are injected, the identity lock is a shared in-memory set and the clock is fixed.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use engram_dashboard_platform::secret::UnsealError;

use super::*;
use crate::link::peer::trusted::{Helper, Permission};

const ACCOUNT_X: &str = "S-1-5-21-10-20-30-1001";
const ACCOUNT_Y: &str = "S-1-5-21-10-20-30-1002";
const MACHINE_1: &str = "aaaa-1111";
const MACHINE_2: &str = "bbbb-2222";

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("engram-peer-store-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone, Copy)]
enum FakeUnseal {
    Unreadable,
    Transient,
    Unsupported,
}

/// The OS fake. `master` stands for the account's DPAPI master key: a blob sealed under another master does not open.
struct FakeOs {
    master: u8,
    fail: Mutex<Option<FakeUnseal>>,
    now: Mutex<SystemTime>,
    locks: Arc<Mutex<HashSet<String>>>,
}

impl FakeOs {
    fn new(master: u8, locks: Arc<Mutex<HashSet<String>>>) -> Arc<Self> {
        Arc::new(Self {
            master,
            fail: Mutex::new(None),
            now: Mutex::new(UNIX_EPOCH + Duration::from_secs(1_795_257_005)),
            locks,
        })
    }

    fn alone(master: u8) -> Arc<Self> {
        Self::new(master, Arc::default())
    }

    fn fail_unseal(&self, how: Option<FakeUnseal>) {
        *self.fail.lock().unwrap() = how;
    }
}

struct Release {
    name: String,
    locks: Arc<Mutex<HashSet<String>>>,
}

impl Drop for Release {
    fn drop(&mut self) {
        self.locks.lock().unwrap().remove(&self.name);
    }
}

impl PeerOs for FakeOs {
    fn seal(&self, secret: &[u8]) -> io::Result<Vec<u8>> {
        let mut blob = vec![b'F', self.master];
        blob.extend(secret.iter().map(|b| b ^ self.master));
        Ok(blob)
    }

    fn unseal(&self, blob: &[u8]) -> Result<Vec<u8>, UnsealError> {
        let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_string());
        match *self.fail.lock().unwrap() {
            Some(FakeUnseal::Unreadable) => return Err(UnsealError::Unreadable(bad("bad data"))),
            Some(FakeUnseal::Transient) => {
                return Err(UnsealError::Transient(io::Error::other(
                    "master key not ready",
                )))
            }
            Some(FakeUnseal::Unsupported) => return Err(UnsealError::Unsupported),
            None => {}
        }
        match blob {
            [b'F', master, rest @ ..] if *master == self.master => {
                Ok(rest.iter().map(|b| b ^ self.master).collect())
            }
            [b'F', _, ..] => Err(UnsealError::Unreadable(bad(
                "not this account's master key",
            ))),
            _ => Err(UnsealError::Unreadable(bad("bad data"))),
        }
    }

    fn lock(&self, name: &str) -> LockAttempt {
        let mut locks = self.locks.lock().unwrap();
        if !locks.insert(name.to_string()) {
            return LockAttempt::HeldByThisAccount;
        }
        LockAttempt::Held(Box::new(Release {
            name: name.to_string(),
            locks: self.locks.clone(),
        }))
    }

    fn now(&self) -> SystemTime {
        *self.now.lock().unwrap()
    }
}

fn open(dir: &Path, account: &str, machine: &str, os: &Arc<FakeOs>) -> PeerStore {
    PeerStore::load(
        dir.to_path_buf(),
        Ok(account.to_string()),
        Ok(machine.to_string()),
        os.clone(),
    )
}

fn peer(n: u8, name: &str) -> TrustedPeer {
    TrustedPeer {
        id: PeerId::from_bytes([n; 32]),
        name: name.to_string(),
        address: None,
        permission: Permission::Mail,
    }
}

fn key_path(dir: &Path, account: &str, machine: &str) -> PathBuf {
    dir.join(format!("peer-key.{account}.{machine}"))
}

fn list_path(dir: &Path, account: &str, machine: &str) -> PathBuf {
    dir.join(format!("peers.{account}.{machine}.json"))
}

fn names(store: &PeerStore) -> Vec<String> {
    store.peers().iter().map(|p| p.name.clone()).collect()
}

/// A loaded store with one PC listed.
fn paired(dir: &Path, os: &Arc<FakeOs>) -> PeerStore {
    let mut store = open(dir, ACCOUNT_X, MACHINE_1, os);
    store.add(peer(1, "home")).unwrap();
    store
}

// ── identity ──

#[test]
fn no_key_file_gives_a_fresh_identity_on_first_need_without_a_prompt() {
    let dir = TempDir::new();
    let os = FakeOs::alone(1);
    let mut store = open(dir.path(), ACCOUNT_X, MACHINE_1, &os);
    assert_eq!(store.status(), PeerStatus::NoIdentityYet);
    assert_eq!(store.id(), None);
    assert!(
        !key_path(dir.path(), ACCOUNT_X, MACHINE_1).exists(),
        "loading creates nothing"
    );

    let id = store.ensure_identity().unwrap();
    assert_eq!(store.status(), PeerStatus::Ready);
    assert_eq!(store.ensure_identity().unwrap(), id, "made once");
    assert!(key_path(dir.path(), ACCOUNT_X, MACHINE_1).exists());

    drop(store); // a restart — the identity lock goes with the process
    let again = open(dir.path(), ACCOUNT_X, MACHINE_1, &os);
    assert_eq!(again.status(), PeerStatus::Ready);
    assert_eq!(again.id(), Some(id));
}

#[test]
fn the_key_file_holds_the_sealed_blob_not_the_secret() {
    let dir = TempDir::new();
    let os = FakeOs::alone(7);
    let mut store = open(dir.path(), ACCOUNT_X, MACHINE_1, &os);
    store.ensure_identity().unwrap();
    let on_disk = std::fs::read(key_path(dir.path(), ACCOUNT_X, MACHINE_1)).unwrap();
    let secret = store.secret().unwrap().expose().to_vec();
    assert_eq!(on_disk, os.seal(&secret).unwrap());
    assert_ne!(on_disk, secret);
}

#[test]
fn a_set_network_also_makes_the_key_on_first_need() {
    let dir = TempDir::new();
    let os = FakeOs::alone(1);
    let mut store = open(dir.path(), ACCOUNT_X, MACHINE_1, &os);
    let network = NetworkSettings {
        port: Some(5000),
        relay: Helper::None,
        ..NetworkSettings::default()
    };
    store.set_network(network.clone()).unwrap();
    assert!(store.id().is_some());
    drop(store);
    let again = open(dir.path(), ACCOUNT_X, MACHINE_1, &os);
    assert_eq!(again.network(), Some(&network));
}

#[test]
fn two_accounts_in_one_folder_keep_separate_keys_and_lists() {
    let dir = TempDir::new();
    let os_x = FakeOs::alone(1);
    let os_y = FakeOs::alone(2);
    let mut x = open(dir.path(), ACCOUNT_X, MACHINE_1, &os_x);
    x.add(peer(1, "home")).unwrap();
    let mut y = open(dir.path(), ACCOUNT_Y, MACHINE_1, &os_y);
    assert_eq!(
        y.status(),
        PeerStatus::NoIdentityYet,
        "x's files are not y's"
    );
    y.add(peer(2, "office")).unwrap();
    drop((x, y));

    let x = open(dir.path(), ACCOUNT_X, MACHINE_1, &os_x);
    let y = open(dir.path(), ACCOUNT_Y, MACHINE_1, &os_y);
    assert_eq!(
        (x.status(), y.status()),
        (PeerStatus::Ready, PeerStatus::Ready)
    );
    assert_ne!(x.id(), y.id());
    assert_eq!(names(&x), ["home"]);
    assert_eq!(names(&y), ["office"]);
}

#[test]
fn this_accounts_key_under_another_machine_id_asks_adopt_or_fresh_and_is_never_adopted_silently() {
    let dir = TempDir::new();
    let os = FakeOs::alone(1);
    let old_id = paired(dir.path(), &os).id().unwrap();

    let mut moved = open(dir.path(), ACCOUNT_X, MACHINE_2, &os);
    assert_eq!(
        moved.status(),
        PeerStatus::AdoptOrFresh {
            machines: vec![MACHINE_1.to_string()]
        }
    );
    assert_eq!(moved.id(), None);
    assert!(matches!(
        moved.ensure_identity(),
        Err(PeerError::Blocked(_))
    ));
    assert!(matches!(
        moved.add(peer(2, "x")),
        Err(PeerError::Blocked(_))
    ));
    assert!(matches!(moved.regenerate(), Err(PeerError::NotOffered(_))));
    assert!(!key_path(dir.path(), ACCOUNT_X, MACHINE_2).exists());

    // Loading again still asks — nothing was taken over meanwhile.
    let mut moved = open(dir.path(), ACCOUNT_X, MACHINE_2, &os);
    assert!(matches!(moved.status(), PeerStatus::AdoptOrFresh { .. }));
    assert!(matches!(
        moved.adopt("cccc-3333"),
        Err(PeerError::Invalid(_))
    ));
    moved.adopt(MACHINE_1).unwrap();
    assert_eq!(moved.status(), PeerStatus::Ready);
    assert_eq!(moved.id(), Some(old_id));
    assert_eq!(
        names(&moved),
        ["home"],
        "the list came along and its MAC holds"
    );
    assert!(
        key_path(dir.path(), ACCOUNT_X, MACHINE_1).exists(),
        "the other machine's files stay"
    );
}

#[test]
fn starting_fresh_instead_of_adopting_leaves_the_other_machines_files_alone() {
    let dir = TempDir::new();
    let os = FakeOs::alone(1);
    let old_id = paired(dir.path(), &os).id().unwrap();
    let old_key = std::fs::read(key_path(dir.path(), ACCOUNT_X, MACHINE_1)).unwrap();
    let old_list = std::fs::read(list_path(dir.path(), ACCOUNT_X, MACHINE_1)).unwrap();

    let mut moved = open(dir.path(), ACCOUNT_X, MACHINE_2, &os);
    let new_id = moved.start_fresh().unwrap();
    assert_ne!(new_id, old_id);
    assert_eq!(moved.status(), PeerStatus::Ready);
    assert!(moved.peers().is_empty());
    assert_eq!(
        std::fs::read(key_path(dir.path(), ACCOUNT_X, MACHINE_1)).unwrap(),
        old_key
    );
    assert_eq!(
        std::fs::read(list_path(dir.path(), ACCOUNT_X, MACHINE_1)).unwrap(),
        old_list
    );
    assert!(matches!(moved.start_fresh(), Err(PeerError::NotOffered(_))));
}

#[test]
fn adopting_a_key_this_account_cannot_open_lands_in_unreadable() {
    let dir = TempDir::new();
    let os = FakeOs::alone(1);
    paired(dir.path(), &os);
    // Same SID, another master key — a roaming copy or a forced password reset.
    let other_master = FakeOs::alone(9);
    let mut moved = open(dir.path(), ACCOUNT_X, MACHINE_2, &other_master);
    moved.adopt(MACHINE_1).unwrap();
    assert!(matches!(
        moved.status(),
        PeerStatus::KeyUnreadable(Unreadable::NotThisAccount(_))
    ));
}

#[test]
fn files_of_other_names_are_left_untouched() {
    let dir = TempDir::new();
    let others = [
        ("agents.json", "{}"),
        ("peer-key.S-1-5-21-other.aaaa-1111", "foreign"),
        ("peers.S-1-5-21-other.aaaa-1111.json", "foreign list"),
        (
            "peer-key.S-1-5-21-10-20-30-1001.aaaa-1111.note",
            "not a key name",
        ),
    ];
    for (name, text) in others {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    let os = FakeOs::alone(1);
    let mut store = paired(dir.path(), &os);
    store.add(peer(2, "office")).unwrap();
    store.regenerate().unwrap();
    store.accept_list().unwrap();
    for (name, text) in others {
        assert_eq!(
            std::fs::read_to_string(dir.path().join(name)).unwrap(),
            text,
            "{name}"
        );
    }
}

#[test]
fn a_blob_sealed_for_another_account_is_unreadable_and_is_not_replaced_on_its_own() {
    let dir = TempDir::new();
    paired(dir.path(), &FakeOs::alone(1));
    let blob = std::fs::read(key_path(dir.path(), ACCOUNT_X, MACHINE_1)).unwrap();

    let mut store = open(dir.path(), ACCOUNT_X, MACHINE_1, &FakeOs::alone(2));
    assert!(matches!(
        store.status(),
        PeerStatus::KeyUnreadable(Unreadable::NotThisAccount(_))
    ));
    assert_eq!(store.id(), None);
    assert!(matches!(
        store.ensure_identity(),
        Err(PeerError::Blocked(_))
    ));
    assert!(matches!(
        store.add(peer(2, "x")),
        Err(PeerError::Blocked(_))
    ));
    assert_eq!(
        std::fs::read(key_path(dir.path(), ACCOUNT_X, MACHINE_1)).unwrap(),
        blob
    );
}

#[test]
fn an_unreadable_unseal_is_unreadable_and_any_other_unseal_error_is_transient() {
    let dir = TempDir::new();
    let os = FakeOs::alone(1);
    let id = paired(dir.path(), &os).id().unwrap();

    os.fail_unseal(Some(FakeUnseal::Unreadable));
    let store = open(dir.path(), ACCOUNT_X, MACHINE_1, &os);
    assert!(matches!(
        store.status(),
        PeerStatus::KeyUnreadable(Unreadable::NotThisAccount(_))
    ));

    os.fail_unseal(Some(FakeUnseal::Transient));
    let mut store = open(dir.path(), ACCOUNT_X, MACHINE_1, &os);
    assert!(matches!(store.status(), PeerStatus::KeyTransient(_)));
    assert!(
        matches!(store.regenerate(), Err(PeerError::NotOffered(_))),
        "not offered for a transient failure"
    );
    assert!(store.retry(), "a transient failure is retried");
    assert!(matches!(store.status(), PeerStatus::KeyTransient(_)));
    os.fail_unseal(None);
    assert!(store.retry());
    assert_eq!(store.status(), PeerStatus::Ready);
    assert_eq!(store.id(), Some(id), "it recovers with the old ID");
    assert!(!store.retry(), "nothing to retry once ready");

    os.fail_unseal(Some(FakeUnseal::Unsupported));
    let store = open(dir.path(), ACCOUNT_X, MACHINE_1, &os);
    assert!(matches!(store.status(), PeerStatus::Unavailable(_)));
}

#[test]
fn a_missing_key_while_the_list_names_pcs_is_unreadable_and_not_recreated() {
    let dir = TempDir::new();
    let os = FakeOs::alone(1);
    paired(dir.path(), &os);
    std::fs::remove_file(key_path(dir.path(), ACCOUNT_X, MACHINE_1)).unwrap();

    let mut store = open(dir.path(), ACCOUNT_X, MACHINE_1, &os);
    assert_eq!(
        store.status(),
        PeerStatus::KeyUnreadable(Unreadable::MissingUnderList)
    );
    assert!(matches!(
        store.ensure_identity(),
        Err(PeerError::Blocked(_))
    ));
    assert!(!key_path(dir.path(), ACCOUNT_X, MACHINE_1).exists());
    // The list is not checked without its key; the human-approved regenerate is offered and then the list needs accepting.
    assert_eq!(names(&store), ["home"]);
    store.regenerate().unwrap();
    assert_eq!(store.status(), PeerStatus::ListChangedOutside);
}

#[test]
fn a_corrupt_key_file_is_unreadable() {
    let dir = TempDir::new();
    let os = FakeOs::alone(1);
    let key = key_path(dir.path(), ACCOUNT_X, MACHINE_1);
    let cases: [(&str, Vec<u8>); 4] = [
        ("empty", Vec::new()),
        ("garbage", b"not a blob".to_vec()),
        ("31 bytes sealed", os.seal(&[5; 31]).unwrap()),
        ("over the cap", vec![b'F'; (64 << 10) + 1]),
    ];
    for (what, bytes) in cases {
        std::fs::write(&key, &bytes).unwrap();
        let store = open(dir.path(), ACCOUNT_X, MACHINE_1, &os);
        assert!(
            matches!(store.status(), PeerStatus::KeyUnreadable(_)),
            "{what}: {:?}",
            store.status()
        );
        assert_eq!(std::fs::read(&key).unwrap(), bytes, "{what}: left as it is");
    }
}

#[test]
fn regenerate_keeps_timestamped_backups_changes_the_id_and_does_not_re_sign_the_list() {
    let dir = TempDir::new();
    let os = FakeOs::alone(1);
    let mut store = paired(dir.path(), &os);
    let first_id = store.id().unwrap();
    let first_blob = std::fs::read(key_path(dir.path(), ACCOUNT_X, MACHINE_1)).unwrap();
    let list_before = std::fs::read(list_path(dir.path(), ACCOUNT_X, MACHINE_1)).unwrap();

    let done = store.regenerate().unwrap();
    assert_eq!(done.old, Some(first_id));
    assert_ne!(done.id, first_id);
    assert_eq!(store.id(), Some(done.id));
    let first_backup = done.backup.clone().unwrap();
    assert_eq!(
        first_backup.file_name().unwrap().to_str().unwrap(),
        format!("peer-key.{ACCOUNT_X}.{MACHINE_1}.bak-20261121T103005Z")
    );
    assert_eq!(std::fs::read(&first_backup).unwrap(), first_blob);
    assert_eq!(
        std::fs::read(list_path(dir.path(), ACCOUNT_X, MACHINE_1)).unwrap(),
        list_before,
        "the list is not re-signed"
    );
    assert_eq!(store.status(), PeerStatus::ListChangedOutside);
    assert_eq!(names(&store), ["home"]);

    // A second regenerate in the same second keeps both backups.
    let second_blob = std::fs::read(key_path(dir.path(), ACCOUNT_X, MACHINE_1)).unwrap();
    let again = store.regenerate().unwrap();
    let second_backup = again.backup.unwrap();
    assert_ne!(second_backup, first_backup);
    assert_eq!(std::fs::read(&second_backup).unwrap(), second_blob);
    assert_eq!(std::fs::read(&first_backup).unwrap(), first_blob);
    assert_ne!(again.id, done.id);
}

#[test]
fn a_restart_between_regenerate_and_accept_brings_the_accept_state_back() {
    let dir = TempDir::new();
    let os = FakeOs::alone(1);
    let mut store = paired(dir.path(), &os);
    let new_id = store.regenerate().unwrap().id;
    drop(store);

    let store = open(dir.path(), ACCOUNT_X, MACHINE_1, &os);
    assert_eq!(store.id(), Some(new_id));
    assert_eq!(store.status(), PeerStatus::ListChangedOutside);
    drop(store);
    let mut store = open(dir.path(), ACCOUNT_X, MACHINE_1, &os);
    assert_eq!(
        store.status(),
        PeerStatus::ListChangedOutside,
        "every start until accepted"
    );

    store.accept_list().unwrap();
    assert_eq!(store.status(), PeerStatus::Ready);
    drop(store);
    let store = open(dir.path(), ACCOUNT_X, MACHINE_1, &os);
    assert_eq!(store.status(), PeerStatus::Ready);
    assert_eq!(names(&store), ["home"]);
}

// ── trusted list ──

/// Rewrites the stored body through `edit` and keeps the old MAC — a hand edit.
fn edit_body(path: &Path, edit: impl FnOnce(&str) -> String) {
    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let body = edit(doc["body"].as_str().unwrap());
    doc["body"] = serde_json::Value::from(body);
    std::fs::write(path, serde_json::to_string(&doc).unwrap()).unwrap();
}

#[test]
fn an_edited_list_turns_the_feature_off_until_accepted_and_accept_re_signs() {
    let dir = TempDir::new();
    let os = FakeOs::alone(1);
    paired(dir.path(), &os);
    let path = list_path(dir.path(), ACCOUNT_X, MACHINE_1);
    edit_body(&path, |body| body.replace("\"home\"", "\"evil\""));

    let mut store = open(dir.path(), ACCOUNT_X, MACHINE_1, &os);
    assert_eq!(store.status(), PeerStatus::ListChangedOutside);
    assert_eq!(
        store.status().to_string(),
        "trusted list was changed outside Engram"
    );
    assert_eq!(
        names(&store),
        ["evil"],
        "the accept question shows the list as it is"
    );
    assert!(matches!(
        store.add(peer(2, "x")),
        Err(PeerError::Blocked(_))
    ));
    assert!(matches!(
        store.remove(&PeerId::from_bytes([1; 32])),
        Err(PeerError::Blocked(_))
    ));

    store.accept_list().unwrap();
    assert_eq!(store.status(), PeerStatus::Ready);
    assert!(matches!(store.accept_list(), Err(PeerError::NotOffered(_))));
    drop(store);
    let store = open(dir.path(), ACCOUNT_X, MACHINE_1, &os);
    assert_eq!(store.status(), PeerStatus::Ready);
    assert_eq!(names(&store), ["evil"]);
}

#[test]
fn a_list_replaced_by_one_signed_under_another_key_is_changed_outside() {
    let dir = TempDir::new();
    let os = FakeOs::alone(1);
    paired(dir.path(), &os);
    // Another account's well-formed list for this account's name.
    let other = TempDir::new();
    let mut theirs = open(other.path(), ACCOUNT_X, MACHINE_1, &FakeOs::alone(2));
    theirs.add(peer(3, "intruder")).unwrap();
    std::fs::copy(
        list_path(other.path(), ACCOUNT_X, MACHINE_1),
        list_path(dir.path(), ACCOUNT_X, MACHINE_1),
    )
    .unwrap();

    let store = open(dir.path(), ACCOUNT_X, MACHINE_1, &os);
    assert_eq!(store.status(), PeerStatus::ListChangedOutside);
}

#[test]
fn a_newer_list_file_is_refused_and_writes_answer_conflict() {
    let dir = TempDir::new();
    let os = FakeOs::alone(1);
    let mut store = open(dir.path(), ACCOUNT_X, MACHINE_1, &os);
    store.ensure_identity().unwrap();
    let path = list_path(dir.path(), ACCOUNT_X, MACHINE_1);
    let newer = r#"{"version":2,"anything":"else"}"#;
    std::fs::write(&path, newer).unwrap();
    drop(store);

    let mut store = open(dir.path(), ACCOUNT_X, MACHINE_1, &os);
    assert_eq!(
        store.status(),
        PeerStatus::ListRefused(ListRefusal::Newer { found: 2 })
    );
    assert!(matches!(
        store.add(peer(1, "home")),
        Err(PeerError::Conflict(_))
    ));
    assert!(matches!(
        store.set_network(NetworkSettings::default()),
        Err(PeerError::Conflict(_))
    ));
    assert!(!store.retry(), "a newer file is not retried");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), newer);
}

#[test]
fn a_newer_list_file_that_appears_during_the_run_is_not_overwritten() {
    let dir = TempDir::new();
    let os = FakeOs::alone(1);
    let mut store = paired(dir.path(), &os);
    let path = list_path(dir.path(), ACCOUNT_X, MACHINE_1);
    let newer = r#"{"version":3}"#;
    std::fs::write(&path, newer).unwrap();
    assert!(matches!(
        store.add(peer(2, "office")),
        Err(PeerError::Conflict(_))
    ));
    assert_eq!(names(&store), ["home"], "a refused add is not kept");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), newer);
}

#[test]
fn a_list_file_that_cannot_be_read_is_refused_until_it_reads() {
    let dir = TempDir::new();
    let os = FakeOs::alone(1);
    let mut store = open(dir.path(), ACCOUNT_X, MACHINE_1, &os);
    store.ensure_identity().unwrap();
    // A folder in the file's place: opening or reading it fails, which is not corruption.
    let path = list_path(dir.path(), ACCOUNT_X, MACHINE_1);
    std::fs::create_dir(&path).unwrap();
    drop(store);

    let mut store = open(dir.path(), ACCOUNT_X, MACHINE_1, &os);
    assert!(matches!(
        store.status(),
        PeerStatus::ListRefused(ListRefusal::ReadFailed(_))
    ));
    assert!(matches!(
        store.add(peer(1, "home")),
        Err(PeerError::Conflict(_))
    ));
    std::fs::remove_dir(&path).unwrap();
    assert!(store.retry());
    assert_eq!(store.status(), PeerStatus::Ready);
}

#[test]
fn a_corrupt_list_file_with_a_key_keeps_the_feature_off_until_accepted() {
    let dir = TempDir::new();
    let os = FakeOs::alone(1);
    let path = list_path(dir.path(), ACCOUNT_X, MACHINE_1);
    let corrupt_bodies = [
        "not json".to_string(),
        r#"{"version":1,"mac":"00","body":"{}"}"#.to_string(),
        format!(
            r#"{{"version":1,"mac":"{}","body":"not a list"}}"#,
            "0".repeat(64)
        ),
        "x".repeat((1 << 20) + 1),
    ];
    for corrupt in corrupt_bodies {
        // A list with a PC and own settings, then damaged.
        let _ = std::fs::remove_file(&path);
        let mut store = paired(dir.path(), &os);
        store
            .set_network(NetworkSettings {
                port: Some(5000),
                ..NetworkSettings::default()
            })
            .unwrap();
        drop(store);
        std::fs::write(&path, &corrupt).unwrap();

        let mut store = open(dir.path(), ACCOUNT_X, MACHINE_1, &os);
        assert!(
            matches!(store.status(), PeerStatus::ListCorrupt(_)),
            "{:?}",
            store.status()
        );
        assert!(store.peers().is_empty());
        assert!(matches!(
            store.add(peer(2, "office")),
            Err(PeerError::Blocked(_))
        ));
        assert!(matches!(
            store.set_network(NetworkSettings::default()),
            Err(PeerError::Blocked(_))
        ));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            corrupt,
            "nothing written before the accept"
        );

        store.accept_list().unwrap();
        assert_eq!(store.status(), PeerStatus::Ready);
        let aside = dir
            .path()
            .join(format!("peers.{ACCOUNT_X}.{MACHINE_1}.json.corrupt"));
        assert_eq!(
            std::fs::read_to_string(&aside).unwrap(),
            corrupt,
            "set aside by the accept"
        );
        drop(store);

        let mut again = open(dir.path(), ACCOUNT_X, MACHINE_1, &os);
        assert_eq!(again.status(), PeerStatus::Ready, "signed");
        assert!(again.peers().is_empty());
        assert_eq!(again.network(), Some(&NetworkSettings::default()));
        again.add(peer(1, "home")).unwrap();
        drop(again);
        std::fs::remove_file(&aside).unwrap();
    }
}

#[test]
fn a_missing_list_file_is_an_empty_list_without_a_question() {
    let dir = TempDir::new();
    let os = FakeOs::alone(1);
    paired(dir.path(), &os);
    std::fs::remove_file(list_path(dir.path(), ACCOUNT_X, MACHINE_1)).unwrap();
    let mut store = open(dir.path(), ACCOUNT_X, MACHINE_1, &os);
    assert_eq!(store.status(), PeerStatus::Ready);
    assert!(store.peers().is_empty());
    assert_eq!(store.network(), Some(&NetworkSettings::default()));
    assert!(matches!(store.accept_list(), Err(PeerError::NotOffered(_))));
    store.add(peer(1, "home")).unwrap();
}

/// No key yet (first run) with a corrupt list: the key is still made on first need without a prompt, and from then on the corrupt
///   list waits for the accept like any other — it may have held pairings, and with the file unreadable "the list names PCs" cannot be
///   ruled out.
#[test]
fn a_corrupt_list_file_without_a_key_asks_once_the_key_is_made() {
    let dir = TempDir::new();
    let os = FakeOs::alone(1);
    let path = list_path(dir.path(), ACCOUNT_X, MACHINE_1);
    std::fs::write(&path, "not json").unwrap();
    let mut store = open(dir.path(), ACCOUNT_X, MACHINE_1, &os);
    assert_eq!(store.status(), PeerStatus::NoIdentityYet);
    assert!(matches!(
        store.add(peer(1, "home")),
        Err(PeerError::Blocked(_))
    ));
    assert!(store.id().is_some(), "the key was made on first need");
    assert!(matches!(store.status(), PeerStatus::ListCorrupt(_)));
    store.accept_list().unwrap();
    store.add(peer(1, "home")).unwrap();
    assert_eq!(names(&store), ["home"]);
}

#[test]
fn entries_follow_the_rules_and_our_own_id_is_refused() {
    let dir = TempDir::new();
    let os = FakeOs::alone(1);
    let mut store = paired(dir.path(), &os);
    let own = store.id().unwrap();
    let mut mine = peer(0, "me");
    mine.id = own;
    assert_eq!(
        store.add(mine),
        Err(PeerError::Invalid("that is this PC's own ID".to_string()))
    );
    assert!(
        matches!(store.add(peer(1, "again")), Err(PeerError::Invalid(_))),
        "listed ID"
    );
    assert!(
        matches!(store.add(peer(2, "HOME")), Err(PeerError::Invalid(_))),
        "listed name"
    );
    assert!(matches!(
        store.add(peer(2, "a@b")),
        Err(PeerError::Invalid(_))
    ));
    assert_eq!(names(&store), ["home"]);
    drop(store);
    let again = open(dir.path(), ACCOUNT_X, MACHINE_1, &os);
    assert_eq!(names(&again), ["home"], "refused adds wrote nothing");
}

#[test]
fn edit_and_remove_change_the_stored_list() {
    let dir = TempDir::new();
    let os = FakeOs::alone(1);
    let mut store = paired(dir.path(), &os);
    store.add(peer(2, "office")).unwrap();
    let home = PeerId::from_bytes([1; 32]);
    let address: SocketAddr = "192.168.0.7:47120".parse().unwrap();
    store
        .edit(
            &home,
            PeerEdit {
                name: Some("house".to_string()),
                address: Some(Some(address)),
            },
        )
        .unwrap();
    assert!(matches!(
        store.edit(
            &home,
            PeerEdit {
                name: Some("OFFICE".to_string()),
                address: None
            }
        ),
        Err(PeerError::Invalid(_))
    ));
    let unknown = PeerId::from_bytes([9; 32]);
    assert_eq!(
        store.edit(&unknown, PeerEdit::default()),
        Err(PeerError::NotFound(unknown))
    );
    assert!(matches!(
        store.remove(&unknown),
        Err(PeerError::NotFound(_))
    ));
    let removal = store.remove(&PeerId::from_bytes([2; 32])).unwrap();
    assert_eq!(removal.removed.name, "office");
    assert_eq!(removal.saved, Ok(()));
    drop(store);

    let again = open(dir.path(), ACCOUNT_X, MACHINE_1, &os);
    assert_eq!(names(&again), ["house"]);
    assert_eq!(again.peers()[0].address, Some(address));
    assert_eq!(again.peers()[0].id, home, "the ID stays on an edit");
}

#[test]
fn a_removal_whose_save_is_refused_still_revokes_for_this_run() {
    let dir = TempDir::new();
    let os = FakeOs::alone(1);
    let mut store = paired(dir.path(), &os);
    store.add(peer(2, "office")).unwrap();
    std::fs::write(
        list_path(dir.path(), ACCOUNT_X, MACHINE_1),
        r#"{"version":9}"#,
    )
    .unwrap();

    let removal = store.remove(&PeerId::from_bytes([1; 32])).unwrap();
    assert!(matches!(removal.saved, Err(PeerError::Conflict(_))));
    assert_eq!(names(&store), ["office"], "revoked in memory");
}

// ── local identity lock ──

#[test]
fn a_second_copy_with_the_same_identity_on_this_machine_and_account_is_refused() {
    let dir = TempDir::new();
    let locks: Arc<Mutex<HashSet<String>>> = Arc::default();
    let first_os = FakeOs::new(1, locks.clone());
    let first = paired(dir.path(), &first_os);
    assert_eq!(first.status(), PeerStatus::Ready);

    // A copy of the folder elsewhere loads the same identity.
    let copy = TempDir::new();
    for name in [
        format!("peer-key.{ACCOUNT_X}.{MACHINE_1}"),
        format!("peers.{ACCOUNT_X}.{MACHINE_1}.json"),
    ] {
        std::fs::copy(dir.path().join(&name), copy.path().join(&name)).unwrap();
    }
    let mut second = open(
        copy.path(),
        ACCOUNT_X,
        MACHINE_1,
        &FakeOs::new(1, locks.clone()),
    );
    assert_eq!(second.id(), first.id());
    assert_eq!(second.status(), PeerStatus::IdentityInUse);
    assert_eq!(
        second.status().to_string(),
        "this PC identity is already in use by another Engram copy on this PC"
    );
    assert!(matches!(
        second.add(peer(2, "x")),
        Err(PeerError::Blocked(_))
    ));

    // A retry while the first copy runs finds it still held; once the first copy is gone, the retry takes the lock in the same run.
    assert!(second.retry());
    assert_eq!(second.status(), PeerStatus::IdentityInUse);
    drop(first);
    assert!(second.retry());
    assert_eq!(second.status(), PeerStatus::Ready);
    assert_eq!(locks.lock().unwrap().len(), 1);
    drop(second);
    let third = open(copy.path(), ACCOUNT_X, MACHINE_1, &FakeOs::new(1, locks));
    assert_eq!(
        third.status(),
        PeerStatus::Ready,
        "released with its holder"
    );
}

#[test]
fn a_regenerate_moves_the_lock_to_the_new_identity() {
    let dir = TempDir::new();
    let locks: Arc<Mutex<HashSet<String>>> = Arc::default();
    let os = FakeOs::new(1, locks.clone());
    let mut store = paired(dir.path(), &os);
    let old = store.id().unwrap();
    let new = store.regenerate().unwrap().id;
    let held = locks.lock().unwrap().clone();
    assert_eq!(held.len(), 1);
    assert!(held.contains(&new.to_text()));
    assert!(!held.contains(&old.to_text()));
}

/// A lock that fails for an OS reason (its folder not made, no such lock on this OS) is gone without, and said so.
#[test]
fn a_lock_that_fails_for_an_os_reason_does_not_stop_the_feature_and_is_noted() {
    struct NotOurs(Arc<FakeOs>);
    impl PeerOs for NotOurs {
        fn seal(&self, secret: &[u8]) -> io::Result<Vec<u8>> {
            self.0.seal(secret)
        }
        fn unseal(&self, blob: &[u8]) -> Result<Vec<u8>, UnsealError> {
            self.0.unseal(blob)
        }
        fn lock(&self, _name: &str) -> LockAttempt {
            LockAttempt::NotHeld("the lock folder could not be made".to_string())
        }
        fn now(&self) -> SystemTime {
            self.0.now()
        }
    }
    let dir = TempDir::new();
    let os = FakeOs::alone(1);
    paired(dir.path(), &os);
    let store = PeerStore::load(
        dir.path().to_path_buf(),
        Ok(ACCOUNT_X.to_string()),
        Ok(MACHINE_1.to_string()),
        Arc::new(NotOurs(os)),
    );
    assert_eq!(store.status(), PeerStatus::Ready);
    assert_eq!(store.lock_note(), Some("the lock folder could not be made"));
}

#[test]
fn unreadable_account_or_machine_ids_leave_peer_links_unavailable() {
    let dir = TempDir::new();
    let os: Arc<dyn PeerOs> = FakeOs::alone(1);
    for (account, machine) in [
        (Err(io::Error::other("no token")), Ok(MACHINE_1.to_string())),
        (
            Ok(ACCOUNT_X.to_string()),
            Err(io::Error::other("no registry value")),
        ),
        (Ok("a.b".to_string()), Ok(MACHINE_1.to_string())),
    ] {
        let mut store = PeerStore::load(dir.path().to_path_buf(), account, machine, os.clone());
        assert!(matches!(store.status(), PeerStatus::Unavailable(_)));
        assert!(matches!(
            store.ensure_identity(),
            Err(PeerError::Blocked(_))
        ));
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

// ── adopt with a list already here ──

/// A key of this account under MACHINE_1 (with a one-PC list), opened as MACHINE_2.
fn moved_folder(dir: &Path, os: &Arc<FakeOs>) -> PathBuf {
    paired(dir, os);
    list_path(dir, ACCOUNT_X, MACHINE_2)
}

#[test]
fn a_local_list_that_must_not_be_written_over_wins_over_the_adopt_or_fresh_question() {
    let dir = TempDir::new();
    let os = FakeOs::alone(1);
    let ours = moved_folder(dir.path(), &os);
    for (what, make) in [
        (
            "newer",
            Box::new(|p: &Path| std::fs::write(p, r#"{"version":5}"#).unwrap())
                as Box<dyn Fn(&Path)>,
        ),
        (
            "unreadable",
            Box::new(|p: &Path| std::fs::create_dir(p).unwrap()),
        ),
    ] {
        make(&ours);
        let mut moved = open(dir.path(), ACCOUNT_X, MACHINE_2, &os);
        assert!(
            matches!(moved.status(), PeerStatus::ListRefused(_)),
            "{what}: {:?}",
            moved.status()
        );
        assert!(
            matches!(moved.adopt(MACHINE_1), Err(PeerError::NotOffered(_))),
            "{what}"
        );
        assert!(
            matches!(moved.start_fresh(), Err(PeerError::NotOffered(_))),
            "{what}"
        );
        assert!(
            matches!(moved.ensure_identity(), Err(PeerError::Blocked(_))),
            "{what}"
        );
        assert!(
            matches!(moved.regenerate(), Err(PeerError::Conflict(_))),
            "{what}"
        );
        assert!(
            !key_path(dir.path(), ACCOUNT_X, MACHINE_2).exists(),
            "{what}: no key written"
        );
        if ours.is_dir() {
            std::fs::remove_dir(&ours).unwrap();
            assert!(moved.retry(), "a failed read is retried");
            assert!(matches!(moved.status(), PeerStatus::AdoptOrFresh { .. }));
        } else {
            assert_eq!(std::fs::read_to_string(&ours).unwrap(), r#"{"version":5}"#);
            std::fs::remove_file(&ours).unwrap();
        }
    }
}

#[test]
fn adopt_refuses_a_local_list_that_turned_newer_after_the_question_and_writes_nothing() {
    let dir = TempDir::new();
    let os = FakeOs::alone(1);
    let ours = moved_folder(dir.path(), &os);
    let mut moved = open(dir.path(), ACCOUNT_X, MACHINE_2, &os);
    assert!(matches!(moved.status(), PeerStatus::AdoptOrFresh { .. }));
    std::fs::write(&ours, r#"{"version":5}"#).unwrap();
    assert!(matches!(
        moved.adopt(MACHINE_1),
        Err(PeerError::Conflict(_))
    ));
    assert!(!key_path(dir.path(), ACCOUNT_X, MACHINE_2).exists());
    assert_eq!(std::fs::read_to_string(&ours).unwrap(), r#"{"version":5}"#);
}

#[test]
fn adopt_sets_a_corrupt_or_settings_only_local_list_aside_before_taking_the_other_list() {
    let settings_only = format!(
        r#"{{"version":1,"mac":"{}","body":"{{\"network\":{{\"port\":5000}}}}"}}"#,
        "0".repeat(64)
    );
    for (what, local) in [
        ("corrupt", "not json".to_string()),
        ("settings only", settings_only),
    ] {
        let dir = TempDir::new();
        let os = FakeOs::alone(1);
        let ours = moved_folder(dir.path(), &os);
        std::fs::write(&ours, &local).unwrap();
        let mut moved = open(dir.path(), ACCOUNT_X, MACHINE_2, &os);
        assert!(
            matches!(moved.status(), PeerStatus::AdoptOrFresh { .. }),
            "{what}"
        );
        moved.adopt(MACHINE_1).unwrap();
        assert_eq!(moved.status(), PeerStatus::Ready, "{what}");
        assert_eq!(names(&moved), ["home"], "{what}");
        let aside = dir
            .path()
            .join(format!("peers.{ACCOUNT_X}.{MACHINE_2}.json.corrupt"));
        assert_eq!(
            std::fs::read_to_string(&aside).unwrap(),
            local,
            "{what}: kept aside"
        );
    }
}

#[test]
fn adopt_keeps_a_local_list_when_the_other_machine_has_none() {
    let dir = TempDir::new();
    let os = FakeOs::alone(1);
    let ours = moved_folder(dir.path(), &os);
    std::fs::remove_file(list_path(dir.path(), ACCOUNT_X, MACHINE_1)).unwrap();
    std::fs::write(&ours, "not json").unwrap();
    let mut moved = open(dir.path(), ACCOUNT_X, MACHINE_2, &os);
    moved.adopt(MACHINE_1).unwrap();
    assert_eq!(std::fs::read_to_string(&ours).unwrap(), "not json");
    assert!(
        matches!(moved.status(), PeerStatus::ListCorrupt(_)),
        "the kept corrupt list waits for the accept"
    );
    assert!(!dir
        .path()
        .join(format!("peers.{ACCOUNT_X}.{MACHINE_2}.json.corrupt"))
        .exists());
}

// ── the link adapter's URL check runs on writes only ──

/// A URL whose form `Helper::parse` accepts but the link adapter's own parse refuses (invalid punycode).
const ADAPTER_REFUSES: &str = "https://xn--a.example";

#[test]
fn a_stored_setting_the_adapter_refuses_still_loads_but_setting_it_is_refused() {
    let dir = TempDir::new();
    let os = FakeOs::alone(1);
    let store = paired(dir.path(), &os);
    // Re-sign a body carrying that URL, as a list saved before a stricter adapter would be.
    let path = list_path(dir.path(), ACCOUNT_X, MACHINE_1);
    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let mut body: serde_json::Value = serde_json::from_str(doc["body"].as_str().unwrap()).unwrap();
    body["network"]["relay"] = serde_json::Value::from(ADAPTER_REFUSES);
    let body = serde_json::to_string(&body).unwrap();
    let mac = mac::sign(store.secret().unwrap(), &body).unwrap();
    doc["body"] = serde_json::Value::from(body);
    doc["mac"] = serde_json::Value::from(engram_dashboard_base::text::hex_lower(&mac));
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();
    drop(store);

    let mut store = open(dir.path(), ACCOUNT_X, MACHINE_1, &os);
    assert_eq!(store.status(), PeerStatus::Ready, "not read as corrupt");
    assert_eq!(names(&store), ["home"], "the peers stay");
    assert_eq!(
        store.network().unwrap().relay,
        Helper::Url(ADAPTER_REFUSES.to_string())
    );
    // Other list changes are not blocked by it.
    store.add(peer(2, "office")).unwrap();
    // Setting it is refused, and the stored settings are unchanged.
    let network = NetworkSettings {
        address_lookup: Helper::Url(ADAPTER_REFUSES.to_string()),
        ..store.network().unwrap().clone()
    };
    assert!(matches!(
        store.set_network(network),
        Err(PeerError::Invalid(_))
    ));
    assert_eq!(store.network().unwrap().address_lookup, Helper::Default);
}
