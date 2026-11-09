//! 실행 전처리 — 스폰 예약 · 「이미 떠 있음」 확인 뒤, 화신 표식 · 세션 파생 앞에서 **명부의** 프로필로 실행 재료를
//! 점검한다(`spawn_agent_watching_link` 단독 호출). 명부에 없는 id 는 넣기 전에도 호출자 사본으로 점검한다
//! (`ensure_registered` 단독 호출). 재료 준비의 실패를 스폰 실패로 옮기는 것(`prepare_failed`)도 여기다 — 준비를
//! 부르는 자리는 스폰 길의 화신 표식 뒤다.
//!
//! 점검([`crate::launch::LaunchKitSource::check`])은 명부 · 예약 · 세션 · 이름 배정 락을 하나도 쥐지 않고 부른다 —
//! asset-library 락 → manager 락 한 방향이라, 쥔 채 부르면 라이브러리 쓰기와 교착한다. 명부에 쓰지 않고 비교만
//! 한다(무인 기본값 채움은 다음 주제 — M16): 점검하는 사이 실행 입력(붙임 · 명령 · env)이 바뀌었으면 막힘이든
//! 통과든 그 답을 버리고 한 번 다시 하고, 또 바뀌었으면 끊는다.
// ADR-0319
// ADR-0324
// ADR-0334
// TRD S25 §1-5 (가)

use super::{profile_vanished_mid_spawn, AgentManager};
use crate::launch::{notice_fingerprint, LaunchBlocked, LaunchPlan, Notice, PrepareError};
use crate::profile::{AgentProfile, LaunchInputs};
use crate::types::{AgentId, PtyError};

/// 점검 → 대조를 몇 번까지 하나 — 처음 한 번 + 실행 입력이 바뀌었을 때 다시 한 번.
// TRD S25 §1-5 (가) 2
const PRELAUNCH_ATTEMPTS: u32 = 2;

impl AgentManager {
    /// 명부에 없는 프로필을 넣기 **전에** 호출자 사본으로 점검한다 — 막히면 아무것도 넣지 않는다. 통과한 계획은
    /// 버린다(넣은 뒤 [`Self::prelaunch`] 가 명부 값으로 다시 점검한다).
    // TRD S25 §1-5 (가) 1 ②
    pub(super) fn check_before_create(&self, profile: &AgentProfile) -> Result<(), PtyError> {
        match self.launch.kits.check(profile) {
            Ok(_) => Ok(()),
            Err(blocked) => Err(launch_blocked(profile.id, blocked)),
        }
    }

    /// 이 에이전트를 띄울 계획 — 명부에서 읽은 프로필을 점검하고, 그 뒤 실행 입력이 그대로일 때만 그 답을 낸다.
    ///
    /// `caller` = 스폰 호출자가 넘긴 사본 — 점검하지 않는다. 있던 id 면 스폰이 이 뒤에 사본의 명령 · env 를 명부에
    /// 병합하므로, 그 둘이 점검한 값과 다르면 점검하지 않은 명령으로 뜨지 않게 끊는다.
    /// `Err` = 막힘([`PtyError::LaunchBlocked`]) · 실행 입력이 두 번 다 바뀜 또는 사본의 명령 · env 가 점검한 값과
    /// 다름([`PtyError::LaunchRaced`]) · 그 사이 지워짐(`SpawnFailed`). 어느 쪽이든 명부 · 세션 · 제어 채널은
    /// 그대로다.
    /// With the plan comes its notice comparison against the fingerprint on the same roster read ([`PlanNotices`]) —
    ///   compared here, recorded by nobody here.
    // ADR-0341
    pub(super) fn prelaunch(
        &self,
        caller: &AgentProfile,
    ) -> Result<(LaunchPlan, PlanNotices), PtyError> {
        let id = caller.id;
        for attempt in 1..=PRELAUNCH_ATTEMPTS {
            // 호출자 사본이 아니라 명부 값 — 사본을 뜬 뒤 붙인 프리셋까지 본다. `get` 은 사본을 뜨고 락을 놓는다.
            let profile = self
                .profiles
                .get(id)
                .ok_or_else(|| profile_vanished_mid_spawn(id, "실행 전처리 읽기"))?;
            let seen = LaunchInputs::of(&profile);
            // ★답을 쥔 채 대조부터 한다 — 막힘도 대조가 맞을 때만 답이다★: 점검하는 사이 바뀐 붙임으로 막혔으면 그
            //   막힘은 이미 낡았다(고친 붙임이 낡은 막힘에 걸려 끊긴다).
            let checked = self.launch.kits.check(&profile);
            match self.profiles.launch_inputs_unchanged(id, &seen) {
                Some(true) => {
                    return settle(caller, &seen, checked).map(|plan| {
                        let notices = PlanNotices::against(&plan, &profile);
                        (plan, notices)
                    })
                }
                Some(false) => tracing::debug!(
                    agent = %id,
                    attempt,
                    "점검하는 사이 실행 입력(붙임 · 명령 · env)이 바뀌었다 — 명부를 다시 읽는다"
                ),
                None => return Err(profile_vanished_mid_spawn(id, "실행 전처리 대조")),
            }
        }
        tracing::info!(
            agent = %id,
            "실행 준비 중 실행 입력(프리셋 · 명령 · 환경)이 계속 바뀌었다 — 띄우지 않는다(다시 시도하면 선다)"
        );
        Err(PtyError::LaunchRaced)
    }
}

/// 대조가 맞은 점검의 답을 낸다 — 사본의 명령 · env 가 점검한 값과 다르면 답과 무관하게 경합이다.
fn settle(
    caller: &AgentProfile,
    seen: &LaunchInputs,
    checked: Result<LaunchPlan, LaunchBlocked>,
) -> Result<LaunchPlan, PtyError> {
    let id = caller.id;
    // ★병합해 덮지 않고 끊는다★: 오늘 명령 · env 를 바꾸는 운영 길은 없어 사본과 명부가 갈리지 않는다 — 갈렸으면 사본을
    //   뜬 뒤 그 둘이 바뀐 것이고, 다시 시도하면 새 사본으로 선다.
    if !seen.same_command_and_env(caller) {
        tracing::info!(
            agent = %id,
            "호출자 사본의 명령 · env 가 점검한 명부 값과 다르다 — 띄우지 않는다(다시 시도하면 선다)"
        );
        return Err(PtyError::LaunchRaced);
    }
    let plan = checked.map_err(|blocked| launch_blocked(id, blocked))?;
    // Every launch logs its notices, whether or not a reply shows them (ADR-0341).
    for notice in &plan.notices {
        tracing::info!(
            agent = %id,
            code = %notice.code,
            args = %notice.args,
            line = ?notice.line,
            "실행 알림"
        );
    }
    Ok(plan)
}

/// A plan's notices against what the agent was last shown — notices once per agent.
// ADR-0341
// TRD S25 §1-5 「Notices once per agent」
pub(super) struct PlanNotices {
    /// The plan's omission identity ([`notice_fingerprint`]) — what a delivering caller records.
    pub(super) fingerprint: String,
    /// For the success reply: the plan's full list, or empty when the stored fingerprint equals `fingerprint`.
    pub(super) shown: Vec<Notice>,
    /// The stored fingerprint `shown` was decided against — the record is a compare-and-set on it.
    pub(super) baseline: Option<String>,
}

impl PlanNotices {
    /// `roster` = the roster copy this plan was checked from — its fingerprint is the one compared.
    fn against(plan: &LaunchPlan, roster: &AgentProfile) -> Self {
        let fingerprint = notice_fingerprint(&plan.notices);
        let baseline = roster.notice_fingerprint.clone();
        let shown = if baseline.as_deref() == Some(fingerprint.as_str()) {
            Vec::new()
        } else {
            plan.notices.clone()
        };
        Self {
            fingerprint,
            shown,
            baseline,
        }
    }
}

fn launch_blocked(id: AgentId, blocked: LaunchBlocked) -> PtyError {
    tracing::info!(agent = %id, %blocked, "실행 재료 점검이 실행을 막았다 — 띄우지 않는다");
    PtyError::LaunchBlocked(blocked)
}

/// 재료 준비([`crate::launch::LaunchKitSource::prepare`])의 실패를 스폰 실패로 옮긴다 — 원본이 점검 뒤 바뀐 것은
/// 경합([`PtyError::LaunchRaced`] — 프로세스를 열려고 한 적이 없다), 그 밖은 `SpawnFailed`.
// TRD S25 §5-2 4 (c)
pub(super) fn prepare_failed(id: AgentId, e: PrepareError) -> PtyError {
    match &e {
        PrepareError::Stale(_) => {
            tracing::info!(
                agent = %id,
                %e,
                "점검 뒤 실행 재료 원본이 바뀌었다 — 띄우지 않는다(다시 시도하면 선다)"
            );
            PtyError::LaunchRaced
        }
        PrepareError::Failed(_) => PtyError::SpawnFailed(format!("{e} (agent {id})")),
    }
}

#[cfg(test)]
impl AgentManager {
    /// 스폰이 조립한 명령과 거기 실은 재료를 시험이 받아 간다 — 받는 쪽이 있으면 프로세스를 띄우기 전에 멈춘다.
    pub(super) fn probe_spec(
        &self,
        spec: &crate::types::CommandSpec,
        artifacts: &crate::launch::LaunchArtifacts,
    ) -> Result<(), PtyError> {
        match engram_dashboard_base::sync::lock(&self.spec_probe).as_ref() {
            Some(tx) => {
                let _ = tx.send((spec.clone(), artifacts.clone()));
                Err(PtyError::SpawnFailed(
                    "시험 — 명령만 조립하고 띄우지 않았다".to_string(),
                ))
            }
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet, VecDeque};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::{channel, Receiver};
    use std::sync::{Arc, Mutex, OnceLock, RwLock};
    use std::time::Duration;

    use engram_dashboard_base::sync;
    use engram_dashboard_platform::shell::console_command;
    use serde::Deserialize;
    use serde_json::json;
    use uuid::Uuid;

    use super::super::tests::{put_live_session, put_live_session_leased};
    use super::super::{AgentManager, NoticeDelivery, SpawnOutcome, SpawnReservation};
    use super::PlanNotices;
    use crate::backend;
    use crate::failure::AgentFailureKind;
    use crate::launch::{
        BlockReason, LaunchArtifacts, LaunchBlocked, LaunchFs, LaunchKit, LaunchKitSource,
        LaunchLease, LaunchPlan, LaunchPorts, MemLaunchFs, NoopLaunchKits, PrepareError, Reason,
    };
    use crate::profile::scripted_store::ScriptedStore;
    use crate::profile::{
        AgentCommand, AgentOutputFormat, AgentProfile, AttachedPreset, ProfileRegistry,
        RestoreOutcome, SpawnMode,
    };
    use crate::profile_edit::Attachments;
    use crate::reaper::ReaperDeps;
    use crate::session::AgentSession;
    use crate::session_tracker::{SessionTracker, TrackerConfig};
    use crate::types::{
        AgentId, AgentInfo, AgentStatus, CommandSpec, ControlChannel, ControlChannelNeeds,
        ControlEndpoint, ProvisionError, PtyError, StatusSink,
    };

    // 골든(`tests/launch_argv_golden.rs`)과 같은 고정값 — 그쪽이 바뀌면 아래 대조가 골든에서 경우를 못 찾아 빨개진다.
    const WORK: &str = "/golden/work";
    const FRESH_SID: Uuid = Uuid::from_u128(0x1111_1111_1111_4111_8111_1111_1111_1111);
    const STORED_SID: Uuid = Uuid::from_u128(0x2222_2222_2222_4222_8222_2222_2222_2222);

    // ── 시험대 ──

    struct QuietStatus;
    impl StatusSink for QuietStatus {
        fn status_changed(&self, _id: AgentId, _s: AgentStatus, _e: u32) {}
        fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
    }

    /// 발급 요청을 센다 — 답은 「제어 채널 없음」(`NoopControlChannel` 과 같다).
    #[derive(Default)]
    struct CountingControl {
        provisions: AtomicUsize,
    }

    impl ControlChannel for CountingControl {
        fn provision(
            &self,
            _id: AgentId,
            _epoch: u32,
            _needs: ControlChannelNeeds,
        ) -> Result<Option<ControlEndpoint>, ProvisionError> {
            self.provisions.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        }
        fn revoke(&self, _id: AgentId, _epoch: u32) {}
    }

    /// 부른 배치 포트 호출을 센다(쓰기 · 확인 모두) — 메모리에만 쓴다.
    #[derive(Default)]
    struct CountingFs {
        calls: AtomicUsize,
        mem: MemLaunchFs,
    }

    impl LaunchFs for CountingFs {
        fn write_file(&self, path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.mem.write_file(path, bytes)
        }
        fn link_dir(
            &self,
            link: &std::path::Path,
            target: &std::path::Path,
        ) -> std::io::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.mem.link_dir(link, target)
        }
        fn exists(&self, path: &std::path::Path) -> bool {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.mem.exists(path)
        }
        fn is_link(&self, path: &std::path::Path) -> std::io::Result<bool> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.mem.is_link(path)
        }
    }

    /// 메모리 저장소 · 셀 수 있는 제어 채널 · 셀 수 있는 배치 포트 · 조립한 명령을 받아 가는 탐침 — 프로세스는 하나도
    /// 띄우지 않는다.
    struct Rig {
        manager: AgentManager,
        store: Arc<ScriptedStore>,
        control: Arc<CountingControl>,
        fs: Arc<CountingFs>,
        specs: Receiver<(CommandSpec, LaunchArtifacts)>,
    }

    impl Rig {
        fn new(kits: Arc<dyn LaunchKitSource>) -> Self {
            let store = ScriptedStore::new();
            let profiles = Arc::new(ProfileRegistry::new(store.clone()));
            let tracker = Arc::new(SessionTracker::new(
                TrackerConfig {
                    enabled: false,
                    poll_interval: Duration::from_secs(1),
                },
                Arc::new(|_, _| {}),
            ));
            let control = Arc::new(CountingControl::default());
            let fs = Arc::new(CountingFs::default());
            let manager = AgentManager::new_with_control(
                Arc::new(QuietStatus),
                profiles,
                tracker,
                control.clone(),
            )
            .with_launch_ports(LaunchPorts {
                kits,
                fs: fs.clone(),
            });
            let (tx, specs) = channel();
            *sync::lock(&manager.spec_probe) = Some(tx);
            Self {
                manager,
                store,
                control,
                fs,
                specs,
            }
        }

        /// 배치 포트를 부른 수 — 쓰기 · 확인 모두.
        fn fs_calls(&self) -> usize {
            self.fs.calls.load(Ordering::SeqCst)
        }

        fn noop() -> Self {
            Self::new(Arc::new(NoopLaunchKits))
        }

        /// 이 스폰이 조립한 명령 — 없으면 스폰이 조립까지 가지 않았다.
        fn spec(&self) -> CommandSpec {
            self.assembled().0
        }

        /// 이 스폰이 조립한 명령과 거기 실은 재료.
        fn assembled(&self) -> (CommandSpec, LaunchArtifacts) {
            self.specs
                .try_recv()
                .expect("스폰이 명령을 조립하지 않았다")
        }

        fn assembled_nothing(&self) -> bool {
            self.specs.try_recv().is_err()
        }

        fn provisions(&self) -> usize {
            self.control.provisions.load(Ordering::SeqCst)
        }
    }

    /// 점검 한 번이 본 것.
    #[derive(Debug, Clone)]
    struct Seen {
        profile: AgentProfile,
        roster_free: bool,
        reservations_free: bool,
        sessions_free: bool,
        names_free: bool,
        reserved: bool,
        roster_epoch: Option<u32>,
        live_session: bool,
    }

    type Meanwhile = Box<dyn FnOnce(&ProfileRegistry, AgentId) + Send>;

    /// 점검 한 번마다 할 일 — 줄이 비면 Noop 계획.
    enum Turn {
        Pass,
        /// 이 계획으로 통과한다.
        Plan(LaunchPlan),
        Block(LaunchBlocked),
        /// 점검하는 사이 명부를 바꾼다(다른 연결의 붙이기 · 지우기 흉내) — 그다음 Noop 계획.
        Meanwhile(Meanwhile),
        /// 명부를 바꾸고 막는다 — 바뀌기 전 값으로 낸 막힘.
        MeanwhileBlock(Meanwhile, LaunchBlocked),
    }

    /// 준비 한 번이 본 것.
    #[derive(Debug, Clone)]
    struct Prepared {
        plan: LaunchPlan,
        id: AgentId,
        epoch: u32,
        /// 명부 · 예약 · 세션 · 이름 배정 락이 모두 비었나.
        locks_free: bool,
        reserved: bool,
        roster_epoch: Option<u32>,
        roster_sid: Option<Uuid>,
        /// 그때까지의 제어 채널 발급 요청 수.
        provisions: usize,
        live_session: bool,
    }

    /// 준비 한 번마다 낼 답 — 줄이 비면 받은 계획 그대로의 Noop 재료.
    enum Prep {
        /// 재료의 계획에 실 재료 표시를 켠다 — 점검한 계획은 Noop 이라, 실린 재료가 켜져 있으면 그것은 재료에서 왔다.
        MarkReal,
        Fail(PrepareError),
        /// 이 휘발 폴더를 단 Noop 재료.
        Volatile(PathBuf),
    }

    /// 거두기 한 번이 본 것 — 무엇을 거뒀나는 `released` 가 같은 차례로 든다.
    #[derive(Debug, Clone)]
    struct Release {
        /// 명부 · 예약 · 세션 · 이름 배정 락이 모두 비었나.
        locks_free: bool,
        /// 그 에이전트 세션이 아직 세션 표에 있나.
        live_session: bool,
        /// reaper 스레드가 불렀나.
        on_reaper: bool,
        /// 그때 명부의 `auto_restore` — `None` = 명부 락이 잡혀 있었거나 항목이 없다.
        auto_restore: Option<bool>,
    }

    struct Tables {
        profiles: Arc<ProfileRegistry>,
        spawning: Arc<Mutex<HashSet<AgentId>>>,
        sessions: Arc<RwLock<HashMap<AgentId, Arc<AgentSession>>>>,
        names: Arc<Mutex<()>>,
        control: Arc<CountingControl>,
    }

    /// 매니저의 명부 · 예약 · 세션 표를 들여다보며 점검 · 준비하고, 거두기를 세는 가짜 재료 출처.
    #[derive(Default)]
    struct ScriptedKits {
        turns: Mutex<VecDeque<Turn>>,
        seen: Mutex<Vec<Seen>>,
        preps: Mutex<VecDeque<Prep>>,
        prepared: Mutex<Vec<Prepared>>,
        released: Mutex<Vec<(AgentId, u32)>>,
        /// 표를 건 시험대에서만 찬다.
        releases: Mutex<Vec<Release>>,
        tables: OnceLock<Tables>,
    }

    impl ScriptedKits {
        fn new(turns: Vec<Turn>) -> Arc<Self> {
            Self::preparing(turns, Vec::new())
        }

        fn preparing(turns: Vec<Turn>, preps: Vec<Prep>) -> Arc<Self> {
            Arc::new(Self {
                turns: Mutex::new(turns.into()),
                preps: Mutex::new(preps.into()),
                ..Self::default()
            })
        }

        fn rig(self: &Arc<Self>) -> Rig {
            let rig = Rig::new(self.clone());
            let manager = &rig.manager;
            assert!(self
                .tables
                .set(Tables {
                    profiles: manager.profiles.clone(),
                    spawning: manager.spawning.clone(),
                    sessions: manager.sessions.clone(),
                    names: manager.name_allocation.clone(),
                    control: rig.control.clone(),
                })
                .is_ok());
            rig
        }

        fn seen(&self) -> Vec<Seen> {
            sync::lock(&self.seen).clone()
        }

        fn prepared(&self) -> Vec<Prepared> {
            sync::lock(&self.prepared).clone()
        }

        fn released(&self) -> Vec<(AgentId, u32)> {
            sync::lock(&self.released).clone()
        }

        fn releases(&self) -> Vec<Release> {
            sync::lock(&self.releases).clone()
        }
    }

    impl LaunchKitSource for ScriptedKits {
        fn check(&self, profile: &AgentProfile) -> Result<LaunchPlan, LaunchBlocked> {
            let t = self.tables.get().expect("시험대가 표를 걸었다");
            let id = profile.id;
            // 잡을 수 있나부터 본다 — 매니저가 쥔 채 불렀으면 아래 읽기가 같은 스레드에서 다시 잡아 매달린다.
            let roster_free = t.profiles.lock_is_free();
            let reservations_free = t.spawning.try_lock().is_ok();
            let sessions_free = t.sessions.try_write().is_ok();
            let names_free = t.names.try_lock().is_ok();
            sync::lock(&self.seen).push(Seen {
                profile: profile.clone(),
                roster_free,
                reservations_free,
                sessions_free,
                names_free,
                reserved: reservations_free && sync::lock(&t.spawning).contains(&id),
                roster_epoch: if roster_free {
                    t.profiles.get(id).map(|p| p.epoch)
                } else {
                    None
                },
                live_session: sessions_free && t.sessions.read().unwrap().contains_key(&id),
            });
            let turn = sync::lock(&self.turns).pop_front();
            match turn {
                None | Some(Turn::Pass) => NoopLaunchKits.check(profile),
                Some(Turn::Plan(plan)) => Ok(plan),
                Some(Turn::Block(blocked)) => Err(blocked),
                Some(Turn::Meanwhile(change)) => {
                    if roster_free {
                        change(&t.profiles, id);
                    }
                    NoopLaunchKits.check(profile)
                }
                Some(Turn::MeanwhileBlock(change, blocked)) => {
                    if roster_free {
                        change(&t.profiles, id);
                    }
                    Err(blocked)
                }
            }
        }

        fn prepare(
            &self,
            plan: LaunchPlan,
            id: AgentId,
            epoch: u32,
        ) -> Result<LaunchKit, PrepareError> {
            let t = self.tables.get().expect("시험대가 표를 걸었다");
            let roster_free = t.profiles.lock_is_free();
            let reservations_free = t.spawning.try_lock().is_ok();
            let sessions_free = t.sessions.try_write().is_ok();
            let names_free = t.names.try_lock().is_ok();
            let roster = if roster_free {
                t.profiles.get(id)
            } else {
                None
            };
            sync::lock(&self.prepared).push(Prepared {
                plan: plan.clone(),
                id,
                epoch,
                locks_free: roster_free && reservations_free && sessions_free && names_free,
                reserved: reservations_free && sync::lock(&t.spawning).contains(&id),
                roster_epoch: roster.as_ref().map(|p| p.epoch),
                roster_sid: roster.and_then(|p| p.backend_session_id),
                provisions: t.control.provisions.load(Ordering::SeqCst),
                live_session: sessions_free && t.sessions.read().unwrap().contains_key(&id),
            });
            let prep = sync::lock(&self.preps).pop_front();
            match prep {
                None => NoopLaunchKits.prepare(plan, id, epoch),
                Some(Prep::MarkReal) => {
                    let mut kit = NoopLaunchKits.prepare(plan, id, epoch)?;
                    kit.plan.real = true;
                    Ok(kit)
                }
                Some(Prep::Fail(e)) => Err(e),
                Some(Prep::Volatile(dir)) => {
                    let mut kit = NoopLaunchKits.prepare(plan, id, epoch)?;
                    kit.volatile_dir = Some(dir);
                    Ok(kit)
                }
            }
        }

        fn release(&self, id: AgentId, epoch: u32) {
            sync::lock(&self.released).push((id, epoch));
            let Some(t) = self.tables.get() else {
                return;
            };
            let sessions_free = t.sessions.try_write().is_ok();
            let roster_free = t.profiles.lock_is_free();
            sync::lock(&self.releases).push(Release {
                locks_free: roster_free
                    && t.spawning.try_lock().is_ok()
                    && sessions_free
                    && t.names.try_lock().is_ok(),
                live_session: !sessions_free || t.sessions.read().unwrap().contains_key(&id),
                on_reaper: std::thread::current().name() == Some("engram-reaper"),
                auto_restore: if roster_free {
                    t.profiles.get(id).map(|p| p.auto_restore)
                } else {
                    None
                },
            });
        }
    }

    // ── 픽스처 ──

    fn strings(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    /// 골든의 명령 다섯 — 이름이 골든 경우 이름의 머리다.
    fn golden_commands() -> Vec<(&'static str, AgentCommand)> {
        vec![
            (
                "claude-terminal",
                AgentCommand::Claude {
                    extra_args: strings(&["--model", "golden-model"]),
                    output_format: AgentOutputFormat::Terminal,
                },
            ),
            (
                "claude-json",
                AgentCommand::Claude {
                    extra_args: strings(&["--model", "golden-model"]),
                    output_format: AgentOutputFormat::StreamJson,
                },
            ),
            (
                "codex-terminal",
                AgentCommand::Codex {
                    extra_args: strings(&["-c", "model=golden-model"]),
                    output_format: AgentOutputFormat::Terminal,
                },
            ),
            (
                "codex-json",
                AgentCommand::Codex {
                    extra_args: strings(&["-c", "model=golden-model"]),
                    output_format: AgentOutputFormat::StreamJson,
                },
            ),
            (
                "shell",
                AgentCommand::Shell {
                    program: "golden-shell".to_string(),
                    args: strings(&["--login", "-i"]),
                },
            ),
        ]
    }

    /// 즉석 생성 길의 명령 셋 — 데몬 `SpawnByCwd`(받은 백엔드 · 공통 기본 모드 · 인자 없음) · 스모크 bin.
    fn spawn_path_commands() -> Vec<(&'static str, AgentCommand)> {
        vec![
            (
                "spawn-by-cwd/claude",
                AgentCommand::Claude {
                    extra_args: Vec::new(),
                    output_format: crate::commands::NEW_AGENT_OUTPUT_FORMAT,
                },
            ),
            (
                "spawn-by-cwd/codex",
                AgentCommand::Codex {
                    extra_args: Vec::new(),
                    output_format: crate::commands::NEW_AGENT_OUTPUT_FORMAT,
                },
            ),
            (
                "smoke-bin/claude-json",
                AgentCommand::Claude {
                    extra_args: strings(&["--model", "golden-model"]),
                    output_format: AgentOutputFormat::StreamJson,
                },
            ),
        ]
    }

    fn golden_env() -> Vec<(String, String)> {
        let path = std::env::join_paths(["/golden/sys", "/golden/tools"])
            .expect("고정 PATH")
            .into_string()
            .expect("UTF-8");
        vec![
            ("PATH".to_string(), path),
            ("GOLDEN_FLAG".to_string(), "1".to_string()),
        ]
    }

    fn profile_of(command: AgentCommand, env: Vec<(String, String)>) -> AgentProfile {
        AgentProfile::new(
            "golden".to_string(),
            command,
            PathBuf::from(WORK),
            env,
            false,
        )
    }

    fn claude_terminal() -> AgentCommand {
        golden_commands().remove(0).1
    }

    fn attachments(default_preset: bool, paths: &[&str]) -> Attachments {
        Attachments {
            default_preset,
            presets: paths
                .iter()
                .map(|p| AttachedPreset {
                    path: (*p).to_string(),
                })
                .collect(),
        }
    }

    fn blocked() -> LaunchBlocked {
        LaunchBlocked {
            reasons: vec![BlockReason::Missing {
                item: "team/a.md".to_string(),
                via: vec!["team/base.yaml".to_string()],
                reason: Reason {
                    code: "missing-file".to_string(),
                    args: json!({ "path": "team/a.md" }),
                    line: Some(1),
                },
            }],
        }
    }

    /// 활성화 갈래 셋 — 이어받기는 저장 손잡이가 있을 때만 이어받고, 없으면 새 대화로 넘긴다(`resume_no_fallback` 머리).
    #[derive(Debug, Clone, Copy)]
    enum Way {
        Fresh,
        Resume,
        ResumeWithoutHandle,
    }

    impl Way {
        const ALL: [Way; 3] = [Way::Fresh, Way::Resume, Way::ResumeWithoutHandle];

        fn mode(self) -> SpawnMode {
            match self {
                Way::Fresh => SpawnMode::Fresh,
                Way::Resume | Way::ResumeWithoutHandle => SpawnMode::Resume,
            }
        }

        /// 이 갈래로 띄울 명부 항목 — 이어받기만 손잡이를 단다.
        fn profile(self) -> AgentProfile {
            let mut p = profile_of(claude_terminal(), golden_env());
            if matches!(self, Way::Resume) {
                p.backend_session_id = Some(STORED_SID);
            }
            p
        }
    }

    // ── 골든 대조 ──

    #[derive(Deserialize)]
    struct GoldenCase {
        case: String,
        console: bool,
        program: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        cwd: String,
    }

    /// (프로그램, 인자, env, cwd) — 골든과 같은 꼴로 편다(`tests/launch_argv_golden.rs` 의 `matches`).
    type Shape = (String, Vec<String>, Vec<(String, String)>, String);

    fn golden(case: &str) -> Shape {
        let all: Vec<GoldenCase> =
            serde_json::from_str(include_str!("../../tests/fixtures/launch_argv_golden.json"))
                .expect("골든은 JSON");
        let g = all
            .into_iter()
            .find(|g| g.case == case)
            .unwrap_or_else(|| panic!("골든에 `{case}` 가 없다 — 이 항목의 전제가 낡았다"));
        let (program, args) = if g.console {
            console_command(&g.program, g.args)
        } else {
            (g.program, g.args)
        };
        (program, args, g.env, g.cwd)
    }

    /// 스폰이 뽑은 세션 id 만 골든의 고정값으로 바꾼다 — 나머지는 바이트 그대로.
    fn shape(spec: &CommandSpec) -> Shape {
        let mut args = spec.args.clone();
        if let Some(i) = args.iter().position(|a| a == "--session-id") {
            if let Some(sid) = args.get_mut(i + 1) {
                if Uuid::parse_str(sid).is_ok_and(|s| s != STORED_SID) {
                    *sid = FRESH_SID.to_string();
                }
            }
        }
        let env = spec
            .env
            .iter()
            .map(|(k, v)| {
                if k.eq_ignore_ascii_case("PATH") {
                    let parts: Vec<String> = std::env::split_paths(v)
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect();
                    (k.clone(), parts.join(";"))
                } else {
                    (k.clone(), v.clone())
                }
            })
            .collect();
        (
            spec.program.clone(),
            args,
            env,
            spec.cwd.to_string_lossy().into_owned(),
        )
    }

    fn assert_golden(case: &str, path: &str, spec: &CommandSpec) {
        assert_eq!(
            shape(spec),
            golden(case),
            "{path} 길의 argv 가 골든 `{case}` 와 다르다 — Noop 실행 재료는 스폰을 바이트 하나 바꾸지 않아야 한다"
        );
    }

    // ── Noop 회귀 ──

    /// ★Noop 재료로는 manager 의 어느 스폰 길도 오늘과 바이트 같은 argv 를 낸다★ — 활성화 · 깨우기(이어받기) · 즉석
    /// 생성(데몬 `SpawnByCwd` · 스모크 bin). 붙은 프리셋 · 「기본」 칸은 Noop 이 읽지 않고, `extra_args` 는 그대로 실린다.
    /// 배치는 기본값을 내고 배치 포트를 한 번도 부르지 않는다.
    // TRD S25 §4 「agent manager」 · §5-2 4
    #[test]
    fn noop_spawn_paths_produce_todays_argv() {
        fn assert_noop(rig: &Rig, case: &str, path: &str) {
            let (spec, artifacts) = rig.assembled();
            assert_golden(case, path, &spec);
            assert_eq!(artifacts, LaunchArtifacts::default(), "{path} · {case}");
            assert_eq!(rig.fs_calls(), 0, "{path} · {case}: 배치 포트를 불렀다");
        }

        for (name, command) in golden_commands() {
            let fresh = format!("{name}/fresh/no-endpoint");

            let rig = Rig::noop();
            let mut p = profile_of(command.clone(), golden_env());
            p.presets = attachments(true, &["/team/a.md"]).presets;
            rig.manager.profiles.upsert(p.clone());
            let _ = rig.manager.activate_profile(&p, SpawnMode::Fresh);
            assert_noop(&rig, &fresh, "활성화");

            let rig = Rig::noop();
            let mut p = profile_of(command.clone(), golden_env());
            p.presets = attachments(true, &["/team/a.md"]).presets;
            if backend::can_resume_stored_session(&command) {
                p.backend_session_id = Some(STORED_SID);
            }
            rig.manager.profiles.upsert(p.clone());
            let _ = rig.manager.activate_profile(&p, SpawnMode::Resume);
            assert_noop(&rig, &format!("{name}/resume/no-endpoint"), "깨우기");

            let rig = Rig::noop();
            let p = profile_of(command, golden_env());
            let _ = rig.manager.spawn_agent(&p, SpawnMode::Fresh);
            assert_noop(&rig, &fresh, "즉석 생성");
        }

        for (name, command) in spawn_path_commands() {
            let rig = Rig::noop();
            let p = profile_of(command, Vec::new());
            assert!(
                p.default_preset,
                "생성자는 「기본」 을 단다 — Noop 은 그것을 읽지 않는다"
            );
            let _ = rig.manager.spawn_agent(&p, SpawnMode::Fresh);
            assert_noop(&rig, &format!("{name}/fresh/no-endpoint"), name);
        }
    }

    /// ★작업 폴더는 호출자 사본이 아니라 명부에서 읽는다★ — 사본을 뜬 뒤 작업 폴더가 바뀌었으면(`agent.setWorkspace`)
    /// 사본의 cwd 는 낡았다.
    // TRD S25 §1-5 (가) 3
    #[test]
    fn the_spawned_cwd_is_the_rosters_not_the_callers_copy() {
        let rig = Rig::noop();
        let mut stored = profile_of(claude_terminal(), golden_env());
        stored.cwd = PathBuf::from("/golden/moved");
        rig.manager.profiles.upsert(stored.clone());
        let mut stale = stored.clone();
        stale.cwd = PathBuf::from(WORK);

        let _ = rig.manager.activate_profile(&stale, SpawnMode::Fresh);

        assert_eq!(rig.spec().cwd, PathBuf::from("/golden/moved"));
        assert_eq!(
            rig.manager.profiles.get(stored.id).unwrap().cwd,
            PathBuf::from("/golden/moved"),
            "사본의 cwd 가 명부를 되돌리지 않는다"
        );
    }

    // ── 순서 ──

    /// ★점검은 예약 뒤 · 화신 표식과 세션 파생 앞에, 명부의 값으로, 매니저 락 없이 돈다★.
    // ADR-0319
    #[test]
    fn the_check_runs_on_the_roster_after_the_reservation_and_before_the_incarnation() {
        let kits = ScriptedKits::new(Vec::new());
        let rig = kits.rig();
        let mut caller = profile_of(claude_terminal(), golden_env());
        caller.backend_session_id = Some(STORED_SID);
        rig.manager.profiles.upsert(caller.clone());
        let attached = attachments(false, &["/team/a.md"]);
        assert!(rig
            .manager
            .profiles
            .try_set_attachments(caller.id, attached.clone())
            .unwrap());
        let epoch_before = rig.manager.profiles.get(caller.id).unwrap().epoch;

        let _ = rig.manager.activate_profile(&caller, SpawnMode::Fresh);

        let seen = kits.seen();
        assert_eq!(seen.len(), 1, "점검은 한 번");
        let s = &seen[0];
        assert!(
            s.roster_free && s.reservations_free && s.sessions_free && s.names_free,
            "점검을 매니저 락을 쥔 채 불렀다: {s:?}"
        );
        assert!(s.reserved, "예약 뒤가 아니다");
        assert!(!s.live_session);
        assert_eq!(
            (s.profile.default_preset, s.profile.presets.clone()),
            (attached.default_preset, attached.presets),
            "호출자 사본이 아니라 명부 값을 점검한다"
        );
        assert_eq!(s.roster_epoch, Some(epoch_before), "화신 표식 앞이 아니다");
        assert_eq!(
            s.profile.backend_session_id,
            Some(STORED_SID),
            "세션 파생(Fresh 비우기) 앞이 아니다"
        );

        // 점검 뒤로 스폰이 이어졌다 — 표식을 뽑고 세션 id 를 비우고 명령을 조립했다.
        let _ = rig.spec();
        let after = rig.manager.profiles.get(caller.id).unwrap();
        assert_ne!(after.epoch, epoch_before);
        assert_eq!(after.backend_session_id, None);
    }

    /// ★명부에 없는 id 는 넣기 전에 호출자 사본으로 한 번, 넣은 뒤 명부 값으로 한 번 점검한다★ — 앞엣것은 막히면
    /// 아무것도 넣지 않으려는 것이고, 띄울 계획은 뒤엣것이다(배정된 이름 · 생성자 기본값을 본다).
    // TRD S25 §1-5 (가) 0 · 1 ②
    #[test]
    fn an_unregistered_profile_is_checked_before_registration_and_again_on_the_roster() {
        let kits = ScriptedKits::new(Vec::new());
        let rig = kits.rig();
        let adhoc = profile_of(spawn_path_commands().remove(0).1, Vec::new());

        let _ = rig.manager.spawn_agent(&adhoc, SpawnMode::Fresh);

        let seen = kits.seen();
        assert_eq!(seen.len(), 2, "넣기 전 · 넣은 뒤 한 번씩");
        let (before, after) = (&seen[0], &seen[1]);
        assert!(
            before.roster_free
                && before.reservations_free
                && before.sessions_free
                && before.names_free,
            "넣기 전 점검을 매니저 락을 쥔 채 불렀다: {before:?}"
        );
        assert!(before.reserved, "넣기 전 점검이 예약 뒤가 아니다");
        assert_eq!(before.roster_epoch, None, "넣기 전 점검이 넣은 뒤에 돌았다");
        assert_eq!(
            before.profile.display_name, None,
            "넣기 전 점검은 호출자 사본을 본다"
        );
        assert_eq!(
            after.roster_epoch,
            Some(adhoc.epoch),
            "점검 때 명부에 있었고 표식은 아직 안 뽑혔다"
        );
        assert!(
            after.profile.display_name.is_some(),
            "이름은 둘째 점검 앞에 배정된다"
        );
        assert!(after.profile.default_preset && after.profile.presets.is_empty());
        let _ = rig.spec();
    }

    /// ★떠 있거나 뜨는 중이면 점검하지 않는다★ — 전처리는 예약 · 「이미 떠 있음」 확인 뒤다.
    #[test]
    fn a_running_or_starting_agent_is_not_checked() {
        let kits = ScriptedKits::new(Vec::new());
        let rig = kits.rig();
        let p = profile_of(claude_terminal(), golden_env());
        rig.manager.profiles.upsert(p.clone());

        {
            let _held =
                SpawnReservation::reserve(rig.manager.spawning.clone(), p.id).expect("예약");
            assert!(matches!(
                rig.manager.spawn_agent(&p, SpawnMode::Fresh),
                Ok(SpawnOutcome::Moot(None))
            ));
        }
        put_live_session(&rig.manager, p.id, 1);
        assert!(matches!(
            rig.manager.spawn_agent(&p, SpawnMode::Fresh),
            Ok(SpawnOutcome::Moot(Some(_)))
        ));

        assert!(kits.seen().is_empty(), "점검이 돌았다: {:?}", kits.seen());
        assert!(rig.assembled_nothing());
    }

    // ── 막힘 · 경합 ──

    /// ★막히면 띄우지 않는다★ — 제어 채널 · 화신 표식 · 세션 id 를 건드리지 않고 구조 있는 거절을 그대로 돌려주며,
    /// 「마지막 실패」 는 「그 밖」 이다(이어받기 길 · 손잡이 없는 이어받기의 새 대화 넘김도 같은 답).
    #[test]
    fn a_blocked_activation_starts_nothing_and_records_other() {
        for way in Way::ALL {
            let kits = ScriptedKits::new(vec![Turn::Block(blocked())]);
            let rig = kits.rig();
            let p = way.profile();
            rig.manager.profiles.upsert(p.clone());
            let epoch_before = rig.manager.profiles.get(p.id).unwrap().epoch;

            let err = rig
                .manager
                .activate_profile(&p, way.mode())
                .expect_err("막힘은 오류다");

            assert!(
                matches!(&err, PtyError::LaunchBlocked(b) if *b == blocked()),
                "{way:?}: {err}"
            );
            assert!(rig.assembled_nothing(), "{way:?}: 명령을 조립했다");
            assert_eq!(rig.provisions(), 0, "{way:?}: 제어 채널을 발급했다");
            assert!(rig.manager.list_agents().is_empty(), "{way:?}");
            let after = rig.manager.profiles.get(p.id).unwrap();
            assert_eq!(after.epoch, epoch_before, "{way:?}: 화신 표식을 뽑았다");
            assert_eq!(after.backend_session_id, p.backend_session_id, "{way:?}");
            assert_eq!(after.last_failure, Some(AgentFailureKind::Other), "{way:?}");
        }
    }

    /// ★부팅 복원이 막히면 띄우지 않고 「실패」 로 보고한다★ — 이어받기 · 새 대화 두 갈래 다(`restore_one`).
    // ADR-0334
    #[test]
    fn a_blocked_boot_restore_reports_failed_and_starts_nothing() {
        for way in [Way::Resume, Way::Fresh] {
            let kits = ScriptedKits::new(vec![Turn::Block(blocked())]);
            let rig = kits.rig();
            let p = way.profile();
            rig.manager.profiles.upsert(p.clone());
            let epoch_before = rig.manager.profiles.get(p.id).unwrap().epoch;

            let outcome = rig.manager.restore_one(&p);

            assert!(
                matches!(&outcome, RestoreOutcome::Failed { reason }
                    if reason.contains(&blocked().to_string())),
                "{way:?}: {outcome:?}"
            );
            assert!(rig.assembled_nothing(), "{way:?}: 명령을 조립했다");
            assert_eq!(rig.provisions(), 0, "{way:?}: 제어 채널을 발급했다");
            let after = rig.manager.profiles.get(p.id).unwrap();
            assert_eq!(after.epoch, epoch_before, "{way:?}: 화신 표식을 뽑았다");
            assert_eq!(after.backend_session_id, p.backend_session_id, "{way:?}");
            assert_eq!(after.last_failure, Some(AgentFailureKind::Other), "{way:?}");
        }
    }

    /// ★An ad-hoc create blocked by the pre-add check adds nothing★ — a block found before the add leaves no roster row
    /// that cannot start. A create that fails after the add leaves the agent asleep, and the wire `SpawnByCwd` answer
    /// names it by `created_agent_id` so the caller wakes it instead of creating another (ADR-0348).
    // TRD S25 §1-5 (가) 1 ②
    #[test]
    fn a_blocked_ad_hoc_spawn_registers_nothing_and_starts_nothing() {
        let kits = ScriptedKits::new(vec![Turn::Block(blocked())]);
        let rig = kits.rig();
        let adhoc = profile_of(spawn_path_commands().remove(0).1, Vec::new());

        let err = rig
            .manager
            .spawn_agent(&adhoc, SpawnMode::Fresh)
            .expect_err("막힘은 오류다");

        assert!(
            matches!(&err, PtyError::LaunchBlocked(b) if *b == blocked()),
            "{err}"
        );
        assert_eq!(kits.seen().len(), 1, "넣기 전 점검에서 끊긴다");
        assert!(rig.manager.profiles.get(adhoc.id).is_none(), "넣었다");
        assert_eq!(rig.store.attempts(), 0, "저장했다");
        assert!(rig.assembled_nothing());
        assert_eq!(rig.provisions(), 0);
    }

    /// 넣은 뒤 명부 값 점검에서 막히면 넣은 항목은 남는다(부분 성공 — 등록은 되돌리지 않는다).
    #[test]
    fn an_ad_hoc_spawn_blocked_after_registration_keeps_it_and_starts_nothing() {
        let kits = ScriptedKits::new(vec![Turn::Pass, Turn::Block(blocked())]);
        let rig = kits.rig();
        let adhoc = profile_of(spawn_path_commands().remove(0).1, Vec::new());

        let err = rig
            .manager
            .spawn_agent(&adhoc, SpawnMode::Fresh)
            .expect_err("막힘은 오류다");

        assert!(matches!(err, PtyError::LaunchBlocked(_)), "{err}");
        assert_eq!(kits.seen().len(), 2);
        assert!(rig.manager.profiles.get(adhoc.id).is_some());
        assert_eq!(rig.store.attempts(), 1, "넣기의 저장 하나");
        assert!(rig.assembled_nothing());
        assert_eq!(rig.provisions(), 0);
    }

    /// ★점검하는 사이 붙임이 바뀌면 명부를 다시 읽어 한 번 더 점검한다★ — 두 번째는 새 값을 본다.
    // TRD S25 §1-5 (가) 2
    #[test]
    fn a_preset_change_during_the_check_is_checked_again_once() {
        let next = attachments(true, &["/team/b.md"]);
        let change = next.clone();
        let kits = ScriptedKits::new(vec![Turn::Meanwhile(Box::new(move |profiles, id| {
            assert!(profiles.try_set_attachments(id, change).unwrap());
        }))]);
        let rig = kits.rig();
        let p = profile_of(claude_terminal(), golden_env());
        rig.manager.profiles.upsert(p.clone());

        let _ = rig.manager.activate_profile(&p, SpawnMode::Fresh);

        let seen = kits.seen();
        assert_eq!(seen.len(), 2, "다시 한 번만 점검한다");
        assert!(seen[0].profile.presets.is_empty());
        assert_eq!(seen[1].profile.presets, next.presets, "다시 읽은 명부 값");
        let _ = rig.spec();
    }

    /// 점검하는 사이 붙임을 `paths` 로 바꾼다.
    fn set_presets(paths: &'static [&'static str]) -> Meanwhile {
        Box::new(move |profiles: &ProfileRegistry, id| {
            assert!(profiles
                .try_set_attachments(id, attachments(true, paths))
                .unwrap());
        })
    }

    /// ★두 번 다 바뀌면 띄우지 않고 경합으로 끊는다★ — 「마지막 실패」 는 「그 밖」(이어받기 길 · 손잡이 없는
    /// 이어받기의 새 대화 넘김도 같은 답).
    #[test]
    fn presets_that_keep_changing_fail_the_launch_as_raced() {
        for way in Way::ALL {
            let kits = ScriptedKits::new(vec![
                Turn::Meanwhile(set_presets(&["/team/b.md"])),
                Turn::Meanwhile(set_presets(&["/team/c.md"])),
            ]);
            let rig = kits.rig();
            let p = way.profile();
            rig.manager.profiles.upsert(p.clone());

            let err = rig
                .manager
                .activate_profile(&p, way.mode())
                .expect_err("경합은 오류다");

            assert!(matches!(err, PtyError::LaunchRaced), "{way:?}: {err}");
            assert_eq!(kits.seen().len(), 2, "{way:?}");
            assert!(rig.assembled_nothing(), "{way:?}");
            assert_eq!(rig.provisions(), 0, "{way:?}");
            let after = rig.manager.profiles.get(p.id).unwrap();
            assert_eq!(after.backend_session_id, p.backend_session_id, "{way:?}");
            assert_eq!(after.last_failure, Some(AgentFailureKind::Other), "{way:?}");
        }
    }

    /// ★점검하는 사이 실행 입력이 바뀌었으면 막힘도 버리고 다시 점검한다★ — 그 막힘은 바뀌기 전 값으로 낸 것이라
    /// 낡았다(고친 붙임이 낡은 막힘에 걸려 끊긴다).
    // TRD S25 §1-5 (가) 1 · 2
    #[test]
    fn a_block_from_presets_that_changed_during_the_check_is_checked_again() {
        let kits = ScriptedKits::new(vec![Turn::MeanwhileBlock(
            set_presets(&["/team/b.md"]),
            blocked(),
        )]);
        let rig = kits.rig();
        let p = profile_of(claude_terminal(), golden_env());
        rig.manager.profiles.upsert(p.clone());

        let _ = rig.manager.activate_profile(&p, SpawnMode::Fresh);

        let seen = kits.seen();
        assert_eq!(seen.len(), 2, "낡은 막힘을 버리고 한 번 더 점검한다");
        assert_eq!(
            seen[1].profile.presets,
            attachments(true, &["/team/b.md"]).presets,
            "다시 읽은 명부 값"
        );
        let _ = rig.spec();
    }

    /// ★바뀐 입력으로 두 번 다 막히면 막힘이 아니라 경합이다★ — 어느 막힘도 지금 명부 값의 답이 아니다.
    #[test]
    fn blocks_from_presets_that_keep_changing_fail_the_launch_as_raced() {
        let kits = ScriptedKits::new(vec![
            Turn::MeanwhileBlock(set_presets(&["/team/b.md"]), blocked()),
            Turn::MeanwhileBlock(set_presets(&["/team/c.md"]), blocked()),
        ]);
        let rig = kits.rig();
        let p = profile_of(claude_terminal(), golden_env());
        rig.manager.profiles.upsert(p.clone());

        let err = rig
            .manager
            .activate_profile(&p, SpawnMode::Fresh)
            .expect_err("경합은 오류다");

        assert!(matches!(err, PtyError::LaunchRaced), "{err}");
        assert_eq!(kits.seen().len(), 2);
        assert!(rig.assembled_nothing());
        assert_eq!(rig.provisions(), 0);
        assert_eq!(
            rig.manager.profiles.get(p.id).unwrap().last_failure,
            Some(AgentFailureKind::Other)
        );
    }

    /// ★넘긴 사본의 명령 · env 가 점검한 명부 값과 다르면 띄우지 않는다★ — 스폰은 그 둘을 사본에서 명부로 옮기므로
    /// 그대로 가면 점검하지 않은 명령으로 뜬다. 명부도 그 사본으로 덮지 않는다.
    // TRD S25 §1-5 (가) 2
    #[test]
    fn a_callers_copy_whose_command_or_env_differs_from_the_roster_is_raced() {
        let edits: [(&str, fn(&mut AgentProfile)); 2] = [
            ("env", |c| {
                c.env.push(("STALE".to_string(), "1".to_string()))
            }),
            ("command", |c| c.command = golden_commands().remove(4).1),
        ];
        for (what, edit) in edits {
            let kits = ScriptedKits::new(Vec::new());
            let rig = kits.rig();
            let stored = profile_of(claude_terminal(), golden_env());
            rig.manager.profiles.upsert(stored.clone());
            let mut caller = stored.clone();
            edit(&mut caller);

            let err = rig
                .manager
                .activate_profile(&caller, SpawnMode::Fresh)
                .expect_err("경합은 오류다");

            assert!(matches!(err, PtyError::LaunchRaced), "{what}: {err}");
            assert_eq!(
                kits.seen().len(),
                1,
                "{what}: 명부가 그대로라 다시 점검하지 않는다"
            );
            assert!(rig.assembled_nothing(), "{what}");
            assert_eq!(rig.provisions(), 0, "{what}");
            let after = rig.manager.profiles.get(stored.id).unwrap();
            assert_eq!(
                (after.command, after.env),
                (stored.command.clone(), stored.env.clone()),
                "{what}: 사본이 명부를 덮었다"
            );
            assert_eq!(after.last_failure, Some(AgentFailureKind::Other), "{what}");
        }
    }

    /// 점검하는 사이 지워졌으면 끊는다 — 지운 프로필로 뜨지 않는다.
    #[test]
    fn a_profile_deleted_during_the_check_is_not_started() {
        let kits = ScriptedKits::new(vec![Turn::Meanwhile(Box::new(|profiles, id| {
            profiles.try_remove(id).unwrap();
        }))]);
        let rig = kits.rig();
        let p = profile_of(claude_terminal(), golden_env());
        rig.manager.profiles.upsert(p.clone());

        let err = rig
            .manager
            .activate_profile(&p, SpawnMode::Fresh)
            .expect_err("지워졌다");

        assert!(
            matches!(&err, PtyError::SpawnFailed(m) if m.contains("실행 전처리 대조")),
            "{err}"
        );
        assert!(
            rig.manager.profiles.get(p.id).is_none(),
            "되살아나지 않는다"
        );
        assert!(rig.assembled_nothing());
    }

    // ── Notices once per agent (ADR-0341) ──

    fn notice(code: &str) -> Reason {
        Reason {
            code: code.to_string(),
            args: json!({ "names": [code] }),
            line: Some(1),
        }
    }

    /// The plan `profile` checks to, carrying `notices`.
    fn plan_with(profile: &AgentProfile, notices: Vec<Reason>) -> LaunchPlan {
        LaunchPlan {
            notices,
            ..NoopLaunchKits.check(profile).unwrap()
        }
    }

    #[test]
    fn plan_notices_are_empty_only_when_the_stored_fingerprint_matches() {
        let mut roster = Way::Fresh.profile();
        let x = plan_with(&roster, vec![notice("a"), notice("b")]);
        let fingerprint = crate::launch::notice_fingerprint(&x.notices);

        let never = PlanNotices::against(&x, &roster);
        assert_eq!(never.fingerprint, fingerprint);
        assert_eq!(never.shown, x.notices, "nothing recorded → the full list");
        assert_eq!(
            never.baseline, None,
            "the record's baseline = the value compared"
        );

        roster.notice_fingerprint = Some(fingerprint.clone());
        let same = PlanNotices::against(&x, &roster);
        assert!(same.shown.is_empty(), "same set");
        assert_eq!(same.baseline, Some(fingerprint.clone()));

        roster.notice_fingerprint = Some(crate::launch::notice_fingerprint(&[notice("a")]));
        assert_eq!(
            PlanNotices::against(&x, &roster).shown,
            x.notices,
            "a changed set → the full list, not the difference"
        );

        roster.notice_fingerprint = Some(String::new());
        let none = PlanNotices::against(&plan_with(&roster, Vec::new()), &roster);
        assert_eq!(none.fingerprint, "");
        assert!(none.shown.is_empty());
    }

    /// The compare reads the roster, not the caller's copy — a copy taken before the last record would re-show the set
    /// (and a copy carrying a record the roster lacks would hide it).
    #[test]
    fn prelaunch_compares_with_the_roster_fingerprint_not_the_callers_copy() {
        let p = Way::Fresh.profile();
        let x = vec![notice("a")];
        let shown_before = crate::launch::notice_fingerprint(&x);

        let kits = ScriptedKits::new(vec![
            Turn::Plan(plan_with(&p, x.clone())),
            Turn::Plan(plan_with(&p, x.clone())),
        ]);
        let rig = kits.rig();
        let mut roster = p.clone();
        roster.notice_fingerprint = Some(shown_before.clone());
        rig.manager.profiles.upsert(roster);

        let (_, notices) = rig.manager.prelaunch(&p).expect("passes");
        assert!(
            notices.shown.is_empty(),
            "the roster was already shown this set"
        );
        assert_eq!(notices.fingerprint, shown_before);

        rig.manager
            .profiles
            .update_with(p.id, |q| q.notice_fingerprint = None);
        let mut stale_copy = p.clone();
        stale_copy.notice_fingerprint = Some(shown_before);
        let (_, notices) = rig.manager.prelaunch(&stale_copy).expect("passes");
        assert_eq!(notices.shown, x, "nothing recorded on the roster");
    }

    /// ★No failure marks a set as seen★ — a block, a race, a prepare failure and a spawn error after the assembly all
    /// answer with an error, so a delivering caller records nothing and the next success shows the set.
    // TRD S25 §1-5 「Notices once per agent」
    #[test]
    fn a_failed_delivered_activation_records_no_fingerprint() {
        type Expect = fn(&PtyError) -> bool;
        let cases: [(&str, Option<PrepareError>, Expect); 4] = [
            ("block", None, |e| matches!(e, PtyError::LaunchBlocked(_))),
            (
                "stale prepare",
                Some(PrepareError::Stale("gone".to_string())),
                |e| matches!(e, PtyError::LaunchRaced),
            ),
            (
                "failed prepare",
                Some(PrepareError::Failed("disk".to_string())),
                |e| matches!(e, PtyError::SpawnFailed(m) if !m.contains("시험")),
            ),
            (
                "spawn error after the assembly",
                None,
                |e| matches!(e, PtyError::SpawnFailed(m) if m.contains("시험")),
            ),
        ];
        for (what, prep, expected) in cases {
            for way in Way::ALL {
                let p = way.profile();
                let turn = if what == "block" {
                    Turn::Block(blocked())
                } else {
                    Turn::Plan(plan_with(&p, vec![notice("a")]))
                };
                let preps = prep.clone().map(Prep::Fail).into_iter().collect();
                let kits = ScriptedKits::preparing(vec![turn], preps);
                let rig = kits.rig();
                rig.manager.profiles.upsert(p.clone());

                let err = rig
                    .manager
                    .activate_profile_reporting(&p, way.mode(), NoticeDelivery::Delivered)
                    .expect_err(what);
                assert!(expected(&err), "{what} ({way:?}): {err}");
                assert_eq!(
                    rig.manager.profiles.get(p.id).unwrap().notice_fingerprint,
                    None,
                    "{what} ({way:?}) recorded the set as seen"
                );
            }
        }
    }

    // ── 재료 준비 ──

    /// 점검이 낸 계획임을 알아볼 표시를 단 Noop 계획.
    fn marked_plan() -> LaunchPlan {
        LaunchPlan {
            notices: vec![Reason {
                code: "checked-plan".to_string(),
                args: json!({}),
                line: None,
            }],
            ..NoopLaunchKits
                .check(&profile_of(claude_terminal(), golden_env()))
                .unwrap()
        }
    }

    /// ★재료는 점검한 계획으로, 화신 표식 뒤 · 세션 id 비우기와 제어 채널 발급 앞에, 매니저 락 없이 준비한다★.
    // TRD S25 §1-5 (나) 4 · §5-2 4 (a)
    #[test]
    fn the_kit_is_prepared_from_the_checked_plan_after_the_incarnation_without_manager_locks() {
        let kits = ScriptedKits::preparing(vec![Turn::Plan(marked_plan())], Vec::new());
        let rig = kits.rig();
        let p = Way::Resume.profile();
        rig.manager.profiles.upsert(p.clone());
        let epoch_before = rig.manager.profiles.get(p.id).unwrap().epoch;

        let _ = rig.manager.activate_profile(&p, SpawnMode::Fresh);

        let prepared = kits.prepared();
        assert_eq!(prepared.len(), 1, "준비는 한 번");
        let s = &prepared[0];
        assert_eq!(s.plan, marked_plan(), "점검이 낸 계획 그대로가 아니다");
        assert!(s.locks_free, "준비를 매니저 락을 쥔 채 불렀다: {s:?}");
        assert!(s.reserved, "예약 밖에서 준비했다");
        assert!(!s.live_session);
        assert_eq!(s.id, p.id);
        assert_ne!(s.epoch, epoch_before, "화신 표식 앞이다");
        assert_eq!(
            s.roster_epoch,
            Some(s.epoch),
            "이 화신의 표식으로 준비하지 않았다"
        );
        assert_eq!(s.roster_sid, Some(STORED_SID), "세션 id 비우기 뒤다");
        assert_eq!(s.provisions, 0, "제어 채널 발급 뒤다");
        let _ = rig.spec();
    }

    /// ★조립에 실리는 재료는 준비가 낸 재료를 한 번 배치한 것이다★ — 점검한 계획은 Noop 인데 재료만 실 재료로 표시하면
    /// 실린 재료도 실 재료다. 번역이 아직 없는 backend 의 배치는 그 표시만 넘기고 배치 포트를 부르지 않는다. Noop 재료는
    /// 기본값 그대로다(argv 골든 = `noop_spawn_paths_produce_todays_argv`).
    // TRD S25 §1-5 (나) 6 · 7
    #[test]
    fn the_assembled_artifacts_come_from_the_prepared_kit() {
        let p = profile_of(claude_terminal(), golden_env());

        let kits = ScriptedKits::preparing(Vec::new(), vec![Prep::MarkReal]);
        let rig = kits.rig();
        rig.manager.profiles.upsert(p.clone());
        let _ = rig.manager.activate_profile(&p, SpawnMode::Fresh);
        assert_eq!(
            rig.assembled().1,
            LaunchArtifacts {
                real: true,
                ..LaunchArtifacts::default()
            }
        );
        assert!(rig.assembled_nothing(), "한 번만 조립한다");
        assert_eq!(rig.fs_calls(), 0);

        let rig = Rig::noop();
        rig.manager.profiles.upsert(p.clone());
        let _ = rig.manager.activate_profile(&p, SpawnMode::Fresh);
        assert_eq!(rig.assembled().1, LaunchArtifacts::default());
        assert_eq!(rig.fs_calls(), 0);
    }

    /// ★배치가 실패하면 띄우지 않고 그 화신의 재료를 가드가 정확히 한 번 거둔다★ — 여기서는 준비가 상대 경로 휘발
    /// 폴더를 내 배치가 끊는다(가둘 뿌리가 데몬 현재 폴더 기준이 된다). 조립 · 띄우기 전이고 세션도 없다.
    // TRD S25 §1-5 (나) 6 · §5-2 4 (c2) ①
    #[test]
    fn a_failed_materialize_releases_the_kit_once_and_starts_nothing() {
        let kits = ScriptedKits::preparing(
            Vec::new(),
            vec![Prep::Volatile(PathBuf::from("relative").join("launch"))],
        );
        let rig = kits.rig();
        let p = profile_of(claude_terminal(), golden_env());
        rig.manager.profiles.upsert(p.clone());

        let err = rig
            .manager
            .activate_profile(&p, SpawnMode::Fresh)
            .expect_err("배치 실패는 오류다");

        assert!(
            matches!(&err, PtyError::SpawnFailed(m) if m.contains("실행 재료 배치 실패")),
            "{err}"
        );
        let prepared = kits.prepared();
        assert_eq!(prepared.len(), 1);
        assert_eq!(
            kits.released(),
            vec![(p.id, prepared[0].epoch)],
            "거두기가 정확히 한 번이 아니다"
        );
        let r = &kits.releases()[0];
        assert!(r.locks_free && !r.on_reaper && !r.live_session, "{r:?}");
        assert!(rig.assembled_nothing(), "배치 앞에서 끊기지 않았다");
        assert_eq!(rig.fs_calls(), 0);
        assert!(rig.manager.list_agents().is_empty());
        assert_eq!(
            rig.manager.profiles.get(p.id).unwrap().last_failure,
            Some(AgentFailureKind::SpawnFailed)
        );
    }

    /// ★준비가 실패하면 그 화신의 재료를 정확히 한 번 거두고 띄우지 않는다★ — 원본이 점검 뒤 바뀐 것은 경합(「마지막
    /// 실패」 = 그 밖), 그 밖은 `SpawnFailed`. 어느 쪽이든 저장된 세션 id 를 밀지 않고 제어 채널을 발급하지 않는다.
    // TRD S25 §5-2 4 (c)
    #[test]
    fn a_failed_prepare_releases_the_kit_once_and_starts_nothing() {
        for way in Way::ALL {
            for stale in [true, false] {
                let err = if stale {
                    PrepareError::Stale("team/a.md 사라짐".to_string())
                } else {
                    PrepareError::Failed("휘발 폴더를 못 만듦".to_string())
                };
                let kits = ScriptedKits::preparing(Vec::new(), vec![Prep::Fail(err)]);
                let rig = kits.rig();
                let p = way.profile();
                rig.manager.profiles.upsert(p.clone());

                let got = rig
                    .manager
                    .activate_profile(&p, way.mode())
                    .expect_err("준비 실패는 오류다");

                let case = format!("{way:?} stale={stale}");
                if stale {
                    assert!(matches!(got, PtyError::LaunchRaced), "{case}: {got}");
                } else {
                    assert!(
                        matches!(&got, PtyError::SpawnFailed(m) if m.contains("휘발 폴더를 못 만듦")),
                        "{case}: {got}"
                    );
                }
                let prepared = kits.prepared();
                assert_eq!(prepared.len(), 1, "{case}");
                assert_eq!(
                    kits.released(),
                    vec![(p.id, prepared[0].epoch)],
                    "{case}: 거두기가 정확히 한 번이 아니다"
                );
                assert!(rig.assembled_nothing(), "{case}");
                assert_eq!(rig.provisions(), 0, "{case}");
                assert!(rig.manager.list_agents().is_empty(), "{case}");
                let after = rig.manager.profiles.get(p.id).unwrap();
                assert_eq!(after.backend_session_id, p.backend_session_id, "{case}");
                let kind = if stale {
                    AgentFailureKind::Other
                } else {
                    AgentFailureKind::SpawnFailed
                };
                assert_eq!(after.last_failure, Some(kind), "{case}");
            }
        }
    }

    /// ★준비 뒤 세션이 넘겨받기 전에 끊겨도 재료를 정확히 한 번 거둔다★ — 여기서는 조립 뒤 시험 탐침이 띄우기 전에
    /// 끊는다(띄우기 · 세션 등록 실패와 같은 구간).
    #[test]
    fn a_failure_between_prepare_and_the_session_releases_the_kit_once() {
        let kits = ScriptedKits::preparing(Vec::new(), Vec::new());
        let rig = kits.rig();
        let p = profile_of(claude_terminal(), golden_env());
        rig.manager.profiles.upsert(p.clone());

        let err = rig
            .manager
            .activate_profile(&p, SpawnMode::Fresh)
            .expect_err("탐침이 띄우기 전에 끊는다");

        assert!(
            matches!(&err, PtyError::SpawnFailed(m) if m.contains("시험")),
            "{err}"
        );
        let _ = rig.spec();
        let prepared = kits.prepared();
        assert_eq!(prepared.len(), 1);
        assert_eq!(kits.released(), vec![(p.id, prepared[0].epoch)]);
    }

    /// 실 셸 하나를 띄울 시험대 — 탐침을 떼 띄우기까지 간다.
    fn started_shell(kits: &Arc<ScriptedKits>) -> (Rig, AgentInfo) {
        let rig = kits.rig();
        *sync::lock(&rig.manager.spec_probe) = None;
        let mut p = profile_of(
            AgentCommand::Shell {
                program: engram_dashboard_platform::shell::default_shell().to_string(),
                args: Vec::new(),
            },
            Vec::new(),
        );
        p.cwd = std::env::temp_dir();
        rig.manager.profiles.upsert(p.clone());
        let info = rig
            .manager
            .activate_profile(&p, SpawnMode::Fresh)
            .expect("셸이 뜬다");
        (rig, info)
    }

    /// 첫 거두기를 기다린 뒤 잠깐 더 기다려 그 뒤로 거두기가 더 오지 않았나까지 본다.
    fn settled_releases(kits: &ScriptedKits) -> Vec<Release> {
        assert!(
            engram_dashboard_base::testing::wait_until(Duration::from_secs(15), || {
                !kits.released().is_empty()
            }),
            "세션이 끝났는데 재료를 아무도 안 거뒀다"
        );
        std::thread::sleep(Duration::from_millis(300));
        kits.releases()
    }

    /// ★뜬 화신의 재료는 가드가 거두지 않는다 — 끄면 세션을 명부에서 뺀 reaper 가 정확히 한 번 거둔다★: kill 의 앞당긴
    /// 권한 회수 옆이 아니라(그때는 자식이 살아 있다), 락을 하나도 쥐지 않은 채, auto_restore 를 내린 뒤에. 시험대는
    /// 생성 뒤 재료 포트를 바꿔 끼운다 — reaper 가 생성 때 잡은 포트로 거두면 여기 아무것도 안 온다.
    // ADR-0019
    // ADR-0334
    // TRD S25 §5-2 4 (c2) ② · ③ · ④
    #[test]
    fn the_reaper_releases_a_killed_agents_kit_once_after_the_session_is_gone() {
        let kits = ScriptedKits::preparing(Vec::new(), Vec::new());
        let (rig, info) = started_shell(&kits);

        assert_eq!(kits.prepared().len(), 1);
        assert!(
            kits.released().is_empty(),
            "뜬 화신의 재료를 가드가 거뒀다: {:?}",
            kits.released()
        );
        rig.manager.kill_agent(info.id).expect("끈다");

        let releases = settled_releases(&kits);
        assert_eq!(
            kits.released(),
            vec![(info.id, info.epoch)],
            "거두기가 이 화신으로 정확히 한 번이 아니다"
        );
        let r = &releases[0];
        assert!(r.on_reaper, "reaper 밖에서 거뒀다: {r:?}");
        assert!(!r.live_session, "세션이 명부에 있는 채 거뒀다: {r:?}");
        assert!(r.locks_free, "매니저 락을 쥔 채 거뒀다: {r:?}");
        assert_eq!(
            r.auto_restore,
            Some(false),
            "auto_restore 를 내리기 전에 거뒀다: {r:?}"
        );
    }

    /// ★스스로 끝난 화신의 재료도 reaper 가 정확히 한 번 거둔다★ — 셸에 `exit` 를 쳐 끝낸다.
    // TRD S25 §5-2 4 (c2) ②
    #[test]
    fn the_reaper_releases_a_naturally_exited_agents_kit_once() {
        let kits = ScriptedKits::preparing(Vec::new(), Vec::new());
        let (rig, info) = started_shell(&kits);

        // 셸이 아직 입력을 안 읽었으면 다시 친다 — 끝난 뒤의 쓰기 실패는 무시한다.
        let exited = (0..10).any(|_| {
            let _ = rig
                .manager
                .write_stdin(info.id, b"exit\r", crate::types::InputOrigin::User);
            engram_dashboard_base::testing::wait_until(Duration::from_secs(3), || {
                !kits.released().is_empty()
            })
        });
        assert!(exited, "셸이 `exit` 로 끝나지 않았다");

        let releases = settled_releases(&kits);
        assert_eq!(kits.released(), vec![(info.id, info.epoch)]);
        let r = &releases[0];
        assert!(r.on_reaper && !r.live_session, "{r:?}");
        assert_eq!(r.auto_restore, Some(false), "{r:?}");
    }

    /// ★거두기는 표식이 맞는 수거 승자 한 번뿐이다★ — 늦게 온 앞 화신의 수거 · 같은 수거의 되풀이는 아무것도 거두지
    /// 않고, 지금 화신의 세션을 빼지도 않는다. 열쇠는 자기 화신만 거둔다.
    // ADR-0084
    // TRD S25 §5-2 4 (c2) ②
    #[test]
    fn a_stale_or_repeated_reap_releases_no_other_incarnation() {
        let kits = ScriptedKits::new(Vec::new());
        let rig = kits.rig();
        let manager = &rig.manager;
        let deps = ReaperDeps {
            sessions: manager.sessions.clone(),
            profiles: manager.profiles.clone(),
            status_sink: manager.status_sink.clone(),
            control: manager.control.clone(),
        };
        let id = Uuid::from_u128(0x5eed);
        let reap = |epoch: u32| crate::types::ReapMsg {
            id,
            epoch,
            reason: crate::types::TerminalReason::Killed,
            intent_at_finish: crate::types::TerminationIntent::UserKill,
            shutting_down_at_finish: true,
        };
        let lease = |epoch: u32| Some(LaunchLease::new(kits.clone(), id, epoch));

        put_live_session_leased(manager, id, 7, lease(7));
        deps.reap_one(reap(3));
        assert!(kits.released().is_empty(), "앞 화신의 수거가 거뒀다");
        assert!(manager.sessions.read().unwrap().contains_key(&id));

        deps.reap_one(reap(7));
        deps.reap_one(reap(7));
        assert_eq!(kits.released(), vec![(id, 7)], "수거 승자 한 번이 아니다");
        assert!(!kits.releases()[0].live_session);

        put_live_session_leased(manager, id, 9, lease(9));
        deps.reap_one(reap(7));
        assert_eq!(
            kits.released(),
            vec![(id, 7)],
            "늦게 온 앞 화신의 수거가 지금 화신을 거뒀다"
        );
        assert!(manager.sessions.read().unwrap().contains_key(&id));
    }

    // ── 소스 순서 ──

    /// 운영 구획에서 `start` 와 `end` 사이를 잘라 주석 줄을 빼고 공백을 전부 지운다(`manager.rs` 시험의 같은 이름
    /// 도우미와 같은 꼴).
    fn squashed(src: &str, start: &str, end: &str) -> String {
        let production = src.split("mod tests {").next().expect("운영 구획");
        let body = production
            .split(start)
            .nth(1)
            .unwrap_or_else(|| panic!("`{start}` 가 운영 구획에 없다 — 이 항목의 전제가 낡았다"))
            .split(end)
            .next()
            .expect("다음 구획까지");
        body.lines()
            .map(str::trim)
            .filter(|l| !l.starts_with("//"))
            .collect::<String>()
            .split_whitespace()
            .collect::<String>()
    }

    fn assert_in_order(body: &str, needles: &[&str]) {
        let mut last: Option<(&str, usize)> = None;
        for needle in needles {
            assert_eq!(
                body.matches(needle).count(),
                1,
                "`{needle}` 가 정확히 한 번이 아니다 — 이 항목이 자리를 특정하지 못한다"
            );
            let at = body.find(needle).unwrap();
            if let Some((prev, prev_at)) = last {
                assert!(
                    prev_at < at,
                    "`{prev}` 가 `{needle}` 보다 앞이 아니다: {body}"
                );
            }
            last = Some((needle, at));
        }
    }

    /// ★전처리 자리를 소스에서 못 박는다★ — 위 시험들은 한 스레드에서 차례로 재므로, 예약 · 떠 있음 확인 앞으로
    /// 옮겨도(같은 에이전트의 두 실행이 함께 점검) 표식 · 세션 파생 뒤로 밀어도(래치 · 표식이 맞추기 전 사본을 봄)
    /// 일부는 초록이다. 막힘이 제어 채널 발급 앞에서 끊기는 것과 명부 다시 읽기가 표식 뒤인 것, 대조가 점검의 답을
    /// 풀기 전인 것, 넣기 전 점검이 이름 배정 게이트 앞인 것도 함께 본다.
    // ADR-0319
    #[test]
    fn the_prelaunch_sits_after_the_reservation_and_before_the_incarnation_in_source() {
        let manager = include_str!("../manager.rs");
        let spawn = squashed(
            manager,
            "fn spawn_agent_watching_link(",
            "pub fn activate_profile(",
        );
        assert_in_order(
            &spawn,
            &[
                "SpawnReservation::reserve(",
                "self.get_session(profile.id)",
                "letinserted=self.ensure_registered(profile,entry)?;",
                "self.prelaunch(profile)?;",
                "if!inserted{self.register_for_spawn(profile,entry)?;}",
                ".epoch_for_spawn(profile.id)",
                "letreread=self.profiles.get(profile.id)",
                "letprofile=&reread;",
                "dunce::canonicalize(&profile.cwd)",
                "fresh_spawn_release_session_id(",
                "self.control.provision(",
                "backend::build_command_spec(",
            ],
        );

        let prelaunch = squashed(
            include_str!("prelaunch.rs"),
            "pub(super) fn prelaunch(",
            "#[cfg(test)]",
        );
        assert_in_order(
            &prelaunch,
            &[
                "self.profiles.get(id)",
                // 답(막힘 포함)을 쥔 채 곧바로 대조한다 — 사이에서 풀면 바뀐 입력의 막힘이 그대로 나간다.
                "letchecked=self.launch.kits.check(&profile);matchself.profiles.launch_inputs_unchanged(id,&seen)",
                "Some(true)=>{returnsettle(caller,&seen,checked)",
            ],
        );

        let ensure = squashed(manager, "fn ensure_registered(", "fn register_for_spawn(");
        assert_in_order(
            &ensure,
            &[
                "self.profiles.get(profile.id).is_some()",
                "self.check_before_create(profile)?;",
                "self.lock_name_allocation()",
                ".upsert_preserving_hierarchy(&self.profiles,fresh)",
            ],
        );

        // 점검을 부르는 자리는 이 파일의 둘(넣기 전 점검 · 전처리)뿐 — 다른 자리에서 부르면 명부 사본 · 락 규율이
        //   갈린다.
        let production = manager.split("mod tests {").next().expect("운영 구획");
        assert!(!production.contains(".kits.check("));
        let own = include_str!("prelaunch.rs")
            .split("mod tests {")
            .next()
            .expect("운영 구획");
        assert_eq!(own.matches(".kits.check(").count(), 2);
    }

    /// ★재료 가드 자리를 소스에서 못 박는다★ — 무장은 준비 **앞**(준비 실패도 거둔다 — Err 경로는 정확히 한 번 ·
    /// 패닉은 운영에서 프로세스를 끝내므로(ADR-0292) 부팅 쓸기가 받친다 — 시험 · contain 안에서만 풀린다) · 준비는
    /// 표식 뒤 · 세션 id 비우기와 제어 채널 발급 앞 · 재료는 한 번 파생해 조립 · 띄우기에 같은 값으로 · 무장 해제는
    /// 세션이 명부에 오른 뒤이고 그 사이에 `?` 가 없다(insert 와 해제 사이 실패 = 주인 둘). 세션 등록 실패는 차례로
    /// 재는 시험이 닿지 않는다. 등록 뒤 거두는 자리가 reaper 하나인 것도 함께 본다.
    // ADR-0334
    // TRD S25 §1-5 (나) 4 · 6 · 7 · §5-2 4 (c2)
    #[test]
    fn the_kit_guard_is_armed_before_prepare_and_handed_off_after_the_session_in_source() {
        let manager = include_str!("../manager.rs");
        let spawn = squashed(
            manager,
            "fn spawn_agent_watching_link(",
            "pub fn activate_profile(",
        );
        assert_in_order(
            &spawn,
            &[
                "let(plan,plan_notices)=self.prelaunch(profile)?;",
                ".epoch_for_spawn(profile.id)",
                "letmutkit_guard=LaunchKitGuard::arm(self.launch.kits.clone(),profile.id,epoch);",
                ".kits.prepare(plan,profile.id,epoch)",
                "letreread=self.profiles.get(profile.id)",
                "fresh_spawn_release_session_id(",
                "self.control.provision(",
                "letartifacts=backend::materialize(&profile.command,&kit,&cwd,&*self.launch.fs)",
                "backend::build_command_spec(",
                "control_endpoint.clone(),&artifacts,);",
                "self.probe_spec(&spec,&artifacts)?;",
                "control_endpoint.as_ref(),&artifacts,)?;",
                "launch_lease:kit_guard.lease(),",
                "self.spawn_session(profile.id,spec,parts,epoch,seed_events,incarnation)?;",
                "kit_guard.disarm();",
            ],
        );
        // TRD S25 §5-2 4 (c2) ①
        let handoff = spawn
            .split("self.spawn_session(profile.id,spec,parts,epoch,seed_events,incarnation)?;")
            .nth(1)
            .and_then(|rest| rest.split("kit_guard.disarm();").next())
            .expect("세션 등록 ~ 무장 해제 구간");
        assert!(
            !handoff.contains('?'),
            "세션 등록과 재료 가드 해제 사이에 실패할 수 있는 단계가 있다: {handoff}"
        );
        assert!(
            !spawn.contains("&LaunchArtifacts::"),
            "준비가 낸 재료 말고 다른 재료를 넘긴다"
        );
        assert_eq!(
            spawn.matches("&artifacts").count(),
            3,
            "조립 · 탐침 · 띄우기 말고 재료를 넘기는 자리가 있다"
        );

        // 준비를 부르는 자리는 스폰 길 하나뿐이다.
        let production: String = manager
            .split("mod tests {")
            .next()
            .expect("운영 구획")
            .lines()
            .map(str::trim)
            .filter(|l| !l.starts_with("//"))
            .collect::<String>()
            .split_whitespace()
            .collect();
        assert_eq!(production.matches(".kits.prepare(").count(), 1);

        // TRD S25 §5-2 4 (c2) ② · ④: 세션 등록 뒤 거두는 자리는 reaper 하나다 — 이 crate 의 다른 운영 소스(kill · 실패
        //   정리 포함)는 거두지 않는다. 출처를 직접 부르는 자리는 가드 · 열쇠(`launch.rs`)뿐이고, 열쇠를 꺼내는 자리는
        //   reaper 하나다(열쇠는 복제되지 않고 거두기가 열쇠를 먹는다).
        let sources = production_sources();
        let callers = |needle: &str| -> Vec<(String, usize)> {
            sources
                .iter()
                .filter_map(|(file, body)| {
                    let n = body.matches(needle).count();
                    (n > 0).then(|| (file.clone(), n))
                })
                .collect()
        };
        assert_eq!(callers("kits.release("), vec![("launch.rs".to_string(), 2)]);
        assert_eq!(
            callers(".take_launch_lease()"),
            vec![("reaper.rs".to_string(), 1)]
        );
        // 열쇠를 만드는 자리는 스폰 길 하나뿐이다(두 번 만들면 한 재료를 두 주인이 거둔다).
        assert_eq!(
            callers("kit_guard.lease()"),
            vec![("manager.rs".to_string(), 1)]
        );
        // 수거 승자가 세션을 빼고 놓고 권한을 거둔 뒤, auto_restore 내리기 · 목록 통지 **뒤**에 거둔다(ADR-0019 「내리기가
        //   마지막」 을 회수가 붙들지 않게).
        let reap = squashed(
            include_str!("../reaper.rs"),
            "pub fn reap_one(",
            "pub fn decide(",
        );
        assert_in_order(
            &reap,
            &[
                "Some(s)ifs.epoch==msg.epoch=>sessions.remove(&msg.id),",
                "letlease=removed.take_launch_lease();",
                "drop(removed);",
                "self.control.revoke(msg.id,msg.epoch);",
                "apply_disposition(",
                "self.status_sink.agent_list_updated(agents);",
                "lease.release();",
            ],
        );
    }

    /// 이 crate 의 운영 소스 — (`src/` 아래 경로, 시험 모듈 앞까지에서 주석 줄을 빼고 공백을 지운 본문). 통째로 시험인
    /// 파일(`tests.rs` · `tests/` 아래)은 뺀다.
    fn production_sources() -> Vec<(String, String)> {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).expect("소스 폴더 읽기") {
                let path = entry.expect("소스 폴더 항목").path();
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if path.is_dir() {
                    if name != "tests" {
                        walk(&path, out);
                    }
                } else if name.ends_with(".rs") && name != "tests.rs" {
                    out.push(path);
                }
            }
        }
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        walk(&src, &mut files);
        assert!(
            files.iter().any(|f| f.ends_with("reaper.rs")),
            "소스 폴더를 못 훑었다: {}",
            src.display()
        );
        files
            .into_iter()
            .map(|file| {
                let text = std::fs::read_to_string(&file).expect("소스 읽기");
                let production = text.split("mod tests {").next().unwrap_or_default();
                let body = production
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.starts_with("//"))
                    .collect::<String>()
                    .split_whitespace()
                    .collect::<String>();
                let rel = file
                    .strip_prefix(&src)
                    .expect("src 아래")
                    .to_string_lossy()
                    .replace('\\', "/");
                (rel, body)
            })
            .collect()
    }
}
