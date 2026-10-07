//! 데이터 폴더 — 루트 찾기 · 폴더를 만드는 쓰기 확인 · 설치 위치 · 데몬 쪽 배치([`DataLayout`]).
//!
//! ★루트 규칙과 `daemon.json` · `logs\` 자리의 정본이 여기다★ — 셸은 데몬 crate 를 운영 의존하지 않으므로 그
//! 셋의 사본을 따로 갖고, 두 벌이 같은 경로를 내는지는 시험이 묶는다(ADR-0271 결정 4). 규칙을 고치면 셸 사본도
//! 같은 변경에서 고친다 — 갈리면 셸이 데몬이 쓰지 않는 폴더에서 `daemon.json` 을 찾는다.
//!
//! ★OS 가름은 [`default_data_dir`] 안에만 있다★ — 배치는 `Path::join` 뿐이다(CLAUDE.md 「플랫폼 중립」).
//!
//! ★배치의 이름들은 디스크 계약이다★ — 바꾸면 기존 폴더의 데이터는 읽히지 않고 남는다. 옮겨 주는 코드는 없다
//! (ADR-0264 — 옛 데이터는 버린다).
// ADR-0271
// ADR-0264

use std::path::{Path, PathBuf};

use engram_dashboard_base::writable::{probe_write_in, retry_if_vanished};
use engram_dashboard_net::portfile::DAEMON_FILE;

// ★ENGRAM_DATA_DIR override (테스트 격리 탈출구 — 배포 노브 아님)★:
//   - **유일한 용도 = 통합 테스트의 데이터 격리.** 실프로세스 통합 테스트(daemon `tests/ws_e2e.rs`)가
//     데몬을 임시 디렉토리로 보내 운영 `<repo>/.engram-dev` 오염을 막기 위함이다. 이 env 가 없으면
//     테스트 데몬이 운영 폴더에 daemon.json/agents.json 을 쓴다(오염).
//   - **배포용 경로 커스터마이즈 노브가 아니다.** 배포 단계의 데이터 위치는 실행 폴더 하위로
//     확정돼 있다(ADR-0134). 이 override 를 "사용자가 데이터 폴더를 바꾸는 수단"으로 쓰지 말 것.
//   - ★ADR-0134 이후 부수 효과★: 데이터 폴더가 곧 단일 인스턴스 스코프라, 이 env 만 갈라 주면
//     인스턴스도 함께 갈린다(따로 챙길 열쇠 변수가 없다).
//   - 부모 환경을 물려받아 뜬 데몬에만 먹는다 — WMI 로 뜬 데몬은 물려받지 않는다(그 한계는 띄우는 쪽인
//     셸이 적는다).
const DATA_DIR_ENV: &str = "ENGRAM_DATA_DIR";

// ADR-0029: debug 분기(walk-up `.engram-dev`)와 그 단위테스트에서만 쓰인다 — release
// default_data_dir 은 exe 옆 `data` 만 쓰므로 release 비-test 빌드에선 dead_code.
// ADR-0264
#[cfg_attr(not(debug_assertions), allow(dead_code))]
const LOCAL_DATA_DIR: &str = ".engram-dev";

/// ADR-0134 결정 2: 릴리스 데이터는 실행 폴더 **하위 한 폴더**에 모인다 — exe 옆에 흩어두면 배포
/// 파일과 섞여 새 버전 압축을 덮어쓸 때 사용자 데이터가 함께 날아간다.
///
/// ★`engram-` 접두사를 붙이지 마라(되살리지 마라)★: 이 폴더는 사용자가 이미 이름 붙인 배포 폴더
/// **안에** 있어 접두사가 부모 이름을 되풀이할 뿐이고, 형제인 `prompts/` 는 접두사가 없어 규칙도
/// 어긋난다. 위 [`LOCAL_DATA_DIR`] 과 이름이 다른 것은 실수가 아니다 — 디버그 쪽은 폴더가 많은 repo
/// 루트에 맨몸으로 놓이고 앞의 점이 가려 주므로 접두사가 값을 한다. 대칭을 맞추려 하지 말 것.
/// (사용자 결정 2026-08-14)
const RELEASE_DATA_DIR: &str = "data";

const DAEMON_DIR: &str = "daemon";
/// 프로그램이 쓰고 다음 실행에 다시 읽는 것(명부·프리셋·거절 기록).
const STATE_DIR: &str = "state";
/// 버려도 되는 것(잠금+접속 파일 · 토큰을 담은 스폰 부착 파일 · 임시 폴더).
const RUN_DIR: &str = "run";
const LOGS_DIR: &str = "logs";
const MCP_CONFIG_DIR: &str = "mcp-config";
const USAGE_PROBE_DIR: &str = "usage-probe";

/// engram 프로세스의 데이터 디렉토리(ADR-0024/0029).
///
/// 우선순위:
/// 1. **`ENGRAM_DATA_DIR`(설정+non-empty)** → 그 경로 그대로(테스트 격리 탈출구 — 배포 노브 아님).
/// 2. **디버그(`cfg!(debug_assertions)`)**: current_exe 에서 위로 올라가 repo 루트(`.git` 또는
///    `Cargo.toml` 의 `[workspace]`)를 찾아 `<root>/.engram-dev`. 루트 못 찾으면 exe 디렉토리
///    fallback, 그것도 안 되면 cwd. → 개발 한 곳에서 여러 빌드(app·daemon)가 한 폴더 공유.
/// 3. **릴리즈(`not(debug_assertions)`)**: **실행 파일 폴더 하위 `data/`**
///    ([`release_data_dir`]). 배포판 폴더를 지우면 흔적이 남지 않는다 — 완전 포터블(ADR-0134 결정 1).
///
/// 어느 경로든 **절대 패닉하지 않는다**(배포·루트 미발견 상황에서도 PathBuf 를 반드시 반환).
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
fn data_dir_env_override() -> Option<PathBuf> {
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

/// 데이터 폴더를 쓸 수 없다 — 원인이 무엇이든 이것 하나로 접는다. 사용자가 할 일은 "쓸 수 있는 곳에
/// 풀기" 하나다.
///
/// ★메시지에 조치가 들어 있다(ADR-0134 결정 4)★: 데몬은 사용자에게 보일 화면이 없어 기동 실패 로그의 이
/// 문구가 남는 전부다. 셸의 사전 점검이 같은 실패를 같은 문구로 프론트까지 올린다(셸 쪽 오류 타입).
#[derive(Debug, thiserror::Error)]
#[error("데이터 폴더에 쓸 수 없음({path}): {reason} — 쓰기 가능한 위치에 압축을 풀어 주세요")]
pub struct DataDirUnwritable {
    path: String,
    reason: String,
}

/// 데이터 폴더를 **만들고** 실제로 쓸 수 있는지 확인한다(ADR-0134 결정 4 — 폴백 없음).
///
/// ★폴더를 소유하는 쪽(데몬) 전용★: 폴더를 생성하는 부수효과가 있다. 아직 그 폴더를 쓸지 확정하지
/// 않은 쪽(셸의 사전 점검)은 만들지 않고 되돌리는 자기 확인을 쓴다(ADR-0271 결정 3).
///
/// 존재 검사만으로는 부족하다: 권한 없는 위치(`C:\Program Files` 하위 등)는 폴더가 이미 있어도 첫
/// 쓰기에서 막힌다.
pub fn ensure_data_dir_writable(dir: &Path) -> Result<(), DataDirUnwritable> {
    retry_if_vanished(|| {
        std::fs::create_dir_all(dir)?;
        probe_write_in(dir)
    })
    .map_err(|e| DataDirUnwritable {
        path: dir.display().to_string(),
        reason: e.to_string(),
    })
}

/// exe 기준으로 해석한 **설치/repo 루트 절대경로**(빌드모드 무관). 데몬이 상대 리소스(예: ADR-0092
/// 프라이밍 `prompts/agent-priming.md`)를 붙일 **신뢰 가능한 base** 로 쓴다.
///
/// 해석(default_data_dir 의 디버그/릴리즈 분기와 동형):
///   - exe 에서 위로 올라가 workspace 루트(`.git` 또는 `Cargo.toml [workspace]`)를 찾으면 그것
///     (개발: `<repo>` — target/debug 어느 exe 에서 올라가도 repo 루트로 수렴).
///   - 루트 못 찾으면 exe 디렉토리(릴리즈: 번들 exe 들이 co-located 되는 폴더 — 리소스도 동거).
///   - exe 조차 못 얻으면 None(호출자가 처리 — 프라이밍은 그때 None 을 산출).
/// ★절대경로 보장★: current_exe 는 절대경로를 주고 walk-up/parent 는 그 절대성을 보존한다.
pub fn find_install_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    if let Some(root) = find_workspace_root(&exe) {
        return Some(root);
    }
    exe.parent().map(|p| p.to_path_buf())
}

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

/// 한 데이터 폴더(`root`)의 데몬 쪽 배치. 경로 계산만 하고 디스크를 보지 않는다 — 예외는
/// [`ensure_daemon_dirs`](Self::ensure_daemon_dirs) 하나다.
///
/// ★셸 폴더(`shell\…` · `webview\`)는 여기 없다★ — 셸이 자기 배치로 갖는다(ADR-0271 결정 4).
///
/// ★한 저장소만 쓰는 파일 이름은 여기 두지 않는다★: `agents.json` · `presets.json` ·
/// `usage_rejects.json` 은 각 저장소가 소유하고 받은 디렉터리 안에서만 붙인다. 이름까지 여기 두면 그
/// 저장소(agent crate)가 데몬을 알아야 한다.
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

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn daemon_state_dir(&self) -> PathBuf {
        self.root.join(DAEMON_DIR).join(STATE_DIR)
    }

    pub fn daemon_run_dir(&self) -> PathBuf {
        self.root.join(DAEMON_DIR).join(RUN_DIR)
    }

    /// 데몬의 잠금이자 접속 파일(ADR-0135) — 클라이언트가 읽는 레코드는 여기에만 발행된다. 셸과 스크립트
    /// (`scripts/engram.mjs`)가 같은 자리를 따로 안다.
    pub fn daemon_file(&self) -> PathBuf {
        self.daemon_run_dir().join(DAEMON_FILE)
    }

    pub fn mcp_config_dir(&self) -> PathBuf {
        self.daemon_run_dir().join(MCP_CONFIG_DIR)
    }

    pub fn usage_probe_dir(&self) -> PathBuf {
        self.daemon_run_dir().join(USAGE_PROBE_DIR)
    }

    /// 데몬 · 셸이 함께 쓰는 로그 폴더(파일은 프로세스마다 따로 — ADR-0264 결정 2). 셸도 사본을 갖는다.
    pub fn logs_dir(&self) -> PathBuf {
        self.root.join(LOGS_DIR)
    }

    /// 데몬의 `state` · `run` 폴더를 만든다(이미 있으면 그대로).
    pub fn ensure_daemon_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.daemon_state_dir())?;
        std::fs::create_dir_all(self.daemon_run_dir())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── data_dir resolver ─────────────────────────────────────────────────────────────

    /// `ENGRAM_DATA_DIR` 은 프로세스 전역 env 라, 이걸 만지는 테스트끼리 병렬로 돌면 서로 set/remove 를
    /// 짓밟는다. ★이 시험 바이너리에서 그 env 를 바꾸는 시험은 전부 이 락 하나를 쥔다★ — 락이 둘이면 서로를
    /// 막지 못한다.
    // ADR-0282
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn data_dir_env_override_returns_path_verbatim() {
        let _g = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let prev = std::env::var_os(DATA_DIR_ENV);
        let want = std::env::temp_dir().join("engram-override-data-dir-test");
        std::env::set_var(DATA_DIR_ENV, &want);
        let got = default_data_dir();
        // 단언 전에 복원해 단언 실패에도 env 가 leak 되지 않게 한다.
        match &prev {
            Some(v) => std::env::set_var(DATA_DIR_ENV, v),
            None => std::env::remove_var(DATA_DIR_ENV),
        }
        assert_eq!(got, want, "ENGRAM_DATA_DIR set 시 그 경로 그대로 반환");
    }

    #[test]
    fn data_dir_empty_env_falls_through_to_default() {
        // 테스트는 항상 debug 빌드라 walk-up `.engram-dev` 분기를 탄다.
        let _g = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let prev = std::env::var_os(DATA_DIR_ENV);
        std::env::set_var(DATA_DIR_ENV, "");
        let got = default_data_dir();
        match &prev {
            Some(v) => std::env::set_var(DATA_DIR_ENV, v),
            None => std::env::remove_var(DATA_DIR_ENV),
        }
        assert!(
            got.ends_with(LOCAL_DATA_DIR),
            "빈 env 는 기본 분기로 통과 → `.engram-dev` 로 끝나야: {got:?}"
        );
    }

    #[test]
    fn default_data_dir_debug_is_local_data_dir() {
        let _g = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let prev = std::env::var_os(DATA_DIR_ENV);
        std::env::remove_var(DATA_DIR_ENV);
        let got = default_data_dir();
        if let Some(v) = &prev {
            std::env::set_var(DATA_DIR_ENV, v);
        }
        assert!(
            got.ends_with(LOCAL_DATA_DIR),
            "debug 는 폴더-로컬 `.engram-dev` 로 끝나야: {got:?}"
        );
    }

    /// release 분기는 `not(debug_assertions)` 라 테스트(항상 debug)가 직접 못 탄다 — 그래서 규칙을
    /// 담은 순수 헬퍼를 cfg 없이 두고 여기서 단언한다(ADR-0134).
    #[test]
    fn release_data_dir_is_exe_adjacent() {
        let exe_dir = Path::new("C:\\portable\\engram");
        assert_eq!(
            release_data_dir(exe_dir),
            exe_dir.join("data"),
            "릴리스 데이터 폴더는 exe 폴더 하위 data(`engram-` 접두사 없음 — 상수 주석 참조)"
        );
    }

    // ── discovery 사본과의 다리(ADR-0271 결정 1) ──────────────────────────────────────
    // 두 벌(이 정본 · 아직 남은 discovery crate)을 묶는다. 다리는 discovery crate 를 지울 때 함께
    // 지우고, 남는 확인은 셸 사본의 같은-경로 시험이다. 시험은 늘 debug 로 돌아 루트 규칙의
    // release(`not(debug_assertions)`) 분기는 비교하지 못한다 — release 1회 실측이 덮는다
    // (`docs/process/S21-crate-boundaries/trd-2-2-discovery-split.md` §5-3).
    // ADR-0282

    /// env 이름을 상수가 아니라 리터럴로 쓴다 — 데몬 상수의 철자가 틀리면 두 벌이 갈려 여기서 잡힌다.
    fn both_data_dirs_with_env(val: Option<&std::ffi::OsStr>) -> (PathBuf, PathBuf) {
        let prev = std::env::var_os("ENGRAM_DATA_DIR");
        match val {
            Some(v) => std::env::set_var("ENGRAM_DATA_DIR", v),
            None => std::env::remove_var("ENGRAM_DATA_DIR"),
        }
        let dir = crate::resolve_data_dir();
        let copy = engram_dashboard_discovery::default_data_dir();
        match &prev {
            Some(v) => std::env::set_var("ENGRAM_DATA_DIR", v),
            None => std::env::remove_var("ENGRAM_DATA_DIR"),
        }
        (dir, copy)
    }

    // ADR-0282
    #[test]
    fn resolve_data_dir_matches_the_discovery_copy() {
        let _g = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (dir, copy) = both_data_dirs_with_env(None);
        assert!(
            dir.ends_with(".engram-dev"),
            "디버그(override 없음)에서 `.engram-dev` 로 끝나야(app 과 동일 폴더): {dir:?}"
        );
        assert_eq!(dir, copy, "env 미설정: 두 벌이 같은 폴더여야");

        let tmp = std::env::temp_dir().join("engram-daemon-bridge-override-test");
        let (dir, copy) = both_data_dirs_with_env(Some(tmp.as_os_str()));
        assert_eq!(dir, tmp, "env 설정: 데몬이 그 경로 그대로여야");
        assert_eq!(dir, copy, "env 설정: 두 벌이 같은 폴더여야");

        let (dir, copy) = both_data_dirs_with_env(Some(std::ffi::OsStr::new("")));
        assert_eq!(dir, copy, "env 빈 값: 두 벌이 같은 폴더여야");
    }

    // ADR-0282
    #[test]
    fn release_rule_install_root_and_layout_match_the_discovery_copy() {
        let exe_dir = Path::new("C:\\portable\\engram");
        assert_eq!(
            release_data_dir(exe_dir),
            engram_dashboard_discovery::release_data_dir(exe_dir)
        );
        assert_eq!(
            find_install_root(),
            engram_dashboard_discovery::find_install_root()
        );

        let root = PathBuf::from("R");
        let ours = DataLayout::new(&root);
        let theirs = engram_dashboard_discovery::DataLayout::new(&root);
        assert_eq!(ours.daemon_state_dir(), theirs.daemon_state_dir());
        assert_eq!(ours.daemon_run_dir(), theirs.daemon_run_dir());
        assert_eq!(ours.daemon_file(), theirs.daemon_file());
        assert_eq!(ours.mcp_config_dir(), theirs.mcp_config_dir());
        assert_eq!(ours.usage_probe_dir(), theirs.usage_probe_dir());
        assert_eq!(ours.logs_dir(), theirs.logs_dir());
    }

    #[test]
    fn resolve_data_dir_honors_env_override() {
        let _g = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let prev = std::env::var_os(DATA_DIR_ENV);
        let want = std::env::temp_dir().join("engram-daemon-resolve-override-test");
        std::env::set_var(DATA_DIR_ENV, &want);
        let got = crate::resolve_data_dir();
        match &prev {
            Some(v) => std::env::set_var(DATA_DIR_ENV, v),
            None => std::env::remove_var(DATA_DIR_ENV),
        }
        assert_eq!(got, want, "ENGRAM_DATA_DIR set 시 그 경로로 격리돼야");
    }

    // ── 데이터 폴더 쓰기 가능 프로브(ADR-0134 결정 4) ─────────────────────────────────

    /// 프로브 전용 유니크 폴더(테스트 병렬 실행에서 서로 밟지 않게).
    fn fresh_probe_dir(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "engram-writable-probe-{tag}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// 폴더 안에 프로브 잔여물이 하나라도 있나(이름이 호출마다 달라 접두사로 센다).
    fn probe_leftovers(dir: &Path) -> usize {
        use engram_dashboard_base::writable::WRITE_PROBE_PREFIX;
        std::fs::read_dir(dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| {
                        e.file_name()
                            .to_string_lossy()
                            .starts_with(WRITE_PROBE_PREFIX)
                    })
                    .count()
            })
            .unwrap_or(0)
    }

    #[test]
    fn writable_probe_accepts_temp_dir_and_leaves_nothing() {
        let dir = fresh_probe_dir("ok");
        ensure_data_dir_writable(&dir).expect("temp 하위는 쓸 수 있어야");
        let left = probe_leftovers(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(left, 0, "프로브 파일을 남기면 안 됨");
    }

    #[test]
    fn writable_probe_rejects_child_of_a_file() {
        // 파일의 자식 경로는 만들 수 없다 — 폴더 생성 자체가 막히는 경우의 대표.
        let file = fresh_probe_dir("blocker");
        std::fs::create_dir_all(file.parent().unwrap()).ok();
        std::fs::write(&file, b"x").expect("blocker 파일 생성");
        let err = ensure_data_dir_writable(&file.join("child")).unwrap_err();
        let _ = std::fs::remove_file(&file);
        assert!(
            err.to_string().contains("쓰기 가능한 위치"),
            "사용자가 할 일이 메시지에 있어야: {err}"
        );
    }

    // ── find_workspace_root / is_workspace_root (임시 디렉토리 트리, 빌드모드 무관) ──────

    fn unique_tmp(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "engram-ws-root-{tag}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn find_workspace_root_detects_git_marker_walking_up() {
        let root = unique_tmp("git");
        let deep = root.join("a").join("b").join("c");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();

        let got = find_workspace_root(&deep).expect(".git 마커를 위로 올라가며 찾아야");
        // 임시 디렉토리는 심볼릭(예: macOS /var→/private) 일 수 있어 canonicalize 후 비교.
        assert_eq!(
            std::fs::canonicalize(&got).unwrap(),
            std::fs::canonicalize(&root).unwrap()
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn find_workspace_root_detects_cargo_workspace_marker() {
        let root = unique_tmp("cargo");
        let sub = root.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(root.join("Cargo.toml"), b"[workspace]\nmembers = [\"x\"]\n").unwrap();

        let got = find_workspace_root(&sub).expect("[workspace] Cargo.toml 을 찾아야");
        assert_eq!(
            std::fs::canonicalize(&got).unwrap(),
            std::fs::canonicalize(&root).unwrap()
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn find_workspace_root_none_when_no_marker() {
        let root = unique_tmp("none");
        let deep = root.join("x").join("y");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(root.join("Cargo.toml"), b"[package]\nname = \"z\"\n").unwrap();

        assert!(
            find_workspace_root(&deep).is_none(),
            "마커 없으면 None — [package] 단독은 workspace 루트가 아님"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn find_install_root_yields_absolute_path() {
        // 구체 경로는 실행 환경 의존이라 값은 단언하지 않고 절대성만 본다.
        let root = find_install_root().expect("테스트 실행 중이면 current_exe 존재 → Some");
        assert!(
            root.is_absolute(),
            "find_install_root 는 절대경로여야(cwd 불신 계약): {root:?}"
        );
    }

    #[test]
    fn is_workspace_root_distinguishes_markers() {
        let base = unique_tmp("is");
        let git_dir = base.join("g");
        std::fs::create_dir_all(git_dir.join(".git")).unwrap();
        assert!(is_workspace_root(&git_dir), ".git 존재 → true");

        let ws_dir = base.join("w");
        std::fs::create_dir_all(&ws_dir).unwrap();
        std::fs::write(ws_dir.join("Cargo.toml"), b"[workspace]\n").unwrap();
        assert!(is_workspace_root(&ws_dir), "[workspace] → true");

        let pkg_dir = base.join("p");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("Cargo.toml"), b"[package]\nname=\"q\"\n").unwrap();
        assert!(!is_workspace_root(&pkg_dir), "[package] 단독 → false");

        let empty_dir = base.join("e");
        std::fs::create_dir_all(&empty_dir).unwrap();
        assert!(!is_workspace_root(&empty_dir), "마커 없음 → false");

        let _ = std::fs::remove_dir_all(&base);
    }

    // ── DataLayout(데몬 쪽 배치) ──────────────────────────────────────────────────────

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
        let top: Vec<_> = std::fs::read_dir(&root)
            .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.file_name()).collect())
            .unwrap_or_default();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(made, (true, true));
        assert_eq!(
            top,
            vec![std::ffi::OsString::from("daemon")],
            "데몬 폴더만 만든다"
        );
    }
}
