//! engram-dashboard-messaging — S18 메시징 커널 crate(ADR-0103/0104/0110).
//!
//! ★역할★: "데몬이 살아 있는 동안 에이전트 간 메시지가 확실히 가게 한다" 는 정책을 담는다 —
//! `mailbox`(부재·busy 수신자 파킹 저장소), `ledger`(전 메시지 이력 + request 회신 추적 + 그룹 배달
//! 장부), `groups`(그룹 명단 레지스트리 + 해석 seam), `envelope`(봉투 조립 + 배달 관측 어휘),
//! `service`(그 셋을 발송 파이프라인에 엮는 `MessagingService` + 접합 포트), `busy`(수신자 턴 상태
//! 게이트).
//!
//! ★완전 상호무지(load-bearing — ADR-0110 결정 2)★: 이 crate 는 **워크스페이스의 어떤 crate 에도
//! 의존하지 않는다**(core 포함). 외부 의존은 `uuid`·`tracing` 뿐이다. 그래서 에이전트 매니저·출력
//! sink·제어 레지스트리 같은 호스트 실물을 **타입으로도 알지 못한다** — 그 이름들은 이 crate 안에
//! 아예 등장하지 않고(격리 grep 게이트), 접합점은 전부 이
//! crate 가 소유한 계약(포트 trait)으로만 뚫린다: `service::DeliveryPort`(배달 실물) ·
//! `service::ControlPlanePort`(봉투 포맷 조회·배달 관측 기록) · `service::FlushTrigger`(도어벨) ·
//! `busy::TapHost`(턴 관측 구독) · `busy::IdleNotifier`(유휴 통지). 실물 어댑터는 호스트(데몬)가
//! 소유한다. 여기에 워크스페이스 의존을 추가하고 싶어지면 그건 "포트를 파야 한다" 는 신호이지 벽을
//! 뚫을 이유가 아니다 — 컴파일러가 강제하는 벽이 이 구조의 요점이다(규약이 아니라 구조).
//!
//! ★순수성 불변식(load-bearing — ADR-0104 seam 격리, ADR-0012)★: `mailbox`·`ledger`·`groups` 는
//! tokio·스레드를 쓰지 않고(동시성은 상위 서비스 소유), **로직 안에서 `Instant::now()`/
//! `SystemTime::now()` 를 절대 부르지 않으며**(시간 의존 메서드는 `now: Instant` 를 **인자로
//! 주입받는다** — clock injection: TTL 경계·타임아웃을 결정적 단위 테스트로 단언 가능, now 를 손으로
//! 밀어 시계 조작), 전역 가변 상태를 두지 않는다. 이 불변식이 깨지면 단위 테스트가 실시간 sleep 에
//! 의존하게 되고(느림·플레이키), seam 격리가 무너진다.
//!
//! ★시간 타입 = `std::time::Instant`★: 봉투에 노출되지 않는 **내부 데이터**(spec §5 — 상태 전이 시각이
//!   곧 회신·발신 시각이나 봉투 미노출)라 monotonic `Instant` 로 충분하다(벽시계 표시가 필요해지면 v2 에서
//!   `SystemTime` 축을 무파괴 추가). TTL·reply-by 는 `Duration` 오프셋으로 표현해 주입된 now 와 비교한다.
// ADR-0103
// ADR-0104
// ADR-0110

// ADR-0110 결정 5: 봉투 계층(ControlIngress 에서 이사) — messaging↔control 순환 해소 + 형제 소비자
//   (채팅)가 공유할 커널 부품을 올바른 자리에.
pub mod envelope;
pub mod groups;
pub mod ledger;
pub mod mailbox;
// C1: 순수 구조를 발송 파이프라인에 엮는 오케스트레이터(MessagingService + delivery seam). 락은
//   여기서 소유(위 순수 구조는 무동시성). ADR-0103/0104.
pub mod service;
// C2: idle 게이트 — 수신자 턴 상태 관측(BusyTracker + 턴 신호 수신구 `TurnProbe`) + 서비스가 묻는
//   BusyGate seam. ★위 "순수성 불변식" 의 예외 구역★: 이 모듈은 호스트의 출력 pump 스레드에서 불리는
//   콜백(`TurnProbe`)을 받고 락·통지 채널을 다루므로 무동시성이 아니다(service.rs 와 같은 오케스트레이션
//   층). 대신 시간·tokio 를 쓰지 않으므로 결정적 단위 테스트는 유지된다(신호를 손으로 먹여 상태머신을
//   단언). ADR-0104 결정 3 · ADR-0110 결정 4(출력 이벤트→턴 신호 **분류**는 백엔드 지식이라 호스트
//   어댑터 소유 — 여기엔 신호 어휘만 남는다).
pub mod busy;

/// 메시징 참여자 id — 이 crate 자체의 id 별칭(ADR-0110 결정 2 "완전 상호무지").
///
/// ★왜 core 의 `AgentId` 를 안 쓰나(load-bearing)★: 그걸 쓰는 순간 이 crate 가 워크스페이스 crate 에
///   의존하게 돼 완전 무지가 깨진다. 바닥 타입이 같은 `uuid::Uuid` 라 호스트(데몬)의 `AgentId` 값이
///   경계에서 **무변환으로 통과**한다 — 즉 벽을 세우는 비용이 사실상 0 이다(ADR-0110 근거: 경계 호출
///   ~66곳 무수정 컴파일). 미래에 에이전트 아닌 참여자(외부 메일 서비스·사람)가 생겨도 이 중립 이름이
///   그대로 버틴다.
pub type PeerId = uuid::Uuid;

/// 발신자 신원(참여자 + epoch) — 이 crate 자체의 값 타입(ADR-0110 결정 3 "포트 소유 구조").
///
/// ★왜 데몬의 `BoundIdentity` 를 안 쓰나★: 그건 제어 채널 토큰 바인딩 개념(호스트 인증의 산물)이라
///   메시징 커널이 알 이유가 없다. 커널이 쓰는 건 "누가·어느 세대로 보냈나" 두 필드뿐이다. 호스트가
///   경계에서 2필드 복사로 변환한다(데몬 측 `From<BoundIdentity>`).
///
/// ★epoch 가 신원의 일부인 이유(load-bearing)★: 같은 참여자라도 재spawn 하면 문맥이 끊긴다. 회신
///   추적·stale 판정이 epoch 를 함께 봐야 죽은 세대의 발신을 산 세대의 것으로 오인하지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SenderIdentity {
    pub peer_id: PeerId,
    pub epoch: u32,
}
