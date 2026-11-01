//! 디렉터리 링크 — 만들기 · 걷기. Windows = 디렉터리 정션(관리자 권한 · 개발자 모드 불요) · 그 밖 = 심볼릭 링크.

// ADR-0336

use std::io;
use std::path::Path;

/// `link` 자리에 폴더 `target` 을 가리키는 디렉터리 링크를 만든다 — Windows = 디렉터리 정션 · 그 밖 = 심볼릭 링크.
///
/// - `target` 은 절대 경로이고 NUL 이 없어야 한다 — 아니면 `InvalidInput` 이고 아무것도 만들지 않는다. 있지 않아도
///   된다(끊긴 링크가 된다).
/// - `link` 자리에 무엇이든 이미 있으면 `AlreadyExists` 이고 그것은 그대로다.
/// - Windows 의 정션은 재해석 지점을 받는 볼륨(NTFS 등)에서만 선다.
/// - Windows 에서 그 밖의 이유로 실패하면 `link` 자리에 빈 폴더가 남을 수 있다 — 지우지 않는다(남이 만든 폴더를
///   지울 수 있어서). 쓰는 곳은 휘발 폴더 안이라 그 폴더째 치워진다.
/// - Windows 에서 링크 자리 폴더를 열 권한이 없으면 접근 거부(원시 5 — [`super::is_access_denied`])다.
#[cfg(windows)]
pub fn link_dir(target: &Path, link: &Path) -> io::Result<()> {
    check_target(target)?;
    check_link(link)?;
    junction::create(target, link).map_err(junction_error)
}
#[cfg(unix)]
pub fn link_dir(target: &Path, link: &Path) -> io::Result<()> {
    check_target(target)?;
    std::os::unix::fs::symlink(target, link)
}

/// [`link_dir`] 로 만든 링크를 걷는다 — 링크만 지우고 대상 폴더와 그 안은 그대로다. 대상이 사라진 끊긴 링크도
/// 걷는다.
///
/// - `link` 가 링크가 아니면 `InvalidInput` 이고 아무것도 지우지 않는다. Windows 는 정션만 링크로 친다 — 진짜
///   폴더 · 파일 · 디렉터리 심볼릭 링크 · 클라우드 자리표시 같은 다른 재해석 지점은 거절한다. 남이 쓰고 있거나 지울
///   수 없는 진짜 폴더도 그렇다(그 자리의 속성조차 못 읽으면 그 열기 오류다). 그 밖은 심볼릭 링크만 친다.
/// - 없으면 `NotFound`.
/// - Windows 는 열어서 정션임을 확인한 바로 그 객체를 같은 핸들로 지운다 — 확인 뒤 그 이름에 다른 객체가 들어서도
///   그것을 지우지 않는다(객체 신원). 쥔 동안 공유는 읽기만 주어 남이 이름 바꾸기나 쓰기 · 삭제 권한 열기를 못
///   하지만, 속성 쓰기(`FILE_WRITE_ATTRIBUTES`)만 청한 핸들은 공유 검사를 건너뛰고 MS-FSA 로는 그 권한으로 재해석
///   지점을 벗길 수 있다(`FSCTL_DELETE_REPARSE_POINT`). 확인과 지우기 사이에 누가 그렇게 벗기면 지워지는 것은 같은
///   객체 — 이제 빈 폴더 — 다(안이 찬 폴더는 지우기 표시가 실패한다). 미검 — 그 경합은 재지 않았다.
/// - Windows 에서 그 정션을 남이 삭제 공유 없이 열고 있거나 쓰기 · 삭제 권한으로 열고 있으면 공유 위반(원시 32 —
///   [`super::is_sharing_violation`]), 지울 권한이 없으면 접근 거부(원시 5)다.
/// - Windows 밖은 확인과 지우기가 한 동작이 아니다(그 사이 바뀐 파일을 지울 수 있다).
/// - Windows 의 볼륨 마운트 지점은 정션과 같은 재해석 tag 라 가르지 않는다(미검 — 마운트 지점은 관리자만 만든다).
#[cfg(windows)]
pub fn unlink_dir(link: &Path) -> io::Result<()> {
    unlink_junction(link).map(|_| ())
}
#[cfg(unix)]
pub fn unlink_dir(link: &Path) -> io::Result<()> {
    if !std::fs::symlink_metadata(link)?.file_type().is_symlink() {
        return Err(not_a_link());
    }
    // 심볼릭 링크는 가리키는 것이 폴더여도 파일로 지운다 — `remove_dir` 는 `ENOTDIR` 이다.
    std::fs::remove_file(link)
}

// `std::fs::remove_dir` 처럼 경로로 다시 찾아 지우면 확인 뒤 그 이름에 들어선 남의 빈 폴더를 지운다 — 확인한 그
//   핸들로 지운다. 재해석 지점 자체를 연 핸들이라 지워지는 것은 링크뿐이고 대상은 건드리지 않는다.
//   `junction::delete` 는 reparse 만 벗겨 빈 폴더를 남기고, `junction::exists` 는 링크를 따라가 끊긴 정션에 `false`
//   라 확인에 못 쓴다. 어느 꼴로 지웠나는 시험이 남의 핸들이 열린 정션의 이름이 언제 빠지는지를 가르는 데 쓴다.
#[cfg(windows)]
fn unlink_junction(link: &Path) -> io::Result<Disposition> {
    let junction = open_junction(link)?;
    delete_on_close(&junction)
}

fn check_target(target: &Path) -> io::Result<()> {
    if !target.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "링크 대상은 절대 경로여야 한다",
        ));
    }
    // `junction::create` 는 NUL 을 보지 않고 넘겨, OS 가 그 앞에서 자른 경로에 링크를 건다(2.1.0 `get_full_path`).
    if target.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "링크 대상에 NUL 이 있다",
        ));
    }
    Ok(())
}

// NUL 은 아래 `symlink_metadata` 도 `InvalidInput` 으로 거절하지만 그 문장은 어느 인자인지 말하지 않는다 — 링크
//   자리라고 밝히려고 먼저 본다.
#[cfg(windows)]
fn check_link(link: &Path) -> io::Result<()> {
    if link.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "링크 자리에 NUL 이 있다",
        ));
    }
    match std::fs::symlink_metadata(link) {
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "링크 자리에 이미 무언가 있다",
        )),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn not_a_link() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "디렉터리 링크가 아니다")
}

// junction 2.1.0 은 링크 폴더 열기가 접근 거부면 심볼릭 링크 특권을 켜 보고, 토큰에 그 특권이 없으면 원래 거부
//   대신 `ERROR_NOT_ALL_ASSIGNED`(1300)를 돌려준다(`helpers.rs` 소스) — 1300 은 그 자리에서만 난다.
#[cfg(windows)]
fn junction_error(e: io::Error) -> io::Error {
    const ERROR_NOT_ALL_ASSIGNED: i32 = 1300;
    if e.raw_os_error() == Some(ERROR_NOT_ALL_ASSIGNED) {
        io::Error::from_raw_os_error(super::ERROR_ACCESS_DENIED)
    } else {
        e
    }
}

// 링크를 따라가지 않고 그 자리 자체를 지울 권한으로 열고 재해석 tag 를 본다 — std 는 tag 를 내주지 않는다. 공유를
//   읽기만 주는 까닭과 그것이 못 막는 것 = `unlink_dir` 문서.
#[cfg(windows)]
fn open_junction(path: &Path) -> io::Result<std::fs::File> {
    use windows::Win32::Storage::FileSystem::{
        DELETE, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    let file = match open_reparse_point(path, (DELETE | FILE_READ_ATTRIBUTES).0, FILE_SHARE_READ.0)
    {
        Ok(file) => file,
        // 지울 권한으로 못 연 것이 남이 쓰고 있거나 지울 수 없는 진짜 폴더라서일 수 있다 — 속성만 여는 핸들로 tag 를
        //   보고 링크가 아니면 `InvalidInput`, 링크이거나 그마저 못 보면 원래 오류다. 그 핸들로는 지우지 않는다.
        Err(e) if super::is_access_denied(&e) || super::is_sharing_violation(&e) => {
            let probed = open_reparse_point(
                path,
                FILE_READ_ATTRIBUTES.0,
                (FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0,
            )
            .and_then(|probe| is_junction(&probe));
            return Err(match probed {
                Ok(false) => not_a_link(),
                _ => e,
            });
        }
        Err(e) => return Err(e),
    };
    if is_junction(&file)? {
        Ok(file)
    } else {
        Err(not_a_link())
    }
}

#[cfg(windows)]
fn open_reparse_point(path: &Path, access: u32, share: u32) -> io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    };
    std::fs::OpenOptions::new()
        .access_mode(access)
        .share_mode(share)
        .custom_flags((FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS).0)
        .open(path)
}

#[cfg(windows)]
fn is_junction(file: &std::fs::File) -> io::Result<bool> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        FileAttributeTagInfo, GetFileInformationByHandleEx, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_ATTRIBUTE_TAG_INFO,
    };
    // 이 상수 하나 때문에 `Win32_System_SystemServices` 를 켜지 않는다.
    const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;

    let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
    // SAFETY: `file` 이 이 호출 동안 열려 있는 핸들이고, 버퍼는 넘긴 크기 그대로의 `FILE_ATTRIBUTE_TAG_INFO` 다.
    unsafe {
        GetFileInformationByHandleEx(
            HANDLE(file.as_raw_handle()),
            FileAttributeTagInfo,
            (&mut info as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    }
    .map_err(super::win32_io_error)?;
    Ok(info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
        && info.ReparseTag == IO_REPARSE_TAG_MOUNT_POINT)
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Disposition {
    Posix,
    Classic,
}

// 지우기 표시만 하고 지워지는 것은 `file` 을 닫을 때다. POSIX 꼴은 이 핸들을 닫는 순간 이름을 뺀다 — 남이 그 정션을
//   열고 있어도 그렇다. 옛 꼴은 남의 핸들이 닫힐 때까지 이름이 남아, 이어서 부모 폴더를 지우면 「비어 있지 않다」(145)
//   로 실패한다(실측 2026-11-01 · Windows 11 26200 NTFS). POSIX 꼴을 모르는 Windows · 파일 시스템이 내는 오류(1 · 50 ·
//   87)면 옛 꼴로 물러난다(미검 — 이 PC 의 NTFS 는 POSIX 꼴을 받는다).
#[cfg(windows)]
fn delete_on_close(file: &std::fs::File) -> io::Result<Disposition> {
    const ERROR_INVALID_FUNCTION: i32 = 1;
    const ERROR_NOT_SUPPORTED: i32 = 50;
    const ERROR_INVALID_PARAMETER: i32 = 87;
    match posix_delete_on_close(file) {
        Ok(()) => Ok(Disposition::Posix),
        Err(e)
            if matches!(
                e.raw_os_error(),
                Some(ERROR_INVALID_FUNCTION | ERROR_NOT_SUPPORTED | ERROR_INVALID_PARAMETER)
            ) =>
        {
            classic_delete_on_close(file).map(|()| Disposition::Classic)
        }
        Err(e) => Err(e),
    }
}

// 읽기 전용 속성이 붙은 정션도 지운다 — std `remove_dir_all` 이 쓰는 깃발 그대로다. 옛 꼴엔 이 깃발이 없어 읽기 전용
//   정션은 접근 거부다(미검). 이 깃발을 모르는 Windows 는 87 을 내 옛 꼴로 물러난다고 본다(미검).
#[cfg(windows)]
fn posix_delete_on_close(file: &std::fs::File) -> io::Result<()> {
    use windows::Win32::Storage::FileSystem::{
        FileDispositionInfoEx, FILE_DISPOSITION_FLAG_DELETE,
        FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE, FILE_DISPOSITION_FLAG_POSIX_SEMANTICS,
        FILE_DISPOSITION_INFO_EX, FILE_DISPOSITION_INFO_EX_FLAGS,
    };
    let info = FILE_DISPOSITION_INFO_EX {
        Flags: FILE_DISPOSITION_INFO_EX_FLAGS(
            FILE_DISPOSITION_FLAG_DELETE.0
                | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS.0
                | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE.0,
        ),
    };
    set_disposition(file, FileDispositionInfoEx, &info)
}

#[cfg(windows)]
fn classic_delete_on_close(file: &std::fs::File) -> io::Result<()> {
    use windows::Win32::Foundation::BOOLEAN;
    use windows::Win32::Storage::FileSystem::{FileDispositionInfo, FILE_DISPOSITION_INFO};
    let info = FILE_DISPOSITION_INFO {
        DeleteFile: BOOLEAN(1),
    };
    set_disposition(file, FileDispositionInfo, &info)
}

#[cfg(windows)]
fn set_disposition<T>(
    file: &std::fs::File,
    class: windows::Win32::Storage::FileSystem::FILE_INFO_BY_HANDLE_CLASS,
    info: &T,
) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::SetFileInformationByHandle;
    // SAFETY: `file` 이 이 호출 동안 열려 있는 핸들이고, `info` 는 넘긴 크기 그대로의 그 정보 꼴 구조체다(부르는
    //   두 곳이 정보 꼴과 구조체를 짝지어 넘긴다).
    unsafe {
        SetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            class,
            (info as *const T).cast(),
            std::mem::size_of::<T>() as u32,
        )
    }
    .map_err(super::win32_io_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::file_identity;
    use std::path::PathBuf;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "engram-platform-link-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir(&dir).expect("시험 폴더");
            Self(dir)
        }

        fn target_with_file(&self) -> PathBuf {
            let target = self.0.join("target");
            std::fs::create_dir(&target).expect("대상 폴더");
            std::fs::write(target.join("inner.txt"), b"kept").expect("대상 안 파일");
            target
        }
    }

    // 단언이 중간에 실패해도 치운다. std `remove_dir_all` 은 링크를 따라 들어가지 않는다.
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn gone(path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_err_and(|e| e.kind() == io::ErrorKind::NotFound)
    }

    #[test]
    fn a_link_reads_through_to_the_target_and_unlinking_leaves_the_target_intact() {
        let scratch = Scratch::new();
        let target = scratch.target_with_file();
        let link = scratch.0.join("link");

        link_dir(&target, &link).expect("링크 만들기");
        assert_eq!(
            std::fs::read(link.join("inner.txt")).expect("링크로 읽기"),
            b"kept"
        );
        assert_eq!(
            file_identity(&link).expect("링크의 신원"),
            file_identity(&target).expect("대상의 신원"),
            "신원은 링크를 따라간다"
        );

        unlink_dir(&link).expect("링크 걷기");
        assert!(gone(&link), "링크 자리가 비어야");
        assert_eq!(
            std::fs::read(target.join("inner.txt")).expect("대상 안 파일"),
            b"kept",
            "대상은 그대로"
        );
    }

    #[test]
    fn unlink_refuses_a_real_directory_and_a_file() {
        let scratch = Scratch::new();
        let real = scratch.target_with_file();
        let empty = scratch.0.join("empty");
        std::fs::create_dir(&empty).expect("빈 폴더");
        let file = scratch.0.join("file.txt");
        std::fs::write(&file, b"f").expect("파일");

        for path in [&real, &empty, &file] {
            let e = unlink_dir(path).expect_err("링크가 아니면 거절");
            assert_eq!(
                e.kind(),
                io::ErrorKind::InvalidInput,
                "{}: {e}",
                path.display()
            );
            assert!(path.exists(), "{} 는 그대로", path.display());
        }
        assert!(real.join("inner.txt").exists());
    }

    #[test]
    fn a_broken_link_is_still_unlinked() {
        let scratch = Scratch::new();
        let target = scratch.target_with_file();
        let link = scratch.0.join("link");
        link_dir(&target, &link).expect("링크 만들기");
        std::fs::remove_dir_all(&target).expect("대상 지우기");

        unlink_dir(&link).expect("끊긴 링크도 걷는다");
        assert!(gone(&link));
    }

    #[test]
    fn unlinking_a_missing_path_is_not_found() {
        let scratch = Scratch::new();
        let e = unlink_dir(&scratch.0.join("missing")).expect_err("없는 자리");
        assert_eq!(e.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn a_relative_target_is_refused_and_nothing_is_made() {
        let scratch = Scratch::new();
        let link = scratch.0.join("link");
        let e = link_dir(Path::new("relative/target"), &link).expect_err("상대 경로 거절");
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput);
        assert!(gone(&link));
    }

    #[test]
    fn an_occupied_link_path_is_left_alone() {
        let scratch = Scratch::new();
        let target = scratch.target_with_file();
        let occupied = scratch.0.join("occupied");
        std::fs::create_dir(&occupied).expect("이미 있는 폴더");
        std::fs::write(occupied.join("mine.txt"), b"m").expect("그 안 파일");
        let empty = scratch.0.join("empty");
        std::fs::create_dir(&empty).expect("이미 있는 빈 폴더");

        let e = link_dir(&target, &occupied).expect_err("이미 있는 자리");
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        assert!(occupied.join("mine.txt").exists(), "있던 것은 그대로");

        let e = link_dir(&target, &empty).expect_err("이미 있는 빈 자리");
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        assert!(
            std::fs::symlink_metadata(&empty).is_ok_and(|m| m.is_dir()),
            "있던 빈 폴더는 링크가 아닌 폴더 그대로"
        );
        assert!(
            std::fs::read_dir(&empty)
                .expect("빈 폴더 읽기")
                .next()
                .is_none(),
            "빈 채로"
        );
    }

    #[test]
    fn a_target_with_nul_is_refused_and_nothing_is_made() {
        let scratch = Scratch::new();
        let link = scratch.0.join("link");
        let e = link_dir(&scratch.0.join("tar\0get"), &link).expect_err("NUL 거절");
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput, "{e}");
        assert!(gone(&link));
    }

    #[test]
    fn unlinking_keeps_everything_under_the_target() {
        let scratch = Scratch::new();
        let target = scratch.target_with_file();
        let nested = target.join("nested");
        std::fs::create_dir(&nested).expect("안 폴더");
        std::fs::write(nested.join("deep.txt"), b"deep").expect("안 폴더의 파일");
        let identity = file_identity(&target).expect("대상의 신원");
        let link = scratch.0.join("link");
        link_dir(&target, &link).expect("링크 만들기");

        unlink_dir(&link).expect("링크 걷기");
        assert!(gone(&link));
        assert_eq!(file_identity(&target).expect("걷은 뒤 신원"), identity);
        assert_eq!(
            std::fs::read(target.join("inner.txt")).expect("대상 안 파일"),
            b"kept"
        );
        assert_eq!(
            std::fs::read(nested.join("deep.txt")).expect("안 폴더의 파일"),
            b"deep"
        );
    }

    // 옮기기만 원시 코드로 본다 — 1300 이 나는 자리(링크 폴더 ACL 거부 + 특권 없는 토큰)는 시험이 만들지 않는다.
    #[cfg(windows)]
    #[test]
    fn a_privilege_failure_from_junction_reads_as_access_denied() {
        let mapped = junction_error(io::Error::from_raw_os_error(1300));
        assert_eq!(
            mapped.raw_os_error(),
            Some(super::super::ERROR_ACCESS_DENIED)
        );
        assert!(super::super::is_access_denied(&mapped));
        assert_eq!(mapped.kind(), io::ErrorKind::PermissionDenied);

        let other = junction_error(io::Error::from_raw_os_error(3));
        assert_eq!(other.raw_os_error(), Some(3), "다른 오류는 그대로");
    }

    // 5천 자 대상은 정션 버퍼(16 KiB)에 안 담겨 junction 2.1.0 의 길이 확인(OS 코드 없는 `InvalidInput`)에서 실패한다.
    //   그 확인은 링크 자리 폴더를 만든 뒤라 빈 폴더가 남는다 — 그것을 재야 지우는 정리 코드가 되살아나면 잡힌다.
    #[cfg(windows)]
    #[test]
    fn a_too_long_target_fails_and_deletes_nothing() {
        let scratch = Scratch::new();
        let sibling = scratch.target_with_file();
        let too_long = scratch.0.join("x".repeat(5000));
        let link = scratch.0.join("link");

        let e = link_dir(&too_long, &link).expect_err("버퍼를 넘는 대상");
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput, "{e:?}");
        assert_eq!(e.raw_os_error(), None);
        assert!(
            std::fs::symlink_metadata(&link).is_ok_and(|m| m.is_dir()),
            "링크 자리엔 링크가 아닌 폴더가 남는다"
        );
        assert!(
            std::fs::read_dir(&link)
                .expect("남은 폴더 읽기")
                .next()
                .is_none(),
            "빈 채로"
        );
        assert_eq!(
            std::fs::read(sibling.join("inner.txt")).expect("곁 폴더 안 파일"),
            b"kept",
            "곁의 것은 그대로"
        );
    }

    #[cfg(windows)]
    #[test]
    fn the_classic_disposition_also_removes_only_the_link() {
        let scratch = Scratch::new();
        let target = scratch.target_with_file();
        let link = scratch.0.join("link");
        link_dir(&target, &link).expect("링크 만들기");

        let junction = open_junction(&link).expect("정션 열기");
        classic_delete_on_close(&junction).expect("옛 꼴 지우기 표시");
        drop(junction);
        assert!(gone(&link));
        assert_eq!(
            std::fs::read(target.join("inner.txt")).expect("대상 안 파일"),
            b"kept"
        );
    }

    // 남이 그 자리 자체를 읽기(또는 읽기+쓰기)로 연다 — 속성만 여는 핸들은 공유 검사에 들지 않아(실측) 그런 핸들로는
    //   아래 시험들이 재는 것이 없다.
    #[cfg(windows)]
    fn hold(path: &Path, write: bool, share: u32) -> std::fs::File {
        use std::os::windows::fs::OpenOptionsExt;
        use windows::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        };
        std::fs::OpenOptions::new()
            .read(true)
            .write(write)
            .share_mode(share)
            .custom_flags((FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS).0)
            .open(path)
            .expect("남이 그 자리를 연다")
    }

    // POSIX 꼴을 고른 까닭(`delete_on_close` 주석)을 잰다 — 남이 정션 자체를 열고 있어도 걷은 즉시 이름이 빠져 부모
    //   폴더를 지울 수 있다. 옛 꼴로 물러난 볼륨이면 이름은 그 핸들이 닫힐 때 빠진다.
    #[cfg(windows)]
    #[test]
    fn a_junction_held_open_elsewhere_is_gone_once_unlinked() {
        use windows::Win32::Storage::FileSystem::{
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        };
        let scratch = Scratch::new();
        let target = scratch.target_with_file();
        let parent = scratch.0.join("parent");
        std::fs::create_dir(&parent).expect("부모 폴더");
        let link = parent.join("link");
        link_dir(&target, &link).expect("링크 만들기");
        let held = hold(
            &link,
            false,
            (FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0,
        );

        match unlink_junction(&link).expect("링크 걷기") {
            Disposition::Posix => {
                assert!(gone(&link), "남의 핸들이 열린 채로도 이름이 빠져야");
                std::fs::remove_dir(&parent).expect("부모 폴더를 지울 수 있어야");
                drop(held);
            }
            Disposition::Classic => {
                drop(held);
                assert!(gone(&link), "남의 핸들이 닫히면 이름이 빠져야");
                std::fs::remove_dir(&parent).expect("부모 폴더를 지울 수 있어야");
            }
        }
        assert!(target.join("inner.txt").exists());
    }

    #[cfg(windows)]
    #[test]
    fn unlink_is_a_sharing_violation_while_someone_holds_the_junction_without_delete_sharing() {
        use windows::Win32::Storage::FileSystem::FILE_SHARE_READ;
        let scratch = Scratch::new();
        let target = scratch.target_with_file();
        let link = scratch.0.join("link");
        link_dir(&target, &link).expect("링크 만들기");
        let held = hold(&link, false, FILE_SHARE_READ.0);

        let e = unlink_dir(&link).expect_err("공유 위반");
        assert!(super::super::is_sharing_violation(&e), "{e:?}");
        drop(held);
        unlink_dir(&link).expect("놓으면 걷힌다");
        assert!(gone(&link));
    }

    // 걷기 핸들은 읽기만 공유한다 — 남이 모든 공유를 주고 쓰기로 연 정션도 공유 위반이다.
    #[cfg(windows)]
    #[test]
    fn unlink_is_a_sharing_violation_while_someone_holds_the_junction_for_writing() {
        use windows::Win32::Storage::FileSystem::{
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        };
        let scratch = Scratch::new();
        let target = scratch.target_with_file();
        let link = scratch.0.join("link");
        link_dir(&target, &link).expect("링크 만들기");
        let held = hold(
            &link,
            true,
            (FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0,
        );

        let e = unlink_dir(&link).expect_err("공유 위반");
        assert!(super::super::is_sharing_violation(&e), "{e:?}");
        drop(held);
        unlink_dir(&link).expect("놓으면 걷힌다");
        assert!(gone(&link));
    }

    // 남이 삭제 공유 없이 연 진짜 폴더(다른 프로세스의 작업 폴더가 그렇다)는 지울 권한으로 못 열지만 링크가 아니라
    //   `InvalidInput` 이다 — 첫 단언이 그 열기가 실제로 막히는지 잰다.
    #[cfg(windows)]
    #[test]
    fn a_busy_real_directory_is_refused_as_not_a_link() {
        use windows::Win32::Storage::FileSystem::{
            DELETE, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
        };
        let scratch = Scratch::new();
        let real = scratch.target_with_file();
        let held = hold(&real, false, (FILE_SHARE_READ | FILE_SHARE_WRITE).0);
        let blocked =
            open_reparse_point(&real, (DELETE | FILE_READ_ATTRIBUTES).0, FILE_SHARE_READ.0)
                .expect_err("전제: 지울 권한 열기가 막힌다");
        assert!(
            super::super::is_sharing_violation(&blocked),
            "전제: {blocked:?}"
        );

        let e = unlink_dir(&real).expect_err("링크가 아니면 거절");
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput, "{e:?}");
        drop(held);
        assert!(real.join("inner.txt").exists(), "진짜 폴더는 그대로");
    }

    // 디렉터리 심볼릭 링크는 개발자 모드 · 특권이 있어야 만들어진다 — 못 만드는 PC 에선 확인할 것이 없어 끝낸다.
    #[cfg(windows)]
    #[test]
    fn a_directory_symlink_is_not_taken_for_a_junction() {
        let scratch = Scratch::new();
        let target = scratch.target_with_file();
        let symlink = scratch.0.join("symlink");
        if let Err(e) = std::os::windows::fs::symlink_dir(&target, &symlink) {
            eprintln!("디렉터리 심볼릭 링크를 만들 수 없어 건너뜀: {e}");
            return;
        }
        let e = unlink_dir(&symlink).expect_err("정션이 아니면 거절");
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput);
        assert!(
            std::fs::symlink_metadata(&symlink).is_ok(),
            "심볼릭 링크는 그대로"
        );
    }
}
