//! 데몬 MCP Streamable HTTP 서버(ADR-0086 스텝 1) — 인증 미들웨어 + `engram_ping` 진단 툴.
//!
//! ★역할★: 스폰된 claude 에이전트가 mcp-config 로 붙는 제어 채널 입구. rmcp `StreamableHttpService`
//!   (Tower service)를 axum `/mcp` 라우트에 nest 하고, 그 앞에 **bearer auth 미들웨어**를 얹는다 —
//!   토큰이 없거나(no header)·모르거나(unknown)·회전됨(stale-epoch)이면 **MCP handshake 전에 401**.
//!   유효하면 검증된 신원(BoundIdentity)을 요청 extensions 에 심어, rmcp 가 그걸 `http::request::Parts`
//!   로 툴 컨텍스트에 흘려준다(공식 "custom extension state" 패턴 — tower.rs docstring).
//!
//! ★OAuth 메타데이터 미광고(load-bearing, #59467)★: StreamableHttpService 는 `.well-known/*` 라우트를
//!   만들지 않고, 우리도 추가하지 않는다. claude 는 서버가 OAuth 메타데이터를 광고하면 정적 Authorization
//!   헤더를 무시하는데(claude-code #59467), 광고 라우트가 없으니 정적 Bearer 가 그대로 실린다(ADR-0086 §근거).
//!
//! ★스텝 1 범위★: `engram_ping`(진단) 툴 하나만 노출한다 — 연결된 에이전트의 `system:init` 에 우리
//!   서버·툴이 뜨는지 + 세션 바인딩이 end-to-end 로 통하는지 증명용. send_message·Validator·Mailbox 는
//!   스텝 2~4(여기서 구현하지 않는다).
//!
//! tauri import 0(daemon crate).

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::State;
use axum::middleware::Next;
use axum::response::Response;
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

use super::registry::{BoundIdentity, ControlRegistry};

/// MCP 서버가 붙는 axum 경로. mcp-config url 도 이 경로를 가리킨다(`http://127.0.0.1:<port>/mcp`).
const MCP_PATH: &str = "/mcp";

/// claude 가 Authorization 헤더로 실어 보내는 세션 식별 헤더명(rmcp/스펙 표준, 소문자 비교).
const SESSION_ID_HEADER: &str = "mcp-session-id";

/// 실행 중 MCP 서버 핸들 — 에이전트가 붙을 엔드포인트 URL + graceful 종료 토큰.
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
    /// 서버를 graceful 하게 내린다(cancel → serve loop 종료 대기).
    pub async fn shutdown(mut self) {
        self.cancel.cancel();
        if let Some(h) = self.serve_handle.take() {
            let _ = h.await;
        }
        // self 가 여기서 drop → Drop::drop 의 cancel 은 멱등 no-op(이미 cancel 됨).
    }
}

impl Drop for McpServerHandle {
    /// ★drop-on-error airtight(round-2 F5)★: 핸들이 `shutdown().await` 없이 그냥 drop 되면(예: MCP
    ///   서버 start 뒤 daemon.json write 같은 **후속** startup 단계가 실패해 에러 반환으로 이 핸들이
    ///   drop 되는 경우) detached serve 태스크가 취소 신호를 못 받고 계속 돌 수 있다. Drop 에서 cancel
    ///   토큰을 발화해, 어느 경로로 drop 되든 serve 태스크(graceful_shutdown 이 cancel 을 관측)가
    ///   확실히 종료되게 한다. 프로세스 종료가 대개 이를 무의미하게 만들지만, in-process 테스트나 부분
    ///   실패 경로에서 태스크 누수를 막아 airtight 하게 만든다. (정상 종료는 shutdown() 이 cancel+await
    ///   를 이미 수행하므로 이 Drop 의 cancel 은 idempotent no-op — CancellationToken.cancel 은 멱등.
    ///   Drop 은 async await 를 못 하므로 join 없이 cancel 신호만 발화한다 — 태스크는 스스로 종료한다.)
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// `engram_ping` 진단 툴을 노출하는 MCP 서버 핸들러.
///
/// ★registry 필드 없음(FIX 12)★: 신원은 auth 미들웨어가 검증해 요청 extensions 에 심고(BoundIdentity),
///   세션↔신원 바인딩·pinning·정리도 전부 미들웨어(State 로 registry 접근)가 한다 — 핸들러는 extensions
///   에서 신원을 읽기만 하므로 registry 를 들 필요가 없다. 예전엔 registry 필드를 두고 `let _ =
///   &self.registry;` 로 dead_code 를 눌렀는데, 실제 쓰임이 없어 필드·인자를 제거했다.
#[derive(Clone)]
pub struct EngramMcpHandler {
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl EngramMcpHandler {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    /// 진단 툴 — "pong" + 바인딩된 신원(AgentId, epoch)을 돌려준다. 연결된 에이전트가 이 툴을 호출하면
    /// 세션 바인딩이 end-to-end 로 통함이 증명된다(스텝 1 acceptance). send_message 는 스텝 2.
    ///
    /// ★신원 출처 = 토큰(ADR-0086)★: 신원은 요청 페이로드가 아니라 auth 미들웨어가 검증해 extensions 에
    ///   심은 BoundIdentity 다(사칭 차단). `RequestContext` → `http::request::Parts` → `parts.extensions`
    ///   순으로 꺼낸다(rmcp 공식 custom-extension 패턴). 없으면(정상적으로는 미들웨어가 401 로 막아 도달
    ///   불가) tool-level 에러.
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
                // 신원은 미들웨어가 검증해 extensions 에 심은 값이다(사칭 차단) — 여기선 그대로 되돌린다.
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
}

impl Default for EngramMcpHandler {
    fn default() -> Self {
        Self::new()
    }
}

/// 툴 인자 — ping 은 인자가 없다(빈 struct). schemars(rmcp 재수출)로 input schema 자동 생성.
#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct PingArgs {}

// router = self.tool_router — 저장한 필드를 실제로 읽게 해 dead_code 를 피하고, 핸들러마다 라우터를
// 재빌드하지 않는다(factory 가 세션마다 new() 하므로 라우터를 필드에 한 번 만들어 두는 게 효율적).
#[tool_handler(router = self.tool_router)]
impl ServerHandler for EngramMcpHandler {
    fn get_info(&self) -> ServerInfo {
        // ServerInfo(=InitializeResult)는 #[non_exhaustive] 라 struct 리터럴 불가 → ctor 체인 사용.
        // tools capability 만 켠다(스텝 1 = 진단 툴 하나). OAuth/resources/prompts 미광고(#59467 회피).
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Engram daemon control channel (ADR-0086 step 1). Only engram_ping is available.",
        )
    }
}

/// bearer auth 미들웨어(ADR-0086) — MCP handshake **전에** 토큰을 검증하고 세션↔신원을 고정한다.
///
/// 흐름:
///   1. Authorization 에서 `Bearer <token>` 추출 → registry.validate. 실패(없음/모름/stale-epoch)면
///      즉시 401(inner 미호출 = handshake 미생성).
///   1.5. ★세션 id 헤더 형식/필수성 검사(400, ADR-0086)★: Mcp-Session-Id 헤더가 **있으나 malformed**
///      (비-UTF-8 등 to_str() 실패)이면 400 — None 으로 접어 sessionless 로 오인시키면 경계를 우회한다.
///      또 **GET/DELETE 는 세션 operation** 이라 세션 id 가 반드시 있어야 한다 — 없으면 400(POST 무-세션id
///      는 예외 = initialize). 이 검사가 아래 바인딩 검사가 "session op 는 반드시 바인딩으로 resolve 된다"를
///      보장한다(rmcp 내부 4xx 동작에 의존하지 않음).
///   2. ★세션 바인딩 검사(FIX 7 + round-2 F1)★: 요청이 **기존 Mcp-Session-Id 를 실어 오면**, 데몬
///      레지스트리에 그 세션 바인딩이 있어야 한다. 바인딩 있고 신원 일치=통과 / 바인딩 있으나 신원
///      불일치=**403**(cross-token takeover — 세션 S 를 토큰 A 로 열고 토큰 B 로 S 에 요청) / 바인딩
///      **없음**=**404**(orphaned/unknown — revoke 로 바인딩만 prune 됐으나 rmcp 세션이 살아 있는 고아
///      세션에 다른 유효 토큰이 attach 하는 탈취를 차단). initialize 는 아직 세션 id 가 없어 이 검사를
///      건너뛴다(세션은 응답에서 생성). DELETE 면 신원 확인 후 세션 바인딩을 prune 한다(FIX 8/F6).
///   3. 검증된 신원을 요청 extensions 에 심어 inner(StreamableHttpService)로 넘긴다 → rmcp 가 Parts 로
///      툴에 흘린다.
///   4. 응답에 새 Mcp-Session-Id 가 있으면(initialize 성공) `bind_session_if_absent` 로 신원을 세션에
///      **한 번만** 고정한다(no-overwrite + validate→bind revoke 재확인 — FIX 7). 실패(중복/죽음)는
///      바인딩 생략(중복은 무해, 죽음은 다음 요청에서 401/403 로 걸린다).
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
    // Authorization: Bearer <token> 추출. 없거나 형식 위반 → 401.
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
    // 모름/stale-epoch → validate None → 401. (회전된 구 epoch 토큰은 registry 에서 이미 제거됨.)
    let Some(identity) = registry.validate(&token) else {
        return unauthorized();
    };

    // ★요청이 실어 온 기존 세션 id(있으면)★ — initialize 이후의 후속 요청(tools/call·GET·DELETE)은
    //   Mcp-Session-Id 를 헤더로 싣는다. 이 값으로 identity pinning 을 검사한다(초기 initialize 는 없음).
    // ★malformed ≠ absent(Codex LOW)★: 헤더가 **있으나** to_str() 이 실패하면(비-UTF-8 등) 이걸 None 으로
    //   접으면 안 된다 — None 으로 접으면 세션-실은 요청이 "sessionless" 로 오인돼 아래 바인딩 검사를 건너뛰고
    //   inner(rmcp)로 통과한다(경계 우회). present-but-malformed 는 클라이언트 오류이므로 바인딩 검사에
    //   닿기 전에 400 으로 끊는다(신원·인증 문제는 아니므로 401/403 이 아니라 400, body 는 비움). 진짜로
    //   **부재**한 헤더만 "sessionless" 로 취급한다(initialize 경로).
    let method = request.method().clone();
    let req_session_id = match request.headers().get(SESSION_ID_HEADER) {
        None => None, // 진짜 부재 = sessionless(initialize 후보).
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
        // 세션을 실어 온 요청은 데몬 레지스트리에 바인딩이 **있어야** 한다. 두 갈래로 처리한다:
        match registry.identity_for_session(sid) {
            // (a) 바인딩 존재 + 신원 일치 → 정상 진행(아래 DELETE prune / next.run).
            Some(bound) if bound == identity => {}
            // (b) 바인딩 존재하나 신원 불일치 = cross-token takeover(FIX 7) → 403.
            Some(_) => {
                tracing::warn!(
                    session = %sid,
                    "제어 채널 cross-token 세션 탈취 거부(403, ADR-0086 FIX 7)"
                );
                return forbidden();
            }
            // (c) ★orphaned-session 거부(round-2 F1)★: 세션 id 를 실어 왔는데 데몬 바인딩이 **없다**.
            //   예전엔 이걸 inner(rmcp)로 통과시켜 rmcp 가 404 를 내게 했는데, 그 경로엔 치명적 창이 있다:
            //   에이전트 A 가 세션 S 를 열어 바인딩됐다가 revoke(kill)로 **바인딩만** prune 되면 rmcp 측
            //   세션 S 는 아직 살아 있을 수 있다. 그때 유효 토큰을 든 에이전트 B 가 S 를 제시하면 미들웨어가
            //   그대로 통과시켜 B 가 A 의 고아 세션 워커에 attach 된다(세션 탈취). 이제 **바인딩 없는
            //   세션-실은 요청은 전부 거부**해 그 창을 닫는다 — rmcp 측에 살아 있으나 데몬이 모르는 세션은
            //   도달 불가(unreachable orphan)가 된다. 이는 DELETE-prune 순서도 fail-safe 로 만든다.
            //   ★404 선택 이유★: "이 세션은 (데몬 인가 관점에서) 존재하지 않는다" 가 정확한 의미이고,
            //   truly-unknown id 는 예전에도 rmcp 404 를 받았으므로 정상 클라이언트가 보는 상태코드가
            //   바뀌지 않는다(happy-path 무영향). 토큰 자체는 유효하므로 401 은 부적절, 다른 신원 소유가
            //   확정된 것도 아니므로(존재 자체가 없음) 403 보다 404 가 정직하다. 응답 body 는 비워 누출 0.
            None => {
                tracing::warn!(
                    session = %sid,
                    "제어 채널 orphaned/unknown 세션 거부(404, ADR-0086 F1)"
                );
                return not_found();
            }
        }
        // 여기 도달 = 바인딩 존재 + 신원 일치. DELETE = 클라이언트가 세션을 접음 → 바인딩 prune.
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

    // 검증된 신원을 extensions 에 심어 inner 로. body 타입을 axum Body 로 정규화(Into 바운드).
    let mut request: Request<axum::body::Body> = request.into();
    request.extensions_mut().insert(identity);

    // handshake 는 inner(StreamableHttpService)가 수행. 여기까지 왔다는 건 토큰 유효 + (세션 있으면)
    //   신원 일치 확정.
    let response = next.run(request).await;

    // 세션 바인딩(ADR-0086): initialize 응답의 Mcp-Session-Id 를 신원과 **한 번만** 묶는다. no-overwrite
    //   + exact-token recheck 는 bind_session_if_absent 가 담당(FIX 7 + round-2 F2) — 여기선 응답 헤더에서
    //   새 세션 id 만 뽑고, **검증에 쓴 그 토큰 문자열**을 함께 넘긴다. bind 는 그 토큰이 아직 이 agent 의
    //   현재 크레덴셜인지 국소 비교해, validate→bind 창의 revoke/재발급을 걸러낸다. 후속 tools/call 은 위
    //   pin 검사를 거친다. 이 바인딩은 acceptance 관측점 + revoke 정리 대상.
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

/// 400 응답(빈 body) — 클라이언트 요청 형식 오류(ADR-0086). malformed(비-UTF-8) Mcp-Session-Id 헤더, 또는
/// 세션 id 없는 GET/DELETE(세션 operation 은 세션을 지목해야). 신원·인증 문제가 아니라 요청 형식 문제라
/// 401/403/404 가 아니라 400. body 는 비워 어떤 정보도 누출하지 않는다.
fn bad_request() -> Response {
    Response::builder()
        .status(StatusCode::BAD_REQUEST)
        .body(axum::body::Body::empty())
        .expect("valid 400 response")
}

/// 데몬 MCP 서버를 127.0.0.1 ephemeral 포트에 띄운다(WS 서버와 나란히). 반환: 엔드포인트 URL·종료 토큰
/// 을 담은 핸들. registry 는 auth 미들웨어(검증)와 provision(발급)이 공유하는 동일 Arc 다.
///
/// ★로컬 전용 + DNS rebinding 방어★: bind 는 127.0.0.1:0(OS 할당 포트). StreamableHttpServerConfig 는
///   기본 allowed_hosts=[localhost,127.0.0.1,::1] 로 로컬 Host 만 허용(rmcp 기본). stateful_mode=true(기본)
///   라 세션이 Mcp-Session-Id 로 유지된다.
pub async fn start_mcp_server(registry: Arc<ControlRegistry>) -> std::io::Result<McpServerHandle> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr: SocketAddr = listener.local_addr()?;
    let url = format!("http://127.0.0.1:{}{}", addr.port(), MCP_PATH);

    let cancel = CancellationToken::new();

    // rmcp Streamable HTTP service — service_factory 는 요청마다(세션마다) 핸들러를 만든다. 핸들러는
    //   이제 상태가 없다(FIX 12 — registry 는 미들웨어가 State 로 쥔다). registry 는 auth 미들웨어와
    //   provision 이 공유한다(아래 layer + DaemonControlChannel).
    // StreamableHttpServerConfig 는 #[non_exhaustive] 라 struct 리터럴 불가 → Default + builder 메서드.
    //   종료 토큰만 연동(cancel 시 활성 세션 정리). 나머지는 rmcp 기본(stateful_mode=true, allowed_hosts=
    //   로컬만 — DNS rebinding 방어, OAuth 미광고).
    let config =
        StreamableHttpServerConfig::default().with_cancellation_token(cancel.child_token());
    let mcp_service = StreamableHttpService::new(
        || Ok(EngramMcpHandler::new()),
        Arc::new(LocalSessionManager::default()),
        config,
    );

    // /mcp 라우트에 MCP service 를 nest + 그 앞에 bearer auth 미들웨어. auth 미들웨어가 State 로 registry
    //   를 받아 검증한다. ★nest_service★: StreamableHttpService 는 Tower service 라 axum 라우터에 그대로 얹힌다.
    // ★body 상한 = RequestBodyLimitLayer(round-2 F4)★: 로컬 제어 채널의 요청 바디(JSON-RPC)는 작다 —
    //   악성/폭주 바디로 메모리를 삼키지 않게 상한을 명시한다. axum `DefaultBodyLimit` 는 **extractor**
    //   (Json/Bytes 등)에만 걸리는데 rmcp `StreamableHttpService` 는 raw body 를 직접 소비하므로(extractor
    //   미경유) 그 상한이 통하지 않는다. `RequestBodyLimitLayer` 는 body 자체를 감싸 하위 소비자 전부
    //   (rmcp 포함)에 상한을 강제하고, 초과 시 413(Payload Too Large)로 끊는다. 1MB 면 initialize/
    //   tools/call 같은 스텝 1 페이로드에 충분하다(send_message 대용량은 스텝 2 설계 시 재검토).
    // ★레이어 순서★: 아래는 바깥→안 순서로 body-limit → auth → nest 로 쌓인다(axum layer 는 나중에 쓴 게
    //   바깥). body-limit 를 가장 바깥에 둬 auth·inner 어느 쪽이 body 를 읽든 그 전에 상한이 적용되게 한다.
    const MAX_BODY_BYTES: usize = 1024 * 1024;
    let app = axum::Router::new()
        .nest_service(MCP_PATH, mcp_service)
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
        // graceful shutdown = cancel 토큰 관측.
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

    #[test]
    fn ping_args_schema_builds() {
        // schemars 가 빈 인자 스키마를 만들 수 있어야(tool 매크로가 컴파일되는지 간접 확인).
        let schema = schemars::schema_for!(PingArgs);
        let _ = serde_json::to_string(&schema).expect("serialize schema");
    }

    #[tokio::test]
    async fn server_starts_and_reports_local_url() {
        let reg = Arc::new(ControlRegistry::new());
        let handle = start_mcp_server(reg).await.expect("start mcp server");
        assert!(
            handle.url.starts_with("http://127.0.0.1:") && handle.url.ends_with("/mcp"),
            "로컬 엔드포인트 URL: {}",
            handle.url
        );
        handle.shutdown().await;
    }

    // ── round-2 F5: 핸들 drop(shutdown 미호출)이 serve 태스크를 취소한다 ──────────────────────
    #[tokio::test]
    async fn dropping_handle_cancels_serve_task() {
        let reg = Arc::new(ControlRegistry::new());
        let handle = start_mcp_server(reg).await.expect("start mcp server");
        // 핸들에서 serve JoinHandle 을 관측용으로 미리 복제할 수는 없으므로(1개뿐), cancel 토큰을
        //   복제해 drop 후 cancel 이 발화됐는지 본다. shutdown() 대신 그냥 drop(후속 startup 실패 모사).
        let watch = handle.cancel.clone();
        assert!(!watch.is_cancelled(), "start 직후엔 cancel 안 됨");
        drop(handle); // ★shutdown().await 없이 drop★ — Drop 이 cancel 을 발화해야(F5).
        assert!(
            watch.is_cancelled(),
            "핸들 drop 시 cancel 토큰이 발화돼 detached serve 태스크가 종료돼야(F5)"
        );
    }
}
