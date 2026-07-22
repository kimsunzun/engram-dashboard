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
use priming::{PrimingProvider, PrimingVariant};
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
    fn build_grants(
        send_exe: Option<&std::path::Path>,
        accepts_mcp_config: bool,
    ) -> Vec<ToolGrant> {
        // ADR-0094 test-seam: MCP 발신 입구를 뺄지(CLI-only 측정). env 미설정/빈 값 = 오늘 동작(포함).
        //   ★이 seam 은 채널 스위치(ADR-0099)와 직교★ — env 는 MCP-capable 백엔드에서도 grant 를 **제거만**
        //     해 CLI-only 라우팅을 실측하는 노브이고, accepts_mcp_config 는 백엔드가 애초에 MCP 를 낄 수
        //     있는지의 물리 축이다. 둘 다 참일 때만 MCP grant 가 방출된다(둘 중 하나라도 거짓이면 제거).
        let disallow_mcp_send = std::env::var("ENGRAM_DISALLOW_MCP_SEND")
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        // ADR-0099: 채널별 grant 방출 — MCP 발신 입구(send_message)는 **MCP-capable 백엔드에서만** grant 한다
        //   (비-MCP 백엔드는 mcp-config 자체를 안 깔아 그 입구가 물리적으로 없다 → grant 도 없다: 정합
        //   불변식). MCP-capable 이라도 CLI-only 측정 seam 이 켜졌으면 제거한다(최소권한: 제거 방향 — 확장 X).
        // ADR-0099
        let mut grants = Vec::new();
        if accepts_mcp_config && !disallow_mcp_send {
            grants.push(ToolGrant::Mcp {
                server: MCP_SERVER_NAME.to_string(),
                tool: SEND_MESSAGE_TOOL.to_string(),
            });
        }
        // CLI 발신 입구는 형제 바이너리가 있을 때만(send_exe 존재 = CLI 입구가 배포됨). exe 값 자체는
        //   ★bare 명령 이름(`engram-send`)★을 담는다 — 절대경로가 아니다.
        //   ★불변식(ADR-0094)★: grant 는 bare 명령 이름을 실어 backend 가 `Bash(engram-send:*)`(prefix
        //     와일드카드)로 번역하고, 스폰된 에이전트는 bare `engram-send` 를 shell 에서 부른다(backend 가
        //     주입한 PATH 로 해석 — claude.rs 참조). 이 세 문자열(grant · 프라이밍이 가르치는 명령 · 실제
        //     invocation)이 모두 bare `engram-send` 로 정렬돼야 claude 권한 게이트를 통과한다.
        //   ★WHY bare 이름(절대경로 폐기)★: 옛 절대경로 grant(`Bash(<abs> *)`, space-star)는 라이브
        //     측정에서 0/38 로 전부 permission-blocked 됐고(패턴 미매칭), 절대 좌표를 grant 에 박아 배포
        //     비친화적(머신마다 경로가 다름)이었다. bare 이름 + 주입 PATH 로 배포 가능하게 정렬한다.
        //   ★단일 출처★: send_exe 는 CLI 입구 **존재 여부**(Some/None)만 판정에 쓴다 — grant 문자열은
        //     프라이밍이 가르치는 명령 이름과 반드시 일치해야 하므로 bare 이름을 여기서 정본으로 박는다.
        if send_exe.is_some() {
            grants.push(ToolGrant::Cli {
                exe: "engram-send".to_string(),
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
        accepts_mcp_config: bool,
    ) -> Result<Option<ControlEndpoint>, ProvisionError> {
        let token = Self::gen_token()
            .ok_or_else(|| ProvisionError("CSPRNG token generation failed".to_string()))?;
        // ADR-0099 test-seam: `ENGRAM_FORCE_CLI_ONLY_SEND` — 스폰을 **비-MCP 로 강제**해 false 분기 전체를
        //   돌린다(no config write + CliOnly 프라이밍 + [Cli]-only grant). ★이 분기 맨 위에서 flag 를
        //   덮어써 채널 물리 배선·프라이밍·grant 가 **한 소스**(effective flag)에서 파생되게 한다 — 정합
        //   불변식(깐 채널 == 프라이밍이 가르치는 채널)이 by-construction 으로 보존된다.★ 이게 옛
        //   `ENGRAM_DISALLOW_MCP_SEND`(grant-only 노브)와 다른 점이다: 후자는 grant 에서 MCP 만 빼고
        //   **MCP 서버는 여전히 mcp-config 로 부착**돼(프라이밍도 both-teaching) 물리/교육 채널이 갈렸다
        //   (측정 전용 — 프롬프트-도구 불일치를 일부러 만든다). 이 seam 은 반대로 **모든 채널을 CLI 로 정렬**
        //   해 실 claude 를 비-MCP 백엔드처럼 굴려 false path 전체를 실측한다.
        //   ★env 게이트(ENGRAM_DISALLOW_MCP_SEND·ENGRAM_WRAP_FORMAT 선례와 동형)★: 설정 + non-empty 일
        //     때만 발동. 미설정/빈 값이면 오늘과 바이트 동일(운영 회귀 0) — env 게이트라 운영 호출자는 무영향.
        //     하네스/운영자 통제·test-only 노브다(운영 스위치 아님).
        //   ★이 seam 을 ENGRAM_PRIMING_FILE override 와 손으로 조합 금지★: override→MCP-teaching 파일 +
        //     force→CLI-only 물리 = 정합 불변식 정면 위반(둘을 함께 쓰면 tooling 이 막던 pairing 위반이 부활).
        // ADR-0099
        let force_cli_only = std::env::var("ENGRAM_FORCE_CLI_ONLY_SEND")
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        let accepts_mcp_config = accepts_mcp_config && !force_cli_only;
        // ADR-0099: 백엔드 MCP-capability 하나가 채널 물리 배선·프라이밍 변형·grant 를 전부 가른다.
        //   ★정합 불변식★: 물리적으로 provision 하는 채널 집합 == 프라이밍이 가르치는 채널 집합. 어기면
        //     발신 freeze 재발(MCP 노출 + CLI-only 지시 = ~6/7 미발신 실측). 그래서 아래 세 갈래
        //     (config_path / priming variant / grants)가 이 flag 하나로 함께 움직인다 — 따로 놀지 않게.
        //   - MCP-capable(claude=true): mcp-config 기록(파일 물리 존재) + MCP endpoint bits(url/token/config)
        //     + both-teaching 프라이밍(send_message + engram-send) + [Mcp, Cli] grant.
        //   - 비-MCP(codex/gemini stub=false): mcp-config **미기록**(파일 물리 부재) + CLI-only 프라이밍
        //     (engram-send 만) + [Cli] grant. MCP 입구가 프롬프트에서 완전히 삭제돼 지시-도구 불일치 없음.
        // ADR-0099
        let (config_path, priming_variant) = if accepts_mcp_config {
            // 순서: 파일 먼저 쓰고(경로 확정) → registry 등록. NEW config write 실패는 치명(FIX 5 §case 2)
            //   → Err 로 fail-closed. (오래된 파일 삭제 실패는 provision 을 막지 않는다 — 아래 boot sweep /
            //   revoke 가 warn 만; 그 잔여 파일은 토큰이 registry 에 없어 inert 다.)
            let path = mcp_config::write_config(&self.data_dir, id, epoch, &self.mcp_url, &token)
                .map_err(|e| {
                    tracing::warn!(agent = %id, epoch, "mcp-config 기록 실패 — fail-closed(스폰 중단): {e}");
                    ProvisionError(format!("mcp-config write failed: {e}"))
                })?;
            // ADR-0099: MCP-capable → config_path = Some(경로) — backend(claude.rs)가 `--mcp-config` 로 주입.
            (Some(path), PrimingVariant::McpPrimary)
        } else {
            // 비-MCP: mcp-config 를 **아예 쓰지 않는다**(물리 부재 = MCP 입구 삭제). config_path = None 으로
            //   부재를 **타입으로 인코딩**한다(옛 빈 PathBuf::new() sentinel 폐기) — backend(claude.rs)는
            //   `Some` 일 때만 `--mcp-config` 를 붙이므로, None 이면 그 플래그가 애초에 생성되지 않는다
            //   (빈-경로 방어 분기 불필요 — 타입이 강제). 이게 정합 불변식의 물리 절반(MCP 채널 없음)이다.
            (None, PrimingVariant::CliOnly)
        };
        // ADR-0099: provision fork 관측성(정합 불변식은 필드로 볼 값어치가 있다 — logging-conventions §계측
        //   의무 "외부 경계·동시성 전이"). token 은 절대 로깅하지 않는다(§보안). effective flag(seam 반영
        //   후)·chosen variant·mcp-config 존재 여부를 field 로 뺀다(메시지 보간 금지).
        tracing::debug!(
            agent = %id,
            epoch,
            accepts_mcp_config,
            force_cli_only,
            ?priming_variant,
            has_mcp_config = config_path.is_some(),
            "제어 채널 provision fork(ADR-0099 채널 스위치)"
        );
        // ADR-0099 fail-closed edge(FIX 2): 비-MCP(effective) 스폰인데 CLI 입구(send_exe)마저 없으면
        //   물리 채널이 **하나도 없다**(MCP 미부착 + CLI 미배포) — 그런데 CLI-only 프라이밍(B)은
        //   engram-send 를 가르친다. 이는 정합 불변식(가르친 채널 == 깐 채널)의 정면 위반이고, 발신 freeze
        //   (가르친 도구가 물리적으로 부재)를 낳는다. 그래서 조용히 반쪽 스폰하지 않고 **loud fail-closed**
        //   로 스폰을 중단한다(mod.rs ~L145 fail-closed 정신 — 제어 채널 없이 몰래 도는 에이전트 금지).
        //   ★MCP-capable && send_exe=None 은 여기 안 걸린다★: 그건 MCP 입구가 물리적으로 살아 있어(both
        //     프라이밍의 주력 경로) 채널 0 이 아니다 — 아래 accepted-edge 로 별도 처리(warn 만).
        //   ★config 아직 미기록★: 이 분기는 !accepts_mcp_config 일 때만 참이라 위 write_config 를 타지
        //     않았다 → 여기서 Err 를 내도 회수할 config 파일이 없다(token 도 아직 issue 전 — leak 0).
        // ADR-0099
        if !accepts_mcp_config && self.send_exe.is_none() {
            let msg = "non-MCP backend with no engram-send binary — zero physical send channels while CLI-only priming teaches engram-send (pairing invariant violation)";
            tracing::warn!(agent = %id, epoch, "제어 채널 provision fail-closed(ADR-0099): {msg}");
            return Err(ProvisionError(msg.to_string()));
        }
        // ADR-0099 accepted-edge(FIX 2): MCP-capable + send_exe=None. MCP 입구는 살아 있으므로(both
        //   프라이밍의 주력) 채널 0 이 아니라 스폰을 허용한다. 다만 both-teaching 의 **폴백 문단**은
        //   engram-send 를 가리키는데 그 바이너리가 없어, 에이전트가 폴백을 시도하면 그 명령이 **가시적으로
        //   실패**한다(조용한 오작동 아님 — 에러가 눈에 보인다). 주력(MCP)이 정상이라 이 폴백 부재는
        //   기능적으로 치명이 아니다 → 허용하되 관측 가능하게 warn 만 남긴다(가시적-실패 엣지로 문서화).
        if accepts_mcp_config && self.send_exe.is_none() {
            tracing::warn!(
                agent = %id,
                epoch,
                "MCP-capable 스폰이나 engram-send 부재 — both-teaching 폴백(CLI)은 가시적으로 실패할 수 있음(주력 MCP 는 정상, accepted edge)"
            );
        }
        self.registry.issue(id, epoch, token.clone());
        // ADR-0092/0099: 프라이밍 파일 경로를 seam 으로 해석해 endpoint 에 싣는다(있으면). 변형은 위
        //   MCP-capability 가 고른다(McpPrimary=both-teaching / CliOnly=engram-send 만). 부재/미구성이면
        //   None — 프라이밍 provider 가 이미 warn 로그를 남겼고, 스폰은 막지 않는다(graceful). 내용은 안
        //   읽고 경로만 나른다(하드코딩 금지).
        let priming_file = self.priming.priming_file(priming_variant);
        // ADR-0094/0099: 발신 입구 pre-authorization grant 를 **여기서**(입구 정의 옆) 채널별로 채운다 —
        //   이름의 정본은 컨트롤 채널이다. backend(claude.rs)는 이 목록을 자기 문법(--allowedTools
        //   mcp__{s}__{t} / Bash({e}:*) + PowerShell({e}:*))으로 번역만 한다. MCP grant 는 MCP-capable
        //   백엔드에서만(비-MCP 는 그 입구가 물리적으로 없으므로 grant 도 없다: 정합 불변식). CLI grant 는
        //   send_exe 존재 시. ★최소권한★: 발신 입구만 담는다 — 나머지 툴은 게이트 유지.
        // ADR-0099
        let grants = Self::build_grants(self.send_exe.as_deref(), accepts_mcp_config);
        Ok(Some(ControlEndpoint {
            url: self.mcp_url.clone(),
            token,
            config_path,
            // F1: 형제 CLI 경로를 endpoint 로 실어 backend 가 ENGRAM_SEND_EXE 로 주입하게 한다(부팅 때 1회 탐색).
            send_exe: self.send_exe.clone(),
            // ADR-0092/0099: 변형별 프라이밍 MD 절대경로(backend 가 --append-system-prompt-file 로 주입).
            priming_file,
            // ADR-0094/0099: 발신 입구 pre-authorization(위 build_grants — 채널별 방출).
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

    /// ★단일 ENV_LOCK(ingress.rs ENV_LOCK 선례, ADR-0099)★: 이 모듈의 두 env 노브
    ///   (`ENGRAM_DISALLOW_MCP_SEND` — build_grants 가 읽음, `ENGRAM_FORCE_CLI_ONLY_SEND` — provision 이
    ///   읽음)는 **하나의** 락으로 직렬화한다. ★왜 노브별 락이 아니라 단일 락인가★: `provision` 은 **두 env 를
    ///   모두** 읽고 모든 provision 테스트가 (설정 안 해도) force env 를 읽는다 — 노브별 락이면 DISALLOW 만
    ///   잡은 reader 가 FORCE 를 세우는 setter 와, 혹은 그 반대로 경합해 플레이키하다(양쪽 knob 을 건드리는
    ///   provision 이 교차한다). 그래서 **어느 knob 이든 읽거나 쓰는 모든 테스트**가 이 하나를 잡는다.
    ///   각 env-touching 테스트는 진입 시 그 env 의 leak 없음을 단언하고, 끝에서 반드시 remove 한다.
    ///   (poison 회복은 기존 테스트대로 plain `.unwrap()` — 다른 테스트 패닉으로 오염되면 그대로 드러낸다.)
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    const DISALLOW_MCP_ENV: &str = "ENGRAM_DISALLOW_MCP_SEND";
    const FORCE_CLI_ENV: &str = "ENGRAM_FORCE_CLI_ONLY_SEND";

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
        // ADR-0099: MCP-capable 백엔드(accepts_mcp_config=true) 기준.
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
    fn build_grants_includes_cli_when_send_exe_present() {
        // send_exe 가 있으면(= CLI 입구 배포됨) CLI 발신 입구도 grant 에 추가된다 — exe 값은 bare 명령
        //   이름 `engram-send`(절대경로 아님). send_exe 는 존재 여부만 판정에 쓰인다(ADR-0094 bare 정렬).
        // ENV_LOCK: 기본 env(MCP grant 포함) 가정 — seam 테스트의 set_var 와 경쟁하지 않게 직렬화.
        let _g = ENV_LOCK.lock().unwrap();
        assert!(std::env::var(DISALLOW_MCP_ENV).is_err());
        let exe = Path::new("C:/app/engram-send.exe");
        // ADR-0099: MCP-capable 백엔드(accepts_mcp_config=true).
        let grants = DaemonControlChannel::build_grants(Some(exe), true);
        assert_eq!(
            grants,
            vec![
                ToolGrant::Mcp {
                    server: MCP_SERVER_NAME.to_string(),
                    tool: SEND_MESSAGE_TOOL.to_string(),
                },
                ToolGrant::Cli {
                    exe: "engram-send".to_string(),
                },
            ],
            "send_exe 있으면 MCP + CLI 두 grant(CLI 는 bare 이름 engram-send)"
        );
    }

    #[test]
    fn build_grants_is_minimal_privilege() {
        // ★최소권한 회귀 가드★: 발신 입구 외 다른 툴(Read/Write/Edit/Bash 일반 등)이 grant 에 절대
        //   섞이지 않는다 — 최대 2개(MCP + CLI)이고 둘 다 발신 입구다.
        // ENV_LOCK: 기본 env 가정 — seam 테스트와 경쟁하지 않게 직렬화.
        let _g = ENV_LOCK.lock().unwrap();
        assert!(std::env::var(DISALLOW_MCP_ENV).is_err());
        let grants =
            DaemonControlChannel::build_grants(Some(Path::new("C:/app/engram-send.exe")), true);
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
        // ADR-0099: MCP-capable 백엔드라도 env seam 이 MCP grant 를 제거(직교 축).
        let grants = DaemonControlChannel::build_grants(Some(exe), true);
        std::env::remove_var(DISALLOW_MCP_ENV); // 반드시 제거(다른 테스트로 새지 않게).
        assert_eq!(
            grants,
            vec![ToolGrant::Cli {
                exe: "engram-send".to_string(),
            }],
            "env 켜짐 → MCP grant 제거, CLI grant(bare engram-send)만 남아야(CLI-only)"
        );
    }

    #[test]
    fn build_grants_disallow_mcp_env_with_no_send_exe_yields_empty() {
        // ★최소권한(제거만)★: env 켜짐 + send_exe 부재면 발신 grant 가 하나도 없다 — seam 은 오직 제거만
        //   하지 절대 다른 권한을 추가하지 않는다(CLI 인프라가 없으면 그 grant 도 없음).
        let _g = ENV_LOCK.lock().unwrap();
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
        // ★운영 회귀 0★: env 가 설정돼도 **빈 값**이면 seam 미발동 = 오늘과 바이트 동일(MCP grant 포함).
        //   ENGRAM_WRAP_FORMAT 선례와 동일한 non-empty 게이트(빈 값 = 미설정 취급).
        let _g = ENV_LOCK.lock().unwrap();
        assert!(std::env::var(DISALLOW_MCP_ENV).is_err());
        std::env::set_var(DISALLOW_MCP_ENV, "");
        let grants =
            DaemonControlChannel::build_grants(Some(Path::new("C:/app/engram-send.exe")), true);
        std::env::remove_var(DISALLOW_MCP_ENV);
        assert_eq!(
            grants,
            vec![
                ToolGrant::Mcp {
                    server: MCP_SERVER_NAME.to_string(),
                    tool: SEND_MESSAGE_TOOL.to_string(),
                },
                ToolGrant::Cli {
                    exe: "engram-send".to_string(),
                },
            ],
            "빈 값 = seam 미발동 → 오늘과 동일(MCP + CLI bare engram-send)"
        );
    }

    // ── ADR-0099: 채널별 grant 방출 — 비-MCP 백엔드는 MCP grant 를 방출하지 않는다 ──────────────────

    #[test]
    fn build_grants_non_mcp_backend_emits_cli_only() {
        // ★핵심(ADR-0099)★: accepts_mcp_config=false(비-MCP 백엔드)면 MCP send_message grant 는 방출되지
        //   않고 CLI grant(send_exe 있을 때)만 남는다 — 비-MCP 스폰은 mcp-config 를 안 깔아 그 입구가
        //   물리적으로 없다(정합 불변식). env seam 과 무관하게 이 물리 축이 MCP grant 를 지운다.
        let _g = ENV_LOCK.lock().unwrap();
        assert!(std::env::var(DISALLOW_MCP_ENV).is_err());
        let exe = Path::new("C:/app/engram-send.exe");
        let grants = DaemonControlChannel::build_grants(Some(exe), false);
        assert_eq!(
            grants,
            vec![ToolGrant::Cli {
                exe: "engram-send".to_string(),
            }],
            "비-MCP 백엔드 → CLI grant 만(MCP 입구 물리 부재)"
        );
    }

    #[test]
    fn build_grants_non_mcp_backend_no_send_exe_yields_empty() {
        // 비-MCP + send_exe 부재 → 발신 grant 0(MCP 입구 없음 + CLI 인프라 없음).
        let _g = ENV_LOCK.lock().unwrap();
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

    /// 요청받은 `PrimingVariant` 를 기록하는 테스트 provider — provision 이 어느 변형을 골랐는지 관측한다.
    ///   경로는 고정 sentinel 을 돌려줘 endpoint.priming_file 에 실렸는지도 볼 수 있게 한다.
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

    /// `send_exe` 주입 가능한 provision 테스트 채널 — 비-MCP 스폰의 fail-closed edge(FIX 2: send_exe=None
    ///   이면 채널 0)를 검증하려면 send_exe 를 켠/끈 채널이 둘 다 필요하다.
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
        // MCP-capable(true): mcp-config 파일이 실제로 쓰이고 endpoint.config_path 가 그 파일을 가리키며,
        //   프라이밍 변형은 McpPrimary(A = both-teaching).
        // ★단일 ENV_LOCK(ADR-0099)★: provision 은 FORCE·DISALLOW env 를 모두 읽으므로, 이 값을 세우지 않는
        //   테스트도 setter 테스트와 경합하지 않게 락을 잡는다(양쪽 env 모두 leak 없음을 단언).
        let _g = ENV_LOCK.lock().unwrap();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());
        let seen = Arc::new(Mutex::new(None));
        // send_exe 를 켜서 accepted-edge warn 경로(MCP-capable + send_exe=None)를 피한다(config Some 검증 목적).
        let (channel, data_dir) =
            provision_test_channel_with_send(seen.clone(), Some(PathBuf::from("engram-send")));
        let id = AgentId::new_v4();
        let ep = channel
            .provision(id, 0, true)
            .expect("provision ok")
            .expect("endpoint");
        // ADR-0099: config_path 는 Option — MCP-capable → Some(실파일).
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
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn provision_non_mcp_skips_config_and_picks_cli_only_priming() {
        // ★핵심(ADR-0099)★: 비-MCP(false)는 mcp-config 파일을 **아예 쓰지 않고**(MCP 입구 물리 삭제)
        //   config_path 가 None(타입-인코딩 부재)이며, 프라이밍 변형은 CliOnly(B = engram-send 만).
        //   정합 불변식의 물리 절반. send_exe 를 켜야(CLI 입구 존재) fail-closed edge(FIX 2)에 안 걸린다.
        // ★단일 ENV_LOCK(ADR-0099)★: provision 이 두 env 를 읽으므로 setter 테스트와 직렬화(둘 다 leak 없음 단언).
        let _g = ENV_LOCK.lock().unwrap();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());
        let seen = Arc::new(Mutex::new(None));
        let (channel, data_dir) =
            provision_test_channel_with_send(seen.clone(), Some(PathBuf::from("engram-send")));
        let id = AgentId::new_v4();
        let ep = channel
            .provision(id, 0, false)
            .expect("provision ok")
            .expect("endpoint");
        assert_eq!(
            ep.config_path, None,
            "비-MCP → mcp-config 미기록(config_path=None, 타입-인코딩 부재)"
        );
        // mcp-config 디렉토리/파일이 애초에 생기지 않아야(물리 부재).
        assert!(
            !data_dir.join("mcp-config").exists(),
            "비-MCP → mcp-config 파일이 물리적으로 없어야"
        );
        assert_eq!(
            *seen.lock().unwrap(),
            Some(PrimingVariant::CliOnly),
            "비-MCP → CliOnly 프라이밍 변형"
        );
        assert_eq!(ep.priming_file, Some(PathBuf::from("B-cli-only")));
        // 비-MCP + send_exe 있음 → grant 는 [Cli] 만(MCP 입구 물리 부재).
        assert_eq!(
            ep.grants,
            vec![ToolGrant::Cli {
                exe: "engram-send".to_string(),
            }],
            "비-MCP → grants == [Cli]"
        );
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    // ── ADR-0099 FIX 2: fail-closed edge — 비-MCP + send_exe=None = 채널 0 → ProvisionError ──────
    #[test]
    fn provision_non_mcp_with_no_send_exe_fails_closed() {
        // ★핵심(FIX 2)★: 비-MCP(effective) 스폰인데 CLI 입구(send_exe)도 없으면 물리 채널이 하나도 없다
        //   — CLI-only 프라이밍이 가르치는 engram-send 가 물리적으로 부재 = 정합 불변식 위반. provision 은
        //   loud fail-closed(Err)로 스폰을 막아야 한다(조용한 반쪽 스폰 금지). config 파일도 안 남는다.
        // ★단일 ENV_LOCK(ADR-0099)★: provision 이 두 env 를 읽으므로 setter 테스트와 직렬화(둘 다 leak 없음 단언).
        let _g = ENV_LOCK.lock().unwrap();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());
        let seen = Arc::new(Mutex::new(None));
        let (channel, data_dir) = provision_test_channel_with_send(seen.clone(), None);
        let id = AgentId::new_v4();
        let err = channel
            .provision(id, 0, false)
            .expect_err("비-MCP + send_exe=None → fail-closed Err");
        // 사유에 원인(non-MCP + engram-send 미해석)이 드러나야(디버깅 가능).
        assert!(
            err.0.contains("non-MCP") && err.0.contains("engram-send"),
            "ProvisionError 사유에 원인 명시: {}",
            err.0
        );
        // config 파일이 생기지 않았어야(None 분기라 write 미실행).
        assert!(
            !data_dir.join("mcp-config").exists(),
            "fail-closed edge → config 파일 미생성"
        );
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn provision_mcp_capable_with_no_send_exe_is_allowed_accepted_edge() {
        // MCP-capable + send_exe=None: MCP 입구가 살아 있어(both 프라이밍 주력) 채널 0 이 아니다 → 허용
        //   (accepted edge — 폴백 engram-send 는 가시적 실패, warn 만). config_path 는 Some 이어야.
        // ★단일 ENV_LOCK(ADR-0099)★: provision 이 두 env 를 읽으므로 setter 테스트와 직렬화(둘 다 leak 없음 단언).
        let _g = ENV_LOCK.lock().unwrap();
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
        // send_exe 부재 → CLI grant 없음, MCP grant 만.
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

    // ── ADR-0099 FIX 3: ENGRAM_FORCE_CLI_ONLY_SEND test-seam — 전체 false path 강제 ──────────────
    #[test]
    fn provision_force_cli_only_seam_runs_entire_false_path() {
        // ★핵심(FIX 3)★: seam env 가 켜지면 MCP-capable(true) 백엔드라도 provision 이 **비-MCP 로 강제**돼
        //   false path 전체가 돈다 — no config write(config_path=None) + CliOnly 프라이밍 + grants==[Cli].
        //   정합 불변식이 by-construction 으로 보존됨(한 effective flag 에서 세 갈래가 파생). env 는 프로세스
        //   전역이라 set→검증→remove 를 한 흐름에서 직렬화(단일 ENV_LOCK — provision 이 두 env 를 모두 읽어
        //   DISALLOW reader 와도 경합하므로 노브별 락이 아니라 하나로).
        let _g = ENV_LOCK.lock().unwrap();
        assert!(
            std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err(),
            "테스트 진입 시 두 env 모두 미설정이어야(leak 감지 — provision 이 둘 다 읽음)"
        );
        let seen = Arc::new(Mutex::new(None));
        // send_exe 를 켠다 — seam 은 스폰을 CLI-only 로 만들므로 CLI 입구가 있어야 fail-closed edge 를 피한다.
        let (channel, data_dir) =
            provision_test_channel_with_send(seen.clone(), Some(PathBuf::from("engram-send")));
        std::env::set_var(FORCE_CLI_ENV, "1");
        let id = AgentId::new_v4();
        // accepts_mcp_config=true 로 물어도(= 실 claude) seam 이 false 로 덮어쓴다.
        let result = channel.provision(id, 0, true);
        std::env::remove_var(FORCE_CLI_ENV); // 반드시 제거(다른 테스트로 새지 않게).
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
                exe: "engram-send".to_string(),
            }],
            "seam 켜짐 → grants == [Cli](권한 절반)"
        );
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn provision_force_cli_only_empty_value_is_inert() {
        // ★운영 회귀 0★: seam env 가 **빈 값**이면 미발동 = 오늘 동작(MCP-capable → config Some).
        //   ENGRAM_DISALLOW_MCP_SEND/ENGRAM_WRAP_FORMAT 와 동일한 non-empty 게이트.
        let _g = ENV_LOCK.lock().unwrap();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());
        let seen = Arc::new(Mutex::new(None));
        let (channel, data_dir) =
            provision_test_channel_with_send(seen.clone(), Some(PathBuf::from("engram-send")));
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
