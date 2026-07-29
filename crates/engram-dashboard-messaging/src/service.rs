//! service — MessagingService: 순수 구조(mailbox·ledger)를 tokio 위에서 발송 파이프라인에 엮는
//! 오케스트레이터(S18 메시징 v1 · ADR-0103/0104 · **ADR-0111/0112/0114 발송 개편**).
//!
//! ★역할★: **다중 수신자 발송**(spec §5)과 등장/idle flush(ADR-0104), TTL sweep, **idle 게이트**,
//!   **회신 계약**(request 장부 오픈·회신 닫기·기한 초과 notice)을 담당한다. 발송 진입점은
//!   `handle_send` **하나**다 — 개인·다중·`@all` 이 전부 같은 레일을 탄다(경로 1벌, ADR-0111 결정 2/4).
//!
//! ★수신자 1명당 3분기(spec §5 — 다중 수신자는 이 판정을 수신자마다 1회씩 = fan-out)★:
//!   1. **입구 반려 → 수신자별 실패 행**(ADR-0111 결정 1): 발송 순간 로스터 스냅샷에 그 이름이 없으면
//!      `failed` + `RECIPIENT_NOT_FOUND`(미스폰·죽음·잠든 세션 전부 동일 — wake 없음), 동명 다수면
//!      `RECIPIENT_AMBIGUOUS`. **파킹하지 않는다** — "없는 이름 파킹"(스폰 전 선지시)은 v1 비지원이다.
//!   2. **배달** — 로스터에 있고 idle + 선행 파킹분 없음 → 즉시 주입 `delivered`(실제 주입 시점, ADR-0104).
//!   3. **파킹 `pending`** — 로스터엔 있는데 지금 못 넣는 경우: **busy 계열**(턴 중이거나 선행 파킹분 뒤
//!      FIFO 합류) **+ 주입(write) 실패**. 보관함 초과면 **그 수신자만** `failed` + `MAILBOX_FULL`이다
//!      (전체 반려로 승격하지 않는다 — spec §5 부분 진행).
//!   등장(스폰/epoch 교체)·**턴 종료(idle 전이)**·flush 시 파킹분을 **오래된 순 일괄** 주입한다(각 메시지
//!   개별 봉투, ADR-0104).
//!
//! ★부분 진행(ADR-0111 결정 3)★: 수신자별 판정은 서로 독립이다 — 일부가 실패해도 나머지에겐 그대로
//!   배달된다. **전체 반려는 발송 자체의 문제뿐**이다: 이 함수가 내는 것은 주소 공간 오류
//!   (`GroupNotFound`/`GroupEmpty` — ADR-0114 결정 3 층위)와 `IdCollision` 이고, 인자 오류·본문 과대는
//!   입구(ingress)가 먼저 반려한다. **전원이 실패해도 응답 shape 은 그대로**(전 행 `failed`)다.
//!
//! ★`@`주소 = 해석 매크로(ADR-0111 결정 4 · ADR-0112 결정 1)★: `@all` 은 발송 순간 로스터 스냅샷에서
//!   **발신자를 뺀** 이름들로 펼쳐져 명시 지목과 **합집합**으로 합류한다(`groups::BuiltinGroups`). 그룹
//!   전용 fan-out 경로·죽은 멤버 skip·발송 순간 `(id,epoch)` 결박은 **전부 폐지**됐다 — 되살리면 ADR-0111
//!   위반이다. 사용자 정의 그룹(등록 명단·관리 툴)도 없다.
//!
//! ★로스터 스냅샷은 발송 1회당 **딱 한 장**(불변식 — ADR-0111 결정 2)★: `handle_send` 첫 줄에서 뜬 한
//!   장으로 `@` 펼침·존재/동명 판정·busy 판정을 **전원 일괄** 처리한다. 수신자별로 다시 뜨면 해석 도중
//!   명단 변동에 의한 반쪽 판정이 재발한다(옛 그룹 스냅샷 원칙에서 존치된 조각).
//!
//! ★봉투 `to` 속성의 동결(spec §1 · load-bearing)★: `to` 는 **수용 판정된(= 실패 행이 아닌) 수신자가 2인
//!   이상일 때만** 실리고, 값은 **전 수신자의 수용 판정이 끝난 뒤 1회 확정**해 그 발송의 모든 봉투(즉시
//!   배달분·파킹분)에 동일하게 싣는다. 수신자별 루프 도중 wrap 하면 첫 수신자가 미완성 `to` 를 받는다.
//!   그래서 판정(pass A) → `to` 확정 → 파킹(pass B) 순서가 **한 락 구간 안에서** 강제된다 — 파킹 payload 가
//!   그 값을 flush 까지 날라야 하기 때문이다(재계산 금지).
//!
//! ★idle 게이트 seam(C2 · ADR-0104 결정 3)★: "수신자가 턴 중인가" 는 `BusyGate`(busy.rs) 너머로
//!   묻는다 — 운영은 `BusyTracker`(출력 스트림 tap 이 턴 이벤트를 관측), 단위 테스트는 가짜 게이트를 끼운다.
//!   게이트를 안 꽂으면 `AlwaysIdleGate`(= 즉시 주입)로 폴백한다(관측 불가 백엔드 폴백과 같은 값).
//!
//! ★순서 보장의 범위(finding 8 · round-7 보정 · load-bearing)★: "오래된 순" 은 **한 flush 배치 내부**
//!   (+ 재파킹 merge 로 배치 간 이월 시 오래된 것 우선)에서 보장한다 — spec §5 가 약속하는 건 **배치 순서**지
//!   전 출처를 아우르는 global total order 가 아니다. 직발송·flush 가 서로를 앞지르지 않도록 합류 판정이
//!   in-flight 까지 보고(`mailbox::has_pending_ahead`), flush 는 같은 수신자에 대해 겹쳐 돌지 않는다
//!   (`flush_for` 0단계) — 즉 **한 수신자가 보는 순서**는 지켜진다. 남는 수용분은 둘이다: ① 합류 판정(락 안)과
//!   그 뒤 inject(락 밖) 사이 마이크로초 창 ② **서로 다른 수신자** 사이의 전역 순서. 둘 다 inject 를 락 안으로
//!   넣어야만 닫히므로(락 규율 정면 위반) 사람 대화 수준 메시지율에서 의도적으로 수용한다.
//!
//! ★단일 락(load-bearing — ADR-0006 정신)★: Mailbox+Ledger 를 **하나의 `Mutex<MessagingState>`** 뒤에 둔다.
//!   락 순서 위험이 없고(락 하나) 메시지율이 극히 낮아(사람 대화 수준) 경합이 무의미하다.
//!   ★절대 규율★: 이 락을 **든 채로 `DeliveryPort`(inject/roster)를 부르지 않는다** — 락 아래에서 결정할
//!   것(파킹/주입 대상 수집)을 먼저 끝내고 락을 놓은 뒤 DeliveryPort(외부 호출)를 부른다. 이걸 어기면
//!   inject 가 내부에서 다른 락(sessions RwLock 등)을 잡아 락 순서 역전·데드락 위험이 생긴다.
//!   (`@`주소 해석기 `BuiltinGroups` 는 상태가 없어 락 밖 필드로 둔다 — 로스터 스냅샷도 락 밖이라 자연스럽다.)
//!
//! ★delivery seam(ADR-0012 · 헤드리스 테스트)★: 호스트의 에이전트 실물을 직접 부르지 않고 `DeliveryPort`
//!   트레잇 너머로 부른다 — 운영 어댑터는 호스트 소유(데몬 `messaging_host::ManagerDeliveryPort`), 단위 테스트는
//!   `FakeDeliveryPort` 를 끼워 3분기·flush·sweep 을 결정적으로 단언한다.
//!
//! ★봉투 = 주입 시점 조립(단일 wrap point, ADR-0096)★: 파킹은 **감싸지 않은 body + 발신자 이름 + 발송
//!   메타**를 저장하고, 봉투는 **주입할 때** `wrap_message`/`wrap_notice`(이 crate `envelope.rs` 단일 wrap
//!   point)로 만든다. 왜: 파킹과 flush 사이 봉투 포맷(colon/xml 전역 스위치)이 바뀔 수 있고, 그때 flush 는
//!   **현재** 포맷으로 감싸야 한다. 그래서 raw body + 속성 재료를 나르고(`ParkPayload`) 조립은 주입 순간
//!   한 곳에서 — 즉시 배달과 늦은 배달의 봉투가 **같아야** 한다.
//!
//! ★회신 계약(spec §3 · ADR-0103 결정 2/3 · ADR-0111 결정 5)★:
//!   - **request 발송** → 수용된(배달/파킹) **수신자마다 독립 계약 1건**을 연다(`Ledger::open_request`,
//!     계약 키 = `(메시지 id, 수신자)`). 예약은 배달 시도 **전에** 하고(발송 기준 시계 — spec §3), 그
//!     수신자가 실패 행으로 끝나면 그 계약만 되돌린다 — 안 되돌리면 배달된 적 없는 요청이 기한 초과 notice 를
//!     쏜다(유령 타임아웃). **계약을 못 연 수신자에겐 메시지도 배달하지 않는다**(`REQUEST_CAPACITY` —
//!     추적 없는 request 배달 금지, ADR-0114 영향 절).
//!   - **회신 발송**(`reply_to`) → 정상 배달/파킹 뒤 `Ledger::close_on_reply`(엄격 매칭 + **회신자로 계약
//!     선택**). 매칭 실패는 **배달에 영향 없다** — 메시지는 그대로 가고 계약만 안 닫힌다.
//!   - **기한 초과** → sweep 이 `due_timeouts` 로 걷어 **발신자에게** `<notice>` 를 보낸다(계약별 1건 —
//!     병합 없음, ADR-0114 결정 2). 배달은 `ParkKind::Notice` 파킹 + flush 도어벨로 일원화한다 — 통지는
//!     **전용 유계 레인**(`mailbox::NOTICE_CAP` = 64)에서 회계된다.
//!
//! 워크스페이스 crate import 0(ADR-0110 — 컴파일러 강제).
// ADR-0103
// ADR-0104
// ADR-0111
// ADR-0112
// ADR-0114

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::busy::{AlwaysIdleGate, BusyGate};
use super::envelope::{
    new_msg_id, wrap_message, wrap_notice, DeliveryObservation, Entrance, EnvelopeFields,
    EnvelopeFormat,
};
use super::groups::{normalize_group_name, BuiltinGroups, GroupError, GroupSource};
use super::ledger::{
    DeliveryStatus, DropOutcome, DueTimeout, Ledger, OpenOutcome, ReplyOutcome,
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
// ADR-0108 (예약 회수 보증 계층 — 기준 = 생존)

/// ★발송 메타(회신 계약 + 봉투 `to` 축)★ — ingress 가 구문 검증을 마친 인자를 서비스가 쓸 형태로 정규화한 값.
///
/// ★raw 표기와 파싱값을 **둘 다** 나르는 이유(load-bearing)★: 봉투 속성 `reply-by` 는 발신자가 쓴 **표기
///   그대로**(`"10m"`) 보여야 하고(spec §1 — 수신 LLM 이 읽는 계약), 장부 타이머는 **절대 기한**을 계산할
///   `Duration` 이 필요하다(spec §3 "데몬이 절대시각 환산"). 한쪽만 나르면 다른 쪽에서 재파싱·재렌더가
///   생겨 표기가 미묘하게 달라진다(`60m` → `1h`).
/// ★`Default` = 통보★: 전 필드 비활성이 곧 기본 메시지(type 없음)다.
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
    /// 이 발송이 봉투에 붙일 속성(spec §1 노출 원칙 — **행동을 바꾸는 필드만**).
    ///
    /// - request → `id`(회신에 필요) + `type="request"` + (있으면) `reply-by`.
    /// - 회신 → `in-reply-to` 만(발신 인자 `reply_to` 가 수신 속성 `in-reply-to` 로 나타난다 — spec §1 표기 매핑).
    /// - 수신자 2인 이상 → `to`(위 필드 주석).
    /// - 통보 1:1 → 전부 None(속성 없는 plain `<message from>`).
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
            // ★수신자 2인 이상일 때만 Some★(위 `to_attr` 주석). 1:1 발송은 속성이 통째로 생략된다.
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
    /// 이 수신자의 결말(spec §6 상태 어휘).
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
    /// 파킹됨 — ledger `pending`. idle 진입·재등장 시 일괄 flush 된다.
    Pending,
    /// ★그 수신자만 실패(ADR-0111 결정 3 신설)★ — 나머지 수신자에겐 그대로 배달된다(부분 진행).
    Failed,
}

/// 수신자별 실패 코드(spec §6 행 코드 — 발송 단위 반려 코드와 **다른 축**이다).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailCode {
    /// 발송 순간 로스터에 그 이름이 없음(미스폰·죽음·**잠든 세션** 전부 동일 — ADR-0111 결정 1 · ADR-0112 결정 2).
    RecipientNotFound,
    /// 같은 이름의 산 에이전트가 둘 이상 — 누구에게 보낼지 데몬이 고를 근거가 없다(ADR-0114 결정 4 과도기 규칙).
    RecipientAmbiguous,
    /// 그 수신자의 보관함(message 레인 100건)이 가득 참 — **회수 시도 없이 즉시**(ADR-0114 결정 1).
    MailboxFull,
    /// 그 수신자의 회신 계약을 열 수 없음(오픈 계약 상한 512, ADR-0108). **그 수신자에겐 배달도 하지 않는다** —
    ///   request 의 본질이 회신 추적이라 추적 없는 배달은 계약 위반이다(ADR-0114 영향 절).
    RequestCapacity,
}

impl FailCode {
    /// wire 코드 문자열(안정 계약 — 발신 LLM 이 이 값으로 분기한다, spec §6).
    pub fn as_str(self) -> &'static str {
        match self {
            FailCode::RecipientNotFound => "RECIPIENT_NOT_FOUND",
            FailCode::RecipientAmbiguous => "RECIPIENT_AMBIGUOUS",
            FailCode::MailboxFull => "MAILBOX_FULL",
            FailCode::RequestCapacity => "REQUEST_CAPACITY",
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
    /// 실제 수용된 바이트 수(성공 시 `bytes_requested` 와 동일).
    pub bytes_written: usize,
    /// 이 유저 턴의 session-level replay-dedup 키.
    pub msg_uuid: uuid::Uuid,
    /// write 를 집행한 incarnation 의 epoch.
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
    /// 배달-경계 관측 레코드 1건을 호스트 싱크에 적재한다(ADR-0088). 운영 = no-op, 하네스 = 회수.
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
    /// 완성된 봉투 바이트를 수신자에게 주입한다(= 수신자 stdin write). 성공 시 `InjectReceipt`(관측 상관용),
    /// 실패 시 에러 문자열(도달 불가·write 실패 — 상위가 파킹으로 처리). 봉투 조립은 상위가 이미 끝냄.
    ///
    /// ★이건 **incarnation 무조건** 주입이다★: 그 PeerId 가 지금 가리키는 세션에 그대로 쓴다 — 재시작으로
    ///   epoch 이 올랐어도 새 incarnation 에 착지한다. 그게 **옳다**: 주소 단위는 이름이고 재스폰 이어받기가
    ///   기능이다(ADR-0101). ★옛 `inject_if_epoch`(발송 순간 incarnation 결박용 조건부 주입)는 제거됐다★ —
    ///   결박 자체가 폐지됐으므로(ADR-0111 결정 6) 조건부 write 를 쓸 호출자가 없다. 되살리려면 먼저
    ///   "이 편지는 발송 순간 화신에게만" 을 v2 개인 메일 옵션으로 정식 재론해야 한다(spec §8).
    // ADR-0111 (결박 폐지 — 조건부 주입 동사 제거)
    fn inject(&self, to_id: PeerId, bytes: &[u8]) -> Result<InjectReceipt, String>;

    /// 지금 살아있고(Running|Exiting) **제어 채널로 도달 가능한**(structured 출력) 에이전트 로스터.
    /// (name, id, epoch) 스냅샷 — resolve(이름→id)·flush(등장/epoch 교체 감지)·`@all`(C4) 공용.
    /// ★도달성 포함★: TUI(비-structured)는 stdin 주입이 유효 라인이 안 되므로 여기서 제외한다(spec §5
    ///   "unreachable → 파킹" 의 판정을 로스터 단계로 당긴다 — 비-도달 이름은 애초에 "산 수신자"가 아님).
    fn live_reachable_agents(&self) -> Vec<LiveAgent>;

    /// id → canonical 표시 이름(봉투 sender·수신자 이름 단일 출처, ADR-0101). 없으면 None.
    fn canonical_name(&self, id: PeerId) -> Option<String>;
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
    fn request_flush(&self, id: PeerId);
}

/// 로스터 항목 — 살아있고 도달 가능한 한 에이전트의 (id, 이름, epoch) 스냅샷.
#[derive(Debug, Clone)]
pub struct LiveAgent {
    pub id: PeerId,
    pub name: String,
    pub epoch: u32,
}

/// ★단일 락 아래 상태(load-bearing — ADR-0006)★: mailbox+ledger+groups 를 한 Mutex 뒤에 함께 둔다.
///   락 순서 위험 제거 + 극저 메시지율이라 경합 무의미.
struct MessagingState {
    mailbox: Mailbox,
    ledger: Ledger,
    /// ★유예된 flush 재-도어벨 장부(round-7 · round-8 보정 · load-bearing)★ — 키 = park 큐 이름,
    /// 값 = 그 큐에 대해 **물러난 모든 도어벨 id**(중복 제거된 집합).
    ///
    /// `flush_for` 는 같은 수신자에 대해 **겹쳐 돌지 않는다**(앞 배치가 락 밖에서 주입 중이면 뒤 배치가
    /// 그 잔여를 앞지른다 — 순서 역전이 한 층 아래에서 재발). 그래서 뒤 flush 는 드레인 없이 물러나는데,
    /// 물러나기만 하면 그 깨우기가 **증발한다**(lost wakeup — C1 finding 3 과 같은 실패 모드). 여기에 사실을
    /// 남겨 두면 **영수증을 쥔 쪽**(진행 중인 flush)이 정산을 마치고 나가면서 도어벨을 다시 눌러 준다.
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

/// MessagingService — 순수 구조를 발송 파이프라인에 엮는 데몬측 오케스트레이터(C1).
///
/// ★공유★: 데몬 부팅에서 `Arc<MessagingService>` 로 만들어 ingress(handle_send)·flush observer·sweep
///   task 가 같은 인스턴스를 공유한다(lib.rs run()).
pub struct MessagingService {
    /// mailbox+ledger 단일 락(모듈 헤더 규율). 이 락을 든 채 port 호출 금지.
    state: Mutex<MessagingState>,
    /// ★`@`주소 해석 소스(seam — ADR-0104 결정 1 · ADR-0112 결정 1)★. v1 은 내장 `@all` 하나뿐이고 **상태가
    ///   없어** 락 밖 필드다(저장형 그룹이 사라져 공유 가변 상태가 없다 — groups.rs 헤더).
    groups: BuiltinGroups,
    /// 배달 seam(ADR-0012) — inject/roster/canonical_name.
    port: Arc<dyn DeliveryPort>,
    /// 봉투 포맷 전역 상태 + 배달 관측 싱크 거처(ADR-0096/0088). inject 마다 format 을 읽고 관측 레코드를
    ///   발행한다 — handle_send 와 **같은 Arc**(전역 상태 하나).
    registry: Arc<dyn ControlPlanePort>,
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
    pub fn new(port: Arc<dyn DeliveryPort>, registry: Arc<dyn ControlPlanePort>) -> Self {
        Self::new_gated(port, registry, Arc::new(AlwaysIdleGate))
    }

    /// port + registry + **idle 게이트** 주입 생성자(C2). 운영 조립(lib.rs)과 게이트 단위 테스트가 쓴다.
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

    /// ★발송 진입점(spec §5 — 다중 수신자 fan-out, **경로 1벌**)★. 입구(ingress)가 인자 검증·auth 를 마친 뒤
    ///   부르는 유일한 발송 함수다. 개인 1:1·여러 명 직접 지목·`@all` 이 전부 이 한 레일을 탄다
    ///   (ADR-0111 결정 2/4 — 그룹 전용 fan-out 분기는 폐지됐다).
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
    /// ★수신자별 3분기(spec §5) — 모듈 헤더가 정본★. 이 함수의 구조는 그 판정을 **두 패스**로 나눈다:
    ///   - **pass A(락 안)**: 수신자마다 입구 반려/계약 오픈/파킹 여부를 판정한다. **아직 파킹하지 않는다.**
    ///   - **`to` 동결**: 수용 판정이 전원 끝난 뒤 봉투 `to` 값을 1회 확정한다(spec §1 — 실패 행 제외).
    ///   - **pass B(같은 락 안)**: 그 값을 실어 실제로 파킹한다(파킹 payload 가 `to` 를 flush 까지 나른다).
    ///   ★왜 두 패스인가(load-bearing)★: 파킹은 봉투 재료를 저장하는데 `to` 는 전원 판정 전엔 알 수 없다.
    ///   한 패스로 하면 첫 수신자의 파킹분이 **미완성 `to`** 를 굳혀 나중 배달분과 봉투가 갈린다. 두 패스를
    ///   **같은 락 구간**에 두는 이유는 그 사이에 큐가 "비어 보이는 창" 이 생기면 동시 직발송이 FIFO 를
    ///   앞지르기 때문이다(`flush_for` 주석과 같은 근거).
    ///
    /// ★회신 계약(spec §3 · ADR-0111 결정 5)★: `meta.request` 면 **수용된 수신자마다** 계약을 연다(키 =
    ///   `(msg_id, 수신자)`). 계약을 못 연 수신자는 `REQUEST_CAPACITY` 실패 행이고 **배달하지 않는다**.
    ///   `meta.reply_to` 면 발송이 접수된 뒤 회신자 기준 엄격 매칭으로 계약을 닫는다(실패해도 배달 무영향).
    ///
    /// ★락 규율(모듈 헤더)★: 로스터 조회·해석은 port 호출(락 밖) → **한 락 구간**에서 판정·계약·파킹 →
    ///   락 해제 → 도어벨·주입(락 밖) → 결과에 따라 다시 짧게 락.
    // ADR-0111 (다중 수신자 fan-out · 부재 반려 · 부분 진행)
    // ADR-0114 (MAILBOX_FULL 행 실패 — 회수 시도 없음)
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
        // ★봉투 `to` 는 입구가 채우지 않는다(spec §1 동결 규칙)★ — 전 수신자 수용 판정 뒤 여기서 1회 확정.
        debug_assert!(
            meta.to_attr.is_none(),
            "봉투 to 는 전 수신자 수용 판정 뒤 handle_send 가 1회 확정한다(spec §1)"
        );
        debug_assert!(
            !to.is_empty(),
            "빈 수신자 목록은 입구가 INVALID_SEND_ARGS 로 반려한다(spec §6)"
        );

        // 1) ★로스터 스냅샷 1장(락 밖 — port 호출)★. 이 한 장으로 `@` 펼침·존재/동명·busy 판정을 전원
        //    일괄 처리한다(ADR-0111 결정 2 불변식 — 수신자별로 다시 뜨면 반쪽 판정이 재발한다).
        let roster = self.port.live_reachable_agents();

        // 2) 주소 해석(순수) — 트림 → `@` 펼침(발신자 제외) → 명시 지목과 합류 → 중복 제거 → 행 순서 확정.
        //    `@`주소 오류는 여기서 **발송 단위 반려**로 끝난다(부작용 0 — ADR-0114 결정 3 층위).
        let addressing = resolve_addressing(&self.groups, to, from, &roster)?;
        // 이 발송이 남길 배달기록 총수 = **실패 행 포함** 수신자 수(spec §5 "실패 수신자도 장부에 남는다").
        //   조회의 잘림 판정(`may_be_truncated`)이 이 값과 남은 행 수를 비교하므로 모든 행이 같은 값을 든다.
        let expected_rows = u16::try_from(addressing.recipients.len()).unwrap_or(u16::MAX);
        let roster_names = live_names_hint(&roster);

        let now = Instant::now();
        let mut plans: Vec<RecipientPlan> = Vec::with_capacity(addressing.recipients.len());
        // 락 안에서 모으고 락 밖에서 찍는 사실들(모듈 헤더 락 규율 — 락 보유 중 tracing 금지).
        let mut park_effects = ParkSideEffects::default();
        let mut retirements = RetirementLog::default();
        let mut duplicate_contracts: Vec<String> = Vec::new();
        let mut doorbells: Vec<PeerId> = Vec::new();

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

            // ── pass A: 수신자별 수용 판정(파킹은 아직 안 한다 — 위 doc "두 패스") ────────────────
            for r in &addressing.recipients {
                // 2-a) 입구 반려 — 판정 근거는 **오직 이 스냅샷**이다(ADR-0111 결정 1 · ADR-0112 결정 2).
                //      미스폰·죽음·잠든 세션이 전부 같은 결말이고, wake 는 발동하지 않는다.
                let Some(target) = r.target.clone() else {
                    let (code, hint) = if let Some(folded) = &r.dup_of {
                        // M3: 같은 이름으로 접히는 다른 실체 — 자기 행을 만들 수 없다는 사실을 드러낸다.
                        (
                            FailCode::RecipientAmbiguous,
                            format!(
                                "'{}' resolves to a different agent that shares the name '{folded}' with another recipient of this send — the broker addresses mailboxes by name, so only one of them can be a recipient here. Send to it in a separate message.",
                                r.display
                            ),
                        )
                    } else if r.live_count == 0 {
                        (
                            FailCode::RecipientNotFound,
                            format!(
                                "No live agent named '{}' right now — nothing was queued for it (spawn it, or fix the name and send again). Live agents: {roster_names}.",
                                r.display
                            ),
                        )
                    } else {
                        (
                            FailCode::RecipientAmbiguous,
                            format!(
                                "'{}' matches {} live agents — send again using the exact agent id, or give them distinct names.",
                                r.display, r.live_count
                            ),
                        )
                    };
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
                };

                // 2-b) request 면 **이 수신자의 계약**을 먼저 연다(발송 기준 시계 — spec §3).
                //      계약을 못 열면 그 수신자에겐 **배달도 하지 않는다**(추적 없는 request 금지).
                let mut contract: PendingContract = None;
                if meta.request {
                    let reply_by = meta.reply_by.zip(meta.reply_by_raw.clone());
                    match st.ledger.open_request(
                        msg_id,
                        sender_name,
                        from.peer_id,
                        &r.key,
                        Some(target.id),
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
                        //   그래도 조용히 삼키지 않고 사실을 모아 락 밖에서 error 로 남긴다(배선 결함 신호).
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

                // 2-c) 지금 주입해도 되나 — 안 되면 파킹 예정(pass B), 보관함이 가득이면 그 수신자만 실패.
                //      판정 동사는 `has_pending_ahead`(= 큐 + 그 이름 앞 in-flight) 하나다: 진행 중인 flush
                //      배치를 직발송이 앞지르지 않게 한다(round-7 — 큐만 보면 그 배치가 안 보인다).
                let busy = self.busy.is_busy(target.id, target.epoch);
                let queued = st.mailbox.has_pending_ahead(&r.key);
                if !busy && !queued {
                    // ★계약은 아직 잠정이다★ — 확정은 주입/파킹 결말이 정해지는 자리에서(PendingContract).
                    plans.push(RecipientPlan::Deliver { target, contract });
                    continue;
                }
                if st.mailbox.can_admit(&r.key, ParkKind::Message) {
                    let hint = if busy {
                        park_hint_busy(&r.display)
                    } else {
                        park_hint_queued(&r.display)
                    };
                    plans.push(RecipientPlan::Park {
                        hint,
                        contract,
                        doorbell: target.id,
                    });
                    continue;
                }
                // ★보관함 가득 = 그 수신자만 실패(ADR-0114 결정 1 — 회수 시도 없음)★. 방금 연 계약은
                //   되돌린다(배달된 적 없는 요청이 기한 초과 notice 를 쏘는 유령 타임아웃 차단).
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

            // ── 봉투 `to` 동결(spec §1) — 수용 판정이 전원 끝난 **뒤** 1회 확정 ─────────────────────
            let admitted: Vec<bool> = plans
                .iter()
                .map(|p| !matches!(p, RecipientPlan::Failed { .. }))
                .collect();
            let admitted_count = admitted.iter().filter(|ok| **ok).count();
            // ★동결은 **주입보다 먼저**여야 한다(M7 — 판정 고정, 리뷰 blind r2 #3 ACCEPTED)★: 봉투는
            //   주입 시점에 조립되고 그 값은 여기서 굳는다. 그래서 **늦은 재확인 실패**(다른 수신자에게
            //   주입하는 동안 동시 발송이 이 수신자 큐를 채워 `MAILBOX_FULL` 로 끝나는 경우) 뒤에는, 이미
            //   나간 봉투가 **결국 실패한 수신자**를 `to` 에 적고 있을 수 있다. 그건 spec §1 과 **정합**이다:
            //   노출 기준이 "수용 판정" 이고 `delivered` 는 실제 주입 시점에야 확정되므로, 최종 결말을
            //   기준 삼으면 봉투를 만들 때 아직 모르는 값을 참조하는 순환이 된다.
            //   ★그래서 "결말 뒤에 wrap" 으로 고치지 말 것★ — 그건 즉시 배달의 `delivered` 회계를 통째로
            //   무너뜨린다(응답이 전부 pending 으로 뭉개진다). 이 비대칭은 **수용된 잔여**다.
            let fanout_meta = SendMeta {
                // 수용 판정된 수신자가 2인 이상일 때만 노출한다(혼자 받은 편지가 아님을 알리는 신호).
                to_attr: (admitted_count >= 2).then(|| build_to_attr(&addressing, &admitted)),
                ..meta.clone()
            };

            // ── pass B: 실제 파킹(동결된 `to` 를 payload 에 실어 flush 까지 나른다) ─────────────────
            let park_targets: Vec<usize> = plans
                .iter()
                .enumerate()
                .filter_map(|(i, p)| matches!(p, RecipientPlan::Park { .. }).then_some(i))
                .collect();
            for i in park_targets {
                let (doorbell, contract) = match &mut plans[i] {
                    RecipientPlan::Park {
                        doorbell, contract, ..
                    } => (*doorbell, contract.take()),
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
                        // 해석된 산 수신자라 id 힌트를 남긴다(동명 충돌 중에도 그 incarnation 으로 배달될
                        //   길 — mailbox `hinted_id` 주석).
                        hinted_id: Some(doorbell),
                        kind: ParkKind::Message,
                        meta: &fanout_meta,
                        expected_rows,
                    },
                    now,
                    &mut park_effects,
                ) {
                    Ok(()) => {
                        // 결말 확정(파킹 접수) — 이제 계약을 확정하고 희생자를 실제로 은퇴시킨다(A2).
                        if let Some(res) = contract {
                            res.commit(&mut st, &mut retirements);
                        }
                        doorbells.push(doorbell);
                    }
                    // ★도달 불가 경로★: 바로 위에서 `can_admit` 이 통과했고 그 사이 같은 락을 놓지 않았다.
                    //   그래도 조용히 삼키지 않는다 — 저장소 회계가 어긋났다는 뜻이므로 실패 행으로 강등한다.
                    Err(ParkError::MailboxFull) => {
                        // 실패로 끝났으니 예약을 되돌린다 — 희생자 표시도 함께 풀린다(A2).
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
            drop(st);

            // 락을 놓았으니 이제 로깅·도어벨·주입(락 밖 규율).
            park_effects.log("send");
            for name in &duplicate_contracts {
                tracing::error!(
                    msg_id = %msg_id,
                    recipient = %name,
                    "회신 계약 키 (msg_id, 수신자) 중복 — 수신자 중복 제거가 어긋났다는 배선 결함 신호(ADR-0111 결정 5)"
                );
            }

            // 3) 도어벨(★락 밖★) — 파킹된 수신자의 큐를 flush 레인이 열게 한다. busy 면 소비 측 게이트가
            //    스킵해 파킹이 유지되고(판정은 소비 측 한 곳 — flush_for 주석), idle 이면 순서대로 나간다.
            //    ★미배선 조립(실험 bin·단위 테스트)만 미룬다★: 그 갈래는 flush 를 이 스레드에서 동기
            //    실행하므로 주입 루프 한가운데서 부르면 ① flush 재진입 ② 응답(`pending`)과 장부(`delivered`)의
            //    시점 불일치가 난다. 응답을 다 만든 뒤로 미루면 두 사고가 구조적으로 불가능해진다.
            let mut deferred_inline: Vec<PeerId> = Vec::new();
            for id in &doorbells {
                self.ring_or_defer(*id, &mut deferred_inline);
            }

            // 4) 주입(★락 밖★) — 봉투는 **한 번만** 조립해 전 수신자가 같은 텍스트를 받는다.
            let wrapped = self.wrap_now(
                sender_name,
                msg_id,
                body,
                &fanout_meta.envelope_fields(msg_id),
            );
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
                    RecipientPlan::Park { hint, .. } => RecipientResult {
                        to: display,
                        status: SendStatus::Pending,
                        code: None,
                        hint: Some(hint),
                    },
                    RecipientPlan::Deliver { target, contract } => self.deliver_one(
                        DeliverOne {
                            msg_id,
                            sender_name,
                            from,
                            entrance,
                            body,
                            display: &display,
                            key: &key,
                            expected_rows,
                            meta: &fanout_meta,
                            wrapped: &wrapped,
                        },
                        &target,
                        contract,
                        &mut deferred_inline,
                        &mut retirements,
                    ),
                };
                results.push(result);
            }

            // ★은퇴 계측은 **결말 루프 뒤**에서 찍는다(M1)★: 커밋이 pass B·주입 루프 안으로 내려갔으므로
            //   (A2) 루프 **전**에 찍으면 주 경로(idle 수신자 request)의 은퇴가 **로그를 하나도 남기지 않는다**
            //   — ADR-0108 결정 2 에서 그 info 로그가 은퇴의 **유일한** 증거다. 락 밖 규율은 그대로.
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

            // 5) ★인라인 폴백 flush — 주입 루프가 끝나고 `results` 가 확정된 뒤★(위 3단계 주석의 분업).
            for id in deferred_inline {
                self.flush_for_agent(id);
            }

            // 6) ★회신 닫기(엄격 매칭 + 회신자 기준)★ — **수용된**(delivered|pending) 회신만 계약을 닫는다.
            //
            // ★A1 회귀(load-bearing · TODO(ratify) — 사용자 재가 대기)★: 옛 구현은 `results` 를 보지 않고
            //   무조건 닫았다. 개편으로 부재 수신자가 **실패 행**이 되면서 그 무조건이 거짓말이 됐다 —
            //   "요청자가 죽은 뒤 일꾼이 회신" 하면 메시지는 **아무에게도 가지 않았는데**(row = failed,
            //   RECIPIENT_NOT_FOUND) 장부는 `replied` 로 뒤집히고, 일꾼의 `reply_owed_by_me` 가 사라지며
            //   `due_timeouts` 는 그 계약을 영영 건너뛴다(기한 통지도 안 나간다). 개편 전에는 부재가 파킹돼
            //   "언젠가 도착한다" 가 성립했으므로 일관됐지만 이제는 아니다.
            // ★규칙★: 회신 발송은 수신자가 정확히 1명이므로(spec §3 항목 7-①) 그 한 행이 `failed` 면
            //   **계약은 열린 채로 남는다** — 장부가 도달하지 않은 메시지를 `replied` 라고 주장하지 않고,
            //   발신자의 기한 통지도 그대로 발화한다. 매칭 실패(모르는 id·내 계약 아님)는 그대로 정상 경로다.
            if let Some(in_reply_to) = &meta.reply_to {
                let admitted = results.iter().any(|r| r.status != SendStatus::Failed);
                if admitted {
                    self.close_reply_contract(in_reply_to, sender_name, from.peer_id);
                } else {
                    tracing::debug!(
                        in_reply_to,
                        "회신이 어느 수신자에게도 배달되지 않아(전 행 failed) 계약을 닫지 않음 — 장부는 도달하지 않은 메시지를 replied 로 주장하지 않는다(A1)"
                    );
                }
            }
            Ok(results)
        }
    }

    /// ★수신자 1명 주입(락 밖) + 실패 시 파킹★ — `handle_send` 4단계의 한 갈래를 함수로 뽑았다.
    ///
    /// ★주입 직전 재확인(load-bearing)★: pass A 의 `Deliver` 판정은 **한 락 구간**에서 전원을 한꺼번에
    ///   정했는데, 실제 주입은 수신자 수만큼의 **순차 blocking write** 다 — 앞 수신자에 쓰는 동안(파이프가
    ///   막히면 길게) 뒤 수신자의 세계가 바뀔 수 있다: ① 그 사이 새 턴을 시작했다 → 그대로 밀면 idle 게이트
    ///   우회 ② 그 사이 그 이름 앞으로 다른 발송이 파킹됐다 → 그대로 밀면 큐를 앞지른다. 그래서 같은 판정을
    ///   주입 **직전**에 한 번 더 태운다. 남는 창(이 확인과 inject 사이 마이크로초)은 inject 를 락 안으로
    ///   넣지 않는 한 구조적으로 닫히지 않는다(모듈 헤더 절대 규율).
    /// ★write 실패 = 파킹(spec §5 분기 3)★: 조용한 유실 금지 — 관측 레코드는 실패로 남기고 메시지는 큐에
    ///   되돌린다. ★self-heal 하지 않는다(의도적)★: 방금 그 incarnation 이 도달 불가해진 것이라 같은 로스터로
    ///   즉시 재-flush 하면 깨진 수신자에 재주입을 반복할 수 있다. 다음 **진짜** 등장(epoch bump)에 맡긴다.
    fn deliver_one(
        &self,
        ctx: DeliverOne<'_>,
        target: &LiveAgent,
        contract: PendingContract<'_>,
        deferred_inline: &mut Vec<PeerId>,
        retirements: &mut RetirementLog,
    ) -> RecipientResult {
        let mut effects = ParkSideEffects::default();
        // 4-a) 재확인(짧은 락) — 뒤집혔으면 그 자리에서 파킹까지 끝낸다.
        //      ★계약 확정/취소도 여기서★: 이 자리가 그 수신자의 **결말이 정해지는 지점**이다(A2).
        let mut contract = contract;
        let late = {
            let mut st = self.state.lock().expect("messaging state poisoned");
            let busy = self.busy.is_busy(target.id, target.epoch);
            let queued = st.mailbox.has_pending_ahead(ctx.key);
            if !busy && !queued {
                None
            } else {
                let hint = if busy {
                    park_hint_busy(ctx.display)
                } else {
                    park_hint_queued(ctx.display)
                };
                let outcome = self.park_or_fail(&mut st, &ctx, Some(target.id), hint, &mut effects);
                match (&outcome, contract.take()) {
                    (Ok(_), Some(res)) => res.commit(&mut st, retirements),
                    (Err(_), Some(res)) => res.rollback(&mut st),
                    _ => {}
                }
                Some(outcome)
            }
        };
        effects.log(ctx.key);
        if let Some(outcome) = late {
            return self.finish_park(outcome, ctx.display, target.id, deferred_inline);
        }

        // 4-b) 주입.
        match self.port.inject(target.id, ctx.wrapped.as_bytes()) {
            Ok(receipt) => {
                {
                    let mut st = self.state.lock().expect("messaging state poisoned");
                    // pending 없이 곧장 delivered(즉시 주입 — ADR-0104 "delivered = 실제 주입 시점").
                    st.ledger.record_with_expected(
                        ctx.msg_id,
                        ctx.sender_name,
                        ctx.key,
                        ctx.body,
                        DeliveryStatus::Delivered,
                        Instant::now(),
                        ctx.expected_rows,
                    );
                    // 결말 확정(배달) — 계약을 확정하고 희생자를 실제로 은퇴시킨다(A2).
                    if let Some(res) = contract.take() {
                        res.commit(&mut st, retirements);
                    }
                }
                // 관측 레코드(ADR-0088) — 락 밖에서 발행. in_reply_to 는 봉투 재파싱 없이 구조화 값 그대로.
                self.observe_success(
                    ctx.msg_id,
                    target,
                    ctx.from,
                    ctx.entrance,
                    ctx.wrapped,
                    &receipt,
                    ctx.meta.reply_to.clone(),
                );
                RecipientResult {
                    to: ctx.display.to_string(),
                    status: SendStatus::Delivered,
                    code: None,
                    hint: None,
                }
            }
            Err(e) => {
                self.observe_failure(
                    ctx.msg_id,
                    target,
                    ctx.from,
                    ctx.entrance,
                    ctx.wrapped,
                    &e,
                    ctx.meta.reply_to.clone(),
                );
                let mut effects = ParkSideEffects::default();
                let outcome = {
                    let mut st = self.state.lock().expect("messaging state poisoned");
                    let outcome = self.park_or_fail(
                        &mut st,
                        &ctx,
                        Some(target.id),
                        format!(
                            "Delivery to '{}' failed ({e}) — parked; retried on next appearance (expires after TTL).",
                            ctx.display
                        ),
                        &mut effects,
                    );
                    // 결말 확정(재파킹 접수) 또는 취소(보관함 가득) — A2.
                    match (&outcome, contract.take()) {
                        (Ok(_), Some(res)) => res.commit(&mut st, retirements),
                        (Err(_), Some(res)) => res.rollback(&mut st),
                        _ => {}
                    }
                    outcome
                };
                effects.log(ctx.key);
                // 도어벨을 누르지 않는다(위 doc "self-heal 하지 않는다") — 다음 등장 flush 가 집는다.
                match outcome {
                    Ok(hint) => RecipientResult {
                        to: ctx.display.to_string(),
                        status: SendStatus::Pending,
                        code: None,
                        hint: Some(hint),
                    },
                    Err(code) => RecipientResult {
                        to: ctx.display.to_string(),
                        status: SendStatus::Failed,
                        code: Some(code),
                        hint: Some(mailbox_full_hint(ctx.display)),
                    },
                }
            }
        }
    }

    /// 락을 **든 채** 파킹을 시도하고, 보관함이 가득이면 그 수신자를 실패 행으로 강등한다(계약 회수 포함).
    /// 반환 `Ok(hint)` = 파킹됨(장부 `pending` 기록 완료) · `Err(code)` = 실패 행(장부 종점 기록 완료).
    fn park_or_fail(
        &self,
        st: &mut MessagingState,
        ctx: &DeliverOne<'_>,
        hinted_id: Option<PeerId>,
        hint: String,
        effects: &mut ParkSideEffects,
    ) -> Result<String, FailCode> {
        let now = Instant::now();
        match park_into(
            st,
            ParkRequest {
                msg_id: ctx.msg_id,
                sender_name: ctx.sender_name,
                from: ctx.from,
                entrance: ctx.entrance,
                recipient: ctx.key,
                body: ctx.body,
                hinted_id,
                kind: ParkKind::Message,
                meta: ctx.meta,
                expected_rows: ctx.expected_rows,
            },
            now,
            effects,
        ) {
            Ok(()) => Ok(hint),
            Err(ParkError::MailboxFull) => {
                // 추적 없는 request 는 남기지 않는다 — 배달도 못 한 계약이 기한 통지를 쏘면 유령 타임아웃.
                // ★이중 결말은 관측한다(L3 — C3 fix 5 의 반환값을 버리지 않는다)★: 제거 시점에 그 계약이
                //   **이미 기한 초과 통지된** 상태였다면(예약↔실패 사이에 sweep 이 끼어든 희귀 레이스) 그
                //   통지는 회수할 수 없다(이미 발신자 큐에 있는 메시지다). 조용히 넘기지 않고 남긴다.
                //   ★락 보유 중이므로 사실만 모아 호출자가 락 밖에서 찍는다★(모듈 헤더 규율 — `effects`).
                if ctx.meta.request
                    && st.ledger.drop_request(ctx.msg_id, ctx.key)
                        == (DropOutcome::Removed { notified: true })
                {
                    effects.notified_drop = Some(ctx.msg_id.to_string());
                }
                st.ledger.record_with_expected(
                    ctx.msg_id,
                    ctx.sender_name,
                    ctx.key,
                    ctx.body,
                    DeliveryStatus::Failed,
                    now,
                    ctx.expected_rows,
                );
                Err(FailCode::MailboxFull)
            }
        }
    }

    /// `park_or_fail` 결과를 응답 행으로 옮기고, 파킹됐으면 도어벨을 누른다(락 밖).
    fn finish_park(
        &self,
        outcome: Result<String, FailCode>,
        display: &str,
        doorbell: PeerId,
        deferred_inline: &mut Vec<PeerId>,
    ) -> RecipientResult {
        match outcome {
            Ok(hint) => {
                self.ring_or_defer(doorbell, deferred_inline);
                RecipientResult {
                    to: display.to_string(),
                    status: SendStatus::Pending,
                    code: None,
                    hint: Some(hint),
                }
            }
            Err(code) => RecipientResult {
                to: display.to_string(),
                status: SendStatus::Failed,
                code: Some(code),
                hint: Some(mailbox_full_hint(display)),
            },
        }
    }

    /// ★회신 계약 닫기★ — 엄격 매칭 + **회신자 기준 계약 선택**(ADR-0111 결정 5). 결과에 따라 **로깅만**
    /// 갈린다(배달·응답에는 영향 없음 — spec §3 항목 7-②).
    ///
    /// - `Closed` = 정상(그 회신자의 첫 유효 회신). 다른 수신자의 계약은 그대로 열려 있다(전체회신 없음).
    /// - `ClosedHistoryAnomaly` = 계약은 닫혔으나 이력이 `Delivered → Replied` 간선을 못 탐(예: 회신이
    ///   원본보다 먼저 처리돼 원본이 아직 `pending`). 계약 정본은 추적이므로 그대로 두고 **관측**만 한다.
    /// - `NoMatch`/`AlreadyClosed` = 틀린 id·내 계약이 아님·이미 닫힘. **정상 경로**다 — 회신 메시지 자체는
    ///   이미 배달/파킹됐다. 발신자에게 새 에러를 만들지 않는다: 고칠 수 없는 상태이고, 반려하면 이미 배달된
    ///   메시지에 대해 재시도를 유발해 중복이 난다.
    /// ★락 규율★: 짧은 단독 락 구간에서 장부만 만지고, 로깅은 **락을 놓은 뒤** 한다.
    fn close_reply_contract(&self, in_reply_to: &str, replier_name: &str, replier_id: PeerId) {
        let outcome = {
            let mut st = self.state.lock().expect("messaging state poisoned");
            st.ledger
                .close_on_reply(in_reply_to, replier_name, replier_id, Instant::now())
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
                replier = %replier_name,
                "reply_to 가 이 회신자의 오픈된 request 를 가리키지 않음 — 메시지는 정상 배달, 닫힌 계약 없음(spec §3 항목 7-②)"
            ),
            ReplyOutcome::AlreadyClosed => tracing::debug!(
                in_reply_to,
                replier = %replier_name,
                "이미 닫힌 계약에 대한 추가 회신 — 메시지는 정상 배달, no-op(spec §3 항목 7-③)"
            ),
        }
    }

    /// ★도어벨 발화 시점 분업★ — 배선돼 있으면 **즉시** enqueue(논블록), 미배선이면 `deferred` 에 담아
    ///   호출자가 **응답 확정 후** 인라인 실행하게 한다.
    ///
    /// ★왜 갈라야 하나★: 두 갈래는 비용 구조가 정반대다. 배선 갈래는 채널 send 라 즉시 부르는 게 항상
    ///   옳고(미루면 그 수신자의 깨우기가 남은 blocking write 뒤로 밀리고, 패닉하면 통째로 유실된다),
    ///   미배선 갈래는 **이 스레드에서 배치 write 를 그대로 실행**하므로 주입 루프 한가운데서 부르면
    ///   flush 재진입 + 회계 skew(응답 `pending` vs 장부 `delivered`)를 만든다.
    /// ★락 밖 호출★: `FlushTrigger`/인라인 flush 모두 messaging 락 밖에서만 부른다(모듈 헤더 규율).
    fn ring_or_defer(&self, id: PeerId, deferred: &mut Vec<PeerId>) {
        match &self.trigger {
            Some(t) => t.request_flush(id),
            None => deferred.push(id),
        }
    }

    /// flush 도어벨을 누른다 — 배선돼 있으면 **다른 스레드**(flush lane)로 넘기고, 없으면 인라인 폴백.
    ///
    /// ★두 갈래의 근거는 `FlushTrigger` 주석★(운영 = 논블록 enqueue / 미배선 조립 = 인라인 flush).
    ///   인라인 폴백도 messaging 락 밖에서 불려야 한다 — 호출부는 모두 park 반환 후(락 해제) 지점이다.
    fn request_flush(&self, id: PeerId) {
        match &self.trigger {
            Some(t) => t.request_flush(id),
            None => self.flush_for_agent(id),
        }
    }

    /// ★`<notice>` 파킹 + ledger `pending` 기록★ — 데몬 통지 전용 래퍼(발송 경로는 `park_or_fail` 을 쓴다).
    ///
    /// ★조용한 유실 금지(ADR-0103)★: park 성공 시 반드시 ledger 에 `pending` 레코드를 남긴다 — 파킹된
    ///   메시지가 장부 밖에 있으면 조회·감사에서 사라진다.
    /// ★notice 는 절대 반려되지 않는다(mailbox `park`)★: `ParkKind::Notice` 는 **자기 레인**
    ///   (`mailbox::NOTICE_CAP` = 64)에서 회계되고 넘치면 가장 오래된 통지를 회수할 뿐이다. 그래서 이 함수는
    ///   반환값이 없고 호출부가 결과를 볼 필요도 없다(반려 = 조용한 유실이 될 자리였다).
    /// ★락 규율★: park+record 를 한 락 구간에서 하되(둘 다 순수 구조 조작, 외부 호출 없음) 그 구간에서
    ///   port 를 부르지 않는다. 회수 사실 로깅은 락을 놓은 뒤 한다.
    #[allow(clippy::too_many_arguments)]
    fn park_notice(&self, msg_id: &str, recipient: &str, body: &str, hinted_id: Option<PeerId>) {
        let now = Instant::now();
        // ★락 보유 중 tracing 금지(모듈 헤더 규율)★ — 회수 사실은 여기 모았다가 락을 놓고 찍는다.
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

    /// ★등장/epoch flush(ADR-0104 — C1)★: 수신자 이름의 파킹분을 **오래된 순 일괄** 주입한다. 데몬측
    ///   로스터 diff(flush observer)가 newly-live/epoch-bump 를 감지해 그 이름들로 부른다.
    ///
    /// ★순서 보장 범위(finding 8 · round-7 보정)★: "오래된 순" 은 **이 배치 내부** 보장이다(재파킹은 순번
    ///   merge 라 배치 간 이월 시에도 오래된 것 우선). 옛 주석은 여기에 "동시 직발송·다른 flush 와의 순서는
    ///   보장하지 않는다" 고 적었는데 **그건 너무 넓었다**: 지금은 ① 동시 직발송/방송이 이 배치를 앞지르지
    ///   못하고(합류 판정이 in-flight 를 본다 — `mailbox::has_pending_ahead`) ② 같은 수신자 flush 도 겹치지
    ///   않는다(아래 0단계). 남는 잔여는 **서로 다른 수신자** 간 전역 순서와 합류 판정↔inject 사이의
    ///   마이크로초 창뿐이다 — 모듈 헤더 "순서 보장의 범위"(accepted trade-off).
    ///
    /// 동작(0~4가 **한 락 구간**, 5만 락 밖 — 아래 "미배달분은 큐를 떠나지 않는다" 참조):
    ///   0. ★같은 수신자에 대한 flush 중복 진입 차단(round-7)★ — 그 이름 앞으로 **아직 정산되지 않은
    ///      in-flight** 가 있으면 = 다른 flush 가 락 밖에서 그 배치를 주입하는 중이라는 뜻이므로, 드레인 없이
    ///      물러난다(아래 "겹쳐 돌면 안 되는 이유").
    ///   1. 락 잡고 `mailbox.drain(recipient)` → deliverable(미만료, 오래된 순) + expired(만료).
    ///   2. expired → ledger `pending→expired`(장부 잔존 — 순수 조작).
    ///   3. deliverable 을 **해석된 타깃별로 분할**(항목별 **id 힌트 우선** → 이름 유일 도달 규칙).
    ///      ★결박 우선 분기는 폐지됐다(ADR-0111 결정 6)★ — 파킹분은 같은 이름의 새 화신에게도 배달된다.
    ///   4. 타깃별 **게이트 1회** → busy 타깃 몫·배달 경로 없는 몫은 **그 자리에서 원래 순서로 복원**.
    ///      락을 떠나는 몫만 `take_in_flight` 로 등록(cap 분모 유지 — 아래 F1 절). **락 해제.**
    ///   5. 배달할 몫만 각각 **개별 봉투**로 감싸 순서대로 inject(락 밖). 성공 → ledger `pending→delivered`
    ///      + 그 1건 in-flight 정산.
    ///      ★부분 실패 무손실(load-bearing)★: 배치 도중 inject 실패(drain 후 수신자 사망)면 **그 타깃의 남은
    ///      몫(실패분 포함)을 `restore_ordered` 로 되돌린다**(cap 우회 + admission 순번 merge = 무손실·순서
    ///      보존, 같은 락 구간에서 그만큼 정산). 다른 타깃 몫은 계속 배달한다(조용한 유실 금지).
    ///
    /// ★in-flight 회계 — cap 이 사이클마다 밀리던 구멍을 막는다(F1 · load-bearing)★: 4단계에서 락을 떠나는
    ///   배치는 큐에 없지만 **여전히 그 수신자 앞 미결 메시지**다. 이걸 분모에서 빼면 다음 인터리빙으로 큐가
    ///   무계로 자란다 — ① drain 이 큐를 비운다 ② 락 밖 inject 동안 동시 발송 k 건이 "빈 큐" 를 보고 cap 검사를
    ///   통과한다 ③ inject 실패로 배치 N 건이 복원된다 → 큐 = N + k, 다음 사이클 배치도 N + k 라 **매 사이클
    ///   +k**. 그래서 나가는 배치를 `take_in_flight` 로 분모에 남기고, 종점(배달/복원)마다 그만큼 정산한다.
    ///   정산 누락은 `FlightSettle`(Drop)이 덮는다. 관측 가능한 대가: **flush 가 도는 동안 그 수신자에게 온
    ///   신규 발송이 `MAILBOX_FULL` 로 반려될 수 있다**(큐는 비어 보여도 분모가 차 있다) — 조용한 성장 대신
    ///   가시적 반려를 택한 spec §5 기조와 같은 선택이다.
    ///   ★in-flight 는 이제 회계 값이 아니라 **순서 계약의 관측 수단**이기도 하다(round-7)★: "그 이름 앞으로
    ///   나가 있는 배치가 있나" 를 이 값 하나로 답할 수 있어졌고, 그 위에 두 판정이 얹혔다 — ① 직발송/방송의
    ///   FIFO 합류(`mailbox::has_pending_ahead`) ② 아래 0단계의 flush 중복 진입 차단.
    ///
    /// ★같은 수신자 flush 는 겹쳐 돌면 안 된다(round-7 · load-bearing)★: 배치 A 가 락 밖에서 주입 중일 때
    ///   배치 B 가 큐를 다시 드레인해 주입하면, A 의 **남은 항목보다 B 가 먼저** 수신자에게 닿는다 — 3-b 가
    ///   막는 순서 역전과 정확히 같은 사고가 한 층 아래에서 재발한다. 그래서 0단계에서 in-flight 를 보고
    ///   물러난다. 물러난 사실은 `deferred_flush` 에 남기고, **영수증을 쥔 쪽**이 정산을 마치며 도어벨을 다시
    ///   눌러 준다(그냥 물러나면 lost wakeup — 그 배치가 다음 idle 통지/등장까지 발이 묶인다).
    ///   ★되울림은 **물러난 id 전부**에 대해 한다(round-8 high)★: 한 이름 큐는 여러 id 로 열리므로(산
    ///   incarnation · 힌트 큐 · 죽어 가는 쪽의 늦은 통지) 유예 표식이 id 하나짜리 슬롯이면 나중 id 가 앞의
    ///   것을 덮는다. 덮인 쪽이 유일하게 배달 가능한 id 였으면 되울림이 **쓸모 없는 깨우기**가 되고(reap 된
    ///   id → `flush_for_agent` 조기 반환) 산 수신자 앞 메일이 TTL 까지 묶인다 — 종료성은 지켜지는데 유용성이
    ///   깨지는 실패 모드다. 그래서 표식은 중복 제거된 **집합**이고 6단계가 전부 누른다(`deferred_flush` 주석).
    ///   ★운영 배선에선 이 갈래가 **도달하지 않는다**(방어선 · 증명)★: 도어벨이 배선된 조립은 flush 를 전부
    ///   **단일 직렬 레인**(ws.rs `run_flush_lane` — FlushMsg 를 하나씩 꺼내 `spawn_blocking` 완료를 await)에서
    ///   실행하므로 두 flush 가 동시에 존재할 수 없다. 겹침이 실재하는 건 **도어벨 미배선 폴백**(실험 bin·
    ///   단위 테스트 — `request_flush` 가 호출 스레드에서 인라인 실행)과 앞으로 생길 다른 호출자다. 레인
    ///   직렬성은 ws.rs 쪽 성질이라 여기서 가정하지 않고, 이 파일 안에서 닫는다.
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
    pub fn flush_for(&self, recipient: &str, to_id: PeerId) {
        // ★execution-time 재해석(finding 2)★: enqueue 시점 (name,id) 는 stale 가능 — 지금 로스터로 재확인.
        //   port 호출이라 messaging 락 밖(모듈 헤더 규율). ★로스터 스냅샷은 1회만 뜬다★ — 이름 유일성
        //   판정과 아래 id-힌트 생존 판정이 **같은 스냅샷**을 봐야 배치 안에서 판정이 흔들리지 않는다.
        let roster = self.port.live_reachable_agents();
        let name_target = unique_reachable_in(&roster, recipient);

        let now = Instant::now();
        // ★락 밖에서 로깅할 사실(finding 3)★: 아래 락 구간에서 **수집만** 하고, 락을 놓은 뒤 찍는다.
        let mut no_target_kept = 0usize;
        let mut busy_skipped: Vec<(PeerId, u32, usize)> = Vec::new();
        // 만료/회수 전이가 링버퍼 evict 때문에 실패한 항목(C4 리뷰 fix J) — 락 밖에서 debug 로 남긴다.
        //   ★의도한 종점 상태를 함께 나른다(round-5 finding 2)★: 이 함에는 `expired`(TTL)와 `skipped`
        //   (notice 레인 은퇴) 두 어휘가 섞여 들어오므로, 상태를 안 실으면 로그가 전부 만료로 뭉개진다.
        let mut evicted_transitions: Vec<EvictedTransition> = Vec::new();
        // 1~4) 드레인 + 만료 장부화 + 타깃 분할 + 게이트 + **미배달분 즉시 복원** — 전부 **한 락 구간**.
        //
        // ★락 보유 중 tracing 금지 — 수집 후 락 밖 로깅(finding 3)★: 동기 포맷팅 subscriber 는 stdout 락에
        //   걸릴 수 있어, 크리티컬 섹션 안에서 찍으면 그 지연이 메시징 락 대기로 번진다.
        //
        // ★왜 게이트·복원을 락 안에서 하나(load-bearing — 관측 가능한 빈 큐 창 제거)★: 예전엔 drain(락) →
        //   락 해제 → 게이트 → 다시 락 → 복원 순서였다. 그 사이 큐는 **비어 보인다** — 그런데 배달되지도
        //   않을(busy 라 곧 되돌릴) 항목까지 사라진 것처럼 보이는 창이다. 그 창에서 직발송이 들어오면
        //   발송 경로(`handle_send`)의 FIFO 합류 검사(`has_pending_ahead`)가 "큐 비었음" 으로 보고 **즉시 주입**해,
        //   되돌려질 옛 메일을 앞지른다(수신자가 보는 순서 역전). 관측자(테스트·통계)도 파킹을 놓친다.
        //   그래서 "배달할 것만 큐에서 나가고, 안 나갈 것은 애초에 큐를 떠나지 않는다" 를 락으로 원자화한다.
        // ★락 안에서 게이트를 부르는 게 규율 위반이 아닌 이유★: `BusyGate` 는 **순수 조회 + 짧은 락**이며
        //   "messaging 락을 든 채 불려도 안전" 을 계약으로 못 박은 seam 이다(busy.rs `BusyGate` 주석). 역방향
        //   (busy 락 → messaging 락) 경로는 존재하지 않는다(tap 은 논블록 채널 send 만 한다) → 락 순서 역전 없음.
        //   금지 대상은 **DeliveryPort(inject/roster)** 다 — 그건 여전히 전부 락 밖이다(아래 5단계).
        // ★labeled block(`'drained`)인 이유★: 만료 전이의 evict 사실은 **락 밖**에서 찍어야 하는데(위 규율),
        //   배달할 게 없다고 여기서 `return` 해 버리면 그 로그가 통째로 사라진다. 그래서 블록을 값으로
        //   빠져나가고(빈 Vec) 로깅은 락 해제 뒤 공통 경로에서 한다(아래 루프들은 빈 입력에 no-op).
        /// 영수증 + 타깃별 배치. `None` = **유예**(0단계에서 물러남 — 큐도 영수증도 건드리지 않았다는 뜻이라
        ///   빈 배치(`Some`)와 구분해야 한다: 빈 배치는 "볼 것이 없었다", 유예는 "지금은 볼 차례가 아니다").
        type Drained = Option<(FlightTicket, Vec<(LiveAgent, Vec<ParkedMessage>)>)>;
        let drained_or_deferred: Drained = 'drained: {
            let mut st = self.state.lock().expect("messaging state poisoned");
            // 0) ★같은 수신자 flush 중복 진입 차단(위 doc "겹쳐 돌면 안 되는 이유")★ — in-flight 가 남아
            //    있다 = 다른 flush 가 이 이름 앞 배치를 락 밖에서 주입하는 중. 지금 드레인하면 그 배치의
            //    **남은 항목보다 우리가 먼저** 수신자에게 닿는다(순서 역전). 유예 사실만 남기고 물러난다 —
            //    판정도 기록도 **같은 락 구간**이라, 영수증 보유자의 정산 마무리(같은 락)와 경합하지 않는다:
            //    그쪽이 먼저면 여기서 in-flight 가 0이라 정상 진행하고, 여기가 먼저면 그쪽이 이 표식을 본다.
            //    ★덮어쓰지 않고 **추가**한다(round-8 high)★: 한 이름 큐를 여러 id 가 열 수 있어(산
            //    incarnation · 힌트 큐 · 죽어 가는 쪽의 늦은 통지) 단일 슬롯이면 나중 id 가 앞의 것을 덮는다.
            //    그 앞 id 가 유일하게 배달 가능한 쪽이었으면 깨우기가 **쓸모를 잃는다**(`deferred_flush` 주석의
            //    stale-id 시나리오). 같은 id 는 두 번 담지 않는다 — 되울림은 idempotent 지만 잉여 도어벨은 공짜가
            //    아니고, 유계 근거도 "서로 다른 id 수" 이기 때문이다.
            if st.mailbox.in_flight_len(recipient) > 0 {
                let waiters = st.deferred_flush.entry(recipient.to_string()).or_default();
                if !waiters.contains(&to_id) {
                    waiters.push(to_id);
                }
                break 'drained None;
            }
            let drained = st.mailbox.drain(recipient, now);
            for ex in &drained.expired {
                // pending → expired(장부 잔존). Illegal 은 best-effort — 무시. NotFound 는 **레코드가 링에서
                //   밀려난 것**이라 만료 사실이 어디에도 안 남는다 → 사실만 모아 락 밖에서 debug(fix J).
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
                    // 배달 경로 없음(이름이 부재/동명 다수 + 힌트도 사망) → 파킹 유지. 종점은 TTL 이 정한다.
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
                "flush 유예: 같은 수신자 앞 배치가 아직 주입 중(in-flight) — 드레인 없이 물러남, 정산 시 재-도어벨(round-7)"
            );
            return;
        };
        // ★단일 출구 정산 가드(F1)★ — 아래 배달 루프는 타깃별 early break 가 있고, 락 밖 외부 호출(inject)이
        //   섞여 있다. 정산을 각 갈래에 흩뿌리면 한 곳만 놓쳐도 그 수신자 레인의 분모가 영구히 부풀어
        //   메일이 영영 안 들어간다. 그래서 남은 영수증은 Drop 이 반납한다(정상 종료·언와인딩 공통).
        let mut flight = FlightSettle {
            svc: self,
            recipient,
            owed: flight,
        };

        // ★락 해제 후 로깅(finding 3)★ — 위에서 모은 사실만 찍는다(포맷팅·stdout 대기가 락 밖이다).
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
                // ★무조건 주입(ADR-0111 결정 6 — 결박 폐지)★: 주소 단위는 이름이고 재스폰 이어받기가
                //   **기능**이다(ADR-0101). 여기에 epoch 검사를 걸면 "재시작한 에이전트가 자기 앞 파킹을
                //   못 받는" 회귀가 된다(옛 조건부 주입 `inject_if_epoch` 은 결박 전용이라 함께 제거됐다).
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
                            // pending → delivered(실제 주입 시점, ADR-0104).
                            let _ = st.ledger.transition(
                                &parked.msg_id,
                                recipient,
                                DeliveryStatus::Delivered,
                                Instant::now(),
                            );
                            // ★의무 이전은 위 주입 **전** 지점으로 옮겼다(H3)★ — 아래 설명은 그 근거다.
                            // ★의무는 봉투를 **실제로 받은 자**를 따른다(round-2 리뷰 F2)★: 이름 큐 파킹은
                            //   재스폰 이어받기가 기능이라(ADR-0101) 발송 시점과 **다른** incarnation
                            //   에게 배달될 수 있다 — exact-id 로 건 request 가 busy 라 파킹됐다가 그 A 가
                            //   죽고 동명 B 가 떠 B 에게 꽂히는 경우가 그렇다. 계약의 recipient_id 를 여기서
                            //   실제 수신자로 고쳐야 B 의 미결 조회가 자기 의무를 본다(안 그러면 봉투를 받은
                            //   쪽이 "답할 게 없다" 고 읽는다). 여기가 착지 incarnation 을 아는 유일한 지점이다.
                            //   통보·notice 는 계약이 없어 no-op.
                            st.mailbox.settle_in_flight(recipient, settled);
                        }
                        // 등장 배달도 배달 경계 관측(ADR-0088) — 발송 경로와 동일하게 발행(락 밖).
                        //   원 발신자 신원·입구는 파킹 payload 에서 복원(파킹→flush 자동배달 acceptance, spec §7).
                        // ★to_name = park 키(recipient)★: 하네스가 "어느 이름 앞 파킹이 배달됐나" 로 회수하므로
                        //   해석된 타깃의 로스터 이름이 아니라 파킹 키를 싣는다(둘은 정상 경로에서 동일하다).
                        //   epoch 은 write 가 실제로 착지한 incarnation 값(outcome.epoch — 로스터 스냅샷이 아님).
                        let observed_target = LiveAgent {
                            id: to_id,
                            name: recipient.to_string(),
                            epoch: outcome.epoch,
                        };
                        // in_reply_to = 파킹 payload 가 실어 온 SendMeta.reply_to(구조화 출처, F1) — 늦은
                        //   배달도 즉시 배달과 같은 파라미터 스레딩 규율(봉투 재파싱 없음).
                        self.observe_success(
                            &parked.msg_id,
                            &observed_target,
                            payload.from,
                            payload.entrance,
                            &wrapped,
                            &outcome,
                            payload.meta.reply_to.clone(),
                        );
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
                        //   ★순서는 타깃 내부·타깃 간 모두 보존된다(round-4 finding 1)★: 복원이 admission 순번
                        //   merge 라 이 루프가 타깃마다 따로 되돌려도 큐는 전역 오래된 순을 유지한다 — 그룹 간
                        //   상대 순서도 무의미하지 않다: 큐 앞머리는 발송 경로의 FIFO 합류 판정과
                        //   다음 배치의 배달 순서를 결정하고, 나이 역전은 sweep 이 만료 항목을 지나치게 만든다.
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
                            // ★락 보유 중 tracing 금지(finding 3)★ — 복원만 하고 즉시 락을 놓은 뒤 로깅한다.
                            let mut st = self.state.lock().expect("messaging state poisoned");
                            st.mailbox.restore_ordered(recipient, remaining);
                            st.mailbox.settle_in_flight(recipient, settled);
                        }
                        tracing::warn!(
                            recipient,
                            agent = %to_id,
                            remaining = remaining_count,
                            "메시지 flush 중 inject 실패/거부 — 그 타깃의 남은 배치 재파킹(무손실 restore_ordered, ADR-0103/0104): {e}"
                        );
                        break;
                    }
                }
            }
        }

        // 6) ★유예된 flush 되울리기(round-7 · load-bearing — lost wakeup 금지)★: 우리가 락 밖에서 주입하는
        //    동안 이 이름 앞 flush 를 물린 호출자가 있으면(0단계), 그 깨우기를 여기서 되살린다. 안 하면 그
        //    배치는 다음 idle 통지/등장/TTL 까지 발이 묶인다 — 코드베이스가 lost wakeup 을 1급 결함으로 다루는
        //    이유는 C1 finding 3 주석들(`self_heal_if_live`·busy-park 도어벨) 참조.
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
        // ★"큐가 비지 않았으면 무조건" 이 아니라 **실제 유예가 있었을 때만**(무한 재기동 방지)★ — 배달 불가
        //   (busy·타깃 없음)로 파킹이 유지되는 정상 상태에서 큐 길이로 되울리면 flush 가 자기를 무한히 다시
        //   부른다. 집합은 유한하고 여기서 제거되므로, 유예한 id 마다 정확히 1회 되울리고 끝난다.
        // ★도어벨 id 는 **유예한 쪽이 쓰려던 값**을 그대로 쓴다★: 그쪽이 열려던 큐를 다시 열게 하는 게
        //   목적이라 우리 to_id 로 바꾸지 않는다(`flush_for_agent` 가 그 id 의 현재 이름 큐 + 힌트 큐를 연다).
        // ★전부 누른다 — 고르지 않는다(round-8 high)★: 어느 id 가 "쓸모 있는" 깨우기인지는 여기서 알 수
        //   없다(로스터를 다시 떠도 힌트 큐 쪽은 못 가린다 — `deferred_flush` 주석). 죽은 id 로 눌러도
        //   `flush_for_agent` 가 조용한 no-op 이라 잉여의 대가는 0에 수렴하는 반면, 하나라도 빠뜨리면
        //   산 수신자 앞 메일이 TTL 까지 묶인다 — 비대칭이라 전부 누르는 쪽이 맞다.
        for id in deferred {
            self.request_flush(id);
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
    /// ★만료 전이의 `NotFound` 는 조용히 삼키지 않는다(C4 리뷰 fix J)★: 방송은 한 발송이 레코드를 **N개**
    ///   쓰므로 이력 링(`ledger::HISTORY_CAPACITY`)이 훨씬 빨리 회전한다 — 파킹이 만료될 즈음 그 레코드가
    ///   이미 밀려났을 수 있고, 그러면 "만료됐다" 는 사실이 **어디에도** 남지 않는다. 전이는 여전히
    ///   best-effort(장부를 되살리진 않는다)지만, 그 사실을 debug 로 남겨 관측 가능하게 한다(락 밖 로깅).
    pub fn sweep(&self, now: Instant) {
        // ★F1 보증 계층 — **버려진** 계약 예약 회수★: RAII Drop 은 `try_lock` 이 실패하면 아무 것도 못
        //   하므로(`Reservation` 헤더), 락을 정상적으로 소유하는 이 지점이 같은 일을 다시 한다. 회수 사실은
        //   **락 밖에서** 찍는다(모듈 헤더 규율).
        // ★판정은 나이가 아니라 **소유자 생존**이다(R1)★: 나이 기준은 아직 주입 중인(= 무계로 블록될 수 있는)
        //   예약을 버려진 것으로 오판해 **계약 없는 request 배달**을 만들었다 — 근거 전문은
        //   `ledger::ReservationLiveness` 헤더. 임계값 상수를 되살리지 말 것.
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

        // 만료 전이가 evict 때문에 실패한 항목 — 락 안에서 모으고 락 밖에서 찍는다(모듈 헤더 규율).
        //   이 경로의 의도 상태는 항상 `Expired`(TTL) 다 — 그래도 상태를 값으로 실어 보낸다(finding 2).
        let mut evicted_transitions: Vec<EvictedTransition> = Vec::new();
        let due: Vec<DueTimeout> = {
            let mut st = self.state.lock().expect("messaging state poisoned");
            let expired = st.mailbox.sweep_expired(now);
            for ex in expired {
                // ★(msg_id, recipient) 로 정확히 지목(C4 — ADR-0104 앵커 해소)★: 장부는 1 msg_id : N 배달
                //   기록이라(그룹 방송) msg_id 만으로는 어느 멤버의 배달인지 특정할 수 없다. mailbox 가
                //   만료 항목마다 **자기 큐 키**를 함께 돌려주므로(`ExpiredParked`) 그 쌍으로 전이한다 —
                //   옛 "첫 pending 레코드 역조회" 헬퍼는 엉뚱한 멤버를 만료시킬 수 있어 제거됐다.
                //   Illegal 은 best-effort — 무시. NotFound(= 링에서 evict 됨)는 사실만 모아 락 밖 debug(fix J).
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
        // 락 밖 로깅(모듈 헤더 규율) — due 유무와 무관하게 먼저 찍는다.
        log_evicted_transitions(&evicted_transitions);
        if due.is_empty() {
            return;
        }
        // 락 밖 · 틱당 1회 스냅샷(위 주석). 아래 전 항목이 이 한 장으로 판정한다.
        let roster = self.port.live_reachable_agents();
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
    /// ★경로 1벌(ADR-0104 "flush = 일괄·오래된 순" 공유)★: 파킹으로 일원화하면 즉시/늦은 배달의 봉투
    ///   조립·관측·장부 전이가 전부 기존 flush 경로 하나를 탄다(두 벌 유지 금지). 봉투는 `ParkKind::Notice`
    ///   를 보고 `wrap_notice` 로 감싸진다.
    /// ★반려 없음(round-6 — notice 전용 레인)★: `ParkKind::Notice` 는 message 백로그와 무관한 자기 레인
    ///   (`mailbox::NOTICE_CAP`)에서 회계되고, 그 레인이 가득 차면 **가장 오래된 통지가 회수될 뿐** 신규는
    ///   반드시 수용된다(회신 계약 통지가 막히면 계약이 반쪽 — ADR-0103 불변식). 그래서 반려 갈래가 없고
    ///   반환값도 없다. 회수분은 조용히 사라지지 않는다 — 장부에 `skipped` 로 남는다(`park_into`).
    /// ★FIFO 정합★: 앞선 파킹이 있으면 notice 도 그 뒤에 붙는다 — 통지가 앞선 메일을 앞지르지 않는다.
    /// ★조용한 유실 금지★: park 와 함께 장부에 `pending` 을 남기고, 실제 주입 때 flush 가 `delivered` 로
    ///   전이한다(발신자 없음이라 장부 from 은 데몬 라벨, 관측 신원은 nil — 위 상수·헬퍼 주석).
    /// ★id 힌트 = **요청 발신자의 PeerId**(리뷰 fix 2 · load-bearing)★: 파킹 키는 발송 시점의 발신자
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
        // ★id 힌트 = 요청 발신자의 PeerId★ — 파킹 키는 발송 시점의 발신자 **이름**인데 그 사이 개명했으면
        //   이름-키 큐를 아무도 열지 않는다(통지는 `notified` 라 재발화도 없으니 영영 stranded). 장부가 함께
        //   들고 있던 발신자 id 를 힌트로 실어, flush 가 이름 유일성과 무관하게 그 incarnation 으로 배달하게 한다.
        self.park_notice(&notice_id, recipient, body, Some(due.sender_id));
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
    /// ★남는 창★: 검사와 실제 record(`park_notice`) 사이의 TOCTOU 창은 **발송 경로와 동일**하고, 전면
    ///   예약을 두지 않기로 한 근거는 `handle_send` 의 id 검사 지점 주석이 정본이다(중복 서술 금지).
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

    /// 봉투를 **현재** 전역 포맷으로 감싼다(단일 wrap point ADR-0096). 속성(`fields`)은 호출자가 만든다 —
    ///   즉시 배달은 `SendMeta`, flush 배달은 파킹 payload 에서 복원한 같은 메타로(두 경로 동일 봉투, C3).
    fn wrap_now(&self, sender: &str, msg_id: &str, body: &str, fields: &EnvelopeFields) -> String {
        let format: EnvelopeFormat = self.registry.envelope_format();
        wrap_message(sender, msg_id, body, format, fields)
    }

    /// 성공 주입 관측 레코드 발행(ADR-0088) — 락 밖. registry.record_delivery 가 observer 규율을 갖는다.
    ///
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

    /// 실패 주입 관측 레코드 발행(ADR-0088) — 실패를 성공으로 삼키지 않음의 증거. 이후 상위가 파킹한다.
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

    /// 관측/테스트용(round-6 I1) — 상한 판정이 보는 슬롯 점유 수(`Ledger::occupied_slots`).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn occupied_slots_for_test(&self) -> usize {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger.occupied_slots()
    }

    /// 테스트 전용(H2 픽스처) — 그 계약을 회신으로 닫아 슬롯 하나를 비운다.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn close_contract_for_test(&self, msg_id: &str, recipient: &str) {
        let mut st = self.state.lock().expect("messaging state poisoned");
        st.ledger.close_for_test(msg_id, recipient, Instant::now());
    }

    /// 관측/테스트용(H2) — 은퇴 예정 표시가 남은 계약 수(잊은 정산의 관측축 — `Reservation` 헤더).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn marked_retirements_for_test(&self) -> usize {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger.marked_retirement_count_for_test()
    }

    /// 관측/테스트용 — 추적 항목 총수(닫힘 포함). 무계 증식(잊은 정산의 두 번째 대가) 단언용.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn tracking_len_for_test(&self) -> usize {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger.tracking_len()
    }

    /// 관측/테스트용(C2) — 그 계약 키가 **추적 목록에 아직 있나**(닫힘·은퇴 표시 무관). 은퇴는 **표시**일
    ///   뿐이고 물리 제거는 커밋에서 일어나므로, 이 값이 잠정 창을 직접 관측하는 축이다(이력 링과 무관하다 —
    ///   `msg_id_in_use` 는 4096 링 때문에 창이 닫힌 뒤에도 true 라 그 창을 관측하지 못한다).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn contract_tracked_for_test(&self, msg_id: &str, recipient: &str) -> bool {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger.is_tracked_for_test(msg_id, recipient)
    }

    /// 관측/테스트용(H3) — 발급 측 충돌 검사가 이 id 를 "사용 중" 으로 보나(추적·이력·잠정 예약 합산).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn msg_id_in_use_for_test(&self, msg_id: &str) -> bool {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger.msg_id_in_use(msg_id)
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

    /// 관측/테스트용(F2) — 특정 msg_id 의 배달기록 **종점 키**(수신자 키) 목록, 오래된 순.
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

    /// 관측/테스트용(C4) — `Mailbox::can_admit` 의 예측값. 봉투 `to` 동결이 이 예측에 기대므로(park 전에
    ///   cap 결과를 알아야 한다) 예측과 실제 admission 이 갈리지 않는지 테스트가 직접 못 박는다.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn can_admit_for_test(&self, recipient: &str) -> bool {
        let st = self.state.lock().expect("messaging state poisoned");
        st.mailbox.can_admit(recipient, ParkKind::Message)
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

    /// 관측/테스트용 — 그 이름 앞으로 **아직 정산되지 않은 in-flight** 건수(정산 누수 단언 · round-7 의
    ///   FIFO 합류/flush 유예 판정이 보는 값과 동일한 출처).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn in_flight_len(&self, recipient: &str) -> usize {
        let st = self.state.lock().expect("messaging state poisoned");
        st.mailbox.in_flight_len(recipient)
    }

    /// 테스트 전용(C3 리뷰 fix 4) — 파킹 항목 하나의 payload 를 **의도적으로 손상**시킨다. 깨진 항목이
    ///   flush 배치를 중단시키지 않고 그 항목만 폴백 봉투로 열화되는지 실제 경로로 단언하는 seam.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn corrupt_parked_payload_for_test(&self, recipient: &str, idx: usize) {
        let mut st = self.state.lock().expect("messaging state poisoned");
        st.mailbox
            .corrupt_envelope_for_test(recipient, idx, "CORRUPT-PAYLOAD".to_string());
    }

    // ── 장부 조회 표면(D · spec §6 `messages { id? }`) ──────────────────────────────────────

    /// ★`messages { id }` — 그 메시지의 배달 장부(spec §6)★. 없으면 `None`(입구가 반려로 번역).
    ///
    /// ★1 msg_id : N 배달기록(spec §4)★: 그룹 방송이면 **멤버당 한 줄**이 나온다. 단일 발송은 한 줄.
    /// ★경과 초(`age_secs`)로 시각을 노출하는 이유★: 장부의 시각은 `Instant`(단조 시계)라 벽시계 값이
    ///   아니다 — 절대 시각으로 바꾸려면 장부에 `SystemTime` 축을 새로 들여야 하고, 그건 v1 인메모리 범위
    ///   밖이다(spec §5 "상태 전이 시각" 은 상대 비교용 데이터). 그래서 조회 순간(`now`) 기준 **경과 초**로
    ///   환산해 내보낸다 — 수신 LLM 에게도 "3분 전" 이 "17:42:03" 보다 바로 쓸모 있다.
    /// ★`now` 를 인자로 받는다★: 장부 순수성(주입 시계, ledger.rs 헤더)과 같은 규율 — 결정적 단위 테스트.
    /// ★완전성을 단언하지 않는다(D 리뷰 B2)★: 링(4096)이 밀리면 이 메시지의 앞쪽 행이 사라질 수 있으므로
    ///   `may_be_truncated` 를 함께 싣는다(판정 근거 = `Ledger::records_for_detailed` 주석).
    /// ★이력이 통째로 밀려난 **열린 계약**은 계약 뷰로 답한다(리뷰 NOTE — 교차 동사 모순 해소)★: B3 이후
    ///   미회신 계약은 이력보다 오래 산다. 그러면 `messages{}` 는 그 id 를 미결로 보여 주는데 `messages{id}`
    ///   는 `MESSAGE_NOT_FOUND` 를 내는 자기모순이 생긴다 — 조회자가 "목록이 거짓말한다" 로 읽는다. 그래서
    ///   행이 없어도 계약이 살아 있으면 **행 0줄 + `awaiting_reply=true` + 잘림 표시**로 답한다.
    pub fn message_state(&self, msg_id: &str, now: Instant) -> Option<MessageStateView> {
        let st = self.state.lock().expect("messaging state poisoned");
        let (records, may_be_truncated) = st.ledger.records_for_detailed(msg_id);
        // request 계약이 아직 열려 있나 — 열린 계약 목록에 이 id 가 있으면 회신 대기 중이다.
        let open = st
            .ledger
            .open_requests()
            .into_iter()
            .find(|r| r.request_id == msg_id);
        if records.is_empty() {
            // 이력은 없는데 계약은 살아 있는 경우만 계약 뷰로 답한다(그 외는 정말 모르는 id → None).
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
    /// ★왜 방향 태그가 필수인가★: 세 줄은 겉모습이 비슷한데 **해야 할 일이 정반대**다(②는 기다리는 것,
    ///   ③은 지금 답해야 하는 것). 태그가 없으면 수신 LLM 이 남의 숙제를 자기 것으로 오독한다.
    /// ★호출자 = (이름, PeerId) 둘 다 받는다(D 리뷰 B1)★. 축마다 매칭 규칙이 다르다:
    ///   - **회신 계약(②③)** = `matches_contract_party` — 계약이 id 를 들고 있으면 **id 로**, 없으면 이름으로.
    ///     동명 다수에서 exact-id 로 지목한 request 의 의무가 쌍둥이에게 잘못 붙는 걸 막는다.
    ///   - **미배달 통보(①)** = 이름 매칭(아래 주석의 문서화된 잔여) — 이력 레코드엔 id 축이 아예 없다.
    /// ★정렬 = 오래된 순★: 오래 묵은 것이 먼저 처리돼야 할 일이다(메일박스 flush 규칙과 같은 방향).
    // 리뷰 B1
    pub fn open_items_for(&self, me: &str, me_id: PeerId, now: Instant) -> Vec<OpenItemView> {
        let st = self.state.lock().expect("messaging state poisoned");
        let mut items: Vec<OpenItemView> = Vec::new();
        // ① 내가 보낸 미배달분. 그룹 방송이면 멤버별로 한 줄씩 나온다(1:N 회계 — 어느 멤버가 안 받았는지가
        //    발신자가 알아야 할 사실이다).
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
                continue; // 남의 계약 — 미결 목록은 호출자와 얽힌 것만 담는다.
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
        // 오래된 순(경과가 큰 것 먼저). 같은 경과면 방향·id 로 안정 정렬해 응답을 결정적으로 만든다.
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
/// ★id 가 없으면 이름 폴백(ADR-0101 WYSIWYA)★: 아직 뜨지 않은 이름 앞으로 건 request 는
///   나중에 그 이름으로 등장한 에이전트가 답할 주체다. ★단 개편(ADR-0111 결정 1) 이후 admitted 수신자의
///   계약은 **항상 id 를 든다** — 이 폴백이 실제로 쓰이는 자리는 "스폰 전 선지시" 가 아니라(그 기능은 폐지)
///   레거시/테스트 경로뿐이다.
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
        // ADR-0111: 수신자별 실패 종점 — 발송 응답 어휘(`failed`)와 **같은 문자열**이라 발신 LLM 이 두 응답을
        //   같은 규칙으로 읽는다(spec §6 대응표 "종점 그대로 기록").
        DeliveryStatus::Failed => "failed",
    }
}

/// `messages { id }` 조회 결과(입구가 JSON 으로 옮긴다 — shape 정본은 ingress `handle_messages` 주석).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageStateView {
    /// 조회한 논리 메시지 id.
    pub id: String,
    /// 발신자 이름(모든 배달기록이 공유).
    pub from: String,
    /// 이 메시지가 request 이고 아직 회신이 안 왔나.
    pub awaiting_reply: bool,
    /// ★행 목록이 불완전할 수 있나(D 리뷰 B2)★ — `true` = 이력 링에서 이 메시지의 앞쪽 행이 밀려났을
    /// 가능성이 있다(그러니 `rows` 를 전부로 읽지 말 것) · `false` = **확실히 전부**다. 판정 근거는
    /// `Ledger::records_for_detailed` 주석. 항상 싣는다 — 없으면 조회자가 침묵을 완전성으로 읽는다.
    pub may_be_truncated: bool,
    /// 수신자별 배달기록(그룹 방송이면 N 줄 — spec §4 1:N).
    pub rows: Vec<DeliveryRowView>,
}

/// 배달기록 한 줄(수신자 1명분).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryRowView {
    /// 이 줄의 수신자 이름.
    pub to: String,
    /// 상태 어휘(pending|delivered|replied|expired|skipped).
    pub status: &'static str,
    /// 발송(레코드 생성)으로부터 경과 초.
    pub age_secs: u64,
    /// 마지막 상태 전이로부터 경과 초.
    pub updated_secs_ago: u64,
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

/// 미결 항목 한 줄.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenItemView {
    /// 이 줄이 무엇인지(위 enum) — 없으면 LLM 이 남의 숙제를 자기 것으로 오독한다.
    pub direction: Direction,
    /// 논리 메시지 id(회신에 되쓸 값).
    pub id: String,
    /// 발신자 이름.
    pub from: String,
    /// 수신자 이름.
    pub to: String,
    /// 발송으로부터 경과 초.
    pub age_secs: u64,
    /// request 였다면 발신자가 쓴 기한 표기 원본(`"10m"`). 통보·기한 없는 request 는 None.
    pub reply_by: Option<String>,
    /// 기한 초과 통지가 이미 나갔나(계약은 여전히 열려 있다 — ledger `OpenRequestView` 주석).
    pub timed_out: bool,
}

/// 수신자 1명의 **판정 결과**(락 안에서 정하고 락 밖에서 집행한다).
///
/// ★왜 판정과 집행을 나누나(락 규율)★: 파킹·장부는 락 안에서 원자적으로 끝내야 하고(큐가 "비어 보이는 창"
///   금지 — flush_for 주석), 주입·도어벨은 절대 락 안에서 하면 안 된다(port 호출). 그래서 락 안에선 "이
///   수신자를 어떻게 할지" 만 정해 이 값으로 들고 나온다.
enum RecipientPlan<'a> {
    /// 지금 주입한다(락 밖). 해석된 산 수신자 + **아직 확정 전인 계약 예약**(아래 `contract` 주석).
    Deliver {
        target: LiveAgent,
        contract: PendingContract<'a>,
    },
    /// 파킹 예정 — pass B 가 실제 파킹을 끝내고, 락 밖에서 도어벨만 누른다.
    Park {
        hint: String,
        doorbell: PeerId,
        contract: PendingContract<'a>,
    },
    /// 이 수신자만 실패(장부 종점 기록 완료) — 락 밖에서 할 일 없음. 나머지 수신자는 그대로 간다.
    Failed { code: FailCode, hint: String },
}

/// ★계약 예약의 **단일 출구 RAII 가드**(H1 · ADR-0108 결정 3 의 보증 소유자)★ — 이 값이 살아 있는 동안
/// 계약은 **잠정**이고, 확정(`commit`)·취소(`rollback`) 중 하나를 **반드시** 거쳐야 소멸한다.
///
/// ★왜 커밋을 결말 뒤로 미루나(A2/A3 — 두 결함을 한 번에 닫는다)★:
///   - **A2(희생자 조기 소멸)**: 커밋은 표시된 희생자를 **물리 제거**한다. pass A 에서 바로 커밋하면 그
///     뒤 락 밖 경로(주입 직전 재확인 → 보관함 가득 / inject 실패 → 파킹 실패)가 이 수신자를 실패 행으로
///     떨굴 수 있는데, 그때 남의 미회신 계약은 **이미 사라진** 상태다 — 아무도 자리를 얻지 못했는데 남의
///     의무만 증발하는 ADR-0108 round-3 실패 모드다.
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
/// ★그 회수는 **이 가드의 생존**을 본다(R1 — load-bearing)★: 가드가 `ReservationLiveness`(강한 쪽)를 들고
///   장부 항목은 약한 쪽만 본다. 그래서 "아직 주입 중인" 예약은 sweep 이 볼 자격이 없다 — 옛 나이 기준이
///   만들었던 **회수 후 커밋**(= 계약 없는 request 배달) 경쟁이 구조적으로 사라졌다. 근거 전문은
///   `ledger::ReservationLiveness` 헤더. 이 필드를 지우거나 `mem::forget` 로 우회하면 그 보증이 무너진다.
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
// R1
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
    /// (`Ledger::attach_reservation_liveness`). 안 붙이면 sweep 이 이 예약을 버려진 것으로 읽는다.
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

    /// 결말 확정 — 잠정 표시를 지우고, 표시된 희생자를 **물리 제거**한다(그때 비로소 은퇴가 사건이 된다).
    /// ★호출자는 락을 쥐고 있어야 한다★(그래서 `st` 를 받는다 — Drop 의 `try_lock` 규율과 짝).
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

    /// 예약 취소 — 잠정 계약을 제거하고 희생자 표시를 해제한다(상한 교환 미성립).
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
        // 락 보유 중일 수 있으므로 **try_lock 만**(데드락 금지 — 위 헤더).
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
        // ★잊은 갈래 = 즉시 red(테스트·debug)★ — 이 가드의 존재 이유다(위 헤더).
        //   ★롤백 **뒤에** 터뜨린다★: 순서를 뒤집으면 패닉이 복구를 막아 "정산을 잊었을 때 상태가 온전한가" 를
        //   테스트가 관측할 수 없다(그리고 릴리즈에선 debug_assert 가 없어 복구만 남는다 — 같은 코드 경로).
        //   ★단 **언와인딩 중이면 건너뛴다**(F1)★ — 거기서 패닉하면 이중 패닉 → abort 다.
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

/// ★해석된 수신자 1명(spec §5 해석 순서 ①~⑤의 산출물)★.
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
    /// 그 이름을 지금 달고 있는 산 에이전트 수 — 실패 사유를 부재(0)와 동명 다수(2+)로 가르는 축.
    live_count: usize,
    /// ★자기 행을 가질 수 없는 중복 지목(M3 · TODO(ratify))★ — `Some(접힌 키)` 면 이 토큰은 **다른 실체**를
    ///   가리키는데 park/장부 키(= 이름)가 앞선 행과 같아 자기 자리를 만들 수 없다. 그 사실을
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
    /// 이름 토큰 — 봉투에는 **정규 이름**(해석되면 canonical, 아니면 표기 그대로)이 실린다.
    Name { key: String },
    /// `@`주소 토큰 — **펼치지 않고** 정규화된 토큰 그대로 실린다(spec §1). `keys` = 그 펼침이 낸 수신자 키.
    Group { label: String, keys: Vec<String> },
}

/// 발송 1건의 주소 해석 결과.
struct Addressing {
    /// **행 순서**대로의 수신자(중복 제거 완료) — spec §5 "행 순서 = 결정적".
    recipients: Vec<ResolvedRecipient>,
    /// 입력 토큰(입력 표기 순) — 봉투 `to` 값의 나열 순서를 정한다.
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
/// ★펼침에서 발신자 제외(spec §4)★: `@all` 은 "나 빼고 전원" 이다. **직접 지목 자기발송은 그대로 배달**
///   되므로(제외는 펼침에만 적용) `["@all", "<자기이름>"]` 은 자기에게 1행이 나간다.
// ADR-0111 (그룹 = 해석 매크로 · 다중 수신자 합류)
// ADR-0114 (@주소 오류 층위 · GROUP_EMPTY = 최종 집합)
fn resolve_addressing(
    groups: &dyn GroupSource,
    to: &[String],
    from: SenderIdentity,
    roster: &[LiveAgent],
) -> Result<Addressing, SendReject> {
    // `@` 펼침이 볼 live 명단 = 스냅샷 이름 verbatim − **발신자 자신의 incarnation**(id 기준).
    //   ★id 로 빼는 이유★: 이름은 겹칠 수 있어 동명 타인까지 함께 빠지면 안 된다(그 타인은 동명 규칙으로
    //   따로 판정된다 — ADR-0114 결정 4).
    let live_names: Vec<String> = roster
        .iter()
        .filter(|a| a.id != from.peer_id)
        .map(|a| a.name.clone())
        .collect();

    let mut tokens: Vec<AddressToken> = Vec::with_capacity(to.len());
    let mut recipients: Vec<ResolvedRecipient> = Vec::new();
    // 펼침 결과는 (토큰 인덱스, 이름)으로 모아 두었다가 **이름 사전순**으로 뒤에 붙인다(행 순서 규칙).
    let mut expanded: Vec<(usize, String)> = Vec::new();

    for (i, raw) in to.iter().enumerate() {
        let token = raw.trim();
        if token.starts_with('@') {
            let norm = normalize_group_name(token).map_err(|e| group_reject(e, token))?;
            let members = groups
                .resolve(&norm, &live_names)
                .map_err(|e| group_reject(e, token))?;
            for m in members {
                expanded.push((i, m));
            }
            tokens.push(AddressToken::Group {
                label: norm,
                keys: Vec::new(),
            });
        } else {
            let key = push_recipient(&mut recipients, token.to_string(), roster);
            tokens.push(AddressToken::Name { key });
        }
    }
    // 사전순(동률이면 토큰 순서) — 결정성 고정.
    expanded.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
    for (i, name) in expanded {
        let key = push_recipient(&mut recipients, name, roster);
        // 그 `@`토큰이 기여한 키로 기록한다 — **중복 제거로 흡수됐어도** 기록한다(A5: 흡수 ≠ 실패).
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
/// ★자기 행을 못 갖는 중복은 **보이게 실패시킨다**(M3 · TODO(ratify))★: 두 토큰이 **서로 다른 산 실체**를
///   exact-id 로 지목했는데 그 둘의 canonical 이름이 같으면(동명), 뒤 토큰은 park·장부 키를 앞 행과 공유할 수
///   없어 자기 자리를 만들 수 없다 — 옛 구현은 그 토큰을 **행 없이 조용히 삼켰다**(spec §6 "수신자 1명 = 1행"
///   위반이고, `RECIPIENT_AMBIGUOUS` hint 가 "exact id 로 다시 보내라" 고 가르치는 바로 그 시도가 무음으로
///   실패한다). 실체 키 병행 트랙은 비용 과다로 거부됐으므로(ADR-0114 거부 대안), **가시적 실패**로 강등한다.
/// ★단 하나의 예외: 해석된 쪽이 이긴다(A8 · TODO(ratify))★ — 두 토큰이 같은 키로 접힐 때 한쪽만
///   `target` 을 갖고 있으면(= exact-PeerId 지목은 동명 다수를 의도적으로 통과한다) **행 위치는 앞선 것을
///   유지하면서 해석 결과만 그쪽으로 갈아끼운다**. 왜: 그러지 않으면 `["dup", "<dup1의 id>"]` 가
///   `RECIPIENT_AMBIGUOUS` 로 끝나고, 그 행의 hint("exact agent id 로 다시 보내라")가 **같은 `to` 에 id 를
///   덧붙인 발신자에게는 거짓말**이 된다(순서를 뒤집으면 배달되는 비대칭도 함께 생긴다). 표기(`display`)는
///   먼저 쓴 것을 남긴다 — 응답의 `to` 는 발신자가 처음 쓴 표기라는 WYSIWYA 계약을 지킨다.
fn push_recipient(
    recipients: &mut Vec<ResolvedRecipient>,
    display: String,
    roster: &[LiveAgent],
) -> String {
    // ★① 같은 **raw 토큰**을 두 번 적었으면 행은 하나다(F2-a)★: 표기가 글자 그대로 같으면 정보가 0 인
    //   중복이다. 이 검사를 앞에 두지 않으면 반복된 exact-id 토큰이 아래 M3 갈래를 매번 새로 타서 **같은
    //   실패 행이 여러 줄** 나온다(`[A-id, B-id, B-id]` → B 행 2줄).
    if let Some(existing) = recipients.iter().find(|r| r.display == display) {
        return existing.key.clone();
    }
    let target = resolve_live(&display, roster);
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
        // TODO(ratify): 아래 두 규칙(해석 우선 · 중복 실체의 가시적 실패)은 리뷰 라운드의 **잠정 정책
        //   선택**이다(사용자 재가 대기 — A8/M3).
        let existing_target = recipients[pos].target.as_ref().map(|a| a.id);
        let new_target = target.as_ref().map(|a| a.id);
        match (existing_target, new_target) {
            // A8: 앞 행이 해석되지 않았고 이 토큰이 해석됐다 → 행 자리는 유지하고 해석만 갈아끼운다.
            (None, Some(_)) => {
                recipients[pos].live_count = roster.iter().filter(|a| a.name == key).count();
                recipients[pos].target = target;
            }
            // M3: 둘 다 해석됐는데 **다른 실체**다 → 뒤 토큰은 자기 행을 만들 수 없으니 보이게 실패시킨다.
            (Some(e), Some(n)) if e != n => {
                let lkey = loser_key(&display);
                recipients.push(ResolvedRecipient {
                    key: lkey.clone(),
                    display,
                    target: None,
                    live_count: 0,
                    dup_of: Some(key),
                });
                // 봉투 `to` 판정이 이 토큰을 "수용" 으로 세지 않게 **loser 키**를 돌려준다(이름 공간 밖이라
                //   다른 수신자의 키와 절대 겹치지 않는다 — `loser_key`).
                return lkey;
            }
            // 그 밖(같은 실체·둘 다 미해석·앞이 해석됨) = 표기 중복 → 흡수(먼저 나온 자리를 남긴다).
            _ => {}
        }
        return key;
    }
    let live_count = roster.iter().filter(|a| a.name == key).count();
    recipients.push(ResolvedRecipient {
        display,
        key: key.clone(),
        target,
        live_count,
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
    // U+0001(START OF HEADING) = canonical 이름에 절대 나타날 수 없는 제어문자.
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
    // 키 → 그 수신자가 수용 판정됐나(행 인덱스는 `recipients` 순서와 같다).
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

/// 자기교정 로스터(부재 실패 행의 hint) — 지금 살아 있는 이름들. 비면 `(none)`.
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

/// busy 파킹 hint(spec §5 분기 3) — 발신자가 "왜 아직 안 갔나" 를 스스로 읽게 한다.
fn park_hint_busy(display: &str) -> String {
    format!(
        "'{display}' is mid-turn — parked; it will be delivered as one batch when that turn ends."
    )
}

/// 선행 파킹분 뒤 FIFO 합류 hint.
fn park_hint_queued(display: &str) -> String {
    format!(
        "'{display}' has earlier queued messages — this one joins that queue and is delivered in order."
    )
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
    /// 실제로 물리 제거된 희생자 — ADR-0108 결정 2 가 말하는 "은퇴의 유일한 증거" 대상.
    real: Vec<RetiredContract>,
    /// 표시까지 붙였는데 커밋 시점에 그 항목이 없었다 — 은퇴가 아니라 **이상**이다(warn).
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
/// ★호출 위치는 **결말 루프 뒤**다(M1)★: 커밋이 pass B·주입 루프 안으로 내려갔으므로(A2) 루프 전에 찍으면
///   주 경로(idle 수신자 request)의 은퇴가 로그를 하나도 남기지 않는다 — ADR-0108 결정 2 에서 이 info 로그가
///   은퇴의 **유일한** 증거다.
/// ★M2 — 옛 "커밋을 락 구간에서 즉시 한다 / 되돌릴 사건 자체가 사라졌다" 논거는 **반증됐다**★: 결말은 락
///   밖 주입 뒤에 정해지고(재확인 실패·inject 실패 → 보관함 가득), 그 갈래가 실제로 롤백을 필요로 한다.
///   그래서 커밋 시점은 `Reservation` 헤더가 정본이고, 그 논거로 커밋을 pass A 로 되돌리면 A2/A3 회귀다.
fn log_contract_retirements(new_msg_id: &str, retired: &[RetiredContract]) {
    for r in retired {
        // ★F5 — 계측의 **관측면**★: 위 doc(M1/M2)이 말하는 대로 이 로그는 은퇴의 **유일한 증거**이고, 그
        //   증거를 좌우하는 건 코드 내용이 아니라 **호출 위치**다(루프 앞에서 찍으면 주 경로의 은퇴가 통째로
        //   기록되지 않는다 — 그리고 상태 단언만으로는 그 회귀가 안 잡힌다: 상태는 어느 위치에서 찍어도 같다).
        //   그래서 "실제로 보고됐나" 를 테스트가 볼 수 있게 thread-local 로 한 벌 흘린다.
        //   ★thread-local 인 이유★: 이 함수는 서비스 핸들이 없는 자유 함수이고(시그니처를 관측을 위해
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

/// `park_into` 인자 묶음 — 파킹 1건에 필요한 재료(인자 수가 많아 struct 로 묶는다).
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

/// 주입 1건에 필요한 재료(`deliver_one`/`park_or_fail` 공용 — 인자 수가 많아 struct 로 묶는다).
struct DeliverOne<'a> {
    msg_id: &'a str,
    sender_name: &'a str,
    from: SenderIdentity,
    entrance: Entrance,
    body: &'a str,
    /// 응답 행 표기(발신자가 쓴 토큰).
    display: &'a str,
    /// 장부·파킹 키(canonical 이름).
    key: &'a str,
    expected_rows: u16,
    /// **동결된** 발송 메타(봉투 `to` 포함) — 즉시 배달분과 파킹분이 같은 봉투를 쓰게 한다.
    meta: &'a SendMeta,
    /// 이미 조립된 봉투 텍스트(전 수신자 공용 — 한 발송 = 한 봉투).
    wrapped: &'a str,
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
    /// 전이하려던 종점 상태(`Expired` = TTL 만료 / `Skipped` = 수신자 소멸로 회수).
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
    /// 아직 반납하지 않은 몫. 갈래별 정산이 `split` 으로 떼어 가고, 남은 건 Drop 이 갚는다.
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
///   밖이어야 한다(모듈 헤더 규율). 그래서 `ring_or_defer(&mut deferred)` 와 같은 축적자 형태로 호출자가
///   들고 다니다가, 락을 놓은 뒤 `log` 를 부른다.
///
/// ★세 갈래를 **따로 센다**★: 회수 사유가 다르기 때문이다(message = 배달 불가 잔해 정리 / notice = 더 최신
///   통지에 밀림 / TTL 초과 = 시계가 먼저 운명을 정함). 한 필드로 합치면 로그가 사유를 뭉개 운영 중 오진을
///   부르고, 특히 TTL 갈래는 **장부 어휘 자체가 다르다**(`expired` — F3).
#[derive(Debug, Default)]
struct ParkSideEffects {
    /// notice 레인 상한으로 밀려난 옛 통지의 msg_id(같은 전이·같은 규율).
    retired_notices: Vec<String>,
    /// ★회수 시점에 이미 TTL 을 넘겨 있던 항목(F3)★ — 장부 어휘가 `expired` 라 위 둘과 분리한다(레인 무관).
    retired_expired: Vec<String>,
    /// 그 전이가 링 evict 로 실패한 항목(의도 상태 동반 — finding 2).
    evicted: Vec<EvictedTransition>,
    /// ★이미 기한 초과 통지된 계약을 실패 갈래에서 회수했다(L3 — C3 fix 5 의 이중 결말)★. 그 통지는 회수
    ///   불가라(이미 발신자에게 갔다) "통지도 갔고 그 수신자는 실패했다" 는 상태가 남는다 — 관측만 한다.
    notified_drop: Option<String>,
}

impl ParkSideEffects {
    /// 락 밖 로깅. 아무 일도 없었으면(대부분) no-op.
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
        if let Some(msg_id) = &self.notified_drop {
            tracing::warn!(
                recipient,
                msg_id = %msg_id,
                "실패 행으로 끝난 request 의 계약이 **이미 기한 초과 통지된** 상태였다 — 통지는 회수 불가(발신자에게 이미 감). 예약↔실패 사이 sweep 이 끼어든 희귀 레이스(C3 fix 5)"
            );
        }
        log_evicted_transitions(&self.evicted);
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
    // park 는 raw body + 봉투/관측에 필요한 최소 메타(sender 이름·발신자 신원·입구·회신 계약·그룹 라벨)를
    //   나른다(봉투는 flush 주입 시점 조립 — 단일 wrap point). ParkedMessage.envelope 계약이 "완성 봉투"라
    //   여기선 ParkPayload 로 인코딩해 그 문자열 슬롯에 실어 보관하고, flush 때 decode 한다.
    let payload = ParkPayload {
        sender_name: req.sender_name.to_string(),
        from: req.from,
        entrance: req.entrance,
        body: req.body.to_string(),
        meta: req.meta.clone(),
    };
    let parked = ParkedMessage {
        msg_id: req.msg_id.to_string(),
        envelope: payload.encode(),
        kind: req.kind,
        parked_at: now,
        // admission 순번은 `park` 이 수용 시점에 부여한다(저장소가 유일 부여자 — mailbox 주석). 여기 값은
        //   무시되므로 placeholder.
        admission_seq: 0,
        hinted_id: req.hinted_id,
    };
    let admitted = st.mailbox.park(req.recipient, parked)?;
    // 회수분 종점 — 기본 어휘는 `skipped`(배달할 수신자가 사라졌거나 더 최신 통지에 밀림)이고, **이미 TTL 을
    //   넘긴 항목만 `expired`**(위 doc "TTL 이 skipped 보다 우선" — F3). 레코드가 이미 링에서 밀려났으면
    //   사실만 모아 락 밖 debug. 로그 갈래는 어휘 우선, 그 다음 레인(`kind`)으로 가른다.
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
/// ★인코딩(v2 태그 — C3 리뷰 fix 4 에서 버전 헤더 도입, C4 에서 그룹 라벨 추가)★:
///   `<ver>\n<sender_len>\n<reply_by_len>\n<reply_to_len>\n<group_len>\n<from_agent_id>\n<from_epoch>\n`
///   `<entrance>\n<flags>\n<sender><reply_by><reply_to><group><body>`
///   앞 9줄은 개행 없는 필드(숫자/uuid/짧은 리터럴)라 개행으로 안전 분리하고, 가변 문자열 5개는 **길이
///   접두**로 경계를 잡는다(body·reply_to 에 개행이 들어와도 안전 — reply_to 는 에이전트 입력이라 임의
///   문자열일 수 있다). 길이 0 = `None`(빈 문자열 `Some("")` 은 입구 검증이 이미 반려하므로 모호하지 않다).
///
/// ★왜 버전 태그인가(fix 4)★: 이 형식은 **프로세스 내부**에서만 쓰이지만(파킹은 인메모리라 데몬 재시작이면
///   소멸), 형식이 바뀌는 순간 옛 payload 가 새 decode 를 만나면 조용히 오해석될 수 있다(길이 필드가 다른
///   자리로 밀려 body 가 잘리는 식). 1글자 태그가 있으면 **모르는 버전 = 즉시 폴백**으로 갈라져, 오해석
///   대신 "봉투 속성 잃고 body 만 남음"(보이는 열화)으로 실패한다. v2 영속화가 파킹을 디스크에 남기면
///   이 태그가 마이그레이션 분기점이 된다.
/// ★C4 에서 `1` → `2`★: 필드가 하나 늘었다(그룹 라벨). 같은 프로세스 안에서만 쓰는 형식이라 옛 payload 가
///   실재할 수는 없지만, 태그를 그대로 두면 **레이아웃이 바뀌었는데 버전은 같은** 상태가 되어 태그의
///   의미(= 이 문자열의 레이아웃 계약)가 무너진다 — "형식이 바뀌면 태그도 바뀐다" 는 규율을 지킨다.
const PARK_PAYLOAD_VERSION: &str = "3";

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
        // 버전 + 앞 8개 개행 필드 + 나머지(sender+reply_by+reply_to+group+body). splitn(10) 으로 body 안
        //   개행을 보존한다(마지막 조각은 자르지 않는다).
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
        // 길이 합이 남은 문자열을 넘으면 인코딩이 깨진 것 — 폴백(패닉 대신).
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
        // ★char 경계 검증(fix 4)★: 길이는 우리가 쓴 **바이트** 길이라 정상 payload 에선 항상 경계와 맞지만,
        //   payload 가 깨졌다면(외부 주입·형식 드리프트) 멀티바이트 문자 중간을 가리킬 수 있고 그때
        //   `split_at` 은 **패닉**한다. 세 절단점을 모두 미리 확인하고, 하나라도 어긋나면 폴백한다.
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
                // 파싱값은 복원하지 않는다(위 struct 주석 — flush 는 표기만 필요).
                reply_by: None,
                reply_to: (!reply_to.is_empty()).then(|| reply_to.to_string()),
                to_attr: (!group.is_empty()).then(|| group.to_string()),
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

/// `to`(이름 또는 PeerId 문자열) → 산·도달 가능 수신자(LiveAgent). 매치 규칙(ingress::resolve_recipient
///   미러 — F2): PeerId 문자열 정확 일치 우선 → 이름 정확 일치. 동명 다수·부재는 여기선 None 으로 접고
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
                roster: Arc::new(StdMutex::new(Vec::new())),
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
        fn set_roster(&self, roster: Vec<LiveAgent>) {
            *self.roster.lock().unwrap() = roster;
        }
        /// ★`inject_if_epoch` 이 보는 "지금" 로스터★ — armed(late-appearance) 가 있으면 그것, 없으면 기본.
        ///   `live_reachable_agents` 와 달리 호출 카운터·hook 을 건드리지 않는다(주입 경계의 판정이지
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
        ///   FIFO 합류·즉시 배달이 실제로 검증된다). `RECIPIENT_NOT_FOUND` 로 반려된 경우에만 `park_into` 로
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
            let absent = resolve_live(to, &self.port.live_reachable_agents()).is_none();
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
                        self.close_reply_contract(in_reply_to, sender_name, from.peer_id);
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
            },
        )
    }

    // ── C2 idle 게이트 테스트 하네스 ─────────────────────────────────────────────────
    /// 가짜 idle 게이트 — (id, epoch) 별 busy 를 테스트가 세팅하고, 호출 횟수를 세어 TOCTOU 시나리오
    ///   (첫 확인은 busy, 재확인은 idle)를 스크립트한다. 실 tap/PTY 없이 게이트 분기만 결정적으로 단언.
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
        // spec §5 분기 3(unreachable/write-fail) → 파킹. 해석은 성공하나 inject Err.
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
    fn idle_recipient_injects_immediately() {
        // 게이트가 있어도 idle 이면 C1 과 동일하게 즉시 주입 → delivered.
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
    fn idle_direct_send_joins_existing_queue_preserving_recipient_order() {
        // ★fix 5 회귀(FIFO 일관성)★: 큐에 m0·m1 이 대기 중인데 새 m2 가 직발송으로 앞지르면 수신자는
        //   (m2, m0, m1) 순서로 본다. 큐가 비어 있지 않으면 직발송도 큐에 합류해야 한다.
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
        assert_eq!(out, SendOutcome::Delivered, "큐가 비면 즉시 주입");
        assert_eq!(port.injected_bodies().len(), 1);
    }

    // ── round-7 high: FIFO 합류가 **in-flight 배치**를 못 보던 사각 ────────────────────────────────
    //    flush 는 큐를 통째로 비운 뒤 락을 놓고 주입한다. 그 구간의 큐는 비어 있어서, 옛 판정
    //    (`deliverable_len` = 큐만)은 "앞에 아무도 없다" 로 읽고 직발송을 즉시 주입했다 — 진행 중인 배치를
    //    통째로 앞지르는 순서 역전. 이 사각은 C2/C3 flush 설계 이래 있었고, round-7 의 in-flight 회계가
    //    비로소 관측 수단을 줬다. 아래 세 테스트가 세 주입 경로(직발송·중복 flush·방송)를 각각 덮는다.

    /// ★(a)★ 배치가 **주입 중**(큐는 비어 있음)일 때 들어온 직발송은 즉시 주입하지 않고 합류한다 —
    ///   그리고 그 배치 **뒤에** 배달된다(길이가 아니라 순서로 단언한다).
    #[test]
    fn a_direct_send_joins_a_batch_that_is_already_in_flight() {
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

        // 첫 항목(b0)을 수신자 stdin 에 쓰는 **그 순간** 새 직발송이 들어온다. 이 시점의 큐는 drain 으로
        //   비어 있고, 배치 2건은 in-flight 다 — 옛 구현이 "앞에 없음" 으로 오판하던 정확히 그 창.
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
            "주입 중인 배치가 있으면 직발송은 파킹(합류): {:?}",
            outcome.lock().unwrap()
        );
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="s">b0</message>"#.to_string(),
                r#"<message from="s">b1</message>"#.to_string(),
                r#"<message from="s">b2</message>"#.to_string(),
            ],
            "수신자가 보는 순서 = 도착 순서(새 발송이 진행 중인 배치를 앞지르지 않는다)"
        );
        assert_eq!(
            svc.parked_len("recv"),
            0,
            "합류분도 결국 배달된다(유예 아님)"
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
        //   이름으로 진입한다(tap 은 id 만 안다). 턴 중에 이름이 바뀌면 옛 이름 큐를 아무도 열지 않아 TTL 까지
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
    /// tap 부착을 하지 않는 no-op TapHost — 이 테스트는 `BusyTracker` 를 **게이트로만** 쓰고 표는 하네스
    ///   seam 으로 조작한다(실 PTY·claude 없이 "상한 sweep → 배달" 전 구간을 잇는다).
    struct NoSubscribeTapHost;
    impl super::super::busy::TapHost for NoSubscribeTapHost {
        fn subscribe_output(
            &self,
            _id: PeerId,
            _expect_epoch: u32,
            _probe: Arc<super::super::busy::TurnProbe>,
        ) -> Result<(), super::super::busy::SubscribeError> {
            Ok(())
        }
        fn current_epoch(&self, _id: PeerId) -> Option<u32> {
            None
        }
    }

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
            Arc::new(FakeControlPlane),
            tracker.clone(),
        ));
        let (id, agent) = live("recv");
        port.set_roster(vec![agent]);

        // 비정상 턴: busy 로 관측됐고 종료 통지가 오지 않는다(시각은 주입 — 실시간 30분 대기 회피).
        let t0 = Instant::now();
        tracker.mark_attached_for_test(id, 0);
        tracker.mark_busy_at_for_test(id, 0, t0);
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
        let (svc, port) = svc();
        let (boss_from, _boss) = live_sender("boss");
        port.set_roster(vec![]);
        // 은퇴 **가능** 계약으로 채운다 — 아래 발송이 희생자를 잠정 표시하는 창을 만들어야 하므로.
        fill_open_request_cap_evictable(&svc, boss_from);
        let cap = svc.occupied_slots_for_test();
        assert!(cap > 0, "상한까지 채워졌다");

        // 산 수신자를 세우고 inject 도중 hook 으로 **잠정 구간 안에서** id 발급을 시험한다.
        let (_t, target) = live("mid");
        port.set_roster(vec![target]);
        let probe: Arc<StdMutex<Option<(bool, String, usize)>>> = Arc::new(StdMutex::new(None));
        let probe_h = probe.clone();
        let svc_h = svc.clone();
        port.set_on_inject(Arc::new(move |_| {
            // 이 시점 = 예약(은퇴) 후 · 커밋 전. 희생자 id 를 뽑으면 재발급돼야 한다.
            let mut two = vec!["m-fresh".to_string(), "cap0".to_string()];
            let drawn = svc_h.draw_daemon_msg_id_with(|| two.pop().expect("draw"));
            // 잠정 창의 **직접 관측**: 표시된 희생자가 아직 추적 목록에 있고(물리 제거는 커밋에서),
            //   발급 검사도 그 id 를 사용 중으로 본다.
            *probe_h.lock().unwrap() = Some((
                svc_h.msg_id_in_use_for_test("cap0"),
                drawn.id.clone(),
                svc_h.open_request_count(),
            ));
        }));

        svc.park_absent_for_test(
            "live-req",
            boss_from,
            "boss",
            "mid",
            "해줘",
            Entrance::Mcp,
            &SendMeta {
                request: true,
                ..SendMeta::default()
            },
        )
        .expect("접수");

        let (in_use, drawn_id, open_in_window) =
            probe.lock().unwrap().clone().expect("hook 이 돌았다");
        // ★잠정 창의 **직접 관측**(C2)★: 은퇴는 표시일 뿐이라 그 창에서는 **희생자 + 새 계약이 동시에**
        //   추적에 있다(cap + 1). 커밋(결말 확정) 뒤에야 희생자가 물리 제거돼 cap 으로 돌아온다 — 이 두
        //   단언이 "예약 가시성" 자체를 보므로, 그 가시성을 없애면 여기서 터진다(옛 단언은 이력 링 때문에
        //   그대로 초록이었다).
        assert_eq!(
            open_in_window,
            cap + 1,
            "잠정 창: 표시된 희생자와 새 계약이 동시에 추적에 있어야"
        );
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
        let list: Vec<String> = to.iter().map(|t| t.to_string()).collect();
        svc.handle_send(
            msg_id,
            from,
            sender_name,
            &list,
            "body",
            Entrance::Mcp,
            &SendMeta::default(),
        )
    }

    #[test]
    fn an_absent_recipient_is_a_failed_row_that_never_parks_but_always_ledgers() {
        // ★spec §7 "부재·잠든 수신자 입구 반려"★: 응답 행 `failed`+`RECIPIENT_NOT_FOUND` · **파킹 큐에
        //   안 실림**(장부에 `pending` 레코드 없음) · 그래도 **종점 행은 남는다**(§5 "실패 수신자도 장부에").
        //   그래서 `messages{id}` 행수 = 발신 응답 행수라 `may_be_truncated` 오탐이 사라진다(§7 두 번째 항목).
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
    // /review code deep — fix round 1 회귀 그물(A1·A2·A3·A5·A6·A7·A8 · C2·C4)
    // ══════════════════════════════════════════════════════════════════════════════════════════

    #[test]
    fn a_reply_that_reached_nobody_leaves_the_contract_open_and_the_timeout_still_fires() {
        // ★A1 회귀(TODO(ratify) — 잠정 정책)★: 요청자가 죽은 뒤 일꾼이 회신하면 그 회신은 **아무에게도**
        //   가지 않는다(row = failed/RECIPIENT_NOT_FOUND). 그때 계약을 닫으면 장부가 도달하지 않은 메시지를
        //   `replied` 로 주장하고, 일꾼의 `reply_owed_by_me` 가 사라지며 기한 통지도 영영 안 나간다.
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

        // 요청자(boss)가 죽는다 — 로스터에서 사라진다.
        port.set_roster(vec![LiveAgent {
            id: w_id,
            name: "worker".to_string(),
            epoch: 0,
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
            "죽은 요청자에게는 회신이 가지 않는다"
        );
        assert_eq!(
            svc.open_request_count(),
            1,
            "배달되지 않은 회신은 계약을 닫지 않는다(A1) — 장부가 replied 로 거짓 주장하지 않는다"
        );
        // 일꾼의 미결에 의무가 그대로 남는다.
        let owed = svc.open_items_for("worker", w_id, Instant::now());
        assert!(
            owed.iter()
                .any(|i| i.direction == Direction::ReplyOwedByMe && i.id == "m-req"),
            "회신 의무가 유지돼야: {owed:?}"
        );
        // 기한 통지도 그대로 발화한다(due_timeouts 가 이 계약을 건너뛰지 않는다).
        svc.sweep(Instant::now() + Duration::from_secs(601));
        assert!(
            svc.ledger_snapshot()
                .iter()
                .any(|(_, from, _, _)| from == NOTICE_SENDER_LABEL),
            "기한 초과 통지가 나가야(계약이 살아 있으므로)"
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
        // ★재현★: 주입 도중(`on_inject`, 락 밖)에 ① 이력 링을 새 행으로 가득 채워 희생자의 행을 밀어내고
        //   ② 희생자 계약을 닫고 ③ 한 행 더 써서 evict→purge 를 발화시킨다. 그러면 커밋 시점에 희생자가 없다.
        let (svc, port) = svc();
        let (from, me) = live_sender("boss");
        let (_w, worker) = live("worker");
        port.set_roster(vec![me, worker]);
        fill_open_request_cap_evictable(&svc, from);
        // 이 발송이 표시할 희생자 = 가장 오래된 은퇴 가능 계약(픽스처의 첫 항목).
        let victim = svc
            .open_items_for("boss", from.peer_id, Instant::now())
            .into_iter()
            .find(|i| i.direction == Direction::AwaitingTheirReply)
            .expect("픽스처 계약");
        let _ = retirement_reports::drain();

        {
            let svc2 = svc.clone();
            let (vid, vto) = (victim.id.clone(), victim.to.clone());
            port.set_on_inject(Arc::new(move |idx| {
                if idx != 0 {
                    return;
                }
                let mut st = svc2.state.lock().expect("lock");
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
            }));
        }

        let out = svc
            .handle_send(
                "m-phantom",
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
            "전제: 주입 갈래: {out:?}"
        );
        assert!(
            !svc.contract_tracked_for_test(&victim.id, &victim.to),
            "전제: 희생자가 커밋 전에 사라졌다(purge) — 이 전제가 깨지면 아래 단언이 무의미하다"
        );
        // ★핵심★: 일어나지 않은 은퇴는 보고하지 않는다.
        let reports = retirement_reports::drain();
        assert!(
            reports.is_empty(),
            "커밋이 실제로 제거한 게 없으면 은퇴 보고도 없어야(유령 은퇴 금지): {reports:?}"
        );
        // 그래도 이 발송 자신의 계약은 정상이다(표시가 헛돌았을 뿐 접수는 성립).
        assert!(
            svc.contract_tracked_for_test("m-phantom", "worker"),
            "발송 자신의 계약은 살아 있어야"
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
        // ★A2 회귀 — **진짜 창을 태운다**(H2)★.
        //
        // ★라운드 1 판이 공허했던 이유(리뷰 prober 실측)★: busy + 보관함 가득 setup 은 **pass A 의
        //   MAILBOX_FULL 갈래**로 흘렀고 그 갈래는 라운드 1에서도 이미 롤백했다 — Park 갈래의 커밋을 판정
        //   pass 로 되돌려도, 심지어 pass A 에서 희생자 표시를 남겨도 전 스위트가 초록이었다. 게다가 단언축이
        //   `open_request_count()`(= `!closed`)라 **표시만 되고 안 풀린 희생자를 볼 수 없었다**.
        // ★이 판★: `deliver_one` 의 **늦은 갈래**(주입 직전 재확인 실패 / inject 실패 후 재파킹 실패)를 직접
        //   몰고, 단언은 ① 은퇴 표시가 하나도 남지 않았나 ② cap 분모가 그대로인가 ③ 그 수신자의 잠정 계약이
        //   사라졌나 로 한다(표시가 남으면 분모가 영구히 줄고 추적이 무계로 자란다).
        for late_branch in ["recheck", "inject"] {
            let (svc, port, gate) = svc_gated();
            let (from, me) = live_sender("boss");
            let (t_id, target) = live("worker");
            let (_l, lead) = live("lead");
            port.set_roster(vec![me, target, lead]);
            // 상한을 **은퇴 가능** 계약으로 채운다 — 그래야 아래 발송이 희생자를 표시한다.
            fill_open_request_cap_evictable(&svc, from);
            let occupied_before = svc.occupied_slots_for_test();
            let tracking_before = svc.tracking_len_for_test();
            assert_eq!(svc.marked_retirements_for_test(), 0, "전제: 남은 표시 없음");

            // 주입 도중(= Deliver 판정 뒤, 락 밖) 그 수신자 큐를 cap 까지 채운다 → 늦은 파킹이 실패한다.
            let (svc_h, from_h, gate_h) = (svc.clone(), from, gate.clone());
            let armed = Arc::new(StdMutex::new(true));
            let armed_h = armed.clone();
            let branch = late_branch;
            // ★두 갈래는 hook 을 **다른 주입에** 건다★:
            //   - `recheck`(4-a): **lead 주입(idx 0)** 중에 worker 큐를 채운다 → worker 의 주입 직전 재확인이
            //     busy/가득을 보고 늦은 파킹을 시도했다가 cap 으로 실패한다.
            //   - `inject`(4-b): **worker 주입(idx 1)** 중에 채운다 → 재확인은 통과했고(그 시점 큐는 비어
            //     있었다) inject 가 Err 를 낸 뒤의 **재파킹**이 cap 으로 실패한다. hook 은 실패 결정 **전에**
            //     돌기 때문에 이 순서가 성립한다.
            let arm_at = if late_branch == "inject" { 1 } else { 0 };
            port.set_on_inject(Arc::new(move |idx| {
                if idx != arm_at {
                    return;
                }
                if !std::mem::replace(&mut *armed_h.lock().unwrap(), false) {
                    return;
                }
                // busy 로 만들어 파킹으로 큐를 cap 까지 채운다(통보라 계약과 무관).
                gate_h.set_busy(t_id, 0);
                for i in 0..100 {
                    svc_h
                        .handle_send(
                            &format!("filler{i}"),
                            from_h,
                            "boss",
                            &["worker".to_string()],
                            "b",
                            Entrance::Mcp,
                            &SendMeta::default(),
                        )
                        .expect("파킹");
                }
                // busy 는 큐를 채우는 수단이었을 뿐이므로 걷는다(재파킹 자체는 cap 으로 막힌다).
                gate_h.clear();
                let _ = branch;
            }));
            if late_branch == "inject" {
                // lead 주입(idx 0)은 성공시키고 **worker 주입만** 실패시킨다 — 그 hook 이 큐를 채운 뒤라
                //   재파킹이 보관함 가득으로 끝난다(4-b 갈래).
                port.fail_at(&[1]);
            }

            // ★4-a(재확인) 갈래는 **앞 수신자 주입 중**에만 만들 수 있다★: 재확인은 inject **앞**에 있으므로
            //   자기 자신의 inject hook 으로는 못 만든다. 그래서 앞자리에 lead 수신자를 두고, 그 주입 hook 이
            //   worker 큐를 채운다 → worker 의 재확인이 busy+가득을 본다(4-b 갈래는 자기 inject 실패로 만든다).
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
                "늦은 갈래({late_branch})가 보관함 가득으로 끝나야: {out:?}"
            );
            assert_eq!(
                svc.marked_retirements_for_test(),
                0,
                "실패로 끝난 request 는 **아무도 은퇴시키지 않는다**({late_branch}) — 표시가 남으면 안 된다"
            );
            assert_eq!(
                svc.occupied_slots_for_test(),
                occupied_before,
                "cap 분모가 그대로여야({late_branch}) — 표시가 남으면 분모가 영구히 줄어든다"
            );
            assert_eq!(
                svc.tracking_len_for_test(),
                tracking_before,
                "잠정 계약도 남지 않아야({late_branch}) — 남으면 추적이 무계로 자란다"
            );
            assert!(!svc.contract_tracked_for_test("m-late", "worker"));
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
        // ★M3(TODO(ratify))★: park·장부 키가 이름이라 두 쌍둥이는 **한 자리**밖에 못 쓴다. 옛 구현은 뒤
        //   토큰을 **행 없이 삼켰다**(spec §6 "수신자 1명 = 1행" 위반 + AMBIGUOUS hint 가 가르친 시도가 무음
        //   실패). 이제 그 토큰은 `RECIPIENT_AMBIGUOUS` **실패 행**으로 보인다.
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
        // ★A8(TODO(ratify) — 잠정 정책)★: 두 토큰이 같은 키로 접힐 때 **해석된 쪽이 이긴다**. 그러지 않으면
        //   AMBIGUOUS 행의 hint("exact agent id 로 다시 보내라")가 같은 `to` 에 id 를 덧붙인 발신자에게
        //   거짓말이 되고, 토큰 순서만 뒤집으면 배달되는 비대칭이 생긴다.
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
            l.close_on_reply("m1", "alice2", b, now),
            ReplyOutcome::Closed
        );
        let open = l.open_requests();
        assert_eq!(open.len(), 1, "하나만 닫혔다: {open:?}");
        assert_eq!(open[0].recipient_id, Some(a), "남은 건 A 의 계약");

        // 이름은 같지만 id 가 다른 제3자는 아무 것도 닫지 못한다(id 매치 우선 → 폴백은 id 없는 계약만).
        let mut l2 = Ledger::new();
        l2.open_request("m2", "boss", PeerId::new_v4(), "alice", Some(a), None, now);
        assert_eq!(
            l2.close_on_reply("m2", "alice", PeerId::new_v4(), now),
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
}
