//! 파일 열기의 OS 규칙 — 남의 쓰기를 막은 채 여는 열기와, 그 열기가 실패한 까닭의 분류. std 만 쓴다 —
//! `OpenOptionsExt::share_mode` 가 그대로 `CreateFileW` 의 공유 모드로 내려가 같은 커널 동작을 unsafe 없이
//! 얻으므로 `windows` 바인딩이 필요 없다.

// ADR-0266

use std::fs::File;
use std::io;
use std::path::Path;

const ERROR_ACCESS_DENIED: i32 = 5;
const ERROR_SHARING_VIOLATION: i32 = 32;

/// 읽기+쓰기로 열고, 없으면 만든다. 있던 내용은 자르지 않는다.
///
/// 이 핸들이 열려 있는 동안 다른 열기는 **읽기만** 공유받는다 — 쓰기 · 삭제(이름 변경 포함)를 요청하는
/// 열기는 공유 위반([`is_sharing_violation`])으로 실패한다. 거꾸로, 남이 이미 이 파일을 쓰기 접근으로 열고
/// 있거나 쓰기 공유를 막고 열고 있으면 이 열기가 공유 위반으로 실패한다. 상대가 쓰기를 공유하며 여는
/// 읽기 열기(Rust `std::fs::read` 의 기본)는 이 핸들 곁에서 열린다. 읽기만 공유하며 여는 읽기 열기는 이
/// 핸들의 쓰기 접근과 부딪혀 공유 위반이다(실측 2026-08-14 — ADR-0135).
///
/// 읽기 전용 **속성**이 붙은 파일이면 공유 위반이 아니라 접근 거부([`is_access_denied`])로 실패한다.
///
/// Windows 밖 = 열기를 시도하지 않고 `ErrorKind::Unsupported` 로 실패한다 — 이 공유 제한을 거는 수단이
/// 없다. 그 오류에는 OS 오류 코드가 없다(`raw_os_error() == None`) — OS 가 열기를 시도하다 낸
/// `Unsupported` 와 이것으로 가른다.
#[cfg(windows)]
pub fn open_deny_write(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    // `FILE_SHARE_READ` 단독. `WRITE` 나 `DELETE` 를 더하면 남의 쓰기 열기 · 삭제가 그대로 성공해 이 함수가
    //   막는 것이 사라진다.
    const FILE_SHARE_READ: u32 = 1;
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .share_mode(FILE_SHARE_READ)
        .open(path)
}
#[cfg(not(windows))]
pub fn open_deny_write(_path: &Path) -> io::Result<File> {
    Err(io::ErrorKind::Unsupported.into())
}

/// 남이 쥔 핸들의 공유 제한에 막혔나(Windows `ERROR_SHARING_VIOLATION`). 그 핸들이 닫히면 같은 열기가
/// 성공할 수 있다. Windows 밖 = 늘 거짓.
///
/// std 는 이 코드를 `ErrorKind::Uncategorized` 로 분류해 `kind()` 로는 못 가른다(실측 2026-08-14).
pub fn is_sharing_violation(e: &io::Error) -> bool {
    cfg!(windows) && e.raw_os_error() == Some(ERROR_SHARING_VIOLATION)
}

/// 접근 자체가 거부됐나(Windows `ERROR_ACCESS_DENIED` — ACL 거부, 그리고 읽기 전용 속성이 붙은 파일을
/// 쓰기로 열 때). 공유 위반과 달리 남의 핸들이 닫혀도 풀리지 않는다. Windows 밖 = 늘 거짓 — 다른 OS 의
/// `PermissionDenied` 는 이것으로 치지 않는다.
pub fn is_access_denied(e: &io::Error) -> bool {
    cfg!(windows) && e.raw_os_error() == Some(ERROR_ACCESS_DENIED)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_codes_are_classified_only_on_windows() {
        let sharing = io::Error::from_raw_os_error(ERROR_SHARING_VIOLATION);
        let denied = io::Error::from_raw_os_error(ERROR_ACCESS_DENIED);
        assert_eq!(is_sharing_violation(&sharing), cfg!(windows));
        assert_eq!(is_access_denied(&denied), cfg!(windows));
        assert!(!is_sharing_violation(&denied));
        assert!(!is_access_denied(&sharing));
        // 코드 없는 같은 종류의 오류는 어느 쪽도 아니다 — 원시 코드로만 가른다.
        assert!(!is_access_denied(&io::ErrorKind::PermissionDenied.into()));
        assert!(!is_sharing_violation(&io::ErrorKind::Other.into()));
    }

    #[cfg(not(windows))]
    #[test]
    fn open_deny_write_refuses_without_an_os_code() {
        let e = open_deny_write(Path::new(".")).expect_err("수단이 없는 OS 는 열지 않는다");
        assert_eq!(e.kind(), io::ErrorKind::Unsupported);
        assert_eq!(e.raw_os_error(), None);
    }

    #[cfg(windows)]
    fn fresh_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "engram-platform-fs-{}-{}.tmp",
            std::process::id(),
            uuid::Uuid::new_v4()
        ))
    }

    #[cfg(windows)]
    #[test]
    fn it_creates_a_missing_file_and_keeps_existing_content() {
        let path = fresh_path();
        drop(open_deny_write(&path).expect("없는 파일은 만들어 연다"));
        assert!(path.exists());

        std::fs::write(&path, b"kept").expect("내용 선기록");
        drop(open_deny_write(&path).expect("있는 파일을 연다"));
        assert_eq!(
            std::fs::read(&path).expect("읽기"),
            b"kept",
            "자르면 안 된다"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[cfg(windows)]
    #[test]
    fn while_held_a_second_writer_is_refused_with_a_sharing_violation() {
        let path = fresh_path();
        let held = open_deny_write(&path).expect("첫 열기");

        let e = open_deny_write(&path).expect_err("두 번째 쓰기 열기는 막혀야");
        assert!(is_sharing_violation(&e), "공유 위반이어야: {e:?}");
        assert!(!is_access_denied(&e), "접근 거부로 읽히면 안 된다: {e:?}");

        drop(held);
        drop(open_deny_write(&path).expect("놓으면 다시 열린다"));
        let _ = std::fs::remove_file(&path);
    }

    #[cfg(windows)]
    #[test]
    fn while_held_a_permissive_reader_opens_but_delete_and_rename_are_refused() {
        let path = fresh_path();
        let held = open_deny_write(&path).expect("열기");

        assert!(std::fs::read(&path).is_ok(), "std 기본 읽기는 열려야");
        let e = std::fs::remove_file(&path).expect_err("삭제는 막혀야");
        assert!(is_sharing_violation(&e), "삭제 거부 = 공유 위반: {e:?}");
        let moved = path.with_extension("moved");
        let e = std::fs::rename(&path, &moved).expect_err("이름 변경도 막혀야");
        assert!(
            is_sharing_violation(&e),
            "이름 변경 거부 = 공유 위반: {e:?}"
        );

        drop(held);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&moved);
    }

    #[cfg(windows)]
    #[test]
    fn a_read_only_attribute_is_access_denied_not_a_sharing_violation() {
        let path = fresh_path();
        std::fs::write(&path, b"").expect("파일 생성");
        let mut perms = std::fs::metadata(&path).expect("메타").permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&path, perms.clone()).expect("읽기 전용 속성");

        let e = open_deny_write(&path).expect_err("읽기 전용 속성은 쓰기 열기를 막는다");
        assert!(is_access_denied(&e), "접근 거부여야: {e:?}");
        assert!(
            !is_sharing_violation(&e),
            "공유 위반으로 읽히면 안 된다: {e:?}"
        );

        perms.set_readonly(false);
        let _ = std::fs::set_permissions(&path, perms);
        let _ = std::fs::remove_file(&path);
    }
}
