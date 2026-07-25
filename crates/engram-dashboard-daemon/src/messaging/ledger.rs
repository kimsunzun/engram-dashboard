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

use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

use engram_dashboard_core::agent::types::AgentId;

/// 이력 링버퍼 용량 — 초과 시 가장 오래된 레코드부터 evict(spec §5 "이력 링버퍼").
///
/// ★조율 대상(사용자 비준 필요)★: 1024 는 "데몬 1회 수명의 메시지 이력을 담기에 충분" 이라는 어림값이다 —
///   spec 은 "이력 링버퍼" 만 명시하고 정확한 수는 정하지 않았다(TTL 24h·cap 100 만 못박음, ADR-0105). 인메모리 단계라
///   메모리 상한 겸 evict 경계이므로 실사용 관측 후 사용자가 조정한다(v1 스코프: 값 하나, 무파괴 변경 가능).
/// ★evict 와 request 추적의 관계(C3 리뷰 fix 3 로 좁혀짐)★: 이력 evict 는 **끝난 계약**(closed 또는 이미
///   통지된)의 추적 항목만 함께 드롭한다. 살아 있는(미회신·미통지) 계약은 evict 를 견디고 남는다 — 예전엔
///   무조건 드롭해서, 이력이 밀려난 오픈 request 가 **회신으로 닫힐 길과 기한 초과 통지를 동시에 잃었다**
///   (조용한 계약 소멸 = 최악 실패 모드). 유계는 이제 이력 용량이 아니라 `MAX_OPEN_REQUESTS` 가 준다.
const HISTORY_CAPACITY: usize = 1024;

/// 동시에 열려 있을 수 있는(미회신·미통지) request 계약 수의 상한.
///
/// ★왜 필요한가(fix 3 의 짝 — load-bearing)★: 오픈 계약이 이력 evict 를 견디게 바꾼 순간(위 상수 주석),
///   추적 목록의 상한을 **이력 용량이 더 이상 대신 주지 않는다**. 상한이 없으면 회신이 영영 안 오는
///   request(기한 없는 것 포함)가 쌓여 인메모리 v1 의 유계 보장이 깨진다. 그래서 오픈 계약 자체에 cap 을
///   두고, cap 에서는 **새 request 를 반려**한다(오래된 계약을 조용히 버리지 않는다 — 조용한 유실 금지).
/// ★512 의 근거★: 보관함 cap 100 × 동시 수신자 수십 규모를 넉넉히 덮는 어림값이다. 사람 대화 수준
///   메시지율에서 오픈 계약이 이 수에 닿는다면 그건 정상 부하가 아니라 회신하지 않는 상대가 쌓인 것이므로,
///   반려로 발신자에게 가시화하는 게 맞다(HISTORY_CAPACITY 와 같은 성격의 조율 대상 값 — 무파괴 변경 가능).
const MAX_OPEN_REQUESTS: usize = 512;

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
    /// TTL(24h) 초과로 파킹 만료(장부 잔존, spec §5, ADR-0105).
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
    /// 요청 발신자 이름(타임아웃 notice 를 받을 대상 — spec §3 "발신자에게"). **발송 시점의** 표시 이름이다.
    sender: String,
    /// ★요청 발신자의 AgentId(C3 리뷰 fix 2 — load-bearing)★: 이름은 발송 후 바뀔 수 있고(display_name
    ///   변경), 그러면 이름-키 파킹만으로는 notice 가 옛 이름 큐에 갇혀 **영영 배달되지 않는다**(통지는
    ///   `notified` 라 재발화도 없다 = 계약이 조용히 반쪽). id 를 함께 들고 있으면 상위가 그걸 파킹 힌트로
    ///   실어 이름과 무관하게 그 incarnation 으로 배달할 수 있다.
    sender_id: AgentId,
    /// 요청 수신자(누가 회신해야 하나 — 관측/보고용).
    recipient: String,
    /// 회신 기한 = (발송 기준 오프셋, **발신자가 쓴 표기 원본**). `None` = 기한 없음(타임아웃 없음).
    ///
    /// ★왜 표기를 함께 보관하나(C3 리뷰 fix 6)★: 예전엔 Duration 만 두고 통지 문구를 만들 때 상위가 표기를
    ///   **역산**했다 — 그 역산이 정규화라 `60m` 로 보낸 기한이 `1h` 로 통지돼 봉투(`reply-by="60m"`)와
    ///   문구가 어긋났다. 계약 문구는 발신자가 쓴 그대로여야 하므로 표기를 원본째 보관한다(둘을 한 튜플로
    ///   묶어 "기한이 있으면 표기도 반드시 있다" 를 타입으로 강제한다).
    reply_by: Option<(Duration, String)>,
    /// 요청 오픈(발송) 시각 — reply_by 절대 기한 = created_at + reply_by.
    created_at: Instant,
    /// 회신으로 닫혔나(replied). true 면 due_timeouts 대상에서 제외.
    closed: bool,
    /// 타임아웃이 이미 보고됐나 — 이중 통지 방지(위 struct 주석).
    notified: bool,
}

impl RequestEntry {
    /// 아직 **살아 있는** 계약인가 — 회신도 안 왔고 통지도 안 나간 상태. 이 부류만 이력 evict 를 견디고
    /// (`record`), `MAX_OPEN_REQUESTS` cap 의 계수 대상이다. 끝난 계약이 무계가 아닌 근거는 두 갈래다:
    /// 이력이 남아 있으면 그 이력이 evict 될 때, 이력이 이미 없으면 **끝나는 그 순간**
    /// (`purge_finished_without_history` — fix 1) 정리된다.
    fn is_live(&self) -> bool {
        !self.closed && !self.notified
    }
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
    /// ★오픈 계약 cap(`MAX_OPEN_REQUESTS`) 도달(C3 리뷰 fix 3)★ — 새 계약을 열지 않는다(no-op). 상위가
    /// 발신자에게 반려로 가시화한다(오래된 계약을 조용히 버리는 대신 새 것을 거절 — 조용한 유실 금지).
    Full,
}

/// `drop_request` 결과 — 제거 여부 + **그 계약이 이미 통지된 상태였는지**(C3 리뷰 fix 5).
///
/// ★왜 notified 를 함께 돌려주나(load-bearing — 관측)★: 반려 회수(`drop_request`)는 "계약이 성립한 적 없음"
///   을 뜻하는데, 그 항목이 이미 `notified` 였다면 **기한 초과 통지가 이미 발신자에게 나간 뒤**라는 말이다
///   (통지는 회수할 수 없다 — 이미 나간 메시지다). 이 이중 결말("통지도 갔고 반려도 됐다")은 드물지만
///   조용히 넘기면 안 되는 상태라 호출자가 로그로 남길 수 있게 사실을 함께 반환한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropOutcome {
    /// 항목을 제거했다. `notified` = 제거 시점에 이미 타임아웃 통지가 나간 상태였나.
    Removed { notified: bool },
    /// 그런 id 가 추적에 없다(멱등 — no-op).
    NotFound,
}

/// 타임아웃 초과 request 1건의 보고 정보(발신자에게 notice 를 만드는 `MessagingService` 용).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueTimeout {
    /// 초과한 request id.
    pub request_id: String,
    /// notice 를 받을 발신자 이름(**발송 시점** 표시 이름 — 그 뒤 개명됐을 수 있다).
    pub sender: String,
    /// ★notice 배달용 발신자 id(C3 리뷰 fix 2)★ — 이름이 바뀌었어도 이 id 로 배달 경로를 찾는다
    ///   (`RequestEntry.sender_id` 주석). 상위가 파킹 힌트 + flush 도어벨 대상으로 쓴다.
    pub sender_id: AgentId,
    /// 회신하지 않은 수신자(notice 문구용).
    pub recipient: String,
    /// ★초과된 기한의 **표기 원본**(C3 리뷰 fix 6 — notice 문구용)★: spec §1 notice 템플릿이
    ///   `기한({reply_by})` 을 그대로 노출하므로, 발신자가 쓴 표기(`"60m"`)를 **그대로** 싣는다. 예전엔
    ///   Duration 만 넘기고 상위가 표기를 역산해(`60m` → `1h`) 봉투 속성과 통지 문구가 어긋났다.
    pub reply_by_raw: String,
}

/// 메시지 장부 — 이력 링버퍼 + request 추적. 순수(주입 시계).
#[derive(Debug)]
pub struct Ledger {
    /// 이력 링버퍼(오래된 순, front = 가장 오래됨). 용량 초과 시 front evict.
    history: VecDeque<MessageRecord>,
    /// 오픈/닫힘 request 추적. 이력과 별도 컬렉션이고, **끝난 항목만** 이력 evict 에 결박된다(record 참조).
    ///   이력이 먼저 사라진 채 끝난 항목은 그 순간 정리된다(`purge_finished_without_history` — fix 1).
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
    /// ★evict = front(오래된 것) + **끝난 계약만** 동반 정리(C3 리뷰 fix 3 · load-bearing)★: 링버퍼라 용량을
    ///   넘기면 가장 오래된 이력부터 버린다. 이때 같은 msg_id 의 request 추적 항목은 **닫혔거나 이미
    ///   통지된 것만** 함께 드롭한다(dangling 정리 + 유계). ★살아 있는 계약(미회신·미통지)은 evict 를
    ///   견딘다★ — 예전엔 무조건 드롭해서, 이력이 밀려난 오픈 request 가 (a) 회신이 와도 닫히지 않고
    ///   (`NoMatch`) (b) 기한이 지나도 통지가 안 나가는 **조용한 계약 소멸**을 겪었다. 계약의 정본은 추적이지
    ///   이력이 아니므로(ReplyOutcome 주석), 이력 용량이 계약을 죽이면 안 된다. 살아 있는 계약의 유계는
    ///   `MAX_OPEN_REQUESTS`(open_request 의 `Full`)가 따로 준다.
    /// ★그 evict 를 견딘 계약이 나중에 끝나면(fix 1)★ 정리해 줄 evict 이벤트가 이미 지나갔으므로 닫힘·
    ///   통지 시점에 즉시 지운다 — 여기 evict 경로의 정리는 그 짝(belt)이다(`purge_finished_without_history`).
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
        // 용량 초과 — 가장 오래된(front) 것부터 evict(오래된 순 유지). evict 뒤 **끝난 계약 중 이력이 사라진
        // 것**을 정리한다(`purge_finished_without_history`) — 살아 있는 계약은 남겨야 회신·통지 경로가
        // 유지된다(위 주석 fix 3). 아직 안 끝난 것들의 상한은 MAX_OPEN_REQUESTS 가 준다.
        let mut evicted_any = false;
        while self.history.len() > self.capacity {
            if self.history.pop_front().is_some() {
                evicted_any = true;
            }
        }
        if evicted_any {
            self.purge_finished_without_history();
        }
    }

    /// ★끝난 계약 중 **가리킬 이력이 없는** 추적 항목을 제거한다(round-final fix 1 · load-bearing)★.
    ///
    /// ★막는 것 = 좀비 추적 항목★: 끝난(closed/notified) 항목의 정상 정리 계기는 "같은 msg_id 이력이
    ///   evict 될 때"(`record`)다. 그런데 fix 3 로 **살아 있는 계약이 evict 를 견디게** 된 순간, 이력이 먼저
    ///   밀려난 계약은 나중에 닫히거나 통지될 때 **정리해 줄 evict 이벤트가 이미 지나간 상태**가 된다. 그
    ///   항목은 `is_live()` 가 false 라 `MAX_OPEN_REQUESTS` 계수에도 안 잡히므로 **어떤 상한도 안 걸린
    ///   채** 영원히 쌓인다(인메모리 v1 의 유계 보장 붕괴). 그래서 끝나는 그 순간(닫힘·통지)과 evict 때,
    ///   이력 유무를 보고 고아를 즉시 지운다.
    /// ★살아 있는 계약은 절대 안 건드린다★: 미회신·미통지 계약은 이력이 없어도 남아야 회신으로 닫히고
    ///   기한 초과 통지가 나간다(fix 3 의 조용한 계약 소멸 방지).
    /// ★비용★: 끝난 항목이 하나도 없으면(대부분) 선형 스캔 한 번으로 즉시 반환하고, 있을 때만 이력
    ///   msg_id 집합(≤ capacity)을 만들어 O(이력 + 추적)으로 판정한다.
    fn purge_finished_without_history(&mut self) {
        if self.requests.iter().all(|r| r.is_live()) {
            return;
        }
        let live_ids: HashSet<&str> = self.history.iter().map(|r| r.msg_id.as_str()).collect();
        self.requests
            .retain(|r| r.is_live() || live_ids.contains(r.request_id.as_str()));
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
    /// ★cap 도달 = `Full`(C3 리뷰 fix 3)★: 살아 있는(미회신·미통지) 계약이 `MAX_OPEN_REQUESTS` 개면 새
    ///   계약을 열지 않는다. 오래된 계약을 밀어내지 않는 이유: 이미 발신자가 기다리는 계약을 조용히 없애는
    ///   건 유실이고, 새 발송을 반려하면 발신자가 즉시 알고 조정할 수 있다(가시적 실패 > 조용한 유실).
    /// ★인자 `reply_by` = (기한, 표기 원본)★: 표기는 통지 문구에 그대로 쓰인다(`DueTimeout.reply_by_raw`).
    ///   튜플로 묶어 "기한이 있으면 표기도 있다" 를 타입으로 강제한다(둘이 어긋날 여지 자체를 없앤다).
    pub fn open_request(
        &mut self,
        request_id: &str,
        sender: &str,
        sender_id: AgentId,
        recipient: &str,
        reply_by: Option<(Duration, String)>,
        now: Instant,
    ) -> OpenOutcome {
        // 같은 id 가 추적에 하나라도 있으면(open/closed 무관) 거부 — id 는 데몬 생성 유일값(재사용 non-scenario).
        if self.requests.iter().any(|r| r.request_id == request_id) {
            return OpenOutcome::DuplicateId;
        }
        // 살아 있는 계약만 센다 — 끝난 항목은 자기 이력이 밀려날 때 정리되므로 상한의 대상이 아니다.
        if self.requests.iter().filter(|r| r.is_live()).count() >= MAX_OPEN_REQUESTS {
            return OpenOutcome::Full;
        }
        self.requests.push(RequestEntry {
            request_id: request_id.to_string(),
            sender: sender.to_string(),
            sender_id,
            recipient: recipient.to_string(),
            reply_by,
            created_at: now,
            closed: false,
            notified: false,
        });
        OpenOutcome::Opened
    }

    /// 이 `msg_id` 가 장부에서 **이미 쓰이고 있나** — 이력 레코드(그룹 방송 포함) 또는 request 추적
    /// (open/closed 무관) 어느 쪽에든 있으면 true.
    ///
    /// ★왜 모든 발송이 이걸 보나(C3 리뷰 fix 12 · load-bearing)★: 예전엔 id 충돌을 **request 발송만**
    ///   잡았다(`open_request` 의 DuplicateId). 그런데 id 는 이력 레코드의 상관 키이자 회신 매칭 키라,
    ///   통보/회신이 기존 id 와 겹치면 (a) `records_for`·`transition` 이 남의 레코드를 집고 (b) 관측 레코드가
    ///   두 메시지를 한 id 로 뭉갠다 — request 가 아니어도 똑같이 해롭다. 그래서 예약 지점에서 종류 무관
    ///   같은 검사를 한다.
    /// ★비용(선택 근거)★: 링버퍼 선형 스캔(≤ HISTORY_CAPACITY) + 추적 선형 스캔이다. 별도 id 집합을 두면
    ///   evict/닫기/제거마다 두 자료구조를 동기화해야 하는데(불일치 = 조용한 오탐), 메시지율이 사람 대화
    ///   수준이라 스캔 비용이 무의미하다 — 단순함을 택했다(v2 영속화 때 인덱스와 함께 재검토).
    pub fn msg_id_in_use(&self, msg_id: &str) -> bool {
        self.history.iter().any(|r| r.msg_id == msg_id)
            || self.requests.iter().any(|r| r.request_id == msg_id)
    }

    /// 이 request 가 **회신으로 닫혔나**(추적에 있고 `closed`). 없는 id 는 false.
    ///
    /// ★용도(C3 리뷰 fix 5 — 타임아웃↔회신 레이스 좁히기)★: `due_timeouts` 로 걷은 뒤 notice 를 파킹하기
    ///   직전에 상위가 다시 확인한다 — 그 사이 회신이 도착해 계약이 닫혔으면 "회신 없음" 통지를 보내지
    ///   않는다. 없는 id 가 false 인 건 의도적이다: evict 등으로 추적이 사라진 경우 타임아웃은 실제로
    ///   발생했으므로 통지를 막을 이유가 없다.
    /// ★잔여(fix 1 과의 상호작용 — 정직한 명시)★: 이력이 이미 evict 된 계약은 **닫히는 순간 추적에서
    ///   제거**되므로(좀비 방지) 그 뒤 이 조회는 false 다 — 즉 "산출 후 회신 도착" 취소가 그 좁은 경우엔
    ///   안 걸리고 통지가 한 번 더 나갈 수 있다. 이력 1024건이 밀려난 뒤 마이크로초 창에서만 성립하는
    ///   경로라, 여기서 유계(좀비 제거)를 택했다.
    pub fn is_request_closed(&self, request_id: &str) -> bool {
        self.requests
            .iter()
            .any(|r| r.request_id == request_id && r.closed)
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
        let outcome = match self.transition(in_reply_to, &recipient, DeliveryStatus::Replied, now) {
            Ok(()) => ReplyOutcome::Closed,
            Err(TransitionError::NotFound) => ReplyOutcome::Closed,
            Err(TransitionError::Illegal { from, .. }) => {
                ReplyOutcome::ClosedHistoryAnomaly { from }
            }
        };
        // 3) 방금 끝난 계약의 이력이 이미 evict 됐다면 그 항목은 **정리해 줄 evict 이벤트가 영영 없다** —
        //    여기서 지운다(좀비 방지, `purge_finished_without_history` 주석). 이력이 남아 있으면 그대로 두고
        //    그 이력이 밀려날 때 함께 정리된다(닫힌 id 재오픈 차단이 그동안 유지된다).
        self.purge_finished_without_history();
        outcome
    }

    /// ★오픈된 request 추적을 **통째로 제거**한다(C3 — 발송이 반려돼 계약이 애초에 성립하지 않은 경우)★.
    /// 제거했으면 `Removed { notified }`(그 항목이 이미 통지된 상태였는지 동봉), 그런 id 가 없으면 `NotFound`.
    ///
    /// ★왜 `close_on_reply` 가 아니라 별도 출구인가(load-bearing — 유계 보장)★: 닫기(`closed=true`)는
    ///   "회신이 와서 계약이 이행됐다" 는 **이력**이라 추적 목록에 남는다. 그 잔존 항목은 같은 msg_id 의
    ///   **이력 레코드가 evict 될 때** 함께 드롭돼 유계가 유지된다(`record` 주석). 그런데 **반려된 발송**은
    ///   이력 레코드가 애초에 없다(park 조차 안 됐다) — 그래서 닫기만 하면 그 항목을 evict 할 계기가 영영
    ///   없어 반려가 반복될수록 추적 목록이 무계 증식한다. 반려는 "계약이 이행됨" 이 아니라 "계약이 성립한
    ///   적 없음" 이므로, 이력을 남기지 않고 흔적째 지우는 게 의미상으로도 맞다.
    /// ★멱등★: 없는 id 면 아무 것도 하지 않는다(`NotFound`).
    /// ★notified 동봉(C3 리뷰 fix 5)★: 제거 시점에 이미 타임아웃 통지가 나갔던 항목이면 그 사실을 함께
    ///   돌려준다 — 호출자가 "통지도 갔는데 반려도 됐다" 는 이중 결말을 로그로 남긴다(`DropOutcome` 주석).
    // ADR-0103
    pub fn drop_request(&mut self, request_id: &str) -> DropOutcome {
        let Some(idx) = self
            .requests
            .iter()
            .position(|r| r.request_id == request_id)
        else {
            return DropOutcome::NotFound;
        };
        let removed = self.requests.remove(idx);
        DropOutcome::Removed {
            notified: removed.notified,
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
            let Some((reply_by, reply_by_raw)) = r.reply_by.clone() else {
                continue; // 기한 없는 request 는 타임아웃 없음.
            };
            let deadline = r.created_at + reply_by;
            if now > deadline {
                r.notified = true; // 이중 통지 방지 — 반환 시점에 마킹.
                due.push(DueTimeout {
                    request_id: r.request_id.clone(),
                    sender: r.sender.clone(),
                    sender_id: r.sender_id,
                    recipient: r.recipient.clone(),
                    // 표기는 발신자가 쓴 원본 그대로 — 통지 문구가 봉투 `reply-by` 와 어긋나지 않게(fix 6).
                    reply_by_raw,
                });
            }
        }
        // 통지로 끝난 계약 중 이력이 이미 evict 된 것은 정리 계기가 영영 없다 — 그 자리에서 지운다(좀비
        //   방지, `purge_finished_without_history` 주석). due 가 빈 대부분의 sweep 은 스캔조차 안 한다.
        if !due.is_empty() {
            self.purge_finished_without_history();
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

    /// 전 이력 레코드(오래된 순) — 관측/테스트 스냅샷. 상위(MessagingService)가 "notice 가 장부에 남았나"
    /// 처럼 msg_id 를 모르는 단언을 할 때 쓴다(msg_id 를 아는 조회는 `records_for`).
    pub fn all_records(&self) -> Vec<&MessageRecord> {
        self.history.iter().collect()
    }

    /// 오픈(미회신) request 수(관측/테스트). closed 제외.
    pub fn open_request_count(&self) -> usize {
        self.requests.iter().filter(|r| !r.closed).count()
    }

    /// 추적 항목 **총수**(끝난 것 포함 — 관측/테스트). 좀비 누적(fix 1)이 없는지 유계를 단언하는 데 쓴다:
    ///   `open_request_count` 는 끝난 항목을 안 세므로 누수를 못 본다.
    pub fn tracking_len(&self) -> usize {
        self.requests.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> Instant {
        Instant::now()
    }

    /// 발신자 AgentId(fix 2) — 대부분의 단언은 값 자체를 안 보므로 매번 새로 뽑는다.
    fn sid() -> AgentId {
        AgentId::new_v4()
    }

    /// 기한 튜플(fix 6) — 표기는 Duration 에서 만든 게 아니라 **발신자가 쓴 것**이라는 전제를 테스트에서도
    /// 유지하려고, 단언이 표기를 안 보는 자리에선 관례적 표기 하나를 쓴다.
    fn rb(d: Duration) -> Option<(Duration, String)> {
        Some((d, format!("{}s", d.as_secs())))
    }

    /// 운영 경로 재현 — 접수된 발송은 **반드시** 이력 레코드를 남기고(park/inject 둘 다 record 한다) 계약을
    /// 연다. 이력 없는 계약은 evict 이후에만 존재하므로(fix 1), 그 케이스를 노리지 않는 테스트는 이 헬퍼로
    /// 이력을 함께 만든다 — 그래야 "닫힌 계약이 추적에 남는다" 같은 단언이 운영 상태를 반영한다.
    fn open_delivered_request(
        l: &mut Ledger,
        id: &str,
        reply_by: Option<(Duration, String)>,
        now: Instant,
    ) {
        l.record(id, "alice", "bob", "q", DeliveryStatus::Pending, now);
        l.open_request(id, "alice", sid(), "bob", reply_by, now);
        assert_eq!(
            l.transition(id, "bob", DeliveryStatus::Delivered, now),
            Ok(()),
            "전제: 주입까지 끝난 계약"
        );
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
            l.open_request("req-1", "alice", sid(), "bob", None, now),
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
        l.open_request("req-1", "alice", sid(), "bob", None, now);
        // 틀린 id 회신 = NoMatch, 아무 것도 안 닫음(엄격 매칭 — 우연 닫힘 오발 거부).
        assert_eq!(l.close_on_reply("req-999", now), ReplyOutcome::NoMatch);
        assert_eq!(l.open_request_count(), 1, "틀린 id 는 request 를 안 닫아야");
    }

    #[test]
    fn second_reply_to_same_request_is_already_closed_noop() {
        let mut l = Ledger::new();
        let now = t0();
        // 이력이 남아 있는 정상 계약 — 닫힌 항목이 추적에 잔존해야 두 번째 회신을 AlreadyClosed 로 구분한다.
        open_delivered_request(&mut l, "req-1", None, now);
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
            l.open_request("req-1", "alice", sid(), "bob", None, now),
            OpenOutcome::Opened
        );
        assert_eq!(
            l.open_request("req-1", "alice", sid(), "carol", None, now),
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
        // 이력이 남아 있는 정상 계약(운영 경로) — 닫힌 항목이 추적에 남아 재오픈을 막는다.
        open_delivered_request(&mut l, "req-1", None, now);
        assert_eq!(l.close_on_reply("req-1", now), ReplyOutcome::Closed);
        // 닫힌 뒤 같은 id 재오픈 시도 → 거부(추적에 여전히 존재).
        assert_eq!(
            l.open_request("req-1", "alice", sid(), "bob", None, now),
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
        l.open_request("req-1", "alice", sid(), "bob", None, now);
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
        l.open_request("req-1", "alice", sid(), "bob", None, now);
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
        l.open_request("req-1", "alice", sid(), "bob", None, now);
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
        l.open_request("req-1", "alice", sid(), "bob", rb(reply_by), now);
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
    fn due_timeout_carries_sender_id_and_raw_notation() {
        // ★fix 2/6★: 보고는 발신자 **id**(개명 대비 배달 힌트)와 **표기 원본**(통지 문구용)을 함께 싣는다.
        //   특히 표기는 정규화하지 않는다 — `60m` 는 `60m` 그대로여야 봉투 reply-by 와 문구가 일치한다.
        let mut l = Ledger::new();
        let now = t0();
        let sender = sid();
        let reply_by = Duration::from_secs(3600);
        l.open_request(
            "req-1",
            "alice",
            sender,
            "bob",
            Some((reply_by, "60m".to_string())),
            now,
        );
        let due = l.due_timeouts(now + reply_by + Duration::from_secs(1));
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].sender_id, sender, "발신자 id 동봉(개명 대비 힌트)");
        assert_eq!(
            due[0].reply_by_raw, "60m",
            "표기 원본 그대로(1h 로 정규화 금지)"
        );
    }

    #[test]
    fn due_timeout_no_double_notification() {
        let mut l = Ledger::new();
        let now = t0();
        let reply_by = Duration::from_secs(600);
        // 이력이 남아 있는 정상 계약 — 재산출을 막는 게 `notified` 플래그임을 단언한다(항목 제거가 아니라).
        open_delivered_request(&mut l, "req-1", rb(reply_by), now);
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
        l.open_request("req-1", "alice", sid(), "bob", rb(reply_by), now);
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
    fn drop_request_removes_the_entry_entirely_unlike_close() {
        // ★C3 반려 회수★: 닫기(close)는 이력으로 **남고**(같은 id 재오픈 불가), 제거(drop)는 흔적째 지워
        //   같은 id 를 다시 열 수 있다. 반려된 발송은 이력 레코드가 없어 evict 계기가 없으므로 제거해야
        //   무계 증식을 막는다(drop_request 주석).
        let mut l = Ledger::new();
        let now = t0();
        let reply_by = Duration::from_secs(600);

        // 닫기: 재오픈 불가(DuplicateId) + due 대상 아님(이력이 남아 있는 정상 계약).
        open_delivered_request(&mut l, "closed-1", rb(reply_by), now);
        assert_eq!(l.close_on_reply("closed-1", now), ReplyOutcome::Closed);
        assert_eq!(
            l.open_request("closed-1", "alice", sid(), "bob", rb(reply_by), now),
            OpenOutcome::DuplicateId,
            "닫힌 항목은 추적에 남아 재오픈을 막는다"
        );

        // 제거: 흔적이 없으니 같은 id 재오픈 가능.
        l.open_request("dropped-1", "alice", sid(), "bob", rb(reply_by), now);
        assert_eq!(
            l.drop_request("dropped-1"),
            DropOutcome::Removed { notified: false },
            "제거 성공 — 통지 전이었으므로 notified=false"
        );
        assert_eq!(
            l.drop_request("dropped-1"),
            DropOutcome::NotFound,
            "멱등 — 두 번째는 NotFound"
        );
        assert_eq!(
            l.open_request("dropped-1", "alice", sid(), "bob", rb(reply_by), now),
            OpenOutcome::Opened,
            "제거된 id 는 다시 열 수 있다(계약 미성립 = 흔적 없음)"
        );
        // 제거됐던 계약은 그 사이 due 로도 안 나왔어야 한다(지금 다시 연 것만 유효).
        assert_eq!(l.open_request_count(), 1, "열린 계약은 방금 것 하나뿐");
    }

    #[test]
    fn request_without_reply_by_never_times_out() {
        let mut l = Ledger::new();
        let now = t0();
        l.open_request("req-1", "alice", sid(), "bob", None, now);
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

    // ── evict ↔ request 추적 결합(finding 6 · C3 리뷰 fix 3 로 재정의) ────────────
    #[test]
    fn eviction_drops_only_finished_request_tracking() {
        // 이력 evict 는 **끝난 계약**(closed/notified)의 추적만 정리한다 — dangling 방지·유계는 유지하되
        //   살아 있는 계약은 건드리지 않는다(아래 별도 테스트).
        let cap = 2;
        let mut l = Ledger::with_capacity(cap);
        let now = t0();
        // 끝난 계약 하나(회신으로 닫힘) + 이후 다른 메시지로 그 이력을 밀어낸다.
        l.record("done", "alice", "bob", "q", DeliveryStatus::Pending, now);
        l.open_request("done", "alice", sid(), "bob", None, now);
        assert!(matches!(
            l.close_on_reply("done", now),
            ReplyOutcome::ClosedHistoryAnomaly { .. } | ReplyOutcome::Closed
        ));
        for i in 0..cap {
            l.record(
                &format!("x{i}"),
                "alice",
                "bob",
                "q",
                DeliveryStatus::Delivered,
                now,
            );
        }
        assert_eq!(l.history_len(), cap, "이력은 용량 유계");
        assert!(
            l.records_for("done").is_empty(),
            "전제: done 이력은 evict 됐다"
        );
        assert!(
            !l.msg_id_in_use("done"),
            "끝난 계약의 추적은 이력 evict 와 함께 드롭(유계 유지)"
        );
    }

    #[test]
    fn eviction_keeps_live_contract_so_reply_and_timeout_still_work() {
        // ★fix 3 회귀★: 이력이 밀려나도 **미회신·미통지** 계약은 살아남아야 한다 — 안 그러면 회신이 와도
        //   NoMatch 로 튕기고 기한이 지나도 통지가 안 나가는 조용한 계약 소멸이 된다.
        let cap = 2;
        let mut l = Ledger::with_capacity(cap);
        let now = t0();
        let reply_by = Duration::from_secs(600);
        l.record("req-1", "alice", "bob", "q", DeliveryStatus::Pending, now);
        l.open_request("req-1", "alice", sid(), "bob", rb(reply_by), now);
        // 뒤이은 메시지들이 req-1 이력을 밀어낸다.
        for i in 0..cap {
            l.record(
                &format!("x{i}"),
                "alice",
                "bob",
                "q",
                DeliveryStatus::Delivered,
                now,
            );
        }
        assert!(l.records_for("req-1").is_empty(), "전제: 이력은 evict 됐다");
        assert_eq!(
            l.open_request_count(),
            1,
            "살아 있는 계약은 evict 를 견딘다"
        );
        // ① 기한 초과 통지가 여전히 나간다.
        let due = l.due_timeouts(now + reply_by + Duration::from_secs(1));
        assert_eq!(due.len(), 1, "evict 됐어도 타임아웃 통지는 살아 있다");
        assert_eq!(due[0].request_id, "req-1");

        // ② 회신도 여전히 계약을 닫는다(다른 장부로 같은 조건 재현 — 위에서 이미 통지된 항목과 섞지 않게).
        let mut l2 = Ledger::with_capacity(cap);
        l2.record("req-2", "alice", "bob", "q", DeliveryStatus::Pending, now);
        l2.open_request("req-2", "alice", sid(), "bob", rb(reply_by), now);
        for i in 0..cap {
            l2.record(
                &format!("y{i}"),
                "alice",
                "bob",
                "q",
                DeliveryStatus::Delivered,
                now,
            );
        }
        assert_eq!(
            l2.close_on_reply("req-2", now),
            ReplyOutcome::Closed,
            "이력이 evict 됐어도 회신은 계약을 닫는다(가리킬 이력만 없음)"
        );
        assert_eq!(l2.open_request_count(), 0);
    }

    /// fix 1 전용 셋업 — 이력이 **먼저 evict 된 살아 있는 계약** 하나만 남은 장부(좀비의 출발 조건).
    fn ledger_with_evicted_live_contract(
        cap: usize,
        id: &str,
        reply_by: Option<(Duration, String)>,
        now: Instant,
    ) -> Ledger {
        let mut l = Ledger::with_capacity(cap);
        l.record(id, "alice", "bob", "q", DeliveryStatus::Pending, now);
        l.open_request(id, "alice", sid(), "bob", reply_by, now);
        for i in 0..cap {
            l.record(
                &format!("filler{i}"),
                "alice",
                "bob",
                "x",
                DeliveryStatus::Delivered,
                now,
            );
        }
        assert!(l.records_for(id).is_empty(), "전제: 그 계약의 이력은 evict");
        assert_eq!(
            l.tracking_len(),
            1,
            "전제: 살아 있는 계약은 evict 를 견딘다"
        );
        l
    }

    #[test]
    fn close_after_history_eviction_removes_the_finished_tracking_entry() {
        // ★fix 1(좀비 방지)★: 이력이 먼저 밀려난 계약은 살아 있는 동안 evict 를 견딘다(fix 3). 그런데 그
        //   계약이 **나중에 회신으로 닫히면** 정리해 줄 evict 이벤트는 이미 지나갔다 — 예전엔 그 항목이
        //   영원히 남았고(live 계수에서도 빠져 cap 이 못 잡는다) 반복되면 추적이 무계 증식했다.
        let now = t0();
        let mut l = ledger_with_evicted_live_contract(2, "req-1", None, now);
        assert_eq!(l.close_on_reply("req-1", now), ReplyOutcome::Closed);
        assert_eq!(
            l.tracking_len(),
            0,
            "닫히는 순간 고아 추적 항목을 제거(좀비 없음)"
        );
        assert!(!l.msg_id_in_use("req-1"), "추적에도 이력에도 없다");
    }

    #[test]
    fn timeout_notice_after_history_eviction_removes_the_finished_tracking_entry() {
        // ★fix 1 의 다른 종점★: 통지(notified)로 끝나는 경우도 같다 — 보고는 정상적으로 나가되(계약 이행
        //   경로 유지) 그 항목은 그 자리에서 정리된다.
        let now = t0();
        let reply_by = Duration::from_secs(600);
        let mut l = ledger_with_evicted_live_contract(2, "req-1", rb(reply_by), now);
        let due = l.due_timeouts(now + reply_by + Duration::from_secs(1));
        assert_eq!(due.len(), 1, "evict 됐어도 통지는 나간다(fix 3 유지)");
        assert_eq!(due[0].request_id, "req-1");
        assert_eq!(
            l.tracking_len(),
            0,
            "통지로 끝난 고아 항목도 그 자리에서 제거"
        );
    }

    #[test]
    fn tracking_stays_bounded_when_evicted_contracts_finish() {
        // ★fix 1 의 유계 단언★: 이력 용량(2)의 수십 배 계약을 열고 매번 이력을 밀어낸 뒤 끝내도 추적은
        //   0 으로 수렴한다. `open_request_count` 는 끝난 항목을 안 세므로 `tracking_len`(총수)으로 본다.
        let cap = 2;
        let mut l = Ledger::with_capacity(cap);
        let now = t0();
        let reply_by = Duration::from_secs(60);
        let over = now + reply_by + Duration::from_secs(1);
        for i in 0..50 {
            let id = format!("r{i}");
            l.record(&id, "alice", "bob", "q", DeliveryStatus::Pending, now);
            l.open_request(&id, "alice", sid(), "bob", rb(reply_by), now);
            // 이 계약의 이력을 곧바로 밀어낸다(cap 개 filler) → 고아 상태의 살아 있는 계약.
            for j in 0..cap {
                l.record(
                    &format!("f{i}-{j}"),
                    "alice",
                    "bob",
                    "x",
                    DeliveryStatus::Delivered,
                    now,
                );
            }
            assert!(l.records_for(&id).is_empty(), "전제: 이력 evict");
            // 절반은 회신으로, 절반은 기한 초과 통지로 끝난다 — 어느 종점이든 남으면 안 된다.
            if i % 2 == 0 {
                assert_eq!(l.close_on_reply(&id, now), ReplyOutcome::Closed);
            } else {
                assert_eq!(l.due_timeouts(over).len(), 1, "이 라운드의 계약만 due");
            }
            assert_eq!(
                l.tracking_len(),
                0,
                "라운드마다 추적이 0 으로 수렴(좀비 누적 없음)"
            );
        }
    }

    #[test]
    fn open_request_rejects_at_capacity_with_full() {
        // ★fix 3 의 짝★: 살아 있는 계약이 cap 이면 새 계약은 Full 로 반려(조용한 밀어내기 금지).
        let mut l = Ledger::new();
        let now = t0();
        for i in 0..MAX_OPEN_REQUESTS {
            assert_eq!(
                l.open_request(&format!("r{i}"), "alice", sid(), "bob", None, now),
                OpenOutcome::Opened
            );
        }
        assert_eq!(
            l.open_request("over", "alice", sid(), "bob", None, now),
            OpenOutcome::Full,
            "cap 도달 시 새 계약은 Full"
        );
        assert_eq!(l.open_request_count(), MAX_OPEN_REQUESTS, "기존 계약 불변");
        // 하나가 끝나면(회신) 자리가 난다 — 계수는 **살아 있는 것**만 세기 때문.
        assert_eq!(l.close_on_reply("r0", now), ReplyOutcome::Closed);
        assert_eq!(
            l.open_request("over", "alice", sid(), "bob", None, now),
            OpenOutcome::Opened,
            "끝난 계약은 cap 계수에서 빠진다"
        );
    }

    #[test]
    fn msg_id_in_use_sees_history_and_tracking() {
        // ★fix 12★: 충돌 검사는 이력·추적 **양쪽**을 본다(통보/회신 id 도 남의 레코드를 앨리어싱하면 안 됨).
        let mut l = Ledger::new();
        let now = t0();
        assert!(!l.msg_id_in_use("m1"), "미사용 id");
        l.record("m1", "a", "b", "x", DeliveryStatus::Delivered, now);
        assert!(l.msg_id_in_use("m1"), "이력에 있으면 사용 중");
        // 이력 없이 추적만 있는 경우(반려 전 예약 등)도 사용 중이다.
        l.open_request("r1", "a", sid(), "b", None, now);
        assert!(l.msg_id_in_use("r1"), "추적에만 있어도 사용 중");
        // 닫힌 계약도 여전히 사용 중(재사용 금지 — 회신 매칭 키 유일성). 이력이 남아 있는 정상 계약 기준:
        //   이력이 이미 evict 된 계약은 닫히는 순간 정리되므로(fix 1) 그 케이스는 별도 테스트가 본다.
        open_delivered_request(&mut l, "r2", None, now);
        assert_eq!(l.close_on_reply("r2", now), ReplyOutcome::Closed);
        assert!(
            l.msg_id_in_use("r2"),
            "닫혀도 이력·추적에 남아 있으면 사용 중"
        );
    }

    #[test]
    fn is_request_closed_only_true_for_closed_entries() {
        // ★fix 5★: 통지 직전 재확인용 — 열려 있으면 false, 회신으로 닫히면 true, 없는 id 는 false.
        let mut l = Ledger::new();
        let now = t0();
        // 이력이 남아 있는 정상 계약 — 닫힌 항목이 추적에 잔존해야 이 조회가 통지를 취소할 수 있다.
        open_delivered_request(&mut l, "r1", None, now);
        assert!(!l.is_request_closed("r1"), "열린 계약은 false");
        assert!(
            !l.is_request_closed("nope"),
            "없는 id 는 false(통지 막지 않음)"
        );
        assert_eq!(l.close_on_reply("r1", now), ReplyOutcome::Closed);
        assert!(l.is_request_closed("r1"), "회신으로 닫히면 true");
    }

    #[test]
    fn drop_request_reports_already_notified_entry() {
        // ★fix 5★: 통지가 이미 나간 계약을 회수하면 그 사실을 알린다(통지는 되돌릴 수 없다 — 이중 결말 관측).
        let mut l = Ledger::new();
        let now = t0();
        let reply_by = Duration::from_secs(600);
        // 이력이 남아 있는 정상 계약 — 통지 뒤에도 항목이 남아 있어야 회수가 그 사실을 보고할 수 있다.
        open_delivered_request(&mut l, "r1", rb(reply_by), now);
        assert_eq!(
            l.due_timeouts(now + reply_by + Duration::from_secs(1))
                .len(),
            1
        );
        assert_eq!(
            l.drop_request("r1"),
            DropOutcome::Removed { notified: true },
            "이미 통지된 계약의 회수는 그 사실을 동봉"
        );
    }
}
