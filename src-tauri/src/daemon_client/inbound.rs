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
//! 있나 → 명부에 있나 → 오류. 셸의 2·3단계는 아직 비어 있다([`Hop::roster`] 주석).
//!
//! ## 배선
//! 부르는 자리는 `connection.rs` 의 `Message::Text` 갈래이고(그 파일의 `accept_inbound`), 봉투를 나르는 wire
//! 프레임도 서 있다 — `AgentEvent::CommandRequest`(데몬→셸)와 `AgentCommand::CommandOutcome`(셸→데몬).
//! 셸이 자기 이름을 데몬 명부에 얹는 것도 매 (재)연결마다 나간다(`register_own_commands`).
//! 보내는 쪽도 이제 있다 — 데몬이 자기 명부에서 주인을 찾아 그 연결로 봉투를 쓴다
//! (`engram_dashboard_daemon::command_delivery` · ADR-0154). 그 다리가 서면서 이 모듈은 한 줄도 안 바뀌었다.
//!
//! ★단 **화면까지는 아직 안 닿는다**★: 데몬→셸 왕복의 답장(`AgentEvent::CommandReply`)을 웹뷰의
//! `handleEvent`(`src/api/protocolClient.ts`)가 아직 갈라내지 않아, 웹뷰가 낸 명령의 promise 는 안 풀린다
//! (그 사실의 정본 = `engram_dashboard_protocol` 의 `AgentEvent::CommandReply` 주석). 이 모듈이 도는 방향
//! (데몬→셸)과는 별개의 다리다.
//!
//! ★검증이 서는 자리★: 실 소켓·실 `AppHandle` 이 필요한 조각(연결 select 루프)은 이 패키지에서 테스트가
//! 아예 안 돌아(`0xc0000139`) 그 갈래 선택만 무커버로 남고, 그 아래 전부(슬롯 조회 · 결말 조립 · 채널 배달 ·
//! 등록 패킷 내용)는 소켓 없는 하네스가 실코드로 덮는다 — 잔여 목록의 정본은 `tests/layout_commands.rs` 헤더다.
//!
//! ★이름 충돌 주의★: `connection.rs` 에는 **다른** `CommandReply`(`oneshot::Sender`)가 있다. 그래서 그 파일은
//! 봉투의 답장 타입을 `BusReply` 로 별칭해 쓴다 — 두 이름을 한 스코프에 그냥 들이지 말 것.
// ADR-0155
// ADR-0081

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use engram_dashboard_command::{
    route, CommandDecl, CommandEnvelope, CommandError, CommandLink, CommandReply, CommandTable,
    ErrorCode, InboundCommands, ReplySink, Roster,
};

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

/// 이 홉이 배달에 쓰는 것 전부 — 태스크로 넘어가야 해서 한 `Arc` 에 묶는다.
struct Hop {
    table: CommandTable,
    /// ★언제나 비어 있다(오늘)★ — 셸이 남의 이름을 대신 배달하는 것은 화면 몫 등록(TRD §6 Step 4)이
    /// 서야 생긴다. 빈 명부라 3단계 배달의 2단계가 항상 미스이고, 그래서 모르는 이름은 `UNKNOWN_COMMAND`
    /// 로 나간다(`route` 의 마지막 갈래).
    roster: Roster,
    link: NoRelay,
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
    pub fn new(table: CommandTable, spawn: Arc<dyn TaskSpawner>, catalog_version: u32) -> Self {
        InboundReceiver {
            hop: Arc::new(Hop {
                table,
                roster: Roster::new(),
                link: NoRelay,
            }),
            spawn,
            catalog_version,
        }
    }

    /// 등록 패킷에 실을 것 — ★선언이 아니라 **표에 실제로 꽂힌 것**을 광고한다★(못 부를 이름을 등록하면
    /// 데몬이 배달한 봉투가 `UNKNOWN_COMMAND` 로 되돌아간다).
    pub fn declarations(&self) -> Vec<CommandDecl> {
        self.hop.table.decls()
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
    /// ★`deliver` 는 적용 태스크에서 불린다 — 연결 태스크가 아니다★. 소켓에 쓰려면 그 태스크로 넘기는
    /// 채널(`ConnectionCommand`)을 통해야 한다(단일 writer 규약 — `connection.rs` 헤더 「동시성」).
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
            let answered = route(&hop.table, &hop.roster, &hop.link, env).await;
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

/// 3단계 배달의 2단계가 쓸 전송 — ★셸엔 아직 내보낼 곳이 없다★.
///
/// 명부가 비어 있어(`Hop::roster`) `route` 는 이 링크에 닿지 않는다. 그래도 패닉이 아니라 값으로 답하는
/// 것은 계약이다 — 명령 핸들러는 터져서 죽지 않는다(TRD §4-⑨). 화면 몫 배달이 서면 이 자리가 웹뷰 링크로
/// 바뀐다.
struct NoRelay;

impl CommandLink for NoRelay {
    fn send(&self, env: CommandEnvelope) -> Pin<Box<dyn Future<Output = CommandReply> + Send>> {
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
