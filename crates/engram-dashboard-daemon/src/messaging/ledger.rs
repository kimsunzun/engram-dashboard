//! ledger — 메시지 이력 + request 회신 추적 + 그룹 배달 장부(spec §2·§3·§5).
//!
//! ★역할★: 세 축을 담는다.
//!   ① **이력 링버퍼** — 전 메시지의 상태 전이 + 시각(`pending→delivered→replied` / `expired` / `skipped`).
//!      "상태 전이 시각이 곧 회신·발신 시각 데이터"(봉투 미노출 — spec §5). 용량 초과 = 오래된 것부터 evict.
//!   ② **request 추적** — `awaiting_reply` 오픈 + `in_reply_to` **엄격 매칭**으로 닫기 + `reply_by` 초과
//!      타임아웃 산출(발신자에게 notice 는 후속 increment 가 생성 — 여기선 "누가 초과했나"만 산출).
//!   ③ **그룹 배달 장부** — 메시지 1 : 배달기록 N(spec §4). 죽은 멤버 `skipped` 지원.
//!
//! ★순수·주입 시계(load-bearing — 모듈 헤더 불변식)★: 상태 전이·타임아웃 판정의 모든 시각은 `now: Instant`
//!   를 인자로 받는다. 링버퍼·추적 맵에 시계가 없다 — TTL·reply-by 경계를 결정적 단위 테스트로 단언한다.
//!
//! ★엄격 회신 매칭(load-bearing — spec §2 · ADR-0103 불변식)★: 회신은 `in_reply_to` 가 **정확히** 오픈된
//!   request id 를 가리킬 때만 그 request 를 닫는다. 관대 매칭(미회신 상대의 다음 메시지를 회신 간주)은
//!   우연 닫힘 오발이라 거부됐다 — 틀린 id 는 아무 것도 닫지 않는다.
// ADR-0103
// ADR-0104

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// 이력 링버퍼 용량 — 초과 시 가장 오래된 레코드부터 evict(spec §5 "이력 링버퍼").
///
/// ★조율 대상(사용자 비준 필요)★: 1024 는 "데몬 1회 수명의 메시지 이력을 담기에 충분" 이라는 어림값이다 —
///   spec 은 "이력 링버퍼" 만 명시하고 정확한 수는 정하지 않았다(TTL 1h·cap 100 만 못박음). 인메모리 단계라
///   메모리 상한 겸 evict 경계이므로 실사용 관측 후 사용자가 조정한다(v1 스코프: 값 하나, 무파괴 변경 가능).
/// ★evict 와 request 추적의 관계(finding 6 보정)★: request 추적은 별도 맵(`requests`)이지만 **이력 evict 에
///   결박된다** — evict 되는 레코드의 msg_id 와 같은 request_id 의 오픈 추적 항목을 함께 드롭한다(`record`
///   참조). 안 그러면 조회 불가해진 레코드를 가리키는 추적이 살아남아 spurious 타임아웃·무계 증식을 낳는다.
///   즉 이력 용량이 회신 계약 추적의 상한도 겸한다(인메모리 v1 유계 보장 — 이력이 사라지면 계약도 추적 밖).
const HISTORY_CAPACITY: usize = 1024;

/// 메시지 배달 1건의 상태(spec §5 상태 어휘 — 새 어휘 발명 금지).
///
/// ★상태 전이(load-bearing)★: `Pending → Delivered → Replied`(request 만) / `Expired`(TTL) / `Skipped`(그룹
///   방송에서 죽은 멤버). 각 전이는 시각을 남긴다(spec §5 "상태 전이 시각이 곧 회신·발신 시각"). busy 대기·
///   부재 파킹은 둘 다 `Pending`(상태 어휘 공유 — spec §5 분기 1 보정).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryStatus {
    /// 주입 대기(부재 파킹 또는 busy 대기 — 어휘 공유, spec §5).
    Pending,
    /// 실제 주입 완료(delivered = 실제 주입 시점, ADR-0104 불변식).
    Delivered,
    /// request 에 회신이 도착해 닫힘(엄격 매칭 성공).
    Replied,
    /// TTL(1h) 초과로 파킹 만료(장부 잔존, spec §5).
    Expired,
    /// 그룹 방송에서 죽은 멤버라 배달 안 함(spec §4 — 방송 소급 금지).
    Skipped,
}

impl DeliveryStatus {
    /// 이 상태에서 `next` 로의 전이가 **합법**인가(spec §5 상태 전이 그래프 — load-bearing).
    ///
    /// ★합법 전이 그래프(spec §5)★:
    ///   - `Pending → Delivered`(실제 주입)
    ///   - `Pending → Expired`(TTL 초과 — 주입 전 만료)
    ///   - `Pending → Skipped` / `Delivered → Skipped`(그룹 방송 미배달·중단 — as applicable)
    ///   - `Delivered → Replied`(request 회신 도착)
    /// 그 밖의 모든 간선은 불법이다 — 특히 **terminal**(`Replied`/`Expired`/`Skipped`)에서의 재전이,
    ///   그리고 되돌림(`*→Pending`)·건너뜀(`Pending→Replied`, `Expired→Delivered` 등)은 거부한다.
    ///   되돌림·건너뜀을 허용하면 "상태 전이 시각 = 회신·발신 시각" 이라는 장부 의미가 오염된다(오발 닫힘·
    ///   시각 소급). 같은 상태로의 자기 전이도 불법(무의미한 시각 갱신 방지)이다.
    fn can_transition_to(self, next: DeliveryStatus) -> bool {
        use DeliveryStatus::*;
        matches!(
            (self, next),
            (Pending, Delivered)
                | (Pending, Expired)
                | (Pending, Skipped)
                | (Delivered, Skipped)
                | (Delivered, Replied)
        )
    }
}

/// 이력 레코드 1건 — 한 (메시지, 수신자) 쌍의 배달 이력. 그룹 방송은 이 레코드 N개가 한 `msg_id` 를 공유한다.
///
/// ★메시지 1 : 배달기록 N(spec §4 · load-bearing)★: 그룹 발송은 하나의 논리 메시지(`msg_id` 공유)를 여러
///   수신자에게 개별 배달하므로, 배달 레코드는 **수신자별로 하나**다(각자 status·시각 독립). 단일 발송은 N=1.
/// ★body 는 요약이 아니라 full 보관(설계 결정)★: 인메모리 단계라 별도 저장소가 없고, 파킹된 봉투 재주입·
///   장부 조회(`messages { id }` — spec §6)에 원문이 필요하다. 요약본만 두면 재주입·감사 때 원문 손실이다.
///   메모리는 링 용량(HISTORY_CAPACITY)이 상한 — v2 영속화(SQLite) 때 요약/오프로드를 재검토한다(무파괴).
#[derive(Debug, Clone)]
pub struct MessageRecord {
    /// 논리 메시지 id(그룹 방송은 여러 레코드가 공유 — 1:N 상관 키).
    pub msg_id: String,
    /// 발신자 이름(WYSIWYA — ADR-0101).
    pub from: String,
    /// 이 레코드의 수신자 이름. 그룹 방송이면 멤버 하나(레코드마다 다름).
    pub to: String,
    /// 본문 전문(요약 아님 — 위 struct 주석의 설계 결정).
    pub body: String,
    /// 현재 상태.
    pub status: DeliveryStatus,
    /// 레코드 생성(발신) 시각 = 발신 시각 데이터(봉투 미노출, spec §5).
    pub created_at: Instant,
    /// 상태가 마지막으로 전이된 시각(delivered/replied/expired/skipped 시점). 회신·완료 시각 데이터.
    pub transitioned_at: Instant,
}

/// 오픈된 request 추적 1건(spec §3). 이력 레코드와 **별도 맵**이라 링 evict 에 영향받지 않는다.
///
/// ★notified 플래그(load-bearing — 이중 통지 방지)★: `reply_by` 초과가 `due_timeouts` 로 한 번 보고되면
///   이 플래그를 세워 **다시 보고하지 않는다**(spec §7 "no double-notification"). 회신이 오면 `closed` 라
///   `due_timeouts` 대상에서 빠진다(replied 는 절대 due 로 안 나옴).
#[derive(Debug, Clone)]
struct RequestEntry {
    /// request 메시지 id(회신의 `in_reply_to` 가 이걸 정확히 가리켜야 닫힘 — 엄격 매칭).
    request_id: String,
    /// 요청 발신자(타임아웃 notice 를 받을 대상 — spec §3 "발신자에게").
    sender: String,
    /// 요청 수신자(누가 회신해야 하나 — 관측/보고용).
    recipient: String,
    /// 회신 기한(발송 기준 오프셋 — spec §3 "reply_by 시계는 발송 기준"). `None` = 기한 없음(타임아웃 없음).
    reply_by: Option<Duration>,
    /// 요청 오픈(발송) 시각 — reply_by 절대 기한 = created_at + reply_by.
    created_at: Instant,
    /// 회신으로 닫혔나(replied). true 면 due_timeouts 대상에서 제외.
    closed: bool,
    /// 타임아웃이 이미 보고됐나 — 이중 통지 방지(위 struct 주석).
    notified: bool,
}

/// request 회신 결과(엄격 매칭, spec §2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyOutcome {
    /// 오픈된 request 를 정확히 닫음(첫 유효 회신). 이력도 `Replied` 로 정상 전이됨.
    Closed,
    /// 계약(추적)은 닫혔으나 이력 `Replied` 전이가 불법 간선이라 못 갔음 — anomaly 관측용.
    ///
    /// ★왜 별도 variant(load-bearing — finding 1)★: 회신은 **실제로 일어났고**, 계약을 다시 여는 것이
    ///   더 나쁘다(정본은 추적). 그러니 계약은 계속 닫는다. 그러나 이력이 아직 `Pending`(미주입) 등
    ///   `Delivered → Replied` 간선을 못 타는 상태면 이력은 회신을 반영 못 한 채 남는다 — 이걸 조용히
    ///   삼키지 않고(예전엔 `Closed` 로 은폐) `from`(그 순간 이력 상태)을 실어 반환해 상위(MessagingService)가
    ///   **관측·로깅**할 수 있게 한다. 계약 닫힘과 이력 부기는 별개 관심사다: anomaly = observable, not silent.
    ClosedHistoryAnomaly { from: DeliveryStatus },
    /// 매칭되는 오픈 request 없음 — 틀린 id 이거나 이미 닫힘/미존재(엄격: 아무 것도 안 닫음).
    NoMatch,
    /// 이미 닫힌 request 에 대한 두 번째 회신 — no-op(중복 회신, 아래 close_on_reply 주석 참조).
    AlreadyClosed,
}

/// `transition` 실패 사유 — 불법 상태 전이(spec §5 그래프 위반) 또는 대상 레코드 부재.
///
/// ★왜 typed 에러인가(load-bearing)★: 예전 `transition` 은 `bool`(성공/미존재)만 냈고 **불법 전이를 조용히
///   수행**했다(`Expired → Delivered` 같은 되돌림·건너뜀 허용). 이는 "상태 전이 시각 = 회신·발신 시각"
///   장부 의미를 오염시킨다. 이제 불법 전이는 타입으로 거부해 상위가 버그를 즉시 감지한다(spec §5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionError {
    /// (msg_id, to) 레코드가 없음 — evict 됐거나 미존재.
    NotFound,
    /// 현재 상태에서 요청한 상태로의 전이가 합법 그래프에 없음(되돌림·건너뜀·terminal 재전이 등).
    Illegal {
        from: DeliveryStatus,
        to: DeliveryStatus,
    },
}

/// request 오픈 결과 — 중복 id 방어(spec §3 · 아래 open_request 주석).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenOutcome {
    /// 새 request 를 열었음.
    Opened,
    /// 같은 request_id 가 추적에 이미 존재(open/closed 무관)해 거부됨 — no-op. id 는 데몬 생성 유일값이라
    /// 재사용은 non-scenario(finding 2 — 관대 재오픈이 shadowing 버그를 낳아 제거).
    DuplicateId,
}

/// 타임아웃 초과 request 1건의 보고 정보(발신자에게 notice 를 만들 상위 increment 용).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueTimeout {
    /// 초과한 request id.
    pub request_id: String,
    /// notice 를 받을 발신자.
    pub sender: String,
    /// 회신하지 않은 수신자(notice 문구용).
    pub recipient: String,
}

/// 메시지 장부 — 이력 링버퍼 + request 추적. 순수(주입 시계).
#[derive(Debug)]
pub struct Ledger {
    /// 이력 링버퍼(오래된 순, front = 가장 오래됨). 용량 초과 시 front evict.
    history: VecDeque<MessageRecord>,
    /// 오픈/닫힘 request 추적. 별도 컬렉션이나 이력 evict 에 결박된다(evict 시 동반 드롭 — finding 6·record 참조).
    requests: Vec<RequestEntry>,
    /// 링버퍼 용량(테스트가 작은 값으로 evict 를 빨리 검증하도록 주입 가능).
    capacity: usize,
}

impl Default for Ledger {
    fn default() -> Self {
        Self::with_capacity(HISTORY_CAPACITY)
    }
}

impl Ledger {
    /// 기본 용량(HISTORY_CAPACITY) 장부.
    pub fn new() -> Self {
        Self::default()
    }

    /// 용량 주입형 — 단위 테스트가 작은 용량으로 evict 경계를 빠르게 검증한다(순수성 원칙과 정합).
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            history: VecDeque::new(),
            requests: Vec::new(),
            // capacity 0 은 무의미(항상 즉시 evict) — 최소 1 로 보정(방어).
            capacity: capacity.max(1),
        }
    }

    /// 새 메시지 배달 레코드를 이력에 append(초기 상태 지정). 용량 초과 시 가장 오래된 레코드 evict.
    ///
    /// ★초기 상태 인자화★: 단일/그룹 발송은 `Pending`(주입 대기) 또는 `Delivered`(즉시 주입 폴백)로,
    ///   그룹 죽은 멤버는 `Skipped` 로 시작하므로 호출자가 초기 상태를 정한다(spec §4·§5).
    /// ★evict = front(오래된 것) + request 추적 동반 정리(load-bearing)★: 링버퍼라 용량을 넘기면 가장
    ///   오래된 이력부터 버린다. 이때 **그 레코드에 매달린 오픈 request 추적 항목도 함께 드롭한다** —
    ///   안 그러면 evict 로 조회 불가해진 레코드를 가리키는 request 가 살아남아 (a) `due_timeouts` 가
    ///   조회 불가 레코드에 대해 spurious 타임아웃을 쏘고 (b) 추적 맵이 무계 증식한다. 이력 evict 를
    ///   회신 계약의 경계로 삼는다(이력이 사라지면 그 회신 계약도 추적 밖 — 인메모리 v1 의 유계 보장).
    pub fn record(
        &mut self,
        msg_id: &str,
        from: &str,
        to: &str,
        body: &str,
        status: DeliveryStatus,
        now: Instant,
    ) {
        self.history.push_back(MessageRecord {
            msg_id: msg_id.to_string(),
            from: from.to_string(),
            to: to.to_string(),
            body: body.to_string(),
            status,
            created_at: now,
            transitioned_at: now,
        });
        // 용량 초과 — 가장 오래된(front) 것부터 evict(오래된 순 유지). evict 되는 msg_id 와 request_id 가
        // 같은 오픈 추적 항목이 있으면 함께 드롭(위 주석 — dangling 방지·유계 유지).
        while self.history.len() > self.capacity {
            if let Some(evicted) = self.history.pop_front() {
                self.requests.retain(|r| r.request_id != evicted.msg_id);
            }
        }
    }

    /// (msg_id, to) 쌍의 이력 레코드를 새 상태로 전이하고 전이 시각을 기록한다. 불법 전이는 거부한다.
    ///
    /// ★왜 (msg_id, to) 로 지목★: 그룹 방송은 한 msg_id 에 수신자별 레코드가 N개라 msg_id 만으로는 어느
    ///   배달인지 특정 못 한다 — 수신자까지 함께 지목해 정확히 한 레코드를 전이한다(1:N 회계, spec §4).
    /// ★합법 전이만 허용(load-bearing — spec §5 그래프)★: 현재 상태에서 `status` 로의 간선이 합법 그래프
    ///   (`can_transition_to`)에 없으면 `TransitionError::Illegal` 로 거부한다 — 되돌림·건너뜀·terminal
    ///   재전이는 장부 시각 의미를 오염시키므로 상태를 바꾸지 않는다. 레코드가 없으면 `NotFound`.
    /// ★반환★: `Ok(())` = 전이 성공(now 를 전이 시각으로 기록). 그 외는 위 typed 에러.
    pub fn transition(
        &mut self,
        msg_id: &str,
        to: &str,
        status: DeliveryStatus,
        now: Instant,
    ) -> Result<(), TransitionError> {
        let Some(rec) = self
            .history
            .iter_mut()
            .find(|r| r.msg_id == msg_id && r.to == to)
        else {
            return Err(TransitionError::NotFound);
        };
        if !rec.status.can_transition_to(status) {
            return Err(TransitionError::Illegal {
                from: rec.status,
                to: status,
            });
        }
        rec.status = status;
        rec.transitioned_at = now;
        Ok(())
    }

    /// request 오픈 — `awaiting_reply` 추적 시작(spec §3 단계 2). 단일 수신자만(그룹 request 는 v1 거부 —
    /// spec §4, 그 거부는 상위 파이프라인이 하므로 여기선 단일 recipient 만 받는다).
    ///
    /// ★reply_by 시계 = 발송 기준(spec §3·§5 · ADR-0104)★: 절대 기한 = `created_at(now) + reply_by`. 수신
    ///   지연과 무관한 발신자 관점 계약이라 now(발송 시각)를 기준으로 굳힌다.
    /// ★중복 id 거부 — 오픈이든 닫힘이든 존재하면 거부(load-bearing · finding 2)★: 같은 `request_id` 가
    ///   추적에 **하나라도 있으면**(open OR closed) `DuplicateId` 로 거부한다(no-op). 메시지 id 는
    ///   **데몬이 생성하는 유일 값**이라 재사용이 애초에 non-scenario 다 — id 는 회신 매칭 키이므로 유일성이
    ///   구조적으로 보장된다(spec §3). 예전의 "닫힌 id 는 재오픈 허용" 관대함은 두 항목(닫힌 것 + 재오픈된 것)을
    ///   동시에 남겨 (a) 회신이 앞쪽 닫힌 항목을 먼저 만나 `AlreadyClosed` 오발, (b) 같은-id 이력 evict 가
    ///   재오픈 추적을 드롭하는 shadowing 버그를 낳았다. 유일성 전제이므로 재오픈 자체를 없애 이 클래스의
    ///   버그를 제거한다.
    pub fn open_request(
        &mut self,
        request_id: &str,
        sender: &str,
        recipient: &str,
        reply_by: Option<Duration>,
        now: Instant,
    ) -> OpenOutcome {
        // 같은 id 가 추적에 하나라도 있으면(open/closed 무관) 거부 — id 는 데몬 생성 유일값(재사용 non-scenario).
        if self.requests.iter().any(|r| r.request_id == request_id) {
            return OpenOutcome::DuplicateId;
        }
        self.requests.push(RequestEntry {
            request_id: request_id.to_string(),
            sender: sender.to_string(),
            recipient: recipient.to_string(),
            reply_by,
            created_at: now,
            closed: false,
            notified: false,
        });
        OpenOutcome::Opened
    }

    /// 회신 도착 처리 — **엄격 매칭**(spec §2 · ADR-0103 불변식). `in_reply_to` 가 오픈된 request id 를
    /// 정확히 가리킬 때만 그 request 를 닫고(`Closed`), 그 시각으로 이력 레코드를 `Replied` 전이한다.
    /// 틀린 id = `NoMatch`(아무 것도 안 닫음). 이미 닫힌 request 에 대한 두 번째 회신 = `AlreadyClosed`(no-op).
    ///
    /// ★엄격의 근거★: 관대 매칭(미회신 상대의 다음 메시지를 회신 간주)은 우연 닫힘 오발이라 거부됐다
    ///   (ADR-0103 거부 대안). 오직 `request_id == in_reply_to` 동등만 인정한다.
    /// ★회신자 신원 미검증(v1 의도적 — spec §2·§8)★: v1 엄격 매칭은 **`in_reply_to` 동등만** 본다 — 누가
    ///   회신했는지(회신자가 실제 그 request 의 recipient 인지)는 **일부러 검증하지 않는다**. 신원 강제는
    ///   ACL 이 들어오는 v2 로 미뤘다(spec §8) — 다음 세션이 "신원이 이미 강제된다" 고 오해하지 않도록 명시.
    /// ★now 로 회신 시각 기록(finding 4 · spec §5)★: `Closed` 시 request 추적을 닫는 것과 **원자적으로**
    ///   매칭 이력 레코드((request_id, recipient))를 `now`(회신 시각)로 `Replied` 전이한다. "상태 전이 시각이
    ///   곧 회신 시각" 이기 때문이다.
    /// ★이력 전이 실패의 정직한 반환(finding 1 · load-bearing)★: 이력 전이는 **best-effort** 지만 결과는
    ///   조용히 삼키지 않는다. 계약 닫힘과 이력 부기는 **별개 관심사**다 — 회신은 실제로 일어났으니 계약은
    ///   항상 닫고(재오픈이 더 나쁨), 이력이 반영 못 하면 그 사실을 variant 로 노출한다:
    ///     - 레코드 부재(evict 됨) → `NotFound`: 가리킬 이력이 아예 없으니 anomaly 아님 → 그냥 `Closed`.
    ///     - 불법 간선(`Illegal` — 아직 `Delivered` 아님 등) → 이력이 회신을 못 담은 채 남음 → 이건 관측
    ///       대상이라 `ClosedHistoryAnomaly { from }`(그 순간 이력 상태)으로 반환한다. 상위가 로깅·관측.
    ///   즉 예전에 `Closed` 로 은폐하던 불법 전이만 anomaly 로 승격한다(evict 는 정상 best-effort skip).
    /// ★두 번째 회신 = no-op 로 문서화★: 같은 request 에 두 번째 회신이 와도 상태를 되돌리거나 재-닫지
    ///   않는다(첫 회신이 이미 계약 이행). 에러가 아니라 `AlreadyClosed` 로 구분해 반환한다(상위 판단용).
    pub fn close_on_reply(&mut self, in_reply_to: &str, now: Instant) -> ReplyOutcome {
        // 1) 추적 항목을 닫는다(정본). recipient 를 꺼내 뒤이어 이력 전이에 쓴다(borrow 분리).
        let recipient = match self
            .requests
            .iter_mut()
            .find(|r| r.request_id == in_reply_to)
        {
            Some(r) if r.closed => return ReplyOutcome::AlreadyClosed,
            Some(r) => {
                r.closed = true;
                r.recipient.clone()
            }
            None => return ReplyOutcome::NoMatch,
        };
        // 2) 매칭 이력 레코드를 Replied 로 전이. 계약은 이미 닫혔다(위) — 여기 결과는 이력 부기 정직성만
        //    가른다. 불법 간선이면 이력이 회신을 못 담은 채 남으므로 anomaly 로 승격(위 주석), evict(NotFound)
        //    는 가리킬 레코드가 없어 정상 best-effort skip → Closed.
        match self.transition(in_reply_to, &recipient, DeliveryStatus::Replied, now) {
            Ok(()) => ReplyOutcome::Closed,
            Err(TransitionError::NotFound) => ReplyOutcome::Closed,
            Err(TransitionError::Illegal { from, .. }) => {
                ReplyOutcome::ClosedHistoryAnomaly { from }
            }
        }
    }

    /// 기한 초과된 미회신 request 목록을 산출한다(발신자에게 notice 를 만들 상위 increment 용).
    ///
    /// ★due 판정(spec §3 단계 4 · load-bearing)★: `reply_by` 가 있고, `now > created_at + reply_by`(경계
    ///   초과), 아직 열려 있고(`!closed`), 아직 통지 안 된(`!notified`) request 만 반환한다.
    /// ★이중 통지 방지(spec §7)★: 반환하며 **그 자리에서 notified 를 세운다** — 같은 request 는 다음 호출에
    ///   다시 나오지 않는다. 회신으로 닫힌(replied) request 는 `closed` 라 절대 반환하지 않는다.
    /// ★경계★: `>` 비교라 정확히 기한인 순간은 아직 due 아님(mailbox TTL 경계와 동일 규약 — 결정적 테스트).
    pub fn due_timeouts(&mut self, now: Instant) -> Vec<DueTimeout> {
        let mut due = Vec::new();
        for r in self.requests.iter_mut() {
            if r.closed || r.notified {
                continue;
            }
            let Some(reply_by) = r.reply_by else {
                continue; // 기한 없는 request 는 타임아웃 없음.
            };
            let deadline = r.created_at + reply_by;
            if now > deadline {
                r.notified = true; // 이중 통지 방지 — 반환 시점에 마킹.
                due.push(DueTimeout {
                    request_id: r.request_id.clone(),
                    sender: r.sender.clone(),
                    recipient: r.recipient.clone(),
                });
            }
        }
        due
    }

    /// 이력 레코드 수(관측/테스트).
    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// msg_id 로 이력 레코드들을 조회한다(그룹 방송은 여러 개 — `messages { id }` 조회 지원, spec §6).
    /// 오래된 순.
    pub fn records_for(&self, msg_id: &str) -> Vec<&MessageRecord> {
        self.history.iter().filter(|r| r.msg_id == msg_id).collect()
    }

    /// 오픈(미회신) request 수(관측/테스트). closed 제외.
    pub fn open_request_count(&self) -> usize {
        self.requests.iter().filter(|r| !r.closed).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> Instant {
        Instant::now()
    }

    // ── 이력 링버퍼 ──────────────────────────────────────────────────────────────
    #[test]
    fn record_appends_and_reports_status() {
        let mut l = Ledger::new();
        let now = t0();
        l.record("m1", "alice", "bob", "hi", DeliveryStatus::Pending, now);
        let recs = l.records_for("m1");
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].status, DeliveryStatus::Pending);
        assert_eq!(recs[0].from, "alice");
        assert_eq!(recs[0].to, "bob");
    }

    #[test]
    fn ring_buffer_evicts_oldest_at_capacity() {
        let mut l = Ledger::with_capacity(3);
        let now = t0();
        for i in 0..5 {
            l.record(
                &format!("m{i}"),
                "a",
                "b",
                "x",
                DeliveryStatus::Delivered,
                now,
            );
        }
        assert_eq!(l.history_len(), 3, "용량 상한 유지");
        // 가장 오래된 m0·m1 은 evict, m2·m3·m4 만 남아야.
        assert!(l.records_for("m0").is_empty(), "가장 오래된 것부터 evict");
        assert!(l.records_for("m1").is_empty());
        assert_eq!(l.records_for("m4").len(), 1, "최근 것은 잔존");
    }

    #[test]
    fn transition_records_timestamp_and_status() {
        let mut l = Ledger::new();
        let now = t0();
        l.record("m1", "a", "b", "x", DeliveryStatus::Pending, now);
        let later = now + Duration::from_secs(5);
        assert_eq!(
            l.transition("m1", "b", DeliveryStatus::Delivered, later),
            Ok(())
        );
        let rec = l.records_for("m1")[0];
        assert_eq!(rec.status, DeliveryStatus::Delivered);
        assert_eq!(rec.transitioned_at, later, "전이 시각 기록");
        assert_eq!(rec.created_at, now, "발신 시각은 불변");
    }

    #[test]
    fn transition_targets_recipient_for_group_broadcast() {
        // 메시지 1 : 배달기록 N — 같은 msg_id, 수신자별 독립 전이(spec §4).
        let mut l = Ledger::new();
        let now = t0();
        l.record("g1", "boss", "a", "rebase", DeliveryStatus::Pending, now);
        l.record("g1", "boss", "b", "rebase", DeliveryStatus::Pending, now);
        l.record("g1", "boss", "c", "rebase", DeliveryStatus::Skipped, now); // 죽은 멤버
                                                                             // a 만 delivered 로 전이 — b·c 는 안 건드려짐.
        let later = now + Duration::from_secs(1);
        assert_eq!(
            l.transition("g1", "a", DeliveryStatus::Delivered, later),
            Ok(())
        );
        let recs = l.records_for("g1");
        assert_eq!(recs.len(), 3, "한 msg_id 에 배달기록 3개");
        let a = recs.iter().find(|r| r.to == "a").unwrap();
        let b = recs.iter().find(|r| r.to == "b").unwrap();
        let c = recs.iter().find(|r| r.to == "c").unwrap();
        assert_eq!(a.status, DeliveryStatus::Delivered);
        assert_eq!(b.status, DeliveryStatus::Pending, "b 는 안 건드려짐");
        assert_eq!(c.status, DeliveryStatus::Skipped, "죽은 멤버 skipped");
    }

    #[test]
    fn transition_missing_record_returns_not_found() {
        let mut l = Ledger::new();
        assert_eq!(
            l.transition("nope", "b", DeliveryStatus::Delivered, t0()),
            Err(TransitionError::NotFound)
        );
    }

    #[test]
    fn transition_rejects_illegal_edges() {
        // spec §5 그래프 위반 간선은 typed Illegal 로 거부되고 상태를 안 바꿈.
        let mut l = Ledger::new();
        let now = t0();

        // Expired → Delivered (되돌림·건너뜀) 거부.
        l.record("e1", "a", "b", "x", DeliveryStatus::Pending, now);
        assert_eq!(
            l.transition("e1", "b", DeliveryStatus::Expired, now),
            Ok(()),
            "Pending → Expired 는 합법"
        );
        assert_eq!(
            l.transition(
                "e1",
                "b",
                DeliveryStatus::Delivered,
                now + Duration::from_secs(1)
            ),
            Err(TransitionError::Illegal {
                from: DeliveryStatus::Expired,
                to: DeliveryStatus::Delivered
            }),
            "Expired → Delivered 는 불법"
        );
        assert_eq!(
            l.records_for("e1")[0].status,
            DeliveryStatus::Expired,
            "불법 전이는 상태를 안 바꿈"
        );

        // Pending → Replied (건너뜀 — Delivered 를 거쳐야) 거부.
        l.record("p1", "a", "b", "x", DeliveryStatus::Pending, now);
        assert_eq!(
            l.transition("p1", "b", DeliveryStatus::Replied, now),
            Err(TransitionError::Illegal {
                from: DeliveryStatus::Pending,
                to: DeliveryStatus::Replied
            }),
            "Pending → Replied 는 불법(Delivered 경유 필요)"
        );

        // Replied → Pending (되돌림, terminal 재전이) 거부.
        l.record("r1", "a", "b", "x", DeliveryStatus::Pending, now);
        assert_eq!(
            l.transition("r1", "b", DeliveryStatus::Delivered, now),
            Ok(())
        );
        assert_eq!(
            l.transition("r1", "b", DeliveryStatus::Replied, now),
            Ok(())
        );
        assert_eq!(
            l.transition("r1", "b", DeliveryStatus::Pending, now),
            Err(TransitionError::Illegal {
                from: DeliveryStatus::Replied,
                to: DeliveryStatus::Pending
            }),
            "Replied → Pending 은 불법(terminal 되돌림)"
        );

        // Skipped → Pending (되돌림) 거부.
        l.record("s1", "a", "b", "x", DeliveryStatus::Skipped, now);
        assert_eq!(
            l.transition("s1", "b", DeliveryStatus::Pending, now),
            Err(TransitionError::Illegal {
                from: DeliveryStatus::Skipped,
                to: DeliveryStatus::Pending
            }),
            "Skipped → Pending 은 불법"
        );
    }

    #[test]
    fn transition_accepts_legal_edges() {
        let now = t0();
        // Pending → Delivered → Replied.
        let mut l = Ledger::new();
        l.record("m", "a", "b", "x", DeliveryStatus::Pending, now);
        assert_eq!(
            l.transition("m", "b", DeliveryStatus::Delivered, now),
            Ok(())
        );
        assert_eq!(l.transition("m", "b", DeliveryStatus::Replied, now), Ok(()));
        // Pending → Skipped, Delivered → Skipped, Pending → Expired.
        let mut l2 = Ledger::new();
        l2.record("a", "x", "y", "b", DeliveryStatus::Pending, now);
        assert_eq!(
            l2.transition("a", "y", DeliveryStatus::Skipped, now),
            Ok(())
        );
        l2.record("c", "x", "y", "b", DeliveryStatus::Pending, now);
        assert_eq!(
            l2.transition("c", "y", DeliveryStatus::Delivered, now),
            Ok(())
        );
        assert_eq!(
            l2.transition("c", "y", DeliveryStatus::Skipped, now),
            Ok(())
        );
        l2.record("d", "x", "y", "b", DeliveryStatus::Pending, now);
        assert_eq!(
            l2.transition("d", "y", DeliveryStatus::Expired, now),
            Ok(())
        );
    }

    // ── request 엄격 회신 매칭 ────────────────────────────────────────────────────
    #[test]
    fn strict_reply_closes_exact_match() {
        let mut l = Ledger::new();
        let now = t0();
        assert_eq!(
            l.open_request("req-1", "alice", "bob", None, now),
            OpenOutcome::Opened
        );
        assert_eq!(l.open_request_count(), 1);
        assert_eq!(l.close_on_reply("req-1", now), ReplyOutcome::Closed);
        assert_eq!(l.open_request_count(), 0, "회신으로 닫힘");
    }

    #[test]
    fn strict_reply_wrong_id_does_not_close() {
        let mut l = Ledger::new();
        let now = t0();
        l.open_request("req-1", "alice", "bob", None, now);
        // 틀린 id 회신 = NoMatch, 아무 것도 안 닫음(엄격 매칭 — 우연 닫힘 오발 거부).
        assert_eq!(l.close_on_reply("req-999", now), ReplyOutcome::NoMatch);
        assert_eq!(l.open_request_count(), 1, "틀린 id 는 request 를 안 닫아야");
    }

    #[test]
    fn second_reply_to_same_request_is_already_closed_noop() {
        let mut l = Ledger::new();
        let now = t0();
        l.open_request("req-1", "alice", "bob", None, now);
        assert_eq!(l.close_on_reply("req-1", now), ReplyOutcome::Closed);
        // 두 번째 회신 = AlreadyClosed(no-op — 첫 회신만 유효, 문서화된 동작).
        assert_eq!(l.close_on_reply("req-1", now), ReplyOutcome::AlreadyClosed);
        assert_eq!(l.open_request_count(), 0);
    }

    #[test]
    fn duplicate_open_request_id_is_rejected() {
        // 같은 request_id 로 두 번 열면 둘째는 DuplicateId(no-op) — 회신 매칭 키 유일성(finding 2·5).
        let mut l = Ledger::new();
        let now = t0();
        assert_eq!(
            l.open_request("req-1", "alice", "bob", None, now),
            OpenOutcome::Opened
        );
        assert_eq!(
            l.open_request("req-1", "alice", "carol", None, now),
            OpenOutcome::DuplicateId,
            "중복 오픈 id 는 거부"
        );
        assert_eq!(l.open_request_count(), 1, "중복은 추적에 추가 안 됨");
    }

    #[test]
    fn closed_id_cannot_be_reopened_and_reply_stays_already_closed() {
        // finding 2: 닫힌 id 재오픈도 거부(id = 데몬 생성 유일값, 재사용 non-scenario).
        //   관대 재오픈은 닫힌 항목 + 재오픈 항목을 동시에 남겨 shadowing 버그를 낳았다 — 이제 아예 막는다.
        let mut l = Ledger::new();
        let now = t0();
        assert_eq!(
            l.open_request("req-1", "alice", "bob", None, now),
            OpenOutcome::Opened
        );
        assert_eq!(l.close_on_reply("req-1", now), ReplyOutcome::Closed);
        // 닫힌 뒤 같은 id 재오픈 시도 → 거부(추적에 여전히 존재).
        assert_eq!(
            l.open_request("req-1", "alice", "bob", None, now),
            OpenOutcome::DuplicateId,
            "닫힌 id 재오픈은 거부(유일성 전제)"
        );
        assert_eq!(l.open_request_count(), 0, "재오픈 안 됐으니 오픈 0");
        // 재오픈이 안 됐으므로 회신 동작은 여전히 AlreadyClosed(첫 회신만 유효 — shadowing 없음).
        assert_eq!(
            l.close_on_reply("req-1", now),
            ReplyOutcome::AlreadyClosed,
            "재오픈 없이 회신하면 AlreadyClosed(shadowing 없음)"
        );
    }

    #[test]
    fn close_on_reply_transitions_history_to_replied_with_timestamp() {
        // finding 4: Closed 시 매칭 이력 레코드를 회신 시각으로 Replied 전이(원자적).
        let mut l = Ledger::new();
        let now = t0();
        // request 발송 이력 + 추적 오픈(request_id = msg_id, recipient = to).
        l.record(
            "req-1",
            "alice",
            "bob",
            "질문",
            DeliveryStatus::Pending,
            now,
        );
        l.open_request("req-1", "alice", "bob", None, now);
        // 주입(Delivered) — Delivered → Replied 만 합법이므로 선행 필요.
        let delivered_at = now + Duration::from_secs(1);
        assert_eq!(
            l.transition("req-1", "bob", DeliveryStatus::Delivered, delivered_at),
            Ok(())
        );
        // 회신 도착.
        let reply_at = now + Duration::from_secs(30);
        assert_eq!(l.close_on_reply("req-1", reply_at), ReplyOutcome::Closed);
        let rec = l.records_for("req-1")[0];
        assert_eq!(rec.status, DeliveryStatus::Replied, "이력이 Replied 로");
        assert_eq!(
            rec.transitioned_at, reply_at,
            "전이 시각 = 회신 시각(spec §5)"
        );
    }

    #[test]
    fn close_on_reply_against_pending_history_is_anomaly_but_still_closes() {
        // finding 1: 이력이 아직 Pending(미주입)이라 Delivered→Replied 간선이 없으면 계약은 닫되(정본은
        //   추적) 이력 전이 실패를 조용히 삼키지 않고 ClosedHistoryAnomaly 로 정직하게 보고한다.
        let mut l = Ledger::new();
        let now = t0();
        l.record("req-1", "alice", "bob", "q", DeliveryStatus::Pending, now);
        l.open_request("req-1", "alice", "bob", None, now);
        assert_eq!(
            l.close_on_reply("req-1", now),
            ReplyOutcome::ClosedHistoryAnomaly {
                from: DeliveryStatus::Pending
            },
            "불법 이력 전이는 anomaly 로 노출(은폐 금지)"
        );
        assert_eq!(
            l.open_request_count(),
            0,
            "그래도 계약은 닫힘(재오픈이 더 나쁨)"
        );
        assert_eq!(
            l.records_for("req-1")[0].status,
            DeliveryStatus::Pending,
            "이력은 불법 전이라 안 건드려짐(그대로 Pending)"
        );
    }

    #[test]
    fn close_on_reply_with_evicted_history_is_plain_closed_not_anomaly() {
        // finding 1 경계: 이력이 evict 돼 가리킬 레코드가 아예 없으면(NotFound) anomaly 아님 → 그냥 Closed.
        //   추적은 이력 evict 에 결박되므로, 이 케이스를 만들려면 이력만 지우고 추적을 남긴다 — capacity 1 로
        //   같은 recipient 다른 msg 를 밀어넣되 추적은 다른 id 로 열어 evict-결박을 피한다.
        let mut l = Ledger::with_capacity(1);
        let now = t0();
        // req-1 이력은 곧 밀려나지만, 추적은 record 와 무관하게 열 수 있다(별도 맵).
        l.open_request("req-1", "alice", "bob", None, now);
        // 다른 msg_id 이력을 밀어넣어 req-1 이력 레코드가 존재하지 않게 만든다(애초에 record 안 함 = NotFound).
        l.record("other", "x", "y", "z", DeliveryStatus::Delivered, now);
        // req-1 이력 레코드는 없음 → transition NotFound → 정상 best-effort skip → 그냥 Closed.
        assert_eq!(
            l.close_on_reply("req-1", now),
            ReplyOutcome::Closed,
            "가리킬 이력 없음(NotFound)은 anomaly 아님 → Closed"
        );
        assert_eq!(l.open_request_count(), 0, "계약 닫힘");
    }

    // ── reply_by 타임아웃 ─────────────────────────────────────────────────────────
    #[test]
    fn due_timeout_respects_deadline_boundary() {
        let mut l = Ledger::new();
        let now = t0();
        let reply_by = Duration::from_secs(600); // 10m
        l.open_request("req-1", "alice", "bob", Some(reply_by), now);
        // 정확히 기한인 순간 = 아직 due 아님(`>` 경계).
        assert!(
            l.due_timeouts(now + reply_by).is_empty(),
            "정확히 기한은 due 아님"
        );
        // 기한 초과 = due.
        let due = l.due_timeouts(now + reply_by + Duration::from_nanos(1));
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].request_id, "req-1");
        assert_eq!(due[0].sender, "alice", "notice 는 발신자에게(spec §3)");
        assert_eq!(due[0].recipient, "bob");
    }

    #[test]
    fn due_timeout_no_double_notification() {
        let mut l = Ledger::new();
        let now = t0();
        let reply_by = Duration::from_secs(600);
        l.open_request("req-1", "alice", "bob", Some(reply_by), now);
        let over = now + reply_by + Duration::from_secs(1);
        assert_eq!(l.due_timeouts(over).len(), 1, "첫 산출은 보고");
        assert!(
            l.due_timeouts(over).is_empty(),
            "두 번째 호출은 이미 통지된 request 를 다시 안 냄(이중 통지 방지)"
        );
    }

    #[test]
    fn replied_request_excluded_from_due_timeouts() {
        let mut l = Ledger::new();
        let now = t0();
        let reply_by = Duration::from_secs(600);
        l.open_request("req-1", "alice", "bob", Some(reply_by), now);
        // 기한 전에 회신 도착 → 닫힘.
        assert_eq!(l.close_on_reply("req-1", now), ReplyOutcome::Closed);
        // 기한을 넘겨도 replied(closed) request 는 절대 due 로 안 나옴.
        let over = now + reply_by + Duration::from_secs(60);
        assert!(
            l.due_timeouts(over).is_empty(),
            "회신된 request 는 타임아웃 대상 아님"
        );
    }

    #[test]
    fn request_without_reply_by_never_times_out() {
        let mut l = Ledger::new();
        let now = t0();
        l.open_request("req-1", "alice", "bob", None, now);
        // 기한 없으면 아무리 시간이 지나도 due 아님.
        let far = now + Duration::from_secs(100_000);
        assert!(
            l.due_timeouts(far).is_empty(),
            "기한 없는 request 는 타임아웃 없음"
        );
    }

    #[test]
    fn skipped_status_for_group_dead_member() {
        // 그룹 방송 죽은 멤버 = Skipped 로 기록(spec §4 방송 소급 금지).
        let mut l = Ledger::new();
        let now = t0();
        l.record("g1", "boss", "dead", "msg", DeliveryStatus::Skipped, now);
        assert_eq!(l.records_for("g1")[0].status, DeliveryStatus::Skipped);
    }

    // ── evict ↔ request 추적 결합(finding 6) ────────────────────────────────────
    #[test]
    fn eviction_drops_dangling_request_tracking() {
        // 용량 초과로 request 이력 레코드가 evict 되면 그 오픈 추적도 함께 드롭돼야:
        //   ① due_timeouts 가 evict/조회불가 레코드에 spurious 타임아웃을 쏘지 않음
        //   ② 추적 맵이 무계 증식하지 않음(유계)
        let cap = 3;
        let mut l = Ledger::with_capacity(cap);
        let now = t0();
        let reply_by = Duration::from_secs(600);
        // 용량을 넘겨 request 를 여러 건 연다 — 각각 이력 1건 + 추적 1건.
        let total = cap + 3; // 6건 → 앞 3건(m0..m2)은 evict.
        for i in 0..total {
            let id = format!("m{i}");
            l.record(&id, "alice", "bob", "q", DeliveryStatus::Pending, now);
            l.open_request(&id, "alice", "bob", Some(reply_by), now);
        }
        // 이력은 용량 상한.
        assert_eq!(l.history_len(), cap, "이력은 용량 유계");
        // 추적도 evict 된 만큼 정리돼 유계(살아남은 이력 수와 일치).
        assert_eq!(
            l.open_request_count(),
            cap,
            "evict 된 request 추적은 드롭돼 유계"
        );
        // 기한 초과 시 evict 된 id 는 due 로 안 나오고, 살아남은 것만 나온다.
        let over = now + reply_by + Duration::from_secs(1);
        let due = l.due_timeouts(over);
        let due_ids: Vec<&str> = due.iter().map(|d| d.request_id.as_str()).collect();
        assert_eq!(due.len(), cap, "살아남은 request 만 due");
        assert!(
            !due_ids.contains(&"m0") && !due_ids.contains(&"m1") && !due_ids.contains(&"m2"),
            "evict 된 id 는 spurious 타임아웃 안 남(dangling 없음)"
        );
        assert!(
            due_ids.contains(&"m3") && due_ids.contains(&"m4") && due_ids.contains(&"m5"),
            "살아남은 id 는 정상 due"
        );
    }
}
