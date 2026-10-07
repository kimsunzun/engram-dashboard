//! 데이터 폴더 — 루트 찾기 · 셸 쪽 배치([`DataLayout`]).
//!
//! ★루트 규칙 · `daemon.json` 자리 · `logs\` 는 데몬 `data_dir` 의 사본이다★ — 셸은 데몬 crate 를 운영 의존하지
//! 않으므로 따로 갖고, 두 벌이 같은 경로를 내는지는 같은 경로 시험이 묶는다(ADR-0271 결정 4). 규칙의 설명 정본은
//! 데몬이다 — 고치면 두 벌을 같은 변경에서 고친다.
//!
//! ★셸 폴더(`shell\…` · `webview\`)의 이름은 여기가 정본이고 디스크 계약이다★ — 바꾸면 기존 폴더의 데이터는
//! 읽히지 않고 남는다. 옮겨 주는 코드는 없다(ADR-0264 — 옛 데이터는 버린다).
// ADR-0271
// ADR-0282
// ADR-0264

use std::path::{Path, PathBuf};

const DATA_DIR_ENV: &str = "ENGRAM_DATA_DIR";

// ADR-0029: debug 분기(walk-up `.engram-dev`)와 그 단위테스트에서만 쓰인다 — release
// default_data_dir 은 exe 옆 `data` 만 쓰므로 release 비-test 빌드에선 dead_code.
// ADR-0264
#[cfg_attr(not(debug_assertions), allow(dead_code))]
const LOCAL_DATA_DIR: &str = ".engram-dev";

const RELEASE_DATA_DIR: &str = "data";

/// 데몬이 쓰고 셸·스크립트가 읽는 접속·잠금 파일 이름(ADR-0135 — 잠금 파일 = 접속 파일).
pub(crate) const DAEMON_FILE: &str = "daemon.json";

const DAEMON_DIR: &str = "daemon";
const SHELL_DIR: &str = "shell";
/// 사용자가 고치는 설정.
const CONFIG_DIR: &str = "config";
/// 프로그램이 쓰고 다음 실행에 다시 읽는 것(화면 상태).
const STATE_DIR: &str = "state";
/// 버려도 되는 것(잠금+접속 파일 · 토큰을 담은 스폰 부착 파일 · 임시 폴더).
const RUN_DIR: &str = "run";
const WEBVIEW_DIR: &str = "webview";
const LOGS_DIR: &str = "logs";

/// engram 프로세스의 데이터 디렉토리 — 데몬 `data_dir::default_data_dir` 의 사본(우선순위 · 분기 설명은 거기).
pub fn default_data_dir() -> PathBuf {
    if let Some(path) = data_dir_env_override() {
        return path;
    }

    #[cfg(debug_assertions)]
    {
        // ★왜 exe-기준 walk-up 인가★: 데몬은 WMI Win32_Process.Create 로 떠 **부모의 cwd 를 상속하지
        // 않는다**(WmiPrvSE 자식) — cwd 는 신뢰할 수 없다. 반면 exe 경로는 신뢰 가능하고, 개발 빌드
        // 산출물은 같은 repo 의 target/ 아래라 어느 exe 에서 올라가도 같은 repo 루트로 수렴한다.
        if let Ok(exe) = std::env::current_exe() {
            if let Some(root) = find_workspace_root(&exe) {
                return root.join(LOCAL_DATA_DIR);
            }
            if let Some(dir) = exe.parent() {
                return dir.join(LOCAL_DATA_DIR);
            }
        }
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(LOCAL_DATA_DIR)
    }

    #[cfg(not(debug_assertions))]
    {
        // exe 경로는 WMI spawn(부모 cwd 미상속) 아래서도 신뢰 가능한 유일한 기준점이다 — 디버그
        // 분기의 walk-up 이 exe 에서 출발하는 것과 같은 이유.
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                return release_data_dir(dir);
            }
        }
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(RELEASE_DATA_DIR)
    }
}

/// `ENGRAM_DATA_DIR` override 가 **활성**이면 그 경로. 빈 값은 미설정과 같다.
///
/// ★단일 출처★: "override 가 켜져 있나"를 묻는 곳이 둘 이상이라 여기로 모은다 — 판정이 갈리면
/// 한쪽은 데몬이 안 쓸 폴더를 보게 된다(`ensure_daemon` 의 사전 점검이 그 자리다).
pub(crate) fn data_dir_env_override() -> Option<PathBuf> {
    let val = std::env::var_os(DATA_DIR_ENV)?;
    if val.is_empty() {
        return None;
    }
    Some(PathBuf::from(val))
}

/// 릴리스 데이터 폴더 = `<exe 폴더>/data`(ADR-0134 결정 1·2).
///
/// ★cfg 를 걸지 않는다(load-bearing)★: 호출부인 [`default_data_dir`] 의 릴리즈 분기는
/// `not(debug_assertions)` 아래에 있고 **테스트는 항상 debug 로 돈다** — 이 함수까지 cfg 로 가리면
/// 릴리스 규칙을 단언할 수단이 사라진다.
pub fn release_data_dir(exe_dir: &Path) -> PathBuf {
    exe_dir.join(RELEASE_DATA_DIR)
}

// 둘 다 디버그 분기만 부른다 — 위 `LOCAL_DATA_DIR` 과 같은 사유(데몬 판은 설치 위치도 불러 이 속성이 없다).
#[cfg_attr(not(debug_assertions), allow(dead_code))]
fn find_workspace_root(start: &Path) -> Option<PathBuf> {
    let mut cur: Option<&Path> = if start.is_dir() {
        Some(start)
    } else {
        start.parent()
    };
    while let Some(dir) = cur {
        if is_workspace_root(dir) {
            return Some(dir.to_path_buf());
        }
        cur = dir.parent();
    }
    None
}

#[cfg_attr(not(debug_assertions), allow(dead_code))]
fn is_workspace_root(dir: &Path) -> bool {
    if dir.join(".git").exists() {
        return true;
    }
    let cargo = dir.join("Cargo.toml");
    match std::fs::read_to_string(&cargo) {
        // 주석에 박힌 `[workspace]` 문자열 같은 극단 케이스는 무시 — repo 루트는 .git 으로도 잡힌다.
        Ok(s) => s.contains("[workspace]"),
        Err(_) => false,
    }
}

/// 한 데이터 폴더(`root`)의 셸 쪽 배치. 경로 계산만 하고 디스크를 보지 않는다.
///
/// ★데몬 폴더는 `daemon.json` 자리(사본)만 안다★ — 나머지는 데몬 `data_dir::DataLayout` 이 갖고(ADR-0271 결정 4),
/// 셸은 데몬 폴더를 만들지 않는다.
// ADR-0264
// ADR-0271
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataLayout {
    root: PathBuf,
}

impl DataLayout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// [`default_data_dir`] 위의 배치 — 셸 운영 진입점.
    pub fn resolve() -> Self {
        Self::new(default_data_dir())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 데몬의 잠금이자 접속 파일 — 클라이언트가 읽는 레코드는 여기에만 발행된다.
    pub fn daemon_file(&self) -> PathBuf {
        self.root.join(DAEMON_DIR).join(RUN_DIR).join(DAEMON_FILE)
    }

    pub fn shell_config_dir(&self) -> PathBuf {
        self.root.join(SHELL_DIR).join(CONFIG_DIR)
    }

    pub fn shell_state_dir(&self) -> PathBuf {
        self.root.join(SHELL_DIR).join(STATE_DIR)
    }

    pub fn shell_run_dir(&self) -> PathBuf {
        self.root.join(SHELL_DIR).join(RUN_DIR)
    }

    pub fn webview_dir(&self) -> PathBuf {
        self.root.join(WEBVIEW_DIR)
    }

    /// 데몬 · 셸이 함께 쓰는 로그 폴더(파일은 프로세스마다 따로 — ADR-0264 결정 2).
    pub fn logs_dir(&self) -> PathBuf {
        self.root.join(LOGS_DIR)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    use engram_dashboard_daemon::data_dir as daemon;

    #[test]
    fn every_shell_path_hangs_off_the_root_in_its_component_and_kind() {
        let root = PathBuf::from("R");
        let l = DataLayout::new(&root);
        assert_eq!(l.root(), root.as_path());
        assert_eq!(
            l.daemon_file(),
            root.join("daemon").join("run").join("daemon.json")
        );
        assert_eq!(l.shell_config_dir(), root.join("shell").join("config"));
        assert_eq!(l.shell_state_dir(), root.join("shell").join("state"));
        assert_eq!(l.shell_run_dir(), root.join("shell").join("run"));
        assert_eq!(l.webview_dir(), root.join("webview"));
        assert_eq!(l.logs_dir(), root.join("logs"));
    }

    // ── 데몬 정본과 같은 경로(ADR-0271 결정 4) ─────────────────────────────────────────
    // 셸 → (dev) 데몬 간선에 기댄다 — 그 간선을 걷는 날(작업 순서 2-4 이후) 이 시험의 자리를 다시 정한다. 늘 debug 로
    // 돌아 루트 규칙의 release 분기 본문은 대조하지 못하고(도우미 `release_data_dir` 만 잰다), walk-up 표지 하나를
    // 빠뜨린 사본도 못 잡는다 — 이 저장소 루트엔 표지 둘이 다 있어 같은 루트가 나온다(표지 시험은 데몬에만 있다).
    // ADR-0271
    // ADR-0282

    /// env 이름을 상수가 아니라 리터럴로 쓴다 — 어느 쪽 상수의 철자가 틀려도 두 벌이 갈려 여기서 잡힌다.
    /// `_guard` = `DATA_DIR_ENV_LOCK` 의 가드 — 프로세스 env 를 바꾸므로 그 잠금 밖에서 부르지 않는다.
    fn both_data_dirs_with_env(
        _guard: &std::sync::MutexGuard<'_, ()>,
        val: Option<&OsStr>,
    ) -> (PathBuf, PathBuf) {
        let prev = std::env::var_os("ENGRAM_DATA_DIR");
        match val {
            Some(v) => std::env::set_var("ENGRAM_DATA_DIR", v),
            None => std::env::remove_var("ENGRAM_DATA_DIR"),
        }
        let ours = default_data_dir();
        let theirs = daemon::default_data_dir();
        match &prev {
            Some(v) => std::env::set_var("ENGRAM_DATA_DIR", v),
            None => std::env::remove_var("ENGRAM_DATA_DIR"),
        }
        (ours, theirs)
    }

    #[test]
    fn the_shell_copies_resolve_the_paths_the_daemon_writes() {
        let guard = crate::discovery::DATA_DIR_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = std::env::temp_dir().join("engram-shell-same-path-test");

        let (unset_shell, unset_daemon) = both_data_dirs_with_env(&guard, None);
        let (empty_shell, empty_daemon) = both_data_dirs_with_env(&guard, Some(OsStr::new("")));
        let (set_shell, set_daemon) = both_data_dirs_with_env(&guard, Some(tmp.as_os_str()));

        // 전제 — 세 상태가 실제로 다른 갈래를 탔다. 환경을 못 바꿨으면 두 벌이 같아도 아무것도 재지 않는다.
        assert!(
            unset_daemon.ends_with(".engram-dev"),
            "env 없음: 데몬이 debug 기본 폴더여야 {unset_daemon:?}"
        );
        assert!(
            empty_daemon.ends_with(".engram-dev"),
            "env 빈 값: 미설정과 같아야 {empty_daemon:?}"
        );
        assert_eq!(set_daemon, tmp, "env 임시 경로: 데몬이 그 경로 그대로여야");

        for (label, shell_root, daemon_root) in [
            ("env 없음", &unset_shell, &unset_daemon),
            ("env 빈 값", &empty_shell, &empty_daemon),
            ("env 임시 경로", &set_shell, &set_daemon),
        ] {
            assert_eq!(shell_root, daemon_root, "{label}: 루트가 같아야");
            let ours = DataLayout::new(daemon_root);
            let theirs = daemon::DataLayout::new(daemon_root);
            assert_eq!(
                ours.daemon_file(),
                theirs.daemon_file(),
                "{label}: daemon.json 자리가 같아야"
            );
            assert_eq!(
                ours.logs_dir(),
                theirs.logs_dir(),
                "{label}: 로그 폴더가 같아야"
            );
        }

        let exe_dir = Path::new("C:\\portable\\engram");
        assert_eq!(
            release_data_dir(exe_dir),
            daemon::release_data_dir(exe_dir),
            "릴리스 데이터 폴더 규칙이 같아야"
        );
    }
}
