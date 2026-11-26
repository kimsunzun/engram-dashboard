//! The host's OS rules as plain answers, for a caller whose own text follows them — no I/O.
// ADR-0266

/// Whether the host uses Windows path rules (`\` as a separator, a drive or UNC prefix). A caller that writes a path
/// into text another program reads (a script path, a config key) asks this instead of branching on the OS itself.
// ADR-0420
pub fn windows_paths() -> bool {
    cfg!(windows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_paths_answers_whether_the_host_separates_paths_with_a_backslash() {
        assert_eq!(windows_paths(), std::path::MAIN_SEPARATOR == '\\');
    }
}
