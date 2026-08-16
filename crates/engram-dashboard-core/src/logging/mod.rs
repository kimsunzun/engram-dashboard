//! tracing 전역 초기화 + 프로세스 실행 1회분 파일 로그.
//!
//! 진입점은 둘이다: [`init_logging`](stdout 만) · [`init_logging_with_file`](stdout + 파일). 둘 다
//! 멱등이고, 프로세스당 먼저 부른 쪽이 이긴다.
//!
//! ★파일 sink 의 존재 이유 = 릴리즈에서 stdout 이 아무 데도 없다는 것★(실측):
//! 릴리즈 바이너리는 `windows_subsystem = "windows"` 라 콘솔이 없고, 데몬은 WMI
//! `Win32_Process.Create` 로 뜨는데 그 API 는 std 핸들을 넘길 수 없어 자식의 stdout/stderr 이
//! 어디에도 붙지 않는다(비-WMI 경로는 아예 null 로 지정한다). 데몬은 부모 환경도 물려받지 않아
//! `RUST_LOG` 도 닿지 않는다. 그래서 파일이 없으면 **기동 실패의 원인이 남는 곳이 한 군데도 없다.**
//!
//! ★로그 폴더는 호출자가 넘긴다 — 코어가 스스로 찾지 않는다★: 데이터 폴더 해석은
//! `engram-dashboard-discovery` 의 몫이고, 코어가 그걸 의존하면 「코어 격리」(ADR-0003)가 깨진다.
//! 코어가 소유하는 것은 그 폴더 아래의 배치 규약(`logs/<종류>-<UTC>-<pid>.log`)뿐이다.

use std::fs::File;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::{layer::SubscriberExt, reload, util::SubscriberInitExt, EnvFilter};

type FilterHandle = reload::Handle<EnvFilter, tracing_subscriber::Registry>;

static RELOAD_HANDLE: OnceLock<FilterHandle> = OnceLock::new();

static BEARER_RE: OnceLock<Regex> = OnceLock::new();
static KEY_RE: OnceLock<Regex> = OnceLock::new();

/// debug 로그 출력 전 민감 값(API키·Bearer 토큰)을 `***`로 치환 (T-1).
/// 기본 로그 레벨(warn)에서는 PTY 텍스트가 찍히지 않으나,
/// debug/trace 활성화 시 실수로 키가 노출되는 것을 방지한다.
///
/// ※ AWS Secret Access Key(40자 base64)는 패턴 식별불가로 미포함.
/// ※ generic api_key=/token= 형태는 오탐 리스크로 미포함.
pub fn mask_secrets(s: &str) -> String {
    let bearer =
        BEARER_RE.get_or_init(|| Regex::new(r"Bearer\s+\S{10,}").expect("bearer regex compile"));
    let keys = KEY_RE.get_or_init(|| {
        Regex::new(
            r"(?:sk-(?:proj-)?[A-Za-z0-9_\-]{20,}|AKIA[A-Z0-9]{16}|(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36}|github_pat_[A-Za-z0-9_]{20,}|AIza[0-9A-Za-z_\-]{35})",
        )
        .expect("key regex compile")
    });
    let step1 = bearer.replace_all(s, "Bearer ***");
    keys.replace_all(step1.as_ref(), "***").into_owned()
}

// ── 파일 sink ────────────────────────────────────────────────────────────────────

/// 로그 파일을 쓰는 프로세스의 종류. **파일 이름의 접두사이자 보존 정리의 단위**다.
///
/// ★자유 문자열이 아니라 닫힌 집합인 이유★: 두 프로세스가 같은 접두사를 고르면 같은 파일을 나눠
/// 쓰게 되고, Windows 에서 그건 곧 줄이 섞여 깨진 로그다. 새 프로세스가 생기면 여기 variant 를
/// 더해 **의식적으로** 다른 이름을 고르게 한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogKind {
    /// 에이전트 호스트 데몬(`engram-dashboard-daemon.exe`).
    Daemon,
    /// 데스크톱 클라이언트 셸(`engram-dashboard.exe`).
    App,
}

impl LogKind {
    fn label(self) -> &'static str {
        match self {
            Self::Daemon => "daemon",
            Self::App => "app",
        }
    }
}

/// 데이터 폴더 아래 로그가 모이는 하위 폴더 이름.
const LOG_SUBDIR: &str = "logs";

const LOG_EXT: &str = ".log";

/// 종류별로 남기는 파일 수 — **이번 실행분을 포함한** 상한이다(그래서 새 파일을 만들기 전에
/// `KEEP_PER_KIND - 1` 개만 남긴다).
const KEEP_PER_KIND: usize = 10;

/// [`Mutex`] 의 기본 [`MakeWriter`] 구현 대신 쓰는 파일 writer.
///
/// ★poison 을 무시하고 계속 쓴다★: 표준 구현은 poison 에서 panic 하는데, 이 sink 의 소비자에는
/// panic hook 이 있다. 한 번 poison 되면 그 hook 의 로그가 다시 panic 해(panic-in-panic → abort)
/// **정작 남겨야 할 죽음의 이유가 사라진다.** 잠금이 지키는 건 줄 섞임뿐이라 뒤엎어도 안전하다.
struct FileSink(Mutex<File>);

struct FileSinkWriter<'a>(MutexGuard<'a, File>);

impl<'a> MakeWriter<'a> for FileSink {
    type Writer = FileSinkWriter<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        FileSinkWriter(self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

impl io::Write for FileSinkWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// UTC 초 단위 타임스탬프 `YYYYMMDD-HHMMSS`. 자릿수가 고정이라 **사전순 = 시간순**이고, 보존 정리가
/// 그 성질에 기댄다.
fn utc_stamp(t: SystemTime) -> String {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (y, m, d) = civil_from_days(days);
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    format!("{y:04}{m:02}{d:02}-{hh:02}{mm:02}{ss:02}")
}

/// Unix epoch 이후 경과 일수 → (년, 월, 일). Howard Hinnant 의 `civil_from_days`(public domain
/// chrono 알고리즘)를 1970 이후 구간으로 좁힌 것.
///
/// ★날짜 crate 를 들이지 않으려고 손으로 둔다★: 코어의 의존은 이 파일명 하나 때문에 늘릴 값이
/// 아니다(로그 **줄**의 타임스탬프는 tracing-subscriber 가 이미 찍는다).
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 같은 종류의 오래된 파일을 지워 `keep` 개만 남긴다.
///
/// 삭제 실패는 삼킨다 — 다른 프로세스가 붙들고 있는 파일이 정상적으로 존재하고(같은 종류의 데몬이
/// 두 데이터 폴더에서 도는 등), 그 하나 때문에 기동이 멈추면 안 된다.
fn prune_old_logs(dir: &Path, kind: LogKind, keep: usize) {
    let prefix = format!("{}-", kind.label());
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with(&prefix) && n.ends_with(LOG_EXT))
        .collect();
    if names.len() <= keep {
        return;
    }
    names.sort_unstable();
    for stale in &names[..names.len() - keep] {
        let _ = std::fs::remove_file(dir.join(stale));
    }
}

/// `<data_dir>/logs/<종류>-<UTC>-<pid>.log` 를 새로 열고 그 경로와 핸들을 돌려준다.
///
/// pid 를 붙이는 이유는 같은 초에 두 번 뜬 실행이 한 파일을 나눠 쓰는 것을 막기 위해서다.
fn open_run_log(data_dir: &Path, kind: LogKind) -> io::Result<(PathBuf, File)> {
    let dir = data_dir.join(LOG_SUBDIR);
    std::fs::create_dir_all(&dir)?;
    prune_old_logs(&dir, kind, KEEP_PER_KIND.saturating_sub(1));

    let stamp = utc_stamp(SystemTime::now());
    let path = dir.join(format!(
        "{}-{stamp}-{}{LOG_EXT}",
        kind.label(),
        std::process::id()
    ));
    let mut file = File::options().create(true).append(true).open(&path)?;
    write_header(&mut file, kind, &stamp);
    Ok((path, file))
}

/// 파일 첫 줄에 이 실행의 신원을 박는다.
///
/// ★로그 이벤트가 아니라 파일 머리글인 이유★: 기본 레벨이 `warn` 이라 **정상 기동은 한 줄도 남기지
/// 않는다** — 그러면 남는 건 빈 파일이고, 언제 뜬 어느 바이너리의 것인지조차 알 수 없다. 그걸 메우려고
/// 기동 알림을 `info` → `warn` 으로 올리면 레벨의 의미가 무너지므로, 이벤트 평면 밖에 한 줄만 둔다.
fn write_header(file: &mut File, kind: LogKind, stamp: &str) {
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|e| format!("(exe 경로 불명: {e})"));
    // 실패해도 삼킨다 — 머리글 한 줄 때문에 프로세스 기동을 막을 이유가 없다.
    let _ = writeln!(
        file,
        "==== engram {kind} | {stamp} UTC | pid {pid} | {exe} ====",
        kind = kind.label(),
        pid = std::process::id()
    );
}

// ── 초기화 ───────────────────────────────────────────────────────────────────────

/// tracing-subscriber 전역 초기화(stdout 만). 앱 부팅 시 1회만 호출 (멱등 — 중복 호출 no-op).
/// 기본 레벨: RUST_LOG 환경변수 우선, 없으면 "warn" (릴리스 기본 OFF — 평상시 거의 무출력).
pub fn init_logging() {
    init_with_optional_file(None);
}

/// [`init_logging`] + `<data_dir>/logs/` 아래 이번 실행 전용 파일 sink.
///
/// 반환값 = 이 호출이 실제로 붙인 로그 파일 경로. `None` = 파일 sink 없음이고, 사유는 셋이다:
/// 이미 초기화됨(조용) · 파일 열기 실패(직후 `warn`) · 남이 먼저 깐 subscriber 라 layer 가 버려짐.
///
/// 레벨 필터·stdout 출력은 [`init_logging`] 과 동일하다 — 파일은 **더해질 뿐** 대체하지 않는다.
pub fn init_logging_with_file(data_dir: &Path, kind: LogKind) -> Option<PathBuf> {
    if RELOAD_HANDLE.get().is_some() {
        return None;
    }

    let opened = open_run_log(data_dir, kind);
    let (path, file) = match opened {
        Ok((p, f)) => (Some(p), Some(f)),
        Err(e) => {
            init_with_optional_file(None);
            tracing::warn!(
                dir = %data_dir.join(LOG_SUBDIR).display(),
                "파일 로그를 열지 못해 stdout 으로만 기록함: {e}"
            );
            return None;
        }
    };

    // 이미 남이 subscriber 를 깔아 둔 경우 우리 layer 는 버려진다 — 그때 경로를 돌려주면 "여기 쓰고
    //   있다"는 거짓말이 되고, 호출자가 그 경로를 로그로 광고한다.
    init_with_optional_file(file).then_some(path).flatten()
}

/// subscriber 를 실제로 설치했으면 `true`.
fn init_with_optional_file(file: Option<File>) -> bool {
    if RELOAD_HANDLE.get().is_some() {
        return false;
    }

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));

    let (filter_layer, handle) = reload::Layer::new(filter);

    // ★비동기 writer(`tracing_appender::non_blocking`)를 쓰지 마라(되살리지 마라)★: 데몬은
    //   `std::process::exit(code)` 로 끝나 소멸자를 건너뛴다. worker 스레드가 아직 안 비운 줄은
    //   그대로 사라지는데, 그 사라지는 줄이 바로 이 기능이 잡으려는 **기동 실패 직전의 마지막 줄**이다.
    //   `File` 은 버퍼가 없어 이벤트 한 줄이 곧 한 번의 write 다 — 프로세스가 죽기 전에 이미 디스크에 있다.
    // ANSI OFF: 파일을 사람이나 LLM 이 그대로 읽는다(색 이스케이프는 잡음).
    let file_layer = file.map(|f| {
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(FileSink(Mutex::new(f)))
    });

    // try_init: 다른 subscriber가 이미 설정된 경우(테스트 등) 무시
    let result = tracing_subscriber::registry()
        .with(filter_layer)
        .with(tracing_subscriber::fmt::layer())
        .with(file_layer)
        .try_init();

    if result.is_ok() {
        let _ = RELOAD_HANDLE.set(handle);
    }
    result.is_ok()
}

/// 런타임 로그 레벨 변경. 유효값: "trace"|"debug"|"info"|"warn"|"error"|"off".
pub fn set_log_level(level: &str) -> Result<(), String> {
    let handle = RELOAD_HANDLE
        .get()
        .ok_or_else(|| "logging not initialized".to_string())?;

    let filter =
        EnvFilter::try_new(level).map_err(|e| format!("invalid log level \"{level}\": {e}"))?;

    handle
        .reload(filter)
        .map_err(|e| format!("reload failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "engram-logtest-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    // ── 타임스탬프 ──
    #[test]
    fn utc_stamp_is_lexicographically_sortable_utc() {
        assert_eq!(utc_stamp(UNIX_EPOCH), "19700101-000000");
        assert_eq!(
            utc_stamp(UNIX_EPOCH + Duration::from_secs(1_755_000_000)),
            "20250812-120000"
        );
        // 윤일 — civil_from_days 의 400년 주기 분기.
        assert_eq!(
            utc_stamp(UNIX_EPOCH + Duration::from_secs(951_782_400)),
            "20000229-000000"
        );
        let early = utc_stamp(UNIX_EPOCH + Duration::from_secs(1_755_000_000));
        let late = utc_stamp(UNIX_EPOCH + Duration::from_secs(1_755_000_001));
        assert!(early < late, "사전순이 시간순이어야: {early} < {late}");
    }

    // ── 파일 생성 ──
    #[test]
    fn open_run_log_creates_logs_subdir_and_kind_prefixed_file() {
        let data_dir = temp_dir("open");
        let (path, _file) = open_run_log(&data_dir, LogKind::Daemon).expect("열려야");

        assert_eq!(path.parent().unwrap(), data_dir.join(LOG_SUBDIR));
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("daemon-"), "종류 접두사: {name}");
        assert!(name.ends_with(LOG_EXT), "확장자: {name}");
        assert!(
            name.contains(&std::process::id().to_string()),
            "pid 포함: {name}"
        );

        // 정상 기동은 warn 레벨에서 무출력이라, 머리글이 없으면 파일이 통째로 빈다.
        let head = std::fs::read_to_string(&path).expect("읽기");
        assert!(head.starts_with("==== engram daemon | "), "머리글: {head}");
        assert!(head.ends_with("====\n"), "머리글 한 줄로 끝나야: {head}");
    }

    #[test]
    fn two_kinds_never_share_a_file() {
        let data_dir = temp_dir("kinds");
        let (daemon, _d) = open_run_log(&data_dir, LogKind::Daemon).expect("열려야");
        let (app, _a) = open_run_log(&data_dir, LogKind::App).expect("열려야");
        assert_ne!(daemon, app);
    }

    // ── 보존 ──
    #[test]
    fn prune_keeps_newest_per_kind_and_ignores_other_kinds() {
        let data_dir = temp_dir("prune");
        let dir = data_dir.join(LOG_SUBDIR);
        std::fs::create_dir_all(&dir).expect("dir");

        for i in 0..15 {
            std::fs::write(dir.join(format!("daemon-20260101-{i:06}-1{LOG_EXT}")), "x")
                .expect("write");
        }
        std::fs::write(dir.join(format!("app-20260101-000000-1{LOG_EXT}")), "x").expect("write");
        std::fs::write(dir.join("daemon-20260101-000000-1.txt"), "x").expect("write");

        prune_old_logs(&dir, LogKind::Daemon, 4);

        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .expect("read")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            vec![
                "app-20260101-000000-1.log".to_string(),
                "daemon-20260101-000000-1.txt".to_string(),
                "daemon-20260101-000011-1.log".to_string(),
                "daemon-20260101-000012-1.log".to_string(),
                "daemon-20260101-000013-1.log".to_string(),
                "daemon-20260101-000014-1.log".to_string(),
            ],
            "같은 종류의 오래된 것만 지워야(다른 종류·다른 확장자는 보존)"
        );
    }

    #[test]
    fn open_run_log_enforces_ceiling_including_this_run() {
        let data_dir = temp_dir("ceiling");
        let dir = data_dir.join(LOG_SUBDIR);
        std::fs::create_dir_all(&dir).expect("dir");
        for i in 0..(KEEP_PER_KIND + 5) {
            std::fs::write(dir.join(format!("daemon-20260101-{i:06}-1{LOG_EXT}")), "x")
                .expect("write");
        }

        let (_path, _file) = open_run_log(&data_dir, LogKind::Daemon).expect("열려야");

        let count = std::fs::read_dir(&dir)
            .expect("read")
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("daemon-"))
            .count();
        assert_eq!(count, KEEP_PER_KIND);
    }

    /// 삭제까지 막는 진짜 배타 열기는 Windows 공유 모드로만 만들 수 있다(std `File::open` 은
    /// `FILE_SHARE_DELETE` 를 포함해 열어 삭제가 그냥 성공한다 — 그걸로는 이 회귀를 못 잡는다).
    #[cfg(windows)]
    #[test]
    fn prune_tolerates_locked_files() {
        use std::os::windows::fs::OpenOptionsExt as _;

        let data_dir = temp_dir("locked");
        let dir = data_dir.join(LOG_SUBDIR);
        std::fs::create_dir_all(&dir).expect("dir");
        for i in 0..5 {
            std::fs::write(
                dir.join(format!("daemon-2026010{i}-000000-1{LOG_EXT}")),
                "x",
            )
            .expect("write");
        }
        let held = dir.join(format!("daemon-20260100-000000-1{LOG_EXT}"));
        let _lock = File::options()
            .read(true)
            .share_mode(0)
            .open(&held)
            .expect("배타 열기");

        prune_old_logs(&dir, LogKind::Daemon, 2);

        assert!(held.exists(), "잠긴 파일은 남아야(삭제 실패를 삼킨다)");
        assert!(
            !dir.join(format!("daemon-20260102-000000-1{LOG_EXT}"))
                .exists(),
            "잠기지 않은 오래된 파일은 그대로 지워져야"
        );
        assert!(dir
            .join(format!("daemon-20260104-000000-1{LOG_EXT}"))
            .exists());
    }

    // ── writer ──
    #[test]
    fn file_sink_writer_survives_poisoned_lock() {
        let data_dir = temp_dir("poison");
        let (path, file) = open_run_log(&data_dir, LogKind::App).expect("열려야");
        let sink = FileSink(Mutex::new(file));

        std::thread::scope(|s| {
            let _ = s
                .spawn(|| {
                    let _guard = sink.0.lock().expect("lock");
                    panic!("poison");
                })
                .join();
        });

        sink.make_writer()
            .write_all(b"after-poison\n")
            .expect("써야");
        assert!(std::fs::read_to_string(&path)
            .expect("읽기")
            .ends_with("after-poison\n"));
    }
}
