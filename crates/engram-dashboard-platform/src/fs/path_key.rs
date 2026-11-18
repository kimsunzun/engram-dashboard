//! Path keys — paths with only case folded as the OS does. [`same_path`] always answers the same as "the two [`path_key`]s are equal".

// ADR-0325

use std::ffi::OsString;
use std::path::Path;

/// [`path_key`]'s answer — a key for tables keyed by path. It is not an openable path and does not turn back into one.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PathKey(OsString);

/// A key with only case folded — two paths whose spelling differs only in case get the same key. Other spelling differences are not
/// folded, so to get the same key for the same file the caller picks the spelling first (separators · relative form · prefix).
///
/// - Windows = folds each UTF-16 code unit with the OS uppercase mapping (`LCMapStringEx` invariant uppercase). Supplementary-plane
///   characters (surrogate pairs) and unpaired surrogates stay as is — NTFS does the same (U+10428 and U+10400 are different names).
///   No folding that expands one character into several (`ß` → `SS`).
/// - Elsewhere = as is (case-sensitive). macOS case-insensitivity and NFC/NFD equivalence are not reflected (known gap).
/// - Separators (`a/b` vs `a\b`) · `.` · `..` · a trailing separator · the `\?\` prefix · relative vs absolute stay as is, so the
///   same file spelled differently is a different key.
/// - ★The real comparison is per volume★ — NTFS uses the uppercase table from format time · exFAT its own table · an SMB share the
///   server's rules. This key uses the one OS table, so it can disagree on rare characters of a volume with a different table.
///   Per-folder case-sensitive folders (WSL `fsutil setCaseSensitiveInfo`) are unsupported.
#[cfg(windows)]
pub fn path_key(path: &Path) -> PathKey {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    let table = upcase_table();
    let folded: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .map(|unit| table[usize::from(unit)])
        .collect();
    PathKey(OsString::from_wide(&folded))
}
#[cfg(not(windows))]
pub fn path_key(path: &Path) -> PathKey {
    PathKey(path.as_os_str().to_owned())
}

/// Whether two paths are spelled the same except for case — exactly `path_key(a) == path_key(b)` (rules · the spelling the caller
/// picks = [`path_key`]). It compares a pair; tables keyed by path use [`path_key`].
pub fn same_path(a: &Path, b: &Path) -> bool {
    path_key(a) == path_key(b)
}

// Measured 2026-11-01 (Windows 11 26200 · three NTFS volumes whose root folders were created (a proxy for format date) 2018-08 ·
//   2019-03 · 2025-08): this mapping and `RtlUpcaseUnicodeChar` gave the same answer on all 63,488 non-surrogate BMP units (why
//   the latter is not used = the `Win32_Globalization` comment in `Cargo.toml`), and matched NTFS's same-name decision on all
//   2378 case-pair candidates. exFAT and network shares were not measured (no exFAT volume on this PC).
#[cfg(windows)]
fn upcase_table() -> &'static [u16] {
    static TABLE: std::sync::OnceLock<Box<[u16]>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| (0..=u16::MAX).map(upcase_unit).collect())
}

// If the mapping fails or answers with more than one unit, that unit is not folded — the key stays consistent within one process
//   (the table is built once). No unit failed in the measurement.
#[cfg(windows)]
fn upcase_unit(unit: u16) -> u16 {
    use windows::Win32::Foundation::LPARAM;
    use windows::Win32::Globalization::{LCMapStringEx, LCMAP_UPPERCASE, LOCALE_NAME_INVARIANT};
    if (0xD800..=0xDFFF).contains(&unit) {
        return unit;
    }
    let mut out = [0u16; 2];
    // SAFETY: input and output are both slices with their lengths; the other pointer arguments are empty.
    let written = unsafe {
        LCMapStringEx(
            LOCALE_NAME_INVARIANT,
            LCMAP_UPPERCASE,
            &[unit],
            Some(&mut out),
            None,
            None,
            LPARAM(0),
        )
    };
    if written == 1 {
        out[0]
    } else {
        unit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // (a, b, does NTFS read them as the same name) — measured 2026-11-01 (the volume set of the mapping comment above). On other
    //   OSes any spelling difference is a different name.
    const PILOT_PAIRS: &[(&str, &str, bool)] = &[
        ("a", "A", true),
        ("\u{E9}", "\u{C9}", true),     // é · É
        ("\u{FF}", "\u{178}", true),    // ÿ · Ÿ
        ("\u{3C3}", "\u{3A3}", true),   // Greek σ · Σ
        ("\u{430}", "\u{410}", true),   // Cyrillic а · А
        ("\u{FF41}", "\u{FF21}", true), // fullwidth ａ · Ａ
        ("\u{B5}", "\u{39C}", false),   // micro sign · Greek capital mu
        ("\u{DF}", "\u{1E9E}", false),  // ß · capital ẞ
        ("\u{DF}", "SS", false),        // ß · two letters
        ("\u{3C2}", "\u{3A3}", false),  // final sigma ς · Σ
        ("\u{131}", "I", false),        // dotless ı
        ("\u{130}", "i", false),        // dotted İ
        ("\u{130}", "I", false),
        ("\u{17F}", "S", false),  // long ſ
        ("\u{212A}", "K", false), // Kelvin sign
        ("\u{212A}", "k", false),
        ("\u{10D0}", "\u{1C90}", false), // Georgian Mkhedruli · Mtavruli (Unicode 11)
        ("\u{10D0}", "\u{10A0}", false), // Georgian Mkhedruli · Asomtavruli
        ("\u{AB70}", "\u{13A0}", false), // Cherokee small · capital (Unicode 8)
        ("\u{13F8}", "\u{13F0}", false),
        ("\u{10428}", "\u{10400}", false), // Deseret — supplementary plane
        ("\u{1E922}", "\u{1E900}", false), // Adlam — supplementary plane
        ("\u{104D8}", "\u{104B0}", false), // Osage — supplementary plane
    ];

    #[test]
    fn pilot_pairs_fold_like_ntfs_on_windows_and_never_elsewhere() {
        for (a, b, ntfs_same) in PILOT_PAIRS {
            let (a, b) = (format!("lib/{a}.md"), format!("LIB/{b}.MD"));
            assert_eq!(
                same_path(Path::new(&a), Path::new(&b)),
                *ntfs_same && cfg!(windows),
                "{a} · {b}"
            );
        }
    }

    #[test]
    fn same_path_always_answers_like_key_equality() {
        let names: Vec<String> = PILOT_PAIRS
            .iter()
            .flat_map(|(a, b, _)| [a.to_string(), b.to_string()])
            .chain(["Lib/Preset.md", "lib\\preset.md", "lib/./preset.md", ""].map(String::from))
            .collect();
        for a in &names {
            for b in &names {
                let (a, b) = (Path::new(a), Path::new(b));
                assert_eq!(same_path(a, b), path_key(a) == path_key(b), "{a:?} · {b:?}");
            }
        }
    }

    #[test]
    fn only_case_is_folded_not_spelling() {
        assert!(!same_path(
            Path::new("lib/preset.md"),
            Path::new("lib/./preset.md")
        ));
        assert!(!same_path(
            Path::new("lib/preset.md"),
            Path::new("lib/preset.md/")
        ));
        assert!(!same_path(
            Path::new("lib/preset.md"),
            Path::new("lib\\preset.md")
        ));
        assert_eq!(
            same_path(Path::new("Lib/Preset.md"), Path::new("lib/PRESET.md")),
            cfg!(windows)
        );
    }

    #[cfg(windows)]
    #[test]
    fn lone_surrogates_pass_through_and_the_rest_still_folds() {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        let wide = |units: &[u16]| std::path::PathBuf::from(OsString::from_wide(units));
        let lower = wide(&[0x61, 0xD801, 0x62]);
        let upper = wide(&[0x41, 0xD801, 0x42]);
        let other = wide(&[0x41, 0xDC01, 0x42]);

        assert!(same_path(&lower, &upper));
        assert!(!same_path(&upper, &other));
        let key: Vec<u16> = path_key(&lower).0.encode_wide().collect();
        assert_eq!(key, [0x41, 0xD801, 0x42]);
    }

    // Measures with real files whether the table agrees with the volume this test runs on (the temp folder) — if creating name b
    //   while a file named a exists says "already exists", that volume reads the two as the same name.
    #[cfg(windows)]
    fn volume_reads_same(dir: &Path, a: &str, b: &str) -> bool {
        let first = dir.join(format!("p{a}q"));
        std::fs::File::create_new(&first).expect("create with name a");
        let second = dir.join(format!("p{b}q"));
        let same = match std::fs::File::create_new(&second) {
            Ok(_) => {
                std::fs::remove_file(&second).expect("remove name b");
                false
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => true,
            Err(e) => panic!("create with name b {a} · {b}: {e}"),
        };
        std::fs::remove_file(&first).expect("remove name a");
        same
    }

    #[cfg(windows)]
    struct Scratch(std::path::PathBuf);

    #[cfg(windows)]
    impl Scratch {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "engram-platform-path-key-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir(&dir).expect("test folder");
            Self(dir)
        }
    }

    #[cfg(windows)]
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    // Compares, with real files, each BMP character against its likely case-pair candidates. Rust's case conversion only widens the
    //   candidates — it is not used for the key (it would merge supplementary-plane names · ADR-0325).
    #[cfg(windows)]
    #[test]
    fn every_bmp_case_candidate_agrees_with_the_volume_under_temp() {
        fn name_unit(unit: u16) -> bool {
            unit >= 0x20
                && !(0xD800..=0xDFFF).contains(&unit)
                && !matches!(
                    unit,
                    0x22 | 0x2A | 0x2F | 0x3A | 0x3C | 0x3E | 0x3F | 0x5C | 0x7C | 0x7F
                )
        }
        fn single_bmp(mut chars: impl Iterator<Item = char>) -> Option<u16> {
            match (chars.next(), chars.next()) {
                (Some(c), None) => u16::try_from(u32::from(c)).ok(),
                _ => None,
            }
        }
        let scratch = Scratch::new();
        let table = upcase_table();
        let mut compared = 0;
        for unit in (0..=u16::MAX).filter(|&u| name_unit(u)) {
            let ch = char::from_u32(u32::from(unit)).expect("surrogates were filtered");
            let mut candidates = vec![table[usize::from(unit)]];
            candidates.extend(single_bmp(ch.to_uppercase()));
            candidates.extend(single_bmp(ch.to_lowercase()));
            candidates.sort_unstable();
            candidates.dedup();
            for other in candidates
                .into_iter()
                .filter(|&o| o != unit && name_unit(o))
            {
                let a = String::from_utf16(&[unit]).expect("one BMP character");
                let b = String::from_utf16(&[other]).expect("one BMP character");
                assert_eq!(
                    same_path(Path::new(&a), Path::new(&b)),
                    volume_reads_same(&scratch.0, &a, &b),
                    "U+{unit:04X} · U+{other:04X}"
                );
                compared += 1;
            }
        }
        assert!(compared > 2000, "too few candidate pairs: {compared}");
    }

    #[cfg(windows)]
    #[test]
    fn pilot_pairs_agree_with_the_volume_under_temp() {
        let scratch = Scratch::new();
        for (a, b, _) in PILOT_PAIRS {
            assert_eq!(
                same_path(Path::new(a), Path::new(b)),
                volume_reads_same(&scratch.0, a, b),
                "{a} · {b}"
            );
        }
    }
}
