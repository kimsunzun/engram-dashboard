//! service — MessagingService: 순수 구조(mailbox·ledger·groups)를 tokio 위에서 발송 파이프라인에
//! 엮는 오케스트레이터(S18 메시징 v1 increment C1 · ADR-0103/0104).
//!
//! ★역할(C1+C2+C3+C4 스코프)★: 단일 수신자 발송의 3분기(spec §5)와 등장/idle flush(ADR-0104), TTL sweep,
//!   **idle 게이트**(C2), **회신 계약**(C3 — request 장부 오픈·회신 닫기·기한 초과 notice), **그룹 fan-out**
//!   (C4 — 발송 순간 스냅샷 방송·멤버별 회계)을 담당한다. 그룹 **관리**(생성·증감·삭제 툴/CLI)는 D 스코프다.
//!     ① resolve+inject 성공 → `delivered`(실제 주입 시점에만, ADR-0104)
//!        — 단 수신자가 **턴 진행 중(busy)** 이면 주입하지 않고 **파킹** = `pending`(C2 idle 게이트)
//!        — 또한 그 이름 앞에 **먼저 나갈 게 남아 있으면**(큐의 파킹 **또는 flush 가 지금 주입 중인 배치**)
//!          직발송이 그것을 앞지르지 않게 **함께 파킹**하고 flush 에 합류시킨다(FIFO 일관성 — 수신자가
//!          보는 순서 = 도착 순서). 판정 동사 = `mailbox::has_pending_ahead`(round-7).
//!     ② 부재(미스폰·죽음, "없는 이름" 포함) → **파킹** = `pending`(RECIPIENT_NOT_FOUND 소멸, spec §5)
//!     ③ 도달 불가/write 실패 → **파킹** = `pending`
//!     보관함 초과 → `MAILBOX_FULL` 반려(spec §5 분기 3).
//!   등장(스폰/epoch 교체)·**턴 종료(idle 전이)**·flush 시 파킹분을 **오래된 순 일괄** 주입(각 메시지
//!   개별 봉투, ADR-0104). 파킹 상태 어휘는 부재 파킹과 **공유**한다(`pending` — 새 상태 발명 금지, spec §5).
//!
//! ★idle 게이트 seam(C2 · ADR-0104 결정 3)★: "수신자가 턴 중인가" 는 `BusyGate`(busy.rs) 너머로
//!   묻는다 — 운영은 `BusyTracker`(출력 스트림 tap 이 턴 이벤트를 관측), 단위 테스트는 가짜 게이트를 끼운다.
//!   게이트를 안 꽂으면 `AlwaysIdleGate`(= 즉시 주입 = C1 동작)로 폴백한다(관측 불가 백엔드 폴백과 같은 값).
//!
//! ★순서 보장의 범위(finding 8 · round-7 보정 · load-bearing)★: "오래된 순" 은 **한 flush 배치 내부**
//!   (+ 재파킹 merge 로 배치 간 이월 시 오래된 것 우선)에서 보장한다 — spec §5 가 약속하는 건 **배치 순서**지
//!   전 출처를 아우르는 global total order 가 아니다.
//!   ★옛 주장 "진행 중인 flush 배치와 동시 직발송이 인터리브될 수 있고 그걸 의도적으로 수용한다" 는 **너무
//!   넓었다**(round-7 high)★: 그 문장 아래에서 실제로 벌어지던 건 "이미 나가 있는 배치를 새 발송이 통째로
//!   **앞지르는**" 순서 역전이었고, 이건 3-b 가 막으려던 바로 그 사고다(큐만 봐서 in-flight 를 못 봤다).
//!   지금은 발송·방송의 합류 판정이 in-flight 까지 보고(`mailbox::has_pending_ahead`), flush 는 같은 수신자에
//!   대해 겹쳐 돌지 않는다(`flush_for` 0단계) — 즉 **한 수신자가 보는 순서**는 지켜진다.
//!   남는 수용분(진짜 잔여)은 둘이다: ① 합류 판정(락 안)과 그 뒤 inject(락 밖) 사이 마이크로초 창 —
//!   그 사이에 새로 파킹된 메일이 먼저 나갈 수 있다 ② **서로 다른 수신자** 사이의 전역 순서. 둘 다 inject 를
//!   락 안으로 넣어야만 닫히므로(락 규율 정면 위반) 사람 대화 수준 메시지율에서 의도적으로 수용한다.
//!
//! ★단일 락(load-bearing — ADR-0006 정신)★: Mailbox+Ledger+Groups 를 **하나의 `Mutex<MessagingState>`**
//!   뒤에 둔다. 락 순서 위험이 없고(락 하나) 메시지율이 극히 낮아(사람 대화 수준) 경합이 무의미하다.
//!   ★절대 규율★: 이 락을 **든 채로 `DeliveryPort`(inject/roster)를 부르지 않는다** — 락 아래에서 결정할
//!   것(파킹/주입 대상 수집)을 먼저 끝내고 락을 놓은 뒤 DeliveryPort(외부 호출)를 부른다. 이걸 어기면
//!   inject 가 내부에서 다른 락(sessions RwLock 등)을 잡아 락 순서 역전·데드락 위험이 생긴다.
//!
//! ★delivery seam(ADR-0012 · 헤드리스 테스트)★: 호스트의 에이전트 실물을 직접 부르지 않고 `DeliveryPort`
//!   트레잇 너머로 부른다 — 운영 어댑터는 호스트 소유(데몬 `messaging_host::ManagerDeliveryPort`), 단위 테스트는
//!   `FakeDeliveryPort` 를 끼워 claude 바이너리·실 PTY 없이 3분기·flush·sweep 을 결정적으로 단언한다.
//!
//! ★봉투 = 주입 시점 조립(단일 wrap point, ADR-0096)★: 파킹은 **감싸지 않은 body + 발신자 이름 + 회신
//!   계약 메타**를 저장하고, 봉투는 **주입할 때** `wrap_message`/`wrap_notice`(이 crate `envelope.rs` 단일 wrap point)로
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
//!     아님). 배달은 `ParkKind::Notice` 파킹 + flush 도어벨로 일원화한다 — 통지는 **전용 레인**
//!     (`mailbox::NOTICE_CAP`)에서 회계돼 message 백로그에 막히지 않고(통지가 막히면 계약이 반쪽) 그
//!     자체로도 무계가 아니며, sweep task 에서 blocking write 를 하지 않기 위함이다(`deliver_notice` 주석).
//!     ★옛 주장 "notice 는 cap 예외이고 그 유계는 `ledger::MAX_OPEN_REQUESTS` 가 준다" 는 **거짓이었다**
//!     (round-6)★: `due_timeouts` 는 `notified` 를 즉시 세워 계약 자리를 비우므로, 앞선 통지가 아직 큐에
//!     파킹된 채로 다음 물결이 계약을 새로 열 수 있다 — 오픈 계약 수는 큐에 쌓인 통지 수를 묶지 못한다.
//!
//! ★그룹 fan-out(C4 · spec §4 · ADR-0103 결정 4 · ADR-0104 결정 1)★:
//!   - **로스터 스냅샷은 발송 순간 딱 한 장**(`handle_group_send` 첫 줄). 그 한 장으로 `@all` 명단 산출·
//!     멤버 해석·busy 판정을 전부 한다 — 방송 소급 금지(ADR-0103 불변식)의 물리적 근거다. 발송 뒤 등장한
//!     에이전트는 어떤 경로로도 이 메시지를 받지 못한다(그래서 **죽은/부재 멤버는 파킹하지 않는다**).
//!     ★이 주장은 2026-07-26 까지 **거짓이었다**(C4 리뷰 fix A)★: 파킹된 방송분이 flush 의 **이름 폴백**을
//!     타고 ① 같은 이름의 새 PeerId ② 같은 PeerId 의 다음 epoch 로 소급 배달됐다. 이제 그룹 파킹은
//!     `bound_incarnation`(발송 순간 `(PeerId, epoch)`)을 달고, flush 는 **정확히 그 incarnation** 일 때만
//!     배달한다(이름 폴백 없음·cross-epoch 없음) — 그래서 위 주장이 참이 됐다.
//!   - **멤버별 3결말**: idle+큐 빔 → 주입 `delivered` / busy·선행 큐 있음 → 파킹 `pending`(아래 해석) /
//!     부재·동명 다수·보관함 가득·**주입(write) 실패** → `skipped`(장부에 남기고 배달 안 함).
//!   - **장부 = 1 msg_id : N 배달기록**(spec §4). 그래서 만료/전이는 반드시 `(msg_id, recipient)` 로 지목한다
//!     (mailbox `ExpiredParked` 가 큐 키를 함께 실어 주는 이유 — 옛 msg_id 단독 역조회는 제거됐다).
//!   - **회신은 그룹으로 못 간다**: request 는 입구가 반려하고(spec §4), 그룹 주소로의 `reply_to` 도 입구가
//!     반려한다(전체회신 없음 — 회신은 항상 발신자 1인에게). 그래서 fan-out 은 항상 **통보**다.
//!
//! 워크스페이스 crate import 0(ADR-0110 — 컴파일러 강제).
// ADR-0103
// ADR-0104

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::busy::{AlwaysIdleGate, BusyGate};
use super::envelope::{
    new_msg_id, wrap_message, wrap_notice, DeliveryObservation, Entrance, EnvelopeFields,
    EnvelopeFormat,
};
use super::groups::{normalize_group_name, GroupError, GroupSource, Groups, ALL_GROUP};
use super::ledger::{
    DeliveryStatus, DropOutcome, DueTimeout, Ledger, OpenOutcome, ReplyOutcome, RetiredContract,
    TransitionError,
};
use super::mailbox::{FlightTicket, Mailbox, ParkError, ParkKind, ParkedMessage};
use crate::{PeerId, SenderIdentity};

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
    /// ★그룹 방송 라벨(C4)★ — 이 발송이 그룹 fan-out 의 한 갈래면 그 **정규화된** 그룹 이름(`@coders`).
    ///   봉투 `to` 속성으로 렌더돼 수신 LLM 에게 "이건 나 혼자에게 온 게 아니라 방송" 임을 알린다(spec §1
    ///   노출 원칙 — `to` 는 그룹일 때만).
    ///
    /// ★왜 `SendMeta` 에 얹나(load-bearing)★: 그룹 파킹분이 **늦게 배달될 때도 같은 봉투**여야 하기 때문이다.
    ///   봉투는 park 시점이 아니라 주입 시점에 조립되므로(모듈 헤더 "봉투 = 주입 시점 조립"), 속성 재료는
    ///   `ParkPayload` 에 실려 flush 까지 살아남아야 한다 — 계약 필드(request/reply_to)와 **정확히 같은 이유**라
    ///   같은 통로(SendMeta → ParkPayload)에 태운다. 별도 축을 만들면 flush 경로가 두 벌이 된다.
    /// ★단일 발송은 항상 `None`★: `handle_single_send` 는 이 값을 채우지 않는다(debug_assert 로 고정).
    pub group: Option<String>,
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
    /// ★가시성 `pub`(ADR-0110 이사로 승격)★: 호스트 입구(데몬 ingress)의 단위 테스트가 "검증된 인자 →
    ///   봉투" 를 한 줄로 단언한다(속성 조립 규칙의 단일 출처를 테스트가 우회해 재구현하지 않게). 커널이
    ///   별도 crate 가 되며 그 호출부가 crate 밖으로 나가 `pub(crate)` 로는 닿지 않는다.
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
            // ★C4★: 그룹 fan-out 만 `to` 를 싣는다(방송임을 알림 — spec §1). 단일 수신자 발송은 `group`
            //   이 `None` 이라 속성이 통째로 생략된다(= C1~C3 봉투와 byte-identical).
            to: self.group.clone(),
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

/// ★그룹 방송의 **멤버별** 결말(C4 · spec §4·§6)★ — 응답 `results[]` 원소 하나에 대응한다.
///
/// ★멤버당 정확히 하나(spec §6)★: 응답은 그룹 하나가 아니라 **멤버 N개**의 결과를 낸다 — 발신 LLM 이
///   "누가 받았고 누가 못 받았나" 를 보고 스스로 조정하게 하는 게 방송 회계의 목적이다(그룹 단위 요약은
///   그 정보를 뭉갠다).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupMemberResult {
    /// 멤버 **이름**(그룹 이름이 아니다 — spec §6 `results[].to`).
    pub to: String,
    /// 이 멤버의 결말.
    pub status: GroupMemberStatus,
    /// 자기교정 힌트(pending/skipped 사유). `delivered` 는 `None`.
    pub hint: Option<String>,
}

/// 그룹 멤버 1인의 배달 상태(spec §5·§6 어휘 — 새 어휘 발명 금지).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupMemberStatus {
    /// 실제 주입 완료(ADR-0104 — delivered = 실제 주입 시점).
    Delivered,
    /// 파킹됨 — 그 멤버가 턴 진행 중이거나 앞선 큐가 있어 배달 순서를 지켜야 하는 경우(아래 해석 참조).
    Pending,
    /// 배달하지 않음 — 부재/죽음(방송 소급 금지)·동명 다수·보관함 가득. 장부에 `skipped` 로 남는다.
    Skipped,
}

/// 그룹 발송 **전체** 반려(spec §4·§6 `{status:"error", code, hint}`). 멤버별 결말(`GroupMemberResult`)과
/// 는 다른 축이다 — 여긴 "이 그룹으로는 아무 것도 보낼 수 없다" 는 판정이라 배달·장부 부작용이 0 이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupReject {
    /// 등록 명단에 없는 그룹 이름 → `GROUP_NOT_FOUND`.
    NotFound { name: String },
    /// 그룹은 있으나 멤버 0명(또는 `@all` 인데 발신자 말고 산 수신자가 없음) → `GROUP_EMPTY`.
    Empty { name: String },
    /// `@` 네임스페이스 규약 위반(`@` 단독·`@@x`·`@a@b`). 상위가 `GROUP_NOT_FOUND` + 규약 hint 로 매핑한다
    ///   (spec §4 에 없는 새 에러 코드를 만들지 않는다 — "그런 그룹은 없다" 가 발신자에게 맞는 사실이다).
    InvalidName { name: String },
    /// 논리 메시지 id 가 장부에 이미 존재(`SendReject::IdCollision` 과 같은 사실·같은 부작용 0 보장).
    ///   호출자가 새 id 로 1회 재시도한다.
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
    ///   epoch 이 올랐어도 새 incarnation 에 착지한다. 그게 **옳은** 경로는 이름 주소 항목(단일 발송·notice)
    ///   뿐이다(재스폰 이어받기가 기능 — ADR-0101). 결박 항목(그룹 방송)은 `inject_if_epoch` 를 써야 한다.
    fn inject(&self, to_id: PeerId, bytes: &[u8]) -> Result<InjectReceipt, String>;

    /// ★incarnation 조건부 주입(round-3 fix 1 · 방송 소급 금지의 마지막 관문)★ — `expected_epoch` 가 **지금**
    /// 그 id 가 가리키는 incarnation 의 epoch 과 같을 때만 쓰고, 다르면 **한 바이트도 쓰지 않고** Err.
    ///
    /// ★왜 별도 동사인가(load-bearing)★: "누구에게 보낼지" 를 정하는 로스터 해석과 실제 write 는 **다른
    ///   시점**이다(그 사이 락도 놓고, 앞 멤버의 blocking write 로 길게 벌어질 수도 있다). 그 창에서
    ///   수신자가 재시작하면 무조건 주입은 **발송 순간에 존재하지도 않던 incarnation** 에 착지한다 —
    ///   ADR-0103 불변식("발송 뒤 등장한 에이전트는 이 방송을 받지 못한다") 위반이고, 결박(`bound_incarnation`)
    ///   으로 대상을 좁혀 놔도 write 자체가 무조건이면 결박은 종잇장이다. 검사를 write 와 같은 단위로
    ///   내려야 닫히는 창이라, 그 판정을 **주입 경계**에 둔다(데몬 어댑터는 이걸 core 의
    ///   epoch-조건부 write API 로 그대로 내려보낸다 — `messaging_host::ManagerDeliveryPort`).
    /// ★계약: 실패 = 부작용 0★ — Err 를 받은 호출자는 "안 보냈다" 로 확정하고 재파킹/skip 을 판정한다.
    fn inject_if_epoch(
        &self,
        to_id: PeerId,
        expected_epoch: u32,
        bytes: &[u8],
    ) -> Result<InjectReceipt, String>;

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
    /// 그룹 명단 레지스트리(C4 — 해석 seam `GroupSource`). 방송 fan-out 이 여기서 멤버를 편다.
    ///   ★관리(생성·증감·삭제) 표면은 D 스코프★ — 지금은 해석만 배선돼 있다(테스트는 직접 채운다).
    groups: Groups,
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
    /// mailbox+ledger+groups 단일 락(모듈 헤더 규율). 이 락을 든 채 port 호출 금지.
    state: Mutex<MessagingState>,
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
                groups: Groups::new(),
                deferred_flush: HashMap::new(),
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

    /// ★단일 수신자 발송(spec §5 3분기 — C1)★. handle_send 의 3-branch rewiring 이 검증·auth 통과 후 부른다.
    ///   - `msg_id`: 상위가 부여한 논리 메시지 id(ledger 상관·ACK 동봉 축).
    ///   - `from`: 발신자 신원(토큰 파생, ADR-0086). 관측 레코드·canonical 이름 조회에 쓴다.
    ///   - `sender_name`: 봉투 sender 표시 이름(상위가 canonical 로 이미 해석 — WYSIWYA ADR-0101).
    ///   - `to`: 수신자 지목(이름 또는 PeerId 문자열).
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
        from: SenderIdentity,
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
        // ★단일 발송은 그룹 라벨을 달지 않는다(C4)★: `to` 속성은 "이건 방송" 이라는 신호라(spec §1 노출
        //   원칙) 1:1 발송에 달리면 수신 LLM 이 자기 앞으로 온 메시지를 방송으로 오독한다. 그룹 갈래는
        //   `handle_group_send` 가 전담하므로 여기 오는 meta 엔 group 이 없어야 한다(배선 실수 방지).
        debug_assert!(
            meta.group.is_none(),
            "그룹 라벨은 handle_group_send 전용 — 단일 발송 봉투에 to 속성이 붙으면 안 된다(spec §1)"
        );

        // 1) 로스터 조회(락 밖 — port 호출) → 이름/PeerId 해석.
        let roster = self.port.live_reachable_agents();
        let resolved = resolve_live(to, &roster);

        // ★park 키 정규화(finding 4 · load-bearing)★: 등장 flush 는 canonical **NAME** 으로 keyed 이다
        //   (flush observer 가 로스터 diff 로 이름을 넘긴다). 그런데 `to` 가 exact PeerId 이고 그 에이전트가
        //   살아는 있으나 **비-도달**(TUI 등 non-structured)이면 resolve_live 는 None 을 내고, 여기서 UUID
        //   문자열 그대로 park 하면 park 키(UUID) ≠ flush 키(canonical name) 라 그 파킹은 **영영 flush 안 됨**.
        //   그래서 park 전에 `to` 를 PeerId 로 파싱해 그 에이전트가 존재하면 canonical_name 으로 park 키를
        //   바꾼다(그 이름이 도달 가능해지면 flush 가 잡게). 파싱 실패/미존재면 리터럴 문자열 그대로(이름 지목).
        // ★C3 에서 앞당김★: 계약 오픈이 "누구에게 건 요청인가"(recipient)를 3분기 **전에** 알아야 해서
        //   여기서 한 번 계산한다. 해석에 성공했으면 그 canonical 이름이 곧 장부/파킹 키다.
        let recipient_key = match &resolved {
            Some(t) => t.name.clone(),
            None => self.canonical_park_key(to),
        };
        // ★압력 회수 기준 = 이 park 키를 지금 달고 있는 산 incarnation 전부(F2)★ — 같은 로스터 스냅샷에서
        //   뽑는다. 동명 다수여서 `resolved` 가 None 인 경우에도 목록은 채워지는데, 그게 정확히 F2 가 지키는
        //   상태다(그 동명들 앞 결박 메일은 살아 있으므로 회수 대상이 아니다). 하나도 없으면 빈 목록 =
        //   "모른다" 로 저장소가 회수를 접는다(mailbox `is_stale_bound`).
        let live_here = live_incarnations_named(&roster, &recipient_key);

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
        // ★그룹 fan-out(C4)도 **같은 결정을 그대로 따른다**(리뷰 항목 K — 명시적 declined)★:
        //   `handle_group_send` 의 id 예약도 검사만 하고 선점하지 않는다. 방송은 한 msg_id 아래 레코드가
        //   N개라 앨리어싱 피해가 N배로 보일 수 있지만, ① 확률은 여전히 같은 2.8×10^12 공간의 마이크로초
        //   창이고 ② 레코드 키가 `(msg_id, to)` 라 **수신자가 다르면 서로를 덮지 않는다**(뭉개지는 건 우연히
        //   같은 수신자를 가진 레코드 하나뿐) ③ 예약 생명주기 배선이 단일 발송보다 더 복잡해진다(멤버별
        //   부분 성공의 회수 규칙이 추가로 필요). 단일 발송의 선례를 갈라 두 규칙을 만들 이유가 없어 거부한다.
        let now = Instant::now();
        let reservation = {
            let mut st = self.state.lock().expect("messaging state poisoned");
            if st.ledger.msg_id_in_use(msg_id) {
                Err(SendReject::IdCollision)
            } else if meta.request {
                // 기한은 (Duration, 표기 원본) 쌍으로 넘긴다 — 통지 문구가 발신자 표기를 그대로 쓰게(fix 6).
                let reply_by = meta.reply_by.zip(meta.reply_by_raw.clone());
                // ★해석된 수신자 id 를 계약에 함께 박는다(리뷰 B1)★: 동명 다수에서 exact-id 로 지목한
                //   request 의 회신 의무가 **쌍둥이에게 잘못 귀속**되는 걸 막는 축이다. 해석 실패(부재
                //   파킹)면 None — 그때는 나중에 그 이름으로 등장한 쪽이 답할 주체다(이름 폴백, WYSIWYA).
                match st.ledger.open_request(
                    msg_id,
                    sender_name,
                    from.peer_id,
                    &recipient_key,
                    resolved.as_ref().map(|t| t.id),
                    reply_by,
                    now,
                ) {
                    OpenOutcome::Opened => Ok(None),
                    // F1/round-5: cap 압력으로 가장 오래된 은퇴 가능 계약에 **은퇴 예정 표시**가 붙었다.
                    //   실제 제거는 커밋에서, 표시 해제는 롤백에서 — 사실을 들고 나가 락 밖에서 남긴다.
                    OpenOutcome::OpenedAfterMarking(r) => Ok(Some(r)),
                    OpenOutcome::DuplicateId => Err(SendReject::IdCollision),
                    OpenOutcome::Full => Err(SendReject::RequestCapacity),
                }
            } else {
                Ok(None)
            }
        };
        // ★락 보유 중 tracing 금지★ — 락을 놓은 뒤 찍는다(모듈 헤더 규율).
        // ★은퇴는 **잠정**이다(round-3 리뷰 G1)★: cap 압력으로 남의 계약이 자리를 내줬더라도, 이 발송이
        //   뒤이어 반려되면 그 교환은 성립하지 않는다 — 아래 결과 갈래에서 커밋(로그)하거나 되돌린다.
        //   그래서 여기서는 **아무 것도 찍지 않고** 값만 들고 나간다(먼저 찍으면 일어나지 않은 일을 보고).
        let pending_retirement = match reservation {
            Ok(retired) => retired,
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
                    "request 계약 오픈 실패 — 미회신 계약이 상한이고 **은퇴 가능한 계약이 하나도 없음**(전부 기한 대기 중 = 데몬이 진 통지 빚, F1)"
                );
                return Err(reject);
            }
        };

        // ★단일 출구 예약 가드(round-4 리뷰 H1)★ — `FlightSettle` 과 같은 규율이다. 아래 `dispatch_single`
        //   은 **락 밖 외부 호출**(DeliveryPort inject)을 포함하므로 패닉이 날 수 있고, 그 패닉은 상위
        //   `spawn_blocking` 이 삼켜 데몬을 살려 둔다(mcp_server 의 JoinError 갈래). 롤백을 결과 분기에만
        //   두면 그 언와인딩 경로에서 **희생자는 사라지고 잠정 계약은 남는** 최악의 잔해가 조용히 굳는다.
        //   그래서 되돌릴 의무를 Drop 에 싣는다 — 정상 반려·언와인딩 공통으로 정확히 한 번 실행된다.
        let mut reservation_guard = ReservationGuard {
            svc: self,
            new_msg_id: msg_id,
            retired: pending_retirement,
            // 잠정 계약은 request 발송일 때만 존재한다(통보/회신은 계약을 열지 않는다).
            provisional: meta.request.then_some(msg_id),
            committed: false,
        };

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
            &live_here,
        );

        // ★예약 확정/취소(round-3 G1 → round-4 H1: RAII 로 승격)★ — 발송이 접수됐을 때만 은퇴가 실제
        //   사건이 되고, 그 외 **모든** 이탈 경로(반려·패닉 언와인딩)는 가드의 Drop 이 원상 복구한다.
        if result.is_ok() {
            reservation_guard.commit();
        }
        drop(reservation_guard); // 반려 갈래의 롤백을 여기서 확정(가독성 — Drop 이 어차 부른다).

        if result.is_ok() {
            // 3) ★C3 회신 닫기(엄격 매칭)★ — 접수된 회신만 계약을 닫는다.
            if let Some(in_reply_to) = &meta.reply_to {
                self.close_reply_contract(in_reply_to);
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

    /// ★그룹 방송 fan-out(C4 · spec §4 · ADR-0103 결정 4)★ — 그룹 주소 하나를 **발송 순간 스냅샷**으로 펼쳐
    ///   멤버별로 개별 배달한다. 입구(handle_send)의 `@` 갈래가 부르는 유일한 진입점이다.
    ///
    ///   - `group`: 발신자가 지목한 그룹 주소(`@all`·`@coders`). 정규화는 여기서 한 번 한다(seam 함수).
    ///   - `meta`: 입구가 검증한 발송 메타 — **계약 필드가 비어 있음을 확인하는 용도**(아래 guard). 봉투용
    ///     메타는 이 함수가 그룹 라벨만 담아 새로 만든다(방송은 항상 통보 — 모듈 헤더 그룹 절).
    ///   - 반환: 멤버별 결과 N개(spec §6 `results[]`) 또는 그룹 전체 반려(`GroupReject`, 부작용 0).
    ///
    /// ★로스터 스냅샷은 딱 한 장(load-bearing — 방송 소급 금지, ADR-0103 불변식)★: 첫 줄에서 뜬 스냅샷
    ///   하나로 ① `@all` 명단 산출 ② 멤버 이름 해석 ③ busy 판정을 **전부** 한다. 도중에 다시 뜨면 한 방송
    ///   안에서 멤버마다 다른 세계를 보게 되고("발송 순간" 이 흐려진다), 발송 뒤 등장한 에이전트가 슬쩍
    ///   섞일 여지가 생긴다. 그 반대급부로 **부재 멤버는 파킹하지 않는다** — 파킹은 "이름이 나중에 등장하면
    ///   배달" 이라 정확히 소급 배달이기 때문이다(단일 발송 전용 기능, spec §5).
    /// ★**파킹된 멤버 몫도 소급하지 않는다**(리뷰 fix A · load-bearing)★: 부재 멤버를 파킹하지 않는 것만으론
    ///   부족하다 — busy/FIFO 로 **파킹된** 몫은 큐에 남는데, 그 큐는 이름-키이고 flush 는 힌트가 죽으면
    ///   이름으로 폴백하므로(모듈 헤더) 그 사이 같은 이름의 새 에이전트가 뜨거나 같은 에이전트가 재시작
    ///   (epoch bump)하면 **그 메시지가 소급 배달**된다(2026-07-26 실측 확인). 그래서 그룹 파킹은
    ///   `bound_incarnation = (target.id, target.epoch)` 을 달고 나가고, flush 는 그 쌍이 지금 로스터에
    ///   그대로 있을 때만 배달한다. 결박 대상이 사라지면 그 항목은 파킹된 채 TTL 로 `expired` 가 된다
    ///   (spec §5 "파킹의 운명" 어휘 그대로 — 새 상태 발명 금지).
    /// ★`@all` 명단 순서 = **정렬된 로스터**(리뷰 fix H)★: 운영 어댑터(`messaging_host::ManagerDeliveryPort`)가 로스터를 (이름, id)
    ///   오름차순으로 낸다 — 그래서 `@all` 의 주입 순서와 `results[]` 순서가 실행마다 같다(결정적). 등록
    ///   그룹은 등록 순서가 정본이다(레지스트리가 순서를 보존).
    ///
    /// ★`@all` 은 발신자를 제외한다(정책 해석 — 보고 대상)★: 스냅샷 이름을 verbatim 넘기되 **발신자 자신의
    ///   incarnation(id 기준)만 뺀다**. 근거: 자기 방송을 자기가 받는 건 정보가 0 인 잡음이고(발신 LLM 은
    ///   방금 자기가 쓴 글을 다시 읽는다), 턴 중이면 자기 큐에 파킹돼 다음 턴을 자기 메아리로 시작한다.
    ///   ★등록 그룹은 제외하지 않는다★ — 명단에 자기를 넣은 건 명시적 의사표시라 그대로 배달한다(`@all` 은
    ///   "관리 불요 내장" 이라 그 의사표시 자리가 없다는 게 차이다). id 로 빼는 이유: 이름은 겹칠 수 있어
    ///   동명 타인까지 함께 빠지면 안 된다(그 타인은 dup-name 규칙으로 따로 판정된다).
    ///
    /// ★멤버별 결말(spec §4·§5)★:
    ///   - **idle + 큐 빔** → 즉시 주입 `delivered`(실제 주입 시점 — ADR-0104).
    ///   - **busy(턴 중)** → 파킹 `pending`(정책 해석 — 보고 대상). "방송 소급 금지" 는 **발송 순간 살아있지
    ///     않던** 수신자에게 배달하지 말라는 규칙이고, busy 멤버는 그 순간 **살아 있다** — idle 게이트는
    ///     멤버십이 아니라 **배달 타이밍**의 문제다(ADR-0104 결정 3: "delivered = 실제 주입 시점"). 여기서
    ///     즉시 주입하면 idle 게이트를 방송만 우회하는 구멍이 되고, 반대로 skip 하면 산 멤버가 턴 중이라는
    ///     이유로 방송을 못 받는다(정보 유실). 그래서 파킹 = spec §5 분기 1 과 **같은 취급**이다.
    ///   - **앞선 파킹이 있음** → 파킹 `pending`(FIFO — 방송이 그 멤버의 옛 메일을 앞지르지 않는다).
    ///   - **부재/죽음** → `skipped`(파킹 없음 — 위 소급 금지).
    ///   - **동명 다수** → `skipped`(정책 해석 — 보고 대상). 이름이 유일하게 안 풀리면 방송이 누구를 고를
    ///     근거가 없다. 단일 발송의 `RECIPIENT_AMBIGUOUS` 반려와 같은 판정을 **멤버 단위**로 적용한 것이고
    ///     (`unique_reachable_in` 공유), 방송 전체를 반려하지 않는 이유는 나머지 멤버의 배달을 볼모로 잡지
    ///     않기 위해서다.
    ///   - **그 멤버의 보관함이 cap** → `skipped`(정책 해석 — 보고 대상). 단일 발송은 `MAILBOX_FULL` 반려지만
    ///     방송에서 그러면 **한 멤버의 가득 찬 메일함이 방송 전체를 막는다**. 방송은 멤버별 best-effort 회계라
    ///     그 멤버만 접고 나머지는 보낸다(발신자는 results 에서 그 사실을 본다 — 조용한 유실 아님).
    ///   - **주입(write) 실패** → `skipped`(리뷰 fix B — 단일 발송과 갈리는 지점). 단일 발송은 실패분을
    ///     파킹하지만(spec §5 분기 3 "unreachable → 파킹"), 방송에서 write 실패는 **바로 그 순간 그 멤버가
    ///     도달 불가**라는 뜻이라 spec §4 의 "발송 순간 살아있지 않으면 배달하지 않는다" 갈래와 같은 사실이다
    ///     — 여기서 파킹하면 그 멤버가 다음에 등장할 때 배달돼 **방송 소급 금지를 정면으로 위반**한다.
    ///     그래서 장부에 `skipped` 로 남기고 hint 에 write 오류를 실어 발신자가 재발송을 판단하게 한다.
    ///
    /// ★장부 = 1 msg_id : N 배달기록(spec §4)★: 멤버마다 레코드 하나(`(msg_id, member)` 키). 전이(delivered/
    ///   expired)는 반드시 그 쌍으로 지목한다 — 그래서 sweep 이 mailbox 로부터 큐 키를 함께 받는다.
    ///
    /// ★락 규율(모듈 헤더)★: 로스터 조회(port)는 락 밖 → **한 락 구간**에서 해석·판정·파킹·skip 장부화를
    ///   끝내고 → 락을 놓은 뒤 도어벨·주입(port)을 한다. 게이트(`BusyGate`)를 락 안에서 부르는 건 계약상
    ///   안전하다(flush_for 와 같은 근거). 판정과 파킹을 한 락에 묶는 이유도 flush_for 와 같다 — 그 사이에
    ///   큐가 "비어 보이는 창" 이 생기면 동시 직발송이 FIFO 를 앞지른다.
    ///
    /// ★주입 = 멤버 수만큼 순차 직접 write(수용된 비용 — 단, 호출 스레드는 반드시 blocking-safe 해야 한다)★:
    ///   각 inject 는 자식 stdin 의 blocking write 이고 상한은 멤버 수(사람이 만든 그룹 = 사람 규모)다.
    ///   ★"1:1 발송 N번과 같은 일" 이 **아니다**(리뷰 fix D — 옛 주석 수정)★: 1:1 발송 N번은 요청 N개가
    ///   각자 스레드를 쓰지만, 방송은 **요청 하나**가 N번의 blocking write 를 직렬로 진다 — 한 멤버의 막힌
    ///   파이프가 나머지 멤버의 배달과 그 워커 스레드를 통째로 head-of-line 블로킹한다. 그래서 입구(MCP 툴·
    ///   HTTP 라우트)가 `handle_send` 전체를 `spawn_blocking` 으로 감싼다(mcp_server.rs). 여기서 별도 워커로
    ///   넘기지 않는 이유는 그대로다: 그러면 `delivered` 를 응답에 실을 수 없어(주입이 아직 안 일어남)
    ///   spec §6 의 멤버별 회계가 전부 `pending` 으로 뭉개진다. 파킹분만 도어벨로 넘긴다.
    // ADR-0103 (결정 4 — 그룹 스냅샷 fan-out)
    // ADR-0104 (결정 1 — GroupSource seam)
    pub fn handle_group_send(
        &self,
        msg_id: &str,
        from: SenderIdentity,
        sender_name: &str,
        group: &str,
        body: &str,
        entrance: Entrance,
        meta: &SendMeta,
    ) -> Result<Vec<GroupMemberResult>, GroupReject> {
        // ★계약 필드 금지의 유일한 검증자는 ingress 다(리뷰 fix I — 단일 발송 guard 와 대칭)★: 그룹
        //   request 는 완료 판정 시맨틱 미정으로 v1 영구 금지고, 그룹 주소로의 회신은 reply-all 이라 금지다
        //   (spec §4). 이 함수는 pub 이라 다른 조립(스모크 bin·미래 입구)이 직접 부를 수 있는데, 계약 필드가
        //   실려 들어오면 방송이 N개의 계약을 열거나 남의 계약을 닫는 뒤엉킨 상태가 된다. 여기서 반려 문구를
        //   **복제하지 않는 이유**는 코드/문구가 입구마다 갈리면 안 되기 때문이고(entrance-agnostic), 대신
        //   디버그 빌드에서 배선 실수를 즉시 터뜨린다.
        debug_assert!(
            !meta.request && meta.reply_to.is_none(),
            "ingress가 유일 검증자 — 그룹 방송에 계약 필드(request/reply_to) 금지(spec §4)"
        );
        // 0) 이름 정규화 — **seam 함수**(groups::normalize_group_name)를 쓴다. 해석기가 쓰는 것과 같은
        //    함수라 봉투 `to` 라벨이 실제 해석 대상과 어긋나지 않고(라벨 단일 출처), v1 레지스트리 타입에
        //    파이프라인을 묶지 않는다(리뷰 fix F — 미래 소스가 다른 이름 문법을 가져도 이 지점은 그대로).
        let group_label = match normalize_group_name(group) {
            Ok(n) => n,
            Err(_) => {
                return Err(GroupReject::InvalidName {
                    name: group.to_string(),
                })
            }
        };

        // 1) ★로스터 스냅샷 1회(락 밖 — port 호출)★. 이 한 장이 이 방송의 "발송 순간" 이다.
        let roster = self.port.live_reachable_agents();
        // `@all` 이 볼 live 명단 = 스냅샷 이름 verbatim − 발신자 자신(위 doc "발신자 제외").
        let live_names: Vec<String> = roster
            .iter()
            .filter(|a| a.id != from.peer_id)
            .map(|a| a.name.clone())
            .collect();

        // 2) 해석 + id 예약 — 부작용 0 지점에서 한 락 구간(순수 조작). 예약 검사의 근거·남는 창은
        //    `handle_single_send` 의 예약 지점 주석이 정본이다(중복 서술 금지).
        let resolved = {
            let st = self.state.lock().expect("messaging state poisoned");
            if st.ledger.msg_id_in_use(msg_id) {
                Err(GroupReject::IdCollision)
            } else {
                st.groups
                    .resolve(&group_label, &live_names)
                    .map_err(|e| match e {
                        GroupError::NotFound { name } => GroupReject::NotFound { name },
                        GroupError::Empty { name } => GroupReject::Empty { name },
                        GroupError::InvalidName { name } => GroupReject::InvalidName { name },
                        // resolve 는 Builtin·InvalidMemberName 을 내지 않는다(CRUD 전용 에러) — 도달 불가
                        //   경로지만 조용히 삼키지 않고 발신자에게 맞는 사실("그런 그룹 없음")로 접는다.
                        GroupError::Builtin | GroupError::InvalidMemberName { .. } => {
                            GroupReject::NotFound {
                                name: group_label.clone(),
                            }
                        }
                    })
            }
        };
        // ★락 밖 로깅(모듈 헤더 규율)★.
        let members = match resolved {
            Ok(m) => dedup_members(m),
            Err(reject) => {
                tracing::debug!(
                    msg_id = %msg_id,
                    group = %group_label,
                    reject = ?reject,
                    "그룹 발송 반려 — 배달·장부 부작용 없음(spec §4)"
                );
                return Err(reject);
            }
        };

        // ★이 방송이 남길 배달기록 총수(round-2 리뷰 F3)★ — 멤버당 정확히 1행(dedup 후 확정). 계획 락과
        //   멤버별 주입 구간이 **같은 값**을 각 행에 박아, 조회가 `남은 행 < 기대` 로 잘림을 결정적으로
        //   판정한다(행이 링에서 연속이라는 위치 가정 없이).
        let fanout_rows = u16::try_from(members.len()).unwrap_or(u16::MAX);

        // 그룹 발송은 항상 **통보**다 — request 도 reply_to 도 입구가 이미 반려한다(모듈 헤더 그룹 절 +
        //   위 debug_assert). 봉투에는 `to="@그룹"` 만 붙는다(노출 원칙 — spec §1). 인자 `meta` 를 그대로
        //   쓰지 않고 **새로 만드는** 이유: 방송 봉투에 실릴 속성은 그룹 라벨 하나뿐이라는 걸 타입 수준이
        //   아니라 이 한 줄로 못 박는다(입구가 실수로 계약 필드를 실어도 봉투로 새지 않는다).
        let fanout_meta = SendMeta {
            group: Some(group_label.clone()),
            ..SendMeta::default()
        };

        // 3) ★멤버별 판정 + 파킹/skip 장부화 — 한 락 구간(주입은 락 밖)★.
        let now = Instant::now();
        let mut plans: Vec<(String, MemberPlan)> = Vec::with_capacity(members.len());
        // 멤버 파킹이 남긴 락 밖 로깅 거리(압력 회수 — round-6). 락 구간 뒤에 찍는다.
        let mut park_effects = ParkSideEffects::default();
        {
            let mut st = self.state.lock().expect("messaging state poisoned");
            for member in &members {
                let plan = match unique_reachable_in(&roster, member) {
                    // 부재/죽음 · 동명 다수 → skipped(파킹 없음 — 방송 소급 금지). 둘을 hint 로 가른다:
                    //   발신자가 할 수 있는 교정이 서로 다르다(스폰/대기 vs 재명명·1:1 재지목).
                    None => {
                        let live_count = roster.iter().filter(|a| &a.name == member).count();
                        let hint = if live_count == 0 {
                            format!(
                                "'{member}' was not live and reachable at send time — broadcasts are never parked for absent members (send it 1:1 if it should wait for them)."
                            )
                        } else {
                            format!(
                                "'{member}' matches {live_count} live agents — a broadcast cannot choose between them; give them distinct names, or send it 1:1 using the exact agent id."
                            )
                        };
                        st.ledger.record_with_expected(
                            msg_id,
                            sender_name,
                            member,
                            body,
                            DeliveryStatus::Skipped,
                            now,
                            fanout_rows,
                        );
                        MemberPlan::Skipped { hint }
                    }
                    // 산 멤버 — "지금 주입해도 되나" 판정(안 되면 그 자리에서 파킹까지). 판정 규칙은
                    //   `plan_live_member` 한 곳에만 있다(주입 직전 재확인과 **같은 함수** — fix C 참조).
                    Some(target) => {
                        // ★회수 기준은 **이 이름의 산 incarnation 전부**(F2)★ — 지금 이 멤버는 유일 해석이라
                        //   보통 1개지만, 목록으로 넘겨야 이 큐에 섞인 **다른 동명 앞 산 메일**이 잔해로
                        //   몰리지 않는다(그 이름이 다시 유일해지기 전에 쌓인 몫이 있을 수 있다).
                        let live_here = live_incarnations_named(&roster, member);
                        self.plan_live_member(
                            &mut st,
                            ParkRequest {
                                msg_id,
                                sender_name,
                                from,
                                entrance,
                                recipient: member,
                                body,
                                // 해석된 산 멤버라 id 힌트를 남긴다(동명 충돌 중에도 그 incarnation 으로
                                //   배달될 길 — mailbox `hinted_id` 주석).
                                hinted_id: Some(target.id),
                                // ★방송 소급 금지의 물리적 강제(fix A)★ — 이 파킹은 **발송 순간의 그
                                //   incarnation** 에게만 배달된다(mailbox `bound_incarnation` 주석).
                                bound_incarnation: Some((target.id, target.epoch)),
                                live_incarnations: &live_here,
                                kind: ParkKind::Message,
                                meta: &fanout_meta,
                                // F3: 이 방송이 남길 행 수 = 멤버 수(두 기록 단계가 같은 값을 쓴다).
                                expected_rows: fanout_rows,
                            },
                            &target,
                            now,
                            &mut park_effects,
                        )
                    }
                };
                plans.push((member.clone(), plan));
            }
        }
        // ★락 밖 로깅(모듈 헤더 규율)★ — fan-out 전체의 회수 사실을 한 번에 찍는다. 멤버마다 큐가 달라
        //   대표 키로 그룹 라벨을 싣는다(항목별 recipient 는 evict 로그가 따로 들고 있다).
        park_effects.log(&group_label);

        // 4) 도어벨(★락 밖★) — 파킹된 멤버의 큐를 flush 레인이 열게 한다. busy 면 소비 측 게이트가 스킵해
        //    파킹이 유지되고(판정은 소비 측 한 곳 — flush_for 주석), idle 이면 그 배치가 순서대로 나간다.
        //
        // ★두 갈래의 시점이 다르다(round-3 fix 3/5 · load-bearing)★:
        //   - **배선된 도어벨(운영)** = 여기서 **즉시** enqueue 한다. 논블록 채널 send 뿐이라 재진입도
        //     회계 오염도 없고, 미루면 멤버 k 의 깨우기가 **남은 전 멤버의 blocking write 뒤로** 밀린다
        //     (지연 손해). debug 빌드에서 주입 루프가 패닉하면 미뤄 둔 도어벨은 통째로 사라지기도 한다.
        //   - **미배선(인라인 폴백 — 실험 bin·단위 테스트)** = `flush_for_agent` 를 **이 스레드에서 그대로**
        //     실행한다. 그걸 여기서 부르면 방송 한가운데서 flush 에 재진입해, 아직 `results[]` 를 조립하지도
        //     않았는데 큐에 있던 그 방송분이 `delivered` 로 전이된다(응답은 `pending` 이라 회계가 어긋난다).
        //     그래서 인라인 갈래만 모아 두고 **주입 루프 종료 + 응답 확정 뒤**에 돌린다(아래 6단계).
        let mut deferred_inline: Vec<PeerId> = Vec::new();
        for (_, plan) in &plans {
            if let MemberPlan::Parked { doorbell, .. } = plan {
                self.ring_or_defer(*doorbell, &mut deferred_inline);
            }
        }

        // 5) 주입(★락 밖★) — 봉투는 **한 번만** 조립해 전 멤버가 같은 텍스트를 받는다(한 방송 = 한 봉투).
        let wrapped = self.wrap_now(
            sender_name,
            msg_id,
            body,
            &fanout_meta.envelope_fields(msg_id),
        );
        let mut results: Vec<GroupMemberResult> = Vec::with_capacity(plans.len());
        for (member, plan) in plans {
            let result = match plan {
                MemberPlan::Skipped { hint } => GroupMemberResult {
                    to: member,
                    status: GroupMemberStatus::Skipped,
                    hint: Some(hint),
                },
                MemberPlan::Parked { hint, .. } => GroupMemberResult {
                    to: member,
                    status: GroupMemberStatus::Pending,
                    hint: Some(hint),
                },
                MemberPlan::Deliver(target) => {
                    // ★멤버별 주입 직전 재확인(리뷰 fix C · load-bearing)★: 3단계의 `Deliver` 판정은 **한
                    //   락 구간**에서 전 멤버를 한꺼번에 정했는데, 실제 주입은 멤버 수만큼의 **순차 blocking
                    //   write** 다 — 앞 멤버에 쓰는 동안(파이프가 막히면 길게) 뒤 멤버의 세계가 바뀔 수 있다:
                    //     ① 그 사이 그 멤버가 **새 턴을 시작**했다 → 그대로 밀면 idle 게이트를 방송만 우회
                    //     ② 그 사이 그 멤버 앞으로 **다른 발송이 파킹**됐다 → 그대로 밀면 방송이 큐를 앞지른다
                    //   둘 다 단일 발송이 이미 3-b 에서 막는 사고라, 같은 판정 함수를 주입 **직전**에 한 번 더
                    //   태워 parity 를 맞춘다. 재확인이 뒤집히면 여기서 파킹(+결박)하고 `pending` 으로 보고한다.
                    // ★남는 창은 좁힐 뿐 없앨 수 없다★: 이 확인과 inject 사이(마이크로초)는 여전히 열려 있다 —
                    //   inject 를 락 안에서 하지 않는 한(모듈 헤더 절대 규율 위반) 구조적으로 닫히지 않는다.
                    let mut late_effects = ParkSideEffects::default();
                    // 회수 기준(F2) — 3단계와 **같은 스냅샷**에서 뽑는다("스냅샷 한 장" 계약).
                    let live_here = live_incarnations_named(&roster, &member);
                    let late = {
                        let mut st = self.state.lock().expect("messaging state poisoned");
                        self.plan_live_member(
                            &mut st,
                            ParkRequest {
                                msg_id,
                                sender_name,
                                from,
                                entrance,
                                recipient: &member,
                                body,
                                hinted_id: Some(target.id),
                                bound_incarnation: Some((target.id, target.epoch)),
                                live_incarnations: &live_here,
                                kind: ParkKind::Message,
                                meta: &fanout_meta,
                                // F3: 이 방송이 남길 행 수 = 멤버 수(두 기록 단계가 같은 값을 쓴다).
                                expected_rows: fanout_rows,
                            },
                            &target,
                            Instant::now(),
                            &mut late_effects,
                        )
                    };
                    late_effects.log(&member); // 락 밖 로깅(모듈 헤더 규율).
                    match late {
                        MemberPlan::Parked { hint, doorbell } => {
                            // 4단계와 **같은 분업**: 배선돼 있으면 즉시 enqueue, 인라인 폴백만 미룬다.
                            self.ring_or_defer(doorbell, &mut deferred_inline);
                            GroupMemberResult {
                                to: member,
                                status: GroupMemberStatus::Pending,
                                hint: Some(hint),
                            }
                        }
                        MemberPlan::Skipped { hint } => GroupMemberResult {
                            to: member,
                            status: GroupMemberStatus::Skipped,
                            hint: Some(hint),
                        },
                        MemberPlan::Deliver(target) => {
                            // ★결박 주입(round-3 fix 1 · load-bearing)★: 재확인조차 **발송 순간 스냅샷의**
                            //   target 을 쓰므로(로스터를 다시 뜨지 않는다 — "스냅샷 한 장" 계약), 재확인과
                            //   이 write 사이에 그 멤버가 재시작하면 무조건 주입은 새 incarnation 에 착지한다.
                            //   그 창은 검사로 못 닫는다(검사와 write 가 별개 연산) — 그래서 판정을 write 와
                            //   같은 단위로 내린다(`inject_if_epoch`). 실패하면 아래 Err 갈래 = 죽은 멤버와
                            //   같은 결말(`skipped`)이고, 그게 정확히 맞다: 발송 순간의 그 수신자는 이제 없다.
                            match self.port.inject_if_epoch(
                                target.id,
                                target.epoch,
                                wrapped.as_bytes(),
                            ) {
                                Ok(outcome) => {
                                    {
                                        let mut st =
                                            self.state.lock().expect("messaging state poisoned");
                                        // pending 없이 곧장 delivered(즉시 주입 — 단일 발송과 동일 규칙).
                                        st.ledger.record_with_expected(
                                            msg_id,
                                            sender_name,
                                            &member,
                                            body,
                                            DeliveryStatus::Delivered,
                                            Instant::now(),
                                            fanout_rows,
                                        );
                                    }
                                    self.observe_success(
                                        msg_id,
                                        &target,
                                        from,
                                        entrance,
                                        &wrapped,
                                        &outcome,
                                        // 그룹 방송은 reply_to 가 입구에서 이미 금지돼 있다(spec §4) — 자연히
                                        //   None. fanout_meta 에서 그대로 스레딩(구조화 출처, F1).
                                        fanout_meta.reply_to.clone(),
                                    );
                                    GroupMemberResult {
                                        to: member,
                                        status: GroupMemberStatus::Delivered,
                                        hint: None,
                                    }
                                }
                                Err(e) => {
                                    // ★write 실패 = 그 순간 도달 불가 = 죽은 멤버와 **같은 결말**(리뷰 fix B)★:
                                    //   spec §4 가 "발송 순간 살아있지 않은 멤버에겐 배달하지 않는다" 이므로
                                    //   여기서 파킹하면 그 멤버의 **다음 등장에 배달**돼 방송 소급 금지를 정면
                                    //   위반한다(옛 구현이 그랬다 — 파킹 + "retried on next appearance" hint).
                                    //   그래서 장부에 `skipped` 로 남기고 write 오류를 hint 에 실어 발신자가
                                    //   재발송 여부를 정하게 한다(조용한 유실 아님 — results 에 그대로 보인다).
                                    // ★epoch 불일치(round-3 fix 1)도 **여기로 온다**★: "그 사이 재시작해 발송
                                    //   순간의 그 incarnation 이 사라졌다" 는 위와 정확히 같은 사실이라 결말도
                                    //   같아야 한다. 원인은 `e`(= "epoch mismatch: …")가 hint 로 실어 나른다 —
                                    //   결말이 같으니 분기를 늘리지 않고 사유만 구분해 보여 준다.
                                    self.observe_failure(
                                        msg_id,
                                        &target,
                                        from,
                                        entrance,
                                        &wrapped,
                                        &e,
                                        fanout_meta.reply_to.clone(),
                                    );
                                    {
                                        let mut st =
                                            self.state.lock().expect("messaging state poisoned");
                                        st.ledger.record_with_expected(
                                            msg_id,
                                            sender_name,
                                            &member,
                                            body,
                                            DeliveryStatus::Skipped,
                                            Instant::now(),
                                            fanout_rows,
                                        );
                                    }
                                    GroupMemberResult {
                                        hint: Some(format!(
                                            "Delivery to '{member}' failed ({e}) — skipped for this broadcast (broadcasts are never re-delivered later; send it 1:1 if it must reach that agent)."
                                        )),
                                        to: member,
                                        status: GroupMemberStatus::Skipped,
                                    }
                                }
                            }
                        }
                    }
                }
            };
            results.push(result);
        }
        // 6) ★인라인 폴백 flush — 주입 루프가 끝나고 `results` 가 확정된 **뒤**(round-3 fix 3/5)★.
        //    배선된 도어벨은 이미 4단계·재확인 시점에 즉시 눌렀다(위 주석의 분업). 여기 남은 건 도어벨이
        //    없는 조립뿐이고, 그 갈래는 flush 를 이 스레드에서 동기 실행하므로 방송 한가운데서 부르면
        //    ① flush 재진입 ② 응답(`pending`)과 장부(`delivered`)의 시점 불일치가 난다. 응답을 다 만든
        //    뒤로 미루면 그 두 사고가 구조적으로 불가능해진다(응답은 이미 값으로 굳었다).
        //    중복 id 는 없다 — 한 멤버는 계획 파킹이거나 재확인 파킹이지 둘 다일 수 없다.
        for id in deferred_inline {
            self.flush_for_agent(id);
        }
        Ok(results)
    }

    /// ★도어벨 발화 시점 분업(round-3 fix 5)★ — 배선돼 있으면 **즉시** enqueue(논블록), 미배선이면
    ///   `deferred` 에 담아 호출자가 **응답 확정 후** 인라인 실행하게 한다.
    ///
    /// ★왜 갈라야 하나★: 두 갈래는 비용 구조가 정반대다. 배선 갈래는 채널 send 라 즉시 부르는 게 항상
    ///   옳고(미루면 그 멤버의 깨우기가 남은 blocking write 뒤로 밀리고, 패닉하면 통째로 유실된다),
    ///   미배선 갈래는 **이 스레드에서 배치 write 를 그대로 실행**하므로 주입 루프 한가운데서 부르면
    ///   flush 재진입 + 회계 skew(응답 `pending` vs 장부 `delivered`)를 만든다. 그래서 "즉시 vs 나중" 을
    ///   도어벨 배선 유무로 가른다. 근거 전문은 `handle_group_send` 4단계 주석.
    /// ★락 밖 호출★: `FlushTrigger`/인라인 flush 모두 messaging 락 밖에서만 부른다(모듈 헤더 규율).
    fn ring_or_defer(&self, id: PeerId, deferred: &mut Vec<PeerId>) {
        match &self.trigger {
            Some(t) => t.request_flush(id),
            None => deferred.push(id),
        }
    }

    /// ★산 멤버 1인의 "지금 주입해도 되나" 판정 + 안 되면 그 자리에서 파킹까지(C4 · 리뷰 fix C)★.
    ///
    /// 반환: `Deliver` = 지금 주입해라(락 밖에서) / `Parked` = 파킹했다(장부 `pending` 기록 완료, 도어벨만
    /// 남았다) / `Skipped` = 보관함 cap 이라 접었다(장부 `skipped` 기록 완료).
    ///
    /// ★왜 함수로 뽑았나(load-bearing — 두 지점의 parity)★: 이 판정은 **두 번** 불린다 — ① fan-out 계획
    ///   단계(전 멤버 한 락 구간) ② 각 멤버 **주입 직전**(계획이 stale 해졌을 수 있으므로 재확인). 두 곳에
    ///   규칙을 복제하면 한쪽만 고쳐져 "계획은 파킹인데 재확인은 주입" 같은 어긋남이 난다. 특히 hint 문구까지
    ///   같아야 발신자가 같은 사유를 같은 말로 본다.
    /// ★락 계약★: 호출자가 messaging 락을 **든 채** 부른다(`st` 를 그대로 받는다). 내부에서 `BusyGate` 를
    ///   부르는 건 계약상 안전하고(busy.rs), `DeliveryPort` 는 절대 부르지 않는다(모듈 헤더 절대 규율).
    ///   같은 이유로 **여기서 tracing 을 찍지 않는다** — park 이 남긴 로깅 거리는 `effects` 에 쌓아 호출자가
    ///   락을 놓은 뒤 찍는다(`ParkSideEffects::log`).
    fn plan_live_member(
        &self,
        st: &mut MessagingState,
        req: ParkRequest<'_>,
        target: &LiveAgent,
        now: Instant,
        effects: &mut ParkSideEffects,
    ) -> MemberPlan {
        // 지금 주입하면 안 되는 두 사유(둘 다 파킹 = `pending`): ① 턴 진행 중(idle 게이트, ADR-0104)
        //   ② 그 멤버 앞에 이미 먼저 나갈 게 있음(FIFO — 방송이 옛 메일을 앞지르지 않게).
        // ★판정 동사는 단일 발송 3-b 와 **같은 것**을 쓴다(round-7)★: 방송의 즉시 배달도 큐를 우회하는
        //   직주입이라 같은 사각을 공유했다 — 진행 중인 flush 배치(in-flight)를 큐만 보고 못 보면 방송이
        //   그 배치를 앞지른다. `has_pending_ahead` = 큐 배달 가능분 + 그 이름 앞 in-flight.
        // ★"큐 항" 은 **이 incarnation 에게 배달 가능한 것만** 센다(round-3 fix 2)★: 다른 incarnation 앞으로
        //   결박된 잔해는 이 멤버에게 절대 배달되지 않으므로 앞지를 대상이 아니다(mailbox `visible_to`).
        //   in-flight 항엔 그 필터가 안 걸린다(과다 차단 = 지연 — mailbox `has_pending_ahead` 주석).
        let busy = self.busy.is_busy(target.id, target.epoch);
        let queued = st
            .mailbox
            .has_pending_ahead(req.recipient, Some((target.id, target.epoch)));
        if !busy && !queued {
            return MemberPlan::Deliver(target.clone());
        }
        let member = req.recipient;
        let hint = if busy {
            format!(
                "'{member}' is mid-turn — this broadcast is parked and delivered as one batch when that turn ends."
            )
        } else {
            format!(
                "'{member}' has earlier queued messages — this broadcast joins that queue and is delivered in order."
            )
        };
        let full_hint = format!(
            "'{member}' mailbox is full — skipped for this broadcast (its oldest parked messages expire by TTL; the other members still received it)."
        );
        let (msg_id, sender_name, recipient, body, expected_rows) = (
            req.msg_id,
            req.sender_name,
            req.recipient,
            req.body,
            req.expected_rows,
        );
        match park_into(st, req, now, effects) {
            Ok(()) => MemberPlan::Parked {
                hint,
                doorbell: target.id,
            },
            // ★멤버 하나의 보관함 cap 이 방송 전체를 막지 않는다(handle_group_send doc 정책)★.
            Err(ParkError::MailboxFull) => {
                st.ledger.record_with_expected(
                    msg_id,
                    sender_name,
                    recipient,
                    body,
                    DeliveryStatus::Skipped,
                    now,
                    expected_rows,
                );
                MemberPlan::Skipped { hint: full_hint }
            }
        }
    }

    /// 발송 3분기 본체(C1/C2) — `handle_single_send` 가 계약 예약/닫기로 감싸는 안쪽.
    ///
    /// 인자 `resolved`/`park_key`/`live_here` 는 호출자가 이미 계산해 넘긴다(계약 오픈이 recipient 를 먼저
    /// 알아야 하므로 로스터 해석을 밖으로 끌어올렸다 — 여기서 재조회하면 두 판정이 서로 다른 스냅샷을 보게
    /// 된다). `live_here` = 그 park 키를 지금 달고 있는 산 incarnation 전부(F2 압력 회수 기준 —
    /// `live_incarnations_named`). 아래 네 파킹 갈래가 **전부 같은 값**을 쓴다: park 키가 갈래마다 다르지
    /// 않기 때문이다(해석 성공이면 `park_key == target.name`).
    #[allow(clippy::too_many_arguments)]
    fn dispatch_single(
        &self,
        msg_id: &str,
        from: SenderIdentity,
        sender_name: &str,
        to: &str,
        body: &str,
        entrance: Entrance,
        meta: &SendMeta,
        resolved: Option<LiveAgent>,
        park_key: &str,
        live_here: &[(PeerId, u32)],
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
                // 유일 해석은 실패했지만 **생사는 알 수도 있다**(동명 다수 갈래) — 그 목록 그대로 넘긴다(F2).
                live_here,
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
                // 회수 기준 = 이 이름의 산 incarnation 전부(결박 아님 — park_pending doc 의 두 축 구분).
                live_here,
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

        // 3-b) ★FIFO 일관성(C2 리뷰 fix 5 · round-7 보정 · load-bearing)★: 수신자는 idle 인데 그 이름 앞에
        //    **먼저 나갈 게 남아 있으면** 직발송이 그것을 앞지른다 — 수신자가 보는 순서가 (새것, 옛것들) 로
        //    뒤집힌다. "오래된 순 일괄" 계약(ADR-0104)은 큐 안에서만 성립하는 게 아니라 **그 수신자가 보는
        //    순서**에 대한 약속이라, 앞에 뭔가 있으면 이 메시지도 큐 뒤에 붙이고 flush 를 눌러 한 배치로
        //    순서대로 나가게 한다. (앞에 아무것도 없으면 앞지를 대상이 없으므로 그대로 직발송 = C1 동작.)
        //    ★"앞에 있다" 는 큐만이 아니다(round-7 high — C2/C3 flush 설계 이래의 사각)★: flush 는 큐를
        //    통째로 비운 뒤 **락을 놓고** 주입한다. 그 구간에 큐는 비어 보이지만 그 배치는 아직 수신자에게
        //    닿지 않았다 — 큐만 세면 여기서 즉시 주입해 **진행 중인 배치를 앞지른다**. 그래서 판정은
        //    `has_pending_ahead`(= 큐 배달 가능분 + 그 이름 앞 in-flight) 한 동사로 한다.
        //    ★"큐 항" 은 **이 incarnation 에게 배달 가능한 것**만 센다(round-3 fix 2)★: 다른 incarnation
        //    앞으로 결박된 방송 잔해는 이 수신자에게 배달될 일이 없으므로 앞지를 대상이 아니다. 총량으로
        //    세면 죽은 방송분이 산 수신자의 즉시 배달을 TTL(24h)까지 막는다(mailbox `visible_to`).
        //    ★in-flight 항은 그 필터를 못 건다(과다 차단을 감수한다 — mailbox `has_pending_ahead` 주석)★:
        //    영수증은 건수만 알아 결박을 모른다. 그 대가는 **지연**뿐이다(파킹 + 도어벨 → 그 flush 가 끝나면
        //    바로 다음 배치로 나간다). 반대 방향의 오차(과소 차단)는 곧바로 순서 역전이라 대칭이 아니다.
        let has_queued = {
            let st = self.state.lock().expect("messaging state poisoned");
            st.mailbox
                .has_pending_ahead(&target.name, Some((target.id, target.epoch)))
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
                live_here,
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
                // in_reply_to = 이 발송의 SendMeta.reply_to 그대로(구조화 출처, F1 — 봉투 재파싱 없음).
                self.observe_success(
                    msg_id,
                    &target,
                    from,
                    entrance,
                    &wrapped,
                    &outcome,
                    meta.reply_to.clone(),
                );
                Ok(SendOutcome::Delivered)
            }
            Err(e) => {
                // 도달 불가/write 실패 → 파킹(spec §5 unreachable → 파킹). 관측 레코드는 실패로 남기고
                //   메시지는 유실하지 않는다(park + ledger pending). park 키 = canonical name(등장 flush 키와 정합).
                // ★self-heal 안 함(의도적 — finding 3 범위)★: finding 3 은 **부재→등장** 레이스(resolve 시점
                //   부재라 park 했으나 그 사이 등장)만 자가치유한다. inject 실패는 방금 그 incarnation 이
                //   도달 불가해진 것이라, 같은 roster 로 즉시 재-flush 하면 깨진 수신자에 재주입을 반복할 수
                //   있다(무한 재시도 위험). 실패분은 다음 **진짜** 등장(epoch bump)의 flush observer 에 맡긴다.
                self.observe_failure(
                    msg_id,
                    &target,
                    from,
                    entrance,
                    &wrapped,
                    &e,
                    meta.reply_to.clone(),
                );
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
                    live_here,
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
    fn request_flush(&self, id: PeerId) {
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
    /// ★kind(C3 · round-6)★: `Notice` 는 **자기 레인**(`mailbox::NOTICE_CAP`)에서 회계되고 넘치면 가장
    ///   오래된 통지를 회수할 뿐 반려되지 않으므로, 이 함수는 notice 에 대해 절대 `MailboxFull` 을 내지
    ///   않는다(mailbox.rs `park`). 그래서 notice 호출부는 반환값을 무시해도 안전하다.
    /// ★meta(C3)★: 회신 계약 속성은 **payload 에 실려 flush 까지 살아남는다** — 늦게 배달되는 request/회신도
    ///   즉시 배달과 **같은 봉투**(같은 속성)로 나가야 하기 때문이다(park 시점에 봉투를 굳히지 않는 설계라
    ///   속성 재료를 함께 날라야 한다 — 모듈 헤더 "봉투 = 주입 시점 조립").
    /// ★incarnation 결박 없음(C4 리뷰 fix A — 이 래퍼의 계약)★: 여기로 오는 파킹은 **단일 발송·notice** 뿐이라
    ///   `bound_incarnation` 을 항상 `None` 으로 고정한다. 그 둘은 이름 주소가 정본이고 재스폰 이어받기가
    ///   **기능**이기 때문이다(canonical_park_key 주석) — 결박은 "발송 순간 스냅샷" 계약을 가진 그룹 방송만의
    ///   축이고, 그 경로는 `park_into` 를 직접 부른다(`handle_group_send`). 여기에 결박 인자를 열면 단일 발송이
    ///   실수로 결박돼 재스폰 배달이 조용히 멈출 수 있다.
    /// ★인자 `live`(F2 — 압력 회수의 기준축)★ = 이 park 시점에 **`recipient` 이름을 달고 있는 산
    ///   incarnation 전부**(호출자가 자기 로스터 스냅샷에서 `live_incarnations_named` 로 뽑는다). 하나도 못
    ///   해석했으면 빈 슬라이스 = "모른다" → 저장소는 아무것도 회수하지 않는다. **결박이 아니다**(위 문단과
    ///   충돌하지 않는다 — `ParkRequest.live_incarnations` 주석의 두 축 구분 참조). 단수 `current` 였던 옛
    ///   축이 왜 산 메일을 잡아먹었는지는 mailbox `is_stale_bound` 주석이 정본.
    #[allow(clippy::too_many_arguments)]
    fn park_pending(
        &self,
        msg_id: &str,
        sender_name: &str,
        from: SenderIdentity,
        entrance: Entrance,
        recipient: &str,
        body: &str,
        hint: String,
        hinted_id: Option<PeerId>,
        live: &[(PeerId, u32)],
        kind: ParkKind,
        meta: &SendMeta,
    ) -> Result<SendOutcome, SendReject> {
        let now = Instant::now();
        // ★락 보유 중 tracing 금지(모듈 헤더 규율)★ — 압력 회수 사실은 여기 모았다가 락을 놓고 찍는다.
        let mut effects = ParkSideEffects::default();
        let parked = {
            let mut st = self.state.lock().expect("messaging state poisoned");
            park_into(
                &mut st,
                ParkRequest {
                    msg_id,
                    sender_name,
                    from,
                    entrance,
                    recipient,
                    body,
                    hinted_id,
                    // 위 doc "incarnation 결박 없음" — 단일 발송·notice 는 이름 주소 규칙 그대로.
                    bound_incarnation: None,
                    live_incarnations: live,
                    kind,
                    meta,
                    // F3: 단일 수신자(및 notice)는 배달기록이 정확히 1행이다.
                    expected_rows: 1,
                },
                now,
                &mut effects,
            )
        };
        effects.log(recipient);
        match parked {
            Ok(()) => Ok(SendOutcome::Parked { hint }),
            Err(ParkError::MailboxFull) => Err(SendReject::MailboxFull),
        }
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
    ///   3. deliverable 을 **해석된 타깃별로 분할**(항목별 **결박(그룹 방송) 우선** → id 힌트 → 이름 유일
    ///      도달 규칙). 결박 항목은 그 `(id, epoch)` 가 없으면 **폴백 없이** 파킹 유지다(fix A).
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
        let mut bound_kept = 0usize;
        let mut busy_skipped: Vec<(PeerId, u32, usize)> = Vec::new();
        // 만료/회수 전이가 링버퍼 evict 때문에 실패한 항목(C4 리뷰 fix J) — 락 밖에서 debug 로 남긴다.
        //   ★의도한 종점 상태를 함께 나른다(round-5 finding 2)★: 이 함에는 `expired`(TTL)와 `skipped`
        //   (확정 사망 회수) 두 어휘가 섞여 들어오므로, 상태를 안 실으면 로그가 전부 만료로 뭉개진다.
        let mut evicted_transitions: Vec<EvictedTransition> = Vec::new();
        // 확정 사망 결박이라 즉시 종점(`skipped`)을 찍고 버린 항목(round-3 fix 6) — 락 밖 debug.
        let mut retired_dead: Vec<String> = Vec::new();
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
            //   규칙(respawn 이 파킹을 이어받는 이름-키 설계 — canonical_park_key 주석).
            //   ★단 결박 항목(그룹 방송)은 이 두 규칙보다 **앞서고 대체한다**★ — 아래 match 첫 팔(fix A).
            //   그룹은 **등장 순서**대로, 그룹 안 인덱스도 **오래된 순**이라 배달 순서가 보존된다.
            let mut groups: Vec<(LiveAgent, Vec<usize>)> = Vec::new();
            let mut restore: Vec<usize> = Vec::new();
            // ★확정 사망 결박(round-3 fix 6)★ — 큐로 **되돌리지 않는다**. 아래에서 장부에 종점을 찍는다.
            let mut retired: Vec<usize> = Vec::new();
            for (idx, parked) in deliverable.iter().enumerate() {
                let target = match parked.bound_incarnation {
                    // ★결박 항목 = 그 (id, epoch) **에게만**(C4 리뷰 fix A · load-bearing)★: 그룹 방송의
                    //   파킹분이다. 이름 폴백도, 같은 id 의 다른 epoch 도 **쓰지 않는다** — 그 둘이 정확히
                    //   "발송 뒤 등장한 수신자에게 소급 배달" 이 나던 두 경로였다(mailbox 주석의 실측 ①②).
                    Some((bid, bepoch)) => match roster.iter().find(|a| a.id == bid) {
                        // 그 incarnation 이 그대로 산 채 — 배달 후보.
                        Some(a) if a.epoch == bepoch => Some(a.clone()),
                        // ★같은 id 가 **더 높은 epoch** 으로 있다 = 확정 사망(round-3 fix 6)★: epoch 은 한
                        //   PeerId 안에서 단조 증가한다(ADR-0007 — 재spawn/복원마다 +1, 되돌아가지 않는다).
                        //   그러니 이건 "지금 안 보인다" 가 아니라 **되돌아올 수 없다는 증거**다. 증거가 있는데
                        //   24시간을 더 붙들면 큐·메모리·스캔 비용만 물고 장부는 그동안 `pending` 이라 거짓말을
                        //   한다(발신자가 조회하면 "아직 배달 대기 중" 으로 보인다). 그래서 즉시 종점을 찍는다.
                        // ★어휘 = `skipped`(신설 금지 — spec §5 어휘 재사용)★: `expired` 는 **시계 사실**(TTL
                        //   도달)을 주장하는 상태라 여기 쓰면 장부가 거짓 원인을 남긴다(사유는 시계가 아니라
                        //   수신자 소멸이다). spec §4 의 `skipped` = "그 멤버에게는 배달하지 않음(부재/죽음)"
                        //   이 정확히 같은 사실이고, fan-out 의 죽은 멤버·write 실패와도 **같은 결말**이라
                        //   발신 LLM 이 보는 회계가 한 어휘로 통일된다.
                        Some(a) if a.epoch > bepoch => {
                            retired.push(idx);
                            continue;
                        }
                        // epoch 이 **더 낮게** 보이는 건 단조성 위반이라 있을 수 없다 — 우리 모델이 틀린
                        //   상황이므로 파괴적 판정(버리기)을 하지 않고 보수적으로 보류한다.
                        Some(_) => None,
                        // ★로스터에 아예 없음 → 보류(의도적)★: "안 보인다" 는 **영구 사망의 증거가 아니다** —
                        //   재시작 중이거나 잠깐 비-도달(비-structured)일 수 있고, 그 판정을 여기서 내리면
                        //   살아 돌아온 **그 incarnation**(같은 id·같은 epoch)이 받을 수 있었던 메시지를 우리가
                        //   먼저 버리는 셈이다. 결박이 이미 소급을 막고 있으므로 위험은 "늦게 배달" 이 아니라
                        //   "안 배달" 뿐이고, 그 종점은 TTL(`expired`)이 정한다.
                        // ★두 규칙을 함께 읽어야 전체 그림이다(round-6)★: 이 보류는 **무한 보관이 아니다**.
                        //   여기서 회수되는 건 "같은 PeerId 가 더 높은 epoch 으로 돌아온" 증거 있는 사망뿐이라,
                        //   같은 이름이 **완전히 새 PeerId** 로 대체된 경우의 옛 결박은 이 경로로는 영영
                        //   회수되지 않는다(고아). 그 갈래의 backstop 은 저장소의 **압력 회수**다 — 큐가
                        //   `mailbox::MAILBOX_CAP` 에 닿으면 park 이 **배달 불가 잔해**(= 결박이 그 park 의
                        //   생사 스냅샷에 없는 항목 — F2)를 오래된 순으로 걷어내고 장부에 종점을 남긴다
                        //   (보통 `skipped`, 이미 TTL 을 넘겼으면 `expired` — F3). 즉 회수 주체는 ① 로스터
                        //   증거가 있으면 여기(flush) ② 증거가 없으면 압력이 찰 때 park, 그 위에 TTL.
                        // ★여기서 걷어내는 게 **복원보다 먼저**인 것이 중요하다(round-6)★: 이 함수는 로스터를
                        //   손에 쥔 유일한 지점이라 증거 있는 사망분을 여기서 떨궈야, 아래 `restore_ordered` 로
                        //   되돌아가는 배치가 그만큼 작아진다(= 되돌아오는 몫과 in-flight 분모를 함께 줄인다).
                        None => None,
                    },
                    None => parked
                        .hinted_id
                        .and_then(|h| roster.iter().find(|a| a.id == h).cloned())
                        .or_else(|| name_target.clone()),
                };
                match target {
                    Some(t) => match groups.iter_mut().find(|(g, _)| g.id == t.id) {
                        Some((_, idxs)) => idxs.push(idx),
                        None => groups.push((t, vec![idx])),
                    },
                    // 배달 경로 없음(이름이 부재/동명 다수 + 힌트도 사망, 또는 결박 incarnation 부재) → 파킹 유지.
                    None => {
                        // 두 사유를 갈라 센다 — 로그가 "이름이 안 풀렸다" 로 뭉개지면 결박 보류를 오진한다.
                        if parked.bound_incarnation.is_some() {
                            bound_kept += 1;
                        } else {
                            no_target_kept += 1;
                        }
                        restore.push(idx);
                    }
                }
            }

            // ★확정 사망분 종점 찍기(round-3 fix 6)★ — 큐에서는 이미 나왔으므로(drain) 되돌리지 않는 것만으로
            //   제거된다. 조용한 유실 금지: 반드시 장부에 `skipped` 를 남긴다. `NotFound`(이력 링에서 evict)는
            //   만료 전이와 **같은 규율**로 사실만 모아 락 밖에서 debug(fix J).
            for &idx in &retired {
                let item = &deliverable[idx];
                transition_or_collect_evicted(
                    &mut st.ledger,
                    &item.msg_id,
                    recipient,
                    DeliveryStatus::Skipped,
                    now,
                    &mut evicted_transitions,
                );
                retired_dead.push(item.msg_id.clone());
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
            let groups: Vec<(LiveAgent, Vec<ParkedMessage>)> = deliver
                .into_iter()
                .map(|(t, idxs)| {
                    let items: Vec<ParkedMessage> =
                        idxs.iter().map(|&i| deliverable[i].clone()).collect();
                    (t, items)
                })
                .collect();
            // ★in-flight 등록 — **락을 떠나는 몫만**(F1 · load-bearing)★: cap 분모의 구멍은 이 락이 풀린
            //   구간에만 있다. 위에서 이미 되돌린 몫(busy·타깃 없음)과 종점을 찍은 몫(확정 사망·만료)은 그
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
        if bound_kept > 0 {
            tracing::debug!(
                recipient,
                stale_id = %to_id,
                keeping = bound_kept,
                "flush skip: 그룹 방송 파킹분의 결박 incarnation(발송 순간 id·epoch)이 지금 로스터에 없음 — 파킹 유지(소급 배달 금지, TTL 이 종점 · fix A)"
            );
        }
        if !retired_dead.is_empty() {
            tracing::debug!(
                recipient,
                retired = retired_dead.len(),
                msg_ids = ?retired_dead,
                "flush retire: 결박 incarnation 이 **확정 사망**(같은 PeerId 가 더 높은 epoch 으로 존재 — epoch 단조, ADR-0007) — 큐에서 제거하고 장부 skipped(TTL 대기 없음 · round-3 fix 6)"
            );
        }
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
                // ★결박 항목만 조건부 주입(round-3 fix 1 · load-bearing)★: 위 타깃 해석은 **락 안의 로스터
                //   스냅샷**으로 했는데, 실제 write 는 락 밖이고 앞 항목의 blocking write 뒤에 온다 — 그
                //   사이 수신자가 재시작하면 무조건 주입이 새 incarnation 에 착지해 결박이 무력화된다.
                //   그래서 결박 항목은 판정을 write 와 같은 단위로 내린다.
                // ★이름 항목(단일 발송·notice)은 **일부러** 무조건 주입한다★: 그쪽 주소 단위는 이름이고
                //   재스폰 이어받기가 **기능**이다(ADR-0101 · canonical_park_key 주석). 여기에 epoch 검사를
                //   걸면 "재시작한 에이전트가 자기 앞 파킹을 못 받는" 회귀가 된다.
                let inject_result = match parked.bound_incarnation {
                    Some((_, bepoch)) => {
                        self.port.inject_if_epoch(to_id, bepoch, wrapped.as_bytes())
                    }
                    None => self.port.inject(to_id, wrapped.as_bytes()),
                };
                match inject_result {
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
                            // ★의무는 봉투를 **실제로 받은 자**를 따른다(round-2 리뷰 F2)★: 이름 큐 파킹은
                            //   재스폰 이어받기가 기능이라(ADR-0101) 발송 시점 결박과 **다른** incarnation
                            //   에게 배달될 수 있다 — exact-id 로 건 request 가 busy 라 파킹됐다가 그 A 가
                            //   죽고 동명 B 가 떠 B 에게 꽂히는 경우가 그렇다. 계약의 recipient_id 를 여기서
                            //   실제 수신자로 고쳐야 B 의 미결 조회가 자기 의무를 본다(안 그러면 봉투를 받은
                            //   쪽이 "답할 게 없다" 고 읽는다). 여기가 착지 incarnation 을 아는 유일한 지점이다.
                            //   통보·notice 는 계약이 없어 no-op.
                            st.ledger.rebind_request_recipient(&parked.msg_id, to_id);
                            st.mailbox.settle_in_flight(recipient, settled);
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
                        //   상대 순서도 무의미하지 않다: 큐 앞머리는 `handle_single_send` 의 FIFO 합류 판정과
                        //   다음 배치의 배달 순서를 결정하고, 나이 역전은 sweep 이 만료 항목을 지나치게 만든다.
                        // ★결박 epoch 불일치(round-3 fix 1)도 이 갈래로 온다 — 같은 처리가 맞다★: 그 수신자가
                        //   재시작했다는 뜻이므로 (a) 결박 항목은 되돌려야 하고(이 배치에서 배달 불가, 다음
                        //   flush 가 "확정 사망" 으로 보고 종점을 찍는다 — fix 6) (b) 남은 이름 항목도 되돌려
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

    /// ★park 키 정규화(finding 4)★: `to` 가 exact PeerId 문자열이고 그 에이전트가 존재하면(도달 여부 무관)
    ///   그 canonical **NAME** 을 park 키로 돌려준다 — 등장 flush 가 canonical name 으로 keyed 이기 때문
    ///   (UUID 로 park 하면 name-keyed flush 가 영영 못 잡는다). 파싱 실패/미존재면 리터럴 문자열 그대로.
    /// ★락 밖 호출★: canonical_name 은 port 호출(내부 락)이라 messaging 락 밖에서만 부른다(모듈 헤더 규율).
    ///
    /// ★파킹은 이름-키가 설계다(finding 4 · ADR-0101/0103)★: 여기서 exact-PeerId 지목조차 canonical NAME
    ///   으로 park 키를 바꾼다는 건, 파킹의 **주소 단위가 이름**이라는 뜻이다(id 가 아니라). 왜:
    ///   - **WYSIWYA 이름 주소(ADR-0101)**: 에이전트가 서로를 부르는 1급 주소는 표시 이름이다. exact-id 는
    ///     그걸 해석하는 send-time **편의**일 뿐, 파킹의 정체성은 이름이다.
    ///   - **respawn 생존(ADR-0103)**: 파킹의 존재 이유가 "지금 없는(또는 곧 죽었다 다시 뜰) 이름 앞으로 미리
    ///     쌓아 둠" 이다. 재스폰된 에이전트는 **새 PeerId** 를 얻으므로, id 로 park 하면 재스폰분이 자기 앞
    ///     파킹을 절대 못 받는다 — 이름으로 park 해야 새 incarnation 이 등장 flush 로 이어받는다.
    ///
    /// 함의(수용된 잔여): exact-PeerId 로 보내 파킹된 메일이, 나중에 **같은 이름의 다른 에이전트**가 유일
    ///   도달이 되면 그쪽으로 배달될 수 있다. 이건 이름 주소의 accepted residual 이며, uniqueness-게이트
    ///   flush/ambiguity 정책(동명 다수면 배달 보류 — finding 2/3, `unique_reachable_in`)과 일관된다:
    ///   이름이 유일하게 풀릴 때만 배달하므로, "그 이름을 쓰는 지금 유일한 에이전트" 로 배달된다는 계약이 유지된다.
    ///
    /// ★그러나 이름-키 **단독**이면 exact-id 발송에 blackhole 이 생긴다(C2 리뷰 fix 2)★: exact-PeerId
    ///   지목은 발송 단계에서 동명 모호성을 **의도적으로 통과**한다(id 가 명시적 승자 — ingress). 그런데
    ///   그 수신자가 턴 중이라 이름-키로 park 되면, 동명이 둘인 동안 flush 의 유일성 게이트가 영영 보류해
    ///   TTL 만료까지 배달되지 않는다. 그래서 park 항목은 해석된 id 를 **힌트**로 함께 들고 다니고
    ///   (`ParkedMessage.hinted_id`), flush 는 **힌트가 아직 살아 있으면 이름 유일성과 무관하게** 그쪽으로
    ///   배달한다(힌트가 죽었으면 위의 이름 규칙으로 복귀 — 재스폰 이어받기 유지). park **키**는 여전히
    ///   이름이다(힌트는 배달 우선순위일 뿐 주소 축이 아니다).
    fn canonical_park_key(&self, to: &str) -> String {
        if let Ok(id) = to.parse::<PeerId>() {
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
    /// ★만료 전이의 `NotFound` 는 조용히 삼키지 않는다(C4 리뷰 fix J)★: 방송은 한 발송이 레코드를 **N개**
    ///   쓰므로 이력 링(`ledger::HISTORY_CAPACITY`)이 훨씬 빨리 회전한다 — 파킹이 만료될 즈음 그 레코드가
    ///   이미 밀려났을 수 있고, 그러면 "만료됐다" 는 사실이 **어디에도** 남지 않는다. 전이는 여전히
    ///   best-effort(장부를 되살리진 않는다)지만, 그 사실을 debug 로 남겨 관측 가능하게 한다(락 밖 로깅).
    pub fn sweep(&self, now: Instant) {
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
        // ★생사 스냅샷은 **반드시 실제로 해석한 값**을 넘긴다(round-6 finding — 대량 회수 결함의 진원)★:
        //   이 값은 park 의 압력 회수에서 "무엇이 배달 불가 잔해인가" 의 기준축이다. 옛 구현은 여기서
        //   `None`(= "지금이 누구인지 모른다")을 넘겼고, 그때의 회수 판정(`!visible_to`)은 그걸 **"큐의 모든
        //   결박 항목이 stale"** 로 읽어 — 바로 이 수신자에게 배달될 산 메일까지 회수 후보에 올렸다. 지금은
        //   저장소가 "모름" 과 "없음" 을 구분해 모르면 회수하지 않지만(mailbox `is_stale_bound`), 그건
        //   방어선이지 면허가 아니다: 우리는 여기서 그 incarnation 을 **이미 알고 있다**(로스터 스냅샷).
        // ★목록 구성(F2)★ = ① 그 이름을 지금 달고 있는 산 incarnation 전부(동명 다수 포함) ② 거기에 장부가
        //   든 **발신자 id** 가 살아 있으면 그것도 더한다 — 그 사이 개명했으면 이름으로는 안 잡히지만 이 큐의
        //   주인은 여전히 그 incarnation 이라, 빼면 자기 앞 결박 메일이 잔해로 몰린다. 둘 다 없으면 빈 목록
        //   (정직한 "모른다" — 회수 없음).
        // ★오늘 notice 레인은 이 값을 소비하지 않는다(회수 기준이 "가장 오래된 통지" 라 결박과 무관)★ —
        //   그래도 넘기는 이유는 park 호출부의 계약을 한 벌로 유지하기 위해서다. "이 호출부만 거짓말해도
        //   된다" 는 예외가 정확히 지난 결함이었다.
        let mut live_here = live_incarnations_named(roster, recipient);
        if let Some(a) = roster.iter().find(|a| a.id == due.sender_id) {
            if !live_here.contains(&(a.id, a.epoch)) {
                live_here.push((a.id, a.epoch));
            }
        }
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
            &live_here,
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

    // ── 그룹 관리 표면(D · spec §4·§6 `group { group?, add?, remove?, delete? }`) ────────────
    //
    // ★C4 의 `_for_test` 통로를 대체한다★: C4 는 해석·방송만 배선하고 명단 채우기는 테스트 seam
    //   (`add_group_member_for_test`)으로 때웠다. D 가 정식 표면을 붙였으므로 그 seam 은 **삭제**됐고,
    //   단위 테스트도 아래 운영 API 를 그대로 쓴다(테스트 전용 경로가 남으면 실제로 도는 코드와 갈린다).
    // ★ACL 없음(사용자 결정 2026-07-26)★: 누구나 어떤 그룹이든 만들고 고치고 지운다 — 행위자 기록도 하지
    //   않는다(v2 백로그). 그래서 이 API 들은 **발신자 신원을 인자로 받지 않는다**(안 쓰는 값을 받으면
    //   "언젠가 검사하겠지" 라는 잘못된 기대를 남긴다).
    // ★알림 없음(사용자 결정 2026-07-26)★: 멤버 증감·삭제는 조용하다 — 어떤 notice 도 만들지 않는다.
    // ★스냅샷 원칙(spec §4 · 불변식)★: 명단 변경은 **앞으로의 발송**에만 영향을 준다. 이미 파킹된 방송분은
    //   발송 순간의 `(id, epoch)` 에 결박돼 있어(C4) 이 API 들이 건드리지 않는다 — 그룹을 지워도 파킹분은
    //   그대로 배달된다(회귀 그물 = `removing_a_member_does_not_cancel_an_already_parked_broadcast`).

    /// 그룹 이름 목록 — 내장 `@all` 이 **항상 맨 앞**, 그 뒤 등록 그룹을 사전순으로.
    ///
    /// ★왜 정렬하나★: `Groups::list` 는 HashMap 순회라 순서가 비결정적이다(그 자체는 정상 — 저장 구조의
    ///   자유). 그런데 이 값은 LLM 이 읽는 응답이자 테스트가 단언하는 값이라, 표시 계층에서 한 번 고정한다
    ///   (`Groups` 를 BTreeMap 으로 바꾸는 대신 여기서 정렬 — 저장 구조 선택을 표시 요구가 끌고 가지 않게).
    pub fn group_list(&self) -> Vec<String> {
        let st = self.state.lock().expect("messaging state poisoned");
        let mut names = st.groups.list();
        drop(st);
        // `@all` 은 항상 첫 원소로 분리해 두고 나머지만 정렬한다(내장이 목록 머리 = 발신자가 먼저 본다).
        let rest_start = usize::from(names.first().map(|n| n == ALL_GROUP).unwrap_or(false));
        names[rest_start..].sort();
        names
    }

    /// 그룹 멤버 조회. 등록 그룹이면 명단 그대로(빈 그룹은 빈 목록), `@all` 이면 **지금 살아 있는 수신 가능
    /// 전원**(발송 시 쓰는 것과 같은 스냅샷 규칙 — verbatim, 정렬·dedup 없음).
    ///
    /// ★락 규율(모듈 헤더)★: `@all` 은 로스터가 필요하므로 port 호출을 **락 밖에서 먼저** 하고, 그 스냅샷을
    ///   들고 락에 들어간다(락 보유 중 port 호출 금지).
    pub fn group_members(&self, group: &str) -> Result<Vec<String>, GroupError> {
        // 이름 규약 위반은 로스터를 조회하기 전에 거른다(불필요한 port 호출 회피 + 판정 순서 고정).
        let norm = normalize_group_name(group)?;
        if norm == ALL_GROUP {
            let live: Vec<String> = self
                .port
                .live_reachable_agents()
                .into_iter()
                .map(|a| a.name)
                .collect();
            // ★락을 잡지 않는다(리뷰 NOTE)★: `@all` 은 저장소에 없는 내장 그룹이라(groups.rs 불변식) 볼
            //   상태가 없다 — 예전엔 "일관성" 명목으로 락만 잡고 아무 것도 안 읽었는데, 그건 독자에게
            //   "여기 공유 상태가 관여한다" 는 거짓 신호를 준다.
            // ★`resolve` 가 아니라 여기서 직접 verbatim★: resolve 는 live 0명을 `Empty` 로 거부하는데(발송
            //   반려용), 조회에서 "지금 아무도 없다" 는 정상 답(빈 목록)이다.
            return Ok(live);
        }
        let st = self.state.lock().expect("messaging state poisoned");
        st.groups.members_of(&norm)
    }

    /// 멤버 증감(spec §6 `group @g --add a,b [--remove c]`). 반환 = **적용 후** 멤버 목록.
    ///
    /// ★암묵 생성(사용자 결정 2026-07-26)★: 없는 그룹에 `add` 하면 그 자리에서 생긴다 — 별도 create 동사는
    ///   두지 않는다(동사 하나를 줄여 LLM 이 틀릴 표면을 없앤다). 반대로 `remove` 만으로는 절대 생기지
    ///   않는다(없는 그룹에서 빼기 = `NotFound`).
    /// ★순서 = add 먼저, remove 나중★: 한 호출에 같은 이름이 양쪽에 있으면 최종 상태는 "빠진 것" 이다.
    ///   반대 순서면 add 가 이겨 "지우라고 했는데 남는" 결과가 되는데, 제거 의도를 무시하는 쪽이 더 위험하다
    ///   (다음 방송이 그 멤버에게 나간다).
    /// ★멱등★: 이미 있는 멤버 add·없는 멤버 remove 는 no-op(에러 아님 — `Groups` 계약).
    /// ★인자 둘 다 비면 순수 조회★로 동작한다(부작용 0) — 입구가 "이름만 준 호출" 을 이 함수로 흘려도
    ///   안전하다. 단 그때도 없는 그룹은 `NotFound`(빈 add 가 그룹을 만들지 않는다).
    pub fn group_update(
        &self,
        group: &str,
        add: &[String],
        remove: &[String],
    ) -> Result<Vec<String>, GroupError> {
        let norm = normalize_group_name(group)?;
        let members = {
            let mut st = self.state.lock().expect("messaging state poisoned");
            // ★원자적 배치(round-3 리뷰 G2)★: 예전엔 여기서 `add_member` 를 루프로 돌려, 배치 중간의
            //   이름 규약 위반이 **앞부분만 반영된 채** 에러를 내는 부분 변경을 남겼다(입구 검증을 우회하는
            //   내부 호출자에게 그대로 노출). 검증·적용의 원자성을 자료구조 쪽에 두고 여기선 위임만 한다.
            //   내장 `@all` 거절·이름 규약도 Groups 가 판정한다(규약 정본을 복제하지 않는다).
            st.groups.update_members(&norm, add, remove)?
        };
        // ★락 밖 계측(모듈 헤더 락 규율)★: 명단 변경은 이후 모든 방송의 수신자 집합을 바꾸는 사건인데
        //   지금까지 아무 흔적도 남기지 않았다(mcp-config write/remove 는 남긴다 — 같은 급의 상태 변경).
        //   ★행위자는 안 찍는다★: ACL·행위자 기록은 v2 로 연기된 별건이고(사용자 결정 2026-07-26), 여기
        //   필드를 늘리면 그 결정을 앞질러 구현하는 셈이 된다. 무엇이 어떻게 바뀌었나만 남긴다.
        if !add.is_empty() || !remove.is_empty() {
            tracing::info!(
                group = %norm,
                added = add.len(),
                removed = remove.len(),
                members = members.len(),
                "그룹 명단 변경(spec §4 — 이후 발송에만 영향)"
            );
        }
        Ok(members)
    }

    /// 그룹 삭제. 없으면 `NotFound`, `@all` 은 `Builtin`.
    ///
    /// ★이미 파킹된 방송분은 살아남는다(스냅샷 원칙 — 위 섹션 주석)★: 삭제는 **명단**을 지우는 것이지 이미
    ///   접수된 배달을 취소하는 게 아니다. 파킹분은 발송 순간 결박된 incarnation 으로 그대로 배달된다.
    pub fn group_delete(&self, group: &str) -> Result<(), GroupError> {
        {
            let mut st = self.state.lock().expect("messaging state poisoned");
            st.groups.delete(group)?;
        }
        tracing::info!(group = %group.trim(), "그룹 삭제(파킹된 방송분은 그대로 배달 — 스냅샷 원칙)");
        Ok(())
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
/// ★id 가 없으면(부재 파킹) 이름 폴백 = WYSIWYA 유지(ADR-0101)★: 아직 뜨지 않은 이름 앞으로 건 request 는
///   나중에 그 이름으로 등장한 에이전트가 답할 주체다. 이 폴백이 "스폰 전 선지시" 를 살린다.
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

/// 그룹 fan-out 의 멤버별 **판정 결과**(락 안에서 정하고 락 밖에서 집행한다 — C4).
///
/// ★왜 판정과 집행을 나누나(락 규율)★: 파킹·장부는 락 안에서 원자적으로 끝내야 하고(큐가 "비어 보이는 창"
///   금지 — flush_for 주석), 주입·도어벨은 절대 락 안에서 하면 안 된다(port 호출). 그래서 락 안에선 "이
///   멤버를 어떻게 할지" 만 정해 이 값으로 들고 나온다.
enum MemberPlan {
    /// 지금 주입한다(락 밖). 해석된 산 멤버.
    Deliver(LiveAgent),
    /// 이미 파킹했다(장부 `pending` 기록 완료) — 락 밖에서 도어벨만 누른다.
    Parked { hint: String, doorbell: PeerId },
    /// 배달하지 않는다(장부 `skipped` 기록 완료) — 락 밖에서 할 일 없음.
    Skipped { hint: String },
}

/// 그룹 멤버 명단에서 **같은 이름 중복을 접는다**(첫 등장 순서 유지 — C4).
///
/// ★왜 필요한가★: 주소 단위는 이름이라(WYSIWYA — ADR-0101) 같은 이름이 두 번 나오면 그건 두 수신자가
///   아니라 **한 수신자**다. `@all` 은 로스터 이름 verbatim 이라 동명 에이전트가 둘이면 같은 이름이 두 번
///   들어온다 — 접지 않으면 응답 `results[]` 에 같은 이름이 두 줄(둘 다 동명 다수 skip) 나와 발신자가
///   "수신자가 둘인가?" 로 오독한다(spec §6 = 멤버당 한 줄).
/// ★등록 명단의 중복도 **의도적으로** 한 줄로 접는다(리뷰 항목 L — 명시적 결정)★: v1 레지스트리는 add
///   단계에서 이미 dedup 하지만 그건 이 함수가 **기대는 보장이 아니다** — 미래 `GroupSource`(폴더·계층)는
///   같은 이름을 여러 번 낼 수 있고, seam 계약은 "명단 verbatim" 만 요구한다. 그때도 답은 같다: **결과의
///   단위는 "멤버"** 이지 "명단 줄" 이 아니다. 같은 수신자에게 같은 방송을 두 번 밀면 수신 LLM 이 같은 지시를
///   두 번 읽는 실해이고, 결말이 하나인데 두 줄로 보고하면 회계만 부풀 뿐 새 사실이 없다. 명단에 이름을 두 번
///   적은 건 정보가 0 인 표기 중복이라 접어도 잃는 사실이 없다.
fn dedup_members(members: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    members
        .into_iter()
        .filter(|m| seen.insert(m.clone()))
        .collect()
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
    /// ★incarnation 결박(C4 리뷰 fix A)★ — `Some((id, epoch))` 면 그 incarnation **에게만** 배달 가능한
    ///   파킹이다(그룹 방송 전용). 단일 발송·notice 는 항상 `None`(이름 주소 규칙 유지). 의미·근거는
    ///   mailbox.rs `ParkedMessage.bound_incarnation` 주석이 정본.
    bound_incarnation: Option<(PeerId, u32)>,
    /// ★압력 회수의 기준 = **생사 스냅샷**(F2 — 결박과 다른 축, 단수에서 집합으로)★: 이 park 시점에
    ///   **park 키(이름)를 달고 있는 산 incarnation 전부**(`live_incarnations_named` 가 로스터 스냅샷에서
    ///   뽑는다). 하나도 해석 못 했으면 빈 슬라이스 = "모른다" → 저장소는 아무것도 회수하지 않는다.
    ///
    /// ★왜 `bound_incarnation` 과 따로 두나★: 결박은 "누구에게만 배달할까"(배달 대상), 이건 "무엇이 이미
    ///   배달 불가한 잔해인가"(회수 대상 판정)다. 그룹 파킹에선 값이 겹쳐 보이지만, **단일 발송은 결박이
    ///   `None` 이면서도 해석된 수신자가 있다**. 하나로 합치면 단일 발송이 결박돼 재스폰 배달이 멈춘다.
    /// ★왜 단수 `current` 가 아니라 집합인가(F2 — 실측 결함)★: 옛 축은 "이 park 이 해석한 그 하나" 였고,
    ///   저장소는 "그것과 다르면 잔해" 로 읽었다. 동명 다수(exact-id 발송·재스폰 공존)에서 그 판정은 **살아
    ///   있는 동명 A 앞 결박 메일**을 B 앞 park 의 압력으로 걷어냈다. 근거는 mailbox `is_stale_bound` 주석.
    /// ★cap 분모에는 관여하지 않는다★(round-6 — 분모는 결박을 모른다. F1 이후 분모는 큐 + in-flight).
    live_incarnations: &'a [(PeerId, u32)],
    kind: ParkKind,
    meta: &'a SendMeta,
    /// ★이 논리 메시지가 남길 배달기록 총수(round-2 리뷰 F3)★ — 단일 발송·notice = 1, 그룹 fan-out = 멤버 수.
    /// 파킹도 이력 행 하나를 남기므로(`Pending`) 그 행에 같은 기대 수를 박아야 조회의 잘림 판정이 성립한다.
    expected_rows: u16,
}

/// ★장부에 **못 남긴** 종점 전이 1건(C4 리뷰 fix J · round-5 finding 2)★ — 레코드가 이력 링에서 이미
///   밀려나(`TransitionError::NotFound`) 전이가 불가능했던 사실을, **찍으려던 상태와 함께** 나른다.
///
/// ★왜 `intended` 를 나르나(finding 2 — load-bearing)★: 이 수집함은 처음엔 만료(`expired`) 전용이었는데
///   이후 `skipped` 전이(확정 사망 회수·압력 회수)까지 같은 함에 담기게 됐다. 상태를 안 나르면 로그가
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

/// ★request 예약의 단일 출구 가드(round-4 리뷰 H1 · load-bearing)★ — `FlightSettle` 과 같은 패턴이다.
///
/// 예약 단계(`handle_single_send` 2단계)는 두 가지 **잠정** 상태를 만든다: ① 상한 압력으로 잠시 빼낸
/// 희생자 계약 ② 아직 접수되지 않은 새 request 의 계약. 둘 다 "발송이 실제로 접수될 때만" 확정돼야 한다.
///
/// ★왜 결과 분기가 아니라 Drop 인가★: 그 사이에 있는 `dispatch_single` 은 **락 밖 외부 호출**
///   (DeliveryPort inject — 운영에선 자식 stdin write)을 한다. 거기서 패닉이 나면 결과 분기는 아예 실행되지
///   않는데, 상위 `spawn_blocking` 이 그 패닉을 JoinError 로 삼켜 데몬은 계속 산다 — 즉 **프로세스가 살아
///   있는 채로** 희생자만 사라지고 잠정 계약은 남는 잔해가 굳는다(상한 계수도 어긋난 채). Drop 은 정상
///   반려와 언와인딩 양쪽에서 정확히 한 번 도므로 그 창을 구조적으로 닫는다.
/// ★poison 관용(FlightSettle 과 동일)★: 언와인딩 중이라면 다른 락이 poison 됐을 수 있다. 여기서 `expect`
///   하면 Drop 안에서 두 번째 패닉 → abort 다. 복구 시도가 실패하면 조용히 포기하는 편이 낫다(그 경우
///   데몬은 이미 비정상 상태이고, abort 는 산 에이전트 전부를 죽인다).
// ADR-0108 (mark-and-sweep 은퇴 — 커밋/롤백 RAII)
struct ReservationGuard<'a> {
    svc: &'a MessagingService,
    /// 이 예약을 만든 새 발송의 id(로그 상관용).
    new_msg_id: &'a str,
    /// 상한 압력으로 **은퇴 예정 표시**된 희생자(있으면) — 표시 정보만(원본은 장부 목록에 그대로 있다).
    ///   커밋 시 take 해 물리 제거 + 계측, 롤백 시 표시 해제.
    retired: Option<RetiredContract>,
    /// 잠정 계약의 id(request 발송일 때만 Some).
    provisional: Option<&'a str>,
    committed: bool,
}

impl ReservationGuard<'_> {
    /// 발송이 접수됐다 — 잠정 상태를 확정한다. 은퇴가 있었으면 예약 id 를 풀고 **그때** 계측을 남긴다
    /// (커밋 전에 찍으면 일어나지 않은 교환을 보고하게 된다).
    fn commit(&mut self) {
        self.committed = true;
        let retired = self.retired.take();
        {
            // 한 락 구간에서: 표시된 희생자 물리 제거 + 잠정 표시 해제(round-5 mark-and-sweep).
            let mut st = self.svc.state.lock().expect("messaging state poisoned");
            st.ledger.commit_open(
                self.provisional,
                retired.as_ref().map(|r| r.request_id.as_str()),
            );
        }
        let Some(r) = retired else {
            return;
        };
        // 필드 전용 계측(mcp-config write/remove 선례) — 본문·토큰은 싣지 않는다. 갈래 (a) 는 이미
        //   발신자에게 기한 초과 통지가 나간 계약이고, (b) 는 기한을 약속한 적 없는 계약이다 — 어느 쪽도
        //   데몬이 진 통지 빚을 어기지 않는다(ledger `open_request` 주석).
        tracing::info!(
            retired_msg_id = %r.request_id,
            from = %r.sender,
            to = %r.recipient,
            age_secs = r.age.as_secs(),
            new_msg_id = %self.new_msg_id,
            "미회신 계약 상한 압력 — 가장 오래된 은퇴 가능 계약을 내보내고 새 request 수용(사용자 결정 2026-07-27)"
        );
    }
}

impl Drop for ReservationGuard<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let retired = self.retired.take();
        let restored_id = retired.as_ref().map(|r| r.request_id.clone());
        if retired.is_none() && self.provisional.is_none() {
            return;
        }
        // ★한 락 구간에서 둘 다(round-4 H2 → round-5 로 단순화)★: 은퇴 표시 해제와 잠정 계약 제거를
        //   `rollback_open` 하나가 처리한다. 물리 제거/재삽입이 사라져 되돌릴 상태 자체가 없으므로,
        //   바깥에서 계약 수가 상한을 넘어 보이는 창도 없다.
        // ★poison 관용★ — 위 struct 주석. 실패하면 조용히 포기(두 번째 패닉 = abort 회피).
        let dropped = match self.svc.state.lock() {
            Ok(mut st) => st
                .ledger
                .rollback_open(self.provisional, restored_id.as_deref()),
            Err(_) => {
                tracing::error!(
                    msg_id = %self.new_msg_id,
                    "예약 롤백 실패 — messaging 상태 락이 poison(언와인딩 중). 잠정 계약/은퇴가 남을 수 있음"
                );
                return;
            }
        };
        // 락 밖 로깅(모듈 헤더 규율).
        if let Some(id) = restored_id {
            tracing::debug!(
                msg_id = %self.new_msg_id,
                unmarked_msg_id = %id,
                "발송이 접수되지 않아 은퇴 예정 표시를 해제 — 상한 교환 미성립(round-5 mark-and-sweep)"
            );
        }
        // ★반려된 request 계약 회수의 관측(기존 규율 유지)★: 이건 **누수 방지**지 "통지가 먼저 나가는
        //   레이스" 방지가 아니다. 예약과 롤백 사이에 sweep 이 끼어들어 기한 초과를 판정하면 통지는 이미
        //   파킹돼 나간다 — 그 통지는 회수할 수 없다(이미 발신자 큐에 있는 메시지다). 남는 잔여: 반려된
        //   발송에 대해 통지가 한 번 갈 수 있다. 실제로는 reply_by 최소 1분(ingress) vs 이 구간 마이크로초라
        //   사실상 도달 불가하고, 발생하면 이 warn 이 그 이중 결말을 관측 가능하게 남긴다.
        if dropped == Some(DropOutcome::Removed { notified: true }) {
            tracing::warn!(
                msg_id = %self.new_msg_id,
                "반려된 request 의 계약이 **이미 기한 초과 통지된** 상태였다 — 통지는 회수 불가(발신자에게 이미 감). 예약↔반려 사이 sweep 이 끼어든 희귀 레이스(ADR-0103)"
            );
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
    /// message 레인 압력으로 걷어낸 배달 불가 잔해의 msg_id(장부 `skipped` 전이는 락 안에서 이미 끝났다).
    retired_stale: Vec<String>,
    /// notice 레인 상한으로 밀려난 옛 통지의 msg_id(같은 전이·같은 규율).
    retired_notices: Vec<String>,
    /// ★회수 시점에 이미 TTL 을 넘겨 있던 항목(F3)★ — 장부 어휘가 `expired` 라 위 둘과 분리한다(레인 무관).
    retired_expired: Vec<String>,
    /// 그 전이가 링 evict 로 실패한 항목(의도 상태 동반 — finding 2).
    evicted: Vec<EvictedTransition>,
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
        if !self.retired_stale.is_empty() {
            tracing::debug!(
                recipient,
                retired = self.retired_stale.len(),
                msg_ids = ?self.retired_stale,
                "park retire: 보관함 cap(mailbox MAILBOX_CAP) 압력 — 결박 incarnation 이 사라져 배달 불가한 방송 잔해를 오래된 순으로 회수하고 장부 skipped(round-6)"
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

/// ★파킹 + 장부 `pending` 기록의 **락 안 알맹이**(단일 발송·그룹 fan-out 공용)★.
///
/// ★왜 자유 함수로 뽑았나★: 그룹 fan-out 은 멤버 N명의 판정·파킹을 **한 락 구간**에서 끝내야 하는데
///   (`handle_group_send` 락 규율), `park_pending` 은 자기가 락을 잡는다 — 그대로 부르면 멤버마다 락을
///   잡았다 놓아 "큐가 비어 보이는 창" 이 열린다. 그래서 락을 **호출자가 쥔 채** 부를 수 있는 알맹이를
///   분리하고, 단일 발송용 `park_pending` 은 이걸 감싸는 얇은 래퍼로 남긴다(파킹 규칙 한 벌 유지 —
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
        bound_incarnation: req.bound_incarnation,
    };
    let admitted = st
        .mailbox
        .park(req.recipient, parked, req.live_incarnations)?;
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
            (false, ParkKind::Message) => effects.retired_stale.push(m.msg_id),
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
///   배달 관측 레코드를 발행한다(등장 배달도 handle_single_send 와 동일하게 관측 — ADR-0088).
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
const PARK_PAYLOAD_VERSION: &str = "2";

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
        // C4: 그룹 라벨(봉투 `to`) — 파킹된 방송이 늦게 배달돼도 "방송" 표시를 잃지 않게 함께 나른다.
        let group = self.meta.group.as_deref().unwrap_or("");
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
                group: (!group.is_empty()).then(|| group.to_string()),
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

/// ★park 시점 생사 스냅샷(F2)★ — 주어진 로스터 스냅샷에서 **그 이름을 지금 달고 있는 산 incarnation
///   전부**를 `(PeerId, epoch)` 로 뽑는다. mailbox 의 압력 회수는 이 목록에 **없는** 결박만 잔해로 본다.
///
/// ★왜 유일 해석(`unique_reachable_in`)이 아니라 전량인가★: 이 시스템은 동명 다수를 1급 상태로 허용한다
///   (exact-id 발송이 모호성을 통과하고, 재스폰 직후 옛·새가 공존한다). 유일 해석은 그 상황에서 `None` 을
///   내거나 한 쪽만 지목하는데, 그 값을 회수 기준으로 쓰면 **다른 쪽 앞 산 메일**이 잔해로 몰려 걷힌다
///   (F2 결함). 회수 판정에 필요한 사실은 "누구에게 보낼까" 가 아니라 "누가 아직 살아 있나" 라서 집합이다.
/// ★증거의 한계(수용된 잔여)★: 근거는 `live_reachable_agents` 스냅샷이라 **살아 있지만 비-도달**
///   (non-structured)인 incarnation 은 여기 안 잡힌다 — 그 앞 결박은 압력이 찰 때 잔해로 판정될 수 있다.
///   더 강한 증거원이 없어 그대로 두되, 목록이 **비면** 저장소가 아예 회수하지 않으므로 최악은 막힌다.
fn live_incarnations_named(roster: &[LiveAgent], name: &str) -> Vec<(PeerId, u32)> {
    roster
        .iter()
        .filter(|a| a.name == name)
        .map(|a| (a.id, a.epoch))
        .collect()
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
        /// roster 밖 id 에 canonical 이름을 부여(비-도달 산 에이전트 모사 — finding 4).
        fn set_canonical(&self, id: PeerId, name: &str) {
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
        /// 로스터 핸들 — 테스트가 **inject 도중**(on_inject hook) 갈아끼우려고 복제해 간다(위 필드 주석).
        fn roster_handle(&self) -> Arc<StdMutex<Vec<LiveAgent>>> {
            self.roster.clone()
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

        fn inject_if_epoch(
            &self,
            to_id: PeerId,
            expected_epoch: u32,
            bytes: &[u8],
        ) -> Result<InjectReceipt, String> {
            self.inject_inner(to_id, Some(expected_epoch), bytes)
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
        port.set_on_inject(Arc::new(move |idx| {
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
        // ★finding 4 회귀★: exact PeerId 로 지목했으나 그 에이전트가 산-비도달(TUI, 로스터 제외)이면
        //   resolve 는 None → 파킹. 이때 UUID 로 park 하면 name-keyed flush 가 영영 못 잡는다 — canonical
        //   NAME 으로 park 해야 그 이름이 도달 가능해질 때 flush 가 배달한다.
        let (svc, port) = svc();
        let tui_id = PeerId::new_v4();
        // 로스터엔 없지만(비-도달) canonical 이름은 "tui-agent" 인 산 에이전트 모사.
        port.set_roster(vec![]);
        port.set_canonical(tui_id, "tui-agent");
        let out = svc
            .handle_single_send(
                "m1",
                ident(),
                "s",
                &tui_id.to_string(), // exact PeerId 지목
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
        //   그 사이 수신자가 respawn(같은 이름, **새 PeerId**)했다 — flush_for 는 enqueue 의 stale id 를
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
            group: None,
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
                group: None,
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
    ) -> (PeerId, PeerId) {
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
            group: None,
        }
    }

    /// 회신 발송 메타.
    fn reply_meta(in_reply_to: &str) -> SendMeta {
        SendMeta {
            request: false,
            reply_by_raw: None,
            reply_by: None,
            reply_to: Some(in_reply_to.to_string()),
            group: None,
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
            group: None,
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
    fn timeout_notice_parks_in_its_own_lane_when_sender_is_absent_and_is_ledgered() {
        // ★레인 분리(round-6 — 옛 "cap 예외" 를 대체)★: 발신자 보관함이 message 로 가득 차 있어도 notice 는
        //   자기 레인에서 수용된다(회신 계약 통지가 막히면 계약이 반쪽 — ADR-0103 불변식). 그리고 조용히
        //   사라지지 않는다 — 장부에 pending 으로 남는다.
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
        const NOTICE_CAP_FOR_TEST: usize = 20;
        let (svc, port) = svc();
        port.set_roster(vec![]); // alice(발신자)·bob(수신자) 모두 부재 → 파킹 + 계약 오픈.
        for i in 0..(NOTICE_CAP_FOR_TEST + 1) {
            svc.handle_single_send(
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
            "통지는 자기 레인 상한에서 멈춘다(옛 구현 = 21 — 무계)"
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
        svc.handle_single_send(
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
            group: None,
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
        let cap = observed_open_request_cap();
        fill_open_request_cap(&svc, boss_from);
        assert_eq!(svc.open_request_count(), cap);

        // 산 수신자를 세우고 inject 도중 hook 으로 **잠정 구간 안에서** id 발급을 시험한다.
        let (_t, target) = live("mid");
        port.set_roster(vec![target]);
        let probe: Arc<StdMutex<Option<(bool, String)>>> = Arc::new(StdMutex::new(None));
        let probe_h = probe.clone();
        let svc_h = svc.clone();
        port.set_on_inject(Arc::new(move |_| {
            // 이 시점 = 예약(은퇴) 후 · 커밋 전. 희생자 id 를 뽑으면 재발급돼야 한다.
            let mut two = vec!["m-fresh".to_string(), "cap0".to_string()];
            let drawn = svc_h.draw_daemon_msg_id_with(|| two.pop().expect("draw"));
            *probe_h.lock().unwrap() =
                Some((svc_h.msg_id_in_use_for_test("cap0"), drawn.id.clone()));
        }));

        svc.handle_single_send(
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

        let (in_use, drawn_id) = probe.lock().unwrap().clone().expect("hook 이 돌았다");
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

    // ── C4: 그룹 방송 fan-out(spec §4 · ADR-0103 결정 4 · ADR-0104 결정 1) ─────────────────────

    /// 발신자 신원 + 그 신원이 로스터에 있는 LiveAgent 를 함께 만든다(@all 자기 제외 검증용).
    fn live_sender(name: &str) -> (SenderIdentity, LiveAgent) {
        let id = PeerId::new_v4();
        (
            SenderIdentity {
                peer_id: id,
                epoch: 0,
            },
            LiveAgent {
                id,
                name: name.to_string(),
                epoch: 0,
            },
        )
    }

    /// 멤버별 (이름, 상태) 로 접어 단언하기 쉽게 만든다(hint 문구는 따로 본다).
    fn member_pairs(v: &[GroupMemberResult]) -> Vec<(String, GroupMemberStatus)> {
        v.iter().map(|m| (m.to.clone(), m.status)).collect()
    }

    #[test]
    fn all_group_excludes_the_sender_and_uses_the_send_time_snapshot_verbatim() {
        // spec §4 `@all` = 발송 순간 살아있는 수신 가능 전원 — 단 **발신자 자신은 뺀다**(자기 방송 메아리
        //   금지, handle_group_send 정책 주석). 명단은 스냅샷 이름 verbatim(정렬·재해석 없음).
        let (svc, port) = svc();
        let (from, sender_agent) = live_sender("boss");
        let (_a, alice) = live("alice");
        let (_b, bob) = live("bob");
        port.set_roster(vec![sender_agent, alice, bob]);

        let results = svc
            .handle_group_send(
                "g1",
                from,
                "boss",
                "@all",
                "리베이스 대기",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("@all 방송");
        assert_eq!(
            member_pairs(&results),
            vec![
                ("alice".to_string(), GroupMemberStatus::Delivered),
                ("bob".to_string(), GroupMemberStatus::Delivered),
            ],
            "발신자(boss)는 @all 명단에서 빠지고, 나머지는 로스터 순서 그대로"
        );
        assert_eq!(port.injected_bodies().len(), 2, "멤버 수만큼 개별 주입");
    }

    #[test]
    fn all_group_with_only_the_sender_live_is_group_empty() {
        // 발신자를 빼면 아무도 안 남는다 → GROUP_EMPTY(반려, 부작용 0). "혼자인데 방송" 은 조용히 성공한
        //   척하기보다 반려로 알려 주는 게 발신자에게 유용하다.
        let (svc, port) = svc();
        let (from, sender_agent) = live_sender("solo");
        port.set_roster(vec![sender_agent]);
        let out = svc.handle_group_send(
            "g1",
            from,
            "solo",
            "@all",
            "x",
            Entrance::Mcp,
            &SendMeta::default(),
        );
        assert_eq!(
            out,
            Err(GroupReject::Empty {
                name: "@all".to_string()
            })
        );
        assert!(port.injected_bodies().is_empty(), "부작용 0");
        assert!(svc.ledger_snapshot().is_empty(), "장부 부작용 0");
    }

    #[test]
    fn registered_group_delivers_to_its_own_list_including_the_sender() {
        // 등록 그룹은 **명단이 정본**이다 — 발신자를 넣은 건 명시적 의사표시라 그대로 배달한다(@all 의
        //   자기 제외와 갈리는 지점 — handle_group_send 정책 주석).
        let (svc, port) = svc();
        let (from, sender_agent) = live_sender("boss");
        let (_a, alice) = live("alice");
        port.set_roster(vec![sender_agent, alice]);
        svc.group_update("@coders", &["boss".to_string()], &[])
            .expect("그룹 멤버 등록");
        svc.group_update("@coders", &["alice".to_string()], &[])
            .expect("그룹 멤버 등록");

        let results = svc
            .handle_group_send(
                "g1",
                from,
                "boss",
                "@coders",
                "x",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("등록 그룹 방송");
        assert_eq!(
            member_pairs(&results),
            vec![
                ("boss".to_string(), GroupMemberStatus::Delivered),
                ("alice".to_string(), GroupMemberStatus::Delivered),
            ],
            "등록 순서대로 배달(발신자 포함)"
        );
    }

    #[test]
    fn unknown_and_empty_and_invalid_group_names_are_distinct_rejects() {
        let (svc, port) = svc();
        let (from, _s) = live_sender("boss");
        let (_a, alice) = live("alice");
        port.set_roster(vec![alice]);

        // 미등록 → NotFound.
        assert_eq!(
            svc.handle_group_send(
                "g1",
                from,
                "boss",
                "@ghost",
                "x",
                Entrance::Mcp,
                &SendMeta::default()
            ),
            Err(GroupReject::NotFound {
                name: "@ghost".to_string()
            })
        );
        // 등록됐으나 멤버 0명 → Empty(둘은 상위에서 다른 코드로 매핑되므로 반드시 구분).
        //   ★빈 그룹은 운영 표면으로 만든다★: 암묵 생성(add) 뒤 그 멤버를 빼면 "아는데 0명" 이 된다 —
        //   별도 create 동사가 없으므로(D 결정) 이게 정식 경로다.
        svc.group_update("@hollow", &["tmp".to_string()], &["tmp".to_string()])
            .expect("빈 그룹 준비");
        assert_eq!(
            svc.handle_group_send(
                "g2",
                from,
                "boss",
                "@hollow",
                "x",
                Entrance::Mcp,
                &SendMeta::default()
            ),
            Err(GroupReject::Empty {
                name: "@hollow".to_string()
            })
        );
        // 이름 규약 위반 → InvalidName(상위가 GROUP_NOT_FOUND + 규약 hint 로 매핑).
        assert_eq!(
            svc.handle_group_send(
                "g3",
                from,
                "boss",
                "@@x",
                "x",
                Entrance::Mcp,
                &SendMeta::default()
            ),
            Err(GroupReject::InvalidName {
                name: "@@x".to_string()
            })
        );
        assert!(port.injected_bodies().is_empty(), "반려는 전부 부작용 0");
    }

    #[test]
    fn mixed_fanout_injects_idle_parks_busy_and_skips_the_dead_member() {
        // ★C4 핵심 시나리오★: 한 방송 안에서 세 결말이 동시에 난다 — idle 은 즉시 주입(delivered),
        //   busy 는 파킹(pending, 산 멤버라 소급 금지에 걸리지 않는다), 부재 멤버는 skipped(파킹 없음).
        let (svc, port, gate) = svc_gated();
        let (from, _s) = live_sender("boss");
        let (_idle_id, idle_agent) = live("idle-one");
        let (busy_id, busy_agent) = live("busy-one");
        port.set_roster(vec![idle_agent, busy_agent]);
        gate.set_busy(busy_id, 0);
        svc.group_update("@team", &["idle-one".to_string()], &[])
            .expect("그룹 멤버 등록");
        svc.group_update("@team", &["busy-one".to_string()], &[])
            .expect("그룹 멤버 등록");
        svc.group_update("@team", &["ghost".to_string()], &[])
            .expect("그룹 멤버 등록"); // 스폰된 적 없는 이름.

        let results = svc
            .handle_group_send(
                "g1",
                from,
                "boss",
                "@team",
                "전원 리베이스",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("혼합 방송");
        assert_eq!(
            member_pairs(&results),
            vec![
                ("idle-one".to_string(), GroupMemberStatus::Delivered),
                ("busy-one".to_string(), GroupMemberStatus::Pending),
                ("ghost".to_string(), GroupMemberStatus::Skipped),
            ]
        );
        // 주입은 idle 멤버 1건뿐(턴 중 멤버에겐 밀지 않는다 — ADR-0104 idle 게이트).
        let injected = port.injected_bodies();
        assert_eq!(injected.len(), 1);
        assert!(injected[0].contains("전원 리베이스"));
        // busy 멤버는 큐에 남고, 죽은 멤버는 **파킹되지 않는다**(방송 소급 금지 — ADR-0103 불변식).
        assert_eq!(svc.parked_len("busy-one"), 1);
        assert_eq!(svc.parked_len("ghost"), 0, "부재 멤버는 파킹 금지");
        // 장부 = 1 msg_id : N 배달기록(멤버별 상태 독립). 판정(파킹·skip)은 락 안에서 먼저 기록되고,
        //   주입 성공 기록은 락 밖 주입 뒤라 순서가 이렇게 갈린다.
        assert_eq!(
            svc.ledger_snapshot()
                .into_iter()
                .map(|(_, _, to, st)| (to, st))
                .collect::<Vec<_>>(),
            vec![
                ("busy-one".to_string(), DeliveryStatus::Pending),
                ("ghost".to_string(), DeliveryStatus::Skipped),
                ("idle-one".to_string(), DeliveryStatus::Delivered),
            ]
        );
    }

    #[test]
    fn duplicate_name_member_is_skipped_not_delivered_twice() {
        // 동명 다수는 방송이 누구를 고를 근거가 없다 → skipped(단일 발송 RECIPIENT_AMBIGUOUS 와 같은 판정을
        //   멤버 단위로). 그리고 결과 줄은 **하나**여야 한다(멤버당 한 줄 — dedup_members).
        let (svc, port) = svc();
        let (from, _s) = live_sender("boss");
        let (_t1, twin1) = live("twin");
        let (_t2, twin2) = live("twin");
        let (_o, other) = live("solo");
        port.set_roster(vec![twin1, twin2, other]);

        let results = svc
            .handle_group_send(
                "g1",
                from,
                "boss",
                "@all",
                "x",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("@all 방송");
        assert_eq!(
            member_pairs(&results),
            vec![
                ("twin".to_string(), GroupMemberStatus::Skipped),
                ("solo".to_string(), GroupMemberStatus::Delivered),
            ],
            "동명 멤버는 한 줄 skipped, 나머지는 정상 배달"
        );
        assert!(
            results[0]
                .hint
                .as_deref()
                .unwrap_or_default()
                .contains("2 live agents"),
            "hint 가 사유(동명 다수)를 알려야: {:?}",
            results[0].hint
        );
        assert_eq!(port.injected_bodies().len(), 1, "동명 멤버에겐 주입 안 함");
    }

    #[test]
    fn member_with_a_full_mailbox_is_skipped_and_the_rest_still_receive() {
        // ★best-effort 정책★: 한 멤버의 보관함이 cap 이어도 방송 전체를 반려하지 않는다(단일 발송의
        //   MAILBOX_FULL 반려와 갈리는 지점 — handle_group_send 정책 주석).
        let (svc, port, gate) = svc_gated();
        let (from, _s) = live_sender("boss");
        let (full_id, full_agent) = live("full-one");
        let (_i, idle_agent) = live("idle-one");
        port.set_roster(vec![full_agent, idle_agent]);
        // full-one 을 턴 중으로 두고 cap 까지 채운다(직발송이 전부 그 큐로 들어가게).
        gate.set_busy(full_id, 0);
        for i in 0..100 {
            svc.handle_single_send(
                &format!("m{i}"),
                ident(),
                "s",
                "full-one",
                "x",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("cap 이내");
        }
        assert_eq!(svc.parked_len("full-one"), 100);
        svc.group_update("@team", &["full-one".to_string()], &[])
            .expect("그룹 멤버 등록");
        svc.group_update("@team", &["idle-one".to_string()], &[])
            .expect("그룹 멤버 등록");

        let results = svc
            .handle_group_send(
                "g1",
                from,
                "boss",
                "@team",
                "공지",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("cap 멤버가 있어도 방송 자체는 접수");
        assert_eq!(
            member_pairs(&results),
            vec![
                ("full-one".to_string(), GroupMemberStatus::Skipped),
                ("idle-one".to_string(), GroupMemberStatus::Delivered),
            ]
        );
        assert!(
            results[0]
                .hint
                .as_deref()
                .unwrap_or_default()
                .contains("mailbox is full"),
            "hint 가 사유를 알려야: {:?}",
            results[0].hint
        );
        assert_eq!(svc.parked_len("full-one"), 100, "cap 넘겨 밀어넣지 않는다");
        assert_eq!(
            svc.ledger_statuses("g1"),
            vec![DeliveryStatus::Skipped, DeliveryStatus::Delivered],
            "skip 도 장부에 남는다(조용한 유실 금지)"
        );
    }

    #[test]
    fn group_send_writes_one_ledger_record_per_member_under_one_msg_id() {
        // spec §4 "장부 = 메시지 1 : 배달기록 N" — 같은 msg_id 아래 멤버별 레코드가 독립 상태를 갖는다.
        let (svc, port, gate) = svc_gated();
        let (from, _s) = live_sender("boss");
        let (busy_id, busy_agent) = live("b");
        let (_i, idle_agent) = live("a");
        port.set_roster(vec![idle_agent, busy_agent]);
        gate.set_busy(busy_id, 0);
        svc.group_update("@t", &["a".to_string()], &[])
            .expect("그룹 멤버 등록");
        svc.group_update("@t", &["b".to_string()], &[])
            .expect("그룹 멤버 등록");
        svc.group_update("@t", &["gone".to_string()], &[])
            .expect("그룹 멤버 등록");

        svc.handle_group_send(
            "g1",
            from,
            "boss",
            "@t",
            "x",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("방송");
        let recs: Vec<(String, DeliveryStatus)> = svc
            .ledger_snapshot()
            .into_iter()
            .map(|(id, _, to, st)| {
                assert_eq!(id, "g1", "전 레코드가 한 논리 메시지 id 를 공유");
                (to, st)
            })
            .collect();
        assert_eq!(recs.len(), 3, "멤버 3명 → 배달기록 3건");
        assert!(recs.contains(&("a".to_string(), DeliveryStatus::Delivered)));
        assert!(recs.contains(&("b".to_string(), DeliveryStatus::Pending)));
        assert!(recs.contains(&("gone".to_string(), DeliveryStatus::Skipped)));
    }

    #[test]
    fn expiry_transitions_only_the_expired_members_record() {
        // ★(msg_id, recipient) 키(C4 — ADR-0104 앵커 해소)★: 만료는 **그 멤버의 배달기록 하나만** 건드려야
        //   한다. 옛 구현은 msg_id 로 "첫 pending 레코드" 를 역조회해 이력 삽입 순서와 만료 순서의 우연한
        //   상관에 정확성을 기댔다 — 이제 mailbox 가 큐 키를 함께 준다(`ExpiredParked`).
        let (svc, port, gate) = svc_gated();
        let (from, _s) = live_sender("boss");
        let (busy_id, busy_agent) = live("waiter");
        let (_i, idle_agent) = live("reader");
        port.set_roster(vec![idle_agent, busy_agent]);
        gate.set_busy(busy_id, 0);
        svc.group_update("@t", &["reader".to_string()], &[])
            .expect("그룹 멤버 등록");
        svc.group_update("@t", &["waiter".to_string()], &[])
            .expect("그룹 멤버 등록");

        svc.handle_group_send(
            "g1",
            from,
            "boss",
            "@t",
            "x",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("방송");
        // 파킹분이 TTL(24h)을 넘도록 sweep. reader 는 이미 delivered 라 만료 대상이 아니다.
        svc.sweep(Instant::now() + Duration::from_secs(25 * 3600));

        let recs: Vec<(String, DeliveryStatus)> = svc
            .ledger_snapshot()
            .into_iter()
            .map(|(_, _, to, st)| (to, st))
            .collect();
        assert!(
            recs.contains(&("waiter".to_string(), DeliveryStatus::Expired)),
            "만료된 멤버의 레코드만 expired: {recs:?}"
        );
        assert!(
            recs.contains(&("reader".to_string(), DeliveryStatus::Delivered)),
            "다른 멤버 레코드는 그대로: {recs:?}"
        );
        assert_eq!(svc.parked_len("waiter"), 0, "만료분은 큐에서 사라진다");
    }

    #[test]
    fn group_envelope_carries_the_to_attribute_immediately_and_after_flush() {
        // spec §1 노출 원칙: `to` 는 **그룹 방송에만** 실린다. 즉시 배달과 파킹→flush 배달의 봉투가 같아야
        //   한다(파킹은 raw 재료만 나르고 봉투는 주입 시점 조립 — ParkPayload 가 그룹 라벨을 실어 나른다).
        let (svc, port, gate) = svc_gated();
        let (from, _s) = live_sender("boss");
        let (busy_id, busy_agent) = live("late");
        let (_i, idle_agent) = live("now");
        port.set_roster(vec![idle_agent, busy_agent]);
        gate.set_busy(busy_id, 0);
        svc.group_update("@coders", &["now".to_string()], &[])
            .expect("그룹 멤버 등록");
        svc.group_update("@coders", &["late".to_string()], &[])
            .expect("그룹 멤버 등록");

        svc.handle_group_send(
            "g1",
            from,
            "boss",
            "@coders",
            "리베이스",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("방송");
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="boss" to="@coders">리베이스</message>"#.to_string()],
            "즉시 배달 봉투에 to 속성(golden)"
        );

        // 턴이 끝나 파킹분이 flush 되면 **같은 봉투**여야 한다.
        gate.clear();
        svc.flush_for("late", busy_id);
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="boss" to="@coders">리베이스</message>"#.to_string(),
                r#"<message from="boss" to="@coders">리베이스</message>"#.to_string(),
            ],
            "flush 배달도 to 속성을 유지(ParkPayload 그룹 라벨 왕복)"
        );
    }

    #[test]
    fn single_send_envelope_has_no_to_attribute() {
        // 대조군 — 1:1 발송은 `to` 를 싣지 않는다(방송 오독 방지, spec §1). C1~C3 봉투와 byte-identical.
        let (svc, port) = svc();
        let (_a, alice) = live("alice");
        port.set_roster(vec![alice]);
        svc.handle_single_send(
            "m1",
            ident(),
            "boss",
            "alice",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("배달");
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="boss">hi</message>"#.to_string()]
        );
    }

    #[test]
    fn a_reply_to_a_broadcast_goes_only_to_the_sender() {
        // spec §4 "회신은 항상 발신자 1인에게(전체회신 없음)" — 그룹 메시지 id 로 회신해도 그건 **1:1
        //   발송**이라 방송으로 번지지 않는다(그룹 주소로의 reply_to 자체는 입구가 반려한다).
        let (svc, port) = svc();
        let (from, boss_agent) = live_sender("boss");
        let (_a, alice) = live("alice");
        let (_b, bob) = live("bob");
        port.set_roster(vec![boss_agent, alice, bob]);
        svc.handle_group_send(
            "g1",
            from,
            "boss",
            "@all",
            "질문",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("방송");
        assert_eq!(port.injected_bodies().len(), 2, "전제: alice·bob 이 받음");

        // alice 가 그 방송 id 로 발신자에게만 회신.
        let out = svc
            .handle_single_send(
                "m-r",
                ident(),
                "alice",
                "boss",
                "받았음",
                Entrance::Mcp,
                &reply_meta("g1"),
            )
            .expect("회신 배달");
        assert_eq!(out, SendOutcome::Delivered);
        let bodies = port.injected_bodies();
        assert_eq!(bodies.len(), 3, "회신은 딱 1건 추가(전체회신 없음)");
        assert_eq!(
            bodies[2], r#"<message from="alice" in-reply-to="g1">받았음</message>"#,
            "회신 봉투엔 to 속성 없음(방송 아님)"
        );
        // 방송은 계약을 열지 않으므로 닫을 것도 없다(엄격 매칭 NoMatch — 배달엔 영향 없음).
        assert_eq!(svc.open_request_count(), 0);
    }

    #[test]
    fn broadcast_joins_an_existing_queue_instead_of_jumping_it() {
        // FIFO 일관성(C2 fix 5 와 같은 규율): 멤버가 idle 이어도 그 앞에 파킹이 쌓여 있으면 방송이 먼저
        //   나가면 안 된다 — 큐 뒤에 붙어 순서대로 배달된다.
        let (svc, port, gate) = svc_gated();
        let (from, _s) = live_sender("boss");
        let (dest_id, dest) = live("dest");
        port.set_roster(vec![dest]);
        gate.set_busy(dest_id, 0);
        svc.handle_single_send(
            "m-old",
            ident(),
            "s",
            "dest",
            "먼저 온 것",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("파킹");
        gate.clear(); // 턴은 끝났지만 아직 flush 전.

        let results = svc
            .handle_group_send(
                "g1",
                from,
                "boss",
                "@all",
                "방송",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("방송");
        assert_eq!(
            member_pairs(&results),
            vec![("dest".to_string(), GroupMemberStatus::Pending)],
            "선행 큐가 있으면 방송도 큐에 합류(pending)"
        );
        // 도어벨(미배선 = 인라인 flush 폴백)이 큐를 오래된 순으로 비운다.
        let bodies = port.injected_bodies();
        assert_eq!(bodies.len(), 2);
        assert!(
            bodies[0].contains("먼저 온 것"),
            "옛 메일이 먼저: {bodies:?}"
        );
        assert!(bodies[1].contains("방송"), "방송은 그 뒤: {bodies:?}");
    }

    /// ★(c) round-7 high — 방송의 즉시 배달도 같은 사각을 공유했다★: 멤버 앞 배치가 **주입 중**이면
    ///   (큐는 비어 보인다) 방송이 그 배치를 앞지른다. 단일 발송 3-b 와 **같은 판정 동사**로 닫혔는지 본다.
    #[test]
    fn a_broadcast_to_an_idle_member_joins_a_batch_that_is_already_in_flight() {
        let (svc, port, gate) = svc_gated(); // 도어벨 미배선 = 인라인 폴백.
        let (from, _s) = live_sender("boss");
        let (dest_id, dest) = live("dest");
        port.set_roster(vec![dest]);
        gate.set_busy(dest_id, 0);
        svc.handle_single_send(
            "m-old",
            ident(),
            "s",
            "dest",
            "먼저 온 것",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("파킹");
        gate.clear();

        // 옛 메일을 stdin 에 쓰는 **그 순간** 방송이 들어온다 — 큐는 drain 으로 비어 있고 그 1건은 in-flight.
        let svc_hook = svc.clone();
        let results: Arc<StdMutex<Vec<GroupMemberResult>>> = Arc::new(StdMutex::new(Vec::new()));
        let results_hook = results.clone();
        port.set_on_inject(Arc::new(move |idx| {
            if idx != 0 {
                return;
            }
            let r = svc_hook
                .handle_group_send(
                    "g1",
                    from,
                    "boss",
                    "@all",
                    "방송",
                    Entrance::Mcp,
                    &SendMeta::default(),
                )
                .expect("방송");
            *results_hook.lock().unwrap() = r;
        }));

        svc.flush_for("dest", dest_id);

        let results = results.lock().unwrap().clone();
        assert_eq!(
            member_pairs(&results),
            vec![("dest".to_string(), GroupMemberStatus::Pending)],
            "주입 중인 배치가 있으면 방송도 합류(pending)"
        );
        assert!(
            results[0]
                .hint
                .as_deref()
                .unwrap_or_default()
                .contains("earlier queued messages"),
            "hint 는 FIFO 합류 사유(큐 축) 그대로 — 사각이 닫혔어도 어휘는 안 늘린다: {:?}",
            results[0].hint
        );
        let bodies = port.injected_bodies();
        assert_eq!(bodies.len(), 2, "둘 다 배달됐다: {bodies:?}");
        assert!(
            bodies[0].contains("먼저 온 것"),
            "진행 중이던 배치가 먼저: {bodies:?}"
        );
        assert!(bodies[1].contains("방송"), "방송은 그 뒤: {bodies:?}");
        assert_eq!(svc.parked_len("dest"), 0);
        assert_eq!(svc.in_flight_len("dest"), 0, "영수증 정산 누수 없음");
    }

    #[test]
    fn inject_failure_during_fanout_skips_that_member_and_keeps_going() {
        // ★리뷰 fix B★: 방송 중 write 실패 = **그 순간 그 멤버가 도달 불가** = 죽은 멤버와 같은 사실이다
        //   (spec §4). 옛 구현은 이걸 파킹해서 그 멤버의 **다음 등장에 배달**했다 — 방송 소급 금지 위반.
        //   이제 `skipped` + write 오류 hint 이고, 그 멤버 큐는 비어 있어야 한다(나중에도 안 간다).
        //   그리고 한 멤버의 실패가 나머지 멤버의 배달을 볼모로 잡지 않는다.
        let (svc, port) = svc();
        let (from, _s) = live_sender("boss");
        let (_a, first) = live("first");
        let (_b, second) = live("second");
        port.set_roster(vec![first, second]);
        port.fail_at(&[0]); // 첫 멤버 주입만 실패.
        svc.group_update("@t", &["first".to_string()], &[])
            .expect("그룹 멤버 등록");
        svc.group_update("@t", &["second".to_string()], &[])
            .expect("그룹 멤버 등록");

        let results = svc
            .handle_group_send(
                "g1",
                from,
                "boss",
                "@t",
                "x",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("방송");
        assert_eq!(
            member_pairs(&results),
            vec![
                ("first".to_string(), GroupMemberStatus::Skipped),
                ("second".to_string(), GroupMemberStatus::Delivered),
            ]
        );
        assert!(
            results[0]
                .hint
                .as_deref()
                .unwrap_or_default()
                .contains("fake inject fail"),
            "hint 가 write 오류를 실어야 발신자가 재발송을 판단한다: {:?}",
            results[0].hint
        );
        assert_eq!(
            svc.parked_len("first"),
            0,
            "실패분은 파킹하지 않는다(파킹하면 다음 등장에 소급 배달된다 — ADR-0103 불변식)"
        );
        // 주입 루프가 멤버 순서대로 도므로 장부 기록도 그 순서(실패→skipped, 그 뒤 성공 delivered).
        assert_eq!(
            svc.ledger_statuses("g1"),
            vec![DeliveryStatus::Skipped, DeliveryStatus::Delivered],
            "실패분도 장부에 남는다(조용한 유실 금지) — 단 배달 대상이 아니라 skipped"
        );
    }

    // ── C4 리뷰 fix A: 방송 소급 금지의 **파킹분** 강제(incarnation 결박) ────────────────────────
    // ★두 테스트는 실측된 두 구멍의 회귀 그물이다★: 옛 구현은 파킹된 방송분이 flush 의 이름 폴백을 타고
    //   ① 같은 이름의 **새 PeerId** ② 같은 PeerId 의 **다음 epoch** 로 배달됐다. 둘 다 "발송 뒤 등장한
    //   에이전트" 라 ADR-0103 불변식 위반. 대조군(단일 발송은 여전히 재스폰이 이어받는다)은
    //   `dead_id_hint_falls_back_to_unique_name_rule_for_respawn` 가 지킨다 — 결박은 그룹 전용 축이다.

    /// 방송을 위해 파킹된 몫은 **같은 이름으로 새로 뜬 다른 PeerId** 에게 가지 않는다(구멍 ①).
    #[test]
    fn parked_broadcast_is_never_delivered_to_a_same_named_respawn() {
        let (svc, port, gate) = svc_gated();
        let (from, _s) = live_sender("boss");
        let (old_id, old_agent) = live("worker");
        port.set_roster(vec![old_agent]);
        gate.set_busy(old_id, 0); // 턴 중이라 방송이 파킹된다(산 멤버 = 소급 금지에 안 걸림).
        svc.group_update("@t", &["worker".to_string()], &[])
            .expect("그룹 멤버 등록");

        let results = svc
            .handle_group_send(
                "g1",
                from,
                "boss",
                "@t",
                "공지",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("방송");
        assert_eq!(
            member_pairs(&results),
            vec![("worker".to_string(), GroupMemberStatus::Pending)],
            "전제: busy 멤버는 파킹(pending)"
        );
        assert_eq!(svc.parked_len("worker"), 1);

        // 그 멤버가 죽고 **같은 이름의 새 에이전트**(새 PeerId)가 등장 — 옛 구현은 여기서 배달했다.
        let (new_id, new_agent) = live("worker");
        port.set_roster(vec![new_agent]);
        gate.clear();
        svc.flush_for("worker", new_id); // 등장 flush(이름 키)
        svc.flush_for_agent(new_id); // idle flush 입구(id 키)도 같이
        assert!(
            port.injected_bodies().is_empty(),
            "발송 순간 스냅샷 밖의 incarnation 에는 배달 금지(ADR-0103 불변식): {:?}",
            port.injected_bodies()
        );
        assert_eq!(
            svc.parked_len("worker"),
            1,
            "배달 못 해도 파킹은 유지(즉시 skip 금지 — TTL 이 종점)"
        );
        assert_eq!(
            svc.ledger_statuses("g1"),
            vec![DeliveryStatus::Pending],
            "아직 pending"
        );

        // TTL 이 지나면 (msg_id, recipient) 로 정확히 expired 전이 — spec §5 "파킹의 운명" 어휘 그대로.
        svc.sweep(Instant::now() + Duration::from_secs(25 * 3600));
        assert_eq!(svc.parked_len("worker"), 0, "만료분은 큐에서 사라진다");
        assert_eq!(
            svc.ledger_snapshot()
                .into_iter()
                .map(|(id, _, to, st)| (id, to, st))
                .collect::<Vec<_>>(),
            vec![(
                "g1".to_string(),
                "worker".to_string(),
                DeliveryStatus::Expired
            )],
            "그 멤버의 배달기록 하나만 expired"
        );
    }

    /// 방송을 위해 파킹된 몫은 **같은 PeerId 의 다음 epoch**(재시작한 그 에이전트)에도 가지 않는다(구멍 ②).
    ///
    /// ★round-3 fix 6 에서 결말이 바뀌었다★: 예전엔 배달만 막고 TTL(24h)까지 붙들었는데, "같은 PeerId 가
    ///   **더 높은 epoch** 으로 있다" 는 **확정 사망의 증거**다(epoch 단조 — ADR-0007). 증거가 있는데 붙들면
    ///   장부가 24시간 동안 `pending`(= "배달 대기 중")이라고 거짓말을 한다. 이제 그 자리에서 `skipped`.
    #[test]
    fn parked_broadcast_is_never_delivered_across_an_epoch_bump() {
        let (svc, port, gate) = svc_gated();
        let (from, _s) = live_sender("boss");
        let (id, agent) = live("worker"); // epoch 0
        port.set_roster(vec![agent]);
        gate.set_busy(id, 0);
        svc.group_update("@t", &["worker".to_string()], &[])
            .expect("그룹 멤버 등록");

        svc.handle_group_send(
            "g1",
            from,
            "boss",
            "@t",
            "공지",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("방송");
        assert_eq!(svc.parked_len("worker"), 1, "전제: 파킹");

        // 같은 PeerId 가 재시작 = **epoch 교체**(같은 이름·같은 id, 다른 incarnation). id 힌트만 보던
        //   옛 구현은 여기서 배달했다 — 그 incarnation 은 발송 순간에 존재하지 않았다.
        port.set_roster(vec![LiveAgent {
            id,
            name: "worker".to_string(),
            epoch: 1,
        }]);
        gate.clear();
        svc.flush_for("worker", id);
        assert!(
            port.injected_bodies().is_empty(),
            "epoch 이 오른 incarnation 은 발송 순간의 그 수신자가 아니다: {:?}",
            port.injected_bodies()
        );
        // ★fix 6★: 확정 사망이므로 TTL 을 기다리지 않고 그 자리에서 큐에서 빠지고 장부 종점이 찍힌다.
        assert_eq!(
            svc.parked_len("worker"),
            0,
            "확정 사망 결박은 24시간 붙들지 않는다(같은 id·더 높은 epoch = 되돌아올 수 없음)"
        );
        assert_eq!(
            svc.ledger_statuses("g1"),
            vec![DeliveryStatus::Skipped],
            "어휘는 spec §5 재사용 — `expired`(시계 사실)가 아니라 `skipped`(수신자 소멸, 죽은 멤버와 같은 결말)"
        );
        // 종점이므로 이후 sweep 이 다시 만져 상태를 흔들지 않는다(terminal 재전이 불법 — ledger).
        svc.sweep(Instant::now() + Duration::from_secs(25 * 3600));
        assert_eq!(svc.ledger_statuses("g1"), vec![DeliveryStatus::Skipped]);
    }

    /// 대조군(fix 6 의 경계) — 결박 대상이 **로스터에서 그냥 안 보이는** 경우는 여전히 보류한다.
    /// "안 보임" 은 재시작 중·일시 비-도달일 수 있어 사망의 증거가 아니다(증거는 오직 '더 높은 epoch').
    #[test]
    fn a_bound_item_is_held_when_its_incarnation_is_merely_absent_not_provably_dead() {
        let (svc, port, gate) = svc_gated();
        let (from, _s) = live_sender("boss");
        let (id, agent) = live("worker");
        port.set_roster(vec![agent]);
        gate.set_busy(id, 0);
        svc.group_update("@t", &["worker".to_string()], &[])
            .expect("그룹 멤버 등록");
        svc.handle_group_send(
            "g1",
            from,
            "boss",
            "@t",
            "공지",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("방송");
        assert_eq!(svc.parked_len("worker"), 1, "전제: 파킹");

        // 그 에이전트가 로스터에서 사라진다(죽었는지 재시작 중인지 알 수 없다).
        port.set_roster(vec![]);
        gate.clear();
        svc.flush_for("worker", id);
        assert!(port.injected_bodies().is_empty());
        assert_eq!(
            svc.parked_len("worker"),
            1,
            "부재는 사망의 증거가 아니다 — 파킹 유지(그 incarnation 이 돌아오면 받을 수 있다)"
        );
        assert_eq!(svc.ledger_statuses("g1"), vec![DeliveryStatus::Pending]);
        // 종점은 여전히 TTL.
        svc.sweep(Instant::now() + Duration::from_secs(25 * 3600));
        assert_eq!(svc.ledger_statuses("g1"), vec![DeliveryStatus::Expired]);
    }

    /// 그 incarnation 이 **돌아오면** 결박 배달이 그대로 성립한다(보류가 '영구 봉쇄'가 아님을 고정).
    #[test]
    fn a_held_bound_item_is_delivered_when_its_exact_incarnation_reappears() {
        let (svc, port, gate) = svc_gated();
        let (from, _s) = live_sender("boss");
        let (id, agent) = live("worker");
        port.set_roster(vec![agent.clone()]);
        gate.set_busy(id, 0);
        svc.group_update("@t", &["worker".to_string()], &[])
            .expect("그룹 멤버 등록");
        svc.handle_group_send(
            "g1",
            from,
            "boss",
            "@t",
            "공지",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("방송");
        port.set_roster(vec![]); // 잠깐 사라졌다가…
        gate.clear();
        svc.flush_for("worker", id);
        assert_eq!(svc.parked_len("worker"), 1, "보류");

        port.set_roster(vec![agent]); // …같은 (id, epoch) 로 다시 보인다.
        svc.flush_for("worker", id);
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="boss" to="@t">공지</message>"#.to_string()],
            "결박 대상 본인이 돌아오면 배달"
        );
        assert_eq!(svc.ledger_statuses("g1"), vec![DeliveryStatus::Delivered]);
    }

    /// 대조군 — 결박된 그 incarnation 이 **그대로** 살아 있으면(턴만 끝나면) 정상 배달된다.
    /// 결박이 "아무에게도 안 보낸다" 가 아니라 "그 수신자에게만 보낸다" 임을 고정한다.
    #[test]
    fn parked_broadcast_still_reaches_the_same_incarnation_when_its_turn_ends() {
        let (svc, port, gate) = svc_gated();
        let (from, _s) = live_sender("boss");
        let (id, agent) = live("worker");
        port.set_roster(vec![agent]);
        gate.set_busy(id, 0);
        svc.group_update("@t", &["worker".to_string()], &[])
            .expect("그룹 멤버 등록");

        svc.handle_group_send(
            "g1",
            from,
            "boss",
            "@t",
            "공지",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("방송");
        gate.clear(); // 턴 종료 — 같은 (id, epoch) 가 그대로 산 채다.
        svc.flush_for("worker", id);
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="boss" to="@t">공지</message>"#.to_string()],
            "결박 대상 본인에겐 늦게라도 배달된다(봉투도 그대로)"
        );
        assert_eq!(svc.ledger_statuses("g1"), vec![DeliveryStatus::Delivered]);
    }

    // ── round-3 fix 1: 해석↔주입 사이 재시작(epoch bump TOCTOU) ────────────────────────────────────
    // ★무엇을 막나★: 결박(`bound_incarnation`)은 **누구에게 보낼지**를 좁힐 뿐, write 자체가 무조건이면
    //   해석 이후 재시작한 새 incarnation 에 그대로 착지한다 — 결박이 종잇장이 되는 구멍이다. 두 테스트가
    //   즉시 fan-out 경로와 flush 경로 각각에서 그 창을 재현한다.
    // ★변이 검증★: 해당 경로의 `inject_if_epoch` 을 `inject` 로 되돌리면 두 테스트 모두 실패한다.

    /// 즉시 fan-out — 앞 멤버에 쓰는 동안 뒤 멤버가 **재시작**(epoch bump)하면 그 멤버에겐 쓰지 않는다.
    #[test]
    fn a_member_that_restarts_mid_fanout_is_not_injected_into_the_new_incarnation() {
        let (svc, port) = svc();
        let (from, _s) = live_sender("boss");
        let (first_id, first) = live("first");
        let (second_id, second) = live("second");
        port.set_roster(vec![first.clone(), second]);
        // 발송 스냅샷(첫 로스터 조회)은 둘 다 epoch 0 을 본다. 그 뒤(= 주입 시점)부터는 second 가
        //   재시작해 epoch 1 이다 — 계획과 write 사이에 세계가 바뀐 상황.
        port.arm_roster_after_first_call(vec![
            first,
            LiveAgent {
                id: second_id,
                name: "second".to_string(),
                epoch: 1,
            },
        ]);
        svc.group_update("@t", &["first".to_string()], &[])
            .expect("그룹 멤버 등록");
        svc.group_update("@t", &["second".to_string()], &[])
            .expect("그룹 멤버 등록");

        let results = svc
            .handle_group_send(
                "g1",
                from,
                "boss",
                "@t",
                "공지",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("방송");
        assert_eq!(
            member_pairs(&results),
            vec![
                ("first".to_string(), GroupMemberStatus::Delivered),
                ("second".to_string(), GroupMemberStatus::Skipped),
            ],
            "재시작한 멤버는 발송 순간의 그 수신자가 아니다 → 죽은 멤버와 같은 결말(skipped)"
        );
        assert!(
            results[1]
                .hint
                .as_deref()
                .unwrap_or_default()
                .contains("epoch mismatch"),
            "hint 가 사유를 실어야 발신자가 재발송을 판단한다: {:?}",
            results[1].hint
        );
        assert_eq!(
            port.injected
                .lock()
                .unwrap()
                .iter()
                .map(|(id, _)| *id)
                .collect::<Vec<_>>(),
            vec![first_id],
            "새 incarnation 에는 단 한 바이트도 가지 않는다"
        );
        assert_eq!(
            svc.parked_len("second"),
            0,
            "방송은 파킹하지 않는다(파킹하면 다음 등장에 소급 배달)"
        );
        assert_eq!(
            svc.ledger_statuses("g1"),
            vec![DeliveryStatus::Delivered, DeliveryStatus::Skipped],
        );
    }

    /// flush — 락 안 로스터 스냅샷으로 타깃을 정한 뒤, 실제 write 직전에 재시작하면 결박분은 쓰지 않는다.
    #[test]
    fn a_bound_item_is_not_injected_when_the_incarnation_flips_between_snapshot_and_write() {
        let (svc, port, gate) = svc_gated();
        let (from, _s) = live_sender("boss");
        let (id, agent) = live("worker");
        port.set_roster(vec![agent]);
        gate.set_busy(id, 0);
        svc.group_update("@t", &["worker".to_string()], &[])
            .expect("그룹 멤버 등록");
        svc.handle_group_send(
            "g1",
            from,
            "boss",
            "@t",
            "공지",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("방송");
        assert_eq!(svc.parked_len("worker"), 1, "전제: busy 라 파킹");

        // flush 의 로스터 스냅샷은 (id, epoch 0) 을 본다 → 배달 후보로 뽑힌다. 그런데 **inject 직전**에
        //   그 에이전트가 재시작한다(epoch 1) — 이 창이 정확히 fix 1 이 닫는 창이다.
        gate.clear();
        let handle = port.roster_handle();
        port.set_on_inject(Arc::new(move |_| {
            *handle.lock().unwrap() = vec![LiveAgent {
                id,
                name: "worker".to_string(),
                epoch: 1,
            }];
        }));
        svc.flush_for("worker", id);

        assert!(
            port.injected_bodies().is_empty(),
            "재시작 후의 incarnation 에는 쓰지 않는다: {:?}",
            port.injected_bodies()
        );
        assert_eq!(
            svc.parked_len("worker"),
            1,
            "거부분은 무손실 복원(장부는 여전히 pending)"
        );
        assert_eq!(svc.ledger_statuses("g1"), vec![DeliveryStatus::Pending]);
        // 다음 flush 는 같은 로스터에서 "확정 사망"(같은 id·더 높은 epoch)을 보고 종점을 찍는다(fix 6 연결).
        port.set_on_inject(Arc::new(|_| {}));
        svc.flush_for("worker", id);
        assert_eq!(svc.parked_len("worker"), 0);
        assert_eq!(svc.ledger_statuses("g1"), vec![DeliveryStatus::Skipped]);
    }

    // ── 다른 incarnation 앞 결박 잔해는 산 수신자의 우편함을 인질로 잡지 못한다(round-3 fix 2 의 요구,
    //    round-6 에서 **메커니즘 교체**: 회계 가시성 예외 → 압력 회수) ────────────────────────────
    /// 죽은 방송분이 cap 을 채워도 같은 이름의 **새 incarnation** 앞 1:1 발송은 반려되지 않는다.
    #[test]
    fn a_dead_broadcast_backlog_does_not_reject_mail_to_the_new_incarnation() {
        let (svc, port, gate) = svc_gated();
        let (from, _s) = live_sender("boss");
        let (old_id, old_agent) = live("worker");
        port.set_roster(vec![old_agent]);
        gate.set_busy(old_id, 0); // 턴 중 → 방송이 파킹된다.
        svc.group_update("@t", &["worker".to_string()], &[])
            .expect("그룹 멤버 등록");
        // cap(100)까지 방송분을 쌓는다 — 전부 (old_id, 0) 결박.
        for i in 0..100 {
            svc.handle_group_send(
                &format!("g{i}"),
                from,
                "boss",
                "@t",
                "공지",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("방송");
        }
        assert_eq!(svc.parked_len("worker"), 100, "전제: 큐가 cap 까지 참");

        // 그 멤버가 죽고 같은 이름의 **새 에이전트**가 뜬다. 결박분은 그쪽으로 절대 안 가지만(fix A),
        //   회계에서도 안 보여야 새 수신자가 정상적으로 메일을 받는다.
        let (new_id, new_agent) = live("worker");
        port.set_roster(vec![new_agent]);
        gate.clear();
        let out = svc
            .handle_single_send(
                "m-direct",
                ident(),
                "s",
                "worker",
                "1:1",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("MAILBOX_FULL 로 반려되면 안 된다(즉시 배달이라 파킹 자체를 안 한다)");
        assert_eq!(
            out,
            SendOutcome::Delivered,
            "FIFO 합류 판정은 잔해를 세지 않는다(visible_to = 순서 축) → 앞지를 대상이 없으니 즉시 배달"
        );
        assert_eq!(
            port.injected
                .lock()
                .unwrap()
                .iter()
                .map(|(id, _)| *id)
                .collect::<Vec<_>>(),
            vec![new_id],
            "새 incarnation 에게만, 그리고 결박분은 하나도 안 나갔다"
        );

        // busy 라 파킹으로 가는 갈래도 같다 — cap 이 잔해로 막히면 안 된다. 단 **메커니즘이 다르다**
        //   (round-6): 잔해도 용량을 차지하므로, 자리는 가장 오래된 잔해를 회수해 만든다(장부 skipped).
        gate.set_busy(new_id, 0);
        let out = svc
            .handle_single_send(
                "m-parked",
                ident(),
                "s",
                "worker",
                "나중에",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("stale 잔해를 회수해 자리를 만들므로 파킹 수용");
        assert!(matches!(out, SendOutcome::Parked { .. }));
        assert_eq!(
            svc.parked_len("worker"),
            100,
            "잔해 99 + 새 파킹 1 — 큐는 cap 을 넘지 않는다(옛 구현 = 101)"
        );
        assert_eq!(
            svc.ledger_statuses("g0"),
            vec![DeliveryStatus::Skipped],
            "밀려난 가장 오래된 잔해는 조용히 사라지지 않고 장부에 남는다"
        );
        assert_eq!(
            svc.ledger_statuses("g1"),
            vec![DeliveryStatus::Pending],
            "필요한 만큼만 회수한다(둘째로 오래된 잔해는 그대로)"
        );
    }

    // ── round-6: **단일 cap** 이 잔해의 무한 성장을 막는다(분모에서 가시성 예외를 걷어냈다) ──────────
    /// ★실측 결함 회귀★: 같은 이름을 **완전히 새 PeerId** 로 계속 갈아치우면 옛 유입 cap 의 분모(가시
    ///   항목)가 세대마다 리셋돼 100건씩 더 수용됐고, 그 잔해는 flush 의 "확정 사망" 판정(같은 id·더 높은
    ///   epoch)에도 안 걸려 최대 24h(TTL) 동안 남았다 — 큐가 무계였다(옛 구현 = 400건). 이제 cap 은 결박을
    ///   모르므로 큐가 100 에서 멈추고, 밀려난 잔해는 **조용히 사라지지 않고** 장부에 `skipped` 로 남는다.
    #[test]
    fn repeated_respawn_broadcasts_stay_under_the_mailbox_cap_and_ledger_every_retirement() {
        let (svc, port, gate) = svc_gated();
        let (from, _s) = live_sender("boss");
        svc.group_update("@t", &["worker".to_string()], &[])
            .expect("그룹 멤버 등록");

        // 4세대 × cap = 400건 방송. 세대마다 같은 이름의 새 PeerId 로 갈아치우고(고아 갈래), 턴 중이라
        //   방송은 전부 그 세대의 incarnation 에 결박된 채 파킹된다.
        let generations = 4usize;
        for g in 0..generations {
            let (id, agent) = live("worker");
            port.set_roster(vec![agent]);
            gate.set_busy(id, 0);
            for i in 0..MAILBOX_CAP_FOR_TEST {
                svc.handle_group_send(
                    &format!("g{g}-{i}"),
                    from,
                    "boss",
                    "@t",
                    "공지",
                    Entrance::Mcp,
                    &SendMeta::default(),
                )
                .expect("방송 접수(멤버 회계는 결과 줄에 실린다)");
            }
        }

        assert_eq!(
            svc.parked_len("worker"),
            MAILBOX_CAP_FOR_TEST,
            "큐는 단일 cap 에서 멈춘다(옛 구현 = 400)"
        );

        let snapshot = svc.ledger_snapshot();
        assert_eq!(
            snapshot.len(),
            generations * MAILBOX_CAP_FOR_TEST,
            "발송마다 레코드 1건(이력 링 4096 안이라 evict 없음)"
        );
        let skipped: Vec<&String> = snapshot
            .iter()
            .filter(|(_, _, _, s)| *s == DeliveryStatus::Skipped)
            .map(|(id, ..)| id)
            .collect();
        let pending = snapshot
            .iter()
            .filter(|(_, _, _, s)| *s == DeliveryStatus::Pending)
            .count();
        assert_eq!(
            skipped.len(),
            (generations - 1) * MAILBOX_CAP_FOR_TEST,
            "밀려난 300건이 전부 장부에 남는다(조용한 유실 0 — 수용 총량 = 잔존 + 회수)"
        );
        assert_eq!(pending, MAILBOX_CAP_FOR_TEST, "큐에 남은 것만 pending");
        // 회수는 **오래된 순** — 마지막 세대만 남고 앞선 세대는 통째로 밀려났다.
        let last_gen = format!("g{}-", generations - 1);
        assert!(
            skipped.iter().all(|id| !id.starts_with(&last_gen)),
            "가장 오래된 세대부터 회수된다(admission 순번 오름차순)"
        );
        let parked = svc.parked_msg_ids("worker");
        assert!(
            parked.iter().all(|id| id.starts_with(&last_gen)),
            "큐에는 마지막 세대만 남는다"
        );
        assert_eq!(
            parked.first().unwrap(),
            &format!("{last_gen}0"),
            "큐 앞머리 = 남은 것 중 가장 오래된 것(순서 보존)"
        );
    }

    // ── F2: 압력 회수는 **산 동명** 앞 메일을 잡아먹지 않는다 ──────────────────────────────────────
    #[test]
    fn capacity_retirement_never_eats_mail_bound_to_a_live_duplicate_name() {
        // ★F2 회귀(round-6 결함 — 실측 가능한 1급 상태)★: 같은 이름의 산 에이전트가 둘인 상황은 이 시스템의
        //   정상 상태다(exact-id 발송이 동명 모호성을 의도적으로 통과하고, 재스폰 직후 옛·새가 공존한다).
        //   옛 회수 판정은 "이 park 이 해석한 current 와 다르면 잔해" 라, B 앞 파킹의 압력이 **살아서 턴 중인
        //   A 앞 방송 메일**을 `skipped` 로 걷어냈다 — "회수가 산 메일을 잡아먹지 않는다" 는 보장 위반.
        let (svc, port, gate) = svc_gated();
        let (from, _s) = live_sender("boss");
        let (a_id, a_agent) = live("worker");
        port.set_roster(vec![a_agent.clone()]);
        gate.set_busy(a_id, 0); // 턴 중 → 방송이 A 결박으로 파킹된다.
        svc.group_update("@t", &["worker".to_string()], &[])
            .expect("그룹 멤버 등록");
        for i in 0..MAILBOX_CAP_FOR_TEST {
            svc.handle_group_send(
                &format!("g{i}"),
                from,
                "boss",
                "@t",
                "공지",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("방송");
        }
        assert_eq!(
            svc.parked_len("worker"),
            MAILBOX_CAP_FOR_TEST,
            "전제: A 결박분으로 cap 까지 참"
        );

        // 같은 이름의 두 번째 에이전트 B 등장 — ★A 는 여전히 살아 있다★(죽지 않았다는 게 이 테스트의 핵심).
        let (b_id, b_agent) = live("worker");
        port.set_roster(vec![a_agent, b_agent.clone()]);
        gate.set_busy(b_id, 0);
        // exact-id 지목은 동명 모호성을 통과한다 → B 앞 파킹 시도 = cap 압력.
        let rejected = svc.handle_single_send(
            "m-to-b",
            ident(),
            "s",
            &b_id.to_string(),
            "1:1",
            Entrance::Mcp,
            &SendMeta::default(),
        );
        assert_eq!(
            rejected,
            Err(SendReject::MailboxFull),
            "생사 스냅샷에 A 가 있으므로 회수할 잔해가 없다 → 산 메일을 먹는 대신 반려"
        );
        assert_eq!(svc.parked_len("worker"), MAILBOX_CAP_FOR_TEST, "부작용 0");
        assert_eq!(
            svc.ledger_statuses("g0"),
            vec![DeliveryStatus::Pending],
            "A 앞 메일은 한 건도 skipped 되지 않았다(옛 구현 = skipped)"
        );

        // 대조군 — A 가 사라지면 그 결박분은 **진짜** 잔해다 → 회수하고 수용(회수 기능이 죽은 게 아니다).
        port.set_roster(vec![b_agent]);
        let parked = svc
            .handle_single_send(
                "m-to-b2",
                ident(),
                "s",
                &b_id.to_string(),
                "1:1",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("A 가 없으면 잔해 회수로 수용");
        assert!(matches!(parked, SendOutcome::Parked { .. }));
        assert_eq!(
            svc.ledger_statuses("g0"),
            vec![DeliveryStatus::Skipped],
            "증거가 생기면 가장 오래된 잔해가 회수되고 장부에 남는다"
        );
    }

    // ── F1: flush 중 "빈 큐 창" 이 cap 을 뚫지 못한다(in-flight 회계) ────────────────────────────
    #[test]
    fn repeated_flush_failure_cycles_never_grow_the_queue_past_the_cap() {
        // ★F1 회귀(무계 성장 — 실측 인터리빙)★: flush 가 락 밖에서 주입하는 동안 큐는 **비어 보인다**. 옛
        //   구현은 그 창에서 동시 발송을 cap 만큼 통째로 받았고(cap 검사가 빈 큐를 봤다), inject 가 실패해
        //   배치가 `restore_ordered` 로 되돌아오면 큐가 그만큼 커졌다 — 사이클마다 +k 로 자라 상한이
        //   무의미해졌다. 장부는 전부 pending 이라 "유실" 로도 안 잡히는, 조용한 메모리 성장이다.
        let (svc, port) = svc();
        let (late_id, late) = live("late");
        port.set_roster(vec![]);
        for i in 0..MAILBOX_CAP_FOR_TEST {
            svc.handle_single_send(
                &format!("m{i}"),
                ident(),
                "s",
                "late",
                "x",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("cap 이내 파킹");
        }

        // inject 도중(= 락 밖·큐가 비어 보이는 창) 동시 발송 20건을 밀어넣는 hook. 로스터를 잠시 비워
        //   그 발송들이 파킹 경로를 타게 한다(부재 파킹) — self-heal 도 그동안 아무 것도 못 찾는다.
        let svc_hook = svc.clone();
        let port_hook = port.clone();
        let late_hook = late.clone();
        let seq = Arc::new(StdMutex::new(0usize));
        let seq_hook = seq.clone();
        port.set_on_inject(Arc::new(move |_| {
            let n = {
                let mut g = seq_hook.lock().unwrap();
                let v = *g;
                *g += 1;
                v
            };
            port_hook.set_roster(vec![]);
            for i in 0..20 {
                let _ = svc_hook.handle_single_send(
                    &format!("c{n}-{i}"),
                    ident(),
                    "s",
                    "late",
                    "x",
                    Entrance::Mcp,
                    &SendMeta::default(),
                );
            }
            port_hook.set_roster(vec![late_hook.clone()]);
        }));
        // 사이클마다 inject 를 **첫 항목에서** 실패시킨다(호출 인덱스는 누적이라 0,1,2 = 각 사이클의 첫 시도).
        port.fail_at(&[0, 1, 2]);
        for cycle in 0..3 {
            port.set_roster(vec![late.clone()]);
            svc.flush_for("late", late_id);
            assert_eq!(
                svc.parked_len("late"),
                MAILBOX_CAP_FOR_TEST,
                "사이클 {cycle}: 큐는 cap 에서 멈춘다(옛 구현 = 120·140·160 으로 성장)"
            );
        }
        assert_eq!(
            svc.ledger_statuses("m0"),
            vec![DeliveryStatus::Pending],
            "되돌아온 배치는 유실 없이 pending 유지(무손실은 그대로)"
        );
    }

    // ── F3: 회수 시점에 이미 TTL 을 넘긴 항목의 장부 어휘는 `expired` ────────────────────────────
    #[test]
    fn a_capacity_retired_item_past_its_ttl_is_ledgered_expired_not_skipped() {
        // ★F3 회귀★: TTL(24h)과 sweep 주기(60s) 사이에는 틈이 있어, **이미 만료됐지만 아직 안 걷힌** 항목이
        //   압력 회수로 먼저 걷힐 수 있다. spec §5 계약상 그 종점은 `skipped`(= 그 수신자에게 배달하지 않음)가
        //   아니라 `expired`(= 시계가 먼저 운명을 정함)다 — 옛 구현은 회수면 무조건 skipped 라 장부가 거짓
        //   원인을 남겼다(발신 LLM 이 "수신자가 사라졌나?" 로 오독한다).
        // ★park_into 를 직접 부르는 이유★: 운영 경로(`park_pending`)는 `Instant::now()` 를 쓰므로 테스트가
        //   24시간 경과를 만들 수 없다. 순수 알맹이에 시각을 주입하는 게 이 계약을 결정적으로 고정하는 길이다.
        let (svc, _port) = svc();
        let t0 = Instant::now();
        let dead = (PeerId::new_v4(), 0u32);
        let alive = (PeerId::new_v4(), 0u32);
        let meta = SendMeta::default();
        let mut effects = ParkSideEffects::default();
        let mut park_bound = |st: &mut MessagingState, id: &str, to: &str, now: Instant| {
            park_into(
                st,
                ParkRequest {
                    msg_id: id,
                    sender_name: "s",
                    from: ident(),
                    entrance: Entrance::Mcp,
                    recipient: to,
                    body: "x",
                    hinted_id: Some(dead.0),
                    bound_incarnation: Some(dead),
                    live_incarnations: &[dead],
                    kind: ParkKind::Message,
                    meta: &meta,
                    expected_rows: 1,
                },
                now,
                &mut effects,
            )
        };
        {
            let mut st = svc.state.lock().expect("state");
            // 두 수신자 큐를 t0 에 cap 까지 채운다(전부 dead 결박 = 회수 대상). w = 만료 갈래, w2 = 대조군.
            for i in 0..MAILBOX_CAP_FOR_TEST {
                park_bound(&mut st, &format!("g{i}"), "w", t0).expect("cap 이내");
                park_bound(&mut st, &format!("h{i}"), "w2", t0).expect("cap 이내");
            }
        }
        // TTL 을 넘긴 시점의 신규 park → 가장 오래된 잔해 1건이 회수되는데, 그건 이미 만료 상태다.
        let later = t0 + PARK_TTL_FOR_TEST + Duration::from_secs(1);
        let mut late_effects = ParkSideEffects::default();
        {
            let mut st = svc.state.lock().expect("state");
            park_into(
                &mut st,
                ParkRequest {
                    msg_id: "m-new",
                    sender_name: "s",
                    from: ident(),
                    entrance: Entrance::Mcp,
                    recipient: "w",
                    body: "x",
                    hinted_id: None,
                    bound_incarnation: None,
                    live_incarnations: &[alive],
                    kind: ParkKind::Message,
                    meta: &meta,
                    expected_rows: 1,
                },
                later,
                &mut late_effects,
            )
            .expect("잔해 회수로 수용");
            // 대조군 — 같은 회수를 **TTL 이전**에 하면 어휘는 그대로 `skipped` 다.
            park_into(
                &mut st,
                ParkRequest {
                    msg_id: "m-new2",
                    sender_name: "s",
                    from: ident(),
                    entrance: Entrance::Mcp,
                    recipient: "w2",
                    body: "x",
                    hinted_id: None,
                    bound_incarnation: None,
                    live_incarnations: &[alive],
                    kind: ParkKind::Message,
                    meta: &meta,
                    expected_rows: 1,
                },
                t0 + Duration::from_secs(1),
                &mut late_effects,
            )
            .expect("잔해 회수로 수용");
        }

        assert_eq!(
            svc.ledger_statuses("g0"),
            vec![DeliveryStatus::Expired],
            "TTL 지난 회수분은 expired(옛 구현 = skipped — 거짓 원인)"
        );
        assert_eq!(
            svc.ledger_statuses("h0"),
            vec![DeliveryStatus::Skipped],
            "TTL 전 회수분은 그대로 skipped(어휘를 뭉개지 않는다)"
        );
        assert_eq!(
            late_effects.retired_expired,
            vec!["g0".to_string()],
            "로그 갈래도 어휘를 따라 갈린다(만료는 잔해 회수 로그로 새지 않는다)"
        );
        assert_eq!(late_effects.retired_stale, vec!["h0".to_string()]);
    }

    // ── round-5 finding 2: evict 된 레코드의 로그는 **시도한 종점 상태**를 말해야 한다 ────────────────
    /// 이력 링에서 밀려난 레코드에 대한 종점 전이는 로그가 **유일하게 남는 감사 증거**다. 옛 구현은 그 수집함이
    /// 만료 전용이던 시절의 이름·문구를 그대로 써서, 회수(`skipped`)까지 `expired` 로 보고했다.
    #[test]
    fn an_evicted_terminal_transition_reports_the_status_that_was_attempted() {
        let t0 = Instant::now();
        let mut ledger = Ledger::with_capacity(1);
        ledger.record("m0", "s", "w", "본문", DeliveryStatus::Pending, t0);
        // 용량 1 이라 이 기록이 m0 를 링에서 밀어낸다(= 운영의 HISTORY_CAPACITY evict 와 같은 상황).
        ledger.record("m1", "s", "w", "본문", DeliveryStatus::Pending, t0);

        let mut evicted = Vec::new();
        // 회수(결박 수신자 소멸) 의도 → `skipped` 로 보고돼야 한다.
        transition_or_collect_evicted(
            &mut ledger,
            "m0",
            "w",
            DeliveryStatus::Skipped,
            t0,
            &mut evicted,
        );
        assert_eq!(
            evicted,
            vec![EvictedTransition {
                msg_id: "m0".to_string(),
                recipient: "w".to_string(),
                intended: DeliveryStatus::Skipped,
            }],
            "회수를 만료로 뭉개지 않는다(finding 2)"
        );
        // 만료 의도는 그대로 `expired` — 같은 지점이 두 어휘를 정확히 나른다.
        transition_or_collect_evicted(
            &mut ledger,
            "m0",
            "w",
            DeliveryStatus::Expired,
            t0,
            &mut evicted,
        );
        assert_eq!(evicted[1].intended, DeliveryStatus::Expired);
        // 살아 있는 레코드는 정상 전이 → 수집되지 않는다(수집함은 evict 사실 전용).
        transition_or_collect_evicted(
            &mut ledger,
            "m1",
            "w",
            DeliveryStatus::Skipped,
            t0,
            &mut evicted,
        );
        assert_eq!(evicted.len(), 2, "전이가 성공하면 수집 없음");
    }

    /// 대조군 — **같은 incarnation** 앞 결박분은 여전히 FIFO 합류 대상이다(잔해 필터가 과교정이 아님).
    #[test]
    fn a_broadcast_parked_for_the_same_incarnation_still_holds_the_fifo_line() {
        let (svc, port, gate) = svc_gated();
        let (from, _s) = live_sender("boss");
        let (id, agent) = live("worker");
        port.set_roster(vec![agent]);
        gate.set_busy(id, 0);
        svc.group_update("@t", &["worker".to_string()], &[])
            .expect("그룹 멤버 등록");
        svc.handle_group_send(
            "g1",
            from,
            "boss",
            "@t",
            "방송",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("방송");
        gate.clear(); // 턴은 끝났지만 아직 flush 전.

        let out = svc
            .handle_single_send(
                "m-direct",
                ident(),
                "s",
                "worker",
                "직발송",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("접수");
        assert!(
            matches!(out, SendOutcome::Parked { .. }),
            "같은 incarnation 앞 결박분은 보이므로 직발송이 큐에 합류한다"
        );
        // 도어벨(미배선 = 인라인 폴백)이 오래된 순으로 비운다.
        let bodies = port.injected_bodies();
        assert_eq!(bodies.len(), 2);
        assert!(bodies[0].contains("방송"), "방송이 먼저: {bodies:?}");
        assert!(bodies[1].contains("직발송"), "직발송은 뒤: {bodies:?}");
    }

    // ── round-3 fix 3/5: 도어벨 시점 — 배선=즉시 enqueue / 미배선(인라인)=응답 확정 후 ────────────────
    /// 인라인 폴백은 **주입 루프가 끝나고 results 가 확정된 뒤** 돈다(방송 한가운데 flush 재진입 금지).
    #[test]
    fn the_inline_flush_fallback_runs_after_the_broadcast_response_is_final() {
        let (svc, port, gate) = svc_gated(); // ★도어벨 미배선★ = 인라인 폴백.
        let (from, _s) = live_sender("boss");
        let (aq_id, aq) = live("aq");
        let (bee_id, bee) = live("bee");
        port.set_roster(vec![aq, bee]);
        svc.group_update("@t", &["aq".to_string()], &[])
            .expect("그룹 멤버 등록");
        svc.group_update("@t", &["bee".to_string()], &[])
            .expect("그룹 멤버 등록");
        // aq 앞에 옛 메일 하나를 파킹시켜 둔다(busy 로 park → 턴 종료). 방송은 여기 합류한다.
        gate.set_busy(aq_id, 0);
        svc.handle_single_send(
            "m-old",
            ident(),
            "s",
            "aq",
            "옛 메일",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("파킹");
        gate.clear();

        let results = svc
            .handle_group_send(
                "g1",
                from,
                "boss",
                "@t",
                "방송",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("방송");
        assert_eq!(
            member_pairs(&results),
            vec![
                ("aq".to_string(), GroupMemberStatus::Pending),
                ("bee".to_string(), GroupMemberStatus::Delivered),
            ],
        );
        // ★핵심 단언★: 방송 자신의 주입(bee)이 **먼저** 끝나고, 그 다음에야 aq 큐의 인라인 flush 가 돈다.
        //   옛 배치(도어벨을 주입 루프 **전에** 눌렀던 코드)에서는 [옛 메일, 방송(aq), 방송(bee)] 이 나왔다 —
        //   응답을 조립하기도 전에 aq 의 방송분이 delivered 로 전이되는 회계 skew 가 그 순서의 증상이다.
        let bodies = port.injected_bodies();
        assert_eq!(bodies.len(), 3, "중복 주입 없음: {bodies:?}");
        assert!(
            bodies[0].contains("방송"),
            "방송 fan-out 이 먼저: {bodies:?}"
        );
        assert_eq!(
            port.injected.lock().unwrap()[0].0,
            bee_id,
            "그 첫 주입은 즉시 배달 대상(bee)"
        );
        assert!(
            bodies[1].contains("옛 메일"),
            "그 뒤 aq 큐 flush: {bodies:?}"
        );
        assert!(
            bodies[2].contains("방송"),
            "aq 의 방송분은 큐 순서대로 마지막"
        );
        assert_eq!(
            svc.ledger_statuses("g1"),
            vec![DeliveryStatus::Delivered, DeliveryStatus::Delivered],
            "인라인 flush 후엔 둘 다 delivered(응답은 그 이전 시점의 사실이었다)"
        );
    }

    /// 배선된 도어벨은 **미루지 않는다** — 파킹이 정해진 그 자리에서 즉시 enqueue(논블록)한다.
    #[test]
    fn a_wired_doorbell_is_rung_immediately_not_deferred_past_the_injection_loop() {
        let (svc, port, gate, bell) = svc_gated_with_doorbell();
        let (from, _s) = live_sender("boss");
        let (aq_id, aq) = live("aq");
        let (bee_id, bee) = live("bee");
        port.set_roster(vec![aq, bee]);
        svc.group_update("@t", &["aq".to_string()], &[])
            .expect("그룹 멤버 등록");
        svc.group_update("@t", &["bee".to_string()], &[])
            .expect("그룹 멤버 등록");
        gate.set_busy(aq_id, 0); // aq 는 파킹 → 도어벨 대상.

        // bee 주입이 일어나는 그 시점에 도어벨이 **이미** 눌려 있어야 한다(미루면 여기서 비어 있다).
        let bell_probe = bell.clone();
        let seen_at_inject = Arc::new(StdMutex::new(Vec::new()));
        let sink = seen_at_inject.clone();
        port.set_on_inject(Arc::new(move |_| {
            *sink.lock().unwrap() = bell_probe.seen();
        }));

        let results = svc
            .handle_group_send(
                "g1",
                from,
                "boss",
                "@t",
                "방송",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("방송");
        assert_eq!(
            member_pairs(&results),
            vec![
                ("aq".to_string(), GroupMemberStatus::Pending),
                ("bee".to_string(), GroupMemberStatus::Delivered),
            ],
        );
        assert_eq!(
            *seen_at_inject.lock().unwrap(),
            vec![aq_id],
            "배선 갈래는 주입 루프 **전에** 이미 눌렸다 — 미루면 그 멤버의 깨우기가 남은 blocking write 뒤로 밀린다"
        );
        assert_eq!(bell.seen(), vec![aq_id], "중복 발화 없음");
        // 소비자가 없는 조립이므로 파킹은 그대로 남는다(= 발신 스레드가 배치 write 를 지지 않았다는 증거).
        assert_eq!(svc.parked_len("aq"), 1);
        assert_eq!(
            port.injected
                .lock()
                .unwrap()
                .iter()
                .map(|(id, _)| *id)
                .collect::<Vec<_>>(),
            vec![bee_id],
        );
    }

    // ── C4 리뷰 fix C: 멤버별 주입 직전 재확인(계획이 순차 write 사이에 stale 해진다) ──────────────

    /// 앞 멤버에 쓰는 동안 뒤 멤버가 **새 턴을 시작**하면, 계획이 `Deliver` 였어도 주입하지 않고 파킹한다
    /// (idle 게이트를 방송만 우회하는 구멍 — 단일 발송 3분기와 parity).
    #[test]
    fn a_member_that_starts_a_turn_mid_fanout_is_parked_instead_of_injected() {
        let (svc, port, gate) = svc_gated();
        let (from, _s) = live_sender("boss");
        let (_f, first) = live("first");
        let (second_id, second) = live("second");
        port.set_roster(vec![first, second]);
        svc.group_update("@t", &["first".to_string()], &[])
            .expect("그룹 멤버 등록");
        svc.group_update("@t", &["second".to_string()], &[])
            .expect("그룹 멤버 등록");
        // 첫 멤버 stdin 에 쓰는 **그 순간** 둘째 멤버가 턴을 시작한다(계획은 이미 굳은 뒤).
        let gate_hook = gate.clone();
        port.set_on_inject(Arc::new(move |idx| {
            if idx == 0 {
                gate_hook.set_busy(second_id, 0);
            }
        }));

        let results = svc
            .handle_group_send(
                "g1",
                from,
                "boss",
                "@t",
                "공지",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("방송");
        assert_eq!(
            member_pairs(&results),
            vec![
                ("first".to_string(), GroupMemberStatus::Delivered),
                ("second".to_string(), GroupMemberStatus::Pending),
            ]
        );
        assert_eq!(
            port.injected_bodies().len(),
            1,
            "턴을 시작한 멤버에겐 밀지 않는다(주입은 첫 멤버 1건뿐)"
        );
        assert_eq!(
            svc.parked_len("second"),
            1,
            "그 몫은 파킹돼 턴 종료를 기다린다"
        );
        assert!(
            results[1]
                .hint
                .as_deref()
                .unwrap_or_default()
                .contains("mid-turn"),
            "hint 는 계획 단계 파킹과 **같은 문구**여야 한다: {:?}",
            results[1].hint
        );
    }

    /// 앞 멤버에 쓰는 동안 뒤 멤버 앞으로 **다른 메일이 파킹**되면, 방송은 그 큐를 앞지르지 않고 합류한다
    /// (FIFO 일관성 — 단일 발송 3-b 와 parity).
    #[test]
    fn a_member_whose_queue_grows_mid_fanout_joins_that_queue_instead_of_jumping_it() {
        let (svc, port, gate) = svc_gated();
        let (from, _s) = live_sender("boss");
        let (_f, first) = live("first");
        let (second_id, second) = live("second");
        port.set_roster(vec![first, second]);
        svc.group_update("@t", &["first".to_string()], &[])
            .expect("그룹 멤버 등록");
        svc.group_update("@t", &["second".to_string()], &[])
            .expect("그룹 멤버 등록");
        // 첫 멤버에 쓰는 동안 둘째 멤버 앞으로 1:1 메일이 파킹된다(그 멤버가 잠깐 턴을 돌았다 끝낸 상황).
        //   busy 축과 분리해 **큐 축만** 뒤집기 위해, 파킹시킨 뒤 곧바로 idle 로 되돌린다.
        let svc_hook = svc.clone();
        let gate_hook = gate.clone();
        port.set_on_inject(Arc::new(move |idx| {
            if idx == 0 {
                gate_hook.set_busy(second_id, 0);
                let _ = svc_hook.handle_single_send(
                    "m-old",
                    ident(),
                    "s",
                    "second",
                    "먼저 온 것",
                    Entrance::Mcp,
                    &SendMeta::default(),
                );
                gate_hook.clear();
            }
        }));

        let results = svc
            .handle_group_send(
                "g1",
                from,
                "boss",
                "@t",
                "방송",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("방송");
        assert_eq!(
            member_pairs(&results),
            vec![
                ("first".to_string(), GroupMemberStatus::Delivered),
                ("second".to_string(), GroupMemberStatus::Pending),
            ]
        );
        assert!(
            results[1]
                .hint
                .as_deref()
                .unwrap_or_default()
                .contains("earlier queued messages"),
            "hint 가 FIFO 합류 사유여야 한다: {:?}",
            results[1].hint
        );
        // 도어벨(미배선 = 인라인 flush 폴백)이 큐를 오래된 순으로 비운다 — 방송이 옛 메일 뒤에 나간다.
        let bodies = port.injected_bodies();
        assert_eq!(bodies.len(), 3, "first 1건 + second 큐 2건: {bodies:?}");
        assert!(
            bodies[1].contains("먼저 온 것"),
            "옛 메일이 먼저: {bodies:?}"
        );
        assert!(bodies[2].contains("방송"), "방송은 그 뒤: {bodies:?}");
    }

    /// ★fix I★ — 그룹 갈래에도 배선 guard 를 둔다(단일 발송 guard 와 대칭). 계약 필드는 입구가 유일 검증자다.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "ingress가 유일 검증자")]
    fn contract_fields_on_the_group_path_trip_the_debug_assert() {
        let (svc, port) = svc();
        let (from, _s) = live_sender("boss");
        let (_a, alice) = live("alice");
        port.set_roster(vec![alice]);
        let _ = svc.handle_group_send(
            "g1",
            from,
            "boss",
            "@all",
            "x",
            Entrance::Mcp,
            &req_meta("10m", 600),
        );
    }

    // ── D: 그룹 관리 표면(group_list / group_members / group_update / group_delete) ──────────

    #[test]
    fn group_list_puts_all_first_then_sorts_the_registered_ones() {
        // 목록 순서는 응답 계약이자 테스트 단언 대상이라 결정적이어야 한다(저장은 HashMap = 비결정).
        let (svc, _port) = svc();
        for g in ["@zeta", "@alpha", "@mid"] {
            svc.group_update(g, &["x".to_string()], &[]).expect("등록");
        }
        assert_eq!(
            svc.group_list(),
            vec!["@all", "@alpha", "@mid", "@zeta"],
            "@all 이 머리, 나머지는 사전순"
        );
    }

    #[test]
    fn group_update_creates_implicitly_and_add_then_remove_within_one_call() {
        // 암묵 생성(사용자 결정 2026-07-26): 없는 그룹에 add 하면 생긴다 — 별도 create 동사 없음.
        let (svc, _port) = svc();
        let after = svc
            .group_update("@coders", &["alice".to_string(), "bob".to_string()], &[])
            .expect("암묵 생성 + add");
        assert_eq!(after, vec!["alice", "bob"], "등록 순서 보존");
        // 같은 호출에 add·remove 가 섞이면 **remove 가 이긴다**(제거 의도를 무시하는 쪽이 더 위험하다).
        let after = svc
            .group_update("@coders", &["carol".to_string()], &["alice".to_string()])
            .expect("증감");
        assert_eq!(after, vec!["bob", "carol"]);
        let after = svc
            .group_update("@coders", &["dave".to_string()], &["dave".to_string()])
            .expect("같은 이름 add+remove");
        assert_eq!(after, vec!["bob", "carol"], "한 호출 내 remove 가 최종");
    }

    /// ★round-3 리뷰 G2 회귀 그물 — 배치 실패는 레지스트리를 **전혀** 바꾸지 않는다★.
    ///
    /// 입구(`validate_group_args`)가 `@` 멤버를 먼저 거르지만, 그건 "입구를 반드시 거친다" 는 가정에 기댄
    /// 안전이다. 내부 호출자가 이 API 를 직접 부르는 경우(= 검증 우회)에도 부분 반영이 없어야 한다.
    #[test]
    fn group_update_is_atomic_when_the_shared_ingress_validation_is_bypassed() {
        let (svc, _port) = svc();
        let err = svc
            .group_update("@g", &["alice".to_string(), " @all".to_string()], &[])
            .expect_err("배치 안의 잘못된 멤버 이름은 반려");
        assert!(
            matches!(err, GroupError::InvalidMemberName { .. }),
            "{err:?}"
        );
        // 옛 구현은 alice 를 넣은 뒤 두 번째에서 에러를 내 그룹이 생긴 채로 남았다.
        assert_eq!(
            svc.group_list(),
            vec!["@all"],
            "실패한 배치는 그룹을 만들지도 않는다(부분 반영 0)"
        );
        assert!(matches!(
            svc.group_members("@g"),
            Err(GroupError::NotFound { .. })
        ));

        // 기존 명단이 있는 경우에도 add/remove 어느 쪽도 새어 들어가지 않는다.
        svc.group_update("@g", &["alice".to_string()], &[]).unwrap();
        let err = svc
            .group_update(
                "@g",
                &["bob".to_string(), "@nested".to_string()],
                &["alice".to_string()],
            )
            .expect_err("반려");
        assert!(matches!(err, GroupError::InvalidMemberName { .. }));
        assert_eq!(
            svc.group_members("@g"),
            Ok(vec!["alice".to_string()]),
            "bob 이 들어가지도, alice 가 빠지지도 않아야"
        );
    }

    #[test]
    fn group_update_with_no_changes_is_a_pure_query_and_never_creates() {
        // 입구가 "이름만 준 호출" 을 그대로 흘려도 안전해야 한다(부작용 0). 없는 그룹은 여전히 NotFound —
        //   빈 add 가 그룹을 만들면 오타 한 번에 유령 그룹이 쌓인다.
        let (svc, _port) = svc();
        assert!(matches!(
            svc.group_update("@ghost", &[], &[]),
            Err(GroupError::NotFound { .. })
        ));
        assert_eq!(svc.group_list(), vec!["@all"], "조회가 그룹을 만들지 않음");
        svc.group_update("@x", &["a".to_string()], &[]).unwrap();
        assert_eq!(svc.group_update("@x", &[], &[]), Ok(vec!["a".to_string()]));
    }

    #[test]
    fn group_members_reports_all_as_the_live_snapshot_and_empty_groups_as_empty() {
        let (svc, port) = svc();
        let (_a, alice) = live("alice");
        let (_b, bob) = live("bob");
        port.set_roster(vec![alice, bob]);
        assert_eq!(
            svc.group_members("@all"),
            Ok(vec!["alice".to_string(), "bob".to_string()]),
            "@all = live 스냅샷 verbatim(정렬·dedup 없음)"
        );
        // 아무도 없어도 조회는 정상(빈 목록) — 발송(resolve)의 GROUP_EMPTY 반려와 갈리는 지점.
        port.set_roster(vec![]);
        assert_eq!(svc.group_members("@all"), Ok(vec![]));
        // 등록 그룹: 멤버를 전부 빼도 "아는데 0명" 은 정상 조회 결과다.
        svc.group_update("@x", &["a".to_string()], &[]).unwrap();
        svc.group_update("@x", &[], &["a".to_string()]).unwrap();
        assert_eq!(svc.group_members("@x"), Ok(vec![]));
    }

    #[test]
    fn group_surface_rejects_bad_names_and_protects_the_builtin() {
        let (svc, _port) = svc();
        // 선행 `@` 없는 이름은 전 동사에서 거부(관대 보정 금지 — @ 네임스페이스 계약).
        for bad in ["coders", "@", "@@x", "@a@b"] {
            assert!(
                matches!(
                    svc.group_update(bad, &["a".to_string()], &[]),
                    Err(GroupError::InvalidName { .. })
                ),
                "add 이름 규약: {bad}"
            );
            assert!(matches!(
                svc.group_members(bad),
                Err(GroupError::InvalidName { .. })
            ));
            assert!(matches!(
                svc.group_delete(bad),
                Err(GroupError::InvalidName { .. })
            ));
        }
        // 내장 `@all` 은 증감·삭제 불가(해석 의미 오염 방지).
        assert_eq!(
            svc.group_update("@all", &["a".to_string()], &[]),
            Err(GroupError::Builtin)
        );
        assert_eq!(svc.group_delete("@all"), Err(GroupError::Builtin));
    }

    #[test]
    fn group_delete_removes_it_and_is_not_idempotent_about_absence() {
        let (svc, _port) = svc();
        svc.group_update("@x", &["a".to_string()], &[]).unwrap();
        assert_eq!(svc.group_delete("@x"), Ok(()));
        assert_eq!(svc.group_list(), vec!["@all"]);
        // 두 번째 삭제는 NotFound — "없는 걸 지웠다" 를 성공으로 답하면 오타를 숨긴다.
        assert!(matches!(
            svc.group_delete("@x"),
            Err(GroupError::NotFound { .. })
        ));
    }

    /// ★스냅샷 원칙 회귀 그물(사용자 결정 2026-07-26 — D)★: 명단 변경은 **앞으로의 발송**에만 영향을 준다.
    ///
    /// ★왜 이 그물이 필요한가★: 방송 파킹분은 "그 그룹의 멤버라서" 큐에 있는 게 아니라 **발송 순간 결박된
    ///   `(id, epoch)` 앞으로 접수된 배달**이다(C4). 그런데 flush 경로가 그룹 명단을 다시 들여다보게 되면
    ///   (예: "지금도 멤버인가" 확인) 멤버 제거 한 번이 이미 접수된 메일을 **조용히 삼킨다** — 발신자는
    ///   `pending` 성공 응답을 받은 뒤라 유실을 알 길이 없다. 그 회귀를 여기서 못 박는다.
    #[test]
    fn removing_a_member_does_not_cancel_an_already_parked_broadcast() {
        let (svc, port, gate) = svc_gated();
        let (from, _s) = live_sender("boss");
        let (worker_id, worker) = live("worker");
        port.set_roster(vec![worker]);
        gate.set_busy(worker_id, 0); // 턴 중 → 방송이 파킹된다(산 멤버라 소급 금지에 안 걸림).
        svc.group_update("@t", &["worker".to_string()], &[])
            .expect("명단 등록");

        let results = svc
            .handle_group_send(
                "g1",
                from,
                "boss",
                "@t",
                "공지",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("방송");
        assert_eq!(
            member_pairs(&results),
            vec![("worker".to_string(), GroupMemberStatus::Pending)],
            "전제: busy 멤버는 파킹"
        );
        assert_eq!(svc.parked_len("worker"), 1);

        // 파킹된 뒤 그 멤버를 명단에서 뺀다(그리고 그룹 자체도 지운다 — 둘 다 파킹분과 무관해야 한다).
        svc.group_update("@t", &[], &["worker".to_string()])
            .expect("멤버 제거");
        assert_eq!(svc.group_members("@t"), Ok(vec![]), "명단은 비었다");
        svc.group_delete("@t").expect("그룹 삭제");
        assert!(matches!(
            svc.group_members("@t"),
            Err(GroupError::NotFound { .. })
        ));

        // 그 멤버가 턴을 마치면 파킹분은 **그대로 배달된다**(발송 시점에 접수된 계약이므로).
        gate.clear();
        svc.flush_for("worker", worker_id);
        let bodies = port.injected_bodies();
        assert_eq!(
            bodies.len(),
            1,
            "명단에서 빠졌어도 파킹분은 배달된다(스냅샷 원칙): {bodies:?}"
        );
        assert!(bodies[0].contains("공지"));
        assert!(
            bodies[0].contains(r#"to="@t""#),
            "봉투의 그룹 라벨도 발송 순간 값 그대로(삭제된 그룹 이름): {bodies:?}"
        );
        assert_eq!(
            svc.ledger_statuses("g1"),
            vec![DeliveryStatus::Delivered],
            "장부도 delivered 로 닫힌다(유실 아님)"
        );
    }

    // ── D: 장부 조회 표면(message_state / open_items_for) ─────────────────────────────────

    #[test]
    fn message_state_returns_one_row_per_recipient_for_a_broadcast() {
        // spec §4 1 msg_id : N 배달기록 — 그룹 조회는 멤버별로 상태가 갈려 보여야 한다.
        let (svc, port, gate) = svc_gated();
        let (from, _s) = live_sender("boss");
        let (_i, idle_agent) = live("idle-one");
        let (busy_id, busy_agent) = live("busy-one");
        port.set_roster(vec![idle_agent, busy_agent]);
        gate.set_busy(busy_id, 0);
        svc.group_update(
            "@team",
            &[
                "idle-one".to_string(),
                "busy-one".to_string(),
                "ghost".to_string(),
            ],
            &[],
        )
        .expect("명단");

        let t0 = Instant::now();
        svc.handle_group_send(
            "g1",
            from,
            "boss",
            "@team",
            "공지",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("방송");

        let view = svc.message_state("g1", t0).expect("조회");
        assert_eq!(view.id, "g1");
        assert_eq!(view.from, "boss");
        assert!(!view.awaiting_reply, "통보는 회신 대기 아님");
        let rows: Vec<(&str, &str)> = view
            .rows
            .iter()
            .map(|r| (r.to.as_str(), r.status))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("busy-one", "pending"),
                ("ghost", "skipped"),
                ("idle-one", "delivered"),
            ],
            "멤버당 한 줄 + 상태 어휘는 발송 응답과 동일"
        );
        assert!(svc.message_state("m-nope", t0).is_none(), "없는 id 는 None");
    }

    #[test]
    fn message_state_marks_an_open_request_as_awaiting_reply_until_the_answer_lands() {
        let (svc, port) = svc();
        let (from, _s) = live_sender("alice");
        let (_b, bob) = live("bob");
        port.set_roster(vec![bob]);
        let t0 = Instant::now();
        svc.handle_single_send(
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
        let (bob_from, _b2) = live_sender("bob");
        let (_a, alice_agent) = live("alice");
        port.set_roster(vec![alice_agent]);
        svc.handle_single_send(
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
        svc.handle_single_send(
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
        svc.handle_single_send(
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
        svc.handle_single_send(
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
        svc.handle_single_send(
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
        svc.handle_single_send(
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
        let (_b, bob) = live("bob");
        port.set_roster(vec![bob]);
        let t0 = Instant::now();
        svc.handle_single_send(
            "m-a",
            alice_from,
            "alice",
            "bob",
            "1",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("req a");
        svc.handle_single_send(
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
        let (bob_from, _b2) = live_sender("bob");
        let (_a, alice_agent) = live("alice");
        port.set_roster(vec![alice_agent]);
        svc.handle_single_send(
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
        svc.handle_single_send(
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
    /// 채운다. 수신자는 미등장(부재 파킹)이라 발송은 전부 접수(pending)된다.
    fn fill_open_request_cap(svc: &Arc<MessagingService>, from: SenderIdentity) {
        for i in 0..observed_open_request_cap() {
            svc.handle_single_send(
                &format!("cap{i}"),
                from,
                "boss",
                &format!("victim{i}"),
                "q",
                Entrance::Mcp,
                &SendMeta {
                    request: true,
                    ..SendMeta::default()
                },
            )
            .expect("cap 이내 request 접수");
        }
    }

    /// ledger 의 `MAX_OPEN_REQUESTS` 는 private 이라 테스트에서 관측 가능한 값으로 되짚는다 —
    /// "몇 개를 열면 cap 인가" 를 실제 동작(더 못 여는 지점)으로 찾지 않고 상수를 복제하면 rot 하므로,
    /// 서비스 레벨에선 open_request_count 가 더 이상 늘지 않는 지점을 상한으로 본다.
    fn observed_open_request_cap() -> usize {
        512
    }

    /// ★round-5 (1) 실입구 판 — 동시 발송이 **남의 미확정 계약**을 은퇴시키지 못한다★.
    ///
    /// `on_inject` hook 으로 A 의 잠정 구간 **한가운데**서 B 의 발송을 끼워 넣는다(운영의 동시 스레드와
    /// 같은 상태 — A 는 아직 커밋 전이다). 픽스처는 **B 의 유일한 후보가 A 의 잠정 계약뿐**이 되게 짠다:
    /// 상한을 기한 대기(은퇴 불가) 계약으로 채우고 은퇴 가능한 하나만 두어 A 가 그걸 표시하게 한다.
    /// 옛 설계에선 B 가 A 의 계약을 지워, A 는 **배달에 성공했는데 계약이 없는** 상태가 됐다(그 request 로
    /// 온 회신이 전부 NoMatch → 발신자는 영원히 기다린다).
    #[test]
    fn a_concurrent_send_cannot_retire_an_in_flight_requests_contract() {
        let (svc, port) = svc();
        let (boss_from, _boss) = live_sender("boss");
        port.set_roster(vec![]);
        let cap = observed_open_request_cap();

        // ① 은퇴 가능(기한 없음) 계약 하나 — 가장 먼저 만들어 최고령으로 둔다(A 가 이걸 표시한다).
        svc.handle_single_send(
            "evictable",
            boss_from,
            "boss",
            "v0",
            "q",
            Entrance::Mcp,
            &SendMeta {
                request: true,
                ..SendMeta::default()
            },
        )
        .expect("접수");
        // ② 나머지는 기한 대기 = 은퇴 불가(데몬이 진 통지 빚).
        for i in 1..cap {
            svc.handle_single_send(
                &format!("locked{i}"),
                boss_from,
                "boss",
                &format!("v{i}"),
                "q",
                Entrance::Mcp,
                &req_meta("10m", 600),
            )
            .expect("접수");
        }
        assert_eq!(svc.open_request_count(), cap, "전제: 상한");

        let (_t, target) = live("mid");
        port.set_roster(vec![target]);
        let (carol_from, _carol) = live_sender("carol");
        let svc_h = svc.clone();
        let b_result: Arc<StdMutex<Option<Result<SendOutcome, SendReject>>>> =
            Arc::new(StdMutex::new(None));
        let b_h = b_result.clone();
        port.set_on_inject(Arc::new(move |idx| {
            if idx != 0 {
                return;
            }
            // A 의 잠정 구간 — B 의 유일한 후보는 A 의 미확정 계약뿐이다.
            let out = svc_h.handle_single_send(
                "B",
                carol_from,
                "carol",
                "nobody-b",
                "해줘",
                Entrance::Mcp,
                &SendMeta {
                    request: true,
                    ..SendMeta::default()
                },
            );
            *b_h.lock().unwrap() = Some(out);
        }));

        svc.handle_single_send(
            "A",
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
        .expect("A 는 접수된다(evictable 을 표시하고 자리 확보)");

        // ★B 는 A 를 뺏지 못한다 — 고를 게 없으니 정직하게 반려★.
        let b = b_result
            .lock()
            .unwrap()
            .clone()
            .expect("hook 이 실제로 돌았다");
        assert_eq!(
            b,
            Err(SendReject::RequestCapacity),
            "잠정 계약을 희생자로 고르면 안 된다 — 고를 게 없으면 반려가 정답(round-5 (1))"
        );
        // ★A 의 계약이 살아 있다★.
        assert!(
            svc.open_items_for("boss", boss_from.peer_id, Instant::now())
                .iter()
                .any(|i| i.id == "A" && i.direction == Direction::AwaitingTheirReply),
            "동시 발송이 A 의 계약을 지우면 안 된다(round-5 (1))"
        );
        assert_eq!(svc.open_request_count(), cap, "상한 불변");
    }

    /// ★round-6 I1 실입구 판 — 잠정 구간에 회신이 먼저 와도 상한이 뚫리지 않는다★.
    ///
    /// `on_inject` 로 A 의 dispatch 한가운데서 ① A 의 request 에 대한 회신(잠정 계약을 닫는다) ② 뒤이은
    /// B 의 발송을 차례로 끼워 넣는다. 그 뒤 A 는 패닉으로 롤백된다(가드의 언와인딩 경로 — H1 과 같은 seam).
    /// 옛 술어에서는 ①에서 A 가 자리를 잃고 ②가 표시 없이 들어와, A 롤백의 표시 해제(+1)만 남아 513 이 됐다.
    #[test]
    fn a_reply_during_the_provisional_window_does_not_let_the_cap_be_exceeded() {
        let (svc, port) = svc();
        let (boss_from, _boss) = live_sender("boss");
        port.set_roster(vec![]);
        let cap = observed_open_request_cap();
        fill_open_request_cap(&svc, boss_from);
        assert_eq!(svc.occupied_slots_for_test(), cap, "전제: 상한");

        let (_t, target) = live("mid");
        port.set_roster(vec![target]);
        let (carol_from, _carol) = live_sender("carol");
        let (victim_from, _v) = live_sender("v-reply");
        let svc_h = svc.clone();
        let seen: Arc<StdMutex<Vec<usize>>> = Arc::new(StdMutex::new(Vec::new()));
        let seen_h = seen.clone();
        port.set_on_inject(Arc::new(move |idx| {
            if idx != 0 {
                return;
            }
            // ① A 의 잠정 계약("A")에 회신이 먼저 도착해 그것을 닫는다.
            let _ = svc_h.handle_single_send(
                "fast-reply",
                victim_from,
                "v-reply",
                "boss",
                "했음",
                Entrance::Mcp,
                &reply_meta("A"),
            );
            seen_h.lock().unwrap().push(svc_h.occupied_slots_for_test());
            // ② 그 직후 B 가 들어온다 — 자리가 없어야 하므로 표시하거나 반려돼야 한다.
            let _ = svc_h.handle_single_send(
                "B",
                carol_from,
                "carol",
                "nobody-b",
                "해줘",
                Entrance::Mcp,
                &SendMeta {
                    request: true,
                    ..SendMeta::default()
                },
            );
            seen_h.lock().unwrap().push(svc_h.occupied_slots_for_test());
            // ③ A 를 롤백시킨다(가드 Drop 경로).
            panic!("seam: A 의 dispatch 실패 모사");
        }));

        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let svc_p = svc.clone();
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _ = svc_p.handle_single_send(
                "A",
                boss_from,
                "boss",
                "mid",
                "해줘",
                Entrance::Mcp,
                &SendMeta {
                    request: true,
                    ..SendMeta::default()
                },
            );
        }));
        std::panic::set_hook(prev);
        assert!(caught.is_err(), "전제: A 가 패닉으로 이탈했다");

        let mid = seen.lock().unwrap().clone();
        assert_eq!(mid.len(), 2, "hook 이 두 지점 모두 관측했다");
        assert_eq!(
            mid[0], cap,
            "회신이 잠정 계약을 닫아도 자리는 유지된다(round-6 I1)"
        );
        assert!(mid[1] <= cap, "B 진입 후에도 상한 이내: {}", mid[1]);
        // ★핵심★: A 롤백 뒤에도 상한을 넘지 않는다(옛 술어에선 513 고착).
        assert!(
            svc.occupied_slots_for_test() <= cap,
            "롤백 뒤 상한 초과 금지(round-6 I1): {}",
            svc.occupied_slots_for_test()
        );
    }

    /// ★round-5 (3) 실입구 판 — 표시 구간에 도착한 회신이 희생자를 제대로 닫는다★.
    ///
    /// A 가 상한 압력으로 최고령 계약 `cap0` 에 은퇴 표시를 건 잠정 구간에서, `cap0` 의 수신자가 회신한다.
    /// 옛 설계에선 `cap0` 이 목록 밖이라 그 회신이 NoMatch 로 빗나갔고 롤백이 "열린 채" 되돌렸다.
    #[test]
    fn a_reply_arriving_during_the_marked_window_closes_the_victim_contract() {
        let (svc, port) = svc();
        let (boss_from, _boss) = live_sender("boss");
        port.set_roster(vec![]);
        let cap = observed_open_request_cap();
        fill_open_request_cap(&svc, boss_from);
        assert_eq!(svc.open_request_count(), cap);

        let (_t, target) = live("mid");
        port.set_roster(vec![target]);
        let svc_h = svc.clone();
        let (victim_from, _v) = live_sender("victim0");
        port.set_on_inject(Arc::new(move |idx| {
            if idx != 0 {
                return;
            }
            // 표시 구간 — cap0 의 수신자가 회신한다(수신자는 부재라 파킹되지만 계약은 닫혀야 한다).
            let _ = svc_h.handle_single_send(
                "reply-to-cap0",
                victim_from,
                "victim0",
                "boss",
                "했음",
                Entrance::Mcp,
                &reply_meta("cap0"),
            );
        }));

        // A 를 **반려**시켜 롤백 갈래까지 함께 검증한다(수신자 mid 는 산 상태라 접수되므로, 대신 A 를
        //   커밋 갈래로 두고 롤백 갈래는 아래 별도 단언으로 본다).
        svc.handle_single_send(
            "A",
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
        .expect("A 접수");

        // ★cap0 은 회신으로 닫혔다★ — 유령도, 뒤늦은 헛 통지도 없다.
        let items = svc.open_items_for("boss", boss_from.peer_id, Instant::now());
        assert!(
            !items
                .iter()
                .any(|i| i.id == "cap0" && i.direction == Direction::AwaitingTheirReply),
            "표시 구간의 회신이 계약을 닫아야(round-5 (3)): {:?}",
            items
                .iter()
                .filter(|i| i.id == "cap0")
                .map(|i| i.direction.as_str())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            svc.ledger_statuses("cap0"),
            vec![DeliveryStatus::Pending],
            "cap0 은 파킹 상태였으므로 이력은 pending 그대로(회신은 계약만 닫는다)"
        );
    }

    /// ★round-4 리뷰 H1 회귀 그물 — dispatch 중 **패닉**이 나도 예약이 원상 복구된다★.
    ///
    /// 운영에서 이 경로는 이렇게 살아난다: `dispatch_single` 안의 inject 는 자식 stdin write(락 밖 외부
    /// 호출)이고, 거기서 패닉이 나면 상위 `spawn_blocking` 이 JoinError 로 삼켜 **데몬은 계속 돈다**.
    /// 롤백이 결과 분기에만 있으면 그 언와인딩 경로에서 희생자는 사라지고 잠정 계약은 남아 상한이 영구히
    /// 어긋난 채 굳는다. 가드의 Drop 이 그 창을 닫는다.
    #[test]
    fn a_panic_inside_dispatch_still_rolls_the_reservation_back() {
        let (svc, port) = svc();
        let (boss_from, _boss) = live_sender("boss");
        let (_t, target) = live("victim-target");
        port.set_roster(vec![]); // cap 채우는 동안은 전원 부재(파킹으로 접수).
        let cap = observed_open_request_cap();
        fill_open_request_cap(&svc, boss_from);
        assert_eq!(svc.open_request_count(), cap, "전제: 미회신 계약이 상한");

        let has_contract = |svc: &Arc<MessagingService>, id: &str| {
            svc.open_items_for("boss", boss_from.peer_id, Instant::now())
                .iter()
                .any(|i| i.id == id && i.direction == Direction::AwaitingTheirReply)
        };
        assert!(has_contract(&svc, "cap0"), "전제: 최고령 계약 존재");

        // 산 수신자를 세워 inject 경로로 들어가게 하고, 그 안에서 패닉시킨다(락 밖 외부 호출 모사).
        port.set_roster(vec![target]);
        port.set_on_inject(Arc::new(|_| panic!("seam: DeliveryPort 패닉 모사")));

        let svc_for_panic = svc.clone();
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {})); // 테스트 출력 오염 방지(이 패닉은 의도된 것).
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _ = svc_for_panic.handle_single_send(
                "doomed",
                boss_from,
                "boss",
                "victim-target",
                "해줘",
                Entrance::Mcp,
                &SendMeta {
                    request: true,
                    ..SendMeta::default()
                },
            );
        }));
        std::panic::set_hook(prev_hook);
        assert!(caught.is_err(), "전제: dispatch 가 패닉했다");

        // ★언와인딩 뒤에도 상태가 온전하다★ — 가드 Drop 이 한 락 구간에서 되돌렸다.
        assert!(
            has_contract(&svc, "cap0"),
            "패닉으로 남의 계약이 사라지면 안 된다(H1)"
        );
        assert!(!has_contract(&svc, "doomed"), "잠정 계약은 남지 않는다(H1)");
        assert_eq!(
            svc.open_request_count(),
            cap,
            "상한 계수 불변 — 언와인딩이 계약 수를 흔들지 않는다"
        );
        // 예약도 풀렸다(그 id 가 영구히 발급 불가로 굳지 않는다).
        assert!(
            !svc.msg_id_in_use_for_test("cap0") || has_contract(&svc, "cap0"),
            "cap0 은 예약이 아니라 **추적**에 있어서 사용 중이어야"
        );
    }

    /// ★round-3 리뷰 G1 회귀 그물 — 반려는 남의 계약을 지우지 않는다(부작용 0)★.
    ///
    /// 시나리오: 미회신 계약이 상한(전부 은퇴 가능) → 새 request 가 **보관함 가득한 수신자**를 향해 들어와
    /// 다운스트림에서 `MAILBOX_FULL` 로 반려된다. 옛 구현은 예약 단계에서 이미 가장 오래된 계약을 지워
    /// 버렸고 롤백은 새 계약만 회수해서, **아무도 얻은 것 없이 남의 미결만 증발**했다(계수도 511 로 샘).
    #[test]
    fn a_rejected_request_does_not_permanently_retire_someone_elses_contract() {
        let (svc, port) = svc();
        let (boss_from, _boss) = live_sender("boss");
        port.set_roster(vec![]); // 전원 부재 → 발송은 파킹으로 접수된다.
        let cap = observed_open_request_cap();
        fill_open_request_cap(&svc, boss_from);
        assert_eq!(svc.open_request_count(), cap, "전제: 미회신 계약이 상한");
        // 전부 기한 없는 계약 = 은퇴 가능(규칙 (b)).

        // 보관함이 가득 찬 수신자를 만든다(같은 이름 앞 파킹 100건 = mailbox cap).
        for i in 0..100 {
            svc.handle_single_send(
                &format!("fill{i}"),
                boss_from,
                "boss",
                "full-one",
                "x",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("cap 이내 파킹");
        }
        assert_eq!(svc.parked_len("full-one"), 100, "전제: 보관함 가득");

        // ★계약 축만 본다★: 같은 id 가 **미배달 통보**(outbound_pending)로도 목록에 뜬다(파킹돼 있으니까) —
        //   은퇴는 **회신 계약**만 없애므로, 그 축을 안 가르면 통보 줄을 보고 "계약이 살아 있다" 고 오독한다.
        let has_contract = |svc: &Arc<MessagingService>, id: &str| {
            svc.open_items_for("boss", boss_from.peer_id, Instant::now())
                .iter()
                .any(|i| i.id == id && i.direction == Direction::AwaitingTheirReply)
        };
        assert!(
            has_contract(&svc, "cap0"),
            "전제: 가장 오래된 계약 cap0 이 미결(회신 대기)에 있다"
        );

        // ★그 request 는 반려된다★ — 은퇴는 성립하면 안 된다.
        let rej = svc
            .handle_single_send(
                "doomed",
                boss_from,
                "boss",
                "full-one",
                "해줘",
                Entrance::Mcp,
                &SendMeta {
                    request: true,
                    ..SendMeta::default()
                },
            )
            .expect_err("보관함 가득 → 반려");
        assert_eq!(rej, SendReject::MailboxFull);

        // ① 희생자의 **계약**이 그대로 있다.
        assert!(
            has_contract(&svc, "cap0"),
            "반려된 발송이 남의 계약을 지우면 안 된다(G1)"
        );
        // ② 미회신 계약 수가 그대로다(511 로 새지 않는다) + 반려된 계약은 남지 않는다.
        assert_eq!(
            svc.open_request_count(),
            cap,
            "상한 계수 불변 — 은퇴도 신규도 없었다"
        );
        assert!(
            !has_contract(&svc, "doomed"),
            "반려된 request 의 계약은 회수된다(기존 규율)"
        );
        // ③ 그 뒤 **정상** request 는 여전히 은퇴 교환으로 통과한다(복원이 상한을 망가뜨리지 않았다).
        svc.handle_single_send(
            "good",
            boss_from,
            "boss",
            "somebody-else",
            "해줘",
            Entrance::Mcp,
            &SendMeta {
                request: true,
                ..SendMeta::default()
            },
        )
        .expect("정상 경로는 은퇴 후 수용");
        assert!(has_contract(&svc, "good"), "새 계약이 열렸다");
        assert!(
            !has_contract(&svc, "cap0"),
            "이번엔 실제로 접수됐으므로 가장 오래된 계약이 은퇴한다(성공 경로는 정확히 1회 은퇴)"
        );
        assert!(
            has_contract(&svc, "cap1"),
            "은퇴는 **정확히 하나**뿐 — 그 다음으로 오래된 계약은 남는다"
        );
        assert_eq!(svc.open_request_count(), cap, "상한 불변(512 초과 없음)");
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

        svc.handle_single_send(
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

    /// ★F3 — 그룹 조회는 남은 행을 **기대 멤버 수**와 비교해 잘림을 답한다★(운영 경로 판).
    #[test]
    fn group_message_state_knows_how_many_rows_it_should_have() {
        let (svc, port) = svc();
        let (from, _s) = live_sender("boss");
        let (_a, a) = live("m1");
        let (_b, b) = live("m2");
        port.set_roster(vec![a, b]);
        svc.group_update("@t", &["m1".to_string(), "m2".to_string()], &[])
            .expect("명단");
        let t0 = Instant::now();
        svc.handle_group_send(
            "g1",
            from,
            "boss",
            "@t",
            "공지",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("방송");
        let view = svc.message_state("g1", t0).expect("조회");
        assert_eq!(view.rows.len(), 2, "멤버당 한 행");
        assert!(
            !view.may_be_truncated,
            "2/2 행이 살아 있으면 완전하다고 단언할 수 있다"
        );
    }

    /// ★B1 의 반대 축 — id 가 없는(부재 파킹) 계약은 **이름으로** 귀속된다(WYSIWYA 유지)★.
    /// 아직 뜨지 않은 이름 앞으로 건 request 는, 나중에 그 이름으로 등장한 에이전트가 답할 주체다.
    #[test]
    fn a_request_parked_for_an_absent_name_obligates_whoever_appears_under_it() {
        let (svc, port) = svc();
        let (boss_from, _boss) = live_sender("boss");
        port.set_roster(vec![]); // 수신자 미등장 → 해석 실패 → 부재 파킹(계약에 id 없음).
        let t0 = Instant::now();
        svc.handle_single_send(
            "m-req",
            boss_from,
            "boss",
            "later-worker",
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("파킹된 request");

        // 그 이름으로 뒤늦게 등장한 에이전트(발송 시점엔 없던 PeerId)가 의무를 본다.
        let (late_from, _late) = live_sender("later-worker");
        let items = svc.open_items_for("later-worker", late_from.peer_id, t0);
        assert_eq!(
            items
                .iter()
                .map(|i| (i.direction.as_str(), i.id.as_str()))
                .collect::<Vec<_>>(),
            vec![("reply_owed_by_me", "m-req")],
            "id 없는 계약은 이름 폴백으로 귀속(스폰 전 선지시 지원): {items:?}"
        );
    }

    /// ★B2 — 뷰가 완전성을 단언하는 경우와 유보하는 경우★.
    #[test]
    fn message_state_reports_whether_its_row_list_can_be_trusted_as_complete() {
        let (svc, port) = svc();
        let (from, _s) = live_sender("alice");
        let (_b, bob) = live("bob");
        port.set_roster(vec![bob]);
        let t0 = Instant::now();
        svc.handle_single_send(
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
        svc.handle_single_send(
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
}
