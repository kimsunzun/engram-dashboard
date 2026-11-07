//! 데몬 → 셸 인바운드 명령 수신기 — 받은 봉투를 **연결 태스크 밖**에서 적용한다.
//!
//! ★이 파일의 존재 이유가 그 한 줄이다★(ADR-0155 결정 4 · ADR-0081 「relay 적용은 액터 밖(비블로킹)」):
//! 연결 태스크가 봉투를 인라인으로 `.await` 하면 합성 명령(`agent.spawnInto` — 핸들러가 자기 안에서
//! `DaemonClient::send_command().await` 를 부른다)이 **자기 답을 자기가 못 꺼낸다**. 그 답은 같은 연결
//! 태스크의 읽기 루프에서만 해소되는데 그 루프가 지금 이 핸들러를 기다리고 있기 때문이다(self-deadlock).
//! [`InboundReceiver::on_command`] 은 그래서 **큐에 밀어 넣고 즉시 반환**하고, 실제 실행은
//! [`TaskSpawner`] 가 띄운 별도 태스크에서 돈다.
//!
//! 배달 규칙은 홉마다 같은 3단계를 쓴다(`engram_dashboard_command::route` — ADR-0155 결정 3): 내 표에
//! 있나 → 명부에 있나 → 오류. 셸의 2단계는 **웹뷰 몫**이다([`ViewCommandPort`]) — 표에 없는 이름은
//! 그 창으로 내려가고, 거기에도 없으면 `UNKNOWN_COMMAND` 로 나간다.
//!
//! ## Wiring
//! The caller is the connection driver (`connection.rs` `accept_inbound`, for a daemon `CommandRequest`); the wire
//! frames are `AgentEvent::CommandRequest` (daemon → shell) and `AgentCommand::CommandOutcome` (shell → daemon). The
//! shell's own names go onto the daemon's roster on every connect (`connection.rs` `register_commands`, packet =
//! [`registration_command`]).
//! 보내는 쪽도 이제 있다 — 데몬이 자기 명부에서 주인을 찾아 그 연결로 봉투를 쓴다
//! (`engram_dashboard_daemon::command_delivery` · ADR-0154). 셸 표에 없는 이름은 여기서 한 홉 더 내려가
//! 웹뷰에 닿는다([`ViewCommandPort`] · 실물 = `crate::view_commands`).
//!
//! ★단 **반대 방향은 아직 안 닿는다**★: 데몬→셸 왕복의 답장(`AgentEvent::CommandReply`)을 웹뷰의
//! `handleEvent`(`src/api/protocolClient.ts`)가 아직 갈라내지 않아, 웹뷰가 낸 명령의 promise 는 안 풀린다
//! (그 사실의 정본 = `engram_dashboard_protocol` 의 `AgentEvent::CommandReply` 주석). 이 모듈이 도는 방향
//! (데몬→셸)과는 별개의 다리다.
//!
//! ★Where it is verified★: `tests/layout_commands.rs` drives the receiver, the outcome writer and the registration
//! packet without a connection (its residue list is canonical); the driver's suite (`connection/tests.rs`) drives the
//! driver's arm on a memory network.
// ADR-0155
// ADR-0081

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use engram_dashboard_command::{
    route, CommandDecl, CommandEnvelope, CommandError, CommandLink, CommandReply, CommandTable,
    ErrorCode, InboundCommands, OwnerLookup, OwnerLookupSource, ReplySink,
};
use engram_dashboard_protocol::{AgentCommand, RequestId};

use super::protocol_state;

/// 태스크 하나 — 출력이 없다(답장은 [`ReplySink`] 가 나른다).
pub type BoxedTask = Pin<Box<dyn Future<Output = ()> + Send>>;

/// 적용을 **어디서 돌리나**의 seam.
///
/// ★주입인 이유★: 「연결 태스크를 안 막는다」는 개수로는 안 잡히고 **언제 도는가**로만 잡힌다. 가짜
/// 구현이 태스크를 쥐고 있는 동안 `on_command` 가 이미 반환했는지를 하네스가 직접 본다(ADR-0012).
/// 구현은 태스크를 **버려도 된다**(런타임 종료 등) — 그 경우 답장은 [`ReplySink`] 의 `Drop` 이 낸다.
pub trait TaskSpawner: Send + Sync {
    fn spawn(&self, task: BoxedTask);
}

/// 운영 구현 — 연결 태스크가 도는 **그 런타임**에 띄운다.
///
/// 새 런타임을 만들지 않는 것이 요점이다: 적용이 데몬 왕복(`agent.spawnInto`)을 하려면 같은 런타임의
/// 소켓 태스크와 인터리브돼야 한다.
pub struct RuntimeSpawner(pub tokio::runtime::Handle);

impl TaskSpawner for RuntimeSpawner {
    fn spawn(&self, task: BoxedTask) {
        self.0.spawn(task);
    }
}

/// 셸이 **자기 표에 없는 이름**을 넘길 곳 — 오늘 그곳은 웹뷰 하나다(TRD §3-8 의 홉 ③).
///
/// ★세 창구를 한 trait 으로 묶는 이유★: 셋 다 같은 한 상태(어느 창이 무엇을 보고했나)를 읽는다. 명부와
/// 링크를 따로 주입받게 하면 조립부가 그 둘이 같은 실물인지를 매번 지켜야 한다.
/// ★주입인 이유★: 실물은 `AppHandle::emit_to` 에 닿아 창 없이 못 세운다(ADR-0012). 실물은
/// `crate::view_commands::ViewCommandBridge` 다.
/// ★[`ViewCommandPort::declarations`] 는 셸 표가 쥔 이름을 실으면 안 된다★ — 실어도 배달은 안 갈린다
/// (`route` 가 표를 먼저 본다) 대신 **등록 패킷에 못 부를 이름**이 하나 는다. 거르는 자리는 구현 쪽이다
/// (`view_commands` 의 예약 이름 필터) — 여기서 한 번 더 거르면 같은 판정이 두 곳에 살고, 둘이 다른
/// 기준(선언 / 꽂힌 것)을 쓰게 되는 순간 어느 쪽이 진짜인지 알 수 없어진다.
pub trait ViewCommandPort: Send + Sync {
    /// 등록 패킷에 얹을 웹뷰 몫.
    fn declarations(&self) -> Vec<CommandDecl>;
    /// 「이 이름을 지금 웹뷰가 받을 수 있나」 — 배달 3단계의 2단계.
    fn lookup(&self, name: &str) -> OwnerLookup;
    /// 봉투를 웹뷰로 내리고 답장을 기다린다.
    fn dispatch(&self, env: CommandEnvelope) -> Pin<Box<dyn Future<Output = CommandReply> + Send>>;
}

/// 웹뷰가 없는 조립 — 2단계가 항상 미스라 모르는 이름은 `UNKNOWN_COMMAND` 로 나간다.
struct NoViewCommands;

impl ViewCommandPort for NoViewCommands {
    fn declarations(&self) -> Vec<CommandDecl> {
        Vec::new()
    }

    fn lookup(&self, _name: &str) -> OwnerLookup {
        OwnerLookup::Unknown
    }

    /// ★위 `lookup` 이 항상 `Unknown` 이라 여기 닿지 않는다★ — 그래도 패닉이 아니라 값으로 답하는 것은
    /// 계약이다(명령 핸들러는 터져서 죽지 않는다 — TRD §4-⑨).
    fn dispatch(&self, env: CommandEnvelope) -> Pin<Box<dyn Future<Output = CommandReply> + Send>> {
        let request_id = env.request_id;
        let name = env.name;
        Box::pin(async move {
            CommandReply::err(
                request_id,
                CommandError::of(
                    ErrorCode::Unsupported,
                    format!("this client cannot forward '{name}' — it has no onward command link"),
                ),
            )
        })
    }
}

/// `route` 가 요구하는 두 창구(`OwnerLookupSource`·`CommandLink`)를 한 포트에서 낸다.
struct ViewHop(Arc<dyn ViewCommandPort>);

impl OwnerLookupSource for ViewHop {
    fn lookup(&self, name: &str) -> OwnerLookup {
        self.0.lookup(name)
    }
}

impl CommandLink for ViewHop {
    fn send(&self, env: CommandEnvelope) -> Pin<Box<dyn Future<Output = CommandReply> + Send>> {
        self.0.dispatch(env)
    }
}

/// 이 홉이 배달에 쓰는 것 전부 — 태스크로 넘어가야 해서 한 `Arc` 에 묶는다.
struct Hop {
    table: CommandTable,
    /// 3단계 배달의 2·3단계 — 명부(어느 이름이 웹뷰 것인가)와 그 링크를 함께 낸다.
    view: ViewHop,
}

/// 받은 봉투를 자기 표로 배달하는 수신기.
pub struct InboundReceiver {
    hop: Arc<Hop>,
    spawn: Arc<dyn TaskSpawner>,
    catalog_version: u32,
}

impl InboundReceiver {
    /// `catalog_version` = 표를 만든 선언 블록의 세대(`CATALOG_VERSION`). 등록 패킷이 진단용으로 싣는다
    /// (받는 쪽이 자기 번호와 비교해 거절하면 틀린다 — TRD §4-①).
    ///
    /// 웹뷰 몫 없이 선다 — 셸 자기 표만 배달한다.
    pub fn new(table: CommandTable, spawn: Arc<dyn TaskSpawner>, catalog_version: u32) -> Self {
        Self::with_view(table, spawn, catalog_version, Arc::new(NoViewCommands))
    }

    /// 웹뷰 몫을 함께 지는 조립 — 운영 경로(`DaemonClient::install_command_table`)가 쓴다.
    pub fn with_view(
        table: CommandTable,
        spawn: Arc<dyn TaskSpawner>,
        catalog_version: u32,
        view: Arc<dyn ViewCommandPort>,
    ) -> Self {
        InboundReceiver {
            hop: Arc::new(Hop {
                table,
                view: ViewHop(view),
            }),
            spawn,
            catalog_version,
        }
    }

    /// 등록 패킷에 실을 것 — ★선언이 아니라 **표에 실제로 꽂힌 것**을 광고한다★(못 부를 이름을 등록하면
    /// 데몬이 배달한 봉투가 `UNKNOWN_COMMAND` 로 되돌아간다).
    ///
    /// 웹뷰 몫이 뒤에 붙는다 — 데몬 눈에 두 층은 구분되지 않고, 구분할 필요도 없다(2단 배달이 셸에서 다시
    /// 갈라 준다 — TRD §3-7 조항 2). ★그래서 이 목록은 **매번 다시 읽는다**★: 웹뷰는 소켓과 무관하게
    /// 늦게 뜨므로, 한 번 만들어 캐시하면 재연결이 옛 목록을 다시 보낸다.
    pub fn declarations(&self) -> Vec<CommandDecl> {
        let mut decls = self.hop.table.decls();
        decls.extend(self.hop.view.0.declarations());
        decls
    }

    pub fn catalog_version(&self) -> u32 {
        self.catalog_version
    }

    /// 연결 루프가 부를 **단 하나의 진입점** — 봉투를 받고, 답장은 `deliver` 로 돌려준다.
    ///
    /// ★상관 키를 인자로 받지 않는 것이 요점이다★: `request_id` 를 봉투에서 **직접** 꺼내 답장 자리를
    /// 만들므로, 부르는 쪽이 다른 요청의 키를 실어 보낼 방법이 없다(그 실수는 남의 왕복에 답을 붙인다).
    /// `deliver` 는 **정확히 한 번** 불린다 — 답을 냈든(`send`), 태스크가 답 없이 사라졌든([`ReplySink`] 의
    /// `Drop` 이 오류 답장을 낸다). 그래서 부르는 쪽은 「답이 안 올 수도 있다」를 다루지 않는다.
    /// ★`deliver` runs on the apply task, not on the connection driver's actor★ — it writes through a registry clone
    /// on the connection the request came on (`link::route::send_command_outcome`).
    pub fn accept(
        &self,
        env: CommandEnvelope,
        deliver: impl FnOnce(CommandReply) + Send + 'static,
    ) {
        let reply = ReplySink::new(env.request_id, deliver);
        self.on_command(env, reply);
    }
}

impl InboundCommands for InboundReceiver {
    /// ★여기서 하는 일은 큐 push 하나뿐이다★ — 부르는 쪽(연결 읽기 루프)이 이 함수 안에서 기다리는 것은
    /// 없다. 되돌려 인라인으로 실행하면 위 헤더의 self-deadlock 이 그대로 부활한다.
    fn on_command(&self, env: CommandEnvelope, reply: ReplySink) {
        let hop = Arc::clone(&self.hop);
        self.spawn.spawn(Box::pin(async move {
            let answered = route(&hop.table, &hop.view, &hop.view, env).await;
            reply.send(answered.outcome);
        }));
    }
}

/// 늦게 채워지는 수신기 자리.
///
/// ★왜 슬롯인가 — 순환 때문이다★: 표의 스폰 포트가 `DaemonClient` 를 쥐고(`agent.spawnInto` 는 데몬 왕복이다)
/// 그 클라이언트의 연결 태스크가 이 수신기를 쥔다. 그래서 조립 순서는 **클라이언트 → 표 → 수신기 → 슬롯
/// 채우기**뿐이고, 연결 태스크는 자기가 뜰 때 슬롯이 비어 있을 수 있다(그때 온 봉투는 아래 `get` 이 `None` 을
/// 주고 호출부가 오류 답장을 낸다 — 조용히 버리지 않는다). 데몬 쪽 `CommandTableSlot` 과 같은 형태다.
/// ★첫 승자만 이긴다★ — 두 번째 `set` 은 무시된다. 표를 갈아 끼우는 경로를 만들면 그 순간 어느 표가 도는지
/// 알 수 없어진다(재연결은 표를 다시 만들지 않는다 — 등록만 다시 보낸다).
pub struct InboundSlot(OnceLock<Arc<InboundReceiver>>);

impl InboundSlot {
    pub fn new() -> Self {
        InboundSlot(OnceLock::new())
    }

    pub fn set(&self, receiver: Arc<InboundReceiver>) {
        if self.0.set(receiver).is_err() {
            tracing::warn!("인바운드 수신기가 이미 꽂혀 있다 — 두 번째 설치는 무시한다");
        }
    }

    pub fn get(&self) -> Option<&Arc<InboundReceiver>> {
        self.0.get()
    }
}

impl Default for InboundSlot {
    fn default() -> Self {
        Self::new()
    }
}

// ── 자기 이름 등록 ────────────────────────────────────────────────────────────
//
// ★등록 패킷의 `owner` 는 **광고**다★ — 데몬은 이 값을 쓰지 않고 **그 패킷이 온 연결**에서 주인 토큰을
//   파생한다(protocol `AgentCommand::RegisterCommands` doc · daemon `note_claimed_owner`). 그래서 여기 무엇을
//   적어도 명부의 주인은 안 바뀐다 — 쓸모는 데몬 로그에서 이 연결이 누구인지 사람이 알아보는 것뿐이다.
// ★데몬의 토큰 접두(`conn-`)를 흉내내지 않는다★ — 그 형태를 적으면 데몬이 「남의 주인 토큰을 적은 등록」으로
//   보고 경고를 남긴다(`note_claimed_owner` 의 접두 분기).
//
// ★★이 값을 「클라이언트가 만든 식별자」로 승격하지 말 것(ADR-0150 결정 1)★★ — 그 식별자는 **연결마다
//   유일**해야 하는데 이 리터럴은 고정이라 두 셸이 같은 값을 쓴다. 데몬 명부는 등록을 이름 단위 last-wins 로
//   덮고 연결 해제를 주인 단위로 제거하므로(ADR-0150 결정 3), 그 승격은 **두 셸이 서로의 이름을 빼앗고 서로를
//   지우는** 동작이 된다. 식별자를 들일 때는 이 상수를 고치는 것이 아니라 연결마다 새 값을 만들어야 한다.
const SHELL_OWNER_ADVERT: &str = "engram-dashboard-shell";

/// 등록·차분 패킷이 함께 쓰는 광고 문자열 — ★두 곳에서 따로 짓지 않는다★. 갈리면 데몬 로그에서 같은
/// 셸의 두 패킷이 남남으로 보인다(명부의 주인 판정은 이 값과 무관하다 — 위 주석).
pub fn shell_owner_advert() -> engram_dashboard_protocol::OwnerToken {
    engram_dashboard_protocol::OwnerToken::new(SHELL_OWNER_ADVERT)
}

/// The registration packet this client sends — `None` when there is nothing to register or the declarations do not fit
/// the raw slot (error log · ADR-0272).
///
/// ★Why `pub` and apart from the send★: sending needs a connection (the driver queues it on every connect), but
/// **what is sent** (the advertised names · the catalog version · the owner string) does not. Apart, a harness asserts
/// the very packet that goes out (`tests/layout_commands.rs`) — a packet rebuilt by hand would stay green when the two
/// drift.
pub fn registration_command(receiver: &InboundReceiver) -> Option<AgentCommand> {
    let decls = receiver.declarations();
    if decls.is_empty() {
        return None;
    }
    let decls = match protocol_state::decls_to_wire(&decls) {
        Ok(decls) => decls,
        Err(e) => {
            tracing::error!(
                names = decls.len(),
                "등록 항목 목록을 원문 칸으로 싸지 못했다 — 이 연결에서는 등록하지 않는다: {e}"
            );
            return None;
        }
    };
    Some(AgentCommand::RegisterCommands {
        owner: shell_owner_advert(),
        decls,
        catalog_version: receiver.catalog_version(),
        request_id: RequestId::new(),
    })
}
