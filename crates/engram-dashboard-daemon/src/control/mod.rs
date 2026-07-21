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
    AgentId, ControlChannel, ControlEndpoint, ProvisionError, ToolGrant,
};

use mcp_config::MCP_SERVER_NAME;
use mcp_server::SEND_MESSAGE_TOOL;
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

    /// ADR-0094: 발신 입구 pre-authorization grant 목록을 만든다(순수 — 단위 테스트 대상). 발신 입구
    /// **이름의 단일 출처**는 컨트롤 채널 정의다: MCP 서버명 = `MCP_SERVER_NAME`(mcp_config), 발신 툴 =
    /// `SEND_MESSAGE_TOOL`(mcp_server). CLI 는 send_exe(형제 바이너리 절대경로)가 있을 때만 grant 한다.
    ///
    /// ★최소권한(ADR-0094)★: 발신 입구만 담는다 — 나머지 툴은 backend 가 아무 것도 안 주입해 게이트 유지.
    /// ★send_exe 부재★: None(부분 빌드 등)이면 CLI grant 를 생략한다 — MCP grant 만으로 발신 입구가 열린다
    ///   (ENGRAM_SEND_EXE env 미주입과 대칭 — CLI 입구 자체가 없으니 그 권한도 없다).
    ///
    /// ★ADR-0094 CLI-only 측정 test-seam(`ENGRAM_DISALLOW_MCP_SEND`)★: ingress.rs 의 `ENGRAM_WRAP_FORMAT`
    ///   스파이크-seam 선례와 동일한 **env 게이트·하네스/운영자 통제·test-only** 노브다(운영 스위치 아님).
    ///   env 가 설정되고 **비어있지 않으면** MCP send_message grant 를 **뺀다** → 에이전트는 CLI(engram-send)
    ///   로만 발신할 수 있어 순수 CLI-only 라우팅을 실측할 수 있다. env 미설정/빈 값이면 오늘과 **바이트 동일**
    ///   (MCP grant 존재) — 운영 회귀 0. ★최소권한 불변식★: 이 seam 은 grant 를 **오직 제거만** 한다(절대
    ///   확장 X) — env 를 켠 하네스라도 오늘보다 더 넓은 권한을 얻지 못한다. env 게이트라 운영 호출자는 무영향.
    fn build_grants(send_exe: Option<&std::path::Path>) -> Vec<ToolGrant> {
        // ADR-0094 test-seam: MCP 발신 입구를 뺄지(CLI-only 측정). env 미설정/빈 값 = 오늘 동작(포함).
        let disallow_mcp_send = std::env::var("ENGRAM_DISALLOW_MCP_SEND")
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        // MCP 발신 입구는 기본 항상 grant(제어 채널이 붙는 한 send_message 는 존재). CLI-only 측정 seam 이
        //   켜졌을 때만 이 grant 를 생략한다(최소권한: 제거 방향 — 절대 확장 X).
        let mut grants = Vec::new();
        if !disallow_mcp_send {
            grants.push(ToolGrant::Mcp {
                server: MCP_SERVER_NAME.to_string(),
                tool: SEND_MESSAGE_TOOL.to_string(),
            });
        }
        // CLI 발신 입구는 형제 바이너리가 있을 때만. exe 는 backend 가 그대로 `Bash(<exe> *)` 로 쓴다 —
        //   ★caveat(ADR-0094)★: 여기서 넘기는 값은 endpoint.send_exe 와 동일한 **절대경로 원문**이다.
        //   에이전트가 실제로 부르는 명령($ENGRAM_SEND_EXE = 이 절대경로)의 prefix 와 문자열로 일치해야
        //   claude Bash 권한이 매칭된다(backend/claude.rs 의 패턴 번역 주석 참조). 이름 재타이핑 금지.
        if let Some(exe) = send_exe {
            grants.push(ToolGrant::Cli {
                exe: exe.to_string_lossy().into_owned(),
            });
        }
        grants
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
        // ADR-0094: 발신 입구 pre-authorization grant 를 **여기서**(입구 정의 옆) 채운다 — 이름의 정본은
        //   컨트롤 채널이다. backend(claude.rs)는 이 목록을 자기 문법(--allowedTools mcp__{s}__{t} /
        //   Bash({e} *))으로 번역만 한다(형식 규칙만 앎 — ADR-0004 격리 + ADR-0094 단일 출처).
        //   ★최소권한★: 발신 입구 2개만 담는다(MCP send_message + engram-send CLI). 나머지 툴은 게이트 유지.
        let grants = Self::build_grants(self.send_exe.as_deref());
        Ok(Some(ControlEndpoint {
            url: self.mcp_url.clone(),
            token,
            config_path,
            // F1: 형제 CLI 경로를 endpoint 로 실어 backend 가 ENGRAM_SEND_EXE 로 주입하게 한다(부팅 때 1회 탐색).
            send_exe: self.send_exe.clone(),
            // ADR-0092: 프라이밍 MD 절대경로(backend 가 --append-system-prompt-file 로 주입).
            priming_file,
            // ADR-0094: 발신 입구 pre-authorization(위 build_grants).
            grants,
        }))
    }

    /// (AgentId, epoch) 토큰 폐기 + mcp-config 파일 삭제. reaper(terminal 단일 소비자)·kill_agent 선제
    /// 에서 불린다. registry.revoke 가 epoch-guard·idempotent 를 담당하고, config 삭제도 idempotent.
    fn revoke(&self, id: AgentId, epoch: u32) {
        self.registry.revoke(id, epoch);
        mcp_config::remove_config(&self.data_dir, id, epoch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// ★ENV_LOCK(ingress.rs ENV_LOCK 선례)★: `build_grants` 는 `ENGRAM_DISALLOW_MCP_SEND`(프로세스 전역
    /// env)를 읽으므로, set/remove·미설정 단언 테스트끼리 직렬화한다 — 병렬 실행 시 한 테스트의 set 이
    /// 다른 테스트의 "미설정(= MCP grant 존재)" 단언을 짓밟지 않게. 각 env-touching 테스트는 진입 시
    /// leak 없음을 단언하고, 끝에서 반드시 remove 한다.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    const DISALLOW_MCP_ENV: &str = "ENGRAM_DISALLOW_MCP_SEND";

    // ── ADR-0094: build_grants — 발신 입구 pre-authorization grant 산출(단일 출처·최소권한) ──────

    #[test]
    fn build_grants_mcp_always_present_with_channel_names() {
        // send_exe 유무와 무관하게 MCP 발신 입구(send_message)는 (기본 env 하에) 항상 grant 된다. server/
        //   tool 이름은 컨트롤 채널 const(단일 출처)에서 온다 — 리터럴 재타이핑 없이 그 const 로 단언한다.
        // ENV_LOCK: build_grants 는 ENGRAM_DISALLOW_MCP_SEND 를 읽으므로 seam 테스트와 경쟁 — 직렬화.
        let _g = ENV_LOCK.lock().unwrap();
        assert!(
            std::env::var(DISALLOW_MCP_ENV).is_err(),
            "테스트 진입 시 env 미설정이어야(leak 감지)"
        );
        let grants = DaemonControlChannel::build_grants(None);
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
    fn build_grants_includes_cli_when_send_exe_present() {
        // send_exe 가 있으면 CLI 발신 입구도 grant 에 추가된다 — exe 는 절대경로 원문 그대로.
        // ENV_LOCK: 기본 env(MCP grant 포함) 가정 — seam 테스트의 set_var 와 경쟁하지 않게 직렬화.
        let _g = ENV_LOCK.lock().unwrap();
        assert!(std::env::var(DISALLOW_MCP_ENV).is_err());
        let exe = Path::new("C:/app/engram-send.exe");
        let grants = DaemonControlChannel::build_grants(Some(exe));
        assert_eq!(
            grants,
            vec![
                ToolGrant::Mcp {
                    server: MCP_SERVER_NAME.to_string(),
                    tool: SEND_MESSAGE_TOOL.to_string(),
                },
                ToolGrant::Cli {
                    exe: "C:/app/engram-send.exe".to_string(),
                },
            ],
            "send_exe 있으면 MCP + CLI 두 grant"
        );
    }

    #[test]
    fn build_grants_is_minimal_privilege() {
        // ★최소권한 회귀 가드★: 발신 입구 외 다른 툴(Read/Write/Edit/Bash 일반 등)이 grant 에 절대
        //   섞이지 않는다 — 최대 2개(MCP + CLI)이고 둘 다 발신 입구다.
        // ENV_LOCK: 기본 env 가정 — seam 테스트와 경쟁하지 않게 직렬화.
        let _g = ENV_LOCK.lock().unwrap();
        assert!(std::env::var(DISALLOW_MCP_ENV).is_err());
        let grants = DaemonControlChannel::build_grants(Some(Path::new("C:/app/engram-send.exe")));
        assert!(grants.len() <= 2, "발신 입구만 — 최대 2개: {grants:?}");
        for g in &grants {
            match g {
                ToolGrant::Mcp { tool, .. } => assert_eq!(tool, SEND_MESSAGE_TOOL),
                ToolGrant::Cli { .. } => {} // CLI 는 send_exe 로만 파생(발신 전용)
            }
        }
    }

    // ── ADR-0094 test-seam: ENGRAM_DISALLOW_MCP_SEND — CLI-only 측정용 MCP grant 제거 ──────────────

    #[test]
    fn build_grants_disallow_mcp_env_removes_mcp_but_keeps_cli() {
        // ★핵심 seam 회귀(ADR-0094)★: env 가 켜지면(non-empty) MCP send_message grant 는 빠지고, CLI grant
        //   (send_exe 있을 때)는 그대로 남는다 → 에이전트는 engram-send CLI 로만 발신(순수 CLI-only 측정).
        //   env 는 프로세스 전역이라 set→단언→remove 를 한 흐름에서 직렬로 하고 끝에서 반드시 제거한다.
        let _g = ENV_LOCK.lock().unwrap();
        assert!(
            std::env::var(DISALLOW_MCP_ENV).is_err(),
            "테스트 진입 시 env 미설정이어야(leak 감지)"
        );
        std::env::set_var(DISALLOW_MCP_ENV, "1");
        let exe = Path::new("C:/app/engram-send.exe");
        let grants = DaemonControlChannel::build_grants(Some(exe));
        std::env::remove_var(DISALLOW_MCP_ENV); // 반드시 제거(다른 테스트로 새지 않게).
        assert_eq!(
            grants,
            vec![ToolGrant::Cli {
                exe: "C:/app/engram-send.exe".to_string(),
            }],
            "env 켜짐 → MCP grant 제거, CLI grant 만 남아야(CLI-only)"
        );
    }

    #[test]
    fn build_grants_disallow_mcp_env_with_no_send_exe_yields_empty() {
        // ★최소권한(제거만)★: env 켜짐 + send_exe 부재면 발신 grant 가 하나도 없다 — seam 은 오직 제거만
        //   하지 절대 다른 권한을 추가하지 않는다(CLI 인프라가 없으면 그 grant 도 없음).
        let _g = ENV_LOCK.lock().unwrap();
        assert!(std::env::var(DISALLOW_MCP_ENV).is_err());
        std::env::set_var(DISALLOW_MCP_ENV, "1");
        let grants = DaemonControlChannel::build_grants(None);
        std::env::remove_var(DISALLOW_MCP_ENV);
        assert!(
            grants.is_empty(),
            "env 켜짐 + send_exe 부재 → 발신 grant 0(제거만, 추가 없음): {grants:?}"
        );
    }

    #[test]
    fn build_grants_disallow_mcp_empty_value_is_production_default() {
        // ★운영 회귀 0★: env 가 설정돼도 **빈 값**이면 seam 미발동 = 오늘과 바이트 동일(MCP grant 포함).
        //   ENGRAM_WRAP_FORMAT 선례와 동일한 non-empty 게이트(빈 값 = 미설정 취급).
        let _g = ENV_LOCK.lock().unwrap();
        assert!(std::env::var(DISALLOW_MCP_ENV).is_err());
        std::env::set_var(DISALLOW_MCP_ENV, "");
        let grants = DaemonControlChannel::build_grants(Some(Path::new("C:/app/engram-send.exe")));
        std::env::remove_var(DISALLOW_MCP_ENV);
        assert_eq!(
            grants,
            vec![
                ToolGrant::Mcp {
                    server: MCP_SERVER_NAME.to_string(),
                    tool: SEND_MESSAGE_TOOL.to_string(),
                },
                ToolGrant::Cli {
                    exe: "C:/app/engram-send.exe".to_string(),
                },
            ],
            "빈 값 = seam 미발동 → 오늘과 동일(MCP + CLI)"
        );
    }
}
