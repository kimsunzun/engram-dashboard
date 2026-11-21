//! Control channel (ADR-0086 step 1) — the daemon control plane that spawned agents attach to.
//!
//! tauri import 0(daemon crate).

pub mod agent;
pub mod catalog;
pub mod commands;
pub mod help;
pub mod ingress;
pub mod mcp_config;
pub mod mcp_server;
pub mod priming;
pub mod registry;

use std::path::PathBuf;
use std::sync::Arc;

use engram_dashboard_agent::types::{
    AgentId, ControlChannel, ControlChannelNeeds, ControlEndpoint, ProvisionError, ToolGrant,
    CLI_EXE_NAME,
};

use mcp_config::MCP_SERVER_NAME;
use mcp_server::SEND_MESSAGE_TOOL;
use priming::PrimingProvider;
use registry::ControlRegistry;

pub struct DaemonControlChannel {
    registry: Arc<ControlRegistry>,
    mcp_url: String,
    mcp_dir: mcp_config::McpDir,
    send_exe: Option<PathBuf>,
    priming: Arc<dyn PrimingProvider>,
}

impl DaemonControlChannel {
    pub fn new(
        registry: Arc<ControlRegistry>,
        mcp_url: String,
        mcp_dir: mcp_config::McpDir,
        send_exe: Option<PathBuf>,
        priming: Arc<dyn PrimingProvider>,
    ) -> Self {
        Self {
            registry,
            mcp_url,
            mcp_dir,
            send_exe,
            priming,
        }
    }

    /// `ENGRAM_DISALLOW_MCP_SEND` is a test-only knob: if set + non-empty, the MCP eg_send grant is removed.
    /// It cannot create CLI-only routing: mail permission is decided by the credential, not the grant — the daemon refuses mail requests with an MCP-capable
    /// spawn's credential (ADR-0133), so removing the grant only drops the send entrance to 0. Spawns run in the auto permission mode, so the grant is a
    /// NO-OP (ADR-0097; measured 2026-08-03, 6/6 agents with the knob on still sent). Only `ENGRAM_FORCE_CLI_ONLY_SEND` (provision) splits the channel wholesale.
    // ADR-0094
    /// If `uses_mail == false`, not a single line is emitted: this list holds only send entrance tools (ADR-0094 least privilege), and a spawn outside the mail
    /// plane has no such entrance. Looking only at `!accepts_mcp_config` once carried a CLI grant, so the daemon recorded "allowed sending" while that credential's mail requests were refused.
    // ADR-0133
    // ADR-0209
    fn build_grants(
        send_exe: Option<&std::path::Path>,
        needs: ControlChannelNeeds,
    ) -> Vec<ToolGrant> {
        if !needs.uses_mail {
            return Vec::new();
        }
        let accepts_mcp_config = needs.accepts_mcp_config;
        let disallow_mcp_send = std::env::var("ENGRAM_DISALLOW_MCP_SEND")
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        let mut grants = Vec::new();
        if accepts_mcp_config && !disallow_mcp_send {
            grants.push(ToolGrant::Mcp {
                server: MCP_SERVER_NAME.to_string(),
                tool: SEND_MESSAGE_TOOL.to_string(),
            });
        }
        // exe is not derived from send_exe: the grant string must equal the bare command name character for character (`CLI_EXE_NAME`, the source of truth);
        //   send_exe decides only whether the CLI entrance exists (an absolute path would not match a bare-name call — ADR-0098).
        // No priming teaches this branch's grant: a non-MCP spawn gets no mail teaching (provision's `wants_priming`). It still emits the grant because `mail_allowed`
        //   authorizes CLI mail for that spawn — diverging, the permission list would block requests the daemon accepts.
        // Known unmet (intentional, awaiting a user decision): since ADR-0133 priming A teaches MCP-capable spawns the bare `engram`, but this branch emits no CLI
        //   grant, so "taught name == grant name" is broken only here. Harmless while spawns run under `--permission-mode bypassPermissions` (the grant is a NO-OP,
        //   ADR-0097); reopening the grant axis is a user decision. Do not delete this sentence and read the alignment as holding — if the base flag is withdrawn this branch breaks first.
        // ADR-0094
        // ADR-0128
        if !accepts_mcp_config && send_exe.is_some() {
            grants.push(ToolGrant::Cli {
                exe: CLI_EXE_NAME.to_string(),
            });
        }
        grants
    }

    /// A harness knob (`ENGRAM_FORCE_CLI_ONLY_SEND`) that forces spawns to non-MCP to measure the entire false path.
    ///
    /// Production builds do not compile it (`const fn false`): it flips the channel wiring, the priming variant and even `mail_allowed`, so a release binary
    /// letting one env variable change the authorization would let an agent edit its own env to move the daemon's decision. `test-harness` is a
    /// self-dev-dependency, so it is not unified into the production dep graph — the condition that breaks that (`--all-targets`, which would bake the knob into
    /// a release binary) is in the Cargo.toml `[dev-dependencies]` comment.
    /// Only the unit tests below turn it on; the non-MCP branch itself is alive (codex · gemini have that shape) and this is the only means to step on it with claude.
    // ADR-0133
    #[cfg(feature = "test-harness")]
    fn force_cli_only() -> bool {
        std::env::var("ENGRAM_FORCE_CLI_ONLY_SEND")
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    }

    /// The production build branch — a `const fn` with no means to read env (reason: the doc above).
    ///
    /// No test on this axis, intentionally: test builds always have the feature on (self-dev-dependency), so `#[cfg(not(feature = …))]` tests are not even type-checked and
    /// rot silently while suggesting the axis is guarded. The compiler guards it: the function returns a constant and the env variable's name is absent from the release binary (verified, 0 occurrences).
    // ADR-0133
    #[cfg(not(feature = "test-harness"))]
    const fn force_cli_only() -> bool {
        false
    }

    /// The same method as lib.rs `generate_token`, but that one is the shell link token (the owner-only token file — ADR-0395) — do not share them (ADR-0086 §Context).
    fn gen_token() -> Option<String> {
        let mut buf = [0u8; 32];
        getrandom::getrandom(&mut buf).ok()?;
        Some(engram_dashboard_base::text::hex_lower(&buf))
    }
}

impl ControlChannel for DaemonControlChannel {
    fn provision(
        &self,
        id: AgentId,
        epoch: u32,
        needs: ControlChannelNeeds,
    ) -> Result<Option<ControlEndpoint>, ProvisionError> {
        let token = Self::gen_token()
            .ok_or_else(|| ProvisionError("CSPRNG token generation failed".to_string()))?;
        // A spawn with this knob on gets no priming: `wants_priming` cuts the non-MCP branch first, so even the `ENGRAM_PRIMING_FILE` override carries no MCP teaching
        //   (the old "teaching=MCP · physics=CLI" combination cannot form). In production `force_cli_only()` is const false, so the expression is `accepts_mcp_config`.
        let force_cli_only = Self::force_cli_only();
        let accepts_mcp_config = needs.accepts_mcp_config && !force_cli_only;
        // The knob turns this field off too ("the entire false path"): leaving the file would leave an MCP credential on disk for a spawn the harness believes is non-MCP.
        let writes_mcp_config_file = needs.writes_mcp_config_file && !force_cli_only;
        let needs = ControlChannelNeeds {
            accepts_mcp_config,
            writes_mcp_config_file,
            ..needs
        };
        // The single derivation point of mail allowance (ADR-0133 decision 2): this one value produces ① the enforcement stamped into the credential (daemon
        //   refusal) ② the same fact the endpoint carries ③ the diagnostic log below.
        //   A spawn using mail over MCP does not use CLI mail — channels split only by capability, no runtime switching or fallback (ADR-0128 decision 1); a spawn
        //   outside the mail plane is neither (`uses_mail`).
        //   Do not write the expression a second time, not even in the log: a diverging copy would tell the opposite fact where "why can't it send letters" is
        //   chased. Hence this line comes right after `accepts_mcp_config` is settled.
        // ADR-0133
        // ADR-0209
        let mail_allowed = needs.uses_mail && !accepts_mcp_config;
        // The gate is the field declaring that the backend reads the file (ADR-0209), not `accepts_mcp_config`: when they were one field, a backend turning on MCP
        //   by another mechanism (codex `-c mcp_servers.engram=…`) got a plaintext Bearer token JSON nobody opens for every default-ACL spawn, and the fail-closed `?`
        //   below could cut the spawn over that file.
        //   Do not move the `mail_allowed` line above to this field — that derivation stays with `accepts_mcp_config` (ADR-0133 decision 2); what split is whether to write a secret to disk.
        // ADR-0099
        // ADR-0209
        let (config_path, settings_file) = if writes_mcp_config_file {
            let path = mcp_config::write_config(&self.mcp_dir, id, epoch, &self.mcp_url, &token)
                .map_err(|e| {
                    tracing::warn!(agent = %id, epoch, "mcp-config write failed — fail-closed (spawn aborted): {e}");
                    ProvisionError(format!("mcp-config write failed: {e}"))
                })?;
            // A write failure of this fragment is not fatal (unlike mcp-config; load-bearing): without mcp-config the MCP channel physically disappears, but this fragment only
            //   overrides a user-global block, so things work if the global setting allows. Blocking the spawn would be a regression; it warns and proceeds without the fragment.
            let settings = match mcp_config::write_settings(&self.mcp_dir, id, epoch) {
                Ok(p) => Some(p),
                Err(e) => {
                    tracing::warn!(
                        agent = %id, epoch,
                        "session settings fragment write failed — spawning without the fragment (if the global allowedMcpServers blocks it, the engram tools may not be visible): {e}"
                    );
                    None
                }
            };
            (Some(path), settings)
        } else {
            (None, None)
        };
        // Priming goes only to spawns that really use MCP mail: the one remaining priming file teaches `eg_send` (ADR-0126 decision 1), and carrying it to a spawn
        //   without that tool records "taught" while the agent calls a nonexistent tool (the send freeze ADR-0099 measured). Hence both axes must be true together.
        //   The CLI mirror variant is gone — do not fill that place with A (user decision 2026-09-19): a non-MCP spawn gets no priming; the mail CLI is a mirror, the
        //   canonical path is attaching MCP to that backend (ADR-0209 decision 3). Silence is not a lie; teaching a nonexistent tool is.
        // ADR-0092
        // ADR-0099
        // ADR-0126
        // ADR-0133
        // ADR-0209
        let wants_priming = needs.uses_mail && accepts_mcp_config;
        tracing::debug!(
            agent = %id,
            epoch,
            accepts_mcp_config,
            writes_mcp_config_file,
            uses_mail = needs.uses_mail,
            force_cli_only,
            wants_priming,
            has_mcp_config = config_path.is_some(),
            "control channel provision fork (ADR-0099 channel switch)"
        );
        // `needs.uses_mail` in the condition is load-bearing: this fail-closed guards the pair violation "the daemon authorized CLI mail for this credential
        //   (`mail_allowed` · CLI grant) but there is no executable to call" — a physically 0 authorized channel silently drops every letter. A spawn outside the
        //   mail plane gets no authorization, so cutting it too would stop mail-unrelated backends entirely in an install without `engram` (missing control verbs is
        //   the fail-open warn below).
        // Missing priming does not relax this: the branch launches having learned nothing but authorization still goes out (`mail_allowed`), so the guarded pair stays.
        // ADR-0099
        // ADR-0133
        // ADR-0209
        if needs.uses_mail && !accepts_mcp_config && self.send_exe.is_none() {
            let msg = format!(
                "non-MCP backend with no `{CLI_EXE_NAME}` binary — zero physical send channels while the daemon authorises CLI mail for this credential (pairing invariant violation)"
            );
            tracing::warn!(agent = %id, epoch, "control channel provision fail-closed (ADR-0099): {msg}");
            // No reclaim code: this branch is true only when !accepts_mcp_config, so write_config did not run and no token is issued — nothing to undo.
            return Err(ProvisionError(msg));
        }
        // Makes issuing a control channel without the CLI entrance visible (ADR-0132 decision 5): MCP-capable spawns and spawns outside the mail plane both reach here
        //   (the fail-closed above lets both through), and either way control verbs are unreachable (nothing to call `engram`). Fail-open is the existing judgment, but
        //   passing without a log line would let the "open to everyone" decision and reality silently diverge. Symptom: `engram: command not found`, cause in the distribution folder.
        // This line says nothing about mail — the two branches differ (MCP works normally; the other has no mail at all), so one side's answer would be false in the other.
        // ADR-0133
        if self.send_exe.is_none() {
            tracing::warn!(
                agent = %id, epoch,
                mail_allowed,
                "issued a control channel but could not find the `{CLI_EXE_NAME}` executable — this spawn cannot use control verbs (engram …) (check that it was deployed as a sibling of the daemon exe)."
            );
        }
        // The enforcement is this one line — the endpoint's same field is only a record nobody reads (that field's doc).
        self.registry.issue(id, epoch, token.clone(), mail_allowed);
        let priming_file = wants_priming.then(|| self.priming.priming_file()).flatten();
        let grants = Self::build_grants(self.send_exe.as_deref(), needs);
        Ok(Some(ControlEndpoint {
            agent_id: id,
            epoch,
            url: self.mcp_url.clone(),
            token,
            config_path,
            // Both branches get the same value — the control CLI is laid down for everyone (ADR-0132 decision 5 · ADR-0133).
            send_exe: self.send_exe.clone(),
            priming_file,
            grants,
            settings_file,
            mail_allowed,
        }))
    }

    fn revoke(&self, id: AgentId, epoch: u32) {
        self.registry.revoke(id, epoch);
        mcp_config::remove_config(&self.mcp_dir, id, epoch);
        mcp_config::remove_settings(&self.mcp_dir, id, epoch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An adapter keeping the tests from the old single-axis (`accepts_mcp_config`) era standing: stamping `uses_mail: true` is that equivalence (the outside-the-mail-plane
    /// branch did not exist then; the section below measures it without this helper).
    ///
    /// Moving the third field together with the first is also that equivalence (before the file write axis split, one field drove both = claude's shape). "Uses MCP but does
    /// not read the file" (codex) is measured by [`the_file_write_follows_the_declaring_backend_not_the_mail_axis`] without this helper.
    fn needs(accepts_mcp_config: bool) -> ControlChannelNeeds {
        ControlChannelNeeds {
            accepts_mcp_config,
            writes_mcp_config_file: accepts_mcp_config,
            uses_mail: true,
        }
    }
    use engram_dashboard_agent::types::CLI_EXE_ENV;
    use std::path::Path;

    /// A single lock, not one per knob: `provision` reads both envs, so a reader holding one races with the other's setter and goes flaky. Every test touching either knob takes it.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Recovers from poison — do not revert to plain `.unwrap()` (measured 2026-08-04): a test panicking under the lock would make every later holder panic at the lock
    /// line, inflating 1 failure into 17 and burying the original assert location. The value is `()`, so nothing polluted is inherited; the mutex still serializes.
    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }
    const DISALLOW_MCP_ENV: &str = "ENGRAM_DISALLOW_MCP_SEND";
    const FORCE_CLI_ENV: &str = "ENGRAM_FORCE_CLI_ONLY_SEND";

    // ── ADR-0094: build_grants — computing the send entrance pre-authorization grant (single source · least privilege) ──────

    #[test]
    fn build_grants_mcp_always_present_with_channel_names() {
        let _g = lock_env();
        assert!(
            std::env::var(DISALLOW_MCP_ENV).is_err(),
            "env must be unset on entering the test (leak detection)"
        );
        let grants = DaemonControlChannel::build_grants(None, needs(true));
        assert_eq!(
            grants,
            vec![ToolGrant::Mcp {
                server: MCP_SERVER_NAME.to_string(),
                tool: SEND_MESSAGE_TOOL.to_string(),
            }],
            "send_exe=None gives only one MCP grant (no CLI entrance)"
        );
    }

    #[test]
    fn build_grants_cli_grant_follows_the_channel_axis_not_send_exe_presence() {
        let _g = lock_env();
        assert!(std::env::var(DISALLOW_MCP_ENV).is_err());
        let exe = Path::new("C:/app/engram.exe");
        assert_eq!(
            DaemonControlChannel::build_grants(Some(exe), needs(true)),
            vec![ToolGrant::Mcp {
                server: MCP_SERVER_NAME.to_string(),
                tool: SEND_MESSAGE_TOOL.to_string(),
            }],
            "MCP-capable + send_exe → only [Mcp] (the CLI grant comes out only on the mail channel axis)"
        );
        assert_eq!(
            DaemonControlChannel::build_grants(Some(exe), needs(false)),
            vec![ToolGrant::Cli {
                exe: CLI_EXE_NAME.to_string(),
            }],
            "non-MCP + send_exe → [Cli] (the CLI grant comes out only on this axis)"
        );
    }

    #[test]
    fn build_grants_is_minimal_privilege() {
        let _g = lock_env();
        assert!(std::env::var(DISALLOW_MCP_ENV).is_err());
        let exe = Path::new("C:/app/engram.exe");
        for accepts_mcp_config in [true, false] {
            let grants = DaemonControlChannel::build_grants(Some(exe), needs(accepts_mcp_config));
            assert_eq!(
                grants.len(),
                1,
                "only one send entrance (accepts_mcp_config={accepts_mcp_config}): {grants:?}"
            );
            match &grants[0] {
                ToolGrant::Mcp { tool, .. } => assert_eq!(tool, SEND_MESSAGE_TOOL),
                ToolGrant::Cli { exe } => assert_eq!(exe, CLI_EXE_NAME),
            }
        }
    }

    // ── ADR-0094 test-seam: ENGRAM_DISALLOW_MCP_SEND — removes the MCP grant ──────────────────────────────

    #[test]
    fn build_grants_disallow_mcp_env_on_mcp_capable_leaves_no_send_grant() {
        let _g = lock_env();
        assert!(
            std::env::var(DISALLOW_MCP_ENV).is_err(),
            "env must be unset on entering the test (leak detection)"
        );
        std::env::set_var(DISALLOW_MCP_ENV, "1");
        let exe = Path::new("C:/app/engram.exe");
        let mcp_capable = DaemonControlChannel::build_grants(Some(exe), needs(true));
        let non_mcp = DaemonControlChannel::build_grants(Some(exe), needs(false));
        std::env::remove_var(DISALLOW_MCP_ENV);
        assert!(
            mcp_capable.is_empty(),
            "env on + MCP-capable → 0 send grants: {mcp_capable:?}"
        );
        assert_eq!(
            non_mcp,
            vec![ToolGrant::Cli {
                exe: CLI_EXE_NAME.to_string(),
            }],
            "even with env on, the non-MCP branch's CLI grant stays (the seam removes only the MCP grant)"
        );
    }

    #[test]
    fn build_grants_disallow_mcp_env_with_no_send_exe_yields_empty() {
        let _g = lock_env();
        assert!(std::env::var(DISALLOW_MCP_ENV).is_err());
        std::env::set_var(DISALLOW_MCP_ENV, "1");
        let grants = DaemonControlChannel::build_grants(None, needs(true));
        std::env::remove_var(DISALLOW_MCP_ENV);
        assert!(
            grants.is_empty(),
            "env on + no send_exe → 0 send grants (only removal, no addition): {grants:?}"
        );
    }

    #[test]
    fn build_grants_disallow_mcp_empty_value_is_production_default() {
        let _g = lock_env();
        assert!(std::env::var(DISALLOW_MCP_ENV).is_err());
        std::env::set_var(DISALLOW_MCP_ENV, "");
        let grants =
            DaemonControlChannel::build_grants(Some(Path::new("C:/app/engram.exe")), needs(true));
        std::env::remove_var(DISALLOW_MCP_ENV);
        assert_eq!(
            grants,
            vec![ToolGrant::Mcp {
                server: MCP_SERVER_NAME.to_string(),
                tool: SEND_MESSAGE_TOOL.to_string(),
            }],
            "empty value = seam not triggered → the production grant of the MCP-capable branch ([Mcp] — ADR-0128)"
        );
    }

    // ── ADR-0099: per-channel grant emission — non-MCP backends do not emit the MCP grant ──────────────────

    #[test]
    fn build_grants_non_mcp_backend_emits_cli_only() {
        let _g = lock_env();
        assert!(std::env::var(DISALLOW_MCP_ENV).is_err());
        let exe = Path::new("C:/app/engram.exe");
        let grants = DaemonControlChannel::build_grants(Some(exe), needs(false));
        assert_eq!(
            grants,
            vec![ToolGrant::Cli {
                exe: CLI_EXE_NAME.to_string(),
            }],
            "non-MCP backend → only the CLI grant (MCP entrance physically absent)"
        );
    }

    #[test]
    fn build_grants_non_mcp_backend_no_send_exe_yields_empty() {
        let _g = lock_env();
        assert!(std::env::var(DISALLOW_MCP_ENV).is_err());
        let grants = DaemonControlChannel::build_grants(None, needs(false));
        assert!(
            grants.is_empty(),
            "non-MCP + no send_exe → 0 send grants: {grants:?}"
        );
    }

    // ── ADR-0099: provision branch — the channel wiring and whether priming is loaded move together with MCP-capability ──────
    //   (not a variant axis — since commit `2ef6902` the non-MCP branch gets no priming at all, ADR-0209).

    use crate::control::priming::PrimingProvider;
    use std::sync::{Arc, Mutex};

    /// Records whether the seam was called: with the variant axis gone, provision decides "whether to ask" (ADR-0209 · user decision 2026-09-19), and `ep.priming_file` alone cannot be told from "the provider gave None".
    struct RecordingPriming {
        seen: Arc<Mutex<bool>>,
    }
    impl PrimingProvider for RecordingPriming {
        fn priming_file(&self) -> Option<PathBuf> {
            *self.seen.lock().unwrap() = true;
            Some(PathBuf::from("A-mcp-primary"))
        }
    }

    fn provision_test_channel_with_send(
        seen: Arc<Mutex<bool>>,
        send_exe: Option<PathBuf>,
    ) -> (DaemonControlChannel, PathBuf) {
        let data_dir =
            std::env::temp_dir().join(format!("engram-provision-adr0099-{}", AgentId::new_v4()));
        let channel = DaemonControlChannel::new(
            Arc::new(ControlRegistry::new()),
            "http://127.0.0.1:1/mcp".to_string(),
            mcp_config::McpDir::new(data_dir.join("mcp-config")),
            send_exe,
            Arc::new(RecordingPriming { seen }),
        );
        (channel, data_dir)
    }

    #[test]
    fn provision_mcp_capable_writes_config_and_picks_mcp_primary_priming() {
        let _g = lock_env();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());
        let seen = Arc::new(Mutex::new(false));
        let (channel, data_dir) =
            provision_test_channel_with_send(seen.clone(), Some(PathBuf::from(CLI_EXE_NAME)));
        let id = AgentId::new_v4();
        let ep = channel
            .provision(id, 0, needs(true))
            .expect("provision ok")
            .expect("endpoint");
        let cfg = ep
            .config_path
            .as_ref()
            .expect("MCP-capable → config_path Some");
        assert!(
            cfg.is_file(),
            "MCP-capable → the mcp-config file physically exists"
        );
        assert!(
            *seen.lock().unwrap(),
            "MCP-capable → must ask the priming seam"
        );
        assert_eq!(ep.priming_file, Some(PathBuf::from("A-mcp-primary")));
        let settings = ep
            .settings_file
            .as_ref()
            .expect("MCP-capable → settings_file Some");
        assert!(
            settings.is_file(),
            "the settings fragment file physically exists"
        );
        let content = std::fs::read_to_string(settings).expect("read settings");
        assert!(
            content.contains("allowedMcpServers") && content.contains(MCP_SERVER_NAME),
            "the fragment must allow the engram server: {content}"
        );
        channel.revoke(id, 0);
        assert!(
            !settings.exists(),
            "the settings fragment is deleted on revoke"
        );
        assert!(!cfg.exists(), "mcp-config is deleted on revoke");
        channel.revoke(id, 0); // a double revoke is safe.
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn provision_non_mcp_skips_config_and_injects_no_priming() {
        let _g = lock_env();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());
        let seen = Arc::new(Mutex::new(false));
        let (channel, data_dir) =
            provision_test_channel_with_send(seen.clone(), Some(PathBuf::from(CLI_EXE_NAME)));
        let id = AgentId::new_v4();
        let ep = channel
            .provision(id, 0, needs(false))
            .expect("provision ok")
            .expect("endpoint");
        assert_eq!(
            ep.config_path, None,
            "non-MCP → mcp-config not written (config_path=None, absent by type encoding)"
        );
        assert!(
            !data_dir.join("mcp-config").exists(),
            "non-MCP → the mcp-config file must be physically absent"
        );
        assert_eq!(
            ep.settings_file, None,
            "non-MCP → settings_file is None too (consistency invariant: laid channel == allowed channel)"
        );
        assert!(
            !*seen.lock().unwrap(),
            "non-MCP → must not ask the priming seam at all (the remaining file teaches only the MCP tool)"
        );
        assert_eq!(
            ep.priming_file, None,
            "non-MCP → priming not injected (silence rather than teaching a tool that does not exist — ADR-0209 decision 3)"
        );
        assert_eq!(
            ep.grants,
            vec![ToolGrant::Cli {
                exe: CLI_EXE_NAME.to_string(),
            }],
            "non-MCP → grants == [Cli]"
        );
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    // ── ADR-0099 FIX 2: fail-closed edge — non-MCP + send_exe=None = 0 channels → ProvisionError ──────
    #[test]
    fn provision_non_mcp_with_no_send_exe_fails_closed() {
        let _g = lock_env();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());
        let seen = Arc::new(Mutex::new(false));
        let (channel, data_dir) = provision_test_channel_with_send(seen.clone(), None);
        let id = AgentId::new_v4();
        let err = channel
            .provision(id, 0, needs(false))
            .expect_err("non-MCP + send_exe=None → fail-closed Err");
        assert!(
            err.0.contains("non-MCP") && err.0.contains(CLI_EXE_NAME),
            "the ProvisionError reason states the cause: {}",
            err.0
        );
        assert!(
            !data_dir.join("mcp-config").exists(),
            "fail-closed edge → no config file created"
        );
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    /// Even if provision's config-write `?` became fail-open no other suite breaks — hence this test: an MCP-capable spawn would come out with `config_path=None` and the backend would read it as CLI-only.
    #[test]
    fn provision_mcp_capable_fails_closed_when_config_write_fails() {
        let _g = lock_env();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());
        let seen = Arc::new(Mutex::new(false));
        let (channel, data_dir) = provision_test_channel_with_send(
            seen.clone(),
            Some(PathBuf::from("C:/app/engram.exe")),
        );
        let id = AgentId::new_v4();
        let cfg_dir =
            mcp_config::config_path(&mcp_config::McpDir::new(data_dir.join("mcp-config")), id, 0)
                .parent()
                .expect("the config path has a parent folder")
                .to_path_buf();
        std::fs::create_dir_all(&data_dir).expect("create data_dir");
        std::fs::write(&cfg_dir, b"occupied").expect("occupy the folder's place with a file");

        let err = channel
            .provision(id, 0, needs(true))
            .expect_err("MCP-capable + config write failure → fail-closed Err");
        assert!(
            err.0.contains("mcp-config write failed"),
            "the ProvisionError reason states the cause: {}",
            err.0
        );
        assert!(
            !*seen.lock().unwrap(),
            "if fail-closed, it does not reach the priming seam (no half provision)"
        );
        let _ = std::fs::remove_file(&cfg_dir);
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn provision_mcp_capable_with_no_send_exe_is_allowed() {
        let _g = lock_env();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());
        let seen = Arc::new(Mutex::new(false));
        let (channel, data_dir) = provision_test_channel_with_send(seen.clone(), None);
        let id = AgentId::new_v4();
        let ep = channel
            .provision(id, 0, needs(true))
            .expect("MCP-capable + send_exe=None is allowed (accepted edge)")
            .expect("endpoint");
        assert!(
            ep.config_path.is_some(),
            "MCP-capable → config_path Some (the MCP entrance is alive)"
        );
        assert_eq!(
            ep.grants,
            vec![ToolGrant::Mcp {
                server: MCP_SERVER_NAME.to_string(),
                tool: SEND_MESSAGE_TOOL.to_string(),
            }],
            "MCP-capable + send_exe=None → grants == [Mcp]"
        );
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    // ── ADR-0209: the file write axis follows only **the backend that declares it reads the file** ────────────────────

    /// Reads the real backend declaration (a literal would make the decision a test copy): both rows have `accepts_mcp_config` true yet results must diverge, so this asserts the gate is on
    /// the file axis, not the mail axis — reverting the gate to `accepts_mcp_config` breaks the codex row, removing it gives that row the plaintext token file again.
    /// The claude row must stand together: codex alone stays green even if the write is removed entirely, silently losing claude's MCP entrance (the file `--mcp-config` points to).
    // ADR-0209
    #[test]
    fn the_file_write_follows_the_declaring_backend_not_the_mail_axis() {
        use engram_dashboard_agent::backend::{
            accepts_mcp_config, uses_mail, writes_mcp_config_file,
        };
        use engram_dashboard_agent::profile::{AgentCommand, AgentOutputFormat};

        let _g = lock_env();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());

        for (name, command, expect_file) in [
            (
                "codex",
                AgentCommand::Codex {
                    extra_args: vec![],
                    output_format: AgentOutputFormat::Terminal,
                },
                false,
            ),
            (
                "claude",
                AgentCommand::Claude {
                    extra_args: vec![],
                    output_format: AgentOutputFormat::StreamJson,
                },
                true,
            ),
        ] {
            let seen = Arc::new(Mutex::new(false));
            let (channel, data_dir) =
                provision_test_channel_with_send(seen, Some(PathBuf::from(CLI_EXE_NAME)));
            let needs = ControlChannelNeeds {
                accepts_mcp_config: accepts_mcp_config(&command),
                writes_mcp_config_file: writes_mcp_config_file(&command),
                uses_mail: uses_mail(&command),
            };
            assert!(
                needs.accepts_mcp_config,
                "{name}: premise — both rows are the MCP mail branch (so this test splits the mail axis from the file axis)"
            );
            let id = AgentId::new_v4();
            let ep = channel
                .provision(id, 0, needs)
                .expect("provision ok")
                .expect("endpoint");

            let cfg_dir = data_dir.join("mcp-config");
            if expect_file {
                let cfg = ep.config_path.as_ref().unwrap_or_else(|| {
                    panic!("{name}: declared that it reads the file but config_path is None")
                });
                assert!(cfg.is_file(), "{name}: the declared file must really exist");
                assert!(
                    std::fs::read_to_string(cfg)
                        .expect("read config")
                        .contains(&ep.token),
                    "{name}: what that file carries must be this spawn's token"
                );
                assert!(
                    ep.settings_file.as_ref().is_some_and(|p| p.is_file()),
                    "{name}: the session settings fragment too"
                );
            } else {
                assert_eq!(
                    ep.config_path, None,
                    "{name}: must not put the path of a file it does not read in the endpoint"
                );
                assert_eq!(
                    ep.settings_file, None,
                    "{name}: the same for the settings fragment"
                );
                // Measuring by the folder's absence: `config_path` alone cannot catch a regression that keeps the write and hides the path (the secret stays on disk).
                assert!(
                    !cfg_dir.exists(),
                    "{name}: the mcp-config folder appeared — a plaintext token file nobody reads is being written per spawn: {}",
                    cfg_dir.display()
                );
            }
            channel.revoke(id, 0);
            let _ = std::fs::remove_dir_all(&data_dir);
        }
    }

    /// Measures daemon→backend end to end that the knob turning off MCP mail reaches codex too (found 2026-09-20): the decision is [`DaemonControlChannel::build_grants`], but codex's
    /// MCP attachment once looked only at the control channel's existence, so the knob closed only claude while codex stayed open with `eg_send` auto-approved.
    /// Backend unit tests alone cannot stand this axis (hand-made endpoint); the two tests guard half each. The on row must stand together: off alone stays green
    /// even if the attachment is removed, which removes codex mail's only physical wiring.
    // ADR-0094
    // ADR-0209
    #[test]
    fn disallowing_mcp_send_reaches_the_codex_argv_too() {
        use engram_dashboard_agent::backend::{
            accepts_mcp_config, uses_mail, writes_mcp_config_file, AgentBackend, CodexBackend,
        };
        use engram_dashboard_agent::profile::{AgentCommand, AgentOutputFormat, SpawnMode};

        let _g = lock_env();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());

        let command = AgentCommand::Codex {
            extra_args: vec![],
            output_format: AgentOutputFormat::Terminal,
        };
        let needs = ControlChannelNeeds {
            accepts_mcp_config: accepts_mcp_config(&command),
            writes_mcp_config_file: writes_mcp_config_file(&command),
            uses_mail: uses_mail(&command),
        };
        // Feed it to the real backend — the contract is "does it reach argv", not "was it carried in the endpoint".
        let attaches_mcp = |ep: ControlEndpoint| -> bool {
            CodexBackend
                .build_spec(
                    &command,
                    SpawnMode::Fresh,
                    None,
                    None,
                    PathBuf::from("."),
                    vec![],
                    Some(ep),
                )
                .args
                .iter()
                .any(|a| a.contains("mcp_servers."))
        };

        let (channel, data_dir) = provision_test_channel_with_send(
            Arc::new(Mutex::new(false)),
            Some(PathBuf::from(CLI_EXE_NAME)),
        );

        let on = channel
            .provision(AgentId::new_v4(), 0, needs)
            .expect("provision ok")
            .expect("endpoint");
        assert!(
            attaches_mcp(on),
            "the codex attachment dropped out in the production default — this backend's send entrance becomes 0"
        );

        std::env::set_var(DISALLOW_MCP_ENV, "1");
        let off = channel.provision(AgentId::new_v4(), 0, needs);
        std::env::remove_var(DISALLOW_MCP_ENV);
        let off = off.expect("provision ok").expect("endpoint");
        assert!(
            off.grants.is_empty(),
            "the knob is on but a send grant remains (this test's premise): {:?}",
            off.grants
        );
        assert!(
            !attaches_mcp(off),
            "the knob is on but the MCP attachment was carried in the codex argv — the daemon's block bypasses only this backend"
        );

        let _ = std::fs::remove_dir_all(&data_dir);
    }

    // ── ADR-0133: wiring for everyone, only mail splits (both branches end to end) ─────────────────────────

    /// Feeds a real backend, not a dummy endpoint: the contract is "does it reach the spawn env", and a hand-made endpoint cannot catch half of provision being cut.
    ///
    /// `profile_env` = the profile env the spawn path puts in before the backend. It must be non-empty — with 0 entries nothing shows "we neither added nor removed" for the PATH-interference assertion.
    fn spawn_spec_from(
        id: AgentId,
        ep: ControlEndpoint,
        profile_env: Vec<(String, String)>,
    ) -> engram_dashboard_agent::types::CommandSpec {
        use engram_dashboard_agent::backend::{AgentBackend, ClaudeBackend};
        use engram_dashboard_agent::profile::{AgentCommand, AgentOutputFormat, SpawnMode};
        ClaudeBackend.build_spec(
            &AgentCommand::Claude {
                extra_args: vec![],
                output_format: AgentOutputFormat::StreamJson,
            },
            SpawnMode::Fresh,
            Some(id),
            None,
            PathBuf::from("."),
            profile_env,
            Some(ep),
        )
    }

    /// Checks end to end that the two branches get the same wiring (real backend, down to the spawn env). Not covered: the boot sibling exe search (`lib.rs::locate_send_exe`).
    #[test]
    fn provision_mcp_capable_wires_the_cli_and_withholds_mail() {
        let _g = lock_env();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());
        let seen = Arc::new(Mutex::new(false));
        // Must be an absolute path — the parent directory is the PATH prepend target; a bare name has no parent and the PATH injection assertion fails.
        let send_exe = PathBuf::from("C:/app/engram.exe");
        let (channel, data_dir) =
            provision_test_channel_with_send(seen.clone(), Some(send_exe.clone()));
        let id = AgentId::new_v4();
        let ep = channel
            .provision(id, 0, needs(true))
            .expect("provision ok")
            .expect("endpoint");
        assert!(
            *seen.lock().unwrap(),
            "this guard means something only for the MCP-capable branch (priming A)"
        );
        assert_eq!(ep.send_exe.as_deref(), Some(send_exe.as_path()));
        assert!(
            !ep.mail_allowed,
            "the daemon refuses CLI mail from an MCP-capable spawn"
        );
        let spec = spawn_spec_from(id, ep, vec![("PATH".to_string(), "C:\\custom".to_string())]);
        for key in ["ENGRAM_TOKEN", "ENGRAM_CONTROL_URL", CLI_EXE_ENV] {
            assert!(
                spec.env.iter().any(|(k, _)| k == key),
                "an MCP-capable spawn must get the control CLI entrance too ({key}): {:?}",
                spec.env
            );
        }
        let path = spec
            .env
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("PATH"))
            .map(|(_, v)| v.as_str())
            .expect("PATH injected when send_exe exists");
        assert_eq!(
            std::env::split_paths(path).next(),
            Some(PathBuf::from("C:/app")),
            "PATH front = the CLI sibling directory: {path}"
        );
        assert!(
            spec.args.iter().any(|a| a == "--mcp-config"),
            "the MCP branch must get mcp-config too: {:?}",
            spec.args
        );
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    /// Pair of the test above (it would stay green with the decision hardcoded false): if this branch degrades, non-MCP backends lose their only mail entrance (they have no MCP entrance at all).
    #[test]
    fn provision_non_mcp_wires_the_same_cli_and_allows_mail() {
        let _g = lock_env();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());
        let seen = Arc::new(Mutex::new(false));
        let send_exe = PathBuf::from("C:/app/engram.exe");
        let (channel, data_dir) =
            provision_test_channel_with_send(seen.clone(), Some(send_exe.clone()));
        let id = AgentId::new_v4();
        let ep = channel
            .provision(id, 0, needs(false))
            .expect("provision ok")
            .expect("endpoint");
        assert!(
            !*seen.lock().unwrap(),
            "this guard means something only for the non-MCP branch (no priming injected)"
        );
        assert!(
            ep.mail_allowed,
            "a non-MCP spawn's mail entrance is the CLI"
        );
        let spec = spawn_spec_from(id, ep, vec![]);
        assert_eq!(
            spec.env
                .iter()
                .find(|(k, _)| k == CLI_EXE_ENV)
                .map(|(_, v)| v.as_str()),
            Some("C:/app/engram.exe"),
            "the CLI absolute path must be carried in the spawn env as ENGRAM_CLI_EXE: {:?}",
            spec.env
        );
        assert!(
            spec.env.iter().any(|(k, _)| k == "ENGRAM_TOKEN")
                && spec.env.iter().any(|(k, _)| k == "ENGRAM_CONTROL_URL"),
            "the credential the CLI entrance attaches with must be carried too: {:?}",
            spec.env
        );
        let path = spec
            .env
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("PATH"))
            .map(|(_, v)| v.as_str())
            .expect("send_exe → PATH must be injected");
        assert_eq!(
            std::env::split_paths(path).next(),
            Some(PathBuf::from("C:/app")),
            "PATH front = the CLI sibling directory: {path}"
        );
        assert!(
            !spec.args.iter().any(|a| a == "--mcp-config"),
            "no mcp-config in the non-MCP branch: {:?}",
            spec.args
        );
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    /// A control channel issued without the CLI entrance must be observable (ADR-0133): fail-open is kept (the spawn is not blocked), but without a log line the
    /// "open to everyone" decision and reality diverge. Checks ① the warning really fires and ② control verbs really become impossible — the warning alone breaks on wording edits, the state alone misses a silence regression.
    // ADR-0133
    #[test]
    fn provision_without_a_cli_inlet_warns_and_leaves_control_unreachable() {
        use std::sync::Mutex as StdMutex;
        use tracing::subscriber;

        /// A minimal collector gathering only warning bodies — this test looks only at "did a WARN event carrying that field occur".
        #[derive(Default)]
        struct WarnCollector {
            lines: Arc<StdMutex<Vec<String>>>,
        }
        struct Visit<'a>(&'a mut String);
        impl tracing::field::Visit for Visit<'_> {
            fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
                self.0.push_str(&format!("{}={:?} ", f.name(), v));
            }
        }
        impl subscriber::Subscriber for WarnCollector {
            fn enabled(&self, m: &tracing::Metadata<'_>) -> bool {
                *m.level() == tracing::Level::WARN
            }
            fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::Id {
                tracing::Id::from_u64(1)
            }
            fn record(&self, _: &tracing::Id, _: &tracing::span::Record<'_>) {}
            fn record_follows_from(&self, _: &tracing::Id, _: &tracing::Id) {}
            fn event(&self, event: &tracing::Event<'_>) {
                let mut buf = String::new();
                event.record(&mut Visit(&mut buf));
                self.lines.lock().unwrap().push(buf);
            }
            fn enter(&self, _: &tracing::Id) {}
            fn exit(&self, _: &tracing::Id) {}
        }

        let _g = lock_env();
        let lines: Arc<StdMutex<Vec<String>>> = Arc::default();
        let seen = Arc::new(Mutex::new(false));
        // send_exe=None + MCP-capable = a deployment that could not find the sibling exe. Non-MCP is fail-closed here and does not reach this
        //   (`provision_non_mcp_with_no_send_exe_fails_closed` above).
        let (channel, data_dir) = provision_test_channel_with_send(seen.clone(), None);
        let id = AgentId::new_v4();
        let ep = subscriber::with_default(
            WarnCollector {
                lines: lines.clone(),
            },
            || channel.provision(id, 0, needs(true)),
        )
        .expect("fail-open kept — no blocking the spawn over missing control")
        .expect("endpoint");

        let warned = lines.lock().unwrap().join("\n");
        assert!(
            warned.contains(CLI_EXE_NAME),
            "issuing a control channel without the CLI entrance must warn: {warned:?}"
        );
        let spec = spawn_spec_from(id, ep, vec![]);
        assert!(
            !spec.env.iter().any(|(k, _)| k == CLI_EXE_ENV)
                && !spec.env.iter().any(|(k, _)| k.eq_ignore_ascii_case("PATH")),
            "that spawn really has no means to call control verbs (the state the warning points to): {:?}",
            spec.env
        );
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    /// The credential (enforcement) and the endpoint (record) must come from the same value (ADR-0133 decision 2), or the recorded fact and the actual refusal silently diverge.
    #[test]
    fn provision_records_the_same_mail_verdict_on_the_credential_and_the_endpoint() {
        let _g = lock_env();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());
        for accepts_mcp_config in [true, false] {
            let registry = Arc::new(ControlRegistry::new());
            let data_dir = std::env::temp_dir()
                .join(format!("engram-provision-adr0133-{}", AgentId::new_v4()));
            let channel = DaemonControlChannel::new(
                registry.clone(),
                "http://127.0.0.1:1/mcp".to_string(),
                mcp_config::McpDir::new(data_dir.join("mcp-config")),
                Some(PathBuf::from(CLI_EXE_NAME)),
                Arc::new(RecordingPriming {
                    seen: Arc::new(Mutex::new(false)),
                }),
            );
            let id = AgentId::new_v4();
            let ep = channel
                .provision(id, 0, needs(accepts_mcp_config))
                .expect("provision ok")
                .expect("endpoint");
            let bound = registry.validate(&ep.token).expect("the issued credential");
            assert_eq!(
                bound.mail_allowed, ep.mail_allowed,
                "the decision stamped into the credential == the value the endpoint carries (accepts_mcp_config={accepts_mcp_config})"
            );
            assert_eq!(
                ep.mail_allowed, !accepts_mcp_config,
                "mail allowance is the opposite of MCP-capability (accepts_mcp_config={accepts_mcp_config})"
            );
            let _ = std::fs::remove_dir_all(&data_dir);
        }
    }

    // ── ADR-0099 FIX 3: ENGRAM_FORCE_CLI_ONLY_SEND test-seam — forcing the entire false path ──────────────
    //
    // These seam tests describe only the `test-harness` branch: in production that knob does not exist (`force_cli_only` is `const fn … { false }`), and that branch is
    // not tested because the self-dev-dependency turns the feature on in every test build (Cargo.toml `[dev-dependencies]` comment) — a `#[cfg(not(feature = …))]` test
    // would not run or even type-check and would rot silently. The compiler guards that axis.

    #[cfg(feature = "test-harness")]
    #[test]
    fn provision_force_cli_only_seam_runs_entire_false_path() {
        let _g = lock_env();
        assert!(
            std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err(),
            "both envs must be unset on entering the test (leak detection — provision reads both)"
        );
        let seen = Arc::new(Mutex::new(false));
        // Turn send_exe on — the seam makes the spawn CLI-only, so without a CLI entrance it hits the fail-closed edge.
        let (channel, data_dir) =
            provision_test_channel_with_send(seen.clone(), Some(PathBuf::from(CLI_EXE_NAME)));
        std::env::set_var(FORCE_CLI_ENV, "1");
        let id = AgentId::new_v4();
        let result = channel.provision(id, 0, needs(true));
        std::env::remove_var(FORCE_CLI_ENV);
        let ep = result.expect("provision ok").expect("endpoint");
        assert_eq!(
            ep.config_path, None,
            "seam on → config not written (the physical half of the false path)"
        );
        assert!(
            !data_dir.join("mcp-config").exists(),
            "seam on → the mcp-config file is physically absent"
        );
        assert!(
            !*seen.lock().unwrap(),
            "seam on → the same branch as non-MCP, so it does not ask for priming"
        );
        assert_eq!(
            ep.grants,
            vec![ToolGrant::Cli {
                exe: CLI_EXE_NAME.to_string(),
            }],
            "seam on → grants == [Cli] (the permission half)"
        );
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[cfg(feature = "test-harness")]
    #[test]
    fn provision_force_cli_only_empty_value_is_inert() {
        let _g = lock_env();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());
        let seen = Arc::new(Mutex::new(false));
        let (channel, data_dir) =
            provision_test_channel_with_send(seen.clone(), Some(PathBuf::from(CLI_EXE_NAME)));
        std::env::set_var(FORCE_CLI_ENV, "");
        let id = AgentId::new_v4();
        let result = channel.provision(id, 0, needs(true));
        std::env::remove_var(FORCE_CLI_ENV);
        let ep = result.expect("provision ok").expect("endpoint");
        assert!(
            ep.config_path.is_some(),
            "empty value = seam not triggered → MCP-capable today's behavior (config Some)"
        );
        assert!(
            *seen.lock().unwrap(),
            "empty value → priming seam called (today's behavior)"
        );
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    /// Pins that the authorization decision hangs on that knob (ADR-0133): if the knob moved only the channel and not `mail_allowed`, a spawn the harness believes measures CLI mail would
    /// really be refused. This also states "why gating is needed" for the production-absence pin above.
    // ADR-0133
    #[cfg(feature = "test-harness")]
    #[test]
    fn the_forcing_knob_moves_the_mail_verdict_too_which_is_why_it_is_gated() {
        let _g = lock_env();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());
        let seen = Arc::new(Mutex::new(false));
        let (channel, data_dir) =
            provision_test_channel_with_send(seen.clone(), Some(PathBuf::from(CLI_EXE_NAME)));
        std::env::set_var(FORCE_CLI_ENV, "1");
        let id = AgentId::new_v4();
        let result = channel.provision(id, 0, needs(true));
        std::env::remove_var(FORCE_CLI_ENV);
        let ep = result.expect("provision ok").expect("endpoint");
        assert!(
            ep.mail_allowed,
            "seam on → even the mail decision moves to the CLI side (that path is what the harness wants to measure)"
        );
        let _ = std::fs::remove_dir_all(&data_dir);
    }
}
