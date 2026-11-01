//! 경로 열쇠 — 대소문자만 OS 처럼 접은 경로. [`same_path`] 는 늘 「두 [`path_key`] 가 같다」 와 같은 답이다.

// ADR-0325

use std::ffi::OsString;
use std::path::Path;

/// [`path_key`] 의 답 — 경로를 열쇠로 쓰는 표의 열쇠다. 열 수 있는 경로가 아니라 경로로 되돌리지 않는다.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PathKey(OsString);

/// 대소문자만 접은 열쇠 — 철자가 대소문자에서만 다른 두 경로는 같은 열쇠다. 철자의 다른 차이는 접지 않으므로
/// 같은 파일을 같은 열쇠로 받으려면 부르는 쪽이 먼저 철자를 고른다(구분자 · 상대 꼴 · 접두).
///
/// - Windows = UTF-16 코드 단위마다 OS 대문자 매핑(`LCMapStringEx` invariant 대문자)으로 접는다. 보충 평면
///   글자(서로게이트 쌍)와 짝 없는 서로게이트는 그대로 둔다 — NTFS 가 그렇다(U+10428 과 U+10400 은 다른 이름).
///   한 글자를 여러 글자로 펴는 접기(`ß` → `SS`)도 하지 않는다.
/// - 그 밖 = 그대로(대소문자를 가린다). macOS 의 대소문자 무시 · NFC/NFD 같음은 반영하지 않는다(알려진 갭).
/// - 구분자(`a/b` 와 `a\b`) · `.` · `..` · 끝 구분자 · `\\?\` 접두 · 상대와 절대는 그대로라, 같은 파일이어도 그
///   철자가 다르면 다른 열쇠다.
/// - ★실제 비교는 볼륨마다다★ — NTFS 는 포맷 때의 대문자 표 · exFAT 은 제 표 · SMB 공유는 서버 규칙을 쓴다. 이
///   열쇠는 OS 표 하나라 그 표와 다른 볼륨의 드문 글자에서 어긋날 수 있다. 폴더마다 대소문자를 가리는 폴더(WSL
///   `fsutil setCaseSensitiveInfo`)는 지원 밖이다.
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

/// 두 경로의 철자가 대소문자 말고는 같은가 — `path_key(a) == path_key(b)` 그대로다(규칙 · 부르는 쪽이 고를 철자 =
/// [`path_key`]). 짝 견주기라 경로를 열쇠로 쓰는 표에는 [`path_key`] 를 쓴다.
pub fn same_path(a: &Path, b: &Path) -> bool {
    path_key(a) == path_key(b)
}

// 실측 2026-11-01(Windows 11 26200 · 루트 폴더 만든 때(포맷 날짜 대용) 2018-08 · 2019-03 · 2025-08 의 NTFS 볼륨 셋):
//   이 매핑과 `RtlUpcaseUnicodeChar` 는 서로게이트 아닌 BMP 단위 63,488 개에서 답이 같았고(그것을 고르지 않은 까닭 =
//   `Cargo.toml` 의 `Win32_Globalization` 주석), 대소문자 짝 후보 2378 쌍에서 NTFS 의 같은 이름 판정과 모두 맞았다.
//   exFAT · 네트워크 공유는 재지 않았다(이 PC 에 exFAT 볼륨이 없다).
#[cfg(windows)]
fn upcase_table() -> &'static [u16] {
    static TABLE: std::sync::OnceLock<Box<[u16]>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| (0..=u16::MAX).map(upcase_unit).collect())
}

// 매핑이 실패하거나 한 단위가 아닌 답을 내면 그 단위는 접지 않는다 — 열쇠는 그래도 한 프로세스 안에서 한결같다
//   (표를 한 번만 짓는다). 실측에서 실패한 단위는 없었다.
#[cfg(windows)]
fn upcase_unit(unit: u16) -> u16 {
    use windows::Win32::Foundation::LPARAM;
    use windows::Win32::Globalization::{LCMapStringEx, LCMAP_UPPERCASE, LOCALE_NAME_INVARIANT};
    if (0xD800..=0xDFFF).contains(&unit) {
        return unit;
    }
    let mut out = [0u16; 2];
    // SAFETY: 입력 · 출력 둘 다 길이가 실린 슬라이스이고 나머지 포인터 인자는 비운다.
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

    // (a, b, NTFS 가 같은 이름으로 읽나) — 실측 2026-11-01(위 매핑 주석의 볼륨 셋). 그 밖의 OS 에서는 철자가 다르면
    //   전부 다른 이름이다.
    const PILOT_PAIRS: &[(&str, &str, bool)] = &[
        ("a", "A", true),
        ("\u{E9}", "\u{C9}", true),     // é · É
        ("\u{FF}", "\u{178}", true),    // ÿ · Ÿ
        ("\u{3C3}", "\u{3A3}", true),   // 그리스 σ · Σ
        ("\u{430}", "\u{410}", true),   // 키릴 а · А
        ("\u{FF41}", "\u{FF21}", true), // 전각 ａ · Ａ
        ("\u{B5}", "\u{39C}", false),   // 마이크로 기호 · 그리스 대문자 뮤
        ("\u{DF}", "\u{1E9E}", false),  // ß · 대문자 ẞ
        ("\u{DF}", "SS", false),        // ß · 두 글자
        ("\u{3C2}", "\u{3A3}", false),  // 끝 시그마 ς · Σ
        ("\u{131}", "I", false),        // 점 없는 ı
        ("\u{130}", "i", false),        // 점 있는 İ
        ("\u{130}", "I", false),
        ("\u{17F}", "S", false),  // 긴 ſ
        ("\u{212A}", "K", false), // 켈빈 기호
        ("\u{212A}", "k", false),
        ("\u{10D0}", "\u{1C90}", false), // 조지아 므헤드룰리 · 므타브룰리(유니코드 11)
        ("\u{10D0}", "\u{10A0}", false), // 조지아 므헤드룰리 · 아솜타브룰리
        ("\u{AB70}", "\u{13A0}", false), // 체로키 소문자 · 대문자(유니코드 8)
        ("\u{13F8}", "\u{13F0}", false),
        ("\u{10428}", "\u{10400}", false), // 데저렛 — 보충 평면
        ("\u{1E922}", "\u{1E900}", false), // 아들람 — 보충 평면
        ("\u{104D8}", "\u{104B0}", false), // 오세이지 — 보충 평면
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

    // 표의 답이 이 시험이 도는 볼륨(임시 폴더)과 맞는지 실 파일로 잰다 — 이름 a 의 파일을 둔 채 이름 b 로 새로
    //   만들기가 「이미 있다」 면 그 볼륨은 둘을 같은 이름으로 읽는다.
    #[cfg(windows)]
    fn volume_reads_same(dir: &Path, a: &str, b: &str) -> bool {
        let first = dir.join(format!("p{a}q"));
        std::fs::File::create_new(&first).expect("이름 a 로 만들기");
        let second = dir.join(format!("p{b}q"));
        let same = match std::fs::File::create_new(&second) {
            Ok(_) => {
                std::fs::remove_file(&second).expect("이름 b 지우기");
                false
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => true,
            Err(e) => panic!("이름 b 로 만들기 {a} · {b}: {e}"),
        };
        std::fs::remove_file(&first).expect("이름 a 지우기");
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
            std::fs::create_dir(&dir).expect("시험 폴더");
            Self(dir)
        }
    }

    #[cfg(windows)]
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    // BMP 의 글자마다 그 글자와 대소문자 짝일 만한 후보를 실 파일로 견준다. Rust 의 대소문자 변환은 후보를 넓히는 데만
    //   쓴다 — 열쇠에는 쓰지 않는다(보충 평면 이름을 합친다 · ADR-0325).
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
            let ch = char::from_u32(u32::from(unit)).expect("서로게이트는 걸렀다");
            let mut candidates = vec![table[usize::from(unit)]];
            candidates.extend(single_bmp(ch.to_uppercase()));
            candidates.extend(single_bmp(ch.to_lowercase()));
            candidates.sort_unstable();
            candidates.dedup();
            for other in candidates
                .into_iter()
                .filter(|&o| o != unit && name_unit(o))
            {
                let a = String::from_utf16(&[unit]).expect("BMP 한 글자");
                let b = String::from_utf16(&[other]).expect("BMP 한 글자");
                assert_eq!(
                    same_path(Path::new(&a), Path::new(&b)),
                    volume_reads_same(&scratch.0, &a, &b),
                    "U+{unit:04X} · U+{other:04X}"
                );
                compared += 1;
            }
        }
        assert!(compared > 2000, "후보 쌍이 너무 적다: {compared}");
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
