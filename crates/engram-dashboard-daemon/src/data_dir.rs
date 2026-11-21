//! The data folder — finding the root · the write check that creates the folder · install location · the daemon-side layout ([`DataLayout`]).
//!
//! ★The root rule and the places of `daemon.json` · `logs\` are defined here★ — the shell does not depend on the daemon crate in production, so it keeps its own copy of those three, and tests tie that the two copies give the same paths (ADR-0271 decision 4 · ADR-0282 decision 3). Change the rule and the shell copy together, or the shell looks for `daemon.json` in a folder the daemon does not use.
//!
// hangul-ok-begin
//! ★The OS split is only inside [`default_data_dir`]★ — the layout is only `Path::join` (CLAUDE.md platform-neutrality rule).
// hangul-ok-end
//!
//! ★The names of the layout are a disk contract★ — renaming leaves existing folders' data unread, and no code moves it (ADR-0264 — old data is discarded).
// ADR-0271
// ADR-0264

use std::path::{Path, PathBuf};

use crate::portfile::DAEMON_FILE;
use engram_dashboard_base::writable::{probe_write_in, retry_if_vanished};

// ★ENGRAM_DATA_DIR override (test isolation escape hatch — not a deployment knob)★:
//   - The only use = data isolation for integration tests: the real-process test (`tests/ws_e2e.rs`) sends the daemon to a temp directory instead of polluting the production `<repo>/.engram-dev` with daemon.json/agents.json.
//   - Not a deployment customization knob — the deployed data location is fixed under the executable folder (ADR-0134). Do not use it as "a way for the user to change the data folder".
//   - ★Since ADR-0134★ the data folder is the single-instance scope, so splitting this env splits the instance too.
//   - It takes effect only on a daemon launched inheriting the parent environment — a WMI-launched daemon does not inherit it (the launching shell documents that limit).
const DATA_DIR_ENV: &str = "ENGRAM_DATA_DIR";

// ADR-0029: used only in the debug branch (walk-up `.engram-dev`) and its unit tests — dead_code in a release non-test build.
// ADR-0264
#[cfg_attr(not(debug_assertions), allow(dead_code))]
const LOCAL_DATA_DIR: &str = ".engram-dev";

/// ADR-0134 decision 2: release data gathers in one folder under the executable folder — scattered next to the exe it would mix with distribution files and be wiped when a new version's archive is extracted over it.
///
/// ★Do not add the `engram-` prefix (do not revive it)★: this folder is inside the distribution folder the user already named, so a prefix only repeats the parent name, and the sibling `prompts/` has none. That it differs from [`LOCAL_DATA_DIR`] is not a mistake — the debug one sits bare in a repo root with many folders and the leading dot hides it, so its prefix earns its keep. Do not make them symmetric (user decision).
const RELEASE_DATA_DIR: &str = "data";

const DAEMON_DIR: &str = "daemon";
/// What the program writes and reads again on the next run (roster · presets).
const STATE_DIR: &str = "state";
/// What may be thrown away (lock + connection file · spawn attachment files holding tokens · temp folders).
const RUN_DIR: &str = "run";
const LOGS_DIR: &str = "logs";
const MCP_CONFIG_DIR: &str = "mcp-config";
const USAGE_PROBE_DIR: &str = "usage-probe";

/// The data directory of the engram processes (ADR-0024/0029).
///
/// Priority:
/// 1. `ENGRAM_DATA_DIR` (set + non-empty) → that path as is (test isolation escape hatch — not a deployment knob).
/// 2. Debug (`cfg!(debug_assertions)`): walk up from current_exe to the repo root (`.git` or `[workspace]` in `Cargo.toml`) → `<root>/.engram-dev`; no root → the exe directory, then cwd. Several builds (app · daemon) in one development place share one folder.
/// 3. Release (`not(debug_assertions)`): `data/` under the executable folder ([`release_data_dir`]). Deleting the distribution folder leaves no trace — fully portable (ADR-0134 decision 1).
///
/// Never panics: it always returns a PathBuf.
pub fn default_data_dir() -> PathBuf {
    if let Some(path) = data_dir_env_override() {
        return path;
    }

    #[cfg(debug_assertions)]
    {
        // ★Why an exe-based walk-up★: the daemon is launched by WMI Win32_Process.Create and does not inherit the parent's cwd (a WmiPrvSE child), so cwd cannot be trusted. Dev build outputs are under the same repo's target/, so walking up from any exe converges on the same root.
        if let Ok(exe) = std::env::current_exe() {
            if let Some(root) = find_workspace_root(&exe) {
                return root.join(LOCAL_DATA_DIR);
            }
            if let Some(dir) = exe.parent() {
                return dir.join(LOCAL_DATA_DIR);
            }
        }
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(LOCAL_DATA_DIR)
    }

    #[cfg(not(debug_assertions))]
    {
        // The exe path is the only trustworthy reference under a WMI spawn — same reason as the debug walk-up.
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                return release_data_dir(dir);
            }
        }
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(RELEASE_DATA_DIR)
    }
}

/// The `ENGRAM_DATA_DIR` override if active; an empty value is the same as unset.
fn data_dir_env_override() -> Option<PathBuf> {
    let val = std::env::var_os(DATA_DIR_ENV)?;
    if val.is_empty() {
        return None;
    }
    Some(PathBuf::from(val))
}

/// Release data folder = `<exe folder>/data` (ADR-0134 decisions 1·2).
///
/// ★No cfg on it (load-bearing)★: the caller's release branch is under `not(debug_assertions)` and tests always run in debug — hiding this function behind cfg would remove the means to assert the release rule.
pub fn release_data_dir(exe_dir: &Path) -> PathBuf {
    exe_dir.join(RELEASE_DATA_DIR)
}

/// The data folder cannot be written — any cause folds into this one; the one thing the user can do is "extract it somewhere writable".
///
/// ★The message holds the remedy (ADR-0134 decision 4)★: the daemon has no screen to show the user, so this wording in the startup failure log is all that remains. The shell's pre-check raises the same failure with the same wording (the shell-side error type).
#[derive(Debug, thiserror::Error)]
#[error(
    "cannot write to the data folder ({path}): {reason} — please extract to a writable location"
)]
pub struct DataDirUnwritable {
    path: String,
    reason: String,
}

/// Creates the data folder and checks it can really be written (ADR-0134 decision 4 — no fallback).
///
/// ★Only for the side that owns the folder (the daemon)★: it creates the folder. The shell's pre-check, which has not settled on using the folder, uses a self-check that does not create and rolls back (ADR-0271 decision 3).
///
/// An existence check is not enough: an unprivileged location (under `C:\Program Files` etc.) blocks the first write even if the folder exists.
pub fn ensure_data_dir_writable(dir: &Path) -> Result<(), DataDirUnwritable> {
    retry_if_vanished(|| {
        std::fs::create_dir_all(dir)?;
        probe_write_in(dir)
    })
    .map_err(|e| DataDirUnwritable {
        path: dir.display().to_string(),
        reason: e.to_string(),
    })
}

/// The absolute path of the install/repo root resolved from the exe (any build mode) — the trusted base the daemon attaches relative resources to (e.g. ADR-0092 priming `prompts/agent-priming.md`).
///
/// Same shape as default_data_dir's debug/release branches:
///   - Walk up from the exe to the workspace root (`.git` or `Cargo.toml [workspace]`) — development: `<repo>`.
///   - No root: the exe directory (release: where the bundled exes and resources are co-located).
///   - No exe: None (the caller handles it — priming then yields None).
/// Always absolute: current_exe gives an absolute path and walk-up/parent preserve it.
pub fn find_install_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    if let Some(root) = find_workspace_root(&exe) {
        return Some(root);
    }
    exe.parent().map(|p| p.to_path_buf())
}

fn find_workspace_root(start: &Path) -> Option<PathBuf> {
    let mut cur: Option<&Path> = if start.is_dir() {
        Some(start)
    } else {
        start.parent()
    };
    while let Some(dir) = cur {
        if is_workspace_root(dir) {
            return Some(dir.to_path_buf());
        }
        cur = dir.parent();
    }
    None
}

fn is_workspace_root(dir: &Path) -> bool {
    if dir.join(".git").exists() {
        return true;
    }
    let cargo = dir.join("Cargo.toml");
    match std::fs::read_to_string(&cargo) {
        // Extreme cases like `[workspace]` embedded in a comment are ignored — the repo root is caught by .git too.
        Ok(s) => s.contains("[workspace]"),
        Err(_) => false,
    }
}

/// The daemon-side layout of one data folder (`root`). It only computes paths and never looks at the disk, except [`ensure_daemon_dirs`](Self::ensure_daemon_dirs).
///
/// ★Shell folders (`shell\…` · `webview\`) are not here★ — the shell holds them in its own layout (ADR-0271 decision 4).
///
/// ★File names used by only one store are not kept here★: `agents.json` · `presets.json` are owned by each store and attached inside the directory it was given — keeping them here would make that store (agent crate) know the daemon.
// ADR-0264
// ADR-0271
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataLayout {
    root: PathBuf,
}

impl DataLayout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn daemon_state_dir(&self) -> PathBuf {
        self.root.join(DAEMON_DIR).join(STATE_DIR)
    }

    pub fn daemon_run_dir(&self) -> PathBuf {
        self.root.join(DAEMON_DIR).join(RUN_DIR)
    }

    /// The daemon's lock and connection file (ADR-0135) — the record clients read is published only here. The shell and `scripts/engram.mjs` know the same place separately.
    pub fn daemon_file(&self) -> PathBuf {
        self.daemon_run_dir().join(DAEMON_FILE)
    }

    /// This account's shell-token file (`account` = platform `account::current_account_id`), next to [`Self::daemon_file`]; the name is
    ///   protocol's [`engram_dashboard_protocol::token_file_name`], which the shell builds too.
    // ADR-0395
    pub fn token_file(&self, account: &str) -> PathBuf {
        self.daemon_run_dir()
            .join(engram_dashboard_protocol::token_file_name(account))
    }

    pub fn mcp_config_dir(&self) -> PathBuf {
        self.daemon_run_dir().join(MCP_CONFIG_DIR)
    }

    pub fn usage_probe_dir(&self) -> PathBuf {
        self.daemon_run_dir().join(USAGE_PROBE_DIR)
    }

    /// The log folder the daemon and the shell share (separate files per process — ADR-0264 decision 2). The shell has a copy too.
    pub fn logs_dir(&self) -> PathBuf {
        self.root.join(LOGS_DIR)
    }

    /// Creates the daemon's `state` · `run` folders (no-op if present).
    pub fn ensure_daemon_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.daemon_state_dir())?;
        std::fs::create_dir_all(self.daemon_run_dir())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── data_dir resolver ─────────────────────────────────────────────────────────────

    /// `ENGRAM_DATA_DIR` is a process-global env, so parallel tests touching it trample each other. ★Every test in this binary that changes that env holds this one lock★ — two locks could not block each other.
    // ADR-0282
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn data_dir_env_override_returns_path_verbatim() {
        let _g = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let prev = std::env::var_os(DATA_DIR_ENV);
        let want = std::env::temp_dir().join("engram-override-data-dir-test");
        std::env::set_var(DATA_DIR_ENV, &want);
        let got = default_data_dir();
        // Restore before asserting so the env does not leak on an assertion failure.
        match &prev {
            Some(v) => std::env::set_var(DATA_DIR_ENV, v),
            None => std::env::remove_var(DATA_DIR_ENV),
        }
        assert_eq!(
            got, want,
            "with ENGRAM_DATA_DIR set, returns that path as is"
        );
    }

    #[test]
    fn data_dir_empty_env_falls_through_to_default() {
        // Tests are always debug builds, so they take the walk-up `.engram-dev` branch.
        let _g = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let prev = std::env::var_os(DATA_DIR_ENV);
        std::env::set_var(DATA_DIR_ENV, "");
        let got = default_data_dir();
        match &prev {
            Some(v) => std::env::set_var(DATA_DIR_ENV, v),
            None => std::env::remove_var(DATA_DIR_ENV),
        }
        assert!(
            got.ends_with(LOCAL_DATA_DIR),
            "an empty env passes through to the default branch → must end in `.engram-dev`: {got:?}"
        );
    }

    #[test]
    fn default_data_dir_debug_is_local_data_dir() {
        let _g = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let prev = std::env::var_os(DATA_DIR_ENV);
        std::env::remove_var(DATA_DIR_ENV);
        let got = default_data_dir();
        if let Some(v) = &prev {
            std::env::set_var(DATA_DIR_ENV, v);
        }
        assert!(
            got.ends_with(LOCAL_DATA_DIR),
            "debug must end in the folder-local `.engram-dev`: {got:?}"
        );
    }

    /// Tests (always debug) cannot take the `not(debug_assertions)` release branch, so the rule lives in a pure helper without cfg and is asserted here (ADR-0134).
    #[test]
    fn release_data_dir_is_exe_adjacent() {
        let exe_dir = Path::new("C:\\portable\\engram");
        assert_eq!(
            release_data_dir(exe_dir),
            exe_dir.join("data"),
            "the release data folder is data under the exe folder (no `engram-` prefix — see the constant's comment)"
        );
    }

    #[test]
    fn resolve_data_dir_honors_env_override() {
        let _g = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let prev = std::env::var_os(DATA_DIR_ENV);
        let want = std::env::temp_dir().join("engram-daemon-resolve-override-test");
        std::env::set_var(DATA_DIR_ENV, &want);
        let got = crate::resolve_data_dir();
        match &prev {
            Some(v) => std::env::set_var(DATA_DIR_ENV, v),
            None => std::env::remove_var(DATA_DIR_ENV),
        }
        assert_eq!(
            got, want,
            "with ENGRAM_DATA_DIR set, it must be isolated to that path"
        );
    }

    // ── data folder writability probe (ADR-0134 decision 4) ─────────────────────────────────

    /// A unique folder for the probe only (parallel tests do not collide).
    fn fresh_probe_dir(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "engram-writable-probe-{tag}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// Any probe leftover in the folder (the name differs per call, so count by prefix).
    fn probe_leftovers(dir: &Path) -> usize {
        use engram_dashboard_base::writable::WRITE_PROBE_PREFIX;
        std::fs::read_dir(dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| {
                        e.file_name()
                            .to_string_lossy()
                            .starts_with(WRITE_PROBE_PREFIX)
                    })
                    .count()
            })
            .unwrap_or(0)
    }

    #[test]
    fn writable_probe_accepts_temp_dir_and_leaves_nothing() {
        let dir = fresh_probe_dir("ok");
        ensure_data_dir_writable(&dir).expect("under temp must be writable");
        let left = probe_leftovers(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(left, 0, "must not leave probe files behind");
    }

    #[test]
    fn writable_probe_rejects_child_of_a_file() {
        // A child path of a file cannot be created — the representative case where folder creation itself is blocked.
        let file = fresh_probe_dir("blocker");
        std::fs::create_dir_all(file.parent().unwrap()).ok();
        std::fs::write(&file, b"x").expect("create blocker file");
        let err = ensure_data_dir_writable(&file.join("child")).unwrap_err();
        let _ = std::fs::remove_file(&file);
        assert!(
            err.to_string().contains("writable location"),
            "the message must hold what the user should do: {err}"
        );
    }

    // ── find_workspace_root / is_workspace_root (temp directory tree, regardless of build mode) ──────

    fn unique_tmp(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "engram-ws-root-{tag}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn find_workspace_root_detects_git_marker_walking_up() {
        let root = unique_tmp("git");
        let deep = root.join("a").join("b").join("c");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();

        let got = find_workspace_root(&deep).expect("must find the .git marker walking up");
        // The temp directory may be a symlink (e.g. macOS /var→/private), so compare after canonicalize.
        assert_eq!(
            std::fs::canonicalize(&got).unwrap(),
            std::fs::canonicalize(&root).unwrap()
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn find_workspace_root_detects_cargo_workspace_marker() {
        let root = unique_tmp("cargo");
        let sub = root.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(root.join("Cargo.toml"), b"[workspace]\nmembers = [\"x\"]\n").unwrap();

        let got = find_workspace_root(&sub).expect("must find the [workspace] Cargo.toml");
        assert_eq!(
            std::fs::canonicalize(&got).unwrap(),
            std::fs::canonicalize(&root).unwrap()
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn find_workspace_root_none_when_no_marker() {
        let root = unique_tmp("none");
        let deep = root.join("x").join("y");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(root.join("Cargo.toml"), b"[package]\nname = \"z\"\n").unwrap();

        assert!(
            find_workspace_root(&deep).is_none(),
            "no marker → None — [package] alone is not a workspace root"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn find_install_root_yields_absolute_path() {
        // The concrete path depends on the environment — assert only absoluteness.
        let root = find_install_root().expect("while a test runs, current_exe exists → Some");
        assert!(
            root.is_absolute(),
            "find_install_root must be an absolute path (the cwd-distrust contract): {root:?}"
        );
    }

    #[test]
    fn is_workspace_root_distinguishes_markers() {
        let base = unique_tmp("is");
        let git_dir = base.join("g");
        std::fs::create_dir_all(git_dir.join(".git")).unwrap();
        assert!(is_workspace_root(&git_dir), ".git exists → true");

        let ws_dir = base.join("w");
        std::fs::create_dir_all(&ws_dir).unwrap();
        std::fs::write(ws_dir.join("Cargo.toml"), b"[workspace]\n").unwrap();
        assert!(is_workspace_root(&ws_dir), "[workspace] → true");

        let pkg_dir = base.join("p");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("Cargo.toml"), b"[package]\nname=\"q\"\n").unwrap();
        assert!(!is_workspace_root(&pkg_dir), "[package] alone → false");

        let empty_dir = base.join("e");
        std::fs::create_dir_all(&empty_dir).unwrap();
        assert!(!is_workspace_root(&empty_dir), "no marker → false");

        let _ = std::fs::remove_dir_all(&base);
    }

    // ── DataLayout (daemon-side layout) ──────────────────────────────────────────────────────

    #[test]
    fn every_path_hangs_off_the_root_in_its_component_and_kind() {
        let root = PathBuf::from("R");
        let l = DataLayout::new(&root);
        assert_eq!(l.root(), root.as_path());
        assert_eq!(l.daemon_state_dir(), root.join("daemon").join("state"));
        assert_eq!(l.daemon_run_dir(), root.join("daemon").join("run"));
        assert_eq!(
            l.daemon_file(),
            root.join("daemon").join("run").join("daemon.json")
        );
        assert_eq!(
            l.token_file("S-1-5-21-9"),
            root.join("daemon")
                .join("run")
                .join("daemon-token.S-1-5-21-9")
        );
        assert_eq!(
            l.mcp_config_dir(),
            root.join("daemon").join("run").join("mcp-config")
        );
        assert_eq!(
            l.usage_probe_dir(),
            root.join("daemon").join("run").join("usage-probe")
        );
        assert_eq!(l.logs_dir(), root.join("logs"));
    }

    #[test]
    fn ensure_daemon_dirs_creates_state_and_run_and_is_idempotent() {
        let root = std::env::temp_dir().join(format!(
            "engram-layout-ensure-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let l = DataLayout::new(&root);
        l.ensure_daemon_dirs().expect("first creation");
        l.ensure_daemon_dirs()
            .expect("success even if it already exists");
        let made = (l.daemon_state_dir().is_dir(), l.daemon_run_dir().is_dir());
        let top: Vec<_> = std::fs::read_dir(&root)
            .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.file_name()).collect())
            .unwrap_or_default();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(made, (true, true));
        assert_eq!(
            top,
            vec![std::ffi::OsString::from("daemon")],
            "creates only the daemon folders"
        );
    }
}
