//! ★Canonical display name derivation (ADR-0101 WYSIWYA)★ — derives the last path segment (basename) of the cwd.
//!
//! ★Single source (backend)★: a Rust port of the frontend `src/util/basename.ts` with **exactly the same rules**. "The visible name = the
//!   name used as the address" (WYSIWYA) holds only if backend (routing · roster · envelope sender) and frontend (tree · popup) derive the
//!   same string; separate copies would diverge at win/posix · root/drive-root/UNC edges (tree shows "engram", routing expects another
//!   string). Pinned in one place, matched 1:1 with the frontend rule (sealed by the corresponding tests).
//!
//! The edge behaviors below mirror `basename.ts` verbatim — if the frontend rule changes, change this side too.

/// A stable placeholder for an empty cwd or one with no derivable segment (prevents a blank label). Production callers go through
/// [`canonical_name_or_id_fallback`], which turns it into the id prefix, so it never becomes a name. Not coupled to the frontend at run time
/// (its blank-path label is the UI-language key `common.noPath`).
pub const PATH_NAME_PLACEHOLDER: &str = "(no path)";

/// ★The return value is never blank (empty / whitespace-only)★: the display name draws from this value alone, so blank would show an empty
///   area. With neither a derivable basename nor a raw cwd, it degrades to the placeholder.
///
/// Edge cases (basename absent or misleading) degrade to the raw cwd, as in the frontend:
///   - empty/whitespace-only string → `PATH_NAME_PLACEHOLDER`
///   - drive-root `C:\` / `C:/` / `C:` → keep the original (collapsing to "C:" would mislead)
///   - posix root `/` · UNC `\\server\share`, where stripping the trailing separator leaves no segment or the original → the raw cwd.
pub fn cwd_basename(cwd: &str) -> String {
    // ★Slight whitespace-set difference from JS trim (accepted divergence)★: Rust str::trim(White_Space) and JS String.trim differ at
    //   U+FEFF · U+0085 etc.; a cwd of only such whitespace may hit the placeholder on one side only. A real filesystem cwd is never such
    //   a string, so no full match is chased.
    if cwd.trim().is_empty() {
        return PATH_NAME_PLACEHOLDER.to_string();
    }
    // Strip the trailing separator (both `/` and `\`). Equivalent to the frontend regex `/[\\/]+$/`.
    let trimmed = cwd.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return cwd.to_string();
    }
    // The position of the last separator (the larger index of the two). Equivalent to the frontend `Math.max(lastIndexOf('/'),
    //   lastIndexOf('\\'))`.
    let idx = trimmed.rfind(['/', '\\']).map(|i| i as isize).unwrap_or(-1);
    let base = if idx >= 0 {
        // idx is a valid byte boundary (separator = 1 ASCII byte), so slicing is safe.
        &trimmed[(idx as usize) + 1..]
    } else {
        trimmed
    };
    if base.is_empty() || is_drive_root(base) {
        return cwd.to_string();
    }
    base.to_string()
}

/// ★Canonical display name for an agent with a profile (ADR-0101)★ — display_name (override) as is if present, otherwise the cwd basename.
///   The **pure core of the chain shared by manager (agent_info) and daemon (sender_display_name)**, 1:1 with the frontend tree rule
///   `display_name ?? basename(cwd)`; the id-prefix fallback for a missing profile (ad-hoc) is attached by each caller.
pub fn resolve_display_name(display_name: Option<&str>, cwd: &str) -> String {
    match display_name {
        Some(n) => n.to_string(),
        None => cwd_basename(cwd),
    }
}

/// ★Canonical name when the profile is missing (ad-hoc / DeleteProfile on a live session) (ADR-0101)★ — the session cwd basename without
///   an override, degrading to the first 8 chars of the id only when the cwd yields a placeholder/empty value. A shared core so manager
///   and reaper use the **same fallback**. The cwd basename is tried first because the frontend tree draws even a profile-less live
///   session by basename(AgentInfo.cwd) — falling straight to the id-prefix would make tree ≠ routing. A whitespace-only display_name
///   counts as no override.
pub fn canonical_name_or_id_fallback(
    display_name: Option<&str>,
    cwd: &str,
    id: crate::types::AgentId,
) -> String {
    if let Some(n) = display_name {
        if !n.trim().is_empty() {
            return n.to_string();
        }
    }
    let base = cwd_basename(cwd);
    // A real filesystem cwd does not reach this branch — it is for defending the edge where the cwd is empty after DeleteProfile.
    if base == PATH_NAME_PLACEHOLDER {
        let s = id.to_string();
        return s[..8.min(s.len())].to_string();
    }
    base
}

/// Equivalent to the frontend regex `/^[A-Za-z]:$/`.
fn is_drive_root(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

#[cfg(test)]
mod tests {
    use super::*;

    // The cases below correspond 1:1 to the frontend `src/util/basename.test.ts` (WYSIWYA seal — a one-sided change breaks the matching case).

    #[test]
    fn posix_path_basename() {
        assert_eq!(cwd_basename("/home/me/project"), "project");
    }

    #[test]
    fn windows_path_basename() {
        assert_eq!(cwd_basename("C:\\work\\engram"), "engram");
    }

    #[test]
    fn trailing_separator_ignored() {
        assert_eq!(cwd_basename("C:/proj/"), "proj");
        assert_eq!(cwd_basename("/a/b/c/"), "c");
    }

    #[test]
    fn no_segment_root_falls_back_to_raw() {
        assert_eq!(cwd_basename("/"), "/");
        assert_eq!(cwd_basename("projectonly"), "projectonly");
    }

    #[test]
    fn drive_root_kept_raw() {
        assert_eq!(cwd_basename("C:\\"), "C:\\");
        assert_eq!(cwd_basename("C:/"), "C:/");
        assert_eq!(cwd_basename("C:"), "C:");
    }

    #[test]
    fn empty_or_whitespace_returns_placeholder() {
        assert_eq!(cwd_basename(""), PATH_NAME_PLACEHOLDER);
        assert_eq!(cwd_basename("   "), PATH_NAME_PLACEHOLDER);
        assert!(!PATH_NAME_PLACEHOLDER.trim().is_empty());
    }

    #[test]
    fn unc_share_derives_last_segment() {
        assert_eq!(cwd_basename("\\\\server\\share"), "share");
        assert_eq!(cwd_basename("\\\\server\\share\\"), "share");
    }

    // ── resolve_display_name (canonical name chain — the frontend tree `display_name ?? basename(cwd)`) ──

    #[test]
    fn resolve_prefers_display_name_override() {
        assert_eq!(
            resolve_display_name(Some("ABC"), "C:\\work\\Filter Library"),
            "ABC"
        );
    }

    #[test]
    fn resolve_falls_back_to_cwd_basename_when_no_override() {
        assert_eq!(resolve_display_name(None, "C:\\work\\engram"), "engram");
        assert_eq!(resolve_display_name(None, "/home/me/project"), "project");
    }

    #[test]
    fn resolve_empty_cwd_falls_back_to_placeholder() {
        assert_eq!(resolve_display_name(None, ""), PATH_NAME_PLACEHOLDER);
    }

    // ── canonical_name_or_id_fallback (missing-profile fallback — core shared by manager · reaper, ADR-0101) ──

    #[test]
    fn fallback_prefers_display_name_override() {
        let id = uuid::Uuid::new_v4();
        assert_eq!(
            canonical_name_or_id_fallback(Some("Alice"), "C:\\work\\engram", id),
            "Alice"
        );
    }

    #[test]
    fn fallback_uses_session_cwd_basename_when_no_override() {
        // ★Even with the profile missing, the tree draws basename(cwd), so it does not fall straight to the id-prefix★ — tree == routing is kept
        //   on a live session whose profile vanished through DeleteProfile.
        let id = uuid::Uuid::new_v4();
        assert_eq!(
            canonical_name_or_id_fallback(None, "C:\\work\\engram", id),
            "engram"
        );
        assert_eq!(
            canonical_name_or_id_fallback(None, "/home/me/project", id),
            "project"
        );
    }

    #[test]
    fn fallback_blank_override_ignored_uses_cwd() {
        let id = uuid::Uuid::new_v4();
        assert_eq!(
            canonical_name_or_id_fallback(Some("   "), "/home/me/project", id),
            "project"
        );
    }

    #[test]
    fn fallback_id_prefix_only_when_cwd_empty() {
        let id = uuid::Uuid::new_v4();
        let expected: String = id.to_string().chars().take(8).collect();
        assert_eq!(canonical_name_or_id_fallback(None, "", id), expected);
        assert_eq!(canonical_name_or_id_fallback(None, "   ", id), expected);
    }
}
