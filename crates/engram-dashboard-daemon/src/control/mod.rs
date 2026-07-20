//! 제어 채널(ADR-0086 스텝 1) — 토큰 레지스트리 + MCP 입구 + mcp-config 생명주기.
//!
//! 구성:
//! - `registry` — (AgentId, epoch)별 bearer 토큰 발급·검증·폐기 + 세션 바인딩.
//! - `mcp_config` — 에이전트별 mcp-config JSON 생성·삭제(claude `--mcp-config` 대상).
//! - `ingress` — ControlIngress seam(스텝 2): 듀얼 입구(MCP+CLI) 공통 파이프라인(정규화→Validator→relay→ACK).
//! - `mcp_server` — 데몬 MCP Streamable HTTP 서버(auth 미들웨어 + `engram_ping`/`send_message` 툴 + `/control/send`).
//! - `DaemonControlChannel`(이 파일) — core `ControlChannel` seam 구현체. spawn=provision(토큰+config
//!   발급), terminal=revoke(폐기+config 삭제). core 는 이 구현을 모르고 trait 만 안다(ADR-0003 idiom).
//!
//! ★인과(ADR-0086 토큰 수명=(AgentId,epoch))★: provision 은 core spawn 경로(spec 조립 직전)에서, revoke
//!   는 reaper 단일 terminal 소비자 + kill_agent 선제에서 불린다 — 회전마다 새 토큰, 어떤 terminal 이든
//!   1회 폐기. 여기 DaemonControlChannel 은 그 seam 에 데몬 자원(registry·MCP url·data_dir)을 이어 붙인다.
//!
//! tauri import 0(daemon crate).

pub mod ingress;
pub mod mcp_config;
pub mod mcp_server;
pub mod priming;
pub mod registry;

use std::path::PathBuf;
use std::sync::Arc;

use engram_dashboard_core::agent::types::{
    AgentId, ControlChannel, ControlEndpoint, ProvisionError,
};

use priming::PrimingProvider;
use registry::ControlRegistry;

/// core `ControlChannel` seam 의 데몬 구현(ADR-0086). MCP 엔드포인트 URL·토큰 레지스트리·데이터
/// 디렉토리를 들고, provision/revoke 를 실제 자원에 잇는다.
pub struct DaemonControlChannel {
    /// 발급된 토큰의 검증 단일 출처(auth 미들웨어와 공유하는 동일 Arc).
    registry: Arc<ControlRegistry>,
    /// 데몬 MCP 서버 엔드포인트 URL(`http://127.0.0.1:<port>/mcp`). 모든 에이전트가 같은 URL 로 붙고,
    /// 신원은 토큰으로 구분한다(에이전트별 서버가 아니라 에이전트별 토큰).
    mcp_url: String,
    /// mcp-config 파일이 사는 데이터 디렉토리(파일은 <data_dir>/mcp-config/ 아래).
    data_dir: PathBuf,
    /// ADR-0086 스텝 2(F1): 데몬이 부팅 시 형제 exe 에서 찾아낸 `engram-send` CLI 절대경로(없으면 None).
    /// provision 이 이 값을 그대로 ControlEndpoint.send_exe 로 실어, backend(claude.rs)가 ENGRAM_SEND_EXE
    /// env 로 주입한다. None(형제 부재)이면 CLI 입구만 비활성 — MCP 입구는 정상.
    send_exe: Option<PathBuf>,
    /// ADR-0092(수신 계약): 스폰 시 시스템 프롬프트에 주입할 프라이밍 파일 경로를 산출하는 seam.
    /// provision 마다 `priming_file()` 을 물어 ControlEndpoint.priming_file 로 실어 보낸다(있으면).
    /// seam 이라 미래 에이전트별 인젝션 시스템으로 구현만 교체된다("길은 뚫어둠", ADR-0092).
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

    /// 256-bit(32B) 토큰을 OS CSPRNG 로 생성해 hex 64자로. lib.rs generate_token 과 동일 방식이나
    /// 그건 WS 클라이언트 토큰(daemon.json)용이라 관심사가 다르다 — 재사용/혼용 금지(ADR-0086 §맥락).
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
    /// (AgentId, epoch)용 토큰 발급 → mcp-config 파일 기록 → registry 등록 → ControlEndpoint 반환.
    ///
    /// ★fail-closed(FIX 2)★: 데몬은 제어 채널을 **쓰는** 구현이므로 CSPRNG/파일 write 실패는 정당한
    ///   부재가 아니라 **실패**다 → `Err(ProvisionError)`(Ok(None) 아님). 호출자(spawn_agent)가 이 Err
    ///   에서 fail-closed 로 스폰을 중단한다(제어 채널 없이 몰래 도는 에이전트 방지). DaemonControlChannel
    ///   은 항상 endpoint 를 주려 하므로 Ok(None) 을 절대 돌려주지 않는다(Ok(None)=Noop 전용).
    /// ★보안★: 토큰은 registry·파일에만 들어가고 로그엔 없다(발급 로그는 registry.issue 가 AgentId 만 찍음).
    fn provision(
        &self,
        id: AgentId,
        epoch: u32,
    ) -> Result<Option<ControlEndpoint>, ProvisionError> {
        let token = Self::gen_token()
            .ok_or_else(|| ProvisionError("CSPRNG token generation failed".to_string()))?;
        // 순서: 파일 먼저 쓰고(경로 확정) → registry 등록. NEW config write 실패는 치명(FIX 5 §case 2)
        //   → Err 로 fail-closed. (오래된 파일 삭제 실패는 provision 을 막지 않는다 — 아래 boot sweep /
        //   revoke 가 warn 만; 그 잔여 파일은 토큰이 registry 에 없어 inert 다.)
        let config_path = mcp_config::write_config(&self.data_dir, id, epoch, &self.mcp_url, &token)
            .map_err(|e| {
                tracing::warn!(agent = %id, epoch, "mcp-config 기록 실패 — fail-closed(스폰 중단): {e}");
                ProvisionError(format!("mcp-config write failed: {e}"))
            })?;
        self.registry.issue(id, epoch, token.clone());
        // ADR-0092: 프라이밍 파일 경로를 seam 으로 해석해 endpoint 에 싣는다(있으면). 부재/미구성이면
        //   None — 프라이밍 provider 가 이미 warn 로그를 남겼고, 스폰은 막지 않는다(graceful, 제어 채널
        //   provision 의 fail-closed 와 다른 정책). 내용은 안 읽고 경로만 나른다(하드코딩 금지).
        let priming_file = self.priming.priming_file();
        Ok(Some(ControlEndpoint {
            url: self.mcp_url.clone(),
            token,
            config_path,
            // F1: 형제 CLI 경로를 endpoint 로 실어 backend 가 ENGRAM_SEND_EXE 로 주입하게 한다(부팅 때 1회 탐색).
            send_exe: self.send_exe.clone(),
            // ADR-0092: 프라이밍 MD 절대경로(backend 가 --append-system-prompt-file 로 주입).
            priming_file,
        }))
    }

    /// (AgentId, epoch) 토큰 폐기 + mcp-config 파일 삭제. reaper(terminal 단일 소비자)·kill_agent 선제
    /// 에서 불린다. registry.revoke 가 epoch-guard·idempotent 를 담당하고, config 삭제도 idempotent.
    fn revoke(&self, id: AgentId, epoch: u32) {
        self.registry.revoke(id, epoch);
        mcp_config::remove_config(&self.data_dir, id, epoch);
    }
}
