//! 서비스 밖(연결 계층·조립) 시험이 실 CLI 없이 사용량 서비스를 세우는 대역.
//!
//! ★조회기 키·정책은 실 조회기에서 빌린다★ — 시험에도 벤더 리터럴을 두지 않는다. 서비스는 실물 인코더를 단다.

use std::collections::BTreeSet;
use std::sync::{mpsc, Arc, Mutex, PoisonError};
use std::time::Instant;

use engram_dashboard_agent::usage::{
    ProbeChild, ProbeCommand, ProbeEnv, ProbeError, ProbeFailure, ProbeSpawner, UsageObservation,
    UsagePolicy, UsageProbe, UsageSource, UsageVendorKey, WindowObs,
};

use super::clock::{OsUsageClock, UsageClock};
use super::reject_store::MemRejectStore;
use super::{OsProbeThreads, UsageParts, UsageService};

/// 자식을 띄우지 않는 스포너 — 불리면 실패다.
pub(crate) struct NoChildren;

impl ProbeSpawner for NoChildren {
    fn spawn(
        &self,
        _cmd: &ProbeCommand,
        _deadline: Instant,
    ) -> Result<Box<dyn ProbeChild>, ProbeError> {
        Err(ProbeError::Spawn("시험 — 자식을 띄우지 않는다".to_owned()))
    }
}

/// 조회마다 들어왔다고 알리고 시험이 놓아 줄 때까지 막힌 뒤 같은 5시간 창 하나를 돌려주는 조회기.
struct GatedProbe {
    key: UsageVendorKey,
    policy: UsagePolicy,
    five_hour: WindowObs,
    entered: tokio::sync::mpsc::UnboundedSender<()>,
    release: Mutex<mpsc::Receiver<()>>,
}

/// [`GatedProbe`] 의 시험 쪽 손잡이 — `entered` 는 조회가 시작될 때마다 한 번 울리고, `release` 로 한 번 보낼 때마다
/// 조회 하나가 값을 돌려준다. ★놓으면(drop) 막혀 있던·뒤에 올 조회는 시한 초과 실패로 끝난다★.
pub(crate) struct ProbeGate {
    pub(crate) entered: tokio::sync::mpsc::UnboundedReceiver<()>,
    pub(crate) release: mpsc::Sender<()>,
}

impl UsageProbe for GatedProbe {
    fn key(&self) -> UsageVendorKey {
        self.key
    }

    fn policy(&self) -> UsagePolicy {
        self.policy
    }

    fn query(&self, _env: &ProbeEnv<'_>) -> Result<UsageObservation, ProbeFailure> {
        let _ = self.entered.send(());
        let released = self
            .release
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .recv();
        if released.is_err() {
            return Err(ProbeError::Timeout.into());
        }
        Ok(UsageObservation {
            vendor: self.key,
            five_hour: Some(self.five_hour),
            weekly: None,
            model_scoped: None,
            plan: None,
            source: UsageSource::Active,
            limits_unavailable: None,
        })
    }
}

/// `real` 의 키·정책을 빌린 [`GatedProbe`] — 조회기는 `'static` 이라 부를 때마다 하나씩 샌다.
pub(crate) fn gated_probe(
    real: &dyn UsageProbe,
    used_pct: f64,
    resets_at: i64,
) -> (&'static dyn UsageProbe, ProbeGate) {
    let (entered_tx, entered) = tokio::sync::mpsc::unbounded_channel();
    let (release, release_rx) = mpsc::channel();
    let probe: &'static dyn UsageProbe = Box::leak(Box::new(GatedProbe {
        key: real.key(),
        policy: real.policy(),
        five_hour: WindowObs {
            used_pct: Some(used_pct),
            resets_at: Some(resets_at),
        },
        entered: entered_tx,
        release: Mutex::new(release_rx),
    }));
    (probe, ProbeGate { entered, release })
}

/// 실물 인코더를 단 서비스. 스케줄러는 띄우지 않는다 — 시각은 시험이 몬다.
pub(crate) fn service(
    probes: Vec<&'static dyn UsageProbe>,
    clock: Arc<dyn UsageClock>,
) -> Arc<UsageService> {
    UsageService::new(UsageParts {
        probes,
        spawner: Arc::new(NoChildren),
        scratch_root: std::env::temp_dir(),
        threads: Arc::new(OsProbeThreads),
        rejects: Arc::new(MemRejectStore::new()),
        clock,
        encoder: Arc::new(crate::agent_conn::UsageEventEncoder),
    })
    .0
}

/// 칸이 없는 서비스 — 사용량을 보지 않는 시험의 조립용.
pub(crate) fn idle_service() -> Arc<UsageService> {
    service(Vec::new(), Arc::new(OsUsageClock::new()))
}

/// 구독 명부의 합집합 — 스케줄러가 돌볼 벤더.
pub(crate) fn union(service: &UsageService) -> BTreeSet<UsageVendorKey> {
    service.watch.union()
}
