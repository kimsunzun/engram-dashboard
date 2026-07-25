//! messaging — S18 메시징 v1 순수 데이터 구조(ADR-0103/0104).
//!
//! ★역할★: 데몬이 살아 있는 동안 에이전트 간 메시지가 확실히 가게 하는 세 축의 **순수 데이터 구조**를
//! 모은다 — `mailbox`(부재·busy 수신자 파킹 저장소), `ledger`(전 메시지 이력 + request 회신 추적 + 그룹
//! 배달 장부), `groups`(그룹 명단 레지스트리 + 해석 seam). 여기엔 **오케스트레이션·동시성이 없다** — 이
//! 구조들을 tokio 위에서 엮는 `MessagingService`(장부·메일박스·그룹을 발송 파이프라인에 연결)는 후속
//! increment 다(이 모듈은 그 서비스가 소유할 상태 컨테이너만 제공).
//!
//! ★순수성 불변식(load-bearing — ADR-0104 seam 격리, ADR-0012)★: 이 모듈은 tokio·Tauri·스레드를 쓰지
//! 않고(동시성은 상위 서비스 소유), **로직 안에서 `Instant::now()`/`SystemTime::now()` 를 절대 부르지
//! 않으며**(시간 의존 메서드는 `now: Instant` 를 **인자로 주입받는다** — clock injection: TTL 경계·
//! 타임아웃을 결정적 단위 테스트로 단언 가능, now 를 손으로 밀어 시계 조작), 전역 가변 상태를 두지
//! 않는다. 이 불변식이 깨지면 단위 테스트가 실시간 sleep 에 의존하게 되고(느림·플레이키), seam 격리가
//! 무너진다.
//!
//! ★시간 타입 = `std::time::Instant`★: 봉투에 노출되지 않는 **내부 데이터**(spec §5 — 상태 전이 시각이
//!   곧 회신·발신 시각이나 봉투 미노출)라 monotonic `Instant` 로 충분하다(벽시계 표시가 필요해지면 v2 에서
//!   `SystemTime` 축을 무파괴 추가). TTL·reply-by 는 `Duration` 오프셋으로 표현해 주입된 now 와 비교한다.
// ADR-0103
// ADR-0104

pub mod groups;
pub mod ledger;
pub mod mailbox;
// C1: 순수 구조를 발송 파이프라인에 엮는 오케스트레이터(MessagingService + delivery seam). tokio·락은
//   여기서 소유(위 순수 구조는 무동시성). ADR-0103/0104.
pub mod service;
// C2: idle 게이트 — 수신자 턴 상태 관측(BusyTracker + 출력 스트림 tap) + 서비스가 묻는 BusyGate seam.
//   ★위 "순수성 불변식" 의 예외 구역★: 이 모듈은 core 의 `OutputSink` 콜백(pump 스레드)에 붙고 락·통지
//   채널을 다루므로 무동시성이 아니다(service.rs 와 같은 오케스트레이션 층). 대신 시간·tokio 를 쓰지
//   않으므로 결정적 단위 테스트는 유지된다(프레임을 손으로 먹여 상태머신을 단언). ADR-0104 결정 3.
pub mod busy;
