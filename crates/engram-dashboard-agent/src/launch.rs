//! 실행 재료 포트 — 붙은 프리셋에서 나온 재료(지침 · 스킬 · 원문 설정)를 manager 가 받아 스폰에 싣는 길.
//!
//! 이 파일이 갖는 것: 재료 출처 [`LaunchKitSource`] 와 기본 구현 [`NoopLaunchKits`] · backend 배치가 디스크에 쓰는
//! [`LaunchFs`] 와 운영 구현 [`StdLaunchFs`] · 배치를 휘발 폴더 안에 가두는 `place_in_volatile` · 배치 결과
//! [`LaunchArtifacts`] · 스폰 실패에서 재료를 거두는 `LaunchKitGuard` · 뜬 화신의 재료를 reaper 가 거둘
//! `LaunchLease` · manager 가 받는 묶음 [`LaunchPorts`].
//! 프리셋 해석은 여기 없다(실물 = 데몬이 꽂는 asset-library 서비스). 플래그 번역도 없다(backend 몫).
// ADR-0334

use std::cmp::Ordering;
use std::fmt;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use engram_dashboard_base::sync;
use engram_dashboard_platform::fs::same_path;
use serde_json::{Number, Value};

use crate::profile::{AgentCommand, AgentProfile};
use crate::types::AgentId;

// ── 계획 · 재료 ───────────────────────────────────────────────────────────────

/// 사유 하나 — 기계가 읽는 코드와 그 인자, 걸린 줄. 막힘 사유와 알림이 같은 꼴을 쓴다.
///
/// `line` = 걸린 에셋 파일의 줄 — 1 부터 센다(첫 줄 = 1). `None` = 줄로 짚을 수 없는 사유.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reason {
    pub code: String,
    pub args: Value,
    pub line: Option<u32>,
}

/// 실행 성공 답에 실리는 알림 — 꼴은 [`Reason`] 그대로다.
pub type Notice = Reason;

// `Value` 에 `Ord` 가 없어 손으로 단다 — `BlockReason` 의 `Ord` 가 이것을 탄다. 순서는 `==` 와 어긋나지 않는다
//   (같다 ⟺ `==`) — 어긋나면 정렬이 패닉할 수 있다.
impl Ord for Reason {
    fn cmp(&self, other: &Self) -> Ordering {
        self.code
            .cmp(&other.code)
            .then_with(|| cmp_json(&self.args, &other.args))
            .then_with(|| self.line.cmp(&other.line))
    }
}

impl PartialOrd for Reason {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// 객체는 키 이름순으로 견준다 — `Value` 의 `==` 는 키 순서를 안 보고, `preserve_order` 기능이 같은 빌드의 다른 crate
//   에서 켜지면 순회 순서가 넣은 순서가 된다.
fn cmp_json(a: &Value, b: &Value) -> Ordering {
    fn rank(v: &Value) -> u8 {
        match v {
            Value::Null => 0,
            Value::Bool(_) => 1,
            Value::Number(_) => 2,
            Value::String(_) => 3,
            Value::Array(_) => 4,
            Value::Object(_) => 5,
        }
    }
    match (a, b) {
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::Number(x), Value::Number(y)) => cmp_number(x, y),
        (Value::String(x), Value::String(y)) => x.cmp(y),
        (Value::Array(x), Value::Array(y)) => {
            for (p, q) in x.iter().zip(y) {
                let o = cmp_json(p, q);
                if o != Ordering::Equal {
                    return o;
                }
            }
            x.len().cmp(&y.len())
        }
        (Value::Object(x), Value::Object(y)) => {
            let mut xs: Vec<_> = x.iter().collect();
            let mut ys: Vec<_> = y.iter().collect();
            xs.sort_by(|p, q| p.0.cmp(q.0));
            ys.sort_by(|p, q| p.0.cmp(q.0));
            for ((pk, pv), (qk, qv)) in xs.iter().zip(&ys) {
                let o = pk.cmp(qk).then_with(|| cmp_json(pv, qv));
                if o != Ordering::Equal {
                    return o;
                }
            }
            xs.len().cmp(&ys.len())
        }
        _ => rank(a).cmp(&rank(b)),
    }
}

// `==` 인 두 수는 같다(-0.0 과 0.0 포함). 아니면 (갈래, 값) 순이고, 그래도 못 가르면 글자로 — 글자 가름은
//   `arbitrary_precision` 기능이 켜져 `==` 가 글자를 견줄 때만 닿는다.
fn cmp_number(a: &Number, b: &Number) -> Ordering {
    if a == b {
        return Ordering::Equal;
    }
    fn kind(n: &Number) -> u8 {
        if n.as_u64().is_some() {
            1
        } else if n.as_i64().is_some() {
            0
        } else {
            2
        }
    }
    let by_value = match (kind(a), kind(b)) {
        (0, 0) => a.as_i64().cmp(&b.as_i64()),
        (1, 1) => a.as_u64().cmp(&b.as_u64()),
        (2, 2) => a
            .as_f64()
            .partial_cmp(&b.as_f64())
            .unwrap_or(Ordering::Equal),
        (x, y) => x.cmp(&y),
    };
    by_value.then_with(|| a.to_string().cmp(&b.to_string()))
}

/// 지침 원천 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRef {
    /// 라이브러리 참조(루트부터의 `/` 경로 또는 기본 제공 `engram:/…`).
    pub path: String,
    /// 그 파일의 절대 경로.
    pub file: PathBuf,
}

/// 실을 스킬 하나 — 이름과 절대 폴더.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillRef {
    pub name: String,
    pub dir: PathBuf,
}

/// 그 백엔드 원문 설정 한 덩이 — `preset` = 그것을 낸 프리셋의 참조.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSetting {
    pub preset: String,
    pub value: Value,
}

/// [`LaunchKitSource::check`] 가 낸 실행 계획.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    /// 에이전트 백엔드 낱말 — [`AgentCommand`] 의 serde `kind` 꼬리표를 소문자로 쓴 것(`claude` · `codex` ·
    /// `shell`). 모든 [`LaunchKitSource`] 구현이 [`backend_word`] 로만 채운다 — 구현마다 따로 만들면 같은 백엔드가
    /// 대소문자 · 철자가 다른 낱말로 갈린다.
    pub backend: String,
    /// 지침 원천 — 합칠 순서대로.
    pub instructions: Vec<SourceRef>,
    pub skills: Vec<SkillRef>,
    /// 그 백엔드 원문 — 붙인 순서대로. 모델 · 강도도 여기 든다(따로 된 칸이 없다).
    pub raw_settings: Vec<RawSetting>,
    /// 합친 지침에 「기본」 의 프라이밍(배포판 `builtin\`)을 넣나. [`LaunchPlan::real`] 일 때만 뜻이 있다 —
    /// `false` 면 아무도 이 칸을 읽지 않고, 프라이밍은 제어 채널이 내는 `priming_file` 그대로다(그 파일은
    /// 백엔드가 제어 채널을 지원하고 · 우편을 쓰고 · MCP 설정을 받고 · 프라이밍 파일이 있을 때만 실린다).
    pub include_priming: bool,
    pub notices: Vec<Notice>,
    /// 실 재료인가. `false` 면 backend 번역이 재료를 하나도 싣지 않아 오늘과 바이트 같은 argv 를 낸다.
    pub real: bool,
}

/// [`LaunchKitSource::prepare`] 가 낸 이 화신의 재료.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchKit {
    pub plan: LaunchPlan,
    /// 이 화신의 휘발 폴더 — 절대 경로. `None` = 폴더를 만들지 않았다(backend 배치는 아무것도 못 쓴다).
    /// backend 배치는 이 폴더 안에만 쓴다(`place_in_volatile` 이 막는다 — 상대 경로면 배치가 실패한다).
    pub volatile_dir: Option<PathBuf>,
    /// 합친 지침 파일. `None` = 실을 지침이 없다.
    pub instructions_file: Option<PathBuf>,
    /// 실을 스킬 — 절대 폴더.
    pub skills: Vec<SkillRef>,
    /// 합친 지침의 해시 — codex JSON 이어받기가 지난 적용값과 견준다. `None` = 실을 지침이 없다.
    pub instructions_hash: Option<String>,
}

// ── 막힘 ──────────────────────────────────────────────────────────────────────

/// 실행을 막는 사유 하나. 변형 순서가 보이는 순서다 — `Ord` 가 이 순서로 가른다(TRD S25 §1-3 8).
///
/// `item` = 걸린 항목의 라이브러리 경로 · `via` = 그 항목을 참조하는 프리셋(온 길).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum BlockReason {
    /// 붙은 파일이 없다.
    Missing {
        item: String,
        via: Vec<String>,
        reason: Reason,
    },
    /// 읽히지 않거나 실을 수 없다 — YAML 문법 · 순환 · 설정 스키마 오류 · codex 가 못 싣는 스킬 · 명령줄 가드 ·
    /// 명령줄 길이 넘침.
    Broken {
        item: String,
        via: Vec<String>,
        reason: Reason,
    },
    /// 충돌 하나 — `items` = 걸린 항목마다 (경로, 온 길).
    Conflict {
        reason: Reason,
        items: Vec<(String, Vec<String>)>,
    },
    /// 라이브러리 폴더가 미설정이거나 없는데 라이브러리 프리셋이 붙었다.
    LibraryUnavailable { reason: Reason },
}

/// [`LaunchKitSource::check`] 의 거절 — 걸린 사유 전부.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchBlocked {
    pub reasons: Vec<BlockReason>,
}

impl fmt::Display for LaunchBlocked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("실행 막힘: ")?;
        if self.reasons.is_empty() {
            return f.write_str("(사유 없음)");
        }
        for (i, r) in self.reasons.iter().enumerate() {
            if i > 0 {
                f.write_str(" · ")?;
            }
            match r {
                BlockReason::Missing { item, reason, .. } => {
                    write!(f, "찾을 수 없음 {item} ({})", reason.code)?
                }
                BlockReason::Broken { item, reason, .. } => {
                    write!(f, "오류 {item} ({})", reason.code)?
                }
                BlockReason::Conflict { reason, items } => {
                    let paths: Vec<&str> = items.iter().map(|(p, _)| p.as_str()).collect();
                    write!(f, "충돌 {} ({})", paths.join(", "), reason.code)?
                }
                BlockReason::LibraryUnavailable { reason } => {
                    write!(f, "라이브러리 폴더 없음 ({})", reason.code)?
                }
            }
        }
        Ok(())
    }
}

impl std::error::Error for LaunchBlocked {}

/// [`LaunchKitSource::prepare`] 의 실패 — manager 가 갈래마다 다른 실패로 옮긴다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrepareError {
    /// 점검 뒤 계획의 원본(지침 · 스킬 · 프리셋 파일)이 사라졌거나 바뀌었다 — 점검이 낸 계획이 이미 낡았다.
    /// manager 는 [`crate::types::PtyError::LaunchRaced`] 로 끊는다(프로세스를 열려고 한 적이 없다).
    // TRD S25 §5-2 4 (c)
    Stale(String),
    /// 그 밖 — 휘발 폴더를 못 만들었다 등. manager 는 `SpawnFailed` 로 끊는다.
    Failed(String),
}

impl fmt::Display for PrepareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale(why) => write!(f, "점검 뒤 실행 재료 원본이 바뀌었다: {why}"),
            Self::Failed(why) => write!(f, "실행 재료 준비 실패: {why}"),
        }
    }
}

impl std::error::Error for PrepareError {}

// ── 재료 출처 포트 ────────────────────────────────────────────────────────────

/// 실행 재료의 출처 — 데몬이 꽂는다(기본 = [`NoopLaunchKits`]).
///
/// manager 는 명부 · 예약 락을 쥐지 않고 부른다 — 구현은 그 안에서 명부를 읽어도 된다(asset-library → manager
/// 한 방향).
// ADR-0334
// ADR-0324
pub trait LaunchKitSource: Send + Sync + 'static {
    /// 이 프로필로 띄울 수 있나 — 막히면 걸린 사유 전부.
    ///
    /// ★읽기만 한다★ — 명부 읽기와 [`crate::profile_edit::ProfileEditPort::workspace_lock`] 까지다.
    /// [`crate::profile_edit::ProfileEditPort::set_attachments`] 를 비롯한 프로필 쓰기는 부르지 않는다: manager 가
    /// 이 호출 전후의 붙임 · 명령 · env 를 견주므로, 여기서 쓰면 그 대조가 늘 어긋나 실행이 경합 실패
    /// ([`crate::types::PtyError::LaunchRaced`])로 끝난다.
    /// ★띄우는 바로 그 에이전트의 `workspace_lock` 은 `Preparing` 이다★ — 이 호출 동안 그 실행의 스폰 예약이 잡혀
    /// 있어 자기 예약을 보는 것이다. 바깥 잠김으로 읽어 막지 말 것(모든 실행이 막힌다). 명부에 아직 없는 프로필(즉석
    /// 생성의 넣기 전 점검)도 이 호출로 온다 — 그 id 의 `workspace_lock` 은 `NotFound` 다(막힘 사유로 읽지 말 것).
    fn check(&self, profile: &AgentProfile) -> Result<LaunchPlan, LaunchBlocked>;

    /// `plan` 대로 이 화신의 재료를 준비한다(휘발 폴더 · 합친 지침 · 스킬 목록). 그 사이 만든 것은 성패와 무관하게
    /// [`LaunchKitSource::release`] 가 거둔다(부르는 쪽은 준비 **앞**에서 거둘 채비를 한다).
    ///
    /// `plan` = 이 실행의 [`LaunchKitSource::check`] 가 낸 그것 — 프로필에서 다시 풀지 않는다. 원본이 점검 뒤 사라졌거나
    /// 바뀌었으면 [`PrepareError::Stale`] 이다(그 밖의 실패와 가른다).
    fn prepare(&self, plan: LaunchPlan, id: AgentId, epoch: u32)
        -> Result<LaunchKit, PrepareError>;

    /// 이 화신의 재료를 거둔다. 몇 번 불려도 되고, 준비한 적 없는 화신이어도 된다.
    ///
    /// 부르는 자리 = 스폰 실패 가드(그 스폰 스레드) · 뜬 화신을 수거한 reaper(전역 단일 직렬 스레드 — 뒤의 모든 수거가
    /// 이 호출 뒤에 선다). 그래서 구현은:
    /// - 유계이고 빠르다 — 기다리기 · 다시 하기 · sleep 이 없다.
    /// - panic 하지 않는다 — reaper 는 가두지 않아 운영에서는 panic 이 데몬을 끝낸다(ADR-0292).
    /// - 정확히 `(id, epoch)` 의 재료만 거둔다 — id 전체를 쓸지 않는다(그 id 의 다음 화신 `prepare(id, e2)` 가 함께 돌
    ///   수 있다).
    /// - 최선 노력이다 — CLI 트리가 아직 살아 휘발 폴더를 쥐고 있을 수 있다: 핸들이 풀리기를 기다리지 않고 못 지운 것은
    ///   남기며(부팅 쓸기가 받친다), 스스로 만들지 않은 재해석 지점은 따라가지 않는다.
    /// - 라이브러리 쓰기 잠금을 잡지 않고, 수거를 기다리는 매니저 길로 되부르지 않는다(reaper 가 자기를 기다린다).
    // TRD S25 §5-2 4 (c2) ⑤
    fn release(&self, id: AgentId, epoch: u32);
}

/// 재료 없음 — 오늘과 같은 실행(백엔드 = 명령 변형, 지침 = 제어 채널의 프라이밍 그대로). 이 계획 · 재료로
/// backend 는 오늘과 바이트 같은 argv 를 낸다(골든 = `tests/launch_argv_golden.rs`).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopLaunchKits;

impl LaunchKitSource for NoopLaunchKits {
    fn check(&self, profile: &AgentProfile) -> Result<LaunchPlan, LaunchBlocked> {
        Ok(LaunchPlan {
            backend: backend_word(&profile.command),
            instructions: Vec::new(),
            skills: Vec::new(),
            raw_settings: Vec::new(),
            // `real` 이 false 라 읽히지 않는다 — 프라이밍을 실을지는 오늘처럼 제어 채널이 정한다(칸 doc).
            include_priming: true,
            notices: Vec::new(),
            real: false,
        })
    }

    fn prepare(
        &self,
        plan: LaunchPlan,
        _id: AgentId,
        _epoch: u32,
    ) -> Result<LaunchKit, PrepareError> {
        Ok(LaunchKit {
            plan,
            volatile_dir: None,
            instructions_file: None,
            skills: Vec::new(),
            instructions_hash: None,
        })
    }

    fn release(&self, _id: AgentId, _epoch: u32) {}
}

/// [`LaunchPlan::backend`] 를 채우는 유일한 길 — `command` 의 serde `kind` 꼬리표를 소문자로.
// 백엔드 이름을 여기 적지 않는다(ADR-0004).
pub fn backend_word(command: &AgentCommand) -> String {
    serde_json::to_value(command)
        .ok()
        .and_then(|v| {
            v.get("kind")
                .and_then(Value::as_str)
                .map(str::to_ascii_lowercase)
        })
        .unwrap_or_default()
}

// ── 배치 포트 ─────────────────────────────────────────────────────────────────

/// backend 배치가 디스크에 쓰는 길 — 휘발 폴더 안에서만 쓴다(작업 폴더에는 쓰지 않는다).
///
/// 지우기가 없는 것은 의도다 — 휘발 폴더 정리는 [`LaunchKitSource::release`] 몫이고, 정션을 따라가 라이브러리
/// 스킬을 지우지 않게 그쪽이 링크를 먼저 뗀다.
///
/// ★운영 구현은 링크를 따라간다★ — 쓰는 경로(`write_file` 의 `path` · `link_dir` 의 `link`)의 조상이 링크면 그 대상
/// 폴더(라이브러리 스킬) 안에 쓴다. 그래서 backend 는 이 포트를 `place_in_volatile` 의 가둠 포트로만 받는다(그쪽이
/// [`LaunchFs::is_link`] 로 막는다). 메모리 구현은 링크 안 쓰기를 거절한다.
// `'static` 을 요구하지 않는다 — 빌린 포트를 감싼 `place_in_volatile` 의 가둠 포트도 이것이다.
// ADR-0334
pub trait LaunchFs: Send + Sync {
    /// `path` 에 `bytes` 를 쓴다 — 있으면 덮고, 없는 부모 폴더는 만든다.
    fn write_file(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;

    /// `link` 자리에 폴더 `target` 을 가리키는 디렉터리 링크(Windows = 정션)를 만든다 — 없는 부모 폴더는
    /// 만든다. `target` 이 절대 경로가 아니거나 NUL 이 있으면 `InvalidInput` 이고 부모 폴더도 만들지 않는다.
    /// `link` 자리에 무엇이든 있으면 `AlreadyExists`.
    fn link_dir(&self, link: &Path, target: &Path) -> io::Result<()>;

    /// 그 자리에 무엇이든 있나 — 링크는 따라가지 않는다(대상이 사라진 링크도 있다고 답한다). 확인이 실패하면
    /// 까닭을 가리지 않고(접근 거부 포함) `false` 다.
    fn exists(&self, path: &Path) -> bool;

    /// 그 자리가 링크인가 — 링크는 따라가지 않는다. 링크 = Windows 의 이름 대리 재해석 지점(정션 · 심볼릭 링크 ·
    /// 마운트 지점) · 그 밖의 심볼릭 링크. 클라우드 자리표시 같은 다른 재해석 지점은 링크가 아니다.
    /// `Ok(false)` = 링크 아닌 것이 있다 · `NotFound` = 없다 · 그 밖의 `Err` = 확인 실패.
    fn is_link(&self, path: &Path) -> io::Result<bool>;
}

/// 운영 배치 — std 파일 쓰기 + platform 디렉터리 링크.
#[derive(Debug, Clone, Copy, Default)]
pub struct StdLaunchFs;

impl LaunchFs for StdLaunchFs {
    fn write_file(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        create_parent(path)?;
        std::fs::write(path, bytes)
    }

    fn link_dir(&self, link: &Path, target: &Path) -> io::Result<()> {
        check_link_target(target)?;
        create_parent(link)?;
        engram_dashboard_platform::fs::link_dir(target, link)
    }

    fn exists(&self, path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_ok()
    }

    // Windows std 의 `is_symlink` = 재해석 지점 중 이름 대리 tag(정션 · 심볼릭 링크)다.
    fn is_link(&self, path: &Path) -> io::Result<bool> {
        std::fs::symlink_metadata(path).map(|m| m.file_type().is_symlink())
    }
}

// platform `link_dir` 도 같은 둘을 보지만 부모 폴더를 만든 뒤라, 거절된 대상이 빈 폴더를 남긴다 — 그 전에 본다.
fn check_link_target(target: &Path) -> io::Result<()> {
    if !target.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "링크 대상은 절대 경로여야 한다",
        ));
    }
    if target.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "링크 대상에 NUL 이 있다",
        ));
    }
    Ok(())
}

fn create_parent(path: &Path) -> io::Result<()> {
    match path.parent() {
        Some(parent) => std::fs::create_dir_all(parent),
        None => Ok(()),
    }
}

/// `place` 를 이 화신의 휘발 폴더에 갇힌 배치 포트로 돌린다 — `backend::materialize` 가 backend 배치를 이것으로만
/// 부른다.
///
/// 막는 쓰기(`write_file` 의 `path` · `link_dir` 의 `link`):
/// - 휘발 폴더 밖 · 휘발 폴더 자신. 고르기 전 원래 철자로 본다 — 앞머리는 휘발 폴더 철자와 대소문자만 달라야 하고
///   ([`engram_dashboard_platform::fs::same_path`] — Windows = 접는다), 구분자 · 접두(`\\?\` 등) 철자가 다르면 밖이다.
/// - 휘발 폴더 뒤를 `\` · `/` 둘 다로 가른 낱말 중 빈 것(겹친 구분자) · `.` · `..` · 끝이 `.` 나 공백인 것 · `:` 가 든
///   것 — 모든 OS 에서 막는다.
///   Windows 는 끝의 점 · 공백을 떼고 열어 다른 이름과 같은 자리가 되고, `:` 는 대체 데이터 스트림이다.
/// - 휘발 폴더 바로 아래부터 쓰는 자리의 부모까지(`write_file` 은 그 자리 자신까지) 이미 있는 것 중 링크
///   ([`LaunchFs::is_link`] — 디스크에 묻는다)가 있으면. 준비가 만든 링크 · 이 배치가 만든 링크 둘 다이고, 대소문자로
///   철자를 바꿔도 디스크가 같은 자리로 답하므로 걸린다(8.3 이름 철자도 — 실측: 막힘, S25 P4b-2 재리뷰 2026-11-01). 그
///   확인이 `NotFound` 아닌 오류여도 막는다.
///
/// 보장하지 않는 것: 확인과 쓰기 사이에 남이 링크를 끼우는 경합 — 배치는 CLI 를 띄우기 전이고 휘발 폴더는 이 화신
/// 몫이라 그 사이 쓰는 이가 없다. 휘발 폴더 자신과 그 위는 보지 않는다(준비가 만든 자리다). 이름 대리 아닌 재해석
/// 지점은 링크로 치지 않는다.
///
/// 막힌 쓰기는 경고 로그 + `PermissionDenied` 이고, 하나라도 있었으면 `place` 의 답과 무관하게 `Err` 다(backend 가 그
/// 오류를 삼켜도 배치는 실패한다 · `place` 도 `Err` 였으면 그 사유를 뒤에 잇는다). 휘발 폴더가 없으면 쓰기를 전부
/// 막는다. `exists` · `is_link` 는 그대로 넘긴다. 휘발 폴더가 절대 경로가 아니면 `place` 를 부르지 않고 `Err` — 상대
/// 경로는 데몬의 현재 폴더 기준이 된다.
// ADR-0334
// TRD S25 §1-5 (나) 6
pub(crate) fn place_in_volatile<T>(
    kit: &LaunchKit,
    fs: &dyn LaunchFs,
    place: impl FnOnce(&dyn LaunchFs) -> Result<T, String>,
) -> Result<T, String> {
    let root = kit.volatile_dir.as_deref();
    if let Some(root) = root.filter(|r| !r.is_absolute()) {
        return Err(format!(
            "휘발 폴더가 절대 경로가 아니다: {}",
            root.display()
        ));
    }
    let confined = VolatileOnly {
        inner: fs,
        root,
        refused: Mutex::new(None),
    };
    let placed = place(&confined);
    if let Some((path, why)) = sync::lock(&confined.refused).take() {
        let refused = format!("휘발 폴더 배치 쓰기를 막았다({why}): {}", path.display());
        return Err(match placed {
            Ok(_) => refused,
            Err(e) => format!("{refused} · 배치 오류: {e}"),
        });
    }
    placed
}

/// [`place_in_volatile`] 의 가둠 포트.
struct VolatileOnly<'a> {
    inner: &'a dyn LaunchFs,
    root: Option<&'a Path>,
    /// 처음 막힌 경로와 그 까닭.
    refused: Mutex<Option<(PathBuf, &'static str)>>,
}

impl VolatileOnly<'_> {
    /// `through_self` = 쓰는 자리 자신이 링크여도 막는다(파일 쓰기).
    fn admit(&self, path: &Path, through_self: bool) -> io::Result<()> {
        let Some(why) = self.refusal(path, through_self) else {
            return Ok(());
        };
        tracing::warn!(path = %path.display(), why, "휘발 폴더 배치 쓰기를 막았다");
        sync::lock(&self.refused).get_or_insert_with(|| (path.to_path_buf(), why));
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("휘발 폴더 배치 쓰기를 막았다: {why}"),
        ))
    }

    fn refusal(&self, path: &Path, through_self: bool) -> Option<&'static str> {
        let Some(root) = self.root else {
            return Some("휘발 폴더 없음");
        };
        if let Some(why) = spelling_refusal(path, root) {
            return Some(why);
        }
        let mut parts = path.components();
        let head: PathBuf = parts.by_ref().take(root.components().count()).collect();
        if !same_path(&head, &root.components().collect::<PathBuf>()) {
            return Some("휘발 폴더 밖");
        }
        let rest: Vec<Component> = parts.collect();
        if rest.is_empty() {
            return Some("휘발 폴더 자신");
        }
        if !rest.iter().all(plain_name) {
            return Some("`..` · 끝의 점 · 공백 · `:` 가 든 이름");
        }
        let walked = if through_self {
            rest.len()
        } else {
            rest.len() - 1
        };
        let mut at = head;
        for name in &rest[..walked] {
            at.push(name);
            match self.inner.is_link(&at) {
                Ok(false) => {}
                Ok(true) => return Some("링크 너머"),
                Err(e) if e.kind() == io::ErrorKind::NotFound => break,
                Err(_) => return Some("링크인지 확인 실패"),
            }
        }
        None
    }
}

/// 원래 철자로 먼저 본다 — `components()` 는 구분자를 고르고 `.` · 겹친 구분자를 떨궈, 그 뒤에 보면 놓친다.
/// 앞머리는 휘발 폴더 철자와 대소문자만 달라야 하고(구분자 · 접두 철자가 다르면 밖), 그 뒤는 `\` · `/` 둘 다로 갈라
/// 낱말마다 본다. UTF-8 아닌 경로는 막는다.
fn spelling_refusal(path: &Path, root: &Path) -> Option<&'static str> {
    let (Some(text), Some(root_text)) = (path.to_str(), root.to_str()) else {
        return Some("UTF-8 아닌 경로");
    };
    let is_sep = |b: u8| b == b'\\' || b == b'/';
    let Some(head) = text.get(..root_text.len()) else {
        return Some("휘발 폴더 밖");
    };
    let separators_match = head
        .bytes()
        .zip(root_text.bytes())
        .all(|(a, b)| a == b || !(is_sep(a) || is_sep(b)));
    if !separators_match || !same_path(Path::new(head), root) {
        return Some("휘발 폴더 밖");
    }
    let mut rest = &text[root_text.len()..];
    if !root_text.bytes().last().is_some_and(is_sep) {
        match rest.bytes().next() {
            None => return Some("휘발 폴더 자신"),
            Some(b) if is_sep(b) => rest = &rest[1..],
            Some(_) => return Some("휘발 폴더 밖"),
        }
    }
    if rest.is_empty() {
        return Some("휘발 폴더 자신");
    }
    let bad = |name: &str| {
        name.is_empty()
            || name == "."
            || name == ".."
            || name.ends_with(['.', ' '])
            || name.contains(':')
    };
    if rest.split(['\\', '/']).any(bad) {
        return Some("빈 낱말 · `.` · `..` · 끝의 점 · 공백 · `:` 가 든 이름");
    }
    None
}

fn plain_name(component: &Component) -> bool {
    let Component::Normal(name) = component else {
        return false;
    };
    let bytes = name.as_encoded_bytes();
    !matches!(bytes.last(), Some(b'.' | b' ')) && !bytes.contains(&b':')
}

impl LaunchFs for VolatileOnly<'_> {
    fn write_file(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        self.admit(path, true)?;
        self.inner.write_file(path, bytes)
    }

    fn link_dir(&self, link: &Path, target: &Path) -> io::Result<()> {
        self.admit(link, false)?;
        self.inner.link_dir(link, target)
    }

    fn exists(&self, path: &Path) -> bool {
        self.inner.exists(path)
    }

    fn is_link(&self, path: &Path) -> io::Result<bool> {
        self.inner.is_link(path)
    }
}

/// backend 배치의 결과 — 그 backend 가 명령줄 · 통로에 실을 경로와 인자 조각. 기본값 = 실을 것 없음.
#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct LaunchArtifacts {
    /// [`LaunchPlan::real`] 그대로.
    pub real: bool,
    pub args: Vec<String>,
    pub instructions_file: Option<PathBuf>,
    /// 스킬을 싣는 뿌리 폴더(휘발 폴더 안).
    pub skill_roots: Vec<PathBuf>,
}

// ── 가드 · 묶음 ───────────────────────────────────────────────────────────────

/// 스폰이 끝나기 전 어디서 실패하든 이 화신의 재료를 거두는 가드 — 무장한 채 떨어지면
/// [`LaunchKitSource::release`] 를 정확히 한 번 부른다. 스폰이 성공하면 [`LaunchKitGuard::disarm`].
///
/// 제어 채널의 `ProvisionGuard` 와 독립이다. `release` 는 디스크를 지우므로 락을 쥔 채 떨어뜨리지 않는다.
// ADR-0334
pub(crate) struct LaunchKitGuard {
    kits: Arc<dyn LaunchKitSource>,
    id: AgentId,
    epoch: u32,
    armed: bool,
}

impl LaunchKitGuard {
    /// [`LaunchKitSource::prepare`] **앞**에서 무장한다 — 준비 도중 실패도 거둔다.
    pub(crate) fn arm(kits: Arc<dyn LaunchKitSource>, id: AgentId, epoch: u32) -> Self {
        Self {
            kits,
            id,
            epoch,
            armed: true,
        }
    }

    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }

    /// 세션에 실을 이 화신의 회수 열쇠 — 가드가 쥔 바로 그 출처(준비를 부른 것)로 거둔다.
    pub(crate) fn lease(&self) -> LaunchLease {
        LaunchLease {
            kits: self.kits.clone(),
            id: self.id,
            epoch: self.epoch,
        }
    }
}

impl Drop for LaunchKitGuard {
    fn drop(&mut self) {
        if self.armed {
            self.armed = false;
            tracing::warn!(
                agent = %self.id,
                epoch = self.epoch,
                "실행 재료 가드 발동 — 세션 등록 전 실패 · 재료 회수"
            );
            self.kits.release(self.id, self.epoch);
        }
    }
}

/// 뜬 화신의 재료를 거둘 열쇠 — 세션이 들고, 그 세션을 명부에서 뺀 reaper 가 꺼내 한 번 쓴다.
///
/// 한 번만 쓰는 것을 타입이 지킨다 — `Clone` 이 없고 [`LaunchLease::release`] 가 열쇠를 먹으며, 세션은 꺼내기로만
/// 내준다(`AgentSession::take_launch_lease`).
/// ★`Drop` 을 달지 말 것★: 세션 `Arc` 는 관측자 · 하네스도 쥐어 어디서 마지막으로 떨어질지 모른다 — 떨어질 때 거두면
///   거두는 이가 둘이 된다. 세션 등록 뒤 거두는 이는 화신 표식으로 거른 reaper 하나다.
/// 출처는 [`LaunchKitGuard::lease`] 가 준다 — 매니저 생성 때 잡은 포트가 아니라 그 화신을 준비한 포트다.
// ADR-0334
// TRD S25 §5-2 4 (c2) ② · ③
pub(crate) struct LaunchLease {
    kits: Arc<dyn LaunchKitSource>,
    id: AgentId,
    epoch: u32,
}

impl LaunchLease {
    #[cfg(test)]
    pub(crate) fn new(kits: Arc<dyn LaunchKitSource>, id: AgentId, epoch: u32) -> Self {
        Self { kits, id, epoch }
    }

    pub(crate) fn epoch(&self) -> u32 {
        self.epoch
    }

    /// 거둔다 — 락을 하나도 쥐지 않은 채 부른다(`release` 는 디스크를 지운다).
    pub(crate) fn release(self) {
        self.kits.release(self.id, self.epoch);
    }
}

/// manager 가 받는 실행 재료 포트 둘.
#[derive(Clone)]
pub struct LaunchPorts {
    pub kits: Arc<dyn LaunchKitSource>,
    pub fs: Arc<dyn LaunchFs>,
}

impl LaunchPorts {
    /// 재료 없음 + 운영 배치 — 오늘과 같은 실행.
    pub fn noop() -> Self {
        Self {
            kits: Arc::new(NoopLaunchKits),
            fs: Arc::new(StdLaunchFs),
        }
    }
}

// ── 시험용 메모리 배치 ─────────────────────────────────────────────────────────

/// 디스크 없는 [`LaunchFs`] — 폴더는 항목의 조상으로만 있고, 링크 안으로는 쓰지 않는다.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct MemLaunchFs {
    entries: std::sync::Mutex<std::collections::BTreeMap<PathBuf, MemEntry>>,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
enum MemEntry {
    File(Vec<u8>),
    Link(PathBuf),
}

#[cfg(test)]
impl MemLaunchFs {
    /// 쓴 자리 전부(파일 · 링크 — 저절로 생긴 부모 폴더는 들지 않는다), 경로순.
    pub(crate) fn paths(&self) -> Vec<PathBuf> {
        engram_dashboard_base::sync::lock(&self.entries)
            .keys()
            .cloned()
            .collect()
    }

    pub(crate) fn file(&self, path: &Path) -> Option<Vec<u8>> {
        match engram_dashboard_base::sync::lock(&self.entries).get(path) {
            Some(MemEntry::File(bytes)) => Some(bytes.clone()),
            _ => None,
        }
    }

    pub(crate) fn link_target(&self, path: &Path) -> Option<PathBuf> {
        match engram_dashboard_base::sync::lock(&self.entries).get(path) {
            Some(MemEntry::Link(target)) => Some(target.clone()),
            _ => None,
        }
    }

    fn occupied(entries: &std::collections::BTreeMap<PathBuf, MemEntry>, path: &Path) -> bool {
        entries.contains_key(path) || entries.keys().any(|k| k != path && k.starts_with(path))
    }

    fn parent_is_not_a_dir(
        entries: &std::collections::BTreeMap<PathBuf, MemEntry>,
        path: &Path,
    ) -> bool {
        path.ancestors().skip(1).any(|a| entries.contains_key(a))
    }
}

#[cfg(test)]
impl LaunchFs for MemLaunchFs {
    fn write_file(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut entries = engram_dashboard_base::sync::lock(&self.entries);
        if Self::parent_is_not_a_dir(&entries, path) {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "부모가 폴더가 아니다",
            ));
        }
        let is_dir = entries.keys().any(|k| k != path && k.starts_with(path));
        if is_dir || matches!(entries.get(path), Some(MemEntry::Link(_))) {
            return Err(io::Error::new(
                io::ErrorKind::IsADirectory,
                "폴더 · 링크 자리다",
            ));
        }
        entries.insert(path.to_path_buf(), MemEntry::File(bytes.to_vec()));
        Ok(())
    }

    fn link_dir(&self, link: &Path, target: &Path) -> io::Result<()> {
        check_link_target(target)?;
        let mut entries = engram_dashboard_base::sync::lock(&self.entries);
        if Self::parent_is_not_a_dir(&entries, link) {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "부모가 폴더가 아니다",
            ));
        }
        if Self::occupied(&entries, link) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "링크 자리에 이미 있다",
            ));
        }
        entries.insert(link.to_path_buf(), MemEntry::Link(target.to_path_buf()));
        Ok(())
    }

    fn exists(&self, path: &Path) -> bool {
        Self::occupied(&engram_dashboard_base::sync::lock(&self.entries), path)
    }

    fn is_link(&self, path: &Path) -> io::Result<bool> {
        let entries = engram_dashboard_base::sync::lock(&self.entries);
        match entries.get(path) {
            Some(MemEntry::Link(_)) => Ok(true),
            Some(MemEntry::File(_)) => Ok(false),
            None if Self::occupied(&entries, path) => Ok(false),
            None => Err(io::Error::from(io::ErrorKind::NotFound)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use serde_json::json;
    use uuid::Uuid;

    use crate::profile::AgentOutputFormat;

    fn profile(command: AgentCommand) -> AgentProfile {
        AgentProfile::new(
            "golden".to_string(),
            command,
            PathBuf::from("work"),
            Vec::new(),
            false,
        )
    }

    fn claude() -> AgentCommand {
        AgentCommand::Claude {
            extra_args: vec!["--x".to_string()],
            output_format: AgentOutputFormat::Terminal,
        }
    }

    fn reason(code: &str, args: Value, line: Option<u32>) -> Reason {
        Reason {
            code: code.to_string(),
            args,
            line,
        }
    }

    /// 부른 `release` 를 센다.
    #[derive(Default)]
    struct CountingKits {
        released: Mutex<Vec<(AgentId, u32)>>,
    }

    impl LaunchKitSource for CountingKits {
        fn check(&self, profile: &AgentProfile) -> Result<LaunchPlan, LaunchBlocked> {
            NoopLaunchKits.check(profile)
        }
        fn prepare(
            &self,
            plan: LaunchPlan,
            id: AgentId,
            epoch: u32,
        ) -> Result<LaunchKit, PrepareError> {
            NoopLaunchKits.prepare(plan, id, epoch)
        }
        fn release(&self, id: AgentId, epoch: u32) {
            self.released.lock().unwrap().push((id, epoch));
        }
    }

    // ── Noop ──

    #[test]
    fn noop_check_plans_no_real_kit_and_names_the_command_variant() {
        let codex = AgentCommand::Codex {
            extra_args: Vec::new(),
            output_format: AgentOutputFormat::StreamJson,
        };
        let shell = AgentCommand::Shell {
            program: "sh".to_string(),
            args: Vec::new(),
        };
        for (command, word) in [(claude(), "claude"), (codex, "codex"), (shell, "shell")] {
            let plan = NoopLaunchKits
                .check(&profile(command))
                .expect("Noop 은 막지 않는다");
            assert_eq!(
                plan,
                LaunchPlan {
                    backend: word.to_string(),
                    instructions: Vec::new(),
                    skills: Vec::new(),
                    raw_settings: Vec::new(),
                    include_priming: true,
                    notices: Vec::new(),
                    real: false,
                }
            );
        }
    }

    #[test]
    fn noop_prepare_makes_a_kit_without_a_folder_and_release_does_nothing() {
        let p = profile(claude());
        let plan = NoopLaunchKits.check(&p).unwrap();
        let kit = NoopLaunchKits.prepare(plan.clone(), p.id, 3).unwrap();
        assert_eq!(
            kit,
            LaunchKit {
                plan,
                volatile_dir: None,
                instructions_file: None,
                skills: Vec::new(),
                instructions_hash: None,
            }
        );
        NoopLaunchKits.release(p.id, 3);
        NoopLaunchKits.release(p.id, 3);
    }

    #[test]
    fn noop_ports_plan_no_real_kit() {
        let ports = LaunchPorts::noop();
        assert!(!ports.kits.check(&profile(claude())).unwrap().real);
    }

    // ── 가드 ──

    #[test]
    fn guard_releases_exactly_once_when_dropped_armed() {
        let kits = Arc::new(CountingKits::default());
        let id = Uuid::from_u128(7);
        drop(LaunchKitGuard::arm(kits.clone(), id, 4));
        assert_eq!(*kits.released.lock().unwrap(), vec![(id, 4)]);
    }

    #[test]
    fn guard_never_releases_after_disarm() {
        let kits = Arc::new(CountingKits::default());
        let mut guard = LaunchKitGuard::arm(kits.clone(), Uuid::from_u128(7), 4);
        guard.disarm();
        drop(guard);
        assert!(kits.released.lock().unwrap().is_empty());
    }

    #[test]
    fn guard_releases_on_an_early_return() {
        fn fails(kits: Arc<dyn LaunchKitSource>) -> Result<(), String> {
            let mut guard = LaunchKitGuard::arm(kits, Uuid::from_u128(9), 1);
            Err::<(), _>("준비 실패".to_string())?;
            guard.disarm();
            Ok(())
        }
        let kits = Arc::new(CountingKits::default());
        assert!(fails(kits.clone()).is_err());
        assert_eq!(
            *kits.released.lock().unwrap(),
            vec![(Uuid::from_u128(9), 1)]
        );
    }

    #[test]
    fn ports_and_guard_cross_threads() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<LaunchPorts>();
        send_sync::<LaunchKitGuard>();
    }

    // ── 막힘 ──

    #[test]
    fn block_reasons_sort_in_the_declared_order() {
        let r = || reason("x", Value::Null, None);
        let mut reasons = vec![
            BlockReason::LibraryUnavailable { reason: r() },
            BlockReason::Conflict {
                reason: r(),
                items: Vec::new(),
            },
            BlockReason::Broken {
                item: "a".to_string(),
                via: Vec::new(),
                reason: r(),
            },
            BlockReason::Missing {
                item: "z".to_string(),
                via: Vec::new(),
                reason: r(),
            },
        ];
        reasons.sort();
        let order: Vec<&str> = reasons
            .iter()
            .map(|b| match b {
                BlockReason::Missing { .. } => "missing",
                BlockReason::Broken { .. } => "broken",
                BlockReason::Conflict { .. } => "conflict",
                BlockReason::LibraryUnavailable { .. } => "library",
            })
            .collect();
        assert_eq!(order, ["missing", "broken", "conflict", "library"]);
    }

    #[test]
    fn reason_order_agrees_with_equality() {
        let samples = [
            json!(null),
            json!(false),
            json!(true),
            json!(-3),
            json!(0),
            json!(7),
            json!(u64::MAX),
            json!(0.0),
            json!(-0.0),
            json!(2.5),
            json!("a"),
            json!("b"),
            json!([]),
            json!([1, 2]),
            json!([1, 3]),
            json!({}),
            json!({"a": 1, "b": 2}),
            json!({"b": 2, "a": 1}),
            json!({"a": 1, "c": 0}),
        ];
        let reasons: Vec<Reason> = samples
            .iter()
            .flat_map(|v| {
                [
                    reason("c", v.clone(), None),
                    reason("c", v.clone(), Some(2)),
                ]
            })
            .collect();
        for a in &reasons {
            for b in &reasons {
                assert_eq!(a.cmp(b) == Ordering::Equal, a == b, "{a:?} vs {b:?}");
                assert_eq!(a.cmp(b), b.cmp(a).reverse(), "{a:?} vs {b:?}");
                for c in &reasons {
                    if a.cmp(b) != Ordering::Greater && b.cmp(c) != Ordering::Greater {
                        assert_ne!(a.cmp(c), Ordering::Greater, "{a:?} ≤ {b:?} ≤ {c:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn blocked_display_names_each_reason() {
        let blocked = LaunchBlocked {
            reasons: vec![
                BlockReason::Missing {
                    item: "team/a.yaml".to_string(),
                    via: vec!["team/base.yaml".to_string()],
                    reason: reason("missing-file", Value::Null, None),
                },
                BlockReason::Conflict {
                    reason: reason("skill-name", json!({"name": "s"}), None),
                    items: vec![
                        ("x/s".to_string(), Vec::new()),
                        ("y/s".to_string(), Vec::new()),
                    ],
                },
                BlockReason::LibraryUnavailable {
                    reason: reason("root-unset", Value::Null, None),
                },
            ],
        };
        assert_eq!(
            blocked.to_string(),
            "실행 막힘: 찾을 수 없음 team/a.yaml (missing-file) · 충돌 x/s, y/s (skill-name) · 라이브러리 폴더 없음 (root-unset)"
        );
    }

    // ── 배치 ──

    fn mem_root() -> PathBuf {
        std::env::temp_dir().join("engram-mem-launch-fs")
    }

    #[test]
    fn mem_fs_writes_with_implicit_parents_and_overwrites() {
        let fs = MemLaunchFs::default();
        let file = mem_root().join("v").join("engram").join("settings.json");
        fs.write_file(&file, b"one").unwrap();
        fs.write_file(&file, b"two").unwrap();
        assert_eq!(fs.file(&file).as_deref(), Some(&b"two"[..]));
        assert!(fs.exists(&file));
        assert!(fs.exists(&mem_root().join("v")));
        assert!(!fs.exists(&mem_root().join("w")));
    }

    #[test]
    fn mem_fs_links_once_and_wants_an_absolute_target() {
        let fs = MemLaunchFs::default();
        let link = mem_root().join("v").join("skills").join("s");
        let target = mem_root().join("library").join("s");
        for bad in [PathBuf::from("relative"), mem_root().join("tar\0get")] {
            assert_eq!(
                fs.link_dir(&link, &bad).unwrap_err().kind(),
                io::ErrorKind::InvalidInput,
                "{bad:?}"
            );
        }
        assert!(fs.paths().is_empty());
        fs.link_dir(&link, &target).unwrap();
        assert_eq!(fs.link_target(&link), Some(target.clone()));
        assert!(fs.exists(&link));
        assert_eq!(
            fs.link_dir(&link, &target).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(
            fs.link_dir(&mem_root().join("v"), &target)
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert!(fs.write_file(&link, b"x").is_err());
        assert!(fs.write_file(&link.join("through"), b"x").is_err());
    }

    #[test]
    fn mem_fs_paths_are_the_written_files_and_links_only() {
        let fs = MemLaunchFs::default();
        let volatile = mem_root().join("v");
        let file = volatile.join("engram").join("settings.json");
        let link = volatile.join("skills").join("s");
        fs.write_file(&file, b"x").unwrap();
        fs.link_dir(&link, &mem_root().join("library").join("s"))
            .unwrap();
        assert_eq!(fs.paths(), vec![file, link]);
    }

    // ── 휘발 폴더 가둠 ──

    fn kit_in(volatile_dir: Option<PathBuf>) -> LaunchKit {
        let p = profile(claude());
        let mut kit = NoopLaunchKits
            .prepare(NoopLaunchKits.check(&p).unwrap(), p.id, 1)
            .unwrap();
        kit.volatile_dir = volatile_dir;
        kit
    }

    /// ★배치 쓰기는 휘발 폴더 안에서만 받은 포트로 간다★ — 밖 · 폴더 자신 · `..` 로 빠지는 경로 · 끝이 점 · 공백이거나
    /// `:` 가 든 이름 · 이 배치가 만든 링크 너머는 포트에 닿지 않고, 하나라도 있었으면 배치 전체가 실패한다(backend 가
    /// 그 오류를 삼켜도).
    // TRD S25 §1-5 (나) 6
    #[test]
    fn placement_writes_reach_the_fs_only_inside_the_volatile_folder() {
        let volatile = mem_root().join("launch").join("a-1");
        let library = mem_root().join("library").join("s");
        let fs = MemLaunchFs::default();
        let placed = place_in_volatile(&kit_in(Some(volatile.clone())), &fs, |fs| {
            fs.write_file(&volatile.join("engram").join("settings.json"), b"{}")
                .map_err(|e| e.to_string())?;
            fs.link_dir(&volatile.join("engram").join("skills").join("s"), &library)
                .map_err(|e| e.to_string())?;
            assert!(fs.exists(&volatile.join("engram")), "확인은 그대로 넘긴다");
            Ok("배치함")
        });
        assert_eq!(placed, Ok("배치함"));
        assert_eq!(
            fs.paths(),
            vec![
                volatile.join("engram").join("settings.json"),
                volatile.join("engram").join("skills").join("s"),
            ]
        );

        let work = mem_root().join("work");
        // 원래 철자로 만든 경로 — `join` 은 이 꼴을 못 만든다.
        let main = std::path::MAIN_SEPARATOR;
        let alt = if main == '\\' { '/' } else { '\\' };
        let root_text = volatile.to_string_lossy().into_owned();
        let last_sep = root_text.rfind(main).unwrap();
        let mut mixed_root = root_text.clone();
        mixed_root.replace_range(last_sep..last_sep + 1, &alt.to_string());
        let spelled = |s: String| PathBuf::from(s);
        let outside: Vec<(&str, PathBuf)> = vec![
            (
                "대문자 · 다른 구분자 앞머리 + `.`",
                spelled(format!(
                    "{}{alt}.{alt}sub\\x",
                    root_text.to_uppercase().replace(main, &alt.to_string())
                )),
            ),
            (
                "앞머리에 섞인 구분자",
                spelled(format!("{mixed_root}{main}engram{main}x")),
            ),
            (
                "겹친 구분자",
                spelled(format!("{root_text}{main}{main}engram{main}x")),
            ),
            (
                "`.` 낱말",
                spelled(format!("{root_text}{main}.{main}engram{main}x")),
            ),
            ("작업 폴더", work.join("AGENTS.override.md")),
            ("휘발 폴더 자신", volatile.clone()),
            (
                "이웃 화신",
                mem_root().join("launch").join("a-10").join("x"),
            ),
            ("`..` 로 빠짐", volatile.join("..").join("a-2").join("x")),
            (
                "링크 너머",
                volatile
                    .join("engram")
                    .join("skills")
                    .join("s")
                    .join("SKILL.md"),
            ),
            (
                "링크 자리 자신",
                volatile.join("engram").join("skills").join("s"),
            ),
            ("끝의 점", volatile.join("engram.").join("x")),
            ("끝의 공백", volatile.join("engram ").join("x")),
            ("스트림", volatile.join("engram").join("x:y")),
            ("점만 든 이름", volatile.join("...").join("x")),
        ];
        for (what, path) in outside {
            let fs = MemLaunchFs::default();
            let placed = place_in_volatile(&kit_in(Some(volatile.clone())), &fs, |fs| {
                fs.link_dir(&volatile.join("engram").join("skills").join("s"), &library)
                    .map_err(|e| e.to_string())?;
                let refused = fs.write_file(&path, b"x").unwrap_err();
                assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied, "{what}");
                Ok(())
            });
            assert!(
                matches!(&placed, Err(m) if m.contains(&path.display().to_string())),
                "{what}: {placed:?}"
            );
            assert_eq!(
                fs.paths(),
                vec![volatile.join("engram").join("skills").join("s")],
                "{what}: 막은 쓰기가 포트에 닿았다"
            );
        }

        // 링크도 같다 — 막힌 링크 자리는 포트에 닿지 않는다.
        let fs = MemLaunchFs::default();
        let placed = place_in_volatile(&kit_in(Some(volatile.clone())), &fs, |fs| {
            let _ = fs.link_dir(&work.join(".agents").join("skills").join("s"), &library);
            Ok(())
        });
        assert!(placed.is_err(), "삼킨 오류도 배치 실패다");
        assert!(fs.paths().is_empty());

        // backend 가 제 사유로 실패했으면 그 사유를 막은 쓰기 뒤에 잇는다(갈아치우지 않는다).
        let placed: Result<(), String> =
            place_in_volatile(&kit_in(Some(volatile.clone())), &fs, |fs| {
                let _ = fs.write_file(&work.join("x"), b"x");
                Err("backend 사유".to_string())
            });
        let message = placed.unwrap_err();
        assert!(
            message.contains(&work.join("x").display().to_string())
                && message.contains("backend 사유"),
            "{message}"
        );
    }

    /// 앞머리는 OS 처럼 대소문자를 접어 견준다 — 접는 OS(Windows)에서는 대소문자만 다른 휘발 폴더 철자도 안이고, 접지
    /// 않는 OS 에서는 밖이다.
    #[test]
    fn placement_compares_the_volatile_prefix_as_the_os_does() {
        let volatile = mem_root().join("launch").join("a-1");
        let spelled = PathBuf::from(volatile.to_string_lossy().to_uppercase())
            .join("engram")
            .join("x");
        let folds = same_path(Path::new("a"), Path::new("A"));
        let fs = MemLaunchFs::default();
        let placed = place_in_volatile(&kit_in(Some(volatile)), &fs, |fs| {
            fs.write_file(&spelled, b"x").map_err(|e| e.to_string())
        });
        assert_eq!(placed.is_ok(), folds, "{placed:?}");
    }

    /// 조상이 링크인지 확인하지 못하면(`NotFound` 아닌 오류) 막는다.
    #[test]
    fn placement_refuses_when_a_link_check_fails() {
        struct Unsure(MemLaunchFs);
        impl LaunchFs for Unsure {
            fn write_file(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
                self.0.write_file(path, bytes)
            }
            fn link_dir(&self, link: &Path, target: &Path) -> io::Result<()> {
                self.0.link_dir(link, target)
            }
            fn exists(&self, path: &Path) -> bool {
                self.0.exists(path)
            }
            fn is_link(&self, _path: &Path) -> io::Result<bool> {
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            }
        }
        let volatile = mem_root().join("launch").join("a-1");
        let fs = Unsure(MemLaunchFs::default());
        let placed = place_in_volatile(&kit_in(Some(volatile.clone())), &fs, |fs| {
            fs.write_file(&volatile.join("engram").join("x"), b"x")
                .map_err(|e| e.to_string())
        });
        assert!(placed.is_err(), "{placed:?}");
        assert!(fs.0.paths().is_empty());
    }

    /// 휘발 폴더가 없으면 쓰기를 전부 막고, 상대 경로면 배치를 부르지도 않는다.
    #[test]
    fn placement_without_an_absolute_volatile_folder_writes_nothing() {
        let fs = MemLaunchFs::default();
        let placed = place_in_volatile(&kit_in(None), &fs, |fs| {
            let _ = fs.write_file(&mem_root().join("x"), b"x");
            Ok(())
        });
        assert!(placed.is_err());
        assert_eq!(
            place_in_volatile(&kit_in(None), &fs, |_| Ok(7)),
            Ok(7),
            "쓰지 않는 배치는 선다"
        );
        assert!(fs.paths().is_empty());

        let called = std::cell::Cell::new(false);
        let placed = place_in_volatile(
            &kit_in(Some(PathBuf::from("relative").join("launch"))),
            &fs,
            |_| {
                called.set(true);
                Ok(())
            },
        );
        assert!(
            matches!(&placed, Err(m) if m.contains("절대 경로")),
            "{placed:?}"
        );
        assert!(!called.get(), "상대 경로 휘발 폴더로 배치를 불렀다");
    }

    /// 지나면 지워지는 임시 폴더 — 안의 링크는 먼저 뗀다.
    struct Scratch {
        root: PathBuf,
        links: Vec<PathBuf>,
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            for link in &self.links {
                let _ = engram_dashboard_platform::fs::unlink_dir(link);
            }
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn std_fs_writes_and_links_with_missing_parents() {
        let mut scratch = Scratch {
            root: std::env::temp_dir().join(format!("engram-launch-fs-{}", Uuid::new_v4())),
            links: Vec::new(),
        };
        let fs = StdLaunchFs;
        let target = scratch.root.join("library").join("skill");
        fs.write_file(&target.join("SKILL.md"), b"body").unwrap();
        assert!(fs.exists(&target.join("SKILL.md")));

        let link = scratch.root.join("volatile").join("skills").join("skill");
        scratch.links.push(link.clone());
        fs.link_dir(&link, &target).unwrap();
        assert_eq!(std::fs::read(link.join("SKILL.md")).unwrap(), b"body");
        assert_eq!(
            fs.link_dir(&link, &target).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );

        let dangling = scratch.root.join("volatile").join("skills").join("gone");
        scratch.links.push(dangling.clone());
        fs.link_dir(&dangling, &scratch.root.join("nowhere"))
            .unwrap();
        assert!(fs.exists(&dangling));
        assert!(!fs.exists(&scratch.root.join("nowhere")));
    }

    #[test]
    fn std_fs_rejects_a_bad_link_target_before_making_parents() {
        let scratch = Scratch {
            root: std::env::temp_dir().join(format!("engram-launch-fs-{}", Uuid::new_v4())),
            links: Vec::new(),
        };
        let link = scratch.root.join("volatile").join("skills").join("skill");
        for bad in [PathBuf::from("relative"), scratch.root.join("tar\0get")] {
            assert_eq!(
                StdLaunchFs.link_dir(&link, &bad).unwrap_err().kind(),
                io::ErrorKind::InvalidInput,
                "{bad:?}"
            );
        }
        assert!(!scratch.root.exists(), "거절된 대상이 부모 폴더를 남겼다");
    }

    /// ★실 디스크에서 철자를 바꿔도 링크를 지나 쓰지 않는다★ — 준비가 휘발 폴더에 만든 링크(Windows = 정션 · 그 밖 =
    /// 심볼릭 링크)를 대소문자 · 끝의 점 · 끝의 공백 · 스트림 이름으로 비켜 쓰거나 링크를 겹쳐 만들어도 라이브러리 스킬
    /// 폴더가 그대로다. 대소문자만 다른 철자는 볼륨이 접을 때만 같은 자리라, 접지 않는 볼륨에서는 휘발 폴더 안 새
    /// 폴더로 들어가 들인다 — 그 갈래는 디스크에 물어 가른다(OS 로 가르지 않는다).
    // TRD S25 §1-5 (나) 6
    #[test]
    fn placement_never_writes_through_a_link_on_a_real_disk() {
        let mut scratch = Scratch {
            root: std::env::temp_dir().join(format!("engram-launch-fs-{}", Uuid::new_v4())),
            links: Vec::new(),
        };
        let library = scratch.root.join("library").join("skills");
        let skill = library.join("s").join("SKILL.md");
        StdLaunchFs.write_file(&skill, b"body").unwrap();
        let volatile = scratch.root.join("launch").join("a-1");
        std::fs::create_dir_all(&volatile).unwrap();
        let link = volatile.join("skills");
        scratch.links.push(link.clone());
        engram_dashboard_platform::fs::link_dir(&library, &link).unwrap();
        let folds = StdLaunchFs.exists(&volatile.join("SKILLS"));
        let elsewhere = scratch.root.join("elsewhere");

        let library_untouched = |what: &str| {
            assert_eq!(std::fs::read(&skill).unwrap(), b"body", "{what}");
            let mut names: Vec<String> = std::fs::read_dir(&library)
                .unwrap()
                .chain(std::fs::read_dir(library.join("s")).unwrap())
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            assert_eq!(names, ["SKILL.md", "s"], "{what}: 라이브러리에 새 항목");
        };

        enum Write {
            File(PathBuf),
            Link(PathBuf),
        }
        let attempts = [
            ("그대로", Write::File(link.join("s").join("x.md")), true),
            (
                "덮어쓰기",
                Write::File(link.join("s").join("SKILL.md")),
                true,
            ),
            ("링크 자리 자신", Write::File(link.clone()), true),
            (
                "대소문자",
                Write::File(volatile.join("SKILLS").join("s").join("x.md")),
                folds,
            ),
            (
                "끝의 점",
                Write::File(volatile.join("skills.").join("s").join("x.md")),
                true,
            ),
            (
                "끝의 공백",
                Write::File(volatile.join("skills ").join("s").join("x.md")),
                true,
            ),
            (
                "스트림",
                Write::File(link.join("s").join("SKILL.md:x")),
                true,
            ),
            (
                "링크 안 링크",
                Write::Link(volatile.join("SKILLS").join("t")),
                folds,
            ),
        ];
        for (what, write, refused) in attempts {
            let placed = place_in_volatile(&kit_in(Some(volatile.clone())), &StdLaunchFs, |fs| {
                match &write {
                    Write::File(path) => fs.write_file(path, b"x"),
                    Write::Link(path) => fs.link_dir(path, &elsewhere),
                }
                .map_err(|e| e.to_string())
            });
            assert_eq!(placed.is_err(), refused, "{what}: {placed:?}");
            library_untouched(what);
        }

        let settings = volatile.join("engram").join("settings.json");
        assert_eq!(
            place_in_volatile(&kit_in(Some(volatile.clone())), &StdLaunchFs, |fs| {
                fs.write_file(&settings, b"{}").map_err(|e| e.to_string())
            }),
            Ok(()),
            "링크 밖 쓰기까지 막았다"
        );
        assert_eq!(std::fs::read(&settings).unwrap(), b"{}");
    }
}
