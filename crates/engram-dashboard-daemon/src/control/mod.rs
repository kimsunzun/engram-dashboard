//! 제어 채널(ADR-0086 스텝 1) — 스폰 에이전트가 붙는 데몬 제어 평면.
//!
//! tauri import 0(daemon crate).

pub mod agent;
pub mod ingress;
pub mod mcp_config;
pub mod mcp_server;
pub mod priming;
pub mod registry;

use std::path::PathBuf;
use std::sync::Arc;

use engram_dashboard_core::agent::types::{
    AgentId, ControlChannel, ControlEndpoint, ProvisionError, ToolGrant, CLI_EXE_NAME,
};

use mcp_config::MCP_SERVER_NAME;
use mcp_server::SEND_MESSAGE_TOOL;
use priming::{PrimingProvider, PrimingVariant};
use registry::ControlRegistry;

pub struct DaemonControlChannel {
    registry: Arc<ControlRegistry>,
    mcp_url: String,
    data_dir: PathBuf,
    send_exe: Option<PathBuf>,
    priming: Arc<dyn PrimingProvider>,
}

impl DaemonControlChannel {
    pub fn new(
        registry: Arc<ControlRegistry>,
        mcp_url: String,
        data_dir: PathBuf,
        send_exe: Option<PathBuf>,
        priming: Arc<dyn PrimingProvider>,
    ) -> Self {
        Self {
            registry,
            mcp_url,
            data_dir,
            send_exe,
            priming,
        }
    }

    /// ★`ENGRAM_DISALLOW_MCP_SEND` = test-only 노브(운영 스위치 아님)★: 설정 + non-empty 면 MCP
    ///   send_message grant 를 **뺀다**.
    /// ★이 노브로는 CLI-only 라우팅을 만들 수 없다★: MCP 가능 스폰엔 CLI 배선도 CLI grant 도 없어
    ///   (ADR-0128) MCP grant 를 빼면 **발신 grant 가 0** 이 된다. 게다가 스폰은 auto 권한 모드라 grant
    ///   자체가 NO-OP 이다(ADR-0097) — 실측(2026-08-03, 6/6)에서 이 노브를 켠 에이전트가 전부 정상
    ///   발신했다. CLI 라우팅 실측은 물리를 가르는 `ENGRAM_FORCE_CLI_ONLY_SEND`(provision)로만 성립한다.
    // ADR-0094
    fn build_grants(
        send_exe: Option<&std::path::Path>,
        accepts_mcp_config: bool,
    ) -> Vec<ToolGrant> {
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
        // exe 는 send_exe 에서 파생하지 않는다 — grant 문자열은 프라이밍이 가르치는 bare 명령 이름과
        //   글자 그대로 같아야 해서 `CLI_EXE_NAME`(정본)을 쓰고, send_exe 는 CLI 입구 **존재 여부**만
        //   판정에 쓴다(절대경로를 넣으면 bare 이름 호출과 매칭되지 않는다 — ADR-0098).
        // ADR-0128
        if !accepts_mcp_config && send_exe.is_some() {
            grants.push(ToolGrant::Cli {
                exe: CLI_EXE_NAME.to_string(),
            });
        }
        grants
    }

    /// lib.rs `generate_token` 과 방식이 같지만 그건 WS 클라이언트 토큰(daemon.json)용이다 — 공용화
    /// 금지(ADR-0086 §맥락).
    fn gen_token() -> Option<String> {
        let mut buf = [0u8; 32];
        getrandom::getrandom(&mut buf).ok()?;
        let mut s = String::with_capacity(64);
        for b in buf {
            use std::fmt::Write as _;
            let _ = write!(s, "{b:02x}");
        }
        Some(s)
    }
}

impl ControlChannel for DaemonControlChannel {
    fn provision(
        &self,
        id: AgentId,
        epoch: u32,
        accepts_mcp_config: bool,
    ) -> Result<Option<ControlEndpoint>, ProvisionError> {
        let token = Self::gen_token()
            .ok_or_else(|| ProvisionError("CSPRNG token generation failed".to_string()))?;
        // `ENGRAM_FORCE_CLI_ONLY_SEND` = test-only 노브(운영 스위치 아님) — 스폰을 **비-MCP 로 강제**해
        //   false path 전체를 실측한다.
        //   ★ENGRAM_PRIMING_FILE override 와 손으로 조합 금지★: override 가 MCP 교육 파일을 물리면
        //     교육=MCP · 물리=CLI 로 갈려 정합 불변식을 정면 위반한다.
        let force_cli_only = std::env::var("ENGRAM_FORCE_CLI_ONLY_SEND")
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        let accepts_mcp_config = accepts_mcp_config && !force_cli_only;
        // ADR-0099
        let (config_path, settings_file, priming_variant) = if accepts_mcp_config {
            let path = mcp_config::write_config(&self.data_dir, id, epoch, &self.mcp_url, &token)
                .map_err(|e| {
                    tracing::warn!(agent = %id, epoch, "mcp-config 기록 실패 — fail-closed(스폰 중단): {e}");
                    ProvisionError(format!("mcp-config write failed: {e}"))
                })?;
            // ★이 조각의 write 실패는 치명이 아니다(mcp-config 와 다른 판단 — load-bearing)★: mcp-config
            //   가 없으면 MCP 채널이 물리적으로 사라지지만, 이 조각은 "유저 전역 차단을 뒤집는 보정"
            //   이라 없어도 전역 설정이 허용이면 정상 동작한다. 그 열화로 스폰을 막으면 회귀라 warn 만
            //   남기고 조각 없이 진행한다.
            let settings = match mcp_config::write_settings(&self.data_dir, id, epoch) {
                Ok(p) => Some(p),
                Err(e) => {
                    tracing::warn!(
                        agent = %id, epoch,
                        "세션 설정 조각 기록 실패 — 조각 없이 스폰 진행(전역 allowedMcpServers 가 차단이면 engram 툴이 안 보일 수 있음): {e}"
                    );
                    None
                }
            };
            (Some(path), settings, PrimingVariant::McpPrimary)
        } else {
            (None, None, PrimingVariant::CliOnly)
        };
        tracing::debug!(
            agent = %id,
            epoch,
            accepts_mcp_config,
            force_cli_only,
            ?priming_variant,
            has_mcp_config = config_path.is_some(),
            "제어 채널 provision fork(ADR-0099 채널 스위치)"
        );
        if !accepts_mcp_config && self.send_exe.is_none() {
            let msg = format!(
                "non-MCP backend with no `{CLI_EXE_NAME}` binary — zero physical send channels while CLI-only priming teaches that command (pairing invariant violation)"
            );
            tracing::warn!(agent = %id, epoch, "제어 채널 provision fail-closed(ADR-0099): {msg}");
            // 회수 코드가 없는 이유: 이 분기는 !accepts_mcp_config 일 때만 참이라 위 write_config 를 타지
            //   않았고 token 도 아직 issue 전이다 — 되돌릴 자원이 없다.
            return Err(ProvisionError(msg));
        }
        self.registry.issue(id, epoch, token.clone());
        let priming_file = self.priming.priming_file(priming_variant);
        let grants = Self::build_grants(self.send_exe.as_deref(), accepts_mcp_config);
        Ok(Some(ControlEndpoint {
            url: self.mcp_url.clone(),
            token,
            config_path,
            // MCP 갈래에서도 값을 지우지 않는다 — 채널 배타성을 backend 의 `config_path` 갈림 **한 곳**
            //   에만 두어 두 군데가 어긋날 여지를 없앤다(ADR-0128).
            send_exe: self.send_exe.clone(),
            priming_file,
            grants,
            settings_file,
        }))
    }

    fn revoke(&self, id: AgentId, epoch: u32) {
        self.registry.revoke(id, epoch);
        mcp_config::remove_config(&self.data_dir, id, epoch);
        mcp_config::remove_settings(&self.data_dir, id, epoch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engram_dashboard_core::agent::types::CLI_EXE_ENV;
    use std::path::Path;

    /// ★노브별 락이 아니라 **단일** 락★: `provision` 이 두 env 를 모두 읽으므로, 노브별로 나누면 한쪽만
    ///   잡은 reader 가 다른 쪽 setter 와 경합해 플레이키해진다. 어느 knob 이든 건드리는 테스트는 전부
    ///   이 하나를 잡는다.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// ★poison 을 복구한다 — plain `.unwrap()` 으로 되돌리지 말 것(실측 2026-08-04)★: 락을 든 테스트가
    ///   패닉하면 뒤 홀더 전부가 **락 줄에서** 패닉해, 진짜 실패 1건이 구별 불가능한 17건으로 불어나고
    ///   원래 assert 위치가 묻힌다(당시 isolation 실행으로만 진단됐다). 값이 `()` 라 복구해도 물려받을
    ///   오염 상태가 없고, 직렬화는 뮤텍스가 그대로 지킨다.
    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }
    const DISALLOW_MCP_ENV: &str = "ENGRAM_DISALLOW_MCP_SEND";
    const FORCE_CLI_ENV: &str = "ENGRAM_FORCE_CLI_ONLY_SEND";

    // ── ADR-0094: build_grants — 발신 입구 pre-authorization grant 산출(단일 출처·최소권한) ──────

    #[test]
    fn build_grants_mcp_always_present_with_channel_names() {
        let _g = lock_env();
        assert!(
            std::env::var(DISALLOW_MCP_ENV).is_err(),
            "테스트 진입 시 env 미설정이어야(leak 감지)"
        );
        let grants = DaemonControlChannel::build_grants(None, true);
        assert_eq!(
            grants,
            vec![ToolGrant::Mcp {
                server: MCP_SERVER_NAME.to_string(),
                tool: SEND_MESSAGE_TOOL.to_string(),
            }],
            "send_exe=None 이면 MCP grant 하나만(CLI 입구 없음)"
        );
    }

    #[test]
    fn build_grants_cli_grant_follows_the_channel_axis_not_send_exe_presence() {
        let _g = lock_env();
        assert!(std::env::var(DISALLOW_MCP_ENV).is_err());
        let exe = Path::new("C:/app/engram.exe");
        assert_eq!(
            DaemonControlChannel::build_grants(Some(exe), true),
            vec![ToolGrant::Mcp {
                server: MCP_SERVER_NAME.to_string(),
                tool: SEND_MESSAGE_TOOL.to_string(),
            }],
            "MCP-capable + send_exe → [Mcp] 만(CLI 배선이 없으니 CLI grant 도 없다)"
        );
        assert_eq!(
            DaemonControlChannel::build_grants(Some(exe), false),
            vec![ToolGrant::Cli {
                exe: CLI_EXE_NAME.to_string(),
            }],
            "비-MCP + send_exe → [Cli](CLI grant 는 이 축으로만 나온다)"
        );
    }

    #[test]
    fn build_grants_is_minimal_privilege() {
        let _g = lock_env();
        assert!(std::env::var(DISALLOW_MCP_ENV).is_err());
        let exe = Path::new("C:/app/engram.exe");
        for accepts_mcp_config in [true, false] {
            let grants = DaemonControlChannel::build_grants(Some(exe), accepts_mcp_config);
            assert_eq!(
                grants.len(),
                1,
                "발신 입구 하나만(accepts_mcp_config={accepts_mcp_config}): {grants:?}"
            );
            match &grants[0] {
                ToolGrant::Mcp { tool, .. } => assert_eq!(tool, SEND_MESSAGE_TOOL),
                ToolGrant::Cli { exe } => assert_eq!(exe, CLI_EXE_NAME),
            }
        }
    }

    // ── ADR-0094 test-seam: ENGRAM_DISALLOW_MCP_SEND — MCP grant 제거 ──────────────────────────────

    #[test]
    fn build_grants_disallow_mcp_env_on_mcp_capable_leaves_no_send_grant() {
        let _g = lock_env();
        assert!(
            std::env::var(DISALLOW_MCP_ENV).is_err(),
            "테스트 진입 시 env 미설정이어야(leak 감지)"
        );
        std::env::set_var(DISALLOW_MCP_ENV, "1");
        let exe = Path::new("C:/app/engram.exe");
        let mcp_capable = DaemonControlChannel::build_grants(Some(exe), true);
        let non_mcp = DaemonControlChannel::build_grants(Some(exe), false);
        std::env::remove_var(DISALLOW_MCP_ENV);
        assert!(
            mcp_capable.is_empty(),
            "env 켜짐 + MCP 가능 → 발신 grant 0: {mcp_capable:?}"
        );
        assert_eq!(
            non_mcp,
            vec![ToolGrant::Cli {
                exe: CLI_EXE_NAME.to_string(),
            }],
            "env 켜짐이어도 비-MCP 갈래의 CLI grant 는 그대로(seam 은 MCP grant 만 제거)"
        );
    }

    #[test]
    fn build_grants_disallow_mcp_env_with_no_send_exe_yields_empty() {
        let _g = lock_env();
        assert!(std::env::var(DISALLOW_MCP_ENV).is_err());
        std::env::set_var(DISALLOW_MCP_ENV, "1");
        let grants = DaemonControlChannel::build_grants(None, true);
        std::env::remove_var(DISALLOW_MCP_ENV);
        assert!(
            grants.is_empty(),
            "env 켜짐 + send_exe 부재 → 발신 grant 0(제거만, 추가 없음): {grants:?}"
        );
    }

    #[test]
    fn build_grants_disallow_mcp_empty_value_is_production_default() {
        let _g = lock_env();
        assert!(std::env::var(DISALLOW_MCP_ENV).is_err());
        std::env::set_var(DISALLOW_MCP_ENV, "");
        let grants = DaemonControlChannel::build_grants(Some(Path::new("C:/app/engram.exe")), true);
        std::env::remove_var(DISALLOW_MCP_ENV);
        assert_eq!(
            grants,
            vec![ToolGrant::Mcp {
                server: MCP_SERVER_NAME.to_string(),
                tool: SEND_MESSAGE_TOOL.to_string(),
            }],
            "빈 값 = seam 미발동 → MCP 가능 갈래의 운영 grant([Mcp] — ADR-0128)"
        );
    }

    // ── ADR-0099: 채널별 grant 방출 — 비-MCP 백엔드는 MCP grant 를 방출하지 않는다 ──────────────────

    #[test]
    fn build_grants_non_mcp_backend_emits_cli_only() {
        let _g = lock_env();
        assert!(std::env::var(DISALLOW_MCP_ENV).is_err());
        let exe = Path::new("C:/app/engram.exe");
        let grants = DaemonControlChannel::build_grants(Some(exe), false);
        assert_eq!(
            grants,
            vec![ToolGrant::Cli {
                exe: CLI_EXE_NAME.to_string(),
            }],
            "비-MCP 백엔드 → CLI grant 만(MCP 입구 물리 부재)"
        );
    }

    #[test]
    fn build_grants_non_mcp_backend_no_send_exe_yields_empty() {
        let _g = lock_env();
        assert!(std::env::var(DISALLOW_MCP_ENV).is_err());
        let grants = DaemonControlChannel::build_grants(None, false);
        assert!(
            grants.is_empty(),
            "비-MCP + send_exe 부재 → 발신 grant 0: {grants:?}"
        );
    }

    // ── ADR-0099: provision 분기 — 채널 물리 배선 + 프라이밍 변형이 MCP-capability 로 함께 움직인다 ──────

    use crate::control::priming::{PrimingProvider, PrimingVariant};
    use std::sync::{Arc, Mutex};

    struct RecordingPriming {
        seen: Arc<Mutex<Option<PrimingVariant>>>,
    }
    impl PrimingProvider for RecordingPriming {
        fn priming_file(&self, variant: PrimingVariant) -> Option<PathBuf> {
            *self.seen.lock().unwrap() = Some(variant);
            Some(PathBuf::from(match variant {
                PrimingVariant::McpPrimary => "A-mcp-primary",
                PrimingVariant::CliOnly => "B-cli-only",
            }))
        }
    }

    fn provision_test_channel_with_send(
        seen: Arc<Mutex<Option<PrimingVariant>>>,
        send_exe: Option<PathBuf>,
    ) -> (DaemonControlChannel, PathBuf) {
        let data_dir =
            std::env::temp_dir().join(format!("engram-provision-adr0099-{}", AgentId::new_v4()));
        let channel = DaemonControlChannel::new(
            Arc::new(ControlRegistry::new()),
            "http://127.0.0.1:1/mcp".to_string(),
            data_dir.clone(),
            send_exe,
            Arc::new(RecordingPriming { seen }),
        );
        (channel, data_dir)
    }

    #[test]
    fn provision_mcp_capable_writes_config_and_picks_mcp_primary_priming() {
        let _g = lock_env();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());
        let seen = Arc::new(Mutex::new(None));
        let (channel, data_dir) =
            provision_test_channel_with_send(seen.clone(), Some(PathBuf::from(CLI_EXE_NAME)));
        let id = AgentId::new_v4();
        let ep = channel
            .provision(id, 0, true)
            .expect("provision ok")
            .expect("endpoint");
        let cfg = ep
            .config_path
            .as_ref()
            .expect("MCP-capable → config_path Some");
        assert!(cfg.is_file(), "MCP-capable → mcp-config 파일 물리 존재");
        assert_eq!(
            *seen.lock().unwrap(),
            Some(PrimingVariant::McpPrimary),
            "MCP-capable → McpPrimary 프라이밍 변형"
        );
        assert_eq!(ep.priming_file, Some(PathBuf::from("A-mcp-primary")));
        let settings = ep
            .settings_file
            .as_ref()
            .expect("MCP-capable → settings_file Some");
        assert!(settings.is_file(), "설정 조각 파일 물리 존재");
        let content = std::fs::read_to_string(settings).expect("read settings");
        assert!(
            content.contains("allowedMcpServers") && content.contains(MCP_SERVER_NAME),
            "조각이 engram 서버를 허용해야: {content}"
        );
        channel.revoke(id, 0);
        assert!(!settings.exists(), "revoke 시 설정 조각 삭제");
        assert!(!cfg.exists(), "revoke 시 mcp-config 삭제");
        channel.revoke(id, 0); // 이중 revoke 안전.
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn provision_non_mcp_skips_config_and_picks_cli_only_priming() {
        let _g = lock_env();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());
        let seen = Arc::new(Mutex::new(None));
        let (channel, data_dir) =
            provision_test_channel_with_send(seen.clone(), Some(PathBuf::from(CLI_EXE_NAME)));
        let id = AgentId::new_v4();
        let ep = channel
            .provision(id, 0, false)
            .expect("provision ok")
            .expect("endpoint");
        assert_eq!(
            ep.config_path, None,
            "비-MCP → mcp-config 미기록(config_path=None, 타입-인코딩 부재)"
        );
        assert!(
            !data_dir.join("mcp-config").exists(),
            "비-MCP → mcp-config 파일이 물리적으로 없어야"
        );
        assert_eq!(
            ep.settings_file, None,
            "비-MCP → settings_file 도 None(정합 불변식: 깐 채널 == 허용한 채널)"
        );
        assert_eq!(
            *seen.lock().unwrap(),
            Some(PrimingVariant::CliOnly),
            "비-MCP → CliOnly 프라이밍 변형"
        );
        assert_eq!(ep.priming_file, Some(PathBuf::from("B-cli-only")));
        assert_eq!(
            ep.grants,
            vec![ToolGrant::Cli {
                exe: CLI_EXE_NAME.to_string(),
            }],
            "비-MCP → grants == [Cli]"
        );
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    // ── ADR-0099 FIX 2: fail-closed edge — 비-MCP + send_exe=None = 채널 0 → ProvisionError ──────
    #[test]
    fn provision_non_mcp_with_no_send_exe_fails_closed() {
        let _g = lock_env();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());
        let seen = Arc::new(Mutex::new(None));
        let (channel, data_dir) = provision_test_channel_with_send(seen.clone(), None);
        let id = AgentId::new_v4();
        let err = channel
            .provision(id, 0, false)
            .expect_err("비-MCP + send_exe=None → fail-closed Err");
        assert!(
            err.0.contains("non-MCP") && err.0.contains(CLI_EXE_NAME),
            "ProvisionError 사유에 원인 명시: {}",
            err.0
        );
        assert!(
            !data_dir.join("mcp-config").exists(),
            "fail-closed edge → config 파일 미생성"
        );
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    /// ★provision 의 config-write `?` 를 fail-open 으로 바꿔도 다른 스위트는 안 깨진다 — 그래서 이 테스트가
    ///   있다★: 그 순간 MCP-capable 스폰이 `config_path=None` 으로 나와 backend 가 CLI 전용으로 읽는다.
    #[test]
    fn provision_mcp_capable_fails_closed_when_config_write_fails() {
        let _g = lock_env();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());
        let seen = Arc::new(Mutex::new(None));
        let (channel, data_dir) = provision_test_channel_with_send(
            seen.clone(),
            Some(PathBuf::from("C:/app/engram.exe")),
        );
        let id = AgentId::new_v4();
        let cfg_dir = mcp_config::config_path(&data_dir, id, 0)
            .parent()
            .expect("config 경로엔 부모 폴더가 있다")
            .to_path_buf();
        std::fs::create_dir_all(&data_dir).expect("data_dir 생성");
        std::fs::write(&cfg_dir, b"occupied").expect("폴더 자리를 파일로 점유");

        let err = channel
            .provision(id, 0, true)
            .expect_err("MCP-capable + config write 실패 → fail-closed Err");
        assert!(
            err.0.contains("mcp-config write failed"),
            "ProvisionError 사유에 원인 명시: {}",
            err.0
        );
        assert!(
            seen.lock().unwrap().is_none(),
            "fail-closed 면 프라이밍 변형 선택까지 가지 않는다(반쪽 provision 금지)"
        );
        let _ = std::fs::remove_file(&cfg_dir);
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn provision_mcp_capable_with_no_send_exe_is_allowed() {
        let _g = lock_env();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());
        let seen = Arc::new(Mutex::new(None));
        let (channel, data_dir) = provision_test_channel_with_send(seen.clone(), None);
        let id = AgentId::new_v4();
        let ep = channel
            .provision(id, 0, true)
            .expect("MCP-capable + send_exe=None 은 허용(accepted edge)")
            .expect("endpoint");
        assert!(
            ep.config_path.is_some(),
            "MCP-capable → config_path Some(MCP 입구 살아 있음)"
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

    // ── ADR-0128: 우편 채널 하드 단일화 — 배선 집합 = 교육 집합(등호, 두 갈래를 end-to-end 로) ────────

    /// 더미 endpoint 가 아니라 **실** backend 에 먹인다 — 계약이 "endpoint 에 실렸나" 가 아니라 "스폰 env 에
    /// 닿나" 라서, 손으로 만든 endpoint 로는 provision 절반이 끊긴 것을 못 잡는다.
    ///
    /// `profile_env` = 스폰 경로가 backend 보다 **먼저** 넣는 프로필 env. PATH 무개입을 단언하려면 비워서는
    ///   안 된다 — 항목이 0 개면 "우리가 얹지도 깎지도 않았다" 를 견줄 대상이 없어 단언이 공허해진다.
    fn spawn_spec_from(
        id: AgentId,
        ep: ControlEndpoint,
        profile_env: Vec<(String, String)>,
    ) -> engram_dashboard_core::agent::types::CommandSpec {
        use engram_dashboard_core::agent::backend::{AgentBackend, ClaudeBackend};
        use engram_dashboard_core::agent::profile::{AgentCommand, ClaudeOutputFormat, SpawnMode};
        ClaudeBackend.build_spec(
            &AgentCommand::Claude {
                extra_args: vec![],
                output_format: ClaudeOutputFormat::StreamJson,
            },
            SpawnMode::Fresh,
            Some(id),
            PathBuf::from("."),
            profile_env,
            Some(ep),
        )
    }

    /// ★endpoint 에 send_exe 가 **실린** 상태로 부재를 단언한다 — 값을 빼고 단언하면 아무것도 검증하지
    ///   않는다★(배타성을 만드는 건 backend 의 `config_path` 갈림뿐이므로).
    /// ★미커버★: 부팅 시 형제 exe 탐색(`lib.rs::locate_send_exe`)은 이 테스트 범위 밖이다.
    #[test]
    fn provision_mcp_capable_does_not_wire_the_cli_into_spawn_env() {
        let _g = lock_env();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());
        let seen = Arc::new(Mutex::new(None));
        // **절대**경로여야 한다 — 부모 디렉토리가 PATH prepend 대상이라, bare 이름이면 부모가 없어 짝
        //   테스트의 PATH 주입이 성립하지 않는다.
        let send_exe = PathBuf::from("C:/app/engram.exe");
        let (channel, data_dir) =
            provision_test_channel_with_send(seen.clone(), Some(send_exe.clone()));
        let id = AgentId::new_v4();
        let ep = channel
            .provision(id, 0, true)
            .expect("provision ok")
            .expect("endpoint");
        assert_eq!(
            *seen.lock().unwrap(),
            Some(PrimingVariant::McpPrimary),
            "MCP 가능 갈래(프라이밍 A)여야 이 가드가 의미를 가진다"
        );
        assert_eq!(
            ep.send_exe.as_deref(),
            Some(send_exe.as_path()),
            "endpoint 는 두 갈래 공용으로 경로를 나른다 — 배선 배타성은 backend 갈림이 만든다"
        );
        assert_eq!(
            ep.grants,
            vec![ToolGrant::Mcp {
                server: MCP_SERVER_NAME.to_string(),
                tool: SEND_MESSAGE_TOOL.to_string(),
            }],
            "MCP 가능 → grants == [Mcp](CLI 권한도 함께 사라진다)"
        );
        let spec = spawn_spec_from(id, ep, vec![("PATH".to_string(), "C:\\custom".to_string())]);
        for key in ["ENGRAM_TOKEN", "ENGRAM_CONTROL_URL", CLI_EXE_ENV] {
            assert!(
                !spec.env.iter().any(|(k, _)| k == key),
                "MCP 가능 스폰 env 에 {key} 가 실리면 ADR-0128 위반: {:?}",
                spec.env
            );
        }
        let paths: Vec<&str> = spec
            .env
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("PATH"))
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(
            paths,
            vec!["C:\\custom"],
            "MCP 가능 스폰의 PATH 는 프로필 값 그대로여야: {:?}",
            spec.env
        );
        assert!(
            spec.args.iter().any(|a| a == "--mcp-config"),
            "MCP 갈래는 mcp-config 를 받아야(등호의 반대 절반): {:?}",
            spec.args
        );
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    /// ★위 테스트만 있으면 "CLI 배선을 통째로 지워도 초록" 이라 짝으로 둔다★ — 이 갈래가 열화하면 비-MCP
    ///   백엔드의 우편이 죽는다(그 백엔드엔 MCP 입구가 아예 없다).
    #[test]
    fn provision_non_mcp_wires_the_cli_into_spawn_env() {
        let _g = lock_env();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());
        let seen = Arc::new(Mutex::new(None));
        let send_exe = PathBuf::from("C:/app/engram.exe");
        let (channel, data_dir) =
            provision_test_channel_with_send(seen.clone(), Some(send_exe.clone()));
        let id = AgentId::new_v4();
        let ep = channel
            .provision(id, 0, false)
            .expect("provision ok")
            .expect("endpoint");
        assert_eq!(
            *seen.lock().unwrap(),
            Some(PrimingVariant::CliOnly),
            "비-MCP 갈래(프라이밍 B)여야 이 가드가 의미를 가진다"
        );
        let spec = spawn_spec_from(id, ep, vec![]);
        assert_eq!(
            spec.env
                .iter()
                .find(|(k, _)| k == CLI_EXE_ENV)
                .map(|(_, v)| v.as_str()),
            Some("C:/app/engram.exe"),
            "CLI 절대경로가 ENGRAM_CLI_EXE 로 스폰 env 에 실려야: {:?}",
            spec.env
        );
        assert!(
            spec.env.iter().any(|(k, _)| k == "ENGRAM_TOKEN")
                && spec.env.iter().any(|(k, _)| k == "ENGRAM_CONTROL_URL"),
            "CLI 입구가 붙을 크레덴셜도 함께 실려야: {:?}",
            spec.env
        );
        let path = spec
            .env
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("PATH"))
            .map(|(_, v)| v.as_str())
            .expect("CLI 전용 스폰 + send_exe → PATH 주입돼야");
        assert_eq!(
            std::env::split_paths(path).next(),
            Some(PathBuf::from("C:/app")),
            "PATH 맨 앞 = CLI 형제 디렉토리: {path}"
        );
        assert!(
            !spec.args.iter().any(|a| a == "--mcp-config"),
            "CLI 전용 갈래엔 mcp-config 없음: {:?}",
            spec.args
        );
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    // ── ADR-0099 FIX 3: ENGRAM_FORCE_CLI_ONLY_SEND test-seam — 전체 false path 강제 ──────────────
    #[test]
    fn provision_force_cli_only_seam_runs_entire_false_path() {
        let _g = lock_env();
        assert!(
            std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err(),
            "테스트 진입 시 두 env 모두 미설정이어야(leak 감지 — provision 이 둘 다 읽음)"
        );
        let seen = Arc::new(Mutex::new(None));
        // send_exe 를 켠다 — seam 이 스폰을 CLI-only 로 만들므로 CLI 입구가 없으면 fail-closed edge 에 걸린다.
        let (channel, data_dir) =
            provision_test_channel_with_send(seen.clone(), Some(PathBuf::from(CLI_EXE_NAME)));
        std::env::set_var(FORCE_CLI_ENV, "1");
        let id = AgentId::new_v4();
        let result = channel.provision(id, 0, true);
        std::env::remove_var(FORCE_CLI_ENV);
        let ep = result.expect("provision ok").expect("endpoint");
        assert_eq!(
            ep.config_path, None,
            "seam 켜짐 → config 미기록(false path 물리 절반)"
        );
        assert!(
            !data_dir.join("mcp-config").exists(),
            "seam 켜짐 → mcp-config 파일 물리 부재"
        );
        assert_eq!(
            *seen.lock().unwrap(),
            Some(PrimingVariant::CliOnly),
            "seam 켜짐 → CliOnly 프라이밍(교육 절반)"
        );
        assert_eq!(
            ep.grants,
            vec![ToolGrant::Cli {
                exe: CLI_EXE_NAME.to_string(),
            }],
            "seam 켜짐 → grants == [Cli](권한 절반)"
        );
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn provision_force_cli_only_empty_value_is_inert() {
        let _g = lock_env();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());
        let seen = Arc::new(Mutex::new(None));
        let (channel, data_dir) =
            provision_test_channel_with_send(seen.clone(), Some(PathBuf::from(CLI_EXE_NAME)));
        std::env::set_var(FORCE_CLI_ENV, "");
        let id = AgentId::new_v4();
        let result = channel.provision(id, 0, true);
        std::env::remove_var(FORCE_CLI_ENV);
        let ep = result.expect("provision ok").expect("endpoint");
        assert!(
            ep.config_path.is_some(),
            "빈 값 = seam 미발동 → MCP-capable 오늘 동작(config Some)"
        );
        assert_eq!(
            *seen.lock().unwrap(),
            Some(PrimingVariant::McpPrimary),
            "빈 값 → McpPrimary(오늘 동작)"
        );
        let _ = std::fs::remove_dir_all(&data_dir);
    }
}
