//! headless CLI 모드 — 릴리즈 exe(`engram-dashboard.exe`)를 데몬 제어 CLI로 겸용한다.
//!
//! ## 왜 (설계 §5 LLM-우선 제어 · ADR-0014 방향)
//! 스폰된 에이전트(claude)는 Bash 툴을 갖지만 WebView2 CDP·`window.__TAURI__` 는 release exe 에서
//! 죽는다(dev 전용). 반면 **데몬 WS 는 빌드 무관으로 산다**. 그래서 에이전트가 다른 에이전트에게
//! 메시지를 보내거나(A→B) 새 에이전트를 스폰하려면 이 CLI 통로가 필요하다 — node 스크립트도,
//! 하드코딩 경로도 없이 자기 자신(exe)을 재실행해 데몬에 붙는다.
//!
//! 이 모듈은 `scripts/engram.mjs`(throwaway node 스파이크)의 Rust 이식이다 — daemon.json 발견 →
//! Auth 핸드셰이크 → AgentCommand JSON 송신 → reply 매칭. 스파이크는 롤백 대상이고 이 exe 가 자립한다.
//!
//! ## GUI 경로 불변(load-bearing)
//! `main.rs` 가 argv 첫 인자를 보고 **알려진 CLI verb 일 때만** 이 모듈로 분기한다. 그 외(인자 없음·
//! `--hidden` autostart 등)는 기존 Tauri/GUI 기동을 **그대로** 탄다 — single-instance 플러그인·창·
//! 트레이 어느 것도 CLI 경로에선 건드리지 않는다(one-shot: 붙어서 명령 1건 처리 후 exit).
//!
//! ## 이 CLI 는 spawn 하지 않는다(ADR-0021 대칭)
//! daemon.json 을 **읽기만** 한다(`read_live_daemon`) — 데몬이 없으면 명확히 에러로 빠진다("no daemon
//! running / is the app open?"). CLI 가 데몬을 깨우지 않는 이유: CLI 는 이미 떠 있는 앱/데몬을 조종하는
//! 보조 통로지, 부팅 주체가 아니다(부팅은 앱/트레이/discovery::ensure 의 몫).

use std::time::Duration;

use engram_dashboard_protocol::{
    AgentCommand, AgentEvent, AgentInfo, DaemonInfo, RequestId, PROTOCOL_VERSION,
};

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

/// 알려진 CLI verb 인가 — main.rs 가 GUI/CLI 분기 판정에 쓴다. 여기 없는 첫 인자(예: `--hidden`)는
/// GUI 경로로 넘어간다(CLI 가 아님). ★이 목록이 GUI 인자와 겹치지 않아야 한다★ — 겹치면 GUI 인자를
/// CLI 로 오인해 창이 안 뜬다. 현재 GUI 인자는 `--hidden` 뿐이라 충돌 없음.
pub fn is_cli_verb(arg: &str) -> bool {
    matches!(arg, "list" | "send" | "spawn" | "kill")
}

/// CLI 진입점. argv(프로그램명 제외)를 받아 one-shot 으로 처리하고 프로세스 exit code 를 반환한다.
/// main.rs 가 이 반환값으로 `std::process::exit` 한다(0=성공, 비0=실패).
///
/// ★전용 최소 tokio 런타임★: GUI 의 DaemonClient(재연결·멀티뷰·상태 watch)를 재사용하지 않고, 여기서
/// current-thread 런타임을 새로 띄워 connect→Auth→Hello→send→reply→print 만 한다. one-shot 이라
/// 재연결·상태머신이 불필요하고, GUI 지향 클라이언트를 끌어오면 배선(router/registry/app handle)이
/// 딸려와 과하다. 핸드셰이크 로직만 connection.rs 와 동형으로 손으로 재현한다(Auth 첫 프레임 규약 동일).
pub fn run_cli(args: &[String]) -> i32 {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: tokio 런타임 생성 실패: {e}");
            return 1;
        }
    };
    match rt.block_on(dispatch(args)) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// verb 라우팅. 첫 인자 = verb, 나머지 = 인자. 알 수 없는 verb 는 usage 로 실패.
async fn dispatch(args: &[String]) -> Result<(), String> {
    let (verb, rest) = args.split_first().ok_or_else(usage)?;
    match verb.as_str() {
        "list" => cmd_list().await,
        "send" => cmd_send(rest).await,
        "spawn" => cmd_spawn(rest).await,
        "kill" => cmd_kill(rest).await,
        _ => Err(usage()),
    }
}

fn usage() -> String {
    "usage: engram-dashboard <list | send <name|id> <text...> | spawn <cwd> | kill <name|id>>"
        .to_string()
}

// ── verbs ────────────────────────────────────────────────────────────────────────

/// `list` — 에이전트 1줄/개: `<id>\t<label>\t<status>\t<cwd>`. label=트리 표시명(profile join).
async fn cmd_list() -> Result<(), String> {
    let mut conn = Connection::open().await?;
    let agents = fetch_agents(&mut conn).await?;
    if agents.is_empty() {
        println!("(no agents)");
    }
    for a in &agents {
        // status 는 enum(Running/Exited{code}/…)이라 문자열이 아니면 JSON 으로 압축 출력.
        println!("{}\t{}\t{}\t{}", a.id, a.label, a.status, a.cwd);
    }
    Ok(())
}

/// `send <name|id> <text...>` — 대상 에이전트 stdin 에 텍스트 주입(A→B 메시지). 이름은 트리 표시명으로도
/// 지목 가능(profile join). 텍스트 끝에 `\r`(PTY Enter)을 붙여 상대가 실제로 입력을 제출하게 한다.
async fn cmd_send(rest: &[String]) -> Result<(), String> {
    let target = rest
        .first()
        .ok_or("usage: engram-dashboard send <name|id> <text...>")?;
    let text = rest.get(1..).unwrap_or(&[]).join(" ");
    if text.is_empty() {
        return Err("usage: engram-dashboard send <name|id> <text...>".to_string());
    }
    let mut conn = Connection::open().await?;
    let agents = fetch_agents(&mut conn).await?;
    let agent = resolve_agent(&agents, target)?;

    // ★\r = PTY Enter(제출)★ — 이게 있어야 상대 에이전트가 입력을 실제로 받는다(engram.mjs 와 동일).
    // data 는 wire 에서 serde_bytes(Vec<u8>) → JSON 숫자배열로 직렬화된다(WriteStdin.data).
    let data = format!("{text}\r").into_bytes();
    let request_id = RequestId::new();
    conn.send(&AgentCommand::WriteStdin {
        agent_id: agent.id,
        data,
        request_id,
    })
    .await?;
    // Ack/Error 를 잠깐 기다리되(3s), 안 와도 성공으로 본다 — WriteStdin ack 보장은 미확정(engram.mjs 동일).
    let _ = conn.wait_reply(request_id, Duration::from_secs(3)).await;
    println!("sent -> {} ({})", agent.label, agent.id);
    Ok(())
}

/// `spawn <cwd>` — SpawnByCwd(ad-hoc 셸 에이전트). 새 agent id 를 출력한다.
async fn cmd_spawn(rest: &[String]) -> Result<(), String> {
    let cwd = rest.first().ok_or("usage: engram-dashboard spawn <cwd>")?;
    let mut conn = Connection::open().await?;
    let request_id = RequestId::new();
    conn.send(&AgentCommand::SpawnByCwd {
        cwd: cwd.clone(),
        request_id,
    })
    .await?;
    match conn.wait_reply(request_id, Duration::from_secs(10)).await? {
        AgentEvent::Spawned { agent, .. } => {
            println!("{}", agent.id);
            Ok(())
        }
        AgentEvent::Error { message, .. } => Err(format!("spawn failed: {message}")),
        other => Err(format!("unexpected reply to spawn: {other:?}")),
    }
}

/// `kill <name|id>` — 대상 에이전트 종료(자원 강제 폐쇄).
async fn cmd_kill(rest: &[String]) -> Result<(), String> {
    let target = rest
        .first()
        .ok_or("usage: engram-dashboard kill <name|id>")?;
    let mut conn = Connection::open().await?;
    let agents = fetch_agents(&mut conn).await?;
    let agent = resolve_agent(&agents, target)?;
    let request_id = RequestId::new();
    conn.send(&AgentCommand::Kill {
        agent_id: agent.id,
        request_id,
    })
    .await?;
    match conn.wait_reply(request_id, Duration::from_secs(5)).await? {
        AgentEvent::Ack { .. } => {
            println!("killed {} ({})", agent.label, agent.id);
            Ok(())
        }
        AgentEvent::Error { message, .. } => Err(format!("kill failed: {message}")),
        other => Err(format!("unexpected reply to kill: {other:?}")),
    }
}

// ── name resolution (engram.mjs resolveAgent 미러) ────────────────────────────────

/// 에이전트 + 트리 표시명(label)을 합친 뷰. label = canonical AgentInfo.name(ADR-0101 이후 데몬이
/// display_name ?? basename(session.cwd)로 파생 — 트리·라우팅과 동일 문자열).
struct ResolvedAgent {
    id: uuid::Uuid,
    label: String,
    cwd: String,
    /// status 를 문자열로 평탄화(Running/Exited{code}/… — 출력·비교용).
    status: String,
}

/// ListAgents 를 조회해 canonical label 을 만든다.
/// ★ADR-0101 (WYSIWYA): label = canonical AgentInfo.name★ — 데몬이 이미 display_name ?? basename(
///   session.cwd)로 파생한 값이라, 이게 트리·라우팅(resolve_recipient)이 쓰는 문자열과 동일하다.
///   예전엔 profile.name(종종 full-path 라벨)을 우선해 display_name 미설정 시 옛 경로 문자열을
///   노출·지목해 트리와 어긋났다 — profile.name 우선을 제거하고 canonical name 을 정본으로 쓴다.
///   (name 이 빈 값일 때만 id 앞 8자로 degrade — 방어용.)
async fn fetch_agents(conn: &mut Connection) -> Result<Vec<ResolvedAgent>, String> {
    let agents = list_agents(conn).await?;
    let out = agents
        .into_iter()
        .map(|a| {
            let label = if a.name.is_empty() {
                a.id.to_string().chars().take(8).collect()
            } else {
                a.name.clone()
            };
            let status = status_str(&a);
            ResolvedAgent {
                id: a.id,
                label,
                cwd: a.cwd,
                status,
            }
        })
        .collect();
    Ok(out)
}

/// status enum → 짧은 문자열. terminal 변형의 부가정보(code/message)는 괄호로 덧붙인다.
fn status_str(a: &AgentInfo) -> String {
    use engram_dashboard_protocol::AgentStatus as S;
    match &a.status {
        S::Running => "Running".to_string(),
        S::Exiting => "Exiting".to_string(),
        S::Exited { code } => match code {
            Some(c) => format!("Exited({c})"),
            None => "Exited".to_string(),
        },
        S::Failed { message } => format!("Failed({message})"),
        S::Killed => "Killed".to_string(),
    }
}

/// 표시명(label) / 전체 id / id 접두사로 에이전트 1명 지목. 모호하면 Err(engram.mjs resolveAgent 미러).
/// 매칭 순서: 정확한 id → 정확한 label(대소문자 무시) → 유일한 id 접두사.
fn resolve_agent<'a>(list: &'a [ResolvedAgent], needle: &str) -> Result<&'a ResolvedAgent, String> {
    if let Some(a) = list.iter().find(|a| a.id.to_string() == needle) {
        return Ok(a);
    }
    let by_label: Vec<&ResolvedAgent> = list
        .iter()
        .filter(|a| a.label.eq_ignore_ascii_case(needle))
        .collect();
    match by_label.len() {
        1 => return Ok(by_label[0]),
        n if n > 1 => {
            return Err(format!(
                "name ambiguous \"{needle}\" — {n} matches. id 로 지목하세요."
            ))
        }
        _ => {}
    }
    let by_prefix: Vec<&ResolvedAgent> = list
        .iter()
        .filter(|a| a.id.to_string().starts_with(needle))
        .collect();
    match by_prefix.len() {
        1 => Ok(by_prefix[0]),
        n if n > 1 => Err(format!("id prefix ambiguous \"{needle}\" — {n} matches.")),
        _ => Err(format!("agent not found: \"{needle}\"")),
    }
}

async fn list_agents(conn: &mut Connection) -> Result<Vec<AgentInfo>, String> {
    let request_id = RequestId::new();
    conn.send(&AgentCommand::ListAgents { request_id }).await?;
    match conn.wait_reply(request_id, Duration::from_secs(5)).await? {
        AgentEvent::AgentList { agents, .. } => Ok(agents),
        other => Err(format!("unexpected reply to ListAgents: {other:?}")),
    }
}

// ── one-shot WS connection (connect → Auth → Hello) ───────────────────────────────

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// one-shot 데몬 WS 연결. connection.rs 의 핸드셰이크(Auth 첫 프레임 → Hello)를 최소로 재현한다.
/// 재연결·상태머신 없음(one-shot). Text(제어 JSON)만 처리하고 Binary(터미널 출력 바이트)는 무시한다.
struct Connection {
    ws: Ws,
}

impl Connection {
    /// daemon.json 발견(read-only, no-spawn) → ws 접속 → Auth 송신 → Hello 대기.
    async fn open() -> Result<Self, String> {
        // ★spawn 안 함(ADR-0021 대칭)★: read_live_daemon 은 살아있는 호환 데몬의 daemon.json 만 읽는다.
        //   없으면(파일 없음/죽음/버전 불일치) None → 명확한 에러("데몬 없음").
        let data_dir = engram_dashboard_discovery::default_data_dir();
        let info: DaemonInfo =
            engram_dashboard_discovery::read_live_daemon(&data_dir).ok_or_else(|| {
                "no daemon running — is the app open? (daemon.json 없음/죽음/버전 불일치)"
                    .to_string()
            })?;

        let url = format!("ws://{}:{}", info.host, info.port);
        let (mut ws, _resp) = connect_async(&url)
            .await
            .map_err(|e| format!("daemon websocket 접속 실패({url}): {e}"))?;

        // ★첫 프레임 = Auth(Text JSON)★ — 데몬은 1초 내 첫 프레임으로 이걸 기대한다(ws.rs AUTH_TIMEOUT).
        //   protocol_version 은 daemon.json echo 가 아니라 우리 컴파일 버전(connection.rs Fix C 와 동형).
        //   token 은 wire 로만 흐른다(로그/에러에 절대 노출 금지 — 보안).
        let auth = AgentCommand::Auth {
            token: info.token.clone(),
            protocol_version: PROTOCOL_VERSION,
        };
        let auth_text =
            serde_json::to_string(&auth).map_err(|e| format!("Auth 직렬화 실패: {e}"))?;
        ws.send(Message::Text(auth_text.into()))
            .await
            .map_err(|e| format!("Auth 전송 실패: {e}"))?;

        let mut conn = Connection { ws };
        // Hello(=인증 성공) 대기. Error(토큰/버전 불일치)면 실패. 5s 상한(loopback 정상은 <1s).
        match conn.next_control(Duration::from_secs(5)).await? {
            AgentEvent::Hello { .. } => Ok(conn),
            AgentEvent::Error { message, .. } => Err(format!("auth failed: {message}")),
            other => Err(format!("expected Hello, got: {other:?}")),
        }
    }

    /// AgentCommand 를 Text JSON 으로 송신.
    async fn send(&mut self, cmd: &AgentCommand) -> Result<(), String> {
        let text = serde_json::to_string(cmd).map_err(|e| format!("command 직렬화 실패: {e}"))?;
        self.ws
            .send(Message::Text(text.into()))
            .await
            .map_err(|e| format!("command 전송 실패: {e}"))
    }

    /// request_id 가 일치하는 reply(또는 request_id 없는 Error)를 timeout 내 대기한다.
    /// request_id 없는 broadcast(AgentListUpdated/StatusChanged/…)와 다른 request_id reply 는 건너뛴다.
    async fn wait_reply(
        &mut self,
        request_id: RequestId,
        timeout: Duration,
    ) -> Result<AgentEvent, String> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline
                .checked_duration_since(tokio::time::Instant::now())
                .ok_or("timeout waiting for reply")?;
            let ev = self.next_control(remaining).await?;
            if reply_matches(&ev, request_id) {
                return Ok(ev);
            }
            // 매칭 안 되는 제어 프레임(broadcast·다른 요청 reply)은 버리고 계속.
        }
    }

    /// 다음 **제어(Text JSON)** 프레임 1개를 timeout 내 수신해 AgentEvent 로 파싱한다.
    /// Binary(터미널 출력)·Ping/Pong 은 건너뛴다. 소켓 닫힘·타임아웃은 Err.
    async fn next_control(&mut self, timeout: Duration) -> Result<AgentEvent, String> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline
                .checked_duration_since(tokio::time::Instant::now())
                .ok_or("timeout waiting for reply")?;
            let msg = match tokio::time::timeout(remaining, self.ws.next()).await {
                Err(_) => return Err("timeout waiting for reply".to_string()),
                Ok(None) => return Err("daemon 연결이 닫힘(reply 전)".to_string()),
                Ok(Some(Err(e))) => return Err(format!("ws read 오류: {e}")),
                Ok(Some(Ok(m))) => m,
            };
            match msg {
                // 제어 프레임(Text JSON) — AgentEvent 로 파싱. 파싱 실패는 건너뛴다(방어).
                Message::Text(t) => {
                    if let Ok(ev) = serde_json::from_str::<AgentEvent>(&t) {
                        return Ok(ev);
                    }
                }
                // Binary = 터미널 출력 바이트(codec frame). 제어 CLI 는 무시.
                Message::Binary(_) => {}
                Message::Close(_) => return Err("daemon 연결이 닫힘(reply 전)".to_string()),
                // Ping/Pong/Frame 등 — 무시하고 계속.
                _ => {}
            }
        }
    }
}

/// 이벤트가 주어진 request_id 의 reply 인가. request_id 를 동봉하는 변형은 echo 를 비교하고,
/// request_id 없는 Error(전역 오류)도 "내 명령 실패"로 받아들인다(연결 단위 one-shot 이라 안전).
fn reply_matches(ev: &AgentEvent, request_id: RequestId) -> bool {
    match ev {
        AgentEvent::Ack { request_id: r } => *r == request_id,
        AgentEvent::AgentList { request_id: r, .. } => *r == request_id,
        AgentEvent::ProfileList { request_id: r, .. } => *r == request_id,
        AgentEvent::Spawned { request_id: r, .. } => *r == request_id,
        AgentEvent::Created { request_id: r, .. } => *r == request_id,
        AgentEvent::Snapshot { request_id: r, .. } => *r == request_id,
        AgentEvent::PresetList { request_id: r, .. } => *r == request_id,
        // Error 는 request_id 가 Option — 있으면 매칭, 없으면(전역) 이 one-shot 명령 실패로 수용.
        AgentEvent::Error { request_id: r, .. } => r.map(|r| r == request_id).unwrap_or(true),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_cli_verb_matches_known_and_rejects_gui_args() {
        for v in ["list", "send", "spawn", "kill"] {
            assert!(is_cli_verb(v), "{v} 는 CLI verb 여야");
        }
        // ★GUI 경로 보존★: GUI 인자·미지정은 CLI 가 아니어야(창이 떠야 한다).
        for v in ["--hidden", "", "gui", "--help", "listagents"] {
            assert!(!is_cli_verb(v), "{v} 는 CLI verb 가 아니어야(GUI 경로)");
        }
    }

    fn agent(id: uuid::Uuid, name: &str, cwd: &str) -> ResolvedAgent {
        ResolvedAgent {
            id,
            label: name.to_string(),
            cwd: cwd.to_string(),
            status: "Running".to_string(),
        }
    }

    #[test]
    fn resolve_by_exact_id() {
        let id = uuid::Uuid::new_v4();
        let list = vec![agent(id, "ACB", "C:/a")];
        assert_eq!(resolve_agent(&list, &id.to_string()).unwrap().id, id);
    }

    #[test]
    fn resolve_by_label_case_insensitive() {
        let id = uuid::Uuid::new_v4();
        let list = vec![agent(id, "DEF", "C:/d")];
        assert_eq!(resolve_agent(&list, "def").unwrap().id, id);
    }

    #[test]
    fn resolve_ambiguous_label_errors() {
        let list = vec![
            agent(uuid::Uuid::new_v4(), "DUP", "C:/1"),
            agent(uuid::Uuid::new_v4(), "DUP", "C:/2"),
        ];
        assert!(resolve_agent(&list, "DUP").is_err());
    }

    #[test]
    fn resolve_by_unique_id_prefix() {
        let id = uuid::Uuid::new_v4();
        let full = id.to_string();
        let prefix = &full[..8];
        let list = vec![agent(id, "ACB", "C:/a")];
        assert_eq!(resolve_agent(&list, prefix).unwrap().id, id);
    }

    #[test]
    fn resolve_not_found_errors() {
        let list = vec![agent(uuid::Uuid::new_v4(), "ACB", "C:/a")];
        assert!(resolve_agent(&list, "ZZZ").is_err());
    }

    #[test]
    fn reply_matches_by_request_id() {
        let rid = RequestId::new();
        let other = RequestId::new();
        assert!(reply_matches(&AgentEvent::Ack { request_id: rid }, rid));
        assert!(!reply_matches(&AgentEvent::Ack { request_id: other }, rid));
        // request_id 없는 전역 Error 는 수용(one-shot 명령 실패).
        assert!(reply_matches(
            &AgentEvent::Error {
                request_id: None,
                message: "x".into()
            },
            rid
        ));
    }
}
