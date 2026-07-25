//! service — MessagingService: 순수 구조(mailbox·ledger·groups)를 tokio 위에서 발송 파이프라인에
//! 엮는 오케스트레이터(S18 메시징 v1 increment C1 · ADR-0103/0104).
//!
//! ★역할(C1 스코프)★: 단일 수신자 발송의 3분기(spec §5)와 등장/idle flush(ADR-0104), TTL sweep 을
//!   담당한다. request/reply(C3)·그룹(C4)·idle 게이트(C2)는 **범위 밖** — 여기 넣지 않는다.
//!     ① resolve+inject 성공 → `delivered`(실제 주입 시점에만, ADR-0104)
//!     ② 부재(미스폰·죽음, "없는 이름" 포함) → **파킹** = `pending`(RECIPIENT_NOT_FOUND 소멸, spec §5)
//!     ③ 도달 불가/write 실패 → **파킹** = `pending`
//!     보관함 초과 → `MAILBOX_FULL` 반려(spec §5 분기 3).
//!   등장(스폰/epoch 교체)·flush 시 파킹분을 **오래된 순 일괄** 주입(각 메시지 개별 봉투, ADR-0104).
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
//! ★봉투 = 주입 시점 조립(단일 wrap point, ADR-0096)★: 파킹은 **감싸지 않은 body + 발신자 이름**을
//!   저장하고, 봉투는 **주입할 때** `wrap_message`(ingress.rs 단일 wrap point)로 만든다. 왜: 파킹과 flush
//!   사이 봉투 포맷(colon/xml 전역 스위치)이 바뀔 수 있고, 그때 flush 는 **현재** 포맷으로 감싸야 한다.
//!   park 시점에 미리 감싸면 옛 포맷이 굳어 버린다. 그래서 raw body 를 나르고 조립은 주입 순간 한 곳에서.
//!
//! tauri import 0(daemon crate).
// ADR-0103
// ADR-0104

use std::sync::{Arc, Mutex};
use std::time::Instant;

use engram_dashboard_core::agent::manager::AgentManager;
use engram_dashboard_core::agent::types::{AgentId, AgentStatus, WriteOutcome};

use super::ledger::{DeliveryStatus, Ledger};
use super::mailbox::{Mailbox, ParkError, ParkKind, ParkedMessage};
use crate::control::ingress::{
    wrap_message, DeliveryObservation, Entrance, EnvelopeFields, EnvelopeFormat,
};
use crate::control::registry::{BoundIdentity, ControlRegistry};

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
}

impl MessagingService {
    /// port + registry 주입 생성자(테스트가 FakeDeliveryPort 를 끼운다).
    pub fn new(port: Arc<dyn DeliveryPort>, registry: Arc<ControlRegistry>) -> Self {
        Self {
            state: Mutex::new(MessagingState {
                mailbox: Mailbox::new(),
                ledger: Ledger::new(),
                groups: super::groups::Groups::new(),
            }),
            port,
            registry,
        }
    }

    /// 운영 편의 생성자 — Arc<AgentManager> 를 ManagerDeliveryPort 로 감싼다(lib.rs 부팅용).
    pub fn for_manager(manager: Arc<AgentManager>, registry: Arc<ControlRegistry>) -> Self {
        Self::new(Arc::new(ManagerDeliveryPort::new(manager)), registry)
    }

    /// ★단일 수신자 발송(spec §5 3분기 — C1)★. handle_send 의 3-branch rewiring 이 검증·auth 통과 후 부른다.
    ///   - `msg_id`: 상위가 부여한 논리 메시지 id(ledger 상관·ACK 동봉 축).
    ///   - `from`: 발신자 신원(토큰 파생, ADR-0086). 관측 레코드·canonical 이름 조회에 쓴다.
    ///   - `sender_name`: 봉투 sender 표시 이름(상위가 canonical 로 이미 해석 — WYSIWYA ADR-0101).
    ///   - `to`: 수신자 지목(이름 또는 AgentId 문자열).
    ///   - `body`: **감싸지 않은** 본문(봉투 조립은 주입 시점 wrap_message — 단일 wrap point).
    ///
    /// 분기(spec §5):
    ///   ① 산 수신자 해석 성공 + inject 성공 → ledger `delivered`(실제 주입, ADR-0104) → `Delivered`.
    ///   ② 부재(로스터에 없음 — "없는 이름" 포함) → park + ledger `pending` → `Parked`. 오타는 TTL 방어.
    ///   ③ 해석 성공했으나 inject 실패(그 틈에 죽음·transport 오류) → **재파킹** + ledger `pending`
    ///      (spec: unreachable → 파킹). 조용한 유실 금지 — 반드시 ledger 에 남긴다.
    ///   cap 초과 → `Err(MailboxFull)`(반려).
    ///
    /// ★락 규율(모듈 헤더)★: 로스터 조회·resolve 는 port 호출(락 밖) → 그 결과로 락을 잡아 ledger record +
    ///   (필요 시)park 결정 → **락 해제** → inject(락 밖) → 결과에 따라 다시 짧게 락 잡아 transition/park.
    pub fn handle_single_send(
        &self,
        msg_id: &str,
        from: BoundIdentity,
        sender_name: &str,
        to: &str,
        body: &str,
        entrance: Entrance,
    ) -> Result<SendOutcome, SendReject> {
        // 1) 로스터 조회(락 밖 — port 호출) → 이름/AgentId 해석.
        let roster = self.port.live_reachable_agents();
        let resolved = resolve_live(to, &roster);

        // 2) 부재 → 파킹(spec §5 분기 2). "없는 이름"도 파킹(스폰 전 선지시 지원, TTL 방어).
        let Some(target) = resolved else {
            // ★park 키 정규화(finding 4 · load-bearing)★: 등장 flush 는 canonical **NAME** 으로 keyed 이다
            //   (flush observer 가 로스터 diff 로 이름을 넘긴다). 그런데 `to` 가 exact AgentId 이고 그 에이전트가
            //   살아는 있으나 **비-도달**(TUI 등 non-structured)이면 resolve_live 는 None 을 내고, 여기서 UUID
            //   문자열 그대로 park 하면 park 키(UUID) ≠ flush 키(canonical name) 라 그 파킹은 **영영 flush 안 됨**.
            //   그래서 park 전에 `to` 를 AgentId 로 파싱해 그 에이전트가 존재하면 canonical_name 으로 park 키를
            //   바꾼다(그 이름이 도달 가능해지면 flush 가 잡게). 파싱 실패/미존재면 리터럴 문자열 그대로(이름 지목).
            let park_key = self.canonical_park_key(to);
            let hint = format!(
                "No live reachable agent named '{to}' — parked; it will be delivered when that name appears (expires after TTL)."
            );
            let outcome =
                self.park_pending(msg_id, sender_name, from, entrance, &park_key, body, hint)?;
            // ★park/appearance TOCTOU self-heal(finding 3)★: resolve↔park 사이 그 이름이 등장했으면 flush
            //   observer 는 빈 큐를 이미 flush 하고 지나가, 방금 park 한 메일이 다음 등장/TTL 까지 발이 묶인다.
            //   그래서 park 직후(락 해제 상태) 그 park_key 가 지금 유일 도달이면 즉시 flush 를 돌려 자가치유한다.
            //   drain 이 큐를 비우므로 flush observer 와 겹쳐도 idempotent(둘 다 drain — 한쪽이 빈 큐를 본다).
            self.self_heal_if_live(&park_key);
            return Ok(outcome);
        };

        // 3) 해석 성공 → 주입 시도. 봉투는 **주입 시점**에 현재 포맷으로 감싼다(단일 wrap point).
        //    ledger 는 주입 **후** 성공/실패에 따라 delivered/pending 을 찍는다(delivered=실제 주입, ADR-0104).
        let now = Instant::now();
        let wrapped = self.wrap_now(sender_name, msg_id, body);
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
                )
            }
        }
    }

    /// 파킹 + ledger `pending` 기록(spec §5 분기 2·3 공통). cap 초과면 `MailboxFull` 반려(spec §5 분기 3).
    ///
    /// ★조용한 유실 금지(ADR-0103)★: park 성공 시 반드시 ledger 에 `pending` 레코드를 남긴다 — 파킹된
    ///   메시지가 장부 밖에 있으면 조회·감사에서 사라진다. cap 초과 반려는 애초에 저장 안 하므로 ledger 도
    ///   안 남긴다(발신자에게 반려로 즉시 가시화 — spec §5 "오래된 것 조용히 버리기 금지" 와 정합).
    /// ★락 규율★: 이 함수는 park+record 를 한 락 구간에서 하되(둘 다 순수 구조 조작, 외부 호출 없음) 그
    ///   구간에서 port 를 부르지 않는다.
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
    ) -> Result<SendOutcome, SendReject> {
        let now = Instant::now();
        let mut st = self.state.lock().expect("messaging state poisoned");
        // park 는 raw body + 봉투/관측에 필요한 최소 메타(sender 이름·발신자 신원·입구)를 나른다(봉투는
        //   flush 주입 시점 조립 — 단일 wrap point). ParkedMessage.envelope 계약이 "완성 봉투"라 여기선
        //   ParkPayload 로 인코딩해 그 문자열 슬롯에 실어 보관하고, flush 때 decode 한다.
        let payload = ParkPayload {
            sender_name: sender_name.to_string(),
            from,
            entrance,
            body: body.to_string(),
        };
        let parked = ParkedMessage {
            msg_id: msg_id.to_string(),
            envelope: payload.encode(),
            kind: ParkKind::Message,
            parked_at: now,
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
    /// 동작:
    ///   1. 락 잡고 `mailbox.drain(recipient)` → deliverable(미만료, 오래된 순) + expired(만료).
    ///   2. expired → ledger `pending→expired`(장부 잔존, 락 안 — 순수 조작). **락 해제.**
    ///   3. deliverable → 각각 **개별 봉투**로 감싸 순서대로 inject(락 밖). 성공 → ledger `pending→delivered`.
    ///   4. ★부분 실패 무손실(load-bearing)★: 배치 도중 inject 실패(drain 후 수신자 사망)면 **남은
    ///      deliverable 을 재파킹**한다(drain 으로 큐가 비었으므로 재-park 가 순서를 보존). 실패분 포함 이후
    ///      전부를 재파킹해 다음 등장에 재시도한다(조용한 유실 금지).
    ///
    /// ★왜 to_id 를 인자로 받나(그리고 왜 execution 시점에 재검증하나 — finding 2)★: flush observer/self-heal
    ///   이 로스터 스냅샷에서 (이름→현재 id) 를 알고 부르지만, 그 스냅샷은 **enqueue 시점** 것이라
    ///   execution(여기)까지 사이에 stale 해질 수 있다 — ① 동명 두 번째 에이전트가 등장해 이름이 ambiguous
    ///   해졌거나 ② 그 수신자가 죽었을 수 있다. 그래서 drain 직전 **현재 로스터**로 이름을 재해석해:
    ///   그 이름이 **정확히 1개** 도달 후보로 풀릴 때만 진행하고, 그 후보의 id 로 to_id 를 갱신한다(등장
    ///   사이 epoch/incarnation 이 바뀌었어도 현재 산 것으로 주입). ambiguous·부재면 skip(파킹 유지 —
    ///   그 이름이 다시 유일해지거나 TTL 로 만료될 때까지 큐에 남는다, tracing::debug). ★uniqueness 로직은
    ///   self_heal_if_live 와 공유★(resolve_unique_reachable) — 이름-키 파킹의 동명 정책을 한 곳에서 판정.
    pub fn flush_for(&self, recipient: &str, to_id: AgentId) {
        // ★execution-time 재해석(finding 2)★: enqueue 시점 (name,id) 는 stale 가능 — 지금 로스터로 재확인.
        //   port 호출이라 messaging 락 밖(모듈 헤더 규율). 유일 도달일 때만 그 현재 id 로 진행한다.
        let to_id = match self.resolve_unique_reachable(recipient) {
            Some(a) => a.id,
            None => {
                tracing::debug!(
                    recipient,
                    stale_id = %to_id,
                    "flush skip: execution 시점 이름이 유일 도달 아님(부재/동명 다수) — 파킹 유지(finding 2)"
                );
                return;
            }
        };
        let now = Instant::now();
        // 1~2) 드레인 + 만료 장부화(락 구간 — 순수 조작만).
        let deliverable = {
            let mut st = self.state.lock().expect("messaging state poisoned");
            let drained = st.mailbox.drain(recipient, now);
            for ex in &drained.expired {
                // pending → expired(장부 잔존). NotFound/Illegal 은 best-effort(레코드 evict 등) — 무시.
                let _ = st
                    .ledger
                    .transition(&ex.msg_id, recipient, DeliveryStatus::Expired, now);
            }
            drained.deliverable
        };
        if deliverable.is_empty() {
            return;
        }
        // 3~4) 오래된 순 개별 주입(락 밖). 실패 시 남은 것(실패분 포함) 재파킹.
        for (idx, parked) in deliverable.iter().enumerate() {
            let payload = ParkPayload::decode(&parked.envelope);
            let wrapped = self.wrap_now(&payload.sender_name, &parked.msg_id, &payload.body);
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
                    let target = LiveAgent {
                        id: to_id,
                        name: recipient.to_string(),
                        epoch: outcome.epoch,
                    };
                    self.observe_success(
                        &parked.msg_id,
                        &target,
                        payload.from,
                        payload.entrance,
                        &wrapped,
                        &outcome,
                    );
                }
                Err(_e) => {
                    // ★부분 실패 무손실(load-bearing — finding 1, ADR-0103/0104)★: 수신자가 drain↔inject
                    //   사이 죽었다 — 남은 것(idx..)을 **restore_front** 로 되돌린다. 왜 park 가 아닌가:
                    //     ① cap 우회 — drain↔inject 사이 **동시 park** 가 큐를 다시 cap 까지 채웠으면 park 는
                    //        MailboxFull 로 반려한다. 그 에러를 무시하면 admitted 메시지가 조용히 유실된다
                    //        (ledger 는 pending 인데 큐엔 없음 — 유령 pending). restore_front 는 cap 을 세지
                    //        않아 무조건 되돌린다(cap 은 유입 통제지 보관 통제가 아님 — mailbox 주석).
                    //     ② FRONT 삽입 — 재파킹분(더 오래됨)이 동시 park 된 신규분(더 최근)보다 앞서야
                    //        "오래된 순" 이 안 깨진다(FIFO 역전 방지). restore_front 가 원래 순서로 큐 앞에 꽂는다.
                    //   parked_at 은 clone 으로 원래 값 유지 — TTL 연장 없음(오배송 방어). ledger 는 이미
                    //   pending 이라 전이 불요(재파킹 = pending 유지). 다음 등장에 재시도.
                    let remaining: Vec<ParkedMessage> = deliverable[idx..].to_vec();
                    let remaining_count = remaining.len();
                    let mut st = self.state.lock().expect("messaging state poisoned");
                    st.mailbox.restore_front(recipient, remaining);
                    tracing::warn!(
                        recipient,
                        remaining = remaining_count,
                        "메시지 flush 중 inject 실패 — 남은 배치 재파킹(무손실 restore_front, ADR-0103/0104)"
                    );
                    break;
                }
            }
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
    ///   flush/ambiguity 정책(동명 다수면 배달 보류 — finding 2/3, resolve_unique_reachable)과 일관된다:
    ///   이름이 유일하게 풀릴 때만 배달하므로, "그 이름을 쓰는 지금 유일한 에이전트" 로 배달된다는 계약이 유지된다.
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
        // 유일 도달일 때만 self-heal(uniqueness 판정은 flush_for 와 공유 — resolve_unique_reachable).
        //   flush_for 도 진입 때 같은 재해석을 하므로 여기 매치는 형식상 중복이나, 유일치 않으면 flush_for
        //   호출 자체를 아끼려고 먼저 본다(불필요한 port 조회 1회 절약은 아니고, 의도 표현 — 동명 skip).
        if let Some(a) = self.resolve_unique_reachable(park_key) {
            // 정확히 1개 — 유일 도달. 그 id 로 flush(파킹 큐를 오래된 순 일괄 주입).
            self.flush_for(park_key, a.id);
        }
    }

    /// ★유일 도달 재해석(finding 2/3 공유)★: 이름을 **현재** 로스터에 대고 풀어, 그 이름의 도달 후보가
    ///   **정확히 1개**면 그 LiveAgent 를 돌려주고, 0개(부재)·2개+(동명 다수)면 None. flush_for(execution
    ///   시점 stale-authority 재검증)와 self_heal_if_live(park 직후 등장 확인)가 같은 동명 정책을 쓰도록
    ///   한 곳에 모은다 — send-side RECIPIENT_AMBIGUOUS 와 일관(동명 다수는 배달하지 않고 파킹 유지).
    /// ★락 밖 호출★: live_reachable_agents 는 port 호출이라 messaging 락 밖에서만 부른다(모듈 헤더 규율).
    fn resolve_unique_reachable(&self, name: &str) -> Option<LiveAgent> {
        let roster = self.port.live_reachable_agents();
        let mut matches = roster.into_iter().filter(|a| a.name == name);
        let first = matches.next()?;
        // 두 번째가 있으면 동명 다수 — None(파킹 유지). 없으면 유일.
        match matches.next() {
            Some(_) => None,
            None => Some(first),
        }
    }

    /// ★TTL sweep(spec §5 — C1)★: 전 수신자에 걸쳐 만료 파킹분을 걷어 ledger `pending→expired` 로 남긴다.
    ///   sweep task 가 주기적으로 부른다(lib.rs). notice 는 cap 예외지만 TTL 은 적용된다(spec) — mailbox
    ///   가 kind 무관하게 만료를 판정하므로 여기선 구분 불요.
    /// ★락 규율★: sweep+transition 은 순수 조작이라 한 락 구간에서 한다(외부 호출 없음).
    pub fn sweep(&self, now: Instant) {
        let mut st = self.state.lock().expect("messaging state poisoned");
        let expired = st.mailbox.sweep_expired(now);
        for ex in expired {
            // 파킹은 어느 수신자 큐에 있었는지 sweep 반환에 없다 — ledger 레코드는 (msg_id, recipient) 키라
            //   recipient 를 알아야 전이한다. sweep_expired 는 recipient 를 안 돌려주므로, msg_id 로 레코드를
            //   찾아 그 to 로 전이한다(records_for → 첫 pending 레코드). 조용한 유실 금지.
            transition_expired_by_msg_id(&mut st.ledger, &ex.msg_id, now);
        }
    }

    /// 봉투를 **현재** 전역 포맷으로 감싼다(단일 wrap point ADR-0096). C1 은 plain `<message from>` 만
    ///   (id/type/reply-by/in-reply-to/to 는 C3/C4). registry 에서 현재 포맷을 읽어 wrap_message 에 넘긴다.
    fn wrap_now(&self, sender: &str, msg_id: &str, body: &str) -> String {
        let format: EnvelopeFormat = self.registry.envelope_format();
        wrap_message(sender, msg_id, body, format, &EnvelopeFields::default())
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

    /// 관측/테스트용 — 수신자 큐 현재 길이.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn parked_len(&self, recipient: &str) -> usize {
        let st = self.state.lock().expect("messaging state poisoned");
        st.mailbox.len(recipient)
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
/// ★인코딩★: `<sender_len>\n<from_agent_id>\n<from_epoch>\n<entrance>\n<sender><body>` — 앞 4줄은 개행
///   없는 필드(uuid/숫자/`mcp`|`cli`)라 개행으로 안전 분리, sender/body 경계만 길이 접두(개행 안전).
struct ParkPayload {
    sender_name: String,
    from: BoundIdentity,
    entrance: Entrance,
    body: String,
}

impl ParkPayload {
    fn encode(&self) -> String {
        let ent = match self.entrance {
            Entrance::Mcp => "mcp",
            Entrance::Cli => "cli",
        };
        format!(
            "{}\n{}\n{}\n{}\n{}{}",
            self.sender_name.len(),
            self.from.agent_id,
            self.from.epoch,
            ent,
            self.sender_name,
            self.body
        )
    }

    /// encode 역연산. 형식이 깨지면(있을 수 없음 — 우리 인코딩) 전체를 body 로 폴백(조용한 유실보다 낫다).
    fn decode(s: &str) -> Self {
        // 앞 4개 개행 필드 + 나머지(sender+body). splitn(5) 로 body 안 개행을 보존.
        let mut it = s.splitn(5, '\n');
        let fallback = || Self {
            sender_name: String::new(),
            from: BoundIdentity {
                agent_id: AgentId::nil(),
                epoch: 0,
            },
            entrance: Entrance::Cli,
            body: s.to_string(),
        };
        let (Some(len_str), Some(id_str), Some(ep_str), Some(ent_str), Some(rest)) =
            (it.next(), it.next(), it.next(), it.next(), it.next())
        else {
            return fallback();
        };
        let (Ok(len), Ok(agent_id), Ok(epoch)) = (
            len_str.parse::<usize>(),
            id_str.parse::<AgentId>(),
            ep_str.parse::<u32>(),
        ) else {
            return fallback();
        };
        if rest.len() < len {
            return fallback();
        }
        let (sender, body) = rest.split_at(len);
        Self {
            sender_name: sender.to_string(),
            from: BoundIdentity { agent_id, epoch },
            entrance: if ent_str == "mcp" {
                Entrance::Mcp
            } else {
                Entrance::Cli
            },
            body: body.to_string(),
        }
    }
}

/// `to`(이름 또는 AgentId 문자열) → 산·도달 가능 수신자(LiveAgent). 매치 규칙(ingress::resolve_recipient
///   미러 — F2): AgentId 문자열 정확 일치 우선 → 이름 정확 일치. 동명 다수·부재는 여기선 None 으로 접고
///   상위(handle_send)가 AMBIGUOUS/파킹을 판정한다.
///
/// ★C1 스코프 = 단일 수신자★: 동명 다수(RECIPIENT_AMBIGUOUS)는 상위가 로스터로 판정(파킹 전에). 여기선
///   유일 매치만 Some — 0개 또는 2개+ 면 None. 상위가 로스터를 다시 보지 않도록 여기 로직을 최소로 둔다.
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
        /// live_reachable_agents 조회 횟수(late-appearance 스크립트용).
        roster_calls: StdMutex<usize>,
        /// 세팅되면 **첫 조회 이후**부터 이 roster 를 돌려준다(첫 조회 = resolve 는 원래 roster, 이후 =
        ///   self_heal 이 보는 late-appearance roster). finding 3 TOCTOU self-heal 재현용.
        roster_after_first: StdMutex<Option<Vec<LiveAgent>>>,
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
            }
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
            // 첫 조회(call 0) = 원래 roster(resolve 가 봄). 이후 = armed roster(있으면 — late appearance).
            if call >= 1 {
                if let Some(r) = self.roster_after_first.lock().unwrap().as_ref() {
                    return r.clone();
                }
            }
            self.roster.lock().unwrap().clone()
        }
        fn canonical_name(&self, id: AgentId) -> Option<String> {
            // roster 우선, 없으면 override(비-도달 산 에이전트) 조회 — finding 4 시나리오 지원.
            self.roster
                .lock()
                .unwrap()
                .iter()
                .find(|a| a.id == id)
                .map(|a| a.name.clone())
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

    #[test]
    fn deliver_ok_marks_delivered_and_injects_wrapped() {
        let (svc, port) = svc();
        let (_id, alice) = live("alice");
        port.set_roster(vec![alice]);
        let out = svc
            .handle_single_send("m1", ident(), "bob", "alice", "hi", Entrance::Mcp)
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
            .handle_single_send("m1", ident(), "bob", "ghost", "hi", Entrance::Cli)
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
            .handle_single_send("m1", ident(), "bob", "alice", "hi", Entrance::Cli)
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
            svc.handle_single_send(m, ident(), "s", "late", &format!("b{i}"), Entrance::Mcp)
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
            svc.handle_single_send(m, ident(), "s", "late", &format!("b{i}"), Entrance::Mcp)
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
            "재파킹분(b1,b2)이 cap 재충전에도 유실되지 않아야(restore_front cap 우회)"
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

    #[test]
    fn cap_exceeded_rejects_mailbox_full() {
        // 같은 부재 이름에 cap(100)까지 파킹 후 101번째 → MAILBOX_FULL 반려.
        let (svc, port) = svc();
        port.set_roster(vec![]);
        for i in 0..100 {
            svc.handle_single_send(&format!("m{i}"), ident(), "s", "full", "x", Entrance::Mcp)
                .expect("cap 이내 파킹");
        }
        assert_eq!(svc.parked_len("full"), 100);
        let rej = svc
            .handle_single_send("over", ident(), "s", "full", "x", Entrance::Mcp)
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
        svc.handle_single_send("m1", ident(), "s", "ghost", "hi", Entrance::Mcp)
            .expect("park");
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Pending]);
        // TTL(1h) + 1s 뒤 sweep — 파킹 시각은 handle_single_send 내부의 Instant::now() 라, 여기선 그보다
        //   충분히 미래인 now 를 준다. mailbox PARK_TTL = 1h.
        let now = Instant::now() + Duration::from_secs(60 * 60 + 1);
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
            .handle_single_send("m1", ident(), "s", "late", "hi", Entrance::Mcp)
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
            .handle_single_send("m1", ident(), "s", "dup", "hi", Entrance::Mcp)
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
        svc.handle_single_send("m1", ident(), "s", "dup", "hi", Entrance::Mcp)
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
        svc.handle_single_send("m1", ident(), "s", "gone", "hi", Entrance::Mcp)
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
        svc.handle_single_send("m1", ident(), "s", "recv", "hi", Entrance::Mcp)
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
        };
        let d = ParkPayload::decode(&p.encode());
        assert_eq!(d.sender_name, "qa-alpha");
        assert_eq!(d.body, "line1\nline2");
        assert_eq!(d.from.agent_id, from.agent_id);
        assert_eq!(d.from.epoch, 3);
        assert!(matches!(d.entrance, Entrance::Mcp));
    }
}
