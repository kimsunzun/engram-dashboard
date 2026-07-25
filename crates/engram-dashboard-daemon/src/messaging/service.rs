//! service — MessagingService: 순수 구조(mailbox·ledger·groups)를 tokio 위에서 발송 파이프라인에
//! 엮는 오케스트레이터(S18 메시징 v1 increment C1 · ADR-0103/0104).
//!
//! ★역할(C1+C2+C3 스코프)★: 단일 수신자 발송의 3분기(spec §5)와 등장/idle flush(ADR-0104), TTL sweep,
//!   **idle 게이트**(C2), **회신 계약**(C3 — request 장부 오픈·회신 닫기·기한 초과 notice)을 담당한다.
//!   그룹(C4)은 **범위 밖** — 여기 넣지 않는다.
//!     ① resolve+inject 성공 → `delivered`(실제 주입 시점에만, ADR-0104)
//!        — 단 수신자가 **턴 진행 중(busy)** 이면 주입하지 않고 **파킹** = `pending`(C2 idle 게이트)
//!        — 또한 그 이름 앞에 **이미 파킹이 쌓여 있으면** 직발송이 큐를 앞지르지 않게 **함께 파킹**하고
//!          flush 에 합류시킨다(FIFO 일관성 — 수신자가 보는 순서 = 도착 순서)
//!     ② 부재(미스폰·죽음, "없는 이름" 포함) → **파킹** = `pending`(RECIPIENT_NOT_FOUND 소멸, spec §5)
//!     ③ 도달 불가/write 실패 → **파킹** = `pending`
//!     보관함 초과 → `MAILBOX_FULL` 반려(spec §5 분기 3).
//!   등장(스폰/epoch 교체)·**턴 종료(idle 전이)**·flush 시 파킹분을 **오래된 순 일괄** 주입(각 메시지
//!   개별 봉투, ADR-0104). 파킹 상태 어휘는 부재 파킹과 **공유**한다(`pending` — 새 상태 발명 금지, spec §5).
//!
//! ★idle 게이트 seam(C2 · ADR-0104 결정 3)★: "수신자가 턴 중인가" 는 `BusyGate`(messaging/busy.rs) 너머로
//!   묻는다 — 운영은 `BusyTracker`(출력 스트림 tap 이 턴 이벤트를 관측), 단위 테스트는 가짜 게이트를 끼운다.
//!   게이트를 안 꽂으면 `AlwaysIdleGate`(= 즉시 주입 = C1 동작)로 폴백한다(관측 불가 백엔드 폴백과 같은 값).
//!
//! ★순서 보장의 범위(finding 8 · load-bearing)★: "오래된 순" 은 **한 flush 배치 내부**(+ 재파킹 front-
//!   restore 로 배치 간 이월 시 오래된 것 우선)에서만 보장한다 — spec §5 가 약속하는 건 **배치 순서**지
//!   전 출처를 아우르는 global total order 가 아니다. 진행 중인 flush 배치와 **동시 직발송**(handle_single_send)
//!   이 수신자 stdin 에서 인터리브될 수 있다(둘 다 락 밖에서 inject) — 사람 대화 수준 메시지율에선 무해하다고
//!   보고 **의도적으로 수용**한다(전역 순서 직렬화는 inject 를 락 안으로 넣어야 해 락 규율과 상충).
//!
//! ★단일 락(load-bearing — ADR-0006 정신)★: Mailbox+Ledger+Groups 를 **하나의 `Mutex<MessagingState>`**
//!   뒤에 둔다. 락 순서 위험이 없고(락 하나) 메시지율이 극히 낮아(사람 대화 수준) 경합이 무의미하다.
//!   ★절대 규율★: 이 락을 **든 채로 AgentManager(inject/roster)를 부르지 않는다** — 락 아래에서 결정할
//!   것(파킹/주입 대상 수집)을 먼저 끝내고 락을 놓은 뒤 DeliveryPort(외부 호출)를 부른다. 이걸 어기면
//!   inject 가 내부에서 다른 락(sessions RwLock 등)을 잡아 락 순서 역전·데드락 위험이 생긴다.
//!
//! ★delivery seam(ADR-0012 · 헤드리스 테스트)★: AgentManager 를 직접 부르지 않고 `DeliveryPort` 트레잇
//!   너머로 부른다 — 운영은 `ManagerDeliveryPort`(Arc<AgentManager> 얇은 래퍼), 단위 테스트는
//!   `FakeDeliveryPort` 를 끼워 claude 바이너리·실 PTY 없이 3분기·flush·sweep 을 결정적으로 단언한다.
//!
//! ★봉투 = 주입 시점 조립(단일 wrap point, ADR-0096)★: 파킹은 **감싸지 않은 body + 발신자 이름 + 회신
//!   계약 메타**를 저장하고, 봉투는 **주입할 때** `wrap_message`/`wrap_notice`(ingress.rs 단일 wrap point)로
//!   만든다. 왜: 파킹과 flush 사이 봉투 포맷(colon/xml 전역 스위치)이 바뀔 수 있고, 그때 flush 는 **현재**
//!   포맷으로 감싸야 한다. park 시점에 미리 감싸면 옛 포맷이 굳어 버린다. 그래서 raw body + 속성 재료를
//!   나르고(`ParkPayload`) 조립은 주입 순간 한 곳에서 — 즉시 배달과 늦은 배달의 봉투가 **같아야** 한다.
//!
//! ★회신 계약(C3 · spec §3 · ADR-0103 결정 2/3)★:
//!   - **request 발송** → 배달이든 파킹이든 **접수되면 계약이 열린다**(`Ledger::open_request`). 예약은 배달
//!     시도 **전에** 하고(발송 기준 시계 — spec §3), 반려(MAILBOX_FULL)로 끝나면 즉시 닫아 유령 타임아웃을
//!     막는다. 봉투에는 `id`/`type="request"`/`reply-by` 속성이 붙는다(노출 원칙 — spec §1).
//!   - **회신 발송**(`reply_to`) → 정상 배달/파킹 뒤 `Ledger::close_on_reply`(엄격 매칭). 매칭 실패는
//!     **배달에 영향 없다** — 메시지는 그대로 가고 계약만 안 닫힌다(응답 shape 도 그대로).
//!   - **기한 초과** → sweep 이 `due_timeouts` 로 걷어 **발신자에게** `<notice>` 를 보낸다(수신자 재촉
//!     아님). 배달은 `ParkKind::Notice` 파킹 + flush 도어벨로 일원화한다 — **cap 예외**(통지가 막히면
//!     계약이 반쪽)이고, sweep task 에서 blocking write 를 하지 않기 위함이다(`deliver_notice` 주석).
//!
//! tauri import 0(daemon crate).
// ADR-0103
// ADR-0104

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use engram_dashboard_core::agent::manager::AgentManager;
use engram_dashboard_core::agent::types::{AgentId, AgentStatus, WriteOutcome};

use super::busy::{AlwaysIdleGate, BusyGate};
use super::ledger::{DeliveryStatus, DropOutcome, DueTimeout, Ledger, OpenOutcome, ReplyOutcome};
use super::mailbox::{Mailbox, ParkError, ParkKind, ParkedMessage};
use crate::control::ingress::{
    new_msg_id, wrap_message, wrap_notice, DeliveryObservation, Entrance, EnvelopeFields,
    EnvelopeFormat,
};
use crate::control::registry::{BoundIdentity, ControlRegistry};

/// ★notice 의 장부상 발신자 라벨(C3)★ — `<notice>` 는 **from 이 없는** 데몬 통지라 진짜 발신자가 없다.
///   그래도 장부 레코드는 `from` 칸을 요구하므로(조용한 유실 금지 — notice 도 반드시 장부에 남는다) 데몬
///   출처임을 나타내는 고정 라벨을 쓴다. ★주소가 아니다★: 이 문자열로 메시지를 보낼 수 없고(로스터 이름이
///   아님) 봉투에도 절대 렌더되지 않는다(notice 봉투엔 from 속성 자체가 없다 — ADR-0103 불변식).
const NOTICE_SENDER_LABEL: &str = "engram";

/// ★C3 발송 메타(회신 계약 축)★ — ingress 가 구문 검증을 마친 인자를 서비스가 쓸 형태로 정규화한 값.
///
/// ★raw 표기와 파싱값을 **둘 다** 나르는 이유(load-bearing)★: 봉투 속성 `reply-by` 는 발신자가 쓴 **표기
///   그대로**(`"10m"`) 보여야 하고(spec §1 — 수신 LLM 이 읽는 계약), 장부 타이머는 **절대 기한**을 계산할
///   `Duration` 이 필요하다(spec §3 "데몬이 절대시각 환산"). 한쪽만 나르면 다른 쪽에서 재파싱·재렌더가
///   생겨 표기가 미묘하게 달라진다(`60m` → `1h`).
/// ★`Default` = 통보★: 전 필드 비활성이 곧 기본 메시지(type 없음)다 — 기존 C1/C2 경로와 byte-identical.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SendMeta {
    /// request 인가 — 장부 미회신 오픈 + 봉투 `type="request"`(+ `id`).
    pub request: bool,
    /// 발신자가 쓴 기한 표기 그대로(`"10m"`). 봉투 `reply-by` 속성 값. `request` 일 때만 의미 있다.
    pub reply_by_raw: Option<String>,
    /// 파싱된 기한 — 장부가 `발송시각 + 이것` 으로 절대 기한을 굳힌다. raw 가 Some 이면 반드시 Some.
    pub reply_by: Option<Duration>,
    /// 어느 request 의 회신인가(원본 id) — 봉투 `in-reply-to` + 장부 엄격 매칭 닫기.
    pub reply_to: Option<String>,
}

impl SendMeta {
    /// 이 발송이 봉투에 붙일 속성(spec §1 노출 원칙 — **행동을 바꾸는 필드만**).
    ///
    /// - request → `id`(회신에 필요) + `type="request"` + (있으면) `reply-by`.
    /// - 회신 → `in-reply-to` 만(발신 인자 `reply_to` 가 수신 속성 `in-reply-to` 로 나타난다 — spec §1 표기 매핑).
    /// - 통보 → 전부 None(속성 없는 plain `<message from>`).
    ///
    /// ★`id` 는 request 에만★: 통보/회신 봉투에 id 를 실으면 수신 LLM 이 "이건 회신해야 하나" 를 헷갈린다 —
    ///   노출 필드가 곧 행동 신호라 필요 없는 축은 아예 숨긴다(spec §1 · ADR-0103 결정 1).
    /// ★가시성 `pub(crate)`★: ingress 단위 테스트가 "검증된 인자 → 봉투" 를 한 줄로 단언한다(속성 조립
    ///   규칙의 단일 출처를 테스트가 우회해 재구현하지 않게).
    pub(crate) fn envelope_fields(&self, msg_id: &str) -> EnvelopeFields {
        EnvelopeFields {
            id: self.request.then(|| msg_id.to_string()),
            msg_type: self.request.then(|| "request".to_string()),
            reply_by: if self.request {
                self.reply_by_raw.clone()
            } else {
                None
            },
            in_reply_to: self.reply_to.clone(),
            // C4(그룹 방송) 자리 — 단일 수신자 발송은 `to` 속성을 싣지 않는다(방송이 아님, spec §1).
            to: None,
        }
    }
}

/// 발송 1건의 결과(spec §6 응답 shape 의 한 수신자 축). 상위(handle_send)가 `results[]` 원소로 싼다.
///
/// ★상태 어휘(spec §5·§6)★: `delivered`(실제 주입) / `pending`(파킹 — 부재·도달불가). `skipped`(그룹
///   죽은 멤버)는 C4, `MAILBOX_FULL`(반려)은 `SendReject` 로 분리(성공 축이 아니라 반려).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
    /// 실제 주입 완료 — ledger `delivered`(ADR-0104 실제 주입 시점).
    Delivered,
    /// 파킹됨 — ledger `pending`. hint 로 사유를 실어 발신자 자기교정을 돕는다(부재/도달불가 구분).
    Parked { hint: String },
}

/// 발송 반려(spec §5·§6 `{status:"error", code, hint}`). 성공 축(SendOutcome)과 분리 — 파킹조차 못 함.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendReject {
    /// 수신자 보관함이 cap(100)에 도달 — 신규 message 반려(spec §5 분기 3). 오래된 것은 TTL 로 소멸.
    MailboxFull,
    /// ★C3 — 논리 메시지 id 가 장부에 이미 존재★(사실상 불가: 2.8×10^12 공간). 이력 레코드든 request
    ///   추적이든 **어느 쪽에든** 같은 id 가 있으면 이 값이다(종류 무관 — 리뷰 fix 12).
    ///
    /// ★부작용 없음 보장(load-bearing — 호출자 재시도 계약)★: id 예약은 이 함수의 **첫 부작용**이라,
    ///   이 값을 돌려줄 때 배달·파킹·장부 레코드는 하나도 일어나지 않았다. 그래서 호출자(ingress)가 새 id 로
    ///   그대로 다시 부르면 중복 배달 없이 안전하게 재시도된다.
    IdCollision,
    /// ★C3 리뷰 fix 3 — 오픈 계약 수가 상한(`MAX_OPEN_REQUESTS`)에 도달★. request 발송만 해당한다
    ///   (통보/회신은 계약을 열지 않으므로 이 반려를 받지 않는다).
    ///
    /// ★왜 반려인가★: 오픈 계약은 이제 이력 evict 를 견디므로(ledger.rs `record`) 상한이 필요하고, 상한에서
    ///   **오래된 계약을 밀어내는 대신 새 발송을 거절**한다 — 이미 기다리는 계약을 조용히 없애는 건 유실이고,
    ///   반려는 발신자가 즉시 알고 조정할 수 있는 가시적 실패다. `IdCollision` 과 마찬가지로 **부작용 0**이다.
    RequestCapacity,
}

/// 배달 경계 seam(ADR-0012) — MessagingService 가 AgentManager 를 직접 부르지 않고 이 너머로 부른다.
///
/// ★왜 트레잇인가★: 헤드리스 단위 테스트가 claude 바이너리·실 PTY·spawn 파이프 없이 3분기·flush·sweep 을
///   결정적으로 검증하게 하려는 seam 이다(FakeDeliveryPort). 운영은 `ManagerDeliveryPort` 가 실
///   AgentManager 로 위임한다.
/// ★락 밖 호출 계약(load-bearing)★: 이 트레잇의 메서드는 MessagingService 의 messaging 락을 **놓은
///   상태**에서만 불린다(모듈 헤더 락 규율). 구현이 내부에서 다른 락을 잡아도 안전하도록 그 전제를 둔다.
pub trait DeliveryPort: Send + Sync {
    /// 완성된 봉투 바이트를 수신자에게 주입한다(= write_stdin). 성공 시 `WriteOutcome`(관측 상관용),
    /// 실패 시 에러 문자열(도달 불가·write 실패 — 상위가 파킹으로 처리). 봉투 조립은 상위가 이미 끝냄.
    fn inject(&self, to_id: AgentId, bytes: &[u8]) -> Result<WriteOutcome, String>;

    /// 지금 살아있고(Running|Exiting) **제어 채널로 도달 가능한**(structured 출력) 에이전트 로스터.
    /// (name, id, epoch) 스냅샷 — resolve(이름→id)·flush(등장/epoch 교체 감지)·`@all`(C4) 공용.
    /// ★도달성 포함★: TUI(비-structured)는 stdin 주입이 유효 라인이 안 되므로 여기서 제외한다(spec §5
    ///   "unreachable → 파킹" 의 판정을 로스터 단계로 당긴다 — 비-도달 이름은 애초에 "산 수신자"가 아님).
    fn live_reachable_agents(&self) -> Vec<LiveAgent>;

    /// id → canonical 표시 이름(봉투 sender·수신자 이름 단일 출처, ADR-0101). 없으면 None.
    fn canonical_name(&self, id: AgentId) -> Option<String>;
}

/// ★flush 도어벨 seam(C2 리뷰 fix 11)★ — "이 에이전트의 파킹 큐를 지금 flush 해라" 를 **다른 스레드에
///   맡기는** 출구. 운영은 flush 채널로 논블록 enqueue 한다(ws.rs `ChannelIdleNotifier`).
///
/// ★왜 필요한가(발신 스레드 보호)★: 자가치유(park 직후 재확인)와 FIFO 합류는 배치 flush 를 부른다 —
///   그 안의 inject 는 자식 stdin **blocking write** 다. 이걸 발신 경로(MCP/HTTP 요청을 처리하는 tokio
///   워커 스레드)에서 그대로 실행하면 한 수신자의 막힌 파이프가 데몬 요청 처리를 잡아먹는다(C1 이
///   flush worker 를 따로 둔 것과 **같은 이유**인데, 자가치유 경로만 그 규율을 우회하고 있었다).
/// ★계약★: `request_flush` 는 **논블록**(채널 enqueue 만). 실제 flush 는 소비자(flush lane)가 한다.
/// ★미배선 폴백(문서화된 두 갈래)★: 도어벨을 꽂지 않은 조립(실험 bin·단위 테스트)은 **인라인 flush** 로
///   폴백한다 — 도어벨 부재가 "배달이 멈춘다" 로 번지지 않게 하는 안전 기본값이다(fail-open). 대가는
///   그 조립에서 호출 스레드가 배치 write 를 지는 것뿐이고, 운영 조립(lib.rs)은 항상 도어벨을 꽂는다.
pub trait FlushTrigger: Send + Sync {
    /// 이 에이전트 앞 파킹을 flush 하라고 요청한다(논블록). 중복 요청은 소비자가 접는다(coalescing).
    fn request_flush(&self, id: AgentId);
}

/// 로스터 항목 — 살아있고 도달 가능한 한 에이전트의 (id, 이름, epoch) 스냅샷.
#[derive(Debug, Clone)]
pub struct LiveAgent {
    pub id: AgentId,
    pub name: String,
    pub epoch: u32,
}

/// 운영 DeliveryPort — Arc<AgentManager> 얇은 래퍼. manager 공개 API 만 부른다(ADR-0006 각 호출이
///   내부에서 sessions lock 을 clone 후 즉시 해제하는 규율을 그대로 탄다).
pub struct ManagerDeliveryPort {
    manager: Arc<AgentManager>,
}

impl ManagerDeliveryPort {
    pub fn new(manager: Arc<AgentManager>) -> Self {
        Self { manager }
    }
}

impl DeliveryPort for ManagerDeliveryPort {
    fn inject(&self, to_id: AgentId, bytes: &[u8]) -> Result<WriteOutcome, String> {
        // manager.rs:818 — 배달-경계 계측판. 완결성 = Ok/Err(WriteOutcome 주석).
        self.manager
            .write_stdin_observed(to_id, bytes)
            .map_err(|e| e.to_string())
    }

    fn live_reachable_agents(&self) -> Vec<LiveAgent> {
        // list_agents 스냅샷 1회 → 산(Running|Exiting) + structured(제어 채널 도달 가능)만.
        self.manager
            .list_agents()
            .into_iter()
            .filter(|a| {
                matches!(a.status, AgentStatus::Running | AgentStatus::Exiting)
                    && a.capabilities.output.structured
            })
            .map(|a| LiveAgent {
                id: a.id,
                name: a.name,
                epoch: a.epoch,
            })
            .collect()
    }

    fn canonical_name(&self, id: AgentId) -> Option<String> {
        self.manager.canonical_name(id)
    }
}

/// ★단일 락 아래 상태(load-bearing — ADR-0006)★: mailbox+ledger+groups 를 한 Mutex 뒤에 함께 둔다.
///   락 순서 위험 제거 + 극저 메시지율이라 경합 무의미. groups 는 C4 에서 쓰나 컨테이너는 지금 배치한다
///   (increment-B 순수 구조를 서비스가 소유 — 스코프 밖 로직은 추가 안 함).
struct MessagingState {
    mailbox: Mailbox,
    ledger: Ledger,
    // C4(그룹 발송)가 쓸 자리. C1 은 소유만 하고 만지지 않는다(over-build 금지 — allow dead 로 명시).
    #[allow(dead_code)]
    groups: super::groups::Groups,
}

/// MessagingService — 순수 구조를 발송 파이프라인에 엮는 데몬측 오케스트레이터(C1).
///
/// ★공유★: 데몬 부팅에서 `Arc<MessagingService>` 로 만들어 ingress(handle_send)·flush observer·sweep
///   task 가 같은 인스턴스를 공유한다(lib.rs run()).
pub struct MessagingService {
    /// mailbox+ledger+groups 단일 락(모듈 헤더 규율). 이 락을 든 채 port 호출 금지.
    state: Mutex<MessagingState>,
    /// 배달 seam(ADR-0012) — inject/roster/canonical_name.
    port: Arc<dyn DeliveryPort>,
    /// 봉투 포맷 전역 상태 + 배달 관측 싱크 거처(ADR-0096/0088). inject 마다 format 을 읽고 관측 레코드를
    ///   발행한다 — handle_send 와 **같은 Arc**(전역 상태 하나).
    registry: Arc<ControlRegistry>,
    /// ★idle 게이트(C2 · ADR-0104 결정 3)★ — 주입 전에 "수신자가 턴 중인가" 를 묻는 seam. 운영은
    ///   `BusyTracker`, 미배선/관측 불가는 `AlwaysIdleGate`(즉시 주입 폴백 — busy.rs 헤더).
    busy: Arc<dyn BusyGate>,
    /// ★flush 도어벨(C2 리뷰 fix 11)★ — 자가치유·FIFO 합류가 배치 flush 를 **다른 스레드**에 넘기는 출구.
    ///   `None` = 미배선 → 인라인 flush 폴백(FlushTrigger 주석의 문서화된 두 갈래).
    trigger: Option<Arc<dyn FlushTrigger>>,
}

impl MessagingService {
    /// port + registry 주입 생성자(테스트가 FakeDeliveryPort 를 끼운다). 게이트 = `AlwaysIdleGate`
    ///   (즉시 주입 = C1 동작). 게이트를 검증하는 조립은 `new_gated` 를 쓴다.
    pub fn new(port: Arc<dyn DeliveryPort>, registry: Arc<ControlRegistry>) -> Self {
        Self::new_gated(port, registry, Arc::new(AlwaysIdleGate))
    }

    /// port + registry + **idle 게이트** 주입 생성자(C2). 운영 조립(lib.rs)과 게이트 단위 테스트가 쓴다.
    pub fn new_gated(
        port: Arc<dyn DeliveryPort>,
        registry: Arc<ControlRegistry>,
        busy: Arc<dyn BusyGate>,
    ) -> Self {
        Self {
            state: Mutex::new(MessagingState {
                mailbox: Mailbox::new(),
                ledger: Ledger::new(),
                groups: super::groups::Groups::new(),
            }),
            port,
            registry,
            busy,
            trigger: None,
        }
    }

    /// ★flush 도어벨 주입(builder — C2 리뷰 fix 11)★: 운영 조립(lib.rs)이 flush 채널 송신단을 꽂아,
    ///   자가치유·FIFO 합류의 배치 write 가 발신 스레드(MCP/HTTP)에서 실행되지 않게 한다.
    ///   생성자 인자로 안 받는 이유: 도어벨은 flush 채널 = 서비스보다 **뒤에 조립되는** 배선이고, 게이트
    ///   미검증 조립(실험 bin·단위 테스트)은 이걸 꽂지 않아도 동작해야 한다(폴백 = 인라인 flush).
    pub fn with_flush_trigger(mut self, trigger: Arc<dyn FlushTrigger>) -> Self {
        self.trigger = Some(trigger);
        self
    }

    /// 운영 편의 생성자 — Arc<AgentManager> 를 ManagerDeliveryPort 로 감싼다. ★게이트 없음(즉시 주입)★ —
    ///   실험 bin 등 idle 게이트를 쓰지 않는 조립용. 데몬 부팅은 `for_manager_gated` 를 쓴다.
    pub fn for_manager(manager: Arc<AgentManager>, registry: Arc<ControlRegistry>) -> Self {
        Self::new(Arc::new(ManagerDeliveryPort::new(manager)), registry)
    }

    /// 운영 편의 생성자(C2) — manager 래핑 + idle 게이트 주입(데몬 부팅·통합 테스트용).
    pub fn for_manager_gated(
        manager: Arc<AgentManager>,
        registry: Arc<ControlRegistry>,
        busy: Arc<dyn BusyGate>,
    ) -> Self {
        Self::new_gated(Arc::new(ManagerDeliveryPort::new(manager)), registry, busy)
    }

    /// ★단일 수신자 발송(spec §5 3분기 — C1)★. handle_send 의 3-branch rewiring 이 검증·auth 통과 후 부른다.
    ///   - `msg_id`: 상위가 부여한 논리 메시지 id(ledger 상관·ACK 동봉 축).
    ///   - `from`: 발신자 신원(토큰 파생, ADR-0086). 관측 레코드·canonical 이름 조회에 쓴다.
    ///   - `sender_name`: 봉투 sender 표시 이름(상위가 canonical 로 이미 해석 — WYSIWYA ADR-0101).
    ///   - `to`: 수신자 지목(이름 또는 AgentId 문자열).
    ///   - `body`: **감싸지 않은** 본문(봉투 조립은 주입 시점 wrap_message — 단일 wrap point).
    ///
    /// 분기(spec §5):
    ///   ① 산 수신자 해석 성공 + **idle** + 큐 비어 있음 + inject 성공 → ledger `delivered`(실제 주입,
    ///      ADR-0104) → `Delivered`.
    ///   ①-b (C2) 해석 성공했으나 **busy**(턴 진행 중) → 주입 없이 park + ledger `pending` → `Parked`.
    ///        턴 종료(idle 전이) 트리거가 오래된 순 일괄 주입한다(ADR-0104 결정 3).
    ///   ①-c (C2 fix 5) idle 이지만 그 이름 앞에 **이미 파킹이 있음** → park + `Parked` + flush 도어벨.
    ///        직발송이 큐를 앞질러 수신자 관점 순서가 뒤집히는 것을 막는다(FIFO 일관성).
    ///   ② 부재(로스터에 없음 — "없는 이름" 포함) → park + ledger `pending` → `Parked`. 오타는 TTL 방어.
    ///   ③ 해석 성공했으나 inject 실패(그 틈에 죽음·transport 오류) → **재파킹** + ledger `pending`
    ///      (spec: unreachable → 파킹). 조용한 유실 금지 — 반드시 ledger 에 남긴다.
    ///   cap 초과 → `Err(MailboxFull)`(반려).
    ///
    /// ★C3 회신 계약(spec §3)★ — 3분기 **바깥**을 감싸는 두 겹:
    ///   - `meta.request` 면 3분기에 들어가기 **전에** 장부 계약을 연다(`open_request`). 왜 먼저인가:
    ///     `reply_by` 시계는 **발송 기준**이라(spec §3·§5) 배달 지연·파킹과 무관해야 하고, 계약은 "배달됐든
    ///     파킹됐든 접수되면 열린다"(spec §3 단계 2)라 분기별로 4곳에 흩뿌리면 한 갈래가 새기 쉽다.
    ///     예약 실패(`DuplicateId`)는 **부작용 0 상태**에서 `IdCollision` 으로 즉시 반려한다(호출자가 새 id 로 재시도).
    ///     반려(cap 초과)로 끝나면 열어 둔 계약을 그 자리에서 닫는다 — 안 닫으면 배달된 적 없는 요청이
    ///     기한 초과 notice 를 쏜다(유령 타임아웃).
    ///   - `meta.reply_to` 면 발송이 **접수된 뒤** 엄격 매칭으로 계약을 닫는다(`close_on_reply`). 매칭 실패
    ///     (NoMatch/AlreadyClosed)는 **에러가 아니다** — 회신 메시지는 정상 배달되고 계약만 안 닫힌다
    ///     (엄격 매칭의 정의 — spec §2). 응답 shape 은 어느 경우든 동일하다(새 필드 없음).
    ///
    /// ★락 규율(모듈 헤더)★: 로스터 조회·resolve 는 port 호출(락 밖) → 그 결과로 락을 잡아 ledger record +
    ///   (필요 시)park 결정 → **락 해제** → inject(락 밖) → 결과에 따라 다시 짧게 락 잡아 transition/park.
    ///   계약 오픈/닫기도 순수 장부 조작이라 **짧은 단독 락 구간**에서 하고, 로깅은 락 밖에서 한다.
    // ADR-0103 (C3 — request/reply 계약)
    #[allow(clippy::too_many_arguments)]
    pub fn handle_single_send(
        &self,
        msg_id: &str,
        from: BoundIdentity,
        sender_name: &str,
        to: &str,
        body: &str,
        entrance: Entrance,
        meta: &SendMeta,
    ) -> Result<SendOutcome, SendReject> {
        // ★상호배타는 ingress 가 **유일한** 검증자다(리뷰 fix 11)★: 이 함수와 `SendMeta` 는 pub 이라 다른
        //   조립(스모크 bin·미래 입구)이 직접 부를 수 있는데, request+reply_to 를 동시에 실으면 한 발송이
        //   계약을 열면서 남의 계약을 닫는 뒤엉킨 상태가 된다(spec §6 상호배타). 여기서 검증을 **복제하지
        //   않는 이유**는 반려 코드/문구가 입구마다 갈리면 안 되기 때문이고(entrance-agnostic — ingress
        //   `validate_contract` 가 정본), 대신 디버그 빌드에서 배선 실수를 즉시 터뜨린다.
        debug_assert!(
            !(meta.request && meta.reply_to.is_some()),
            "ingress가 유일 검증자 — request와 reply_to는 상호배타(spec §6)"
        );
        // 기한이 있으면 표기도 반드시 함께 온다(SendMeta 주석) — 장부는 통지 문구에 표기 원본을 쓴다.
        debug_assert_eq!(
            meta.reply_by.is_some(),
            meta.reply_by_raw.is_some(),
            "reply_by 는 파싱값과 표기 원본을 쌍으로 나른다(SendMeta)"
        );

        // 1) 로스터 조회(락 밖 — port 호출) → 이름/AgentId 해석.
        let roster = self.port.live_reachable_agents();
        let resolved = resolve_live(to, &roster);

        // ★park 키 정규화(finding 4 · load-bearing)★: 등장 flush 는 canonical **NAME** 으로 keyed 이다
        //   (flush observer 가 로스터 diff 로 이름을 넘긴다). 그런데 `to` 가 exact AgentId 이고 그 에이전트가
        //   살아는 있으나 **비-도달**(TUI 등 non-structured)이면 resolve_live 는 None 을 내고, 여기서 UUID
        //   문자열 그대로 park 하면 park 키(UUID) ≠ flush 키(canonical name) 라 그 파킹은 **영영 flush 안 됨**.
        //   그래서 park 전에 `to` 를 AgentId 로 파싱해 그 에이전트가 존재하면 canonical_name 으로 park 키를
        //   바꾼다(그 이름이 도달 가능해지면 flush 가 잡게). 파싱 실패/미존재면 리터럴 문자열 그대로(이름 지목).
        // ★C3 에서 앞당김★: 계약 오픈이 "누구에게 건 요청인가"(recipient)를 3분기 **전에** 알아야 해서
        //   여기서 한 번 계산한다. 해석에 성공했으면 그 canonical 이름이 곧 장부/파킹 키다.
        let recipient_key = match &resolved {
            Some(t) => t.name.clone(),
            None => self.canonical_park_key(to),
        };

        // 2) ★id 예약 + C3 계약 예약(발송 기준 시계)★ — 배달 시도 전, 부작용 0 지점에서 **한 락 구간**.
        //
        // ★id 충돌은 발송 종류를 안 가린다(리뷰 fix 12 · load-bearing)★: 예전엔 request 만 충돌을 봤다
        //   (`open_request` 의 DuplicateId). 그런데 msg_id 는 이력 레코드 키((msg_id,to))이자 관측 상관 키라,
        //   통보/회신이 기존 id 와 겹치면 `transition` 이 남의 레코드를 집고 장부·관측이 두 메시지를 한 id 로
        //   뭉갠다 — request 가 아니어도 똑같이 해롭다. 그래서 종류 무관 같은 검사를 먼저 한다.
        // ★한 락 안에서 검사+예약(원자성)★: 검사와 open_request 사이에 락을 놓으면 그 틈에 같은 id 가
        //   기록될 수 있다(사실상 불가하지만 검사의 의미가 사라진다).
        // ★남는 창: "동시에 뽑힌 같은 fresh id" — 전면 예약(reservation)을 **거부한 결정**(round-final fix 2)★
        //   이 검사는 **이미 장부에 있는 id** 와의 충돌만 잡는다. 두 발송이 같은 값을 **동시에 뽑아** 둘 다
        //   검사를 통과하고(둘 다 아직 미기록) 뒤이어 각자 record 하는 창은 남는다. 이걸 없애려면 검사
        //   시점에 id 를 장부에 **선점(reserve)** 해 두고 배달 성패에 따라 커밋/해제해야 하는데:
        //     · 발생 확률 = 36^8(2.8×10^12) 공간에서 같은 값을 마이크로초 창 안에 두 번 뽑는 경우 — 실질 0.
        //     · 피해 = 이력 레코드 하나가 두 메시지를 한 id 로 뭉갠다(**관측상** 오염). 배달·회신 계약은
        //       (msg_id, to) 키라 수신자가 다르면 서로를 덮지도 않는다.
        //     · 비용 = 예약 생명주기(커밋/해제/누수 회수)가 mailbox·flush·반려 경로 전부와 락으로 얽힌다 —
        //       drop_request 급 회수 경로를 하나 더 만드는 셈이고, 그 배선 버그가 위 확률보다 훨씬 크다.
        //   그래서 **검사만** 하고 예약은 두지 않는다. 걸리면 호출자(ingress)가 새 id 로 1회 재시도한다.
        let now = Instant::now();
        let reservation = {
            let mut st = self.state.lock().expect("messaging state poisoned");
            if st.ledger.msg_id_in_use(msg_id) {
                Err(SendReject::IdCollision)
            } else if meta.request {
                // 기한은 (Duration, 표기 원본) 쌍으로 넘긴다 — 통지 문구가 발신자 표기를 그대로 쓰게(fix 6).
                let reply_by = meta.reply_by.zip(meta.reply_by_raw.clone());
                match st.ledger.open_request(
                    msg_id,
                    sender_name,
                    from.agent_id,
                    &recipient_key,
                    reply_by,
                    now,
                ) {
                    OpenOutcome::Opened => Ok(()),
                    OpenOutcome::DuplicateId => Err(SendReject::IdCollision),
                    OpenOutcome::Full => Err(SendReject::RequestCapacity),
                }
            } else {
                Ok(())
            }
        };
        // ★락 보유 중 tracing 금지★ — 락을 놓은 뒤 찍는다(모듈 헤더 규율).
        match reservation {
            Ok(()) => {}
            Err(SendReject::IdCollision) => {
                tracing::error!(
                    msg_id = %msg_id,
                    "메시지 id 예약 실패 — 장부(이력/계약)에 같은 id 가 이미 있음(사실상 불가). 호출자가 새 id 로 재시도(ADR-0103)"
                );
                return Err(SendReject::IdCollision);
            }
            Err(reject) => {
                tracing::warn!(
                    msg_id = %msg_id,
                    "request 계약 오픈 실패 — 미회신 계약이 상한에 도달(ADR-0103 · ledger MAX_OPEN_REQUESTS)"
                );
                return Err(reject);
            }
        }

        let result = self.dispatch_single(
            msg_id,
            from,
            sender_name,
            to,
            body,
            entrance,
            meta,
            resolved,
            &recipient_key,
        );

        match &result {
            Ok(_) => {
                // 3) ★C3 회신 닫기(엄격 매칭)★ — 접수된 회신만 계약을 닫는다.
                if let Some(in_reply_to) = &meta.reply_to {
                    self.close_reply_contract(in_reply_to);
                }
            }
            Err(_) => {
                // 4) ★반려 시 예약 회수(누수 방지 — load-bearing)★: cap 초과로 **아예 접수되지 않은** 요청의
                //    계약이 열린 채 남으면 (a) 기한이 지나 발신자에게 "회신 없음" notice 가 가고(보낸 적 없는
                //    요청에 대해) (b) 이력 레코드가 없어 evict 계기도 없으니 추적이 무계 증식한다. 그래서
                //    반려 갈래에서 그 자리에서 예약을 **지운다**(닫는 게 아니라 제거 — 반려는 "계약 이행" 이
                //    아니라 "계약 미성립": ledger.rs `drop_request` 주석).
                // ★이 회수가 막는 것과 못 막는 것(리뷰 fix 5 — 과대 주장 보정)★: 이건 **누수 방지**지
                //    "통지가 먼저 나가는 레이스" 방지가 아니다. 예약(위 2단계)과 여기 사이에 sweep 이 끼어들어
                //    기한 초과를 판정하면 통지는 이미 파킹돼 나간다 — 그 통지는 회수할 수 없다(이미 발신자
                //    큐에 있는 메시지다). 남는 잔여: 반려된 발송에 대해 통지가 한 번 갈 수 있다. 실제로는
                //    reply_by 최소 1분(ingress) vs 이 구간 마이크로초라 사실상 도달 불가하고, 발생하면
                //    아래 warn 로그가 그 이중 결말을 관측 가능하게 남긴다.
                if meta.request {
                    let outcome = {
                        let mut st = self.state.lock().expect("messaging state poisoned");
                        st.ledger.drop_request(msg_id)
                    };
                    // 락 밖 로깅(모듈 헤더 규율).
                    if outcome == (DropOutcome::Removed { notified: true }) {
                        tracing::warn!(
                            msg_id = %msg_id,
                            "반려된 request 의 계약이 **이미 기한 초과 통지된** 상태였다 — 통지는 회수 불가(발신자에게 이미 감). 예약↔반려 사이 sweep 이 끼어든 희귀 레이스(ADR-0103)"
                        );
                    }
                }
            }
        }
        result
    }

    /// ★회신 계약 닫기(C3)★ — 엄격 매칭 결과에 따라 **로깅만** 갈린다(배달·응답에는 영향 없음).
    ///
    /// - `Closed` = 정상(첫 유효 회신).
    /// - `ClosedHistoryAnomaly` = 계약은 닫혔으나 이력이 `Delivered → Replied` 간선을 못 탐(예: 회신이
    ///   원본보다 먼저 처리돼 원본이 아직 `pending`). 계약 정본은 추적이므로 그대로 두고 **관측**만 한다
    ///   (ledger.rs `ReplyOutcome` 주석 — anomaly = observable, not silent).
    /// - `NoMatch`/`AlreadyClosed` = 틀린 id 이거나 이미 닫힘. **정상 경로**다 — 엄격 매칭이 아무 것도 닫지
    ///   않았을 뿐 회신 메시지 자체는 이미 배달/파킹됐다(spec §2). 발신자에게 새 에러를 만들지 않는다:
    ///   "회신은 갔는데 계약이 안 닫혔다" 는 발신자가 고칠 수 없는 상태이고, 반려하면 이미 배달된 메시지에
    ///   대해 재시도를 유발해 중복이 난다.
    /// ★락 규율★: 짧은 단독 락 구간에서 장부만 만지고, 로깅은 **락을 놓은 뒤** 한다.
    fn close_reply_contract(&self, in_reply_to: &str) {
        let outcome = {
            let mut st = self.state.lock().expect("messaging state poisoned");
            st.ledger.close_on_reply(in_reply_to, Instant::now())
        };
        match outcome {
            ReplyOutcome::Closed => {}
            ReplyOutcome::ClosedHistoryAnomaly { from } => tracing::warn!(
                in_reply_to,
                history_status = ?from,
                "회신으로 계약은 닫혔으나 이력이 Replied 로 전이 못 함(불법 간선) — 계약 정본은 추적, 이력만 미반영(ADR-0103)"
            ),
            ReplyOutcome::NoMatch => tracing::debug!(
                in_reply_to,
                "reply_to 가 오픈된 request 를 가리키지 않음 — 메시지는 정상 배달, 닫힌 계약 없음(엄격 매칭, spec §2)"
            ),
            ReplyOutcome::AlreadyClosed => tracing::debug!(
                in_reply_to,
                "이미 닫힌 request 에 대한 추가 회신 — 메시지는 정상 배달, no-op(spec §2)"
            ),
        }
    }

    /// 발송 3분기 본체(C1/C2) — `handle_single_send` 가 계약 예약/닫기로 감싸는 안쪽.
    ///
    /// 인자 `resolved`/`park_key` 는 호출자가 이미 계산해 넘긴다(계약 오픈이 recipient 를 먼저 알아야 하므로
    /// 로스터 해석을 밖으로 끌어올렸다 — 여기서 재조회하면 두 판정이 서로 다른 스냅샷을 보게 된다).
    #[allow(clippy::too_many_arguments)]
    fn dispatch_single(
        &self,
        msg_id: &str,
        from: BoundIdentity,
        sender_name: &str,
        to: &str,
        body: &str,
        entrance: Entrance,
        meta: &SendMeta,
        resolved: Option<LiveAgent>,
        park_key: &str,
    ) -> Result<SendOutcome, SendReject> {
        // 2) 부재 → 파킹(spec §5 분기 2). "없는 이름"도 파킹(스폰 전 선지시 지원, TTL 방어).
        let Some(target) = resolved else {
            let hint = format!(
                "No live reachable agent named '{to}' — parked; it will be delivered when that name appears (expires after TTL)."
            );
            // id 힌트 없음(부재 파킹) — 해석된 산 수신자가 없다(mailbox.rs `hinted_id` 주석).
            let outcome = self.park_pending(
                msg_id,
                sender_name,
                from,
                entrance,
                park_key,
                body,
                hint,
                None,
                ParkKind::Message,
                meta,
            )?;
            // ★park/appearance TOCTOU self-heal(finding 3)★: resolve↔park 사이 그 이름이 등장했으면 flush
            //   observer 는 빈 큐를 이미 flush 하고 지나가, 방금 park 한 메일이 다음 등장/TTL 까지 발이 묶인다.
            //   그래서 park 직후(락 해제 상태) 그 park_key 가 지금 유일 도달이면 즉시 flush 를 돌려 자가치유한다.
            //   drain 이 큐를 비우므로 flush observer 와 겹쳐도 idempotent(둘 다 drain — 한쪽이 빈 큐를 본다).
            self.self_heal_if_live(park_key);
            return Ok(outcome);
        };

        // 3) ★idle 게이트(C2 · ADR-0104 결정 3 · spec §5 분기 1 보정)★: 해석은 됐지만 수신자가 **턴 진행
        //    중**이면 주입하지 않고 파킹한다 — 상태 어휘는 부재 파킹과 **공유**(`pending`, 새 상태 발명 금지),
        //    hint 만 사유를 구분한다. 왜 CLI stdin 에 미리 밀지 않나: 턴 중 주입은 CLI 내부 큐로 들어가
        //    데몬 손을 떠나므로 ① `delivered` 장부가 "실제로 봤음" 과 어긋나고 ② 배치·순서 제어권을 잃는다
        //    (ADR-0104 거부 대안 "즉시 주입"). 쌓인 건 턴 종료(idle 전이) 때 **오래된 순 일괄** 주입된다.
        //    게이트 키 = (id, epoch) — 로스터가 준 현재 incarnation 축(busy.rs epoch 키 정합).
        if self.busy.is_busy(target.id, target.epoch) {
            let hint = format!(
                "'{}' is mid-turn — parked; it will be delivered as one batch when that turn ends.",
                target.name
            );
            let outcome = self.park_pending(
                msg_id,
                sender_name,
                from,
                entrance,
                &target.name,
                body,
                hint,
                // ★id 힌트(fix 2)★: 이 발송은 구체적 산 수신자를 해석했다 — 동명 다수여도(exact-id 지목은
                //   AMBIGUOUS 를 의도적으로 통과한다) 그 id 로 배달될 길을 flush 에 남긴다.
                Some(target.id),
                ParkKind::Message,
                meta,
            )?;
            // ★busy-park TOCTOU self-heal(C1 finding 3 와 대칭)★: 게이트 확인↔park 사이에 그 턴이 끝났으면
            //   (MessageDone) idle 트리거는 **빈 큐**를 이미 flush 하고 지나가, 방금 park 한 메일이 다음 턴
            //   종료·등장·TTL 까지 발이 묶인다(lost wakeup). 그래서 park 직후 도어벨을 **무조건** 누른다.
            // ★왜 무조건인가(fix 4 와의 분업 — load-bearing)★: 여기서 `!is_busy` 를 재확인해 걸러면 그
            //   확인과 소비 사이에 또 창이 생긴다. 대신 소비 측(flush 경로)이 drain 직전에 게이트를 한 번
            //   보므로(fix 4), 아직 턴 중이면 그쪽이 스킵하고 파킹을 유지한다 — 판정은 한 곳(소비 측)에서
            //   하고 여기선 **깨우기만** 한다. 잉여 도어벨의 대가는 빈/스킵 flush no-op 뿐이다.
            self.request_flush(target.id);
            return Ok(outcome);
        }

        // 3-b) ★FIFO 일관성(C2 리뷰 fix 5 · load-bearing)★: 수신자는 idle 인데 그 이름 앞에 **이미 파킹이
        //    쌓여 있으면** 직발송이 큐를 앞지른다 — 수신자가 보는 순서가 (새것, 옛것들) 로 뒤집힌다.
        //    "오래된 순 일괄" 계약(ADR-0104)은 큐 안에서만 성립하는 게 아니라 **그 수신자가 보는 순서**에
        //    대한 약속이라, 큐가 비어 있지 않으면 이 메시지도 큐 뒤에 붙이고 flush 를 눌러 한 배치로
        //    순서대로 나가게 한다. (큐가 비어 있으면 앞지를 대상이 없으므로 그대로 직발송 = C1 동작.)
        let has_queued = {
            let st = self.state.lock().expect("messaging state poisoned");
            st.mailbox.len(&target.name) > 0
        };
        if has_queued {
            let hint = format!(
                "'{}' has earlier queued messages — this one joins that queue and is delivered in order.",
                target.name
            );
            let outcome = self.park_pending(
                msg_id,
                sender_name,
                from,
                entrance,
                &target.name,
                body,
                hint,
                Some(target.id),
                ParkKind::Message,
                meta,
            )?;
            self.request_flush(target.id);
            return Ok(outcome);
        }

        // 4) idle + 큐 비어 있음 → 주입 시도. 봉투는 **주입 시점**에 현재 포맷으로 감싼다(단일 wrap point).
        //    ledger 는 주입 **후** 성공/실패에 따라 delivered/pending 을 찍는다(delivered=실제 주입, ADR-0104).
        let now = Instant::now();
        let wrapped = self.wrap_now(sender_name, msg_id, body, &meta.envelope_fields(msg_id));
        let inject_result = self.port.inject(target.id, wrapped.as_bytes());

        match inject_result {
            Ok(outcome) => {
                // ledger: pending 없이 곧장 delivered 로 기록(즉시 주입 — record(Delivered)).
                {
                    let mut st = self.state.lock().expect("messaging state poisoned");
                    st.ledger.record(
                        msg_id,
                        sender_name,
                        &target.name,
                        body,
                        DeliveryStatus::Delivered,
                        now,
                    );
                }
                // 관측 레코드(ADR-0088) — 락 밖에서 발행(registry.record_delivery 가 자체 규율).
                self.observe_success(msg_id, &target, from, entrance, &wrapped, &outcome);
                Ok(SendOutcome::Delivered)
            }
            Err(e) => {
                // 도달 불가/write 실패 → 파킹(spec §5 unreachable → 파킹). 관측 레코드는 실패로 남기고
                //   메시지는 유실하지 않는다(park + ledger pending). park 키 = canonical name(등장 flush 키와 정합).
                // ★self-heal 안 함(의도적 — finding 3 범위)★: finding 3 은 **부재→등장** 레이스(resolve 시점
                //   부재라 park 했으나 그 사이 등장)만 자가치유한다. inject 실패는 방금 그 incarnation 이
                //   도달 불가해진 것이라, 같은 roster 로 즉시 재-flush 하면 깨진 수신자에 재주입을 반복할 수
                //   있다(무한 재시도 위험). 실패분은 다음 **진짜** 등장(epoch bump)의 flush observer 에 맡긴다.
                self.observe_failure(msg_id, &target, from, entrance, &wrapped, &e);
                self.park_pending(
                    msg_id,
                    sender_name,
                    from,
                    entrance,
                    &target.name,
                    body,
                    format!(
                        "Delivery to '{}' failed ({e}) — parked; retried on next appearance (expires after TTL).",
                        target.name
                    ),
                    // 해석된 산 수신자가 있었으므로 힌트를 남긴다 — 그 id 가 여전히 로스터에 있으면(일시적
                    //   write 오류) 이름 유일성과 무관하게 그쪽으로 재시도되고, 죽었으면 이름 규칙으로 돌아간다.
                    Some(target.id),
                    ParkKind::Message,
                    meta,
                )
            }
        }
    }

    /// flush 도어벨을 누른다 — 배선돼 있으면 **다른 스레드**(flush lane)로 넘기고, 없으면 인라인 폴백.
    ///
    /// ★두 갈래의 근거는 `FlushTrigger` 주석★(운영 = 논블록 enqueue / 미배선 조립 = 인라인 flush).
    ///   인라인 폴백도 messaging 락 밖에서 불려야 한다 — 호출부는 모두 park 반환 후(락 해제) 지점이다.
    fn request_flush(&self, id: AgentId) {
        match &self.trigger {
            Some(t) => t.request_flush(id),
            None => self.flush_for_agent(id),
        }
    }

    /// 파킹 + ledger `pending` 기록(spec §5 분기 2·3 공통). cap 초과면 `MailboxFull` 반려(spec §5 분기 3).
    ///
    /// ★조용한 유실 금지(ADR-0103)★: park 성공 시 반드시 ledger 에 `pending` 레코드를 남긴다 — 파킹된
    ///   메시지가 장부 밖에 있으면 조회·감사에서 사라진다. cap 초과 반려는 애초에 저장 안 하므로 ledger 도
    ///   안 남긴다(발신자에게 반려로 즉시 가시화 — spec §5 "오래된 것 조용히 버리기 금지" 와 정합).
    /// ★락 규율★: 이 함수는 park+record 를 한 락 구간에서 하되(둘 다 순수 구조 조작, 외부 호출 없음) 그
    ///   구간에서 port 를 부르지 않는다.
    /// ★kind(C3)★: `Notice` 는 **cap 예외**라 이 함수가 절대 `MailboxFull` 을 내지 않는다(mailbox.rs `park`).
    ///   그래서 notice 호출부는 반환값을 무시해도 안전하다.
    /// ★meta(C3)★: 회신 계약 속성은 **payload 에 실려 flush 까지 살아남는다** — 늦게 배달되는 request/회신도
    ///   즉시 배달과 **같은 봉투**(같은 속성)로 나가야 하기 때문이다(park 시점에 봉투를 굳히지 않는 설계라
    ///   속성 재료를 함께 날라야 한다 — 모듈 헤더 "봉투 = 주입 시점 조립").
    #[allow(clippy::too_many_arguments)]
    fn park_pending(
        &self,
        msg_id: &str,
        sender_name: &str,
        from: BoundIdentity,
        entrance: Entrance,
        recipient: &str,
        body: &str,
        hint: String,
        hinted_id: Option<AgentId>,
        kind: ParkKind,
        meta: &SendMeta,
    ) -> Result<SendOutcome, SendReject> {
        let now = Instant::now();
        let mut st = self.state.lock().expect("messaging state poisoned");
        // park 는 raw body + 봉투/관측에 필요한 최소 메타(sender 이름·발신자 신원·입구·회신 계약)를
        //   나른다(봉투는 flush 주입 시점 조립 — 단일 wrap point). ParkedMessage.envelope 계약이 "완성 봉투"라
        //   여기선 ParkPayload 로 인코딩해 그 문자열 슬롯에 실어 보관하고, flush 때 decode 한다.
        let payload = ParkPayload {
            sender_name: sender_name.to_string(),
            from,
            entrance,
            body: body.to_string(),
            meta: meta.clone(),
        };
        let parked = ParkedMessage {
            msg_id: msg_id.to_string(),
            envelope: payload.encode(),
            kind,
            parked_at: now,
            // admission 순번은 `park` 이 수용 시점에 부여한다(저장소가 유일 부여자 — mailbox 주석). 여기 값은
            //   무시되므로 placeholder.
            admission_seq: 0,
            hinted_id,
        };
        match st.mailbox.park(recipient, parked) {
            Ok(()) => {
                st.ledger.record(
                    msg_id,
                    sender_name,
                    recipient,
                    body,
                    DeliveryStatus::Pending,
                    now,
                );
                Ok(SendOutcome::Parked { hint })
            }
            Err(ParkError::MailboxFull) => Err(SendReject::MailboxFull),
        }
    }

    /// ★등장/epoch flush(ADR-0104 — C1)★: 수신자 이름의 파킹분을 **오래된 순 일괄** 주입한다. 데몬측
    ///   로스터 diff(flush observer)가 newly-live/epoch-bump 를 감지해 그 이름들로 부른다.
    ///
    /// ★순서 보장 범위(finding 8)★: "오래된 순" 은 **이 배치 내부** 보장이다(재파킹 front-restore 로 배치 간
    ///   이월 시에도 오래된 것 우선). 동시 직발송(handle_single_send)이나 다른 flush 호출과의 전역 순서는
    ///   보장하지 않는다 — 모듈 헤더 "순서 보장의 범위" 참조(accepted trade-off).
    ///
    /// 동작(1~4가 **한 락 구간**, 5만 락 밖 — 아래 "미배달분은 큐를 떠나지 않는다" 참조):
    ///   1. 락 잡고 `mailbox.drain(recipient)` → deliverable(미만료, 오래된 순) + expired(만료).
    ///   2. expired → ledger `pending→expired`(장부 잔존 — 순수 조작).
    ///   3. deliverable 을 **해석된 타깃별로 분할**(항목별 id 힌트 우선 → 이름 유일 도달 규칙).
    ///   4. 타깃별 **게이트 1회** → busy 타깃 몫·배달 경로 없는 몫은 **그 자리에서 원래 순서로 복원**. **락 해제.**
    ///   5. 배달할 몫만 각각 **개별 봉투**로 감싸 순서대로 inject(락 밖). 성공 → ledger `pending→delivered`.
    ///      ★부분 실패 무손실(load-bearing)★: 배치 도중 inject 실패(drain 후 수신자 사망)면 **그 타깃의 남은
    ///      몫(실패분 포함)을 `restore_ordered` 로 되돌린다**(cap 우회 + admission 순번 merge = 무손실·순서
    ///      보존). 다른 타깃 몫은 계속 배달한다(조용한 유실 금지).
    ///
    /// ★왜 to_id 를 인자로 받나(그리고 왜 execution 시점에 재검증하나 — finding 2)★: flush observer/self-heal
    ///   이 로스터 스냅샷에서 (이름→현재 id) 를 알고 부르지만, 그 스냅샷은 **enqueue 시점** 것이라
    ///   execution(여기)까지 사이에 stale 해질 수 있다 — ① 동명 두 번째 에이전트가 등장해 이름이 ambiguous
    ///   해졌거나 ② 그 수신자가 죽었을 수 있다. 그래서 drain 직전 **현재 로스터**로 이름을 재해석해:
    ///   그 이름이 **정확히 1개** 도달 후보로 풀리면 그 id 로 주입한다(등장 사이 epoch/incarnation 이
    ///   바뀌었어도 현재 산 것으로). ambiguous·부재면 항목별 id 힌트(fix 2)로 한 번 더 시도하고, 그것도
    ///   없으면 skip(파킹 유지 — 그 이름이 다시 유일해지거나 TTL 로 만료될 때까지 큐에 남는다).
    ///   인자 `to_id` 는 이제 **호출자가 믿었던 stale 후보**로 로그에만 쓴다(권위는 execution 시점 재해석).
    ///   ★uniqueness 로직은 self_heal_if_live 와 공유★(`unique_reachable_in`) — 이름-키 파킹의 동명 정책을
    ///   한 곳에서 판정.
    ///
    /// ★게이트는 **타깃별로 딱 한 번** 본다(C2 리뷰 fix 4 + round-3 finding 3 · load-bearing)★:
    ///   - **타깃 1개당 첫 주입 전 1회 확인**: 그 수신자가 지금 턴 중이면 **그 타깃 몫을** 원래 순서로
    ///     되돌린다(파킹 유지 — 그 턴의 MessageDone 이 다시 트리거를 낸다). 등장 flush 가 프라이밍 턴
    ///     중간에 떨어지는 경우 등에서 "턴 중 주입" 을 막는 장치다.
    ///   - **왜 "첫 항목만" 이 아니라 "타깃별" 인가(round-3 finding 3)★: 항목별 타깃은 id 힌트(fix 2)에
    ///     따라 갈리므로 **한 이름-키 배치가 서로 다른 에이전트로 쪼개질 수 있다**(동명 충돌 + 힌트 혼재).
    ///     첫 항목만 검사하면 `[A(idle) 힌트, B(busy) 힌트]` 배치에서 B 에게 **턴 중 주입**이 된다 — 게이트를
    ///     통째로 우회하는 구멍이었다. 그래서 drain 결과를 **해석된 타깃별로 분할**하고 각 타깃에 1회씩
    ///     게이트를 적용한다(busy 타깃은 파킹 유지, idle 타깃은 오래된 순 배달).
    ///   - **mid-batch 재검사 금지(의도적 — 타깃 안에서는 불변)**: 배치의 첫 주입이 수신자 측 "입력 시점
    ///     유저 에코"(claude 는 이걸 `Structured` 로 낸다 = tap 이 busy 로 관측)를 즉시 발생시키므로, 항목마다
    ///     게이트를 보면 **배치가 1건 만에 중단**된다(= 드리블 주입 = ADR-0104 거부 대안). 한 타깃의 배치를
    ///     시작했으면 그 타깃 몫은 끝까지 민다.
    ///   - **왜 drain 전이 아니라 drain 후인가**: 항목별 타깃은 id 힌트에 따라 달라져 **drain 하기 전엔 알
    ///     수 없다**. 이름으로만 게이트하면 힌트로 배달되는 경로가 게이트를 우회한다. drain 후 복원은
    ///     같은 락 구간 안이고 무손실·순서 보존이므로(restore_ordered) 외부에 관측 가능한 차이가 없다.
    ///   게이트가 안전한 전제는 tap 이 **live-only** 라는 것이다(busy.rs fix 1) — busy = 지금 진행 중인
    ///   실제 턴이므로 그 종료 통지가 반드시 온다(과거 transcript 로 인한 깨울 수 없는 busy 없음). 그 통지가
    ///   유실되는 비정상 턴은 `BUSY_MAX_TURN` 상한 sweep 이 fail-open 으로 깨운다(busy.rs).
    /// ★미배달분은 **큐를 떠나지 않는다**(락 원자성 — load-bearing)★: drain·타깃 분할·게이트·스킵분 복원을
    ///   **한 락 구간**에서 끝내고, 락 밖으로는 **배달할 항목만** 들고 나간다. 예전엔 drain 후 락을 놓고
    ///   게이트를 본 뒤 다시 락을 잡아 복원했는데, 그 사이 큐가 **비어 보이는 창**이 생겨 ① 동시 직발송의
    ///   FIFO 합류 검사(`mailbox.len() > 0`)가 큐를 비었다고 보고 즉시 주입해 되돌려질 옛 메일을 앞지르고
    ///   ② 관측자가 파킹을 놓쳤다. 게이트를 락 안에서 부르는 근거는 `BusyGate` 계약(순수 조회·짧은 락·
    ///   messaging 락 보유 중 호출 안전)이고, **DeliveryPort 는 여전히 전부 락 밖**이다.
    /// ★복원 순서(round-4 finding 1)★: 한 flush 는 같은 이름 큐에 재파킹을 **여러 번** 부를 수 있다 — 락 안
    ///   스킵분 1회 + 락 밖 타깃별 실패분 n회. 그래서 `restore_ordered`(admission 순번 merge)로 되돌린다:
    ///   호출 횟수·순서와 무관하게 큐가 항상 전역 오래된 순을 유지한다. 락 안 스킵분은 인덱스를 정렬해
    ///   순번 오름차순 계약을 지켜 넘긴다(옛 FRONT 삽입은 두 번째 호출이 첫 호출 앞에 꽂혀 나이 순서가
    ///   뒤집혔다 — 그 역전이 sweep 의 만료 항목 은폐로도 번졌다).
    /// ★수용된 잔여(residual)★: 배치 도중 수신자가 **새 턴을 스스로 시작**하면 남은 주입은 CLI 내부 stdin
    ///   큐로 들어간다(유실 없음, "언제 읽히나" 만 흐려짐 — spec §7 미검증 항목).
    pub fn flush_for(&self, recipient: &str, to_id: AgentId) {
        // ★execution-time 재해석(finding 2)★: enqueue 시점 (name,id) 는 stale 가능 — 지금 로스터로 재확인.
        //   port 호출이라 messaging 락 밖(모듈 헤더 규율). ★로스터 스냅샷은 1회만 뜬다★ — 이름 유일성
        //   판정과 아래 id-힌트 생존 판정이 **같은 스냅샷**을 봐야 배치 안에서 판정이 흔들리지 않는다.
        let roster = self.port.live_reachable_agents();
        let name_target = unique_reachable_in(&roster, recipient);

        let now = Instant::now();
        // ★락 밖에서 로깅할 사실(finding 3)★: 아래 락 구간에서 **수집만** 하고, 락을 놓은 뒤 찍는다.
        let mut no_target_kept = 0usize;
        let mut busy_skipped: Vec<(AgentId, u32, usize)> = Vec::new();
        // 1~4) 드레인 + 만료 장부화 + 타깃 분할 + 게이트 + **미배달분 즉시 복원** — 전부 **한 락 구간**.
        //
        // ★락 보유 중 tracing 금지 — 수집 후 락 밖 로깅(finding 3)★: 동기 포맷팅 subscriber 는 stdout 락에
        //   걸릴 수 있어, 크리티컬 섹션 안에서 찍으면 그 지연이 메시징 락 대기로 번진다.
        //
        // ★왜 게이트·복원을 락 안에서 하나(load-bearing — 관측 가능한 빈 큐 창 제거)★: 예전엔 drain(락) →
        //   락 해제 → 게이트 → 다시 락 → 복원 순서였다. 그 사이 큐는 **비어 보인다** — 그런데 배달되지도
        //   않을(busy 라 곧 되돌릴) 항목까지 사라진 것처럼 보이는 창이다. 그 창에서 직발송이 들어오면
        //   `handle_single_send` 의 FIFO 합류 검사(`mailbox.len() > 0`)가 "큐 비었음" 으로 보고 **즉시 주입**해,
        //   되돌려질 옛 메일을 앞지른다(수신자가 보는 순서 역전). 관측자(테스트·통계)도 파킹을 놓친다.
        //   그래서 "배달할 것만 큐에서 나가고, 안 나갈 것은 애초에 큐를 떠나지 않는다" 를 락으로 원자화한다.
        // ★락 안에서 게이트를 부르는 게 규율 위반이 아닌 이유★: `BusyGate` 는 **순수 조회 + 짧은 락**이며
        //   "messaging 락을 든 채 불려도 안전" 을 계약으로 못 박은 seam 이다(busy.rs `BusyGate` 주석). 역방향
        //   (busy 락 → messaging 락) 경로는 존재하지 않는다(tap 은 논블록 채널 send 만 한다) → 락 순서 역전 없음.
        //   금지 대상은 **DeliveryPort(inject/roster)** 다 — 그건 여전히 전부 락 밖이다(아래 5단계).
        let groups: Vec<(LiveAgent, Vec<ParkedMessage>)> = {
            let mut st = self.state.lock().expect("messaging state poisoned");
            let drained = st.mailbox.drain(recipient, now);
            for ex in &drained.expired {
                // pending → expired(장부 잔존). NotFound/Illegal 은 best-effort(레코드 evict 등) — 무시.
                let _ = st
                    .ledger
                    .transition(&ex.msg_id, recipient, DeliveryStatus::Expired, now);
            }
            let deliverable = drained.deliverable;
            if deliverable.is_empty() {
                return;
            }

            // ★항목별 타깃 해석 → 타깃별 분할(round-3 finding 3)★: ① park 시 해석돼 있던 id 힌트가 **아직
            //   로스터에 살아 있으면** 이름 유일성과 무관하게 그쪽으로 배달한다(exact-id 지목이 동명 다수
            //   때문에 TTL 까지 blackhole 되는 걸 막는다 — fix 2) → ② 힌트가 없거나 죽었으면 이름 유일 도달
            //   규칙(respawn 이 파킹을 이어받는 이름-키 설계 — canonical_park_key 주석).
            //   그룹은 **등장 순서**대로, 그룹 안 인덱스도 **오래된 순**이라 배달 순서가 보존된다.
            let mut groups: Vec<(LiveAgent, Vec<usize>)> = Vec::new();
            let mut restore: Vec<usize> = Vec::new();
            for (idx, parked) in deliverable.iter().enumerate() {
                let target = parked
                    .hinted_id
                    .and_then(|h| roster.iter().find(|a| a.id == h).cloned())
                    .or_else(|| name_target.clone());
                match target {
                    Some(t) => match groups.iter_mut().find(|(g, _)| g.id == t.id) {
                        Some((_, idxs)) => idxs.push(idx),
                        None => groups.push((t, vec![idx])),
                    },
                    // 배달 경로 없음(이름이 부재/동명 다수 + 힌트도 사망) → 파킹 유지.
                    None => restore.push(idx),
                }
            }
            // 로깅은 락 밖에서(finding 3) — 여기선 사실만 센다.
            if !restore.is_empty() {
                no_target_kept = restore.len();
            }

            // ★flush 게이트(fix 4 + finding 3) — 타깃당 정확히 1회★. 그 타깃 배치가 시작된 뒤엔 절대 다시
            //   보지 않는다(위 doc: 첫 주입의 유저 에코가 busy 를 만들어 배치를 1건에서 끊는다).
            let mut deliver: Vec<(LiveAgent, Vec<usize>)> = Vec::new();
            for (target, idxs) in groups {
                if self.busy.is_busy(target.id, target.epoch) {
                    // 로깅은 락 밖에서(finding 3) — 여기선 사실만 모은다.
                    busy_skipped.push((target.id, target.epoch, idxs.len()));
                    restore.extend(idxs);
                    continue;
                }
                deliver.push((target, idxs));
            }

            // ★복원은 원래 순서로 한 번에★: 인덱스를 정렬해 오래된 순(= admission 순번 오름차순)을 복구한 뒤
            //   되돌린다 — `restore_ordered` 의 호출자 계약이 "items 는 순번 오름차순" 이다.
            if !restore.is_empty() {
                restore.sort_unstable();
                let items: Vec<ParkedMessage> = restore
                    .iter()
                    .map(|&idx| deliverable[idx].clone())
                    .collect();
                st.mailbox.restore_ordered(recipient, items);
            }
            // 배달 대상만 소유권을 들고 락 밖으로 나간다(인덱스 → 항목).
            deliver
                .into_iter()
                .map(|(t, idxs)| {
                    let items = idxs.iter().map(|&i| deliverable[i].clone()).collect();
                    (t, items)
                })
                .collect()
        };

        // ★락 해제 후 로깅(finding 3)★ — 위에서 모은 사실만 찍는다(포맷팅·stdout 대기가 락 밖이다).
        if no_target_kept > 0 {
            tracing::debug!(
                recipient,
                stale_id = %to_id,
                keeping = no_target_kept,
                "flush skip: execution 시점 이름이 유일 도달 아님(부재/동명 다수)이고 id 힌트도 사망 — 파킹 유지(finding 2)"
            );
        }
        for (agent_id, epoch, parked) in &busy_skipped {
            tracing::debug!(
                recipient,
                agent = %agent_id,
                epoch,
                parked,
                "flush skip: 수신자가 턴 진행 중 — 그 타깃 몫 미시작(파킹 유지, 턴 종료 통지가 재시도)"
            );
        }

        // 5) 타깃별 배달(★락 밖★ — inject 는 자식 stdin blocking write). 실패면 그 타깃의 남은 몫만 되돌린다.
        for (target, items) in &groups {
            for (n, parked) in items.iter().enumerate() {
                let to_id = target.id;
                let payload = ParkPayload::decode(&parked.envelope);
                // ★늦은 배달도 같은 봉투(C3 — load-bearing)★: 파킹된 항목의 종류·계약 속성을 그대로 되살려
                //   감싼다. notice 는 `<notice>`(from 없음), request/회신은 자기 속성(id/type/reply-by/
                //   in-reply-to)이 붙은 `<message>` 다 — 즉시 배달과 flush 배달의 봉투가 갈리면 수신 LLM 이
                //   같은 메시지를 다르게 읽는다(회신 불가한 request 등).
                let wrapped = match parked.kind {
                    ParkKind::Notice => wrap_notice(&payload.body),
                    ParkKind::Message => self.wrap_now(
                        &payload.sender_name,
                        &parked.msg_id,
                        &payload.body,
                        &payload.meta.envelope_fields(&parked.msg_id),
                    ),
                };
                match self.port.inject(to_id, wrapped.as_bytes()) {
                    Ok(outcome) => {
                        {
                            let mut st = self.state.lock().expect("messaging state poisoned");
                            // pending → delivered(실제 주입 시점, ADR-0104).
                            let _ = st.ledger.transition(
                                &parked.msg_id,
                                recipient,
                                DeliveryStatus::Delivered,
                                Instant::now(),
                            );
                        }
                        // 등장 배달도 배달 경계 관측(ADR-0088) — handle_single_send 와 동일하게 발행(락 밖).
                        //   원 발신자 신원·입구는 파킹 payload 에서 복원(파킹→flush 자동배달 acceptance, spec §7).
                        // ★to_name = park 키(recipient)★: 하네스가 "어느 이름 앞 파킹이 배달됐나" 로 회수하므로
                        //   해석된 타깃의 로스터 이름이 아니라 파킹 키를 싣는다(둘은 정상 경로에서 동일하다).
                        //   epoch 은 write 가 실제로 착지한 incarnation 값(outcome.epoch — 로스터 스냅샷이 아님).
                        let observed_target = LiveAgent {
                            id: to_id,
                            name: recipient.to_string(),
                            epoch: outcome.epoch,
                        };
                        self.observe_success(
                            &parked.msg_id,
                            &observed_target,
                            payload.from,
                            payload.entrance,
                            &wrapped,
                            &outcome,
                        );
                    }
                    Err(_e) => {
                        // ★부분 실패 무손실(load-bearing — finding 1, ADR-0103/0104)★: 수신자가 drain↔inject
                        //   사이 죽었다 — 그 타깃의 남은 몫(실패분 포함)을 되돌린다. 왜 `park` 가 아니라
                        //   `restore_ordered` 인가:
                        //     ① cap 우회 — drain↔inject 사이 **동시 park** 가 큐를 다시 cap 까지 채웠으면 park 는
                        //        MailboxFull 로 반려한다. 그 에러를 무시하면 admitted 메시지가 조용히 유실된다
                        //        (ledger 는 pending 인데 큐엔 없음 — 유령 pending). restore_ordered 는 cap 을 세지
                        //        않아 무조건 되돌린다(cap 은 유입 통제지 보관 통제가 아님 — mailbox 주석).
                        //     ② admission 순번 merge — 재파킹분(더 오래됨)이 동시 park 된 신규분(더 최근)보다
                        //        앞서야 "오래된 순" 이 안 깨진다. 단순 앞쪽 삽입이 아니라 merge 인 이유는
                        //        **이 루프가 타깃마다 따로 복원**하기 때문이다(round-4 finding 1): 두 타깃이
                        //        모두 실패하면 두 번째 복원이 첫 번째 앞에 꽂혀 그룹 간 나이 순서가 뒤집힌다.
                        //        merge 는 몇 번 불려도 전역 오래된 순을 유지한다(mailbox 주석).
                        //   parked_at 은 clone 으로 원래 값 유지 — TTL 연장 없음(오배송 방어). ledger 는 이미
                        //   pending 이라 전이 불요(재파킹 = pending 유지). 다음 등장에 재시도.
                        //   ★다른 타깃 그룹은 계속 진행한다★: 이 실패는 **이 수신자** 의 도달 불가라 다른
                        //   에이전트의 배달을 막을 근거가 없다(막으면 남은 그룹이 근거 없이 지연된다).
                        //   ★순서는 타깃 내부·타깃 간 모두 보존된다(round-4 finding 1)★: 복원이 admission 순번
                        //   merge 라 이 루프가 타깃마다 따로 되돌려도 큐는 전역 오래된 순을 유지한다 — 그룹 간
                        //   상대 순서도 무의미하지 않다: 큐 앞머리는 `handle_single_send` 의 FIFO 합류 판정과
                        //   다음 배치의 배달 순서를 결정하고, 나이 역전은 sweep 이 만료 항목을 지나치게 만든다.
                        let remaining: Vec<ParkedMessage> = items[n..].to_vec();
                        let remaining_count = remaining.len();
                        {
                            // ★락 보유 중 tracing 금지(finding 3)★ — 복원만 하고 즉시 락을 놓은 뒤 로깅한다.
                            let mut st = self.state.lock().expect("messaging state poisoned");
                            st.mailbox.restore_ordered(recipient, remaining);
                        }
                        tracing::warn!(
                            recipient,
                            agent = %to_id,
                            remaining = remaining_count,
                            "메시지 flush 중 inject 실패 — 그 타깃의 남은 배치 재파킹(무손실 restore_ordered, ADR-0103/0104)"
                        );
                        break;
                    }
                }
            }
        }
    }

    /// ★턴 종료(idle 전이) flush(C2 · ADR-0104 결정 3)★: **id 로** 지목된 flush — 그 에이전트의 canonical
    ///   이름을 풀어 `flush_for` 에 위임한다. 왜 id 입구가 따로 있나: 턴 관측 tap 은 출력 스트림에 붙어
    ///   있어 **id/epoch 만 안다**(이름을 모른다 — 이름은 프로필·cwd 파생이라 core 출력 경계에 없다).
    ///   반면 파킹은 이름-키다(respawn 생존 — canonical_park_key 주석). 그 간극을 여기서 한 번 메운다:
    ///   id → canonical name → 기존 flush 경로(경로 2벌 금지 — ADR-0104 "flush = 일괄·오래된 순" 공유).
    ///
    /// ★이름이 턴 중에 바뀌면 옛 이름 큐가 고아가 된다 — 힌트로 함께 찾는다(round-3 finding 2)★: busy
    ///   파킹은 **발송 시점의** canonical 이름 큐에 들어가지만, 이 입구는 **현재** 이름으로만 큐를 연다.
    ///   그 사이 이름이 바뀌면(display_name 변경 등) 옛 이름 큐를 아무도 열지 않아 그 배치가 TTL 까지
    ///   stranded 된다(배달 안 됨 = 최악 실패 모드). 그래서 ① 현재 이름 큐 ② **이 id 를 힌트로 든 항목이
    ///   있는 큐**(`mailbox.queues_with_hint` — 비용 근거는 그 주석) 둘 다 flush 한다. 항목별 힌트-우선
    ///   해석은 `flush_for` 가 이미 하므로, 여기서 필요한 건 "어느 큐를 열까" 뿐이다.
    /// ★빈 큐 조기 반환(비용 절감 — 잉여 통지 idempotency 의 실효 비용을 여기서 깎는다)★: idle 통지는
    ///   **턴마다** 온다(busy.rs `IdleNotifier` 주석 — 누락보다 잉여를 택한 설계). 그 대부분은 파킹이 없는
    ///   no-op 이므로, 로스터 스냅샷(list_agents = 전 세션 순회)을 돌리기 **전에** 짧은 락으로 열 큐를
    ///   고르고, 하나도 없으면 즉시 빠진다.
    /// ★락 규율★: canonical_name·flush_for 는 port 호출이라 락 밖. 대상 큐 선정만 짧게 락을 잡는다.
    pub fn flush_for_agent(&self, to_id: AgentId) {
        let Some(name) = self.port.canonical_name(to_id) else {
            // 이미 사라진 id(reap 완료) — 그 이름 앞 파킹은 다음 등장의 로스터 diff 가 잡는다. 힌트 큐도
            //   보지 않는다(그 id 로는 배달할 수 없으니 지금 열어 봐야 복원만 하고 끝난다).
            return;
        };
        let targets: Vec<String> = {
            let st = self.state.lock().expect("messaging state poisoned");
            let mut t: Vec<String> = Vec::new();
            if st.mailbox.len(&name) > 0 {
                t.push(name);
            }
            for key in st.mailbox.queues_with_hint(to_id) {
                if !t.contains(&key) {
                    t.push(key);
                }
            }
            t
        };
        for key in targets {
            self.flush_for(&key, to_id);
        }
    }

    /// ★park 키 정규화(finding 4)★: `to` 가 exact AgentId 문자열이고 그 에이전트가 존재하면(도달 여부 무관)
    ///   그 canonical **NAME** 을 park 키로 돌려준다 — 등장 flush 가 canonical name 으로 keyed 이기 때문
    ///   (UUID 로 park 하면 name-keyed flush 가 영영 못 잡는다). 파싱 실패/미존재면 리터럴 문자열 그대로.
    /// ★락 밖 호출★: canonical_name 은 port 호출(내부 락)이라 messaging 락 밖에서만 부른다(모듈 헤더 규율).
    ///
    /// ★파킹은 이름-키가 설계다(finding 4 · ADR-0101/0103)★: 여기서 exact-AgentId 지목조차 canonical NAME
    ///   으로 park 키를 바꾼다는 건, 파킹의 **주소 단위가 이름**이라는 뜻이다(id 가 아니라). 왜:
    ///   - **WYSIWYA 이름 주소(ADR-0101)**: 에이전트가 서로를 부르는 1급 주소는 표시 이름이다. exact-id 는
    ///     그걸 해석하는 send-time **편의**일 뿐, 파킹의 정체성은 이름이다.
    ///   - **respawn 생존(ADR-0103)**: 파킹의 존재 이유가 "지금 없는(또는 곧 죽었다 다시 뜰) 이름 앞으로 미리
    ///     쌓아 둠" 이다. 재스폰된 에이전트는 **새 AgentId** 를 얻으므로, id 로 park 하면 재스폰분이 자기 앞
    ///     파킹을 절대 못 받는다 — 이름으로 park 해야 새 incarnation 이 등장 flush 로 이어받는다.
    ///
    /// 함의(수용된 잔여): exact-AgentId 로 보내 파킹된 메일이, 나중에 **같은 이름의 다른 에이전트**가 유일
    ///   도달이 되면 그쪽으로 배달될 수 있다. 이건 이름 주소의 accepted residual 이며, uniqueness-게이트
    ///   flush/ambiguity 정책(동명 다수면 배달 보류 — finding 2/3, `unique_reachable_in`)과 일관된다:
    ///   이름이 유일하게 풀릴 때만 배달하므로, "그 이름을 쓰는 지금 유일한 에이전트" 로 배달된다는 계약이 유지된다.
    ///
    /// ★그러나 이름-키 **단독**이면 exact-id 발송에 blackhole 이 생긴다(C2 리뷰 fix 2)★: exact-AgentId
    ///   지목은 발송 단계에서 동명 모호성을 **의도적으로 통과**한다(id 가 명시적 승자 — ingress). 그런데
    ///   그 수신자가 턴 중이라 이름-키로 park 되면, 동명이 둘인 동안 flush 의 유일성 게이트가 영영 보류해
    ///   TTL 만료까지 배달되지 않는다. 그래서 park 항목은 해석된 id 를 **힌트**로 함께 들고 다니고
    ///   (`ParkedMessage.hinted_id`), flush 는 **힌트가 아직 살아 있으면 이름 유일성과 무관하게** 그쪽으로
    ///   배달한다(힌트가 죽었으면 위의 이름 규칙으로 복귀 — 재스폰 이어받기 유지). park **키**는 여전히
    ///   이름이다(힌트는 배달 우선순위일 뿐 주소 축이 아니다).
    fn canonical_park_key(&self, to: &str) -> String {
        if let Ok(id) = to.parse::<AgentId>() {
            if let Some(name) = self.port.canonical_name(id) {
                return name;
            }
        }
        to.to_string()
    }

    /// ★park/appearance TOCTOU 자가치유(finding 3)★: 방금 park 한 이름이 **지금** 유일 도달(live+structured)
    ///   이면 즉시 flush 를 돌린다. resolve↔park 사이 수신자가 등장해 flush observer 가 빈 큐를 이미 지나친
    ///   경우(lost wakeup)를 스스로 메운다. drain 이 큐를 비우므로 observer 와 겹쳐도 idempotent(둘 중 하나가
    ///   빈 큐를 본다). 동명 다수면 flush 안 함(finding 2 정합 — 이름이 다시 유일해질 때 flush observer 가 잡음).
    /// ★락 밖 호출★: roster 조회·flush_for 는 port 호출이라 messaging 락 밖에서만(park_pending 반환 후) 부른다.
    fn self_heal_if_live(&self, park_key: &str) {
        // 유일 도달일 때만 self-heal(uniqueness 판정은 flush_for 와 공유 — unique_reachable_in).
        //   유일치 않으면 도어벨 자체를 아낀다(그 이름이 다시 유일해질 때 등장 flush 가 잡는다).
        if let Some(a) = self.resolve_unique_reachable(park_key) {
            // 정확히 1개 — 유일 도달. 그 id 로 flush 도어벨(배치 write 를 발신 스레드에서 떼어낸다 — fix 11).
            self.request_flush(a.id);
        }
    }

    /// ★유일 도달 재해석(finding 2/3 공유)★: 이름을 **현재** 로스터에 대고 풀어, 그 이름의 도달 후보가
    ///   **정확히 1개**면 그 LiveAgent 를 돌려주고, 0개(부재)·2개+(동명 다수)면 None. flush_for(execution
    ///   시점 stale-authority 재검증)와 self_heal_if_live(park 직후 등장 확인)가 같은 동명 정책을 쓰도록
    ///   판정 로직을 `unique_reachable_in` 한 곳에 모은다 — send-side RECIPIENT_AMBIGUOUS 와 일관(동명
    ///   다수는 배달하지 않고 파킹 유지).
    /// ★락 밖 호출★: live_reachable_agents 는 port 호출이라 messaging 락 밖에서만 부른다(모듈 헤더 규율).
    fn resolve_unique_reachable(&self, name: &str) -> Option<LiveAgent> {
        unique_reachable_in(&self.port.live_reachable_agents(), name)
    }

    /// ★TTL sweep + 회신 기한 초과 통지(spec §5·§3 — C1/C3)★: 주기 task(lib.rs)가 부르는 단일 유지보수 진입점.
    ///   ① 전 수신자 만료 파킹분을 걷어 ledger `pending→expired` 로 남기고(장부 잔존)
    ///   ② 기한 초과된 미회신 request 를 걷어 **발신자에게** `<notice>` 를 배달한다(spec §3 단계 4).
    ///
    /// ★notice 는 수신자 재촉이 아니다★: 통지는 요청을 **건 쪽**에게 간다 — 재촉할지 포기할지는 발신 LLM 의
    ///   판단이라(ADR-0103 결정 3) 데몬은 사실만 알린다.
    /// ★정확히 1회(spec §7)★: `due_timeouts` 가 반환하며 `notified` 를 세우므로 같은 request 는 다음 sweep 에
    ///   다시 나오지 않는다 — 이중 통지 방지 책임은 장부에 있고 여기선 걷은 것만 배달한다.
    /// ★논블록(load-bearing — sweep task 보호)★: 이 함수는 **주입하지 않는다**. notice 는 파킹 + 도어벨까지만
    ///   하고(`deliver_notice` 주석) 실제 배치 write 는 flush 레인이 한다 — 이 함수는 60초 주기 tokio task 에서
    ///   돌고, 그 task 는 busy 상한 sweep(멈춘 턴의 fail-open 깨우기)도 함께 돌리므로 여기서 자식 stdin
    ///   blocking write 를 하면 그 안전장치까지 같이 멈춘다.
    /// ★락 규율(load-bearing)★: 만료 전이·due 산출은 **한 락 구간**(순수 조작)에서 끝내고, notice 파킹은
    ///   **락을 놓은 뒤** 한다 — 파킹 전 로스터 조회가 DeliveryPort 호출이라 락 안에서 부르면 모듈 헤더의
    ///   절대 규율(messaging 락 보유 중 port 금지)을 어겨 락 순서 역전 위험이 생긴다.
    /// ★로스터는 **틱당 한 번**만 뜬다(리뷰 fix 8)★: 예전엔 due 항목마다 `live_reachable_agents`(= 전 세션
    ///   순회)를 다시 불러, 한 틱 안에서 항목별로 **다른 스냅샷**을 보고 판정했다(같은 틱인데 앞 항목은
    ///   배달, 뒤 항목은 부재로 갈릴 수 있었다). 한 번 떠서 전 항목에 같은 스냅샷을 쓴다 — 비용도 O(due)
    ///   에서 O(1) 로 준다. due 가 비면 아예 뜨지 않는다(대부분의 틱).
    pub fn sweep(&self, now: Instant) {
        let due: Vec<DueTimeout> = {
            let mut st = self.state.lock().expect("messaging state poisoned");
            let expired = st.mailbox.sweep_expired(now);
            for ex in expired {
                // 파킹은 어느 수신자 큐에 있었는지 sweep 반환에 없다 — ledger 레코드는 (msg_id, recipient) 키라
                //   recipient 를 알아야 전이한다. sweep_expired 는 recipient 를 안 돌려주므로, msg_id 로 레코드를
                //   찾아 그 to 로 전이한다(records_for → 첫 pending 레코드). 조용한 유실 금지.
                transition_expired_by_msg_id(&mut st.ledger, &ex.msg_id, now);
            }
            st.ledger.due_timeouts(now)
        };
        if due.is_empty() {
            return;
        }
        // 락 밖 · 틱당 1회 스냅샷(위 주석). 아래 전 항목이 이 한 장으로 판정한다.
        let roster = self.port.live_reachable_agents();
        for d in due {
            // spec §1 notice 템플릿(한국어 계약 — 문구를 바꾸면 프라이밍·수용 기준과 어긋난다).
            //   기한 표기는 **발신자가 쓴 원본**을 그대로 쓴다(fix 6 — 봉투 `reply-by` 와 어긋나지 않게).
            let body = format!(
                "요청 {} 기한({}) 초과 — {} 회신 없음",
                d.request_id, d.reply_by_raw, d.recipient
            );
            self.deliver_notice(&d, &body, &roster);
        }
    }

    /// ★`<notice>` 배달(C3)★ — 데몬 통지를 한 수신자에게 보낸다. **항상 파킹 + 도어벨**이다.
    ///
    /// ★왜 여기서 직접 inject 하지 않나(load-bearing — sweep task 보호)★: 이 함수의 유일한 호출자는
    ///   `sweep` 이고, sweep 은 데몬 수명 동안 도는 **tokio task** 에서 60초마다 돈다(lib.rs). `inject` 는
    ///   자식 stdin 의 **blocking write** 라, 여기서 직접 부르면 한 수신자의 막힌 파이프가 sweep task 를
    ///   통째로 잡아먹는다 — TTL 만료 처리와 **busy 상한 sweep**(멈춘 턴의 fail-open 깨우기)까지 함께
    ///   멈춘다. 그래서 `FlushTrigger`(fix 11)와 **같은 규율**을 따른다: 큐에 넣고 도어벨만 누른 뒤 즉시
    ///   반환하고, 실제 배치 write 는 flush 레인(전용 스레드)이 한다.
    /// ★경로 1벌(ADR-0104 "flush = 일괄·오래된 순" 공유)★: 파킹으로 일원화하면 즉시/늦은 배달의 봉투
    ///   조립·관측·장부 전이가 전부 기존 flush 경로 하나를 탄다(두 벌 유지 금지). 봉투는 `ParkKind::Notice`
    ///   를 보고 `wrap_notice` 로 감싸진다.
    /// ★cap 예외★: `ParkKind::Notice` 라 보관함이 가득 차도 반드시 수용된다(회신 계약 통지가 막히면 계약이
    ///   반쪽 — ADR-0103 불변식). 그래서 반려 갈래가 없고 반환값도 없다.
    /// ★FIFO 정합★: 앞선 파킹이 있으면 notice 도 그 뒤에 붙는다 — 통지가 앞선 메일을 앞지르지 않는다.
    /// ★조용한 유실 금지★: park 와 함께 장부에 `pending` 을 남기고, 실제 주입 때 flush 가 `delivered` 로
    ///   전이한다(발신자 없음이라 장부 from 은 데몬 라벨, 관측 신원은 nil — 위 상수·헬퍼 주석).
    /// ★id 힌트 = **요청 발신자의 AgentId**(리뷰 fix 2 · load-bearing)★: 파킹 키는 발송 시점의 발신자
    ///   **이름**인데, 그 사이 그 에이전트가 개명했으면 이름-키 큐를 아무도 열지 않는다 — 통지는
    ///   `notified` 라 재발화도 없으니 그 notice 는 **영영 stranded**(계약이 조용히 반쪽) 된다. 그래서
    ///   장부가 함께 들고 있던 발신자 id 를 힌트로 실어, flush 가 이름 유일성과 무관하게 그 incarnation 으로
    ///   배달하게 한다(id 가 죽었으면 이름 규칙으로 자동 복귀 — respawn 이어받기 유지).
    /// ★도어벨은 **파킹 뒤 반드시** 누른다(같은 fix)★: 예전엔 이름이 유일 도달로 풀릴 때만 눌러서, 개명·
    ///   동명 다수 상황에서 큐에 넣고 아무도 깨우지 않는 lost wakeup 이 났다. 이제 발신자 id 로 한 번
    ///   (`flush_for_agent` 가 힌트 큐까지 연다), 이름이 다른 산 에이전트로 풀리면 그쪽으로도 한 번 누른다.
    /// ★락 규율★: 로스터는 호출자가 뜬 스냅샷을 받고(틱당 1회 — sweep 주석), park 만 짧은 락, 도어벨은 park
    ///   뒤(락 밖).
    // ADR-0103
    fn deliver_notice(&self, due: &DueTimeout, body: &str, roster: &[LiveAgent]) {
        // ★타임아웃↔회신 레이스 좁히기(리뷰 fix 5)★: due 산출(락 해제)과 여기 사이에 회신이 도착해 계약이
        //   닫혔을 수 있다 — 그러면 발신자는 회신을 받고도 "회신 없음" 통지를 뒤이어 받는다(모순된 통지).
        //   파킹 **직전**에 장부를 다시 보고 닫혔으면 통지를 접는다. 남는 잔여: 이 확인과 park 가 별개 락
        //   구간이라 그 사이(마이크로초)에 닫히는 경우는 여전히 통지가 나간다 — 한 락으로 묶으려면 park 를
        //   장부 안으로 끌어와야 해(관심사 혼합) 여기선 창을 좁히는 선에서 멈춘다.
        let closed_since_collection = {
            let st = self.state.lock().expect("messaging state poisoned");
            st.ledger.is_request_closed(&due.request_id)
        };
        if closed_since_collection {
            tracing::debug!(
                request_id = %due.request_id,
                "기한 초과 통지 취소 — 산출 후 파킹 전에 회신이 도착해 계약이 닫힘(spec §3)"
            );
            return;
        }

        let recipient = &due.sender;
        // ★notice id 도 같은 충돌 검사를 탄다(round-final fix 2)★ — 아래 `draw_daemon_msg_id` 주석.
        let drawn = self.draw_daemon_msg_id();
        if let Some(collided) = &drawn.collided {
            // 락 밖 로깅(모듈 헤더 규율).
            tracing::error!(
                collided = %collided,
                replacement = %drawn.id,
                still_colliding = drawn.still_colliding,
                "notice id 충돌 — 새 id 로 1회 재시도(ADR-0103 · 사실상 불가한 경로라 난수/장부 배선을 의심할 것)"
            );
        }
        let notice_id = drawn.id;
        // 이름이 지금 유일 도달로 풀리면 그쪽 도어벨도 누른다(발신자가 죽고 같은 이름이 재스폰된 경우).
        let by_name = unique_reachable_in(roster, recipient);
        let hint = format!("notice for '{recipient}' parked until that agent can receive it.");
        let _ = self.park_pending(
            &notice_id,
            NOTICE_SENDER_LABEL,
            daemon_identity(),
            Entrance::Daemon,
            recipient,
            body,
            hint,
            Some(due.sender_id),
            ParkKind::Notice,
            &SendMeta::default(),
        );
        // 도어벨(락 밖) — idle 이면 그 자리에서(다른 스레드) 배달되고, 턴 중이면 소비 측 게이트가 스킵해
        //   파킹이 유지된다(판정은 소비 측 한 곳 — fix 4 와 같은 분업). 죽은 id 로 눌러도 무해한 no-op 다.
        self.request_flush(due.sender_id);
        if let Some(t) = by_name.filter(|t| t.id != due.sender_id) {
            self.request_flush(t.id);
        }
    }

    /// ★데몬 자가 발신(`<notice>`)의 msg_id 를 발송과 **같은 충돌 검사**로 뽑는다(round-final fix 2)★.
    ///
    /// ★왜 notice 도 검사하나(load-bearing)★: 예전엔 `new_msg_id()` 결과를 그대로 썼다 — 에이전트 발송만
    ///   `Ledger::msg_id_in_use` 를 통과하고 **데몬이 만든 id 는 무검사**였다. msg_id 는 이력 레코드 키
    ///   ((msg_id, to))이자 관측 상관 키라, notice id 가 기존 id 와 겹치면 `transition` 이 남의 레코드를 집고
    ///   장부가 두 메시지를 한 id 로 뭉갠다 — 발신 주체가 데몬이라고 덜 해롭지 않다. 그래서 같은 검사를
    ///   같은 규율(한 락 구간 검사 + **1회** 재-draw)로 태운다.
    /// ★두 번째도 겹치면 그 id 를 그대로 쓴다(notice 만의 규칙)★: 에이전트 발송은 `IdCollision` 으로 반려해도
    ///   호출자가 재시도하면 그만이지만, notice 는 **버릴 수 없다** — 회신 계약 통지가 사라지면 계약이 조용히
    ///   반쪽 난다(보관함 cap 예외를 둔 이유와 같다). 관측상 오염(이력 앨리어싱)이 통지 유실보다 낫다. 그
    ///   상태는 `still_colliding` 으로 노출해 호출자가 락 밖에서 error 로 남긴다.
    /// ★남는 창★: 검사와 실제 record(`park_pending`) 사이의 TOCTOU 창은 **발송 경로와 동일**하고, 전면
    ///   예약을 두지 않기로 한 근거는 `handle_single_send` 의 예약 지점 주석이 정본이다(중복 서술 금지).
    fn draw_daemon_msg_id(&self) -> DrawnMsgId {
        self.draw_daemon_msg_id_with(new_msg_id)
    }

    /// 위 함수의 **테스트 seam** — id 생성기를 주입해 충돌 분기를 결정적으로 단언한다(난수로는 36^8 분의 1
    ///   확률이라 재현 불가). 주입 클로저는 락 보유 중 불리므로 순수해야 한다(테스트는 미리 만든 목록에서
    ///   꺼내기만 한다).
    fn draw_daemon_msg_id_with(&self, mut draw: impl FnMut() -> String) -> DrawnMsgId {
        let st = self.state.lock().expect("messaging state poisoned");
        let first = draw();
        if !st.ledger.msg_id_in_use(&first) {
            return DrawnMsgId {
                id: first,
                collided: None,
                still_colliding: false,
            };
        }
        let second = draw();
        let still_colliding = st.ledger.msg_id_in_use(&second);
        DrawnMsgId {
            id: second,
            collided: Some(first),
            still_colliding,
        }
    }

    /// 봉투를 **현재** 전역 포맷으로 감싼다(단일 wrap point ADR-0096). 속성(`fields`)은 호출자가 만든다 —
    ///   즉시 배달은 `SendMeta`, flush 배달은 파킹 payload 에서 복원한 같은 메타로(두 경로 동일 봉투, C3).
    fn wrap_now(&self, sender: &str, msg_id: &str, body: &str, fields: &EnvelopeFields) -> String {
        let format: EnvelopeFormat = self.registry.envelope_format();
        wrap_message(sender, msg_id, body, format, fields)
    }

    /// 성공 주입 관측 레코드 발행(ADR-0088) — 락 밖. registry.record_delivery 가 observer 규율을 갖는다.
    fn observe_success(
        &self,
        msg_id: &str,
        target: &LiveAgent,
        from: BoundIdentity,
        entrance: Entrance,
        wrapped: &str,
        outcome: &WriteOutcome,
    ) {
        self.registry.record_delivery(DeliveryObservation {
            msg_id: msg_id.to_string(),
            to_id: target.id,
            to_name: target.name.clone(),
            from,
            entrance,
            bytes_requested: outcome.bytes_requested,
            bytes_written: Some(outcome.bytes_written),
            msg_uuid: Some(outcome.msg_uuid),
            to_epoch: Some(outcome.epoch),
            error: None,
        });
        // 보안: body/토큰 미로깅 — 바이트 수·id 만.
        tracing::info!(
            from = %from.agent_id,
            to = %target.id,
            to_name = %target.name,
            msg_id = %msg_id,
            entrance = ?entrance,
            bytes = wrapped.len(),
            "메시징 배달(delivered, ADR-0103/0104)"
        );
    }

    /// 실패 주입 관측 레코드 발행(ADR-0088) — 실패를 성공으로 삼키지 않음의 증거. 이후 상위가 파킹한다.
    fn observe_failure(
        &self,
        msg_id: &str,
        target: &LiveAgent,
        from: BoundIdentity,
        entrance: Entrance,
        wrapped: &str,
        err: &str,
    ) {
        self.registry.record_delivery(DeliveryObservation {
            msg_id: msg_id.to_string(),
            to_id: target.id,
            to_name: target.name.clone(),
            from,
            entrance,
            bytes_requested: wrapped.len(),
            bytes_written: None,
            msg_uuid: None,
            to_epoch: None,
            error: Some(err.to_string()),
        });
        tracing::warn!(
            to = %target.id,
            to_name = %target.name,
            msg_id = %msg_id,
            "메시징 주입 실패 — 파킹으로 전환(무손실): {err}"
        );
    }

    /// 관측/테스트용 — 특정 msg_id 의 ledger 상태 목록(오래된 순).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn ledger_statuses(&self, msg_id: &str) -> Vec<DeliveryStatus> {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger
            .records_for(msg_id)
            .iter()
            .map(|r| r.status)
            .collect()
    }

    /// 관측/테스트용(C3) — 아직 회신 안 온 request 계약 수. 계약 오픈/닫기/회수를 단언한다.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn open_request_count(&self) -> usize {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger.open_request_count()
    }

    /// 관측/테스트용(C3) — 장부 이력 스냅샷 `(msg_id, from, to, status)`(오래된 순). msg_id 를 모르는
    ///   단언(데몬이 만든 notice 가 장부에 남았나 등)에 쓴다.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn ledger_snapshot(&self) -> Vec<(String, String, String, DeliveryStatus)> {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger
            .all_records()
            .iter()
            .map(|r| (r.msg_id.clone(), r.from.clone(), r.to.clone(), r.status))
            .collect()
    }

    /// 관측/테스트용 — 수신자 큐 현재 길이.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn parked_len(&self, recipient: &str) -> usize {
        let st = self.state.lock().expect("messaging state poisoned");
        st.mailbox.len(recipient)
    }

    /// 관측/테스트용 — 수신자 큐의 **현재 순서**(msg_id, 앞→뒤). 재파킹이 나이 순서를 지켰는지 단언하는 데
    ///   쓴다(round-4 finding 1) — 길이만으로는 순서 역전이 안 잡힌다.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn parked_msg_ids(&self, recipient: &str) -> Vec<String> {
        let st = self.state.lock().expect("messaging state poisoned");
        st.mailbox.msg_ids(recipient)
    }

    /// 테스트 전용(C3 리뷰 fix 4) — 파킹 항목 하나의 payload 를 **의도적으로 손상**시킨다. 깨진 항목이
    ///   flush 배치를 중단시키지 않고 그 항목만 폴백 봉투로 열화되는지 실제 경로로 단언하는 seam.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn corrupt_parked_payload_for_test(&self, recipient: &str, idx: usize) {
        let mut st = self.state.lock().expect("messaging state poisoned");
        st.mailbox
            .corrupt_envelope_for_test(recipient, idx, "CORRUPT-PAYLOAD".to_string());
    }
}

/// 파킹 payload — flush 주입 시점에 봉투 조립·관측 레코드 발행에 필요한 최소 메타(sender 이름·발신자
///   신원·입구·raw body). ParkedMessage.envelope 계약("완성 봉투")을 우회해 raw 를 나르기 위한 내부 인코딩.
///   flush 주입 시점에 decode → wrap_now 로 **현재** 포맷 봉투를 조립하고(단일 wrap point), 발신자 신원으로
///   배달 관측 레코드를 발행한다(등장 배달도 handle_single_send 와 동일하게 관측 — ADR-0088).
///
/// ★왜 from/entrance 도 나르나★: 파킹→flush 자동 배달도 배달 경계 관측(ADR-0088)의 대상이다 — 하네스가
///   "누가 누구에게 배달됐나" 를 flush 경로에서도 회수해야 한다(파킹→스폰→자동배달 acceptance, spec §7).
///   그러려면 원래 발신자 신원(BoundIdentity)과 입구를 flush 시점까지 보존해야 한다.
/// ★왜 meta(회신 계약)도 나르나(C3 · load-bearing)★: 파킹된 request/회신이 늦게 배달될 때도 **즉시 배달과
///   동일한 봉투 속성**(id/type/reply-by/in-reply-to)이 붙어야 한다. 안 나르면 파킹을 거친 request 는
///   속성 없는 plain 메시지로 도착해 수신 LLM 이 회신할 id 를 모르고, 발신자만 기한 초과 notice 를 받는다
///   (계약이 조용히 깨지는 최악 모드). 봉투를 park 시점에 굳히지 않는 설계의 대가로 재료를 나른다.
/// ★인코딩(v1 태그 — C3 리뷰 fix 4 에서 버전 헤더 도입)★:
///   `<ver>\n<sender_len>\n<reply_by_len>\n<reply_to_len>\n<from_agent_id>\n<from_epoch>\n<entrance>\n<flags>\n`
///   `<sender><reply_by><reply_to><body>`
///   앞 8줄은 개행 없는 필드(숫자/uuid/짧은 리터럴)라 개행으로 안전 분리하고, 가변 문자열 4개는 **길이
///   접두**로 경계를 잡는다(body·reply_to 에 개행이 들어와도 안전 — reply_to 는 에이전트 입력이라 임의
///   문자열일 수 있다). 길이 0 = `None`(빈 문자열 `Some("")` 은 입구 검증이 이미 반려하므로 모호하지 않다).
///
/// ★왜 버전 태그인가(fix 4)★: 이 형식은 **프로세스 내부**에서만 쓰이지만(파킹은 인메모리라 데몬 재시작이면
///   소멸), 형식이 바뀌는 순간 옛 payload 가 새 decode 를 만나면 조용히 오해석될 수 있다(길이 필드가 다른
///   자리로 밀려 body 가 잘리는 식). 1글자 태그가 있으면 **모르는 버전 = 즉시 폴백**으로 갈라져, 오해석
///   대신 "봉투 속성 잃고 body 만 남음"(보이는 열화)으로 실패한다. v2 영속화가 파킹을 디스크에 남기면
///   이 태그가 마이그레이션 분기점이 된다.
const PARK_PAYLOAD_VERSION: &str = "1";

struct ParkPayload {
    sender_name: String,
    from: BoundIdentity,
    entrance: Entrance,
    body: String,
    /// C3 회신 계약 메타(파싱된 Duration 은 **복원하지 않는다** — 계약은 발송 시점에 이미 장부에 열렸고,
    ///   flush 가 필요한 건 봉투 속성뿐이다). 그래서 `reply_by` 는 raw 표기만 살아남는다.
    meta: SendMeta,
}

impl ParkPayload {
    fn encode(&self) -> String {
        let ent = match self.entrance {
            Entrance::Mcp => "mcp",
            Entrance::Cli => "cli",
            Entrance::Daemon => "daemon",
        };
        let reply_by = self.meta.reply_by_raw.as_deref().unwrap_or("");
        let reply_to = self.meta.reply_to.as_deref().unwrap_or("");
        let flags = if self.meta.request { "r" } else { "-" };
        format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}{}{}{}",
            PARK_PAYLOAD_VERSION,
            self.sender_name.len(),
            reply_by.len(),
            reply_to.len(),
            self.from.agent_id,
            self.from.epoch,
            ent,
            flags,
            self.sender_name,
            reply_by,
            reply_to,
            self.body
        )
    }

    /// encode 역연산. 형식이 깨지면(있을 수 없음 — 우리 인코딩) 전체를 body 로 폴백(조용한 유실보다 낫다).
    ///
    /// ★절대 패닉하지 않는다(fix 4 · load-bearing)★: 이 함수는 flush 배치 루프 **안에서 항목마다** 불린다 —
    ///   여기서 패닉하면 그 배치의 나머지 항목이 통째로 날아가고(release 는 panic=abort 라 데몬 자체가 죽는다)
    ///   깨진 항목 하나가 멀쩡한 메시지들을 끌고 간다. 그래서 모든 실패 경로가 **항목 단위 폴백**으로 끝난다:
    ///   길이 필드가 거짓이거나(합이 남은 길이 초과) **char 경계를 안 가르거나**(멀티바이트 중간 절단 —
    ///   `split_at` 이 패닉하는 그 경우) 버전이 모르는 값이면, 봉투 속성을 잃되 원문을 body 로 살려 보낸다.
    ///   덕분에 배치의 다른 항목은 정상 배달된다(테스트: corrupt 항목 1개가 배치를 중단시키지 않음).
    fn decode(s: &str) -> Self {
        // 버전 + 앞 7개 개행 필드 + 나머지(sender+reply_by+reply_to+body). splitn(9) 로 body 안 개행을 보존.
        let mut it = s.splitn(9, '\n');
        let fallback = || Self {
            sender_name: String::new(),
            from: BoundIdentity {
                agent_id: AgentId::nil(),
                epoch: 0,
            },
            entrance: Entrance::Cli,
            body: s.to_string(),
            meta: SendMeta::default(),
        };
        let (
            Some(ver),
            Some(sender_len),
            Some(rb_len),
            Some(rt_len),
            Some(id_str),
            Some(ep_str),
            Some(ent_str),
            Some(flags),
            Some(rest),
        ) = (
            it.next(),
            it.next(),
            it.next(),
            it.next(),
            it.next(),
            it.next(),
            it.next(),
            it.next(),
            it.next(),
        )
        else {
            return fallback();
        };
        // 모르는 버전 = 오해석 금지(위 상수 주석) — 폴백으로 갈라 보이는 열화로 실패한다.
        if ver != PARK_PAYLOAD_VERSION {
            return fallback();
        }
        // ★enum 은 **엄격 파싱**한다(round-final fix 3 · load-bearing)★: 예전엔 모르는 입구 문자열이 조용히
        //   `Cli` 로, 모르는 플래그가 조용히 "request 아님" 으로 떨어졌다. 그건 길이 필드가 깨졌을 때와 같은
        //   종류의 손상인데 **혼자만 다른 실패 모드**(폴백이 아니라 임의 재해석)를 갖는다는 뜻이라,
        //   `<message>` 로 나가야 할 게 관측상 다른 입구로 기록되거나 request 봉투(id/reply-by)를 잃은 채
        //   plain 으로 도착한다 — 후자는 수신 LLM 이 회신할 id 를 모르는 계약 파손이다. 어휘 밖 값은 손상의
        //   증거이므로 길이 손상과 **같은 항목 단위 폴백**으로 보낸다(조용한 재해석 금지).
        let (Some(entrance), Some(request)) = (parse_entrance(ent_str), parse_request_flag(flags))
        else {
            return fallback();
        };
        let (Ok(sender_len), Ok(rb_len), Ok(rt_len), Ok(agent_id), Ok(epoch)) = (
            sender_len.parse::<usize>(),
            rb_len.parse::<usize>(),
            rt_len.parse::<usize>(),
            id_str.parse::<AgentId>(),
            ep_str.parse::<u32>(),
        ) else {
            return fallback();
        };
        // 길이 합이 남은 문자열을 넘으면 인코딩이 깨진 것 — 폴백(패닉 대신).
        let Some(total) = sender_len
            .checked_add(rb_len)
            .and_then(|v| v.checked_add(rt_len))
        else {
            return fallback();
        };
        if rest.len() < total {
            return fallback();
        }
        // ★char 경계 검증(fix 4)★: 길이는 우리가 쓴 **바이트** 길이라 정상 payload 에선 항상 경계와 맞지만,
        //   payload 가 깨졌다면(외부 주입·형식 드리프트) 멀티바이트 문자 중간을 가리킬 수 있고 그때
        //   `split_at` 은 **패닉**한다. 세 절단점을 모두 미리 확인하고, 하나라도 어긋나면 폴백한다.
        let cut1 = sender_len;
        let cut2 = cut1 + rb_len;
        let cut3 = cut2 + rt_len;
        if !rest.is_char_boundary(cut1)
            || !rest.is_char_boundary(cut2)
            || !rest.is_char_boundary(cut3)
        {
            return fallback();
        }
        let (sender, tail) = rest.split_at(cut1);
        let (reply_by, tail) = tail.split_at(rb_len);
        let (reply_to, body) = tail.split_at(rt_len);
        Self {
            sender_name: sender.to_string(),
            from: BoundIdentity { agent_id, epoch },
            entrance,
            body: body.to_string(),
            meta: SendMeta {
                request,
                reply_by_raw: (!reply_by.is_empty()).then(|| reply_by.to_string()),
                // 파싱값은 복원하지 않는다(위 struct 주석 — flush 는 표기만 필요).
                reply_by: None,
                reply_to: (!reply_to.is_empty()).then(|| reply_to.to_string()),
            },
        }
    }
}

/// `ParkPayload` 의 입구 어휘 ↔ enum(**엄격** — 어휘 밖은 `None` = 손상 신호, fix 3). `encode` 의 리터럴과
///   한 쌍이라 입구 종류가 늘면 둘을 함께 고친다(어긋나면 정상 payload 가 폴백돼 round-trip 테스트가 잡는다).
fn parse_entrance(s: &str) -> Option<Entrance> {
    match s {
        "mcp" => Some(Entrance::Mcp),
        "cli" => Some(Entrance::Cli),
        "daemon" => Some(Entrance::Daemon),
        _ => None,
    }
}

/// `ParkPayload` 의 request 플래그 어휘 ↔ bool(**엄격**). `r` = request, `-` = 통보/회신, 그 밖 = 손상.
fn parse_request_flag(s: &str) -> Option<bool> {
    match s {
        "r" => Some(true),
        "-" => Some(false),
        _ => None,
    }
}

/// ★데몬 자가 발신의 신원(C3)★ — `<notice>` 에는 발신 에이전트가 없다. 관측 레코드(`DeliveryObservation.from`)
///   가 신원을 요구하므로 **nil AgentId** 를 데몬 출처 표식으로 쓴다(어떤 실제 에이전트와도 겹치지 않는다).
///   `Entrance::Daemon` 과 짝을 이뤄 "이건 인프라 통지" 를 레코드만으로 판별하게 한다.
fn daemon_identity() -> BoundIdentity {
    BoundIdentity {
        agent_id: AgentId::nil(),
        epoch: 0,
    }
}

/// `draw_daemon_msg_id` 결과 — 선택된 id + 관측용 충돌 흔적(로깅은 **락 밖** 호출자가 한다 — 모듈 헤더 규율).
struct DrawnMsgId {
    /// 실제로 쓸 id(검사를 통과했거나, 재-draw 까지 하고도 걸린 최후의 값).
    id: String,
    /// 첫 draw 가 장부와 충돌해 버려진 경우 그 값 — 조사 단서는 **충돌한 쪽**에 있다(ingress 재시도 로그와
    ///   같은 규율: 대체 id 만 남기면 무엇과 부딪혔는지 추적 불가).
    collided: Option<String>,
    /// 재-draw 한 id **마저** 충돌했나. true = 난수/장부 배선 의심 신호지만, notice 는 그래도 내보낸다.
    still_colliding: bool,
}

// ★기간 표기 역산(duration_notation)은 제거됐다(C3 리뷰 fix 6)★: `Duration` → 표기 복원은 정규화라
//   `"60m"` 으로 보낸 기한을 `"1h"` 로 통지해 봉투 속성(`reply-by="60m"`)과 문구가 어긋났다. 이제 장부가
//   발신자 표기를 원본째 보관하고(ledger.rs `RequestEntry.reply_by`) 통지는 그걸 그대로 쓴다 — 역산 함수가
//   다시 생기면 같은 불일치가 재발한다.

/// `to`(이름 또는 AgentId 문자열) → 산·도달 가능 수신자(LiveAgent). 매치 규칙(ingress::resolve_recipient
///   미러 — F2): AgentId 문자열 정확 일치 우선 → 이름 정확 일치. 동명 다수·부재는 여기선 None 으로 접고
///   상위(handle_send)가 AMBIGUOUS/파킹을 판정한다.
///
/// ★C1 스코프 = 단일 수신자★: 동명 다수(RECIPIENT_AMBIGUOUS)는 상위가 로스터로 판정(파킹 전에). 여기선
///   유일 매치만 Some — 0개 또는 2개+ 면 None. 상위가 로스터를 다시 보지 않도록 여기 로직을 최소로 둔다.
/// ★이름 유일 도달 판정(단일 출처)★: 주어진 로스터 **스냅샷**에서 그 이름의 도달 후보가 정확히 1개면
///   그 항목, 0개(부재)·2개+(동명 다수)면 None. 스냅샷을 인자로 받는 이유: flush 배치는 이름 판정과
///   id-힌트 생존 판정을 **같은 스냅샷**으로 해야 배치 도중 판정이 흔들리지 않는다(로스터 재조회 금지).
fn unique_reachable_in(roster: &[LiveAgent], name: &str) -> Option<LiveAgent> {
    let mut matches = roster.iter().filter(|a| a.name == name);
    let first = matches.next()?;
    // 두 번째가 있으면 동명 다수 — None(파킹 유지). 없으면 유일.
    match matches.next() {
        Some(_) => None,
        None => Some(first.clone()),
    }
}

fn resolve_live(to: &str, roster: &[LiveAgent]) -> Option<LiveAgent> {
    // F2: AgentId 문자열 정확 일치 우선(이름=UUID 충돌이 ID 지목을 가로채지 못하게 — ingress 미러).
    if let Some(a) = roster.iter().find(|a| a.id.to_string() == to) {
        return Some(a.clone());
    }
    let by_name: Vec<&LiveAgent> = roster.iter().filter(|a| a.name == to).collect();
    if by_name.len() == 1 {
        return Some(by_name[0].clone());
    }
    None
}

/// sweep 만료분을 ledger 에 `expired` 로 남긴다(recipient 를 sweep 반환이 안 주므로 msg_id 로 레코드 역조회).
///   같은 msg_id 의 첫 `pending` 레코드를 찾아 그 to 로 전이한다. 조용한 유실 금지(spec §5 expired 장부 잔존).
///
/// ★C4 위험(finding 7 · load-bearing 경고)★: "첫 pending 레코드" 선택은 **단일 수신자 C1 에서만 옳다**.
///   C4 그룹 방송은 한 msg_id 에 수신자별 레코드가 N개(1 msg_id : N records)라, 만료된 특정 수신자 큐 항목이
///   어느 레코드인지 msg_id 만으로는 특정 못 한다 — 첫 pending 을 집으면 **엉뚱한 수신자 레코드**를 expired 로
///   전이할 수 있다. C4 가 들어오면 sweep_expired 가 (msg_id, recipient) 를 함께 반환하도록 mailbox seam 을
///   넓혀 여기서 recipient 로 정확히 지목해야 한다(msg_id 단독 역조회 금지). 그 전까지 이 헬퍼는 C1 전용이다.
// ADR-0104
fn transition_expired_by_msg_id(ledger: &mut Ledger, msg_id: &str, now: Instant) {
    // 이 msg_id 의 pending 레코드의 recipient(to)를 찾는다. records_for 는 &, transition 은 &mut 이라
    //   recipient 를 String 으로 복사해 borrow 를 끊는다(락 구간 내 순수 조작).
    let recipient = ledger
        .records_for(msg_id)
        .iter()
        .find(|r| r.status == DeliveryStatus::Pending)
        .map(|r| r.to.clone());
    if let Some(to) = recipient {
        let _ = ledger.transition(msg_id, &to, DeliveryStatus::Expired, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;

    /// 헤드리스 테스트용 DeliveryPort — 로스터·주입 성공/실패를 스크립트한다(claude/PTY 없음).
    struct FakeDeliveryPort {
        /// 살아있는 도달 가능 로스터(테스트가 세팅).
        roster: StdMutex<Vec<LiveAgent>>,
        /// 주입 시도 로그(순서대로 (to_id, 바이트)). 오래된 순 주입·개별 봉투 단언용.
        injected: StdMutex<Vec<(AgentId, Vec<u8>)>>,
        /// inject 호출 횟수(0-based) — fail_at_call 인덱스 매칭용.
        call_count: StdMutex<usize>,
        /// 이 호출 인덱스(0-based)들에서 inject 를 실패시킨다(부분 실패·전체 실패 시나리오 스크립트).
        fail_at_call: StdMutex<Vec<usize>>,
        /// inject 호출 시점에 실행할 부수 hook(drain↔inject 사이 동시 park 레이스 재현용). 호출 인덱스를
        ///   받아 임의 조작(예: 재파킹으로 큐 재충전)을 한다 — 이 hook 안에서 서비스 락을 다시 잡지 않는다
        ///   (inject 는 락 밖 호출이므로 안전).
        on_inject: StdMutex<Option<Box<dyn Fn(usize) + Send>>>,
        /// roster 밖 id 의 canonical_name override(비-도달 수신자 = TUI 시나리오 — finding 4). roster 에
        ///   없어도 이 맵에 있으면 canonical_name 이 그 이름을 돌려준다(로스터엔 안 뜨는 산 에이전트 모사).
        canonical_overrides: StdMutex<std::collections::HashMap<AgentId, String>>,
        /// live_reachable_agents 조회 횟수(late-appearance 스크립트용 + sweep 스냅샷 1회 단언 — fix 8).
        roster_calls: StdMutex<usize>,
        /// 세팅되면 **첫 조회 이후**부터 이 roster 를 돌려준다(첫 조회 = resolve 는 원래 roster, 이후 =
        ///   self_heal 이 보는 late-appearance roster). finding 3 TOCTOU self-heal 재현용.
        roster_after_first: StdMutex<Option<Vec<LiveAgent>>>,
        /// ★일회성 roster 조회 hook(fix 5 레이스 재현)★ — 로스터를 뜨는 **그 순간** 다른 일이 벌어진 상황을
        ///   결정적으로 만든다(예: sweep 의 due 산출 뒤·notice 파킹 전에 회신이 도착). 한 번 쓰고 비우므로
        ///   hook 안에서 다시 발송해도 재귀하지 않는다.
        on_roster: StdMutex<Option<Box<dyn Fn() + Send>>>,
    }

    impl FakeDeliveryPort {
        fn new() -> Self {
            Self {
                roster: StdMutex::new(Vec::new()),
                injected: StdMutex::new(Vec::new()),
                call_count: StdMutex::new(0),
                fail_at_call: StdMutex::new(Vec::new()),
                on_inject: StdMutex::new(None),
                canonical_overrides: StdMutex::new(std::collections::HashMap::new()),
                roster_calls: StdMutex::new(0),
                roster_after_first: StdMutex::new(None),
                on_roster: StdMutex::new(None),
            }
        }
        /// 다음 roster 조회 때 **한 번만** 실행할 hook(fix 5 레이스 재현).
        fn arm_on_next_roster(&self, f: Box<dyn Fn() + Send>) {
            *self.on_roster.lock().unwrap() = Some(f);
        }
        /// live_reachable_agents 총 호출 수(fix 8 — sweep 이 틱당 1회만 뜨는지 단언).
        fn roster_call_count(&self) -> usize {
            *self.roster_calls.lock().unwrap()
        }
        /// roster 밖 id 에 canonical 이름을 부여(비-도달 산 에이전트 모사 — finding 4).
        fn set_canonical(&self, id: AgentId, name: &str) {
            self.canonical_overrides
                .lock()
                .unwrap()
                .insert(id, name.to_string());
        }
        /// 첫 live_reachable_agents 조회 이후부터 돌려줄 roster 세팅(late-appearance — finding 3 self-heal).
        fn arm_roster_after_first_call(&self, roster: Vec<LiveAgent>) {
            *self.roster_after_first.lock().unwrap() = Some(roster);
        }
        fn set_roster(&self, roster: Vec<LiveAgent>) {
            *self.roster.lock().unwrap() = roster;
        }
        /// 주어진 inject 호출 인덱스(0-based)들에서 Err 를 낸다(그 외는 성공).
        fn fail_at(&self, indices: &[usize]) {
            *self.fail_at_call.lock().unwrap() = indices.to_vec();
        }
        /// inject 호출마다(성공/실패 결정 전) 부를 hook 설치 — 동시 park 레이스 재현용.
        fn set_on_inject(&self, f: Box<dyn Fn(usize) + Send>) {
            *self.on_inject.lock().unwrap() = Some(f);
        }
        fn injected_bodies(&self) -> Vec<String> {
            self.injected
                .lock()
                .unwrap()
                .iter()
                .map(|(_, b)| String::from_utf8_lossy(b).to_string())
                .collect()
        }
    }

    impl DeliveryPort for FakeDeliveryPort {
        fn inject(&self, to_id: AgentId, bytes: &[u8]) -> Result<WriteOutcome, String> {
            let idx = {
                let mut c = self.call_count.lock().unwrap();
                let i = *c;
                *c += 1;
                i
            };
            // 동시 park 레이스 재현 hook(설치돼 있으면). fail/success 결정 전에 호출한다.
            if let Some(f) = self.on_inject.lock().unwrap().as_ref() {
                f(idx);
            }
            if self.fail_at_call.lock().unwrap().contains(&idx) {
                return Err("fake inject fail".to_string());
            }
            self.injected.lock().unwrap().push((to_id, bytes.to_vec()));
            Ok(WriteOutcome {
                bytes_requested: bytes.len(),
                bytes_written: bytes.len(),
                msg_uuid: uuid::Uuid::new_v4(),
                epoch: 0,
            })
        }
        fn live_reachable_agents(&self) -> Vec<LiveAgent> {
            let call = {
                let mut c = self.roster_calls.lock().unwrap();
                let i = *c;
                *c += 1;
                i
            };
            // 일회성 hook — 락을 놓고 부른다(hook 이 다시 발송해도 자기 락에 걸리지 않게).
            let hook = self.on_roster.lock().unwrap().take();
            if let Some(f) = hook {
                f();
            }
            // 첫 조회(call 0) = 원래 roster(resolve 가 봄). 이후 = armed roster(있으면 — late appearance).
            if call >= 1 {
                if let Some(r) = self.roster_after_first.lock().unwrap().as_ref() {
                    return r.clone();
                }
            }
            self.roster.lock().unwrap().clone()
        }
        fn canonical_name(&self, id: AgentId) -> Option<String> {
            // roster 우선 → armed(late-appearance) roster → override(비-도달 산 에이전트) 조회.
            //   ★armed 도 보는 이유★: `arm_roster_after_first_call` 은 "그 사이 등장했다" 를 모사하는데,
            //   canonical_name 이 그걸 못 보면 fake 가 자기모순이다(등장했는데 이름이 없는 에이전트).
            //   실 구현(ManagerDeliveryPort)은 같은 manager 스냅샷을 보므로 항상 일관된다.
            let by_roster = self
                .roster
                .lock()
                .unwrap()
                .iter()
                .find(|a| a.id == id)
                .map(|a| a.name.clone());
            by_roster
                .or_else(|| {
                    self.roster_after_first
                        .lock()
                        .unwrap()
                        .as_ref()
                        .and_then(|r| r.iter().find(|a| a.id == id).map(|a| a.name.clone()))
                })
                .or_else(|| self.canonical_overrides.lock().unwrap().get(&id).cloned())
        }
    }

    fn svc() -> (Arc<MessagingService>, Arc<FakeDeliveryPort>) {
        let port = Arc::new(FakeDeliveryPort::new());
        let registry = Arc::new(ControlRegistry::new()); // 기본 봉투 = xml(ADR-0103).
        let svc = Arc::new(MessagingService::new(port.clone(), registry));
        (svc, port)
    }

    fn ident() -> BoundIdentity {
        BoundIdentity {
            agent_id: AgentId::new_v4(),
            epoch: 0,
        }
    }

    fn live(name: &str) -> (AgentId, LiveAgent) {
        let id = AgentId::new_v4();
        (
            id,
            LiveAgent {
                id,
                name: name.to_string(),
                epoch: 0,
            },
        )
    }

    // ── C2 idle 게이트 테스트 하네스 ─────────────────────────────────────────────────
    /// 가짜 idle 게이트 — (id, epoch) 별 busy 를 테스트가 세팅하고, 호출 횟수를 세어 TOCTOU 시나리오
    ///   (첫 확인은 busy, 재확인은 idle)를 스크립트한다. 실 tap/PTY 없이 게이트 분기만 결정적으로 단언.
    struct FakeGate {
        busy: StdMutex<std::collections::HashSet<(AgentId, u32)>>,
        calls: StdMutex<usize>,
        /// 세팅되면 이 호출 인덱스(0-based) **이후** 부터는 항상 idle 로 답한다(park↔턴종료 레이스 모사).
        idle_after_call: StdMutex<Option<usize>>,
    }
    impl FakeGate {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                busy: StdMutex::new(std::collections::HashSet::new()),
                calls: StdMutex::new(0),
                idle_after_call: StdMutex::new(None),
            })
        }
        fn set_busy(&self, id: AgentId, epoch: u32) {
            self.busy.lock().unwrap().insert((id, epoch));
        }
        fn clear(&self) {
            self.busy.lock().unwrap().clear();
        }
        /// 첫 `n` 회 조회까지만 busy 로 답하고 그 뒤엔 idle(= 그 사이 턴이 끝난 상황).
        fn arm_idle_after_call(&self, n: usize) {
            *self.idle_after_call.lock().unwrap() = Some(n);
        }
        fn call_count(&self) -> usize {
            *self.calls.lock().unwrap()
        }
    }
    impl super::super::busy::BusyGate for FakeGate {
        fn is_busy(&self, id: AgentId, epoch: u32) -> bool {
            let idx = {
                let mut c = self.calls.lock().unwrap();
                let i = *c;
                *c += 1;
                i
            };
            if let Some(n) = *self.idle_after_call.lock().unwrap() {
                if idx >= n {
                    return false;
                }
            }
            self.busy.lock().unwrap().contains(&(id, epoch))
        }
    }

    /// 게이트를 끼운 서비스 조립(C2). ★도어벨 미배선★ = 인라인 flush 폴백(FlushTrigger 주석) — 자가치유가
    ///   동기라 단언이 결정적이다. 도어벨 배선 자체는 `svc_gated_with_doorbell` 이 검증한다.
    fn svc_gated() -> (Arc<MessagingService>, Arc<FakeDeliveryPort>, Arc<FakeGate>) {
        let port = Arc::new(FakeDeliveryPort::new());
        let gate = FakeGate::new();
        let registry = Arc::new(ControlRegistry::new());
        let svc = Arc::new(MessagingService::new_gated(
            port.clone(),
            registry,
            gate.clone(),
        ));
        (svc, port, gate)
    }

    /// 도어벨 요청을 기록만 하는 FlushTrigger — 운영 조립처럼 flush 를 **다른 스레드로 넘기는** 모양을
    ///   모사한다(여기선 아무도 소비하지 않으므로 "발신 스레드에서 주입이 일어나지 않음" 을 단언할 수 있다).
    struct FakeTrigger {
        seen: StdMutex<Vec<AgentId>>,
    }
    impl FakeTrigger {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                seen: StdMutex::new(Vec::new()),
            })
        }
        fn seen(&self) -> Vec<AgentId> {
            self.seen.lock().unwrap().clone()
        }
    }
    impl FlushTrigger for FakeTrigger {
        fn request_flush(&self, id: AgentId) {
            self.seen.lock().unwrap().push(id);
        }
    }

    /// 게이트 + **도어벨** 배선 조립(운영 미러 — fix 11).
    fn svc_gated_with_doorbell() -> (
        Arc<MessagingService>,
        Arc<FakeDeliveryPort>,
        Arc<FakeGate>,
        Arc<FakeTrigger>,
    ) {
        let port = Arc::new(FakeDeliveryPort::new());
        let gate = FakeGate::new();
        let bell = FakeTrigger::new();
        let registry = Arc::new(ControlRegistry::new());
        let svc = Arc::new(
            MessagingService::new_gated(port.clone(), registry, gate.clone())
                .with_flush_trigger(bell.clone()),
        );
        (svc, port, gate, bell)
    }

    #[test]
    fn deliver_ok_marks_delivered_and_injects_wrapped() {
        let (svc, port) = svc();
        let (_id, alice) = live("alice");
        port.set_roster(vec![alice]);
        let out = svc
            .handle_single_send(
                "m1",
                ident(),
                "bob",
                "alice",
                "hi",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("no reject");
        assert_eq!(out, SendOutcome::Delivered);
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Delivered],
            "즉시 주입 = delivered(ADR-0104)"
        );
        // 봉투는 주입 시점에 xml 로 감싸진다(단일 wrap point).
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="bob">hi</message>"#.to_string()],
            "주입 = 현재 포맷(xml) 봉투"
        );
    }

    #[test]
    fn absent_recipient_parks_pending_not_error() {
        // spec §5 분기 2: "없는 이름" 도 파킹(RECIPIENT_NOT_FOUND 소멸).
        let (svc, port) = svc();
        port.set_roster(vec![]); // 아무도 없음.
        let out = svc
            .handle_single_send(
                "m1",
                ident(),
                "bob",
                "ghost",
                "hi",
                Entrance::Cli,
                &SendMeta::default(),
            )
            .expect("파킹은 반려 아님");
        assert!(matches!(out, SendOutcome::Parked { .. }), "부재 → 파킹");
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Pending],
            "파킹 = pending 장부 기록(조용한 유실 금지)"
        );
        assert_eq!(svc.parked_len("ghost"), 1, "이름 기준 파킹");
        assert!(port.injected_bodies().is_empty(), "부재는 주입 안 함");
    }

    #[test]
    fn inject_failure_parks_pending() {
        // spec §5 분기 3(unreachable/write-fail) → 파킹. 해석은 성공하나 inject Err.
        let (svc, port) = svc();
        let (_id, alice) = live("alice");
        port.set_roster(vec![alice]);
        port.fail_at(&[0]); // 첫(유일) inject 실패.
        let out = svc
            .handle_single_send(
                "m1",
                ident(),
                "bob",
                "alice",
                "hi",
                Entrance::Cli,
                &SendMeta::default(),
            )
            .expect("파킹은 반려 아님");
        assert!(
            matches!(out, SendOutcome::Parked { .. }),
            "inject 실패 → 파킹"
        );
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Pending],
            "inject 실패도 pending 장부(유실 금지)"
        );
        assert_eq!(svc.parked_len("alice"), 1);
    }

    #[test]
    fn flush_on_appearance_delivers_oldest_first_individually_wrapped() {
        // 부재 시 3건 파킹 → 등장 flush → 오래된 순 + 각자 개별 봉투 주입.
        let (svc, port) = svc();
        port.set_roster(vec![]);
        for (i, m) in ["m0", "m1", "m2"].iter().enumerate() {
            svc.handle_single_send(
                m,
                ident(),
                "sndr",
                "late",
                &format!("body{i}"),
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("park");
        }
        assert_eq!(svc.parked_len("late"), 3);
        // 등장 — 로스터에 유일 도달로 올린다(finding 2: flush_for 는 execution 시점 로스터로 재검증).
        let (late_id, late) = live("late");
        port.set_roster(vec![late]);
        svc.flush_for("late", late_id);
        // 오래된 순 + 각자 봉투(단일 wrap point, 배치 내 개별).
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="sndr">body0</message>"#.to_string(),
                r#"<message from="sndr">body1</message>"#.to_string(),
                r#"<message from="sndr">body2</message>"#.to_string(),
            ],
            "flush = 오래된 순 개별 봉투 일괄 주입(ADR-0104)"
        );
        // 전부 delivered 로 전이.
        assert_eq!(svc.ledger_statuses("m0"), vec![DeliveryStatus::Delivered]);
        assert_eq!(svc.ledger_statuses("m2"), vec![DeliveryStatus::Delivered]);
        assert_eq!(svc.parked_len("late"), 0, "flush 후 큐 비움");
    }

    #[test]
    fn flush_partial_failure_reparks_remaining_no_loss() {
        // ★부분 실패 무손실(load-bearing, ADR-0104)★: 3건 파킹 → flush 중 **두 번째** inject 실패 →
        //   첫 건은 delivered, 둘째(실패분)+셋째(미시도)는 재파킹돼 다음 등장에 재시도된다(조용한 유실 금지).
        let (svc, port) = svc();
        port.set_roster(vec![]);
        for (i, m) in ["m0", "m1", "m2"].iter().enumerate() {
            svc.handle_single_send(
                m,
                ident(),
                "s",
                "late",
                &format!("b{i}"),
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("park");
        }
        let (late_id, late) = live("late");
        // finding 2: flush_for 는 로스터 유일 도달 재검증 후 진행.
        port.set_roster(vec![late]);
        // inject 호출 인덱스 1(= 둘째 배치 항목 b1)에서 실패 → idx 1 에서 break, [1..](b1·b2) 재파킹.
        port.fail_at(&[1]);
        svc.flush_for("late", late_id);
        // 첫 건(b0)만 주입 성사.
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="s">b0</message>"#.to_string()],
            "실패 전(b0)만 주입"
        );
        assert_eq!(
            svc.ledger_statuses("m0"),
            vec![DeliveryStatus::Delivered],
            "b0 delivered"
        );
        // b1·b2 는 재파킹(pending 유지) — 큐에 2건.
        assert_eq!(svc.parked_len("late"), 2, "실패분+미시도분 재파킹(무손실)");
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Pending],
            "b1 재파킹 pending"
        );
        assert_eq!(
            svc.ledger_statuses("m2"),
            vec![DeliveryStatus::Pending],
            "b2 재파킹 pending"
        );
        // 이제 실패 예약 해제 후 재-flush → 남은 b1·b2 가 오래된 순 보존돼 주입.
        port.fail_at(&[]);
        svc.flush_for("late", late_id);
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="s">b0</message>"#.to_string(),
                r#"<message from="s">b1</message>"#.to_string(),
                r#"<message from="s">b2</message>"#.to_string(),
            ],
            "재파킹분 재-flush 도 오래된 순 보존(b1→b2)"
        );
        assert_eq!(svc.parked_len("late"), 0, "재-flush 후 큐 비움");
    }

    #[test]
    fn flush_repark_with_full_queue_loses_nothing_and_preserves_order() {
        // ★finding 1(BLOCK) 회귀★: flush 중 inject 실패로 재파킹할 때, drain↔inject 사이 **동시 park** 가
        //   큐를 cap 까지 재충전했어도 admitted 항목이 유실되면 안 되고(조용한 유실 금지), 재파킹분(더 오래됨)이
        //   동시 park 신규분보다 앞서야 한다(FIFO 역전 방지). 옛 park-기반 재파킹은 MailboxFull 로 유실됐다.
        let (svc, port) = svc();
        let (late_id, late) = live("late");
        // 오래된 파킹 3건(m0,m1,m2) — 로스터 부재라 파킹된다.
        port.set_roster(vec![]);
        for (i, m) in ["m0", "m1", "m2"].iter().enumerate() {
            svc.handle_single_send(
                m,
                ident(),
                "s",
                "late",
                &format!("b{i}"),
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("park");
        }
        // ★finding 2 정합★: flush_for 는 execution 시점 로스터로 "late" 를 유일 도달로 재검증하므로, flush
        //   호출 직전 "late" 를 올린다. 단 hook 의 동시 sends 는 **부재**여야 파킹되므로, hook 이 idx 0
        //   진입에서 로스터를 잠시 비운 뒤 sends 를 돌린다(재검증은 이미 통과 후라 무영향).
        let svc_hook = svc.clone();
        let port_hook = port.clone();
        let late_for_hook = late.clone();
        port.set_on_inject(Box::new(move |idx| {
            if idx == 0 {
                // 동시 park 가 파킹되도록 로스터를 잠시 비운다(부재). flush_for 재검증은 이 hook 전에 끝남.
                port_hook.set_roster(vec![]);
                for i in 0..MAILBOX_CAP_FOR_TEST {
                    // 동시 발송(신규 admission) — cap 까지 채운다(로스터 부재라 파킹).
                    let _ = svc_hook.handle_single_send(
                        &format!("c{i}"),
                        ident(),
                        "s",
                        "late",
                        &format!("c{i}"),
                        Entrance::Mcp,
                        &SendMeta::default(),
                    );
                }
                // 2차 flush 재검증이 통과하도록 다시 유일 도달로 되돌린다.
                port_hook.set_roster(vec![late_for_hook.clone()]);
            }
        }));
        // flush_for 진입 재검증(finding 2)이 통과하도록 "late" 를 유일 도달로 올린다.
        port.set_roster(vec![late.clone()]);
        // idx 1(둘째 항목 b1)에서 실패 → [b1,b2] 재파킹. 그 사이 hook(idx 0)이 큐를 cap 까지 채웠다.
        port.fail_at(&[1]);
        svc.flush_for("late", late_id);

        // b0 만 배달.
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="s">b0</message>"#.to_string()],
            "실패 전(b0)만 주입"
        );
        // ★유실 0★: b1·b2(admitted 재파킹) + 동시 park c0..c99(cap) = cap+2 건이 큐에 남아야.
        assert_eq!(
            svc.parked_len("late"),
            MAILBOX_CAP_FOR_TEST + 2,
            "재파킹분(b1,b2)이 cap 재충전에도 유실되지 않아야(restore_ordered cap 우회)"
        );
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Pending],
            "b1 재파킹 pending(유령 아님)"
        );
        assert_eq!(
            svc.ledger_statuses("m2"),
            vec![DeliveryStatus::Pending],
            "b2 재파킹 pending(유령 아님)"
        );
        // ★순서★: hook 을 제거하고 재-flush → 재파킹분(b1,b2)이 동시 park 분(c0..)보다 먼저 나와야.
        port.set_on_inject(Box::new(|_| {}));
        port.fail_at(&[]);
        svc.flush_for("late", late_id);
        let bodies = port.injected_bodies();
        // b0(첫 flush) 다음이 b1,b2(재파킹 오래된 순) 그 뒤 c0..(동시 park 최근).
        assert_eq!(bodies[0], r#"<message from="s">b0</message>"#);
        assert_eq!(
            bodies[1], r#"<message from="s">b1</message>"#,
            "재파킹분이 동시 park 신규분보다 앞서야(FIFO 역전 방지)"
        );
        assert_eq!(bodies[2], r#"<message from="s">b2</message>"#);
        assert_eq!(
            bodies[3], r#"<message from="s">c0</message>"#,
            "동시 park 신규분은 재파킹분 뒤"
        );
        assert_eq!(svc.parked_len("late"), 0, "재-flush 후 큐 비움");
    }

    /// 테스트에서 mailbox cap(100)을 참조하기 위한 상수(mailbox 비공개 MAILBOX_CAP 미러 — 값 동기).
    const MAILBOX_CAP_FOR_TEST: usize = 100;

    /// 테스트에서 mailbox TTL(24h)을 참조하기 위한 상수(mailbox 비공개 PARK_TTL 미러 — 값 동기, ADR-0105).
    const PARK_TTL_FOR_TEST: Duration = Duration::from_secs(24 * 60 * 60);

    #[test]
    fn cap_exceeded_rejects_mailbox_full() {
        // 같은 부재 이름에 cap(100)까지 파킹 후 101번째 → MAILBOX_FULL 반려.
        let (svc, port) = svc();
        port.set_roster(vec![]);
        for i in 0..100 {
            svc.handle_single_send(
                &format!("m{i}"),
                ident(),
                "s",
                "full",
                "x",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("cap 이내 파킹");
        }
        assert_eq!(svc.parked_len("full"), 100);
        let rej = svc
            .handle_single_send(
                "over",
                ident(),
                "s",
                "full",
                "x",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect_err("cap 초과는 반려");
        assert_eq!(rej, SendReject::MailboxFull);
        assert_eq!(svc.parked_len("full"), 100, "반려된 건 큐에 안 들어감");
        // 반려는 ledger 에 기록하지 않는다(저장 안 됨 — 발신자에게 반려로 즉시 가시화).
        assert!(
            svc.ledger_statuses("over").is_empty(),
            "cap 반려는 ledger 미기록"
        );
    }

    #[test]
    fn sweep_expires_parked_and_records_in_ledger() {
        // 파킹 후 TTL 초과 sweep → ledger pending→expired(장부 잔존, spec §5). 즉시 주입 없음이 전제.
        let (svc, port) = svc();
        port.set_roster(vec![]);
        svc.handle_single_send(
            "m1",
            ident(),
            "s",
            "ghost",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Pending]);
        // TTL(24h) + 1s 뒤 sweep — 파킹 시각은 handle_single_send 내부의 Instant::now() 라, 여기선 그보다
        //   충분히 미래인 now 를 준다. mailbox PARK_TTL 미러(PARK_TTL_FOR_TEST, ADR-0105) 사용 — 값
        //   변경 시 여기도 함께 갱신.
        let now = Instant::now() + PARK_TTL_FOR_TEST + Duration::from_secs(1);
        svc.sweep(now);
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Expired],
            "sweep = pending→expired 장부 전이"
        );
        assert_eq!(svc.parked_len("ghost"), 0, "만료분 큐에서 제거");
    }

    #[test]
    fn exact_id_of_non_reachable_recipient_parks_under_canonical_name() {
        // ★finding 4 회귀★: exact AgentId 로 지목했으나 그 에이전트가 산-비도달(TUI, 로스터 제외)이면
        //   resolve 는 None → 파킹. 이때 UUID 로 park 하면 name-keyed flush 가 영영 못 잡는다 — canonical
        //   NAME 으로 park 해야 그 이름이 도달 가능해질 때 flush 가 배달한다.
        let (svc, port) = svc();
        let tui_id = AgentId::new_v4();
        // 로스터엔 없지만(비-도달) canonical 이름은 "tui-agent" 인 산 에이전트 모사.
        port.set_roster(vec![]);
        port.set_canonical(tui_id, "tui-agent");
        let out = svc
            .handle_single_send(
                "m1",
                ident(),
                "s",
                &tui_id.to_string(), // exact AgentId 지목
                "hi",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("park");
        assert!(matches!(out, SendOutcome::Parked { .. }));
        // ★핵심★: park 키가 UUID 가 아니라 canonical name.
        assert_eq!(
            svc.parked_len("tui-agent"),
            1,
            "exact-id 산-비도달 지목은 canonical name 으로 park(finding 4)"
        );
        assert_eq!(
            svc.parked_len(&tui_id.to_string()),
            0,
            "UUID 키로는 park 되지 않아야(그러면 flush 가 못 잡음)"
        );
        // 이제 그 이름이 도달 가능해지면(등장 flush) 배달된다.
        let (reach_id, mut reachable) = live("tui-agent");
        reachable.id = reach_id;
        port.set_roster(vec![reachable]);
        svc.flush_for("tui-agent", reach_id);
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
    }

    #[test]
    fn park_then_late_appearance_self_heals_delivery() {
        // ★finding 3 회귀(park/appearance TOCTOU)★: resolve 시점엔 부재라 park 했지만, park 직후 그 이름이
        //   유일 도달이면(등장이 resolve↔park 사이 끼어들어 flush observer 가 빈 큐를 지나친 상황을 모사)
        //   self-heal 이 즉시 flush 를 돌려 배달한다. 여기선 park 전 roster 는 비어 resolve=None(park) 이지만,
        //   park **후** self_heal_if_live 가 보는 roster 에 그 이름이 있게 세팅해 late appearance 를 모사한다.
        let (svc, port) = svc();
        // resolve 시점 roster 는 비어 있음(부재 → park).
        port.set_roster(vec![]);
        // ★late appearance 모사★: on_inject hook 없이, park 직후 self_heal 이 roster 를 다시 읽을 때
        //   그 이름이 뜨도록 미리 세팅할 수는 없다(resolve 도 같은 roster 를 본다). 대신 handle_single_send
        //   호출 전에 roster 를 채우면 resolve 가 잡아 즉시 배달돼 버린다. 그래서 resolve 는 빈 roster,
        //   self_heal 은 채워진 roster 를 보게 하려면 roster 를 "1회 조회 후 교체" 해야 한다.
        //   → live_reachable_agents 를 2회차부터 채우는 스크립트 훅을 쓴다.
        let (late_id, late) = live("late");
        port.arm_roster_after_first_call(vec![late]);
        let out = svc
            .handle_single_send(
                "m1",
                ident(),
                "s",
                "late",
                "hi",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("park");
        assert!(
            matches!(out, SendOutcome::Parked { .. }),
            "resolve 시점 부재 → park"
        );
        // self-heal 이 late appearance 를 잡아 즉시 배달했어야.
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Delivered],
            "park 직후 유일 도달이면 self-heal 이 즉시 배달(finding 3)"
        );
        assert_eq!(svc.parked_len("late"), 0, "self-heal flush 로 큐 비움");
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="s">hi</message>"#.to_string()],
            "self-heal 배달도 현재 포맷 봉투"
        );
        let _ = late_id;
    }

    #[test]
    fn self_heal_skips_when_name_ambiguous_after_park() {
        // finding 3 경계: park 후 그 이름이 동명 다수면 self-heal 안 함(finding 2 정합) — 이름이 다시
        //   유일해질 때 flush observer 가 잡는다. 여기선 resolve 시점 부재(park) → self_heal roster 에 동명 2개.
        let (svc, port) = svc();
        port.set_roster(vec![]);
        let (_a, dup_a) = live("dup");
        let (_b, dup_b) = live("dup");
        port.arm_roster_after_first_call(vec![dup_a, dup_b]);
        let out = svc
            .handle_single_send(
                "m1",
                ident(),
                "s",
                "dup",
                "hi",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("park");
        assert!(matches!(out, SendOutcome::Parked { .. }));
        // 동명 다수라 self-heal 안 함 → 여전히 파킹(pending).
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Pending]);
        assert_eq!(
            svc.parked_len("dup"),
            1,
            "동명 다수면 self-heal 안 함(파킹 유지)"
        );
        assert!(port.injected_bodies().is_empty());
    }

    #[test]
    fn flush_absent_queue_is_noop() {
        // 파킹이 없는 이름 flush → 주입 없음(방어적 no-op).
        let (svc, port) = svc();
        let (id, a) = live("nobody");
        port.set_roster(vec![a]); // finding 2: flush_for 재검증 통과용(큐가 비어 어차피 no-op).
        svc.flush_for("nobody", id);
        assert!(port.injected_bodies().is_empty());
    }

    #[test]
    fn flush_skips_when_name_ambiguous_at_execution_time() {
        // ★finding 2 회귀★: enqueue 시점엔 유일했으나 execution 시점에 동명 두 번째가 등장했으면(ambiguous)
        //   flush_for 는 skip 하고 메일을 파킹 상태로 남긴다 — enqueue 스냅샷을 맹신하지 않고 현재 로스터로
        //   재검증한다(임의 incarnation 배달 금지, send-side AMBIGUOUS 정합).
        let (svc, port) = svc();
        port.set_roster(vec![]); // 부재 → park.
        svc.handle_single_send(
            "m1",
            ident(),
            "s",
            "dup",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        assert_eq!(svc.parked_len("dup"), 1);
        // execution 시점 로스터엔 동명 2개(ambiguous).
        let (dup_id, dup_a) = live("dup");
        let (_b, dup_b) = live("dup");
        port.set_roster(vec![dup_a, dup_b]);
        svc.flush_for("dup", dup_id);
        assert_eq!(
            svc.parked_len("dup"),
            1,
            "execution 시점 동명 다수 → flush skip(파킹 유지)"
        );
        assert!(
            port.injected_bodies().is_empty(),
            "ambiguous 면 주입 안 함(finding 2)"
        );
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Pending],
            "파킹 유지 pending"
        );
    }

    #[test]
    fn flush_skips_when_target_died_at_execution_time() {
        // ★finding 2 회귀★: enqueue 시점엔 살아 있었으나 execution 시점에 그 이름이 부재(죽음)면 flush_for
        //   는 skip 하고 메일을 파킹 상태로 남긴다 — 죽은 대상에 주입 시도(stale id)를 하지 않는다.
        let (svc, port) = svc();
        port.set_roster(vec![]); // 부재 → park.
        svc.handle_single_send(
            "m1",
            ident(),
            "s",
            "gone",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        assert_eq!(svc.parked_len("gone"), 1);
        // execution 시점에도 로스터엔 그 이름이 없음(enqueue 사이 죽음 — stale id 로 호출).
        let (stale_id, _gone) = live("gone");
        svc.flush_for("gone", stale_id);
        assert_eq!(
            svc.parked_len("gone"),
            1,
            "execution 시점 부재 → flush skip(파킹 유지)"
        );
        assert!(
            port.injected_bodies().is_empty(),
            "죽은 대상엔 주입 안 함(finding 2)"
        );
    }

    #[test]
    fn flush_reinjects_to_current_id_when_supplied_id_is_stale() {
        // ★finding 2 회귀(stale to_id → 현재 id 재바인딩)★: enqueue 시점 (name, id) 로 flush 를 걸었으나
        //   그 사이 수신자가 respawn(같은 이름, **새 AgentId**)했다 — flush_for 는 enqueue 의 stale id 를
        //   맹신하지 않고 execution 시점 로스터로 이름을 재해석해 **현재 유일 후보의 새 id** 로 주입해야 한다.
        //   (앞 두 테스트는 재검증이 skip 으로 가는 축(부재·ambiguous)만 덮었고, 재검증이 **성공하되 id 가
        //   갱신되는** 핵심 축은 미검증이었다 — 이 테스트가 그 갭을 메운다.)
        let (svc, port) = svc();
        port.set_roster(vec![]); // 부재 → park(이름 앞으로 파킹).
        svc.handle_single_send(
            "m1",
            ident(),
            "s",
            "recv",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        assert_eq!(svc.parked_len("recv"), 1);

        // ── respawn 모사: 같은 이름 "recv" 지만 enqueue 때와 다른 새 AgentId 로 로스터에 등장 ──────────
        let stale_id = AgentId::new_v4(); // enqueue 스냅샷이 알던(이제 죽은) 옛 incarnation id.
        let (new_id, recv_new) = live("recv"); // 현재 살아있는 유일 도달 후보(새 id).
        assert_ne!(
            stale_id, new_id,
            "테스트 전제: stale id 와 현재 id 는 달라야 의미가 있다"
        );
        port.set_roster(vec![recv_new]);

        // stale id 로 flush 를 걸어도(observer/enqueue 가 옛 스냅샷을 줬어도) 현재 id 로 배달돼야 한다.
        svc.flush_for("recv", stale_id);

        // 배달 성공(skip 아님) + 큐 비움 + delivered 전이.
        assert_eq!(
            svc.parked_len("recv"),
            0,
            "재바인딩 배달로 큐 비움(skip 아님)"
        );
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Delivered],
            "stale id 여도 현재 유일 후보로 배달 → delivered"
        );
        // ★핵심 단언★: 주입 대상 id 가 stale 이 아니라 **현재 새 id** 여야 한다.
        let injected: Vec<AgentId> = port
            .injected
            .lock()
            .unwrap()
            .iter()
            .map(|(id, _)| *id)
            .collect();
        assert_eq!(
            injected,
            vec![new_id],
            "주입은 stale id 가 아니라 execution 시점 현재 id 로(finding 2 재바인딩)"
        );
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="s">hi</message>"#.to_string()],
            "봉투 내용은 정상 배달분"
        );
    }

    #[test]
    fn park_payload_roundtrip_survives_newline_in_body() {
        // sender/body 경계는 길이 접두라 body 개행에도 안전. from/entrance 도 무손실 복원.
        let from = BoundIdentity {
            agent_id: AgentId::new_v4(),
            epoch: 3,
        };
        let p = ParkPayload {
            sender_name: "qa-alpha".to_string(),
            from,
            entrance: Entrance::Mcp,
            body: "line1\nline2".to_string(),
            meta: SendMeta::default(),
        };
        let d = ParkPayload::decode(&p.encode());
        assert_eq!(d.sender_name, "qa-alpha");
        assert_eq!(d.body, "line1\nline2");
        assert_eq!(d.from.agent_id, from.agent_id);
        assert_eq!(d.from.epoch, 3);
        assert!(matches!(d.entrance, Entrance::Mcp));
        assert_eq!(d.meta, SendMeta::default(), "통보는 계약 메타가 비어야");
    }

    #[test]
    fn park_payload_roundtrip_preserves_contract_meta_with_newlines() {
        // ★C3★: request/회신 메타가 park→flush 를 무손실로 건넌다(늦은 배달도 같은 봉투 — 모듈 헤더).
        //   reply_to 는 **에이전트 입력**이라 개행·멀티바이트가 섞일 수 있다 — 길이 접두 인코딩이 이를 견딘다.
        let from = BoundIdentity {
            agent_id: AgentId::new_v4(),
            epoch: 7,
        };
        let meta = SendMeta {
            request: true,
            reply_by_raw: Some("10m".to_string()),
            // 파싱값은 park 를 건너지 않는다(계약은 이미 장부에 열렸다 — ParkPayload 주석).
            reply_by: Some(Duration::from_secs(600)),
            reply_to: None,
        };
        let p = ParkPayload {
            sender_name: "발신자-한글".to_string(),
            from,
            entrance: Entrance::Daemon,
            body: "본문\n둘째 줄".to_string(),
            meta: meta.clone(),
        };
        let d = ParkPayload::decode(&p.encode());
        assert_eq!(
            d.sender_name, "발신자-한글",
            "멀티바이트 sender 경계(바이트 길이 접두)"
        );
        assert_eq!(d.body, "본문\n둘째 줄");
        assert!(matches!(d.entrance, Entrance::Daemon));
        assert!(d.meta.request, "request 플래그 복원");
        assert_eq!(d.meta.reply_by_raw.as_deref(), Some("10m"), "표기 복원");
        assert_eq!(d.meta.reply_by, None, "파싱값은 의도적으로 미복원");
        assert_eq!(d.meta.reply_to, None);

        // 회신 축(개행 포함 reply_to) — 길이 접두 경계 검증.
        let reply = ParkPayload {
            sender_name: "s".to_string(),
            from,
            entrance: Entrance::Cli,
            body: "b".to_string(),
            meta: SendMeta {
                request: false,
                reply_by_raw: None,
                reply_by: None,
                reply_to: Some("m-abc\nxyz".to_string()),
            },
        };
        let d = ParkPayload::decode(&reply.encode());
        assert_eq!(d.meta.reply_to.as_deref(), Some("m-abc\nxyz"));
        assert_eq!(d.body, "b");
        assert!(!d.meta.request);
    }

    // ── C2: idle 게이트(spec §5 주입 타이밍 · ADR-0104 결정 3) ────────────────────────────────

    #[test]
    fn busy_recipient_parks_pending_without_inject() {
        // ★핵심 분기★: 산·도달 수신자인데 턴 진행 중 → 주입 금지, 파킹(pending). 상태 어휘는 부재 파킹과
        //   공유하고(새 상태 발명 금지) hint 만 사유를 구분한다.
        let (svc, port, gate) = svc_gated();
        let (alice_id, alice) = live("alice");
        port.set_roster(vec![alice]);
        gate.set_busy(alice_id, 0);
        let out = svc
            .handle_single_send(
                "m1",
                ident(),
                "bob",
                "alice",
                "hi",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("busy 파킹은 반려 아님");
        match out {
            SendOutcome::Parked { hint } => {
                assert!(
                    hint.contains("mid-turn"),
                    "hint 가 busy 사유를 알린다: {hint}"
                )
            }
            other => panic!("busy 수신자는 파킹이어야: {other:?}"),
        }
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Pending],
            "busy 파킹 = pending 장부(delivered 는 실제 주입 시점에만 — ADR-0104)"
        );
        assert_eq!(svc.parked_len("alice"), 1, "이름 큐에 대기");
        assert!(
            port.injected_bodies().is_empty(),
            "턴 중에는 stdin 에 밀지 않는다(CLI 큐 우회 금지)"
        );
    }

    #[test]
    fn idle_recipient_injects_immediately() {
        // 게이트가 있어도 idle 이면 C1 과 동일하게 즉시 주입 → delivered.
        let (svc, port, _gate) = svc_gated();
        let (_id, alice) = live("alice");
        port.set_roster(vec![alice]);
        let out = svc
            .handle_single_send(
                "m1",
                ident(),
                "bob",
                "alice",
                "hi",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("no reject");
        assert_eq!(out, SendOutcome::Delivered);
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="bob">hi</message>"#.to_string()]
        );
    }

    #[test]
    fn unobserved_recipient_is_treated_idle_and_injected() {
        // ★폴백 불변식(ADR-0104 영향 절)★: 관측 근거가 없으면(다른 epoch 만 busy 로 알려진 등) idle 취급 →
        //   즉시 주입. "모른다" 를 busy 로 읽으면 관측 불가 백엔드에서 배달이 영구 대기한다.
        let (svc, port, gate) = svc_gated();
        let (alice_id, alice) = live("alice"); // 로스터 epoch = 0
        port.set_roster(vec![alice]);
        gate.set_busy(alice_id, 1); // 옛/다른 incarnation 만 busy 로 알려짐
        let out = svc
            .handle_single_send(
                "m1",
                ident(),
                "bob",
                "alice",
                "hi",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("no reject");
        assert_eq!(
            out,
            SendOutcome::Delivered,
            "미관측 (id, epoch) 는 idle 폴백 → 즉시 주입"
        );
    }

    #[test]
    fn multiple_parked_during_busy_flush_as_one_batch_oldest_first_on_idle() {
        // ★spec §7 배치 검증 중점★: busy 중 3건 도착 → 전부 파킹(주입 0) → 턴 종료(게이트 idle) 후 idle
        //   flush 1회에 **오래된 순 일괄** 주입(각자 개별 봉투). 드리블(1건씩) 금지.
        let (svc, port, gate) = svc_gated();
        let (recv_id, recv) = live("recv");
        port.set_roster(vec![recv]);
        gate.set_busy(recv_id, 0);
        for (i, m) in ["m0", "m1", "m2"].iter().enumerate() {
            let out = svc
                .handle_single_send(
                    m,
                    ident(),
                    "s",
                    "recv",
                    &format!("b{i}"),
                    Entrance::Mcp,
                    &SendMeta::default(),
                )
                .expect("park");
            assert!(matches!(out, SendOutcome::Parked { .. }));
        }
        assert_eq!(svc.parked_len("recv"), 3);
        assert!(port.injected_bodies().is_empty(), "턴 중엔 주입 0");

        // 턴 종료 → idle 트리거(worker 가 하는 일 = flush_for_agent(id)).
        gate.clear();
        svc.flush_for_agent(recv_id);
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="s">b0</message>"#.to_string(),
                r#"<message from="s">b1</message>"#.to_string(),
                r#"<message from="s">b2</message>"#.to_string(),
            ],
            "idle 진입 시 오래된 순 개별 봉투 일괄 주입(ADR-0104)"
        );
        assert_eq!(svc.ledger_statuses("m0"), vec![DeliveryStatus::Delivered]);
        assert_eq!(svc.ledger_statuses("m2"), vec![DeliveryStatus::Delivered]);
        assert_eq!(svc.parked_len("recv"), 0);
    }

    #[test]
    fn busy_park_self_heals_when_turn_ends_during_park() {
        // ★busy-park TOCTOU 자가치유(C1 finding 3 와 대칭)★: 게이트 확인(busy)↔park 사이에 턴이 끝나면
        //   idle 트리거는 빈 큐를 이미 지나갔다(lost wakeup) → park 직후 재확인이 idle 이면 즉시 flush.
        let (svc, port, gate) = svc_gated();
        let (recv_id, recv) = live("recv");
        port.set_roster(vec![recv]);
        gate.set_busy(recv_id, 0);
        // 첫 조회(게이트 확인)만 busy, 그 뒤(재확인)는 idle → self-heal 발동.
        gate.arm_idle_after_call(1);
        let out = svc
            .handle_single_send(
                "m1",
                ident(),
                "s",
                "recv",
                "hi",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("park");
        assert!(
            matches!(out, SendOutcome::Parked { .. }),
            "발신 응답은 파킹(pending) — self-heal 은 그 뒤의 배달"
        );
        assert_eq!(gate.call_count(), 2, "확인 + park 후 재확인 = 2회");
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Delivered],
            "park 직후 idle 이면 self-heal flush 로 즉시 배달(pending→delivered)"
        );
        assert_eq!(svc.parked_len("recv"), 0, "self-heal flush 로 큐 비움");
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="s">hi</message>"#.to_string()]
        );
    }

    #[test]
    fn busy_park_does_not_self_heal_while_still_busy() {
        // 경계: 재확인도 busy 면 self-heal 하지 않는다(파킹 유지 — 턴 종료 트리거를 기다린다).
        let (svc, port, gate) = svc_gated();
        let (recv_id, recv) = live("recv");
        port.set_roster(vec![recv]);
        gate.set_busy(recv_id, 0);
        svc.handle_single_send(
            "m1",
            ident(),
            "s",
            "recv",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        assert_eq!(svc.parked_len("recv"), 1, "여전히 busy → 파킹 유지");
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Pending]);
        assert!(port.injected_bodies().is_empty());
    }

    #[test]
    fn flush_skips_while_busy_then_delivers_whole_batch_when_idle() {
        // ★fix 4★: 게이트는 **배치 시작 전 1회**만 본다.
        //   ① pre-drain 확인이 busy 면 배치를 시작하지 않는다(파킹 유지 — 턴 종료 통지가 재시도).
        //   ② 일단 시작하면 mid-batch 재검사를 하지 않는다(첫 주입의 유저 에코가 busy 를 만들어 배치가
        //      1건 만에 끊기는 드리블 방지 = ADR-0104 거부 대안). 게이트 조회 횟수로 그 "1회" 를 못 박는다.
        let (svc, port, gate) = svc_gated();
        let (recv_id, recv) = live("recv");
        // 먼저 부재 상태로 3건 파킹(게이트 무관 경로).
        port.set_roster(vec![]);
        for (i, m) in ["m0", "m1", "m2"].iter().enumerate() {
            svc.handle_single_send(
                m,
                ident(),
                "s",
                "recv",
                &format!("b{i}"),
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("park");
        }
        // 등장했지만 턴 중 → flush 스킵(주입 0, 파킹 유지).
        port.set_roster(vec![recv]);
        gate.set_busy(recv_id, 0);
        let calls_before = gate.call_count();
        svc.flush_for("recv", recv_id);
        assert!(
            port.injected_bodies().is_empty(),
            "턴 중 flush 는 배치를 시작하지 않는다(pre-drain 게이트)"
        );
        assert_eq!(svc.parked_len("recv"), 3, "파킹 유지");
        assert_eq!(
            gate.call_count() - calls_before,
            1,
            "게이트 조회는 배치당 1회(pre-drain)"
        );
        // 턴 종료 → 전량 배달, 그 사이 게이트를 다시 보지 않는다.
        gate.clear();
        let calls_before = gate.call_count();
        svc.flush_for("recv", recv_id);
        assert_eq!(
            port.injected_bodies().len(),
            3,
            "idle 이면 배치 완주(mid-batch 재검사 없음 — 드리블 금지)"
        );
        assert_eq!(
            gate.call_count() - calls_before,
            1,
            "3건 배치에도 게이트 조회는 1회뿐"
        );
        assert_eq!(svc.parked_len("recv"), 0);
    }

    #[test]
    fn flush_skip_while_busy_still_ledgers_expired_items() {
        // 게이트 스킵이 **조용한 유실**을 만들지 않는지: 스킵 경로도 drain 을 거치므로 만료분은 장부에
        //   expired 로 남고 미만료분은 원래 순서로 복원된다(restore_ordered — cap 우회·순번 merge).
        let (svc, port, gate) = svc_gated();
        let (recv_id, recv) = live("recv");
        port.set_roster(vec![recv.clone()]);
        // busy 상태에서 2건 파킹.
        gate.set_busy(recv_id, 0);
        for (i, m) in ["m0", "m1"].iter().enumerate() {
            svc.handle_single_send(
                m,
                ident(),
                "s",
                "recv",
                &format!("b{i}"),
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("park");
        }
        assert_eq!(svc.parked_len("recv"), 2);
        // 여전히 busy → flush 스킵. 큐·장부 그대로.
        svc.flush_for("recv", recv_id);
        assert_eq!(svc.parked_len("recv"), 2, "스킵은 큐를 건드리지 않는다");
        assert_eq!(svc.ledger_statuses("m0"), vec![DeliveryStatus::Pending]);
        // idle 이 되면 오래된 순 그대로 나간다(복원 순서 회귀 방지).
        gate.clear();
        svc.flush_for("recv", recv_id);
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="s">b0</message>"#.to_string(),
                r#"<message from="s">b1</message>"#.to_string(),
            ]
        );
    }

    #[test]
    fn exact_id_send_to_busy_agent_is_delivered_via_id_hint_despite_duplicate_names() {
        // ★fix 2 회귀(blackhole)★: 동명 둘 중 하나를 **exact-AgentId** 로 지목했고 그가 턴 중이면, 파킹은
        //   이름-키라 flush 의 유일성 게이트가 영영 보류한다(TTL 까지 배달 안 됨). park 항목의 id 힌트가
        //   그 사각지대를 메운다 — 힌트가 살아 있으면 이름 유일성과 무관하게 그 id 로 배달.
        let (svc, port, gate) = svc_gated();
        let (id_a, dup_a) = live("dup");
        let (_id_b, dup_b) = live("dup"); // 같은 이름의 두 번째 에이전트(이름 해석은 영구 ambiguous).
        port.set_roster(vec![dup_a, dup_b]);
        gate.set_busy(id_a, 0);
        // exact-id 지목(동명 모호성을 의도적으로 통과) → busy 라 파킹.
        let out = svc
            .handle_single_send(
                "m1",
                ident(),
                "s",
                &id_a.to_string(),
                "hi",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("park");
        assert!(matches!(out, SendOutcome::Parked { .. }));
        assert_eq!(svc.parked_len("dup"), 1, "park 키는 여전히 이름(dup)");
        // 턴 종료 → id 지목 flush. 이름은 아직 동명 다수(유일성 게이트로는 배달 불가)여야 한다.
        gate.clear();
        svc.flush_for_agent(id_a);
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="s">hi</message>"#.to_string()],
            "동명 다수여도 id 힌트로 배달돼야(blackhole 금지)"
        );
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
        assert_eq!(svc.parked_len("dup"), 0);
        // 배달 대상이 힌트의 id 였는지(엉뚱한 동명이 아니라) 확인.
        assert_eq!(
            port.injected.lock().unwrap()[0].0,
            id_a,
            "힌트가 지목한 그 incarnation 에 배달"
        );
    }

    #[test]
    fn dead_id_hint_falls_back_to_unique_name_rule_for_respawn() {
        // ★힌트는 권위가 아니라 우선순위★: 힌트 id 가 죽었으면(재스폰) 무시하고 이름 규칙으로 배달한다 —
        //   "재스폰된 동명이 파킹을 이어받는다" 는 이름-키 설계(canonical_park_key)가 유지돼야 한다.
        let (svc, port, gate) = svc_gated();
        let (old_id, old_agent) = live("recv");
        port.set_roster(vec![old_agent]);
        gate.set_busy(old_id, 0);
        svc.handle_single_send(
            "m1",
            ident(),
            "s",
            "recv",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        assert_eq!(svc.parked_len("recv"), 1);
        // 재스폰: 옛 id 는 사라지고 같은 이름의 **새 AgentId** 가 등장.
        gate.clear();
        let (new_id, new_agent) = live("recv");
        port.set_roster(vec![new_agent]);
        svc.flush_for("recv", new_id);
        assert_eq!(
            port.injected.lock().unwrap()[0].0,
            new_id,
            "죽은 힌트는 무시 — 이름 유일 도달(재스폰분)로 배달"
        );
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
    }

    #[test]
    fn idle_direct_send_joins_existing_queue_preserving_recipient_order() {
        // ★fix 5 회귀(FIFO 일관성)★: 큐에 m0·m1 이 대기 중인데 새 m2 가 직발송으로 앞지르면 수신자는
        //   (m2, m0, m1) 순서로 본다. 큐가 비어 있지 않으면 직발송도 큐에 합류해야 한다.
        let (svc, port, gate) = svc_gated();
        let (recv_id, recv) = live("recv");
        port.set_roster(vec![recv]);
        gate.set_busy(recv_id, 0);
        for (i, m) in ["m0", "m1"].iter().enumerate() {
            svc.handle_single_send(
                m,
                ident(),
                "s",
                "recv",
                &format!("b{i}"),
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("park");
        }
        // 턴 종료(게이트 idle) — 하지만 flush 를 아직 돌리지 않은 상태에서 새 발송이 들어온다.
        gate.clear();
        let out = svc
            .handle_single_send(
                "m2",
                ident(),
                "s",
                "recv",
                "b2",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("no reject");
        assert!(
            matches!(out, SendOutcome::Parked { .. }),
            "큐가 비어 있지 않으면 직발송도 파킹(합류): {out:?}"
        );
        // 도어벨 미배선 조립이라 park 직후 인라인 flush 가 돌아 배치가 순서대로 나간다.
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="s">b0</message>"#.to_string(),
                r#"<message from="s">b1</message>"#.to_string(),
                r#"<message from="s">b2</message>"#.to_string(),
            ],
            "수신자가 보는 순서 = 도착 순서(직발송이 큐를 앞지르지 않는다)"
        );
        assert_eq!(svc.parked_len("recv"), 0);
    }

    #[test]
    fn empty_queue_idle_send_still_injects_directly() {
        // fix 5 의 경계: 앞지를 대상이 없으면(큐 비어 있음) 직발송 그대로 = C1 동작(불필요한 파킹 금지).
        let (svc, port, _gate) = svc_gated();
        let (_id, recv) = live("recv");
        port.set_roster(vec![recv]);
        let out = svc
            .handle_single_send(
                "m1",
                ident(),
                "s",
                "recv",
                "hi",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("no reject");
        assert_eq!(out, SendOutcome::Delivered, "큐가 비면 즉시 주입");
        assert_eq!(port.injected_bodies().len(), 1);
    }

    #[test]
    fn busy_park_rings_doorbell_instead_of_flushing_on_sender_thread() {
        // ★fix 11 회귀★: 도어벨이 배선돼 있으면 자가치유는 **발신 스레드에서 flush 하지 않는다** —
        //   요청만 남기고 즉시 반환(배치 blocking write 가 MCP/HTTP 워커를 잡지 않게).
        let (svc, port, gate, bell) = svc_gated_with_doorbell();
        let (recv_id, recv) = live("recv");
        port.set_roster(vec![recv]);
        gate.set_busy(recv_id, 0);
        svc.handle_single_send(
            "m1",
            ident(),
            "s",
            "recv",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        assert_eq!(
            bell.seen(),
            vec![recv_id],
            "도어벨 요청 1건(무조건 enqueue)"
        );
        assert!(
            port.injected_bodies().is_empty(),
            "발신 스레드에서 주입하지 않는다(소비는 flush lane)"
        );
        assert_eq!(svc.parked_len("recv"), 1, "파킹 유지");
        // 소비자(flush lane)가 도어벨을 처리하면 배달된다 — 턴 종료 후.
        gate.clear();
        svc.flush_for_agent(recv_id);
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
    }

    #[test]
    fn absent_park_rings_no_doorbell_when_name_stays_absent() {
        // 등장하지 않았으면 도어벨도 울리지 않는다(잉여 요청 억제 — 그 이름은 등장 flush 가 잡는다).
        let (svc, port, _gate, bell) = svc_gated_with_doorbell();
        port.set_roster(vec![]);
        svc.handle_single_send(
            "m1",
            ident(),
            "s",
            "ghost",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        assert!(bell.seen().is_empty(), "부재 그대로면 도어벨 없음");
        assert_eq!(svc.parked_len("ghost"), 1);
    }

    #[test]
    fn absent_park_late_appearance_rings_doorbell_instead_of_flushing_inline() {
        // 부재 파킹 자가치유(finding 3)도 도어벨로 나간다 — park↔등장 레이스의 lost wakeup 을 메우되
        //   실제 flush 는 lane 이 한다(발신 스레드에서 배치 write 금지).
        //   ★roster_calls 는 서비스 단위 누적이라 이 시나리오는 fresh 조립에서 본다★(첫 조회 = resolve).
        let (svc, port, _gate, bell) = svc_gated_with_doorbell();
        let (late_id, late) = live("late");
        port.set_roster(vec![]);
        port.arm_roster_after_first_call(vec![late]);
        svc.handle_single_send(
            "m1",
            ident(),
            "s",
            "late",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        assert_eq!(
            bell.seen(),
            vec![late_id],
            "park 직후 유일 도달로 관측되면 도어벨"
        );
        assert!(
            port.injected_bodies().is_empty(),
            "발신 스레드에서 주입하지 않는다(소비는 flush lane)"
        );
        assert_eq!(svc.parked_len("late"), 1, "배달은 lane 이 할 몫");
    }

    // ── round-3 finding 2: 턴 중 rename 이 옛 이름 큐를 고아로 만들지 않는다 ────────────────────
    #[test]
    fn rename_mid_turn_still_flushes_old_name_queue_via_id_hint() {
        // ★round-3 finding 2 회귀★: busy 파킹은 **발송 시점** 이름 큐에 들어가고, 턴 종료 flush 는 **현재**
        //   이름으로 진입한다(tap 은 id 만 안다). 턴 중에 이름이 바뀌면 옛 이름 큐를 아무도 열지 않아 TTL 까지
        //   stranded 된다 — id 힌트 역방향 조회로 그 큐도 함께 연다.
        let (svc, port, gate) = svc_gated();
        let (id, agent) = live("old-name");
        port.set_roster(vec![agent]);
        gate.set_busy(id, 0);
        svc.handle_single_send(
            "m1",
            ident(),
            "s",
            "old-name",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        assert_eq!(svc.parked_len("old-name"), 1, "발송 시점 이름 큐에 파킹");

        // 턴 중 rename — 같은 id, 새 canonical 이름.
        port.set_roster(vec![LiveAgent {
            id,
            name: "new-name".to_string(),
            epoch: 0,
        }]);
        gate.clear(); // 턴 종료.
        svc.flush_for_agent(id); // 턴 종료 통지 = id 입구(이름을 모른다).

        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="s">hi</message>"#.to_string()],
            "rename 후에도 옛 이름 큐가 배달돼야(고아 금지 — finding 2)"
        );
        assert_eq!(svc.parked_len("old-name"), 0, "옛 이름 큐 비움");
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
    }

    #[test]
    fn rename_flush_ignores_queues_without_a_hint_for_that_id() {
        // 힌트 없는 부재 파킹("없는 이름" 선지시)은 id 입구가 열지 않는다 — 그건 이름 규칙(등장 flush)의 몫
        //   이고, 여기서 열면 엉뚱한 이름 앞 메일을 이 id 로 배달할 수 있다(이름 주소 계약 위반).
        let (svc, port, _gate) = svc_gated();
        let (id, agent) = live("recv");
        port.set_roster(vec![]); // 부재 → 힌트 없는 파킹.
        svc.handle_single_send(
            "m1",
            ident(),
            "s",
            "somebody-else",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        port.set_roster(vec![agent]);
        svc.flush_for_agent(id);
        assert!(
            port.injected_bodies().is_empty(),
            "힌트 없는 다른 이름 큐는 id 입구가 건드리지 않는다"
        );
        assert_eq!(svc.parked_len("somebody-else"), 1);
    }

    // ── round-3 finding 3: 한 배치가 여러 타깃으로 갈릴 때 게이트는 타깃별로 ──────────────────────
    /// 동명 두 에이전트에게 **exact-id 지목**으로 각각 파킹시킨다(둘 다 busy) — 이름-키 한 큐에 서로 다른
    ///   타깃 힌트가 섞인 배치를 만드는 유일한 경로다(fix 2 의 exact-id 통과 + 동명 충돌).
    fn parked_mixed_batch(
        svc: &Arc<MessagingService>,
        port: &Arc<FakeDeliveryPort>,
        gate: &Arc<FakeGate>,
        order: &[(usize, &str)],
    ) -> (AgentId, AgentId) {
        let (id_a, dup_a) = live("dup");
        let (id_b, dup_b) = live("dup");
        port.set_roster(vec![dup_a, dup_b]);
        gate.set_busy(id_a, 0);
        gate.set_busy(id_b, 0);
        for (which, msg_id) in order {
            let to = if *which == 0 { id_a } else { id_b };
            svc.handle_single_send(
                msg_id,
                ident(),
                "s",
                &to.to_string(),
                &format!("body-{msg_id}"),
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("park");
        }
        (id_a, id_b)
    }

    #[test]
    fn mixed_target_batch_gates_each_target_independently() {
        // ★round-3 finding 3 회귀(핵심 버그)★: [A(곧 idle) 힌트, B(계속 busy) 힌트] 배치에서 옛 코드는
        //   **첫 항목의 타깃만** 게이트했다 → A 가 idle 이면 배치를 시작하고 두 번째 항목을 **턴 중인 B 에게
        //   주입**했다(게이트 우회). 이제 타깃별로 1회씩 게이트한다.
        let (svc, port, gate) = svc_gated();
        let (id_a, id_b) = parked_mixed_batch(
            &svc,
            &port,
            &gate,
            &[(0, "m0"), (1, "m1"), (0, "m2"), (1, "m3")],
        );
        assert_eq!(
            svc.parked_len("dup"),
            4,
            "한 이름 큐에 두 타깃 몫이 섞여 있다"
        );

        // A 만 턴 종료(B 는 여전히 턴 중).
        gate.clear();
        gate.set_busy(id_b, 0);
        let calls_before = gate.call_count();
        svc.flush_for("dup", id_a);

        assert_eq!(
            gate.call_count() - calls_before,
            2,
            "게이트 조회 = 배치 안 **서로 다른 타깃 수**(항목 수가 아니다)"
        );
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="s">body-m0</message>"#.to_string(),
                r#"<message from="s">body-m2</message>"#.to_string(),
            ],
            "idle 타깃 몫만 오래된 순으로 배달"
        );
        let targets: Vec<AgentId> = port
            .injected
            .lock()
            .unwrap()
            .iter()
            .map(|(id, _)| *id)
            .collect();
        assert_eq!(targets, vec![id_a, id_a], "턴 중인 B 에는 한 건도 안 간다");
        assert_eq!(svc.parked_len("dup"), 2, "busy 타깃 몫은 파킹 유지");
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Pending]);
        assert_eq!(svc.ledger_statuses("m3"), vec![DeliveryStatus::Pending]);

        // B 도 턴 종료 → 남은 몫이 **원래 순서**로 나간다(복원이 순서를 뒤집지 않았다는 증거).
        gate.clear();
        svc.flush_for("dup", id_b);
        assert_eq!(
            port.injected_bodies()[2..],
            [
                r#"<message from="s">body-m1</message>"#.to_string(),
                r#"<message from="s">body-m3</message>"#.to_string(),
            ],
            "복원된 몫도 오래된 순 보존"
        );
        assert_eq!(svc.parked_len("dup"), 0);
    }

    #[test]
    fn mixed_target_batch_busy_first_still_delivers_the_idle_target() {
        // 반대 방향: **첫 항목의 타깃이 busy** 인 경우. 옛 코드는 배치 전량을 되돌려 idle 인 A 몫까지
        //   근거 없이 지연시켰다(첫 항목만 보는 게이트의 다른 얼굴). 이제 A 는 그대로 배달된다.
        let (svc, port, gate) = svc_gated();
        let (id_a, id_b) = parked_mixed_batch(&svc, &port, &gate, &[(1, "m0"), (0, "m1")]);
        gate.clear();
        gate.set_busy(id_b, 0); // B(첫 항목 타깃)만 여전히 턴 중.
        svc.flush_for("dup", id_b);
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="s">body-m1</message>"#.to_string()],
            "첫 항목 타깃이 busy 여도 idle 타깃 몫은 배달"
        );
        assert_eq!(port.injected.lock().unwrap()[0].0, id_a);
        assert_eq!(svc.parked_len("dup"), 1, "busy 타깃 몫만 남는다");
        assert_eq!(svc.ledger_statuses("m0"), vec![DeliveryStatus::Pending]);
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
    }

    #[test]
    fn mixed_target_batch_inject_failure_does_not_block_the_other_target() {
        // 한 타깃의 inject 실패는 그 타깃의 남은 몫만 되돌린다 — 다른 에이전트의 배달을 막을 근거가 없다.
        let (svc, port, gate) = svc_gated();
        let (id_a, _id_b) = parked_mixed_batch(&svc, &port, &gate, &[(0, "m0"), (1, "m1")]);
        gate.clear();
        port.fail_at(&[0]); // 첫 주입(A 몫) 실패.
        svc.flush_for("dup", id_a);
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="s">body-m1</message>"#.to_string()],
            "실패한 타깃 몫만 재파킹, 다른 타깃은 배달"
        );
        assert_eq!(svc.parked_len("dup"), 1);
        assert_eq!(svc.ledger_statuses("m0"), vec![DeliveryStatus::Pending]);
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
    }

    #[test]
    fn multi_group_inject_failure_keeps_queue_globally_oldest_first() {
        // ★round-4 finding 1 회귀(핵심 버그)★: 한 이름 큐에 두 타깃 몫이 섞인 배치에서 **두 그룹 다** inject
        //   실패하면 복원이 두 번 일어난다. 옛 FRONT 삽입은 두 번째(B 몫)가 첫 번째(A 몫) **앞**에 꽂혀
        //   큐가 [m1,m3,m0,m2] 로 뒤집혔다 — 수신자가 보는 순서가 역전되고, 나이 역전은 sweep 의 만료 은폐로도
        //   번진다. admission 순번 merge 는 복원 횟수와 무관하게 [m0,m1,m2,m3] 를 유지한다.
        let (svc, port, gate) = svc_gated();
        let (id_a, _id_b) = parked_mixed_batch(
            &svc,
            &port,
            &gate,
            &[(0, "m0"), (1, "m1"), (0, "m2"), (1, "m3")],
        );
        assert_eq!(
            svc.parked_msg_ids("dup"),
            vec!["m0", "m1", "m2", "m3"],
            "park 직후 큐 = 수용 순서"
        );
        gate.clear(); // 둘 다 idle → 두 그룹 모두 배치 시작.
        port.fail_at(&[0, 1]); // 각 그룹의 첫 주입이 실패 → 그룹마다 남은 몫 전량 복원.
        svc.flush_for("dup", id_a);

        assert!(port.injected_bodies().is_empty(), "전부 실패 = 배달 0건");
        assert_eq!(
            svc.parked_msg_ids("dup"),
            vec!["m0", "m1", "m2", "m3"],
            "그룹별 복원이 두 번 일어나도 큐는 전역 오래된 순(복원 순서에 의한 역전 없음)"
        );
        for id in ["m0", "m1", "m2", "m3"] {
            assert_eq!(
                svc.ledger_statuses(id),
                vec![DeliveryStatus::Pending],
                "재파킹분은 pending 유지(무손실)"
            );
        }
    }

    #[test]
    fn busy_skip_and_inject_failure_restores_stay_oldest_first() {
        // ★두 복원 경로가 섞이는 케이스(round-4 finding 1)★: busy 스킵분은 **락 안**에서, inject 실패분은
        //   **락 밖**에서 되돌아온다 — 서로 다른 호출이라 옛 FRONT 삽입에선 나중 것(실패분)이 앞에 꽂혔다.
        //   여기선 A(m0, busy 유지) 스킵 + B(m1, 주입 실패) 복원이 [m0,m1] 순서를 지켜야 한다.
        let (svc, port, gate) = svc_gated();
        let (id_a, id_b) = parked_mixed_batch(&svc, &port, &gate, &[(0, "m0"), (1, "m1")]);
        gate.clear();
        gate.set_busy(id_a, 0); // A 는 여전히 턴 중 → 락 안에서 m0 복원.
        port.fail_at(&[0]); // B 몫(m1) 주입 실패 → 락 밖에서 m1 복원.
        svc.flush_for("dup", id_b);

        assert!(port.injected_bodies().is_empty(), "배달 0건");
        assert_eq!(
            svc.parked_msg_ids("dup"),
            vec!["m0", "m1"],
            "락 안 스킵분(더 오래됨)이 락 밖 실패분보다 앞에 남아야"
        );
    }

    // ── round-3 finding 4: busy 상한 sweep 이 깨울 수 없는 busy 를 풀고 대기 메일을 배달 ────────────
    /// tap 부착을 하지 않는 no-op TapHost — 이 테스트는 `BusyTracker` 를 **게이트로만** 쓰고 표는 하네스
    ///   seam 으로 조작한다(실 PTY·claude 없이 "상한 sweep → 배달" 전 구간을 잇는다).
    struct NoSubscribeTapHost;
    impl super::super::busy::TapHost for NoSubscribeTapHost {
        fn subscribe_output(
            &self,
            _id: AgentId,
            _expect_epoch: u32,
            _sink: Arc<dyn engram_dashboard_core::agent::types::OutputSink>,
        ) -> Result<(), super::super::busy::SubscribeError> {
            Ok(())
        }
        fn current_epoch(&self, _id: AgentId) -> Option<u32> {
            None
        }
    }

    /// 도어벨 통지를 기록하는 IdleNotifier(운영은 flush 채널) — sweep 이 **깨우는지**를 단언한다.
    struct RecordingIdle {
        seen: StdMutex<Vec<AgentId>>,
    }
    impl super::super::busy::IdleNotifier for RecordingIdle {
        fn notify_idle(&self, id: AgentId) {
            self.seen.lock().unwrap().push(id);
        }
    }

    #[test]
    fn stale_busy_sweep_unblocks_parked_mail() {
        // ★round-3 finding 4 회귀(전 구간)★: MessageDone 이 영영 오지 않는 턴은 busy 를 영구화해 그 수신자
        //   앞 배달을 TTL 까지 막는다. 상한 sweep 이 ① 표를 지우고 ② 도어벨을 눌러야 대기 메일이 나간다.
        use super::super::busy::{BusyTracker, BUSY_MAX_TURN};
        let port = Arc::new(FakeDeliveryPort::new());
        let notifier = Arc::new(RecordingIdle {
            seen: StdMutex::new(Vec::new()),
        });
        let tracker = Arc::new(BusyTracker::new(
            Arc::new(NoSubscribeTapHost),
            notifier.clone(),
        ));
        let svc = Arc::new(MessagingService::new_gated(
            port.clone(),
            Arc::new(ControlRegistry::new()),
            tracker.clone(),
        ));
        let (id, agent) = live("recv");
        port.set_roster(vec![agent]);

        // 비정상 턴: busy 로 관측됐고 종료 통지가 오지 않는다(시각은 주입 — 실시간 30분 대기 회피).
        let t0 = Instant::now();
        tracker.mark_attached_for_test(id, 0);
        tracker.mark_busy_at_for_test(id, 0, t0);
        svc.handle_single_send(
            "m1",
            ident(),
            "s",
            "recv",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        assert_eq!(svc.parked_len("recv"), 1, "busy 라 파킹");
        assert!(
            port.injected_bodies().is_empty(),
            "턴 중이므로 주입 없음(정상)"
        );

        // 상한 경과 sweep → 표 청소 + 도어벨.
        assert_eq!(
            tracker.sweep_stale_busy(t0 + BUSY_MAX_TURN + Duration::from_secs(1)),
            1,
            "상한 초과 busy 잔해 청소"
        );
        assert_eq!(
            notifier.seen.lock().unwrap().clone(),
            vec![id],
            "청소한 id 를 깨워야 대기 메일이 나간다(지우기만 하면 다음 트리거가 없다)"
        );
        // 운영에선 flush lane 이 그 도어벨을 소비한다 — 여기선 직접 그 소비를 수행.
        svc.flush_for_agent(id);
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="s">hi</message>"#.to_string()],
            "상한 sweep 이후 배달됨(TTL blackhole 해소)"
        );
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
    }

    #[test]
    fn flush_for_agent_resolves_name_and_is_noop_without_parked() {
        // id 입구(턴 관측 tap 은 이름을 모른다) → canonical name 해석 후 기존 flush 경로 재사용.
        //   파킹이 없으면 no-op(잉여 idle 통지의 비용을 여기서 깎는다 — 조기 반환).
        let (svc, port, _gate) = svc_gated();
        let (recv_id, recv) = live("recv");
        port.set_roster(vec![recv]);
        svc.flush_for_agent(recv_id);
        assert!(port.injected_bodies().is_empty(), "파킹 없으면 no-op");
        // 파킹이 있으면 id → 이름으로 풀려 배달된다.
        port.set_roster(vec![]);
        svc.handle_single_send(
            "m1",
            ident(),
            "s",
            "recv",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        let (_again, recv2) = live("recv");
        let recv2 = LiveAgent {
            id: recv_id,
            ..recv2
        };
        port.set_roster(vec![recv2]);
        svc.flush_for_agent(recv_id);
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
    }

    #[test]
    fn flush_for_agent_unknown_id_is_noop() {
        // 이미 reap 된 id → canonical_name None → no-op(다음 등장 diff 가 잡는다).
        let (svc, port, _gate) = svc_gated();
        port.set_roster(vec![]);
        svc.handle_single_send(
            "m1",
            ident(),
            "s",
            "ghost",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        svc.flush_for_agent(AgentId::new_v4());
        assert_eq!(
            svc.parked_len("ghost"),
            1,
            "미존재 id flush 는 무해한 no-op"
        );
        assert!(port.injected_bodies().is_empty());
    }
    // ── C3: 회신 계약(request/reply/notice — spec §2·§3 · ADR-0103 결정 2/3) ─────────────────

    /// request 발송 메타(기한 표기 + 파싱값 쌍 — SendMeta 주석의 "둘 다 나르는 이유").
    fn req_meta(raw: &str, secs: u64) -> SendMeta {
        SendMeta {
            request: true,
            reply_by_raw: Some(raw.to_string()),
            reply_by: Some(Duration::from_secs(secs)),
            reply_to: None,
        }
    }

    /// 회신 발송 메타.
    fn reply_meta(in_reply_to: &str) -> SendMeta {
        SendMeta {
            request: false,
            reply_by_raw: None,
            reply_by: None,
            reply_to: Some(in_reply_to.to_string()),
        }
    }

    #[test]
    fn notice_uses_the_senders_own_notation_not_a_normalized_one() {
        // ★리뷰 fix 6 회귀★: 예전엔 Duration 에서 표기를 역산해 `60m` 이 `1h` 로 통지됐다 — 봉투는
        //   `reply-by="60m"` 인데 통지는 `기한(1h)` 이라 같은 계약이 두 표기로 보였다. 이제 발신자 표기를
        //   장부가 원본째 들고 있다가 그대로 쓴다.
        let (svc, port) = svc();
        let (_a, alice) = live("alice");
        port.set_roster(vec![alice]);
        svc.handle_single_send(
            "m-60m",
            ident(),
            "alice",
            "ghost",
            "해줘",
            Entrance::Mcp,
            &req_meta("60m", 3600),
        )
        .expect("parked");
        svc.sweep(Instant::now() + Duration::from_secs(3601));
        assert_eq!(
            port.injected_bodies(),
            vec!["<notice>요청 m-60m 기한(60m) 초과 — ghost 회신 없음</notice>".to_string()],
            "봉투에 쓴 표기 그대로(1h 로 정규화 금지)"
        );
    }

    #[test]
    fn delivered_request_opens_contract_and_renders_request_envelope() {
        // ★골든(즉시 배달)★: request 봉투 = from → id → type → reply-by 순서(spec §1 고정).
        let (svc, port) = svc();
        let (_id, alice) = live("alice");
        port.set_roster(vec![alice]);
        let out = svc
            .handle_single_send(
                "m-req1",
                ident(),
                "qa-alpha",
                "alice",
                "코드 짜고 회신해",
                Entrance::Mcp,
                &req_meta("10m", 600),
            )
            .expect("delivered");
        assert_eq!(out, SendOutcome::Delivered);
        assert_eq!(
            port.injected_bodies(),
            vec![concat!(
                r#"<message from="qa-alpha" id="m-req1" type="request" reply-by="10m">"#,
                "코드 짜고 회신해</message>"
            )
            .to_string()],
            "request 봉투 골든(속성 순서·kebab-case)"
        );
        assert_eq!(
            svc.open_request_count(),
            1,
            "배달된 request 는 계약이 열린다"
        );
        assert_eq!(
            svc.ledger_statuses("m-req1"),
            vec![DeliveryStatus::Delivered]
        );
    }

    #[test]
    fn parked_request_opens_contract_too_and_keeps_envelope_on_flush() {
        // ★spec §3 단계 2★: 계약은 **배달이든 파킹이든** 접수되면 열린다. 그리고 늦게 배달돼도 봉투 속성이
        //   같아야 한다(안 그러면 수신자가 회신할 id 를 모른 채 발신자만 타임아웃 통지를 받는다).
        let (svc, port) = svc();
        port.set_roster(vec![]);
        let out = svc
            .handle_single_send(
                "m-req2",
                ident(),
                "qa-alpha",
                "bob",
                "하고 알려줘",
                Entrance::Mcp,
                &req_meta("1h", 3600),
            )
            .expect("parked");
        assert!(matches!(out, SendOutcome::Parked { .. }));
        assert_eq!(
            svc.open_request_count(),
            1,
            "파킹된 request 도 계약이 열린다"
        );
        assert_eq!(svc.ledger_statuses("m-req2"), vec![DeliveryStatus::Pending]);

        // 등장 → flush. 봉투는 즉시 배달과 **동일**해야 한다.
        let (bob_id, bob) = live("bob");
        port.set_roster(vec![bob]);
        svc.flush_for("bob", bob_id);
        assert_eq!(
            port.injected_bodies(),
            vec![concat!(
                r#"<message from="qa-alpha" id="m-req2" type="request" reply-by="1h">"#,
                "하고 알려줘</message>"
            )
            .to_string()],
            "파킹→flush 배달도 같은 request 봉투(C3 ParkPayload meta 보존)"
        );
        assert_eq!(
            svc.ledger_statuses("m-req2"),
            vec![DeliveryStatus::Delivered]
        );
    }

    #[test]
    fn reply_envelope_carries_in_reply_to_immediate_and_flushed() {
        // ★골든(회신)★: 발신 인자 reply_to → 수신 속성 in-reply-to(spec §1 표기 매핑). id/type 은 없다
        //   (노출 원칙 — 회신은 회신할 대상이 아니다).
        let (svc, port) = svc();
        let (_id, alice) = live("alice");
        port.set_roster(vec![alice]);
        svc.handle_single_send(
            "m-rep1",
            ident(),
            "qa-bravo",
            "alice",
            "다 짰음",
            Entrance::Mcp,
            &reply_meta("m-7f3k"),
        )
        .expect("delivered");
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="qa-bravo" in-reply-to="m-7f3k">다 짰음</message>"#.to_string()],
        );

        // 파킹 경로도 동일 속성.
        let (svc2, port2) = super::tests::svc();
        port2.set_roster(vec![]);
        svc2.handle_single_send(
            "m-rep2",
            ident(),
            "qa-bravo",
            "carol",
            "늦은 회신",
            Entrance::Cli,
            &reply_meta("m-7f3k"),
        )
        .expect("parked");
        let (carol_id, carol) = live("carol");
        port2.set_roster(vec![carol]);
        svc2.flush_for("carol", carol_id);
        assert_eq!(
            port2.injected_bodies(),
            vec![
                r#"<message from="qa-bravo" in-reply-to="m-7f3k">늦은 회신</message>"#.to_string()
            ],
            "파킹→flush 회신도 in-reply-to 를 유지"
        );
    }

    #[test]
    fn plain_send_envelope_is_unchanged_by_c3() {
        // 회귀 방어: 통보는 여전히 속성 없는 plain 봉투(SendMeta::default()).
        let (svc, port) = svc();
        let (_id, alice) = live("alice");
        port.set_roster(vec![alice]);
        svc.handle_single_send(
            "m1",
            ident(),
            "s",
            "alice",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("delivered");
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="s">hi</message>"#.to_string()]
        );
        assert_eq!(svc.open_request_count(), 0, "통보는 계약을 열지 않는다");
    }

    #[test]
    fn correct_reply_id_closes_contract_and_marks_replied() {
        // 엄격 매칭 성공 — 계약 닫힘 + 이력 Delivered→Replied 전이(전이 시각 = 회신 시각).
        let (svc, port) = svc();
        let (_a, alice) = live("alice");
        let (_b, bob) = live("bob");
        port.set_roster(vec![alice, bob]);
        svc.handle_single_send(
            "m-req",
            ident(),
            "alice",
            "bob",
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("delivered");
        assert_eq!(svc.open_request_count(), 1);

        svc.handle_single_send(
            "m-rep",
            ident(),
            "bob",
            "alice",
            "했음",
            Entrance::Mcp,
            &reply_meta("m-req"),
        )
        .expect("delivered");
        assert_eq!(
            svc.open_request_count(),
            0,
            "정확한 id 회신은 계약을 닫는다"
        );
        assert_eq!(
            svc.ledger_statuses("m-req"),
            vec![DeliveryStatus::Replied],
            "원본 레코드가 Replied 로 전이"
        );
    }

    #[test]
    fn wrong_reply_id_still_delivers_but_closes_nothing() {
        // ★엄격 매칭(spec §2)★: 틀린 id 는 아무 것도 닫지 않는다 — 그래도 **회신 메시지 자체는 정상 배달**
        //   되고 응답 shape 도 그대로다(반려로 승격하면 이미 배달된 메시지에 재시도가 붙어 중복이 난다).
        let (svc, port) = svc();
        let (_a, alice) = live("alice");
        let (_b, bob) = live("bob");
        port.set_roster(vec![alice, bob]);
        svc.handle_single_send(
            "m-req",
            ident(),
            "alice",
            "bob",
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("delivered");

        let out = svc
            .handle_single_send(
                "m-rep",
                ident(),
                "bob",
                "alice",
                "엉뚱한 id 로 회신",
                Entrance::Mcp,
                &reply_meta("m-nope"),
            )
            .expect("회신은 정상 배달돼야");
        assert_eq!(out, SendOutcome::Delivered);
        assert_eq!(svc.open_request_count(), 1, "틀린 id 는 계약을 못 닫는다");
        assert_eq!(
            svc.ledger_statuses("m-req"),
            vec![DeliveryStatus::Delivered],
            "원본은 여전히 미회신"
        );
        assert_eq!(
            port.injected_bodies().len(),
            2,
            "request + 회신 둘 다 실제로 주입됐다"
        );
    }

    #[test]
    fn second_reply_to_closed_request_is_noop_and_still_delivers() {
        let (svc, port) = svc();
        let (_a, alice) = live("alice");
        let (_b, bob) = live("bob");
        port.set_roster(vec![alice, bob]);
        svc.handle_single_send(
            "m-req",
            ident(),
            "alice",
            "bob",
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("delivered");
        svc.handle_single_send(
            "m-r1",
            ident(),
            "bob",
            "alice",
            "1차",
            Entrance::Mcp,
            &reply_meta("m-req"),
        )
        .expect("delivered");
        svc.handle_single_send(
            "m-r2",
            ident(),
            "bob",
            "alice",
            "2차",
            Entrance::Mcp,
            &reply_meta("m-req"),
        )
        .expect("2차 회신도 배달돼야");
        assert_eq!(svc.open_request_count(), 0);
        assert_eq!(port.injected_bodies().len(), 3, "3건 모두 주입");
    }

    #[test]
    fn parked_request_reply_closes_contract_even_before_delivery() {
        // 원본이 아직 pending(미주입)인데 회신이 도착 — 계약은 닫히고(정본은 추적) 이력은 anomaly 로 남는다
        //   (ledger.rs ClosedHistoryAnomaly: 계약 닫힘과 이력 부기는 별개 관심사).
        let (svc, port) = svc();
        let (_a, alice) = live("alice");
        port.set_roster(vec![alice]);
        svc.handle_single_send(
            "m-req",
            ident(),
            "alice",
            "ghost",
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("parked");
        assert_eq!(svc.open_request_count(), 1);
        svc.handle_single_send(
            "m-rep",
            ident(),
            "ghost",
            "alice",
            "했음",
            Entrance::Mcp,
            &reply_meta("m-req"),
        )
        .expect("delivered");
        assert_eq!(svc.open_request_count(), 0, "계약은 닫힌다");
        assert_eq!(
            svc.ledger_statuses("m-req"),
            vec![DeliveryStatus::Pending],
            "이력은 미주입 상태 유지(불법 간선 — anomaly 로 관측만)"
        );
    }

    #[test]
    fn rejected_request_does_not_leave_a_ghost_contract() {
        // ★유령 타임아웃 방지★: cap 초과로 **접수조차 안 된** request 의 계약이 남으면, 보낸 적 없는 요청에
        //   대해 기한 초과 notice 가 간다. 반려 갈래가 예약을 회수해야 한다.
        let (svc, port) = svc();
        port.set_roster(vec![]);
        for i in 0..100 {
            svc.handle_single_send(
                &format!("m{i}"),
                ident(),
                "s",
                "ghost",
                "x",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("cap 이내");
        }
        let rejected = svc.handle_single_send(
            "m-req",
            ident(),
            "alice",
            "ghost",
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        );
        assert_eq!(rejected, Err(SendReject::MailboxFull));
        assert_eq!(
            svc.open_request_count(),
            0,
            "반려된 request 의 계약은 회수돼야(유령 타임아웃 금지)"
        );
        // sweep 을 기한 한참 뒤로 돌려도 notice 가 나오면 안 된다.
        svc.sweep(Instant::now() + Duration::from_secs(3600));
        assert!(
            port.injected_bodies().is_empty(),
            "반려분에 대한 notice 는 없어야"
        );
        // ★흔적째 제거(무계 증식 방지)★: 반려는 이력 레코드를 안 남기므로 추적을 "닫기" 만 하면 evict 될
        //   계기가 영영 없다 — 같은 id 로 다시 시도할 수 있어야(= 추적에서 사라졌어야) 한다.
        let retry = svc.handle_single_send(
            "m-req",
            ident(),
            "alice",
            "alice",
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        );
        assert!(
            !matches!(retry, Err(SendReject::IdCollision)),
            "반려된 id 는 추적에 잔존하지 않아야(닫기가 아니라 제거): {retry:?}"
        );
    }

    #[test]
    fn reply_by_timeout_delivers_notice_to_sender_exactly_once() {
        // ★spec §3 단계 4 · §1 notice 템플릿★: 기한 초과 → **발신자에게** notice. 두 번 sweep 해도 1회.
        let (svc, port) = svc();
        let (_a, alice) = live("alice");
        port.set_roster(vec![alice]);
        svc.handle_single_send(
            "m-7f3k",
            ident(),
            "alice",
            "qa-bravo",
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("수신자 부재 → 파킹(계약은 열림)");
        assert!(port.injected_bodies().is_empty(), "부재라 아직 주입 없음");

        svc.sweep(Instant::now() + Duration::from_secs(601));
        assert_eq!(
            port.injected_bodies(),
            vec!["<notice>요청 m-7f3k 기한(10m) 초과 — qa-bravo 회신 없음</notice>".to_string()],
            "notice 골든(from 속성 없음 = 회신 대상 아님)"
        );

        // 이중 통지 방지(장부 notified 플래그).
        svc.sweep(Instant::now() + Duration::from_secs(1200));
        assert_eq!(port.injected_bodies().len(), 1, "notice 는 정확히 1회");
    }

    #[test]
    fn replied_request_never_produces_a_timeout_notice() {
        let (svc, port) = svc();
        let (_a, alice) = live("alice");
        let (_b, bob) = live("bob");
        port.set_roster(vec![alice, bob]);
        svc.handle_single_send(
            "m-req",
            ident(),
            "alice",
            "bob",
            "해줘",
            Entrance::Mcp,
            &req_meta("1m", 60),
        )
        .expect("delivered");
        svc.handle_single_send(
            "m-rep",
            ident(),
            "bob",
            "alice",
            "했음",
            Entrance::Mcp,
            &reply_meta("m-req"),
        )
        .expect("delivered");
        svc.sweep(Instant::now() + Duration::from_secs(600));
        assert_eq!(
            port.injected_bodies().len(),
            2,
            "회신된 계약엔 notice 가 없다(request + reply 2건뿐)"
        );
    }

    #[test]
    fn request_without_reply_by_never_times_out() {
        let (svc, port) = svc();
        let (_a, alice) = live("alice");
        port.set_roster(vec![alice]);
        let meta = SendMeta {
            request: true,
            reply_by_raw: None,
            reply_by: None,
            reply_to: None,
        };
        svc.handle_single_send(
            "m-req",
            ident(),
            "alice",
            "ghost",
            "해줘",
            Entrance::Mcp,
            &meta,
        )
        .expect("parked");
        svc.sweep(Instant::now() + Duration::from_secs(100_000));
        assert!(
            port.injected_bodies().is_empty(),
            "기한 없는 request 는 타임아웃 없음"
        );
        assert_eq!(svc.open_request_count(), 1, "계약은 열린 채 유지");
    }

    #[test]
    fn timeout_notice_parks_cap_exempt_when_sender_is_absent_and_is_ledgered() {
        // ★cap 예외(spec §5 · ADR-0103 불변식)★: 발신자 보관함이 가득 차 있어도 notice 는 반드시 수용된다.
        //   그리고 조용히 사라지지 않는다 — 장부에 pending 으로 남는다.
        let (svc, port) = svc();
        port.set_roster(vec![]); // alice 도 부재.
        for i in 0..100 {
            svc.handle_single_send(
                &format!("m{i}"),
                ident(),
                "s",
                "alice",
                "x",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("cap 이내");
        }
        assert_eq!(
            svc.handle_single_send(
                "over",
                ident(),
                "s",
                "alice",
                "x",
                Entrance::Mcp,
                &SendMeta::default()
            ),
            Err(SendReject::MailboxFull),
            "message 는 cap 에서 반려(대조군)"
        );

        svc.handle_single_send(
            "m-req",
            ident(),
            "alice",
            "bob",
            "해줘",
            Entrance::Mcp,
            &req_meta("1m", 60),
        )
        .expect("parked");
        svc.sweep(Instant::now() + Duration::from_secs(61));

        assert_eq!(
            svc.parked_len("alice"),
            101,
            "notice 는 cap 을 무시하고 얹혀야(100 message + 1 notice)"
        );
        let noticed: Vec<_> = svc
            .ledger_snapshot()
            .into_iter()
            .filter(|(_, from, _, _)| from == NOTICE_SENDER_LABEL)
            .collect();
        assert_eq!(
            noticed.len(),
            1,
            "notice 도 장부에 남는다(조용한 유실 금지)"
        );
        assert_eq!(noticed[0].2, "alice", "notice 수신자 = 요청 발신자");
        assert_eq!(noticed[0].3, DeliveryStatus::Pending, "파킹이므로 pending");
    }

    #[test]
    fn parked_notice_flushes_as_notice_tag_and_marks_delivered() {
        // 파킹된 notice 가 등장 flush 로 배달될 때도 notice 태그여야 한다(kind 기반 wrap 분기).
        let (svc, port) = svc();
        port.set_roster(vec![]);
        svc.handle_single_send(
            "m-9x",
            ident(),
            "alice",
            "bob",
            "해줘",
            Entrance::Mcp,
            &req_meta("1m", 60),
        )
        .expect("parked");
        svc.sweep(Instant::now() + Duration::from_secs(61));
        assert!(
            port.injected_bodies().is_empty(),
            "발신자도 부재 → notice 파킹"
        );

        let (alice_id, alice) = live("alice");
        port.set_roster(vec![alice]);
        svc.flush_for("alice", alice_id);
        assert_eq!(
            port.injected_bodies(),
            vec!["<notice>요청 m-9x 기한(1m) 초과 — bob 회신 없음</notice>".to_string()],
            "flush 배달도 notice 태그(from 없음)"
        );
        let noticed: Vec<_> = svc
            .ledger_snapshot()
            .into_iter()
            .filter(|(_, from, _, _)| from == NOTICE_SENDER_LABEL)
            .collect();
        assert_eq!(
            noticed[0].3,
            DeliveryStatus::Delivered,
            "실제 주입 시점 delivered"
        );
    }

    #[test]
    fn timeout_notice_parks_when_sender_is_mid_turn() {
        // idle 게이트는 notice 에도 적용된다(턴 중 주입 금지 — 배치 제어권 유지). 단 cap 만 예외.
        let (svc, port, gate) = svc_gated();
        let (alice_id, alice) = live("alice");
        port.set_roster(vec![alice]);
        gate.set_busy(alice_id, 0);
        svc.handle_single_send(
            "m-b1",
            ident(),
            "alice",
            "ghost",
            "해줘",
            Entrance::Mcp,
            &req_meta("1m", 60),
        )
        .expect("parked");
        svc.sweep(Instant::now() + Duration::from_secs(61));
        assert!(
            port.injected_bodies().is_empty(),
            "발신자가 턴 중이면 notice 도 주입하지 않는다"
        );
        assert_eq!(svc.parked_len("alice"), 1, "notice 가 alice 앞에 파킹");

        gate.clear();
        svc.flush_for("alice", alice_id);
        assert_eq!(
            port.injected_bodies(),
            vec!["<notice>요청 m-b1 기한(1m) 초과 — ghost 회신 없음</notice>".to_string()],
            "턴 종료 flush 로 배달"
        );
    }

    // ── C3 리뷰 라운드 fix ────────────────────────────────────────────────────────────

    #[test]
    fn timeout_notice_reaches_a_renamed_sender_via_id_hint() {
        // ★fix 2 회귀★: 계약을 연 뒤 발신자가 개명하면 notice 는 **옛 이름** 큐에 파킹된다. 이름-키만
        //   보는 flush 는 그 큐를 영영 열지 않고, 통지는 notified 라 재발화도 없다 = 영구 stranded.
        //   장부가 함께 든 발신자 id 를 힌트로 실으면, 개명 후에도 그 incarnation 으로 배달된다.
        let (svc, port) = svc();
        let alice_id = AgentId::new_v4();
        let old = LiveAgent {
            id: alice_id,
            name: "alice".to_string(),
            epoch: 0,
        };
        port.set_roster(vec![old]);
        svc.handle_single_send(
            "m-req",
            BoundIdentity {
                agent_id: alice_id,
                epoch: 0,
            },
            "alice",
            "ghost",
            "해줘",
            Entrance::Mcp,
            &req_meta("1m", 60),
        )
        .expect("parked(수신자 부재)");

        // 발신자 개명 — 같은 id, 다른 이름. 이제 "alice" 라는 이름은 로스터에 없다.
        let renamed = LiveAgent {
            id: alice_id,
            name: "alice-v2".to_string(),
            epoch: 0,
        };
        port.set_roster(vec![renamed]);

        svc.sweep(Instant::now() + Duration::from_secs(61));
        // 도어벨 미배선 = 인라인 flush 폴백이므로 sweep 안에서 배달까지 끝난다.
        assert_eq!(
            port.injected_bodies(),
            vec!["<notice>요청 m-req 기한(1m) 초과 — ghost 회신 없음</notice>".to_string()],
            "개명한 발신자에게도 id 힌트로 배달돼야(옛 이름 큐에 갇히지 않는다)"
        );
        assert_eq!(svc.parked_len("alice"), 0, "큐가 비었다 = 실제로 나갔다");
    }

    #[test]
    fn sweep_snapshots_the_roster_once_per_tick() {
        // ★fix 8★: due 항목마다 로스터를 다시 뜨면 한 틱 안에서 항목별로 다른 스냅샷을 본다(판정 흔들림).
        //   틱당 1회만 뜨는지 호출 수로 단언한다.
        let (svc, port) = svc();
        port.set_roster(vec![]);
        for i in 0..3 {
            svc.handle_single_send(
                &format!("m-r{i}"),
                ident(),
                &format!("sender{i}"),
                "ghost",
                "해줘",
                Entrance::Mcp,
                &req_meta("1m", 60),
            )
            .expect("parked");
        }
        let before = port.roster_call_count();
        svc.sweep(Instant::now() + Duration::from_secs(61));
        let used = port.roster_call_count() - before;
        // 전제: 발신자들이 로스터에 없어 인라인 flush 폴백이 canonical_name 단계에서 조기 반환한다 —
        //   그래서 이 틱의 로스터 조회는 sweep 자신의 스냅샷 **하나뿐**이어야 한다. 항목별 재조회로
        //   되돌리면 due 수(3)만큼 늘어 여기서 잡힌다.
        assert_eq!(
            used, 1,
            "sweep 은 due 수와 무관하게 틱당 로스터 1회만 떠야(항목별 재조회 금지)"
        );
        assert_eq!(svc.parked_len("sender0"), 1, "notice 는 정상 파킹");
    }

    #[test]
    fn timeout_notice_is_skipped_when_reply_lands_before_parking() {
        // ★fix 5★: due 산출(락 해제)과 notice 파킹 사이에 회신이 도착하면, 발신자는 회신을 받고도
        //   "회신 없음" 통지를 뒤이어 받는다(모순). 파킹 직전 재확인으로 그 통지를 접는다.
        //   레이스는 roster hook 으로 결정적으로 만든다(sweep 은 due 산출 뒤 로스터를 뜬다).
        let (svc, port) = svc();
        port.set_roster(vec![]);
        svc.handle_single_send(
            "m-req",
            ident(),
            "alice",
            "bob",
            "해줘",
            Entrance::Mcp,
            &req_meta("1m", 60),
        )
        .expect("parked");

        let svc2 = svc.clone();
        port.arm_on_next_roster(Box::new(move || {
            // 이 시점 = due 는 이미 걷혔고(notified 세워짐) notice 는 아직 파킹 전.
            svc2.handle_single_send(
                "m-rep",
                ident(),
                "bob",
                "alice",
                "했음",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("parked(alice 부재)");
        }));

        svc.sweep(Instant::now() + Duration::from_secs(61));
        let notices: Vec<_> = svc
            .ledger_snapshot()
            .into_iter()
            .filter(|(_, from, _, _)| from == NOTICE_SENDER_LABEL)
            .collect();
        assert!(
            notices.is_empty(),
            "회신이 먼저 닿았으면 통지는 취소돼야: {notices:?}"
        );
        assert_eq!(svc.open_request_count(), 0, "계약은 회신으로 닫혔다");
    }

    #[test]
    fn plain_send_with_colliding_id_is_rejected_without_side_effects() {
        // ★fix 12★: id 충돌은 request 만의 문제가 아니다 — 통보/회신도 같은 id 면 남의 이력 레코드를
        //   앨리어싱한다. 종류 무관 반려 + 부작용 0(호출자가 새 id 로 재시도).
        let (svc, port) = svc();
        let (_id, alice) = live("alice");
        port.set_roster(vec![alice]);
        svc.handle_single_send(
            "dup",
            ident(),
            "s",
            "alice",
            "1",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("delivered");
        let before = svc.ledger_snapshot().len();
        let injected_before = port.injected_bodies().len();

        // 통보 재사용.
        assert_eq!(
            svc.handle_single_send(
                "dup",
                ident(),
                "s",
                "alice",
                "2",
                Entrance::Mcp,
                &SendMeta::default()
            ),
            Err(SendReject::IdCollision),
            "통보도 id 충돌이면 반려"
        );
        // 회신 재사용.
        assert_eq!(
            svc.handle_single_send(
                "dup",
                ident(),
                "s",
                "alice",
                "3",
                Entrance::Mcp,
                &reply_meta("m-other"),
            ),
            Err(SendReject::IdCollision),
            "회신도 id 충돌이면 반려"
        );
        assert_eq!(svc.ledger_snapshot().len(), before, "장부 레코드 증가 없음");
        assert_eq!(
            port.injected_bodies().len(),
            injected_before,
            "주입 없음(부작용 0)"
        );
    }

    #[test]
    fn request_beyond_open_contract_cap_is_rejected_without_side_effects() {
        // ★fix 3 의 짝★: 오픈 계약이 상한이면 새 request 는 RequestCapacity 로 반려된다(부작용 0).
        //   통보는 계약을 열지 않으므로 계속 통과해야 한다(반려가 메시징 전체를 막지 않는다).
        let (svc, port) = svc();
        port.set_roster(vec![]);
        // 수신자를 흩어 mailbox cap(수신자당 100)에 안 걸리게 한다 — 여기서 재는 건 계약 상한이다.
        let cap = 512;
        for i in 0..cap {
            svc.handle_single_send(
                &format!("m-r{i}"),
                ident(),
                "alice",
                &format!("ghost{}", i % 64),
                "해줘",
                Entrance::Mcp,
                &req_meta("10m", 600),
            )
            .expect("cap 이내");
        }
        let before = svc.ledger_snapshot().len();
        assert_eq!(
            svc.handle_single_send(
                "m-over",
                ident(),
                "alice",
                "ghost0",
                "해줘",
                Entrance::Mcp,
                &req_meta("10m", 600),
            ),
            Err(SendReject::RequestCapacity),
            "상한 초과 request 는 RequestCapacity"
        );
        assert_eq!(
            svc.ledger_snapshot().len(),
            before,
            "반려는 부작용 0(장부 레코드 없음)"
        );
        assert!(
            svc.handle_single_send(
                "m-plain",
                ident(),
                "alice",
                "ghost0",
                "알림",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .is_ok(),
            "통보는 계약을 열지 않으므로 상한과 무관하게 통과"
        );
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "ingress가 유일 검증자")]
    fn mutually_exclusive_contract_fields_trip_the_debug_assert() {
        // ★fix 11★: 이 함수는 pub 이라 다른 조립이 직접 부를 수 있다 — 상호배타 위반 배선을 debug 에서
        //   즉시 터뜨린다(운영 반려 문구는 ingress 가 정본이므로 검증을 복제하지 않는다).
        let (svc, port) = svc();
        port.set_roster(vec![]);
        let bad = SendMeta {
            request: true,
            reply_by_raw: None,
            reply_by: None,
            reply_to: Some("m-1".to_string()),
        };
        let _ = svc.handle_single_send("m-x", ident(), "s", "bob", "x", Entrance::Mcp, &bad);
    }

    // ── ParkPayload 견고성(fix 4) ─────────────────────────────────────────────────────

    #[test]
    fn park_payload_decode_falls_back_on_corrupt_input_without_panicking() {
        // ★fix 4★: 어떤 입력이 와도 패닉하지 않는다 — 특히 길이 필드가 **멀티바이트 문자 중간**을 가리키는
        //   경우(`split_at` 이 패닉하는 그 지점). 실패는 전부 "속성 잃고 body 만 남는" 폴백이다.
        let good = ParkPayload {
            sender_name: "qa".to_string(),
            from: BoundIdentity {
                agent_id: AgentId::new_v4(),
                epoch: 3,
            },
            entrance: Entrance::Mcp,
            body: "안녕\n둘째 줄".to_string(),
            meta: req_meta("10m", 600),
        };
        let encoded = good.encode();
        // 대조군: 정상 payload 는 그대로 복원된다.
        let ok = ParkPayload::decode(&encoded);
        assert_eq!(ok.body, "안녕\n둘째 줄");
        assert_eq!(ok.sender_name, "qa");
        assert!(ok.meta.request);

        for corrupt in [
            // 헤더 자체가 없음.
            "그냥 본문".to_string(),
            // 버전 태그가 모르는 값(형식 드리프트) — 오해석 대신 폴백.
            encoded.replacen('1', "9", 1),
            // 길이 필드가 숫자가 아님.
            "1\nx\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nmcp\n-\nbody".to_string(),
            // 길이 합이 남은 문자열보다 큼(절단).
            "1\n99\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nmcp\n-\nshort".to_string(),
            // ★char 경계 중간 절단★: '한'은 3바이트인데 sender_len=1 이라 문자 중간을 가른다.
            "1\n1\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nmcp\n-\n한글".to_string(),
            // reply_to 길이가 경계를 어긋나게 만드는 경우(두 번째 절단점).
            "1\n0\n1\n0\n00000000-0000-0000-0000-000000000000\n0\nmcp\n-\n한글".to_string(),
            // 헤더 줄 수 부족.
            "1\n0\n0\n0\n".to_string(),
            // ★어휘 밖 입구(fix 3)★: 예전엔 조용히 Cli 로 떨어졌다 — 이제 손상으로 보고 폴백한다.
            "1\n0\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nsmtp\n-\nbody".to_string(),
            // 입구 칸이 비어 있음(형식 드리프트).
            "1\n0\n0\n0\n00000000-0000-0000-0000-000000000000\n0\n\n-\nbody".to_string(),
            // 대소문자도 어휘 밖(정규화하지 않는다 — 우리가 쓴 값만 인정).
            "1\n0\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nMCP\n-\nbody".to_string(),
            // ★어휘 밖 플래그(fix 3)★: 예전엔 조용히 "request 아님" 으로 떨어져 계약 속성을 잃었다.
            "1\n0\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nmcp\nR\nbody".to_string(),
            "1\n0\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nmcp\n\nbody".to_string(),
            "1\n0\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nmcp\nr-\nbody".to_string(),
        ] {
            let p = ParkPayload::decode(&corrupt);
            assert_eq!(
                p.body, corrupt,
                "깨진 payload 는 전체를 body 로 폴백(조용한 유실 금지): {corrupt:?}"
            );
            assert!(!p.meta.request, "폴백은 계약 속성을 주장하지 않는다");
        }
    }

    #[test]
    fn one_corrupt_parked_item_does_not_abort_the_flush_batch() {
        // ★fix 4 의 목적★: decode 는 flush 배치 안에서 항목마다 불린다 — 한 항목이 깨졌다고 배치가
        //   중단되면 멀쩡한 메시지까지 발이 묶인다(release 는 panic=abort 라 데몬 자체가 죽는다).
        let (svc, port) = svc();
        port.set_roster(vec![]);
        for (id, body) in [("m-a", "첫째"), ("m-b", "둘째"), ("m-c", "셋째")] {
            svc.handle_single_send(
                id,
                ident(),
                "qa",
                "bob",
                body,
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("parked");
        }
        // 가운데 항목의 payload 를 손상시킨다(테스트 전용 주입 — 실제로는 일어나지 않지만 방어의 대상).
        svc.corrupt_parked_payload_for_test("bob", 1);

        let (bob_id, bob) = live("bob");
        port.set_roster(vec![bob]);
        svc.flush_for("bob", bob_id);

        let bodies = port.injected_bodies();
        assert_eq!(bodies.len(), 3, "깨진 항목 하나가 배치를 끊지 않는다");
        assert!(bodies[0].contains("첫째") && bodies[2].contains("셋째"));
        assert!(
            bodies[1].contains("CORRUPT"),
            "깨진 항목은 폴백 봉투로라도 나간다(조용한 유실 금지): {}",
            bodies[1]
        );
    }

    // ── 데몬 자가 발신 id 충돌 검사(round-final fix 2) ────────────────────────────────

    #[test]
    fn daemon_notice_id_goes_through_the_same_collision_check_as_sends() {
        // ★fix 2★: 데몬이 만드는 notice id 도 장부 충돌 검사(`msg_id_in_use`)를 탄다 — 예전엔 무검사라
        //   기존 id 를 앨리어싱할 수 있었다(이력 레코드 키 공유 = 남의 레코드를 전이·뭉갬).
        let (svc, port) = svc();
        port.set_roster(vec![]);
        svc.handle_single_send(
            "m-taken",
            ident(),
            "alice",
            "bob",
            "x",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("parked");

        // ① 첫 draw 가 이미 쓰인 id → 새 id 로 딱 1회 갈아탄다(ingress 재시도와 같은 규율).
        //    (`pop()` 이 뒤에서 꺼내므로 목록은 draw 순서의 역순이다.)
        let mut two = vec!["m-fresh".to_string(), "m-taken".to_string()];
        let drawn = svc.draw_daemon_msg_id_with(|| two.pop().expect("draw"));
        assert_eq!(drawn.id, "m-fresh", "충돌한 id 는 버리고 새 id 를 쓴다");
        assert_eq!(
            drawn.collided.as_deref(),
            Some("m-taken"),
            "충돌한 쪽이 조사 단서 — 로그가 그 값을 찍는다"
        );
        assert!(!drawn.still_colliding);

        // ② 두 번째도 충돌 → notice 는 **그래도 나간다**(버리면 계약이 조용히 반쪽) + 신호를 세워 관측.
        let mut both = vec!["m-taken".to_string(), "m-taken".to_string()];
        let drawn = svc.draw_daemon_msg_id_with(|| both.pop().expect("draw"));
        assert_eq!(drawn.id, "m-taken", "통지 유실보다 관측상 오염이 낫다");
        assert!(
            drawn.still_colliding,
            "재-draw 마저 충돌하면 신호(로그 대상)"
        );

        // ③ 대조군: 충돌이 없으면 첫 draw 를 그대로 쓴다(불필요한 재-draw 없음).
        let mut once = vec!["m-new".to_string()];
        let drawn = svc.draw_daemon_msg_id_with(|| once.pop().expect("draw 는 1회뿐이어야"));
        assert_eq!(drawn.id, "m-new");
        assert!(drawn.collided.is_none());
    }

    #[test]
    fn id_collision_rejects_without_side_effects() {
        // ★부작용 0 보장(SendReject::IdCollision 주석)★: 같은 id 로 두 번째 request 를 보내면 배달·파킹·
        //   장부 레코드가 **하나도** 늘지 않아야 호출자가 새 id 로 안전하게 재시도할 수 있다.
        let (svc, port) = svc();
        port.set_roster(vec![]);
        svc.handle_single_send(
            "dup",
            ident(),
            "alice",
            "bob",
            "1",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("parked");
        let before = svc.ledger_snapshot().len();
        let again = svc.handle_single_send(
            "dup",
            ident(),
            "alice",
            "bob",
            "2",
            Entrance::Mcp,
            &req_meta("10m", 600),
        );
        assert_eq!(again, Err(SendReject::IdCollision));
        assert_eq!(svc.ledger_snapshot().len(), before, "장부 레코드 증가 없음");
        assert_eq!(svc.parked_len("bob"), 1, "파킹 증가 없음");
        assert!(port.injected_bodies().is_empty());
    }
}
