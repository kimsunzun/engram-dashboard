//! codex 스레드 id 를 **락 파일의 소유자**로 회수한다 — 우리가 띄운 자식이 쥐고 있는 락 파일의 이름이
//! 곧 그 자식의 스레드 id 다(ADR-0218 결정 1).
//!
//! ★이 파일이 쥐는 codex 지식은 둘뿐★ — 락 디렉터리의 위치([`lock_dir`])와 파일 이름이 곧 id 라는 것.
//! 「이 파일을 누가 쥐고 있나」와 「이 PID 아래 무엇이 살아 있나」는 도메인 지식이 0 이라
//! [`crate::platform::file_holders`]·[`crate::platform::process_tree`] 가 답한다
//! (ADR-0004 · ADR-0218 결정 11). ★**파일 내용은 읽지 않는다**★ — 0바이트이고, 읽기 시작하면 벤더
//! 저장 포맷 의존이 된다(ADR-0203).
//!
//! 진입점 둘 — [`plan_capture`](이 spawn 에서 회수를 돌릴지와 그 재료를 고른다)와
//! [`spawn_capture`](그 재료로 폴링 스레드를 띄운다). 판정은 [`scan_for_child`] 가 지고, 기다림과
//! 자식 생존 확인은 [`CaptureClock`] 이 진다. 파일시스템·OS 질의가 전부 그 둘과 [`LockHolderProbe`]
//! 뒤에 있어 루프 규칙을 OS 없이 잰다(ADR-0012).
//!
//! ★회수한 값이 나가는 길은 [`crate::backend::SessionIdSink`] 하나다★ — 조립점이 화신 표식을 묶어
//! 건넨 그 동사이고, 이 파일은 프로필도 그 스키마도 모른다(ADR-0004 · ADR-0218 결정 6). 둘째 쓰기
//! 경로를 만들지 말 것 — 그 순간 화신 가드를 우회하는 갈래가 생긴다.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use uuid::Uuid;

use crate::backend::SessionIdSink;
use crate::platform::file_holders::{self, Holder};
use crate::platform::process_tree::{self, ProcessIdentity};

/// codex 홈 아래 writer 락이 사는 폴더 이름.
///
/// ★문서화되지 않은 배치다★ — 상류가 바꾸면 오류가 아니라 조용히 후보 0 개가 된다(ADR-0218 「근거」의
/// 미측정 ③). 실패 모드가 「못 받음」이라 그 조용함이 오검출로 번지지는 않는다.
const LOCK_DIR: &str = "thread-writer-locks";

/// 락 파일의 확장자. 이름의 나머지가 스레드 id 다.
const LOCK_EXTENSION: &str = "lock";

/// [`CODEX_HOME_ENV`] 가 없을 때의 codex 홈(사용자 홈 기준 상대 경로).
const CODEX_HOME_SUBDIR: &str = ".codex";

/// codex 상태 디렉터리를 통째로 옮기는 env 변수 — 실재를 실측으로 확인했고, **건 프로세스에만**
/// 적용된다(`docs/research/codex-session-id-recovery-survey-2026-09-19.md` 의 `CODEX_HOME` 절).
const CODEX_HOME_ENV: &str = "CODEX_HOME";

/// 이 모듈이 OS 를 만나는 자리 전부 — 파일 홀더 · 폴더 목록 · 우리 프로세스 나무.
///
/// ★셋을 한 seam 에 둔 것은 판정 규칙을 OS 없이 재기 위해서다★ — 그래야 [`scan_for_child`] 가
/// 실제 파일 한 개, 실제 프로세스 한 개 없이 돌아간다(ADR-0012).
pub(crate) trait LockHolderProbe {
    /// 못 물어본 것과 홀더가 없는 것을 **가르지 않는다** — 둘 다 빈 목록이다. 어차피 판정이 같은
    /// 자리(「후보 아님」)로 떨어지고, 폴링이 다음 바퀴에 다시 묻는다.
    fn holders(&self, path: &Path) -> Vec<Holder>;

    /// 우리가 띄운 자식과 그 아래 **살아 있는** 후손 전부의 신원. 빈 목록 = 그 PID 가 더는 우리
    /// 자식이 아니다.
    ///
    /// ★왜 나무인가 — 우리가 쥔 PID 는 codex 가 아니다★: Windows 에서 스폰은 `cmd.exe /c codex …`
    ///   한 겹을 지나므로([`crate::backend::console_command`]) 통로가 돌려준 PID 는 **그 래퍼**다.
    ///   락을 쥔 것은 그 아래의 `codex.exe` 자신이다(ADR-0218 「근거」 — 홀더는 래퍼도 자식도 아닌
    ///   codex.exe 였다). ★래퍼 PID 하나만 대조하면 일치가 **영영** 안 나고 증상은 오류가 아니라
    ///   침묵이다★ — 스레드가 에이전트 수명 내내 빈손으로 돈다(실측 2026-09-21: `cmd.exe /c …` →
    ///   래퍼 38240 · 그 아래 4816,19468).
    /// ★바퀴마다 다시 푼다 — 한 번 떠 둔 명단을 재사용하지 말 것★: shim 이 codex 를 늦게 띄우고
    ///   codex 가 또 도구를 띄우므로 나무는 시간에 따라 자란다. 굳혀 두면 늦게 뜬 codex 를 못 본다.
    fn our_processes(&self, root_pid: u32, root_start_time: u64) -> Vec<ProcessIdentity> {
        process_tree::subtree(root_pid, root_start_time)
    }

    /// `dir` 바로 아래의 **정규 파일** 경로 전부(이름 판정은 [`scan_for_child`] 몫). 못 읽으면 빈
    /// 목록이다: 아직 안 생긴 디렉터리가 정상 경로라 오류로 올리지 않는다.
    ///
    /// ★디렉터리와 재분석 지점(symlink·junction)은 후보에서 뺀다★ — `<uuid>.lock` 이라는 이름의
    ///   폴더는 락이 아니고, 재분석 지점을 홀더 조회에 넘기면 우리가 고르지 않은 대상이 열린다.
    ///   `file_type()` 은 링크를 따라가지 않으므로 `is_file()` 이 그 둘을 함께 거른다. 형식을
    ///   못 읽는 항목도 뺀다 — 모르는 것을 후보로 올리지 않는다.
    fn entries(&self, dir: &Path) -> Vec<PathBuf> {
        let Ok(read) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        read.flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .map(|entry| entry.path())
            .collect()
    }
}

/// 운영에서 쓰는 구현 — [`crate::platform::file_holders`] 에 묻고, 물음 자체가 실패하면 빈 목록으로
/// 접는다(그 접기의 사유는 [`LockHolderProbe::holders`] 의 계약).
pub(crate) struct RestartManagerProbe;

impl LockHolderProbe for RestartManagerProbe {
    fn holders(&self, path: &Path) -> Vec<Holder> {
        match file_holders::holders_of(path) {
            Ok(holders) => holders,
            Err(err) => {
                tracing::debug!(path = %path.display(), %err, "락 파일 홀더 조회 실패");
                Vec::new()
            }
        }
    }
}

/// [`scan_for_child`] 의 답. ★채택 가능한 것은 `Found` 뿐이다★.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScanOutcome {
    /// 우리 나무 안의 프로세스가 쥔 락이 정확히 하나 — 그 파일 이름의 스레드 id.
    Found(Uuid),
    /// 후보 0 개. 아직 락이 안 생겼거나 이 자식이 락을 만들지 않는다 — 계속 기다린다(ADR-0218 결정 4).
    NotYet,
    /// 후보 n(≥2) 개 — ★이 바퀴에는 아무것도 채택하지 않는다★. 한 codex 프로세스가 락을 둘 이상 쥐는
    /// 경우가 있는지 미측정이라, 추측 대신 거절한다(ADR-0218 결정 3 · 못 받는 것이 잘못 받는 것보다
    /// 낫다). ★「거절」이 「포기」는 아니다 — 폴링은 계속된다★(결정 4 의 멈춤 목록에 이 결말은 없다).
    Ambiguous(usize),
}

/// `lock_dir` 안에서 **우리 나무 안의 프로세스가 쥔** 락 파일을 찾아 그 이름의 스레드 id 를 돌려준다.
///
/// 판정 규칙(전부 ADR-0218):
/// - **대조 상대는 `child_pid` 하나가 아니라 그 PID 를 뿌리로 하는 나무 전부다**
///   ([`LockHolderProbe::our_processes`] 가 사유의 정본 — 우리가 쥔 PID 는 `cmd.exe` 래퍼이고 락을
///   쥔 것은 그 아래 `codex.exe` 다).
/// - 후보 = 이름이 `<uuid>.lock` 이고, 홀더 중 **PID 와 시작시각이 둘 다** 나무의 한 항목과 같은 것이
///   있는 파일. ★어느 한쪽만 맞는 것은 후보가 아니다★ — PID 는 재사용되므로 그 판정이 남의 스레드를
///   우리 손잡이에 적는다(결정 2). 나무를 넓혔다고 이 쌍이 풀리는 것이 아니다: 후손의 시작시각도
///   열거하는 그 자리에서 함께 읽어 온다.
/// - ★**세는 것은 파일이 아니라 「맞은 홀더」다**★ — 결정 3 의 「후보가 정확히 하나」는 **짝**의 수이지
///   파일 이름의 수가 아니다. 한 파일을 우리 프로세스 둘이 함께 쥐고 있으면 「그 락이 누구 것인가」가
///   하나로 안 좁혀진 상태이고, 파일만 세면 그 상태가 `Found` 로 통과한다.
///   ★그 대가를 알고 고른다★: 자식이 핸들을 물려받는 경우(codex 가 띄운 도구가 같은 락 핸들을 상속)가
///   있으면 그 구간 내내 채택이 안 된다. 그것이 견딜 만한 것은 [`run_capture`] 가 `Ambiguous` 에서
///   **멈추지 않기** 때문이다 — 상속한 쪽이 끝나면 다음 바퀴에 채택한다. 상속이 실제로 일어나는지는
///   미측정이고, 일어난다면 증상은 「회수가 늦어진다」이지 오검출이 아니다.
///   ★**이 대가에 이견이 있다 — 다시 꺼내기 전에 이 줄을 읽을 것**★(2026-09-21): 「둘 다 **우리** 것이면
///   그 락은 우리 것이니 채택이 옳다」는 반론이 있었고, 그 판정 자체는 이 라운드의 범위 밖이라 코드를
///   그대로 뒀다. 즉 지금 모양은 **결정이 아니라 보류**다. 바꾸려면 상속이 실재하는지를 먼저 재고
///   ADR-0218 결정 3 을 그 측정 위에서 다시 열 것 — 여기서 조용히 뒤집지 말 것.
/// - 맞은 홀더가 정확히 하나면 [`ScanOutcome::Found`], 0 이면 [`ScanOutcome::NotYet`], 둘 이상이면
///   [`ScanOutcome::Ambiguous`](결정 3).
/// - `child_pid` 나 `child_start_time` 이 0 이면 **묻지도 않고** `NotYet` 이다. 0 은 「미상」이고, 미상을
///   일치로 세면 남는 것은 PID 단독 대조라 결정 2 가 금한 판정이 된다.
/// - 나무가 비면(자식이 죽었거나 PID 가 재사용됐다) 파일을 하나도 묻지 않고 `NotYet` 이다 —
///   홀더 조회는 바퀴당 후보 파일 수만큼 Restart Manager 세션을 여는 비싼 물음이라, 답이 정해진
///   자리에서 태우지 않는다.
///
/// 같은 파일을 여러 프로세스가 열고 있어도 된다 — 그중 하나가 우리 나무 안이면 후보다.
pub(crate) fn scan_for_child(
    lock_dir: &Path,
    child_pid: u32,
    child_start_time: u64,
    probe: &dyn LockHolderProbe,
) -> ScanOutcome {
    if child_pid == 0 || child_start_time == 0 {
        return ScanOutcome::NotYet;
    }
    let ours = probe.our_processes(child_pid, child_start_time);
    if ours.is_empty() {
        return ScanOutcome::NotYet;
    }

    let mut matches: usize = 0;
    let mut first: Option<Uuid> = None;
    for path in probe.entries(lock_dir) {
        let Some(id) = thread_id_from_lock_name(&path) else {
            continue;
        };
        let held_by_us = probe
            .holders(&path)
            .iter()
            .filter(|holder| {
                ours.iter()
                    .any(|mine| mine.pid == holder.pid && mine.start_time == holder.start_time)
            })
            .count();
        if held_by_us > 0 {
            matches += held_by_us;
            first.get_or_insert(id);
        }
    }

    match (matches, first) {
        (1, Some(id)) => ScanOutcome::Found(id),
        (n, Some(_)) if n > 1 => ScanOutcome::Ambiguous(n),
        _ => ScanOutcome::NotYet,
    }
}

/// **우리 프로세스의** 환경에서 본 락 디렉터리. 홈을 못 찾으면 `None`.
///
/// ★스폰이 자기 [`CODEX_HOME_ENV`] 를 싣는 경우는 이 함수가 답하지 않는다★ — 프로필 env 로 그 변수를
/// 건 자식은 다른 홈을 쓴다. 그 자리에서는 [`lock_dir_under`] 에 그 홈을 직접 준다.
pub(crate) fn lock_dir() -> Option<PathBuf> {
    Some(lock_dir_under(&codex_home()?))
}

/// 홈을 아는 호출자용 — `<codex home>/thread-writer-locks`.
pub(crate) fn lock_dir_under(codex_home: &Path) -> PathBuf {
    codex_home.join(LOCK_DIR)
}

/// 락 파일 이름 → 스레드 id. `<uuid>.lock` 이 아니면 `None` — 오류가 아니라 **건너뛴다**(실측: 같은
/// 폴더에 `.coordination.lock` 처럼 id 가 아닌 파일이 함께 산다).
///
/// ★정규형(하이픈 36자)만 받는다 — `Uuid::parse_str` 단독으로 두지 말 것★: 그 함수는 하이픈 없는
///   32자·중괄호·`urn:uuid:` 까지 받아서, codex 가 만들지 않는 이름이 후보로 올라온다. 후보가 하나 더
///   늘면 판정이 `Found` 에서 `Ambiguous` 로 뒤집혀 **회수가 통째로 멈춘다** — 우리가 만들지 않은 파일
///   하나가 남의 폴더에 놓인 것만으로 그렇게 된다. 대소문자는 안 가린다(Windows 파일 이름이 그렇다).
fn thread_id_from_lock_name(path: &Path) -> Option<Uuid> {
    if !path.extension()?.eq_ignore_ascii_case(LOCK_EXTENSION) {
        return None;
    }
    let stem = path.file_stem()?.to_str()?;
    let id = Uuid::parse_str(stem).ok()?;
    (id.hyphenated().to_string() == stem.to_ascii_lowercase()).then_some(id)
}

fn codex_home() -> Option<PathBuf> {
    codex_home_from(std::env::var_os(CODEX_HOME_ENV), user_home())
}

/// [`codex_home`] 의 규칙만 — env 를 건드리지 않고 재려고 갈라 뒀다.
fn codex_home_from(override_var: Option<OsString>, user_home: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(dir) = override_var {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    user_home.map(|home| home.join(CODEX_HOME_SUBDIR))
}

#[cfg(windows)]
fn user_home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(PathBuf::from)
}

#[cfg(not(windows))]
fn user_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// 첫 스캔까지의 대기. ★0 이 아닌 것이 의도다★ — 실측된 스폰→락 생성 지연의 최솟값이 0.735 초라
/// (ADR-0218 「근거」) 띄우자마자 훑으면 확실한 빈손 한 바퀴를 공짜로 산다.
const FIRST_DELAY: Duration = Duration::from_millis(500);

/// 빈손일 때마다 대기를 두 배로 늘릴 때의 상한.
///
/// ★왜 백오프인가 — 한 바퀴가 공짜가 아니다★: [`scan_for_child`] 는 후보 파일 **하나마다** Restart
///   Manager 세션을 하나 연다. 실사용자의 락 폴더에 파일이 8 개 있었다(실측). 그래서 고정 짧은 주기는
///   락을 영영 안 만드는 자식(상류가 배치를 바꿨거나 `codex exec` 인 경우) 아래에서 그 에이전트가 사는
///   내내 초당 수 건의 RM 세션을 태운다.
/// ★그런데도 상한 **시각**은 두지 않는다(ADR-0218 결정 4)★ — 판정이 시각이 아니라 소유권이라 늦게
///   잡아도 오검출이 늘지 않는다. 느려질 뿐 포기하지 않는 것이 백오프와 시간창의 차이다.
/// ★첫 몇 바퀴가 촘촘한 이유★: 0.735 · 0.879 · 0.907 · 1.891 초가 실측된 생성 지연이고, 재현되지 않은
///   21 초 사례가 하나 있다. 0.5 초에서 두 배씩 늘리면 여섯째 바퀴가 30 초 근처라 그 꼬리도 덮는다.
const MAX_DELAY: Duration = Duration::from_secs(15);

/// 폴링 루프가 OS 를 만나는 나머지 두 자리 — 자식이 아직 살아 있나와 다음 바퀴까지의 기다림.
///
/// ★[`LockHolderProbe`] 와 갈라 둔 것은 축이 다르기 때문이다★ — 그쪽은 「무엇이 보이나」, 이쪽은
///   「언제 다시 보나 · 볼 이유가 남았나」다. 둘을 합치면 루프 규칙을 잴 때 시계까지 가짜로 만들어야
///   하는 자리와 목록만 바꾸면 되는 자리가 섞인다.
pub(crate) trait CaptureClock {
    /// `false` = 그 PID 가 더는 **우리 자식이 아니다**(죽었거나 PID 가 재사용됐다). 루프의 정상 종료
    /// 조건이고 오류가 아니다.
    fn child_alive(&self) -> bool;

    /// 다음 바퀴까지 기다린다. ★락을 하나도 쥐지 않은 채 불린다★ — 이 자리가 루프에서 가장 오래
    /// 머무는 곳이다.
    fn sleep(&self, delay: Duration);
}

/// 운영에서 쓰는 시계 — 자식 판정은 PID 와 시작시각을 **함께** 본다(ADR-0218 결정 2 와 같은 규칙).
struct ChildClock {
    pid: u32,
    start_time: u64,
}

impl CaptureClock for ChildClock {
    fn child_alive(&self) -> bool {
        engram_dashboard_base::platform::pid_alive_with_start_time(self.pid, self.start_time)
    }

    fn sleep(&self, delay: Duration) {
        std::thread::sleep(delay);
    }
}

/// 이 spawn 에서 회수를 돌릴 재료 한 벌. ★만들어졌다는 것 자체가 「돌릴 조건을 다 채웠다」다★ —
/// [`plan_capture`] 가 하나라도 못 채우면 `None` 이라, [`run_capture`] 안에 조건문이 남지 않는다.
pub(crate) struct CapturePlan {
    /// 이 **자식이** 쓰는 락 폴더 — 우리 것이 아니다([`plan_capture`] 의 계약).
    pub(crate) lock_dir: PathBuf,
    pub(crate) child_pid: u32,
    pub(crate) child_start_time: u64,
    /// 조립점이 화신 표식을 묶어 건넨 기록 동사. 회수한 값이 나가는 **유일한** 길이다.
    ///
    /// ★이 `Arc` 가 무엇을 살려 두나 — 재 봤다★: 안쪽 클로저가 `Arc<ProfileRegistry>` 를 쥔다.
    ///   그런데 그 명부는 `AgentManager` 가 **데몬 수명 내내** 들고 있는 단일 소유물이라, 이
    ///   clone 은 그 수명을 한 순간도 늘리지 않는다. 늘리는 것은 스레드 자기 수명뿐이고 그것은
    ///   자식이 죽으면 끝난다([`run_capture`] 의 멈춤 조건).
    ///   ★`Weak` 으로 바꾸지 않은 이유★: 강한 쪽을 들고 있을 자리가 없다 — 조립점은 이 동사를
    ///   `open_spawn` 에 넘기고 그대로 놓는다. 세션이 들게 하려면 `AgentSession` 의 소유물 분할
    ///   (「소유권 분할」 불변식)을 고쳐야 하고, 그것은 이 회수 경로 혼자 요구할 변경이 아니다.
    pub(crate) sink: SessionIdSink,
}

/// 이 spawn 이 락 회수를 돌려야 하나 — 돌린다면 그 재료를, 아니면 `None`.
///
/// 조건(하나라도 빠지면 `None`):
/// - **이 spawn 이 이어받기 argv 를 내지 않는다**(`resumes_by_argv == false`). ★묻는 것이
///   「저장된 손잡이가 있나」가 **아니라** 「그 손잡이가 실제로 argv 로 나갔나」인 것이 요점이다★ —
///   손잡이가 있어도 그 값을 실을 수 없으면(`resume_argument` 가 거절하는 값) codex 는 **새 대화로**
///   뜨고, 그 화신의 id 는 아무도 안 적는다. 「있다」로 판정하던 옛 형태가 정확히 그 구멍이었다
///   (적출 2026-09-21). 두 결정이 **같은 술어 하나**를 보게 해서 그 어긋남을 없앤다
///   (ADR-0218 결정 5 · 술어 정본 = `super::resume_argument`).
///   ★Fresh 인데 손잡이가 실려 오는 조합은 조립점이 막는다★ — `manager::resume_handle_for`
///   (그 doc 과 그 시험이 정본). 그래서 이 칸은 모드를 안 받고도 성립한다.
/// - 기록할 곳이 있다(`sink`). 없으면 받아도 남길 데가 없다.
/// - 자식의 PID 와 **시작시각**을 둘 다 안다. 어느 하나라도 미상이면 판정이 PID 단독 대조로 내려앉아
///   ADR-0218 결정 2 가 금한 모양이 된다 — 그래서 여기서 끊는다.
/// - 락 폴더를 안다.
///
/// ★락 폴더는 **자식의** 환경에서 푼다 — 우리 것으로 대신하지 말 것★: `CODEX_HOME` 은 건 프로세스에만
///   적용되므로, 프로필이 그 값을 걸면 자식의 락은 우리가 보는 폴더에 **영영 안 생긴다**. 그 어긋남은
///   오류가 아니라 「계속 빈손」이라 화면에 아무 신호가 없다.
/// `our_lock_dir` = 자식이 우리 환경을 그대로 물려받았을 때 쓸 값([`lock_dir`]). 인자로 받는 것은
///   이 판정을 env 없이 재기 위해서다.
// ADR-0218
pub(crate) fn plan_capture(
    child_env: &[(String, String)],
    resumes_by_argv: bool,
    child_pid: Option<u32>,
    child_start_time: Option<u64>,
    sink: Option<SessionIdSink>,
    our_lock_dir: Option<PathBuf>,
) -> Option<CapturePlan> {
    if resumes_by_argv {
        return None;
    }
    Some(CapturePlan {
        lock_dir: child_lock_dir(child_env, our_lock_dir)?,
        child_pid: child_pid?,
        child_start_time: child_start_time?,
        sink: sink?,
    })
}

/// [`plan_capture`] 의 폴더 규칙만 — 스폰 env 가 `CODEX_HOME` 을 걸면 그 홈이, 아니면 우리 것이 이긴다.
///
/// ★**마지막** 항목을 고른다★ — 스폰 env 는 목록이고 같은 키가 두 번 실릴 수 있는데, 통로가 그 목록을
///   순서대로 `cmd.env` 에 밀어 넣으므로 자식이 실제로 받는 것은 뒤엣것이다.
/// ★키 대조가 대소문자 무시인 것도 그 자리를 따른다★ — Windows 환경변수 이름은 대소문자를 안 가린다.
/// ★빈 값은 「안 걸었다」로 본다★ — [`codex_home_from`] 이 우리 쪽 env 를 그렇게 읽으므로 같은 규칙을 쓴다.
/// ★알려진 구멍 — 상대 경로★: 그런 값은 나중에 **우리** cwd 기준으로 풀리는데 자식은 자기 cwd 로
///   풀 테니 둘이 갈릴 수 있다. 증상은 오검출이 아니라 「계속 빈손」이다 — 후보 판정은 그대로 자식의
///   PID·시작시각을 요구한다. 실제로 그런 프로필이 있는지는 미확인이라 고치지 않고 적어만 둔다.
fn child_lock_dir(
    child_env: &[(String, String)],
    our_lock_dir: Option<PathBuf>,
) -> Option<PathBuf> {
    let overridden = child_env
        .iter()
        .rev()
        .find(|(key, _)| key.eq_ignore_ascii_case(CODEX_HOME_ENV))
        .map(|(_, value)| value.as_str())
        .filter(|value| !value.is_empty());
    match overridden {
        Some(home) => Some(lock_dir_under(Path::new(home))),
        None => our_lock_dir,
    }
}

/// 회수 폴링을 **별도 스레드**로 띄운다 — 실패해도 스폰은 그대로 간다.
///
/// ★스폰 경로도 pump 도 막지 않는다★: 이 스레드는 `sleep` 과 OS 질의만 하고 세션의 어떤 락도 쥐지
///   않는다. 세션을 붙들지도 않는다 — 쥐고 있는 것은 기록 동사 하나뿐이라, 이 스레드가 남아 있다고
///   해서 에이전트가 늦게 거두어지지 않는다.
/// ★스레드를 못 띄우면 경고만 남기고 끝낸다★ — 실패 모드는 「id 를 못 받음」이고 그것은 스폰을 끊을
///   이유가 아니다(ADR-0218 결정 10 의 같은 처분).
pub(crate) fn spawn_capture(plan: CapturePlan) {
    let clock = ChildClock {
        pid: plan.child_pid,
        start_time: plan.child_start_time,
    };
    tracing::debug!(
        pid = plan.child_pid,
        lock_dir = %plan.lock_dir.display(),
        "codex 세션 id 회수 폴링을 시작한다"
    );
    let spawned = std::thread::Builder::new()
        .name("codex-thread-lock".to_string())
        .spawn(move || run_capture(&plan, &RestartManagerProbe, &clock));
    if let Err(err) = spawned {
        tracing::warn!(%err, "codex 세션 id 회수 스레드를 띄우지 못했다 — 이 화신은 id 를 못 받는다");
    }
}

/// 잡을 때까지 폴링한다 — ★멈추는 조건은 셋뿐이고 시한은 그중에 없다★(ADR-0218 결정 4).
///
/// 1. [`ScanOutcome::Found`] — 채택하고 멈춘다.
/// 2. 자식이 죽었다([`CaptureClock::child_alive`]).
/// 3. 세션이 내려간다 — ★별도 신호가 없는 것이 의도다★: 종료는 통로가 자식을 죽이는 것으로 시작하므로
///    (「kill 인과」) 그 결말이 2 번으로 도착한다. 둘째 신호를 달면 종료 순서에 이 관찰자가 끼어든다.
///
/// ★[`ScanOutcome::Ambiguous`] 는 멈춤이 아니다 — 「이번 바퀴에 채택 안 함」이다★: ADR-0218 결정 3 이
///   말하는 것은 **거절**이고, 결정 4 의 멈춤 목록에 이 결말은 없다. 겹침이 한때뿐일 수 있는데
///   (스레드가 갈리는 순간의 짧은 중첩) 거기서 스레드를 끝내면 그 에이전트는 회수를 **영영** 잃는다.
///   경고는 첫 바퀴에만 낸다 — 매 바퀴 내면 로그가 그 한 사실로 뒤덮인다.
///
/// 채택은 [`CapturePlan::sink`] 로만 나간다 — 그 동사가 값을 버려도(죽은 화신의 기록이라 거절된
/// 경우) 여기서 다른 길로 새지 않고 그대로 끝낸다.
///
/// ★kill 과 기록 사이의 창은 닫지 않았다 — 재 보고 남긴 결론이다★: 생존 확인을 지난 뒤 자식이
///   죽어도 이 바퀴의 기록은 나간다. **그것이 옳다** — 적히는 값은 방금 죽은 그 화신이 실제로 쓰던
///   스레드 id 이고, 다음 활성화가 그 대화를 이어받는 것이 목적이다. 해로운 조합은 그 사이에
///   **다음 화신이 이미 선** 경우 하나인데, 그때는 표식이 갈려 조립점의 화신 가드가 거절한다.
///   ★그 「거절한다」가 왜 빈틈없나 — 결론만 적지 않고 근거를 남긴다(다음 읽는 사람이 다시 유도하지
///   않게)★:
///     1. 화신 표식을 **올리는** 자리는 `ProfileRegistry::epoch_for_spawn` 하나뿐이다. 나머지 한
///        쓰기(`merge_preserving_live`)는 live 값을 그대로 **보존**할 뿐 올리지 않는다.
///     2. `AgentManager::spawn_agent` 이 그것을 spawn 당 정확히 한 번 부르고, 그 호출은 Fresh 가
///        손잡이를 비우는 `clear_session_id` 보다 **앞**이다.
///     그래서 「표식은 그대로인데 손잡이만 비워진」 구간이 없다 — 지각 기록이 옛 표식으로 들어올 수
///     있는 시점에는 비우기가 아직 안 일어났고, 비우기가 일어났으면 표식은 이미 갈려 있다. 프로필이
///     통째로 지워진 경우는 `observe_session_id` 의 조회 자체가 실패해 같은 자리에서 떨어진다.
///   그래서 여기에 취소 신호를 새로 달지 않았다 — 닫을 구멍이 아니라 **의도된 결말**이다.
///
/// ★한 번 잡으면 그 뒤 스레드가 바뀌어도 안 따라간다 — 알려진 미확인이다★: 사용자가 대화 도중
///   새 스레드를 열면 codex 가 락을 갈아 쥘 텐데(미측정), 이 루프는 이미 멈춰 있어 프로필에는 첫
///   스레드 id 가 남는다. 그 상태로 이어받으면 사용자가 마지막으로 보던 대화가 아니라 그 첫 대화가
///   열린다. 「잡으면 즉시 멈춘다」는 ADR-0218 결정 4 의 선택이라 여기서 뒤집지 않는다.
fn run_capture(plan: &CapturePlan, probe: &dyn LockHolderProbe, clock: &dyn CaptureClock) {
    let mut delay = FIRST_DELAY;
    let mut warned_ambiguous = false;
    loop {
        clock.sleep(delay);
        if !clock.child_alive() {
            tracing::debug!(
                pid = plan.child_pid,
                "codex 자식이 먼저 끝나 세션 id 회수를 접는다"
            );
            return;
        }
        match scan_for_child(&plan.lock_dir, plan.child_pid, plan.child_start_time, probe) {
            ScanOutcome::Found(id) => {
                (plan.sink)(&id.to_string());
                return;
            }
            ScanOutcome::Ambiguous(n) => {
                if !warned_ambiguous {
                    warned_ambiguous = true;
                    tracing::warn!(
                        pid = plan.child_pid,
                        candidates = n,
                        lock_dir = %plan.lock_dir.display(),
                        "codex 나무가 쥔 락이 둘 이상이라 이번 바퀴에는 세션 id 를 채택하지 않는다"
                    );
                }
                delay = next_delay(delay);
            }
            ScanOutcome::NotYet => delay = next_delay(delay),
        }
    }
}

/// 빈손 한 바퀴 뒤의 다음 대기 — 두 배씩, [`MAX_DELAY`] 에서 멈춘다(사유 = 그 상수의 doc).
fn next_delay(previous: Duration) -> Duration {
    (previous * 2).min(MAX_DELAY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // ★가짜 나무는 **운영에서 실제로 나오는 모양**이다 — 이것이 평평하면 시험 전체가 거짓이 된다★:
    //   우리가 쥐는 PID 는 `cmd.exe` 래퍼이고(`crate::backend::console_command`) 락을 쥐는 것은 그 아래
    //   `codex.exe` 다. 홀더를 래퍼 PID 로 심어 두면 「래퍼만 대조하는」 구현도 초록이 된다 — 실제로 그렇게
    //   심어 뒀다가 회수가 영영 안 되는 구현을 통과시켰다(적출 2026-09-21).
    const WRAPPER_PID: u32 = 4242;
    const WRAPPER_START: u64 = 134_344_468_098_866_343;
    /// 래퍼의 자식 = 실제로 락을 쥐는 프로세스.
    const CODEX_PID: u32 = 4816;
    const CODEX_START: u64 = 134_344_468_099_111_111;
    /// codex 가 띄운 도구 — 나무가 한 단 더 깊어도 잡아야 한다.
    const TOOL_PID: u32 = 19_468;
    const TOOL_START: u64 = 134_344_468_099_222_222;

    const ID_A: &str = "01a0c2c4-97f5-75e2-8ea0-4855b7a0997d";
    const ID_B: &str = "01a0c2c3-4f4c-7ee2-a858-3db9f72c2e78";

    /// 목록·홀더·나무를 통째로 들고 있는 가짜 — 파일시스템도 OS 도 안 탄다.
    struct FakeProbe {
        entries: Vec<PathBuf>,
        holders: HashMap<PathBuf, Vec<Holder>>,
        tree: Vec<ProcessIdentity>,
    }

    impl Default for FakeProbe {
        /// 기본 나무 = 래퍼 + codex 두 단. 더 깊은 나무가 필요한 시험만 [`FakeProbe::with_tool`] 로 넓힌다.
        fn default() -> Self {
            Self {
                entries: Vec::new(),
                holders: HashMap::new(),
                tree: vec![wrapper_id(), codex_id()],
            }
        }
    }

    impl FakeProbe {
        fn file(mut self, name: &str, holders: &[Holder]) -> Self {
            let path = PathBuf::from("Z:/locks").join(name);
            self.entries.push(path.clone());
            self.holders.insert(path, holders.to_vec());
            self
        }
        fn with_tool(mut self) -> Self {
            self.tree.push(tool_id());
            self
        }
        fn with_no_tree(mut self) -> Self {
            self.tree.clear();
            self
        }
    }

    impl LockHolderProbe for FakeProbe {
        fn holders(&self, path: &Path) -> Vec<Holder> {
            self.holders.get(path).cloned().unwrap_or_default()
        }
        /// 뿌리의 신원이 안 맞으면 나무가 없다 — 실물 `process_tree::subtree` 와 같은 규칙.
        fn our_processes(&self, root_pid: u32, root_start_time: u64) -> Vec<ProcessIdentity> {
            if root_pid != WRAPPER_PID || root_start_time != WRAPPER_START {
                return Vec::new();
            }
            self.tree.clone()
        }
        fn entries(&self, _dir: &Path) -> Vec<PathBuf> {
            self.entries.clone()
        }
    }

    fn wrapper_id() -> ProcessIdentity {
        ProcessIdentity {
            pid: WRAPPER_PID,
            start_time: WRAPPER_START,
        }
    }

    fn codex_id() -> ProcessIdentity {
        ProcessIdentity {
            pid: CODEX_PID,
            start_time: CODEX_START,
        }
    }

    fn tool_id() -> ProcessIdentity {
        ProcessIdentity {
            pid: TOOL_PID,
            start_time: TOOL_START,
        }
    }

    /// 락을 쥐는 쪽 — ★우리가 쥔 PID 가 아니라 그 **자식**이다★.
    fn codex() -> Holder {
        Holder {
            pid: CODEX_PID,
            start_time: CODEX_START,
        }
    }

    fn tool() -> Holder {
        Holder {
            pid: TOOL_PID,
            start_time: TOOL_START,
        }
    }

    /// ★언제나 **래퍼** PID 로 묻는다 — 운영에서 우리가 쥐는 것이 그것이다★.
    fn scan(probe: &FakeProbe) -> ScanOutcome {
        scan_for_child(Path::new("Z:/locks"), WRAPPER_PID, WRAPPER_START, probe)
    }

    fn found(id: &str) -> ScanOutcome {
        ScanOutcome::Found(Uuid::parse_str(id).unwrap())
    }

    // ── 채택 ────────────────────────────────────────────────────────────────────────

    /// ★이 항목이 지키는 것 = 「래퍼 PID 하나만 대조하면 안 된다」★: 묻는 PID 는 래퍼인데 홀더는 그
    /// 자식이다. 대조 상대를 나무로 넓히지 않으면 여기서 `NotYet` 이 나온다.
    #[test]
    fn a_lock_held_by_our_childs_child_is_found() {
        let probe = FakeProbe::default().file(&format!("{ID_A}.lock"), &[codex()]);
        assert_eq!(scan(&probe), found(ID_A));
    }

    /// 나무가 한 단 더 깊어도 잡는다 — codex 가 띄운 도구가 쥔 자리.
    #[test]
    fn a_lock_held_by_a_deeper_descendant_is_found() {
        let probe = FakeProbe::default()
            .with_tool()
            .file(&format!("{ID_A}.lock"), &[tool()]);
        assert_eq!(scan(&probe), found(ID_A));
    }

    /// 열거가 못 본 프로세스가 쥔 락은 후보가 아니다 — 우리 것이라고 말할 근거가 없다.
    #[test]
    fn a_holder_outside_the_reported_tree_is_not_a_candidate() {
        let probe = FakeProbe::default().file(&format!("{ID_A}.lock"), &[tool()]);
        assert_eq!(scan(&probe), ScanOutcome::NotYet);
    }

    /// 나무가 비면(자식이 죽었거나 PID 재사용) 파일을 묻지도 않는다.
    #[test]
    fn an_empty_tree_short_circuits_to_not_yet() {
        let probe = FakeProbe::default()
            .with_no_tree()
            .file(&format!("{ID_A}.lock"), &[codex()]);
        assert_eq!(scan(&probe), ScanOutcome::NotYet);
    }

    /// 뿌리의 신원이 어긋나면 나무 자체가 안 선다 — 남의 PID 나무를 우리 것이라 부르지 않는다.
    #[test]
    fn a_root_whose_start_time_drifted_owns_no_tree() {
        let probe = FakeProbe::default().file(&format!("{ID_A}.lock"), &[codex()]);
        assert_eq!(
            scan_for_child(
                Path::new("Z:/locks"),
                WRAPPER_PID,
                WRAPPER_START - 1,
                &probe
            ),
            ScanOutcome::NotYet
        );
    }

    #[test]
    fn other_holders_on_the_same_file_do_not_block_the_match() {
        let holders = [
            Holder {
                pid: 7,
                start_time: 9,
            },
            codex(),
            Holder {
                pid: 8,
                start_time: 10,
            },
        ];
        let probe = FakeProbe::default().file(&format!("{ID_A}.lock"), &holders);
        assert_eq!(scan(&probe), found(ID_A));
    }

    #[test]
    fn locks_held_by_other_processes_are_ignored() {
        let probe = FakeProbe::default()
            .file(
                &format!("{ID_B}.lock"),
                &[Holder {
                    pid: 999,
                    start_time: 1,
                }],
            )
            .file(&format!("{ID_A}.lock"), &[codex()]);
        assert_eq!(scan(&probe), found(ID_A));
    }

    // ── 두 칸이 **함께** 맞아야 한다(ADR-0218 결정 2) ──────────────────────────────

    #[test]
    fn same_pid_with_a_different_start_time_is_not_a_match() {
        let stale = Holder {
            pid: CODEX_PID,
            start_time: CODEX_START - 1,
        };
        let probe = FakeProbe::default().file(&format!("{ID_A}.lock"), &[stale]);
        assert_eq!(scan(&probe), ScanOutcome::NotYet);
    }

    #[test]
    fn same_start_time_with_a_different_pid_is_not_a_match() {
        let other = Holder {
            pid: CODEX_PID + 1,
            start_time: CODEX_START,
        };
        let probe = FakeProbe::default().file(&format!("{ID_A}.lock"), &[other]);
        assert_eq!(scan(&probe), ScanOutcome::NotYet);
    }

    #[test]
    fn an_unknown_child_start_time_never_matches() {
        let probe = FakeProbe::default().file(
            &format!("{ID_A}.lock"),
            &[Holder {
                pid: WRAPPER_PID,
                start_time: 0,
            }],
        );
        assert_eq!(
            scan_for_child(Path::new("Z:/locks"), WRAPPER_PID, 0, &probe),
            ScanOutcome::NotYet,
            "시작시각 미상(0)이면 남는 것이 PID 단독 대조라 채택하지 않는다"
        );
    }

    #[test]
    fn an_unknown_child_pid_never_matches() {
        let probe = FakeProbe::default().file(
            &format!("{ID_A}.lock"),
            &[Holder {
                pid: 0,
                start_time: WRAPPER_START,
            }],
        );
        assert_eq!(
            scan_for_child(Path::new("Z:/locks"), 0, WRAPPER_START, &probe),
            ScanOutcome::NotYet
        );
    }

    // ── 후보 0 개 / 2 개 이상 ──────────────────────────────────────────────────────

    #[test]
    fn an_empty_directory_is_not_yet() {
        assert_eq!(scan(&FakeProbe::default()), ScanOutcome::NotYet);
    }

    #[test]
    fn a_lock_with_no_holder_is_not_yet() {
        let probe = FakeProbe::default().file(&format!("{ID_A}.lock"), &[]);
        assert_eq!(scan(&probe), ScanOutcome::NotYet);
    }

    #[test]
    fn two_matching_locks_are_ambiguous() {
        let probe = FakeProbe::default()
            .file(&format!("{ID_A}.lock"), &[codex()])
            .file(&format!("{ID_B}.lock"), &[codex()]);
        assert_eq!(scan(&probe), ScanOutcome::Ambiguous(2));
    }

    // ── 이름 판정 ──────────────────────────────────────────────────────────────────

    #[test]
    fn names_that_are_not_a_uuid_lock_are_skipped() {
        let probe = FakeProbe::default()
            .file(".coordination.lock", &[codex()])
            .file("not-a-uuid.lock", &[codex()])
            .file(&format!("{ID_B}.tmp"), &[codex()])
            .file(ID_B, &[codex()])
            .file(&format!("{ID_A}.lock"), &[codex()]);
        assert_eq!(
            scan(&probe),
            found(ID_A),
            "id 가 아닌 이름은 후보 수를 늘리지 않는다"
        );
    }

    #[test]
    fn a_held_lock_with_an_unparseable_name_is_not_a_candidate() {
        let probe = FakeProbe::default().file(".coordination.lock", &[codex()]);
        assert_eq!(scan(&probe), ScanOutcome::NotYet);
    }

    /// ★정규형이 아닌 uuid 이름은 후보가 아니다★ — 받아 주면 우리가 만들지 않은 파일 하나가 후보 수를
    /// 늘려 판정을 `Ambiguous` 로 뒤집고, 그 에이전트의 회수가 통째로 멈춘다.
    #[test]
    fn non_canonical_uuid_names_are_not_candidates() {
        let simple = ID_A.replace('-', "");
        for name in [
            format!("{simple}.lock"),
            format!("{{{ID_A}}}.lock"),
            format!("urn:uuid:{ID_A}.lock"),
        ] {
            let probe = FakeProbe::default().file(&name, &[codex()]);
            assert_eq!(
                scan(&probe),
                ScanOutcome::NotYet,
                "{name} 이 후보로 올라왔다"
            );
        }
    }

    /// 대문자 하이픈형은 받는다 — Windows 파일 이름은 대소문자를 안 가린다.
    #[test]
    fn an_uppercase_canonical_name_is_still_a_candidate() {
        let probe = FakeProbe::default().file(&format!("{}.lock", ID_A.to_uppercase()), &[codex()]);
        assert_eq!(scan(&probe), found(ID_A));
    }

    /// ★세는 것은 파일이 아니라 맞은 **홀더**다★ — 한 락을 우리 프로세스 둘이 쥐고 있으면 「그 락이
    /// 누구 것인가」가 아직 하나로 안 좁혀졌다. 파일만 세면 그 상태가 `Found` 로 통과한다.
    #[test]
    fn two_of_our_processes_on_one_lock_are_ambiguous() {
        let probe = FakeProbe::default()
            .with_tool()
            .file(&format!("{ID_A}.lock"), &[codex(), tool()]);
        assert_eq!(scan(&probe), ScanOutcome::Ambiguous(2));
    }

    /// 남이 함께 쥐고 있는 것은 수를 안 늘린다 — 세는 것은 **우리** 홀더뿐이다.
    #[test]
    fn strangers_sharing_the_lock_do_not_make_it_ambiguous() {
        let probe = FakeProbe::default().file(
            &format!("{ID_A}.lock"),
            &[
                Holder {
                    pid: 7,
                    start_time: 9,
                },
                codex(),
            ],
        );
        assert_eq!(scan(&probe), found(ID_A));
    }

    // ── 경로 산출 ──────────────────────────────────────────────────────────────────

    #[test]
    fn the_lock_dir_hangs_off_the_codex_home() {
        assert_eq!(
            lock_dir_under(Path::new("C:/Users/x/.codex")),
            PathBuf::from("C:/Users/x/.codex").join("thread-writer-locks")
        );
    }

    #[test]
    fn codex_home_prefers_the_env_override() {
        assert_eq!(
            codex_home_from(
                Some(OsString::from("D:/alt-codex")),
                Some(PathBuf::from("C:/Users/x"))
            ),
            Some(PathBuf::from("D:/alt-codex"))
        );
    }

    #[test]
    fn an_empty_or_absent_override_falls_back_to_the_user_home() {
        assert_eq!(
            codex_home_from(Some(OsString::new()), Some(PathBuf::from("C:/Users/x"))),
            Some(PathBuf::from("C:/Users/x").join(".codex"))
        );
        assert_eq!(
            codex_home_from(None, Some(PathBuf::from("C:/Users/x"))),
            Some(PathBuf::from("C:/Users/x").join(".codex"))
        );
    }

    #[test]
    fn no_home_means_no_lock_dir() {
        assert_eq!(codex_home_from(None, None), None);
    }

    // ── 회수 루프(ADR-0218 결정 4·5·6) ───────────────────────────────────────────

    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};

    /// 바퀴 수를 세는 칸 — 가짜 시계가 올리고 가짜 프로브가 읽어, 「n 번째 바퀴에는 이것이 보인다」를
    /// 쓴다. 두 가짜가 같은 칸을 보는 것이 이 묶음의 전부다.
    type Ticks = Rc<Cell<usize>>;

    struct FakeClock {
        ticks: Ticks,
        /// 이 바퀴 수까지만 자식이 살아 있다.
        alive_through: usize,
        slept: RefCell<Vec<Duration>>,
    }

    impl FakeClock {
        fn immortal(ticks: Ticks) -> Self {
            Self {
                ticks,
                alive_through: usize::MAX,
                slept: RefCell::new(Vec::new()),
            }
        }
        fn dies_after(ticks: Ticks, rounds: usize) -> Self {
            Self {
                ticks,
                alive_through: rounds,
                slept: RefCell::new(Vec::new()),
            }
        }
        fn rounds(&self) -> usize {
            self.slept.borrow().len()
        }
    }

    impl CaptureClock for FakeClock {
        fn child_alive(&self) -> bool {
            self.ticks.get() <= self.alive_through
        }
        fn sleep(&self, delay: Duration) {
            self.slept.borrow_mut().push(delay);
            self.ticks.set(self.ticks.get() + 1);
        }
    }

    /// 바퀴마다 다른 폴더 모습을 보여 주는 프로브 — 목록이 바닥나면 마지막 모습이 계속 간다.
    struct ScriptedProbe {
        ticks: Ticks,
        rounds: Vec<FakeProbe>,
    }

    impl ScriptedProbe {
        fn round(&self) -> &FakeProbe {
            let at = self.ticks.get().saturating_sub(1);
            self.rounds
                .get(at)
                .or_else(|| self.rounds.last())
                .expect("적어도 한 바퀴는 적어 둔다")
        }
    }

    impl LockHolderProbe for ScriptedProbe {
        fn holders(&self, path: &Path) -> Vec<Holder> {
            self.round().holders(path)
        }
        /// ★세 메서드를 **전부** 위임한다 — 하나라도 빠뜨리면 그 자리만 조용히 실 OS 를 탄다★.
        /// 실제로 이것을 빠뜨렸더니 나무 조회가 기본 구현으로 새서 가짜 PID 로 진짜 프로세스 표를
        /// 훑었고, 답이 늘 빈손이라 루프가 안 끝났다(실측 2026-09-21 — 세 항목이 60초 넘게 매달렸다).
        fn our_processes(&self, root_pid: u32, root_start_time: u64) -> Vec<ProcessIdentity> {
            self.round().our_processes(root_pid, root_start_time)
        }
        fn entries(&self, dir: &Path) -> Vec<PathBuf> {
            self.round().entries(dir)
        }
    }

    /// 받은 문자열을 그대로 쌓는 기록 동사 — 조립점이 건네는 것과 **같은 타입**이다.
    fn recording_sink() -> (SessionIdSink, Arc<Mutex<Vec<String>>>) {
        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let write = seen.clone();
        let sink: SessionIdSink = Arc::new(move |raw: &str| {
            write.lock().expect("sink poisoned").push(raw.to_string());
        });
        (sink, seen)
    }

    fn plan_with(sink: SessionIdSink) -> CapturePlan {
        CapturePlan {
            lock_dir: PathBuf::from("Z:/locks"),
            child_pid: WRAPPER_PID,
            child_start_time: WRAPPER_START,
            sink,
        }
    }

    fn drive(rounds: Vec<FakeProbe>, clock_for: fn(Ticks) -> FakeClock) -> (Vec<String>, usize) {
        let ticks: Ticks = Rc::new(Cell::new(0));
        let probe = ScriptedProbe {
            ticks: ticks.clone(),
            rounds,
        };
        let clock = clock_for(ticks);
        let (sink, seen) = recording_sink();
        run_capture(&plan_with(sink), &probe, &clock);
        let got = seen.lock().expect("sink poisoned").clone();
        (got, clock.rounds())
    }

    #[test]
    fn the_loop_keeps_waiting_until_the_lock_shows_up_and_then_adopts_it() {
        let (got, rounds) = drive(
            vec![
                FakeProbe::default(),
                FakeProbe::default().file(&format!("{ID_B}.lock"), &[]),
                FakeProbe::default().file(&format!("{ID_A}.lock"), &[codex()]),
            ],
            FakeClock::immortal,
        );
        assert_eq!(
            got,
            vec![ID_A.to_string()],
            "세 번째 바퀴에서 채택해야 한다"
        );
        assert_eq!(rounds, 3, "빈손 두 바퀴를 지나 세 번째에 멈춰야 한다");
    }

    /// ★거절이지 포기가 아니다(ADR-0218 결정 3 대 결정 4)★ — 겹침이 풀리면 그다음 바퀴에 채택한다.
    /// 여기서 스레드를 끝내 버리면 스레드가 갈리는 순간의 짧은 중첩 하나가 그 에이전트의 회수를 영영
    /// 끝낸다.
    #[test]
    fn an_ambiguous_scan_adopts_nothing_but_keeps_polling() {
        let (got, rounds) = drive(
            vec![
                FakeProbe::default()
                    .file(&format!("{ID_A}.lock"), &[codex()])
                    .file(&format!("{ID_B}.lock"), &[codex()]),
                FakeProbe::default().file(&format!("{ID_A}.lock"), &[codex()]),
            ],
            FakeClock::immortal,
        );
        assert_eq!(
            got,
            vec![ID_A.to_string()],
            "겹침이 풀린 다음 바퀴에 채택해야 한다"
        );
        assert_eq!(rounds, 2, "겹친 바퀴에서 스레드를 끝내 버렸다");
    }

    /// 겹침이 안 풀리면 채택은 끝까지 안 한다 — 「거절」 쪽은 그대로다.
    #[test]
    fn an_ambiguous_scan_that_never_clears_adopts_nothing() {
        let (got, rounds) = drive(
            vec![FakeProbe::default()
                .file(&format!("{ID_A}.lock"), &[codex()])
                .file(&format!("{ID_B}.lock"), &[codex()])],
            |ticks| FakeClock::dies_after(ticks, 3),
        );
        assert!(got.is_empty(), "둘 이상인데 채택했다: {got:?}");
        assert_eq!(rounds, 4, "자식이 죽을 때까지는 계속 돌아야 한다");
    }

    #[test]
    fn the_child_exiting_ends_the_loop() {
        let (got, rounds) = drive(vec![FakeProbe::default()], |ticks| {
            FakeClock::dies_after(ticks, 2)
        });
        assert!(got.is_empty(), "자식이 죽었는데 채택했다: {got:?}");
        assert_eq!(rounds, 3, "생존 확인이 실패한 바퀴에서 끝나야 한다");
    }

    /// ★채택은 건네받은 동사 **하나로만** 나간다 — 그 동사가 값을 버려도 둘째 길이 없다★.
    ///
    /// 여기 쓰는 가짜는 죽은 화신의 기록 동사와 같은 모양이다(받되 프로필에는 아무것도 안 남긴다).
    /// 운영 동사에 실제로 화신 가드가 실려 있다는 것은 조립점 쪽
    /// `manager::tests::a_dead_incarnations_sink_cannot_overwrite_a_live_value` 가 잰다 —
    /// `ProfileRegistry` 를 backend 로 들이지 않는 것이 ADR-0004 라, 이 자리는 **거절하는 동사**로 세운다.
    #[test]
    fn a_sink_that_drops_the_value_does_not_make_the_loop_look_elsewhere() {
        let calls = Arc::new(Mutex::new(0usize));
        let count = calls.clone();
        let sink: SessionIdSink = Arc::new(move |_raw: &str| {
            *count.lock().expect("sink poisoned") += 1;
        });
        let ticks: Ticks = Rc::new(Cell::new(0));
        let probe = ScriptedProbe {
            ticks: ticks.clone(),
            rounds: vec![FakeProbe::default().file(&format!("{ID_A}.lock"), &[codex()])],
        };
        let clock = FakeClock::immortal(ticks);

        run_capture(&plan_with(sink), &probe, &clock);

        assert_eq!(
            *calls.lock().expect("sink poisoned"),
            1,
            "기록 동사가 정확히 한 번 불려야 한다"
        );
        assert_eq!(clock.rounds(), 1, "거절을 재시도로 읽고 계속 돌면 안 된다");
    }

    #[test]
    fn the_delay_doubles_and_stops_at_the_ceiling() {
        assert_eq!(next_delay(FIRST_DELAY), FIRST_DELAY * 2);
        assert_eq!(next_delay(MAX_DELAY), MAX_DELAY);
        assert_eq!(
            next_delay(MAX_DELAY / 2 + Duration::from_secs(1)),
            MAX_DELAY
        );
    }

    // ── 회수를 돌릴 조건(ADR-0218 결정 5) ────────────────────────────────────────

    const OUR_HOME: &str = "C:/Users/x/.codex";

    fn ours() -> Option<PathBuf> {
        Some(lock_dir_under(Path::new(OUR_HOME)))
    }

    fn plan(
        env: &[(&str, &str)],
        resumes_by_argv: bool,
        pid: Option<u32>,
        start: Option<u64>,
    ) -> Option<CapturePlan> {
        let env: Vec<(String, String)> = env
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        plan_capture(
            &env,
            resumes_by_argv,
            pid,
            start,
            Some(recording_sink().0),
            ours(),
        )
    }

    /// ★계획은 **건네받은** 기록 동사를 그대로 싣는다★ — 여기서 다른 동사를 지어내면 조립점이 묶어 둔
    /// 화신 가드가 통째로 빠진다(그 가드의 실물은 `manager::session_id_sink`).
    #[test]
    fn the_plan_carries_the_sink_it_was_handed() {
        let (sink, seen) = recording_sink();
        let made = plan_capture(
            &[],
            false,
            Some(WRAPPER_PID),
            Some(WRAPPER_START),
            Some(sink),
            ours(),
        )
        .expect("fresh 는 돈다");

        (made.sink)("probe");

        assert_eq!(
            *seen.lock().expect("sink poisoned"),
            vec!["probe".to_string()],
            "계획이 다른 동사를 들고 있다"
        );
    }

    #[test]
    fn a_fresh_incarnation_gets_a_plan() {
        let made = plan(&[], false, Some(WRAPPER_PID), Some(WRAPPER_START)).expect("fresh 는 돈다");
        assert_eq!(made.child_pid, WRAPPER_PID);
        assert_eq!(made.child_start_time, WRAPPER_START);
        assert_eq!(made.lock_dir, lock_dir_under(Path::new(OUR_HOME)));
    }

    /// ★게이트가 보는 것은 「argv 가 실제로 이어받았나」다★ — 「손잡이가 있나」가 아니다. 그 둘을
    /// 가르는 자리(실을 수 없는 손잡이)는 이 폴더의
    /// `tests::a_handle_we_refuse_to_resume_with_is_still_captured` 가 잰다.
    #[test]
    fn a_resume_incarnation_never_starts_the_loop() {
        assert!(
            plan(&[], true, Some(WRAPPER_PID), Some(WRAPPER_START)).is_none(),
            "이어받기 화신은 id 를 argv 로 이미 들고 나갔다 — 회수할 것이 없다"
        );
    }

    #[test]
    fn an_unknown_pid_or_start_time_never_starts_the_loop() {
        assert!(plan(&[], false, None, Some(WRAPPER_START)).is_none());
        assert!(
            plan(&[], false, Some(WRAPPER_PID), None).is_none(),
            "시작시각을 모르면 남는 것이 PID 단독 대조다"
        );
    }

    #[test]
    fn without_a_sink_or_a_lock_dir_there_is_nothing_to_run() {
        assert!(
            plan_capture(
                &[],
                false,
                Some(WRAPPER_PID),
                Some(WRAPPER_START),
                None,
                ours()
            )
            .is_none(),
            "받아도 남길 데가 없다"
        );
        assert!(plan_capture(
            &[],
            false,
            Some(WRAPPER_PID),
            Some(WRAPPER_START),
            Some(recording_sink().0),
            None,
        )
        .is_none());
    }

    /// ★프로필이 건 `CODEX_HOME` 이 우리 것을 이긴다★ — 그 변수는 건 프로세스에만 적용되므로, 우리
    /// 폴더를 훑으면 그 자식의 락은 영영 안 보인다(오류 없이 계속 빈손).
    #[test]
    fn the_lock_dir_comes_from_the_childs_environment() {
        let made = plan(
            &[("CODEX_HOME", "D:/alt-codex")],
            false,
            Some(WRAPPER_PID),
            Some(WRAPPER_START),
        )
        .expect("fresh 는 돈다");
        assert_eq!(made.lock_dir, lock_dir_under(Path::new("D:/alt-codex")));
    }

    #[test]
    fn the_childs_env_is_matched_case_insensitively_and_last_one_wins() {
        let made = plan(
            &[("CODEX_HOME", "D:/first"), ("codex_home", "D:/second")],
            false,
            Some(WRAPPER_PID),
            Some(WRAPPER_START),
        )
        .expect("fresh 는 돈다");
        assert_eq!(
            made.lock_dir,
            lock_dir_under(Path::new("D:/second")),
            "통로가 순서대로 env 를 밀어 넣으므로 자식이 받는 것은 뒤엣것이다"
        );
    }

    #[test]
    fn an_empty_codex_home_falls_back_to_ours() {
        let made = plan(
            &[("CODEX_HOME", "")],
            false,
            Some(WRAPPER_PID),
            Some(WRAPPER_START),
        )
        .expect("fresh 는 돈다");
        assert_eq!(made.lock_dir, lock_dir_under(Path::new(OUR_HOME)));
    }

    // ── 실 프로브와의 결합(Windows) ────────────────────────────────────────────────

    /// 가짜가 아니라 [`RestartManagerProbe`] 로 한 바퀴 — 이 프로세스가 연 `<uuid>.lock` 을 자식인 척
    /// 찾는다(자기 PID·시작시각으로 묻는다). 같은 폴더에 **안 쥔** 락과 id 가 아닌 이름을 함께 두어
    /// 목록 쪽 규칙도 실물로 지난다.
    #[cfg(windows)]
    #[test]
    fn the_real_probe_finds_a_lock_this_process_holds() {
        let dir = std::env::temp_dir().join(format!(
            "engram-thread-lock-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("임시 락 폴더 생성");
        std::fs::write(dir.join(format!("{ID_B}.lock")), b"").expect("안 쥔 락");
        std::fs::write(dir.join(".coordination.lock"), b"").expect("id 아닌 이름");
        let file = std::fs::File::create(dir.join(format!("{ID_A}.lock"))).expect("쥔 락");

        let me = std::process::id();
        let start = engram_dashboard_base::platform::process_creation_time(me)
            .expect("자기 creation time 조회 가능");
        let outcome = scan_for_child(&dir, me, start, &RestartManagerProbe);

        drop(file);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(outcome, found(ID_A));
    }
}
