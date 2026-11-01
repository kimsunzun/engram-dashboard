//! 파일 신원 — 한 볼륨 안의 파일 · 폴더를 이름과 무관하게 가리키는 값(이름을 바꿔도 같다).

// ADR-0325

use std::io;
use std::path::Path;

/// [`file_identity`] 의 답 — 견주기 · 해시 열쇠로만 쓴다.
///
/// - Windows = 볼륨 일련번호 + 파일 id. 128비트 `FileIdInfo` 를 먼저 묻고, 그 질의가 실패하거나 고유 id 가 없다는
///   값을 주면 32비트 볼륨 일련번호 + 64비트 파일 인덱스로 물러난다. ★두 꼴은 같은 파일이어도 같지 않다★ — 한
///   볼륨에서는 파일 시스템이 정한 한 꼴만 나온다고 본다(가정 — 물러나는 볼륨은 재 보지 않았다). 64비트 꼴은
///   ReFS 에서 유일하지 않을 수 있다(MS `BY_HANDLE_FILE_INFORMATION` 문서).
/// - 그 밖 = (장치 번호, inode).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileIdentity {
    volume: u64,
    file: u128,
}

/// `path` 가 가리키는 파일 · 폴더의 신원. 링크는 따라간다 — 정션 · 심볼릭 링크면 대상의 신원이고, 끊긴 링크면
/// `NotFound` 다. 실패 = 열기 · 질의의 OS 오류 그대로 · Windows 에서 두 질의 다 고유 id 를 주지 않으면
/// `Unsupported`(OS 오류 코드 없음).
///
/// `Unsupported` 는 그 파일에 신원이 없다는 답이다 — 부르는 쪽은 신원 없이 다루고, 훑기 실패로 치지 않는다. FAT
/// 볼륨의 루트 폴더는 파일 id 가 0 이라 이 답일 가능성이 높다(fastfat 샘플 소스 · 미측 · exFAT 은 모름).
#[cfg(windows)]
pub fn file_identity(path: &Path) -> io::Result<FileIdentity> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE,
    };

    // 폴더도 열려면 `FILE_FLAG_BACKUP_SEMANTICS` 가 있어야 한다(std `File::open` 은 폴더를 못 연다).
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES.0)
        .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
        .open(path)?;
    match wide_identity(&file) {
        Ok(id) if !is_sentinel_id(id.file) => Ok(id),
        _ => match narrow_identity(&file)? {
            id if is_sentinel_id(id.file) => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "이 볼륨은 파일마다 고유한 id 를 주지 않는다",
            )),
            id => Ok(id),
        },
    }
}

// 고유 id 가 없다는 값 — 0 · 64비트 -1(Windows 헤더의 `FILE_INVALID_FILE_ID`)과 그 -1 을 128비트로 넓힌 두 꼴(0
//   채움 · 1 채움). 어느 것도 이 PC 볼륨에서 본 적 없다(미검). 이 값을 신원으로 쓰면 그 볼륨의 파일이 전부 한
//   신원이 된다.
#[cfg(windows)]
fn is_sentinel_id(file: u128) -> bool {
    file == 0 || file == u128::from(u64::MAX) || file == u128::MAX
}
#[cfg(unix)]
pub fn file_identity(path: &Path) -> io::Result<FileIdentity> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(path)?;
    Ok(FileIdentity {
        volume: meta.dev(),
        file: u128::from(meta.ino()),
    })
}

#[cfg(windows)]
fn wide_identity(file: &std::fs::File) -> io::Result<FileIdentity> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        FileIdInfo, GetFileInformationByHandleEx, FILE_ID_INFO,
    };
    let mut info = FILE_ID_INFO::default();
    // SAFETY: `file` 이 이 호출 동안 열려 있는 핸들이고, 버퍼는 넘긴 크기 그대로의 `FILE_ID_INFO` 다.
    unsafe {
        GetFileInformationByHandleEx(
            HANDLE(file.as_raw_handle()),
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    }
    .map_err(super::win32_io_error)?;
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
    // SAFETY: `file` 이 이 호출 동안 열려 있는 핸들이고, 넘기는 것은 구조체 하나다.
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
    use std::path::PathBuf;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "engram-platform-identity-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir(&dir).expect("시험 폴더");
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn two_files_differ_and_a_rename_keeps_the_identity() {
        let scratch = Scratch::new();
        let first = scratch.0.join("first.md");
        let second = scratch.0.join("second.md");
        std::fs::write(&first, b"1").expect("첫 파일");
        std::fs::write(&second, b"2").expect("둘째 파일");

        let id = file_identity(&first).expect("첫 신원");
        assert_ne!(id, file_identity(&second).expect("둘째 신원"));

        let renamed = scratch.0.join("renamed.md");
        std::fs::rename(&first, &renamed).expect("이름 바꾸기");
        assert_eq!(file_identity(&renamed).expect("바꾼 뒤 신원"), id);
    }

    #[test]
    fn a_directory_has_its_own_stable_identity() {
        let scratch = Scratch::new();
        let inner = scratch.0.join("inner.md");
        std::fs::write(&inner, b"i").expect("안 파일");

        let dir = file_identity(&scratch.0).expect("폴더 신원");
        assert_eq!(file_identity(&scratch.0).expect("다시"), dir);
        assert_ne!(file_identity(&inner).expect("파일 신원"), dir);
    }

    #[test]
    fn another_spelling_of_the_same_path_has_the_same_identity() {
        let scratch = Scratch::new();
        let file = scratch.0.join("Note.md");
        std::fs::write(&file, b"n").expect("파일");
        let id = file_identity(&file).expect("신원");

        assert_eq!(
            file_identity(&scratch.0.join(".").join("Note.md")).expect("`.` 철자"),
            id
        );
        if cfg!(windows) {
            assert_eq!(
                file_identity(&scratch.0.join("NOTE.MD")).expect("대소문자 철자"),
                id
            );
        }
    }

    // 물러난 길이 실제로 도는 볼륨(128비트 질의가 실패하는 볼륨)은 이 PC 에 없어 그 갈래는 재지 못한다 — 여기서는
    //   두 질의가 다 돌고 128비트가 쓰인다는 것만 본다.
    #[cfg(windows)]
    #[test]
    fn the_128_bit_answer_is_used_where_the_volume_gives_it() {
        let scratch = Scratch::new();
        let file = scratch.0.join("wide.md");
        std::fs::write(&file, b"w").expect("파일");
        let opened = std::fs::File::open(&file).expect("열기");

        let wide = wide_identity(&opened).expect("128비트 질의");
        let narrow = narrow_identity(&opened).expect("64비트 질의");
        assert_eq!(file_identity(&file).expect("신원"), wide);
        assert_eq!(
            wide.volume as u32 as u64, narrow.volume,
            "볼륨 일련번호 아래 32비트"
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
    fn a_missing_path_is_not_found() {
        let scratch = Scratch::new();
        let e = file_identity(&scratch.0.join("missing")).expect_err("없는 경로");
        assert_eq!(e.kind(), io::ErrorKind::NotFound);
    }
}
