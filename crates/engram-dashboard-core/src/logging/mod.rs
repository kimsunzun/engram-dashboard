//! tracing 전역 초기화 + 프로세스 실행 1회분 파일 로그.
//!
//! ADR-0138 — 릴리스 로그는 파일에 **동기로** 남긴다(비동기 writer 금지 · 폴더는 호출자가 넘긴다).
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
//!
//! ★그 폴더를 못 쓰면 조용히 포기하지 않고 `%TEMP%` 아래로 물러난다★: 안 그러면 이 기능이 **자기
//! 목적에서 실패한다** — 데이터 폴더를 못 쓰는 것이야말로 데몬이 기동에 실패하는 흔한 이유인데,
//! 하필 그때만 사유가 stdout(= 릴리즈에 없는 곳)으로 간다. 이 폴백은 ADR-0134 결정 4(데이터 폴더
//! 폴백 없음)를 건드리지 않는다 — 그 결정이 막는 손상은 **상태**(명부·잠금)가 두 곳에 생기는 것이고,
//! 여기서 옮기는 것은 상태가 없는 사유 한 장뿐이다. 둘 다 실패하면 정말 남는 곳이 없고, 그 경우의
//! 주인은 클라이언트가 spawn 전에 도는 사전 점검이다(ADR-0135 — 데몬이 아니라 창에 사유를 띄운다).

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

/// 1차 폴더를 못 쓸 때 물러나는 자리 = `%TEMP%/<이 이름>/logs/`. 데이터 폴더와 **이름이 겹치지 않게**
/// 제품 이름을 그대로 쓴다(한 사람의 임시 폴더에 여러 배포판이 물러나면 한 폴더를 나눠 쓰게 되는데,
/// 파일 이름에 pid 가 붙어 섞이지는 않고 보존 상한만 함께 쓴다).
const FALLBACK_DIR_NAME: &str = "engram-dashboard";

/// 실제로 열린 로그 파일. **subscriber 설치에 이긴 뒤에만 채워진다** — 그 전에 쓰이면 삼킨다.
static LOG_FILE: OnceLock<Mutex<File>> = OnceLock::new();

/// [`Mutex`] 의 기본 [`MakeWriter`] 구현 대신 쓰는 파일 writer. 가리키는 칸이 아직 비어 있으면
/// 그 줄을 버린다 — layer 는 설치 시점에 만들어지고 파일은 그보다 **뒤에** 열리기 때문이다.
///
/// ★poison 을 무시하고 계속 쓴다★: 표준 구현은 poison 에서 panic 하는데, 이 sink 의 소비자에는
/// panic hook 이 있다. 한 번 poison 되면 그 hook 의 로그가 다시 panic 해(panic-in-panic → abort)
/// **정작 남겨야 할 죽음의 이유가 사라진다.** 잠금이 지키는 건 줄 섞임뿐이라 뒤엎어도 안전하다.
///
/// ★알려진 미수정 — 이 잠금을 쥔 채 panic 하면 hook 이 자기 자신을 기다린다★: panic hook 은 되감기
/// **전에, panic 한 그 스레드에서** 돌고(데몬 `install_panic_hook`), 그 안의 `error!` 가 같은
/// `Mutex` 를 다시 잠근다 → 자기교착(프로세스가 멎는다 — abort 보다 나쁘다). 창은 `write_all` 안에서
/// panic 할 때뿐이라(디스크 I/O 중 std 가 panic 하는 경우) 실측된 적이 없고, 없애려면 잠금 없는
/// 쓰기(줄 섞임)나 hook 전용 우회 경로가 필요해 대가가 더 크다. 재진입을 감지해 건너뛰는 손질은
/// 이 파일이 아니라 hook 쪽에서 해야 한다.
struct FileSink(&'static OnceLock<Mutex<File>>);

enum FileSinkWriter<'a> {
    Live(MutexGuard<'a, File>),
    /// 파일이 아직(또는 끝내) 안 열린 상태 — 조용히 버린다. stdout layer 는 그대로 받는다.
    Void,
}

impl<'a> MakeWriter<'a> for FileSink {
    type Writer = FileSinkWriter<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        match self.0.get() {
            Some(m) => FileSinkWriter::Live(m.lock().unwrap_or_else(|e| e.into_inner())),
            None => FileSinkWriter::Void,
        }
    }
}

impl io::Write for FileSinkWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Live(f) => f.write(buf),
            Self::Void => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Live(f) => f.flush(),
            Self::Void => Ok(()),
        }
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

/// 이 이름이 **이 종류의** 실행 로그인가 — `<종류>-YYYYMMDD-HHMMSS-<pid>.log` 문법에 정확히 맞는
/// 것만 참이다.
///
/// ★접두사 일치(`starts_with("<종류>-")`)로 고르지 마라(되살리지 마라)★: 그러면 둘이 깨진다.
/// ① 라벨이 다른 라벨의 접두사이기만 하면(`app` vs `app-updater`) 한쪽의 정리가 **다른 종류의
/// 파일을 지운다** — 라벨은 앞으로도 늘어난다. ② 이름이 곧 정렬 키라, 손으로 둔 `daemon-메모.log`
/// 하나가 사전순 끝에 눌러앉아 **진짜 로그를 상한 밖으로 밀어내 지운다.**
///
/// 달력 유효성까지 보지는 않는다(`daemon-20261301-…` 는 통과). 문법만으로도 위 둘은 막히고, 우리가
/// 쓴 이름은 [`utc_stamp`] 가 만들어 애초에 유효하다.
fn is_run_log_name(name: &str, kind: LogKind) -> bool {
    let Some(rest) = name
        .strip_prefix(kind.label())
        .and_then(|r| r.strip_prefix('-'))
        .and_then(|r| r.strip_suffix(LOG_EXT))
    else {
        return false;
    };
    let mut parts = rest.split('-');
    let (Some(date), Some(time), Some(pid), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    date.len() == 8
        && time.len() == 6
        && !pid.is_empty()
        && [date, time, pid]
            .iter()
            .all(|s| s.bytes().all(|b| b.is_ascii_digit()))
}

/// 같은 종류의 오래된 파일을 지워 `keep` 개만 남긴다. 반환 = **삼킨 실패들의 사유**.
///
/// 삭제 실패로 기동을 멈추지는 않는다 — 다른 프로세스가 붙들고 있는 파일이 정상적으로 존재하고(같은
/// 종류의 데몬이 두 데이터 폴더에서 도는 등), 그 하나 때문에 못 뜨면 안 된다. 다만 **조용히**
/// 삼키지는 않는다: 이건 영원히 실패해도 아무도 모르는 종류의 일이라, 호출자가 subscriber 설치
/// 직후 한 번에 낸다(여기서는 못 낸다 — 이 함수가 도는 시점엔 subscriber 가 아직 없다).
fn prune_old_logs(dir: &Path, kind: LogKind, keep: usize) -> Vec<String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            return vec![format!(
                "{} 목록을 읽지 못해 보존 정리 생략: {e}",
                dir.display()
            )]
        }
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| is_run_log_name(n, kind))
        .collect();
    if names.len() <= keep {
        return Vec::new();
    }
    names.sort_unstable();
    let mut failures = Vec::new();
    for stale in &names[..names.len() - keep] {
        if let Err(e) = std::fs::remove_file(dir.join(stale)) {
            failures.push(format!("낡은 로그 {stale} 를 지우지 못함: {e}"));
        }
    }
    failures
}

/// 이번 실행이 실제로 연 파일 한 벌 + **그때는 낼 수 없었던 진단**.
///
/// `deferred` 는 subscriber 설치 전에 일어난 실패의 사유들이다(보존 삭제 · 머리글 쓰기 · 폴백).
/// 그 시점엔 로그를 낼 곳이 없어 모아 두고, 호출자가 설치 직후 한 번에 낸다.
struct OpenedLog {
    path: PathBuf,
    file: File,
    deferred: Vec<String>,
}

/// `<data_dir>/logs/<종류>-<UTC>-<pid>.log` 를 새로 열고 그 핸들을 돌려준다.
///
/// pid 를 붙이는 이유는 같은 초에 두 번 뜬 실행이 한 파일을 나눠 쓰는 것을 막기 위해서다.
///
/// 알려진 잔여(수정 안 함): pid 가 같은 UTC 초 안에서 재사용되거나 시계가 뒤로 돌면 이름이 겹치고,
/// append 라 **두 실행분이 한 파일에 이어진다**. 머리글이 그때마다 다시 찍혀 경계는 읽을 수 있고,
/// 이걸 막으려면 열기에 배타 생성 + 재시도 루프가 필요해 대가가 이득을 넘는다.
fn open_run_log(data_dir: &Path, kind: LogKind) -> io::Result<OpenedLog> {
    let dir = data_dir.join(LOG_SUBDIR);
    std::fs::create_dir_all(&dir)?;
    let mut deferred = prune_old_logs(&dir, kind, KEEP_PER_KIND.saturating_sub(1));

    let stamp = utc_stamp(SystemTime::now());
    let path = dir.join(format!(
        "{}-{stamp}-{}{LOG_EXT}",
        kind.label(),
        std::process::id()
    ));
    let mut file = File::options().create(true).append(true).open(&path)?;
    if let Err(e) = write_header(&mut file, kind, &stamp) {
        deferred.push(format!("로그 머리글을 쓰지 못함: {e}"));
    }
    Ok(OpenedLog {
        path,
        file,
        deferred,
    })
}

/// [`open_run_log`] 를 1차(호출자 폴더) → 2차(`%TEMP%`) 순으로 시도한다. `Err` = 둘 다 실패, 값은
/// 두 사유.
///
/// ★2차가 있는 이유 = 1차가 실패하는 그 상황이 곧 진단이 가장 필요한 상황이라서★: 데이터 폴더를
/// 못 쓰면 데몬은 그 직후 기동을 포기하는데, 폴백이 없으면 그 사유가 stdout(릴리즈엔 없다)으로만
/// 간다. 근거·이 폴백이 ADR-0134 결정 4를 침범하지 않는 이유는 이 파일 머리말.
fn open_run_log_with_fallback(data_dir: &Path, kind: LogKind) -> Result<OpenedLog, Vec<String>> {
    let primary = match open_run_log(data_dir, kind) {
        Ok(opened) => return Ok(opened),
        Err(e) => format!("{} 를 열지 못함: {e}", data_dir.join(LOG_SUBDIR).display()),
    };

    let fallback_root = std::env::temp_dir().join(FALLBACK_DIR_NAME);
    match open_run_log(&fallback_root, kind) {
        Ok(mut opened) => {
            opened.deferred.insert(
                0,
                format!("{primary} — 대신 {} 에 기록한다", opened.path.display()),
            );
            Ok(opened)
        }
        Err(e) => Err(vec![
            primary,
            format!(
                "{} 폴백도 열지 못함: {e}",
                fallback_root.join(LOG_SUBDIR).display()
            ),
        ]),
    }
}

/// 파일 첫 줄에 이 실행의 신원을 박는다.
///
/// ★로그 이벤트가 아니라 파일 머리글인 이유★: 기본 레벨이 `warn` 이라 **정상 기동은 한 줄도 남기지
/// 않는다** — 그러면 남는 건 빈 파일이고, 언제 뜬 어느 바이너리의 것인지조차 알 수 없다. 그걸 메우려고
/// 기동 알림을 `info` → `warn` 으로 올리면 레벨의 의미가 무너지므로, 이벤트 평면 밖에 한 줄만 둔다.
fn write_header(file: &mut File, kind: LogKind, stamp: &str) -> io::Result<()> {
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|e| format!("(exe 경로 불명: {e})"));
    writeln!(
        file,
        "==== engram {kind} | {stamp} UTC | pid {pid} | {exe} ====",
        kind = kind.label(),
        pid = std::process::id()
    )
}

// ── 초기화 ───────────────────────────────────────────────────────────────────────

/// tracing-subscriber 전역 초기화(stdout 만). 앱 부팅 시 1회만 호출 (멱등 — 중복 호출 no-op).
/// 기본 레벨: RUST_LOG 환경변수 우선, 없으면 "warn" (릴리스 기본 OFF — 평상시 거의 무출력).
pub fn init_logging() {
    init_subscriber(false);
}

/// [`init_logging`] + `<data_dir>/logs/` 아래 이번 실행 전용 파일 sink.
///
/// 반환값 = 이 호출이 실제로 붙인 로그 파일 경로. **1차 폴더를 못 쓰면 `%TEMP%` 아래 경로일 수
/// 있다**(그 사실은 반환 직전 `warn` 으로도 남는다 — 이 파일 머리말). `None` = 파일 sink 없음이고,
/// 사유는 셋이다: 이미 초기화됨(조용) · 남이 먼저 깐 subscriber 라 layer 가 버려짐(조용) ·
/// 1차·폴백 둘 다 열기 실패(직후 `warn` — 단 릴리즈에선 그 `warn` 도 갈 곳이 없다).
///
/// 레벨 필터·stdout 출력은 [`init_logging`] 과 동일하다 — 파일은 **더해질 뿐** 대체하지 않는다.
///
/// 알려진 잔여(수정 안 함): 같은 종류를 두 스레드가 동시에 부르면 둘 다 맨 앞 검사를 통과할 수 있다.
/// 진 쪽은 `try_init` 에서 걸러져 `None` 을 돌려주고 파일도 만들지 않으므로 손상은 없다 — 정확한
/// 배제를 하려면 초기화 전용 잠금이 필요하고, 부팅 1회 호출에 그 값은 없다.
pub fn init_logging_with_file(data_dir: &Path, kind: LogKind) -> Option<PathBuf> {
    if RELOAD_HANDLE.get().is_some() {
        return None;
    }

    // ★파일은 subscriber 설치에 **이긴 뒤에만** 연다★: 순서를 뒤집으면 진 호출도 파일을 만들고
    //   보존 정리까지 돌려, 아무도 쓰지 않을 빈 파일이 **진짜 로그 하나를 지우고** 그 자리를 영구히
    //   차지한다. layer 는 아직 안 열린 칸을 가리킨 채 설치되고(그 사이 줄은 [`FileSink`] 가 버린다),
    //   칸이 채워지는 건 바로 아래다.
    if !init_subscriber(true) {
        return None;
    }

    match open_run_log_with_fallback(data_dir, kind) {
        Ok(opened) => {
            let path = opened.path;
            // set 실패 = 이 정적 칸을 채운 다른 호출이 있다는 뜻인데, 그건 try_init 에 이긴 하나뿐이라
            //   여기까지 오지 않는다. 그래도 파일 없이 경로를 광고하지는 않는다.
            if LOG_FILE.set(Mutex::new(opened.file)).is_err() {
                return None;
            }
            // 이제야 낼 수 있는 것들 — 모아 둔 사유는 sink 가 붙은 다음에 내야 파일에 남는다.
            for line in opened.deferred {
                tracing::warn!("{line}");
            }
            Some(path)
        }
        Err(reasons) => {
            // ★여기가 이 기능의 마지막 구멍이다★: 갈 곳이 stdout 뿐인데 릴리즈엔 stdout 이 없다.
            //   이 경우(1차 폴더도 임시 폴더도 못 씀)의 주인은 클라이언트가 데몬을 띄우기 전에 도는
            //   데이터 폴더 사전 점검이고(ADR-0135), 사용자는 그쪽 창에서 사유를 본다.
            for r in reasons {
                tracing::warn!("파일 로그를 열지 못해 stdout 으로만 기록함: {r}");
            }
            None
        }
    }
}

/// subscriber 를 실제로 설치했으면 `true`. `with_file` 은 파일 layer 를 **자리만** 붙일지다 —
/// 실제 파일은 [`LOG_FILE`] 이 채워질 때 살아난다.
fn init_subscriber(with_file: bool) -> bool {
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
    let file_layer = with_file.then(|| {
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(FileSink(&LOG_FILE))
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
        let opened = open_run_log(&data_dir, LogKind::Daemon).expect("열려야");
        let path = opened.path;

        assert!(opened.deferred.is_empty(), "정상 경로는 진단이 없어야");
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
        let daemon = open_run_log(&data_dir, LogKind::Daemon).expect("열려야");
        let app = open_run_log(&data_dir, LogKind::App).expect("열려야");
        assert_ne!(daemon.path, app.path);
    }

    // 폴백(1차 폴더 불가 → `%TEMP%`)은 `%TEMP%` 를 갈아끼운 별도 프로세스가 필요해 통합 테스트에
    // 있다(`tests/logging_fallback.rs`) — 여기서 하면 다른 테스트가 보는 임시 폴더까지 건드린다.

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

        assert!(
            prune_old_logs(&dir, LogKind::Daemon, 4).is_empty(),
            "삭제 실패 없음"
        );

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

    /// 라벨이 다른 라벨의 접두사인 경우와, 문법에 안 맞는 이름이 정렬 끝에 눌러앉는 경우 — 접두사
    /// 일치로 되돌리면 둘 다 **남의 파일을 지운다**.
    #[test]
    fn prune_selects_by_filename_grammar_not_prefix() {
        let data_dir = temp_dir("grammar");
        let dir = data_dir.join(LOG_SUBDIR);
        std::fs::create_dir_all(&dir).expect("dir");

        // 아직 없는 미래 종류(라벨이 `app-` 로 시작한다)와 손으로 둔 이름.
        for name in [
            "app-updater-20260101-000000-1.log",
            "app-메모.log",
            "app-zzzz-99999999-999999-1.log",
        ] {
            std::fs::write(dir.join(name), "x").expect("write");
        }
        for i in 0..5 {
            std::fs::write(dir.join(format!("app-20260101-{i:06}-1{LOG_EXT}")), "x")
                .expect("write");
        }

        prune_old_logs(&dir, LogKind::App, 2);

        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .expect("read")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            vec![
                "app-20260101-000003-1.log".to_string(),
                "app-20260101-000004-1.log".to_string(),
                "app-updater-20260101-000000-1.log".to_string(),
                "app-zzzz-99999999-999999-1.log".to_string(),
                "app-메모.log".to_string(),
            ],
            "문법에 맞는 이 종류의 파일만 후보여야"
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

        let _opened = open_run_log(&data_dir, LogKind::Daemon).expect("열려야");

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

        let failures = prune_old_logs(&dir, LogKind::Daemon, 2);

        assert!(held.exists(), "잠긴 파일은 남아야(삭제 실패를 삼킨다)");
        assert_eq!(failures.len(), 1, "삼키되 사유는 돌려줘야: {failures:?}");
        assert!(
            failures[0].contains("daemon-20260100-000000-1.log"),
            "어느 파일인지 남아야: {failures:?}"
        );
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
    /// 운영에서는 [`LOG_FILE`] 하나뿐이지만 그건 프로세스당 한 번만 채워져 테스트가 나눠 쓸 수 없다.
    /// 그래서 sink 가 가리킬 칸을 테스트마다 새로 만든다.
    fn leaked_slot() -> &'static OnceLock<Mutex<File>> {
        Box::leak(Box::new(OnceLock::new()))
    }

    #[test]
    fn file_sink_writer_survives_poisoned_lock() {
        let data_dir = temp_dir("poison");
        let opened = open_run_log(&data_dir, LogKind::App).expect("열려야");
        let slot = leaked_slot();
        assert!(slot.set(Mutex::new(opened.file)).is_ok(), "칸 채우기");
        let sink = FileSink(slot);

        std::thread::scope(|s| {
            let _ = s
                .spawn(|| {
                    let _guard = slot.get().expect("칸").lock().expect("lock");
                    panic!("poison");
                })
                .join();
        });

        sink.make_writer()
            .write_all(b"after-poison\n")
            .expect("써야");
        assert!(std::fs::read_to_string(&opened.path)
            .expect("읽기")
            .ends_with("after-poison\n"));
    }

    /// 파일이 아직 안 열린 창(= layer 설치 ~ 칸 채우기 사이)에 오는 줄은 버린다 — 여기서 막지 않으면
    /// `unwrap` 계열이 부팅 초입의 로그 한 줄로 프로세스를 죽인다.
    #[test]
    fn file_sink_writer_discards_lines_before_the_file_exists() {
        let sink = FileSink(leaked_slot());
        sink.make_writer().write_all(b"nowhere\n").expect("삼켜야");
    }
}
