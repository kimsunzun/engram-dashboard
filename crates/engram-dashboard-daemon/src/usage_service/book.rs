//! 사용량 책 — 벤더 칸마다의 순수 상태기(TRD §1-4). 스레드·락·시계·I/O·발행이 없다: 부르는 쪽이 시각
//! ([`Now`])을 넘기고 무엇이 바뀌었는지를 돌려받는다. 잠그는 것·발행하는 것은 서비스 몫이다.
//!
//! ★revision 은 보이는 것이 바뀔 때만 오른다(§3 #49)★ — 스냅숏이 싣는 값(창 %·리셋·만료·출처 · 모델별 · plan ·
//!   상태 · detail) 또는 다음 자동 조회 기한. 같은 값을 다시 주워 나이만 새로워진 것은 바뀜이 아니다. 예외 = 조회
//!   시작·끝은 늘 오른다(`in_flight` 가 바뀐다).
//! ★시각만 흘러 생기는 바뀜(만료 래치 · 거절 끝 → 조회 실패)은 [`UsageBook::eval_time`] 이 칸에 새기고 그때
//!   revision 이 오른다★ — ★스냅숏을 뜨기 전에 이것을 부르는 것은 부르는 쪽(서비스)의 의무다★. 줍기 적용·조회 끝은
//!   스스로 부른다.
//! ★무엇을 받아도 패닉하지 않는다★ — 릴리즈는 `panic = "abort"` 다. 시각 산술은 전부 포화·checked 다.

use std::time::Duration;

use engram_dashboard_agent::usage::{
    ProbeError, ProbeFailure, ScopedWindowObs, UsageAccountKey, UsageDetail, UsageKey,
    UsageObservation, UsagePolicy, UsageSource, UsageVendorKey, WindowObs,
};
use engram_dashboard_protocol::{
    AgentBackendKind, UsageLimitSnapshot, UsageScopedWindow, UsageSourceKind, UsageStateDetail,
    UsageVendorState, UsageWindow,
};
use serde::de::value::{Error as ValueError, StrDeserializer};
use serde::de::IntoDeserializer;
use serde::Deserialize;

/// 거절 대기 — 상류가 쓸 만한 대기를 안 줬을 때(없음·0)(D4).
pub const REJECT_FALLBACK: Duration = Duration::from_secs(5 * 60);
/// 거절 대기 상한 — 상류가 준 대기를 여기서 자른다(§3 #37).
pub const REJECT_MAX: Duration = Duration::from_secs(24 * 60 * 60);
/// 두 `resets_at` 이 이 안이면 같은 리셋이다 — 경계 포함(60초 차 = 같음 · 61초 차 = 다름, §3 #46).
pub const RESET_SAME_TOLERANCE: Duration = Duration::from_secs(60);
/// 모델별 창을 한 관측에서 받는 상한 — 줍기는 pump 스레드에서 곧장 돈다. 넘는 것은 앞에서부터 이만큼만 받는다.
pub const MODEL_SCOPED_MAX: usize = 16;

/// [`super::clock::UsageClock`] 을 한 번 읽은 값 — `mono` = 그 시계 기점부터의 단조 경과, `wall` = epoch 초.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Now {
    pub mono: Duration,
    pub wall: i64,
}

/// [`UsageBook::apply_passive`] 의 결과. `changed` 면 revision 이 올랐다 · `next_auto_changed` 면 스케줄러를
/// 깨울 일이다(D12 — `changed` 도 함께 선다).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PassiveApplied {
    pub changed: bool,
    pub next_auto_changed: bool,
}

/// [`UsageBook::finish_probe`] 의 결과. `reject_changed` = 거절 기한이 새로 섰거나 바뀌었거나 지워졌다(조회
/// 성공) — 부르는 쪽이 거절 기한을 저장한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FinishApplied {
    pub reject_changed: bool,
}

/// 칸마다 하나. 키 목록은 만들 때 고정된다.
pub struct UsageBook {
    cells: Vec<Cell>,
}

impl UsageBook {
    /// 받은 벤더마다 기본 계정 칸 하나 — 같은 벤더가 두 번 오면 앞의 것만 둔다.
    pub fn new(vendors: &[(UsageVendorKey, UsagePolicy)]) -> Self {
        let mut cells: Vec<Cell> = Vec::with_capacity(vendors.len());
        for &(vendor, policy) in vendors {
            if cells.iter().all(|cell| cell.key.vendor != vendor) {
                cells.push(Cell::new(vendor, policy));
            }
        }
        Self { cells }
    }

    /// 만든 순서.
    pub fn keys(&self) -> Vec<UsageKey> {
        self.cells.iter().map(|cell| cell.key.clone()).collect()
    }

    pub fn revision(&self, key: &UsageKey) -> Option<u64> {
        self.cell(key).map(|cell| cell.revision)
    }

    pub fn in_flight(&self, key: &UsageKey) -> Option<bool> {
        self.cell(key).map(|cell| cell.in_flight)
    }

    /// 다음 자동 조회의 `mono` 기한(R29) = `max(max(last_query, passive_reset) + cooldown, reject_until)` —
    /// 기준점이 없으면 `reject_until`, 그것도 없으면 `now.mono`(= 지금 기한). 진행 중인지는 보지 않는다.
    /// `None` = 모르는 키.
    pub fn next_auto(&self, key: &UsageKey, now: Now) -> Option<Duration> {
        self.cell(key).map(|cell| cell.next_auto(now))
    }

    /// 줍기 관측 한 건을 창 단위로 합친다(R16·D18·D12) — 칸 = 관측의 벤더 + 기본 계정. 모르는 벤더이거나
    /// `source` 가 `Passive` 가 아니면 아무것도 안 한다(후자는 warn).
    pub fn apply_passive(&mut self, obs: &UsageObservation, now: Now) -> PassiveApplied {
        if obs.source != UsageSource::Passive {
            // 빠진 칸의 뜻이 반대라(조회 = 없음) 부분 관측으로 합치면 틀린다. 값은 로그에 싣지 않는다.
            tracing::warn!(
                vendor = obs.vendor.as_str(),
                "사용량 줍기 입구에 조회 관측이 왔다(source mismatch) — 버린다"
            );
            return PassiveApplied::default();
        }
        let account = UsageAccountKey::default();
        let Some(cell) = self
            .cells
            .iter_mut()
            .find(|cell| cell.key.vendor == obs.vendor && cell.key.account == account)
        else {
            return PassiveApplied::default();
        };
        // wire 규칙 「`Unavailable` 은 값을 싣지 않는다」 — % 가 없는 줍기는 그 칸을 아예 건드리지 않는다.
        if matches!(cell.state, CellState::Unavailable(_)) && !carries_pct(obs) {
            return PassiveApplied::default();
        }
        let before = cell.visible(now);
        cell.merge_passive(obs, now);
        cell.eval_time(now);
        let after = cell.visible(now);
        let changed = !before.same(&after);
        if changed {
            cell.bump();
        }
        PassiveApplied {
            changed,
            next_auto_changed: before.next_auto != after.next_auto,
        }
    }

    /// 조회 시작 — `in_flight` 를 세우고 revision +1. 모르는 키이거나 이미 진행 중이면 `false`(아무것도 안 바뀜).
    pub fn begin_probe(&mut self, key: &UsageKey) -> bool {
        match self.cell_mut(key) {
            Some(cell) if !cell.in_flight => {
                cell.in_flight = true;
                cell.bump();
                true
            }
            _ => false,
        }
    }

    /// 조회 끝(성공·한도 정보 없음·실패 — 시한 초과 포함). 결과를 적용하고 `last_query = now.mono` 가 새 기준점이
    /// 된다(D12 의 새로 들어옴도 여기서 지운다). 성공(한도 정보 없음 포함)은 거절 기한을 지운다. 진행 중이
    /// 아니었어도 적용하고, revision 은 늘 +1 이다. 결과의 `source` 가 `Active` 가 아니어도 warn 만 하고 조회
    /// 결과로 적용한다 — 끝이 안 서면 `in_flight` 가 남는다. 모르는 키면 아무것도 안 한다.
    pub fn finish_probe(
        &mut self,
        key: &UsageKey,
        result: Result<UsageObservation, ProbeFailure>,
        now: Now,
    ) -> FinishApplied {
        let Some(cell) = self.cell_mut(key) else {
            return FinishApplied::default();
        };
        let reject_before = cell.reject_until;
        match result {
            Ok(obs) => {
                if obs.source != UsageSource::Active {
                    tracing::warn!(
                        vendor = obs.vendor.as_str(),
                        "사용량 조회 결과가 조회 관측이 아니다(source mismatch) — 조회 결과로 적용한다"
                    );
                }
                cell.apply_active(obs, now.mono);
            }
            Err(failure) => cell.apply_failure(failure, now.mono),
        }
        cell.last_query = Some(now.mono);
        cell.fresh_five_hour = false;
        cell.fresh_weekly = false;
        cell.in_flight = false;
        // 상류가 이미 지난 리셋을 줬으면 받은 자리에서 만료로 보인다 — 어차피 오르는 revision 에 싣는다.
        cell.eval_time(now);
        cell.bump();
        FinishApplied {
            reject_changed: cell.reject_until != reject_before,
        }
    }

    /// 시각만 흘러 생기는 바뀜을 칸에 새긴다 — 만료 래치(R32: `resets_at <= now.wall` 인 창마다 한 번) · 거절 끝
    /// (`reject_until <= now.mono` 인 `Rejected` → `Failed`, detail 유지 — R31). 새긴 것이 있으면 revision +1 과 `true`.
    pub fn eval_time(&mut self, key: &UsageKey, now: Now) -> bool {
        let Some(cell) = self.cell_mut(key) else {
            return false;
        };
        let changed = cell.eval_time(now);
        if changed {
            cell.bump();
        }
        changed
    }

    /// `now` 기준 wire 한 장. `None` = 모르는 키, 또는 그 벤더 낱말이 wire 벤더로 안 바뀐다(만들 때 warn 으로 남겼다).
    /// ★먼저 [`UsageBook::eval_time`] 을 부른다★ — 안 불러도 끝난 거절은 조회 실패로 보이지만, 그 바뀜은 revision
    /// 에 없다.
    pub fn snapshot(&self, key: &UsageKey, now: Now) -> Option<UsageLimitSnapshot> {
        let cell = self.cell(key)?;
        Some(UsageLimitSnapshot {
            vendor: cell.wire_vendor?,
            account_key: cell.key.account.as_str().to_owned(),
            five_hour: cell.five_hour.as_ref().and_then(|w| w.wire(now.mono)),
            weekly: cell.weekly.as_ref().and_then(|w| w.wire(now.mono)),
            model_scoped: cell
                .model_scoped
                .iter()
                .filter_map(|(label, w)| {
                    w.wire(now.mono).map(|window| UsageScopedWindow {
                        label: label.clone(),
                        window,
                    })
                })
                .collect(),
            plan: cell.plan.clone(),
            in_flight: cell.in_flight,
            state: cell.wire_state(now),
            revision: cell.revision,
        })
    }

    fn cell(&self, key: &UsageKey) -> Option<&Cell> {
        self.cells.iter().find(|cell| cell.key == *key)
    }

    fn cell_mut(&mut self, key: &UsageKey) -> Option<&mut Cell> {
        self.cells.iter_mut().find(|cell| cell.key == *key)
    }
}

/// 칸이 쥔 상태. 비정상 다섯은 모두 detail 을 든다(§1-4 「실패 추적」) — `Rejected` 가 끝나 `Failed` 로 보여도
/// 그대로다. 거절 기한은 칸의 `reject_until` 이 쥔다.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CellState {
    Ready,
    NotInstalled(UsageDetail),
    NeedsLogin(UsageDetail),
    Unavailable(UsageDetail),
    Failed(UsageDetail),
    Rejected(UsageDetail),
}

struct Cell {
    key: UsageKey,
    policy: UsagePolicy,
    /// `None` = 이 벤더 낱말이 wire 벤더로 안 바뀐다 — 스냅숏을 못 뜬다.
    wire_vendor: Option<AgentBackendKind>,
    five_hour: Option<Window>,
    weekly: Option<Window>,
    /// 라벨 순(같은 라벨끼리는 받은 순) — 받을 때 정렬해 두어 상류의 나열 순서가 바뀜으로 읽히지 않게.
    model_scoped: Vec<(String, Window)>,
    plan: Option<String>,
    state: CellState,
    last_query: Option<Duration>,
    passive_reset: Option<Duration>,
    reject_until: Option<Duration>,
    /// D12 — 지금 기준점 뒤로 줍기 `used_pct` 가 실려 온 창(§3 #4 — 리셋만 온 창은 안 센다).
    fresh_five_hour: bool,
    fresh_weekly: bool,
    in_flight: bool,
    revision: u64,
}

impl Cell {
    fn new(vendor: UsageVendorKey, policy: UsagePolicy) -> Self {
        Self {
            key: UsageKey {
                vendor,
                account: UsageAccountKey::default(),
            },
            policy,
            wire_vendor: wire_vendor(vendor),
            five_hour: None,
            weekly: None,
            model_scoped: Vec::new(),
            plan: None,
            state: CellState::Ready,
            last_query: None,
            passive_reset: None,
            reject_until: None,
            fresh_five_hour: false,
            fresh_weekly: false,
            in_flight: false,
            revision: 0,
        }
    }

    fn bump(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }

    fn next_auto(&self, now: Now) -> Duration {
        match self.last_query.max(self.passive_reset) {
            Some(reference) => {
                let due = reference.saturating_add(self.policy.cooldown);
                self.reject_until.map_or(due, |until| due.max(until))
            }
            None => self.reject_until.unwrap_or(now.mono),
        }
    }

    fn visible(&self, now: Now) -> Visible {
        Visible {
            five_hour: self.five_hour.clone(),
            weekly: self.weekly.clone(),
            model_scoped: self.model_scoped.clone(),
            plan: self.plan.clone(),
            state: self.state.clone(),
            in_flight: self.in_flight,
            reject_until: self.reject_until,
            next_auto: self.next_auto(now),
        }
    }

    fn merge_passive(&mut self, obs: &UsageObservation, now: Now) {
        if let Some(window) = obs.five_hour {
            merge_window(&mut self.five_hour, window, now).mark(&mut self.fresh_five_hour);
        }
        if let Some(window) = obs.weekly {
            merge_window(&mut self.weekly, window, now).mark(&mut self.fresh_weekly);
        }
        if let Some(list) = &obs.model_scoped {
            self.model_scoped = scoped_windows(list, UsageSource::Passive, now.mono);
        }
        if let Some(plan) = &obs.plan {
            self.plan = Some(plan.clone());
        }
        if self.fresh_five_hour && self.fresh_weekly {
            self.passive_reset = Some(now.mono);
            self.fresh_five_hour = false;
            self.fresh_weekly = false;
        }
        // 이 계정엔 한도가 있다는 증거다(§3 #52). 다른 비정상 상태는 줍기가 안 바꾼다 — 값만 합친다(R22).
        if matches!(self.state, CellState::Unavailable(_)) && carries_pct(obs) {
            self.state = CellState::Ready;
        }
    }

    fn apply_active(&mut self, obs: UsageObservation, mono: Duration) {
        // 상류가 답을 줬으니 거절은 끝났다 — 남겨 두면 지난 기한이 다음 자동 조회를 붙든다.
        self.reject_until = None;
        self.plan = obs.plan;
        if let Some(detail) = obs.limits_unavailable {
            self.five_hour = None;
            self.weekly = None;
            self.model_scoped.clear();
            self.state = CellState::Unavailable(detail);
            return;
        }
        self.five_hour = obs
            .five_hour
            .and_then(|w| Window::observed(w, UsageSource::Active, mono));
        self.weekly = obs
            .weekly
            .and_then(|w| Window::observed(w, UsageSource::Active, mono));
        self.model_scoped = obs
            .model_scoped
            .map(|list| scoped_windows(&list, UsageSource::Active, mono))
            .unwrap_or_default();
        self.state = CellState::Ready;
    }

    fn apply_failure(&mut self, failure: ProbeFailure, mono: Duration) {
        let ProbeFailure { error, detail } = failure;
        let detail = detail.unwrap_or(UsageDetail {
            kind: error.kind_word(),
            code: None,
            upstream: None,
        });
        // 와일드카드를 두지 않는다 — 분류가 늘면 여기서 컴파일이 멈춰 보이는 상태를 정하게 한다.
        self.state = match error {
            ProbeError::NotInstalled => CellState::NotInstalled(detail),
            ProbeError::Unauthenticated => CellState::NeedsLogin(detail),
            ProbeError::RateLimited { retry_after } => {
                self.reject_until = Some(reject_deadline(mono, retry_after));
                CellState::Rejected(detail)
            }
            ProbeError::Timeout
            | ProbeError::Unsupported
            | ProbeError::Spawn(_)
            | ProbeError::Io(_)
            | ProbeError::Parse(_)
            | ProbeError::Upstream(_) => CellState::Failed(detail),
        };
    }

    /// [`UsageBook::eval_time`] 의 칸 몫 — revision 은 부르는 쪽이 올린다. 한 창에서 멈추지 않고 전부 본다.
    fn eval_time(&mut self, now: Now) -> bool {
        let mut changed = false;
        for window in self
            .five_hour
            .iter_mut()
            .chain(self.weekly.iter_mut())
            .chain(self.model_scoped.iter_mut().map(|(_, w)| w))
        {
            changed |= window.latch(now.wall);
        }
        if let CellState::Rejected(detail) = &self.state {
            if self.reject_until.is_none_or(|until| until <= now.mono) {
                self.state = CellState::Failed(detail.clone());
                changed = true;
            }
        }
        changed
    }

    fn wire_state(&self, now: Now) -> UsageVendorState {
        let failed = |detail: &UsageDetail| UsageVendorState::Failed {
            next_attempt_in_secs: self.next_auto(now).saturating_sub(now.mono).as_secs(),
            detail: Some(wire_detail(detail)),
        };
        match &self.state {
            CellState::Ready => UsageVendorState::Ready,
            CellState::NotInstalled(detail) => UsageVendorState::NotInstalled {
                detail: Some(wire_detail(detail)),
            },
            CellState::NeedsLogin(detail) => UsageVendorState::NeedsLogin {
                detail: Some(wire_detail(detail)),
            },
            CellState::Unavailable(detail) => UsageVendorState::Unavailable {
                detail: Some(wire_detail(detail)),
            },
            CellState::Failed(detail) => failed(detail),
            // 끝난 거절 → 조회 실패는 `eval_time` 이 새긴다. 이 갈래는 그것을 거르고 뜬 스냅숏의 폴백이다.
            CellState::Rejected(detail) => match self.reject_until {
                Some(until) if until > now.mono => UsageVendorState::Rejected {
                    retry_in_secs: ceil_secs(until - now.mono),
                    detail: Some(wire_detail(detail)),
                },
                _ => failed(detail),
            },
        }
    }
}

/// 바뀜 판정용 사본 — 스냅숏이 보이는 것 전부에서 나이(`Window::observed`)만 뺀다.
struct Visible {
    five_hour: Option<Window>,
    weekly: Option<Window>,
    model_scoped: Vec<(String, Window)>,
    plan: Option<String>,
    state: CellState,
    in_flight: bool,
    reject_until: Option<Duration>,
    next_auto: Duration,
}

impl Visible {
    fn same(&self, other: &Self) -> bool {
        let same_slot = |a: &Option<Window>, b: &Option<Window>| match (a, b) {
            (Some(a), Some(b)) => a.looks_same(b),
            (None, None) => true,
            _ => false,
        };
        same_slot(&self.five_hour, &other.five_hour)
            && same_slot(&self.weekly, &other.weekly)
            && self.model_scoped.len() == other.model_scoped.len()
            && self
                .model_scoped
                .iter()
                .zip(&other.model_scoped)
                .all(|((la, a), (lb, b))| la == lb && a.looks_same(b))
            && self.plan == other.plan
            && self.state == other.state
            && self.in_flight == other.in_flight
            && self.reject_until == other.reject_until
            && self.next_auto == other.next_auto
    }
}

#[derive(Debug, Clone)]
struct Window {
    used_pct: Option<f64>,
    resets_at: Option<i64>,
    source: UsageSource,
    /// 만료 래치(R32) — `wall >= resets_at` 을 한 번 보면 서고, 이 창의 새 값(`used_pct`)만 내린다. 벽시계가
    /// 되감겨도 안 내린다. % 는 지우지 않는다 — 가리는 것은 받는 쪽이다.
    expired: bool,
    /// 이 값을 관측한 `mono` — 나이의 기점.
    observed: Duration,
}

impl Window {
    /// 두 칸 다 없으면 `None` — 값이 없는 창을 두지 않는다.
    fn observed(obs: WindowObs, source: UsageSource, mono: Duration) -> Option<Self> {
        let obs = ingest(obs);
        (obs.used_pct.is_some() || obs.resets_at.is_some()).then_some(Self {
            used_pct: obs.used_pct,
            resets_at: obs.resets_at,
            source,
            expired: false,
            observed: mono,
        })
    }

    fn looks_same(&self, other: &Self) -> bool {
        same_pct(self.used_pct, other.used_pct)
            && self.resets_at == other.resets_at
            && self.source == other.source
            && self.expired == other.expired
    }

    fn latch(&mut self, wall: i64) -> bool {
        match self.resets_at {
            Some(at) if !self.expired && at <= wall => {
                self.expired = true;
                true
            }
            _ => false,
        }
    }

    fn wire(&self, mono: Duration) -> Option<UsageWindow> {
        let resets_at = self.resets_at.and_then(|at| u64::try_from(at).ok());
        (self.used_pct.is_some() || resets_at.is_some()).then(|| UsageWindow {
            used_pct: self.used_pct,
            resets_at,
            age_secs: mono.saturating_sub(self.observed).as_secs(),
            expired: self.expired,
            source: match self.source {
                UsageSource::Passive => UsageSourceKind::Passive,
                UsageSource::Active => UsageSourceKind::Active,
            },
        })
    }
}

/// 받는 자리의 정규화 — 0 이하 리셋은 없는 것으로 읽는다(wire 가 못 싣는 값이 칸에 들면 보이지 않는 바뀜이 된다).
fn ingest(obs: WindowObs) -> WindowObs {
    WindowObs {
        resets_at: obs.resets_at.filter(|&at| at > 0),
        ..obs
    }
}

/// [`merge_window`] 가 그 창의 D12 「새로 들어옴」에 한 일.
enum Fresh {
    /// `used_pct` 가 실렸다 — 선다.
    Carried,
    /// 바뀐 리셋이 % 를 지웠다 — 앞서 선 것도 내린다(지운 % 로 쿨타임을 초기화하지 않게).
    Cleared,
    Untouched,
}

impl Fresh {
    fn mark(self, flag: &mut bool) {
        match self {
            Self::Carried => *flag = true,
            Self::Cleared => *flag = false,
            Self::Untouched => {}
        }
    }
}

/// 줍기 창 하나를 들고 있던 창에 합친다(§1-4 「줍기 관측」 · §3 #3·#46).
fn merge_window(slot: &mut Option<Window>, obs: WindowObs, now: Now) -> Fresh {
    let obs = ingest(obs);
    let carried = if obs.used_pct.is_some() {
        Fresh::Carried
    } else {
        Fresh::Untouched
    };
    let Some(window) = slot.as_mut() else {
        *slot = Window::observed(obs, UsageSource::Passive, now.mono);
        return carried;
    };
    let mut fresh = carried;
    match (obs.used_pct, window.resets_at, obs.resets_at) {
        (Some(pct), kept, new) => {
            window.used_pct = Some(pct);
            window.expired = false;
            window.observed = now.mono;
            window.source = UsageSource::Passive;
            // 리셋이 지난 뒤 온 % 는 새 창의 값이고 그 창의 리셋은 모른다(R32 「새 값이 올 때까지」) — 지난 리셋을
            // 남기면 받은 자리에서 다시 만료로 보인다.
            if new.is_none() && kept.is_some_and(|at| at <= now.wall) {
                window.resets_at = None;
            }
        }
        (None, Some(old), Some(new)) if old.abs_diff(new) > RESET_SAME_TOLERANCE.as_secs() => {
            // 새 창의 옛 % 는 틀린 값이다. 남는 값은 이 관측의 리셋뿐이라 나이·출처도 이 관측 것이다.
            window.used_pct = None;
            window.observed = now.mono;
            window.source = UsageSource::Passive;
            fresh = Fresh::Cleared;
        }
        _ => {}
    }
    if obs.resets_at.is_some() {
        window.resets_at = obs.resets_at;
    }
    fresh
}

/// 목록 전량 교체용 — 줍기든 조회든 `Some` 은 합치지 않고 통째로 바꾼다(`UsageObservation` 계약).
fn scoped_windows(
    list: &[ScopedWindowObs],
    source: UsageSource,
    mono: Duration,
) -> Vec<(String, Window)> {
    if list.len() > MODEL_SCOPED_MAX {
        // 지금 생산자 둘은 상한만큼만 싣는다 — 여기 걸리면 생산자 쪽이 어긋난 것이다(타입이 상한을 선언하지는
        // 않는다).
        tracing::warn!(
            received = list.len(),
            kept = MODEL_SCOPED_MAX,
            "모델별 창이 상한을 넘었다 — 앞에서부터 상한만큼만 받는다"
        );
    }
    let mut windows: Vec<(String, Window)> = list
        .iter()
        .take(MODEL_SCOPED_MAX)
        .filter_map(|scoped| {
            Window::observed(scoped.window, source, mono).map(|w| (scoped.label.clone(), w))
        })
        .collect();
    windows.sort_by(|(a, _), (b, _)| a.cmp(b));
    windows
}

fn carries_pct(obs: &UsageObservation) -> bool {
    let has = |w: &Option<WindowObs>| w.is_some_and(|w| w.used_pct.is_some());
    has(&obs.five_hour)
        || has(&obs.weekly)
        || obs
            .model_scoped
            .iter()
            .flat_map(|list| list.iter().take(MODEL_SCOPED_MAX))
            .any(|scoped| scoped.window.used_pct.is_some())
}

/// NaN 끼리도 같다고 본다 — 생산자는 유한수를 약속하지만, 어기면 같은 값이 올 때마다 바뀜으로 읽힌다.
fn same_pct(a: Option<f64>, b: Option<f64>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a == b || a.to_bits() == b.to_bits(),
        (None, None) => true,
        _ => false,
    }
}

fn reject_deadline(mono: Duration, retry_after: Option<Duration>) -> Duration {
    let wait = match retry_after {
        Some(wait) if !wait.is_zero() => wait.min(REJECT_MAX),
        _ => REJECT_FALLBACK,
    };
    // 넘치면 끝까지 거절로 둔다 — 넘침을 거절 없음으로 읽으면 상류를 곧바로 다시 두드린다.
    mono.checked_add(wait).unwrap_or(Duration::MAX)
}

/// 올림 — 거절 중에는 0 초로 안 보인다.
fn ceil_secs(d: Duration) -> u64 {
    d.as_secs().saturating_add(u64::from(d.subsec_nanos() > 0))
}

fn wire_detail(detail: &UsageDetail) -> UsageStateDetail {
    UsageStateDetail {
        kind: detail.kind.to_owned(),
        code: detail.code,
        upstream: detail.upstream.as_ref().map(|u| u.as_str().to_owned()),
    }
}

/// 조회기 낱말 → wire 벤더(§1-6 키 변환) — 벤더 match 없이 wire 역직렬화에 맡긴다(대소문자는 그쪽이 접는다).
// ADR-0004
fn wire_vendor(vendor: UsageVendorKey) -> Option<AgentBackendKind> {
    let word: StrDeserializer<'_, ValueError> = vendor.as_str().into_deserializer();
    match AgentBackendKind::deserialize(word) {
        Ok(kind) => Some(kind),
        Err(e) => {
            tracing::warn!(
                vendor = vendor.as_str(),
                error = %e,
                "조회기 낱말이 wire 벤더가 아니다 — 이 칸은 스냅숏을 못 뜬다"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log_capture::capture_loud;
    use engram_dashboard_agent::backend::usage_probes;
    use engram_dashboard_agent::usage::UpstreamText;

    const H: i64 = 3_600;
    const T0: i64 = 1_900_000_000;

    // 조회기 키로만 칸을 만든다 — 시험에도 벤더 리터럴을 두지 않는다.
    fn key(i: usize) -> UsageKey {
        UsageKey {
            vendor: usage_probes()[i].key(),
            account: UsageAccountKey::default(),
        }
    }

    fn cooldown(i: usize) -> Duration {
        usage_probes()[i].policy().cooldown
    }

    fn book() -> UsageBook {
        UsageBook::new(&usage_probes().map(|p| (p.key(), p.policy())))
    }

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    fn at(mono_s: u64, wall: i64) -> Now {
        Now {
            mono: secs(mono_s),
            wall,
        }
    }

    fn w(used_pct: Option<f64>, resets_at: Option<i64>) -> Option<WindowObs> {
        Some(WindowObs {
            used_pct,
            resets_at,
        })
    }

    fn obs(
        i: usize,
        source: UsageSource,
        five_hour: Option<WindowObs>,
        weekly: Option<WindowObs>,
    ) -> UsageObservation {
        UsageObservation {
            vendor: usage_probes()[i].key(),
            five_hour,
            weekly,
            model_scoped: None,
            plan: None,
            source,
            limits_unavailable: None,
        }
    }

    fn passive(i: usize, five: Option<WindowObs>, weekly: Option<WindowObs>) -> UsageObservation {
        obs(i, UsageSource::Passive, five, weekly)
    }

    fn active(i: usize, five: Option<WindowObs>, weekly: Option<WindowObs>) -> UsageObservation {
        obs(i, UsageSource::Active, five, weekly)
    }

    fn scoped(label: &str, window: Option<WindowObs>) -> ScopedWindowObs {
        ScopedWindowObs {
            label: label.to_owned(),
            window: window.expect("창"),
        }
    }

    fn succeed(b: &mut UsageBook, i: usize, o: UsageObservation, now: Now) {
        assert!(b.begin_probe(&key(i)));
        b.finish_probe(&key(i), Ok(o), now);
    }

    fn fail(b: &mut UsageBook, i: usize, failure: ProbeFailure, now: Now) -> FinishApplied {
        assert!(b.begin_probe(&key(i)));
        b.finish_probe(&key(i), Err(failure), now)
    }

    fn snap(b: &UsageBook, i: usize, now: Now) -> UsageLimitSnapshot {
        b.snapshot(&key(i), now).expect("아는 키")
    }

    fn pct(window: &Option<UsageWindow>) -> Option<f64> {
        window.as_ref().and_then(|w| w.used_pct)
    }

    fn reset(window: &Option<UsageWindow>) -> Option<u64> {
        window.as_ref().and_then(|w| w.resets_at)
    }

    fn expired(window: &Option<UsageWindow>) -> bool {
        window.as_ref().expect("창").expired
    }

    /// 상태 태그 낱말(wire `kind`).
    fn tag(state: &UsageVendorState) -> String {
        serde_json::to_value(state).expect("직렬화")["kind"]
            .as_str()
            .expect("태그")
            .to_owned()
    }

    fn detail_of(state: &UsageVendorState) -> Option<&UsageStateDetail> {
        match state {
            UsageVendorState::Ready => None,
            UsageVendorState::NotInstalled { detail }
            | UsageVendorState::NeedsLogin { detail }
            | UsageVendorState::Unavailable { detail }
            | UsageVendorState::Failed { detail, .. }
            | UsageVendorState::Rejected { detail, .. } => detail.as_ref(),
        }
    }

    fn rich_detail() -> UsageDetail {
        UsageDetail {
            kind: "some_kind",
            code: Some(429),
            upstream: Some(UpstreamText::new("slow down")),
        }
    }

    /// 두 창 40%·10% 를 조회로 받아 둔 책(mono 100 · 리셋은 한참 뒤).
    fn seeded() -> UsageBook {
        let mut b = book();
        succeed(
            &mut b,
            0,
            active(
                0,
                w(Some(40.0), Some(T0 + 5 * H)),
                w(Some(10.0), Some(T0 + 100 * H)),
            ),
            at(100, T0),
        );
        b
    }

    // ── 병합(R16·D18·§3 #3·#46) ──

    #[test]
    fn passive_keeps_windows_it_does_not_carry() {
        let mut b = seeded();
        b.apply_passive(&passive(0, w(Some(55.0), None), None), at(200, T0));
        let s = snap(&b, 0, at(200, T0));
        assert_eq!(pct(&s.five_hour), Some(55.0));
        assert_eq!(
            (pct(&s.weekly), reset(&s.weekly)),
            (Some(10.0), Some((T0 + 100 * H) as u64))
        );

        let mut plan_only = passive(0, None, None);
        plan_only.plan = Some("plan-a".to_owned());
        b.apply_passive(&plan_only, at(300, T0));
        let s = snap(&b, 0, at(300, T0));
        assert_eq!(
            (pct(&s.five_hour), pct(&s.weekly)),
            (Some(55.0), Some(10.0))
        );
        assert_eq!(s.plan.as_deref(), Some("plan-a"));
    }

    #[test]
    fn active_null_or_missing_windows_mean_none() {
        let mut b = book();
        let mut full = active(
            0,
            w(Some(40.0), Some(T0 + H)),
            w(Some(10.0), Some(T0 + 9 * H)),
        );
        full.model_scoped = Some(vec![scoped("m", w(Some(3.0), None))]);
        full.plan = Some("plan-a".to_owned());
        succeed(&mut b, 0, full, at(100, T0));

        succeed(&mut b, 0, active(0, w(None, None), None), at(200, T0));
        let s = snap(&b, 0, at(200, T0));
        assert_eq!(s.five_hour, None);
        assert_eq!(s.weekly, None);
        assert!(s.model_scoped.is_empty());
        assert_eq!(s.plan, None);
        assert_eq!(s.state, UsageVendorState::Ready);
    }

    #[test]
    fn null_is_not_zero() {
        let mut b = book();
        b.apply_passive(&passive(0, w(None, Some(T0 + H)), None), at(10, T0));
        let s = snap(&b, 0, at(10, T0));
        let five = s.five_hour.expect("리셋만 아는 창");
        assert_eq!(five.used_pct, None);
        assert_eq!(s.weekly, None, "안 실린 창은 0% 창이 아니라 없음이다");

        b.apply_passive(&passive(0, w(Some(0.0), None), None), at(20, T0));
        assert_eq!(pct(&snap(&b, 0, at(20, T0)).five_hour), Some(0.0));
    }

    #[test]
    fn reset_only_for_the_same_reset_keeps_pct_and_takes_the_new_reset() {
        let mut b = seeded();
        b.apply_passive(
            &passive(0, w(None, Some(T0 + 5 * H + 30)), None),
            at(200, T0),
        );
        let s = snap(&b, 0, at(200, T0));
        assert_eq!(
            (pct(&s.five_hour), reset(&s.five_hour)),
            (Some(40.0), Some((T0 + 5 * H + 30) as u64))
        );
    }

    #[test]
    fn reset_only_for_a_changed_reset_clears_pct() {
        let mut b = seeded();
        b.apply_passive(&passive(0, w(None, Some(T0 + 10 * H)), None), at(200, T0));
        let s = snap(&b, 0, at(200, T0));
        assert_eq!(
            (pct(&s.five_hour), reset(&s.five_hour)),
            (None, Some((T0 + 10 * H) as u64))
        );
        let five = s.five_hour.expect("리셋은 남는다");
        assert_eq!((five.source, five.age_secs), (UsageSourceKind::Passive, 0));
    }

    #[test]
    fn reset_tolerance_is_sixty_seconds_inclusive() {
        for (delta, keeps) in [(60, true), (-60, true), (61, false), (-61, false)] {
            let mut b = seeded();
            let moved = T0 + 5 * H + delta;
            b.apply_passive(&passive(0, w(None, Some(moved)), None), at(200, T0));
            let s = snap(&b, 0, at(200, T0));
            assert_eq!(pct(&s.five_hour), keeps.then_some(40.0), "차 {delta}초");
            assert_eq!(reset(&s.five_hour), Some(moved as u64), "차 {delta}초");
        }
    }

    #[test]
    fn learning_a_reset_keeps_pct() {
        let mut b = book();
        succeed(&mut b, 0, active(0, w(Some(40.0), None), None), at(100, T0));
        b.apply_passive(&passive(0, w(None, Some(T0 + H)), None), at(200, T0));
        let s = snap(&b, 0, at(200, T0));
        assert_eq!(
            (pct(&s.five_hour), reset(&s.five_hour)),
            (Some(40.0), Some((T0 + H) as u64))
        );
    }

    #[test]
    fn passive_none_reset_keeps_and_pct_without_reset_replaces_only_pct() {
        let mut b = seeded();
        let before = b.revision(&key(0));
        let applied = b.apply_passive(&passive(0, w(None, None), None), at(200, T0));
        assert_eq!(applied, PassiveApplied::default());
        assert_eq!(b.revision(&key(0)), before);

        b.apply_passive(&passive(0, w(Some(55.0), None), None), at(300, T0));
        let s = snap(&b, 0, at(300, T0));
        assert_eq!(
            (pct(&s.five_hour), reset(&s.five_hour)),
            (Some(55.0), Some((T0 + 5 * H) as u64))
        );
        let five = s.five_hour.expect("창");
        assert_eq!((five.source, five.age_secs), (UsageSourceKind::Passive, 0));
    }

    #[test]
    fn model_scoped_list_is_replaced_whole_and_kept_when_absent() {
        let mut b = book();
        let mut full = active(0, None, None);
        full.model_scoped = Some(vec![
            scoped("b", w(Some(2.0), None)),
            scoped("a", w(Some(1.0), None)),
            scoped("empty", w(None, None)),
        ]);
        succeed(&mut b, 0, full, at(100, T0));
        let labels = |b: &UsageBook| -> Vec<String> {
            snap(b, 0, at(100, T0))
                .model_scoped
                .into_iter()
                .map(|s| s.label)
                .collect()
        };
        assert_eq!(labels(&b), ["a", "b"], "라벨 순 · 값 없는 창은 빠진다");

        b.apply_passive(&passive(0, w(Some(5.0), None), None), at(150, T0));
        assert_eq!(labels(&b), ["a", "b"], "안 실린 목록은 유지");

        let mut one = passive(0, None, None);
        one.model_scoped = Some(vec![scoped("c", w(Some(9.0), None))]);
        b.apply_passive(&one, at(200, T0));
        assert_eq!(labels(&b), ["c"], "실린 목록은 전량 교체");
    }

    // ── D12 ──

    #[test]
    fn two_fresh_windows_arriving_separately_reset_the_cooldown() {
        let mut b = seeded();
        let cd = cooldown(0);
        assert_eq!(b.next_auto(&key(0), at(100, T0)), Some(secs(100) + cd));

        let first = b.apply_passive(&passive(0, w(Some(41.0), None), None), at(200, T0));
        assert_eq!(
            first,
            PassiveApplied {
                changed: true,
                next_auto_changed: false
            }
        );
        assert_eq!(b.next_auto(&key(0), at(200, T0)), Some(secs(100) + cd));

        let second = b.apply_passive(&passive(0, None, w(Some(11.0), None)), at(300, T0));
        assert_eq!(
            second,
            PassiveApplied {
                changed: true,
                next_auto_changed: true
            }
        );
        assert_eq!(b.next_auto(&key(0), at(300, T0)), Some(secs(300) + cd));

        // 초기화했으니 다음 둘이 다시 모여야 한다.
        b.apply_passive(&passive(0, w(Some(42.0), None), None), at(400, T0));
        assert_eq!(b.next_auto(&key(0), at(400, T0)), Some(secs(300) + cd));
    }

    #[test]
    fn a_finished_probe_is_a_new_reference_point_for_freshness() {
        let mut b = book();
        let cd = cooldown(0);
        b.apply_passive(&passive(0, w(Some(1.0), None), None), at(50, T0));
        succeed(
            &mut b,
            0,
            active(0, w(Some(1.0), None), w(Some(2.0), None)),
            at(100, T0),
        );
        b.apply_passive(&passive(0, None, w(Some(3.0), None)), at(150, T0));
        assert_eq!(
            b.next_auto(&key(0), at(150, T0)),
            Some(secs(100) + cd),
            "조회 전의 새로 들어옴은 안 센다"
        );
    }

    #[test]
    fn only_windows_carrying_pct_count_as_fresh() {
        let mut b = seeded();
        b.apply_passive(&passive(0, w(Some(41.0), None), None), at(200, T0));
        let applied = b.apply_passive(
            &passive(0, None, w(None, Some(T0 + 100 * H + 10))),
            at(300, T0),
        );
        assert!(!applied.next_auto_changed);
        assert_eq!(
            b.next_auto(&key(0), at(300, T0)),
            Some(secs(100) + cooldown(0))
        );
    }

    #[test]
    fn fresh_windows_without_a_reference_point_start_the_cooldown() {
        let mut b = book();
        assert_eq!(
            b.next_auto(&key(0), at(70, T0)),
            Some(secs(70)),
            "기준점 없음 = 지금"
        );
        let applied = b.apply_passive(
            &passive(0, w(Some(1.0), None), w(Some(2.0), None)),
            at(70, T0),
        );
        assert!(applied.next_auto_changed);
        assert_eq!(
            b.next_auto(&key(0), at(80, T0)),
            Some(secs(70) + cooldown(0))
        );
    }

    #[test]
    fn a_passive_cooldown_reset_does_not_shorten_a_rejection() {
        let mut b = seeded();
        fail(
            &mut b,
            0,
            ProbeError::RateLimited {
                retry_after: Some(REJECT_MAX),
            }
            .into(),
            at(100, T0),
        );
        let until = secs(100) + REJECT_MAX;
        b.apply_passive(
            &passive(0, w(Some(41.0), None), w(Some(11.0), None)),
            at(200, T0),
        );
        assert_eq!(
            b.next_auto(&key(0), at(200, T0)),
            Some((secs(200) + cooldown(0)).max(until))
        );
        let s = snap(&b, 0, at(200, T0));
        assert_eq!(tag(&s.state), "Rejected");
        assert!(matches!(
            s.state,
            UsageVendorState::Rejected { retry_in_secs, .. } if retry_in_secs == (until - secs(200)).as_secs()
        ));
    }

    // ── 상태(R22·R31·§3 #52) ──

    #[test]
    fn failure_keeps_values_and_folds_by_error() {
        let cases = [
            (ProbeError::NotInstalled, "NotInstalled"),
            (ProbeError::Unauthenticated, "NeedsLogin"),
            (ProbeError::RateLimited { retry_after: None }, "Rejected"),
            (ProbeError::Timeout, "Failed"),
            (ProbeError::Unsupported, "Failed"),
            (ProbeError::Spawn("x".to_owned()), "Failed"),
            (ProbeError::Io("x".to_owned()), "Failed"),
            (ProbeError::Parse("x".to_owned()), "Failed"),
            (ProbeError::Upstream("x".to_owned()), "Failed"),
        ];
        for (error, expected) in cases {
            let mut b = seeded();
            let kind = error.kind_word();
            fail(&mut b, 0, error.into(), at(200, T0));
            let s = snap(&b, 0, at(200, T0));
            assert_eq!(tag(&s.state), expected, "{kind}");
            assert_eq!(
                (pct(&s.five_hour), pct(&s.weekly)),
                (Some(40.0), Some(10.0)),
                "{kind}"
            );
            let detail = detail_of(&s.state).expect("비정상 상태는 detail 을 든다");
            assert_eq!(
                (
                    detail.kind.as_str(),
                    detail.code,
                    detail.upstream.as_deref()
                ),
                (kind, None, None),
                "근거가 없으면 분류 낱말만"
            );
        }
    }

    #[test]
    fn failure_detail_is_carried_as_given() {
        let mut b = seeded();
        fail(
            &mut b,
            0,
            ProbeFailure::with_detail(ProbeError::Timeout, rich_detail()),
            at(200, T0),
        );
        let s = snap(&b, 0, at(200, T0));
        let detail = detail_of(&s.state).expect("detail");
        assert_eq!(
            (
                detail.kind.as_str(),
                detail.code,
                detail.upstream.as_deref()
            ),
            ("some_kind", Some(429), Some("slow down"))
        );
    }

    #[test]
    fn limits_unavailable_clears_windows_and_waits_a_normal_cooldown() {
        let mut b = seeded();
        let mut unavailable = active(0, None, None);
        unavailable.plan = Some("plan-b".to_owned());
        unavailable.limits_unavailable = Some(UsageDetail {
            kind: "no_limits_kind",
            code: None,
            upstream: Some(UpstreamText::new("no limits here")),
        });
        succeed(&mut b, 0, unavailable, at(1_000, T0));

        let s = snap(&b, 0, at(1_000, T0));
        assert_eq!((s.five_hour, s.weekly), (None, None));
        assert!(s.model_scoped.is_empty());
        assert_eq!(s.plan.as_deref(), Some("plan-b"));
        assert_eq!(
            s.state,
            UsageVendorState::Unavailable {
                detail: Some(UsageStateDetail {
                    kind: "no_limits_kind".to_owned(),
                    code: None,
                    upstream: Some("no limits here".to_owned()),
                })
            }
        );
        assert_eq!(
            b.next_auto(&key(0), at(1_000, T0)),
            Some(secs(1_000) + cooldown(0))
        );
    }

    #[test]
    fn ready_clears_detail() {
        let mut b = seeded();
        fail(
            &mut b,
            0,
            ProbeFailure::with_detail(ProbeError::Timeout, rich_detail()),
            at(200, T0),
        );
        succeed(&mut b, 0, active(0, w(Some(50.0), None), None), at(300, T0));
        assert_eq!(snap(&b, 0, at(300, T0)).state, UsageVendorState::Ready);
    }

    #[test]
    fn unavailable_turns_ready_on_a_passive_pct_only() {
        let mut b = book();
        let mut unavailable = active(0, None, None);
        unavailable.limits_unavailable = Some(rich_detail());
        succeed(&mut b, 0, unavailable, at(100, T0));

        let rev = b.revision(&key(0));
        let before = snap(&b, 0, at(200, T0));

        // % 가 없는 줍기(리셋만 · plan 만 · % 없는 모델별 창)는 칸을 건드리지 않는다 — 값을 싣지 않는다.
        let mut no_pct = passive(0, w(None, Some(T0 + H)), w(None, Some(T0 + 9 * H)));
        no_pct.plan = Some("plan-a".to_owned());
        no_pct.model_scoped = Some(vec![scoped("m", w(None, Some(T0 + H)))]);
        assert_eq!(
            b.apply_passive(&no_pct, at(200, T0)),
            PassiveApplied::default()
        );
        assert_eq!(b.revision(&key(0)), rev);
        let s = snap(&b, 0, at(200, T0));
        assert_eq!(s, before);
        assert_eq!((s.five_hour, s.weekly), (None, None));
        assert!(s.model_scoped.is_empty());
        assert_eq!(tag(&s.state), "Unavailable");

        // % 가 실리면 정상으로 돌아가고 평소대로 합친다.
        let applied = b.apply_passive(
            &passive(0, w(None, Some(T0 + H)), w(Some(7.0), None)),
            at(300, T0),
        );
        assert!(applied.changed);
        let s = snap(&b, 0, at(300, T0));
        assert_eq!(s.state, UsageVendorState::Ready);
        assert_eq!(pct(&s.weekly), Some(7.0));
        assert_eq!(reset(&s.five_hour), Some((T0 + H) as u64));

        // 모델별 창의 % 도 증거다.
        let mut b = book();
        let mut unavailable = active(0, None, None);
        unavailable.limits_unavailable = Some(rich_detail());
        succeed(&mut b, 0, unavailable, at(100, T0));
        let mut scoped_pct = passive(0, None, None);
        scoped_pct.model_scoped = Some(vec![scoped("m", w(Some(3.0), None))]);
        assert!(b.apply_passive(&scoped_pct, at(200, T0)).changed);
        let s = snap(&b, 0, at(200, T0));
        assert_eq!(s.state, UsageVendorState::Ready);
        assert_eq!(s.model_scoped.len(), 1);
    }

    #[test]
    fn passive_does_not_change_other_abnormal_states() {
        let errors = [
            ProbeError::NotInstalled,
            ProbeError::Unauthenticated,
            ProbeError::RateLimited { retry_after: None },
            ProbeError::Timeout,
        ];
        for error in errors {
            let mut b = seeded();
            let kind = error.kind_word();
            fail(
                &mut b,
                0,
                ProbeFailure::with_detail(error, rich_detail()),
                at(200, T0),
            );
            let state_before = snap(&b, 0, at(250, T0)).state;
            let applied = b.apply_passive(&passive(0, w(Some(70.0), None), None), at(250, T0));
            assert!(applied.changed, "{kind}");
            let s = snap(&b, 0, at(250, T0));
            assert_eq!(s.state, state_before, "{kind}");
            assert_eq!(pct(&s.five_hour), Some(70.0), "{kind}: 값은 반영된다");
        }
    }

    #[test]
    fn rejection_ending_folds_to_failed_and_keeps_detail() {
        let mut b = seeded();
        let wait = secs(90);
        fail(
            &mut b,
            0,
            ProbeFailure::with_detail(
                ProbeError::RateLimited {
                    retry_after: Some(wait),
                },
                rich_detail(),
            ),
            at(100, T0),
        );
        let expected_detail = Some(wire_detail(&rich_detail()));

        let s = snap(&b, 0, at(100, T0));
        assert_eq!(
            s.state,
            UsageVendorState::Rejected {
                retry_in_secs: 90,
                detail: expected_detail.clone()
            }
        );

        let almost = Now {
            mono: secs(189) + Duration::from_millis(500),
            wall: T0,
        };
        let s = b.snapshot(&key(0), almost).expect("키");
        assert_eq!(
            s.state,
            UsageVendorState::Rejected {
                retry_in_secs: 1,
                detail: expected_detail.clone()
            }
        );

        let s = snap(&b, 0, at(190, T0));
        let next = (secs(100) + cooldown(0)).max(secs(190));
        assert_eq!(
            s.state,
            UsageVendorState::Failed {
                next_attempt_in_secs: (next - secs(190)).as_secs(),
                detail: expected_detail,
            }
        );
    }

    #[test]
    fn six_states_fold_with_next_attempt_always_present() {
        let failures: [(Option<ProbeError>, &str); 5] = [
            (None, "Ready"),
            (Some(ProbeError::NotInstalled), "NotInstalled"),
            (Some(ProbeError::Unauthenticated), "NeedsLogin"),
            (Some(ProbeError::Timeout), "Failed"),
            (
                Some(ProbeError::RateLimited { retry_after: None }),
                "Rejected",
            ),
        ];
        for (error, expected) in failures {
            let mut b = book();
            match error {
                Some(error) => {
                    fail(&mut b, 0, error.into(), at(100, T0));
                }
                None => succeed(&mut b, 0, active(0, None, None), at(100, T0)),
            }
            let s = snap(&b, 0, at(100, T0));
            assert_eq!(tag(&s.state), expected);
            assert_eq!(
                detail_of(&s.state).is_some(),
                expected != "Ready",
                "{expected}"
            );
        }
        let mut b = book();
        let mut unavailable = active(0, None, None);
        unavailable.limits_unavailable = Some(rich_detail());
        succeed(&mut b, 0, unavailable, at(100, T0));
        assert_eq!(tag(&snap(&b, 0, at(100, T0)).state), "Unavailable");

        // 다음 시도 = 다음 자동 기한까지 — 지나면 0 에서 멈춘다.
        let mut b = book();
        fail(&mut b, 0, ProbeError::Timeout.into(), at(100, T0));
        let due = secs(100) + cooldown(0);
        let next_attempt = |b: &UsageBook, now: Now| match snap(b, 0, now).state {
            UsageVendorState::Failed {
                next_attempt_in_secs,
                ..
            } => next_attempt_in_secs,
            other => panic!("Failed 여야 한다: {other:?}"),
        };
        assert_eq!(next_attempt(&b, at(100, T0)), cooldown(0).as_secs());
        assert_eq!(
            next_attempt(
                &b,
                Now {
                    mono: due,
                    wall: T0
                }
            ),
            0
        );
        assert_eq!(
            next_attempt(
                &b,
                Now {
                    mono: due + secs(3_600),
                    wall: T0
                }
            ),
            0
        );
    }

    #[test]
    fn rate_limit_wait_falls_back_and_is_clamped() {
        let cases = [
            (None, REJECT_FALLBACK),
            (Some(Duration::ZERO), REJECT_FALLBACK),
            (Some(secs(90)), secs(90)),
            (Some(REJECT_MAX + secs(1)), REJECT_MAX),
            (Some(Duration::MAX), REJECT_MAX),
        ];
        for (retry_after, expected) in cases {
            let mut b = book();
            let applied = fail(
                &mut b,
                0,
                ProbeError::RateLimited { retry_after }.into(),
                at(100, T0),
            );
            assert!(applied.reject_changed, "{retry_after:?}");
            let s = snap(&b, 0, at(100, T0));
            assert!(
                matches!(s.state, UsageVendorState::Rejected { retry_in_secs, .. } if retry_in_secs == expected.as_secs()),
                "{retry_after:?}: {:?}",
                s.state
            );
            assert_eq!(
                b.next_auto(&key(0), at(100, T0)),
                Some((secs(100) + cooldown(0)).max(secs(100) + expected))
            );
        }

        // 같은 기한이 다시 서면 저장할 것이 없다 · 거절이 아닌 실패는 기한을 안 건드린다.
        let mut b = book();
        let rate_limited = || {
            ProbeFailure::from(ProbeError::RateLimited {
                retry_after: Some(secs(90)),
            })
        };
        assert!(fail(&mut b, 0, rate_limited(), at(100, T0)).reject_changed);
        assert!(!fail(&mut b, 0, rate_limited(), at(100, T0)).reject_changed);
        assert!(!fail(&mut b, 0, ProbeError::Timeout.into(), at(300, T0)).reject_changed);
    }

    // ── 래치(R32·§3 #29) ──

    #[test]
    fn latch_sets_once_after_reset_and_survives_a_wall_rewind() {
        let mut b = book();
        let r = T0 + 100;
        succeed(
            &mut b,
            0,
            active(0, w(Some(40.0), Some(r)), None),
            at(10, T0),
        );
        let rev = b.revision(&key(0)).expect("키");

        assert!(!b.eval_time(&key(0), at(20, r - 1)));
        assert_eq!(b.revision(&key(0)), Some(rev));
        assert!(b.eval_time(&key(0), at(30, r)));
        assert_eq!(b.revision(&key(0)), Some(rev + 1));
        assert!(!b.eval_time(&key(0), at(40, r + 5)), "한 번만 선다");
        assert_eq!(b.revision(&key(0)), Some(rev + 1));

        // 벽시계가 되감겨도 유지 · % 는 지우지 않는다.
        assert!(!b.eval_time(&key(0), at(50, T0)));
        let s = snap(&b, 0, at(50, T0));
        assert!(expired(&s.five_hour));
        assert_eq!(pct(&s.five_hour), Some(40.0));
    }

    #[test]
    fn latch_is_cleared_only_by_a_new_value() {
        let mut b = book();
        let r = T0 + 100;
        succeed(
            &mut b,
            0,
            active(0, w(Some(40.0), Some(r)), None),
            at(10, T0),
        );
        assert!(b.eval_time(&key(0), at(20, r)));

        b.apply_passive(&passive(0, w(None, Some(r + 30)), None), at(30, T0));
        assert!(
            expired(&snap(&b, 0, at(30, T0)).five_hour),
            "같은 리셋만 — 새 값이 아니다"
        );
        b.apply_passive(&passive(0, w(None, Some(r + 5 * H)), None), at(40, T0));
        let s = snap(&b, 0, at(40, T0));
        assert!(
            expired(&s.five_hour),
            "바뀐 리셋만 — % 는 지워도 새 값은 아니다"
        );
        assert_eq!(pct(&s.five_hour), None);

        b.apply_passive(&passive(0, w(Some(3.0), None), None), at(50, T0));
        assert!(
            !expired(&snap(&b, 0, at(50, T0)).five_hour),
            "줍기 % 가 내린다"
        );

        // 조회 성공도 내린다.
        assert!(b.eval_time(&key(0), at(60, r + 5 * H)));
        succeed(
            &mut b,
            0,
            active(0, w(Some(4.0), Some(r + 10 * H)), None),
            at(70, r + 5 * H),
        );
        assert!(!expired(&snap(&b, 0, at(70, r + 5 * H)).five_hour));
    }

    #[test]
    fn latch_covers_every_window_and_a_past_reset_latches_on_arrival() {
        let mut b = book();
        let r = T0 + 100;
        let mut full = active(0, w(Some(1.0), Some(r + H)), w(Some(2.0), Some(r)));
        full.model_scoped = Some(vec![scoped("m", w(Some(3.0), Some(r)))]);
        succeed(&mut b, 0, full, at(10, T0));
        assert!(b.eval_time(&key(0), at(20, r)));
        let s = snap(&b, 0, at(20, r));
        assert!(!expired(&s.five_hour));
        assert!(expired(&s.weekly));
        assert!(s.model_scoped[0].window.expired);

        // 이미 지난 리셋을 받으면 받은 자리에서 만료다 — 줍기도 조회도.
        let mut b = book();
        succeed(
            &mut b,
            0,
            active(0, w(Some(5.0), Some(T0 - 1)), None),
            at(10, T0),
        );
        assert!(expired(&snap(&b, 0, at(10, T0)).five_hour));
        let applied = b.apply_passive(&passive(0, None, w(Some(6.0), Some(T0 - 1))), at(20, T0));
        assert!(applied.changed);
        assert!(expired(&snap(&b, 0, at(20, T0)).weekly));
    }

    // ── revision(§3 #49) ──

    #[test]
    fn revision_rises_on_change_and_not_on_a_repeat() {
        let mut b = book();
        assert_eq!(b.revision(&key(0)), Some(0));
        let o = passive(0, w(Some(40.0), Some(T0 + H)), None);
        assert!(b.apply_passive(&o, at(10, T0)).changed);
        assert_eq!(b.revision(&key(0)), Some(1));

        // 같은 값을 다시 주우면 나이만 새로워진다.
        assert_eq!(snap(&b, 0, at(70, T0)).five_hour.expect("창").age_secs, 60);
        assert!(!b.apply_passive(&o, at(70, T0)).changed);
        assert_eq!(b.revision(&key(0)), Some(1));
        assert_eq!(snap(&b, 0, at(70, T0)).five_hour.expect("창").age_secs, 0);

        let mut planned = passive(0, None, None);
        planned.plan = Some("plan-a".to_owned());
        assert!(b.apply_passive(&planned, at(80, T0)).changed);
        assert!(!b.apply_passive(&planned, at(90, T0)).changed);
        assert_eq!(b.revision(&key(0)), Some(2));
    }

    #[test]
    fn revision_rises_when_only_the_next_auto_moves() {
        let mut b = book();
        b.apply_passive(
            &passive(0, w(Some(40.0), None), w(Some(10.0), None)),
            at(200, T0),
        );
        let rev = b.revision(&key(0)).expect("키");
        assert!(
            !b.apply_passive(&passive(0, w(Some(40.0), None), None), at(300, T0))
                .changed
        );
        // 값은 그대로지만 두 창이 다 새로 들어와 기한이 옮는다(D12).
        let applied = b.apply_passive(&passive(0, None, w(Some(10.0), None)), at(400, T0));
        assert_eq!(
            applied,
            PassiveApplied {
                changed: true,
                next_auto_changed: true
            }
        );
        assert_eq!(b.revision(&key(0)), Some(rev + 1));
    }

    #[test]
    fn begin_and_finish_always_bump() {
        let mut b = book();
        assert!(b.begin_probe(&key(0)));
        assert_eq!(
            (b.revision(&key(0)), b.in_flight(&key(0))),
            (Some(1), Some(true))
        );
        assert!(!b.begin_probe(&key(0)), "이미 진행 중");
        assert_eq!(b.revision(&key(0)), Some(1));
        assert!(snap(&b, 0, at(0, T0)).in_flight);

        let o = active(0, w(Some(1.0), None), None);
        b.finish_probe(&key(0), Ok(o.clone()), at(10, T0));
        assert_eq!(
            (b.revision(&key(0)), b.in_flight(&key(0))),
            (Some(2), Some(false))
        );
        // 같은 결과여도 끝은 오른다 · 진행 중이 아니어도 적용한다.
        b.finish_probe(&key(0), Ok(o), at(10, T0));
        assert_eq!(b.revision(&key(0)), Some(3));
    }

    #[test]
    fn cells_are_independent() {
        let mut b = book();
        b.apply_passive(&passive(0, w(Some(40.0), None), None), at(10, T0));
        assert!(b.begin_probe(&key(0)));
        assert_eq!(b.revision(&key(1)), Some(0));
        assert_eq!(b.in_flight(&key(1)), Some(false));
        assert_eq!(snap(&b, 1, at(10, T0)).five_hour, None);
        assert!(b.begin_probe(&key(1)), "다른 칸은 따로 시작한다");
    }

    #[test]
    fn unknown_keys_are_no_ops() {
        let mut b = book();
        let stranger = UsageKey {
            vendor: UsageVendorKey::new("no-such-vendor"),
            account: UsageAccountKey::default(),
        };
        assert_eq!(b.revision(&stranger), None);
        assert_eq!(b.in_flight(&stranger), None);
        assert_eq!(b.next_auto(&stranger, at(0, T0)), None);
        assert_eq!(b.snapshot(&stranger, at(0, T0)), None);
        assert!(!b.begin_probe(&stranger));
        assert!(!b.eval_time(&stranger, at(0, T0)));
        assert_eq!(
            b.finish_probe(
                &stranger,
                Err(ProbeError::RateLimited { retry_after: None }.into()),
                at(0, T0)
            ),
            FinishApplied::default()
        );
        let mut o = passive(0, w(Some(1.0), None), None);
        o.vendor = stranger.vendor;
        assert_eq!(b.apply_passive(&o, at(0, T0)), PassiveApplied::default());
        for k in b.keys() {
            assert_eq!(b.revision(&k), Some(0));
        }
    }

    // ── 스냅숏(§1-6) ──

    #[test]
    fn snapshot_carries_the_key_and_window_fields() {
        let mut b = book();
        for (i, k) in b.keys().iter().enumerate() {
            let s = snap(&b, i, at(0, T0));
            let word = serde_json::to_value(s.vendor).expect("직렬화");
            assert!(
                word.as_str()
                    .is_some_and(|w| w.eq_ignore_ascii_case(k.vendor.as_str())),
                "{word}"
            );
            assert_eq!(s.account_key, UsageAccountKey::default().as_str());
            assert_eq!(s.revision, 0);
        }

        succeed(
            &mut b,
            0,
            active(0, w(Some(40.0), Some(T0 + H)), None),
            at(100, T0),
        );
        b.apply_passive(&passive(0, None, w(Some(9.0), None)), at(130, T0));
        let s = snap(&b, 0, at(160, T0));
        assert_eq!(
            s.five_hour,
            Some(UsageWindow {
                used_pct: Some(40.0),
                resets_at: Some((T0 + H) as u64),
                age_secs: 60,
                expired: false,
                source: UsageSourceKind::Active,
            })
        );
        let weekly = s.weekly.expect("주간");
        assert_eq!(
            (weekly.source, weekly.age_secs, weekly.resets_at),
            (UsageSourceKind::Passive, 30, None)
        );
    }

    #[test]
    fn a_vendor_word_the_wire_cannot_name_has_no_snapshot() {
        let odd = UsageVendorKey::new("no-such-backend");
        let (mut b, loud) = capture_loud(|| {
            UsageBook::new(&[
                (odd, usage_probes()[0].policy()),
                (odd, usage_probes()[1].policy()),
            ])
        });
        assert_eq!(loud.len(), 1, "벤더마다 한 번: {loud:?}");
        let k = UsageKey {
            vendor: odd,
            account: UsageAccountKey::default(),
        };
        assert_eq!(b.keys(), vec![k.clone()], "같은 벤더는 한 칸");
        assert!(b.begin_probe(&k));
        assert_eq!(b.revision(&k), Some(1));
        assert_eq!(b.snapshot(&k, at(0, T0)), None);
    }

    #[test]
    fn extreme_values_do_not_panic() {
        let mut b = book();
        // 리셋 끝값 — 0 이하는 없는 것으로 받는다 · 두 끝의 차도 넘치지 않는다.
        succeed(
            &mut b,
            0,
            active(0, w(Some(1.0), Some(i64::MIN)), w(None, Some(i64::MAX))),
            at(0, i64::MIN),
        );
        let s = snap(&b, 0, at(0, i64::MIN));
        assert_eq!((pct(&s.five_hour), reset(&s.five_hour)), (Some(1.0), None));
        assert_eq!(reset(&s.weekly), Some(i64::MAX as u64));
        b.apply_passive(
            &passive(0, w(None, Some(1)), w(None, Some(i64::MIN))),
            at(1, i64::MIN),
        );
        let s = snap(&b, 0, at(1, i64::MIN));
        assert_eq!(
            (pct(&s.five_hour), reset(&s.five_hour)),
            (Some(1.0), Some(1))
        );
        assert_eq!(
            reset(&s.weekly),
            Some(i64::MAX as u64),
            "음수 리셋은 안 실린 것"
        );
        b.apply_passive(&passive(0, None, w(None, Some(1))), at(2, i64::MIN));
        assert_eq!(reset(&snap(&b, 0, at(2, i64::MIN)).weekly), Some(1));
        b.eval_time(&key(0), at(3, i64::MAX));
        b.eval_time(&key(0), at(4, i64::MIN));

        // 단조 시간 끝값 — 거절 기한·다음 자동·나이가 포화한다.
        let mut b = book();
        let edge = Now {
            mono: Duration::MAX,
            wall: T0,
        };
        succeed(&mut b, 0, active(0, w(Some(1.0), None), None), edge);
        assert_eq!(b.next_auto(&key(0), edge), Some(Duration::MAX));
        fail(
            &mut b,
            0,
            ProbeError::RateLimited {
                retry_after: Some(Duration::MAX),
            }
            .into(),
            edge,
        );
        // 포화한 기한 = 지금이라 받은 자리에서 조회 실패로 새겨진다.
        let s = snap(&b, 0, edge);
        assert!(
            matches!(
                s.state,
                UsageVendorState::Failed {
                    next_attempt_in_secs: 0,
                    ..
                }
            ),
            "{:?}",
            s.state
        );
        // 되돌아간 시각 — 나이는 0, 새긴 상태는 그대로다.
        let s = snap(&b, 0, at(0, T0));
        assert_eq!(s.five_hour.expect("창").age_secs, 0);
        assert!(
            matches!(s.state, UsageVendorState::Failed { .. }),
            "{:?}",
            s.state
        );

        // 유한수 약속을 어긴 % — 같은 NaN 이 거듭 와도 바뀜이 아니다.
        let mut b = book();
        let nan = passive(0, w(Some(f64::NAN), None), None);
        assert!(b.apply_passive(&nan, at(0, T0)).changed);
        assert!(!b.apply_passive(&nan, at(1, T0)).changed);
    }

    // ── 리뷰 1회차 보정 ──

    #[test]
    fn rejection_ending_is_written_once_by_eval_time_and_keeps_detail() {
        let mut b = seeded();
        fail(
            &mut b,
            0,
            ProbeFailure::with_detail(ProbeError::RateLimited { retry_after: None }, rich_detail()),
            at(100, T0),
        );
        let until = secs(100) + REJECT_FALLBACK;
        let rev = b.revision(&key(0)).expect("키");
        let before_end = Now {
            mono: until - secs(1),
            wall: T0,
        };
        assert!(!b.eval_time(&key(0), before_end));
        assert_eq!(tag(&snap(&b, 0, before_end).state), "Rejected");

        let end = Now {
            mono: until,
            wall: T0,
        };
        let fallback = snap(&b, 0, end);
        assert!(b.eval_time(&key(0), end));
        assert_eq!(b.revision(&key(0)), Some(rev + 1));
        let written = snap(&b, 0, end);
        assert_eq!(written.state, fallback.state, "폴백과 새긴 값이 같다");
        assert_eq!(written.revision, fallback.revision + 1);
        assert_eq!(tag(&written.state), "Failed");
        assert_eq!(
            detail_of(&written.state),
            Some(&wire_detail(&rich_detail()))
        );

        let later = Now {
            mono: until + secs(60),
            wall: T0,
        };
        assert!(!b.eval_time(&key(0), later), "한 번만 새긴다");
        assert_eq!(b.revision(&key(0)), Some(rev + 1));
    }

    #[test]
    fn passive_application_also_writes_a_finished_rejection() {
        let mut b = seeded();
        fail(
            &mut b,
            0,
            ProbeError::RateLimited { retry_after: None }.into(),
            at(100, T0),
        );
        let end = Now {
            mono: secs(100) + REJECT_FALLBACK,
            wall: T0,
        };
        // 값은 그대로인 줍기 — 바뀜은 거절 끝뿐이다.
        let same = passive(0, w(None, Some(T0 + 5 * H)), None);
        assert!(b.apply_passive(&same, end).changed);
        assert!(!b.eval_time(&key(0), end));
        assert_eq!(tag(&snap(&b, 0, end).state), "Failed");
    }

    #[test]
    fn a_successful_probe_clears_the_rejection() {
        let mut b = seeded();
        let rate_limited = || {
            ProbeFailure::from(ProbeError::RateLimited {
                retry_after: Some(REJECT_MAX),
            })
        };
        fail(&mut b, 0, rate_limited(), at(100, T0));
        let mut unavailable = active(0, None, None);
        unavailable.limits_unavailable = Some(rich_detail());
        assert!(b.begin_probe(&key(0)));
        let applied = b.finish_probe(&key(0), Ok(unavailable), at(200, T0));
        assert!(applied.reject_changed, "지운 기한도 저장할 바뀜이다");
        assert_eq!(
            b.next_auto(&key(0), at(200, T0)),
            Some(secs(200) + cooldown(0))
        );

        fail(&mut b, 0, rate_limited(), at(300, T0));
        succeed(&mut b, 0, active(0, w(Some(1.0), None), None), at(400, T0));
        assert_eq!(
            b.next_auto(&key(0), at(400, T0)),
            Some(secs(400) + cooldown(0))
        );
        assert_eq!(snap(&b, 0, at(400, T0)).state, UsageVendorState::Ready);
    }

    #[test]
    fn non_positive_resets_are_read_as_absent() {
        let mut b = book();
        for bad in [0, -1, i64::MIN] {
            let applied = b.apply_passive(
                &passive(0, w(None, Some(bad)), w(None, Some(bad))),
                at(10, T0),
            );
            assert_eq!(applied, PassiveApplied::default(), "{bad}");
        }
        assert_eq!(b.revision(&key(0)), Some(0));
        let s = snap(&b, 0, at(10, T0));
        assert_eq!((s.five_hour, s.weekly), (None, None));

        succeed(
            &mut b,
            0,
            active(0, w(None, Some(-5)), w(Some(3.0), Some(0))),
            at(20, T0),
        );
        let s = snap(&b, 0, at(20, T0));
        assert_eq!(s.five_hour, None, "리셋만 있던 창은 안 선다");
        assert_eq!((pct(&s.weekly), reset(&s.weekly)), (Some(3.0), None));

        // 들고 있는 리셋을 0 이하 관측이 지우지도 바꾸지도 않는다.
        b.apply_passive(&passive(0, None, w(None, Some(T0 + H))), at(30, T0));
        let rev = b.revision(&key(0));
        assert!(
            !b.apply_passive(&passive(0, None, w(None, Some(-1))), at(40, T0))
                .changed
        );
        assert_eq!(b.revision(&key(0)), rev);
        assert_eq!(
            reset(&snap(&b, 0, at(40, T0)).weekly),
            Some((T0 + H) as u64)
        );
    }

    #[test]
    fn a_passive_entry_refuses_an_active_observation() {
        let mut b = seeded();
        let rev = b.revision(&key(0));
        let (applied, loud) =
            capture_loud(|| b.apply_passive(&active(0, w(Some(99.0), None), None), at(200, T0)));
        assert_eq!(applied, PassiveApplied::default());
        assert_eq!(loud.len(), 1, "{loud:?}");
        assert!(!loud[0].contains("99"), "값은 로그에 싣지 않는다: {loud:?}");
        assert!(loud[0].contains("vendor="), "{loud:?}");
        assert_eq!(b.revision(&key(0)), rev);
        let s = snap(&b, 0, at(200, T0));
        assert_eq!(
            (pct(&s.five_hour), pct(&s.weekly)),
            (Some(40.0), Some(10.0))
        );
    }

    #[test]
    fn a_probe_result_with_the_wrong_source_still_finishes_the_probe() {
        let mut b = seeded();
        assert!(b.begin_probe(&key(0)));
        let (_, loud) = capture_loud(|| {
            b.finish_probe(
                &key(0),
                Ok(passive(0, w(Some(5.0), None), None)),
                at(200, T0),
            )
        });
        assert_eq!(loud.len(), 1, "{loud:?}");
        assert!(loud[0].contains("vendor="), "{loud:?}");
        assert_eq!(b.in_flight(&key(0)), Some(false));
        assert_eq!(
            b.next_auto(&key(0), at(200, T0)),
            Some(secs(200) + cooldown(0))
        );
        let s = snap(&b, 0, at(200, T0));
        assert_eq!(pct(&s.five_hour), Some(5.0));
        assert_eq!(s.five_hour.expect("창").source, UsageSourceKind::Active);
        assert_eq!(s.weekly, None, "조회 결과로 적용한다 — 빠진 창 = 없음");
    }

    #[test]
    fn model_scoped_entries_are_capped() {
        let over = MODEL_SCOPED_MAX + 4;
        let many = |pct_from: usize| -> Vec<ScopedWindowObs> {
            (0..over)
                .map(|i| {
                    let pct = (i >= pct_from).then_some(i as f64);
                    scoped(&format!("m{i:02}"), w(pct, Some(T0 + H)))
                })
                .collect()
        };
        let mut b = book();
        let mut o = passive(0, None, None);
        o.model_scoped = Some(many(0));
        let (_, loud) = capture_loud(|| b.apply_passive(&o, at(10, T0)));
        assert_eq!(loud.len(), 1, "생산자가 약속을 어겼다: {loud:?}");
        let s = snap(&b, 0, at(10, T0));
        assert_eq!(s.model_scoped.len(), MODEL_SCOPED_MAX);
        assert_eq!(
            s.model_scoped.last().map(|m| m.label.clone()),
            Some(format!("m{:02}", MODEL_SCOPED_MAX - 1)),
            "앞에서부터 받는다"
        );

        let mut full = active(0, None, None);
        full.model_scoped = Some(many(0));
        let ((), loud) = capture_loud(|| succeed(&mut b, 0, full, at(20, T0)));
        assert_eq!(loud.len(), 1, "{loud:?}");
        assert_eq!(snap(&b, 0, at(20, T0)).model_scoped.len(), MODEL_SCOPED_MAX);

        // 버린 칸의 % 는 한도가 있다는 증거로도 안 센다.
        let mut b = book();
        let mut unavailable = active(0, None, None);
        unavailable.limits_unavailable = Some(rich_detail());
        succeed(&mut b, 0, unavailable, at(10, T0));
        let mut beyond = passive(0, None, None);
        beyond.model_scoped = Some(many(MODEL_SCOPED_MAX));
        assert_eq!(
            b.apply_passive(&beyond, at(20, T0)),
            PassiveApplied::default()
        );
        assert_eq!(tag(&snap(&b, 0, at(20, T0)).state), "Unavailable");
    }

    #[test]
    fn a_pct_arriving_after_the_reset_drops_the_stale_reset() {
        let r = T0 + 100;
        let mut b = book();
        succeed(
            &mut b,
            0,
            active(0, w(Some(40.0), Some(r)), None),
            at(10, T0),
        );
        assert!(b.eval_time(&key(0), at(20, r)));
        b.apply_passive(&passive(0, w(Some(3.0), None), None), at(30, r + 10));
        let five = snap(&b, 0, at(30, r + 10)).five_hour.expect("창");
        assert_eq!(
            (five.used_pct, five.resets_at, five.expired),
            (Some(3.0), None, false)
        );

        // 리셋 전에 온 % 는 들고 있던 리셋을 지킨다.
        let mut b = book();
        succeed(
            &mut b,
            0,
            active(0, w(Some(40.0), Some(r)), None),
            at(10, T0),
        );
        b.apply_passive(&passive(0, w(Some(41.0), None), None), at(20, r - 1));
        assert_eq!(reset(&snap(&b, 0, at(20, r - 1)).five_hour), Some(r as u64));
    }

    #[test]
    fn a_cleared_pct_no_longer_counts_as_fresh() {
        let mut b = seeded();
        b.apply_passive(&passive(0, w(Some(41.0), None), None), at(200, T0));
        // 바뀐 리셋이 5시간 창의 % 를 지운다 — 앞서 선 새로 들어옴도 내린다.
        b.apply_passive(&passive(0, w(None, Some(T0 + 10 * H)), None), at(250, T0));
        let applied = b.apply_passive(&passive(0, None, w(Some(11.0), None)), at(300, T0));
        assert!(!applied.next_auto_changed);
        assert_eq!(
            b.next_auto(&key(0), at(300, T0)),
            Some(secs(100) + cooldown(0))
        );
    }
}
