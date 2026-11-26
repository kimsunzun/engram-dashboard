//! File identity — a value naming a file or folder within one volume regardless of its name (a rename keeps it).

// ADR-0325

use std::io;
use std::path::Path;

/// [`file_identity`]'s answer — only for comparison and as a hash key.
///
/// - Windows = volume serial number + file id. Asks the 128-bit `FileIdInfo` first; if that query fails or gives a "no unique id"
///   value, falls back to the 32-bit volume serial number + 64-bit file index. ★The two forms differ even for the same file★ — we
///   assume one volume yields only the one form its file system decides (assumption — a falling-back volume was not measured). The
///   64-bit form may not be unique on ReFS (MS `BY_HANDLE_FILE_INFORMATION` docs).
/// - Elsewhere = (device number, inode).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileIdentity {
    volume: u64,
    file: u128,
}

/// The identity of the file or folder `path` points to. Links are followed — a junction or symbolic link gives the target's
/// identity, a broken link is `NotFound`. Failure = the OS error of the open or query as is · `Unsupported` (no OS error code) on
/// Windows when neither query gives a unique id.
///
/// `Unsupported` means the file has no identity — the caller handles it without one and does not count it as a scan failure. The
/// root folder of a FAT volume has file id 0, so it likely gets this answer (fastfat sample source · not measured · exFAT unknown).
#[cfg(windows)]
pub fn file_identity(path: &Path) -> io::Result<FileIdentity> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE,
    };

    // Opening a folder too needs `FILE_FLAG_BACKUP_SEMANTICS` (std `File::open` cannot open folders).
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES.0)
        .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
        .open(path)?;
    opened_identity(&file)
}

/// The identity of what `file` holds open — [`file_identity`]'s answer (and its `Unsupported`) for that handle, with no
/// name looked up again.
#[cfg(windows)]
pub fn opened_identity(file: &std::fs::File) -> io::Result<FileIdentity> {
    match wide_identity(file) {
        Ok(id) if !is_sentinel_id(id.file) => Ok(id),
        _ => match narrow_identity(file)? {
            id if is_sentinel_id(id.file) => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "this volume does not give each file a unique id",
            )),
            id => Ok(id),
        },
    }
}

// "No unique id" values — 0 · 64-bit -1 (`FILE_INVALID_FILE_ID` in the Windows headers) and that -1 widened to 128 bits (zero-
//   filled · one-filled). None was seen on this PC's volumes (unverified). Used as an identity, every file of that volume would
//   share one identity.
#[cfg(windows)]
fn is_sentinel_id(file: u128) -> bool {
    file == 0 || file == u128::from(u64::MAX) || file == u128::MAX
}
#[cfg(unix)]
pub fn file_identity(path: &Path) -> io::Result<FileIdentity> {
    identity_of(&std::fs::metadata(path)?)
}
#[cfg(unix)]
pub fn opened_identity(file: &std::fs::File) -> io::Result<FileIdentity> {
    identity_of(&file.metadata()?)
}
#[cfg(unix)]
fn identity_of(meta: &std::fs::Metadata) -> io::Result<FileIdentity> {
    use std::os::unix::fs::MetadataExt;
    Ok(FileIdentity {
        volume: meta.dev(),
        file: u128::from(meta.ino()),
    })
}

#[cfg(windows)]
fn wide_identity(file: &std::fs::File) -> io::Result<FileIdentity> {
    use windows::Win32::Storage::FileSystem::{FileIdInfo, FILE_ID_INFO};
    let info: FILE_ID_INFO = super::file_info(file, FileIdInfo)?;
    Ok(FileIdentity {
        volume: info.VolumeSerialNumber,
        file: u128::from_le_bytes(info.FileId.Identifier),
    })
}

#[cfg(windows)]
fn narrow_identity(file: &std::fs::File) -> io::Result<FileIdentity> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: `file` is a handle open for this call, and what is passed is one struct.
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
        .map_err(super::win32_io_error)?;
    Ok(FileIdentity {
        volume: u64::from(info.dwVolumeSerialNumber),
        file: (u128::from(info.nFileIndexHigh) << 32) | u128::from(info.nFileIndexLow),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::Scratch;

    #[test]
    fn two_files_differ_and_a_rename_keeps_the_identity() {
        let scratch = Scratch::new("identity");
        let first = scratch.0.join("first.md");
        let second = scratch.0.join("second.md");
        std::fs::write(&first, b"1").expect("first file");
        std::fs::write(&second, b"2").expect("second file");

        let id = file_identity(&first).expect("first identity");
        assert_ne!(id, file_identity(&second).expect("second identity"));

        let renamed = scratch.0.join("renamed.md");
        std::fs::rename(&first, &renamed).expect("rename");
        assert_eq!(file_identity(&renamed).expect("identity after rename"), id);
    }

    #[test]
    fn a_directory_has_its_own_stable_identity() {
        let scratch = Scratch::new("identity");
        let inner = scratch.0.join("inner.md");
        std::fs::write(&inner, b"i").expect("inner file");

        let dir = file_identity(&scratch.0).expect("folder identity");
        assert_eq!(file_identity(&scratch.0).expect("again"), dir);
        assert_ne!(file_identity(&inner).expect("file identity"), dir);
    }

    #[test]
    fn another_spelling_of_the_same_path_has_the_same_identity() {
        let scratch = Scratch::new("identity");
        let file = scratch.0.join("Note.md");
        std::fs::write(&file, b"n").expect("file");
        let id = file_identity(&file).expect("identity");

        assert_eq!(
            file_identity(&scratch.0.join(".").join("Note.md")).expect("`.` spelling"),
            id
        );
        if cfg!(windows) {
            assert_eq!(
                file_identity(&scratch.0.join("NOTE.MD")).expect("case spelling"),
                id
            );
        }
    }

    // No volume on this PC takes the fallback (where the 128-bit query fails), so that branch is not measured — this only checks
    //   that both queries run and the 128-bit answer is used.
    #[cfg(windows)]
    #[test]
    fn the_128_bit_answer_is_used_where_the_volume_gives_it() {
        let scratch = Scratch::new("identity");
        let file = scratch.0.join("wide.md");
        std::fs::write(&file, b"w").expect("file");
        let opened = std::fs::File::open(&file).expect("open");

        let wide = wide_identity(&opened).expect("128-bit query");
        let narrow = narrow_identity(&opened).expect("64-bit query");
        assert_eq!(file_identity(&file).expect("identity"), wide);
        assert_eq!(
            wide.volume as u32 as u64, narrow.volume,
            "low 32 bits of the volume serial number"
        );
    }

    #[cfg(windows)]
    #[test]
    fn sentinel_ids_are_not_identities() {
        for sentinel in [0, u128::from(u64::MAX), u128::MAX] {
            assert!(is_sentinel_id(sentinel), "{sentinel:#x}");
        }
        for real in [
            1,
            u128::from(u64::MAX) - 1,
            u128::from(u64::MAX) + 1,
            u128::MAX - 1,
        ] {
            assert!(!is_sentinel_id(real), "{real:#x}");
        }
    }

    #[test]
    fn an_opened_file_has_the_identity_of_its_path() {
        let scratch = Scratch::new("identity");
        let file = scratch.0.join("opened.md");
        std::fs::write(&file, b"o").expect("file");

        let opened = std::fs::File::open(&file).expect("open");
        assert_eq!(
            opened_identity(&opened).expect("identity of the handle"),
            file_identity(&file).expect("identity of the path")
        );
    }

    #[test]
    fn a_missing_path_is_not_found() {
        let scratch = Scratch::new("identity");
        let e = file_identity(&scratch.0.join("missing")).expect_err("missing path");
        assert_eq!(e.kind(), io::ErrorKind::NotFound);
    }
}
