//! 설정 파일(`<셸 config 폴더>\settings.json`)을 아는 유일한 자리 — 이름 · 위치 · 형식 · 관용 규칙(TRD §5-3).
//!
//! 형식 = 평평한 점 키 객체 + 예약 키 `"$version": 1`. **기본값과 다른 값만 적는다.**
//!
//! - **적재([`load`])는 파일을 만들지도 고치지도 않는다** — 셸 빌드 전에 돈다(TRD §5-3 서비스 수명). 로거도
//!   그 전이라 적재는 로그를 내지 않고 [`LoadNote`] 로 모은다 — 서비스가 로거가 선 뒤에 낸다.
//! - **쓰기([`write`])는 읽고-고치고-쓰기다** — 호출자가 읽은 문서([`read_document`])에서 받은 키만 바꾼다.
//!   앱 밖에서 고친 다른 키와 모르는 키가 그대로 남는다. 서비스의 쓰기 직렬화 락 아래서만 부른다 — 두
//!   쓰기의 읽고-고치고-쓰기가 겹치면 나중 쓰기가 먼저 쓰기의 변경을 지운다.
//! - **통째로 못 쓰는 파일**(JSON 아님 · 객체 아님 · 상한 초과 · UTF-8 아님 · 모르는 `$version`)은 적재가
//!   기본값으로 접고 손대지 않는다. 그 위에 쓰는 첫 쓰기가 원본을 `settings.json.corrupt` 로 **떠 둔 뒤** 그
//!   자리를 원자적으로 갈아끼운다 — 파일이 없는 순간이 없다. 새 파일은 메모리의 값에서 다시 짓는다. 사본은
//!   하나뿐이다([`SettingsFiles::copy_aside`]). 떠 두기가 실패하면 그 쓰기도 실패하고 원본은 그대로다.
//!   ★읽기 자체의 IO 실패는 그 무리가 아니다★ — 잠깐 잠긴 멀쩡한 파일을 덮지 않도록 그때 쓰기는 실패한다.
//! - **아는 키의 못 쓸 값**은 기본값으로 접는다 — 파일에서는 그 키를 쓰거나 되돌릴 때까지 그대로다.
// ADR-0265

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::registry::{self, SETTINGS};

const SETTINGS_FILE: &str = "settings.json";
const VERSION_KEY: &str = "$version";
const VERSION: u64 = 1;
/// 사람이 손으로 늘려도 여기까진 정상인 선 — 실제 크기는 1 KiB 안팎이다. ★쓰기도 이 선을 넘지 않는다★ —
/// 넘는 원문을 쓰면 다음 적재가 그 파일 전체를 못 쓴다고 접는다.
const MAX_SETTINGS_BYTES: u64 = 64 * 1024;

/// 파일 원문을 가져온 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawFile {
    Missing,
    Text(String),
    /// 파일은 있으나 원문이 못 된다(상한 초과 · UTF-8 아님) — 사유 문구. 손상과 같은 무게로 다룬다.
    Unusable(String),
}

/// 설정 파일 하나에 대한 IO seam — 시험은 메모리 가짜(`fake::MemFiles`)로 돈다.
///
/// ★서비스의 쓰기 직렬화 락을 쥔 채 불린다★ — 구현이 [`super::SettingsService`] 의 쓰기를 다시 부르면
/// 교착이다(읽기는 그 락을 안 잡는다).
// ADR-0012
pub trait SettingsFiles: Send {
    /// `Err` = 읽기 자체의 IO 실패(권한 · 공유 위반 등). 이때 쓰기는 파일을 덮지 않고 실패한다.
    fn read(&self) -> io::Result<RawFile>;
    /// 원자적으로 통째로 갈아끼운다. 폴더가 없으면 만든다.
    fn write_atomic(&self, text: &str) -> io::Result<()>;
    /// 지금 파일을 옆 이름(`<이름>.corrupt`)에 **떠 두고** 그 자리를 돌려준다 — 원본은 그 자리에 그대로 둔다.
    /// ★앞서 떠 둔 사본을 덮는다★(이름이 하나뿐 — [`crate::fsutil::copy_aside`]). `Err` 면 앞선 사본도 그대로다.
    fn copy_aside(&self) -> io::Result<PathBuf>;
    /// 로그에 실을 출처(경로).
    fn origin(&self) -> String;
}

/// 운영 구현 — `<dir>\settings.json`.
pub struct FsSettingsFiles {
    path: PathBuf,
}

impl FsSettingsFiles {
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            path: dir.join(SETTINGS_FILE),
        }
    }
}

impl SettingsFiles for FsSettingsFiles {
    fn read(&self) -> io::Result<RawFile> {
        let file = match std::fs::File::open(&self.path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(RawFile::Missing),
            Err(e) => return Err(e),
        };
        match crate::fsutil::read_capped(file, MAX_SETTINGS_BYTES) {
            Ok(text) => Ok(RawFile::Text(text)),
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                Ok(RawFile::Unusable(e.to_string()))
            }
            Err(e) => Err(e),
        }
    }

    fn write_atomic(&self, text: &str) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            // 셸 config 폴더는 아무도 미리 만들지 않는다 — 적재가 만들면 안 되므로 첫 쓰기가 만든다.
            std::fs::create_dir_all(dir)?;
        }
        crate::fsutil::write_atomic(&self.path, text)
    }

    fn copy_aside(&self) -> io::Result<PathBuf> {
        crate::fsutil::copy_aside(&self.path)
    }

    fn origin(&self) -> String {
        self.path.display().to_string()
    }
}

/// 원문을 문서로 본 결과.
#[derive(Debug)]
pub(super) enum Document {
    Missing,
    Usable(Map<String, Value>),
    /// 통째로 못 쓴다 — 사유 문구.
    Unusable(String),
}

impl Document {
    /// 파일의 `key` 항목이 목표와 다른가 — 목표 `None` = 그 키가 없어야 한다(기본값) · `Some(v)` = 정규 문자열
    /// `v` 여야 한다. 철자만 다른 값(`"LIGHT"`)도 다르다. `None` = 가를 수 없다(통째로 못 쓰는 문서).
    pub(super) fn differs(&self, key: &str, target: Option<&str>) -> Option<bool> {
        match self {
            Document::Missing => Some(target.is_some()),
            Document::Usable(map) => Some(match (map.get(key), target) {
                (None, None) => false,
                (Some(Value::String(found)), Some(target)) => found != target,
                _ => true,
            }),
            Document::Unusable(_) => None,
        }
    }
}

/// 파일을 한 번 읽어 문서로 — `Err` = 읽기 자체의 IO 실패.
pub(super) fn read_document(files: &dyn SettingsFiles) -> io::Result<Document> {
    let text = match files.read()? {
        RawFile::Missing => return Ok(Document::Missing),
        RawFile::Unusable(reason) => return Ok(Document::Unusable(reason)),
        RawFile::Text(text) => text,
    };
    // 메모장 · PowerShell 이 UTF-8 로 저장하면 BOM 을 붙인다 — 손으로 고친 파일이 그것만으로 못 쓰게 되지 않게.
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let map = match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(map)) => map,
        Ok(_) => return Ok(Document::Unusable("JSON 객체가 아니다".to_string())),
        Err(e) => return Ok(Document::Unusable(format!("JSON 이 아니다: {e}"))),
    };
    // 없으면 1 로 본다(손으로 쓴 파일) · 다른 값이면 모르는 형식이라 통째로 못 쓴다 — 고쳐 쓰면 그 형식을
    // 1 로 낮춰 덮게 된다. 떠 둔 사본에는 남는다. `1.0` 은 1 이다(JSON 도구가 수를 실수로 다시 쓰기도 한다).
    match map.get(VERSION_KEY) {
        None => {}
        Some(v) if v.as_f64() == Some(VERSION as f64) => {}
        Some(_) => {
            return Ok(Document::Unusable(format!(
                "모르는 `{VERSION_KEY}` — 이 셸은 {VERSION} 만 안다"
            )))
        }
    }
    Ok(Document::Usable(map))
}

/// 기본값과 다른 값(정규형)만 — 키 = 표의 키.
pub(super) type Overrides = BTreeMap<&'static str, String>;

/// 적재가 남길 로그 한 줄 — [`LoadNote::emit`] 이 적재 때 냈을 수준 · 필드 그대로 낸다. ★값은 싣지 않는다★ —
/// 밖에서 쓰는 파일이라 무엇이 들었는지 우리가 정하지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum LoadNote {
    Missing,
    /// 통째로 못 쓰는 파일 — 사유 문구.
    Unusable(String),
    /// 읽기 자체의 IO 실패 — 오류 문구.
    ReadFailed(String),
    /// 아는 키의 못 쓸 값 — 사유 문구([`registry::SettingDef::normalize`] 의 `Err` 는 값을 싣지 않는다).
    BadValue {
        key: &'static str,
        reason: String,
    },
    /// 다 읽었다 — 덮어쓴 키 수 · 모르는 키 수(모르는 키는 이름도 싣지 않는다).
    Loaded {
        overrides: usize,
        unknown: usize,
    },
}

impl LoadNote {
    /// `origin` = [`SettingsFiles::origin`].
    pub(super) fn emit(&self, origin: &str) {
        match self {
            LoadNote::Missing => tracing::debug!(
                module = "settings",
                source = %origin,
                "설정 파일이 없어 기본값으로 둔다"
            ),
            LoadNote::Unusable(reason) => tracing::error!(
                module = "settings",
                source = %origin,
                "설정 파일을 통째로 못 써 기본값으로 둔다(첫 쓰기가 옆에 떠 둔 뒤 새로 쓴다): {reason}"
            ),
            LoadNote::ReadFailed(error) => tracing::warn!(
                module = "settings",
                source = %origin,
                "설정 파일을 못 읽어 기본값으로 둔다: {error}"
            ),
            LoadNote::BadValue { key, reason } => tracing::warn!(
                module = "settings",
                source = %origin,
                key = *key,
                "설정 값을 못 써 기본값으로 둔다(파일은 덮일 때까지 그대로): {reason}"
            ),
            LoadNote::Loaded { overrides, unknown } => tracing::debug!(
                module = "settings",
                source = %origin,
                overrides = *overrides,
                unknown = *unknown,
                "설정을 읽었다"
            ),
        }
    }
}

/// 파일 → 기본값과 다른 값 + 남길 로그. ★실패는 전부 기본값으로 접는다★(패닉 · 전파 없음) · 파일을 건드리지
/// 않는다 · 로그를 내지 않는다(모듈 헤더).
pub(super) fn load(files: &dyn SettingsFiles) -> (Overrides, Vec<LoadNote>) {
    let map = match read_document(files) {
        Ok(Document::Usable(map)) => map,
        Ok(Document::Missing) => return (Overrides::new(), vec![LoadNote::Missing]),
        Ok(Document::Unusable(reason)) => {
            return (Overrides::new(), vec![LoadNote::Unusable(reason)])
        }
        Err(e) => return (Overrides::new(), vec![LoadNote::ReadFailed(e.to_string())]),
    };

    let mut overrides = Overrides::new();
    let mut notes = Vec::new();
    for def in SETTINGS {
        let Some(raw) = map.get(def.key) else {
            continue;
        };
        let normalized = match raw.as_str() {
            Some(text) => def.normalize(text),
            None => Err("문자열이 아니다".to_string()),
        };
        match normalized {
            Ok(value) if value == def.default => {}
            Ok(value) => {
                overrides.insert(def.key, value);
            }
            Err(reason) => notes.push(LoadNote::BadValue {
                key: def.key,
                reason,
            }),
        }
    }
    let unknown = map
        .keys()
        .filter(|key| key.as_str() != VERSION_KEY && registry::find(key).is_none())
        .count();
    notes.push(LoadNote::Loaded {
        overrides: overrides.len(),
        unknown,
    });
    (overrides, notes)
}

/// 받은 키만 바꿔 다시 쓴다 — `Some` = 그 값을 적는다 · `None` = 그 키를 지운다(기본값).
///
/// `document` = 이 쓰기 직전에 [`read_document`] 로 읽은 것. 파일이 없거나 통째로 못 쓰면 새 파일을 `memory`
/// (지금 유효한 덮어쓰기 전부)에서 짓는다 — 안 그러면 쓰기 한 번이 화면에 아직 보이는 다른 값을 디스크에서
/// 지운다. 통째로 못 쓰는 파일은 갈아끼우기 전에 옆에 떠 둔다([`SettingsFiles::copy_aside`]).
///
/// `Err` 면 파일은 그대로다. ★떠 두기 뒤의 실패면 앞서 떠 둔 사본은 이미 이 원본으로 덮였다★(사본 이름이
/// 하나뿐). 지은 원문이 읽기 상한을 넘으면 떠 두지도 쓰지도 않고 `Err`.
/// ★한 호출 = 한 파일(TRD §5-2)★ — 키가 여러 파일로 갈리게 되면 여기서 걸치는 묶음을 거절한다.
// ADR-0265
pub(super) fn write(
    files: &dyn SettingsFiles,
    document: Document,
    memory: &Overrides,
    changes: &[(&'static str, Option<String>)],
) -> io::Result<()> {
    let from_memory = || -> Map<String, Value> {
        memory
            .iter()
            .map(|(key, value)| ((*key).to_string(), Value::String(value.clone())))
            .collect()
    };
    let (mut map, unusable) = match document {
        Document::Usable(map) => (map, None),
        Document::Missing => (from_memory(), None),
        Document::Unusable(reason) => (from_memory(), Some(reason)),
    };
    for (key, value) in changes {
        match value {
            Some(value) => map.insert((*key).to_string(), Value::String(value.clone())),
            None => map.remove(*key),
        };
    }
    map.insert(VERSION_KEY.to_string(), Value::from(VERSION));
    let mut text = serde_json::to_string_pretty(&Value::Object(map))
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    // 사람이 손으로도 고치는 파일이라 줄 끝을 남긴다.
    text.push('\n');
    if text.len() as u64 > MAX_SETTINGS_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "쓸 원문({} 바이트)이 읽기 상한 {MAX_SETTINGS_BYTES} 바이트를 넘는다",
                text.len()
            ),
        ));
    }
    if let Some(reason) = unusable {
        let to = files.copy_aside()?;
        tracing::warn!(
            module = "settings",
            source = %files.origin(),
            copied_to = %to.display(),
            "못 쓰는 설정 파일을 옆에 떠 두고(앞선 사본이 있었으면 덮었다) 그 자리에 새로 쓴다: {reason}"
        );
    }
    files.write_atomic(&text)
}

/// 메모리 가짜 디스크 — 시험이 손잡이를 쥔 채 서비스에 넘긴다.
#[cfg(test)]
pub(super) mod fake {
    use std::io;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex, MutexGuard};

    use super::{RawFile, SettingsFiles};

    #[derive(Default)]
    pub struct Disk {
        /// 지금 파일 원문. `None` = 없음.
        pub text: Option<String>,
        /// 원문이 못 된다(상한 초과 흉내) — `text` 보다 먼저 본다.
        pub unusable: bool,
        pub fail_read: bool,
        pub fail_write: bool,
        pub fail_copy: bool,
        pub writes: usize,
        /// 떠 둔 사본 자리(`memory.corrupt` — 하나뿐이라 뜰 때마다 덮인다)의 원문. `None` = 사본 없음.
        pub corrupt: Option<String>,
        /// 떠 둔 횟수.
        pub copies: usize,
    }

    #[derive(Clone, Default)]
    pub struct MemFiles(Arc<Mutex<Disk>>);

    impl MemFiles {
        pub fn with_text(text: &str) -> Self {
            let files = Self::default();
            files.disk().text = Some(text.to_string());
            files
        }

        pub fn disk(&self) -> MutexGuard<'_, Disk> {
            self.0.lock().unwrap()
        }

        pub fn text(&self) -> Option<String> {
            self.disk().text.clone()
        }

        pub fn json(&self) -> serde_json::Value {
            serde_json::from_str(&self.text().expect("파일이 있어야 한다")).expect("JSON")
        }
    }

    impl SettingsFiles for MemFiles {
        fn read(&self) -> io::Result<RawFile> {
            let disk = self.disk();
            if disk.fail_read {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "가짜 읽기 실패",
                ));
            }
            if disk.unusable {
                return Ok(RawFile::Unusable("가짜 상한 초과".to_string()));
            }
            Ok(match &disk.text {
                Some(text) => RawFile::Text(text.clone()),
                None => RawFile::Missing,
            })
        }

        fn write_atomic(&self, text: &str) -> io::Result<()> {
            let mut disk = self.disk();
            if disk.fail_write {
                return Err(io::Error::other("가짜 쓰기 실패"));
            }
            disk.text = Some(text.to_string());
            disk.unusable = false;
            disk.writes += 1;
            Ok(())
        }

        fn copy_aside(&self) -> io::Result<PathBuf> {
            let mut disk = self.disk();
            if disk.fail_copy {
                return Err(io::Error::other("가짜 사본 실패"));
            }
            let Some(text) = disk.text.clone() else {
                return Err(io::Error::from(io::ErrorKind::NotFound));
            };
            disk.corrupt = Some(text);
            disk.copies += 1;
            Ok(PathBuf::from("memory.corrupt"))
        }

        fn origin(&self) -> String {
            "memory".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::MemFiles;
    use super::*;

    fn overrides_of(files: &MemFiles) -> Overrides {
        load(files).0
    }

    fn write_doc(files: &MemFiles, changes: &[(&'static str, Option<String>)]) -> io::Result<()> {
        let document = read_document(files)?;
        write(files, document, &Overrides::new(), changes)
    }

    // ── 적재 ──

    #[test]
    fn a_missing_file_loads_as_defaults() {
        let files = MemFiles::default();
        let (overrides, notes) = load(&files);
        assert!(overrides.is_empty());
        assert_eq!(notes, vec![LoadNote::Missing]);
        assert_eq!(files.disk().writes, 0);
    }

    #[test]
    fn values_are_normalized_and_defaults_dropped_on_load() {
        let files = MemFiles::with_text(
            r#"{"$version":1,"theme.default":"LIGHT","chat.style.fontSize":"13px","chat.style.userPy":" 9PX "}"#,
        );
        let loaded = overrides_of(&files);
        assert_eq!(
            loaded.get("theme.default").map(String::as_str),
            Some("light")
        );
        assert_eq!(
            loaded.get("chat.style.userPy").map(String::as_str),
            Some("9px")
        );
        assert!(
            !loaded.contains_key("chat.style.fontSize"),
            "기본값과 같은 값은 덮어쓰기가 아니다"
        );
    }

    #[test]
    fn a_bad_known_value_folds_to_default_and_stays_in_the_file() {
        let original =
            r#"{"theme.default":"purple","chat.style.fontSize":99,"chat.style.userPy":"9px"}"#;
        let files = MemFiles::with_text(original);
        let (loaded, notes) = load(&files);
        assert!(!loaded.contains_key("theme.default"));
        assert!(!loaded.contains_key("chat.style.fontSize"));
        assert_eq!(
            loaded.get("chat.style.userPy").map(String::as_str),
            Some("9px")
        );
        assert_eq!(
            files.text().as_deref(),
            Some(original),
            "적재는 파일을 안 고친다"
        );
        let bad: Vec<&str> = notes
            .iter()
            .filter_map(|note| match note {
                LoadNote::BadValue { key, .. } => Some(*key),
                _ => None,
            })
            .collect();
        assert_eq!(bad, vec!["theme.default", "chat.style.fontSize"]);
        assert!(
            !format!("{notes:?}").contains("purple"),
            "값은 싣지 않는다: {notes:?}"
        );
    }

    #[test]
    fn unknown_keys_are_ignored_on_load_and_only_counted() {
        let files = MemFiles::with_text(r#"{"future.key":"x","theme.default":"light"}"#);
        let (loaded, notes) = load(&files);
        assert_eq!(loaded.len(), 1);
        assert_eq!(
            notes,
            vec![LoadNote::Loaded {
                overrides: 1,
                unknown: 1
            }]
        );
    }

    #[test]
    fn a_wholly_unusable_file_loads_as_defaults_and_is_left_alone() {
        for text in [
            "{not json",
            "[1,2]",
            "\"light\"",
            r#"{"$version":2,"theme.default":"light"}"#,
            r#"{"$version":"1","theme.default":"light"}"#,
            r#"{"$version":1.5,"theme.default":"light"}"#,
        ] {
            let files = MemFiles::with_text(text);
            let (overrides, notes) = load(&files);
            assert!(overrides.is_empty(), "{text}");
            assert!(
                matches!(notes.as_slice(), [LoadNote::Unusable(_)]),
                "{text}: {notes:?}"
            );
            let disk = files.disk();
            assert_eq!(disk.text.as_deref(), Some(text), "{text}");
            assert_eq!(disk.writes, 0);
            assert_eq!(disk.copies, 0, "적재는 떠 두지 않는다: {text}");
        }
        let files = MemFiles::default();
        files.disk().unusable = true;
        assert!(overrides_of(&files).is_empty());
        assert_eq!(files.disk().copies, 0);
    }

    #[test]
    fn a_read_failure_loads_as_defaults() {
        let files = MemFiles::with_text(r#"{"theme.default":"light"}"#);
        files.disk().fail_read = true;
        let (overrides, notes) = load(&files);
        assert!(overrides.is_empty());
        assert!(
            matches!(notes.as_slice(), [LoadNote::ReadFailed(_)]),
            "{notes:?}"
        );
    }

    #[test]
    fn a_leading_bom_is_not_a_reason_to_reject_the_file() {
        let files = MemFiles::with_text("\u{feff}{\"theme.default\":\"light\"}");
        assert_eq!(
            overrides_of(&files)
                .get("theme.default")
                .map(String::as_str),
            Some("light")
        );
    }

    #[test]
    fn version_one_written_as_a_float_is_version_one() {
        let files = MemFiles::with_text(r#"{"$version":1.0,"theme.default":"light"}"#);
        assert_eq!(
            overrides_of(&files)
                .get("theme.default")
                .map(String::as_str),
            Some("light")
        );
    }

    // ── 문서 대조 ──

    #[test]
    fn differs_compares_the_raw_entry_with_the_target() {
        let usable = |text: &str| match read_document(&MemFiles::with_text(text)).unwrap() {
            doc @ Document::Usable(_) => doc,
            other => panic!("{other:?}"),
        };
        let doc = usable(r#"{"theme.default":"light","chat.style.fontSize":"bad","n":1}"#);
        assert_eq!(doc.differs("theme.default", Some("light")), Some(false));
        assert_eq!(doc.differs("theme.default", Some("e-ink")), Some(true));
        assert_eq!(doc.differs("theme.default", None), Some(true));
        assert_eq!(doc.differs("chat.style.fontSize", None), Some(true));
        assert_eq!(doc.differs("chat.style.userPy", None), Some(false));
        assert_eq!(doc.differs("n", Some("1")), Some(true), "문자열이 아니다");
        assert_eq!(
            usable(r#"{"theme.default":"LIGHT"}"#).differs("theme.default", Some("light")),
            Some(true),
            "정규 철자가 아니면 다르다"
        );
        assert_eq!(
            Document::Missing.differs("theme.default", None),
            Some(false)
        );
        assert_eq!(
            Document::Missing.differs("theme.default", Some("light")),
            Some(true)
        );
        assert_eq!(
            Document::Unusable(String::new()).differs("theme.default", None),
            None
        );
    }

    // ── 쓰기 ──

    #[test]
    fn a_write_creates_the_file_with_the_version_key() {
        let files = MemFiles::default();
        write_doc(&files, &[("theme.default", Some("light".into()))]).unwrap();
        assert_eq!(
            files.json(),
            serde_json::json!({"$version": 1, "theme.default": "light"})
        );
        assert!(files.text().unwrap().ends_with('\n'));
    }

    #[test]
    fn a_write_changes_only_its_key_and_keeps_unknown_and_other_keys() {
        let files = MemFiles::with_text(
            r#"{"$version":1,"future.key":[1,{"a":2}],"chat.style.userPy":"BAD","theme.default":"light"}"#,
        );
        write_doc(&files, &[("chat.style.fontSize", Some("15px".into()))]).unwrap();
        assert_eq!(
            files.json(),
            serde_json::json!({
                "$version": 1,
                "future.key": [1, {"a": 2}],
                "chat.style.userPy": "BAD",
                "theme.default": "light",
                "chat.style.fontSize": "15px",
            })
        );
    }

    #[test]
    fn a_none_change_removes_the_key() {
        let files = MemFiles::with_text(r#"{"$version":1,"theme.default":"light","x":1}"#);
        write_doc(&files, &[("theme.default", None)]).unwrap();
        assert_eq!(files.json(), serde_json::json!({"$version": 1, "x": 1}));
    }

    #[test]
    fn a_usable_file_is_not_reseeded_from_memory() {
        let files = MemFiles::with_text(r#"{"$version":1}"#);
        let memory = Overrides::from([("chat.style.userPy", "9px".to_string())]);
        let document = read_document(&files).unwrap();
        write(
            &files,
            document,
            &memory,
            &[("theme.default", Some("light".into()))],
        )
        .unwrap();
        assert_eq!(
            files.json(),
            serde_json::json!({"$version": 1, "theme.default": "light"}),
            "멀쩡한 파일에서 밖이 지운 키를 되살리지 않는다"
        );
    }

    #[test]
    fn an_unusable_file_is_copied_aside_and_rebuilt_from_memory() {
        let files = MemFiles::with_text("{broken");
        let memory = Overrides::from([
            ("chat.style.userPy", "9px".to_string()),
            ("theme.default", "e-ink".to_string()),
        ]);
        let document = read_document(&files).unwrap();
        write(
            &files,
            document,
            &memory,
            &[("theme.default", Some("light".into()))],
        )
        .unwrap();
        assert_eq!(files.disk().corrupt.as_deref(), Some("{broken"));
        assert_eq!(
            files.json(),
            serde_json::json!({"$version": 1, "theme.default": "light", "chat.style.userPy": "9px"})
        );
    }

    #[test]
    fn a_missing_file_is_rebuilt_from_memory() {
        let files = MemFiles::default();
        let memory = Overrides::from([("chat.style.userPy", "9px".to_string())]);
        write(
            &files,
            Document::Missing,
            &memory,
            &[("theme.default", Some("light".into()))],
        )
        .unwrap();
        assert_eq!(
            files.json(),
            serde_json::json!({"$version": 1, "theme.default": "light", "chat.style.userPy": "9px"})
        );
        assert_eq!(files.disk().copies, 0, "없는 파일은 떠 둘 것이 없다");
    }

    #[test]
    fn a_failed_copy_fails_the_write_and_leaves_the_file() {
        let files = MemFiles::with_text("{broken");
        files.disk().fail_copy = true;
        write_doc(&files, &[("theme.default", Some("light".into()))]).unwrap_err();
        assert_eq!(files.text().as_deref(), Some("{broken"));
        assert_eq!(files.disk().writes, 0);
    }

    #[test]
    fn a_failed_replace_after_the_copy_leaves_the_original_in_place() {
        let files = MemFiles::with_text("{broken");
        files.disk().fail_write = true;
        write_doc(&files, &[("theme.default", Some("light".into()))]).unwrap_err();
        let disk = files.disk();
        assert_eq!(
            disk.text.as_deref(),
            Some("{broken"),
            "파일이 없는 순간이 없다"
        );
        assert_eq!(disk.corrupt.as_deref(), Some("{broken"));
    }

    #[test]
    fn every_write_over_a_still_broken_file_copies_it_onto_the_one_name() {
        let files = MemFiles::with_text("{broken");
        files.disk().fail_write = true;
        let attempt = || write_doc(&files, &[("theme.default", Some("light".into()))]);

        attempt().unwrap_err();
        attempt().unwrap_err();
        {
            let disk = files.disk();
            assert_eq!(disk.copies, 2);
            assert_eq!(disk.corrupt.as_deref(), Some("{broken"));
        }

        // 밖에서 다른 못 쓰는 원문으로 고쳤다 — 앞서 떠 둔 사본은 덮인다(TRD §10 F21).
        files.disk().text = Some("{BROKEN".to_string());
        attempt().unwrap_err();
        assert_eq!(files.disk().corrupt.as_deref(), Some("{BROKEN"));
    }

    #[test]
    fn a_text_over_the_read_cap_is_never_written() {
        let pad = "x".repeat(MAX_SETTINGS_BYTES as usize - 20);
        let original = format!("{{\"pad\":\"{pad}\"}}");
        assert!(original.len() as u64 <= MAX_SETTINGS_BYTES);
        let files = MemFiles::with_text(&original);
        let err = write_doc(&files, &[("theme.default", Some("light".into()))]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        let disk = files.disk();
        assert_eq!(disk.text.as_deref(), Some(original.as_str()));
        assert_eq!(disk.writes, 0);
    }
}
