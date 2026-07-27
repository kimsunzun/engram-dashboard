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
/// ★4096 (사용자 비준 2026-07-26 — 1024 에서 상향)★: 이 링의 단위는 메시지가 아니라 **배달기록**이다.
///   방송 1건이 `(msg_id, 멤버)` 레코드를 **N개** 쓰므로(spec §4 "1 msg_id : N 배달기록") 10인 그룹에 100번
///   방송하면 1024 는 그것만으로 가득 찼다 — 회전이 너무 빨라 ① 조회 이력이 급격히 짧아지고 ② 만료/회수
///   전이가 `NotFound` 로 떨어져 그 사실이 장부에 안 남았다(그때 유일하게 남는 증거는 `sweep`/`flush_for`
///   의 debug 로그다 — C4 리뷰 fix J). 4배로 올려 그룹 규모의 fan-out 을 흡수한다.
/// ★메모리 프로필(정직한 상한)★: 레코드는 **본문 전체를 그대로 보관한다**(요약·절단 없음). 그래서 최악은
///   `본문 최대 크기 × 용량` = 64KiB(`control::ingress::MAX_BODY_BYTES`) × 4096 ≈ **256MiB** 다 — 사람 대화
///   규모의 본문(수백 바이트~수 KiB)이면 ~1–2MB 에 그치지만, 최대 크기 메시지가 연속으로 들어오는 병적
///   스트림에서는 그보다 훨씬 크다는 뜻이다. 인메모리 단계 한정 값이고 무파괴 변경 가능한 조율 대상이다.
/// ★후속(식별만 — 지금 구현하지 않는다, 사용자 언급 2026-07-26)★: ⓐ 이 값을 런타임 설정/커맨드로 노출
///   ⓑ 감사 목적상 본문을 절단해 저장(전문 보관 대신). 둘 다 별건이며 이 상수 변경의 전제가 아니다.
/// ★evict 와 request 추적의 관계(C3 리뷰 fix 3 로 좁혀짐)★: 이력 evict 는 **끝난 계약**(closed 또는 이미
///   통지된)의 추적 항목만 함께 드롭한다. 살아 있는(미회신·미통지) 계약은 evict 를 견디고 남는다 — 예전엔
///   무조건 드롭해서, 이력이 밀려난 오픈 request 가 **회신으로 닫힐 길과 기한 초과 통지를 동시에 잃었다**
///   (조용한 계약 소멸 = 최악 실패 모드). 유계는 이제 이력 용량이 아니라 `MAX_OPEN_REQUESTS` 가 준다.
const HISTORY_CAPACITY: usize = 4096;

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
    /// ★이 논리 메시지가 남길 **배달기록 총수**(발송 시점에 확정 — round-2 리뷰 F3)★. 단일 발송·notice = 1,
    /// 그룹 fan-out = 멤버 수 N. 같은 `msg_id` 의 모든 행이 **같은 값**을 든다.
    ///
    /// ★왜 필요한가(옛 "front 위치" 증명이 틀렸던 지점)★: 조회는 "남은 행이 전부인가" 를 답해야 하는데,
    ///   그룹 행은 **두 단계**로 기록된다(계획 락에서 parked/skipped, 그 뒤 멤버별 구간에서 delivered).
    ///   그 사이 다른 메시지의 행이 끼어들 수 있어 한 msg_id 의 행이 링에서 **연속이 아니다** — 앞쪽 행이
    ///   evict 되고 뒤쪽 행만 남아도 링 front 는 남의 행일 수 있다. 그래서 "front 가 아니면 완전" 이라는
    ///   위치 기반 증명은 **거짓 음성**을 낸다. 기대 개수를 발송 시점에 박아 두면 `남은 행 수 < 기대` 라는
    ///   **결정적** 비교로 바뀐다(위치·순서에 의존하지 않는다).
    /// ★u16 인 이유★: 한 방송의 멤버 수는 로스터 규모라 65535 로 충분하고, 레코드당 2바이트라 4096개 링에
    ///   8KiB 만 더한다(본문 보관 비용에 비하면 무시 가능).
    // round-2 리뷰 F3
    pub expected_rows: u16,
}

/// 오픈된 request 추적 1건(spec §3). 이력 레코드와 **별도 맵**이라 링 evict 에 영향받지 않는다.
///
/// ★notified 플래그(load-bearing — 이중 통지 방지)★: `reply_by` 초과가 `due_timeouts` 로 한 번 보고되면
///   이 플래그를 세워 **다시 보고하지 않는다**(spec §7 "no double-notification"). 회신이 오면 `closed` 라
///   `due_timeouts` 대상에서 빠진다(replied 는 절대 due 로 안 나옴).
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// ★해석된 수신자 AgentId(D 리뷰 B1 — load-bearing)★: 발송 시점에 수신자가 **산 에이전트로 해석됐으면**
    /// 그 AgentId. 부재 파킹(아직 안 뜬 이름·죽은 이름)이면 `None`.
    ///
    /// ★왜 이름만으로는 안 되나★: 같은 이름의 산 에이전트가 둘일 때(동명 다수) 발신자는 exact AgentId 로
    ///   지목해 한쪽에만 request 를 보낼 수 있는데, 계약은 이름(`recipient`)으로만 기록됐다 — 그러면
    ///   **메시지를 받은 적도 없는 쌍둥이**가 미결 조회에서 그 의무를 자기 것으로 본다(잘못된 의무 귀속).
    ///   id 를 함께 붙들면 "누가 답해야 하나" 를 정확히 가를 수 있다.
    /// ★epoch 는 담지 않는다★: 같은 에이전트의 재시작은 AgentId 를 유지하고 epoch 만 올린다(ADR-0007) —
    ///   재시작한 그 에이전트는 여전히 답할 주체이므로 epoch 로 좁히면 의무가 부당하게 사라진다.
    /// ★None 의 의미 = 이름 폴백★: 아직 뜨지 않은 이름 앞으로 건 request 는 나중에 그 이름으로 등장한
    ///   에이전트가 답할 주체다(WYSIWYA — ADR-0101). 그래서 id 가 없으면 이름으로만 매칭한다.
    // 리뷰 B1
    recipient_id: Option<AgentId>,
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
    /// ★상한 압력으로 **은퇴 예정 표시**됨(round-5 mark-and-sweep)★ — 아직 목록에 살아 있고 회신도 받을 수
    /// 있다. 커밋 때 비로소 물리 제거되고, 롤백이면 표시만 지워져 아무 일도 없던 상태로 돌아간다.
    ///
    /// ★왜 표시인가(물리 제거를 버린 이유 — load-bearing)★: 예전 설계는 예약 시점에 희생자를 목록에서
    ///   **꺼냈다**. 그 창 동안 그 계약은 세상에 없는 것처럼 굴어서 ① 정당한 회신이 `close_on_reply` 에서
    ///   `NoMatch` 로 빗나가고 ② 롤백이 "열린 채" 되돌려 유령 상태·헛 통지를 만들었다. 꺼내지 않고 표시만
    ///   하면 그 창 자체가 없다 — 회신·조회·중복검사 전부 평소 경로로 계속 동작한다.
    /// ★상한 계수에서만 빠진다★: `occupies_slot` 참조(곧 비워질 자리로 계산해 새 계약을 받아들인다).
    // round-5 mark-and-sweep
    pending_retirement: bool,
    /// ★아직 접수 확정되지 않은 신규 계약(round-5 mark-and-sweep)★ — 발송이 dispatch 를 통과하면 커밋에서
    /// 이 표시가 지워지고, 반려·패닉이면 롤백이 이 항목을 제거한다.
    ///
    /// ★왜 필요한가★: 이 항목은 **슬롯을 차지하지만 아직 남의 자리를 뺏을 자격은 없다**. 표시가 없으면
    ///   동시에 들어온 다른 발송이 이 미확정 계약을 "가장 오래된 은퇴 가능 계약" 으로 골라 없애 버릴 수
    ///   있고, 그러면 원 발송은 **배달에 성공했는데 계약이 없는** 상태가 된다(그 request 에 온 회신이
    ///   전부 `NoMatch`). 그래서 희생자 선정에서 명시적으로 제외한다.
    // round-5 mark-and-sweep
    provisional: bool,
}

impl RequestEntry {
    /// 아직 **살아 있는**(= 미회신) 계약인가. 이 부류만 이력 evict 를 견디고(`record`),
    /// `MAX_OPEN_REQUESTS` cap 의 계수 대상이다.
    ///
    /// ★기준 = `!closed` 단독(D 리뷰 B3 — 옛 `!closed && !notified` 에서 교정)★: 예전엔 기한 초과 통지가
    ///   나간 계약(`notified`)을 "끝난 것" 으로 취급해 이력 evict 때 함께 지웠다. 그런데 통지는 **발신자에게
    ///   알렸다**는 사실일 뿐 회신이 온 게 아니다 — 수신자는 여전히 답할 의무가 있고(spec §3 "늦어도
    ///   회신하라"), D 의 미결 조회는 그런 계약을 `timed_out=true` 로 **계속 보여주기로** 계약했다
    ///   (`open_requests` 는 `!closed` 로 거른다). 두 기준이 갈리면 실제로 이런 순서에서 의무가 증발했다:
    ///   ① request 오픈 → ② 4096건이 밀려 그 이력 행이 evict → ③ 기한 초과로 `notified=true` →
    ///   ④ `purge_finished_without_history` 가 "끝났고 이력도 없다" 며 계약 삭제 → 미결 목록에서 소멸.
    ///   발신자·수신자 양쪽이 "끝난 일" 로 오독하는 조용한 유실이라, 정의를 미결 조회 쪽에 맞춘다.
    /// ★유계는 유지된다★: 이제 통지된 미회신 계약도 cap 에 잡히므로 추적 목록은 `MAX_OPEN_REQUESTS`(512)로
    ///   묶인다 — 한도에 닿으면 새 request 가 `REQUEST_CAPACITY` 로 반려된다(조용한 유실 대신 가시적 실패).
    // 리뷰 B3
    fn is_live(&self) -> bool {
        !self.closed
    }

    /// ★`MAX_OPEN_REQUESTS` 슬롯을 차지하는가(round-5 mark-and-sweep · round-6 I1)★ — 상한 판정의 유일한 기준.
    ///
    /// - **은퇴 표시된 계약은 세지 않는다**: 커밋 때 빠질 자리라 이미 새 계약의 몫이다. 세면 표시 직후에도
    ///   여전히 cap 이라 새 계약을 못 받아 은퇴 자체가 무의미해진다.
    /// - **잠정 계약은 **센다**(중요)**: 아직 확정 전이어도 실재하는 접수분이고, 안 세면 동시 발송 여러 건이
    ///   모두 "자리 있음" 으로 판정해 상한을 넘겨 들어온다(cap 이 뚫린다).
    /// - ★**닫힌 잠정 계약도 정산 전까지는 계속 센다**(round-6 I1 · load-bearing)★: `!closed` 만 보면
    ///   잠정 구간 도중 회신이 도착하는 순간 그 계약이 **자기 자리를 잃는다**. 실제로 다음 6단계가 상한을
    ///   영구히 뚫었다: ① A 가 V1 을 표시하고 잠정 PA 를 넣는다(512) ② 빠른 회신이 PA 를 닫는다(511) ③ B 가
    ///   "자리 있음" 으로 보고 **아무도 표시하지 않은 채** PB 를 넣는다(512) ④ A 가 롤백해 V1 표시를 풀면
    ///   +1, 닫힌 PA 를 지워도 0 → **513 고착**. 자리는 발송 접수의 대가로 **예약된 것**이라, 그 예약은
    ///   가드가 정산(커밋/롤백)할 때까지 유지돼야 한다 — 회신이 빨랐다는 사실이 남의 자리를 만들어내면 안 된다.
    /// ★정산 시 산술(두 갈래 모두 정확히 맞는다)★:
    ///   - **커밋**: 잠정 표시가 풀린다 → 그 계약이 닫혀 있었다면 그때 자리가 **정당하게** 해제된다
    ///     (회신을 실제로 받은 계약이므로 자리를 놓는 게 맞다).
    ///   - **롤백**: 닫힌 잠정 계약을 제거(-1)하고 희생자 표시를 해제(+1) → 합이 0, 정확히 cap 유지.
    // round-5 mark-and-sweep / round-6 I1
    // ADR-0108 (용량 술어 단일점 — 잠정은 닫혀도 무게 유지)
    fn occupies_slot(&self) -> bool {
        (!self.closed || self.provisional) && !self.pending_retirement
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

/// ★상한 압력으로 은퇴한 계약 1건(round-2 리뷰 F1 · 사용자 결정 2026-07-27)★ — 호출자가 **락 밖에서**
/// 계측 로그를 남길 재료다(조용한 소멸 금지).
///
/// ★언제 생기나★: 미회신 계약이 `MAX_OPEN_REQUESTS` 에 닿았는데 새 request 가 들어왔고, 추적 목록에
///   **은퇴 가능한**(= 발신자에게 남은 통지 약속이 없는) 계약이 있을 때. 그 중 가장 오래된 하나가 자리를
///   내준다 — 메일박스·notice 레인이 cap 에서 "가장 오래된 것을 은퇴" 시키는 것과 같은 패턴이다.
/// ★은퇴 **예정** 표시된 계약의 표시 정보(round-5 mark-and-sweep)★ — 커밋 시점의 계측 로그용이다.
///
/// ★값만 담는다(원본 항목을 들고 다니지 않는다)★: 희생자는 목록에서 나간 적이 없으므로 되돌릴 상태가
///   없다 — 롤백은 그 항목의 표시를 지우기만 하면 된다. 그래서 이 구조체는 "무엇이 은퇴하려 했나" 를
///   사람이 읽을 수 있게 나르는 것 이상을 하지 않는다(옛 설계의 entry/index 운반은 삭제됐다).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetiredContract {
    /// 은퇴 예정으로 표시된 request id.
    pub request_id: String,
    /// 그 계약의 발신자 이름.
    pub sender: String,
    /// 그 계약의 수신자 이름.
    pub recipient: String,
    /// 표시 시점 기준 나이(로그용 — 벽시계가 아니라 경과).
    pub age: Duration,
}

/// request 오픈 결과 — 중복 id 방어(spec §3 · 아래 open_request 주석).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenOutcome {
    /// 새 request 를 열었음.
    Opened,
    /// 새 request 를 (잠정으로) 열되, 상한 압력으로 **가장 오래된 은퇴 가능 계약**에 은퇴 예정 표시를 했음.
    ///
    /// ★호출자 의무(round-5 mark-and-sweep · load-bearing)★: 표시는 **잠정**이다. 호출자는 반드시 둘 중
    ///   하나를 부른다 — ① 발송 접수 시 `commit_open`(표시된 희생자를 물리 제거 + 잠정 표시 해제, 그때
    ///   계측 로그) ② 그 외 모든 이탈(반려·패닉) 시 `rollback_open`(표시 해제 + 잠정 계약 제거).
    ///   `ReservationGuard` 의 Drop 이 ②를 구조적으로 보장한다.
    OpenedAfterMarking(RetiredContract),
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

/// ★미회신 request 1건의 조회 뷰(S18 D — `messages` 무인자 "내 미결")★.
///
/// ★왜 `RequestEntry` 를 직접 노출하지 않나★: 추적 항목은 장부의 **내부 상태**(closed/notified 플래그,
///   sender_id 등 배달 배선용 필드)를 담는다 — 그대로 내보내면 조회 표면이 내부 표현에 유착돼 v2 영속화 때
///   같이 굳는다. 조회에 필요한 사실만 값으로 복사해 넘긴다(순수·읽기 전용).
/// ★`notified` 를 싣는 이유★: 기한이 이미 지나 발신자에게 통지가 나간 계약도 **여전히 미회신**이다(회신이
///   오면 그때 닫힌다). 미결 목록에서 빼면 "답할 게 남았는데 목록엔 없는" 상태가 되므로 포함하되, 통지가
///   나갔다는 사실은 구분할 수 있게 함께 싣는다(상위가 표시 여부를 정한다).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenRequestView {
    /// request 메시지 id(회신의 `in_reply_to` 가 가리킬 값).
    pub request_id: String,
    /// 요청 발신자 이름(발송 시점 표시 이름).
    pub sender: String,
    /// 요청 발신자의 AgentId — 미결 조회가 "내가 건 요청" 을 **이름이 아니라 id 로** 가르는 축(리뷰 B1).
    pub sender_id: AgentId,
    /// 요청 수신자 이름 — 회신 의무를 진 쪽.
    pub recipient: String,
    /// 해석된 수신자 AgentId(부재 파킹이면 None) — 동명 다수에서 의무를 정확히 귀속시키는 축(리뷰 B1).
    pub recipient_id: Option<AgentId>,
    /// 발신자가 쓴 기한 표기 원본(`"10m"`). 기한 없는 request 는 None.
    pub reply_by_raw: Option<String>,
    /// 계약 오픈(발송) 시각 — 상위가 `now` 와 빼서 경과를 만든다(장부는 벽시계를 모른다).
    pub created_at: Instant,
    /// 기한 초과 통지가 이미 나갔나(계약은 여전히 열려 있다 — struct 주석).
    pub notified: bool,
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
    /// ★evict 가 한 번이라도 일어났나(D 리뷰 B2 — 조회 정직성)★. 링이 가득 차 앞쪽 레코드를 버리기
    /// 시작하면 `records_for` 가 돌려주는 행 집합이 **그 메시지의 전부라는 보장이 사라진다** — 그런데
    /// `messages { id }` 응답은 그걸 완전한 목록인 양 보여 준다(그룹 방송이면 "일부 멤버는 아예 없었던 것"
    /// 처럼 읽힌다). 이 플래그가 그 불완전 가능성을 조회에 실어 나르는 최소 신호다.
    /// ★왜 "정확히 몇 건 잘렸나" 가 아닌가★: 어떤 msg_id 의 행이 몇 개 사라졌는지는 이미 버린 데이터라
    ///   재구성할 수 없다. 없는 정확도를 지어내지 않고 **"잘렸을 수 있다" 는 사실만** 정직하게 전한다
    ///   (`records_for_detailed` 가 이 플래그와 위치로 판정을 좁힌다).
    // 리뷰 B2
    evicted_any: bool,
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
            evicted_any: false,
        }
    }

    /// 새 메시지 배달 레코드를 이력에 append(초기 상태 지정). 용량 초과 시 가장 오래된 레코드 evict.
    ///
    /// ★초기 상태 인자화★: 단일/그룹 발송은 `Pending`(주입 대기) 또는 `Delivered`(즉시 주입 폴백)로,
    ///   그룹 죽은 멤버는 `Skipped` 로 시작하므로 호출자가 초기 상태를 정한다(spec §4·§5).
    /// ★evict = front(오래된 것) + **회신으로 닫힌 계약만** 동반 정리(C3 fix 3 → D 리뷰 B3 로 좁힘)★: 링버퍼라
    ///   용량을 넘기면 가장 오래된 이력부터 버린다. 이때 같은 msg_id 의 request 추적 항목은 **회신으로 닫힌
    ///   것만** 함께 드롭한다(dangling 정리 + 유계). ★미회신 계약은 통지가 나갔든 아니든 evict 를 견딘다★ —
    ///   예전엔 통지된 것도 함께 드롭해서, 이력이 먼저 밀려난 뒤 기한이 지난 계약이 **미결 목록에서 통째로
    ///   증발**했다(리뷰 B3 — `RequestEntry::is_live` 주석에 시퀀스). 계약의 정본은 추적이지 이력이 아니므로
    ///   (ReplyOutcome 주석), 이력 용량이 계약을 죽이면 안 된다. 미회신 계약의 유계는 `MAX_OPEN_REQUESTS`
    ///   (open_request 의 `Full`)가 따로 준다.
    /// ★그 evict 를 견딘 계약이 나중에 **닫히면**(fix 1)★ 정리해 줄 evict 이벤트가 이미 지나갔으므로 닫힘
    ///   시점에 즉시 지운다 — 여기 evict 경로의 정리는 그 짝(belt)이다(`purge_finished_without_history`).
    /// ★evict 사실은 남긴다(리뷰 B2)★: 한 번이라도 버렸으면 `evicted_any` 를 세워, 조회(`messages { id }`)가
    ///   자기 행 목록이 완전하다고 단언하지 못하게 한다(`records_for_detailed`).
    /// 단일 수신자 발송(및 notice)용 — 기대 배달기록 수 = 1. 그룹 fan-out 은 `record_with_expected` 로
    /// 멤버 수 N 을 실어야 조회가 잘림을 정확히 판정한다(`MessageRecord.expected_rows`).
    pub fn record(
        &mut self,
        msg_id: &str,
        from: &str,
        to: &str,
        body: &str,
        status: DeliveryStatus,
        now: Instant,
    ) {
        self.record_with_expected(msg_id, from, to, body, status, now, 1);
    }

    /// `record` + **기대 배달기록 수**(round-2 리뷰 F3). 같은 `msg_id` 의 모든 행이 같은 `expected` 를 든다 —
    /// 그룹 fan-out 은 두 단계(계획 락 / 멤버별 구간)로 기록하므로 두 곳 모두 같은 N 을 넘겨야 한다.
    #[allow(clippy::too_many_arguments)]
    pub fn record_with_expected(
        &mut self,
        msg_id: &str,
        from: &str,
        to: &str,
        body: &str,
        status: DeliveryStatus,
        now: Instant,
        expected_rows: u16,
    ) {
        self.history.push_back(MessageRecord {
            msg_id: msg_id.to_string(),
            from: from.to_string(),
            to: to.to_string(),
            body: body.to_string(),
            status,
            created_at: now,
            transitioned_at: now,
            // 0 은 의미가 없다(모든 발송은 최소 1행) — 방어적으로 1 로 올려 `rows < expected` 판정이
            //   "행이 있는데 기대가 0" 같은 모순 상태에 빠지지 않게 한다.
            expected_rows: expected_rows.max(1),
        });
        // 용량 초과 — 가장 오래된(front) 것부터 evict(오래된 순 유지). evict 뒤 **닫힌 계약 중 이력이 사라진
        // 것**을 정리한다(`purge_finished_without_history`) — 미회신 계약은 남겨야 회신·통지·미결 조회 경로가
        // 유지된다(위 주석 B3). 미회신분의 상한은 MAX_OPEN_REQUESTS 가 준다.
        let mut evicted_now = false;
        while self.history.len() > self.capacity {
            if self.history.pop_front().is_some() {
                evicted_now = true;
            }
        }
        if evicted_now {
            // 조회 정직성 플래그(B2) — 한 번 서면 내려가지 않는다(버린 데이터는 돌아오지 않는다).
            self.evicted_any = true;
            self.purge_finished_without_history();
        }
    }

    /// ★**닫힌**(회신 온) 계약 중 가리킬 이력이 없는 추적 항목을 제거한다(round-final fix 1 · load-bearing)★.
    ///
    /// ★막는 것 = 좀비 추적 항목★: 닫힌 항목의 정상 정리 계기는 "같은 msg_id 이력이 evict 될 때"(`record`)다.
    ///   그런데 fix 3 로 **미회신 계약이 evict 를 견디게** 된 순간, 이력이 먼저 밀려난 계약은 나중에 닫힐 때
    ///   **정리해 줄 evict 이벤트가 이미 지나간 상태**가 된다. 그 항목은 `is_live()` 가 false 라
    ///   `MAX_OPEN_REQUESTS` 계수에도 안 잡히므로 **어떤 상한도 안 걸린 채** 영원히 쌓인다(인메모리 v1 의
    ///   유계 보장 붕괴). 그래서 닫히는 그 순간과 evict 때, 이력 유무를 보고 고아를 즉시 지운다.
    /// ★미회신 계약은 절대 안 건드린다(D 리뷰 B3 로 범위 축소)★: 예전엔 **통지된**(기한 초과) 계약도 "끝난
    ///   것" 으로 보고 함께 지웠는데, 통지는 회신이 아니다 — 그 삭제가 미결 조회에서 의무를 증발시켰다
    ///   (`RequestEntry::is_live` 주석의 4단계 시퀀스). 이제 `is_live() == !closed` 라 통지분은 여기 걸리지
    ///   않고, 그 유계는 `MAX_OPEN_REQUESTS` 가 준다.
    /// ★비용★: 닫힌 항목이 하나도 없으면(대부분) 선형 스캔 한 번으로 즉시 반환하고, 있을 때만 이력
    ///   msg_id 집합(≤ capacity)을 만들어 O(이력 + 추적)으로 판정한다.
    // 리뷰 B3
    // ADR-0108 (잠정 purge 면제 — 미정산 항목의 수명은 가드 소유)
    fn purge_finished_without_history(&mut self) {
        if self.requests.iter().all(|r| r.is_live() || r.provisional) {
            return;
        }
        let live_ids: HashSet<&str> = self.history.iter().map(|r| r.msg_id.as_str()).collect();
        // ★잠정 계약은 **절대 여기서 지우지 않는다**(round-6 I1 · load-bearing)★: 잠정 구간에는 아직 이력
        //   행이 없을 수 있다(계약 예약은 dispatch **전**, 이력 기록은 dispatch **안**이다). 그 창에 회신이
        //   도착해 계약이 닫히면 "끝났고 이력도 없다" 는 조건이 성립해 이 정리가 항목을 통째로 지웠다 —
        //   그러면 예약해 둔 자리가 증발해 I1 이 막으려던 상한 초과가 **다른 문으로** 되살아나고, 가드의
        //   롤백은 `NotFound` 를 받아 아무 것도 정산하지 못한다. 미확정 항목의 수명은 **가드가 소유한다**.
        self.requests
            .retain(|r| r.is_live() || r.provisional || live_ids.contains(r.request_id.as_str()));
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
    /// ★cap 도달 = **가장 오래된 은퇴 가능 계약을 내보내고 수용**(사용자 결정 2026-07-27 · round-2 F1)★.
    ///   전량 반려(`Full`)는 은퇴시킬 게 하나도 없을 때만이다.
    ///
    /// ★왜 바뀌었나(B3 가 남긴 구멍)★: B3 로 "통지된 미회신 계약" 이 추적에 남게 되면서, 그 부류에는
    ///   **TTL 도 취소도 없다** — 회신이 영영 안 오면 슬롯을 영구 점유한다. 512개가 그렇게 차면 데몬을
    ///   재시작할 때까지 **모든** 새 request 가 `REQUEST_CAPACITY` 로 막힌다(전역 기능 정지). 메일박스·
    ///   notice 레인이 cap 에서 "가장 오래된 것을 은퇴" 시키는 것과 같은 패턴으로 압력을 푼다.
    /// ★은퇴 가능(evictable) = 발신자에게 **남은 통지 약속이 없는** 계약★:
    ///     (a) `notified == true` — 기한 초과 통지가 이미 나갔다(발신자는 결말을 통보받았다), 또는
    ///     (b) `reply_by == None` — 애초에 기한이 없어 통지를 약속한 적이 없다.
    ///   ★절대 은퇴시키지 않는 것★: 기한이 남아 있는데 아직 통지 안 된 계약 — 그 계약은 **데몬이 발신자에게
    ///   진 빚**(기한 초과 시 notice)이다. 그걸 지우면 약속한 통지가 영영 안 나가는 조용한 위약이 된다.
    /// ★오래된 순★: `requests` 는 append 순서 = 발송 순서라 첫 매치가 가장 오래된 것이다.
    /// ★조용한 소멸 금지★: 표시 사실은 `OpenedAfterMarking` 으로 호출자에게 올라가, **커밋 시점**에 락 밖
    ///   계측 로그가 된다(표시만 하고 롤백되면 아무 일도 없었으므로 로그도 없다).
    ///   이력 링의 행은 손대지 않는다(링이 자기 수명을 소유 — 은퇴는 **계약 추적**만의 일이다).
    // round-2 리뷰 F1 / 사용자 결정 2026-07-27
    /// ★인자 `reply_by` = (기한, 표기 원본)★: 표기는 통지 문구에 그대로 쓰인다(`DueTimeout.reply_by_raw`).
    ///   튜플로 묶어 "기한이 있으면 표기도 있다" 를 타입으로 강제한다(둘이 어긋날 여지 자체를 없앤다).
    /// ★인자 `recipient_id`(D 리뷰 B1)★: 발송 시점에 수신자가 산 에이전트로 **해석됐으면** 그 AgentId,
    ///   부재 파킹이면 `None`. 동명 다수에서 회신 의무를 정확히 귀속시키는 축이다(`RequestEntry.recipient_id`).
    #[allow(clippy::too_many_arguments)]
    pub fn open_request(
        &mut self,
        request_id: &str,
        sender: &str,
        sender_id: AgentId,
        recipient: &str,
        recipient_id: Option<AgentId>,
        reply_by: Option<(Duration, String)>,
        now: Instant,
    ) -> OpenOutcome {
        // 같은 id 가 추적에 하나라도 있으면(open/closed 무관) 거부 — id 는 데몬 생성 유일값(재사용 non-scenario).
        if self.requests.iter().any(|r| r.request_id == request_id) {
            return OpenOutcome::DuplicateId;
        }
        // 슬롯 계수 정본 = `occupies_slot`(은퇴 표시 제외 · 잠정은 닫혀도 포함) — 그 주석 참조.
        let mut retired = None;
        if self.occupied_slots() >= MAX_OPEN_REQUESTS {
            // cap 압력 — 가장 오래된 **은퇴 가능** 계약에 은퇴 예정 표시를 한다(제거하지 않는다).
            // ★후보에서 빼는 두 부류(round-5 mark-and-sweep · load-bearing)★:
            //   ① 이미 은퇴 표시된 계약(`pending_retirement`) — 두 발송이 같은 희생자를 노리면 한쪽 커밋이
            //      다른 쪽의 희생자를 먼저 지워, 남은 쪽의 롤백/커밋이 허공을 가리킨다(계수도 어긋난다).
            //   ② **잠정 계약**(`provisional`) — 아직 접수 확정 전인 남의 신규 계약이다. 이걸 희생자로 고르면
            //      그 발송은 **배달에 성공했는데 계약이 없는** 상태가 되고, 그 request 로 온 회신이 전부
            //      `NoMatch` 로 빗나간다(발신자는 영원히 기다리고 기한 통지도 못 받는다).
            // ★"가장 오래된" 은 `created_at` 기준★ — 목록 위치에 의존하지 않는다. 동률이면 `min_by_key` 가
            //   첫 원소를 주므로 append 순서가 타이브레이크로 남는다.
            let victim = self
                .requests
                .iter_mut()
                .filter(|r| {
                    r.occupies_slot() && !r.provisional && (r.notified || r.reply_by.is_none())
                })
                .min_by_key(|r| r.created_at);
            match victim {
                Some(v) => {
                    v.pending_retirement = true;
                    retired = Some(RetiredContract {
                        request_id: v.request_id.clone(),
                        sender: v.sender.clone(),
                        recipient: v.recipient.clone(),
                        age: now.saturating_duration_since(v.created_at),
                    });
                }
                // 표시할 수 있는 계약이 하나도 없다 = 남은 건 데몬이 진 통지 빚이거나 남의 미확정 접수분뿐.
                //   그때만 반려한다(가시적 실패 — 발신자가 통보로 우회하거나 잠시 뒤 재시도한다).
                //   ★잠정 계약만 남아 `Full` 이 나오는 것도 정직한 답이다★: 그 순간 상한은 실제로 차 있고,
                //   경합 상대가 커밋/롤백을 끝내면 곧 자리가 난다.
                None => return OpenOutcome::Full,
            }
        }
        self.requests.push(RequestEntry {
            request_id: request_id.to_string(),
            sender: sender.to_string(),
            sender_id,
            recipient: recipient.to_string(),
            recipient_id,
            reply_by,
            created_at: now,
            closed: false,
            notified: false,
            pending_retirement: false,
            // 접수 확정 전 — 커밋이 이 표시를 지운다(그전엔 남의 희생자가 될 수 없다).
            provisional: true,
        });
        match retired {
            Some(r) => OpenOutcome::OpenedAfterMarking(r),
            None => OpenOutcome::Opened,
        }
    }

    /// ★예약 확정(커밋) — 표시된 희생자를 **물리 제거**하고 잠정 표시를 지운다(round-5 mark-and-sweep)★.
    ///
    /// 여기가 은퇴가 실제 사건이 되는 유일한 지점이다(호출자는 이 뒤에 계측 로그를 찍는다 — 그전에 찍으면
    /// 일어나지 않은 일을 보고하게 된다).
    /// ★희생자가 그 사이 회신으로 닫혔어도 그냥 제거한다★: `replied` 는 종점이라 더 볼 일이 없고, 그
    ///   사실은 이력 레코드에 이미 `Replied` 로 남아 있다(추적 항목은 회계용일 뿐이다).
    /// ★희생자를 못 찾으면 no-op★: 정상 흐름엔 없다(표시된 항목은 커밋/롤백까지 목록에 남는다).
    // round-5 mark-and-sweep
    pub fn commit_open(&mut self, provisional_id: Option<&str>, retired_id: Option<&str>) {
        if let Some(id) = provisional_id {
            if let Some(e) = self.requests.iter_mut().find(|r| r.request_id == id) {
                e.provisional = false;
            }
        }
        if let Some(id) = retired_id {
            self.requests
                .retain(|r| !(r.request_id == id && r.pending_retirement));
            // 이력이 이미 밀려난 채 끝난 항목이 있으면 함께 정리(좀비 방지 — purge 주석).
            self.purge_finished_without_history();
        }
    }

    /// ★예약 취소(롤백) — 표시만 지우고 잠정 계약을 제거한다(round-5 mark-and-sweep)★. 반환 = 잠정 계약
    /// 제거 결과(호출자가 락 밖에서 로깅).
    ///
    /// ★되돌릴 상태가 없다는 게 이 설계의 요점★: 희생자는 목록을 떠난 적이 없으므로 표시 한 비트만 지우면
    ///   **아무 일도 없던 상태**다 — 옛 설계의 재삽입(위치·나이 복원) 기계가 통째로 사라졌고, 그와 함께
    ///   "복원했더니 남이 이미 그 자리를 썼더라" 류의 실패 모드도 사라졌다.
    /// ★그 사이 희생자가 회신으로 닫혔어도 표시만 지운다★: 닫힘은 그대로 유지된다(정당한 회신이었다) —
    ///   유령 재개방도, 뒤늦은 헛 기한 통지도 없다(`due_timeouts` 는 `closed` 를 건너뛴다).
    /// ★한 락 구간에서 둘 다★: 호출자가 이 한 번의 호출로 끝내므로 바깥에서 계약 수가 상한을 넘어 보이는
    ///   창이 없다(옛 분리-락 배선이 만들던 513 표류의 근원).
    /// ★알려진 잔여 — id ABA(round-6 리뷰 note · 기계 추가 안 함)★: **은퇴 표시된** 희생자가 그 사이 회신으로
    ///   닫히고 자기 이력 행까지 링에서 밀려나면 `purge_finished_without_history` 가 그 항목을 지울 수 있다
    ///   (잠정 계약 쪽은 그 정리에서 **제외**했으므로 이 잔여에 해당하지 않는다 — 그 함수 주석 참조).
    ///   그러면 그 id 가 다시 발급 가능해지고, 가드가 정산하기 전에 **같은 랜덤 id** 로 새 계약이 태어나
    ///   하필 은퇴 표시까지 붙으면, 여기 표시 해제가 그 **새 계약**의 표시를 대신 지운다. 확률은
    ///   36^8(2.8×10^12) 공간에서 마이크로~밀리초 창에 같은 값을 뽑고 그게 다시 희생자로 뽑히는 곱이고,
    ///   피해는 은퇴 1건 취소(상한 압력이 한 번 덜 풀림 — 다음 발송이 다시 표시한다)로 유계다. 막으려면
    ///   id 예약 집합을 되살려야 하는데(round-4 에서 지운 기계) 그 복잡도가 이 확률에 값하지 않는다고
    ///   판단했다 — **모르고 지나친 게 아니라 값을 매겨 남긴 잔여**다.
    // round-5 mark-and-sweep
    pub fn rollback_open(
        &mut self,
        provisional_id: Option<&str>,
        retired_id: Option<&str>,
    ) -> Option<DropOutcome> {
        if let Some(id) = retired_id {
            if let Some(e) = self
                .requests
                .iter_mut()
                .find(|r| r.request_id == id && r.pending_retirement)
            {
                e.pending_retirement = false;
            }
        }
        provisional_id.map(|id| self.drop_request(id))
    }

    /// ★실제 배달된 수신자로 계약의 `recipient_id` 를 고쳐 박는다(round-2 리뷰 F2 · load-bearing)★.
    /// 그런 계약이 없으면(통보였거나 이미 닫힘) no-op.
    ///
    /// ★막는 것 = 배달자/의무자 불일치★: exact AgentId 로 건 request 가 그 순간 busy 라 **이름 키**로
    ///   파킹되면, 봉투는 이름 큐에 놓이고 id 는 힌트일 뿐이다. 그 뒤 A 가 죽고 같은 이름의 B 가 뜨면
    ///   flush 의 이름 폴백이 **B 에게 배달**한다(단일 발송은 재스폰 이어받기가 기능이다 — ADR-0101).
    ///   그런데 계약의 `recipient_id` 는 여전히 A 라, id 기준 매처(`matches_contract_party`)에서 B 의 미결
    ///   조회는 그 의무를 **못 본다** — 봉투를 실제로 받은 쪽이 "답할 게 없다" 고 읽는 최악의 조합이다.
    ///   그래서 **봉투가 실제로 꽂힌 시점**(pending→delivered 전이 자리, 착지 incarnation 을 아는 유일한
    ///   지점)에 의무를 그 수신자에게 옮긴다 — "의무는 봉투를 받은 자를 따른다".
    /// ★epoch 은 담지 않는다★: 같은 에이전트의 재시작은 AgentId 를 유지한다(ADR-0007) — 의무는 유지돼야 한다.
    /// ★닫힌 계약은 건드리지 않는다★: 이미 회신이 온 계약의 상대를 뒤늦게 바꾸면 이력이 오염된다.
    // round-2 리뷰 F2
    pub fn rebind_request_recipient(&mut self, request_id: &str, delivered_to: AgentId) {
        if let Some(r) = self
            .requests
            .iter_mut()
            .find(|r| r.request_id == request_id && !r.closed)
        {
            r.recipient_id = Some(delivered_to);
        }
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
            // ★은퇴 예정 표시된 계약도, 잠정 계약도 **여기 그대로 있다**(round-5 mark-and-sweep)★ —
            //   물리 제거를 없앤 덕에 별도 예약 집합(옛 `reserved_ids`) 없이 평소 추적 조회만으로 충분하다.
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
    ///   안 걸리고 통지가 한 번 더 나갈 수 있다. 이력 용량(HISTORY_CAPACITY)만큼이 밀려난 뒤 마이크로초 창에서만 성립하는
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
    /// ★표시(은퇴 예정·잠정)는 매칭을 가리지 않는다(round-5 mark-and-sweep · load-bearing)★: 두 표시는
    ///   **회계용**이지 존재 여부가 아니다. 여기서 `!closed` 만 보므로 ① 은퇴 예정으로 표시된 계약에 온
    ///   정당한 회신도 정상적으로 닫히고(옛 물리 제거 설계에선 이게 `NoMatch` 로 빗나갔다 — 발신자는 답을
    ///   받았는데 계약은 안 닫히고 나중에 헛 기한 통지까지 날 수 있었다) ② 아직 확정 전인 잠정 계약에 온
    ///   빠른 회신도 닫힌다. 닫힌 뒤의 커밋(제거)·롤백(표시 해제) 어느 쪽도 그 사실을 되돌리지 않는다.
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
    /// ★은퇴 예정 표시는 건너뛰지 않고, **잠정 계약은 건너뛴다**(round-5 → round-6 I2 로 갈래 분리)★:
    ///   - **은퇴 표시된 계약은 애초에 due 가 될 수 없다**(구조적, 그대로 유지): 희생자 자격이
    ///     `notified || reply_by.is_none()` 이고 due 자격은 `!notified && reply_by.is_some()` 이라 두 집합은
    ///     **서로소**다. 그래서 표시 검사를 넣어 봐야 죽은 분기다 — 넣지 않는 편이 정직하다.
    ///   - ★**잠정 계약은 명시적으로 건너뛴다**(round-6 I2 · load-bearing)★. 옛 근거("잠정 구간은
    ///     마이크로초라 1분 기한에 닿을 수 없다")는 **틀렸다**: 잠정 구간은 dispatch 를 감싸고, dispatch 는
    ///     자식 stdin `write_all` 을 한다 — 우리 `stdio.rs` 가 스스로 문서화하듯 파이프 역압 아래에서 그
    ///     쓰기는 **무한정 블록될 수 있다**. 즉 1분 기한 request 가 잠정 구간 안에서 sweep 에 걸려 통지가
    ///     나갈 수 있고, 그 뒤 발송이 반려되면 발신자는 **공식적으로 존재한 적 없는 요청**(반려를 받은)에
    ///     대한 기한 초과 통지를 손에 쥔다. 통지는 회수 불가라 되돌릴 수도 없다.
    ///   - ★유실 없음(hand-off)★: 건너뛴 계약의 `created_at` 은 **원본 그대로**다. 커밋되면 그 다음 sweep
    ///     (60초 주기)이 이미 지난 기한을 보고 **즉시** 통지한다 — 지연될 뿐 사라지지 않는다. 롤백되면
    ///     계약 자체가 없었던 일이 되므로 통지도 없어야 맞다. 양쪽 다 정답이 되는 유일한 배치다.
    // ADR-0108 (잠정 스킵 — 커밋 후 다음 스윕이 원래 시각으로 통지)
    pub fn due_timeouts(&mut self, now: Instant) -> Vec<DueTimeout> {
        let mut due = Vec::new();
        for r in self.requests.iter_mut() {
            // `provisional` = 아직 접수 확정 전 — 위 doc 의 hand-off 규약(커밋 후 다음 sweep 이 집는다).
            if r.closed || r.notified || r.provisional {
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

    /// ★`records_for` + **불완전 신호**(D 리뷰 B2 · round-2 리뷰 F3 로 판정 교체)★ — `(행 목록, truncated)`.
    ///
    /// ★왜 필요한가★: 링(4096)이 가득 차면 앞쪽 행이 조용히 사라진다. 그런데 `messages { id }` 응답은 남은
    ///   행을 **그 메시지의 전부**인 양 보여 준다 — 10인 방송의 6행이 밀려나면 발신자는 "4명에게만 나갔다"
    ///   고 오독한다(실제로는 10명 모두에게 나갔고 기록만 사라졌다).
    /// ★판정 = `남은 행 수 < 기대 행 수`(결정적)★. 기대 수는 발송 시점에 각 행에 박힌다
    ///   (`MessageRecord.expected_rows`) — 단일 발송·notice = 1, 그룹 fan-out = 멤버 수 N.
    /// ★옛 "front 위치" 증명을 폐기한 이유(round-2 F3 — 거짓 음성)★: 그 판정은 "한 msg_id 의 행이 링에서
    ///   **연속**" 이라는 전제 위에 있었는데 그게 틀렸다. 그룹 행은 **두 단계**로 기록된다(계획 락에서
    ///   parked/skipped → 그 뒤 멤버별 구간에서 delivered), 그 사이 다른 메시지의 행이 끼어든다. 그래서
    ///   앞쪽 행이 evict 됐는데도 링 front 는 남의 행일 수 있고, 그때 옛 판정은 `false`(= "확실히 완전")를
    ///   내놨다 — 증명을 자처하면서 틀리는 최악의 형태였다. 위치가 아니라 **개수**를 보면 그 전제가 필요 없다.
    /// ★행이 **통째로** 사라진 경우는 여기서 안 보인다★: 그건 빈 목록으로 나가고 상위가 계약 뷰 또는
    ///   `MESSAGE_NOT_FOUND` 로 답한다(그 hint 가 이력 회전을 알린다).
    // 리뷰 B2 / round-2 리뷰 F3
    pub fn records_for_detailed(&self, msg_id: &str) -> (Vec<&MessageRecord>, bool) {
        let rows: Vec<&MessageRecord> =
            self.history.iter().filter(|r| r.msg_id == msg_id).collect();
        // 기대 수는 모든 행이 공유하므로 남은 아무 행에서나 읽으면 된다(첫 행 사용).
        let truncated = rows
            .first()
            .is_some_and(|r| rows.len() < usize::from(r.expected_rows));
        (rows, truncated)
    }

    /// 이력 링이 한 번이라도 evict 했나(B2 — 조회 정직성 신호의 원천). 테스트·상위 판정용.
    pub fn history_evicted(&self) -> bool {
        self.evicted_any
    }

    /// 전 이력 레코드(오래된 순) — 관측/테스트 스냅샷. 상위(MessagingService)가 "notice 가 장부에 남았나"
    /// 처럼 msg_id 를 모르는 단언을 할 때 쓴다(msg_id 를 아는 조회는 `records_for`).
    pub fn all_records(&self) -> Vec<&MessageRecord> {
        self.history.iter().collect()
    }

    /// ★지금 `MAX_OPEN_REQUESTS` 슬롯을 차지하는 계약 수(round-6 I1)★ — 상한 판정이 보는 바로 그 값이다.
    ///
    /// `open_request_count`(= 미회신 계약 수)와 **다르다**: 은퇴 표시된 계약은 빠지고, 정산 전 잠정 계약은
    /// 닫혔더라도 남는다(`occupies_slot` 주석). 상한 산술을 단언하려면 이쪽을 봐야 한다.
    pub fn occupied_slots(&self) -> usize {
        self.requests.iter().filter(|r| r.occupies_slot()).count()
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

    /// ★미회신(열려 있는) request 전부를 조회 뷰로(S18 D — `messages` 무인자)★. **오래된 순**(`created_at`
    /// 오름차순, 동률이면 현재 목록 순서 — stable sort).
    ///
    /// ★왜 명시적으로 정렬하나(round-4 리뷰 H4)★: 예전엔 "추가 순서 = 발송 순서" 라는 이유로 raw Vec 순서를
    ///   그대로 냈는데, 그 전제는 호출자가 단조 시계를 쓸 때만 참인 **가정**이지 이 자료구조가 강제하는
    ///   성질이 아니다(시계는 주입된다 — 모듈 헤더 순수성 불변식). 문서가
    ///   약속한 순서와 실제가 갈리면 조회 소비자가 조용히 어긋난 목록을 본다(그리고 그 어긋남은 복원이
    ///   일어난 드문 경로에서만 나타나 재현이 어렵다). ≤512개라 매 조회 정렬 비용이 무시 가능하므로 약속을
    ///   코드로 지킨다.
    /// ★이중 정렬 아님★: 상위 `open_items_for` 는 세 갈래를 **합친 뒤** 경과 내림차순으로 다시 정렬하므로
    ///   여기 순서에 의존하지 않는다(같은 결과). 이 정렬은 이 함수의 계약을 지키기 위한 것이다.
    /// ★표시된 계약도 **보인다**(round-5 mark-and-sweep — 명시적 선택)★:
    ///   - **은퇴 예정 표시**: 커밋 전까지는 여전히 열린 계약이다. 그 마이크로초 창에 조회가 걸리면 목록에
    ///     뜨는데, 그게 **사실**이다(아직 아무 것도 은퇴하지 않았다). 미리 숨기면 커밋되지 않을 수도 있는
    ///     제거를 조회가 먼저 보고하는 셈이라 더 나쁘다.
    ///   - **잠정 계약**: 실재하는 접수분이라 보이는 게 맞다. 반려로 끝나면 그때 사라진다.
    ///
    /// ★포함 기준 = `!closed`(= `is_live()`) — load-bearing★: 통지가 나갔어도 회신은 여전히 안 왔고,
    ///   수신자는 아직 답할 의무가 있다(spec §3: 늦어도 회신하라). 미결 조회에서 빼면 발신자·수신자 양쪽이
    ///   "목록에 없으니 끝난 것" 으로 오독한다. 그래서 통지 여부는 **필드로 노출**하고 목록에서 제외하지 않는다.
    /// ★`is_live()` 와 같은 기준이라는 게 핵심(D 리뷰 B3)★: 예전엔 `is_live()` 가 `!closed && !notified` 라
    ///   둘이 갈렸고, 그 틈으로 evict-후-통지된 계약이 추적에서 삭제돼 이 목록에서도 증발했다. 이제 두
    ///   정의가 하나다 — **여기 기준을 바꾸면 `is_live()` 도 함께 바꿔야 한다**(갈리면 같은 버그가 재발).
    /// ★필터는 상위가★: 이름별(발신/수신) 갈래는 호출자가 정한다 — 장부는 이름 규약을 모른다.
    // ADR-0103 (spec §6 messages 무인자 = 내 미결)
    pub fn open_requests(&self) -> Vec<OpenRequestView> {
        let mut out: Vec<OpenRequestView> = self
            .requests
            .iter()
            .filter(|r| !r.closed)
            .map(|r| OpenRequestView {
                request_id: r.request_id.clone(),
                sender: r.sender.clone(),
                sender_id: r.sender_id,
                recipient: r.recipient.clone(),
                recipient_id: r.recipient_id,
                reply_by_raw: r.reply_by.as_ref().map(|(_, raw)| raw.clone()),
                created_at: r.created_at,
                notified: r.notified,
            })
            .collect();
        // stable sort — 같은 시각이면 현재 목록 순서(= 발송 순서)가 타이브레이크로 남는다.
        out.sort_by_key(|r| r.created_at);
        out
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

    /// ★확정된 계약 픽스처(round-5 mark-and-sweep)★ — `open_request` 로 열고 **즉시 커밋**한다.
    ///
    /// ★왜 커밋이 필요한가★: 새로 열린 계약은 `provisional` 표시를 달고 나오고, 그 표시가 있는 동안은
    ///   **희생자 후보에서 제외**된다(남의 미확정 접수분을 뺏지 않기 위한 규칙 — round-5 (1)). 운영에선
    ///   `ReservationGuard` 가 반드시 커밋/롤백하므로 "확정된 계약" 이 정상 상태다. 상한 픽스처를 커밋 없이
    ///   쌓으면 전부 잠정으로 남아 `Full` 만 나오므로(그게 정직한 동작이다), 테스트도 운영과 같은 상태를
    ///   만들어야 한다.
    fn open_committed(
        l: &mut Ledger,
        id: &str,
        reply_by: Option<(Duration, String)>,
        now: Instant,
    ) -> OpenOutcome {
        let out = l.open_request(id, "alice", sid(), "bob", None, reply_by, now);
        l.commit_open(Some(id), None);
        out
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
        l.open_request(id, "alice", sid(), "bob", None, reply_by, now);
        // round-5: 접수 확정(운영의 ReservationGuard::commit 과 같은 자리) — 안 하면 잠정으로 남는다.
        l.commit_open(Some(id), None);
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
            l.open_request("req-1", "alice", sid(), "bob", None, None, now),
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
        l.open_request("req-1", "alice", sid(), "bob", None, None, now);
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
            l.open_request("req-1", "alice", sid(), "bob", None, None, now),
            OpenOutcome::Opened
        );
        assert_eq!(
            l.open_request("req-1", "alice", sid(), "carol", None, None, now),
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
            l.open_request("req-1", "alice", sid(), "bob", None, None, now),
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
        l.open_request("req-1", "alice", sid(), "bob", None, None, now);
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
        l.open_request("req-1", "alice", sid(), "bob", None, None, now);
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
        l.open_request("req-1", "alice", sid(), "bob", None, None, now);
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
                                                 // round-6 I2: 확정된 계약이어야 sweep 대상이다(잠정은 건너뛴다) — 운영의 커밋과 같은 자리.
        open_committed(&mut l, "req-1", rb(reply_by), now);
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
            None,
            Some((reply_by, "60m".to_string())),
            now,
        );
        l.commit_open(Some("req-1"), None); // round-6 I2: 확정 후에야 sweep 대상.
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
        l.open_request("req-1", "alice", sid(), "bob", None, rb(reply_by), now);
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
            l.open_request("closed-1", "alice", sid(), "bob", None, rb(reply_by), now),
            OpenOutcome::DuplicateId,
            "닫힌 항목은 추적에 남아 재오픈을 막는다"
        );

        // 제거: 흔적이 없으니 같은 id 재오픈 가능.
        l.open_request("dropped-1", "alice", sid(), "bob", None, rb(reply_by), now);
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
            l.open_request("dropped-1", "alice", sid(), "bob", None, rb(reply_by), now),
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
        l.open_request("req-1", "alice", sid(), "bob", None, None, now);
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
        // round-6: 확정 계약이어야 evict 동반 정리의 대상이다(잠정 항목은 가드가 소유해 정리에서 제외).
        open_committed(&mut l, "done", None, now);
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
        open_committed(&mut l, "req-1", rb(reply_by), now); // round-6 I2: 확정 후 sweep 대상.
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
        open_committed(&mut l2, "req-2", rb(reply_by), now);
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
        l.open_request(id, "alice", sid(), "bob", None, reply_by, now);
        // round-6 I2: 접수 확정(운영의 ReservationGuard::commit) — 잠정 상태로는 sweep 대상이 아니다.
        l.commit_open(Some(id), None);
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
    fn timeout_notice_after_history_eviction_keeps_the_unanswered_contract_tracked() {
        // ★D 리뷰 B3 로 **결론이 뒤집힌 테스트**(옛 이름: `..._removes_the_finished_tracking_entry`)★.
        //
        // 옛 단언은 "통지가 나갔으면 끝난 계약이니 고아 항목을 제거한다" 였다. 그 전제가 틀렸다 — 통지는
        //   발신자에게 알렸다는 사실일 뿐 **회신은 여전히 안 왔다**. 제거하면 D 의 미결 조회
        //   (`open_requests` = `!closed`)에서 그 의무가 통째로 사라져, 수신자는 "답할 게 없다" 로, 발신자는
        //   "끝난 일" 로 읽는다(조용한 유실). 그래서 통지된 미회신 계약은 **추적에 남는다**.
        // ★유계는 어디서 오나★: 이제 이런 항목도 `is_live()` 라 `MAX_OPEN_REQUESTS`(512) 계수에 잡힌다 —
        //   무한 누적이 아니라 cap 에서 `REQUEST_CAPACITY` 반려로 가시화된다.
        let now = t0();
        let reply_by = Duration::from_secs(600);
        let mut l = ledger_with_evicted_live_contract(2, "req-1", rb(reply_by), now);
        let due = l.due_timeouts(now + reply_by + Duration::from_secs(1));
        assert_eq!(due.len(), 1, "evict 됐어도 통지는 나간다(fix 3 유지)");
        assert_eq!(due[0].request_id, "req-1");
        assert_eq!(
            l.tracking_len(),
            1,
            "통지는 회신이 아니다 — 미회신 계약은 추적에 남는다(B3)"
        );
        // 그리고 미결 조회에 **여전히** 보인다(D 계약: timed_out=true 로 표시되되 목록에 남는다).
        let open = l.open_requests();
        assert_eq!(open.len(), 1, "미결 목록에 남아야: {open:?}");
        assert!(open[0].notified, "통지 사실은 플래그로만 구분");
        // 회신이 오면 그때 닫히고, 이력이 없는 고아라 그 순간 정리된다(fix 1 의 원래 목적은 유지).
        assert_eq!(l.close_on_reply("req-1", now), ReplyOutcome::Closed);
        assert_eq!(l.tracking_len(), 0, "닫히는 순간 고아 항목 제거");
    }

    #[test]
    fn tracking_stays_bounded_when_evicted_contracts_are_answered() {
        // ★D 리뷰 B3 로 범위가 좁아진 유계 단언(옛 이름: `..._when_evicted_contracts_finish`)★.
        //
        // 옛 테스트는 "회신이든 통지든 끝나면 추적이 0 으로 수렴" 을 요구했다. 통지분을 남기기로 한 지금
        //   그 요구는 미결 조회 계약과 정면으로 어긋나므로(B3), 유계 단언을 **회신으로 닫힌 계약**에 한정한다.
        //   통지된 미회신분의 유계는 `MAX_OPEN_REQUESTS` cap 이 별도로 주고, 그건 아래 짝 테스트가 지킨다.
        let cap = 2;
        let mut l = Ledger::with_capacity(cap);
        let now = t0();
        let reply_by = Duration::from_secs(60);
        for i in 0..50 {
            let id = format!("r{i}");
            l.record(&id, "alice", "bob", "q", DeliveryStatus::Pending, now);
            open_committed(&mut l, &id, rb(reply_by), now);
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
            // 회신으로 닫히는 순간 고아 항목이 제거된다(fix 1 의 원래 목적).
            assert_eq!(l.close_on_reply(&id, now), ReplyOutcome::Closed);
            assert_eq!(
                l.tracking_len(),
                0,
                "라운드마다 추적이 0 으로 수렴(좀비 누적 없음)"
            );
        }
    }

    /// ★B3 의 짝(round-2 F1 로 결말 갱신) — 통지된 미회신 계약은 추적에 남지만, cap 에서 **은퇴 대상**이라
    /// 전역 기능 정지를 만들지 않는다★. 예전 결말은 `Full` 반려였는데, 그러면 512개가 그 상태로 차는 순간
    /// 데몬 재시작 전까지 모든 새 request 가 막힌다(F1).
    #[test]
    fn a_cap_full_of_notified_contracts_retires_the_oldest_instead_of_blocking_forever() {
        let mut l = Ledger::new();
        let now = t0();
        let reply_by = Duration::from_secs(60);
        let over = now + reply_by + Duration::from_secs(1);
        for i in 0..MAX_OPEN_REQUESTS {
            let id = format!("r{i}");
            l.record(&id, "alice", "bob", "q", DeliveryStatus::Pending, now);
            assert_eq!(
                open_committed(&mut l, &id, rb(reply_by), now),
                OpenOutcome::Opened
            );
        }
        assert_eq!(
            l.due_timeouts(over).len(),
            MAX_OPEN_REQUESTS,
            "전원 기한 초과 통지(= 발신자는 이미 결말을 통보받았다 → 은퇴 가능)"
        );
        assert_eq!(
            l.tracking_len(),
            MAX_OPEN_REQUESTS,
            "통지돼도 미회신이면 남는다(B3)"
        );

        // 새 request → 가장 **오래된** 은퇴 가능 계약(r0)에 표시가 붙고 수용된다.
        let outcome = l.open_request("over", "alice", sid(), "bob", None, None, over);
        match outcome {
            OpenOutcome::OpenedAfterMarking(r) => {
                assert_eq!(r.request_id, "r0", "가장 오래된 것부터 은퇴 표시");
                assert_eq!(r.sender, "alice");
                assert_eq!(r.recipient, "bob");
            }
            other => panic!("표시 후 수용이어야: {other:?}"),
        }
        // ★표시 단계에선 아직 아무 것도 사라지지 않는다(round-5)★ — 회신도 조회도 평소대로 동작한다.
        assert!(
            l.open_requests().iter().any(|r| r.request_id == "r0"),
            "커밋 전에는 희생자가 여전히 열린 계약이다"
        );
        assert_eq!(
            l.tracking_len(),
            MAX_OPEN_REQUESTS + 1,
            "표시 구간엔 +1(잠정분)"
        );
        // 커밋 → 그때 물리 제거되고 총량이 상한으로 돌아온다.
        l.commit_open(Some("over"), Some("r0"));
        assert!(
            !l.open_requests().iter().any(|r| r.request_id == "r0"),
            "커밋에서 비로소 은퇴한다"
        );
        assert_eq!(l.tracking_len(), MAX_OPEN_REQUESTS, "유계 유지(512 불변)");
        assert!(l.open_requests().iter().any(|r| r.request_id == "over"));
    }

    /// ★F1 — 기한 없는 계약도 은퇴 가능(규칙 (b): 통지를 약속한 적이 없다)★.
    #[test]
    fn contracts_without_a_deadline_are_evictable_at_capacity() {
        let mut l = Ledger::new();
        let now = t0();
        for i in 0..MAX_OPEN_REQUESTS {
            assert_eq!(
                open_committed(&mut l, &format!("r{i}"), None, now),
                OpenOutcome::Opened
            );
        }
        let outcome = l.open_request("over", "alice", sid(), "bob", None, None, now);
        assert!(
            matches!(outcome, OpenOutcome::OpenedAfterMarking(ref r) if r.request_id == "r0"),
            "기한 없는 계약은 통지 빚이 없어 은퇴 가능: {outcome:?}"
        );
        l.commit_open(Some("over"), Some("r0"));
        assert_eq!(l.tracking_len(), MAX_OPEN_REQUESTS);
    }

    /// ★F1 의 반대 축 — 통지 뺚이 남은 계약만 있으면 **은퇴하지 않고 반려**한다★.
    ///
    /// 기한이 아직 안 지나 통지가 안 나간 계약은 "데몬이 발신자에게 진 뺚" 이다. 그걸 지우면 약속한 notice 가
    /// 영영 안 나가는 조용한 위약이 되므로, 그 부류뿐일 때는 `Full` 로 **가시적으로** 반려한다.
    #[test]
    fn a_cap_full_of_pending_deadline_contracts_rejects_instead_of_breaking_a_notice_promise() {
        let mut l = Ledger::new();
        let now = t0();
        let reply_by = Duration::from_secs(600);
        for i in 0..MAX_OPEN_REQUESTS {
            assert_eq!(
                open_committed(&mut l, &format!("r{i}"), rb(reply_by), now),
                OpenOutcome::Opened
            );
        }
        // 아직 기한 전 — 아무도 통지되지 않았다(= 전부 은퇴 불가).
        assert!(l.due_timeouts(now).is_empty(), "전제: 기한 전");
        assert_eq!(
            l.open_request("over", "alice", sid(), "bob", None, rb(reply_by), now),
            OpenOutcome::Full,
            "은퇴 가능한 계약이 없으면 반려(통지 약속을 어기지 않는다)"
        );
        assert_eq!(
            l.tracking_len(),
            MAX_OPEN_REQUESTS,
            "아무도 지워지지 않았다"
        );
        // 하나가 기한을 넘겨 통지되면 그 순간부터 은퇴 가능해진다(압력이 풀린다).
        let over = now + reply_by + Duration::from_secs(1);
        assert_eq!(l.due_timeouts(over).len(), MAX_OPEN_REQUESTS);
        assert!(matches!(
            l.open_request("over", "alice", sid(), "bob", None, None, over),
            OpenOutcome::OpenedAfterMarking(_)
        ));
    }

    /// ★B1 — 계약이 **해석된 수신자 id** 를 들고 다닌다★(상위가 의무 귀속을 id 로 가르는 재료).
    #[test]
    fn open_requests_expose_both_party_ids_for_obligation_scoping() {
        let mut l = Ledger::new();
        let now = t0();
        let sender = sid();
        let recipient = sid();
        l.record("r1", "alice", "worker", "q", DeliveryStatus::Pending, now);
        l.open_request("r1", "alice", sender, "worker", Some(recipient), None, now);
        // 부재 파킹(해석 실패) — id 없이 이름만 남는다(이름 폴백 축).
        l.record("r2", "alice", "ghost", "q", DeliveryStatus::Pending, now);
        l.open_request("r2", "alice", sender, "ghost", None, None, now);

        let open = l.open_requests();
        assert_eq!(open[0].sender_id, sender);
        assert_eq!(
            open[0].recipient_id,
            Some(recipient),
            "해석된 수신자는 id 를 남긴다(동명 다수 오귀속 차단의 재료)"
        );
        assert_eq!(
            open[1].recipient_id, None,
            "부재 파킹은 id 가 없다 — 나중에 그 이름으로 등장한 쪽이 답할 주체(WYSIWYA)"
        );
    }

    /// ★B2 + round-2 F3 — 잘림 판정은 **기대 행 수 대비 남은 행 수**다(위치 증명 폐기)★.
    #[test]
    fn records_for_detailed_compares_surviving_rows_against_the_expected_count() {
        let now = t0();
        let mut l = Ledger::with_capacity(8);
        // 단일 발송(기대 1행) — 온전하면 잘림 아님.
        l.record("m1", "a", "b", "x", DeliveryStatus::Delivered, now);
        let (rows, truncated) = l.records_for_detailed("m1");
        assert_eq!(rows.len(), 1);
        assert!(!truncated, "기대 1행이 그대로 있으면 완전");

        // 3인 방송(기대 3행) — 세 행이 다 있으면 완전.
        for to in ["x", "y", "z"] {
            l.record_with_expected("g1", "a", to, "b", DeliveryStatus::Delivered, now, 3);
        }
        let (rows, truncated) = l.records_for_detailed("g1");
        assert_eq!(rows.len(), 3);
        assert!(!truncated, "3/3 이면 완전");

        // ★거짓 음성 회귀 그물★: 방송 행 **사이에 남의 행이 끼어든** 뒤 앞부분만 evict 되게 만든다.
        //   옛 판정(front 위치)은 링 front 가 남의 행이라 "완전" 이라 답했다 — 개수 판정은 잡는다.
        let mut l2 = Ledger::with_capacity(4);
        // 1단계(계획 락): 방송 두 행. 2단계 전에 **남의 행**이 끼어든다. 그 뒤 방송 마지막 행.
        l2.record_with_expected("g2", "a", "m1", "b", DeliveryStatus::Pending, now, 3);
        l2.record_with_expected("g2", "a", "m2", "b", DeliveryStatus::Pending, now, 3);
        l2.record("other1", "a", "z", "x", DeliveryStatus::Delivered, now); // 끼어든 남의 행
        l2.record_with_expected("g2", "a", "m3", "b", DeliveryStatus::Delivered, now, 3);
        // 링(4)을 넘겨 방송의 **앞 두 행만** 밀어낸다 → 남은 방송 행 앞에 남의 행이 선다.
        l2.record("other2", "a", "z", "x", DeliveryStatus::Delivered, now);
        l2.record("other3", "a", "z", "x", DeliveryStatus::Delivered, now);
        let (rows, truncated) = l2.records_for_detailed("g2");
        assert_eq!(rows.len(), 1, "g2 는 3행 중 1행만 남았다: {rows:?}");
        assert_ne!(
            l2.all_records()[0].msg_id,
            "g2",
            "전제: 링 front 는 남의 행 — 옛 위치 증명이 '완전' 이라 답하던 배치"
        );
        assert!(truncated, "남은 행(1) < 기대(3) 이면 잘림(F3)");
    }

    #[test]
    fn open_request_rejects_at_capacity_with_full() {
        // ★fix 3 의 짝(round-2 F1 로 전제 갱신)★: cap 에서 새 계약을 조용히 밀어내지 않는다.
        //   단 F1 이후 "밀어내지 않는다" 가 성립하는 건 **은퇴 불가**(기한 대기 중) 계약뿐이라, 이 테스트는
        //   그 부류로 cap 을 채운다(기한 없는 계약으로 채우면 이제 은퇴가 일어난다 — 별도 테스트가 덮는다).
        let mut l = Ledger::new();
        let now = t0();
        let reply_by = Duration::from_secs(600);
        for i in 0..MAX_OPEN_REQUESTS {
            assert_eq!(
                open_committed(&mut l, &format!("r{i}"), rb(reply_by), now),
                OpenOutcome::Opened
            );
        }
        assert_eq!(
            l.open_request("over", "alice", sid(), "bob", None, rb(reply_by), now),
            OpenOutcome::Full,
            "cap 도달 + 은퇴 가능분 없음 → Full"
        );
        assert_eq!(l.open_request_count(), MAX_OPEN_REQUESTS, "기존 계약 불변");
        // 하나가 끝나면(회신) 자리가 난다 — 계수는 **미회신인 것**만 세기 때문.
        assert_eq!(l.close_on_reply("r0", now), ReplyOutcome::Closed);
        assert_eq!(
            l.open_request("over", "alice", sid(), "bob", None, rb(reply_by), now),
            OpenOutcome::Opened,
            "닫힌 계약은 cap 계수에서 빠진다"
        );
    }

    /// ★G1(round-5 로 단순화) — 롤백은 표시 한 비트만 지운다: 나이·통지 플래그가 애초에 흔들리지 않는다★.
    #[test]
    fn rollback_leaves_the_victim_exactly_as_it_was() {
        let mut l = Ledger::new();
        let now = t0();
        let reply_by = Duration::from_secs(60);
        for i in 0..MAX_OPEN_REQUESTS {
            open_committed(&mut l, &format!("r{i}"), rb(reply_by), now);
        }
        let over = now + reply_by + Duration::from_secs(1);
        assert_eq!(l.due_timeouts(over).len(), MAX_OPEN_REQUESTS);
        let before: Vec<String> = l
            .open_requests()
            .into_iter()
            .map(|r| r.request_id)
            .collect();

        let OpenOutcome::OpenedAfterMarking(victim) =
            l.open_request("new", "alice", sid(), "bob", None, None, over)
        else {
            panic!("전제: 표시 후 수용");
        };
        assert_eq!(victim.request_id, "r0");

        l.rollback_open(Some("new"), Some("r0"));
        let after: Vec<String> = l
            .open_requests()
            .into_iter()
            .map(|r| r.request_id)
            .collect();
        assert_eq!(after, before, "목록·순서가 원상 복구");
        let r0 = l
            .open_requests()
            .into_iter()
            .find(|r| r.request_id == "r0")
            .expect("그대로 있다");
        assert!(r0.notified, "통지 플래그 불변(건드린 적이 없다)");
        assert_eq!(r0.created_at, now, "나이 불변");
        // 표시가 지워졌으므로 다시 압력을 주면 같은 희생자가 다시 뽑힌다.
        assert!(matches!(
            l.open_request("new2", "alice", sid(), "bob", None, None, over),
            OpenOutcome::OpenedAfterMarking(ref v) if v.request_id == "r0"
        ));
    }

    /// ★round-6 I1 — 닫힌 잠정 계약은 **정산 전까지 자기 자리를 지킨다**(상한 영구 초과 차단)★.
    ///
    /// 리뷰가 짚은 6단계를 그대로 재현한다: 잠정 구간에 회신이 먼저 도착하면 옛 술어(`!closed`)에서는 그
    /// 계약이 자리를 잃고, 그 빈자리를 본 다음 발송이 **아무도 표시하지 않은 채** 들어와, 첫 발송이 롤백할 때
    /// 희생자 표시 해제(+1)만 남아 513 이 고착됐다.
    #[test]
    fn a_closed_provisional_keeps_its_reserved_slot_until_its_guard_settles() {
        let cap = MAX_OPEN_REQUESTS;
        let now = t0();

        // 공용 픽스처: 상한을 **은퇴 가능**(기한 없음) 확정 계약으로 채운다.
        let build = || {
            let mut l = Ledger::new();
            for i in 0..cap {
                open_committed(&mut l, &format!("c{i}"), None, now);
            }
            assert_eq!(l.occupied_slots(), cap, "전제: 상한");
            l
        };

        // ── 롤백 갈래 ──────────────────────────────────────────────────────────────
        let mut l = build();
        // ① A: V1(=c0) 표시 + 잠정 PA 삽입.
        let OpenOutcome::OpenedAfterMarking(v1) =
            l.open_request("PA", "alice", sid(), "bob", None, None, now)
        else {
            panic!("전제: 표시 후 수용");
        };
        assert_eq!(v1.request_id, "c0");
        assert_eq!(l.occupied_slots(), cap, "표시+삽입 후에도 정확히 상한");
        // ② 빠른 회신이 PA 를 닫는다 — ★자리는 그대로 유지돼야 한다★.
        assert_eq!(l.close_on_reply("PA", now), ReplyOutcome::Closed);
        assert_eq!(
            l.occupied_slots(),
            cap,
            "닫힌 잠정 계약도 정산 전까지 자리를 지킨다(round-6 I1)"
        );
        // ③ B: 자리가 없으므로 **반드시 표시**하고 들어온다(옛 술어에선 표시 없이 들어왔다).
        let b = l.open_request("PB", "carol", sid(), "dave", None, None, now);
        assert!(
            matches!(b, OpenOutcome::OpenedAfterMarking(ref v) if v.request_id == "c1"),
            "B 도 자기 몫의 희생자를 표시해야: {b:?}"
        );
        assert_eq!(l.occupied_slots(), cap);
        // ④ A 롤백: 닫힌 PA 제거(-1) + V1 표시 해제(+1) → 정확히 상한.
        l.rollback_open(Some("PA"), Some("c0"));
        assert_eq!(
            l.occupied_slots(),
            cap,
            "롤백 뒤에도 정확히 상한 — 513 고착 없음(round-6 I1)"
        );
        // B 까지 정산하면 그 표시도 풀린다(전체 산술 확인).
        l.commit_open(Some("PB"), Some("c1"));
        assert_eq!(l.occupied_slots(), cap, "B 커밋 후에도 상한 유지");

        // ── 커밋 갈래 ──────────────────────────────────────────────────────────────
        let mut l = build();
        let OpenOutcome::OpenedAfterMarking(v1) =
            l.open_request("PA", "alice", sid(), "bob", None, None, now)
        else {
            panic!("전제");
        };
        assert_eq!(v1.request_id, "c0");
        assert_eq!(l.close_on_reply("PA", now), ReplyOutcome::Closed);
        assert_eq!(l.occupied_slots(), cap, "정산 전엔 자리 유지");
        // 커밋: 잠정 표시가 풀리며 **닫힌** PA 가 자리를 정당하게 놓고(회신을 실제로 받았다), V1 은 제거된다.
        l.commit_open(Some("PA"), Some("c0"));
        assert_eq!(
            l.occupied_slots(),
            cap - 1,
            "회신으로 끝난 계약은 커밋 시점에 자리를 놓는다(초과 아님 — 여유가 생긴 것)"
        );
        assert!(
            l.occupied_slots() <= MAX_OPEN_REQUESTS,
            "어느 갈래도 상한 초과 없음"
        );
    }

    /// ★round-6 I2 — 잠정 계약은 sweep 대상이 아니고, 커밋되면 **원래 시각으로** 곧바로 잡힌다★.
    ///
    /// 잠정 구간은 dispatch 를 감싸고 dispatch 는 자식 stdin write 를 한다 — 파이프 역압 아래에서 그 쓰기는
    /// 무한정 블록될 수 있다(stdio.rs). 그래서 "잠정 구간은 마이크로초" 라는 옛 가정이 깨지고, 1분 기한
    /// request 가 잠정 상태로 sweep 에 걸려 **반려될 요청에 대한 기한 통지**가 나갈 수 있었다.
    #[test]
    fn a_provisional_contract_is_not_swept_but_is_collected_right_after_commit() {
        let mut l = Ledger::new();
        let now = t0();
        let reply_by = Duration::from_secs(60);
        let over = now + reply_by + Duration::from_secs(1);
        // 잠정 상태 그대로(커밋 전) — 기한이 지나도 수집되지 않는다.
        l.open_request("p", "alice", sid(), "bob", None, rb(reply_by), now);
        assert!(
            l.due_timeouts(over).is_empty(),
            "잠정 계약은 sweep 대상이 아니다(round-6 I2)"
        );
        // 통지 플래그도 서지 않았다 — 나중에 정상적으로 잡힐 수 있어야 한다.
        assert!(l.open_requests().iter().all(|r| !r.notified));

        // 커밋 → 다음 sweep 이 **원래 created_at** 기준으로 즉시 수집한다(지연될 뿐 유실 없음).
        l.commit_open(Some("p"), None);
        let due = l.due_timeouts(over);
        assert_eq!(due.len(), 1, "커밋 후엔 곧바로 수집된다");
        assert_eq!(due[0].request_id, "p");
        assert_eq!(
            due[0].reply_by_raw,
            format!("{}s", reply_by.as_secs()),
            "표기 원본 그대로"
        );

        // 롤백 갈래: 계약 자체가 없었던 일이 되므로 통지도 없다.
        let mut l = Ledger::new();
        l.open_request("q", "alice", sid(), "bob", None, rb(reply_by), now);
        assert!(l.due_timeouts(over).is_empty());
        l.rollback_open(Some("q"), None);
        assert!(
            l.due_timeouts(over + Duration::from_secs(9999)).is_empty(),
            "반려된 요청엔 기한 통지가 없다"
        );
    }

    /// ★round-5 (1) — 동시 opener 는 **남의 잠정 계약**을 희생자로 고를 수 없다★.
    ///
    /// 옛 설계의 치명적 인터리빙: A 가 잠정 계약을 열어 둔 창에서 B 가 상한에 부딪히면, B 의 희생자 스캔이
    /// A 의 미확정 계약을 "가장 오래된 은퇴 가능" 으로 집어 없앴다 — A 는 **배달에 성공했는데 계약이 없는**
    /// 상태가 되고, 그 request 에 온 회신은 전부 `NoMatch` 로 빗나간다(발신자는 영원히 기다린다).
    #[test]
    fn a_concurrent_opener_cannot_select_someone_elses_provisional_entry_as_victim() {
        let mut l = Ledger::new();
        let base = t0();
        // cap 을 은퇴 가능(기한 없음) 계약으로 채우되, **가장 오래된 하나만** 남기고 나머지는 은퇴 불가로
        //   만든다 → B 의 유일한 합법 희생자가 그 하나임을 강제한다.
        let reply_by = Duration::from_secs(600);
        open_committed(&mut l, "evictable", None, base);
        for i in 1..MAX_OPEN_REQUESTS {
            let at = base + Duration::from_secs(i as u64);
            open_committed(&mut l, &format!("locked{i}"), rb(reply_by), at);
        }
        // A: 상한에 부딪혀 evictable 을 표시하고 자기 잠정 계약을 연다.
        let a_at = base + Duration::from_secs(1000);
        assert!(matches!(
            l.open_request("A", "alice", sid(), "bob", None, None, a_at),
            OpenOutcome::OpenedAfterMarking(ref r) if r.request_id == "evictable"
        ));

        // B: A 가 커밋/롤백하기 전에 들어온다. 남은 후보는 ① 표시된 evictable(제외) ② 기한 대기 중인
        //    locked*(제외) ③ **A 의 잠정 계약**(제외돼야 한다) → 고를 게 없으니 정직하게 Full.
        let b = l.open_request("B", "carol", sid(), "dave", None, None, a_at);
        assert_eq!(
            b,
            OpenOutcome::Full,
            "잠정 계약을 희생자로 고르면 안 된다 — 고를 게 없으면 Full 이 정답: {b:?}"
        );
        // ★A 의 계약은 멀쩡하다★ — 그리고 회신이 정상적으로 닫는다(옛 설계에선 NoMatch 였다).
        assert!(l.open_requests().iter().any(|r| r.request_id == "A"));
        assert_eq!(l.close_on_reply("A", a_at), ReplyOutcome::Closed);
    }

    /// ★round-5 (2) — 롤백은 계수를 정확히 되돌린다(513 영구 표류 없음)★.
    ///
    /// 옛 설계에선 B 가 A 의 잠정 계약을 은퇴시키면 A 의 롤백이 `drop_request`=NotFound 를 받고도 희생자를
    /// **무조건 되살려** 513개로 굳었다. 표시 방식에선 되살릴 게 없으므로(표시 해제뿐) 그 산술이 불가능하다.
    #[test]
    fn rollback_unmarks_and_drops_atomically_without_drift() {
        let mut l = Ledger::new();
        let now = t0();
        for i in 0..MAX_OPEN_REQUESTS {
            open_committed(&mut l, &format!("r{i}"), None, now);
        }
        // 반려 사이클을 반복해도 계약 수가 표류하지 않는다.
        for k in 0..50 {
            let provisional = format!("p{k}");
            let OpenOutcome::OpenedAfterMarking(victim) =
                l.open_request(&provisional, "alice", sid(), "bob", None, None, now)
            else {
                panic!("전제: 매 사이클 표시 후 수용");
            };
            assert_eq!(
                victim.request_id, "r0",
                "사이클 {k}: 늘 같은 최고령이 표시된다"
            );
            let dropped = l.rollback_open(Some(provisional.as_str()), Some("r0"));
            assert_eq!(
                dropped,
                Some(DropOutcome::Removed { notified: false }),
                "사이클 {k}: 잠정 계약이 같은 구간에서 제거됐다"
            );
            assert_eq!(
                l.open_request_count(),
                MAX_OPEN_REQUESTS,
                "사이클 {k}: 계약 수는 정확히 상한(513 로 새지 않는다)"
            );
            assert!(
                !l.open_requests()
                    .iter()
                    .any(|r| r.request_id == provisional),
                "사이클 {k}: 잠정 계약은 남지 않는다"
            );
            assert!(
                l.open_requests().iter().any(|r| r.request_id == "r0"),
                "사이클 {k}: 희생자는 표시만 지워진 채 그대로다"
            );
        }
        assert_eq!(l.tracking_len(), MAX_OPEN_REQUESTS, "추적 총량도 불변");
    }

    /// ★round-5 (3) — 표시 구간에 들어온 **정당한 회신**이 희생자를 제대로 닫는다★.
    ///
    /// 옛 설계에선 희생자가 목록 밖이라 그 회신이 `NoMatch` 로 빗나갔고, 뒤이은 롤백이 "열린 채" 되돌려
    /// 유령 상태를 남겼다(나중에 헛 기한 통지까지 날 수 있었다).
    #[test]
    fn a_reply_to_a_marked_victim_during_the_window_closes_it_properly() {
        let base = t0();
        let reply_by = Duration::from_secs(600);
        // (a) 롤백 갈래 — 닫힘이 유지되고 헛 통지가 없다.
        let mut l = Ledger::new();
        l.record("v", "alice", "bob", "q", DeliveryStatus::Pending, base);
        open_committed(&mut l, "v", rb(reply_by), base);
        assert_eq!(
            l.transition("v", "bob", DeliveryStatus::Delivered, base),
            Ok(())
        );
        // v 를 은퇴 가능하게 만든다(기한 초과 통지 발화) 뒤 cap 을 채운다.
        assert_eq!(
            l.due_timeouts(base + reply_by + Duration::from_secs(1))
                .len(),
            1
        );
        for i in 1..MAX_OPEN_REQUESTS {
            let at = base + Duration::from_secs(3600 + i as u64);
            open_committed(&mut l, &format!("f{i}"), None, at);
        }
        let win = base + Duration::from_secs(9000);
        assert!(matches!(
            l.open_request("new", "alice", sid(), "bob", None, None, win),
            OpenOutcome::OpenedAfterMarking(ref r) if r.request_id == "v"
        ));
        // ★표시 구간의 회신 — 정상적으로 닫힌다★.
        assert_eq!(
            l.close_on_reply("v", win),
            ReplyOutcome::Closed,
            "표시는 매칭을 가리지 않는다(round-5)"
        );
        // 롤백: 표시만 해제 → v 는 **닫힌 채** 남고 미결에서 빠진다(유령 재개방 없음).
        let tracked_before = l.tracking_len();
        l.rollback_open(Some("new"), Some("v"));
        assert_eq!(
            l.tracking_len(),
            tracked_before - 1,
            "잠정 계약 1건만 줄어야 — 롤백이 희생자를 **지우면** 안 된다(표시 해제뿐)"
        );
        assert!(
            l.msg_id_in_use("v"),
            "닫힌 희생자는 추적에 그대로 남는다(삭제 아님)"
        );
        assert!(
            !l.open_requests().iter().any(|r| r.request_id == "v"),
            "회신으로 닫혔으므로 미결이 아니다"
        );
        assert!(
            l.due_timeouts(win + Duration::from_secs(99999)).is_empty(),
            "닫힌 계약엔 헛 기한 통지가 나가지 않는다"
        );
        assert!(!l.open_requests().iter().any(|r| r.request_id == "new"));

        // (b) 커밋 갈래 — 닫힌 희생자를 제거하는 것도 안전하다(replied 는 종점).
        let mut l = Ledger::new();
        open_committed(&mut l, "v", None, base);
        for i in 1..MAX_OPEN_REQUESTS {
            let at = base + Duration::from_secs(i as u64);
            open_committed(&mut l, &format!("f{i}"), None, at);
        }
        assert!(matches!(
            l.open_request("new", "alice", sid(), "bob", None, None, win),
            OpenOutcome::OpenedAfterMarking(ref r) if r.request_id == "v"
        ));
        assert_eq!(l.close_on_reply("v", win), ReplyOutcome::Closed);
        l.commit_open(Some("new"), Some("v"));
        assert!(!l.open_requests().iter().any(|r| r.request_id == "v"));
        assert_eq!(
            l.open_request_count(),
            MAX_OPEN_REQUESTS,
            "커밋 후 계수는 정확히 상한"
        );
    }

    /// ★round-5 — 표시된 희생자·잠정 계약 모두 발급 충돌 검사에 **평소대로** 잡힌다★(옛 `reserved_ids`
    /// 기계가 필요 없어진 이유: 둘 다 목록을 떠나지 않는다).
    #[test]
    fn marked_and_provisional_entries_stay_visible_to_the_mint_collision_check() {
        let cap = 2;
        let mut l = Ledger::with_capacity(cap);
        let now = t0();
        for i in 0..MAX_OPEN_REQUESTS {
            let id = format!("r{i}");
            l.record(&id, "alice", "bob", "q", DeliveryStatus::Pending, now);
            open_committed(&mut l, &id, None, now);
        }
        // r0 의 이력 행을 링에서 밀어낸다 — 이력 축으로는 안 보이게 만든다.
        for j in 0..cap {
            l.record(
                &format!("f{j}"),
                "alice",
                "bob",
                "x",
                DeliveryStatus::Delivered,
                now,
            );
        }
        assert!(l.records_for("r0").is_empty(), "전제: 이력 evict");

        assert!(matches!(
            l.open_request("new", "alice", sid(), "bob", None, None, now),
            OpenOutcome::OpenedAfterMarking(ref r) if r.request_id == "r0"
        ));
        assert!(
            l.msg_id_in_use("r0"),
            "표시된 희생자는 추적에 그대로 있어 사용 중으로 보인다"
        );
        assert!(l.msg_id_in_use("new"), "잠정 계약도 사용 중");
        // 커밋 뒤에야 희생자 id 가 풀린다(이력도 없으므로 완전히 미사용).
        l.commit_open(Some("new"), Some("r0"));
        assert!(!l.msg_id_in_use("r0"));
        assert!(l.msg_id_in_use("new"), "확정된 계약은 계속 사용 중");
    }

    /// ★H4 — 조회는 문서가 약속한 대로 **오래된 순**이다(목록 위치와 무관)★.
    ///
    /// ★왜 위치를 못 믿나★: 이 장부는 시계를 **주입받는다**(모듈 헤더 순수성 불변식) — 즉 "추가 순서 =
    ///   시각 순서" 는 호출자가 단조 시계를 쓸 때만 참인 **가정**이지 이 자료구조가 강제하는 성질이 아니다.
    ///   그래서 Vec 순서와 `created_at` 순서는 갈릴 수 있다. 문서가 약속한 순서는 코드가 지켜야 하므로
    ///   조회가 직접 정렬한다.
    #[test]
    fn open_requests_are_sorted_oldest_first_regardless_of_list_position() {
        let mut l = Ledger::new();
        let base = t0();
        // 주입 시계를 **역순**으로 준다(장부 계약상 허용되는 입력) — Vec 순서 ≠ 시각 순서를 만든다.
        open_committed(&mut l, "late", None, base + Duration::from_secs(300));
        open_committed(&mut l, "middle", None, base + Duration::from_secs(100));
        open_committed(&mut l, "early", None, base);

        let ids: Vec<String> = l
            .open_requests()
            .into_iter()
            .map(|r| r.request_id)
            .collect();
        assert_eq!(
            ids,
            vec!["early", "middle", "late"],
            "추가 순서(late→middle→early)와 무관하게 오래된 순이어야(H4)"
        );
        let times: Vec<_> = l
            .open_requests()
            .into_iter()
            .map(|r| r.created_at)
            .collect();
        assert!(
            times.windows(2).all(|w| w[0] <= w[1]),
            "created_at 오름차순"
        );
    }

    /// ★H4 의 짝 — 은퇴 표시 해제 뒤에도 조회 순서가 유지된다★.
    #[test]
    fn open_requests_stay_sorted_after_a_marked_retirement_is_rolled_back() {
        let mut l = Ledger::new();
        let base = t0();
        for i in 0..MAX_OPEN_REQUESTS {
            let at = base + Duration::from_secs(i as u64);
            open_committed(&mut l, &format!("r{i}"), None, at);
        }
        let OpenOutcome::OpenedAfterMarking(victim) = l.open_request(
            "new",
            "alice",
            sid(),
            "bob",
            None,
            None,
            base + Duration::from_secs(9999),
        ) else {
            panic!("전제");
        };
        assert_eq!(victim.request_id, "r0", "가장 오래된 것이 희생자");
        l.rollback_open(Some("new"), Some(victim.request_id.as_str()));
        let times: Vec<_> = l
            .open_requests()
            .into_iter()
            .map(|r| r.created_at)
            .collect();
        assert!(
            times.windows(2).all(|w| w[0] <= w[1]),
            "복원 뒤에도 오래된 순: {times:?}"
        );
        assert_eq!(
            l.open_requests().first().map(|r| r.request_id.clone()),
            Some("r0".to_string()),
            "복원된 최고령 계약이 다시 맨 앞"
        );
    }

    /// ★F2 — 실제 배달된 수신자로 계약이 다시 묶인다★(상위가 flush 착지 시점에 부른다).
    #[test]
    fn rebind_request_recipient_moves_the_obligation_to_the_actual_deliveree() {
        let mut l = Ledger::new();
        let now = t0();
        let a = sid();
        let b = sid();
        l.record("r1", "boss", "worker", "q", DeliveryStatus::Pending, now);
        l.open_request("r1", "boss", sid(), "worker", Some(a), None, now);
        // 전제: 봉투가 실제로 꽂혀 이력이 delivered 다(회신이 정상 간선을 타게).
        assert_eq!(
            l.transition("r1", "worker", DeliveryStatus::Delivered, now),
            Ok(())
        );
        assert_eq!(l.open_requests()[0].recipient_id, Some(a));
        // 이름 큐 flush 가 동명 B 에게 꿂았다 → 의무도 B 로 옮겨진다.
        l.rebind_request_recipient("r1", b);
        assert_eq!(
            l.open_requests()[0].recipient_id,
            Some(b),
            "의무는 봉투를 실제로 받은 자를 따른다(F2)"
        );
        // 닫힌 계약은 건드리지 않는다(이력 오염 방지).
        assert_eq!(l.close_on_reply("r1", now), ReplyOutcome::Closed);
        l.rebind_request_recipient("r1", a);
        assert!(l.open_requests().is_empty(), "닫힌 계약은 미결이 아니다");
        // 없는 id 는 no-op(통보·notice 경로가 그냥 부른다).
        l.rebind_request_recipient("nope", a);
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
        l.open_request("r1", "a", sid(), "b", None, None, now);
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

    // ── S18 D: open_requests(미결 조회 뷰) ────────────────────────────────────────────────

    #[test]
    fn open_requests_lists_unanswered_contracts_oldest_first_with_notation() {
        let mut l = Ledger::new();
        let now = t0();
        l.record("r1", "alice", "bob", "q1", DeliveryStatus::Pending, now);
        l.open_request(
            "r1",
            "alice",
            sid(),
            "bob",
            None,
            rb(Duration::from_secs(600)),
            now,
        );
        let later = now + Duration::from_secs(5);
        l.record("r2", "carol", "alice", "q2", DeliveryStatus::Pending, later);
        l.open_request("r2", "carol", sid(), "alice", None, None, later);

        let open = l.open_requests();
        assert_eq!(open.len(), 2);
        assert_eq!(open[0].request_id, "r1", "오래된 순");
        assert_eq!(open[0].sender, "alice");
        assert_eq!(open[0].recipient, "bob");
        assert_eq!(
            open[0].reply_by_raw.as_deref(),
            Some("600s"),
            "표기는 발신자 원본 그대로(역산 금지)"
        );
        assert_eq!(
            open[0].created_at, now,
            "발송 시각 그대로(장부는 벽시계 모름)"
        );
        assert!(!open[0].notified);
        assert_eq!(open[1].request_id, "r2");
        assert_eq!(open[1].reply_by_raw, None, "기한 없는 request");
    }

    #[test]
    fn open_requests_drops_replied_but_keeps_timed_out_ones() {
        // ★핵심 구분★: 회신으로 닫힌 계약은 미결이 아니다(빠진다). 반면 **기한 초과 통지가 나간** 계약은
        //   여전히 회신을 기다리므로 목록에 남고, notified 플래그로만 구분된다 — 빼면 "답할 게 남았는데
        //   목록엔 없는" 상태가 된다(is_live() 기준을 쓰면 그렇게 된다).
        let mut l = Ledger::new();
        let now = t0();
        let d = Duration::from_secs(600);
        open_delivered_request(&mut l, "replied", None, now);
        open_delivered_request(&mut l, "timedout", rb(d), now);
        assert_eq!(l.close_on_reply("replied", now), ReplyOutcome::Closed);
        assert_eq!(l.due_timeouts(now + d + Duration::from_secs(1)).len(), 1);

        let open = l.open_requests();
        assert_eq!(open.len(), 1, "회신으로 닫힌 계약은 빠진다: {open:?}");
        assert_eq!(open[0].request_id, "timedout");
        assert!(
            open[0].notified,
            "통지 나간 사실은 필드로 노출(목록에서 제외하지 않는다)"
        );
    }

    #[test]
    fn open_requests_is_empty_without_any_contract() {
        let mut l = Ledger::new();
        let now = t0();
        // 통보만 있는 장부 — request 추적이 없으므로 미결도 없다.
        l.record("m1", "alice", "bob", "hi", DeliveryStatus::Delivered, now);
        assert!(l.open_requests().is_empty());
    }
}
