//! 거절 기한 저장 — 상류가 조회를 거절한 기한을 데몬 재시작 너머로 들고 간다(TRD §3 #30).
//!
//! ★I/O 와 키 정규화뿐이다 — 시각으로 거르지 않는다★: 지난 항목 버림·기한 상한 절단·단조 시계 환산은 부르는
//!   쪽(서비스의 책) 몫이다. 값·쿨타임은 저장하지 않는다.
//! ★잠그지 않는다 — 저장을 한 줄로 세우는 것은 부르는 쪽이다★(서비스의 `save_lock`). 동시에 부르면 tmp 파일
//!   하나를 둘이 밟고, 마지막 쓰기가 다른 쪽 항목을 지운다.
//! ★실패하지 않는다★ — 파일 없음 = 빈 표 · 깨진 파일·모르는 판 = 빈 표 + warn · 쓰기 실패 = warn 후 계속.
//!   파일이 무엇을 담든 패닉하지 않는다(릴리즈는 `panic = "abort"` 라 기동 중 패닉이 매 부팅 중단이 된다).

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use engram_dashboard_agent::backend::usage_probe_for;
use engram_dashboard_agent::usage::{UsageAccountKey, UsageKey};
use serde::{Deserialize, Serialize};

const SCHEMA_VERSION: u32 = 1;
const FILE_NAME: &str = "usage_rejects.json";
const TMP_NAME: &str = "usage_rejects.json.tmp";

/// 칸 하나의 거절 기한. `until_epoch_s` = 거절이 풀리는 벽시계 시각, epoch 초.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectEntry {
    pub key: UsageKey,
    pub until_epoch_s: i64,
}

pub trait RejectStore: Send + Sync {
    /// 저장된 항목. 실패하지 않는다 — 못 읽으면 빈 표다.
    ///
    /// ★돌려주는 표의 보장★: 키마다 많아야 하나(같은 키가 여럿이었으면 더 늦은 기한) · 기본 계정뿐.
    ///   벤더 키의 철자는 구현이 정한다 — 파일 저장소는 조회기의 정본 철자로 맞추고, 메모리 대역은 받은 키 그대로다.
    fn load(&self) -> Vec<RejectEntry>;
    /// 받은 항목으로 전량 교체한다. 실패하지 않는다 — 쓰기 실패 = warn 후 계속.
    fn save(&self, entries: &[RejectEntry]);
}

#[derive(Serialize, Deserialize)]
struct RejectFile {
    schema_version: u32,
    entries: Vec<RawEntry>,
}

/// 판만 먼저 읽는다 — 판이 다르면 `entries` 모양을 믿을 수 없다.
#[derive(Deserialize)]
struct RejectFileHeader {
    schema_version: u32,
}

#[derive(Serialize, Deserialize)]
struct RawEntry {
    /// 쓸 때는 조회기 키의 정본 철자다. 읽을 때는 손으로 고친 철자일 수 있다([`normalize`]).
    vendor: String,
    account_key: String,
    reject_until_epoch_s: i64,
}

/// 실물 — 데몬 데이터 폴더의 `usage_rejects.json`. 쓰기 = tmp + rename(`FilePresetStore` 선례)이라 크래시가
/// 나도 파일은 완전한 옛/새 내용 둘 중 하나다.
pub struct FileRejectStore {
    dir: PathBuf,
}

impl FileRejectStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            dir: data_dir.to_path_buf(),
        }
    }

    fn path(&self) -> PathBuf {
        self.dir.join(FILE_NAME)
    }

    fn read(&self) -> Vec<RawEntry> {
        let path = self.path();
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Vec::new(),
            Err(e) => {
                tracing::warn!(?path, error = %e, "거절 기한 파일을 못 읽었다 — 빈 표로 시작한다");
                return Vec::new();
            }
        };
        match serde_json::from_slice::<RejectFileHeader>(&bytes) {
            Ok(header) if header.schema_version == SCHEMA_VERSION => {}
            Ok(header) => {
                tracing::warn!(
                    ?path,
                    schema_version = header.schema_version,
                    "거절 기한 파일의 판을 모른다 — 빈 표로 시작한다"
                );
                return Vec::new();
            }
            Err(e) => {
                tracing::warn!(?path, error = %e, "거절 기한 파일이 깨졌다 — 빈 표로 시작한다");
                return Vec::new();
            }
        }
        match serde_json::from_slice::<RejectFile>(&bytes) {
            Ok(file) => file.entries,
            Err(e) => {
                tracing::warn!(?path, error = %e, "거절 기한 파일이 깨졌다 — 빈 표로 시작한다");
                Vec::new()
            }
        }
    }

    fn write_atomic(&self, entries: &[RejectEntry]) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;

        let payload = RejectFile {
            schema_version: SCHEMA_VERSION,
            entries: entries
                .iter()
                .map(|entry| RawEntry {
                    vendor: entry.key.vendor.as_str().to_owned(),
                    account_key: entry.key.account.as_str().to_owned(),
                    reject_until_epoch_s: entry.until_epoch_s,
                })
                .collect(),
        };
        let json = serde_json::to_vec_pretty(&payload)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let tmp = self.dir.join(TMP_NAME);
        {
            let mut f = File::create(&tmp)?;
            f.write_all(&json)?;
            f.sync_all()?;
        }

        fs::rename(&tmp, self.path())?;

        // 폴더 fsync 는 best-effort — 파일 내용은 위에서 이미 sync 됐다. Windows 에서는 폴더를 파일로 열 수
        // 없어 늘 여기서 실패한다(no-op).
        if let Err(e) = File::open(&self.dir).and_then(|dir| dir.sync_all()) {
            tracing::debug!(dir = ?self.dir, error = %e, "거절 기한 폴더 fsync 건너뜀");
        }
        Ok(())
    }
}

impl RejectStore for FileRejectStore {
    fn load(&self) -> Vec<RejectEntry> {
        let normalized = normalize(self.read());
        // 낱말 자체는 찍지 않는다 — 파일 내용이라 길이 제한이 없다.
        if normalized.unknown_vendor > 0 {
            tracing::warn!(
                dropped = normalized.unknown_vendor,
                "거절 기한 파일에 모르는 벤더 항목이 있다 — 버린다"
            );
        }
        if normalized.foreign_account > 0 {
            tracing::warn!(
                dropped = normalized.foreign_account,
                "거절 기한 파일에 기본이 아닌 계정 항목이 있다 — 버린다"
            );
        }
        tracing::debug!(
            kept = normalized.entries.len(),
            dropped_unknown_vendor = normalized.unknown_vendor,
            dropped_foreign_account = normalized.foreign_account,
            "거절 기한 읽음"
        );
        normalized.entries
    }

    fn save(&self, entries: &[RejectEntry]) {
        match self.write_atomic(entries) {
            Ok(()) => tracing::debug!(entries = entries.len(), "거절 기한 저장"),
            Err(e) => {
                tracing::warn!(path = ?self.path(), error = %e, "거절 기한 저장 실패 — 이번 기한은 재시작을 못 건넌다")
            }
        }
    }
}

struct Normalized {
    entries: Vec<RejectEntry>,
    unknown_vendor: usize,
    foreign_account: usize,
}

/// 파일 낱말 → 조회기 키. 손으로 고친 대소문자도 같은 칸으로 접힌다 — 키는 들어온 철자가 아니라 조회기의
/// `key()` 로 만든다. 모르는 벤더·기본이 아닌 계정은 버린다.
fn normalize(raw: Vec<RawEntry>) -> Normalized {
    let account = UsageAccountKey::default();
    let mut keyed = Vec::with_capacity(raw.len());
    let mut unknown_vendor = 0usize;
    let mut foreign_account = 0usize;
    for entry in raw {
        let Some(probe) = usage_probe_for(&entry.vendor) else {
            unknown_vendor += 1;
            continue;
        };
        if entry.account_key != account.as_str() {
            foreign_account += 1;
            continue;
        }
        keyed.push(RejectEntry {
            key: UsageKey {
                vendor: probe.key(),
                account: account.clone(),
            },
            until_epoch_s: entry.reject_until_epoch_s,
        });
    }
    Normalized {
        entries: keep_latest_per_key(keyed),
        unknown_vendor,
        foreign_account,
    }
}

/// 같은 키가 여럿이면 더 늦은 기한 하나만 남긴다. 순서 = 각 키가 처음 나온 자리.
fn keep_latest_per_key(entries: impl IntoIterator<Item = RejectEntry>) -> Vec<RejectEntry> {
    let mut out: Vec<RejectEntry> = Vec::new();
    for entry in entries {
        match out.iter_mut().find(|kept| kept.key == entry.key) {
            Some(kept) => kept.until_epoch_s = kept.until_epoch_s.max(entry.until_epoch_s),
            None => out.push(entry),
        }
    }
    out
}

/// 메모리 대역 — 파일 없이 조립할 때 쓴다. `load` 는 심어 둔 값 또는 마지막 `save` 값을 돌려준다.
///
/// `load` 보장 중 여기서 세우는 것은 키마다 하나뿐이다 — 기본 계정은 타입이 막고(`UsageAccountKey` 는 기본값만
/// 만들 수 있다), 벤더 키는 정규화하지 않는다: 시험 대역의 리터럴 `UsageVendorKey::new` 가 그대로 살아야 한다.
pub struct MemRejectStore {
    entries: Mutex<Vec<RejectEntry>>,
    last_saved: Mutex<Vec<RejectEntry>>,
}

impl MemRejectStore {
    pub fn new() -> Self {
        Self::with(Vec::new())
    }

    /// `load` 가 처음 돌려줄 항목을 심는다 — 같은 키가 여럿이면 더 늦은 기한 하나로 접는다.
    pub fn with(entries: Vec<RejectEntry>) -> Self {
        Self {
            entries: Mutex::new(keep_latest_per_key(entries)),
            last_saved: Mutex::new(Vec::new()),
        }
    }

    /// 마지막 `save` 가 받은 항목 그대로(접지 않는다). 한 번도 안 불렸으면 빈 표다(심어 둔 값이 아니다).
    pub fn saved(&self) -> Vec<RejectEntry> {
        lock(&self.last_saved).clone()
    }
}

impl Default for MemRejectStore {
    fn default() -> Self {
        Self::new()
    }
}

impl RejectStore for MemRejectStore {
    fn load(&self) -> Vec<RejectEntry> {
        lock(&self.entries).clone()
    }

    fn save(&self, entries: &[RejectEntry]) {
        *lock(&self.entries) = keep_latest_per_key(entries.iter().cloned());
        *lock(&self.last_saved) = entries.to_vec();
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log_capture::capture_loud;
    use engram_dashboard_agent::backend::usage_probes;

    /// 시험마다 자기 폴더 — drop 에 지운다(단언이 실패해도).
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let dir =
                std::env::temp_dir().join(format!("engram-usage-rejects-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&dir).expect("시험 폴더");
            Self(dir)
        }

        fn file(&self) -> PathBuf {
            self.0.join(FILE_NAME)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// 조회기 키로만 칸을 만든다 — 시험에도 벤더 리터럴을 두지 않는다.
    fn entry(index: usize, until_epoch_s: i64) -> RejectEntry {
        RejectEntry {
            key: UsageKey {
                vendor: usage_probes()[index].key(),
                account: UsageAccountKey::default(),
            },
            until_epoch_s,
        }
    }

    fn raw_file(entries: serde_json::Value) -> String {
        serde_json::json!({ "schema_version": 1, "entries": entries }).to_string()
    }

    fn raw_entry(vendor: &str, account_key: &str, until: i64) -> serde_json::Value {
        serde_json::json!({
            "vendor": vendor, "account_key": account_key, "reject_until_epoch_s": until
        })
    }

    #[test]
    fn file_round_trip_keeps_every_vendor() {
        let dir = TempDir::new();
        let written: Vec<RejectEntry> = (0..usage_probes().len())
            .map(|i| entry(i, 1_900_000_000 + i as i64))
            .collect();
        assert!(
            written.len() >= 2,
            "조회기가 둘 이상이어야 이 시험이 뜻이 있다"
        );

        FileRejectStore::new(&dir.0).save(&written);

        assert_eq!(FileRejectStore::new(&dir.0).load(), written);
        assert!(!dir.0.join(TMP_NAME).exists(), "tmp 가 남았다");
    }

    #[test]
    fn saved_file_uses_canonical_words_and_the_pinned_shape() {
        let dir = TempDir::new();
        let written = entry(0, -3);
        FileRejectStore::new(&dir.0).save(std::slice::from_ref(&written));

        let on_disk: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.file()).expect("파일")).expect("JSON");
        assert_eq!(
            on_disk,
            serde_json::json!({
                "schema_version": 1,
                "entries": [{
                    "vendor": written.key.vendor.as_str(),
                    "account_key": UsageAccountKey::default().as_str(),
                    "reject_until_epoch_s": -3
                }]
            })
        );
    }

    #[test]
    fn save_replaces_the_whole_table() {
        let dir = TempDir::new();
        let store = FileRejectStore::new(&dir.0);
        store.save(&[entry(0, 10), entry(1, 20)]);
        store.save(&[entry(1, 30)]);
        assert_eq!(store.load(), vec![entry(1, 30)]);
        store.save(&[]);
        assert_eq!(store.load(), Vec::new());
    }

    #[test]
    fn missing_file_and_missing_dir_load_empty_and_save_creates_the_dir() {
        let dir = TempDir::new();
        let nested = dir.0.join("아직").join("없다");
        let store = FileRejectStore::new(&nested);
        let (loaded, loud) = capture_loud(|| store.load());
        assert_eq!(loaded, Vec::new());
        assert!(loud.is_empty(), "파일 없음은 정상이다: {loud:?}");

        store.save(&[entry(0, 5)]);
        assert_eq!(store.load(), vec![entry(0, 5)]);
    }

    #[test]
    fn corrupt_or_foreign_files_load_empty_with_a_warning() {
        let cases: [&[u8]; 10] = [
            b"",
            b"\xff\xfe\x00garbage",
            b"not json",
            b"null",
            b"[]",
            br#"{"schema_version": 1}"#,
            br#"{"schema_version": 1, "entries": {}}"#,
            br#"{"schema_version": 1, "entries": [{"vendor": 3}]}"#,
            // i64 를 넘는 기한 — 파싱 실패로 떨어져야지 감겨서 과거가 되면 안 된다.
            br#"{"schema_version": 1, "entries": [{"vendor": "x", "account_key": "default", "reject_until_epoch_s": 18446744073709551615}]}"#,
            br#"{"schema_version": 2, "entries": []}"#,
        ];
        for raw in cases {
            let dir = TempDir::new();
            fs::write(dir.file(), raw).expect("씨앗 파일");
            let (loaded, loud) = capture_loud(|| FileRejectStore::new(&dir.0).load());
            let shown = String::from_utf8_lossy(raw);
            assert_eq!(loaded, Vec::new(), "{shown}");
            assert_eq!(loud.len(), 1, "{shown}: {loud:?}");
        }
    }

    #[test]
    fn unreadable_path_loads_empty_with_a_warning() {
        let dir = TempDir::new();
        // 파일 자리에 폴더 — 읽기가 NotFound 가 아닌 오류로 실패한다.
        fs::create_dir_all(dir.file()).expect("폴더");
        let (loaded, loud) = capture_loud(|| FileRejectStore::new(&dir.0).load());
        assert_eq!(loaded, Vec::new());
        assert_eq!(loud.len(), 1, "{loud:?}");
    }

    #[test]
    fn write_failure_warns_and_returns() {
        let dir = TempDir::new();
        // 데이터 폴더 자리에 파일 — create_dir_all 이 실패한다.
        let blocker = dir.0.join("blocker");
        fs::write(&blocker, b"x").expect("막는 파일");
        let store = FileRejectStore::new(&blocker);
        let ((), loud) = capture_loud(|| store.save(&[entry(0, 1)]));
        assert_eq!(loud.len(), 1, "{loud:?}");
        assert_eq!(fs::read(&blocker).expect("막는 파일"), b"x");
    }

    #[test]
    fn load_normalizes_words_and_drops_what_it_cannot_key() {
        let dir = TempDir::new();
        let canonical = usage_probes()[0].key().as_str();
        let default = UsageAccountKey::default();
        let title_case = {
            let mut chars = canonical.chars();
            chars
                .next()
                .map(|c| c.to_ascii_uppercase().to_string() + chars.as_str())
                .unwrap_or_default()
        };
        let other = usage_probes()[1].key().as_str().to_ascii_uppercase();
        fs::write(
            dir.file(),
            raw_file(serde_json::json!([
                raw_entry(canonical, default.as_str(), 100),
                raw_entry(&title_case, default.as_str(), 300),
                raw_entry(&canonical.to_ascii_uppercase(), default.as_str(), 200),
                raw_entry("no-such-vendor", default.as_str(), 999),
                raw_entry(canonical, "someone-else", 999),
                raw_entry(&other, default.as_str(), 50),
            ])),
        )
        .expect("씨앗 파일");

        let (loaded, loud) = capture_loud(|| FileRejectStore::new(&dir.0).load());

        // 대소문자만 다른 셋은 한 칸 — 가장 늦은 기한이 남는다. 키 철자는 조회기의 정본이다.
        assert_eq!(loaded, vec![entry(0, 300), entry(1, 50)]);
        assert_eq!(loaded[0].key.vendor.as_str(), canonical);
        assert_eq!(loud.len(), 2, "모르는 벤더·남의 계정 한 번씩: {loud:?}");
    }

    #[test]
    fn duplicate_keys_keep_the_later_deadline_regardless_of_order() {
        let dir = TempDir::new();
        let word = usage_probes()[1].key().as_str();
        let default = UsageAccountKey::default();
        fs::write(
            dir.file(),
            raw_file(serde_json::json!([
                raw_entry(word, default.as_str(), 700),
                raw_entry(word, default.as_str(), i64::MIN),
                raw_entry(word, default.as_str(), 400),
            ])),
        )
        .expect("씨앗 파일");
        assert_eq!(FileRejectStore::new(&dir.0).load(), vec![entry(1, 700)]);
    }

    #[test]
    fn mem_store_round_trips_and_reports_the_last_save() {
        let store = MemRejectStore::with(vec![entry(0, 1)]);
        assert_eq!(store.load(), vec![entry(0, 1)]);
        assert_eq!(store.saved(), Vec::new(), "심은 값은 저장이 아니다");

        store.save(&[entry(0, 2), entry(1, 3)]);
        assert_eq!(store.load(), vec![entry(0, 2), entry(1, 3)]);
        assert_eq!(store.saved(), vec![entry(0, 2), entry(1, 3)]);

        // 같은 키가 겹치면 `load` 는 파일 실물과 같은 보장을 선다 — `saved` 는 받은 그대로다.
        let seeded =
            MemRejectStore::with(vec![entry(1, 9), entry(0, 4), entry(1, 12), entry(1, 3)]);
        assert_eq!(seeded.load(), vec![entry(1, 12), entry(0, 4)]);
        let overlapping = [entry(0, 8), entry(0, 5)];
        seeded.save(&overlapping);
        assert_eq!(seeded.load(), vec![entry(0, 8)]);
        assert_eq!(seeded.saved(), overlapping.to_vec());

        let empty = MemRejectStore::new();
        assert_eq!(empty.load(), Vec::new());
        assert_eq!(MemRejectStore::default().saved(), Vec::new());
    }
}
