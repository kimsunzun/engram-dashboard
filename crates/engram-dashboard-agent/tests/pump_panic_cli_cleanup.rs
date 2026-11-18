//! CLI cleanup after a contained pump panic — per pump kind (PTY · stdio · codex reader), raise a panic inside the contained pump of an agent
//! launched with the operating manager and measure whether the existing cleanup path alone (`finish` → `Failed` → reaper removes the
//! session → last `Arc` drop → link drop → group owner drop → Job `KILL_ON_JOB_CLOSE`), with no explicit termination, makes the whole CLI
//! tree (root `cmd /c`, the CLI, the CLI's grandchild) disappear within the deadline.
//!
//! Entry = `AgentManager::spawn_agent(Fresh)`. Panics inside the activation entry's decision window (`early_activation_verdict` ·
//! `link_activation_verdict`) and during the codex handshake are not measured.
//!
//! ★The test turns nothing off until the decision is over★ — no kill · shutdown call; `Cleanup` runs only in the drop after the decision.
//! Otherwise something the test turned off could be the ground for passing.
//!
//! The fake CLI is a PowerShell script (no real claude · codex).
//!
//! Windows only: the measured end link (Job) and the tree-viewing means (`engram_dashboard_platform::process`) exist only there. Elsewhere
//! the group owner is a no-op and the CLI likely remains (not measured — TRD S21-panic-policy §7 「accepted residue」); the `#[ignore]`
//!   marker exposes that gap.
// ADR-0292

#[cfg(not(windows))]
#[test]
#[ignore = "an OS without a group (Job) — the link that would end the CLI after a contained pump panic is a no-op, so there is nothing to measure (unmeasured residue = TRD S21-panic-policy §7)"]
fn the_cli_tree_cleanup_after_a_contained_pump_panic_is_unmeasured_off_windows() {}

#[cfg(windows)]
mod on_windows {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use uuid::Uuid;

    use engram_dashboard_agent::manager::AgentManager;
    use engram_dashboard_agent::persistence::{FilePresetStore, FileProfileStore};
    use engram_dashboard_agent::preset::PresetRegistry;
    use engram_dashboard_agent::profile::{
        AgentCommand, AgentOutputFormat, AgentProfile, ProfileRegistry, SpawnMode,
    };
    use engram_dashboard_agent::session_tracker::{SessionTracker, TrackerConfig};
    use engram_dashboard_agent::types::{
        AgentId, AgentInfo, AgentStatus, OutputFrame, OutputSink, SinkError, SinkId, StatusSink,
    };
    use engram_dashboard_base::panic::is_contained;
    use engram_dashboard_base::testing::wait_until;
    use engram_dashboard_platform::process::{
        pid_alive, process_creation_time, process_parent_table, process_start, subtree,
        terminate_if_same, ProcessIdentity, ProcessStart,
    };
    use engram_dashboard_platform::testing::is_ping;

    /// Bound from the panic until the whole tree is gone — the budget the explicit kill path gives the pump join (`AgentManager::kill_agent`'s
    /// `session.kill(5 s)` · ADR-0001). Decision A claims 「it ends through the existing cleanup path」, so that path's budget is the
    /// yardstick, not a value fitted to this measurement (ADR-0038). The chain has no polling step.
    const BOUND: Duration = Duration::from_secs(5);

    /// Deadline telling apart an experiment that did not stand (the fake failed to launch · no panic) and for watching an overrun tree — not a
    /// measurement bound. PowerShell startup time was not measured.
    const SETUP: Duration = Duration::from_secs(30);

    const POLL: Duration = Duration::from_millis(10);

    /// The deadline for retrying the test folder deletion (`Scratch` drop) — a cleanup job unrelated to the decision, and an unmeasured value.
    const DELETE_RETRY: Duration = Duration::from_secs(5);

    const PANIC_MARK: &str = "U4: subscriber panic inside the contained pump";

    /// The fake CLI. It launches a grandchild (a long `ping`), announces its own and the grandchild's identity (PID · start time) in a
    /// marker file, answers the two handshake requests if codex, then emits one line every 200 ms forever (so the pump emits after any
    ///   subscription · arming).
    /// ★stdout is raw bytes★ — `Write-Output` encoding · BOM can make the first line not JSON (as the fake app-server in
    ///   `backend/codex/mod.rs`).
    /// ★The marker is written by a rename **after** launching the grandchild★ — when the test reads it, the content is complete. If the marker
    /// cannot be left, it turns off the grandchild and ends (an unknown-identity grandchild would live for an hour).
    /// ★Start times come from each process's own handle★, so the marker identity stays theirs after PID reuse. `StartTime` is the creation
    /// FILETIME in local time and `ToFileTimeUtc` converts back (same 100 ns tick); in the DST-overlap hour it can be off — the
    /// cross-check assertion goes red and cleanup skips the mismatched identity.
    /// ★The grandchild has its own hidden console and no pipes★ — so its disappearance is explained only by closing the group (Job) (the control
    /// group below measures this). `Send` swallows write failures so the CLI does not die on its own after the pump stops reading.
    const FAKE_CLI_PS1: &str = r#"
param([string]$Mode)
$ErrorActionPreference = 'Stop'
$so = [Console]::OpenStandardOutput()
function Send([string]$s) {
  $b = [Text.Encoding]::ASCII.GetBytes($s + "`n")
  try { $so.Write($b, 0, $b.Length); $so.Flush() } catch { }
}
$psi = New-Object Diagnostics.ProcessStartInfo
$psi.FileName = Join-Path $env:SystemRoot 'System32\PING.EXE'
$psi.Arguments = '-n 3600 127.0.0.1'
$psi.UseShellExecute = $false
$psi.CreateNoWindow = $true
$sleeper = [Diagnostics.Process]::Start($psi)
try {
  $self = [Diagnostics.Process]::GetCurrentProcess()
  $marker = Join-Path $PSScriptRoot 'cli.pid'
  $identities = "$($self.Id) $($self.StartTime.ToFileTimeUtc()) $($sleeper.Id) $($sleeper.StartTime.ToFileTimeUtc())"
  [IO.File]::WriteAllText("$marker.tmp", $identities)
  [IO.File]::Move("$marker.tmp", $marker)
} catch {
  try { $sleeper.Kill() } catch { }
  exit 1
}
if ($Mode -eq 'codex') {
  while ($null -ne ($line = [Console]::In.ReadLine())) {
    if ($line -match '"id":(-?\d+)') {
      $rid = $Matches[1]
      if ($line -match '"method":"initialize"') {
        Send ('{"id":' + $rid + ',"result":{"codexHome":"h","platformFamily":"windows","platformOs":"windows","userAgent":"engram-fake/0"}}')
      } elseif ($line -match '"method":"thread/start"') {
        Send ('{"id":' + $rid + ',"result":{"thread":{"id":"u4-fake-thread","cliVersion":"0.0.0-fake"}}}')
        break
      }
    }
  }
}
while ($true) {
  Send 'TICK_PLACEHOLDER'
  Start-Sleep -Milliseconds 200
}
"#;

    #[derive(Clone, Copy)]
    enum Pump {
        Pty,
        Stdio,
        Codex,
    }

    impl Pump {
        fn tag(self) -> &'static str {
            match self {
                Pump::Pty => "pty",
                Pump::Stdio => "stdio",
                Pump::Codex => "codex",
            }
        }

        /// A line link's decoder turns into an event — other lines make no pump emit (claude decoder drops non-JSON, codex reader drops
        ///   non-notifications).
        fn tick(self) -> &'static str {
            match self {
                Pump::Pty => "tick",
                Pump::Stdio => {
                    r#"{"type":"assistant","message":{"id":"u4-tick","content":[{"type":"text","text":"tick"}]}}"#
                }
                Pump::Codex => {
                    r#"{"method":"error","params":{"error":{"message":"tick"},"willRetry":false}}"#
                }
            }
        }

        /// claude · codex: the backend fixes the name as `cmd.exe /c claude|codex`, so the shim must have that name.
        fn shim(self) -> &'static str {
            match self {
                Pump::Pty => "cli.cmd",
                Pump::Stdio => "claude.cmd",
                Pump::Codex => "codex.cmd",
            }
        }

        fn profile(self, cli_dir: &Path) -> AgentProfile {
            // ★The child's PATH and working folder hold only the fakes★ — cmd finds `claude` · `codex` only there, so the real CLI is
            //   unreachable (as `claude_profile_that_cannot_launch` in the `manager.rs` tests). PTY calls the shim by absolute path, so
            //   PATH stays untouched — emptied, portable-pty cannot find `cmd.exe`.
            let only_fakes = vec![("PATH".to_string(), cli_dir.to_string_lossy().into_owned())];
            let (command, env) = match self {
                Pump::Pty => (
                    AgentCommand::Shell {
                        program: "cmd.exe".into(),
                        args: vec![
                            "/c".into(),
                            cli_dir.join(self.shim()).to_string_lossy().into_owned(),
                        ],
                    },
                    vec![],
                ),
                Pump::Stdio => (
                    AgentCommand::Claude {
                        extra_args: vec![],
                        output_format: AgentOutputFormat::StreamJson,
                    },
                    only_fakes,
                ),
                Pump::Codex => (
                    AgentCommand::Codex {
                        extra_args: vec![],
                        output_format: AgentOutputFormat::StreamJson,
                    },
                    only_fakes,
                ),
            };
            AgentProfile::new(
                format!("u4-{}", self.tag()),
                command,
                cli_dir.to_path_buf(),
                env,
                false,
            )
        }
    }

    struct Scratch {
        root: PathBuf,
        cli: PathBuf,
        store: PathBuf,
    }

    impl Scratch {
        fn new(pump: Pump) -> Self {
            let root = std::env::temp_dir().join(format!(
                "engram-u4-pump-panic-{}-{}",
                pump.tag(),
                Uuid::new_v4()
            ));
            let cli = root.join("cli");
            let store = root.join("store");
            std::fs::create_dir_all(&cli).expect("fake folder");
            std::fs::create_dir_all(&store).expect("store folder");
            std::fs::write(
                cli.join("fake_cli.ps1"),
                FAKE_CLI_PS1.replace("TICK_PLACEHOLDER", pump.tick()),
            )
            .expect("fake script");
            std::fs::write(
                cli.join(pump.shim()),
                format!(
                    "@\"%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"%~dp0fake_cli.ps1\" {}\r\n",
                    pump.tag()
                ),
            )
            .expect("shim");
            Self { root, cli, store }
        }
    }

    impl Drop for Scratch {
        /// A just-ended process may still hold the working folder and fail deletion (measured once) — retry briefly.
        fn drop(&mut self) {
            wait_until(DELETE_RETRY, || {
                std::fs::remove_dir_all(&self.root).is_ok() || !self.root.exists()
            });
        }
    }

    #[derive(Default)]
    struct Seen {
        statuses: Mutex<Vec<(AgentId, AgentStatus)>>,
        lists: Mutex<Vec<(Instant, Vec<AgentId>)>>,
    }

    impl Seen {
        fn statuses_of(&self, id: AgentId) -> Vec<AgentStatus> {
            self.statuses
                .lock()
                .unwrap()
                .iter()
                .filter(|(a, _)| *a == id)
                .map(|(_, s)| s.clone())
                .collect()
        }

        /// The list notification sent after the reaper removed the session (`ReaperDeps::reap_one`).
        fn dropped_from_list_since(&self, id: AgentId, since: Instant) -> bool {
            self.lists
                .lock()
                .unwrap()
                .iter()
                .any(|(at, ids)| *at >= since && !ids.contains(&id))
        }
    }

    impl StatusSink for Seen {
        fn status_changed(&self, id: AgentId, status: AgentStatus, _epoch: u32) {
            self.statuses.lock().unwrap().push((id, status));
        }
        fn agent_list_updated(&self, agents: Vec<AgentInfo>) {
            self.lists
                .lock()
                .unwrap()
                .push((Instant::now(), agents.into_iter().map(|a| a.id).collect()));
        }
    }

    /// Panics on the first frame received **inside the mark** after arming. Before arming · outside the mark (the test thread replaying at
    /// subscription) it only receives — so the panic comes after the tree was captured, and only inside the pump's `contain`.
    struct PanicOnArm {
        id: SinkId,
        armed: AtomicBool,
        fired: AtomicBool,
        panicked_at: Mutex<Option<Instant>>,
    }

    impl PanicOnArm {
        fn new() -> Self {
            Self {
                id: Uuid::new_v4(),
                armed: AtomicBool::new(false),
                fired: AtomicBool::new(false),
                panicked_at: Mutex::new(None),
            }
        }

        fn panicked_at(&self) -> Option<Instant> {
            *self.panicked_at.lock().unwrap()
        }
    }

    impl OutputSink for PanicOnArm {
        fn send(&self, _frame: OutputFrame<'_>) -> Result<(), SinkError> {
            if !self.armed.load(Ordering::SeqCst) || !is_contained() {
                return Ok(());
            }
            if self.fired.swap(true, Ordering::SeqCst) {
                return Ok(());
            }
            *self.panicked_at.lock().unwrap() = Some(Instant::now());
            panic!("{PANIC_MARK}");
        }
        fn sink_id(&self) -> SinkId {
            self.id
        }
    }

    /// After the decision (or a mid-way fall) this withdraws only what this test launched. ★Runs only in drop★ — during the measurement it
    /// would make the ground for passing.
    /// ★Nothing is turned off by PID alone★ — a live session by explicit kill; a held root by capturing what is below it, then through its
    /// handle; an identity only after `terminate_if_same` confirmed the start time via the same handle. Unreadable identities are left and
    ///   the fact is written down.
    struct Cleanup<'a> {
        session: Option<(&'a AgentManager, AgentId)>,
        pipe_root: Option<std::process::Child>,
        pty_root: Option<Box<dyn portable_pty::Child + Send + Sync>>,
        tree: Vec<ProcessIdentity>,
    }

    impl<'a> Cleanup<'a> {
        fn new(session: Option<(&'a AgentManager, AgentId)>) -> Self {
            Self {
                session,
                pipe_root: None,
                pty_root: None,
                tree: Vec::new(),
            }
        }

        fn hold(&mut self, p: ProcessIdentity) {
            if !self.tree.contains(&p) {
                self.tree.push(p);
            }
        }
    }

    impl Drop for Cleanup<'_> {
        fn drop(&mut self) {
            if let Some((manager, id)) = self.session {
                let _ = manager.kill_agent(id);
            }
            if let Some(root) = self.pipe_root.as_mut() {
                end_tree_under(root.id());
                let _ = root.kill();
                let _ = root.wait();
            }
            if let Some(root) = self.pty_root.as_mut() {
                if let Some(pid) = root.process_id() {
                    end_tree_under(pid);
                }
                let _ = root.kill();
            }
            for p in &self.tree {
                end(p);
            }
        }
    }

    /// Captures what is below by the start time read while the held root handle pins that PID, then turns them off one by one — if the
    /// marker never arrived the list is empty and the root alone would leave the CLI and grandchild. The pin makes a child naming that PID
    ///   as parent really its child.
    fn end_tree_under(root_pid: u32) {
        if let Some(root_start) = process_creation_time(root_pid) {
            for p in subtree(root_pid, root_start) {
                end(&p);
            }
        }
    }

    fn end(p: &ProcessIdentity) {
        if matches!(process_start(p.pid), ProcessStart::Unknown) {
            eprintln!("[U4 cleanup] could not read the identity of PID {}, so it is left without being turned off", p.pid);
        } else if let Err(e) = terminate_if_same(p.pid, p.start_time) {
            eprintln!(
                "[U4 cleanup] could not confirm PID {}, so it is left without being turned off: {e}",
                p.pid
            );
        }
    }

    /// The identities below the root (the `cmd.exe` this test launched) at **capture time** — later-born descendants are not included (the
    ///   fake launches one grandchild only).
    struct Tree {
        root: ProcessIdentity,
        cli: ProcessIdentity,
        grandchild: ProcessIdentity,
        all: Vec<ProcessIdentity>,
    }

    impl Tree {
        fn role(&self, p: &ProcessIdentity) -> &'static str {
            if *p == self.root {
                "root cmd"
            } else if *p == self.cli {
                "CLI powershell"
            } else if *p == self.grandchild {
                "grandchild ping"
            } else {
                "other descendant"
            }
        }
    }

    fn read_identities(path: &Path) -> (ProcessIdentity, ProcessIdentity) {
        let text = std::fs::read_to_string(path).expect("read marker");
        let fields: Vec<u64> = text
            .split_whitespace()
            .filter_map(|s| s.parse().ok())
            .collect();
        let [cli_pid, cli_start, grandchild_pid, grandchild_start] = fields[..] else {
            panic!("the marker must be `<CLI PID> <start time> <grandchild PID> <start time>`: {text:?}");
        };
        let identity = |pid: u64, start_time: u64| ProcessIdentity {
            pid: u32::try_from(pid).expect("PID is u32"),
            start_time,
        };
        (
            identity(cli_pid, cli_start),
            identity(grandchild_pid, grandchild_start),
        )
    }

    /// Finds and captures the CLI's tree via the marker. Its identities go into `cleanup` **before anything else that can fall over**, so
    /// the grandchild is withdrawn even if the check falls.
    fn observe_tree(cli_dir: &Path, cleanup: &mut Cleanup) -> Tree {
        let marker = cli_dir.join("cli.pid");
        assert!(
            wait_until(SETUP, || marker.exists()),
            "the fake CLI left no marker within {SETUP:?} — powershell failed to launch or could not find the shim (experiment did not stand)"
        );
        let (reported_cli, reported_grandchild) = read_identities(&marker);
        cleanup.hold(reported_cli);
        cleanup.hold(reported_grandchild);
        let (cli, grandchild) = (reported_cli.pid, reported_grandchild.pid);

        let table = process_parent_table().expect("process table");
        let parent_of = |pid: u32| table.iter().find(|(p, _)| *p == pid).map(|(_, pp)| *pp);
        let root = parent_of(cli).expect("parent of the CLI");
        assert_eq!(
            parent_of(root),
            Some(std::process::id()),
            "the root must be one this test process launched (the manager launches it inside the process)"
        );
        assert_eq!(
            parent_of(grandchild),
            Some(cli),
            "the grandchild must be a child of the CLI"
        );
        assert!(is_ping(grandchild), "the grandchild is not ping");
        let root_start = process_creation_time(root).expect("root start time");
        let all = subtree(root, root_start);
        for p in &all {
            cleanup.hold(*p);
        }
        for reported in [reported_cli, reported_grandchild] {
            assert!(
                all.contains(&reported),
                "the identity {reported:?} the fake announced is not in the root tree as is — the start-time scale is off or that PID is another \
                 process: {all:?}"
            );
        }
        Tree {
            root: all[0],
            cli: reported_cli,
            grandchild: reported_grandchild,
            all,
        }
    }

    /// That identity's process is still running. An ended process's object remains while someone holds a handle (`ProcessStart` doc), so
    /// besides the start-time cross-check it checks exit. Unreadable (`Unknown`) counts as running — a false failure rather than a false pass.
    fn running(p: &ProcessIdentity) -> bool {
        match process_start(p.pid) {
            ProcessStart::Known(start) if start == p.start_time => pid_alive(p.pid),
            ProcessStart::Known(_) | ProcessStart::Gone => false,
            ProcessStart::Unknown => true,
        }
    }

    fn manager(store: &Path, seen: Arc<Seen>) -> AgentManager {
        let profiles = Arc::new(ProfileRegistry::new(Arc::new(FileProfileStore::new(
            store.join("profiles"),
        ))));
        let presets = Arc::new(PresetRegistry::new(Arc::new(FilePresetStore::new(
            store.join("presets"),
        ))));
        let tracker = Arc::new(SessionTracker::new(
            TrackerConfig {
                enabled: false,
                poll_interval: Duration::from_secs(1),
            },
            Arc::new(|_, _| {}),
        ));
        AgentManager::new(seen, profiles, presets, tracker)
    }

    fn a_contained_pump_panic_ends_the_whole_cli_tree(pump: Pump) {
        let scratch = Scratch::new(pump);
        let seen = Arc::new(Seen::default());
        let manager = manager(&scratch.store, seen.clone());
        let profile = pump.profile(&scratch.cli);
        let id = profile.id;
        let mut cleanup = Cleanup::new(Some((&manager, id)));
        manager
            .spawn_agent(&profile, SpawnMode::Fresh)
            .expect("spawn")
            .into_started()
            .expect("this call launched it");

        let trigger = Arc::new(PanicOnArm::new());
        manager.subscribe(id, trigger.clone()).expect("subscribe");

        let tree = observe_tree(&scratch.cli, &mut cleanup);
        trigger.armed.store(true, Ordering::SeqCst);
        assert!(
            wait_until(SETUP, || trigger.panicked_at().is_some()),
            "the contained pump did not emit within {SETUP:?} after arming (experiment did not stand)"
        );
        let panicked_at = trigger.panicked_at().expect("just saw it");

        let mut gone_after: Vec<Option<Duration>> = vec![None; tree.all.len()];
        let mut dropped_after = None;
        loop {
            let now = Instant::now();
            for (p, gone) in tree.all.iter().zip(gone_after.iter_mut()) {
                if gone.is_none() && !running(p) {
                    *gone = Some(now - panicked_at);
                }
            }
            if dropped_after.is_none() && seen.dropped_from_list_since(id, panicked_at) {
                dropped_after = Some(now - panicked_at);
            }
            let settled = gone_after.iter().all(Option::is_some) && dropped_after.is_some();
            if settled || now - panicked_at >= SETUP {
                break;
            }
            std::thread::sleep(POLL);
        }
        let tree_gone_after = gone_after
            .iter()
            .copied()
            .collect::<Option<Vec<_>>>()
            .and_then(|all| all.into_iter().max());
        let each: Vec<String> = tree
            .all
            .iter()
            .zip(&gone_after)
            .map(|(p, gone)| format!("{} {gone:?}", tree.role(p)))
            .collect();
        eprintln!(
            "[U4 {}] panic → dropped from the list {dropped_after:?} · whole tree gone {tree_gone_after:?} (bound {BOUND:?}) · {each:?}",
            pump.tag()
        );

        let statuses = seen.statuses_of(id);
        assert!(
            !statuses
                .iter()
                .any(|s| matches!(s, AgentStatus::Exiting | AgentStatus::Killed)),
            "took the explicit termination path — this test does not measure that path: {statuses:?}"
        );
        match statuses
            .iter()
            .filter(|s| !s.is_live())
            .collect::<Vec<_>>()
            .as_slice()
        {
            [AgentStatus::Failed { message }] => {
                assert!(message.starts_with("pump panicked: "), "{message}");
                assert!(message.contains(PANIC_MARK), "{message}");
            }
            other => panic!(
                "a contained pump panic must be the end state Failed exactly once: {other:?}"
            ),
        }

        assert!(
            dropped_after.is_some(),
            "the reaper did not remove the session within {SETUP:?} after the panic"
        );
        assert!(
            !manager.list_agents().iter().any(|a| a.id == id),
            "the session remained in the registry"
        );

        let survivors: Vec<String> = tree
            .all
            .iter()
            .zip(&gone_after)
            .filter(|(_, gone)| gone.is_none_or(|d| d > BOUND))
            .map(|(p, gone)| {
                let when = match gone {
                    Some(d) => format!("{d:?}"),
                    None => format!("not gone within {SETUP:?}"),
                };
                format!("{} (PID {}) — gone at {when}", tree.role(p), p.pid)
            })
            .collect();
        assert!(
            survivors.is_empty(),
            "decision A did not hold — after a contained pump panic the CLI tree remained at {BOUND:?} (a matter for the user to decide · do not move on to B): {survivors:#?}"
        );
    }

    #[test]
    fn a_contained_pty_pump_panic_ends_the_whole_cli_tree() {
        a_contained_pump_panic_ends_the_whole_cli_tree(Pump::Pty);
    }

    #[test]
    fn a_contained_stdio_pump_panic_ends_the_whole_cli_tree() {
        a_contained_pump_panic_ends_the_whole_cli_tree(Pump::Stdio);
    }

    #[test]
    fn a_contained_codex_reader_panic_ends_the_whole_cli_tree() {
        a_contained_pump_panic_ends_the_whole_cli_tree(Pump::Codex);
    }

    // ── Control group: without a group (Job), the grandchild lives past the bound ──
    //
    // Releasing everything else the contained panic path releases (output read end · stdin · root · PTY console) without a group still
    // leaves the grandchild alive for the bound — so in the test above its disappearance is explained only by closing the group. Goes red
    //   if the fake's grandchild dies on its own.

    fn assert_the_grandchild_outlives_the_bound(tree: &Tree) {
        std::thread::sleep(BOUND);
        let alive: Vec<&str> = tree
            .all
            .iter()
            .filter(|p| running(p))
            .map(|p| tree.role(p))
            .collect();
        eprintln!("[U4 control group] alive after {BOUND:?} without a group: {alive:?}");
        assert!(
            running(&tree.grandchild),
            "the grandchild died within {BOUND:?} even without a group — the grandchild's disappearance in the test above is no evidence of closing the group"
        );
    }

    /// The branch where the fake cannot leave the marker — its place is blocked in advance so the rename fails; the fake ends in failure
    /// and turns off the grandchild. Evidence of reaching the branch = `cli.pid.tmp` written before the rename; the grandchild is measured
    /// by that identity (had powershell fallen earlier there would be no grandchild and 「no grandchild left」 would pass falsely).
    #[test]
    fn a_fake_cli_that_cannot_write_its_marker_ends_its_grandchild() {
        use std::process::{Command, Stdio};

        let scratch = Scratch::new(Pump::Pty);
        std::fs::write(scratch.cli.join("cli.pid"), "").expect("block the marker's place");
        let mut cleanup = Cleanup::new(None);
        let powershell = PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot"))
            .join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
        let mut cmd = Command::new(powershell);
        cmd.args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(scratch.cli.join("fake_cli.ps1"))
        .arg(Pump::Pty.tag())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
        engram_dashboard_platform::spawn::hide_console_window(&mut cmd);
        let cli = cleanup
            .pipe_root
            .insert(cmd.spawn().expect("launch the fake"));
        let pid = cli.id();
        let start = process_creation_time(pid).expect("fake start time");

        assert!(
            wait_until(SETUP, || matches!(cli.try_wait(), Ok(Some(_)))),
            "the fake that could not leave the marker did not end within {SETUP:?}"
        );
        let status = cli.wait().expect("exit status");
        assert!(
            !status.success(),
            "it could not leave the marker yet ended in success: {status:?}"
        );
        let written = scratch.cli.join("cli.pid.tmp");
        assert!(
            written.exists(),
            "the fake ended before reaching the rename failure (experiment did not stand)"
        );
        let (reported_cli, grandchild) = read_identities(&written);
        cleanup.hold(grandchild);
        // If the scale is off, the grandchild's start time is off too, and a live grandchild reads as 「gone」.
        assert_eq!(
            reported_cli,
            ProcessIdentity {
                pid,
                start_time: start
            },
            "the identity the fake wrote for itself differs from the real one — the start-time scale is off"
        );
        assert!(
            wait_until(SETUP, || !running(&grandchild)),
            "the grandchild of the fake that could not leave the marker remained: {grandchild:?}"
        );
    }

    #[test]
    fn without_a_job_the_grandchild_outlives_lost_pipes_and_a_dead_root() {
        use std::process::{Command, Stdio};

        let scratch = Scratch::new(Pump::Stdio);
        let mut cleanup = Cleanup::new(None);
        let mut cmd = Command::new("cmd.exe");
        cmd.arg("/c")
            .arg(scratch.cli.join(Pump::Stdio.shim()))
            .current_dir(&scratch.cli)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        engram_dashboard_platform::spawn::hide_console_window(&mut cmd);
        cleanup.pipe_root = Some(cmd.spawn().expect("launch the root"));
        let tree = observe_tree(&scratch.cli, &mut cleanup);

        let root = cleanup.pipe_root.as_mut().expect("just held it");
        drop(root.stdout.take());
        drop(root.stdin.take());
        drop(root.stderr.take());
        let _ = root.kill();
        let _ = root.wait();

        assert_the_grandchild_outlives_the_bound(&tree);
    }

    #[test]
    fn without_a_job_the_grandchild_outlives_a_closed_pseudoconsole() {
        use portable_pty::{native_pty_system, CommandBuilder, PtySize};

        let scratch = Scratch::new(Pump::Pty);
        let mut cleanup = Cleanup::new(None);
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty");
        let mut cmd = CommandBuilder::new("cmd.exe");
        cmd.arg("/c");
        cmd.arg(scratch.cli.join(Pump::Pty.shim()));
        cmd.cwd(&scratch.cli);
        cleanup.pty_root = Some(pair.slave.spawn_command(cmd).expect("launch the root"));
        drop(pair.slave);
        let reader = pair.master.try_clone_reader().expect("read end");
        let tree = observe_tree(&scratch.cli, &mut cleanup);

        drop(reader);
        drop(pair.master);

        assert_the_grandchild_outlives_the_bound(&tree);
    }
}
