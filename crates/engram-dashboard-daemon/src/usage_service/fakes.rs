//! A stand-in with which tests outside the service (connection layer · assembly) set up the usage service without a real CLI.
//!
//! ★Prober keys and policy are borrowed from the real prober★ — vendor literals are not put in tests either. The service attaches the real encoder.

use std::collections::BTreeSet;
use std::sync::{mpsc, Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use engram_dashboard_agent::usage::{
    ProbeChild, ProbeCommand, ProbeEnv, ProbeError, ProbeFailure, ProbeSpawner, UsageObservation,
    UsagePolicy, UsageProbe, UsageSource, UsageVendorKey, WindowObs,
};

use super::clock::{OsUsageClock, UsageClock};
use super::{OsProbeThreads, UsageParts, UsageService};

/// A spawner that launches no child — it is a failure if called.
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

/// A prober that reports it came in on every query, blocks until the test releases it, then returns one same 5-hour window.
struct GatedProbe {
    key: UsageVendorKey,
    policy: UsagePolicy,
    five_hour: WindowObs,
    entered: tokio::sync::mpsc::UnboundedSender<()>,
    release: Mutex<mpsc::Receiver<()>>,
}

/// The test-side handle of [`GatedProbe`] — `entered` rings once each time a query starts, and each `release` sent once
/// lets one query return a value. ★If dropped, queries that were blocked and ones that come later end as a timeout failure★.
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

/// A [`GatedProbe`] borrowing `real`'s key and policy — a prober is `'static` so one leaks each time it is called.
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

/// A service with the real encoder attached. The scheduler is not launched — the test drives the time.
pub(crate) fn service(
    probes: Vec<&'static dyn UsageProbe>,
    clock: Arc<dyn UsageClock>,
) -> Arc<UsageService> {
    UsageService::new(parts(probes, clock)).0
}

/// Same as [`service`] but the test sets the ⟳ reply wait cap — so a test asserting 「답이 아직 없다」 (there is no answer yet) during a blocked query does not
/// lean on the production cap (`REPLY_WAIT_MAX`) (on a slow runner, if that cap ended first it would fail falsely).
pub(crate) fn service_with_wait(
    probes: Vec<&'static dyn UsageProbe>,
    clock: Arc<dyn UsageClock>,
    reply_wait: Duration,
) -> Arc<UsageService> {
    UsageService::build(parts(probes, clock), reply_wait).0
}

fn parts(probes: Vec<&'static dyn UsageProbe>, clock: Arc<dyn UsageClock>) -> UsageParts {
    UsageParts {
        probes,
        spawner: Arc::new(NoChildren),
        scratch_root: std::env::temp_dir(),
        threads: Arc::new(OsProbeThreads),
        clock,
        encoder: Arc::new(crate::agent_conn::UsageEventEncoder),
    }
}

/// A service with no cells — for assembly in tests that do not look at usage.
pub(crate) fn idle_service() -> Arc<UsageService> {
    service(Vec::new(), Arc::new(OsUsageClock::new()))
}

/// The union of the subscription registry — the vendors the scheduler looks after.
pub(crate) fn union(service: &UsageService) -> BTreeSet<UsageVendorKey> {
    service.watch.union()
}
