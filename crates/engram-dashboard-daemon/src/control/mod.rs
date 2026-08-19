//! 제어 채널(ADR-0086 스텝 1) — 스폰 에이전트가 붙는 데몬 제어 평면.
//!
//! tauri import 0(daemon crate).

pub mod agent;
pub mod catalog;
pub mod commands;
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
    /// ★이 노브로는 CLI-only 라우팅을 만들 수 없다★: 우편 가부를 가르는 것은 grant 가 아니라 자격증명이다
    ///   — MCP 가능 스폰의 자격증명으로 온 우편 요청은 데몬이 거절하므로(ADR-0133), MCP grant 를 빼도
    ///   CLI 우편으로 넘어가지 않고 **발신 입구가 0** 이 될 뿐이다. 게다가 스폰은 auto 권한 모드라 grant
    ///   자체가 NO-OP 이다(ADR-0097) — 실측(2026-08-03, 6/6)에서 이 노브를 켠 에이전트가 전부 정상
    ///   발신했다. CLI 라우팅 실측은 채널을 통째로 가르는 `ENGRAM_FORCE_CLI_ONLY_SEND`(provision)로만
    ///   성립한다.
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
        // ★알려진 미충족(의도적, 사용자 결정 대기)★: ADR-0133 이후 프라이밍 A 도 MCP 가능 스폰에
        //   bare `engram`(제어 발견 한 줄)을 가르치지만, 이 갈래는 CLI grant 를 방출하지 않는다 — 즉 위
        //   "가르치는 이름 == grant 이름" 정렬이 **그 갈래에서만 깨져 있다**. 지금 무해한 이유는 스폰이
        //   `--permission-mode bypassPermissions` 아래 돌아 grant 가 NO-OP 이기 때문이고(ADR-0097),
        //   grant 축을 다시 여는 것은 사용자의 별도 결정이라 이 조각에서 손대지 않았다. **이 문장을 지우고
        //   정렬이 성립한다고 읽지 말 것** — base 플래그를 걷는 날 이 갈래가 먼저 깨진다.
        // ADR-0094
        // ADR-0128
        if !accepts_mcp_config && send_exe.is_some() {
            grants.push(ToolGrant::Cli {
                exe: CLI_EXE_NAME.to_string(),
            });
        }
        grants
    }

    /// 스폰을 **비-MCP 로 강제**해 false path 전체를 실측하는 하네스 노브(`ENGRAM_FORCE_CLI_ONLY_SEND`).
    ///
    /// ★운영 빌드는 이 노브를 아예 컴파일하지 않는다(`const fn false`)★: 이 값은 채널 물리 배선·프라이밍
    ///   변형뿐 아니라 **우편 인가 판정(`mail_allowed`)까지** 뒤집는다 — 즉 릴리즈 바이너리가 환경변수
    ///   하나로 인가 결과를 바꿀 수 있게 두면, 에이전트가 자기 env 를 고쳐 데몬의 판정을 옮기는 경로가 생긴다
    ///   (표식과 달리 이쪽은 강제 그 자체다). `test-harness` 는 self-dev-dependency 로만 켜지므로 운영
    ///   dep 그래프엔 유니피케이션되지 않는다 — **그 비유니피케이션과 그것이 깨지는 조건(`--all-targets`)
    ///   둘 다의 정본은 Cargo.toml `[dev-dependencies]` 의 self-dev-dependency 주석이다**(배포 릴리스를
    ///   `--all-targets` 로 만들면 이 노브가 그 바이너리에 박힌다).
    /// ★유일한 소비자 = `roundtrip-smoke`★ — 그 bin 자체가 `required-features = ["test-harness"]` 라
    ///   게이팅으로 실측이 끊기지 않는다.
    // ADR-0133
    #[cfg(feature = "test-harness")]
    fn force_cli_only() -> bool {
        std::env::var("ENGRAM_FORCE_CLI_ONLY_SEND")
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    }

    /// 운영 빌드 갈래 — `const fn` 이라 env 를 읽을 수단 자체가 없다(위 doc 이 사유의 정본).
    ///
    /// ★이 축엔 테스트가 없고, 그게 의도다★: 테스트 빌드는 self-dev-dependency 때문에 feature 가 항상 ON
    ///   이라 `#[cfg(not(feature = …))]` 테스트는 **타입 검사조차 되지 않는다** — 돌지 않는 것을 넘어 조용히
    ///   썩고, 그러면서 "그 축은 테스트가 지킨다" 는 착각을 남긴다. 여기서 축을 지키는 것은 컴파일러다:
    ///   이 함수가 상수를 반환하므로 릴리스 바이너리엔 그 환경변수 이름조차 남지 않는다(검증됨 —
    ///   릴리스 산출물에서 문자열 occurrence 0).
    // ADR-0133
    #[cfg(not(feature = "test-harness"))]
    const fn force_cli_only() -> bool {
        false
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
        // ★ENGRAM_PRIMING_FILE override 와 손으로 조합 금지★: override 가 MCP 교육 파일을 물리면
        //   교육=MCP · 물리=CLI 로 갈려 정합 불변식을 정면 위반한다.
        // 운영 빌드에선 `force_cli_only()` 가 const false 라 아래 식이 `accepts_mcp_config` 그대로다.
        let force_cli_only = Self::force_cli_only();
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
        // ★CLI 입구 없이 제어 채널을 발급하는 경우를 **보이게** 만든다(ADR-0132 결정 5)★: 여기 닿는 것은
        //   `accepts_mcp_config == true` 갈래뿐이다(비-MCP + send_exe 부재는 바로 위에서 fail-closed).
        //   그 스폰은 우편은 MCP 로 멀쩡히 쓰지만 **제어 동사에는 닿을 수 없다** — `engram` 을 부를 수단이
        //   없기 때문이다. fail-open 을 유지하는 것은 기존 판단이고(제어 부재로 스폰을 막지 않는다), 다만
        //   그 상태가 **로그 한 줄도 없이** 지나가면 "전원 개방" 이라 적힌 결정과 실제가 조용히 갈린다.
        //   증상: 에이전트가 `engram: command not found` 를 만나고, 그 원인이 배포 폴더에 있다.
        // ADR-0133
        if self.send_exe.is_none() {
            tracing::warn!(
                agent = %id, epoch,
                "제어 채널을 발급했으나 `{CLI_EXE_NAME}` 실행파일을 찾지 못했다 — 이 스폰은 제어 동사(engram …)를 쓸 수 없다(데몬 exe 형제로 배포됐는지 확인). 우편은 MCP 로 정상 동작한다."
            );
        }
        // ★우편 가부의 **단일 파생 지점**(ADR-0133 결정 2)★: MCP 로 우편을 쓰는 스폰은 CLI 우편을 쓰지
        //   않는다 — 채널은 capability 로만 갈리고 런타임 스위칭·폴백이 없다(ADR-0128 결정 1). 이 한 값이
        //   ① 자격증명에 박히는 강제(데몬 거절)와 ② endpoint 에 실려 나가는 표식(교육)을 **함께** 낳는다.
        //   두 자리가 따로 판정하면 "가르치는 것 ≠ 거절하는 것" 이 되어 조용히 갈린다.
        // ADR-0133
        let mail_allowed = !accepts_mcp_config;
        self.registry.issue(id, epoch, token.clone(), mail_allowed);
        let priming_file = self.priming.priming_file(priming_variant);
        let grants = Self::build_grants(self.send_exe.as_deref(), accepts_mcp_config);
        Ok(Some(ControlEndpoint {
            url: self.mcp_url.clone(),
            token,
            config_path,
            // 두 갈래가 같은 값을 받는다 — 제어 CLI 는 전원에게 깔린다(ADR-0132 결정 5 · ADR-0133).
            send_exe: self.send_exe.clone(),
            priming_file,
            grants,
            settings_file,
            mail_allowed,
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
    use engram_dashboard_core::agent::types::{
        CLI_EXE_ENV, MAIL_MARKER_ENV, MAIL_MARKER_OFF, MAIL_MARKER_ON,
    };
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
            "MCP-capable + send_exe → [Mcp] 만(CLI grant 는 우편 채널 축으로만 나온다)"
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

    // ── ADR-0133: 배선은 전원에게, 우편만 갈린다(두 갈래를 end-to-end 로) ─────────────────────────

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

    /// ★두 갈래가 **같은** 배선을 받는지를 end-to-end 로 본다★: endpoint 필드만 보면 provision 절반이
    ///   끊긴 것을 못 잡으므로 실 backend 에 먹여 스폰 env 까지 확인한다.
    /// ★미커버★: 부팅 시 형제 exe 탐색(`lib.rs::locate_send_exe`)은 이 테스트 범위 밖이다.
    #[test]
    fn provision_mcp_capable_wires_the_cli_and_marks_mail_off() {
        let _g = lock_env();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());
        let seen = Arc::new(Mutex::new(None));
        // **절대**경로여야 한다 — 부모 디렉토리가 PATH prepend 대상이라, bare 이름이면 부모가 없어 PATH
        //   주입 단언이 성립하지 않는다.
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
        assert_eq!(ep.send_exe.as_deref(), Some(send_exe.as_path()));
        assert!(
            !ep.mail_allowed,
            "MCP 가능 스폰의 우편은 데몬이 거절한다 → 표식 off"
        );
        let spec = spawn_spec_from(id, ep, vec![("PATH".to_string(), "C:\\custom".to_string())]);
        for key in ["ENGRAM_TOKEN", "ENGRAM_CONTROL_URL", CLI_EXE_ENV] {
            assert!(
                spec.env.iter().any(|(k, _)| k == key),
                "MCP 가능 스폰도 제어 CLI 입구를 받아야({key}): {:?}",
                spec.env
            );
        }
        assert_eq!(
            spec.env
                .iter()
                .find(|(k, _)| k == MAIL_MARKER_ENV)
                .map(|(_, v)| v.as_str()),
            Some(MAIL_MARKER_OFF),
            "스폰 env 의 표식이 off 여야: {:?}",
            spec.env
        );
        let path = spec
            .env
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("PATH"))
            .map(|(_, v)| v.as_str())
            .expect("send_exe 있으면 PATH 주입");
        assert_eq!(
            std::env::split_paths(path).next(),
            Some(PathBuf::from("C:/app")),
            "PATH 맨 앞 = CLI 형제 디렉토리: {path}"
        );
        assert!(
            spec.args.iter().any(|a| a == "--mcp-config"),
            "MCP 갈래는 mcp-config 도 함께 받아야: {:?}",
            spec.args
        );
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    /// ★위 테스트만 있으면 표식을 상수 off 로 박아도 초록이라 짝으로 둔다★ — 이 갈래가 열화하면 비-MCP
    ///   백엔드의 우편이 사용법에서 사라진다(그 백엔드엔 MCP 입구가 아예 없다).
    #[test]
    fn provision_non_mcp_wires_the_same_cli_and_marks_mail_on() {
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
        assert!(
            ep.mail_allowed,
            "비-MCP 스폰의 우편 입구는 CLI 다 → 표식 on"
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
        assert_eq!(
            spec.env
                .iter()
                .find(|(k, _)| k == MAIL_MARKER_ENV)
                .map(|(_, v)| v.as_str()),
            Some(MAIL_MARKER_ON),
            "스폰 env 의 표식이 on 이어야: {:?}",
            spec.env
        );
        let path = spec
            .env
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("PATH"))
            .map(|(_, v)| v.as_str())
            .expect("send_exe → PATH 주입돼야");
        assert_eq!(
            std::env::split_paths(path).next(),
            Some(PathBuf::from("C:/app")),
            "PATH 맨 앞 = CLI 형제 디렉토리: {path}"
        );
        assert!(
            !spec.args.iter().any(|a| a == "--mcp-config"),
            "비-MCP 갈래엔 mcp-config 없음: {:?}",
            spec.args
        );
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    /// ★CLI 입구 없이 발급된 제어 채널은 **관측 가능해야 한다**(ADR-0133)★: 이 조합은 fail-open 이 유지되는
    ///   정당한 상태지만(스폰을 막지 않는다 — 기존 판단), 로그 한 줄도 없이 지나가면 "제어는 전원 개방"
    ///   이라 적힌 결정과 실제가 조용히 갈린다. 그래서 ① 경고가 실제로 발화하는지와 ② 그때 제어 동사가
    ///   실제로 불가능해지는지를 함께 본다 — 경고만 보면 문구 손질에 깨지고, 상태만 보면 침묵 회귀를
    ///   놓친다.
    // ADR-0133
    #[test]
    fn provision_without_a_cli_inlet_warns_and_leaves_control_unreachable() {
        use std::sync::Mutex as StdMutex;
        use tracing::subscriber;

        /// 경고 본문만 모으는 최소 수집기 — `tracing-subscriber` 의 포맷 레이어를 쓰지 않는 이유는 이
        ///   테스트가 보는 것이 "그 필드가 실린 WARN 이벤트가 났는가" 하나뿐이라서다.
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
        let seen = Arc::new(Mutex::new(None));
        // send_exe=None + MCP-capable = 형제 exe 를 못 찾은 배포. 비-MCP 는 이 조합에서 fail-closed 라
        //   여기 닿지 않는다(위 `provision_non_mcp_with_no_send_exe_fails_closed`).
        let (channel, data_dir) = provision_test_channel_with_send(seen.clone(), None);
        let id = AgentId::new_v4();
        let ep = subscriber::with_default(
            WarnCollector {
                lines: lines.clone(),
            },
            || channel.provision(id, 0, true),
        )
        .expect("fail-open 유지 — 제어 부재로 스폰을 막지 않는다")
        .expect("endpoint");

        let warned = lines.lock().unwrap().join("\n");
        assert!(
            warned.contains(CLI_EXE_NAME),
            "CLI 입구 없이 제어 채널을 발급하면 경고가 나야: {warned:?}"
        );
        let spec = spawn_spec_from(id, ep, vec![]);
        assert!(
            !spec.env.iter().any(|(k, _)| k == CLI_EXE_ENV)
                && !spec.env.iter().any(|(k, _)| k.eq_ignore_ascii_case("PATH")),
            "그 스폰은 실제로 제어 동사를 부를 수단이 없다(경고가 가리키는 상태): {:?}",
            spec.env
        );
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    /// ★교육(표식)과 강제(자격증명)는 **같은 한 값**에서 나와야 한다(ADR-0133 결정 2)★: 둘이 따로
    ///   판정되면 "사용법엔 없는데 되는" 또는 "가르쳤는데 거절당하는" 상태가 조용히 생긴다.
    #[test]
    fn provision_records_the_same_mail_verdict_on_the_credential_and_the_marker() {
        let _g = lock_env();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());
        for accepts_mcp_config in [true, false] {
            let registry = Arc::new(ControlRegistry::new());
            let data_dir = std::env::temp_dir()
                .join(format!("engram-provision-adr0133-{}", AgentId::new_v4()));
            let channel = DaemonControlChannel::new(
                registry.clone(),
                "http://127.0.0.1:1/mcp".to_string(),
                data_dir.clone(),
                Some(PathBuf::from(CLI_EXE_NAME)),
                Arc::new(RecordingPriming {
                    seen: Arc::new(Mutex::new(None)),
                }),
            );
            let id = AgentId::new_v4();
            let ep = channel
                .provision(id, 0, accepts_mcp_config)
                .expect("provision ok")
                .expect("endpoint");
            let bound = registry.validate(&ep.token).expect("발급된 자격증명");
            assert_eq!(
                bound.mail_allowed, ep.mail_allowed,
                "자격증명에 박힌 판정 == endpoint 가 나르는 표식(accepts_mcp_config={accepts_mcp_config})"
            );
            assert_eq!(
                ep.mail_allowed, !accepts_mcp_config,
                "우편 가부는 MCP-capability 의 반대(accepts_mcp_config={accepts_mcp_config})"
            );
            let _ = std::fs::remove_dir_all(&data_dir);
        }
    }

    // ── ADR-0099 FIX 3: ENGRAM_FORCE_CLI_ONLY_SEND test-seam — 전체 false path 강제 ──────────────
    //
    // ★아래 seam 테스트들은 `test-harness` 갈래만 기술한다★: 운영 빌드엔 그 노브가 **존재하지 않는다**
    //   (`force_cli_only` 의 운영 갈래가 `const fn … { false }` — env 를 읽을 수단 자체가 없다). 그 갈래를
    //   테스트로 덮지 않는 이유는 이 crate 의 self-dev-dependency 가 테스트 빌드에서 feature 를 **항상**
    //   켜기 때문이다(Cargo.toml `[dev-dependencies]` 주석) — `#[cfg(not(feature = …))]` 테스트는 여기서
    //   실행되지 않을 뿐 아니라 **타입 검사조차 되지 않아** 조용히 썩는다(참조하는 헬퍼를 개명해도 안 깨진다).
    //   그 축은 컴파일러가 지킨다.

    #[cfg(feature = "test-harness")]
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

    #[cfg(feature = "test-harness")]
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

    /// ★인가 판정이 그 노브에 매달렸다는 것을 못박는다(ADR-0133)★: 노브가 채널만 옮기고 `mail_allowed` 는
    ///   안 옮기게 바뀌면 하네스가 "CLI 우편 실측" 이라 믿는 스폰이 실제론 거절당한다. 이 축이 있어야 위
    ///   운영-부재 pin 이 "왜 게이팅이 필요한가" 를 함께 진술한다.
    // ADR-0133
    #[cfg(feature = "test-harness")]
    #[test]
    fn the_forcing_knob_moves_the_mail_verdict_too_which_is_why_it_is_gated() {
        let _g = lock_env();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());
        let seen = Arc::new(Mutex::new(None));
        let (channel, data_dir) =
            provision_test_channel_with_send(seen.clone(), Some(PathBuf::from(CLI_EXE_NAME)));
        std::env::set_var(FORCE_CLI_ENV, "1");
        let id = AgentId::new_v4();
        let result = channel.provision(id, 0, true);
        std::env::remove_var(FORCE_CLI_ENV);
        let ep = result.expect("provision ok").expect("endpoint");
        assert!(
            ep.mail_allowed,
            "seam 켜짐 → 우편 판정까지 CLI 쪽으로 옮겨진다(하네스가 실측하려는 것이 그 경로다)"
        );
        let _ = std::fs::remove_dir_all(&data_dir);
    }
}
