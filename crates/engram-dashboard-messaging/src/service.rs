//! service — MessagingService: 순수 구조(mailbox·ledger)를 tokio 위에서 발송 파이프라인에 엮는
//! 오케스트레이터(S18 메시징 v1 · ADR-0103/0104 · **ADR-0111/0112/0114 발송 개편** · **ADR-0125 적재-후-드레인**).
//!
//! ★역할★: **다중 수신자 발송**(spec §5)과 등장/idle flush(ADR-0104), TTL sweep, **idle 게이트**,
//!   **회신 계약**(request 장부 오픈·회신 닫기·기한 초과 notice)을 담당한다. 발송 진입점은
//!   `handle_send` **하나**다 — 개인·다중·`@all` 이 전부 같은 레일을 탄다(경로 1벌, ADR-0111 결정 2/4).
//!
//! ★적재 → 드레인(ADR-0125 · spec §5 — 이 파일의 뼈대)★: 발송은 **예외 없이 큐 꼬리에 적재된 뒤**, 같은
//!   호출 안에서 그 수신자 큐를 앞에서부터 드레인하고 리턴한다(적재 = 락 안, 드레인 = 락 밖).
//!
//! ★수신자 1명당 3분기(spec §5 · **ADR-0116 결정 1** — 다중 수신자는 이 판정을 수신자마다
//!   1회씩 = fan-out)★. 이 판정은 **신원 축만** 본다("이 수신자가 누구냐") — "지금 받을 수 있나" 는 뒤이은
//!   드레인의 몫이다(ADR-0125). 판정 소스는 **둘**(로스터 / 프로필 목록)이고 원칙 한 줄은
//!   **"살아 있으면 보내고, 나중에 살아날 수 있으면 기다리고, 없으면 시끄럽게 실패한다"** 다:
//!   1. **입구 반려 → 수신자별 실패 행**:
//!      - **없음** — 로스터·프로필 **둘 다에 없다**(오타·미스폰·삭제됨) → `RECIPIENT_NOT_FOUND`.
//!      - 동명 다수는 **어느 층에서든** `RECIPIENT_AMBIGUOUS`. 산·잠듦에 걸친 동명은 **산 쪽이 이긴다**
//!        (로스터 판정이 먼저).
//!   2. **산 수신자 — 적재한 뒤 같은 호출의 드레인이 결말을 정한다.** **capability 는 "언제 넣을지" 만
//!      가른다**(ADR-0116 결정 7 — 자격 조건이 아니다).
//!   3. **잠듦 — 적재하고 `pending`**(로스터엔 없지만 **프로필이 실재**해 복원 가능 — ADR-0116 결정 1).
//!      드레인할 산 실체가 없어 복원(재등장 flush)까지 큐에 머문다. 옛 "이름 없어도 파킹" 과 다르다:
//!      프로필에 실재하는 이름만 수용하므로 오타가 조용히 쌓이는 경로는 없다.
//!   ★보관함 초과는 어느 분기에서든 **그 수신자만** `failed` + `MAILBOX_FULL`★(전체 반려로 승격하지 않는다 —
//!   spec §5 부분 진행).
//!   **깨우기(wake)는 없다** — 잠든 수신자는 파킹될 뿐이다(spec §8 v2 후보).
//!
//! ★판정 소스 스냅샷은 발송 1회당 **딱 한 장**(불변식 — ADR-0111 결정 2 · ADR-0116 으로 소스 확장)★:
//!   수신자별로 다시 뜨면 해석 도중 명단 변동에 의한 반쪽 판정이 재발한다.
//!   ★물리 조회는 **2회**다★: 세션 스냅샷(`list_agents()`) 1회 + 프로필 목록 1회. 프로필 목록과의 경합은
//!   유계라 수용하고(spec §5), 그 잔여는 TTL·삭제 정리가 거둔다.
//!
//! ★순서 보장의 범위(finding 8 · ADR-0125 · load-bearing)★: 한 수신자가 보는
//!   순서는 **적재 순서**다 — 발송이 예외 없이 큐 꼬리에 들어가고 드레인이 앞에서부터 빼기 때문이다. 그래서
//!   좌석 예약도, "앞에 먼저 나갈 게 있나" 합류 판정도 **없다**(다시 필요해졌다면 발송 경로가 둘로
//!   갈라졌다는 신호다).
//!   남는 수용분은 **하나**다: **서로 다른 수신자** 사이의 전역 순서 — inject 를 락 안으로 넣어야만 닫히므로
//!   (락 규율 정면 위반) 사람 대화 수준 메시지율에서 의도적으로 수용한다.
//!
//! ★단일 락(load-bearing — ADR-0006 정신)★: Mailbox+Ledger 를 **하나의 `Mutex<MessagingState>`** 뒤에 둔다.
//!   락 순서 위험이 없고(락 하나) 메시지율이 극히 낮아(사람 대화 수준) 경합이 무의미하다.
//!   ★절대 규율★: 이 락을 **든 채로 `DeliveryPort`(inject/roster)를 부르지 않는다** — 락 아래에서 결정할
//!   것(파킹/주입 대상 수집)을 먼저 끝내고 락을 놓은 뒤 DeliveryPort(외부 호출)를 부른다. 이걸 어기면
//!   inject 가 내부에서 다른 락(sessions RwLock 등)을 잡아 락 순서 역전·데드락 위험이 생긴다.
//!
//! ★봉투 = 주입 시점 조립(단일 wrap point, ADR-0096)★: 파킹은 **감싸지 않은 body + 발신자 이름 + 발송
//!   메타**를 저장하고, 봉투는 **주입할 때** `wrap_message`/`wrap_notice`(이 crate `envelope.rs` 단일 wrap
//!   point)로 만든다. 왜: 파킹과 flush 사이 봉투 포맷(colon/xml 전역 스위치)이 바뀔 수 있고, 그때 flush 는
//!   **현재** 포맷으로 감싸야 한다. 그래서 raw body + 속성 재료를 나르고(`ParkPayload`) 조립은 주입 순간
//!   한 곳에서 — 즉시 배달과 늦은 배달의 봉투가 **같아야** 한다.
//!
//! 워크스페이스 crate import 0(ADR-0110 — 컴파일러 강제).
// ADR-0103
// ADR-0104
// ADR-0111
// ADR-0112
// ADR-0114
// ADR-0116 (입구 3분기 · 턴 신호 없으면 즉시 주입 · 잠듦 파킹 · 회신 계약 실패 종결 · 삭제 정리)
// ADR-0118 (계약 수명 접합 — 가드 우선 · 512 계수)
// ADR-0121 (@all/@here 분리 · 턴 신호 없는 부류의 순서 보장 · 게이트 술어 단일 정의)
// ADR-0125 (전부 적재 후 동기 드레인 — 직발송 지름길·좌석 예약 폐지 · delivered 복원)

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::busy::{AlwaysIdleGate, BusyGate};
use super::envelope::{
    new_msg_id, wrap_message, wrap_notice, DeliveryObservation, Entrance, EnvelopeFields,
    EnvelopeFormat,
};
use super::groups::{normalize_group_name, BuiltinGroups, GroupError, GroupSource, MemberPools};
use super::ledger::{
    DeliveryStatus, DueTimeout, Ledger, OpenOutcome, ReplyFailOutcome, ReplyOutcome,
    ReservationLiveness, RetiredContract, TransitionError,
};
use super::mailbox::{FlightTicket, Mailbox, ParkError, ParkKind, ParkedMessage};
use crate::{PeerId, SenderIdentity};

/// ★notice 의 장부상 발신자 라벨(C3)★ — `<notice>` 는 **from 이 없는** 데몬 통지라 진짜 발신자가 없다.
///   그래도 장부 레코드는 `from` 칸을 요구하므로(조용한 유실 금지 — notice 도 반드시 장부에 남는다) 데몬
///   출처임을 나타내는 고정 라벨을 쓴다. ★주소가 아니다★: 이 문자열로 메시지를 보낼 수 없고(로스터 이름이
///   아님) 봉투에도 절대 렌더되지 않는다(notice 봉투엔 from 속성 자체가 없다 — ADR-0103 불변식).
const NOTICE_SENDER_LABEL: &str = "engram";

// ★삭제됨(R1) — 옛 `STALE_RESERVATION_AFTER`(5초 유예)★: 예약 회수를 **나이**로 판정하던 상수다. 되살리지
//   말 것 — 어떤 값도 안전하지 않다. 예약 구간의 락 밖 주입은 자식 stdin write 이고 그건 backpressure 로
//   **무계로 블록될 수 있다**(core `stdio.rs`). 즉 "N초 넘었으면 버려진 것" 은 아직 일하는 소유자를 오판하고,
//   그 오판의 결과가 **계약 없는 request 배달**이었다(실패 사슬 전문 = `ledger::ReservationLiveness` 헤더).
//   회수 기준은 이제 소유자 가드의 **생존**이다(`ledger::reclaim_abandoned_reservations`).
// ★7차 보정 — 위 논거의 전제가 바뀌었다(ADR-0125). 결론은 그대로다★: 전부-큐가 되면서 예약 창(open→settle)이
//   **적재 락 안에 통째로 들어와** 정상 경로에는 "예약 구간의 락 밖 주입" 이 더 이상 없다. 그러니 위 문단을
//   읽고 "무계 블록이 없어졌으니 나이 기준을 되살려도 되겠다" 로 가지 말 것 — 나이 기준을 금지하는 진짜 이유는
//   **어떤 임계값도 소유자 생존을 대신 판정할 수 없다**는 것이고, 그건 창의 길이와 무관하다. 이 상수를
//   되살리는 것은 ADR-0108 재론이다.
// ADR-0108 (예약 회수 보증 계층 — 기준 = 생존)
// ADR-0125 (예약 창이 적재 락 안으로 들어옴 — 논거 전제 보정)

/// ★raw 표기와 파싱값을 **둘 다** 나르는 이유(load-bearing)★: 봉투 속성 `reply-by` 는 발신자가 쓴 **표기
///   그대로**(`"10m"`) 보여야 하고(spec §1 — 수신 LLM 이 읽는 계약), 장부 타이머는 **절대 기한**을 계산할
///   `Duration` 이 필요하다(spec §3 "데몬이 절대시각 환산"). 한쪽만 나르면 다른 쪽에서 재파싱·재렌더가
///   생겨 표기가 미묘하게 달라진다(`60m` → `1h`).
/// ★`Default` = 통보★: 전 필드 비활성이 곧 기본 메시지(type 없음)다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SendMeta {
    pub request: bool,
    /// 발신자가 쓴 기한 표기 그대로(`"10m"`). 봉투 `reply-by` 속성 값. `request` 일 때만 의미 있다.
    pub reply_by_raw: Option<String>,
    /// 파싱된 기한 — 장부가 `발송시각 + 이것` 으로 절대 기한을 굳힌다. raw 가 Some 이면 반드시 Some.
    pub reply_by: Option<Duration>,
    pub reply_to: Option<String>,
    /// ★봉투 `to` 속성 값(spec §1 — ADR-0111 로 "그룹 라벨" 에서 재정의)★. `Some` 이면 그 문자열이 그대로
    ///   XML `to` 속성이 된다. 값의 형태 = **수용된 명시 지목의 정규 이름 + `@`주소 토큰(펼치지 않음)** 을
    ///   **입력 표기 순**으로 이은 것(`"@all,carol"`).
    ///
    /// ★노출 조건 = 수용 판정된 수신자 2인 이상★(그룹 여부가 아니다 — ADR-0111 로 그룹 전용 semantics 가
    ///   폐지돼 `@all` 발송과 여러 명 직접 지목이 같은 경로가 됐다). 세는 것은 결과 상태가 아니라 **수용
    ///   판정**이다 — `delivered` 는 실제 주입 시점에야 확정되므로 그걸 기준 삼으면 봉투를 만들 때 아직
    ///   모르는 값을 참조하는 순환이 된다.
    /// ★왜 `SendMeta` 에 얹나(load-bearing)★: 파킹분이 **늦게 배달될 때도 같은 봉투**여야 하기 때문이다.
    ///   봉투는 park 시점이 아니라 주입 시점에 조립되므로(모듈 헤더 "봉투 = 주입 시점 조립"), 속성 재료는
    ///   `ParkPayload` 에 실려 flush 까지 살아남아야 한다 — 계약 필드(request/reply_to)와 **정확히 같은
    ///   이유**라 같은 통로에 태운다. 별도 축을 만들면 flush 경로가 두 벌이 된다.
    /// ★입구는 이 값을 채우지 않는다★: 값의 확정은 전 수신자의 수용 판정이 끝난 뒤 `handle_send` 가 1회만
    ///   한다(debug_assert 로 고정) — 그래야 그 발송의 모든 봉투가 **같은** `to` 를 싣는다.
    // ADR-0111
    pub to_attr: Option<String>,
}

impl SendMeta {
    /// 이 발송이 봉투에 붙일 속성(spec §1 노출 원칙 — **행동을 바꾸는 필드만**). 발신 인자 `reply_to` 가
    /// 수신 속성 `in-reply-to` 로 나타난다(spec §1 표기 매핑).
    ///
    /// ★`id` 는 request 에만★: 통보/회신 봉투에 id 를 실으면 수신 LLM 이 "이건 회신해야 하나" 를 헷갈린다 —
    ///   노출 필드가 곧 행동 신호라 필요 없는 축은 아예 숨긴다(spec §1 · ADR-0103 결정 1).
    /// ★가시성 `pub`(ADR-0110 이사로 승격)★: 호스트 입구(데몬 ingress)의 단위 테스트가 "검증된 인자 →
    ///   봉투" 를 한 줄로 단언한다(속성 조립 규칙의 단일 출처를 테스트가 우회해 재구현하지 않게).
    pub fn envelope_fields(&self, msg_id: &str) -> EnvelopeFields {
        EnvelopeFields {
            id: self.request.then(|| msg_id.to_string()),
            msg_type: self.request.then(|| "request".to_string()),
            reply_by: if self.request {
                self.reply_by_raw.clone()
            } else {
                None
            },
            in_reply_to: self.reply_to.clone(),
            to: self.to_attr.clone(),
        }
    }
}

/// 수신자 1명분의 발송 결과(spec §6 `results[]` 원소). 상위(입구)가 wire JSON 으로 옮긴다.
///
/// ★수신자 1명 = 행 1개(spec §5 해석 순서 ④)★: 중복 지목(`@all` + 같은 이름 직접 지목)은 해석 단계에서
///   이미 접혔으므로 여기서 같은 수신자가 두 번 나올 수 없다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipientResult {
    /// 수신자 표기 — **발신자가 쓴 토큰**(트림만, WYSIWYA · ADR-0101). `@all` 펼침 결과는 로스터 이름이다.
    pub to: String,
    pub status: SendStatus,
    /// `Failed` 일 때 **필수**인 실패 코드(spec §6 행 코드). 그 외엔 `None`.
    pub code: Option<FailCode>,
    /// 자기교정 힌트(파킹 사유·실패 사유). `Delivered` 는 `None`.
    pub hint: Option<String>,
}

/// 수신자 1명의 결말 어휘(spec §6 — `delivered|pending|failed`).
///
/// ★`skipped` 는 **응답 어휘에서 폐지**됐다(ADR-0111 결정 3)★ — 그룹 멤버 skip 규칙과 함께 소멸했다.
/// 장부 종점 어휘의 `skipped`(notice 레인 은퇴)는 다른 축이라 잔존한다(spec §6 대응표).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendStatus {
    /// 실제 주입 완료 — ledger `delivered`(ADR-0104 실제 주입 시점).
    Delivered,
    /// 파킹됨 — ledger `pending`.
    Pending,
    /// ★그 수신자만 실패(ADR-0111 결정 3 신설)★ — 나머지 수신자에겐 그대로 배달된다(부분 진행).
    Failed,
}

/// 수신자별 실패 코드(spec §6 행 코드 — 발송 단위 반려 코드와 **다른 축**이다).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailCode {
    /// ★로스터·프로필 **둘 다에 없는 이름**(오타·미스폰·이미 삭제됨)★
    ///   (ADR-0116 결정 1): 잠든 세션은 이 코드가 아니라 파킹(`pending`)이고, **살아 있으나 구조화
    ///   출력이 없는 상대도 이 코드가 아니다** — 게이트 없이 즉시 주입돼 정상 배달된다(결정 7).
    ///   ★옛 `RecipientUnreachable` 는 폐기됐다★ — 그 코드가 가리킬
    ///   상태가 없다. 되살리려면 새 결정이 먼저다(ADR-0116 거부한 대안).
    // ADR-0116 (의미 축소 · 결정 7)
    RecipientNotFound,
    /// 같은 이름의 에이전트가 둘 이상 — 누구에게 보낼지 데몬이 고를 근거가 없다(ADR-0114 결정 4 과도기 규칙).
    ///   ★어느 층에서든 같은 코드다★: 산 층(로스터 동명)뿐 아니라 **잠듦 층**(같은 이름으로 파생되는
    ///   프로필이 둘 이상)도 이 코드로 실패한다 — 이름 키 파킹은 "먼저 복원된 쪽이 조용히 받는" 구멍이 된다
    ///   (ADR-0116 결정 1). ADR-0115(스폰 이름 유일성)가 이 부류를 사문화할 예정이다.
    RecipientAmbiguous,
    /// 그 수신자의 보관함(message 레인 100건)이 가득 참 — **회수 시도 없이 즉시**(ADR-0114 결정 1).
    MailboxFull,
    /// 그 수신자의 회신 계약을 열 수 없음(오픈 계약 상한 512, ADR-0108). **그 수신자에겐 배달도 하지 않는다** —
    ///   request 의 본질이 회신 추적이라 추적 없는 배달은 계약 위반이다(ADR-0114 영향 절).
    RequestCapacity,
    /// ★파킹돼 기다리는 동안 수신자 프로필이 삭제돼 미배달 종결됨(ADR-0116 결정 3)★.
    ///
    /// ★발송 응답에는 **절대 나타나지 않는다**(spec §6)★: 발송 시점엔 `pending` 이었고 이 코드는 삭제 정리가
    ///   사후에 찍는다 — 그래서 발신 LLM 은 이걸 `messages{id}` **조회**에서 처음 본다(조회 행이 code 와
    ///   힌트를 함께 싣는 이유 — `message_state`). `RecipientNotFound`("보낼 때부터 없었다")와 **시점이
    ///   다르다**: 발신자가 "이름을 잘못 썼구나" 와 "기다리다 상대가 지워졌구나" 를 구분해야 재발송 판단이
    ///   갈린다.
    // ADR-0116 (결정 3/4)
    RecipientDeleted,
}

impl FailCode {
    /// wire 코드 문자열(안정 계약 — 발신 LLM 이 이 값으로 분기한다, spec §6).
    pub fn as_str(self) -> &'static str {
        match self {
            FailCode::RecipientNotFound => "RECIPIENT_NOT_FOUND",
            FailCode::RecipientAmbiguous => "RECIPIENT_AMBIGUOUS",
            FailCode::MailboxFull => "MAILBOX_FULL",
            FailCode::RequestCapacity => "REQUEST_CAPACITY",
            FailCode::RecipientDeleted => "RECIPIENT_DELETED",
        }
    }
}

/// ★발송 **단위** 반려(spec §5·§6 `{status:"error", code, hint}`)★ — 수신자별로 나눌 수 없는 문제만.
///
/// ★층위(ADR-0114 결정 3 — 이 enum 의 존재 이유)★: **이름의 부재 = 런타임 상태**(그 순간 그 에이전트가
///   없을 뿐, 표기는 정상) → 수신자별 실패 행 / **`@`주소 오류 = 주소 공간 오류**(존재하지 않는 주소 문법·
///   오타 그룹명 = 발신자가 고쳐서 다시 보내야 하는 것) → 전체 반려. 이 한 줄이 부분 진행의 경계를 정한다.
/// ★인자 오류(`INVALID_SEND_ARGS`)·본문 과대(`BODY_TOO_LARGE`)는 여기 없다★ — 입구(ingress)가 서비스에
///   내려오기 전에 반려한다(entrance-agnostic 검증 단일점).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendReject {
    /// `@all` 이 아닌 `@주소`(또는 `@` 규약 위반) → `GROUP_NOT_FOUND`. 혼용 `to` 여도 **전체 반려**다
    ///   (산 수신자에게도 안 간다 — 오타가 반쯤 성공한 채 지나가지 않게).
    GroupNotFound { name: String },
    /// **최종 수신자 집합**(펼침 ∪ 명시 지목)이 비었음 → `GROUP_EMPTY`(ADR-0114 결정 3 — 원소 단위가 아니라
    ///   해석 완료 후 집합 기준). `["@all"]` 에 생존자가 발신자뿐인 경우가 전형이다.
    GroupEmpty,
    /// ★논리 메시지 id 가 장부에 이미 존재★(사실상 불가: 2.8×10^12 공간). 이력 레코드든 request 추적이든
    ///   **어느 쪽에든** 같은 id 가 있으면 이 값이다.
    ///
    /// ★부작용 없음 보장(load-bearing — 호출자 재시도 계약)★: id 검사는 이 함수의 **첫 부작용 지점**이라,
    ///   이 값을 돌려줄 때 배달·파킹·장부 레코드는 하나도 일어나지 않았다. 그래서 호출자(ingress)가 새 id 로
    ///   그대로 다시 부르면 중복 배달 없이 안전하게 재시도된다.
    IdCollision,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ProfileDeletedOutcome {
    /// ★조건 미성립★ — 게이트 두 축 중 하나라도 "아직 살아 있다" 여서 아무것도 하지 않았다.
    ///   어느 축이 막았는지는 이 값으로 구분되지 않는다(로그가 구분한다).
    ///   이 값이 `true` 면 나머지 필드는 전부 0이다.
    pub skipped_live: bool,
    /// `failed` + `RECIPIENT_DELETED` 로 종결한 파킹분 수(레인 무관 — notice 포함).
    pub failed_parked: usize,
    /// `reply_failed` 로 종결한 오픈 계약 수(그 이름이 **요청자**인 것만).
    pub failed_contracts: usize,
    /// 가드(잠정·은퇴 예정) 때문에 건너뛴 계약 수.
    pub guard_held_contracts: usize,
}

/// ★주입 영수증(ADR-0110 결정 2 — 이 crate 자체 타입)★ — `DeliveryPort` 가 "봉투 바이트를 실제로
///   꽂았다" 를 증명하며 돌려주는 4필드 값. 호스트 어댑터가 자기 write 결과를 이 모양으로 복사해 준다.
///
/// ★왜 호스트 타입을 그대로 안 받나★: 그러면 이 crate 가 호스트 crate(core)를 알아야 해 완전 상호무지가
///   깨진다. 필드 넷은 전부 순수 스칼라라 경계 변환 비용이 1회 복사뿐이다(ADR-0110 근거).
/// ★의미(관측 상관 — ADR-0088)★: `msg_uuid` 는 수신자 세션이 이 유저 턴에 부여한 replay-dedup 키,
///   `epoch` 은 write 를 **집행한** incarnation 의 세대다(해석 시점 스냅샷이 아니다 — 그 비대칭이
///   mid-flight epoch race 를 레코드만으로 단언 가능하게 한다).
/// ★`bytes_written` 은 short-write 탐지 축이 아니다★: 성공이면 by-construction 으로 `bytes_requested`
///   와 같다(배달 완결성의 1차 증거는 Ok/Err 다 — `DeliveryObservation::is_delivered`).
// ADR-0110
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InjectReceipt {
    /// 주입을 요청한 논리 메시지 바이트 수(봉투 문자열 길이).
    pub bytes_requested: usize,
    pub bytes_written: usize,
    pub msg_uuid: uuid::Uuid,
    pub epoch: u32,
}

/// ★제어 평면 포트(ADR-0110 결정 3 — 포트는 lib, 어댑터는 호스트)★ — 서비스가 호스트의 제어 평면에
///   묻는 **두 가지**만 담은 좁은 계약이다: ① 지금 쓸 봉투 포맷 ② 배달 관측 레코드 적재.
///
/// ★왜 이 둘뿐인가(실측 근거)★: 분리 전 서비스는 데몬의 `ControlRegistry` 를 통째로 들고 있었지만 실제
///   호출은 `envelope_format()` 과 `record_delivery()` 두 곳뿐이었다(ADR-0110 근거). 포트를 그 실사용
///   면적으로 깎아 두면 미래 호스트(에이전트 매니저가 아예 없는 독립 메일 서비스)가 구현할 게 두 개뿐이다.
/// ★fail-open 성질★: 두 메서드 모두 **선택적 기능**의 성격이다 — 포맷은 기본값(Xml)이 있고 관측은 운영
///   데몬에서 no-op 다. 구현이 아무것도 하지 않아도 배달은 그대로 간다(ADR-0110 영향 §"턴 관측·봉투
///   포맷은 선택").
/// ★락 밖 호출 계약★: `DeliveryPort` 와 같다 — messaging 락을 **놓은 상태**에서만 불린다.
// ADR-0110
pub trait ControlPlanePort: Send + Sync {
    /// 지금 조립에 쓸 봉투 포맷(호스트 전역 상태 — 런타임 전환 가능, ADR-0096).
    fn envelope_format(&self) -> EnvelopeFormat;
    /// 배달-경계 관측 레코드 1건을 호스트 싱크에 적재한다(ADR-0088).
    fn record_delivery(&self, obs: DeliveryObservation);
}

/// 배달 경계 seam(ADR-0012) — MessagingService 가 호스트의 에이전트 실물을 직접 부르지 않고 이 너머로 부른다.
///
/// ★왜 트레잇인가★: 헤드리스 단위 테스트가 claude 바이너리·실 PTY·spawn 파이프 없이 3분기·flush·sweep 을
///   결정적으로 검증하게 하려는 seam 이다(FakeDeliveryPort). 운영 구현은 호스트 소유
///   (데몬 `messaging_host::ManagerDeliveryPort` — ADR-0110 결정 3).
/// ★락 밖 호출 계약(load-bearing)★: 이 트레잇의 메서드는 MessagingService 의 messaging 락을 **놓은
///   상태**에서만 불린다(모듈 헤더 락 규율). 구현이 내부에서 다른 락을 잡아도 안전하도록 그 전제를 둔다.
pub trait DeliveryPort: Send + Sync {
    /// 완성된 봉투 바이트를 수신자에게 주입한다(= 수신자 stdin write). `Err` = 도달 불가·write 실패이고
    /// **상위가 파킹으로 처리**한다. 봉투 조립은 상위가 이미 끝냈다.
    ///
    /// ★이건 **incarnation 무조건** 주입이다★: 그 PeerId 가 지금 가리키는 세션에 그대로 쓴다 — 재시작으로
    ///   epoch 이 올랐어도 새 incarnation 에 착지한다. 그게 **옳다**: 주소 단위는 이름이고 재스폰 이어받기가
    ///   기능이다(ADR-0101). ★옛 `inject_if_epoch`(발송 순간 incarnation 결박용 조건부 주입)는 제거됐다★ —
    ///   결박 자체가 폐지됐으므로(ADR-0111 결정 6) 조건부 write 를 쓸 호출자가 없다. 되살리려면 먼저
    ///   "이 편지는 발송 순간 화신에게만" 을 v2 개인 메일 옵션으로 정식 재론해야 한다(spec §8).
    // ADR-0111 (결박 폐지 — 조건부 주입 동사 제거)
    fn inject(&self, to_id: PeerId, bytes: &[u8]) -> Result<InjectReceipt, String>;

    /// ★로스터 = 지금 **프로세스가 붙어 있는** 에이전트 전원(`Running`|`Exiting`)★ — (name, id, epoch,
    /// turn_signal) 스냅샷. resolve(이름→id)·flush(등장/epoch 교체 감지)·`@`주소 펼침·입구 판정 공용.
    ///
    /// ★술어는 **상태뿐**이다(load-bearing — ADR-0116 결정 1·7)★: 구조화 출력(턴 신호) 유무는 **자격
    ///   조건이 아니다**. 옛 술어는 `structured` 를 함께 걸어 터미널/콘솔 모드 세션을 로스터에서 빼고 그
    ///   부류를 반려(`RECIPIENT_UNREACHABLE`)했는데, 그 전제가 틀렸다(근거 = `LiveAgent::turn_signal`).
    ///   capability 는 멤버십이 아니라 **"언제 넣을지"** 를 가르는 축이다.
    /// ★"세션 목록에 있음" 이 아니다(load-bearing)★: 세션은 reaper 가 수거하기 전까지 맵에 남으므로, 단순
    ///   존재로 판정하면 **방금 종료된 에이전트**가 잠듦(파킹) 대신 배달 대상으로 오분류된다. 이 술어는
    ///   뮤테이션 실측에서 무방비로 확인됐다(지워도 전 테스트 초록) → 실물 어댑터 레벨 봉인 테스트가 정본.
    // ADR-0116 (로스터 술어 = 상태만 · 결정 7)
    fn live_agents(&self) -> Vec<LiveAgent>;

    /// ★입구 판정 소스 **한 장 스냅샷**(spec §5 3분기 — ADR-0116)★: 발송 1회당 **정확히 한 번** 불린다.
    ///
    /// ★왜 두 조회로 나누지 않고 한 동사로 묶나(load-bearing)★: 3분기는 두 술어(로스터 / 프로필)를 보는데,
    ///   조회를 나누면 그 사이 명단이 바뀌어 **한 발송 안에서 수신자마다 다른 세계를 보는** 반쪽 판정이
    ///   재발한다(ADR-0111 결정 2 가 금지한 부류). 한 동사로 묶으면 "세션 스냅샷 1회 + 프로필 1회" 를
    ///   어댑터가 **구조적으로** 보장한다.
    /// ★구현 계약(어댑터가 지켜야 하는 것 — 어기면 조용히 오분류된다)★:
    ///   - `roster` = `live_agents()` 와 **동일 술어**(`Running|Exiting`, capability 조건 없음)이고 같은
    ///     `list_agents()` **한 장**에서 유도한다. 두 번 뜨면 그 자체가 결함이다.
    ///   - `dormant_names` = **그 프로필의 세션이 살아 있지 않은** 프로필의 canonical 이름이고, **산 세션과
    ///     같은 규칙으로 파생**해야 한다(spec §5 — 다르면 파킹 키와 복원 후 이름이 어긋나 편지가 주인을 못
    ///     만난다). 동명 판정을 위해 **중복을 접지 않는다**(같은 이름 2개면 2개 그대로).
    // ADR-0116 (판정 소스 2종 — 물리 조회 2회)
    fn addressing_sources(&self) -> AddressingSources;

    /// ★이 id 의 세션이 **지금 살아 있나**(`Running`|`Exiting` = 로스터와 같은 술어)★.
    ///
    /// ★존재 이유 = 삭제 정리 게이트(spec §5 · ADR-0116 결정 3 — 리뷰 fix D1)★. 게이트가 **id 축**이어야
    ///   하는 근거(개명 면역)는 `handle_profile_deleted` doc 이 정본이다. 프로필 id 는 세션 id 와 같고
    ///   (`activate_profile`) 개명에 흔들리지 않으므로 그걸 축으로 쓴다.
    /// ★`addressing_sources().roster` 를 쓰지 않는 이유★: 그쪽은 프로필 목록 조회 + 잠든 이름 파생(경우에
    ///   따라 fs 접근)까지 딸린 **발송용** 한 장이다. 게이트는 단일 술어 하나만 필요하므로 좁은 동사를 따로
    ///   둔다(어댑터는 같은 `is_live` 술어를 공유해 두 판정이 갈리지 않게 한다).
    // ADR-0116 (결정 3 — 삭제 정리 게이트) / 리뷰 fix D1
    fn is_agent_live(&self, id: PeerId) -> bool;

    /// id → canonical 표시 이름(봉투 sender·수신자 이름 단일 출처, ADR-0101). 없으면 None.
    fn canonical_name(&self, id: PeerId) -> Option<String>;
}

/// ★입구 판정 소스 한 장(spec §5 3분기 · ADR-0116)★ — `DeliveryPort::addressing_sources` 의 산출물.
///
/// 두 필드는 **한 시점의 사실**이라 서로 정합해야 한다: `dormant_names` 는 `roster` 에 세션이 없는
/// 프로필들의 이름이다(교집합이 없다). 그 정합성은 어댑터가 보장하고(위 메서드의 구현 계약), 서비스는
/// 그걸 전제로 3분기를 판정한다.
#[derive(Debug, Default, Clone)]
pub struct AddressingSources {
    /// **`@here` 펼침이 보는 유일한 소스**다(spec §4 — "지금 여기 있는 전원"). `@all` 은 여기에
    ///   `dormant_names` 를 **더해서** 본다(ADR-0121 결정 1 — 두 어휘가 같은 소스를 보면 그 결정 위반이다).
    pub roster: Vec<LiveAgent>,
    /// ★**중복을 접지 않는다** — 이게 동명 판정의 입력이다★: `push_recipient` 는 이 목록에서 같은 이름의
    ///   개수를 **직접 세어** `RECIPIENT_AMBIGUOUS` 를 낸다(펼침 결과를 세는 게 아니다). 그래서 `@all` 펼침이
    ///   같은 이름을 두 번 내도 행은 하나로 접히고 결말은 달라지지 않는다 — 판정이 걸려 있는 곳은 **이 필드**다.
    pub dormant_names: Vec<String>,
}

/// ★flush 도어벨 seam(C2 리뷰 fix 11)★ — "이 에이전트의 파킹 큐를 지금 flush 해라" 를 **다른 스레드에
///   맡기는** 출구. 운영은 flush 채널로 논블록 enqueue 한다(호스트측 `messaging_host.rs`
///   `ChannelIdleNotifier` — 커널은 그 구현을 모른다).
///
/// ★왜 필요한가(ADR-0125)★: inject 는 자식 stdin **blocking write** 라, 발신 경로
///   (MCP/HTTP 요청을 처리하는 tokio 워커 스레드)에서 남의 큐까지 배치로 밀면 막힌 파이프 하나가 데몬 요청
///   처리를 잡아먹는다. 그런데 **자기가 지목한 수신자의 큐를 발신 스레드가 비우는 것은 이제 정상 경로**다
///   (동기 드레인 — ADR-0125 결정 1이 그 비용을 명시적으로 수용했다). 그래서 이 도어벨이 떼어내는 몫은
///   **자기 발송과 무관한 큐**뿐이다: 물러난 남의 드레인 요청(유예 표식 되갚기) · idle 전이 · 재등장.
/// ★계약★: `request_flush` 는 **논블록**(채널 enqueue 만). 실제 flush 는 소비자(flush lane)가 한다.
/// ★미배선 폴백(문서화된 두 갈래)★: 도어벨을 꽂지 않은 조립(실험 bin·단위 테스트)은 **인라인 flush** 로
///   폴백한다 — 도어벨 부재가 "배달이 멈춘다" 로 번지지 않게 하는 안전 기본값이다(fail-open). 대가는
///   그 조립에서 호출 스레드가 배치 write 를 지는 것뿐이고, 운영 조립(lib.rs)은 항상 도어벨을 꽂는다.
pub trait FlushTrigger: Send + Sync {
    /// 중복 요청은 소비자가 접는다(coalescing).
    fn request_flush(&self, id: PeerId);
}

#[derive(Debug, Clone)]
pub struct LiveAgent {
    pub id: PeerId,
    pub name: String,
    pub epoch: u32,
    /// ★턴 경계를 관측할 수 있나(= 이 수신자에게 idle 게이트가 성립하나) — ADR-0116 결정 7 · load-bearing★.
    ///
    /// `true`(구조화 출력 백엔드) → 주입 전에 `BusyGate` 에 묻고 busy 면 파킹한다.
    /// `false`(터미널/콘솔 모드) → **idle 게이트를 묻지 않는다**. 근거는 그 CLI 가 **자기 입력 큐**를 갖고
    /// 있다는 관측이다(턴 중 입력을 물고 있다가 턴 후 소비) — 우리가 idle 을 관측할 이유가 없다. 그래서 이
    /// 값은 **멤버십이 아니라 타이밍**만 가른다.
    /// ★이 값이 가르지 **않는** 것 = 순서(ADR-0121 결정 2 · ADR-0125)★: 위 근거는 "받을 준비가 됐나" 만
    /// 정당화하고 "어떤 순서로 도착하나" 는 정당화하지 않는다.
    // ADR-0116 (결정 7)
    // ADR-0121 (결정 2 — 순서는 부류 무관)
    pub turn_signal: bool,
}

struct MessagingState {
    mailbox: Mailbox,
    ledger: Ledger,
    /// ★유예된 flush 재-도어벨 장부(round-7 · round-8 보정 · load-bearing)★ — 키 = park 큐 이름,
    /// 값 = 그 큐에 대해 **물러난 모든 도어벨 id**(중복 제거된 집합).
    ///
    /// `drain_queue` 는 같은 수신자에 대해 **겹쳐 돌지 않는다**(앞 배치가 락 밖에서 주입 중이면 뒤 배치가
    /// 그 잔여를 앞지른다 — 순서 역전). 그래서 뒤 드레인은 큐를 건드리지 않고 물러나는데, 물러나기만 하면
    /// 그 깨우기가 **증발한다**(lost wakeup — C1 finding 3 과 같은 실패 모드). 여기에 사실을 남겨 두면
    /// **영수증을 쥔 쪽**(진행 중인 드레인)이 정산을 마치고 나가면서 도어벨을 다시 눌러 준다.
    /// ★7차엔 이 표식이 동시 발송의 편지를 여는 계기이기도 하다(ADR-0125)★ — 물러난 발신자는 응답을
    /// `pending`(확인 불가)으로 답했고, 그 편지의 결말은 이긴 쪽 배치 아니면 이 되울림이 연다.
    ///
    /// ★왜 id 하나가 아니라 **집합**인가(round-8 high — 옛 단일 슬롯이 틀렸던 지점)★: 한 이름 큐는 여러
    /// id 로 열린다 — 같은 이름의 산 incarnation, 개명 전 이름 큐를 힌트로 여는 옛 id, 죽어 가는
    /// incarnation 의 늦은 idle 통지 등(`flush_for_agent` 가 이름 큐 + 힌트 큐를 함께 여는 이유와 같은
    /// 사정). 단일 슬롯은 **last-writer-wins** 라, 산 id 가 먼저 물러나고 그 뒤 이미 reap 된 stale id 가
    /// 물러나면 산 id 의 깨우기가 덮여 사라진다. 그러면 정산 후 되울린 건 stale id 뿐이고,
    /// `flush_for_agent` 는 `canonical_name == None` 에서 조기 반환하므로 **아무도 드레인하지 않는다** —
    /// 산 수신자가 멀쩡히 있는데도 admitted 메일이 다음 사건이나 TTL 까지 묶인다. 종료성만 지켜지고
    /// 깨우기의 **유용성**이 깨진 셈이라, 요청된 깨우기를 하나도 잃지 않도록 전부 보관한다.
    /// ★왜 "이름만 저장하고 정산 시점에 로스터로 다시 푼다" 가 아닌가★: 그 해석은 **이름으로 풀리는 id**
    /// 만 되살린다 — 힌트 큐를 열려던 id(개명·동명 다수)는 이름 해석으로 돌아오지 않아 같은 유실이 남는다.
    /// 게다가 정산 경로에 port 호출(로스터)이 새로 끼어든다. 요청된 id 를 그대로 보관하는 쪽이 더 작고 더
    /// 정확하다.
    /// ★유계★: 한 배치가 도는 동안 실제로 경합한 **서로 다른** id 수만큼이라(중복은 넣지 않는다) 사람
    /// 대화 수준에서 0~2개다. 선형 `contains` 로 충분하다(HashSet 이 필요할 규모가 아니다).
    /// ★왜 "큐가 비지 않았으면 무조건 재-도어벨" 이 아닌가★: 그러면 배달 불가(busy·타깃 없음)로 파킹이
    /// 유지되는 정상 상태에서 flush 가 자기 자신을 무한히 재기동한다. 실제 유예가 있었을 때만, 유예한
    /// id 마다 정확히 1회 되울린다(집합은 유한하고 정산 때 통째로 꺼내므로 종료성은 그대로다).
    // ADR-0107 (flush 중첩 유예 마커 — id 집합, 전원 재타)
    deferred_flush: HashMap<String, Vec<PeerId>>,
}

/// ★공유★: 데몬 부팅에서 `Arc<MessagingService>` 로 만들어 ingress(handle_send)·flush observer·sweep
///   task 가 같은 인스턴스를 공유한다(lib.rs run()).
pub struct MessagingService {
    state: Mutex<MessagingState>,
    /// ★`@`주소 해석 소스(seam — ADR-0104 결정 1 · ADR-0112 결정 1)★. v1 은 내장 `@all` 하나뿐이고 **상태가
    ///   없어** 락 밖 필드다(저장형 그룹이 사라져 공유 가변 상태가 없다 — groups.rs 헤더).
    groups: BuiltinGroups,
    port: Arc<dyn DeliveryPort>,
    registry: Arc<dyn ControlPlanePort>,
    /// 운영은 `BusyPolicy`, 미배선/관측 불가는 `AlwaysIdleGate`(즉시 주입 폴백 — busy.rs 헤더).
    busy: Arc<dyn BusyGate>,
    /// `None` = 미배선 → 인라인 드레인 폴백(FlushTrigger 주석의 문서화된 두 갈래).
    trigger: Option<Arc<dyn FlushTrigger>>,
    /// ★수용 임계구역 관측 훅(테스트 전용 — 리뷰 fix D2 원자성 단언)★: **수용을 기록한 그 락을 아직 쥔 채**
    ///   장부를 그대로 보여 준다. 발화 지점은 `handle_send` 주 임계구역 이탈 직전 **하나**다.
    ///
    /// ★왜 이 모양이어야 하나(결정론)★: "수용과 계약 닫기가 한 임계구역" 은 **최종 상태만 봐선 단언할 수
    ///   없다** — 두 락으로 갈라도 경합이 없으면 결과가 같다. 그래서 테스트는 **락 안에서** 계약이 이미 닫혔는지
    ///   본다: 닫기가 다른 락으로 빠지면 이 시점 관측이 `awaiting_reply` 라 즉시 빨개진다(sleep·스레드 없음).
    /// ★훅 계약★: 넘겨받은 장부만 읽는다 — **서비스 락을 다시 잡지 않는다**(std Mutex 는 비재진입 = 데드락).
    /// ★`OnceLock` 인 이유★: 읽기에 락이 필요 없어(state 락 보유 중 호출) 락 순서 규약을 건드리지 않는다.
    #[cfg(any(test, feature = "test-harness"))]
    accept_hook: std::sync::OnceLock<Arc<dyn Fn(&Ledger) + Send + Sync>>,
}

impl MessagingService {
    /// 게이트를 검증하는 조립은 `new_gated` 를 쓴다.
    pub fn new(port: Arc<dyn DeliveryPort>, registry: Arc<dyn ControlPlanePort>) -> Self {
        Self::new_gated(port, registry, Arc::new(AlwaysIdleGate))
    }

    pub fn new_gated(
        port: Arc<dyn DeliveryPort>,
        registry: Arc<dyn ControlPlanePort>,
        busy: Arc<dyn BusyGate>,
    ) -> Self {
        Self {
            state: Mutex::new(MessagingState {
                mailbox: Mailbox::new(),
                ledger: Ledger::new(),
                deferred_flush: HashMap::new(),
            }),
            groups: BuiltinGroups,
            port,
            registry,
            busy,
            trigger: None,
            #[cfg(any(test, feature = "test-harness"))]
            accept_hook: std::sync::OnceLock::new(),
        }
    }

    /// 1회만 설치된다(멱등, 이후 무시).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn set_accept_hook_for_test(&self, hook: Arc<dyn Fn(&Ledger) + Send + Sync>) {
        let _ = self.accept_hook.set(hook);
    }

    #[cfg(any(test, feature = "test-harness"))]
    fn fire_accept_hook(&self, ledger: &Ledger) {
        if let Some(h) = self.accept_hook.get() {
            h(ledger);
        }
    }

    /// 생성자 인자로 안 받는 이유: 도어벨은 flush 채널 = 서비스보다 **뒤에 조립되는** 배선이고, 게이트
    ///   미검증 조립(실험 bin·단위 테스트)은 이걸 꽂지 않아도 동작해야 한다(폴백 = 인라인 flush).
    pub fn with_flush_trigger(mut self, trigger: Arc<dyn FlushTrigger>) -> Self {
        self.trigger = Some(trigger);
        self
    }

    /// ★"이 수신자가 지금 턴 중인가" 의 **유일한 질의 지점**(ADR-0121 §영향 불변식 · load-bearing)★.
    ///
    /// idle 게이트는 **턴 경계를 관측할 수 있는 백엔드에만** 적용된다(ADR-0116 결정 7). 그 부류 판정
    /// (`turn_signal`)과 실제 busy 조회를 **한 함수 안에서** 묶어, 판정 지점들이 서로 다른 술어를 쓰는 것이
    /// 불가능하게 한다 — 같은 조건을 여러 곳에 각각 적으면 한쪽만 고쳐져 두 지점이 **다른 세계를 본다**
    /// (2026-07-30 리뷰가 다른 곳에서 잡은 결함 부류).
    /// 부르는 지점이 하나여도 술어를 한 곳에 두는 규율은 유지한다(드레인 안에서 부류 판정이 갈라지면 같은
    /// 사고가 재현된다).
    ///
    /// ★턴 신호 없는 부류에 게이트를 묻지 않는 근거는 순서까지 덮지 않는다★: "받을 준비가 됐나" 와 "편지가
    /// 어떤 순서로 도착하나" 는 다른 문제고, 후자는 **적재 순서**가 답한다(ADR-0125). 그래서 이 함수는
    /// 큐를 보지 않는다(그걸 여기 섞으면 두 규칙이 다시 한 조건으로 엉킨다).
    // ADR-0121 (게이트 술어 단일 정의)
    // ADR-0116 (결정 7 — 턴 신호 없는 부류는 게이트 생략)
    fn gate_says_busy(&self, target: &LiveAgent) -> bool {
        target.turn_signal && self.busy.is_busy(target.id, target.epoch)
    }

    /// 입구(ingress)가 인자 검증·auth 를 마친 뒤 부르는 유일한 발송 함수다.
    ///
    ///   - `msg_id`: 상위가 부여한 논리 메시지 id(ledger 상관·응답 `id` 축).
    ///   - `from`: 발신자 신원(토큰 파생, ADR-0086). 관측 레코드·`@all` 발신자 제외에 쓴다.
    ///   - `sender_name`: 봉투 sender 표시 이름(상위가 canonical 로 이미 해석 — WYSIWYA ADR-0101).
    ///   - `to`: **수신자 토큰 목록**. 각 원소 = 에이전트 이름 · 정확한 PeerId 문자열 · `@`주소(혼용 가능).
    ///     콤마 분해는 **CLI 입구 전용**이라 여기 오기 전에 끝나 있다(MCP 배열 원소는 절대 쪼개지 않는다 —
    ///     spec §6). 빈 목록은 입구가 `INVALID_SEND_ARGS` 로 반려하므로 여기선 비어 있지 않다.
    ///   - `body`: **감싸지 않은** 본문(봉투 조립은 주입 시점 wrap_message — 단일 wrap point).
    ///
    /// ★반환★: 수신자별 결과 N행(`results[]`, spec §6) 또는 **발송 단위 반려**(`SendReject` — 주소 공간
    ///   오류·id 충돌뿐. 부작용 0). **전원이 실패해도 `Ok(전 행 failed)`** 다 — 전체 반려로 승격하지 않는다
    ///   (단일 수신자 부재도 예외가 아니다: 답은 `error` 가 아니라 `failed` 행 1개다 — spec §5 부분 진행).
    ///
    /// ★응답 어휘★: 드레인이 **이번 편지를 실제로 주입했으면 `delivered`**, 확인하지 못했으면 `pending`
    ///   이다(어휘 정의 = spec §6).
    ///
    /// ★수신자별 3분기(spec §5) — 모듈 헤더가 정본★. 이 함수는 그 판정을 **두 패스**로 나눈다.
    ///   ★왜 두 패스인가(load-bearing)★: 파킹은 봉투 재료를 저장하는데 `to` 는 전원 판정 전엔 알 수 없다.
    ///   한 패스로 하면 첫 수신자의 파킹분이 **미완성 `to`** 를 굳혀 나중 배달분과 봉투가 갈린다.
    ///
    /// ★회신 계약(spec §3 · ADR-0111 결정 5)★: `meta.request` 면 **수용된 수신자마다** 계약을 연다(키 =
    ///   `(msg_id, 수신자)`). 그 오픈 판정은 **적재 전**에 끝난다 — 뒤로 미루면 추적 없는 request 가
    ///   배달된다(spec §3 항목 5).
    ///   `meta.reply_to` 면 발송이 접수된 뒤 회신자 기준 엄격 매칭으로 계약을 닫는다(실패해도 배달 무영향).
    // ADR-0111 (다중 수신자 fan-out · 부재 반려 · 부분 진행)
    // ADR-0114 (MAILBOX_FULL 행 실패 — 회수 시도 없음)
    // ADR-0125 (전부 적재 후 동기 드레인 — 직발송 지름길 폐지 · delivered 복원)
    #[allow(clippy::too_many_arguments)]
    pub fn handle_send(
        &self,
        msg_id: &str,
        from: SenderIdentity,
        sender_name: &str,
        to: &[String],
        body: &str,
        entrance: Entrance,
        meta: &SendMeta,
    ) -> Result<Vec<RecipientResult>, SendReject> {
        // ★상호배타는 ingress 가 **유일한** 검증자다★: 이 함수와 `SendMeta` 는 pub 이라 다른 조립(스모크
        //   bin·미래 입구)이 직접 부를 수 있는데, request+reply_to 를 동시에 실으면 한 발송이 계약을 열면서
        //   남의 계약을 닫는 뒤엉킨 상태가 된다. 여기서 검증을 **복제하지 않는 이유**는 반려 코드/문구가
        //   입구마다 갈리면 안 되기 때문이고(entrance-agnostic), 대신 디버그 빌드에서 배선 실수를 터뜨린다.
        debug_assert!(
            !(meta.request && meta.reply_to.is_some()),
            "ingress가 유일 검증자 — request와 reply_to는 상호배타(spec §6)"
        );
        debug_assert_eq!(
            meta.reply_by.is_some(),
            meta.reply_by_raw.is_some(),
            "reply_by 는 파싱값과 표기 원본을 쌍으로 나른다(SendMeta)"
        );
        debug_assert!(
            meta.to_attr.is_none(),
            "봉투 to 는 전 수신자 수용 판정 뒤 handle_send 가 1회 확정한다(spec §1)"
        );
        debug_assert!(
            !to.is_empty(),
            "빈 수신자 목록은 입구가 INVALID_SEND_ARGS 로 반려한다(spec §6)"
        );

        // 1) 판정 소스 스냅샷 1장(락 밖).
        let sources = self.port.addressing_sources();
        // ★이름 폴백 허용 판정은 **락 밖 스냅샷**으로 여기서 한 번 한다(리뷰 fix D4)★: 회신자 이름이 산
        //   세션 둘 이상에 걸리면 이름으로 남의 계약을 닫을 수 있어(귀속 날조) 폴백을 금지한다.
        let mut closing = ReplyClosing {
            in_reply_to: meta.reply_to.as_deref(),
            replier_name: sender_name,
            replier_id: from.peer_id,
            allow_name_fallback: sources
                .roster
                .iter()
                .filter(|a| a.name == sender_name)
                .count()
                <= 1,
            done: None,
        };

        // 2) 주소 해석(순수).
        let addressing = resolve_addressing(&self.groups, to, from, sender_name, &sources)?;
        // 이 발송이 남길 배달기록 총수 = **실패 행 포함** 수신자 수(spec §5 "실패 수신자도 장부에 남는다").
        //   조회의 잘림 판정(`may_be_truncated`)이 이 값과 남은 행 수를 비교하므로 모든 행이 같은 값을 든다.
        let expected_rows = u16::try_from(addressing.recipients.len()).unwrap_or(u16::MAX);
        let roster_names = live_names_hint(&sources.roster);

        let now = Instant::now();
        let mut plans: Vec<RecipientPlan> = Vec::with_capacity(addressing.recipients.len());
        let mut park_effects = ParkSideEffects::default();
        let mut retirements = RetirementLog::default();
        let mut duplicate_contracts: Vec<String> = Vec::new();

        {
            let mut st = self.state.lock().expect("messaging state poisoned");
            // ★id 충돌 검사는 **첫 부작용 지점**이다(부작용 0 반려 계약 — `SendReject::IdCollision`)★.
            //   ★남는 창: "동시에 뽑힌 같은 fresh id" — 전면 예약을 **거부한 결정**★ 이 검사는 이미 장부에
            //   있는 id 와의 충돌만 잡는다. 두 발송이 같은 값을 동시에 뽑아 둘 다 통과하는 창은 남는데,
            //   ① 확률 = 36^8(2.8×10^12) 공간에서 마이크로초 창 안의 동일 값 ② 피해 = 이력 레코드 앨리어싱
            //   (배달·계약 키는 `(msg_id, 수신자)` 라 수신자가 다르면 서로를 덮지 않는다) ③ 예약 생명주기
            //   배선의 버그 확률이 그보다 훨씬 크다. 그래서 검사만 하고 예약은 두지 않는다.
            if st.ledger.msg_id_in_use(msg_id) {
                return Err(SendReject::IdCollision);
            }

            // ── pass A: 수신자별 수용 판정 ────────────────
            for r in &addressing.recipients {
                // 2-a) 입구 3분기(spec §5 · ADR-0116 결정 1).
                let target = match r.target.clone() {
                    Some(t) => Some(t),
                    None => match absent_disposition(r, &roster_names) {
                        AbsentDisposition::Failed { code, hint } => {
                            // ★실패 수신자도 장부에 남는다(spec §5)★ — 종점 행이라 더 움직이지 않는다.
                            st.ledger.record_with_expected(
                                msg_id,
                                sender_name,
                                &r.key,
                                body,
                                DeliveryStatus::Failed,
                                now,
                                expected_rows,
                            );
                            plans.push(RecipientPlan::Failed { code, hint });
                            continue;
                        }
                        // ★잠듦 = 수용(ADR-0116 결정 1)★ — 도어벨은 없다(누를 id 가 없다 — 재등장 flush 가
                        //   집는다).
                        AbsentDisposition::Dormant => None,
                    },
                };

                // 2-b) ★파킹도 계약을 연다 — 잠듦 포함(spec §3 항목 2 · ADR-0116 결정 1)★: 언젠가 주입될
                //      메시지라 회신 의무가 성립하고, 그래서 기한 스윕도 정상 발화한다.
                let mut contract: PendingContract = None;
                if meta.request {
                    let reply_by = meta.reply_by.zip(meta.reply_by_raw.clone());
                    match st.ledger.open_request(
                        msg_id,
                        sender_name,
                        from.peer_id,
                        &r.key,
                        target.as_ref().map(|t| t.id),
                        reply_by,
                        now,
                    ) {
                        OpenOutcome::Opened => {
                            contract = Some(open_reservation(&mut st, self, msg_id, &r.key, None))
                        }
                        // cap 압력으로 가장 오래된 은퇴 가능 계약에 **은퇴 예정 표시**가 붙었다(mark-and-sweep,
                        //   ADR-0108). 실제 제거는 아래 커밋에서 — 이 수신자가 실패로 끝나면 표시를 되돌린다.
                        OpenOutcome::OpenedAfterMarking(rc) => {
                            contract =
                                Some(open_reservation(&mut st, self, msg_id, &r.key, Some(rc)))
                        }
                        // 수신자는 해석 단계에서 이미 중복 제거되므로 `(id, 수신자)` 중복은 도달 불가다.
                        OpenOutcome::DuplicateId => duplicate_contracts.push(r.key.clone()),
                        OpenOutcome::Full => {
                            st.ledger.record_with_expected(
                                msg_id,
                                sender_name,
                                &r.key,
                                body,
                                DeliveryStatus::Failed,
                                now,
                                expected_rows,
                            );
                            plans.push(RecipientPlan::Failed {
                                code: FailCode::RequestCapacity,
                                hint: format!(
                                    "Too many replies are still outstanding, so the daemon could not open a reply contract for '{}' — it was NOT delivered. Send it as a plain notification, or wait for earlier requests to be answered or to time out.",
                                    r.display
                                ),
                            });
                            continue;
                        }
                    }
                }

                // 2-c) ★적재는 무조건이다(ADR-0125 · spec §5 — load-bearing)★. 입구는 "지금 넣을 수
                //      있나" 를 **묻지 않는다**: busy 도(게이트) 선행분도(옛 합류 판정) 조회하지 않고 큐
                //      꼬리에 넣는다. 여기에 게이트를 되돌리면 적재를 건너뛰는 갈래 = 직발송이 부활하고,
                //      순서가 적재 순서에서 풀린다(그 순간 좌석 예약·합류 판정이 다시 필요해진다 = 개편
                //      역주행 신호).
                //      ★적재 여부를 가르는 유일한 축 = 보관함 상한★ — 그래서 **모든 발송이 cap 게이트를
                //      지난다**(직발송이 cap 을 우회하던 5차 계약은 폐지됐다: `MAILBOX_FULL` 이 늘어나는
                //      것은 설계 귀결이지 회귀가 아니다 — spec §5).
                if st.mailbox.can_admit(&r.key, ParkKind::Message) {
                    plans.push(RecipientPlan::Park {
                        contract,
                        target: target.clone(),
                    });
                    continue;
                }
                // 방금 연 계약은 되돌린다(배달된 적 없는 요청이 기한 초과 notice 를 쏘는 유령 타임아웃 차단).
                if let Some(res) = contract {
                    res.rollback(&mut st);
                }
                st.ledger.record_with_expected(
                    msg_id,
                    sender_name,
                    &r.key,
                    body,
                    DeliveryStatus::Failed,
                    now,
                    expected_rows,
                );
                plans.push(RecipientPlan::Failed {
                    code: FailCode::MailboxFull,
                    hint: mailbox_full_hint(&r.display),
                });
            }

            // ── 봉투 `to` 동결(spec §1) ─────────────────────
            let admitted: Vec<bool> = plans
                .iter()
                .map(|p| !matches!(p, RecipientPlan::Failed { .. }))
                .collect();
            let admitted_count = admitted.iter().filter(|ok| **ok).count();
            // ★동결은 **주입보다 먼저**여야 한다(M7 — 판정 고정, 리뷰 blind r2 #3 ACCEPTED)★: 봉투는
            //   주입 시점에 조립되는데 그 재료인 `to` 는 여기서 굳는다.
            //   ★"결말 뒤에 wrap" 으로 고치지 말 것★ — 그건 아래 드레인의 `delivered` 회계를 통째로
            //   무너뜨린다(응답이 전부 pending 으로 뭉개진다).
            let fanout_meta = SendMeta {
                to_attr: (admitted_count >= 2).then(|| build_to_attr(&addressing, &admitted)),
                ..meta.clone()
            };

            // ── pass B: 실제 적재 ─────────────────
            let park_targets: Vec<usize> = plans
                .iter()
                .enumerate()
                .filter_map(|(i, p)| matches!(p, RecipientPlan::Park { .. }).then_some(i))
                .collect();
            for i in park_targets {
                let (hinted_id, contract) = match &mut plans[i] {
                    RecipientPlan::Park { target, contract } => {
                        (target.as_ref().map(|t| t.id), contract.take())
                    }
                    _ => unreachable!("park_targets 는 Park 만 담는다"),
                };
                let r = &addressing.recipients[i];
                match park_into(
                    &mut st,
                    ParkRequest {
                        msg_id,
                        sender_name,
                        from,
                        entrance,
                        recipient: &r.key,
                        body,
                        // ★잠듦은 `None`★ — 그 순간 산 실체가 없으므로 이름 큐로만 열린다(복원 후
                        //   canonical 이름 = 파킹 키라는 전제가 load-bearing).
                        hinted_id,
                        kind: ParkKind::Message,
                        meta: &fanout_meta,
                        expected_rows,
                    },
                    now,
                    &mut park_effects,
                ) {
                    Ok(()) => {
                        if let Some(res) = contract {
                            res.commit(&mut st, &mut retirements);
                        }
                        closing.close_in_lock(&mut st);
                    }
                    // ★도달 불가 경로★: 바로 위에서 `can_admit` 이 통과했고 그 사이 같은 락을 놓지 않았다.
                    //   그래도 조용히 삼키지 않는다 — 저장소 회계가 어긋났다는 뜻이므로 실패 행으로 강등한다.
                    Err(ParkError::MailboxFull) => {
                        if let Some(res) = contract {
                            res.rollback(&mut st);
                        }
                        st.ledger.record_with_expected(
                            msg_id,
                            sender_name,
                            &r.key,
                            body,
                            DeliveryStatus::Failed,
                            now,
                            expected_rows,
                        );
                        plans[i] = RecipientPlan::Failed {
                            code: FailCode::MailboxFull,
                            hint: mailbox_full_hint(&r.display),
                        };
                    }
                }
            }
            // 훅을 이 임계구역 **밖**으로 내리면 그 자체가 창을 만들어 테스트가 무의미해진다.
            #[cfg(any(test, feature = "test-harness"))]
            self.fire_accept_hook(&st.ledger);
            drop(st);

            park_effects.log("send");
            for name in &duplicate_contracts {
                tracing::error!(
                    msg_id = %msg_id,
                    recipient = %name,
                    "회신 계약 키 (msg_id, 수신자) 중복 — 수신자 중복 제거가 어긋났다는 배선 결함 신호(ADR-0111 결정 5)"
                );
            }

            // 3) 동기 드레인(락 밖 — ADR-0125 결정 1).
            let mut deferred_inline: Vec<PeerId> = Vec::new();
            let mut results: Vec<RecipientResult> = Vec::with_capacity(plans.len());
            for (i, plan) in plans.into_iter().enumerate() {
                let display = addressing.recipients[i].display.clone();
                let key = addressing.recipients[i].key.clone();
                let result = match plan {
                    RecipientPlan::Failed { code, hint } => RecipientResult {
                        to: display,
                        status: SendStatus::Failed,
                        code: Some(code),
                        hint: Some(hint),
                    },
                    RecipientPlan::Park { target: None, .. } => RecipientResult {
                        to: display.clone(),
                        status: SendStatus::Pending,
                        code: None,
                        hint: Some(park_hint_dormant(&display)),
                    },
                    RecipientPlan::Park {
                        target: Some(t), ..
                    } => {
                        let report = self.drain_queue(&key, t.id, Some(&sources.roster));
                        if report.injected.iter().any(|id| id == msg_id) {
                            RecipientResult {
                                to: display,
                                status: SendStatus::Delivered,
                                code: None,
                                hint: None,
                            }
                        } else {
                            // ★못 낸 몫에는 열 계기가 남아야 한다(spec §5 "고립 없음")★ — 하지만 도어벨을
                            //   누르는 조건은 좁다: ① 물러난 경우(㉯)는 **이긴 쪽이 정산하며 되울려 준다**
                            //   (`deferred_flush` — 여기서 또 누르면 표식만 왕복한다) ② **자기 주입 실패로**
                            //   누르는 것은 금지다(도달 불가해진 수신자에 재주입 반복 — spec §5). 남는
                            //   갈래(게이트에 걸림 · 드레인 시점에 타깃이 안 풀림)만 누른다.
                            if !report.retreated && report.inject_error.is_none() {
                                self.ring_or_defer(t.id, &mut deferred_inline);
                            }
                            RecipientResult {
                                to: display.clone(),
                                status: SendStatus::Pending,
                                code: None,
                                hint: Some(pending_hint(&display, &report)),
                            }
                        }
                    }
                };
                results.push(result);
            }

            // ★은퇴 계측은 **결말 루프 뒤**에서 찍는다(M1)★: 커밋이 pass B·주입 루프 안으로 내려갔으므로
            //   (A2) 루프 **전**에 찍으면 주 경로(idle 수신자 request)의 은퇴가 **로그를 하나도 남기지 않는다**
            //   — ADR-0108 결정 2 에서 그 info 로그가 은퇴의 **유일한** 증거다.
            log_contract_retirements(msg_id, &retirements.real);
            // ★계획했으나 일어나지 않은 은퇴(R2)★ — 은퇴로 세지 않고 이상으로 남긴다(계측 오염 금지).
            for r in &retirements.phantom {
                tracing::warn!(
                    planned_msg_id = %r.request_id,
                    to = %r.recipient,
                    new_msg_id = %msg_id,
                    "은퇴 예정 표시했던 계약이 커밋 시점에 이미 없었다 — 은퇴는 일어나지 않았다(상한 압력이 한 번 덜 풀렸다)"
                );
            }

            // 4) 도어벨 미배선 조립의 인라인 드레인(`ring_or_defer` 분업).
            //    ★이것도 직발송이 아니다(spec §5 예외 항목)★: 발신 스레드가 **공용 드레인 루틴**을 그 자리
            //    에서 도는 것이다(커널 단위 테스트의 결정성 수단이기도 하다). "발신 스레드 주입" 으로 오독해
            //    걷어내면 하네스가 비결정이 되고 정상 경로도 함께 잘린다.
            for id in deferred_inline {
                self.flush_for_agent(id);
            }

            // 5) 회신 계약 처리(spec §3 항목 7-④ · ADR-0116 결정 2 · ADR-0118).
            //
            // ★왜 이 세 갈래인가(load-bearing — 두 방향의 장부 거짓말을 동시에 피한다)★:
            //   - 옛 구현(무조건 닫기)은 **도달하지 않은 메시지를 `replied` 로 주장**했다. 요청자가 죽은 뒤
            //     일꾼이 회신하면 메시지는 아무에게도 가지 않았는데 계약이 닫혀 일꾼의 `reply_owed_by_me` 가
            //     사라지고 기한 통지도 영영 안 나갔다.
            //   - 개편기 잠정안(A1 — 사유 무관 오픈 유지)은 그 반대로 **갈 곳 없는 계약을 영구 방치**했다.
            //   - 재가된 규칙은 둘을 가른다: **파킹도 수용이다**("꽂으면 계약 완료" — 잠든 요청자에게 회신이
            //     쌓이면 회신자의 의무는 그 시점에 이행됐다. 그 뒤 요청자가 삭제돼 그 파킹분이
            //     `RECIPIENT_DELETED` 로 치워져도 **계약은 `replied` 로 남는다** — 되돌리지 않는다: 계약 축은
            //     "답을 했나" 에, 배달 축은 "도착했나" 에 각자 사실을 적는다, spec §6 축 구분). 요청자 이름이
            //     **세상에 없을 때만**(`RECIPIENT_NOT_FOUND`) 갈 곳이 없으므로 실패 종결한다. 보관함 가득·
            //     동명·도달 불가는 **그 순간의 환경**이라 재시도가 성공할 수 있어 무동작이다(닫으면 재시도가
            //     실제로 배달돼도 장부는 영영 "회신 실패" — 거짓말의 반대 방향).
            // ★회신 경로에서 발생하지 않는 코드★: `REQUEST_CAPACITY`(회신은 `reply_to`↔`request` 상호배타라
            //   계약을 열지 않는다) · `RECIPIENT_DELETED`(발송 응답이 아니라 조회 시점 코드) — 분기를 두지 않는다.
            // ADR-0116 (결정 2) / ADR-0118 (결정 1·2·3)
            if let Some(in_reply_to) = &meta.reply_to {
                match (closing.done, reply_disposition(&results)) {
                    // 수용 → in-lock 으로 이미 닫혔다. 결과만 락 밖에서 찍는다.
                    (Some(outcome), _) => log_reply_close(outcome, in_reply_to, sender_name),
                    (None, ReplyDisposition::RequesterGone) => self.fail_reply_contract(
                        in_reply_to,
                        sender_name,
                        from.peer_id,
                        closing.allow_name_fallback,
                    ),
                    (None, ReplyDisposition::NoOp) => tracing::debug!(
                        in_reply_to,
                        "회신이 일시 사유(보관함 가득·동명)로 실패 — 계약은 **오픈 유지**(무동작). 재시도가 정상 경로로 닫는다(spec §3 항목 7-④)"
                    ),
                    // ★수용 행이 있는데 in-lock 닫기가 안 됐다 = 배선 결함★: 수용 갈래는 전부
                    //   `close_in_lock` 을 거치므로 도달 불가다. 조용히 넘기지 않고 남긴다(계약이 안 닫힌 채
                    //   기한 통지가 나가는 결말이라 관측이 필요하다).
                    (None, ReplyDisposition::Accepted) => tracing::error!(
                        in_reply_to,
                        replier = %sender_name,
                        "회신이 수용됐는데 계약 닫기가 임계구역에서 일어나지 않았다 — 수용 갈래 배선 누락(리뷰 fix D2)"
                    ),
                }
            }
            Ok(results)
        }
    }

    /// ★회신 계약 닫기★ — 엄격 매칭 + **회신자 기준 계약 선택**(ADR-0111 결정 5). 결과에 따라 **로깅만**
    /// 갈린다(배달·응답에는 영향 없음 — spec §3 항목 7-②).
    ///
    /// ★매칭 실패(`NoMatch`/`AlreadyClosed`)에 새 에러를 만들지 않는다★: 고칠 수 없는 상태이고, 반려하면
    ///   이미 배달된 메시지에 대해 재시도를 유발해 중복이 난다.
    /// ★운영 발송 경로는 이 동사를 쓰지 않는다 — **테스트 seam 전용이다**(리뷰 fix D2)★. 운영은 **수용을
    ///   기록한 그 락 구간에서** `ReplyClosing::close_in_lock` 으로 닫는다(그 struct 헤더가 근거 정본): 두 락
    ///   으로 갈리면 그 사이 삭제 정리가 계약을 `reply_failed` 로 닫아 "수용됐는데 회신 실패" 가 된다.
    ///   그래서 이 동사는 cfg 로 **운영 빌드에서 컴파일되지 않게** 막아 둔다 — 남겨 두면 다음 세션이 "간단한
    ///   쪽" 을 골라 그 레이스를 되살린다(레거시 파킹 seam 만 쓴다 — 그래서 `cfg(test)` 단독이다:
    ///   `test-harness` 피처만 켠 빌드엔 호출자가 없다).
    #[cfg(test)]
    fn close_reply_contract(
        &self,
        in_reply_to: &str,
        replier_name: &str,
        replier_id: PeerId,
        allow_name_fallback: bool,
    ) {
        let outcome = {
            let mut st = self.state.lock().expect("messaging state poisoned");
            st.ledger.close_on_reply(
                in_reply_to,
                replier_name,
                replier_id,
                allow_name_fallback,
                Instant::now(),
            )
        };
        log_reply_close(outcome, in_reply_to, replier_name);
    }

    /// ★회신 계약 **실패 종결**(spec §3 항목 7-④ · ADR-0116 결정 2 · ADR-0118 결정 1)★ — 회신 발송 행이
    /// `RECIPIENT_NOT_FOUND` 였을 때만 불린다(요청자 이름이 로스터·프로필 전부에 없다 = 갈 곳이 없다).
    ///
    /// ★이 동사가 하는 일 = 계약 축 종결뿐★: 배달 축은 이미 실패 행으로 기록됐고(회신 메시지 자체의 결말),
    ///   원 request 의 배달기록은 그대로 `delivered` 에 머문다(spec §6 축 구분 — `delivered → reply_failed`
    ///   같은 배달 상태 전이는 존재하지 않는다).
    /// ★수용 갈래와 달리 **락을 갈라도 안전하다**(리뷰 fix D2)★: 이 경로엔 수용이 없다(전 행 실패) — 그
    ///   사이 삭제 정리가 같은 계약을 `reply_failed` 로 닫아도 결말이 같으므로 되돌림이 성립하지 않는다.
    /// ★인자 `allow_name_fallback`★: `close_on_reply` 와 **같은 값**을 받는다 — 성공 경로와 실패 경로가 다른
    ///   계약을 지목하면 안 된다(`Ledger::match_contract_for_replier` doc).
    // ADR-0116 (결정 2) / ADR-0118 (결정 1·3) / 리뷰 fix D4
    fn fail_reply_contract(
        &self,
        in_reply_to: &str,
        replier_name: &str,
        replier_id: PeerId,
        allow_name_fallback: bool,
    ) {
        let outcome = {
            let mut st = self.state.lock().expect("messaging state poisoned");
            st.ledger.fail_on_undeliverable_reply(
                in_reply_to,
                replier_name,
                replier_id,
                allow_name_fallback,
            )
        };
        match outcome {
            ReplyFailOutcome::Failed => tracing::info!(
                in_reply_to,
                replier = %replier_name,
                "회신이 도달 불가 확정(요청자 이름이 어디에도 없음) — 계약을 reply_failed 로 실패 종결(오픈 목록·기한 스윕·512 계수에서 제거, 이력 잔존 — ADR-0116 결정 2)"
            ),
            ReplyFailOutcome::NoMatch => tracing::debug!(
                in_reply_to,
                replier = %replier_name,
                "reply_to 가 이 회신자의 오픈 계약을 가리키지 않음 — 종결할 계약 없음(정상 경로, spec §3 항목 7-②)"
            ),
            ReplyFailOutcome::AlreadyClosed => tracing::debug!(
                in_reply_to,
                replier = %replier_name,
                "이미 종결된 계약 — 되돌리지 않는다(no-op, spec §3 항목 7-③/7-④)"
            ),
            ReplyFailOutcome::GuardHeld => tracing::debug!(
                in_reply_to,
                replier = %replier_name,
                "잠정·은퇴 예정 표시 계약이라 실패 종결을 적용하지 않음 — 수명은 그 가드 소유(ADR-0118 결정 3, 무동작)"
            ),
        }
    }

    /// ★프로필 삭제 일괄 정리(spec §5 · ADR-0116 결정 3)★ — 호스트(데몬)가 프로필을 지운 **직후** 부른다.
    ///
    /// ★발동 조건 = 삭제 ∧ **그 프로필 id 의 세션이 살아 있지 않음** ∧ **그 이름을 지닌 산 세션도 없음**
    ///   (사용자 결정 2026-07-30 · 리뷰 fix D1 = id 축 · 리뷰 fix N1 = 이름 축 — load-bearing)★: 프로필 삭제
    ///   커맨드는 실행 중 세션을 죽이지 않으므로 "트리 항목만 지웠고 프로세스는 살아 있는" 상태가 존재한다.
    ///   그 상대는 **곧 idle 이 되면 받을 수 있는 산 수신자**라 정리 대상이 아니다 — 정리하면 배달될 메일을
    ///   죽이고 성립할 계약을 실패로 적는다(이 개정 취지의 정반대). 그 세션이 실제로 사라지는 시점엔 프로필도
    ///   없으므로 자연히 "없음" 부류로 수렴한다.
    /// ★게이트는 **두 축**이고 둘 다 "죽었다" 여야 정리가 발동한다(load-bearing)★ — 술어는 양쪽 모두
    ///   **산 명단**(`Running|Exiting`)이다:
    ///   - **① id 축(`is_agent_live(profile_id)`) — 개명 면역**: 이 함수는 프로필이 **지워진 뒤** 불리는데,
    ///     프로필이 없으면 그 산 세션의 canonical 이름이 `display_name` → `basename(session.cwd)` 로
    ///     **바뀐다**. 그래서 삭제 전에 뽑은 이름(= `display_name`)으로만 삭제 후 명단을 찾으면 절대 매치되지
    ///     않고 정리가 **항상** 발동한다 — `RenameProfile` 한 번이면 재현되는 평범한 경로다.
    ///   - **② 이름 축(`live_agents()` 에 그 이름이 있나) — 파괴의 축이 이름이기 때문이다(리뷰 fix N1)**:
    ///     아래가 지우는 것은 id 가 아니라 **이름 전체**(`purge_recipient(name)` ·
    ///     `fail_open_requests_from(name)`)다. 그래서 id 축만 보면 **그 이름을 공유하는 다른 산 세션**의
    ///     파킹 메일이 `RECIPIENT_DELETED`(그 수신자는 삭제되지 않았으니 **거짓 사유**)로 죽고, 그가 요청자인
    ///     오픈 계약이 `reply_failed` 로 닫혀 대기 목록·기한 통지에서 사라진다 — ADR-0118 결정 2·spec §5 가
    ///     금지한 그 결말이고, 유계 잔여로 문서화된 적도 없다. **재현은 지원되는 조작뿐이다**(이름 유일성
    ///     강제는 ADR-0115 소관이라 아직 없다): 프로필 둘을 같은 `display_name` 으로 개명 → 한쪽만 스폰 →
    ///     잠든 쪽을 삭제하면 id 축은 거짓이라 통과한다.
    ///   - **왜 fail-safe(건너뛰기) 방향인가**: 건너뛴 정리의 잔여는 **이미 문서화된 결말**(삭제 시점 단발 +
    ///     TTL 24h `expired` — 아래 항목)이다. 반대 방향(오정리)은 산 에이전트의 배달될 메일·계약을 죽이는
    ///     비가역 손실이다. 두 축의 port 호출이 원자적이지 않은 것도 같은 이유로 무해하다 — 그 사이 명단이
    ///     바뀌면 결과는 "건너뜀"(안전) 쪽으로만 기울고, 정리는 **양쪽 다 죽었을 때만** 나온다.
    /// ★그래도 이름은 필요하다(파킹은 이름 키다)★: 큐·장부·계약의 축은 canonical 이름이므로 정리 대상은
    ///   이름으로 지목한다.
    /// ★하는 일 두 가지★: ① 그 이름 앞 **파킹분 전량**을 `failed` + `RECIPIENT_DELETED` 로 종결(장부 종점 +
    ///   조회 힌트 — 대기열에서만 사라지고 이력은 남아 발신자가 `messages{id}` 로 사유를 본다. "산 메일
    ///   조용히 버리기 금지" 불변식 유지) ② 그 이름이 **요청자**인 오픈 계약을 `reply_failed` 로 종결(회신
    ///   도달 불가 확정). 이미 `replied` 인 계약은 되돌리지 않고, **회신자 쪽이 삭제된 계약은 유지**한다
    ///   (발신자는 기한 통지로 무응답을 알게 되는 기존 경로가 살아 있다).
    /// ★삭제 시점 **단발**이다 — 재평가 트리거를 두지 않는다(정직 명시 · spec §5)★: 발동 조건은 삭제 그
    ///   순간에만 평가된다. 그래서 ⓐ 판정과 파킹 삽입 사이에 삭제가 끼면 그 항목은 이 스캔을 놓치고 ⓑ 프로필만
    ///   지운 산 세션이 나중에 죽어도 거둬갈 사건이 없다 — **그 잔여의 결말은 TTL 24h `expired`** 이고
    ///   `RECIPIENT_DELETED` 가 아니다(알고 수용한 성질). 대칭 트리거·삽입 시점 tombstone 은 **비채택**
    ///   (ADR-0116 거부한 대안) — 되살리려면 새 결정이 먼저다.
    /// ★호출자 의무 둘(ADR-0006)★: ① **프로필 레지스트리 락을 해제한 뒤에** 부른다(레지스트리 락 보유 중
    ///   메시징 락을 잡으면 락 순서가 뒤집힌다 — spec §5) ② `name` 은 **프로필 제거 전에**
    ///   `AgentProfile::canonical_name_when_live()` 로 파생한 값이어야 한다(제거 후엔 파생 재료 자체가 없고,
    ///   그 값이 파킹 키다).
    // ADR-0116 (결정 3) / ADR-0118 (결정 3 — 가드 우선) / 리뷰 fix D1 (id 축) / 리뷰 fix N1 (이름 축)
    pub fn handle_profile_deleted(&self, profile_id: PeerId, name: &str) -> ProfileDeletedOutcome {
        // 1-a) id 축(위 doc ①).
        if self.port.is_agent_live(profile_id) {
            tracing::debug!(
                deleted = %name,
                profile = %profile_id,
                "프로필 삭제 정리 건너뜀 — 그 프로필의 세션이 아직 살아 있다(트리 항목만 지운 상태). 파킹분·계약은 그대로 두고 idle 전이 시 정상 배달된다(spec §5)"
            );
            return ProfileDeletedOutcome {
                skipped_live: true,
                ..Default::default()
            };
        }
        // 1-b) 이름 축(위 doc ② · 리뷰 fix N1). 비교는 로스터 해석과 같은 **정확 일치**다
        //      (`resolve_live`/`push_recipient` 와 같은 축).
        if let Some(alive) = self.port.live_agents().into_iter().find(|a| a.name == name) {
            tracing::debug!(
                deleted = %name,
                profile = %profile_id,
                live_id = %alive.id,
                "프로필 삭제 정리 건너뜀 — 같은 canonical 이름의 **다른 산 세션**이 있다(과도기 동명 — ADR-0115 전). 이름 축으로 지우면 그 산 세션의 메일·계약이 죽는다(리뷰 fix N1). 잔여는 삭제 시점 단발 + TTL 24h 소관(spec §5)"
            );
            return ProfileDeletedOutcome {
                skipped_live: true,
                ..Default::default()
            };
        }

        let now = Instant::now();
        let mut evicted: Vec<EvictedTransition> = Vec::new();
        let mut illegal = 0usize;
        let (purged, contracts) = {
            let mut st = self.state.lock().expect("messaging state poisoned");
            let purged = st.mailbox.purge_recipient(name);
            for m in &purged {
                // ★`pending → failed`(삭제 정리 한정 간선 — spec §6)★: 전용 동사라 범용 전이로는 못 만든다.
                match st.ledger.fail_pending(
                    &m.msg_id,
                    name,
                    FailCode::RecipientDeleted.as_str(),
                    now,
                ) {
                    Ok(()) => {}
                    Err(TransitionError::NotFound) => evicted.push(EvictedTransition {
                        msg_id: m.msg_id.clone(),
                        recipient: name.to_string(),
                        intended: DeliveryStatus::Failed,
                    }),
                    Err(TransitionError::Illegal { .. }) => illegal += 1,
                }
            }
            let contracts = st.ledger.fail_open_requests_from(name);
            (purged, contracts)
        };

        log_evicted_transitions(&evicted);
        if illegal > 0 {
            tracing::error!(
                deleted = %name,
                count = illegal,
                "삭제 정리: 큐에 있던 항목의 장부 상태가 pending 이 아니었다 — 큐↔장부 회계 어긋남(배선 결함 신호)"
            );
        }
        if !purged.is_empty() || !contracts.failed.is_empty() {
            tracing::info!(
                deleted = %name,
                parked_failed = purged.len(),
                contracts_failed = contracts.failed.len(),
                guard_held = contracts.guard_held,
                "프로필 삭제 정리 — 파킹분을 RECIPIENT_DELETED 로 종결하고 그 이름이 요청자인 오픈 계약을 reply_failed 로 종결(ADR-0116 결정 3)"
            );
        }
        if contracts.guard_held > 0 {
            tracing::debug!(
                deleted = %name,
                held = contracts.guard_held,
                "삭제 정리: 잠정·은퇴 예정 표시 계약은 건드리지 않았다(수명 = 가드 소유, ADR-0118 결정 3). 재발화 없음 — 잔여는 TTL 소관(spec §5)"
            );
        }
        ProfileDeletedOutcome {
            skipped_live: false,
            failed_parked: purged.len(),
            failed_contracts: contracts.failed.len(),
            guard_held_contracts: contracts.guard_held,
        }
    }

    /// ★도어벨 발화 시점 분업★ — 배선돼 있으면 **즉시** enqueue(논블록), 미배선이면 `deferred` 에 담아
    ///   호출자가 **응답 확정 후** 인라인 실행하게 한다.
    ///
    /// ★왜 갈라야 하나★: 두 갈래는 비용 구조가 정반대다. 배선 갈래는 채널 send 라 즉시 부르는 게 항상
    ///   옳고(미루면 그 수신자의 깨우기가 남은 blocking write 뒤로 밀리고, 패닉하면 통째로 유실된다),
    ///   미배선 갈래는 **이 스레드에서 배치 write 를 그대로 실행**하므로 주입 루프 한가운데서 부르면
    ///   flush 재진입 + 회계 skew(응답 `pending` vs 장부 `delivered`)를 만든다.
    fn ring_or_defer(&self, id: PeerId, deferred: &mut Vec<PeerId>) {
        match &self.trigger {
            Some(t) => t.request_flush(id),
            None => deferred.push(id),
        }
    }

    /// ★인라인 폴백도 messaging 락 밖에서 불려야 한다★ — 호출부는 모두 park 반환 후(락 해제) 지점이다.
    fn request_flush(&self, id: PeerId) {
        match &self.trigger {
            Some(t) => t.request_flush(id),
            None => self.flush_for_agent(id),
        }
    }

    /// ★`<notice>` 파킹 + ledger `pending` 기록★ — 데몬 통지 전용 래퍼(발송 경로는 `park_into` 를 직접 쓴다).
    ///
    /// ★notice 는 절대 반려되지 않는다(mailbox `park`)★: `ParkKind::Notice` 는 **자기 레인**
    ///   (`mailbox::NOTICE_CAP` = 64)에서 회계되고 넘치면 가장 오래된 통지를 회수할 뿐이다. 그래서 이 함수는
    ///   반환값이 없고 호출부가 결과를 볼 필요도 없다(반려 = 조용한 유실이 될 자리였다).
    #[allow(clippy::too_many_arguments)]
    fn park_notice(&self, msg_id: &str, recipient: &str, body: &str, hinted_id: Option<PeerId>) {
        let now = Instant::now();
        let mut effects = ParkSideEffects::default();
        {
            let mut st = self.state.lock().expect("messaging state poisoned");
            let _ = park_into(
                &mut st,
                ParkRequest {
                    msg_id,
                    sender_name: NOTICE_SENDER_LABEL,
                    from: daemon_identity(),
                    entrance: Entrance::Daemon,
                    recipient,
                    body,
                    hinted_id,
                    kind: ParkKind::Notice,
                    meta: &SendMeta::default(),
                    // notice 는 배달기록이 정확히 1행이다.
                    expected_rows: 1,
                },
                now,
                &mut effects,
            );
        }
        effects.log(recipient);
    }

    /// ★비동기 계기용 얇은 입구(등장/epoch/idle/도어벨 — ADR-0104)★: 데몬측 flush 레인·로스터 diff 가
    ///   부른다. **공용 드레인 루틴을 그대로 부르고 보고서만 버린다** — 그쪽 계기엔 응답할 발신자가 없다.
    pub fn flush_for(&self, recipient: &str, to_id: PeerId) {
        let _ = self.drain_queue(recipient, to_id, None);
    }

    /// ★주입 코드 한 벌 — 이 함수가 그 한 벌이다(불변식 · ADR-0125 §영향 · load-bearing)★: 수신자 이름 앞
    ///   파킹분을 **오래된 순 일괄** 주입한다.
    ///   ★발송 전용 주입 코드를 따로 만들면 이 결정 위반이다★ — 그게 0124/0125가 없앤 "경로 2벌" 결함이고,
    ///   되살아나는 순간 좌석 예약·합류 판정 같은 순서 장치가 다시 필요해진다.
    ///
    /// ★인자 `snapshot`★: `Some` = 호출자가 방금 뜬 로스터를 재사용한다(동기 드레인 — 발송 1회당 스냅샷
    ///   1장 불변식, ADR-0111 결정 2). `None` = 여기서 뜬다(비동기 계기 — stale 가능성 때문에 재해석 필요).
    ///
    /// ★"오래된 순" 은 **이 배치 내부** 보장이다(finding 8)★ — 재파킹은 순번 merge 라 배치 간 이월 시에도
    ///   오래된 것이 우선한다. 한 수신자가 보는 순서를 **적재 순서**로 성립시키는 장치는 아래 0단계다.
    ///
    /// ★in-flight 회계 — cap 이 사이클마다 밀리던 구멍을 막는다(F1 · load-bearing)★: 4단계에서 락을 떠나는
    ///   배치는 큐에 없지만 **여전히 그 수신자 앞 미결 메시지**다. 이걸 분모에서 빼면 다음 인터리빙으로 큐가
    ///   무계로 자란다 — ① drain 이 큐를 비운다 ② 락 밖 inject 동안 동시 발송 k 건이 "빈 큐" 를 보고 cap 검사를
    ///   통과한다 ③ inject 실패로 배치 N 건이 복원된다 → 큐 = N + k, 다음 사이클 배치도 N + k 라 **매 사이클
    ///   +k**. 그래서 나가는 배치를 `take_in_flight` 로 분모에 남기고, 종점(배달/복원)마다 그만큼 정산한다.
    ///   정산 누락은 `FlightSettle`(Drop)이 덮는다. 관측 가능한 대가: **flush 가 도는 동안 그 수신자에게 온
    ///   신규 발송이 `MAILBOX_FULL` 로 반려될 수 있다**(큐는 비어 보여도 분모가 차 있다) — 조용한 성장 대신
    ///   가시적 반려를 택한 spec §5 기조와 같은 선택이다.
    ///
    /// ★★같은 수신자 드레인은 겹쳐 돌면 안 된다 — 제거 금지(round-7 · **ADR-0125의 전제**)★★: 배치 A 가
    ///   락 밖에서 주입 중일 때 배치 B 가 큐를 다시 드레인해 주입하면, A 의 **남은 항목보다 B 가 먼저**
    ///   수신자에게 닿는다(순서 역전). 그래서 0단계에서 in-flight 를 보고 물러난다.
    ///   ★7차엔 이 가드가 **더** 중요하다★: 드레인이 발송 호출 안에서 도니까 **두 발신 스레드가 같은
    ///   수신자를 동시에 드레인하는 것이 상시 경우**가 됐다. 이걸 "in-flight 회계로 대체 가능한 잉여" 로
    ///   읽고 걷어내면 배치가 뒤엉켜 "배달 순서 = 적재 순서" 가 즉시 깨진다.
    ///   ★물러난 쪽의 편지는 유실되지 않는다★ — 이긴 쪽 배치에 실려 나가고, 못 나갔으면 **영수증을 쥔 쪽**이
    ///   정산을 마치며 도어벨을 다시 눌러 준다(그냥 물러나면 lost wakeup — 다음 idle 통지/등장까지 발이
    ///   묶인다. 표식 = `deferred_flush`).
    ///   ★운영 배선에선 이 갈래가 **도달하지 않는다**(방어선 · 증명)★: 도어벨이 배선된 조립은 flush 를 전부
    ///   **단일 직렬 레인**(호스트측 `messaging_host.rs` `run_flush_lane` — FlushMsg 를 하나씩 꺼내
    ///   `spawn_blocking` 완료를 await)에서 실행하므로 두 flush 가 동시에 존재할 수 없다. 겹침이 실재하는 건
    ///   **도어벨 미배선 폴백**(실험 bin· 단위 테스트 — `request_flush` 가 호출 스레드에서 인라인 실행)과
    ///   앞으로 생길 다른 호출자다. 레인 직렬성은 **호스트 쪽 성질**이라(커널은 그 배선을 모른다) 여기서
    ///   가정하지 않고, 이 파일 안에서 닫는다.
    ///
    /// ★왜 to_id 를 인자로 받나(그리고 왜 execution 시점에 재검증하나 — finding 2)★: flush observer/도어벨
    ///   이 로스터 스냅샷에서 (이름→현재 id) 를 알고 부르지만, 그 스냅샷은 **enqueue 시점** 것이라
    ///   execution(여기)까지 사이에 stale 해질 수 있다 — ① 동명 두 번째 에이전트가 등장해 이름이 ambiguous
    ///   해졌거나 ② 그 수신자가 죽었을 수 있다. 그래서 drain 직전 **현재 로스터**로 다시 해석한다(해석
    ///   순서는 아래 타깃 분할 주석). 어느 규칙으로도 안 풀리면 skip(파킹 유지 — 그 이름이 다시 유일해지거나
    ///   TTL 로 만료될 때까지 큐에 남는다).
    ///   인자 `to_id` 는 **호출자가 믿었던 stale 후보**로 로그에만 쓴다(권위는 execution 시점 재해석).
    ///   ★uniqueness 로직은 `unique_reachable_in` 한 곳이다★(여기 · `deliver_notice` 의 이름 도어벨 ·
    ///   삭제 정리의 이름 축) — 이름-키 파킹의 동명 정책을 한 함수에서 판정한다.
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
    ///     유저 에코"(claude 는 이걸 `Structured` 로 낸다 = 턴 진행으로 관측된다)를 즉시 발생시키므로, 항목마다
    ///     게이트를 보면 **배치가 1건 만에 중단**된다(= 드리블 주입 = ADR-0104 거부 대안). 한 타깃의 배치를
    ///     시작했으면 그 타깃 몫은 끝까지 민다.
    ///   - **왜 drain 전이 아니라 drain 후인가**: 항목별 타깃은 id 힌트에 따라 달라져 **drain 하기 전엔 알
    ///     수 없다**. 이름으로만 게이트하면 힌트로 배달되는 경로가 게이트를 우회한다. drain 후 복원은
    ///     같은 락 구간 안이고 무손실·순서 보존이므로(restore_ordered) 외부에 관측 가능한 차이가 없다.
    ///   게이트가 안전한 전제는 관측이 **라이브 출력에서만** 시작한다는 것이다(재개 transcript 는 관측하지
    ///   않는다 — core `OutputCore::seed`). busy = 지금 진행 중인 실제 턴이므로 그 종료 통지가 반드시 온다
    ///   (과거 기록으로 인한 깨울 수 없는 busy 없음). 그 통지가
    ///   유실되는 비정상 턴은 `BUSY_MAX_TURN` 상한 sweep 이 fail-open 으로 깨운다(busy.rs).
    /// ★미배달분은 **큐를 떠나지 않는다**(락 원자성 — load-bearing)★: drain·타깃 분할·게이트·스킵분 복원을
    ///   **한 락 구간**에서 끝내고, 락 밖으로는 **배달할 항목만** 들고 나간다. 예전엔 drain 후 락을 놓고
    ///   게이트를 본 뒤 다시 락을 잡아 복원했는데, 그 사이 큐가 **비어 보이는 창**이 생겨 되돌려질 옛 메일이
    ///   관측에서 사라지고 동시 발송의 판정이 흔들렸다.
    /// ★복원 순서(round-4 finding 1)★: 한 드레인은 같은 이름 큐에 재파킹을 **여러 번** 부를 수 있다 — 락 안
    ///   스킵분 1회 + 락 밖 타깃별 실패분 n회. 그래서 `restore_ordered`(admission 순번 merge)로 되돌린다:
    ///   호출 횟수·순서와 무관하게 큐가 항상 전역 오래된 순을 유지한다. 락 안 스킵분은 인덱스를 정렬해
    ///   순번 오름차순 계약을 지켜 넘긴다(옛 FRONT 삽입은 두 번째 호출이 첫 호출 앞에 꽂혀 나이 순서가
    ///   뒤집혔다 — 그 역전이 sweep 의 만료 항목 은폐로도 번졌다).
    /// ★수용된 잔여(residual)★: 배치 도중 수신자가 **새 턴을 스스로 시작**하면 남은 주입은 CLI 내부 stdin
    ///   큐로 들어간다(유실 없음, "언제 읽히나" 만 흐려짐 — spec §7 미검증 항목).
    // ADR-0125 (주입 코드 한 벌 — 동기·비동기 두 계기가 이 루틴을 공유한다)
    fn drain_queue(
        &self,
        recipient: &str,
        to_id: PeerId,
        snapshot: Option<&[LiveAgent]>,
    ) -> DrainReport {
        // ★로스터 스냅샷은 1회만 뜬다★ — 이름 유일성 판정과 아래 id-힌트 생존 판정이 **같은 스냅샷**을
        //   봐야 배치 안에서 판정이 흔들리지 않는다.
        let owned;
        let roster: &[LiveAgent] = match snapshot {
            Some(r) => r,
            None => {
                owned = self.port.live_agents();
                &owned
            }
        };
        let name_target = unique_reachable_in(roster, recipient);
        let mut report = DrainReport::default();

        let now = Instant::now();
        let mut no_target_kept = 0usize;
        let mut busy_skipped: Vec<(PeerId, u32, usize)> = Vec::new();
        let mut evicted_transitions: Vec<EvictedTransition> = Vec::new();
        // 1~4) 드레인 + 만료 장부화 + 타깃 분할 + 게이트 + 미배달분 즉시 복원.
        //
        // ★락 보유 중 tracing 금지 — 수집 후 락 밖 로깅(finding 3)★: 동기 포맷팅 subscriber 는 stdout 락에
        //   걸릴 수 있어, 크리티컬 섹션 안에서 찍으면 그 지연이 메시징 락 대기로 번진다.
        // ★락 안에서 게이트를 부르는 게 규율 위반이 아닌 이유★: `BusyGate` 는 **순수 조회 + 짧은 락**이며
        //   "messaging 락을 든 채 불려도 안전" 을 계약으로 못 박은 seam 이다(busy.rs `BusyGate` 주석). 역방향
        //   (busy 락 → messaging 락) 경로는 존재하지 않는다(통지는 논블록 채널 send 만 한다) → 락 순서 역전 없음.
        //   금지 대상은 **DeliveryPort(inject/roster)** 다 — 그건 여전히 전부 락 밖이다(아래 5단계).
        // ★labeled block(`'drained`)인 이유★: 만료 전이의 evict 사실은 **락 밖**에서 찍어야 하는데(위 규율),
        //   배달할 게 없다고 여기서 `return` 해 버리면 그 로그가 통째로 사라진다. 그래서 블록을 값으로
        //   빠져나가고(빈 Vec) 로깅은 락 해제 뒤 공통 경로에서 한다(아래 루프들은 빈 입력에 no-op).
        /// 영수증 + 타깃별 배치. `None` = **유예**(0단계에서 물러남 — 큐도 영수증도 건드리지 않았다는 뜻이라
        ///   빈 배치(`Some`)와 구분해야 한다: 빈 배치는 "볼 것이 없었다", 유예는 "지금은 볼 차례가 아니다").
        type Drained = Option<(FlightTicket, Vec<(LiveAgent, Vec<ParkedMessage>)>)>;
        let drained_or_deferred: Drained = 'drained: {
            let mut st = self.state.lock().expect("messaging state poisoned");
            // 0) 같은 수신자 flush 중복 진입 차단(위 doc "겹쳐 돌면 안 되는 이유"). 판정도 기록도 **같은 락
            //    구간**이라, 영수증 보유자의 정산 마무리(같은 락)와 경합하지 않는다: 그쪽이 먼저면 여기서
            //    in-flight 가 0이라 정상 진행하고, 여기가 먼저면 그쪽이 이 표식을 본다.
            //    같은 id 는 두 번 담지 않는다 — 되울림은 idempotent 지만 잉여 도어벨은 공짜가 아니고,
            //    유계 근거도 "서로 다른 id 수" 이기 때문이다(`deferred_flush` 주석).
            if st.mailbox.in_flight_len(recipient) > 0 {
                let waiters = st.deferred_flush.entry(recipient.to_string()).or_default();
                if !waiters.contains(&to_id) {
                    waiters.push(to_id);
                }
                report.retreated = true;
                break 'drained None;
            }
            let drained = st.mailbox.drain(recipient, now);
            for ex in &drained.expired {
                transition_or_collect_evicted(
                    &mut st.ledger,
                    &ex.msg_id,
                    recipient,
                    DeliveryStatus::Expired,
                    now,
                    &mut evicted_transitions,
                );
            }
            let deliverable = drained.deliverable;
            if deliverable.is_empty() {
                break 'drained Some((FlightTicket::default(), Vec::new()));
            }

            // ★항목별 타깃 해석 → 타깃별 분할(round-3 finding 3)★: ① park 시 해석돼 있던 id 힌트가 **아직
            //   로스터에 살아 있으면** 이름 유일성과 무관하게 그쪽으로 배달한다(exact-id 지목이 동명 다수
            //   때문에 TTL 까지 blackhole 되는 걸 막는다 — fix 2) → ② 힌트가 없거나 죽었으면 이름 유일 도달
            //   규칙(재스폰이 파킹을 이어받는 이름-키 설계 — ADR-0101).
            //   ★결박(bound_incarnation) 분기는 제거됐다(ADR-0111 결정 6)★: 파킹분은 **같은 이름의 새
            //   화신(epoch)에게도 배달**된다 — 개인 편지와 동일 규칙이라 여기 특례가 없다. 그 분기가 하던
            //   "확정 사망 회수"(같은 PeerId 의 더 높은 epoch)도 함께 사라졌다: 새 화신이 **받을 자격이 있는**
            //   수신자가 됐으므로 회수할 잔해가 아니다. 되살리면 ADR-0111 위반이다.
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
                    None => {
                        no_target_kept += 1;
                        restore.push(idx);
                    }
                }
            }

            // ★flush 게이트(fix 4 + finding 3) — 타깃당 정확히 1회★. 그 타깃 배치가 시작된 뒤엔 절대 다시
            //   보지 않는다(위 doc: 첫 주입의 유저 에코가 busy 를 만들어 배치를 1건에서 끊는다).
            let mut deliver: Vec<(LiveAgent, Vec<usize>)> = Vec::new();
            for (target, idxs) in groups {
                if self.gate_says_busy(&target) {
                    busy_skipped.push((target.id, target.epoch, idxs.len()));
                    restore.extend(idxs);
                    report.gated = true;
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
            // ★알려진 잔여(리뷰 blind #3 — 이 라운드에서 고치지 않는다)★: 여기서 나간 배치가 락 밖 inject
            //   도중 **패닉**하면 `FlightSettle`(Drop)이 in-flight 회계는 갚지만 **항목 자체는 큐로 돌아오지
            //   않는다**(소유권이 스택에 있었다). v1 이전부터의 조건이고 개편이 만든 게 아니다 — 가드된
            //   경로로 오해하지 말 것(release 는 panic=abort 라 실제 노출면은 debug/테스트 조립뿐).
            let groups: Vec<(LiveAgent, Vec<ParkedMessage>)> = deliver
                .into_iter()
                .map(|(t, idxs)| {
                    let items: Vec<ParkedMessage> =
                        idxs.iter().map(|&i| deliverable[i].clone()).collect();
                    (t, items)
                })
                .collect();
            // ★in-flight 등록 — **락을 떠나는 몫만**(F1 · load-bearing)★: cap 분모의 구멍은 이 락이 풀린
            //   구간에만 있다. 위에서 이미 되돌린 몫(busy·타깃 없음)과 종점을 찍은 몫(만료)은 그
            //   구멍 밖이라 세지 않는다 — 세면 분모가 근거 없이 부풀어 정상 유입이 `MailboxFull` 로 반려된다.
            //   영수증은 아래 `FlightSettle` 이 **반드시** 반납한다(누락 = 그 수신자 레인 영구 봉쇄).
            let flight = st
                .mailbox
                .take_in_flight(recipient, groups.iter().flat_map(|(_, items)| items.iter()));
            Some((flight, groups))
        };
        // 유예(0단계) — 큐도 영수증도 건드리지 않았으므로 로깅할 사실도, 정산할 것도, 되울릴 것도 없다
        //   (되울리기는 영수증 보유자의 몫 — 아래 꼬리). 로그는 debug 한 줄만.
        let Some((flight, groups)) = drained_or_deferred else {
            tracing::debug!(
                recipient,
                stale_id = %to_id,
                "드레인 유예: 같은 수신자 앞 배치가 아직 주입 중(in-flight) — 드레인 없이 물러남, 정산 시 재-도어벨(round-7 · ADR-0125)"
            );
            return report;
        };
        // ★단일 출구 정산 가드(F1)★ — 아래 배달 루프는 타깃별 early break 가 있고, 락 밖 외부 호출(inject)이
        //   섞여 있다. 정산을 각 갈래에 흩뿌리면 한 곳만 놓쳐도 그 수신자 레인의 분모가 영구히 부풀어
        //   메일이 영영 안 들어간다. 그래서 남은 영수증은 Drop 이 반납한다(정상 종료·언와인딩 공통).
        let mut flight = FlightSettle {
            svc: self,
            recipient,
            owed: flight,
        };

        log_evicted_transitions(&evicted_transitions);
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

        // 5) 타깃별 배달(★락 밖★ — inject 는 자식 stdin blocking write).
        for (target, items) in &groups {
            for (n, parked) in items.iter().enumerate() {
                let to_id = target.id;
                let payload = ParkPayload::decode(&parked.envelope);
                let wrapped = match parked.kind {
                    ParkKind::Notice => wrap_notice(&payload.body),
                    ParkKind::Message => self.wrap_now(
                        &payload.sender_name,
                        &parked.msg_id,
                        &payload.body,
                        &payload.meta.envelope_fields(&parked.msg_id),
                    ),
                };
                // ★H3: 계약을 **주입 전에** 착지 incarnation 으로 옮긴다(load-bearing)★
                //
                // ★막는 것 = 회신이 계약을 못 찾는 창★: 이름 큐 파킹은 재스폰 이어받기가 기능이라(ADR-0101)
                //   발송 시점과 **다른** incarnation 에게 배달될 수 있다(A 에게 건 request 가 A 사망 후 동명
                //   B 에게 flush). 옛 배선은 재바인딩을 주입 **뒤**에 했는데, 회신 매칭이 두 패스(id 우선 —
                //   A6)로 엄격해진 뒤로는 그 순서가 곧 결함이다: B 의 회신이 재바인딩보다 먼저 도착하면
                //   계약은 아직 A 의 id 를 들고 있어 `NoMatch` 로 빗나가고(회신이 계약에서 유실), 나중에
                //   거짓 기한 통지가 나간다. 주입은 그 자체로 수신자 턴을 깨우므로 이 창은 좁지 않다.
                // ★주입 실패 시의 잔여(수용 — 무해)★: 계약이 "시도했던 incarnation" 을 가리킨 채 남는다.
                //   그 항목은 재파킹돼 다음 flush 가 다시 해석하고 그때 또 재바인딩하므로 자기교정된다.
                {
                    let mut st = self.state.lock().expect("messaging state poisoned");
                    st.ledger
                        .rebind_request_recipient(&parked.msg_id, recipient, to_id);
                }
                match self.port.inject(to_id, wrapped.as_bytes()) {
                    Ok(outcome) => {
                        // 이 1건은 종점을 맞았다 — in-flight 영수증에서 즉시 떼어 낸다(F1). 배치 끝까지
                        //   붙들면 긴 배치가 도는 동안 그 수신자 앞 정상 유입이 통째로 반려된다.
                        let settled = flight.owed.split([parked.kind]);
                        {
                            let mut st = self.state.lock().expect("messaging state poisoned");
                            let _ = st.ledger.transition(
                                &parked.msg_id,
                                recipient,
                                DeliveryStatus::Delivered,
                                Instant::now(),
                            );
                            st.mailbox.settle_in_flight(recipient, settled);
                        }
                        // ★to_name = park 키(recipient)★: 하네스가 "어느 이름 앞 파킹이 배달됐나" 로 회수하므로
                        //   해석된 타깃의 로스터 이름이 아니라 파킹 키를 싣는다(둘은 정상 경로에서 동일하다).
                        let observed_target = LiveAgent {
                            id: to_id,
                            name: recipient.to_string(),
                            epoch: outcome.epoch,
                            // 관측 레코드는 턴 신호 축을 쓰지 않는다(배달 사실만 싣는다) — 값은 무의미하다.
                            turn_signal: false,
                        };
                        self.observe_success(
                            &parked.msg_id,
                            &observed_target,
                            payload.from,
                            payload.entrance,
                            &wrapped,
                            &outcome,
                            payload.meta.reply_to.clone(),
                        );
                        report.injected.push(parked.msg_id.clone());
                    }
                    Err(e) => {
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
                        //   ★그룹 간 나이 순서도 무의미하지 않다★: 큐 앞머리가 다음 배치의 배달 순서를
                        //   결정하고(뒤이은 발송의 동기 드레인도 거기서 뺀다 — ADR-0125), 나이 역전은 sweep 이
                        //   만료 항목을 지나치게 만든다.
                        // ★배치를 여기서 끊는 게 옳다★: 이 실패는 그 수신자가 재시작했거나 도달 불가해진
                        //   것이므로 (a) 실패분은 되돌려 다음 flush 에 맡기고 (b) 남은 항목도 되돌려
                        //   **새 incarnation 기준으로 게이트를 다시 받게** 해야 한다(그냥 밀면 새 턴 한가운데에
                        //   주입될 수 있다). 그래서 여기서 배치를 끊는 게 옳고, 사유는 `e` 가 남긴다.
                        let remaining: Vec<ParkedMessage> = items[n..].to_vec();
                        let remaining_count = remaining.len();
                        // ★복원과 정산은 **한 락 구간에서 짝**이어야 한다(F1 불변식의 근거)★: 복원은 이
                        //   항목들을 in-flight 에서 큐로 **옮기는** 것이라, 둘 사이에 락을 놓으면 그 찰나에
                        //   같은 항목이 큐와 in-flight 양쪽에 잡혀(이중 계수) 분모가 부풀고, 반대로 정산만
                        //   먼저 하면 창이 다시 열린다. 합(`queue + in_flight`)은 이 짝 덕분에 불변이다.
                        let settled = flight.owed.split(remaining.iter().map(|m| m.kind));
                        {
                            let mut st = self.state.lock().expect("messaging state poisoned");
                            st.mailbox.restore_ordered(recipient, remaining);
                            st.mailbox.settle_in_flight(recipient, settled);
                        }
                        // ★이 지점이 발송의 실패를 관측 레코드로 남기는 유일한 자리다(ADR-0088)★.
                        self.observe_failure(
                            &parked.msg_id,
                            target,
                            payload.from,
                            payload.entrance,
                            &wrapped,
                            &e,
                            payload.meta.reply_to.clone(),
                        );
                        tracing::warn!(
                            recipient,
                            agent = %to_id,
                            remaining = remaining_count,
                            "메시지 드레인 중 inject 실패/거부 — 그 타깃의 남은 배치 재파킹(무손실 restore_ordered, ADR-0103/0104): {e}"
                        );
                        report.inject_error = Some(e);
                        break;
                    }
                }
            }
        }

        // 6) 유예된 드레인 되울리기 — 0단계에서 물러난 호출자의 깨우기(`deferred_flush` 주석).
        // ★먼저 영수증을 비운다★: 되울린 flush 가 (인라인 폴백에선 **이 스레드에서 곧바로**) 0단계를 다시
        //    보므로, 우리 영수증이 남아 있으면 그 자리에서 또 유예돼 깨우기가 표식으로만 왕복한다.
        //    Drop 에 맡기지 않고 명시적으로 떨구는 이유가 이 순서다(정상 경로에선 이미 0건이라 no-op).
        drop(flight);
        // ★집합을 **락 안에서 통째로 꺼낸다**(round-8 · load-bearing)★: 아래 되울림은 인라인 폴백에서 이
        //   스레드의 flush 재진입을 부르고, 그 flush 가 또 유예되면 같은 키에 id 를 **새로 넣는다**. 맵을 들고
        //   순회하면 그 추가분과 섞여 무엇을 눌렀는지 흐려지고 재진입 중 재대여 위험도 생긴다. 통째로 꺼내
        //   소유하면 우리는 "이 스냅샷" 만 책임지고, 순회 중 새로 쌓인 유예는 **그때 영수증을 쥔 쪽**(재진입한
        //   flush)의 6단계가 가져간다 — 책임이 겹치지도 새지도 않는다.
        let deferred = {
            let mut st = self.state.lock().expect("messaging state poisoned");
            st.deferred_flush.remove(recipient).unwrap_or_default()
        };
        // ★도어벨 id 는 **유예한 쪽이 쓰려던 값**을 그대로 쓴다★: 그쪽이 열려던 큐를 다시 열게 하는 게
        //   목적이라 우리 to_id 로 바꾸지 않는다(`flush_for_agent` 가 그 id 의 현재 이름 큐 + 힌트 큐를 연다).
        // ★전부 누른다 — 고르지 않는다(round-8 high)★: 어느 id 가 "쓸모 있는" 깨우기인지는 여기서 알 수
        //   없다(로스터를 다시 떠도 힌트 큐 쪽은 못 가린다 — `deferred_flush` 주석). 죽은 id 로 눌러도
        //   `flush_for_agent` 가 조용한 no-op 이라 잉여의 대가는 0에 수렴하는 반면, 하나라도 빠뜨리면
        //   산 수신자 앞 메일이 TTL 까지 묶인다 — 비대칭이라 전부 누르는 쪽이 맞다.
        for id in deferred {
            self.request_flush(id);
        }
        report
    }

    /// ★턴 종료(idle 전이) flush(C2 · ADR-0104 결정 3)★: **id 로** 지목된 flush — 그 에이전트의 canonical
    ///   이름을 풀어 `flush_for` 에 위임한다. 왜 id 입구가 따로 있나: 턴 종료 통지는 출력 경계에서 나와
    ///   **id/epoch 만 안다**(이름을 모른다 — 이름은 프로필·cwd 파생이라 core 출력 경계에 없다).
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
    pub fn flush_for_agent(&self, to_id: PeerId) {
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

    /// ★TTL sweep + 회신 기한 초과 통지(spec §5·§3 — C1/C3)★: 주기 task(lib.rs)가 부르는 단일 유지보수 진입점.
    ///   ① 전 수신자 만료 파킹분을 걷어 ledger `pending→expired` 로 남기고(장부 잔존)
    ///   ② 기한 초과된 미회신 request 를 걷어 **발신자에게** `<notice>` 를 배달한다(spec §3 단계 4).
    ///
    /// ★notice 는 수신자 재촉이 아니다★: 통지는 요청을 **건 쪽**에게 간다 — 재촉할지 포기할지는 발신 LLM 의
    ///   판단이라(ADR-0103 결정 3) 데몬은 사실만 알린다.
    /// ★정확히 1회(spec §7)★: `due_timeouts` 가 반환하며 `notified` 를 세우므로 같은 request 는 다음 sweep 에
    ///   다시 나오지 않는다 — 이중 통지 방지 책임은 장부에 있고 여기선 걷은 것만 배달한다.
    /// ★논블록(load-bearing — sweep task 보호)★: 이 함수는 **주입하지 않는다**. notice 는 파킹 + 도어벨까지만
    ///   하고 실제 배치 write 는 flush 레인이 한다(근거 = `deliver_notice` 주석).
    /// ★로스터는 **틱당 한 번**만 뜬다(리뷰 fix 8)★: 예전엔 due 항목마다 `live_agents`(= 전 세션
    ///   순회)를 다시 불러, 한 틱 안에서 항목별로 **다른 스냅샷**을 보고 판정했다(같은 틱인데 앞 항목은
    ///   배달, 뒤 항목은 부재로 갈릴 수 있었다). 한 번 떠서 전 항목에 같은 스냅샷을 쓴다 — 비용도 O(due)
    ///   에서 O(1) 로 준다. due 가 비면 아예 뜨지 않는다(대부분의 틱).
    /// ★만료 전이의 `NotFound` 는 조용히 삼키지 않는다(C4 리뷰 fix J)★: 방송은 한 발송이 레코드를 **N개**
    ///   쓰므로 이력 링(`ledger::HISTORY_CAPACITY`)이 훨씬 빨리 회전한다 — 파킹이 만료될 즈음 그 레코드가
    ///   이미 밀려났을 수 있고, 그러면 "만료됐다" 는 사실이 **어디에도** 남지 않는다. 전이는 여전히
    ///   best-effort(장부를 되살리진 않는다)지만, 그 사실을 debug 로 남겨 관측 가능하게 한다(락 밖 로깅).
    pub fn sweep(&self, now: Instant) {
        // ★F1 보증 계층 — **버려진** 계약 예약 회수★: RAII Drop 은 `try_lock` 이 실패하면 아무 것도 못
        //   하므로(`Reservation` 헤더), 락을 정상적으로 소유하는 이 지점이 같은 일을 다시 한다.
        // ★7차부터 이 회수는 정상 경로에서 **도달하지 않는다**(ADR-0125 — 전제 보정)★: 예약 창이 적재 락 안에
        //   전부 들어와, 소유자가 락을 놓은 채 예약을 든 구간이 없다. 그래서 여기 걸릴 잔해는 **잊은 갈래·
        //   언와인딩** 부류뿐이고 이 지점은 그 방어선이다. "돌아도 아무것도 안 나오니 지우자" 로 읽지 말 것 —
        //   제거는 ADR-0108 재론이고, 그 순간 `Reservation` 의 Drop 실패 갈래에 보증이 사라진다.
        let reclaimed = {
            let mut st = self.state.lock().expect("messaging state poisoned");
            st.ledger.reclaim_abandoned_reservations()
        };
        for (msg_id, recipient) in &reclaimed {
            tracing::error!(
                msg_id = %msg_id,
                recipient = %recipient,
                "정산되지 않은 계약 예약을 sweep 이 회수 — 발송 경로에 커밋/롤백 누락이 있거나 그 스레드가 패닉했다(F1 보증 계층)"
            );
        }

        let mut evicted_transitions: Vec<EvictedTransition> = Vec::new();
        let due: Vec<DueTimeout> = {
            let mut st = self.state.lock().expect("messaging state poisoned");
            let expired = st.mailbox.sweep_expired(now);
            for ex in expired {
                // ★(msg_id, recipient) 로 정확히 지목(C4 — ADR-0104 앵커 해소)★: 장부는 1 msg_id : N 배달
                //   기록이라(그룹 방송) msg_id 만으로는 어느 멤버의 배달인지 특정할 수 없다. mailbox 가
                //   만료 항목마다 **자기 큐 키**를 함께 돌려주므로(`ExpiredParked`) 그 쌍으로 전이한다 —
                //   옛 "첫 pending 레코드 역조회" 헬퍼는 엉뚱한 멤버를 만료시킬 수 있어 제거됐다.
                transition_or_collect_evicted(
                    &mut st.ledger,
                    &ex.msg.msg_id,
                    &ex.recipient,
                    DeliveryStatus::Expired,
                    now,
                    &mut evicted_transitions,
                );
            }
            st.ledger.due_timeouts(now)
        };
        // due 유무와 무관하게 먼저 찍는다.
        log_evicted_transitions(&evicted_transitions);
        if due.is_empty() {
            return;
        }
        let roster = self.port.live_agents();
        for d in due {
            // spec §1 notice 템플릿(한국어 계약 — 문구를 바꾸면 프라이밍·수용 기준과 어긋난다).
            //   기한 표기는 **발신자가 쓴 원본**을 그대로 쓴다(fix 6 — 봉투 `reply-by` 와 어긋나지 않게).
            // ★`[engram]` 접두 = 발신 주체 표시(사용자 요청 2026-07-26)★: `<notice>` 에 `from` 이 없다는
            //   구조적 신호만으로는 수신 LLM 이 "누가 보냈나" 를 놓칠 수 있다(프라이밍을 못 읽었거나 봉투가
            //   요약돼 전달되는 경우). 그래서 본문 첫 토큰으로 시스템 출처를 명시한다 — 프라이밍이 없어도
            //   읽히도록. ★파싱 계약이 아니다★: 사람/LLM 가독용 라벨이고, 기계 판정은 여전히 태그 모양
            //   (`<notice>` · `from` 부재)이 정본이다(코드가 이 접두를 되읽지 않는다).
            let body = format!(
                "[engram] 요청 {} 기한({}) 초과 — {} 회신 없음",
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
    /// ★FIFO 정합★: 앞선 파킹이 있으면 notice 도 그 뒤에 붙는다 — 통지가 앞선 메일을 앞지르지 않는다.
    /// ★id 힌트 = **요청 발신자의 PeerId**(리뷰 fix 2 · load-bearing)★: 파킹 키는 발송 시점의 발신자
    ///   **이름**인데, 그 사이 그 에이전트가 개명했으면 이름-키 큐를 아무도 열지 않는다 — 통지는
    ///   `notified` 라 재발화도 없으니 그 notice 는 **영영 stranded**(계약이 조용히 반쪽) 된다. 그래서
    ///   장부가 함께 들고 있던 발신자 id 를 힌트로 실어, flush 가 이름 유일성과 무관하게 그 incarnation 으로
    ///   배달하게 한다(id 가 죽었으면 이름 규칙으로 자동 복귀 — respawn 이어받기 유지).
    /// ★도어벨은 **파킹 뒤 반드시** 누른다(같은 fix)★: 예전엔 이름이 유일 도달로 풀릴 때만 눌러서, 개명·
    ///   동명 다수 상황에서 큐에 넣고 아무도 깨우지 않는 lost wakeup 이 났다. 이제 발신자 id 로 한 번
    ///   (`flush_for_agent` 가 힌트 큐까지 연다), 이름이 다른 산 에이전트로 풀리면 그쪽으로도 한 번 누른다.
    // ADR-0103
    fn deliver_notice(&self, due: &DueTimeout, body: &str, roster: &[LiveAgent]) {
        // ★타임아웃↔회신 레이스 좁히기(리뷰 fix 5)★: due 산출(락 해제)과 여기 사이에 회신이 도착해 계약이
        //   닫혔을 수 있다 — 그러면 발신자는 회신을 받고도 "회신 없음" 통지를 뒤이어 받는다(모순된 통지).
        //   파킹 **직전**에 장부를 다시 보고 닫혔으면 통지를 접는다. 남는 잔여: 이 확인과 park 가 별개 락
        //   구간이라 그 사이(마이크로초)에 닫히는 경우는 여전히 통지가 나간다 — 한 락으로 묶으려면 park 를
        //   장부 안으로 끌어와야 해(관심사 혼합) 여기선 창을 좁히는 선에서 멈춘다.
        let closed_since_collection = {
            let st = self.state.lock().expect("messaging state poisoned");
            st.ledger.is_request_closed(&due.request_id, &due.recipient)
        };
        if closed_since_collection {
            tracing::debug!(
                request_id = %due.request_id,
                "기한 초과 통지 취소 — 산출 후 파킹 전에 회신이 도착해 계약이 닫힘(spec §3)"
            );
            return;
        }

        let recipient = &due.sender;
        let drawn = self.draw_daemon_msg_id();
        if let Some(collided) = &drawn.collided {
            tracing::error!(
                collided = %collided,
                replacement = %drawn.id,
                still_colliding = drawn.still_colliding,
                "notice id 충돌 — 새 id 로 1회 재시도(ADR-0103 · 사실상 불가한 경로라 난수/장부 배선을 의심할 것)"
            );
        }
        let notice_id = drawn.id;
        let by_name = unique_reachable_in(roster, recipient);
        self.park_notice(&notice_id, recipient, body, Some(due.sender_id));
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
    /// ★알려진 잔여(리뷰 blind #5 — 이 라운드에서 고치지 않는다)★: 이 검사와 실제 기록 사이에 **예약 창이
    ///   없다** — 두 발송이 같은 fresh id 를 동시에 뽑으면 둘 다 통과할 수 있다(v1 이전부터의 조건, 개편이
    ///   만든 게 아니다). 확률·피해·비용 판단은 `handle_send` 의 예약 주석 참조. 가드된 경로로 오해하지 말 것.
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

    /// 봉투를 **현재** 전역 포맷으로 감싼다(단일 wrap point — ADR-0096).
    fn wrap_now(&self, sender: &str, msg_id: &str, body: &str, fields: &EnvelopeFields) -> String {
        let format: EnvelopeFormat = self.registry.envelope_format();
        wrap_message(sender, msg_id, body, format, fields)
    }

    /// ★`in_reply_to` = 호출자가 넘긴 구조화 값(F1 리뷰 fix, load-bearing — 보안)★: 옛 구현은 렌더된 봉투
    ///   문자열(`wrapped`)을 `in-reply-to="…"` 로 substring 탐색해 파생했는데, 본문 이스케이프
    ///   (`escape_xml_text`)가 따옴표를 이스케이프하지 않아 발신자가 본문에 그 속성 문자열을 흉내 내
    ///   넣으면 관측이 위조됐다(재현됨 — envelope.rs `DeliveryObservation.in_reply_to` 주석). 그래서 이제
    ///   `wrapped` 는 파싱하지 않고, 호출부가 봉투를 조립할 때 이미 쓴 `SendMeta.reply_to`(ingress
    ///   `validate_contract` 가 검증한 값)를 파라미터로 그대로 받는다 — 텍스트 재해석 없음.
    fn observe_success(
        &self,
        msg_id: &str,
        target: &LiveAgent,
        from: SenderIdentity,
        entrance: Entrance,
        wrapped: &str,
        outcome: &InjectReceipt,
        in_reply_to: Option<String>,
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
            in_reply_to,
            error: None,
        });
        // 보안: body/토큰 미로깅 — 바이트 수·id 만.
        tracing::info!(
            from = %from.peer_id,
            to = %target.id,
            to_name = %target.name,
            msg_id = %msg_id,
            entrance = ?entrance,
            bytes = wrapped.len(),
            "메시징 배달(delivered, ADR-0103/0104)"
        );
    }

    /// 실패를 성공으로 삼키지 않음의 증거(ADR-0088).
    ///
    /// ★`in_reply_to` = 구조화 값(F1 — `observe_success` 주석과 같은 이유)★: 파라미터로 받은 값을 그대로
    ///   싣는다(봉투 재파싱 없음). write 가 실패해 실제로 도달하지 않았어도 "무엇에 대한 회신이었나"는
    ///   발신 인자에서 이미 정해진 사실이라 그대로 기록한다 — 단 이 레코드는 `is_delivered()==false`
    ///   이므로 완결성 판정 소비자는 이 값을 "회신이 갔다"의 증거로 쓰면 안 된다(호출자 규율).
    fn observe_failure(
        &self,
        msg_id: &str,
        target: &LiveAgent,
        from: SenderIdentity,
        entrance: Entrance,
        wrapped: &str,
        err: &str,
        in_reply_to: Option<String>,
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
            in_reply_to,
            error: Some(err.to_string()),
        });
        tracing::warn!(
            to = %target.id,
            to_name = %target.name,
            msg_id = %msg_id,
            "메시징 주입 실패 — 파킹으로 전환(무손실): {err}"
        );
    }

    #[cfg(any(test, feature = "test-harness"))]
    pub fn occupied_slots_for_test(&self) -> usize {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger.occupied_slots()
    }

    #[cfg(any(test, feature = "test-harness"))]
    pub fn close_contract_for_test(&self, msg_id: &str, recipient: &str) {
        let mut st = self.state.lock().expect("messaging state poisoned");
        st.ledger.close_for_test(msg_id, recipient, Instant::now());
    }

    #[cfg(any(test, feature = "test-harness"))]
    pub fn marked_retirements_for_test(&self) -> usize {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger.marked_retirement_count_for_test()
    }

    /// 추적 항목 총수 — **닫힘 포함**(`open_request_count` 와 다른 축).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn tracking_len_for_test(&self) -> usize {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger.tracking_len()
    }

    /// ★계약 축 종점 어휘★: `awaiting_reply` | `replied` | `reply_failed`(추적에 없으면
    ///   `None`). `replied` 와 `reply_failed` 는 오픈 목록·기한 스윕·상한 계수에서 **똑같이 빠지므로** 그
    ///   축으로는 구분되지 않는다 — "수용은 완료로, 도달 불가 확정만 실패로"(spec §3 항목 7-④)와 "삭제 정리는
    ///   `replied` 를 되돌리지 않는다" 를 단언하려면 사유 자체를 봐야 한다.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn contract_outcome_for_test(&self, msg_id: &str, recipient: &str) -> Option<&'static str> {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger.contract_outcome_for_test(msg_id, recipient)
    }

    /// 그 계약에 박힌 수신자 id — `None` = 이름 귀속 구간 = 잠듦 파킹(리뷰 fix D4).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn contract_recipient_id_for_test(&self, msg_id: &str, recipient: &str) -> Option<PeerId> {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger.contract_recipient_id_for_test(msg_id, recipient)
    }

    /// 그 계약 키가 **추적 목록에 아직 있나**(닫힘·은퇴 표시 무관). 은퇴는 **표시**일
    ///   뿐이고 물리 제거는 커밋에서 일어나므로, 이 값이 잠정 창을 직접 관측하는 축이다(이력 링과 무관하다 —
    ///   `msg_id_in_use` 는 4096 링 때문에 창이 닫힌 뒤에도 true 라 그 창을 관측하지 못한다).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn contract_tracked_for_test(&self, msg_id: &str, recipient: &str) -> bool {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger.is_tracked_for_test(msg_id, recipient)
    }

    /// 발급 측 충돌 검사가 이 id 를 "사용 중" 으로 보나 — 추적·이력·잠정 예약 **합산**.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn msg_id_in_use_for_test(&self, msg_id: &str) -> bool {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger.msg_id_in_use(msg_id)
    }

    /// 특정 msg_id 의 ledger 상태 목록 — **오래된 순**.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn ledger_statuses(&self, msg_id: &str) -> Vec<DeliveryStatus> {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger
            .records_for(msg_id)
            .iter()
            .map(|r| r.status)
            .collect()
    }

    /// 특정 msg_id 의 배달기록 **종점 키**(수신자 키) 목록, 오래된 순.
    ///
    /// ★왜 상태가 아니라 키를 보나★: 한 발송의 행마다 장부 종점이 **정확히 하나**여야 한다(spec §8). 상태
    ///   목록만 보면 "행 수 == 레코드 수" 는 확인되지만 **두 행이 같은 키를 발급받아 서로를 덮는** 충돌은
    ///   보이지 않는다(F2 의 (b) 갈래). 그래서 키 자체를 노출한다.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn ledger_endpoint_keys(&self, msg_id: &str) -> Vec<String> {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger
            .records_for(msg_id)
            .iter()
            .map(|r| r.to.clone())
            .collect()
    }

    #[cfg(any(test, feature = "test-harness"))]
    pub fn open_request_count(&self) -> usize {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger.open_request_count()
    }

    /// 장부 이력 스냅샷 `(msg_id, from, to, status)` — 오래된 순.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn ledger_snapshot(&self) -> Vec<(String, String, String, DeliveryStatus)> {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger
            .all_records()
            .iter()
            .map(|r| (r.msg_id.clone(), r.from.clone(), r.to.clone(), r.status))
            .collect()
    }

    /// `Mailbox::can_admit` 의 예측값. 봉투 `to` 동결이 이 예측에 기대므로(park 전에
    ///   cap 결과를 알아야 한다) 예측과 실제 admission 이 갈리지 않는지 테스트가 직접 못 박는다.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn can_admit_for_test(&self, recipient: &str) -> bool {
        let st = self.state.lock().expect("messaging state poisoned");
        st.mailbox.can_admit(recipient, ParkKind::Message)
    }

    #[cfg(any(test, feature = "test-harness"))]
    pub fn parked_len(&self, recipient: &str) -> usize {
        let st = self.state.lock().expect("messaging state poisoned");
        st.mailbox.len(recipient)
    }

    /// 수신자 큐의 **현재 순서**(msg_id, 앞→뒤).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn parked_msg_ids(&self, recipient: &str) -> Vec<String> {
        let st = self.state.lock().expect("messaging state poisoned");
        st.mailbox.msg_ids(recipient)
    }

    /// 그 이름 앞으로 **아직 정산되지 않은 in-flight** 건수 — flush 유예 판정이 보는 값과 같은 출처.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn in_flight_len(&self, recipient: &str) -> usize {
        let st = self.state.lock().expect("messaging state poisoned");
        st.mailbox.in_flight_len(recipient)
    }

    /// 파킹 항목 하나의 payload 를 **의도적으로 손상**시킨다 — 깨진 항목이 flush 배치를 중단시키지 않고 그
    ///   항목만 폴백 봉투로 열화되는지 실제 경로로 단언하는 seam(C3 리뷰 fix 4).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn corrupt_parked_payload_for_test(&self, recipient: &str, idx: usize) {
        let mut st = self.state.lock().expect("messaging state poisoned");
        st.mailbox
            .corrupt_envelope_for_test(recipient, idx, "CORRUPT-PAYLOAD".to_string());
    }

    // ── 장부 조회 표면(D · spec §6 `messages { id? }`) ──────────────────────────────────────

    /// ★`messages { id }` — 그 메시지의 배달 장부(spec §6)★. 없으면 `None`(입구가 반려로 번역).
    ///
    /// ★경과 초(`age_secs`)로 시각을 노출하는 이유★: 장부의 시각은 `Instant`(단조 시계)라 벽시계 값이
    ///   아니다 — 절대 시각으로 바꾸려면 장부에 `SystemTime` 축을 새로 들여야 하고, 그건 v1 인메모리 범위
    ///   밖이다(spec §5 "상태 전이 시각" 은 상대 비교용 데이터). 그래서 조회 순간(`now`) 기준 **경과 초**로
    ///   환산해 내보낸다 — 수신 LLM 에게도 "3분 전" 이 "17:42:03" 보다 바로 쓸모 있다.
    /// ★`now` 를 인자로 받는다★: 장부 순수성(주입 시계, ledger.rs 헤더)과 같은 규율 — 결정적 단위 테스트.
    /// ★이력이 통째로 밀려난 **열린 계약**은 계약 뷰로 답한다(리뷰 NOTE — 교차 동사 모순 해소)★: B3 이후
    ///   미회신 계약은 이력보다 오래 산다. 그러면 `messages{}` 는 그 id 를 미결로 보여 주는데 `messages{id}`
    ///   는 `MESSAGE_NOT_FOUND` 를 내는 자기모순이 생긴다 — 조회자가 "목록이 거짓말한다" 로 읽는다. 그래서
    ///   행이 없어도 계약이 살아 있으면 **행 0줄 + `awaiting_reply=true` + 잘림 표시**로 답한다.
    pub fn message_state(&self, msg_id: &str, now: Instant) -> Option<MessageStateView> {
        let st = self.state.lock().expect("messaging state poisoned");
        let (records, may_be_truncated) = st.ledger.records_for_detailed(msg_id);
        let open = st
            .ledger
            .open_requests()
            .into_iter()
            .find(|r| r.request_id == msg_id);
        if records.is_empty() {
            let open = open?;
            return Some(MessageStateView {
                id: msg_id.to_string(),
                from: open.sender,
                awaiting_reply: true,
                // 행이 하나도 안 남았다 = 이력이 통째로 밀려났다는 뜻이므로 잘림이 **확정**이다.
                may_be_truncated: true,
                rows: Vec::new(),
            });
        }
        // 발신자는 모든 레코드가 공유한다(같은 논리 메시지) — 첫 줄에서 뽑아 상단에 한 번만 싣는다.
        let from = records[0].from.clone();
        let rows = records
            .iter()
            .map(|r| DeliveryRowView {
                to: r.to.clone(),
                status: status_label(r.status),
                age_secs: secs_since(r.created_at, now),
                updated_secs_ago: secs_since(r.transitioned_at, now),
                // 힌트 문구는 `row_hint` 단일점 — 장부는 코드만 보관한다.
                code: r.fail_code,
                hint: r.fail_code.and_then(row_hint),
            })
            .collect();
        Some(MessageStateView {
            id: msg_id.to_string(),
            from,
            awaiting_reply: open.is_some(),
            may_be_truncated,
            rows,
        })
    }

    /// ★`messages` 무인자 — 호출자의 "미결"(spec §6)★. 세 갈래를 **한 목록**으로 합쳐 오래된 순으로 준다:
    ///   ① 내가 보냈는데 아직 안 꽂힌 것(`from=me` + `pending`) ② 내가 건 request 의 회신 대기
    ///   (`from=me` + awaiting_reply) ③ 내가 받은 request 중 아직 답 안 한 것(`to=me` + awaiting_reply).
    ///
    /// ★호출자 = (이름, PeerId) 둘 다 받는다(D 리뷰 B1)★. 축마다 매칭 규칙이 다르다:
    ///   - **회신 계약(②③)** = `matches_contract_party` — 계약이 id 를 들고 있으면 **id 로**, 없으면 이름으로.
    ///     동명 다수에서 exact-id 로 지목한 request 의 의무가 쌍둥이에게 잘못 붙는 걸 막는다.
    ///   - **미배달 통보(①)** = 이름 매칭(아래 주석의 문서화된 잔여) — 이력 레코드엔 id 축이 아예 없다.
    /// ★정렬 = 오래된 순★: 오래 묵은 것이 먼저 처리돼야 할 일이다(메일박스 flush 규칙과 같은 방향).
    // 리뷰 B1
    pub fn open_items_for(&self, me: &str, me_id: PeerId, now: Instant) -> Vec<OpenItemView> {
        let st = self.state.lock().expect("messaging state poisoned");
        let mut items: Vec<OpenItemView> = Vec::new();
        // ① 내가 보낸 미배달분.
        // ★잔여(문서화된 한계 — 리뷰 B1 의 범위 밖)★: 이력 레코드(`MessageRecord`)는 이름만 담고 PeerId
        //   축이 없다. 그래서 같은 이름의 발신자가 둘이면 서로의 미배달 통보가 이 목록에 섞여 보인다.
        //   계약(②③)과 달리 여기엔 붙들 id 가 애초에 없어(장부 스키마 변경이 필요) 이번 수정 범위에서
        //   제외했다 — 오귀속의 피해도 다르다: 통보는 "누가 답할 의무를 지나" 가 아니라 관측 정보다.
        for r in st.ledger.all_records() {
            if r.from == me && r.status == DeliveryStatus::Pending {
                items.push(OpenItemView {
                    direction: Direction::OutboundPending,
                    id: r.msg_id.clone(),
                    from: r.from.clone(),
                    to: r.to.clone(),
                    age_secs: secs_since(r.created_at, now),
                    reply_by: None,
                    timed_out: false,
                });
            }
        }
        // ②·③ 미회신 계약. 같은 계약이 ①에도 나올 수 있다(파킹된 request) — 그건 중복이 아니라 **다른
        //    사실**이다("아직 안 꽂혔다" 와 "회신이 안 왔다" 는 별개 상태이고 처방도 다르다).
        for r in st.ledger.open_requests() {
            // ★수신 의무(③)를 먼저 본다★: self-send(내가 나에게 건 request)면 두 갈래가 동시에 참인데,
            //   그때 알려야 할 사실은 "네가 답해야 한다" 쪽이다(기다리기만 하면 영원히 안 닫힌다).
            let direction = if matches_contract_party(me, me_id, &r.recipient, r.recipient_id) {
                Direction::ReplyOwedByMe
            } else if matches_contract_party(me, me_id, &r.sender, Some(r.sender_id)) {
                Direction::AwaitingTheirReply
            } else {
                continue;
            };
            items.push(OpenItemView {
                direction,
                id: r.request_id.clone(),
                from: r.sender.clone(),
                to: r.recipient.clone(),
                age_secs: secs_since(r.created_at, now),
                reply_by: r.reply_by_raw.clone(),
                timed_out: r.notified,
            });
        }
        drop(st);
        // 같은 경과면 방향·id 로 안정 정렬해 응답을 결정적으로 만든다.
        items.sort_by(|a, b| {
            b.age_secs
                .cmp(&a.age_secs)
                .then_with(|| a.direction.as_str().cmp(b.direction.as_str()))
                .then_with(|| a.id.cmp(&b.id))
                .then_with(|| a.to.cmp(&b.to))
        });
        items
    }
}

/// ★계약의 한쪽 당사자가 **나인가**(D 리뷰 B1 · load-bearing)★ — 미결 조회의 의무 귀속 판정.
///
/// 규칙: 계약이 그 자리의 PeerId 를 들고 있으면 **id 로만** 판정하고, 없으면 이름으로 판정한다.
///
/// ★왜 id 가 있을 때 이름을 보지 않나★: 정확히 그 경우가 버그의 현장이다. 같은 이름의 산 에이전트가 둘일 때
///   발신자는 exact PeerId 로 한쪽에만 request 를 보낼 수 있는데, 계약이 이름으로만 기록되면 **메시지를 본
///   적도 없는 쌍둥이**가 "내가 답해야 한다"(`reply_owed_by_me`)를 받는다. 이름을 OR 로 함께 보면 그 오귀속이
///   그대로 남으므로, id 가 있으면 id 가 유일 기준이다.
/// ★왜 epoch 는 안 보나★: 같은 에이전트의 재시작은 PeerId 를 유지하고 epoch 만 올린다(ADR-0007). 재시작한
///   그 에이전트는 여전히 답할 주체이므로 epoch 로 좁히면 의무가 부당하게 사라진다.
/// ★id 가 없으면 이름 폴백(ADR-0101 WYSIWYA)★: 아직 뜨지 않은 이름 앞으로 건 request 는 나중에 그 이름으로
///   등장한 에이전트가 답할 주체다. ★이 폴백은 **운영 경로다**(ADR-0116 결정 1)★ — 잠든 수신자에게
///   건 request 는 산 incarnation 이 없어 `recipient_id = None` 으로 열리므로, 복원 전까지 의무 귀속이
///   이름으로만 성립한다(복원 후 실제 배달 시점에 `rebind_request_recipient` 가 착지 id 를 박아 그 뒤로는
///   id 축이 정본이 된다).
/// ★의도된 귀결★: 계약이 가리키던 incarnation 이 죽고 **다른** 에이전트가 같은 이름으로 뜨면 그 새 에이전트는
///   의무를 물려받지 않는다 — 그는 그 메시지를 받은 적이 없어 답할 수가 없다(모르는 요청을 떠안기는 쪽이 더
///   나쁘다). 발신자 쪽 `awaiting_their_reply` 는 그대로 남아 기한 초과 통지로 귀결된다.
fn matches_contract_party(
    me: &str,
    me_id: PeerId,
    party_name: &str,
    party_id: Option<PeerId>,
) -> bool {
    match party_id {
        Some(id) => id == me_id,
        None => party_name == me,
    }
}

/// 두 `Instant` 사이 경과 초(음수 없음). `now` 가 과거면 0 — 장부 시각은 단조 시계라 정상 흐름엔 없지만,
/// 테스트가 손으로 밀어 넣는 `now` 에서 saturating 이 없으면 패닉한다.
fn secs_since(then: Instant, now: Instant) -> u64 {
    now.saturating_duration_since(then).as_secs()
}

/// 장부 상태 → wire 어휘(spec §5). 새 문자열을 발명하지 않는다 — 발송 응답 `results[].status` 와 **같은
/// 어휘**여야 발신 LLM 이 두 응답을 같은 규칙으로 읽는다.
fn status_label(s: DeliveryStatus) -> &'static str {
    match s {
        DeliveryStatus::Pending => "pending",
        DeliveryStatus::Delivered => "delivered",
        DeliveryStatus::Replied => "replied",
        DeliveryStatus::Expired => "expired",
        DeliveryStatus::Skipped => "skipped",
        DeliveryStatus::Failed => "failed",
    }
}

/// `messages { id }` 조회 결과(입구가 JSON 으로 옮긴다 — shape 정본은 ingress `handle_messages` 주석).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageStateView {
    pub id: String,
    pub from: String,
    pub awaiting_reply: bool,
    pub may_be_truncated: bool,
    pub rows: Vec<DeliveryRowView>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryRowView {
    pub to: String,
    pub status: &'static str,
    pub age_secs: u64,
    pub updated_secs_ago: u64,
    pub code: Option<&'static str>,
    pub hint: Option<String>,
}

/// 조회 행의 코드 → 힌트 문구(**단일점** — 장부는 코드만 보관하고 문구는 여기서만 만든다).
///
/// ★왜 문구를 저장하지 않나★: 레코드마다 문자열을 들면 이력 링(4096)의 메모리가 근거 없이 늘고, 문구를
///   고칠 때 이미 저장된 옛 문구가 남아 두 표현이 공존한다. 코드가 안정 계약이고 문구는 표현이므로, 표현은
///   조회 시점에 만든다.
fn row_hint(code: &'static str) -> Option<String> {
    match code {
        c if c == FailCode::RecipientDeleted.as_str() => Some(deleted_hint()),
        // 다른 코드는 조회 축에 힌트를 두지 않는다(발송 응답이 그 자리에서 이미 전달했다 — spec §6).
        _ => None,
    }
}

/// 미결 항목의 **방향**(spec §6 무인자 조회). 세 갈래는 처방이 달라 반드시 구분해 노출한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// 내가 보냈는데 아직 수신자에게 안 꽂혔다(파킹·busy 대기). 할 일: 기다리기(또는 상대 상태 확인).
    OutboundPending,
    /// 내가 건 request 의 회신을 기다린다. 할 일: 기다리기.
    AwaitingTheirReply,
    /// 내가 받은 request 에 아직 답을 안 했다. **할 일: 지금 회신하기.**
    ReplyOwedByMe,
}

impl Direction {
    /// wire 토큰(안정 계약 — 수신 LLM 이 이 문자열로 분기한다).
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::OutboundPending => "outbound_pending",
            Direction::AwaitingTheirReply => "awaiting_their_reply",
            Direction::ReplyOwedByMe => "reply_owed_by_me",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenItemView {
    pub direction: Direction,
    pub id: String,
    pub from: String,
    pub to: String,
    pub age_secs: u64,
    /// request 였다면 발신자가 쓴 기한 표기 원본(`"10m"`). 통보·기한 없는 request 는 None.
    pub reply_by: Option<String>,
    /// 기한 초과 통지가 이미 나갔나(계약은 여전히 열려 있다 — ledger `OpenRequestView` 주석).
    pub timed_out: bool,
}

/// 수신자 1명의 **판정 결과**(락 안에서 정하고 락 밖에서 집행한다).
///
/// ★왜 판정과 집행을 나누나(락 규율)★: 적재·장부는 락 안에서 원자적으로 끝내야 하고(큐가 "비어 보이는 창"
///   금지 — `drain_queue` 주석), 주입은 절대 락 안에서 하면 안 된다(port 호출). 그래서 락 안에선 "이
///   수신자를 어떻게 할지" 만 정해 이 값으로 들고 나온다.
/// ★갈래가 둘뿐인 것이 이 개편의 모양이다(ADR-0125)★: 신원이 성립하면 **무조건 적재**(`Park`)고, 아니면
///   실패 행이다 — "지금 바로 주입한다"(옛 `Deliver`) 갈래는 직발송 지름길과 함께 사라졌다. 되살리면 적재를
///   건너뛰는 경로가 생겨 순서가 적재 순서에서 풀린다.
enum RecipientPlan<'a> {
    Park {
        /// ★드레인 대상(`None` = 잠듦 — ADR-0116 결정 1)★: 산 수신자면 이 id 로 드레인하고 park 항목의
        ///   id 힌트로도 쓴다. 잠듦은 **드레인할 산 실체가 없다**(그래서 `hinted_id` 도 `None` 이고, 배달
        ///   계기는 재등장 시 로스터 diff 가 여는 **이름 큐** 드레인뿐이다).
        target: Option<LiveAgent>,
        contract: PendingContract<'a>,
    },
    /// 이 수신자만 실패(장부 종점 기록 완료) — 락 밖에서 할 일 없음. 나머지 수신자는 그대로 간다.
    Failed { code: FailCode, hint: String },
}

/// ★드레인 1회가 무엇을 했나(ADR-0125)★ — 동기 드레인의 호출자(`handle_send`)가 **자기 편지의 결말**을
/// 읽는 창구. 비동기 계기(`flush_for`)는 답할 발신자가 없어 그대로 버린다.
///
/// ★왜 "무엇을 주입했나" 를 돌려주나★: 응답 `delivered` 의 정의가 **"이번 호출의 드레인이 실제로 주입했다"**
///   이기 때문이다(spec §6). 큐 상태나 장부를 나중에 다시 읽어 추론하면 그 사이 다른 드레인이 넣은 것까지
///   자기 공으로 세게 되고(거짓 `delivered`), 그건 이 어휘가 유일하게 보장하는 값을 무너뜨린다.
/// ★`retreated` 는 별도 축이다★: "안 나갔다" 가 아니라 **"알 수 없다"** 다(spec §6 ㉯) — 이긴 쪽 배치가
///   이미 냈을 수 있다. 그래서 사유 셋(㉮)에 섞지 않는다.
#[derive(Debug, Default)]
struct DrainReport {
    injected: Vec<String>,
    /// 0단계에서 물러났다 — 같은 수신자 앞 배치가 주입 중.
    retreated: bool,
    /// idle 게이트에 걸려 미룬 타깃이 있었다(턴 신호 있는 백엔드가 턴 중).
    gated: bool,
    /// ★자기 도어벨 금지의 판정 근거이기도 하다★.
    inject_error: Option<String>,
}

/// ★계약 예약의 **단일 출구 RAII 가드**(H1 · ADR-0108 결정 3 의 보증 소유자)★ — 이 값이 살아 있는 동안
/// 계약은 **잠정**이고, 확정(`commit`)·취소(`rollback`) 중 하나를 **반드시** 거쳐야 소멸한다.
///
/// ★왜 커밋을 결말 뒤로 미루나(A2/A3 — 두 결함을 한 번에 닫는다)★:
///   - **A2(희생자 조기 소멸)**: 커밋은 표시된 희생자를 **물리 제거**한다. 계약을 연 자리(pass A)에서 바로
///     커밋하면, 그 뒤 **같은 pass A 의 cap 게이트**가 이 수신자를 실패 행으로 떨굴 때 남의 미회신 계약은
///     **이미 사라진** 상태다 — 아무도 자리를 얻지 못했는데 남의 의무만 증발하는 ADR-0108 round-3 실패
///     모드다. ★7차에 이 창이 좁아졌다(ADR-0125)★: 결말이 **적재 락 안에서** 확정되므로(락 밖 "주입 직전
///     재확인" 갈래는 직발송과 함께 사라졌다) 남은 실패 갈래는 그 pass A 안뿐이다. 좁아졌을 뿐 사라지지
///     않았고, 커밋을 계약 오픈 자리로 되돌리면 그 갈래가 곧바로 회귀한다.
///   - **A3(같은 발송 내 자기잠식)**: 잠정 계약은 희생자 후보에서 제외된다(`ledger::open_request` 의
///     `!r.provisional` 필터). 커밋을 미루면 수신자 1의 계약이 **같은 발송의** 수신자 2에게 잡아먹히는 일이
///     구조적으로 불가능해진다(배치 스코프 마커를 따로 둘 필요가 없다).
///
/// ★왜 **타입**(RAII)이어야 하나(H1 — 라운드 1 의 `Option<Option<_>>` 이 부족했던 이유)★: 그건 평범한
///   값이라 **잊은 갈래를 컴파일러도 테스트도 잡지 못했다**(리뷰 prober 실측: 재파킹 갈래의 정산을
///   `let _ = contract.take();` 로 바꿔도 전 스위트 초록). 그런데 잊음의 대가는 **영구적**이다:
///     ① 계약이 `provisional` 로 남아 `due_timeouts` 가 영영 건너뛴다(기한 통지 소멸 = 계약이 조용히 반쪽)
///     ② 표시된 희생자가 `pending_retirement` 로 남아 `occupies_slot()` 에서 빠진다 → cap 분모가 **영구히**
///        줄고 `requests` 는 무계로 자란다.
///   그래서 "정산을 잊으면 **눈에 보이게** 실패한다" 를 타입으로 만든다: 소비되지 않은 채 Drop 되면
///   `debug_assert` 로 즉시 터지고(테스트·debug 빌드에서 잊은 갈래가 곧바로 red), 릴리즈에서는 error 로그와
///   함께 **롤백을 시도**한다.
/// ★릴리즈는 `panic = "abort"` 다★ — 즉 이 가드의 동기는 "패닉 언와인딩 복구" 가 **아니라** ① 잊은 갈래
///   부류(정적으로 못 잡는 경로 누락) ② debug/테스트 빌드의 언와인딩이다. 그 둘이 실제 노출면이다.
/// ★Drop 은 **빠른 경로**이고 그 자체가 보증은 아니다(F1 — load-bearing)★: 정산 지점은 **락 보유 중**이라
///   (`commit`/`rollback` 이 `&mut MessagingState` 를 받는다) Drop 에서 `lock()` 을 걸면 같은 스레드가 자기
///   락을 다시 잡아 **데드락**이 된다. 그래서 `try_lock` 으로만 시도하고, 실패하면(락 경합 · 같은 스레드가
///   락 보유 중 · poison) 롤백을 **건너뛴다**.
/// ★보증 = 주기 sweep 의 **버려진** 예약 회수(`ledger::reclaim_abandoned_reservations`)★: 락을 정상적으로
///   소유하는 유지보수 지점이 소유자가 사라진 잠정 예약을 되돌린다. 즉 최종 보증은 **Drop 타이밍에 의존하지
///   않는다** — Drop 이 성공하면 즉시, 실패해도 늦어도 다음 sweep 틱에 정리된다.
/// ★7차 보정 — 이 창이 어디 사는지가 바뀌었다(ADR-0125). 결론(가드 존치)은 그대로다★: 전부-큐가 되면서
///   open(`open_reservation`) → settle(`commit`/`rollback`)이 **적재 락 한 구간 안에** 전부 들어왔다. 그래서
///   정상 경로에는 "예약을 든 채 락을 놓고 주입하는" 구간이 없고, `try_lock` 이 실패하는 경우도 **같은 스레드가
///   락을 쥔 언와인딩** 부류로 좁혀졌다(그 갈래에선 sweep 의 poison `expect` 도 못 넘어간다 — 즉 위 "다음 sweep
///   틱" 폴백은 그 좁은 부류에선 실효가 없다). ★그러니 "락 밖 주입이 없어졌으니 이 기계는 obsolete" 로 읽지
///   말 것★ — 이 가드가 실제로 막는 것은 **정적으로 못 잡는 정산 누락(잊은 갈래)** 이고 그 위험은 창의 길이와
///   무관하다(오히려 두 pass 를 오가는 지금 형태에서 더 쉽게 생긴다). 제거 = ADR-0108 재론이며, 그 순간
///   `STALE_RESERVATION_AFTER` 부류의 회귀가 되돌아온다.
/// ★그 회수는 **이 가드의 생존**을 본다(R1 — load-bearing)★: 근거 전문은 `ledger::ReservationLiveness`
///   헤더. 이 필드를 지우거나 `mem::forget` 로 우회하면 그 보증이 무너진다.
/// ★언와인딩 중에는 `debug_assert` 를 건너뛴다★: Drop 은 패닉 중에도 불리고 거기서 다시 패닉하면 **이중
///   패닉 → abort** 다(테스트 프로세스가 죽어 원인조차 못 본다). 그래서 `thread::panicking()` 이면 기록만
///   남기고, 회수는 sweep 이 맡는다.
// ADR-0108 (mark-and-sweep — 예약 창의 보증 소유자)
struct Reservation<'a> {
    svc: &'a MessagingService,
    msg_id: String,
    recipient: String,
    /// 상한 압력으로 **은퇴 예정 표시**된 희생자(있으면). 커밋 = 물리 제거 + 계측, 롤백 = 표시 해제.
    retired: Option<RetiredContract>,
    /// 정산 완료 표식 — `commit`/`rollback` 이 세운다. Drop 은 이게 false 일 때만 개입한다.
    settled: bool,
    /// ★소유자 생존 토큰(R1)★ — 이 값이 살아 있는 동안 sweep 은 이 예약을 회수하지 않는다. 읽지 않고
    /// **들고만 있는** 게 역할이다(장부가 약한 쪽으로 관찰한다).
    _liveness: ReservationLiveness,
}

/// ★가드 생성 + 생존 토큰 부착을 **한 동사로** 묶는다(R1 — load-bearing)★.
///
/// 두 단계를 호출자가 따로 하면 "가드는 만들었는데 붙이는 걸 잊은" 갈래가 생기고, 그 예약은 소유자가
/// 일하는 중에도 sweep 에게 버려진 것으로 보인다(= 라운드 4가 잡은 실패 사슬의 재발). 그래서 예약을 여는
/// 유일한 방법을 이 함수로 만든다 — `Reservation::new` 를 직접 부르는 새 경로를 추가하지 말 것.
/// ★호출 위치 = `open_request` 와 **같은 락 구간**★(그래서 `st` 를 받는다 — 그 사이 sweep 이 끼어들 창을
/// 만들지 않는다).
fn open_reservation<'a>(
    st: &mut MessagingState,
    svc: &'a MessagingService,
    msg_id: &str,
    recipient: &str,
    retired: Option<RetiredContract>,
) -> Reservation<'a> {
    let liveness = ReservationLiveness::new();
    st.ledger
        .attach_reservation_liveness(msg_id, recipient, liveness.watch());
    Reservation::new(svc, msg_id, recipient, retired, liveness)
}

impl<'a> Reservation<'a> {
    /// ★호출자 의무(R1)★: `liveness.watch()` 를 **같은 락 구간에서** 장부 항목에 붙여야 한다
    /// (`Ledger::attach_reservation_liveness`).
    fn new(
        svc: &'a MessagingService,
        msg_id: &str,
        recipient: &str,
        retired: Option<RetiredContract>,
        liveness: ReservationLiveness,
    ) -> Self {
        Self {
            svc,
            msg_id: msg_id.to_string(),
            recipient: recipient.to_string(),
            retired,
            settled: false,
            _liveness: liveness,
        }
    }

    fn commit(mut self, st: &mut MessagingState, log: &mut RetirementLog) {
        self.settled = true;
        let retired = self.retired.take();
        let outcome = st.ledger.commit_open(
            Some((self.msg_id.as_str(), self.recipient.as_str())),
            retired
                .as_ref()
                .map(|r| (r.request_id.as_str(), r.recipient.as_str())),
        );
        // ★계측은 **일어난 일**만 보고한다(R2)★: 계획한 희생자가 그 사이 사라졌을 수 있다(회신으로 닫히고
        //   이력 행까지 밀려나면 `purge_finished_without_history` 가 정리한다 — `ledger::rollback_open` 의
        //   알려진 잔여). 계획을 사실로 보고하면 ADR-0108 결정 2 가 은퇴의 유일한 증거라고 못 박은 축이
        //   오염된다. 그래서 장부가 **실제로 제거했다고 답할 때만** 은퇴로 싣고, 어긋나면 이상 기록으로 돌린다.
        if let Some(r) = retired {
            match outcome.retired {
                true => log.real.push(r),
                false => log.phantom.push(r),
            }
        }
    }

    fn rollback(mut self, st: &mut MessagingState) {
        self.settled = true;
        let retired = self.retired.take();
        rollback_reservation(
            st,
            self.msg_id.as_str(),
            self.recipient.as_str(),
            retired.as_ref(),
        );
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        let retired = self.retired.take();
        match self.svc.state.try_lock() {
            Ok(mut st) => {
                rollback_reservation(
                    &mut st,
                    self.msg_id.as_str(),
                    self.recipient.as_str(),
                    retired.as_ref(),
                );
                tracing::error!(
                    msg_id = %self.msg_id,
                    recipient = %self.recipient,
                    "정산되지 않은 계약 예약을 Drop 에서 롤백 — 호출 경로에 커밋/롤백 누락이 있다(H1)"
                );
            }
            Err(_) => tracing::error!(
                msg_id = %self.msg_id,
                recipient = %self.recipient,
                "정산되지 않은 계약 예약을 롤백하지 못함(락 보유 중이거나 poison) — 잠정 계약과 은퇴 표시가 남는다(H1)"
            ),
        }
        // ★롤백 **뒤에** 터뜨린다★: 순서를 뒤집으면 패닉이 복구를 막아 "정산을 잊었을 때 상태가 온전한가" 를
        //   테스트가 관측할 수 없다(그리고 릴리즈에선 debug_assert 가 없어 복구만 남는다 — 같은 코드 경로).
        debug_assert!(
            std::thread::panicking(),
            "회신 계약 예약이 정산 없이 소멸했다 — commit/rollback 중 하나를 반드시 거쳐야 한다(H1)"
        );
    }
}

/// 예약 취소의 알맹이(가드의 `rollback` 과 Drop 이 공유 — 규칙이 두 곳에 갈리지 않게).
fn rollback_reservation(
    st: &mut MessagingState,
    msg_id: &str,
    recipient: &str,
    retired: Option<&RetiredContract>,
) {
    st.ledger.rollback_open(
        Some((msg_id, recipient)),
        retired.map(|r| (r.request_id.as_str(), r.recipient.as_str())),
    );
}

/// 아직 정산되지 않은 예약(계약이 없는 통보/회신은 `None`).
type PendingContract<'a> = Option<Reservation<'a>>;

struct ResolvedRecipient {
    /// 응답 행에 실을 **발신자 표기**(트림만 — WYSIWYA, ADR-0101). `@` 펼침 결과면 로스터 이름이다.
    display: String,
    /// 장부·파킹 키. 해석되면 canonical 이름, 못 하면 표기 그대로.
    ///
    /// ★왜 표기와 키를 나누나(load-bearing)★: 파킹·flush 는 **이름-키**다(재스폰 이어받기 — ADR-0101).
    ///   그런데 발신자는 exact PeerId 로도 지목할 수 있어, 그 표기를 그대로 park 키로 쓰면 등장 flush(이름
    ///   키)가 그 큐를 영영 못 연다. 그래서 해석되면 canonical 이름을 키로 쓰고, 응답에는 발신자가 쓴
    ///   표기를 그대로 돌려준다(둘은 정상 경로에서 동일하다).
    key: String,
    /// ★해석된 산 수신자(발송 순간 스냅샷 기준)★ — `None` = 부재 또는 동명 다수.
    ///
    /// ★왜 여기 들고 다니나(load-bearing)★: 해석을 **한 번만** 한다(ADR-0111 결정 2 "스냅샷 한 장"). 판정
    ///   단계에서 `key` 로 다시 풀면 **exact-PeerId 지목이 이름 해석으로 강등**돼, 동명 다수를 의도적으로
    ///   통과하던 id 지목이 `RECIPIENT_AMBIGUOUS` 로 뒤집힌다(같은 스냅샷인데 답이 갈리는 자기모순).
    target: Option<LiveAgent>,
    live_count: usize,
    // ADR-0116 (결정 1 — 잠듦 파킹 · 잠듦 층 동명)
    dormant_count: usize,
    /// ★자기 행을 가질 수 없는 중복 지목(M3 · ADR-0116 결정 5)★ — `Some(접힌 키)` 면 이 토큰은 **다른
    ///   실체**를 가리키는데 park/장부 키(= 이름)가 앞선 행과 같아 자기 자리를 만들 수 없다. 그 사실을
    ///   `RECIPIENT_AMBIGUOUS` **실패 행**으로 드러낸다(조용히 사라지지 않게 — `push_recipient` 주석).
    dup_of: Option<String>,
}

/// 입력 토큰 하나(입력 순서 보존) — 봉투 `to` 조립의 순서 축이자 **그 토큰이 기여한 수신자 키**의 출처.
///
/// ★왜 "기여한 키" 를 토큰이 들고 있나(A5 회귀 · load-bearing)★: 옛 구현은 `to` 속성 포함 여부를 "그 토큰이
///   중복 제거를 통과해 **행을 만들었나**" 로 판정했다. 그러면 `["bob","carol","@all"]` 처럼 `@all` 의 펼침이
///   앞선 명시 지목에 **완전히 흡수**된 경우 `@all` 이 통째로 빠진다(`to="bob,carol"`). spec §1 은 값이
///   **발신자가 쓴 토큰**의 나열이고 빠지는 건 **실패한 토큰**뿐이라고 못 박는다 — 흡수는 실패가 아니다.
///   그래서 토큰마다 자기가 해석해 낸 키 전부를 (중복 제거 **이전**에) 기록하고, 포함 조건을 "그 키들 중
///   하나라도 수용 판정됐나" 로 바꾼다.
enum AddressToken {
    Name { key: String },
    Group { label: String, keys: Vec<String> },
}

struct Addressing {
    recipients: Vec<ResolvedRecipient>,
    tokens: Vec<AddressToken>,
}

/// ★수신자 해석(spec §5 해석 순서 · 순수 — 로스터 스냅샷만 본다)★.
///
/// 순서: ① 트림(콤마 분해는 **CLI 입구 전용**이라 이미 끝나 있다) → ② `@` 원소 펼침(**발신자 제외**) →
/// ③ 이름 원소와 합류 → ④ 중복 제거(수신자 1명 = 배달 1회·결과 1행) → ⑤ 로스터 대조는 호출자가 한다.
///
/// ★행 순서 = 결정적(테스트 고정 — spec §5)★: ① 입력에 적힌 **명시 토큰을 적힌 순서대로**, ② 그 뒤에
///   `@`주소 **펼침 결과를 이름 사전순**으로 잇는다. 중복 제거는 **먼저 나온 것을 남긴다**(명시 지목이
///   펼침 결과보다 항상 앞서므로, 겹친 수신자의 행 위치는 명시 위치를 따른다).
/// ★중복 제거 축 = park 키(해석된 canonical 이름)★: 표기 문자열이 아니라 키로 접어야 `@all` 이 낸 이름과
///   같은 에이전트를 가리키는 exact-id 지목이 두 행으로 갈리지 않는다.
/// ★`@`주소 오류 = 발송 단위 전체 반려(ADR-0114 결정 3)★: `@all` 이 아닌 `@이름`·규약 위반은
///   `GROUP_NOT_FOUND` 다(혼용 `to` 여도 나머지 수신자에게 가지 않는다). **최종 집합이 비면**
///   `GROUP_EMPTY` — 원소 단위가 아니라 펼침 ∪ 명시 지목의 결과로 판정한다.
/// ★펼침에서 발신자 제외(spec §4 — **정본은 spec 이지 ADR-0111 이 아니다**)★: 두 어휘 모두 "나 빼고" 다.
///   ADR-0111 결정 4 엔 그 문구가 없으니 거기서 찾지 말 것 — 빼먹으면 발신자가 자기 방송을 받는다.
///   **직접 지목 자기발송은 그대로 배달**되므로(제외는 펼침에만 적용) `["@all", "<자기이름>"]` 은 자기에게
///   1행이 나간다.
// ADR-0111 (그룹 = 해석 매크로 · 다중 수신자 합류)
// ADR-0114 (@주소 오류 층위 · GROUP_EMPTY = 최종 집합)
// ADR-0121 (두 어휘 — @all = 명부 전원 / @here = 산 전원)
fn resolve_addressing(
    groups: &dyn GroupSource,
    to: &[String],
    from: SenderIdentity,
    sender_name: &str,
    sources: &AddressingSources,
) -> Result<Addressing, SendReject> {
    // ★어느 어휘가 어느 풀을 읽는지는 **소스**가 정한다(`groups::MemberPools`)★ — 여기서 하는 일은 풀을
    //   만들고 **발신자를 빼는 것**뿐이다(ADR-0121 결정 1).
    //   ★산 풀에서 id 로 빼는 이유★: 이름은 겹칠 수 있어 동명 타인까지 함께 빠지면 안 된다(그 타인은 동명
    //   규칙으로 따로 판정된다 — ADR-0114 결정 4).
    //   ★턴 신호 없는 산 세션은 산 풀에 **들어온다**★ — 로스터 자격에 capability 조건이 없으므로(ADR-0116
    //   결정 7) 방송이 그 부류를 빼지 않는다(게이트 없이 즉시 주입된다).
    let live_names: Vec<String> = sources
        .roster
        .iter()
        .filter(|a| a.id != from.peer_id)
        .map(|a| a.name.clone())
        .collect();
    // ★잠듦 풀에서는 **이름으로** 뺀다(load-bearing — 발신자 제외를 뚫는 경로 차단)★: 발신자와 같은 이름의
    //   잠든 프로필이 있으면 그 이름이 `@all` 펼침에 들어오고, 상위 해석은 "산 쪽이 이긴다" 규칙에 따라 그
    //   이름을 **발신자 자신의 산 세션**으로 풀어 자기 편지를 배달한다(spec §4 발신자 제외 위반). 이름 유일성
    //   (ADR-0120)이 그런 프로필의 존재를 막지만, 그 보증이 깨졌을 때 무너지는 쪽이 자기 방송 메아리라
    //   여기서 한 번 더 막는다. 잠듦 층 동명은 여전히 접지 않는다(같은 이름 2개 → `RECIPIENT_AMBIGUOUS`).
    let dormant_names: Vec<String> = sources
        .dormant_names
        .iter()
        .filter(|n| n.as_str() != sender_name)
        .cloned()
        .collect();
    let pools = MemberPools {
        live: &live_names,
        dormant: &dormant_names,
    };

    let mut tokens: Vec<AddressToken> = Vec::with_capacity(to.len());
    let mut recipients: Vec<ResolvedRecipient> = Vec::new();
    let mut expanded: Vec<(usize, String)> = Vec::new();

    for (i, raw) in to.iter().enumerate() {
        let token = raw.trim();
        if token.starts_with('@') {
            let norm = normalize_group_name(token).map_err(|e| group_reject(e, token))?;
            let members = groups
                .resolve(&norm, pools)
                .map_err(|e| group_reject(e, token))?;
            for m in members {
                expanded.push((i, m));
            }
            tokens.push(AddressToken::Group {
                label: norm,
                keys: Vec::new(),
            });
        } else {
            let key = push_recipient(&mut recipients, token.to_string(), sources);
            tokens.push(AddressToken::Name { key });
        }
    }
    expanded.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
    for (i, name) in expanded {
        let key = push_recipient(&mut recipients, name, sources);
        if let AddressToken::Group { keys, .. } = &mut tokens[i] {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }

    if recipients.is_empty() {
        // 명시 지목이 하나라도 있으면 여기 올 수 없다(이름 토큰은 해석 실패해도 행을 만든다) — 즉 이 갈래는
        //   "`@`주소만 있었고 전부 0명으로 펼쳐졌다" 뿐이다(발신자만 살아 있는 `["@all"]` 이 전형).
        return Err(SendReject::GroupEmpty);
    }
    Ok(Addressing { recipients, tokens })
}

/// 해석된 수신자 1명을 행 목록에 추가하고 **그 수신자의 park 키**를 돌려준다 — 키 기준 중복 제거.
///
/// ★중복 제거 = 먼저 나온 자리를 남긴다(spec §5)★ — 겹친 수신자의 **행 위치**는 명시 지목 자리를 따른다.
/// ★자기 행을 못 갖는 중복은 **보이게 실패시킨다**(M3)★: 두 토큰이 **서로 다른 산 실체**를
///   exact-id 로 지목했는데 그 둘의 canonical 이름이 같으면(동명), 뒤 토큰은 park·장부 키를 앞 행과 공유할 수
///   없어 자기 자리를 만들 수 없다 — 옛 구현은 그 토큰을 **행 없이 조용히 삼켰다**(spec §6 "수신자 1명 = 1행"
///   위반). 실체 키 병행 트랙은 비용 과다로 거부됐으므로(ADR-0114 거부 대안), **가시적 실패**로 강등한다.
/// ★단 하나의 예외: 해석된 쪽이 이긴다(A8)★ — 두 토큰이 같은 키로 접힐 때 한쪽만 `target` 을 갖고 있으면
///   (= exact-PeerId 지목은 동명 다수를 의도적으로 통과한다) **행 위치는 앞선 것을 유지하면서 해석 결과만
///   그쪽으로 갈아끼운다**. 왜: 그러지 않으면 `["dup", "<dup1의 id>"]` 가 `RECIPIENT_AMBIGUOUS` 로 끝나고,
///   순서를 뒤집으면 배달되는 비대칭이 생긴다. 표기(`display`)는 먼저 쓴 것을 남긴다 — 응답의 `to` 는
///   발신자가 처음 쓴 표기라는 WYSIWYA 계약을 지킨다.
/// ★위 두 규칙의 지위(ADR-0116 결정 5 — 재가 완료)★: **정책으로 승격하지 않는다**(spec 문구화 없음).
///   동명이인 자체가 **미지원**이고 뿌리 제거는 ADR-0115(스폰 이름 유일성)가 한다 — 그때까지 이 코드는
///   "조용한 행 소멸 방지" 라는 **과도기 방어**로만 존치한다(그래서 힌트도 exact-id 재발송을 가르치지 않고
///   사용자 에스컬레이션을 가리킨다 — `ambiguous_hint`). 사용자 재가 대기 상태가 아니다.
fn push_recipient(
    recipients: &mut Vec<ResolvedRecipient>,
    display: String,
    sources: &AddressingSources,
) -> String {
    // ★① 같은 **raw 토큰**을 두 번 적었으면 행은 하나다(F2-a)★: 표기가 글자 그대로 같으면 정보가 0 인
    //   중복이다. 이 검사를 앞에 두지 않으면 반복된 exact-id 토큰이 아래 M3 갈래를 매번 새로 타서 **같은
    //   실패 행이 여러 줄** 나온다(`[A-id, B-id, B-id]` → B 행 2줄).
    if let Some(existing) = recipients.iter().find(|r| r.display == display) {
        return existing.key.clone();
    }
    let target = resolve_live(&display, &sources.roster);
    let key = target
        .as_ref()
        .map(|a| a.name.clone())
        .unwrap_or_else(|| display.clone());
    // ★② 자리 다툼은 **실 수신자 행**끼리만 한다(F2-b)★: loser 행(`dup_of`)은 이름 공간의 점유자가 아니다 —
    //   그걸 후보에 넣으면 뒤 토큰이 loser 행의 자리를 물려받아(A8 승격 경로) 그 실패가 사라지고 남의 표기로
    //   배달이 보고된다. loser 키는 애초에 이름 공간 밖이지만(아래 `loser_key`), 검색 조건도 명시해 둔다.
    if let Some(pos) = recipients
        .iter()
        .position(|r| r.dup_of.is_none() && r.key == key)
    {
        let existing_target = recipients[pos].target.as_ref().map(|a| a.id);
        let new_target = target.as_ref().map(|a| a.id);
        match (existing_target, new_target) {
            (None, Some(_)) => {
                recipients[pos].live_count =
                    sources.roster.iter().filter(|a| a.name == key).count();
                recipients[pos].target = target;
                // 로스터로 해석됐으므로 로스터 밖 축(잠듦)은 더 이상 의미가 없다 — 지운다.
                //   (안 지우면 앞 행이 잠듦으로 판정됐던 흔적이 남아 3분기가 두 갈래를 동시에 본다.)
                recipients[pos].dormant_count = 0;
            }
            (Some(e), Some(n)) if e != n => {
                let lkey = loser_key(&display);
                recipients.push(ResolvedRecipient {
                    key: lkey.clone(),
                    display,
                    target: None,
                    live_count: 0,
                    dormant_count: 0,
                    dup_of: Some(key),
                });
                // 봉투 `to` 판정이 이 토큰을 "수용" 으로 세지 않게 **loser 키**를 돌려준다(`loser_key`).
                return lkey;
            }
            _ => {}
        }
        return key;
    }
    let live_count = sources.roster.iter().filter(|a| a.name == key).count();
    // ★잠듦 축은 **로스터 판정이 끝난 뒤에만** 계산한다(spec §5 분기 순서 — 산 쪽이 이긴다)★:
    //   로스터로 해석됐거나(target) 로스터 동명(live_count ≥ 2)이면 결말이 이미 정해졌으므로 프로필 층을
    //   보지 않는다. 그래야 "산 1 + 잠든 1 동명 → 산 쪽으로 배달" 이 성립한다(잠듦 판정이 끼어들 여지 없음).
    let dormant_count = if target.is_some() || live_count > 0 {
        0
    } else {
        sources.dormant_names.iter().filter(|n| **n == key).count()
    };
    recipients.push(ResolvedRecipient {
        display,
        key: key.clone(),
        target,
        live_count,
        dormant_count,
        dup_of: None,
    });
    key
}

/// ★자기 행을 못 갖는 중복 지목(loser 행)의 **장부·판정 키**(F2 — load-bearing)★.
///
/// ★왜 표기(`display`)를 그대로 쓰면 안 되나(실측 세 갈래)★: 표기는 **canonical 이름 공간과 같은 공간**이다.
///   ① 어떤 에이전트 C 의 canonical 이름이 우연히 문자열 `"<B의 uuid>"` 와 같으면, C 를 지목한 토큰이 B 의
///   loser 행을 자기 자리로 착각해(또는 그 반대로) B 의 실패가 사라지고 C 의 배달이 B 의 표기로 보고된다
///   ② 순서를 뒤집으면 장부 키 `(msg_id, "<B의 uuid>")` 가 **두 행**에 중복 발급된다 ③ `build_to_attr` 의
///   키 조회가 실패 행을 수용 행으로 오독할 수 있다.
/// ★해법 = 이름 공간 밖의 키★: `U+0001`(제어문자) 접두를 붙인다. canonical 이름은 display_name 또는 경로
///   basename 에서 오고 그 어느 쪽도 제어문자를 담을 수 없으므로(파일시스템·표시 이름 모두 금지) 이 접두가
///   붙은 문자열은 **어떤 에이전트의 이름도 될 수 없다** — 즉 충돌이 구조적으로 불가능하다.
/// ★행마다 유일★: 표기별로 유일하고(같은 표기는 위 ① 검사에서 이미 접힌다) 그래서 장부 종점 키도 행마다
///   하나씩이다. 응답 `results[].to` 는 여전히 **발신자 표기**라 이 내부 키는 발신 LLM 에게 보이지 않는다.
fn loser_key(display: &str) -> String {
    format!("\u{1}ambiguous:{display}")
}

/// `@`주소 해석 에러 → 발송 단위 반려. 규약 위반도 "그런 주소는 없다" 로 접는다 — 발신자에게 맞는 사실이고
/// spec §6 어휘에 없는 새 코드를 만들지 않는다(교정 방법은 입구의 hint 가 알려 준다).
fn group_reject(e: GroupError, raw: &str) -> SendReject {
    match e {
        GroupError::NotFound { name } => SendReject::GroupNotFound { name },
        GroupError::InvalidName { .. } => SendReject::GroupNotFound {
            name: raw.to_string(),
        },
    }
}

/// ★봉투 `to` 값 조립(spec §1 — 동결 대상)★: **수용된 명시 지목은 정규 이름**, **`@`주소는 펼치지 않고
/// 토큰 그대로**, **실패한 토큰은 제외**, 나열은 **입력 표기 순**(`"@all,carol"`).
///
/// ★포함 조건 = "그 토큰이 해석해 낸 수신자가 **전원 실패는 아니다**"(A5 회귀 — load-bearing)★:
///   판정 축은 "행을 만들었나" 가 **아니다**. 중복 제거로 흡수된 토큰(`["bob","carol","@all"]` 의 `@all`)은
///   행을 만들지 않지만 **실패한 것도 아니다** — 그 수신자들은 다른 토큰 자리에서 정상 배달된다. spec §1 이
///   빼라고 한 건 **실패한 토큰**뿐이므로, 흡수는 포함 사유를 잃게 하지 않는다.
/// ★같은 값은 한 번만 적는다(M4)★: 표기 중복(`["bob","bob"]`)이나 접힌 지목(`["<dup1 id>","dup"]`)은
///   **같은 수신자 하나**를 가리키므로 `to="bob,bob"` 은 수신 LLM 에게 거짓 인원수를 알린다. 먼저 나온
///   자리를 남기고 뒤 중복을 접는다(행 순서 규칙과 같은 방향).
/// ★펼친 명단을 대신 싣지 않는다★ — 수신자 명단·역할 공개(cc)는 v2 다(spec §8).
fn build_to_attr(addr: &Addressing, admitted: &[bool]) -> String {
    // `admitted` 의 인덱스는 `recipients` 순서와 같다.
    let admitted_key = |key: &str| {
        addr.recipients
            .iter()
            .position(|r| r.key == key)
            .is_some_and(|pos| admitted[pos])
    };
    let mut parts: Vec<String> = Vec::new();
    for token in &addr.tokens {
        match token {
            AddressToken::Name { key } => {
                if admitted_key(key) && !parts.iter().any(|p| p == key) {
                    parts.push(key.clone());
                }
            }
            AddressToken::Group { label, keys } => {
                if keys.iter().any(|k| admitted_key(k)) && !parts.iter().any(|p| p == label) {
                    parts.push(label.clone());
                }
            }
        }
    }
    parts.join(",")
}

fn live_names_hint(roster: &[LiveAgent]) -> String {
    if roster.is_empty() {
        return "(none)".to_string();
    }
    roster
        .iter()
        .map(|a| a.name.clone())
        .collect::<Vec<_>>()
        .join(", ")
}

/// ★로스터 밖 수신자의 결말(spec §5 분기 1/3 — ADR-0116 결정 1)★. `Failed` = 실패 행(파킹 없음) ·
/// `Dormant` = 잠듦 파킹(수용).
enum AbsentDisposition {
    Failed { code: FailCode, hint: String },
    Dormant,
}

/// ★로스터 밖 수신자를 3분기의 나머지 두 갈래로 가르는 **단일 판정점**(spec §5 · ADR-0116 결정 1)★.
///
/// 판정 순서가 곧 정책이다(순서를 바꾸면 결말이 바뀐다):
///   ① **접힌 중복 지목**(`dup_of`) → `RECIPIENT_AMBIGUOUS`(과도기 방어 — `push_recipient` 헤더)
///   ② **로스터 동명 다수** → `RECIPIENT_AMBIGUOUS`(산 층)
///   ③ **잠듦 동명 다수** → `RECIPIENT_AMBIGUOUS`(잠듦 층 — 이름 키 파킹은 "먼저 복원된 쪽이 조용히 받는"
///      구멍을 만든다)
///   ④ **잠듦 1개** → 파킹(수용) · ⑤ **아무 데도 없음** → `RECIPIENT_NOT_FOUND`
/// ★로스터 판정이 이 함수보다 먼저다(호출자)★: 로스터에 있으면 여기 오지 않는다 — 턴 신호가 없어도 그건
///   배달 대상이다(ADR-0116 결정 7. 옛 `RECIPIENT_UNREACHABLE` 갈래는 폐기 — 되살리지 말 것).
// ADR-0116 (결정 1 — 3분기 판정)
fn absent_disposition(r: &ResolvedRecipient, roster_names: &str) -> AbsentDisposition {
    if let Some(folded) = &r.dup_of {
        return AbsentDisposition::Failed {
            code: FailCode::RecipientAmbiguous,
            hint: format!(
                "'{}' resolves to a different agent that shares the name '{folded}' with another recipient of this send — the broker addresses mailboxes by name, so only one of them can be a recipient here. {}",
                r.display, AMBIGUOUS_ESCALATION
            ),
        };
    }
    if r.live_count >= 2 {
        return AbsentDisposition::Failed {
            code: FailCode::RecipientAmbiguous,
            hint: ambiguous_hint(&r.display, r.live_count, "live"),
        };
    }
    if r.dormant_count >= 2 {
        return AbsentDisposition::Failed {
            code: FailCode::RecipientAmbiguous,
            hint: ambiguous_hint(&r.display, r.dormant_count, "saved"),
        };
    }
    if r.dormant_count == 1 {
        return AbsentDisposition::Dormant;
    }
    AbsentDisposition::Failed {
        code: FailCode::RecipientNotFound,
        hint: format!(
            "No agent named '{}' exists right now — not running and no saved agent by that name, so nothing was queued for it (fix the name, or create/spawn it and send again). Live agents: {roster_names}.",
            r.display
        ),
    }
}

/// ★회신 발송의 계약 처리 갈래(spec §3 항목 7-④)★ — `handle_send` 6단계가 이 값으로 분기한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplyDisposition {
    /// 수용됨(`delivered`|`pending`) → `replied` 닫힘. **파킹도 수용이다**.
    Accepted,
    /// 요청자 이름이 어디에도 없다(`RECIPIENT_NOT_FOUND`) → `reply_failed` 실패 종결.
    RequesterGone,
    /// 그 밖의 실패(`MAILBOX_FULL`·`RECIPIENT_AMBIGUOUS`) → **무동작**(오픈 유지).
    NoOp,
}

/// ★회신 결말 판정(spec §3 항목 7-④ · ADR-0116 결정 2)★ — 응답 행만 보고 정한다(장부를 다시 보지 않는다).
///
/// ★회신은 수신자가 정확히 1명이다(spec §3 항목 7-①, ingress 가 표기 단계에서 강제)★. 그래도 이 함수는
///   행 목록으로 판정한다 — `handle_send` 는 pub 이라 다른 조립(스모크 bin·미래 입구)이 여러 행으로 부를 수
///   있고, 그때 "하나라도 수용됐으면 수용" 이 유일하게 안전한 해석이다(회신 하나가 실제로 도착했다면 의무는
///   이행됐다). ★`RECIPIENT_NOT_FOUND` 는 **전 행이 실패**일 때만 본다★: 한 명에게라도 갔으면 수용이 이긴다.
// ADR-0116 (결정 2)
fn reply_disposition(results: &[RecipientResult]) -> ReplyDisposition {
    if results.iter().any(|r| r.status != SendStatus::Failed) {
        return ReplyDisposition::Accepted;
    }
    if results
        .iter()
        .any(|r| r.code == Some(FailCode::RecipientNotFound))
    {
        return ReplyDisposition::RequesterGone;
    }
    ReplyDisposition::NoOp
}

/// ★동명 실패 힌트의 공통 꼬리(ADR-0116 결정 5 — 사용자 결정 2026-07-30)★.
///
/// ★exact-id 재발송 안내를 **하지 않는다**★: 동명이인 자체가 미지원이므로(뿌리 제거 = ADR-0115) id-구제
///   흐름을 지원 정책처럼 가르치면 안 된다. 발신 LLM 이 해야 할 일은 재시도가 아니라 **사용자에게 알리는
///   것**이다 — 재발송은 같은 결과를 반복하고 컨텍스트만 태운다.
const AMBIGUOUS_ESCALATION: &str = "Duplicate agent names are not supported: do NOT resend this message — tell the user, who can rename or retire one of them.";

/// 동명 다수 실패 행 hint(산 층·잠듦 층 공용 — `layer` 는 사람이 읽을 층 이름).
///
/// ★카운트와 라벨이 반드시 같은 층을 가리켜야 한다(리뷰 fix D6-a)★: 산 층은 **로스터 카운트**를 세고
///   라벨이 `live` 다 — 4차 로스터는 "산 세션 전원"(턴 신호 무관)이므로 그 라벨이 사실과 정확히 맞는다
///   (3차엔 구조화 조건이 걸려 있어 "live" 가 실제보다 적은 수를 가리켰다). 잠듦 층은 프로필 카운트 +
///   `saved` 다. 한쪽만 바꾸면 발신자가 "둘이라는데 하나만 보인다" 로 읽는다(spec §5 힌트 라벨 규칙).
fn ambiguous_hint(display: &str, count: usize, layer: &str) -> String {
    format!("'{display}' matches {count} {layer} agents, so the broker cannot tell which one you mean — nothing was queued. {AMBIGUOUS_ESCALATION}")
}

/// ★삭제 정리로 종결된 파킹분의 조회 힌트(spec §6 `RECIPIENT_DELETED`)★ — 코드 옆에 다음 행동까지 실어 준다.
fn deleted_hint() -> String {
    "The recipient was deleted while this message was still parked, so it was closed as undelivered — that name no longer exists, resending is pointless. Tell the user if this still matters.".to_string()
}

/// 잠듦 파킹 hint(spec §5 분기 3 — ADR-0116 결정 1). 발신자가 "왜 pending 인가" 와 결말을 읽게 한다.
///
/// ★만료는 조용하다(사용자 결정 2026-07-30 — 알고 수용)★: 아무도 복원하지 않으면 24h TTL 로 `expired` 되고
///   **발신자에게 능동 통지는 없다**(발송 응답의 `pending` + 이후 `messages` 조회가 전부). 그래서 그 사실을
///   힌트에 적어 둔다 — 발신 LLM 이 "언젠가 반드시 간다" 로 오독하지 않게.
fn park_hint_dormant(display: &str) -> String {
    format!(
        "'{display}' is not running right now but it is a saved agent — parked; it will be delivered as one batch when that agent is restored. Nobody is notified if it expires first (24h TTL), so check with `messages` if it matters."
    )
}

/// busy hint(spec §6 ㉮①).
fn park_hint_busy(display: &str) -> String {
    format!(
        "'{display}' is mid-turn — queued; it will be delivered as one batch when that turn ends."
    )
}

/// 주입 실패 hint(spec §6 ㉮③).
fn park_hint_inject_failed(display: &str, err: &str) -> String {
    format!(
        "Delivery to '{display}' failed ({err}) — it stays queued and is retried on the next drain (expires after TTL)."
    )
}

/// ★겹친 드레인에서 물러났다 — "확인 불가" hint(spec §6 ㉯ · load-bearing)★.
///
/// ★문구가 "안 갔다" 로 읽히면 안 된다★: 이 편지는 **이긴 쪽 배치에 실려 이미 배달됐을 수 있다**. 발신
///   LLM 이 `pending` 을 "상대가 못 받았다" 로 단정해 보고하는 것이 spec §7 프라이밍이 막는 오독이고,
///   힌트가 그 오독을 부추기면 안 된다 — 실제 결말은 `messages{id}` 조회로 본다.
fn park_hint_overlapping(display: &str) -> String {
    format!(
        "Another send to '{display}' was draining that mailbox at the same time, so this call could not confirm delivery — it was NOT lost: it either went out with that batch or is still queued. Check `messages` if you need to know which."
    )
}

/// 드레인이 타깃을 풀지 못해 큐에 남았다(이름이 부재/동명 다수가 됐고 id 힌트도 죽음).
fn park_hint_queued(display: &str) -> String {
    format!(
        "'{display}' could not be reached by this drain — it stays queued and goes out on the next occasion (appearance / idle transition / a later send's drain)."
    )
}

/// ★드레인이 이번 편지를 못 냈을 때의 `pending` hint 선택(spec §6)★ — 물러남(㉯)이 최우선이다(다른 사유와
/// 섞이면 발신자가 결말을 오독한다 — `park_hint_overlapping`).
fn pending_hint(display: &str, report: &DrainReport) -> String {
    if report.retreated {
        return park_hint_overlapping(display);
    }
    if let Some(err) = &report.inject_error {
        return park_hint_inject_failed(display, err);
    }
    if report.gated {
        return park_hint_busy(display);
    }
    park_hint_queued(display)
}

/// 보관함 가득 실패 행 hint(회수 시도 없음 — ADR-0114 결정 1).
fn mailbox_full_hint(display: &str) -> String {
    format!(
        "'{display}' mailbox is full (100 parked messages) — nothing was queued for it; its oldest messages expire by TTL, so retry later. Other recipients were unaffected."
    )
}

/// ★한 발송이 모은 은퇴 사실(R2)★ — **일어난 것**과 **계획했지만 안 일어난 것**을 분리해 나른다.
///
/// 둘을 한 통에 담으면 계측이 오염된다(`Reservation::commit` 주석). 락 밖에서 각각 다른 급으로 찍는다.
#[derive(Default)]
struct RetirementLog {
    real: Vec<RetiredContract>,
    phantom: Vec<RetiredContract>,
}

/// ★은퇴 계측의 테스트 관측면(F5)★ — `log_contract_retirements` 가 실제로 보고한 `(계약 msg_id, 수신자)`.
///
/// 로그 캡처 하네스(tracing subscriber) 없이 **호출이 일어났나**만 본다. 관측 대상은 로그 문구가 아니라
/// **그 호출이 결말 루프 뒤에 남아 있나**다(M1 회귀축).
#[cfg(any(test, feature = "test-harness"))]
pub(crate) mod retirement_reports {
    use std::cell::RefCell;

    thread_local! {
        static REPORTS: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
    }

    pub(crate) fn push(request_id: &str, recipient: &str) {
        REPORTS.with(|r| {
            r.borrow_mut()
                .push((request_id.to_string(), recipient.to_string()))
        });
    }

    /// 지금까지의 보고를 **꺼내 비운다**(구간별 단언 — 픽스처가 남긴 잉여가 섞이지 않게).
    ///
    /// `test-harness` 피처만 켠 lib 빌드(테스트 타깃 아님)에서는 호출자가 없어 dead_code 경고가 난다 —
    /// 관측면은 테스트 전용이라 그게 정상이다.
    #[allow(dead_code)]
    pub(crate) fn drain() -> Vec<(String, String)> {
        REPORTS.with(|r| std::mem::take(&mut *r.borrow_mut()))
    }
}

/// 상한 압력으로 실제 은퇴한 계약을 **락 밖에서** 계측한다(커밋 전에 찍으면 일어나지 않은 일을 보고한다).
///
/// ★호출 위치는 **결말 루프 뒤**다(M1)★: 커밋이 pass B 안으로 내려갔으므로(A2) 루프 전에 찍으면 주 경로
///   (idle 수신자 request)의 은퇴가 로그를 하나도 남기지 않는다 — ADR-0108 결정 2 에서 이 info 로그가 은퇴의
///   **유일한** 증거다.
/// ★M2 — "계약을 연 자리에서 곧바로 커밋한다" 로 되돌리지 말 것★: 같은 pass A 의 cap 게이트가 그 뒤에
///   이 수신자를 실패 행으로 떨굴 수 있고, 그 갈래가 실제로 롤백을 필요로 한다(7차에 창이 좁아졌을 뿐 —
///   `Reservation` 헤더가 정본). 되돌리면 A2/A3 회귀다.
fn log_contract_retirements(new_msg_id: &str, retired: &[RetiredContract]) {
    for r in retired {
        // ★thread-local 인 이유(F5)★: 이 함수는 서비스 핸들이 없는 자유 함수이고(시그니처를 관측을 위해
        //   바꾸지 않는다), 발송 경로는 호출 스레드에서 끝까지 돌아 관측이 스레드 경계를 넘지 않는다.
        #[cfg(any(test, feature = "test-harness"))]
        retirement_reports::push(&r.request_id, &r.recipient);
        // 필드 전용 계측 — 본문·토큰은 싣지 않는다. 은퇴 가능 조건상 발신자에게 진 통지 빚은 없다
        //   (통지 완료분 또는 기한 없는 계약뿐 — ledger `open_request` 주석).
        tracing::info!(
            retired_msg_id = %r.request_id,
            from = %r.sender,
            to = %r.recipient,
            age_secs = r.age.as_secs(),
            new_msg_id = %new_msg_id,
            "미회신 계약 상한 압력 — 가장 오래된 은퇴 가능 계약을 내보내고 새 request 수용(ADR-0108)"
        );
    }
}

struct ParkRequest<'a> {
    msg_id: &'a str,
    sender_name: &'a str,
    from: SenderIdentity,
    entrance: Entrance,
    recipient: &'a str,
    body: &'a str,
    hinted_id: Option<PeerId>,
    kind: ParkKind,
    meta: &'a SendMeta,
    /// ★이 논리 메시지가 남길 배달기록 총수(round-2 리뷰 F3)★ — notice = 1, 발송 = **실패 행 포함** 수신자 수.
    /// 파킹도 이력 행 하나를 남기므로(`Pending`) 그 행에 같은 기대 수를 박아야 조회의 잘림 판정이 성립한다.
    expected_rows: u16,
}

/// ★회신 계약 닫기를 **수용 기록과 같은 임계구역에서** 끝내는 장치(리뷰 fix D2 — load-bearing)★.
///
/// ★막는 것 = "수용됐는데 계약은 `reply_failed`" 라는 되돌림★(재가된 규칙 "수용 = 완료, 되돌림 없음" 위반):
///   예전 배선은 ① 락 A 에서 회신을 파킹(수용 확정) → 락 해제 → ② 락 B 에서 계약을 `replied` 로 닫았다.
///   두 락 사이에 **삭제 정리**(`handle_profile_deleted`)가 끼면 그 계약을 요청자 삭제로 `reply_failed` 로
///   닫아 버리고, 뒤늦은 ②는 `AlreadyClosed` 로 조용히 물러난다 — 결과는 "회신은 `pending` 으로 접수됐는데
///   계약은 회신 실패" 다. 파킹 수용 경로에는 두 락을 갈라야 할 이유가 **하나도 없다**(파킹은 inject 를
///   부르지 않는다 = 락 안에서 못 할 일이 없다). 그래서 그 자리에서 닫는다.
/// ★7차에는 수용 갈래가 **적재 하나**다(ADR-0125)★: 발송이 예외 없이 큐에 적재되므로 수용 확정 지점도
///   적재 락 하나이고, 그 안에서 닫으면 원자성이 **전 갈래**에서 성립한다(옛 즉시 배달 갈래의 두 번째 확정
///   지점은 직발송 폐지와 함께 사라졌다). 뒤이은 드레인이 배달로 승격시키는 것은 **배달 축**이라 계약 축의
///   원자성과 무관하다. (대안이었던 "mid-flight 마커로 정리를 막기" 는 새 상태를 도입하고 누출 시 계약이
///   영영 안 닫히는 위험이 있어 채택하지 않았다 — 여기 방식은 새 상태가 0이다.)
/// ★멱등★: `done` 이 채워지면 다시 닫지 않는다(다중 수신자 회신에서 첫 수용이 계약을 닫고 나머지는 무동작 —
///   "하나라도 수용되면 수용" 규칙과 같은 방향, `reply_disposition`).
/// ★단서 — 멱등이 **결말 종류를 가리지 않는다**(현행 무해 · 미래 함정, 리뷰 fix N-nit)★: `NoMatch`(내 계약이
///   아님)도 "닫았다" 로 기록돼 뒤 행이 다시 시도하지 않는다. 지금은 무해하다 — 입구가 회신 발송을 **수신자
///   정확히 1명**으로 강제하므로(`control/ingress.rs` 규칙 2 · spec §3 항목 7-①) 행이 애초에 하나다. 다만
///   `handle_send` 는 pub 이라 그 강제를 안 타는 호출자가 다중 행 회신을 넣으면 **앞 행의 `NoMatch` 가 뒤 행의
///   진짜 매치를 가린다**. 다중 행 회신을 허용하려면 이 조건을 성공 결말(`Closed`/`ClosedHistoryAnomaly`)로
///   좁혀야 한다 — 그때 `NoMatch` 는 "아직 안 닫음" 으로 남겨야 한다.
/// ★로깅은 락 밖★: 결과만 들고 나와 호출자가 `log_reply_close` 로 찍는다(락 보유 중 tracing 금지).
// ADR-0116 (결정 2) / ADR-0118 (결정 1·2) / 리뷰 fix D2
struct ReplyClosing<'a> {
    /// `Some` = 이 발송이 회신이다(그 계약 id). `None` 이면 이 장치는 통째로 no-op.
    in_reply_to: Option<&'a str>,
    replier_name: &'a str,
    replier_id: PeerId,
    /// 이름 폴백 허용 여부(리뷰 fix D4 — 동명 산 세션 2개 이상이면 `false`).
    allow_name_fallback: bool,
    /// 이미 닫았으면 그 결과(락 밖 로깅용 + 재닫기 차단).
    done: Option<ReplyOutcome>,
}

impl ReplyClosing<'_> {
    fn close_in_lock(&mut self, st: &mut MessagingState) {
        let Some(in_reply_to) = self.in_reply_to else {
            return;
        };
        if self.done.is_some() {
            return;
        }
        self.done = Some(st.ledger.close_on_reply(
            in_reply_to,
            self.replier_name,
            self.replier_id,
            self.allow_name_fallback,
            Instant::now(),
        ));
    }
}

/// ★회신 계약 닫기 결과 로깅(**락 밖에서만** — 모듈 헤더 규율)★ — 결과에 따라 로깅만 갈린다(배달·응답에는
/// 영향 없음, spec §3 항목 7-②).
///
/// - `Closed` = 정상(그 회신자의 첫 유효 회신). 다른 수신자의 계약은 그대로 열려 있다(전체회신 없음).
/// - `ClosedHistoryAnomaly` = 계약은 닫혔으나 이력이 `Delivered → Replied` 간선을 못 탐(예: 회신이 원본보다
///   먼저 처리돼 원본이 아직 `pending`). 계약 정본은 추적이라 그대로 두고 **관측**만 한다.
/// - `NoMatch`/`AlreadyClosed` = 틀린 id·내 계약이 아님·이미 닫힘. **정상 경로**다(회신 메시지 자체는 이미
///   배달/파킹됐다) — 발신자에게 새 에러를 만들지 않는다: 고칠 수 없는 상태이고, 반려하면 이미 배달된
///   메시지에 재시도를 유발해 중복이 난다.
fn log_reply_close(outcome: ReplyOutcome, in_reply_to: &str, replier_name: &str) {
    match outcome {
        ReplyOutcome::Closed => {}
        ReplyOutcome::ClosedHistoryAnomaly { from } => tracing::warn!(
            in_reply_to,
            history_status = ?from,
            "회신으로 계약은 닫혔으나 이력이 Replied 로 전이 못 함(불법 간선) — 계약 정본은 추적, 이력만 미반영(ADR-0103)"
        ),
        ReplyOutcome::NoMatch => tracing::debug!(
            in_reply_to,
            replier = %replier_name,
            "reply_to 가 이 회신자의 오픈된 request 를 가리키지 않음 — 메시지는 정상 배달, 닫힌 계약 없음(spec §3 항목 7-②. 동명 산 세션 다수면 이름 폴백이 금지돼 여기로 온다 — 리뷰 fix D4)"
        ),
        ReplyOutcome::AlreadyClosed => tracing::debug!(
            in_reply_to,
            replier = %replier_name,
            "이미 닫힌 계약에 대한 추가 회신 — 메시지는 정상 배달, no-op(spec §3 항목 7-③)"
        ),
    }
}

/// ★장부에 **못 남긴** 종점 전이 1건(C4 리뷰 fix J · round-5 finding 2)★ — 레코드가 이력 링에서 이미
///   밀려나(`TransitionError::NotFound`) 전이가 불가능했던 사실을, **찍으려던 상태와 함께** 나른다.
///
/// ★왜 `intended` 를 나르나(finding 2 — load-bearing)★: 이 수집함은 처음엔 만료(`expired`) 전용이었는데
///   이후 `skipped` 전이(notice 레인 은퇴)까지 같은 함에 담기게 됐다. 상태를 안 나르면 로그가
///   전부 "expired" 로 뭉개져 — 레코드가 사라진 마당에 **유일하게 남는 감사 증거**가 거짓 원인을 말한다.
///   그래서 전이에 쓴 상태를 그대로 실어 보낸다(전이와 로그가 **같은 값** 하나를 쓴다 — 갈릴 여지 없음).
#[derive(Debug, Clone, PartialEq, Eq)]
struct EvictedTransition {
    msg_id: String,
    recipient: String,
    intended: DeliveryStatus,
}

/// 종점 전이 1회 + evict 사실 수집(**단일 지점** — 의도 상태가 전이와 로그에서 갈릴 수 없다).
///
/// `Illegal` 은 best-effort 로 무시한다(장부 그래프 위반은 상위 버그지만 배달을 막을 이유가 없다).
/// `NotFound` 만 수집해 호출자가 **락 밖에서** 로깅한다(락 보유 중 tracing 금지 — 모듈 헤더 규율).
fn transition_or_collect_evicted(
    ledger: &mut Ledger,
    msg_id: &str,
    recipient: &str,
    intended: DeliveryStatus,
    now: Instant,
    evicted: &mut Vec<EvictedTransition>,
) {
    if let Err(TransitionError::NotFound) = ledger.transition(msg_id, recipient, intended, now) {
        evicted.push(EvictedTransition {
            msg_id: msg_id.to_string(),
            recipient: recipient.to_string(),
            intended,
        });
    }
}

/// 수집된 evict 사실을 찍는다 — **락 밖에서만** 부른다(모듈 헤더 규율).
fn log_evicted_transitions(evicted: &[EvictedTransition]) {
    for e in evicted {
        tracing::debug!(
            msg_id = %e.msg_id,
            recipient = %e.recipient,
            intended_status = ?e.intended,
            "record evicted before terminal transition — 종점(만료/회수)은 실제로 일어났으나 이력 링버퍼에서 이미 밀려나 장부에 남길 레코드가 없음(ledger HISTORY_CAPACITY)"
        );
    }
}

/// ★in-flight 영수증의 단일 출구 정산 가드(F1 · load-bearing)★ — `Mailbox::take_in_flight` 로 올려 둔 cap
///   분모를 **어떤 경로로 flush 를 빠져나가든** 반드시 내린다.
///
/// ★왜 스택 가드인가★: 배달 루프는 타깃마다 early break 가 있고 중간에 외부 호출(inject)이 섞여 있다.
///   정산을 갈래마다 흩뿌리면 한 곳만 놓쳐도 그 수신자 레인의 분모가 **영구히** 부풀어(누수) 그 이름 앞
///   메일이 영영 안 들어간다 — 조용한 유실보다 고치기 어려운 실패 모드다. 그래서 "남은 건 Drop 이 갚는다"
///   로 뒤집어, 갈래별 정산은 **최적화**(분모를 실제 미결 건수에 맞추기)일 뿐 정확성 조건이 아니게 만든다.
/// ★Drop 에서 락 실패를 삼키는 이유★: 언와인딩 중일 수 있고, 그때 messaging 락은 이미 poisoned 다. 거기서
///   다시 패닉하면 이중 패닉으로 프로세스가 죽는다 — 그리고 락이 poisoned 라는 건 이 서비스가 이미 죽었다는
///   뜻이라(모든 경로가 `expect` 로 패닉한다) 분모를 되돌릴 실익도 없다.
/// ★정산 누락은 없지만 **일시 과다 계수**는 있다(명시 계약)★: 배달 완료·복원 갈래는 그 자리에서 떼어 내지만,
///   그 사이 구간에서는 나가 있는 건수만큼 분모가 높다 — 그게 F1 이 막으려는 창 그 자체라 의도된 값이다.
struct FlightSettle<'a> {
    svc: &'a MessagingService,
    recipient: &'a str,
    owed: FlightTicket,
}

impl Drop for FlightSettle<'_> {
    fn drop(&mut self) {
        if self.owed.is_zero() {
            return;
        }
        if let Ok(mut st) = self.svc.state.lock() {
            st.mailbox.settle_in_flight(self.recipient, self.owed);
        }
    }
}

/// ★park 1건이 남긴 **락 밖에서 처리할 사실**(round-6)★ — 압력 회수는 락 안에서 일어나지만 로깅은 락
///   밖이어야 한다(모듈 헤더 규율). 그래서 축적자 형태로 호출자가 들고 다니다가, 락을 놓은 뒤 `log` 를 부른다.
///
/// ★갈래를 **따로 센다**★: 회수 사유가 다르기 때문이다(notice = 더 최신 통지에 밀림 / TTL 초과 = 시계가
///   먼저 운명을 정함). 한 필드로 합치면 로그가 사유를 뭉개 운영 중 오진을 부르고, 특히 TTL 갈래는
///   **장부 어휘 자체가 다르다**(`expired` — F3).
#[derive(Debug, Default)]
struct ParkSideEffects {
    retired_notices: Vec<String>,
    retired_expired: Vec<String>,
    evicted: Vec<EvictedTransition>,
}

impl ParkSideEffects {
    fn log(&self, recipient: &str) {
        if !self.retired_expired.is_empty() {
            tracing::debug!(
                recipient,
                retired = self.retired_expired.len(),
                msg_ids = ?self.retired_expired,
                "park retire: 회수 시점에 이미 TTL(mailbox PARK_TTL) 초과 — sweep(60s 주기)이 아직 안 걷었을 뿐이라 장부 어휘는 skipped 가 아니라 expired(spec §5 · F3)"
            );
        }
        if !self.retired_notices.is_empty() {
            tracing::debug!(
                recipient,
                retired = self.retired_notices.len(),
                msg_ids = ?self.retired_notices,
                "park retire: notice 레인 상한(mailbox NOTICE_CAP) 초과 — 가장 오래된 통지를 회수하고 장부 skipped(신규 통지는 항상 수용 — round-6)"
            );
        }
        log_evicted_transitions(&self.evicted);
    }
}

/// `ParkRequest` → 저장 항목 1건. 조립을 한 곳에 모아 두면 봉투 payload·id 힌트·TTL 기준시각이 삽입
/// 경로마다 갈리지 않는다(옛 좌석 복원 경로가 사라진 뒤로 호출자는 `park_into` 하나다 — ADR-0125).
fn build_parked(
    req: &ParkRequest<'_>,
    payload: ParkPayload,
    now: Instant,
    admission_seq: u64,
) -> ParkedMessage {
    ParkedMessage {
        msg_id: req.msg_id.to_string(),
        envelope: payload.encode(),
        kind: req.kind,
        parked_at: now,
        admission_seq,
        hinted_id: req.hinted_id,
    }
}

/// ★파킹 + 장부 `pending` 기록의 **락 안 알맹이**(단일 발송·그룹 fan-out 공용)★.
///
/// ★왜 자유 함수로 뽑았나★: 그룹 fan-out 은 멤버 N명의 판정·파킹을 **한 락 구간**에서 끝내야 하는데
///   (`handle_send` 락 규율), 자기가 락을 잡는 래퍼(`park_notice`)를 그대로 부르면 수신자마다 락을
///   잡았다 놓아 "큐가 비어 보이는 창" 이 열린다. 그래서 락을 **호출자가 쥔 채** 부를 수 있는 알맹이를
///   분리하고, notice 용 `park_notice` 는 이걸 감싸는 얇은 래퍼로 남긴다(파킹 규칙 한 벌 유지 —
///   조용한 유실 금지·payload 인코딩·장부 기록이 두 경로에서 갈리면 안 된다).
/// ★조용한 유실 금지(ADR-0103)★: park 성공 시 반드시 장부에 `pending` 을 남긴다. cap 초과 반려는 저장
///   자체를 안 하므로 장부도 안 남긴다(호출자가 반려/skip 으로 가시화한다).
/// ★압력 회수의 장부화(round-6 · F3 어휘 보정)★: 이 park 이 자리를 만드느라 걷어낸 항목
///   (`ParkAdmitted.retired` — message 레인의 배달 불가 잔해 또는 notice 레인의 옛 통지)은 **여기서** 장부
///   종점으로 전이한다 — 저장소는 순수라 장부를 모르고, 그 사실을 아는 유일한 지점이 이 경계다. 걷어내고
///   장부를 안 고치면 유령 pending 이 남는다(큐엔 없는데 조회하면 "배달 대기 중"). 사유별 로깅 거리는
///   `effects` 에 나눠 모아 **락 밖**에서 찍는다(모듈 헤더 규율).
/// ★어휘는 항목마다 갈린다 — TTL 이 `skipped` 보다 우선(F3)★: 기본은 `skipped`("그 수신자에게는 배달하지
///   않음")지만, sweep 주기(60s)와 TTL(24h) 사이의 틈 때문에 **이미 TTL 을 넘겼는데 아직 안 걷힌** 항목이
///   회수될 수 있다. 그건 spec §5 계약상 `expired` 다(시계가 먼저 그 항목의 운명을 정했고, 회수는 그저 그
///   사실을 늦게 발견한 것이다). 판정은 mailbox 의 `ParkedMessage::is_expired(now)` 를 그대로 재사용한다 —
///   TTL 상수도 `>=` 경계 규약도 저장소 한 곳에만 두려고 리터럴을 복제하지 않는다.
// ADR-0107 (회수분 장부 종점 — skipped/expired 분류)
fn park_into(
    st: &mut MessagingState,
    req: ParkRequest<'_>,
    now: Instant,
    effects: &mut ParkSideEffects,
) -> Result<(), ParkError> {
    let payload = ParkPayload {
        sender_name: req.sender_name.to_string(),
        from: req.from,
        entrance: req.entrance,
        body: req.body.to_string(),
        meta: req.meta.clone(),
    };
    // admission 순번은 `park` 이 수용 시점에 부여한다(저장소가 유일 부여자 — mailbox 주석). 여기 값은
    //   무시되므로 placeholder.
    let parked = build_parked(&req, payload, now, 0);
    let admitted = st.mailbox.park(req.recipient, parked)?;
    for m in admitted.retired {
        let expired = m.is_expired(now);
        let intended = if expired {
            DeliveryStatus::Expired
        } else {
            DeliveryStatus::Skipped
        };
        transition_or_collect_evicted(
            &mut st.ledger,
            &m.msg_id,
            req.recipient,
            intended,
            now,
            &mut effects.evicted,
        );
        match (expired, m.kind) {
            (true, _) => effects.retired_expired.push(m.msg_id),
            // ADR-0114: message 레인의 압력 회수는 폐지됐다 — 이 채널로 오는 건 notice 은퇴뿐이다.
            //   그래도 갈래를 남겨 두는 이유는 저장소가 언젠가 다른 회수 사유를 낼 때 조용히 뭉개지지 않게.
            (false, ParkKind::Message) => effects.retired_notices.push(m.msg_id),
            (false, ParkKind::Notice) => effects.retired_notices.push(m.msg_id),
        }
    }
    st.ledger.record_with_expected(
        req.msg_id,
        req.sender_name,
        req.recipient,
        req.body,
        DeliveryStatus::Pending,
        now,
        req.expected_rows,
    );
    Ok(())
}

/// ★인코딩(C3 리뷰 fix 4 에서 버전 헤더 도입)★:
///   `<ver>\n<sender_len>\n<reply_by_len>\n<reply_to_len>\n<group_len>\n<from_agent_id>\n<from_epoch>\n`
///   `<entrance>\n<flags>\n<sender><reply_by><reply_to><group><body>`
///   앞 9줄은 개행 없는 필드(숫자/uuid/짧은 리터럴)라 개행으로 안전 분리하고, 가변 문자열 앞 4개는 **길이
///   접두**로 경계를 잡는다(body 는 나머지 전부 — body·reply_to 에 개행이 들어와도 안전하고, reply_to 는
///   에이전트 입력이라 임의 문자열일 수 있다). 길이 0 = `None`(빈 문자열 `Some("")` 은 입구 검증이 이미
///   반려하므로 모호하지 않다).
///
/// ★왜 버전 태그인가(fix 4)★: 이 형식은 **프로세스 내부**에서만 쓰이지만(파킹은 인메모리라 데몬 재시작이면
///   소멸), 형식이 바뀌는 순간 옛 payload 가 새 decode 를 만나면 조용히 오해석될 수 있다(길이 필드가 다른
///   자리로 밀려 body 가 잘리는 식). 1글자 태그가 있으면 **모르는 버전 = 즉시 폴백**으로 갈라져, 오해석
///   대신 "봉투 속성 잃고 body 만 남음"(보이는 열화)으로 실패한다. v2 영속화가 파킹을 디스크에 남기면
///   이 태그가 마이그레이션 분기점이 된다.
/// ★레이아웃을 바꾸면 이 값도 올린다★: 같은 프로세스 안에서만 쓰는 형식이라 옛 payload 가 실재할 수는
///   없지만, 태그를 그대로 두면 **레이아웃이 바뀌었는데 버전은 같은** 상태가 되어 태그의 의미(= 이 문자열의
///   레이아웃 계약)가 무너진다.
const PARK_PAYLOAD_VERSION: &str = "3";

/// 파킹 payload — flush 주입 시점에 봉투 조립·관측 레코드 발행에 필요한 최소 메타(sender 이름·발신자
///   신원·입구·raw body). ParkedMessage.envelope 계약("완성 봉투")을 우회해 raw 를 나르기 위한 내부 인코딩.
///   flush 주입 시점에 decode → wrap_now 로 **현재** 포맷 봉투를 조립하고(단일 wrap point), 발신자 신원으로
///   배달 관측 레코드를 발행한다(등장 배달도 발송 경로와 동일하게 관측 — ADR-0088).
///
/// ★왜 from/entrance 도 나르나★: 파킹→flush 자동 배달도 배달 경계 관측(ADR-0088)의 대상이다 — 하네스가
///   "누가 누구에게 배달됐나" 를 flush 경로에서도 회수해야 한다(파킹→스폰→자동배달 acceptance, spec §7).
///   그러려면 원래 발신자 신원(SenderIdentity)과 입구를 flush 시점까지 보존해야 한다.
/// ★왜 meta(회신 계약)도 나르나(C3 · load-bearing)★: 파킹된 request/회신이 늦게 배달될 때도 **즉시 배달과
///   동일한 봉투 속성**(id/type/reply-by/in-reply-to)이 붙어야 한다. 안 나르면 파킹을 거친 request 는
///   속성 없는 plain 메시지로 도착해 수신 LLM 이 회신할 id 를 모르고, 발신자만 기한 초과 notice 를 받는다
///   (계약이 조용히 깨지는 최악 모드). 봉투를 park 시점에 굳히지 않는 설계의 대가로 재료를 나른다.
struct ParkPayload {
    sender_name: String,
    from: SenderIdentity,
    entrance: Entrance,
    body: String,
    /// C3 회신 계약 메타 + C4 그룹 라벨(파싱된 Duration 은 **복원하지 않는다** — 계약은 발송 시점에 이미
    ///   장부에 열렸고, flush 가 필요한 건 봉투 속성뿐이다). 그래서 `reply_by` 는 raw 표기만 살아남는다.
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
        // 봉투 `to`(동결값) — 파킹분이 늦게 배달돼도 발송 순간 표기를 잃지 않게 함께 나른다(spec §1).
        let group = self.meta.to_attr.as_deref().unwrap_or("");
        let flags = if self.meta.request { "r" } else { "-" };
        format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}{}{}{}{}",
            PARK_PAYLOAD_VERSION,
            self.sender_name.len(),
            reply_by.len(),
            reply_to.len(),
            group.len(),
            self.from.peer_id,
            self.from.epoch,
            ent,
            flags,
            self.sender_name,
            reply_by,
            reply_to,
            group,
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
        // splitn(10) — 마지막 조각을 자르지 않아 body 안 개행이 보존된다.
        let mut it = s.splitn(10, '\n');
        let fallback = || Self {
            sender_name: String::new(),
            from: SenderIdentity {
                peer_id: PeerId::nil(),
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
            Some(grp_len),
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
            it.next(),
        )
        else {
            return fallback();
        };
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
        let (Ok(sender_len), Ok(rb_len), Ok(rt_len), Ok(grp_len), Ok(agent_id), Ok(epoch)) = (
            sender_len.parse::<usize>(),
            rb_len.parse::<usize>(),
            rt_len.parse::<usize>(),
            grp_len.parse::<usize>(),
            id_str.parse::<PeerId>(),
            ep_str.parse::<u32>(),
        ) else {
            return fallback();
        };
        let Some(total) = sender_len
            .checked_add(rb_len)
            .and_then(|v| v.checked_add(rt_len))
            .and_then(|v| v.checked_add(grp_len))
        else {
            return fallback();
        };
        if rest.len() < total {
            return fallback();
        }
        let cut1 = sender_len;
        let cut2 = cut1 + rb_len;
        let cut3 = cut2 + rt_len;
        let cut4 = cut3 + grp_len;
        if !rest.is_char_boundary(cut1)
            || !rest.is_char_boundary(cut2)
            || !rest.is_char_boundary(cut3)
            || !rest.is_char_boundary(cut4)
        {
            return fallback();
        }
        let (sender, tail) = rest.split_at(cut1);
        let (reply_by, tail) = tail.split_at(rb_len);
        let (reply_to, tail) = tail.split_at(rt_len);
        let (group, body) = tail.split_at(grp_len);
        Self {
            sender_name: sender.to_string(),
            from: SenderIdentity {
                peer_id: agent_id,
                epoch,
            },
            entrance,
            body: body.to_string(),
            meta: SendMeta {
                request,
                reply_by_raw: (!reply_by.is_empty()).then(|| reply_by.to_string()),
                reply_by: None,
                reply_to: (!reply_to.is_empty()).then(|| reply_to.to_string()),
                to_attr: (!group.is_empty()).then(|| group.to_string()),
            },
        }
    }
}

/// `encode` 의 리터럴과 한 쌍이라 입구 종류가 늘면 둘을 함께 고친다(어긋나면 정상 payload 가 폴백돼
///   round-trip 테스트가 잡는다).
fn parse_entrance(s: &str) -> Option<Entrance> {
    match s {
        "mcp" => Some(Entrance::Mcp),
        "cli" => Some(Entrance::Cli),
        "daemon" => Some(Entrance::Daemon),
        _ => None,
    }
}

fn parse_request_flag(s: &str) -> Option<bool> {
    match s {
        "r" => Some(true),
        "-" => Some(false),
        _ => None,
    }
}

/// ★데몬 자가 발신의 신원(C3)★ — `<notice>` 에는 발신 에이전트가 없다. 관측 레코드(`DeliveryObservation.from`)
///   가 신원을 요구하므로 **nil PeerId** 를 데몬 출처 표식으로 쓴다(어떤 실제 에이전트와도 겹치지 않는다).
///   `Entrance::Daemon` 과 짝을 이뤄 "이건 인프라 통지" 를 레코드만으로 판별하게 한다.
fn daemon_identity() -> SenderIdentity {
    SenderIdentity {
        peer_id: PeerId::nil(),
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

/// 이름 → **산 수신자**(LiveAgent). 4차 이후 "도달 가능" 은 로스터 자격이 아니다(턴 신호 없는 산 세션도
///   로스터에 있다 — ADR-0116 결정 7. 이름만 남은 함수명은 유지: 호출부가 많다).
///
/// ★유일 매치만 Some — 0개(부재)·2개+(동명 다수)는 None★: 그 판정(`RECIPIENT_AMBIGUOUS`/파킹)은 상위가
///   한다(파킹 전에). 상위가 로스터를 다시 보지 않도록 여기 로직을 최소로 둔다.
/// ★스냅샷을 인자로 받는 이유★: flush 배치는 이름 판정과 id-힌트 생존 판정을 **같은 스냅샷**으로 해야
///   배치 도중 판정이 흔들리지 않는다(로스터 재조회 금지).
fn unique_reachable_in(roster: &[LiveAgent], name: &str) -> Option<LiveAgent> {
    let mut matches = roster.iter().filter(|a| a.name == name);
    let first = matches.next()?;
    match matches.next() {
        Some(_) => None,
        None => Some(first.clone()),
    }
}

fn resolve_live(to: &str, roster: &[LiveAgent]) -> Option<LiveAgent> {
    // F2: PeerId 문자열 정확 일치 우선(이름=UUID 충돌이 ID 지목을 가로채지 못하게 — ingress 미러).
    if let Some(a) = roster.iter().find(|a| a.id.to_string() == to) {
        return Some(a.clone());
    }
    let by_name: Vec<&LiveAgent> = roster.iter().filter(|a| a.name == to).collect();
    if by_name.len() == 1 {
        return Some(by_name[0].clone());
    }
    None
}

// ★제거됨(C4 — ADR-0104 앵커 해소)★: 옛 `transition_expired_by_msg_id` 는 sweep 만료분을 msg_id 만으로
//   장부에서 역조회해 "첫 pending 레코드" 를 expired 로 전이했다. 그 선택은 **단일 수신자에서만** 옳다 —
//   그룹 방송은 한 msg_id 에 수신자별 레코드가 N개라 엉뚱한 멤버를 만료시킬 수 있고, 무엇보다 이력 삽입
//   순서와 만료 순서의 **우연한 상관**에 정확성을 기대는 구조였다. 이제 mailbox 가 만료 항목마다 큐 키를
//   함께 돌려주므로(`ExpiredParked`) `sweep` 이 `(msg_id, recipient)` 로 직접 지목한다. 이 헬퍼를 되살리지
//   말 것 — msg_id 단독 역조회는 1:N 장부와 구조적으로 양립하지 않는다.
// ADR-0104

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;

    /// 헤드리스 테스트용 ControlPlanePort — 운영 데몬(`ControlRegistry` 어댑터)의 기본 동작을 모사한다:
    ///   봉투 포맷은 기본값 Xml(ADR-0103), 배달 관측은 no-op(운영 데몬도 observer 미설치 = no-op).
    ///   관측 레코드를 단언하는 검증은 데몬 통합 하네스가 한다(ADR-0088 — 여기 단위 테스트 범위 밖).
    struct FakeControlPlane;
    impl ControlPlanePort for FakeControlPlane {
        fn envelope_format(&self) -> EnvelopeFormat {
            EnvelopeFormat::default()
        }
        fn record_delivery(&self, _obs: DeliveryObservation) {}
    }

    /// 헤드리스 테스트용 DeliveryPort — 로스터·주입 성공/실패를 스크립트한다(claude/PTY 없음).
    struct FakeDeliveryPort {
        /// 살아있는 도달 가능 로스터(테스트가 세팅). ★Arc 인 이유★: 테스트가 핸들을 복제해 **inject 도중**
        ///   (on_inject hook 안에서) 로스터를 갈아끼울 수 있어야 한다 — "로스터 스냅샷 이후·write 이전에
        ///   수신자가 재시작했다" 는 TOCTOU 를 결정적으로 재현하는 유일한 방법이다(port 자체를 hook 에
        ///   캡처하면 자기참조 순환이 된다).
        roster: Arc<StdMutex<Vec<LiveAgent>>>,
        /// 주입 시도 로그(순서대로 (to_id, 바이트)). 오래된 순 주입·개별 봉투 단언용.
        injected: StdMutex<Vec<(PeerId, Vec<u8>)>>,
        /// inject 호출 횟수(0-based) — fail_at_call 인덱스 매칭용.
        call_count: StdMutex<usize>,
        /// 이 호출 인덱스(0-based)들에서 inject 를 실패시킨다(부분 실패·전체 실패 시나리오 스크립트).
        fail_at_call: StdMutex<Vec<usize>>,
        /// inject 호출 시점에 실행할 부수 hook(drain↔inject 사이 동시 park 레이스 재현용). 호출 인덱스를
        ///   받아 임의 조작(예: 재파킹으로 큐 재충전)을 한다 — 이 hook 안에서 서비스 락을 다시 잡지 않는다
        ///   (inject 는 락 밖 호출이므로 안전).
        /// ★Box 가 아니라 Arc 인 이유(round-7)★: hook 안에서 **다시 주입이 일어날 수 있다**(예: 유예된
        ///   flush 가 되울려 그 자리에서 배달). Box 를 락 가드 아래에서 호출하면 그 재진입이 이 뮤텍스에
        ///   걸려 **데드락**(std Mutex 는 비재진입)이라, 복제 가능한 Arc 로 바꿔 락을 놓고 부른다.
        on_inject: StdMutex<Option<Arc<dyn Fn(usize) + Send + Sync>>>,
        /// roster 밖 id 의 canonical_name override(비-도달 수신자 = TUI 시나리오 — finding 4). roster 에
        ///   없어도 이 맵에 있으면 canonical_name 이 그 이름을 돌려준다(로스터엔 안 뜨는 산 에이전트 모사).
        canonical_overrides: StdMutex<std::collections::HashMap<PeerId, String>>,
        /// live_agents 조회 횟수(late-appearance 스크립트용 + sweep 스냅샷 1회 단언 — fix 8).
        roster_calls: StdMutex<usize>,
        /// 세팅되면 **첫 조회 이후**부터 이 roster 를 돌려준다(첫 조회 = resolve 는 원래 roster, 이후 =
        ///   self_heal 이 보는 late-appearance roster). finding 3 TOCTOU self-heal 재현용.
        roster_after_first: StdMutex<Option<Vec<LiveAgent>>>,
        /// ★일회성 roster 조회 hook(fix 5 레이스 재현)★ — 로스터를 뜨는 **그 순간** 다른 일이 벌어진 상황을
        ///   결정적으로 만든다(예: sweep 의 due 산출 뒤·notice 파킹 전에 회신이 도착). 한 번 쓰고 비우므로
        ///   hook 안에서 다시 발송해도 재귀하지 않는다.
        on_roster: StdMutex<Option<Box<dyn Fn() + Send>>>,
        /// ★잠든 이름 목록(프로필 실재 · 세션 없음 — spec §5 분기 3)★. **중복을 접지 않는다**(같은 이름 2개 =
        ///   잠듦 층 동명). 운영 어댑터는 `ProfileRegistry::list()` 에서 산 세션 몫을 뺀 뒤 canonical 규칙으로
        ///   파생하고, 여기선 그 결과만 스크립트한다.
        dormant: StdMutex<Vec<String>>,
        /// ★`is_agent_live` 가 참을 돌려줄 id 들(삭제 정리 게이트 시나리오 — 리뷰 fix D1)★.
        ///
        /// ★로스터와 **따로** 스크립트한다(의도)★: 게이트는 그 좁은 동사로 물으므로, 로스터에서 유도해 버리면
        ///   "게이트가 로스터를 본다" 는 배선을 fake 가 대신 만들어 버려 테스트가 실물 배선을 못 본다.
        live_ids: StdMutex<Vec<PeerId>>,
    }

    impl FakeDeliveryPort {
        fn new() -> Self {
            Self {
                roster: Arc::new(StdMutex::new(Vec::new())),
                injected: StdMutex::new(Vec::new()),
                call_count: StdMutex::new(0),
                fail_at_call: StdMutex::new(Vec::new()),
                on_inject: StdMutex::new(None),
                canonical_overrides: StdMutex::new(std::collections::HashMap::new()),
                roster_calls: StdMutex::new(0),
                roster_after_first: StdMutex::new(None),
                on_roster: StdMutex::new(None),
                dormant: StdMutex::new(Vec::new()),
                live_ids: StdMutex::new(Vec::new()),
            }
        }
        /// 잠든 이름 세팅(중복 허용 — 잠듦 층 동명 시나리오) — spec §5 분기 3.
        fn set_dormant(&self, names: &[&str]) {
            *self.dormant.lock().unwrap() = names.iter().map(|n| n.to_string()).collect();
        }
        /// ★삭제 정리 게이트가 "아직 살아 있다" 로 볼 id 세팅(리뷰 fix D1)★ — 로스터와 독립이다(터미널
        ///   모드로 살아 있는 세션·개명된 프로필처럼 이름으로는 안 잡히는 경우를 그대로 모사한다).
        fn set_live_ids(&self, ids: &[PeerId]) {
            *self.live_ids.lock().unwrap() = ids.to_vec();
        }
        /// 다음 roster 조회 때 **한 번만** 실행할 hook(fix 5 레이스 재현).
        fn arm_on_next_roster(&self, f: Box<dyn Fn() + Send>) {
            *self.on_roster.lock().unwrap() = Some(f);
        }
        /// live_agents 총 호출 수(fix 8 — sweep 이 틱당 1회만 뜨는지 단언).
        fn roster_call_count(&self) -> usize {
            *self.roster_calls.lock().unwrap()
        }
        fn set_roster(&self, roster: Vec<LiveAgent>) {
            *self.roster.lock().unwrap() = roster;
        }
        /// ★`inject_if_epoch` 이 보는 "지금" 로스터★ — armed(late-appearance) 가 있으면 그것, 없으면 기본.
        ///   `live_agents` 와 달리 호출 카운터·hook 을 건드리지 않는다(주입 경계의 판정이지
        ///   로스터 **조회**가 아니다 — 운영 구현도 세션 맵을 직접 보지 로스터를 다시 뜨지 않는다).
        fn effective_roster(&self) -> Vec<LiveAgent> {
            match self.roster_after_first.lock().unwrap().as_ref() {
                Some(r) => r.clone(),
                None => self.roster.lock().unwrap().clone(),
            }
        }
        /// 주어진 inject 호출 인덱스(0-based)들에서 Err 를 낸다(그 외는 성공).
        fn fail_at(&self, indices: &[usize]) {
            *self.fail_at_call.lock().unwrap() = indices.to_vec();
        }
        /// inject 호출마다(성공/실패 결정 전) 부를 hook 설치 — 동시 park 레이스 재현용.
        fn set_on_inject(&self, f: Arc<dyn Fn(usize) + Send + Sync>) {
            *self.on_inject.lock().unwrap() = Some(f);
        }
        /// 지금까지의 로스터 조회 횟수 — "발송 1회당 스냅샷 1장"(ADR-0111 결정 2) 단언용.
        fn roster_calls(&self) -> usize {
            *self.roster_calls.lock().unwrap()
        }
        fn injected_bodies(&self) -> Vec<String> {
            self.injected
                .lock()
                .unwrap()
                .iter()
                .map(|(_, b)| String::from_utf8_lossy(b).to_string())
                .collect()
        }
        /// 주입이 **누구에게** 갔는지 — "발신자 stdin 엔 아무것도 쓰이지 않는다" 처럼 대상 축을 봐야 하는
        ///   단언용(본문만 보면 자기발송을 못 잡는다).
        fn injected_targets(&self) -> Vec<PeerId> {
            self.injected
                .lock()
                .unwrap()
                .iter()
                .map(|(id, _)| *id)
                .collect()
        }
    }

    impl FakeDeliveryPort {
        /// 두 주입 동사의 공통 본체 — 호출 인덱스 부여·hook·실패 스크립트·기록을 한 벌로 유지한다.
        ///   `expected_epoch = Some(e)` 면 **운영 구현과 같은 판정**(그 id 의 현재 epoch 과 다르면 쓰지 않고
        ///   Err)을 한다. hook 은 판정 **전에** 부른다 — "스냅샷 이후·write 이전 재시작" 이 그 순서다.
        fn inject_inner(
            &self,
            to_id: PeerId,
            expected_epoch: Option<u32>,
            bytes: &[u8],
        ) -> Result<InjectReceipt, String> {
            let idx = {
                let mut c = self.call_count.lock().unwrap();
                let i = *c;
                *c += 1;
                i
            };
            // 동시 park·재시작 레이스 재현 hook(설치돼 있으면). epoch 판정·fail/success 결정 전에 호출한다.
            //   ★락을 놓고 부른다★ — hook 이 다시 주입을 유발해도 이 뮤텍스에 재진입하지 않게(필드 주석).
            let hook = self.on_inject.lock().unwrap().clone();
            if let Some(f) = hook {
                f(idx);
            }
            if let Some(expected) = expected_epoch {
                let now_epoch = self
                    .effective_roster()
                    .iter()
                    .find(|a| a.id == to_id)
                    .map(|a| a.epoch);
                if now_epoch != Some(expected) {
                    // 부작용 0(기록도 안 남긴다) — 운영 core 계약과 동일.
                    return Err(format!(
                        "epoch mismatch: agent {to_id} is now at epoch {now_epoch:?}, caller required {expected} — nothing was written"
                    ));
                }
            }
            if self.fail_at_call.lock().unwrap().contains(&idx) {
                return Err("fake inject fail".to_string());
            }
            self.injected.lock().unwrap().push((to_id, bytes.to_vec()));
            Ok(InjectReceipt {
                bytes_requested: bytes.len(),
                bytes_written: bytes.len(),
                msg_uuid: uuid::Uuid::new_v4(),
                epoch: expected_epoch.unwrap_or(0),
            })
        }
    }

    impl DeliveryPort for FakeDeliveryPort {
        fn inject(&self, to_id: PeerId, bytes: &[u8]) -> Result<InjectReceipt, String> {
            self.inject_inner(to_id, None, bytes)
        }

        fn live_agents(&self) -> Vec<LiveAgent> {
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
        /// ★입구 판정 소스 한 장(ADR-0116)★ — 운영 어댑터와 **같은 관계**를 모사한다: `roster` 는
        ///   `live_agents()` 와 **같은 값**이고(술어 = 상태만) `dormant_names` 는 별도 소스다.
        ///   ★로스터 조회 카운터를 **함께 올린다**★: "발송 1회당 스냅샷 1장" 회귀 테스트가 세는 축이 곧 이
        ///   호출이다(입구가 `live_agents` 대신 이걸 부르게 됐으므로 카운터가 그쪽으로 옮겨간다).
        fn addressing_sources(&self) -> AddressingSources {
            AddressingSources {
                roster: self.live_agents(),
                dormant_names: self.dormant.lock().unwrap().clone(),
            }
        }

        /// ★삭제 정리 게이트(리뷰 fix D1)★ — 테스트가 세팅한 id 집합만 본다(로스터에서 유도하지 않는다 —
        ///   필드 주석).
        fn is_agent_live(&self, id: PeerId) -> bool {
            self.live_ids.lock().unwrap().contains(&id)
        }
        fn canonical_name(&self, id: PeerId) -> Option<String> {
            // roster 우선 → armed(late-appearance) roster → override(비-도달 산 에이전트) 조회.
            //   ★armed 도 보는 이유★: `arm_roster_after_first_call` 은 "그 사이 등장했다" 를 모사하는데,
            //   canonical_name 이 그걸 못 보면 fake 가 자기모순이다(등장했는데 이름이 없는 에이전트).
            //   실 구현(호스트 ManagerDeliveryPort)은 같은 manager 스냅샷을 보므로 항상 일관된다.
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

    /// ★회신자 신원(A6)★ — 계약은 `(id, 수신자)` 로 지목되고 매칭은 **id 우선**이므로(ledger
    ///   `close_on_reply`), 회신은 **그 수신자 본인의 신원**으로 보내야 자기 계약을 닫는다. 옛 테스트들은
    ///   고정 `ident()` 로 회신해 이름 매치에 기댔는데, 그 관대함이 남의 계약을 닫는 경로였다.
    fn reply_from(id: PeerId) -> SenderIdentity {
        SenderIdentity {
            peer_id: id,
            epoch: 0,
        }
    }

    /// 산 발신자 하나 — 신원과 로스터 항목을 함께 만든다(발신자도 로스터에 있어야 `@all` 제외 규칙을 본다).
    fn live_sender(name: &str) -> (SenderIdentity, LiveAgent) {
        let (id, agent) = live(name);
        (
            SenderIdentity {
                peer_id: id,
                epoch: agent.epoch,
            },
            agent,
        )
    }

    /// ★테스트 전용 단일 수신자 shim(ADR-0111 개편의 회귀 그물 보존)★ — 새 진입점은 다중 수신자
    ///   `handle_send` 하나뿐이지만, 이 파일의 기존 회귀 테스트 대부분은 "수신자 1명" 축(파킹·flush·게이트·
    ///   계약)만 본다. 그 자산을 버리지 않으려고 1행 응답을 옛 결말 어휘로 접어 준다.
    ///
    /// ★shim 이 감추지 않는 것★: 부재·동명·보관함 가득은 이제 **행 실패**라 `Err(SendFail::…)` 로 나온다 —
    ///   즉 "부재면 파킹" 을 기대하던 테스트는 이 shim 을 써도 **그대로 실패한다**(개편이 그물에 잡힌다).
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum SendOutcome {
        Delivered,
        Parked { hint: String },
    }

    /// shim 의 실패 축 — 수신자별 실패 코드 + 발송 단위 반려를 한 값으로 접는다(테스트 편의).
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum SendFail {
        Row(FailCode),
        Reject(SendReject),
    }

    impl MessagingService {
        #[allow(clippy::too_many_arguments)]
        fn handle_single_send(
            &self,
            msg_id: &str,
            from: SenderIdentity,
            sender_name: &str,
            to: &str,
            body: &str,
            entrance: Entrance,
            meta: &SendMeta,
        ) -> Result<SendOutcome, SendFail> {
            let rows = self
                .handle_send(
                    msg_id,
                    from,
                    sender_name,
                    &[to.to_string()],
                    body,
                    entrance,
                    meta,
                )
                .map_err(SendFail::Reject)?;
            assert_eq!(rows.len(), 1, "단일 수신자 발송은 행이 정확히 1개다");
            let row = &rows[0];
            match row.status {
                SendStatus::Delivered => Ok(SendOutcome::Delivered),
                SendStatus::Pending => Ok(SendOutcome::Parked {
                    hint: row.hint.clone().unwrap_or_default(),
                }),
                SendStatus::Failed => Err(SendFail::Row(row.code.expect("실패 행은 code 필수"))),
            }
        }
    }

    impl MessagingService {
        /// ★레거시 파킹 seam(ADR-0111 개편 대응 · 테스트 전용)★ — "그 이름 앞 큐에 메일이 있다" 는 **상태**를
        ///   만든다. 개편 전 테스트들은 그 상태를 **부재 파킹**으로 만들었는데, 부재는 이제 실패 행이라
        ///   (ADR-0111 결정 1) 그 진입로가 사라졌다. 그 테스트들의 **주제는 파킹 진입이 아니라 큐의 거동**
        ///   (flush 순서·게이트·TTL·재파킹·in-flight 회계)이므로, 진입만 대체하고 그 뒤 경로는 전부 실제
        ///   코드가 돌게 한다.
        ///
        /// ★운영 경로를 우선한다★: 수신자가 로스터에 있으면 **그냥 `handle_single_send`** 다(즉 busy 파킹·
        ///   적재-후-드레인이 실제로 검증된다). `RECIPIENT_NOT_FOUND` 로 반려된 경우에만 `park_into` 로
        ///   직행한다 — 장부 `pending` 기록·payload 인코딩은 운영과 **같은 함수**를 쓴다.
        /// ★이 seam 은 "부재는 실패" 계약을 가리지 않는다★: 그 계약의 회귀 그물은 `handle_send` 를 직접
        ///   부르는 신설 테스트들이 맡는다(부재 → 실패 행 + 파킹 레코드 없음).
        #[allow(clippy::too_many_arguments)]
        fn park_absent_for_test(
            &self,
            msg_id: &str,
            from: SenderIdentity,
            sender_name: &str,
            to: &str,
            body: &str,
            entrance: Entrance,
            meta: &SendMeta,
        ) -> Result<SendOutcome, SendFail> {
            // 운영 진입점과 **같은 배선 가드**를 먼저 태운다(우회 경로가 그 계약을 느슨하게 만들지 않게).
            debug_assert!(
                !(meta.request && meta.reply_to.is_some()),
                "ingress가 유일 검증자 — request와 reply_to는 상호배타(spec §6)"
            );
            debug_assert!(
                meta.to_attr.is_none(),
                "봉투 to 는 전 수신자 수용 판정 뒤 handle_send 가 1회 확정한다(spec §1)"
            );
            // ★로스터를 **먼저** 본다(load-bearing)★: 운영 경로를 태운 뒤 실패 행을 보고 되돌리면 그
            //   반려가 남긴 장부 종점 행(`failed`)이 큐 항목과 겹쳐 조회가 2행이 된다. 부재가 확실할 때만
            //   진입을 우회한다.
            let absent = resolve_live(to, &self.port.live_agents()).is_none();
            match absent {
                true => {
                    // id 충돌 검사는 운영과 같은 자리(첫 부작용 지점)에서 — 우회 경로도 부작용 0 반려다.
                    {
                        let st = self.state.lock().expect("messaging state poisoned");
                        if st.ledger.msg_id_in_use(msg_id) {
                            return Err(SendFail::Reject(SendReject::IdCollision));
                        }
                    }
                    // 옛 부재 파킹은 **계약도 열었다**(spec §3 "배달됐든 파킹됐든 접수되면 열린다").
                    //   그 축을 보는 테스트(기한 초과 통지·미결 조회)를 살리려고 여기서도 연다.
                    if meta.request {
                        let mut st = self.state.lock().expect("messaging state poisoned");
                        let reply_by = meta.reply_by.zip(meta.reply_by_raw.clone());
                        // ★은퇴 표시를 반드시 커밋으로 넘긴다★: 안 넘기면 표시된 희생자가 영원히 목록에
                        //   남아(표시된 항목은 슬롯을 안 세므로) 상한이 **절대 도달하지 않는다** — cap 픽스처가
                        //   무한 루프가 된다(실측). 운영 경로(`commit_contract`)와 같은 규율이다.
                        let retired = match st.ledger.open_request(
                            msg_id,
                            sender_name,
                            from.peer_id,
                            to,
                            None,
                            reply_by,
                            Instant::now(),
                        ) {
                            OpenOutcome::OpenedAfterMarking(rc) => Some(rc),
                            OpenOutcome::Full => {
                                return Err(SendFail::Row(FailCode::RequestCapacity))
                            }
                            _ => None,
                        };
                        st.ledger.commit_open(
                            Some((msg_id, to)),
                            retired
                                .as_ref()
                                .map(|r| (r.request_id.as_str(), r.recipient.as_str())),
                        );
                    }
                    let mut effects = ParkSideEffects::default();
                    {
                        let mut st = self.state.lock().expect("messaging state poisoned");
                        park_into(
                            &mut st,
                            ParkRequest {
                                msg_id,
                                sender_name,
                                from,
                                entrance,
                                recipient: to,
                                body,
                                hinted_id: None,
                                kind: ParkKind::Message,
                                meta,
                                expected_rows: 1,
                            },
                            Instant::now(),
                            &mut effects,
                        )
                        .map_err(|_| SendFail::Row(FailCode::MailboxFull))?;
                    }
                    effects.log(to);
                    // 접수된 회신은 계약을 닫는다(운영과 같은 규율 — 매칭 실패는 무동작).
                    if let Some(in_reply_to) = &meta.reply_to {
                        // 레거시 seam 은 동명 판정을 하지 않는다 — 이름 폴백 허용(옛 동작 유지).
                        self.close_reply_contract(in_reply_to, sender_name, from.peer_id, true);
                    }
                    Ok(SendOutcome::Parked {
                        hint: format!("(legacy park seam) queued for '{to}'"),
                    })
                }
                false => {
                    self.handle_single_send(msg_id, from, sender_name, to, body, entrance, meta)
                }
            }
        }
    }

    fn svc() -> (Arc<MessagingService>, Arc<FakeDeliveryPort>) {
        let port = Arc::new(FakeDeliveryPort::new());
        let registry = Arc::new(FakeControlPlane); // 기본 봉투 = xml(ADR-0103).
        let svc = Arc::new(MessagingService::new(port.clone(), registry));
        (svc, port)
    }

    fn ident() -> SenderIdentity {
        SenderIdentity {
            peer_id: PeerId::new_v4(),
            epoch: 0,
        }
    }

    fn live(name: &str) -> (PeerId, LiveAgent) {
        let id = PeerId::new_v4();
        (
            id,
            LiveAgent {
                id,
                name: name.to_string(),
                epoch: 0,
                // 기본 = 턴 신호 있음(구조화 출력 백엔드) — 이 파일의 게이트·파킹 회귀 자산이 보는 부류다.
                //   턴 신호 **없는** 부류(즉시 주입 — ADR-0116 결정 7)는 `live_no_turn_signal` 로 만든다.
                turn_signal: true,
            },
        )
    }

    /// ★턴 신호 없는 산 에이전트(터미널/콘솔 모드 — ADR-0116 결정 7)★ — 로스터에 **있고**(멤버십 조건이
    ///   아니다) 게이트 없이 즉시 주입되는 부류.
    fn live_no_turn_signal(name: &str) -> (PeerId, LiveAgent) {
        let (id, mut a) = live(name);
        a.turn_signal = false;
        (id, a)
    }

    // ── C2 idle 게이트 테스트 하네스 ─────────────────────────────────────────────────
    /// 가짜 idle 게이트 — (id, epoch) 별 busy 를 테스트가 세팅하고, 호출 횟수를 세어 TOCTOU 시나리오
    ///   (첫 확인은 busy, 재확인은 idle)를 스크립트한다. 실 관측/PTY 없이 게이트 분기만 결정적으로 단언.
    struct FakeGate {
        busy: StdMutex<std::collections::HashSet<(PeerId, u32)>>,
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
        fn set_busy(&self, id: PeerId, epoch: u32) {
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
        fn is_busy(&self, id: PeerId, epoch: u32) -> bool {
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
        let registry = Arc::new(FakeControlPlane);
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
        seen: StdMutex<Vec<PeerId>>,
    }
    impl FakeTrigger {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                seen: StdMutex::new(Vec::new()),
            })
        }
        fn seen(&self) -> Vec<PeerId> {
            self.seen.lock().unwrap().clone()
        }
        /// 지금까지 본 도어벨을 **꺼내 비운다**(구간별 단언 — 앞 구간의 잉여 도어벨이 섞이지 않게).
        fn take(&self) -> Vec<PeerId> {
            std::mem::take(&mut *self.seen.lock().unwrap())
        }
    }
    impl FlushTrigger for FakeTrigger {
        fn request_flush(&self, id: PeerId) {
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
        let registry = Arc::new(FakeControlPlane);
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
            .park_absent_for_test(
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
    fn inject_failure_parks_pending() {
        // spec §5 분기 3(주입 실패) → 파킹. 해석은 성공하나 inject Err.
        let (svc, port) = svc();
        let (_id, alice) = live("alice");
        port.set_roster(vec![alice]);
        port.fail_at(&[0]); // 첫(유일) inject 실패.
        let out = svc
            .park_absent_for_test(
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
    fn inject_failure_parks_pending_without_a_turn_signal() {
        // ★막는 회귀 2종★: ① 이 부류의 주입 실패를 실패 행/유실로 바꾸는 변경(ADR-0116 결정 7) ② 그렇게
        //   쌓인 백로그를 **새 발송이 앞지르게** 만드는 변경(ADR-0121 결정 2 — 게이트 생략은 유지하되 순서는
        //   지킨다). ②의 성립 근거가 7차에 바뀌었다: 판정 지점을 봉인하는 게 아니라 **적재 순서**가 답한다
        //   (ADR-0125 — 새 편지도 큐 꼬리로 가므로 앞지를 방법 자체가 없다).
        // ★고립 없음의 계기 ㉠도 여기서 단언한다(spec §7 ④)★: m1 의 주입 실패분을 여는 것은 **뒤이은 발송의
        //   동기 드레인**이다 — 도어벨도, 자기 재시도도 아니다(자기 주입 실패로 자기 도어벨을 누르는 것은
        //   금지 — spec §5). 그래서 m2 발송 하나만으로 m1 이 나가야 한다.
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_tui_id, tui) = live_no_turn_signal("tui");
        port.set_roster(vec![me, tui]);
        port.fail_at(&[0]); // 첫 inject 실패 → m1 파킹.

        // 본문을 갈라 둔다 — 주입 **순서**를 바이트로 단언하려면 두 편지가 구별돼야 한다.
        let out =
            send_body(&svc, "m1", from, "alice", &["tui"], "묵은 편지").expect("파킹은 반려 아님");
        assert_eq!(
            (out[0].status, out[0].code),
            (SendStatus::Pending, None),
            "주입 실패 → 파킹(실패 행도, 조용한 유실도 아니다): {out:?}"
        );
        assert_eq!(svc.parked_len("tui"), 1);
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Pending],
            "장부도 pending 으로 남아야(유실 금지)"
        );

        // ★새 편지는 묵은 것을 앞지르지 않는다(ADR-0121 결정 2 · ADR-0125)★ — 큐 꼬리에 적재된 뒤 **같은
        //   호출의 드레인**이 앞에서부터 비우므로 m1 이 먼저, m2 가 그 뒤에 나간다. m2 의 응답은 그 드레인이
        //   자기 편지까지 넣은 것을 확인했으므로 `delivered` 다(6차의 `pending` 은 ADR-0125로 반전됐다).
        let second = send_body(&svc, "m2", from, "alice", &["tui"], "새 편지").expect("행 응답");
        assert_eq!(
            second[0].status,
            SendStatus::Delivered,
            "적재 후 드레인이 묵은 것과 함께 냈다 = delivered(직발송이 아니라 큐를 거친 결과다): {second:?}"
        );
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="alice">묵은 편지</message>"#.to_string(),
                r#"<message from="alice">새 편지</message>"#.to_string()
            ],
            "묵은 것(m1) → 새것(m2) 순으로 한 배치에 나갔다(오래된 순 — 뒤이은 발송의 드레인이 열었다)"
        );
        assert_eq!(svc.parked_len("tui"), 0, "고립 없음 — 배치가 큐를 비웠다");
        assert_eq!(
            (svc.ledger_statuses("m1"), svc.ledger_statuses("m2")),
            (
                vec![DeliveryStatus::Delivered],
                vec![DeliveryStatus::Delivered]
            ),
            "둘 다 실제 주입 시점에 delivered"
        );
    }

    #[test]
    fn a_mid_batch_injection_failure_stops_there_and_leaves_the_rest_pending() {
        // ★ADR-0121 결정 2 후단 — 배치 부분 실패의 결말★: 실패 지점에서 멈추고, **이미 나간 앞부분은
        //   `delivered`**, 실패 지점 이후는 파킹에 `pending` 으로 남는다. 부분 성공을 전량 실패로 되돌리지
        //   않는다 — `delivered → failed` 는 장부 전이 그래프상 **불법**이라(ledger `fail_pending` 가드가
        //   정본) 롤백을 표현할 수단 자체가 없다. 그래서 이 단언이 곧 "장부 행이 그래프를 지킨다" 의 관측이다.
        // ★백로그를 게이트로 만드는 이유(7차 — ADR-0125)★: 이제 **모든 발송이 자기 호출에서 드레인을
        //   돌리므로**, 도어벨 미소비만으로는 큐가 쌓이지 않는다(예전엔 그 방법이 통했다). 3건 배치를 만들려면
        //   드레인이 **못 나가게** 해야 하고, 그 정식 수단이 idle 게이트다(턴 신호 있는 수신자를 busy 로).
        let (svc, port, gate) = svc_gated();
        let (from, me) = live_sender("alice");
        let (recv_id, recv) = live("recv"); // 턴 신호 있음 = 게이트 대상.
        port.set_roster(vec![me, recv]);
        gate.set_busy(recv_id, 0);

        for (id, body) in [("m1", "하나"), ("m2", "둘"), ("m3", "셋")] {
            let out = send_body(&svc, id, from, "alice", &["recv"], body).expect("행 응답");
            assert_eq!(
                out[0].status,
                SendStatus::Pending,
                "{id} 는 적재되고 드레인이 게이트에 걸린다: {out:?}"
            );
        }
        assert_eq!(
            svc.parked_msg_ids("recv"),
            vec!["m1", "m2", "m3"],
            "적재 순서 = 큐 순서(오래된 순)"
        );

        // 턴이 끝난 뒤의 배치에서 **2번째 주입**을 실패시킨다(게이트에 걸린 동안엔 주입이 0회였다).
        gate.clear();
        port.fail_at(&[1]);
        svc.flush_for("recv", recv_id);

        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="alice">하나</message>"#.to_string()],
            "실패 지점에서 배치가 끊긴다 — 그 뒤 항목은 쓰이지 않았다"
        );
        assert_eq!(
            svc.parked_msg_ids("recv"),
            vec!["m2", "m3"],
            "실패 지점 이후는 나이 순서를 지킨 채 큐로 되돌아온다(TTL 연장 없음)"
        );
        assert_eq!(
            (
                svc.ledger_statuses("m1"),
                svc.ledger_statuses("m2"),
                svc.ledger_statuses("m3")
            ),
            (
                vec![DeliveryStatus::Delivered],
                vec![DeliveryStatus::Pending],
                vec![DeliveryStatus::Pending]
            ),
            "앞부분은 delivered 로 남고(되돌리지 않는다) 나머지는 pending 유지"
        );
    }

    #[test]
    fn an_overlapping_drain_retreats_with_pending_and_the_letter_still_goes_out_in_order() {
        // ★spec §7 ⑦ · ADR-0125 §영향 미검증 ② — 옛 좌석 단언의 **이전분**★. 좌석 예약은 폐지됐지만 그것이
        //   봉인하던 두 성질은 남는다: **순서 역전 없음**과 **고립 없음(`parked_len == 0`)**.
        // 재현하는 인터리빙(7차엔 상시 경우다 — 드레인이 발송 호출 안에서 돈다):
        //   ① m1 이 적재 후 자기 드레인에 들어가 inject 안에서 멈춘다(배치가 in-flight)
        //   ② 그 사이 m2 가 도착해 같은 수신자를 지목한다 → 적재는 되고, 그 드레인은 **0단계에서 물러난다**
        //   ③ m1 의 inject 가 실패해 그 배치가 재파킹되고, 정산이 물러난 쪽 도어벨을 되울린다
        // 가드가 없으면 ②가 큐를 다시 드레인해 **m1 의 잔여보다 먼저** 수신자에게 닿는다(순서 역전).
        // ★결정론★: `set_on_inject` hook 은 fake port 의 `inject` **안에서 같은 스레드로** 돌므로 "드레인 중에
        //   도착한 동시 발송" 을 sleep 없이 정확히 그 지점에 꽂는다(스레드·타이밍 의존 0).
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_tui_id, tui) = live_no_turn_signal("tui");
        port.set_roster(vec![me, tui]);

        // hook 안 단언은 패닉이 언와인딩으로 새 나가 원인이 흐려지므로, 결과만 담아 와서 밖에서 단언한다.
        let observed = Arc::new(StdMutex::new(None::<RecipientResult>));
        let observed_h = observed.clone();
        let svc_h = svc.clone();
        let armed = Arc::new(StdMutex::new(true));
        let armed_h = armed.clone();
        port.set_on_inject(Arc::new(move |idx| {
            // m1 의 주입(호출 0) 한가운데 — 여기서 m2 가 도착한다.
            if idx != 0 || !std::mem::replace(&mut *armed_h.lock().unwrap(), false) {
                return;
            }
            let second =
                send_body(&svc_h, "m2", from, "alice", &["tui"], "새 편지").expect("행 응답");
            *observed_h.lock().unwrap() = Some(second[0].clone());
        }));
        port.fail_at(&[0]); // m1 의 주입 실패 → 그 배치가 재파킹(나이 보존).

        let first =
            send_body(&svc, "m1", from, "alice", &["tui"], "묵은 편지").expect("파킹은 반려 아님");
        assert_eq!(first[0].status, SendStatus::Pending, "{first:?}");
        let loser = observed.lock().unwrap().clone().expect("동시 발송 행");
        assert_eq!(
            loser.status,
            SendStatus::Pending,
            "겹친 드레인에서 물러난 쪽은 **보수적으로** pending 을 답한다(자기 편지의 주입 여부를 모른다)"
        );
        assert!(
            loser.hint.unwrap_or_default().contains("at the same time"),
            "그 pending 은 ㉯(확인 불가)여야 — ㉮(큐에 남음) 사유로 답하면 '안 갔다' 로 오독된다(spec §6)"
        );
        // ★유실 0 · 순서 유지 · 고립 없음★: m2 는 m1 이 in-flight 인 동안 꼬리에 적재됐고, 재파킹된 m1 은
        //   나이(admission 순번)로 그 앞에 선다. 물러난 쪽 도어벨을 영수증 보유자가 되울려 주므로 수동 flush
        //   없이 여기까지 온다(spec §7 ④ ㉡ — 어느 계기가 냈는지가 곧 이 단언이다).
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="alice">묵은 편지</message>"#.to_string(),
                r#"<message from="alice">새 편지</message>"#.to_string()
            ],
            "배달 순서 = 적재 순서(m1 → m2). 겹침 가드가 없으면 m2 가 m1 을 앞지른다"
        );
        assert_eq!(svc.parked_len("tui"), 0, "고립 없음 — 큐가 비워졌다");
        assert_eq!(
            (svc.ledger_statuses("m1"), svc.ledger_statuses("m2")),
            (
                vec![DeliveryStatus::Delivered],
                vec![DeliveryStatus::Delivered]
            ),
            "응답이 pending 이어도 장부는 실제 주입 시점에 delivered 다(두 축은 다르다 — spec §6)"
        );
    }

    #[test]
    fn a_concurrent_burst_loses_nothing_and_keeps_each_senders_own_order() {
        // ★이것은 **soak 테스트**다 — 겹침 가드의 봉인이 아니다(리뷰 지적, 두 리뷰어 합치)★.
        //   ★이 이름이 약속하지 **않는** 것★: 전역 배달 순서가 뮤텍스 획득(= 적재) 순서와 일치한다는 단언은
        //   여기 없다. **스레드 사이**의 적재 순서는 스케줄러가 정하므로 관측 자체가 불가하고, 겹침 가드를
        //   지워도 이 테스트는 **확률적으로만** 빨개진다. 가드의 결정적 봉인은 `an_overlapping_drain_...`
        //   (hook 으로 인터리빙을 그 지점에 꽂는다)과 두 유예-되울림 hook 테스트가 진다 — 그쪽을 지우고
        //   여기를 근거로 삼지 말 것.
        // ★그래서 여기가 실제로 고정하는 것★: 실 스레드 경합 아래에서 ① 한 발신자가 자기 프로그램 순서대로
        //   적재한 편지들의 상대 순서가 배달에서도 유지되는가(자기 앞 편지는 항상 큐에서 자기보다 앞이다 —
        //   깨지면 배치가 큐 앞머리를 건너뛴 것이다) ② 유실 0 · 중복 0 ③ 큐 잔여 0(고립 없음 — 물러난 쪽
        //   도어벨을 영수증 보유자가 갚는다) ④ 응답이 `delivered|pending` 뿐(경합은 신원 축을 안 건드리므로
        //   `failed` 가 없다). 7차엔 여러 발신 스레드가 같은 수신자를 동시에 드레인하는 것이 **상시 경우**라
        //   이 넷이 실경로에서 무너지지 않는지 보는 값이 있다.
        // ★cap 은 일부러 안 건드린다★ — 총량을 cap(100) 아래로 잡아 경합이 `MAILBOX_FULL` 로 새지 않게 한다
        //   (그 축은 `the_cap_gate_applies_to_a_live_idle_recipient_too` 소관).
        const SENDERS: usize = 4;
        const PER_SENDER: usize = 20;

        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_tui_id, tui) = live_no_turn_signal("tui"); // 게이트 없음 = 드레인이 항상 시도된다.
        port.set_roster(vec![me, tui]);

        let handles: Vec<_> = (0..SENDERS)
            .map(|k| {
                let svc_h = svc.clone();
                std::thread::spawn(move || {
                    (0..PER_SENDER)
                        .map(|n| {
                            let tag = format!("t{k}-{n}");
                            let out = send_body(&svc_h, &tag, from, "alice", &["tui"], &tag)
                                .expect("행 응답");
                            out[0].status
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let statuses: Vec<SendStatus> = handles
            .into_iter()
            .flat_map(|h| h.join().expect("발신 스레드"))
            .collect();

        assert!(
            statuses
                .iter()
                .all(|s| matches!(s, SendStatus::Delivered | SendStatus::Pending)),
            "경합은 실패 행을 만들지 않는다(신원 축은 경합과 무관): {statuses:?}"
        );
        assert_eq!(svc.parked_len("tui"), 0, "고립 없음 — 큐가 끝내 비워졌다");

        let bodies = port.injected_bodies();
        assert_eq!(
            bodies.len(),
            SENDERS * PER_SENDER,
            "유실 0 · 중복 0 — 모든 편지가 정확히 한 번 나갔다"
        );
        let order: Vec<(usize, usize)> = bodies
            .iter()
            .map(|b| {
                let start = b.find('>').expect("여는 태그") + 1;
                let end = b.rfind('<').expect("닫는 태그");
                let (k, n) = b[start..end]
                    .trim_start_matches('t')
                    .split_once('-')
                    .expect("태그 표기");
                (k.parse().expect("발신자"), n.parse().expect("순번"))
            })
            .collect();
        for k in 0..SENDERS {
            let seen: Vec<usize> = order
                .iter()
                .filter(|(s, _)| *s == k)
                .map(|(_, n)| *n)
                .collect();
            assert_eq!(seen.len(), PER_SENDER, "발신자 {k} 의 편지가 전부 나가야");
            assert!(
                seen.windows(2).all(|w| w[0] < w[1]),
                "발신자 {k} 가 적재한 순서가 배달에서 뒤집혔다 — 배치가 큐 앞머리를 건너뛰었다: {seen:?}"
            );
        }
    }

    #[test]
    fn the_cap_gate_applies_to_a_live_idle_recipient_too() {
        // ★ADR-0125 §영향 · spec §5 — cap 적용 표면 확대★: 5차까지 보관함 cap 검사는 **파킹 갈래에서만** 돌았다.
        //   빈 큐 + idle 이면 직발송이 cap 을 우회했고 "101번째도 즉시 주입되며 그것은 cap 위반이 아니다" 가
        //   명시 계약이었다. 전부-큐가 되면서 **모든 발송이 적재 게이트를 지나므로** 산 수신자에게도
        //   `MAILBOX_FULL` 이 날 수 있다 — **설계 귀결이지 회귀가 아니다**.
        // ★"전반적으로 늘어난다" 로 읽지 않게 좁은 구간까지 함께 고정한다(spec §5 정정)★: 수신자가 유휴면
        //   드레인이 큐를 비우므로 아무리 보내도 cap 은 애초에 물리지 않는다(①). 실제로 달라지는 구간은
        //   드레인이 따라잡기 전에 큐가 cap 을 넘길 때뿐이고, 여기선 그것을 idle 게이트로 결정적으로 만든다(②③).
        let (svc, port, gate) = svc_gated();
        let (from, me) = live_sender("boss");
        let (r_id, recv) = live("recv");
        port.set_roster(vec![me, recv]);

        // ① 유휴 수신자 — cap(100)의 두 배를 보내도 한 건도 안 걸린다(드레인이 매번 큐를 비운다).
        for i in 0..200 {
            let out = send(&svc, &format!("idle{i}"), from, "boss", &["recv"]).expect("행 응답");
            assert_eq!(
                out[0].status,
                SendStatus::Delivered,
                "유휴 수신자에겐 cap 이 물리지 않는다(#{i}): {out:?}"
            );
        }
        assert_eq!(svc.parked_len("recv"), 0, "큐가 비어 있으니 cap 분모도 0");

        // ② 드레인이 못 도는 구간(턴 중)에서만 큐가 cap 까지 찬다.
        gate.set_busy(r_id, 0);
        for i in 0..100 {
            let out = send(&svc, &format!("busy{i}"), from, "boss", &["recv"]).expect("행 응답");
            assert_eq!(
                out[0].status,
                SendStatus::Pending,
                "턴 중 = 큐에 남는다(#{i})"
            );
        }
        assert_eq!(svc.parked_len("recv"), 100);

        // ③ 그 다음 발송은 **산 수신자인데도** cap 게이트에 걸린다 — 5차엔 직발송이 우회하던 자리다.
        let over = send(&svc, "over", from, "boss", &["recv"]).expect("행 응답");
        assert_eq!(
            (over[0].status, over[0].code),
            (SendStatus::Failed, Some(FailCode::MailboxFull)),
            "산 수신자도 cap 게이트를 지난다(직발송 우회 폐지 — ADR-0125): {over:?}"
        );
        assert_eq!(svc.parked_len("recv"), 100, "반려분은 큐에 안 들어간다");
        // 실패 행도 장부 종점으로 남는다(spec §5 — 조회 행수 = 응답 행수).
        assert_eq!(svc.ledger_statuses("over"), vec![DeliveryStatus::Failed]);
    }

    #[test]
    fn a_drain_that_deferred_a_flush_repays_it_even_when_its_own_inject_fails() {
        // ★영수증을 쥔 쪽이 유예 깨우기를 갚는다(spec §5 불변식 · ADR-0121 → ADR-0125로 상시 경우가 됐다)★:
        //   배치가 in-flight 인 동안 같은 이름 앞 flush 를 시도한 쪽은 `flush_for` 0단계에서 물러나며
        //   `deferred_flush` 에 표식만 남긴다(진행 중 배치를 앞지르지 않으려고 = 드레인 중복 진입 가드).
        //   영수증 보유자가 정산하며 되울려 주지 않으면 그 깨우기가 증발해(lost wakeup) 재파킹분이 다음
        //   사건이나 TTL 까지 묶인다. ★7차엔 이 의무를 **발송 호출의 동기 드레인**도 진다★ — 드레인이 발신
        //   스레드에서 도니까 영수증을 쥐는 것이 상시 경우이고, 그래서 이 갈래가 예외가 아니라 주 경로다.
        // ★자기 재시도가 아니다★: 여기서 갚는 것은 *다른 호출자가 요청해 둔* 깨우기다. 표식이 없는 단독 실패
        //   경로는 여전히 아무 도어벨도 누르지 않는다(`inject_failure_parks_pending_without_a_turn_signal` 의
        //   "발송 후 parked_len == 1" 단언이 그쪽을 지킨다).
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (tui_id, tui) = live_no_turn_signal("tui");
        port.set_roster(vec![me, tui]);

        let svc_h = svc.clone();
        let armed = Arc::new(StdMutex::new(true));
        let armed_h = armed.clone();
        port.set_on_inject(Arc::new(move |idx| {
            if idx != 0 || !std::mem::replace(&mut *armed_h.lock().unwrap(), false) {
                return;
            }
            // 앞 배치가 in-flight 라 이 flush 는 드레인 없이 물러나고 표식만 남긴다(0단계 = 중복 진입 가드).
            svc_h.flush_for("tui", tui_id);
        }));
        port.fail_at(&[0]); // 드레인의 주입 실패 → 재파킹(자기 도어벨은 금지 — spec §5).

        send_body(&svc, "m1", from, "alice", &["tui"], "묵은 편지").expect("파킹");

        // 되울림이 없으면 m1 은 큐에 남고 주입 기록도 없다(= 유예 깨우기 증발).
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="alice">묵은 편지</message>"#.to_string()],
            "물러났던 flush 가 정산 후 되울려져 재파킹분을 집어야 한다"
        );
        assert_eq!(svc.parked_len("tui"), 0);
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
    }

    #[test]
    fn a_successful_drain_also_repays_the_flush_it_deferred() {
        // ★같은 계약의 성공 갈래★: 배치 영수증은 주입이 성공해도 정산 시점까지 in-flight 다 — 그동안 물러난 flush 를
        //   갚아야 하는 의무는 실패 갈래와 같다. 성공 갈래만 빼먹으면 "우연히 다른 도어벨이 있었나" 에 배달이
        //   좌우된다(유예한 id 가 개명·힌트 큐라 아무 도어벨로도 안 열리는 경우가 실경로다 — `deferred_flush`).
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (tui_id, tui) = live_no_turn_signal("tui");
        port.set_roster(vec![me, tui]);

        let svc_h = svc.clone();
        let armed = Arc::new(StdMutex::new(true));
        let armed_h = armed.clone();
        port.set_on_inject(Arc::new(move |idx| {
            if idx != 0 || !std::mem::replace(&mut *armed_h.lock().unwrap(), false) {
                return;
            }
            // 주입 중에 그 이름 앞으로 파킹분이 생기고(도어벨을 누르지 않는 경로) flush 가 물러난다.
            svc_h
                .park_absent_for_test(
                    "m0",
                    ident(),
                    "s",
                    "tui",
                    "덤",
                    Entrance::Mcp,
                    &SendMeta::default(),
                )
                .expect("park");
            svc_h.flush_for("tui", tui_id);
        }));

        send_body(&svc, "m1", from, "alice", &["tui"], "내 편지").expect("행 응답");

        assert_eq!(
            svc.parked_len("tui"),
            0,
            "성공 갈래도 유예 표식을 갚아야 — 안 갚으면 m0 이 다음 사건까지 큐에 묶인다"
        );
        assert_eq!(svc.ledger_statuses("m0"), vec![DeliveryStatus::Delivered]);
    }

    #[test]
    fn the_flush_gate_asks_the_same_predicate_the_send_path_asks() {
        // ★ADR-0121 §영향 — 게이트 술어는 한 곳에서만 정의된다★: 발송측이 "이 부류엔 게이트 없음" 으로
        //   판정하는데 flush측이 게이트에 걸어 물러나면, 순서 보장 때문에 큐에 합류한 편지를 **아무도 열지
        //   않는다**(배달 정지). 그래서 두 측이 같은 술어(`gate_says_busy`)를 부르는지 여기서 봉인한다.
        // ★게이트를 busy 로 세팅한 채 단언한다★: 턴 신호 없는 부류는 그 값을 **보지 않아야** 한다 —
        //   `flush_for` 가 `busy.is_busy` 를 직접 부르는 옛 형태로 돌아가면 배치가 스킵돼 여기서 빨개진다.
        let (svc, port, gate) = svc_gated();
        let (from, me) = live_sender("alice");
        let (tui_id, tui) = live_no_turn_signal("tui");
        port.set_roster(vec![me, tui]);
        gate.set_busy(tui_id, 0);

        // 적재 → 드레인의 첫 주입이 실패 → 재파킹. 도어벨은 안 눌린다(자기 주입 실패로 자기 도어벨을 누르는
        //   것은 spec §5 금지 — 도달 불가해진 수신자에 재주입을 반복하게 된다).
        port.fail_at(&[0]);
        let out = send(&svc, "m1", from, "alice", &["tui"]).expect("행 응답");
        assert_eq!(out[0].status, SendStatus::Pending, "{out:?}");
        assert_eq!(svc.parked_len("tui"), 1);

        port.fail_at(&[]); // 다음 주입은 성공한다.
        svc.flush_for("tui", tui_id);
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Delivered],
            "턴 신호 없는 수신자의 배치는 게이트가 busy 라 해도 나간다(발송측과 같은 술어)"
        );
        assert_eq!(svc.parked_len("tui"), 0);
    }

    #[test]
    fn flush_on_appearance_delivers_oldest_first_individually_wrapped() {
        // 부재 시 3건 파킹 → 등장 flush → 오래된 순 + 각자 개별 봉투 주입.
        let (svc, port) = svc();
        port.set_roster(vec![]);
        for (i, m) in ["m0", "m1", "m2"].iter().enumerate() {
            svc.park_absent_for_test(
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
            svc.park_absent_for_test(
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
            svc.park_absent_for_test(
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
        port.set_on_inject(Arc::new(move |idx| {
            if idx == 0 {
                // 동시 park 가 파킹되도록 로스터를 잠시 비운다(부재). flush_for 재검증은 이 hook 전에 끝남.
                port_hook.set_roster(vec![]);
                for i in 0..MAILBOX_CAP_FOR_TEST {
                    // 동시 발송(신규 admission) — cap 까지 채운다(로스터 부재라 파킹).
                    let _ = svc_hook.park_absent_for_test(
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
        // ★유실 0 — 단 cap 을 넘기지도 않는다(F1 에서 기대값 보정: 102 → 99)★. 옛 기대값은 "동시 park 가
        //   cap(100)까지 다 들어오고 재파킹분 2건이 그 위에 얹힌다" 였는데, 그게 바로 사이클마다 큐를 키우던
        //   구멍이다(빈 큐 창). 이제 drain 한 배치 3건이 in-flight 로 분모에 남아 동시 park 는 97건만
        //   수용되고, 재파킹분 2건이 되돌아와 97 + 2 = 99 ≤ cap 이 된다. **무손실 성질은 그대로**다 —
        //   b1·b2 는 한 건도 안 사라졌고(아래 pending 단언), 못 들어온 3건은 발신자에게 `MAILBOX_FULL` 로
        //   가시화됐다(조용한 드롭 아님 — spec §5).
        assert_eq!(
            svc.parked_len("late"),
            MAILBOX_CAP_FOR_TEST - 1,
            "재파킹분(b1,b2)은 유실되지 않고, 합계는 cap 을 넘지 않는다(97 동시 park + 2 재파킹)"
        );
        assert!(
            svc.ledger_statuses("c99").is_empty(),
            "in-flight 때문에 반려된 동시 발송은 장부에도 안 남는다(반려 = 저장 자체를 안 함)"
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
        port.set_on_inject(Arc::new(|_| {}));
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
            svc.park_absent_for_test(
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
            .park_absent_for_test(
                "over",
                ident(),
                "s",
                "full",
                "x",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect_err("cap 초과는 반려");
        assert_eq!(rej, SendFail::Row(FailCode::MailboxFull));
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
        svc.park_absent_for_test(
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
        svc.park_absent_for_test(
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
        svc.park_absent_for_test(
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
        //   그 사이 수신자가 respawn(같은 이름, **새 PeerId**)했다 — flush_for 는 enqueue 의 stale id 를
        //   맹신하지 않고 execution 시점 로스터로 이름을 재해석해 **현재 유일 후보의 새 id** 로 주입해야 한다.
        //   (앞 두 테스트는 재검증이 skip 으로 가는 축(부재·ambiguous)만 덮었고, 재검증이 **성공하되 id 가
        //   갱신되는** 핵심 축은 미검증이었다 — 이 테스트가 그 갭을 메운다.)
        let (svc, port) = svc();
        port.set_roster(vec![]); // 부재 → park(이름 앞으로 파킹).
        svc.park_absent_for_test(
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

        // ── respawn 모사: 같은 이름 "recv" 지만 enqueue 때와 다른 새 PeerId 로 로스터에 등장 ──────────
        let stale_id = PeerId::new_v4(); // enqueue 스냅샷이 알던(이제 죽은) 옛 incarnation id.
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
        let injected: Vec<PeerId> = port
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
        let from = SenderIdentity {
            peer_id: PeerId::new_v4(),
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
        assert_eq!(d.from.peer_id, from.peer_id);
        assert_eq!(d.from.epoch, 3);
        assert!(matches!(d.entrance, Entrance::Mcp));
        assert_eq!(d.meta, SendMeta::default(), "통보는 계약 메타가 비어야");
    }

    #[test]
    fn park_payload_roundtrip_preserves_contract_meta_with_newlines() {
        // ★C3★: request/회신 메타가 park→flush 를 무손실로 건넌다(늦은 배달도 같은 봉투 — 모듈 헤더).
        //   reply_to 는 **에이전트 입력**이라 개행·멀티바이트가 섞일 수 있다 — 길이 접두 인코딩이 이를 견딘다.
        let from = SenderIdentity {
            peer_id: PeerId::new_v4(),
            epoch: 7,
        };
        let meta = SendMeta {
            request: true,
            reply_by_raw: Some("10m".to_string()),
            // 파싱값은 park 를 건너지 않는다(계약은 이미 장부에 열렸다 — ParkPayload 주석).
            reply_by: Some(Duration::from_secs(600)),
            reply_to: None,
            to_attr: None,
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
                to_attr: None,
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
        // ★핵심 분기★: 산·도달 수신자인데 턴 진행 중 → 주입 금지, 파킹(pending). 상태 어휘는 주입 실패 파킹과
        //   공유하고(새 상태 발명 금지) hint 만 사유를 구분한다.
        let (svc, port, gate) = svc_gated();
        let (alice_id, alice) = live("alice");
        port.set_roster(vec![alice]);
        gate.set_busy(alice_id, 0);
        let out = svc
            .park_absent_for_test(
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
    fn an_idle_recipient_gets_it_within_the_same_send_call() {
        // ★옛 `idle_recipient_injects_immediately` 의 **이전분**(ADR-0125)★: 게이트가 idle 이면 그 편지는
        //   **이 호출 안에서** 나가고 응답은 `delivered` 다 — 관측 결말은 그대로고, 경로만 적재-후-드레인이다
        //   (즉시 주입 = 직발송이라는 옛 축은 폐지됐다). 게이트가 이 결말을 가른다는 성질은 유지된다.
        // ★단 이 테스트가 보는 것은 **게이트 축**뿐이다★ — 여기 단언은 적재를 건너뛰는 지름길로도 만족되므로
        //   "적재를 거쳤나" 의 봉인은 `an_idle_sends_letter_is_observably_taken_out_of_the_queue_not_bypassed`
        //   가 진다(리뷰 blind MEDIUM).
        let (svc, port, _gate) = svc_gated();
        let (_id, alice) = live("alice");
        port.set_roster(vec![alice]);
        let out = svc
            .park_absent_for_test(
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
        assert_eq!(svc.parked_len("alice"), 0, "드레인이 큐를 비웠다");
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
            .park_absent_for_test(
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
                .park_absent_for_test(
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
            .park_absent_for_test(
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
        svc.park_absent_for_test(
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
            svc.park_absent_for_test(
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
            svc.park_absent_for_test(
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
        // ★fix 2 회귀(blackhole)★: 동명 둘 중 하나를 **exact-PeerId** 로 지목했고 그가 턴 중이면, 파킹은
        //   이름-키라 flush 의 유일성 게이트가 영영 보류한다(TTL 까지 배달 안 됨). park 항목의 id 힌트가
        //   그 사각지대를 메운다 — 힌트가 살아 있으면 이름 유일성과 무관하게 그 id 로 배달.
        let (svc, port, gate) = svc_gated();
        let (id_a, dup_a) = live("dup");
        let (_id_b, dup_b) = live("dup"); // 같은 이름의 두 번째 에이전트(이름 해석은 영구 ambiguous).
        port.set_roster(vec![dup_a, dup_b]);
        gate.set_busy(id_a, 0);
        // exact-id 지목(동명 모호성을 의도적으로 통과) → busy 라 파킹.
        let out = svc
            .park_absent_for_test(
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
        svc.park_absent_for_test(
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
        // 재스폰: 옛 id 는 사라지고 같은 이름의 **새 PeerId** 가 등장.
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
    fn a_new_send_never_overtakes_an_existing_queue() {
        // ★fix 5 회귀(FIFO 일관성) — 7차 축 갱신(ADR-0125)★: 큐에 m0·m1 이 대기 중일 때 새 m2 가 그것들을
        //   앞지르면 수신자는 (m2, m0, m1) 순서로 본다. 옛 판은 "큐가 비어 있지 않으면 직발송도 파킹" 이라는
        //   **합류 판정**으로 그걸 막았는데, 그 판정은 폐지됐다 — 이제 m2 는 **무조건 꼬리에 적재**되고 같은
        //   호출의 드레인이 앞에서부터 비우므로 순서가 판정 없이 성립한다. 그래서 단언 축이 바뀐다:
        //   "m2 가 파킹됐나" 가 아니라 **"수신자가 본 순서가 적재 순서인가"** 이고, m2 의 응답은 그 드레인이
        //   자기 편지까지 냈으므로 `delivered` 다.
        let (svc, port, gate) = svc_gated();
        let (recv_id, recv) = live("recv");
        port.set_roster(vec![recv]);
        gate.set_busy(recv_id, 0);
        for (i, m) in ["m0", "m1"].iter().enumerate() {
            svc.park_absent_for_test(
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
            .park_absent_for_test(
                "m2",
                ident(),
                "s",
                "recv",
                "b2",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("no reject");
        assert_eq!(
            out,
            SendOutcome::Delivered,
            "적재 후 그 호출의 드레인이 묵은 것과 함께 냈다 = delivered: {out:?}"
        );
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="s">b0</message>"#.to_string(),
                r#"<message from="s">b1</message>"#.to_string(),
                r#"<message from="s">b2</message>"#.to_string(),
            ],
            "수신자가 보는 순서 = 적재 순서(새 발송이 큐를 앞지르지 않는다)"
        );
        assert_eq!(svc.parked_len("recv"), 0);
    }

    #[test]
    fn an_empty_queue_and_an_idle_recipient_still_ends_as_delivered() {
        // ★spec §7 ① — 옛 `empty_queue_idle_send_still_injects_directly` 의 **이전분**(약화 아님)★:
        //   관측 결말(그 호출 안에서 주입 + 응답 `delivered`)은 그대로 살아나되, 단언 축이 바뀐다 —
        //   "직발송했다" 가 아니라 **"적재된 뒤 공용 드레인이 냈다"** 를 본다(ADR-0125).
        // ★경로를 눈으로 확인한다★: 드레인이 큐에서 뺀 것이므로 그 자리에 **파킹 레코드가 존재했고**
        //   (`hinted_id` 를 실은 항목) 지금은 비어 있다. 적재를 건너뛰는 갈래가 되살아나면 아래 in-flight
        //   회계·큐 상태가 아니라 **드레인 자체가 돌지 않아** 응답이 `pending` 으로 떨어진다.
        let (svc, port, _gate) = svc_gated();
        let (_id, recv) = live("recv");
        port.set_roster(vec![recv]);
        let out = svc
            .park_absent_for_test(
                "m1",
                ident(),
                "s",
                "recv",
                "hi",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("no reject");
        assert_eq!(
            out,
            SendOutcome::Delivered,
            "빈 큐 + idle 이어도 적재를 거치고, 같은 호출의 드레인이 내면 delivered"
        );
        assert_eq!(port.injected_bodies().len(), 1);
        assert_eq!(svc.parked_len("recv"), 0, "드레인이 그 항목을 비웠다");
        assert_eq!(
            svc.in_flight_len("recv"),
            0,
            "영수증 정산 누락 없음 — 남으면 그 수신자 레인이 영구 봉쇄된다"
        );
        // ★이 테스트만으로는 봉인이 안 된다(리뷰 blind MEDIUM)★: 위 넷은 **결말**이라 적재를 건너뛰는
        //   지름길도 똑같이 만들어 낸다. 경로 자체의 봉인은 아래
        //   `an_idle_sends_letter_is_observably_taken_out_of_the_queue_not_bypassed` 가 진다.
    }

    #[test]
    fn an_idle_sends_letter_is_observably_taken_out_of_the_queue_not_bypassed() {
        // ★spec §7 ① 의 **반사실 봉인**(리뷰 blind MEDIUM — "주입 코드는 한 벌" 을 실제로 지키는 단언)★.
        //
        // ★이 단언이 죽이는 반사실★: "빈 큐 + idle 이면 적재를 건너뛰고 곧바로 `port.inject` 한 뒤
        //   `Delivered` 를 답하는 지름길"(= 5차 직발송의 부활). **그 갈래를 되살리면 여기가 red 가 된다.**
        //   바로 위 `an_empty_queue_and_an_idle_recipient_still_ends_as_delivered` 와
        //   `an_idle_recipient_gets_it_within_the_same_send_call` 은 *결말*(주입 1회 · 큐 0 · in-flight 0 ·
        //   `delivered`)만 보는데 지름길도 그 결말을 그대로 만든다 — 그래서 그 둘은 이 계약을 못 지킨다.
        // ★어떻게 관측하나★: 주입 **한가운데**를 들여다본다(`on_inject` hook = 락 밖, 드레인이 영수증을 쥔
        //   시점). 지름길은 저장소를 아예 안 건드리므로 세 값이 전부 갈린다:
        //     ① in-flight = 1 — 그 편지가 **큐에서 빠져 나온 몫**으로 cap 회계에 잡혀 있다(지름길이면 0).
        //     ② 장부 행 = `pending` — 적재가 락 안에서 이미 행을 남겼다(지름길이면 이 시점에 행이 없거나
        //        곧장 `delivered` 다 — 5차가 그랬다). 이 값이 미명세 (ii)"행이 pending 을 거치는가" 의
        //        구현 위치이기도 하다.
        //     ③ 큐 길이 = 0 — ①이 "아직 안 뺐다" 가 아니라 "빼서 들고 나갔다" 임을 고정한다.
        let (svc, port, _gate) = svc_gated();
        let (from, me) = live_sender("alice");
        let (_r, recv) = live("recv");
        port.set_roster(vec![me, recv]);

        // hook 안 패닉은 언와인딩으로 새 나가 원인이 흐려지므로, 관측만 담아 와서 밖에서 단언한다.
        let seen: Arc<StdMutex<Option<(usize, usize, Vec<DeliveryStatus>)>>> =
            Arc::new(StdMutex::new(None));
        let seen_h = seen.clone();
        let svc_h = svc.clone();
        port.set_on_inject(Arc::new(move |_| {
            *seen_h.lock().unwrap() = Some((
                svc_h.in_flight_len("recv"),
                svc_h.parked_len("recv"),
                svc_h.ledger_statuses("m1"),
            ));
        }));

        let out = send(&svc, "m1", from, "alice", &["recv"]).expect("행 응답");
        assert_eq!(
            out[0].status,
            SendStatus::Delivered,
            "전제: 빈 큐 + idle 갈래여야 이 반사실이 의미를 갖는다: {out:?}"
        );

        let (in_flight, queued, statuses) = seen.lock().unwrap().clone().expect("hook 이 돌았다");
        assert_eq!(
            in_flight, 1,
            "★직발송 갈래가 되살아나면 여기가 0 이 된다★ — 주입 중인 편지는 큐에서 빠져 나온 몫으로 회계에 잡혀 있어야 한다"
        );
        assert_eq!(
            queued, 0,
            "빠져 나왔으니 큐엔 없다(①이 '아직 안 뺐다' 가 아님을 고정)"
        );
        assert_eq!(
            statuses,
            vec![DeliveryStatus::Pending],
            "★적재가 락 안에서 남긴 행★ — 직발송이면 이 시점에 행이 아예 없거나 이미 delivered 다"
        );

        // 결말은 위 테스트와 같다 — 여기서는 거기까지 가는 **경로**를 봤을 뿐이다.
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
        assert_eq!(svc.parked_len("recv"), 0);
        assert_eq!(svc.in_flight_len("recv"), 0, "영수증 정산 누락 없음");
    }

    // ── round-7 high → ADR-0125: 진행 중인 배치를 앞지르지 않는다 ──────────────────────────────────
    //    드레인은 큐를 통째로 비운 뒤 락을 놓고 주입한다. 그 구간의 큐는 비어 있어서, 큐 길이만 보는 판정은
    //    "앞에 아무도 없다" 로 읽는다 — 그 오판이 진행 중인 배치를 통째로 앞지르는 순서 역전이었다.
    //    7차에는 새 발송이 **무조건 꼬리에 적재**되므로 앞지를 방법이 없고, 그 발송의 드레인이 겹치는 것은
    //    **0단계 가드**가 막는다(물러난 쪽은 `pending`). 아래 테스트들이 그 결말을 경로별로 덮는다.

    /// ★(a)★ 배치가 **주입 중**(큐는 비어 있음)일 때 들어온 발송은 적재되고, 그 드레인은 물러난다 —
    ///   편지는 그 배치 **뒤에** 배달된다(길이가 아니라 순서로 단언한다).
    #[test]
    fn a_send_arriving_mid_batch_is_appended_and_delivered_after_it() {
        let (svc, port, gate) = svc_gated(); // 도어벨 미배선 = 인라인 폴백(단언이 결정적).
        let (recv_id, recv) = live("recv");
        port.set_roster(vec![recv]);
        // 큐에 m0·m1 을 쌓아 둔다(busy 파킹 → 턴 종료).
        gate.set_busy(recv_id, 0);
        for (i, m) in ["m0", "m1"].iter().enumerate() {
            svc.park_absent_for_test(
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
        gate.clear();

        // 첫 항목(b0)을 수신자 stdin 에 쓰는 **그 순간** 새 발송이 들어온다. 이 시점의 큐는 drain 으로
        //   비어 있고, 배치 2건은 in-flight 다 — 큐 길이만 보는 판정이 "앞에 없음" 으로 오판하던 그 창.
        let svc_hook = svc.clone();
        let outcome: Arc<StdMutex<Option<SendOutcome>>> = Arc::new(StdMutex::new(None));
        let outcome_hook = outcome.clone();
        port.set_on_inject(Arc::new(move |idx| {
            if idx != 0 {
                return;
            }
            let out = svc_hook
                .park_absent_for_test(
                    "m2",
                    ident(),
                    "s",
                    "recv",
                    "b2",
                    Entrance::Mcp,
                    &SendMeta::default(),
                )
                .expect("no reject");
            *outcome_hook.lock().unwrap() = Some(out);
        }));

        svc.flush_for("recv", recv_id);

        assert!(
            matches!(
                outcome.lock().unwrap().as_ref(),
                Some(SendOutcome::Parked { .. })
            ),
            "주입 중인 배치가 있으면 새 발송은 꼬리에 적재되고 그 드레인은 물러난다(응답 pending): {:?}",
            outcome.lock().unwrap()
        );
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="s">b0</message>"#.to_string(),
                r#"<message from="s">b1</message>"#.to_string(),
                r#"<message from="s">b2</message>"#.to_string(),
            ],
            "수신자가 보는 순서 = 적재 순서(새 발송이 진행 중인 배치를 앞지르지 않는다)"
        );
        assert_eq!(
            svc.parked_len("recv"),
            0,
            "물러났던 몫도 결국 배달된다(고립 없음)"
        );
        assert_eq!(svc.in_flight_len("recv"), 0, "영수증 정산 누수 없음");
        assert_eq!(svc.ledger_statuses("m2"), vec![DeliveryStatus::Delivered]);
    }

    /// ★(b)★ 같은 수신자 flush 는 겹쳐 돌지 않는다(뒤 배치가 앞 배치의 잔여를 앞지르는 것 = 한 층 아래의
    ///   같은 사고). 유예된 깨우기는 **영수증 정산 시 되울려** 발이 묶이지 않는다.
    #[test]
    fn a_second_flush_defers_while_a_batch_is_in_flight_and_the_settlement_re_rings() {
        // 도어벨 **배선**(기록만 하고 아무도 소비하지 않음) — "되울림이 실제로 눌렸나" 를 인라인 실행과
        //   섞이지 않게 단언하려고 이 조립을 쓴다.
        let (svc, port, gate, bell) = svc_gated_with_doorbell();
        let (recv_id, recv) = live("recv");
        port.set_roster(vec![recv]);
        gate.set_busy(recv_id, 0);
        for (i, m) in ["m0", "m1"].iter().enumerate() {
            svc.park_absent_for_test(
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
        gate.clear();

        // 배치가 주입 중인 동안 ① 새 메일이 파킹되고 ② **두 번째 flush** 가 같은 이름으로 들어온다.
        let svc_hook = svc.clone();
        let port_hook = port.clone();
        let bell_hook = bell.clone();
        // (겹친 flush 가 주입한 건수, 그 시점까지의 도어벨 횟수).
        let probe: Arc<StdMutex<(usize, usize)>> = Arc::new(StdMutex::new((0, 0)));
        let probe_hook = probe.clone();
        port.set_on_inject(Arc::new(move |idx| {
            if idx != 0 {
                return;
            }
            let before = port_hook.injected_bodies().len();
            svc_hook
                .park_absent_for_test(
                    "m2",
                    ident(),
                    "s",
                    "recv",
                    "b2",
                    Entrance::Mcp,
                    &SendMeta::default(),
                )
                .expect("no reject");
            svc_hook.flush_for("recv", recv_id); // 겹친 두 번째 flush.
            let after = port_hook.injected_bodies().len();
            *probe_hook.lock().unwrap() = (after - before, bell_hook.seen().len());
        }));

        svc.flush_for("recv", recv_id);

        let (overlapped_injects, bells_at_hook) = *probe.lock().unwrap();
        assert_eq!(
            overlapped_injects, 0,
            "겹친 flush 는 드레인도 주입도 하지 않는다(유예)"
        );
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="s">b0</message>"#.to_string(),
                r#"<message from="s">b1</message>"#.to_string(),
            ],
            "이 조립엔 도어벨 소비자가 없으므로 합류분은 아직 큐에 있다"
        );
        assert_eq!(
            svc.parked_msg_ids("recv"),
            vec!["m2".to_string()],
            "유예분이 사라지지 않는다(무손실)"
        );
        assert_eq!(svc.in_flight_len("recv"), 0, "영수증 정산 누수 없음");
        assert_eq!(
            bell.seen().len(),
            bells_at_hook + 1,
            "정산을 마치며 유예된 flush 를 정확히 1회 되울린다(lost wakeup 금지)"
        );
        // ★여기선 유예한 id 가 **하나뿐**이라 되울림도 1건이다(단일 슬롯 시절과 값이 같다)★ — 표식이
        //   집합이라는 사실은 이 테스트가 아니라 아래 stale-id 테스트가 강제한다(round-8).
        assert_eq!(
            bell.seen().last().copied(),
            Some(recv_id),
            "되울림 대상 = 유예한 쪽이 열려던 큐의 도어벨 id"
        );
        // 되울림을 소비하면(운영에선 flush 레인) 합류분이 그제서야 순서대로 나간다.
        svc.flush_for("recv", recv_id);
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="s">b0</message>"#.to_string(),
                r#"<message from="s">b1</message>"#.to_string(),
                r#"<message from="s">b2</message>"#.to_string(),
            ],
        );
        assert_eq!(svc.parked_len("recv"), 0);
    }

    /// ★(d) round-8 high★ — 유예 표식이 id 하나짜리 슬롯이면 **나중 유예가 앞의 것을 덮는다**. 산
    ///   incarnation 이 먼저 물러나고 그 뒤 이미 reap 된 stale id 가 물러나면, 정산 후 되울리는 건 stale id
    ///   뿐이다 → `flush_for_agent` 가 `canonical_name == None` 에서 조기 반환 → **아무도 드레인하지 않는다**.
    ///   산 수신자가 멀쩡한데도 admitted 메일이 TTL 까지 묶인다(종료성은 지켜지고 깨우기의 *유용성*이 깨진다).
    #[test]
    fn a_stale_id_deferral_does_not_erase_the_live_one_and_the_mail_still_drains() {
        let (svc, port, gate, bell) = svc_gated_with_doorbell();
        let (live_id, recv) = live("recv");
        port.set_roster(vec![recv]);
        // 이미 reap 된 옛 incarnation — 로스터에도 canonical override 에도 없으므로 이 id 로는 어떤 큐도
        //   열리지 않는다(늦게 도착한 idle 통지가 딱 이 모양이다).
        let stale_id = PeerId::new_v4();

        gate.set_busy(live_id, 0);
        for (i, m) in ["m0", "m1"].iter().enumerate() {
            svc.park_absent_for_test(
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
        gate.clear();

        // 배치가 주입 중인 동안: 새 메일이 합류하고, 두 개의 flush 가 **서로 다른 id 로** 물러난다
        //   (산 id 가 먼저 — 옛 단일 슬롯은 여기서 stale 이 산 id 를 덮었다).
        let svc_hook = svc.clone();
        let bell_hook = bell.clone();
        let bells_at_hook = Arc::new(StdMutex::new(0usize));
        let bells_probe = bells_at_hook.clone();
        port.set_on_inject(Arc::new(move |idx| {
            if idx != 0 {
                return;
            }
            svc_hook
                .park_absent_for_test(
                    "m2",
                    ident(),
                    "s",
                    "recv",
                    "b2",
                    Entrance::Mcp,
                    &SendMeta::default(),
                )
                .expect("no reject");
            svc_hook.flush_for("recv", live_id); // 유예 ①(산 id)
            svc_hook.flush_for("recv", stale_id); // 유예 ②(reap 된 id)
            *bells_probe.lock().unwrap() = bell_hook.seen().len();
        }));

        svc.flush_for("recv", live_id);

        let at_hook = *bells_at_hook.lock().unwrap();
        let re_rung: Vec<PeerId> = bell.seen()[at_hook..].to_vec();
        assert_eq!(
            re_rung,
            vec![live_id, stale_id],
            "물러난 id 를 **전부** 되울린다(유예 순서 보존) — 단일 슬롯이면 [stale_id] 뿐이다"
        );

        // 운영의 flush 레인이 하는 일을 그대로 흉내 낸다: 눌린 도어벨을 순서대로 소비한다.
        for id in re_rung {
            svc.flush_for_agent(id);
        }
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="s">b0</message>"#.to_string(),
                r#"<message from="s">b1</message>"#.to_string(),
                r#"<message from="s">b2</message>"#.to_string(),
            ],
            "산 id 의 깨우기가 살아남아 합류분이 배달된다(stale 만 남으면 여기서 2건에서 멈춘다)"
        );
        assert_eq!(svc.parked_len("recv"), 0, "stranded 없음");
        assert_eq!(svc.in_flight_len("recv"), 0, "영수증 정산 누수 없음");
        assert_eq!(svc.ledger_statuses("m2"), vec![DeliveryStatus::Delivered]);
    }

    #[test]
    fn busy_park_rings_doorbell_instead_of_flushing_on_sender_thread() {
        // ★fix 11 회귀★: 도어벨이 배선돼 있으면 자가치유는 **발신 스레드에서 flush 하지 않는다** —
        //   요청만 남기고 즉시 반환(배치 blocking write 가 MCP/HTTP 워커를 잡지 않게).
        let (svc, port, gate, bell) = svc_gated_with_doorbell();
        let (recv_id, recv) = live("recv");
        port.set_roster(vec![recv]);
        gate.set_busy(recv_id, 0);
        svc.park_absent_for_test(
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

    // ── round-3 finding 2: 턴 중 rename 이 옛 이름 큐를 고아로 만들지 않는다 ────────────────────
    #[test]
    fn rename_mid_turn_still_flushes_old_name_queue_via_id_hint() {
        // ★round-3 finding 2 회귀★: busy 파킹은 **발송 시점** 이름 큐에 들어가고, 턴 종료 flush 는 **현재**
        //   이름으로 진입한다(턴 종료 통지는 id 만 안다). 턴 중에 이름이 바뀌면 옛 이름 큐를 아무도 열지 않아 TTL 까지
        //   stranded 된다 — id 힌트 역방향 조회로 그 큐도 함께 연다.
        let (svc, port, gate) = svc_gated();
        let (id, agent) = live("old-name");
        port.set_roster(vec![agent]);
        gate.set_busy(id, 0);
        svc.park_absent_for_test(
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
            turn_signal: true,
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
        // 힌트 없는 파킹(레거시 seam 이 만든 것)은 id 입구가 열지 않는다 — 그건 이름 규칙(등장 flush)의 몫
        //   이고, 여기서 열면 엉뚱한 이름 앞 메일을 이 id 로 배달할 수 있다(이름 주소 계약 위반).
        let (svc, port, _gate) = svc_gated();
        let (id, agent) = live("recv");
        port.set_roster(vec![]); // 부재 → 힌트 없는 파킹.
        svc.park_absent_for_test(
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
    ) -> (PeerId, PeerId) {
        let (id_a, dup_a) = live("dup");
        let (id_b, dup_b) = live("dup");
        port.set_roster(vec![dup_a, dup_b]);
        gate.set_busy(id_a, 0);
        gate.set_busy(id_b, 0);
        for (which, msg_id) in order {
            let to = if *which == 0 { id_a } else { id_b };
            svc.park_absent_for_test(
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
        let targets: Vec<PeerId> = port
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
    /// 도어벨 통지를 기록하는 IdleNotifier(운영은 flush 채널) — sweep 이 **깨우는지**를 단언한다.
    struct RecordingIdle {
        seen: StdMutex<Vec<PeerId>>,
    }
    impl super::super::busy::IdleNotifier for RecordingIdle {
        fn notify_idle(&self, id: PeerId) {
            self.seen.lock().unwrap().push(id);
        }
    }

    #[test]
    fn stale_busy_sweep_unblocks_parked_mail() {
        // ★round-3 finding 4 회귀(전 구간)★: 턴 종료 신호가 영영 오지 않는 턴은 busy 판정을 영구화해 그
        //   수신자 앞 배달을 TTL 까지 막는다. 상한이 ① 판정을 idle 로 돌리고 ② sweep 이 도어벨을 눌러야
        //   대기 메일이 나간다.
        use super::super::busy::{BusyPolicy, ScriptedTurnFacts, BUSY_MAX_TURN};
        let port = Arc::new(FakeDeliveryPort::new());
        let notifier = Arc::new(RecordingIdle {
            seen: StdMutex::new(Vec::new()),
        });
        let facts = ScriptedTurnFacts::new();
        let gate = Arc::new(BusyPolicy::new(facts.clone(), notifier.clone()));
        let svc = Arc::new(MessagingService::new_gated(
            port.clone(),
            Arc::new(FakeControlPlane),
            gate.clone(),
        ));
        let (id, agent) = live("recv");
        port.set_roster(vec![agent]);

        // 비정상 턴: 턴 중으로 관측됐고 종료 신호가 오지 않는다(시각은 주입 — 실시간 30분 대기 회피).
        let t0 = Instant::now();
        facts.set_in_turn(id, 0, t0);
        svc.park_absent_for_test(
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

        // 상한 경과 → 판정이 idle 로 돌아가고 sweep 이 도어벨을 누른다.
        assert_eq!(
            gate.sweep_stale_busy(t0 + BUSY_MAX_TURN + Duration::from_secs(1)),
            1,
            "상한 초과 잔해를 깨운다"
        );
        assert_eq!(
            notifier.seen.lock().unwrap().clone(),
            vec![id],
            "깨워야 대기 메일이 나간다(판정만 뒤집으면 다음 트리거가 없다)"
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
        // id 입구(턴 종료 통지는 이름을 모른다) → canonical name 해석 후 기존 flush 경로 재사용.
        //   파킹이 없으면 no-op(잉여 idle 통지의 비용을 여기서 깎는다 — 조기 반환).
        let (svc, port, _gate) = svc_gated();
        let (recv_id, recv) = live("recv");
        port.set_roster(vec![recv]);
        svc.flush_for_agent(recv_id);
        assert!(port.injected_bodies().is_empty(), "파킹 없으면 no-op");
        // 파킹이 있으면 id → 이름으로 풀려 배달된다.
        port.set_roster(vec![]);
        svc.park_absent_for_test(
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
        svc.park_absent_for_test(
            "m1",
            ident(),
            "s",
            "ghost",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        svc.flush_for_agent(PeerId::new_v4());
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
            to_attr: None,
        }
    }

    /// 회신 발송 메타.
    fn reply_meta(in_reply_to: &str) -> SendMeta {
        SendMeta {
            request: false,
            reply_by_raw: None,
            reply_by: None,
            reply_to: Some(in_reply_to.to_string()),
            to_attr: None,
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
        svc.park_absent_for_test(
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
            vec![
                "<notice>[engram] 요청 m-60m 기한(60m) 초과 — ghost 회신 없음</notice>".to_string()
            ],
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
            .park_absent_for_test(
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
            .park_absent_for_test(
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
        svc.park_absent_for_test(
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
        svc2.park_absent_for_test(
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
        svc.park_absent_for_test(
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
        let (b_id, bob) = live("bob");
        port.set_roster(vec![alice, bob]);
        svc.park_absent_for_test(
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

        svc.park_absent_for_test(
            "m-rep",
            reply_from(b_id),
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
        svc.park_absent_for_test(
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
            .park_absent_for_test(
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
        let (b_id, bob) = live("bob");
        port.set_roster(vec![alice, bob]);
        svc.park_absent_for_test(
            "m-req",
            ident(),
            "alice",
            "bob",
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("delivered");
        svc.park_absent_for_test(
            "m-r1",
            reply_from(b_id),
            "bob",
            "alice",
            "1차",
            Entrance::Mcp,
            &reply_meta("m-req"),
        )
        .expect("delivered");
        svc.park_absent_for_test(
            "m-r2",
            reply_from(b_id),
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
        svc.park_absent_for_test(
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
        svc.park_absent_for_test(
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
    fn reply_by_timeout_delivers_notice_to_sender_exactly_once() {
        // ★spec §3 단계 4 · §1 notice 템플릿★: 기한 초과 → **발신자에게** notice. 두 번 sweep 해도 1회.
        let (svc, port) = svc();
        let (_a, alice) = live("alice");
        port.set_roster(vec![alice]);
        svc.park_absent_for_test(
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
            vec![
                "<notice>[engram] 요청 m-7f3k 기한(10m) 초과 — qa-bravo 회신 없음</notice>"
                    .to_string()
            ],
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
        let (b_id, bob) = live("bob");
        port.set_roster(vec![alice, bob]);
        svc.park_absent_for_test(
            "m-req",
            ident(),
            "alice",
            "bob",
            "해줘",
            Entrance::Mcp,
            &req_meta("1m", 60),
        )
        .expect("delivered");
        svc.park_absent_for_test(
            "m-rep",
            reply_from(b_id),
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
            to_attr: None,
        };
        svc.park_absent_for_test(
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
    fn timeout_notice_parks_in_its_own_lane_when_sender_is_absent_and_is_ledgered() {
        // ★레인 분리(round-6 — 옛 "cap 예외" 를 대체)★: 발신자 보관함이 message 로 가득 차 있어도 notice 는
        //   자기 레인에서 수용된다(회신 계약 통지가 막히면 계약이 반쪽 — ADR-0103 불변식). 그리고 조용히
        //   사라지지 않는다 — 장부에 pending 으로 남는다.
        let (svc, port) = svc();
        port.set_roster(vec![]); // alice 도 부재.
        for i in 0..100 {
            svc.park_absent_for_test(
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
            svc.park_absent_for_test(
                "over",
                ident(),
                "s",
                "alice",
                "x",
                Entrance::Mcp,
                &SendMeta::default()
            ),
            Err(SendFail::Row(FailCode::MailboxFull)),
            "message 는 cap 에서 반려(대조군)"
        );

        svc.park_absent_for_test(
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
            "message 백로그가 통지를 막지 않는다(100 message + 1 notice — 레인이 다르다)"
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

    /// ★notice 레인도 무계가 아니다(round-6)★ — 옛 구현은 notice 를 두 상한 모두에서 면제하고 그 유계를
    ///   `ledger::MAX_OPEN_REQUESTS` 가 준다고 적었는데 **거짓이었다**: `due_timeouts` 는 `notified` 를 즉시
    ///   세워 계약 자리를 비우므로, 앞선 통지가 큐에 파킹된 채로 다음 물결이 계약을 새로 열 수 있다. 이제
    ///   통지는 자기 레인 상한(20)에서 멈추고, 밀려난 **가장 오래된** 통지는 장부에 `skipped` 로 남는다.
    #[test]
    fn the_notice_lane_is_bounded_and_retires_its_oldest_to_the_ledger() {
        /// mailbox 비공개 `NOTICE_CAP` 미러(값 동기).
        // ADR-0114 결정 2: 20 → 64 상향(다중 request 의 계약별 통지가 계약 수만큼 쌓인다).
        const NOTICE_CAP_FOR_TEST: usize = 64;
        let (svc, port) = svc();
        port.set_roster(vec![]); // alice(발신자)·bob(수신자) 모두 부재 → 파킹 + 계약 오픈.
        for i in 0..(NOTICE_CAP_FOR_TEST + 1) {
            svc.park_absent_for_test(
                &format!("req{i}"),
                ident(),
                "alice",
                "bob",
                "해줘",
                Entrance::Mcp,
                &req_meta("1m", 60),
            )
            .expect("parked");
        }
        // 한 번의 sweep 이 21건을 전부 기한 초과로 걷어 통지 21건을 alice 큐에 park 한다.
        svc.sweep(Instant::now() + Duration::from_secs(61));

        assert_eq!(
            svc.parked_len("alice"),
            NOTICE_CAP_FOR_TEST,
            "통지는 자기 레인 상한에서 멈춘다(옛 구현 = cap+1 — 무계)"
        );
        let noticed: Vec<_> = svc
            .ledger_snapshot()
            .into_iter()
            .filter(|(_, from, _, _)| from == NOTICE_SENDER_LABEL)
            .collect();
        assert_eq!(
            noticed.len(),
            NOTICE_CAP_FOR_TEST + 1,
            "밀려난 통지도 장부에 남는다(조용한 유실 0 — 반려가 아니라 회수)"
        );
        // 장부 스냅샷은 삽입 순서(링 순서)라 첫 통지가 가장 오래된 것이다.
        assert_eq!(
            noticed[0].3,
            DeliveryStatus::Skipped,
            "가장 오래된 통지가 회수돼 skipped(신규 통지는 절대 반려되지 않는다)"
        );
        assert!(
            noticed[1..]
                .iter()
                .all(|(_, _, _, s)| *s == DeliveryStatus::Pending),
            "나머지는 큐에 남아 pending"
        );
    }

    #[test]
    fn parked_notice_flushes_as_notice_tag_and_marks_delivered() {
        // 파킹된 notice 가 등장 flush 로 배달될 때도 notice 태그여야 한다(kind 기반 wrap 분기).
        let (svc, port) = svc();
        port.set_roster(vec![]);
        svc.park_absent_for_test(
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
            vec!["<notice>[engram] 요청 m-9x 기한(1m) 초과 — bob 회신 없음</notice>".to_string()],
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
        svc.park_absent_for_test(
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
            vec!["<notice>[engram] 요청 m-b1 기한(1m) 초과 — ghost 회신 없음</notice>".to_string()],
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
        let alice_id = PeerId::new_v4();
        let old = LiveAgent {
            id: alice_id,
            name: "alice".to_string(),
            epoch: 0,
            turn_signal: true,
        };
        port.set_roster(vec![old]);
        svc.park_absent_for_test(
            "m-req",
            SenderIdentity {
                peer_id: alice_id,
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
            turn_signal: true,
        };
        port.set_roster(vec![renamed]);

        svc.sweep(Instant::now() + Duration::from_secs(61));
        // 도어벨 미배선 = 인라인 flush 폴백이므로 sweep 안에서 배달까지 끝난다.
        assert_eq!(
            port.injected_bodies(),
            vec![
                "<notice>[engram] 요청 m-req 기한(1m) 초과 — ghost 회신 없음</notice>".to_string()
            ],
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
            svc.park_absent_for_test(
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
        svc.park_absent_for_test(
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
            svc2.park_absent_for_test(
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
        svc.park_absent_for_test(
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
            svc.park_absent_for_test(
                "dup",
                ident(),
                "s",
                "alice",
                "2",
                Entrance::Mcp,
                &SendMeta::default()
            ),
            Err(SendFail::Reject(SendReject::IdCollision)),
            "통보도 id 충돌이면 반려"
        );
        // 회신 재사용.
        assert_eq!(
            svc.park_absent_for_test(
                "dup",
                ident(),
                "s",
                "alice",
                "3",
                Entrance::Mcp,
                &reply_meta("m-other"),
            ),
            Err(SendFail::Reject(SendReject::IdCollision)),
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
            to_attr: None,
        };
        let _ = svc.park_absent_for_test("m-x", ident(), "s", "bob", "x", Entrance::Mcp, &bad);
    }

    // ── ParkPayload 견고성(fix 4) ─────────────────────────────────────────────────────

    #[test]
    fn park_payload_decode_falls_back_on_corrupt_input_without_panicking() {
        // ★fix 4★: 어떤 입력이 와도 패닉하지 않는다 — 특히 길이 필드가 **멀티바이트 문자 중간**을 가리키는
        //   경우(`split_at` 이 패닉하는 그 지점). 실패는 전부 "속성 잃고 body 만 남는" 폴백이다.
        let good = ParkPayload {
            sender_name: "qa".to_string(),
            from: SenderIdentity {
                peer_id: PeerId::new_v4(),
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

        // 버전 태그를 모르는 값으로 바꾼 것(형식 드리프트 모사) — 태그는 항상 맨 앞이라 위치로 자른다
        //   (옛 `replacen('1', …)` 는 버전 문자가 바뀌면 엉뚱한 자리를 건드려 조용히 유효 payload 가 됐다).
        let unknown_version = {
            let mut s = encoded.clone();
            s.replace_range(0..PARK_PAYLOAD_VERSION.len(), "9");
            s
        };
        // v2 레이아웃: ver·sender_len·rb_len·rt_len·grp_len·uuid·epoch·entrance·flags 9줄 + 나머지.
        for corrupt in [
            // 헤더 자체가 없음.
            "그냥 본문".to_string(),
            // 버전 태그가 모르는 값(형식 드리프트) — 오해석 대신 폴백.
            unknown_version,
            // 길이 필드가 숫자가 아님.
            "2\nx\n0\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nmcp\n-\nbody".to_string(),
            // 길이 합이 남은 문자열보다 큼(절단).
            "2\n99\n0\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nmcp\n-\nshort".to_string(),
            // ★char 경계 중간 절단★: '한'은 3바이트인데 sender_len=1 이라 문자 중간을 가른다.
            "2\n1\n0\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nmcp\n-\n한글".to_string(),
            // reply_to 길이가 경계를 어긋나게 만드는 경우(두 번째 절단점).
            "2\n0\n1\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nmcp\n-\n한글".to_string(),
            // ★그룹 라벨 절단점(C4 — 네 번째 절단점)★: 새로 늘어난 칸도 같은 방어를 받아야 한다.
            "2\n0\n0\n0\n1\n00000000-0000-0000-0000-000000000000\n0\nmcp\n-\n한글".to_string(),
            // 헤더 줄 수 부족.
            "2\n0\n0\n0\n".to_string(),
            // ★어휘 밖 입구(fix 3)★: 예전엔 조용히 Cli 로 떨어졌다 — 이제 손상으로 보고 폴백한다.
            "2\n0\n0\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nsmtp\n-\nbody".to_string(),
            // 입구 칸이 비어 있음(형식 드리프트).
            "2\n0\n0\n0\n0\n00000000-0000-0000-0000-000000000000\n0\n\n-\nbody".to_string(),
            // 대소문자도 어휘 밖(정규화하지 않는다 — 우리가 쓴 값만 인정).
            "2\n0\n0\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nMCP\n-\nbody".to_string(),
            // ★어휘 밖 플래그(fix 3)★: 예전엔 조용히 "request 아님" 으로 떨어져 계약 속성을 잃었다.
            "2\n0\n0\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nmcp\nR\nbody".to_string(),
            "2\n0\n0\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nmcp\n\nbody".to_string(),
            "2\n0\n0\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nmcp\nr-\nbody".to_string(),
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
            svc.park_absent_for_test(
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
        svc.park_absent_for_test(
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

    /// ★H3 — 발급 경로가 **잠정 예약**을 보고 재발급한다★(기존 INTERNAL_ID_COLLISION 테스트와 같은 방식:
    /// id 생성기를 주입해 충돌을 결정적으로 만든다).
    ///
    /// 잠정 은퇴 중인 희생자는 추적에도 이력에도 없을 수 있는데(이력이 이미 밀려난 경우), 그 창에서 같은
    /// id 가 새로 발급되면 복원이 원본을 잃는다. 그래서 예약 집합이 발급 검사에 참여해야 한다.
    #[test]
    fn the_mint_path_regenerates_when_it_draws_a_provisionally_retired_id() {
        // ★7차에 창을 여는 수단이 바뀌었다(ADR-0125 — 이전이지 약화가 아니다)★: 옛 판은 `on_inject` 훅에서
        //   발급을 시험했다. 그때는 주입이 **표시와 커밋 사이**였기 때문이다. 전부-큐가 되면서 그 사이가 적재
        //   락 안으로 들어갔고 주입은 락 밖으로 나갔다 — 발급 경로(`draw_daemon_msg_id_with`)는 같은 락을
        //   잡으므로 **훅에서는 원리적으로 이 창을 볼 수 없다**. 대신 창은 실물로 남아 있다: 예약 가드가
        //   락을 놓은 채 살아 있는 구간(정산 전 — `Reservation` 헤더의 sweep 보증이 그 구간을 위한 것이다).
        //   그래서 운영과 같은 동사로 가드를 열어 두고 그 밖에서 발급을 시험한다.
        let (svc, port) = svc();
        let (boss_from, boss) = live_sender("boss");
        port.set_roster(vec![boss]);
        // 은퇴 **가능** 계약으로 채운다 — 아래 예약이 희생자를 잠정 표시하는 창을 만들어야 하므로.
        fill_open_request_cap_evictable(&svc, boss_from);
        let cap = svc.occupied_slots_for_test();
        assert!(cap > 0, "상한까지 채워졌다");
        assert!(
            svc.msg_id_in_use_for_test("cap0"),
            "전제: 희생자가 될 픽스처 계약이 존재한다"
        );

        // 예약(은퇴 표시) 후 · 커밋 전 — 이 가드가 살아 있는 동안이 잠정 창이다.
        let res = reserve_marked_contract(&svc, boss_from, "m-provisional");
        // ★잠정 창의 **직접 관측**(C2)★: 은퇴는 표시일 뿐이라 그 창에서는 **희생자 + 새 계약이 동시에**
        //   추적에 있다(cap + 1). 커밋(결말 확정) 뒤에야 희생자가 물리 제거돼 cap 으로 돌아온다 — 이 두
        //   단언이 "예약 가시성" 자체를 보므로, 그 가시성을 없애면 여기서 터진다(옛 단언은 이력 링 때문에
        //   그대로 초록이었다).
        assert_eq!(
            svc.open_request_count(),
            cap + 1,
            "잠정 창: 표시된 희생자와 새 계약이 동시에 추적에 있어야"
        );
        // 이 창에서 희생자 id 를 뽑으면 재발급돼야 한다(H3).
        let mut two = vec!["m-fresh".to_string(), "cap0".to_string()];
        let drawn = svc.draw_daemon_msg_id_with(|| two.pop().expect("draw"));
        let in_use = svc.msg_id_in_use_for_test("cap0");
        let drawn_id = drawn.id.clone();

        let mut log = RetirementLog::default();
        {
            let mut st = svc.state.lock().expect("lock");
            res.commit(&mut st, &mut log);
        }
        assert_eq!(
            svc.open_request_count(),
            cap,
            "커밋(결말 확정) 뒤에는 희생자가 물리 제거돼 상한으로 돌아온다"
        );
        // ★잠정 창을 **직접** 관측한다(리뷰 C2)★: 옛 단언은 `msg_id_in_use("cap0")` 뿐이었는데 그 값은
        //   이력 링(4096)에 남은 행 때문에도 true 라, 예약 가시성을 없애도 초록이었다(= 창을 관측하지 못함).
        //   은퇴는 **표시**일 뿐 물리 제거는 커밋에서 일어나므로, 그 표시 창에서는 희생자가 **추적 목록에
        //   그대로 있어야** 한다(위 두 단언). 아래 `in_use` 는 발급 검사 축이다.
        assert!(
            in_use,
            "잠정 은퇴 중인 cap0 은 발급 검사에서 '사용 중' 이어야(H3)"
        );
        assert_eq!(
            drawn_id, "m-fresh",
            "그래서 발급 경로가 그 id 를 버리고 새 id 로 갈아탄다"
        );
    }

    #[test]
    fn id_collision_rejects_without_side_effects() {
        // ★부작용 0 보장(SendReject::IdCollision 주석)★: 같은 id 로 두 번째 request 를 보내면 배달·파킹·
        //   장부 레코드가 **하나도** 늘지 않아야 호출자가 새 id 로 안전하게 재시도할 수 있다.
        let (svc, port) = svc();
        port.set_roster(vec![]);
        svc.park_absent_for_test(
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
        let again = svc.park_absent_for_test(
            "dup",
            ident(),
            "alice",
            "bob",
            "2",
            Entrance::Mcp,
            &req_meta("10m", 600),
        );
        assert_eq!(again, Err(SendFail::Reject(SendReject::IdCollision)));
        assert_eq!(svc.ledger_snapshot().len(), before, "장부 레코드 증가 없음");
        assert_eq!(svc.parked_len("bob"), 1, "파킹 증가 없음");
        assert!(port.injected_bodies().is_empty());
    }

    // ── D: 장부 조회 표면(message_state / open_items_for) ─────────────────────────────────

    #[test]
    fn message_state_marks_an_open_request_as_awaiting_reply_until_the_answer_lands() {
        let (svc, port) = svc();
        let (from, _s) = live_sender("alice");
        let (b_id, bob) = live("bob");
        port.set_roster(vec![bob]);
        let t0 = Instant::now();
        svc.park_absent_for_test(
            "m-req",
            from,
            "alice",
            "bob",
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("request");
        let view = svc.message_state("m-req", t0).expect("조회");
        assert!(view.awaiting_reply, "회신 전엔 대기 중");
        assert_eq!(view.rows.len(), 1);
        assert_eq!(view.rows[0].status, "delivered");

        // bob 이 회신하면 계약이 닫히고 장부도 replied 로 간다.
        // ★A6: 회신은 **그 계약의 수신자 본인 신원**으로 보낸다★(새 PeerId 를 뽑으면 자기 계약이 아니다).
        let bob_from = reply_from(b_id);
        let (_a, alice_agent) = live("alice");
        port.set_roster(vec![alice_agent]);
        svc.park_absent_for_test(
            "m-ans",
            bob_from,
            "bob",
            "alice",
            "했음",
            Entrance::Mcp,
            &reply_meta("m-req"),
        )
        .expect("회신");
        let view = svc.message_state("m-req", t0).expect("조회");
        assert!(!view.awaiting_reply, "회신 도착 후엔 대기 해제");
        assert_eq!(view.rows[0].status, "replied");
    }

    #[test]
    fn message_state_ages_are_measured_from_the_ledger_instants() {
        // 경과 초는 조회 시각(now) 기준 — 장부는 벽시계를 모르므로 상대값으로만 노출한다.
        let (svc, port) = svc();
        let (from, _s) = live_sender("alice");
        let (_b, bob) = live("bob");
        port.set_roster(vec![bob]);
        let t0 = Instant::now();
        svc.park_absent_for_test(
            "m1",
            from,
            "alice",
            "bob",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("발송");
        let view = svc
            .message_state("m1", t0 + Duration::from_secs(90))
            .expect("조회");
        assert!(
            (85..=95).contains(&view.rows[0].age_secs),
            "발송 경과 ≈ 90초: {}",
            view.rows[0].age_secs
        );
        // now 가 과거여도 패닉하지 않는다(saturating — 손으로 밀어 넣는 시각 방어).
        let view = svc
            .message_state("m1", t0 - Duration::from_secs(10))
            .expect("조회");
        assert_eq!(view.rows[0].age_secs, 0);
    }

    #[test]
    fn open_items_tags_each_direction_and_ignores_other_peoples_business() {
        let (svc, port) = svc();
        // ★신원과 로스터 항목이 **같은 PeerId** 를 공유하게 만든다(리뷰 B1)★: 의무 귀속이 id 기준이라,
        //   같은 논리 에이전트의 발신 신원과 로스터 엔트리가 다른 id 를 갖는 픽스처는 현실과 어긋난다.
        let (alice_from, alice_agent) = live_sender("alice");
        let (bob_from, bob_agent) = live_sender("bob");
        let (carol_from, _carol_agent) = live_sender("carol");
        port.set_roster(vec![bob_agent.clone()]);
        let t0 = Instant::now();

        // ① alice → ghost(부재) 통보 = 파킹(outbound_pending).
        svc.park_absent_for_test(
            "m-park",
            alice_from,
            "alice",
            "ghost",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("파킹");
        // ② alice → bob request = 회신 대기(awaiting_their_reply).
        svc.park_absent_for_test(
            "m-out",
            alice_from,
            "alice",
            "bob",
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("request");
        // ③ carol → alice request = 내가 답할 차례(reply_owed_by_me).
        port.set_roster(vec![alice_agent]);
        svc.park_absent_for_test(
            "m-in",
            carol_from,
            "carol",
            "alice",
            "이거 해줘",
            Entrance::Mcp,
            &req_meta("30m", 1800),
        )
        .expect("수신 request");
        // ④ 남의 계약(carol → bob) — 내 미결이 아니다.
        port.set_roster(vec![bob_agent]);
        svc.park_absent_for_test(
            "m-other",
            carol_from,
            "carol",
            "bob",
            "너 해라",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("남의 request");

        let items = svc.open_items_for("alice", alice_from.peer_id, t0);
        let tags: Vec<(&str, &str)> = items
            .iter()
            .map(|i| (i.direction.as_str(), i.id.as_str()))
            .collect();
        // 1차 정렬은 오래된 순인데 이 테스트는 세 건이 **같은 순간**에 생겨 경과가 전부 0이다 → 동률
        //   타이브레이크(방향 토큰 사전순 → id)가 순서를 결정한다. 그 결정성 자체가 응답 안정성의 계약이다.
        assert_eq!(
            tags,
            vec![
                ("awaiting_their_reply", "m-out"),
                ("outbound_pending", "m-park"),
                ("reply_owed_by_me", "m-in"),
            ],
            "세 방향이 태그로 구분되고 남의 계약은 빠진다: {items:?}"
        );
        assert_eq!(items[0].reply_by.as_deref(), Some("10m"), "기한 표기 원본");
        assert_eq!(items[2].from, "carol", "내가 답할 상대");
        assert!(items.iter().all(|i| !i.timed_out));
        // 남(bob) 관점에서는 자기 것만 보인다.
        let bob_items = svc.open_items_for("bob", bob_from.peer_id, t0);
        let bob_tags: Vec<(&str, &str)> = bob_items
            .iter()
            .map(|i| (i.direction.as_str(), i.id.as_str()))
            .collect();
        assert_eq!(
            bob_tags,
            vec![
                ("reply_owed_by_me", "m-other"),
                ("reply_owed_by_me", "m-out"),
            ],
            "bob 은 자기가 답할 두 건만: {bob_items:?}"
        );
    }

    #[test]
    fn open_items_drops_answered_contracts_but_keeps_timed_out_ones() {
        let (svc, port) = svc();
        let (alice_from, _s) = live_sender("alice");
        let (b_id, bob) = live("bob");
        port.set_roster(vec![bob]);
        let t0 = Instant::now();
        svc.park_absent_for_test(
            "m-a",
            alice_from,
            "alice",
            "bob",
            "1",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("req a");
        svc.park_absent_for_test(
            "m-b",
            alice_from,
            "alice",
            "bob",
            "2",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("req b");
        // m-a 에 회신이 오면 미결에서 빠진다.
        // ★A6: 회신은 **그 계약의 수신자 본인 신원**으로 보낸다★(새 PeerId 를 뽑으면 자기 계약이 아니다).
        let bob_from = reply_from(b_id);
        let (_a, alice_agent) = live("alice");
        port.set_roster(vec![alice_agent]);
        svc.park_absent_for_test(
            "m-ans",
            bob_from,
            "bob",
            "alice",
            "됨",
            Entrance::Mcp,
            &reply_meta("m-a"),
        )
        .expect("회신");
        // m-b 는 기한 초과 통지가 나가도 **여전히 미결**이다(회신은 아직 안 왔다).
        svc.sweep(t0 + Duration::from_secs(601));

        let items = svc.open_items_for("alice", alice_from.peer_id, t0 + Duration::from_secs(601));
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, vec!["m-b"], "회신 온 계약만 빠진다: {items:?}");
        assert!(
            items[0].timed_out,
            "기한 초과 사실은 플래그로 보이되 목록에는 남는다"
        );
    }

    /// ★B1 회귀 그물 — 동명 다수에서 회신 의무가 **쌍둥이에게 잘못 붙지 않는다**★.
    ///
    /// 시나리오: 같은 이름("worker")의 산 에이전트가 둘. 발신자가 exact PeerId 로 **A 에게만** request 를
    /// 건다. 옛 구현은 계약을 이름으로만 기록해, 메시지를 본 적도 없는 B 의 미결 조회가
    /// `reply_owed_by_me` 를 돌려줬다(답할 수 없는 의무를 떠안김 + A 는 자기 것인지 확신 못 함).
    #[test]
    fn a_request_addressed_by_exact_id_only_obligates_that_twin() {
        let (svc, port) = svc();
        let (boss_from, _boss) = live_sender("boss");
        let (twin_a_from, twin_a) = live_sender("worker");
        let (twin_b_from, twin_b) = live_sender("worker"); // 같은 이름, 다른 PeerId.
        port.set_roster(vec![twin_a.clone(), twin_b.clone()]);
        let t0 = Instant::now();

        // exact PeerId 로 A 만 지목(동명 다수라 이름 지목은 애초에 반려된다 — ingress AMBIGUOUS).
        svc.park_absent_for_test(
            "m-req",
            boss_from,
            "boss",
            &twin_a.id.to_string(),
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("exact-id request");

        let a_items = svc.open_items_for("worker", twin_a_from.peer_id, t0);
        assert_eq!(
            a_items
                .iter()
                .map(|i| (i.direction.as_str(), i.id.as_str()))
                .collect::<Vec<_>>(),
            vec![("reply_owed_by_me", "m-req")],
            "지목된 쌍둥이 A 는 의무를 진다: {a_items:?}"
        );
        let b_items = svc.open_items_for("worker", twin_b_from.peer_id, t0);
        assert!(
            b_items.is_empty(),
            "받은 적 없는 쌍둥이 B 에게 의무가 붙으면 안 된다(B1): {b_items:?}"
        );
        // 발신자 쪽은 그대로 회신 대기.
        let boss_items = svc.open_items_for("boss", boss_from.peer_id, t0);
        assert_eq!(
            boss_items
                .iter()
                .map(|i| i.direction.as_str())
                .collect::<Vec<_>>(),
            vec!["awaiting_their_reply"]
        );
    }

    /// 상한 은퇴 시나리오 공용 셋업 — `MAX_OPEN_REQUESTS` 개의 **은퇴 가능**(기한 없는) 계약으로 cap 을
    /// 채운다. 수신자는 미등장이라 레거시 seam 이 직접 파킹한다(발송은 전부 접수 = pending).
    /// ★오픈 계약 상한을 **은퇴 불가** 계약으로 채운다(A7 — `REQUEST_CAPACITY` 재현)★.
    ///
    /// 은퇴 가능 조건은 `notified || reply_by.is_none()` 이므로(ledger `open_request`), **기한이 있고 아직
    /// 통지되지 않은** 계약으로 채우면 희생자 후보가 하나도 없어 `Full` 이 나온다 — 그게 spec §3 항목 5 가
    /// 말하는 "은퇴 불가 계약만으로 상한이 찬 상태" 다.
    fn fill_open_request_cap(svc: &Arc<MessagingService>, from: SenderIdentity) {
        fill_open_request_cap_with(svc, from, &req_meta("10m", 600));
    }

    /// ★상한을 **한 자리 남기고** 은퇴 불가 계약으로 채운다(H2 — A3 의 진짜 창을 만들기 위한 픽스처)★.
    ///   그 남은 자리를 호출자가 은퇴 가능 계약 1건으로 메우면, 그 뒤 발송의 **두 번째** 수신자가 볼 은퇴
    ///   가능 후보는 **그 발송이 방금 연 첫 수신자 계약뿐**이 된다(= 자기잠식이 일어날 수 있는 유일한 배치).
    fn fill_open_request_cap_leaving_one_free(svc: &Arc<MessagingService>, from: SenderIdentity) {
        fill_open_request_cap(svc, from);
        // 마지막 1건을 회신으로 닫아 자리를 하나 비운다(닫힌 계약은 슬롯을 놓는다).
        let last = svc
            .open_items_for("boss", from.peer_id, Instant::now())
            .into_iter()
            .filter(|i| i.direction == Direction::AwaitingTheirReply)
            .last()
            .expect("채운 계약이 있다");
        svc.close_contract_for_test(&last.id, &last.to);
        assert!(
            svc.occupied_slots_for_test() > 0,
            "전제: 상한 근처까지 채워졌다"
        );
    }

    /// ★상한을 **은퇴 가능** 계약으로 채운다(A2/A3 — 희생자 교환을 재현)★. 기한이 없으면(= 데몬이 진 통지
    /// 빚이 없으면) 그 계약은 은퇴 후보다.
    fn fill_open_request_cap_evictable(svc: &Arc<MessagingService>, from: SenderIdentity) {
        fill_open_request_cap_with(
            svc,
            from,
            &SendMeta {
                request: true,
                ..SendMeta::default()
            },
        );
    }

    /// ★상수를 복제하지 않는다(A7 — 옛 하드코딩 512 는 자기 doc 과 모순이었다)★: 성장축은 **cap 판정이
    /// 보는 값**(`occupied_slots`)이고, 더 늘지 않는 지점이 곧 상한이다. 상한이 바뀌어도 픽스처가 따라온다.
    ///   ★`open_request_count`(= `!closed`)를 쓰면 안 된다★: 은퇴 표시된 희생자는 슬롯을 안 세지만 그
    ///   카운트에는 남아 무한히 자란다(실측 — 은퇴 가능 채움에서 루프가 끝나지 않는다).
    fn fill_open_request_cap_with(
        svc: &Arc<MessagingService>,
        from: SenderIdentity,
        meta: &SendMeta,
    ) {
        let mut i = 0usize;
        loop {
            let before = svc.occupied_slots_for_test();
            svc.park_absent_for_test(
                &format!("cap{i}"),
                from,
                "boss",
                &format!("victim{i}"),
                "q",
                Entrance::Mcp,
                meta,
            )
            .ok();
            i += 1;
            if svc.occupied_slots_for_test() == before {
                break;
            }
            assert!(i < 100_000, "상한이 관측되지 않는다 — 픽스처 전제 붕괴");
        }
    }

    /// ★round-2 리뷰 F2 회귀 그물 — 의무는 **봉투를 실제로 받은 자**를 따른다★.
    ///
    /// 시나리오(배달자/의무자 불일치): exact PeerId 로 A 에게 request → A 가 turn 중이라 **이름 큐**에 파킹
    /// (봉투는 이름 키, id 는 힌트일 뿐) → A 가 죽고 같은 이름의 B 가 등장 → flush 의 이름 폴백이 **B 에게**
    /// 배달. 옛 구현은 계약의 recipient_id 가 A 로 굳어 있어, 봉투를 받은 B 의 미결 조회가 그 의무를 못 봤다
    /// (= 받은 쪽은 "답할 게 없다" 고 읽고, A 는 이미 없다 → 계약이 아무에게도 안 보이는 유령이 된다).
    #[test]
    fn the_reply_obligation_follows_whoever_actually_received_the_envelope() {
        let (svc, port, gate) = svc_gated();
        let (boss_from, _boss) = live_sender("boss");
        let (a_from, a_agent) = live_sender("worker");
        port.set_roster(vec![a_agent.clone()]);
        gate.set_busy(a_agent.id, 0); // 턴 중 → exact-id 지목이어도 이름 큐로 파킹된다.
        let t0 = Instant::now();

        svc.park_absent_for_test(
            "m-req",
            boss_from,
            "boss",
            &a_agent.id.to_string(),
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("busy 수신자 → 파킹된 request");
        assert_eq!(svc.parked_len("worker"), 1, "전제: 이름 큐 파킹");

        // A 가 죽고 **같은 이름의 새 PeerId** B 가 등장(단일 발송은 재스폰 이어받기가 기능 — ADR-0101).
        let (b_from, b_agent) = live_sender("worker");
        port.set_roster(vec![b_agent.clone()]);
        gate.clear();
        svc.flush_for("worker", b_agent.id);
        assert_eq!(
            port.injected_bodies().len(),
            1,
            "이름 폴백으로 B 에게 배달된다(전제): {:?}",
            port.injected_bodies()
        );

        // ★핵심★: 봉투를 받은 B 가 자기 의무를 본다.
        let b_items = svc.open_items_for("worker", b_from.peer_id, t0);
        assert_eq!(
            b_items
                .iter()
                .map(|i| (i.direction.as_str(), i.id.as_str()))
                .collect::<Vec<_>>(),
            vec![("reply_owed_by_me", "m-req")],
            "실제 수신자 B 에게 의무가 옮겨져야(F2): {b_items:?}"
        );
        // 죽은 A 의 신원으로 물으면 그 의무는 더 이상 자기 것이 아니다(유령 귀속 없음).
        let a_items = svc.open_items_for("worker", a_from.peer_id, t0);
        assert!(
            a_items.is_empty(),
            "봉투를 못 받은 옛 incarnation 에는 의무가 남지 않는다: {a_items:?}"
        );
        // 발신자 쪽은 그대로 회신 대기.
        let boss_items = svc.open_items_for("boss", boss_from.peer_id, t0);
        assert_eq!(boss_items.len(), 1);
        assert_eq!(boss_items[0].direction.as_str(), "awaiting_their_reply");
    }

    #[test]
    fn a_reply_racing_its_own_injection_still_finds_the_contract() {
        // ★F4 — H3(주입 **전** 재바인딩)의 회귀 그물★.
        //
        // ★태우는 창★: 이름 큐에 파킹된 request 가 **다른 incarnation** 에게 배달될 수 있다(재스폰 이어받기 =
        //   기능, ADR-0101). 주입은 그 자체로 수신자 턴을 깨우므로 **회신이 주입 직후 바로** 올라올 수 있다.
        //   회신 매칭이 두 패스(id 우선 — A6)로 엄격해진 뒤로는 재바인딩 순서가 곧 정확성이다:
        //     - 재바인딩이 주입 **뒤**면 → 그 회신이 도착한 순간 계약은 아직 **죽은 A 의 id** 를 들고 있다.
        //       id 패스는 어긋나고 이름 폴백은 `recipient_id.is_none()` 으로 제한돼 있어 `NoMatch` — 회신이
        //       계약에서 유실되고, 나중에 **거짓 기한 통지**가 발신자에게 나간다.
        //     - 재바인딩이 주입 **전**이면 → 같은 회신이 자기 계약을 찾아 닫는다.
        // ★레이스를 어떻게 결정적으로 재현하나★: `on_inject` hook 은 write 직전에 불린다. 그 안에서 회신을
        //   발송하면 "주입과 동시에 도착한 회신" 을 **정확히** 모사한다(sleep·스레드 없이 결정적).
        let (svc, port, gate) = svc_gated();
        let (boss_from, boss) = live_sender("boss");
        let (a_from, a_agent) = live_sender("worker");
        port.set_roster(vec![boss.clone(), a_agent.clone()]);
        gate.set_busy(a_agent.id, 0); // 턴 중 → exact-id 지목이어도 이름 큐로 파킹된다.
        let t0 = Instant::now();

        svc.park_absent_for_test(
            "m-req",
            boss_from,
            "boss",
            &a_agent.id.to_string(),
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("busy 수신자 → 파킹된 request");
        assert_eq!(svc.parked_len("worker"), 1, "전제: 이름 큐 파킹");
        assert_eq!(svc.open_request_count(), 1, "전제: 계약 1건 열림");

        // A 가 죽고 **같은 이름의 새 PeerId** B 가 등장.
        let (_b_from, b_agent) = live_sender("worker");
        let b_id = b_agent.id;
        port.set_roster(vec![boss.clone(), b_agent.clone()]);
        gate.clear();

        // hook: **B 에게 주입되는 그 순간** B 가 회신한다(idx 0 = flush 의 주입. 회신 자신의 주입은 idx 1 이라
        //   재진입하지 않는다 — 재진입하면 무한 재귀가 된다).
        {
            let svc2 = svc.clone();
            port.set_on_inject(Arc::new(move |idx| {
                if idx != 0 {
                    return;
                }
                svc2.handle_send(
                    "m-reply",
                    reply_from(b_id),
                    "worker",
                    &["boss".to_string()],
                    "다 했다",
                    Entrance::Mcp,
                    &reply_meta("m-req"),
                )
                .expect("회신 발송은 성공해야");
            }));
        }
        svc.flush_for("worker", b_id);

        let bodies = port.injected_bodies();
        assert_eq!(
            bodies.len(),
            2,
            "flush 배달 1 + 회신 1(전제 — 회신이 실제로 나갔다): {bodies:?}"
        );
        // 회신이 **바깥 write 가 기록되기도 전에** 들어간다(hook 은 write 직전 지점) — 즉 이 그물은 창의
        //   가장 이른 순간을 태운다. 그래서 순서 단언은 인덱스가 아니라 존재로 한다.
        assert!(
            bodies.iter().any(|b| b.contains(r#"in-reply-to="m-req""#)),
            "회신 봉투가 실제로 나갔어야: {bodies:?}"
        );
        assert!(
            bodies.iter().any(|b| b.contains(r#"type="request""#)),
            "파킹돼 있던 request 도 배달됐어야: {bodies:?}"
        );

        // ★핵심 ①★: 주입과 동시에 온 회신이 자기 계약을 찾아 닫았다.
        assert_eq!(
            svc.open_request_count(),
            0,
            "주입 전 재바인딩(H3)이 없으면 이 회신은 NoMatch 로 빗나가 계약이 열린 채 남는다"
        );
        assert!(
            svc.open_items_for("worker", b_id, t0).is_empty(),
            "회신한 B 에게 남은 의무도 없다"
        );
        assert!(
            svc.open_items_for("boss", boss_from.peer_id, t0).is_empty(),
            "발신자도 더 기다리지 않는다"
        );

        // ★핵심 ②★: 기한을 넘겨도 **거짓 기한 통지가 없다**(유실된 회신의 대가가 여기서 드러난다).
        svc.sweep(t0 + Duration::from_secs(700));
        assert_eq!(
            port.injected_bodies().len(),
            2,
            "닫힌 계약엔 기한 통지가 없다: {:?}",
            port.injected_bodies()
        );
        assert_eq!(svc.parked_len("boss"), 0, "통지가 파킹으로 새지도 않는다");
        let _ = a_from;
    }

    /// ★B2 — 뷰가 완전성을 단언하는 경우와 유보하는 경우★.
    #[test]
    fn message_state_reports_whether_its_row_list_can_be_trusted_as_complete() {
        let (svc, port) = svc();
        let (from, _s) = live_sender("alice");
        let (_b, bob) = live("bob");
        port.set_roster(vec![bob]);
        let t0 = Instant::now();
        svc.park_absent_for_test(
            "m1",
            from,
            "alice",
            "bob",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("발송");
        let view = svc.message_state("m1", t0).expect("조회");
        assert!(
            !view.may_be_truncated,
            "evict 전이면 rows 가 전부라고 단언할 수 있다"
        );
    }

    #[test]
    fn open_items_is_empty_when_nothing_is_outstanding() {
        let (svc, port) = svc();
        let (from, _s) = live_sender("alice");
        let (_b, bob) = live("bob");
        port.set_roster(vec![bob]);
        svc.park_absent_for_test(
            "m1",
            from,
            "alice",
            "bob",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("배달 완료");
        assert!(
            svc.open_items_for("alice", from.peer_id, Instant::now())
                .is_empty(),
            "배달된 통보는 미결이 아니다"
        );
    }

    // ══════════════════════════════════════════════════════════════════════════════════════════
    // S18 발송 개편 수용 기준(spec §7) — ADR-0111/0112/0114
    //
    // ★이 절이 정본인 계약★: 부재·동명 = 수신자별 실패 행 · 다중 수신자 fan-out(스냅샷 1장·경로 1벌) ·
    //   `@all` = 나 빼고 전원(매크로) · 부분 진행 · 봉투 `to` 동결 · 다중 request N계약 · 결박/회수 폐지.
    //   여기 단언을 느슨하게 만드는 수정은 곧 **계약 변경**이다(사용자 재가 없이 완화 금지).
    // ══════════════════════════════════════════════════════════════════════════════════════════

    /// 행을 `(to, status, code)` 튜플로 — 순서·상태·코드를 한 번에 단언하기 위한 축약.
    fn rows(v: &[RecipientResult]) -> Vec<(String, SendStatus, Option<FailCode>)> {
        v.iter().map(|r| (r.to.clone(), r.status, r.code)).collect()
    }

    /// 통보 발송 1회(입구가 이미 트림·분해를 끝낸 토큰 목록을 그대로 넘긴다).
    fn send(
        svc: &Arc<MessagingService>,
        msg_id: &str,
        from: SenderIdentity,
        sender_name: &str,
        to: &[&str],
    ) -> Result<Vec<RecipientResult>, SendReject> {
        send_body(svc, msg_id, from, sender_name, to, "body")
    }

    /// 본문을 지정하는 통보 발송 — **주입 순서를 바이트로 단언**해야 할 때 쓴다(같은 본문이면 배치 안의
    ///   두 편지를 구별할 수 없다).
    fn send_body(
        svc: &Arc<MessagingService>,
        msg_id: &str,
        from: SenderIdentity,
        sender_name: &str,
        to: &[&str],
        body: &str,
    ) -> Result<Vec<RecipientResult>, SendReject> {
        let list: Vec<String> = to.iter().map(|t| t.to_string()).collect();
        svc.handle_send(
            msg_id,
            from,
            sender_name,
            &list,
            body,
            Entrance::Mcp,
            &SendMeta::default(),
        )
    }

    #[test]
    fn an_absent_recipient_is_a_failed_row_that_never_parks_but_always_ledgers() {
        // ★spec §7 "없는 이름 입구 반려"(4차 개정)★: **로스터·프로필 둘 다에** 없는 이름 → 응답 행
        //   `failed`+`RECIPIENT_NOT_FOUND` · **파킹 큐에 안 실림**(장부에 `pending` 레코드 없음) · 그래도
        //   **종점 행은 남는다**(§5 "실패 수신자도 장부에"). 그래서 `messages{id}` 행수 = 발신 응답 행수라
        //   `may_be_truncated` 오탐이 사라진다.
        // ★4차 보정★: 여기 "없음" 은 **잠든 세션도, 턴 신호 없는 산 세션도 아니다** — 잠듦(프로필 실재)은
        //   파킹 `pending` 이고(`a_dormant_recipient_is_parked_*`), 턴 신호 없는 산 세션은 게이트 없이
        //   **배달**된다(`a_live_agent_without_a_turn_signal_*`). 이 테스트는 그 둘과 **배타적**이어야 한다.
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        port.set_roster(vec![me]);

        let out = send(&svc, "m1", from, "alice", &["ghost"]).expect("행 응답(전체 반려 아님)");
        assert_eq!(
            rows(&out),
            vec![(
                "ghost".to_string(),
                SendStatus::Failed,
                Some(FailCode::RecipientNotFound)
            )],
            "부재 1인 발송의 답은 error 가 아니라 failed 행 1개다(경로 1벌 — spec §5)"
        );
        assert!(out[0].hint.is_some(), "실패 행엔 code + hint 필수(spec §6)");

        assert_eq!(svc.parked_len("ghost"), 0, "파킹 큐에 안 실린다");
        assert!(
            svc.ledger_statuses("m1")
                .iter()
                .all(|s| *s != DeliveryStatus::Pending),
            "장부에 pending 상태 레코드가 없어야(파킹한 적 없음)"
        );
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Failed],
            "종점 `failed` 행 1개만 남는다"
        );

        // 조회 행수 = 발신 응답 행수(잘림 오탐 없음).
        let view = svc
            .message_state("m1", Instant::now())
            .expect("실패 행도 조회된다");
        assert_eq!(view.rows.len(), out.len());
        assert!(!view.may_be_truncated, "기대 행수와 남은 행수가 같다");
        assert_eq!(view.rows[0].status, "failed");
    }

    #[test]
    fn one_roster_snapshot_drives_the_whole_fan_out_and_at_all_uses_the_direct_path() {
        // ★spec §7 "다중 수신자 fan-out"★: ① 로스터 스냅샷 **1장**으로 전원 판정 ② `@all` 펼침이 직접
        //   지목과 **같은 코드 경로**임을 동치 입력의 동일 결과로 단언한다(그룹 전용 분기 부활 방지).
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_b, bob) = live("bob");
        let (_c, carol) = live("carol");
        port.set_roster(vec![me, bob.clone(), carol.clone()]);

        let via_all = send(&svc, "m-all", from, "alice", &["@all"]).expect("ok");
        let via_names = send(&svc, "m-names", from, "alice", &["bob", "carol"]).expect("ok");
        assert_eq!(
            rows(&via_all),
            rows(&via_names),
            "`@all` 펼침 결과와 직접 지목이 **같은 결말**이어야(경로 1벌 — ADR-0111 결정 4)"
        );
        assert_eq!(
            rows(&via_all)
                .iter()
                .map(|(t, s, _)| (t.clone(), *s))
                .collect::<Vec<_>>(),
            vec![
                ("bob".to_string(), SendStatus::Delivered),
                ("carol".to_string(), SendStatus::Delivered),
            ]
        );

        // ★스냅샷 1장★: 발송 1회가 로스터를 **한 번만** 뜬다(수신자별 재조회 = 반쪽 판정의 재발 경로).
        let before = port.roster_calls();
        let _ = send(&svc, "m-snap", from, "alice", &["@all", "bob"]).expect("ok");
        assert_eq!(
            port.roster_calls() - before,
            1,
            "수신자 수와 무관하게 로스터 조회는 발송당 1회(ADR-0111 결정 2)"
        );
    }

    #[test]
    fn at_all_excludes_the_sender_but_explicit_self_naming_still_delivers() {
        // ★spec §7 "펼침 발신자 제외"★ 쌍: 생존자가 발신자뿐일 때
        //   `["@all"]` → 최종 집합 0명 → `GROUP_EMPTY` 전체 반려 **vs**
        //   `["@all", "<자기이름>"]` → `@all` 기여 0 + 명시 지목 1행 배달(ADR-0114 결정 3).
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        port.set_roster(vec![me]);

        assert_eq!(
            send(&svc, "m1", from, "alice", &["@all"]),
            Err(SendReject::GroupEmpty),
            "발신자만 살아 있으면 @all 은 최종 집합 0명 → 전체 반려"
        );
        assert!(
            svc.ledger_snapshot().is_empty(),
            "전체 반려는 부작용 0(장부 레코드 없음)"
        );

        let out = send(&svc, "m2", from, "alice", &["@all", "alice"]).expect("반려 아님");
        assert_eq!(
            rows(&out),
            vec![("alice".to_string(), SendStatus::Delivered, None)],
            "직접 지목 자기발송은 살아남는다(제외는 **펼침에만** 적용 — spec §4)"
        );

        // ★5차 경계(ADR-0121 결정 1)★: `@all` 은 명부를 보므로 **잠든 이름 하나만 있어도** 최종 집합이
        //   비지 않는다 — 그 몫은 파킹된다. `@here` 는 산 명단만 보니 여전히 `GROUP_EMPTY` 다. 이 대비가
        //   없으면 "발신자뿐 → 반려" 를 어휘 구분 없이 읽는 다음 세션이 두 어휘를 다시 합친다.
        port.set_dormant(&["sleepy"]);
        let all = send(&svc, "m3", from, "alice", &["@all"]).expect("잠든 이름이 있으면 반려 아님");
        assert_eq!(
            rows(&all),
            vec![("sleepy".to_string(), SendStatus::Pending, None)],
            "@all 은 잠든 이름을 펼쳐 파킹한다(발신자는 여전히 제외): {all:?}"
        );
        assert_eq!(
            send(&svc, "m4", from, "alice", &["@here"]),
            Err(SendReject::GroupEmpty),
            "@here 는 잠든 이름을 보지 않으므로 여전히 전체 반려"
        );
    }

    #[test]
    fn an_explicit_dormant_token_and_at_all_fold_into_one_row() {
        // ★spec §5 해석 순서 ④(중복 제거 = 수신자 1명 = 배달 1회·결과 1행)이 **새 `@all` 에도 성립**★:
        //   `@all` 이 잠든 이름을 펼치게 된 뒤로 "명시 지목 + 펼침" 이 같은 잠든 이름을 두 번 낼 수 있다.
        //   행이 갈리면 파킹이 2건 생기고 발신자는 한 사람에게 두 번 보낸 셈이 된다.
        // ★행 위치는 명시 지목 자리★(먼저 나온 것을 남긴다 — 펼침 결과보다 항상 앞선다).
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_b, bob) = live("bob");
        port.set_roster(vec![me, bob]);
        port.set_dormant(&["sleepy"]);

        let out = send(&svc, "m1", from, "alice", &["sleepy", "@all"]).expect("반려 아님");
        assert_eq!(
            rows(&out),
            vec![
                ("sleepy".to_string(), SendStatus::Pending, None),
                ("bob".to_string(), SendStatus::Delivered, None),
            ],
            "잠든 이름은 명시 지목 자리에 **1행**만 남는다(펼침분은 흡수): {out:?}"
        );
        assert_eq!(
            svc.parked_len("sleepy"),
            1,
            "파킹도 1건이어야(이중 배달 금지)"
        );
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Pending, DeliveryStatus::Delivered],
            "장부 행수 = 응답 행수(2행)"
        );
    }

    #[test]
    fn at_all_reports_a_duplicate_live_name_as_an_ambiguous_row_and_still_delivers_the_rest() {
        // ★spec §7 "`@all` 동명 예외"(ADR-0114 결정 4 과도기 규칙)★: 임의 선택도, 양쪽 배달도 아니다 —
        //   그 이름만 `RECIPIENT_AMBIGUOUS` 실패 행이고 나머지 전원은 그대로 받는다.
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_d1, dup1) = live("dup");
        let (_d2, dup2) = live("dup");
        let (_z, zed) = live("zed");
        port.set_roster(vec![me, dup1, dup2, zed]);

        let out = send(&svc, "m1", from, "alice", &["@all"]).expect("ok");
        assert_eq!(
            rows(&out),
            vec![
                (
                    "dup".to_string(),
                    SendStatus::Failed,
                    Some(FailCode::RecipientAmbiguous)
                ),
                ("zed".to_string(), SendStatus::Delivered, None),
            ],
            "동명은 실패 행 1줄(두 줄로 부풀지 않는다) · 나머지는 배달"
        );
        assert_eq!(
            port.injected_bodies().len(),
            1,
            "동명에겐 한 통도 안 나간다"
        );
    }

    #[test]
    fn mixed_to_folds_duplicates_once_and_row_order_is_deterministic() {
        // ★spec §7 "혼용 `to` 중복 제거·행 순서"★: 겹친 수신자는 **배달 1회·결과 1행**이고 그 행 위치는
        //   **명시 지목 자리**를 따른다. 행 순서 = ① 명시 토큰 입력 순 ② 펼침 결과 이름 사전순.
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_b, bob) = live("bob");
        let (_c, carol) = live("carol");
        let (_z, zed) = live("zed");
        port.set_roster(vec![me, bob, carol, zed]);

        let out = send(&svc, "m1", from, "alice", &["zed", "@all"]).expect("ok");
        assert_eq!(
            out.iter().map(|r| r.to.as_str()).collect::<Vec<_>>(),
            vec!["zed", "bob", "carol"],
            "명시 토큰(zed) 먼저 → 그 뒤 펼침 결과 사전순(bob, carol). zed 는 한 줄뿐"
        );
        assert_eq!(
            out.len(),
            3,
            "@all 에 이미 든 이름을 겹쳐 적어도 행이 늘지 않는다"
        );
        assert_eq!(port.injected_bodies().len(), 3, "겹친 수신자에게 배달 1회");
    }

    #[test]
    fn a_name_typo_fails_only_its_own_row_while_live_recipients_still_receive() {
        // ★spec §7 "층위 분리" 의 **이름 축**(ADR-0114 결정 3)★: 이름의 부재 = 런타임 상태 → 그 행만 실패.
        //   (`@`오타 = 주소 공간 오류 → 전체 반려 축은 통합 테스트 `c3_invalid_contract_args…` 가 커버.)
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_b, bob) = live("bob");
        port.set_roster(vec![me, bob]);

        let out = send(&svc, "m1", from, "alice", &["없는이름", "bob"]).expect("부분 진행");
        assert_eq!(
            rows(&out),
            vec![
                (
                    "없는이름".to_string(),
                    SendStatus::Failed,
                    Some(FailCode::RecipientNotFound)
                ),
                ("bob".to_string(), SendStatus::Delivered, None),
            ]
        );
        assert_eq!(port.injected_bodies().len(), 1, "산 수신자에겐 그대로 간다");

        // 대조군 — `@`오타는 **전체 반려**라 산 수신자에게도 안 간다(부작용 0).
        let before = port.injected_bodies().len();
        assert!(matches!(
            send(&svc, "m2", from, "alice", &["@typo", "bob"]),
            Err(SendReject::GroupNotFound { .. })
        ));
        assert_eq!(port.injected_bodies().len(), before, "전체 반려는 배달 0");
    }

    #[test]
    fn partial_progress_mixes_delivered_pending_and_failed_in_one_send() {
        // ★spec §7 "부분 진행"★: 세 결말이 한 발송에 공존한다. 전원 실패도 `{id, results}` shape 유지.
        let (svc, port, gate) = svc_gated();
        let (from, me) = live_sender("alice");
        let (b_id, bob) = live("bob");
        let (_c, carol) = live("carol");
        port.set_roster(vec![me, bob, carol]);
        gate.set_busy(b_id, 0); // bob 은 턴 진행 중 → 파킹

        let out = svc
            .handle_send(
                "m1",
                from,
                "alice",
                &["bob".to_string(), "carol".to_string(), "ghost".to_string()],
                "body",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("부분 진행은 Ok");
        assert_eq!(
            rows(&out),
            vec![
                ("bob".to_string(), SendStatus::Pending, None),
                ("carol".to_string(), SendStatus::Delivered, None),
                (
                    "ghost".to_string(),
                    SendStatus::Failed,
                    Some(FailCode::RecipientNotFound)
                ),
            ]
        );

        // 전원 실패도 **shape 그대로**(전체 반려로 승격하지 않는다).
        let all_failed = send(&svc, "m2", from, "alice", &["ghost1", "ghost2"]).expect("Ok shape");
        assert!(
            all_failed.iter().all(|r| r.status == SendStatus::Failed),
            "전 행 failed"
        );
        assert_eq!(all_failed.len(), 2);
    }

    #[test]
    fn a_full_mailbox_fails_only_that_row_without_any_reclaim_attempt() {
        // ★spec §7 "다중 수신자 `MAILBOX_FULL` 은 행 실패로만"(ADR-0114 결정 1 — 회수 시도 없음)★.
        //   ★회수 폐지의 관측 가능한 증거★: cap 을 채운 큐의 **가장 오래된 항목이 그대로 남는다**(옛 압력
        //   회수는 그걸 걷어내고 신규를 수용했다) + 그 수신자만 실패 행이고 나머지는 배달된다.
        let (svc, port, gate) = svc_gated();
        let (from, me) = live_sender("alice");
        let (f_id, full) = live("full");
        let (_c, carol) = live("carol");
        port.set_roster(vec![me, full, carol]);
        gate.set_busy(f_id, 0); // 계속 busy → 파킹으로 큐를 채운다

        for i in 0..100 {
            let out = send(&svc, &format!("f{i}"), from, "alice", &["full"]).expect("cap 이내");
            assert_eq!(out[0].status, SendStatus::Pending, "{i}번째");
        }
        assert_eq!(svc.parked_len("full"), 100);
        let oldest_before = svc.parked_msg_ids("full")[0].clone();

        let out = send(&svc, "over", from, "alice", &["full", "carol"]).expect("부분 진행");
        assert_eq!(
            rows(&out),
            vec![
                (
                    "full".to_string(),
                    SendStatus::Failed,
                    Some(FailCode::MailboxFull)
                ),
                ("carol".to_string(), SendStatus::Delivered, None),
            ],
            "가득 찬 한 수신자가 나머지 배달을 막지 않는다(전체 반려 승격 없음)"
        );
        assert_eq!(svc.parked_len("full"), 100, "큐 길이 불변");
        assert_eq!(
            svc.parked_msg_ids("full")[0],
            oldest_before,
            "가장 오래된 항목이 그대로 있다 = **회수 시도 없음**(ADR-0114 결정 1)"
        );
        assert_eq!(
            svc.ledger_statuses("over"),
            vec![DeliveryStatus::Failed, DeliveryStatus::Delivered],
            "실패 수신자도 장부 종점 행을 남긴다"
        );
    }

    #[test]
    fn a_multi_recipient_request_opens_one_independent_contract_per_recipient() {
        // ★spec §7 "다중 request"(ADR-0111 결정 5)★: N 수신자 = 독립 계약 N개(키 = (메시지 id, 수신자)) ·
        //   한 명이 회신하면 **그 계약만** 닫히고 나머지는 오픈 유지 · 기한 통지는 **계약별 1건**.
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (b_id, bob) = live("bob");
        let (c_id, carol) = live("carol");
        port.set_roster(vec![me, bob, carol]);

        let out = svc
            .handle_send(
                "m-req",
                from,
                "alice",
                &["bob".to_string(), "carol".to_string()],
                "해줘",
                Entrance::Mcp,
                &req_meta("10m", 600),
            )
            .expect("ok");
        assert!(out.iter().all(|r| r.status == SendStatus::Delivered));
        assert_eq!(svc.open_request_count(), 2, "수신자마다 계약 1건");

        // bob 이 회신 → **bob 의 계약만** 닫힌다.
        let bob_from = SenderIdentity {
            peer_id: b_id,
            epoch: 0,
        };
        svc.handle_send(
            "m-rep",
            bob_from,
            "bob",
            &["alice".to_string()],
            "했음",
            Entrance::Mcp,
            &reply_meta("m-req"),
        )
        .expect("회신 배달");
        assert_eq!(
            svc.open_request_count(),
            1,
            "전체회신 없음 — carol 계약은 그대로 열려 있다"
        );

        // 기한 초과 → 남은 계약 **1건에 대해서만** 통지(계약별 1건, 병합 없음 — ADR-0114 결정 2).
        svc.sweep(Instant::now() + Duration::from_secs(601));
        let notices: Vec<_> = svc
            .ledger_snapshot()
            .into_iter()
            .filter(|(_, from, _, _)| from == NOTICE_SENDER_LABEL)
            .collect();
        assert_eq!(notices.len(), 1, "회신한 계약엔 통지가 없다: {notices:?}");
        let _ = c_id;
    }

    #[test]
    fn the_envelope_to_attribute_is_frozen_at_send_time_and_survives_the_flush() {
        // ★spec §7/§1 "봉투 `to` 속성"★: ① 수용 판정된 수신자 **2인 이상**일 때만 노출 ② 값은 발송 시점에
        //   **동결**돼 파킹분이 한참 뒤 주입돼도 그대로 ③ **실패 토큰 제외** ④ 나열은 **입력 표기 순**
        //   (`@`주소는 펼치지 않고 토큰 그대로).
        let (svc, port, gate) = svc_gated();
        let (from, me) = live_sender("alice");
        let (b_id, bob) = live("bob");
        let (_c, carol) = live("carol");
        port.set_roster(vec![me, bob, carol]);
        gate.set_busy(b_id, 0); // bob 만 파킹 — 그 봉투는 flush 때 조립된다

        let out = svc
            .handle_send(
                "m1",
                from,
                "alice",
                &[
                    "@all".to_string(),
                    "ghost".to_string(), // 실패 토큰 — `to` 에서 제외돼야
                ],
                "전원 대기",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("ok");
        assert_eq!(
            rows(&out),
            vec![
                (
                    "ghost".to_string(),
                    SendStatus::Failed,
                    Some(FailCode::RecipientNotFound)
                ),
                ("bob".to_string(), SendStatus::Pending, None),
                ("carol".to_string(), SendStatus::Delivered, None),
            ]
        );
        // 즉시 배달분(carol)의 봉투 — `@all` 토큰 그대로, 실패한 `ghost` 는 없다.
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="alice" to="@all">전원 대기</message>"#.to_string()],
            "수용 2인 → to 노출 · @주소는 펼치지 않고 토큰 그대로 · 실패 토큰 제외"
        );

        // 파킹분(bob)이 나중에 배달돼도 **같은 `to`**(재계산 금지 — 동결).
        gate.clear();
        svc.flush_for("bob", b_id);
        assert_eq!(
            port.injected_bodies().len(),
            2,
            "파킹분이 flush 로 배달됐다"
        );
        assert_eq!(
            port.injected_bodies()[1],
            r#"<message from="alice" to="@all">전원 대기</message>"#,
            "발송 순간 표기 그대로(park payload 가 동결값을 flush 까지 나른다)"
        );

        // 대조군 — 수용 1인이면 속성 자체가 없다(혼자 받은 편지).
        drop(svc);
        let (svc2, port2) = super::tests::svc();
        let (from2, me2) = live_sender("alice");
        let (_z, zed) = live("zed");
        port2.set_roster(vec![me2, zed]);
        send(&svc2, "m2", from2, "alice", &["zed"]).expect("ok");
        assert_eq!(
            port2.injected_bodies(),
            vec![r#"<message from="alice">body</message>"#.to_string()],
            "수용 1인 → to 속성 생략"
        );
    }

    #[test]
    fn the_to_attribute_lists_tokens_in_input_notation_order() {
        // ★입력 표기 순★ — `results[]` 행 순서 축(명시 → 펼침 사전순)과 **별개**다(spec §1).
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_b, bob) = live("bob");
        let (_c, carol) = live("carol");
        port.set_roster(vec![me, bob, carol]);

        send(&svc, "m1", from, "alice", &["@all", "carol"]).expect("ok");
        assert!(
            port.injected_bodies()[0].contains(r#"to="@all,carol""#),
            "발신자가 쓴 순서 그대로(@all 먼저, 그 다음 명시 이름): {:?}",
            port.injected_bodies()
        );
    }

    // ── 되살린 회귀(옛 C4 스위트가 들고 있던 load-bearing 축) ────────────────────────────────

    #[test]
    fn the_cap_denominator_counts_the_batch_that_left_the_queue_during_a_flush() {
        // ★F1 회귀(cap = 큐 + in-flight)★: flush 가 락 밖으로 들고 나간 배치도 그 수신자 앞 미결이다 —
        //   그 창에서 큐만 세면 신규 유입이 cap 만큼 통째로 통과해 사이클마다 큐가 자란다(무계 성장).
        //   ★관측 방법★: inject hook 안에서(= 배치가 나가 있는 순간) 같은 수신자에게 발송해 `MAILBOX_FULL`
        //   행이 나오는지 본다. 옛 구멍에선 "큐가 비었다" 며 수용됐다.
        let (svc, port, gate) = svc_gated();
        let (from, me) = live_sender("alice");
        let (r_id, recv) = live("recv");
        port.set_roster(vec![me, recv]);
        gate.set_busy(r_id, 0);
        for i in 0..100 {
            send(&svc, &format!("p{i}"), from, "alice", &["recv"]).expect("cap 이내 파킹");
        }
        assert_eq!(svc.parked_len("recv"), 100);

        // flush 도중(배치가 락 밖으로 나가 큐가 비어 보이는 순간) 동시 발송.
        gate.clear();
        let during: Arc<StdMutex<Vec<SendStatus>>> = Arc::new(StdMutex::new(Vec::new()));
        let (svc_h, from_h, seen_h) = (svc.clone(), from, during.clone());
        port.set_on_inject(Arc::new(move |idx| {
            if idx != 0 {
                return;
            }
            let out = svc_h
                .handle_send(
                    "concurrent",
                    from_h,
                    "alice",
                    &["recv".to_string()],
                    "b",
                    Entrance::Mcp,
                    &SendMeta::default(),
                )
                .expect("행 응답");
            seen_h.lock().unwrap().push(out[0].status);
        }));
        svc.flush_for("recv", r_id);

        assert_eq!(
            during.lock().unwrap().as_slice(),
            &[SendStatus::Failed],
            "배치가 나가 있는 동안의 신규 유입은 분모(큐 + in-flight)에 걸려 반려된다(F1)"
        );
        assert_eq!(
            svc.ledger_statuses("concurrent"),
            vec![DeliveryStatus::Failed],
            "반려도 실패 행으로 장부에 남는다(ADR-0111 — 옛 '반려는 장부 미기록' 과 반전)"
        );
    }

    #[test]
    fn a_second_flush_defers_while_a_batch_is_in_flight_and_the_settlement_re_rings_the_doorbell() {
        // ★도어벨/유예 회귀(round-7/8)★: 같은 수신자에 대한 flush 는 겹쳐 돌지 않는다(순서 역전 방지).
        //   물러난 쪽의 깨우기는 **증발하면 안 되고**(lost wakeup), 영수증을 쥔 쪽이 정산하며 되울린다.
        let (svc, port, gate, bell) = svc_gated_with_doorbell();
        let (from, me) = live_sender("alice");
        let (r_id, recv) = live("recv");
        port.set_roster(vec![me, recv]);
        gate.set_busy(r_id, 0);
        for i in 0..2 {
            send(&svc, &format!("p{i}"), from, "alice", &["recv"]).expect("파킹");
        }
        gate.clear();
        bell.take(); // 파킹 도어벨은 여기서 관심 밖 — 유예/재타 축만 본다.

        // 배치가 락 밖으로 나가 있는 순간(inject hook) 두 번째 flush 를 밀어 넣는다 → 유예돼야 한다.
        let svc_h = svc.clone();
        port.set_on_inject(Arc::new(move |idx| {
            if idx == 0 {
                svc_h.flush_for("recv", r_id);
            }
        }));
        svc.flush_for("recv", r_id);

        assert_eq!(
            port.injected_bodies().len(),
            2,
            "겹쳐 돌지 않으므로 배치는 한 번만, 그러나 전부 나간다"
        );
        assert!(
            bell.take().contains(&r_id),
            "유예된 깨우기는 정산 시 **되울려야** 한다(lost wakeup 금지)"
        );
    }

    // ══════════════════════════════════════════════════════════════════════════════════════════
    // /review code deep — fix round 1 회귀 그물(A2·A3·A5·A6·A7·A8 · C2·C4)
    //   ★A1(회신 계약 처리)은 4차에서 **재가된 규칙으로 교체**됐다★ — 그 자리는 아래 두 테스트가 지킨다
    //   (도달 불가 확정 → `reply_failed` / 일시 사유 → 오픈 유지 + 재시도 닫힘). ADR-0116 결정 2 · ADR-0118.
    // ══════════════════════════════════════════════════════════════════════════════════════════

    #[test]
    fn a_reply_to_a_requester_that_exists_nowhere_fails_the_contract_and_stops_the_timeout() {
        // ★spec §7 회신 계약 규칙 ③(ADR-0116 결정 2 · 사용자 결정 2026-07-30)★: 요청자 이름이 로스터·산
        //   명단·프로필 **전부에 없으면** 회신은 갈 곳이 없다(row = failed/`RECIPIENT_NOT_FOUND`). 그 계약은
        //   `reply_failed` **실패 종결**이다 — 미결 조회에서 사라지고 기한 스윕이 발화하지 않으며 512 계수도
        //   줄고, 이력은 남는다.
        // ★이 테스트는 옛 A1 잠정안(사유 무관 오픈 유지)을 **뒤집은 것**이다★: 옛 단언("계약이 열린 채로
        //   남고 기한 통지가 나간다")은 재가된 규칙과 정반대라, 그 초록을 근거로 옛 semantics 를 유지하면
        //   갈 곳 없는 좀비 계약이 상한까지 남는다(ADR-0116 거부한 대안 "계약 열어두기").
        let (svc, port) = svc();
        let (boss_from, boss) = live_sender("boss");
        let (w_id, worker) = live("worker");
        port.set_roster(vec![boss.clone(), worker]);

        svc.handle_send(
            "m-req",
            boss_from,
            "boss",
            &["worker".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("배달");
        assert_eq!(svc.open_request_count(), 1);
        let slots_before = svc.occupied_slots_for_test();

        // 요청자(boss)가 죽고 **프로필도 없다** — 어느 소스에도 없는 이름이 된다.
        port.set_roster(vec![LiveAgent {
            id: w_id,
            name: "worker".to_string(),
            epoch: 0,
            turn_signal: true,
        }]);
        let worker_from = SenderIdentity {
            peer_id: w_id,
            epoch: 0,
        };
        let rows_out = svc
            .handle_send(
                "m-rep",
                worker_from,
                "worker",
                &["boss".to_string()],
                "했음",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("행 응답");
        assert_eq!(
            rows_out[0].code,
            Some(FailCode::RecipientNotFound),
            "없는 요청자에게는 회신이 가지 않는다"
        );
        assert_eq!(
            svc.open_request_count(),
            0,
            "도달 불가 확정 회신은 계약을 reply_failed 로 **종결**한다(ADR-0116 결정 2)"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-req", "worker"),
            Some("reply_failed"),
            "종점 어휘는 replied 가 아니다 — 회신이 성립했다고 주장하지 않는다(spec §6 축 구분)"
        );
        assert_eq!(
            svc.occupied_slots_for_test(),
            slots_before - 1,
            "512 계수에서 빠진다(ADR-0118 결정 4 — 좀비가 상한을 먹지 않게)"
        );
        // 일꾼의 미결에서 의무가 사라진다(기다려도 갈 곳이 없다).
        let owed = svc.open_items_for("worker", w_id, Instant::now());
        assert!(
            !owed.iter().any(|i| i.id == "m-req"),
            "종결된 계약은 미결 목록에 없어야: {owed:?}"
        );
        // 이력은 잔존한다(원 request 의 배달기록은 그대로 delivered — 계약 축과 배달 축은 별개다).
        assert_eq!(
            svc.ledger_statuses("m-req"),
            vec![DeliveryStatus::Delivered]
        );
        // 기한 스윕이 발화하지 않는다(종결된 계약은 due 대상이 아니다).
        svc.sweep(Instant::now() + Duration::from_secs(601));
        assert!(
            !svc.ledger_snapshot()
                .iter()
                .any(|(_, from, _, _)| from == NOTICE_SENDER_LABEL),
            "종결된 계약엔 기한 초과 통지가 나가지 않아야"
        );
    }

    #[test]
    fn a_reply_that_failed_for_a_transient_reason_keeps_the_contract_open_and_a_retry_closes_it() {
        // ★spec §7 회신 계약 규칙 ④(ADR-0118 결정 2)★: `MAILBOX_FULL`·동명·도달 불가는 **그 순간의 환경**이라
        //   계약을 닫지 않는다(무동작 = 오픈 유지). 여기서 닫으면 잠시 뒤 재시도가 **실제로 배달에 성공해도**
        //   장부는 영영 "회신 실패" 로 남고 요청자의 미결 목록에서도 사라진다(장부 거짓말의 반대 방향).
        let (svc, port, gate) = svc_gated();
        let (boss_from, boss) = live_sender("boss");
        let (w_id, worker) = live("worker");
        let boss_id = boss_from.peer_id;
        port.set_roster(vec![boss.clone(), worker.clone()]);

        // boss → worker request(즉시 배달).
        svc.handle_send(
            "m-req",
            boss_from,
            "boss",
            &["worker".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("배달");

        // boss 의 보관함을 가득 채운다(busy 로 파킹 100건) → 다음 발송은 `MAILBOX_FULL` 행.
        gate.set_busy(boss_id, 0);
        for i in 0..100 {
            send(&svc, &format!("fill{i}"), boss_from, "boss", &["boss"]).expect("파킹");
        }
        let worker_from = SenderIdentity {
            peer_id: w_id,
            epoch: 0,
        };
        let rows_out = svc
            .handle_send(
                "m-rep",
                worker_from,
                "worker",
                &["boss".to_string()],
                "했음",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("행 응답");
        assert_eq!(
            rows_out[0].code,
            Some(FailCode::MailboxFull),
            "보관함이 가득이라 이 회신은 실패 행이다"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-req", "worker"),
            Some("awaiting_reply"),
            "일시 사유 실패는 **무동작** — 계약은 오픈 유지(ADR-0118 결정 2)"
        );

        // ★회귀 핵심★: 자리가 생긴 뒤 재시도가 정상 경로로 계약을 닫는다.
        svc.flush_for("boss", boss_id); // busy 라 드레인되지 않지만, 게이트를 풀고 다시 flush 한다.
        gate.clear();
        svc.flush_for("boss", boss_id);
        let retry = svc
            .handle_send(
                "m-rep2",
                worker_from,
                "worker",
                &["boss".to_string()],
                "했음(재시도)",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("행 응답");
        assert_eq!(
            retry[0].status,
            SendStatus::Delivered,
            "재시도는 배달된다: {retry:?}"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-req", "worker"),
            Some("replied"),
            "재시도 성공이 정상 경로로 계약을 닫는다(오픈 유지의 존재 이유)"
        );
    }

    #[test]
    fn an_ambiguous_reply_target_also_leaves_the_contract_open() {
        // ★리뷰 fix D7 — 위 테스트의 빈칸★: 옛 판은 `MAILBOX_FULL` **하나만** 봐서, 무동작 목록에서 다른
        //   코드를 빼도 초록이었다. 무동작 부류는 spec §3 항목 7-④가 **둘**로 못 박았다(`MAILBOX_FULL` ·
        //   `RECIPIENT_AMBIGUOUS`) — 동명은 "그 순간의 환경" 이지 도달 불가 확정이 아니다.
        let (svc, port) = svc();
        let (boss_from, boss) = live_sender("boss");
        let (w_id, worker) = live("worker");
        port.set_roster(vec![boss.clone(), worker]);
        svc.handle_send(
            "m-req",
            boss_from,
            "boss",
            &["worker".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("배달");

        // 요청자 이름이 산 세션 **둘**에 걸린다(동명) → 회신은 `RECIPIENT_AMBIGUOUS` 실패 행.
        let (_b2, boss_twin) = live("boss");
        let (_w2, worker_again) = live("worker");
        port.set_roster(vec![boss, boss_twin, worker_again]);
        let worker_from = SenderIdentity {
            peer_id: w_id,
            epoch: 0,
        };
        let rows_out = svc
            .handle_send(
                "m-rep",
                worker_from,
                "worker",
                &["boss".to_string()],
                "했음",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("행 응답");
        assert_eq!(
            rows_out[0].code,
            Some(FailCode::RecipientAmbiguous),
            "동명 요청자에게는 배달할 수 없다: {rows_out:?}"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-req", "worker"),
            Some("awaiting_reply"),
            "동명 실패는 **무동작** — 계약 오픈 유지(사용자가 이름을 정리하면 재시도가 닫는다)"
        );
        assert_eq!(
            svc.open_request_count(),
            1,
            "512 계수에서 빠지지 않는다(종결이 아니다)"
        );
    }

    #[test]
    fn a_planned_retirement_that_vanished_before_commit_is_not_reported() {
        // ★R2 — 계측은 **계획**이 아니라 **사실**을 보고해야 한다★.
        //
        // ★왜 이 경로가 실재하나★: 은퇴 표시된 희생자는 커밋 전에 사라질 수 있다 — 그 사이 회신으로 닫히고
        //   자기 이력 행까지 링에서 밀려나면 `purge_finished_without_history` 가 정리한다(`ledger::rollback_open`
        //   의 "알려진 잔여"). 그러면 커밋의 물리 제거는 **아무 것도 지우지 않는다**. 옛 배선은 그때도 계획을
        //   그대로 보고해서, ADR-0108 결정 2 가 "은퇴의 유일한 증거" 라고 못 박은 축에 유령 은퇴를 심었다.
        // ★7차에 재현 수단이 바뀌었다(ADR-0125 — 이전이지 약화가 아니다)★: 옛 판은 `on_inject` 훅(락 밖)에서
        //   희생자를 증발시켰다. 그때는 주입이 **표시와 커밋 사이**에 있었기 때문이다. 전부-큐가 되면서 커밋은
        //   적재 락 안으로 들어갔고 주입(드레인)은 그 락을 놓은 **뒤**로 갔다 — 그래서 훅으로는 이 창에 더 이상
        //   닿지 못한다(닿는다면 락 규율이 깨졌다는 뜻이다). 창 자체는 그대로 있으므로 **예약 가드를 운영과 같은
        //   동사로 직접 열어**(`reserve_marked_contract` — `open_request` → `open_reservation`) 그 안에서 재현한다.
        // ★재현★: 표시 뒤·커밋 전에 ① 이력 링을 새 행으로 가득 채워 희생자의 행을 밀어내고 ② 희생자 계약을
        //   닫고 ③ 한 행 더 써서 evict→purge 를 발화시킨다. 그러면 커밋의 물리 제거가 아무 것도 지우지 않는다.
        let (svc, port) = svc();
        let (from, me) = live_sender("boss");
        port.set_roster(vec![me]);
        fill_open_request_cap_evictable(&svc, from);
        let _ = retirement_reports::drain();

        // 잠정 창을 연다 — 희생자는 **표시**만 됐고 물리 제거는 커밋에서 일어난다.
        let res = reserve_marked_contract(&svc, from, "m-phantom");
        let (vid, vto) = {
            let r = res.retired.as_ref().expect("전제: 희생자가 표시됐다");
            (r.request_id.clone(), r.recipient.clone())
        };

        {
            let mut st = svc.state.lock().expect("lock");
            let now = Instant::now();
            // ① 희생자의 이력 행을 링에서 밀어낸다(이 시점엔 아직 열려 있어 evict 정리 대상이 아니다).
            //    ★링 용량 상수를 복제하지 않는다★: "그 행이 사라졌나" 를 직접 보고 멈춘다(용량이 바뀌어도
            //    픽스처가 따라온다).
            let mut i = 0usize;
            while !st.ledger.records_for(&vid).is_empty() {
                st.ledger.record(
                    &format!("flood{i}"),
                    "boss",
                    "sink",
                    "x",
                    DeliveryStatus::Delivered,
                    now,
                );
                i += 1;
                assert!(i < 100_000, "링 회전이 관측되지 않는다 — 픽스처 전제 붕괴");
            }
            // ② 닫고 ③ 한 행 더 → evict 경로의 purge 가 "끝났고 이력도 없는" 그 계약을 정리한다.
            st.ledger.close_for_test(&vid, &vto, now);
            st.ledger.record(
                "flood-last",
                "boss",
                "sink",
                "x",
                DeliveryStatus::Delivered,
                now,
            );
        }
        assert!(
            !svc.contract_tracked_for_test(&vid, &vto),
            "전제: 희생자가 커밋 전에 사라졌다(purge) — 이 전제가 깨지면 아래 단언이 무의미하다"
        );

        // 커밋 = 결말 확정. 계획한 희생자가 없으므로 은퇴는 **일어나지 않았다**.
        let mut log = RetirementLog::default();
        {
            let mut st = svc.state.lock().expect("lock");
            res.commit(&mut st, &mut log);
        }
        assert!(
            log.real.is_empty(),
            "커밋이 실제로 제거한 게 없으면 은퇴로 세지 않는다: {:?}",
            log.real
        );
        assert_eq!(log.phantom.len(), 1, "대신 이상(유령 계획)으로 남는다");
        // ★핵심★: 그래서 계측 축(ADR-0108 결정 2 의 유일한 증거)에는 한 줄도 나가지 않는다.
        log_contract_retirements("m-phantom", &log.real);
        let reports = retirement_reports::drain();
        assert!(
            reports.is_empty(),
            "커밋이 실제로 제거한 게 없으면 은퇴 보고도 없어야(유령 은퇴 금지): {reports:?}"
        );
        // 그래도 이 예약 자신의 계약은 정상이다(표시가 헛돌았을 뿐 접수는 성립).
        assert!(
            svc.contract_tracked_for_test("m-phantom", "ghost-worker"),
            "예약 자신의 계약은 살아 있어야"
        );
        assert_eq!(svc.marked_retirements_for_test(), 0, "표시가 남지 않는다");
    }

    #[test]
    fn a_retirement_is_actually_reported_and_only_when_it_happened() {
        // ★F5 — 은퇴 계측의 **호출 위치**를 고정한다(M1 회귀축)★: ADR-0108 결정 2 에서 이 info 로그는 은퇴의
        //   **유일한 증거**다. 그런데 커밋이 pass B 로 내려간 뒤(A2) 계측을 결말 루프 **앞**에서 찍으면
        //   ① 주 경로(idle 수신자 request)의 은퇴가 **한 줄도 기록되지 않고** ② 실패로 끝난 발송이 **일어나지
        //   않은 은퇴를 보고한다**. 두 회귀 모두 상태 단언으로는 안 잡힌다(상태는 찍는 위치와 무관).
        let (svc, port) = svc();
        let (from, me) = live_sender("boss");
        let (_w, worker) = live("worker");
        port.set_roster(vec![me, worker]);
        fill_open_request_cap_evictable(&svc, from);
        // 픽스처(레거시 seam)가 남긴 잉여를 비우고 시작한다 — 이 구간의 보고만 본다.
        let _ = retirement_reports::drain();

        // ① 실제로 은퇴가 일어나는 발송(idle 수신자 = 즉시 배달 갈래) → 정확히 1건 보고.
        let out = svc
            .handle_send(
                "m-retire",
                from,
                "boss",
                &["worker".to_string()],
                "해줘",
                Entrance::Mcp,
                &SendMeta {
                    request: true,
                    ..SendMeta::default()
                },
            )
            .expect("행 응답");
        assert_eq!(out[0].status, SendStatus::Delivered, "전제: 즉시 배달 갈래");
        let reports = retirement_reports::drain();
        assert_eq!(
            reports.len(),
            1,
            "은퇴 1건은 정확히 1줄로 보고돼야 — 계측이 결말 루프 앞에 있으면 0줄이다: {reports:?}"
        );
        assert!(
            !reports[0].0.is_empty() && reports[0].0 != "m-retire",
            "보고는 **내보낸 옛 계약**을 지목해야(새 발송이 아니라): {reports:?}"
        );
        assert!(
            !svc.contract_tracked_for_test(&reports[0].0, &reports[0].1),
            "보고된 계약은 실제로 사라졌어야(일어나지 않은 일을 보고하면 안 된다)"
        );

        // ② 실패로 끝난 발송은 **아무 것도 보고하지 않는다**(상한을 **은퇴 불가**로 채워 RequestCapacity 로
        //    끝낸다 — 새 서비스로 시작해야 ①의 은퇴 가능 계약이 희생자로 잡히지 않는다).
        let (svc, port) = super::tests::svc();
        let (from, me) = live_sender("boss");
        let (_w2, worker2) = live("worker");
        port.set_roster(vec![me, worker2]);
        fill_open_request_cap(&svc, from);
        let _ = retirement_reports::drain();
        let out2 = svc
            .handle_send(
                "m-nope",
                from,
                "boss",
                &["worker".to_string()],
                "해줘",
                Entrance::Mcp,
                &req_meta("10m", 600),
            )
            .expect("행 응답");
        assert_eq!(
            out2[0].code,
            Some(FailCode::RequestCapacity),
            "전제: 상한 포화로 실패 행: {out2:?}"
        );
        assert!(
            retirement_reports::drain().is_empty(),
            "일어나지 않은 은퇴를 보고하면 운영자가 유령 은퇴를 쫓는다"
        );
    }

    #[test]
    fn a_request_that_ends_as_a_failed_row_retires_nobody() {
        // ★A2 회귀 — 실패로 끝난 수신자는 **남의 계약을 은퇴시키지 않는다**★.
        //
        // ★7차에 이 회귀가 태우는 창이 바뀌었다(ADR-0125 — 이전이지 약화가 아니다)★: 5차엔 "주입 직전
        //   재확인" 이라는 **락 밖 늦은 갈래**가 있어서, 희생자를 표시한(pass A) 한참 뒤에 그 수신자가 실패
        //   행으로 떨어질 수 있었다. 전부-큐가 되면서 그 갈래는 사라졌다 — 수용 판정과 적재가 **같은 락
        //   구간**이라 판정 뒤에 결말이 뒤집히지 않는다. 그래서 옛 판이 몰던 인터리빙(앞 수신자 주입 중에
        //   뒤 수신자 큐를 채우기)은 이제 **재현 불가**이고, 되살리려면 직발송을 되살려야 한다.
        // ★남은 진짜 창 = 같은 발송의 수신자 사이★: 수신자 1이 희생자를 표시해 잠정 계약을 든 채로,
        //   수신자 2가 같은 pass A 에서 cap 에 걸려 실패한다. 둘이 뒤엉키면 ① 성립한 은퇴까지 덤으로 풀리거나
        //   ② 실패한 쪽 표시가 남아 cap 분모가 영구히 준다. 단언축은 옛 판 그대로다: 표시 잔여 0 · 분모 불변 ·
        //   실패한 수신자의 잠정 계약 소멸(+ 성공한 쪽 계약과 그 은퇴는 살아 있다).
        {
            let (svc, port, gate) = svc_gated();
            let (from, me) = live_sender("boss");
            let (t_id, target) = live("worker");
            let (_l, lead) = live("lead");
            port.set_roster(vec![me, target, lead]);
            // worker 큐만 cap 까지 채운다(통보라 계약 축과 무관). ★busy 가 필요한 이유(7차)★: 이제 모든
            //   발송이 자기 호출에서 드레인을 돌리므로 유휴 수신자에겐 큐가 쌓이지 않는다 — 큐를 채우려면
            //   드레인을 막아야 하고 그 정식 수단이 idle 게이트다.
            gate.set_busy(t_id, 0);
            for i in 0..100 {
                send(&svc, &format!("filler{i}"), from, "boss", &["worker"]).expect("파킹");
            }
            // 상한을 **은퇴 가능** 계약으로 채운다 — 그래야 아래 발송이 희생자를 표시한다.
            fill_open_request_cap_evictable(&svc, from);
            let occupied_before = svc.occupied_slots_for_test();
            let tracking_before = svc.tracking_len_for_test();
            assert_eq!(svc.marked_retirements_for_test(), 0, "전제: 남은 표시 없음");
            let _ = retirement_reports::drain(); // 픽스처(레거시 seam) 잉여를 비우고 이 구간만 본다.

            let out = svc
                .handle_send(
                    "m-late",
                    from,
                    "boss",
                    &["lead".to_string(), "worker".to_string()],
                    "해줘",
                    Entrance::Mcp,
                    &req_meta("10m", 600),
                )
                .expect("행 응답");
            let worker_row = out.iter().find(|r| r.to == "worker").expect("worker 행");
            assert_eq!(
                worker_row.code,
                Some(FailCode::MailboxFull),
                "보관함이 가득한 수신자는 cap 게이트에서 실패 행으로 끝나야: {out:?}"
            );
            let lead_row = out.iter().find(|r| r.to == "lead").expect("lead 행");
            assert_eq!(
                lead_row.status,
                SendStatus::Delivered,
                "전제: 앞 수신자는 정상 수용돼 자기 희생자를 실제로 은퇴시킨다: {out:?}"
            );
            assert_eq!(
                svc.marked_retirements_for_test(),
                0,
                "실패로 끝난 request 는 **아무도 은퇴시키지 않는다** — 표시가 남으면 안 된다"
            );
            assert_eq!(
                svc.occupied_slots_for_test(),
                occupied_before,
                "cap 분모가 그대로여야(성공분은 −희생자 +신규로 상쇄) — 실패분 표시가 남으면 분모가 영구히 준다"
            );
            assert_eq!(
                svc.tracking_len_for_test(),
                tracking_before,
                "잠정 계약도 남지 않아야 — 남으면 추적이 무계로 자란다"
            );
            assert!(!svc.contract_tracked_for_test("m-late", "worker"));
            assert!(
                svc.contract_tracked_for_test("m-late", "lead"),
                "실패한 수신자의 롤백이 같은 발송의 성공한 수신자 계약까지 걷어가면 안 된다"
            );
            // 은퇴는 **성공한 수신자 몫 1건뿐**이다 — 실패분이 덤으로 은퇴를 보고하면 계측이 오염된다.
            let reports = retirement_reports::drain();
            assert_eq!(
                reports.len(),
                1,
                "은퇴는 수용된 수신자 몫 1건뿐: {reports:?}"
            );
            assert_ne!(reports[0].1, "worker", "실패 행은 은퇴의 주체가 아니다");
        }

        // ── pass A 갈래(별도 단언) — 판정 단계에서 이미 큐가 가득한 경우 ─────────────────────
        let (svc, port, gate) = svc_gated();
        let (from, me) = live_sender("boss");
        let (f_id, full) = live("full");
        port.set_roster(vec![me, full]);
        gate.set_busy(f_id, 0);
        for i in 0..100 {
            send(&svc, &format!("p{i}"), from, "boss", &["full"]).expect("파킹");
        }
        fill_open_request_cap_evictable(&svc, from);
        let occupied_before = svc.occupied_slots_for_test();
        let out = svc
            .handle_send(
                "m-passa",
                from,
                "boss",
                &["full".to_string()],
                "해줘",
                Entrance::Mcp,
                &req_meta("10m", 600),
            )
            .expect("행 응답");
        assert_eq!(out[0].code, Some(FailCode::MailboxFull));
        assert_eq!(
            svc.marked_retirements_for_test(),
            0,
            "pass A 실패도 은퇴를 성립시키지 않는다"
        );
        assert_eq!(svc.occupied_slots_for_test(), occupied_before);
    }

    #[test]
    fn a_send_never_cannibalizes_a_contract_it_opened_itself() {
        // ★A3 회귀 — **진짜 창을 태운다**(H2)★.
        //
        // ★라운드 1 판이 두 번 공허했던 이유(리뷰 prober 실측)★: (a) 픽스처가 상한을 **이 발송보다 오래된**
        //   계약으로만 채웠고 희생자 선정은 `min_by_key(created_at)` 이라 이 발송 자신의 계약은 **애초에
        //   뽑힐 수 없었다**(`!r.provisional` 필터를 지워도 초록) (b) 단언이 총수(`>= before`)라 잡아먹혀도
        //   (희생자 −1 + 신규 +1 = 0) 통과했다.
        // ★이 판★: 상한을 **은퇴 불가로 한 자리 남기고** 채우고 그 자리를 **은퇴 가능 1건**으로 메운다 →
        //   첫 수신자가 그 1건을 먹고 들어가면, 두 번째 수신자가 볼 은퇴 가능 후보는 **이 발송이 방금 연 첫
        //   수신자 계약뿐**이다(그게 뽑히면 자기잠식). 단언은 총수가 아니라 **수신자별 계약 존재**다.
        let (svc, port) = svc();
        let (from, me) = live_sender("boss");
        let (_a, w1) = live("w1");
        let (_b, w2) = live("w2");
        port.set_roster(vec![me, w1, w2]);

        fill_open_request_cap_leaving_one_free(&svc, from);
        svc.park_absent_for_test(
            "spare",
            from,
            "boss",
            "spare-victim",
            "q",
            Entrance::Mcp,
            &SendMeta {
                request: true,
                ..SendMeta::default()
            },
        )
        .expect("여유 1자리를 은퇴 가능 계약으로 메운다");

        let out = svc
            .handle_send(
                "m-two",
                from,
                "boss",
                &["w1".to_string(), "w2".to_string()],
                "해줘",
                Entrance::Mcp,
                &SendMeta {
                    request: true,
                    ..SendMeta::default()
                },
            )
            .expect("행 응답");
        assert_eq!(out.len(), 2);
        assert_eq!(
            out[0].status,
            SendStatus::Delivered,
            "첫 수신자는 여유 희생자를 얻어 접수된다: {out:?}"
        );
        assert!(
            svc.contract_tracked_for_test("m-two", "w1"),
            "★핵심★: 첫 수신자의 계약이 살아 있어야 한다 — 같은 발송의 두 번째 수신자가 잡아먹으면 안 된다"
        );
        match out[1].status {
            SendStatus::Failed => assert_eq!(
                out[1].code,
                Some(FailCode::RequestCapacity),
                "두 번째 수신자의 실패 사유는 상한뿐(자기 발송을 먹지 못하므로): {out:?}"
            ),
            _ => assert!(
                svc.contract_tracked_for_test("m-two", "w2"),
                "접수된 수신자는 자기 계약을 가져야: {out:?}"
            ),
        }
        // 발신자 귀속으로 교차 확인 — 접수 수신자 수 == 이 발송으로 열린 계약 수(총수 대신 귀속).
        let admitted = out
            .iter()
            .filter(|r| r.status != SendStatus::Failed)
            .count();
        let mine = svc
            .open_items_for("boss", from.peer_id, Instant::now())
            .into_iter()
            .filter(|i| i.direction == Direction::AwaitingTheirReply && i.id == "m-two")
            .count();
        assert_eq!(
            mine, admitted,
            "접수된 수신자마다 계약이 하나씩 있어야(자기잠식이면 모자란다)"
        );
    }

    #[test]
    fn an_unsettled_reservation_panics_in_debug_and_rolls_itself_back() {
        // ★H1 회귀★: 라운드 1의 `Option<Option<_>>` 은 **잊은 정산을 아무도 잡지 못했다**(prober: 재파킹
        //   갈래 정산을 `let _ = contract.take();` 로 바꿔도 전 스위트 초록). 이제 정산 없이 소멸하면
        //   ① debug 빌드에서 **즉시 패닉**(= 잊은 갈래가 테스트에서 red) ② 그러면서도 **롤백**해 잠정 계약과
        //   은퇴 표시를 남기지 않는다.
        let (svc, port) = svc();
        let (from, me) = live_sender("boss");
        port.set_roster(vec![me]);
        fill_open_request_cap_evictable(&svc, from);
        let occupied_before = svc.occupied_slots_for_test();
        let tracking_before = svc.tracking_len_for_test();

        // 상한 압력 아래에서 계약을 열어 **희생자 표시까지** 만든 뒤, 가드를 정산 없이 떨군다.
        {
            let mut st = svc.state.lock().expect("lock");
            let retired = match st.ledger.open_request(
                "m-drop",
                "boss",
                from.peer_id,
                "ghost-worker",
                None,
                None,
                Instant::now(),
            ) {
                OpenOutcome::OpenedAfterMarking(rc) => Some(rc),
                other => panic!("전제: 상한 압력으로 표시가 붙어야 — {other:?}"),
            };
            assert_eq!(st.ledger.marked_retirement_count_for_test(), 1, "표시 1건");
            let res = open_reservation(&mut st, &svc, "m-drop", "ghost-worker", retired);
            drop(st); // Drop 의 try_lock 이 성공하도록 락을 놓는다(운영에서도 정산은 락 안, Drop 은 락 밖).
            let hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(|_| {})); // 기대된 패닉의 노이즈를 죽인다.
            let hit = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                let _unsettled = res;
            }));
            std::panic::set_hook(hook);
            assert!(
                hit.is_err(),
                "정산 없이 소멸하면 debug 빌드에서 즉시 터져야(H1 — 잊은 갈래를 테스트가 잡는다)"
            );
        }

        assert!(
            !svc.contract_tracked_for_test("m-drop", "ghost-worker"),
            "Drop 이 잠정 계약을 제거해야"
        );
        assert_eq!(
            svc.marked_retirements_for_test(),
            0,
            "Drop 이 희생자 표시도 풀어야 — 남으면 cap 분모가 영구히 줄어든다"
        );
        assert_eq!(svc.occupied_slots_for_test(), occupied_before);
        assert_eq!(svc.tracking_len_for_test(), tracking_before);
    }

    #[test]
    fn a_parked_recipient_keeps_the_contract_its_own_send_opened() {
        // ★F3 — A3 그물의 **파킹 갈래**★: 위 A3 판은 첫 수신자가 **즉시 배달**되는 배치만 태웠다. 그런데
        //   정산 지점은 갈래마다 다르다 — 즉시 배달은 `deliver_one` 뒤, 파킹은 `finish_park` 뒤다. 파킹 갈래의
        //   커밋을 빼먹으면 ① 첫 수신자의 계약이 `provisional` 로 남아 기한 통지 축에서 사라지고 ② 표시된
        //   희생자가 안 지워져 cap 분모가 영구히 줄어든다 — **둘 다 배달 갈래 테스트로는 안 잡힌다**.
        // 배치: 상한을 은퇴 불가로 한 자리 남기고 채움 → 그 자리를 은퇴 가능 1건으로 메움 → 첫 수신자는
        //   **턴 중(busy)** 이라 파킹되고, 두 번째 수신자가 볼 은퇴 가능 후보는 이 발송이 방금 연 첫 수신자
        //   계약뿐이다(그게 뽑히면 자기잠식).
        let (svc, port, gate) = svc_gated();
        let (from, me) = live_sender("boss");
        let (w1_id, w1) = live("w1");
        let (_b, w2) = live("w2");
        port.set_roster(vec![me, w1, w2]);
        gate.set_busy(w1_id, 0); // 첫 수신자만 턴 중 → 파킹 갈래.

        fill_open_request_cap_leaving_one_free(&svc, from);
        svc.park_absent_for_test(
            "spare",
            from,
            "boss",
            "spare-victim",
            "q",
            Entrance::Mcp,
            &SendMeta {
                request: true,
                ..SendMeta::default()
            },
        )
        .expect("여유 1자리를 은퇴 가능 계약으로 메운다");

        let out = svc
            .handle_send(
                "m-park",
                from,
                "boss",
                &["w1".to_string(), "w2".to_string()],
                "해줘",
                Entrance::Mcp,
                &SendMeta {
                    request: true,
                    ..SendMeta::default()
                },
            )
            .expect("행 응답");
        assert_eq!(out.len(), 2);
        assert_eq!(
            out[0].status,
            SendStatus::Pending,
            "전제: 첫 수신자는 턴 중이라 파킹된다(이 갈래를 태우는 게 이 테스트의 목적): {out:?}"
        );
        assert_eq!(svc.parked_len("w1"), 1, "실제로 큐에 실렸다");
        assert!(
            svc.contract_tracked_for_test("m-park", "w1"),
            "★핵심★: 파킹된 수신자의 계약이 살아 있어야 — 같은 발송의 두 번째 수신자가 잡아먹으면 안 된다"
        );
        assert!(
            !svc.contract_tracked_for_test("spare", "spare-victim"),
            "파킹 갈래도 **커밋**으로 정산돼 표시된 희생자를 물리 제거해야(롤백/미정산이면 표시가 남아 cap 분모가 영구히 준다)"
        );
        assert_eq!(
            svc.marked_retirements_for_test(),
            0,
            "정산이 끝나면 표시가 남아 있을 수 없다"
        );
        match out[1].status {
            SendStatus::Failed => assert_eq!(
                out[1].code,
                Some(FailCode::RequestCapacity),
                "두 번째 수신자의 실패 사유는 상한뿐(자기 발송을 먹지 못하므로): {out:?}"
            ),
            _ => assert!(
                svc.contract_tracked_for_test("m-park", "w2"),
                "접수된 수신자는 자기 계약을 가져야: {out:?}"
            ),
        }
        // 발신자 귀속 교차 확인 — 접수(배달+파킹) 수신자 수 == 이 발송으로 열린 계약 수.
        let admitted = out
            .iter()
            .filter(|r| r.status != SendStatus::Failed)
            .count();
        let mine = svc
            .open_items_for("boss", from.peer_id, Instant::now())
            .into_iter()
            .filter(|i| i.direction == Direction::AwaitingTheirReply && i.id == "m-park")
            .count();
        assert_eq!(
            mine, admitted,
            "접수된 수신자마다 계약이 하나씩 — 파킹된 계약이 잠정으로 남으면 여기서 모자란다"
        );
    }

    /// 상한 압력 아래에서 **표시까지 붙은** 잠정 예약 하나를 만든다(정산은 호출자가 한다/안 한다).
    ///
    /// F1 의 두 하드 경로(`try_lock` 실패 · 언와인딩)는 "예약이 이미 존재하는 상태" 를 전제로 하므로 그
    /// 전제만 따로 만든다 — 발송 경로를 타면 그 안에서 정산까지 끝나 버려 재현이 안 된다.
    fn reserve_marked_contract<'a>(
        svc: &'a Arc<MessagingService>,
        from: SenderIdentity,
        msg_id: &str,
    ) -> Reservation<'a> {
        let mut st = svc.state.lock().expect("lock");
        let retired = match st.ledger.open_request(
            msg_id,
            "boss",
            from.peer_id,
            "ghost-worker",
            None,
            None,
            Instant::now(),
        ) {
            OpenOutcome::OpenedAfterMarking(rc) => Some(rc),
            other => panic!("전제: 상한 압력으로 표시가 붙어야 — {other:?}"),
        };
        assert_eq!(st.ledger.marked_retirement_count_for_test(), 1, "표시 1건");
        // ★운영과 같은 동사로 가드를 만든다(R1)★ — 생존 토큰 부착까지 한 번에. 여기서 `Reservation::new` 를
        //   직접 부르면 토큰 없는 예약이 되어 "가드가 살아 있어도 회수된다" 는 가짜 통과를 만든다.
        let res = open_reservation(&mut st, svc, msg_id, "ghost-worker", retired);
        drop(st);
        res
    }

    /// 기대된 패닉의 스택트레이스 노이즈를 죽인 채 클로저를 돌린다.
    fn catch_quietly(f: impl FnOnce()) -> std::thread::Result<()> {
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        std::panic::set_hook(hook);
        r
    }

    #[test]
    fn a_sweep_during_a_slow_injection_never_touches_the_live_reservation() {
        // ★R1 행렬 (a) — 라운드 4 프로브가 실측한 구멍의 정본 회귀★.
        //
        // ★옛 실패 사슬(나이 기준 회수)★: 주입은 유계가 아니다(자식 stdin 은 backpressure 로 무한 블록 가능 —
        //   core `stdio.rs`). 그래서 "예약이 5초 넘었으면 버려진 것" 이라는 판정은 **아직 일하는 소유자**를
        //   회수했고, 그 뒤 주입이 성공해 `commit` 이 돌면 계약이 이미 없었다. 결과는 최악의 조용한 반쪽이다:
        //   `type="request"` 봉투는 배달됐는데 발신자 `awaiting_their_reply` 0 · 수신자 `reply_owed_by_me` 0 ·
        //   기한 통지 영원히 없음 · 나중에 온 정당한 회신은 `NoMatch` 라 이력 행이 영구히 `Delivered`
        //   (= 감사 기록이 "답 없음" 이라 거짓말한다) · 게다가 커밋이 **일어나지 않은 은퇴**를 보고했다.
        // ★재현 방식★: `on_inject` hook 이 주입 도중에 `sweep` 을 돌린다(= 블록된 주입 위로 sweep 틱이 지나간
        //   순간). 시각은 **한 시간 뒤**로 준다 — 어떤 시간 임계값도 이 발송을 구할 수 없었다는 뜻이고, 생존
        //   기준에선 그 값이 아무 의미도 없다는 뜻이다.
        let (svc, port) = svc();
        let (from, me) = live_sender("boss");
        let (w_id, worker) = live("worker");
        port.set_roster(vec![me, worker]);
        fill_open_request_cap_evictable(&svc, from); // 상한 포화 → 이 발송은 희생자를 표시한다.
        let occupied_before = svc.occupied_slots_for_test();
        let _ = retirement_reports::drain();

        {
            let svc2 = svc.clone();
            port.set_on_inject(Arc::new(move |idx| {
                if idx != 0 {
                    return;
                }
                svc2.sweep(Instant::now() + Duration::from_secs(3600));
            }));
        }
        let out = svc
            .handle_send(
                "m-slow",
                from,
                "boss",
                &["worker".to_string()],
                "해줘",
                Entrance::Mcp,
                &req_meta("10m", 600),
            )
            .expect("행 응답");
        assert_eq!(
            out[0].status,
            SendStatus::Delivered,
            "전제: 주입 갈래를 탔다: {out:?}"
        );

        // ★① 계약이 살아 있다★ — 프로브가 잡은 "계약 없는 request 배달" 의 직접 반증.
        assert!(
            svc.contract_tracked_for_test("m-slow", "worker"),
            "소유자가 살아 있는 예약을 sweep 이 회수하면 안 된다(R1)"
        );
        let sender_side: Vec<String> = svc
            .open_items_for("boss", from.peer_id, Instant::now())
            .into_iter()
            .filter(|i| i.id == "m-slow")
            .map(|i| i.direction.as_str().to_string())
            .collect();
        assert_eq!(
            sender_side,
            vec!["awaiting_their_reply".to_string()],
            "발신자는 회신을 기다려야: {sender_side:?}"
        );
        let recipient_side: Vec<String> = svc
            .open_items_for("worker", w_id, Instant::now())
            .into_iter()
            .filter(|i| i.id == "m-slow")
            .map(|i| i.direction.as_str().to_string())
            .collect();
        assert_eq!(
            recipient_side,
            vec!["reply_owed_by_me".to_string()],
            "수신자는 답할 의무를 봐야: {recipient_side:?}"
        );

        // ★② 은퇴는 실제로 일어났고 정확히 1줄 보고됐다★(계측 오염 없음 — R2 와 같은 축).
        let reports = retirement_reports::drain();
        assert_eq!(reports.len(), 1, "은퇴 1건 = 보고 1줄: {reports:?}");
        assert_eq!(svc.marked_retirements_for_test(), 0, "표시가 남지 않는다");
        assert_eq!(
            svc.occupied_slots_for_test(),
            occupied_before,
            "cap 산술도 그대로(은퇴 1 − 신규 1)"
        );

        // ★③ 나중에 온 정당한 회신이 계약을 닫는다★ — 프로브가 본 `NoMatch`·영구 `Delivered` 의 반증.
        svc.handle_send(
            "m-reply",
            reply_from(w_id),
            "worker",
            &["boss".to_string()],
            "다 했다",
            Entrance::Mcp,
            &reply_meta("m-slow"),
        )
        .expect("회신 발송");
        // 닫힘의 관측면 = **미결 조회**(추적 항목 자체는 이력과 함께 정리될 때까지 남는다).
        assert!(
            svc.open_items_for("boss", from.peer_id, Instant::now())
                .iter()
                .all(|i| i.id != "m-slow"),
            "회신이 계약을 닫아야 — 발신자의 미결에서 사라진다(옛 구멍에선 NoMatch 라 영원히 남았다)"
        );
        assert!(
            svc.open_items_for("worker", w_id, Instant::now())
                .iter()
                .all(|i| i.id != "m-slow"),
            "회신자의 의무도 사라져야"
        );
        assert_eq!(
            svc.ledger_statuses("m-slow"),
            vec![DeliveryStatus::Replied],
            "이력도 `Replied` 로 닫혀야 — 프로브가 본 영구 `Delivered`(감사 기록의 거짓말)의 반증"
        );
    }

    #[test]
    fn a_reservation_dropped_while_another_thread_holds_the_lock_is_reclaimed_by_the_sweep() {
        // ★F1-(a) — Drop 은 보증이 아니다★: `Reservation::drop` 은 데드락을 피하려 `try_lock` 만 한다. 그래서
        //   **다른 스레드가 상태 락을 쥔 순간** 에 떨어지면 롤백이 아예 일어나지 않는다. 그때 남는 잔해의 대가는
        //   영구적이다(잠정 계약 = 기한 통지 소멸 · 표시된 희생자 = cap 분모 영구 감소 · 추적 목록 무계 증가).
        //   보증은 **주기 sweep** 이 진다 — 이 테스트가 그 보증을 `try_lock` 실패 경로에서 직접 관측한다.
        let (svc, port) = svc();
        let (from, me) = live_sender("boss");
        port.set_roster(vec![me]);
        fill_open_request_cap_evictable(&svc, from);
        let occupied_before = svc.occupied_slots_for_test();
        let tracking_before = svc.tracking_len_for_test();

        let guard = reserve_marked_contract(&svc, from, "m-locked");

        // 다른 스레드가 락을 **쥔 채 대기** → 그 창에서 가드를 떨군다(운영의 락 경합 재현).
        let (locked_tx, locked_rx) = std::sync::mpsc::channel::<()>();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let holder = {
            let svc2 = svc.clone();
            std::thread::spawn(move || {
                let _st = svc2.state.lock().expect("lock");
                locked_tx.send(()).expect("신호");
                release_rx.recv().expect("해제 신호"); // 락을 쥔 채 대기.
            })
        };
        locked_rx.recv().expect("락 점유 확인");
        // debug 빌드에선 정산 누락 가드(`debug_assert`)가 여기서 터진다 — 그게 **의도**이므로 삼킨다.
        let hit = catch_quietly(|| drop(guard));
        if cfg!(debug_assertions) {
            assert!(hit.is_err(), "정산 누락은 debug 에서 red 여야(H1)");
        }
        release_tx.send(()).expect("해제");
        holder.join().expect("holder");

        // ① 실제로 **롤백되지 않았다**(= 이 경로에 구멍이 있다는 사실 자체를 고정한다).
        assert!(
            svc.contract_tracked_for_test("m-locked", "ghost-worker"),
            "try_lock 이 실패했으니 Drop 은 아무 것도 못 했다 — 이 전제가 깨지면 아래 보증 단언이 무의미해진다"
        );
        assert_eq!(svc.marked_retirements_for_test(), 1, "표시도 남아 있다");

        // ② 보증: sweep 이 유예를 넘긴 예약을 회수한다.
        svc.sweep(Instant::now() + Duration::from_secs(30));
        assert!(
            !svc.contract_tracked_for_test("m-locked", "ghost-worker"),
            "sweep 이 잠정 계약을 회수해야(F1 보증 계층)"
        );
        assert_eq!(
            svc.marked_retirements_for_test(),
            0,
            "희생자 표시도 풀려야 — 남으면 cap 분모가 영구히 준다"
        );
        assert_eq!(svc.occupied_slots_for_test(), occupied_before);
        assert_eq!(svc.tracking_len_for_test(), tracking_before);
    }

    #[test]
    fn a_reservation_dropped_while_unwinding_neither_aborts_nor_leaks() {
        // ★F1-(b) — 이중 패닉 금지★: Drop 은 패닉 언와인딩 중에도 불린다. 거기서 `debug_assert!(false, …)` 로
        //   다시 패닉하면 **이중 패닉 → abort** 다: 테스트 프로세스가 통째로 죽어 원래 원인조차 못 본다(그리고
        //   운영 릴리즈는 `panic = "abort"` 라 더 나쁘다). 그래서 `thread::panicking()` 이면 단언을 건너뛰고
        //   기록만 남긴다 — 이 테스트가 **끝까지 실행된다는 사실 자체**가 abort 없음의 증거다.
        let (svc, port) = svc();
        let (from, me) = live_sender("boss");
        port.set_roster(vec![me]);
        fill_open_request_cap_evictable(&svc, from);
        let occupied_before = svc.occupied_slots_for_test();
        let tracking_before = svc.tracking_len_for_test();

        let hit = catch_quietly(|| {
            let _guard = reserve_marked_contract(&svc, from, "m-unwind");
            panic!("boom");
        });
        let payload = *hit
            .expect_err("패닉이 전파돼야")
            .downcast::<&str>()
            .expect("페이로드는 원래 패닉 그대로");
        assert_eq!(
            payload, "boom",
            "Drop 이 원래 패닉을 자기 패닉으로 갈아치우면 원인이 사라진다"
        );

        // 락은 아무도 안 쥐고 있었으니 이 갈래에선 Drop 의 `try_lock` 이 성공해 **즉시** 회수된다.
        assert!(
            !svc.contract_tracked_for_test("m-unwind", "ghost-worker"),
            "언와인딩 중에도 롤백은 최선 노력으로 수행된다"
        );
        assert_eq!(svc.marked_retirements_for_test(), 0);
        assert_eq!(svc.occupied_slots_for_test(), occupied_before);
        assert_eq!(svc.tracking_len_for_test(), tracking_before);
    }

    #[test]
    fn a_parked_reply_still_closes_the_contract() {
        // ★M5(라운드 1 A1 수정의 새 구멍 — prober 실측)★: 닫힘 게이트를 "delivered 만" 으로 좁혀도 전
        //   스위트가 초록이었다. 그러면 **턴 중인 요청자에게 회신**하면(파킹 = 곧 배달) 계약이 열린 채 남아
        //   발신자에게 거짓 기한 통지가 가고, 회신자의 `reply_owed_by_me` 가 답한 뒤에도 남는다.
        let (svc, port, gate) = svc_gated();
        let (boss_from, boss) = live_sender("boss");
        let boss_id = boss.id;
        let (w_id, worker) = live("worker");
        port.set_roster(vec![boss, worker]);

        svc.handle_send(
            "m-req",
            boss_from,
            "boss",
            &["worker".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("배달");
        assert_eq!(svc.open_request_count(), 1);

        gate.set_busy(boss_id, 0); // 요청자가 턴 진행 중 → 회신은 파킹된다
        let rows_out = svc
            .handle_send(
                "m-rep",
                reply_from(w_id),
                "worker",
                &["boss".to_string()],
                "했음",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("행 응답");
        assert_eq!(
            rows_out[0].status,
            SendStatus::Pending,
            "턴 중 요청자에게는 파킹된다: {rows_out:?}"
        );
        assert_eq!(
            svc.open_request_count(),
            0,
            "★파킹된 회신도 계약을 닫는다(M5)★ — 접수됐으므로 곧 배달된다"
        );
        svc.sweep(Instant::now() + Duration::from_secs(601));
        assert!(
            !svc.ledger_snapshot()
                .iter()
                .any(|(_, from, _, _)| from == NOTICE_SENDER_LABEL),
            "닫힌 계약엔 거짓 기한 통지가 없다"
        );
    }

    #[test]
    fn the_to_attribute_rules_hold_for_every_token_shape() {
        // ★M4 + M6 — `build_to_attr` 직접 단위 테스트★.
        //
        // ★왜 화이트박스인가(M6 의 도달 가능성)★: "`@`토큰의 펼침이 **전원 실패**" 상태는 `@all` 이 유일한
        //   소스인 v1 에선 블랙박스로 만들 수 없다 — `@all` 의 키는 곧 산 로스터이고, 봉투가 나가려면(수용
        //   2인 이상) 그 중 최소 둘이 수용돼야 하므로 "전원 실패" 와 양립하지 않는다. 그래도 그 갈래는
        //   미래 소스(폴더 그룹)가 곧 쓸 규칙이고 mutation 이 무방비였으므로, 판정 함수를 직접 못 박는다.
        let tok_name = |k: &str| AddressToken::Name { key: k.to_string() };
        let tok_group = |l: &str, keys: &[&str]| AddressToken::Group {
            label: l.to_string(),
            keys: keys.iter().map(|k| k.to_string()).collect(),
        };
        let rcpt = |k: &str| ResolvedRecipient {
            display: k.to_string(),
            key: k.to_string(),
            target: None,
            live_count: 0,
            dormant_count: 0,
            dup_of: None,
        };

        // ① Name 토큰: 수용은 싣고 실패는 뺀다.
        let addr = Addressing {
            recipients: vec![rcpt("bob"), rcpt("ghost")],
            tokens: vec![tok_name("bob"), tok_name("ghost")],
        };
        assert_eq!(build_to_attr(&addr, &[true, false]), "bob");

        // ② M4: 표기 중복은 **한 번만** 적는다(먼저 나온 자리 유지).
        let addr = Addressing {
            recipients: vec![rcpt("bob"), rcpt("carol")],
            tokens: vec![tok_name("bob"), tok_name("bob"), tok_name("carol")],
        };
        assert_eq!(
            build_to_attr(&addr, &[true, true]),
            "bob,carol",
            "같은 값을 두 번 적으면 수신 LLM 이 인원수를 오독한다(M4)"
        );

        // ③ M6: `@`토큰의 펼침이 **전원 실패**면 그 토큰은 빠진다.
        let addr = Addressing {
            recipients: vec![rcpt("carol"), rcpt("dup")],
            tokens: vec![tok_name("carol"), tok_group("@all", &["dup"])],
        };
        assert_eq!(
            build_to_attr(&addr, &[true, false]),
            "carol",
            "펼침이 전부 실패한 @토큰은 이 발송에 아무 것도 기여하지 않았다(M6)"
        );

        // ④ 하나라도 수용되면 남는다 + A5(흡수도 기여로 센다).
        let addr = Addressing {
            recipients: vec![rcpt("bob"), rcpt("carol")],
            tokens: vec![
                tok_name("bob"),
                tok_name("carol"),
                tok_group("@all", &["bob", "carol"]),
            ],
        };
        assert_eq!(
            build_to_attr(&addr, &[true, true]),
            "bob,carol,@all",
            "펼침이 앞선 명시 지목에 완전히 흡수돼도 실패는 아니다(A5)"
        );

        // ⑤ M4(그룹 축): 같은 라벨이 두 번 적히지 않는다.
        let addr = Addressing {
            recipients: vec![rcpt("bob")],
            tokens: vec![tok_group("@all", &["bob"]), tok_group("@all", &["bob"])],
        };
        assert_eq!(build_to_attr(&addr, &[true]), "@all");
    }

    #[test]
    fn a_duplicate_name_token_is_written_once_in_the_envelope() {
        // ★M4 블랙박스★: `["bob","bob","carol"]` 은 수신자 둘이고 봉투도 `to="bob,carol"` 여야 한다.
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_b, bob) = live("bob");
        let (_c, carol) = live("carol");
        port.set_roster(vec![me, bob, carol]);
        let out = send(&svc, "m1", from, "alice", &["bob", "bob", "carol"]).expect("ok");
        assert_eq!(out.len(), 2, "표기 중복은 행을 만들지 않는다");
        assert!(
            port.injected_bodies()[0].contains(r#"to="bob,carol""#),
            "봉투 to 도 중복 없이: {:?}",
            port.injected_bodies()
        );
    }

    #[test]
    fn two_exact_ids_of_same_named_twins_both_get_a_row() {
        // ★M3(과도기 방어 — 정책 아님, ADR-0116 결정 5)★: park·장부 키가 이름이라 두 쌍둥이는 **한 자리**
        //   밖에 못 쓴다. 옛 구현은 뒤 토큰을 **행 없이 삼켰다**(spec §6 "수신자 1명 = 1행" 위반). 이제 그
        //   토큰은 `RECIPIENT_AMBIGUOUS` **실패 행**으로 보인다 — 동명이인 자체는 미지원이고(힌트가 사용자
        //   에스컬레이션을 가리킨다) 뿌리 제거는 ADR-0115 가 한다. 재가 대기 상태가 아니다.
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (d1, dup1) = live("dup");
        let (d2, dup2) = live("dup");
        port.set_roster(vec![me, dup1, dup2]);

        // ① 두 exact id
        let (id1, id2) = (d1.to_string(), d2.to_string());
        let out = send(&svc, "m1", from, "alice", &[&id1, &id2]).expect("ok");
        assert_eq!(
            out.len(),
            2,
            "토큰마다 행이 있어야(조용한 삼킴 금지): {out:?}"
        );
        assert_eq!(out[0].status, SendStatus::Delivered);
        assert_eq!(
            (out[1].status, out[1].code),
            (SendStatus::Failed, Some(FailCode::RecipientAmbiguous)),
            "뒤 토큰은 보이는 실패: {out:?}"
        );
        assert_eq!(port.injected_bodies().len(), 1, "배달은 한 통");

        // ② 이름 + 두 exact id(3토큰)
        let (svc2, port2) = super::tests::svc();
        let (from2, me2) = live_sender("alice");
        let (e1, t1) = live("dup");
        let (e2, t2) = live("dup");
        port2.set_roster(vec![me2, t1, t2]);
        let out2 = send(
            &svc2,
            "m2",
            from2,
            "alice",
            &["dup", &e1.to_string(), &e2.to_string()],
        )
        .expect("ok");
        assert_eq!(
            out2.len(),
            2,
            "이름 토큰과 첫 id 는 접히고, 둘째 id 가 행을 얻는다: {out2:?}"
        );
        assert_eq!(
            out2[0].status,
            SendStatus::Delivered,
            "이름 자리가 첫 id 의 해석을 물려받아 배달된다(A8): {out2:?}"
        );
        assert_eq!(
            (out2[1].status, out2[1].code),
            (SendStatus::Failed, Some(FailCode::RecipientAmbiguous))
        );
        assert_eq!(port2.injected_bodies().len(), 1);
    }

    #[test]
    fn a_repeated_identical_id_token_yields_exactly_one_row() {
        // ★F2-(a)★: 같은 exact-id 를 두 번 적으면 정보량이 0 인 표기 중복이다 — 행은 하나여야 한다. 옛
        //   구현은 M3 갈래(동명 쌍둥이의 뒤 토큰 = 실패 행)를 **매 토큰마다 새로** 타서 `[A-id, B-id, B-id]`
        //   에 B 실패 행이 **두 줄** 났고, 두 줄이 같은 장부 키를 발급받아 서로를 덮었다.
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (d1, dup1) = live("dup");
        let (d2, dup2) = live("dup");
        port.set_roster(vec![me, dup1, dup2]);
        let (id1, id2) = (d1.to_string(), d2.to_string());

        let out = send(&svc, "m1", from, "alice", &[&id1, &id2, &id2, &id1, &id2]).expect("ok");
        assert_eq!(
            rows(&out),
            vec![
                (id1.clone(), SendStatus::Delivered, None),
                (
                    id2.clone(),
                    SendStatus::Failed,
                    Some(FailCode::RecipientAmbiguous)
                ),
            ],
            "반복 토큰은 접히고 행은 2개(수용 1 + loser 1): {out:?}"
        );
        // 종점 키가 행마다 하나씩·서로 달라야 한다(loser 키가 이름 공간 밖이라는 사실의 관측면).
        let keys = svc.ledger_endpoint_keys("m1");
        assert_eq!(keys.len(), 2, "장부 종점도 행 수와 같아야: {keys:?}");
        assert_ne!(keys[0], keys[1], "두 행이 같은 키를 쓰면 서로를 덮는다");
        assert_eq!(port.injected_bodies().len(), 1, "배달은 한 통");
    }

    #[test]
    fn a_loser_row_never_collides_with_an_agent_whose_name_is_another_agents_id() {
        // ★F2-(b)(c) — 최악의 교차 충돌★: 어떤 에이전트 C 의 canonical 이름이 **글자 그대로 다른 에이전트
        //   B 의 id 문자열**인 병리적 로스터. loser 행의 키가 표기(`display`)였다면 ① C 를 지목한 토큰이 B 의
        //   loser 행 자리를 물려받아(A8 승격) B 의 실패가 사라지고 C 의 배달이 B 의 표기로 보고되거나
        //   ② 순서를 뒤집으면 장부 키가 **두 행에 중복 발급**된다. 두 순서 모두 무결해야 한다.
        for order in 0..2 {
            let (svc, port) = svc();
            let (from, me) = live_sender("alice");
            let (d1, dup1) = live("dup");
            let (d2, dup2) = live("dup");
            // C 의 이름 = B(=d2)의 id 문자열. `resolve_live` 는 id 정확 일치를 먼저 보므로 이 토큰은 B 를 가리킨다.
            let evil_name = d2.to_string();
            let (c, mut trap) = live("placeholder");
            trap.name = evil_name.clone();
            port.set_roster(vec![me, dup1, dup2, trap]);
            let id1 = d1.to_string();

            // 두 토큰: `d1-id`(수용) + `d2-id`(= C 의 이름과 동일한 문자열 → M3 loser 행).
            let list: Vec<&str> = if order == 0 {
                vec![id1.as_str(), evil_name.as_str()]
            } else {
                vec![evil_name.as_str(), id1.as_str()]
            };
            let out = send(&svc, "m1", from, "alice", &list).expect("ok");
            assert_eq!(out.len(), 2, "행 2개(order={order}): {out:?}");
            let statuses: Vec<SendStatus> = out.iter().map(|r| r.status).collect();
            assert_eq!(
                statuses,
                vec![SendStatus::Delivered, SendStatus::Failed],
                "먼저 쓴 토큰이 수용, 뒤가 loser(order={order}): {out:?}"
            );
            assert_eq!(
                out[1].code,
                Some(FailCode::RecipientAmbiguous),
                "order={order}"
            );
            assert_eq!(
                out.iter().map(|r| r.to.clone()).collect::<Vec<_>>(),
                list.iter().map(|t| t.to_string()).collect::<Vec<_>>(),
                "표기는 입력 그대로(WYSIWYA · order={order})"
            );

            let keys = svc.ledger_endpoint_keys("m1");
            assert_eq!(keys.len(), 2, "종점 2개(order={order}): {keys:?}");
            assert_ne!(
                keys[0], keys[1],
                "loser 키가 이름 공간과 겹치면 두 행이 한 종점을 공유한다(order={order}): {keys:?}"
            );
            assert!(
                keys.iter().all(|k| *k != evil_name || k == &keys[0]),
                "order={order}: {keys:?}"
            );
            // 봉투 `to` 에 loser 토큰이 **수용**으로 세어져선 안 된다 → 단일 수신자라 `to` 속성 자체가 없다.
            let body = &port.injected_bodies()[0];
            assert!(
                !body.contains(" to=\""),
                "수용 1명이면 `to` 속성은 나오지 않는다(loser 를 세면 2명이 된다 · order={order}): {body}"
            );
            // C 자신은 아무 것도 못 받는다 — 그 토큰은 처음부터 B 를 가리켰다(id 우선 해석).
            assert_eq!(svc.parked_len(&evil_name), 0, "order={order}");
            assert_eq!(port.injected_bodies().len(), 1, "order={order}");
            let _ = c;
        }
    }

    #[test]
    fn the_loser_key_prefix_keeps_it_outside_the_agent_name_space() {
        // ★R3 — `loser_key` 의 "충돌은 구조적으로 불가능" 주장을 **테스트로 못 박는다**★. 접두를 지워도 전
        //   스위트가 초록이었다 = 그 주장이 무방비였다(그리고 짝인 `dup_of.is_none()` 필터는 접두가 성립하는
        //   동안만 잉여다). 두 층으로 고정한다.
        // ① 성질: loser 키에는 canonical 이름이 담을 수 없는 문자(제어문자)가 반드시 들어 있다.
        assert!(
            loser_key("anything").chars().any(char::is_control),
            "제어문자 접두가 없으면 loser 키가 이름 공간 **안**으로 들어온다"
        );

        // ② 병리적 로스터: 어떤 에이전트의 canonical 이름이 **접두를 뗀 loser 키와 글자 그대로 같다**.
        //    접두가 없으면 그 토큰이 loser 행과 같은 키를 받아 행 하나가 삼켜지거나 장부 키가 중복 발급된다.
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (d1, dup1) = live("dup");
        let (d2, dup2) = live("dup");
        let unprefixed = loser_key(&d2.to_string()).replace('\u{1}', ""); // = "ambiguous:<d2>"
        let (_t, mut trap) = live("placeholder");
        trap.name = unprefixed.clone();
        port.set_roster(vec![me, dup1, dup2, trap]);

        let (id1, id2) = (d1.to_string(), d2.to_string());
        let out = send(
            &svc,
            "m1",
            from,
            "alice",
            &[&id1, &id2, unprefixed.as_str()],
        )
        .expect("ok");
        assert_eq!(
            rows(&out),
            vec![
                (id1.clone(), SendStatus::Delivered, None),
                (
                    id2.clone(),
                    SendStatus::Failed,
                    Some(FailCode::RecipientAmbiguous)
                ),
                (unprefixed.clone(), SendStatus::Delivered, None),
            ],
            "세 토큰이 각자 자기 행을 가져야 — 접두가 없으면 셋째가 loser 행에 흡수된다: {out:?}"
        );
        let keys = svc.ledger_endpoint_keys("m1");
        assert_eq!(keys.len(), 3, "종점도 행마다 하나: {keys:?}");
        let mut uniq = keys.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(
            uniq.len(),
            3,
            "종점 키가 중복 발급되면 행이 서로를 덮는다: {keys:?}"
        );
        assert_eq!(port.injected_bodies().len(), 2, "배달은 두 통(d1 + trap)");
    }

    #[test]
    fn a_cap_saturated_request_fails_that_row_and_delivers_nothing_to_it() {
        // ★A7 — REQUEST_CAPACITY 는 이제까지 **테스트가 0개**였다(mutation #11: Full 갈래가 계약 없이
        //   배달해도 전 스위트 green)★. spec §3 항목 5 · ADR-0114 영향절: 계약을 못 연 수신자에겐
        //   **메시지도 배달하지 않는다**(추적 없는 request 배달 금지).
        let (svc, port) = svc();
        let (from, me) = live_sender("boss");
        let (_w, worker) = live("worker");
        port.set_roster(vec![me, worker]);
        fill_open_request_cap(&svc, from);
        let injected_before = port.injected_bodies().len();

        let out = svc
            .handle_send(
                "m-over",
                from,
                "boss",
                &["worker".to_string()],
                "해줘",
                Entrance::Mcp,
                &req_meta("10m", 600),
            )
            .expect("행 응답(전체 반려 아님 — 행 실패)");
        assert_eq!(
            rows(&out),
            vec![(
                "worker".to_string(),
                SendStatus::Failed,
                Some(FailCode::RequestCapacity)
            )]
        );
        assert_eq!(
            port.injected_bodies().len(),
            injected_before,
            "계약을 못 연 수신자에겐 **배달도 하지 않는다**(추적 없는 request 금지)"
        );
        assert_eq!(
            svc.ledger_statuses("m-over"),
            vec![DeliveryStatus::Failed],
            "장부에 delivered 가 아니라 종점 failed 만 남는다"
        );
        assert_eq!(svc.parked_len("worker"), 0, "파킹도 없다");
    }

    #[test]
    fn the_to_attribute_keeps_a_token_whose_expansion_was_fully_absorbed_by_explicit_names() {
        // ★A5 회귀★: `["bob","carol","@all"]` 에서 `@all` 의 펼침은 앞선 명시 지목에 **완전히 흡수**된다 —
        //   행은 만들지 않지만 **실패한 것도 아니다**. spec §1 이 빼라고 한 건 실패 토큰뿐이므로 `@all` 은
        //   `to` 에 남아야 한다(옛 판정은 "행을 만들었나" 라서 통째로 빠졌다).
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_b, bob) = live("bob");
        let (_c, carol) = live("carol");
        port.set_roster(vec![me, bob, carol]);

        let out = send(&svc, "m1", from, "alice", &["bob", "carol", "@all"]).expect("ok");
        assert_eq!(out.len(), 2, "행은 흡수돼 2개");
        assert!(
            port.injected_bodies()[0].contains(r#"to="bob,carol,@all""#),
            "흡수된 @all 토큰도 입력 표기 순으로 남아야: {:?}",
            port.injected_bodies()
        );
    }

    #[test]
    fn an_exact_id_token_rescues_an_ambiguous_name_in_the_same_to_list() {
        // ★A8(과도기 방어 — 정책 아님, ADR-0116 결정 5)★: 두 토큰이 같은 키로 접힐 때 **해석된 쪽이 이긴다**.
        //   그러지 않으면 토큰 순서만 뒤집으면 배달되는 비대칭이 생긴다(같은 `to` 에 id 를 덧붙였는데 앞뒤에
        //   따라 결말이 갈린다). 힌트는 이제 exact-id 재발송을 가르치지 않지만, 이 방어는 그 비대칭 자체를
        //   막는 것이라 존치한다 — ADR-0115 가 동명 부류를 없애면 함께 사문화된다. 재가 대기 상태가 아니다.
        for order in 0..2 {
            let (svc, port) = svc();
            let (from, me) = live_sender("alice");
            let (d1, dup1) = live("dup");
            let (_d2, dup2) = live("dup");
            port.set_roster(vec![me, dup1, dup2]);
            let id = d1.to_string();
            let list: Vec<&str> = if order == 0 {
                vec!["dup", id.as_str()]
            } else {
                vec![id.as_str(), "dup"]
            };
            let out = send(&svc, "m1", from, "alice", &list).expect("ok");
            assert_eq!(out.len(), 1, "같은 수신자라 행 1개(order={order})");
            assert_eq!(
                out[0].status,
                SendStatus::Delivered,
                "exact-id 가 동명 모호성을 통과시킨다(order={order}): {out:?}"
            );
            assert_eq!(
                out[0].to, list[0],
                "표기는 **먼저 쓴 토큰**을 남긴다(WYSIWYA · order={order})"
            );
        }
    }

    #[test]
    fn a_reply_matches_its_own_contract_by_id_before_falling_back_to_the_name() {
        // ★A6 회귀★: 계약 A(recipient "alice", id a)가 목록에서 앞서고 계약 B(같은 이름, id b)가 뒤에 있으면
        //   한 패스 OR 매칭은 **b 의 회신으로 A 를 닫았다**(자기 것 아닌 계약). 두 패스(id 우선)면 각자 자기
        //   계약만 닫는다.
        let mut l = Ledger::new();
        let now = Instant::now();
        let a = PeerId::new_v4();
        let b = PeerId::new_v4();
        l.open_request("m1", "boss", PeerId::new_v4(), "alice", Some(a), None, now);
        l.open_request("m1", "boss", PeerId::new_v4(), "alice2", Some(b), None, now);
        // b 가 회신 — 자기 계약(alice2)만 닫혀야 한다. 옛 OR 한 패스는 이름 매치가 아니어도 id 순서에
        //   따라 앞의 항목을 집을 수 있었다(여기선 id 우선 패스가 정확히 b 를 고른다).
        assert_eq!(
            l.close_on_reply("m1", "alice2", b, true, now),
            ReplyOutcome::Closed
        );
        let open = l.open_requests();
        assert_eq!(open.len(), 1, "하나만 닫혔다: {open:?}");
        assert_eq!(open[0].recipient_id, Some(a), "남은 건 A 의 계약");

        // 이름은 같지만 id 가 다른 제3자는 아무 것도 닫지 못한다(id 매치 우선 → 폴백은 id 없는 계약만).
        let mut l2 = Ledger::new();
        l2.open_request("m2", "boss", PeerId::new_v4(), "alice", Some(a), None, now);
        assert_eq!(
            l2.close_on_reply("m2", "alice", PeerId::new_v4(), true, now),
            ReplyOutcome::NoMatch,
            "id 를 든 계약은 이름만 같은 쌍둥이가 닫을 수 없다(조회 축과 동일)"
        );
    }

    #[test]
    fn can_admits_prediction_always_matches_what_park_actually_does() {
        // ★C4(mutation #15′)★: 봉투 `to` 동결은 "파킹 **전에** cap 결과를 안다" 는 전제 위에 있다.
        //   `can_admit` 의 분모가 조용히 달라지면(예: in-flight 를 빼면) 그 전제가 깨지는데, `park` 이 스스로
        //   다시 계산하므로 기존 테스트로는 안 잡힌다. 두 판정이 **항상 같은 답**임을 직접 못 박는다.
        let (svc, port, gate) = svc_gated();
        let (from, me) = live_sender("alice");
        let (r_id, recv) = live("recv");
        port.set_roster(vec![me, recv]);
        gate.set_busy(r_id, 0);
        for i in 0..100 {
            assert_eq!(
                svc.can_admit_for_test("recv"),
                true,
                "{i}번째 예측: 아직 여유"
            );
            send(&svc, &format!("p{i}"), from, "alice", &["recv"]).expect("파킹");
        }
        assert!(!svc.can_admit_for_test("recv"), "cap 도달 예측");
        assert_eq!(
            send(&svc, "over", from, "alice", &["recv"]).expect("행")[0].code,
            Some(FailCode::MailboxFull),
            "실제 park 도 반려 — 예측과 일치"
        );

        // ★in-flight 창에서도 일치★: 배치가 락 밖으로 나가 큐가 비어 보이는 순간의 예측도 park 과 같다.
        gate.clear();
        let (svc_h, from_h) = (svc.clone(), from);
        let seen: Arc<StdMutex<Vec<(bool, bool)>>> = Arc::new(StdMutex::new(Vec::new()));
        let seen_h = seen.clone();
        port.set_on_inject(Arc::new(move |idx| {
            if idx != 0 {
                return;
            }
            let predicted = svc_h.can_admit_for_test("recv");
            let actual = svc_h
                .handle_send(
                    "during",
                    from_h,
                    "alice",
                    &["recv".to_string()],
                    "b",
                    Entrance::Mcp,
                    &SendMeta::default(),
                )
                .expect("행")[0]
                .code
                .is_none();
            seen_h.lock().unwrap().push((predicted, actual));
        }));
        svc.flush_for("recv", r_id);
        let obs = seen.lock().unwrap().clone();
        assert_eq!(obs.len(), 1);
        assert_eq!(
            obs[0].0, obs[0].1,
            "in-flight 창에서도 예측 == 실제(분모가 갈리면 여기서 터진다)"
        );
    }

    // ══════════════════════════════════════════════════════════════════════════════════════════
    // S18.6 입구 3분기 — 잠듦 파킹 · 턴 신호 없으면 즉시 주입 · 삭제 정리(spec §7 · ADR-0116/0117/0118)
    // ══════════════════════════════════════════════════════════════════════════════════════════

    #[test]
    fn a_dormant_recipient_is_parked_without_a_wake_and_flushed_on_restore() {
        // ★spec §7 "잠든 수신자 파킹"(ADR-0116 결정 1)★: 로스터엔 없지만 **프로필이 실재하는** 이름 →
        //   응답 행 `pending` · 파킹 레코드 생성 · **wake 미발동**(아무것도 깨우지 않는다 — 주입 시도 0) ·
        //   세션 복원(재등장) 시 flush 로 `delivered` 전이.
        // ★"wake 미발동" 의 관측 방법★: 커널엔 깨우기 seam 자체가 없다(있다면 새 포트 동사여야 한다) —
        //   그래서 "그 발송이 주입을 한 건도 시도하지 않았다" + "큐에 남았다" 로 단언한다.
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        port.set_roster(vec![me]);
        port.set_dormant(&["sleepy"]);

        let out = send(&svc, "m-dorm", from, "alice", &["sleepy"]).expect("반려 아님");
        assert_eq!(
            (out[0].status, out[0].code),
            (SendStatus::Pending, None),
            "잠듦은 수용(파킹)이다 — 실패 행이 아니다: {out:?}"
        );
        assert!(
            out[0].hint.as_deref().unwrap_or("").contains("saved agent"),
            "힌트가 '저장된 에이전트라 복원 시 배달' 을 말해야: {:?}",
            out[0].hint
        );
        assert_eq!(
            svc.ledger_statuses("m-dorm"),
            vec![DeliveryStatus::Pending],
            "파킹 레코드가 장부에 남는다(조용한 유실 금지)"
        );
        assert_eq!(svc.parked_len("sleepy"), 1, "이름 큐에 파킹된다");
        assert!(
            port.injected_bodies().is_empty(),
            "잠든 상대에게 주입을 시도하지 않는다(wake 미발동)"
        );

        // 복원(재등장) — 로스터에 그 이름이 뜨고 flush 가 집는다(도어벨은 없었지만 로스터 diff 경로가 부른다).
        let (late_id, late) = live("sleepy");
        port.set_roster(vec![late]);
        port.set_dormant(&[]);
        svc.flush_for("sleepy", late_id);
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="alice">body</message>"#.to_string()],
            "복원 시 flush 로 배달된다"
        );
        assert_eq!(
            svc.ledger_statuses("m-dorm"),
            vec![DeliveryStatus::Delivered]
        );
        assert_eq!(svc.parked_len("sleepy"), 0);
    }

    #[test]
    fn at_here_never_expands_to_a_dormant_name_but_direct_naming_parks_it() {
        // ★ADR-0121 결정 1 — `@here` = 지금 살아 있는 전원★: 잠든 이름은 이 어휘의 펼침에 들어오지 않는다.
        //   (이 단언은 옛 `@all` 의 계약이었다 — 어휘가 갈리면서 그 의미를 `@here` 가 물려받았다.)
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_b, bob) = live("bob");
        port.set_roster(vec![me, bob]);
        port.set_dormant(&["sleepy"]);

        let here = send(&svc, "m-here", from, "alice", &["@here"]).expect("반려 아님");
        assert_eq!(
            here.iter().map(|r| r.to.clone()).collect::<Vec<_>>(),
            vec!["bob".to_string()],
            "@here 펼침에 잠든 이름이 끼면 안 된다: {here:?}"
        );
        assert_eq!(
            svc.parked_len("sleepy"),
            0,
            "@here 는 잠든 큐를 만들지 않는다"
        );

        // 같은 발송에 직접 지목을 섞으면 그 행만 파킹된다(층위 분리 — spec §7 혼용 `to`).
        let mixed = send(&svc, "m-mix", from, "alice", &["sleepy", "@here"]).expect("반려 아님");
        assert_eq!(
            rows(&mixed),
            vec![
                ("sleepy".to_string(), SendStatus::Pending, None),
                ("bob".to_string(), SendStatus::Delivered, None),
            ],
            "잠든 행은 pending, 산 행은 배달: {mixed:?}"
        );
    }

    #[test]
    fn at_all_expands_to_the_whole_roster_including_dormant_names() {
        // ★ADR-0121 결정 1 — `@all` = 명부 전원(산 것 + 잠든 것) − 발신자★: 잠든 몫은 파킹되고 그 에이전트가
        //   등장할 때 배달된다. 이름이 "all" 인데 잠든 상대가 빠지면 발신 LLM 이 "전원에게 알렸다" 고 믿는다.
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_b, bob) = live("bob");
        port.set_roster(vec![me, bob]);
        port.set_dormant(&["sleepy"]);

        let all = send(&svc, "m-all", from, "alice", &["@all"]).expect("반려 아님");
        // 행 순서 = 펼침 결과 이름 사전순(spec §5) — 명시 토큰이 없으므로 전부 펼침분이다.
        assert_eq!(
            rows(&all),
            vec![
                ("bob".to_string(), SendStatus::Delivered, None),
                ("sleepy".to_string(), SendStatus::Pending, None),
            ],
            "산 멤버는 배달, 잠든 멤버는 파킹(수용): {all:?}"
        );
        assert_eq!(svc.parked_len("sleepy"), 1, "잠든 몫은 그 이름 큐에 쌓인다");

        // 복원(재등장) → flush 가 그 몫을 집어 배달한다(직접 지목 파킹과 같은 기계).
        let (late_id, late) = live("sleepy");
        port.set_roster(vec![late]);
        port.set_dormant(&[]);
        svc.flush_for("sleepy", late_id);
        assert_eq!(
            svc.ledger_statuses("m-all"),
            vec![DeliveryStatus::Delivered, DeliveryStatus::Delivered],
            "잠든 몫도 등장하면 배달된다"
        );
    }

    #[test]
    fn neither_broadcast_vocabulary_ever_reaches_the_sender() {
        // ★spec §4 발신자 제외 — 두 어휘 모두(ADR-0121 §영향)★. 정본이 spec 이라는 게 load-bearing 이다:
        //   ADR-0111 결정 4 엔 이 문구가 없어서 거기만 보고 구현하면 빠뜨린다.
        // ★잠듦 축도 함께 막는다★: 발신자와 같은 이름의 잠든 프로필이 있으면 `@all` 펼침이 그 이름을 싣고,
        //   "산 쪽이 이긴다" 규칙이 그걸 **발신자 자신**으로 풀어 자기 편지를 배달한다(이름 유일성 ADR-0120 이
        //   막지만, 그 보증이 깨졌을 때 무너지는 방향이 자기 방송 메아리라 여기서 봉인한다).
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_b, bob) = live("bob");
        port.set_roster(vec![me, bob]);
        port.set_dormant(&["alice", "sleepy"]);

        for (msg_id, token) in [("m-all", "@all"), ("m-here", "@here")] {
            let out = send(&svc, msg_id, from, "alice", &[token]).expect("반려 아님");
            assert!(
                !out.iter().any(|r| r.to == "alice"),
                "{token} 펼침에 발신자 행이 있다(자기 방송 메아리): {out:?}"
            );
        }
        assert_eq!(
            svc.parked_len("alice"),
            0,
            "발신자 이름 앞으로 파킹도 생기지 않는다"
        );
        assert!(
            !port.injected_targets().contains(&from.peer_id),
            "발신자 stdin 에 아무것도 쓰이지 않는다"
        );
    }

    #[test]
    fn a_dormant_request_opens_a_contract_whose_deadline_notice_still_fires() {
        // ★spec §7 "잠든 수신자 파킹 — request 면 계약이 열리고 기한 스윕이 정상 발화"★: 파킹은 수용이므로
        //   회신 의무가 성립한다(spec §3 항목 2). 시계는 **발송 기준**이라 복원과 무관하게 기한이 흐른다.
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        port.set_roster(vec![me]);
        port.set_dormant(&["sleepy"]);

        let out = svc
            .handle_send(
                "m-req",
                from,
                "alice",
                &["sleepy".to_string()],
                "해줘",
                Entrance::Mcp,
                &req_meta("1m", 60),
            )
            .expect("반려 아님");
        assert_eq!(out[0].status, SendStatus::Pending);
        assert_eq!(svc.open_request_count(), 1, "잠듦 파킹도 계약을 연다");
        assert_eq!(
            svc.contract_outcome_for_test("m-req", "sleepy"),
            Some("awaiting_reply")
        );

        // 기한 초과 → 발신자에게 notice(계약이 살아 있으므로 스윕이 걷는다).
        svc.sweep(Instant::now() + Duration::from_secs(61));
        assert!(
            svc.ledger_snapshot()
                .iter()
                .any(|(_, f, to, _)| f == NOTICE_SENDER_LABEL && to == "alice"),
            "잠듦 계약도 기한 통지가 나가야: {:?}",
            svc.ledger_snapshot()
        );
    }

    #[test]
    fn two_dormant_profiles_sharing_a_name_block_the_park_as_ambiguous() {
        // ★spec §7 "잠듦 층 동명 차단"(ADR-0116 결정 1/4)★: 같은 이름의 잠든 프로필이 둘이면 파킹하지 않고
        //   `RECIPIENT_AMBIGUOUS` 실패 행이다 — 이름 키로 파킹하면 **먼저 복원된 쪽이 조용히 받는다**.
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        port.set_roster(vec![me]);
        port.set_dormant(&["twin", "twin"]);

        let out = send(&svc, "m1", from, "alice", &["twin"]).expect("행 응답");
        assert_eq!(
            (out[0].status, out[0].code),
            (SendStatus::Failed, Some(FailCode::RecipientAmbiguous)),
            "잠듦 층 동명은 파킹이 아니라 실패 행: {out:?}"
        );
        assert_eq!(svc.parked_len("twin"), 0, "조용히 받는 경로가 없어야");
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Failed],
            "실패 행도 장부 종점으로 남는다"
        );
    }

    #[test]
    fn a_live_agent_wins_over_a_dormant_namesake() {
        // ★spec §7 "산 1 + 잠든 1 동명 → 산 쪽으로 배달"★: 로스터 판정이 먼저다(지금 받을 수 있는 실체가
        //   있으면 그쪽이 수신자다). 잠듦 판정이 끼어들면 배달될 메일이 파킹되거나 모호로 반려된다.
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_d, dup) = live("dup");
        port.set_roster(vec![me, dup]);
        port.set_dormant(&["dup"]);

        let out = send(&svc, "m1", from, "alice", &["dup"]).expect("행 응답");
        assert_eq!(
            out[0].status,
            SendStatus::Delivered,
            "산 쪽이 이긴다: {out:?}"
        );
        assert_eq!(port.injected_bodies().len(), 1);
        assert_eq!(svc.parked_len("dup"), 0);
    }

    // ★이 테스트가 덮는 것 = **busy 축**뿐(착각 금지)★: busy 를 세팅한 채 배달을 단언하므로 발송측·재확인측
    //   **둘 다** 게이트를 묻지 않는지를 잡는다(둘 중 하나만 게이트를 부활시켜도 파킹으로 뒤집힌다). 반면
    //   **순서 축**(백로그가 있을 때 앞지르지 않기)은 여기서 안 본다 — 그쪽은
    //   `inject_failure_parks_pending_without_a_turn_signal`(합류·오래된 순)과
    //   `the_flush_gate_asks_the_same_predicate_the_send_path_asks`(flush측 술어 일치)가 지킨다. 데몬 통합
    //   (`daemon/tests/control_send.rs` — shell 수신자)은 턴 관측도 백로그도 없어 **둘 다** 못 잡는다(실측).
    #[test]
    fn a_live_agent_without_a_turn_signal_is_injected_with_no_gate() {
        // ★spec §7 "턴 신호 없는 백엔드 = idle 게이트 없음"(ADR-0116 결정 7 — `RECIPIENT_UNREACHABLE` 폐기)★:
        //   살아 있고 구조화 출력이 없는 상대 → **busy 판정 없이 바로 주입**되고 행은 `delivered` ·
        //   **파킹 레코드 미생성** · request 면 계약도 정상 오픈 · **선행 파킹분이 없으면** 연속 2건도 둘 다
        //   주입된다(백로그가 있을 때의 결말은 ADR-0121 결정 2 — 위 주석의 순서 축 테스트가 정본) ·
        //   `@all` 펼침에도 **포함**된다(로스터 자격에 capability 조건이 없다).
        // ★게이트가 busy 라고 답해도 무관함★: 이 부류는 게이트를 **묻지 않는다** — 그래서 busy 를 세팅한
        //   채로 배달을 단언한다(옛 술어/게이트 경유를 되살리면 여기서 파킹으로 뒤집혀 잡힌다).
        let (svc, port, gate) = svc_gated();
        let (from, me) = live_sender("alice");
        let (tui_id, tui) = live_no_turn_signal("tui");
        port.set_roster(vec![me.clone(), tui]);
        gate.set_busy(tui_id, 0); // 턴 신호 없는 부류엔 이 값이 보이지 않아야 한다.

        let out = svc
            .handle_send(
                "m1",
                from,
                "alice",
                &["tui".to_string()],
                "해줘",
                Entrance::Mcp,
                &req_meta("10m", 600),
            )
            .expect("행 응답");
        assert_eq!(
            (out[0].status, out[0].code),
            (SendStatus::Delivered, None),
            "턴 신호가 없어도 배달 대상이다(게이트 없이 즉시 주입): {out:?}"
        );
        assert_eq!(port.injected_bodies().len(), 1, "실제로 주입됐다");
        assert_eq!(svc.parked_len("tui"), 0, "busy 파킹이 없는 부류다");
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Delivered],
            "delivered = 실제 주입 시점"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m1", "tui"),
            Some("awaiting_reply"),
            "배달됐으므로 계약도 정상 오픈"
        );

        // 연속 2건 — 앞 건이 즉시 배달돼 큐가 비어 있으므로 둘 다 주입된다(백로그가 있을 때만 합류한다).
        let second = send(&svc, "m2", from, "alice", &["tui"]).expect("행 응답");
        assert_eq!(second[0].status, SendStatus::Delivered);
        assert_eq!(port.injected_bodies().len(), 2, "둘 다 주입된다");

        // `@all` 펼침 포함 — 방송이 이 부류를 빼지 않는다.
        let all = send(&svc, "m3", from, "alice", &["@all"]).expect("행 응답");
        assert_eq!(
            all.iter().map(|r| r.to.clone()).collect::<Vec<_>>(),
            vec!["tui".to_string()],
            "@all 은 턴 신호 없는 산 세션도 펼친다: {all:?}"
        );

        // ★NOT_FOUND 는 로스터·프로필 **둘 다** 비었을 때만이다★.
        port.set_roster(vec![me]);
        let gone = send(&svc, "m4", from, "alice", &["tui"]).expect("행 응답");
        assert_eq!(
            gone[0].code,
            Some(FailCode::RecipientNotFound),
            "로스터·프로필에 없으면 그때는 없음 코드다: {gone:?}"
        );
    }

    #[test]
    fn the_ambiguous_hint_escalates_to_the_user_instead_of_teaching_exact_ids() {
        // ★spec §7 "동명 힌트 문구"(ADR-0116 결정 4/5)★: 두 층 모두 "동명이인 미지원 — 재발송 말고 사용자에게
        //   알려라" 를 말하고, **exact-id 재발송 안내는 어디에도 없다**(동명이인 자체가 미지원이므로 id-구제
        //   흐름을 지원 정책처럼 가르치지 않는다).
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_a, dup1) = live("dup");
        let (_b, dup2) = live("dup");
        port.set_roster(vec![me, dup1, dup2]);
        port.set_dormant(&["twin", "twin"]);

        let live_row = send(&svc, "m1", from, "alice", &["dup"]).expect("행")[0]
            .hint
            .clone()
            .unwrap_or_default();
        let dormant_row = send(&svc, "m2", from, "alice", &["twin"]).expect("행")[0]
            .hint
            .clone()
            .unwrap_or_default();
        for (layer, hint) in [("live", &live_row), ("dormant", &dormant_row)] {
            assert!(
                hint.contains("not supported")
                    && hint.contains("do NOT resend")
                    && hint.contains("tell the user"),
                "{layer} 힌트가 사용자 에스컬레이션을 말해야: {hint}"
            );
            assert!(
                !hint.contains("exact agent id") && !hint.contains("exact id"),
                "{layer} 힌트에 exact-id 재발송 안내가 남아 있다: {hint}"
            );
        }
    }

    #[test]
    fn a_reply_to_a_dormant_requester_is_accepted_and_stays_replied_after_the_deletion() {
        // ★spec §7 회신 계약 규칙 ①②(ADR-0116 결정 2)★: 잠든 요청자에게 회신 → `pending` **수용** + 계약
        //   `replied` 정상 닫힘. 그 뒤 **요청자가 삭제돼 파킹된 회신이 `RECIPIENT_DELETED` 로 치워져도 계약은
        //   `replied` 로 남는다**(되돌림 없음 — 계약 축은 "답을 했나", 배달 축이 "도착했나" 를 적는다).
        let (svc, port) = svc();
        let (boss_from, boss) = live_sender("boss");
        let (w_id, worker) = live("worker");
        port.set_roster(vec![boss, worker.clone()]);

        svc.handle_send(
            "m-req",
            boss_from,
            "boss",
            &["worker".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("배달");

        // 요청자(boss)가 잠든다 — 로스터에서 빠지고 프로필만 남는다.
        port.set_roster(vec![worker.clone()]);
        port.set_dormant(&["boss"]);
        let worker_from = SenderIdentity {
            peer_id: w_id,
            epoch: 0,
        };
        let rep = svc
            .handle_send(
                "m-rep",
                worker_from,
                "worker",
                &["boss".to_string()],
                "했음",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("행 응답");
        assert_eq!(
            rep[0].status,
            SendStatus::Pending,
            "잠든 요청자에게 회신은 파킹된다: {rep:?}"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-req", "worker"),
            Some("replied"),
            "파킹도 수용이다 — 꽂으면 계약 완료(ADR-0116 결정 2)"
        );

        // 요청자 프로필 삭제 → 파킹된 회신이 RECIPIENT_DELETED 로 치워진다.
        //   ★게이트는 **프로필 id** 로 묻는다(리뷰 fix D1)★ — boss 의 세션은 없으므로(잠듦) 통과한다.
        port.set_dormant(&[]);
        let cleanup = svc.handle_profile_deleted(boss_from.peer_id, "boss");
        assert_eq!(cleanup.failed_parked, 1, "파킹된 회신이 종결된다");
        assert_eq!(
            svc.ledger_statuses("m-rep"),
            vec![DeliveryStatus::Failed],
            "배달 축은 사실대로 실패를 적는다"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-req", "worker"),
            Some("replied"),
            "★되돌리지 않는다★ — 계약은 replied 로 남는다(ADR-0116 결정 2 · 종점 되돌림 금지)"
        );
    }

    #[test]
    fn deleting_a_profile_fails_its_parked_mail_and_the_contracts_it_was_waiting_on() {
        // ★spec §7 "프로필 삭제 일괄 정리" ①③④(ADR-0116 결정 3)★:
        //   ① 삭제 ∧ 로스터 부재 → 그 이름 앞 파킹분 `failed`+`RECIPIENT_DELETED`(장부 종점 + **조회 힌트**) ·
        //      그 이름이 **요청자**인 오픈 계약 `reply_failed`
        //   ③ **회신자 쪽**이 삭제된 계약은 유지(발신자는 기한 통지로 무응답을 알게 된다)
        let (svc, port) = svc();
        let (gone_from, _gone_live) = live_sender("gone");
        let (w_id, worker) = live("worker");
        port.set_roster(vec![worker.clone()]);
        port.set_dormant(&["gone"]);
        let worker_from = SenderIdentity {
            peer_id: w_id,
            epoch: 0,
        };

        // (a) gone 이 **요청자**인 계약 — worker 에게 request(즉시 배달).
        svc.handle_send(
            "m-out",
            gone_from,
            "gone",
            &["worker".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("배달");
        // (b) gone 이 **회신자**인 계약 — worker → gone request(잠듦이라 파킹 + 계약 오픈).
        svc.handle_send(
            "m-in",
            worker_from,
            "worker",
            &["gone".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("반려 아님");
        assert_eq!(svc.parked_len("gone"), 1);

        // 프로필 삭제(그 프로필의 세션이 살아 있지 않다) → 정리 발동. 게이트 축 = **프로필 id**(리뷰 fix D1).
        port.set_dormant(&[]);
        let out = svc.handle_profile_deleted(gone_from.peer_id, "gone");
        assert!(!out.skipped_live);
        assert_eq!(out.failed_parked, 1, "파킹분 종결");
        assert_eq!(out.failed_contracts, 1, "요청자 쪽 계약만 종결");
        assert_eq!(svc.parked_len("gone"), 0, "대기열에서 사라진다");

        // 장부 종점 + 조회 코드/힌트(발신자가 사유를 다시 볼 수 있어야 — spec §6).
        assert_eq!(svc.ledger_statuses("m-in"), vec![DeliveryStatus::Failed]);
        let view = svc
            .message_state("m-in", Instant::now())
            .expect("조회 가능");
        assert_eq!(view.rows[0].code, Some("RECIPIENT_DELETED"));
        assert!(
            view.rows[0]
                .hint
                .as_deref()
                .unwrap_or("")
                .contains("resending is pointless"),
            "조회 힌트가 다음 행동을 말해야: {:?}",
            view.rows[0].hint
        );

        // 계약 축: 요청자(gone) 계약은 reply_failed, 회신자(gone) 계약은 **유지**.
        assert_eq!(
            svc.contract_outcome_for_test("m-out", "worker"),
            Some("reply_failed"),
            "그 이름이 요청자인 계약은 실패 종결"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-in", "gone"),
            Some("awaiting_reply"),
            "회신자 쪽이 삭제된 계약은 유지(기한 통지 경로 존속)"
        );
    }

    #[test]
    fn deleting_a_profile_whose_session_is_still_live_cleans_nothing() {
        // ★spec §7 "프로필 삭제 일괄 정리" ②(오동작 방지 회귀)★: 삭제됐지만 세션이 살아 있으면(트리 항목만
        //   지운 상태) 정리하지 않는다 — 그 상대는 곧 idle 이 되면 받을 수 있는 산 수신자다. 정리하면 배달될
        //   메일을 죽이고 성립할 계약을 실패로 적는다(이 개정 취지의 정반대).
        // ★게이트 축 = **프로필 id × 산 명단**(리뷰 fix D1)★ — 이름이 아니다. 그래서 이 테스트는 로스터를
        //   비운 채(= 이름으로는 절대 못 찾는 상태) `is_agent_live(id)` 만 참으로 두고 보호를 단언한다:
        //   프로필이 지워지면 그 산 세션의 canonical 이름이 실제로 바뀌므로(개명·override 소멸) 이름 축
        //   게이트는 이 상황을 **구조적으로** 놓친다.
        let (svc, port, gate) = svc_gated();
        let (from, me) = live_sender("alice");
        let (r_id, recv) = live("recv");
        port.set_roster(vec![me, recv]);
        gate.set_busy(r_id, 0); // 턴 중 → 파킹.
        svc.handle_send(
            "m-req",
            from,
            "alice",
            &["recv".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("파킹");
        assert_eq!(svc.parked_len("recv"), 1);

        // 삭제 시점: **같은 id 가 다른 이름으로** 로스터에 있다(프로필이 지워지면 canonical 이름이
        //   `display_name` → `basename(session.cwd)` 로 바뀌는 그 상태 미러). 그래서 이름 축 게이트는 "recv"
        //   를 찾지 못하고 정리를 발동시키지만, id 축 게이트는 그가 아직 산 것을 안다.
        port.set_roster(vec![
            live_sender("alice").1,
            LiveAgent {
                id: r_id,
                name: "recv-after-delete".to_string(),
                epoch: 0,
                turn_signal: true,
            },
        ]);
        port.set_live_ids(&[r_id]);
        let out = svc.handle_profile_deleted(r_id, "recv");
        assert!(out.skipped_live, "그 프로필의 세션이 살아 있으면 건너뛴다");
        assert_eq!(out.failed_parked, 0);
        assert_eq!(svc.parked_len("recv"), 1, "파킹분 보존");
        assert_eq!(
            svc.contract_outcome_for_test("m-req", "recv"),
            Some("awaiting_reply"),
            "계약 오픈 유지"
        );

        // 턴이 끝나면 정상 배달된다(정리가 죽이지 않았다는 증거).
        gate.clear();
        svc.flush_for("recv", r_id);
        assert_eq!(
            svc.ledger_statuses("m-req"),
            vec![DeliveryStatus::Delivered]
        );
    }

    #[test]
    fn deleting_a_profile_whose_live_session_has_no_turn_signal_cleans_nothing() {
        // ★리뷰 fix D7 — 위 테스트의 빈칸★: 옛 판은 **구조화 로스터만** 만들어, "게이트가 턴 신호 없는 산
        //   세션을 놓친다" 는 부류를 전혀 보지 못했다(그 부류는 옛 로스터 술어에서 아예 빠져 있었다).
        //   4차 로스터는 그 부류를 포함하므로, 그가 write 실패로 남긴 파킹분도 정리에서 보호돼야 한다.
        // ★파킹 진입로★: 이 부류엔 busy 파킹이 없다(게이트를 묻지 않는다) — 유일한 파킹 경로는 **주입 실패**
        //   다. 그래서 fake 의 inject 를 1회 실패시켜 그 상태를 실제 코드 경로로 만든다.
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (tui_id, tui) = live_no_turn_signal("tui");
        port.set_roster(vec![me, tui]);
        port.fail_at(&[0]); // 첫 주입 실패 → 재파킹(spec §5 분기 3).

        let out = svc
            .handle_send(
                "m-req",
                from,
                "alice",
                &["tui".to_string()],
                "해줘",
                Entrance::Mcp,
                &req_meta("10m", 600),
            )
            .expect("행 응답");
        assert_eq!(
            out[0].status,
            SendStatus::Pending,
            "주입 실패는 파킹으로 흡수된다(조용한 유실 금지): {out:?}"
        );
        assert_eq!(svc.parked_len("tui"), 1);

        // 프로필만 지운 상태 — 세션은 (턴 신호 없이) 살아 있다.
        port.set_live_ids(&[tui_id]);
        let cleaned = svc.handle_profile_deleted(tui_id, "tui");
        assert!(
            cleaned.skipped_live,
            "턴 신호가 없어도 산 세션이다 — 게이트가 놓치면 배달될 메일이 죽는다"
        );
        assert_eq!(svc.parked_len("tui"), 1, "파킹분 보존");
        assert_eq!(
            svc.contract_outcome_for_test("m-req", "tui"),
            Some("awaiting_reply"),
            "계약 오픈 유지"
        );
    }

    #[test]
    fn deleting_a_dormant_profile_spares_a_live_namesakes_mail_and_contracts() {
        // ★리뷰 fix N1 — 게이트는 id 로 묻는데 파괴는 **이름 전체**로 한다★: 정리는
        //   `purge_recipient(name)` + `fail_open_requests_from(name)` 이라, 같은 canonical 이름을 지닌 **산
        //   세션**이 따로 있으면 id 축 게이트만으로는 그를 못 지킨다(그 id 는 실제로 죽었으니 통과한다).
        // ★재현 = 지원되는 조작뿐이다(이름 유일성 강제는 ADR-0115 소관 — 아직 없다)★: 프로필 P1·P2 를 같은
        //   `display_name`("boss")으로 개명 → **P2 만** 스폰 → 잠든 P1 삭제. id 축은 P1 이 죽은 걸 맞게 보고
        //   통과하고, 그 뒤 이름 "boss" 로 쓸어 P2 의 파킹 메일이 `RECIPIENT_DELETED`(그 수신자는 삭제되지
        //   않았으므로 **거짓 사유**)로 죽고 P2 가 요청자인 오픈 계약이 `reply_failed` 로 닫힌다 —
        //   ADR-0118 결정 2·spec §5 가 금지한 결말이고 유계 잔여로 문서화된 적도 없다.
        // ★이름 축 가드를 지우면 이 테스트가 즉시 빨개진다★(fail-before 증거: `failed_parked=1` ·
        //   `failed_contracts=1` · `m-in` 이 `Failed` · `m-out` 계약이 `reply_failed`).
        let (svc, port, gate) = svc_gated();
        let (alice_from, alice) = live_sender("alice");
        // P2 — 산 세션. 잠든 P1 과 canonical 이름이 겹친다(개명으로 성립).
        let (boss_id, boss) = live("boss");
        port.set_roster(vec![alice.clone(), boss.clone()]);
        let boss_from = SenderIdentity {
            peer_id: boss_id,
            epoch: 0,
        };

        // (a) 산 P2 가 **요청자**인 오픈 계약 — boss → alice request(즉시 배달).
        svc.handle_send(
            "m-out",
            boss_from,
            "boss",
            &["alice".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("배달");

        // (b) 산 P2 앞 파킹분 — 턴 중이라 "boss" 키로 쌓인다.
        gate.set_busy(boss_id, 0);
        svc.handle_send(
            "m-in",
            alice_from,
            "alice",
            &["boss".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("파킹");
        assert_eq!(svc.parked_len("boss"), 1);

        // 잠든 P1 삭제 — 그 id 의 세션은 **없고**(id 축 통과) 산 명단엔 동명 P2 가 남아 있다.
        let dormant_p1 = PeerId::new_v4();
        port.set_dormant(&[]);
        port.set_live_ids(&[boss_id]); // 산 세션 = P2 뿐(P1 은 없다).
        let out = svc.handle_profile_deleted(dormant_p1, "boss");

        assert!(
            out.skipped_live,
            "동명 산 세션이 있으면 정리를 **통째로** 건너뛴다(fail-safe — 잔여는 단발+TTL 소관): {out:?}"
        );
        assert_eq!(out.failed_parked, 0, "산 동명의 파킹분을 건드리지 않는다");
        assert_eq!(
            out.failed_contracts, 0,
            "산 동명이 요청자인 계약을 닫지 않는다"
        );
        assert_eq!(svc.parked_len("boss"), 1, "대기열 보존");
        assert_eq!(
            svc.ledger_statuses("m-in"),
            vec![DeliveryStatus::Pending],
            "★거짓 사유 금지★ — 삭제되지 않은 수신자의 메일이 RECIPIENT_DELETED 로 죽으면 안 된다"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-out", "alice"),
            Some("awaiting_reply"),
            "산 동명이 요청자인 오픈 계약은 유지된다(대기 목록·기한 통지에서 사라지면 안 된다 — ADR-0118 결정 2)"
        );

        // 턴이 끝나면 정상 배달된다(정리가 죽이지 않았다는 종결 증거).
        gate.clear();
        svc.flush_for("boss", boss_id);
        assert_eq!(svc.ledger_statuses("m-in"), vec![DeliveryStatus::Delivered]);
    }

    #[test]
    fn a_reply_accepted_as_a_park_closes_the_contract_in_the_same_critical_section() {
        // ★리뷰 fix D2 — 수용과 계약 닫기의 원자성(결정론 단언 · sleep 없음)★.
        //
        // ★막는 결말★: ① 스레드 A 가 잠든 요청자에게 회신을 파킹(수용 확정) → 락 해제 ② 그 사이 삭제 정리가
        //   그 계약을 `reply_failed` 로 닫음 ③ 뒤늦은 A 의 닫기가 `AlreadyClosed` 로 물러남 → "회신은 수용
        //   (`pending`)됐는데 계약은 회신 실패" (재가된 규칙 "수용 = 완료, 되돌림 없음" 위반).
        // ★결정론 장치(sleep·스레드 없음)★: 서비스가 **수용을 기록한 그 락을 쥔 채** 장부를 보여 주는 훅
        //   안에서 계약 상태를 관측한다. 닫기가 별도 락으로 빠지면 이 시점 관측이 `awaiting_reply` 라 즉시
        //   빨개진다 — 최종 상태만 보는 테스트는 경합이 없으면 두 배선을 구분하지 못한다(그게 이 장치의 이유).
        let (svc, port) = svc();
        let (boss_from, boss) = live_sender("boss");
        let (w_id, worker) = live("worker");
        port.set_roster(vec![boss, worker.clone()]);
        svc.handle_send(
            "m-req",
            boss_from,
            "boss",
            &["worker".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("배달");

        // 요청자가 잠든다 → 회신은 파킹(수용)된다.
        port.set_roster(vec![worker]);
        port.set_dormant(&["boss"]);

        // 임계구역 안에서 본 계약 상태를 모은다(훅은 넘겨받은 장부만 읽는다 — 락 재취득 금지).
        let in_lock = Arc::new(StdMutex::new(Vec::<Option<&'static str>>::new()));
        let seen = in_lock.clone();
        svc.set_accept_hook_for_test(Arc::new(move |ledger| {
            seen.lock()
                .unwrap()
                .push(ledger.contract_outcome_for_test("m-req", "worker"));
        }));

        let rep = svc
            .handle_send(
                "m-rep",
                SenderIdentity {
                    peer_id: w_id,
                    epoch: 0,
                },
                "worker",
                &["boss".to_string()],
                "했음",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("행 응답");
        assert_eq!(rep[0].status, SendStatus::Pending, "회신이 파킹 수용됐다");
        assert_eq!(
            *in_lock.lock().unwrap(),
            vec![Some("replied")],
            "★수용을 기록한 그 락 안에서 이미 닫혀 있어야 한다★ — 여기가 `awaiting_reply` 면 닫기가 다른 락으로 빠졌다는 뜻이고, 그 창에 삭제 정리가 끼면 '수용됐는데 회신 실패' 가 된다(리뷰 fix D2)"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-req", "worker"),
            Some("replied"),
            "수용 = 완료 — 계약은 그 임계구역에서 닫혔다"
        );

        // 이제(락 밖) 삭제 정리가 돌아도 종점을 되돌리지 않는다.
        port.set_dormant(&[]);
        let cleaned = svc.handle_profile_deleted(boss_from.peer_id, "boss");
        assert_eq!(cleaned.failed_parked, 1, "파킹된 회신은 사실대로 실패 종결");
        assert_eq!(
            svc.contract_outcome_for_test("m-req", "worker"),
            Some("replied"),
            "계약 축은 되돌리지 않는다(ADR-0116 결정 2)"
        );
    }

    #[test]
    fn a_reply_that_goes_out_in_its_own_call_closes_the_contract_in_the_enqueue_lock() {
        // ★리뷰 fix D2 의 7차 판(ADR-0125)★: 수용 확정 지점이 **적재 락 하나**로 합쳐졌다 — 발송이 예외 없이
        //   큐에 적재되므로 "수용 기록과 계약 닫기가 원자적" 이 **전 갈래**에서 그 한 락으로 성립한다.
        //   ★단언 축의 이전★: 옛 판은 훅이 두 번 발화하고 **마지막**(배달 기록 락)이 `replied` 이길 요구했다
        //   — 그 두 번째 발화 지점은 직발송 폐지와 함께 사라졌다. 이제 요구는 더 세다: **첫 발화(적재 락)에서
        //   이미 닫혀 있어야 한다**. 여기가 `awaiting_reply` 면 닫기가 그 락 밖으로 빠졌다는 뜻이고, 그 창에
        //   삭제 정리가 끼면 "수용됐는데 회신 실패" 가 된다. 뒤이은 드레인이 배달로 승격시키는 것은 배달
        //   축이라 계약 축의 원자성과 무관하다.
        let (svc, port) = svc();
        let (boss_from, boss) = live_sender("boss");
        let (w_id, worker) = live("worker");
        port.set_roster(vec![boss, worker]);
        svc.handle_send(
            "m-req",
            boss_from,
            "boss",
            &["worker".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("배달");

        let in_lock = Arc::new(StdMutex::new(Vec::<Option<&'static str>>::new()));
        let seen = in_lock.clone();
        svc.set_accept_hook_for_test(Arc::new(move |ledger| {
            seen.lock()
                .unwrap()
                .push(ledger.contract_outcome_for_test("m-req", "worker"));
        }));

        let rep = svc
            .handle_send(
                "m-rep",
                SenderIdentity {
                    peer_id: w_id,
                    epoch: 0,
                },
                "worker",
                &["boss".to_string()],
                "했음",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("행 응답");
        assert_eq!(
            rep[0].status,
            SendStatus::Delivered,
            "그 호출의 드레인이 냈으므로 delivered(경로는 적재-후-드레인)"
        );
        assert_eq!(
            *in_lock.lock().unwrap(),
            vec![Some("replied")],
            "★적재 락 안에서 이미 닫혀 있어야 한다★ — `awaiting_reply` 로 관측되면 닫기가 그 락 밖으로 빠졌다는 뜻이다(리뷰 fix D2 · ADR-0125로 확정 지점이 하나가 됐다)"
        );
    }

    #[test]
    fn a_dormant_contract_binds_the_real_id_when_the_parked_request_is_injected() {
        // ★리뷰 fix D4 전반부★: 잠든 수신자의 계약은 `recipient_id = None` 으로 열린다 — 그 구간엔 **이름
        //   폴백만** 계약을 닫을 수 있다. 그래서 복원 후 그 파킹분이 **실제로 주입되는 시점**에 착지 id 를
        //   계약에 박아 넣어야 한다(그 뒤로는 id 축이 살아 동명 오폐쇄가 불가능해진다).
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        port.set_roster(vec![me]);
        port.set_dormant(&["sleepy"]);
        svc.handle_send(
            "m-req",
            from,
            "alice",
            &["sleepy".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("파킹");
        assert_eq!(
            svc.contract_recipient_id_for_test("m-req", "sleepy"),
            None,
            "잠듦 계약은 산 incarnation 이 없어 id 없이 열린다"
        );

        // 복원 → flush 로 주입되는 순간 id 가 박힌다.
        let (late_id, late) = live("sleepy");
        port.set_roster(vec![late]);
        port.set_dormant(&[]);
        svc.flush_for("sleepy", late_id);
        assert_eq!(
            svc.contract_recipient_id_for_test("m-req", "sleepy"),
            Some(late_id),
            "주입 시점에 착지 incarnation 의 id 를 계약에 박는다(의무는 봉투를 받은 자를 따른다)"
        );
    }

    #[test]
    fn a_namesake_cannot_close_a_dormant_contract_it_never_received() {
        // ★리뷰 fix D4 후반부★: 잠든 수신자의 계약(`recipient_id = None`)은 이름 폴백으로 닫히는데, 그 이름을
        //   가진 **산 세션이 둘 이상**이면 누가 실제로 요청을 받았는지 알 수 없다 → 이름 폴백을 **금지**하고
        //   무동작(계약 오픈 유지)한다. 잘못 닫는 것보다 안 닫는 게 낫다(귀속 날조 금지).
        let (svc, port) = svc();
        let (boss_from, boss) = live_sender("boss");
        port.set_roster(vec![boss.clone()]);
        port.set_dormant(&["twin"]);
        svc.handle_send(
            "m-req",
            boss_from,
            "boss",
            &["twin".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("파킹");
        assert_eq!(
            svc.contract_outcome_for_test("m-req", "twin"),
            Some("awaiting_reply")
        );

        // 같은 이름의 산 세션이 **둘** 등장 — 어느 쪽도 그 계약을 닫을 수 없다.
        let (twin_a, a) = live("twin");
        let (_twin_b, b) = live("twin");
        port.set_roster(vec![boss.clone(), a, b]);
        port.set_dormant(&[]);
        let rep = svc
            .handle_send(
                "m-rep",
                SenderIdentity {
                    peer_id: twin_a,
                    epoch: 0,
                },
                "twin",
                &["boss".to_string()],
                "했음",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("행 응답");
        assert_eq!(
            rep[0].status,
            SendStatus::Delivered,
            "회신 **배달**은 막지 않는다(계약 쪽만 무동작 — spec §3 항목 7-②): {rep:?}"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-req", "twin"),
            Some("awaiting_reply"),
            "동명 다수에서는 이름 폴백을 금지한다(오폐쇄 금지 — 리뷰 fix D4)"
        );

        // 동명이 하나로 줄면 정상 경로로 닫힌다(폴백 자체를 죽인 게 아님을 못 박는다).
        let (solo, solo_agent) = live("twin");
        port.set_roster(vec![boss, solo_agent]);
        svc.handle_send(
            "m-rep2",
            SenderIdentity {
                peer_id: solo,
                epoch: 0,
            },
            "twin",
            &["boss".to_string()],
            "했음",
            Entrance::Mcp,
            &reply_meta("m-req"),
        )
        .expect("행 응답");
        assert_eq!(
            svc.contract_outcome_for_test("m-req", "twin"),
            Some("replied"),
            "동명이 사라지면 이름 폴백이 정상 작동한다"
        );
    }

    #[test]
    fn every_fail_code_has_a_frozen_wire_string() {
        // ★뮤테이션 프로브 D9-d★: `FailCode::as_str` 의 문자열을 **서로 맞바꿔도** 메시징 전 테스트가 초록
        //   이었다(데몬 테스트 2개만 잡았다). 이 값들은 발신 LLM 이 분기하는 **안정 계약**(spec §6)이라
        //   커널에서 직접 봉인한다. 새 variant 를 추가하면 이 매치가 컴파일 에러로 갱신을 강제한다.
        for (code, wire) in [
            (FailCode::RecipientNotFound, "RECIPIENT_NOT_FOUND"),
            (FailCode::RecipientAmbiguous, "RECIPIENT_AMBIGUOUS"),
            (FailCode::MailboxFull, "MAILBOX_FULL"),
            (FailCode::RequestCapacity, "REQUEST_CAPACITY"),
            (FailCode::RecipientDeleted, "RECIPIENT_DELETED"),
        ] {
            assert_eq!(code.as_str(), wire, "{code:?} 의 wire 문자열이 바뀌었다");
            // 컴파일러가 variant 누락을 잡게 한다(새 코드가 오면 이 match 가 깨진다).
            match code {
                FailCode::RecipientNotFound
                | FailCode::RecipientAmbiguous
                | FailCode::MailboxFull
                | FailCode::RequestCapacity
                | FailCode::RecipientDeleted => {}
            }
        }
    }
}
