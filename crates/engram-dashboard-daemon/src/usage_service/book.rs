//! 사용량 책 — 벤더 칸마다의 순수 상태기(TRD §1-4). 스레드·락·시계·I/O·발행이 없다: 부르는 쪽이 시각
//! ([`Now`])을 넘기고 무엇이 바뀌었는지·무엇을 할지(조회 시작·발행·잠)를 돌려받는다. 잠그는 것·조회를 띄우는
//! 것·발행하는 것은 서비스 몫이다.
//!
//! ★revision 은 보이는 것이 바뀔 때만 오른다(§3 #49)★ — 스냅숏이 싣는 값(창 %·리셋·만료 · 모델별 · plan ·
//!   상태 · detail) 또는 다음 자동 조회 기한. 같은 값을 다시 주워 나이만 새로워진 것은 바뀜이 아니다. 예외 = 조회
//!   시작·끝은 늘 오른다(`in_flight` 가 바뀐다).
//! ★시각만 흘러 생기는 바뀜(만료 래치 · 거절 끝 → 조회 실패)은 [`UsageBook::eval_time`] 이 칸에 새기고 그때
//!   revision 이 오른다★ — ★스냅숏을 뜨기 전에 이것을 부르는 것은 부르는 쪽(서비스)의 의무다★. 줍기 적용·조회
//!   끝·[`UsageBook::plan_tick`]·발행 한 장([`UsageBook::broadcast_sheet`]·[`UsageBook::coalesce_passive`])은 스스로
//!   부른다.
//! ★빚 — revision 이 오르면 그 칸은 구독자 전부에게 한 번 발행할 빚을 진다★(줍기 값의 바뀜 = 합칠 수 있는 빚 ·
//!   그 밖(래치·거절 끝 — 줍기 적용 안에서 선 것도 · 조회 시작·끝 · 복원) = 곧바로 갚을 빚). ★빚을 갚는 것은 구독자
//!   전부에게 가는 발행뿐이다★ — [`UsageBook::broadcast_sheet`] · [`Coalesce::PublishNow`] · [`TickPlan::publish`]
//!   가 갚음과 한 장 뜨기를 한 호출에서 한다(락을 다시 잡아 갚으면 그 사이 선 빚이 안 실린 채 지워진다). 구독 추가의
//!   첫 한 장(`eval_time` + `snapshot`)은 그 연결에만 가므로 갚지 않는다 — 깨운 스케줄러가 전부에게 낸다(새 연결엔
//!   한 장이 겹친다). ★그래서 서비스가 발행을 빠뜨려도 구독된 칸이면 다음 [`UsageBook::plan_tick`] 이 갚는다★.
//! ★무엇을 받아도 패닉하지 않는다★ — 릴리즈는 `panic = "abort"` 다. 시각 산술은 전부 포화·checked 다.

use std::time::Duration;

use engram_dashboard_agent::usage::{
    ProbeError, ProbeFailure, ScopedWindowObs, UsageAccountKey, UsageDetail, UsageKey,
    UsageObservation, UsagePolicy, UsageVendorKey, WindowObs,
};
use engram_dashboard_protocol::{
    AgentBackendKind, UsageLimitSnapshot, UsageScopedWindow, UsageStateDetail, UsageVendorState,
    UsageWindow,
};
use serde::de::value::{Error as ValueError, StrDeserializer};
use serde::de::IntoDeserializer;
use serde::Deserialize;

use super::reject_store::RejectEntry;

/// 거절 대기 — 상류가 쓸 만한 대기를 안 줬을 때(없음·0)(D4).
pub const REJECT_FALLBACK: Duration = Duration::from_secs(5 * 60);
/// 거절 대기 상한 — 상류가 준 대기를 여기서 자른다(§3 #37).
pub const REJECT_MAX: Duration = Duration::from_secs(24 * 60 * 60);
/// 두 `resets_at` 이 이 안이면 같은 리셋이다 — 경계 포함(60초 차 = 같음 · 61초 차 = 다름, §3 #46).
pub const RESET_SAME_TOLERANCE: Duration = Duration::from_secs(60);
/// 모델별 창을 한 관측에서 받는 상한 — 줍기는 pump 스레드에서 곧장 돈다. 넘는 것은 앞에서부터 이만큼만 받는다.
pub const MODEL_SCOPED_MAX: usize = 16;
/// 줍기 발행을 칸마다 이 창으로 합친다(§3 #60) — 줍기 변화만이다. 조회 시작·끝·래치는 곧바로 나간다.
/// 출처 규칙이 아니다 — 스트림 입구([`UsageBook::apply_passive`])로 몰려 드는 바뀜의 빈도 제어다(§3 #88).
pub const USAGE_PUBLISH_COALESCE: Duration = Duration::from_secs(1);
/// 스케줄러 잠 상한(§3 #58) — 대기 타이머가 절전을 안 셀 수 있고 리셋은 벽시계라, 복귀·시계 이동 뒤 늦음을
/// 여기로 묶는다.
pub const SCHEDULE_MAX_SLEEP: Duration = Duration::from_secs(60);

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

/// 요청 종류(§1-4 요청 표). `Refresh` = ⟳ — 쿨타임을 무시하되 거절은 못 넘는다(D11).
///
/// ★`Refresh` 에 최소 간격을 두지 않는다(사용자 결정 2026-09-29)★ — 조회가 막 끝난 직후의 ⟳ 도 새 조회다.
///   남는 제동은 거절 기한(R24)과 진행 중 합류뿐이다 — 피어에 수동 간격이 없고, 상류가 막으면 부르는 쪽이 스스로
///   줄인다.
// ADR-0257
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    Get,
    Refresh,
}

/// [`UsageBook::judge`] 의 판정.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Judgment {
    /// 조회 없이 지금 캐시로 답한다 — 기한 전 `Get`.
    Cached,
    /// 거절 기한 전의 `Refresh` — 조회를 내보내지 않는다(R24).
    Rejected,
    /// 진행 중인 조회가 있다 — 그 끝을 기다린다.
    Join,
    /// ★책이 이미 `in_flight` 를 세우고 revision 을 올렸다★([`UsageBook::begin_probe`] 와 같다) — 부르는 쪽은
    /// [`UsageBook::broadcast_sheet`] 로 발행하고 조회를 띄운다. 끝은 반드시 [`UsageBook::finish_probe`] 로
    /// 닫는다 — 안 닫으면 그 칸은 영영 진행 중이다.
    Start,
}

/// [`UsageBook::coalesce_passive`] 의 결과.
#[derive(Debug, Clone, PartialEq)]
pub enum Coalesce {
    /// 지금 그 칸의 구독자 전부에게 보낼 한 장 — 책은 빚을 갚은 것으로 적었다.
    PublishNow(UsageLimitSnapshot),
    /// 지금 보낼 한 장이 없다 — 부르는 쪽은 스케줄러를 깨운다. 빚이 남았으면 [`UsageBook::plan_tick`] 이 합침
    /// 기한에 그때의 한 장을 낸다.
    Deferred,
}

/// [`UsageBook::plan_tick`] 의 결과. 두 목록의 순서 = 칸을 만든 순서.
#[derive(Debug, Clone, PartialEq)]
pub struct TickPlan {
    /// 시작한 조회 — 책이 이미 `in_flight` 를 세웠다([`Judgment::Start`] 와 같은 책임이 따른다).
    pub start: Vec<UsageKey>,
    /// 지금 그 칸의 구독자 전부에게 보낼 한 장들(키 = 명부에서 대상을 찾을 칸) — 책은 빚을 갚은 것으로 적었다.
    pub publish: Vec<(UsageKey, UsageLimitSnapshot)>,
    /// 다음 깰 때까지 — `0 < sleep ≤ SCHEDULE_MAX_SLEEP`.
    pub sleep: Duration,
}

// 손으로 쓴다 — 파생하면 잠이 0 이라 그대로 기다리는 구동부가 돈다.
impl Default for TickPlan {
    fn default() -> Self {
        Self {
            start: Vec::new(),
            publish: Vec::new(),
            sleep: SCHEDULE_MAX_SLEEP,
        }
    }
}

/// 칸마다 하나. 키 목록은 만들 때 고정된다.
pub struct UsageBook {
    cells: Vec<Cell>,
}

impl UsageBook {
    /// 받은 벤더마다 기본 계정 칸 하나 — 같은 벤더가 두 번 오면 앞의 것만 본다. ★wire 벤더로 못 바꾸는 낱말은
    /// 등록하지 않는다(warn 한 번)★ — 그래서 등록된 칸은 늘 스냅숏을 뜨고, 그 벤더는 모든 메서드에 모르는 키다.
    pub fn new(vendors: &[(UsageVendorKey, UsagePolicy)]) -> Self {
        let mut cells: Vec<Cell> = Vec::with_capacity(vendors.len());
        let mut seen: Vec<UsageVendorKey> = Vec::with_capacity(vendors.len());
        for &(vendor, policy) in vendors {
            if seen.contains(&vendor) {
                continue;
            }
            seen.push(vendor);
            if let Some(wire) = wire_vendor(vendor) {
                cells.push(Cell::new(vendor, wire, policy));
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

    /// 다음 자동 조회의 `mono` 기한(R29) = `max(max(last_query, fresh_reset) + cooldown, reject_until)` —
    /// 기준점이 없으면 `reject_until`, 그것도 없으면 `now.mono`(= 지금 기한). 진행 중인지는 보지 않는다.
    /// `None` = 모르는 키.
    pub fn next_auto(&self, key: &UsageKey, now: Now) -> Option<Duration> {
        self.cell(key).map(|cell| cell.next_auto(now))
    }

    /// 줍기(pump) 입구 — 관측 한 건의 값을 조회 성공과 같은 병합으로 합친다(R16·D12 · 출처를 가르지 않는다).
    /// ★같은 것은 값 병합뿐이다★ — 조회 끝만 하는 일(거절 기한 지움 · `last_query` 기준점 · D12 표시 지움 · 성공의
    /// `Ready`/`Unavailable` 접기)은 여기 없다. 값의 바뀜은 합칠 수 있는 빚이다([`UsageBook::coalesce_passive`]).
    /// 칸 = 관측의 벤더 + 기본 계정. 모르는 벤더면 아무것도 안 한다.
    pub fn apply_passive(&mut self, obs: &UsageObservation, now: Now) -> PassiveApplied {
        let account = UsageAccountKey::default();
        let Some(cell) = self
            .cells
            .iter_mut()
            .find(|cell| cell.key.vendor == obs.vendor && cell.key.account == account)
        else {
            return PassiveApplied::default();
        };
        let before = cell.visible(now);
        cell.merge(obs, now);
        let latched = cell.eval_time(now);
        let after = cell.visible(now);
        let changed = !before.same(&after);
        if changed {
            // 합칠 수 있는 것은 줍기 값의 바뀜뿐이다 — 같은 호출이 새긴 래치·거절 끝은 곧바로 나간다.
            cell.bump_owing(if latched {
                Owed::Now
            } else {
                Owed::Coalescable
            });
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
                cell.start();
                true
            }
            _ => false,
        }
    }

    /// 조회 끝(성공·한도 정보 없음·실패 — 시한 초과 포함). 성공의 값은 줍기와 같은 한 길로 합친다 — 안 실린
    /// 창은 들고 있던 값 그대로다. 성공은 `Ready`(한도 정보 없음이면 창을 비우고 `Unavailable`) · 실패는 값을
    /// 건드리지 않고 그 실패 상태로 접는다. `last_query = now.mono` 가 새 기준점이 된다(D12 의 새로 들어옴도
    /// 여기서 지운다). 성공(한도 정보 없음 포함)은 거절 기한을 지운다. 진행 중이 아니었어도 적용하고, revision
    /// 은 늘 +1 이다. 모르는 키면 아무것도 안 한다.
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
            Ok(obs) => cell.apply_success(&obs, now),
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

    /// `now` 기준 wire 한 장 — 빚을 갚지 않는다(구독 첫 한 장은 이것으로 뜬다). `None` = 모르는 키.
    /// ★먼저 [`UsageBook::eval_time`] 을 부른다★ — 안 불러도 끝난 거절은 조회 실패로 보이지만, 그 바뀜은 revision
    /// 에 없다.
    pub fn snapshot(&self, key: &UsageKey, now: Now) -> Option<UsageLimitSnapshot> {
        self.cell(key).map(|cell| cell.sheet(now))
    }

    /// 그 칸의 구독자 전부에게 보낼 한 장 — 조회 시작·끝 · 요청 때 래치의 발행. `eval_time`(바뀌면 revision +1)
    /// → 빚 갚음(직전 발행 = `now`) → 스냅숏을 한 호출에서 한다(머리 「빚」). `None` = 모르는 키.
    pub fn broadcast_sheet(&mut self, key: &UsageKey, now: Now) -> Option<UsageLimitSnapshot> {
        let cell = self.cell_mut(key)?;
        if cell.eval_time(now) {
            cell.bump();
        }
        Some(cell.discharge(now))
    }

    /// 요청 하나의 판정 — §1-4 요청 표를 **위 행부터 차례로** 본다: ① `Get` 이고 `now < next_auto` → 캐시
    /// ② `Refresh` 이고 `now < reject_until` → 거절 ③ 그 밖 → 진행 중이면 합류, 아니면 시작. ★합류는 ③ 에만
    /// 있다★ — 진행 중이어도 앞 두 행에 걸리면 그 행으로 답한다. `None` = 모르는 키(아무것도 안 바뀐다).
    ///
    /// 판정은 [`UsageBook::eval_time`] 이 새기는 것(래치·거절 끝)을 읽지 않는다 — 부르는 쪽이 판정 전에 그것을
    /// 부르는 것은(요청 도착 행) 그 바뀜을 발행하려는 것이지 판정을 바꾸려는 것이 아니다.
    pub fn judge(&mut self, key: &UsageKey, kind: RequestKind, now: Now) -> Option<Judgment> {
        let cell = self.cell_mut(key)?;
        let judgment = match kind {
            RequestKind::Get if now.mono < cell.next_auto(now) => Judgment::Cached,
            RequestKind::Refresh if cell.reject_until.is_some_and(|until| now.mono < until) => {
                Judgment::Rejected
            }
            _ if cell.in_flight => Judgment::Join,
            _ => {
                cell.start();
                Judgment::Start
            }
        };
        Some(judgment)
    }

    /// 저장된 거절 기한을 되살린다 — 조립 때 새 책에 한 번(§1-4 「거절 저장」). `until_epoch_s > now.wall` 인
    /// 항목만, 남은 시간 = `min(until − wall, REJECT_MAX)` 를 `now.mono` 에 더한다 — 넘치면 그 항목만 버리고 warn.
    /// 되살린 칸 = `Rejected` + 분류 낱말만 든 detail(파일은 분류·원문을 싣지 않는다) · revision +1. 책에 없는
    /// 키는 버리고 warn. 같은 키가 둘이면 더 늦은 기한이 남는다. 돌려주는 값 = 되살린 항목 수.
    pub fn restore_rejects(&mut self, entries: &[RejectEntry], now: Now) -> usize {
        let mut restored = 0;
        for entry in entries {
            if entry.until_epoch_s <= now.wall {
                continue;
            }
            let vendor = entry.key.vendor.as_str();
            let Some(cell) = self.cell_mut(&entry.key) else {
                tracing::warn!(vendor, "저장된 거절 기한의 칸이 책에 없다 — 버린다");
                continue;
            };
            let remaining =
                Duration::from_secs(entry.until_epoch_s.abs_diff(now.wall)).min(REJECT_MAX);
            let Some(until) = now.mono.checked_add(remaining) else {
                tracing::warn!(vendor, "저장된 거절 기한이 단조 시계에서 넘친다 — 버린다");
                continue;
            };
            cell.reject_until = Some(cell.reject_until.map_or(until, |kept| kept.max(until)));
            cell.state =
                CellState::Rejected(kind_only(&ProbeError::RateLimited { retry_after: None }));
            cell.bump();
            restored += 1;
        }
        restored
    }

    /// 저장할 거절 기한 — `reject_until > now.mono` 인 칸마다 하나(지난 것은 안 싣는다). `until_epoch_s = now.wall +
    /// 남은 초의 올림`(포화) — 올림이라 되살린 기한이 원래보다 이르지 않다.
    pub fn reject_entries(&self, now: Now) -> Vec<RejectEntry> {
        self.cells
            .iter()
            .filter_map(|cell| {
                let until = cell.reject_until.filter(|&until| until > now.mono)?;
                let remaining = i64::try_from(ceil_secs(until - now.mono)).unwrap_or(i64::MAX);
                Some(RejectEntry {
                    key: cell.key.clone(),
                    until_epoch_s: now.wall.saturating_add(remaining),
                })
            })
            .collect()
    }

    /// 줍기가 칸을 바꾼 뒤(`PassiveApplied.changed`) 같은 `now` 로 부른다 — 줍기만 합친다(§3 #60). 곧바로 갚을
    /// 빚(래치·거절 끝·조회 시작·끝 등)이 있으면 `PublishNow` · 합칠 수 있는 빚뿐이면 직전 발행이 없거나
    /// [`USAGE_PUBLISH_COALESCE`] 이상 지났을 때 `PublishNow`, 아니면 `Deferred` · 빚이 없으면 `Deferred`.
    /// `None` = 모르는 키.
    pub fn coalesce_passive(&mut self, key: &UsageKey, now: Now) -> Option<Coalesce> {
        let cell = self.cell_mut(key)?;
        Some(if cell.owes_at(now.mono) {
            Coalesce::PublishNow(cell.discharge(now))
        } else {
            Coalesce::Deferred
        })
    }

    /// 스케줄러 한 번(§1-4 「스케줄러 깸」 · §3 #58·#85). 구독된 칸마다: [`UsageBook::eval_time`] → 진행 중 아님 ∧
    /// `now ≥ next_auto` 면 조회 시작(요청 경로와 같은 시작 — 뒤이은 요청은 합류한다) → 곧바로 갚을 빚이 있거나
    /// 합칠 수 있는 빚의 합침 기한이 찼으면 빚을 갚고 그 한 장을 `publish` 에 싣는다. ★서비스가 발행을 빠뜨린
    /// 빚도 여기서 갚는다★ — 구독된 칸이면 늦어도 다음 깸(≤ [`SCHEDULE_MAX_SLEEP`])에 닿는다.
    ///
    /// `sleep` = 구독된 칸의 `next_auto`(진행 중이면 뺀다) · 안 선 래치의 `resets_at`(벽시계 차) · `Rejected` 칸의
    /// `reject_until` · 합칠 빚의 합침 기한 중 가장 이른 것까지, 상한 [`SCHEDULE_MAX_SLEEP`]. ★찬 기한은 이 한
    /// 번에 전부 처리하므로 `sleep` 은 0 이 아니다★. 구독된 칸이 없으면 시작도 기한도 없다(상한 잠).
    ///
    /// ★구독 밖 칸은 건너뛸 뿐 빚을 건드리지 않는다★ — `subscribed` 는 명부 락 아래 뜬 사본이라 낡을 수 있다:
    /// 방금 구독한 연결이 받을 발행을 낡은 사본이 지우면 안 된다. 남은 빚은 나중에 한 장이 겹쳐 나갈 뿐이다
    /// (revision 이 가른다). `subscribed` 의 중복·모르는 벤더는 무시한다.
    pub fn plan_tick<'a>(
        &mut self,
        subscribed: impl IntoIterator<Item = &'a UsageVendorKey>,
        now: Now,
    ) -> TickPlan {
        let subscribed: Vec<UsageVendorKey> = subscribed.into_iter().copied().collect();
        let mut plan = TickPlan::default();
        for cell in &mut self.cells {
            if !subscribed.contains(&cell.key.vendor) {
                continue;
            }
            if cell.eval_time(now) {
                cell.bump();
            }
            if !cell.in_flight && now.mono >= cell.next_auto(now) {
                cell.start();
                plan.start.push(cell.key.clone());
            }
            if cell.owes_at(now.mono) {
                plan.publish.push((cell.key.clone(), cell.discharge(now)));
            }
            if let Some(wait) = cell.next_wake(now) {
                plan.sleep = plan.sleep.min(wait);
            }
        }
        plan
    }

    fn cell(&self, key: &UsageKey) -> Option<&Cell> {
        self.cells.iter().find(|cell| cell.key == *key)
    }

    fn cell_mut(&mut self, key: &UsageKey) -> Option<&mut Cell> {
        self.cells.iter_mut().find(|cell| cell.key == *key)
    }
}

/// 칸이 쥔 상태. 비정상 다섯은 모두 detail 을 든다(§1-4 「실패 추적」) — `Rejected` 가 끝나 `Failed` 로 보여도
/// 그대로다. 창 값을 싣는 관측은 어느 상태든 `Ready` 로 되돌린다([`Cell::merge`]).
#[derive(Debug, Clone, PartialEq, Eq)]
enum CellState {
    Ready,
    NotInstalled(UsageDetail),
    NeedsLogin(UsageDetail),
    Unavailable(UsageDetail),
    Failed(UsageDetail),
    Rejected(UsageDetail),
}

/// 칸이 구독자 전부에게 진 발행 빚(머리 「빚」). 선언 순서가 크기다 — 겹치면 큰 쪽이 남는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Owed {
    None,
    /// 줍기 값의 바뀜뿐이다 — 직전 발행 뒤 [`USAGE_PUBLISH_COALESCE`] 가 지나면 갚는다.
    Coalescable,
    /// 곧바로 갚는다.
    Now,
}

struct Cell {
    key: UsageKey,
    policy: UsagePolicy,
    wire_vendor: AgentBackendKind,
    five_hour: Option<Window>,
    weekly: Option<Window>,
    /// 라벨 순(같은 라벨끼리는 받은 순) — 받을 때 정렬해 두어 상류의 나열 순서가 바뀜으로 읽히지 않게.
    model_scoped: Vec<(String, Window)>,
    plan: Option<String>,
    state: CellState,
    last_query: Option<Duration>,
    /// D12 — 두 창에 새 `used_pct` 가 다 실려 쿨타임 기점이 다시 선 `mono`.
    fresh_reset: Option<Duration>,
    /// ★상태와 따로 산다★ — 값이 와서 `Ready` 로 보여도 이 기한 전에는 자동 조회도 ⟳ 도 안 나간다(R24).
    reject_until: Option<Duration>,
    /// D12 — 지금 기준점 뒤로 `used_pct` 가 실려 온 창(§3 #4 — 리셋만 온 창은 안 센다).
    fresh_five_hour: bool,
    fresh_weekly: bool,
    in_flight: bool,
    revision: u64,
    /// 발행 빚과 합침 기점(§3 #60) — 보이는 것이 아니다(revision 밖). `last_publish` = 마지막으로 빚을 갚은 `mono`.
    owed: Owed,
    last_publish: Option<Duration>,
}

impl Cell {
    fn new(vendor: UsageVendorKey, wire_vendor: AgentBackendKind, policy: UsagePolicy) -> Self {
        Self {
            key: UsageKey {
                vendor,
                account: UsageAccountKey::default(),
            },
            policy,
            wire_vendor,
            five_hour: None,
            weekly: None,
            model_scoped: Vec::new(),
            plan: None,
            state: CellState::Ready,
            last_query: None,
            fresh_reset: None,
            reject_until: None,
            fresh_five_hour: false,
            fresh_weekly: false,
            in_flight: false,
            revision: 0,
            owed: Owed::None,
            last_publish: None,
        }
    }

    fn bump(&mut self) {
        self.bump_owing(Owed::Now);
    }

    fn bump_owing(&mut self, owed: Owed) {
        self.revision = self.revision.saturating_add(1);
        self.owed = self.owed.max(owed);
    }

    fn start(&mut self) {
        self.in_flight = true;
        self.bump();
    }

    /// 빚을 갚고 그 한 장을 뜬다 — 빚이 없어도 뜬다(겹친 한 장은 revision 이 가른다).
    fn discharge(&mut self, now: Now) -> UsageLimitSnapshot {
        // 늦게 읽은 시각으로 불려도 되감지 않는다 — 되감으면 합침 창이 앞당겨져 1초 안에 두 번 나간다.
        self.last_publish = Some(
            self.last_publish
                .map_or(now.mono, |last| last.max(now.mono)),
        );
        self.owed = Owed::None;
        self.sheet(now)
    }

    fn owes_at(&self, mono: Duration) -> bool {
        match self.owed {
            Owed::None => false,
            Owed::Coalescable => self.coalesce_deadline() <= mono,
            Owed::Now => true,
        }
    }

    /// 직전 발행 뒤 합침 창이 닫히는 `mono` — 발행한 적이 없으면 0(= 이미 닫혔다).
    fn coalesce_deadline(&self) -> Duration {
        self.last_publish.map_or(Duration::ZERO, |last| {
            last.saturating_add(USAGE_PUBLISH_COALESCE)
        })
    }

    /// [`UsageBook::plan_tick`] 의 칸 몫 — 이 칸의 가장 이른 다음 기한까지 남은 시간. 찬 기한은 부르는 쪽이 이미
    /// 처리했으므로 앞에 남은 기한만 센다 — 그래서 0 을 내지 않는다. 기한이 없으면 `None`.
    fn next_wake(&self, now: Now) -> Option<Duration> {
        let ahead = |deadline: Duration| deadline.checked_sub(now.mono).filter(|d| !d.is_zero());
        let auto = (!self.in_flight)
            .then(|| self.next_auto(now))
            .and_then(ahead);
        // 벽시계 차를 그대로 잠으로 쓴다 — `mono` 에 더했다 빼면 `mono` 끝에서 포화해 0 이 된다.
        let reset = self
            .windows()
            .filter(|window| !window.expired)
            .filter_map(|window| window.resets_at)
            .filter(|&at| at > now.wall)
            .map(|at| Duration::from_secs(at.abs_diff(now.wall)))
            .min();
        let reject = match self.state {
            CellState::Rejected(_) => self.reject_until.and_then(ahead),
            _ => None,
        };
        let coalesce = (self.owed == Owed::Coalescable)
            .then(|| self.coalesce_deadline())
            .and_then(ahead);
        [auto, reset, reject, coalesce].into_iter().flatten().min()
    }

    fn windows(&self) -> impl Iterator<Item = &Window> {
        self.five_hour
            .iter()
            .chain(self.weekly.iter())
            .chain(self.model_scoped.iter().map(|(_, w)| w))
    }

    fn next_auto(&self, now: Now) -> Duration {
        match self.last_query.max(self.fresh_reset) {
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

    /// 관측 하나의 값을 합친다 — 줍기·조회 성공 공통의 한 길. 실린 창·칸은 덮고 안 실린 것은 둔다
    /// (`model_scoped`·`plan` 은 실리면 통째로 바꾼다). ★창 값(% 또는 리셋)을 하나라도 실으면 어느 상태에서든
    /// `Ready` 다★ — 값이 온다는 것이 정상의 증거다(§3 #52). 거절 기한은 건드리지 않는다.
    fn merge(&mut self, obs: &UsageObservation, now: Now) {
        if let Some(window) = obs.five_hour {
            merge_window(&mut self.five_hour, window, now).mark(&mut self.fresh_five_hour);
        }
        if let Some(window) = obs.weekly {
            merge_window(&mut self.weekly, window, now).mark(&mut self.fresh_weekly);
        }
        if let Some(list) = &obs.model_scoped {
            self.model_scoped = scoped_windows(list, now.mono);
        }
        if let Some(plan) = &obs.plan {
            self.plan = Some(plan.clone());
        }
        if self.fresh_five_hour && self.fresh_weekly {
            self.fresh_reset = Some(now.mono);
            self.fresh_five_hour = false;
            self.fresh_weekly = false;
        }
        if carries_value(obs) {
            self.state = CellState::Ready;
        }
    }

    fn apply_success(&mut self, obs: &UsageObservation, now: Now) {
        // 상류가 답을 줬으니 거절은 끝났다 — 남겨 두면 지난 기한이 다음 자동 조회를 붙든다.
        self.reject_until = None;
        self.merge(obs, now);
        self.state = match &obs.limits_unavailable {
            // 계정에 대한 답이다 — wire 규칙 「`Unavailable` 은 값을 싣지 않는다」.
            Some(detail) => {
                self.five_hour = None;
                self.weekly = None;
                self.model_scoped.clear();
                CellState::Unavailable(detail.clone())
            }
            None => CellState::Ready,
        };
    }

    fn apply_failure(&mut self, failure: ProbeFailure, mono: Duration) {
        let ProbeFailure { error, detail } = failure;
        let detail = detail.unwrap_or_else(|| kind_only(&error));
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

    fn sheet(&self, now: Now) -> UsageLimitSnapshot {
        UsageLimitSnapshot {
            vendor: self.wire_vendor,
            account_key: self.key.account.as_str().to_owned(),
            five_hour: self.five_hour.as_ref().and_then(|w| w.wire(now.mono)),
            weekly: self.weekly.as_ref().and_then(|w| w.wire(now.mono)),
            model_scoped: self
                .model_scoped
                .iter()
                .filter_map(|(label, w)| {
                    w.wire(now.mono).map(|window| UsageScopedWindow {
                        label: label.clone(),
                        window,
                    })
                })
                .collect(),
            plan: self.plan.clone(),
            in_flight: self.in_flight,
            state: self.wire_state(now),
            revision: self.revision,
        }
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
    /// 만료 래치(R32) — `wall >= resets_at` 을 한 번 보면 서고, 이 창의 새 값(`used_pct`)만 내린다. 벽시계가
    /// 되감겨도 안 내린다. % 는 지우지 않는다 — 가리는 것은 받는 쪽이다.
    expired: bool,
    /// 이 값을 관측한 `mono` — 나이의 기점.
    observed: Duration,
}

impl Window {
    /// 두 칸 다 없으면 `None` — 값이 없는 창을 두지 않는다.
    fn observed(obs: WindowObs, mono: Duration) -> Option<Self> {
        let obs = ingest(obs);
        (obs.used_pct.is_some() || obs.resets_at.is_some()).then_some(Self {
            used_pct: obs.used_pct,
            resets_at: obs.resets_at,
            expired: false,
            observed: mono,
        })
    }

    fn looks_same(&self, other: &Self) -> bool {
        same_pct(self.used_pct, other.used_pct)
            && self.resets_at == other.resets_at
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

/// 실린 창 하나를 들고 있던 창에 합친다(§1-4 「줍기 관측」 · §3 #3·#46) — 줍기·조회 공통.
fn merge_window(slot: &mut Option<Window>, obs: WindowObs, now: Now) -> Fresh {
    let obs = ingest(obs);
    let carried = if obs.used_pct.is_some() {
        Fresh::Carried
    } else {
        Fresh::Untouched
    };
    let Some(window) = slot.as_mut() else {
        *slot = Window::observed(obs, now.mono);
        return carried;
    };
    let mut fresh = carried;
    match (obs.used_pct, window.resets_at, obs.resets_at) {
        (Some(pct), kept, new) => {
            window.used_pct = Some(pct);
            window.expired = false;
            window.observed = now.mono;
            // 리셋이 지난 뒤 온 % 는 새 창의 값이고 그 창의 리셋은 모른다(R32 「새 값이 올 때까지」) — 지난 리셋을
            // 남기면 받은 자리에서 다시 만료로 보인다.
            if new.is_none() && kept.is_some_and(|at| at <= now.wall) {
                window.resets_at = None;
            }
        }
        (None, Some(old), Some(new)) if old.abs_diff(new) > RESET_SAME_TOLERANCE.as_secs() => {
            // 새 창의 옛 % 는 틀린 값이다. 남는 값은 이 관측의 리셋뿐이라 나이도 이 관측 것이다.
            window.used_pct = None;
            window.observed = now.mono;
            fresh = Fresh::Cleared;
        }
        _ => {}
    }
    if obs.resets_at.is_some() {
        window.resets_at = obs.resets_at;
    }
    fresh
}

/// 목록 전량 교체용 — `Some` 은 합치지 않고 통째로 바꾼다(`UsageObservation` 계약).
fn scoped_windows(list: &[ScopedWindowObs], mono: Duration) -> Vec<(String, Window)> {
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
            Window::observed(scoped.window, mono).map(|w| (scoped.label.clone(), w))
        })
        .collect();
    windows.sort_by(|(a, _), (b, _)| a.cmp(b));
    windows
}

/// 받는 자리([`ingest`] · [`MODEL_SCOPED_MAX`])를 거친 뒤에도 창 값이 남는가 — 버려질 값은 안 센다.
fn carries_value(obs: &UsageObservation) -> bool {
    let has = |w: WindowObs| {
        let w = ingest(w);
        w.used_pct.is_some() || w.resets_at.is_some()
    };
    obs.five_hour.is_some_and(has)
        || obs.weekly.is_some_and(has)
        || obs
            .model_scoped
            .iter()
            .flat_map(|list| list.iter().take(MODEL_SCOPED_MAX))
            .any(|scoped| has(scoped.window))
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

fn kind_only(error: &ProbeError) -> UsageDetail {
    UsageDetail {
        kind: error.kind_word(),
        code: None,
        upstream: None,
    }
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
pub(crate) fn wire_vendor(vendor: UsageVendorKey) -> Option<AgentBackendKind> {
    let word: StrDeserializer<'_, ValueError> = vendor.as_str().into_deserializer();
    match AgentBackendKind::deserialize(word) {
        Ok(kind) => Some(kind),
        Err(e) => {
            tracing::warn!(
                vendor = vendor.as_str(),
                error = %e,
                "조회기 낱말이 wire 벤더가 아니다 — 이 벤더는 칸을 두지 않는다"
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
    use engram_dashboard_agent::usage::{UpstreamText, UsageSource};

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

    // ── 병합(R16·§3 #3·#46·#88) ──

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
    fn a_probe_result_keeps_windows_it_does_not_carry() {
        let mut b = book();
        let mut full = active(
            0,
            w(Some(40.0), Some(T0 + H)),
            w(Some(10.0), Some(T0 + 9 * H)),
        );
        full.model_scoped = Some(vec![scoped("m", w(Some(3.0), None))]);
        full.plan = Some("plan-a".to_owned());
        succeed(&mut b, 0, full, at(100, T0));
        let held = snap(&b, 0, at(200, T0));

        // null 창 · 빠진 창 · 빠진 목록 · 빠진 plan — 줍기와 같이 전부 그대로다.
        succeed(&mut b, 0, active(0, w(None, None), None), at(200, T0));
        let s = snap(&b, 0, at(200, T0));
        assert_eq!(
            (&s.five_hour, &s.weekly, &s.model_scoped, &s.plan),
            (
                &held.five_hour,
                &held.weekly,
                &held.model_scoped,
                &held.plan
            )
        );
        assert_eq!(s.state, UsageVendorState::Ready);

        // 실린 창만 덮는다.
        succeed(&mut b, 0, active(0, None, w(Some(12.0), None)), at(300, T0));
        let s = snap(&b, 0, at(300, T0));
        assert_eq!(
            (pct(&s.five_hour), pct(&s.weekly), reset(&s.weekly)),
            (Some(40.0), Some(12.0), Some((T0 + 9 * H) as u64))
        );
        assert_eq!(s.model_scoped.len(), 1);
        assert_eq!(s.plan.as_deref(), Some("plan-a"));
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
        assert_eq!(s.five_hour.expect("리셋은 남는다").age_secs, 0);
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
        assert_eq!(s.five_hour.expect("창").age_secs, 0);
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

        let mut opus = active(0, None, None);
        opus.model_scoped = Some(vec![scoped("Opus", w(Some(30.0), None))]);
        succeed(&mut b, 0, opus, at(300, T0));
        assert_eq!(labels(&b), ["Opus"]);
        let mut emptied = active(0, None, None);
        emptied.model_scoped = Some(vec![]);
        succeed(&mut b, 0, emptied, at(400, T0));
        assert!(
            labels(&b).is_empty(),
            "빈 목록도 실린 것이다 — 사라진 모델 창을 지운다"
        );
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
        assert_eq!(snap(&b, 0, at(200, T0)).state, UsageVendorState::Ready);
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

        // plan 을 안 실은 답은 들고 있던 plan 을 둔다.
        let mut no_plan = active(0, None, None);
        no_plan.limits_unavailable = Some(rich_detail());
        succeed(&mut b, 0, no_plan, at(2_000, T0));
        let s = snap(&b, 0, at(2_000, T0));
        assert_eq!(tag(&s.state), "Unavailable");
        assert_eq!(s.plan.as_deref(), Some("plan-b"));
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

    fn unavailable() -> UsageBook {
        let mut b = book();
        let mut unavailable = active(0, None, None);
        unavailable.limits_unavailable = Some(rich_detail());
        succeed(&mut b, 0, unavailable, at(100, T0));
        b
    }

    #[test]
    fn unavailable_turns_ready_on_any_window_value() {
        // 창 값이 없는 관측(plan 만 · 값 없는 모델별 창)은 상태를 안 바꾼다 — plan 은 합친다.
        let mut b = unavailable();
        let mut no_value = passive(0, w(None, None), None);
        no_value.plan = Some("plan-a".to_owned());
        no_value.model_scoped = Some(vec![scoped("m", w(None, None))]);
        assert!(b.apply_passive(&no_value, at(200, T0)).changed);
        let s = snap(&b, 0, at(200, T0));
        assert_eq!(tag(&s.state), "Unavailable");
        assert_eq!(s.plan.as_deref(), Some("plan-a"));
        assert_eq!((s.five_hour, s.weekly), (None, None));
        assert!(s.model_scoped.is_empty());

        // 리셋만 실려도 값이다.
        let applied = b.apply_passive(&passive(0, w(None, Some(T0 + H)), None), at(300, T0));
        assert!(applied.changed);
        let s = snap(&b, 0, at(300, T0));
        assert_eq!(s.state, UsageVendorState::Ready);
        assert_eq!(
            (pct(&s.five_hour), reset(&s.five_hour)),
            (None, Some((T0 + H) as u64))
        );

        // % 도 · 모델별 창의 값도 · 조회 성공도 같다.
        let mut b = unavailable();
        assert!(
            b.apply_passive(&passive(0, None, w(Some(7.0), None)), at(200, T0))
                .changed
        );
        let s = snap(&b, 0, at(200, T0));
        assert_eq!(
            (s.state, pct(&s.weekly)),
            (UsageVendorState::Ready, Some(7.0))
        );

        let mut b = unavailable();
        let mut scoped_value = passive(0, None, None);
        scoped_value.model_scoped = Some(vec![scoped("m", w(None, Some(T0 + H)))]);
        assert!(b.apply_passive(&scoped_value, at(200, T0)).changed);
        let s = snap(&b, 0, at(200, T0));
        assert_eq!(s.state, UsageVendorState::Ready);
        assert_eq!(s.model_scoped.len(), 1);

        let mut b = unavailable();
        succeed(&mut b, 0, active(0, None, None), at(200, T0));
        assert_eq!(snap(&b, 0, at(200, T0)).state, UsageVendorState::Ready);
    }

    #[test]
    fn a_value_turns_every_abnormal_state_ready_and_keeps_merging() {
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
            assert_ne!(tag(&snap(&b, 0, at(250, T0)).state), "Ready", "{kind}");
            let applied = b.apply_passive(&passive(0, w(Some(70.0), None), None), at(250, T0));
            assert!(applied.changed, "{kind}");
            let s = snap(&b, 0, at(250, T0));
            assert_eq!(s.state, UsageVendorState::Ready, "{kind}: detail 도 지운다");
            assert_eq!(
                (pct(&s.five_hour), pct(&s.weekly)),
                (Some(70.0), Some(10.0)),
                "{kind}: 실린 창만 덮는다"
            );
        }

        // 끝난 거절(`Failed` 로 새겨진 뒤) · 되살린 거절도 같다.
        let mut b = seeded();
        fail(&mut b, 0, rate_limited(Some(secs(90))), at(200, T0));
        assert!(b.eval_time(&key(0), at(290, T0)));
        assert_eq!(tag(&snap(&b, 0, at(290, T0)).state), "Failed");
        b.apply_passive(&passive(0, None, w(None, Some(T0 + 100 * H))), at(300, T0));
        assert_eq!(snap(&b, 0, at(300, T0)).state, UsageVendorState::Ready);

        let mut b = book();
        b.restore_rejects(&[entry(0, T0 + 90)], at(10, T0));
        b.apply_passive(&passive(0, w(Some(5.0), None), None), at(20, T0));
        assert_eq!(snap(&b, 0, at(20, T0)).state, UsageVendorState::Ready);
    }

    #[test]
    fn a_value_during_a_rejection_shows_ready_but_keeps_the_deadline() {
        for restored in [false, true] {
            let mut b = settled();
            if restored {
                b.restore_rejects(&[entry(0, T0 + 190)], at(10, T0));
            } else {
                fail(&mut b, 0, rate_limited(Some(secs(90))), at(110, T0));
            }
            let until = secs(200);
            let saved = b.reject_entries(at(150, T0));
            assert_eq!(saved, vec![entry(0, T0 + 50)], "restored {restored}");
            b.apply_passive(&passive(0, w(Some(41.0), None), None), at(150, T0));
            assert_eq!(snap(&b, 0, at(150, T0)).state, UsageVendorState::Ready);

            // 기한은 그대로 산다 — 저장도 · ⟳ 거절도 · 다음 자동 기한도.
            assert_eq!(b.reject_entries(at(150, T0)), saved, "restored {restored}");
            let before_end = Now {
                mono: until - Duration::from_millis(1),
                wall: T0,
            };
            assert_eq!(
                judge(&mut b, 0, RequestKind::Refresh, before_end),
                Judgment::Rejected
            );
            assert!(b.next_auto(&key(0), before_end) >= Some(until));
            assert!(tick(&mut b, &[0], before_end).start.is_empty());

            // 거절 끝은 `Ready` 를 조회 실패로 되돌리지 않는다 — 새길 것도 발행할 것도 없다.
            let end = Now {
                mono: until,
                wall: T0,
            };
            let rev = b.revision(&key(0));
            assert!(!b.eval_time(&key(0), end), "restored {restored}");
            assert_eq!(b.revision(&key(0)), rev);
            assert_eq!(snap(&b, 0, end).state, UsageVendorState::Ready);
            assert_eq!(judge(&mut b, 0, RequestKind::Refresh, end), Judgment::Start);
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

        // 조회 성공이라도 그 창을 안 실었으면(null · 빠짐) 래치는 그대로다 — 풀면 옛 % 가 되살아난다.
        let later = r + 10 * H;
        assert!(b.eval_time(&key(0), at(80, later)));
        succeed(
            &mut b,
            0,
            active(0, w(None, None), w(Some(5.0), None)),
            at(90, later),
        );
        let s = snap(&b, 0, at(90, later));
        assert!(expired(&s.five_hour));
        assert_eq!(pct(&s.five_hour), Some(4.0));
        assert!(!expired(&s.weekly));
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
            })
        );
        let weekly = s.weekly.expect("주간");
        assert_eq!((weekly.age_secs, weekly.resets_at), (30, None));
    }

    #[test]
    fn a_vendor_word_the_wire_cannot_name_is_not_registered() {
        let odd = UsageVendorKey::new("no-such-backend");
        let (mut b, loud) = capture_loud(|| {
            UsageBook::new(&[
                (odd, usage_probes()[0].policy()),
                (usage_probes()[0].key(), usage_probes()[0].policy()),
                (odd, usage_probes()[1].policy()),
                (usage_probes()[0].key(), usage_probes()[1].policy()),
            ])
        });
        assert_eq!(loud.len(), 1, "벤더마다 한 번: {loud:?}");
        assert_eq!(
            b.keys(),
            vec![key(0)],
            "못 바꾸는 벤더는 없고 같은 벤더는 한 칸"
        );
        let k = UsageKey {
            vendor: odd,
            account: UsageAccountKey::default(),
        };
        assert_eq!(b.revision(&k), None);
        assert!(!b.begin_probe(&k));
        assert_eq!(b.snapshot(&k, at(0, T0)), None);
        assert_eq!(b.broadcast_sheet(&k, at(0, T0)), None);
        assert_eq!(b.coalesce_passive(&k, at(0, T0)), None);
        assert_eq!(b.judge(&k, RequestKind::Get, at(0, T0)), None);
        let mut o = passive(0, w(Some(1.0), None), None);
        o.vendor = odd;
        assert_eq!(b.apply_passive(&o, at(0, T0)), PassiveApplied::default());
        assert_eq!(b.plan_tick(&[odd], at(0, T0)), TickPlan::default());
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
        // 창 값이 없는 줍기 — 바뀜은 거절 끝뿐이다.
        let mut plan_only = passive(0, None, None);
        plan_only.plan = Some("plan-a".to_owned());
        assert!(b.apply_passive(&plan_only, end).changed);
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
    fn both_entries_merge_whatever_the_observation_says_its_source_is() {
        let mut b = seeded();
        let (applied, loud) =
            capture_loud(|| b.apply_passive(&active(0, w(Some(99.0), None), None), at(200, T0)));
        assert!(applied.changed);
        assert!(loud.is_empty(), "{loud:?}");
        let s = snap(&b, 0, at(200, T0));
        assert_eq!(
            (pct(&s.five_hour), pct(&s.weekly)),
            (Some(99.0), Some(10.0))
        );

        assert!(b.begin_probe(&key(0)));
        let (_, loud) = capture_loud(|| {
            b.finish_probe(
                &key(0),
                Ok(passive(0, w(Some(5.0), None), None)),
                at(300, T0),
            )
        });
        assert!(loud.is_empty(), "{loud:?}");
        assert_eq!(b.in_flight(&key(0)), Some(false));
        assert_eq!(
            b.next_auto(&key(0), at(300, T0)),
            Some(secs(300) + cooldown(0))
        );
        let s = snap(&b, 0, at(300, T0));
        assert_eq!((pct(&s.five_hour), pct(&s.weekly)), (Some(5.0), Some(10.0)));
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

        // 버린 칸의 값은 정상의 증거로도 안 센다.
        let mut b = unavailable();
        let mut beyond = passive(0, None, None);
        beyond.model_scoped = Some(
            (0..over)
                .map(|i| {
                    let value = (i >= MODEL_SCOPED_MAX).then_some(i as f64);
                    scoped(&format!("m{i:02}"), w(value, None))
                })
                .collect(),
        );
        let (applied, _) = capture_loud(|| b.apply_passive(&beyond, at(200, T0)));
        assert_eq!(applied, PassiveApplied::default());
        assert_eq!(tag(&snap(&b, 0, at(200, T0)).state), "Unavailable");
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

    fn at_ms(mono_ms: u64, wall: i64) -> Now {
        Now {
            mono: Duration::from_millis(mono_ms),
            wall,
        }
    }

    /// [`seeded`] + 시험 산술의 전제 — 조회 끝(mono 100) 뒤 몇 분 안에는 다음 자동 기한이 오지 않는다.
    fn settled() -> UsageBook {
        assert!(
            cooldown(0) > secs(300) + SCHEDULE_MAX_SLEEP,
            "시험 전제: 쿨타임이 몇 분보다 길다"
        );
        seeded()
    }

    fn judge(b: &mut UsageBook, i: usize, kind: RequestKind, now: Now) -> Judgment {
        b.judge(&key(i), kind, now).expect("아는 키")
    }

    fn rate_limited(retry_after: Option<Duration>) -> ProbeFailure {
        ProbeError::RateLimited { retry_after }.into()
    }

    fn restored_detail() -> UsageStateDetail {
        wire_detail(&kind_only(&ProbeError::RateLimited { retry_after: None }))
    }

    fn entry(i: usize, until_epoch_s: i64) -> RejectEntry {
        RejectEntry {
            key: key(i),
            until_epoch_s,
        }
    }

    /// 스케줄러 한 번 + 불변식: 잠 ∈ (0, 상한] · 같은 순간 다시 돌면 할 일이 없다(0 초 잠 되풀이 없음).
    fn tick(b: &mut UsageBook, subscribed: &[usize], now: Now) -> TickPlan {
        let vendors: Vec<UsageVendorKey> = subscribed.iter().map(|&i| key(i).vendor).collect();
        let plan = b.plan_tick(&vendors, now);
        assert!(
            plan.sleep > Duration::ZERO && plan.sleep <= SCHEDULE_MAX_SLEEP,
            "{plan:?}"
        );
        let again = b.plan_tick(&vendors, now);
        assert_eq!(
            again,
            TickPlan {
                sleep: plan.sleep,
                ..TickPlan::default()
            },
            "찬 기한은 한 번에 처리한다"
        );
        plan
    }

    // ── 판정(§1-4 요청 표) ──

    #[test]
    fn get_before_the_deadline_is_cached_and_starts_nothing() {
        let mut b = settled();
        let due = secs(100) + cooldown(0);
        let rev = b.revision(&key(0));
        let before = Now {
            mono: due - Duration::from_millis(1),
            wall: T0,
        };
        assert_eq!(judge(&mut b, 0, RequestKind::Get, before), Judgment::Cached);
        assert_eq!(
            (b.revision(&key(0)), b.in_flight(&key(0))),
            (rev, Some(false))
        );

        let at_due = Now {
            mono: due,
            wall: T0,
        };
        assert_eq!(judge(&mut b, 0, RequestKind::Get, at_due), Judgment::Start);
        assert_eq!(
            (b.revision(&key(0)), b.in_flight(&key(0))),
            (rev.map(|r| r + 1), Some(true))
        );

        // 기준점이 없으면 지금이 기한이다.
        let mut b = book();
        assert_eq!(
            judge(&mut b, 0, RequestKind::Get, at(0, T0)),
            Judgment::Start
        );
    }

    #[test]
    fn a_rejection_ending_or_a_reset_passing_does_not_pull_the_probe_forward() {
        let mut b = settled();
        fail(&mut b, 0, rate_limited(Some(secs(90))), at(200, T0));
        let ended = at(300, T0);
        assert!(b.eval_time(&key(0), ended), "거절 끝을 새긴다");
        assert_eq!(judge(&mut b, 0, RequestKind::Get, ended), Judgment::Cached);
        assert!(tick(&mut b, &[0], at(301, T0)).start.is_empty());

        let mut b = settled();
        let past_reset = at(300, T0 + 5 * H + 1);
        assert!(b.eval_time(&key(0), past_reset), "리셋이 지나 래치가 선다");
        assert_eq!(
            judge(&mut b, 0, RequestKind::Get, past_reset),
            Judgment::Cached
        );
        assert!(tick(&mut b, &[0], past_reset).start.is_empty());
        assert_eq!(b.in_flight(&key(0)), Some(false));
    }

    #[test]
    fn a_passive_cooldown_reset_during_a_rejection_cannot_shorten_it() {
        let mut b = settled();
        fail(&mut b, 0, rate_limited(Some(REJECT_MAX)), at(100, T0));
        let until = secs(100) + REJECT_MAX;
        b.apply_passive(
            &passive(0, w(Some(41.0), None), w(Some(11.0), None)),
            at(200, T0),
        );
        let after_cooldown = Now {
            mono: secs(200) + cooldown(0) + secs(1),
            wall: T0,
        };
        assert!(after_cooldown.mono < until);
        assert_eq!(
            judge(&mut b, 0, RequestKind::Get, after_cooldown),
            Judgment::Cached
        );
        assert_eq!(
            judge(&mut b, 0, RequestKind::Refresh, after_cooldown),
            Judgment::Rejected
        );
        assert!(tick(&mut b, &[0], after_cooldown).start.is_empty());

        let ended = Now {
            mono: until,
            wall: T0,
        };
        assert_eq!(judge(&mut b, 0, RequestKind::Get, ended), Judgment::Start);
    }

    /// ★최소 간격이 없다(사용자 결정 2026-09-29)★ — 조회 끝과 같은 순간의 ⟳ 도 새 조회다.
    #[test]
    fn refresh_ignores_the_cooldown_and_starts_right_after_a_finished_probe() {
        let mut b = settled();
        let done = at(100, T0);
        assert!(done.mono < secs(100) + cooldown(0), "쿨타임 전이다");
        assert_eq!(judge(&mut b, 0, RequestKind::Get, done), Judgment::Cached);
        assert_eq!(
            judge(&mut b, 0, RequestKind::Refresh, done),
            Judgment::Start
        );
        assert_eq!(b.in_flight(&key(0)), Some(true));

        b.finish_probe(&key(0), Ok(active(0, w(Some(1.0), None), None)), done);
        assert_eq!(b.in_flight(&key(0)), Some(false));
        assert_eq!(
            judge(&mut b, 0, RequestKind::Refresh, done),
            Judgment::Start,
            "끝난 그 순간에도"
        );

        let mut b = book();
        assert_eq!(
            judge(&mut b, 0, RequestKind::Refresh, at(0, T0)),
            Judgment::Start
        );
    }

    #[test]
    fn refresh_during_a_rejection_is_rejected_until_it_ends() {
        // 과거 시각은 `Duration` 으로 0 으로만 올 수 있다 — 그 몫이 `Some(0)` 칸이다.
        let cases = [
            (None, REJECT_FALLBACK),
            (Some(Duration::ZERO), REJECT_FALLBACK),
            (Some(secs(90)), secs(90)),
            (Some(secs(100 * 24 * 3_600)), REJECT_MAX),
        ];
        for (retry_after, wait) in cases {
            let mut b = book();
            fail(&mut b, 0, rate_limited(retry_after), at(100, T0));
            let before_end = Now {
                mono: secs(100) + wait - Duration::from_millis(1),
                wall: T0,
            };
            assert_eq!(
                judge(&mut b, 0, RequestKind::Refresh, before_end),
                Judgment::Rejected,
                "{retry_after:?}"
            );
            assert_eq!(b.in_flight(&key(0)), Some(false), "{retry_after:?}");
            let end = Now {
                mono: secs(100) + wait,
                wall: T0,
            };
            assert_eq!(
                judge(&mut b, 0, RequestKind::Refresh, end),
                Judgment::Start,
                "{retry_after:?}"
            );
        }
    }

    #[test]
    fn a_three_hour_mono_jump_starts_exactly_one_probe() {
        let mut b = settled();
        let jumped = at(100 + 3 * 3_600, T0);
        assert_eq!(judge(&mut b, 0, RequestKind::Get, jumped), Judgment::Start);
        assert_eq!(judge(&mut b, 0, RequestKind::Get, jumped), Judgment::Join);
        assert_eq!(
            judge(&mut b, 0, RequestKind::Refresh, jumped),
            Judgment::Join
        );
        assert!(tick(&mut b, &[0], jumped).start.is_empty());
    }

    #[test]
    fn a_wall_jump_does_not_change_the_judgment() {
        let judgments = |wall: i64| {
            let mut b = settled();
            let before_due = at(500, wall);
            let due = Now {
                mono: secs(100) + cooldown(0),
                wall,
            };
            b.eval_time(&key(0), before_due);
            [
                judge(&mut b, 0, RequestKind::Get, before_due),
                judge(&mut b, 0, RequestKind::Refresh, at(110, wall)),
                judge(&mut b, 0, RequestKind::Get, due),
            ]
        };
        let expected = [Judgment::Cached, Judgment::Start, Judgment::Join];
        for wall in [T0 - 2 * H, T0, T0 + 2 * H] {
            assert_eq!(judgments(wall), expected, "wall {wall}");
        }
    }

    #[test]
    fn a_timeout_fails_and_waits_a_normal_cooldown() {
        let mut b = settled();
        fail(&mut b, 0, ProbeError::Timeout.into(), at(200, T0));
        assert_eq!(tag(&snap(&b, 0, at(200, T0)).state), "Failed");
        let due = secs(200) + cooldown(0);
        assert_eq!(b.next_auto(&key(0), at(200, T0)), Some(due));
        let before = Now {
            mono: due - secs(1),
            wall: T0,
        };
        assert_eq!(judge(&mut b, 0, RequestKind::Get, before), Judgment::Cached);
        let at_due = Now {
            mono: due,
            wall: T0,
        };
        assert_eq!(judge(&mut b, 0, RequestKind::Get, at_due), Judgment::Start);
    }

    #[test]
    fn requests_join_a_probe_in_flight_only_past_the_earlier_rows() {
        let mut b = book();
        assert!(b.begin_probe(&key(0)));
        let rev = b.revision(&key(0));
        assert_eq!(
            judge(&mut b, 0, RequestKind::Get, at(0, T0)),
            Judgment::Join
        );
        assert_eq!(
            judge(&mut b, 0, RequestKind::Refresh, at(0, T0)),
            Judgment::Join
        );
        assert_eq!(b.revision(&key(0)), rev, "합류는 아무것도 안 바꾼다");

        // 표 순서 그대로 — 진행 중이어도 기한 전 `Get` 은 캐시다. ⟳ 는 직전 조회가 막 끝났어도 합류한다.
        let mut b = settled();
        assert!(b.begin_probe(&key(0)));
        assert_eq!(
            judge(&mut b, 0, RequestKind::Get, at(100, T0)),
            Judgment::Cached
        );
        assert_eq!(
            judge(&mut b, 0, RequestKind::Refresh, at(100, T0)),
            Judgment::Join
        );

        let stranger = UsageKey {
            vendor: UsageVendorKey::new("no-such-vendor"),
            account: UsageAccountKey::default(),
        };
        assert_eq!(b.judge(&stranger, RequestKind::Get, at(0, T0)), None);
    }

    // ── 거절 복원(§1-4 「거절 저장」) ──

    #[test]
    fn restore_keeps_only_future_rejects_and_clamps_them() {
        let mut b = book();
        assert_eq!(
            b.restore_rejects(&[entry(0, T0 - 1), entry(1, T0)], at(10, T0)),
            0
        );
        assert_eq!(
            (b.revision(&key(0)), b.revision(&key(1))),
            (Some(0), Some(0))
        );
        assert_eq!(snap(&b, 0, at(10, T0)).state, UsageVendorState::Ready);

        let mut b = book();
        let entries = [entry(0, T0 + 90), entry(1, T0 + 100 * 24 * H)];
        assert_eq!(b.restore_rejects(&entries, at(10, T0)), 2);
        assert_eq!(b.next_auto(&key(0), at(10, T0)), Some(secs(100)));
        assert_eq!(
            b.next_auto(&key(1), at(10, T0)),
            Some(secs(10) + REJECT_MAX)
        );
        assert_eq!(b.revision(&key(0)), Some(1));
        assert_eq!(
            snap(&b, 0, at(10, T0)).state,
            UsageVendorState::Rejected {
                retry_in_secs: 90,
                detail: Some(restored_detail()),
            }
        );
    }

    #[test]
    fn restore_drops_what_it_cannot_place_without_panicking() {
        let mut b = book();
        let edge = Now {
            mono: Duration::MAX - secs(10),
            wall: T0,
        };
        let (restored, loud) =
            capture_loud(|| b.restore_rejects(&[entry(0, T0 + 100), entry(1, T0 + 5)], edge));
        assert_eq!(restored, 1);
        assert_eq!(loud.len(), 1, "넘친 항목만: {loud:?}");
        assert_eq!(b.revision(&key(0)), Some(0));
        assert_eq!(b.next_auto(&key(1), edge), Some(edge.mono + secs(5)));

        let stranger = RejectEntry {
            key: UsageKey {
                vendor: UsageVendorKey::new("no-such-vendor"),
                account: UsageAccountKey::default(),
            },
            until_epoch_s: T0 + 100,
        };
        let (restored, loud) = capture_loud(|| b.restore_rejects(&[stranger], at(10, T0)));
        assert_eq!(restored, 0);
        assert_eq!(loud.len(), 1, "{loud:?}");

        // 두 끝의 차도 넘치지 않는다.
        let mut b = book();
        assert_eq!(b.restore_rejects(&[entry(0, i64::MAX)], at(0, i64::MIN)), 1);
        assert_eq!(b.next_auto(&key(0), at(0, i64::MIN)), Some(REJECT_MAX));
    }

    #[test]
    fn a_restored_rejection_waits_for_its_end_and_keeps_its_detail() {
        let mut b = book();
        b.restore_rejects(&[entry(0, T0 + 90)], at(10, T0));
        let right_after = at(10, T0);
        assert_eq!(
            judge(&mut b, 0, RequestKind::Refresh, right_after),
            Judgment::Rejected
        );
        assert_eq!(
            judge(&mut b, 0, RequestKind::Get, right_after),
            Judgment::Cached
        );
        let plan = tick(&mut b, &[0, 1], right_after);
        assert_eq!(plan.start, vec![key(1)], "되살린 칸만 막힌다");

        // 기준점이 없으니 기한 = 거절 끝.
        assert_eq!(b.next_auto(&key(0), right_after), Some(secs(100)));
        assert_eq!(
            judge(&mut b, 0, RequestKind::Get, at(99, T0)),
            Judgment::Cached
        );
        let end = at(100, T0);
        assert!(b.eval_time(&key(0), end));
        assert_eq!(
            snap(&b, 0, end).state,
            UsageVendorState::Failed {
                next_attempt_in_secs: 0,
                detail: Some(restored_detail()),
            }
        );
        assert_eq!(judge(&mut b, 0, RequestKind::Get, end), Judgment::Start);
    }

    #[test]
    fn a_restored_rejection_end_folds_and_starts_in_one_tick() {
        let mut b = book();
        b.restore_rejects(&[entry(0, T0 + 30)], at(10, T0));
        assert_eq!(tick(&mut b, &[0], at(10, T0)).sleep, secs(30));
        let plan = tick(&mut b, &[0], at(40, T0));
        assert_eq!(plan.start, vec![key(0)]);
        assert_eq!(
            plan.publish,
            vec![(key(0), snap(&b, 0, at(40, T0)))],
            "한 장"
        );
        assert_eq!(plan.publish[0].1.revision, 3, "복원 · 거절 끝 · 조회 시작");
    }

    #[test]
    fn restoring_the_same_key_twice_keeps_the_later_deadline() {
        for entries in [
            [entry(0, T0 + 90), entry(0, T0 + 300)],
            [entry(0, T0 + 300), entry(0, T0 + 90)],
        ] {
            let mut b = book();
            b.restore_rejects(&entries, at(10, T0));
            assert_eq!(
                b.next_auto(&key(0), at(10, T0)),
                Some(secs(310)),
                "{entries:?}"
            );
        }
    }

    #[test]
    fn reject_entries_carry_only_future_deadlines_rounded_up() {
        let mut b = book();
        fail(&mut b, 0, rate_limited(Some(secs(90))), at(100, T0));
        assert_eq!(
            b.reject_entries(at_ms(100_500, T0 + 7)),
            vec![entry(0, T0 + 97)],
            "89.5초 → 90초"
        );
        assert_eq!(b.reject_entries(at_ms(189_999, T0)), vec![entry(0, T0 + 1)]);
        assert!(b.reject_entries(at(190, T0)).is_empty(), "지난 기한");

        succeed(&mut b, 0, active(0, w(Some(1.0), None), None), at(120, T0));
        assert!(b.reject_entries(at(120, T0)).is_empty(), "성공이 지웠다");

        let mut b = book();
        b.restore_rejects(&[entry(0, i64::MAX)], at(0, T0));
        assert_eq!(
            b.reject_entries(at(0, i64::MAX)),
            vec![entry(0, i64::MAX)],
            "포화"
        );
    }

    #[test]
    fn reject_entries_and_restore_round_trip() {
        let mut first = book();
        fail(&mut first, 0, rate_limited(Some(secs(90))), at(100, T0));
        fail(&mut first, 1, rate_limited(Some(REJECT_MAX)), at(100, T0));
        let saved = first.reject_entries(at(100, T0));
        assert_eq!(saved.len(), 2);

        let mut second = book();
        assert_eq!(second.restore_rejects(&saved, at(5, T0)), 2);
        assert_eq!(second.reject_entries(at(5, T0)), saved);
        assert_eq!(
            judge(&mut second, 0, RequestKind::Refresh, at(94, T0)),
            Judgment::Rejected
        );
        assert_eq!(
            judge(&mut second, 0, RequestKind::Refresh, at(95, T0)),
            Judgment::Start
        );
    }

    // ── 줍기 합침(§3 #60) ──

    /// 5시간 창 % 만 바꾸는 줍기 + 합침 — D12 초기화가 안 서게 한 창만 싣는다.
    fn passive_pct(b: &mut UsageBook, pct: f64, now: Now) -> Option<Coalesce> {
        assert!(
            b.apply_passive(&passive(0, w(Some(pct), None), None), now)
                .changed
        );
        b.coalesce_passive(&key(0), now)
    }

    #[test]
    fn passive_changes_within_a_second_publish_now_once_and_once_at_the_deadline() {
        let mut b = settled();
        let first = at_ms(200_000, T0);
        let now_sheet = passive_pct(&mut b, 50.0, first);
        assert_eq!(now_sheet, Some(Coalesce::PublishNow(snap(&b, 0, first))));
        for (pct, t) in [(51.0, 200_300), (52.0, 200_600)] {
            assert_eq!(
                passive_pct(&mut b, pct, at_ms(t, T0)),
                Some(Coalesce::Deferred),
                "{t}"
            );
        }

        let plan = tick(&mut b, &[0], at_ms(200_600, T0));
        assert!(plan.publish.is_empty());
        assert_eq!(plan.sleep, Duration::from_millis(400), "합침 기한에 깬다");

        let deadline = at_ms(201_000, T0);
        let plan = tick(&mut b, &[0], deadline);
        assert_eq!(
            plan.publish,
            vec![(key(0), snap(&b, 0, deadline))],
            "끝 발행 한 장"
        );
        assert_eq!(pct(&plan.publish[0].1.five_hour), Some(52.0), "마지막 값");
        assert!(plan.start.is_empty());
    }

    #[test]
    fn passive_changes_a_second_or_more_apart_publish_each() {
        let mut b = settled();
        for (n, t) in [200_000, 201_000, 202_500].into_iter().enumerate() {
            assert!(
                matches!(
                    passive_pct(&mut b, 50.0 + n as f64, at_ms(t, T0)),
                    Some(Coalesce::PublishNow(_))
                ),
                "{t}"
            );
        }
        assert!(tick(&mut b, &[0], at_ms(202_600, T0)).publish.is_empty());
    }

    #[test]
    fn any_other_broadcast_before_the_deadline_carries_and_clears_the_pending_change() {
        let mut b = settled();
        passive_pct(&mut b, 50.0, at_ms(200_000, T0));
        assert_eq!(
            passive_pct(&mut b, 51.0, at_ms(200_300, T0)),
            Some(Coalesce::Deferred)
        );
        // 조회 끝 발행이 먼저 나간다 — 그 한 장이 미뤄 둔 값을 싣는다.
        let sheet = b
            .broadcast_sheet(&key(0), at_ms(200_500, T0))
            .expect("한 장");
        assert_eq!(pct(&sheet.five_hour), Some(51.0));
        let plan = tick(&mut b, &[0], at_ms(201_000, T0));
        assert!(plan.publish.is_empty(), "{plan:?}");
        assert_eq!(
            plan.sleep, SCHEDULE_MAX_SLEEP,
            "대기가 없으니 합침 기한도 없다"
        );

        // 창은 그 발행부터 다시 잰다.
        assert_eq!(
            passive_pct(&mut b, 52.0, at_ms(201_200, T0)),
            Some(Coalesce::Deferred)
        );
        assert_eq!(
            tick(&mut b, &[0], at_ms(201_200, T0)).sleep,
            Duration::from_millis(300)
        );

        // 늦게 읽은 시각은 직전 발행을 되감지 않는다.
        b.broadcast_sheet(&key(0), at_ms(203_000, T0));
        b.broadcast_sheet(&key(0), at_ms(202_000, T0));
        assert_eq!(
            passive_pct(&mut b, 53.0, at_ms(203_500, T0)),
            Some(Coalesce::Deferred)
        );

        let stranger = UsageKey {
            vendor: UsageVendorKey::new("no-such-vendor"),
            account: UsageAccountKey::default(),
        };
        assert_eq!(b.coalesce_passive(&stranger, at(0, T0)), None);
        assert_eq!(b.broadcast_sheet(&stranger, at(0, T0)), None);
    }

    #[test]
    fn broadcast_sheet_evaluates_marks_and_snapshots_in_one_call() {
        let mut b = settled();
        let rev = b.revision(&key(0)).expect("키");
        let past_reset = at(200, T0 + 5 * H);
        let sheet = b.broadcast_sheet(&key(0), past_reset).expect("한 장");
        assert!(expired(&sheet.five_hour), "래치를 새긴 뒤 뜬다");
        assert_eq!(sheet.revision, rev + 1);
        assert_eq!(sheet, snap(&b, 0, past_reset));
        let unchanged = b.broadcast_sheet(&key(0), past_reset).expect("한 장");
        assert_eq!(
            unchanged.revision,
            rev + 1,
            "바뀐 게 없으면 revision 그대로"
        );

        // 표시도 했다 — 1초 안의 줍기는 미뤄진다.
        assert_eq!(
            passive_pct(&mut b, 60.0, at_ms(200_400, T0 + 5 * H)),
            Some(Coalesce::Deferred)
        );
        // 미뤄 둔 값을 싣고 갚는다 — 그 뒤 빚이 없으면 합침 입구도 보낼 것이 없다.
        let sheet = b
            .broadcast_sheet(&key(0), at_ms(200_500, T0 + 5 * H))
            .expect("한 장");
        assert_eq!(pct(&sheet.five_hour), Some(60.0));
        assert_eq!(
            b.coalesce_passive(&key(0), at_ms(200_600, T0 + 5 * H)),
            Some(Coalesce::Deferred)
        );
        assert!(tick(&mut b, &[0], at_ms(202_000, T0 + 5 * H))
            .publish
            .is_empty());
    }

    #[test]
    fn a_value_that_ends_a_visible_rejection_is_coalesced_unless_it_latches() {
        for latches in [false, true] {
            let mut b = settled();
            fail(&mut b, 0, rate_limited(Some(REJECT_MAX)), at(110, T0));
            b.broadcast_sheet(&key(0), at(200, T0));
            // 5시간 창의 리셋(T0 + 5H)을 지난 벽시계면 같은 호출이 래치를 새긴다.
            let wall = if latches { T0 + 5 * H } else { T0 };
            let now = at_ms(200_300, wall);
            let applied = b.apply_passive(&passive(0, None, w(Some(11.0), None)), now);
            assert!(applied.changed, "latches {latches}");
            let coalesced = b.coalesce_passive(&key(0), now);
            if latches {
                match coalesced {
                    Some(Coalesce::PublishNow(sheet)) => {
                        assert_eq!(sheet.state, UsageVendorState::Ready);
                        assert!(expired(&sheet.five_hour));
                    }
                    other => panic!("래치는 합치지 않는다: {other:?}"),
                }
            } else {
                assert_eq!(
                    coalesced,
                    Some(Coalesce::Deferred),
                    "상태가 바뀌어도 값의 바뀜이다"
                );
                let plan = tick(&mut b, &[0], at_ms(201_000, wall));
                assert_eq!(plan.publish.len(), 1, "합침 기한에 한 장");
                assert_eq!(plan.publish[0].1.state, UsageVendorState::Ready);
            }
        }
    }

    #[test]
    fn a_passive_that_latches_is_owed_at_once_even_inside_the_window() {
        let mut b = settled();
        passive_pct(&mut b, 50.0, at(200, T0));
        // 값만 바뀐 줍기는 합친다.
        let calm = at_ms(200_300, T0);
        assert!(
            b.apply_passive(&passive(0, None, w(Some(11.0), None)), calm)
                .changed
        );
        assert_eq!(b.coalesce_passive(&key(0), calm), Some(Coalesce::Deferred));
        // 같은 호출이 5시간 창의 리셋을 지나 래치를 새기면 곧바로 갚는다.
        let latched = at_ms(200_600, T0 + 5 * H);
        assert!(
            b.apply_passive(&passive(0, None, w(Some(12.0), None)), latched)
                .changed
        );
        let Some(Coalesce::PublishNow(sheet)) = b.coalesce_passive(&key(0), latched) else {
            panic!("래치는 합치지 않는다");
        };
        assert!(expired(&sheet.five_hour));
        assert_eq!(pct(&sheet.weekly), Some(12.0));
        assert_eq!(sheet, snap(&b, 0, latched));
        assert!(tick(&mut b, &[0], at_ms(201_000, T0 + 5 * H))
            .publish
            .is_empty());
    }

    #[test]
    fn a_bump_the_service_did_not_broadcast_goes_out_on_the_next_tick() {
        let mut b = book();
        assert!(b.begin_probe(&key(0)));
        let plan = tick(&mut b, &[0], at(0, T0));
        assert!(plan.start.is_empty());
        assert_eq!(plan.publish, vec![(key(0), snap(&b, 0, at(0, T0)))]);
        assert!(plan.publish[0].1.in_flight, "조회 시작의 빚");

        b.finish_probe(&key(0), Ok(active(0, w(Some(5.0), None), None)), at(5, T0));
        let plan = tick(&mut b, &[0], at(5, T0));
        assert_eq!(plan.publish, vec![(key(0), snap(&b, 0, at(5, T0)))]);
        assert!(!plan.publish[0].1.in_flight, "조회 끝의 빚");

        let start = at(40, T0);
        assert_eq!(
            judge(&mut b, 0, RequestKind::Refresh, start),
            Judgment::Start
        );
        let plan = tick(&mut b, &[0], start);
        assert_eq!(
            plan.publish,
            vec![(key(0), snap(&b, 0, start))],
            "판정 시작의 빚"
        );

        // 갚은 빚은 다시 안 나간다.
        b.broadcast_sheet(&key(0), at(41, T0));
        assert!(tick(&mut b, &[0], at(41, T0)).publish.is_empty());

        // 복원도 빚을 진다.
        let mut b = book();
        b.restore_rejects(&[entry(1, T0 + 90)], at(10, T0));
        let plan = tick(&mut b, &[1], at(10, T0));
        assert!(plan.start.is_empty());
        assert_eq!(plan.publish, vec![(key(1), snap(&b, 1, at(10, T0)))]);
    }

    #[test]
    fn a_first_sheet_latch_is_still_owed_to_every_subscriber() {
        let mut b = settled();
        tick(&mut b, &[0], at(100, T0));
        // 구독 추가의 첫 한 장 = eval_time + snapshot — 그 연결에만 가므로 빚을 안 갚는다.
        let now = at(200, T0 + 5 * H);
        assert!(b.eval_time(&key(0), now));
        let first_sheet = snap(&b, 0, now);
        let plan = tick(&mut b, &[0], now);
        assert_eq!(
            plan.publish,
            vec![(key(0), first_sheet)],
            "전부에게 나간다 — 새 연결엔 한 장이 겹친다"
        );
    }

    #[test]
    fn published_sheets_carry_their_keys_and_unsubscribed_debts_wait() {
        let mut b = book();
        assert!(b.begin_probe(&key(0)));
        assert!(b.begin_probe(&key(1)));
        let plan = tick(&mut b, &[0], at(0, T0));
        assert_eq!(plan.publish, vec![(key(0), snap(&b, 0, at(0, T0)))]);
        let plan = tick(&mut b, &[1, 0], at(1, T0));
        assert_eq!(
            plan.publish,
            vec![(key(1), snap(&b, 1, at(1, T0)))],
            "구독 밖에서 기다린 빚"
        );

        let mut b = book();
        assert!(b.begin_probe(&key(0)));
        assert!(b.begin_probe(&key(1)));
        let plan = tick(&mut b, &[1, 0], at(0, T0));
        assert_eq!(
            plan.publish,
            vec![
                (key(0), snap(&b, 0, at(0, T0))),
                (key(1), snap(&b, 1, at(0, T0)))
            ],
            "칸을 만든 순서"
        );
    }

    #[test]
    fn a_sub_second_coalesce_deadline_does_not_spin() {
        let mut b = settled();
        passive_pct(&mut b, 50.0, at_ms(200_000, T0));
        passive_pct(&mut b, 51.0, at_ms(200_999, T0));
        let plan = tick(&mut b, &[0], at_ms(200_999, T0));
        assert_eq!(plan.sleep, Duration::from_millis(1));
        let plan = tick(&mut b, &[0], at_ms(201_000, T0));
        assert_eq!(plan.publish.len(), 1);
        assert_eq!(plan.sleep, SCHEDULE_MAX_SLEEP);
    }

    #[test]
    fn a_scheduled_start_carries_and_clears_a_pending_change() {
        let mut b = book();
        passive_pct(&mut b, 50.0, at(0, T0));
        assert_eq!(
            passive_pct(&mut b, 51.0, at_ms(300, T0)),
            Some(Coalesce::Deferred)
        );
        let now = at_ms(500, T0);
        let plan = tick(&mut b, &[0], now);
        assert_eq!(plan.start, vec![key(0)], "기준점이 없어 지금 기한");
        assert_eq!(plan.publish, vec![(key(0), snap(&b, 0, now))]);
        assert!(plan.publish[0].1.in_flight);
        assert_eq!(pct(&plan.publish[0].1.five_hour), Some(51.0));
        assert_eq!(
            plan.sleep, SCHEDULE_MAX_SLEEP,
            "옛 합침 기한은 깰 때가 아니다"
        );
        assert!(tick(&mut b, &[0], at(1, T0)).publish.is_empty());
    }

    #[test]
    fn a_pending_change_on_a_probe_in_flight_goes_out_at_its_deadline() {
        let mut b = settled();
        let start = at(200, T0);
        assert_eq!(
            judge(&mut b, 0, RequestKind::Refresh, start),
            Judgment::Start
        );
        assert!(b.broadcast_sheet(&key(0), start).expect("한 장").in_flight);
        assert_eq!(
            passive_pct(&mut b, 55.0, at_ms(200_300, T0)),
            Some(Coalesce::Deferred)
        );
        assert_eq!(
            tick(&mut b, &[0], at_ms(200_300, T0)).sleep,
            Duration::from_millis(700),
            "진행 중이라 자동 기한은 빠진다"
        );
        let deadline = at(201, T0);
        let plan = tick(&mut b, &[0], deadline);
        assert!(plan.start.is_empty());
        assert_eq!(plan.publish, vec![(key(0), snap(&b, 0, deadline))]);
        assert!(plan.publish[0].1.in_flight);
        assert_eq!(pct(&plan.publish[0].1.five_hour), Some(55.0));
    }

    // ── 스케줄러(`plan_tick`) ──

    #[test]
    fn a_due_subscribed_vendor_starts_one_probe_and_requests_join_it() {
        let mut b = book();
        let plan = tick(&mut b, &[0], at(0, T0));
        assert_eq!(plan.start, vec![key(0)]);
        assert_eq!(
            plan.publish,
            vec![(key(0), snap(&b, 0, at(0, T0)))],
            "조회 시작은 발행한다"
        );
        assert!(plan.publish[0].1.in_flight);
        assert_eq!(
            (b.in_flight(&key(0)), b.revision(&key(0))),
            (Some(true), Some(1))
        );
        assert_eq!(
            judge(&mut b, 0, RequestKind::Get, at(0, T0)),
            Judgment::Join
        );
        assert_eq!(
            judge(&mut b, 0, RequestKind::Refresh, at(0, T0)),
            Judgment::Join
        );

        // 요청이 먼저 시작했으면 스케줄러가 다시 시작하지 않는다.
        let mut b = book();
        assert_eq!(
            judge(&mut b, 0, RequestKind::Get, at(0, T0)),
            Judgment::Start
        );
        assert!(tick(&mut b, &[0], at(0, T0)).start.is_empty());
    }

    #[test]
    fn a_probe_in_flight_is_not_restarted_and_its_deadline_is_not_a_wake() {
        let near = Now {
            mono: secs(100) + cooldown(0) - secs(10),
            wall: T0,
        };
        let mut b = settled();
        assert_eq!(tick(&mut b, &[0], near).sleep, secs(10));

        let mut b = settled();
        assert!(b.begin_probe(&key(0)));
        assert_eq!(tick(&mut b, &[0], near).sleep, SCHEDULE_MAX_SLEEP);
        let past_due = Now {
            mono: near.mono + secs(20),
            wall: T0,
        };
        let plan = tick(&mut b, &[0], past_due);
        assert!(plan.start.is_empty());
        assert_eq!(plan.sleep, SCHEDULE_MAX_SLEEP);

        // 조회 끝 → 다음 기한으로 깬다.
        b.finish_probe(&key(0), Ok(active(0, None, None)), past_due);
        let almost = Now {
            mono: past_due.mono + cooldown(0) - secs(7),
            wall: T0,
        };
        assert_eq!(tick(&mut b, &[0], almost).sleep, secs(7));
    }

    #[test]
    fn a_reset_wakes_the_scheduler_and_its_latch_is_published() {
        let mut b = settled();
        let r = T0 + 30;
        succeed(
            &mut b,
            0,
            active(0, w(Some(40.0), Some(r)), None),
            at(100, T0),
        );
        let plan = tick(&mut b, &[0], at(100, T0));
        assert_eq!(plan.sleep, secs(30), "벽시계 차");
        let rev = b.revision(&key(0)).expect("키");

        let plan = tick(&mut b, &[0], at(131, r));
        assert_eq!(plan.publish, vec![(key(0), snap(&b, 0, at(131, r)))]);
        assert!(plan.start.is_empty());
        assert_eq!(plan.publish[0].1.revision, rev + 1);
        assert!(expired(&plan.publish[0].1.five_hour));
    }

    #[test]
    fn a_rejection_end_wakes_the_scheduler_and_its_fold_is_published() {
        let mut b = settled();
        fail(&mut b, 0, rate_limited(Some(secs(20))), at(100, T0));
        assert_eq!(tick(&mut b, &[0], at(100, T0)).sleep, secs(20));
        let plan = tick(&mut b, &[0], at(120, T0));
        assert_eq!(plan.publish, vec![(key(0), snap(&b, 0, at(120, T0)))]);
        assert!(plan.start.is_empty(), "거절 끝은 조회를 앞당기지 않는다");
        assert_eq!(tag(&plan.publish[0].1.state), "Failed");
    }

    #[test]
    fn no_subscribed_vendor_means_no_probe_and_the_longest_sleep() {
        assert_eq!(
            TickPlan::default().sleep,
            SCHEDULE_MAX_SLEEP,
            "기본값도 0 이 아니다"
        );
        let mut b = settled();
        passive_pct(&mut b, 50.0, at(200, T0));
        assert_eq!(
            passive_pct(&mut b, 51.0, at_ms(200_300, T0)),
            Some(Coalesce::Deferred)
        );
        let rev = (b.revision(&key(0)), b.revision(&key(1)));
        let plan = tick(&mut b, &[], at(201, T0 + 200 * H));
        assert_eq!(plan, TickPlan::default());
        assert_eq!(
            (b.revision(&key(0)), b.revision(&key(1))),
            rev,
            "래치도 안 본다"
        );
        assert_eq!(b.in_flight(&key(1)), Some(false), "기한이 지난 칸도 조회 0");

        // 구독 밖이어도 대기는 남는다 — 구독 사본이 낡았을 수 있다. 구독된 뒤 한 장이 나간다.
        let later = at(202, T0);
        let plan = tick(&mut b, &[0], later);
        assert_eq!(plan.publish, vec![(key(0), snap(&b, 0, later))]);
        assert_eq!(pct(&plan.publish[0].1.five_hour), Some(51.0));
    }

    #[test]
    fn an_unsubscribed_due_vendor_is_not_probed() {
        let mut b = book();
        let plan = tick(&mut b, &[1, 1], at(0, T0));
        assert_eq!(plan.start, vec![key(1)], "중복은 한 번");
        assert_eq!(b.in_flight(&key(0)), Some(false));

        let odd = UsageVendorKey::new("no-such-vendor");
        let plan = b.plan_tick(&[odd], at(1, T0));
        assert_eq!(plan.start, Vec::<UsageKey>::new());
    }

    #[test]
    fn subscribing_late_starts_a_past_deadline_at_once() {
        let mut b = settled();
        let late = Now {
            mono: secs(100) + cooldown(0) + secs(500),
            wall: T0,
        };
        assert!(tick(&mut b, &[], late).start.is_empty());
        assert_eq!(tick(&mut b, &[0], late).start, vec![key(0)]);
    }

    #[test]
    fn a_three_hour_mono_jump_is_one_scheduled_probe_not_one_per_interval() {
        let mut b = settled();
        let jumped = at(100 + 3 * 3_600, T0);
        assert_eq!(tick(&mut b, &[0], jumped).start, vec![key(0)]);
        let done = at(100 + 3 * 3_600 + 5, T0);
        b.finish_probe(&key(0), Ok(active(0, None, None)), done);
        assert!(tick(&mut b, &[0], done).start.is_empty());
    }

    #[test]
    fn extreme_times_do_not_panic_and_sleep_stays_bounded() {
        const YEAR_9999: i64 = 253_402_300_799;
        let mut b = book();
        let pre_1970 = at(10, -1_000_000);
        succeed(
            &mut b,
            0,
            active(0, w(Some(1.0), Some(YEAR_9999)), w(None, Some(i64::MAX))),
            pre_1970,
        );
        let plan = tick(&mut b, &[0, 1], pre_1970);
        assert_eq!(plan.start, vec![key(1)]);
        tick(&mut b, &[0, 1], at(20, i64::MIN));
        tick(&mut b, &[0, 1], at(30, i64::MAX));

        for mono in [Duration::MAX - Duration::from_nanos(1), Duration::MAX] {
            let edge = Now { mono, wall: T0 };
            let mut b = book();
            succeed(&mut b, 0, active(0, w(Some(1.0), Some(T0 + H)), None), edge);
            b.coalesce_passive(&key(0), edge);
            b.coalesce_passive(&key(0), edge);
            fail(&mut b, 1, rate_limited(Some(REJECT_MAX)), edge);
            tick(&mut b, &[0, 1], edge);
            b.restore_rejects(&[entry(0, T0 + 10)], edge);
            tick(&mut b, &[0, 1], edge);
            b.reject_entries(edge);
            judge(&mut b, 0, RequestKind::Refresh, edge);
            judge(&mut b, 1, RequestKind::Get, edge);
        }
    }
}
