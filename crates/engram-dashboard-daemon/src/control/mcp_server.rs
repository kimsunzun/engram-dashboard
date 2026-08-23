//! 데몬 MCP Streamable HTTP 서버 — 스폰된 claude 에이전트가 mcp-config 로 붙는 제어 채널 입구.
//!
//! ★소유★: rmcp `StreamableHttpService`(Tower service)를 axum `/mcp` 에 nest 하고 그 앞에 bearer auth
//!   미들웨어를 얹는다. 같은 서버·포트·미들웨어에 CLI 평문 라우트를 나란히 태운다 — 그 **명단의 정본은
//!   `ControlRoute`** 이고 여기서 다시 세지 않는다(세던 목록이 라우트 추가 때 뒤처진다).
//!   툴 표면 = `engram_ping` · `send_message` · `messages`.
//!   ★제어 동사는 MCP 툴로 내지 않는다★(ADR-0132 결정 = CLI) — 툴 스키마는 모든 에이전트 컨텍스트에 상시
//!   상주하는데 제어는 빈도가 낮다. 그래서 `/control/agent` 에는 짝이 되는 `#[tool]` 이 없다.
//!
//! ★OAuth 메타데이터 미광고(load-bearing, #59467)★: StreamableHttpService 는 `.well-known/*` 라우트를
//!   만들지 않고, 우리도 추가하지 않는다. claude 는 서버가 OAuth 메타데이터를 광고하면 정적 Authorization
//!   헤더를 무시하는데(claude-code #59467), 광고 라우트가 없으니 정적 Bearer 가 그대로 실린다(ADR-0086 §근거).
//!
//! tauri import 0(daemon crate).

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::State;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use engram_dashboard_core::agent::manager::AgentManager;
use http::{Method, Request, StatusCode};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, ServerCapabilities, ServerInfo};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{schemars, tool, tool_handler, tool_router, ErrorData, RoleServer, ServerHandler};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use engram_dashboard_messaging::envelope::Entrance; // ADR-0110: 입구 라벨은 커널 crate 소유.

use super::ingress::{handle_messages, handle_send, ControlCommand, SendContract};
use super::registry::{BoundIdentity, ControlRegistry};
// ★로그 필드에 실을 호출자 문자열은 이 함수를 거친다 — 이웃 `agent::preview` 가 아니다★:
//   그 갈림의 정본은 [`super::agent::preview`] doc(「로그에 쓰지 말 것」)과 [`sanitize_for_log`] doc 이다.
use crate::connection_core::sanitize_for_log;

/// MCP 서버가 붙는 axum 경로. mcp-config url 도 이 경로를 가리킨다(`http://127.0.0.1:<port>/mcp`).
///
/// ★keep-in-sync(M5)★: `ControlEndpoint.url` 은 이 경로가 붙은 MCP 라우트다. claude backend 가 CLI 용
///   base URL(ENGRAM_CONTROL_URL)을 파생할 때 이 리터럴 suffix("/mcp")를 **문자열로 벗긴다** —
///   `crates/engram-dashboard-core/src/agent/backend/claude.rs`(strip_suffix("/mcp")). 이 값을 바꾸면
///   거기 strip 리터럴도 함께 고쳐야 한다 — 빌드가 강제하지 않아 어긋나면 base 파생이 틀어지고 CLI 가
///   조용히 404 를 받는다.
const MCP_PATH: &str = "/mcp";

/// CLI 입구(ADR-0086 스텝 2) — `engram mail send` 가 POST 하는 평문 JSON 라우트. CLI 가 base URL
/// (ENGRAM_CONTROL_URL)에 이 경로를 조립한다.
const CONTROL_SEND_PATH: &str = "/control/send";

/// CLI 조회 입구(D · 표면 정본 = ADR-0132) — `engram mail status <id>` / `engram mail pending` 이 POST
/// 하는 라우트.
/// ★POST 인 이유(GET 아님)★: 같은 bearer 미들웨어를 타야 하는데, 미들웨어는 세션 id 없는 **GET 을 400** 으로
///   끊는다(세션 operation 규약 — bearer_auth 1.5단계). 조회는 세션을 쓰지 않으므로 send 와 같은 무-세션
///   POST 형태로 맞춘다(경로마다 인증 규칙을 갈라 두 규율을 만들지 않는다).
const CONTROL_MESSAGES_PATH: &str = "/control/messages";

/// CLI 제어 입구(ADR-0132 결정 6) — `engram agent <동사>` 가 POST 하는 라우트. 동사는 경로가 아니라
/// **바디**(`{verb: …}`)로 온다: CLI 의 HTTP 클라이언트는 std 만으로 손조립한 단발 POST 하나뿐이라, 동사마다
/// 경로를 늘리면 그 클라이언트가 경로 표를 지게 되고 라우트 상수도 동사 수만큼 늘어난다.
/// ★인증은 `/control/send` 와 같은 미들웨어다★ — 무-세션 POST 라 위 `CONTROL_MESSAGES_PATH` 주석의 규율이
///   그대로 적용된다(경로마다 인증 규칙을 갈라 두 규율을 만들지 않는다).
/// ★제어는 전원 개방이다(ADR-0132 결정 5)★ — 스폰된 에이전트는 백엔드와 무관하게 이 라우트에 닿는다.
///   우편만 자격증명으로 갈린다(`ControlRoute::is_mail`).
const CONTROL_AGENT_PATH: &str = "/control/agent";

/// CLI 발견 입구 — 데몬 자기 표와 붙어 있는 클라이언트 명부를 **한 목록**으로 낸다(`control::catalog`).
/// ★읽기 전용이다★ — 어떤 상태도 바꾸지 않는다.
/// ★POST 인 이유(GET 아님)★: 위 `CONTROL_MESSAGES_PATH` 주석의 규율 그대로다(무-세션 GET 은 미들웨어가
///   400 으로 끊는다). 경로마다 인증 규칙을 갈라 두 규율을 만들지 않는다.
/// ★우편이 막힌 자격증명도 닿는다★ — 발견은 제어 평면이고, 편지를 못 쓰는 백엔드도 무엇을 부를 수
///   있는지는 알아야 한다(`ControlRoute::is_mail`).
/// ★`pub(super)` 인 이유★: 이웃 `catalog` 의 회복 안내가 이 경로를 문구에 싣고, 그 테스트가 이 상수를 태워
///   대조한다 — 문구가 없는 주소를 가리키는 것을 막는 유일한 수단이다.
pub(super) const CONTROL_COMMANDS_PATH: &str = "/control/commands";

/// CLI 범용 호출 입구 — 명령을 **전체 이름**으로 부른다(`{name, args}`).
///
/// ★`/control/agent` 의 별칭이 아니다★: 그쪽은 한 계열의 이름을 달고 있고 그 계열 어휘(`{verb}`)와
///   계열별 회복 안내를 진다. 한 주소가 두 뜻을 갖게 하면 닫힌 라우트 열거가 분류를 강제하는 힘을 잃는다.
/// ★검문은 형제와 **같은 함수**다★(`commands::call_daemon_command`) — 검문 없는 두 번째 입구를 만들지
///   않는다는 것이 이 라우트를 더하며 지킨 유일한 하드 제약이다.
const CONTROL_CALL_PATH: &str = "/control/call";

/// 제어 평면 경로의 네임스페이스 접두 — **분류를 빠뜨린 경로를 fail-closed 로 접는 기준**이다.
///
/// ★비교는 **경로 세그먼트 경계**로 한다(맨 `starts_with` 금지)★: 문자열 접두만 보면 `/controlfoo`·
///   `/control-x` 처럼 이 네임스페이스와 무관한 이름까지 접혀, 그 자리에 생길 미래 라우트가 이유 없이
///   거절당한다. 대상은 `/control` 자신과 `/control/` 아래뿐이다.
// ADR-0133
const CONTROL_PATH_PREFIX: &str = "/control";

/// 데몬이 여는 CLI 평문 라우트 전량(`/mcp` nest 는 rmcp 소유라 여기 없다).
///
/// ★라우터 조립과 우편 분류의 **단일 명단**이다★: `start_mcp_server` 가 이 명단을 돌며 라우트를 얹는다.
///   명단에 들어온 라우트는 아래 `is_mail` 의 exhaustive match 가 **컴파일 단계에서** 분류를 강제한다.
/// ★명단 밖 제어 경로는 우편으로 접힌다(`mail_gated_path`)★ — 순회 뒤에 `.route()` 를 손으로 덧붙이는
///   실수를 컴파일러가 막지는 못하지만, 그렇게 생긴 경로는 **분류 누락이 곧 거절**이 되어 조용히 열리지
///   않는다. 그래서 `.route()` 호출 지점을 이 순회 하나로 유지하는 것은 여전히 규약이되, 그 규약이 깨졌을 때
///   기본값이 안전한 쪽이다.
// ADR-0133
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControlRoute {
    Send,
    Messages,
    Agent,
    Commands,
    Call,
}

impl ControlRoute {
    const ALL: [ControlRoute; 5] = [
        Self::Send,
        Self::Messages,
        Self::Agent,
        Self::Commands,
        Self::Call,
    ];

    const fn path(self) -> &'static str {
        match self {
            Self::Send => CONTROL_SEND_PATH,
            Self::Messages => CONTROL_MESSAGES_PATH,
            Self::Agent => CONTROL_AGENT_PATH,
            Self::Commands => CONTROL_COMMANDS_PATH,
            Self::Call => CONTROL_CALL_PATH,
        }
    }

    /// 이 라우트가 **우편**인가 — 우편 가부 거절(ADR-0133 결정 3)이 걸리는 축.
    ///
    /// ★`_` arm 을 넣지 말 것★: catch-all 이 생기는 순간 새 라우트가 조용히 "우편 아님"(= 전원 통과)으로
    ///   분류된다. 그 실수는 런타임에 아무 신호도 내지 않는다.
    ///
    /// ★이 exhaustive match 가 강제하는 것은 **라우트**뿐이다★ — `/control/call` 은 경로가 아니라 **바디의
    ///   이름**으로 명령을 고르므로, 그 축은 여기서 못 닫는다. 그 축을 닫는 것은 [`mail_by_name`] 이고
    ///   호출 라우트가 그것을 자기 자리에서 부른다.
    ///   ★「오늘 데몬 표가 `agent.*` 만 쥐고 있어 도달 불가」를 근거로 되돌리지 말 것 — 그 문장은 이제
    ///   거짓이다★: 도달 가능한 집합은 데몬 표 **∪ 명부**이고(ADR-0160), 명부의 이름은 대시보드가 정한다
    ///   (`src-tauri/src/layout/commands.rs`). 그 이름을 짓는 사람은 이 주석을 영영 읽지 않으므로 산문으로
    ///   맡길 수 있는 축이 아니다.
    // ADR-0133
    // ADR-0160
    const fn is_mail(self) -> bool {
        match self {
            Self::Send | Self::Messages => true,
            // 제어 평면 — 발견과 호출은 우편이 아니다. 편지를 못 쓰는 자격증명도 무엇을 부를 수 있는지
            //   알아야 하고, 부를 수 있어야 한다(ADR-0132 결정 5 — 제어는 전원 개방).
            Self::Agent | Self::Commands | Self::Call => false,
        }
    }

    /// 명단 밖 경로(예: `/mcp` 와 그 하위)는 `None`.
    fn from_path(path: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.path() == path)
    }
}

/// 이 요청 경로에 우편 거절 게이트가 걸리는가(ADR-0133 결정 3).
///
/// ★분류 누락은 **거절**로 접는다(fail-closed)★: 명단에 있는 라우트는 자기 분류를 따르고, 명단 **밖**의
///   제어 경로(`/control…`)는 우편으로 본다. 이 게이트의 목적 자체가 거절이라, 누군가 라우트를 늘리며
///   분류를 빠뜨렸을 때의 기본값은 "막힌다" 여야 한다 — 반대로 두면 그 실수가 런타임에 아무 신호도 없이
///   입구를 하나 여는 것이 된다.
/// ★`/mcp` 와 제어 밖 경로는 대상이 아니다★: MCP 는 그 자격증명의 **자기 채널**이고(ADR-0128 결정 1),
///   나머지는 애초에 이 서버가 열지 않는 경로다.
// ADR-0133
fn mail_gated_path(path: &str) -> bool {
    match ControlRoute::from_path(path) {
        Some(route) => route.is_mail(),
        None => {
            path == CONTROL_PATH_PREFIX
                || path
                    .strip_prefix(CONTROL_PATH_PREFIX)
                    .is_some_and(|rest| rest.starts_with('/'))
        }
    }
}

/// 이 자격증명으로 **이 HTTP/CLI 우편 입구**를 쓸 수 없을 때의 반려 코드(ADR-0133 결정 3).
///
/// ★이름이 주장하는 범위를 좁혀 읽을 것 — "우편 금지" 가 아니다★: 이 코드가 나가는 스폰(= MCP 가능
///   백엔드)은 `/mcp` 의 `send_message` 로 **정상적으로 우편을 쓴다**. 채널은 백엔드 capability 로만
///   갈리고 런타임 스위칭이 없다는 것이 설계이고(ADR-0128 결정 1), 이 게이트는 그 설계에서 **닫혀 있어야
///   할 쪽 입구**를 닫는다. 이름만 보고 "MCP 로 우회 가능 = 게이트 구멍" 으로 읽지 말 것.
const MAIL_NOT_ALLOWED_CODE: &str = "MAIL_NOT_ALLOWED";

/// ★대안 채널을 알리지 않는다(ADR-0133 결정 3)★: 여기서 "대신 이걸 써라" 를 말하는 순간, 프라이밍 두 파일이
///   서로의 채널을 이름조차 언급하지 않도록 봉인한 게이트(ADR-0128 의 실질 게이트)가 이 응답 경로로
///   우회된다. 그래서 문구는 "이 자격증명으로는 우편을 못 쓴다" 까지이고 상대 채널 이름·툴 이름을 담지
///   않는다. 재시도 무의미까지만 알려 무한 재시도를 막는다.
const MAIL_NOT_ALLOWED_HINT: &str =
    "This credential is not allowed to use mail. Retrying will not change that.";

/// 이 **명령 이름**이 우편인가 — 경로가 아니라 바디로 명령을 고르는 라우트(`/control/call`)의 우편 축.
///
/// ★이름 하나로 닫는 이유★: 그 라우트가 도달할 수 있는 집합은 데몬 자기 표 **∪ 명부**이고(ADR-0160),
/// 명부 쪽 이름은 이 crate 밖에서 정해진다(대시보드가 자기 명령을 얹는다). 즉 이 축은 **여기 있는 코드를
/// 다 읽어도** 알 수 없는 이름까지 포함하므로, 「오늘은 그런 이름이 없다」는 근거가 될 수 없다.
/// ★계열 구분자는 명령 이름 규약(`<계열>.<동사>`)의 것이다★ — 계열이 `mail` 이면 우편이다. 접두 비교로
/// 쓰지 말 것: `mailbox.x` 는 우편 계열이 아닌데 접두로는 걸린다. 그 규약의 정본은 명령 표이고 이 글자를
/// 공유 상수로 올리지 않은 이유는 CLI 도 자기 것을 따로 들고 있어서다(`bin/engram.rs`) — 셋째 사본을
/// 만들기 전에 그 상수를 어디 둘지가 먼저 정해져야 한다.
/// ★오늘 이 판정이 참이 되는 이름은 하나도 없다(실측)★ — 데몬 표는 `agent.*` 뿐이고 대시보드가 얹는
/// 이름에도 `mail.*` 이 없다. 그래서 이것은 구멍을 막는 것이 아니라 **난간**이다: `mail.*` 이 어느 쪽으로
/// 들어오든 그날 이 줄이 이미 서 있다.
// ADR-0133
// ADR-0160
fn mail_by_name(name: &str) -> bool {
    name.split_once('.')
        .is_some_and(|(group, _)| group == engram_dashboard_core::agent::types::CLI_GROUP_MAIL)
}

/// ★manager 늦은 주입 슬롯(순환 해소)★: 데몬 기동은 MCP 서버를 **먼저** 띄우고(그 URL 로 mcp-config 를
/// 발급하는 DaemonControlChannel 을 만들어야 하므로) 그 다음 AgentManager 를 배선한다 — 즉 서버 start
/// 시점엔 아직 manager 가 없다. 그래서 서버엔 빈 슬롯을 넘기고, manager 조립 직후 `set` 으로 채운다.
/// 요청 처리(send)는 accept loop 이후라 이 시점엔 항상 채워져 있다(에이전트가 붙기 전에 set 완료).
#[derive(Default)]
pub struct ManagerSlot {
    inner: std::sync::OnceLock<Arc<AgentManager>>,
}

impl ManagerSlot {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set(&self, manager: Arc<AgentManager>) {
        let _ = self.inner.set(manager);
    }
    fn get(&self) -> Option<&Arc<AgentManager>> {
        self.inner.get()
    }
}

/// ★MessagingService 늦은 주입 슬롯(순환 해소 — C1)★: ManagerSlot 과 동형. MessagingService 는
/// AgentManager 를 감싸므로(DeliveryPort) manager 조립 **후**에야 만들어진다.
#[derive(Default)]
pub struct MessagingSlot {
    inner: std::sync::OnceLock<Arc<engram_dashboard_messaging::service::MessagingService>>,
}

impl MessagingSlot {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set(&self, svc: Arc<engram_dashboard_messaging::service::MessagingService>) {
        let _ = self.inner.set(svc);
    }
    /// messaging_host 의 flush 레인(`run_flush_lane`)도 부르므로 crate 내부에 노출한다.
    pub(crate) fn get(
        &self,
    ) -> Option<&Arc<engram_dashboard_messaging::service::MessagingService>> {
        self.inner.get()
    }
}

/// ★명부 통지 팬아웃 늦은 주입 슬롯(ADR-0132)★: ManagerSlot 과 동형. 팬아웃은 연결 레지스트리에서
/// 파생되는데 그 레지스트리는 MCP 서버보다 **뒤에** 조립된다(lib.rs `run()` 5c → 6).
///
/// ★비어 있는 것이 정상인 조립이 있다★: 스모크 bin·격리 하네스는 붙을 클라이언트가 없다. 운영 데몬에서
///   비어 있으면 제어 동사로 바뀐 명부가 화면에 반영되지 않는다(`agent::RosterBroadcast` 참조).
#[derive(Default)]
pub struct RosterBroadcastSlot {
    inner: std::sync::OnceLock<Arc<dyn super::agent::RosterBroadcast>>,
}

impl RosterBroadcastSlot {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set(&self, broadcast: Arc<dyn super::agent::RosterBroadcast>) {
        let _ = self.inner.set(broadcast);
    }
    /// 명령 표의 통지 어댑터도 읽는다 — 그쪽은 **부를 때마다** 이것을 본다(`commands` 모듈).
    pub(crate) fn get(&self) -> Option<&Arc<dyn super::agent::RosterBroadcast>> {
        self.inner.get()
    }
}

/// ★데몬 명령 표 늦은 주입 슬롯(ADR-0155)★: ManagerSlot 과 동형. 표는 매니저를 쥐므로 매니저 조립
/// **뒤**에야 만들어지는데, 그 매니저를 담을 슬롯 자체는 MCP 서버보다 앞에 있어야 한다.
///
/// ★표가 늦게 와도 명부 통지는 안 늦는다★ — 표가 쥐는 것은 팬아웃 값이 아니라 위 슬롯이다
/// (`commands::make_daemon_table`).
#[derive(Default)]
pub struct CommandTableSlot {
    inner: std::sync::OnceLock<Arc<engram_dashboard_command::CommandTable>>,
}

impl CommandTableSlot {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set(&self, table: Arc<engram_dashboard_command::CommandTable>) {
        let _ = self.inner.set(table);
    }
    /// `/control/agent` 어댑터가 요청마다 읽는다 — 비어 있으면 그 라우트는 503 이다(요청 형식·인증
    /// 문제가 아니라 배선 순서 이상이라 4xx 가 아니다).
    pub(crate) fn get(&self) -> Option<&Arc<engram_dashboard_command::CommandTable>> {
        self.inner.get()
    }
}

/// MCP 세션 식별 헤더명(rmcp/스펙 표준, 소문자).
const SESSION_ID_HEADER: &str = "mcp-session-id";

/// ★`send_message` MCP 툴 이름 = **단일 출처(ADR-0094)**★. 아래 `#[tool]` 메서드명이 곧 rmcp 가
///   `tools/list` 에 노출하는 툴 이름이고, 이 const 가 그 이름의 **정본**이다 — ADR-0094 발신 권한
///   grant 가 `mcp__{server}__{tool}` 패턴을 만들 때 tool 로 쓴다(DaemonControlChannel.provision).
///   claude 문법(`mcp__..`) 지식은 backend/claude.rs 단독 — 이 const 는 이름만 제공한다(ADR-0004/0094).
pub const SEND_MESSAGE_TOOL: &str = "send_message";

/// ★`messages` MCP 툴 이름(D · spec §6)★ — `SEND_MESSAGE_TOOL` 과 같은 규율.
///   ★grant 대상 아님(의도적)★: ADR-0094 의 pre-authorization 은 **발신 입구**만 담는다는 결정이라
///   (control/mod.rs `build_grants`), 조회 툴은 grant 목록에 넣지 않는다.
pub const MESSAGES_TOOL: &str = "messages";

pub struct McpServerHandle {
    /// mcp-config 에 박아 넣을 엔드포인트 URL(예: `http://127.0.0.1:54321/mcp`).
    pub url: String,
    /// 종료 신호 — cancel 하면 accept loop + 활성 세션이 정리된다.
    cancel: CancellationToken,
    /// axum::serve 태스크 핸들(종료 시 join 대기 — 테스트 누수 방지). ★Option★: Drop 이 있는 타입에서
    ///   `shutdown(self)` 가 핸들을 move 해 await 할 수 있게 take 로 꺼낸다(Drop 트레이트 타입은 필드
    ///   부분 이동 불가 — round-2 F5).
    serve_handle: Option<tokio::task::JoinHandle<()>>,
}

impl McpServerHandle {
    pub async fn shutdown(mut self) {
        self.cancel.cancel();
        if let Some(h) = self.serve_handle.take() {
            let _ = h.await;
        }
    }
}

impl Drop for McpServerHandle {
    /// ★drop-on-error airtight(round-2 F5)★: 핸들이 `shutdown().await` 없이 그냥 drop 되면(예: MCP
    ///   서버 start 뒤 daemon.json write 같은 **후속** startup 단계가 실패해 에러 반환으로 이 핸들이
    ///   drop 되는 경우) detached serve 태스크가 취소 신호를 못 받고 계속 돌 수 있다. Drop 에서 cancel
    ///   토큰을 발화해, 어느 경로로 drop 되든 serve 태스크(graceful_shutdown 이 cancel 을 관측)가
    ///   확실히 종료되게 한다. 정상 종료 경로에선 `shutdown()` 이 이미 cancel 했으므로 여기 cancel 은
    ///   멱등 no-op 이다.
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// ★신원 검증 = 미들웨어(FIX 12)★: 신원은 auth 미들웨어가 검증해 요청 extensions 에 심고(BoundIdentity),
///   세션↔신원 바인딩·pinning·정리도 전부 미들웨어(State 로 registry 접근)가 한다 — 핸들러는 extensions
///   에서 신원을 읽기만 한다.
#[derive(Clone)]
pub struct EngramMcpHandler {
    tool_router: ToolRouter<Self>,
    manager: Arc<ManagerSlot>,
    /// 미들웨어와 공유하는 **동일 Arc** — 두 번째 registry 를 만들지 않는다.
    registry: Arc<ControlRegistry>,
    messaging: Arc<MessagingSlot>,
}

#[tool_router]
impl EngramMcpHandler {
    pub fn new(
        manager: Arc<ManagerSlot>,
        registry: Arc<ControlRegistry>,
        messaging: Arc<MessagingSlot>,
    ) -> Self {
        Self {
            tool_router: Self::tool_router(),
            manager,
            registry,
            messaging,
        }
    }

    /// 연결된 에이전트가 이 툴을 호출하면 세션 바인딩이 end-to-end 로 통함이 증명된다(acceptance 관측점).
    #[tool(description = "Diagnostic ping — returns pong and the caller's bound agent identity")]
    async fn engram_ping(
        &self,
        _params: Parameters<PingArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let identity = ctx
            .extensions
            .get::<http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<BoundIdentity>().copied());
        match identity {
            Some(BoundIdentity { agent_id, epoch }) => {
                Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                    "pong agent={agent_id} epoch={epoch}"
                ))]))
            }
            None => Err(ErrorData::invalid_request(
                "no bound identity in request context (auth middleware should have set it)",
                None,
            )),
        }
    }

    /// ★★이 메서드명(`send_message`)은 반드시 `SEND_MESSAGE_TOOL` 상수와 같아야 한다(ADR-0094 단일
    ///   출처)★★. rmcp `#[tool]` 매크로가 **메서드명**을 `tools/list` 툴 이름으로 그대로 쓰므로(2.2.0
    ///   tool.rs: `name = attribute.name.unwrap_or_else(|| fn_ident.to_string())`, 매크로 attr 는 String
    ///   **리터럴**만 받아 const 주입 불가 — 검증함), 이름 정본인 그 const 를 컴파일타임에 여기 붙일 방법이
    ///   없다. 런타임 테스트 `tools_list_exposes_send_message_tool` 이 두 곳을 묶는다.
    ///   ⚠️ 이 메서드명을 바꾸려면 `SEND_MESSAGE_TOOL` const 도 함께 바꿔라 — 안 그러면 grant 가 존재하지
    ///   않는 툴을 가리켜 발신 입구가 조용히 막히고, 테스트만 이를 잡는다.
    // ADR-0086 / ADR-0094(단일 출처 결합)
    #[tool(
        description = "Send a message to teammate agents. You are one agent on a team; use this \
        tool to reply to or reach other live agents. `to` = one teammate or a LIST of them — each \
        entry is an agent name (or agent id), or a group address: \"@here\" = everyone live right \
        now EXCEPT you, \"@all\" = every agent in the team tree EXCEPT you, including ones that \
        are not running (their copy waits and is delivered when they come back). You can mix them, \
        e.g. [\"@here\", \"qa-bravo\"]. `body` = your message text. \
        The sender envelope (who you are, message id) is added automatically by the broker — your \
        identity comes from your bound session, not from arguments, so just write the body \
        naturally. Set `request` = true when you need answers back (optionally with `reply_by` = \
        \"5m\"/\"10m\"/\"1h\" — at least 1 minute, after which YOU get notified for each recipient \
        that did not reply); with several recipients that opens one independent reply contract per \
        recipient. When you answer a message that arrived with type=\"request\" and an id, pass \
        `reply_to` = that id — a reply must have exactly one recipient. `request` and `reply_to` \
        are mutually exclusive. The result has one row per recipient: status delivered (injected \
        now), pending (queued until that agent finishes its turn) or failed (that recipient only — \
        `code` says why, e.g. RECIPIENT_NOT_FOUND if it is not running; the others still got it). \
        Delivery is at-least-once: if this call fails or times out without a result, the message \
        may already have been delivered — check before resending, because a retry is a NEW \
        message, not a replacement."
    )]
    async fn send_message(
        &self,
        params: Parameters<SendArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let Some(from) = ctx
            .extensions
            .get::<http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<BoundIdentity>().copied())
        else {
            return Err(ErrorData::invalid_request(
                "no bound identity in request context (auth middleware should have set it)",
                None,
            ));
        };

        let Some(manager) = self.manager.get() else {
            // ★F6 계측(error, M4)★: 정상 흐름엔 없는 branch — manager 슬롯이 아직 안 채워짐(배선 순서 이상).
            //   안전한 폴백이 없다(메시지 배달 불가) → warn 이 아니라 error(logging-conventions: error =
            //   "사람이 반드시 봐야 함" = 배선 결함). 데몬 배선 순서를 손봐야 하는 결함 신호다.
            tracing::error!(
                entrance = "mcp",
                "제어 채널 send 불가 — manager 슬롯 미설정(배선 순서 이상, ADR-0086 F6)"
            );
            return Err(ErrorData::internal_error(
                "control channel not ready (manager not wired)",
                None,
            ));
        };
        let Some(messaging) = self.messaging.get() else {
            tracing::error!(
                entrance = "mcp",
                "제어 채널 send 불가 — messaging 슬롯 미설정(배선 순서 이상, C1)"
            );
            return Err(ErrorData::internal_error(
                "control channel not ready (messaging not wired)",
                None,
            ));
        };
        let Parameters(SendArgs {
            to,
            body,
            request,
            reply_by,
            reply_to,
        }) = params;
        let cmd = ControlCommand {
            from,
            to: to.into_tokens(),
            body,
            contract: SendContract {
                request: request.unwrap_or(false),
                reply_by,
                reply_to,
            },
        };
        // ★blocking 경계(C4 리뷰 fix D · load-bearing)★: `handle_send` 안의 `inject` 는 자식 stdin 의
        //   **blocking write** 다. 그룹 방송이면 한 요청이 멤버 수만큼 그 write 를 **직렬로** 지므로, 이
        //   async 핸들러에서 그대로 부르면 막힌 파이프 하나가 tokio 워커 스레드를 통째로 잡고 그 스레드에
        //   얹힌 **다른 요청까지** head-of-line 블로킹한다(런타임 워커는 코어 수만큼뿐). 그래서 blocking
        //   풀로 옮긴다 — 단일 발송도 같은 write 를 하므로 같은 대우를 받는다(경로를 갈라 두 규율을
        //   만들지 않는다).
        //   ★flush 레인과 같은 규율★: 배치 write 를 요청 처리 스레드에서 떼어내는 것(service.rs FlushTrigger).
        //
        // ★그 대가 = **at-least-once 배달**(round-3 fix 7 · 의도된 설계, 문서화 필요)★: `spawn_blocking`
        //   클로저는 **abort 불가**다. 호출자가 요청을 중도 취소하면(HTTP 연결 끊김·MCP 클라이언트 종료·
        //   타임아웃) 이 `.await` 는 사라지지만 **블로킹 태스크는 끝까지 돈다** — 즉 배달과 장부 커밋은
        //   그대로 일어나고 **응답만** 유실된다. 발신자가 재시도하면 데몬은 그걸 새 `msg_id` 의 새 발송으로
        //   보므로(멱등 키가 없다) 같은 내용이 한 번 더 배달/방송된다.
        //   ★왜 이대로 두나★: 반대 선택(취소 시 배달도 취소)은 불가능하다 — 자식 stdin 에 이미 쓴 바이트는
        //   회수할 수 없고, "쓰기 전에 취소를 확인" 은 또 하나의 TOCTOU 다. 그래서 **중복 > 유실**을 택한다:
        //   중복은 수신 LLM 이 읽고 판단할 수 있는 가시적 사실이지만, 유실은 아무도 모르는 조용한 실패다
        //   (ADR-0103 "조용한 유실 금지" 와 같은 방향).
        //   ★미래 확장점★: 발신자가 고르는 **멱등 키**(재시도 시 같은 값)를 받아 장부에서 중복 발송을 접는
        //   것. 지금은 `msg_id` 가 데몬 생성이라 그 역할을 못 한다(재시도마다 새 값).
        let (manager, registry, messaging) =
            (manager.clone(), self.registry.clone(), messaging.clone());
        let result = tokio::task::spawn_blocking(move || {
            handle_send(&manager, &registry, &messaging, Entrance::Mcp, cmd)
        })
        .await
        .map_err(|e| {
            tracing::error!(entrance = "mcp", "제어 채널 send 태스크 실패(패닉): {e}");
            ErrorData::internal_error("send task failed", None)
        })?;
        let json = serde_json::to_string(&result.to_json()).unwrap_or_default();
        Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
    }

    #[tool(
        description = "Look up message state on the Engram broker. With no arguments it returns \
        YOUR open items — messages you sent that have not landed yet, requests you are waiting on \
        an answer for, and requests other agents sent you that you have NOT answered yet (each row \
        is tagged with `direction`, and `reply_owed_by_me` means you still owe that agent a reply). \
        Pass `id` = a message id (e.g. m-7f3k9q2d) to see that one message's delivery state \
        instead; for a group broadcast you get one row per recipient. This tool only reads — it \
        never sends, replies, or changes anything."
    )]
    async fn messages(
        &self,
        params: Parameters<MessagesArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let Some(from) = ctx
            .extensions
            .get::<http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<BoundIdentity>().copied())
        else {
            return Err(ErrorData::invalid_request(
                "no bound identity in request context (auth middleware should have set it)",
                None,
            ));
        };
        let (Some(manager), Some(messaging)) = (self.manager.get(), self.messaging.get()) else {
            tracing::error!(
                entrance = "mcp",
                "messages 조회 불가 — manager/messaging 슬롯 미설정(배선 순서 이상)"
            );
            return Err(ErrorData::internal_error("control channel not ready", None));
        };
        let Parameters(MessagesArgs { id }) = params;
        let result = handle_messages(manager, messaging, from, id.as_deref());
        let json = serde_json::to_string(&result.to_json()).unwrap_or_default();
        Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
    }
}

/// 툴 인자 — ping 은 인자가 없다(빈 struct). schemars(rmcp 재수출)로 input schema 자동 생성.
#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct PingArgs {}

/// ★`to` 의 복수 표기(spec §6 — ADR-0111 다중 수신자)★: **문자열 1개 또는 문자열 배열**.
///
/// ★단일 문자열 형태는 그대로 유효하다(하위 호환)★ — 기존 호출·기존 프라이밍이 무변경으로 돈다. 각 원소는
///   에이전트 이름·agent id·`@`주소이며 **혼용 가능**하다.
/// ★MCP 배열 원소는 **절대 콤마로 쪼개지 않는다**(spec §6)★: 배열이라는 구조가 이미 경계를 주므로 이중
///   분해를 하지 않는다 — 원소 `"a,b"` 는 그런 **이름 하나**로 취급되고(없으면 `RECIPIENT_NOT_FOUND` 행),
///   콤마 분해는 CLI 입구(`/control/send`)만의 규칙이다.
/// ★schemars★: `#[serde(untagged)]` 라 입력 스키마가 `anyOf[string, array<string>]` 로 나간다 — 호출 LLM 이
///   두 형태 모두 유효함을 스키마만 보고 안다.
// ADR-0111
#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum ToField {
    /// 수신자 하나(이름·agent id·`@`주소).
    One(String),
    /// 수신자 여러 명(각 원소가 이름·agent id·`@`주소).
    Many(Vec<String>),
}

impl ToField {
    pub fn into_tokens(self) -> Vec<String> {
        match self {
            ToField::One(s) => vec![s],
            ToField::Many(v) => v,
        }
    }
}

/// `send_message` 인자 — 수신자 지목(`to`) + 본문(`body`) + 회신 계약(C3, 전부 선택). ★from 필드 없음★:
/// 발신자는 세션 신원에서만 파생한다(payload from 금지 — ADR-0086 불변식). schemars 로 input schema 자동 생성.
///
/// ★타입 문자열 인자 없음(구조적 — ADR-0103 불변식)★: `type` 을 문자열로 받지 않고 `request: bool` 만 둔다 —
///   그래야 에이전트가 `type="notice"`(데몬 전용 태그)를 밀반입할 표면 자체가 없다.
/// ★doc 주석 = 툴 스키마 설명★: schemars 가 이 주석을 property description 으로 싣는다(수신 LLM 이 읽는 계약).
// ADR-0103 (C3 — spec §6 send_message { to, body, request?, reply_by?, reply_to? })
#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct SendArgs {
    /// Who to send to: one teammate, or a list. Each entry is an agent name, an exact agent id, or a
    /// group address: "@here" (everyone live except you) or "@all" (every agent in the team tree
    /// except you, including ones that are not running — their copy is delivered when they come
    /// back). You can mix them, e.g. ["@here", "qa-bravo"].
    pub to: ToField,
    /// 메시지 본문(텍스트).
    pub body: String,
    /// Set true when you need answers back: the broker tracks this message as awaiting a reply.
    /// With several recipients this opens ONE INDEPENDENT reply contract per recipient — each of
    /// them owes you their own answer, one of them replying does not close the others, and each
    /// silent one gets its own deadline notice. Mutually exclusive with reply_to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<bool>,
    /// Reply deadline for a request, as an integer + unit: "5m", "10m", "1h" (minimum 1 minute —
    /// deadlines are checked once a minute, so anything shorter is rejected; "60s" is accepted).
    /// Only valid together with request. On timeout the broker notifies YOU (the sender), not the recipient.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_by: Option<String>,
    /// The id of the request you are answering (the `id` attribute on the message you received).
    /// A reply goes to EXACTLY ONE recipient — the agent that sent the request; passing several
    /// recipients (or any "@" address) with reply_to is rejected. Mutually exclusive with request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
}

/// `messages` 인자(D · spec §6 `messages { id? }`). 전부 선택 — 무인자가 "내 미결" 조회다.
/// ★신원 필드 없음★: "나" 는 세션 신원에서만 온다(payload 로 남의 미결을 볼 수 없다 — ADR-0086 불변식).
#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct MessagesArgs {
    /// A message id to inspect (e.g. "m-7f3k9q2d"). Omit to list your own open items instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

// router = self.tool_router — 저장한 필드를 실제로 읽게 해 dead_code 를 피하고, 핸들러마다 라우터를
// 재빌드하지 않는다(factory 가 세션마다 new() 하므로 라우터를 필드에 한 번 만들어 두는 게 효율적).
#[tool_handler(router = self.tool_router)]
impl ServerHandler for EngramMcpHandler {
    fn get_info(&self) -> ServerInfo {
        // ServerInfo(=InitializeResult)는 #[non_exhaustive] 라 struct 리터럴 불가 → ctor 체인 사용.
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Engram daemon control channel (ADR-0086). Available tools: engram_ping, send_message, messages.",
        )
    }
}

/// bearer auth 미들웨어(ADR-0086) — MCP handshake **전에** 토큰을 검증하고 세션↔신원을 고정한다.
///
/// 검사 순서:
///   1. Authorization 의 `Bearer <token>` → registry.validate. 실패면 401(inner 미호출 = handshake 미생성).
///   1.2. 우편 가부(ADR-0133 결정 3) — 요청 **경로**가 우편 라우트인데 이 자격증명의 우편이 막혀 있으면
///      여기서 반려한다. `/mcp` 와 제어 라우트는 이 검사에 걸리지 않는다(`ControlRoute::is_mail`).
///   1.5. Mcp-Session-Id 형식/필수성 → 400. 이 검사가 아래 바인딩 검사의 "session op 는 반드시 바인딩으로
///      resolve 된다"를 보장한다(rmcp 내부 4xx 동작에 의존하지 않음).
///   2. 세션 바인딩 검사(FIX 7 + round-2 F1) — 일치=통과 / 신원 불일치=**403** / 바인딩 없음=**404**.
///      initialize 는 아직 세션 id 가 없어 건너뛴다(세션은 응답에서 생성). DELETE 면 신원 확인 후 세션
///      바인딩을 prune 한다(FIX 8/F6).
///   3. 검증된 신원을 요청 extensions 에 심어 inner(StreamableHttpService)로 넘긴다 → rmcp 가
///      `http::request::Parts` 로 툴에 흘린다(공식 custom-extension 패턴).
///   4. 응답에 새 Mcp-Session-Id 가 있으면(initialize 성공) `bind_session_if_absent` 로 신원을 세션에
///      **한 번만** 고정한다. 실패(중복/죽음)는 바인딩 생략(중복은 무해, 죽음은 다음 요청에서 401/403).
///
/// ★왜 미들웨어에서 401/403(handshake 전)인가★: rmcp 는 인증을 내장하지 않는다(공식 auth 패턴 = axum
///   미들웨어). 검증을 handshake 안으로 미루면 잘못된 토큰도 세션을 만든다 — 여기서 막아 "거부는 어떤
///   MCP 세션 상태 변경도 전에"를 보장한다(acceptance).
async fn bearer_auth<B>(
    State(registry): State<Arc<ControlRegistry>>,
    request: Request<B>,
    next: Next,
) -> Response
where
    B: Send + 'static,
    Request<B>: Into<Request<axum::body::Body>>,
{
    // ★"Bearer " 접두 엄격성은 의도적(FIX 13)★: 이 헤더는 데몬이 mcp-config 에 **직접 authored** 한
    //   값이라(claude 가 그대로 전송) 형식이 고정돼 있다 — 대소문자 변형·여분 공백 등 관대한 파싱을
    //   할 이유가 없다(범용 서버가 아니다). 정확히 `"Bearer "` prefix 만 허용.
    let token = request
        .headers()
        .get(http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string());

    let Some(token) = token else {
        return unauthorized();
    };
    let Some(binding) = registry.validate(&token) else {
        return unauthorized();
    };
    let identity = binding.identity;

    // ★우편 거절은 핸들러가 아니라 여기다(ADR-0133 결정 3)★: 이 미들웨어는 라우터 **전체**를 감싸므로
    //   우편 라우트가 늘어도 한 자리로 덮인다 — 핸들러마다 검사를 두면 새 핸들러가 검사를 빠뜨린다.
    //   판정 재료는 발급 시점에 박힌 자격증명의 사실 하나뿐이라 매니저 조회도, 바디 파싱도 필요 없다.
    // ★이것이 유일한 강제 지점이다★: 에이전트 쪽 표식(`MAIL_MARKER_ENV`)은 사용법을 가릴 뿐이고 조작
    //   가능하다 — 이 검사를 빼고 표식만 남기면 표식을 떼는 순간 우편이 열린다.
    // ★막는 것은 이 HTTP/CLI 우편 표면뿐이다★: MCP 가능 에이전트는 `/mcp` 의 `send_message` 로 계속 우편을
    //   쓴다(ADR-0128 결정 1 — 채널은 capability 로만 갈린다). 이 거절은 "우편 금지" 가 아니라 "이 입구는
    //   네 채널이 아니다" 다.
    // ★warn 인 이유(info 아님)★: 기본 로그 레벨이 warn 이라 info 로 두면 "왜 이 에이전트가 편지를 못
    //   보내나" 를 쫓는 운영자에게 **아무 흔적도 남지 않는다**. ADR-0133 의 재검토 트리거(거절만 받은
    //   에이전트의 행동 관측)도 이 줄이 보여야 발동한다. 형제 거부(401/403/404/400)와 같은 레벨로 맞춘다.
    // ★이 표면의 우편 축은 **둘**이다 — 이 줄은 경로 축이다★: 바디로 명령을 고르는 라우트는 경로로
    //   분류되지 않아 자기 자리에서 같은 판정·같은 로그를 낸다(`control_call_handler` 의 이름 축).
    // ADR-0133
    if !binding.mail_allowed && mail_gated_path(request.uri().path()) {
        tracing::warn!(
            agent = %identity.agent_id,
            epoch = identity.epoch,
            "우편 요청 거절 — 이 자격증명은 CLI/HTTP 우편 입구의 것이 아니다(ADR-0133)"
        );
        return mail_not_allowed();
    }

    // ★요청이 실어 온 기존 세션 id(있으면)★ — initialize 이후의 후속 요청(tools/call·GET·DELETE)은
    //   Mcp-Session-Id 를 헤더로 싣는다. 이 값으로 identity pinning 을 검사한다(초기 initialize 는 없음).
    // ★malformed ≠ absent(Codex LOW)★: 헤더가 **있으나** to_str() 이 실패하면(비-UTF-8 등) 이걸 None 으로
    //   접으면 안 된다 — None 으로 접으면 세션-실은 요청이 "sessionless" 로 오인돼 아래 바인딩 검사를 건너뛰고
    //   inner(rmcp)로 통과한다(경계 우회). present-but-malformed 는 클라이언트 오류이므로 바인딩 검사에
    //   닿기 전에 400 으로 끊는다(신원·인증 문제는 아니므로 401/403 이 아니라 400, body 는 비움). 진짜로
    //   **부재**한 헤더만 "sessionless" 로 취급한다(initialize 경로).
    let method = request.method().clone();
    let req_session_id = match request.headers().get(SESSION_ID_HEADER) {
        None => None,
        Some(v) => match v.to_str() {
            Ok(s) => Some(s.to_string()),
            Err(_) => {
                tracing::warn!(
                    "제어 채널 malformed Mcp-Session-Id 헤더 거부(400, ADR-0086 Codex LOW)"
                );
                return bad_request();
            }
        },
    };

    // ★세션 operation(GET/DELETE)은 세션 바인딩으로 resolve 돼야(security lens)★: GET(SSE stream)·DELETE
    //   (teardown)은 **기존 세션에 대한 조작**이라 반드시 세션 id 를 실어야 한다. 세션 id 없는 GET/DELETE 는
    //   바인딩으로 귀결될 수 없으므로 inner 로 넘기지 않고 여기서 400 으로 끊는다("no inner reach without a
    //   binding" 경계 무결성을 rmcp 내부 4xx 동작에 의존하지 않고 미들웨어에서 보장). POST 무-세션id 는
    //   예외 — 그게 initialize 경로다(세션은 응답에서 생성되므로 아직 세션 id 가 없는 게 정상).
    if req_session_id.is_none() && (method == Method::GET || method == Method::DELETE) {
        tracing::warn!(
            method = %method,
            "제어 채널 세션 operation 무-세션id 거부(400, ADR-0086 — session op 는 바인딩으로 resolve 돼야)"
        );
        return bad_request();
    }

    if let Some(sid) = &req_session_id {
        match registry.identity_for_session(sid) {
            Some(bound) if bound == identity => {}
            Some(_) => {
                // 세션 id 도 호출자가 고른 헤더 값이라 [`sanitize_for_log`] 를 거친다(같은 위조 축).
                //   HTTP 파서가 CR/LF 는 이미 거르지만 탭·비-ASCII obs-text 는 통과하고 길이도 우리 것이
                //   아니다 — 여기서 폭까지 우리가 정한다.
                tracing::warn!(
                    session = %sanitize_for_log(sid),
                    "제어 채널 cross-token 세션 탈취 거부(403, ADR-0086 FIX 7)"
                );
                return forbidden();
            }
            // ★orphaned-session 거부(round-2 F1)★: 세션 id 를 실어 왔는데 데몬 바인딩이 **없다**.
            //   예전엔 이걸 inner(rmcp)로 통과시켜 rmcp 가 404 를 내게 했는데, 그 경로엔 치명적 창이 있다:
            //   에이전트 A 가 세션 S 를 열어 바인딩됐다가 revoke(kill)로 **바인딩만** prune 되면 rmcp 측
            //   세션 S 는 아직 살아 있을 수 있다. 그때 유효 토큰을 든 에이전트 B 가 S 를 제시하면 미들웨어가
            //   그대로 통과시켜 B 가 A 의 고아 세션 워커에 attach 된다(세션 탈취). 이제 **바인딩 없는
            //   세션-실은 요청은 전부 거부**해 그 창을 닫는다 — rmcp 측에 살아 있으나 데몬이 모르는 세션은
            //   도달 불가(unreachable orphan)가 된다. 이는 DELETE-prune 순서도 fail-safe 로 만든다.
            //   truly-unknown id 는 예전에도 rmcp 404 를 받았으므로 정상 클라이언트가 보는 상태코드는
            //   바뀌지 않는다(happy-path 무영향).
            None => {
                tracing::warn!(
                    session = %sanitize_for_log(sid),
                    "제어 채널 orphaned/unknown 세션 거부(404, ADR-0086 F1)"
                );
                return not_found();
            }
        }
        // ★unbind-before-inner 순서 선택(round-2 F6)★: inner(rmcp)가 실제 세션 close 를 하기 **전에**
        //   데몬 바인딩을 먼저 지운다. F1(바인딩 없는 세션-실은 요청 거부)이 들어온 지금 이 순서가
        //   fail-safe 다: unbind 후 inner close 가 어떤 이유로 실패해 rmcp 측 세션이 남더라도, 데몬
        //   바인딩이 이미 없으므로 그 세션은 F1 에 의해 **도달 불가(unreachable orphan)**가 된다 —
        //   즉 "바인딩은 지웠는데 세션 워커는 살아 있는" 상태가 보안 창을 열지 않는다. 반대로
        //   unbind-after-close 로 하면 close 성공에 prune 이 매달려, close 실패 시 바인딩이 남아
        //   무한 성장·stale 바인딩 위험이 생긴다. 신원 검사(위 match)를 통과한 뒤라 임의 prune 도 아니다.
        if method == Method::DELETE {
            registry.unbind_session(sid);
        }
    }

    let mut request: Request<axum::body::Body> = request.into();
    request.extensions_mut().insert(identity);
    // ★권한 비트를 신원과 **따로** 싣는다★: `BoundIdentity` 는 세션 pinning 의 동일성 키라 권한이 끼면
    //   인가 문제가 세션 탈취 오탐으로 둔갑한다(그 타입 doc). 그래도 핸들러가 이 비트를 봐야 하는 자리가
    //   하나 있다 — 바디로 명령을 고르는 라우트의 우편 축([`mail_by_name`])은 위 라우트 분류가 못 닫는다.
    request.extensions_mut().insert(binding);

    let response = next.run(request).await;

    // ★검증에 쓴 그 토큰 문자열을 함께 넘긴다(FIX 7 + round-2 F2)★: bind 가 그 토큰이 아직 이 agent 의
    //   현재 크레덴셜인지 국소 비교해, validate→bind 창의 revoke/재발급을 걸러낸다.
    if let Some(session_id) = response
        .headers()
        .get(SESSION_ID_HEADER)
        .and_then(|v| v.to_str().ok())
    {
        registry.bind_session_if_absent(session_id, identity, &token);
    }
    response
}

/// 401 응답(빈 body). WWW-Authenticate 는 굳이 넣지 않는다(정적 Bearer 이라 챌린지 불필요).
fn unauthorized() -> Response {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .body(axum::body::Body::empty())
        .expect("valid 401 response")
}

/// 403 응답(빈 body) — cross-token 세션 탈취 거부(FIX 7). 토큰 자체는 유효하나(그래서 401 아님) 이
/// 세션에 접근할 권한이 없다(다른 신원에 고정된 세션).
fn forbidden() -> Response {
    Response::builder()
        .status(StatusCode::FORBIDDEN)
        .body(axum::body::Body::empty())
        .expect("valid 403 response")
}

/// 404 응답(빈 body) — orphaned/unknown 세션 거부(round-2 F1). 데몬 바인딩이 없는 세션 id 를 실어 온
/// 요청. 토큰은 유효하나 이 세션은 데몬 인가 관점에서 존재하지 않는다(다른 신원 소유가 확정된 것도
/// 아니므로 403 이 아니라 404). body 는 비워 어떤 세션·신원 정보도 누출하지 않는다.
fn not_found() -> Response {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(axum::body::Body::empty())
        .expect("valid 404 response")
}

/// 우편 거절 응답(ADR-0133 결정 3).
///
/// ★200 + 반려 봉투인 이유(빈 403 이 아니라)★: 우편 라우트의 계약이 "항상 200 + JSON" 이고, CLI 는
///   stdout 형태가 아니라 **exit code** 로 판정한다 — 이 봉투는 검증된 반려 shape 이라 기존 3분법의
///   **실패(1)** 로 그대로 접힌다(새 결말 부류를 만들지 않는다). 빈 403 으로 내면 CLI 는 빈 줄만 찍고
///   호출자는 왜 실패했는지 알 방법이 없다.
/// ★body 문구는 `MAIL_NOT_ALLOWED_HINT` 가 정본★ — 대안 채널을 알리지 않는다.
// ADR-0133
fn mail_not_allowed() -> Response {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "error",
            "code": MAIL_NOT_ALLOWED_CODE,
            "hint": MAIL_NOT_ALLOWED_HINT,
        })),
    )
        .into_response()
}

/// 400 응답(빈 body) — 클라이언트 요청 형식 오류(ADR-0086). 신원·인증 문제가 아니라 요청 형식 문제라
/// 401/403/404 가 아니라 400. body 는 비워 어떤 정보도 누출하지 않는다.
fn bad_request() -> Response {
    Response::builder()
        .status(StatusCode::BAD_REQUEST)
        .body(axum::body::Body::empty())
        .expect("valid 400 response")
}

// ── CLI 입구(/control/send) ────────────────────────────────────────────────────────

/// `/control/send` 요청 바디. `{to, body, request?, reply_by?, reply_to?}` — from 필드 없음(신원은 토큰에서만).
/// C3 인자는 전부 선택이라 옛 `{to, body}` 바디와 **wire 호환**이다(누락 = 통보).
#[derive(Debug, serde::Deserialize)]
struct SendRequest {
    /// 수신자 — 문자열 1개(콤마 목록 허용) 또는 배열.
    to: ToField,
    body: String,
    #[serde(default)]
    request: Option<bool>,
    #[serde(default)]
    reply_by: Option<String>,
    #[serde(default)]
    reply_to: Option<String>,
}

/// ★CLI 입구의 수신자 토큰화(순수 함수 — 리뷰 C3)★: `engram mail send --to a,b` 는 셸에서 목록을 표현할 방법이
/// 콤마뿐이라 **이 입구에서만** 한 번 쪼갠다. 분해 규칙이 **입구별로 다른 게 의도**다(MCP 쪽 규칙은
/// `ToField` 주석).
fn cli_recipient_tokens(to: ToField) -> Vec<String> {
    to.into_tokens()
        .into_iter()
        .flat_map(|t| t.split(',').map(|p| p.to_string()).collect::<Vec<_>>())
        .collect()
}

/// `/control/send` 라우트 State — MCP factory 와 **같은 Arc** 를 공유한다(두 번째 registry 를 만들지 않는다).
#[derive(Clone)]
struct ControlSendState {
    manager: Arc<ManagerSlot>,
    registry: Arc<ControlRegistry>,
    messaging: Arc<MessagingSlot>,
}

/// 항상 200 + JSON body(성공/교정 에러 모두 열린 요청에 실린다 — CLI 가 JSON 을 파싱해 exit code 를 정한다).
async fn control_send_handler(
    axum::extract::State(state): axum::extract::State<ControlSendState>,
    identity: Option<axum::Extension<BoundIdentity>>,
    body: Option<Json<SendRequest>>,
) -> Response {
    let Some(axum::Extension(from)) = identity else {
        // 미들웨어가 신원을 심지 않았다 = 인증 경로 이상. 방어적 401(정상 흐름에선 도달 불가).
        return unauthorized();
    };
    let Some(Json(req)) = body else {
        return bad_request();
    };
    let Some(manager) = state.manager.get() else {
        tracing::error!(
            entrance = "cli",
            "제어 채널 send 불가 — manager 슬롯 미설정(배선 순서 이상, ADR-0086 F6)"
        );
        return service_unavailable();
    };
    let Some(messaging) = state.messaging.get() else {
        tracing::error!(
            entrance = "cli",
            "제어 채널 send 불가 — messaging 슬롯 미설정(배선 순서 이상, C1)"
        );
        return service_unavailable();
    };
    let cmd = ControlCommand {
        from,
        to: cli_recipient_tokens(req.to),
        body: req.body,
        contract: SendContract {
            request: req.request.unwrap_or(false),
            reply_by: req.reply_by,
            reply_to: req.reply_to,
        },
    };
    let (manager, registry, messaging) =
        (manager.clone(), state.registry.clone(), messaging.clone());
    let Ok(result) = tokio::task::spawn_blocking(move || {
        handle_send(&manager, &registry, &messaging, Entrance::Cli, cmd)
    })
    .await
    else {
        // JoinError = blocking 태스크 패닉(정상 흐름엔 없음). "항상 200 + JSON" 계약은 **핸들러가 답을
        //   만들어낸 경우**의 계약이라, 답 자체가 없는 이 경로는 500 으로 갈라 CLI 가 성공으로 오독하지
        //   않게 한다(새 wire 에러 코드를 발명하지 않는다 — spec §6 어휘 고정).
        tracing::error!(entrance = "cli", "제어 채널 send 태스크 실패(패닉)");
        return Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(axum::body::Body::empty())
            .expect("valid 500 response");
    };
    Json(result.to_json()).into_response()
}

/// `/control/messages` 요청 바디(D) — `{id?}`. 신원 필드 없음(토큰 파생 — send 와 같은 불변식).
#[derive(Debug, Default, serde::Deserialize)]
struct MessagesRequest {
    #[serde(default)]
    id: Option<String>,
}

/// ★빈 바디 허용★: `pending`(무인자 조회)은 보낼 필드가 없다. CLI 가 `{}` 를 싣지만, 바디 자체가 없거나
///   파싱이 안 돼도 **무인자 조회로 접는다** — 조회는 부작용이 없어 관대해도 안전하고, 여기서 400 을 내면
///   "인자를 안 준 것" 과 "요청이 깨진 것" 을 CLI 가 구분하지 못해 자기교정이 헛돈다(send 는 반대로 400 —
///   거긴 필수 필드가 있고 부작용이 있다).
async fn control_messages_handler(
    axum::extract::State(state): axum::extract::State<ControlSendState>,
    identity: Option<axum::Extension<BoundIdentity>>,
    body: Option<Json<MessagesRequest>>,
) -> Response {
    let Some(axum::Extension(from)) = identity else {
        return unauthorized();
    };
    let (Some(manager), Some(messaging)) = (state.manager.get(), state.messaging.get()) else {
        tracing::error!(
            entrance = "cli",
            "messages 조회 불가 — manager/messaging 슬롯 미설정(배선 순서 이상)"
        );
        return service_unavailable();
    };
    let req = body.map(|Json(b)| b).unwrap_or_default();
    let result = handle_messages(manager, messaging, from, req.id.as_deref());
    Json(result.to_json()).into_response()
}

// ── CLI 제어 입구(/control/agent) ──────────────────────────────────────────────────

/// `/control/agent` 라우트 State — 명령 표 슬롯 하나뿐이다(ADR-0155).
///
/// ★매니저·팬아웃 슬롯을 담지 않는다★: 둘 다 **표가** 쥐고 있다(`commands::make_daemon_table`). 여기서
///   다시 담으면 같은 실물로 가는 두 번째 경로가 생기고, 통지가 두 곳에서 나가면 동사별 통지 횟수(깨우기
///   0 · 변경 1)라는 분담이 무너진다. registry·messaging 도 안 담는다 — 제어 동사는 신원을 인가에만
///   쓰고 발신자 파생이 없다.
#[derive(Clone)]
struct ControlAgentState {
    commands: Arc<CommandTableSlot>,
}

/// 항상 200 + JSON body(성공/반려 모두) — `/control/messages` 와 같은 계약이라 CLI 의 조회 판정기가
/// 그대로 쓰인다.
///
/// ★신원은 인가에만 쓴다★: 제어 동사엔 "나" 가 등장하지 않는다(발신자 파생 없음). 그래도 미들웨어가 심은
///   신원의 **존재**는 요구한다 — 토큰 없는 호출이 여기 닿으면 안 되기 때문이다(방어적 401).
/// ★blocking 경계★: 아래 `handle_agent` 는 blocking 이다(그 함수 doc — resume 폴링 ~3초 + 프로필 디스크
///   저장). async 핸들러에서 그대로 부르면 tokio 워커 스레드 하나가 그 시간 동안 묶여 같은 스레드에 얹힌
///   **다른 요청까지** 막힌다(send 라우트가 자식 stdin write 를 blocking 풀로 옮긴 것과 같은 이유).
async fn control_agent_handler(
    axum::extract::State(state): axum::extract::State<ControlAgentState>,
    identity: Option<axum::Extension<BoundIdentity>>,
    body: axum::body::Bytes,
) -> Response {
    if identity.is_none() {
        return unauthorized();
    }
    // ★`Json<…>` 추출기를 쓰지 않는 이유★: 그 추출기는 역직렬화가 실패하면 **빈 400** 을 낸다 —
    //   타입이 어긋난 값·중복 키·객체가 아닌 바디·깨진 JSON 이 전부 그 자리로 떨어져, 호출자(LLM)는 무엇을
    //   고쳐야 하는지 모른 채 재시도한다. 직접 역직렬화하면 serde 의 사유를 그대로 봉투에 실을 수 있고,
    //   그러면 이 라우트가 빈 body 를 내는 경우는 인증 실패와 크기 초과뿐이다.
    let req: super::agent::AgentRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return Json(
                super::agent::malformed_body(&e.to_string(), &String::from_utf8_lossy(&body))
                    .to_json(),
            )
            .into_response()
        }
    };
    let Some(table) = state.commands.get() else {
        tracing::error!(
            entrance = "cli",
            "제어 동사 처리 불가 — 명령 표 슬롯 미설정(배선 순서 이상, ADR-0155)"
        );
        return service_unavailable();
    };
    let table = table.clone();
    let Ok(result) =
        tokio::task::spawn_blocking(move || super::agent::handle_agent(&table, req)).await
    else {
        // JoinError = blocking 태스크 패닉. 답 자체가 없으므로 "항상 200 + JSON" 계약 밖이다 — 500 으로
        //   갈라 CLI 가 성공으로 오독하지 않게 한다(send 라우트와 같은 규율). ★그 계약이 **debug 에서만**
        //   성립한다는 것은 `internal_error` 주석이 정본이다★ — 릴리스는 `panic = "abort"` 다.
        tracing::error!(entrance = "cli", "제어 동사 태스크 실패(패닉)");
        return internal_error();
    };
    Json(result.to_json()).into_response()
}

// ── CLI 발견·범용 호출 입구(/control/commands · /control/call) ──────────────────────

/// 두 카탈로그 라우트의 State — 발견이 합치는 **두 출처**를 그대로 든다(`control::catalog::merge`).
///
/// ★명부는 슬롯이 아니라 **핸들**이다★: 표(`CommandTableSlot`)는 매니저 뒤에 생겨 늦은 주입이 필요하지만,
///   명부는 `Clone` 인 공유 핸들이고 조립부가 서버보다 **먼저** 만든다(`lib.rs` run()·accept loop). 늦은
///   주입 슬롯을 하나 더 두면 「비어 있으면 반쪽 목록」이라는 갈래를 이유 없이 늘린다.
/// ★두 번째 명부를 만들지 않는다★ — 여기 드는 것은 연결이 등록을 얹는 **그 한 부**여야 한다. 사본을 쥐면
///   발견은 영원히 빈 목록을 내고 그 증상은 에러도 로그도 없다.
#[derive(Clone)]
struct ControlCatalogState {
    commands: Arc<CommandTableSlot>,
    /// 명부 + 자리 표 + 1단계 표를 한 값으로 든다 — ★소켓 입구가 드는 **그 한 부**여야 한다★
    /// (ADR-0160). 사본을 쥐면 발견은 빈 목록을 내고 중계는 자기만의 자리 표를 쓴다.
    bus: crate::command_delivery::CommandBus,
}

/// 발견 목록 — 성공·반려 모두 200 + JSON이고, 답 자체가 없는 경우만 500 이다(그 갈래의 범위는
/// [`internal_error`] 주석). ★표 슬롯이 비어도 503 이 아니다★: 같은 상태에서 WS 목록이 내는 답이
/// 「명부 절반」이라, 여기서 503 을 내면 두 표면이 같은 상태를 두고 다른 말을 한다(그 선택의 정본 =
/// `catalog::handle_list` doc).
///
/// ★`spawn_blocking` 이 무엇을 고쳤고 무엇을 안 고쳤나 — 넘겨짚지 말 것★
///   - **고친 것 = 실행기 위험.** `CommandRoster::entries` 가 잡는 것은 blocking 뮤텍스라, async 워커에서
///     부르면 그 워커는 양보가 아니라 **park** 된다. 이제 그 대기는 blocking 풀이 진다. 형제
///     (`control_agent_handler`)와 대우를 맞춘 것이기도 하다 — 같은 잠금을 한쪽만 워커에서 잡을 이유가 없다.
///   - **안 고친 것 = 잠금 유지 시간.** `entries` 는 여전히 뮤텍스를 **쥔 채** 명부 전량(상한 4096 ×
///     help 4 KiB)을 복제하고, 그 뮤텍스는 명령 배달(`OwnerLookupSource::lookup`)과 연결 정리가 함께 쓴다.
///     즉 이 라우트를 도는 자식은 **여전히 배달과 다툰다** — 바뀐 것은 **누가 기다리는가**이지 얼마나 오래
///     잠기는가가 아니다. 그 복제 규율은 명부 모듈 소유라 여기서 줄일 수 없다(`command_roster` 헤더의
///     「알려진 비용」).
/// ★`decls()` 는 부를 때마다 `help` 를 다시 만든다(도구 crate `CommandTable::decls`) — 그대로 둔다★:
///   데몬 자기 표는 오늘 5개뿐이라 이 응답의 비용은 명부 쪽이 지배하고, 메모이제이션은 그 crate 소유라
///   여기서 손댈 자리가 아니다. 표가 커지면 그쪽에서 잰다.
async fn control_commands_handler(
    axum::extract::State(state): axum::extract::State<ControlCatalogState>,
    identity: Option<axum::Extension<BoundIdentity>>,
) -> Response {
    if identity.is_none() {
        return unauthorized();
    }
    let Ok(result) = tokio::task::spawn_blocking(move || {
        let mine = state
            .commands
            .get()
            .map(|table| table.decls())
            .unwrap_or_default();
        let registered = state.bus.roster().entries();
        super::catalog::handle_list(mine, registered)
    })
    .await
    else {
        tracing::error!(entrance = "cli", "발견 목록 태스크 실패(패닉)");
        return internal_error();
    };
    Json(result.into_json()).into_response()
}

/// 전체 이름 호출 — 항상 200 + JSON(성공/반려 모두), `/control/agent` 와 같은 계약이다.
///
/// ★`Json<…>` 추출기를 쓰지 않는 이유·`spawn_blocking` 경계 모두 형제(`control_agent_handler`)와 같다★ —
///   그쪽 주석이 정본이다. 두 라우트가 같은 표를 같은 함수로 태우므로 대접도 같아야 한다.
///
/// ★이 라우트는 **마감까지 지연될 수 있다**(ADR-0160 영향절)★: 남의 이름은 그 주인에게 중계되고, 주인이
///   답하지 않으면 배달 마감(`CommandDeliveries::DEFAULT_DEADLINE`)이 `TIMEOUT` 을 낸다. 데몬 자기 이름은
///   예전 그대로 그 자리에서 끝나므로 이 지연을 지지 않는다.
/// ★`request_id` 는 **호출자가 실어 보낼 수 있다**★(ADR-0161 결정 1) — 안 실었으면 판정부가 발급한다
///   (`catalog::handle_call`). 그 갈림을 여기서 다시 만들지 않는다.
async fn control_call_handler(
    axum::extract::State(state): axum::extract::State<ControlCatalogState>,
    binding: Option<axum::Extension<super::registry::TokenBinding>>,
    body: axum::body::Bytes,
) -> Response {
    let Some(axum::Extension(binding)) = binding else {
        return unauthorized();
    };
    let caller = binding.identity;
    let req: super::catalog::CallRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return Json(
                super::catalog::malformed_body(&e.to_string(), &String::from_utf8_lossy(&body))
                    .to_json(),
            )
            .into_response()
        }
    };
    // ★우편 축은 **이름으로** 닫는다★ — 이 라우트는 경로가 아니라 바디로 명령을 고르므로 미들웨어의
    //   라우트 분류가 여기까지 못 온다(그 분류 doc · [`mail_by_name`]). 실행에 필요한 무엇을 잡기 전에,
    //   그리고 이름이 어느 출처의 것인지 가리기 전에 판정한다 — 데몬 표든 명부든 같은 답이어야 한다.
    // ★거절을 형제와 **같은 레벨·같은 필드**로 남긴다★(`bearer_auth` 의 경로 축 게이트): 안 남기면 우편이
    //   막힌 자격증명이 **이름으로 더듬어 보는 동안 서버에 흔적이 하나도 안 남고**, ADR-0133 의 재검토
    //   트리거(거절만 받은 에이전트의 행동 관측)도 이 축에서는 발동할 재료를 잃는다.
    // ★이름은 [`sanitize_for_log`] 를 거친다 — `agent::preview` 를 쓰지 말 것★: 이 줄은 **검증 전**에
    //   찍히므로 이름은 완전히 임의의 문자열이고, 그 함수는 길이만 자르고 제어문자를 그대로 흘린다. 개행
    //   하나면 로그 줄이 둘로 쪼개져 뒤 줄이 우리가 찍은 항목처럼 보인다(위조 항목 — 그 함수 doc 이 이
    //   실패 모드를 적는다). `warn` 이라 기본 레벨에서 실제로 파일에 남는다는 점이 이 축을 진짜로 만든다.
    // ADR-0133
    if !binding.mail_allowed && mail_by_name(&req.name) {
        tracing::warn!(
            agent = %caller.agent_id,
            epoch = caller.epoch,
            command = %sanitize_for_log(&req.name),
            "우편 요청 거절 — 이 자격증명은 CLI/HTTP 우편 입구의 것이 아니다(이름 축, ADR-0133)"
        );
        return mail_not_allowed();
    }
    // ★이 검사는 **배선 경보**이지 라우팅 권위가 아니다★: 권위는 아래 `bus.locals()` 하나이고(그래야 판정과
    //   배달이 같은 사실을 본다 — `catalog::handle_call` 의 그 문단), 이 줄이 지키는 것은 「표를 늦게 꽂는
    //   조립에서 이름을 보기도 전에 크게 드러난다」는 이 라우트의 계약뿐이다(그 대접의 정본 =
    //   `tests/control_agent.rs` 의 두 라우트 대조 시험). ★조립부가 이 슬롯과 버스의 1단계 표를 **같은 한
    //   부**로 넘겨야 이 경보가 실제 상태를 가리킨다★ — 운영 조립이 그렇게 한다(`lib.rs`).
    if state.commands.get().is_none() {
        tracing::error!(
            entrance = "cli",
            "전체 이름 호출 불가 — 명령 표 슬롯 미설정(배선 순서 이상, ADR-0155)"
        );
        return service_unavailable();
    }
    let bus = state.bus.clone();
    let locals = Arc::clone(bus.locals());
    let registered = bus.roster().clone();
    let Ok(outcome) = tokio::task::spawn_blocking(move || {
        super::catalog::handle_call(locals.as_ref(), &registered, req)
    })
    .await
    else {
        tracing::error!(entrance = "cli", "전체 이름 호출 태스크 실패(패닉)");
        return internal_error();
    };
    let result = match outcome {
        super::catalog::CallOutcome::Answered(result) => result,
        super::catalog::CallOutcome::Relay {
            name,
            args,
            request_id,
        } => {
            // ★누가 시킨 명령인지 남긴다★: 중계된 이름은 대시보드의 창·탭·슬롯을 실제로 움직이므로
            //   (`src-tauri` 의 layout 명령들) 「내 창을 누가 닫았나」에 답할 수 있어야 한다. 그 재료는
            //   이 문에 **이미 도착해 있는데**(`BoundIdentity`) 여태 존재 확인에만 쓰고 버렸다.
            //   ★열쇠는 싣지 않는다★ — 신원은 열쇠에서 파생됐지만 그 자체는 자격증명이 아니다
            //   (로깅 컨벤션 「보안」).
            // ★warn 인 이유(info 아님)★: 이 줄이 담는 것은 **사건이 지나간 뒤 어디서도 다시 만들 수 없는
            //   사실**이다 — 사용자 눈에 보이는 변경을 **어느 자격증명이** 시켰나. 조작의 결과는 화면과
            //   상태 조회에 남지만 주인은 아무 데도 안 남으므로, 나중에 레벨을 올려도 이미 벌어진 조작의
            //   주인은 그때도 모른다. 그리고 이 라우트는 「에이전트가 사람 화면을 움직였다」를 평상시 잡음이
            //   아니라 **봐야 할 사건으로** 취급하기로 했다 — 같은 판단이 이 파일의 형제 게이트에 이미 박혀
            //   있다(우편 거절의 `warn` 절 — `bearer_auth`).
            //   ★「기본 레벨이 warn 이라 info 는 안 찍힌다」를 사유로 쓰지 말 것★ — 로깅 컨벤션이 그 논거를
            //   명시적으로 금한다(정상 알림의 레벨을 기본 필터 때문에 올리는 것). 위 두 문장이 사유다.
            //   ★그래서 중계 한 번에 warn 은 이 한 줄이다★ — 결말 쪽 성공 줄은 `info` 로 남는다(그쪽 주석).
            //   ★이 예외를 로깅 컨벤션 문서에 적는 일은 아직 안 됐다(미결 — 이 라운드 범위 밖).★
            //   도착률은 이 문의 자연 상한 안이다(요청당 연결 · `MAX_RELAYED_IN_FLIGHT`).
            // ★입구 라벨은 상수를 쓴다★ — 이 호출은 `CommandBus::invoke` 를 타므로 그쪽이 남기는 라벨과
            //   같아야 한다. `"cli"` 는 **중계를 안 타는 직접 실행** 몫이라(그 상수 doc) 여기 것이 아니다.
            // ★이름은 [`sanitize_for_log`] 를 거친다★ — 사유는 위 우편 거절 줄과 같다(호출자가 고른
            //   문자열이 `warn` 로 로그에 들어간다). 배달 쪽도 같은 함수를 쓴다(`command_delivery::deliver`
            //   첫 줄) — 한 값이 두 층에서 다른 모양으로 적히지 않게 하려는 것이다.
            // ★문구가 「중계했다」고 말하지 않는다 — 이 줄은 왕복 **앞**이다★: 이 뒤의 `invoke` 는 동시
            //   상한을 쳤으면 아무것도 안 돌리고 반려하므로(`MAX_RELAYED_IN_FLIGHT`), 「중계한다」로 적으면
            //   그 갈래에서 일어나지 않은 일을 기록한 줄이 남는다. 사실만 적고 결말은 짝 줄이 낸다 —
            //   `CommandBus::invoke` 가 성공/실패 한 줄을, 상한 거절은 `RelaySlot::admit` 이 한 줄을 낸다
            //   (셋 다 같은 `entrance`·`request_id` 를 실어 한 왕복으로 묶인다).
            // ★그래도 이 줄을 왕복 **뒤로** 미루지 말 것★: 귀속은 사건이 지나가면 어디서도 다시 만들 수
            //   없으므로(아래 사유) 데몬이 왕복 중에 죽으면 미룬 줄은 아예 안 남는다.
            tracing::warn!(
                entrance = crate::command_delivery::ENTRANCE_CONTROL,
                agent = %caller.agent_id,
                epoch = caller.epoch,
                command = %sanitize_for_log(&name),
                %request_id,
                "에이전트가 남의 명령을 불렀다 — 이 데몬이 그 주인 앞으로 접수했다"
            );
            let reply = bus.invoke(request_id, name, args).await;
            super::catalog::relayed(reply)
        }
    };
    Json(result.into_json()).into_response()
}

/// 500 응답(빈 body) — blocking 태스크가 패닉해 **답 자체가 없는** 경우.
///
/// ★릴리스에서는 이 갈래에 닿지 못한다(알려진 범위)★: 워크스페이스 `[profile.release]` 가
///   `panic = "abort"` 라 blocking 태스크의 패닉은 `JoinError` 가 아니라 **프로세스 종료**다. 즉 "패닉 →
///   500" 은 debug 에서만 성립하는 계약이고, 배포판에서 이 자리가 지키는 것은 없다. 그래도 지우지 않는
///   이유는 debug·테스트 빌드에서 그 갈래가 실재하고, 없으면 그 빌드에서 답 없는 요청이 매달리기 때문이다.
fn internal_error() -> Response {
    Response::builder()
        .status(StatusCode::INTERNAL_SERVER_ERROR)
        .body(axum::body::Body::empty())
        .expect("valid 500 response")
}

/// 503 응답(빈 body) — 슬롯 미설정(배선 순서 이상). 요청 형식·인증 문제가 아니므로 4xx 가 아니다.
fn service_unavailable() -> Response {
    Response::builder()
        .status(StatusCode::SERVICE_UNAVAILABLE)
        .body(axum::body::Body::empty())
        .expect("valid 503 response")
}

/// registry 는 auth 미들웨어(검증)와 provision(발급)이 공유하는 **동일 Arc** 여야 한다.
///
/// ★로컬 전용 + DNS rebinding 방어★: bind 는 127.0.0.1:0(OS 할당 포트). StreamableHttpServerConfig 는
///   기본 allowed_hosts=[localhost,127.0.0.1,::1] 로 로컬 Host 만 허용(rmcp 기본). stateful_mode=true(기본)
///   라 세션이 Mcp-Session-Id 로 유지된다.
pub async fn start_mcp_server(
    registry: Arc<ControlRegistry>,
    manager: Arc<ManagerSlot>,
    messaging: Arc<MessagingSlot>,
    // ★팬아웃 슬롯은 여기로 오지 않는다★: 그것을 읽는 것은 명령 표뿐이고(`commands::make_daemon_table`),
    //   서버가 두 번째 사본을 쥐면 **조립부가 다른 슬롯 둘을 넘겨도 아무도 안 알려 준다** — 증상은 에러도
    //   로그도 없이 트리가 영원히 옛 명부를 보여 주는 것이다. 인자에서 뺀 것이 그 갈림을 없앤다.
    commands: Arc<CommandTableSlot>,
    // ★명령 버스는 **값으로** 온다★: `Clone` 인 공유 핸들이고 조립부가 이 서버보다 먼저 만들 수
    //   있으므로(그렇게 하도록 `lib.rs` 를 고쳤다) 늦은 주입 슬롯이 필요 없다. 발견 목록의 절반과 중계
    //   배달이 여기서 나오므로 조립부가 **수락 루프가 드는 그 한 부**를 넘겨야 한다 — 새로 만들어 넘기면
    //   발견은 영원히 데몬 자기 이름만 내고 중계는 자기만의 자리 표를 쓴다(ADR-0160).
    bus: crate::command_delivery::CommandBus,
) -> std::io::Result<McpServerHandle> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr: SocketAddr = listener.local_addr()?;
    let url = format!("http://127.0.0.1:{}{}", addr.port(), MCP_PATH);

    let cancel = CancellationToken::new();

    // StreamableHttpServerConfig 는 #[non_exhaustive] 라 struct 리터럴 불가 → Default + builder 메서드.
    let config =
        StreamableHttpServerConfig::default().with_cancellation_token(cancel.child_token());
    let factory_manager = manager.clone();
    let factory_registry = registry.clone();
    let factory_messaging = messaging.clone();
    let mcp_service = StreamableHttpService::new(
        move || {
            Ok(EngramMcpHandler::new(
                factory_manager.clone(),
                factory_registry.clone(),
                factory_messaging.clone(),
            ))
        },
        Arc::new(LocalSessionManager::default()),
        config,
    );

    // ★nest_service★: StreamableHttpService 는 Tower service 라 axum 라우터에 그대로 얹힌다.
    // ★body 상한 = RequestBodyLimitLayer(round-2 F4)★: 로컬 제어 채널의 요청 바디는 작다 — 악성/폭주 바디로
    //   메모리를 삼키지 않게 상한을 명시한다. axum `DefaultBodyLimit` 는 **extractor**(Json/Bytes 등)에만
    //   걸리는데 rmcp `StreamableHttpService` 는 raw body 를 직접 소비하므로(extractor 미경유) 그 상한이 통하지
    //   않는다. `RequestBodyLimitLayer` 는 body 자체를 감싸 하위 소비자 전부(rmcp 포함)에 상한을 강제하고,
    //   초과 시 413 로 끊는다. 1MB 면 initialize/tools/call·send POST 페이로드에 충분하다.
    // ★레이어 순서★: 아래는 바깥→안 순서로 body-limit → auth → 라우트로 쌓인다(axum layer 는 나중에 쓴 게
    //   바깥). body-limit 를 가장 바깥에 둬 auth·inner 어느 쪽이 body 를 읽든 그 전에 상한이 적용되게 한다.
    const MAX_BODY_BYTES: usize = 1024 * 1024;
    let send_state = ControlSendState {
        manager: manager.clone(),
        registry: registry.clone(),
        messaging: messaging.clone(),
    };
    let catalog_state = ControlCatalogState {
        commands: commands.clone(),
        bus,
    };
    let agent_state = ControlAgentState { commands };
    // ★명단(`ControlRoute::ALL`)을 돌며 얹는다 — 빌더 체인으로 되돌리지 말 것★: 새 라우트가 명단에
    //   들어와야 서빙이 되고, 들어오면 `is_mail` 의 exhaustive match 가 우편 분류를 컴파일 단계에서
    //   강제한다(ADR-0133). 체인은 그 강제를 우회한다.
    let mut app = axum::Router::new().nest_service(MCP_PATH, mcp_service);
    for route in ControlRoute::ALL {
        app = match route {
            ControlRoute::Send => app.route(
                route.path(),
                axum::routing::post(control_send_handler).with_state(send_state.clone()),
            ),
            ControlRoute::Messages => app.route(
                route.path(),
                axum::routing::post(control_messages_handler).with_state(send_state.clone()),
            ),
            ControlRoute::Agent => app.route(
                route.path(),
                axum::routing::post(control_agent_handler).with_state(agent_state.clone()),
            ),
            ControlRoute::Commands => app.route(
                route.path(),
                axum::routing::post(control_commands_handler).with_state(catalog_state.clone()),
            ),
            ControlRoute::Call => app.route(
                route.path(),
                axum::routing::post(control_call_handler).with_state(catalog_state.clone()),
            ),
        };
    }
    let app = app
        .layer(axum::middleware::from_fn_with_state(
            registry.clone(),
            bearer_auth,
        ))
        .layer(tower_http::limit::RequestBodyLimitLayer::new(
            MAX_BODY_BYTES,
        ));

    let serve_cancel = cancel.clone();
    let serve_handle = tokio::spawn(async move {
        let server = axum::serve(listener, app.into_make_service());
        let graceful = server.with_graceful_shutdown(async move {
            serve_cancel.cancelled().await;
        });
        if let Err(e) = graceful.await {
            tracing::warn!("MCP axum serve 종료: {e}");
        }
    });

    tracing::info!(
        port = addr.port(),
        path = MCP_PATH,
        "MCP 서버 시작(ADR-0086)"
    );
    Ok(McpServerHandle {
        url,
        cancel,
        serve_handle: Some(serve_handle),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ADR-0133: 라우트 분류 — 새 라우트가 조용히 "우편 아님" 으로 새지 않는다 ──────────────────

    /// ★이 함수가 이 테스트의 본체다★: variant 를 늘리면 여기서 **컴파일이 깨져** 분류를 손으로 적게 된다.
    ///   `ControlRoute::is_mail` 과 **따로** 진술하는 것이 요점이라 그쪽을 부르지 않는다 — 둘이 갈리면
    ///   아래 단언이 빨개진다.
    fn expected_is_mail(route: ControlRoute) -> bool {
        match route {
            ControlRoute::Send => true,
            ControlRoute::Messages => true,
            ControlRoute::Agent => false,
            ControlRoute::Commands => false,
            ControlRoute::Call => false,
        }
    }

    #[test]
    fn every_registered_control_route_is_explicitly_classified() {
        for route in ControlRoute::ALL {
            assert_eq!(
                route.is_mail(),
                expected_is_mail(route),
                "{route:?}({}) 의 우편 분류가 갈렸다",
                route.path()
            );
            assert_eq!(
                ControlRoute::from_path(route.path()),
                Some(route),
                "명단에 있으면 경로로 되찾을 수 있어야: {route:?}"
            );
        }
        // 라우터는 이 명단을 돌며 조립된다 — 길이가 줄면 라우트가 조용히 사라진 것이다.
        assert_eq!(ControlRoute::ALL.len(), 5);
        assert_eq!(
            ControlRoute::ALL.iter().filter(|r| r.is_mail()).count(),
            2,
            "우편 라우트는 발송·조회 둘 — 제어 평면이 늘어도 이 수는 안 는다"
        );
    }

    /// `/mcp` 와 제어 밖 경로는 게이트 대상이 아니다(MCP 는 그 자격증명의 자기 채널이다).
    #[test]
    fn the_mcp_route_is_outside_the_mail_gate() {
        for path in [MCP_PATH, "/mcp/", "/mcp/whatever", "/", "/health"] {
            assert!(
                !mail_gated_path(path),
                "제어 평면 밖 경로에 우편 게이트가 걸리면 안 된다: {path}"
            );
        }
    }

    /// ★분류를 빠뜨린 제어 경로는 거절 쪽으로 접힌다★: 이 방향이 뒤집히면, 라우트를 늘리며 명단 등록을
    ///   잊은 실수가 **아무 신호 없이 우편 입구를 하나 여는 것**이 된다.
    #[test]
    fn an_unclassified_control_path_folds_to_the_mail_gate() {
        for path in [
            "/control/whatever",
            "/control/send/extra",
            "/control",
            "/control/",
        ] {
            assert_eq!(
                ControlRoute::from_path(path),
                None,
                "전제: 명단 밖 경로여야 이 단언이 의미를 가진다({path})"
            );
            assert!(
                mail_gated_path(path),
                "명단 밖 제어 경로는 우편으로 접혀야(fail-closed): {path}"
            );
        }
        // 명단에 있는 라우트는 자기 분류를 그대로 따른다(접기가 분류를 덮어쓰지 않는다).
        for open in [CONTROL_AGENT_PATH, CONTROL_COMMANDS_PATH, CONTROL_CALL_PATH] {
            assert!(!mail_gated_path(open), "제어 평면은 전원 개방: {open}");
        }
        for route in ControlRoute::ALL {
            assert_eq!(mail_gated_path(route.path()), route.is_mail(), "{route:?}");
        }
        // ★세그먼트 경계★: 이름만 비슷할 뿐 이 네임스페이스가 아닌 경로까지 접으면, 그 자리에 생길 미래
        //   라우트가 이유 없이 거절당한다(맨 문자열 접두 비교의 실패 모드).
        for path in [
            "/controlfoo",
            "/control-x",
            "/controls/send",
            "/CONTROL/send",
        ] {
            assert!(
                !mail_gated_path(path),
                "제어 네임스페이스 밖 이름을 접으면 안 된다: {path}"
            );
        }
    }

    /// ★거절 응답은 대안 채널을 알리지 않는다(ADR-0133 결정 3)★: 여기서 상대 채널을 알리면 프라이밍 두
    ///   파일의 상호 배타성 봉인이 이 경로로 우회된다.
    ///
    /// ★실제 생성자(`mail_not_allowed`)가 만든 body 를 읽는다 — 같은 JSON 을 여기서 다시 조립하지 말 것★:
    ///   사본을 검사하면 생성자가 필드를 하나 더 싣거나 문구를 바꿔도 이 테스트는 초록으로 남는다.
    #[tokio::test]
    async fn the_mail_rejection_names_no_other_channel() {
        let resp = mail_not_allowed();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "우편 라우트의 '항상 200 + JSON' 계약을 따른다(CLI 는 exit code 로 판정)"
        );
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .expect("거절 body");
        let body = String::from_utf8(bytes.to_vec()).expect("UTF-8 body");
        let lowered = body.to_lowercase();
        for forbidden in [SEND_MESSAGE_TOOL, MESSAGES_TOOL, "mcp", "tool", "server"] {
            assert!(
                !lowered.contains(&forbidden.to_lowercase()),
                "거절 응답이 상대 채널을 가리키면 안 된다({forbidden}): {body}"
            );
        }
        // 반려 shape 은 CLI 의 기존 3분법에 그대로 접힌다(새 결말 부류를 만들지 않는다).
        let v: serde_json::Value = serde_json::from_str(&body).expect("JSON");
        assert_eq!(v["status"], "error");
        assert!(v["code"].as_str().is_some_and(|c| !c.is_empty()));
        assert!(v["hint"].as_str().is_some_and(|h| !h.is_empty()));
    }

    // ── 로그 위조 방지 — 호출자가 고른 문자열이 로그 필드에 원문으로 들어가지 않는다 ──────────────

    use crate::log_capture::capture_loud_async as capture_loud;

    /// ★검증 **전에** 찍히는 로그 줄에 호출자 문자열이 원문으로 들어가면 로그 위조가 된다★
    ///
    /// 이 라우트의 우편 거절은 이름을 보기만 하고 아무것도 검증하지 않은 상태에서 `warn` 을 남긴다 — 즉
    /// 그 이름은 완전히 임의의 문자열이다. 개행 하나가 로그 줄을 둘로 쪼개면 뒤 줄은 우리가 찍은 항목처럼
    /// 보이고, 기본 레벨이 warn 이라 그 줄은 **운영 로그 파일에 실제로 남는다**.
    /// ★재는 것이 「함수를 불렀나」가 아니라 「그 필드에 원문이 갔나」다★ — 그래서 실제 핸들러를 태우고
    /// 로그 이벤트의 필드 텍스트를 읽는다. 길이만 자르는 함수(`agent::preview`)로 되돌리면 여기가 빨개진다.
    #[test]
    fn a_control_character_in_the_called_name_never_reaches_the_log_field_verbatim() {
        // 개행 + 위조 항목 + **폭 밖으로 밀려나야 하는 꼬리표** 하나(두 성질을 한 값으로 잰다).
        let forged = "mail.probe\n2026-08-19T00:00:00Z  WARN forged entry padded out past the field width TAILMARK";
        let lines = capture_loud(async move {
            let (bus, sweeper) = crate::command_delivery::CommandBus::without_commands();
            let state = ControlCatalogState {
                commands: empty_commands_slot(),
                bus,
            };
            let binding = axum::Extension(crate::control::registry::TokenBinding {
                identity: BoundIdentity {
                    agent_id: engram_dashboard_core::agent::types::AgentId::new_v4(),
                    epoch: 0,
                },
                mail_allowed: false,
            });
            let body = serde_json::json!({ "name": forged }).to_string();
            let resp = control_call_handler(
                axum::extract::State(state),
                Some(binding),
                axum::body::Bytes::from(body),
            )
            .await;
            assert_eq!(resp.status(), StatusCode::OK, "우편 거절은 200 + 반려 봉투");
            sweeper.stop().await;
        });

        let logged = lines
            .iter()
            .find(|line| line.contains("command="))
            .unwrap_or_else(|| panic!("거절 줄이 남아야 한다(계측 의무): {lines:?}"));
        assert!(
            !logged.contains('\n'),
            "로그 필드가 줄을 쪼갤 수 있으면 위조 항목이 생긴다: {logged:?}"
        );
        assert!(
            logged.contains("\\n"),
            "제어문자는 escape 된 모양으로 남아야(값을 지우는 것이 아니다): {logged:?}"
        );
        assert!(
            !logged.contains("TAILMARK"),
            "로그 한 줄의 폭을 상대가 정하게 하면 안 된다(폭 상한): {logged:?}"
        );
    }

    #[test]
    fn ping_args_schema_builds() {
        let schema = schemars::schema_for!(PingArgs);
        let _ = serde_json::to_string(&schema).expect("serialize schema");
    }

    /// 빈 manager 슬롯(send 를 안 부르는 서버-수명 테스트용 — start/drop 만 검증).
    fn empty_slot() -> Arc<ManagerSlot> {
        Arc::new(ManagerSlot::new())
    }

    /// 빈 messaging 슬롯(위와 동일 — start/drop 테스트용, send 미호출).
    fn empty_messaging_slot() -> Arc<MessagingSlot> {
        Arc::new(MessagingSlot::new())
    }

    /// 빈 명령 표 슬롯 — 이 파일의 테스트는 제어 동사를 부르지 않는다(부르면 그 라우트는 503 이다).
    fn empty_commands_slot() -> Arc<CommandTableSlot> {
        Arc::new(CommandTableSlot::new())
    }

    #[test]
    fn send_args_schema_builds() {
        let schema = schemars::schema_for!(SendArgs);
        let s = serde_json::to_string(&schema).expect("serialize schema");
        assert!(
            s.contains("\"to\"") && s.contains("\"body\""),
            "스키마에 to/body: {s}"
        );
        assert!(
            !s.contains("\"from\""),
            "send_message 스키마에 from 필드가 없어야: {s}"
        );
    }

    #[test]
    fn to_field_tokenizes_per_entrance_without_double_splitting() {
        let owned = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<String>>();
        // ── MCP 입구 ──
        assert_eq!(
            ToField::One("a,b".to_string()).into_tokens(),
            owned(&["a,b"])
        );
        assert_eq!(
            ToField::Many(owned(&["a,b", "@all"])).into_tokens(),
            owned(&["a,b", "@all"])
        );
        // ── CLI 입구 ──
        assert_eq!(
            cli_recipient_tokens(ToField::One("a,b".to_string())),
            owned(&["a", "b"])
        );
        assert_eq!(
            cli_recipient_tokens(ToField::Many(owned(&["a,b", "@all"]))),
            owned(&["a", "b", "@all"])
        );
        assert_ne!(
            ToField::One("a,b".to_string()).into_tokens(),
            cli_recipient_tokens(ToField::One("a,b".to_string()))
        );
    }

    #[test]
    fn to_field_deserializes_both_string_and_array_shapes() {
        let one: ToField = serde_json::from_str(r#""bob""#).expect("string 형태");
        assert_eq!(one.into_tokens(), vec!["bob".to_string()]);
        let many: ToField = serde_json::from_str(r#"["bob","@all"]"#).expect("array 형태");
        assert_eq!(
            many.into_tokens(),
            vec!["bob".to_string(), "@all".to_string()]
        );
    }

    #[test]
    fn tools_list_exposes_send_message_tool() {
        let router = EngramMcpHandler::tool_router();
        assert!(
            router.has_route(SEND_MESSAGE_TOOL),
            "라우터에 '{SEND_MESSAGE_TOOL}' 툴이 등록돼 있어야(const ↔ #[tool] 메서드명 일치 강제)"
        );
    }

    #[test]
    fn tools_list_exposes_the_query_tools() {
        // const ↔ #[tool] 메서드명이 어긋나면 프라이밍이 가르치는 툴 이름과 실제 노출 이름이 갈려 조회
        //   입구가 조용히 없는 것이 된다.
        let router = EngramMcpHandler::tool_router();
        assert!(router.has_route(MESSAGES_TOOL), "'{MESSAGES_TOOL}' 툴 등록");
    }

    #[test]
    fn query_tool_schemas_build_and_carry_no_identity_field() {
        let m =
            serde_json::to_string(&schemars::schema_for!(MessagesArgs)).expect("messages schema");
        assert!(m.contains("\"id\""), "messages 스키마에 id: {m}");
        assert!(
            !m.contains("\"from\"") && !m.contains("required"),
            "messages 인자는 전부 선택 + from 없음: {m}"
        );
    }

    #[tokio::test]
    async fn server_starts_and_reports_local_url() {
        let reg = Arc::new(ControlRegistry::new());
        // ★수거기를 들고 있어야 한다★ — 떨어뜨리면 그 자리에서 자리 표가 닫혀 그 뒤 왕복이 전부 반려된다.
        let (relay_bus, _relay_sweeper) = crate::command_delivery::CommandBus::without_commands();
        let handle = start_mcp_server(
            reg,
            empty_slot(),
            empty_messaging_slot(),
            empty_commands_slot(),
            relay_bus,
        )
        .await
        .expect("start mcp server");
        assert!(
            handle.url.starts_with("http://127.0.0.1:") && handle.url.ends_with("/mcp"),
            "로컬 엔드포인트 URL: {}",
            handle.url
        );
        handle.shutdown().await;
    }

    // ── round-2 F5 ──────────────────────────────────────────────────────────────
    #[tokio::test]
    async fn dropping_handle_cancels_serve_task() {
        let reg = Arc::new(ControlRegistry::new());
        // ★수거기를 들고 있어야 한다★ — 떨어뜨리면 그 자리에서 자리 표가 닫혀 그 뒤 왕복이 전부 반려된다.
        let (relay_bus, _relay_sweeper) = crate::command_delivery::CommandBus::without_commands();
        let handle = start_mcp_server(
            reg,
            empty_slot(),
            empty_messaging_slot(),
            empty_commands_slot(),
            relay_bus,
        )
        .await
        .expect("start mcp server");
        let watch = handle.cancel.clone();
        assert!(!watch.is_cancelled(), "start 직후엔 cancel 안 됨");
        drop(handle);
        assert!(
            watch.is_cancelled(),
            "핸들 drop 시 cancel 토큰이 발화돼 detached serve 태스크가 종료돼야(F5)"
        );
    }
}
