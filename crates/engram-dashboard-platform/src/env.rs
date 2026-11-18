//! OS rules for the process environment: home directory, executable file name, path of an executable next to the running one,
//!   environment-variable name comparison. Uses only std.

// ADR-0230

use std::path::PathBuf;

/// The user home directory = one environment variable: Windows = `USERPROFILE`, elsewhere = `HOME`; `None` only when it is missing (an
///   empty value is not filtered). ★Do not swap in `std::env::home_dir`★ — on Windows, when the variable is missing it
/// falls through to an OS API and the meaning changes. Not looking at `HOME` on Windows is also deliberate — changing either makes a
///   caller that looks under home silently look at a different folder, with no error.
#[cfg(windows)]
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(PathBuf::from)
}
#[cfg(not(windows))]
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

pub fn exe_file_name(stem: &str) -> String {
    format!("{stem}{}", std::env::consts::EXE_SUFFIX)
}

/// Path of the `stem` executable in the running executable's folder — file name from [`exe_file_name`]. ★Does not check the file exists★ —
///   what to do after finding it (log, try the next candidate, filter) differs per caller. `None` only when the
/// running executable's path or folder cannot be obtained.
// ADR-0282
pub fn sibling_exe(stem: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join(exe_file_name(stem)))
}

/// Are two environment-variable names the same variable? Windows names are case-insensitive (`Path` = `PATH`) — only ASCII is folded;
///   other OSes must match exactly.
pub fn env_key_eq(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exe_file_name_appends_exe_only_on_windows() {
        let expected = if cfg!(windows) {
            "engram.exe"
        } else {
            "engram"
        };
        assert_eq!(exe_file_name("engram"), expected);
    }

    #[test]
    fn sibling_exe_is_the_named_exe_in_the_running_exe_folder() {
        let running = std::env::current_exe().expect("current executable path");
        let want = running
            .parent()
            .expect("current executable's folder")
            .join(exe_file_name("engram"));
        assert_eq!(sibling_exe("engram"), Some(want));
    }

    #[test]
    fn env_key_eq_folds_ascii_case_only_on_windows() {
        assert!(env_key_eq("PATH", "PATH"));
        assert_eq!(env_key_eq("Path", "PATH"), cfg!(windows));
        assert!(!env_key_eq("PATH", "PATHEXT"));
    }
}
