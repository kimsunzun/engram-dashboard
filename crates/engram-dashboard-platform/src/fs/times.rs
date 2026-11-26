//! Setting a folder's modified time — a copy keeps it (ADR-0409 M13). std sets a file's times through
//! `File::set_times`, but on Windows it cannot open a folder to do so: that open needs `FILE_FLAG_BACKUP_SEMANTICS`
//! (std's own open answers access denied, measured 2026-11-24).

// ADR-0409

use std::fs::{File, FileTimes};
use std::io;
use std::path::Path;
use std::time::SystemTime;

/// Sets the modified time of the folder `dir` (the access time is left as is). The folder is opened by its path, a link
/// among its names followed — the caller passes a folder it made.
///
/// - Windows: opened for writing attributes only, with `FILE_FLAG_BACKUP_SEMANTICS` (a folder needs it); a read-only
///   folder takes it too.
/// - Other OSes: opened for reading (a folder cannot be opened for writing); setting times needs the owner or write
///   permission, as `futimens` does.
#[cfg(windows)]
pub fn set_folder_modified(dir: &Path, modified: SystemTime) -> io::Result<()> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    File::options()
        .access_mode(FILE_WRITE_ATTRIBUTES)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(dir)?
        .set_times(FileTimes::new().set_modified(modified))
}
#[cfg(not(windows))]
pub fn set_folder_modified(dir: &Path, modified: SystemTime) -> io::Result<()> {
    File::open(dir)?.set_times(FileTimes::new().set_modified(modified))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::super::Scratch;
    use super::*;

    #[test]
    fn a_folders_modified_time_is_set_also_when_it_is_read_only() {
        let scratch = Scratch::new("folder-times");
        let dir = scratch.0.join("d");
        std::fs::create_dir(&dir).unwrap();
        let then = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);

        set_folder_modified(&dir, then).unwrap();
        assert_eq!(std::fs::metadata(&dir).unwrap().modified().unwrap(), then);

        let mut read_only = std::fs::metadata(&dir).unwrap().permissions();
        read_only.set_readonly(true);
        std::fs::set_permissions(&dir, read_only.clone()).unwrap();
        let later = then + Duration::from_secs(60);
        let set = set_folder_modified(&dir, later);
        #[allow(clippy::permissions_set_readonly_false)]
        read_only.set_readonly(false);
        std::fs::set_permissions(&dir, read_only).unwrap();
        set.unwrap();
        assert_eq!(std::fs::metadata(&dir).unwrap().modified().unwrap(), later);
    }

    #[test]
    fn a_missing_folder_is_not_found() {
        let scratch = Scratch::new("folder-times-missing");
        let then = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        let missing = set_folder_modified(&scratch.0.join("gone"), then).unwrap_err();
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
    }
}
