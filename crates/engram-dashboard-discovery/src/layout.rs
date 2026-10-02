//! 데이터 폴더 배치 — 디렉터리와 **둘 이상의 프로세스가 보는 파일**(`daemon.json`) 경로의 단일 출처.
//!
//! ★한 저장소만 쓰는 파일 이름은 여기 두지 않는다★: `agents.json` · `presets.json` ·
//! `usage_rejects.json` 은 각 저장소가 소유하고 받은 디렉터리 안에서만 붙인다. 이름까지 끌어오면
//! agent crate 가 이 crate 를 의존해야 한다.
//!
//! ★OS 가름은 [`crate::default_data_dir`] 안에만 있다★ — 여기는 `Path::join` 뿐이다(CLAUDE.md 「플랫폼
//! 중립」). [`DataLayout::webview_dir`] 도 모든 OS 에서 계산한다.
//!
//! ★이 이름들은 디스크 계약이다★ — 바꾸면 기존 폴더의 데이터는 읽히지 않고 남는다. 옮겨 주는 코드는 없다
//! (ADR-0264 — 옛 데이터는 버린다).

use std::path::{Path, PathBuf};

use crate::default_data_dir;

/// 데몬이 쓰고 셸·스크립트가 읽는 접속·잠금 파일 이름(ADR-0135 — 잠금 파일 = 접속 파일).
pub(crate) const DAEMON_FILE: &str = "daemon.json";

const DAEMON_DIR: &str = "daemon";
const SHELL_DIR: &str = "shell";
/// 사용자가 고치는 설정.
const CONFIG_DIR: &str = "config";
/// 프로그램이 쓰고 다음 실행에 다시 읽는 것(명부·프리셋·거절 기록 · 화면 상태).
const STATE_DIR: &str = "state";
/// 버려도 되는 것(잠금+접속 파일 · 토큰을 담은 스폰 부착 파일 · 임시 폴더).
const RUN_DIR: &str = "run";
const WEBVIEW_DIR: &str = "webview";
const LOGS_DIR: &str = "logs";
const MCP_CONFIG_DIR: &str = "mcp-config";
const USAGE_PROBE_DIR: &str = "usage-probe";

/// 한 데이터 폴더(`root`)의 배치. 경로 계산만 하고 디스크를 보지 않는다 — 예외는
/// [`ensure_daemon_dirs`](Self::ensure_daemon_dirs) 하나다.
// ADR-0264
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataLayout {
    root: PathBuf,
}

impl DataLayout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// [`default_data_dir`] 위의 배치 — 데몬·셸 운영 진입점.
    pub fn resolve() -> Self {
        Self::new(default_data_dir())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn daemon_state_dir(&self) -> PathBuf {
        self.root.join(DAEMON_DIR).join(STATE_DIR)
    }

    pub fn daemon_run_dir(&self) -> PathBuf {
        self.root.join(DAEMON_DIR).join(RUN_DIR)
    }

    /// 데몬의 잠금이자 접속 파일 — 클라이언트가 읽는 레코드는 여기에만 발행된다.
    pub fn daemon_file(&self) -> PathBuf {
        self.daemon_run_dir().join(DAEMON_FILE)
    }

    pub fn mcp_config_dir(&self) -> PathBuf {
        self.daemon_run_dir().join(MCP_CONFIG_DIR)
    }

    pub fn usage_probe_dir(&self) -> PathBuf {
        self.daemon_run_dir().join(USAGE_PROBE_DIR)
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

    pub fn logs_dir(&self) -> PathBuf {
        self.root.join(LOGS_DIR)
    }

    /// 데몬의 `state` · `run` 폴더를 만든다(이미 있으면 그대로). 데몬 전용 — 셸은 데몬 폴더를 만들지 않는다.
    pub fn ensure_daemon_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.daemon_state_dir())?;
        std::fs::create_dir_all(self.daemon_run_dir())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_path_hangs_off_the_root_in_its_component_and_kind() {
        let root = PathBuf::from("R");
        let l = DataLayout::new(&root);
        assert_eq!(l.root(), root.as_path());
        assert_eq!(l.daemon_state_dir(), root.join("daemon").join("state"));
        assert_eq!(l.daemon_run_dir(), root.join("daemon").join("run"));
        assert_eq!(
            l.daemon_file(),
            root.join("daemon").join("run").join("daemon.json")
        );
        assert_eq!(
            l.mcp_config_dir(),
            root.join("daemon").join("run").join("mcp-config")
        );
        assert_eq!(
            l.usage_probe_dir(),
            root.join("daemon").join("run").join("usage-probe")
        );
        assert_eq!(l.shell_config_dir(), root.join("shell").join("config"));
        assert_eq!(l.shell_state_dir(), root.join("shell").join("state"));
        assert_eq!(l.shell_run_dir(), root.join("shell").join("run"));
        assert_eq!(l.webview_dir(), root.join("webview"));
        assert_eq!(l.logs_dir(), root.join("logs"));
    }

    #[test]
    fn ensure_daemon_dirs_creates_state_and_run_and_is_idempotent() {
        let root = std::env::temp_dir().join(format!(
            "engram-layout-ensure-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let l = DataLayout::new(&root);
        l.ensure_daemon_dirs().expect("첫 생성");
        l.ensure_daemon_dirs().expect("이미 있어도 성공");
        let made = (l.daemon_state_dir().is_dir(), l.daemon_run_dir().is_dir());
        let shell_made = l.root().join(SHELL_DIR).exists();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(made, (true, true));
        assert!(!shell_made, "데몬 폴더만 만든다");
    }
}
