//! ★Canonical display name derivation (ADR-0101 WYSIWYA)★ — derives the last path segment (basename) of the cwd.
//!
//! ★Single source (backend)★: this is a Rust port of the frontend `src/util/basename.ts` with **exactly the same rules**.
//!   The invariant "the visible name = the name used as the address" (WYSIWYA) holds only if the backend (routing · roster · envelope sender) and the frontend (tree · popup display)
//!   derive the same string. If each copies it, the win/posix · root/drive-root/UNC edges
//!   diverge, and a mismatch arises where the tree shows "engram" but routing expects a different string —
//!   so the rule is pinned in one place and matched 1:1 with the frontend rule (sealed by the corresponding tests).
//!
//! The edge behaviors below mirror `basename.ts` verbatim — if the frontend rule changes, change this side together.

/// A stable placeholder for when the cwd is empty or has no segment to derive (prevents a blank label).
/// It must be the same string as the frontend `PATH_NAME_PLACEHOLDER` (display consistency).
pub const PATH_NAME_PLACEHOLDER: &str = "(경로 없음)";

/// ★The return value is never blank (empty / whitespace-only)★: the upper layer (display name) draws from this one value alone, so if it is blank
///   it shows as an empty area. When there is neither a basename to derive nor a raw cwd, it degrades to the placeholder.
///
/// Edge cases (when the basename is absent or misleading) are not derived and degrade to the raw cwd (same as the frontend):
///   - empty/whitespace-only string → `PATH_NAME_PLACEHOLDER`
///   - drive-root `C:\` / `C:/` / `C:` → keep the original (collapsing to "C:" would mislead)
///   - posix root `/` · UNC `\\server\share`, where after stripping the trailing separator there is no segment or it equals the original
///     → return the raw cwd (prevents collapse into a wrong segment).
pub fn cwd_basename(cwd: &str) -> String {
    // ★Slight whitespace-set difference from JS trim (an accepted divergence)★: the whitespace recognized by Rust str::trim(White_Space) and
    //   JS String.trim differs a little at U+FEFF · U+0085 and so on — a cwd made only of such exotic whitespace may fall to the placeholder on one side only and
    //   diverge. But a real filesystem cwd is never such a string, so we do not chase a complete match of the JS
    //   whitespace set (no effect on real paths).
    if cwd.trim().is_empty() {
        return PATH_NAME_PLACEHOLDER.to_string();
    }
    // Strip the trailing separator (both `/` and `\`). Equivalent to the frontend regex `/[\\/]+$/`.
    let trimmed = cwd.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return cwd.to_string();
    }
    // The position of the last separator (the larger index of the two). Equivalent to the frontend `Math.max(lastIndexOf('/'), lastIndexOf('\\'))`.
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

/// ★Canonical display name derivation for an agent with a profile (ADR-0101)★ — if display_name (override) exists, it as is,
///   otherwise the cwd basename. It is the **pure core of the chain shared by manager (agent_info) and daemon (sender_display_name)**.
///   The id-prefix fallback for a missing profile (ad-hoc) is attached by each caller (so as not to pass the id here —
///   this function stays a pure derivation in 1:1 correspondence with the frontend tree rule `display_name ?? basename(cwd)`).
pub fn resolve_display_name(display_name: Option<&str>, cwd: &str) -> String {
    match display_name {
        Some(n) => n.to_string(),
        None => cwd_basename(cwd),
    }
}

/// ★Canonical name derivation when the profile is missing (ad-hoc / DeleteProfile on a live session) (ADR-0101)★ —
///   derive from the session cwd basename without an override, and degrade to the first 8 chars of the id only when the cwd yields a placeholder/empty value.
///   A shared core so manager and reaper use the **same fallback** (no logic duplication):
///   the frontend tree draws even a live session whose profile vanished by basename(AgentInfo.cwd), so if the backend falls straight to
///   the id-prefix, tree ≠ routing arises. That is why the cwd basename is tried first.
///   A whitespace-only display_name is treated as no override.
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

/// Is it a lone [letter]+colon like `C:`. Equivalent to the frontend regex `/^[A-Za-z]:$/`.
fn is_drive_root(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

#[cfg(test)]
mod tests {
    use super::*;

    // The cases below correspond 1:1 to the frontend `src/util/basename.test.ts` (WYSIWYA seal — if one side changes,
    //   the corresponding case breaks and immediately shows that the two rules diverged).

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
        // Tree rename scenario.
        assert_eq!(
            resolve_display_name(Some("ABC"), "C:\\work\\Filter Library"),
            "ABC"
        );
    }

    #[test]
    fn resolve_falls_back_to_cwd_basename_when_no_override() {
        // It is the cwd basename, not profile.name.
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
        // ★Even with the profile missing, the tree draws basename(cwd), so it does not fall straight to the id-prefix★
        //   — on a live session where only the profile vanished through DeleteProfile, tree == routing is kept.
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
