//! ledger — 메시지 이력 + request 회신 추적 + 그룹 배달 장부(spec §2·§3·§5).
//!
//! ★역할★: 세 축을 담는다.
//!   ① **이력 링버퍼** — 전 메시지의 상태 전이 + 시각(`pending→delivered→replied` / `expired` / `skipped`).
//!      "상태 전이 시각이 곧 회신·발신 시각 데이터"(봉투 미노출 — spec §5).
//!   ② **request 추적** — `awaiting_reply` 오픈 + `in_reply_to` **엄격 매칭**으로 닫기 + `reply_by` 초과
//!      타임아웃 산출(발신자에게 notice 는 후속 increment 가 생성 — 여기선 "누가 초과했나"만 산출).
//!      ★종결 사유는 둘(4차 — ADR-0116 결정 2/3 · ADR-0118 결정 1)★: 회신 수용(`replied`) 또는 **실패
//!      종결**(`reply_failed` — 회신 발송이 `RECIPIENT_NOT_FOUND` / 요청자 프로필 삭제 정리). 기한 초과는
//!      여전히 **종결 사유가 아니다**(ADR-0108 결정 1 — 통지 ≠ 종결).
//!   ③ **그룹 배달 장부** — 메시지 1 : 배달기록 N(spec §4). 죽은 멤버 `skipped` 지원.
//!
//! ★순수·주입 시계(load-bearing — 모듈 헤더 불변식)★: 상태 전이·타임아웃 판정의 모든 시각은 `now: Instant`
//!   를 인자로 받는다. 링버퍼·추적 맵에 시계가 없다 — TTL·reply-by 경계를 결정적 단위 테스트로 단언한다.
//!
//! ★엄격 회신 매칭(load-bearing — spec §2 · ADR-0103 불변식)★: 회신은 `in_reply_to` 가 **정확히** 오픈된
//!   request id 를 가리킬 때만 그 request 를 닫는다. 관대 매칭(미회신 상대의 다음 메시지를 회신 간주)은
//!   우연 닫힘 오발이라 거부됐다 — 틀린 id 는 아무 것도 닫지 않는다.
// ADR-0103

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use crate::PeerId;

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
/// ★evict 와 request 추적의 관계(C3 리뷰 fix 3 로 좁혀짐)★: 예전엔 이력 evict 가 추적 항목을
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

/// 메시지 배달 1건의 상태(spec §5·§6 상태 어휘 — 새 어휘 발명 금지).
///
/// ★상태 전이(load-bearing)★: `Pending → Delivered → Replied`(request 만) / `Expired`(TTL) / `Skipped`
///   (notice 레인 은퇴) / `Failed`.
///   각 전이는 시각을 남긴다(spec §5 "상태 전이 시각이 곧 회신·발신 시각"). busy 대기·주입 실패 파킹·
///   **잠듦 파킹**(ADR-0116 결정 1)은 전부 `Pending`(상태 어휘 공유 — spec §5 분기 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryStatus {
    Pending,
    /// 실제 주입 완료(delivered = 실제 주입 시점, ADR-0104 불변식).
    Delivered,
    Replied,
    /// TTL(24h) 초과로 파킹 만료(장부 잔존, spec §5, ADR-0105).
    Expired,
    /// ★장부 전용 종점★ — notice 레인(`NOTICE_CAP`) 초과로 가장 오래된 통지가 은퇴됨(spec §6 대응표).
    ///   ★발송 응답 어휘에는 없다★(ADR-0111 결정 3 으로 그룹 멤버 skip 이 사라지며 응답 축에서 폐지).
    Skipped,
    /// ★수신자별 실패 종점(신설 — ADR-0111 결정 3 · spec §5 "실패 수신자도 장부에 남는다")★:
    ///   입구 반려(`RECIPIENT_NOT_FOUND`/`RECIPIENT_AMBIGUOUS`) ·
    ///   `MAILBOX_FULL` · `REQUEST_CAPACITY` · **삭제 정리**(`RECIPIENT_DELETED` — 4차).
    ///
    /// ★왜 장부에 남기나(load-bearing)★: 발신자가 나중에 `messages{id}` 로 "누가 못 받았나" 를 다시 볼 수
    ///   있어야 하고, 그래야 **장부 기대 행수 = 발신 응답 행수**가 맞아 `may_be_truncated` 오탐이 사라진다
    ///   (spec §5·§6). 파킹은 없지만 기록은 있다.
    /// ★기록 시점이 곧 종점 — **단 하나의 예외**(4차 · ADR-0116 결정 4)★: 입구 반려 계열은 이 상태로
    ///   `record` 되고 어떤 전이도 나가지 않는다. 삭제 정리만 *이미 파킹된* 레코드를 사후 종결하므로
    ///   `Pending → Failed` 한 간선을 쓴다.
    // ADR-0111
    // ADR-0116 (pending→failed = 삭제 정리 한정)
    Failed,
}

impl DeliveryStatus {
    /// 이 상태에서 `next` 로의 전이가 **합법**인가(spec §5 상태 전이 그래프 — load-bearing).
    ///
    /// 그 밖의 모든 간선은 불법이다 — 특히 **terminal**(`Replied`/`Expired`/`Skipped`/`Failed`)에서의
    ///   재전이, 그리고 되돌림(`*→Pending`)·건너뜀(`Pending→Replied`, `Expired→Delivered` 등)은 거부한다.
    ///   되돌림·건너뜀을 허용하면 "상태 전이 시각 = 회신·발신 시각" 이라는 장부 의미가 오염된다(오발 닫힘·
    ///   시각 소급). 같은 상태로의 자기 전이도 불법(무의미한 시각 갱신 방지)이다.
    /// ★`Failed` 로 들어오는 간선은 이 **범용** 그래프에 없다(ADR-0111 · 4차에도 유지)★: 실패 행은
    ///   **처음부터** 그 상태로 기록된다. 범용 전이로 열어 두면 "배달됐다가 실패로 돌아간" 불가능한 이력이
    ///   표현 가능해지므로, 4차에서 추가된 `Pending → Failed`(삭제 정리)도 여기에 넣지 않고 **전용 동사**
    ///   (`fail_pending` → `can_fail_by_cleanup`)로만 연다.
    // ADR-0116 (pending→failed 는 경로 한정 — 범용 그래프 불변)
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

    /// ★삭제 정리 전용 간선(`Pending → Failed`, spec §6 · ADR-0116 결정 4)★ — 이 상태에서 **삭제 정리로**
    /// `Failed` 로 갈 수 있나.
    ///
    /// ★왜 별도 술어인가(load-bearing)★: 이 간선은 "이미 파킹된 레코드를 사후 종결하는 유일한 경로" 에만
    ///   합법이다. 범용 그래프(`can_transition_to`)에 넣으면 **어떤 호출자든** pending 행을 실패로 만들 수
    ///   있어(예: 만료 스윕 버그·flush 오분기) "배달 대기 중이던 메일이 사유 없이 실패로 적힌" 이력이
    ///   생긴다. 술어를 갈라 두면 그 능력이 `fail_pending` 한 동사에만 있고, `Delivered → Failed` 는
    ///   두 술어 어디에도 없어 영구히 불법이다.
    fn can_fail_by_cleanup(self) -> bool {
        matches!(self, DeliveryStatus::Pending)
    }
}

/// 이력 레코드 1건 — 한 (메시지, 수신자) 쌍의 배달 이력.
///
/// ★메시지 1 : 배달기록 N(spec §4 · load-bearing)★: 그룹 발송은 하나의 논리 메시지(`msg_id` 공유)를 여러
///   수신자에게 개별 배달하므로, 배달 레코드는 **수신자별로 하나**다(각자 status·시각 독립). 단일 발송은 N=1.
/// ★body 는 요약이 아니라 full 보관(설계 결정)★: 인메모리 단계라 별도 저장소가 없고, 파킹된 봉투 재주입·
///   장부 조회(`messages { id }` — spec §6)에 원문이 필요하다. 요약본만 두면 재주입·감사 때 원문 손실이다.
///   메모리는 링 용량(HISTORY_CAPACITY)이 상한 — v2 영속화(SQLite) 때 요약/오프로드를 재검토한다(무파괴).
#[derive(Debug, Clone)]
pub struct MessageRecord {
    pub msg_id: String,
    /// 발신자 이름(WYSIWYA — ADR-0101).
    pub from: String,
    pub to: String,
    pub body: String,
    pub status: DeliveryStatus,
    pub created_at: Instant,
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
    /// ★조회에 실을 실패 코드(4차 신설 — spec §6 `RECIPIENT_DELETED`)★.
    ///
    /// ★왜 wire 문자열을 그대로 담나(load-bearing)★: 이 코드가 처음 보이는 곳은 **발송 응답이 아니라
    ///   `messages{id}` 조회**다(발송 시점엔 `pending` 이었다 — spec §6). 즉 값을 발송 응답과 다른 시점까지
    ///   **보관**해야 하므로 레코드에 실린다. 어휘 정본은 `service::FailCode::as_str` 이고 여기엔 그 반환값이
    ///   그대로 들어온다 — 장부가 service 의 enum 을 타입으로 알면 순수 저장 계층이 정책 어휘에 유착되므로
    ///   `&'static str` 한 겹으로만 받는다(값 복제 없음).
    /// ★지금 채워지는 경로는 삭제 정리 하나뿐★: 입구 반려 계열의 `failed` 행은 발송 응답이 그 자리에서
    ///   code/hint 를 이미 전달했으므로(spec §6) 조회 축에 중복 보관하지 않는다(`None`).
    // ADR-0116 (RECIPIENT_DELETED — 조회 시점 코드)
    pub fail_code: Option<&'static str>,
}

/// 오픈된 request 추적 1건(spec §3). 이력 레코드와 **별도 맵**이라 링 evict 에 영향받지 않는다.
///
/// ★notified 플래그(load-bearing — 이중 통지 방지)★: `reply_by` 초과가 `due_timeouts` 로 한 번 보고되면
///   이 플래그를 세워 **다시 보고하지 않는다**(spec §7 "no double-notification"). 회신이 오면 `closed` 라
///   `due_timeouts` 대상에서 빠진다(replied 는 절대 due 로 안 나옴).
// R1: `reservation_token`(Weak)은 값 비교의 의미가 없어 PartialEq/Eq 파생을 뺐다(아무도 쓰지 않았다).
#[derive(Debug, Clone)]
struct RequestEntry {
    request_id: String,
    /// **발송 시점의** 표시 이름이다.
    sender: String,
    /// ★요청 발신자의 PeerId(C3 리뷰 fix 2 — load-bearing)★: 이름은 발송 후 바뀔 수 있고(display_name
    ///   변경), 그러면 이름-키 파킹만으로는 notice 가 옛 이름 큐에 갇혀 **영영 배달되지 않는다**(통지는
    ///   `notified` 라 재발화도 없다 = 계약이 조용히 반쪽). id 를 함께 들고 있으면 상위가 그걸 파킹 힌트로
    ///   실어 이름과 무관하게 그 incarnation 으로 배달할 수 있다.
    sender_id: PeerId,
    recipient: String,
    /// ★해석된 수신자 PeerId(D 리뷰 B1 — load-bearing)★: 발송 시점에 수신자가 **산 에이전트로 해석됐으면**
    /// 그 PeerId.
    /// ★`None` 은 4차부터 **운영 경로에 다시 존재한다**(ADR-0116 결정 1 — 잠듦 파킹 부활)★: 잠든(프로필만
    /// 있는) 수신자에게 건 request 는 계약을 열지만 그 순간 산 incarnation 이 없어 id 를 못 붙인다. 그
    /// 계약은 **이름으로** 의무를 귀속하다가(아래 폴백), 복원 후 실제 배달 시점에
    /// `rebind_request_recipient` 가 착지 incarnation 의 id 를 박는다.
    ///
    /// ★왜 이름만으로는 안 되나★: 같은 이름의 산 에이전트가 둘일 때(동명 다수) 발신자는 exact PeerId 로
    ///   지목해 한쪽에만 request 를 보낼 수 있는데, 계약은 이름(`recipient`)으로만 기록됐다 — 그러면
    ///   **메시지를 받은 적도 없는 쌍둥이**가 미결 조회에서 그 의무를 자기 것으로 본다(잘못된 의무 귀속).
    ///   id 를 함께 붙들면 "누가 답해야 하나" 를 정확히 가를 수 있다.
    /// ★epoch 는 담지 않는다★: 같은 에이전트의 재시작은 PeerId 를 유지하고 epoch 만 올린다(ADR-0007) —
    ///   재시작한 그 에이전트는 여전히 답할 주체이므로 epoch 로 좁히면 의무가 부당하게 사라진다.
    /// ★None 의 의미 = 이름 폴백★: 아직 뜨지 않은 이름 앞으로 건 request 는 나중에 그 이름으로 등장한
    ///   에이전트가 답할 주체다(WYSIWYA — ADR-0101). 그래서 id 가 없으면 이름으로만 매칭한다.
    // 리뷰 B1
    recipient_id: Option<PeerId>,
    /// 회신 기한 = (발송 기준 오프셋, **발신자가 쓴 표기 원본**). `None` = 기한 없음(타임아웃 없음).
    ///
    /// ★왜 표기를 함께 보관하나(C3 리뷰 fix 6)★: 예전엔 Duration 만 두고 통지 문구를 만들 때 상위가 표기를
    ///   **역산**했다 — 그 역산이 정규화라 `60m` 로 보낸 기한이 `1h` 로 통지돼 봉투(`reply-by="60m"`)와
    ///   문구가 어긋났다. 계약 문구는 발신자가 쓴 그대로여야 하므로 표기를 원본째 보관한다(둘을 한 튜플로
    ///   묶어 "기한이 있으면 표기도 반드시 있다" 를 타입으로 강제한다).
    reply_by: Option<(Duration, String)>,
    created_at: Instant,
    closed: bool,
    /// ★실패 종결 표식(4차 신설 — ADR-0116 결정 2/3 · ADR-0118 결정 1)★. `closed` 와 **항상 함께** 세워지고,
    /// 이 값이 `true` 면 종점 어휘가 `replied` 가 아니라 `reply_failed` 다(계약 축 — spec §6).
    ///
    /// ★왜 `closed` 옆의 한 비트인가(load-bearing)★: 종결의 **결과**(오픈 목록 제거·기한 스윕 미발화·512
    ///   계수 제외)는 `replied` 와 완전히 같다 — 그래서 그 판정들을 두 벌로 만들지 않고 `closed` 단일 술어를
    ///   그대로 쓴다(ADR-0118 결정 4 "`occupied_slots` 단일 술어 유지"). 이 비트는 **사유**만 나른다:
    ///   "회신이 성립했다" 를 주장하지 않는 종점이라는 사실(§6 축 구분)과, 삭제 정리가 **이미 `replied` 인
    ///   계약은 건드리지 않는다**(§3 항목 7-④ 되돌림 금지)는 규칙의 관측면이다.
    /// ★발화 경로는 딱 둘★: ① 회신 발송 행이 `RECIPIENT_NOT_FOUND`(`fail_on_undeliverable_reply`)
    ///   ② 요청자 프로필 삭제 정리(`fail_open_requests_from`). 그 밖의 회신 실패는 **무동작**이다.
    // ADR-0116 (결정 2) / ADR-0118 (결정 1·4)
    reply_failed: bool,
    notified: bool,
    /// ★상한 압력으로 **은퇴 예정 표시**됨(round-5 mark-and-sweep)★ — 아직 목록에 살아 있고 회신도 받을 수
    /// 있다. 커밋 때 비로소 물리 제거되고, 롤백이면 표시만 지워져 아무 일도 없던 상태로 돌아간다.
    ///
    /// ★왜 표시인가(물리 제거를 버린 이유 — load-bearing)★: 예전 설계는 예약 시점에 희생자를 목록에서
    ///   **꺼냈다**. 그 창 동안 그 계약은 세상에 없는 것처럼 굴어서 ① 정당한 회신이 `close_on_reply` 에서
    ///   `NoMatch` 로 빗나가고 ② 롤백이 "열린 채" 되돌려 유령 상태·헛 통지를 만들었다. 꺼내지 않고 표시만
    ///   하면 그 창 자체가 없다 — 회신·조회·중복검사 전부 평소 경로로 계속 동작한다.
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
    /// ★이 예약이 **은퇴 예정 표시를 붙인 희생자**의 계약 키(F1)★ — `Some((msg_id, recipient))`.
    ///
    /// ★왜 기억하나(load-bearing)★: 정산 없이 남은 예약을 **주기 sweep 이 스스로 회수**할 수 있어야 한다
    ///   (RAII Drop 은 `try_lock` 이 실패하면 아무 것도 못 한다 — `service::Reservation` 헤더). 그때 sweep 은
    ///   "이 예약이 누구에게 표시를 붙였나" 를 알아야 그 표시만 정확히 풀 수 있다. 호출자가 들고 다니던
    ///   값(`RetiredContract`)은 예약이 유실되면 함께 사라지므로, 장부 안에 남겨 둔다.
    marked_victim: Option<(String, String)>,
    /// ★이 예약의 **소유자 생존 토큰**(R1)★ — `Some(weak)` = 가드가 붙어 있다, `None` = 붙은 적 없다.
    ///
    /// sweep 의 회수 기준이다(`reclaim_abandoned_reservations`): upgrade 되면 소유자가 아직 일하는 중이므로
    /// **건드리지 않는다**. 근거·거부한 대안(시간 임계값)은 `ReservationLiveness` 헤더.
    /// ★`None` 이 회수 대상인 이유(fail-safe)★: 부착은 `open_request` 와 **같은 락 구간**에서 일어난다
    ///   (운영 pass A). 즉 sweep 이 `None` 인 잠정 항목을 볼 수 있다는 건 여는 쪽이 가드를 만들지 않고 락을
    ///   놓았다는 뜻 = 아무도 정산하지 않을 예약이다. 그 부류를 남겨 두면 cap 분모가 영구히 줄어든다.
    // R1
    reservation_token: Option<Weak<()>>,
}

impl RequestEntry {
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
    /// ★`reply_failed` 도 여기서 **자동으로 빠진다**(ADR-0118 결정 4 — 두 번째 용량 개념 금지)★: 실패 종결은
    ///   `closed` 를 세우므로 이 술어가 그대로 false 를 낸다. 별도 분기를 추가하지 말 것 — 상한 계수는
    ///   단일 술어라는 게 이 함수의 존재 이유다(정산 산술이 여기 한 곳에서만 성립한다).
    // round-5 mark-and-sweep / round-6 I1
    // ADR-0108 (용량 술어 단일점 — 잠정은 닫혀도 무게 유지)
    // ADR-0118 (결정 4 — reply_failed 는 계수에서 빠진다)
    fn occupies_slot(&self) -> bool {
        (!self.closed || self.provisional) && !self.pending_retirement
    }
}

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
    /// 이미 닫힌 request 에 대한 두 번째 회신 — no-op(첫 회신이 이미 계약을 이행했다).
    AlreadyClosed,
}

/// ★회신 실패 종결 시도의 결말(4차 신설 — spec §3 항목 7-④ · ADR-0118)★.
///
/// ★`Failed` 외 셋은 전부 **무동작**이다★ — 계약은 그대로 오픈이거나 이미 종결이다. 호출자는 결말별로
///   로그만 갈린다(배달·응답에는 영향 없음 — 회신 메시지 자체의 결말은 이미 실패 행으로 보고됐다).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyFailOutcome {
    /// 계약을 `reply_failed` 종점으로 닫았다(오픈 목록·기한 스윕·512 계수에서 제거, 이력 잔존).
    Failed,
    /// 그 회신자의 오픈 계약이 없다 — 모르는 id·남의 계약(정상 경로, `close_on_reply` 의 `NoMatch` 와 동형).
    NoMatch,
    /// 이미 종결된 계약(`replied` 이거나 이미 `reply_failed`) — 되돌리지 않는다.
    AlreadyClosed,
    /// ★가드 보유 중(ADR-0118 결정 3)★ — 잠정·은퇴 예정 표시가 붙어 있어 수명이 그 가드 소유다. 무동작.
    GuardHeld,
}

/// ★요청자 삭제 정리가 계약 축에서 한 일(spec §5 삭제 정리 ②)★ — 호출자가 **락 밖에서** 로그로 남긴다.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RequesterCleanup {
    /// `reply_failed` 로 종결한 계약 키 `(request_id, recipient)`.
    pub failed: Vec<(String, String)>,
    /// 가드(잠정·은퇴 예정) 때문에 건너뛴 계약 수 — 재발화하지 않는다(삭제 시점 단발, spec §5).
    pub guard_held: usize,
}

/// ★왜 typed 에러인가(load-bearing)★: 예전 `transition` 은 `bool`(성공/미존재)만 냈고 **불법 전이를 조용히
///   수행**했다(`Expired → Delivered` 같은 되돌림·건너뜀 허용). 이는 "상태 전이 시각 = 회신·발신 시각"
///   장부 의미를 오염시킨다. 이제 불법 전이는 타입으로 거부해 상위가 버그를 즉시 감지한다(spec §5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionError {
    /// (msg_id, to) 레코드가 없음 — evict 됐거나 미존재.
    NotFound,
    Illegal {
        from: DeliveryStatus,
        to: DeliveryStatus,
    },
}

/// ★상한 압력으로 **은퇴 예정 표시**된 계약 1건(round-2 리뷰 F1 · 사용자 결정 2026-07-27)★ — 호출자가
/// **락 밖에서** 계측 로그를 남길 재료다(조용한 소멸 금지).
///
/// ★언제 생기나★: 미회신 계약이 `MAX_OPEN_REQUESTS` 에 닿았는데 새 request 가 들어왔고, 추적 목록에
///   **은퇴 가능한**(= 발신자에게 남은 통지 약속이 없는) 계약이 있을 때. 그 중 가장 오래된 하나가 자리를
///   내준다 — 메일박스·notice 레인이 cap 에서 "가장 오래된 것을 은퇴" 시키는 것과 같은 패턴이다.
///
/// ★값만 담는다(원본 항목을 들고 다니지 않는다)★: 희생자는 목록에서 나간 적이 없으므로 되돌릴 상태가
///   없다 — 롤백은 그 항목의 표시를 지우기만 하면 된다. 그래서 이 구조체는 "무엇이 은퇴하려 했나" 를
///   사람이 읽을 수 있게 나르는 것 이상을 하지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetiredContract {
    pub request_id: String,
    pub sender: String,
    pub recipient: String,
    /// 표시 시점 기준 나이(로그용 — 벽시계가 아니라 경과).
    pub age: Duration,
}

/// ★예약 생존 토큰(R1 — 시간 임계값을 **생존 판정**으로 교체)★. 강한 쪽은 `service::Reservation` 가드가
/// 들고, 장부는 약한 쪽(`Weak`)만 본다. 가드가 살아 있으면 upgrade 가 성공하고, 정산 없이 소멸하면 실패한다.
///
/// ★왜 시간이 아니라 생존인가(load-bearing — 이 설계가 제거하는 실패 모드)★: 예약은 "락 안 판정 → 락 밖
///   주입 → 락 안 정산" 구간을 산다. 그 **주입은 유계가 아니다** — 자식 stdin 은 backpressure 로 무한히
///   블록될 수 있다(agent `stdio.rs` 참조). 그래서 "N초 넘으면 버려진 것" 이라는 어떤 상수도 틀릴 수 있고,
///   틀리는 방식이 최악이었다: sweep 이 **아직 일하고 있는** 예약을 회수해 버리면 그 뒤 주입이 성공하고
///   `commit` 이 돌아도 계약이 이미 없다 → `type="request"` 봉투는 배달됐는데 계약이 없다(발신자
///   `awaiting_their_reply` 0 · 수신자 `reply_owed_by_me` 0 · 기한 통지 영원히 없음 · 나중에 온 정당한 회신은
///   `NoMatch` 라 이력 행이 영구히 `Delivered` = 감사 기록이 "답 없음" 이라 거짓말한다). 생존 판정은 그
///   경쟁 자체를 **구조적으로** 없앤다 — 소유자가 살아 있는 동안 sweep 은 그 예약을 볼 자격이 없다.
/// ★임계값 상수는 삭제됐다★: 되살리지 말 것(위 근거 = 무계 주입).
// ADR-0108 (mark-and-sweep — 회수 기준 = 생존)
#[derive(Debug)]
pub struct ReservationLiveness(Arc<()>);

impl ReservationLiveness {
    pub fn new() -> Self {
        Self(Arc::new(()))
    }

    pub fn watch(&self) -> Weak<()> {
        Arc::downgrade(&self.0)
    }
}

impl Default for ReservationLiveness {
    fn default() -> Self {
        Self::new()
    }
}

/// ★커밋이 **실제로 한 일**(R2)★ — 계획이 아니라 사실이다.
///
/// ★왜 필요한가(load-bearing — 계측 오염)★: 옛 `commit_open` 은 반환이 없었고 호출자는 "계획한 은퇴가
///   일어났다" 고 **가정해** 계측 로그를 찍었다. 그런데 표시된 희생자는 커밋 전에 사라질 수 있다(그 사이
///   회신으로 닫히고 이력 행까지 링에서 밀려나면 `purge_finished_without_history` 가 정리한다 —
///   `rollback_open` 주석의 알려진 잔여). 그러면 **일어나지 않은 은퇴**가 보고돼, ADR-0108 결정 2 가 은퇴의
///   유일한 증거라고 못 박은 축이 오염된다(운영자가 유령 은퇴를 쫓는다).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommitOutcome {
    /// 잠정 표시를 실제로 지웠나(= 그 계약이 아직 목록에 있었나).
    pub confirmed: bool,
    /// 표시된 희생자를 **실제로 물리 제거**했나 = 은퇴가 실제로 일어났나.
    pub retired: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenOutcome {
    Opened,
    /// 새 request 를 (잠정으로) 열되, 상한 압력으로 **가장 오래된 은퇴 가능 계약**에 은퇴 예정 표시를 했음.
    ///
    /// ★호출자 의무(round-5 mark-and-sweep · load-bearing)★: 표시는 **잠정**이다. 호출자는 반드시 둘 중
    ///   하나를 부른다 — ① 발송 접수 시 `commit_open`(표시된 희생자를 물리 제거 + 잠정 표시 해제, 그때
    ///   계측 로그) ② 그 외 모든 이탈(반려·패닉) 시 `rollback_open`(표시 해제 + 잠정 계약 제거).
    ///   ★②의 구조적 보장 소유자 = `service::Reservation`(RAII 가드)의 Drop★(ADR-0108 결정 3). 그 값이
    ///   정산 없이 소멸하면 debug 빌드는 즉시 터지고 릴리즈는 error 로그와 함께 롤백을 시도한다 — 라운드 1의
    ///   평범한 `Option<Option<_>>` 은 **잊은 갈래를 아무도 잡지 못했다**(리뷰 prober 실측).
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueTimeout {
    pub request_id: String,
    /// notice 를 받을 발신자 이름(**발송 시점** 표시 이름 — 그 뒤 개명됐을 수 있다).
    pub sender: String,
    /// ★notice 배달용 발신자 id(C3 리뷰 fix 2)★ — 이름이 바뀌었어도 이 id 로 배달 경로를 찾는다
    ///   (`RequestEntry.sender_id` 주석). 상위가 파킹 힌트 + flush 도어벨 대상으로 쓴다.
    pub sender_id: PeerId,
    pub recipient: String,
    /// ★초과된 기한의 **표기 원본**(C3 리뷰 fix 6 — notice 문구용)★: spec §1 notice 템플릿이
    ///   `기한({reply_by})` 을 그대로 노출하므로, 발신자가 쓴 표기(`"60m"`)를 **그대로** 싣는다.
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
    pub request_id: String,
    pub sender: String,
    /// 요청 발신자의 PeerId — 미결 조회가 "내가 건 요청" 을 **이름이 아니라 id 로** 가르는 축(리뷰 B1).
    pub sender_id: PeerId,
    pub recipient: String,
    pub recipient_id: Option<PeerId>,
    pub reply_by_raw: Option<String>,
    pub created_at: Instant,
    pub notified: bool,
}

#[derive(Debug)]
pub struct Ledger {
    history: VecDeque<MessageRecord>,
    requests: Vec<RequestEntry>,
    capacity: usize,
    /// ★evict 가 한 번이라도 일어났나(D 리뷰 B2 — 조회 정직성)★.
    /// ★왜 "정확히 몇 건 잘렸나" 가 아닌가★: 어떤 msg_id 의 행이 몇 개 사라졌는지는 이미 버린 데이터라
    ///   재구성할 수 없다. 없는 정확도를 지어내지 않고 **"잘렸을 수 있다" 는 사실만** 정직하게 전한다.
    // 리뷰 B2
    evicted_any: bool,
}

impl Default for Ledger {
    fn default() -> Self {
        Self::with_capacity(HISTORY_CAPACITY)
    }
}

impl Ledger {
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

    /// ★초기 상태 인자화★: 단일/그룹 발송은 `Pending`(주입 대기) 또는 `Delivered`(즉시 주입 폴백)로,
    ///   그룹 죽은 멤버는 `Skipped` 로 시작하므로 호출자가 초기 상태를 정한다(spec §4·§5).
    /// ★evict = front(오래된 것) + **회신으로 닫힌 계약만** 동반 정리(C3 fix 3 → D 리뷰 B3 로 좁힘)★: 링버퍼라
    ///   용량을 넘기면 가장 오래된 이력부터 버린다. 이때 같은 msg_id 의 request 추적 항목은 **회신으로 닫힌
    ///   것만** 함께 드롭한다(dangling 정리 + 유계). ★미회신 계약은 통지가 나갔든 아니든 evict 를 견딘다★ —
    ///   예전엔 통지된 것도 함께 드롭해서, 이력이 먼저 밀려난 뒤 기한이 지난 계약이 **미결 목록에서 통째로
    ///   증발**했다(리뷰 B3 — `RequestEntry::is_live` 주석에 시퀀스). 계약의 정본은 추적이지 이력이 아니므로
    ///   (ReplyOutcome 주석), 이력 용량이 계약을 죽이면 안 된다. 미회신 계약의 유계는 `MAX_OPEN_REQUESTS`
    ///   (open_request 의 `Full`)가 따로 준다.
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

    /// `record` + **기대 배달기록 수**(round-2 리뷰 F3). 그룹 fan-out 은 두 단계(계획 락 / 멤버별 구간)로
    /// 기록하므로 두 곳 모두 같은 N 을 넘겨야 한다.
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
            fail_code: None,
        });
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

    /// ★왜 (msg_id, to) 로 지목★: 그룹 방송은 한 msg_id 에 수신자별 레코드가 N개라 msg_id 만으로는 어느
    ///   배달인지 특정 못 한다 — 수신자까지 함께 지목해 정확히 한 레코드를 전이한다(1:N 회계, spec §4).
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

    /// ★삭제 정리 전용 종결(`Pending → Failed` + 사유 코드 — spec §5 삭제 정리 · §6 전이 그래프 개정)★.
    ///
    /// ★왜 **사유 코드가 필수 인자**인가(load-bearing)★: 조회가 코드 없는 `failed` 를 보고 "왜 실패했나" 를
    ///   답할 수 없는 상태를 타입으로 막는다.
    /// ★`NotFound`(레코드가 링에서 밀려남)는 호출자가 **락 밖에서** debug 로 남긴다 — 조용한 유실 금지.
    // ADR-0116 (결정 3/4 — 삭제 정리)
    pub fn fail_pending(
        &mut self,
        msg_id: &str,
        to: &str,
        code: &'static str,
        now: Instant,
    ) -> Result<(), TransitionError> {
        let Some(rec) = self
            .history
            .iter_mut()
            .find(|r| r.msg_id == msg_id && r.to == to)
        else {
            return Err(TransitionError::NotFound);
        };
        if !rec.status.can_fail_by_cleanup() {
            return Err(TransitionError::Illegal {
                from: rec.status,
                to: DeliveryStatus::Failed,
            });
        }
        rec.status = DeliveryStatus::Failed;
        rec.fail_code = Some(code);
        rec.transitioned_at = now;
        Ok(())
    }

    /// request 오픈 — `awaiting_reply` 추적 시작(spec §3 단계 2). **계약 키 = `(request_id, recipient)`**.
    ///
    /// ★다중 수신자 request = 독립 계약 N개(ADR-0111 결정 5 · load-bearing)★: 메시지 id 는 N계약 공통이고
    ///   (장부 = 메시지 1 : 배달기록 N 원칙 유지) **회신자가 누구냐로 계약이 갈린다**. 그래서 이 함수는 같은
    ///   `request_id` 로 **수신자마다 한 번씩** 불린다.
    ///   배달되지 못한 수신자(입구 반려·`MAILBOX_FULL`·`REQUEST_CAPACITY`)의 계약은 열지 않는다.
    ///
    /// ★reply_by 시계 = 발송 기준(spec §3·§5 · ADR-0104)★: 절대 기한 = `created_at(now) + reply_by`. 수신
    ///   지연과 무관한 발신자 관점 계약이라 now(발송 시각)를 기준으로 굳힌다.
    /// ★중복 **키** 거부 — 오픈이든 닫힘이든 존재하면 거부(load-bearing · finding 2)★: 같은
    ///   `(request_id, recipient)` 가 추적에 **하나라도 있으면**(open OR closed) `DuplicateId` 로 거부한다
    ///   (no-op). 메시지 id 는 **데몬이 생성하는 유일 값**이고 수신자는 발송 해석 단계에서 이미 중복 제거되므로
    ///   (spec §5 해석 순서 ④) 이 조합의 재사용은 non-scenario 다. ★파킹된 수신자도 계약을 연다(spec §3
    ///   항목 2)★ — busy 든 **잠듦**(ADR-0116 결정 1)이든 수용은 수용이라 회신 의무가 성립하고 기한 스윕도
    ///   정상 발화한다.
    ///   예전의 "닫힌 id 는 재오픈 허용" 관대함은
    ///   두 항목을 동시에 남겨 (a) 회신이 앞쪽 닫힌 항목을 먼저 만나 `AlreadyClosed` 오발, (b) 같은-id 이력
    ///   evict 가 재오픈 추적을 드롭하는 shadowing 버그를 낳았다.
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
    /// ★이력 링의 행은 손대지 않는다★(링이 자기 수명을 소유 — 은퇴는 **계약 추적**만의 일이다).
    // round-2 리뷰 F1 / 사용자 결정 2026-07-27
    /// ★인자 `recipient_id` 의 `None` 은 운영 경로에 실재한다(4차 — ADR-0116 결정 1)★:
    ///   **잠든 수신자**에게 파킹되는 request 는 산 incarnation 이 없어 `None` 으로 열린다
    ///   (`service::handle_send` 의 잠듦 갈래). 그 구간의 계약을 닫는 유일한 경로가 `close_on_reply` 의
    ///   **이름 폴백**이므로 그 팔을 "죽은 코드" 로 읽고 지우면 잠든 요청자 계약이 영영 안 닫힌다.
    ///   복원 후 실제 주입 시점에 `rebind_request_recipient` 가 착지 id 를 박아 그 뒤로는 id 축이 산다.
    #[allow(clippy::too_many_arguments)]
    pub fn open_request(
        &mut self,
        request_id: &str,
        sender: &str,
        sender_id: PeerId,
        recipient: &str,
        recipient_id: Option<PeerId>,
        reply_by: Option<(Duration, String)>,
        now: Instant,
    ) -> OpenOutcome {
        if self
            .requests
            .iter()
            .any(|r| r.request_id == request_id && r.recipient == recipient)
        {
            return OpenOutcome::DuplicateId;
        }
        let mut retired = None;
        if self.occupied_slots() >= MAX_OPEN_REQUESTS {
            // ★후보에서 빼는 두 부류(round-5 mark-and-sweep · load-bearing)★:
            //   ① 이미 은퇴 표시된 계약(`pending_retirement`) — 두 발송이 같은 희생자를 노리면 한쪽 커밋이
            //      다른 쪽의 희생자를 먼저 지워, 남은 쪽의 롤백/커밋이 허공을 가리킨다(계수도 어긋난다).
            //   ② **잠정 계약**(`provisional`) — 그 필드 주석의 실패 모드.
            // ADR-0108 (cap 은퇴 — 은퇴 가능분 최고령 선택)
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
                // ★잠정 계약만 남아 `Full` 이 나오는 것도 정직한 답이다★: 그 순간 상한은 실제로 차 있고,
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
            reply_failed: false,
            notified: false,
            pending_retirement: false,
            provisional: true,
            marked_victim: retired
                .as_ref()
                .map(|r: &RetiredContract| (r.request_id.clone(), r.recipient.clone())),
            reservation_token: None,
        });
        match retired {
            Some(r) => OpenOutcome::OpenedAfterMarking(r),
            None => OpenOutcome::Opened,
        }
    }

    /// ★방금 연 잠정 예약에 **소유자 생존 토큰**을 붙인다(R1)★ — 반드시 `open_request` 와 **같은 락 구간**에서
    /// 부른다(그 사이에 sweep 이 끼어들면 가드가 붙기 전의 항목을 버려진 것으로 읽는다 —
    /// `RequestEntry::reservation_token` 주석).
    ///
    /// 항목이 없으면 no-op(정상 흐름엔 없다 — 방금 만든 항목이다).
    // R1
    pub fn attach_reservation_liveness(
        &mut self,
        request_id: &str,
        recipient: &str,
        watch: Weak<()>,
    ) {
        if let Some(e) = self
            .requests
            .iter_mut()
            .find(|r| r.request_id == request_id && r.recipient == recipient && r.provisional)
        {
            e.reservation_token = Some(watch);
        }
    }

    /// ★예약 확정(커밋) — 표시된 희생자를 **물리 제거**하고 잠정 표시를 지운다(round-5 mark-and-sweep)★.
    ///
    /// ★희생자가 그 사이 회신으로 닫혔어도 그냥 제거한다★: `replied` 는 종점이라 더 볼 일이 없고, 그
    ///   사실은 이력 레코드에 이미 `Replied` 로 남아 있다(추적 항목은 회계용일 뿐이다).
    /// ★희생자를 못 찾으면 no-op★: 정상 흐름엔 없다(표시된 항목은 커밋/롤백까지 목록에 남는다).
    /// ★반환 = **실제로 한 일**(R2)★ — 계획한 은퇴가 일어났는지는 호출자가 가정하지 말고 이 값을 봐야 한다
    /// (`CommitOutcome` 헤더의 계측 오염 근거).
    // round-5 mark-and-sweep
    pub fn commit_open(
        &mut self,
        provisional: Option<(&str, &str)>,
        retired: Option<(&str, &str)>,
    ) -> CommitOutcome {
        let mut out = CommitOutcome {
            confirmed: false,
            retired: false,
        };
        if let Some((id, recipient)) = provisional {
            if let Some(e) = self
                .requests
                .iter_mut()
                .find(|r| r.request_id == id && r.recipient == recipient)
            {
                e.provisional = false;
                // 확정됐으니 표시 기억·생존 토큰은 더 필요 없다(희생자는 아래에서 물리 제거된다).
                e.marked_victim = None;
                e.reservation_token = None;
                out.confirmed = true;
            }
        }
        if let Some((id, recipient)) = retired {
            let before = self.requests.len();
            self.requests.retain(|r| {
                !(r.request_id == id && r.recipient == recipient && r.pending_retirement)
            });
            // ★purge **전에** 판정한다(R2)★: purge 도 항목을 지우므로 뒤에서 길이를 비교하면 "내가 은퇴시킨
            //   것" 과 "좀비 정리로 사라진 것" 이 섞인다.
            out.retired = self.requests.len() < before;
            self.purge_finished_without_history();
        }
        out
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
        provisional: Option<(&str, &str)>,
        retired: Option<(&str, &str)>,
    ) -> Option<DropOutcome> {
        if let Some((id, recipient)) = retired {
            if let Some(e) = self
                .requests
                .iter_mut()
                .find(|r| r.request_id == id && r.recipient == recipient && r.pending_retirement)
            {
                e.pending_retirement = false;
            }
        }
        provisional.map(|(id, recipient)| self.drop_request(id, recipient))
    }

    /// ★실제 배달된 수신자로 계약의 `recipient_id` 를 고쳐 박는다(round-2 리뷰 F2 · load-bearing)★.
    /// 그런 계약이 없으면(통보였거나 이미 닫힘) no-op.
    ///
    /// ★막는 것 = 배달자/의무자 불일치★: exact PeerId 로 건 request 가 그 순간 busy 라 **이름 키**로
    ///   파킹되면, 봉투는 이름 큐에 놓이고 id 는 힌트일 뿐이다. 그 뒤 A 가 죽고 같은 이름의 B 가 뜨면
    ///   flush 의 이름 폴백이 **B 에게 배달**한다(단일 발송은 재스폰 이어받기가 기능이다 — ADR-0101).
    ///   그런데 계약의 `recipient_id` 는 여전히 A 라, id 기준 매처(`matches_contract_party`)에서 B 의 미결
    ///   조회는 그 의무를 **못 본다** — 봉투를 실제로 받은 쪽이 "답할 게 없다" 고 읽는 최악의 조합이다.
    ///   그래서 **봉투가 실제로 꽂힌 시점**(pending→delivered 전이 자리, 착지 incarnation 을 아는 유일한
    ///   지점)에 의무를 그 수신자에게 옮긴다 — "의무는 봉투를 받은 자를 따른다".
    /// ★닫힌 계약은 건드리지 않는다★: 이미 회신이 온 계약의 상대를 뒤늦게 바꾸면 이력이 오염된다.
    // round-2 리뷰 F2
    pub fn rebind_request_recipient(
        &mut self,
        request_id: &str,
        recipient: &str,
        delivered_to: PeerId,
    ) {
        if let Some(r) = self
            .requests
            .iter_mut()
            .find(|r| r.request_id == request_id && r.recipient == recipient && !r.closed)
        {
            r.recipient_id = Some(delivered_to);
        }
    }

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

    /// ★용도(C3 리뷰 fix 5 — 타임아웃↔회신 레이스 좁히기)★: `due_timeouts` 로 걷은 뒤 notice 를 파킹하기
    ///   직전에 상위가 다시 확인한다 — 그 사이 회신이 도착해 계약이 닫혔으면 "회신 없음" 통지를 보내지
    ///   않는다. 없는 id 가 false 인 건 의도적이다: evict 등으로 추적이 사라진 경우 타임아웃은 실제로
    ///   발생했으므로 통지를 막을 이유가 없다.
    /// ★잔여(fix 1 과의 상호작용 — 정직한 명시)★: 이력이 이미 evict 된 계약은 **닫히는 순간 추적에서
    ///   제거**되므로(좀비 방지) 그 뒤 이 조회는 false 다 — 즉 "산출 후 회신 도착" 취소가 그 좁은 경우엔
    ///   안 걸리고 통지가 한 번 더 나갈 수 있다. 이력 용량(HISTORY_CAPACITY)만큼이 밀려난 뒤 마이크로초 창에서만 성립하는
    ///   경로라, 여기서 유계(좀비 제거)를 택했다.
    pub fn is_request_closed(&self, request_id: &str, recipient: &str) -> bool {
        self.requests
            .iter()
            .any(|r| r.request_id == request_id && r.recipient == recipient && r.closed)
    }

    /// ★표시(은퇴 예정·잠정)는 매칭을 가리지 않는다(round-5 mark-and-sweep · load-bearing)★: 두 표시는
    ///   **회계용**이지 존재 여부가 아니다. 여기서 `!closed` 만 보므로 ① 은퇴 예정으로 표시된 계약에 온
    ///   정당한 회신도 정상적으로 닫히고(옛 물리 제거 설계에선 이게 `NoMatch` 로 빗나갔다 — 발신자는 답을
    ///   받았는데 계약은 안 닫히고 나중에 헛 기한 통지까지 날 수 있었다) ② 아직 확정 전인 잠정 계약에 온
    ///   빠른 회신도 닫힌다. 닫힌 뒤의 커밋(제거)·롤백(표시 해제) 어느 쪽도 그 사실을 되돌리지 않는다.
    /// ★회신자로 계약을 고른다(ADR-0111 결정 5)★: 한 `request_id` 아래
    ///   계약이 **여러 개**일 수 있으므로(다중 수신자 request) `in_reply_to` 만으로는 어느 계약을 닫을지
    ///   특정할 수 없다. spec §3 은 "그 **회신자의 계약만** 닫힌다(다른 수신자 계약은 오픈 유지 — 전체회신
    ///   없음)" 이므로, 매칭 키는 `(request_id, 회신자)` 다.
    /// ★"내 계약이 아니다" = `NoMatch` = **배달은 그대로**(spec §3 항목 7-②)★: 모르는 id·이미 닫힌 계약·
    ///   나에게 없는 계약 어느 쪽이든 **계약 쪽만 무동작**이고 회신 메시지 자체는 정상 배달된다. 회신이
    ///   통째로 사라지는 편이 더 나쁘다.
    // L1
    pub fn close_on_reply(
        &mut self,
        in_reply_to: &str,
        replier_name: &str,
        replier_id: PeerId,
        allow_name_fallback: bool,
        now: Instant,
    ) -> ReplyOutcome {
        // 1) 추적 항목을 닫는다(정본). recipient 를 꺼내 뒤이어 이력 전이에 쓴다(borrow 분리).
        // ★계약 선택 = **두 패스**(id 먼저, 없으면 이름) — load-bearing(A6 회귀)★.
        //
        // ★왜 한 패스 OR 이 틀렸나★: `find` 는 **먼저 만나는 항목**을 집는다. 그래서 계약 A(recipient
        //   "alice", id a1)가 목록에서 앞서고 계약 B(개명·쌍둥이로 recipient 도 "alice", id b1)가 뒤에 있으면,
        //   **b1 이 보낸 회신이 이름 매치로 A 를 먼저 닫는다** — 자기 계약이 아닌 것을 닫는다. 게다가 미결
        //   조회의 귀속 판정(`matches_contract_party`)은 **id 단독**이라, 그 에이전트는 자기 화면에 보이지도
        //   않는 의무를 닫아 버리는 비대칭이 생긴다.
        // ★두 패스면 그 비대칭이 사라진다★: id 로 지목된 계약이 있으면 그게 유일한 정답이고(조회 축과 동일),
        //   id 매치가 하나도 없을 때만 이름으로 떨어진다 — 재스폰(새 PeerId)·잠듦 파킹처럼 계약이 id 를 들고
        //   있지 않은 경우의 정상 회신 경로를 그대로 살린다.
        // ★남는 위험을 **한 겹 더 좁혔다**(리뷰 fix D4)★: 계약이 id 를 안 들고 있는 구간(**잠듦 파킹** —
        //   운영 경로에 실재한다)에서 동명 쌍둥이가 이름으로 남의 계약을 닫는 경로가 있었다. 이제 상위가
        //   "그 이름이 산 세션 2개 이상에 걸리나" 를 판정해 `allow_name_fallback = false` 로 내려보내고,
        //   그때는 이름 폴백을 아예 타지 않는다(무동작 = 계약 오픈 유지 — 잘못 닫는 것보다 안 닫는 게 낫다).
        //   근본 제거는 여전히 ADR-0115(스폰 이름 유일성)다.
        let matched = self.match_contract_for_replier(
            in_reply_to,
            replier_name,
            replier_id,
            allow_name_fallback,
        );
        let recipient = match matched.map(|i| &mut self.requests[i]) {
            Some(r) if r.closed => return ReplyOutcome::AlreadyClosed,
            Some(r) => {
                r.closed = true;
                r.recipient.clone()
            }
            None => return ReplyOutcome::NoMatch,
        };
        let outcome = match self.transition(in_reply_to, &recipient, DeliveryStatus::Replied, now) {
            Ok(()) => ReplyOutcome::Closed,
            Err(TransitionError::NotFound) => ReplyOutcome::Closed,
            Err(TransitionError::Illegal { from, .. }) => {
                ReplyOutcome::ClosedHistoryAnomaly { from }
            }
        };
        self.purge_finished_without_history();
        outcome
    }

    /// ★회신자의 계약을 고르는 **단일 규칙**(id 우선 → 이름 폴백) — `close_on_reply` 와
    /// `fail_on_undeliverable_reply` 가 **같은 함수**를 쓴다(load-bearing)★.
    ///
    /// ★왜 함수로 묶나★: 두 동사는 같은 회신 1건의 두 결말(수용 → `replied` / 도달 불가 확정 →
    ///   `reply_failed`)이다. 선택 규칙이 갈리면 **같은 회신이 서로 다른 계약을 지목**해, 성공 경로가 닫은
    ///   계약과 실패 경로가 종결한 계약이 어긋난다(A6 가 잡은 부류의 재발 — 그때도 원인은 "한 규칙을 두 곳에
    ///   따로 쓴 것" 이었다). 규칙 본문의 근거·수용된 잔여는 `close_on_reply` 헤더가 정본이다.
    fn match_contract_for_replier(
        &self,
        in_reply_to: &str,
        replier_name: &str,
        replier_id: PeerId,
        allow_name_fallback: bool,
    ) -> Option<usize> {
        self.requests
            .iter()
            .position(|r| r.request_id == in_reply_to && r.recipient_id == Some(replier_id))
            .or_else(|| {
                if !allow_name_fallback {
                    return None;
                }
                self.requests.iter().position(|r| {
                    r.request_id == in_reply_to
                        && r.recipient_id.is_none()
                        && r.recipient == replier_name
                })
            })
    }

    /// ★회신이 **도달 불가 확정**이라 계약을 실패 종결한다(spec §3 항목 7-④ · ADR-0116 결정 2 · ADR-0118)★ —
    /// 회신 발송 행이 `RECIPIENT_NOT_FOUND`(= 요청자 이름이 로스터·프로필 **둘 다에 없다**) 일 때만
    /// 호출된다.
    ///
    /// ★그 밖의 회신 실패는 이 동사를 부르지 않는다(load-bearing)★: `MAILBOX_FULL`·`RECIPIENT_AMBIGUOUS`
    ///   는 **그 순간의 환경**이라 재시도가 실제로 배달에 성공할 수 있다 — 그때
    ///   장부가 영영 "회신 실패" 로 남으면 거짓말이 된다(ADR-0118 결정 2). 그 부류는 **무동작**(계약 오픈
    ///   유지)이고, 좀비가 아닌 근거는 "기한 통지가 나가면 ADR-0108 은퇴 자격을 얻는다" 다.
    /// ★가드 우선(ADR-0118 결정 3 — ADR-0108 결정 4 불변식 존중)★: **잠정(`provisional`)·은퇴 예정
    ///   (`pending_retirement`) 표시 계약은 건드리지 않는다**(`GuardHeld`). 미정산 항목의 수명은 그 가드
    ///   소유이므로, 여기서 닫으면 ① 가드의 커밋/롤백이 이미 종결된 계약을 되살리거나(유령 재개방)
    ///   ② 표시된 희생자를 남이 종결해 상한 교환이 반쪽 난다. 그 사이 도착한 실패 회신은 무동작이다.
    /// ★배달 상태는 건드리지 않는다(spec §6 축 구분)★: `reply_failed` 는 **계약 축** 종점이다 — 원 request 의
    ///   배달기록은 그대로 `delivered`(또는 `pending`)에 머문다. 여기서 이력을 전이하면 "배달됐다가 실패로
    ///   돌아간" 이력이 표현 가능해진다.
    // ADR-0116 (결정 2) / ADR-0118 (결정 1·2·3)
    pub fn fail_on_undeliverable_reply(
        &mut self,
        in_reply_to: &str,
        replier_name: &str,
        replier_id: PeerId,
        allow_name_fallback: bool,
    ) -> ReplyFailOutcome {
        let Some(idx) = self.match_contract_for_replier(
            in_reply_to,
            replier_name,
            replier_id,
            allow_name_fallback,
        ) else {
            return ReplyFailOutcome::NoMatch;
        };
        let entry = &mut self.requests[idx];
        if entry.closed {
            return ReplyFailOutcome::AlreadyClosed;
        }
        if entry.provisional || entry.pending_retirement {
            return ReplyFailOutcome::GuardHeld;
        }
        entry.closed = true;
        entry.reply_failed = true;
        self.purge_finished_without_history();
        ReplyFailOutcome::Failed
    }

    /// ★요청자 프로필 삭제 정리 — 그 이름이 **요청자**인 오픈 계약을 `reply_failed` 로 종결한다★
    /// (spec §5 삭제 정리 ② · ADR-0116 결정 3).
    ///
    /// ★대상 = 요청자(`sender`) 쪽만★: 회신이 갈 곳이 사라졌으므로 그 계약은 결말이 확정됐다. **회신자
    ///   (`recipient`) 쪽이 삭제된 계약은 유지한다** — 발신자는 기한 통지(spec §3 항목 4~5)로 무응답을 알게
    ///   되는 기존 경로가 살아 있고, 그 계약을 여기서 닫으면 "답을 못 받았다" 는 사실이 사라진다.
    /// ★이미 종결된 계약은 건드리지 않는다★: `replied` 는 **되돌리지 않는다**(spec §3 항목 7-④ "수용 =
    ///   완료" — 파킹된 회신이 나중에 `RECIPIENT_DELETED` 로 치워져도 계약은 `replied` 로 남는다).
    ///   `!closed` 필터가 그 규칙을 구조적으로 보장한다.
    /// ★가드 우선(ADR-0118 결정 3)★: 잠정·은퇴 예정 표시 계약은 건너뛰고 그 수를 함께 돌려준다(정산 후
    ///   재발화는 **하지 않는다** — 삭제 정리는 삭제 시점 단발이고, 그 잔여의 결말은 TTL 이다. spec §5).
    /// ★이름 기준 매칭★: 계약의 발신자 축은 이름(+id)이지만 삭제된 프로필의 **id 로 매칭하지 않는다** —
    ///   계약의 `sender` 는 **발송 시점 표시 이름**이고 파킹 큐 키도 이름이므로, 같은 축으로 판정해야 정리
    ///   대상이 갈리지 않는다(id 로 바꾸면 개명 전 이름으로 열린 계약을 놓친다).
    ///   ★발동 게이트는 그 위에서 id 로 판정한다(리뷰 fix D1 — `service::handle_profile_deleted`)★: 여기까지
    ///   왔다는 것은 이미 "그 프로필의 세션이 살아 있지 않다" 가 확정됐다는 뜻이다. 게이트(정확성)와 대상
    ///   선택(이름 축)이 서로 다른 축을 쓰는 것은 의도된 분업이다.
    // ADR-0116 (결정 3) / ADR-0118 (결정 3)
    pub fn fail_open_requests_from(&mut self, sender: &str) -> RequesterCleanup {
        let mut out = RequesterCleanup::default();
        for r in self.requests.iter_mut() {
            if r.closed || r.sender != sender {
                continue;
            }
            if r.provisional || r.pending_retirement {
                out.guard_held += 1;
                continue;
            }
            r.closed = true;
            r.reply_failed = true;
            out.failed.push((r.request_id.clone(), r.recipient.clone()));
        }
        if !out.failed.is_empty() {
            self.purge_finished_without_history();
        }
        out
    }

    /// ★오픈된 request 추적을 **통째로 제거**한다(C3 — 발송이 반려돼 계약이 애초에 성립하지 않은 경우)★.
    ///
    /// ★왜 `close_on_reply` 가 아니라 별도 출구인가(load-bearing — 유계 보장)★: 닫기(`closed=true`)는
    ///   "회신이 와서 계약이 이행됐다" 는 **이력**이라 추적 목록에 남는다. 그 잔존 항목은 같은 msg_id 의
    ///   **이력 레코드가 evict 될 때** 함께 드롭돼 유계가 유지된다(`record` 주석). 그런데 **반려된 발송**은
    ///   이력 레코드가 애초에 없다(park 조차 안 됐다) — 그래서 닫기만 하면 그 항목을 evict 할 계기가 영영
    ///   없어 반려가 반복될수록 추적 목록이 무계 증식한다. 반려는 "계약이 이행됨" 이 아니라 "계약이 성립한
    ///   적 없음" 이므로, 이력을 남기지 않고 흔적째 지우는 게 의미상으로도 맞다.
    pub fn drop_request(&mut self, request_id: &str, recipient: &str) -> DropOutcome {
        let Some(idx) = self
            .requests
            .iter()
            .position(|r| r.request_id == request_id && r.recipient == recipient)
        else {
            return DropOutcome::NotFound;
        };
        let removed = self.requests.remove(idx);
        DropOutcome::Removed {
            notified: removed.notified,
        }
    }

    /// ★**버려진** 예약을 회수한다(F1 보증 계층 · R1 기준 교체)★: 소유자 가드가 사라진 **잠정** 계약을 통째로
    /// 되돌린다(잠정 계약 제거 + 그 예약이 붙인 희생자 표시 해제). 회수한 계약 키를 돌려준다.
    ///
    /// ★왜 이게 **보증**인가(RAII Drop 은 보증이 아니다)★: `Reservation::drop` 은 `try_lock` 이 성공할 때만
    ///   롤백한다 — 락 경합이나 "같은 스레드가 상태 락을 쥔 채 Drop" 에서는 **아무 것도 못 한다**. 그때 남는
    ///   잔해의 대가는 영구적이다: ① 잠정 계약은 `due_timeouts` 가 영영 건너뛴다(기한 통지 소멸) ② 표시된
    ///   희생자는 `occupies_slot()` 에서 빠져 cap 분모가 영구히 줄고 ③ 추적 목록이 무계로 자란다.
    ///   그래서 **주기 sweep**(락을 정상적으로 소유하는 유일한 유지보수 지점)이 같은 일을 다시 한다 —
    ///   Drop 은 빠른 경로, 이쪽이 보증이다.
    /// ★멱등★: 정산된 항목은 잠정이 아니므로(커밋 = 표시 해제 · 롤백 = 제거) 두 번째 호출은 아무 것도 하지
    ///   않는다.
    /// ★회수는 파괴적이지 않다★: 잠정 계약은 **아직 발신자에게 접수를 보고하지 않은** 예약이거나(패닉으로
    ///   응답이 사라진 경우) 이미 실패 행으로 보고된 것이다. 어느 쪽이든 되돌리는 게 옳다.
    // ADR-0108 (mark-and-sweep — 예약 회수의 보증 계층)
    pub fn reclaim_abandoned_reservations(&mut self) -> Vec<(String, String)> {
        let abandoned: Vec<(String, String, Option<(String, String)>)> = self
            .requests
            .iter()
            .filter(|r| {
                r.provisional
                    && r.reservation_token
                        .as_ref()
                        .is_none_or(|w| w.upgrade().is_none())
            })
            .map(|r| {
                (
                    r.request_id.clone(),
                    r.recipient.clone(),
                    r.marked_victim.clone(),
                )
            })
            .collect();
        let mut reclaimed = Vec::with_capacity(abandoned.len());
        for (id, recipient, victim) in abandoned {
            // 표시 해제 + 잠정 계약 제거 = 롤백과 **같은 규칙**(두 곳이 갈리지 않게 같은 동사를 쓴다).
            if let Some((vid, vrecipient)) = &victim {
                if let Some(v) = self
                    .requests
                    .iter_mut()
                    .find(|r| &r.request_id == vid && &r.recipient == vrecipient)
                {
                    v.pending_retirement = false;
                }
            }
            self.drop_request(&id, &recipient);
            reclaimed.push((id, recipient));
        }
        reclaimed
    }

    /// ★경계★: `>` 비교라 정확히 기한인 순간은 아직 due 아님(mailbox TTL 경계와 동일 규약 — 결정적 테스트).
    /// ★은퇴 예정 표시는 건너뛰지 않고, **잠정 계약은 건너뛴다**(round-5 → round-6 I2 로 갈래 분리)★:
    ///   - **은퇴 표시된 계약은 애초에 due 가 될 수 없다**(구조적, 그대로 유지): 희생자 자격이
    ///     `notified || reply_by.is_none()` 이고 due 자격은 `!notified && reply_by.is_some()` 이라 두 집합은
    ///     **서로소**다. 그래서 표시 검사를 넣어 봐야 죽은 분기다 — 넣지 않는 편이 정직하다.
    ///   - ★**잠정 계약은 명시적으로 건너뛴다**(round-6 I2 · load-bearing)★. 잠정 구간은 **예약부터 그
    ///     수신자의 결말 확정까지**를 덮고(service.rs `PendingContract` — A2 라운드에서 커밋을 결말 뒤로
    ///     미뤘다), 그 사이에 자식 stdin `write_all` 이 들어 있다 — 우리 `stdio.rs` 가 스스로 문서화하듯 파이프 역압 아래에서 그
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
            if r.closed || r.notified || r.provisional {
                continue;
            }
            let Some((reply_by, reply_by_raw)) = r.reply_by.clone() else {
                continue;
            };
            let deadline = r.created_at + reply_by;
            if now > deadline {
                r.notified = true;
                due.push(DueTimeout {
                    request_id: r.request_id.clone(),
                    sender: r.sender.clone(),
                    sender_id: r.sender_id,
                    recipient: r.recipient.clone(),
                    reply_by_raw,
                });
            }
        }
        if !due.is_empty() {
            self.purge_finished_without_history();
        }
        due
    }

    /// 이력 레코드 수(관측/테스트).
    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// 오래된 순.
    pub fn records_for(&self, msg_id: &str) -> Vec<&MessageRecord> {
        self.history.iter().filter(|r| r.msg_id == msg_id).collect()
    }

    /// ★`records_for` + **불완전 신호**(D 리뷰 B2 · round-2 리뷰 F3 로 판정 교체)★ — `(행 목록, truncated)`.
    ///
    /// ★왜 필요한가★: 링(4096)이 가득 차면 앞쪽 행이 조용히 사라진다. 그런데 `messages { id }` 응답은 남은
    ///   행을 **그 메시지의 전부**인 양 보여 준다 — 10인 방송의 6행이 밀려나면 발신자는 "4명에게만 나갔다"
    ///   고 오독한다(실제로는 10명 모두에게 나갔고 기록만 사라졌다).
    /// ★판정 = `남은 행 수 < 기대 행 수`(결정적)★ — `MessageRecord.expected_rows`.
    /// ★행이 **통째로** 사라진 경우는 여기서 안 보인다★: 그건 빈 목록으로 나가고 상위가 계약 뷰 또는
    ///   `MESSAGE_NOT_FOUND` 로 답한다(그 hint 가 이력 회전을 알린다).
    // 리뷰 B2 / round-2 리뷰 F3
    pub fn records_for_detailed(&self, msg_id: &str) -> (Vec<&MessageRecord>, bool) {
        let rows: Vec<&MessageRecord> =
            self.history.iter().filter(|r| r.msg_id == msg_id).collect();
        let truncated = rows
            .first()
            .is_some_and(|r| rows.len() < usize::from(r.expected_rows));
        (rows, truncated)
    }

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

    /// 테스트 전용(H2 픽스처) — 그 계약을 회신 없이 닫는다(슬롯 비우기 — 픽스처 전용 지름길).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn close_for_test(&mut self, request_id: &str, recipient: &str, now: Instant) {
        if let Some(r) = self
            .requests
            .iter_mut()
            .find(|r| r.request_id == request_id && r.recipient == recipient)
        {
            r.closed = true;
        }
        let _ = now;
    }

    /// 테스트 전용(H2) — **은퇴 예정 표시가 달린 계약 수**. 표시는 `occupies_slot()` 에서 빠지므로, 이 값이
    ///   0 이 아닌 채 남으면 cap 분모가 영구히 줄어든다(잊은 정산의 관측축 — `service::Reservation` 헤더).
    ///   ★희생자 선정 로직을 복제하지 않는다★: "누가 표시됐나" 가 아니라 "표시가 남았나" 만 센다(drift 없음).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn marked_retirement_count_for_test(&self) -> usize {
        self.requests
            .iter()
            .filter(|r| r.pending_retirement)
            .count()
    }

    /// ★계약 축 종점 어휘 관측(4차 — spec §6 `awaiting_reply`/`replied`/`reply_failed`)★. 추적에 없으면
    ///   `None`(이력만 남았거나 모르는 키).
    ///
    /// ★왜 필요한가★: `replied` 와 `reply_failed` 는 오픈 목록·기한 스윕·상한 계수에서 **똑같이 빠지므로**
    ///   그 축으로는 구분되지 않는다. "수용은 완료로, 도달 불가 확정만 실패로" 라는 규칙(§3 항목 7-④)과
    ///   "삭제 정리는 `replied` 를 되돌리지 않는다" 를 단언하려면 사유 자체를 봐야 한다.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn contract_outcome_for_test(
        &self,
        request_id: &str,
        recipient: &str,
    ) -> Option<&'static str> {
        self.requests
            .iter()
            .find(|r| r.request_id == request_id && r.recipient == recipient)
            .map(|r| match (r.closed, r.reply_failed) {
                (true, true) => "reply_failed",
                (true, false) => "replied",
                (false, _) => "awaiting_reply",
            })
    }

    /// ★계약에 박힌 수신자 id 관측(리뷰 fix D4)★ — `None` = 아직 이름으로만 귀속된 계약(잠듦 파킹 구간).
    ///
    /// ★왜 필요한가★: "복원 후 실제 주입 시점에 착지 id 를 박는다"(`rebind_request_recipient`)는 **상태로만**
    ///   확인할 수 있고, 그 배선이 빠지면 동명 세션이 남의 계약을 이름으로 닫는 경로가 열린다.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn contract_recipient_id_for_test(
        &self,
        request_id: &str,
        recipient: &str,
    ) -> Option<PeerId> {
        self.requests
            .iter()
            .find(|r| r.request_id == request_id && r.recipient == recipient)
            .and_then(|r| r.recipient_id)
    }

    /// 테스트 전용(C2) — 그 계약 키가 추적 목록에 존재하나(닫힘·은퇴 표시 무관). 잠정/은퇴 창의 관측축.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn is_tracked_for_test(&self, request_id: &str, recipient: &str) -> bool {
        self.requests
            .iter()
            .any(|r| r.request_id == request_id && r.recipient == recipient)
    }

    /// ★왜 테스트에만★: 운영 호출부는 계약 키를 **둘 다** 손에 쥔 채 부른다(발송·회신 경로가 수신자를
    ///   이미 안다) — 이 역조회는 옛 "id 하나짜리 계약" 시절 테스트를 새 키에 맞추는 편의일 뿐이다.
    #[cfg(test)]
    fn party_of(&self, request_id: &str) -> Option<(String, Option<PeerId>)> {
        self.requests
            .iter()
            .find(|r| r.request_id == request_id)
            .map(|r| (r.recipient.clone(), r.recipient_id))
    }

    /// 오픈(미회신) request 수(관측/테스트).
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
    /// ★포함 기준 = `!closed`(= `is_live()`) — load-bearing★: 통지 여부는 **필드로 노출**하고 목록에서
    ///   제외하지 않는다. **여기 기준을 바꾸면 `is_live()` 도 함께 바꿔야 한다**(갈리면 같은 버그가 재발 —
    ///   `is_live` 주석의 4단계 시퀀스).
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
        out.sort_by_key(|r| r.created_at);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 옛 "id 하나 = 계약 하나" 테스트를 계약 키(ADR-0111 결정 5)에 얹는 어댑터 — 그 계약의
    /// **수신자 본인이 회신했다** 로 해석한다.
    fn reply(l: &mut Ledger, id: &str, now: Instant) -> ReplyOutcome {
        match l.party_of(id) {
            Some((name, pid)) => {
                l.close_on_reply(id, &name, pid.unwrap_or_else(PeerId::nil), true, now)
            }
            None => l.close_on_reply(id, "nobody", PeerId::nil(), true, now),
        }
    }

    fn drop_req(l: &mut Ledger, id: &str) -> DropOutcome {
        match l.party_of(id) {
            Some((name, _)) => l.drop_request(id, &name),
            None => l.drop_request(id, "nobody"),
        }
    }

    fn closed(l: &Ledger, id: &str) -> bool {
        match l.party_of(id) {
            Some((name, _)) => l.is_request_closed(id, &name),
            None => false,
        }
    }

    fn commit(l: &mut Ledger, provisional: Option<&str>, retired: Option<&str>) {
        let pv = provisional.and_then(|id| l.party_of(id).map(|(n, _)| (id.to_string(), n)));
        let rt = retired.and_then(|id| l.party_of(id).map(|(n, _)| (id.to_string(), n)));
        l.commit_open(
            pv.as_ref().map(|(i, n)| (i.as_str(), n.as_str())),
            rt.as_ref().map(|(i, n)| (i.as_str(), n.as_str())),
        );
    }

    fn rollback(
        l: &mut Ledger,
        provisional: Option<&str>,
        retired: Option<&str>,
    ) -> Option<DropOutcome> {
        let pv = provisional.and_then(|id| l.party_of(id).map(|(n, _)| (id.to_string(), n)));
        let rt = retired.and_then(|id| l.party_of(id).map(|(n, _)| (id.to_string(), n)));
        l.rollback_open(
            pv.as_ref().map(|(i, n)| (i.as_str(), n.as_str())),
            rt.as_ref().map(|(i, n)| (i.as_str(), n.as_str())),
        )
    }

    fn t0() -> Instant {
        Instant::now()
    }

    /// 발신자 PeerId(fix 2) — 대부분의 단언은 값 자체를 안 보므로 매번 새로 뽑는다.
    fn sid() -> PeerId {
        PeerId::new_v4()
    }

    /// 기한 튜플(fix 6) — 표기는 Duration 에서 만든 게 아니라 **발신자가 쓴 것**이라는 전제를 테스트에서도
    /// 유지하려고, 단언이 표기를 안 보는 자리에선 관례적 표기 하나를 쓴다.
    fn rb(d: Duration) -> Option<(Duration, String)> {
        Some((d, format!("{}s", d.as_secs())))
    }

    /// 커밋을 빠뜨린 상한 픽스처는 전부 잠정으로 남아 `Full` 만 나온다 — 운영 발송 경로는 결말 확정 시
    /// 반드시 커밋/롤백하므로(`service::commit_contract`) 테스트도 같은 상태를 만든다.
    fn open_committed(
        l: &mut Ledger,
        id: &str,
        reply_by: Option<(Duration, String)>,
        now: Instant,
    ) -> OpenOutcome {
        let out = l.open_request(id, "alice", sid(), "bob", None, reply_by, now);
        commit(l, Some(id), None);
        out
    }

    /// 이력 없는 계약은 evict 이후에만 존재한다 — 그 케이스를 노리지 않는 테스트는 이 헬퍼로 이력을 함께
    /// 만들어 운영 상태를 재현한다.
    fn open_delivered_request(
        l: &mut Ledger,
        id: &str,
        reply_by: Option<(Duration, String)>,
        now: Instant,
    ) {
        l.record(id, "alice", "bob", "q", DeliveryStatus::Pending, now);
        l.open_request(id, "alice", sid(), "bob", None, reply_by, now);
        commit(l, Some(id), None);
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
        let mut l = Ledger::new();
        let now = t0();
        l.record("g1", "boss", "a", "rebase", DeliveryStatus::Pending, now);
        l.record("g1", "boss", "b", "rebase", DeliveryStatus::Pending, now);
        l.record("g1", "boss", "c", "rebase", DeliveryStatus::Skipped, now);
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
        let mut l = Ledger::new();
        let now = t0();

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

        l.record("p1", "a", "b", "x", DeliveryStatus::Pending, now);
        assert_eq!(
            l.transition("p1", "b", DeliveryStatus::Replied, now),
            Err(TransitionError::Illegal {
                from: DeliveryStatus::Pending,
                to: DeliveryStatus::Replied
            }),
            "Pending → Replied 는 불법(Delivered 경유 필요)"
        );

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
        let mut l = Ledger::new();
        l.record("m", "a", "b", "x", DeliveryStatus::Pending, now);
        assert_eq!(
            l.transition("m", "b", DeliveryStatus::Delivered, now),
            Ok(())
        );
        assert_eq!(l.transition("m", "b", DeliveryStatus::Replied, now), Ok(()));
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
        assert_eq!(reply(&mut l, "req-1", now), ReplyOutcome::Closed);
        assert_eq!(l.open_request_count(), 0, "회신으로 닫힘");
    }

    #[test]
    fn strict_reply_wrong_id_does_not_close() {
        let mut l = Ledger::new();
        let now = t0();
        l.open_request("req-1", "alice", sid(), "bob", None, None, now);
        assert_eq!(reply(&mut l, "req-999", now), ReplyOutcome::NoMatch);
        assert_eq!(l.open_request_count(), 1, "틀린 id 는 request 를 안 닫아야");
    }

    #[test]
    fn second_reply_to_same_request_is_already_closed_noop() {
        let mut l = Ledger::new();
        let now = t0();
        open_delivered_request(&mut l, "req-1", None, now);
        assert_eq!(reply(&mut l, "req-1", now), ReplyOutcome::Closed);
        assert_eq!(reply(&mut l, "req-1", now), ReplyOutcome::AlreadyClosed);
        assert_eq!(l.open_request_count(), 0);
    }

    #[test]
    fn duplicate_open_request_id_is_rejected() {
        let mut l = Ledger::new();
        let now = t0();
        assert_eq!(
            l.open_request("req-1", "alice", sid(), "bob", None, None, now),
            OpenOutcome::Opened
        );
        assert_eq!(
            l.open_request("req-1", "alice", sid(), "carol", None, None, now),
            OpenOutcome::Opened,
            "같은 메시지 id 의 **다른 수신자** = 독립 계약(다중 수신자 request)"
        );
        assert_eq!(
            l.open_request("req-1", "alice", sid(), "bob", None, None, now),
            OpenOutcome::DuplicateId,
            "같은 계약 키(id, 수신자)의 재오픈은 거부"
        );
        assert_eq!(
            l.open_request_count(),
            2,
            "열린 계약 = (req-1,bob)·(req-1,carol) 둘뿐 — 중복 키는 추적에 추가 안 됨"
        );
    }

    #[test]
    fn closed_id_cannot_be_reopened_and_reply_stays_already_closed() {
        let mut l = Ledger::new();
        let now = t0();
        open_delivered_request(&mut l, "req-1", None, now);
        assert_eq!(reply(&mut l, "req-1", now), ReplyOutcome::Closed);
        assert_eq!(
            l.open_request("req-1", "alice", sid(), "bob", None, None, now),
            OpenOutcome::DuplicateId,
            "닫힌 id 재오픈은 거부(유일성 전제)"
        );
        assert_eq!(l.open_request_count(), 0, "재오픈 안 됐으니 오픈 0");
        assert_eq!(
            reply(&mut l, "req-1", now),
            ReplyOutcome::AlreadyClosed,
            "재오픈 없이 회신하면 AlreadyClosed(shadowing 없음)"
        );
    }

    #[test]
    fn close_on_reply_transitions_history_to_replied_with_timestamp() {
        let mut l = Ledger::new();
        let now = t0();
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
        let reply_at = now + Duration::from_secs(30);
        assert_eq!(reply(&mut l, "req-1", reply_at), ReplyOutcome::Closed);
        let rec = l.records_for("req-1")[0];
        assert_eq!(rec.status, DeliveryStatus::Replied, "이력이 Replied 로");
        assert_eq!(
            rec.transitioned_at, reply_at,
            "전이 시각 = 회신 시각(spec §5)"
        );
    }

    #[test]
    fn close_on_reply_against_pending_history_is_anomaly_but_still_closes() {
        let mut l = Ledger::new();
        let now = t0();
        l.record("req-1", "alice", "bob", "q", DeliveryStatus::Pending, now);
        l.open_request("req-1", "alice", sid(), "bob", None, None, now);
        assert_eq!(
            reply(&mut l, "req-1", now),
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
        let mut l = Ledger::with_capacity(1);
        let now = t0();
        // 추적은 이력과 별도 맵이라 `record` 없이 계약만 열 수 있다 — req-1 은 가리킬 이력 행이 처음부터 없다.
        l.open_request("req-1", "alice", sid(), "bob", None, None, now);
        l.record("other", "x", "y", "z", DeliveryStatus::Delivered, now);
        assert_eq!(
            reply(&mut l, "req-1", now),
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
        let reply_by = Duration::from_secs(600);
        open_committed(&mut l, "req-1", rb(reply_by), now);
        assert!(
            l.due_timeouts(now + reply_by).is_empty(),
            "정확히 기한은 due 아님"
        );
        let due = l.due_timeouts(now + reply_by + Duration::from_nanos(1));
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].request_id, "req-1");
        assert_eq!(due[0].sender, "alice", "notice 는 발신자에게(spec §3)");
        assert_eq!(due[0].recipient, "bob");
    }

    #[test]
    fn due_timeout_carries_sender_id_and_raw_notation() {
        // 표기를 정규화하지 않는 이유 — `60m` 가 그대로여야 봉투 reply-by 와 통지 문구가 일치한다.
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
        commit(&mut l, Some("req-1"), None);
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
        assert_eq!(reply(&mut l, "req-1", now), ReplyOutcome::Closed);
        let over = now + reply_by + Duration::from_secs(60);
        assert!(
            l.due_timeouts(over).is_empty(),
            "회신된 request 는 타임아웃 대상 아님"
        );
    }

    #[test]
    fn drop_request_removes_the_entry_entirely_unlike_close() {
        let mut l = Ledger::new();
        let now = t0();
        let reply_by = Duration::from_secs(600);

        open_delivered_request(&mut l, "closed-1", rb(reply_by), now);
        assert_eq!(reply(&mut l, "closed-1", now), ReplyOutcome::Closed);
        assert_eq!(
            l.open_request("closed-1", "alice", sid(), "bob", None, rb(reply_by), now),
            OpenOutcome::DuplicateId,
            "닫힌 항목은 추적에 남아 재오픈을 막는다"
        );

        l.open_request("dropped-1", "alice", sid(), "bob", None, rb(reply_by), now);
        assert_eq!(
            drop_req(&mut l, "dropped-1"),
            DropOutcome::Removed { notified: false },
            "제거 성공 — 통지 전이었으므로 notified=false"
        );
        assert_eq!(
            drop_req(&mut l, "dropped-1"),
            DropOutcome::NotFound,
            "멱등 — 두 번째는 NotFound"
        );
        assert_eq!(
            l.open_request("dropped-1", "alice", sid(), "bob", None, rb(reply_by), now),
            OpenOutcome::Opened,
            "제거된 id 는 다시 열 수 있다(계약 미성립 = 흔적 없음)"
        );
        assert_eq!(l.open_request_count(), 1, "열린 계약은 방금 것 하나뿐");
    }

    #[test]
    fn request_without_reply_by_never_times_out() {
        let mut l = Ledger::new();
        let now = t0();
        l.open_request("req-1", "alice", sid(), "bob", None, None, now);
        let far = now + Duration::from_secs(100_000);
        assert!(
            l.due_timeouts(far).is_empty(),
            "기한 없는 request 는 타임아웃 없음"
        );
    }

    #[test]
    fn skipped_status_for_group_dead_member() {
        let mut l = Ledger::new();
        let now = t0();
        l.record("g1", "boss", "dead", "msg", DeliveryStatus::Skipped, now);
        assert_eq!(l.records_for("g1")[0].status, DeliveryStatus::Skipped);
    }

    // ── evict ↔ request 추적 결합 ────────────────────────────────────────────────
    #[test]
    fn eviction_drops_only_finished_request_tracking() {
        let cap = 2;
        let mut l = Ledger::with_capacity(cap);
        let now = t0();
        l.record("done", "alice", "bob", "q", DeliveryStatus::Pending, now);
        open_committed(&mut l, "done", None, now);
        assert!(matches!(
            reply(&mut l, "done", now),
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
        let cap = 2;
        let mut l = Ledger::with_capacity(cap);
        let now = t0();
        let reply_by = Duration::from_secs(600);
        l.record("req-1", "alice", "bob", "q", DeliveryStatus::Pending, now);
        open_committed(&mut l, "req-1", rb(reply_by), now);
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
        let due = l.due_timeouts(now + reply_by + Duration::from_secs(1));
        assert_eq!(due.len(), 1, "evict 됐어도 타임아웃 통지는 살아 있다");
        assert_eq!(due[0].request_id, "req-1");

        // 다른 장부로 재현 — 위에서 이미 통지된 항목과 섞지 않으려고 분리한다.
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
            reply(&mut l2, "req-2", now),
            ReplyOutcome::Closed,
            "이력이 evict 됐어도 회신은 계약을 닫는다(가리킬 이력만 없음)"
        );
        assert_eq!(l2.open_request_count(), 0);
    }

    fn ledger_with_evicted_live_contract(
        cap: usize,
        id: &str,
        reply_by: Option<(Duration, String)>,
        now: Instant,
    ) -> Ledger {
        let mut l = Ledger::with_capacity(cap);
        l.record(id, "alice", "bob", "q", DeliveryStatus::Pending, now);
        l.open_request(id, "alice", sid(), "bob", None, reply_by, now);
        commit(&mut l, Some(id), None);
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
        let now = t0();
        let mut l = ledger_with_evicted_live_contract(2, "req-1", None, now);
        assert_eq!(reply(&mut l, "req-1", now), ReplyOutcome::Closed);
        assert_eq!(
            l.tracking_len(),
            0,
            "닫히는 순간 고아 추적 항목을 제거(좀비 없음)"
        );
        assert!(!l.msg_id_in_use("req-1"), "추적에도 이력에도 없다");
    }

    #[test]
    fn timeout_notice_after_history_eviction_keeps_the_unanswered_contract_tracked() {
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
        let open = l.open_requests();
        assert_eq!(open.len(), 1, "미결 목록에 남아야: {open:?}");
        assert!(open[0].notified, "통지 사실은 플래그로만 구분");
        assert_eq!(reply(&mut l, "req-1", now), ReplyOutcome::Closed);
        assert_eq!(l.tracking_len(), 0, "닫히는 순간 고아 항목 제거");
    }

    #[test]
    fn tracking_stays_bounded_when_evicted_contracts_are_answered() {
        let cap = 2;
        let mut l = Ledger::with_capacity(cap);
        let now = t0();
        let reply_by = Duration::from_secs(60);
        for i in 0..50 {
            let id = format!("r{i}");
            l.record(&id, "alice", "bob", "q", DeliveryStatus::Pending, now);
            open_committed(&mut l, &id, rb(reply_by), now);
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
            assert_eq!(reply(&mut l, &id, now), ReplyOutcome::Closed);
            assert_eq!(
                l.tracking_len(),
                0,
                "라운드마다 추적이 0 으로 수렴(좀비 누적 없음)"
            );
        }
    }

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

        let outcome = l.open_request("over", "alice", sid(), "bob", None, None, over);
        match outcome {
            OpenOutcome::OpenedAfterMarking(r) => {
                assert_eq!(r.request_id, "r0", "가장 오래된 것부터 은퇴 표시");
                assert_eq!(r.sender, "alice");
                assert_eq!(r.recipient, "bob");
            }
            other => panic!("표시 후 수용이어야: {other:?}"),
        }
        assert!(
            l.open_requests().iter().any(|r| r.request_id == "r0"),
            "커밋 전에는 희생자가 여전히 열린 계약이다"
        );
        assert_eq!(
            l.tracking_len(),
            MAX_OPEN_REQUESTS + 1,
            "표시 구간엔 +1(잠정분)"
        );
        commit(&mut l, Some("over"), Some("r0"));
        assert!(
            !l.open_requests().iter().any(|r| r.request_id == "r0"),
            "커밋에서 비로소 은퇴한다"
        );
        assert_eq!(l.tracking_len(), MAX_OPEN_REQUESTS, "유계 유지(512 불변)");
        assert!(l.open_requests().iter().any(|r| r.request_id == "over"));
    }

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
        commit(&mut l, Some("over"), Some("r0"));
        assert_eq!(l.tracking_len(), MAX_OPEN_REQUESTS);
    }

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
        let over = now + reply_by + Duration::from_secs(1);
        assert_eq!(l.due_timeouts(over).len(), MAX_OPEN_REQUESTS);
        assert!(matches!(
            l.open_request("over", "alice", sid(), "bob", None, None, over),
            OpenOutcome::OpenedAfterMarking(_)
        ));
    }

    #[test]
    fn open_requests_expose_both_party_ids_for_obligation_scoping() {
        let mut l = Ledger::new();
        let now = t0();
        let sender = sid();
        let recipient = sid();
        l.record("r1", "alice", "worker", "q", DeliveryStatus::Pending, now);
        l.open_request("r1", "alice", sender, "worker", Some(recipient), None, now);
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
            "잠듦 파킹은 id 가 없다 — 나중에 그 이름으로 등장한 쪽이 답할 주체(WYSIWYA)"
        );
    }

    #[test]
    fn records_for_detailed_compares_surviving_rows_against_the_expected_count() {
        let now = t0();
        let mut l = Ledger::with_capacity(8);
        l.record("m1", "a", "b", "x", DeliveryStatus::Delivered, now);
        let (rows, truncated) = l.records_for_detailed("m1");
        assert_eq!(rows.len(), 1);
        assert!(!truncated, "기대 1행이 그대로 있으면 완전");

        for to in ["x", "y", "z"] {
            l.record_with_expected("g1", "a", to, "b", DeliveryStatus::Delivered, now, 3);
        }
        let (rows, truncated) = l.records_for_detailed("g1");
        assert_eq!(rows.len(), 3);
        assert!(!truncated, "3/3 이면 완전");

        let mut l2 = Ledger::with_capacity(4);
        l2.record_with_expected("g2", "a", "m1", "b", DeliveryStatus::Pending, now, 3);
        l2.record_with_expected("g2", "a", "m2", "b", DeliveryStatus::Pending, now, 3);
        l2.record("other1", "a", "z", "x", DeliveryStatus::Delivered, now);
        l2.record_with_expected("g2", "a", "m3", "b", DeliveryStatus::Delivered, now, 3);
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
        // cap 을 **은퇴 불가**(기한 대기 중) 계약으로 채운다 — 기한 없는 계약으로 채우면 은퇴가 일어나
        //   Full 이 나오지 않는다(그 갈래는 별도 테스트).
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
        assert_eq!(reply(&mut l, "r0", now), ReplyOutcome::Closed);
        assert_eq!(
            l.open_request("over", "alice", sid(), "bob", None, rb(reply_by), now),
            OpenOutcome::Opened,
            "닫힌 계약은 cap 계수에서 빠진다"
        );
    }

    // G1
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

        rollback(&mut l, Some("new"), Some("r0"));
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
        assert!(matches!(
            l.open_request("new2", "alice", sid(), "bob", None, None, over),
            OpenOutcome::OpenedAfterMarking(ref v) if v.request_id == "r0"
        ));
    }

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
        let OpenOutcome::OpenedAfterMarking(v1) =
            l.open_request("PA", "alice", sid(), "bob", None, None, now)
        else {
            panic!("전제: 표시 후 수용");
        };
        assert_eq!(v1.request_id, "c0");
        assert_eq!(l.occupied_slots(), cap, "표시+삽입 후에도 정확히 상한");
        assert_eq!(reply(&mut l, "PA", now), ReplyOutcome::Closed);
        assert_eq!(
            l.occupied_slots(),
            cap,
            "닫힌 잠정 계약도 정산 전까지 자리를 지킨다(round-6 I1)"
        );
        let b = l.open_request("PB", "carol", sid(), "dave", None, None, now);
        assert!(
            matches!(b, OpenOutcome::OpenedAfterMarking(ref v) if v.request_id == "c1"),
            "B 도 자기 몫의 희생자를 표시해야: {b:?}"
        );
        assert_eq!(l.occupied_slots(), cap);
        rollback(&mut l, Some("PA"), Some("c0"));
        assert_eq!(
            l.occupied_slots(),
            cap,
            "롤백 뒤에도 정확히 상한 — 513 고착 없음(round-6 I1)"
        );
        commit(&mut l, Some("PB"), Some("c1"));
        assert_eq!(l.occupied_slots(), cap, "B 커밋 후에도 상한 유지");

        // ── 커밋 갈래 ──────────────────────────────────────────────────────────────
        let mut l = build();
        let OpenOutcome::OpenedAfterMarking(v1) =
            l.open_request("PA", "alice", sid(), "bob", None, None, now)
        else {
            panic!("전제");
        };
        assert_eq!(v1.request_id, "c0");
        assert_eq!(reply(&mut l, "PA", now), ReplyOutcome::Closed);
        assert_eq!(l.occupied_slots(), cap, "정산 전엔 자리 유지");
        commit(&mut l, Some("PA"), Some("c0"));
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

    #[test]
    fn a_provisional_contract_is_not_swept_but_is_collected_right_after_commit() {
        let mut l = Ledger::new();
        let now = t0();
        let reply_by = Duration::from_secs(60);
        let over = now + reply_by + Duration::from_secs(1);
        l.open_request("p", "alice", sid(), "bob", None, rb(reply_by), now);
        assert!(
            l.due_timeouts(over).is_empty(),
            "잠정 계약은 sweep 대상이 아니다(round-6 I2)"
        );
        assert!(l.open_requests().iter().all(|r| !r.notified));

        commit(&mut l, Some("p"), None);
        let due = l.due_timeouts(over);
        assert_eq!(due.len(), 1, "커밋 후엔 곧바로 수집된다");
        assert_eq!(due[0].request_id, "p");
        assert_eq!(
            due[0].reply_by_raw,
            format!("{}s", reply_by.as_secs()),
            "표기 원본 그대로"
        );

        let mut l = Ledger::new();
        l.open_request("q", "alice", sid(), "bob", None, rb(reply_by), now);
        assert!(l.due_timeouts(over).is_empty());
        rollback(&mut l, Some("q"), None);
        assert!(
            l.due_timeouts(over + Duration::from_secs(9999)).is_empty(),
            "반려된 요청엔 기한 통지가 없다"
        );
    }

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
        let a_at = base + Duration::from_secs(1000);
        assert!(matches!(
            l.open_request("A", "alice", sid(), "bob", None, None, a_at),
            OpenOutcome::OpenedAfterMarking(ref r) if r.request_id == "evictable"
        ));

        let b = l.open_request("B", "carol", sid(), "dave", None, None, a_at);
        assert_eq!(
            b,
            OpenOutcome::Full,
            "잠정 계약을 희생자로 고르면 안 된다 — 고를 게 없으면 Full 이 정답: {b:?}"
        );
        assert!(l.open_requests().iter().any(|r| r.request_id == "A"));
        assert_eq!(reply(&mut l, "A", a_at), ReplyOutcome::Closed);
    }

    #[test]
    fn rollback_unmarks_and_drops_atomically_without_drift() {
        let mut l = Ledger::new();
        let now = t0();
        for i in 0..MAX_OPEN_REQUESTS {
            open_committed(&mut l, &format!("r{i}"), None, now);
        }
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
            let dropped = rollback(&mut l, Some(provisional.as_str()), Some("r0"));
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

    #[test]
    fn reclamation_follows_owner_liveness_not_the_clock() {
        let mut l = Ledger::new();
        let now = t0();
        for i in 0..MAX_OPEN_REQUESTS {
            open_committed(&mut l, &format!("r{i}"), None, now);
        }
        let occupied_before = l.occupied_slots();
        let tracking_before = l.tracking_len();

        let OpenOutcome::OpenedAfterMarking(victim) =
            l.open_request("p-live", "alice", sid(), "bob", None, None, now)
        else {
            panic!("전제: 상한 압력으로 표시 후 수용");
        };
        assert_eq!(victim.request_id, "r0");
        let live = ReservationLiveness::new();
        l.attach_reservation_liveness("p-live", "bob", live.watch());
        assert_eq!(l.marked_retirement_count_for_test(), 1, "표시 1건");

        for round in 0..3 {
            assert!(
                l.reclaim_abandoned_reservations().is_empty(),
                "round {round}: 소유자 생존 예약은 sweep 대상이 아니다"
            );
        }
        assert!(l.is_tracked_for_test("p-live", "bob"), "그대로 남아 있어야");
        assert_eq!(l.marked_retirement_count_for_test(), 1, "표시도 그대로");

        drop(live);
        let reclaimed = l.reclaim_abandoned_reservations();
        assert_eq!(
            reclaimed,
            vec![("p-live".to_string(), "bob".to_string())],
            "회수한 계약 키를 호출자에게 알려야(락 밖 기록용): {reclaimed:?}"
        );
        assert!(
            !l.is_tracked_for_test("p-live", "bob"),
            "잠정 계약이 제거돼야"
        );
        assert_eq!(
            l.marked_retirement_count_for_test(),
            0,
            "희생자 표시도 풀려야 — 남으면 cap 분모가 영구히 준다"
        );
        assert!(
            l.open_requests().iter().any(|r| r.request_id == "r0"),
            "희생자 자신은 열린 채 그대로(교환이 성립하지 않았으니)"
        );
        assert_eq!(l.occupied_slots(), occupied_before);
        assert_eq!(l.tracking_len(), tracking_before);

        assert!(l.reclaim_abandoned_reservations().is_empty());
    }

    #[test]
    fn a_settled_reservation_is_never_reclaimed_even_after_its_guard_is_gone() {
        let mut l = Ledger::new();
        let now = t0();
        l.record("p-ok", "alice", "bob", "q", DeliveryStatus::Pending, now);
        let OpenOutcome::Opened = l.open_request("p-ok", "alice", sid(), "bob", None, None, now)
        else {
            panic!("전제: 여유 상태에서 오픈");
        };
        let live = ReservationLiveness::new();
        l.attach_reservation_liveness("p-ok", "bob", live.watch());
        let committed = l.commit_open(Some(("p-ok", "bob")), None);
        assert!(committed.confirmed, "전제: 커밋이 항목을 찾았다");
        drop(live);

        assert!(
            l.reclaim_abandoned_reservations().is_empty(),
            "정산된 계약을 회수하면 접수 보고한 request 가 계약 없이 남는다"
        );
        assert!(
            l.is_tracked_for_test("p-ok", "bob"),
            "계약은 그대로 살아 있어야"
        );
    }

    #[test]
    fn commit_reports_whether_the_planned_retirement_actually_happened() {
        let mut l = Ledger::new();
        let now = t0();

        for i in 0..MAX_OPEN_REQUESTS {
            open_committed(&mut l, &format!("r{i}"), None, now);
        }
        let OpenOutcome::OpenedAfterMarking(v) =
            l.open_request("p1", "alice", sid(), "bob", None, None, now)
        else {
            panic!("전제: 표시 후 수용");
        };
        let out = l.commit_open(
            Some(("p1", "bob")),
            Some((v.request_id.as_str(), v.recipient.as_str())),
        );
        assert_eq!(
            out,
            CommitOutcome {
                confirmed: true,
                retired: true
            },
            "실제로 은퇴했으면 그렇다고 답해야"
        );

        let OpenOutcome::OpenedAfterMarking(v2) =
            l.open_request("p2", "alice", sid(), "bob2", None, None, now)
        else {
            panic!("전제: 표시 후 수용");
        };
        // 표시된 희생자를 밖에서 제거해 "커밋 시점엔 이미 없다" 를 만든다(purge 경로와 같은 결과 모양).
        assert!(matches!(
            l.drop_request(&v2.request_id, &v2.recipient),
            DropOutcome::Removed { .. }
        ));
        let out2 = l.commit_open(
            Some(("p2", "bob2")),
            Some((v2.request_id.as_str(), v2.recipient.as_str())),
        );
        assert_eq!(
            out2,
            CommitOutcome {
                confirmed: true,
                retired: false
            },
            "계약은 확정됐지만 은퇴는 없었다 — 이 값이 계측의 조건이다(R2)"
        );

        let out3 = l.commit_open(Some(("nope", "nobody")), None);
        assert_eq!(
            out3,
            CommitOutcome {
                confirmed: false,
                retired: false
            }
        );
    }

    #[test]
    fn a_reply_to_a_marked_victim_during_the_window_closes_it_properly() {
        let base = t0();
        let reply_by = Duration::from_secs(600);
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
        assert_eq!(
            reply(&mut l, "v", win),
            ReplyOutcome::Closed,
            "표시는 매칭을 가리지 않는다(round-5)"
        );
        let tracked_before = l.tracking_len();
        rollback(&mut l, Some("new"), Some("v"));
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
        assert_eq!(reply(&mut l, "v", win), ReplyOutcome::Closed);
        commit(&mut l, Some("new"), Some("v"));
        assert!(!l.open_requests().iter().any(|r| r.request_id == "v"));
        assert_eq!(
            l.open_request_count(),
            MAX_OPEN_REQUESTS,
            "커밋 후 계수는 정확히 상한"
        );
    }

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
        commit(&mut l, Some("new"), Some("r0"));
        assert!(!l.msg_id_in_use("r0"));
        assert!(l.msg_id_in_use("new"), "확정된 계약은 계속 사용 중");
    }

    #[test]
    fn open_requests_are_sorted_oldest_first_regardless_of_list_position() {
        let mut l = Ledger::new();
        let base = t0();
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
        rollback(&mut l, Some("new"), Some(victim.request_id.as_str()));
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
        l.rebind_request_recipient("r1", "worker", b);
        assert_eq!(
            l.open_requests()[0].recipient_id,
            Some(b),
            "의무는 봉투를 실제로 받은 자를 따른다(F2)"
        );
        assert_eq!(reply(&mut l, "r1", now), ReplyOutcome::Closed);
        l.rebind_request_recipient("r1", "worker", a);
        assert!(l.open_requests().is_empty(), "닫힌 계약은 미결이 아니다");
        l.rebind_request_recipient("nope", "worker", a);
    }

    #[test]
    fn msg_id_in_use_sees_history_and_tracking() {
        let mut l = Ledger::new();
        let now = t0();
        assert!(!l.msg_id_in_use("m1"), "미사용 id");
        l.record("m1", "a", "b", "x", DeliveryStatus::Delivered, now);
        assert!(l.msg_id_in_use("m1"), "이력에 있으면 사용 중");
        l.open_request("r1", "a", sid(), "b", None, None, now);
        assert!(l.msg_id_in_use("r1"), "추적에만 있어도 사용 중");
        open_delivered_request(&mut l, "r2", None, now);
        assert_eq!(reply(&mut l, "r2", now), ReplyOutcome::Closed);
        assert!(
            l.msg_id_in_use("r2"),
            "닫혀도 이력·추적에 남아 있으면 사용 중"
        );
    }

    #[test]
    fn is_request_closed_only_true_for_closed_entries() {
        let mut l = Ledger::new();
        let now = t0();
        open_delivered_request(&mut l, "r1", None, now);
        assert!(!closed(&l, "r1"), "열린 계약은 false");
        assert!(!closed(&l, "nope"), "없는 id 는 false(통지 막지 않음)");
        assert_eq!(reply(&mut l, "r1", now), ReplyOutcome::Closed);
        assert!(closed(&l, "r1"), "회신으로 닫히면 true");
    }

    #[test]
    fn drop_request_reports_already_notified_entry() {
        let mut l = Ledger::new();
        let now = t0();
        let reply_by = Duration::from_secs(600);
        open_delivered_request(&mut l, "r1", rb(reply_by), now);
        assert_eq!(
            l.due_timeouts(now + reply_by + Duration::from_secs(1))
                .len(),
            1
        );
        assert_eq!(
            drop_req(&mut l, "r1"),
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
        let mut l = Ledger::new();
        let now = t0();
        let d = Duration::from_secs(600);
        open_delivered_request(&mut l, "replied", None, now);
        open_delivered_request(&mut l, "timedout", rb(d), now);
        assert_eq!(reply(&mut l, "replied", now), ReplyOutcome::Closed);
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
        l.record("m1", "alice", "bob", "hi", DeliveryStatus::Delivered, now);
        assert!(l.open_requests().is_empty());
    }

    // ── S18.6: `pending → failed` 간선 + 계약 실패 종결(spec §6 · ADR-0116/0118) ────────────────

    #[test]
    fn pending_to_failed_is_legal_only_through_the_cleanup_verb() {
        let mut l = Ledger::new();
        let now = t0();
        l.record("m1", "a", "b", "x", DeliveryStatus::Pending, now);
        assert_eq!(
            l.transition("m1", "b", DeliveryStatus::Failed, now),
            Err(TransitionError::Illegal {
                from: DeliveryStatus::Pending,
                to: DeliveryStatus::Failed
            }),
            "임의 pending→failed 호출은 거부된다(간선은 전용 동사에만 열려 있다)"
        );
        assert_eq!(l.fail_pending("m1", "b", "RECIPIENT_DELETED", now), Ok(()));
        let rows = l.records_for("m1");
        assert_eq!(rows[0].status, DeliveryStatus::Failed);
        assert_eq!(
            rows[0].fail_code,
            Some("RECIPIENT_DELETED"),
            "사유 코드가 레코드에 남아 조회가 이유를 답할 수 있다"
        );

        l.record("m2", "a", "b", "x", DeliveryStatus::Delivered, now);
        assert_eq!(
            l.fail_pending("m2", "b", "RECIPIENT_DELETED", now),
            Err(TransitionError::Illegal {
                from: DeliveryStatus::Delivered,
                to: DeliveryStatus::Failed
            }),
            "delivered→failed 는 여전히 불법"
        );
        assert_eq!(
            l.fail_pending("nope", "b", "RECIPIENT_DELETED", now),
            Err(TransitionError::NotFound)
        );
    }

    #[test]
    fn a_failed_reply_never_touches_a_guard_marked_contract() {
        let mut l = Ledger::new();
        let now = t0();
        let w = PeerId::new_v4();
        l.record("m1", "boss", "worker", "q", DeliveryStatus::Delivered, now);
        l.open_request("m1", "boss", PeerId::new_v4(), "worker", Some(w), None, now);
        assert_eq!(
            l.fail_on_undeliverable_reply("m1", "worker", w, true),
            ReplyFailOutcome::GuardHeld,
            "잠정 계약은 건드리지 않는다"
        );
        assert_eq!(
            l.contract_outcome_for_test("m1", "worker"),
            Some("awaiting_reply")
        );
        l.commit_open(Some(("m1", "worker")), None);
        assert_eq!(
            l.fail_on_undeliverable_reply("m1", "worker", w, true),
            ReplyFailOutcome::Failed
        );
        assert_eq!(
            l.contract_outcome_for_test("m1", "worker"),
            Some("reply_failed")
        );
        assert_eq!(
            l.fail_on_undeliverable_reply("m1", "worker", w, true),
            ReplyFailOutcome::AlreadyClosed
        );
        assert_eq!(
            l.fail_on_undeliverable_reply("nope", "worker", w, true),
            ReplyFailOutcome::NoMatch
        );
    }

    #[test]
    fn both_guard_marks_hold_off_the_two_failure_closers() {
        // ★리뷰 fix D7 — 가드 커버리지의 빈칸★: 기존 가드 테스트는 **잠정(`provisional`)만** 만들어서,
        //   `pending_retirement` 쪽 조건을 한 줄 지워도 전부 초록이었다(무방비 실측).
        // ★표시는 **운영 경로**로 만든다★: 테스트 setter 로 플래그를 세우면 "그 표시가 실제로 생기는 경로"
        //   와 갈릴 수 있다.
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
            "전원 통지 완료 = 은퇴 가능"
        );
        let marked = match l.open_request("over", "alice", sid(), "bob2", None, None, over) {
            OpenOutcome::OpenedAfterMarking(r) => r,
            other => panic!("표시 후 수용이어야: {other:?}"),
        };
        assert_eq!(marked.request_id, "r0");
        assert_eq!(l.marked_retirement_count_for_test(), 1);

        assert_eq!(
            l.fail_on_undeliverable_reply("r0", "bob", PeerId::new_v4(), true),
            ReplyFailOutcome::GuardHeld,
            "은퇴 예정 표시 계약은 실패 종결 대상이 아니다(ADR-0118 결정 3)"
        );
        assert_eq!(
            l.contract_outcome_for_test("r0", "bob"),
            Some("awaiting_reply")
        );

        let out = l.fail_open_requests_from("alice");
        assert!(
            !out.failed.iter().any(|(id, _)| id == "r0" || id == "over"),
            "표시된 계약(r0=은퇴 예정 · over=잠정)은 종결 목록에 없어야: {out:?}"
        );
        assert!(
            out.guard_held >= 2,
            "두 표시가 모두 건너뛴 수로 보고돼야: {out:?}"
        );
        assert_eq!(
            l.contract_outcome_for_test("r0", "bob"),
            Some("awaiting_reply"),
            "은퇴 예정 표시 계약의 수명은 그 가드 소유다"
        );
        assert_eq!(
            l.contract_outcome_for_test("over", "bob2"),
            Some("awaiting_reply"),
            "잠정 계약도 그대로(기존 커버리지 유지)"
        );
    }

    #[test]
    fn the_general_transition_graph_rejects_both_failed_edges() {
        // ★뮤테이션 프로브 D9-c★: 범용 전이 그래프에 `(Delivered, Failed)`·`(Pending, Failed)` 를 **추가해도**
        //   메시징 전 테스트가 초록이었다(기존 불법-간선 테스트는 무관한 세 간선만 봤다).
        let mut l = Ledger::new();
        let now = t0();
        l.record("m-del", "a", "b", "x", DeliveryStatus::Delivered, now);
        assert_eq!(
            l.transition("m-del", "b", DeliveryStatus::Failed, now),
            Err(TransitionError::Illegal {
                from: DeliveryStatus::Delivered,
                to: DeliveryStatus::Failed
            }),
            "delivered→failed 는 범용 전이에서 불법(§6)"
        );
        assert_eq!(
            l.records_for("m-del")[0].status,
            DeliveryStatus::Delivered,
            "거부된 전이는 상태를 바꾸지 않는다"
        );
        l.record("m-park", "a", "b", "x", DeliveryStatus::Pending, now);
        assert_eq!(
            l.transition("m-park", "b", DeliveryStatus::Failed, now),
            Err(TransitionError::Illegal {
                from: DeliveryStatus::Pending,
                to: DeliveryStatus::Failed
            }),
            "임의 pending→failed 호출은 거부된다(간선은 fail_pending 에만 열려 있다)"
        );
        assert_eq!(l.records_for("m-park")[0].status, DeliveryStatus::Pending);
        assert!(
            l.records_for("m-park")[0].fail_code.is_none(),
            "거부된 전이는 사유 코드도 남기지 않는다"
        );
    }

    #[test]
    fn the_requester_cleanup_closes_open_contracts_but_respects_guards_and_replied() {
        let mut l = Ledger::new();
        let now = t0();
        let (a, b, c) = (PeerId::new_v4(), PeerId::new_v4(), PeerId::new_v4());
        let gone = PeerId::new_v4();
        l.record("m-open", "gone", "w1", "q", DeliveryStatus::Delivered, now);
        l.open_request("m-open", "gone", gone, "w1", Some(a), None, now);
        l.commit_open(Some(("m-open", "w1")), None);
        l.open_request("m-done", "gone", gone, "w2", Some(b), None, now);
        l.commit_open(Some(("m-done", "w2")), None);
        l.record("m-done", "gone", "w2", "x", DeliveryStatus::Delivered, now);
        assert_eq!(
            l.close_on_reply("m-done", "w2", b, true, now),
            ReplyOutcome::Closed
        );
        // 잠정 계약 — 커밋하지 않아 가드가 남는다.
        l.open_request("m-prov", "gone", gone, "w3", Some(c), None, now);
        l.open_request("m-other", "someone", a, "w4", Some(a), None, now);
        l.commit_open(Some(("m-other", "w4")), None);

        let out = l.fail_open_requests_from("gone");
        assert_eq!(
            out.failed,
            vec![("m-open".to_string(), "w1".to_string())],
            "오픈 계약 하나만 종결: {out:?}"
        );
        assert_eq!(out.guard_held, 1, "잠정 계약은 건너뛴 수로 보고된다");
        assert_eq!(
            l.contract_outcome_for_test("m-open", "w1"),
            Some("reply_failed")
        );
        assert_eq!(
            l.contract_outcome_for_test("m-done", "w2"),
            Some("replied"),
            "★되돌리지 않는다★ — 수용 = 완료(spec §3 항목 7-④)"
        );
        assert_eq!(
            l.contract_outcome_for_test("m-prov", "w3"),
            Some("awaiting_reply"),
            "가드 보유 계약은 그대로"
        );
        assert_eq!(
            l.contract_outcome_for_test("m-other", "w4"),
            Some("awaiting_reply"),
            "남의 계약은 무관"
        );
        assert_eq!(
            l.occupied_slots(),
            2,
            "m-prov(잠정) + m-other 만 자리를 차지한다"
        );
    }
}
