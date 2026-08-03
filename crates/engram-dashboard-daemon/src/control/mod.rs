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
    /// provision 이 이 값을 그대로 ControlEndpoint.send_exe 로 실어, backend 가 **CLI 전용 스폰에서만**
    /// ENGRAM_SEND_EXE·PATH 로 주입한다(ADR-0128 — MCP 가능 스폰은 이 배선을 받지 않는다).
    /// None(형제 부재)이면 CLI 입구 자체가 없다 → 비-MCP 스폰은 provision 이 fail-closed 로 끊는다.
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
    /// `SEND_MESSAGE_TOOL`(mcp_server).
    ///
    /// ★채널 하나만 방출(ADR-0128 등호 불변식)★: MCP-capable → `[Mcp]`, 비-MCP + send_exe → `[Cli]`.
    ///   grant 집합은 **물리 배선 집합과 같아야** 한다 — MCP 가능 스폰엔 `engram-send` 를 깔지 않으므로
    ///   그 CLI grant 도 방출하지 않는다(권한만 남기면 실체 없는 정책 표면이 된다).
    /// ★최소권한(ADR-0094)★: 발신 입구만 담는다 — 나머지 툴은 backend 가 아무 것도 안 주입해 게이트 유지.
    /// ★send_exe 부재★: None(부분 빌드 등)이면 CLI grant 를 생략한다 — CLI 입구 자체가 없으니 그 권한도
    ///   없다(ENGRAM_SEND_EXE env 미주입과 대칭).
    ///
    /// ★ADR-0094 CLI-only 측정 test-seam(`ENGRAM_DISALLOW_MCP_SEND`)★: ingress.rs 의 `ENGRAM_WRAP_FORMAT`
    ///   스파이크-seam 선례와 동일한 **env 게이트·하네스/운영자 통제·test-only** 노브다(운영 스위치 아님).
    ///   env 가 설정되고 **비어있지 않으면** MCP send_message grant 를 **뺀다**. env 미설정/빈 값이면 오늘과
    ///   **바이트 동일**(MCP grant 존재) — 운영 회귀 0.
    ///   ★이 노브로는 CLI-only 라우팅을 만들 수 없다(ADR-0128 이후)★: MCP 가능 스폰엔 CLI 배선도 CLI grant 도
    ///   없으므로 MCP grant 를 빼면 **발신 grant 가 0** 이 된다. 게다가 스폰은 auto 권한 모드라 grant 자체가
    ///   NO-OP 이다(ADR-0097) — 실측(2026-08-03, 6/6)에서 이 노브를 켠 에이전트가 전부 정상 발신했다.
    ///   CLI 라우팅 실측은 물리를 가르는 `ENGRAM_FORCE_CLI_ONLY_SEND`(provision)로만 성립한다.
    // ADR-0128
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
        // CLI 발신 입구는 **비-MCP 스폰 + 형제 바이너리 존재**일 때만(ADR-0128 — MCP 가능 스폰엔 CLI 를
        //   깔지 않으니 그 권한도 없다). exe 값 자체는 ★bare 명령 이름(`engram-send`)★을 담는다 —
        //   절대경로가 아니다.
        //   ★불변식(ADR-0094)★: grant 는 bare 명령 이름을 실어 backend 가 `Bash(engram-send:*)`(prefix
        //     와일드카드)로 번역하고, 스폰된 에이전트는 bare `engram-send` 를 shell 에서 부른다(backend 가
        //     주입한 PATH 로 해석 — claude.rs 참조). 이 세 문자열(grant · CLI-only 프라이밍(B)이 가르치는
        //     명령 · 실제 invocation)이 모두 bare `engram-send` 로 정렬돼야 claude 권한 게이트를 통과한다.
        //   ★WHY bare 이름(절대경로 폐기)★: 옛 절대경로 grant(`Bash(<abs> *)`, space-star)는 라이브
        //     측정에서 0/38 로 전부 permission-blocked 됐고(패턴 미매칭), 절대 좌표를 grant 에 박아 배포
        //     비친화적(머신마다 경로가 다름)이었다. bare 이름 + 주입 PATH 로 배포 가능하게 정렬한다.
        //   ★단일 출처★: send_exe 는 CLI 입구 **존재 여부**(Some/None)만 판정에 쓴다 — grant 문자열은
        //     프라이밍이 가르치는 명령 이름과 반드시 일치해야 하므로 bare 이름을 여기서 정본으로 박는다.
        // ADR-0128
        if !accepts_mcp_config && send_exe.is_some() {
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
        //   불변식(가르치는 채널 ⊆ 깐 채널, ADR-0126 결정 4)이 by-construction 으로 보존된다.★ 이게 옛
        //   `ENGRAM_DISALLOW_MCP_SEND`(grant-only 노브)와 다른 점이다: 후자는 grant 에서 MCP 만 빼고
        //   **MCP 서버는 여전히 mcp-config 로 부착**돼 물리와 권한이 갈렸다(측정 전용 — 프롬프트-도구
        //   불일치를 일부러 만든다). ★ADR-0126 이후 그 노브는 기본 프라이밍과도 어긋난다★: A 는 이제
        //   send_message **만** 가르치므로, grant 만 빼면 에이전트는 유일하게 배운 입구가 막히고 CLI 는
        //   배운 적이 없는 상태가 된다 — CLI 라우팅을 보려면 CLI-only 프라이밍을 함께 줘야 한다.
        //   이 seam 은 반대로 **모든 채널을 CLI 로 정렬**해 실 claude 를 비-MCP 백엔드처럼 굴려 false path
        //   전체를 실측한다.
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
        //   ★정합 불변식(ADR-0128 로 등호 복원)★: 프라이밍이 **가르치는** 채널 집합 **=** 물리적으로
        //     provision 하는 채널 집합. 안 깐 채널을 가르치면 발신 freeze 재발(MCP 노출 + CLI-only 지시 =
        //     ~6/7 미발신 실측), 반대로 깔고도 안 가르치면 아무도 통제하지 못하는 우회 표면이 남는다
        //     (실측: 채널 통제는 권한이 아니라 물리 배선에만 걸린다). 그래서 아래 세 갈래
        //     (config_path / priming variant / grants)가 이 flag 하나로 함께 움직인다 — 따로 놀지 않게.
        //   - MCP-capable(claude=true): mcp-config 기록(파일 물리 존재) + MCP endpoint bits(url/token/config)
        //     + MCP-only 교육 프라이밍(send_message 만 — ADR-0126 결정 1) + [Mcp] grant. engram-send 배선은
        //     **없다**(backend 가 CLI 크레덴셜·PATH 를 이 갈래에 주입하지 않는다 — ADR-0128 결정 2).
        //   - 비-MCP(codex/gemini stub=false): mcp-config **미기록**(파일 물리 부재) + CLI-only 프라이밍
        //     (engram-send 만) + [Cli] grant. MCP 입구가 프롬프트에서 완전히 삭제돼 지시-도구 불일치 없음.
        // ADR-0099 / ADR-0126 / ADR-0128
        let (config_path, settings_file, priming_variant) = if accepts_mcp_config {
            // 순서: 파일 먼저 쓰고(경로 확정) → registry 등록. NEW config write 실패는 치명(FIX 5 §case 2)
            //   → Err 로 fail-closed. (오래된 파일 삭제 실패는 provision 을 막지 않는다 — 아래 boot sweep /
            //   revoke 가 warn 만; 그 잔여 파일은 토큰이 registry 에 없어 inert 다.)
            let path = mcp_config::write_config(&self.data_dir, id, epoch, &self.mcp_url, &token)
                .map_err(|e| {
                    tracing::warn!(agent = %id, epoch, "mcp-config 기록 실패 — fail-closed(스폰 중단): {e}");
                    ProvisionError(format!("mcp-config write failed: {e}"))
                })?;
            // S18 D(spec §6): 세션 한정 설정 조각도 **MCP-capable 일 때만** 쓴다 — 조각의 내용이
            //   `allowedMcpServers`(engram 서버 허용)뿐이라, MCP 채널을 안 까는 스폰엔 의미가 없다(허용할
            //   서버 자체가 없다). 그래서 config_path 와 **같은 갈래**에 묶어 정합 불변식(깐 채널 == 허용한
            //   채널)이 by-construction 으로 유지되게 한다.
            // ★write 실패는 치명이 아니다(mcp-config 와 다른 판단 — load-bearing)★: mcp-config 가 없으면
            //   MCP 채널이 **물리적으로 없어** 발신 입구가 사라지므로 fail-closed 가 맞다. 반면 이 조각은
            //   "유저 전역 차단을 뒤집는 보정" 이라, 없으면 **전역 설정이 허용일 때는 정상 동작**하고
            //   차단일 때만 툴이 안 보인다. 그 열화 때문에 스폰 자체를 막으면 회귀(오늘까지 이 파일 없이도
            //   스폰은 됐다)라, warn 만 남기고 조각 없이 진행한다(priming_file 의 graceful 과 같은 등급).
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
            // ADR-0099: MCP-capable → config_path = Some(경로) — backend(claude.rs)가 `--mcp-config` 로 주입.
            (Some(path), settings, PrimingVariant::McpPrimary)
        } else {
            // 비-MCP: mcp-config 를 **아예 쓰지 않는다**(물리 부재 = MCP 입구 삭제). config_path = None 으로
            //   부재를 **타입으로 인코딩**한다(옛 빈 PathBuf::new() sentinel 폐기) — backend(claude.rs)는
            //   `Some` 일 때만 `--mcp-config` 를 붙이므로, None 이면 그 플래그가 애초에 생성되지 않는다
            //   (빈-경로 방어 분기 불필요 — 타입이 강제). 이게 정합 불변식의 물리 절반(MCP 채널 없음)이다.
            //   설정 조각도 함께 생략한다(허용할 MCP 서버가 없다 — 위 갈래 주석).
            (None, None, PrimingVariant::CliOnly)
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
        //   engram-send 를 가르친다. 이는 정합 불변식(가르치는 채널 == 깐 채널 — ADR-0128)이 금지하는
        //   바로 그 방향의 위반이고, 발신 freeze(가르친 도구가 물리적으로 부재)를 낳는다. 그래서 조용히
        //   반쪽 스폰하지 않고 **loud fail-closed** 로 스폰을 중단한다(mod.rs ~L145 fail-closed 정신 —
        //   제어 채널 없이 몰래 도는 에이전트 금지).
        //   ★MCP-capable && send_exe=None 은 여기 안 걸린다★: 그건 MCP 입구가 물리적으로 살아 있어(A
        //     프라이밍이 가르치는 유일한 입구) 채널 0 이 아니다 — 그 갈래는 `engram-send` 를 애초에 깔지
        //     않으므로(ADR-0128 결정 2) 형제 바이너리 부재가 결손조차 아니다(경고 없이 정상 스폰).
        //   ★config 아직 미기록★: 이 분기는 !accepts_mcp_config 일 때만 참이라 위 write_config 를 타지
        //     않았다 → 여기서 Err 를 내도 회수할 config 파일이 없다(token 도 아직 issue 전 — leak 0).
        // ADR-0099
        if !accepts_mcp_config && self.send_exe.is_none() {
            let msg = "non-MCP backend with no engram-send binary — zero physical send channels while CLI-only priming teaches engram-send (pairing invariant violation)";
            tracing::warn!(agent = %id, epoch, "제어 채널 provision fail-closed(ADR-0099): {msg}");
            return Err(ProvisionError(msg.to_string()));
        }
        self.registry.issue(id, epoch, token.clone());
        // ADR-0092/0099: 프라이밍 파일 경로를 seam 으로 해석해 endpoint 에 싣는다(있으면). 변형은 위
        //   MCP-capability 가 고른다(McpPrimary=send_message 만 / CliOnly=engram-send 만 — ADR-0126 결정 1
        //   로 A 의 CLI 우회 교육이 폐지됐다). 부재/미구성이면 None — 프라이밍 provider 가 이미 warn 로그를
        //   남겼고, 스폰은 막지 않는다(graceful). 내용은 안 읽고 경로만 나른다(하드코딩 금지).
        let priming_file = self.priming.priming_file(priming_variant);
        // ADR-0094/0099: 발신 입구 pre-authorization grant 를 **여기서**(입구 정의 옆) 채널별로 채운다 —
        //   이름의 정본은 컨트롤 채널이다. backend(claude.rs)는 이 목록을 자기 문법(--allowedTools
        //   mcp__{s}__{t} / Bash({e}:*) + PowerShell({e}:*))으로 번역만 한다. MCP grant 는 MCP-capable
        //   백엔드에서만(비-MCP 는 그 입구가 물리적으로 없으므로 grant 도 없다: 정합 불변식). CLI grant 는
        //   그 거울로 **비-MCP + send_exe 존재**일 때만(ADR-0128 — MCP 갈래엔 CLI 배선이 없다).
        //   ★최소권한★: 발신 입구만 담는다 — 나머지 툴은 게이트 유지.
        // ADR-0099 / ADR-0128
        let grants = Self::build_grants(self.send_exe.as_deref(), accepts_mcp_config);
        Ok(Some(ControlEndpoint {
            url: self.mcp_url.clone(),
            token,
            config_path,
            // F1: 형제 CLI 경로를 endpoint 로 실어 backend 가 CLI 전용 스폰에서 ENGRAM_SEND_EXE·PATH 로
            //   주입하게 한다(부팅 때 1회 탐색).
            // ★두 갈래 공용 한 줄 — 소비는 CLI 갈래만(ADR-0128)★: MCP 가능 endpoint 에도 값이 실리지만
            //   backend 가 `config_path=Some` 갈래에서 이 필드를 쓰지 않는다. 여기서 갈래별로 지우지 않는
            //   이유는 배타성을 **한 곳**(backend 의 config_path 갈림)에만 두어 두 군데가 어긋날 여지를
            //   만들지 않기 때문이다 — 그 배타성을 지키는 가드가 아래 테스트
            //   (`provision_mcp_capable_does_not_wire_engram_send_into_spawn_env`)이고, CLI 갈래의 배선
            //   생존은 그 짝 테스트가 본다.
            // ADR-0128
            send_exe: self.send_exe.clone(),
            // ADR-0092/0099: 변형별 프라이밍 MD 절대경로(backend 가 --append-system-prompt-file 로 주입).
            priming_file,
            // ADR-0094/0099: 발신 입구 pre-authorization(위 build_grants — 채널별 방출).
            grants,
            // S18 D(spec §6): 세션 한정 설정 조각 경로(backend 가 --settings 로 주입). 비-MCP·write 실패면 None.
            settings_file,
        }))
    }

    /// (AgentId, epoch) 토큰 폐기 + mcp-config·설정 조각 파일 삭제. reaper(terminal 단일 소비자)·
    /// kill_agent 선제에서 불린다. registry.revoke 가 epoch-guard·idempotent 를 담당하고, 파일 삭제도
    /// 둘 다 idempotent(없으면 no-op) — 조각을 안 쓴 스폰(비-MCP)에서도 안전하다.
    fn revoke(&self, id: AgentId, epoch: u32) {
        self.registry.revoke(id, epoch);
        mcp_config::remove_config(&self.data_dir, id, epoch);
        // S18 D: 조각도 같은 수명(같은 폴더) — 함께 지운다(mcp_config::settings_path 주석).
        mcp_config::remove_settings(&self.data_dir, id, epoch);
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
    ///
    /// ★poison 은 복구한다 — 오염 자체는 정보가 아니다(`lock_env`, 실측 2026-08-04)★: 이 락을 든 테스트가
    ///   패닉하면 뒤따르는 모든 홀더의 `.lock()` 이 PoisonError 를 받는다. plain `.unwrap()` 이면 그 홀더들이
    ///   **락 줄에서** 패닉해, 진짜 실패 1건이 구별 불가능한 17건으로 불어나고 원래 assert 위치가 묻힌다
    ///   (실측: grant 회귀 1건이 17 failures 로 보고돼, 정작 그 회귀를 잡은 테스트가 락 줄을 가리켰다 —
    ///   isolation 실행으로만 진단 가능했다). 그래서 guard 를 꺼내 계속 쓴다: 직렬화는 그대로 유지되고
    ///   (뮤텍스는 여전히 배타적), 실패는 각자의 단언 위치에 남는다. 이 락은 값이 `()` 라 복구해도 볼 상태가
    ///   없다 — 오염된 데이터를 물려받는 위험이 없다. env 누수 감지는 poison 이 아니라 각 테스트 진입부의
    ///   leak 단언이 담당한다(그 장치가 이 역할의 정본).
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// ENV_LOCK 획득 — poison 을 복구해 실패가 자기 단언 위치에 남게 한다(위 static 주석의 근거).
    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }
    const DISALLOW_MCP_ENV: &str = "ENGRAM_DISALLOW_MCP_SEND";
    const FORCE_CLI_ENV: &str = "ENGRAM_FORCE_CLI_ONLY_SEND";

    // ── ADR-0094: build_grants — 발신 입구 pre-authorization grant 산출(단일 출처·최소권한) ──────

    #[test]
    fn build_grants_mcp_always_present_with_channel_names() {
        // send_exe 유무와 무관하게 MCP 발신 입구(send_message)는 (기본 env 하에) 항상 grant 된다. server/
        //   tool 이름은 컨트롤 채널 const(단일 출처)에서 온다 — 리터럴 재타이핑 없이 그 const 로 단언한다.
        // ENV_LOCK: build_grants 는 ENGRAM_DISALLOW_MCP_SEND 를 읽으므로 seam 테스트와 경쟁 — 직렬화.
        let _g = lock_env();
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
    fn build_grants_cli_grant_follows_the_channel_axis_not_send_exe_presence() {
        // ★ADR-0128★: send_exe 존재만으로 CLI grant 가 붙지 않는다 — 채널 축(accepts_mcp_config)이 함께
        //   결정한다. 같은 send_exe 로 두 갈래를 돌려 grant 가 **채널당 하나**임을 못박는다(예전엔 MCP
        //   가능 스폰이 CLI grant 를 함께 받았고, 그 배선이 없어진 지금 그 권한도 없어야 한다).
        //   CLI grant 의 exe 값은 bare 명령 이름 `engram-send`(절대경로 아님) — send_exe 는 존재 여부만
        //   판정에 쓰인다(ADR-0094 bare 정렬).
        // ENV_LOCK: 기본 env(MCP grant 포함) 가정 — seam 테스트의 set_var 와 경쟁하지 않게 직렬화.
        let _g = lock_env();
        assert!(std::env::var(DISALLOW_MCP_ENV).is_err());
        let exe = Path::new("C:/app/engram-send.exe");
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
                exe: "engram-send".to_string(),
            }],
            "비-MCP + send_exe → [Cli](CLI grant 는 이 축으로만 나온다)"
        );
    }

    #[test]
    fn build_grants_is_minimal_privilege() {
        // ★최소권한 회귀 가드★: 발신 입구 외 다른 툴(Read/Write/Edit/Bash 일반 등)이 grant 에 절대
        //   섞이지 않는다. ★상한은 채널당 1개(ADR-0128)★ — 우편 채널이 하나로 단일화돼 한 스폰이 두 발신
        //   입구를 동시에 갖지 않는다(옛 상한 2 는 더 이상 조이지 못하는 죽은 단언이었다). send_exe 를 켠
        //   상태로 두 축을 모두 돌려, 어느 축에서도 grant 가 늘지 않음을 본다.
        // ENV_LOCK: 기본 env 가정 — seam 테스트와 경쟁하지 않게 직렬화.
        // ADR-0128
        let _g = lock_env();
        assert!(std::env::var(DISALLOW_MCP_ENV).is_err());
        let exe = Path::new("C:/app/engram-send.exe");
        for accepts_mcp_config in [true, false] {
            let grants = DaemonControlChannel::build_grants(Some(exe), accepts_mcp_config);
            assert_eq!(
                grants.len(),
                1,
                "발신 입구 하나만(accepts_mcp_config={accepts_mcp_config}): {grants:?}"
            );
            match &grants[0] {
                ToolGrant::Mcp { tool, .. } => assert_eq!(tool, SEND_MESSAGE_TOOL),
                ToolGrant::Cli { exe } => assert_eq!(exe, "engram-send"),
            }
        }
    }

    // ── ADR-0094 test-seam: ENGRAM_DISALLOW_MCP_SEND — CLI-only 측정용 MCP grant 제거 ──────────────

    #[test]
    fn build_grants_disallow_mcp_env_on_mcp_capable_leaves_no_send_grant() {
        // ★seam 회귀 + 사정거리(ADR-0094 → ADR-0128)★: env 가 켜지면(non-empty) MCP send_message grant 가
        //   빠진다. MCP 가능 스폰엔 CLI 배선·CLI grant 가 없으므로(ADR-0128) 남는 발신 grant 는 **0** 이다
        //   — 이 노브는 CLI 라우팅을 만들지 못한다(그 측정은 물리를 가르는 FORCE_CLI_ENV 로만 성립).
        //   env 는 프로세스 전역이라 set→단언→remove 를 한 흐름에서 직렬로 하고 끝에서 반드시 제거한다.
        // ADR-0128
        let _g = lock_env();
        assert!(
            std::env::var(DISALLOW_MCP_ENV).is_err(),
            "테스트 진입 시 env 미설정이어야(leak 감지)"
        );
        std::env::set_var(DISALLOW_MCP_ENV, "1");
        let exe = Path::new("C:/app/engram-send.exe");
        let mcp_capable = DaemonControlChannel::build_grants(Some(exe), true);
        // 같은 env 아래 비-MCP 갈래는 무영향이어야(seam 은 MCP grant 만 제거 — 확장·부작용 없음).
        let non_mcp = DaemonControlChannel::build_grants(Some(exe), false);
        std::env::remove_var(DISALLOW_MCP_ENV); // 반드시 제거(다른 테스트로 새지 않게).
        assert!(
            mcp_capable.is_empty(),
            "env 켜짐 + MCP 가능 → 발신 grant 0: {mcp_capable:?}"
        );
        assert_eq!(
            non_mcp,
            vec![ToolGrant::Cli {
                exe: "engram-send".to_string(),
            }],
            "env 켜짐이어도 비-MCP 갈래의 CLI grant 는 그대로(seam 은 MCP grant 만 제거)"
        );
    }

    #[test]
    fn build_grants_disallow_mcp_env_with_no_send_exe_yields_empty() {
        // ★최소권한(제거만)★: env 켜짐 + send_exe 부재면 발신 grant 가 하나도 없다 — seam 은 오직 제거만
        //   하지 절대 다른 권한을 추가하지 않는다(CLI 인프라가 없으면 그 grant 도 없음).
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
        // ★운영 회귀 0★: env 가 설정돼도 **빈 값**이면 seam 미발동 = 오늘과 바이트 동일(MCP grant 포함).
        //   ENGRAM_WRAP_FORMAT 선례와 동일한 non-empty 게이트(빈 값 = 미설정 취급).
        let _g = lock_env();
        assert!(std::env::var(DISALLOW_MCP_ENV).is_err());
        std::env::set_var(DISALLOW_MCP_ENV, "");
        let grants =
            DaemonControlChannel::build_grants(Some(Path::new("C:/app/engram-send.exe")), true);
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
        // ★핵심(ADR-0099)★: accepts_mcp_config=false(비-MCP 백엔드)면 MCP send_message grant 는 방출되지
        //   않고 CLI grant(send_exe 있을 때)만 남는다 — 비-MCP 스폰은 mcp-config 를 안 깔아 그 입구가
        //   물리적으로 없다(정합 불변식). env seam 과 무관하게 이 물리 축이 MCP grant 를 지운다.
        let _g = lock_env();
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
        //   프라이밍 변형은 McpPrimary(A = send_message 만 가르친다, ADR-0126 결정 1).
        // ★단일 ENV_LOCK(ADR-0099)★: provision 은 FORCE·DISALLOW env 를 모두 읽으므로, 이 값을 세우지 않는
        //   테스트도 setter 테스트와 경합하지 않게 락을 잡는다(양쪽 env 모두 leak 없음을 단언).
        let _g = lock_env();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());
        let seen = Arc::new(Mutex::new(None));
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
        // S18 D(spec §6): MCP-capable → 세션 설정 조각도 함께 기록되고 endpoint 가 그 경로를 싣는다.
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
        // revoke 가 조각까지 idempotent 하게 지운다(수명 = mcp-config 와 동일).
        channel.revoke(id, 0);
        assert!(!settings.exists(), "revoke 시 설정 조각 삭제");
        assert!(!cfg.exists(), "revoke 시 mcp-config 삭제");
        channel.revoke(id, 0); // 이중 revoke 안전.
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn provision_non_mcp_skips_config_and_picks_cli_only_priming() {
        // ★핵심(ADR-0099)★: 비-MCP(false)는 mcp-config 파일을 **아예 쓰지 않고**(MCP 입구 물리 삭제)
        //   config_path 가 None(타입-인코딩 부재)이며, 프라이밍 변형은 CliOnly(B = engram-send 만).
        //   정합 불변식의 물리 절반. send_exe 를 켜야(CLI 입구 존재) fail-closed edge(FIX 2)에 안 걸린다.
        // ★단일 ENV_LOCK(ADR-0099)★: provision 이 두 env 를 읽으므로 setter 테스트와 직렬화(둘 다 leak 없음 단언).
        let _g = lock_env();
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
        // S18 D: 설정 조각도 같은 갈래라 함께 생략된다(허용할 MCP 서버가 없다).
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
        let _g = lock_env();
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

    /// ★backend 채널 갈림이 서는 토대(ADR-0128)★: backend(claude.rs)는 `config_path` **하나로** 우편 채널을
    /// 가른다 — `Some`=MCP 배선만, `None`=CLI 배선만. 그 신호가 등호로 성립하는 근거는 **여기의 fail-closed**
    /// 다: MCP-capable 스폰의 config write 가 실패하면 provision 이 통째로 Err 를 내 스폰이 없다. 그래서
    /// `config_path=None` 인 endpoint 는 비-MCP 스폰에서만 나온다.
    ///
    /// ★이 `?` 를 fail-open 으로 바꾸면 어떤 스위트도 안 깨진다(그래서 이 테스트가 있다)★: 그 순간
    ///   MCP-capable 스폰이 `config_path=None` 으로 나와 backend 가 그것을 CLI 전용으로 읽는다 → 프라이밍은
    ///   `send_message` 를 가르치고(교육 A) 배선은 CLI 이고 grants 는 `[Mcp]` 인 삼중 불일치 — ADR-0128 이
    ///   막으려는 바로 그 상태다.
    /// ★실패 주입 방식(결정적·이식성)★: mcp-config 폴더가 될 자리를 **파일**로 점유해 `create_dir_all` 이
    ///   결정적으로 실패하게 한다(권한 조작·경로 길이 트릭 없음 — OS 무관하게 "파일이 있는 자리에 디렉토리를
    ///   못 만든다"). 폴더명은 `mcp_config::config_path` 에서 파생해 하드코딩하지 않는다(rot 방지).
    // ADR-0128
    #[test]
    fn provision_mcp_capable_fails_closed_when_config_write_fails() {
        let _g = lock_env();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());
        let seen = Arc::new(Mutex::new(None));
        let (channel, data_dir) = provision_test_channel_with_send(
            seen.clone(),
            Some(PathBuf::from("C:/app/engram-send.exe")),
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
        // MCP-capable + send_exe=None: MCP 입구가 살아 있고(A 프라이밍이 가르치는 유일한 입구) 이 갈래는
        //   engram-send 를 애초에 깔지 않으므로(ADR-0128 결정 2) 형제 바이너리 부재가 결손조차 아니다 →
        //   경고 없이 정상 스폰. config_path 는 Some 이어야.
        // ★단일 ENV_LOCK(ADR-0099)★: provision 이 두 env 를 읽으므로 setter 테스트와 직렬화(둘 다 leak 없음 단언).
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

    // ── ADR-0128: 우편 채널 하드 단일화 — 배선 집합 = 교육 집합(등호, 두 갈래를 end-to-end 로) ────────

    /// 데몬이 준 endpoint 를 그대로 실 backend(ClaudeBackend)에 먹여 **스폰될 프로세스가 받을** env 를
    /// 얻는다. 계약은 "endpoint 에 실렸나" 가 아니라 "스폰 env 에 닿나" 라서, 손으로 만든 더미 endpoint 로
    /// 검사하면 provision 절반이 끊긴 것을 못 잡는다.
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

    /// ★MCP 가능 스폰에 CLI 배선이 새지 않는지 지키는 가드(ADR-0128 결정 2)★: 배선 축은 세 구간이고 이
    /// 테스트가 보는 건 ② provision → `ControlEndpoint.send_exe` · ③ backend → 스폰 env 다. ① 부팅 시
    /// 형제 exe 탐색(`lib.rs::locate_send_exe`)은 여전히 미커버 — 그쪽은 그 함수에 박은 앵커가 방어선이다.
    ///
    /// ★endpoint 가 경로를 나르는 상태로 단언하는 이유★: 데몬은 send_exe 를 두 갈래 공용 한 줄로 실으므로
    ///   MCP 가능 endpoint 에도 값이 있다. 즉 배타성을 만드는 건 backend 의 `config_path` 갈림뿐이고, 그
    ///   갈림이 무너지면 곧바로 크레덴셜·PATH 가 새어 이 결정이 깨진다 — 그러니 값이 **있는** 상태에서
    ///   부재를 단언해야 그 갈림을 실제로 시험한다(값을 빼고 단언하면 아무것도 검증하지 않는다).
    // ADR-0128
    #[test]
    fn provision_mcp_capable_does_not_wire_engram_send_into_spawn_env() {
        // ★단일 ENV_LOCK(ADR-0099)★: provision 이 두 env 를 모두 읽으므로 setter 테스트와 직렬화.
        let _g = lock_env();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());
        let seen = Arc::new(Mutex::new(None));
        // lib.rs `locate_send_exe` 가 돌려주는 것과 같은 **절대경로**로 모사한다 — 부모 디렉토리가 PATH
        //   prepend 대상이라 경로 형태가 load-bearing 이다(bare 이름이면 부모가 없어 PATH 주입이 안 돈다).
        let send_exe = PathBuf::from("C:/app/engram-send.exe");
        let (channel, data_dir) =
            provision_test_channel_with_send(seen.clone(), Some(send_exe.clone()));
        let id = AgentId::new_v4();
        let ep = channel
            .provision(id, 0, true)
            .expect("provision ok")
            .expect("endpoint");
        // 이 스폰이 정말 MCP 가능 갈래였는지 먼저 고정 — 비-MCP 로 새면 이 가드가 지키는 케이스가 아니다.
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
        // 프로필 PATH 를 실어 둔다 — 빈 env 면 PATH 항목이 0 개라 아래 무개입 단언이 공허해진다(기존 항목의
        //   앞에 형제 디렉토리를 끼워 넣는 회귀를 못 잡는다).
        let spec = spawn_spec_from(id, ep, vec![("PATH".to_string(), "C:\\custom".to_string())]);
        for key in ["ENGRAM_TOKEN", "ENGRAM_CONTROL_URL", "ENGRAM_SEND_EXE"] {
            assert!(
                !spec.env.iter().any(|(k, _)| k == key),
                "MCP 가능 스폰 env 에 {key} 가 실리면 ADR-0128 위반: {:?}",
                spec.env
            );
        }
        // ★단언 범위 = "우리가 안 얹었다"(사용자 결정)★: 프로필이 준 PATH 가 글자 그대로 남아야 한다 —
        //   형제 디렉토리를 prepend 하지도, 프로필 값을 깎지도 않는다.
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

    /// ★CLI 전용 백엔드의 배선이 살아 있는지 지키는 짝 가드(ADR-0128)★: 위 테스트만 있으면 "CLI 배선을
    /// 통째로 지워도 초록" 이 되므로 두 방향을 함께 못박는다 — 이 갈래가 조용히 열화하면 비-MCP 백엔드의
    /// 우편이 죽는다(그 백엔드엔 MCP 입구가 아예 없다).
    // ADR-0128
    #[test]
    fn provision_non_mcp_wires_engram_send_into_spawn_env() {
        let _g = lock_env();
        assert!(std::env::var(FORCE_CLI_ENV).is_err() && std::env::var(DISALLOW_MCP_ENV).is_err());
        let seen = Arc::new(Mutex::new(None));
        let send_exe = PathBuf::from("C:/app/engram-send.exe");
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
        // 프로필 PATH 없이 짓는다 — 이 갈래는 PATH 항목을 **새로 만드는** 쪽이라 그 생성 자체가 단언 대상
        //   이다(프로필 PATH 를 tail 로 보존하는 경로는 core 쪽 claude backend 테스트가 본다).
        let spec = spawn_spec_from(id, ep, vec![]);
        assert_eq!(
            spec.env
                .iter()
                .find(|(k, _)| k == "ENGRAM_SEND_EXE")
                .map(|(_, v)| v.as_str()),
            Some("C:/app/engram-send.exe"),
            "CLI 절대경로가 ENGRAM_SEND_EXE 로 스폰 env 에 실려야: {:?}",
            spec.env
        );
        assert!(
            spec.env.iter().any(|(k, _)| k == "ENGRAM_TOKEN")
                && spec.env.iter().any(|(k, _)| k == "ENGRAM_CONTROL_URL"),
            "CLI 입구가 붙을 크레덴셜도 함께 실려야: {:?}",
            spec.env
        );
        // PATH 맨 앞 = 형제 디렉토리 — 에이전트(와 그 자식 shell 도구)가 bare `engram-send` 를 해석하는
        //   경로다. 구분자·표기 차를 흡수하려 문자열 prefix 가 아니라 split_paths 분해로 단언한다.
        let path = spec
            .env
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("PATH"))
            .map(|(_, v)| v.as_str())
            .expect("CLI 전용 스폰 + send_exe → PATH 주입돼야");
        assert_eq!(
            std::env::split_paths(path).next(),
            Some(PathBuf::from("C:/app")),
            "PATH 맨 앞 = engram-send 형제 디렉토리: {path}"
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
        // ★핵심(FIX 3)★: seam env 가 켜지면 MCP-capable(true) 백엔드라도 provision 이 **비-MCP 로 강제**돼
        //   false path 전체가 돈다 — no config write(config_path=None) + CliOnly 프라이밍 + grants==[Cli].
        //   정합 불변식이 by-construction 으로 보존됨(한 effective flag 에서 세 갈래가 파생). env 는 프로세스
        //   전역이라 set→검증→remove 를 한 흐름에서 직렬화(단일 ENV_LOCK — provision 이 두 env 를 모두 읽어
        //   DISALLOW reader 와도 경합하므로 노브별 락이 아니라 하나로).
        let _g = lock_env();
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
        let _g = lock_env();
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
