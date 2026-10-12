//! 가둔 펌프 패닉 뒤 CLI 정리 — 펌프 종류마다(PTY · stdio · codex 리더) 운영 manager 로 띄운 에이전트의 가둔 펌프
//! 안에서 panic 을 내고, 명시 종료 없이 기존 정리 경로(`finish` → `Failed` → reaper 가 세션을 뺀다 → 마지막 `Arc`
//! drop → 통로 drop → 무리 주인 drop → Job `KILL_ON_JOB_CLOSE`)만으로 CLI 프로세스 트리 전체가 — 뿌리 `cmd /c` 만이
//! 아니라 그 아래 CLI 와 CLI 가 띄운 손자까지 — 시한 안에 사라지는지 잰다.
//!
//! 입구는 `AgentManager::spawn_agent(Fresh)` 다. 활성화 입구의 판정 창(`early_activation_verdict` ·
//! `link_activation_verdict`) 안에서 난 panic 과 codex 핸드셰이크 도중에 난 panic 은 재지 않는다.
//!
//! ★시험은 판정이 끝날 때까지 아무것도 끄지 않는다★ — kill · shutdown 을 부르지 않고, 정리(`Cleanup`)는 판정 뒤
//! drop 에서만 돈다. 시험이 끈 것이 통과의 근거가 되면 이 시험은 아무것도 안 잰다.
//!
//! 가짜 CLI 는 PowerShell 스크립트다 — 실 claude · codex 를 띄우지 않는다.
//!
//! Windows 전용: 재는 끝 고리(Job)와 트리를 보는 수단(`engram_dashboard_platform::process`)이 Windows 에만 있다. 다른
//! OS 에서는 무리 주인이 무동작이라 CLI 가 남을 가능성이 높다(미측정 — TRD S21-panic-policy §7 「받는 잔여」) — 아래
//! `#[ignore]` 표지가 그 공백을 드러낸다.
// ADR-0292

#[cfg(not(windows))]
#[test]
#[ignore = "무리(Job)가 없는 OS — 가둔 펌프 패닉 뒤 CLI 를 끝낼 고리가 무동작이라 잴 것이 없다(미측정 잔여 = TRD S21-panic-policy §7)"]
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
        ProcessIdentity, ProcessStart,
    };
    use engram_dashboard_platform::testing::{is_ping, terminate_if_same};

    /// panic 에서 트리가 다 사라질 때까지의 상한 — 명시 kill 경로가 펌프 합류에 주는 예산(`AgentManager::kill_agent`
    /// 의 `session.kill(5 s)` · ADR-0001)과 같다. 결정 A 는 「기존 정리 경로로 끝난다」는 주장이라 그 경로의 기존
    /// 예산을 잣대로 쓴다 — 이 측정에 맞춰 고른 값이 아니다(ADR-0038). 사슬에 폴링 걸음은 없다(reaper 는 채널 수신).
    const BOUND: Duration = Duration::from_secs(5);

    /// 실험이 서지 않은 것(가짜가 못 떴다 · panic 이 안 났다)을 가르는 시한이고, 상한을 넘긴 트리를 계속 지켜보는
    /// 시한이다 — 측정 상한이 아니다. PowerShell 기동 시간은 재지 않았다.
    const SETUP: Duration = Duration::from_secs(30);

    /// 잰 시간의 눈금.
    const POLL: Duration = Duration::from_millis(10);

    /// 시험 폴더 지우기를 다시 하는 시한(`Scratch` drop) — 판정과 무관한 정리 몫이고 재지 않은 값이다.
    const DELETE_RETRY: Duration = Duration::from_secs(5);

    const PANIC_MARK: &str = "U4: 가둔 펌프 안의 구독자 panic";

    /// 가짜 CLI. 손자(긴 `ping`)를 먼저 띄우고 자기와 손자의 신원(PID · 시작시각)을 표식 파일로 알린 뒤, codex 면
    /// 핸드셰이크 두 요청에 답하고, 한 줄을 200 ms 마다 끝없이 낸다 — 끝없이인 것은 구독 · 무장이 언제 서든 그 뒤에
    /// 펌프가 emit 하게 하려는 것이다.
    /// ★stdout 은 raw 바이트로 쓴다★ — `Write-Output` 의 인코딩 · BOM 이 첫 줄을 JSON 이 아니게 만들 수 있다(같은
    /// 사유 = `backend/codex/mod.rs` 의 가짜 app-server).
    /// ★표식은 손자를 띄운 **뒤** 이름 바꾸기로 쓴다★ — 시험이 표식을 읽는 순간 손자가 이미 있고 내용이 다 차 있다.
    /// 표식을 못 남기면 손자를 끄고 끝난다 — 아무도 그 신원을 모르는 채 CLI 가 죽으면 뿌리에서 찍어도 닿지 않아 손자가
    /// 한 시간 남는다.
    /// ★시작시각은 각자 쥔 손잡이로 읽는다★(자기 = 현재 프로세스 · 손자 = 띄우며 받은 `Process`) — 그래서 표식의 신원은
    /// 그 PID 가 나중에 남에게 넘어가도 이 프로세스들의 것이다. `StartTime` 은 생성 FILETIME 을 지역 시각으로 바꾼
    /// 값이고 `ToFileTimeUtc` 가 되돌린다(눈금 100 ns 로 같다). 일광 절약이 겹치는 한 시간에는 어긋날 수 있다 — 그때는
    /// 시험의 대조 단언이 빨개지고, 정리는 어긋난 신원을 끄지 않는다(시작시각이 달라서).
    /// ★손자는 제 (숨긴) 콘솔로 띄우고 파이프를 물리지 않는다★ — PTY 콘솔이 닫히거나 파이프가 끊겨서 죽을 길을 없앤다.
    /// 그래서 손자가 사라진 것은 무리(Job) 닫기 말고는 설명되지 않는다(아래 대조군이 잰다). `Send` 가 쓰기 실패를 삼키는
    /// 것은 펌프가 읽기를 멈춘 뒤 CLI 가 제풀에 죽지 않게 하려는 것이다.
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

        /// 그 통로의 decoder 가 이벤트로 옮기는 줄 — 옮기지 않는 줄로는 펌프가 emit 하지 않는다(claude decoder 는 JSON
        /// 아닌 줄을, codex 리더는 알림 아닌 줄을 버린다).
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

        /// claude · codex 는 backend 가 `cmd.exe /c claude|codex` 로 이름을 고정하므로 심(shim)이 그 이름이어야 한다.
        fn shim(self) -> &'static str {
            match self {
                Pump::Pty => "cli.cmd",
                Pump::Stdio => "claude.cmd",
                Pump::Codex => "codex.cmd",
            }
        }

        fn profile(self, cli_dir: &Path) -> AgentProfile {
            // ★자식 PATH 와 작업 폴더가 가짜만 든 폴더다★ — cmd 는 `claude` · `codex` 를 그 둘에서만 찾으므로 실 CLI 에
            //   닿을 길이 없다(같은 막기 = `manager.rs` 시험의 `claude_profile_that_cannot_launch`). PTY 는 심을 절대
            //   경로로 부르는 셸 에이전트라 PATH 를 건드리지 않는다 — 비우면 portable-pty 가 `cmd.exe` 부터 못 찾는다.
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

    /// 시험 하나의 폴더 — `cli` 는 가짜 셋(심 · 스크립트 · 표식)만 든다(그 폴더가 자식의 PATH 다).
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
            std::fs::create_dir_all(&cli).expect("가짜 폴더");
            std::fs::create_dir_all(&store).expect("저장 폴더");
            std::fs::write(
                cli.join("fake_cli.ps1"),
                FAKE_CLI_PS1.replace("TICK_PLACEHOLDER", pump.tick()),
            )
            .expect("가짜 스크립트");
            std::fs::write(
                cli.join(pump.shim()),
                format!(
                    "@\"%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"%~dp0fake_cli.ps1\" {}\r\n",
                    pump.tag()
                ),
            )
            .expect("심");
            Self { root, cli, store }
        }
    }

    impl Drop for Scratch {
        /// 막 끝난 프로세스가 작업 폴더를 아직 쥐고 있으면 지우기가 실패한다(실측 — 한 번 `cli` 폴더가 남았다) — 잠깐
        /// 다시 한다.
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

        /// panic 뒤의 목록 통지 중 그 에이전트가 빠진 것 — reaper 가 세션을 뺀 뒤에 내는 통지다(`ReaperDeps::reap_one`).
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

    /// 무장된 뒤 **표시 안에서** 처음 받은 프레임에서 panic 한다. 무장 전 · 표시 밖(구독 때 replay 를 넘기는 시험
    /// 스레드, 표시를 끈 자리)에서는 받기만 한다 — 그래서 panic 은 트리를 찍은 뒤에, 펌프의 `contain` 안에서만 난다.
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

    /// 판정 뒤(또는 실험이 도중에 넘어졌을 때) 이 시험이 띄운 것만 거둔다. ★drop 에서만 돈다★ — 측정 동안 돌면 통과의
    /// 근거를 시험이 만든다.
    /// ★PID 만 들고는 끄지 않는다★ — 산 세션은 명시 kill(무리째), 쥔 뿌리는 그 아래를 찍어 끈 뒤 그 손잡이로 끄고,
    /// 신원은 `terminate_if_same` 이 시작시각을 같은 손잡이로 확인한 뒤에만 끈다. 신원을 못 읽은 것은 끄지 않고 남긴
    /// 사실을 적는다.
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

    /// 쥔 뿌리 손잡이가 그 PID 를 붙들고 있는 동안 읽은 시작시각으로 그 아래를 찍어 하나씩 끈다 — 표식이 오기 전에
    /// 넘어졌으면 명단이 비어 있고, 뿌리만 끄면 CLI 와 손자가 남는다. 손잡이가 뿌리 PID 를 붙드므로 뿌리가 끝난 뒤에도
    /// 그 PID 를 부모로 단 자식은 진짜 그 뿌리의 자식이다.
    fn end_tree_under(root_pid: u32) {
        if let Some(root_start) = process_creation_time(root_pid) {
            for p in subtree(root_pid, root_start) {
                end(&p);
            }
        }
    }

    fn end(p: &ProcessIdentity) {
        if matches!(process_start(p.pid), ProcessStart::Unknown) {
            eprintln!("[U4 정리] PID {} 의 신원을 못 읽어 끄지 않고 남긴다", p.pid);
        } else if let Err(e) = terminate_if_same(p.pid, p.start_time) {
            eprintln!(
                "[U4 정리] PID {} 를 확인하지 못해 끄지 않고 남긴다: {e}",
                p.pid
            );
        }
    }

    /// 뿌리(이 시험 프로세스가 띄운 `cmd.exe`) 아래 신원을 **찍은 순간**의 목록 — 그 뒤 태어난 후손은 들지 않는다(가짜는
    /// 손자 하나를 띄운 뒤 더 띄우지 않는다).
    struct Tree {
        root: ProcessIdentity,
        cli: ProcessIdentity,
        grandchild: ProcessIdentity,
        all: Vec<ProcessIdentity>,
    }

    impl Tree {
        fn role(&self, p: &ProcessIdentity) -> &'static str {
            if *p == self.root {
                "뿌리 cmd"
            } else if *p == self.cli {
                "CLI powershell"
            } else if *p == self.grandchild {
                "손자 ping"
            } else {
                "그 밖의 후손"
            }
        }
    }

    fn read_identities(path: &Path) -> (ProcessIdentity, ProcessIdentity) {
        let text = std::fs::read_to_string(path).expect("표식 읽기");
        let fields: Vec<u64> = text
            .split_whitespace()
            .filter_map(|s| s.parse().ok())
            .collect();
        let [cli_pid, cli_start, grandchild_pid, grandchild_start] = fields[..] else {
            panic!("표식은 `<CLI PID> <시작시각> <손자 PID> <시작시각>` 여야: {text:?}");
        };
        let identity = |pid: u64, start_time: u64| ProcessIdentity {
            pid: u32::try_from(pid).expect("PID 는 u32"),
            start_time,
        };
        (
            identity(cli_pid, cli_start),
            identity(grandchild_pid, grandchild_start),
        )
    }

    /// 표식으로 CLI 의 트리를 찾아 찍는다. 표식의 신원(가짜가 자기 손잡이로 읽은 PID · 시작시각)은 **넘어질 수 있는
    /// 다른 무엇보다 먼저** `cleanup` 에 올린다 — 확인에서 넘어져도 손자까지 거두게.
    fn observe_tree(cli_dir: &Path, cleanup: &mut Cleanup) -> Tree {
        let marker = cli_dir.join("cli.pid");
        assert!(
            wait_until(SETUP, || marker.exists()),
            "가짜 CLI 가 {SETUP:?} 안에 표식을 안 남겼다 — powershell 이 못 떴거나 심을 못 찾았다(실험 불성립)"
        );
        let (reported_cli, reported_grandchild) = read_identities(&marker);
        cleanup.hold(reported_cli);
        cleanup.hold(reported_grandchild);
        let (cli, grandchild) = (reported_cli.pid, reported_grandchild.pid);

        let table = process_parent_table().expect("프로세스 표");
        let parent_of = |pid: u32| table.iter().find(|(p, _)| *p == pid).map(|(_, pp)| *pp);
        let root = parent_of(cli).expect("CLI 의 부모");
        assert_eq!(
            parent_of(root),
            Some(std::process::id()),
            "뿌리는 이 시험 프로세스가 띄운 것이어야 한다(manager 가 프로세스 안에서 띄운다)"
        );
        assert_eq!(
            parent_of(grandchild),
            Some(cli),
            "손자는 CLI 의 자식이어야 한다"
        );
        assert!(is_ping(grandchild), "손자가 ping 이 아니다");
        let root_start = process_creation_time(root).expect("뿌리 시작시각");
        let all = subtree(root, root_start);
        for p in &all {
            cleanup.hold(*p);
        }
        for reported in [reported_cli, reported_grandchild] {
            assert!(
                all.contains(&reported),
                "가짜가 알린 신원 {reported:?} 가 뿌리 트리에 그대로 없다 — 시작시각 척도가 어긋났거나 그 PID 가 다른 \
                 프로세스다: {all:?}"
            );
        }
        Tree {
            root: all[0],
            cli: reported_cli,
            grandchild: reported_grandchild,
            all,
        }
    }

    /// 그 신원의 프로세스가 아직 돈다. 끝났어도 누가 핸들을 쥐면 객체가 남으므로(`ProcessStart` doc) 시작시각 대조에
    /// 더해 종료 여부를 본다. 못 읽으면(`Unknown`) 돈다고 본다 — 거짓 통과보다 거짓 실패 쪽.
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
            .expect("이 호출이 띄웠다");

        let trigger = Arc::new(PanicOnArm::new());
        manager.subscribe(id, trigger.clone()).expect("구독");

        let tree = observe_tree(&scratch.cli, &mut cleanup);
        trigger.armed.store(true, Ordering::SeqCst);
        assert!(
            wait_until(SETUP, || trigger.panicked_at().is_some()),
            "무장 뒤 {SETUP:?} 안에 가둔 펌프가 emit 하지 않았다(실험 불성립)"
        );
        let panicked_at = trigger.panicked_at().expect("방금 봤다");

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
            "[U4 {}] panic → 목록에서 빠짐 {dropped_after:?} · 트리 전부 사라짐 {tree_gone_after:?}(상한 {BOUND:?}) · {each:?}",
            pump.tag()
        );

        let statuses = seen.statuses_of(id);
        assert!(
            !statuses
                .iter()
                .any(|s| matches!(s, AgentStatus::Exiting | AgentStatus::Killed)),
            "명시 종료 경로를 탔다 — 이 시험은 그 길을 재지 않는다: {statuses:?}"
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
            other => panic!("가둔 펌프 panic 은 끝 상태 Failed 한 번이어야: {other:?}"),
        }

        assert!(
            dropped_after.is_some(),
            "panic 뒤 {SETUP:?} 안에 reaper 가 세션을 빼지 않았다"
        );
        assert!(
            !manager.list_agents().iter().any(|a| a.id == id),
            "세션이 명부에 남았다"
        );

        let survivors: Vec<String> = tree
            .all
            .iter()
            .zip(&gone_after)
            .filter(|(_, gone)| gone.is_none_or(|d| d > BOUND))
            .map(|(p, gone)| {
                let when = match gone {
                    Some(d) => format!("{d:?}"),
                    None => format!("{SETUP:?} 안에 안 사라짐"),
                };
                format!("{} (PID {}) — 사라진 때 {when}", tree.role(p), p.pid)
            })
            .collect();
        assert!(
            survivors.is_empty(),
            "결정 A 불성립 — 가둔 펌프 panic 뒤 {BOUND:?} 에 CLI 트리가 남았다(사용자 결정 거리 · B 로 넘어가지 말 것): {survivors:#?}"
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

    // ── 대조군: 무리(Job) 없이는 손자가 상한을 넘겨 산다 ──
    //
    // 가둔 panic 경로가 무리보다 먼저 · 함께 놓는 것(출력 읽기 끝 · stdin · 뿌리 · PTY 콘솔)을 무리 없이 다 놓아도 가짜의
    // 손자는 상한 동안 산다 — 그래서 위 시험에서 손자가 상한 안에 사라진 것은 무리 닫기 말고는 설명되지 않는다.
    // 가짜가 바뀌어 손자가 제풀에 죽게 되면 여기가 빨개진다.

    fn assert_the_grandchild_outlives_the_bound(tree: &Tree) {
        std::thread::sleep(BOUND);
        let alive: Vec<&str> = tree
            .all
            .iter()
            .filter(|p| running(p))
            .map(|p| tree.role(p))
            .collect();
        eprintln!("[U4 대조군] 무리 없이 {BOUND:?} 뒤 산 것: {alive:?}");
        assert!(
            running(&tree.grandchild),
            "무리 없이도 손자가 {BOUND:?} 안에 죽었다 — 위 시험의 손자 소멸이 무리 닫기의 증거가 못 된다"
        );
    }

    /// 가짜가 표식을 못 남기는 갈래 — 표식 자리를 미리 막아 이름 바꾸기를 실패시킨다. 가짜는 실패로 끝나고 손자를 함께
    /// 끈다. 그 갈래에 닿았다는 증거는 이름 바꾸기 앞에서 쓴 `cli.pid.tmp` 이고, 손자는 거기 적힌 신원으로 잰다 —
    /// powershell 이 그 전에 넘어지면 손자가 아예 없어 「남은 손자 없음」이 헛통과한다.
    #[test]
    fn a_fake_cli_that_cannot_write_its_marker_ends_its_grandchild() {
        use std::process::{Command, Stdio};

        let scratch = Scratch::new(Pump::Pty);
        std::fs::write(scratch.cli.join("cli.pid"), "").expect("표식 자리 막기");
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
        let cli = cleanup.pipe_root.insert(cmd.spawn().expect("가짜 띄우기"));
        let pid = cli.id();
        let start = process_creation_time(pid).expect("가짜 시작시각");

        assert!(
            wait_until(SETUP, || matches!(cli.try_wait(), Ok(Some(_)))),
            "표식을 못 남긴 가짜가 {SETUP:?} 안에 끝나지 않았다"
        );
        let status = cli.wait().expect("종료 상태");
        assert!(
            !status.success(),
            "표식을 못 남겼는데 성공으로 끝났다: {status:?}"
        );
        let written = scratch.cli.join("cli.pid.tmp");
        assert!(
            written.exists(),
            "가짜가 이름 바꾸기 실패에 닿기 전에 끝났다(실험 불성립)"
        );
        let (reported_cli, grandchild) = read_identities(&written);
        cleanup.hold(grandchild);
        // 척도가 어긋나면 손자의 시작시각도 어긋나 산 손자가 「사라졌다」로 읽힌다.
        assert_eq!(
            reported_cli,
            ProcessIdentity {
                pid,
                start_time: start
            },
            "가짜가 적은 자기 신원이 실제와 다르다 — 시작시각 척도가 어긋났다"
        );
        assert!(
            wait_until(SETUP, || !running(&grandchild)),
            "표식을 못 남긴 가짜의 손자가 남았다: {grandchild:?}"
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
        cleanup.pipe_root = Some(cmd.spawn().expect("뿌리 띄우기"));
        let tree = observe_tree(&scratch.cli, &mut cleanup);

        let root = cleanup.pipe_root.as_mut().expect("방금 쥐었다");
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
        cleanup.pty_root = Some(pair.slave.spawn_command(cmd).expect("뿌리 띄우기"));
        drop(pair.slave);
        let reader = pair.master.try_clone_reader().expect("읽기 끝");
        let tree = observe_tree(&scratch.cli, &mut cleanup);

        drop(reader);
        drop(pair.master);

        assert_the_grandchild_outlives_the_bound(&tree);
    }
}
