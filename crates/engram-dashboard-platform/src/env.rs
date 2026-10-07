//! 프로세스 환경의 OS 규칙 — 홈 디렉터리 · 실행 파일 이름 · 지금 실행 파일 옆 실행 파일 경로 · 환경변수
//! 이름 비교. std 만 쓴다.

// ADR-0230

use std::path::PathBuf;

/// 사용자 홈 디렉터리 = 환경변수 하나 — Windows = `USERPROFILE`, 그 밖 = `HOME`. 그 변수가 없을 때만
/// `None` 이다(빈 값은 걸러내지 않는다).
///
/// ★`std::env::home_dir` 로 바꾸지 말 것★ — Windows 에서 그 변수가 없으면 OS API 로 넘어가 뜻이 달라진다.
/// Windows 에서 `HOME` 을 보지 않는 것도 지금 뜻 그대로다 — 둘 중 하나라도 바꾸면 홈 아래 폴더를 찾는
/// 부르는 쪽이 오류 없이 다른 폴더를 본다.
#[cfg(windows)]
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(PathBuf::from)
}
#[cfg(not(windows))]
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

pub fn exe_file_name(stem: &str) -> String {
    format!("{stem}{}", std::env::consts::EXE_SUFFIX)
}

/// 지금 도는 실행 파일과 같은 폴더의 `stem` 실행 파일 경로 — 파일 이름은 [`exe_file_name`] 이다.
///
/// ★있는지는 보지 않는다★ — 찾은 뒤 하는 일(로그 · 다음 후보 · 걸러 내기)이 부르는 쪽마다 다르다.
/// `None` 은 지금 실행 파일의 경로나 그 폴더를 얻지 못할 때뿐이다.
// ADR-0282
pub fn sibling_exe(stem: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join(exe_file_name(stem)))
}

/// 두 환경변수 이름이 같은 변수인가. Windows 의 환경변수 이름은 대소문자를 가리지 않는다(`Path` 와 `PATH` 는
/// 한 변수) — 여기선 ASCII 만 접는다. 그 밖의 OS 는 정확히 같아야 같다.
pub fn env_key_eq(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exe_file_name_appends_exe_only_on_windows() {
        let expected = if cfg!(windows) {
            "engram.exe"
        } else {
            "engram"
        };
        assert_eq!(exe_file_name("engram"), expected);
    }

    #[test]
    fn sibling_exe_is_the_named_exe_in_the_running_exe_folder() {
        let running = std::env::current_exe().expect("지금 실행 파일 경로");
        let want = running
            .parent()
            .expect("지금 실행 파일의 폴더")
            .join(exe_file_name("engram"));
        assert_eq!(sibling_exe("engram"), Some(want));
    }

    #[test]
    fn env_key_eq_folds_ascii_case_only_on_windows() {
        assert!(env_key_eq("PATH", "PATH"));
        assert_eq!(env_key_eq("Path", "PATH"), cfg!(windows));
        assert!(!env_key_eq("PATH", "PATHEXT"));
    }
}
