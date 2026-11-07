//! Data folder — root finding · the shell-side layout ([`DataLayout`]).
//!
//! ★The root rule · the `daemon.json` place · `logs\` are copies of the daemon's `data_dir`★ — the shell has no production dependency on the daemon crate,
//! so it keeps its own, and whether the two copies yield the same path is bound by a same-path test (ADR-0271 decision 4). The canonical explanation of the rule is
//! the daemon's — if it is changed, both copies are changed in the same change.
//!
//! ★The names of the shell folders (`shell\…` · `webview\`) are canonical here and are a disk contract★ — if changed, the data of the existing folders
//! is left unread. There is no code that moves it over (ADR-0264 — old data is discarded).
// ADR-0271
// ADR-0282
// ADR-0264

use std::path::{Path, PathBuf};

const DATA_DIR_ENV: &str = "ENGRAM_DATA_DIR";

// ADR-0029: used only in the debug branch (walk-up `.engram-dev`) and its unit tests — the release
// default_data_dir uses only the `data` next to the exe, so in a release non-test build it is dead_code.
// ADR-0264
#[cfg_attr(not(debug_assertions), allow(dead_code))]
const LOCAL_DATA_DIR: &str = ".engram-dev";

const RELEASE_DATA_DIR: &str = "data";

/// The connection · lock file name that the daemon writes and the shell · scripts read (ADR-0135 — lock file = connection file).
pub(crate) const DAEMON_FILE: &str = "daemon.json";

const DAEMON_DIR: &str = "daemon";
const SHELL_DIR: &str = "shell";
/// Settings the user edits.
const CONFIG_DIR: &str = "config";
/// What the program writes and reads again on the next run (screen state).
const STATE_DIR: &str = "state";
/// What may be discarded (lock+connection file · the spawn attachment file carrying the token · temp folder).
const RUN_DIR: &str = "run";
const WEBVIEW_DIR: &str = "webview";
const LOGS_DIR: &str = "logs";

/// The data directory of the engram process — a copy of the daemon's `data_dir::default_data_dir` (priority · branch explanation is there).
pub fn default_data_dir() -> PathBuf {
    if let Some(path) = data_dir_env_override() {
        return path;
    }

    #[cfg(debug_assertions)]
    {
        // ★Why exe-based walk-up★: the daemon is launched via WMI Win32_Process.Create and **does not inherit the parent's
        // cwd** (a WmiPrvSE child) — cwd cannot be trusted. The exe path, on the other hand, is reliable, and development build
        // artifacts are under the same repo's target/, so walking up from any exe converges on the same repo root.
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
        // The exe path is the only reference point that remains reliable even under a WMI spawn (parent cwd not inherited) — the same reason
        // the debug branch's walk-up starts from the exe.
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

/// If the `ENGRAM_DATA_DIR` override is **active**, that path. An empty value is the same as unset.
///
/// ★Single source★: there is more than one place that asks "is the override on", so they are gathered here — if the decisions diverge,
/// one side would look at a folder the daemon does not use (the precheck of `ensure_daemon` is that place).
pub(crate) fn data_dir_env_override() -> Option<PathBuf> {
    let val = std::env::var_os(DATA_DIR_ENV)?;
    if val.is_empty() {
        return None;
    }
    Some(PathBuf::from(val))
}

/// Release data folder = `<exe 폴더>/data` (ADR-0134 decisions 1·2).
///
/// ★Do not put a cfg on it (load-bearing)★: the release branch of the caller [`default_data_dir`] is under
/// `not(debug_assertions)` and **tests always run as debug** — if this function were also hidden behind a cfg,
/// there would be no means to assert the release rule.
pub fn release_data_dir(exe_dir: &Path) -> PathBuf {
    exe_dir.join(RELEASE_DATA_DIR)
}

// Both call only the debug branch — the same reason as `LOCAL_DATA_DIR` above (the daemon's version also calls the install location, so it lacks this attribute).
#[cfg_attr(not(debug_assertions), allow(dead_code))]
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

#[cfg_attr(not(debug_assertions), allow(dead_code))]
fn is_workspace_root(dir: &Path) -> bool {
    if dir.join(".git").exists() {
        return true;
    }
    let cargo = dir.join("Cargo.toml");
    match std::fs::read_to_string(&cargo) {
        // Extreme cases such as a `[workspace]` string embedded in a comment are ignored — the repo root is also caught by .git.
        Ok(s) => s.contains("[workspace]"),
        Err(_) => false,
    }
}

/// The shell-side layout of one data folder (`root`). It only computes paths and does not look at the disk.
///
/// ★The daemon folder knows only the `daemon.json` place (a copy)★ — the rest is held by the daemon's `data_dir::DataLayout` (ADR-0271 decision 4),
/// and the shell does not create the daemon folder.
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

    /// The layout over [`default_data_dir`] — the shell's production entry point.
    pub fn resolve() -> Self {
        Self::new(default_data_dir())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The daemon's lock and also its connection file — the record clients read is published only here.
    pub fn daemon_file(&self) -> PathBuf {
        self.root.join(DAEMON_DIR).join(RUN_DIR).join(DAEMON_FILE)
    }

    pub fn shell_config_dir(&self) -> PathBuf {
        self.root.join(SHELL_DIR).join(CONFIG_DIR)
    }

    pub fn shell_state_dir(&self) -> PathBuf {
        self.root.join(SHELL_DIR).join(STATE_DIR)
    }

    pub fn shell_run_dir(&self) -> PathBuf {
        self.root.join(SHELL_DIR).join(RUN_DIR)
    }

    pub fn webview_dir(&self) -> PathBuf {
        self.root.join(WEBVIEW_DIR)
    }

    /// The log folder shared by the daemon · shell (files are separate per process — ADR-0264 decision 2).
    pub fn logs_dir(&self) -> PathBuf {
        self.root.join(LOGS_DIR)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    use engram_dashboard_daemon::data_dir as daemon;

    #[test]
    fn every_shell_path_hangs_off_the_root_in_its_component_and_kind() {
        let root = PathBuf::from("R");
        let l = DataLayout::new(&root);
        assert_eq!(l.root(), root.as_path());
        assert_eq!(
            l.daemon_file(),
            root.join("daemon").join("run").join("daemon.json")
        );
        assert_eq!(l.shell_config_dir(), root.join("shell").join("config"));
        assert_eq!(l.shell_state_dir(), root.join("shell").join("state"));
        assert_eq!(l.shell_run_dir(), root.join("shell").join("run"));
        assert_eq!(l.webview_dir(), root.join("webview"));
        assert_eq!(l.logs_dir(), root.join("logs"));
    }

    // ── same path as the daemon source of truth (ADR-0271 decision 4) ─────────────────────────────────────────
    // Relies on the shell → (dev) daemon edge (that edge is kept — ADR-0286) — on the day it is removed, this test's place is decided again. It always runs as debug, so
    // the release-branch body of the root rule cannot be compared (only the helper `release_data_dir` is measured), and it also cannot catch a copy that omitted one walk-up marker —
    // this repository's root has both markers, so the same root comes out (the marker test exists only in the daemon).
    // ADR-0271
    // ADR-0282

    /// The env name is written as a literal, not a constant — if the spelling of either side's constant is wrong, the two copies diverge and are caught here.
    /// `_guard` = the guard of `DATA_DIR_ENV_LOCK` — it changes the process env, so do not call it outside that lock.
    fn both_data_dirs_with_env(
        _guard: &std::sync::MutexGuard<'_, ()>,
        val: Option<&OsStr>,
    ) -> (PathBuf, PathBuf) {
        let prev = std::env::var_os("ENGRAM_DATA_DIR");
        match val {
            Some(v) => std::env::set_var("ENGRAM_DATA_DIR", v),
            None => std::env::remove_var("ENGRAM_DATA_DIR"),
        }
        let ours = default_data_dir();
        let theirs = daemon::default_data_dir();
        match &prev {
            Some(v) => std::env::set_var("ENGRAM_DATA_DIR", v),
            None => std::env::remove_var("ENGRAM_DATA_DIR"),
        }
        (ours, theirs)
    }

    #[test]
    fn the_shell_copies_resolve_the_paths_the_daemon_writes() {
        let guard = crate::discovery::DATA_DIR_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = std::env::temp_dir().join("engram-shell-same-path-test");

        let (unset_shell, unset_daemon) = both_data_dirs_with_env(&guard, None);
        let (empty_shell, empty_daemon) = both_data_dirs_with_env(&guard, Some(OsStr::new("")));
        let (set_shell, set_daemon) = both_data_dirs_with_env(&guard, Some(tmp.as_os_str()));

        // Precondition — the three states actually took different branches. If the environment could not be changed, nothing is measured even if the two copies are the same.
        assert!(
            unset_daemon.ends_with(".engram-dev"),
            "env 없음: 데몬이 debug 기본 폴더여야 {unset_daemon:?}"
        );
        assert!(
            empty_daemon.ends_with(".engram-dev"),
            "env 빈 값: 미설정과 같아야 {empty_daemon:?}"
        );
        assert_eq!(set_daemon, tmp, "env 임시 경로: 데몬이 그 경로 그대로여야");

        for (label, shell_root, daemon_root) in [
            ("env 없음", &unset_shell, &unset_daemon),
            ("env 빈 값", &empty_shell, &empty_daemon),
            ("env 임시 경로", &set_shell, &set_daemon),
        ] {
            assert_eq!(shell_root, daemon_root, "{label}: 루트가 같아야");
            let ours = DataLayout::new(daemon_root);
            let theirs = daemon::DataLayout::new(daemon_root);
            assert_eq!(
                ours.daemon_file(),
                theirs.daemon_file(),
                "{label}: daemon.json 자리가 같아야"
            );
            assert_eq!(
                ours.logs_dir(),
                theirs.logs_dir(),
                "{label}: 로그 폴더가 같아야"
            );
        }

        let exe_dir = Path::new("C:\\portable\\engram");
        assert_eq!(
            release_data_dir(exe_dir),
            daemon::release_data_dir(exe_dir),
            "릴리스 데이터 폴더 규칙이 같아야"
        );
    }
}
